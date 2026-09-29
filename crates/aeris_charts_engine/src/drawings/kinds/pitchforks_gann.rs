//! B8 Pitchforks & Gann family (wire ids 96..=127): the Andrews, Schiff, modified Schiff, and
//! inside pitchforks, the pitchfan, the Gann box, Gann square, Gann square fixed, and Gann fan.
//!
//! Pitchforks resolve one frame from their three anchors `A`, `B`, `C`: a median pivot, a base
//! center, and a half-handle vector. Andrews starts the median at `A`, Schiff halfway between
//! `A`'s and `B`'s prices at `A`'s time, modified Schiff at the midpoint of `A` and `B`; all three
//! run it through the midpoint of `B` and `C`, the half handle reaching from that midpoint to `C`.
//! The inside pitchfork starts at the midpoint of `A` and `B` and runs through `C`, its base
//! centered on `C` with the half handle reaching back to `B`. Level `v` of the drawing's `levels`
//! is the pair of tines parallel to the median through `center ± v · half` (level 1 passes
//! through the handle's ends). Unextended lines reach one median length past the base; the common
//! `extend_left`/`extend_right` extend every line backward/forward to the pane edge. The pitchfan
//! draws the same levels as rays from `A` through the level points of the Andrews base.
//!
//! The Gann box divides the two anchors' box by the price `levels` (horizontal) and
//! `tool_options.gann.time_levels` (vertical) with zone fills and ratio labels, and optionally
//! Gann angles from its pivot corner. The Gann squares draw the `levels` grid, the
//! `tool_options.gann.angles` fan, and `tool_options.gann.arcs` from their pivot corner; the
//! fixed square is one anchor plus a size in bars and a price-per-bar scale ratio. The Gann fan's
//! `levels` are angles as multiples of the 1×1 slope (`2` is 1×2: two price units per bar unit),
//! with the 1×1 through the second anchor or at `tool_options.gann.scale_ratio` price per bar.
//!
//! The pitchforks and the pitchfan add a derived handle on the base midpoint between `B` and `C`
//! that moves both, and the fixed square one on its far corner that resizes it (the shared
//! `handles` and `drag` hooks).
//!
//! Level geometry is affine in the anchors' px (a level on a logarithmic scale sits at its screen
//! fraction, not its price fraction). Geometry reaching past the anchors (tines, levels, arcs, the
//! fixed square, a flat fan's lines) is bounded by the family's `paint_bounds` hook, which
//! resolves the same corners and line ends in media px, so culling and hit candidates hold on
//! every scale mode.

use aeris_charts_core::model::data_validation::MAX_SAFE_VALUE;
use aeris_charts_render::color::Color;
use aeris_charts_render::draw_list::LineStyle;
use aeris_charts_render::shape::{self, Point, Rect};

use super::super::handles::{DrawingHandle, HandleDrag, HandleShape};
use super::super::parts::{
    text_on, DrawingParts, PartContext, PartLabel, PartStroke, CURVE_TOLERANCE, STATS_ALPHA,
    STATS_GAP, STATS_PADDING,
};
use super::super::tools::{
    DrawingHandleMode, DrawingLogicalExtent, DrawingMovementAxis, DrawingPlacement,
    DrawingPriceExtent, DrawingStraightenMode, DrawingTextLayout, DrawingToolSpec,
};
use super::super::{Drawing, DrawingTextHAlign, DrawingTextVAlign};
use super::DrawingFamily;
use crate::{
    ChartEngine, DrawingDragPart, DrawingKind, DrawingKindOptions, DrawingLevel, DrawingPoint,
    DrawingPropertyDescriptor, DrawingPropertyType, DrawingToolOptions, MAX_DRAWING_LEVELS,
};

/// Upper bound on the fixed Gann square's side in bars.
pub const MAX_GANN_SQUARE_BARS: f64 = 100_000.0;
/// The fixed Gann square's default side in bars.
const DEFAULT_SQUARE_BARS: f64 = 20.0;

/// Gann-tool options (`tool_options.gann`); absent fields keep their defaults. Each field applies
/// to the Gann tools named on it; the others ignore it.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct GannToolOptions {
    /// Gann box: vertical levels as fractions of the box width from the pivot corner (the
    /// drawing's `levels` are its horizontal price levels).
    pub time_levels: Vec<DrawingLevel>,
    /// Gann box (with `show_angles`) and Gann squares: angle lines from the pivot corner, each a
    /// multiple of the 1×1 slope (the box diagonal); values must be positive.
    pub angles: Vec<DrawingLevel>,
    /// Gann squares: quarter arcs around the pivot corner, radii as fractions of the side;
    /// values must be positive.
    pub arcs: Vec<DrawingLevel>,
    /// Gann box and squares: measure from the second anchor's price instead of the first's (the
    /// box also counts time from the second anchor); the fixed square grows downward.
    pub reverse: bool,
    /// Gann box: paint `angles` from the pivot corner.
    pub show_angles: bool,
    /// Gann squares: the measurement box (price range, bars, and price per bar).
    pub show_stats: bool,
    /// Gann fan and fixed square: price units per bar of the 1×1 angle. `None` makes the fan's
    /// 1×1 pass through its second anchor and the fixed square a square on screen.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scale_ratio: Option<f64>,
    /// Fixed square: side length in bars (1..=[`MAX_GANN_SQUARE_BARS`]).
    pub size_bars: f64,
}

impl Default for GannToolOptions {
    fn default() -> Self {
        Self {
            time_levels: gann_box_levels(),
            angles: gann_angle_levels(false),
            arcs: gann_arc_levels(),
            reverse: false,
            show_angles: false,
            show_stats: true,
            scale_ratio: None,
            size_bars: DEFAULT_SQUARE_BARS,
        }
    }
}

impl GannToolOptions {
    /// Bounded lists of valid levels (positive angle and arc values), a positive finite scale
    /// ratio, and a side in `1..=MAX_GANN_SQUARE_BARS`.
    pub(crate) fn validate(&self) -> bool {
        let list = |levels: &[DrawingLevel], positive: bool| {
            levels.len() <= MAX_DRAWING_LEVELS
                && levels
                    .iter()
                    .all(|level| level.validate() && (!positive || level.value > 0.0))
        };
        list(&self.time_levels, false)
            && list(&self.angles, true)
            && list(&self.arcs, true)
            && self
                .scale_ratio
                .is_none_or(|ratio| ratio.is_finite() && ratio > 0.0)
            && self.size_bars.is_finite()
            && (1.0..=MAX_GANN_SQUARE_BARS).contains(&self.size_bars)
    }
}

/// Median color of the pitchforks and the pitchfan (TradingView's red median).
const MEDIAN_COLOR: &str = "#f23645";
/// Gap between a line end or box edge and its label, in CSS px.
const LABEL_GAP: f64 = 4.0;
/// Width of the shifted-pivot pitchforks' dashed `A`–`B` swing guide, in CSS px.
const GUIDE_WIDTH: f64 = 1.0;

/// Pitchfork and pitchfan levels (TradingView's defaults): median offsets in half-handle widths;
/// 0.5 and 1 (the tines through the handle ends) are visible.
const PITCHFORK_LEVELS: [(f64, &str, bool); 9] = [
    (0.25, "#ffb74d", false),
    (0.382, "#81c784", false),
    (0.5, "#089981", true),
    (0.618, "#4caf50", false),
    (0.75, "#00bcd4", false),
    (1.0, "#2962ff", true),
    (1.5, "#9c27b0", false),
    (1.75, "#e91e63", false),
    (2.0, "#f23645", false),
];

/// Gann box price and time levels (TradingView's defaults), all visible with zone fills.
const GANN_BOX_LEVELS: [(f64, &str); 7] = [
    (0.0, "#787b86"),
    (0.25, "#ff9800"),
    (0.382, "#4caf50"),
    (0.5, "#089981"),
    (0.618, "#00bcd4"),
    (0.75, "#2962ff"),
    (1.0, "#787b86"),
];

/// Gann angles as multiples of the 1×1 slope, flattest first: 8×1, 4×1, 3×1, 2×1, 1×1, 1×2,
/// 1×3, 1×4, 1×8 (time units × price units).
const GANN_ANGLES: [(f64, &str); 9] = [
    (0.125, "#ff9800"),
    (0.25, "#4caf50"),
    (1.0 / 3.0, "#089981"),
    (0.5, "#00bcd4"),
    (1.0, "#787b86"),
    (2.0, "#2962ff"),
    (3.0, "#673ab7"),
    (4.0, "#9c27b0"),
    (8.0, "#f23645"),
];

/// Gann square grid: the side in fifths (TradingView's 0–5 grid).
const GANN_SQUARE_GRID: [(f64, &str); 6] = [
    (0.0, "#787b86"),
    (0.2, "#ff9800"),
    (0.4, "#4caf50"),
    (0.6, "#089981"),
    (0.8, "#2962ff"),
    (1.0, "#787b86"),
];

/// Gann square arcs: radii in fifths of the side.
const GANN_SQUARE_ARCS: [(f64, &str); 5] = [
    (0.2, "#ff9800"),
    (0.4, "#4caf50"),
    (0.6, "#089981"),
    (0.8, "#2962ff"),
    (1.0, "#787b86"),
];

fn level(value: f64, color: &str, visible: bool, fill_between: bool, label: bool) -> DrawingLevel {
    DrawingLevel {
        visible,
        fill_between,
        label_visible: label,
        ..DrawingLevel::at(value, color)
    }
}

fn pitchfork_levels() -> Vec<DrawingLevel> {
    PITCHFORK_LEVELS
        .iter()
        .map(|&(value, color, visible)| level(value, color, visible, true, false))
        .collect()
}

fn gann_box_levels() -> Vec<DrawingLevel> {
    GANN_BOX_LEVELS
        .iter()
        .map(|&(value, color)| level(value, color, true, true, true))
        .collect()
}

fn gann_angle_levels(labels: bool) -> Vec<DrawingLevel> {
    GANN_ANGLES
        .iter()
        .map(|&(value, color)| level(value, color, true, true, labels))
        .collect()
}

fn gann_square_grid() -> Vec<DrawingLevel> {
    GANN_SQUARE_GRID
        .iter()
        .map(|&(value, color)| level(value, color, true, false, false))
        .collect()
}

fn gann_arc_levels() -> Vec<DrawingLevel> {
    GANN_SQUARE_ARCS
        .iter()
        .map(|&(value, color)| level(value, color, true, true, false))
        .collect()
}

/// Three-anchor pitchfork behavior; every pitchfork spec below overrides its identity.
const PITCHFORK_TOOL: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::AndrewsPitchfork,
    wire_id: 96,
    name: "andrews_pitchfork",
    placement: DrawingPlacement::ClickAnchors { count: 3 },
    handles: DrawingHandleMode::Anchors,
    movement_axis: DrawingMovementAxis::Both,
    straighten: DrawingStraightenMode::None,
    // Tines, levels, arcs, and the fixed square reach past the anchors by a screen-derived
    // amount no data-space box bounds; the family's `paint_bounds` bounds them in px instead.
    logical_extent: DrawingLogicalExtent::Full,
    price_extent: DrawingPriceExtent::Full,
    bounds_padding_ratio: 0.0,
    default_width: 1.0,
    requests_text_editor: false,
    family: Some(&FAMILY),
    text_layout: DrawingTextLayout::Box,
    axis_price_label: false,
};

pub(crate) const ANDREWS_PITCHFORK: DrawingToolSpec = PITCHFORK_TOOL;

pub(crate) const SCHIFF_PITCHFORK: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::SchiffPitchfork,
    wire_id: 97,
    name: "schiff_pitchfork",
    ..PITCHFORK_TOOL
};

pub(crate) const MODIFIED_SCHIFF_PITCHFORK: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::ModifiedSchiffPitchfork,
    wire_id: 98,
    name: "modified_schiff_pitchfork",
    ..PITCHFORK_TOOL
};

pub(crate) const INSIDE_PITCHFORK: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::InsidePitchfork,
    wire_id: 99,
    name: "inside_pitchfork",
    ..PITCHFORK_TOOL
};

pub(crate) const PITCHFAN: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::Pitchfan,
    wire_id: 100,
    name: "pitchfan",
    ..PITCHFORK_TOOL
};

pub(crate) const GANN_BOX: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::GannBox,
    wire_id: 101,
    name: "gann_box",
    placement: DrawingPlacement::ClickAnchors { count: 2 },
    handles: DrawingHandleMode::RectangleBounds,
    straighten: DrawingStraightenMode::Square,
    ..PITCHFORK_TOOL
};

pub(crate) const GANN_SQUARE: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::GannSquare,
    wire_id: 102,
    name: "gann_square",
    ..GANN_BOX
};

pub(crate) const GANN_SQUARE_FIXED: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::GannSquareFixed,
    wire_id: 103,
    name: "gann_square_fixed",
    placement: DrawingPlacement::ClickAnchors { count: 1 },
    handles: DrawingHandleMode::Anchors,
    straighten: DrawingStraightenMode::None,
    ..PITCHFORK_TOOL
};

pub(crate) const GANN_FAN: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::GannFan,
    wire_id: 104,
    name: "gann_fan",
    placement: DrawingPlacement::ClickAnchors { count: 2 },
    straighten: DrawingStraightenMode::Segment45,
    ..PITCHFORK_TOOL
};

pub(crate) static FAMILY: DrawingFamily = {
    let mut family = DrawingFamily::new(build_parts, kind_options);
    family.apply_defaults = apply_defaults;
    family.decoration_extent = decoration_extent;
    family.extend_schema = extend_schema;
    family.paint_bounds = paint_bounds;
    family.handles = handles;
    family.drag = drag;
    family.rescale_price_options = rescale_price_options;
    family
};

/// A price-basis rescale scales the fan's and the fixed square's `scale_ratio` (price per bar)
/// with their pivot anchor, so the angles and the square keep measuring the same bars. A scaled
/// ratio outside `(0, MAX_SAFE_VALUE]` rejects the rescale.
fn rescale_price_options(
    kind: DrawingKind,
    options: &mut DrawingToolOptions,
    factor: f64,
    apply: bool,
) -> Option<bool> {
    if !matches!(kind, DrawingKind::GannFan | DrawingKind::GannSquareFixed) {
        return Some(false);
    }
    let Some(ratio) = options
        .gann
        .as_mut()
        .and_then(|gann| gann.scale_ratio.as_mut())
    else {
        return Some(false);
    };
    let scaled = *ratio * factor;
    if !scaled.is_finite() || scaled <= 0.0 || scaled > MAX_SAFE_VALUE {
        return None;
    }
    if apply {
        *ratio = scaled;
    }
    Some(true)
}

fn is_gann(kind: DrawingKind) -> bool {
    matches!(
        kind,
        DrawingKind::GannBox
            | DrawingKind::GannSquare
            | DrawingKind::GannSquareFixed
            | DrawingKind::GannFan
    )
}

fn apply_defaults(drawing: &mut Drawing) {
    drawing.fill_enabled = true;
    match drawing.kind {
        DrawingKind::GannBox => drawing.levels = gann_box_levels(),
        DrawingKind::GannSquare | DrawingKind::GannSquareFixed => {
            drawing.levels = gann_square_grid();
        }
        DrawingKind::GannFan => {
            drawing.levels = gann_angle_levels(true);
            drawing.extend_right = true;
        }
        _ => {
            drawing.color = MEDIAN_COLOR.to_string();
            drawing.levels = pitchfork_levels();
        }
    }
}

/// The drawing's Gann block, or the shared defaults (borrowed: part builds run every frame).
fn options(drawing: &Drawing) -> &GannToolOptions {
    static DEFAULTS: std::sync::LazyLock<GannToolOptions> =
        std::sync::LazyLock::new(GannToolOptions::default);
    drawing.tool_options.gann.as_ref().unwrap_or(&DEFAULTS)
}

// --- shared pieces ------------------------------------------------------------------------------

fn add(a: Point, b: Point) -> Point {
    (a.0 + b.0, a.1 + b.1)
}

fn sub(a: Point, b: Point) -> Point {
    (a.0 - b.0, a.1 - b.1)
}

fn scaled(a: Point, factor: f64) -> Point {
    (a.0 * factor, a.1 * factor)
}

fn degenerate(v: Point) -> bool {
    v.0.hypot(v.1) <= f64::EPSILON
}

fn level_stroke(level: &DrawingLevel, drawing: &Drawing) -> PartStroke {
    PartStroke {
        color: Some(level.stroke_color(drawing)),
        width: None,
        style: Some(level.line_style()),
    }
}

/// Visible levels with finite values accepted by `keep`, ordered by value.
fn visible_levels(levels: &[DrawingLevel], keep: impl Fn(f64) -> bool) -> Vec<&DrawingLevel> {
    let mut visible = levels
        .iter()
        .filter(|level| level.visible && level.value.is_finite() && keep(level.value))
        .collect::<Vec<_>>();
    visible.sort_by(|a, b| a.value.total_cmp(&b.value));
    visible
}

/// Ratio text of a level value: `0.382`, `1`, `1.5`.
fn value_text(value: f64) -> String {
    let rounded = (value * 1000.0).round() / 1000.0;
    format!("{}", rounded + 0.0)
}

/// Gann angle name of a slope multiple: `1x2` for 2, `2x1` for 0.5, `1x1` for 1.
fn angle_text(multiple: f64) -> String {
    if multiple >= 1.0 {
        format!("1x{}", value_text(multiple))
    } else {
        format!("{}x1", value_text(1.0 / multiple))
    }
}

/// Four significant digits for a price-per-bar ratio.
fn ratio_text(ratio: f64) -> String {
    if ratio == 0.0 || !ratio.is_finite() {
        return "0".to_string();
    }
    let digits = (3 - ratio.abs().log10().floor() as i32).clamp(0, 8) as usize;
    format!("{ratio:.digits$}")
}

/// One level label in the level's color at the drawing's label size (a body hit target).
fn level_label(
    ctx: &PartContext<'_>,
    anchor: Point,
    (h_align, v_align): (DrawingTextHAlign, DrawingTextVAlign),
    text: String,
    color: Color,
) -> PartLabel {
    PartLabel {
        anchor,
        h_align,
        v_align,
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

/// The pane's four corners, clockwise from the top-left.
fn pane_corners(pane: Rect) -> [Point; 4] {
    [
        (pane.left, pane.top),
        (pane.right, pane.top),
        (pane.right, pane.bottom),
        (pane.left, pane.bottom),
    ]
}

/// Fill a convex zone clipped to the pane. Like the rectangle's interior, a zone is a drag target
/// only while its drawing is selected. Zones with non-finite corners (absurd level values) are
/// skipped.
fn fill_clipped(ctx: &PartContext<'_>, parts: &mut DrawingParts, polygon: &[Point], color: Color) {
    if !polygon
        .iter()
        .all(|point| point.0.is_finite() && point.1.is_finite())
    {
        return;
    }
    let mut clipped = Vec::with_capacity(polygon.len() + 4);
    shape::clip_polygon_to_rect(polygon, ctx.pane, &mut clipped);
    if clipped.len() >= 3 {
        parts.fill_convex(&clipped, Some(color), ctx.fills_hit());
    }
}

/// A line from `start` along `direction` over `[0, 1]`, extended backward/forward to the pane
/// edge when asked (`shape::extend_segment`).
fn extended_line(
    ctx: &PartContext<'_>,
    start: Point,
    direction: Point,
    (back, forward): (bool, bool),
) -> Option<(Point, Point)> {
    if degenerate(direction) {
        return None;
    }
    Some(shape::extend_segment(
        start,
        add(start, direction),
        ctx.pane,
        back,
        forward,
    ))
}

/// The strip between the parallel lines through `inner` and `outer` along `direction` over
/// `[0, 1]`, widened to cover the pane on the extended sides, clipped to the pane.
fn fill_strip(
    ctx: &PartContext<'_>,
    parts: &mut DrawingParts,
    (inner, outer): (Point, Point),
    direction: Point,
    (back, forward): (bool, bool),
    color: Color,
) {
    let length_sq = direction.0 * direction.0 + direction.1 * direction.1;
    if length_sq <= f64::EPSILON {
        return;
    }
    let (mut t0, mut t1) = (0.0_f64, 1.0_f64);
    // Every pane point's projection parameter onto either line lies within the corners'.
    for corner in pane_corners(ctx.pane) {
        for base in [inner, outer] {
            let t =
                ((corner.0 - base.0) * direction.0 + (corner.1 - base.1) * direction.1) / length_sq;
            if back {
                t0 = t0.min(t);
            }
            if forward {
                t1 = t1.max(t);
            }
        }
    }
    let at = |point: Point, t: f64| add(point, scaled(direction, t));
    fill_clipped(
        ctx,
        parts,
        &[at(inner, t0), at(outer, t0), at(outer, t1), at(inner, t1)],
        color,
    );
}

/// The sector between the rays from `pivot` along `first` and `second` (less than half a turn
/// apart), out past the farthest pane corner, clipped to the pane.
fn fill_sector(
    ctx: &PartContext<'_>,
    parts: &mut DrawingParts,
    pivot: Point,
    (first, second): (Point, Point),
    color: Color,
) {
    let unit = |v: Point| {
        let length = v.0.hypot(v.1);
        (length > f64::EPSILON).then(|| scaled(v, 1.0 / length))
    };
    let (Some(a), Some(b)) = (unit(first), unit(second)) else {
        return;
    };
    let Some(middle) = unit(add(a, b)) else {
        return;
    };
    // With the middle vertex each half spans at most a quarter turn, so a radius of 1.5× the
    // farthest corner distance covers the sector inside the pane.
    let reach = pane_corners(ctx.pane)
        .iter()
        .map(|corner| (corner.0 - pivot.0).hypot(corner.1 - pivot.1))
        .fold(0.0_f64, f64::max);
    let radius = reach * 1.5 + 1.0;
    fill_clipped(
        ctx,
        parts,
        &[
            pivot,
            add(pivot, scaled(a, radius)),
            add(pivot, scaled(middle, radius)),
            add(pivot, scaled(b, radius)),
        ],
        color,
    );
}

// --- pitchforks ---------------------------------------------------------------------------------

/// One pitchfork frame in caller px: the median pivot, the base center, and the half-handle
/// vector (level `v` passes through `center ± v · half`).
#[derive(Clone, Copy, Debug)]
struct Fork {
    pivot: Point,
    center: Point,
    half: Point,
}

impl Fork {
    fn resolve(kind: DrawingKind, px: &[Point]) -> Option<Self> {
        let (&a, &b, &c) = (px.first()?, px.get(1)?, px.get(2)?);
        let andrews_half = scaled(sub(c, b), 0.5);
        Some(match kind {
            DrawingKind::SchiffPitchfork => Self {
                pivot: (a.0, (a.1 + b.1) / 2.0),
                center: shape::midpoint(b, c),
                half: andrews_half,
            },
            DrawingKind::ModifiedSchiffPitchfork => Self {
                pivot: shape::midpoint(a, b),
                center: shape::midpoint(b, c),
                half: andrews_half,
            },
            DrawingKind::InsidePitchfork => Self {
                pivot: shape::midpoint(a, b),
                center: c,
                half: sub(b, c),
            },
            // Andrews, and the pitchfan's base.
            _ => Self {
                pivot: a,
                center: shape::midpoint(b, c),
                half: andrews_half,
            },
        })
    }

    fn base(self, offset: f64) -> Point {
        add(self.center, scaled(self.half, offset))
    }

    fn direction(self) -> Point {
        sub(self.center, self.pivot)
    }
}

/// Visible pitchfork levels by absolute value (a level draws on both sides of the median).
fn fork_levels(drawing: &Drawing) -> Vec<&DrawingLevel> {
    let mut levels = visible_levels(&drawing.levels, |value| value != 0.0);
    levels.sort_by(|a, b| a.value.abs().total_cmp(&b.value.abs()));
    levels
}

/// Reach of the outermost visible level in half handles (the handle itself at least).
fn fork_reach(levels: &[&DrawingLevel]) -> f64 {
    levels
        .iter()
        .map(|level| level.value.abs())
        .fold(1.0, f64::max)
}

fn pitchfork_parts(ctx: &PartContext<'_>, parts: &mut DrawingParts) {
    let drawing = ctx.drawing;
    let Some(fork) = Fork::resolve(drawing.kind, ctx.px) else {
        return;
    };
    let direction = fork.direction();
    let extend = (drawing.extend_left, drawing.extend_right);
    let levels = fork_levels(drawing);
    if drawing.fill_enabled && !degenerate(direction) {
        let mut inner = 0.0;
        for level in &levels {
            let outer = level.value.abs();
            if level.fill_between && outer > inner {
                let color = level.zone_fill(drawing);
                for side in [-1.0, 1.0] {
                    let bases = (fork.base(side * inner), fork.base(side * outer));
                    fill_strip(ctx, parts, bases, direction, extend, color);
                }
            }
            inner = inner.max(outer);
        }
    }
    let reach = fork_reach(&levels);
    parts.stroke(
        &[fork.base(-reach), fork.base(reach)],
        PartStroke::default(),
        false,
    );
    if drawing.kind != DrawingKind::AndrewsPitchfork {
        // The swing the shifted pivot derives from, so the first anchor stays connected.
        parts.stroke(
            &ctx.px[..2],
            PartStroke::decoration(GUIDE_WIDTH, LineStyle::Dashed),
            false,
        );
    }
    if let Some((start, end)) = extended_line(ctx, fork.pivot, scaled(direction, 2.0), extend) {
        parts.stroke(&[start, end], PartStroke::default(), false);
    }
    for level in &levels {
        let stroke = level_stroke(level, drawing);
        for side in [-1.0, 1.0] {
            let base = fork.base(side * level.value.abs());
            if let Some((start, end)) = extended_line(ctx, base, direction, extend) {
                parts.stroke(&[start, end], stroke, false);
            }
            if level.label_visible && !degenerate(direction) {
                let end = add(base, direction);
                let forward = direction.0 >= 0.0;
                let gap = LABEL_GAP * ctx.scale;
                parts.label(level_label(
                    ctx,
                    (if forward { end.0 + gap } else { end.0 - gap }, end.1),
                    (
                        if forward {
                            DrawingTextHAlign::Left
                        } else {
                            DrawingTextHAlign::Right
                        },
                        DrawingTextVAlign::Middle,
                    ),
                    value_text(level.value.abs()),
                    level.stroke_color(drawing),
                ));
            }
        }
    }
}

fn pitchfan_parts(ctx: &PartContext<'_>, parts: &mut DrawingParts) {
    let drawing = ctx.drawing;
    let Some(fork) = Fork::resolve(DrawingKind::Pitchfan, ctx.px) else {
        return;
    };
    let apex = fork.pivot;
    let extend = (drawing.extend_left, drawing.extend_right);
    let levels = fork_levels(drawing);
    // Rays reach as far past the base as the apex sits before it.
    let ray = |offset: f64| scaled(sub(fork.base(offset), apex), 2.0);
    if drawing.fill_enabled {
        let mut inner = 0.0;
        for level in &levels {
            let outer = level.value.abs();
            if level.fill_between && outer > inner {
                let color = level.zone_fill(drawing);
                for side in [-1.0, 1.0] {
                    let (first, second) = (ray(side * inner), ray(side * outer));
                    if drawing.extend_right {
                        fill_sector(ctx, parts, apex, (first, second), color);
                    } else {
                        fill_clipped(
                            ctx,
                            parts,
                            &[apex, add(apex, first), add(apex, second)],
                            color,
                        );
                    }
                }
            }
            inner = inner.max(outer);
        }
    }
    let reach = fork_reach(&levels);
    parts.stroke(
        &[fork.base(-reach), fork.base(reach)],
        PartStroke::default(),
        false,
    );
    if let Some((start, end)) = extended_line(ctx, apex, ray(0.0), extend) {
        parts.stroke(&[start, end], PartStroke::default(), false);
    }
    for level in &levels {
        let stroke = level_stroke(level, drawing);
        for side in [-1.0, 1.0] {
            let direction = ray(side * level.value.abs());
            if let Some((start, end)) = extended_line(ctx, apex, direction, extend) {
                parts.stroke(&[start, end], stroke, false);
            }
            if level.label_visible && !degenerate(direction) {
                let end = add(apex, direction);
                let forward = direction.0 >= 0.0;
                let gap = LABEL_GAP * ctx.scale;
                parts.label(level_label(
                    ctx,
                    (if forward { end.0 + gap } else { end.0 - gap }, end.1),
                    (
                        if forward {
                            DrawingTextHAlign::Left
                        } else {
                            DrawingTextHAlign::Right
                        },
                        DrawingTextVAlign::Middle,
                    ),
                    value_text(level.value.abs()),
                    level.stroke_color(drawing),
                ));
            }
        }
    }
}

// --- Gann box -----------------------------------------------------------------------------------

/// The Gann box/square pivot corner and its opposite corner: the first anchor's corner, or with
/// `reverse` the one at the second anchor's price (the box also swaps time).
fn box_corners(kind: DrawingKind, px: &[Point], reverse: bool) -> Option<(Point, Point)> {
    let (&a, &b) = (px.first()?, px.get(1)?);
    Some(match (kind, reverse) {
        (_, false) => (a, b),
        (DrawingKind::GannBox, true) => (b, a),
        (_, true) => ((a.0, b.1), (b.0, a.1)),
    })
}

fn gann_box_parts(ctx: &PartContext<'_>, parts: &mut DrawingParts) {
    let drawing = ctx.drawing;
    let options = options(drawing);
    let Some((pivot, far)) = box_corners(drawing.kind, ctx.px, options.reverse) else {
        return;
    };
    let x_at = |value: f64| pivot.0 + (far.0 - pivot.0) * value;
    let y_at = |value: f64| pivot.1 + (far.1 - pivot.1) * value;
    let (left, right) = (pivot.0.min(far.0), pivot.0.max(far.0));
    let (top, bottom) = (pivot.1.min(far.1), pivot.1.max(far.1));
    let prices = visible_levels(&drawing.levels, |_| true);
    let times = visible_levels(&options.time_levels, |_| true);
    if drawing.fill_enabled {
        for pair in prices.windows(2) {
            if pair[1].fill_between {
                let (y0, y1) = (y_at(pair[0].value), y_at(pair[1].value));
                fill_clipped(
                    ctx,
                    parts,
                    &[(left, y0), (right, y0), (right, y1), (left, y1)],
                    pair[1].zone_fill(drawing),
                );
            }
        }
        for pair in times.windows(2) {
            if pair[1].fill_between {
                let (x0, x1) = (x_at(pair[0].value), x_at(pair[1].value));
                fill_clipped(
                    ctx,
                    parts,
                    &[(x0, top), (x1, top), (x1, bottom), (x0, bottom)],
                    pair[1].zone_fill(drawing),
                );
            }
        }
    }
    if options.show_angles {
        angle_fan(ctx, parts, pivot, far, &options.angles);
    }
    let gap = LABEL_GAP * ctx.scale;
    for level in &prices {
        let y = y_at(level.value);
        parts.hline(y, left, right, level_stroke(level, drawing));
        if level.label_visible {
            let color = level.stroke_color(drawing);
            let text = value_text(level.value);
            parts.label(level_label(
                ctx,
                (left - gap, y),
                (DrawingTextHAlign::Right, DrawingTextVAlign::Middle),
                text.clone(),
                color,
            ));
            parts.label(level_label(
                ctx,
                (right + gap, y),
                (DrawingTextHAlign::Left, DrawingTextVAlign::Middle),
                text,
                color,
            ));
        }
    }
    for level in &times {
        let x = x_at(level.value);
        parts.vline(x, top, bottom, level_stroke(level, drawing));
        if level.label_visible {
            let color = level.stroke_color(drawing);
            let text = value_text(level.value);
            parts.label(level_label(
                ctx,
                (x, top - gap),
                (DrawingTextHAlign::Center, DrawingTextVAlign::Bottom),
                text.clone(),
                color,
            ));
            parts.label(level_label(
                ctx,
                (x, bottom + gap),
                (DrawingTextHAlign::Center, DrawingTextVAlign::Top),
                text,
                color,
            ));
        }
    }
}

/// Gann angles from `pivot` inside the box to `far`: the multiple-`m` line runs along
/// `(dx, m · dy)` until it meets the box's far time or price edge.
fn angle_fan(
    ctx: &PartContext<'_>,
    parts: &mut DrawingParts,
    pivot: Point,
    far: Point,
    angles: &[DrawingLevel],
) {
    let (dx, dy) = sub(far, pivot);
    for level in visible_levels(angles, |value| value > 0.0) {
        let t = 1.0_f64.min(1.0 / level.value);
        let end = add(pivot, (dx * t, dy * level.value * t));
        parts.stroke(&[pivot, end], level_stroke(level, ctx.drawing), false);
    }
}

// --- Gann squares -------------------------------------------------------------------------------

/// The fixed square's far corner in caller px: `size_bars` right of the anchor, and up (down with
/// `reverse`) by `size_bars × scale_ratio` in price, or by the same screen length without a ratio.
fn fixed_far_corner(ctx: &PartContext<'_>, options: &GannToolOptions) -> Option<Point> {
    let anchor = *ctx.drawing.points.first()?;
    let pivot = *ctx.px.first()?;
    let bars = options.size_bars;
    let (x, _) = ctx.point_px(DrawingPoint {
        logical: anchor.logical + bars,
        price: anchor.price,
    })?;
    let y = match options.scale_ratio {
        Some(ratio) => {
            let rise = bars * ratio;
            ctx.point_px(DrawingPoint {
                logical: anchor.logical,
                price: anchor.price + if options.reverse { -rise } else { rise },
            })?
            .1
        }
        None => {
            let height = (x - pivot.0).abs() / ctx.x_scale * ctx.scale;
            if options.reverse {
                pivot.1 + height
            } else {
                pivot.1 - height
            }
        }
    };
    Some((x, y))
}

fn square_corners(ctx: &PartContext<'_>, options: &GannToolOptions) -> Option<(Point, Point)> {
    if ctx.drawing.kind == DrawingKind::GannSquareFixed {
        Some((*ctx.px.first()?, fixed_far_corner(ctx, options)?))
    } else {
        box_corners(ctx.drawing.kind, ctx.px, options.reverse)
    }
}

/// `segments + 1` points of the quarter ellipse around `pivot` from `(pivot.x + rx, pivot.y)`
/// to `(pivot.x, pivot.y + ry)` (signed radii follow the square's direction).
fn quarter_arc(pivot: Point, (rx, ry): Point, segments: usize, out: &mut Vec<Point>) {
    out.clear();
    for step in 0..=segments {
        let angle = std::f64::consts::FRAC_PI_2 * step as f64 / segments as f64;
        let (sin, cos) = angle.sin_cos();
        out.push((pivot.0 + rx * cos, pivot.1 + ry * sin));
    }
}

fn square_parts(ctx: &PartContext<'_>, parts: &mut DrawingParts) {
    let drawing = ctx.drawing;
    let options = options(drawing);
    let Some((pivot, far)) = square_corners(ctx, options) else {
        return;
    };
    let (dx, dy) = sub(far, pivot);
    let arcs = visible_levels(&options.arcs, |value| value > 0.0);
    if !arcs.is_empty() {
        // One chord count for every arc (the outermost's), so paired arc chains fill cleanly.
        let outer = arcs.last().map_or(1.0, |level| level.value);
        let segments = shape::arc_segment_count(
            (outer * dx).abs().max((outer * dy).abs()),
            std::f64::consts::FRAC_PI_2,
            CURVE_TOLERANCE,
        );
        let (mut inner_arc, mut arc) = (vec![pivot; segments + 1], Vec::new());
        let hit = ctx.fills_hit();
        for level in &arcs {
            quarter_arc(
                pivot,
                (dx * level.value, dy * level.value),
                segments,
                &mut arc,
            );
            if !arc
                .iter()
                .all(|point| point.0.is_finite() && point.1.is_finite())
            {
                break;
            }
            if drawing.fill_enabled && level.fill_between {
                parts.fill(&arc, &inner_arc, Some(level.zone_fill(drawing)), hit);
            }
            parts.stroke(&arc, level_stroke(level, drawing), false);
            std::mem::swap(&mut inner_arc, &mut arc);
        }
    }
    let (left, right) = (pivot.0.min(far.0), pivot.0.max(far.0));
    let (top, bottom) = (pivot.1.min(far.1), pivot.1.max(far.1));
    for level in visible_levels(&drawing.levels, |_| true) {
        let stroke = level_stroke(level, drawing);
        parts.vline(pivot.0 + dx * level.value, top, bottom, stroke);
        parts.hline(pivot.1 + dy * level.value, left, right, stroke);
    }
    angle_fan(ctx, parts, pivot, far, &options.angles);
    if options.show_stats {
        square_stats(ctx, parts, options, pivot, far);
    }
}

/// The square's price range, bars, and price per bar, in caller px of `far`.
fn square_stat_lines(
    ctx: &PartContext<'_>,
    options: &GannToolOptions,
    pivot: Point,
    far: Point,
) -> Option<Vec<String>> {
    let engine = ctx.engine;
    let drawing = ctx.drawing;
    let (bars, range) = if drawing.kind == DrawingKind::GannSquareFixed {
        let range = match options.scale_ratio {
            Some(ratio) => options.size_bars * ratio,
            None => {
                let price = |point: Point| {
                    engine
                        .drawing_from_px_for(
                            drawing.pane_index,
                            drawing.price_scale,
                            point.0 / ctx.x_scale,
                            point.1 / ctx.scale,
                        )
                        .map(|point| point.price)
                };
                (price(far)? - price(pivot)?).abs()
            }
        };
        (options.size_bars, range)
    } else {
        let (a, b) = (drawing.points.first()?, drawing.points.get(1)?);
        ((b.logical - a.logical).abs(), (b.price - a.price).abs())
    };
    let mut lines = vec![
        engine.drawing_price_text(drawing, range),
        format!("{} bars", bars.round() as i64),
    ];
    if bars > f64::EPSILON {
        lines.push(format!("{}/bar", ratio_text(range / bars)));
    }
    Some(lines)
}

fn square_stats(
    ctx: &PartContext<'_>,
    parts: &mut DrawingParts,
    options: &GannToolOptions,
    pivot: Point,
    far: Point,
) {
    let Some(lines) = square_stat_lines(ctx, options, pivot, far) else {
        return;
    };
    let forward = far.0 >= pivot.0;
    let gap = STATS_GAP * ctx.scale;
    let base = ctx.drawing.stroke_color();
    let background = Color::rgba(base.r(), base.g(), base.b(), STATS_ALPHA);
    let h_align = if forward {
        DrawingTextHAlign::Left
    } else {
        DrawingTextHAlign::Right
    };
    parts.label(ctx.stats_label(
        (if forward { far.0 + gap } else { far.0 - gap }, far.1),
        (h_align, DrawingTextVAlign::Middle),
        lines,
        background,
        text_on(background),
    ));
}

// --- Gann fan -----------------------------------------------------------------------------------

/// The fan's 1×1 vector in caller px: toward the second anchor, or `scale_ratio` price per bar
/// over the second anchor's bar distance in its price direction.
fn fan_unit(ctx: &PartContext<'_>, options: &GannToolOptions) -> Option<Point> {
    let (&o, &b) = (ctx.px.first()?, ctx.px.get(1)?);
    let Some(ratio) = options.scale_ratio else {
        return Some(sub(b, o));
    };
    let (first, second) = (ctx.drawing.points.first()?, ctx.drawing.points.get(1)?);
    let bars = second.logical - first.logical;
    if bars.abs() <= f64::EPSILON {
        return Some(sub(b, o));
    }
    let rise = bars.abs() * ratio;
    let target = ctx.point_px(DrawingPoint {
        logical: second.logical,
        price: first.price
            + if second.price < first.price {
                -rise
            } else {
                rise
            },
    })?;
    Some(sub(target, o))
}

/// One resolved Gann fan in caller px: the pivot, the second anchor (the anchors' box corner), and
/// the 1×1 vector.
#[derive(Clone, Copy, Debug)]
struct Fan {
    pivot: Point,
    corner: Point,
    unit: Point,
}

impl Fan {
    fn resolve(ctx: &PartContext<'_>) -> Option<Self> {
        let (&pivot, &corner) = (ctx.px.first()?, ctx.px.get(1)?);
        let unit = fan_unit(ctx, options(ctx.drawing))?;
        (!degenerate(unit)).then_some(Self {
            pivot,
            corner,
            unit,
        })
    }

    /// The multiple-`m` line: its direction `(ux, m · uy)`, where it leaves the anchors' box, and
    /// whether it leaves through the far time edge (vertical) rather than the far price edge. A
    /// flat box (no price edge) ends every line on the time edge.
    fn line(self, multiple: f64) -> (Point, Point, bool) {
        let (unit, extent) = (self.unit, sub(self.corner, self.pivot));
        let direction = (unit.0, unit.1 * multiple);
        let along_x = (unit.0.abs() > f64::EPSILON).then(|| (extent.0 / unit.0).abs());
        let along_y = (unit.1.abs() > f64::EPSILON && extent.1.abs() > f64::EPSILON)
            .then(|| (extent.1 / (unit.1 * multiple)).abs());
        let (t, time_edge) = match (along_x, along_y) {
            (Some(x), Some(y)) if y < x => (y, false),
            (Some(x), _) => (x, true),
            (None, Some(y)) => (y, false),
            (None, None) => (1.0, true),
        };
        (direction, add(self.pivot, scaled(direction, t)), time_edge)
    }
}

fn gann_fan_parts(ctx: &PartContext<'_>, parts: &mut DrawingParts) {
    let drawing = ctx.drawing;
    let Some(fan) = Fan::resolve(ctx) else {
        return;
    };
    let (pivot, corner, unit) = (fan.pivot, fan.corner, fan.unit);
    let extent = sub(corner, pivot);
    let extend = (drawing.extend_left, drawing.extend_right);
    let angles = visible_levels(&drawing.levels, |value| value > 0.0);
    if drawing.fill_enabled {
        for pair in angles.windows(2) {
            if !pair[1].fill_between {
                continue;
            }
            let color = pair[1].zone_fill(drawing);
            let (first, first_end, first_time) = fan.line(pair[0].value);
            let (second, second_end, second_time) = fan.line(pair[1].value);
            if drawing.extend_right {
                fill_sector(ctx, parts, pivot, (first, second), color);
            } else if first_time && !second_time {
                // The rays leave through different edges: the box corner closes the sector.
                fill_clipped(ctx, parts, &[pivot, first_end, corner, second_end], color);
            } else {
                fill_clipped(ctx, parts, &[pivot, first_end, second_end], color);
            }
        }
    }
    let gap = LABEL_GAP * ctx.scale;
    for level in &angles {
        let (direction, end, time_edge) = fan.line(level.value);
        let stroke = level_stroke(level, drawing);
        let segment = if extend.0 || extend.1 {
            extended_line(ctx, pivot, direction, extend)
        } else {
            Some((pivot, end))
        };
        if let Some((start, finish)) = segment {
            parts.stroke(&[start, finish], stroke, false);
        }
        if level.label_visible {
            let color = level.stroke_color(drawing);
            let (anchor, align) = if time_edge {
                let forward = extent.0 >= 0.0;
                (
                    (if forward { end.0 + gap } else { end.0 - gap }, end.1),
                    (
                        if forward {
                            DrawingTextHAlign::Left
                        } else {
                            DrawingTextHAlign::Right
                        },
                        DrawingTextVAlign::Middle,
                    ),
                )
            } else {
                let upward = unit.1 * level.value <= 0.0;
                (
                    (end.0, if upward { end.1 - gap } else { end.1 + gap }),
                    (
                        DrawingTextHAlign::Center,
                        if upward {
                            DrawingTextVAlign::Bottom
                        } else {
                            DrawingTextVAlign::Top
                        },
                    ),
                )
            };
            parts.label(level_label(
                ctx,
                anchor,
                align,
                angle_text(level.value),
                color,
            ));
        }
    }
}

// --- hooks --------------------------------------------------------------------------------------

fn build_parts(ctx: &PartContext<'_>, parts: &mut DrawingParts) {
    match ctx.drawing.kind {
        DrawingKind::Pitchfan => pitchfan_parts(ctx, parts),
        DrawingKind::GannBox => gann_box_parts(ctx, parts),
        DrawingKind::GannSquare | DrawingKind::GannSquareFixed => square_parts(ctx, parts),
        DrawingKind::GannFan => gann_fan_parts(ctx, parts),
        _ => pitchfork_parts(ctx, parts),
    }
}

/// Whether `kind` resolves a pitchfork frame from three anchors (the pitchforks and the
/// pitchfan).
fn is_fork(kind: DrawingKind) -> bool {
    matches!(
        kind,
        DrawingKind::AndrewsPitchfork
            | DrawingKind::SchiffPitchfork
            | DrawingKind::ModifiedSchiffPitchfork
            | DrawingKind::InsidePitchfork
            | DrawingKind::Pitchfan
    )
}

/// Derived handles: a pitchfork's (and the pitchfan's) base midpoint between its handle anchors
/// B and C, which moves both together, and the fixed Gann square's far corner, which resizes it.
fn handles(
    engine: &ChartEngine,
    drawing: &Drawing,
    px: &[Point],
    handles: &mut Vec<DrawingHandle>,
) {
    let (point, cursor) = if is_fork(drawing.kind) {
        let (Some(&b), Some(&c)) = (px.get(1), px.get(2)) else {
            return;
        };
        (shape::midpoint(b, c), "pointer")
    } else if drawing.kind == DrawingKind::GannSquareFixed {
        let Some(corner) = PartContext::media(engine, drawing, px)
            .and_then(|ctx| fixed_far_corner(&ctx, options(drawing)))
        else {
            return;
        };
        let cursor = if options(drawing).reverse {
            "nwse-resize"
        } else {
            "nesw-resize"
        };
        (corner, cursor)
    } else {
        return;
    };
    handles.push(DrawingHandle {
        point,
        part: DrawingDragPart::Handle(0),
        cursor,
        shape: HandleShape::Disc,
    });
}

/// Derived-handle drags. The pitchfork base midpoint translates both handle anchors by the
/// midpoint's (magnet-snapped) move. The fixed square's corner sets the side in whole bars from
/// the corner's time and grows the square toward the corner's side of the anchor; with a
/// `scale_ratio` the corner's price sets the ratio (Shift keeps it), and without one the
/// square stays square on screen, sized by the corner's larger distance from the anchor. A
/// pointer rounds the side to the nearest bar; a keyboard step moves it at least one whole bar
/// the way the key moved (sized by the moved axis without a ratio), so sub-bar steps still
/// resize the square and a step along the other axis keeps the side.
fn drag(
    engine: &ChartEngine,
    drawing: &Drawing,
    sample: &HandleDrag<'_>,
    points: &mut [DrawingPoint],
) -> Option<DrawingToolOptions> {
    if sample.part != DrawingDragPart::Handle(0) {
        return None;
    }
    if is_fork(drawing.kind) {
        let (&b, &c) = (sample.start_px.get(1)?, sample.start_px.get(2)?);
        let from = shape::midpoint(b, c);
        let delta = sub(sample.target_px, from);
        let moved_b = engine.drawing_anchor_at(drawing, add(b, delta))?;
        let moved_c = engine.drawing_anchor_at(drawing, add(c, delta))?;
        points[1] = moved_b;
        points[2] = moved_c;
        return None;
    }
    if drawing.kind != DrawingKind::GannSquareFixed {
        return None;
    }
    let (&anchor, &pivot) = (sample.start_points.first()?, sample.start_px.first()?);
    let mut gann = sample.start_tool_options.gann.clone().unwrap_or_default();
    let (dx, dy) = sub(sample.target_px, pivot);
    let bars = match gann.scale_ratio {
        Some(_) => sample.target.logical - anchor.logical,
        None => {
            // Square on screen: the larger distance wins (the rectangle's Shift-square rule); a
            // keyboard step sizes by the axis it moved, so it can shrink the square too.
            let side = match sample.keyboard_step {
                Some((step_x, step_y)) if step_y.abs() > step_x.abs() => dy.abs(),
                Some(_) => dx.max(0.0),
                None => dx.max(0.0).max(dy.abs()),
            };
            engine
                .drawing_anchor_at(drawing, (pivot.0 + side, pivot.1))?
                .logical
                - anchor.logical
        }
    };
    if !bars.is_finite() {
        return None;
    }
    // Float noise from the px round trip never counts as a keyboard step.
    const STEP_EPSILON: f64 = 1e-6;
    let side = gann.size_bars;
    let bars = match sample.keyboard_step {
        None => bars.round(),
        Some(_) if bars > side + STEP_EPSILON => (bars - STEP_EPSILON).ceil(),
        Some(_) if bars < side - STEP_EPSILON => (bars + STEP_EPSILON).floor(),
        Some(_) => side,
    };
    gann.size_bars = bars.clamp(1.0, MAX_GANN_SQUARE_BARS);
    gann.reverse = dy > 0.0;
    if let (Some(_), false) = (gann.scale_ratio, sample.straighten) {
        let ratio = (sample.target.price - anchor.price).abs() / gann.size_bars;
        if ratio.is_finite() && ratio > 0.0 {
            gann.scale_ratio = Some(ratio);
        }
    }
    let mut tool_options = drawing.tool_options.clone();
    tool_options.gann = Some(gann);
    Some(tool_options)
}

/// Media-px box of every stroke and fill (text pads through `decoration_extent`); an extended
/// drawing reaches the pane edge and keeps the whole pane.
fn paint_bounds(engine: &ChartEngine, drawing: &Drawing, px: &[Point]) -> Option<Rect> {
    if drawing.extend_left || drawing.extend_right {
        return None;
    }
    let ctx = PartContext::media(engine, drawing, px)?;
    let mut points = px.to_vec();
    match drawing.kind {
        DrawingKind::GannBox | DrawingKind::GannSquare | DrawingKind::GannSquareFixed => {
            let options = options(drawing);
            let (pivot, far) = if drawing.kind == DrawingKind::GannBox {
                box_corners(drawing.kind, px, options.reverse)?
            } else {
                square_corners(&ctx, options)?
            };
            let delta = sub(far, pivot);
            // Levels and arcs may reach past the box; angles stay inside it.
            let mut low = 0.0_f64;
            let mut high = 1.0_f64;
            let lists: [&[DrawingLevel]; 3] =
                [&drawing.levels, &options.time_levels, &options.arcs];
            for level in lists.into_iter().flatten().filter(|level| level.visible) {
                low = low.min(level.value);
                high = high.max(level.value);
            }
            points.push(add(pivot, scaled(delta, low)));
            points.push(add(pivot, scaled(delta, high)));
        }
        DrawingKind::GannFan => {
            // Lines stop where they leave the anchors' box, except on a flat box with a scale
            // ratio, where they reach the time edge at their own slope.
            if let Some(fan) = Fan::resolve(&ctx) {
                for level in visible_levels(&drawing.levels, |value| value > 0.0) {
                    points.push(fan.line(level.value).1);
                }
            }
        }
        DrawingKind::Pitchfan => {
            let fork = Fork::resolve(drawing.kind, px)?;
            let reach = fork_reach(&fork_levels(drawing));
            for offset in [-reach, reach] {
                let base = fork.base(offset);
                points.push(base);
                points.push(add(fork.pivot, scaled(sub(base, fork.pivot), 2.0)));
            }
        }
        _ => {
            let fork = Fork::resolve(drawing.kind, px)?;
            let reach = fork_reach(&fork_levels(drawing));
            let direction = fork.direction();
            points.push(fork.pivot);
            points.push(add(fork.center, direction));
            for offset in [-reach, reach] {
                let base = fork.base(offset);
                points.push(base);
                points.push(add(base, direction));
            }
        }
    }
    Rect::bounding(&points)
}

/// Conservative CSS-px reach of labels and the stats box beyond the painted geometry. Stats that
/// follow the viewport (a screen square's price range) get four ems of slack.
fn decoration_extent(engine: &ChartEngine, drawing: &Drawing) -> f64 {
    let family = &engine.options.get().layout.font_family;
    let size = engine.drawing_text_size(drawing);
    let weight = drawing.text_weight.unwrap_or(400);
    let options = options(drawing);
    let mut extent: f64 = 0.0;
    let mut measure_levels = |levels: &[DrawingLevel], text: fn(f64) -> String| {
        for level in levels
            .iter()
            .filter(|level| level.visible && level.label_visible)
        {
            let width = engine.measure_text_run(
                &text(level.value),
                size,
                family,
                weight,
                drawing.text_italic,
            );
            extent = extent.max(LABEL_GAP + width + size);
        }
    };
    match drawing.kind {
        DrawingKind::GannFan => measure_levels(&drawing.levels, angle_text),
        DrawingKind::GannBox => {
            measure_levels(&drawing.levels, value_text);
            measure_levels(&options.time_levels, value_text);
        }
        DrawingKind::GannSquare | DrawingKind::GannSquareFixed => {}
        _ => measure_levels(&drawing.levels, |value| value_text(value.abs())),
    }
    if matches!(
        drawing.kind,
        DrawingKind::GannSquare | DrawingKind::GannSquareFixed
    ) && options.show_stats
    {
        let size = engine.drawing_stats_size();
        let width = ["000000.00", "00000 bars", "0000.000/bar"]
            .iter()
            .map(|line| engine.measure_text_run(line, size, family, 400, false))
            .fold(0.0_f64, f64::max);
        extent = extent.max(STATS_GAP + width + 2.0 * STATS_PADDING.0 + 4.0 * size);
        extent = extent.max(STATS_GAP + 3.0 * size * 1.25 + 2.0 * STATS_PADDING.1);
    }
    extent
}

fn descriptor(
    field: &str,
    property_type: DrawingPropertyType,
    default: serde_json::Value,
) -> DrawingPropertyDescriptor {
    crate::drawing_contract::descriptor(
        format!("tool_options.gann.{field}"),
        property_type,
        default,
    )
}

fn extend_schema(template: &Drawing, properties: &mut Vec<DrawingPropertyDescriptor>) {
    let defaults = GannToolOptions::default();
    let levels = |levels: &[DrawingLevel]| serde_json::to_value(levels).unwrap_or_default();
    let fields: &[&str] = match template.kind {
        DrawingKind::GannBox => &["time_levels", "angles", "reverse", "show_angles"],
        DrawingKind::GannSquare => &["angles", "arcs", "reverse", "show_stats"],
        DrawingKind::GannSquareFixed => &[
            "angles",
            "arcs",
            "reverse",
            "show_stats",
            "size_bars",
            "scale_ratio",
        ],
        DrawingKind::GannFan => &["scale_ratio"],
        _ => &[],
    };
    for &field in fields {
        properties.push(match field {
            "time_levels" => descriptor(
                field,
                DrawingPropertyType::Levels,
                levels(&defaults.time_levels),
            ),
            "angles" => descriptor(field, DrawingPropertyType::Levels, levels(&defaults.angles)),
            "arcs" => descriptor(field, DrawingPropertyType::Levels, levels(&defaults.arcs)),
            "reverse" => descriptor(
                field,
                DrawingPropertyType::Boolean,
                serde_json::json!(defaults.reverse),
            ),
            "show_angles" => descriptor(
                field,
                DrawingPropertyType::Boolean,
                serde_json::json!(defaults.show_angles),
            ),
            "show_stats" => descriptor(
                field,
                DrawingPropertyType::Boolean,
                serde_json::json!(defaults.show_stats),
            ),
            "size_bars" => DrawingPropertyDescriptor {
                min: Some(1.0),
                max: Some(MAX_GANN_SQUARE_BARS),
                ..descriptor(
                    field,
                    DrawingPropertyType::Number,
                    serde_json::json!(defaults.size_bars),
                )
            },
            _ => descriptor(field, DrawingPropertyType::Number, serde_json::Value::Null),
        });
    }
}

fn kind_options(drawing: &Drawing) -> DrawingKindOptions {
    if !is_gann(drawing.kind) {
        return DrawingKindOptions::Pitchfork {
            levels: drawing.levels.clone(),
        };
    }
    let options = options(drawing).clone();
    DrawingKindOptions::Gann {
        levels: drawing.levels.clone(),
        time_levels: options.time_levels,
        angles: options.angles,
        arcs: options.arcs,
        reverse: options.reverse,
        show_angles: options.show_angles,
        show_stats: options.show_stats,
        scale_ratio: options.scale_ratio,
        size_bars: options.size_bars,
    }
}

#[cfg(test)]
mod tests;
