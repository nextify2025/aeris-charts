//! Platform-free chart host layout negotiation.
//!
//! Hosts supply glyph widths, while the engine owns label formatting, axis visibility, grow-fast /
//! shrink-on-full policy, even-pixel axis snapping, pane geometry, and time-scale width.

use crate::{
    AxisFrame, ChartEngine, ChartFrame, FramePaneSegments, Pane, PriceScaleSide, PriceScaleTarget,
};
use aeris_charts_core::scale::time_scale_core::TimeScaleCore;
use aeris_charts_render::draw_list::Prim;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FinancialFramePreparation {
    pub frame_built: bool,
    pub axis_rebuilt: bool,
    pub layout_recomputed: bool,
    pub dpr_changed: bool,
}

/// One host viewport transaction and its reusable frame output buffers.
pub struct FinancialFrameRequest<'a> {
    pub width: f64,
    pub height: f64,
    pub dpr: f64,
    pub force_layout: bool,
    /// Full layout mutations may shrink axes; incremental repaints only grow them.
    pub allow_axis_shrink: bool,
    /// Rebuild for host-owned fixture or plugin changes that do not invalidate engine state.
    pub force_frame: bool,
    /// Host chrome or plugin changes can require new axis primitives without a pane relayout.
    pub force_axis: bool,
    /// Synchronous host getters need settled geometry before the next rendered frame.
    pub layout_only: bool,
    pub fit_content: bool,
    pub frame: &'a mut ChartFrame,
    /// Retain the engine's base axis frame for host-owned plugin labels.
    pub axis_frame: Option<&'a mut AxisFrame>,
    /// Native hosts lower axis primitives immediately. Plugin hosts lower after insertion.
    pub axis_primitives: Option<&'a mut Vec<Prim>>,
}

fn negotiated_axis_width(current: f64, measured: f64, allow_shrink: bool) -> f64 {
    if allow_shrink || current <= 0.0 {
        measured
    } else {
        current.max(measured)
    }
}

impl ChartEngine {
    /// Prepare one complete backend-neutral financial frame for a native host.
    ///
    /// The host supplies only viewport values and text measurement. The engine owns dimension
    /// installation, layout invalidation checks, optional initial fit, axis label-width policy,
    /// axis-frame construction, chart-frame construction, and axis primitive construction.
    pub fn prepare_financial_frame_with_measure<F, G>(
        &mut self,
        request: FinancialFrameRequest<'_>,
        measure: F,
        countdown_measure: G,
    ) -> FinancialFramePreparation
    where
        F: Fn(&str, bool) -> f64 + Copy,
        G: Fn(&str, bool) -> f64 + Copy,
    {
        if !request.width.is_finite()
            || !request.height.is_finite()
            || !request.dpr.is_finite()
            || request.width <= 0.0
            || request.height <= 0.0
            || request.dpr <= 0.0
        {
            return FinancialFramePreparation::default();
        }
        let dimensions_changed = self.css_width != request.width
            || self.css_height != request.height
            || self.dpr != request.dpr;
        let dpr_changed = self.dpr != request.dpr;
        if dimensions_changed {
            self.css_width = request.width;
            self.css_height = request.height;
            self.dpr = request.dpr;
        }
        self.begin_frame_build();
        let (input_frame, input_layout) = self.input.take_frame_invalidation();
        let layout_recomputed = dimensions_changed
            || request.force_layout
            || input_layout
            || self.frame_requires_layout();
        if !layout_recomputed
            && !request.force_frame
            && !request.force_axis
            && !input_frame
            && !self.frame_invalidated_since_prepare()
            && !self.frame_requires_layout()
            && !self.frame_requires_axis()
            && !request.frame.panes.is_empty()
        {
            return FinancialFramePreparation {
                frame_built: false,
                axis_rebuilt: false,
                layout_recomputed: false,
                dpr_changed,
            };
        }
        if layout_recomputed {
            self.recompute_layout_with_measure(
                request.allow_axis_shrink || dimensions_changed,
                measure,
                countdown_measure,
            );
            if request.fit_content {
                self.fit_content();
                self.recompute_layout_with_measure(true, measure, countdown_measure);
            }
        }
        if request.layout_only {
            return FinancialFramePreparation {
                frame_built: false,
                axis_rebuilt: false,
                layout_recomputed,
                dpr_changed,
            };
        }
        let max_label_width = self.axis_label_width_cap();
        self.build_frame_into_accumulating(request.frame);
        let axis_rebuilt = request.force_axis
            || request.axis_primitives.is_some()
            || layout_recomputed
            || self.frame_requires_axis()
            || request
                .axis_frame
                .as_ref()
                .is_some_and(|axis| axis.labels.is_empty());
        if axis_rebuilt {
            let axis_frame = self.build_axis_frame(max_label_width, measure, countdown_measure);
            if let Some(primitives) = request.axis_primitives {
                self.build_axis_primitives_into(&axis_frame, primitives);
            }
            if let Some(output) = request.axis_frame {
                *output = axis_frame;
            }
        }
        self.frame_prepared();
        FinancialFramePreparation {
            frame_built: true,
            axis_rebuilt,
            layout_recomputed,
            dpr_changed,
        }
    }

    /// Recompute pane and axis geometry from the current CSS size using host-native text widths.
    ///
    /// The operation is idempotent and performs the same two-pass refinement used by the browser
    /// host. `allow_axis_shrink` should be true for a full resize/layout and false for ordinary
    /// repaints, where axes grow immediately but do not visually breathe smaller.
    pub fn recompute_layout_with_measure<F, G>(
        &mut self,
        allow_axis_shrink: bool,
        measure: F,
        countdown_measure: G,
    ) where
        F: Fn(&str, bool) -> f64,
        G: Fn(&str, bool) -> f64,
    {
        self.frame_build_stats.layout_rebuilds += 1;
        // Tick density derives from the resolved axis metrics: sync before negotiating so the
        // measured tick sets match what the frame will build.
        self.sync_axis_tick_fonts();
        let content_h = (self.css_height - self.time_axis_height()).max(1.0);
        self.layout_panes(content_h);
        let right_visible = self.options.get().right_price_scale.visible;
        let left_visible = self.options.get().left_price_scale.visible;

        let measure_builtins = |engine: &mut ChartEngine| {
            let has_named_right = engine.panes.iter().any(|pane| {
                pane.named_scales
                    .iter()
                    .any(|entry| entry.visible && entry.side == PriceScaleSide::Right)
            });
            let has_named_left = engine.panes.iter().any(|pane| {
                pane.named_scales
                    .iter()
                    .any(|entry| entry.visible && entry.side == PriceScaleSide::Left)
            });
            let measured_right = if right_visible {
                if has_named_right {
                    (0..engine.panes.len())
                        .map(|pane| {
                            engine.optimal_exact_price_axis_width_for(
                                pane,
                                PriceScaleTarget::Right,
                                |text, bold| measure(text, bold),
                                |text, bold| countdown_measure(text, bold),
                            )
                        })
                        .fold(0.0_f64, f64::max)
                } else {
                    engine.optimal_price_axis_width_for(
                        PriceScaleTarget::Right,
                        |text, bold| measure(text, bold),
                        |text, bold| countdown_measure(text, bold),
                    )
                }
            } else {
                0.0
            };
            let measured_left = if left_visible {
                if has_named_left {
                    (0..engine.panes.len())
                        .map(|pane| {
                            engine.optimal_exact_price_axis_width_for(
                                pane,
                                PriceScaleTarget::Left,
                                |text, bold| measure(text, bold),
                                |text, bold| countdown_measure(text, bold),
                            )
                        })
                        .fold(0.0_f64, f64::max)
                } else {
                    engine.optimal_price_axis_width_for(
                        PriceScaleTarget::Left,
                        |text, bold| measure(text, bold),
                        |text, bold| countdown_measure(text, bold),
                    )
                }
            } else {
                0.0
            };
            engine.right_builtin_axis_w = if right_visible {
                negotiated_axis_width(
                    engine.right_builtin_axis_w,
                    measured_right,
                    allow_axis_shrink,
                )
            } else {
                0.0
            };
            engine.left_builtin_axis_w = if left_visible {
                negotiated_axis_width(engine.left_builtin_axis_w, measured_left, allow_axis_shrink)
            } else {
                0.0
            };
        };
        measure_builtins(self);

        let measure_named = |engine: &mut ChartEngine| {
            let targets: Vec<_> = engine
                .panes
                .iter()
                .enumerate()
                .flat_map(|(pane, state)| {
                    state
                        .named_scales
                        .iter()
                        .filter(|entry| entry.visible)
                        .map(move |entry| (pane, entry.id, entry.width))
                })
                .collect();
            for (pane, id, current) in targets {
                let target = PriceScaleTarget::Named(id);
                let measured = engine.optimal_exact_price_axis_width_for(
                    pane,
                    target,
                    |text, bold| measure(text, bold),
                    |text, bold| countdown_measure(text, bold),
                );
                if let Some(entry) = engine.panes[pane].named_scale_mut(id) {
                    entry.width = negotiated_axis_width(current, measured, allow_axis_shrink);
                }
            }
        };
        measure_named(self);
        self.measure_general_axis_widths(&measure, allow_axis_shrink);

        let side_total = |engine: &ChartEngine, pane_index: usize, side: PriceScaleSide| {
            let financial = engine.panes[pane_index]
                .ordered_side_targets(side)
                .into_iter()
                .filter(|target| engine.price_scale_visible_for(pane_index, *target))
                .filter_map(|target| engine.price_scale_axis_width(pane_index, target))
                .sum::<f64>();
            financial + engine.general_axis_side_width(pane_index, side)
        };
        let mut axis_w = (0..self.panes.len())
            .map(|pane| side_total(self, pane, PriceScaleSide::Right))
            .fold(0.0_f64, f64::max);
        let mut left_axis_w = (0..self.panes.len())
            .map(|pane| side_total(self, pane, PriceScaleSide::Left))
            .fold(0.0_f64, f64::max);

        for _ in 0..2 {
            let pane_w = (self.css_width - left_axis_w - axis_w).max(1.0);
            self.pane_left = left_axis_w;
            self.left_axis_w = left_axis_w;
            self.axis_w = axis_w;
            self.time_scale.set_width(pane_w);
            self.autoscale_visible();
            measure_builtins(self);
            measure_named(self);
            let new_w = (0..self.panes.len())
                .map(|pane| side_total(self, pane, PriceScaleSide::Right))
                .fold(0.0_f64, f64::max);
            let new_left_w = (0..self.panes.len())
                .map(|pane| side_total(self, pane, PriceScaleSide::Left))
                .fold(0.0_f64, f64::max);
            if new_w == axis_w && new_left_w == left_axis_w {
                break;
            }
            axis_w = new_w;
            left_axis_w = new_left_w;
        }

        self.pane_left = left_axis_w;
        self.left_axis_w = left_axis_w;
        self.pane_w = (self.css_width - left_axis_w - axis_w).max(1.0);
        self.pane_h = content_h;
        self.axis_w = axis_w;
        self.frame_layout_prepared();
    }

    /// Capture a complete frame and its axis layer at an image-export viewport, then restore
    /// the live viewport. The live host's retained frame is invalidated, so its next prepared
    /// frame rebuilds against its own measurements instead of reusing export geometry.
    ///
    /// A request with the live CSS size keeps the on-screen layout and only changes device
    /// resolution; another size runs a full layout with `measure` and `countdown_measure`.
    pub fn capture_export_frame<F, G>(
        &mut self,
        request: ExportFrameRequest,
        measure: F,
        countdown_measure: G,
    ) -> ExportFrame
    where
        F: Fn(&str, bool) -> f64 + Copy,
        G: Fn(&str, bool) -> f64 + Copy,
    {
        let resized = (request.width, request.height) != (self.css_width, self.css_height);
        // Restores on every exit, including a panicking host measure callback, so a caught
        // export failure never leaves the live chart at the export viewport.
        let live = LiveViewRestore::capture(self);
        let chart = &mut *live.engine;
        chart.css_width = request.width;
        chart.css_height = request.height;
        chart.dpr = request.dpr;
        chart.invalidate_frame_all();
        if resized {
            chart.recompute_layout_with_measure(true, measure, countdown_measure);
        }
        let frame = chart.build_frame();
        let segments = (0..frame.panes.len())
            .map(|pane| chart.frame_pane_segments(pane).unwrap_or_default())
            .collect();
        let axis_frame = chart.build_axis_frame_impl(
            chart.axis_label_width_cap(),
            measure,
            countdown_measure,
            request.include_crosshair,
        );
        let mut axis_primitives = Vec::new();
        chart.build_axis_primitives_into(&axis_frame, &mut axis_primitives);
        drop(live);
        ExportFrame {
            frame,
            segments,
            axis_primitives,
        }
    }
}

/// The live viewport, view, and layout state an export capture overwrites. Dropping the guard
/// writes it back and invalidates the retained frame so the live host rebuilds.
struct LiveViewRestore<'a> {
    engine: &'a mut ChartEngine,
    size: (f64, f64, f64),
    time_scale: TimeScaleCore,
    panes: Vec<Pane>,
    layout: [f64; 7],
    general_axis_thickness: Vec<f64>,
}

impl<'a> LiveViewRestore<'a> {
    fn capture(engine: &'a mut ChartEngine) -> Self {
        Self {
            size: (engine.css_width, engine.css_height, engine.dpr),
            time_scale: engine.time_scale.clone(),
            panes: engine.panes.clone(),
            layout: [
                engine.pane_w,
                engine.pane_h,
                engine.pane_left,
                engine.left_axis_w,
                engine.axis_w,
                engine.left_builtin_axis_w,
                engine.right_builtin_axis_w,
            ],
            general_axis_thickness: engine
                .general_axes
                .iter()
                .map(|axis| axis.layout_thickness)
                .collect(),
            engine,
        }
    }
}

impl Drop for LiveViewRestore<'_> {
    fn drop(&mut self) {
        let engine = &mut *self.engine;
        (engine.css_width, engine.css_height, engine.dpr) = self.size;
        std::mem::swap(&mut engine.time_scale, &mut self.time_scale);
        std::mem::swap(&mut engine.panes, &mut self.panes);
        [
            engine.pane_w,
            engine.pane_h,
            engine.pane_left,
            engine.left_axis_w,
            engine.axis_w,
            engine.left_builtin_axis_w,
            engine.right_builtin_axis_w,
        ] = self.layout;
        for (axis, thickness) in engine
            .general_axes
            .iter_mut()
            .zip(&self.general_axis_thickness)
        {
            axis.layout_thickness = *thickness;
        }
        engine.invalidate_frame_all();
    }
}

/// Viewport of one image-export capture, in CSS pixels at `dpr` device pixels per CSS pixel.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ExportFrameRequest {
    pub width: f64,
    pub height: f64,
    pub dpr: f64,
    /// Crosshair lines and their axis labels; hidden for a clean image.
    pub include_crosshair: bool,
}

/// A frame captured for image export: the pane layers with their paint-order segments, and the
/// unscissored axis/top layer painted above every pane.
#[derive(Clone, Debug, Default)]
pub struct ExportFrame {
    pub frame: ChartFrame,
    pub segments: Vec<FramePaneSegments>,
    pub axis_primitives: Vec<Prim>,
}

#[cfg(test)]
mod tests {
    use super::{negotiated_axis_width, FinancialFrameRequest};
    use crate::{ChartEngine, PriceScaleSide, SeriesKind};

    #[test]
    fn axes_grow_immediately_but_shrink_only_during_full_layout() {
        assert_eq!(negotiated_axis_width(58.0, 64.0, false), 64.0);
        assert_eq!(negotiated_axis_width(58.0, 52.0, false), 58.0);
        assert_eq!(negotiated_axis_width(58.0, 52.0, true), 52.0);
        assert_eq!(negotiated_axis_width(0.0, 56.0, false), 56.0);
    }

    #[test]
    fn axis_label_cap_uses_the_painted_axis_font() {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        assert_eq!(chart.axis_font_size(), 11.0);
        assert_eq!(chart.axis_label_width_cap(), 75.0);
        chart
            .options
            .apply_str(r#"{"layout":{"fontSize":24}}"#)
            .unwrap();
        chart.set_tick_mark_max_character_length(5);
        assert_eq!(chart.axis_font_size(), 22.0);
        assert_eq!(chart.axis_label_width_cap(), 81.25);
    }

    #[test]
    fn preparation_preserves_grow_only_axes_until_a_full_layout() {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        chart
            .set_series_data(
                0,
                &[1.0, 2.0],
                &[101.0, 102.0],
                &[102.0, 103.0],
                &[100.0, 101.0],
                &[101.0, 102.0],
            )
            .unwrap();
        let mut frame = crate::ChartFrame::default();
        let mut axis = Vec::new();
        let mut prepare = |allow_axis_shrink, glyph_width| {
            chart.prepare_financial_frame_with_measure(
                FinancialFrameRequest {
                    width: 800.0,
                    height: 500.0,
                    dpr: 1.0,
                    force_layout: true,
                    allow_axis_shrink,
                    force_frame: false,
                    force_axis: false,
                    layout_only: false,
                    fit_content: false,
                    frame: &mut frame,
                    axis_frame: None,
                    axis_primitives: Some(&mut axis),
                },
                |text, _| text.len() as f64 * glyph_width,
                |text, _| text.len() as f64 * glyph_width,
            );
            chart.axis_w
        };
        let wide = prepare(true, 12.0);
        assert_eq!(prepare(false, 4.0), wide);
        assert!(prepare(true, 4.0) < wide);
    }

    #[test]
    fn export_capture_restores_the_live_view_and_forces_its_rebuild() {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        chart
            .set_series_data(
                0,
                &[1.0, 2.0],
                &[101.0, 102.0],
                &[102.0, 103.0],
                &[100.0, 101.0],
                &[101.0, 102.0],
            )
            .unwrap();
        chart.crosshair = Some((200.0, 120.0));
        let mut frame = crate::ChartFrame::default();
        let mut axis = Vec::new();
        let mut prepare = |chart: &mut ChartEngine| {
            chart.prepare_financial_frame_with_measure(
                FinancialFrameRequest {
                    width: 800.0,
                    height: 500.0,
                    dpr: 1.0,
                    force_layout: false,
                    allow_axis_shrink: false,
                    force_frame: false,
                    force_axis: false,
                    layout_only: false,
                    fit_content: true,
                    frame: &mut frame,
                    axis_frame: None,
                    axis_primitives: Some(&mut axis),
                },
                |text, _| text.len() as f64 * 7.0,
                |text, _| text.len() as f64 * 6.0,
            )
        };
        prepare(&mut chart);
        let live = (chart.pane_w, chart.axis_w, chart.time_scale.width());

        let export = chart.capture_export_frame(
            super::ExportFrameRequest {
                width: 400.0,
                height: 300.0,
                dpr: 2.0,
                include_crosshair: false,
            },
            |text, _| text.len() as f64 * 7.0,
            |text, _| text.len() as f64 * 6.0,
        );
        assert_eq!(export.frame.pixel_ratio, 2.0);
        assert!(export.frame.width < 400.0 && export.frame.height < 300.0);
        assert_eq!(export.segments.len(), export.frame.panes.len());
        assert!(!export.axis_primitives.is_empty());
        assert!(chart.pane_w < 800.0 && chart.css_width == 800.0 && chart.dpr == 1.0);
        assert_eq!((chart.pane_w, chart.axis_w, chart.time_scale.width()), live);
        assert!(
            prepare(&mut chart).frame_built,
            "the live host must rebuild instead of reusing export layers"
        );
        assert_eq!((frame.width, frame.pixel_ratio), (live.0, 1.0));
    }

    #[test]
    fn export_capture_restores_the_live_view_when_measurement_panics() {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        chart
            .set_series_data(
                0,
                &[1.0, 2.0],
                &[101.0, 102.0],
                &[102.0, 103.0],
                &[100.0, 101.0],
                &[101.0, 102.0],
            )
            .unwrap();
        chart.fit_content();
        chart.recompute_layout_with_measure(
            true,
            |text, _| text.len() as f64 * 7.0,
            |text, _| text.len() as f64 * 6.0,
        );
        let live = (
            chart.css_width,
            chart.css_height,
            chart.dpr,
            chart.pane_w,
            chart.pane_h,
            chart.axis_w,
            chart.time_scale.width(),
            chart.panes.len(),
        );

        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            chart.capture_export_frame(
                super::ExportFrameRequest {
                    width: 400.0,
                    height: 300.0,
                    dpr: 2.0,
                    include_crosshair: false,
                },
                |_, _| -> f64 { panic!("host measurement failed") },
                |text, _| text.len() as f64 * 6.0,
            )
        }));
        assert!(outcome.is_err(), "the measure callback must have run");
        assert_eq!(
            (
                chart.css_width,
                chart.css_height,
                chart.dpr,
                chart.pane_w,
                chart.pane_h,
                chart.axis_w,
                chart.time_scale.width(),
                chart.panes.len(),
            ),
            live
        );
    }

    #[test]
    fn named_scale_labels_do_not_inflate_the_builtin_axis_strip() {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        chart
            .set_series_data(
                0,
                &[1.0, 2.0, 3.0],
                &[10.0, 11.0, 12.0],
                &[10.0, 11.0, 12.0],
                &[10.0, 11.0, 12.0],
                &[10.0, 11.0, 12.0],
            )
            .unwrap();
        let named = chart
            .add_price_scale(0, "large-values", PriceScaleSide::Right, None, true)
            .unwrap();
        chart.fit_content();
        chart.recompute_layout_with_measure(
            true,
            |text, _bold| text.len() as f64 * 7.0,
            |text, _bold| text.len() as f64 * 6.0,
        );
        let builtin_width = chart.right_builtin_axis_w;

        let comparison = chart.add_series(SeriesKind::Line);
        chart
            .set_series_data(
                comparison,
                &[1.0, 2.0, 3.0],
                &[1_000_000.0, 1_100_000.0, 1_200_000.0],
                &[1_000_000.0, 1_100_000.0, 1_200_000.0],
                &[1_000_000.0, 1_100_000.0, 1_200_000.0],
                &[1_000_000.0, 1_100_000.0, 1_200_000.0],
            )
            .unwrap();
        assert!(chart.series_apply_price_format_json(
            comparison,
            r#"{"type":"price","precision":4,"min_move":0.0001}"#
        ));
        chart.set_series_price_scale(comparison, named);
        chart.recompute_layout_with_measure(
            true,
            |text, _bold| text.len() as f64 * 7.0,
            |text, _bold| text.len() as f64 * 6.0,
        );

        assert_eq!(chart.right_builtin_axis_w, builtin_width);
        assert!(chart.price_scale_axis_width(0, named).unwrap() > builtin_width);
    }

    #[test]
    fn one_preparation_operation_owns_viewport_layout_axis_and_frame() {
        let mut chart = ChartEngine::new(1.0, 1.0, 1.0);
        chart
            .set_series_data(
                0,
                &[1.0, 2.0],
                &[10.0, 11.0],
                &[10.0, 11.0],
                &[10.0, 11.0],
                &[10.0, 11.0],
            )
            .unwrap();
        let mut frame = crate::ChartFrame::default();
        let mut axis = Vec::new();
        let prepared = chart.prepare_financial_frame_with_measure(
            FinancialFrameRequest {
                width: 800.0,
                height: 500.0,
                dpr: 2.0,
                force_layout: true,
                allow_axis_shrink: true,
                force_frame: false,
                force_axis: false,
                layout_only: false,
                fit_content: true,
                frame: &mut frame,
                axis_frame: None,
                axis_primitives: Some(&mut axis),
            },
            |text, _| text.len() as f64 * 7.0,
            |text, _| text.len() as f64 * 6.0,
        );
        assert!(prepared.frame_built);
        assert!(prepared.layout_recomputed);
        assert!(prepared.dpr_changed);
        assert_eq!(
            (chart.css_width, chart.css_height, chart.dpr),
            (800.0, 500.0, 2.0)
        );
        assert!(!frame.panes.is_empty());

        let retained = chart.prepare_financial_frame_with_measure(
            FinancialFrameRequest {
                width: 800.0,
                height: 500.0,
                dpr: 2.0,
                force_layout: false,
                allow_axis_shrink: false,
                force_frame: false,
                force_axis: false,
                layout_only: false,
                fit_content: false,
                frame: &mut frame,
                axis_frame: None,
                axis_primitives: Some(&mut axis),
            },
            |text, _| text.len() as f64 * 7.0,
            |text, _| text.len() as f64 * 6.0,
        );
        assert!(!retained.frame_built);

        let host_changed = chart.prepare_financial_frame_with_measure(
            FinancialFrameRequest {
                width: 800.0,
                height: 500.0,
                dpr: 2.0,
                force_layout: false,
                allow_axis_shrink: false,
                force_frame: true,
                force_axis: false,
                layout_only: false,
                fit_content: false,
                frame: &mut frame,
                axis_frame: None,
                axis_primitives: Some(&mut axis),
            },
            |text, _| text.len() as f64 * 7.0,
            |text, _| text.len() as f64 * 6.0,
        );
        assert!(host_changed.frame_built);
        assert!(!host_changed.layout_recomputed);

        chart.set_time_axis_visible(false);
        let frame_before_eager_layout = frame.clone();
        let eager_layout = chart.prepare_financial_frame_with_measure(
            FinancialFrameRequest {
                width: 800.0,
                height: 500.0,
                dpr: 2.0,
                force_layout: true,
                allow_axis_shrink: true,
                force_frame: false,
                force_axis: false,
                layout_only: true,
                fit_content: false,
                frame: &mut frame,
                axis_frame: None,
                axis_primitives: None,
            },
            |text, _| text.len() as f64 * 7.0,
            |text, _| text.len() as f64 * 6.0,
        );
        assert!(eager_layout.layout_recomputed);
        assert!(!eager_layout.frame_built);
        assert_eq!(frame, frame_before_eager_layout);
        let rendered = chart.prepare_financial_frame_with_measure(
            FinancialFrameRequest {
                width: 800.0,
                height: 500.0,
                dpr: 2.0,
                force_layout: false,
                allow_axis_shrink: false,
                force_frame: false,
                force_axis: false,
                layout_only: false,
                fit_content: false,
                frame: &mut frame,
                axis_frame: None,
                axis_primitives: Some(&mut axis),
            },
            |text, _| text.len() as f64 * 7.0,
            |text, _| text.len() as f64 * 6.0,
        );
        assert!(rendered.frame_built);
        assert!(!rendered.layout_recomputed);
    }
}
