//! Measurement statistics shared by drawing labels (the Lines family stats box first; measuring
//! tools reuse it). Values are engine-formatted from the drawing's own price scale formatter,
//! anchor time identity, and media-px geometry, so every host and executor shows the same text.

use super::{Drawing, DrawingPriceScale};
use crate::{ChartEngine, DrawingLabelMetric, PriceScaleTarget};

impl DrawingPriceScale {
    pub(crate) fn target(self) -> PriceScaleTarget {
        match self {
            Self::Right => PriceScaleTarget::Right,
            Self::Left => PriceScaleTarget::Left,
            Self::Overlay => PriceScaleTarget::Overlay,
        }
    }
}

/// Whether formatted text reads as zero: it carries no digit other than `0`.
fn prints_zero(text: &str) -> bool {
    !text.chars().any(|c| c.is_ascii_digit() && c != '0')
}

/// Stats line a metric belongs to: price, time, then geometry.
fn metric_group(metric: DrawingLabelMetric) -> usize {
    match metric {
        DrawingLabelMetric::Price
        | DrawingLabelMetric::PriceChange
        | DrawingLabelMetric::PercentChange
        | DrawingLabelMetric::Ticks => 0,
        DrawingLabelMetric::BarCount
        | DrawingLabelMetric::DateTimeRange
        | DrawingLabelMetric::Duration
        | DrawingLabelMetric::VolumeInRange => 1,
        DrawingLabelMetric::Angle | DrawingLabelMetric::Distance => 2,
    }
}

/// Compact signed duration: the most significant of days, hours, minutes, and seconds plus the
/// next unit when it is non-zero.
pub(crate) fn format_duration(seconds: f64) -> String {
    const UNITS: [(&str, u64); 4] = [("d", 86_400), ("h", 3_600), ("m", 60), ("s", 1)];
    let sign = if seconds < 0.0 { "-" } else { "" };
    let total = seconds.abs().round() as u64;
    let Some(first) = UNITS.iter().position(|&(_, size)| total >= size) else {
        return "0s".to_string();
    };
    let (unit, size) = UNITS[first];
    let mut text = format!("{sign}{}{unit}", total / size);
    if let Some(&(next_unit, next_size)) = UNITS.get(first + 1) {
        let next = total % size / next_size;
        if next > 0 {
            text.push_str(&format!(" {next}{next_unit}"));
        }
    }
    text
}

impl ChartEngine {
    /// Glyph size of a drawing's own text and level labels in CSS px: its `text_size`, else the
    /// chart font size.
    pub(crate) fn drawing_text_size(&self, drawing: &Drawing) -> f64 {
        drawing.resolved_text_size(self.options.get().layout.font_size)
    }

    /// Glyph size of measurement boxes in CSS px: the chart font size, at least 11, shared by
    /// the Long/Short Position label chips and every family stats box.
    pub(crate) fn drawing_stats_size(&self) -> f64 {
        self.options.get().layout.font_size.max(11.0)
    }

    /// A price or price difference in the drawing's own price format: the host formatter, then
    /// instrument precision on the tick grid, then the bound scale's series format, then the
    /// default. One owner for every price a drawing prints (positions, ranges, Fibonacci, Gann),
    /// in the order the price axis itself follows.
    pub(crate) fn drawing_price_text(&self, drawing: &Drawing, value: f64) -> String {
        if let Some(text) = self
            .price_formatter_fn
            .as_ref()
            .and_then(|format| format(value))
        {
            return text;
        }
        if let Some(precision) = self.trading_state.instrument.price_precision {
            let tick = self.position_price_tick(drawing.pane_index, drawing.price_scale);
            return crate::PriceFormatter::from_precision(
                precision,
                tick.unwrap_or(10.0_f64.powi(-(precision as i32))),
            )
            .format(value);
        }
        self.scale_formatter_source(drawing.pane_index, drawing.price_scale.target())
            .and_then(|series| self.format_with_price_format(&series.price_format, value))
            .unwrap_or_else(|| self.price_formatter.format(value))
    }

    /// Screen angle (degrees, rising positive) and length (CSS px) between two anchors.
    pub(crate) fn drawing_screen_vector(
        &self,
        drawing: &Drawing,
        from: usize,
        to: usize,
    ) -> Option<(f64, f64)> {
        let a = self.drawing_point_px(drawing, *drawing.points.get(from)?)?;
        let b = self.drawing_point_px(drawing, *drawing.points.get(to)?)?;
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        Some(((-dy).atan2(dx).to_degrees(), dx.hypot(dy)))
    }

    /// One metric's engine-formatted text from anchor `from` to anchor `to`; `None` when the
    /// metric has no value on the current axis (for example a duration before time data).
    pub(crate) fn drawing_metric_text(
        &self,
        drawing: &Drawing,
        metric: DrawingLabelMetric,
        from: usize,
        to: usize,
    ) -> Option<String> {
        let first = drawing.points.get(from)?;
        let last = drawing.points.get(to)?;
        let change = last.price - first.price;
        Some(match metric {
            DrawingLabelMetric::Price => self.drawing_price_text(drawing, last.price),
            DrawingLabelMetric::PriceChange => {
                let text = self.drawing_price_text(drawing, change);
                // Zero is unsigned: a change that rounds to nothing prints without a sign.
                if prints_zero(&text) {
                    text.trim_start_matches(['-', '\u{2212}', '+']).to_string()
                } else if change > 0.0 {
                    format!("+{text}")
                } else {
                    text
                }
            }
            DrawingLabelMetric::PercentChange => {
                if first.price.abs() <= f64::EPSILON {
                    return None;
                }
                let percent = change / first.price.abs() * 100.0;
                if prints_zero(&format!("{percent:.2}")) {
                    "0.00%".to_string()
                } else {
                    format!("{percent:+.2}%")
                }
            }
            DrawingLabelMetric::Ticks => {
                // Ticks count on the grid the anchors snap to: the instrument tick or price-band
                // ladder, falling back to the bound scale's `min_move`. The count is a magnitude;
                // the sign follows the price change.
                let (pane, scale) = (drawing.pane_index, drawing.price_scale);
                let ticks = self
                    .position_price_ticks_between(pane, scale, first.price, last.price)
                    // Free anchors on a price-band ladder sit off its grid: count between the
                    // nearest grid prices instead of dropping the metric.
                    .or_else(|| {
                        let snap = |price| self.snap_position_price(pane, scale, price);
                        self.position_price_ticks_between(
                            pane,
                            scale,
                            snap(first.price),
                            snap(last.price),
                        )
                    })?
                    .round() as i64;
                let ticks = if change < 0.0 { -ticks } else { ticks };
                if ticks == 0 {
                    "0 ticks".to_string()
                } else {
                    format!("{ticks:+} ticks")
                }
            }
            DrawingLabelMetric::BarCount => {
                format!("{} bars", (last.logical - first.logical).round() as i64)
            }
            DrawingLabelMetric::DateTimeRange => {
                let start = self.drawing_anchor_time_of(drawing, from)?;
                let end = self.drawing_anchor_time_of(drawing, to)?;
                format!(
                    "{} – {}",
                    self.format_crosshair_ts(start.round() as i64),
                    self.format_crosshair_ts(end.round() as i64)
                )
            }
            DrawingLabelMetric::Duration => {
                let start = self.drawing_anchor_time_of(drawing, from)?;
                let end = self.drawing_anchor_time_of(drawing, to)?;
                format_duration(end - start)
            }
            DrawingLabelMetric::Angle => {
                let (angle, _) = self.drawing_screen_vector(drawing, from, to)?;
                format!("{angle:.2}°")
            }
            DrawingLabelMetric::Distance => {
                let (_, length) = self.drawing_screen_vector(drawing, from, to)?;
                format!("{length:.0} px")
            }
            // Volume needs a host-declared volume source; drawings carry none yet.
            DrawingLabelMetric::VolumeInRange => return None,
        })
    }

    /// Stats text of `drawing`'s visible `labels`, measured from anchor `from` to anchor `to`:
    /// one line each for the price, time, and geometry metrics present, values in label order
    /// joined by two spaces. A label's explicit `text` replaces its metric value; metrics without
    /// a value on the current axis (for example a duration before time data) are omitted.
    pub(crate) fn drawing_stat_lines(
        &self,
        drawing: &Drawing,
        from: usize,
        to: usize,
    ) -> Vec<String> {
        let mut groups: [Vec<String>; 3] = Default::default();
        for label in drawing.labels.iter().filter(|label| label.visible) {
            let value = label
                .text
                .clone()
                .or_else(|| self.drawing_metric_text(drawing, label.metric, from, to));
            if let Some(value) = value.filter(|value| !value.is_empty()) {
                groups[metric_group(label.metric)].push(value);
            }
        }
        groups
            .into_iter()
            .filter(|group| !group.is_empty())
            .map(|group| group.join("  "))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::format_duration;

    #[test]
    fn durations_keep_the_two_most_significant_units() {
        assert_eq!(format_duration(0.0), "0s");
        assert_eq!(format_duration(45.0), "45s");
        assert_eq!(format_duration(3_600.0), "1h");
        assert_eq!(
            format_duration(2.0 * 86_400.0 + 3.0 * 3_600.0 + 59.0),
            "2d 3h"
        );
        assert_eq!(format_duration(-(4.0 * 3_600.0 + 30.0 * 60.0)), "-4h 30m");
        assert_eq!(format_duration(86_400.0 + 30.0), "1d");
        assert_eq!(format_duration(90.0), "1m 30s");
    }
}
