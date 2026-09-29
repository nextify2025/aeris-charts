//! B8 Patterns, Elliott waves, and cycles family (wire ids 160..=191).
//!
//! Patterns: XABCD, cypher, ABCD, head and shoulders, triangle pattern, and three drives. Each is
//! a zigzag through its anchors with boxed point labels above peaks and below troughs; the
//! harmonic tools add dashed connectors carrying Fibonacci ratios measured in price, XABCD and
//! cypher shade their two triangles, head and shoulders draws the neckline and shades the
//! shoulders and head against it, and the triangle pattern extends its A–C and B–D sides to their
//! apex and shades the triangle between them.
//!
//! Elliott waves: impulse (0-1-2-3-4-5), correction (0-A-B-C), triangle (0-A-B-C-D-E), double
//! combination (0-W-X-Y), and triple combination (0-W-X-Y-X-Z), labeled with the notation of
//! their degree (`tool_options.pattern.degree`, supermillennium through subminuette).
//!
//! Cycles: cyclic lines (vertical lines every anchor interval from the earlier anchor to the
//! right), time cycles (half-ellipse arches of the anchors' width and height, repeated in both
//! directions), and the sine line (a sine through both anchors, peak to trough in one
//! half period, across the pane). Repeats are resolved only over the visible pane, and repeats
//! closer than [`MIN_REPEAT_SPACING`] collapse to the defining cycle, so frame work is bounded by
//! the pane rather than by history.
//!
//! Multi-anchor tools preview progressively while being placed
//! ([`DrawingFamily::partial_preview`]): every part below resolves from however many anchors
//! exist.

use std::f64::consts::PI;

use aeris_charts_render::color::Color;
use aeris_charts_render::draw_list::LineStyle;
use aeris_charts_render::shape::{self, Point};

use super::super::parts::{
    text_on, DrawingParts, PartContext, PartLabel, PartStroke, CURVE_TOLERANCE,
};
use super::super::tools::{
    DrawingHandleMode, DrawingLogicalExtent, DrawingMovementAxis, DrawingPlacement,
    DrawingPriceExtent, DrawingStraightenMode, DrawingTextLayout, DrawingToolSpec,
};
use super::super::{Drawing, DrawingTextHAlign, DrawingTextVAlign};
use super::DrawingFamily;
use crate::drawing_contract::descriptor;
use crate::{
    ChartEngine, DrawingKind, DrawingKindOptions, DrawingPropertyDescriptor, DrawingPropertyType,
};

/// Elliott wave degree, largest first. Each degree has its own label notation (see
/// [`ElliottWaveDegree::label`]); the default is TradingView's `Intermediate`.
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

/// How a degree writes wave numbers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Numerals {
    UpperRoman,
    Arabic,
    LowerRoman,
}

/// What a degree wraps its labels in. A ring is engine geometry around the label rather than a
/// circled glyph, so it renders identically on every executor and font.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Enclosure {
    None,
    Parentheses,
    Ring,
    Braces,
    Brackets,
    Angles,
}

/// One Elliott label: a wave number (1–5) or a wave letter (A–E, W, X, Y, Z).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WaveMark {
    Number(u8),
    Letter(char),
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

    fn name(self) -> &'static str {
        match self {
            Self::Supermillennium => "supermillennium",
            Self::Millennium => "millennium",
            Self::Submillennium => "submillennium",
            Self::GrandSupercycle => "grand_supercycle",
            Self::Supercycle => "supercycle",
            Self::Cycle => "cycle",
            Self::Primary => "primary",
            Self::Intermediate => "intermediate",
            Self::Minor => "minor",
            Self::Minute => "minute",
            Self::Minuette => "minuette",
            Self::Subminuette => "subminuette",
        }
    }

    /// The Frost–Prechter notation: each triad of degrees shares a numeral system (upper Roman,
    /// Arabic, lower Roman) and cycles ring, parentheses, and bare labels; letters are uppercase
    /// on the Arabic triad and lowercase elsewhere. The three millennium degrees extend the upper
    /// Roman triad with braces, brackets, and angle brackets.
    fn notation(self) -> (Numerals, bool, Enclosure) {
        match self {
            Self::Supermillennium => (Numerals::UpperRoman, false, Enclosure::Braces),
            Self::Millennium => (Numerals::UpperRoman, false, Enclosure::Brackets),
            Self::Submillennium => (Numerals::UpperRoman, false, Enclosure::Angles),
            Self::GrandSupercycle => (Numerals::UpperRoman, false, Enclosure::Ring),
            Self::Supercycle => (Numerals::UpperRoman, false, Enclosure::Parentheses),
            Self::Cycle => (Numerals::UpperRoman, false, Enclosure::None),
            Self::Primary => (Numerals::Arabic, true, Enclosure::Ring),
            Self::Intermediate => (Numerals::Arabic, true, Enclosure::Parentheses),
            Self::Minor => (Numerals::Arabic, true, Enclosure::None),
            Self::Minute => (Numerals::LowerRoman, false, Enclosure::Ring),
            Self::Minuette => (Numerals::LowerRoman, false, Enclosure::Parentheses),
            Self::Subminuette => (Numerals::LowerRoman, false, Enclosure::None),
        }
    }

    /// The label text of `mark` in this degree and whether it is ringed.
    fn label(self, mark: WaveMark) -> (String, bool) {
        const UPPER_ROMAN: [&str; 5] = ["I", "II", "III", "IV", "V"];
        const LOWER_ROMAN: [&str; 5] = ["i", "ii", "iii", "iv", "v"];
        let (numerals, upper_letters, enclosure) = self.notation();
        let core = match mark {
            WaveMark::Number(number) => {
                let index = usize::from(number.clamp(1, 5) - 1);
                match numerals {
                    Numerals::UpperRoman => UPPER_ROMAN[index].to_string(),
                    Numerals::Arabic => number.to_string(),
                    Numerals::LowerRoman => LOWER_ROMAN[index].to_string(),
                }
            }
            WaveMark::Letter(letter) if upper_letters => letter.to_ascii_uppercase().to_string(),
            WaveMark::Letter(letter) => letter.to_ascii_lowercase().to_string(),
        };
        match enclosure {
            Enclosure::None | Enclosure::Ring => (core, enclosure == Enclosure::Ring),
            Enclosure::Parentheses => (format!("({core})"), false),
            Enclosure::Braces => (format!("{{{core}}}"), false),
            Enclosure::Brackets => (format!("[{core}]"), false),
            Enclosure::Angles => (format!("<{core}>"), false),
        }
    }
}

/// Family options (`tool_options.pattern`); absent fields keep their defaults.
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

/// Region fill alpha over the drawing color when `fill_color` is unset: TradingView's pattern
/// transparency of 85 %.
const FILL_ALPHA: u8 = 38;
/// Gap between an anchor and its point label, in CSS px.
const LABEL_GAP: f64 = 6.0;
/// Point and ratio label box padding (horizontal, vertical) in CSS px.
const LABEL_PADDING: (f64, f64) = (4.0, 2.0);
/// Space between an Elliott label's glyphs and its degree ring, in CSS px.
const RING_PADDING: f64 = 2.0;
/// Width of dashed ratio connectors, cycle connectors, and degree rings in CSS px.
const DECORATION_WIDTH: f64 = 1.0;
/// Repeats of a cycle tool closer than this (CSS px) collapse to the defining cycle: they would
/// paint a solid block and cost work proportional to the zoom-out rather than to the pane.
pub(crate) const MIN_REPEAT_SPACING: f64 = 3.0;
/// Point budget of one drawing's time-cycle arches or sine wave; beyond it the chord tolerance
/// grows so the tessellation stays bounded however many repeats are visible.
const MAX_CURVE_POINTS: usize = 16_384;

/// Pattern behavior shared by every tool below; each spec overrides its identity.
const PATTERN_TOOL: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::XabcdPattern,
    wire_id: 160,
    name: "xabcd_pattern",
    placement: DrawingPlacement::ClickAnchors { count: 5 },
    handles: DrawingHandleMode::Anchors,
    movement_axis: DrawingMovementAxis::Both,
    straighten: DrawingStraightenMode::Segment45,
    logical_extent: DrawingLogicalExtent::Finite,
    price_extent: DrawingPriceExtent::Finite,
    bounds_padding_ratio: 0.0,
    default_width: 2.0,
    requests_text_editor: false,
    family: Some(&FAMILY),
    text_layout: DrawingTextLayout::Box,
    axis_price_label: false,
};

pub(crate) const XABCD_PATTERN: DrawingToolSpec = PATTERN_TOOL;

pub(crate) const CYPHER_PATTERN: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::CypherPattern,
    wire_id: 161,
    name: "cypher_pattern",
    ..PATTERN_TOOL
};

pub(crate) const ABCD_PATTERN: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::AbcdPattern,
    wire_id: 162,
    name: "abcd_pattern",
    placement: DrawingPlacement::ClickAnchors { count: 4 },
    ..PATTERN_TOOL
};

pub(crate) const HEAD_AND_SHOULDERS: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::HeadAndShoulders,
    wire_id: 163,
    name: "head_and_shoulders",
    placement: DrawingPlacement::ClickAnchors { count: 7 },
    ..PATTERN_TOOL
};

// The sides extend to their apex up to one pattern width beyond the anchors, which the padded
// logical bounds cover; the apex height is unbounded by the anchors, so price never culls.
pub(crate) const TRIANGLE_PATTERN: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::TrianglePattern,
    wire_id: 164,
    name: "triangle_pattern",
    placement: DrawingPlacement::ClickAnchors { count: 4 },
    price_extent: DrawingPriceExtent::Full,
    bounds_padding_ratio: 1.0,
    ..PATTERN_TOOL
};

pub(crate) const THREE_DRIVES_PATTERN: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::ThreeDrivesPattern,
    wire_id: 165,
    name: "three_drives_pattern",
    placement: DrawingPlacement::ClickAnchors { count: 7 },
    ..PATTERN_TOOL
};

pub(crate) const ELLIOTT_IMPULSE_WAVE: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::ElliottImpulseWave,
    wire_id: 166,
    name: "elliott_impulse_wave",
    placement: DrawingPlacement::ClickAnchors { count: 6 },
    ..PATTERN_TOOL
};

pub(crate) const ELLIOTT_CORRECTION_WAVE: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::ElliottCorrectionWave,
    wire_id: 167,
    name: "elliott_correction_wave",
    placement: DrawingPlacement::ClickAnchors { count: 4 },
    ..PATTERN_TOOL
};

pub(crate) const ELLIOTT_TRIANGLE_WAVE: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::ElliottTriangleWave,
    wire_id: 168,
    name: "elliott_triangle_wave",
    placement: DrawingPlacement::ClickAnchors { count: 6 },
    ..PATTERN_TOOL
};

pub(crate) const ELLIOTT_DOUBLE_COMBO: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::ElliottDoubleCombo,
    wire_id: 169,
    name: "elliott_double_combo",
    placement: DrawingPlacement::ClickAnchors { count: 4 },
    ..PATTERN_TOOL
};

pub(crate) const ELLIOTT_TRIPLE_COMBO: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::ElliottTripleCombo,
    wire_id: 170,
    name: "elliott_triple_combo",
    placement: DrawingPlacement::ClickAnchors { count: 6 },
    ..PATTERN_TOOL
};

// Vertical lines from the earlier anchor to the right edge, spanning the pane's height.
pub(crate) const CYCLIC_LINES: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::CyclicLines,
    wire_id: 171,
    name: "cyclic_lines",
    placement: DrawingPlacement::ClickAnchors { count: 2 },
    logical_extent: DrawingLogicalExtent::FromFirst,
    price_extent: DrawingPriceExtent::Full,
    default_width: 1.0,
    ..PATTERN_TOOL
};

// Arches repeat in both directions between the anchors' two prices.
pub(crate) const TIME_CYCLES: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::TimeCycles,
    wire_id: 172,
    name: "time_cycles",
    placement: DrawingPlacement::ClickAnchors { count: 2 },
    straighten: DrawingStraightenMode::None,
    logical_extent: DrawingLogicalExtent::Full,
    ..PATTERN_TOOL
};

// The wave spans the pane between the anchors' two prices.
pub(crate) const SINE_LINE: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::SineLine,
    wire_id: 173,
    name: "sine_line",
    placement: DrawingPlacement::ClickAnchors { count: 2 },
    straighten: DrawingStraightenMode::None,
    logical_extent: DrawingLogicalExtent::Full,
    ..PATTERN_TOOL
};

pub(crate) static FAMILY: DrawingFamily = {
    let mut family = DrawingFamily::new(build_parts, kind_options);
    family.apply_defaults = apply_defaults;
    family.decoration_extent = decoration_extent;
    family.extend_schema = extend_schema;
    family.partial_preview = true;
    family
};

/// TradingView's default colors per tool; the region tools also start with their fill on.
fn apply_defaults(drawing: &mut Drawing) {
    let (color, fill) = match drawing.kind {
        DrawingKind::XabcdPattern | DrawingKind::CypherPattern => ("#2962FF", true),
        DrawingKind::AbcdPattern => ("#089981", false),
        DrawingKind::HeadAndShoulders => ("#089981", true),
        DrawingKind::TrianglePattern => ("#673AB7", true),
        DrawingKind::ThreeDrivesPattern => ("#673AB7", false),
        DrawingKind::ElliottImpulseWave | DrawingKind::ElliottCorrectionWave => ("#3D85C6", false),
        DrawingKind::ElliottTriangleWave => ("#FF9800", false),
        DrawingKind::ElliottDoubleCombo | DrawingKind::ElliottTripleCombo => ("#6AA84F", false),
        DrawingKind::CyclicLines => ("#80CCDB", false),
        DrawingKind::TimeCycles => ("#159980", true),
        DrawingKind::SineLine => ("#159980", false),
        _ => return,
    };
    drawing.color = color.to_string();
    drawing.fill_enabled = fill;
}

fn options(drawing: &Drawing) -> PatternToolOptions {
    drawing.tool_options.pattern.unwrap_or_default()
}

fn is_elliott(kind: DrawingKind) -> bool {
    matches!(
        kind,
        DrawingKind::ElliottImpulseWave
            | DrawingKind::ElliottCorrectionWave
            | DrawingKind::ElliottTriangleWave
            | DrawingKind::ElliottDoubleCombo
            | DrawingKind::ElliottTripleCombo
    )
}

/// One dashed ratio connector: drawn from anchor `from` to anchor `to` and labeled with
/// `|price[numerator.1] - price[numerator.0]| / |price[denominator.1] - price[denominator.0]|`.
#[derive(Clone, Copy, Debug)]
struct Ratio {
    from: usize,
    to: usize,
    numerator: (usize, usize),
    denominator: (usize, usize),
}

/// The ratio of leg `index → index + 1` to the leg before it, drawn across both legs.
const fn leg_ratio(index: usize) -> Ratio {
    Ratio {
        from: index - 1,
        to: index + 1,
        numerator: (index, index + 1),
        denominator: (index - 1, index),
    }
}

/// XABCD (X, A, B, C, D = 0..4): AB/XA on XB, BC/AB on AC, CD/BC on BD, and AD/XA on XD.
const XABCD_RATIOS: [Ratio; 4] = [
    leg_ratio(1),
    leg_ratio(2),
    leg_ratio(3),
    Ratio {
        from: 0,
        to: 4,
        numerator: (1, 4),
        denominator: (0, 1),
    },
];
/// Cypher: AB/XA on XB, XC/XA (C's extension of XA) on AC, and CD/XC (D's retracement of XC)
/// on XD.
const CYPHER_RATIOS: [Ratio; 3] = [
    leg_ratio(1),
    Ratio {
        from: 1,
        to: 3,
        numerator: (0, 3),
        denominator: (0, 1),
    },
    Ratio {
        from: 0,
        to: 4,
        numerator: (3, 4),
        denominator: (0, 3),
    },
];
/// ABCD: BC/AB on AC and CD/BC on BD.
const ABCD_RATIOS: [Ratio; 2] = [leg_ratio(1), leg_ratio(2)];
/// Three drives: each retracement against its drive and each drive against its retracement.
const THREE_DRIVES_RATIOS: [Ratio; 4] = [leg_ratio(1), leg_ratio(2), leg_ratio(3), leg_ratio(4)];

fn ratios(kind: DrawingKind) -> &'static [Ratio] {
    match kind {
        DrawingKind::XabcdPattern => &XABCD_RATIOS,
        DrawingKind::CypherPattern => &CYPHER_RATIOS,
        DrawingKind::AbcdPattern => &ABCD_RATIOS,
        DrawingKind::ThreeDrivesPattern => &THREE_DRIVES_RATIOS,
        _ => &[],
    }
}

/// The ratio's value from the drawing's prices; `None` while an anchor is missing or the
/// reference leg is flat.
fn ratio_value(drawing: &Drawing, ratio: Ratio) -> Option<f64> {
    let price = |index: usize| drawing.points.get(index).map(|point| point.price);
    let numerator = (price(ratio.numerator.1)? - price(ratio.numerator.0)?).abs();
    let denominator = (price(ratio.denominator.1)? - price(ratio.denominator.0)?).abs();
    let value = numerator / denominator;
    (denominator > f64::EPSILON && value.is_finite()).then_some(value)
}

/// The label text of every ratio the drawing's prices define, in connector order.
fn ratio_texts(drawing: &Drawing) -> impl Iterator<Item = (Ratio, String)> + '_ {
    ratios(drawing.kind).iter().filter_map(|&ratio| {
        ratio_value(drawing, ratio).map(|value| (ratio, format!("{value:.3}")))
    })
}

/// Boxed point labels of the pattern tools, one slot per anchor.
fn pattern_labels(kind: DrawingKind) -> &'static [Option<&'static str>] {
    match kind {
        DrawingKind::XabcdPattern | DrawingKind::CypherPattern => {
            &[Some("X"), Some("A"), Some("B"), Some("C"), Some("D")]
        }
        DrawingKind::AbcdPattern | DrawingKind::TrianglePattern => {
            &[Some("A"), Some("B"), Some("C"), Some("D")]
        }
        DrawingKind::HeadAndShoulders => &[
            None,
            Some("Left Shoulder"),
            None,
            Some("Head"),
            None,
            Some("Right Shoulder"),
            None,
        ],
        DrawingKind::ThreeDrivesPattern => {
            &[None, Some("1"), None, Some("2"), None, Some("3"), None]
        }
        _ => &[],
    }
}

/// Elliott marks, one slot per anchor; the first anchor is the unlabeled wave start.
fn wave_marks(kind: DrawingKind) -> &'static [Option<WaveMark>] {
    use WaveMark::{Letter, Number};
    match kind {
        DrawingKind::ElliottImpulseWave => &[
            None,
            Some(Number(1)),
            Some(Number(2)),
            Some(Number(3)),
            Some(Number(4)),
            Some(Number(5)),
        ],
        DrawingKind::ElliottCorrectionWave => &[
            None,
            Some(Letter('A')),
            Some(Letter('B')),
            Some(Letter('C')),
        ],
        DrawingKind::ElliottTriangleWave => &[
            None,
            Some(Letter('A')),
            Some(Letter('B')),
            Some(Letter('C')),
            Some(Letter('D')),
            Some(Letter('E')),
        ],
        DrawingKind::ElliottDoubleCombo => &[
            None,
            Some(Letter('W')),
            Some(Letter('X')),
            Some(Letter('Y')),
        ],
        DrawingKind::ElliottTripleCombo => &[
            None,
            Some(Letter('W')),
            Some(Letter('X')),
            Some(Letter('Y')),
            Some(Letter('X')),
            Some(Letter('Z')),
        ],
        _ => &[],
    }
}

fn build_parts(ctx: &PartContext<'_>, parts: &mut DrawingParts) {
    match ctx.drawing.kind {
        DrawingKind::HeadAndShoulders => head_and_shoulders(ctx, parts),
        DrawingKind::TrianglePattern => triangle_pattern(ctx, parts),
        DrawingKind::CyclicLines => cyclic_lines(ctx, parts),
        DrawingKind::TimeCycles => time_cycles(ctx, parts),
        DrawingKind::SineLine => sine_line(ctx, parts),
        kind if is_elliott(kind) => elliott_wave(ctx, parts),
        _ => harmonic_pattern(ctx, parts),
    }
}

/// The region fill: `fill_color`, else the stroke at [`FILL_ALPHA`]. `None` when fill is off.
fn fill_color(drawing: &Drawing) -> Option<Color> {
    if !drawing.fill_enabled {
        return None;
    }
    Some(drawing.fill_or_wash(FILL_ALPHA))
}

fn fill_triangle(ctx: &PartContext<'_>, parts: &mut DrawingParts, corners: [Option<Point>; 3]) {
    let (Some(fill), [Some(a), Some(b), Some(c)]) = (fill_color(ctx.drawing), corners) else {
        return;
    };
    parts.fill_convex(&[a, b, c], Some(fill), ctx.fills_hit());
}

/// Whether anchor `index` reads as a high (its label goes above) rather than a low: higher than
/// both neighbours, or, inside a monotonic run, reached by a rising leg.
fn is_high(px: &[Point], index: usize) -> bool {
    let y = px[index].1;
    let previous = index.checked_sub(1).and_then(|i| px.get(i)).map(|p| p.1);
    let next = px.get(index + 1).map(|p| p.1);
    match (previous, next) {
        (Some(previous), Some(next)) if y <= previous.min(next) => true,
        (Some(previous), Some(next)) if y >= previous.max(next) => false,
        (Some(previous), _) => y < previous,
        (None, Some(next)) => y < next,
        (None, None) => true,
    }
}

/// A boxed label in the stroke color with contrasting (or the drawing's `text_color`) text.
fn boxed_label(
    ctx: &PartContext<'_>,
    anchor: Point,
    v_align: DrawingTextVAlign,
    text: String,
) -> PartLabel {
    let drawing = ctx.drawing;
    let background = drawing.stroke_color();
    let color = drawing
        .text_color
        .as_deref()
        .and_then(Color::parse_css)
        .unwrap_or(text_on(background));
    PartLabel {
        anchor,
        h_align: DrawingTextHAlign::Center,
        v_align,
        lines: vec![text],
        size: ctx.engine.drawing_text_size(drawing) * ctx.scale,
        weight: drawing.text_weight.unwrap_or(400),
        italic: drawing.text_italic,
        color: Some(color),
        background: Some(background),
        border: None,
        padding: (LABEL_PADDING.0 * ctx.scale, LABEL_PADDING.1 * ctx.scale),
        hit: true,
    }
}

/// Boxed point labels above highs and below lows.
fn point_labels(ctx: &PartContext<'_>, parts: &mut DrawingParts) {
    let gap = LABEL_GAP * ctx.scale;
    for (index, (&point, label)) in ctx
        .px
        .iter()
        .zip(pattern_labels(ctx.drawing.kind))
        .enumerate()
    {
        let Some(text) = label else {
            continue;
        };
        let label = if is_high(ctx.px, index) {
            boxed_label(
                ctx,
                (point.0, point.1 - gap),
                DrawingTextVAlign::Bottom,
                text.to_string(),
            )
        } else {
            boxed_label(
                ctx,
                (point.0, point.1 + gap),
                DrawingTextVAlign::Top,
                text.to_string(),
            )
        };
        parts.label(label);
    }
}

/// The zigzag through every anchor placed so far.
fn zigzag(ctx: &PartContext<'_>, parts: &mut DrawingParts) {
    parts.stroke(ctx.px, PartStroke::default(), false);
}

/// XABCD, cypher, ABCD, and three drives: shaded XAB/BCD triangles (XABCD and cypher), the
/// zigzag, dashed ratio connectors with their ratios, and the point labels.
fn harmonic_pattern(ctx: &PartContext<'_>, parts: &mut DrawingParts) {
    let drawing = ctx.drawing;
    let px = ctx.px;
    if matches!(
        drawing.kind,
        DrawingKind::XabcdPattern | DrawingKind::CypherPattern
    ) {
        let at = |index: usize| px.get(index).copied();
        fill_triangle(ctx, parts, [at(0), at(1), at(2)]);
        fill_triangle(ctx, parts, [at(2), at(3), at(4)]);
    }
    zigzag(ctx, parts);
    if options(drawing).show_ratios {
        // Every connector paints before every ratio label, so a later connector never strikes
        // through an earlier label.
        let connector = PartStroke::decoration(DECORATION_WIDTH, LineStyle::Dashed);
        let placed = ratio_texts(drawing)
            .filter_map(|(ratio, text)| Some((*px.get(ratio.from)?, *px.get(ratio.to)?, text)))
            .collect::<Vec<_>>();
        for &(from, to, _) in &placed {
            parts.stroke(&[from, to], connector, false);
        }
        for (from, to, text) in placed {
            let middle = ((from.0 + to.0) / 2.0, (from.1 + to.1) / 2.0);
            parts.label(boxed_label(ctx, middle, DrawingTextVAlign::Middle, text));
        }
    }
    point_labels(ctx, parts);
}

/// Where the infinite line `a → b` crosses the segment `c → d`.
fn line_meets_segment(a: Point, b: Point, c: Point, d: Point) -> Option<Point> {
    let (t, u) = shape::line_intersection(a, b, c, d)?;
    (0.0..=1.0)
        .contains(&u)
        .then_some((a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t))
}

/// Head and shoulders (left base, left shoulder, neck, head, neck, right shoulder, right base):
/// the neckline through both neck anchors between the outer legs, the shoulders and head shaded
/// against it, the zigzag, and the three part labels.
fn head_and_shoulders(ctx: &PartContext<'_>, parts: &mut DrawingParts) {
    let px = ctx.px;
    let at = |index: usize| px.get(index).copied();
    let neckline = match (at(2), at(4)) {
        (Some(left_neck), Some(right_neck)) => {
            // The neckline runs from where it meets the first leg to where it meets the last one,
            // or from the neck anchors themselves where it misses those legs.
            let start = at(0)
                .zip(at(1))
                .and_then(|(base, shoulder)| {
                    line_meets_segment(left_neck, right_neck, base, shoulder)
                })
                .unwrap_or(left_neck);
            let end = at(5)
                .zip(at(6))
                .and_then(|(shoulder, base)| {
                    line_meets_segment(left_neck, right_neck, shoulder, base)
                })
                .unwrap_or(right_neck);
            Some((start, end))
        }
        _ => None,
    };
    if let Some((start, end)) = neckline {
        fill_triangle(ctx, parts, [Some(start), at(1), at(2)]);
        fill_triangle(ctx, parts, [at(2), at(3), at(4)]);
        fill_triangle(ctx, parts, [at(4), at(5), Some(end)]);
    }
    zigzag(ctx, parts);
    if let Some((start, end)) = neckline {
        parts.stroke(&[start, end], PartStroke::default(), false);
    }
    point_labels(ctx, parts);
}

/// Whether a quadrilateral is strictly convex (every turn the same way), so its ribbon covers
/// exactly its area.
fn is_convex(polygon: &[Point; 4]) -> bool {
    let mut sign = 0.0_f64;
    for index in 0..4 {
        let (a, b, c) = (
            polygon[index],
            polygon[(index + 1) % 4],
            polygon[(index + 2) % 4],
        );
        let cross = (b.0 - a.0) * (c.1 - b.1) - (b.1 - a.1) * (c.0 - b.0);
        if cross.abs() <= f64::EPSILON {
            return false;
        }
        if sign != 0.0 && cross.signum() != sign {
            return false;
        }
        sign = cross.signum();
    }
    true
}

/// Triangle pattern (A, B, C, D on alternating highs and lows): the A–C and B–D sides extended to
/// their apex when it lies ahead within one pattern width, the triangle between them shaded, the
/// zigzag, and the point labels. Sides that do not converge ahead stay between their anchors.
fn triangle_pattern(ctx: &PartContext<'_>, parts: &mut DrawingParts) {
    let px = ctx.px;
    if let [a, b, c, d] = *px {
        let forward = if (c.0 + d.0) >= (a.0 + b.0) {
            1.0
        } else {
            -1.0
        };
        let frontier = px
            .iter()
            .map(|p| p.0 * forward)
            .fold(f64::NEG_INFINITY, f64::max);
        let (left, right) = px
            .iter()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(l, r), p| {
                (l.min(p.0), r.max(p.0))
            });
        let apex = shape::line_intersection(a, c, b, d)
            .map(|(t, _)| (a.0 + (c.0 - a.0) * t, a.1 + (c.1 - a.1) * t))
            .filter(|apex| {
                let ahead = apex.0 * forward - frontier;
                apex.0.is_finite() && apex.1.is_finite() && ahead > 0.0 && ahead <= right - left
            });
        let fill = fill_color(ctx.drawing);
        match apex {
            Some(apex) => {
                if let Some(fill) = fill {
                    parts.fill_convex(&[a, apex, b], Some(fill), ctx.fills_hit());
                }
                zigzag(ctx, parts);
                parts.stroke(&[a, apex], PartStroke::default(), false);
                parts.stroke(&[b, apex], PartStroke::default(), false);
            }
            None => {
                if let Some(fill) = fill.filter(|_| is_convex(&[a, c, d, b])) {
                    parts.fill_convex(&[a, c, d, b], Some(fill), ctx.fills_hit());
                }
                zigzag(ctx, parts);
                parts.stroke(&[a, c], PartStroke::default(), false);
                parts.stroke(&[b, d], PartStroke::default(), false);
            }
        }
    } else {
        zigzag(ctx, parts);
        if let (Some(&a), Some(&c)) = (px.first(), px.get(2)) {
            parts.stroke(&[a, c], PartStroke::default(), false);
        }
    }
    point_labels(ctx, parts);
}

/// One Elliott label's text, ring, and glyph metrics in caller px.
struct WaveLabel {
    text: String,
    ring: bool,
    size: f64,
    width: f64,
}

impl WaveLabel {
    /// Half the label's vertical footprint: the ring's radius or half a text line.
    fn half_extent(&self, scale: f64) -> f64 {
        if self.ring {
            self.ring_radius(scale)
        } else {
            self.size * 1.25 / 2.0
        }
    }

    fn ring_radius(&self, scale: f64) -> f64 {
        self.width.max(self.size) / 2.0 + RING_PADDING * scale
    }
}

fn wave_label(engine: &ChartEngine, drawing: &Drawing, mark: WaveMark, scale: f64) -> WaveLabel {
    let (text, ring) = options(drawing).degree.label(mark);
    let size = engine.drawing_text_size(drawing) * scale;
    let width = engine.measure_text_run(
        &text,
        size,
        &engine.options.get().layout.font_family,
        drawing.text_weight.unwrap_or(400),
        drawing.text_italic,
    );
    WaveLabel {
        text,
        ring,
        size,
        width,
    }
}

/// Elliott waves: the wave polyline (unless `show_wave` is off) and each wave's degree label
/// above highs and below lows, ringed for the ringed degrees.
fn elliott_wave(ctx: &PartContext<'_>, parts: &mut DrawingParts) {
    let drawing = ctx.drawing;
    if options(drawing).show_wave {
        zigzag(ctx, parts);
    }
    let color = drawing
        .text_color
        .as_deref()
        .and_then(Color::parse_css)
        .unwrap_or_else(|| drawing.stroke_color());
    let gap = LABEL_GAP * ctx.scale;
    let mut ring = Vec::new();
    for (index, (&point, mark)) in ctx.px.iter().zip(wave_marks(drawing.kind)).enumerate() {
        let Some(mark) = *mark else {
            continue;
        };
        let label = wave_label(ctx.engine, drawing, mark, ctx.scale);
        let offset = gap + label.half_extent(ctx.scale);
        let center = if is_high(ctx.px, index) {
            (point.0, point.1 - offset)
        } else {
            (point.0, point.1 + offset)
        };
        if label.ring {
            ring.clear();
            shape::EllipseArc::circle(center, label.ring_radius(ctx.scale), 0.0, 2.0 * PI)
                .append_points(CURVE_TOLERANCE, &mut ring);
            parts.stroke(
                &ring,
                PartStroke {
                    color: Some(color),
                    width: Some(DECORATION_WIDTH),
                    style: Some(LineStyle::Solid),
                },
                false,
            );
        }
        parts.label(PartLabel {
            anchor: center,
            h_align: DrawingTextHAlign::Center,
            v_align: DrawingTextVAlign::Middle,
            lines: vec![label.text],
            size: label.size,
            weight: drawing.text_weight.unwrap_or(400),
            italic: drawing.text_italic,
            color: Some(color),
            background: None,
            border: None,
            padding: (0.0, 0.0),
            hit: true,
        });
    }
}

/// The repeat indexes `k` whose position `start + k·spacing` (or the interval after it, with
/// `whole_interval`) meets `[low, high]`, limited to `k >= min`.
fn visible_repeats(
    start: f64,
    spacing: f64,
    (low, high): (f64, f64),
    min: Option<i64>,
    whole_interval: bool,
) -> std::ops::RangeInclusive<i64> {
    let reach = if whole_interval { 1.0 } else { 0.0 };
    let first = ((low - start) / spacing - reach).ceil() as i64;
    let last = ((high - start) / spacing).floor() as i64;
    first.max(min.unwrap_or(i64::MIN))..=last
}

/// Cyclic lines: a dashed connector between the anchors and full-height vertical lines every
/// anchor interval from the earlier anchor to the pane's right edge.
fn cyclic_lines(ctx: &PartContext<'_>, parts: &mut DrawingParts) {
    let [a, b] = *ctx.px else {
        return;
    };
    let start = a.0.min(b.0);
    let spacing = (b.0 - a.0).abs();
    let pane = ctx.pane;
    let stroke = PartStroke::default();
    if spacing >= MIN_REPEAT_SPACING * ctx.x_scale {
        for k in visible_repeats(start, spacing, (pane.left, pane.right), Some(0), false) {
            parts.vline(start + k as f64 * spacing, pane.top, pane.bottom, stroke);
        }
    } else {
        parts.vline(a.0, pane.top, pane.bottom, stroke);
        parts.vline(b.0, pane.top, pane.bottom, stroke);
    }
    parts.stroke(
        &[a, b],
        PartStroke::decoration(DECORATION_WIDTH, LineStyle::Dashed),
        false,
    );
}

/// Time cycles: half-ellipse arches as wide as the anchors' interval and as tall as their price
/// difference, standing on the first anchor's price toward the second's, repeated both ways
/// across the pane and shaded when fill is on.
fn time_cycles(ctx: &PartContext<'_>, parts: &mut DrawingParts) {
    let [a, b] = *ctx.px else {
        return;
    };
    let width = (b.0 - a.0).abs();
    if width <= f64::EPSILON {
        parts.stroke(&[a, b], PartStroke::default(), false);
        return;
    }
    let start = a.0.min(b.0);
    let pane = ctx.pane;
    let repeats = if width >= MIN_REPEAT_SPACING * ctx.x_scale {
        visible_repeats(start, width, (pane.left, pane.right), None, true)
    } else {
        0..=0
    };
    let (rx, ry) = (width / 2.0, (b.1 - a.1).abs());
    // Up (screen y decreasing) sweeps π → 2π, down sweeps π → 0.
    let sweep = if b.1 <= a.1 { PI } else { -PI };
    let count = usize::try_from(
        repeats
            .end()
            .saturating_sub(*repeats.start())
            .saturating_add(1),
    )
    .unwrap_or(0);
    let per_arch = shape::arc_segment_count(rx.max(ry), PI, CURVE_TOLERANCE);
    let excess = (count * per_arch) as f64 / MAX_CURVE_POINTS as f64;
    let tolerance = CURVE_TOLERANCE * excess.max(1.0).powi(2);
    let fill = fill_color(ctx.drawing);
    let hits = ctx.fills_hit();
    let mut arch = Vec::new();
    for k in repeats {
        arch.clear();
        let center = (start + (k as f64 + 0.5) * width, a.1);
        shape::EllipseArc {
            center,
            rx,
            ry,
            rotation: 0.0,
            start: PI,
            sweep,
        }
        .append_points(tolerance, &mut arch);
        if let Some(fill) = fill {
            parts.fill_convex(&arch, Some(fill), hits);
        }
        parts.stroke(&arch, PartStroke::default(), false);
    }
}

/// Sine line: a sine whose peak (or trough) sits on the first anchor and whose opposite extreme
/// sits on the second, half a period later, continued across the pane.
fn sine_line(ctx: &PartContext<'_>, parts: &mut DrawingParts) {
    let [a, b] = *ctx.px else {
        return;
    };
    let half_period = (b.0 - a.0).abs();
    if half_period <= f64::EPSILON {
        parts.stroke(&[a, b], PartStroke::default(), false);
        return;
    }
    let (middle, amplitude) = ((a.1 + b.1) / 2.0, (a.1 - b.1) / 2.0);
    let pane = ctx.pane;
    let (first, last) = if half_period >= MIN_REPEAT_SPACING * ctx.x_scale {
        let repeats = visible_repeats(a.0, half_period, (pane.left, pane.right), None, true);
        (*repeats.start(), *repeats.end())
    } else {
        let first = if b.0 >= a.0 { 0 } else { -1 };
        (first, first)
    };
    let half_periods = usize::try_from(last.saturating_sub(first).saturating_add(1))
        .unwrap_or(0)
        .max(1);
    // Chord error of a sine sampled every `step` px is at most amplitude · (π/H)² · step² / 8.
    let curvature = amplitude.abs() * (PI / half_period).powi(2);
    let step = (8.0 * CURVE_TOLERANCE / curvature.max(f64::MIN_POSITIVE)).sqrt();
    let per_half = ((half_period / step).ceil() as usize)
        .clamp(2, 256)
        .min((MAX_CURVE_POINTS / half_periods).max(2));
    let mut wave = Vec::with_capacity(half_periods * per_half + 1);
    for sample in 0..=half_periods * per_half {
        let phase = first as f64 + sample as f64 / per_half as f64;
        wave.push((
            a.0 + phase * half_period,
            middle + amplitude * (PI * phase).cos(),
        ));
    }
    parts.stroke(&wave, PartStroke::default(), false);
}

/// Conservative reach of point, ratio, and wave labels beyond the anchors' box, in CSS px.
fn decoration_extent(engine: &ChartEngine, drawing: &Drawing) -> f64 {
    let size = engine.drawing_text_size(drawing);
    let family = &engine.options.get().layout.font_family;
    let weight = drawing.text_weight.unwrap_or(400);
    let measure =
        |text: &str| engine.measure_text_run(text, size, family, weight, drawing.text_italic);
    if is_elliott(drawing.kind) {
        return wave_marks(drawing.kind)
            .iter()
            .flatten()
            .map(|&mark| {
                // The label centers `gap + half_extent` off its anchor; beyond that center reach
                // its ring and its text box (a line tall, which outgrows the ring at large sizes).
                let label = wave_label(engine, drawing, mark, 1.0);
                let half_box = label.size * 1.25 / 2.0;
                let ring = if label.ring {
                    label.ring_radius(1.0)
                } else {
                    0.0
                };
                let vertical = LABEL_GAP + label.half_extent(1.0) + half_box.max(ring);
                vertical.max(label.width / 2.0)
            })
            .fold(0.0, f64::max);
    }
    // Ratio labels are centered on connectors inside the anchors' box, so half their width can
    // reach past its edge. They are measured from the drawing's own prices (the hook reruns on
    // every drawing mutation): a near-flat reference leg prints a ratio of any width.
    let widest = pattern_labels(drawing.kind)
        .iter()
        .flatten()
        .map(|text| measure(text))
        .chain(
            options(drawing)
                .show_ratios
                .then(|| ratio_texts(drawing).map(|(_, text)| measure(&text)))
                .into_iter()
                .flatten(),
        )
        .fold(0.0_f64, f64::max);
    if widest <= 0.0 {
        return 0.0;
    }
    let height = size * 1.25 + 2.0 * LABEL_PADDING.1;
    (LABEL_GAP + height).max(widest / 2.0 + LABEL_PADDING.0)
}

fn extend_schema(template: &Drawing, properties: &mut Vec<DrawingPropertyDescriptor>) {
    let defaults = options(template);
    if is_elliott(template.kind) {
        let mut degree = descriptor(
            "tool_options.pattern.degree",
            DrawingPropertyType::Enum,
            serde_json::json!(defaults.degree.name()),
        );
        degree.enum_values = ElliottWaveDegree::ALL
            .into_iter()
            .map(|degree| degree.name().to_string())
            .collect();
        properties.push(degree);
        properties.push(descriptor(
            "tool_options.pattern.show_wave",
            DrawingPropertyType::Boolean,
            serde_json::json!(defaults.show_wave),
        ));
    } else if !ratios(template.kind).is_empty() {
        properties.push(descriptor(
            "tool_options.pattern.show_ratios",
            DrawingPropertyType::Boolean,
            serde_json::json!(defaults.show_ratios),
        ));
    }
}

fn kind_options(drawing: &Drawing) -> DrawingKindOptions {
    let options = options(drawing);
    if is_elliott(drawing.kind) {
        DrawingKindOptions::ElliottWave {
            degree: options.degree,
            show_wave: options.show_wave,
        }
    } else if !ratios(drawing.kind).is_empty() {
        DrawingKindOptions::Pattern {
            show_ratios: options.show_ratios,
        }
    } else {
        DrawingKindOptions::Generic
    }
}

#[cfg(test)]
mod tests;
