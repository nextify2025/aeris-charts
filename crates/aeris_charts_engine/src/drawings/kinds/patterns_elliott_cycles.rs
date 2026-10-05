//! B8 Patterns, Elliott waves, and cycles options and legacy defaults. Upstream renders the
//! harmonic patterns, head and shoulders, triangle pattern, three drives, the Elliott waves, and
//! the cycle tools from its catalog spec; this module keeps the fork's public option block
//! ([`PatternToolOptions`], [`ElliottWaveDegree`]) and, for documents the fork wrote, its pre-merge
//! kind defaults ([`legacy_defaults`]).

use super::super::Drawing;
use crate::DrawingKind;

/// The fork's Elliott wave degree, largest first; the default is TradingView's `Intermediate`.
/// Its snake_case names are the upstream `wave_degree` values, which upstream labels as
/// `<label> (<degree>)`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ElliottWaveDegree {
    Supermillennium,
    Millennium,
    Submillennium,
    GrandSupercycle,
    Supercycle,
    Cycle,
    Primary,
    #[default]
    Intermediate,
    Minor,
    Minute,
    Minuette,
    Subminuette,
}

impl ElliottWaveDegree {
    /// Every degree, largest first (the schema's enum order).
    pub const ALL: [Self; 12] = [
        Self::Supermillennium,
        Self::Millennium,
        Self::Submillennium,
        Self::GrandSupercycle,
        Self::Supercycle,
        Self::Cycle,
        Self::Primary,
        Self::Intermediate,
        Self::Minor,
        Self::Minute,
        Self::Minuette,
        Self::Subminuette,
    ];
}

/// The fork's pattern options (`tool_options.pattern`); absent fields keep their defaults. Upstream
/// renders the patterns and Elliott waves: `degree` is an input alias of `wave_degree` (see
/// `drawing_contract::take_legacy_flat_options`); `show_ratios` and `show_wave` are stored but not
/// rendered.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct PatternToolOptions {
    /// Harmonic patterns (XABCD, cypher, ABCD, three drives): the dashed ratio connectors and
    /// their ratio labels. Default `true`.
    pub show_ratios: bool,
    /// Elliott waves: the degree whose notation labels the waves. Default `intermediate`.
    pub degree: ElliottWaveDegree,
    /// Elliott waves: the wave polyline (`false` leaves only the labels). Default `true`.
    pub show_wave: bool,
}

impl Default for PatternToolOptions {
    fn default() -> Self {
        Self {
            show_ratios: true,
            degree: ElliottWaveDegree::default(),
            show_wave: true,
        }
    }
}

/// The fork's pre-merge defaults of the pattern, Elliott, and cycle tools (see
/// [`super::apply_legacy_fork_defaults`]): TradingView's color per tool, the region tools with
/// their fill on, and the triangle pattern's apex sides.
pub(super) fn legacy_defaults(drawing: &mut Drawing) {
    let (color, fill) = match drawing.kind {
        DrawingKind::PatternXabcd | DrawingKind::PatternCypher => ("#2962FF", true),
        DrawingKind::PatternAbcd => ("#089981", false),
        DrawingKind::PatternHeadShoulders => ("#089981", true),
        DrawingKind::PatternTriangle => ("#673AB7", true),
        DrawingKind::PatternThreeDrives => ("#673AB7", false),
        DrawingKind::ElliottImpulse | DrawingKind::ElliottCorrection => ("#3D85C6", false),
        DrawingKind::ElliottTriangle => ("#FF9800", false),
        DrawingKind::ElliottDoubleCombination | DrawingKind::ElliottTripleCombination => {
            ("#6AA84F", false)
        }
        DrawingKind::CyclicLines => ("#80CCDB", false),
        DrawingKind::TimeCycles => ("#159980", true),
        DrawingKind::SineLine => ("#159980", false),
        _ => return,
    };
    drawing.color = color.to_string();
    drawing.fill_enabled = fill;
    // The fork always drew a triangle pattern's sides on to their apex; on upstream's lowering
    // `extend_left`/`extend_right` select them (on the side the apex lies on).
    if drawing.kind == DrawingKind::PatternTriangle {
        drawing.extend_left = true;
        drawing.extend_right = true;
    }
}

#[cfg(test)]
mod tests;
