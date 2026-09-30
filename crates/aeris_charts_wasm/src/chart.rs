//! The chart object exported to JS.
//!
//! Shared-frame rendering for the browser host:
//! - pane geometry and engine chrome (watermark, axes, and crosshair labels) are emitted as
//!   backend-neutral primitives;
//! - WebGPU consumes every engine primitive in one render pass, with a final unscissored top-layer
//!   group, while Canvas2D executes the same retained frame as the fallback and screenshot path;
//! - the transparent DOM overlay remains an input surface and a compatibility escape hatch only
//!   for plugin `text_views` that cannot yet enter the shared frame.
//!
//! Axis text is browser-rasterized into the shared WebGPU atlas. At fractional DPR, browser font
//! hinting and WebGPU's 4x-MSAA rounded-label coverage can leave bounded antialiasing differences
//! from direct Canvas2D; all geometry, placement, colors, and paint order stay shared.
//!
//! Multiple series share one time axis via [`DataLayer`] (the merged time-point list). Each
//! series maps its data onto merged indices; a series absent at an index is whitespace there.

mod custom_series;
mod depth;
mod feature_series;
mod footprint;
mod general_charts;
mod image_runs;
mod inner_api;
mod inner_render;
mod native_primitives;
mod primitives;
mod ring;
mod series_update;
mod sessions;
mod text_runs;

use custom_series::CustomSeriesEntry;
use ring::{BoundRing, RingLayoutInput};
use text_runs::TextRunStore;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{
    atomic::{AtomicBool, AtomicU32, Ordering},
    Arc,
};

use js_sys::Float64Array;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::CanvasRenderingContext2d;

use crate::backend_policy::{
    surface_error_action, BackendStartupFailure, BackendStatus, BackendWarningDeduplicator,
    SurfaceErrorAction,
};
use crate::telemetry::{FrameTelemetry, FRAME_STATS_LEN};
use aeris_charts_core::model::data_layer::SeriesId;
use aeris_charts_core::model::data_validation::sanitize_ohlc;
use aeris_charts_core::model::plot_list::MismatchDirection;
use aeris_charts_core::options::{ChartOptions, ChartTheme};
use aeris_charts_core::scale::price_scale_core::PriceScaleMode;
use aeris_charts_engine::{
    crosshair_mode_from_u8, line_style_from_u8, marker_pos, marker_shape, AccountId, AlertId,
    AlertLine, AlertSnapshot, AxisFrame, AxisLabel, AxisLabelCorners, AxisTextAlign,
    AxisTextMidpoint, BrushRange, BrushStyle, ChartEngine, DrawingKind, DrawingModifiers,
    ExecutionId, FeatureSeriesKind, GestureResolver, GestureUpdate, InputDevice, InputModifiers,
    InputTarget, InstrumentMetadata, Marker, OrderId, PaneId, PointerSample, PositionId,
    PriceFormatterFn, PriceScaleId, PriceScaleSide, PriceScaleTarget,
    PrimitiveAutoscaleContribution, SeriesKind, TickMarkFormatterFn, TimeFormatterFn,
    TradingExecution, TradingPosition, TradingSnapshot, TradingStyleOptions, WorkingOrder,
};
use aeris_charts_render::canvas2d::{
    execute as execute_canvas2d, Canvas2d, Viewport as CanvasViewport,
};
use aeris_charts_render::color::Color;
use aeris_charts_render::draw_list::{LineType, Prim};
use aeris_charts_render_wgpu::{
    prims_to_group, render_frame, DrawGroup, FrameResources, GpuTimer, LabelAtlas, MsaaTarget,
    QuadRenderer, TexQuadRenderer, TriRenderer, SAMPLE_COUNT,
};

#[wasm_bindgen(inline_js = r#"
export function notify_aeris_charts_backend_loss(generation) {
    globalThis.dispatchEvent(new CustomEvent('aeris_charts-chart-backend-lost', { detail: generation }));
}
"#)]
extern "C" {
    fn notify_aeris_charts_backend_loss(generation: u32);
}

static GPU_LOSS_GENERATION: AtomicU32 = AtomicU32::new(0);

thread_local! {
    static BACKEND_WARNINGS: RefCell<BackendWarningDeduplicator> = RefCell::default();
}

fn browser_gpu_capabilities() -> (Option<bool>, Option<bool>) {
    let global = js_sys::global();
    let secure_context = js_sys::Reflect::get(&global, &"isSecureContext".into())
        .ok()
        .and_then(|value| value.as_bool());
    let navigator_gpu = js_sys::Reflect::get(&global, &"navigator".into())
        .ok()
        .filter(|navigator| !navigator.is_null() && !navigator.is_undefined())
        .and_then(|navigator| js_sys::Reflect::get(&navigator, &"gpu".into()).ok())
        .map(|gpu| !gpu.is_null() && !gpu.is_undefined());
    (secure_context, navigator_gpu)
}

fn warn_backend_fallback(status: &BackendStatus) {
    if !BACKEND_WARNINGS.with(|warnings| warnings.borrow_mut().should_warn(status)) {
        return;
    }
    let detail = status
        .detail
        .as_deref()
        .map_or(String::new(), |detail| format!("; detail={detail}"));
    web_sys::console::warn_1(
        &format!(
            "aeris_charts: WebGPU fallback stage={} reason={} secure_context={:?} navigator_gpu={:?}; using Canvas2D{detail}",
            status.stage, status.reason, status.secure_context, status.navigator_gpu
        )
        .into(),
    );
}

fn validation_diagnostics_json(
    report: &aeris_charts_core::model::data_validation::ValidationReport,
) -> Option<String> {
    (!report.is_clean()).then(|| {
        let status = if report.accepted == 0 && report.dropped_invalid > 0 {
            "rejected"
        } else {
            "accepted_with_diagnostics"
        };
        serde_json::json!({
            "status": status,
            "accepted": report.accepted,
            "dropped_invalid": report.dropped_invalid,
            "dropped_non_finite": report.dropped_non_finite,
            "dropped_out_of_range": report.dropped_out_of_range,
            "deduplicated": report.dropped_duplicate,
            "reordered": report.reordered,
            "semantic_anomalies": report.semantic_anomalies,
        })
        .to_string()
    })
}

fn rejected_diagnostics_json(reason: impl core::fmt::Display) -> String {
    serde_json::json!({
        "status": "rejected",
        "accepted": 0,
        "dropped_invalid": 0,
        "dropped_non_finite": 0,
        "dropped_out_of_range": 0,
        "deduplicated": 0,
        "reordered": false,
        "semantic_anomalies": 0,
        "reason": reason.to_string(),
    })
    .to_string()
}

/// Rejected ingestion for a host write to an engine-derived series (footprint, synthetic, or
/// resampled bars), which changes only through its source.
fn derived_series_rejection(id: u32) -> String {
    rejected_diagnostics_json(format_args!(
        "series {id} is derived by the engine; write to its source instead"
    ))
}

fn rejected_validation_diagnostics_json(
    error: aeris_charts_core::model::data_validation::ValidationError,
) -> String {
    use aeris_charts_core::model::data_validation::{TimestampErrorCategory, ValidationError};

    let (dropped_invalid, dropped_non_finite, dropped_out_of_range) = match &error {
        ValidationError::InvalidTimestamp { error, .. } => (
            1,
            usize::from(error.category == TimestampErrorCategory::NonFinite),
            usize::from(error.category == TimestampErrorCategory::OutOfRange),
        ),
        _ => (0, 0, 0),
    };
    serde_json::json!({
        "status": "rejected",
        "accepted": 0,
        "dropped_invalid": dropped_invalid,
        "dropped_non_finite": dropped_non_finite,
        "dropped_out_of_range": dropped_out_of_range,
        "deduplicated": 0,
        "reordered": false,
        "semantic_anomalies": 0,
        "reason": error.to_string(),
    })
    .to_string()
}

fn trading_result_json(result: Result<(), aeris_charts_engine::ChartError>) -> String {
    match result {
        Ok(()) => r#"{"ok":true}"#.to_string(),
        Err(error) => serde_json::json!({
            "ok": false,
            "error": {
                "code": error.code().name(),
                "message": error.message(),
            }
        })
        .to_string(),
    }
}

const INPUT_UPDATE_LEN: usize = 12;

fn input_device_from_u8(value: u8) -> InputDevice {
    match value {
        1 => InputDevice::Touch,
        2 => InputDevice::Pen,
        _ => InputDevice::Mouse,
    }
}

fn input_target_from_u8(value: u8) -> InputTarget {
    match value {
        1 => InputTarget::Drawing,
        2 => InputTarget::Trading,
        3 => InputTarget::PriceAxis,
        4 => InputTarget::TimeAxis,
        5 => InputTarget::Separator,
        6 => InputTarget::Alert,
        _ => InputTarget::Pane,
    }
}

#[allow(clippy::too_many_arguments)]
fn pointer_sample(
    id: u32,
    device: u8,
    target: u8,
    modifiers: u8,
    x: f64,
    y: f64,
    timestamp_ms: f64,
    pressure: f64,
    tilt_x: f64,
    tilt_y: f64,
) -> PointerSample {
    PointerSample {
        id,
        device: input_device_from_u8(device),
        target: input_target_from_u8(target),
        modifiers: InputModifiers {
            shift: modifiers & 1 != 0,
            control: modifiers & 2 != 0,
            alt: modifiers & 4 != 0,
            meta: modifiers & 8 != 0,
        },
        x,
        y,
        timestamp_ms,
        pressure,
        tilt_x,
        tilt_y,
    }
}

fn write_input_update(out: &mut [f64], update: GestureUpdate) -> bool {
    if out.len() < INPUT_UPDATE_LEN {
        return false;
    }
    out[..INPUT_UPDATE_LEN].copy_from_slice(&[
        update.kind as u8 as f64,
        update.state as u8 as f64,
        f64::from(update.pointer_id),
        update.target as u8 as f64,
        update.device as u8 as f64,
        update.x,
        update.y,
        update.previous_x,
        update.previous_y,
        update.scale_delta,
        f64::from(update.active_pointers),
        if update.prevent_default { 1.0 } else { 0.0 },
    ]);
    true
}

fn broadcast_gpu_loss() {
    let generation = GPU_LOSS_GENERATION.fetch_add(1, Ordering::AcqRel) + 1;
    notify_aeris_charts_backend_loss(generation);
}

// Aeris's canonical default palette
// Axis palette (as CSS color strings for the 2D overlay)
// industry-standard volume: translucent green on up bars, red on down bars.

// Crosshair marker (line/area) — line-series.ts defaults.

/// JSON shape accepted from the JS boundary for `set_series_markers`.
#[derive(serde::Deserialize)]
struct MarkerInput {
    time: f64,
    #[serde(default)]
    position: String,
    #[serde(default)]
    shape: String,
    #[serde(default)]
    color: String,
    #[serde(default)]
    text: String,
    #[serde(default)]
    id: String,
    #[serde(default = "default_marker_size")]
    size: f64,
    #[serde(default)]
    price: Option<f64>,
}

fn default_marker_size() -> f64 {
    1.0
}

fn price_scale_mode_from_u8(mode: u8) -> PriceScaleMode {
    match mode {
        1 => PriceScaleMode::Logarithmic,
        2 => PriceScaleMode::Percentage,
        3 => PriceScaleMode::IndexedTo100,
        _ => PriceScaleMode::Normal,
    }
}

fn price_scale_mode_to_u8(mode: PriceScaleMode) -> u8 {
    match mode {
        PriceScaleMode::Normal => 0,
        PriceScaleMode::Logarithmic => 1,
        PriceScaleMode::Percentage => 2,
        PriceScaleMode::IndexedTo100 => 3,
    }
}

fn price_scale_target_from_u32(target: u32) -> PriceScaleTarget {
    match target {
        1 => PriceScaleTarget::Left,
        2 => PriceScaleTarget::Overlay,
        3.. => PriceScaleId::try_from(target - 2)
            .map(PriceScaleTarget::Named)
            .unwrap_or(PriceScaleTarget::Right),
        _ => PriceScaleTarget::Right,
    }
}

fn price_scale_target_to_u32(target: PriceScaleTarget) -> u32 {
    match target {
        PriceScaleTarget::Right => 0,
        PriceScaleTarget::Left => 1,
        PriceScaleTarget::Overlay => 2,
        PriceScaleTarget::Named(id) => id.get() + 2,
    }
}

fn mismatch_direction_from_i8(direction: i8) -> MismatchDirection {
    match direction {
        -1 => MismatchDirection::NearestLeft,
        1 => MismatchDirection::NearestRight,
        _ => MismatchDirection::None,
    }
}

/// Renderers shared across charts for one surface format. Pipelines and shader modules are
/// the expensive part of device setup; they depend only on the (shared) device and format.
struct FormatRenderers {
    quad: QuadRenderer,
    tri: TriRenderer,
    tex: TexQuadRenderer,
    rotated_tex: TexQuadRenderer,
    image: TexQuadRenderer,
}

/// One GPU context shared by every chart instance in the page:
/// a single adapter/device/queue, one label atlas (glyphs cached across charts), and a
/// per-format renderer cache. Per-chart state is only the surface, its config, and the
/// size-dependent MSAA target. wasm32 is single-threaded, so `Rc`/`RefCell` suffice.
struct SharedGpu {
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
    atlas: RefCell<LabelAtlas>,
    image_atlas: RefCell<LabelAtlas>,
    renderers: RefCell<std::collections::HashMap<wgpu::TextureFormat, Rc<FormatRenderers>>>,
    device_lost: Arc<AtomicBool>,
}

impl SharedGpu {
    fn renderers_for(&self, format: wgpu::TextureFormat) -> Rc<FormatRenderers> {
        if let Some(renderers) = self.renderers.borrow().get(&format) {
            return Rc::clone(renderers);
        }
        let renderers = Rc::new(FormatRenderers {
            quad: QuadRenderer::new(&self.device, format, SAMPLE_COUNT),
            tri: TriRenderer::new(&self.device, format, SAMPLE_COUNT),
            tex: TexQuadRenderer::new(
                &self.device,
                format,
                self.atlas.borrow().view(),
                SAMPLE_COUNT,
            ),
            rotated_tex: TexQuadRenderer::new_rotated(
                &self.device,
                format,
                self.atlas.borrow().view(),
                SAMPLE_COUNT,
            ),
            image: TexQuadRenderer::new(
                &self.device,
                format,
                self.image_atlas.borrow().view(),
                SAMPLE_COUNT,
            ),
        });
        self.renderers
            .borrow_mut()
            .insert(format, Rc::clone(&renderers));
        renderers
    }
}

struct Gfx {
    shared: Rc<SharedGpu>,
    renderers: Rc<FormatRenderers>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    msaa: MsaaTarget,
    device_lost: Arc<AtomicBool>,
    /// GPU pass timing for `frame_stats().gpu_ms`. Created lazily on the first frame after the
    /// host reads `frame_stats()` (telemetry.rs: nobody pays for a query set they never read),
    /// and stays `None` forever on a device without `timestamp-query`.
    timer: Option<GpuTimer>,
    frame_resources: FrameResources,
}

enum PaneRenderOutcome {
    Presented,
    Timeout,
    Fallback(String),
    Canvas2d,
}

struct ChartInner {
    gfx: Option<Gfx>,
    gpu_pane: Option<web_sys::HtmlCanvasElement>,
    fallback_pane: Option<web_sys::HtmlCanvasElement>,
    pane_ctx: CanvasRenderingContext2d,
    axis_ctx: CanvasRenderingContext2d,
    backend_status: BackendStatus,
    bitmap_w: u32,
    bitmap_h: u32,
    engine: ChartEngine,
    input: GestureResolver,
    frame: aeris_charts_engine::ChartFrame,
    axis_frame: AxisFrame,
    /// Backend-neutral top-layer primitives: watermark, axis chrome, ticks, and all axis/crosshair
    /// labels. WebGPU appends this as an unscissored draw group; Canvas2D executes the same list.
    axis_prims: Vec<Prim>,
    axis_revision: u64,
    axis_dirty: bool,
    gpu_groups: Vec<DrawGroup>,
    gpu_atlas_epoch: u64,
    gpu_image_atlas_epoch: u64,
    /// Pane-primitive registry (plugin platform Phase C-a): host-retained JS plugin objects,
    /// drawn into the pane layers during `render`. Ids are never reused within a chart.
    primitives: Vec<PanePrimitiveEntry>,
    /// Series-primitive registry (plugin platform Phase C-b): same retained-object model as
    /// the pane registry, but each entry is bound to an owning series — its views resolve
    /// against that series' price scale, and removing the series auto-detaches them.
    /// Shares `next_primitive_id` with the pane registry so ids stay unique within a chart.
    series_primitives: Vec<SeriesPrimitiveEntry>,
    next_primitive_id: u32,
    /// Custom-series registry (plugin platform Phase C-c): one retained pane-view plugin
    /// object plus its raw items per custom series, aligned with the engine's time-only
    /// rows. Removing the series drops the entry (firing the view's `destroy` hook).
    custom_series: Vec<CustomSeriesEntry>,
    /// Overlay text draws collected from primitives' `text_views` hooks during primitive passes.
    /// The legacy Canvas2D compatibility overlay sits above the backend surface, so every draw is
    /// clipped to its owning pane and cannot cover shared price/time-axis chrome.
    primitive_texts: Vec<PrimitiveOverlayText>,
    /// Whether the transparent input overlay contains plugin text from the previous frame. Normal
    /// WebGPU frames never touch this Canvas2D surface; it is cleared only on plugin detach.
    overlay_had_plugin_text: bool,
    /// Browser-rasterized text-run store for `Prim::Text` on the WebGPU backend (offscreen
    /// canvas + atlas cache). `None` only if the offscreen context could not be created —
    /// the Canvas2D backend draws text directly and never consults this.
    text_runs: Option<TextRunStore>,
    /// Canvas2D fallback resources for immutable engine raster-image primitives.
    canvas_images: RefCell<crate::canvas2d_target::CanvasImageStore>,
    /// Host-pinned clock (UTC seconds) for the candle-close countdown labels. `None` = the
    /// render path feeds the browser's system time every frame; `set_now_seconds` pins a value
    /// (the package's 1s countdown timer), which then drives every render until replaced.
    now_override: Option<f64>,
    /// `SharedArrayBuffer` ring data sources (`series_api.set_ring_source`), at most one per
    /// series, drained once per frame tick by `drain_ring_sources`.
    rings: Vec<BoundRing>,
    /// Rolling last-frame telemetry behind `chart_api.frame_stats()`.
    telemetry: FrameTelemetry,
    /// The page's high-resolution clock, resolved once (`None` = no `performance` global, so
    /// `frame_stats().cpu_ms` stays 0 rather than costing a failed lookup per frame).
    clock: Option<web_sys::Performance>,
}

/// One in-pane overlay text draw registered by a primitive's `text_views` hook (plugin
/// platform Phase 3.5). Painted through the legacy Canvas2D compatibility overlay and clipped to
/// its owning pane, so it remains above pane geometry without reaching shared axis chrome. `font`
/// is a fully-resolved CSS font shorthand; `align`/`baseline` are canvas keywords.
pub(super) struct PrimitiveOverlayText {
    pub(super) text: String,
    pub(super) x: f64,
    pub(super) y: f64,
    /// Owning pane in media coordinates. The compatibility overlay sits above the backend canvas,
    /// so clipping is what preserves the historical guarantee that plugin text cannot cover axes.
    pub(super) clip: [f64; 4],
    pub(super) color: String,
    pub(super) font: String,
    pub(super) align: String,
    pub(super) baseline: String,
}

/// One attached pane primitive (reference `IPanePrimitive`, adapted to a plain JS object by the TS
/// package): the pane index it draws on plus the retained object itself.
struct PanePrimitiveEntry {
    id: u32,
    pane: u32,
    obj: js_sys::Object,
}

/// One attached series primitive (reference `ISeriesPrimitive`, Phase C-b): the owning series id
/// plus the retained object. The pane/scale binding is re-resolved from the series each frame,
/// so the views follow the series across pane moves and scale rebinding.
struct SeriesPrimitiveEntry {
    id: u32,
    series: u32,
    obj: js_sys::Object,
}
impl std::ops::Deref for ChartInner {
    type Target = ChartEngine;

    fn deref(&self) -> &Self::Target {
        &self.engine
    }
}

impl std::ops::DerefMut for ChartInner {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.engine
    }
}

/// Keeps the `ResizeObserver` and its callback alive for the chart's lifetime.
struct ResizeBinding {
    observer: web_sys::ResizeObserver,
    _callback: Closure<dyn FnMut(js_sys::Array)>,
}

impl Drop for ResizeBinding {
    fn drop(&mut self) {
        self.observer.disconnect();
    }
}

/// The chart handle exported to JS. Wraps [`ChartInner`] in `Rc<RefCell<..>>` so an
/// engine-owned `ResizeObserver` callback can mutate it, and holds the canvas elements so
/// the engine can size their backing stores itself. Public methods delegate to the inner.
#[wasm_bindgen]
pub struct AerisChart {
    inner: Rc<RefCell<ChartInner>>,
    gpu_pane: Option<web_sys::HtmlCanvasElement>,
    fallback_pane: Option<web_sys::HtmlCanvasElement>,
    overlay: Option<web_sys::HtmlCanvasElement>,
    _resize: Option<ResizeBinding>,
    disposed: bool,
}

/// Reads the exact physical-pixel size of a `ResizeObserverEntry`'s device-pixel content box.
/// This is the crisp-rendering crux: `round(cssSize * devicePixelRatio)` only approximates the
/// element's true physical footprint, so at fractional ratios (e.g. 150% scaling) the backing
/// store no longer maps 1:1 to device pixels and the compositor resamples the bitmap — soft,
/// "thicker" 1px wicks. `devicePixelContentBoxSize` is the exact integer count. Returns `None`
/// when the browser lacks the API (Safari < 16.4), so the caller can fall back to the approx.
fn device_pixel_box(entry: &web_sys::ResizeObserverEntry) -> Option<(f64, f64)> {
    // Read the property reflectively: on WebKit it is absent, and the typed getter would yield an
    // `undefined` that panics when indexed. Reflect returns a plain `undefined` JsValue instead,
    // which is not an `Array`, so we cleanly fall back to `None`.
    let value = js_sys::Reflect::get(entry, &"devicePixelContentBoxSize".into()).ok()?;
    let arr = value.dyn_ref::<js_sys::Array>()?;
    let first = arr.get(0);
    if first.is_undefined() {
        return None;
    }
    let size = first.dyn_into::<web_sys::ResizeObserverSize>().ok()?;
    Some((size.inline_size(), size.block_size()))
}

/// Whether this engine exposes `ResizeObserverEntry.devicePixelContentBoxSize` (Chromium/Firefox,
/// not Safari/WebKit). Observing with the `device-pixel-content-box` option *throws* on engines
/// that lack it, so we feature-detect and fall back to a plain content-box observation there.
fn supports_device_pixel_content_box() -> bool {
    let Some(window) = web_sys::window() else {
        return false;
    };
    let Ok(ctor) = js_sys::Reflect::get(&window, &"ResizeObserverEntry".into()) else {
        return false;
    };
    if ctor.is_undefined() {
        return false;
    }
    let Ok(proto) = js_sys::Reflect::get(&ctor, &"prototype".into()) else {
        return false;
    };
    js_sys::Reflect::has(&proto, &"devicePixelContentBoxSize".into()).unwrap_or(false)
}

fn set_backend_visibility(
    gpu_pane: Option<&web_sys::HtmlCanvasElement>,
    fallback_pane: Option<&web_sys::HtmlCanvasElement>,
    use_webgpu: bool,
) {
    let Some((gpu_pane, fallback_pane)) = gpu_pane.zip(fallback_pane) else {
        return;
    };
    let _ = gpu_pane
        .style()
        .set_property("visibility", if use_webgpu { "visible" } else { "hidden" });
    let _ = fallback_pane
        .style()
        .set_property("visibility", if use_webgpu { "hidden" } else { "visible" });
}

/// Sizes all three canvases to `(bw, bh)` device pixels while pinning their CSS box to the real
/// displayed size, then resizes + repaints the engine. Shared by the initial bind and every
/// observer callback.
#[allow(clippy::too_many_arguments)] // three canvases + the full size/DPR tuple
fn apply_device_size(
    inner: &Rc<RefCell<ChartInner>>,
    gpu_pane: &web_sys::HtmlCanvasElement,
    fallback_pane: &web_sys::HtmlCanvasElement,
    overlay: &web_sys::HtmlCanvasElement,
    css_w: f64,
    css_h: f64,
    bw: f64,
    bh: f64,
) {
    let (bw_u, bh_u) = (bw.max(1.0) as u32, bh.max(1.0) as u32);
    for c in [gpu_pane, fallback_pane, overlay] {
        c.set_width(bw_u);
        c.set_height(bh_u);
        let style = c.style();
        let _ = style.set_property("width", &format!("{css_w}px"));
        let _ = style.set_property("height", &format!("{css_h}px"));
    }
    // Exact effective ratio -> the engine's internal round(css*dpr) lands back on (bw, bh),
    // so surface, canvas backing store and physical pixels all agree.
    let dpr = bw / css_w.max(1.0);
    let mut c = inner.borrow_mut();
    c.resize(css_w.max(1.0), css_h.max(1.0), dpr);
    let _ = c.render();
}

/// Creates a chart bound to dedicated WebGPU and Canvas2D pane canvases plus an axis/text overlay.
/// All three must be full chart size with bitmap size = css size * dpr, already set by the caller.
/// Call [`AerisChart::enable_auto_resize`] to have the engine own sizing from then on.
#[allow(clippy::too_many_arguments)] // public JS entry point: three canvases + size/DPR/backend
#[wasm_bindgen]
pub async fn create_chart(
    gpu_pane_canvas: web_sys::HtmlCanvasElement,
    fallback_pane_canvas: web_sys::HtmlCanvasElement,
    overlay_canvas: web_sys::HtmlCanvasElement,
    css_width: f64,
    css_height: f64,
    dpr: f64,
    force_canvas2d: bool,
    simulate_adapter_failure: bool,
    force_fallback_adapter: bool,
    initial_horizontal_domain_json: String,
) -> Result<AerisChart, JsValue> {
    console_error_panic_hook::set_once();
    let initial_horizontal_domain =
        general_charts::parse_initial_horizontal_domain(&initial_horizontal_domain_json)
            .map_err(|error| JsValue::from_str(error.message()))?;

    // Keep handles to all canvas elements so the engine can own device-pixel resizing
    // (create_surface takes the pane canvas by value; the clone is just a JS reference).
    let gpu_pane_el = gpu_pane_canvas.clone();
    let fallback_pane_el = fallback_pane_canvas.clone();
    let overlay_el = overlay_canvas.clone();

    let axis_ctx = overlay_canvas
        .get_context("2d")?
        .ok_or_else(|| JsValue::from_str("no 2d context"))?
        .dyn_into::<CanvasRenderingContext2d>()?;

    let bitmap_w = (css_width * dpr).round().max(1.0) as u32;
    let bitmap_h = (css_height * dpr).round().max(1.0) as u32;
    let (secure_context, navigator_gpu) = browser_gpu_capabilities();
    // A canvas cannot change context type after WebGPU has claimed it. Keep a dedicated 2D pane
    // warm from construction so a device loss can switch backends without replacing DOM nodes or
    // rebuilding chart state.
    let pane_ctx = fallback_pane_el
        .get_context("2d")?
        .ok_or_else(|| JsValue::from_str("no 2d pane context"))?
        .dyn_into::<CanvasRenderingContext2d>()?;
    let (gfx, backend_status) = if force_canvas2d {
        (
            None,
            BackendStatus::canvas2d_requested(secure_context, navigator_gpu),
        )
    } else {
        match try_create_gfx(
            wgpu::SurfaceTarget::Canvas(gpu_pane_canvas),
            css_width,
            css_height,
            dpr,
            simulate_adapter_failure,
            force_fallback_adapter,
        )
        .await
        {
            Ok(gfx) => (
                Some(gfx),
                BackendStatus::webgpu_ready(secure_context, navigator_gpu),
            ),
            Err(error) => {
                let status = BackendStatus::startup_fallback(error, secure_context, navigator_gpu);
                warn_backend_fallback(&status);
                (None, status)
            }
        }
    };
    set_backend_visibility(Some(&gpu_pane_el), Some(&fallback_pane_el), gfx.is_some());

    let mut inner = ChartInner {
        gfx,
        gpu_pane: Some(gpu_pane_el.clone()),
        fallback_pane: Some(fallback_pane_el.clone()),
        pane_ctx,
        axis_ctx,
        backend_status,
        bitmap_w,
        bitmap_h,
        engine: ChartEngine::new_with_initial_domain(
            css_width,
            css_height,
            dpr,
            initial_horizontal_domain,
        )
        .map_err(|error| JsValue::from_str(error.message()))?,
        input: GestureResolver::default(),
        frame: aeris_charts_engine::ChartFrame::default(),
        axis_frame: AxisFrame::default(),
        axis_prims: Vec::new(),
        axis_revision: 0,
        axis_dirty: true,
        gpu_groups: Vec::new(),
        gpu_atlas_epoch: 0,
        gpu_image_atlas_epoch: 0,
        primitives: Vec::new(),
        series_primitives: Vec::new(),
        next_primitive_id: 1,
        custom_series: Vec::new(),
        primitive_texts: Vec::new(),
        overlay_had_plugin_text: false,
        now_override: None,
        rings: Vec::new(),
        telemetry: FrameTelemetry::default(),
        clock: crate::telemetry::performance(),
        text_runs: match TextRunStore::new() {
            Ok(store) => Some(store),
            Err(error) => {
                web_sys::console::warn_1(
                    &format!(
                        "aeris_charts: text-run rasterizer unavailable ({error:?}); WebGPU text disabled"
                    )
                    .into(),
                );
                None
            }
        },
        canvas_images: RefCell::default(),
    };
    // Drawing-label hit boxes measure through the same axis canvas the engine's own labels use
    // (drawings.rs `TextMeasureFn` — the host-injected formatter-hook pattern, so the engine
    // stays headless). The font spec matches the `Prim::Text` rasterizer exactly.
    let measure_ctx = inner.axis_ctx.clone();
    inner.engine.set_text_measure(Some(Box::new(
        move |text: &str, size: f64, family: &str, weight: u16, italic: bool| {
            measure_ctx.set_font(&aeris_charts_render::draw_list::text_font_spec(
                size as f32,
                family,
                weight,
                italic,
            ));
            measure_ctx
                .measure_text(text)
                .map(|m| m.width())
                .unwrap_or(0.0)
        },
    )));

    Ok(AerisChart {
        inner: Rc::new(RefCell::new(inner)),
        gpu_pane: Some(gpu_pane_el),
        fallback_pane: Some(fallback_pane_el),
        overlay: Some(overlay_el),
        _resize: None,
        disposed: false,
    })
}

/// Creates a worker-safe chart over transferred offscreen canvases. The first canvas is reserved
/// for WebGPU and the second remains a warm Canvas2D fallback because a canvas cannot switch context
/// types after creation. Width, height, and DPR are explicit; workers have no layout box to infer.
#[allow(clippy::too_many_arguments)]
#[wasm_bindgen]
pub async fn create_offscreen_chart(
    gpu_pane_canvas: web_sys::OffscreenCanvas,
    fallback_pane_canvas: web_sys::OffscreenCanvas,
    css_width: f64,
    css_height: f64,
    dpr: f64,
    force_canvas2d: bool,
    simulate_adapter_failure: bool,
    force_fallback_adapter: bool,
) -> Result<AerisChart, JsValue> {
    console_error_panic_hook::set_once();
    let css_width = css_width.max(1.0);
    let css_height = css_height.max(1.0);
    let dpr = dpr.max(f64::EPSILON);
    let bitmap_w = (css_width * dpr).round().max(1.0) as u32;
    let bitmap_h = (css_height * dpr).round().max(1.0) as u32;
    let (secure_context, navigator_gpu) = browser_gpu_capabilities();
    for canvas in [&gpu_pane_canvas, &fallback_pane_canvas] {
        canvas.set_width(bitmap_w);
        canvas.set_height(bitmap_h);
    }

    // OffscreenCanvasRenderingContext2D exposes the same methods used by the shared executor.
    // web-sys models it as a separate nominal type, so select the compatible method bindings.
    let pane_ctx = fallback_pane_canvas
        .get_context("2d")?
        .ok_or_else(|| JsValue::from_str("no offscreen 2d pane context"))?
        .unchecked_into::<CanvasRenderingContext2d>();
    let measure_canvas = web_sys::OffscreenCanvas::new(bitmap_w, bitmap_h)?;
    let axis_ctx = measure_canvas
        .get_context("2d")?
        .ok_or_else(|| JsValue::from_str("no offscreen 2d measurement context"))?
        .unchecked_into::<CanvasRenderingContext2d>();

    let (gfx, backend_status) = if force_canvas2d {
        (
            None,
            BackendStatus::canvas2d_requested(secure_context, navigator_gpu),
        )
    } else {
        match try_create_gfx(
            wgpu::SurfaceTarget::OffscreenCanvas(gpu_pane_canvas),
            css_width,
            css_height,
            dpr,
            simulate_adapter_failure,
            force_fallback_adapter,
        )
        .await
        {
            Ok(gfx) => (
                Some(gfx),
                BackendStatus::webgpu_ready(secure_context, navigator_gpu),
            ),
            Err(error) => {
                let status = BackendStatus::startup_fallback(error, secure_context, navigator_gpu);
                warn_backend_fallback(&status);
                (None, status)
            }
        }
    };

    let mut inner = ChartInner {
        gfx,
        gpu_pane: None,
        fallback_pane: None,
        pane_ctx,
        axis_ctx,
        backend_status,
        bitmap_w,
        bitmap_h,
        engine: ChartEngine::new(css_width, css_height, dpr),
        input: GestureResolver::default(),
        frame: aeris_charts_engine::ChartFrame::default(),
        axis_frame: AxisFrame::default(),
        axis_prims: Vec::new(),
        axis_revision: 0,
        axis_dirty: true,
        gpu_groups: Vec::new(),
        gpu_atlas_epoch: 0,
        gpu_image_atlas_epoch: 0,
        primitives: Vec::new(),
        series_primitives: Vec::new(),
        next_primitive_id: 1,
        custom_series: Vec::new(),
        primitive_texts: Vec::new(),
        overlay_had_plugin_text: false,
        text_runs: match TextRunStore::new_offscreen() {
            Ok(store) => Some(store),
            Err(error) => {
                web_sys::console::warn_1(
                    &format!(
                        "aeris_charts: offscreen text-run rasterizer unavailable ({error:?}); WebGPU text disabled"
                    )
                    .into(),
                );
                None
            }
        },
        canvas_images: RefCell::default(),
        now_override: None,
        rings: Vec::new(),
        telemetry: FrameTelemetry::default(),
        clock: crate::telemetry::performance(),
    };
    let measure_ctx = inner.axis_ctx.clone();
    inner.engine.set_text_measure(Some(Box::new(
        move |text: &str, size: f64, family: &str, weight: u16, italic: bool| {
            measure_ctx.set_font(&aeris_charts_render::draw_list::text_font_spec(
                size as f32,
                family,
                weight,
                italic,
            ));
            measure_ctx
                .measure_text(text)
                .map(|metrics| metrics.width())
                .unwrap_or(0.0)
        },
    )));

    Ok(AerisChart {
        inner: Rc::new(RefCell::new(inner)),
        gpu_pane: None,
        fallback_pane: None,
        overlay: None,
        _resize: None,
        disposed: false,
    })
}

/// Public JS surface. Sizing is engine-owned once [`enable_auto_resize`] is called; the rest
/// delegate straight through to the inner chart.
#[wasm_bindgen]
impl AerisChart {
    /// Replace the chart-local, host-authoritative alert indicators transactionally.
    pub fn set_alert_snapshot_json(&mut self, snapshot_json: &str) -> String {
        let snapshot = match serde_json::from_str::<AlertSnapshot>(snapshot_json) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                return trading_result_json(Err(aeris_charts_engine::ChartError::new(
                    aeris_charts_engine::ErrorCode::InvalidData,
                    error.to_string(),
                )))
            }
        };
        trading_result_json(self.inner.borrow_mut().engine.set_alert_snapshot(snapshot))
    }

    pub fn alert_snapshot_json(&self) -> String {
        serde_json::to_string(&self.inner.borrow().engine.alert_snapshot())
            .unwrap_or_else(|_| "{}".to_string())
    }

    pub fn update_alert_line_json(&mut self, line_json: &str) -> String {
        let line = match serde_json::from_str::<AlertLine>(line_json) {
            Ok(line) => line,
            Err(error) => {
                return trading_result_json(Err(aeris_charts_engine::ChartError::new(
                    aeris_charts_engine::ErrorCode::InvalidData,
                    error.to_string(),
                )))
            }
        };
        trading_result_json(self.inner.borrow_mut().engine.update_alert_line(line))
    }

    pub fn remove_alert_line(&mut self, id: &str) -> bool {
        AlertId::new(id).is_ok_and(|id| self.inner.borrow_mut().engine.remove_alert_line(&id))
    }

    pub fn set_alert_create_button_visible(&mut self, visible: bool) -> bool {
        self.inner
            .borrow_mut()
            .engine
            .set_alert_create_button_visible(visible)
    }

    pub fn alert_create_hit_at(&self, x_css: f64, y_css: f64) -> bool {
        self.inner.borrow().engine.alert_create_hit_at(x_css, y_css)
    }

    pub fn activate_alert_create_at(&mut self, x_css: f64, y_css: f64) -> bool {
        self.inner
            .borrow_mut()
            .engine
            .activate_alert_create_at(x_css, y_css)
    }

    pub fn take_alert_create_requests_json(&mut self) -> String {
        serde_json::to_string(&self.inner.borrow_mut().engine.take_alert_create_requests())
            .unwrap_or_else(|_| "[]".to_string())
    }

    /// Replace the chart-local, host-authoritative runtime trading state transactionally.
    pub fn set_trading_snapshot_json(&mut self, snapshot_json: &str) -> String {
        let snapshot = match serde_json::from_str::<TradingSnapshot>(snapshot_json) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                return trading_result_json(Err(aeris_charts_engine::ChartError::new(
                    aeris_charts_engine::ErrorCode::InvalidData,
                    error.to_string(),
                )))
            }
        };
        trading_result_json(
            self.inner
                .borrow_mut()
                .engine
                .set_trading_snapshot(snapshot),
        )
    }

    pub fn trading_snapshot_json(&self) -> String {
        serde_json::to_string(&self.inner.borrow().engine.trading_snapshot())
            .unwrap_or_else(|_| "{}".to_string())
    }

    pub fn set_trading_visible_account(&mut self, account_id: Option<String>) -> String {
        let account_id = match account_id {
            Some(value) => match AccountId::new(value) {
                Ok(account_id) => Some(account_id),
                Err(error) => return trading_result_json(Err(error)),
            },
            None => None,
        };
        self.inner
            .borrow_mut()
            .engine
            .set_trading_visible_account(account_id);
        trading_result_json(Ok(()))
    }

    pub fn set_host_overlay_json(&mut self, overlay_json: &str) -> String {
        let overlay =
            match serde_json::from_str::<aeris_charts_engine::HostOverlaySnapshot>(overlay_json) {
                Ok(overlay) => overlay,
                Err(error) => {
                    return trading_result_json(Err(aeris_charts_engine::ChartError::new(
                        aeris_charts_engine::ErrorCode::InvalidData,
                        error.to_string(),
                    )))
                }
            };
        trading_result_json(self.inner.borrow_mut().engine.set_host_overlay(overlay))
    }

    pub fn host_overlay_json(&self) -> String {
        serde_json::to_string(self.inner.borrow().engine.host_overlay())
            .unwrap_or_else(|_| "{}".to_string())
    }

    pub fn host_event_hit_json(&self, x_css: f64, y_css: f64) -> String {
        serde_json::to_string(&self.inner.borrow().engine.host_event_hit_at(x_css, y_css))
            .unwrap_or_else(|_| "null".to_string())
    }

    pub fn update_trading_position_json(&mut self, position_json: &str) -> String {
        let position = match serde_json::from_str::<TradingPosition>(position_json) {
            Ok(position) => position,
            Err(error) => {
                return trading_result_json(Err(aeris_charts_engine::ChartError::new(
                    aeris_charts_engine::ErrorCode::InvalidData,
                    error.to_string(),
                )))
            }
        };
        trading_result_json(
            self.inner
                .borrow_mut()
                .engine
                .update_trading_position(position),
        )
    }

    pub fn remove_trading_position(&mut self, id: &str) -> bool {
        PositionId::new(id)
            .is_ok_and(|id| self.inner.borrow_mut().engine.remove_trading_position(&id))
    }

    pub fn update_working_order_json(&mut self, order_json: &str) -> String {
        let order = match serde_json::from_str::<WorkingOrder>(order_json) {
            Ok(order) => order,
            Err(error) => {
                return trading_result_json(Err(aeris_charts_engine::ChartError::new(
                    aeris_charts_engine::ErrorCode::InvalidData,
                    error.to_string(),
                )))
            }
        };
        trading_result_json(self.inner.borrow_mut().engine.update_working_order(order))
    }

    pub fn remove_working_order(&mut self, id: &str) -> bool {
        OrderId::new(id).is_ok_and(|id| self.inner.borrow_mut().engine.remove_working_order(&id))
    }

    pub fn apply_trading_execution_json(&mut self, execution_json: &str) -> String {
        let execution = match serde_json::from_str::<TradingExecution>(execution_json) {
            Ok(execution) => execution,
            Err(error) => {
                return trading_result_json(Err(aeris_charts_engine::ChartError::new(
                    aeris_charts_engine::ErrorCode::InvalidData,
                    error.to_string(),
                )))
            }
        };
        trading_result_json(
            self.inner
                .borrow_mut()
                .engine
                .apply_trading_execution(execution),
        )
    }

    pub fn remove_trading_execution(&mut self, id: &str) -> bool {
        ExecutionId::new(id)
            .is_ok_and(|id| self.inner.borrow_mut().engine.remove_trading_execution(&id))
    }

    pub fn set_instrument_metadata_json(&mut self, instrument_json: &str) -> String {
        let instrument = match serde_json::from_str::<InstrumentMetadata>(instrument_json) {
            Ok(instrument) => instrument,
            Err(error) => {
                return trading_result_json(Err(aeris_charts_engine::ChartError::new(
                    aeris_charts_engine::ErrorCode::InvalidData,
                    error.to_string(),
                )))
            }
        };
        trading_result_json(
            self.inner
                .borrow_mut()
                .engine
                .set_instrument_metadata(instrument),
        )
    }

    pub fn apply_trading_style_json(&mut self, options_json: &str) -> String {
        let options = match serde_json::from_str::<TradingStyleOptions>(options_json) {
            Ok(options) => options,
            Err(error) => {
                return trading_result_json(Err(aeris_charts_engine::ChartError::new(
                    aeris_charts_engine::ErrorCode::InvalidData,
                    error.to_string(),
                )))
            }
        };
        trading_result_json(self.inner.borrow_mut().engine.apply_trading_style(options))
    }

    pub fn place_bracket_order_from_drawing(&mut self, drawing_id: u32, quantity: f64) -> String {
        trading_result_json(
            self.inner
                .borrow_mut()
                .engine
                .place_bracket_order_from_drawing(drawing_id, quantity),
        )
    }

    pub fn trading_hit_json(&self, x_css: f64, y_css: f64) -> String {
        self.trading_hit_json_device(x_css, y_css, InputDevice::Mouse as u8)
    }

    pub fn trading_hit_json_device(&self, x_css: f64, y_css: f64, device: u8) -> String {
        let profile = aeris_charts_engine::HitProfile::for_device(input_device_from_u8(device));
        let hit = self
            .inner
            .borrow()
            .engine
            .trading_hit_at_with_profile(x_css, y_css, profile);
        match hit {
            None => "null".to_string(),
            Some(hit) => {
                let (object_type, id) = match hit.object {
                    aeris_charts_engine::TradingObjectId::Position(id) => {
                        ("position", id.as_str().to_string())
                    }
                    aeris_charts_engine::TradingObjectId::Order(id) => {
                        ("order", id.as_str().to_string())
                    }
                    aeris_charts_engine::TradingObjectId::Execution(id) => {
                        ("execution", id.as_str().to_string())
                    }
                };
                let kind = match hit.kind {
                    aeris_charts_engine::TradingHitKind::PositionLine => "position_line",
                    aeris_charts_engine::TradingHitKind::OrderLine => "order_line",
                    aeris_charts_engine::TradingHitKind::CancelButton => "cancel_button",
                    aeris_charts_engine::TradingHitKind::ExecutionMarker => "execution_marker",
                    aeris_charts_engine::TradingHitKind::Annotation => "annotation",
                };
                serde_json::json!({
                    "object_type": object_type,
                    "id": id,
                    "kind": kind,
                    "distance": hit.distance,
                    "annotation_id": hit.annotation_id,
                })
                .to_string()
            }
        }
    }

    pub fn trading_hover_at(&mut self, x_css: f64, y_css: f64) -> bool {
        self.inner
            .borrow_mut()
            .engine
            .set_trading_hover(x_css, y_css)
    }

    pub fn trading_cursor_at(&self, x_css: f64, y_css: f64) -> u8 {
        match self.inner.borrow().engine.trading_hit_at(x_css, y_css) {
            None => 0,
            Some(hit) if hit.kind == aeris_charts_engine::TradingHitKind::OrderLine => 2,
            Some(hit) if matches!(hit.kind, aeris_charts_engine::TradingHitKind::CancelButton) => 1,
            Some(_) => 0,
        }
    }

    /// Reveal the hovered trading control's action tooltip once the host's hover dwell elapses.
    /// Returns whether the frame changed, so the caller can skip the repaint.
    pub fn arm_trading_tooltip(&mut self) -> bool {
        self.inner.borrow_mut().engine.arm_trading_tooltip()
    }

    pub fn clear_trading_hover(&mut self) -> bool {
        self.inner.borrow_mut().engine.clear_trading_hover()
    }

    pub fn trading_pressed_at(&mut self, x_css: f64, y_css: f64) -> bool {
        self.inner
            .borrow_mut()
            .engine
            .set_trading_pressed(x_css, y_css)
    }

    pub fn trading_pressed_at_device(&mut self, x_css: f64, y_css: f64, device: u8) -> bool {
        let profile = aeris_charts_engine::HitProfile::for_device(input_device_from_u8(device));
        self.inner
            .borrow_mut()
            .engine
            .set_trading_pressed_with_profile(x_css, y_css, profile)
    }

    pub fn clear_trading_pressed(&mut self) -> bool {
        self.inner.borrow_mut().engine.clear_trading_pressed()
    }

    pub fn deactivate_trading_group(&mut self) -> bool {
        self.inner.borrow_mut().engine.deactivate_trading_group()
    }

    pub fn trading_drag_start_at(&mut self, x_css: f64, y_css: f64) -> bool {
        self.inner
            .borrow_mut()
            .engine
            .trading_drag_start_at(x_css, y_css)
    }
    pub fn trading_drag_start_at_device(&mut self, x_css: f64, y_css: f64, device: u8) -> bool {
        let profile = aeris_charts_engine::HitProfile::for_device(input_device_from_u8(device));
        self.inner
            .borrow_mut()
            .engine
            .trading_drag_start_at_with_profile(x_css, y_css, profile)
    }
    pub fn trading_keyboard_start_order(&mut self, id: &str) -> bool {
        OrderId::new(id).is_ok_and(|id| {
            self.inner
                .borrow_mut()
                .engine
                .trading_keyboard_start_order(&id)
        })
    }
    pub fn trading_keyboard_adjust(&mut self, ticks: i32) -> bool {
        self.inner
            .borrow_mut()
            .engine
            .trading_keyboard_adjust(ticks)
    }
    pub fn trading_keyboard_commit_json(&mut self) -> String {
        serde_json::to_string(&self.inner.borrow_mut().engine.trading_keyboard_commit())
            .unwrap_or_else(|_| "null".to_string())
    }

    pub fn trading_drag_to(&mut self, y_css: f64) -> bool {
        self.inner.borrow_mut().engine.trading_drag_to(y_css)
    }

    pub fn trading_drag_end_json(&mut self) -> String {
        serde_json::to_string(&self.inner.borrow_mut().engine.trading_drag_end())
            .unwrap_or_else(|_| "null".to_string())
    }

    pub fn cancel_trading_drag(&mut self) -> bool {
        self.inner.borrow_mut().engine.cancel_trading_drag()
    }

    pub fn discard_trading_interaction(&mut self) -> bool {
        self.inner.borrow_mut().engine.discard_trading_interaction()
    }

    pub fn trading_activate_at(&mut self, x_css: f64, y_css: f64) -> bool {
        self.inner
            .borrow_mut()
            .engine
            .trading_activate_at(x_css, y_css)
    }

    pub fn trading_preview_json(&self) -> String {
        serde_json::to_string(&self.inner.borrow().engine.trading_preview())
            .unwrap_or_else(|_| "null".to_string())
    }

    pub fn resolve_trading_intent(&mut self, sequence: u32, accepted: bool) -> bool {
        self.inner
            .borrow_mut()
            .engine
            .resolve_trading_intent(sequence, accepted)
    }

    pub fn take_trading_intents_json(&mut self) -> String {
        serde_json::to_string(&self.inner.borrow_mut().engine.take_trading_intents())
            .unwrap_or_else(|_| "[]".to_string())
    }

    /// Explicit, idempotent pre-drop cleanup. The TypeScript owner calls this immediately before
    /// wasm-bindgen `free()` so retained JS handles cannot retain chart, extension, ring, or GPU
    /// resources through garbage-collection timing.
    pub fn dispose(&mut self) {
        if self.disposed {
            return;
        }
        self.disposed = true;
        self._resize = None;
        let mut inner = self.inner.borrow_mut();
        inner.dispose_extensions();
        inner.gfx = None;
        inner.gpu_groups.clear();
        inner.frame = aeris_charts_engine::ChartFrame::default();
        inner.axis_frame = AxisFrame::default();
        inner.axis_prims.clear();
        inner.text_runs = None;
        inner.gpu_pane = None;
        inner.fallback_pane = None;
    }

    /// Binds the engine to `container`, sizing both canvases to the container's exact
    /// device-pixel content box (crisp at any devicePixelRatio, fractional included) and
    /// re-rendering on every size/DPR change. After this, the embedder never sizes canvases.
    pub fn enable_auto_resize(&mut self, container: web_sys::HtmlElement) -> Result<(), JsValue> {
        let inner = self.inner.clone();
        let gpu_pane = self
            .gpu_pane
            .clone()
            .ok_or_else(|| JsValue::from_str("auto resize is unavailable for OffscreenCanvas"))?;
        let fallback_pane = self
            .fallback_pane
            .clone()
            .ok_or_else(|| JsValue::from_str("auto resize is unavailable for OffscreenCanvas"))?;
        let overlay = self
            .overlay
            .clone()
            .ok_or_else(|| JsValue::from_str("auto resize is unavailable for OffscreenCanvas"))?;
        let container_cb = container.clone();

        let callback = Closure::wrap(Box::new(move |entries: js_sys::Array| {
            let rect = container_cb.get_bounding_client_rect();
            // A hidden or detached host (a grid-maximize off-slot, `display:none`, an
            // unmounted cell) reports ~0x0: keep the last good size instead of following.
            // Collapsing through 1px forces the bar spacing to its minimum, and the scale
            // does not restore it — the chart comes back scrolled into empty whitespace.
            if rect.width() < 2.0 || rect.height() < 2.0 {
                return;
            }
            let (css_w, css_h) = (rect.width().max(1.0), rect.height().max(1.0));
            let dpr = web_sys::window()
                .map(|w| w.device_pixel_ratio())
                .unwrap_or(1.0);
            // Prefer the exact device-pixel content box; fall back to round(css*dpr).
            let device = entries
                .get(0)
                .dyn_into::<web_sys::ResizeObserverEntry>()
                .ok()
                .and_then(|e| device_pixel_box(&e));
            let (bw, bh) = match device {
                Some((dw, dh)) => {
                    // Bogus-report guard: at dpr != 1 the device box must exceed the CSS box.
                    // When it comes back equal (some engines/emulators report CSS px here), the
                    // report is untrustworthy — use round(css*dpr) instead of downscaling to
                    // a blurry dpr-1 bitmap.
                    if dpr > 1.0 && (dw - css_w).abs() <= 1.0 && (dh - css_h).abs() <= 1.0 {
                        ((css_w * dpr).round(), (css_h * dpr).round())
                    } else {
                        (dw, dh)
                    }
                }
                None => ((css_w * dpr).round(), (css_h * dpr).round()),
            };
            apply_device_size(
                &inner,
                &gpu_pane,
                &fallback_pane,
                &overlay,
                css_w,
                css_h,
                bw,
                bh,
            );
        }) as Box<dyn FnMut(js_sys::Array)>);

        let observer = web_sys::ResizeObserver::new(callback.as_ref().unchecked_ref())?;
        // Prefer the device-pixel-content-box (fires on DPR changes and is crisp at fractional
        // ratios). Safari/WebKit lacks it and *throws* if asked, so fall back to a plain content-box
        // observation there — the callback already degrades to round(css*dpr) when the exact box is
        // unavailable.
        if supports_device_pixel_content_box() {
            let opts = web_sys::ResizeObserverOptions::new();
            opts.set_box(web_sys::ResizeObserverBoxOptions::DevicePixelContentBox);
            observer.observe_with_options(&container, &opts);
        } else {
            observer.observe(&container);
        }

        // Size once now so the first paint is correct even before the observer first fires.
        let rect = container.get_bounding_client_rect();
        if rect.width() >= 2.0 && rect.height() >= 2.0 {
            let (css_w, css_h) = (rect.width(), rect.height());
            let dpr = web_sys::window()
                .map(|w| w.device_pixel_ratio())
                .unwrap_or(1.0);
            apply_device_size(
                &self.inner,
                self.gpu_pane.as_ref().expect("HTML canvas checked above"),
                self.fallback_pane
                    .as_ref()
                    .expect("HTML canvas checked above"),
                self.overlay.as_ref().expect("HTML canvas checked above"),
                css_w,
                css_h,
                (css_w * dpr).round(),
                (css_h * dpr).round(),
            );
        }

        self._resize = Some(ResizeBinding {
            observer,
            _callback: callback,
        });
        Ok(())
    }

    /// Disconnect the engine-owned resize observer so manual sizing becomes authoritative again.
    pub fn disable_auto_resize(&mut self) {
        self._resize = None;
    }

    /// Adds a series and returns its id. `kind`: 0 candles, 1 bars, 2 line, 3 area, 4 histogram.
    pub fn add_series(&mut self, kind: u8) -> u32 {
        self.inner.borrow_mut().add_series(kind)
    }

    /// Install or clear transient brush styling on an ordinary Area series. This does not create a
    /// feature series or alter canonical data ownership.
    pub fn set_series_area_brush_state(&mut self, id: u32, state_json: &str) -> bool {
        self.inner
            .borrow_mut()
            .set_series_area_brush_state(id, state_json)
    }

    /// Add one of Aeris's engine-owned advanced series. `kind` is [`FeatureSeriesKind::to_u8`];
    /// the host supplies data and options, while every render/scale semantic stays in Rust.
    pub fn add_feature_series(&mut self, kind: u8, adopt_primary: bool, options_json: &str) -> u32 {
        self.inner
            .borrow_mut()
            .add_feature_series(kind, adopt_primary, options_json)
    }

    /// Replace an engine-owned advanced series' typed items.
    pub fn set_feature_series_data(&mut self, id: u32, items: js_sys::Array) -> Option<String> {
        self.inner.borrow_mut().set_feature_series_data(id, items)
    }

    pub fn update_feature_series_item(&mut self, id: u32, item: JsValue) -> Option<String> {
        self.inner.borrow_mut().update_feature_series_item(id, item)
    }

    pub fn feature_series_data(&self, id: u32) -> JsValue {
        self.inner.borrow().feature_series_data(id)
    }

    pub fn feature_series_data_by_index(&self, id: u32, index: f64, mismatch: i8) -> JsValue {
        self.inner
            .borrow()
            .feature_series_data_by_index(id, index, mismatch)
    }

    pub fn feature_series_kind(&self, id: u32) -> Option<u8> {
        self.inner
            .borrow()
            .engine
            .feature_series_kind(id)
            .map(FeatureSeriesKind::to_u8)
    }

    pub fn feature_series_options_json(&self, id: u32) -> String {
        self.inner
            .borrow()
            .engine
            .feature_series_options_json(id)
            .unwrap_or_else(|| "{}".to_string())
    }

    pub fn add_footprint_series(&mut self, adopt_primary: bool, options_json: &str) -> u32 {
        self.inner
            .borrow_mut()
            .add_footprint_series(adopt_primary, options_json)
    }

    pub fn add_trade_stream(&mut self, key: &str, options_json: &str) -> u32 {
        self.inner.borrow_mut().add_trade_stream(key, options_json)
    }

    pub fn add_depth_stream(&mut self, key: &str, options_json: &str) -> u32 {
        self.inner.borrow_mut().add_depth_stream(key, options_json)
    }

    pub fn depth_stream_id(&self, key: &str) -> u32 {
        self.inner.borrow().depth_stream_id(key)
    }

    pub fn remove_depth_stream(&mut self, stream_id: u32) -> bool {
        self.inner.borrow_mut().remove_depth_stream(stream_id)
    }

    pub fn add_depth_heatmap(&mut self, stream_id: u32, options_json: &str) -> u32 {
        self.inner
            .borrow_mut()
            .add_depth_heatmap(stream_id, options_json)
    }

    pub fn remove_depth_heatmap(&mut self, id: u32) -> bool {
        self.inner.borrow_mut().remove_depth_heatmap(id)
    }

    pub fn add_depth_event_layer(&mut self, stream_id: u32, options_json: &str) -> u32 {
        self.inner
            .borrow_mut()
            .add_depth_event_layer(stream_id, options_json)
    }

    pub fn remove_depth_event_layer(&mut self, id: u32) -> bool {
        self.inner.borrow_mut().remove_depth_event_layer(id)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn set_depth_events_typed(
        &mut self,
        stream_id: u32,
        timestamps_micros: &Float64Array,
        prices: &Float64Array,
        sizes: &Float64Array,
        sides: &js_sys::Int8Array,
        kinds: &js_sys::Uint8Array,
        labels_json: &str,
    ) -> String {
        self.inner.borrow_mut().set_depth_events_typed(
            stream_id,
            timestamps_micros,
            prices,
            sizes,
            sides,
            kinds,
            labels_json,
        )
    }

    pub fn depth_ladder_json(
        &self,
        stream_id: u32,
        levels_per_side: u32,
        minimum_size: f64,
        max_distance_ticks: u32,
    ) -> String {
        self.inner.borrow().depth_ladder_json(
            stream_id,
            levels_per_side,
            minimum_size,
            max_distance_ticks,
        )
    }

    pub fn depth_study_json(
        &self,
        stream_id: u32,
        levels_per_side: u32,
        minimum_size: f64,
        max_distance_ticks: u32,
    ) -> String {
        self.inner.borrow().depth_study_json(
            stream_id,
            levels_per_side,
            minimum_size,
            max_distance_ticks,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn set_depth_snapshot_typed(
        &mut self,
        stream_id: u32,
        timestamp_micros: f64,
        sequence_high: u32,
        sequence_low: u32,
        bid_prices: &Float64Array,
        bid_sizes: &Float64Array,
        bid_order_counts: &js_sys::Uint32Array,
        ask_prices: &Float64Array,
        ask_sizes: &Float64Array,
        ask_order_counts: &js_sys::Uint32Array,
    ) -> String {
        self.inner.borrow_mut().set_depth_snapshot_typed(
            stream_id,
            timestamp_micros,
            sequence_high,
            sequence_low,
            bid_prices,
            bid_sizes,
            bid_order_counts,
            ask_prices,
            ask_sizes,
            ask_order_counts,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn update_depth_typed(
        &mut self,
        stream_id: u32,
        timestamps_micros: &Float64Array,
        sequence_high: &js_sys::Uint32Array,
        sequence_low: &js_sys::Uint32Array,
        previous_high: &js_sys::Uint32Array,
        previous_low: &js_sys::Uint32Array,
        sides: &js_sys::Uint8Array,
        prices: &Float64Array,
        sizes: &Float64Array,
        order_counts: &js_sys::Uint32Array,
    ) -> String {
        self.inner.borrow_mut().update_depth_typed(
            stream_id,
            timestamps_micros,
            sequence_high,
            sequence_low,
            previous_high,
            previous_low,
            sides,
            prices,
            sizes,
            order_counts,
        )
    }

    pub fn replay_clock_micros(&self) -> f64 {
        self.inner.borrow().replay_clock_micros()
    }

    pub fn set_replay_clock_micros(&mut self, clock_micros: f64) -> String {
        self.inner
            .borrow_mut()
            .set_replay_clock_micros(clock_micros)
    }

    pub fn configure_synthetic_bar_series(&mut self, id: u32, options_json: &str) -> String {
        self.inner
            .borrow_mut()
            .configure_synthetic_bar_series(id, options_json)
    }

    pub fn set_synthetic_bar_source_typed(
        &mut self,
        id: u32,
        times: &Float64Array,
        open: &Float64Array,
        high: &Float64Array,
        low: &Float64Array,
        close: &Float64Array,
    ) -> String {
        self.inner
            .borrow_mut()
            .set_synthetic_bar_source_typed(id, times, open, high, low, close)
    }

    pub fn update_synthetic_bar_source(
        &mut self,
        id: u32,
        time: f64,
        open: f64,
        high: f64,
        low: f64,
        close: f64,
    ) -> String {
        self.inner
            .borrow_mut()
            .update_synthetic_bar_source(id, time, open, high, low, close)
    }

    pub fn set_trade_stream_sessions(&mut self, stream_id: u32, sessions_json: &str) -> String {
        self.inner
            .borrow_mut()
            .set_trade_stream_sessions(stream_id, sessions_json)
    }

    pub fn add_trade_volume_series(&mut self, stream_id: u32, pane_index: usize) -> u32 {
        self.inner
            .borrow_mut()
            .add_trade_volume_series(stream_id, pane_index)
    }

    pub fn configure_resampled_series(&mut self, target: u32, options_json: &str) -> String {
        self.inner
            .borrow_mut()
            .configure_resampled_series(target, options_json)
    }

    pub fn resampled_bars_json(&self, target: u32) -> String {
        self.inner.borrow().resampled_bars_json(target)
    }

    pub fn resample_stats_json(&self, target: u32) -> String {
        self.inner.borrow().resample_stats_json(target)
    }

    pub fn trade_stream_id(&self, key: &str) -> u32 {
        self.inner.borrow().trade_stream_id(key)
    }

    pub fn trade_stream_revision(&self, stream_id: u32) -> u32 {
        self.inner.borrow().trade_stream_revision(stream_id)
    }

    pub fn trade_stream_stats_json(&self, stream_id: u32) -> String {
        self.inner.borrow().trade_stream_stats_json(stream_id)
    }

    pub fn time_and_sales_json(&self, stream_id: u32, options_json: &str) -> String {
        self.inner
            .borrow()
            .time_and_sales_json(stream_id, options_json)
    }

    pub fn trade_stream_replay_clock_micros(&self, stream_id: u32) -> f64 {
        self.inner
            .borrow()
            .trade_stream_replay_clock_micros(stream_id)
    }

    pub fn set_trade_stream_replay_clock_micros(
        &mut self,
        stream_id: u32,
        clock_micros: f64,
    ) -> String {
        self.inner
            .borrow_mut()
            .set_trade_stream_replay_clock_micros(stream_id, clock_micros)
    }

    pub fn bind_footprint_series_to_stream(&mut self, id: u32, stream_id: u32) -> bool {
        self.inner
            .borrow_mut()
            .bind_footprint_series_to_stream(id, stream_id)
    }

    pub fn bind_trade_bar_series_to_stream(&mut self, id: u32, stream_id: u32) -> bool {
        self.inner
            .borrow_mut()
            .bind_trade_bar_series_to_stream(id, stream_id)
    }

    pub fn add_cvd_series(
        &mut self,
        stream_id: u32,
        pane_index: usize,
        reset: u8,
        anchor_timestamp_micros: f64,
    ) -> u32 {
        self.inner.borrow_mut().add_cvd_series(
            stream_id,
            pane_index,
            reset,
            anchor_timestamp_micros,
        )
    }

    pub fn add_delta_series(&mut self, stream_id: u32, pane_index: usize) -> u32 {
        self.inner
            .borrow_mut()
            .add_delta_series(stream_id, pane_index)
    }

    pub fn add_trade_bubbles(
        &mut self,
        stream_id: u32,
        series_id: u32,
        minimum_volume: f64,
        max_markers: usize,
        aggregation_window_micros: f64,
    ) -> bool {
        self.inner.borrow_mut().add_trade_bubbles(
            stream_id,
            series_id,
            minimum_volume,
            max_markers,
            aggregation_window_micros,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn set_footprint_trades_typed(
        &mut self,
        id: u32,
        timestamps: &[f64],
        prices: &[f64],
        volumes: &[f64],
        sides: &[u8],
        bids: &[f64],
        asks: &[f64],
        sequences: &[f64],
        trade_ids: &[f64],
        conditions: &[u32],
        session_ids: &[f64],
    ) -> String {
        self.inner.borrow_mut().set_footprint_trades_typed(
            id,
            timestamps,
            prices,
            volumes,
            sides,
            bids,
            asks,
            sequences,
            trade_ids,
            conditions,
            session_ids,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn set_trade_stream_trades_typed(
        &mut self,
        stream_id: u32,
        timestamps: &[f64],
        prices: &[f64],
        volumes: &[f64],
        sides: &[u8],
        bids: &[f64],
        asks: &[f64],
        sequences: &[f64],
        trade_ids: &[f64],
        conditions: &[u32],
        session_ids: &[f64],
    ) -> String {
        self.inner.borrow_mut().set_trade_stream_trades_typed(
            stream_id,
            timestamps,
            prices,
            volumes,
            sides,
            bids,
            asks,
            sequences,
            trade_ids,
            conditions,
            session_ids,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn update_footprint_trades_typed(
        &mut self,
        id: u32,
        timestamps: &[f64],
        prices: &[f64],
        volumes: &[f64],
        sides: &[u8],
        bids: &[f64],
        asks: &[f64],
        sequences: &[f64],
        trade_ids: &[f64],
        conditions: &[u32],
        session_ids: &[f64],
    ) -> String {
        self.inner.borrow_mut().update_footprint_trades_typed(
            id,
            timestamps,
            prices,
            volumes,
            sides,
            bids,
            asks,
            sequences,
            trade_ids,
            conditions,
            session_ids,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn update_trade_stream_trades_typed(
        &mut self,
        stream_id: u32,
        timestamps: &[f64],
        prices: &[f64],
        volumes: &[f64],
        sides: &[u8],
        bids: &[f64],
        asks: &[f64],
        sequences: &[f64],
        trade_ids: &[f64],
        conditions: &[u32],
        session_ids: &[f64],
    ) -> String {
        self.inner.borrow_mut().update_trade_stream_trades_typed(
            stream_id,
            timestamps,
            prices,
            volumes,
            sides,
            bids,
            asks,
            sequences,
            trade_ids,
            conditions,
            session_ids,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn update_footprint_trade_typed(
        &mut self,
        id: u32,
        timestamp: f64,
        price: f64,
        volume: f64,
        side: u8,
        bid: f64,
        ask: f64,
        sequence: f64,
        trade_id: f64,
        conditions: u32,
        session_id: f64,
    ) -> String {
        self.inner.borrow_mut().update_footprint_trade_typed(
            id, timestamp, price, volume, side, bid, ask, sequence, trade_id, conditions,
            session_id,
        )
    }

    pub fn footprint_bars_json(&self, id: u32) -> String {
        self.inner
            .borrow()
            .engine
            .footprint_bars(id)
            .and_then(|bars| serde_json::to_string(&bars).ok())
            .unwrap_or_else(|| "null".to_string())
    }

    pub fn footprint_bar_json(&self, id: u32, index: usize) -> String {
        self.inner
            .borrow()
            .engine
            .footprint_bar(id, index)
            .and_then(|bar| serde_json::to_string(&bar).ok())
            .unwrap_or_else(|| "null".to_string())
    }

    pub fn footprint_options_json(&self, id: u32) -> String {
        self.inner
            .borrow()
            .engine
            .footprint_series_options(id)
            .map_or_else(
                || "{}".to_string(),
                |options| footprint::options_json(&options),
            )
    }

    pub fn apply_footprint_options(&mut self, id: u32, options_json: &str) -> String {
        self.inner
            .borrow_mut()
            .apply_footprint_options(id, options_json)
    }

    /// Merge advanced-series options into the engine-owned state.
    pub fn apply_feature_series_options(&mut self, id: u32, options_json: &str) -> bool {
        self.inner
            .borrow_mut()
            .apply_feature_series_options(id, options_json)
    }

    pub fn add_native_bands_indicator(&mut self, series_id: u32, options_json: &str) -> u32 {
        self.inner
            .borrow_mut()
            .add_native_bands_indicator(series_id, options_json)
    }

    pub fn set_native_bands_indicator_options(&mut self, id: u32, options_json: &str) -> bool {
        self.inner
            .borrow_mut()
            .set_native_bands_indicator_options(id, options_json)
    }

    pub fn add_native_overlay_price_scale(&mut self, series_id: u32, options_json: &str) -> u32 {
        self.inner
            .borrow_mut()
            .add_native_overlay_price_scale(series_id, options_json)
    }

    pub fn set_native_overlay_price_scale_options(&mut self, id: u32, options_json: &str) -> bool {
        self.inner
            .borrow_mut()
            .set_native_overlay_price_scale_options(id, options_json)
    }

    /// Attach the shared-engine point focus ring used by the browser accessibility controller.
    pub fn add_native_accessibility_focus(&mut self, series_id: u32, options_json: &str) -> u32 {
        self.inner
            .borrow_mut()
            .add_native_accessibility_focus(series_id, options_json)
    }

    /// Move, hide, or restyle an accessibility focus ring (`NaN` hides it).
    pub fn set_native_accessibility_focus(
        &mut self,
        primitive_id: u32,
        time: f64,
        options_json: &str,
    ) -> bool {
        let time = if time.is_nan() {
            None
        } else if let Ok(time) = aeris_charts_core::model::data_validation::validate_timestamp(time)
        {
            Some(time)
        } else {
            return false;
        };
        self.inner
            .borrow_mut()
            .set_native_accessibility_focus(primitive_id, time, options_json)
    }

    /// Attach an engine-owned image watermark. The host decodes the source image once and passes
    /// bounded RGBA8 pixels; placement and execution are shared across every backend.
    pub fn add_native_image_watermark(
        &mut self,
        series_id: u32,
        width: u32,
        height: u32,
        pixels: &[u8],
        options_json: &str,
    ) -> u32 {
        self.inner.borrow_mut().add_native_image_watermark(
            series_id,
            width,
            height,
            pixels,
            options_json,
        )
    }

    /// Attach viewport-aligned text whose layout and rendering are owned by the Rust frame.
    pub fn add_native_anchored_text(&mut self, series_id: u32, options_json: &str) -> u32 {
        self.inner
            .borrow_mut()
            .add_native_anchored_text(series_id, options_json)
    }

    /// Transactionally replace all anchored-text options.
    pub fn set_native_anchored_text_options(&mut self, id: u32, options_json: &str) -> bool {
        self.inner
            .borrow_mut()
            .set_native_anchored_text_options(id, options_json)
    }

    /// Attach the official multi-line text watermark through the shared frame.
    pub fn add_native_text_watermark(&mut self, pane_index: usize, options_json: &str) -> u32 {
        self.inner
            .borrow_mut()
            .add_native_text_watermark(pane_index, options_json)
    }

    /// Transactionally replace all text-watermark options.
    pub fn set_native_text_watermark_options(&mut self, id: u32, options_json: &str) -> bool {
        self.inner
            .borrow_mut()
            .set_native_text_watermark_options(id, options_json)
    }

    /// Attach the series-primitive vertical line and optional time-axis label. This is not the
    /// interactive `DrawingKind::VerticalLine` tool.
    pub fn add_native_vertical_line(
        &mut self,
        series_id: u32,
        time: f64,
        options_json: &str,
    ) -> u32 {
        self.inner
            .borrow_mut()
            .add_native_vertical_line(series_id, time, options_json)
    }

    pub fn add_native_delta_tooltip(&mut self, series_id: u32, options_json: &str) -> u32 {
        self.inner
            .borrow_mut()
            .add_native_delta_tooltip(series_id, options_json)
    }

    pub fn add_native_tooltip(&mut self, series_id: u32, options_json: &str) -> u32 {
        self.inner
            .borrow_mut()
            .add_native_tooltip(series_id, options_json)
    }

    pub fn set_native_tooltip_options(&mut self, primitive_id: u32, options_json: &str) -> bool {
        self.inner
            .borrow_mut()
            .set_native_tooltip_options(primitive_id, options_json)
    }

    pub fn native_tooltip_snapshot_json(&self, primitive_id: u32) -> String {
        self.inner
            .borrow()
            .native_tooltip_snapshot_json(primitive_id)
    }

    pub fn native_delta_tooltip_active_range_json(&self, primitive_id: u32) -> String {
        self.inner
            .borrow()
            .native_delta_tooltip_active_range_json(primitive_id)
    }
    pub fn clear_native_delta_tooltip(&mut self, primitive_id: u32) -> bool {
        self.inner
            .borrow_mut()
            .engine
            .clear_delta_tooltip(primitive_id)
    }

    /// Forward normalized host mouse samples to every engine-owned delta tooltip.
    pub fn native_delta_tooltip_mouse_down(&mut self, x: f64, shift: bool) -> bool {
        self.inner
            .borrow_mut()
            .engine
            .delta_tooltip_mouse_down_with_shift(x, shift)
    }

    pub fn native_delta_tooltip_mouse_move(&mut self, x: f64) -> bool {
        self.inner.borrow_mut().engine.delta_tooltip_mouse_move(x)
    }

    pub fn native_delta_tooltip_mouse_up(&mut self) -> bool {
        self.inner.borrow_mut().engine.delta_tooltip_mouse_up()
    }

    pub fn native_delta_tooltip_touch_move(&mut self, xs: &[f64]) -> bool {
        self.inner.borrow_mut().engine.delta_tooltip_touch_move(xs)
    }

    pub fn native_delta_tooltip_active(&self) -> bool {
        self.inner.borrow().engine.has_delta_tooltip()
    }

    pub fn native_delta_tooltip_leave(&mut self) -> bool {
        self.inner.borrow_mut().engine.delta_tooltip_leave()
    }

    /// Attach the two-point series primitive with endpoint labels. This is not the interactive
    /// `DrawingKind::TrendLine` tool.
    #[allow(clippy::too_many_arguments)]
    pub fn add_native_trend_line(
        &mut self,
        series_id: u32,
        first_time: f64,
        first_price: f64,
        second_time: f64,
        second_price: f64,
        options_json: &str,
    ) -> u32 {
        self.inner.borrow_mut().add_native_trend_line(
            series_id,
            first_time,
            first_price,
            second_time,
            second_price,
            options_json,
        )
    }

    /// Attach declarative session shading; JS only serializes the boundary options.
    pub fn add_native_session_highlighting(&mut self, series_id: u32, options_json: &str) -> u32 {
        self.inner
            .borrow_mut()
            .add_native_session_highlighting(series_id, options_json)
    }

    /// Replace callback-derived `{time,color}` records for a session-highlighting primitive.
    pub fn set_native_session_highlighting_data(
        &mut self,
        primitive_id: u32,
        highlights_json: &str,
    ) -> bool {
        self.inner
            .borrow_mut()
            .set_native_session_highlighting_data(primitive_id, highlights_json)
    }

    /// Merge `{time,color}` records for rows a live update appended or replaced at the tail.
    /// Returns false (unchanged) when the result would not align with the source.
    pub fn upsert_native_session_highlighting_data(
        &mut self,
        primitive_id: u32,
        highlights_json: &str,
    ) -> bool {
        self.inner
            .borrow_mut()
            .upsert_native_session_highlighting_data(primitive_id, highlights_json)
    }

    /// Canonical source times at or after `since` for incremental per-row host callbacks.
    pub fn series_times_since(&self, series_id: u32, since: f64) -> Vec<f64> {
        let inner = self.inner.borrow();
        let since = if since.is_nan() {
            i64::MIN
        } else {
            since.clamp(i64::MIN as f64, i64::MAX as f64) as i64
        };
        inner
            .series_times_since(series_id, since)
            .iter()
            .map(|&time| time as f64)
            .collect()
    }

    /// Attach a retained-frame bar-slot highlight driven directly by the engine crosshair.
    pub fn add_native_crosshair_highlight(&mut self, series_id: u32, color: Option<String>) -> u32 {
        self.inner
            .borrow_mut()
            .add_native_crosshair_highlight(series_id, color)
    }

    /// Create a calculated visible-range OHLCV distribution, not a pre-binned primitive.
    pub fn add_volume_profile_indicator(
        &mut self,
        source: u32,
        volume_source: u32,
        options_json: &str,
    ) -> u32 {
        let Ok(options) = serde_json::from_str::<aeris_charts_engine::VolumeProfileIndicatorOptions>(
            options_json,
        ) else {
            return 0;
        };
        self.inner
            .borrow_mut()
            .engine
            .add_volume_profile_indicator(source, volume_source, options)
            .unwrap_or(0)
    }

    pub fn set_volume_profile_indicator_options(&mut self, id: u32, options_json: &str) -> bool {
        let Ok(options) = serde_json::from_str::<aeris_charts_engine::VolumeProfileIndicatorOptions>(
            options_json,
        ) else {
            return false;
        };
        self.inner
            .borrow_mut()
            .engine
            .set_volume_profile_indicator_options(id, options)
    }

    pub fn volume_profile_indicator_options(&self, id: u32) -> String {
        serde_json::to_string(
            &self
                .inner
                .borrow()
                .engine
                .volume_profile_indicator_options(id),
        )
        .expect("validated profile options serialize")
    }

    pub fn volume_profile_indicator_snapshot(&mut self, id: u32) -> String {
        let mut inner = self.inner.borrow_mut();
        let Some(snapshot) = inner.engine.volume_profile_indicator_snapshot(id) else {
            return "null".into();
        };
        let profile = &snapshot.profile;
        let rows = profile
            .rows
            .iter()
            .map(|row| {
                serde_json::json!({ "low": row.low, "high": row.high, "volume": row.volume,
                    "up_volume": row.up_volume, "down_volume": row.down_volume })
            })
            .collect::<Vec<_>>();
        let poc = profile.poc_index.map(|index| {
            let row = &profile.rows[index];
            row.low + (row.high - row.low) * 0.5
        });
        serde_json::json!({ "rows": rows, "total_volume": profile.total_volume, "bar_count": profile.bar_count,
            "poc": poc, "value_area_low": profile.value_area_low_index.map(|index| profile.rows[index].low),
            "value_area_high": profile.value_area_high_index.map(|index| profile.rows[index].high),
            "calculation_revision": snapshot.calculation_revision, "error": snapshot.error,
            "method": "ohlcv_uniform" }).to_string()
    }

    /// Attach the official time-anchored profile schema (`time`, `profile[{price,vol}]`, `width`).
    pub fn add_native_volume_profile(
        &mut self,
        series_id: u32,
        data_json: &str,
        options_json: &str,
    ) -> u32 {
        self.inner
            .borrow_mut()
            .add_native_volume_profile(series_id, data_json, options_json)
    }

    pub fn set_native_volume_profile_data(&mut self, id: u32, data_json: &str) -> bool {
        self.inner
            .borrow_mut()
            .set_native_volume_profile_data(id, data_json)
    }

    pub fn remove_native_primitive(&mut self, id: u32) -> bool {
        self.inner.borrow_mut().remove_native_primitive(id)
    }

    /// Remove a series (and any indicators derived from it). Returns true if a live series
    /// was removed; any id may be removed (reference `removeSeries`) — "primary series" consumers
    /// fall back to the first visible non-removed series.
    pub fn remove_series(&mut self, id: u32) -> bool {
        self.inner.borrow_mut().remove_series(id)
    }

    /// `remove_series` reporting every tombstoned id (the series plus derived indicator
    /// outputs) for the host's per-series removal events; empty = nothing removed.
    pub fn remove_series_tracked(&mut self, id: u32) -> Vec<u32> {
        self.inner.borrow_mut().remove_series_tracked(id)
    }

    /// JSON lineage of an indicator output series (`{"kind","period","deviation","source",
    /// "output_index"}`), or `null` for a plain/source series — platform chip chrome backing.
    pub fn series_indicator_info_json(&self, id: u32) -> String {
        self.inner.borrow().series_indicator_info_json(id)
    }

    pub fn set_indicator_output_style(&mut self, id: u32, style_json: &str) -> bool {
        self.inner
            .borrow_mut()
            .set_indicator_output_style(id, style_json)
    }

    pub fn indicator_schema_json(&self, kind: &str, period: u32, deviation: f64) -> String {
        self.inner
            .borrow()
            .indicator_schema_json(kind, period, deviation)
    }

    /// reference v5.2 `ISeriesApi.pop(count)`: remove the last `count` data points (count clamps
    /// to the data length; per-point colors shift along). Returns the new data length.
    pub fn series_pop(&mut self, id: u32, count: u32) -> u32 {
        self.inner.borrow_mut().series_pop(id, count)
    }

    /// reference `ISeriesApi.lastValueData(globalLast)`: JSON `{"value","formatted","time"}` of
    /// the last (`global_last` true) or last visible (false) non-whitespace bar, the value
    /// formatted with the series' price format. "" when there is no such bar.
    pub fn series_last_value_data(&self, id: u32, global_last: bool) -> String {
        self.inner.borrow().series_last_value_data(id, global_last)
    }

    /// Format a value with the series' resolved price format (custom fn → built-ins → chart
    /// formatter fallback), backing the TS `series.priceFormatter()`.
    pub fn series_format_price(&self, id: u32, value: f64) -> String {
        self.inner.borrow().series_format_price(id, value)
    }

    /// Series ids in current render order (topmost LAST) as a JSON array — reference
    /// `chart.seriesOrder()` backing.
    pub fn series_order_json(&self) -> String {
        self.inner.borrow().series_order_json()
    }

    /// reference `chart.setSeriesOrder`: reorder which series paints on top. Every live series id
    /// must be present exactly once, else the call is rejected (false, no state change).
    pub fn set_series_order(&mut self, ids: Vec<u32>) -> bool {
        self.inner.borrow_mut().set_series_order(ids)
    }

    /// Add a Rust-native simple moving-average line derived from `source_id`.
    pub fn add_sma(&mut self, source_id: u32, period: u32) -> u32 {
        self.inner.borrow_mut().add_sma(source_id, period)
    }

    pub fn add_sma_with_source(&mut self, source_id: u32, source: &str, period: u32) -> u32 {
        self.inner
            .borrow_mut()
            .add_sma_with_source(source_id, source, period)
    }

    /// Add a Rust-native exponential moving-average line derived from `source_id`.
    pub fn add_ema(&mut self, source_id: u32, period: u32) -> u32 {
        self.inner.borrow_mut().add_ema(source_id, period)
    }

    pub fn add_dema(&mut self, source_id: u32, period: u32) -> u32 {
        self.inner.borrow_mut().add_dema(source_id, period)
    }

    pub fn add_tema(&mut self, source_id: u32, period: u32) -> u32 {
        self.inner.borrow_mut().add_tema(source_id, period)
    }

    pub fn add_smma(&mut self, source_id: u32, period: u32) -> u32 {
        self.inner.borrow_mut().add_smma(source_id, period)
    }

    pub fn add_rma(&mut self, source_id: u32, period: u32) -> u32 {
        self.inner.borrow_mut().add_rma(source_id, period)
    }

    pub fn add_hma(&mut self, source_id: u32, period: u32) -> u32 {
        self.inner.borrow_mut().add_hma(source_id, period)
    }

    pub fn add_vwma(&mut self, source_id: u32, volume_source: i32, period: u32) -> u32 {
        self.inner
            .borrow_mut()
            .add_vwma(source_id, volume_source, period)
    }

    pub fn add_standard_deviation(&mut self, source_id: u32, period: u32) -> u32 {
        self.inner
            .borrow_mut()
            .add_standard_deviation(source_id, period)
    }

    pub fn add_cci(&mut self, source_id: u32, period: u32) -> u32 {
        self.inner.borrow_mut().add_cci(source_id, period)
    }

    pub fn add_williams_r(&mut self, source_id: u32, period: u32) -> u32 {
        self.inner.borrow_mut().add_williams_r(source_id, period)
    }

    pub fn add_stochastic_rsi(
        &mut self,
        source_id: u32,
        rsi_period: u32,
        stochastic_period: u32,
    ) -> u32 {
        self.inner
            .borrow_mut()
            .add_stochastic_rsi(source_id, rsi_period, stochastic_period)
    }

    pub fn add_momentum(&mut self, source_id: u32, period: u32) -> u32 {
        self.inner.borrow_mut().add_momentum(source_id, period)
    }

    pub fn add_roc(&mut self, source_id: u32, period: u32) -> u32 {
        self.inner.borrow_mut().add_roc(source_id, period)
    }

    pub fn add_donchian(&mut self, source_id: u32, period: u32) -> Vec<u32> {
        self.inner.borrow_mut().add_donchian(source_id, period)
    }

    /// Add five previous-session pivot levels. `variant` is 1=standard, 2=Fibonacci,
    /// 3=Camarilla, 4=Woodie, or 5=DeMark; invalid values return an empty array.
    pub fn add_pivot_points(&mut self, source_id: u32, variant: u32) -> Vec<u32> {
        self.inner.borrow_mut().add_pivot_points(source_id, variant)
    }

    /// Add a ZigZag line. `deviation_percent` is the minimum reversal percentage.
    pub fn add_zigzag(&mut self, source_id: u32, deviation_percent: f64) -> u32 {
        self.inner
            .borrow_mut()
            .add_zigzag(source_id, deviation_percent)
    }

    pub fn add_keltner(&mut self, source_id: u32, period: u32, multiplier: f64) -> Vec<u32> {
        self.inner
            .borrow_mut()
            .add_keltner(source_id, period, multiplier)
    }

    pub fn add_adx_dmi(&mut self, source_id: u32, period: u32) -> Vec<u32> {
        self.inner.borrow_mut().add_adx_dmi(source_id, period)
    }

    pub fn add_parabolic_sar(&mut self, source_id: u32) -> u32 {
        self.inner.borrow_mut().add_parabolic_sar(source_id)
    }

    pub fn add_supertrend(&mut self, source_id: u32, period: u32, multiplier: f64) -> u32 {
        self.inner
            .borrow_mut()
            .add_supertrend(source_id, period, multiplier)
    }

    pub fn add_ichimoku(&mut self, source_id: u32) -> Vec<u32> {
        self.inner.borrow_mut().add_ichimoku(source_id)
    }

    /// Add one five-output EMA ribbon on the source pane.
    pub fn add_ema_ribbon(
        &mut self,
        source_id: u32,
        period_1: u32,
        period_2: u32,
        period_3: u32,
        period_4: u32,
        period_5: u32,
    ) -> Vec<u32> {
        self.inner.borrow_mut().add_ema_ribbon(
            source_id,
            [period_1, period_2, period_3, period_4, period_5],
        )
    }

    /// Atomically update an EMA ribbon while retaining all five output identities.
    pub fn set_ema_ribbon_periods(
        &mut self,
        id: u32,
        period_1: u32,
        period_2: u32,
        period_3: u32,
        period_4: u32,
        period_5: u32,
    ) -> bool {
        self.inner
            .borrow_mut()
            .set_ema_ribbon_periods(id, [period_1, period_2, period_3, period_4, period_5])
    }

    /// Add upper, middle, and lower Bollinger-band lines. Returns an empty array for invalid input.
    pub fn add_bollinger(&mut self, source_id: u32, period: u32, deviation: f64) -> Vec<u32> {
        self.inner
            .borrow_mut()
            .add_bollinger(source_id, period, deviation)
    }

    pub fn add_bollinger_with_source(
        &mut self,
        source_id: u32,
        source: &str,
        period: u32,
        deviation: f64,
    ) -> Vec<u32> {
        self.inner
            .borrow_mut()
            .add_bollinger_with_source(source_id, source, period, deviation)
    }

    /// Add a Wilder RSI line in its own oscillator pane (30/70 band lines + channel fill).
    pub fn add_rsi(&mut self, source_id: u32, period: u32) -> u32 {
        self.inner.borrow_mut().add_rsi(source_id, period)
    }

    pub fn add_rsi_with_source(&mut self, source_id: u32, source: &str, period: u32) -> u32 {
        self.inner
            .borrow_mut()
            .add_rsi_with_source(source_id, source, period)
    }

    pub fn set_indicator_input_source(&mut self, id: u32, source: &str) -> bool {
        self.inner
            .borrow_mut()
            .set_indicator_input_source(id, source)
    }

    /// Add MACD line, signal line, and histogram (four-state colors) in their own pane.
    pub fn add_macd(&mut self, source_id: u32, fast: u32, slow: u32, signal: u32) -> Vec<u32> {
        self.inner
            .borrow_mut()
            .add_macd(source_id, fast, slow, signal)
    }

    /// Add Stochastic %K and %D lines in their own pane (20/80 band lines + channel fill).
    pub fn add_stochastic(&mut self, source_id: u32, k_period: u32, d_period: u32) -> Vec<u32> {
        self.inner
            .borrow_mut()
            .add_stochastic(source_id, k_period, d_period)
    }

    /// Add a Wilder ATR line in its own oscillator pane.
    pub fn add_atr(&mut self, source_id: u32, period: u32) -> u32 {
        self.inner.borrow_mut().add_atr(source_id, period)
    }

    /// Add a session-anchored VWAP line on the source's pane (`volume_source` -1 = unit weights).
    pub fn add_vwap(&mut self, source_id: u32, volume_source: i32) -> u32 {
        self.inner.borrow_mut().add_vwap(source_id, volume_source)
    }

    /// Add on-balance volume in its own oscillator pane (`volume_source` is required).
    pub fn add_obv(&mut self, source_id: u32, volume_source: i32) -> u32 {
        self.inner.borrow_mut().add_obv(source_id, volume_source)
    }

    /// Add Chaikin money flow in its own oscillator pane.
    pub fn add_cmf(&mut self, source_id: u32, volume_source: i32, period: u32) -> u32 {
        self.inner
            .borrow_mut()
            .add_cmf(source_id, volume_source, period)
    }

    /// Add money flow index in its own oscillator pane.
    pub fn add_mfi(&mut self, source_id: u32, volume_source: i32, period: u32) -> u32 {
        self.inner
            .borrow_mut()
            .add_mfi(source_id, volume_source, period)
    }

    /// Add volume columns and their moving average in an oscillator pane.
    pub fn add_volume(&mut self, source_id: u32, volume_source: i32, period: u32) -> Vec<u32> {
        self.inner
            .borrow_mut()
            .add_volume(source_id, volume_source, period)
    }

    /// Add VWAP basis, standard-deviation and percentage bands. `reset` is session, weekly, or monthly.
    pub fn add_vwap_bands(
        &mut self,
        source_id: u32,
        volume_source: i32,
        reset: &str,
        standard_deviation: f64,
        percent: f64,
    ) -> Vec<u32> {
        self.inner.borrow_mut().add_vwap_bands(
            source_id,
            volume_source,
            reset,
            standard_deviation,
            percent,
        )
    }

    /// Add a weighted moving-average line on the source's pane.
    pub fn add_wma(&mut self, source_id: u32, period: u32) -> u32 {
        self.inner.borrow_mut().add_wma(source_id, period)
    }

    /// Add KDJ K, D and J lines in their own oscillator pane (20/80 band lines).
    pub fn add_kdj(
        &mut self,
        source_id: u32,
        period: u32,
        k_smoothing: u32,
        d_smoothing: u32,
    ) -> Vec<u32> {
        self.inner
            .borrow_mut()
            .add_kdj(source_id, period, k_smoothing, d_smoothing)
    }

    /// Add a built-in indicator from its typed JSON definition. `convention` is `""`,
    /// `"tradingview"` or `"china"`; explicit definition fields override it. Series ids of -1
    /// mean "none". Returns an empty array for invalid input.
    pub fn add_indicator(
        &mut self,
        source_id: u32,
        source: &str,
        kind_json: &str,
        convention: &str,
        volume_source: i32,
        amount_source: i32,
    ) -> Vec<u32> {
        self.inner.borrow_mut().add_indicator(
            source_id,
            source,
            kind_json,
            convention,
            volume_source,
            amount_source,
        )
    }

    /// Sets the main series' data (series 0). `times` are ascending UTC seconds.
    pub fn set_data(
        &mut self,
        times: &[f64],
        open: &[f64],
        high: &[f64],
        low: &[f64],
        close: &[f64],
    ) {
        self.inner
            .borrow_mut()
            .set_data(times, open, high, low, close);
    }

    /// Sets a series' data by id.
    pub fn set_series_data(
        &mut self,
        id: u32,
        times: &[f64],
        open: &[f64],
        high: &[f64],
        low: &[f64],
        close: &[f64],
    ) {
        self.inner
            .borrow_mut()
            .set_series_data(id, times, open, high, low, close);
    }

    /// Typed-array ingestion path: wasm-bindgen passes the JS views as externrefs and the engine
    /// takes one owned copy, avoiding the temporary slice copy generated for `&[f64]` methods.
    pub fn set_series_data_typed(
        &mut self,
        id: u32,
        times: &Float64Array,
        open: &Float64Array,
        high: &Float64Array,
        low: &Float64Array,
        close: &Float64Array,
    ) -> Option<String> {
        self.inner
            .borrow_mut()
            .set_series_data_typed(id, times, open, high, low, close)
    }

    /// Columnar streaming append: a batch of points in `set_series_data_typed`'s column layout,
    /// appended (or replacing the series' last point) in one call. The streaming counterpart to
    /// `set_series_data_typed` — no per-point JS object crosses the boundary.
    pub fn update_series_bars_typed(
        &mut self,
        id: u32,
        times: &Float64Array,
        open: &Float64Array,
        high: &Float64Array,
        low: &Float64Array,
        close: &Float64Array,
    ) -> Option<String> {
        self.inner
            .borrow_mut()
            .update_series_bars_typed(id, times, open, high, low, close)
    }

    /// Bind a `SharedArrayBuffer` ring as a series' data source. `bytes` and `cursor_view` must be
    /// views over the same buffer (the façade builds both); `layout_json` is the public
    /// `ring_source_layout`. Replaces any ring already bound to this series. Returns `""` on
    /// success, else a message describing why the layout was rejected.
    pub fn set_ring_source(
        &mut self,
        series_id: u32,
        bytes: js_sys::Uint8Array,
        cursor_view: js_sys::Int32Array,
        layout_json: &str,
    ) -> String {
        self.inner
            .borrow_mut()
            .set_ring_source(series_id, bytes, cursor_view, layout_json)
    }

    /// Unbind a series' ring source, releasing the engine's views over the shared buffer. A series
    /// with no ring bound is a no-op.
    pub fn clear_ring_source(&mut self, series_id: u32) {
        self.inner.borrow_mut().clear_ring_source(series_id);
    }

    /// Drain every bound ring once. Called by the façade on its frame tick, so the per-tick rate of
    /// the producer never reaches the engine as a call.
    ///
    /// Writes `[pair_count, series_id, rows, series_id, rows, ...]` into `out` for the series that
    /// received rows, and returns the total row count so the caller can skip a repaint when nothing
    /// arrived. `out` should hold `1 + 2 * ring_count` slots; rings beyond that still drain, they
    /// just are not reported.
    pub fn drain_ring_sources(&mut self, out: &mut [f64]) -> u32 {
        self.inner.borrow_mut().drain_ring_sources(out)
    }

    /// Number of bound ring sources — the façade sizes its report scratch from this.
    pub fn ring_source_count(&self) -> u32 {
        self.inner.borrow().rings.len() as u32
    }

    /// Streaming update of the main series (append new time or replace last).
    pub fn update_bar(&mut self, time: f64, open: f64, high: f64, low: f64, close: f64) {
        self.inner
            .borrow_mut()
            .update_bar(time, open, high, low, close);
    }

    /// Streaming update of an arbitrary series by id (append new time or replace last).
    pub fn update_series_bar(
        &mut self,
        series_id: u32,
        time: f64,
        open: f64,
        high: f64,
        low: f64,
        close: f64,
    ) {
        self.inner
            .borrow_mut()
            .update_series_bar(series_id, time, open, high, low, close);
    }

    /// Per-data-point color overrides (reference data-item colors). Each channel is a `Uint32Array`
    /// of packed RGBA values — `0xRRGGBBAA` (e.g. opaque red = `0xFF0000FF`, half-alpha green
    /// = `0x00FF0080`) — or `undefined`/`null`/empty for absent. Within a present channel, a
    /// `0` entry means "no override at this row" (a fully transparent color is not renderable,
    /// so 0 is reserved as the absent marker). Channels: `body` = candle/bar body, line/area
    /// stroke + point marker, histogram column; `wick`/`border` = candlestick parts. Lengths
    /// must equal the series' row count or the whole call is rejected (console warning, no
    /// partial state). `set_series_data` resets all point colors for the series — call this
    /// right after it.
    pub fn set_series_point_colors(
        &mut self,
        id: u32,
        body: Option<Vec<u32>>,
        wick: Option<Vec<u32>>,
        border: Option<Vec<u32>>,
    ) {
        self.inner
            .borrow_mut()
            .set_series_point_colors(id, body, wick, border);
    }

    /// Streaming update like [`update_series_bar`] that also sets the target bar's three
    /// per-point color channels (`undefined` = no custom color for that channel; packed RGBA
    /// `0xRRGGBBAA` as in [`set_series_point_colors`]). Append-new-time vs replace-last
    /// semantics mirror the plain update. Returns `false` when the engine rejected the bar (an
    /// engine-derived series, or values the host boundary did not validate).
    #[allow(clippy::too_many_arguments)] // mirrors update_series_bar plus the three reference color slots
    pub fn update_series_bar_styled(
        &mut self,
        id: u32,
        time: f64,
        open: f64,
        high: f64,
        low: f64,
        close: f64,
        body: Option<u32>,
        wick: Option<u32>,
        border: Option<u32>,
    ) -> bool {
        self.inner
            .borrow_mut()
            .update_series_bar_styled(id, time, open, high, low, close, body, wick, border)
    }

    /// Partial streaming merge (`series.merge`): `NaN` open/high/low/close are absent and keep the
    /// existing bar's values; the engine normalizes the merged high/low envelope. `sequence` is
    /// the optional monotonic guard (`NaN` = none). Returns ingestion diagnostics JSON when the
    /// merge was rejected or stale, `undefined` when it applied.
    #[allow(clippy::too_many_arguments)] // OHLC channels, three color slots, and the guard
    pub fn merge_series_bar(
        &mut self,
        id: u32,
        time: f64,
        open: f64,
        high: f64,
        low: f64,
        close: f64,
        body: Option<u32>,
        wick: Option<u32>,
        border: Option<u32>,
        sequence: f64,
    ) -> Option<String> {
        self.inner.borrow_mut().merge_series_bar(
            id, time, open, high, low, close, body, wick, border, sequence,
        )
    }

    /// [`update_series_bar_styled`] behind the per-series sequence guard: a `sequence` that is
    /// not newer than the last applied one is rejected as stale (diagnostics JSON).
    #[allow(clippy::too_many_arguments)] // mirrors update_series_bar_styled plus the guard
    pub fn update_series_bar_sequenced(
        &mut self,
        id: u32,
        time: f64,
        open: f64,
        high: f64,
        low: f64,
        close: f64,
        body: Option<u32>,
        wick: Option<u32>,
        border: Option<u32>,
        sequence: f64,
    ) -> Option<String> {
        self.inner.borrow_mut().update_series_bar_sequenced(
            id, time, open, high, low, close, body, wick, border, sequence,
        )
    }

    /// [`update_series_bars_typed`] behind the per-series sequence guard (one sequence per batch).
    #[allow(clippy::too_many_arguments)] // the typed OHLC columns plus the guard
    pub fn update_series_bars_typed_sequenced(
        &mut self,
        id: u32,
        times: &Float64Array,
        open: &Float64Array,
        high: &Float64Array,
        low: &Float64Array,
        close: &Float64Array,
        sequence: f64,
    ) -> Option<String> {
        self.inner
            .borrow_mut()
            .update_series_bars_typed_sequenced(id, times, open, high, low, close, sequence)
    }

    /// Typed partial merge (`series.merge_typed`): each row merges like `merge_series_bar`, with
    /// `NaN` entries and omitted columns absent, applied in input order with one synchronization.
    /// The batch is rejected atomically (diagnostics JSON); `undefined` when it applied.
    #[allow(clippy::too_many_arguments)] // the optional OHLC columns plus the guard
    pub fn merge_series_bars_typed(
        &mut self,
        id: u32,
        times: &Float64Array,
        open: Option<Float64Array>,
        high: Option<Float64Array>,
        low: Option<Float64Array>,
        close: Option<Float64Array>,
        sequence: f64,
    ) -> Option<String> {
        self.inner
            .borrow_mut()
            .merge_series_bars_typed(id, times, open, high, low, close, sequence)
    }

    /// Install a sequence baseline (for example a snapshot's sequence) or clear it with `NaN`.
    /// A full data install clears it on its own. Returns false for an unknown series or a
    /// sequence that is not a non-negative safe integer.
    pub fn set_series_update_sequence(&mut self, id: u32, sequence: f64) -> bool {
        self.inner
            .borrow_mut()
            .set_series_update_sequence(id, sequence)
    }

    /// Apply a per-series `priceFormat` (reference PriceFormat) as JSON:
    /// `{"type":"price"|"volume"|"percent", "precision"?, "min_move"?, "tick_ladder"?}` or
    /// `{"type":"custom", "min_move"?}` (keeps a formatter installed via
    /// [`set_series_price_formatter`]; switching to a non-custom type clears it). Returns false,
    /// leaving the format unchanged, for malformed JSON, an unknown type/id, or a rejected
    /// `tick_ladder`.
    pub fn series_apply_price_format_json(&mut self, id: u32, json: &str) -> bool {
        self.inner
            .borrow_mut()
            .series_apply_price_format_json(id, json)
    }

    /// reference `priceFormat: {type:"custom", formatter}`: install the series' custom formatter fn
    /// `(price: number) => string`. A throw or non-string result falls back to the built-in
    /// price formatter. The fn is cleared by applying a non-custom `priceFormat` type.
    pub fn set_series_price_formatter(&mut self, id: u32, formatter: js_sys::Function) {
        self.inner
            .borrow_mut()
            .set_series_price_formatter(id, formatter);
    }

    /// reference series option `autoscaleInfoProvider`: install (or clear with `undefined`) an
    /// adapter `(base) => info` whose result REPLACES the series' autoscale info. `base` is
    /// `{price_range: {min_value, max_value} | null, margins?: {above, below}}` or `null`; the
    /// adapter runs during rendering and must not call back into the chart.
    pub fn set_series_autoscale_info_provider(
        &mut self,
        id: u32,
        provider: Option<js_sys::Function>,
    ) {
        self.inner
            .borrow_mut()
            .set_series_autoscale_info_provider(id, provider);
    }

    /// Sets a series' line/area color (overrides the kind default).
    pub fn set_series_color(&mut self, id: u32, r: u8, g: u8, b: u8) {
        self.inner.borrow_mut().set_series_color(id, r, g, b);
    }

    /// Sets a series' line/area/histogram stroke color from a CSS string, preserving alpha
    /// (the r/g/b `set_series_color` form is opaque-only). Unparseable strings are ignored.
    pub fn set_series_color_css(&mut self, id: u32, css: &str) {
        self.inner.borrow_mut().set_series_color_css(id, css);
    }

    /// Toggle a series while preserving its data and derived-indicator binding.
    pub fn set_series_visible(&mut self, id: u32, visible: bool) {
        self.inner.borrow_mut().set_series_visible(id, visible);
    }

    /// Stops drawing a series' rows at or after `time` (UTC seconds) while keeping its data
    /// (`series_options.render_before_time`). `undefined`/`null` draws every row.
    pub fn set_series_render_before_time(&mut self, id: u32, time: Option<f64>) {
        let time = time
            .filter(|time| time.is_finite())
            .map(|time| time.floor().clamp(i64::MIN as f64, i64::MAX as f64) as i64);
        self.inner
            .borrow_mut()
            .set_series_render_before_time(id, time);
    }

    /// Retention ceiling for a series (`series_options.max_points`): at most this many rows, oldest
    /// evicted first. `undefined`/`null`/`0` clears the cap back to unbounded.
    pub fn set_series_max_points(&mut self, id: u32, max_points: Option<f64>) {
        self.inner
            .borrow_mut()
            .set_series_max_points(id, max_points);
    }

    /// This series' retention ceiling, or `undefined` when unbounded.
    pub fn series_max_points(&self, id: u32) -> Option<f64> {
        self.inner.borrow().series_max_points(id)
    }

    /// Multi-calendar overlay alignment (`series_options.time_alignment`): `as_of` false keeps
    /// the default union; true adds no time points and shows the series' last row at or before
    /// each point, blank once that row is more than `max_staleness` seconds old (`undefined` =
    /// no limit). Returns `{"ok":true}` or `{"ok":false,"error":{code,message}}`.
    pub fn set_series_time_alignment(
        &mut self,
        id: u32,
        as_of: bool,
        max_staleness: Option<f64>,
    ) -> String {
        let alignment = match (as_of, max_staleness) {
            (false, _) => aeris_charts_engine::TimeAlignment::Union,
            (true, None) => aeris_charts_engine::TimeAlignment::AsOf {
                max_staleness: None,
            },
            (true, Some(seconds))
                if seconds.is_finite()
                    && seconds >= 0.0
                    && seconds.fract() == 0.0
                    && seconds <= 9_007_199_254_740_991.0 =>
            {
                aeris_charts_engine::TimeAlignment::AsOf {
                    max_staleness: Some(seconds as i64),
                }
            }
            (true, Some(_)) => {
                return trading_result_json(Err(aeris_charts_engine::ChartError::new(
                    aeris_charts_engine::ErrorCode::InvalidOptions,
                    "as_of_max_staleness must be a non-negative whole number of seconds or null",
                )))
            }
        };
        trading_result_json(
            self.inner
                .borrow_mut()
                .engine
                .set_series_time_alignment(id as SeriesId, alignment),
        )
    }

    /// Set candlestick/bar body colors per direction; same keep/clear/pin contract as the wick
    /// and border setters (`"transparent"` gives a hollow body).
    pub fn set_series_updown_colors(&mut self, id: u32, up: Option<String>, down: Option<String>) {
        self.inner
            .borrow_mut()
            .set_series_updown_colors(id, up, down);
    }

    /// Set candlestick wick colors per direction. `undefined` = keep current, `""` = clear the
    /// override (follow the direction's body color), a CSS color = pin it.
    pub fn set_series_wick_colors(&mut self, id: u32, up: Option<String>, down: Option<String>) {
        self.inner.borrow_mut().set_series_wick_colors(id, up, down);
    }

    /// Set candlestick border colors per direction; same keep/clear/pin contract as the wicks.
    pub fn set_series_border_colors(&mut self, id: u32, up: Option<String>, down: Option<String>) {
        self.inner
            .borrow_mut()
            .set_series_border_colors(id, up, down);
    }

    /// Toggle candlestick wick visibility (default visible).
    pub fn set_series_wick_visible(&mut self, id: u32, visible: bool) {
        self.inner.borrow_mut().set_series_wick_visible(id, visible);
    }

    /// Toggle candlestick body-border visibility (default visible).
    pub fn set_series_border_visible(&mut self, id: u32, visible: bool) {
        self.inner
            .borrow_mut()
            .set_series_border_visible(id, visible);
    }

    /// Set a line/area series' stroke width (css px).
    pub fn set_series_line_width(&mut self, id: u32, width: f64) {
        self.inner.borrow_mut().set_series_line_width(id, width);
    }

    /// Set an area series' fill gradient colors (top at the line, bottom at the base; CSS strings).
    pub fn set_series_area_colors(&mut self, id: u32, top: &str, bottom: &str) {
        self.inner
            .borrow_mut()
            .set_series_area_colors(id, top, bottom);
    }

    /// Color a histogram (volume) by the main price series' up/down direction per bar
    /// (industry-standard volume).
    pub fn set_series_histogram_updown(&mut self, id: u32, enabled: bool) {
        self.inner
            .borrow_mut()
            .set_series_histogram_updown(id, enabled);
    }

    /// Set a line/area series' join type: 0 = simple, 1 = stepped, 2 = curved. Call `render()`
    /// after (roadmap Phase B3).
    pub fn set_series_line_type(&mut self, id: u32, line_type: u8) {
        self.inner.borrow_mut().set_series_line_type(id, line_type);
    }

    /// Toggle per-point disc markers on a line/area series. Call `render()` after (Phase B3).
    pub fn set_series_point_markers(&mut self, id: u32, visible: bool) {
        self.inner
            .borrow_mut()
            .set_series_point_markers(id, visible);
    }

    /// Toggle the close tick on OHLC bars. Set this and `open_visible` false for high-low bars.
    pub fn set_series_close_visible(&mut self, id: u32, visible: bool) {
        self.inner
            .borrow_mut()
            .set_series_close_visible(id, visible);
    }

    /// Set a Baseline series' baseline price (`NaN` = auto). Call `render()` after (Phase B3).
    pub fn set_series_baseline(&mut self, id: u32, price: f64) {
        self.inner.borrow_mut().set_series_baseline(id, price);
    }

    /// Toggle the pulsing last-price ring on a series (roadmap Phase B3).
    pub fn set_series_last_price_animation(&mut self, id: u32, enabled: bool) {
        self.inner
            .borrow_mut()
            .set_series_last_price_animation(id, enabled);
    }

    /// Add a horizontal price line to a series; returns its id. `style`: 0 solid, 1 dotted, 2
    /// dashed, 3 large-dashed, 4 sparse-dotted. Call `render()` after (roadmap Phase B4).
    #[allow(clippy::too_many_arguments)]
    pub fn create_price_line(
        &mut self,
        series_id: u32,
        price: f64,
        r: u8,
        g: u8,
        b: u8,
        width: u32,
        style: u8,
        title: &str,
    ) -> u32 {
        self.inner
            .borrow_mut()
            .create_price_line(series_id, price, r, g, b, width, style, title)
    }
    /// Remove a price line by id. Call `render()` after (roadmap Phase B4).
    pub fn remove_price_line(&mut self, id: u32) {
        self.inner.borrow_mut().remove_price_line(id);
    }

    /// Merge a JSON options patch into an existing price line (reference `IPriceLine.applyOptions`;
    /// snake_case keys, camelCase aliases accepted). Call `render()` after.
    pub fn price_line_apply_options(&mut self, id: u32, json: &str) {
        self.inner.borrow_mut().price_line_apply_options(id, json);
    }

    /// The price line's full options as a snake_case JSON string (reference `IPriceLine.options`;
    /// "" for an unknown id).
    pub fn price_line_options_json(&self, id: u32) -> String {
        self.inner.borrow().price_line_options_json(id)
    }

    /// Replace a series' markers from a JSON array. Call `render()` after (roadmap Phase B4).
    pub fn set_series_markers(&mut self, series_id: u32, json: &str) -> bool {
        self.inner.borrow_mut().set_series_markers(series_id, json)
    }
    /// Toggle marker pixel margins in price-scale autoscaling (enabled by default, as in reference).
    pub fn set_series_markers_auto_scale(&mut self, series_id: u32, enabled: bool) {
        self.inner
            .borrow_mut()
            .set_series_markers_auto_scale(series_id, enabled);
    }
    /// Set the official marker layer (`normal`, `aboveSeries`, or `top`) by wire value.
    pub fn set_series_markers_z_order(&mut self, series_id: u32, z_order: u8) -> bool {
        self.inner
            .borrow_mut()
            .set_series_markers_z_order(series_id, z_order)
    }
    /// Whether any series wants the last-price pulse (host uses this to run/stop its rAF loop).
    pub fn wants_animation(&self) -> bool {
        self.inner.borrow().wants_animation()
    }
    /// Set the host animation clock (ms). Call before `render()` in the rAF loop (Phase B3).
    pub fn set_animation_time(&mut self, t_ms: f64) {
        self.inner.borrow_mut().set_animation_time(t_ms);
    }

    /// Pin the countdown clock (UTC seconds). Until the first pin the render path feeds the
    /// browser's system time every frame; the package's countdown timer pins once per second.
    pub fn set_now_seconds(&mut self, now: f64) {
        self.inner.borrow_mut().now_override = if now.is_finite() { Some(now) } else { None };
    }

    /// Move a series to the bottom-band overlay (volume) price scale with the given fractional
    /// margins (top/bottom of pane height). Call `render()` after (roadmap Phase B2).
    pub fn set_series_overlay(&mut self, id: u32, top: f64, bottom: f64) {
        self.inner.borrow_mut().set_series_overlay(id, top, bottom);
    }

    /// Move a series into stacked pane `pane_index` (0 = top/price pane), creating panes as needed;
    /// `stretch_factor` sizes a newly-created pane relative to the others. Call `render()` after
    /// (roadmap Phase B1).
    pub fn set_series_pane(&mut self, id: u32, pane_index: usize, stretch_factor: f64) {
        self.inner
            .borrow_mut()
            .set_series_pane(id, pane_index, stretch_factor);
    }
    pub fn try_set_series_pane(&mut self, id: u32, pane_index: usize, stretch_factor: f64) -> bool {
        self.inner
            .borrow_mut()
            .try_set_series_pane(id, pane_index, stretch_factor)
    }
    pub fn try_set_series_pane_and_scale(
        &mut self,
        id: u32,
        pane_index: usize,
        stretch_factor: f64,
        price_scale_id: &str,
    ) -> bool {
        self.inner.borrow_mut().try_set_series_pane_and_scale(
            id,
            pane_index,
            stretch_factor,
            price_scale_id,
        )
    }

    /// Number of stacked panes.
    pub fn pane_count(&self) -> usize {
        self.inner.borrow().pane_count()
    }
    /// Stable identity for a live pane at `index` (`undefined` for a stale index).
    pub fn pane_stable_id(&self, index: u32) -> Option<u32> {
        self.inner.borrow().pane_stable_id(index)
    }
    /// Current index for a stable pane identity (`undefined` after that pane is removed).
    pub fn pane_index_for_id(&self, stable_id: u32) -> Option<u32> {
        self.inner.borrow().pane_index_for_id(stable_id)
    }
    /// CSS Y of each pane boundary (for the host to hit-test separators).
    pub fn pane_separator_ys(&self) -> Vec<f64> {
        self.inner.borrow().pane_separator_ys()
    }
    /// JSON `{left, top, width, height}` of pane `i`'s content area in CSS px relative to the
    /// chart container — the anchor for platform-rendered per-pane chrome (indicator chips).
    pub fn pane_geometry_json(&self, i: usize) -> String {
        self.inner.borrow().pane_geometry_json(i)
    }

    /// Drag the separator below pane `i` by `delta_css`. Call `render()` after (roadmap Phase B1).
    pub fn drag_pane_separator(&mut self, i: usize, delta_css: f64) {
        self.inner.borrow_mut().drag_pane_separator(i, delta_css);
    }
    /// CSS height of pane `i` from the last layout pass (0 if out of range).
    pub fn pane_height(&self, i: usize) -> f64 {
        self.inner.borrow().pane_height(i)
    }
    /// Relative stretch factor of pane `i` (1 if out of range).
    pub fn pane_stretch(&self, i: usize) -> f64 {
        self.inner.borrow().pane_stretch(i)
    }
    /// Set pane `i`'s stretch factor (relative height weight). Call `render()` after.
    pub fn set_pane_stretch(&mut self, i: usize, factor: f64) {
        self.inner.borrow_mut().set_pane_stretch(i, factor);
    }
    /// Resize pane `i` to `height_css` px, taking the difference from its neighbour. Render after.
    pub fn set_pane_height(&mut self, i: usize, height_css: f64) {
        self.inner.borrow_mut().set_pane_height(i, height_css);
    }

    /// reference v5 `chart.addPane(preserveEmptyPane)`: append a pane and return its index.
    pub fn add_pane(&mut self, preserve_empty: bool) -> Option<u32> {
        self.inner.borrow_mut().add_pane(preserve_empty)
    }

    /// Add a pane with an explicit non-financial horizontal domain.
    pub fn add_general_pane_result_json(&mut self, options_json: &str) -> String {
        self.inner
            .borrow_mut()
            .add_general_pane_result_json(options_json)
    }

    /// Add one engine-owned general axis.
    pub fn add_general_axis_result_json(&mut self, options_json: &str) -> String {
        self.inner
            .borrow_mut()
            .add_general_axis_result_json(options_json)
    }

    pub fn update_general_axis_result_json(&mut self, options_json: &str) -> String {
        self.inner
            .borrow_mut()
            .update_general_axis_result_json(options_json)
    }

    pub fn general_axis_json(&self, id: &str) -> String {
        self.inner.borrow().general_axis_json(id)
    }

    pub fn general_axis_handle_token(&self, id: &str) -> u32 {
        self.inner.borrow().general_axis_handle_token(id)
    }

    /// General axis IDs in insertion order. `pane < 0` selects every pane.
    pub fn general_axis_ids_json(&self, pane: i32) -> String {
        self.inner.borrow().general_axis_ids_json(pane)
    }

    pub fn set_general_axis_visible(&mut self, id: &str, visible: bool) -> bool {
        self.inner
            .borrow_mut()
            .set_general_axis_visible(id, visible)
    }

    pub fn add_general_reference_result_json(&mut self, options_json: &str) -> String {
        self.inner
            .borrow_mut()
            .add_general_reference_result_json(options_json)
    }

    pub fn general_reference_options_json(&self, id: u32) -> String {
        self.inner.borrow().general_reference_options_json(id)
    }

    pub fn general_reference_ids_json(&self, pane: i32) -> String {
        self.inner.borrow().general_reference_ids_json(pane)
    }

    pub fn remove_general_reference(&mut self, id: u32) -> bool {
        self.inner.borrow_mut().remove_general_reference(id)
    }

    pub fn general_series_ids(&self, pane: usize) -> Vec<u32> {
        self.inner.borrow().general_series_ids(pane)
    }

    /// General-series paint and legend order (bottom first). `pane < 0` selects every pane.
    pub fn general_series_order_json(&self, pane: i32) -> String {
        self.inner.borrow().general_series_order_json(pane)
    }

    /// Reorder every general series in the selected scope. A bad permutation is rejected.
    pub fn set_general_series_order(&mut self, pane: i32, ids: Vec<u32>) -> bool {
        self.inner.borrow_mut().set_general_series_order(pane, ids)
    }

    pub fn general_series_catalog_json(&self) -> String {
        self.inner.borrow().general_series_catalog_json()
    }

    pub fn general_series_options_json(&self, series: u32) -> String {
        self.inner.borrow().general_series_options_json(series)
    }

    pub fn update_general_series_options_result_json(
        &mut self,
        series: u32,
        options_json: &str,
    ) -> String {
        self.inner
            .borrow_mut()
            .update_general_series_options_result_json(series, options_json)
    }

    /// Engine-owned general-series legend metadata. `pane < 0` selects every pane.
    pub fn general_legend_snapshot_json(&self, pane: i32) -> String {
        self.inner.borrow().general_legend_snapshot_json(pane)
    }

    pub fn remove_general_axis(&mut self, id: &str) -> bool {
        self.inner.borrow_mut().remove_general_axis(id)
    }

    pub fn pan_general_axis_result_json(&mut self, id: &str, fraction: f64) -> String {
        match self.inner.borrow_mut().pan_general_axis(id, fraction) {
            Ok(()) => serde_json::json!({ "ok": true, "result": null }).to_string(),
            Err(error) => serde_json::json!({ "ok": false, "error": { "code": error.code().name(), "message": error.message() } }).to_string(),
        }
    }

    pub fn zoom_general_axis_result_json(
        &mut self,
        id: &str,
        factor: f64,
        anchor_value: f64,
    ) -> String {
        match self
            .inner
            .borrow_mut()
            .zoom_general_axis(id, factor, anchor_value)
        {
            Ok(()) => serde_json::json!({ "ok": true, "result": null }).to_string(),
            Err(error) => serde_json::json!({ "ok": false, "error": { "code": error.code().name(), "message": error.message() } }).to_string(),
        }
    }

    pub fn zoom_general_category_axis_result_json(
        &mut self,
        id: &str,
        factor: f64,
        anchor_value: &str,
    ) -> String {
        match self
            .inner
            .borrow_mut()
            .zoom_general_category_axis(id, factor, anchor_value)
        {
            Ok(()) => serde_json::json!({ "ok": true, "result": null }).to_string(),
            Err(error) => serde_json::json!({ "ok": false, "error": { "code": error.code().name(), "message": error.message() } }).to_string(),
        }
    }

    pub fn reset_general_axis_view(&mut self, id: &str) -> bool {
        self.inner.borrow_mut().reset_general_axis_view(id)
    }

    pub fn add_general_series_result_json(&mut self, kind: &str, options_json: &str) -> String {
        self.inner
            .borrow_mut()
            .add_general_series_result_json(kind, options_json)
    }

    pub fn set_general_numeric_data_typed(
        &mut self,
        dataset: u32,
        metadata_json: &str,
        x: &Float64Array,
        y: &Float64Array,
        y_valid: Option<js_sys::Uint8Array>,
    ) -> String {
        self.inner.borrow_mut().set_general_numeric_data_typed(
            dataset,
            metadata_json,
            x,
            y,
            y_valid,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn set_general_bubble_data_typed(
        &mut self,
        dataset: u32,
        metadata_json: &str,
        x: &Float64Array,
        y: &Float64Array,
        y_valid: Option<js_sys::Uint8Array>,
        size: &Float64Array,
        size_valid: Option<js_sys::Uint8Array>,
    ) -> String {
        self.inner.borrow_mut().set_general_bubble_data_typed(
            dataset,
            metadata_json,
            x,
            y,
            y_valid,
            size,
            size_valid,
        )
    }

    pub fn set_general_temporal_data_typed(
        &mut self,
        dataset: u32,
        metadata_json: &str,
        x_epoch_ms: &Float64Array,
        y: &Float64Array,
        y_valid: Option<js_sys::Uint8Array>,
    ) -> String {
        self.inner.borrow_mut().set_general_temporal_data_typed(
            dataset,
            metadata_json,
            x_epoch_ms,
            y,
            y_valid,
        )
    }

    pub fn set_general_category_data_typed(
        &mut self,
        dataset: u32,
        ids_json: &str,
        metadata_json: &str,
        category_indices: &js_sys::Uint32Array,
        y: &Float64Array,
        y_valid: Option<js_sys::Uint8Array>,
    ) -> String {
        self.inner.borrow_mut().set_general_category_data_typed(
            dataset,
            ids_json,
            metadata_json,
            category_indices,
            y,
            y_valid,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn set_general_range_numeric_data_typed(
        &mut self,
        dataset: u32,
        metadata_json: &str,
        x: &Float64Array,
        low: &Float64Array,
        low_valid: Option<js_sys::Uint8Array>,
        high: &Float64Array,
        high_valid: Option<js_sys::Uint8Array>,
    ) -> String {
        self.inner
            .borrow_mut()
            .set_general_range_numeric_data_typed(
                dataset,
                metadata_json,
                x,
                low,
                low_valid,
                high,
                high_valid,
            )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn set_general_heatmap_category_data_typed(
        &mut self,
        dataset: u32,
        ids_json: &str,
        metadata_json: &str,
        x_category_indices: &js_sys::Uint32Array,
        y_category_indices: &js_sys::Uint32Array,
        value: &Float64Array,
        value_valid: Option<js_sys::Uint8Array>,
    ) -> String {
        self.inner
            .borrow_mut()
            .set_general_heatmap_category_data_typed(
                dataset,
                ids_json,
                metadata_json,
                x_category_indices,
                y_category_indices,
                value,
                value_valid,
            )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn set_general_heatmap_numeric_data_typed(
        &mut self,
        dataset: u32,
        metadata_json: &str,
        x: &Float64Array,
        y_coordinate: &Float64Array,
        value: &Float64Array,
        value_valid: Option<js_sys::Uint8Array>,
    ) -> String {
        self.inner
            .borrow_mut()
            .set_general_heatmap_numeric_data_typed(
                dataset,
                metadata_json,
                x,
                y_coordinate,
                value,
                value_valid,
            )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn set_general_heatmap_temporal_data_typed(
        &mut self,
        dataset: u32,
        metadata_json: &str,
        x_epoch_ms: &Float64Array,
        y_coordinate: &Float64Array,
        value: &Float64Array,
        value_valid: Option<js_sys::Uint8Array>,
    ) -> String {
        self.inner
            .borrow_mut()
            .set_general_heatmap_temporal_data_typed(
                dataset,
                metadata_json,
                x_epoch_ms,
                y_coordinate,
                value,
                value_valid,
            )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn set_general_error_numeric_data_typed(
        &mut self,
        dataset: u32,
        metadata_json: &str,
        x: &Float64Array,
        y: &Float64Array,
        y_valid: Option<js_sys::Uint8Array>,
        x_low: &Float64Array,
        x_low_valid: Option<js_sys::Uint8Array>,
        x_high: &Float64Array,
        x_high_valid: Option<js_sys::Uint8Array>,
        y_low: &Float64Array,
        y_low_valid: Option<js_sys::Uint8Array>,
        y_high: &Float64Array,
        y_high_valid: Option<js_sys::Uint8Array>,
    ) -> String {
        self.inner
            .borrow_mut()
            .set_general_error_numeric_data_typed(
                dataset,
                metadata_json,
                x,
                y,
                y_valid,
                x_low,
                x_low_valid,
                x_high,
                x_high_valid,
                y_low,
                y_low_valid,
                y_high,
                y_high_valid,
            )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn set_general_range_temporal_data_typed(
        &mut self,
        dataset: u32,
        metadata_json: &str,
        x_epoch_ms: &Float64Array,
        low: &Float64Array,
        low_valid: Option<js_sys::Uint8Array>,
        high: &Float64Array,
        high_valid: Option<js_sys::Uint8Array>,
    ) -> String {
        self.inner
            .borrow_mut()
            .set_general_range_temporal_data_typed(
                dataset,
                metadata_json,
                x_epoch_ms,
                low,
                low_valid,
                high,
                high_valid,
            )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn set_general_error_temporal_data_typed(
        &mut self,
        dataset: u32,
        metadata_json: &str,
        x_epoch_ms: &Float64Array,
        y: &Float64Array,
        y_valid: Option<js_sys::Uint8Array>,
        x_low_epoch_ms: &Float64Array,
        x_low_valid: Option<js_sys::Uint8Array>,
        x_high_epoch_ms: &Float64Array,
        x_high_valid: Option<js_sys::Uint8Array>,
        y_low: &Float64Array,
        y_low_valid: Option<js_sys::Uint8Array>,
        y_high: &Float64Array,
        y_high_valid: Option<js_sys::Uint8Array>,
    ) -> String {
        self.inner
            .borrow_mut()
            .set_general_error_temporal_data_typed(
                dataset,
                metadata_json,
                x_epoch_ms,
                y,
                y_valid,
                x_low_epoch_ms,
                x_low_valid,
                x_high_epoch_ms,
                x_high_valid,
                y_low,
                y_low_valid,
                y_high,
                y_high_valid,
            )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn set_general_range_category_data_typed(
        &mut self,
        dataset: u32,
        ids_json: &str,
        metadata_json: &str,
        category_indices: &js_sys::Uint32Array,
        low: &Float64Array,
        low_valid: Option<js_sys::Uint8Array>,
        high: &Float64Array,
        high_valid: Option<js_sys::Uint8Array>,
    ) -> String {
        self.inner
            .borrow_mut()
            .set_general_range_category_data_typed(
                dataset,
                ids_json,
                metadata_json,
                category_indices,
                low,
                low_valid,
                high,
                high_valid,
            )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn set_general_error_category_data_typed(
        &mut self,
        dataset: u32,
        ids_json: &str,
        metadata_json: &str,
        category_indices: &js_sys::Uint32Array,
        y: &Float64Array,
        y_valid: Option<js_sys::Uint8Array>,
        y_low: &Float64Array,
        y_low_valid: Option<js_sys::Uint8Array>,
        y_high: &Float64Array,
        y_high_valid: Option<js_sys::Uint8Array>,
    ) -> String {
        self.inner
            .borrow_mut()
            .set_general_error_category_data_typed(
                dataset,
                ids_json,
                metadata_json,
                category_indices,
                y,
                y_valid,
                y_low,
                y_low_valid,
                y_high,
                y_high_valid,
            )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn set_general_box_category_data_typed(
        &mut self,
        dataset: u32,
        ids_json: &str,
        metadata_json: &str,
        category_indices: &js_sys::Uint32Array,
        min: &Float64Array,
        min_valid: Option<js_sys::Uint8Array>,
        q1: &Float64Array,
        q1_valid: Option<js_sys::Uint8Array>,
        median: &Float64Array,
        median_valid: Option<js_sys::Uint8Array>,
        q3: &Float64Array,
        q3_valid: Option<js_sys::Uint8Array>,
        max: &Float64Array,
        max_valid: Option<js_sys::Uint8Array>,
    ) -> String {
        self.inner.borrow_mut().set_general_box_category_data_typed(
            dataset,
            ids_json,
            metadata_json,
            category_indices,
            min,
            min_valid,
            q1,
            q1_valid,
            median,
            median_valid,
            q3,
            q3_valid,
            max,
            max_valid,
        )
    }

    pub fn upsert_general_numeric_data_typed(
        &mut self,
        dataset: u32,
        metadata_json: &str,
        x: &Float64Array,
        y: &Float64Array,
        y_valid: Option<js_sys::Uint8Array>,
        max_rows: u32,
    ) -> String {
        self.inner.borrow_mut().upsert_general_numeric_data_typed(
            dataset,
            metadata_json,
            x,
            y,
            y_valid,
            max_rows,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn upsert_general_bubble_data_typed(
        &mut self,
        dataset: u32,
        metadata_json: &str,
        x: &Float64Array,
        y: &Float64Array,
        y_valid: Option<js_sys::Uint8Array>,
        size: &Float64Array,
        size_valid: Option<js_sys::Uint8Array>,
        max_rows: u32,
    ) -> String {
        self.inner.borrow_mut().upsert_general_bubble_data_typed(
            dataset,
            metadata_json,
            x,
            y,
            y_valid,
            size,
            size_valid,
            max_rows,
        )
    }

    pub fn upsert_general_temporal_data_typed(
        &mut self,
        dataset: u32,
        metadata_json: &str,
        x_epoch_ms: &Float64Array,
        y: &Float64Array,
        y_valid: Option<js_sys::Uint8Array>,
        max_rows: u32,
    ) -> String {
        self.inner.borrow_mut().upsert_general_temporal_data_typed(
            dataset,
            metadata_json,
            x_epoch_ms,
            y,
            y_valid,
            max_rows,
        )
    }

    pub fn upsert_general_category_data_typed(
        &mut self,
        dataset: u32,
        ids_json: &str,
        update_json: &str,
        category_indices: &js_sys::Uint32Array,
        y: &Float64Array,
        y_valid: Option<js_sys::Uint8Array>,
    ) -> String {
        self.inner.borrow_mut().upsert_general_category_data_typed(
            dataset,
            ids_json,
            update_json,
            category_indices,
            y,
            y_valid,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn upsert_general_range_numeric_data_typed(
        &mut self,
        dataset: u32,
        metadata_json: &str,
        x: &Float64Array,
        low: &Float64Array,
        low_valid: Option<js_sys::Uint8Array>,
        high: &Float64Array,
        high_valid: Option<js_sys::Uint8Array>,
        max_rows: u32,
    ) -> String {
        self.inner
            .borrow_mut()
            .upsert_general_range_numeric_data_typed(
                dataset,
                metadata_json,
                x,
                low,
                low_valid,
                high,
                high_valid,
                max_rows,
            )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn upsert_general_heatmap_category_data_typed(
        &mut self,
        dataset: u32,
        ids_json: &str,
        update_json: &str,
        x_category_indices: &js_sys::Uint32Array,
        y_category_indices: &js_sys::Uint32Array,
        value: &Float64Array,
        value_valid: Option<js_sys::Uint8Array>,
    ) -> String {
        self.inner
            .borrow_mut()
            .upsert_general_heatmap_category_data_typed(
                dataset,
                ids_json,
                update_json,
                x_category_indices,
                y_category_indices,
                value,
                value_valid,
            )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn upsert_general_heatmap_numeric_data_typed(
        &mut self,
        dataset: u32,
        metadata_json: &str,
        x: &Float64Array,
        y_coordinate: &Float64Array,
        value: &Float64Array,
        value_valid: Option<js_sys::Uint8Array>,
        max_rows: u32,
    ) -> String {
        self.inner
            .borrow_mut()
            .upsert_general_heatmap_numeric_data_typed(
                dataset,
                metadata_json,
                x,
                y_coordinate,
                value,
                value_valid,
                max_rows,
            )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn upsert_general_heatmap_temporal_data_typed(
        &mut self,
        dataset: u32,
        metadata_json: &str,
        x_epoch_ms: &Float64Array,
        y_coordinate: &Float64Array,
        value: &Float64Array,
        value_valid: Option<js_sys::Uint8Array>,
        max_rows: u32,
    ) -> String {
        self.inner
            .borrow_mut()
            .upsert_general_heatmap_temporal_data_typed(
                dataset,
                metadata_json,
                x_epoch_ms,
                y_coordinate,
                value,
                value_valid,
                max_rows,
            )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn upsert_general_error_numeric_data_typed(
        &mut self,
        dataset: u32,
        metadata_json: &str,
        x: &Float64Array,
        y: &Float64Array,
        y_valid: Option<js_sys::Uint8Array>,
        x_low: &Float64Array,
        x_low_valid: Option<js_sys::Uint8Array>,
        x_high: &Float64Array,
        x_high_valid: Option<js_sys::Uint8Array>,
        y_low: &Float64Array,
        y_low_valid: Option<js_sys::Uint8Array>,
        y_high: &Float64Array,
        y_high_valid: Option<js_sys::Uint8Array>,
        max_rows: u32,
    ) -> String {
        self.inner
            .borrow_mut()
            .upsert_general_error_numeric_data_typed(
                dataset,
                metadata_json,
                x,
                y,
                y_valid,
                x_low,
                x_low_valid,
                x_high,
                x_high_valid,
                y_low,
                y_low_valid,
                y_high,
                y_high_valid,
                max_rows,
            )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn upsert_general_range_temporal_data_typed(
        &mut self,
        dataset: u32,
        metadata_json: &str,
        x_epoch_ms: &Float64Array,
        low: &Float64Array,
        low_valid: Option<js_sys::Uint8Array>,
        high: &Float64Array,
        high_valid: Option<js_sys::Uint8Array>,
        max_rows: u32,
    ) -> String {
        self.inner
            .borrow_mut()
            .upsert_general_range_temporal_data_typed(
                dataset,
                metadata_json,
                x_epoch_ms,
                low,
                low_valid,
                high,
                high_valid,
                max_rows,
            )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn upsert_general_error_temporal_data_typed(
        &mut self,
        dataset: u32,
        metadata_json: &str,
        x_epoch_ms: &Float64Array,
        y: &Float64Array,
        y_valid: Option<js_sys::Uint8Array>,
        x_low_epoch_ms: &Float64Array,
        x_low_valid: Option<js_sys::Uint8Array>,
        x_high_epoch_ms: &Float64Array,
        x_high_valid: Option<js_sys::Uint8Array>,
        y_low: &Float64Array,
        y_low_valid: Option<js_sys::Uint8Array>,
        y_high: &Float64Array,
        y_high_valid: Option<js_sys::Uint8Array>,
        max_rows: u32,
    ) -> String {
        self.inner
            .borrow_mut()
            .upsert_general_error_temporal_data_typed(
                dataset,
                metadata_json,
                x_epoch_ms,
                y,
                y_valid,
                x_low_epoch_ms,
                x_low_valid,
                x_high_epoch_ms,
                x_high_valid,
                y_low,
                y_low_valid,
                y_high,
                y_high_valid,
                max_rows,
            )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn upsert_general_range_category_data_typed(
        &mut self,
        dataset: u32,
        ids_json: &str,
        update_json: &str,
        category_indices: &js_sys::Uint32Array,
        low: &Float64Array,
        low_valid: Option<js_sys::Uint8Array>,
        high: &Float64Array,
        high_valid: Option<js_sys::Uint8Array>,
    ) -> String {
        self.inner
            .borrow_mut()
            .upsert_general_range_category_data_typed(
                dataset,
                ids_json,
                update_json,
                category_indices,
                low,
                low_valid,
                high,
                high_valid,
            )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn upsert_general_error_category_data_typed(
        &mut self,
        dataset: u32,
        ids_json: &str,
        update_json: &str,
        category_indices: &js_sys::Uint32Array,
        y: &Float64Array,
        y_valid: Option<js_sys::Uint8Array>,
        y_low: &Float64Array,
        y_low_valid: Option<js_sys::Uint8Array>,
        y_high: &Float64Array,
        y_high_valid: Option<js_sys::Uint8Array>,
    ) -> String {
        self.inner
            .borrow_mut()
            .upsert_general_error_category_data_typed(
                dataset,
                ids_json,
                update_json,
                category_indices,
                y,
                y_valid,
                y_low,
                y_low_valid,
                y_high,
                y_high_valid,
            )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn upsert_general_box_category_data_typed(
        &mut self,
        dataset: u32,
        ids_json: &str,
        update_json: &str,
        category_indices: &js_sys::Uint32Array,
        min: &Float64Array,
        min_valid: Option<js_sys::Uint8Array>,
        q1: &Float64Array,
        q1_valid: Option<js_sys::Uint8Array>,
        median: &Float64Array,
        median_valid: Option<js_sys::Uint8Array>,
        q3: &Float64Array,
        q3_valid: Option<js_sys::Uint8Array>,
        max: &Float64Array,
        max_valid: Option<js_sys::Uint8Array>,
    ) -> String {
        self.inner
            .borrow_mut()
            .upsert_general_box_category_data_typed(
                dataset,
                ids_json,
                update_json,
                category_indices,
                min,
                min_valid,
                q1,
                q1_valid,
                median,
                median_valid,
                q3,
                q3_valid,
                max,
                max_valid,
            )
    }

    pub fn remove_general_series(&mut self, series: u32, dataset: u32) -> bool {
        self.inner
            .borrow_mut()
            .remove_general_series(series, dataset)
    }

    pub fn set_general_series_visible(&mut self, series: u32, visible: bool) -> bool {
        self.inner
            .borrow_mut()
            .set_general_series_visible(series, visible)
    }

    pub fn general_tooltip_json(&self, series: u32, row: usize) -> String {
        self.inner.borrow().general_tooltip_json(series, row)
    }

    pub fn general_shared_tooltip_json(&self, series: u32, row: usize) -> String {
        self.inner.borrow().general_shared_tooltip_json(series, row)
    }

    pub fn set_general_brush_result_json(
        &mut self,
        axis_id: &str,
        from_css: f64,
        to_css: f64,
    ) -> String {
        self.inner
            .borrow_mut()
            .set_general_brush_result_json(axis_id, from_css, to_css)
    }

    pub fn general_brush_snapshot_json(&self) -> String {
        self.inner.borrow().general_brush_snapshot_json()
    }

    pub fn clear_general_brush(&mut self) {
        self.inner.borrow_mut().clear_general_brush();
    }

    pub fn general_accessibility_json(&self, series: u32, offset: usize, limit: usize) -> String {
        self.inner
            .borrow()
            .general_accessibility_json(series, offset, limit)
    }

    /// `max_distance < 0` requests exact hit testing.
    pub fn general_hit_test_json(&self, pane: usize, x: f64, y: f64, max_distance: f64) -> String {
        self.inner
            .borrow()
            .general_hit_test_json(pane, x, y, max_distance)
    }

    pub fn general_selected_hit_json(&self) -> String {
        self.inner.borrow().general_selected_hit_json()
    }

    pub fn general_accessibility_focused_hit_json(&self) -> String {
        self.inner.borrow().general_accessibility_focused_hit_json()
    }

    pub fn select_general_hovered(&mut self) -> bool {
        self.inner.borrow_mut().select_general_hovered()
    }

    pub fn clear_general_selection(&mut self) {
        self.inner.borrow_mut().clear_general_selection();
    }

    pub fn set_general_accessibility_focus(&mut self, series: u32, row: usize) -> bool {
        self.inner
            .borrow_mut()
            .set_general_accessibility_focus(series, row)
    }

    pub fn clear_general_accessibility_focus(&mut self) {
        self.inner.borrow_mut().clear_general_accessibility_focus();
    }

    /// Versioned semantic chart-state export. The JSON envelope contains either a V1 document or
    /// a structured public error; runtime caches and host market data are never included.
    pub fn export_state_result_json(&self) -> String {
        self.inner.borrow().export_state_result_json()
    }

    /// Validate and atomically restore a V1 semantic chart-state document.
    pub fn import_state_result_json(&mut self, document: &str) -> String {
        self.inner.borrow_mut().import_state_result_json(document)
    }

    /// reference `chart.removePane`: rejects stale indices and a non-empty final pane. An empty
    /// preserved final pane is retired and replaced by a fresh default pane so its handle stales
    /// while the engine retains one layout slot. Pane series become pane-less until re-assigned;
    /// panes below shift one index up. Call `render()` after.
    pub fn remove_pane(&mut self, index: u32) -> bool {
        self.inner.borrow_mut().remove_pane(index)
    }

    /// reference `chart.swapPanes`: the two panes trade places — series assignments, stretch
    /// factors, scales, and preserve flags ride along. Call `render()` after.
    pub fn swap_panes(&mut self, first: u32, second: u32) -> bool {
        self.inner.borrow_mut().swap_panes(first, second)
    }

    /// reference `IPaneApi.moveTo`: relocate the pane (with its series) to a new index. False for
    /// a stale index. Call `render()` after.
    pub fn pane_move_to(&mut self, index: u32, target: u32) -> bool {
        self.inner.borrow_mut().pane_move_to(index, target)
    }

    /// reference `IPaneApi.preserveEmptyPane` (false for a stale index).
    pub fn pane_preserve_empty(&self, index: u32) -> bool {
        self.inner.borrow().pane_preserve_empty(index)
    }

    /// reference `IPaneApi.setPreserveEmptyPane`: an empty pane collapses on the next series
    /// removal/move-out unless this flag holds it open.
    pub fn pane_set_preserve_empty(&mut self, index: u32, flag: bool) {
        self.inner.borrow_mut().pane_set_preserve_empty(index, flag);
    }

    /// reference `IPaneApi.getSeries`: the pane's live series ids in render order (bottom first).
    pub fn pane_series_ids(&self, index: u32) -> Vec<u32> {
        self.inner.borrow().pane_series_ids(index)
    }

    /// Attach a pane primitive (reference `IPaneApi.attachPrimitive`, plugin platform Phase C-a):
    /// a plain JS object with optional `attached`/`detached`/`update_all_views`/`pane_views`/
    /// `price_axis_views`/`time_axis_views` hooks. Its `pane_views()` renderers record Prim
    /// commands through a host-built draw context instead of touching a canvas, so the output
    /// feeds both backends identically. Returns the primitive id (0 = rejected), used by
    /// [`detach_pane_primitive`]. Call `render()` after.
    pub fn attach_pane_primitive(&mut self, pane: u32, primitive: js_sys::Object) -> u32 {
        self.inner
            .borrow_mut()
            .attach_pane_primitive(pane, primitive)
    }

    /// Detach a pane primitive by id (reference `IPaneApi.detachPrimitive`): fires its `detached`
    /// hook and drops the retained JS object. False for an unknown id. Call `render()` after.
    pub fn detach_pane_primitive(&mut self, id: u32) -> bool {
        self.inner.borrow_mut().detach_pane_primitive(id)
    }

    /// Attach a series primitive (reference `ISeriesApi.attachPrimitive`, plugin platform Phase C-b):
    /// a plain JS object like [`attach_pane_primitive`] plus an optional `autoscale_info(from,
    /// to)` hook merged into the owning series' price-scale range. Its `pane_views()` renderers
    /// get the same command-recording context, except `price_to_y(price)` is bound to the
    /// owning series' scale. Returns the primitive id (0 = rejected), used by
    /// [`detach_series_primitive`]. Call `render()` after.
    pub fn attach_series_primitive(&mut self, series_id: u32, primitive: js_sys::Object) -> u32 {
        self.inner
            .borrow_mut()
            .attach_series_primitive(series_id, primitive)
    }

    /// Detach a series primitive by id (reference `ISeriesApi.detachPrimitive`): fires its `detached`
    /// hook and drops the retained JS object. False for an unknown id. Removing the owning
    /// series auto-detaches all its primitives. Call `render()` after.
    pub fn detach_series_primitive(&mut self, id: u32) -> bool {
        self.inner.borrow_mut().detach_series_primitive(id)
    }

    /// Add a custom series (plugin platform Phase C-c; reference `IChartApi.addCustomSeries`): a
    /// user-defined series type whose pane view (`price_value_builder`, `is_whitespace?`,
    /// `render`, plus optional `default_options`/`destroy`) renders each bar through the
    /// command-recording draw context, so its output is pixel-identical on both backends.
    /// `adopt_primary` converts the engine's construction-time series 0 (the TS package's
    /// first-series adoption, mirroring `add_series`). Returns the series id (u32::MAX =
    /// rejected). Call `render()` after.
    pub fn add_custom_series(&mut self, pane_view: js_sys::Object, adopt_primary: bool) -> u32 {
        self.inner
            .borrow_mut()
            .add_custom_series(pane_view, adopt_primary)
    }

    /// Replace a custom series' items (reference `ISeriesApi.setData`): a JS array of `{time, ...}`
    /// objects (times in UTC seconds). The raw items are stored host-side verbatim; their
    /// times enter the engine as whitespace-style rows. Call `render()` after.
    pub fn set_custom_series_data(&mut self, id: u32, items: js_sys::Array) -> Option<String> {
        self.inner.borrow_mut().set_custom_series_data(id, items)
    }

    /// Streaming update of a custom series (reference `ISeriesApi.update`): append a new time or
    /// replace the item at an existing one. Call `render()` after.
    pub fn update_custom_series_item(&mut self, id: u32, item: JsValue) -> Option<String> {
        self.inner.borrow_mut().update_custom_series_item(id, item)
    }

    /// The custom series' raw items aligned with the engine rows (post-sanitize order),
    /// backing the TS `series.data()` (`null` for an unknown id).
    pub fn custom_series_data(&self, id: u32) -> JsValue {
        self.inner.borrow().custom_series_data(id)
    }

    /// The custom item at a logical index (the engine plot's mismatch-direction search),
    /// backing the TS `series.data_by_index` (`null` off the data or for an unknown id).
    pub fn custom_series_data_by_index(&self, id: u32, index: f64, mismatch: i8) -> JsValue {
        self.inner
            .borrow()
            .custom_series_data_by_index(id, index, mismatch)
    }

    /// Merge a snake_case JSON patch of price-scale options into one pane scale (reference
    /// `priceScale.applyOptions`; unknown keys ignored). Keys: `mode`, `auto_scale`,
    /// `invert_scale`, `scale_margins`, `align_labels`, `ticks_visible`, `entire_text_only`,
    /// `minimum_width`, `text_color` (`""`/`null` = follow `layout.textColor`).
    pub fn price_scale_apply_options_json(&mut self, pane: u32, target: u32, json: &str) {
        self.inner
            .borrow_mut()
            .price_scale_apply_options_json(pane, target, json);
    }

    /// One pane scale's full options as a snake_case JSON string (reference `priceScale.options()`;
    /// "" for an unknown pane/target).
    pub fn price_scale_options_json(&self, pane: u32, target: u32) -> String {
        self.inner.borrow().price_scale_options_json(pane, target)
    }

    pub fn add_price_scale_result_json(&mut self, pane: u32, json: &str) -> String {
        self.inner
            .borrow_mut()
            .add_price_scale_result_json(pane, json)
    }
    pub fn price_scales_json(&self, pane: u32) -> String {
        self.inner.borrow().price_scales_json(pane)
    }
    pub fn price_scale_target_by_id(&self, pane: u32, id: &str) -> Option<u32> {
        self.inner.borrow().price_scale_target_by_id(pane, id)
    }
    pub fn move_price_scale_result_json(
        &mut self,
        pane: u32,
        target: u32,
        side: &str,
        order: usize,
    ) -> String {
        self.inner
            .borrow_mut()
            .move_price_scale_result_json(pane, target, side, order)
    }
    pub fn remove_price_scale_result_json(&mut self, pane: u32, target: u32) -> String {
        self.inner
            .borrow_mut()
            .remove_price_scale_result_json(pane, target)
    }

    /// 0 = candlestick, 1 = OHLC bars, 2 = line, 3 = area, 4 = histogram (sets the main series).
    pub fn set_series_type(&mut self, kind: u8) {
        self.inner.borrow_mut().set_series_type(kind);
    }
    pub fn set_series_kind(&mut self, id: u32, kind: u8) -> bool {
        self.inner.borrow_mut().set_series_kind(id, kind)
    }

    pub fn set_time_visible(&mut self, visible: bool) {
        self.inner.borrow_mut().set_time_visible(visible);
    }

    /// reference `timeScale.visible` (default true): reserve/paint the whole time-axis strip. When
    /// false the strip collapses to zero height. Distinct from `set_time_visible`, which only
    /// governs label content.
    pub fn set_time_axis_visible(&mut self, visible: bool) {
        self.inner.borrow_mut().set_time_axis_visible(visible);
    }

    /// reference `timeScale.ticksVisible` (default false): tick marks beside the time-axis labels.
    pub fn set_time_ticks_visible(&mut self, visible: bool) {
        self.inner.borrow_mut().set_time_ticks_visible(visible);
    }

    /// reference `timeScale.minimumHeight` (CSS px; 0 = the 28px auto height): floor for the
    /// time-axis strip height.
    pub fn set_time_axis_minimum_height(&mut self, height: f64) {
        self.inner.borrow_mut().set_time_axis_minimum_height(height);
    }

    /// reference `timeScale.tickMarkMaxCharacterLength` (default 8; 0 restores it): tick-label
    /// width cap in characters, driving tick density.
    pub fn set_tick_mark_max_character_length(&mut self, n: u32) {
        self.inner
            .borrow_mut()
            .set_tick_mark_max_character_length(n);
    }

    /// Set the hovered pane separator for the `layout.panes.separatorHoverColor` band
    /// (reference pane-separator.ts hover handle; -1 = none). Call `render()` to repaint.
    pub fn set_separator_hover(&mut self, index: i32) {
        self.inner.borrow_mut().set_separator_hover(index);
    }

    /// reference `timeScale.secondsVisible`: include seconds in time labels.
    pub fn set_seconds_visible(&mut self, visible: bool) {
        self.inner.borrow_mut().set_seconds_visible(visible);
    }

    /// reference `timeScale.minBarSpacing` (CSS px).
    pub fn set_min_bar_spacing(&mut self, spacing: f64) {
        self.inner.borrow_mut().set_min_bar_spacing(spacing);
    }

    /// reference `timeScale.maxBarSpacing` (CSS px; 0 restores the default half-width cap).
    pub fn set_max_bar_spacing(&mut self, spacing: f64) {
        self.inner.borrow_mut().set_max_bar_spacing(spacing);
    }

    /// reference `timeScale().applyOptions({ barSpacing })`: write the option and apply it live.
    pub fn apply_bar_spacing_option(&mut self, spacing: f64) {
        self.inner.borrow_mut().apply_bar_spacing_option(spacing);
    }

    /// reference `timeScale().applyOptions({ rightOffset })`: write the option and apply it live.
    pub fn apply_right_offset_option(&mut self, offset: f64) {
        self.inner.borrow_mut().apply_right_offset_option(offset);
    }

    /// reference `timeScale.rightOffsetPixels` (px): pin the right offset in pixels, converting to a
    /// bar offset via the current bar spacing exactly like reference time-scale.ts.
    pub fn set_right_offset_pixels(&mut self, pixels: f64) {
        self.inner.borrow_mut().set_right_offset_pixels(pixels);
    }

    /// reference `timeScale.fixLeftEdge`.
    pub fn set_fix_left_edge(&mut self, fix: bool) {
        self.inner.borrow_mut().set_fix_left_edge(fix);
    }

    /// reference `timeScale.fixRightEdge`.
    pub fn set_fix_right_edge(&mut self, fix: bool) {
        self.inner.borrow_mut().set_fix_right_edge(fix);
    }

    /// reference `timeScale.lockVisibleTimeRangeOnResize`.
    pub fn set_lock_visible_time_range_on_resize(&mut self, lock: bool) {
        self.inner
            .borrow_mut()
            .set_lock_visible_time_range_on_resize(lock);
    }

    /// reference `timeScale.rightBarStaysOnScroll`.
    pub fn set_right_bar_stays_on_scroll(&mut self, stays: bool) {
        self.inner.borrow_mut().set_right_bar_stays_on_scroll(stays);
    }

    /// Aeris `timeScale.lock_visible_logical_range` (default false): hold the visible logical
    /// range exactly across data updates and resizes (fixed full-session views).
    pub fn set_lock_visible_logical_range(&mut self, lock: bool) {
        self.inner.borrow_mut().set_lock_visible_logical_range(lock);
    }

    /// reference `timeScale.shiftVisibleRangeOnNewBar` (default true): when the last bar is
    /// visible, the view follows newly appended bars; scrolled back, the same bars stay.
    pub fn set_shift_visible_range_on_new_bar(&mut self, shift: bool) {
        self.inner
            .borrow_mut()
            .set_shift_visible_range_on_new_bar(shift);
    }

    /// reference `timeScale.allowShiftVisibleRangeOnWhitespaceReplacement` (default false): also
    /// follow when the new bar replaces an existing whitespace time point.
    pub fn set_allow_shift_visible_range_on_whitespace_replacement(&mut self, allow: bool) {
        self.inner
            .borrow_mut()
            .set_allow_shift_visible_range_on_whitespace_replacement(allow);
    }

    /// reference `timeScale.allowBoldLabels` (default true): bold the major time tick labels.
    pub fn set_allow_bold_labels(&mut self, allow: bool) {
        self.inner.borrow_mut().set_allow_bold_labels(allow);
    }

    /// reference `localization.dateFormat` (default `dd MMM \'yy`): the crosshair time-label
    /// pattern. Tokens: `dd`/`d`, `MM`/`M`/`MMM`/`MMMM`, `yy`/`yyyy`, `'…'` quoted literals.
    pub fn set_date_format(&mut self, pattern: &str) {
        self.inner.borrow_mut().set_date_format(pattern);
    }

    /// reference `localization.locale`: regenerate the engine's month-name tables from
    /// `Intl.DateTimeFormat` for this locale (drives the `MMM`/`MMMM` date-format tokens and
    /// the month tick labels). Unsupported tags warn and keep the current tables.
    pub fn set_locale(&mut self, locale: &str) {
        self.inner.borrow_mut().set_locale(locale);
    }

    /// Push the host's "all scaling and scrolling disabled" aggregate (reference
    /// `_isAllScalingAndScrollingDisabled`): forces fix-edge semantics on the time scale.
    pub fn set_interaction_disabled(&mut self, disabled: bool) {
        self.inner.borrow_mut().set_interaction_disabled(disabled);
    }

    /// reference `localization.priceFormatter`: `(price: number) => string`. Pass `null` to clear.
    pub fn set_price_formatter(&mut self, f: Option<js_sys::Function>) {
        self.inner.borrow_mut().set_price_formatter(f);
    }

    /// reference `timeScale.tickMarkFormatter`: `(timeSeconds, tickMarkType) => string`. `null` clears.
    pub fn set_tick_mark_formatter(&mut self, f: Option<js_sys::Function>) {
        self.inner.borrow_mut().set_tick_mark_formatter(f);
    }

    /// reference `localization.timeFormatter`: `(timeSeconds: number) => string`. Pass `null` to clear.
    pub fn set_time_formatter(&mut self, f: Option<js_sys::Function>) {
        self.inner.borrow_mut().set_time_formatter(f);
    }

    /// Exchange time zone / trading-day start / explicit time-axis marks (`{"timeZone": "UTC" |
    /// transitions, "sessionStart": seconds, "tickMarks": [{time, label?}] | null}`), validated
    /// together. Returns "" on success or the validation message; a rejection changes nothing.
    pub fn set_exchange_time_json(&mut self, time_scale_json: &str) -> String {
        self.inner
            .borrow_mut()
            .set_exchange_time_json(time_scale_json)
    }

    /// Whether the financial time points are calendar dates (business-day input).
    pub fn set_calendar_date_axis(&mut self, calendar_dates: bool) {
        self.inner
            .borrow_mut()
            .set_calendar_date_axis(calendar_dates);
    }

    /// Exchange-local wall-clock seconds for a UTC timestamp.
    pub fn exchange_local_seconds(&self, time: f64) -> f64 {
        self.inner.borrow().exchange_local_seconds(time)
    }

    /// Bit 0 `timeVisible`, bit 1 `secondsVisible`, bit 2 calendar-date axis.
    pub fn time_label_flags(&self) -> u32 {
        self.inner.borrow().time_label_flags()
    }

    /// 0 = normal, 1 = magnet (reference default), 2 = hidden, 3 = magnet OHLC.
    pub fn set_crosshair_mode(&mut self, mode: u8) {
        self.inner.borrow_mut().set_crosshair_mode(mode);
    }

    /// Deep-merge a JSON options patch (reference `applyOptions` semantics) — e.g.
    /// `{"grid":{"vertLines":{"color":"#334"}},"layout":{"background":{"color":"#111"}}}`.
    /// Malformed JSON is ignored with a console warning. Call `render()` after (roadmap Phase A2).
    pub fn apply_options(&mut self, patch_json: &str) {
        self.inner.borrow_mut().apply_options(patch_json);
    }

    /// Restore Aeris-owned visual defaults without changing the live view or chart contents.
    /// The browser host supplies its selected light/dark theme because theme choice is package state.
    pub fn reset_style_to_defaults(&mut self, light_theme: bool) {
        let theme = if light_theme {
            ChartTheme::Light
        } else {
            ChartTheme::Dark
        };
        self.inner.borrow_mut().reset_style_to_defaults(theme);
    }

    /// Current (deep-merged) chart options as a JSON string.
    pub fn options_json(&self) -> String {
        self.inner.borrow().options_json()
    }

    /// A series' current options as a snake_case JSON string (TS `series_options` field
    /// names; "" for an unknown/removed id).
    pub fn series_options_json(&self, id: u32) -> String {
        self.inner.borrow().series_options_json(id)
    }

    /// Merge a snake_case JSON patch of series style options (reference `series.applyOptions`;
    /// unknown keys are ignored gracefully). Call `render()` after.
    pub fn series_apply_options_json(&mut self, id: u32, json: &str) {
        self.inner.borrow_mut().series_apply_options_json(id, json);
    }

    /// All time-scale options as a snake_case JSON string (`bar_spacing`, `right_offset`,
    /// `min_bar_spacing`, `max_bar_spacing`, `right_offset_pixels`, `time_visible`,
    /// `seconds_visible`, `fix_left_edge`, `fix_right_edge`,
    /// `lock_visible_time_range_on_resize`, `right_bar_stays_on_scroll`,
    /// `shift_visible_range_on_new_bar`,
    /// `allow_shift_visible_range_on_whitespace_replacement`, `lock_visible_logical_range`).
    pub fn time_scale_options_json(&self) -> String {
        self.inner.borrow().time_scale_options_json()
    }

    /// Manual resize (still available for embedders not using `enable_auto_resize`, and for tests).
    pub fn resize(&mut self, css_width: f64, css_height: f64, dpr: f64) {
        self.inner.borrow_mut().resize(css_width, css_height, dpr);
    }

    pub fn zoom(&mut self, x_css: f64, scale: f64) {
        self.inner.borrow_mut().zoom(x_css, scale);
    }
    pub fn zoom_focused(&mut self, x_css: f64, scale: f64) {
        self.inner.borrow_mut().zoom_focused(x_css, scale);
    }
    pub fn scroll_start(&mut self, x_css: f64) {
        self.inner.borrow_mut().scroll_start(x_css);
    }
    pub fn scroll_move(&mut self, x_css: f64) {
        self.inner.borrow_mut().scroll_move(x_css);
    }
    pub fn scroll_end(&mut self) {
        self.inner.borrow_mut().scroll_end();
    }

    // --- engine-owned interaction models (the TS recognizer forwards samples; all formulas
    // live in the engine — see aeris_charts_engine::interaction) ---

    pub fn input_update_len() -> usize {
        INPUT_UPDATE_LEN
    }

    #[allow(clippy::too_many_arguments)]
    pub fn input_pointer_down(
        &mut self,
        id: u32,
        device: u8,
        target: u8,
        modifiers: u8,
        x: f64,
        y: f64,
        timestamp_ms: f64,
        pressure: f64,
        tilt_x: f64,
        tilt_y: f64,
        out: &mut [f64],
    ) -> bool {
        let sample = pointer_sample(
            id,
            device,
            target,
            modifiers,
            x,
            y,
            timestamp_ms,
            pressure,
            tilt_x,
            tilt_y,
        );
        write_input_update(out, self.inner.borrow_mut().input.pointer_down(sample))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn input_pointer_move(
        &mut self,
        id: u32,
        device: u8,
        target: u8,
        modifiers: u8,
        x: f64,
        y: f64,
        timestamp_ms: f64,
        pressure: f64,
        tilt_x: f64,
        tilt_y: f64,
        out: &mut [f64],
    ) -> bool {
        let sample = pointer_sample(
            id,
            device,
            target,
            modifiers,
            x,
            y,
            timestamp_ms,
            pressure,
            tilt_x,
            tilt_y,
        );
        write_input_update(out, self.inner.borrow_mut().input.pointer_move(sample))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn input_pointer_up(
        &mut self,
        id: u32,
        device: u8,
        target: u8,
        modifiers: u8,
        x: f64,
        y: f64,
        timestamp_ms: f64,
        pressure: f64,
        tilt_x: f64,
        tilt_y: f64,
        out: &mut [f64],
    ) -> bool {
        let sample = pointer_sample(
            id,
            device,
            target,
            modifiers,
            x,
            y,
            timestamp_ms,
            pressure,
            tilt_x,
            tilt_y,
        );
        write_input_update(out, self.inner.borrow_mut().input.pointer_up(sample))
    }

    pub fn input_long_press(&mut self, id: u32, out: &mut [f64]) -> bool {
        write_input_update(out, self.inner.borrow_mut().input.long_press(id))
    }

    pub fn input_cancel_all(&mut self, out: &mut [f64]) -> bool {
        write_input_update(out, self.inner.borrow_mut().input.cancel())
    }

    pub fn classify_wheel(
        &self,
        behavior: u8,
        delta_x: f64,
        delta_y: f64,
        delta_mode: u8,
        control: bool,
        shift: bool,
    ) -> u8 {
        let behavior = match behavior {
            1 => aeris_charts_engine::WheelBehavior::Pan,
            2 => aeris_charts_engine::WheelBehavior::Zoom,
            _ => aeris_charts_engine::WheelBehavior::Auto,
        };
        let delta_mode = match delta_mode {
            1 => aeris_charts_engine::WheelDeltaMode::Line,
            2 => aeris_charts_engine::WheelDeltaMode::Page,
            _ => aeris_charts_engine::WheelDeltaMode::Pixel,
        };
        aeris_charts_engine::WheelSample {
            delta_x,
            delta_y,
            delta_mode,
            modifiers: aeris_charts_engine::InputModifiers {
                control,
                shift,
                ..aeris_charts_engine::InputModifiers::default()
            },
            ..aeris_charts_engine::WheelSample::default()
        }
        .intent(behavior) as u8
    }

    /// reference wheel zoom increment: `sign(deltaY) * min(1, |deltaY|)`.
    pub fn wheel_zoom_scale(&self, delta_y: f64) -> f64 {
        self.inner.borrow().wheel_zoom_scale(delta_y)
    }
    /// reference pinch zoom increment: the scale-ratio delta ×5.
    pub fn pinch_zoom_scale(&self, scale_delta: f64) -> f64 {
        self.inner.borrow().pinch_zoom_scale(scale_delta)
    }
    /// reference wheel scroll delta: `deltaX * -80` px.
    pub fn wheel_scroll_delta(&self, delta_x: f64) -> f64 {
        self.inner.borrow().wheel_scroll_delta(delta_x)
    }

    /// Open a kinetic sampling session alongside the drag (`enabled = false` = no coast).
    pub fn kinetic_begin_sampling(&mut self, enabled: bool, position: f64, now_ms: f64) {
        self.inner
            .borrow_mut()
            .kinetic_begin_sampling(enabled, position, now_ms);
    }
    pub fn kinetic_add_sample(&mut self, position: f64, now_ms: f64) {
        self.inner.borrow_mut().kinetic_add_sample(position, now_ms);
    }
    /// The drag was released: whether a momentum coast engaged (drive `kinetic_position`
    /// per frame instead of ending the scroll session).
    pub fn kinetic_release(&mut self, position: f64, now_ms: f64) -> bool {
        self.inner.borrow_mut().kinetic_release(position, now_ms)
    }
    /// The coast's logical rightOffset at `now_ms` (NaN when no coast is running).
    pub fn kinetic_position(&self, now_ms: f64) -> f64 {
        self.inner.borrow().kinetic_position(now_ms)
    }
    pub fn kinetic_finished(&self, now_ms: f64) -> bool {
        self.inner.borrow().kinetic_finished(now_ms)
    }
    pub fn kinetic_stop(&mut self) {
        self.inner.borrow_mut().kinetic_stop();
    }

    /// Start or retune one held keyboard-pan direction in logical bars.
    pub fn start_keyboard_scroll(&mut self, delta_bars: f64, now_ms: f64) {
        self.inner
            .borrow_mut()
            .start_keyboard_scroll(delta_bars, now_ms);
    }
    /// Apply one keyboard-pan tick; NaN when no held kinetic session is active.
    pub fn keyboard_scroll_tick(&mut self, now_ms: f64) -> f64 {
        self.inner.borrow_mut().keyboard_scroll_tick(now_ms)
    }
    pub fn cancel_keyboard_scroll(&mut self) {
        self.inner.borrow_mut().cancel_keyboard_scroll();
    }

    /// Axis drag-to-scale (reference pressedMouseMove on the axis widgets).
    pub fn time_axis_start_scale(&mut self, x_css: f64) {
        self.inner.borrow_mut().time_axis_start_scale(x_css);
    }
    pub fn time_axis_scale_to(&mut self, x_css: f64) {
        self.inner.borrow_mut().time_axis_scale_to(x_css);
    }
    pub fn time_axis_end_scale(&mut self) {
        self.inner.borrow_mut().time_axis_end_scale();
    }
    /// Whether a price-axis drag can scale this scale (false in percentage/indexed modes).
    pub fn price_axis_scalable(&self, pane: usize, target: u32) -> bool {
        self.inner.borrow().price_axis_scalable(pane, target)
    }
    pub fn price_axis_start_scale(&mut self, pane: usize, target: u32, y_css: f64) {
        self.inner
            .borrow_mut()
            .price_axis_start_scale(pane, target, y_css);
    }
    pub fn price_axis_scale_to(&mut self, pane: usize, target: u32, y_css: f64) {
        self.inner
            .borrow_mut()
            .price_axis_scale_to(pane, target, y_css);
    }
    pub fn price_axis_end_scale(&mut self, pane: usize, target: u32) {
        self.inner.borrow_mut().price_axis_end_scale(pane, target);
    }
    /// industry-standard bid/ask quotes: push the current values for a series (NaN clears that
    /// side). Lines and chips render while the series' `bid_ask_visible` option holds. Call
    /// `render()` after.
    pub fn set_series_bid_ask(&mut self, id: usize, bid: f64, ask: f64) {
        self.inner.borrow_mut().engine.set_bid_ask(
            id as SeriesId,
            (bid.is_finite()).then_some(bid),
            (ask.is_finite()).then_some(ask),
        );
    }
    /// industry-standard wheel zoom on the price axis: `scale` is the normalized wheel
    /// increment (`wheel_zoom_scale`); anchored at the cursor's price. Call `render()` after.
    pub fn price_axis_wheel_zoom(&mut self, pane: usize, target: u32, y_css: f64, scale: f64) {
        self.inner.borrow_mut().engine.price_axis_wheel_zoom(
            pane,
            price_scale_target_from_u32(target),
            y_css,
            scale,
        );
    }
    /// Vertical price pan (reference `startScrollPrice`/`scrollPriceTo`).
    pub fn price_axis_start_scroll(&mut self, pane: usize, target: u32, y_css: f64) {
        self.inner
            .borrow_mut()
            .price_axis_start_scroll(pane, target, y_css);
    }
    pub fn price_axis_scroll_to(&mut self, pane: usize, target: u32, y_css: f64) {
        self.inner
            .borrow_mut()
            .price_axis_scroll_to(pane, target, y_css);
    }
    pub fn price_axis_end_scroll(&mut self, pane: usize, target: u32) {
        self.inner.borrow_mut().price_axis_end_scroll(pane, target);
    }
    /// Resolve the intended series scale and begin its drag session when already manual.
    pub fn begin_price_pan_at(&mut self, pane: usize, x_css: f64, y_css: f64) -> Option<u32> {
        self.inner
            .borrow_mut()
            .begin_price_pan_at(pane, x_css, y_css)
    }
    /// Resolve the intended pane price scale without opening or mutating its drag session.
    pub fn price_pan_target_at(&self, pane: usize, x_css: f64, y_css: f64) -> Option<u32> {
        self.inner.borrow().price_pan_target_at(pane, x_css, y_css)
    }

    /// Eased scroll-to-position (the engine owns the cubic ease-out and applies each tick).
    pub fn start_scroll_animation(&mut self, target: f64, duration_ms: f64, now_ms: f64) {
        self.inner
            .borrow_mut()
            .start_scroll_animation(target, duration_ms, now_ms);
    }
    /// Apply the eased position for `now_ms`; NaN when finished/none (stop scheduling).
    pub fn scroll_animation_tick(&mut self, now_ms: f64) -> f64 {
        self.inner.borrow_mut().scroll_animation_tick(now_ms)
    }
    pub fn cancel_scroll_animation(&mut self) {
        self.inner.borrow_mut().cancel_scroll_animation();
    }
    /// Index of the stacked pane containing content-y `y` (engine-owned pane bounds).
    pub fn pane_index_at_y(&self, y_css: f64) -> usize {
        self.inner.borrow().pane_index_at_y(y_css)
    }
    /// Engine-owned secondary-click context as
    /// `[x, y, pane, time|NaN, logical|NaN, price, series_id|NaN]`.
    pub fn chart_context_at(&self, x_css: f64, y_css: f64) -> Vec<f64> {
        self.inner.borrow().chart_context_at(x_css, y_css)
    }
    pub fn price_axis_target_at(&self, pane: usize, x_css: f64) -> Option<u32> {
        self.inner.borrow().price_axis_target_at(pane, x_css)
    }
    pub fn fit_content(&mut self) {
        self.inner.borrow_mut().fit_content();
    }
    pub fn set_bar_spacing(&mut self, spacing: f64) {
        self.inner.borrow_mut().set_bar_spacing(spacing);
    }
    pub fn set_right_offset(&mut self, offset: f64) {
        self.inner.borrow_mut().set_right_offset(offset);
    }
    pub fn set_crosshair(&mut self, x_css: f64, y_css: f64) {
        self.inner.borrow_mut().set_crosshair(x_css, y_css);
    }
    /// the public reference's Ctrl-held magnet: the gesture layer forwards the live modifier state; a
    /// Normal-mode crosshair then snaps to the hovered bar's rendered prices on the next
    /// `render()` (OHLC for candles/bars, close/value for scalar series).
    pub fn set_crosshair_ohlc_magnet(&mut self, enabled: bool) {
        self.inner.borrow_mut().engine.crosshair_ohlc_magnet = enabled;
    }

    /// Whether the OHLC crosshair magnet is currently engaged.
    pub fn crosshair_ohlc_magnet(&self) -> bool {
        self.inner.borrow().engine.crosshair_ohlc_magnet
    }
    pub fn clear_crosshair(&mut self) {
        self.inner.borrow_mut().clear_crosshair();
    }
    /// Hover hit testing (plugin platform Phase C-d): the primitive object and/or series
    /// under pane-relative CSS px `(x_css, y_css)` as a JSON
    /// `{"series_id":number|null,"object_id":string|null,"cursor":string|null}` (see the
    /// inner method for the reference arbitration). Also refreshes the engine's hovered series
    /// for the `hoveredSeriesOnTop` z-bump, so call `render()` afterwards.
    pub fn hover_at(&mut self, x_css: f64, y_css: f64) -> String {
        self.inner.borrow_mut().hover_at(x_css, y_css)
    }
    /// Release hover promotion (cursor left the chart): the `hoveredSeriesOnTop` promotion
    /// lets go on the next `render()` and active drawings return to stable order. The text
    /// drawings' hover ring releases too.
    pub fn clear_hover(&mut self) {
        let mut inner = self.inner.borrow_mut();
        inner.engine.set_hovered_series(None);
        inner.engine.set_hovered_text(None);
        inner.engine.set_hovered_drawing(None);
        inner.engine.clear_general_hover();
    }
    /// industry-standard click-to-select: the host's click pipeline sets the series under the
    /// click (`None` on empty pane space); the engine snapshots sparse canonical anchor identities
    /// and reprojects them until deselection. Call `render()` afterwards.
    pub fn set_selected_series(&mut self, id: Option<u32>) {
        self.inner
            .borrow_mut()
            .engine
            .set_selected_series(id.map(|id| id as SeriesId));
    }

    /// Deterministic browser-test hook for the transient canonical selection-anchor timestamps.
    #[doc(hidden)]
    pub fn selection_anchor_identities_json(&self) -> String {
        serde_json::to_string(self.inner.borrow().engine.selection_anchor_identities())
            .unwrap_or_else(|_| "[]".to_string())
    }

    // --- drawing tools (engine-owned drawing objects; aeris_charts_engine drawings.rs) ---
    // Kinds are the engine catalog's wire ids (`DrawingKind::to_u8`, TS `DRAWING_KIND_TO_U8`):
    // core tools 0..=31, B8 drawing families from 32 in reserved per-family ranges. All
    // coordinates are pane-relative CSS px (x from the pane's left, y from the chart's top — the
    // crosshair's space). Call `render()` after mutations.

    /// Add a drawing to `pane` from a JSON `[{logical, price}, ...]` anchor array and an
    /// optional options patch ("" = defaults). Returns the drawing id (> 0), or 0 when the
    /// engine rejects it (unknown kind, stale pane, wrong anchor count, non-finite anchors).
    pub fn add_drawing(
        &mut self,
        kind: u8,
        pane: usize,
        points_json: &str,
        options_json: &str,
    ) -> u32 {
        self.inner
            .borrow_mut()
            .add_drawing(kind, pane, points_json, options_json)
    }
    /// Merge an options patch into the drawing (reference `applyOptions` merge semantics).
    pub fn drawing_apply_options(&mut self, id: u32, options_json: &str) -> bool {
        self.inner
            .borrow_mut()
            .drawing_apply_options(id, options_json)
    }
    /// Replace the drawing's anchors from a JSON `[{logical?, time?, price}, ...]` array.
    pub fn drawing_set_points(&mut self, id: u32, points_json: &str) -> bool {
        self.inner.borrow_mut().drawing_set_points(id, points_json)
    }
    /// `add_drawing` with a `{ok, id}` / `{ok:false, error:{code, message}}` result envelope.
    pub fn add_drawing_result_json(
        &mut self,
        kind: u8,
        pane: usize,
        points_json: &str,
        options_json: &str,
    ) -> String {
        self.inner
            .borrow_mut()
            .add_drawing_result_json(kind, pane, points_json, options_json)
    }
    /// `drawing_set_points` with a `{ok}` / `{ok:false, error}` result envelope.
    pub fn drawing_set_points_result_json(&mut self, id: u32, points_json: &str) -> String {
        self.inner
            .borrow_mut()
            .drawing_set_points_result_json(id, points_json)
    }
    /// Atomic multi-drawing anchor rewrite (`[{id, points}]`, one undo step).
    pub fn set_drawings_points_result_json(&mut self, updates_json: &str) -> String {
        self.inner
            .borrow_mut()
            .set_drawings_points_result_json(updates_json)
    }
    /// Price-basis rescale of drawing prices by time segments (no undo step).
    pub fn rescale_drawing_prices_result_json(
        &mut self,
        segments_json: &str,
        price_basis: Option<String>,
    ) -> String {
        self.inner
            .borrow_mut()
            .rescale_drawing_prices_result_json(segments_json, price_basis)
    }
    pub fn set_drawing_price_basis(&mut self, basis: Option<String>) -> bool {
        self.inner.borrow_mut().set_drawing_price_basis(basis)
    }
    pub fn drawing_price_basis(&self) -> Option<String> {
        self.inner.borrow().drawing_price_basis()
    }
    /// Chart drawing magnet: 0 off, 1 weak, 2 strong.
    pub fn set_drawing_magnet_mode(&mut self, mode: u8) -> bool {
        self.inner.borrow_mut().set_drawing_magnet_mode(mode)
    }
    pub fn drawing_magnet_mode(&self) -> u8 {
        self.inner.borrow().drawing_magnet_mode()
    }
    /// Effective magnet (0/1/2) of the armed tool's next placement given the Ctrl/Cmd toggle.
    pub fn armed_drawing_magnet(&self, toggle: bool) -> u8 {
        self.inner.borrow().armed_drawing_magnet(toggle)
    }
    /// Keyboard-reachable handle count of a drawing (-1 when unknown or unplaceable).
    pub fn drawing_handle_count(&self, id: u32) -> i32 {
        self.inner.borrow().drawing_handle_count(id)
    }
    /// The drawing's options as snake_case JSON ("" for an unknown id).
    pub fn drawing_options_json(&self, id: u32) -> String {
        self.inner.borrow().drawing_options_json(id)
    }
    pub fn drawing_property_schema_json(&self, id: u32) -> String {
        self.inner.borrow().drawing_property_schema_json(id)
    }

    pub fn drawing_kind_options_json(&self, id: u32) -> String {
        self.inner.borrow().drawing_kind_options_json(id)
    }
    pub fn drawing_object_tree_json(&self) -> String {
        self.inner.borrow().drawing_object_tree_json()
    }
    /// The drawing's anchors as a JSON `[{logical, price}, ...]` array ("" for an unknown id).
    pub fn drawing_points_json(&self, id: u32) -> String {
        self.inner.borrow().drawing_points_json(id)
    }
    /// One anchor's CSS-px position `[x, y]` in overlay space (empty when it cannot convert).
    pub fn drawing_point_to_coordinate(&self, id: u32, index: usize) -> Vec<f64> {
        self.inner.borrow().drawing_point_to_coordinate(id, index)
    }
    /// Exact text-run anchor `[x, y]` in overlay CSS px for inline drawing editing.
    pub fn drawing_text_coordinate(&self, id: u32) -> Vec<f64> {
        self.inner.borrow().drawing_text_coordinate(id)
    }
    /// Exact text-run transform `[x, y, clockwise_radians]` in overlay CSS px.
    pub fn drawing_text_transform(&self, id: u32) -> Vec<f64> {
        self.inner.borrow().drawing_text_transform(id)
    }
    /// Trend-line label/placeholder hit identity, or zero when the point misses.
    pub fn drawing_text_hit_at(&self, x_css: f64, y_css: f64) -> u32 {
        self.inner.borrow().drawing_text_hit_at(x_css, y_css)
    }
    /// Every drawing as a JSON array in z-order (`{id, kind, pane_index, points, ...options}`).
    pub fn drawings_json(&self) -> String {
        self.inner.borrow().drawings_json()
    }
    pub fn set_drawing_interval(&mut self, interval_json: &str) -> bool {
        self.inner.borrow_mut().set_drawing_interval(interval_json)
    }
    pub fn selected_drawings_json(&self) -> String {
        self.inner.borrow().selected_drawings_json()
    }
    pub fn set_selected_drawings(&mut self, ids_json: &str) -> bool {
        self.inner.borrow_mut().set_selected_drawings(ids_json)
    }
    pub fn copy_drawings_json(&self, ids_json: &str) -> String {
        self.inner.borrow().copy_drawings_json(ids_json)
    }
    pub fn paste_drawings_json(
        &mut self,
        payload: &str,
        pane: usize,
        logical_offset: f64,
        price_offset: f64,
    ) -> String {
        self.inner
            .borrow_mut()
            .paste_drawings_json(payload, pane, logical_offset, price_offset)
    }
    pub fn clone_drawing(&mut self, id: u32, logical_offset: f64, price_offset: f64) -> u32 {
        self.inner
            .borrow_mut()
            .clone_drawing(id, logical_offset, price_offset)
    }
    pub fn move_drawing_z_order(&mut self, id: u32, delta: i32) -> bool {
        self.inner.borrow_mut().move_drawing_z_order(id, delta)
    }
    pub fn set_drawing_visibility(&mut self, id: u32, visible: bool) -> bool {
        self.inner.borrow_mut().set_drawing_visibility(id, visible)
    }
    pub fn set_drawing_locked(&mut self, id: u32, locked: bool) -> bool {
        self.inner.borrow_mut().set_drawing_locked(id, locked)
    }
    pub fn set_drawing_group(&mut self, id: u32, group_json: &str) -> bool {
        self.inner.borrow_mut().set_drawing_group(id, group_json)
    }
    pub fn set_drawing_group_visibility(&mut self, group_id: &str, visible: bool) -> u32 {
        self.inner
            .borrow_mut()
            .set_drawing_group_visibility(group_id, visible)
    }
    pub fn set_drawing_group_locked(&mut self, group_id: &str, locked: bool) -> u32 {
        self.inner
            .borrow_mut()
            .set_drawing_group_locked(group_id, locked)
    }
    pub fn move_drawing_group(
        &mut self,
        group_id: &str,
        logical_delta: f64,
        price_delta: f64,
    ) -> u32 {
        self.inner
            .borrow_mut()
            .move_drawing_group(group_id, logical_delta, price_delta)
    }
    pub fn drawing_sync_payload_json(&self, source: &str) -> String {
        self.inner.borrow().drawing_sync_payload_json(source)
    }
    pub fn apply_drawing_sync_payload_json(&mut self, payload: &str) -> bool {
        self.inner
            .borrow_mut()
            .apply_drawing_sync_payload_json(payload)
    }
    pub fn apply_drawing_template_json(&mut self, id: u32, template_json: &str) -> bool {
        self.inner
            .borrow_mut()
            .apply_drawing_template_json(id, template_json)
    }
    pub fn drawing_template_json(&self, id: u32, name: &str) -> String {
        self.inner.borrow().drawing_template_json(id, name)
    }
    /// Internal benchmark counters; not part of the package chart API.
    pub fn drawing_work_stats_json(&self) -> String {
        serde_json::to_string(&self.inner.borrow().engine.drawing_work_stats()).unwrap_or_default()
    }
    /// Reset internal drawing benchmark counters.
    pub fn reset_drawing_work_stats(&self) {
        self.inner.borrow().engine.reset_drawing_work_stats();
    }
    pub fn remove_drawing(&mut self, id: u32) -> bool {
        self.inner.borrow_mut().remove_drawing(id)
    }
    /// Remove every drawing (the demo's "clear all").
    pub fn clear_drawings(&mut self) {
        self.inner.borrow_mut().clear_drawings();
    }
    /// Click-to-select arbitration: selects the drawing under the point (clears on a miss) and
    /// reports whether one was hit, so the host skips its series-selection path.
    pub fn select_drawing_at(&mut self, x_css: f64, y_css: f64) -> bool {
        self.inner.borrow_mut().select_drawing_at(x_css, y_css)
    }
    pub fn set_selected_drawing(&mut self, id: Option<u32>) {
        self.inner.borrow_mut().set_selected_drawing(id);
    }
    /// Whether the host's inline editor can edit the drawing's text in place (the text tool, a
    /// trend label, or a family text box), while it is unlocked, visible, and shown.
    pub fn drawing_text_editable(&self, id: u32) -> bool {
        self.inner.borrow().engine.drawing_text_editable(id)
    }
    /// Open the engine's text-edit session for the host editor. Frame construction keeps the
    /// committed glyphs under the overlay caret while it is open. False when not editable.
    pub fn begin_drawing_text_edit(&mut self, id: u32) -> bool {
        self.inner.borrow_mut().engine.begin_drawing_text_edit(id)
    }
    /// Live text of the open session: repaints without an undo step or a sync revision.
    pub fn set_drawing_edit_text(&mut self, text: &str) -> bool {
        self.inner.borrow_mut().engine.set_drawing_edit_text(text)
    }
    /// Close the open session: commit records one undo step, cancel restores the text.
    pub fn end_drawing_text_edit(&mut self, commit: bool) -> bool {
        self.inner.borrow_mut().engine.end_drawing_text_edit(commit)
    }
    pub fn editing_drawing(&self) -> Option<u32> {
        self.inner.borrow().engine.editing_drawing()
    }
    /// A family text box's editor layout as JSON (`{x, y, line_height, size, font_family,
    /// weight, italic, color, rect}` in overlay CSS px), or an empty string.
    pub fn drawing_text_edit_layout_json(&self, id: u32) -> String {
        self.inner.borrow().drawing_text_edit_layout_json(id)
    }
    pub fn selected_drawing(&self) -> Option<u32> {
        self.inner.borrow().selected_drawing()
    }
    /// Delete/Backspace: remove the selected drawing. False while nothing is selected.
    pub fn remove_selected_drawing(&mut self) -> bool {
        self.inner.borrow_mut().remove_selected_drawing()
    }
    /// Press routing for the gesture layer: opens an anchor/body drag on the drawing under the
    /// point (false = the host falls through to pan/scroll). A successful grab selects the
    /// drawing (reference-informed behavior).
    pub fn drawing_drag_start_at(&mut self, x_css: f64, y_css: f64) -> bool {
        self.inner.borrow_mut().drawing_drag_start_at(x_css, y_css)
    }
    pub fn drawing_drag_start_at_device(&mut self, x_css: f64, y_css: f64, device: u8) -> bool {
        let profile = aeris_charts_engine::HitProfile::for_device(input_device_from_u8(device));
        self.inner
            .borrow_mut()
            .engine
            .drawing_drag_start_at_with_profile(x_css, y_css, profile)
    }
    pub fn drawing_drag_to(&mut self, x_css: f64, y_css: f64, magnet: bool, straighten: bool) {
        self.inner
            .borrow_mut()
            .drawing_drag_to(x_css, y_css, magnet, straighten);
    }
    pub fn drawing_drag_end(&mut self) {
        self.inner.borrow_mut().drawing_drag_end();
    }
    pub fn drawing_drag_cancel(&mut self) {
        self.inner.borrow_mut().engine.drawing_drag_cancel();
    }
    pub fn drawing_drag_active(&self) -> bool {
        self.inner.borrow().drawing_drag_active()
    }
    pub fn nudge_selected_drawing(&mut self, dx_css: f64, dy_css: f64, anchor: i32) -> bool {
        self.inner.borrow_mut().engine.nudge_selected_drawing(
            dx_css,
            dy_css,
            usize::try_from(anchor).ok(),
        )
    }
    /// Undo one committed drawing mutation in this chart's bounded semantic history.
    pub fn undo_drawing(&mut self) -> bool {
        self.inner.borrow_mut().undo_drawing()
    }
    /// Redo one previously undone drawing mutation in this chart.
    pub fn redo_drawing(&mut self) -> bool {
        self.inner.borrow_mut().redo_drawing()
    }
    pub fn can_undo_drawing(&self) -> bool {
        self.inner.borrow().can_undo_drawing()
    }
    pub fn can_redo_drawing(&self) -> bool {
        self.inner.borrow().can_redo_drawing()
    }

    /// Arm/disarm the engine-owned drawing-tool controller. `kind = -1` disarms;
    /// `pane = -1` lets the first placement bind the pane.
    pub fn set_drawing_tool(&mut self, kind: i32, options_json: &str, pane: i32) -> bool {
        self.inner
            .borrow_mut()
            .set_drawing_tool(kind, options_json, pane)
    }
    pub fn active_drawing_tool(&self) -> i32 {
        self.inner.borrow().active_drawing_tool()
    }
    pub fn active_drawing_tool_pane(&self) -> i32 {
        self.inner.borrow().active_drawing_tool_pane()
    }
    pub fn drawing_tool_apply_options(&mut self, options_json: &str) -> bool {
        self.inner
            .borrow_mut()
            .drawing_tool_apply_options(options_json)
    }
    pub fn drawing_tool_pointer_down(
        &mut self,
        x_css: f64,
        y_css: f64,
        magnet: bool,
        straighten: bool,
    ) -> u32 {
        self.inner
            .borrow_mut()
            .drawing_tool_pointer_down(x_css, y_css, magnet, straighten)
    }
    pub fn drawing_tool_pointer_move(
        &mut self,
        x_css: f64,
        y_css: f64,
        magnet: bool,
        straighten: bool,
        pressed: bool,
    ) -> bool {
        self.inner
            .borrow_mut()
            .drawing_tool_pointer_move(x_css, y_css, magnet, straighten, pressed)
    }
    pub fn drawing_tool_pointer_up(
        &mut self,
        x_css: f64,
        y_css: f64,
        magnet: bool,
        straighten: bool,
    ) -> u32 {
        self.inner
            .borrow_mut()
            .drawing_tool_pointer_up(x_css, y_css, magnet, straighten)
    }
    pub fn drawing_tool_activate(
        &mut self,
        x_css: f64,
        y_css: f64,
        magnet: bool,
        straighten: bool,
    ) -> u32 {
        self.inner
            .borrow_mut()
            .drawing_tool_activate(x_css, y_css, magnet, straighten)
    }
    pub fn drawing_tool_finish(&mut self) -> u32 {
        self.inner.borrow_mut().drawing_tool_finish()
    }
    pub fn drawing_tool_pop_anchor(&mut self) -> bool {
        self.inner.borrow_mut().drawing_tool_pop_anchor()
    }
    pub fn drawing_tool_capture_active(&self) -> bool {
        self.inner.borrow().drawing_tool_capture_active()
    }
    pub fn drawing_tool_sequence_active(&self) -> bool {
        self.inner.borrow().drawing_tool_sequence_active()
    }
    pub fn drawing_requests_text_edit(&self, id: u32) -> bool {
        self.inner.borrow().drawing_requests_text_edit(id)
    }
    pub fn cancel_drawing_creation(&mut self) {
        self.inner.borrow_mut().cancel_drawing_creation();
    }
    pub fn cancel_drawing_tool(&mut self) {
        self.inner.borrow_mut().cancel_drawing_tool();
    }

    /// Arm interactive creation of a tool kind ("" options = defaults): the next clicks place
    /// anchors through `drawing_create_click`, moves preview through `drawing_create_move`.
    pub fn drawing_create_begin(&mut self, kind: u8, options_json: &str) -> bool {
        self.inner
            .borrow_mut()
            .drawing_create_begin(kind, options_json)
    }
    pub fn drawing_create_apply_options(&mut self, options_json: &str) -> bool {
        self.inner
            .borrow_mut()
            .drawing_create_apply_options(options_json)
    }
    /// Place the next creation anchor: 0 unarmed, -1 pending more anchors, > 0 the committed
    /// drawing's id (left selected, industry-standard). `magnet` snaps the anchor to the nearest
    /// rendered bar price (OHLC for candles/bars, close/value for scalar series); `straighten`
    /// constrains a second anchor to 0°/45°/90° (a rectangle to a square).
    pub fn drawing_create_click(
        &mut self,
        x_css: f64,
        y_css: f64,
        magnet: bool,
        straighten: bool,
    ) -> i64 {
        self.inner
            .borrow_mut()
            .drawing_create_click(x_css, y_css, magnet, straighten)
    }
    pub fn drawing_create_move(&mut self, x_css: f64, y_css: f64, magnet: bool, straighten: bool) {
        self.inner
            .borrow_mut()
            .drawing_create_move(x_css, y_css, magnet, straighten);
    }
    /// Finish an active multi-click path. Returns the committed drawing id, or 0 when the
    /// pending creation is not a valid path.
    pub fn drawing_create_finish(&mut self) -> u32 {
        self.inner.borrow_mut().drawing_create_finish()
    }
    /// Remove the latest placed vertex from an active multi-click path.
    pub fn drawing_create_pop_anchor(&mut self) -> bool {
        self.inner.borrow_mut().drawing_create_pop_anchor()
    }
    pub fn drawing_create_cancel(&mut self) {
        self.inner.borrow_mut().drawing_create_cancel();
    }
    pub fn drawing_create_active(&self) -> bool {
        self.inner.borrow().drawing_create_active()
    }

    // --- freehand brush (press-drag-release capture; engine owns input decimation) ---

    /// Begin a brush stroke (pointer-down with the brush tool armed; "" options = defaults).
    /// False off the panes/data.
    pub fn brush_create_start(&mut self, options_json: &str, x_css: f64, y_css: f64) -> bool {
        self.inner
            .borrow_mut()
            .brush_create_start(options_json, x_css, y_css)
    }
    /// Capture the next stroke point from a pointer move (engine-decimated by distance).
    pub fn brush_create_add(&mut self, x_css: f64, y_css: f64) {
        self.inner.borrow_mut().brush_create_add(x_css, y_css);
    }
    /// Commit the stroke (pointer-up): the captured path is stored as-is (input decimation
    /// already bounded it) and rendered as a smooth curved polyline, left selected. 0 =
    /// degenerate stroke discarded (a click without a drag).
    pub fn brush_create_end(&mut self) -> u32 {
        self.inner.borrow_mut().brush_create_end()
    }
    pub fn brush_create_cancel(&mut self) {
        self.inner.borrow_mut().brush_create_cancel();
    }
    pub fn brush_create_active(&self) -> bool {
        self.inner.borrow().brush_create_active()
    }
    /// industry-standard reset view: default bar spacing/right offset plus autoscale restored
    /// on every pane's price scales; the next `render()` recalculates the visible ranges.
    pub fn reset_view(&mut self) {
        self.inner.borrow_mut().engine.reset_view();
    }
    /// Restore autoscale on every pane-local built-in, named, hidden, and overlay price scale.
    pub fn reset_price_scales(&mut self) {
        self.inner.borrow_mut().engine.reset_price_scales();
    }
    /// Restore autoscale only on the exact pane-local price scale under an axis double-click.
    pub fn reset_price_scale(&mut self, pane: usize, target: u32) {
        self.inner
            .borrow_mut()
            .engine
            .reset_price_scale(pane, price_scale_target_from_u32(target));
    }
    /// reference `chart.setCrosshairPosition(price, time, series)`: position the crosshair at a
    /// data point with no DOM event — `time` must resolve exactly to a bar (false
    /// otherwise); x is that bar's coordinate and y the price mapped through the given
    /// series' price scale. A following `render()` shows it.
    pub fn set_crosshair_position(&mut self, price: f64, time: f64, series_id: u32) -> bool {
        self.inner
            .borrow_mut()
            .set_crosshair_position(price, time, series_id)
    }
    /// reference `chart.clearCrosshairPosition`: clear the crosshair along with any saved offset,
    /// so later scale changes cannot resurrect it.
    pub fn clear_crosshair_position(&mut self) {
        self.inner.borrow_mut().clear_crosshair_position();
    }

    pub fn crosshair_sync_json(&self) -> String {
        serde_json::to_string(&self.inner.borrow().crosshair_sync_position())
            .unwrap_or_else(|_| "null".to_string())
    }

    pub fn apply_external_crosshair_json(&mut self, position_json: &str) -> bool {
        let position = serde_json::from_str::<Option<aeris_charts_engine::CrosshairSyncPosition>>(
            position_json,
        )
        .ok();
        let Some(position) = position else {
            return false;
        };
        self.inner.borrow_mut().apply_external_crosshair(position)
    }

    pub fn take_sync_events_json(&mut self) -> String {
        serde_json::to_string(&self.inner.borrow_mut().take_sync_events())
            .unwrap_or_else(|_| "[]".to_string())
    }
    pub fn bar_spacing(&self) -> f64 {
        self.inner.borrow().bar_spacing()
    }
    pub fn right_offset(&self) -> f64 {
        self.inner.borrow().right_offset()
    }
    pub fn scroll_position(&self) -> f64 {
        self.inner.borrow().scroll_position()
    }
    pub fn scroll_to_position(&mut self, position: f64) {
        self.inner.borrow_mut().scroll_to_position(position);
    }
    /// Jump to the real-time edge (the configured `right_offset`, reference `scrollToRealTime`).
    pub fn scroll_to_real_time(&mut self) {
        self.inner.borrow_mut().scroll_to_real_time();
    }
    /// Animate to the real-time edge; drive with `scroll_animation_tick` until it returns NaN.
    pub fn start_real_time_scroll_animation(&mut self, duration_ms: f64, now_ms: f64) {
        self.inner
            .borrow_mut()
            .start_real_time_scroll_animation(duration_ms, now_ms);
    }
    pub fn reset_time_scale(&mut self) {
        self.inner.borrow_mut().reset_time_scale();
    }
    pub fn time_scale_width(&self) -> f64 {
        self.inner.borrow().time_scale_width()
    }
    pub fn time_scale_height(&self) -> f64 {
        self.inner.borrow().time_scale_height()
    }
    pub fn price_scale_width(&self, pane: usize, target: u32) -> f64 {
        self.inner.borrow().price_scale_width(pane, target)
    }
    pub fn price_scale_visible_range(&self, pane: usize, target: u32) -> Vec<f64> {
        self.inner.borrow().price_scale_visible_range(pane, target)
    }
    pub fn set_price_scale_visible_range(&mut self, pane: usize, target: u32, from: f64, to: f64) {
        self.inner
            .borrow_mut()
            .set_price_scale_visible_range(pane, target, from, to);
    }
    pub fn price_scale_auto_scale(&self, pane: usize, target: u32) -> Option<bool> {
        self.inner.borrow().price_scale_auto_scale(pane, target)
    }
    pub fn set_price_scale_auto_scale(&mut self, pane: usize, target: u32, enabled: bool) {
        self.inner
            .borrow_mut()
            .set_price_scale_auto_scale(pane, target, enabled);
    }
    pub fn price_scale_inverted(&self, pane: usize, target: u32) -> Option<bool> {
        self.inner.borrow().price_scale_inverted(pane, target)
    }
    pub fn set_price_scale_inverted(&mut self, pane: usize, target: u32, inverted: bool) {
        self.inner
            .borrow_mut()
            .set_price_scale_inverted(pane, target, inverted);
    }
    pub fn price_scale_margins(&self, pane: usize, target: u32) -> Vec<f64> {
        self.inner.borrow().price_scale_margins(pane, target)
    }
    pub fn set_price_scale_margins(&mut self, pane: usize, target: u32, top: f64, bottom: f64) {
        self.inner
            .borrow_mut()
            .set_price_scale_margins(pane, target, top, bottom);
    }
    pub fn price_scale_mode(&self, pane: usize, target: u32) -> Option<u8> {
        self.inner.borrow().price_scale_mode(pane, target)
    }
    pub fn set_price_scale_mode(&mut self, pane: usize, target: u32, mode: u8) {
        self.inner
            .borrow_mut()
            .set_price_scale_mode(pane, target, mode);
    }
    pub fn series_pane_index(&self, id: u32) -> Option<usize> {
        self.inner.borrow().series_pane_index(id)
    }
    pub fn series_is_overlay(&self, id: u32) -> Option<bool> {
        self.inner.borrow().series_is_overlay(id)
    }
    pub fn series_price_scale_id(&self, id: u32) -> Option<u32> {
        self.inner.borrow().series_price_scale_id(id)
    }
    pub fn series_price_scale_name(&self, id: u32) -> String {
        self.inner.borrow().series_price_scale_name(id)
    }
    pub fn set_series_price_scale_by_name(&mut self, id: u32, name: &str) -> bool {
        self.inner
            .borrow_mut()
            .set_series_price_scale_by_name(id, name)
    }
    pub fn set_series_price_scale(&mut self, id: u32, target: u32) {
        self.inner.borrow_mut().set_series_price_scale(id, target);
    }
    pub fn series_price_to_coordinate(&self, id: u32, price: f64) -> Option<f64> {
        self.inner.borrow().series_price_to_coordinate(id, price)
    }
    pub fn series_coordinate_to_price(&self, id: u32, coordinate: f64) -> Option<f64> {
        self.inner
            .borrow()
            .series_coordinate_to_price(id, coordinate)
    }
    pub fn series_kind(&self, id: u32) -> Option<u8> {
        self.inner.borrow().series_kind(id)
    }
    pub fn series_data_by_index(&self, id: u32, index: f64, mismatch: i8) -> Vec<f64> {
        self.inner
            .borrow()
            .series_data_by_index(id, index, mismatch)
    }
    pub fn series_data(&self, id: u32) -> Vec<f64> {
        self.inner.borrow().series_data(id)
    }
    /// Every live series' latest value, or exact value at a merged logical index, as one JSON
    /// transfer. `NaN` selects independently-resolved latest mode.
    pub fn value_snapshot_json(&self, logical_index: f64) -> String {
        self.inner.borrow().value_snapshot_json(logical_index)
    }
    /// Set the shared comparison anchor in UTC seconds. Pass `NaN` to clear it.
    pub fn set_comparison_anchor(&mut self, time: f64) -> bool {
        self.inner.borrow_mut().set_comparison_anchor(time)
    }
    pub fn comparison_anchor(&self) -> f64 {
        self.inner.borrow().comparison_anchor()
    }
    pub fn comparison_legend_json(&self) -> String {
        self.inner.borrow().comparison_legend_json()
    }
    pub fn series_bars_in_logical_range(&self, id: u32, from: f64, to: f64) -> Vec<f64> {
        self.inner
            .borrow()
            .series_bars_in_logical_range(id, from, to)
    }
    pub fn price_axis_width(&self) -> f64 {
        self.inner.borrow().price_axis_width()
    }
    pub fn pane_left(&self) -> f64 {
        self.inner.borrow().pane_left()
    }

    // --- coordinate & logical-range API (roadmap Phase A4) ---

    /// Y (CSS px, chart content) for a price on pane 0's default price scale, or `undefined` if
    /// that scale has no range yet.
    pub fn price_to_coordinate(&self, price: f64) -> Option<f64> {
        self.inner.borrow().price_to_coordinate(price)
    }
    /// Price for a Y (CSS px, chart content) on the default price scale of the pane containing
    /// it, or `undefined` if that scale has no range yet.
    pub fn coordinate_to_price(&self, y_css: f64) -> Option<f64> {
        self.inner.borrow().coordinate_to_price(y_css)
    }
    /// X (CSS px) for a UTC-seconds timestamp on a data point, else `undefined`.
    pub fn time_to_coordinate(&self, time: f64) -> Option<f64> {
        self.inner.borrow().time_to_coordinate(time)
    }
    /// UTC seconds of the data point nearest X (CSS px), or `undefined` off-chart.
    pub fn coordinate_to_time(&self, x_css: f64) -> Option<f64> {
        self.inner.borrow().coordinate_to_time(x_css)
    }
    /// Integer logical (bar) index owning X (CSS px), or `undefined` if there is no data.
    pub fn coordinate_to_logical(&self, x_css: f64) -> Option<f64> {
        self.inner.borrow().coordinate_to_logical(x_css)
    }
    /// X (CSS px) for an integer logical index.
    pub fn logical_to_coordinate(&self, logical: f64) -> Option<f64> {
        self.inner.borrow().logical_to_coordinate(logical)
    }
    /// Logical index for a UTC-seconds timestamp. `find_nearest` follows reference lower-bound rules.
    pub fn time_to_index(&self, time: f64, find_nearest: bool) -> Option<i64> {
        self.inner.borrow().time_to_index(time, find_nearest)
    }
    /// Visible window in logical (bar) units as a `[from, to]` Float64Array (empty if no data).
    pub fn visible_logical_range(&self) -> Vec<f64> {
        self.inner.borrow().visible_logical_range()
    }
    /// Set the visible window in logical (bar) units; call `render()` after.
    pub fn set_visible_logical_range(&mut self, from: f64, to: f64) {
        self.inner.borrow_mut().set_visible_logical_range(from, to);
    }
    /// Visible window as a `[from_time, to_time]` Float64Array of UTC seconds (empty if no data).
    pub fn visible_time_range(&self) -> Vec<f64> {
        self.inner.borrow().visible_time_range()
    }
    /// Set the visible window to bracket `[from_time, to_time]` UTC seconds; call `render()` after.
    pub fn set_visible_time_range(&mut self, from_time: f64, to_time: f64) {
        self.inner
            .borrow_mut()
            .set_visible_time_range(from_time, to_time);
    }

    pub fn render(&mut self) -> Result<(), JsValue> {
        self.inner.borrow_mut().render()
    }

    /// Paint the retained backend-neutral frame into the warm Canvas2D pane without changing the
    /// active onscreen backend. The TypeScript package uses this to implement its synchronous,
    /// deterministic composed screenshot API even while WebGPU is active.
    #[doc(hidden)]
    pub fn render_canvas2d_snapshot(&self, include_axis: bool) -> Result<(), JsValue> {
        self.inner.borrow().render_canvas2d_with_axis(include_axis)
    }

    /// Reports the active pane backend for diagnostics and runtime-matrix tests.
    pub fn backend_kind(&self) -> String {
        self.inner.borrow().backend_kind()
    }

    pub fn backend_status_json(&self) -> String {
        serde_json::to_string(&self.inner.borrow().backend_status)
            .expect("backend status is always serializable")
    }

    /// Number of `f64` slots [`Self::frame_stats_into`] writes. The façade allocates one scratch
    /// array of this length per chart and reuses it, so reading stats never allocates.
    pub fn frame_stats_len() -> usize {
        FRAME_STATS_LEN
    }

    /// Fill `out` with the last frame's telemetry (see `crate::telemetry::slot` for the layout).
    /// Writes in place rather than returning a fresh array so a host polling every frame stays
    /// allocation-free. The first call also arms WebGPU timestamp collection, so `gpu_ms` turns
    /// non-null from the *next* presented frame on a device with `timestamp-query`.
    pub fn frame_stats_into(&self, out: &mut [f64]) {
        let inner = self.inner.borrow();
        let gpu_ms = inner
            .gfx
            .as_ref()
            .and_then(|gfx| gfx.timer.as_ref())
            .and_then(GpuTimer::last_ms);
        inner.telemetry.write_into(out, gpu_ms);
    }

    /// Deterministic browser-matrix hook. This is intentionally absent from the public TypeScript
    /// chart API; it marks the current device as lost so the next render exercises real failover.
    #[doc(hidden)]
    pub fn simulate_device_loss_for_test(&mut self) {
        if let Some(gfx) = self.inner.borrow().gfx.as_ref() {
            gfx.device_lost.store(true, Ordering::Release);
            broadcast_gpu_loss();
        }
    }

    /// Test-only instrumentation for the `Prim::Text` texture cache: `{"entries":n,
    /// "rasterizations":n}` — identical text runs must rasterize once. Absent from the public
    /// TypeScript API (reached as `chart.wasm.text_cache_debug()` in the browser specs).
    #[doc(hidden)]
    pub fn text_cache_debug(&self) -> String {
        self.inner.borrow().text_runs.as_ref().map_or(
            "{\"entries\":0,\"rasterizations\":0}".to_string(),
            |store| store.debug_stats(),
        )
    }
}

impl ChartInner {
    // --- rendering ---
}

/// Outcome of a shared-GPU init attempt, delivered to every waiting `create_chart` call.
type SharedGpuResult = Result<Rc<SharedGpu>, BackendStartupFailure>;
type SharedGpuWaiters = Rc<RefCell<Vec<futures_channel::oneshot::Sender<SharedGpuResult>>>>;

/// Slot for the page-wide shared GPU context. `Pending` serializes concurrent `create_chart`
/// calls so exactly one adapter/device request is ever in flight; waiters are woken with the
/// outcome. An unavailable adapter is memoized until page reload so each virtualized chart does
/// not repeat the same failed browser request. Other initialization errors remain retryable; a
/// `Ready` context whose device was later lost is discarded and recreated on demand.
enum SharedGpuSlot {
    Empty,
    Pending(SharedGpuWaiters),
    Ready(Rc<SharedGpu>),
    AdapterUnavailable {
        error: BackendStartupFailure,
        force_fallback_adapter: bool,
    },
}

thread_local! {
    static SHARED_GPU: RefCell<SharedGpuSlot> = const { RefCell::new(SharedGpuSlot::Empty) };
}

enum SharedGpuAction {
    Ready(Rc<SharedGpu>),
    Wait(futures_channel::oneshot::Receiver<SharedGpuResult>),
    Create(SharedGpuWaiters),
    AdapterUnavailable(BackendStartupFailure),
}

async fn shared_gpu(force_fallback_adapter: bool) -> Result<Rc<SharedGpu>, BackendStartupFailure> {
    let action = SHARED_GPU.with(|slot| {
        let mut slot = slot.borrow_mut();
        if let SharedGpuSlot::AdapterUnavailable {
            error,
            force_fallback_adapter: attempted_fallback,
        } = &*slot
        {
            if *attempted_fallback == force_fallback_adapter {
                return SharedGpuAction::AdapterUnavailable(error.clone());
            }
            // Explicit fallback-adapter diagnostics must not inherit a failure from the normal
            // adapter request (or poison subsequent normal requests with their own result).
            *slot = SharedGpuSlot::Empty;
        }
        if let SharedGpuSlot::Ready(shared) = &*slot {
            if !shared.device_lost.load(Ordering::Acquire) {
                return SharedGpuAction::Ready(Rc::clone(shared));
            }
            // Lost device: drop the poisoned context; every chart already fell back to
            // Canvas2D individually, and new charts must not inherit a dead device.
            *slot = SharedGpuSlot::Empty;
        }
        match &*slot {
            SharedGpuSlot::Pending(waiters) => {
                let (tx, rx) = futures_channel::oneshot::channel();
                waiters.borrow_mut().push(tx);
                SharedGpuAction::Wait(rx)
            }
            SharedGpuSlot::Empty => {
                let waiters = Rc::new(RefCell::new(Vec::new()));
                *slot = SharedGpuSlot::Pending(Rc::clone(&waiters));
                SharedGpuAction::Create(waiters)
            }
            SharedGpuSlot::Ready(_) => unreachable!("ready slot handled above"),
            SharedGpuSlot::AdapterUnavailable { .. } => {
                unreachable!("adapter failure handled above")
            }
        }
    });
    match action {
        SharedGpuAction::Ready(shared) => Ok(shared),
        SharedGpuAction::AdapterUnavailable(error) => Err(error),
        SharedGpuAction::Wait(rx) => rx.await.map_err(|_| {
            BackendStartupFailure::initialization("shared GPU init dropped".to_string())
        })?,
        SharedGpuAction::Create(waiters) => {
            let result = create_shared_gpu(force_fallback_adapter).await;
            SHARED_GPU.with(|slot| {
                let mut slot = slot.borrow_mut();
                match &result {
                    Ok(shared) => *slot = SharedGpuSlot::Ready(Rc::clone(shared)),
                    Err(error) if error.is_adapter_unavailable() => {
                        *slot = SharedGpuSlot::AdapterUnavailable {
                            error: error.clone(),
                            force_fallback_adapter,
                        };
                    }
                    Err(_) => *slot = SharedGpuSlot::Empty,
                }
            });
            for tx in waiters.borrow_mut().drain(..) {
                let _ = tx.send(result.clone());
            }
            result
        }
    }
}

async fn create_shared_gpu(
    force_fallback_adapter: bool,
) -> Result<Rc<SharedGpu>, BackendStartupFailure> {
    let mut instance_descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
    instance_descriptor.backends = wgpu::Backends::BROWSER_WEBGPU;
    let instance = wgpu::Instance::new(instance_descriptor);
    let adapter = match instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter,
            apply_limit_buckets: false,
        })
        .await
    {
        Ok(adapter) => adapter,
        Err(high_performance_error) => instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::default(),
                compatible_surface: None,
                force_fallback_adapter,
                apply_limit_buckets: false,
            })
            .await
            .map_err(|default_error| {
                BackendStartupFailure::adapter(format!(
                    "request_adapter failed after high-performance and default attempts: {default_error}; high-performance attempt: {high_performance_error}"
                ))
            })?,
    };
    // `timestamp-query` powers `frame_stats().gpu_ms`. It is strictly optional: request it only
    // when the adapter advertises it, so a device lacking the feature (or a browser that has not
    // shipped it) still creates a chart — `gpu_ms` then reports `null` (frame_stats docs).
    let optional_features = adapter.features() & wgpu::Features::TIMESTAMP_QUERY;
    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor {
            required_features: optional_features,
            ..Default::default()
        })
        .await
        .map_err(|e| BackendStartupFailure::device(format!("request_device failed: {e}")))?;
    let device_lost = Arc::new(AtomicBool::new(false));
    let lost_flag = Arc::clone(&device_lost);
    device.set_device_lost_callback(move |reason, _message| {
        // `Destroyed` is the expected callback when resources are intentionally dropped during an
        // already-completed fallback. Only an unknown/driver loss needs to initiate recovery.
        if reason == wgpu::DeviceLostReason::Unknown {
            lost_flag.store(true, Ordering::Release);
            broadcast_gpu_loss();
        }
    });
    Ok(Rc::new(SharedGpu {
        atlas: RefCell::new(LabelAtlas::new(&device)),
        image_atlas: RefCell::new(LabelAtlas::new(&device)),
        renderers: RefCell::new(std::collections::HashMap::new()),
        instance,
        adapter,
        device,
        queue,
        device_lost,
    }))
}

/// Attempt to initialize WebGPU. A failure is recoverable because the same chart frame can be
/// executed by the Canvas2D backend. The adapter/device/queue/atlas/pipelines come from the
/// page-wide shared context (`shared_gpu`); only the surface, its config, and the MSAA target
/// are per chart.
async fn try_create_gfx(
    surface_target: wgpu::SurfaceTarget<'static>,
    css_width: f64,
    css_height: f64,
    dpr: f64,
    simulate_adapter_failure: bool,
    force_fallback_adapter: bool,
) -> Result<Gfx, BackendStartupFailure> {
    if simulate_adapter_failure {
        return Err(BackendStartupFailure::adapter(
            "webgpu found no adapters".to_string(),
        ));
    }
    let shared = shared_gpu(force_fallback_adapter).await?;
    let surface = shared
        .instance
        .create_surface(surface_target)
        .map_err(|e| BackendStartupFailure::surface(format!("create_surface failed: {e}")))?;
    let bitmap_w = (css_width * dpr).round().max(1.0) as u32;
    let bitmap_h = (css_height * dpr).round().max(1.0) as u32;
    let config = surface
        .get_default_config(&shared.adapter, bitmap_w, bitmap_h)
        .ok_or_else(|| {
            BackendStartupFailure::surface("surface not supported by adapter".to_string())
        })?;
    surface.configure(&shared.device, &config);
    let renderers = shared.renderers_for(config.format);
    let msaa = MsaaTarget::new(&shared.device, config.format, bitmap_w, bitmap_h);
    let device_lost = Arc::clone(&shared.device_lost);
    Ok(Gfx {
        shared,
        renderers,
        surface,
        config,
        msaa,
        device_lost,
        timer: None,
        frame_resources: FrameResources::default(),
    })
}
