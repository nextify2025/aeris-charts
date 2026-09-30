//! Axis frame production: price/time labels, widths, marker/price-line/crosshair labels.
//!
//! All sizes come from the shared [`AxisMetrics`](crate::axis_metrics::AxisMetrics): axis text
//! at 11/12 of `layout.fontSize`, countdown text at 10/12, price chrome of 12 CSS px around
//! measured text, and an even-snapped time strip. Hosts measure axis strings at the axis size
//! (`measure`) and countdown strings at the countdown size (`countdown_measure`), each with
//! matching weight — advances are never derived by shrinking another size's measurement.

use super::*;
use crate::axis_metrics::{AxisMetrics, AXIS_FONT_SCALE, COUNTDOWN_FONT_SCALE};

/// A last-value label candidate before axis overlap resolution (reference IPriceAxisView state:
/// the source `coordinate` plus the render coordinate the overlap pass adjusts). `align`
/// is the owning scale's `alignLabels` — a scale with it off leaves its labels at their raw
/// coordinates (price-axis-widget.ts:633 early-return).
///
/// The candidate is an industry-standard CLUSTER of up to three independently-toggleable parts:
/// the title chip (a darker shade of the label color, left of the price text), the price text,
/// and a candle-close countdown row stacked below. `y`/`height` describe the whole cluster
/// (center + total height), which is what the overlap pass spaces; `top_height` is the title +
/// price row's share (`0` when both are hidden, e.g. a countdown-only cluster).
struct LastValueLabel {
    /// Price text; `None` when the series' `lastValueVisible` is off (the cluster can still
    /// render its title chip and/or countdown row).
    price_text: Option<String>,
    /// Title chip text (the series' `title`); `None` when unset or `title_visible` is off.
    title: Option<String>,
    /// Countdown row text; `None` when `countdown_visible` is off, the series has no usable
    /// bar interval, or no host clock is installed.
    countdown: Option<String>,
    /// The owning series' id — the attach group of this cluster's chips, so its price and
    /// countdown chips paint with a shared edge WITHOUT chaining into another series' cluster
    /// (a shared constant chained every cluster on the axis into one giant box).
    group_id: u32,
    y: f64,
    height: f64,
    top_height: f64,
    color: Color,
    align: bool,
    primary: bool,
    hollow: bool,
    /// The owning series is the chart's current selection — its chip carries the active accent.
    selected: bool,
}

#[derive(Clone, Copy)]
struct LivePriceRegion {
    pane_index: usize,
    target: PriceScaleTarget,
    top: f64,
    bottom: f64,
    primary: bool,
}

/// the public reference's active-chip indication: a bar in a lighter shade of the series color pinned to
/// the axis-facing edge of that series' last-value chip, painted over the chip it marks. Emitted
/// after the chip so it lands on top, and with no attach group so it cannot join the
/// price/countdown shared-edge chain.
#[allow(clippy::too_many_arguments)]
fn selected_chip_accent(
    chip_x: f64,
    chip_y: f64,
    chip_w: f64,
    chip_h: f64,
    color: Color,
    right_strip: bool,
    align: AxisTextAlign,
) -> AxisLabel {
    /// Accent thickness in media px.
    const ACCENT_W: f64 = 3.0;
    let x = if right_strip {
        chip_x + chip_w - ACCENT_W
    } else {
        chip_x
    };
    AxisLabel {
        text: String::new(),
        x,
        y: chip_y + chip_h / 2.0,
        color,
        align,
        midpoint: AxisTextMidpoint::Label,
        font_scale: AXIS_FONT_SCALE,
        bold: false,
        background: Some((x, chip_y, ACCENT_W, chip_h, color.lighten(0.45))),
        background_corners: AxisLabelCorners::NONE,
        measure_extra: 0.0,
        attach_group: None,
        border: None,
    }
}

/// Median of the last up-to-10 inter-bar deltas of a series' bar times (fallback: with a single
/// delta the median IS that delta); `None` with fewer than two usable bars, which hides the
/// countdown row. Non-positive deltas (duplicate times) are skipped.
pub(crate) fn median_bar_interval(times: &[i64]) -> Option<f64> {
    let tail = &times[times.len().saturating_sub(11)..];
    let mut deltas: Vec<i64> = tail
        .windows(2)
        .filter_map(|w| w[1].checked_sub(w[0]))
        .filter(|&d| d > 0)
        .collect();
    if deltas.is_empty() {
        return None;
    }
    deltas.sort_unstable();
    let n = deltas.len();
    Some(if n % 2 == 1 {
        deltas[n / 2] as f64
    } else {
        (deltas[n / 2 - 1] as f64 + deltas[n / 2] as f64) / 2.0
    })
}

/// the public reference countdown format by remaining magnitude: `mm:ss` zero-padded below an hour,
/// `hh:mm:ss` below a day, `"Xd Xh"` at a day and beyond.
pub(crate) fn format_countdown_remaining(remaining: f64) -> String {
    let secs = remaining.max(0.0).floor() as u64;
    if secs < 3600 {
        format!("{:02}:{:02}", secs / 60, secs % 60)
    } else if secs < 86400 {
        format!(
            "{:02}:{:02}:{:02}",
            secs / 3600,
            secs % 3600 / 60,
            secs % 60
        )
    } else {
        let days = secs / 86400;
        if days > 9999 {
            "9999d+".to_string()
        } else {
            format!("{}d {}h", days, secs % 86400 / 3600)
        }
    }
}

fn countdown_layout_key(remaining: f64) -> usize {
    let secs = remaining.max(0.0).floor() as u64;
    if secs < 3600 {
        1
    } else if secs < 86400 {
        2
    } else {
        let days = secs / 86400;
        if days > 9999 {
            3
        } else {
            let day_digits = if days == 0 {
                1
            } else {
                days.ilog10() as usize + 1
            };
            let hours = secs % 86400 / 3600;
            let hour_digits = if hours < 10 { 1 } else { 2 };
            100 + day_digits * 10 + hour_digits
        }
    }
}

/// Digit-stable countdown advance measured AT the countdown size (never an axis-size
/// measurement shrunk to fit): every ASCII digit is substituted so ticking digits cannot
/// outgrow the reservation. Countdown labels never render bold.
fn countdown_text_width<F>(text: &str, countdown_measure: &F) -> f64
where
    F: Fn(&str, bool) -> f64,
{
    let mut width = countdown_measure(text, false);
    for digit in '0'..='9' {
        let candidate: String = text
            .chars()
            .map(|character| {
                if character.is_ascii_digit() {
                    digit
                } else {
                    character
                }
            })
            .collect();
        width = width.max(countdown_measure(&candidate, false));
    }
    width
}

fn fit_axis_text<F>(text: &str, max_width: f64, measure: &F) -> Option<String>
where
    F: Fn(&str, bool) -> f64,
{
    if max_width <= 0.0 {
        return None;
    }
    if measure(text, false) <= max_width {
        return Some(text.to_string());
    }
    const ELLIPSIS: &str = "...";
    if measure(ELLIPSIS, false) > max_width {
        return None;
    }

    let boundaries: Vec<usize> = text.char_indices().map(|(index, _)| index).collect();
    let mut low = 0;
    let mut high = boundaries.len();
    while low < high {
        let middle = (low + high).div_ceil(2);
        let end = boundaries.get(middle).copied().unwrap_or(text.len());
        let candidate = format!("{}{}", &text[..end], ELLIPSIS);
        if measure(&candidate, false) <= max_width {
            low = middle;
        } else {
            high = middle - 1;
        }
    }
    let end = boundaries.get(low).copied().unwrap_or(text.len());
    Some(format!("{}{}", &text[..end], ELLIPSIS))
}

/// Minimal port of reference price-axis-widget.ts `_fixLabelOverlap` + `recalculateOverlapping`
/// for the last-value labels of one axis side: split the labels around the center label
/// (the first series', the reference's `centerSource`), clamp edge labels into the viewport, then push
/// overlapping labels apart outward from the center, shifting whole groups back when they
/// would fall off the scale. Only labels from `alignLabels` scales participate (reference gates
/// the whole pass on that option); the rest keep their raw coordinates. Fewer than two
/// aligned labels are left untouched.
fn resolve_last_value_label_overlap(labels: &mut [LastValueLabel], scale_height: f64) {
    let aligned: Vec<usize> = (0..labels.len()).filter(|&i| labels[i].align).collect();
    if aligned.len() < 2 {
        return;
    }
    let center = labels[aligned[0]].y;
    // Split around the center and sort each side toward it (reference sorts by the source
    // coordinate, so capture the order before any adjustment).
    let mut top: Vec<usize> = aligned
        .iter()
        .copied()
        .filter(|&i| labels[i].y <= center)
        .collect();
    top.sort_by(|&a, &b| labels[b].y.total_cmp(&labels[a].y)); // center-to-top
    let mut bottom: Vec<usize> = aligned
        .iter()
        .copied()
        .filter(|&i| labels[i].y > center)
        .collect();
    if !top.is_empty() && !bottom.is_empty() {
        bottom.push(top[0]); // share the center label between both passes
    }
    bottom.sort_by(|&a, &b| labels[a].y.total_cmp(&labels[b].y));
    // Edge clamp (price-axis-widget.ts:659-669): a label half-off the scale snaps fully inside.
    for &i in &aligned {
        let label = &mut labels[i];
        let half = (label.height / 2.0).floor();
        if label.y > -half && label.y < half {
            label.y = half;
        }
        if label.y > scale_height - half && label.y < scale_height + half {
            label.y = scale_height - half;
        }
    }
    recalculate_overlapping(labels, &top, 1.0, scale_height);
    recalculate_overlapping(labels, &bottom, -1.0, scale_height);
}

/// reference `recalculateOverlapping` (price-axis-widget.ts:77-121): walk the labels outward from
/// the center (`direction` 1 = toward the top, -1 = toward the bottom) and push each
/// overlapping label past its predecessor; when a pushed group would leave the viewport,
/// shift the whole group back by the space that was free before it.
fn recalculate_overlapping(
    labels: &mut [LastValueLabel],
    order: &[usize],
    direction: f64,
    scale_height: f64,
) {
    if order.is_empty() {
        return;
    }
    let first = order[0];
    let init_height = labels[first].height;
    let mut space_before_group = (if direction > 0.0 {
        scale_height / 2.0 - (labels[first].y - init_height / 2.0)
    } else {
        labels[first].y - init_height / 2.0 - scale_height / 2.0
    })
    .max(0.0);
    let mut group_start = 0usize;
    for i in 1..order.len() {
        let view = order[i];
        let prev = order[i - 1];
        let height = labels[prev].height;
        let overlap = if direction > 0.0 {
            labels[view].y > labels[prev].y - height
        } else {
            labels[view].y < labels[prev].y + height
        };
        if overlap {
            let render_y = labels[prev].y - height * direction;
            labels[view].y = render_y;
            let edge_point = render_y - direction * height / 2.0;
            let out_of_viewport = if direction > 0.0 {
                edge_point < 0.0
            } else {
                edge_point > scale_height
            };
            if out_of_viewport && space_before_group > 0.0 {
                let desired_shift = if direction > 0.0 {
                    -1.0 - edge_point
                } else {
                    edge_point - scale_height
                };
                let shift = desired_shift.min(space_before_group);
                for &k in &order[group_start..] {
                    labels[k].y += direction * shift;
                }
                space_before_group -= shift;
            }
        } else {
            group_start = i;
            space_before_group = if direction > 0.0 {
                labels[prev].y - height - labels[view].y
            } else {
                labels[view].y - (labels[prev].y + height)
            };
        }
    }
}

impl ChartEngine {
    fn chart_surface_color(&self) -> Color {
        let fallback = aeris_charts_core::style::DEFAULT_SURFACE_RGB;
        Color::parse_css(&self.options.get().layout.background.color)
            .unwrap_or(Color::rgb(fallback.0, fallback.1, fallback.2))
            .solid()
    }

    pub(crate) fn primary_text_color(&self) -> Color {
        let fallback = aeris_charts_core::style::DEFAULT_FOREGROUND_RGB;
        Color::parse_css(&self.options.get().layout.text_color)
            .unwrap_or(Color::rgb(fallback.0, fallback.1, fallback.2))
    }

    pub(super) fn axis_label_text_color(&self, background: Color) -> Color {
        let surface = self.chart_surface_color();
        background.contrast_text_over(surface)
    }

    /// Lowest-z-order source attached to a scale, matching the reference formatter owner.
    pub(crate) fn scale_formatter_source(
        &self,
        pane_index: usize,
        target: PriceScaleTarget,
    ) -> Option<&crate::SeriesEntry> {
        self.series_order
            .iter()
            .filter_map(|id| self.series_entry(*id))
            .find(|series| series.pane_index == pane_index && series_scale_target(series) == target)
    }

    #[cfg(test)]
    pub(crate) fn scale_tick_base(&self, pane_index: usize, target: PriceScaleTarget) -> i64 {
        let Some(scale) = self.price_scale_for(pane_index, target) else {
            return 100;
        };
        if matches!(
            scale.mode(),
            PriceScaleMode::Percentage | PriceScaleMode::IndexedTo100
        ) {
            return 100;
        }
        self.scale_formatter_source(pane_index, target)
            .map_or(100, |series| series.price_format.base())
    }

    /// One scale's tick marks on its price grid; the axis labels, the horizontal grid, and axis
    /// width negotiation all share this set. The grid is the formatter source's minimum move,
    /// or, with a tick-size ladder, the LCM of the band ticks over each price interval the
    /// builder spans (per mark on a log scale, so lower bands keep their finer grid).
    /// Percentage and indexed-to-100 scales use the reference's fixed base-100 grid (`0.01`).
    pub(crate) fn scale_tick_marks(
        &self,
        pane_index: usize,
        target: PriceScaleTarget,
        entire_text_only_margin: f64,
    ) -> Vec<aeris_charts_core::scale::price_scale_core::PriceMark> {
        const DEFAULT_MIN_MOVE: f64 = 0.01;
        let Some(scale) = self.price_scale_for(pane_index, target) else {
            return Vec::new();
        };
        let format = (!matches!(
            scale.mode(),
            PriceScaleMode::Percentage | PriceScaleMode::IndexedTo100
        ))
        .then(|| self.scale_formatter_source(pane_index, target))
        .flatten()
        .map(|series| &series.price_format);
        match format {
            Some(format) => scale.build_tick_marks_on_grid(
                |low, high| format.tick_grid(low, high),
                entire_text_only_margin,
            ),
            None => scale.build_tick_marks(DEFAULT_MIN_MOVE, entire_text_only_margin),
        }
    }

    pub(crate) fn scale_autoscale_min_move(
        &self,
        pane_index: usize,
        target: PriceScaleTarget,
    ) -> f64 {
        let Some(scale) = self.price_scale_for(pane_index, target) else {
            return 1.0;
        };
        if matches!(
            scale.mode(),
            PriceScaleMode::Percentage | PriceScaleMode::IndexedTo100
        ) {
            return 1.0;
        }
        self.scale_formatter_source(pane_index, target)
            .map_or(1.0, |series| series.price_format.min_move)
    }

    pub(super) fn format_scale_value(&self, scale: &PriceScaleCore, value: f64) -> String {
        if scale.mode() == PriceScaleMode::Percentage {
            // Percentage mode has its own formatter; the host price formatter does not apply here
            // (matching reference, where percentage display is independent of `priceFormatter`).
            return PercentageFormatter::default().format(value);
        }
        if scale.mode() == PriceScaleMode::IndexedTo100 {
            return Self::format_indexed_value(value);
        }
        if let Some(f) = &self.price_formatter_fn {
            if let Some(s) = f(value) {
                return s;
            }
        }
        self.price_formatter.format(value)
    }

    /// Format one value through a series' `priceFormat` (reference series.ts `_recreateFormatter`):
    /// volume = K/M/B suffixes, percent = `%` sign, price = precision/min_move decimals,
    /// custom = the installed host fn. Returns `None` when the format defers to the
    /// scale/chart-level resolution (a factory-default price format, or a custom format with
    /// no/declining fn).
    pub(crate) fn format_with_price_format(
        &self,
        format: &SeriesPriceFormat,
        value: f64,
    ) -> Option<String> {
        match format.kind {
            // reference custom: `formatTickmarks` defaults to mapping the formatter over the values,
            // so one per-value fn serves ticks and labels alike; a `None` return (the callback
            // threw at the boundary) defers to the built-in fallback.
            PriceFormatKind::Custom => format.formatter.as_ref().and_then(|f| f(value)),
            PriceFormatKind::Volume => Some(VolumeFormatter::new(format.precision).format(value)),
            // reference wires `PercentageFormatter(precision)` — passing the precision as the raw
            // price scale, which reads like an upstream quirk (precision 2 -> one decimal).
            // We treat precision as decimal digits (10^precision), matching the percentage
            // scale mode's output at the default precision 2.
            PriceFormatKind::Percent => Some(
                PercentageFormatter::with_price_scale(10i64.pow(format.precision)).format(value),
            ),
            PriceFormatKind::Price => {
                if let Some(ladder) = format.active_tick_ladder() {
                    // Each price rounds to its own band tick and prints with that band's
                    // precision.
                    Some(ladder.format(value))
                } else if format.is_reference_default() {
                    None
                } else {
                    Some(
                        PriceFormatter::from_precision(format.precision, format.min_move)
                            .format(value),
                    )
                }
            }
        }
    }

    /// reference IndexedTo100 labels: a fixed `PriceFormatter(100, 1)` (price-scale.ts
    /// `updateFormatter`) for ticks, crosshair, last-value, and price-line labels alike —
    /// independent of the series format and the localization price formatter.
    fn format_indexed_value(value: f64) -> String {
        PriceFormatter::new(100, 1.0).format(value)
    }

    /// A series' OWN format drives its last-value label, its price-line labels, and the
    /// crosshair price label when the series is the label source. the reference's
    /// `localization.priceFormatter` keeps precedence when installed (price-scale.ts
    /// `_formatValue` consults it before the scale/series formatter).
    pub(super) fn format_series_value(
        &self,
        series: &crate::SeriesEntry,
        scale: &PriceScaleCore,
        value: f64,
    ) -> String {
        if scale.mode() == PriceScaleMode::Percentage {
            return PercentageFormatter::default().format(value);
        }
        if scale.mode() == PriceScaleMode::IndexedTo100 {
            return Self::format_indexed_value(value);
        }
        if let Some(f) = &self.price_formatter_fn {
            if let Some(s) = f(value) {
                return s;
            }
        }
        self.format_with_price_format(&series.price_format, value)
            .unwrap_or_else(|| self.price_formatter.format(value))
    }

    /// A series value outside any scale context (tooltips): the host `priceFormatter`, then
    /// the series' own price format, then the chart's built-in price formatter.
    pub(crate) fn format_series_plain_value(
        &self,
        series: &crate::SeriesEntry,
        value: f64,
    ) -> String {
        if let Some(f) = &self.price_formatter_fn {
            if let Some(s) = f(value) {
                return s;
            }
        }
        self.format_with_price_format(&series.price_format, value)
            .unwrap_or_else(|| self.price_formatter.format(value))
    }

    /// Axis TICK label formatting: the format of the scale's primary source — the first
    /// non-overlay series bound to that scale (reference uses the scale's main source for
    /// ticks: price-scale.ts `updateFormatter` picks the lowest-zorder data source). A primary
    /// source with the factory-default price format defers to the chart-level/built-in
    /// formatter, exactly like before per-series formats existed.
    pub(super) fn format_tick_value(
        &self,
        pane_index: usize,
        target: PriceScaleTarget,
        scale: &PriceScaleCore,
        value: f64,
    ) -> String {
        if scale.mode() == PriceScaleMode::Percentage {
            return PercentageFormatter::default().format(value);
        }
        if scale.mode() == PriceScaleMode::IndexedTo100 {
            return Self::format_indexed_value(value);
        }
        let primary = self.scale_formatter_source(pane_index, target);
        if let Some(series) = primary {
            if let Some(s) = self.format_with_price_format(&series.price_format, value) {
                return s;
            }
        }
        self.format_scale_value(scale, value)
    }

    /// Crosshair time label, honoring a host `timeFormatter` when installed. Otherwise the
    /// engine's `localization.dateFormat` pattern with the locale month-name table (reference
    /// chart-options-defaults.ts:34-37), in exchange wall-clock time.
    pub(crate) fn format_crosshair_ts(&self, ts: i64) -> String {
        // `ts` is a bar's identity time; the text prints its label time.
        let ts = self.bar_label_time(ts);
        if let Some(s) = self.host_time_label(ts) {
            return s;
        }
        format_crosshair_time_in(
            ts,
            self.time_visible,
            self.seconds_visible,
            &self.date_format,
            &self.month_names,
            &self.exchange_time,
        )
    }

    /// The host `timeFormatter` text for a timestamp, when one is installed and answers. Every
    /// engine surface that prints a point in time consults this first.
    pub(crate) fn host_time_label(&self, ts: i64) -> Option<String> {
        self.time_formatter_fn.as_ref().and_then(|f| f(ts))
    }

    /// Aeris extension: bold round-figure price tick labels (the public reference decile rule). Uniform
    /// ticks: the value is a multiple of `step × 10`. Non-uniform (log-style) ticks: the value
    /// is an exact power of ten.
    pub(crate) fn bold_round_decisions(logicals: &[f64], enabled: bool) -> Vec<bool> {
        if !enabled || logicals.is_empty() {
            return vec![false; logicals.len()];
        }
        let step = logicals
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(f64::INFINITY, f64::min);
        let uniform = step.is_finite()
            && step > 0.0
            && logicals
                .windows(2)
                .all(|w| ((w[1] - w[0]).abs() - step).abs() <= step * 1e-6 + 1e-12);
        logicals
            .iter()
            .map(|&v| {
                if uniform {
                    let ratio = v / step;
                    let nearest = ratio.round();
                    (ratio - nearest).abs() < 1e-6 && (nearest as i64) % 10 == 0
                } else {
                    v != 0.0 && {
                        let log = v.abs().log10();
                        (log.round() - log).abs() < 1e-9
                    }
                }
            })
            .collect()
    }

    /// Bold decisions for built tick marks. Boundary marks from `ensure_edge_tick_marks_visible`
    /// sit off the uniform span: they are never bold and never break the regular marks' uniform
    /// step detection.
    fn bold_round_mark_decisions(
        marks: &[aeris_charts_core::scale::price_scale_core::PriceMark],
        enabled: bool,
    ) -> Vec<bool> {
        let regular: Vec<f64> = marks
            .iter()
            .filter(|mark| !mark.edge)
            .map(|mark| mark.logical)
            .collect();
        let mut regular_bold = Self::bold_round_decisions(&regular, enabled).into_iter();
        marks
            .iter()
            .map(|mark| !mark.edge && regular_bold.next().unwrap_or(false))
            .collect()
    }

    /// Build backend-neutral axis label decisions. The host supplies only font measurement —
    /// axis strings at the resolved axis size (`measure`) and countdown strings at the
    /// countdown size (`countdown_measure`), each with matching weight; all visible ranges,
    /// scale choices, snapping, formatting, and label positions come from the engine so
    /// Canvas2D, WebGPU text, and native glyph backends share one layout result.
    pub fn build_axis_frame<F, G>(
        &mut self,
        max_label_width: f64,
        measure: F,
        countdown_measure: G,
    ) -> AxisFrame
    where
        F: Fn(&str, bool) -> f64,
        G: Fn(&str, bool) -> f64,
    {
        self.sync_frame_input_invalidation();
        let frame = self.build_axis_frame_impl(max_label_width, measure, countdown_measure, true);
        self.retained_frame.axis_generation = self.frame_invalidation.axis;
        frame
    }

    /// `include_transient` gates the crosshair labels: they paint per frame, but the axis-width
    /// negotiation must never see them — a wide hovered price would inflate the strip and the
    /// grow-fast/shrink-lazy policy would pin that width forever.
    fn build_axis_frame_impl<F, G>(
        &mut self,
        max_label_width: f64,
        measure: F,
        countdown_measure: G,
        include_transient: bool,
    ) -> AxisFrame
    where
        F: Fn(&str, bool) -> f64,
        G: Fn(&str, bool) -> f64,
    {
        let uninitialized = !self.retained_frame.initialized;
        if uninitialized || self.retained_frame.layout_generation != self.frame_invalidation.layout
        {
            self.layout_for_frame();
            self.retained_frame.layout_generation = self.frame_invalidation.layout;
            self.frame_build_stats.layout_rebuilds += 1;
        }
        if uninitialized
            || self.retained_frame.autoscale_generation != self.frame_invalidation.autoscale
        {
            self.autoscale_visible();
        }
        self.retained_frame.initialized = true;
        let mut out = AxisFrame {
            separator_hover: self.separator_hover,
            ..AxisFrame::default()
        };
        let visible = self.visible_range_for_frame();
        let layout_text_color = self.primary_text_color();
        // Per-scale label color (reference `textColor`): the scale's own color when set, else the
        // layout text color (price-axis-widget.ts:569).
        let scale_text_color = |scale: &PriceScaleCore| {
            scale
                .options()
                .text_color
                .as_deref()
                .and_then(Color::parse_css)
                .unwrap_or(layout_text_color)
        };
        let metrics = self.axis_metrics();
        for (pi, pane) in self.panes.iter().enumerate() {
            for target in pane.scale_targets() {
                let Some((side, strip_x, strip_width)) = self.price_scale_axis_geometry(pi, target)
                else {
                    continue;
                };
                let Some(scale) = pane.scale(target) else {
                    continue;
                };
                // reference `entireTextOnly`: corner marks shift in by half the font height so no
                // label text is clipped (price-tick-mark-builder.ts:71). The scale's tick font
                // is engine-synced to the resolved axis size, so this margin tracks it.
                let entire_margin = if scale.options().entire_text_only {
                    scale.options().font_size / 2.0
                } else {
                    0.0
                };
                let text_color = scale_text_color(scale);
                let ticks_visible = scale.options().ticks_visible;
                let marks = self.scale_tick_marks(pi, target, entire_margin);
                let bold_round =
                    Self::bold_round_mark_decisions(&marks, scale.options().bold_round_labels);
                for (mark, bold) in marks.iter().zip(bold_round) {
                    let y = mark.coord;
                    // A tick coordinate is the glyph center, not its visible bounds. Every pane
                    // edge leaves room for the complete line box plus one CSS pixel of clearance,
                    // even when entireTextOnly is disabled: a price label never spills into the
                    // time axis, a neighbouring pane, or past the chart top.
                    let edge_inset = metrics.axis / 2.0 + 1.0;
                    let top = pane.top + edge_inset;
                    let bottom = pane.top + pane.height - edge_inset;
                    if y >= top && y <= bottom {
                        if ticks_visible {
                            let left = side == PriceScaleSide::Left;
                            out.price_ticks.push(PriceAxisTick {
                                y,
                                x: if left {
                                    strip_x + strip_width - AxisMetrics::TICK_LENGTH
                                } else {
                                    strip_x
                                },
                                left,
                            });
                        }
                        out.labels.push(AxisLabel {
                            text: self.format_tick_value(pi, target, scale, mark.logical),
                            x: if side == PriceScaleSide::Left {
                                (strip_x + strip_width - AxisMetrics::PRICE_TEXT_INSET).max(0.0)
                            } else {
                                strip_x + AxisMetrics::PRICE_TEXT_INSET
                            },
                            y,
                            color: text_color,
                            align: if side == PriceScaleSide::Left {
                                AxisTextAlign::Right
                            } else {
                                AxisTextAlign::Left
                            },
                            midpoint: AxisTextMidpoint::Label,
                            font_scale: AXIS_FONT_SCALE,
                            bold,
                            background: None,
                            background_corners: AxisLabelCorners::NONE,
                            measure_extra: 0.0,
                            attach_group: None,
                            border: None,
                        });
                    }
                }
                if matches!(target, PriceScaleTarget::Named(_)) {
                    let border_css = if side == PriceScaleSide::Left {
                        &self.options.get().left_price_scale.border_color
                    } else {
                        &self.options.get().right_price_scale.border_color
                    };
                    let border = Color::parse_css(border_css).unwrap_or(GRID);
                    // The brand border width, snapped like the main axis borders.
                    let border_width =
                        aeris_charts_core::style::border_width_device_px(self.dpr) / self.dpr;
                    out.bands.push(AxisBand {
                        x: if side == PriceScaleSide::Left {
                            strip_x + strip_width - border_width
                        } else {
                            strip_x
                        },
                        y: pane.top,
                        width: border_width,
                        height: pane.height,
                        color: border,
                    });
                }
            }
        }
        let explicit_marks = self.resolved_time_tick_marks();
        if let (Some(range), Some(explicit)) = (visible, explicit_marks.as_deref()) {
            if self.time_axis_visible {
                self.append_explicit_time_labels(&mut out, explicit, range, &measure);
            }
            self.append_native_vertical_line_labels(&mut out.labels, &measure);
        } else if let Some((from, to)) = visible {
            let time_marks = self.time_marks(max_label_width);
            let maximum_weight = time_marks
                .iter()
                .map(|(_, weight)| *weight)
                .max()
                .unwrap_or(0);
            for &(index, weight) in &time_marks {
                if index < from || index > to {
                    continue;
                }
                let Some(ts) = self.axis_time_key_at(index as usize) else {
                    continue;
                };
                // A hidden time axis (reference `timeScale.visible` false) drops its whole strip,
                // tick labels and tick stubs included.
                if !self.time_axis_visible {
                    continue;
                }
                let x = self.pane_left + self.time_scale.index_to_coordinate(index);
                if self.time_ticks_visible {
                    out.time_ticks.push(x);
                }
                let kind =
                    weight_to_tick_mark_type(weight, self.time_visible, self.seconds_visible);
                // The tick sits on the bar with this identity time and prints its label time.
                let printed = self.bar_label_time(ts);
                let custom_text = self
                    .tick_mark_formatter_fn
                    .as_ref()
                    .and_then(|formatter| formatter(printed, kind as u8));
                let built_in = custom_text.is_none();
                let text = custom_text.unwrap_or_else(|| {
                    format_tick_label_in(printed, kind, &self.month_names, &self.exchange_time)
                });
                if kind == TickMarkType::Year
                    && built_in
                    && text.chars().count() > self.tick_mark_max_character_length as usize
                {
                    continue;
                }
                out.labels.push(AxisLabel {
                    text,
                    x,
                    y: self.pane_h + metrics.time_text_dy(),
                    color: layout_text_color,
                    align: AxisTextAlign::Center,
                    midpoint: AxisTextMidpoint::None,
                    font_scale: AXIS_FONT_SCALE,
                    // reference `timeScale.allowBoldLabels` (default true): bold major labels.
                    bold: self.time_scale.options().allow_bold_labels && weight >= maximum_weight,
                    background: None,
                    background_corners: AxisLabelCorners::NONE,
                    measure_extra: 0.0,
                    attach_group: None,
                    border: None,
                });
            }
            self.append_native_vertical_line_labels(&mut out.labels, &measure);
        }
        self.append_rectangle_drawing_axis_views(&mut out, &measure);
        self.append_position_drawing_axis_views(&mut out, &measure);
        self.append_price_line_labels(&mut out.labels, &measure);
        self.append_drawing_line_labels(&mut out.labels, &measure);
        let last_value_start = out.labels.len();
        let live_price_regions =
            self.append_last_value_label(&mut out.labels, &measure, &countdown_measure);
        let mut action_labels = Vec::new();
        self.append_action_axis_labels(&mut action_labels, &live_price_regions, &measure);
        out.labels
            .splice(last_value_start..last_value_start, action_labels);
        if include_transient {
            self.append_crosshair_labels(&mut out.labels, &measure);
            self.append_alert_create_chip(&mut out);
        }
        self.append_general_axis_frame(&mut out, &measure);
        out.separators = self
            .panes
            .iter()
            .skip(1)
            .map(|p| p.top - PANE_SEPARATOR)
            .collect();
        out
    }

    fn append_rectangle_drawing_axis_views<F>(&self, out: &mut AxisFrame, measure: &F)
    where
        F: Fn(&str, bool) -> f64,
    {
        for drawing in &self.drawings {
            if drawing.kind == DrawingKind::Rectangle && drawing.points.len() == 2 {
                self.append_rectangle_axis_view(
                    drawing,
                    &drawing.points,
                    self.selected_drawing == Some(drawing.id),
                    false,
                    out,
                    measure,
                );
            }
        }
        let Some(pending) = self.pending_drawing() else {
            return;
        };
        if pending.drawing.kind != DrawingKind::Rectangle {
            return;
        }
        let mut points = pending.drawing.points.clone();
        if points.len() < 2 {
            if let Some(preview) = pending.preview {
                points.push(preview);
            }
        }
        if points.len() == 2 {
            self.append_rectangle_axis_view(&pending.drawing, &points, true, true, out, measure);
        }
    }

    fn append_rectangle_axis_view<F>(
        &self,
        drawing: &crate::Drawing,
        points: &[crate::DrawingPoint],
        active: bool,
        preview: bool,
        out: &mut AxisFrame,
        measure: &F,
    ) where
        F: Fn(&str, bool) -> f64,
    {
        let Some(pane) = self.panes.get(drawing.pane_index) else {
            return;
        };
        let (Some(first), Some(second)) = (
            self.drawing_to_px_for(drawing.pane_index, drawing.price_scale, points[0]),
            self.drawing_to_px_for(drawing.pane_index, drawing.price_scale, points[1]),
        ) else {
            return;
        };
        let stroke = Color::parse_css(&drawing.color).unwrap_or(PRIMARY);
        let fill = if preview {
            drawing
                .preview_fill_color
                .as_deref()
                .or(drawing.fill_color.as_deref())
        } else {
            drawing.fill_color.as_deref()
        }
        .and_then(Color::parse_css)
        .unwrap_or(Color::rgba(stroke.r(), stroke.g(), stroke.b(), 51));
        let band_color = Color::rgba(
            fill.r(),
            fill.g(),
            fill.b(),
            u16::from(fill.a()).div_ceil(2) as u8,
        );
        let active_band_color = Color::rgba(PRIMARY.r(), PRIMARY.g(), PRIMARY.b(), 64);

        if active || drawing.axis_bands_visible {
            let top = first.1.min(second.1).max(pane.top);
            let bottom = first.1.max(second.1).min(pane.top + pane.height);
            if bottom >= top {
                let left_visible = self.options.get().left_price_scale.visible
                    && self.left_axis_w > 0.0
                    && matches!(drawing.price_scale, crate::DrawingPriceScale::Left);
                let left_visible = left_visible
                    || (!active
                        && self.options.get().left_price_scale.visible
                        && self.left_axis_w > 0.0
                        && drawing.price_scale == crate::DrawingPriceScale::Overlay);
                let right_visible = self.options.get().right_price_scale.visible
                    && self.axis_w > 0.0
                    && matches!(
                        drawing.price_scale,
                        crate::DrawingPriceScale::Right | crate::DrawingPriceScale::Overlay
                    );
                if left_visible {
                    out.bands.push(AxisBand {
                        x: self.pane_left - self.left_axis_w.min(15.0),
                        y: top,
                        width: self.left_axis_w.min(15.0),
                        height: (bottom - top).max(1.0 / self.dpr),
                        color: if active {
                            active_band_color
                        } else {
                            band_color
                        },
                    });
                }
                if right_visible {
                    out.bands.push(AxisBand {
                        x: self.pane_left + self.pane_w,
                        y: top,
                        width: self.axis_w.min(15.0),
                        height: (bottom - top).max(1.0 / self.dpr),
                        color: if active {
                            active_band_color
                        } else {
                            band_color
                        },
                    });
                }
            }
            if drawing.axis_bands_visible && self.time_axis_visible {
                let left = first.0.min(second.0).max(0.0);
                let right = first.0.max(second.0).min(self.pane_w);
                if right >= left {
                    out.bands.push(AxisBand {
                        x: self.pane_left + left,
                        y: self.pane_h,
                        width: (right - left).max(1.0 / self.dpr),
                        height: self.time_axis_height().min(15.0),
                        color: band_color,
                    });
                }
            }
        }

        if !active && !drawing.show_labels {
            return;
        }
        let drawing_label_background = drawing
            .label_color
            .as_deref()
            .and_then(Color::parse_css)
            .unwrap_or(stroke);
        let drawing_label_text = drawing
            .label_text_color
            .as_deref()
            .and_then(Color::parse_css)
            .unwrap_or_else(|| self.axis_label_text_color(drawing_label_background));
        let (price_label_background, price_label_text) = if active {
            (PRIMARY, self.axis_label_text_color(PRIMARY))
        } else {
            (drawing_label_background, drawing_label_text)
        };
        let metrics = self.axis_metrics();
        let target = match drawing.price_scale {
            crate::DrawingPriceScale::Right => PriceScaleTarget::Right,
            crate::DrawingPriceScale::Left => PriceScaleTarget::Left,
            crate::DrawingPriceScale::Overlay => PriceScaleTarget::Overlay,
        };
        let Some(scale) = self.price_scale_for(drawing.pane_index, target) else {
            return;
        };
        let base = self.drawing_scale_base_for(drawing.pane_index, drawing.price_scale);
        for left_side in [true, false] {
            let visible = if left_side {
                self.options.get().left_price_scale.visible
                    && matches!(drawing.price_scale, crate::DrawingPriceScale::Left)
                    || (!active
                        && self.options.get().left_price_scale.visible
                        && drawing.price_scale == crate::DrawingPriceScale::Overlay)
            } else {
                self.options.get().right_price_scale.visible
                    && matches!(
                        drawing.price_scale,
                        crate::DrawingPriceScale::Right | crate::DrawingPriceScale::Overlay
                    )
            };
            if !visible {
                continue;
            }
            for (point, (_, y)) in points.iter().zip([first, second]) {
                if y < pane.top || y > pane.top + pane.height {
                    continue;
                }
                let logical_price = scale.price_to_logical_value(point.price, base);
                let text = self.format_tick_value(drawing.pane_index, target, scale, logical_price);
                let width = AxisMetrics::price_tag_width(measure(&text, false));
                let height = metrics.price_tag_height();
                let (x, align, background_x) = if left_side {
                    (
                        self.pane_left - AxisMetrics::PRICE_TEXT_INSET,
                        AxisTextAlign::Right,
                        self.pane_left - width,
                    )
                } else {
                    (
                        self.pane_left + self.pane_w + AxisMetrics::PRICE_TEXT_INSET,
                        AxisTextAlign::Left,
                        self.pane_left + self.pane_w,
                    )
                };
                out.labels.push(AxisLabel {
                    text,
                    x,
                    y,
                    color: price_label_text,
                    align,
                    midpoint: AxisTextMidpoint::Label,
                    font_scale: AXIS_FONT_SCALE,
                    bold: false,
                    background: Some((
                        background_x,
                        y - height / 2.0,
                        width,
                        height,
                        price_label_background,
                    )),
                    background_corners: AxisLabelCorners::for_align(align),
                    measure_extra: 0.0,
                    attach_group: None,
                    border: None,
                });
            }
        }
        if drawing.show_labels && self.time_axis_visible {
            for (point, (x, _)) in points.iter().zip([first, second]) {
                if x < 0.0 || x > self.pane_w {
                    continue;
                }
                let logical = point.logical.round();
                // Anchors beyond the data show their extrapolated anchor time.
                let Some(time) = usize::try_from(logical as i64)
                    .ok()
                    .and_then(|index| self.axis_time_key_at(index))
                    .or_else(|| {
                        self.anchor_time_at_logical(logical)
                            .map(|time| time.floor() as i64)
                    })
                else {
                    continue;
                };
                let printed = self.bar_label_time(time);
                let text = self.host_time_label(printed).unwrap_or_else(|| {
                    format_date_pattern(
                        self.exchange_time.local_seconds(printed),
                        "M/d/yyyy",
                        &self.month_names,
                    )
                });
                let width = AxisMetrics::time_tag_width(measure(&text, false));
                let height = metrics.time_strip_height();
                let chart_x = self.pane_left + x;
                let box_x = (chart_x - width / 2.0).clamp(
                    self.pane_left,
                    (self.pane_left + self.pane_w - width).max(self.pane_left),
                );
                out.labels.push(AxisLabel {
                    text,
                    x: box_x + width / 2.0,
                    y: self.pane_h + metrics.time_text_dy(),
                    color: drawing_label_text,
                    align: AxisTextAlign::Center,
                    midpoint: AxisTextMidpoint::None,
                    font_scale: AXIS_FONT_SCALE,
                    bold: false,
                    background: Some((box_x, self.pane_h, width, height, drawing_label_background)),
                    background_corners: AxisLabelCorners::BOTTOM,
                    measure_extra: 0.0,
                    attach_group: None,
                    border: None,
                });
            }
        }
    }

    fn append_position_drawing_axis_views<F>(&self, out: &mut AxisFrame, measure: &F)
    where
        F: Fn(&str, bool) -> f64,
    {
        for drawing in &self.drawings {
            if matches!(
                drawing.kind,
                DrawingKind::LongPosition | DrawingKind::ShortPosition
            ) && drawing.points.len() == 3
            {
                self.append_position_axis_view(drawing, &drawing.points, out, measure);
            }
        }

        let Some(pending) = self.pending_drawing() else {
            return;
        };
        if !matches!(
            pending.drawing.kind,
            DrawingKind::LongPosition | DrawingKind::ShortPosition
        ) {
            return;
        }
        let mut points = pending.drawing.points.clone();
        if points.len() < 3 {
            if let Some(preview) = pending.preview {
                points.push(preview);
            }
        }
        if points.len() == 3 {
            self.append_position_axis_view(&pending.drawing, &points, out, measure);
        }
    }

    fn append_position_axis_view<F>(
        &self,
        drawing: &crate::Drawing,
        points: &[crate::DrawingPoint],
        out: &mut AxisFrame,
        measure: &F,
    ) where
        F: Fn(&str, bool) -> f64,
    {
        let Some(pane) = self.panes.get(drawing.pane_index) else {
            return;
        };
        if points.len() != 3 {
            return;
        }
        let target = match drawing.price_scale {
            crate::DrawingPriceScale::Right => PriceScaleTarget::Right,
            crate::DrawingPriceScale::Left => PriceScaleTarget::Left,
            crate::DrawingPriceScale::Overlay => PriceScaleTarget::Overlay,
        };
        let Some(scale) = self.price_scale_for(drawing.pane_index, target) else {
            return;
        };
        let base = self.drawing_scale_base_for(drawing.pane_index, drawing.price_scale);
        let metrics = self.axis_metrics();
        let reward = Color::parse_css(aeris_charts_core::style::MARKET_UP_CSS)
            .unwrap_or(Color::rgb(8, 153, 129))
            .solid();
        let risk = Color::parse_css(aeris_charts_core::style::MARKET_DOWN_CSS)
            .unwrap_or(Color::rgb(247, 82, 95))
            .solid();
        let colors = [POSITION_ENTRY, reward, risk];

        let left_side = matches!(drawing.price_scale, crate::DrawingPriceScale::Left);
        let visible = if left_side {
            self.options.get().left_price_scale.visible
        } else {
            self.options.get().right_price_scale.visible
        };
        if !visible {
            return;
        }

        for (point, background) in points.iter().zip(colors) {
            let Some((_, y)) =
                self.drawing_to_px_for(drawing.pane_index, drawing.price_scale, *point)
            else {
                continue;
            };
            if y < pane.top || y > pane.top + pane.height {
                continue;
            }
            let logical_price = scale.price_to_logical_value(point.price, base);
            let text = self.format_tick_value(drawing.pane_index, target, scale, logical_price);
            let width = AxisMetrics::price_tag_width(measure(&text, false));
            let height = metrics.price_tag_height();
            let (x, align, background_x) = if left_side {
                (
                    self.pane_left - AxisMetrics::PRICE_TEXT_INSET,
                    AxisTextAlign::Right,
                    self.pane_left - width,
                )
            } else {
                (
                    self.pane_left + self.pane_w + AxisMetrics::PRICE_TEXT_INSET,
                    AxisTextAlign::Left,
                    self.pane_left + self.pane_w,
                )
            };
            out.labels.push(AxisLabel {
                text,
                x,
                y,
                color: self.axis_label_text_color(background),
                align,
                midpoint: AxisTextMidpoint::Label,
                font_scale: AXIS_FONT_SCALE,
                bold: false,
                background: Some((background_x, y - height / 2.0, width, height, background)),
                background_corners: AxisLabelCorners::for_align(align),
                measure_extra: 0.0,
                attach_group: None,
                border: None,
            });
        }
    }

    fn append_native_vertical_line_labels<F>(&self, labels: &mut Vec<AxisLabel>, measure: &F)
    where
        F: Fn(&str, bool) -> f64,
    {
        if !self.time_axis_visible {
            return;
        }
        let metrics = self.axis_metrics();
        for series in &self.series {
            if !series.visible || series.removed {
                continue;
            }
            for primitive in &series.native_primitives {
                let crate::native_primitives::NativeSeriesPrimitiveKind::VerticalLine {
                    time,
                    options,
                } = &primitive.kind
                else {
                    continue;
                };
                if !options.show_label {
                    continue;
                }
                let Some(x) = self.time_to_coordinate(*time as f64) else {
                    continue;
                };
                if x < 0.0 || x > self.pane_w {
                    continue;
                }
                let width = AxisMetrics::time_tag_width(measure(&options.label_text, false));
                let height = metrics.time_strip_height();
                let x = self.pane_left + x;
                let box_x = (x - width / 2.0).clamp(
                    self.pane_left,
                    (self.pane_left + self.pane_w - width).max(self.pane_left),
                );
                labels.push(AxisLabel {
                    text: options.label_text.clone(),
                    x: box_x + width / 2.0,
                    y: self.pane_h + metrics.time_text_dy(),
                    color: options.label_text_color.unwrap_or_else(|| {
                        self.axis_label_text_color(options.label_background_color)
                    }),
                    align: AxisTextAlign::Center,
                    midpoint: AxisTextMidpoint::None,
                    font_scale: AXIS_FONT_SCALE,
                    bold: false,
                    background: Some((
                        box_x,
                        self.pane_h,
                        width,
                        height,
                        options.label_background_color,
                    )),
                    background_corners: AxisLabelCorners::BOTTOM,
                    measure_extra: 0.0,
                    attach_group: None,
                    border: None,
                });
            }
        }
    }

    /// reference-compatible right-axis width negotiated from engine-formatted labels and host glyph
    /// measurement. The host contributes font metrics only, measured at the resolved axis and
    /// countdown sizes; label selection and formatting stay headless. The result is snapped to
    /// an even media-pixel width.
    pub fn optimal_price_axis_width<F, G>(&mut self, measure: F, countdown_measure: G) -> f64
    where
        F: Fn(&str, bool) -> f64,
        G: Fn(&str, bool) -> f64,
    {
        self.optimal_price_axis_width_for(PriceScaleTarget::Right, measure, countdown_measure)
    }

    /// Measure one visible side independently. Overlay scales deliberately share no axis strip.
    /// Chrome follows the shared metrics (canonical border width, 3 px tick allowance, 4 px padding per
    /// side) instead of the reference's wider `optimalWidth` paddings.
    pub fn optimal_price_axis_width_for<F, G>(
        &mut self,
        target: PriceScaleTarget,
        measure: F,
        countdown_measure: G,
    ) -> f64
    where
        F: Fn(&str, bool) -> f64,
        G: Fn(&str, bool) -> f64,
    {
        let frame = self.build_axis_frame_impl(80.0, &measure, &countdown_measure, false);
        let target_side = self
            .panes
            .iter()
            .find_map(|pane| pane.scale_side(target))
            .unwrap_or(PriceScaleSide::Right);
        let wanted_align = if target_side == PriceScaleSide::Left {
            AxisTextAlign::Right
        } else {
            AxisTextAlign::Left
        };
        let mut max_text_width = frame
            .labels
            .iter()
            .filter(|label| label.align == wanted_align)
            .map(|label| {
                // Advances are true host measurements at the rendered size: axis strings at
                // the axis size (with matching weight), countdown strings at the countdown
                // size. No shrink factors.
                if label.font_scale == COUNTDOWN_FONT_SCALE {
                    countdown_text_width(&label.text, &countdown_measure) + label.measure_extra
                } else {
                    measure(&label.text, label.bold) + label.measure_extra
                }
            })
            .fold(0.0_f64, f64::max);
        // reference optimalWidth reserves room for the crosshair label via a STATIC worst-case
        // sample (never the live label): the top/bottom prices snapped outward with a
        // 0.11111111111111 fractional tail, formatted by the label source's own formatter.
        if self.crosshair_mode != CrosshairMode::Hidden
            && self.options.get().crosshair.horz_line.label_visible
        {
            for (pi, pane) in self.panes.iter().enumerate() {
                let Some(scale) = pane.scale(target) else {
                    continue;
                };
                if scale.is_empty() {
                    continue;
                }
                let Some((from, _)) = self.visible_range_for_frame() else {
                    continue;
                };
                let series = self.series.iter().find(|series| {
                    series.pane_index == pi && series.price_scale_target == target && series.visible
                });
                let Some(base_value) =
                    series.and_then(|series| self.series_base_value(series.id, from))
                else {
                    continue;
                };
                let top_value = scale.coordinate_to_price(1.0, base_value);
                let bottom_value = scale.coordinate_to_price(pane.height - 2.0, base_value);
                let low = top_value.min(bottom_value).floor() + 0.111_111_111_111_11;
                let high = top_value.max(bottom_value).ceil() - 0.111_111_111_111_11;
                for sample in [low, high] {
                    let text = match series {
                        Some(series) => self.format_series_value(
                            series,
                            scale,
                            scale.price_to_logical_value(sample, base_value),
                        ),
                        None => self.format_scale_value(
                            scale,
                            scale.price_to_logical_value(sample, base_value),
                        ),
                    };
                    max_text_width = max_text_width.max(measure(&text, false));
                }
            }
        }
        let text_width = if max_text_width > 0.0 {
            max_text_width
        } else {
            AxisMetrics::DEFAULT_TEXT_WIDTH
        };
        // reference `minimumWidth` floors the negotiated strip width (chart-widget.ts
        // `_adjustSizeImpl`: `Math.max(optimalWidth(), minimumWidth)` across the pane's
        // scales on this side).
        let minimum_width = if target == PriceScaleTarget::Overlay {
            0.0
        } else {
            self.panes
                .iter()
                .filter_map(|pane| pane.scale(target))
                .map(|scale| scale.options().minimum_width)
                .fold(0.0_f64, f64::max)
        };
        AxisMetrics::price_strip_width(text_width, minimum_width)
    }

    pub(crate) fn optimal_exact_price_axis_width_for<F, G>(
        &mut self,
        pane_index: usize,
        target: PriceScaleTarget,
        measure: F,
        countdown_measure: G,
    ) -> f64
    where
        F: Fn(&str, bool) -> f64,
        G: Fn(&str, bool) -> f64,
    {
        // Sync tick sizing first: this path also runs standalone (tests, direct negotiation),
        // and its tick sets must match what the frame will build.
        self.sync_axis_tick_fonts();
        // Tick sizing follows the resolved axis metrics through the synced scale font, so the
        // exact path shares Path A's chrome instead of its own narrower constant.
        let Some(pane) = self.panes.get(pane_index) else {
            return 0.0;
        };
        let Some(scale) = pane.scale(target) else {
            return 0.0;
        };
        let marks = self.scale_tick_marks(pane_index, target, 0.0);
        let bold_round = Self::bold_round_mark_decisions(&marks, scale.options().bold_round_labels);
        let mut text_width = marks
            .iter()
            .zip(bold_round)
            .map(|(mark, bold)| {
                measure(
                    &self.format_tick_value(pane_index, target, scale, mark.logical),
                    bold,
                )
            })
            .fold(0.0_f64, f64::max);
        if let Some((from, to)) = self.visible_range_for_frame() {
            for series in self.series.iter().filter(|series| {
                let display_target = if series.price_scale_target == PriceScaleTarget::Overlay {
                    PriceScaleTarget::Right
                } else {
                    series.price_scale_target
                };
                series.visible && series.pane_index == pane_index && display_target == target
            }) {
                let conversion_scale = pane_scale(pane, series.price_scale_target);
                let value = if series.kind == SeriesKind::Custom {
                    series.custom_frame.last_visible.map(|last| last.value)
                } else {
                    self.data
                        .plot(series.id)
                        .last_non_whitespace_row(to)
                        .map(|row| {
                            self.heikin_ashi_row(series.id, row)
                                .map(|values| values[3])
                                .unwrap_or_else(|| {
                                    self.data
                                        .plot(series.id)
                                        .value_at(row, PlotValueIndex::Close)
                                })
                        })
                };
                if let (Some(value), Some(base)) = (value, self.series_base_value(series.id, from))
                {
                    text_width = text_width.max(measure(
                        &self.format_series_value(
                            series,
                            conversion_scale,
                            conversion_scale.price_to_logical_value(value, base),
                        ),
                        false,
                    ));
                }
                if series.countdown_visible {
                    if let Some(countdown) = self.series_countdown_text(series.id) {
                        text_width =
                            text_width.max(countdown_text_width(&countdown, &countdown_measure));
                    }
                }
                for line in &series.price_lines {
                    if !line.axis_label_visible {
                        continue;
                    }
                    let text = if line.title.is_empty() {
                        let Some(base) = self.series_base_value(series.id, from) else {
                            continue;
                        };
                        self.format_series_value(
                            series,
                            conversion_scale,
                            conversion_scale.price_to_logical_value(line.price, base),
                        )
                    } else {
                        line.title.clone()
                    };
                    text_width = text_width.max(measure(&text, false));
                }
            }
            if self.crosshair_mode != CrosshairMode::Hidden
                && self.options.get().crosshair.horz_line.label_visible
                && !scale.is_empty()
            {
                if let Some(series) = self.series.iter().find(|series| {
                    series.visible
                        && series.pane_index == pane_index
                        && series.price_scale_target == target
                }) {
                    if let Some(base) = self.series_base_value(series.id, from) {
                        let top = scale.coordinate_to_price(1.0, base);
                        let bottom = scale.coordinate_to_price(pane.height - 2.0, base);
                        for sample in [
                            top.min(bottom).floor() + 0.111_111_111_111_11,
                            top.max(bottom).ceil() - 0.111_111_111_111_11,
                        ] {
                            text_width = text_width.max(measure(
                                &self.format_series_value(
                                    series,
                                    scale,
                                    scale.price_to_logical_value(sample, base),
                                ),
                                false,
                            ));
                        }
                    }
                }
            }
        }
        AxisMetrics::price_strip_width(
            text_width.max(AxisMetrics::DEFAULT_TEXT_WIDTH),
            scale.options().minimum_width,
        )
    }

    pub(super) fn append_price_line_labels<F>(&self, labels: &mut Vec<AxisLabel>, measure: &F)
    where
        F: Fn(&str, bool) -> f64,
    {
        let metrics = self.axis_metrics();
        for (pi, pane) in self.panes.iter().enumerate() {
            for s in &self.series {
                if s.pane_index != pi {
                    continue;
                }
                let target = series_scale_target(s);
                let scale = pane_scale(pane, target);
                let display_target = if target == PriceScaleTarget::Overlay {
                    PriceScaleTarget::Right
                } else {
                    target
                };
                let Some((side, strip_x, strip_width)) =
                    self.price_scale_axis_geometry(pi, display_target)
                else {
                    continue;
                };
                let Some(base_value) = self.visible_series_base_value(s.id) else {
                    continue;
                };
                for line in &s.price_lines {
                    // reference `axisLabelVisible`: a hidden label leaves the line itself drawn.
                    if !line.axis_label_visible {
                        continue;
                    }
                    if scale.is_empty() {
                        continue;
                    }
                    let y = scale.price_to_coordinate(line.price, base_value);
                    if y < pane.top || y > pane.top + pane.height {
                        continue;
                    }
                    let text = if line.title.is_empty() {
                        // reference custom-price-line-price-axis-view.ts: the label is the line's
                        // price in the OWNING series' format.
                        self.format_series_value(
                            s,
                            scale,
                            scale.price_to_logical_value(line.price, base_value),
                        )
                    } else {
                        line.title.clone()
                    };
                    let width = AxisMetrics::price_tag_width(measure(&text, false));
                    let height = metrics.price_tag_height();
                    let (x, align, background_x) = if side == PriceScaleSide::Left {
                        (
                            strip_x + strip_width - AxisMetrics::PRICE_TEXT_INSET,
                            AxisTextAlign::Right,
                            strip_x + strip_width - width,
                        )
                    } else {
                        (
                            strip_x + AxisMetrics::PRICE_TEXT_INSET,
                            AxisTextAlign::Left,
                            strip_x,
                        )
                    };
                    // The label background follows the line color; omitted text automatically
                    // contrasts with that effective background.
                    let background = line
                        .axis_label_color
                        .as_deref()
                        .and_then(Color::parse_css)
                        .unwrap_or(line.color);
                    let text_color = line
                        .axis_label_text_color
                        .as_deref()
                        .and_then(Color::parse_css)
                        .unwrap_or_else(|| self.axis_label_text_color(background));
                    labels.push(AxisLabel {
                        text,
                        x,
                        y,
                        color: text_color,
                        align,
                        midpoint: AxisTextMidpoint::Label,
                        font_scale: AXIS_FONT_SCALE,
                        bold: false,
                        background: Some((
                            background_x,
                            y - height / 2.0,
                            width,
                            height,
                            background,
                        )),
                        background_corners: AxisLabelCorners::for_align(align),
                        measure_extra: 0.0,
                        attach_group: None,
                        border: None,
                    });
                }
            }
        }
    }

    fn append_action_axis_labels<F>(
        &self,
        labels: &mut Vec<AxisLabel>,
        live_price_regions: &[LivePriceRegion],
        measure: &F,
    ) where
        F: Fn(&str, bool) -> f64,
    {
        let metrics = self.axis_metrics();
        let fallback = aeris_charts_core::style::DEFAULT_SURFACE_RGB;
        let chip_fill = Color::parse_css(&self.options.get().layout.background.color)
            .unwrap_or(Color::rgb(fallback.0, fallback.1, fallback.2))
            .solid();
        let mut append = |pane_index: usize,
                          target: PriceScaleTarget,
                          price: f64,
                          text: String,
                          color: Color,
                          solid: bool,
                          hollow_at_live_price: bool,
                          bold: bool| {
            let Some(pane) = self.panes.get(pane_index) else {
                return;
            };
            let Some(y) = self.runtime_price_coordinate(pane_index, target, price) else {
                return;
            };
            if y < pane.top || y > pane.top + pane.height {
                return;
            }
            if target == PriceScaleTarget::Overlay {
                return;
            }
            let width = AxisMetrics::price_tag_width(measure(&text, false));
            let height = metrics.price_tag_height();
            let meets_live_price = live_price_regions.iter().any(|primary| {
                primary.primary && primary.pane_index == pane_index && primary.target == target && {
                    y + height / 2.0 > primary.top && y - height / 2.0 < primary.bottom
                }
            });
            let solid = solid && !(hollow_at_live_price && meets_live_price);
            let (x, align, background_x) = if target == PriceScaleTarget::Left {
                (
                    self.pane_left - AxisMetrics::PRICE_TEXT_INSET,
                    AxisTextAlign::Right,
                    self.pane_left - width,
                )
            } else {
                (
                    self.pane_left + self.pane_w + AxisMetrics::PRICE_TEXT_INSET,
                    AxisTextAlign::Left,
                    self.pane_left + self.pane_w,
                )
            };
            labels.push(AxisLabel {
                text,
                x,
                y,
                color: if solid { color.contrast_text() } else { color },
                align,
                midpoint: AxisTextMidpoint::Label,
                font_scale: AXIS_FONT_SCALE,
                bold,
                background: Some((
                    background_x,
                    y - height / 2.0,
                    width,
                    height,
                    if solid { color.solid() } else { chip_fill },
                )),
                background_corners: AxisLabelCorners::for_align(align),
                measure_extra: 0.0,
                attach_group: None,
                border: (!solid).then_some((1.0, color)),
            });
        };
        for position in &self.trading_state.positions {
            append(
                position.pane_index,
                position.price_scale.into(),
                position.average_price,
                self.format_trading_price(position.average_price),
                self.trading_position_color(position.side),
                true,
                true,
                false,
            );
        }
        for order in &self.trading_state.orders {
            let color = super::trading_geometry::trading_order_color(
                &self.trading_state.style,
                order.kind,
                order.side,
                order.role,
                order.status,
            );
            append(
                order.pane_index,
                order.price_scale.into(),
                self.trading_effective_order_price(order),
                self.format_trading_price(self.trading_effective_order_price(order)),
                color,
                order.status == crate::OrderStatus::Filled,
                true,
                false,
            );
            if order.kind == crate::OrderKind::StopLimit {
                if let Some(stop_price) = order.stop_price {
                    append(
                        order.pane_index,
                        order.price_scale.into(),
                        stop_price,
                        self.format_trading_price(stop_price),
                        color,
                        false,
                        true,
                        false,
                    );
                }
            }
        }
        for line in &self.alert_state.lines {
            let target = line.price_scale.into();
            let price = self
                .visible_range_for_frame()
                .and_then(|(from, _)| {
                    let series = self.scale_formatter_source(line.pane_index, target)?;
                    let scale = pane_scale(self.panes.get(line.pane_index)?, target);
                    let base = self.series_base_value(series.id, from)?;
                    Some(self.format_series_value(
                        series,
                        scale,
                        scale.price_to_logical_value(line.price, base),
                    ))
                })
                .unwrap_or_else(|| self.price_formatter.format(line.price));
            // Labels are host metadata. The tag always shows the actual formatted price; the
            // attached bell badge is what marks the line as an alert.
            append(
                line.pane_index,
                target,
                line.price,
                price,
                self.alert_color(line.status),
                true,
                false,
                false,
            );
        }
    }

    /// the public reference's horizontal-line/ray axis label: the drawing's price boxed on the price
    /// axis in the LINE's own color (the label is part of the drawing — recoloring the line
    /// recolors the label on the next frame), with automatic contrasting text unless explicitly
    /// configured. Formatted
    /// with the pane's primary series' price format, like the price-line labels.
    pub(super) fn append_drawing_line_labels<F>(&self, labels: &mut Vec<AxisLabel>, measure: &F)
    where
        F: Fn(&str, bool) -> f64,
    {
        let metrics = self.axis_metrics();
        for (pi, pane) in self.panes.iter().enumerate() {
            let Some(scale) = self.drawing_scale(pi) else {
                continue;
            };
            if scale.is_empty() {
                continue;
            }
            let base = self.drawing_scale_base(pi);
            let series = self.series.iter().find(|s| {
                s.pane_index == pi && s.price_scale_target == PriceScaleTarget::Right && s.visible
            });
            for drawing in &self.drawings {
                if drawing.pane_index != pi || !drawing.kind.spec().axis_price_label {
                    continue;
                }
                let Some(point) = drawing.points.first() else {
                    continue;
                };
                let y = scale.price_to_coordinate(point.price, base);
                if y < pane.top || y > pane.top + pane.height {
                    continue;
                }
                let logical = scale.price_to_logical_value(point.price, base);
                let text = match series {
                    Some(s) => self.format_series_value(s, scale, logical),
                    None => self.format_scale_value(scale, logical),
                };
                // The label IS the line: background in the drawing's color (parsed per frame,
                // so option changes track), with an explicit text override when configured.
                let background = Color::parse_css(&drawing.color).unwrap_or(PRIMARY).solid();
                let text_color = drawing
                    .label_text_color
                    .as_deref()
                    .and_then(Color::parse_css)
                    .unwrap_or_else(|| self.axis_label_text_color(background));
                let width = AxisMetrics::price_tag_width(measure(&text, false));
                let height = metrics.price_tag_height();
                labels.push(AxisLabel {
                    text,
                    x: self.pane_left + self.pane_w + AxisMetrics::PRICE_TEXT_INSET,
                    y,
                    color: text_color,
                    align: AxisTextAlign::Left,
                    midpoint: AxisTextMidpoint::Label,
                    font_scale: AXIS_FONT_SCALE,
                    bold: false,
                    background: Some((
                        self.pane_left + self.pane_w,
                        y - height / 2.0,
                        width,
                        height,
                        background,
                    )),
                    background_corners: AxisLabelCorners::for_align(AxisTextAlign::Left),
                    measure_extra: 0.0,
                    attach_group: None,
                    border: None,
                });
            }
        }
    }

    /// reference SeriesPriceAxisView: every visible series with `lastValueVisible` (default true)
    /// gets a last-value label on its price scale — the background is the series' bar color,
    /// contrasting black/white text, and the last visible bar's close in the scale's format.
    /// Labels sharing an axis side are pushed apart with the reference's overlap resolution
    /// (price-axis-widget.ts `_fixLabelOverlap`).
    ///
    /// industry-standard cluster extension: the label is one connected box of up to three
    /// independently-toggleable parts — a title chip (the series' `title` in a darker shade of
    /// the label color, left of the price text), the price text itself, and a candle-close
    /// countdown row stacked below, spanning the cluster's full width. The cluster renders while
    /// ANY part is enabled (e.g. `lastValueVisible: false` still leaves title chip + countdown).
    /// The overlap pass runs on the cluster's total height, and the axis-facing corners of the
    /// cluster's outer edges are rounded (internal boundaries stay sharp).
    fn append_last_value_label<F, G>(
        &self,
        labels: &mut Vec<AxisLabel>,
        measure: &F,
        countdown_measure: &G,
    ) -> Vec<LivePriceRegion>
    where
        F: Fn(&str, bool) -> f64,
        G: Fn(&str, bool) -> f64,
    {
        let Some((from, to)) = self.visible_range_for_frame() else {
            return Vec::new();
        };
        let metrics = self.axis_metrics();
        let row_height = metrics.price_tag_height();
        // Countdown row: 10px secondary text with the shared 2px vertical padding, keeping the
        // cluster compact without competing with the price.
        let countdown_row_height = metrics.countdown_row_height();
        let mut groups: Vec<(usize, PriceScaleTarget, Vec<LastValueLabel>)> = Vec::new();
        for (pi, pane) in self.panes.iter().enumerate() {
            for series in &self.series {
                if !series.visible || series.pane_index != pi {
                    continue;
                }
                let show_price = series.last_value_visible;
                let title = if series.title_visible && !series.title.is_empty() {
                    Some(series.title.clone())
                } else {
                    None
                };
                let countdown = if series.countdown_visible {
                    self.series_countdown_text(series.id)
                } else {
                    None
                };
                if !show_price && title.is_none() && countdown.is_none() {
                    continue;
                }
                let target = series_scale_target(series);
                let scale = pane_scale(pane, target);
                let plot = self.data.plot(series.id);
                if plot.is_empty() || scale.is_empty() {
                    continue;
                }
                // The cluster anchors at the last VISIBLE bar's value in the series' bar color,
                // exactly like the plain label (reference series.ts lastValueData(false));
                // a custom series reads its host-recorded frame values instead (Phase C-c).
                let (y, color, text, stale) = if series.kind == SeriesKind::Custom {
                    let Some(last) = series.custom_frame.last_visible else {
                        continue;
                    };
                    let Some(base_value) = self.series_base_value(series.id, from) else {
                        continue;
                    };
                    let y = scale.price_to_coordinate(last.value, base_value);
                    if y < 0.0 || y > self.pane_h {
                        continue;
                    }
                    let text = self.format_series_value(
                        series,
                        scale,
                        scale.price_to_logical_value(last.value, base_value),
                    );
                    (
                        y,
                        self.effective_series_live_color(series, last.color),
                        text,
                        // A custom series reports only its last VISIBLE frame value, so there is
                        // no final row to compare against — treat it as live.
                        false,
                    )
                } else {
                    // whitespace rows are skipped (the reference's plot list omits them).
                    let Some(row) = plot.last_non_whitespace_row(to) else {
                        continue;
                    };
                    let close = self
                        .heikin_ashi_row(series.id, row)
                        .map(|values| values[3])
                        .unwrap_or_else(|| plot.value_at(row, PlotValueIndex::Close));
                    if !close.is_finite() {
                        continue;
                    }
                    let Some(base_value) = self.series_base_value(series.id, from) else {
                        continue;
                    };
                    let y = scale.price_to_coordinate(close, base_value);
                    if y < 0.0 || y > self.pane_h {
                        continue;
                    }
                    let baseline = if series.kind == SeriesKind::Baseline {
                        self.resolved_baseline_price(series.id, from, to)
                    } else {
                        None
                    };
                    let color = self.effective_series_live_color(
                        series,
                        self.series_bar_color(series, row, baseline),
                    );
                    // The series' OWN priceFormat drives its last-value label (reference
                    // series-price-axis-view.ts text, via the scale's series formatter).
                    let text = self.format_series_value(
                        series,
                        scale,
                        scale.price_to_logical_value(close, base_value),
                    );
                    // reference `lastValueData(false)` anchors on the last VISIBLE bar. When the
                    // series' real final bar is scrolled out of view — what a negative right
                    // offset produces — the chip is showing a stale value, and the public reference marks
                    // that by outlining the chip instead of filling it.
                    let stale = plot
                        .last_non_whitespace_row_before(plot.size())
                        .is_some_and(|last| last != row);
                    (y, color, text, stale)
                };
                // reference appends overlay (no-scale) series' labels to the pane's default axis
                // (price-axis-widget.ts:601-607); the engine's default axis is the right one.
                // The label's `alignLabels` comes from the axis it lands on.
                let display_target = if target == PriceScaleTarget::Overlay {
                    PriceScaleTarget::Right
                } else {
                    target
                };
                if self.price_scale_axis_geometry(pi, display_target).is_none() {
                    continue;
                }
                let align = pane_scale(pane, display_target).options().align_labels;
                let primary = self
                    .scale_formatter_source(pi, display_target)
                    .is_some_and(|source| source.id == series.id);
                let group_index = groups
                    .iter()
                    .position(|(pane, candidate, _)| *pane == pi && *candidate == display_target)
                    .unwrap_or_else(|| {
                        groups.push((pi, display_target, Vec::new()));
                        groups.len() - 1
                    });
                // The top row exists only for the price TEXT: when the price chip is off, the
                // title chip attaches to the countdown row instead of leaving a phantom blank
                // row in the strip (and the cluster centers on the value, no shift).
                let top_height = if show_price { row_height } else { 0.0 };
                let countdown_height = if countdown.is_some() {
                    countdown_row_height
                } else {
                    0.0
                };
                // A title-only cluster (price off, countdown off) still gets one row so the
                // outside chip has somewhere to live, centered on the value.
                let height = if top_height + countdown_height > 0.0 {
                    top_height + countdown_height
                } else {
                    row_height
                };
                groups[group_index].2.push(LastValueLabel {
                    price_text: show_price.then_some(text),
                    title,
                    countdown,
                    group_id: series.id,
                    // The price row stays centered on the value coordinate; the countdown row
                    // hangs below, so a FULL cluster's center shifts down by half the countdown
                    // row. Without a price row the cluster centers on the value directly.
                    y: y + if top_height > 0.0 {
                        countdown_height / 2.0
                    } else {
                        0.0
                    },
                    height,
                    top_height,
                    // Chip backgrounds follow the series color but always paint solid: a
                    // translucent bar/line color must not bleed through the chips.
                    color: color.solid(),
                    align,
                    primary,
                    // The scale's primary source is the symbol that scale belongs to, and its
                    // chip is always filled — a comparison series on its own scale or pane is
                    // primary there too, so it stays filled as well. Only a secondary source
                    // sharing someone else's scale outlines, and only once its value goes stale.
                    hollow: stale && !primary,
                    selected: self.series_is_selected(series.id),
                });
                // industry-standard bid/ask chips (`bid_ask_visible`, default off): one
                // title+price cluster per side with a live quote, centered on the quote's
                // coordinate. Their attach groups are offset far from any series id so the
                // chips never chain into the main cluster (or each other) when adjacent.
                if series.bid_ask_visible {
                    let sides = [
                        ("Bid", series.bid, series.bid_color.as_str(), PRIMARY, 1u32),
                        ("Ask", series.ask, series.ask_color.as_str(), DOWN, 2u32),
                    ];
                    for (side, value, css, fallback, side_offset) in sides {
                        let Some(quote) = value else {
                            continue;
                        };
                        let Some(base_value) = self.series_base_value(series.id, from) else {
                            continue;
                        };
                        let quote_y = scale.price_to_coordinate(quote, base_value);
                        if quote_y < 0.0 || quote_y > self.pane_h {
                            continue;
                        }
                        let quote_text = self.format_series_value(
                            series,
                            scale,
                            scale.price_to_logical_value(quote, base_value),
                        );
                        let side_color = Color::parse_css(css).unwrap_or(fallback).solid();
                        groups[group_index].2.push(LastValueLabel {
                            price_text: Some(quote_text),
                            title: Some(side.to_string()),
                            countdown: None,
                            group_id: (1u32 << 30) + series.id * 4 + side_offset,
                            y: quote_y,
                            height: row_height,
                            top_height: row_height,
                            color: side_color,
                            align,
                            primary: false,
                            hollow: false,
                            // Bid/ask quote chips are not selectable sources of their own.
                            selected: false,
                        });
                    }
                }
            }
        }
        // Reference aligns labels independently per price-axis widget.
        let mut live_price_regions = Vec::new();
        for (pane_index, target, mut group) in groups {
            // Chips that would collide are SPACED, not restyled: `resolve_last_value_label_overlap`
            // already pushes them a full box apart, so a chip's fill carries only whether its
            // value is live (see `hollow`) rather than doubling as collision feedback.
            resolve_last_value_label_overlap(&mut group, self.pane_h);
            for label in &group {
                live_price_regions.push(LivePriceRegion {
                    pane_index,
                    target,
                    top: label.y - label.height / 2.0,
                    bottom: label.y + label.height / 2.0,
                    primary: label.primary,
                });
            }
            let Some((side, strip_x, strip_width)) =
                self.price_scale_axis_geometry(pane_index, target)
            else {
                continue;
            };
            // Reference geometry (price-axis-view-renderer.ts `_calculateGeometry`): every boxed
            // label is its own content-sized box — borderSize + paddingInner + paddingOuter +
            // tickLength on the text — no cross-label width sharing.
            for label in group {
                // Plain single-box label (no chip, no countdown): the reference-shaped emission,
                // byte-identical to pre-cluster behavior.
                if label.title.is_none() && label.countdown.is_none() {
                    let Some(text) = label.price_text else {
                        continue;
                    };
                    let width = AxisMetrics::price_tag_width(measure(&text, false));
                    let (x, align, background_x) = if side == PriceScaleSide::Left {
                        (
                            strip_x + strip_width - AxisMetrics::PRICE_TEXT_INSET,
                            AxisTextAlign::Right,
                            strip_x + strip_width - width,
                        )
                    } else {
                        (
                            strip_x + AxisMetrics::PRICE_TEXT_INSET,
                            AxisTextAlign::Left,
                            strip_x,
                        )
                    };
                    labels.push(AxisLabel {
                        text,
                        x,
                        y: label.y,
                        color: if label.hollow {
                            label.color
                        } else {
                            self.axis_label_text_color(label.color)
                        },
                        align,
                        midpoint: AxisTextMidpoint::Label,
                        font_scale: AXIS_FONT_SCALE,
                        bold: false,
                        background: Some((
                            background_x,
                            label.y - label.height / 2.0,
                            width,
                            label.height,
                            if label.hollow {
                                self.chart_surface_color()
                            } else {
                                label.color
                            },
                        )),
                        background_corners: AxisLabelCorners::for_align(align),
                        measure_extra: 0.0,
                        attach_group: None,
                        border: label.hollow.then_some((1.0, label.color)),
                    });
                    if label.selected {
                        labels.push(selected_chip_accent(
                            background_x,
                            label.y - label.height / 2.0,
                            width,
                            label.height,
                            label.color,
                            side == PriceScaleSide::Right,
                            align,
                        ));
                    }
                    continue;
                }
                self.append_last_value_cluster(
                    labels,
                    &label,
                    pane_index,
                    target,
                    measure,
                    countdown_measure,
                );
            }
        }
        live_price_regions
    }

    /// Emit one industry-standard last-value cluster (see `append_last_value_label`): one
    /// connected box whose top row holds the title chip (darker shade, left) + price area and
    /// whose optional countdown row spans the full width below. Box width covers the widest row;
    /// each row's text is centered in its area. Axis-facing outer corners are rounded (right
    /// corners on the right strip, left corners on the left strip); internal boundaries and the
    /// chart-facing side stay sharp. On the left strip the chip is the leftmost (axis-facing)
    /// top-row box, so it carries that side's rounded corners instead of the price area.
    fn append_last_value_cluster<F, G>(
        &self,
        labels: &mut Vec<AxisLabel>,
        label: &LastValueLabel,
        pane_index: usize,
        target: PriceScaleTarget,
        measure: &F,
        countdown_measure: &G,
    ) where
        F: Fn(&str, bool) -> f64,
        G: Fn(&str, bool) -> f64,
    {
        let Some((side, strip_x, strip_width)) = self.price_scale_axis_geometry(pane_index, target)
        else {
            return;
        };
        let right_strip = side == PriceScaleSide::Right;
        let fill = if label.hollow {
            self.chart_surface_color()
        } else {
            label.color
        };
        let border = label.hollow.then_some((1.0, label.color));
        let text_color = if label.hollow {
            label.color
        } else {
            self.axis_label_text_color(label.color)
        };
        let countdown_text_color =
            Color::rgba(text_color.r(), text_color.g(), text_color.b(), 0xb3);
        // The title chip shares the main label color by default (matching the price and
        // countdown chips).
        let chip_color = fill;
        let fitted_title = label
            .title
            .as_deref()
            .and_then(|title| fit_axis_text(title, (self.pane_w - 10.0).max(0.0), measure));
        let title_w = fitted_title.as_deref().map(|text| measure(text, false));
        let chip_w = title_w.map(|w| w + 10.0).unwrap_or(0.0);
        let price_w = label
            .price_text
            .as_deref()
            .map(|text| measure(text, false))
            .unwrap_or(0.0);
        let countdown_w = label
            .countdown
            .as_deref()
            .map(|text| countdown_text_width(text, countdown_measure))
            .unwrap_or(0.0);
        // the public reference geometry: the title chip sits outside the strip and the price/countdown
        // box sits inside it. Their logical bounds meet at the border; the primitive encoder
        // excludes the border's exact device pixels from both axis-side boxes.
        // Inside rows share one width, stack flush, and start text at the tick-label inset from
        // the border; the box retains the shared strip padding.
        const TEXT_INSET: f64 = AxisMetrics::PRICE_TEXT_INSET;
        const RIGHT_PAD: f64 = AxisMetrics::PRICE_PAD_OUTER;
        let inner_text_w = price_w.max(countdown_w);
        // Shared box: border + gap + text + outer padding, matching the tag chrome (the inset
        // already contains the border, so it must not be added twice).
        let inner_w = 1.0 + AxisMetrics::PRICE_TEXT_GAP + inner_text_w + RIGHT_PAD;
        let border_x = if right_strip {
            strip_x
        } else {
            strip_x + strip_width
        };
        let inner_x = if right_strip {
            border_x
        } else {
            border_x - inner_w
        };
        let text_x = if right_strip {
            border_x + TEXT_INSET
        } else {
            border_x - TEXT_INSET
        };
        let text_align = if right_strip {
            AxisTextAlign::Left
        } else {
            AxisTextAlign::Right
        };
        let top_y = label.y - label.height / 2.0;
        let has_countdown = label.countdown.is_some();
        let axis_corners_top = if right_strip {
            AxisLabelCorners {
                top_right: true,
                bottom_right: !has_countdown,
                ..AxisLabelCorners::NONE
            }
        } else {
            AxisLabelCorners {
                top_left: true,
                bottom_left: !has_countdown,
                ..AxisLabelCorners::NONE
            }
        };
        let axis_corners_bottom = if right_strip {
            AxisLabelCorners::RIGHT
        } else {
            AxisLabelCorners::LEFT
        };
        // Title chip: outside the strip, ending exactly at the border's chart-side edge. The
        // primitive encoder starts the price box after the border's strip-side edge, so the border
        // is the complete seam: neither chip overlaps it and no chart-surface gap is introduced.
        if let (Some(title), Some(_)) = (&fitted_title, title_w) {
            let chip_x = if right_strip {
                border_x - chip_w
            } else {
                border_x
            };
            // Without a price row the chip attaches to the cluster's single inside row (the
            // countdown row, or the title-only row) — never a phantom blank top row.
            let chip_row_h = if label.top_height > 0.0 {
                label.top_height
            } else {
                label.height - label.top_height
            };
            labels.push(AxisLabel {
                text: title.clone(),
                x: chip_x + chip_w / 2.0,
                y: top_y + chip_row_h / 2.0,
                color: text_color,
                align: AxisTextAlign::Center,
                midpoint: AxisTextMidpoint::Label,
                font_scale: AXIS_FONT_SCALE,
                bold: false,
                background: Some((chip_x, top_y, chip_w, chip_row_h, chip_color)),
                // The outside chip rounds only its OUTER (chart-facing) side — sharp on the
                // axis-facing side, matching the side rule for axis labels.
                background_corners: if right_strip {
                    AxisLabelCorners::LEFT
                } else {
                    AxisLabelCorners::RIGHT
                },
                // It lives on the pane, not in the strip: it never widens the axis.
                measure_extra: 0.0,
                attach_group: None,
                border,
            });
        }
        // The inside price chip renders only when the price text is present (never an empty box).
        if label.top_height > 0.0 && label.price_text.is_some() {
            labels.push(AxisLabel {
                text: label.price_text.clone().unwrap_or_default(),
                x: text_x,
                y: top_y + label.top_height / 2.0,
                color: text_color,
                align: text_align,
                midpoint: AxisTextMidpoint::Label,
                font_scale: AXIS_FONT_SCALE,
                bold: false,
                background: Some((inner_x, top_y, inner_w, label.top_height, fill)),
                background_corners: axis_corners_top,
                // text + the standard 21px label padding already covers the chip box.
                measure_extra: 0.0,
                // Price chip and countdown chip paint with a shared edge (attached) — the
                // group id is the series id, so the attach never chains into another series'
                // cluster on the same strip.
                attach_group: Some(label.group_id),
                border,
            });
        }
        if let Some(countdown) = &label.countdown {
            let countdown_y = top_y + label.top_height;
            let countdown_height = label.height - label.top_height;
            // A countdown-only cluster's top edge is the cluster's top edge, so the countdown
            // box carries the top axis-facing corner as well.
            let standalone = label.top_height == 0.0;
            let corners = if standalone {
                axis_corners_bottom
            } else if right_strip {
                AxisLabelCorners {
                    top_right: false,
                    ..AxisLabelCorners::RIGHT
                }
            } else {
                AxisLabelCorners {
                    top_left: false,
                    ..AxisLabelCorners::LEFT
                }
            };
            labels.push(AxisLabel {
                text: countdown.clone(),
                x: text_x,
                y: countdown_y + countdown_height / 2.0,
                color: countdown_text_color,
                align: text_align,
                midpoint: AxisTextMidpoint::Label,
                font_scale: COUNTDOWN_FONT_SCALE,
                bold: false,
                background: Some((inner_x, countdown_y, inner_w, countdown_height, fill)),
                background_corners: corners,
                measure_extra: 0.0,
                // Attached to the price chip above (shared edge, no rounding gap) — the
                // group id is the series id, so the attach never chains into another series'
                // cluster on the same strip.
                attach_group: Some(label.group_id),
                border,
            });
        }
        // Selected-series accent (the public reference's active-chip indication): a bar in a lighter shade
        // of the series color, pinned to the cluster's axis-facing edge and painted over the
        // chips it marks. Pushed last so it lands on top; it carries no attach group so it cannot
        // disturb the price/countdown shared-edge chain.
        if label.selected && label.price_text.is_some() {
            labels.push(selected_chip_accent(
                inner_x,
                top_y,
                inner_w,
                label.height,
                label.color,
                right_strip,
                text_align,
            ));
        }
    }

    /// The series' candle-close countdown text (industry-standard `countdown_visible`): the time
    /// until the forming bar closes. The interval is the median of the last up-to-10 inter-bar
    /// deltas of the series' own bar times (fallback: the last delta). The countdown shows only
    /// while the host clock is inside the last bar's interval `[last_bar_time, last_bar_time +
    /// interval)`; outside it — a lunch break, overnight, a weekend, or after an early close —
    /// the row hides instead of cycling through intervals the market never trades. Calendar-date
    /// bars form during their exchange trading day(s), or through the end of their calendar
    /// month(s) for monthly and longer bars. `None` (the row hides) with fewer than two bars or no
    /// installed host clock (`now_override`).
    fn series_countdown_remaining_at(&self, id: SeriesId, now: f64) -> Option<f64> {
        if self.sequence_points().is_some() {
            return None;
        }
        let plot = self.data.plot(id);
        // The forming bar is the last traded row: hosts may pre-fill the rest of a session with
        // whitespace slots, and an all-whitespace series has no forming bar yet.
        let anchor = plot.last_non_whitespace_row_before(plot.size())?;
        // Up to 11 slots around it (→ 10 deltas) feed the interval inference. Whitespace slots
        // count, so the first traded bar of a pre-filled session still has a neighbour. An as-of
        // overlay forms its own bars, so it infers from its own rows rather than the points
        // repeating them.
        let (times, anchor, len) = if plot.is_as_of() {
            let (times, _) = self.data.series_data(id)?;
            (times, plot.source_row(anchor), times.len())
        } else {
            (self.data.merged_times(), anchor, plot.size())
        };
        let time_at = |row: usize| {
            if plot.is_as_of() {
                return times.get(row).copied();
            }
            plot.index_at(row)
                .and_then(|index| times.get(index as usize).copied())
        };
        let window_end = (anchor + 1).min(len - 1);
        let tail_times: Vec<i64> = (window_end.saturating_sub(10)..=window_end)
            .filter_map(time_at)
            .collect();
        let interval = median_bar_interval(&tail_times)?;
        let last_time = time_at(anchor)?;
        let (start, end) = if self.exchange_time.calendar_dates() {
            let (start, end) = self
                .exchange_time
                .calendar_bar_window_utc(last_time.div_euclid(86_400), interval);
            (start as f64, end as f64)
        } else {
            (last_time as f64, last_time as f64 + interval)
        };
        if now < start || now >= end {
            return None;
        }
        Some(end - now)
    }

    pub(crate) fn series_countdown_layout_key_at(
        &self,
        id: SeriesId,
        now: Option<f64>,
    ) -> Option<usize> {
        let remaining = self.series_countdown_remaining_at(id, now?)?;
        Some(countdown_layout_key(remaining))
    }

    pub(crate) fn series_countdown_text(&self, id: SeriesId) -> Option<String> {
        let remaining = self.series_countdown_remaining_at(id, self.now_override?)?;
        Some(format_countdown_remaining(remaining))
    }

    pub(super) fn append_crosshair_labels<F>(&self, labels: &mut Vec<AxisLabel>, measure: &F)
    where
        F: Fn(&str, bool) -> f64,
    {
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
        // Price-axis label tracks the horizontal line (reference `horzLine`); time-axis label tracks the
        // vertical line (reference `vertLine`). Each carries its own `labelVisible`/`labelBackgroundColor`,
        // and text automatically contrasts with each configured background.
        let options = self.options.get();
        let metrics = self.axis_metrics();
        let ch = &options.crosshair;
        if ch.horz_line.label_visible {
            if let Some(pi) = self
                .panes
                .iter()
                .position(|p| y_css >= p.top && y_css <= p.top + p.height)
            {
                // The horizontal line has one shared media-space coordinate. Each visible scale
                // independently maps that coordinate through its own range/mode/formatter.
                let snap_y = self.crosshair_snap(pi, x_css, y_css, from, to).1;
                for target in self.panes[pi].scale_targets() {
                    let Some((side, strip_x, strip_width)) =
                        self.price_scale_axis_geometry(pi, target)
                    else {
                        continue;
                    };
                    let Some(series) = self.scale_formatter_source(pi, target) else {
                        continue;
                    };
                    let scale = pane_scale(&self.panes[pi], target);
                    if scale.is_empty() {
                        continue;
                    }
                    let Some(base_value) = self.series_base_value(series.id, from) else {
                        continue;
                    };
                    let price = scale.coordinate_to_price(snap_y, base_value);
                    let text = self.format_series_value(
                        series,
                        scale,
                        scale.price_to_logical_value(price, base_value),
                    );
                    let width = AxisMetrics::price_tag_width(measure(&text, false));
                    let height = metrics.crosshair_price_tag_height();
                    let (label_x, align, background_x) = if side == PriceScaleSide::Left {
                        (
                            strip_x + strip_width - AxisMetrics::PRICE_TEXT_INSET,
                            AxisTextAlign::Right,
                            strip_x + strip_width - width,
                        )
                    } else {
                        (
                            strip_x + AxisMetrics::PRICE_TEXT_INSET,
                            AxisTextAlign::Left,
                            strip_x,
                        )
                    };
                    let label_bg =
                        css_color(&ch.horz_line.label_background_color, CROSSHAIR_LABEL_BG);
                    labels.push(AxisLabel {
                        text,
                        x: label_x,
                        y: snap_y,
                        color: self.axis_label_text_color(label_bg),
                        align,
                        midpoint: AxisTextMidpoint::Label,
                        font_scale: AXIS_FONT_SCALE,
                        bold: false,
                        background: Some((
                            background_x,
                            snap_y - height / 2.0,
                            width,
                            height,
                            label_bg,
                        )),
                        background_corners: AxisLabelCorners::for_align(align),
                        measure_extra: 0.0,
                        attach_group: None,
                        border: None,
                    });
                }
            }
        }
        if ch.vert_line.label_visible && x_css <= self.pane_w && self.time_axis_visible {
            let index = self.snapped_crosshair_index(x_css);
            // reference `indexToTime` returns null in the empty area — the time label is hidden
            // when the snapped index has no bar (past either data edge).
            if index >= 0 {
                let Some(time) = self.axis_time_key_at(index as usize) else {
                    return;
                };
                let text = self.format_crosshair_ts(time);
                let width = AxisMetrics::time_tag_width(measure(&text, false));
                let height = metrics.time_strip_height();
                let x = self.pane_left + self.time_scale.index_to_coordinate(index);
                let box_x = (x - width / 2.0).clamp(
                    self.pane_left,
                    (self.pane_left + self.pane_w - width).max(self.pane_left),
                );
                let label_bg = css_color(&ch.vert_line.label_background_color, CROSSHAIR_LABEL_BG);
                labels.push(AxisLabel {
                    text,
                    x: box_x + width / 2.0,
                    y: self.pane_h + metrics.time_text_dy(),
                    color: self.axis_label_text_color(label_bg),
                    align: AxisTextAlign::Center,
                    midpoint: AxisTextMidpoint::StableTime,
                    font_scale: AXIS_FONT_SCALE,
                    bold: false,
                    background: Some((box_x, self.pane_h, width, height, label_bg)),
                    background_corners: AxisLabelCorners::BOTTOM,
                    measure_extra: 0.0,
                    attach_group: None,
                    border: None,
                });
            }
        }
    }
}
