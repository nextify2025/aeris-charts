//! Shared drawing-part vocabulary for B8 family tools.
//!
//! A family resolves one drawing's anchors into an ordered list of parts in the caller's
//! coordinate space (bitmap px for frame emission, media px for hit testing). Frame construction
//! lowers the same parts into `Prim`s (`frame/drawings.rs`) and precise hit testing tests the same
//! parts here, so the painted body, its decorations, and the interactive body cannot drift apart.
//! Families never emit `Prim`s and executors never see a drawing kind.

use aeris_charts_render::color::Color;
use aeris_charts_render::draw_list::LineStyle;
use aeris_charts_render::shape::{self, Point, Rect};

use super::{Drawing, DrawingTextHAlign, DrawingTextVAlign};
use crate::ChartEngine;

/// Gap between a measured anchor or edge and its stats box, in CSS px.
pub(crate) const STATS_GAP: f64 = 8.0;
/// Stats box padding (horizontal, vertical) in CSS px: the Long/Short Position chips' padding.
pub(crate) const STATS_PADDING: (f64, f64) = (6.0, 3.0);
/// Stats box background alpha over the drawing color.
pub(crate) const STATS_ALPHA: u8 = 224;

/// Stroke overrides of one part; `None` fields follow the drawing's own stroke.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct PartStroke {
    pub(crate) color: Option<Color>,
    /// Stroke width in CSS px.
    pub(crate) width: Option<f64>,
    pub(crate) style: Option<LineStyle>,
}

impl PartStroke {
    /// A thin decoration stroke (reference lines, arcs) in the drawing color.
    pub(crate) const fn decoration(width_css: f64, style: LineStyle) -> Self {
        Self {
            color: None,
            width: Some(width_css),
            style: Some(style),
        }
    }

    pub(crate) fn width_css(self, drawing: &Drawing) -> f64 {
        self.width.unwrap_or(drawing.width)
    }

    pub(crate) fn line_style(self, drawing: &Drawing) -> LineStyle {
        self.style.unwrap_or(drawing.style)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum DrawingPart {
    /// Anti-aliased open polyline over `points[start..end]` (a segment is two points). The
    /// `label_gap` stroke is the one a middle segment-layout text label splits.
    Stroke {
        start: usize,
        end: usize,
        stroke: PartStroke,
        label_gap: bool,
    },
    /// Crisp full-pixel horizontal line (the horizontal-line tool's primitive).
    HLine {
        y: f64,
        x0: f64,
        x1: f64,
        stroke: PartStroke,
    },
    /// Crisp full-pixel vertical line (the vertical-line tool's primitive).
    VLine {
        x: f64,
        y0: f64,
        y1: f64,
        stroke: PartStroke,
    },
    /// Solid region between two paired chains `points[upper..upper + count]` and
    /// `points[lower..lower + count]` (lowered to `Prim::BandFill`). The quads between paired
    /// points must not overlap; [`DrawingParts::fill_convex`] builds them for convex polygons.
    /// `color: None` fills with the drawing color. `hit` makes the region a body target.
    Fill {
        upper: usize,
        lower: usize,
        count: usize,
        color: Option<Color>,
        hit: bool,
    },
    /// Filled disc (`Prim::Circle`) with a caller-px radius; `color: None` is the drawing color.
    Disc {
        center: Point,
        radius: f64,
        color: Option<Color>,
    },
    /// Text block `labels[index]`.
    Label { index: usize },
}

/// A text block placed through the shared drawing label layout: `anchor` is where the box edge
/// selected by `h_align`/`v_align` sits, lines stack at 1.25 × size, and the optional box pads the
/// run. Rendering and hit testing both resolve the box through [`PartLabel::layout`].
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PartLabel {
    pub(crate) anchor: Point,
    pub(crate) h_align: DrawingTextHAlign,
    pub(crate) v_align: DrawingTextVAlign,
    pub(crate) lines: Vec<String>,
    /// Glyph size in caller px.
    pub(crate) size: f64,
    pub(crate) weight: u16,
    pub(crate) italic: bool,
    /// `None` follows the drawing's label color.
    pub(crate) color: Option<Color>,
    pub(crate) background: Option<Color>,
    pub(crate) border: Option<Color>,
    /// Horizontal and vertical box padding in caller px.
    pub(crate) padding: (f64, f64),
    /// The box is a body hit target.
    pub(crate) hit: bool,
}

/// Resolved label box in caller px.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct LabelLayout {
    pub(crate) rect: Rect,
    /// Left edge of every text line (left-aligned runs).
    pub(crate) text_x: f64,
    /// Vertical center of the first line.
    pub(crate) first_y: f64,
    pub(crate) line_height: f64,
}

impl PartLabel {
    pub(crate) fn layout(&self, measure: impl Fn(&str) -> f64) -> LabelLayout {
        let line_height = self.size * 1.25;
        let text_width = self
            .lines
            .iter()
            .map(|line| measure(line))
            .fold(0.0_f64, f64::max);
        let width = text_width + 2.0 * self.padding.0;
        let height = self.lines.len() as f64 * line_height + 2.0 * self.padding.1;
        let left = match self.h_align {
            DrawingTextHAlign::Left => self.anchor.0,
            DrawingTextHAlign::Center => self.anchor.0 - width / 2.0,
            DrawingTextHAlign::Right => self.anchor.0 - width,
        };
        let top = match self.v_align {
            DrawingTextVAlign::Top => self.anchor.1,
            DrawingTextVAlign::Middle => self.anchor.1 - height / 2.0,
            DrawingTextVAlign::Bottom => self.anchor.1 - height,
        };
        LabelLayout {
            rect: Rect {
                left,
                top,
                right: left + width,
                bottom: top + height,
            },
            text_x: left + self.padding.0,
            first_y: top + self.padding.1 + line_height / 2.0,
            line_height,
        }
    }
}

/// Everything a family needs to resolve one drawing into parts.
pub(crate) struct PartContext<'a> {
    pub(crate) engine: &'a ChartEngine,
    pub(crate) drawing: &'a Drawing,
    /// The drawing's anchors in caller px.
    pub(crate) px: &'a [Point],
    /// The drawing's pane in caller px (x from the pane's left edge, y from the chart top).
    pub(crate) pane: Rect,
    /// Caller px per CSS px for sizes (stroke widths, glyphs, gaps) and for y: the vertical pixel
    /// ratio at render, 1 at hit test.
    pub(crate) scale: f64,
    /// Caller px per media px for x: the horizontal pixel ratio at render (it differs from the
    /// vertical one when a fractional-DPR pane dimension rounds), 1 at hit test.
    pub(crate) x_scale: f64,
    /// The host's inline editor edits the drawing's `text` ([`ChartEngine::editing_drawing`];
    /// forced on when the engine resolves the editor's layout).
    pub(crate) text_editing: bool,
}

impl<'a> PartContext<'a> {
    /// The media-px context of `drawing` with anchors at media px `px` (hit testing and the
    /// hooks that resolve geometry outside frame construction). `None` for a stale pane.
    pub(crate) fn media(
        engine: &'a ChartEngine,
        drawing: &'a Drawing,
        px: &'a [Point],
    ) -> Option<Self> {
        let pane = engine.panes.get(drawing.pane_index)?;
        Some(Self {
            engine,
            drawing,
            px,
            pane: Rect {
                left: 0.0,
                top: pane.top,
                right: engine.pane_w,
                bottom: pane.top + pane.height,
            },
            scale: 1.0,
            x_scale: 1.0,
            text_editing: engine.editing_drawing() == Some(drawing.id),
        })
    }

    /// A logical/price point on this drawing's pane and price scale in caller px, the anchors'
    /// space: derived geometry (level lines, time zones, data-driven points) maps through here
    /// instead of rescaling media px itself. `None` when the scale cannot place it yet.
    pub(crate) fn point_px(&self, point: super::DrawingPoint) -> Option<Point> {
        let (x, y) = self.engine.drawing_point_px(self.drawing, point)?;
        Some((x * self.x_scale, y * self.scale))
    }

    /// A measurement (stats) box: `lines` at the stats glyph size in `text` on `background`,
    /// with the shared stats padding, a body target like every family label box.
    pub(crate) fn stats_label(
        &self,
        anchor: Point,
        (h_align, v_align): (DrawingTextHAlign, DrawingTextVAlign),
        lines: Vec<String>,
        background: Color,
        text: Color,
    ) -> PartLabel {
        PartLabel {
            anchor,
            h_align,
            v_align,
            lines,
            size: self.engine.drawing_stats_size() * self.scale,
            weight: 400,
            italic: false,
            color: Some(text),
            background: Some(background),
            border: None,
            padding: (STATS_PADDING.0 * self.scale, STATS_PADDING.1 * self.scale),
            hit: true,
        }
    }

    /// A drawing's stats box: [`Self::stats_label`] on `background`, by default the drawing's
    /// stroke color at [`STATS_ALPHA`], in the text color `text` picks against that box (the
    /// line tools' black or white by [`text_on`]; the ranges' `text_color`, else their contrast
    /// rule). The measurement boxes a tool paints beside its geometry share this look.
    pub(crate) fn stats_box(
        &self,
        anchor: Point,
        align: (DrawingTextHAlign, DrawingTextVAlign),
        lines: Vec<String>,
        background: Option<Color>,
        text: impl FnOnce(Color) -> Color,
    ) -> PartLabel {
        let background = background.unwrap_or_else(|| {
            let base = self.drawing.stroke_color();
            Color::rgba(base.r(), base.g(), base.b(), STATS_ALPHA)
        });
        self.stats_label(anchor, align, lines, background, text(background))
    }

    /// The drawing's own `text` as label lines ([`text_lines`]), with one empty line while the
    /// host's inline editor edits an empty text, so the box and the caret stay in place as the
    /// last character is deleted.
    pub(crate) fn text_lines(&self) -> Vec<String> {
        let mut lines = text_lines(&self.drawing.text);
        if lines.is_empty() && self.text_editing {
            lines.push(String::new());
        }
        lines
    }

    /// Whether region fills are body targets: only while the drawing is selected (the
    /// rectangle's convention), so an unselected tool never swallows chart drags inside its
    /// shading.
    pub(crate) fn fills_hit(&self) -> bool {
        self.engine.selected_drawing() == Some(self.drawing.id)
    }

    /// Whether the drawing is focused: hovered or selected (a fork-form note shows its box
    /// then; the frame rebuilds the drawing layer when such a drawing gains or loses focus).
    pub(crate) fn focused(&self) -> bool {
        let id = Some(self.drawing.id);
        self.engine.selected_drawing() == id || self.engine.hovered_drawing() == id
    }
}

/// The label that holds a drawing's own `text` (see [`DrawingParts::text_label`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TextPart {
    /// Index into [`DrawingParts::labels`].
    pub(crate) label: usize,
    /// The label's first line of `text`; earlier lines are engine text (a formatted price).
    pub(crate) first_line: usize,
}

/// A drawing's `text` split into label lines at `\n` (a trailing `\r` dropped), none when empty.
pub(crate) fn text_lines(text: &str) -> Vec<String> {
    if text.is_empty() {
        return Vec::new();
    }
    text.split('\n')
        .map(|line| line.trim_end_matches('\r').to_string())
        .collect()
}

/// Reusable part buffer: a shared point pool, the ordered parts, and their text blocks.
#[derive(Debug, Default)]
pub(crate) struct DrawingParts {
    pub(crate) points: Vec<Point>,
    pub(crate) items: Vec<DrawingPart>,
    pub(crate) labels: Vec<PartLabel>,
    /// The label holding the drawing's own `text`, which the host's inline editor edits in
    /// place; `None` when the drawing paints none.
    pub(crate) text: Option<TextPart>,
}

impl DrawingParts {
    pub(crate) fn stroke(&mut self, points: &[Point], stroke: PartStroke, label_gap: bool) {
        if points.len() < 2 {
            return;
        }
        let start = self.points.len();
        self.points.extend_from_slice(points);
        self.items.push(DrawingPart::Stroke {
            start,
            end: self.points.len(),
            stroke,
            label_gap,
        });
    }

    pub(crate) fn hline(&mut self, y: f64, x0: f64, x1: f64, stroke: PartStroke) {
        self.items.push(DrawingPart::HLine { y, x0, x1, stroke });
    }

    pub(crate) fn vline(&mut self, x: f64, y0: f64, y1: f64, stroke: PartStroke) {
        self.items.push(DrawingPart::VLine { x, y0, y1, stroke });
    }

    /// Region between two paired chains of equal length (see [`DrawingPart::Fill`]).
    pub(crate) fn fill(
        &mut self,
        upper: &[Point],
        lower: &[Point],
        color: Option<Color>,
        hit: bool,
    ) {
        let count = upper.len().min(lower.len());
        if count < 2 {
            return;
        }
        let first = self.points.len();
        self.points.extend_from_slice(&upper[..count]);
        self.points.extend_from_slice(&lower[..count]);
        self.items.push(DrawingPart::Fill {
            upper: first,
            lower: first + count,
            count,
            color,
            hit,
        });
    }

    /// Convex polygon region (triangles, quads, tessellated ellipses).
    pub(crate) fn fill_convex(&mut self, polygon: &[Point], color: Option<Color>, hit: bool) {
        let mut chains = Vec::with_capacity(polygon.len() + 2);
        let count = shape::convex_ribbon(polygon, &mut chains);
        let (upper, lower) = chains.split_at(count);
        self.fill(upper, lower, color, hit);
    }

    pub(crate) fn disc(&mut self, center: Point, radius: f64, color: Option<Color>) {
        self.items.push(DrawingPart::Disc {
            center,
            radius,
            color,
        });
    }

    pub(crate) fn label(&mut self, label: PartLabel) {
        if label.lines.is_empty() {
            return;
        }
        self.items.push(DrawingPart::Label {
            index: self.labels.len(),
        });
        self.labels.push(label);
    }

    /// A label whose lines from `first_line` on are the drawing's own `text`
    /// ([`PartContext::text_lines`]): the part the host's inline editor edits in place
    /// (`ChartEngine::drawing_text_edit_layout`). A drawing that paints one is text-editable;
    /// only its first is the editable one.
    pub(crate) fn text_label(&mut self, label: PartLabel, first_line: usize) {
        if label.lines.is_empty() {
            return;
        }
        if self.text.is_none() {
            self.text = Some(TextPart {
                label: self.labels.len(),
                first_line,
            });
        }
        self.label(label);
    }

    /// Stroke `a → b` with the drawing's configured end caps on the ends that are not extended
    /// (see [`DrawingParts::capped_polyline`]); a middle segment-layout text label splits it.
    pub(crate) fn capped_segment(
        &mut self,
        drawing: &Drawing,
        a: Point,
        b: Point,
        caps: (bool, bool),
        scale: f64,
    ) {
        self.capped_polyline(drawing, &[a, b], caps, (None, None), scale, true);
    }

    /// One line-end decoration at `endpoint` pointing away from `toward` (the core tools' cap
    /// geometry: a disc, or an arrowhead twice as long as its half-width).
    pub(crate) fn line_cap(
        &mut self,
        cap: crate::DrawingLineCap,
        endpoint: Point,
        toward: Point,
        width: f64,
    ) {
        // A cap needs a direction: none on a degenerate end.
        let Some(arrow) = arrow_cap_triangle(endpoint, toward, width) else {
            return;
        };
        match cap {
            crate::DrawingLineCap::Circle => self.disc(endpoint, cap_radius(width), None),
            crate::DrawingLineCap::Arrow => self.fill_convex(&arrow, None, true),
            crate::DrawingLineCap::None => {}
        }
    }

    /// Stroke an open polyline (segments, curves, multi-vertex lines) with the drawing's end caps
    /// on the ends `caps` selects: an arrow end trims the stroke back by one stroke width along
    /// the polyline so the butt end cannot poke out of the narrowing arrowhead, and the caps
    /// paint after the stroke, like every core line tool. Each cap points away from its `toward`
    /// point (a curve's tangent control point); `None`, or a point on the end itself, follows the
    /// first distinct vertex from that end. `label_gap` marks the stroke a middle segment-layout
    /// text label splits.
    pub(crate) fn capped_polyline(
        &mut self,
        drawing: &Drawing,
        points: &[Point],
        (cap_start, cap_end): (bool, bool),
        (toward_start, toward_end): (Option<Point>, Option<Point>),
        scale: f64,
        label_gap: bool,
    ) {
        let (Some(&first), Some(&last)) = (points.first(), points.last()) else {
            return;
        };
        let width = drawing.width * scale;
        let cap = |enabled: bool, cap: crate::DrawingLineCap| {
            if enabled {
                cap
            } else {
                crate::DrawingLineCap::None
            }
        };
        let (start_cap, end_cap) = (
            cap(cap_start, drawing.stroke_start),
            cap(cap_end, drawing.stroke_end),
        );
        let length: f64 = points
            .windows(2)
            .map(|pair| (pair[1].0 - pair[0].0).hypot(pair[1].1 - pair[0].1))
            .sum();
        let trims = |cap: crate::DrawingLineCap| {
            cap == crate::DrawingLineCap::Arrow && length > width * 2.0
        };
        // Trim in the shared point pool, so a capped stroke allocates nothing per build.
        let start = self.points.len();
        self.points.extend_from_slice(points);
        if trims(start_cap) {
            trim_polyline_front(&mut self.points, start, width);
        }
        if trims(end_cap) {
            self.points[start..].reverse();
            trim_polyline_front(&mut self.points, start, width);
            self.points[start..].reverse();
        }
        if self.points.len() - start >= 2 {
            self.items.push(DrawingPart::Stroke {
                start,
                end: self.points.len(),
                stroke: PartStroke::default(),
                label_gap,
            });
        } else {
            self.points.truncate(start);
        }
        let start_toward = cap_toward(first, toward_start, points.iter());
        let end_toward = cap_toward(last, toward_end, points.iter().rev());
        self.line_cap(start_cap, first, start_toward, width);
        self.line_cap(end_cap, last, end_toward, width);
    }

    /// Precise body test of every part at caller point `p`. `tolerance` is the pointer slack
    /// (hit profile) beyond each stroke's half width; `measure` measures one label line at the
    /// label's glyph size.
    pub(crate) fn hit(
        &self,
        drawing: &Drawing,
        p: Point,
        tolerance: f64,
        measure: impl Fn(&PartLabel, &str) -> f64,
    ) -> bool {
        self.items.iter().any(|part| match *part {
            DrawingPart::Stroke {
                start, end, stroke, ..
            } => {
                shape::distance_to_polyline(p, &self.points[start..end])
                    <= stroke.width_css(drawing) / 2.0 + tolerance
            }
            DrawingPart::HLine { y, x0, x1, stroke } => {
                (p.1 - y).abs() <= stroke.width_css(drawing) / 2.0 + tolerance
                    && p.0 >= x0.min(x1) - tolerance
                    && p.0 <= x0.max(x1) + tolerance
            }
            DrawingPart::VLine { x, y0, y1, stroke } => {
                (p.0 - x).abs() <= stroke.width_css(drawing) / 2.0 + tolerance
                    && p.1 >= y0.min(y1) - tolerance
                    && p.1 <= y0.max(y1) + tolerance
            }
            DrawingPart::Fill {
                upper,
                lower,
                count,
                hit,
                ..
            } => {
                hit && shape::point_in_ribbon(
                    p,
                    &self.points[upper..upper + count],
                    &self.points[lower..lower + count],
                )
            }
            DrawingPart::Disc { center, radius, .. } => {
                (p.0 - center.0).hypot(p.1 - center.1) <= radius + tolerance
            }
            DrawingPart::Label { index } => {
                let label = &self.labels[index];
                label.hit && label.layout(|line| measure(label, line)).rect.contains(p)
            }
        })
    }
}

/// Black or white label text against `background` (the Long/Short Position chips' rule).
pub(crate) fn text_on(background: Color) -> Color {
    if background.luminance() > 175.0 {
        Color::rgb(0, 0, 0)
    } else {
        Color::rgb(255, 255, 255)
    }
}

/// Radius of a line cap in the caller px of `width` (the stroke width): a disc's radius and an
/// arrowhead's half-width, the arrowhead twice as long (the core tools' cap geometry).
pub(crate) fn cap_radius(width: f64) -> f64 {
    (width * 1.75).max(3.0)
}

/// The arrowhead of a line cap at `endpoint` pointing away from `toward`, for a stroke `width`
/// wide (caller px): its tip on the endpoint and its base two [`cap_radius`] back, one radius to
/// either side. `None` when `toward` coincides with the endpoint (no direction). The frame's core
/// line caps and the parts layer share it, so every cap has one geometry.
pub(crate) fn arrow_cap_triangle(endpoint: Point, toward: Point, width: f64) -> Option<[Point; 3]> {
    let dx = toward.0 - endpoint.0;
    let dy = toward.1 - endpoint.1;
    let distance = dx.hypot(dy);
    if distance <= f64::EPSILON {
        return None;
    }
    let (ux, uy) = (dx / distance, dy / distance);
    let radius = cap_radius(width);
    let base = (
        endpoint.0 + ux * radius * 2.0,
        endpoint.1 + uy * radius * 2.0,
    );
    let side = (-uy * radius, ux * radius);
    Some([
        endpoint,
        (base.0 + side.0, base.1 + side.1),
        (base.0 - side.0, base.1 - side.1),
    ])
}

/// The point a cap at `end` points away from: the hint when it is off the end, else the first
/// vertex of `rest` distinct from the end.
fn cap_toward<'a>(
    end: Point,
    hint: Option<Point>,
    mut rest: impl Iterator<Item = &'a Point>,
) -> Point {
    hint.filter(|&point| point != end)
        .or_else(|| rest.find(|&&point| point != end).copied())
        .unwrap_or(end)
}

/// Remove `distance` of arc length from the start of the polyline `points[from..]` (callers
/// keep it longer).
fn trim_polyline_front(points: &mut Vec<Point>, from: usize, distance: f64) {
    let mut remaining = distance;
    for index in from..points.len().saturating_sub(1) {
        let (a, b) = (points[index], points[index + 1]);
        let length = (b.0 - a.0).hypot(b.1 - a.1);
        if length > remaining {
            let t = remaining / length;
            points[index] = (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
            points.drain(from..index);
            return;
        }
        remaining -= length;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DrawingKind, DrawingLineCap};

    fn drawing() -> Drawing {
        Drawing::new(1, DrawingKind::TrendLine, 0, Vec::new())
    }

    #[test]
    fn label_layout_aligns_the_box_edge_to_the_anchor() {
        let mut label = PartLabel {
            anchor: (100.0, 50.0),
            h_align: DrawingTextHAlign::Left,
            v_align: DrawingTextVAlign::Top,
            lines: vec!["abcd".into(), "ab".into()],
            size: 10.0,
            weight: 400,
            italic: false,
            color: None,
            background: None,
            border: None,
            padding: (4.0, 2.0),
            hit: true,
        };
        let measure = |line: &str| line.len() as f64 * 6.0;
        let layout = label.layout(measure);
        assert_eq!(layout.rect.left, 100.0);
        assert_eq!(layout.rect.top, 50.0);
        assert_eq!(layout.rect.right, 100.0 + 24.0 + 8.0);
        assert_eq!(layout.rect.bottom, 50.0 + 25.0 + 4.0);
        assert_eq!(layout.first_y, 50.0 + 2.0 + 6.25);
        label.h_align = DrawingTextHAlign::Right;
        label.v_align = DrawingTextVAlign::Middle;
        let layout = label.layout(measure);
        assert_eq!(layout.rect.right, 100.0);
        assert!((layout.rect.top + layout.rect.bottom - 100.0).abs() < 1e-9);
    }

    #[test]
    fn parts_hit_strokes_regions_discs_and_boxed_labels() {
        let mut drawing = drawing();
        drawing.width = 2.0;
        let mut parts = DrawingParts::default();
        parts.stroke(&[(0.0, 0.0), (100.0, 0.0)], PartStroke::default(), true);
        parts.hline(
            50.0,
            0.0,
            100.0,
            PartStroke::decoration(1.0, LineStyle::Dashed),
        );
        parts.vline(200.0, 0.0, 100.0, PartStroke::default());
        parts.fill_convex(&[(300.0, 0.0), (340.0, 0.0), (320.0, 30.0)], None, true);
        parts.fill_convex(&[(400.0, 0.0), (440.0, 0.0), (420.0, 30.0)], None, false);
        parts.disc((500.0, 50.0), 5.0, None);
        parts.label(PartLabel {
            anchor: (600.0, 50.0),
            h_align: DrawingTextHAlign::Center,
            v_align: DrawingTextVAlign::Middle,
            lines: vec!["box".into()],
            size: 10.0,
            weight: 400,
            italic: false,
            color: None,
            background: None,
            border: None,
            padding: (4.0, 4.0),
            hit: true,
        });
        let measure = |_: &PartLabel, line: &str| line.len() as f64 * 6.0;
        let hit = |x: f64, y: f64| parts.hit(&drawing, (x, y), 3.0, measure);
        assert!(
            hit(50.0, 3.9) && !hit(50.0, 4.1),
            "half width 1 + tolerance 3"
        );
        assert!(
            hit(50.0, 53.4) && !hit(50.0, 53.6),
            "decoration half width 0.5"
        );
        assert!(hit(203.9, 50.0) && !hit(204.1, 50.0));
        assert!(hit(320.0, 10.0), "hittable region");
        assert!(!hit(420.0, 10.0), "paint-only region");
        assert!(hit(507.9, 50.0) && !hit(508.1, 50.0));
        assert!(hit(600.0, 50.0) && hit(612.0, 50.0) && !hit(614.0, 50.0));
        assert!(!hit(700.0, 50.0));
    }

    #[test]
    fn capped_segments_trim_arrow_ends_and_skip_extended_ends() {
        let mut drawing = drawing();
        drawing.width = 2.0;
        drawing.stroke_end = DrawingLineCap::Arrow;
        drawing.stroke_start = DrawingLineCap::Circle;
        let mut parts = DrawingParts::default();
        parts.capped_segment(&drawing, (0.0, 0.0), (100.0, 0.0), (true, true), 1.0);
        let DrawingPart::Stroke { start, end, .. } = parts.items[0] else {
            panic!("stroke first");
        };
        assert_eq!(parts.points[start..end], [(0.0, 0.0), (98.0, 0.0)]);
        assert!(matches!(parts.items[1], DrawingPart::Disc { radius, .. } if radius == 3.5));
        let DrawingPart::Fill { upper, count, .. } = parts.items[2] else {
            panic!("arrowhead fill");
        };
        assert_eq!(
            parts.points[upper],
            (100.0, 0.0),
            "the arrow tip stays on the anchor"
        );
        assert_eq!(count, 3);

        let mut parts = DrawingParts::default();
        parts.capped_segment(&drawing, (0.0, 0.0), (100.0, 0.0), (false, false), 1.0);
        assert_eq!(parts.items.len(), 1, "extended ends carry no caps");
    }

    #[test]
    fn capped_polylines_trim_arrows_along_the_stroke_and_follow_tangents() {
        let mut drawing = Drawing::new(1, DrawingKind::TrendLine, 0, Vec::new());
        drawing.width = 2.0;
        drawing.stroke_start = DrawingLineCap::Arrow;
        drawing.stroke_end = DrawingLineCap::Circle;
        let points = [(0.0, 0.0), (1.0, 0.0), (1.0, 50.0), (60.0, 50.0)];
        let mut parts = DrawingParts::default();
        parts.capped_polyline(&drawing, &points, (true, true), (None, None), 1.0, false);
        let DrawingPart::Stroke { start, end, .. } = parts.items[0] else {
            panic!("stroke first");
        };
        // Two px of arc length leave the first segment and continue down the second.
        assert_eq!(parts.points[start], (1.0, 1.0));
        assert_eq!(
            parts.points[end - 1],
            (60.0, 50.0),
            "a disc end is not trimmed"
        );
        let DrawingPart::Fill { upper, lower, .. } = parts.items[1] else {
            panic!("arrowhead");
        };
        assert_eq!(parts.points[upper], (0.0, 0.0), "the tip stays on the end");
        // The arrowhead opens toward the first distinct vertex (along +x).
        assert!(parts.points[lower + 1].0 > 0.0);
        assert!(
            matches!(parts.items[2], DrawingPart::Disc { center, .. } if center == (60.0, 50.0))
        );

        // A tangent hint turns the arrowhead: pointing along -y instead.
        let mut parts = DrawingParts::default();
        parts.capped_polyline(
            &drawing,
            &points,
            (true, false),
            (Some((0.0, 10.0)), None),
            1.0,
            false,
        );
        let DrawingPart::Fill { lower, count, .. } = parts.items[1] else {
            panic!("arrowhead");
        };
        assert!(parts.points[lower..lower + count]
            .iter()
            .all(|point| point.1 >= 0.0));
        assert_eq!(parts.items.len(), 2, "the end cap is switched off");
    }
}
