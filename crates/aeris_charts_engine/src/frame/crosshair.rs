//! Crosshair geometry: pane hit, index snap, magnet price snap, crosshair prims.

use super::feature_geometry::positions_line;
use super::*;

/// Fallback for an unparsable `crosshair.shadeRight.color`: the default tint, the crosshair line
/// token at 12% alpha (`CrosshairShadeOptions::default().color`, alpha 0.12 * 255 rounds to 31).
const CROSSHAIR_SHADE_COLOR: Color = Color::rgba(
    DEFAULT_CROSSHAIR_LINE_RGB.0,
    DEFAULT_CROSSHAIR_LINE_RGB.1,
    DEFAULT_CROSSHAIR_LINE_RGB.2,
    31,
);

impl ChartEngine {
    /// Interactive chart objects own pointer feedback while they are hovered or manipulated.
    /// Keep the stored crosshair position for host callbacks and snapping, but do not paint its
    /// lines, markers, or labels through the object the pointer is acting on.
    pub(crate) fn crosshair_suppressed_by_interaction(&self) -> bool {
        // A live Shift-click measure reads the pointer through the crosshair, even while it
        // passes over other chart objects.
        if self.measure_following() {
            return false;
        }
        // An armed tool's placement stepped back to no points draws nothing at the pointer yet, so
        // it aims with the crosshair, as before its first click.
        self.hovered_drawing.is_some()
            || self.drawing_drag.is_some()
            || self.pending_drawing().is_some_and(|pending| {
                !pending.drawing.points.is_empty() || self.active_drawing_tool().is_none()
            })
            || self.brush_capture().is_some()
            || self.trading_state.feedback_hover.is_some()
            || matches!(
                self.trading_state.interaction,
                crate::trading::TradingInteractionState::Hovering { .. }
                    | crate::trading::TradingInteractionState::DraggingOrder { .. }
            )
    }

    pub(super) fn build_crosshair_frame(
        &self,
        pane_index: usize,
        pane_w_px: i32,
        hpr: f64,
        vpr: f64,
        out: &mut Vec<Prim>,
    ) {
        let Some((x_css, y_css)) = self.clamped_crosshair() else {
            return;
        };
        let Some((from, to)) = self.visible_range_for_frame() else {
            return;
        };
        if self.crosshair_mode == CrosshairMode::Hidden
            || self.crosshair_suppressed_by_interaction()
        {
            return;
        }
        let index = self.snapped_crosshair_index(x_css);
        let snapped_x = self.time_scale.index_to_coordinate(index);
        let ch = &self.options.get().crosshair;
        let vert_color = css_color(&ch.vert_line.color, CROSSHAIR_COLOR);
        let horz_color = css_color(&ch.horz_line.color, CROSSHAIR_COLOR);
        // reference lineWidth is in CSS px; generalize the crisp "1 CSS px" rule (grid uses the same
        // `max(1, floor(ratio))`) so width 1 keeps today's output. Vertical lines take the
        // horizontal ratio for thickness, horizontal lines the vertical ratio. Style is the
        // lineStyle u8 (default Dashed — the large pattern), expanded by the backends.
        let vert_width = 1f64.max((ch.vert_line.width * hpr).floor()) as i32;
        let horz_width = 1f64.max((ch.horz_line.width * vpr).floor()) as i32;
        let vert_style = crate::line_style_from_u8(ch.vert_line.style);
        let horz_style = crate::line_style_from_u8(ch.horz_line.style);
        let pane = &self.panes[pane_index];
        let y0 = (pane.top * vpr).round() as i32;
        let y1 = ((pane.top + pane.height) * vpr).round() as i32;
        // `shadeRight`: a veil from the snapped bar's right edge to the pane edge, in every
        // stacked pane like the vertical line (time-aligned, not gated on the hovered pane) and
        // independent of the line's own visibility. The edge comes from the same device-pixel
        // bar rule as the `HighlightBarCrosshair` primitive so the two abut exactly; when that
        // edge rounds onto or past the pane edge there is nothing to veil and no zero-width quad
        // reaches an executor. It paints before the lines so they stay untinted.
        if ch.shade_right.visible {
            let (left, width) = positions_line(snapped_x, hpr, self.time_scale.bar_spacing());
            let x0 = left + width;
            let w = pane_w_px - x0;
            if w > 0 {
                out.push(Prim::Rect {
                    rect: IRect {
                        x: x0,
                        y: y0,
                        w,
                        h: y1 - y0,
                    },
                    color: css_color(&ch.shade_right.color, CROSSHAIR_SHADE_COLOR),
                });
            }
        }
        if ch.vert_line.visible {
            out.push(Prim::VLine {
                x: (snapped_x * hpr).round() as i32,
                y0,
                y1,
                width: vert_width,
                style: vert_style,
                color: vert_color,
            });
        }
        if self.pane_at_y(y_css) != Some(pane_index) {
            return;
        }
        let snap_y = self.crosshair_snap(pane_index, x_css, y_css, from, to).1;
        if ch.horz_line.visible {
            out.push(Prim::HLine {
                y: (snap_y * vpr).round() as i32,
                x0: 0,
                x1: pane_w_px,
                width: horz_width,
                style: horz_style,
                color: horz_color,
            });
        }
        // reference crosshair-marks-pane-view.ts: one mark per visible line-like series holding a
        // bar at the crosshair index. Indicator outputs are ordinary engine line series, so they
        // use this same path without host-specific handling.
        let background = css_color(
            &self.options.get().layout.background.color,
            Color::rgb(0xff, 0xff, 0xff),
        );
        for series in &self.series {
            if !series.visible
                || !matches!(
                    series.kind,
                    SeriesKind::Line | SeriesKind::Area | SeriesKind::Baseline
                )
                || !series.crosshair_marker_visible
                || series.pane_index != pane_index
            {
                continue;
            }
            // The marker sits on the drawn line: an eased last bar reads its display values here
            // while the magnet snap above keeps the real ones.
            let plot = self.display_plot(series.id);
            let Some(row) = plot.search(index, MismatchDirection::None) else {
                continue;
            };
            let close = plot.value_at(row, PlotValueIndex::Close);
            if !close.is_finite() {
                continue;
            }
            let scale = pane_scale(pane, series_scale_target(series));
            if scale.is_empty() {
                continue;
            }
            let Some(base_value) = self.series_base_value(series.id, from) else {
                continue;
            };
            let baseline = if series.kind == SeriesKind::Baseline {
                self.resolved_baseline_price(plot, series.id, from, to)
            } else {
                None
            };
            let fill = series
                .crosshair_marker_background_color
                .as_deref()
                .and_then(Color::parse_css)
                .unwrap_or_else(|| self.series_bar_color(series, plot, row, baseline));
            let border = series
                .crosshair_marker_border_color
                .as_deref()
                .and_then(Color::parse_css)
                .unwrap_or(background);
            let cx = (snapped_x * hpr) as f32;
            let cy = (scale.price_to_coordinate(close, base_value) * vpr) as f32;
            if series.crosshair_marker_border_width > 0.0 {
                out.push(Prim::Circle {
                    cx,
                    cy,
                    radius: ((series.crosshair_marker_radius
                        + series.crosshair_marker_border_width)
                        * vpr) as f32,
                    fill: border,
                    stroke_width: 0.0,
                    stroke: border,
                });
            }
            out.push(Prim::Circle {
                cx,
                cy,
                radius: (series.crosshair_marker_radius * vpr) as f32,
                fill,
                stroke_width: 0.0,
                stroke: fill,
            });
        }
    }

    /// reference `PaneWidget._setCrosshairPosition` (pane-widget.ts:714-719) clamps the cursor into
    /// the pane instead of dropping the crosshair: x into `[0, width - 1]`, y into
    /// `[0, height - 1]` (height = the full stacked-pane region).
    pub(crate) fn clamped_crosshair(&self) -> Option<(f64, f64)> {
        let (x, y) = self.crosshair?;
        Some((
            x.clamp(0.0, (self.pane_w - 1.0).max(0.0)),
            y.clamp(0.0, (self.pane_h - 1.0).max(0.0)),
        ))
    }

    pub(crate) fn snapped_crosshair_index(&self, x_css: f64) -> i64 {
        // reference `setAndSaveCurrentPosition` clamps into visibleStrictRange — the FULL visible
        // window including the empty area (right offset), NOT the data-bounded range. In the
        // empty area the index lands on a hypothetical slot: the vertical line follows the
        // cursor there, while magnet/markers/the time label see no bar (exact searches miss).
        let index = self.time_scale.coordinate_to_index(x_css);
        let index = match self.time_scale.visible_strict_range() {
            Some(strict) => index.clamp(strict.left(), strict.right()),
            None => index,
        };
        self.snap_index_to_visible_series(index)
    }

    /// reference `Crosshair.snapToVisibleSeriesIfNeeded` (model/crosshair.ts:273-316): with
    /// `doNotSnapToHiddenSeriesIndices` set, move the snapped index to the nearest bar index
    /// held by any visible series (min |Δx|, ties to the left like the reference's `indexOf(min)`).
    /// Default off — the index is unchanged.
    fn snap_index_to_visible_series(&self, index: i64) -> i64 {
        if !self
            .options
            .get()
            .crosshair
            .do_not_snap_to_hidden_series_indices
        {
            return index;
        }
        let mut closest_left: Option<i64> = None;
        let mut closest_right: Option<i64> = None;
        for s in &self.series {
            if !s.visible {
                continue;
            }
            let plot = self.data.plot(s.id);
            // Whitespace rows hold no bar (the reference's plot list omits them); scan past them.
            if let Some(row) = plot.last_non_whitespace_row(index) {
                let candidate = plot.index_at(row).expect("crosshair row index");
                if candidate == index {
                    return index; // already snapped
                }
                closest_left = Some(closest_left.map_or(candidate, |l: i64| l.max(candidate)));
            }
            if let Some(row) = plot.first_non_whitespace_row(index) {
                let candidate = plot.index_at(row).expect("crosshair row index");
                if candidate == index {
                    return index; // already snapped
                }
                closest_right = Some(closest_right.map_or(candidate, |r: i64| r.min(candidate)));
            }
        }
        let x = self.time_scale.index_to_coordinate(index);
        let mut best = index;
        let mut best_dist = f64::INFINITY;
        for candidate in [closest_left, closest_right].into_iter().flatten() {
            let dist = (x - self.time_scale.index_to_coordinate(candidate)).abs();
            if dist < best_dist {
                best_dist = dist;
                best = candidate;
            }
        }
        best
    }

    /// The pane's default price scale (reference `Pane.defaultPriceScale`): the scale of the first
    /// visible, non-overlay series on the pane, else the pane's right scale. Returns the scale
    /// and its base (first) value for coordinate conversion.
    pub(crate) fn pane_default_scale_target(&self, pane_index: usize) -> PriceScaleTarget {
        let series = self.series.iter().find(|s| {
            s.visible
                && s.price_scale_target != PriceScaleTarget::Overlay
                && s.pane_index == pane_index
        });
        series
            .map(series_scale_target)
            .unwrap_or(PriceScaleTarget::Right)
    }

    pub(crate) fn pane_default_scale(
        &self,
        pane_index: usize,
        from: i64,
    ) -> (&PriceScaleCore, f64) {
        let target = self.pane_default_scale_target(pane_index);
        let series = self
            .series
            .iter()
            .find(|s| s.visible && s.pane_index == pane_index && series_scale_target(s) == target);
        let base_value = series
            .and_then(|s| self.series_base_value(s.id, from))
            .unwrap_or(0.0);
        (pane_scale(&self.panes[pane_index], target), base_value)
    }

    /// Resolve the rendered source-series candidate nearest `(x_css, y_css)` in pixel space.
    /// Drawings and the crosshair share this path so they choose the same bar, visible series, and
    /// field. Derived indicator and external-study outputs remain inspectable but never attract
    /// the magnet.
    /// OHLC mode exposes all four prices only for series that paint them; scalar-rendered series
    /// expose their close/value so hidden input columns cannot attract the magnet.
    pub(crate) fn magnet_snap_coordinate(
        &self,
        pane_index: usize,
        x_css: f64,
        y_css: f64,
        include_ohlc: bool,
    ) -> Option<(i64, f64)> {
        if !(0.0..=self.pane_w).contains(&x_css) || !y_css.is_finite() {
            return None;
        }
        let (from, _) = self.visible_range_for_frame()?;
        let index = self.snapped_crosshair_index(x_css);
        let pane = self.panes.get(pane_index)?;
        let mut candidates = Vec::new();
        for series in &self.series {
            if !series.visible
                || series.removed
                || self.indicator_binding_id(series.id).is_some()
                || self.external_study_for_series(series.id).is_some()
                || series.price_scale_target == PriceScaleTarget::Overlay
                || series.pane_index != pane_index
            {
                continue;
            }
            let scale = pane_scale(pane, series_scale_target(series));
            if scale.is_empty() {
                continue;
            }
            let plot = self.data.plot(series.id);
            let Some(row) = plot.search(index, MismatchDirection::None) else {
                continue;
            };
            if plot.is_whitespace_row(row) {
                continue;
            }
            let Some(base_value) = self.series_base_value(series.id, from) else {
                continue;
            };
            let keys: &[PlotValueIndex] = if include_ohlc
                && matches!(
                    series.kind,
                    SeriesKind::Candlestick | SeriesKind::Bar | SeriesKind::Footprint
                ) {
                &[
                    PlotValueIndex::Open,
                    PlotValueIndex::High,
                    PlotValueIndex::Low,
                    PlotValueIndex::Close,
                ]
            } else {
                &[PlotValueIndex::Close]
            };
            candidates.extend(keys.iter().filter_map(|&key| {
                let value = plot.value_at(row, key);
                value
                    .is_finite()
                    .then(|| scale.price_to_coordinate(value, base_value))
                    .filter(|coordinate| coordinate.is_finite())
            }));
        }
        magnet_snap_coordinate(y_css, &candidates).map(|coordinate| (index, coordinate))
    }

    /// Port of reference `Magnet.align` (model/magnet.ts:30-86): in Magnet modes the horizontal
    /// line snaps to the rendered-price candidate gathered from every visible, non-overlay source
    /// series on the pane with a bar exactly at the snapped index. Derived indicator outputs do not
    /// participate. OHLC-rendered series contribute all requested fields; scalar-rendered source
    /// series contribute only their close/value. The nearest candidate wins in *pixel* space
    /// (after conversion on its own series scale), then converts back to a price on the pane's
    /// default scale. Normal/Hidden mode, or no candidates, keeps the raw cursor price.
    pub(crate) fn crosshair_snap(
        &self,
        pane_index: usize,
        x_css: f64,
        y_css: f64,
        from: i64,
        _to: i64,
    ) -> (f64, f64) {
        let (default_scale, default_base) = self.pane_default_scale(pane_index, from);
        let price = default_scale.coordinate_to_price(y_css, default_base);
        // The snapped price source: the configured crosshair magnet mode, or, while a drawing is
        // armed, being created or dragged on a Normal-mode crosshair, that drawing's effective
        // magnet (`effective_drawing_magnet` with the held Ctrl/Cmd the input controller
        // reports) so the crosshair lands where the anchor will. A Weak magnet only
        // snaps inside its pixel radius.
        let (include_ohlc, radius) = match self.crosshair_mode {
            CrosshairMode::MagnetOhlc => (Some(true), None),
            CrosshairMode::Magnet => (Some(false), None),
            CrosshairMode::Normal => match self.drawing_work_magnet() {
                crate::DrawingMagnetMode::Strong => (Some(true), None),
                crate::DrawingMagnetMode::Weak => {
                    (Some(true), Some(crate::DRAWING_WEAK_MAGNET_DISTANCE))
                }
                crate::DrawingMagnetMode::Off => (None, None),
            },
            _ => (None, None),
        };
        let Some(include_ohlc) = include_ohlc else {
            return (price, y_css);
        };
        match self
            .magnet_snap_coordinate(pane_index, x_css, y_css, include_ohlc)
            .filter(|(_, nearest)| radius.is_none_or(|radius| (nearest - y_css).abs() <= radius))
        {
            Some((_, nearest)) => (
                default_scale.coordinate_to_price(nearest, default_base),
                nearest,
            ),
            None => (price, y_css),
        }
    }
}
