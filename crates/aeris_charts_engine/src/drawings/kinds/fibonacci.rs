//! B8 Fibonacci options and legacy defaults. Upstream renders every Fibonacci tool (retracement,
//! extension, channel, time zones, trend time, speed fan and arcs, circles, spiral, wedge) from
//! its catalog spec and the flat level fields; this module keeps the fork's public option block
//! ([`FibonacciToolOptions`]) and, for documents the fork wrote, its pre-merge kind defaults
//! ([`legacy_defaults`]).

use aeris_charts_render::draw_list::LineStyle;

use super::super::Drawing;
use crate::{DrawingKind, DrawingLevel, FIBONACCI_RATIOS, FIBONACCI_TIME_ZONES};

/// Horizontal placement of level labels. Horizontal and sloped levels: beyond the line's left
/// end, centered on it, or beyond its right end (inside at the pane edge when that end is
/// extended). Vertical levels: left of, centered on, or right of the line.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FibonacciLabelHAlign {
    Left,
    Center,
    Right,
}

/// Vertical placement of level labels. Horizontal and sloped levels: above, on, or below the
/// line. Vertical levels: at the pane's top, middle, or bottom.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FibonacciLabelVAlign {
    Top,
    Middle,
    Bottom,
}

/// The fork's Fibonacci options (`tool_options.fibonacci`); absent fields keep their defaults.
/// Upstream renders every Fibonacci tool from the flat level fields: `reverse`, `show_prices`,
/// `log_scale`, `show_levels`/`levels_as_percent`, and `label_h_align` are input aliases of
/// `level_reverse`, `level_show_prices`, `level_log_scale`, `level_show_values`/
/// `level_show_percents`, and `level_label_align` (see
/// `drawing_contract::take_legacy_flat_options`); the other fields are stored but not rendered.
/// Their defaults are upstream's look (no trend line, no fan grid), so a block a patch creates
/// for one key switches nothing else on; documents the fork wrote get the fork's values through
/// `kinds::legacy_fork_tool_options`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct FibonacciToolOptions {
    /// Swap the ends levels 0 and 1 sit at (retracement, extension, channel, fan), project time
    /// zones backward, or turn the spiral counterclockwise. Default false.
    pub reverse: bool,
    /// Show level values in labels. Default true.
    pub show_levels: bool,
    /// Show level prices in labels (retracement, extension). Default true.
    pub show_prices: bool,
    /// Show level values as percents (`61.8%` instead of `0.618`). Default false.
    pub levels_as_percent: bool,
    /// Interpolate price levels in log space (retracement, extension, channel). Default false.
    pub log_scale: bool,
    /// Show the dashed trend line through the anchors. Default false (the fork's default, true,
    /// reaches documents it wrote).
    pub trend_line: bool,
    /// Show the speed resistance fan's grid. Default false (the fork's default, true, reaches
    /// documents it wrote).
    pub grid: bool,
    /// Draw speed resistance arcs as full circles. Default false.
    pub full_circles: bool,
    /// Level label placement; `None` is the tool's default (left for price levels, right for
    /// time levels).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label_h_align: Option<FibonacciLabelHAlign>,
    /// Level label placement; `None` is the tool's default (middle for price levels, bottom for
    /// time levels).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label_v_align: Option<FibonacciLabelVAlign>,
}

impl Default for FibonacciToolOptions {
    fn default() -> Self {
        Self {
            reverse: false,
            show_levels: true,
            show_prices: true,
            levels_as_percent: false,
            log_scale: false,
            trend_line: false,
            grid: false,
            full_circles: false,
            label_h_align: None,
            label_v_align: None,
        }
    }
}

/// Neutral gray of the fork's trend line and levels 0 and 1.
const NEUTRAL: &str = "#787b86";
/// Level colors in the order of [`FIBONACCI_RATIOS`] (0, 0.236, 0.382, 0.5, 0.618, 0.786, 1,
/// 1.618, 2.618, 3.618, 4.236): TradingView's retracement palette. Other values cycle it by list
/// position.
const PALETTE: [&str; 11] = [
    "#787b86", "#f23645", "#ff9800", "#4caf50", "#089981", "#00bcd4", "#787b86", "#2962ff",
    "#f23645", "#9c27b0", "#e91e63",
];
/// Trend-based Fibonacci time ratios of the first leg's duration.
const TREND_TIME_RATIOS: [f64; 11] = [
    0.0, 0.382, 0.5, 0.618, 1.0, 1.382, 1.618, 2.0, 2.382, 2.618, 3.0,
];
/// Speed resistance fan ratios (price and time rays).
const SPEED_RESISTANCE_RATIOS: [f64; 7] = [0.0, 0.25, 0.382, 0.5, 0.618, 0.75, 1.0];
/// Arc and circle radii as ratios of the anchors' distance.
const ARC_RATIOS: [f64; 10] = [
    0.236, 0.382, 0.5, 0.618, 0.786, 1.0, 1.618, 2.618, 3.618, 4.236,
];
/// Wedge arc radii as ratios of the first edge.
const WEDGE_RATIOS: [f64; 6] = [0.236, 0.382, 0.5, 0.618, 0.786, 1.0];

/// The fork's default level ratios of each Fibonacci tool.
fn default_ratios(kind: DrawingKind) -> Vec<f64> {
    match kind {
        DrawingKind::FibonacciRetracement
        | DrawingKind::FibonacciExtension
        | DrawingKind::FibonacciChannel => FIBONACCI_RATIOS.to_vec(),
        DrawingKind::FibonacciTimeZones => FIBONACCI_TIME_ZONES
            .iter()
            .map(|&zone| f64::from(zone))
            .collect(),
        DrawingKind::FibonacciTrendTime => TREND_TIME_RATIOS.to_vec(),
        DrawingKind::FibonacciSpeedFan => SPEED_RESISTANCE_RATIOS.to_vec(),
        DrawingKind::FibonacciSpeedArcs | DrawingKind::FibonacciCircles => ARC_RATIOS.to_vec(),
        DrawingKind::FibonacciWedge => WEDGE_RATIOS.to_vec(),
        _ => Vec::new(),
    }
}

/// A fork default level: visible, solid, labeled, and filling the band below it.
fn default_level(value: f64, index: usize) -> DrawingLevel {
    let color = FIBONACCI_RATIOS
        .iter()
        .position(|&ratio| ratio == value)
        .map_or(PALETTE[index % PALETTE.len()], |position| PALETTE[position]);
    DrawingLevel {
        fill_between: true,
        ..DrawingLevel::at(value, color)
    }
}

/// The fork's pre-merge defaults of the Fibonacci tools (see
/// [`super::apply_legacy_fork_defaults`]): its palette levels, the band fill (not for time zones
/// or the spiral), and the neutral dashed auxiliary line (solid on the wedge, the drawing color on
/// the spiral).
pub(super) fn legacy_defaults(drawing: &mut Drawing) {
    if !matches!(
        drawing.kind,
        DrawingKind::FibonacciRetracement
            | DrawingKind::FibonacciExtension
            | DrawingKind::FibonacciChannel
            | DrawingKind::FibonacciTimeZones
            | DrawingKind::FibonacciTrendTime
            | DrawingKind::FibonacciSpeedFan
            | DrawingKind::FibonacciSpeedArcs
            | DrawingKind::FibonacciCircles
            | DrawingKind::FibonacciSpiral
            | DrawingKind::FibonacciWedge
    ) {
        return;
    }
    drawing.levels = default_ratios(drawing.kind)
        .into_iter()
        .enumerate()
        .map(|(index, value)| default_level(value, index))
        .collect();
    drawing.fill_enabled = !matches!(
        drawing.kind,
        DrawingKind::FibonacciTimeZones | DrawingKind::FibonacciSpiral
    );
    match drawing.kind {
        // The spiral is its own stroke in the drawing color.
        DrawingKind::FibonacciSpiral => {}
        // The wedge's edges are its outline.
        DrawingKind::FibonacciWedge => drawing.color = NEUTRAL.to_string(),
        _ => {
            drawing.color = NEUTRAL.to_string();
            drawing.style = LineStyle::Dashed;
        }
    }
}

/// The fork's unstored `tool_options.fibonacci` defaults (see
/// `kinds::legacy_fork_tool_options`): the dashed trend line on the seven tools that drew one,
/// the speed resistance fan's grid, and each tool's own vertical label placement (the fork wrote
/// `label_v_align` only when set: middle on price levels, bottom on time levels).
pub(super) fn legacy_tool_options(kind: DrawingKind) -> Option<(&'static str, serde_json::Value)> {
    let block = match kind {
        DrawingKind::FibonacciRetracement | DrawingKind::FibonacciExtension => {
            serde_json::json!({"trend_line": true, "label_v_align": "middle"})
        }
        DrawingKind::FibonacciChannel => serde_json::json!({"label_v_align": "middle"}),
        DrawingKind::FibonacciTimeZones | DrawingKind::FibonacciTrendTime => {
            serde_json::json!({"trend_line": true, "label_v_align": "bottom"})
        }
        DrawingKind::FibonacciSpeedFan => serde_json::json!({"grid": true}),
        DrawingKind::FibonacciSpeedArcs
        | DrawingKind::FibonacciCircles
        | DrawingKind::FibonacciSpiral => serde_json::json!({"trend_line": true}),
        _ => return None,
    };
    Some(("fibonacci", block))
}

#[cfg(test)]
mod tests;
