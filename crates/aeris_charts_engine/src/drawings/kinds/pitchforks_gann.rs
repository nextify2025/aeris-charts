//! B8 Pitchforks & Gann options, legacy defaults, and the fork features re-applied on upstream's
//! lowering. Upstream renders the Andrews, Schiff, modified Schiff, and inside pitchforks, the
//! pitchfan, the Gann box, square, fixed square, and fan from its catalog spec and the flat level
//! fields (`geometry.rs`, the frame's `Pitchfork`, `GannGrid` and Fibonacci arms, and
//! `drawing_body_hit`). This module keeps the fork's public option block ([`GannToolOptions`],
//! [`MAX_GANN_SQUARE_BARS`]) and, for documents the fork wrote, its pre-merge kind defaults
//! ([`legacy_defaults`]); and it owns what those arms read from the block: the Gann box's own
//! time levels ([`box_time_levels`]) and angles ([`angle_levels`]), the squares' stats box
//! ([`square_stats`]), the fan's and fixed square's scale ratio ([`ratio_point`], rescaled with the
//! price basis by [`rescale_scale_ratio`]), and the derived handles with their drags (the
//! pitchforks' base midpoint and the fixed square's corner: [`derived_handles`],
//! [`drag_handle`]).

use std::sync::LazyLock;

use aeris_charts_core::model::data_validation::MAX_SAFE_VALUE;
use aeris_charts_render::color::Color;
use aeris_charts_render::draw_list::LineStyle;
use aeris_charts_render::shape::{self, Point};

use super::super::geometry::{GannGridGeometry, gann_fixed_end};
use super::super::handles::{DrawingHandle, HandleDrag, HandleShape};
use super::super::parts::{DrawingParts, PartContext, STATS_GAP, STATS_PADDING, text_on};
use super::super::{Drawing, DrawingTextHAlign, DrawingTextVAlign};
use crate::{
    ChartEngine, DrawingDragPart, DrawingKind, DrawingLevel, DrawingPoint,
    DrawingPropertyDescriptor, DrawingPropertyType, DrawingToolOptions, MAX_DRAWING_LEVELS,
};

/// Upper bound on the fixed Gann square's side in bars.
pub const MAX_GANN_SQUARE_BARS: f64 = 100_000.0;
/// The fixed Gann square's default side in bars.
const DEFAULT_SQUARE_BARS: f64 = 20.0;

/// The fork's Gann-tool options (`tool_options.gann`); absent fields keep their defaults. Upstream
/// renders the Gann tools from the flat fields, and some keys are input aliases of them: `reverse`
/// of `level_reverse`, and on the squares `angles` and `arcs` of `gann_fans` and `gann_arcs` (see
/// `drawing_contract::take_legacy_flat_options`). Upstream's arms read the rest: the box's
/// `time_levels`, `angles` and `show_angles`, the squares' `show_stats`, and the fan's and fixed
/// square's `scale_ratio`. Only `size_bars` stays stored, read when a fork document's one-anchor
/// square converts (decision G3).
/// The defaults of the fields that switch a look on (`time_levels`, `show_stats`) are upstream's
/// look, so a block a patch creates for one key switches nothing else on; documents the fork
/// wrote get the fork's values through `kinds::legacy_fork_tool_options`.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct GannToolOptions {
    /// Gann box: vertical levels as fractions of the box width from the pivot corner, painted
    /// with their own labels above the box; the drawing's `levels` are then its horizontal price
    /// levels, and the zone fills become overlapping per-axis bands (price bands across the box's
    /// width, time bands across its height). Empty (the default): the vertical levels follow
    /// `levels`, as upstream draws them, with its diagonal cells.
    pub time_levels: Vec<DrawingLevel>,
    /// Gann box (with `show_angles`): angle lines from the pivot corner, each a multiple of the
    /// 1×1 slope (the box diagonal) running to the box edge, unfilled; on the squares an input
    /// alias of `gann_fans`. Values must be positive.
    pub angles: Vec<DrawingLevel>,
    /// Gann squares: quarter arcs around the pivot corner, radii as fractions of the side;
    /// values must be positive.
    pub arcs: Vec<DrawingLevel>,
    /// Gann box and squares: measure from the second anchor's price instead of the first's (the
    /// box also counts time from the second anchor); the fixed square grows downward.
    pub reverse: bool,
    /// Gann box: paint `angles` from the pivot corner.
    pub show_angles: bool,
    /// Gann squares: the measurement box beside the far corner (price range, bars, and price per
    /// bar). Default false; documents the fork wrote default to true.
    pub show_stats: bool,
    /// Gann fan and fixed square: price units per bar of the 1×1 angle. The fan's 1×1 then runs
    /// to the second anchor's bar at that slope (in the second anchor's price direction), and the
    /// fixed square's far corner sits at the second anchor's bar and the first anchor's price
    /// plus or minus the bars times the ratio. `None` makes the fan's 1×1 pass through its second
    /// anchor and the fixed square a square on screen (upstream's). A price-basis rescale scales
    /// it with the anchors.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scale_ratio: Option<f64>,
    /// Fixed square: the fork's side length in bars (1..=[`MAX_GANN_SQUARE_BARS`]), read only when
    /// a fork document's one-anchor square converts to two anchors; the anchors carry the size
    /// since (no numeric size property, decision G3).
    pub size_bars: f64,
}

impl Default for GannToolOptions {
    fn default() -> Self {
        Self {
            time_levels: Vec::new(),
            angles: gann_angle_levels(false),
            arcs: gann_arc_levels(),
            reverse: false,
            show_angles: false,
            show_stats: false,
            scale_ratio: None,
            size_bars: DEFAULT_SQUARE_BARS,
        }
    }
}

impl GannToolOptions {
    /// Bounded lists of valid levels (positive angle and arc values), a scale ratio in
    /// `(0, MAX_SAFE_VALUE]` (the range a price-basis rescale and the corner drag keep), and a
    /// side in `1..=MAX_GANN_SQUARE_BARS`.
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
                .is_none_or(|ratio| ratio > 0.0 && ratio <= MAX_SAFE_VALUE)
            && self.size_bars.is_finite()
            && (1.0..=MAX_GANN_SQUARE_BARS).contains(&self.size_bars)
    }
}

/// Median color of the fork's pitchforks and pitchfan (TradingView's red median).
const MEDIAN_COLOR: &str = "#f23645";

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

/// The fork's pre-merge defaults of the pitchfork and Gann tools (see
/// [`super::apply_legacy_fork_defaults`]): zone fills on, its level lists, the Gann fan extended
/// to the right, and the pitchforks' red median.
pub(super) fn legacy_defaults(drawing: &mut Drawing) {
    match drawing.kind {
        DrawingKind::GannBox => drawing.levels = gann_box_levels(),
        DrawingKind::GannSquare | DrawingKind::GannSquareFixed => {
            drawing.levels = gann_square_grid();
        }
        DrawingKind::GannFan => {
            drawing.levels = gann_angle_levels(true);
            drawing.extend_right = true;
        }
        DrawingKind::AndrewsPitchfork
        | DrawingKind::SchiffPitchfork
        | DrawingKind::ModifiedSchiffPitchfork
        | DrawingKind::InsidePitchfork
        | DrawingKind::Pitchfan => {
            drawing.color = MEDIAN_COLOR.to_string();
            drawing.levels = pitchfork_levels();
        }
        _ => return,
    }
    drawing.fill_enabled = true;
}

/// The fork's unstored `tool_options.gann` defaults (see `kinds::legacy_fork_tool_options`): the
/// Gann box's own time levels and the squares' stats box.
pub(super) fn legacy_tool_options(kind: DrawingKind) -> Option<(&'static str, serde_json::Value)> {
    let block = match kind {
        DrawingKind::GannBox => serde_json::json!({"time_levels": gann_box_levels()}),
        DrawingKind::GannSquare | DrawingKind::GannSquareFixed => {
            serde_json::json!({"show_stats": true})
        }
        _ => return None,
    };
    Some(("gann", block))
}

/// Upstream's band fill alpha over a level's color (its `Pitchfork` frame arm).
const UPSTREAM_BAND_ALPHA: u8 = 35;

/// Convert the levels of a pitchfork or pitchfan the fork wrote to upstream's meaning (a no-op for
/// every other kind). The fork drew each visible level `v` on both sides of an always-drawn
/// median, `|v|` half-handle widths from it, and filled the band between consecutive visible
/// levels (by `|v|`) in the outer level's fill. Upstream places a level along the handle from its
/// second anchor (0) to its third (1), with the median at 0.5, and fills the band between
/// consecutive visible levels of the list in the later level's fill (a hidden level ends the
/// chain). So each fork level becomes the two levels `0.5 ∓ |v|/2` (for every kind: the inside
/// pitchfork's doubled handle and the pitchfan's rays included), and a median level 0.5 in the
/// drawing's color and style joins them. The visible levels come first, ascending, then the
/// hidden ones; on the lower side each visible level takes the fill of the next visible level
/// outward, and the median that of the innermost one, so every band keeps the fork's fill. A
/// fill the shift moves onto another level's color is written as an explicit `fill_color`.
/// Zero levels (which the fork never drew) drop out, and at most 31 fork levels convert, the
/// visible ones first (innermost first), which keeps the list within [`MAX_DRAWING_LEVELS`].
pub(crate) fn legacy_levels_to_upstream(drawing: &mut Drawing) {
    if !matches!(
        drawing.kind,
        DrawingKind::AndrewsPitchfork
            | DrawingKind::SchiffPitchfork
            | DrawingKind::ModifiedSchiffPitchfork
            | DrawingKind::InsidePitchfork
            | DrawingKind::Pitchfan
    ) {
        return;
    }
    let mut fork = drawing
        .levels
        .iter()
        .filter(|level| level.value.is_finite() && level.value != 0.0)
        .collect::<Vec<_>>();
    fork.sort_by(|a, b| a.value.abs().total_cmp(&b.value.abs()));
    let (mut visible, mut hidden): (Vec<&DrawingLevel>, Vec<&DrawingLevel>) =
        fork.into_iter().partition(|level| level.visible);
    let cap = (MAX_DRAWING_LEVELS - 1) / 2;
    visible.truncate(cap);
    hidden.truncate(cap - visible.len());
    let side = |level: &DrawingLevel, sign: f64| DrawingLevel {
        value: 0.5 + sign * level.value.abs() / 2.0,
        ..level.clone()
    };
    // `level` with the fill of `source`, which upstream would otherwise derive from `level`'s own
    // color.
    let filled_as = |level: DrawingLevel, source: Option<&DrawingLevel>| {
        let Some(source) = source else {
            return DrawingLevel {
                fill_between: false,
                ..level
            };
        };
        let fill_color = source.fill_color.clone().or_else(|| {
            (source.color != level.color).then(|| {
                let color =
                    Color::parse_css(&source.color).unwrap_or_else(|| drawing.stroke_color());
                format!("{}{UPSTREAM_BAND_ALPHA:02x}", color.to_hex())
            })
        });
        DrawingLevel {
            fill_between: source.fill_between,
            fill_color,
            ..level
        }
    };
    let median = DrawingLevel {
        value: 0.5,
        color: String::new(),
        visible: true,
        style: match drawing.style {
            LineStyle::Solid => "solid",
            LineStyle::Dotted => "dotted",
            LineStyle::Dashed => "dashed",
        }
        .to_string(),
        fill_between: false,
        fill_color: None,
        label_visible: false,
    };
    let mut levels = Vec::with_capacity(2 * (visible.len() + hidden.len()) + 1);
    for (index, level) in visible.iter().enumerate().rev() {
        levels.push(filled_as(
            side(level, -1.0),
            visible.get(index + 1).copied(),
        ));
    }
    levels.push(filled_as(median, visible.first().copied()));
    levels.extend(visible.iter().map(|level| side(level, 1.0)));
    for level in &hidden {
        levels.push(side(level, -1.0));
    }
    for level in &hidden {
        levels.push(side(level, 1.0));
    }
    levels[2 * visible.len() + 1..].sort_by(|a, b| a.value.total_cmp(&b.value));
    drawing.levels = levels;
}

// --- re-applied on upstream's lowering --------------------------------------------------------

/// The drawing's Gann block, or the upstream-neutral defaults (borrowed: the arms read it every
/// frame and hit test).
fn options(drawing: &Drawing) -> &GannToolOptions {
    static DEFAULTS: LazyLock<GannToolOptions> = LazyLock::new(GannToolOptions::default);
    drawing.tool_options.gann.as_ref().unwrap_or(&DEFAULTS)
}

/// Whether `kind` resolves a pitchfork from three anchors (the four pitchforks and the pitchfan).
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

/// A Gann box's own vertical (time) levels, when `tool_options.gann.time_levels` splits the axes;
/// `None` for every other drawing and for a box whose vertical levels follow `levels`.
pub(crate) fn box_time_levels(drawing: &Drawing) -> Option<&[DrawingLevel]> {
    let levels = drawing.tool_options.gann.as_ref()?.time_levels.as_slice();
    (drawing.kind == DrawingKind::GannBox && !levels.is_empty()).then_some(levels)
}

/// The zone bands of a Gann grid between consecutive visible `levels` (the drawing's own, or a
/// box's time levels), as `(previous, value, level)` with effective values
/// (`Drawing::level_value`): upstream's grid rule, where a hidden level is skipped without
/// breaking the chain and a level fills toward its predecessor while the drawing's fill is on
/// and the level's `fill_between` is set.
fn grid_band_pairs<'a>(
    drawing: &'a Drawing,
    levels: &'a [DrawingLevel],
) -> impl Iterator<Item = (f64, f64, &'a DrawingLevel)> {
    let levels = if drawing.fill_enabled { levels } else { &[] };
    let mut previous = None;
    levels
        .iter()
        .filter(|level| level.visible)
        .filter_map(move |level| {
            let value = drawing.level_value(level.value);
            let prior = previous.replace(value)?;
            level.fill_between.then_some((prior, value, level))
        })
}

/// The zone fills of a resolved Gann `grid` in paint order, as `(x range, y range, level)` in the
/// grid's px: upstream's diagonal cells between consecutive levels or, for a box with its own
/// time levels (decision G1), its price bands across the box's width and then its time bands
/// down its height, each chained by [`grid_band_pairs`]. The frame paints them and the selected
/// hit tests them.
pub(crate) fn grid_bands(
    drawing: &Drawing,
    grid: GannGridGeometry,
) -> impl Iterator<Item = ((f64, f64), (f64, f64), &DrawingLevel)> {
    let x_at = move |value: f64| grid.start.0 + (grid.end.0 - grid.start.0) * value;
    let y_at = move |value: f64| grid.start.1 + (grid.end.1 - grid.start.1) * value;
    let bounds = grid.bounds();
    let time_levels = box_time_levels(drawing);
    let across = time_levels.map(|_| (bounds.left, bounds.right));
    let price = grid_band_pairs(drawing, &drawing.levels).map(move |(previous, value, level)| {
        let xs = across.unwrap_or((x_at(previous), x_at(value)));
        (xs, (y_at(previous), y_at(value)), level)
    });
    let down = (bounds.top, bounds.bottom);
    let time = grid_band_pairs(drawing, time_levels.unwrap_or_default())
        .map(move |(previous, value, level)| ((x_at(previous), x_at(value)), down, level));
    price.chain(time)
}

/// Whether a Gann square paints its stats box (`tool_options.gann.show_stats`).
pub(crate) fn shows_stats(drawing: &Drawing) -> bool {
    matches!(
        drawing.kind,
        DrawingKind::GannSquare | DrawingKind::GannSquareFixed
    ) && options(drawing).show_stats
}

/// The angle lines a Gann grid strokes from its pivot corner: a box's `tool_options.gann.angles`
/// while `show_angles` is on, a square's `gann_fans`, none otherwise.
pub(crate) fn angle_levels(drawing: &Drawing) -> &[DrawingLevel] {
    match drawing.kind {
        DrawingKind::GannBox => {
            let options = options(drawing);
            if options.show_angles {
                &options.angles
            } else {
                &[]
            }
        }
        DrawingKind::GannSquare | DrawingKind::GannSquareFixed => &drawing.gann_fans,
        _ => &[],
    }
}

/// The derived point a `scale_ratio` puts at the second anchor's bar on a Gann fan (its 1×1
/// target) or fixed square (its far corner): the first anchor's price plus or minus the bars
/// between the anchors times the ratio, toward the second anchor's price. `None` without a
/// ratio or bars between the anchors. The engine appends it to the drawing's render px, so the
/// frame, both hit paths, culling and placement previews resolve one geometry.
pub(crate) fn ratio_point(drawing: &Drawing) -> Option<DrawingPoint> {
    if !matches!(
        drawing.kind,
        DrawingKind::GannFan | DrawingKind::GannSquareFixed
    ) {
        return None;
    }
    let ratio = drawing.tool_options.gann.as_ref()?.scale_ratio?;
    let [a, b] = drawing.points.as_slice() else {
        return None;
    };
    let bars = (b.logical - a.logical).abs();
    if !bars.is_finite() || bars <= f64::EPSILON {
        return None;
    }
    let rise = bars * ratio;
    let price = if b.price < a.price {
        a.price - rise
    } else {
        a.price + rise
    };
    price.is_finite().then_some(DrawingPoint {
        logical: b.logical,
        price,
    })
}

/// The far price of a fixed square's second anchor `bars` from a first anchor at `price`,
/// toward `direction` (±1 in price): a span the anchor's magnitude sets, so the square's time
/// side normally stays its smaller one under ordinary price zoom (upstream squares the smaller
/// side every frame); a large price zoom-out can still let the price side win. Below a positive
/// anchor that span would cross zero, where a logarithmic or percentage scale places nothing, so
/// the corner divides the price instead, which guarantees only a positive price. The
/// fork-document conversion and the corner drag share this rule.
pub(crate) fn fixed_square_far_price(price: f64, bars: f64, direction: f64) -> f64 {
    let additive = price + direction * bars * price.abs().max(1.0);
    if price > 0.0 && additive <= 0.0 {
        price / (1.0 + bars)
    } else {
        additive
    }
}

/// A price-basis rescale scales the fan's and the fixed square's `scale_ratio` (price per bar)
/// by `factor`, the factor at their first anchor, so the angle and the square keep measuring the
/// same bars. `Some(changed)`; `None` when the scaled ratio leaves `(0, MAX_SAFE_VALUE]`, which
/// rejects the whole rescale. Applies only with `apply` (the engine validates first).
pub(crate) fn rescale_scale_ratio(
    kind: DrawingKind,
    options: &mut DrawingToolOptions,
    factor: f64,
    apply: bool,
) -> Option<bool> {
    if !matches!(kind, DrawingKind::GannFan | DrawingKind::GannSquareFixed) || factor == 1.0 {
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

/// Four significant digits for a price-per-bar ratio.
fn ratio_text(ratio: f64) -> String {
    if ratio == 0.0 || !ratio.is_finite() {
        return "0".to_string();
    }
    let digits = (3 - ratio.abs().log10().floor() as i32).clamp(0, 8) as usize;
    format!("{ratio:.digits$}")
}

/// The stats box of a Gann square or fixed square with `tool_options.gann.show_stats`, beside the
/// far corner of its resolved `grid` (caller px; left of it when the square grows left): the
/// price range, the bars, and the price per bar (omitted for a square without bars), in the
/// shared translucent stats box (a body target). Paint and hit resolve it from the same grid.
pub(crate) fn square_stats(
    ctx: &PartContext<'_>,
    grid: GannGridGeometry,
    parts: &mut DrawingParts,
) {
    let drawing = ctx.drawing;
    if !shows_stats(drawing) {
        return;
    }
    let ([a, b], [pa, pb, ..]) = (drawing.points.as_slice(), ctx.px) else {
        return;
    };
    let (pivot, far) = if drawing.level_reverse {
        (grid.end, grid.start)
    } else {
        (grid.start, grid.end)
    };
    let range = if drawing.kind == DrawingKind::GannSquare {
        (b.price - a.price).abs()
    } else if let Some(corner) = ratio_point(drawing) {
        (corner.price - a.price).abs()
    } else {
        let price = |(x, y): Point| {
            ctx.engine
                .drawing_from_px_for(
                    drawing.pane_index,
                    drawing.price_scale,
                    x / ctx.x_scale,
                    y / ctx.scale,
                )
                .map(|point| point.price)
        };
        let (Some(far), Some(pivot)) = (price(grid.end), price(grid.start)) else {
            return;
        };
        (far - pivot).abs()
    };
    // The painted side in bars: the anchors' bars scaled by the side's share of their width
    // (upstream's fixed square takes the smaller side), and none without width.
    let width = (pb.0 - pa.0).abs();
    let bars = if width <= f64::EPSILON {
        0.0
    } else {
        (b.logical - a.logical).abs() * (grid.end.0 - grid.start.0).abs() / width
    };
    if !range.is_finite() || !bars.is_finite() {
        return;
    }
    let mut lines = vec![
        ctx.engine.format_drawing_price(drawing, range),
        format!("{} bars", bars.round() as i64),
    ];
    if bars > f64::EPSILON {
        lines.push(format!("{}/bar", ratio_text(range / bars)));
    }
    let forward = far.0 >= pivot.0;
    let gap = STATS_GAP * ctx.scale;
    let (x, h_align) = if forward {
        (far.0 + gap, DrawingTextHAlign::Left)
    } else {
        (far.0 - gap, DrawingTextHAlign::Right)
    };
    parts.label(ctx.stats_box(
        (x, far.1),
        (h_align, DrawingTextVAlign::Middle),
        lines,
        None,
        text_on,
    ));
}

/// The culling pad (CSS px) of what the Gann arms paint past a grid's box (see
/// `kinds::upstream_decoration_extent`): a box's time-level labels above it (the 8 CSS px gap
/// and the label's height, half its width either side of its line), and the squares' stats box,
/// a template of its widest lines plus four ems of slack, since a screen square's price range
/// follows the viewport.
pub(crate) fn upstream_decoration_extent(engine: &ChartEngine, drawing: &Drawing) -> f64 {
    let layout = &engine.options.get().layout;
    let mut extent: f64 = 0.0;
    for level in box_time_levels(drawing)
        .into_iter()
        .flatten()
        .filter(|level| level.visible && level.label_visible)
    {
        if let Some(text) = engine.drawing_level_label(drawing, level.value, None) {
            let width = engine.measure_text_run(
                &text,
                layout.font_size,
                &layout.font_family,
                drawing.text_weight.unwrap_or(400),
                drawing.text_italic,
            );
            extent = extent.max(8.0 + layout.font_size).max(width / 2.0);
        }
    }
    if shows_stats(drawing) {
        let size = engine.drawing_stats_size();
        let width = ["000000.00", "00000 bars", "0000.000/bar"]
            .iter()
            .map(|line| engine.measure_text_run(line, size, &layout.font_family, 400, false))
            .fold(0.0_f64, f64::max);
        extent = extent
            .max(STATS_GAP + width + 2.0 * STATS_PADDING.0 + 4.0 * size)
            .max(STATS_GAP + 3.0 * size * 1.25 + 2.0 * STATS_PADDING.1);
    }
    extent
}

/// The `tool_options.gann` descriptors the Gann kinds read on upstream's arms (see
/// `kinds::extend_upstream_schema`): the box's time levels and angles, the squares' stats box,
/// and the fan's and fixed square's scale ratio. The fixed square's `size_bars` has no row: its
/// anchors carry the size (decision G3).
pub(crate) fn extend_upstream_schema(
    kind: DrawingKind,
    properties: &mut Vec<DrawingPropertyDescriptor>,
) {
    let defaults = GannToolOptions::default();
    let descriptor = |name: &str, property_type, default| {
        crate::drawing_contract::descriptor(
            format!("tool_options.gann.{name}"),
            property_type,
            default,
        )
    };
    let levels = |levels: &[DrawingLevel]| serde_json::to_value(levels).unwrap_or_default();
    let scale_ratio = || {
        descriptor(
            "scale_ratio",
            DrawingPropertyType::Number,
            serde_json::Value::Null,
        )
    };
    let show_stats = || {
        descriptor(
            "show_stats",
            DrawingPropertyType::Boolean,
            serde_json::json!(defaults.show_stats),
        )
    };
    match kind {
        DrawingKind::GannBox => {
            properties.push(descriptor(
                "time_levels",
                DrawingPropertyType::Levels,
                levels(&defaults.time_levels),
            ));
            properties.push(descriptor(
                "angles",
                DrawingPropertyType::Levels,
                levels(&defaults.angles),
            ));
            properties.push(descriptor(
                "show_angles",
                DrawingPropertyType::Boolean,
                serde_json::json!(defaults.show_angles),
            ));
        }
        DrawingKind::GannSquare => properties.push(show_stats()),
        DrawingKind::GannSquareFixed => {
            properties.push(show_stats());
            properties.push(scale_ratio());
        }
        DrawingKind::GannFan => properties.push(scale_ratio()),
        _ => {}
    }
}

/// The derived handles of the pitchforks and the fixed Gann square (see
/// `kinds::upstream_derived_handles`; `px` are the anchors' media px): a pitchfork's (and the
/// pitchfan's) base midpoint between its handle anchors B and C, appended as `Handle(0)` (so the
/// fourth keyboard handle), which moves both; and the fixed square's painted far corner, which
/// takes the second anchor's place as `Handle(0)` (that anchor often lies off the square, far
/// past it for a fork document) and resizes the square.
pub(crate) fn derived_handles(
    engine: &ChartEngine,
    drawing: &Drawing,
    px: &[Point],
    handles: &mut Vec<DrawingHandle>,
) {
    if is_fork(drawing.kind) {
        let (Some(&b), Some(&c)) = (px.get(1), px.get(2)) else {
            return;
        };
        handles.push(DrawingHandle {
            point: shape::midpoint(b, c),
            part: DrawingDragPart::Handle(0),
            cursor: "pointer",
            shape: HandleShape::Disc,
        });
    } else if drawing.kind == DrawingKind::GannSquareFixed {
        let (Some(&pivot), Some(&second)) = (px.first(), px.get(1)) else {
            return;
        };
        let ratio_corner =
            ratio_point(drawing).and_then(|point| engine.drawing_point_px(drawing, point));
        let corner = gann_fixed_end(pivot, second, ratio_corner);
        // Up-right or down-left of the pivot resizes along the rising diagonal.
        let cursor = if (corner.0 >= pivot.0) == (corner.1 <= pivot.1) {
            "nesw-resize"
        } else {
            "nwse-resize"
        };
        let corner_handle = DrawingHandle {
            point: corner,
            part: DrawingDragPart::Handle(0),
            cursor,
            shape: HandleShape::Disc,
        };
        match handles
            .iter_mut()
            .find(|handle| handle.part == DrawingDragPart::Anchor(1))
        {
            Some(handle) => *handle = corner_handle,
            None => handles.push(corner_handle),
        }
    }
}

/// One derived-handle drag sample (see `kinds::drag_derived_handle`): edits `points` (the
/// baseline anchors on entry) and returns the tool options the sample sets, if any, or `None` to
/// reject the whole sample (an anchor it cannot place, such as a time-snapped anchor past the
/// data), so the drag keeps its last valid sample.
///
/// The pitchfork base midpoint shifts both handle anchors by whole bars toward the slot its handle
/// lands on (a magnet's bar, or a keyboard step's bars) and their prices by its vertical move.
/// The fixed square's corner sets the side in whole bars, toward the corner's side of the first
/// anchor; a pointer rounds to the nearest bar and a keyboard step
/// moves at least one whole bar the way the key moved, so sub-bar steps still resize it. With a
/// `scale_ratio` the side comes from the corner's bar, its price sets the ratio (Shift keeps the
/// press ratio), and the second anchor lands on the ratio corner. Without one the square stays
/// square on screen: a pointer sizes it by the corner's larger distance from the first anchor, a
/// keyboard step by the axis it moved, and the second anchor takes the new bar and keeps its
/// price while that lies beyond the new corner on the drag's vertical side; otherwise its price
/// moves far beyond the corner ([`fixed_square_far_price`]), or onto the corner when the scale
/// cannot place that beyond it. The time side then normally stays the smaller one, so the square
/// keeps its whole bars under ordinary price zoom.
pub(crate) fn drag_handle(
    engine: &ChartEngine,
    drawing: &Drawing,
    sample: &HandleDrag<'_>,
    points: &mut [DrawingPoint],
) -> Option<Option<DrawingToolOptions>> {
    if sample.part != DrawingDragPart::Handle(0) {
        return Some(None);
    }
    if is_fork(drawing.kind) {
        // One whole-bar shift for both: an odd base's midpoint sits half a bar off the grid,
        // where snapping B and C on their own would round two .5 ties apart. A pointer's target
        // is the slot under the handle, whose offset truncates, so the base moves once the
        // pointer crosses a bar; a keyboard target moved whole bars from the midpoint, so
        // rounding only drops the px round trip's noise. The prices follow the px move, and a
        // time-snapped pitchfork keeps both on the data.
        let (&b, &c) = (sample.start_px.get(1)?, sample.start_px.get(2)?);
        let dy = sample.target_px.1 - shape::midpoint(b, c).1;
        let (start_b, start_c) = (sample.start_points.get(1)?, sample.start_points.get(2)?);
        let offset = sample.target.logical - (start_b.logical + start_c.logical) / 2.0;
        let shift = match sample.keyboard_step {
            Some(_) => offset.round(),
            None => offset.trunc(),
        };
        let moved = |start: &DrawingPoint, (x, y): Point| {
            let price = engine
                .drawing_from_px_for(drawing.pane_index, drawing.price_scale, x, y + dy)?
                .price;
            let point = DrawingPoint {
                logical: start.logical + shift,
                price,
            };
            if drawing.snap_time_to_data {
                engine.snap_drawing_time_to_data(point)
            } else {
                Some(point)
            }
        };
        // Both or neither: a shift that carries C past the data moves no anchor.
        let (moved_b, moved_c) = (moved(start_b, b)?, moved(start_c, c)?);
        (points[1], points[2]) = (moved_b, moved_c);
        return Some(None);
    }
    if drawing.kind != DrawingKind::GannSquareFixed || points.len() != 2 {
        return Some(None);
    }
    let (&a, &pivot) = (sample.start_points.first()?, sample.start_px.first()?);
    let press = *sample.start_points.get(1)?;
    let press_y = sample.start_px.get(1)?.1;
    let logical_at = |x: f64| {
        engine
            .drawing_from_px_for(drawing.pane_index, drawing.price_scale, x, pivot.1)
            .map(|point| point.logical)
    };
    // Float noise from the px round trip never counts as a keyboard step.
    const STEP_EPSILON: f64 = 1e-6;
    // The painted side at the press, in signed bars (negative grows left): the second anchor's
    // own bars when the corner sits on its bar (always with a ratio), so a step that changes no
    // bar keeps its logical bit-identical.
    let anchor_bars = press.logical - a.logical;
    let painted_bars = logical_at(sample.handle_px.0)? - a.logical;
    let press_bars = if (painted_bars - anchor_bars).abs() <= STEP_EPSILON {
        anchor_bars
    } else {
        painted_bars
    };
    let mut gann = sample.start_tool_options.gann.clone().unwrap_or_default();
    let (dx, dy) = (sample.target_px.0 - pivot.0, sample.target_px.1 - pivot.1);
    let raw = match gann.scale_ratio {
        Some(_) => sample.target.logical - a.logical,
        None => {
            let side = match sample.keyboard_step {
                Some((step_x, step_y)) if step_y.abs() > step_x.abs() => dy.abs(),
                Some(_) => dx.abs(),
                None => dx.abs().max(dy.abs()),
            };
            let toward = if dx != 0.0 {
                dx.signum()
            } else {
                press_bars.signum()
            };
            logical_at(pivot.0 + toward * side)? - a.logical
        }
    };
    if !raw.is_finite() {
        return None;
    }
    let bars = match sample.keyboard_step {
        None => raw.round(),
        Some(_) if raw > press_bars + STEP_EPSILON => (raw - STEP_EPSILON).ceil(),
        Some(_) if raw < press_bars - STEP_EPSILON => (raw + STEP_EPSILON).floor(),
        Some(_) => press_bars,
    };
    let sign = if bars != 0.0 {
        bars.signum()
    } else if press_bars != 0.0 {
        press_bars.signum()
    } else {
        1.0
    };
    let bars = sign * bars.abs().clamp(1.0, MAX_GANN_SQUARE_BARS);
    let logical = if bars == anchor_bars {
        press.logical
    } else {
        a.logical + bars
    };
    // A time-snapped square keeps its second anchor on the data, as the anchor drag does: the
    // larger-distance (or clamped) side can reach past the last bar the pointer is over.
    if drawing.snap_time_to_data && logical != press.logical {
        engine.snap_drawing_time_to_data(DrawingPoint {
            logical,
            price: a.price,
        })?;
    }
    if let Some(press_ratio) = gann.scale_ratio {
        let mut ratio = press_ratio;
        if !sample.straighten {
            let dragged = (sample.target.price - a.price).abs() / bars.abs();
            if dragged.is_finite() && dragged > 0.0 {
                ratio = dragged;
            }
        }
        let direction = if sample.target.price != a.price {
            (sample.target.price - a.price).signum()
        } else if press.price < a.price {
            -1.0
        } else {
            1.0
        };
        let price = a.price + direction * bars.abs() * ratio;
        if !price.is_finite() || price.abs() > MAX_SAFE_VALUE || ratio > MAX_SAFE_VALUE {
            return None;
        }
        points[1] = DrawingPoint { logical, price };
        gann.scale_ratio = Some(ratio);
        let mut tool_options = drawing.tool_options.clone();
        tool_options.gann = Some(gann);
        return Some(Some(tool_options));
    }
    // Screen square: the new side in px, on the drag's vertical side of the pivot. The second
    // anchor keeps its price while that lies beyond the new corner; otherwise it moves to the
    // far-price rule's price, or to the corner itself when the scale cannot place that beyond
    // the corner.
    let side = (engine.time_scale.logical_to_coordinate(logical) - pivot.0).abs();
    let down = if dy != 0.0 {
        dy > 0.0
    } else {
        press_y > pivot.1
    };
    let toward = if down { 1.0 } else { -1.0 };
    let beyond = |y: f64| (y - pivot.1) * toward >= side;
    let corner = engine
        .drawing_from_px_for(
            drawing.pane_index,
            drawing.price_scale,
            pivot.0,
            pivot.1 + toward * side,
        )?
        .price;
    let direction = (corner - a.price).signum();
    let far = fixed_square_far_price(a.price, bars.abs(), direction);
    let price = if beyond(press_y) {
        press.price
    } else if far.is_finite()
        && far.abs() <= MAX_SAFE_VALUE
        && engine
            .drawing_point_px(
                drawing,
                DrawingPoint {
                    logical,
                    price: far,
                },
            )
            .is_some_and(|(_, y)| beyond(y))
    {
        far
    } else {
        corner
    };
    points[1] = DrawingPoint { logical, price };
    Some(None)
}

#[cfg(test)]
mod tests;
