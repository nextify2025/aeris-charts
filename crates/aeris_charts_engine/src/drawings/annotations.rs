//! Shared layout for the box annotations: note, comment, callout, price note, and signpost.
//!
//! One resolution feeds frame construction, precise hit testing, culling, the painted caret, and
//! the text-editor layout, so the painted box, its grab area, and the editing caret always
//! coincide. Every length is CSS px multiplied by the caller's `scale` (1 for media px, the
//! device ratio for bitmap px). A fork-form drawing (`kinds::projection_annotations::fork_form`)
//! has no layout here: its pin, bubble, boxed price, or plate is the fork's.

use super::geometry::arc_segments;
use super::{Drawing, DrawingKind};
use crate::ChartEngine;
use crate::frame::ChromeTokens;
use aeris_charts_render::color::Color;

/// Shown in an empty annotation's box while its text is being typed.
pub(crate) const ANNOTATION_PLACEHOLDER: &str = "Add text";
/// Box padding around the text run.
const PAD_X: f64 = 10.0;
const PAD_Y: f64 = 6.0;
/// The comment bubble's fully rounded ends need more side room.
const COMMENT_PAD_X: f64 = 14.0;
const BOX_RADIUS: f64 = 4.0;
const CALLOUT_RADIUS: f64 = 6.0;
/// The callout tail's base width where it joins the box edge.
const CALLOUT_TAIL_BASE: f64 = 18.0;
/// A one-click signpost's text box sits this far above the clicked bar.
pub(crate) const SIGNPOST_PRESET_HEIGHT: f64 = 96.0;

/// A box annotation's colors: its box fill and border, its text ink, and the placeholder's
/// muted ink. Shared by the frame and the text editor so typed text matches the painted text.
pub(crate) struct AnnotationPaint {
    pub(crate) fill: Color,
    pub(crate) border: Option<Color>,
    pub(crate) ink: Color,
    pub(crate) muted: Color,
}

impl AnnotationPaint {
    /// The text's color: `text_color` when set, else the ink contrasting with the fill; the
    /// empty box's placeholder is muted.
    pub(crate) fn text_color(&self, drawing: &Drawing, placeholder: bool) -> Color {
        if placeholder {
            return self.muted;
        }
        drawing
            .text_color
            .as_deref()
            .and_then(Color::parse_css)
            .unwrap_or(self.ink)
    }
}

/// One annotation's resolved geometry in the caller's px basis.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct AnnotationLayout {
    /// The box: left, top, width, height.
    pub(crate) rect: [f64; 4],
    /// Corner radii: left-top, right-top, right-bottom, left-bottom.
    pub(crate) radii: [f64; 4],
    /// The text: left edge, the first line's vertical center, glyph size, string, line count
    /// and pitch, and whether it is the empty-box placeholder. A multi-line kind
    /// ([`DrawingKind::multiline_kind`]) stacks its `\n`-separated lines (owner decision R9);
    /// the signpost's box is one line.
    pub(crate) text_x: f64,
    pub(crate) text_y: f64,
    pub(crate) size: f64,
    pub(crate) text: String,
    pub(crate) lines: usize,
    pub(crate) line_height: f64,
    pub(crate) placeholder: bool,
    /// The note's leader, the price note's line, or the signpost's post.
    pub(crate) connector: Option<((f64, f64), (f64, f64))>,
    /// The callout's tail: tip, then its two base points on the box edge facing the tip, in
    /// outline order.
    pub(crate) tail: Option<[(f64, f64); 3]>,
    /// The callout's one closed outline (rounded box with the tail spliced into its edge),
    /// starting and ending mid-edge so a stroked seam lands on a straight run.
    pub(crate) outline: Option<Vec<(f64, f64)>>,
}

impl AnnotationLayout {
    pub(crate) fn contains(&self, (x, y): (f64, f64), tolerance: f64) -> bool {
        let [left, top, width, height] = self.rect;
        x >= left - tolerance
            && x <= left + width + tolerance
            && y >= top - tolerance
            && y <= top + height + tolerance
    }

    /// Each painted line with its vertical center, top to bottom.
    pub(crate) fn rows(&self) -> impl Iterator<Item = (f64, &str)> {
        self.text
            .splitn(self.lines, '\n')
            .enumerate()
            .map(|(row, line)| (self.text_y + row as f64 * self.line_height, line))
    }
}

impl ChartEngine {
    /// The annotation's text and whether it is the placeholder: a price note shows its custom
    /// text or the first anchor's formatted price; the other kinds show their text or, while
    /// empty, [`ANNOTATION_PLACEHOLDER`].
    pub(crate) fn annotation_text(&self, drawing: &Drawing) -> (String, bool) {
        if !drawing.text.is_empty() {
            return (drawing.display_text().to_string(), false);
        }
        if drawing.kind == DrawingKind::PriceNote {
            let price = drawing.points.first().map_or(0.0, |point| point.price);
            return (self.format_drawing_price(drawing, price), false);
        }
        (ANNOTATION_PLACEHOLDER.to_string(), true)
    }

    /// The box colors of `drawing`. Unset box colors follow the painted theme's tokens (note,
    /// signpost) or the drawing color (comment and price-note fills, the callout's opaque tint
    /// and border); the text contrasts with its fill.
    pub(crate) fn annotation_paint(&self, drawing: &Drawing) -> AnnotationPaint {
        let color = drawing.stroke_color();
        let tokens = ChromeTokens::for_theme(self.surface_theme());
        let parse = |css: &Option<String>| css.as_deref().and_then(Color::parse_css);
        let (box_fill, box_border) = (parse(&drawing.box_color), parse(&drawing.box_border_color));
        let (fill, border, ink, muted) = match drawing.kind {
            DrawingKind::Note => (
                box_fill.unwrap_or(tokens.accent),
                box_border,
                tokens.foreground,
                tokens.muted,
            ),
            DrawingKind::Signpost => (
                box_fill.unwrap_or(tokens.surface),
                Some(box_border.unwrap_or(tokens.border)),
                tokens.foreground,
                tokens.muted,
            ),
            // An opaque tint of the drawing color over the chart background: the box and its
            // tail fill as one surface, so their overlap never doubles any alpha.
            DrawingKind::Callout => (
                box_fill.unwrap_or_else(|| {
                    let background = Color::parse_css(&self.options.get().layout.background.color)
                        .unwrap_or(tokens.surface);
                    let mix = |a: u8, b: u8| {
                        ((f64::from(a) * 0.55) + (f64::from(b) * 0.45)).round() as u8
                    };
                    Color::rgb(
                        mix(background.r(), color.r()),
                        mix(background.g(), color.g()),
                        mix(background.b(), color.b()),
                    )
                }),
                Some(box_border.unwrap_or(color)),
                tokens.foreground,
                tokens.muted,
            ),
            _ => {
                let fill = box_fill.unwrap_or(color);
                let ink = fill.contrast_text();
                (
                    fill,
                    box_border,
                    ink,
                    Color::rgba(ink.r(), ink.g(), ink.b(), 170),
                )
            }
        };
        AnnotationPaint {
            fill,
            border,
            ink,
            muted,
        }
    }

    /// The annotation's box, text, and connector for anchors `px` (semantic order, in the same
    /// px basis as `scale`). A legacy one-anchor note or price note places its box on that
    /// anchor with no connector.
    pub(crate) fn annotation_layout(
        &self,
        drawing: &Drawing,
        px: &[(f64, f64)],
        scale: f64,
    ) -> Option<AnnotationLayout> {
        if !drawing.kind.is_annotation() || super::kinds::projection_annotations::fork_form(drawing)
        {
            return None;
        }
        let &first = px.first()?;
        let second = px.get(1).copied();
        let layout = &self.options.get().layout;
        let size = drawing.resolved_text_size(layout.font_size) * scale;
        let (text, placeholder) = self.annotation_text(drawing);
        let weight = annotation_text_weight(drawing);
        let measure = |line: &str| {
            self.measure_text_run(line, size, &layout.font_family, weight, drawing.text_italic)
        };
        let (advance, lines) = if drawing.kind.multiline_kind() && text.contains('\n') {
            text.split('\n')
                .fold((0.0_f64, 0), |(widest, count), line| {
                    (widest.max(measure(line)), count + 1)
                })
        } else {
            (measure(&text), 1)
        };
        let pad_x = if drawing.kind == DrawingKind::Comment {
            COMMENT_PAD_X
        } else {
            PAD_X
        } * scale;
        let line_height = size * 1.2;
        let width = advance + 2.0 * pad_x;
        // Lines stack inside the box: the note and signpost grow down from their hanging edge,
        // the comment, callout, and price note up from their bottom edge.
        let height = lines as f64 * line_height + 2.0 * PAD_Y * scale;
        let radius = BOX_RADIUS * scale;
        let mut connector = None;
        let mut tail = None;
        let mut outline = None;
        let (left, top, radii) = match drawing.kind {
            // The box hangs from its anchor at the top center, led from the pinned point.
            DrawingKind::Note => {
                let anchor = second.unwrap_or(first);
                if second.is_some() {
                    connector = Some((first, anchor));
                }
                (anchor.0 - width / 2.0, anchor.1, [radius; 4])
            }
            // The post top carries the box (top center); the post drops to its base.
            DrawingKind::Signpost => {
                let anchor = second.unwrap_or(first);
                let bottom = anchor.1 + height;
                if first.1 > bottom {
                    connector = Some(((anchor.0, bottom), (anchor.0, first.1)));
                } else if first.1 < anchor.1 {
                    connector = Some(((anchor.0, anchor.1), (anchor.0, first.1)));
                }
                (anchor.0 - width / 2.0, anchor.1, [radius; 4])
            }
            // A speech bubble whose square corner sits on the anchor; its round corners keep a
            // one-line bubble's radius as lines are added.
            DrawingKind::Comment => {
                let round = (line_height + 2.0 * PAD_Y * scale) / 2.0;
                (first.0, first.1 - height, [round, round, round, 0.0])
            }
            // The price tag sits on its anchor (bottom center) above the line.
            DrawingKind::PriceNote => {
                if let Some(end) = second {
                    connector = Some((first, end));
                }
                (first.0 - width / 2.0, first.1 - height, [radius; 4])
            }
            // The box's bottom-left corner is the second anchor; its tail grows out of the edge
            // facing the tip as part of one outline.
            _ => {
                let anchor = second.unwrap_or(first);
                let rect = [anchor.0, anchor.1 - height, width, height];
                let corner = CALLOUT_RADIUS * scale;
                let callout = second.and_then(|_| callout_tail(rect, corner, first, scale));
                tail = callout.map(|(_, [base_a, tip, base_b])| [tip, base_a, base_b]);
                outline = Some(callout_outline(rect, corner, callout));
                (rect[0], rect[1], [corner; 4])
            }
        };
        Some(AnnotationLayout {
            rect: [left, top, width, height],
            radii,
            text_x: left + pad_x,
            text_y: top + PAD_Y * scale + line_height / 2.0,
            size,
            text,
            lines,
            line_height,
            placeholder,
            connector,
            tail,
            outline,
        })
    }
}

/// A callout tail for a `tip` outside the box: on the edge facing it (the axis it overhangs
/// more), with its base centered on the tip's projection and kept clear of the rounded corners.
/// Returns the edge (0 top, 1 right, 2 bottom, 3 left, clockwise) and `[base_a, tip, base_b]`
/// in outline order. A tip inside the box has no tail.
fn callout_tail(
    rect: [f64; 4],
    corner: f64,
    tip: (f64, f64),
    scale: f64,
) -> Option<(usize, [(f64, f64); 3])> {
    let [left, top, width, height] = rect;
    let (right, bottom) = (left + width, top + height);
    let over_x = (left - tip.0).max(tip.0 - right).max(0.0);
    let over_y = (top - tip.1).max(tip.1 - bottom).max(0.0);
    if over_x <= 0.0 && over_y <= 0.0 {
        return None;
    }
    let side = if over_y >= over_x {
        if tip.1 < top { 0 } else { 2 }
    } else if tip.0 > right {
        1
    } else {
        3
    };
    let (start, end) = edge_runs(rect, corner)[side];
    let length = (end.0 - start.0).hypot(end.1 - start.1);
    let base = (CALLOUT_TAIL_BASE * scale).min(length);
    if base < 2.0 * scale {
        return None;
    }
    let direction = ((end.0 - start.0) / length, (end.1 - start.1) / length);
    let along = ((tip.0 - start.0) * direction.0 + (tip.1 - start.1) * direction.1)
        .clamp(base / 2.0, length - base / 2.0);
    let at = |offset: f64| {
        (
            start.0 + direction.0 * offset,
            start.1 + direction.1 * offset,
        )
    };
    Some((side, [at(along - base / 2.0), tip, at(along + base / 2.0)]))
}

/// The straight run of each box edge between its rounded corners, clockwise from the top.
fn edge_runs(rect: [f64; 4], corner: f64) -> [((f64, f64), (f64, f64)); 4] {
    let [left, top, width, height] = rect;
    let (right, bottom) = (left + width, top + height);
    [
        ((left + corner, top), (right - corner, top)),
        ((right, top + corner), (right, bottom - corner)),
        ((right - corner, bottom), (left + corner, bottom)),
        ((left, bottom - corner), (left, top + corner)),
    ]
}

/// The callout's closed outline: the rounded box clockwise with the tail spliced into its edge.
/// It starts and ends mid-way along the edge opposite the tail, so a stroke's two ends meet
/// flush on a straight run.
fn callout_outline(
    rect: [f64; 4],
    corner: f64,
    tail: Option<(usize, [(f64, f64); 3])>,
) -> Vec<(f64, f64)> {
    let runs = edge_runs(rect, corner);
    // Each corner arc follows its edge: top-right after the top edge, and so on clockwise.
    let [left, top, width, height] = rect;
    let (right, bottom) = (left + width, top + height);
    let centers = [
        (right - corner, top + corner),
        (right - corner, bottom - corner),
        (left + corner, bottom - corner),
        (left + corner, top + corner),
    ];
    let arc_steps = arc_segments(corner, std::f64::consts::FRAC_PI_2);
    let first = tail.map_or(0, |(side, _)| (side + 2) % 4);
    let middle = |edge: usize| {
        let (start, end) = runs[edge];
        ((start.0 + end.0) / 2.0, (start.1 + end.1) / 2.0)
    };
    let mut points = vec![middle(first)];
    for step in 0..4 {
        let edge = (first + step) % 4;
        if step > 0 {
            points.push(runs[edge].0);
        }
        if let Some((side, spliced)) = tail
            && side == edge
        {
            points.extend(spliced);
        }
        points.push(runs[edge].1);
        let start_angle = -std::f64::consts::FRAC_PI_2 + edge as f64 * std::f64::consts::FRAC_PI_2;
        for arc in 1..arc_steps {
            let angle =
                start_angle + std::f64::consts::FRAC_PI_2 * f64::from(arc) / f64::from(arc_steps);
            points.push((
                centers[edge].0 + corner * angle.cos(),
                centers[edge].1 + corner * angle.sin(),
            ));
        }
    }
    points.push(runs[first].0);
    points.push(middle(first));
    points
}

/// Price tags read in a semibold weight unless the drawing sets one.
pub(crate) fn annotation_text_weight(drawing: &Drawing) -> u16 {
    drawing
        .text_weight
        .unwrap_or(if drawing.kind == DrawingKind::PriceNote {
            600
        } else {
            400
        })
}
