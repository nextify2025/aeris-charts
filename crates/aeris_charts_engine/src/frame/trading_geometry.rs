use super::*;
use crate::trading::{OrderRole, OrderSide, OrderStatus, PositionSide, TradingGroupVisualState};
use crate::Pane;
use aeris_charts_core::style::{
    DARK_ACCENT_RGB, DARK_ACTIVE_RGB, LIGHT_ACCENT_RGB, LIGHT_ACTIVE_RGB, RADIUS_LARGE,
    RADIUS_SMALL,
};

#[derive(Clone, Copy)]
struct TradingChipLayout {
    x: f64,
    y: f64,
    hpr: f64,
    vpr: f64,
}

/// Design-unit envelope of one execution arrow (`size` CSS px is 70 units).
const EXECUTION_ARROW_UNITS: f64 = 70.0;
/// Design-unit pitch between the chevrons of a multi-fill mark.
const EXECUTION_CHEVRON_PITCH: f64 = 24.0;
/// Most chevrons one mark draws, so a busy bar cannot grow its mark without limit.
const MAX_EXECUTION_CHEVRONS: usize = 5;

/// One execution arrow: every visible fill of one side on one bar.
pub(crate) struct TradingExecutionMark {
    /// Range of this mark's fills in [`TradingExecutionLayout::order`].
    pub fills: std::ops::Range<usize>,
    pub side: OrderSide,
    /// Bar center and arrow center in CSS px.
    pub x: f64,
    pub y: f64,
    /// Arrow width envelope in CSS px; one design unit is `size / 70`.
    pub size: f64,
    /// Vertical extent in CSS px: `size` for one chevron, one pitch taller per extra chevron.
    pub height: f64,
    /// Chevrons drawn: one per fill up to [`MAX_EXECUTION_CHEVRONS`]; 1 for non-arrow shapes.
    pub chevrons: usize,
}

#[derive(Default)]
pub(crate) struct TradingExecutionLayout {
    /// Execution slots sorted by bar, side, and time; marks index contiguous runs of it.
    pub order: Vec<usize>,
    pub marks: Vec<TradingExecutionMark>,
}

#[derive(Clone)]
struct TradingTooltip {
    text: String,
    layout: TradingChipLayout,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TradingControlSegmentKind {
    Quantity,
    Pnl,
    OrderType,
    TakeProfit,
    StopLoss,
    Cancel,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum TradingControlFeedback {
    #[default]
    Idle,
    Hovered,
    Pressed,
}

#[derive(Clone, Copy)]
struct TradingControlSegment<'a> {
    kind: TradingControlSegmentKind,
    text: &'a str,
    width: f64,
    color: Color,
    filled: bool,
}

/// One order or position marker: quantity, PnL/order type, and a trailing integrated close cell
/// inside one solid-outlined pill. Every segment before `Cancel` is a readout cell.
struct TradingControlCluster<'a> {
    segments: &'a [TradingControlSegment<'a>],
    left: f64,
    color: Color,
}

struct TradingTriggerLine {
    pane_index: usize,
    price_scale: crate::TradingPriceScale,
    trigger_price: Option<f64>,
    display_price: f64,
    color: Color,
}

impl<'a> TradingControlCluster<'a> {
    fn start(&self) -> f64 {
        self.left
    }

    fn close(&self) -> Option<&TradingControlSegment<'a>> {
        self.segments
            .last()
            .filter(|segment| segment.kind == TradingControlSegmentKind::Cancel)
    }

    fn body(&self) -> &'a [TradingControlSegment<'a>] {
        let body = self.segments.len() - usize::from(self.close().is_some());
        &self.segments[..body]
    }

    /// Width of the readout portion before the integrated close cell.
    fn body_width(&self) -> f64 {
        self.body().iter().map(|segment| segment.width).sum()
    }

    /// Width of the complete integrated marker — what the hit test measures against.
    fn width(&self) -> f64 {
        self.segments.iter().map(|segment| segment.width).sum()
    }
}

const QUANTITY_PAD_X: f64 = 8.0;
const MAX_QUANTITY_WIDTH: f64 = 120.0;
const PNL_WIDTH: f64 = 96.0;
const ORDER_TYPE_WIDTH: f64 = 92.0;
const PROTECTION_BUTTON_WIDTH: f64 = 30.0;
const PROTECTION_BUTTON_GAP: f64 = 4.0;
/// Separation between independent annotation chips.
const ANNOTATION_GAP: f64 = 5.0;
const ORDER_MARKER_SPAN: f64 = 304.0;
/// Visual inset for the close control's secondary-surface pill. Its larger containing cell remains
/// the hit target, so the affordance stays easy to activate without looking oversized.
const CLOSE_SURFACE_INSET: f64 = 3.0;
/// Deliberate chip-surface gap between the outline's inner edge and an edge cell's fill. It is
/// measured from the inside border, not the pill edge, so on the capsule ends the border's and the
/// fill's anti-aliased curves never share pixels and read as two clean rings.
const CELL_FILL_GAP: f64 = 1.5;
/// Vertical breathing room around the marker text. At the canonical 12px font this produces a
/// 24px control instead of compressing a financial action into the axis-label line box.
const CONTROL_PAD_Y: f64 = 12.0;

/// Protection semantics take precedence over their broker-side implementation: an SL remains
/// warning yellow and a TP remains profit green. Ordinary orders always follow their side token,
/// including resting buy and sell limits.
pub(crate) fn trading_order_color(
    style: &crate::TradingStyle,
    _kind: crate::OrderKind,
    side: OrderSide,
    role: OrderRole,
    status: OrderStatus,
) -> Color {
    let by_side = match side {
        OrderSide::Buy => style.buy,
        OrderSide::Sell => style.sell,
    };
    match status {
        OrderStatus::Rejected | OrderStatus::Cancelled | OrderStatus::Expired => style.rejected,
        OrderStatus::PendingSubmit | OrderStatus::PendingModify | OrderStatus::PendingCancel => {
            style.pending
        }
        _ if role == OrderRole::TakeProfit => style.take_profit,
        _ if role == OrderRole::StopLoss => style.stop_loss,
        OrderStatus::Filled | OrderStatus::Working | OrderStatus::PartiallyFilled => by_side,
    }
}

impl ChartEngine {
    fn trading_control_kind(kind: crate::TradingHitKind) -> Option<TradingControlSegmentKind> {
        match kind {
            crate::TradingHitKind::TakeProfitButton => Some(TradingControlSegmentKind::TakeProfit),
            crate::TradingHitKind::StopLossButton => Some(TradingControlSegmentKind::StopLoss),
            crate::TradingHitKind::CancelButton => Some(TradingControlSegmentKind::Cancel),
            _ => None,
        }
    }

    pub(crate) fn runtime_scale_base(&self, pane_index: usize, target: PriceScaleTarget) -> f64 {
        if let Some(base) = self
            .price_scale_for(pane_index, target)
            .and_then(|scale| scale.options().base_value)
        {
            return base;
        }
        self.visible_range()
            .and_then(|(from, _)| {
                let series = self.series.iter().find(|series| {
                    series.visible
                        && !series.removed
                        && series.pane_index == pane_index
                        && series_scale_target(series) == target
                })?;
                self.series_base_value(series.id, from)
            })
            .unwrap_or(0.0)
    }

    pub(crate) fn runtime_price_coordinate(
        &self,
        pane_index: usize,
        target: PriceScaleTarget,
        price: f64,
    ) -> Option<f64> {
        let pane = self.panes.get(pane_index)?;
        let scale = pane_scale(pane, target);
        if scale.is_empty() {
            return None;
        }
        Some(scale.price_to_coordinate(price, self.runtime_scale_base(pane_index, target)))
    }

    pub(crate) fn trading_price_coordinate(
        &self,
        pane_index: usize,
        target: crate::TradingPriceScale,
        price: f64,
    ) -> Option<f64> {
        let target = PriceScaleTarget::from(target);
        self.runtime_price_coordinate(pane_index, target, price)
    }

    /// Vertical extent `(top, bottom)` in CSS px of what the primary series of `target` paints
    /// within `half_width` of bar `index`: the wick for OHLC series (Heikin-Ashi when shown), the
    /// column top for histograms, and for line, area, and baseline series the stroked line itself
    /// across the mark's width (the slope toward each neighbor, or a stepped line's riser) padded
    /// by half the line width. Execution marks sit outside this extent, so they clear the rendered
    /// shape on every series type instead of touching a sloped line. Only the primary series
    /// anchors them: fills belong to the traded instrument's bars, so overlays on the same scale
    /// (moving averages, host studies, compare lines) must not push them away from their bar.
    fn trading_bar_extent(
        &self,
        pane_index: usize,
        target: PriceScaleTarget,
        index: i64,
        from: i64,
        half_width: f64,
    ) -> Option<(f64, f64)> {
        let pane = self.panes.get(pane_index)?;
        let scale = pane_scale(pane, target);
        if scale.is_empty() {
            return None;
        }
        let anchor = self
            .primary_series_on_price_scale(pane_index, target)?
            .series_id;
        let series = self.series.iter().find(|series| series.id == anchor)?;
        let render_end = self.series_render_end(series.id, i64::MAX);
        if !series.visible || self.indicator_binding_id(series.id).is_some() || index > render_end {
            return None;
        }
        let plot = self.data.plot(series.id);
        let row = plot.search(index, MismatchDirection::None)?;
        if plot.is_whitespace_row(row) {
            return None;
        }
        let base_value = self.series_base_value(series.id, from)?;
        let bar_spacing = self.time_scale.bar_spacing();
        let mut extent: Option<(f64, f64)> = None;
        let mut include = |y: f64, pad: f64| {
            if y.is_finite() {
                extent = Some(extent.map_or((y - pad, y + pad), |(top, bottom)| {
                    (top.min(y - pad), bottom.max(y + pad))
                }));
            }
        };
        let y_of = |price: f64| {
            if price.is_finite() {
                scale.price_to_coordinate(price, base_value)
            } else {
                f64::NAN
            }
        };
        match series.kind {
            SeriesKind::Candlestick | SeriesKind::Bar | SeriesKind::Footprint => {
                let [high, low] = self
                    .heikin_ashi_row(series.id, row)
                    .map(|values| [values[1], values[2]])
                    .unwrap_or_else(|| {
                        [
                            plot.value_at(row, PlotValueIndex::High),
                            plot.value_at(row, PlotValueIndex::Low),
                        ]
                    });
                include(y_of(high), 0.0);
                include(y_of(low), 0.0);
            }
            SeriesKind::Histogram => {
                include(y_of(plot.value_at(row, PlotValueIndex::Close)), 0.0);
            }
            SeriesKind::Line | SeriesKind::Area | SeriesKind::Baseline => {
                // Baseline quadrants may override the width; clear the widest stroke.
                let width = [series.top_line_width, series.bottom_line_width]
                    .into_iter()
                    .flatten()
                    .fold(series.line_width.unwrap_or(LINE_WIDTH), f64::max);
                let pad = width / 2.0;
                let y = y_of(plot.value_at(row, PlotValueIndex::Close));
                include(y, pad);
                for step in [-1_isize, 1] {
                    let Some(neighbor) = row.checked_add_signed(step) else {
                        continue;
                    };
                    let Some(neighbor_index) = plot.index_at(neighbor) else {
                        continue;
                    };
                    if neighbor_index > render_end || plot.is_whitespace_row(neighbor) {
                        continue;
                    }
                    let neighbor_y = y_of(plot.value_at(neighbor, PlotValueIndex::Close));
                    let gap = (neighbor_index - index).abs() as f64 * bar_spacing;
                    match series.line_type {
                        // A stepped line runs flat at its own value, then turns at the next
                        // bar: the previous bar's riser stands exactly on this bar's x.
                        LineType::WithSteps => {
                            if step < 0 {
                                include(neighbor_y, pad);
                            }
                        }
                        LineType::Simple | LineType::Curved => {
                            if gap > 0.0 {
                                let t = (half_width / gap).min(1.0);
                                include(y + (neighbor_y - y) * t, pad);
                            }
                        }
                    }
                }
            }
            SeriesKind::Custom | SeriesKind::Feature => {}
        }
        extent
    }

    /// Lay out execution marks for one pane, shared by the frame and hit testing. Each fill
    /// resolves to the bar that contains its time, and every visible fill of one side on one bar
    /// becomes a single arrow: buys below the bar, sells above it. Only visible bars are laid out,
    /// so work stays bounded by the viewport rather than the fill history.
    pub(crate) fn trading_execution_layout(&self, pane_index: usize) -> TradingExecutionLayout {
        let mut layout = TradingExecutionLayout::default();
        let Some((from, to)) = self.visible_range_for_frame() else {
            return layout;
        };
        let first_time = self.axis_time_key_at(0);
        let mut entries = Vec::new();
        for (slot, execution) in self.trading_state.executions.iter().enumerate() {
            if execution.pane_index != pane_index
                || !self
                    .trading_state
                    .account_visible(execution.account_id.as_ref())
                || !self.replay_time_is_visible(execution.time)
                // A fill before the loaded history has no bar to sit on yet.
                || first_time.is_some_and(|first| execution.time < first)
            {
                continue;
            }
            let Some(index) = self.axis_index_for_time(execution.time) else {
                continue;
            };
            let index = index as i64;
            if (from..=to).contains(&index) {
                let side = u8::from(execution.side == OrderSide::Sell);
                entries.push((index, side, execution.time, slot));
            }
        }
        entries.sort_unstable();
        layout.order = entries.iter().map(|&(.., slot)| slot).collect();

        let bar_spacing = self.time_scale.bar_spacing();
        let envelope = marker_envelope_size(bar_spacing);
        let margin = marker_margin(bar_spacing);
        let mut start = 0;
        while start < entries.len() {
            let (index, side, ..) = entries[start];
            let end = start
                + entries[start..]
                    .iter()
                    .take_while(|entry| entry.0 == index && entry.1 == side)
                    .count();
            let fills = &layout.order[start..end];
            let executions = &self.trading_state.executions;
            let quantity: f64 = fills.iter().map(|&slot| executions[slot].quantity).sum();
            let scale = if fills.iter().any(|&slot| executions[slot].size_by_quantity) {
                (quantity.abs().sqrt() / 2.0).clamp(0.75, 2.0)
            } else {
                1.0
            };
            let size = envelope.clamp(13.0, 18.0) * scale;
            let chevrons = if executions[fills[fills.len() - 1]].marker_shape
                == crate::ExecutionMarkerShape::Arrow
            {
                fills.len().min(MAX_EXECUTION_CHEVRONS)
            } else {
                1
            };
            let height = size
                * (EXECUTION_ARROW_UNITS + EXECUTION_CHEVRON_PITCH * (chevrons - 1) as f64)
                / EXECUTION_ARROW_UNITS;
            let target = PriceScaleTarget::from(executions[fills[0]].price_scale);
            let extent = self
                .trading_bar_extent(pane_index, target, index, from, size / 2.0)
                .or_else(|| {
                    // No painted bar here (whitespace or a custom series): bracket the fills.
                    fills
                        .iter()
                        .fold(None, |extent: Option<(f64, f64)>, &slot| {
                            let fill = &executions[slot];
                            let y = self.trading_price_coordinate(
                                pane_index,
                                fill.price_scale,
                                fill.price,
                            )?;
                            Some(extent.map_or((y, y), |(top, bottom)| (top.min(y), bottom.max(y))))
                        })
                });
            if let Some((top, bottom)) = extent {
                let side = if side == 0 {
                    OrderSide::Buy
                } else {
                    OrderSide::Sell
                };
                let y = match side {
                    OrderSide::Buy => bottom + margin + height / 2.0,
                    OrderSide::Sell => top - margin - height / 2.0,
                };
                layout.marks.push(TradingExecutionMark {
                    fills: start..end,
                    side,
                    x: self.time_scale.index_to_coordinate(index),
                    y,
                    size,
                    height,
                    chevrons,
                });
            }
            start = end;
        }
        layout
    }

    pub(crate) fn trading_coordinate_to_price(
        &self,
        pane_index: usize,
        target: crate::TradingPriceScale,
        y: f64,
    ) -> Option<f64> {
        let pane = self.panes.get(pane_index)?;
        let target = PriceScaleTarget::from(target);
        let scale = pane_scale(pane, target);
        if scale.is_empty() {
            return None;
        }
        Some(scale.coordinate_to_price(y, self.runtime_scale_base(pane_index, target)))
    }

    pub(crate) fn format_trading_price(&self, value: f64) -> String {
        match self.trading_state.instrument.price_precision {
            Some(precision) => format!("{value:.precision$}", precision = precision as usize),
            None => self.price_formatter.format(value),
        }
    }

    pub(crate) fn format_trading_quantity(&self, value: f64) -> String {
        match self.trading_state.instrument.quantity_precision {
            Some(precision) => format!("{value:.precision$}", precision = precision as usize),
            // Without host precision, show the exact size without trailing zeros. Eight places
            // covers fractional crypto sizes that four places would round away.
            None if value.fract().abs() < f64::EPSILON => format!("{value:.0}"),
            None => format!("{value:.8}")
                .trim_end_matches('0')
                .trim_end_matches('.')
                .to_string(),
        }
    }

    fn trading_order_preview(&self, order: &crate::WorkingOrder) -> Option<&crate::TradingPreview> {
        self.trading_state.interaction.preview().filter(|preview| {
            matches!(
                &preview.source,
                crate::TradingPreviewSource::Order { order_id } if order_id == &order.id
            )
        })
    }

    pub(crate) fn trading_effective_order_price(&self, order: &crate::WorkingOrder) -> f64 {
        self.trading_order_preview(order)
            .map_or(order.price, |preview| preview.price)
    }

    fn trading_pnl_text(&self, value: f64, currency: Option<&str>) -> String {
        let currency = currency
            .or(self.trading_state.instrument.currency.as_deref())
            .unwrap_or("");
        format!(
            "{}{value:.2}{}{}",
            if value >= 0.0 { "+" } else { "" },
            if currency.is_empty() { "" } else { " " },
            currency
        )
    }

    fn trading_chip_background(&self) -> Color {
        let fallback = aeris_charts_core::style::DEFAULT_SURFACE_RGB;
        Color::parse_css(&self.options.get().layout.background.color)
            .unwrap_or(Color::rgb(fallback.0, fallback.1, fallback.2))
            .solid()
    }

    /// Resolve a brand token for the theme the chart surface belongs to. Chart surfaces are
    /// opaque, so the shared token file carries the opaque equivalents of translucent host tokens.
    fn trading_theme_token(&self, light: (u8, u8, u8), dark: (u8, u8, u8)) -> Color {
        let token = if self.trading_chip_background().luminance() > 160.0 {
            light
        } else {
            dark
        };
        Color::rgb(token.0, token.1, token.2)
    }

    /// Neutral control feedback from the brand's `--hover-bg` / `--active-bg`. Feedback never
    /// takes the order's buy/sell color: the outline already identifies the order, and a colored
    /// hover would read as a state change of the order itself. Both stay opaque so the marker line
    /// never shows through the control under the pointer.
    fn trading_feedback_surface(&self, feedback: TradingControlFeedback) -> Option<Color> {
        match feedback {
            TradingControlFeedback::Idle => None,
            TradingControlFeedback::Hovered => {
                Some(self.trading_theme_token(LIGHT_ACCENT_RGB, DARK_ACCENT_RGB))
            }
            TradingControlFeedback::Pressed => {
                Some(self.trading_theme_token(LIGHT_ACTIVE_RGB, DARK_ACTIVE_RGB))
            }
        }
    }

    /// `--border-width` (1 CSS px) on the device grid with browser border semantics: whole
    /// device pixels, rounded down, never thinner than one device pixel.
    fn trading_border_width(vpr: f64) -> f64 {
        aeris_charts_core::style::border_width_device_px(vpr)
    }

    pub(crate) fn trading_control_height(&self) -> f64 {
        self.options.get().layout.font_size + CONTROL_PAD_Y
    }

    /// Device-pixel `(left, top, right, bottom)` of a control box centered on `y`, snapped once.
    /// The marker pill and the TP/SL buttons share this rect, so every control on a line has the
    /// same whole-pixel height instead of a fractional box antialiasing into an extra row.
    fn trading_control_rect(&self, left: f64, width: f64, y: f64, hpr: f64, vpr: f64) -> [f64; 4] {
        let height = self.trading_control_height();
        let top = ((y - height / 2.0) * vpr).round();
        [
            (left * hpr).round(),
            top,
            ((left + width) * hpr).round(),
            top + (height * vpr).round(),
        ]
    }

    /// Logical offset from a control's vertical center to its text anchor. `Prim::Text` anchors on
    /// the em-box middle, which leaves capitals and figures visibly high inside a padded control;
    /// the host glyph metric moves their ink onto the center so top and bottom padding match.
    /// Every cell of a marker shares this one offset, so adjacent readouts keep one baseline.
    fn trading_text_offset(&self) -> f64 {
        let layout = &self.options.get().layout;
        self.text_cap_center(layout.font_size, &layout.font_family, 400, false)
    }

    /// The close chip keeps equal width and height, so its glyph sits on the marker's rhythm.
    pub(crate) fn trading_close_width(&self) -> f64 {
        self.trading_control_height()
    }

    pub(crate) fn trading_marker_end(&self) -> f64 {
        self.pane_w.max(6.0)
    }

    pub(crate) fn trading_marker_start(&self) -> f64 {
        (self.trading_marker_end() - ORDER_MARKER_SPAN).max(6.0)
    }

    /// Quantity cells fit their formatted text instead of reserving a fixed-width box. The upper
    /// bound keeps extreme finite magnitudes from expanding a marker without limit.
    fn trading_quantity_width(&self, text: &str) -> f64 {
        let layout = &self.options.get().layout;
        (self.measure_text_run(text, layout.font_size, &layout.font_family, 400, false)
            + QUANTITY_PAD_X * 2.0)
            .ceil()
            .clamp(self.trading_control_height(), MAX_QUANTITY_WIDTH)
    }

    fn trading_order_quantity_text(&self, order: &crate::WorkingOrder) -> String {
        let remaining = (order.quantity - order.filled_quantity).max(0.0);
        if order.filled_quantity > 0.0 {
            format!(
                "{}/{}",
                self.format_trading_quantity(remaining),
                self.format_trading_quantity(order.quantity)
            )
        } else {
            self.format_trading_quantity(remaining)
        }
    }

    fn trading_position_quantity_text(&self, position: &crate::TradingPosition) -> String {
        let signed_quantity = if position.side == PositionSide::Long {
            position.quantity
        } else {
            -position.quantity
        };
        self.format_trading_quantity(signed_quantity)
    }

    /// Width of the second cell: a working order names itself, a protection order reports the PnL
    /// it would realise.
    fn trading_order_detail_width(order: &crate::WorkingOrder) -> f64 {
        if order.role == OrderRole::Working {
            ORDER_TYPE_WIDTH
        } else {
            PNL_WIDTH
        }
    }

    pub(crate) fn trading_order_cluster_width(&self, order: &crate::WorkingOrder) -> f64 {
        self.trading_quantity_width(&self.trading_order_quantity_text(order))
            + Self::trading_order_detail_width(order)
            + self.trading_close_width()
    }

    pub(crate) fn trading_position_cluster_width(&self, position: &crate::TradingPosition) -> f64 {
        self.trading_quantity_width(&self.trading_position_quantity_text(position))
            + PNL_WIDTH
            + self.trading_close_width()
    }

    fn trading_protection_button_hit(
        &self,
        show_take_profit: bool,
        show_stop_loss: bool,
        x: f64,
    ) -> Option<crate::TradingHitKind> {
        let mut right = self.trading_marker_start() - PROTECTION_BUTTON_GAP;
        for (visible, kind) in [
            (show_stop_loss, crate::TradingHitKind::StopLossButton),
            (show_take_profit, crate::TradingHitKind::TakeProfitButton),
        ] {
            if !visible {
                continue;
            }
            let left = right - PROTECTION_BUTTON_WIDTH;
            if x >= left && x <= right {
                return Some(kind);
            }
            right = left - PROTECTION_BUTTON_GAP;
        }
        None
    }

    pub(crate) fn trading_order_protection_hit(
        &self,
        order: &crate::WorkingOrder,
        x: f64,
    ) -> Option<crate::TradingHitKind> {
        self.trading_protection_button_hit(
            self.trading_order_protection_preview(order, OrderRole::TakeProfit)
                .is_some(),
            self.trading_order_protection_preview(order, OrderRole::StopLoss)
                .is_some(),
            x,
        )
    }

    pub(crate) fn trading_position_protection_hit(
        &self,
        position: &crate::TradingPosition,
        x: f64,
    ) -> Option<crate::TradingHitKind> {
        self.trading_protection_button_hit(
            self.trading_position_protection_preview(position, OrderRole::TakeProfit)
                .is_some(),
            self.trading_position_protection_preview(position, OrderRole::StopLoss)
                .is_some(),
            x,
        )
    }

    pub(crate) fn trading_annotation_hit(
        &self,
        annotations: &[crate::TradingAnnotation],
        line_y: f64,
        x: f64,
        y: f64,
    ) -> Option<String> {
        let height = self.trading_control_height();
        let mut cursor = self.trading_marker_start();
        let visible = annotations.len().min(3);
        for annotation in annotations.iter().take(visible) {
            let width = self.trading_annotation_width(&annotation.text);
            let center_y = match annotation.placement {
                crate::TradingAnnotationPlacement::Above => line_y - height - 2.0,
                crate::TradingAnnotationPlacement::Below => line_y + height + 2.0,
                crate::TradingAnnotationPlacement::Inline => line_y,
            };
            if x >= cursor && x <= cursor + width && (y - center_y).abs() <= height / 2.0 {
                return Some(annotation.id.clone());
            }
            cursor += width + ANNOTATION_GAP;
        }
        None
    }

    fn trading_annotation_width(&self, text: &str) -> f64 {
        (self.measure_text_run(
            text,
            self.options.get().layout.font_size,
            &self.options.get().layout.font_family,
            400,
            false,
        ) + 10.0)
            .ceil()
            .clamp(24.0, 180.0)
    }

    fn trading_annotation_color(&self, tone: crate::TradingAnnotationTone) -> Color {
        match tone {
            crate::TradingAnnotationTone::Neutral => self.trading_state.style.control,
            crate::TradingAnnotationTone::Info => self.trading_state.style.buy,
            crate::TradingAnnotationTone::Warning => self.trading_state.style.pending,
            crate::TradingAnnotationTone::Danger => self.trading_state.style.risk,
        }
    }

    pub(crate) fn push_trading_annotations(
        &self,
        out: &mut Vec<Prim>,
        annotations: &[crate::TradingAnnotation],
        line_y: f64,
        hpr: f64,
        vpr: f64,
    ) {
        let height = self.trading_control_height();
        let text_offset = self.trading_text_offset();
        let mut cursor = self.trading_marker_start();
        let visible = annotations.len().min(3);
        for annotation in annotations.iter().take(visible) {
            let width = self.trading_annotation_width(&annotation.text);
            let center_y = match annotation.placement {
                crate::TradingAnnotationPlacement::Above => line_y - height - 2.0,
                crate::TradingAnnotationPlacement::Below => line_y + height + 2.0,
                crate::TradingAnnotationPlacement::Inline => line_y,
            };
            let color = self.trading_annotation_color(annotation.tone);
            let device =
                super::DeviceBox::snap(cursor, center_y - height / 2.0, width, height, hpr, vpr);
            out.push(Prim::RoundRect {
                x: device.x,
                y: device.y,
                w: device.w,
                h: device.h,
                radii: [2.0; 4],
                fill: self.trading_chip_background(),
                border_width: Self::trading_border_width(vpr) as f32,
                border_color: color,
            });
            out.push(Prim::Text {
                x: device.center_x(),
                y: device.y + device.h / 2.0 + (text_offset * vpr) as f32,
                text: annotation.text.clone(),
                color,
                size: (self.options.get().layout.font_size * vpr) as f32,
                family: self.options.get().layout.font_family.clone(),
                align: TextAlign::Center,
                weight: 400,
                italic: false,
            });
            cursor += width + ANNOTATION_GAP;
        }
        if annotations.len() > visible {
            let text = format!("+{}", annotations.len() - visible);
            let width = self.trading_annotation_width(&text);
            let device =
                super::DeviceBox::snap(cursor, line_y - height / 2.0, width, height, hpr, vpr);
            out.push(Prim::RoundRect {
                x: device.x,
                y: device.y,
                w: device.w,
                h: device.h,
                radii: [2.0; 4],
                fill: self.trading_chip_background(),
                border_width: Self::trading_border_width(vpr) as f32,
                border_color: self.trading_state.style.control,
            });
            out.push(Prim::Text {
                x: device.center_x(),
                y: device.y + device.h / 2.0 + (text_offset * vpr) as f32,
                text,
                color: self.trading_state.style.control,
                size: (self.options.get().layout.font_size * vpr) as f32,
                family: self.options.get().layout.font_family.clone(),
                align: TextAlign::Center,
                weight: 400,
                italic: false,
            });
        }
    }

    fn push_host_trigger_line(
        &self,
        lines: &mut Vec<Prim>,
        trigger: TradingTriggerLine,
        hpr: f64,
        vpr: f64,
    ) {
        let Some(trigger_price) = trigger
            .trigger_price
            .filter(|price| *price != trigger.display_price)
        else {
            return;
        };
        let Some(y) =
            self.trading_price_coordinate(trigger.pane_index, trigger.price_scale, trigger_price)
        else {
            return;
        };
        lines.push(Prim::HLine {
            y: (y * vpr).round() as i32,
            x0: 0,
            x1: (self.pane_w * hpr).round() as i32,
            width: vpr.floor().max(1.0) as i32,
            style: LineStyle::Dotted,
            color: trigger.color,
        });
    }

    fn trading_cluster_hit(
        &self,
        left: f64,
        width: f64,
        x: f64,
        line: crate::TradingHitKind,
    ) -> crate::TradingHitKind {
        let close = self.trading_close_width();
        if x >= left + width - close && x <= left + width {
            crate::TradingHitKind::CancelButton
        } else {
            line
        }
    }

    pub(crate) fn trading_order_chip_hit(
        &self,
        order: &crate::WorkingOrder,
        x: f64,
    ) -> crate::TradingHitKind {
        self.trading_cluster_hit(
            self.trading_marker_start(),
            self.trading_order_cluster_width(order),
            x,
            crate::TradingHitKind::OrderLine,
        )
    }

    pub(crate) fn trading_position_chip_hit(
        &self,
        position: &crate::TradingPosition,
        x: f64,
    ) -> crate::TradingHitKind {
        self.trading_cluster_hit(
            self.trading_marker_start(),
            self.trading_position_cluster_width(position),
            x,
            crate::TradingHitKind::PositionLine,
        )
    }

    fn push_trading_segment(
        &self,
        out: &mut Vec<Prim>,
        segment: TradingControlSegment<'_>,
        feedback: TradingControlFeedback,
        layout: TradingChipLayout,
    ) {
        let TradingControlSegment {
            text,
            width,
            color,
            filled,
            ..
        } = segment;
        let TradingChipLayout { x, y, hpr, vpr } = layout;
        let font_size = self.options.get().layout.font_size;
        let [left, top, right, bottom] = self.trading_control_rect(x, width, y, hpr, vpr);
        let radius = (RADIUS_LARGE * hpr.min(vpr)).min((bottom - top) / 2.0) as f32;
        let fill = match (filled, feedback) {
            (true, TradingControlFeedback::Idle) => color.solid(),
            (true, TradingControlFeedback::Hovered) => color.solid().lighten(0.16),
            (true, TradingControlFeedback::Pressed) => color.solid().darken(0.72),
            (false, feedback) => self
                .trading_feedback_surface(feedback)
                .unwrap_or_else(|| self.trading_chip_background()),
        };
        out.push(Prim::RoundRect {
            x: left as f32,
            y: top as f32,
            w: (right - left) as f32,
            h: (bottom - top) as f32,
            radii: [radius; 4],
            fill,
            border_width: Self::trading_border_width(vpr) as f32,
            border_color: color,
        });
        out.push(Prim::Text {
            x: ((left + right) / 2.0) as f32,
            y: ((top + bottom) / 2.0 + self.trading_text_offset() * vpr) as f32,
            text: text.to_string(),
            color: if filled { color.contrast_text() } else { color },
            size: (font_size * vpr) as f32,
            family: self.options.get().layout.font_family.clone(),
            align: TextAlign::Center,
            weight: 400,
            italic: false,
        });
    }

    fn push_trading_protection_buttons(
        &self,
        out: &mut Vec<Prim>,
        show_take_profit: bool,
        show_stop_loss: bool,
        hovered: Option<crate::TradingHitKind>,
        pressed: Option<crate::TradingHitKind>,
        layout: TradingChipLayout,
    ) {
        let TradingChipLayout { y, hpr, vpr, .. } = layout;
        let mut right = self.trading_marker_start() - PROTECTION_BUTTON_GAP;
        for (visible, kind, segment_kind, label, color) in [
            (
                show_stop_loss,
                crate::TradingHitKind::StopLossButton,
                TradingControlSegmentKind::StopLoss,
                "SL",
                self.trading_state.style.stop_loss,
            ),
            (
                show_take_profit,
                crate::TradingHitKind::TakeProfitButton,
                TradingControlSegmentKind::TakeProfit,
                "TP",
                self.trading_state.style.take_profit,
            ),
        ] {
            if !visible {
                continue;
            }
            let left = right - PROTECTION_BUTTON_WIDTH;
            let feedback = if pressed == Some(kind) {
                TradingControlFeedback::Pressed
            } else if hovered == Some(kind) {
                TradingControlFeedback::Hovered
            } else {
                TradingControlFeedback::Idle
            };
            self.push_trading_segment(
                out,
                TradingControlSegment {
                    kind: segment_kind,
                    text: label,
                    width: PROTECTION_BUTTON_WIDTH,
                    color,
                    filled: false,
                },
                feedback,
                TradingChipLayout {
                    x: left,
                    y,
                    hpr,
                    vpr,
                },
            );
            right = left - PROTECTION_BUTTON_GAP;
        }
    }

    /// Draw the close icon as two anti-aliased strokes. `Polyline` is the one stroke primitive
    /// every executor antialiases identically; separate triangles and cap discs are not, and
    /// GPUI feathered the tiny cap discs into blobs around hard-edged arms.
    fn push_trading_close_icon(
        &self,
        out: &mut Vec<Prim>,
        points: &mut Vec<[f32; 2]>,
        center: (f64, f64),
        color: Color,
        hpr: f64,
        vpr: f64,
    ) {
        // Keep the icon optically compact inside its inset hover surface. The surrounding cell,
        // not the visible glyph, owns the larger interaction target.
        let arm = (self.options.get().layout.font_size * 0.30).max(3.5);
        let width = (1.5 * hpr.min(vpr)).max(1.0) as f32;
        let (center_x, center_y) = center;
        for slope in [1.0_f64, -1.0] {
            let first_point = points.len() as u32;
            points.extend([
                [
                    ((center_x - arm) * hpr) as f32,
                    ((center_y - arm * slope) * vpr) as f32,
                ],
                [
                    ((center_x + arm) * hpr) as f32,
                    ((center_y + arm * slope) * vpr) as f32,
                ],
            ]);
            out.push(Prim::Polyline {
                first_point,
                point_count: 2,
                width,
                style: LineStyle::Solid,
                line_type: LineType::Simple,
                color,
            });
        }
    }

    fn push_trading_cluster(
        &self,
        out: &mut Vec<Prim>,
        points: &mut Vec<[f32; 2]>,
        cluster: &TradingControlCluster<'_>,
        hovered: Option<TradingControlSegmentKind>,
        pressed: Option<TradingControlSegmentKind>,
        layout: TradingChipLayout,
    ) {
        let TradingChipLayout { y, hpr, vpr, .. } = layout;
        let font_size = self.options.get().layout.font_size;
        let color = cluster.color;
        let left = cluster.start();
        // Snap the pill to whole device pixels once. The container, cell fills, close surface,
        // and outline all derive from this rect, so their edges agree exactly and stay crisp.
        let [pill_left, pill_top, pill_right, pill_bottom] =
            self.trading_control_rect(left, cluster.width(), y, hpr, vpr);
        // The shared 999px token resolves with CSS border-radius clamping to half the height.
        let radius = (RADIUS_LARGE * hpr.min(vpr)).min((pill_bottom - pill_top) / 2.0) as f32;
        // Anchor text on the snapped pill's own center so glyphs and outline share one axis.
        let text_y = (pill_top + pill_bottom) / 2.0 + self.trading_text_offset() * vpr;
        let cell_feedback = |kind: TradingControlSegmentKind| {
            if pressed == Some(kind) {
                TradingControlFeedback::Pressed
            } else if hovered == Some(kind) {
                TradingControlFeedback::Hovered
            } else {
                TradingControlFeedback::Idle
            }
        };
        // One container owns the readout and close control with one solid semantic outline.
        out.push(Prim::RoundRect {
            x: pill_left as f32,
            y: pill_top as f32,
            w: (pill_right - pill_left) as f32,
            h: (pill_bottom - pill_top) as f32,
            radii: [radius; 4],
            fill: self.trading_chip_background(),
            border_width: Self::trading_border_width(vpr) as f32,
            border_color: color,
        });
        let border = Self::trading_border_width(vpr);
        let inset_x = border + (CELL_FILL_GAP * hpr).round();
        let inset_y = border + (CELL_FILL_GAP * vpr).round();
        let mut cursor = left;
        let body = cluster.body();
        for (index, segment) in body.iter().enumerate() {
            let feedback = cell_feedback(segment.kind);
            let fill = match (segment.filled, feedback) {
                (true, TradingControlFeedback::Idle) => Some(color.solid()),
                (true, TradingControlFeedback::Hovered) => Some(color.solid().lighten(0.16)),
                (true, TradingControlFeedback::Pressed) => Some(color.solid().darken(0.72)),
                (false, feedback) => self.trading_feedback_surface(feedback),
            };
            if let Some(fill) = fill {
                // Inset the fill inside the outline on every edge it shares with the pill, and
                // round those edges concentrically with the outline's capsule ends. Edges facing
                // a neighbouring cell stay flush and square.
                let left_edge = index == 0;
                let right_edge = index + 1 == cluster.segments.len();
                let fill_left = if left_edge {
                    pill_left + inset_x
                } else {
                    (cursor * hpr).round()
                };
                let fill_right = if right_edge {
                    pill_right - inset_x
                } else {
                    ((cursor + segment.width) * hpr).round()
                };
                let fill_top = pill_top + inset_y;
                let fill_bottom = pill_bottom - inset_y;
                let fill_radius = ((fill_bottom - fill_top) / 2.0) as f32;
                out.push(Prim::RoundRect {
                    x: fill_left as f32,
                    y: fill_top as f32,
                    w: (fill_right - fill_left) as f32,
                    h: (fill_bottom - fill_top) as f32,
                    radii: [
                        if left_edge { fill_radius } else { 0.0 },
                        if right_edge { fill_radius } else { 0.0 },
                        if right_edge { fill_radius } else { 0.0 },
                        if left_edge { fill_radius } else { 0.0 },
                    ],
                    fill,
                    border_width: 0.0,
                    border_color: fill,
                });
            }
            out.push(Prim::Text {
                x: ((cursor + segment.width / 2.0) * hpr) as f32,
                y: text_y as f32,
                text: segment.text.to_string(),
                color: if segment.filled {
                    color.contrast_text()
                } else {
                    segment.color
                },
                size: (font_size * vpr) as f32,
                family: self.options.get().layout.font_family.clone(),
                align: TextAlign::Center,
                weight: 400,
                italic: false,
            });
            cursor += segment.width;
        }
        // The close control is the integrated final cell, immediately after the PnL/order text.
        // At rest it sits directly on the container surface and draws its icon in the line's own
        // color; only hover/press paint the brand feedback surface behind it.
        if let Some(close) = cluster.close() {
            let surface_left = ((left + cluster.body_width() + CLOSE_SURFACE_INSET) * hpr).round();
            let surface_top = pill_top + (CLOSE_SURFACE_INSET * vpr).round();
            let surface_bottom = pill_bottom - (CLOSE_SURFACE_INSET * vpr).round();
            let surface_size = (surface_bottom - surface_top).max(1.0);
            if let Some(fill) = self.trading_feedback_surface(cell_feedback(close.kind)) {
                out.push(Prim::RoundRect {
                    x: surface_left as f32,
                    y: surface_top as f32,
                    w: surface_size as f32,
                    h: surface_size as f32,
                    radii: [(RADIUS_LARGE * hpr.min(vpr)).min(surface_size / 2.0) as f32; 4],
                    fill,
                    border_width: 0.0,
                    border_color: fill,
                });
            }
            self.push_trading_close_icon(
                out,
                points,
                (
                    (surface_left + surface_size / 2.0) / hpr,
                    (surface_top + surface_size / 2.0) / vpr,
                ),
                color,
                hpr,
                vpr,
            );
        }
    }

    pub(crate) fn trading_position_color(&self, side: PositionSide) -> Color {
        match side {
            PositionSide::Long => self.trading_state.style.buy,
            PositionSide::Short => self.trading_state.style.sell,
        }
    }

    fn trading_protection_pnl(
        &self,
        order: &crate::WorkingOrder,
        price: f64,
    ) -> Option<(String, Color)> {
        let position = self
            .trading_state
            .positions
            .iter()
            .find(|position| order.position_id.as_ref() == Some(&position.id))?;
        let direction = if position.side == PositionSide::Long {
            1.0
        } else {
            -1.0
        };
        let quantity = (order.quantity - order.filled_quantity).max(0.0);
        let value = (price - position.average_price)
            * direction
            * quantity
            * self.trading_state.instrument.point_value.unwrap_or(1.0);
        let color = if value >= 0.0 {
            self.trading_state.style.profit
        } else {
            self.trading_state.style.risk
        };
        Some((
            self.trading_pnl_text(value, position.currency.as_deref()),
            color,
        ))
    }

    fn push_trading_endpoint(&self, out: &mut Vec<Prim>, y: f64, color: Color, hpr: f64, vpr: f64) {
        out.push(Prim::Circle {
            cx: ((self.pane_w - 8.0) * hpr) as f32,
            cy: (y * vpr) as f32,
            radius: (3.0 * vpr) as f32,
            fill: self.trading_chip_background(),
            stroke_width: (1.0 * vpr) as f32,
            stroke: color,
        });
    }

    /// The chart's own border token, used for chrome that belongs to the surface rather than to a
    /// traded object.
    fn trading_chrome_border(&self) -> Color {
        let options = self.options.get();
        let fallback = aeris_charts_core::style::DEFAULT_BORDER_RGB;
        Color::parse_css(&options.right_price_scale.border_color)
            .or_else(|| Color::parse_css(&options.left_price_scale.border_color))
            .unwrap_or(Color::rgb(fallback.0, fallback.1, fallback.2))
            .solid()
    }

    /// An action tooltip is chart chrome, not part of the object it describes: it follows the
    /// active theme's surface, border, and text tokens rather than the order's buy/sell color, so
    /// it reads the same on every line and in both themes.
    fn push_trading_tooltip(
        &self,
        out: &mut Vec<Prim>,
        text: &str,
        pane: &Pane,
        layout: TradingChipLayout,
    ) {
        let font_size = self.options.get().layout.font_size;
        let above =
            layout.y - (font_size + 5.0) / 2.0 - Self::trading_tooltip_height(font_size) - 5.0;
        let y = if above >= pane.top + 2.0 {
            above
        } else {
            layout.y + (font_size + 5.0) / 2.0 + 5.0
        };
        self.push_trading_tooltip_box(out, text, layout.x, y, layout.hpr, layout.vpr);
    }

    pub(super) fn trading_tooltip_height(font_size: f64) -> f64 {
        font_size + 7.0
    }

    /// Tooltip chrome centered on `center_x` with its top edge at `y` (CSS px).
    pub(super) fn push_trading_tooltip_box(
        &self,
        out: &mut Vec<Prim>,
        text: &str,
        center_x: f64,
        y: f64,
        hpr: f64,
        vpr: f64,
    ) {
        let font_size = self.options.get().layout.font_size;
        let height = Self::trading_tooltip_height(font_size);
        let radius = (RADIUS_SMALL * hpr.min(vpr)) as f32;
        let width = self.measure_text_run(
            text,
            font_size,
            &self.options.get().layout.font_family,
            400,
            false,
        ) + 12.0;
        let x = (center_x - width / 2.0).clamp(4.0, (self.pane_w - width - 4.0).max(4.0));
        let device = super::DeviceBox::snap(x, y, width, height, hpr, vpr);
        out.push(Prim::RoundRect {
            x: device.x,
            y: device.y,
            w: device.w,
            h: device.h,
            radii: [radius.round(); 4],
            fill: self.trading_chip_background(),
            border_width: Self::trading_border_width(vpr) as f32,
            border_color: self.trading_chrome_border(),
        });
        out.push(Prim::Text {
            x: device.center_x(),
            y: device.y + device.h / 2.0 + (self.trading_text_offset() * vpr) as f32,
            text: text.to_string(),
            color: self.primary_text_color(),
            size: (font_size * vpr) as f32,
            family: self.options.get().layout.font_family.clone(),
            align: TextAlign::Center,
            weight: 400,
            italic: false,
        });
    }

    /// One execution mark centered on `(x_device, mark.y)`, drawn as open strokes with
    /// `Polyline`, the stroke primitive every executor antialiases identically. In design units
    /// (`size / 70`): a single fill is a 60-unit shaft with one chevron (wings 22 out and back
    /// from the tip, stroke 10). Each further fill on one side of one bar stacks one identical,
    /// tailless chevron 24 units nearer the bar (up to [`MAX_EXECUTION_CHEVRONS`]), so the mark
    /// counts the fills while only the outermost chevron carries the shaft.
    fn push_trading_execution_arrow(
        out: &mut Vec<Prim>,
        points: &mut Vec<[f32; 2]>,
        mark: &TradingExecutionMark,
        shape: crate::ExecutionMarkerShape,
        color: Color,
        layout: TradingChipLayout,
    ) {
        let TradingChipLayout {
            x: x_device,
            hpr,
            vpr,
            ..
        } = layout;
        let unit = mark.size / 70.0;
        // Up for buys, down for sells.
        let direction = if mark.side == OrderSide::Buy {
            -1.0
        } else {
            1.0
        };
        let point = |dx: f64, dy: f64| {
            [
                (x_device + dx * unit * hpr) as f32,
                ((mark.y + direction * dy * unit) * vpr) as f32,
            ]
        };
        match shape {
            crate::ExecutionMarkerShape::Arrow => {
                let mut stroke = |path: &[[f32; 2]], stroke_units: f64| {
                    let first_point = points.len() as u32;
                    points.extend_from_slice(path);
                    out.push(Prim::Polyline {
                        first_point,
                        point_count: path.len() as u32,
                        width: ((stroke_units * unit).max(1.5) * hpr.min(vpr)) as f32,
                        style: LineStyle::Solid,
                        line_type: LineType::Simple,
                        color,
                    });
                };
                // The outermost chevron (the newest fill) keeps the single arrow's shaft; every
                // earlier fill adds one tailless chevron of the same size, one pitch nearer the bar.
                let outer_tip = 30.0 - EXECUTION_CHEVRON_PITCH * (mark.chevrons - 1) as f64 / 2.0;
                stroke(&[point(0.0, outer_tip - 60.0), point(0.0, outer_tip)], 10.0);
                for k in 0..mark.chevrons {
                    let tip = outer_tip + EXECUTION_CHEVRON_PITCH * k as f64;
                    stroke(
                        &[
                            point(-22.0, tip - 22.0),
                            point(0.0, tip),
                            point(22.0, tip - 22.0),
                        ],
                        10.0,
                    );
                }
            }
            crate::ExecutionMarkerShape::Triangle => out.push(Prim::Triangle {
                a: point(0.0, 30.0),
                b: point(-30.0, -30.0),
                c: point(30.0, -30.0),
                color,
            }),
            crate::ExecutionMarkerShape::Circle => out.push(Prim::Circle {
                cx: x_device as f32,
                cy: (mark.y * vpr) as f32,
                radius: (mark.size * 0.4 * hpr) as f32,
                fill: color,
                stroke_width: 0.0,
                stroke: color,
            }),
        }
    }

    /// Execution arrows, then the exact-fill detail of the hovered or pressed arrow: a tick at
    /// every fill's own price on the bar, a dotted lead from the arrow, and a fill tooltip placed
    /// on the arrow's outer side so it never covers the bar.
    fn push_trading_executions(
        &self,
        out: &mut Vec<Prim>,
        points: &mut Vec<[f32; 2]>,
        pane: &Pane,
        pane_index: usize,
        hpr: f64,
        vpr: f64,
    ) {
        let min_line_width = vpr.floor().max(1.0) as i32;
        let layout = self.trading_execution_layout(pane_index);
        if layout.marks.is_empty() {
            return;
        }
        let executions = &self.trading_state.executions;
        let focused = |slot: &usize| {
            [
                &self.trading_state.feedback_hover,
                &self.trading_state.feedback_pressed,
            ]
            .into_iter()
            .flatten()
            .any(|hit| {
                matches!(&hit.object, crate::TradingObjectId::Execution(id) if id == &executions[*slot].id)
            })
        };
        // Same device-pixel centering as series markers, so both arrow kinds align on a bar.
        let correction = (hpr.floor() as i64).rem_euclid(2) as f64 * 0.5;
        let mut detail = None;
        for mark in &layout.marks {
            let fills = &layout.order[mark.fills.clone()];
            let color = match mark.side {
                OrderSide::Buy => self.trading_state.style.execution_buy,
                OrderSide::Sell => self.trading_state.style.execution_sell,
            };
            let x = (mark.x * hpr).round() + correction;
            let shape = executions[fills[fills.len() - 1]].marker_shape;
            Self::push_trading_execution_arrow(
                out,
                points,
                mark,
                shape,
                color,
                TradingChipLayout {
                    x,
                    y: mark.y,
                    hpr,
                    vpr,
                },
            );
            if detail.is_none() && fills.iter().any(focused) {
                detail = Some((mark, fills, color));
            }
        }

        let Some((mark, fills, color)) = detail else {
            return;
        };
        let tick_half = (self.time_scale.bar_spacing() * 0.5).clamp(4.0, 12.0);
        let x = (mark.x * hpr).round() as i32;
        let (arrow_top, arrow_bottom) = (mark.y - mark.height / 2.0, mark.y + mark.height / 2.0);
        // Fills may sit on either side of the arrow (a line paints only the close), so the lead
        // and tooltip span whichever side the exact prices fall on.
        let (mut fills_top, mut fills_bottom) = (arrow_top, arrow_bottom);
        let mut quantity = 0.0;
        let mut notional = 0.0;
        for &slot in fills {
            let fill = &executions[slot];
            quantity += fill.quantity;
            notional += fill.price * fill.quantity;
            let Some(fill_y) =
                self.trading_price_coordinate(pane_index, fill.price_scale, fill.price)
            else {
                continue;
            };
            fills_top = fills_top.min(fill_y);
            fills_bottom = fills_bottom.max(fill_y);
            out.push(Prim::HLine {
                y: (fill_y * vpr).round() as i32,
                x0: ((mark.x - tick_half) * hpr).round() as i32,
                x1: ((mark.x + tick_half) * hpr).round() as i32,
                width: min_line_width,
                style: LineStyle::Solid,
                color,
            });
            out.push(Prim::Circle {
                cx: (mark.x * hpr) as f32,
                cy: (fill_y * vpr) as f32,
                radius: (2.5 * vpr) as f32,
                fill: self.trading_chip_background(),
                stroke_width: (1.0 * vpr) as f32,
                stroke: color,
            });
        }
        for (y0, y1) in [(fills_top, arrow_top), (arrow_bottom, fills_bottom)] {
            if y1 > y0 {
                out.push(Prim::VLine {
                    x,
                    y0: (y0 * vpr).round() as i32,
                    y1: (y1 * vpr).round() as i32,
                    width: min_line_width,
                    style: LineStyle::Dotted,
                    color,
                });
            }
        }

        let side = match mark.side {
            OrderSide::Buy => "Buy",
            OrderSide::Sell => "Sell",
        };
        let text = if fills.len() == 1 {
            let fill = &executions[fills[0]];
            format!(
                "{side} {} @ {}",
                self.format_trading_quantity(fill.quantity),
                self.format_trading_price(fill.price)
            )
        } else {
            format!(
                "{side} {} @ {} avg · {} fills",
                self.format_trading_quantity(quantity),
                self.format_trading_price(notional / quantity),
                fills.len()
            )
        };
        let height = Self::trading_tooltip_height(self.options.get().layout.font_size);
        let above = fills_top - 4.0 - height;
        let below = fills_bottom + 4.0;
        let fits_above = above >= pane.top + 2.0;
        let fits_below = below + height <= pane.top + pane.height - 2.0;
        let y = match mark.side {
            OrderSide::Buy if fits_below || !fits_above => below,
            OrderSide::Sell if fits_above || !fits_below => above,
            OrderSide::Buy => above,
            OrderSide::Sell => below,
        };
        self.push_trading_tooltip_box(out, &text, mark.x, y, hpr, vpr);
    }

    #[cfg(test)]
    pub(crate) fn build_trading_frame_for_test(
        &self,
        pane_index: usize,
        hpr: f64,
        vpr: f64,
        regions: &mut Vec<Prim>,
        lines: &mut Vec<Prim>,
    ) {
        let mut points = Vec::new();
        self.build_trading_frame(pane_index, hpr, vpr, regions, lines, &mut points);
    }

    pub(super) fn build_trading_frame(
        &self,
        pane_index: usize,
        hpr: f64,
        vpr: f64,
        regions: &mut Vec<Prim>,
        lines: &mut Vec<Prim>,
        points: &mut Vec<[f32; 2]>,
    ) {
        let Some(pane) = self.panes.get(pane_index) else {
            return;
        };
        let width = (self.pane_w * hpr).round() as i32;
        let min_line_width = vpr.floor().max(1.0) as i32;
        let mut tooltip = None;

        if let Some(clock_micros) = self.replay_clock_micros {
            let time = if self.sequence_points().is_some() {
                clock_micros as f64 / 1_000_000.0
            } else {
                clock_micros.div_euclid(1_000_000) as f64
            };
            if let Some(index) = self.time_to_index(time, true) {
                let x = self.time_scale.index_to_coordinate(index);
                let color = Color::parse_css(match self.theme {
                    crate::ChartTheme::Light => aeris_charts_core::style::LIGHT_PRIMARY_CSS,
                    crate::ChartTheme::Dark => aeris_charts_core::style::DARK_PRIMARY_CSS,
                })
                .unwrap_or(self.trading_state.style.control);
                lines.push(Prim::VLine {
                    x: (x * hpr).round() as i32,
                    y0: (pane.top * vpr).round() as i32,
                    y1: ((pane.top + pane.height) * vpr).round() as i32,
                    width: min_line_width,
                    style: LineStyle::Dashed,
                    color,
                });
                lines.push(Prim::Text {
                    x: (x * hpr) as f32,
                    y: ((pane.top + 12.0) * vpr) as f32,
                    text: "Replay".to_string(),
                    color,
                    size: (self.options.get().layout.font_size * vpr) as f32,
                    family: self.options.get().layout.font_family.clone(),
                    align: TextAlign::Center,
                    weight: 600,
                    italic: false,
                });
            }
        }

        // Host context is a separate, non-persisted layer. Windows are lowered first and event
        // markers use deterministic LOD collapse when releases share the same pixel column.
        if !self.data.merged_times().is_empty() {
            let logical = |time: i64| self.axis_index_for_time(time).map(|index| index as i64);
            for window in &self.trading_state.host_overlay.windows {
                if !self.replay_time_is_visible(window.start_time) {
                    continue;
                }
                let end_time = self
                    .replay_cutoff_seconds()
                    .map_or(window.end_time, |cutoff| window.end_time.min(cutoff));
                let Some(start_index) = logical(window.start_time) else {
                    continue;
                };
                let Some(end_index) = logical(end_time) else {
                    continue;
                };
                let x0 = self.time_scale.index_to_coordinate(start_index);
                let x1 = self.time_scale.index_to_coordinate(end_index);
                regions.push(Prim::Rect {
                    rect: IRect {
                        x: (x0.min(x1) * hpr).round() as i32,
                        y: (pane.top * vpr).round() as i32,
                        w: ((x1 - x0).abs() * hpr).round().max(1.0) as i32,
                        h: (pane.height * vpr).round().max(1.0) as i32,
                    },
                    color: Color::rgba(
                        self.trading_state.style.pending.r(),
                        self.trading_state.style.pending.g(),
                        self.trading_state.style.pending.b(),
                        24,
                    ),
                });
            }
            let mut last_event_x = f64::NEG_INFINITY;
            for event in &self.trading_state.host_overlay.events {
                if !self.replay_time_is_visible(event.time) {
                    continue;
                }
                let Some(index) = logical(event.time) else {
                    continue;
                };
                let x = self.time_scale.index_to_coordinate(index);
                if x - last_event_x < 8.0 {
                    continue;
                }
                last_event_x = x;
                let color = if event.importance >= 2 {
                    self.trading_state.style.risk
                } else {
                    self.trading_state.style.control
                };
                lines.push(Prim::VLine {
                    x: (x * hpr).round() as i32,
                    y0: (pane.top * vpr).round() as i32,
                    y1: ((pane.top + pane.height) * vpr).round() as i32,
                    width: min_line_width,
                    style: LineStyle::Dotted,
                    color,
                });
                if !event.label.is_empty() {
                    lines.push(Prim::Text {
                        x: (x * hpr) as f32,
                        y: ((pane.top + 12.0) * vpr) as f32,
                        text: event.label.clone(),
                        color,
                        size: (self.options.get().layout.font_size * vpr) as f32,
                        family: self.options.get().layout.font_family.clone(),
                        align: TextAlign::Center,
                        weight: if event.importance >= 2 { 700 } else { 400 },
                        italic: false,
                    });
                }
            }
        }

        // A control lights up only while it can act; a locked or status-inert one shows no hover
        // or press feedback and no tooltip (`trading_feedback`).
        let feedback_hover = self.trading_feedback(self.trading_state.feedback_hover.as_ref());
        let feedback_pressed = self.trading_feedback(self.trading_state.feedback_pressed.as_ref());
        for position in &self.trading_state.positions {
            if !self
                .trading_state
                .account_visible(position.account_id.as_ref())
            {
                continue;
            }
            if position.pane_index != pane_index {
                continue;
            }
            let Some(y) = self.trading_price_coordinate(
                pane_index,
                position.price_scale,
                position.average_price,
            ) else {
                continue;
            };
            let hovered = feedback_hover.filter(|hit| {
                matches!(&hit.object, crate::TradingObjectId::Position(id) if id == &position.id)
            });
            let pressed = feedback_pressed.filter(|hit| {
                matches!(&hit.object, crate::TradingObjectId::Position(id) if id == &position.id)
            });
            let position_color = self.trading_position_color(position.side);
            lines.push(Prim::HLine {
                y: (y * vpr).round() as i32,
                x0: 0,
                x1: (self.trading_marker_end() * hpr).round() as i32,
                width: min_line_width,
                style: LineStyle::Solid,
                color: position_color,
            });
            if hovered.is_some() {
                self.push_trading_endpoint(lines, y, position_color, hpr, vpr);
            }
            let quantity = self.trading_position_quantity_text(position);
            let pnl = position.display_pnl.map_or_else(
                || "—".to_string(),
                |value| self.trading_pnl_text(value, position.currency.as_deref()),
            );
            let pnl_color = position.display_pnl.map_or(position_color, |value| {
                if value >= 0.0 {
                    self.trading_state.style.profit
                } else {
                    self.trading_state.style.risk
                }
            });
            let segments = [
                TradingControlSegment {
                    kind: TradingControlSegmentKind::Quantity,
                    text: quantity.as_str(),
                    width: self.trading_quantity_width(&quantity),
                    color: position_color,
                    filled: true,
                },
                // The PnL text keeps its profit/loss tint — the container around it is what
                // carries the position's direction color.
                TradingControlSegment {
                    kind: TradingControlSegmentKind::Pnl,
                    text: pnl.as_str(),
                    width: PNL_WIDTH,
                    color: pnl_color,
                    filled: false,
                },
                TradingControlSegment {
                    kind: TradingControlSegmentKind::Cancel,
                    text: "",
                    width: self.trading_close_width(),
                    color: position_color,
                    filled: false,
                },
            ];
            let cluster = TradingControlCluster {
                segments: &segments,
                left: self.trading_marker_start(),
                color: position_color,
            };
            let hovered_segment = hovered.and_then(|hit| Self::trading_control_kind(hit.kind));
            let pressed_segment = pressed.and_then(|hit| Self::trading_control_kind(hit.kind));
            self.push_trading_protection_buttons(
                lines,
                self.trading_position_protection_preview(position, OrderRole::TakeProfit)
                    .is_some(),
                self.trading_position_protection_preview(position, OrderRole::StopLoss)
                    .is_some(),
                hovered.map(|hit| hit.kind),
                pressed.map(|hit| hit.kind),
                TradingChipLayout {
                    x: self.trading_marker_start(),
                    y,
                    hpr,
                    vpr,
                },
            );
            self.push_trading_cluster(
                lines,
                points,
                &cluster,
                hovered_segment,
                pressed_segment,
                TradingChipLayout {
                    x: cluster.start(),
                    y,
                    hpr,
                    vpr,
                },
            );
            self.push_trading_annotations(lines, &position.annotations, y, hpr, vpr);
            if self.trading_state.tooltip_armed
                && hovered.is_some_and(|hit| hit.kind == crate::TradingHitKind::CancelButton)
            {
                tooltip = Some(TradingTooltip {
                    text: "Close position".to_string(),
                    layout: TradingChipLayout {
                        x: cluster.start() + cluster.width() - self.trading_close_width() / 2.0,
                        y,
                        hpr,
                        vpr,
                    },
                });
            }
        }

        for order in &self.trading_state.orders {
            if !self
                .trading_state
                .account_visible(order.account_id.as_ref())
            {
                continue;
            }
            if order.pane_index != pane_index {
                continue;
            }
            let display_price = self.trading_effective_order_price(order);
            let Some(y) =
                self.trading_price_coordinate(pane_index, order.price_scale, display_price)
            else {
                continue;
            };
            let preview = self.trading_order_preview(order);
            let creating_protection =
                self.trading_state
                    .interaction
                    .preview()
                    .is_some_and(|preview| {
                        matches!(
                            &preview.source,
                            crate::TradingPreviewSource::OrderStopLoss { order_id }
                                | crate::TradingPreviewSource::OrderTakeProfit { order_id }
                                if order_id == &order.id
                        )
                    });
            let base_color = trading_order_color(
                &self.trading_state.style,
                order.kind,
                order.side,
                order.role,
                order.status,
            );
            // Only a live drag dims the line; a released change is already applied, so nothing
            // lingers in a pending tint.
            let color = if preview.is_some() {
                Color::rgba(base_color.r(), base_color.g(), base_color.b(), 176)
            } else {
                base_color
            };
            let hovered = feedback_hover.filter(
                |hit| matches!(&hit.object, crate::TradingObjectId::Order(id) if id == &order.id),
            );
            let pressed = feedback_pressed.filter(
                |hit| matches!(&hit.object, crate::TradingObjectId::Order(id) if id == &order.id),
            );
            lines.push(Prim::HLine {
                y: (y * vpr).round() as i32,
                x0: 0,
                x1: (self.trading_marker_end() * hpr).round() as i32,
                // Hover is communicated by the dash pattern, not a thickness jump. Dash metrics
                // scale with stroke width in every executor, so keeping the hairline also keeps
                // the hover dashes compact and consistent across DPRs.
                width: min_line_width,
                style: if creating_protection {
                    LineStyle::Dashed
                } else if preview.is_some()
                    || matches!(
                        order.status,
                        OrderStatus::PendingSubmit
                            | OrderStatus::PendingModify
                            | OrderStatus::PendingCancel
                    )
                {
                    LineStyle::Dotted
                } else {
                    LineStyle::Solid
                },
                color,
            });
            if hovered.is_some() {
                self.push_trading_endpoint(lines, y, color, hpr, vpr);
            }
            let remaining = (order.quantity - order.filled_quantity).max(0.0);
            let quantity = self.trading_order_quantity_text(order);
            let kind = match order.kind {
                crate::OrderKind::Market => "Market",
                crate::OrderKind::Limit => "Limit",
                crate::OrderKind::Stop => "Stop",
                crate::OrderKind::StopLimit => "Stop Limit",
            };
            let descriptor = if preview.is_some() {
                kind.to_string()
            } else {
                format!(
                    "{} {kind}",
                    if order.side == OrderSide::Buy {
                        "Buy"
                    } else {
                        "Sell"
                    }
                )
            };
            let main_x = self.trading_marker_start();
            // A drag names its side ahead of the readout chip so the pointer never hides which way
            // the order goes. Release commits the modification directly — a host that wants a
            // confirmation step runs it around the emitted intent, not inside the chart.
            if preview.is_some() {
                self.push_trading_segment(
                    lines,
                    TradingControlSegment {
                        kind: TradingControlSegmentKind::Quantity,
                        text: if order.side == OrderSide::Buy {
                            "Buy"
                        } else {
                            "Sell"
                        },
                        width: 46.0,
                        color: base_color,
                        filled: true,
                    },
                    TradingControlFeedback::Idle,
                    TradingChipLayout {
                        x: main_x - 52.0,
                        y,
                        hpr,
                        vpr,
                    },
                );
            }
            let hovered_segment = hovered.and_then(|hit| Self::trading_control_kind(hit.kind));
            let pressed_segment = pressed.and_then(|hit| Self::trading_control_kind(hit.kind));
            let (detail_kind, detail_text, detail_color) = if order.role == OrderRole::Working {
                (
                    TradingControlSegmentKind::OrderType,
                    descriptor.clone(),
                    color,
                )
            } else {
                // The PnL text keeps its profit/loss tint; the container carries the order's
                // buy/sell color.
                let (pnl, pnl_color) = self
                    .trading_protection_pnl(order, display_price)
                    .unwrap_or_else(|| ("—".to_string(), color));
                (TradingControlSegmentKind::Pnl, pnl, pnl_color)
            };
            let segments = [
                TradingControlSegment {
                    kind: TradingControlSegmentKind::Quantity,
                    text: quantity.as_str(),
                    width: self.trading_quantity_width(&quantity),
                    color,
                    filled: true,
                },
                TradingControlSegment {
                    kind: detail_kind,
                    text: detail_text.as_str(),
                    width: Self::trading_order_detail_width(order),
                    color: detail_color,
                    filled: false,
                },
                TradingControlSegment {
                    kind: TradingControlSegmentKind::Cancel,
                    text: "",
                    width: self.trading_close_width(),
                    color,
                    filled: false,
                },
            ];
            let cluster = TradingControlCluster {
                segments: &segments,
                left: main_x,
                color,
            };
            self.push_trading_protection_buttons(
                lines,
                self.trading_order_protection_preview(order, OrderRole::TakeProfit)
                    .is_some(),
                self.trading_order_protection_preview(order, OrderRole::StopLoss)
                    .is_some(),
                hovered.map(|hit| hit.kind),
                pressed.map(|hit| hit.kind),
                TradingChipLayout {
                    x: self.trading_marker_start(),
                    y,
                    hpr,
                    vpr,
                },
            );
            self.push_trading_cluster(
                lines,
                points,
                &cluster,
                hovered_segment,
                pressed_segment,
                TradingChipLayout {
                    x: main_x,
                    y,
                    hpr,
                    vpr,
                },
            );
            self.push_trading_annotations(lines, &order.annotations, y, hpr, vpr);
            // The tooltip names exactly what the click does. An order's close control cancels the
            // unfilled remainder (any filled part already belongs to the position), and it only
            // acts on live orders, so pending or terminal orders get no action tooltip.
            let cancel_label = match (order.role, order.status) {
                (_, status)
                    if !matches!(status, OrderStatus::Working | OrderStatus::PartiallyFilled) =>
                {
                    None
                }
                (OrderRole::TakeProfit, _) => Some("Cancel take profit".to_string()),
                (OrderRole::StopLoss, _) => Some("Cancel stop loss".to_string()),
                (OrderRole::Working, OrderStatus::PartiallyFilled) => Some(format!(
                    "Cancel remaining {}",
                    self.format_trading_quantity(remaining)
                )),
                (OrderRole::Working, _) => Some("Cancel order".to_string()),
            };
            if let Some(text) = cancel_label.filter(|_| {
                self.trading_state.tooltip_armed
                    && hovered.is_some_and(|hit| hit.kind == crate::TradingHitKind::CancelButton)
            }) {
                tooltip = Some(TradingTooltip {
                    text,
                    layout: TradingChipLayout {
                        x: cluster.start() + cluster.width() - self.trading_close_width() / 2.0,
                        y,
                        hpr,
                        vpr,
                    },
                });
            }
            if order.kind == crate::OrderKind::StopLimit {
                if let Some(stop_price) = order.stop_price.filter(|price| *price != display_price) {
                    if let Some(stop_y) =
                        self.trading_price_coordinate(pane_index, order.price_scale, stop_price)
                    {
                        lines.push(Prim::HLine {
                            y: (stop_y * vpr).round() as i32,
                            x0: 0,
                            x1: (self.pane_w * hpr).round() as i32,
                            width: min_line_width,
                            style: LineStyle::Dotted,
                            color,
                        });
                        let trigger =
                            format!("Trigger {}", self.format_trading_quantity(remaining));
                        self.push_trading_segment(
                            lines,
                            TradingControlSegment {
                                kind: TradingControlSegmentKind::OrderType,
                                text: &trigger,
                                width: 92.0,
                                color,
                                filled: false,
                            },
                            TradingControlFeedback::Idle,
                            TradingChipLayout {
                                x: self.trading_marker_start(),
                                y: stop_y,
                                hpr,
                                vpr,
                            },
                        );
                    }
                }
            }
            self.push_host_trigger_line(
                lines,
                TradingTriggerLine {
                    pane_index,
                    price_scale: order.price_scale,
                    trigger_price: order.trailing_trigger_price,
                    display_price,
                    color: self.trading_state.style.pending,
                },
                hpr,
                vpr,
            );
            self.push_host_trigger_line(
                lines,
                TradingTriggerLine {
                    pane_index,
                    price_scale: order.price_scale,
                    trigger_price: order.break_even_trigger_price,
                    display_price,
                    color: self.trading_state.style.take_profit,
                },
                hpr,
                vpr,
            );
        }

        if let Some(preview) = self
            .trading_state
            .interaction
            .preview()
            .filter(|preview| preview.pane_index == pane_index)
        {
            if let Some(preview_y) =
                self.trading_price_coordinate(pane_index, preview.price_scale, preview.price)
            {
                let creating_protection =
                    !matches!(preview.source, crate::TradingPreviewSource::Order { .. });
                if creating_protection {
                    let semantic = if preview.role == OrderRole::TakeProfit {
                        self.trading_state.style.take_profit
                    } else {
                        self.trading_state.style.stop_loss
                    };
                    lines.push(Prim::HLine {
                        y: (preview_y * vpr).round() as i32,
                        x0: 0,
                        x1: (self.trading_marker_end() * hpr).round() as i32,
                        width: min_line_width,
                        style: LineStyle::Dotted,
                        color: semantic,
                    });
                    let quantity = self.format_trading_quantity(preview.quantity);
                    let role = if preview.role == OrderRole::TakeProfit {
                        "TP"
                    } else {
                        "SL"
                    };
                    let segments = [
                        TradingControlSegment {
                            kind: TradingControlSegmentKind::Quantity,
                            text: quantity.as_str(),
                            width: self.trading_quantity_width(&quantity),
                            color: semantic,
                            filled: true,
                        },
                        TradingControlSegment {
                            kind: TradingControlSegmentKind::OrderType,
                            text: role,
                            width: ORDER_TYPE_WIDTH,
                            color: semantic,
                            filled: false,
                        },
                    ];
                    let cluster = TradingControlCluster {
                        segments: &segments,
                        left: self.trading_marker_start(),
                        color: semantic,
                    };
                    self.push_trading_cluster(
                        lines,
                        points,
                        &cluster,
                        None,
                        None,
                        TradingChipLayout {
                            x: cluster.start(),
                            y: preview_y,
                            hpr,
                            vpr,
                        },
                    );
                }
                if let Some((anchor_price, long)) = self.trading_preview_relation(preview) {
                    if let Some(anchor_y) =
                        self.trading_price_coordinate(pane_index, preview.price_scale, anchor_price)
                    {
                        let valid = match (long, preview.role) {
                            (true, OrderRole::TakeProfit) => preview.price > anchor_price,
                            (true, OrderRole::StopLoss) => preview.price < anchor_price,
                            (false, OrderRole::TakeProfit) => preview.price < anchor_price,
                            (false, OrderRole::StopLoss) => preview.price > anchor_price,
                            (_, OrderRole::Working) => false,
                        };
                        if valid {
                            let top = anchor_y.min(preview_y).max(pane.top);
                            let bottom = anchor_y.max(preview_y).min(pane.top + pane.height);
                            if bottom > top {
                                let fill = if preview.role == OrderRole::TakeProfit {
                                    self.trading_state.style.profit
                                } else {
                                    self.trading_state.style.risk
                                };
                                regions.push(Prim::Rect {
                                    rect: IRect {
                                        x: 0,
                                        y: (top * vpr).round() as i32,
                                        w: width,
                                        h: ((bottom - top) * vpr).round().max(1.0) as i32,
                                    },
                                    color: Color::rgba(fill.r(), fill.g(), fill.b(), 32),
                                });
                            }
                        }
                        if (anchor_y - preview_y).abs() > 0.5 {
                            let connector = self.trading_state.style.position;
                            let x = ((self.pane_w - 8.0) * hpr).round() as i32;
                            lines.push(Prim::VLine {
                                x,
                                y0: (anchor_y.min(preview_y) * vpr).round() as i32,
                                y1: (anchor_y.max(preview_y) * vpr).round() as i32,
                                width: min_line_width,
                                style: LineStyle::Solid,
                                color: connector,
                            });
                            for cy in [anchor_y, preview_y] {
                                lines.push(Prim::Circle {
                                    cx: x as f32,
                                    cy: (cy * vpr) as f32,
                                    radius: (3.0 * vpr) as f32,
                                    fill: self.trading_chip_background(),
                                    stroke_width: (1.0 * vpr) as f32,
                                    stroke: connector,
                                });
                            }
                        }
                    }
                }
            }
        }

        if let TradingGroupVisualState::Active(group) = &self.trading_state.group_visual {
            let mut top = f64::INFINITY;
            let mut bottom = f64::NEG_INFINITY;
            let mut member_count = 0usize;
            let mut include = |y: f64| {
                top = top.min(y);
                bottom = bottom.max(y);
                member_count += 1;
            };
            for position in &self.trading_state.positions {
                if !self
                    .trading_state
                    .account_visible(position.account_id.as_ref())
                {
                    continue;
                }
                if position.pane_index == pane_index
                    && self.trading_group_contains_position(group, position)
                {
                    if let Some(y) = self.trading_price_coordinate(
                        pane_index,
                        position.price_scale,
                        position.average_price,
                    ) {
                        include(y);
                    }
                }
            }
            for order in &self.trading_state.orders {
                if !self
                    .trading_state
                    .account_visible(order.account_id.as_ref())
                {
                    continue;
                }
                if order.pane_index == pane_index && self.trading_group_contains_order(group, order)
                {
                    if let Some(y) = self.trading_price_coordinate(
                        pane_index,
                        order.price_scale,
                        self.trading_effective_order_price(order),
                    ) {
                        include(y);
                    }
                }
            }
            if member_count >= 2 && bottom - top > 0.5 {
                let connector_x = ((self.pane_w - 8.0) * hpr).round() as i32;
                let connector_color = self.trading_state.style.position;
                lines.push(Prim::VLine {
                    x: connector_x,
                    y0: (top * vpr).round() as i32,
                    y1: (bottom * vpr).round() as i32,
                    width: min_line_width,
                    style: LineStyle::Solid,
                    color: connector_color,
                });
                for position in &self.trading_state.positions {
                    if !self
                        .trading_state
                        .account_visible(position.account_id.as_ref())
                    {
                        continue;
                    }
                    if position.pane_index == pane_index
                        && self.trading_group_contains_position(group, position)
                    {
                        if let Some(y) = self.trading_price_coordinate(
                            pane_index,
                            position.price_scale,
                            position.average_price,
                        ) {
                            self.push_trading_endpoint(lines, y, connector_color, hpr, vpr);
                        }
                    }
                }
                for order in &self.trading_state.orders {
                    if !self
                        .trading_state
                        .account_visible(order.account_id.as_ref())
                    {
                        continue;
                    }
                    if order.pane_index == pane_index
                        && self.trading_group_contains_order(group, order)
                    {
                        if let Some(y) = self.trading_price_coordinate(
                            pane_index,
                            order.price_scale,
                            self.trading_effective_order_price(order),
                        ) {
                            self.push_trading_endpoint(lines, y, connector_color, hpr, vpr);
                        }
                    }
                }
            }
        }

        for round_trip in self
            .trading_state
            .round_trips
            .iter()
            .take(crate::MAX_TRADING_ROUND_TRIPS)
        {
            let Some(entry) = self
                .trading_state
                .executions
                .iter()
                .find(|execution| execution.id == round_trip.entry_execution_id)
            else {
                continue;
            };
            let Some(exit) = self
                .trading_state
                .executions
                .iter()
                .find(|execution| execution.id == round_trip.exit_execution_id)
            else {
                continue;
            };
            if !self
                .trading_state
                .account_visible(entry.account_id.as_ref())
                || !self.trading_state.account_visible(exit.account_id.as_ref())
                || !self.replay_time_is_visible(entry.time)
                || !self.replay_time_is_visible(exit.time)
                || entry.pane_index != pane_index
                || exit.pane_index != pane_index
                || self.data.merged_times().is_empty()
            {
                continue;
            }
            let Some(entry_logical) = self
                .axis_index_for_time(entry.time)
                .map(|index| index as i64)
            else {
                continue;
            };
            let Some(exit_logical) = self
                .axis_index_for_time(exit.time)
                .map(|index| index as i64)
            else {
                continue;
            };
            let entry_x = self.time_scale.index_to_coordinate(entry_logical);
            let exit_x = self.time_scale.index_to_coordinate(exit_logical);
            let Some(entry_y) =
                self.trading_price_coordinate(pane_index, entry.price_scale, entry.price)
            else {
                continue;
            };
            let Some(exit_y) =
                self.trading_price_coordinate(pane_index, exit.price_scale, exit.price)
            else {
                continue;
            };
            let color = match round_trip.outcome {
                crate::TradingRoundTripOutcome::Profit => self.trading_state.style.profit,
                crate::TradingRoundTripOutcome::Loss => self.trading_state.style.risk,
                crate::TradingRoundTripOutcome::Flat => self.trading_state.style.control,
            };
            let x0 = (entry_x * hpr).round() as i32;
            let x1 = (exit_x * hpr).round() as i32;
            lines.push(Prim::VLine {
                x: x0,
                y0: (entry_y.min(exit_y) * vpr).round() as i32,
                y1: (entry_y.max(exit_y) * vpr).round() as i32,
                width: min_line_width,
                style: LineStyle::Dotted,
                color,
            });
            lines.push(Prim::HLine {
                y: (exit_y * vpr).round() as i32,
                x0: x0.min(x1),
                x1: x0.max(x1),
                width: min_line_width,
                style: LineStyle::Dotted,
                color,
            });
            lines.push(Prim::Text {
                x: (((entry_x + exit_x) / 2.0) * hpr) as f32,
                y: (exit_y * vpr) as f32,
                text: round_trip.result_label.clone(),
                color,
                size: (self.options.get().layout.font_size * vpr) as f32,
                family: self.options.get().layout.font_family.clone(),
                align: TextAlign::Center,
                weight: 400,
                italic: false,
            });
        }
        self.push_trading_executions(lines, points, pane, pane_index, hpr, vpr);
        if let Some(tooltip) = tooltip {
            self.push_trading_tooltip(lines, &tooltip.text, pane, tooltip.layout);
        }
    }
}
