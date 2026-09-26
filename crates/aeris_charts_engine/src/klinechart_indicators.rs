//! Chart presentation for KLineChart indicator bindings ([`IndicatorKind::KLineChart`]).
//!
//! The formulas live in `aeris_charts_indicators::klinechart`. This module draws the outputs the
//! way KLineChart does: lines at 1px in KLineChart's five-color line palette, bars and dots colored
//! row by row by the template's own rules, price-overlay templates on the source pane and every
//! other template in a pane of its own, and no last-value labels on the price axis.
//!
//! One deliberate difference: KLineChart draws a rising MACD or AO column as an outline. Aeris
//! histograms have no outline style, so those columns are drawn filled at a lighter alpha.

use super::*;
use aeris_charts_core::model::data_layer::POINT_COLOR_ABSENT;
use aeris_charts_indicators::klinechart::{Figure, Indicator, Placement, ValueFormat};

/// KLineChart's default line palette (`indicator.lines[*].color`), assigned to line outputs in
/// order.
pub const KLINECHART_LINE_COLORS: [&str; 5] =
    ["#FF9600", "#935EBD", "#1677FF", "#E11D74", "#01C5C4"];

/// KLineChart's line width for indicator lines.
const KLINECHART_LINE_WIDTH: f64 = 1.0;
/// KLineChart fills bars and dots at 70% opacity.
const FILL_ALPHA: u8 = 0xb3;
/// Stand-in for KLineChart's outlined (hollow) columns.
const HOLLOW_ALPHA: u8 = 0x4d;
/// KLineChart's `noChangeColor`, `#76808F`.
const NO_CHANGE: u32 = 0x7680_8fff;

const fn rgba(rgb: (u8, u8, u8), alpha: u8) -> u32 {
    (rgb.0 as u32) << 24 | (rgb.1 as u32) << 16 | (rgb.2 as u32) << 8 | alpha as u32
}

const UP_FILL: u32 = rgba(aeris_charts_core::style::MARKET_UP_RGB, FILL_ALPHA);
const UP_HOLLOW: u32 = rgba(aeris_charts_core::style::MARKET_UP_RGB, HOLLOW_ALPHA);
const DOWN_FILL: u32 = rgba(aeris_charts_core::style::MARKET_DOWN_RGB, FILL_ALPHA);
const DOWN_HOLLOW: u32 = rgba(aeris_charts_core::style::MARKET_DOWN_RGB, HOLLOW_ALPHA);

/// The `IndicatorInfo::kind` / schema name of a KLineChart binding: `klinechart_` followed by the
/// template name in lower case.
pub(crate) fn klinechart_kind_name(indicator: &Indicator) -> &'static str {
    match indicator {
        Indicator::Ma { .. } => "klinechart_ma",
        Indicator::Ema { .. } => "klinechart_ema",
        Indicator::Sma { .. } => "klinechart_sma",
        Indicator::Bbi { .. } => "klinechart_bbi",
        Indicator::Vol { .. } => "klinechart_vol",
        Indicator::Macd { .. } => "klinechart_macd",
        Indicator::Boll { .. } => "klinechart_boll",
        Indicator::Kdj { .. } => "klinechart_kdj",
        Indicator::Rsi { .. } => "klinechart_rsi",
        Indicator::Bias { .. } => "klinechart_bias",
        Indicator::Brar { .. } => "klinechart_brar",
        Indicator::Cci { .. } => "klinechart_cci",
        Indicator::Cr { .. } => "klinechart_cr",
        Indicator::Dma { .. } => "klinechart_dma",
        Indicator::Dmi { .. } => "klinechart_dmi",
        Indicator::Emv { .. } => "klinechart_emv",
        Indicator::Mtm { .. } => "klinechart_mtm",
        Indicator::Obv { .. } => "klinechart_obv",
        Indicator::Pvt => "klinechart_pvt",
        Indicator::Psy { .. } => "klinechart_psy",
        Indicator::Roc { .. } => "klinechart_roc",
        Indicator::Sar { .. } => "klinechart_sar",
        Indicator::Trix { .. } => "klinechart_trix",
        Indicator::Vr { .. } => "klinechart_vr",
        Indicator::Wr { .. } => "klinechart_wr",
        Indicator::Ao { .. } => "klinechart_ao",
        Indicator::Avp => "klinechart_avp",
    }
}

/// The template for a `klinechart_*` kind name, with KLineChart's default parameters.
pub fn klinechart_indicator_for_kind_name(name: &str) -> Option<Indicator> {
    name.strip_prefix("klinechart_")
        .and_then(Indicator::from_name)
}

/// `IndicatorInfo::period` for a KLineChart binding: its first whole-number parameter, or 0.
pub(crate) fn klinechart_primary_period(indicator: &Indicator) -> usize {
    indicator
        .params()
        .into_iter()
        .find(|param| param.integer)
        .map_or(0, |param| param.value as usize)
}

/// The palette color of a line output; bars and dots are colored per row instead.
pub(crate) fn klinechart_output_color(
    indicator: &Indicator,
    output_index: usize,
) -> Option<&'static str> {
    let figures = indicator.figures();
    if figures.get(output_index) != Some(&Figure::Line) {
        return None;
    }
    let line_index = figures[..output_index]
        .iter()
        .filter(|figure| **figure == Figure::Line)
        .count();
    Some(KLINECHART_LINE_COLORS[line_index % KLINECHART_LINE_COLORS.len()])
}

/// KLineChart's presentation defaults for one output series. Applied when the binding is created
/// and again when chart styles are reset; everything stays overridable through series options.
pub(crate) fn apply_klinechart_output_style(
    series: &mut SeriesEntry,
    indicator: &Indicator,
    output_index: usize,
) {
    series.line_width = Some(KLINECHART_LINE_WIDTH);
    series.title_visible = false;
    series.last_value_visible = false;
    series.price_line_visible = false;
    if indicator.figures().get(output_index) == Some(&Figure::Circle) {
        series.line_visible = false;
        series.point_markers = true;
        series.point_markers_radius = Some(2.0);
    }
}

/// The value format KLineChart uses for the template. Price overlays keep the source's format,
/// which the binding copies when it is created.
pub(crate) fn apply_klinechart_value_format(series: &mut SeriesEntry, indicator: &Indicator) {
    match indicator.value_format() {
        ValueFormat::Price => {}
        ValueFormat::Decimals(decimals) => {
            series.price_format.kind = PriceFormatKind::Price;
            series.price_format.precision = decimals;
            series.price_format.min_move = 10f64.powi(-(decimals as i32));
        }
        ValueFormat::Volume => {
            series.price_format.kind = PriceFormatKind::Volume;
            series.price_format.precision = 2;
            series.price_format.min_move = 0.01;
        }
    }
}

/// How one bar or dot output is colored row by row, following the template's KLineChart
/// `styles` callback.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum KLineChartColorRule {
    /// `VOL` bars: up when the candle closed above its open, down below it, grey when flat.
    CandleDirection,
    /// `MACD` bars: colored by sign (grey at zero), outlined while rising.
    MacdColumn,
    /// `AO` bars: up and outlined while rising, down otherwise.
    AoColumn,
    /// `SAR` dots: up below the candle's midpoint, down above it.
    SarDot,
}

pub(crate) fn klinechart_color_rule(
    indicator: &Indicator,
    output_index: usize,
) -> Option<KLineChartColorRule> {
    match (indicator, indicator.figures().get(output_index)?) {
        (Indicator::Vol { .. }, Figure::Bar) => Some(KLineChartColorRule::CandleDirection),
        (Indicator::Macd { .. }, Figure::Bar) => Some(KLineChartColorRule::MacdColumn),
        (Indicator::Ao { .. }, Figure::Bar) => Some(KLineChartColorRule::AoColumn),
        (Indicator::Sar { .. }, Figure::Circle) => Some(KLineChartColorRule::SarDot),
        _ => None,
    }
}

/// KLineChart substitutes `Number.MIN_SAFE_INTEGER` for a missing previous value.
const MIN_SAFE_INTEGER: f64 = -9_007_199_254_740_991.0;

impl KLineChartColorRule {
    /// The color of one row. `candle` is the source bar as `[open, high, low, close]`.
    fn color(self, value: f64, previous: Option<f64>, candle: [f64; 4]) -> u32 {
        if !value.is_finite() {
            return POINT_COLOR_ABSENT;
        }
        let previous = previous
            .filter(|p| p.is_finite())
            .unwrap_or(MIN_SAFE_INTEGER);
        let [open, high, low, close] = candle;
        match self {
            Self::CandleDirection => {
                if close > open {
                    UP_FILL
                } else if close < open {
                    DOWN_FILL
                } else {
                    NO_CHANGE
                }
            }
            Self::MacdColumn => {
                let rising = previous < value;
                if value > 0.0 {
                    if rising {
                        UP_HOLLOW
                    } else {
                        UP_FILL
                    }
                } else if value < 0.0 {
                    if rising {
                        DOWN_HOLLOW
                    } else {
                        DOWN_FILL
                    }
                } else {
                    NO_CHANGE
                }
            }
            Self::AoColumn => {
                if value > previous {
                    UP_HOLLOW
                } else {
                    DOWN_FILL
                }
            }
            Self::SarDot => {
                if value < (high + low) / 2.0 {
                    UP_FILL
                } else {
                    DOWN_FILL
                }
            }
        }
    }
}

impl ChartEngine {
    /// Add a KLineChart indicator (see [`aeris_charts_indicators::klinechart::Indicator`]) bound to
    /// `source`, returning its output series in output order. `volume_source` is required by
    /// `VOL`, `OBV`, `PVT`, `EMV`, `VR`, and `AVP` and must be omitted otherwise. An invalid
    /// definition returns no outputs and leaves the chart unchanged.
    pub fn add_klinechart_indicator(
        &mut self,
        source: SeriesId,
        indicator: Indicator,
        volume_source: Option<SeriesId>,
    ) -> Vec<SeriesId> {
        self.add_indicator_kind(source, IndicatorKind::KLineChart(indicator), volume_source)
    }

    /// Turns bar outputs into histograms and moves pane templates into a pane of their own.
    pub(crate) fn lay_out_klinechart_outputs(&mut self, indicator: &Indicator, ids: &[SeriesId]) {
        for (&id, figure) in ids.iter().zip(indicator.figures()) {
            if figure == Figure::Bar {
                self.convert_series_kind(id, SeriesKind::Histogram);
            }
        }
        if indicator.placement() == Placement::Pane {
            self.place_outputs_in_oscillator_pane(ids);
        }
    }

    /// Recolors the rows of a bar or dot output from `from_row` (all rows when `full`), after the
    /// output's values changed.
    pub(crate) fn color_klinechart_output(
        &mut self,
        rule: KLineChartColorRule,
        source: SeriesId,
        output: SeriesId,
        from_row: usize,
        full: bool,
    ) {
        let (from, colors) = {
            let Some((_, output_values)) = self.data.series_data(output) else {
                return;
            };
            let values = output_values[3];
            let Some((_, bars)) = self.data.series_data(source) else {
                return;
            };
            // Outputs are aligned to the end of their source.
            let Some(offset) = bars[3].len().checked_sub(values.len()) else {
                return;
            };
            let from = if full { 0 } else { from_row.min(values.len()) };
            let colors = (from..values.len())
                .map(|row| {
                    let source_row = offset + row;
                    let candle = [
                        bars[0][source_row],
                        bars[1][source_row],
                        bars[2][source_row],
                        bars[3][source_row],
                    ];
                    let previous = row.checked_sub(1).map(|previous| values[previous]);
                    rule.color(values[row], previous, candle)
                })
                .collect::<Vec<_>>();
            (from, colors)
        };
        if full {
            self.data
                .set_point_colors(output, [Some(colors), None, None]);
        } else {
            for (offset, color) in colors.into_iter().enumerate() {
                self.data
                    .set_point_color(output, PointColorChannel::Body, from + offset, color);
            }
        }
    }
}

#[cfg(test)]
mod tests;
