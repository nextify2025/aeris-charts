//! B8 Pitchforks & Gann options and legacy defaults. Upstream renders the Andrews, Schiff,
//! modified Schiff, and inside pitchforks, the pitchfan, the Gann box, square, fixed square, and
//! fan from its catalog spec and the flat level fields; this module keeps the fork's public option
//! block ([`GannToolOptions`], [`MAX_GANN_SQUARE_BARS`]) and, for documents the fork wrote, its
//! pre-merge kind defaults ([`legacy_defaults`]).

use aeris_charts_render::color::Color;
use aeris_charts_render::draw_list::LineStyle;

use super::super::Drawing;
use crate::{DrawingKind, DrawingLevel, MAX_DRAWING_LEVELS};

/// Upper bound on the fixed Gann square's side in bars.
pub const MAX_GANN_SQUARE_BARS: f64 = 100_000.0;
/// The fixed Gann square's default side in bars.
const DEFAULT_SQUARE_BARS: f64 = 20.0;

/// The fork's Gann-tool options (`tool_options.gann`); absent fields keep their defaults. Upstream
/// renders the Gann tools from the flat fields: `reverse` is an input alias of `level_reverse`, and
/// on the squares `angles` and `arcs` are input aliases of `gann_fans` and `gann_arcs` (see
/// `drawing_contract::take_legacy_flat_options`); the other fields are stored but not rendered.
/// The defaults of the fields that switch a look on (`time_levels`, `show_stats`) are upstream's
/// look, so a block a patch creates for one key switches nothing else on; documents the fork
/// wrote get the fork's values through `kinds::legacy_fork_tool_options`.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct GannToolOptions {
    /// Gann box: vertical levels as fractions of the box width from the pivot corner (the
    /// drawing's `levels` are its horizontal price levels). Empty (the default): the vertical
    /// levels follow `levels`, as upstream draws them.
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
    /// Gann squares: the measurement box (price range, bars, and price per bar). Default false.
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

#[cfg(test)]
mod tests;
