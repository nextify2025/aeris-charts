//! B8 Fibonacci options, legacy defaults, and the fork presentation layered on upstream's arms.
//! Upstream renders every Fibonacci tool (retracement, extension, channel, time zones, trend time,
//! speed fan and arcs, circles, spiral, wedge) from its catalog spec and the flat level fields
//! (`geometry.rs` resolver, the Fibonacci, time-level and Fibonacci-arc frame arms, their hit
//! code). This module keeps the fork's public option block ([`FibonacciToolOptions`]), its
//! pre-merge kind defaults for documents the fork wrote ([`legacy_defaults`],
//! [`legacy_tool_options`]), and the stored options' reading for those arms: the trend line
//! ([`trend_line`]), the fan grid ([`draws_grid`]), full circles (`DrawingGeometryOptions`), the
//! vertical label placement ([`label_v_align`]), the golden spiral of an empty spiral
//! ([`phi_spiral`]), the precise rings of a stored block ([`precise_rings`]), the labels' culling
//! pad ([`upstream_decoration_extent`]) and schema rows ([`extend_upstream_schema`]). Level labels
//! and, while selected, bands are body hit targets of every level arm.

use aeris_charts_render::draw_list::{LineStyle, TextAlign};
use aeris_charts_render::shape::Point;

use super::super::Drawing;
use crate::{
    ChartEngine, DrawingKind, DrawingLevel, DrawingPropertyDescriptor, DrawingPropertyType,
    FIBONACCI_RATIOS, FIBONACCI_TIME_ZONES,
};

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
/// Upstream renders every Fibonacci tool from the flat level fields: `reverse` (except the
/// spiral's), `show_prices`, `log_scale`, `show_levels`/`levels_as_percent`, and `label_h_align`
/// are input aliases of `level_reverse`, `level_show_prices`, `level_log_scale`,
/// `level_show_values`/`level_show_percents`, and `level_label_align` (see
/// `drawing_contract::take_legacy_flat_options`); the other fields are rendered on upstream's
/// arms. Their defaults are upstream's look (no fan grid, half arcs, labels above their lines), so
/// a block a patch creates for one key switches nothing else on ([`default_options`]: its trend
/// line starts on only on the extension and the trend-based time, standing in for upstream's
/// guides); documents the fork wrote get the fork's values through
/// `kinds::legacy_fork_tool_options`. A stored block also tessellates the ring tools' arcs within
/// a tenth of a pixel over the part the pane shows ([`precise_rings`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct FibonacciToolOptions {
    /// Swap the ends levels 0 and 1 sit at (retracement, extension, channel, fan), project time
    /// zones backward, or turn the golden spiral (an empty spiral's) counterclockwise. Default
    /// false.
    pub reverse: bool,
    /// Show level values in labels. Default true.
    pub show_levels: bool,
    /// Show level prices in labels (retracement, extension). Default true.
    pub show_prices: bool,
    /// Show level values as percents (`61.8%` instead of `0.618`). Default false.
    pub levels_as_percent: bool,
    /// Interpolate price levels in log space (retracement, extension, channel). Default false.
    pub log_scale: bool,
    /// Stroke the trend line through the anchors in the drawing's own stroke (the spiral's as a
    /// 1 CSS px dashed line; the circles' as the level-1 diameter through both anchors):
    /// retracement, extension, time zones, trend time, speed arcs, circles, spiral. Default false,
    /// true on the extension and trend time ([`default_options`]; the fork's default, true,
    /// reaches documents it wrote).
    pub trend_line: bool,
    /// Show the speed resistance fan's grid: each visible level's horizontal and vertical line
    /// at that level's ratio of the anchors' box (past it for levels outside 0..1). Default false (the fork's default, true, reaches documents it
    /// wrote).
    pub grid: bool,
    /// Draw speed resistance arcs as full circles. Default false.
    pub full_circles: bool,
    /// Level label placement; `None` is the tool's default (left for price levels, right for
    /// time levels).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label_h_align: Option<FibonacciLabelHAlign>,
    /// Vertical level label placement (retracement, extension, channel, time zones, trend time);
    /// `None` is upstream's, `top` (above the line; time labels at the pane's top). `middle`
    /// centers a price label on its line beyond the line's end (`level_label_align` picks the
    /// end), `bottom` puts it below; time labels sit at the pane's middle or bottom. The fork's
    /// defaults (middle for price levels, bottom for time levels) reach documents it wrote.
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

/// Gap between a level line's end and a label beside it (the middle row), CSS px.
const LABEL_GAP: f64 = 6.0;
/// Width of the spiral's dashed trend line, CSS px.
const SPIRAL_TREND_WIDTH: f64 = 1.0;

/// The stored `tool_options.fibonacci` block of `drawing`, or its defaults.
pub(crate) fn options(drawing: &Drawing) -> FibonacciToolOptions {
    drawing
        .tool_options
        .fibonacci
        .unwrap_or_else(|| default_options(drawing.kind))
}

/// The `tool_options.fibonacci` block of a `kind` drawing without a stored one, and the block a
/// patch that starts one begins from: the defaults, with the trend line on for the extension and
/// the trend-based time. Without a stored block those two draw upstream's construction guides
/// through their anchors ([`draws_guides`]); a patch that stores one switches them to the trend
/// line, which therefore starts on so the legs stay visible.
pub(crate) fn default_options(kind: DrawingKind) -> FibonacciToolOptions {
    FibonacciToolOptions {
        trend_line: matches!(
            kind,
            DrawingKind::FibonacciExtension | DrawingKind::FibonacciTrendTime
        ),
        ..FibonacciToolOptions::default()
    }
}

/// `drawing` paints and hits upstream's construction guides (`ResolvedDrawingGeometry::guides`):
/// every drawing except a Fibonacci tool with a stored `tool_options.fibonacci` block, which
/// keeps the fork's trend line ([`trend_line`]: its own style, above the band fills) in their
/// place, so a document the fork wrote keeps its legs (the block-presence rule).
pub(crate) fn draws_guides(drawing: &Drawing) -> bool {
    !(drawing.kind.is_fibonacci() && drawing.tool_options.fibonacci.is_some())
}

/// The trend line `drawing` strokes through its anchors `px` (caller px) while
/// `tool_options.fibonacci.trend_line` is on, as up to three points and their count: the first
/// leg of the retracement, time zones, speed arcs and spiral, both legs of the extension and the
/// trend-based time (only with a stored block: without one, upstream's guides are their legs,
/// see [`draws_guides`]), and the circles' level-1 diameter from the first anchor through the
/// center (the second anchor). `None` for the other kinds.
pub(crate) fn trend_line(drawing: &Drawing, px: &[Point]) -> Option<([Point; 3], usize)> {
    if !drawing.kind.is_fibonacci() || !options(drawing).trend_line {
        return None;
    }
    let (&a, &b) = (px.first()?, px.get(1)?);
    match drawing.kind {
        DrawingKind::FibonacciRetracement
        | DrawingKind::FibonacciTimeZones
        | DrawingKind::FibonacciSpeedArcs
        | DrawingKind::FibonacciSpiral => Some(([a, b, b], 2)),
        DrawingKind::FibonacciExtension | DrawingKind::FibonacciTrendTime
            if drawing.tool_options.fibonacci.is_some() =>
        {
            Some(([a, b, *px.get(2)?], 3))
        }
        DrawingKind::FibonacciCircles => Some(([a, (2.0 * b.0 - a.0, 2.0 * b.1 - a.1), b], 2)),
        _ => None,
    }
}

/// The trend line's stroke width in CSS px and style: the drawing's own, except the spiral's
/// 1 CSS px dashed decoration.
pub(crate) fn trend_stroke(drawing: &Drawing) -> (f64, LineStyle) {
    if drawing.kind == DrawingKind::FibonacciSpiral {
        (SPIRAL_TREND_WIDTH, LineStyle::Dashed)
    } else {
        (drawing.width, drawing.style)
    }
}

/// The speed resistance fan draws its level grid (`tool_options.fibonacci.grid`).
pub(crate) fn draws_grid(drawing: &Drawing) -> bool {
    drawing.kind == DrawingKind::FibonacciSpeedFan && options(drawing).grid
}

/// The vertical placement of `drawing`'s level labels: the stored `label_v_align` on the five
/// tools with horizontal, sloped or vertical level lines, upstream's top row everywhere else.
pub(crate) fn label_v_align(drawing: &Drawing) -> FibonacciLabelVAlign {
    match drawing.kind {
        DrawingKind::FibonacciRetracement
        | DrawingKind::FibonacciExtension
        | DrawingKind::FibonacciChannel
        | DrawingKind::FibonacciTimeZones
        | DrawingKind::FibonacciTrendTime => options(drawing)
            .label_v_align
            .unwrap_or(FibonacciLabelVAlign::Top),
        _ => FibonacciLabelVAlign::Top,
    }
}

/// A spiral without levels (the fork's spiral, which documents it wrote restore as) paints the
/// golden spiral through its second anchor instead of upstream's level spirals, which an empty
/// list leaves invisible.
pub(crate) fn phi_spiral(drawing: &Drawing) -> bool {
    drawing.kind == DrawingKind::FibonacciSpiral && drawing.levels.is_empty()
}

/// A ring tool (speed arcs, circles, wedge) that stores the `tool_options.fibonacci` block (every
/// document the fork wrote; the wedge's carries it empty) tessellates its rings and bands within
/// a tenth of a pixel over the part of the arc the pane shows (`geometry::Rings`); otherwise
/// upstream's rings take whole circles of `geometry::arc_segments` chords at the same tolerance.
pub(crate) fn precise_rings(drawing: &Drawing) -> bool {
    matches!(
        drawing.kind,
        DrawingKind::FibonacciSpeedArcs
            | DrawingKind::FibonacciCircles
            | DrawingKind::FibonacciWedge
    ) && drawing.tool_options.fibonacci.is_some()
}

/// The largest effective value among `drawing`'s visible levels, 0 without a positive one: the
/// ring tools' outermost radius in units of the anchors' distance.
pub(crate) fn largest_level(drawing: &Drawing) -> f64 {
    drawing
        .levels
        .iter()
        .filter(|level| level.visible)
        .map(|level| drawing.level_value(level.value).max(0.0))
        .fold(0.0, f64::max)
}

/// The dash period of a stroke `width` caller px wide in `style`, caller px; 0 when solid.
pub(crate) fn dash_period(style: LineStyle, width: f64) -> f64 {
    style
        .dash_pattern(width as f32)
        .iter()
        .map(|&length| f64::from(length))
        .sum()
}

/// A level label as the level arms paint it (`Prim::Text`): `x` the `align` edge, `y` the
/// vertical center, both in caller px. Frame lowering and hit testing resolve it alike.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct LevelLabel {
    pub(crate) x: f64,
    pub(crate) y: f64,
    pub(crate) text: String,
    pub(crate) align: TextAlign,
}

/// The middle-row placement of a price level label on the line `(a, b)` (caller px): beyond the
/// line's left or right end (`level_label_align`), inside at an end extended to the pane edge
/// (`extended`, left and right of the screen), centered on the end's y; or centered on the line's
/// midpoint. `gap` is [`LABEL_GAP`] in caller px.
pub(crate) fn middle_label_anchor(
    drawing: &Drawing,
    (a, b): (Point, Point),
    extended: (bool, bool),
    gap: f64,
) -> (f64, f64, TextAlign) {
    let (left, right) = if a.0 <= b.0 { (a, b) } else { (b, a) };
    match drawing.level_label_align.as_str() {
        "left" if extended.0 => (left.0 + gap, left.1, TextAlign::Left),
        "left" => (left.0 - gap, left.1, TextAlign::Right),
        "center" => ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0, TextAlign::Center),
        _ if extended.1 => (right.0 - gap, right.1, TextAlign::Right),
        _ => (right.0 + gap, right.1, TextAlign::Left),
    }
}

/// [`LABEL_GAP`] in caller px at `scale` caller px per CSS px.
pub(crate) fn label_gap(scale: f64) -> f64 {
    LABEL_GAP * scale
}

/// Conservative CSS-px reach of the level labels beyond a level arm's lines (the culling pad, see
/// [`super::upstream_decoration_extent`]): the widest label plus the gap and four ems of slack,
/// since price text follows the scale's formatter between text-key refreshes. Every kind the
/// Fibonacci, time-level and Fibonacci-arc arms paint (the Gann fan shares the Fibonacci arm);
/// 0 without a labeled level.
pub(crate) fn upstream_decoration_extent(engine: &ChartEngine, drawing: &Drawing) -> f64 {
    if !drawing.kind.is_fibonacci() && drawing.kind != DrawingKind::GannFan {
        return 0.0;
    }
    let layout = &engine.options.get().layout;
    let size = layout.font_size;
    let (zero, one) = match drawing.points.as_slice() {
        [a, b, c, ..] if drawing.kind == DrawingKind::FibonacciExtension => {
            (c.price, c.price + b.price - a.price)
        }
        [a, b, ..] => (a.price, b.price),
        _ => (0.0, 0.0),
    };
    let width = drawing
        .levels
        .iter()
        .filter(|level| level.visible && level.label_visible)
        .filter_map(|level| {
            let price = zero + (one - zero) * drawing.level_value(level.value);
            engine.drawing_level_label(drawing, level.value, Some(price))
        })
        .map(|text| {
            engine.measure_text_run(
                &text,
                size,
                &layout.font_family,
                drawing.text_weight.unwrap_or(400),
                drawing.text_italic,
            )
        })
        .fold(0.0_f64, f64::max);
    if width <= 0.0 {
        return 0.0;
    }
    LABEL_GAP + width + 4.0 * size
}

fn descriptor(
    name: &str,
    property_type: DrawingPropertyType,
    default: serde_json::Value,
) -> DrawingPropertyDescriptor {
    crate::drawing_contract::descriptor(
        format!("tool_options.fibonacci.{name}"),
        property_type,
        default,
    )
}

/// The `tool_options.fibonacci` descriptors the Fibonacci kinds read on upstream's arms (see
/// [`super::extend_upstream_schema`]): the trend line on its seven tools, the fan's grid, the
/// speed arcs' full circles, the vertical label placement on the five line-level tools, and the
/// golden spiral's turn. Defaults follow `template` (a kind's `Drawing::new`, which has no block).
pub(crate) fn extend_upstream_schema(
    kind: DrawingKind,
    template: &Drawing,
    properties: &mut Vec<DrawingPropertyDescriptor>,
) {
    if !kind.is_fibonacci() {
        return;
    }
    let resolved = options(template);
    let boolean = |name: &str, value: bool| {
        descriptor(name, DrawingPropertyType::Boolean, serde_json::json!(value))
    };
    if matches!(
        kind,
        DrawingKind::FibonacciRetracement
            | DrawingKind::FibonacciExtension
            | DrawingKind::FibonacciTimeZones
            | DrawingKind::FibonacciTrendTime
            | DrawingKind::FibonacciSpeedArcs
            | DrawingKind::FibonacciCircles
            | DrawingKind::FibonacciSpiral
    ) {
        properties.push(boolean("trend_line", resolved.trend_line));
    }
    match kind {
        DrawingKind::FibonacciSpeedFan => properties.push(boolean("grid", resolved.grid)),
        DrawingKind::FibonacciSpeedArcs => {
            properties.push(boolean("full_circles", resolved.full_circles));
        }
        DrawingKind::FibonacciSpiral => properties.push(boolean("reverse", resolved.reverse)),
        DrawingKind::FibonacciRetracement
        | DrawingKind::FibonacciExtension
        | DrawingKind::FibonacciChannel
        | DrawingKind::FibonacciTimeZones
        | DrawingKind::FibonacciTrendTime => {
            let align = resolved.label_v_align.unwrap_or(FibonacciLabelVAlign::Top);
            properties.push(DrawingPropertyDescriptor {
                enum_values: ["top", "middle", "bottom"].map(str::to_string).to_vec(),
                ..descriptor(
                    "label_v_align",
                    DrawingPropertyType::Enum,
                    serde_json::to_value(align).unwrap_or_default(),
                )
            });
        }
        _ => {}
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
    if !drawing.kind.is_fibonacci() {
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
/// `label_v_align` only when set: middle on price levels, bottom on time levels). Every ring tool
/// gets the block, which selects the fork's precise rings ([`precise_rings`]); the wedge's is
/// empty.
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
        DrawingKind::FibonacciWedge => serde_json::json!({}),
        _ => return None,
    };
    Some(("fibonacci", block))
}

#[cfg(test)]
mod tests;
