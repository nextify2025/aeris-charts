//! Optional GPUI executor for Aeris's backend-neutral `DrawList<Prim>` frame.
//!
//! Aeris's engine emits one ordered, backend-neutral primitive stream; WebGPU, Canvas2D and
//! tiny-skia already consume it. This crate is a fourth executor that translates the *same*
//! prepared frame into official GPUI scene primitives, so a GPUI application (Aeris Terminal) can host
//! a Aeris chart without Aeris growing a dependency on GPUI.
//!
//! # Layering
//!
//! ```text
//! aeris_charts_core -> aeris_charts_engine -> aeris_charts_render::DrawList<Prim> -> aeris_charts_render_gpui -> gpui
//! ```
//!
//! Nothing below this crate knows GPUI exists. `aeris_charts_core`, `aeris_charts_engine` and `aeris_charts_render`
//! contain no GPUI import, type, or `#[cfg]` branch, and GPUI is an **optional** dependency here:
//! without the `gpui-backend` feature the library never compiles it. Only this crate's
//! dev-dependencies (the probe's window tests and the pixel-parity harness) build GPUI for tests.
//!
//! # Two halves
//!
//! 1. **Lowering** (always compiled, GPUI-free): [`executor`] walks the prims in order and appends
//!    to a [`ScenePlan`] — the exact list of draw calls GPUI can make, in device px. All the
//!    interesting behavior (paint order, clipping, snapping, dash phase, gradient extents,
//!    tessellation, text placement) lives here and is unit-tested without a GPU.
//! 2. **Painting** (`gpui-backend` only): [`backend`] walks that plan and issues
//!    `paint_quad` / `paint_path` / text calls.
//!
//! # Calling this from a GPUI element (the Aeris Terminal boundary)
//!
//! Hold one [`GpuiChartRenderer`] per chart — it owns the cross-frame caches — and call
//! [`GpuiChartRenderer::paint_frame`] from your element's `paint` phase:
//!
//! ```ignore
//! impl Element for Aeris TerminalChart {
//!     fn paint(&mut self, _id: Option<&GlobalElementId>, bounds: Bounds<Pixels>,
//!              _: &mut (), _: &mut (), window: &mut Window, cx: &mut App) {
//!         // The frame is prepared BEFORE paint: no model, layout, or indicator work here.
//!         let prepared = self.chart.prepared_frame();
//!         let viewport = AerisViewport::from_bounds(
//!             bounds.origin.x.into(), bounds.origin.y.into(),
//!             bounds.size.width.into(), bounds.size.height.into(),
//!         );
//!         let metrics = self.renderer
//!             .paint_frame(&prepared, viewport, window.scale_factor(), window, cx)
//!             .expect("frame and window agree on DPR");
//!         self.telemetry.record(metrics);
//!     }
//! }
//! ```
//!
//! Build the frame *before* `paint` (`ChartEngine::build_frame_into`), then paint from the
//! immutable result: [`PreparedAerisFrame`] borrows, so the adapter cannot mutate market data,
//! models, indicators or layout while painting, and holds no engine lock during scene emission.

pub mod executor;
pub mod fixtures;
pub mod geometry;
pub mod image_cache;
pub mod metrics;
pub mod scene;
pub mod text;

#[cfg(feature = "gpui-backend")]
pub mod backend;
#[cfg(feature = "gpui-backend")]
pub mod input;

use aeris_charts_engine::{ChartEngine, ChartFrame};
use aeris_charts_render::color::Color;
use aeris_charts_render::draw_list::Prim;

pub use executor::ExecutorOptions;
pub use metrics::GpuiFrameMetrics;
pub use scene::{DeviceRect, MeshVertex, Paint, SceneOp, ScenePlan, TextRun};
pub use text::{TextCache, TextKey, TextMetrics, TextPlacement};

/// The immutable frame the adapter paints.
///
/// Mirrors exactly what the shipping WebGPU host composes (`aeris_charts_wasm::chart::inner_render`): the
/// engine's `ChartFrame` of stacked panes, then one final **unscissored** top layer carrying the
/// watermark, axis chrome, and axis/crosshair labels. The host owns that last conversion (it is
/// where browser- or application-specific axis policy lives), so the adapter takes it as prims
/// rather than reproducing it.
///
/// Every field is a shared borrow: painting cannot mutate engine state.
#[derive(Clone, Copy, Debug)]
pub struct PreparedAerisFrame<'a> {
    /// The engine's pane frame, from `ChartEngine::build_frame` / `build_frame_into`.
    pub frame: &'a ChartFrame,
    /// The final unscissored top layer, in paint order after every pane.
    pub axis_prims: &'a [Prim],
    /// Point pool referenced by `axis_prims` (the WebGPU host passes an empty pool).
    pub axis_points: &'a [[f32; 2]],
    /// Aeris-owned paint covering the complete chart surface before pane content.
    pub background: Paint,
}

impl<'a> PreparedAerisFrame<'a> {
    /// A frame with no axis layer — the pane content only.
    pub fn new(frame: &'a ChartFrame) -> Self {
        let background = aeris_charts_core::style::DEFAULT_SURFACE_RGB;
        Self {
            frame,
            axis_prims: &[],
            axis_points: &[],
            background: Paint::Solid(Color::rgb(background.0, background.1, background.2)),
        }
    }

    /// A frame whose complete surface paint is resolved from its owning engine options.
    pub fn from_engine(frame: &'a ChartFrame, engine: &ChartEngine) -> Self {
        let options = &engine.options.get().layout.background;
        let fallback = aeris_charts_core::style::DEFAULT_SURFACE_RGB;
        let fallback = Color::rgb(fallback.0, fallback.1, fallback.2);
        let background = if matches!(options.kind.as_str(), "gradient" | "vertical_gradient") {
            Paint::VGradient {
                top: Color::parse_css(&options.top_color).unwrap_or(fallback),
                bottom: Color::parse_css(&options.bottom_color).unwrap_or(fallback),
            }
        } else {
            Paint::Solid(Color::parse_css(&options.color).unwrap_or(fallback))
        };
        Self {
            frame,
            axis_prims: &[],
            axis_points: &[],
            background,
        }
    }

    /// Attach the host's axis/top layer.
    pub fn with_axis(mut self, axis_prims: &'a [Prim], axis_points: &'a [[f32; 2]]) -> Self {
        self.axis_prims = axis_prims;
        self.axis_points = axis_points;
        self
    }
}

/// Where the chart sits inside the GPUI window, in GPUI **logical** px.
///
/// GPUI's `paint_*` entry points take logical px and scale by the window's scale factor
/// themselves, whereas Aeris's `Prim` IR is in device px with the DPR already baked in. The
/// adapter therefore divides device coordinates by the scale factor and offsets them by
/// `offset_x`/`offset_y` at the GPUI boundary — see [`GpuiChartRenderer::paint_frame`].
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AerisViewport {
    /// Chart top-left within the window, logical px.
    pub offset_x: f32,
    pub offset_y: f32,
    /// Chart size, logical px. Matches `ChartFrame::width`/`height`.
    pub width: f32,
    pub height: f32,
}

impl AerisViewport {
    pub const fn new(offset_x: f32, offset_y: f32, width: f32, height: f32) -> Self {
        Self {
            offset_x,
            offset_y,
            width,
            height,
        }
    }

    /// From a GPUI element's `Bounds<Pixels>` components.
    pub const fn from_bounds(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self::new(x, y, width, height)
    }
}

/// Why a frame could not be painted.
#[derive(Clone, Debug, PartialEq)]
pub enum GpuiRenderError {
    /// The frame's `pixel_ratio` disagrees with the window's scale factor.
    ///
    /// The `Prim` IR is in device px computed from `pixel_ratio`. Painting it into a window at a
    /// different scale factor silently mis-scales the whole chart, so the adapter refuses rather
    /// than emitting a wrong frame: rebuild the frame at the window's scale factor first.
    ScaleFactorMismatch { frame: f64, window: f32 },
    /// The window's scale factor is not a usable positive number.
    InvalidScaleFactor(f32),
}

impl std::fmt::Display for GpuiRenderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GpuiRenderError::ScaleFactorMismatch { frame, window } => write!(
                f,
                "frame was built at pixel_ratio {frame} but the window reports scale factor \
                 {window}; rebuild the frame at the window's scale factor before painting"
            ),
            GpuiRenderError::InvalidScaleFactor(v) => {
                write!(f, "window scale factor {v} is not a positive finite number")
            }
        }
    }
}

impl std::error::Error for GpuiRenderError {}

/// Tolerance for the scale-factor agreement check.
///
/// GPUI reports fractional `f32` scale factors (1.25, 1.5, 2.5) while the engine stores an `f64`
/// `pixel_ratio`; the f32/f64 round-trip is the only difference we tolerate. Anything larger is a
/// real mismatch.
const SCALE_EPSILON: f64 = 1e-4;

/// A chart's GPUI renderer: the persistent, cross-frame state the adapter needs.
///
/// One per chart. Reused every frame so steady-state painting neither reallocates the scene plan
/// nor re-measures unchanged text.
pub struct GpuiChartRenderer {
    plan: ScenePlan,
    options: ExecutorOptions,
    text: TextCache,
    /// GPUI-owned shaped lines are feature-local so the default build remains GPUI-free.
    #[cfg(feature = "gpui-backend")]
    shaped_text: backend::ShapedTextCache,
    #[cfg(feature = "gpui-backend")]
    images: backend::RasterImageCache,
    /// Reusable tessellation buffers. Held here rather than rebuilt per frame: see
    /// [`geometry::Scratch`].
    scratch: geometry::Scratch,
}

impl Default for GpuiChartRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl GpuiChartRenderer {
    pub fn new() -> Self {
        Self::with_options(ExecutorOptions::default())
    }

    pub fn with_options(options: ExecutorOptions) -> Self {
        Self {
            plan: ScenePlan::default(),
            options,
            text: TextCache::default(),
            #[cfg(feature = "gpui-backend")]
            shaped_text: backend::ShapedTextCache::default(),
            #[cfg(feature = "gpui-backend")]
            images: backend::RasterImageCache::default(),
            scratch: geometry::Scratch::default(),
        }
    }

    pub fn options(&self) -> ExecutorOptions {
        self.options
    }

    /// Change how prims are expressed in GPUI. Invalidates nothing: the option affects lowering
    /// only, and the plan is rebuilt every frame.
    pub fn set_options(&mut self, options: ExecutorOptions) {
        self.options = options;
    }

    /// The plan produced by the most recent [`GpuiChartRenderer::plan_frame`] or
    /// [`GpuiChartRenderer::paint_frame`] call. Exposed for tests, golden captures and host
    /// diagnostics.
    pub fn plan(&self) -> &ScenePlan {
        &self.plan
    }

    pub fn text_cache(&self) -> &TextCache {
        &self.text
    }

    /// Bytes retained by the reusable tessellation buffers. Grows to the largest frame seen and
    /// then stays flat, which is what a replay test asserts on.
    pub fn scratch_bytes(&self) -> usize {
        self.scratch.capacity_bytes()
    }

    /// Drop every cached resource.
    ///
    /// Explicit rather than inferred so resource invalidation remains deterministic.
    /// Call when something the cache keys cannot observe changes: a registered font set, a theme
    /// swap that re-resolves font families, or a DPR change.
    pub fn invalidate_caches(&mut self) {
        self.text.invalidate();
        #[cfg(feature = "gpui-backend")]
        self.shaped_text.invalidate();
    }

    /// Lower a bare `Prim` layer into the scene plan, with no frame or clip around it.
    ///
    /// The frame path ([`GpuiChartRenderer::plan_frame`]) is what a host uses. This is for
    /// fixtures and diagnostics that need one primitive list rendered exactly as given — notably
    /// the pixel-parity harness, which must compare the *same* prim list across backends.
    pub fn plan_prims(&mut self, prims: &[Prim], points: &[[f32; 2]]) -> GpuiFrameMetrics {
        let started = std::time::Instant::now();
        let mut metrics = GpuiFrameMetrics::default();
        self.plan.clear();
        self.text.reset_counters();

        executor::execute_layer(
            prims,
            points,
            self.options,
            &mut self.scratch,
            &mut self.plan,
            &mut metrics,
        );
        metrics.mesh_vertices = self.plan.vertices.len() as u32;
        metrics.plan_nanos = started.elapsed().as_nanos() as u64;
        metrics
    }

    /// Lower a prepared frame into the scene plan **without** touching GPUI.
    ///
    /// This is the whole executor: the same call `paint_frame` makes before handing the plan to
    /// GPUI. Available with GPUI disabled, which is what makes ordering, clipping, geometry and
    /// snapping testable on any machine.
    pub fn plan_frame(
        &mut self,
        prepared: &PreparedAerisFrame<'_>,
        scale_factor: f32,
    ) -> Result<GpuiFrameMetrics, GpuiRenderError> {
        if !scale_factor.is_finite() || scale_factor <= 0.0 {
            return Err(GpuiRenderError::InvalidScaleFactor(scale_factor));
        }
        let ratio = prepared.frame.pixel_ratio;
        if (ratio - scale_factor as f64).abs() > SCALE_EPSILON {
            return Err(GpuiRenderError::ScaleFactorMismatch {
                frame: ratio,
                window: scale_factor,
            });
        }

        let started = std::time::Instant::now();
        let mut metrics = GpuiFrameMetrics::default();
        self.plan.clear();
        self.text.reset_counters();

        // One clipped group per stacked pane, then the unscissored axis layer — the exact
        // composition order the WebGPU host uses (`aeris_charts_wasm::chart::inner_render`).
        for pane in &prepared.frame.panes {
            let [x, y, w, h] = pane.scissor;
            let clip = DeviceRect::new(x as f32, y as f32, w as f32, h as f32);
            let clipped = executor::push_clip(&mut self.plan, &mut metrics, clip);
            for layer in [&pane.under, &pane.main, &pane.top_prims] {
                executor::execute_layer(
                    layer,
                    &pane.points,
                    self.options,
                    &mut self.scratch,
                    &mut self.plan,
                    &mut metrics,
                );
            }
            if clipped {
                executor::pop_clip(&mut self.plan, &mut metrics);
            }
        }
        executor::execute_layer(
            prepared.axis_prims,
            prepared.axis_points,
            self.options,
            &mut self.scratch,
            &mut self.plan,
            &mut metrics,
        );

        metrics.mesh_vertices = self.plan.vertices.len() as u32;
        metrics.plan_nanos = started.elapsed().as_nanos() as u64;
        Ok(metrics)
    }
}

/// Convert a device-px x coordinate to the GPUI logical px the window expects.
///
/// GPUI multiplies by the scale factor again inside `paint_quad`/`paint_path`, so this is the
/// inverse of that step: `device / scale_factor * scale_factor` recovers the device coordinate to
/// within f32 rounding (~1e-5 px at chart scale), far below one pixel of coverage.
pub fn to_logical_x(device_x: f32, viewport: AerisViewport, scale_factor: f32) -> f32 {
    viewport.offset_x + device_x / scale_factor
}

/// Convert a device-px y coordinate to GPUI logical px. See [`to_logical_x`].
pub fn to_logical_y(device_y: f32, viewport: AerisViewport, scale_factor: f32) -> f32 {
    viewport.offset_y + device_y / scale_factor
}

/// Convert a device-px length to GPUI logical px (no offset offset).
pub fn to_logical_len(device_len: f32, scale_factor: f32) -> f32 {
    device_len / scale_factor
}

#[cfg(test)]
mod tests {
    use super::*;
    use aeris_charts_engine::FramePane;
    use aeris_charts_render::color::Color;
    use aeris_charts_render::draw_list::IRect;

    const C: Color = Color::rgb(0x10, 0x20, 0x30);

    fn frame_with(panes: Vec<FramePane>, pixel_ratio: f64) -> ChartFrame {
        ChartFrame {
            width: 200.0,
            height: 100.0,
            pixel_ratio,
            panes,
        }
    }

    #[test]
    fn prepared_frame_resolves_surface_paint_from_aeris_options() {
        let mut engine = ChartEngine::new(200.0, 100.0, 1.0);
        let frame = ChartFrame::default();
        let surface = aeris_charts_core::style::DEFAULT_SURFACE_RGB;
        assert_eq!(
            PreparedAerisFrame::from_engine(&frame, &engine).background,
            Paint::Solid(Color::rgb(surface.0, surface.1, surface.2))
        );

        engine
            .options
            .apply_str(
                r##"{"layout":{"background":{"type":"gradient","topColor":"#010203","bottomColor":"#040506"}}}"##,
            )
            .unwrap();
        assert_eq!(
            PreparedAerisFrame::from_engine(&frame, &engine).background,
            Paint::VGradient {
                top: Color::rgb(1, 2, 3),
                bottom: Color::rgb(4, 5, 6),
            }
        );

        engine
            .options
            .apply_str(r#"{"layout":{"background":{"type":"solid","color":"not-a-color"}}}"#)
            .unwrap();
        assert_eq!(
            PreparedAerisFrame::from_engine(&frame, &engine).background,
            Paint::Solid(Color::rgb(surface.0, surface.1, surface.2))
        );
    }

    fn pane(scissor: [u32; 4], main: Vec<Prim>) -> FramePane {
        FramePane {
            top: 0.0,
            height: 100.0,
            scissor,
            under: Vec::new(),
            main,
            top_prims: Vec::new(),
            series_paint_marks: Vec::new(),
            points: Vec::new(),
        }
    }

    fn rect(x: i32) -> Prim {
        Prim::Rect {
            rect: IRect {
                x,
                y: 0,
                w: 5,
                h: 5,
            },
            color: C,
        }
    }

    #[test]
    fn scale_factor_must_match_the_frames_pixel_ratio() {
        let frame = frame_with(vec![pane([0, 0, 200, 100], vec![rect(0)])], 1.5);
        let prepared = PreparedAerisFrame::new(&frame);
        let mut r = GpuiChartRenderer::new();

        assert!(r.plan_frame(&prepared, 1.5).is_ok());
        assert_eq!(
            r.plan_frame(&prepared, 2.0),
            Err(GpuiRenderError::ScaleFactorMismatch {
                frame: 1.5,
                window: 2.0
            })
        );
    }

    #[test]
    fn nonsense_scale_factors_are_rejected() {
        let frame = frame_with(vec![], 1.0);
        let prepared = PreparedAerisFrame::new(&frame);
        let mut r = GpuiChartRenderer::new();
        for bad in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            assert!(
                matches!(
                    r.plan_frame(&prepared, bad),
                    Err(GpuiRenderError::InvalidScaleFactor(_))
                ),
                "scale factor {bad} should have been rejected"
            );
        }
    }

    #[test]
    fn f32_f64_rounding_of_a_fractional_dpr_is_tolerated() {
        // 1.25 and 2.5 are exact in binary; 1.1 is not — the f32->f64 widening must still pass.
        let frame = frame_with(vec![], 1.100000023841858);
        let prepared = PreparedAerisFrame::new(&frame);
        let mut r = GpuiChartRenderer::new();
        assert!(r.plan_frame(&prepared, 1.1).is_ok());
    }

    #[test]
    fn each_pane_is_wrapped_in_its_own_clip() {
        let frame = frame_with(
            vec![
                pane([0, 0, 200, 60], vec![rect(0)]),
                pane([0, 60, 200, 40], vec![rect(10)]),
            ],
            1.0,
        );
        let prepared = PreparedAerisFrame::new(&frame);
        let mut r = GpuiChartRenderer::new();
        let metrics = r.plan_frame(&prepared, 1.0).unwrap();

        assert_eq!(metrics.clips, 2);
        let kinds: Vec<&str> = r
            .plan()
            .ops
            .iter()
            .map(|op| match op {
                SceneOp::PushClip(_) => "push",
                SceneOp::PopClip => "pop",
                SceneOp::Quad { .. } => "quad",
                SceneOp::Mesh { .. } => "mesh",
                SceneOp::Text(_) => "text",
                SceneOp::Image { .. } => "image",
            })
            .collect();
        assert_eq!(
            kinds,
            vec!["push", "quad", "pop", "push", "quad", "pop"],
            "every pane's content must sit inside that pane's clip"
        );
    }

    #[test]
    fn a_degenerate_pane_scissor_pushes_no_clip_and_stays_balanced() {
        let frame = frame_with(vec![pane([0, 0, 0, 0], vec![rect(0)])], 1.0);
        let prepared = PreparedAerisFrame::new(&frame);
        let mut r = GpuiChartRenderer::new();
        let metrics = r.plan_frame(&prepared, 1.0).unwrap();
        assert_eq!(metrics.clips, 0);
        let pushes = r
            .plan()
            .ops
            .iter()
            .filter(|op| matches!(op, SceneOp::PushClip(_)))
            .count();
        let pops = r
            .plan()
            .ops
            .iter()
            .filter(|op| matches!(op, SceneOp::PopClip))
            .count();
        assert_eq!((pushes, pops), (0, 0), "clip ops must stay balanced");
    }

    #[test]
    fn clip_pushes_and_pops_are_always_balanced() {
        let frame = frame_with(
            vec![
                pane([0, 0, 200, 30], vec![rect(0)]),
                pane([0, 0, 0, 30], vec![rect(1)]),
                pane([0, 60, 200, 40], vec![rect(2)]),
            ],
            1.0,
        );
        let prepared = PreparedAerisFrame::new(&frame);
        let mut r = GpuiChartRenderer::new();
        r.plan_frame(&prepared, 1.0).unwrap();

        let mut depth = 0i32;
        for op in &r.plan().ops {
            match op {
                SceneOp::PushClip(_) => depth += 1,
                SceneOp::PopClip => depth -= 1,
                _ => {}
            }
            assert!(depth >= 0, "a pop without a matching push");
        }
        assert_eq!(depth, 0, "unbalanced clip stack");
    }

    #[test]
    fn pane_layers_paint_under_then_main_then_top() {
        let mut p = pane([0, 0, 200, 100], vec![rect(1)]);
        p.under = vec![rect(0)];
        p.top_prims = vec![rect(2)];
        let frame = frame_with(vec![p], 1.0);
        let prepared = PreparedAerisFrame::new(&frame);
        let mut r = GpuiChartRenderer::new();
        r.plan_frame(&prepared, 1.0).unwrap();

        let xs: Vec<f32> = r
            .plan()
            .ops
            .iter()
            .filter_map(|op| match op {
                SceneOp::Quad { rect, .. } => Some(rect.x),
                _ => None,
            })
            .collect();
        assert_eq!(xs, vec![0.0, 1.0, 2.0]);
    }

    #[test]
    fn the_axis_layer_paints_last_and_unclipped() {
        let frame = frame_with(vec![pane([0, 0, 200, 100], vec![rect(0)])], 1.0);
        let axis = vec![rect(50)];
        let prepared = PreparedAerisFrame::new(&frame).with_axis(&axis, &[]);
        let mut r = GpuiChartRenderer::new();
        r.plan_frame(&prepared, 1.0).unwrap();

        let ops = &r.plan().ops;
        assert!(matches!(ops.last(), Some(SceneOp::Quad { rect, .. }) if rect.x == 50.0));
        // The final quad sits after the pane's PopClip, so it is unclipped.
        let last_pop = ops
            .iter()
            .rposition(|op| matches!(op, SceneOp::PopClip))
            .expect("a pane clip was popped");
        assert_eq!(last_pop, ops.len() - 2);
    }

    #[test]
    fn replanning_reuses_the_allocation_and_does_not_accumulate() {
        let frame = frame_with(
            vec![pane([0, 0, 200, 100], (0..20).map(rect).collect())],
            1.0,
        );
        let prepared = PreparedAerisFrame::new(&frame);
        let mut r = GpuiChartRenderer::new();

        let first = r.plan_frame(&prepared, 1.0).unwrap();
        let ops_after_first = r.plan().ops.len();
        let cap = r.plan().ops.capacity();

        let second = r.plan_frame(&prepared, 1.0).unwrap();
        assert_eq!(
            r.plan().ops.len(),
            ops_after_first,
            "ops must not accumulate"
        );
        assert_eq!(first.quads, second.quads);
        assert_eq!(first.prims, second.prims);
        assert!(
            r.plan().ops.capacity() >= cap,
            "the plan's allocation should be reused, not shrunk"
        );
    }

    #[test]
    fn metrics_count_every_prim_including_the_axis_layer() {
        let frame = frame_with(vec![pane([0, 0, 200, 100], vec![rect(0), rect(1)])], 1.0);
        let axis = vec![rect(2)];
        let prepared = PreparedAerisFrame::new(&frame).with_axis(&axis, &[]);
        let mut r = GpuiChartRenderer::new();
        let metrics = r.plan_frame(&prepared, 1.0).unwrap();
        assert_eq!(metrics.prims, 3);
        assert_eq!(metrics.quads, 3);
    }

    #[test]
    fn device_to_logical_round_trips_within_a_hundredth_of_a_pixel() {
        // GPUI multiplies by the scale factor again; the round trip must land back on the device
        // coordinate well inside one pixel of coverage.
        let viewport = AerisViewport::new(0.0, 0.0, 800.0, 600.0);
        for sf in [1.0f32, 1.25, 1.5, 2.0, 2.5] {
            for device in [0.0f32, 1.0, 37.0, 100.0, 999.0, 1919.0] {
                let logical = to_logical_x(device, viewport, sf);
                let back = (logical - viewport.offset_x) * sf;
                assert!(
                    (back - device).abs() < 0.01,
                    "dpr {sf}: {device} -> {logical} -> {back}"
                );
            }
        }
    }

    #[test]
    fn viewport_offset_offsets_the_chart_within_the_window() {
        let viewport = AerisViewport::new(40.0, 12.0, 800.0, 600.0);
        assert_eq!(to_logical_x(0.0, viewport, 2.0), 40.0);
        assert_eq!(to_logical_y(0.0, viewport, 2.0), 12.0);
        assert_eq!(to_logical_x(100.0, viewport, 2.0), 90.0);
        assert_eq!(to_logical_len(100.0, 2.0), 50.0);
    }

    #[test]
    fn invalidate_caches_bumps_the_text_generation() {
        let mut r = GpuiChartRenderer::new();
        let before = r.text_cache().generation();
        r.invalidate_caches();
        assert_eq!(r.text_cache().generation(), before + 1);
    }

    #[test]
    fn error_messages_name_the_actual_mismatch() {
        let e = GpuiRenderError::ScaleFactorMismatch {
            frame: 1.5,
            window: 2.0,
        };
        let msg = e.to_string();
        assert!(msg.contains("1.5") && msg.contains('2'), "{msg}");
    }
}
