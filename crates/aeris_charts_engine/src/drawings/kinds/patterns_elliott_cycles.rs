//! B8 Patterns, Elliott waves, and cycles options, legacy defaults, and the fork presentation
//! layered on upstream's lowering. Upstream renders the harmonic patterns, head and shoulders,
//! triangle pattern, three drives, and the Elliott waves as its `Polyline` body (`geometry.rs`
//! resolver, the frame's polyline arm with its vertex labels, the polyline hit arm), and the cycle
//! tools from its catalog spec. This module keeps the fork's public option block
//! ([`PatternToolOptions`], [`ElliottWaveDegree`]), the fork's pre-merge kind defaults for
//! documents it wrote ([`legacy_defaults`]), and what the fork drew over the zigzag, resolved into
//! shared parts layered on that arm ([`pattern_parts`], under and over the zigzag): the harmonic
//! patterns' dashed ratio connectors and boxed ratios (`show_ratios`, on by default), the shaded
//! XABCD and cypher triangles (`fill_enabled`), the head-and-shoulders neckline (always) with its
//! shoulders and head shaded (`fill_enabled`), and the triangle pattern's sides to their apex
//! (`extend_left`/`extend_right`). The vertex labels keep upstream's placement and are body hit
//! targets ([`vertex_labels`]); Elliott waves label in the Frost–Prechter notation of their
//! `wave_degree` with the start unlabeled, and `show_wave: false` leaves only the labels.

use std::f64::consts::PI;

use aeris_charts_render::color::Color;
use aeris_charts_render::draw_list::LineStyle;
use aeris_charts_render::shape::{self, Point};

use super::super::geometry::CURVE_TOLERANCE;
use super::super::parts::{DrawingParts, PartContext, PartLabel, PartStroke, text_on};
use super::super::{Drawing, DrawingTextHAlign, DrawingTextVAlign};
use crate::{ChartEngine, DrawingKind, DrawingPropertyDescriptor, DrawingPropertyType};

/// The fork's Elliott wave degree, largest first; the default is TradingView's `Intermediate`.
/// Its snake_case names are the upstream `wave_degree` values, and each degree labels its waves
/// in its own notation ([`ElliottWaveDegree::label`]).
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
pub(crate) enum WaveMark {
    Number(u8),
    Letter(char),
}

impl WaveMark {
    /// The mark of one of upstream's Elliott vertex labels (`DrawingKind::vertex_labels`); `None`
    /// for the wave start `"0"`, which the notation leaves unlabeled.
    fn of(label: &str) -> Option<Self> {
        let mut chars = label.chars();
        let (Some(first), None) = (chars.next(), chars.next()) else {
            return None;
        };
        match first {
            '1'..='5' => Some(Self::Number(first as u8 - b'0')),
            letter if letter.is_ascii_alphabetic() => Some(Self::Letter(letter)),
            _ => None,
        }
    }
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

    /// The degree named by an upstream `wave_degree` value.
    pub(crate) fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|degree| degree.name() == name)
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
    pub(crate) fn label(self, mark: WaveMark) -> (String, bool) {
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

/// The fork's pattern options (`tool_options.pattern`); absent fields keep their defaults.
/// `degree` is an input alias of the flat `wave_degree` (see
/// `drawing_contract::take_legacy_flat_options`); `show_ratios` and `show_wave` are rendered on
/// upstream's polyline arm.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct PatternToolOptions {
    /// Harmonic patterns (XABCD, cypher, ABCD, three drives): the dashed ratio connectors and
    /// their ratio labels. Default `true`.
    pub show_ratios: bool,
    /// Elliott waves: an input alias of `wave_degree`, whose notation labels the waves. Default
    /// `intermediate` (the fork's default degree, which its documents omitted).
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
/// Upstream's vertex label offset: the label's center sits this many CSS px above its vertex.
const LABEL_OFFSET: f64 = 8.0;
/// Gap between a vertex and the bottom of its label's degree ring, in CSS px.
const RING_GAP: f64 = 6.0;
/// Space between an Elliott label's glyphs and its degree ring, in CSS px.
const RING_PADDING: f64 = 2.0;
/// Ratio label box padding (horizontal, vertical) in CSS px.
const LABEL_PADDING: (f64, f64) = (4.0, 2.0);
/// Width of the dashed ratio connectors and the degree rings in CSS px.
pub(crate) const DECORATION_WIDTH: f64 = 1.0;

fn options(drawing: &Drawing) -> PatternToolOptions {
    drawing.tool_options.pattern.unwrap_or_default()
}

/// Whether the kind layers [`pattern_parts`] on upstream's polyline arm: the harmonic patterns,
/// head and shoulders, and the triangle pattern (the Elliott waves add only their labels).
pub(crate) const fn layers_parts(kind: DrawingKind) -> bool {
    matches!(
        kind,
        DrawingKind::PatternXabcd
            | DrawingKind::PatternCypher
            | DrawingKind::PatternAbcd
            | DrawingKind::PatternHeadShoulders
            | DrawingKind::PatternTriangle
            | DrawingKind::PatternThreeDrives
    )
}

/// Whether the polyline arm strokes (and hit-tests) the zigzag: always, except an Elliott wave
/// whose `show_wave` is off, which keeps only its labels.
pub(crate) fn draws_wave(drawing: &Drawing) -> bool {
    !drawing.kind.is_elliott() || options(drawing).show_wave
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
/// Three drives (upstream's anchors 0, 1, A, 2, B, 3): each retracement against its drive and
/// each drive against its retracement.
const THREE_DRIVES_RATIOS: [Ratio; 4] = [leg_ratio(1), leg_ratio(2), leg_ratio(3), leg_ratio(4)];

fn ratios(kind: DrawingKind) -> &'static [Ratio] {
    match kind {
        DrawingKind::PatternXabcd => &XABCD_RATIOS,
        DrawingKind::PatternCypher => &CYPHER_RATIOS,
        DrawingKind::PatternAbcd => &ABCD_RATIOS,
        DrawingKind::PatternThreeDrives => &THREE_DRIVES_RATIOS,
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

/// The label text of every ratio the drawing shows (`show_ratios`) and its prices define, in
/// connector order.
fn ratio_texts(drawing: &Drawing) -> impl Iterator<Item = (Ratio, String)> + '_ {
    let ratios = if options(drawing).show_ratios {
        ratios(drawing.kind)
    } else {
        &[]
    };
    ratios.iter().filter_map(|&ratio| {
        ratio_value(drawing, ratio).map(|value| (ratio, format!("{value:.3}")))
    })
}

/// Which side of upstream's polyline arm a [`pattern_parts`] layer paints on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PatternLayer {
    /// Before the zigzag: the region fills.
    Under,
    /// After the zigzag and before the vertex labels: the neckline or triangle sides, then every
    /// ratio connector, then every ratio label (so no connector strikes through a label).
    Over,
}

/// The parts a pattern ([`layers_parts`]) layers on upstream's polyline arm at its anchors
/// `ctx.px`, resolved from however many anchors exist (a placement preview): the harmonic ratio
/// connectors and labels, the XABCD and cypher triangles, the head-and-shoulders neckline and its
/// shading, and the triangle pattern's sides. Region fills are body targets only while the
/// drawing is selected; the strokes and ratio labels always are.
pub(crate) fn pattern_parts(ctx: &PartContext<'_>, layer: PatternLayer, parts: &mut DrawingParts) {
    let at = |index: usize| ctx.px.get(index).copied();
    match ctx.drawing.kind {
        DrawingKind::PatternXabcd | DrawingKind::PatternCypher if layer == PatternLayer::Under => {
            fill_triangle(ctx, parts, [at(0), at(1), at(2)]);
            fill_triangle(ctx, parts, [at(2), at(3), at(4)]);
        }
        DrawingKind::PatternHeadShoulders => {
            let Some((start, end)) = neckline(ctx.px) else {
                return;
            };
            if layer == PatternLayer::Under {
                fill_triangle(ctx, parts, [Some(start), at(1), at(2)]);
                fill_triangle(ctx, parts, [at(2), at(3), at(4)]);
                fill_triangle(ctx, parts, [at(4), at(5), Some(end)]);
            } else {
                parts.stroke(&[start, end], PartStroke::default(), false);
            }
        }
        DrawingKind::PatternTriangle => triangle_sides(ctx, layer, parts),
        _ => {}
    }
    if layer == PatternLayer::Over {
        ratio_parts(ctx, parts);
    }
}

/// The region fill: `fill_color`, else the stroke at [`FILL_ALPHA`]. `None` when fill is off.
fn fill_color(drawing: &Drawing) -> Option<Color> {
    drawing
        .fill_enabled
        .then(|| drawing.fill_or_wash(FILL_ALPHA))
}

fn fill_triangle(ctx: &PartContext<'_>, parts: &mut DrawingParts, corners: [Option<Point>; 3]) {
    let (Some(fill), [Some(a), Some(b), Some(c)]) = (fill_color(ctx.drawing), corners) else {
        return;
    };
    parts.fill_convex(&[a, b, c], Some(fill), ctx.fills_hit());
}

/// Every ratio connector (dashed, [`DECORATION_WIDTH`]) and then every boxed ratio at its
/// connector's midpoint: the box in the stroke color, the text in `text_color` or contrasting.
fn ratio_parts(ctx: &PartContext<'_>, parts: &mut DrawingParts) {
    let drawing = ctx.drawing;
    let placed = ratio_texts(drawing)
        .filter_map(|(ratio, text)| Some((*ctx.px.get(ratio.from)?, *ctx.px.get(ratio.to)?, text)))
        .collect::<Vec<_>>();
    let connector = PartStroke::decoration(DECORATION_WIDTH, LineStyle::Dashed);
    for &(from, to, _) in &placed {
        parts.stroke(&[from, to], connector, false);
    }
    let background = drawing.stroke_color();
    let color = drawing
        .text_color
        .as_deref()
        .and_then(Color::parse_css)
        .unwrap_or(text_on(background));
    for (from, to, text) in placed {
        parts.label(PartLabel {
            anchor: ((from.0 + to.0) / 2.0, (from.1 + to.1) / 2.0),
            h_align: DrawingTextHAlign::Center,
            v_align: DrawingTextVAlign::Middle,
            lines: vec![text],
            size: ctx.engine.drawing_text_size(drawing) * ctx.scale,
            weight: drawing.text_weight.unwrap_or(400),
            italic: drawing.text_italic,
            color: Some(color),
            background: Some(background),
            border: None,
            padding: (LABEL_PADDING.0 * ctx.scale, LABEL_PADDING.1 * ctx.scale),
            hit: true,
        });
    }
}

/// Where the infinite line `a → b` crosses the segment `c → d`.
fn line_meets_segment(a: Point, b: Point, c: Point, d: Point) -> Option<Point> {
    let (t, u) = shape::line_intersection(a, b, c, d)?;
    (0.0..=1.0)
        .contains(&u)
        .then_some((a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t))
}

/// The head-and-shoulders neckline (anchors left base, left shoulder, neck, head, neck, right
/// shoulder, right base): the line through both neck anchors, from where it meets the first leg
/// to where it meets the last one, or from the neck anchors themselves where it misses those legs
/// (or they are not placed yet). `None` until both neck anchors exist.
fn neckline(px: &[Point]) -> Option<(Point, Point)> {
    let at = |index: usize| px.get(index).copied();
    let (left_neck, right_neck) = (at(2)?, at(4)?);
    let start = at(0)
        .zip(at(1))
        .and_then(|(base, shoulder)| line_meets_segment(left_neck, right_neck, base, shoulder))
        .unwrap_or(left_neck);
    let end = at(5)
        .zip(at(6))
        .and_then(|(shoulder, base)| line_meets_segment(left_neck, right_neck, shoulder, base))
        .unwrap_or(right_neck);
    Some((start, end))
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

/// The triangle pattern's sides over its first four anchors A, B, C, D (upstream's fifth, E,
/// does not shape them; on a migrated fork triangle it lies on line A–C). The sides run A–C and
/// B–D; where they converge ahead of the anchors (in the direction from A, B toward C, D) within
/// one pattern width, they reach their apex instead. The sides paint only while the extend flag
/// of that direction is set (`extend_right` forward to the right, `extend_left` to the left);
/// fork documents set both. The fill (`fill_enabled`) shades the triangle A, apex, B, else the
/// convex quad A, C, D, B.
fn triangle_sides(ctx: &PartContext<'_>, layer: PatternLayer, parts: &mut DrawingParts) {
    let drawing = ctx.drawing;
    let [a, b, c, d] = match ctx.px {
        [a, b, c, d, ..] => [*a, *b, *c, *d],
        _ => return,
    };
    let (apex, sides) = triangle_apex(drawing, [a, b, c, d]);
    match (layer, apex) {
        (PatternLayer::Under, Some(apex)) => {
            if let Some(fill) = fill_color(drawing) {
                parts.fill_convex(&[a, apex, b], Some(fill), ctx.fills_hit());
            }
        }
        (PatternLayer::Under, None) => {
            if let Some(fill) = fill_color(drawing).filter(|_| is_convex(&[a, c, d, b])) {
                parts.fill_convex(&[a, c, d, b], Some(fill), ctx.fills_hit());
            }
        }
        (PatternLayer::Over, _) if !sides => {}
        (PatternLayer::Over, Some(apex)) => {
            parts.stroke(&[a, apex], PartStroke::default(), false);
            parts.stroke(&[b, apex], PartStroke::default(), false);
        }
        (PatternLayer::Over, None) => {
            parts.stroke(&[a, c], PartStroke::default(), false);
            parts.stroke(&[b, d], PartStroke::default(), false);
        }
    }
}

/// The triangle pattern's apex at its sides A–C and B–D (any px space), when the extend flag of
/// its forward direction selects the sides and they converge ahead within one pattern width, and
/// whether that flag selects the sides at all.
fn triangle_apex(drawing: &Drawing, [a, b, c, d]: [Point; 4]) -> (Option<Point>, bool) {
    let forward = if (c.0 + d.0) >= (a.0 + b.0) {
        1.0
    } else {
        -1.0
    };
    let sides = if forward > 0.0 {
        drawing.extend_right
    } else {
        drawing.extend_left
    };
    if !sides {
        return (None, false);
    }
    let corners = [a, b, c, d];
    let frontier = corners
        .iter()
        .map(|p| p.0 * forward)
        .fold(f64::NEG_INFINITY, f64::max);
    let (left, right) = corners
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
    (apex, true)
}

/// Whether a triangle pattern's sides may reach past its anchors (to an apex up to one pattern
/// width ahead), which `DrawingBounds` covers with logical bounds padded by the anchors' span.
pub(crate) fn extends_to_apex(drawing: &Drawing) -> bool {
    drawing.kind == DrawingKind::PatternTriangle && (drawing.extend_left || drawing.extend_right)
}

/// One vertex label as upstream's polyline arm paints it: a centered one-line run whose
/// vertical center is `center`, and for a ringed Elliott degree the ring's radius around it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct VertexLabel {
    pub(crate) center: Point,
    pub(crate) text: String,
    pub(crate) ring: Option<f64>,
}

/// The texts of a drawing's vertex labels by anchor index with whether each is ringed:
/// upstream's labels, or an Elliott wave's in the notation of its `wave_degree` without the
/// wave start.
fn vertex_label_texts(drawing: &Drawing) -> impl Iterator<Item = (usize, String, bool)> + '_ {
    let elliott = drawing.kind.is_elliott().then(|| {
        ElliottWaveDegree::from_name(&drawing.wave_degree).unwrap_or(ElliottWaveDegree::Minor)
    });
    drawing
        .kind
        .vertex_labels()
        .unwrap_or(&[])
        .iter()
        .enumerate()
        .filter_map(move |(index, &label)| match elliott {
            Some(degree) => {
                let (text, ring) = degree.label(WaveMark::of(label)?);
                Some((index, text, ring))
            }
            None => Some((index, label.to_string(), false)),
        })
}

/// The radius of the degree ring around a label `width` wide at glyph `size` (caller px, `scale`
/// caller px per CSS px).
fn ring_radius(width: f64, size: f64, scale: f64) -> f64 {
    width.max(size) / 2.0 + RING_PADDING * scale
}

/// The vertex labels of `drawing` at its anchors `px` (caller px, `scale` caller px per CSS px),
/// with their glyph size in caller px: upstream's placement, each run centered [`LABEL_OFFSET`]
/// above its vertex, except that a ringed label rises until its ring clears the vertex by
/// [`RING_GAP`]. Paint (the polyline arm), hit testing and the culling pad share it.
pub(crate) fn vertex_labels(
    engine: &ChartEngine,
    drawing: &Drawing,
    px: &[Point],
    scale: f64,
) -> (f64, Vec<VertexLabel>) {
    let size = engine.drawing_text_size(drawing) * scale;
    let labels = vertex_label_texts(drawing)
        .filter_map(|(index, text, ring)| {
            let (x, y) = *px.get(index)?;
            if ring {
                let radius = ring_radius(label_width(engine, drawing, &text, size), size, scale);
                Some(VertexLabel {
                    center: (x, y - (RING_GAP * scale + radius)),
                    text,
                    ring: Some(radius),
                })
            } else {
                Some(VertexLabel {
                    center: (x, y - LABEL_OFFSET * scale),
                    text,
                    ring: None,
                })
            }
        })
        .collect();
    (size, labels)
}

fn label_width(engine: &ChartEngine, drawing: &Drawing, text: &str, size: f64) -> f64 {
    engine.measure_text_run(
        text,
        size,
        &engine.options.get().layout.font_family,
        drawing.text_weight.unwrap_or(400),
        drawing.text_italic,
    )
}

/// The degree ring around `label` as a closed polyline in its caller px, flattened within
/// [`CURVE_TOLERANCE`]; nothing for an unringed label.
pub(crate) fn ring_points(label: &VertexLabel, out: &mut Vec<Point>) {
    out.clear();
    if let Some(radius) = label.ring {
        shape::EllipseArc::circle(label.center, radius, 0.0, 2.0 * PI)
            .append_points(CURVE_TOLERANCE, out);
    }
}

/// Conservative CSS-px reach beyond the anchors' box of what the polyline arm paints for a
/// vertex-label kind (the culling pad, see [`super::upstream_decoration_extent`]): each vertex
/// label (its offset above the vertex plus half a line, or its ring and the ring's hit tolerance)
/// and each ratio label, centered on a connector inside the box, by half its measured width plus
/// padding. The ratios are measured from the drawing's own prices (the cache refreshes on every
/// mutation): a near-flat reference leg prints a ratio of any width. 0 for other kinds.
pub(crate) fn upstream_decoration_extent(engine: &ChartEngine, drawing: &Drawing) -> f64 {
    if drawing.kind.vertex_labels().is_none() {
        return 0.0;
    }
    let size = engine.drawing_text_size(drawing);
    let labels = vertex_label_texts(drawing)
        .map(|(_, text, ring)| {
            let width = label_width(engine, drawing, &text, size);
            if ring {
                // The ring's hit area reaches the stroke tolerance past its edge (at its widest,
                // the touch profile the screen bounds assume).
                let radius = ring_radius(width, size, 1.0);
                RING_GAP + 2.0 * radius + crate::HitProfile::TOUCH.drawing_stroke_tolerance
            } else {
                (LABEL_OFFSET + size * 1.25 / 2.0).max(width / 2.0)
            }
        })
        .fold(0.0_f64, f64::max);
    ratio_texts(drawing)
        .map(|(_, text)| {
            let width = label_width(engine, drawing, &text, size);
            (width / 2.0 + LABEL_PADDING.0).max(size * 1.25 / 2.0 + LABEL_PADDING.1)
        })
        .fold(labels, f64::max)
}

/// The `tool_options.pattern` descriptors the pattern kinds read on upstream's polyline arm (see
/// [`super::extend_upstream_schema`]): `show_ratios` on the four harmonic patterns and
/// `show_wave` on the Elliott waves (whose `degree` alias is the flat `wave_degree` row).
/// Defaults follow `template` (a kind's `Drawing::new`, which has no block).
pub(crate) fn extend_upstream_schema(
    kind: DrawingKind,
    template: &Drawing,
    properties: &mut Vec<DrawingPropertyDescriptor>,
) {
    let defaults = options(template);
    let (name, value) = if kind.is_elliott() {
        ("show_wave", defaults.show_wave)
    } else if !ratios(kind).is_empty() {
        ("show_ratios", defaults.show_ratios)
    } else {
        return;
    };
    properties.push(crate::drawing_contract::descriptor(
        format!("tool_options.pattern.{name}"),
        DrawingPropertyType::Boolean,
        serde_json::json!(value),
    ));
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
