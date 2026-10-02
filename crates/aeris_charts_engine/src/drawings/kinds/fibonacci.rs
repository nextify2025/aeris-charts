//! B8 Fibonacci family (wire ids 64..=95): retracement, trend-based extension, channel, time
//! zones, trend-based time, speed resistance fan and arcs, circles, spiral, and wedge.
//!
//! Every tool except the spiral paints the drawing's level list (`Drawing::levels`: value,
//! color, visibility, line style, per-level band fill, label visibility). Visible levels sort by
//! value; the band between two neighbours takes the upper level's fill (its `fill_color`, else
//! its color at 20% alpha) when `fill_enabled` (the tool's background switch) and that level's
//! `fill_between` are on. The drawing's own stroke (color, width, style) is the tool's auxiliary
//! line: the dashed trend line through the anchors, the fan's grid, the wedge's edges, or the
//! spiral itself. Family options live in `tool_options.fibonacci` ([`FibonacciToolOptions`]).
//!
//! Geometry:
//! - Price levels (retracement, extension, channel) are computed in price space (log space with
//!   `log_scale`) and mapped through the drawing's scale, so they sit on exact prices on every
//!   scale mode. Level 0 is the retracement's second anchor and level 1 its first; the extension
//!   projects the first leg's move from the third anchor; the channel offsets the first leg's
//!   line toward the third anchor. `reverse` swaps the ends levels 0 and 1 sit at.
//! - Time levels (time zones, trend-based time) are vertical lines at ratio multiples of the
//!   first leg's time distance, from the first anchor (zones) or the third (trend-based).
//! - Screen-space tools (fan, arcs, circles, spiral, wedge) are resolved from the anchors' px,
//!   so circles stay circular under any zoom; their semantic bounds are unbounded. Arcs,
//!   circles, and the wedge cull and hit-test by their paint box (the largest visible ring
//!   around its center, see [`paint_bounds`]); the fan's rays and the spiral reach the pane edge
//!   and keep the whole pane.

use aeris_charts_render::color::Color;
use aeris_charts_render::draw_list::LineStyle;
use aeris_charts_render::shape::{self, Point, Rect};

use super::super::parts::{DrawingParts, PartContext, PartLabel, PartStroke, CURVE_TOLERANCE};
use super::super::tools::{
    DrawingAnchorLink, DrawingHandleMode, DrawingLogicalExtent, DrawingMovementAxis,
    DrawingPlacement, DrawingPriceExtent, DrawingStraightenMode, DrawingTextLayout,
    DrawingToolSpec,
};
use super::super::{Drawing, DrawingPoint, DrawingTextHAlign, DrawingTextVAlign};
use super::{DrawingFamily, FamilyBounds};
use crate::{
    ChartEngine, DrawingKind, DrawingKindOptions, DrawingLevel, DrawingPropertyDescriptor,
    DrawingPropertyType, FIBONACCI_RATIOS, FIBONACCI_TIME_ZONES, MAX_DRAWING_LEVELS,
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

impl FibonacciLabelHAlign {
    fn name(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Center => "center",
            Self::Right => "right",
        }
    }
}

impl FibonacciLabelVAlign {
    fn name(self) -> &'static str {
        match self {
            Self::Top => "top",
            Self::Middle => "middle",
            Self::Bottom => "bottom",
        }
    }
}

/// Fibonacci-family options (`tool_options.fibonacci`); absent fields keep their defaults.
/// Each tool reads the fields its schema lists (see `docs/Public_api.md`).
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
    /// Show the dashed trend line through the anchors. Default true.
    pub trend_line: bool,
    /// Show the speed resistance fan's grid. Default true.
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
            trend_line: true,
            grid: true,
            full_circles: false,
            label_h_align: None,
            label_v_align: None,
        }
    }
}

/// Neutral gray of the trend line, the fan grid, and levels 0 and 1.
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
/// Gap between a level line's end (or a vertical line) and its label, in CSS px.
const LABEL_GAP: f64 = 6.0;
/// Gap between a level line and a label above or below it, in CSS px.
const LABEL_LIFT: f64 = 2.0;
/// The spiral's inner end, in CSS px from its center.
const SPIRAL_MIN_RADIUS: f64 = 0.5;
/// Upper bound on the spiral's quarter turns (φ^128 ≈ 1.6e26 px: beyond any pane).
const MAX_SPIRAL_QUARTERS: usize = 128;
/// The golden ratio φ = (1 + √5) / 2: the spiral grows by φ every quarter turn. It is computed
/// instead of written as a literal because `std::f64::consts::GOLDEN_RATIO` is stable only from
/// Rust 1.99, hosts pin their own toolchain, and Clippy denies the literal where the constant exists.
fn golden_ratio() -> f64 {
    (1.0 + 5.0_f64.sqrt()) / 2.0
}

/// Shared two-anchor Fibonacci behavior; every spec below overrides its identity.
const FIB_TOOL: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::FibRetracement,
    wire_id: 64,
    name: "fib_retracement",
    placement: DrawingPlacement::ClickAnchors { count: 2 },
    handles: DrawingHandleMode::Anchors,
    movement_axis: DrawingMovementAxis::Both,
    straighten: DrawingStraightenMode::Segment45,
    logical_extent: DrawingLogicalExtent::Finite,
    price_extent: DrawingPriceExtent::Finite,
    bounds_padding_ratio: 0.0,
    default_width: 1.0,
    requests_text_editor: false,
    family: Some(&FAMILY),
    text_layout: DrawingTextLayout::Box,
    axis_price_label: false,
    grid_snap: false,
    anchor_link: DrawingAnchorLink::None,
};

/// Screen-space tools reach beyond their anchors by radii that depend on the zoom, so no
/// semantic box bounds them: the ring tools cull by their paint box and the rest by the pane
/// (their parts skip what lies outside it).
const SCREEN_TOOL: DrawingToolSpec = DrawingToolSpec {
    logical_extent: DrawingLogicalExtent::Full,
    price_extent: DrawingPriceExtent::Full,
    ..FIB_TOOL
};

pub(crate) const FIB_RETRACEMENT: DrawingToolSpec = FIB_TOOL;

pub(crate) const TREND_BASED_FIB_EXTENSION: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::TrendBasedFibExtension,
    wire_id: 65,
    name: "trend_based_fib_extension",
    placement: DrawingPlacement::ClickAnchors { count: 3 },
    ..FIB_TOOL
};

pub(crate) const FIB_CHANNEL: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::FibChannel,
    wire_id: 66,
    name: "fib_channel",
    placement: DrawingPlacement::ClickAnchors { count: 3 },
    ..FIB_TOOL
};

pub(crate) const FIB_TIME_ZONE: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::FibTimeZone,
    wire_id: 67,
    name: "fib_time_zone",
    ..FIB_TOOL
};

pub(crate) const TREND_BASED_FIB_TIME: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::TrendBasedFibTime,
    wire_id: 68,
    name: "trend_based_fib_time",
    placement: DrawingPlacement::ClickAnchors { count: 3 },
    ..FIB_TOOL
};

pub(crate) const FIB_SPEED_RESISTANCE_FAN: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::FibSpeedResistanceFan,
    wire_id: 69,
    name: "fib_speed_resistance_fan",
    ..SCREEN_TOOL
};

pub(crate) const FIB_SPEED_RESISTANCE_ARCS: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::FibSpeedResistanceArcs,
    wire_id: 70,
    name: "fib_speed_resistance_arcs",
    ..SCREEN_TOOL
};

pub(crate) const FIB_CIRCLES: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::FibCircles,
    wire_id: 71,
    name: "fib_circles",
    ..SCREEN_TOOL
};

pub(crate) const FIB_SPIRAL: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::FibSpiral,
    wire_id: 72,
    name: "fib_spiral",
    ..SCREEN_TOOL
};

pub(crate) const FIB_WEDGE: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::FibWedge,
    wire_id: 73,
    name: "fib_wedge",
    placement: DrawingPlacement::ClickAnchors { count: 3 },
    ..SCREEN_TOOL
};

pub(crate) static FAMILY: DrawingFamily = {
    let mut family = DrawingFamily::new(build_parts, kind_options);
    family.apply_defaults = apply_defaults;
    family.bounds = bounds;
    family.decoration_extent = decoration_extent;
    family.extend_schema = extend_schema;
    family.paint_bounds = paint_bounds;
    family.partial_preview = true;
    family
};

/// The ring tools' center and unit radius in the px of their anchors: arcs and the wedge turn
/// around the first anchor at the anchors' distance, circles around the anchors' midpoint at half
/// of it. `None` for every other tool.
fn ring_frame(kind: DrawingKind, px: &[Point]) -> Option<(Point, f64)> {
    let (&a, &b) = (px.first()?, px.get(1)?);
    let distance = (b.0 - a.0).hypot(b.1 - a.1);
    match kind {
        DrawingKind::FibSpeedResistanceArcs | DrawingKind::FibWedge => Some((a, distance)),
        DrawingKind::FibCircles => Some((shape::midpoint(a, b), distance / 2.0)),
        _ => None,
    }
}

/// Media-px box of a ring tool's strokes and fills: its anchors and the square around its center
/// reaching the largest visible level's radius (at least the unit, which the trend line and the
/// wedge's edges reach). Level labels pad through `decoration_extent`. The fan's rays and the
/// spiral reach the pane edge, so they keep the whole pane.
fn paint_bounds(_: &ChartEngine, drawing: &Drawing, px: &[Point]) -> Option<Rect> {
    let (center, unit) = ring_frame(drawing.kind, px)?;
    let largest = drawing
        .levels
        .iter()
        .take(MAX_DRAWING_LEVELS)
        .filter(|level| level.visible && level.value.is_finite() && level.value > 0.0)
        .map(|level| level.value)
        .fold(1.0_f64, f64::max);
    let radius = unit * largest;
    if !radius.is_finite() {
        return None;
    }
    let mut points = px.to_vec();
    points.push((center.0 - radius, center.1 - radius));
    points.push((center.0 + radius, center.1 + radius));
    Rect::bounding(&points)
}

/// The kind's default level ratios.
fn default_ratios(kind: DrawingKind) -> Vec<f64> {
    match kind {
        DrawingKind::FibRetracement
        | DrawingKind::TrendBasedFibExtension
        | DrawingKind::FibChannel => FIBONACCI_RATIOS.to_vec(),
        DrawingKind::FibTimeZone => FIBONACCI_TIME_ZONES
            .iter()
            .map(|&zone| f64::from(zone))
            .collect(),
        DrawingKind::TrendBasedFibTime => TREND_TIME_RATIOS.to_vec(),
        DrawingKind::FibSpeedResistanceFan => SPEED_RESISTANCE_RATIOS.to_vec(),
        DrawingKind::FibSpeedResistanceArcs | DrawingKind::FibCircles => ARC_RATIOS.to_vec(),
        DrawingKind::FibWedge => WEDGE_RATIOS.to_vec(),
        _ => Vec::new(),
    }
}

/// A default level: visible, solid, labeled, and filling the band below it.
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

fn apply_defaults(drawing: &mut Drawing) {
    drawing.levels = default_ratios(drawing.kind)
        .into_iter()
        .enumerate()
        .map(|(index, value)| default_level(value, index))
        .collect();
    drawing.fill_enabled = !matches!(
        drawing.kind,
        DrawingKind::FibTimeZone | DrawingKind::FibSpiral
    );
    match drawing.kind {
        // The spiral is its own stroke in the drawing color.
        DrawingKind::FibSpiral => {}
        // The wedge's edges are its outline.
        DrawingKind::FibWedge => drawing.color = NEUTRAL.to_string(),
        _ => {
            drawing.color = NEUTRAL.to_string();
            drawing.style = LineStyle::Dashed;
        }
    }
}

fn options(drawing: &Drawing) -> FibonacciToolOptions {
    drawing.tool_options.fibonacci.unwrap_or_default()
}

/// The options with the kind's label placement filled in: time levels label the bottom right of
/// their lines, every other tool the left end, centered on the line.
fn resolved(drawing: &Drawing) -> FibonacciToolOptions {
    let mut options = options(drawing);
    let time = matches!(
        drawing.kind,
        DrawingKind::FibTimeZone | DrawingKind::TrendBasedFibTime
    );
    options.label_h_align.get_or_insert(if time {
        FibonacciLabelHAlign::Right
    } else {
        FibonacciLabelHAlign::Left
    });
    options.label_v_align.get_or_insert(if time {
        FibonacciLabelVAlign::Bottom
    } else {
        FibonacciLabelVAlign::Middle
    });
    options
}

fn kind_options(drawing: &Drawing) -> DrawingKindOptions {
    DrawingKindOptions::Fibonacci(resolved(drawing))
}

/// The `tool_options.fibonacci` fields each tool reads.
fn option_names(kind: DrawingKind) -> &'static [&'static str] {
    match kind {
        DrawingKind::FibRetracement | DrawingKind::TrendBasedFibExtension => &[
            "reverse",
            "show_levels",
            "show_prices",
            "levels_as_percent",
            "log_scale",
            "trend_line",
            "label_h_align",
            "label_v_align",
        ],
        DrawingKind::FibChannel => &[
            "reverse",
            "show_levels",
            "levels_as_percent",
            "log_scale",
            "label_h_align",
            "label_v_align",
        ],
        DrawingKind::FibTimeZone => &[
            "reverse",
            "show_levels",
            "trend_line",
            "label_h_align",
            "label_v_align",
        ],
        DrawingKind::TrendBasedFibTime => &[
            "show_levels",
            "levels_as_percent",
            "trend_line",
            "label_h_align",
            "label_v_align",
        ],
        DrawingKind::FibSpeedResistanceFan => {
            &["reverse", "show_levels", "levels_as_percent", "grid"]
        }
        DrawingKind::FibSpeedResistanceArcs => &[
            "show_levels",
            "levels_as_percent",
            "full_circles",
            "trend_line",
        ],
        DrawingKind::FibCircles => &["show_levels", "levels_as_percent", "trend_line"],
        DrawingKind::FibSpiral => &["reverse", "trend_line"],
        DrawingKind::FibWedge => &["show_levels", "levels_as_percent"],
        _ => &[],
    }
}

fn extend_schema(template: &Drawing, properties: &mut Vec<DrawingPropertyDescriptor>) {
    let Ok(serde_json::Value::Object(defaults)) = serde_json::to_value(resolved(template)) else {
        return;
    };
    use FibonacciLabelHAlign as H;
    use FibonacciLabelVAlign as V;
    for &name in option_names(template.kind) {
        let enum_values = match name {
            "label_h_align" => [H::Left, H::Center, H::Right].map(H::name).to_vec(),
            "label_v_align" => [V::Top, V::Middle, V::Bottom].map(V::name).to_vec(),
            _ => Vec::new(),
        };
        properties.push(DrawingPropertyDescriptor {
            name: format!("tool_options.fibonacci.{name}"),
            property_type: if enum_values.is_empty() {
                DrawingPropertyType::Boolean
            } else {
                DrawingPropertyType::Enum
            },
            default: defaults.get(name).cloned().unwrap_or_default(),
            min: None,
            max: None,
            enum_values: enum_values.into_iter().map(str::to_string).collect(),
        });
    }
}

/// One visible level resolved for painting.
#[derive(Clone, Copy, Debug)]
struct Level {
    value: f64,
    color: Color,
    style: LineStyle,
    /// Fill of the band between this level and the previous visible one, when painted.
    fill: Option<Color>,
    label: bool,
}

impl Level {
    fn stroke(&self) -> PartStroke {
        PartStroke {
            color: Some(self.color),
            width: None,
            style: Some(self.style),
        }
    }
}

/// The drawing's visible levels (at most [`MAX_DRAWING_LEVELS`]) sorted by value.
fn visible_levels(drawing: &Drawing) -> Vec<Level> {
    let mut levels = drawing
        .levels
        .iter()
        .take(MAX_DRAWING_LEVELS)
        .filter(|level| level.visible && level.value.is_finite())
        .map(|level| {
            let fill =
                (drawing.fill_enabled && level.fill_between).then(|| level.zone_fill(drawing));
            Level {
                value: level.value,
                color: level.stroke_color(drawing),
                style: level.line_style(),
                fill,
                label: level.label_visible,
            }
        })
        .collect::<Vec<_>>();
    levels.sort_by(|a, b| a.value.total_cmp(&b.value));
    levels
}

/// `value` with at most `decimals` fraction digits and no trailing zeros.
fn trimmed(value: f64, decimals: usize) -> String {
    let text = format!("{value:.decimals$}");
    let text = if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.')
    } else {
        text.as_str()
    };
    if text == "-0" {
        "0".to_string()
    } else {
        text.to_string()
    }
}

/// Level label text: the value (`0.618` or `61.8%`) and/or the formatted price in parentheses.
fn level_text(
    engine: &ChartEngine,
    drawing: &Drawing,
    options: &FibonacciToolOptions,
    value: f64,
    price: Option<f64>,
) -> Option<String> {
    let value = options.show_levels.then(|| {
        if options.levels_as_percent {
            format!("{}%", trimmed(value * 100.0, 2))
        } else {
            trimmed(value, 4)
        }
    });
    let price = price
        .filter(|price| options.show_prices && price.is_finite())
        .map(|price| engine.drawing_price_text(drawing, price));
    match (value, price) {
        (Some(value), Some(price)) => Some(format!("{value} ({price})")),
        (Some(text), None) | (None, Some(text)) => Some(text),
        (None, None) => None,
    }
}

/// The price at ratio `value` from `from` (value 0) toward `to` (value 1), linear beyond them;
/// in log space when `log` and both prices are positive.
fn ratio_price(from: f64, to: f64, value: f64, log: bool) -> f64 {
    if log && from > 0.0 && to > 0.0 {
        (from.ln() + (to.ln() - from.ln()) * value).exp()
    } else {
        from + (to - from) * value
    }
}

/// The prices a price-level tool's levels 0 and 1 sit at.
fn price_ends(
    kind: DrawingKind,
    points: &[DrawingPoint],
    options: &FibonacciToolOptions,
) -> Option<(f64, f64)> {
    let (zero, one) = match kind {
        // Level 0 on the second anchor, level 1 on the first.
        DrawingKind::FibRetracement => (points.get(1)?.price, points.first()?.price),
        // The first leg's move projected from the third anchor.
        DrawingKind::TrendBasedFibExtension => {
            let (a, b, c) = (
                points.first()?.price,
                points.get(1)?.price,
                points.get(2)?.price,
            );
            let log = options.log_scale && a > 0.0 && b > 0.0 && c > 0.0;
            (c, if log { c * b / a } else { c + b - a })
        }
        _ => return None,
    };
    Some(if options.reverse {
        (one, zero)
    } else {
        (zero, one)
    })
}

/// The channel's level line `value` (0 on the first two anchors, 1 through the third) as two
/// semantic endpoints at the first two anchors' times. A vertical first leg offsets in time.
fn channel_line(
    points: &[DrawingPoint],
    value: f64,
    options: &FibonacciToolOptions,
) -> Option<(DrawingPoint, DrawingPoint)> {
    let (a, b, c) = (*points.first()?, *points.get(1)?, *points.get(2)?);
    let value = if options.reverse { 1.0 - value } else { value };
    let span = b.logical - a.logical;
    if span.abs() <= f64::EPSILON {
        let shift = (c.logical - a.logical) * value;
        return Some((
            DrawingPoint {
                logical: a.logical + shift,
                price: a.price,
            },
            DrawingPoint {
                logical: b.logical + shift,
                price: b.price,
            },
        ));
    }
    let t = (c.logical - a.logical) / span;
    let (start, end) = if options.log_scale && a.price > 0.0 && b.price > 0.0 && c.price > 0.0 {
        let (la, lb) = (a.price.ln(), b.price.ln());
        let shift = (c.price.ln() - (la + (lb - la) * t)) * value;
        ((la + shift).exp(), (lb + shift).exp())
    } else {
        let shift = (c.price - (a.price + (b.price - a.price) * t)) * value;
        (a.price + shift, b.price + shift)
    };
    Some((
        DrawingPoint {
            logical: a.logical,
            price: start,
        },
        DrawingPoint {
            logical: b.logical,
            price: end,
        },
    ))
}

/// Logical position of time level `value` for the time tools.
fn time_logical(
    kind: DrawingKind,
    points: &[DrawingPoint],
    value: f64,
    reverse: bool,
) -> Option<f64> {
    let (a, b) = (points.first()?.logical, points.get(1)?.logical);
    match kind {
        DrawingKind::FibTimeZone => Some(a + (b - a) * if reverse { -value } else { value }),
        DrawingKind::TrendBasedFibTime => Some(points.get(2)?.logical + (b - a) * value),
        _ => None,
    }
}

/// Running min/max of one dimension; `None` once any value is non-finite (unbounded).
#[derive(Clone, Copy)]
struct Reach(Option<(f64, f64)>);

impl Reach {
    fn new() -> Self {
        Self(Some((f64::INFINITY, f64::NEG_INFINITY)))
    }

    fn add(&mut self, value: f64) {
        self.0 = self
            .0
            .filter(|_| value.is_finite())
            .map(|(min, max)| (min.min(value), max.max(value)));
    }

    fn get(self) -> Option<(f64, f64)> {
        self.0.filter(|(min, max)| min <= max)
    }
}

/// Semantic reach of the price- and time-level tools; screen-space tools keep their unbounded
/// spec extents.
fn bounds(drawing: &Drawing) -> Option<FamilyBounds> {
    let points = &drawing.points;
    let options = options(drawing);
    let mut logical = Reach::new();
    let mut price = Reach::new();
    for point in points {
        logical.add(point.logical);
        price.add(point.price);
    }
    let levels = visible_levels(drawing);
    match drawing.kind {
        DrawingKind::FibRetracement | DrawingKind::TrendBasedFibExtension => {
            let (zero, one) = price_ends(drawing.kind, points, &options)?;
            if drawing.kind == DrawingKind::TrendBasedFibExtension {
                let (a, b, c) = (points[0].logical, points[1].logical, points[2].logical);
                logical.add(c + (b - a));
            }
            for level in &levels {
                price.add(ratio_price(zero, one, level.value, options.log_scale));
            }
        }
        DrawingKind::FibChannel => {
            for level in &levels {
                let (start, end) = channel_line(points, level.value, &options)?;
                for point in [start, end] {
                    logical.add(point.logical);
                    price.add(point.price);
                }
            }
        }
        DrawingKind::FibTimeZone | DrawingKind::TrendBasedFibTime => {
            for level in &levels {
                logical.add(time_logical(
                    drawing.kind,
                    points,
                    level.value,
                    options.reverse,
                )?);
            }
            // Vertical lines span the pane.
            return Some(FamilyBounds {
                logical: logical.get(),
                price: None,
            });
        }
        _ => return None,
    }
    Some(FamilyBounds {
        logical: logical.get(),
        price: price.get(),
    })
}

/// Every level label of a price- or time-level tool (the text its labels can reach with).
fn label_texts(engine: &ChartEngine, drawing: &Drawing) -> Vec<String> {
    let options = resolved(drawing);
    let ends = price_ends(drawing.kind, &drawing.points, &options);
    visible_levels(drawing)
        .iter()
        .filter(|level| level.label)
        .filter_map(|level| {
            let price =
                ends.map(|(zero, one)| ratio_price(zero, one, level.value, options.log_scale));
            level_text(engine, drawing, &options, level.value, price)
        })
        .collect()
}

/// Conservative reach of level labels beyond the level lines (or a ring tool's paint box), in CSS
/// px. The fan and the spiral cull by the pane only, so they need none. A ring label sits past its
/// ring (lifted above or below a circle, beyond the arc along the wedge's bisector), centered on
/// it. Price text follows the scale's formatter, so four ems of slack keep the cached pad valid
/// between text-key refreshes.
fn decoration_extent(engine: &ChartEngine, drawing: &Drawing) -> f64 {
    let rings = matches!(
        drawing.kind,
        DrawingKind::FibSpeedResistanceArcs | DrawingKind::FibCircles | DrawingKind::FibWedge
    );
    if !rings && drawing.kind.spec().logical_extent == DrawingLogicalExtent::Full {
        return 0.0;
    }
    let family = &engine.options.get().layout.font_family;
    let size = engine.drawing_text_size(drawing);
    let weight = drawing.text_weight.unwrap_or(400);
    let width = label_texts(engine, drawing)
        .iter()
        .map(|text| engine.measure_text_run(text, size, family, weight, drawing.text_italic))
        .fold(0.0_f64, f64::max);
    if width <= 0.0 {
        return 0.0;
    }
    if rings {
        return LABEL_GAP.max(LABEL_LIFT) + width / 2.0 + size * 1.25;
    }
    (LABEL_GAP + width + 4.0 * size).max(LABEL_LIFT + size * 1.25)
}

fn build_parts(ctx: &PartContext<'_>, parts: &mut DrawingParts) {
    match ctx.drawing.kind {
        DrawingKind::FibRetracement | DrawingKind::TrendBasedFibExtension => {
            price_levels(ctx, parts)
        }
        DrawingKind::FibChannel => channel(ctx, parts),
        DrawingKind::FibTimeZone | DrawingKind::TrendBasedFibTime => time_levels(ctx, parts),
        DrawingKind::FibSpeedResistanceFan => fan(ctx, parts),
        DrawingKind::FibSpeedResistanceArcs | DrawingKind::FibCircles => circles(ctx, parts),
        DrawingKind::FibSpiral => spiral(ctx, parts),
        DrawingKind::FibWedge => wedge(ctx, parts),
        _ => {}
    }
}

/// The dashed trend line through `points` in the drawing's own stroke.
fn trend_line(ctx: &PartContext<'_>, parts: &mut DrawingParts, points: &[Point]) {
    if resolved(ctx.drawing).trend_line {
        parts.stroke(points, PartStroke::default(), false);
    }
}

fn label(
    ctx: &PartContext<'_>,
    anchor: Point,
    align: (DrawingTextHAlign, DrawingTextVAlign),
    text: String,
    color: Color,
) -> PartLabel {
    PartLabel {
        anchor,
        h_align: align.0,
        v_align: align.1,
        lines: vec![text],
        size: ctx.engine.drawing_text_size(ctx.drawing) * ctx.scale,
        weight: ctx.drawing.text_weight.unwrap_or(400),
        italic: ctx.drawing.text_italic,
        color: Some(color),
        background: None,
        border: None,
        padding: (0.0, 0.0),
        hit: true,
    }
}

/// Push `label` unless its box cannot reach the pane. One glyph size per character bounds the
/// width of level text (digits, signs, percents, and formatted prices), so a label whose level
/// lies just outside the pane still paints while far-off levels emit nothing.
fn push_label(ctx: &PartContext<'_>, parts: &mut DrawingParts, label: PartLabel) {
    let characters = label.lines.iter().map(|line| line.chars().count()).max();
    let width = label.size * characters.unwrap_or(0) as f64;
    let height = label.size * 1.25 * label.lines.len() as f64;
    let (x, y) = label.anchor;
    let pane = ctx.pane;
    if x >= pane.left - width
        && x <= pane.right + width
        && y >= pane.top - height
        && y <= pane.bottom + height
    {
        parts.label(label);
    }
}

/// A label for the level line `start → end` whose ends are `extended` to the pane edge.
fn line_label(
    ctx: &PartContext<'_>,
    (start, end): (Point, Point),
    extended: (bool, bool),
    options: &FibonacciToolOptions,
    text: String,
    color: Color,
) -> PartLabel {
    let (left, right, left_extended, right_extended) = if start.0 <= end.0 {
        (start, end, extended.0, extended.1)
    } else {
        (end, start, extended.1, extended.0)
    };
    let gap = LABEL_GAP * ctx.scale;
    let (anchor, h_align) = match options.label_h_align.unwrap_or(FibonacciLabelHAlign::Left) {
        FibonacciLabelHAlign::Left if left_extended => {
            ((left.0 + gap, left.1), DrawingTextHAlign::Left)
        }
        FibonacciLabelHAlign::Left => ((left.0 - gap, left.1), DrawingTextHAlign::Right),
        FibonacciLabelHAlign::Right if right_extended => {
            ((right.0 - gap, right.1), DrawingTextHAlign::Right)
        }
        FibonacciLabelHAlign::Right => ((right.0 + gap, right.1), DrawingTextHAlign::Left),
        FibonacciLabelHAlign::Center => (
            ((left.0 + right.0) / 2.0, (left.1 + right.1) / 2.0),
            DrawingTextHAlign::Center,
        ),
    };
    let lift = LABEL_LIFT * ctx.scale;
    let (dy, v_align) = match options
        .label_v_align
        .unwrap_or(FibonacciLabelVAlign::Middle)
    {
        FibonacciLabelVAlign::Top => (-lift, DrawingTextVAlign::Bottom),
        FibonacciLabelVAlign::Middle => (0.0, DrawingTextVAlign::Middle),
        FibonacciLabelVAlign::Bottom => (lift, DrawingTextVAlign::Top),
    };
    label(
        ctx,
        (anchor.0, anchor.1 + dy),
        (h_align, v_align),
        text,
        color,
    )
}

/// Whether a horizontal or vertical line at `value` (y or x) with `half` width can touch
/// `[low, high]`.
fn within(value: f64, low: f64, high: f64, half: f64) -> bool {
    value >= low - half && value <= high + half
}

/// Horizontal price levels of the retracement and the extension: background bands, level
/// lines, the trend line, then labels.
fn price_levels(ctx: &PartContext<'_>, parts: &mut DrawingParts) {
    let drawing = ctx.drawing;
    let (Some(&a), Some(&b)) = (ctx.px.first(), ctx.px.get(1)) else {
        return;
    };
    let extension = drawing.kind == DrawingKind::TrendBasedFibExtension;
    let c = match ctx.px.get(2) {
        Some(&c) if extension => c,
        // The extension's first leg while its third anchor is being placed.
        None if extension => {
            trend_line(ctx, parts, &[a, b]);
            return;
        }
        _ => a,
    };
    let options = resolved(drawing);
    let Some((zero, one)) = price_ends(drawing.kind, &drawing.points, &options) else {
        return;
    };
    // The retracement spans its anchors; the extension spans the first leg's width from the
    // third anchor.
    let span = if extension {
        (c.0, c.0 + (b.0 - a.0))
    } else {
        (a.0, b.0)
    };
    let (extend_left, extend_right) = (drawing.extend_left, drawing.extend_right);
    let x0 = if extend_left {
        ctx.pane.left
    } else {
        span.0.min(span.1)
    };
    let x1 = if extend_right {
        ctx.pane.right
    } else {
        span.0.max(span.1)
    };
    let logical = drawing.points[0].logical;
    let levels = visible_levels(drawing);
    let ys = levels
        .iter()
        .map(|level| {
            let price = ratio_price(zero, one, level.value, options.log_scale);
            ctx.point_px(DrawingPoint { logical, price })
                .map(|(_, y)| y)
                .filter(|y| y.is_finite())
        })
        .collect::<Vec<_>>();
    let (top, bottom) = (ctx.pane.top, ctx.pane.bottom);
    let hit = ctx.fills_hit();
    for index in 1..levels.len() {
        let (Some(fill), Some(y0), Some(y1)) = (levels[index].fill, ys[index - 1], ys[index])
        else {
            continue;
        };
        let (y0, y1) = (y0.clamp(top, bottom), y1.clamp(top, bottom));
        if y0 != y1 {
            parts.fill(
                &[(x0, y0), (x1, y0)],
                &[(x0, y1), (x1, y1)],
                Some(fill),
                hit,
            );
        }
    }
    let half = drawing.width * ctx.scale;
    for (level, y) in levels.iter().zip(&ys) {
        if let Some(y) = y.filter(|&y| within(y, top, bottom, half)) {
            parts.hline(y, x0, x1, level.stroke());
        }
    }
    if extension {
        trend_line(ctx, parts, &[a, b, c]);
    } else {
        trend_line(ctx, parts, &[a, b]);
    }
    for (level, y) in levels.iter().zip(&ys) {
        let Some(y) = y.filter(|_| level.label) else {
            continue;
        };
        let price = ratio_price(zero, one, level.value, options.log_scale);
        if let Some(text) = level_text(ctx.engine, drawing, &options, level.value, Some(price)) {
            let label = line_label(
                ctx,
                ((x0, y), (x1, y)),
                (extend_left, extend_right),
                &options,
                text,
                level.color,
            );
            push_label(ctx, parts, label);
        }
    }
}

/// Whether the segment `start → end` touches `pane` grown by `margin`.
fn touches(pane: Rect, (start, end): (Point, Point), margin: f64) -> bool {
    let rect = pane.inflate(margin);
    rect.contains(start)
        || rect.contains(end)
        || shape::line_rect_interval(start, end, rect)
            .is_some_and(|(t0, t1)| t0 <= 1.0 && t1 >= 0.0)
}

/// The convex part of `pane` on the `inside` point's side of every line `(p, q)`, into `out`.
/// `false` when a line holds its `inside` point (a degenerate region), when less than a triangle
/// remains, or when far-off lines overflow the clip.
fn clip_pane(
    pane: Rect,
    sides: impl IntoIterator<Item = ((Point, Point), Point)>,
    out: &mut Vec<Point>,
) -> bool {
    out.clear();
    out.extend_from_slice(&pane_polygon(pane));
    let mut clipped = Vec::with_capacity(8);
    for ((p, q), inside) in sides {
        let side = (q.0 - p.0) * (inside.1 - p.1) - (q.1 - p.1) * (inside.0 - p.0);
        if side.is_nan() || side == 0.0 {
            return false;
        }
        let (a, b) = if side > 0.0 { (p, q) } else { (q, p) };
        shape::clip_to_half_plane(out, a, b, &mut clipped);
        std::mem::swap(out, &mut clipped);
    }
    out.len() >= 3
        && out
            .iter()
            .all(|point| point.0.is_finite() && point.1.is_finite())
}

/// Level lines parallel to the first leg, offset toward the third anchor.
fn channel(ctx: &PartContext<'_>, parts: &mut DrawingParts) {
    let drawing = ctx.drawing;
    let (Some(&a), Some(&b)) = (ctx.px.first(), ctx.px.get(1)) else {
        return;
    };
    if ctx.px.len() < 3 {
        // The base line while the third anchor is being placed.
        parts.stroke(&[a, b], PartStroke::default(), false);
        return;
    }
    let options = resolved(drawing);
    let (extend_left, extend_right) = (drawing.extend_left, drawing.extend_right);
    let levels = visible_levels(drawing);
    // Each level's segment between the first two anchors' times, ordered left to right.
    let ends = levels
        .iter()
        .map(|level| {
            let (start, end) = channel_line(&drawing.points, level.value, &options)?;
            let (p, q) = (ctx.point_px(start)?, ctx.point_px(end)?);
            [p, q]
                .iter()
                .all(|point| point.0.is_finite() && point.1.is_finite())
                .then_some(if p.0 <= q.0 { (p, q) } else { (q, p) })
        })
        .collect::<Vec<_>>();
    // A band is the pane between two level lines, closed at the anchors' times on the ends that
    // do not extend; clipping the pane covers the corners extended lines leave through.
    let hit = ctx.fills_hit();
    let mut region = Vec::new();
    for index in 1..levels.len() {
        let (Some(fill), Some(upper), Some(lower)) =
            (levels[index].fill, ends[index - 1], ends[index])
        else {
            continue;
        };
        let sides = [
            Some((upper, shape::midpoint(lower.0, lower.1))),
            Some((lower, shape::midpoint(upper.0, upper.1))),
            (!extend_left).then(|| ((upper.0, lower.0), shape::midpoint(upper.1, lower.1))),
            (!extend_right).then(|| ((upper.1, lower.1), shape::midpoint(upper.0, lower.0))),
        ];
        if clip_pane(ctx.pane, sides.into_iter().flatten(), &mut region) {
            parts.fill_convex(&region, Some(fill), hit);
        }
    }
    let half = drawing.width * ctx.scale;
    let lines = ends
        .iter()
        .map(|end| {
            let (left, right) = (*end)?;
            let line = shape::extend_segment(left, right, ctx.pane, extend_left, extend_right);
            touches(ctx.pane, line, half).then_some(line)
        })
        .collect::<Vec<_>>();
    for (level, line) in levels.iter().zip(&lines) {
        if let Some((start, end)) = line {
            parts.stroke(&[*start, *end], level.stroke(), false);
        }
    }
    for (level, end) in levels.iter().zip(&ends) {
        let Some((left, right)) = end.filter(|_| level.label) else {
            continue;
        };
        if let Some(text) = level_text(ctx.engine, drawing, &options, level.value, None) {
            let line = shape::extend_segment(left, right, ctx.pane, extend_left, extend_right);
            let label = line_label(
                ctx,
                line,
                (extend_left, extend_right),
                &options,
                text,
                level.color,
            );
            push_label(ctx, parts, label);
        }
    }
}

/// Vertical time levels: time zones from the first anchor, trend-based time from the third.
fn time_levels(ctx: &PartContext<'_>, parts: &mut DrawingParts) {
    let drawing = ctx.drawing;
    let (Some(&a), Some(&b)) = (ctx.px.first(), ctx.px.get(1)) else {
        return;
    };
    let trend_based = drawing.kind == DrawingKind::TrendBasedFibTime;
    if trend_based && ctx.px.len() < 3 {
        trend_line(ctx, parts, &[a, b]);
        return;
    }
    let options = resolved(drawing);
    // The time axis is affine in logical position, so ratio positions interpolate the anchors'
    // px exactly.
    let (start, step) = if trend_based {
        (ctx.px[2].0, b.0 - a.0)
    } else if options.reverse {
        (a.0, a.0 - b.0)
    } else {
        (a.0, b.0 - a.0)
    };
    let (left, right) = (ctx.pane.left, ctx.pane.right);
    let (top, bottom) = (ctx.pane.top, ctx.pane.bottom);
    let levels = visible_levels(drawing);
    let xs = levels
        .iter()
        .map(|level| start + step * level.value)
        .collect::<Vec<_>>();
    let hit = ctx.fills_hit();
    for index in 1..levels.len() {
        let Some(fill) = levels[index].fill else {
            continue;
        };
        let (x0, x1) = (
            xs[index - 1].clamp(left, right),
            xs[index].clamp(left, right),
        );
        if x0 != x1 {
            parts.fill(
                &[(x0, top), (x0, bottom)],
                &[(x1, top), (x1, bottom)],
                Some(fill),
                hit,
            );
        }
    }
    let half = drawing.width * ctx.scale;
    for (level, &x) in levels.iter().zip(&xs) {
        if within(x, left, right, half) {
            parts.vline(x, top, bottom, level.stroke());
        }
    }
    if trend_based {
        trend_line(ctx, parts, &[a, b, ctx.px[2]]);
    } else {
        trend_line(ctx, parts, &[a, b]);
    }
    let gap = LABEL_GAP * ctx.scale;
    let lift = LABEL_LIFT * ctx.scale;
    for (level, &x) in levels.iter().zip(&xs) {
        if !level.label {
            continue;
        }
        let Some(text) = level_text(ctx.engine, drawing, &options, level.value, None) else {
            continue;
        };
        let (anchor_x, h_align) = match options.label_h_align.unwrap_or(FibonacciLabelHAlign::Right)
        {
            FibonacciLabelHAlign::Left => (x - gap, DrawingTextHAlign::Right),
            FibonacciLabelHAlign::Center => (x, DrawingTextHAlign::Center),
            FibonacciLabelHAlign::Right => (x + gap, DrawingTextHAlign::Left),
        };
        let (anchor_y, v_align) = match options
            .label_v_align
            .unwrap_or(FibonacciLabelVAlign::Bottom)
        {
            FibonacciLabelVAlign::Top => (top + lift, DrawingTextVAlign::Top),
            FibonacciLabelVAlign::Middle => ((top + bottom) / 2.0, DrawingTextVAlign::Middle),
            FibonacciLabelVAlign::Bottom => (bottom - lift, DrawingTextVAlign::Bottom),
        };
        let label = label(
            ctx,
            (anchor_x, anchor_y),
            (h_align, v_align),
            text,
            level.color,
        );
        push_label(ctx, parts, label);
    }
}

/// The pane rectangle as a clockwise polygon.
fn pane_polygon(pane: Rect) -> [Point; 4] {
    [
        (pane.left, pane.top),
        (pane.right, pane.top),
        (pane.right, pane.bottom),
        (pane.left, pane.bottom),
    ]
}

/// The convex part of the pane between the rays `apex → first` and `apex → second` (the angle
/// below π), or `false` when the rays are parallel.
fn wedge_region(
    pane: Rect,
    apex: Point,
    first: Point,
    second: Point,
    out: &mut Vec<Point>,
) -> bool {
    clip_pane(
        pane,
        [((apex, first), second), ((apex, second), first)],
        out,
    )
}

/// Price rays from the first anchor through the second anchor's time at each level, time rays
/// through its price, the level grid inside the anchors' box, bands, and labels.
fn fan(ctx: &PartContext<'_>, parts: &mut DrawingParts) {
    let drawing = ctx.drawing;
    let (Some(&a), Some(&b)) = (ctx.px.first(), ctx.px.get(1)) else {
        return;
    };
    let options = resolved(drawing);
    let ratio = |value: f64| if options.reverse { 1.0 - value } else { value };
    let price_point = |value: f64| (b.0, b.1 + (a.1 - b.1) * ratio(value));
    let time_point = |value: f64| (b.0 + (a.0 - b.0) * ratio(value), b.1);
    // A ray's direction as a point at most one px per axis from the apex, so rays through far-off
    // levels stay finite; `None` for a ray through the apex itself.
    let toward = |through: Point| {
        let (dx, dy) = (through.0 - a.0, through.1 - a.1);
        let length = dx.abs().max(dy.abs());
        (length > f64::EPSILON && length.is_finite())
            .then(|| (a.0 + dx / length, a.1 + dy / length))
    };
    let levels = visible_levels(drawing);
    let hit = ctx.fills_hit();
    let mut region = Vec::new();
    let fans: [&dyn Fn(f64) -> Point; 2] = [&price_point, &time_point];
    for through in fans {
        for index in 1..levels.len() {
            let Some(fill) = levels[index].fill else {
                continue;
            };
            let (Some(first), Some(second)) = (
                toward(through(levels[index - 1].value)),
                toward(through(levels[index].value)),
            ) else {
                continue;
            };
            if wedge_region(ctx.pane, a, first, second, &mut region) {
                parts.fill_convex(&region, Some(fill), hit);
            }
        }
    }
    let half = drawing.width * ctx.scale;
    let pane = ctx.pane;
    if options.grid {
        for level in &levels {
            let (x, y) = (time_point(level.value).0, price_point(level.value).1);
            if within(y, pane.top, pane.bottom, half) {
                parts.hline(y, a.0, b.0, PartStroke::default());
            }
            if within(x, pane.left, pane.right, half) {
                parts.vline(x, a.1.min(b.1), a.1.max(b.1), PartStroke::default());
            }
        }
    }
    // Every ray runs from the apex to the pane edge.
    let ray = |through: Point| {
        toward(through).map(|direction| shape::extend_segment(a, direction, pane, false, true))
    };
    for level in &levels {
        let (price, time) = (price_point(level.value), time_point(level.value));
        if let Some((start, end)) = ray(price) {
            parts.stroke(&[start, end], level.stroke(), false);
        }
        // Level 0 of both fans is the ray through the second anchor; paint it once.
        if (time.0 - price.0).hypot(time.1 - price.1) > f64::EPSILON {
            if let Some((start, end)) = ray(time) {
                parts.stroke(&[start, end], level.stroke(), false);
            }
        }
    }
    let gap = LABEL_GAP * ctx.scale;
    let (right, down) = (b.0 >= a.0, b.1 >= a.1);
    for level in levels.iter().filter(|level| level.label) {
        let Some(text) = level_text(ctx.engine, drawing, &options, level.value, None) else {
            continue;
        };
        let price = price_point(level.value);
        let price_label = label(
            ctx,
            (price.0 + if right { gap } else { -gap }, price.1),
            (
                if right {
                    DrawingTextHAlign::Left
                } else {
                    DrawingTextHAlign::Right
                },
                DrawingTextVAlign::Middle,
            ),
            text.clone(),
            level.color,
        );
        push_label(ctx, parts, price_label);
        let time = time_point(level.value);
        let time_label = label(
            ctx,
            (time.0, time.1 + if down { gap } else { -gap }),
            (
                DrawingTextHAlign::Center,
                if down {
                    DrawingTextVAlign::Top
                } else {
                    DrawingTextVAlign::Bottom
                },
            ),
            text,
            level.color,
        );
        push_label(ctx, parts, time_label);
    }
}

/// Unit vectors at `segments + 1` evenly spaced screen angles over `(start, sweep)` (positive
/// sweeps turn clockwise on screen). Every arc of one tool scales this one table, so paired band
/// chains share their angles and the trigonometry runs once per tool, not once per arc.
fn unit_arc((start, sweep): (f64, f64), segments: usize) -> Vec<Point> {
    (0..=segments)
        .map(|step| {
            let angle = start + sweep * step as f64 / segments as f64;
            (angle.cos(), angle.sin())
        })
        .collect()
}

/// The arc of `radius` around `center` at the angles of `unit` (a [`unit_arc`] table).
fn arc_points(center: Point, radius: f64, unit: &[Point], out: &mut Vec<Point>) {
    out.clear();
    out.extend(
        unit.iter()
            .map(|&(cos, sin)| (center.0 + radius * cos, center.1 + radius * sin)),
    );
}

/// The dash period of `stroke` in caller px as frame lowering splits it, 0 when solid.
fn dash_period(ctx: &PartContext<'_>, stroke: PartStroke) -> f64 {
    let style = stroke.line_style(ctx.drawing);
    if style == LineStyle::Solid {
        return 0.0;
    }
    let width = (stroke.width_css(ctx.drawing) * ctx.scale) as f32;
    style
        .dash_pattern(width)
        .iter()
        .map(|&length| f64::from(length))
        .sum()
}

/// One tool's concentric arcs: the whole arc `(start, sweep)` around `center`, the part of it the
/// pane shows (`window`, from [`visible_arc`]), and the window's shared unit-angle table, which
/// pairs band chains.
struct Rings {
    center: Point,
    arc: (f64, f64),
    window: (f64, f64),
    angles: Vec<Point>,
}

impl Rings {
    /// Stroke the ring of `radius` over the window. A dashed or dotted ring keeps the pattern of
    /// the whole arc, which starts at the arc's start: a windowed piece starts at the last
    /// dash-period boundary before the window (in arc length, off the pane), and a full circle
    /// restarts its pattern where the whole circle does, so dashes stay put while the pane scrolls
    /// instead of following the window's edge.
    fn stroke(
        &self,
        ctx: &PartContext<'_>,
        parts: &mut DrawingParts,
        radius: f64,
        stroke: PartStroke,
        points: &mut Vec<Point>,
    ) {
        let period = dash_period(ctx, stroke);
        if period <= 0.0 || self.window == self.arc {
            arc_points(self.center, radius, &self.angles, points);
            parts.stroke(points, stroke, false);
            return;
        }
        let (arc_start, sweep) = self.arc;
        let direction = sweep.signum();
        let length = self.window.1.abs();
        // How far along the arc the window starts, within one turn.
        let from = ((self.window.0 - arc_start) * direction).rem_euclid(std::f64::consts::TAU);
        let wrapped = (from + length - std::f64::consts::TAU).max(0.0);
        for (start, span) in [(from, length - wrapped), (0.0, wrapped)] {
            if span <= 0.0 {
                continue;
            }
            let back = (start * radius).rem_euclid(period) / radius;
            let piece = (
                arc_start + direction * (start - back),
                direction * (span + back),
            );
            let segments = shape::arc_segment_count(radius, piece.1, CURVE_TOLERANCE);
            arc_points(self.center, radius, &unit_arc(piece, segments), points);
            parts.stroke(points, stroke, false);
        }
    }
}

/// The part of the arc `(start, sweep)` around `center` that can reach `pane`: the whole arc
/// when the center lies in the pane, else its overlap with the angular window the pane subtends
/// from the center (under π wide; a full turn becomes the window, and an arc of at most π meets
/// it in one piece). `None` when they miss. Tessellating only this part keeps radii far beyond
/// the pane within the curve tolerance at the capped segment count, and skips invisible work.
fn visible_arc(pane: Rect, center: Point, (start, sweep): (f64, f64)) -> Option<(f64, f64)> {
    use std::f64::consts::{PI, TAU};
    if center.0 >= pane.left
        && center.0 <= pane.right
        && center.1 >= pane.top
        && center.1 <= pane.bottom
    {
        return Some((start, sweep));
    }
    let reference = ((pane.top + pane.bottom) / 2.0 - center.1)
        .atan2((pane.left + pane.right) / 2.0 - center.0);
    let (mut low, mut high) = (0.0_f64, 0.0_f64);
    for corner in pane_polygon(pane) {
        let angle = (corner.1 - center.1).atan2(corner.0 - center.0);
        let delta = (angle - reference + PI).rem_euclid(TAU) - PI;
        low = low.min(delta);
        high = high.max(delta);
    }
    let window = (reference + low, reference + high);
    if sweep.abs() >= TAU - 1e-9 {
        return Some((window.0, window.1 - window.0));
    }
    let arc = if sweep >= 0.0 {
        (start, start + sweep)
    } else {
        (start + sweep, start)
    };
    // Both ranges lie within (-2π, 2π], so shifts of up to two turns align them.
    (-2..=2).find_map(|turns| {
        let shift = TAU * f64::from(turns);
        let (from, to) = (arc.0.max(window.0 + shift), arc.1.min(window.1 + shift));
        // Keep the arc's direction so paired band chains and the stroke agree.
        (to > from).then_some(if sweep >= 0.0 {
            (from, to - from)
        } else {
            (to, from - to)
        })
    })
}

/// Concentric ratio arcs: speed resistance arcs around the first anchor (the half toward the
/// second anchor, or full circles) and Fibonacci circles around the anchors' midpoint, level 1
/// through both anchors. Bands fill between neighbouring arcs; labels sit at each arc's apex.
fn circles(ctx: &PartContext<'_>, parts: &mut DrawingParts) {
    let drawing = ctx.drawing;
    let (Some(&a), Some(&b), Some((center, unit))) = (
        ctx.px.first(),
        ctx.px.get(1),
        ring_frame(drawing.kind, ctx.px),
    ) else {
        return;
    };
    let options = resolved(drawing);
    let arcs = drawing.kind == DrawingKind::FibSpeedResistanceArcs;
    let up = b.1 <= a.1;
    let sweep = if !arcs || options.full_circles {
        (0.0, std::f64::consts::TAU)
    } else if up {
        (std::f64::consts::PI, std::f64::consts::PI)
    } else {
        (0.0, std::f64::consts::PI)
    };
    let levels = visible_levels(drawing)
        .into_iter()
        .filter(|level| level.value > 0.0)
        .collect::<Vec<_>>();
    let half = drawing.width * ctx.scale;
    // A ring between radii `inner` and `outer` reaches the pane when it passes between the
    // pane's nearest and farthest points from the center.
    let (near, far) = pane_distances(ctx.pane, center);
    let reaches = |inner: f64, outer: f64| outer + half >= near && inner - half <= far;
    // The whole pane lies within `far` of the center, so a band's outer edge beyond it closes at
    // `cap` instead: the same pane pixels, with finite points and a segment count sized by the
    // largest radius that can show.
    let cap = far + half + 1.0;
    let visible = visible_arc(ctx.pane, center, sweep).filter(|_| unit > f64::EPSILON);
    if let (Some(window), Some(last)) = (visible, levels.last()) {
        let largest = (unit * last.value).min(cap);
        let segments = shape::arc_segment_count(largest, window.1, CURVE_TOLERANCE);
        let rings = Rings {
            center,
            arc: sweep,
            window,
            angles: unit_arc(window, segments),
        };
        let hit = ctx.fills_hit();
        let (mut inner, mut outer) = (Vec::new(), Vec::new());
        for index in 1..levels.len() {
            let Some(fill) = levels[index].fill else {
                continue;
            };
            let (low, high) = (unit * levels[index - 1].value, unit * levels[index].value);
            if reaches(low, high) {
                arc_points(center, low, &rings.angles, &mut inner);
                arc_points(center, high.min(cap), &rings.angles, &mut outer);
                parts.fill(&outer, &inner, Some(fill), hit);
            }
        }
        for level in &levels {
            let radius = unit * level.value;
            if reaches(radius, radius) {
                rings.stroke(ctx, parts, radius, level.stroke(), &mut outer);
            }
        }
    }
    trend_line(ctx, parts, &[a, b]);
    let lift = LABEL_LIFT * ctx.scale;
    let label_up = !arcs || options.full_circles || up;
    for level in levels
        .iter()
        .filter(|level| level.label && visible.is_some())
    {
        let radius = unit * level.value;
        let Some(text) = level_text(ctx.engine, drawing, &options, level.value, None) else {
            continue;
        };
        let (y, v_align) = if label_up {
            (center.1 - radius - lift, DrawingTextVAlign::Bottom)
        } else {
            (center.1 + radius + lift, DrawingTextVAlign::Top)
        };
        let label = label(
            ctx,
            (center.0, y),
            (DrawingTextHAlign::Center, v_align),
            text,
            level.color,
        );
        push_label(ctx, parts, label);
    }
}

/// Distances from `point` to the nearest and the farthest point of `pane`.
fn pane_distances(pane: Rect, point: Point) -> (f64, f64) {
    let dx = (pane.left - point.0).max(point.0 - pane.right).max(0.0);
    let dy = (pane.top - point.1).max(point.1 - pane.bottom).max(0.0);
    let far = pane_polygon(pane)
        .iter()
        .map(|corner| (corner.0 - point.0).hypot(corner.1 - point.1))
        .fold(0.0_f64, f64::max);
    (dx.hypot(dy), far)
}

/// The golden spiral around the first anchor through the second, growing by φ every quarter
/// turn (clockwise on screen, counterclockwise with `reverse`), from a sub-pixel radius until it
/// leaves the pane for good. Only the parts of each quarter turn inside the pane's angular window
/// are tessellated, and quarter turns that cannot reach the pane are skipped, so a spiral whose
/// center lies far off the pane stays within the curve tolerance. A dashed or dotted spiral starts
/// each piece at the last dash-period boundary before it (in arc length from the inner end, off
/// the pane), so its dashes stay put while the pane scrolls.
fn spiral(ctx: &PartContext<'_>, parts: &mut DrawingParts) {
    let (Some(&a), Some(&b)) = (ctx.px.first(), ctx.px.get(1)) else {
        return;
    };
    let options = resolved(ctx.drawing);
    let r0 = (b.0 - a.0).hypot(b.1 - a.1);
    if r0 > f64::EPSILON {
        use std::f64::consts::{FRAC_PI_2, PI, TAU};
        let theta0 = (b.1 - a.1).atan2(b.0 - a.0);
        let turn = if options.reverse { -1.0 } else { 1.0 };
        // r = r0·e^(k·t) after turning `t` radians past the second anchor; k = ln φ / (π/2).
        let growth = golden_ratio().ln() / FRAC_PI_2;
        let radius = |t: f64| r0 * (growth * t).exp();
        let turned = |r: f64| (r / r0).ln() / growth;
        // A logarithmic spiral's arc length grows linearly with its radius.
        let length_per_radius = (1.0 + growth * growth).sqrt() / growth;
        let stroke = PartStroke::default();
        let period = dash_period(ctx, stroke);
        let half = ctx.drawing.width * ctx.scale;
        let (near, far) = pane_distances(ctx.pane, a);
        let inner = (SPIRAL_MIN_RADIUS * ctx.scale).min(r0);
        let start = turned(inner);
        // Past the farthest pane point the radius only grows, so the spiral never returns.
        let end = turned(far + half);
        let quarters = ((end - start) / FRAC_PI_2)
            .ceil()
            .clamp(1.0, MAX_SPIRAL_QUARTERS as f64) as usize;
        let mut run: Vec<Point> = Vec::new();
        let mut run_end = f64::NAN;
        for index in 0..quarters {
            let t0 = start + FRAC_PI_2 * index as f64;
            if radius(t0 + FRAC_PI_2) + half < near {
                continue;
            }
            let angle0 = (theta0 + turn * t0 + PI).rem_euclid(TAU) - PI;
            let Some((from, sweep)) = visible_arc(ctx.pane, a, (angle0, turn * FRAC_PI_2)) else {
                continue;
            };
            let (t_from, t_to) = (
                t0 + (from - angle0) * turn,
                t0 + (from + sweep - angle0) * turn,
            );
            let t_start = if (t_from - run_end).abs() <= 1e-9 {
                // This piece continues the previous one.
                t_from
            } else {
                parts.stroke(&run, stroke, false);
                run.clear();
                if period > 0.0 {
                    let travelled = (radius(t_from) - inner) * length_per_radius;
                    turned(radius(t_from) - travelled.rem_euclid(period) / length_per_radius)
                } else {
                    t_from
                }
            };
            let segments = shape::arc_segment_count(radius(t_to), t_to - t_start, CURVE_TOLERANCE);
            let first = usize::from(!run.is_empty());
            for step in first..=segments {
                let t = t_start + (t_to - t_start) * step as f64 / segments as f64;
                let (r, angle) = (radius(t), theta0 + turn * t);
                run.push((a.0 + r * angle.cos(), a.1 + r * angle.sin()));
            }
            run_end = t_to;
        }
        parts.stroke(&run, stroke, false);
    }
    if options.trend_line {
        parts.stroke(
            &[a, b],
            PartStroke::decoration(1.0, LineStyle::Dashed),
            false,
        );
    }
}

/// Ratio arcs around the first anchor between the edges toward the second and third anchors,
/// level 1 at the second anchor's distance.
fn wedge(ctx: &PartContext<'_>, parts: &mut DrawingParts) {
    let drawing = ctx.drawing;
    let (Some(&a), Some(&b)) = (ctx.px.first(), ctx.px.get(1)) else {
        return;
    };
    let unit = (b.0 - a.0).hypot(b.1 - a.1);
    let Some(&c) = ctx.px.get(2).filter(|_| unit > f64::EPSILON) else {
        parts.stroke(&[a, b], PartStroke::default(), false);
        return;
    };
    let options = resolved(drawing);
    let first = (b.1 - a.1).atan2(b.0 - a.0);
    let second = (c.1 - a.1).atan2(c.0 - a.0);
    let pi = std::f64::consts::PI;
    let sweep = (second - first + pi).rem_euclid(std::f64::consts::TAU) - pi;
    let levels = visible_levels(drawing)
        .into_iter()
        .filter(|level| level.value > 0.0)
        .collect::<Vec<_>>();
    let half = drawing.width * ctx.scale;
    let (near, far) = pane_distances(ctx.pane, a);
    // The whole pane lies within `far` of the apex, so the edges and a band's outer arc beyond it
    // stop at `cap`: the same pane pixels, with finite points.
    let cap = far + half + 1.0;
    let largest = (levels.last().map_or(1.0, |level| level.value).max(1.0) * unit).min(cap);
    let reaches = |inner: f64, outer: f64| outer + half >= near && inner - half <= far;
    if near <= largest + half {
        if let Some(window) = visible_arc(ctx.pane, a, (first, sweep)) {
            let segments = shape::arc_segment_count(largest, window.1, CURVE_TOLERANCE);
            let rings = Rings {
                center: a,
                arc: (first, sweep),
                window,
                angles: unit_arc(window, segments),
            };
            let hit = ctx.fills_hit();
            let (mut inner, mut outer) = (Vec::new(), Vec::new());
            for index in 1..levels.len() {
                let Some(fill) = levels[index].fill else {
                    continue;
                };
                let (low, high) = (unit * levels[index - 1].value, unit * levels[index].value);
                if reaches(low, high) {
                    arc_points(a, low, &rings.angles, &mut inner);
                    arc_points(a, high.min(cap), &rings.angles, &mut outer);
                    parts.fill(&outer, &inner, Some(fill), hit);
                }
            }
            for level in &levels {
                let radius = unit * level.value;
                if reaches(radius, radius) {
                    rings.stroke(ctx, parts, radius, level.stroke(), &mut outer);
                }
            }
        }
        for angle in [first, second] {
            let end = (a.0 + largest * angle.cos(), a.1 + largest * angle.sin());
            parts.stroke(&[a, end], PartStroke::default(), false);
        }
    }
    let gap = LABEL_GAP * ctx.scale;
    let middle = first + sweep / 2.0;
    for level in levels.iter().filter(|level| level.label) {
        let Some(text) = level_text(ctx.engine, drawing, &options, level.value, None) else {
            continue;
        };
        let distance = unit * level.value + gap;
        let label = label(
            ctx,
            (a.0 + distance * middle.cos(), a.1 + distance * middle.sin()),
            (DrawingTextHAlign::Center, DrawingTextVAlign::Middle),
            text,
            level.color,
        );
        push_label(ctx, parts, label);
    }
}

#[cfg(test)]
mod tests;
