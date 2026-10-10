//! Drawing-tool frame emission (model in drawings.rs): each pane's committed drawings in
//! z-order, the selected drawing's anchor handles, and the interactive-creation preview.
//!
//! Coordinates follow the frame build's conventions (frame/mod.rs): x is pane-local media px
//! scaled by the exact horizontal ratio (the trailing `translate_prims_x` shifts everything
//! when a left axis reserves space), y is chart-top-relative media px scaled by the vertical
//! ratio — the same space the price-line/series geometry uses, so a drawing's prims land
//! exactly on its converted anchors. Standalone drawing text uses `Prim::Text`; trend labels
//! use the backend-neutral `Prim::RotatedText` contract so every executor receives the same
//! aligned anchor and segment-normalized angle.

use aeris_charts_render::color::Color;
use aeris_charts_render::draw_list::{IRect, LineStyle, LineType, Prim, TextAlign};
use aeris_charts_render::line::{
    crisp_span, push_clipped_stroke, push_line_stroke, push_styled_stroke,
};
use std::fmt::Write;

use super::{POSITION_ENTRY, PRIMARY};
use crate::drawings::handles::{
    ANCHOR_BORDER_WIDTH, ANCHOR_RADIUS, DrawingHandle, HandleShape, handle_set,
};
use crate::drawings::kinds::patterns_elliott_cycles::{self, PatternLayer};
use crate::drawings::kinds::projection_annotations::{self, ForkGlyph, fork_glyph_parts};
use crate::drawings::kinds::{channels, fibonacci, lines, pitchforks_gann, shapes};
use crate::drawings::{
    Drawing, DrawingBodyGeometry, DrawingGeometryOptions, DrawingHandleMode, DrawingId,
    DrawingKind, DrawingPart, DrawingParts, DrawingTextHAlign, DrawingTextLayout,
    FibonacciArcGeometry, POINT_LABEL_GAP_CSS, PartContext, PositionGeometry, PositionZone,
    TEXT_CHROME_PAD, TEXT_PAD, TREND_TEXT_PLACEHOLDER, TextBlock, TimeLevelGeometry,
    annotation_text_weight, arc_segments, arrow_marker_icon, builtin_icon, cap_radius,
    clear_label_center, closed_outline, curve_clip, ellipse_outline, gann_arc_segments,
    level_band_pairs, path_arrow_points, resolve_drawing_geometry,
};
use crate::{ChartEngine, FibonacciLabelVAlign};
use aeris_charts_core::model::plot_list::PlotValueIndex;

/// Drawing handles (`push_handle`): a theme-derived fill inside a primary-token border, round or
/// square. The fill radius plus border (`ANCHOR_RADIUS`, `ANCHOR_BORDER_WIDTH`) makes a 12 px
/// handle, larger than the series selection anchors (series_geometry.rs) since these are drag
/// targets. The border floors to whole device pixels so it stays a light ring: 1 px at 1x, 3 px
/// at 2x. Square handles keep slightly rounded corners.
const ANCHOR_SQUARE_RADIUS: f64 = 2.0;
const ANCHOR_BORDER: Color = PRIMARY;
const POSITION_ENTRY_WIDTH_CSS: f64 = 0.5;
const POSITION_ZONE_ALPHA: u8 = 70;
/// Progress must read as the emphasized portion of either semantic side on its own. Keep this
/// above the base-zone alpha instead of relying on a second lower-alpha pass to become visible
/// only through accidental compositing.
const POSITION_PROGRESS_ALPHA: u8 = 96;
/// The hover ring's dimmed variant of the focus border (the public reference shows the same border at
/// roughly half strength until the drawing is actually selected).
const HOVER_BORDER: Color = Color(PRIMARY.0 & 0xFFFF_FF00 | 0x73);
const TREND_TEXT_PLACEHOLDER_ALPHA: u8 = 0x99;

fn point_on_segment(a: (f64, f64), b: (f64, f64), t: f64) -> (f64, f64) {
    (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
}

/// A core segment stroke; a dashed or dotted one reaches executors as solid dash runs clipped to
/// `pane` ([`push_styled_stroke`]).
/// Chord tolerance of the highlighter's tube outline, in device px.
const HIGHLIGHTER_TUBE_TOLERANCE: f64 = 0.25;

fn push_segment(
    a: (f64, f64),
    b: (f64, f64),
    (stroke, pane): ((f32, LineStyle, Color), aeris_charts_render::shape::Rect),
    out: &mut Vec<Prim>,
    points: &mut Vec<[f32; 2]>,
) {
    if (a.0 - b.0).abs() <= f64::EPSILON && (a.1 - b.1).abs() <= f64::EPSILON {
        return;
    }
    push_styled_stroke(out, points, &[a, b], LineType::Simple, stroke, pane);
}

/// A Fibonacci tool's trend line through its anchors `px` (`kinds::fibonacci::trend_line`), in
/// the drawing's color and [`fibonacci::trend_stroke`], lowered like a core segment.
fn push_fibonacci_trend_line(
    drawing: &Drawing,
    px: &[(f64, f64)],
    vpr: f64,
    pane: aeris_charts_render::shape::Rect,
    out: &mut Vec<Prim>,
    points: &mut Vec<[f32; 2]>,
) {
    if let Some((path, count)) = fibonacci::trend_line(drawing, px) {
        let (width, style) = fibonacci::trend_stroke(drawing);
        push_styled_stroke(
            out,
            points,
            &path[..count],
            LineType::Simple,
            ((width * vpr) as f32, style, drawing.stroke_color()),
            pane,
        );
    }
}

/// Whether the box `center ± (rx, ry)` meets `clip` (a closed curve inside that box can paint).
fn box_meets(center: (f64, f64), rx: f64, ry: f64, clip: aeris_charts_render::shape::Rect) -> bool {
    clip.intersects(&aeris_charts_render::shape::Rect {
        left: center.0 - rx,
        top: center.1 - ry,
        right: center.0 + rx,
        bottom: center.1 + ry,
    })
}

/// A closed polygon's outline as one run from mid-edge ([`closed_outline`]), in
/// [`push_segment`]'s styling.
fn push_closed_outline(
    vertices: &[(f64, f64)],
    (stroke, pane): ((f32, LineStyle, Color), aeris_charts_render::shape::Rect),
    out: &mut Vec<Prim>,
    points: &mut Vec<[f32; 2]>,
) {
    let mut outline = Vec::new();
    closed_outline(vertices, &mut outline);
    if outline.len() >= 3 {
        push_styled_stroke(out, points, &outline, LineType::Simple, stroke, pane);
    }
}

/// A region fill: the paired chains `ribbon` (each `count` long) as one `Prim::BandFill`.
fn push_ribbon(
    ribbon: &[(f64, f64)],
    count: usize,
    fill: Color,
    out: &mut Vec<Prim>,
    points: &mut Vec<[f32; 2]>,
) {
    if count == 0 {
        return;
    }
    let upper_first = points.len() as u32;
    points.extend(ribbon.iter().map(|&(x, y)| [x as f32, y as f32]));
    out.push(Prim::BandFill {
        upper_first,
        lower_first: upper_first + count as u32,
        point_count: count as u32,
        line_type: LineType::Simple,
        fill,
    });
}

/// A crisp dashed axis-aligned outline through two opposite corners.
fn push_dashed_outline(a: (f64, f64), b: (f64, f64), color: Color, vpr: f64, out: &mut Vec<Prim>) {
    let width = vpr.round().max(1.0) as i32;
    let (left, right) = (a.0.min(b.0).round() as i32, a.0.max(b.0).round() as i32);
    let (top, bottom) = (a.1.min(b.1).round() as i32, a.1.max(b.1).round() as i32);
    for y in [top, bottom] {
        out.push(Prim::HLine {
            y,
            x0: left,
            x1: right,
            width,
            style: LineStyle::Dashed,
            color,
        });
    }
    for x in [left, right] {
        out.push(Prim::VLine {
            x,
            y0: top,
            y1: bottom,
            width,
            style: LineStyle::Dashed,
            color,
        });
    }
}

/// A line end decoration at `endpoint` for a stroke arriving from `toward`. `line_width` is the
/// drawing's CSS width and `width` the same stroke in device px.
#[allow(clippy::too_many_arguments)] // the cap, its segment, the stroke, and the output pools
fn push_drawing_cap(
    cap: crate::DrawingLineCap,
    endpoint: (f64, f64),
    toward: (f64, f64),
    line_width: f64,
    vpr: f64,
    color: Color,
    out: &mut Vec<Prim>,
    points: &mut Vec<[f32; 2]>,
) {
    // A cap needs a direction: none on a degenerate end.
    if cap == crate::DrawingLineCap::None
        || (toward.0 - endpoint.0).hypot(toward.1 - endpoint.1) <= f64::EPSILON
    {
        return;
    }
    let width = line_width * vpr;
    match cap {
        crate::DrawingLineCap::Circle => out.push(Prim::Circle {
            cx: endpoint.0 as f32,
            cy: endpoint.1 as f32,
            radius: cap_radius(width) as f32,
            fill: color,
            stroke_width: 0.0,
            stroke: color,
        }),
        // The Path tool's open chevron (`path_arrow_points`), stroked like the line.
        crate::DrawingLineCap::Arrow => {
            let Some(wings) = path_arrow_points(&[toward, endpoint], line_width, vpr) else {
                return;
            };
            let first_point = points.len() as u32;
            points.extend(wings.map(|(x, y)| [x as f32, y as f32]));
            out.push(Prim::Polyline {
                first_point,
                point_count: 3,
                width: width as f32,
                style: LineStyle::Solid,
                line_type: LineType::Simple,
                color,
            });
        }
        crate::DrawingLineCap::None => {}
    }
}

#[cfg(test)]
mod trend_label_tests {
    use super::*;
    use crate::drawings::{DrawingPoint, DrawingTextVAlign};

    #[test]
    fn all_nine_trend_label_positions_follow_the_segment() {
        let mut drawing = Drawing::new(
            1,
            DrawingKind::TrendLine,
            0,
            vec![
                DrawingPoint {
                    logical: 0.0,
                    price: 0.0,
                },
                DrawingPoint {
                    logical: 1.0,
                    price: 1.0,
                },
            ],
        );
        for line in [
            [(10.0, 50.0), (90.0, 50.0)],
            [(10.0, 80.0), (90.0, 20.0)],
            [(50.0, 90.0), (50.0, 10.0)],
            [(90.0, 20.0), (10.0, 80.0)],
        ] as [[(f64, f64); 2]; 4]
        {
            let mut start = line[0];
            let mut end = line[1];
            if end.0 < start.0 || ((end.0 - start.0).abs() <= f64::EPSILON && end.1 > start.1) {
                std::mem::swap(&mut start, &mut end);
            }
            let dx: f64 = end.0 - start.0;
            let dy: f64 = end.1 - start.1;
            let length = dx.hypot(dy);
            let (ux, uy) = (dx / length, dy / length);
            for (h_align, expected_distance) in [
                (DrawingTextHAlign::Left, 4.0),
                (DrawingTextHAlign::Center, length / 2.0),
                (DrawingTextHAlign::Right, length - 4.0),
            ] {
                drawing.text_h_align = h_align;
                for (v_align, expected_normal) in [
                    (DrawingTextVAlign::Top, 11.2),
                    (DrawingTextVAlign::Middle, 0.0),
                    (DrawingTextVAlign::Bottom, -11.2),
                ] {
                    drawing.text_v_align = v_align;
                    let (x, y, align, angle) = ChartEngine::drawing_text_placement(
                        &drawing, &line, 100.0, 0.0, 100.0, 12.0, 1.0,
                    );
                    assert_eq!(align, h_align);
                    let (from_x, from_y) = (x - start.0, y - start.1);
                    assert!((from_x * ux + from_y * uy - expected_distance).abs() < 1e-9);
                    assert!((from_x * uy - from_y * ux - expected_normal).abs() < 1e-9);
                    assert!(
                        (-std::f64::consts::FRAC_PI_2..=std::f64::consts::FRAC_PI_2)
                            .contains(&angle)
                    );
                }
            }
        }

        drawing.text_h_align = DrawingTextHAlign::Right;
        drawing.text_v_align = DrawingTextVAlign::Middle;
        let line = [(10.0, 80.0), (90.0, 20.0)];
        let reversed = [line[1], line[0]];
        let (x, y, _, angle) =
            ChartEngine::drawing_text_placement(&drawing, &reversed, 100.0, 0.0, 100.0, 12.0, 1.0);
        assert!((x - 86.8).abs() < 1e-9);
        assert!((y - 22.4).abs() < 1e-9);
        assert!((angle - (-0.6_f64).atan2(0.8)).abs() < 1e-9);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PositionRunSide {
    Reward,
    Risk,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct PositionRunProgress {
    pub(super) start: crate::drawings::DrawingPoint,
    pub(super) point: crate::drawings::DrawingPoint,
    pub(super) side: PositionRunSide,
    pub(super) closed: bool,
}

/// Where one engine-painted text caret bar sits: the run's anchor `(x, y)` in bitmap px, the
/// caret's offset `local_x` along the run, half the bar's height, and the run's clockwise angle.
struct CaretBar {
    x: f64,
    y: f64,
    local_x: f64,
    half_height: f64,
    angle: f64,
}

/// Emit one caret bar: a crisp device-grid rect when unrotated (one thickness wherever the caret
/// moves), a two-point solid polyline when the run is rotated.
fn push_caret_bar(
    out: &mut Vec<Prim>,
    points: &mut Vec<[f32; 2]>,
    bar: CaretBar,
    vpr: f64,
    color: Color,
) {
    let CaretBar {
        x,
        y,
        local_x,
        half_height,
        angle,
    } = bar;
    if angle.abs() <= 1e-6 {
        // An unrotated caret snaps to the device grid like every other crisp 1px rule, so
        // it keeps one thickness wherever the caret moves.
        let width = vpr.round().max(1.0) as i32;
        out.push(Prim::Rect {
            rect: IRect {
                x: (x + local_x).round() as i32,
                y: (y - half_height).round() as i32,
                w: width,
                h: ((y + half_height).round() - (y - half_height).round()).max(1.0) as i32,
            },
            color,
        });
        return;
    }
    let (sin, cos) = angle.sin_cos();
    let at = |local_y: f64| {
        [
            (x + cos * local_x - sin * local_y) as f32,
            (y + sin * local_x + cos * local_y) as f32,
        ]
    };
    let first_point = points.len() as u32;
    points.extend([at(-half_height), at(half_height)]);
    out.push(Prim::Polyline {
        first_point,
        point_count: 2,
        width: vpr.max(1.0) as f32,
        style: LineStyle::Solid,
        line_type: LineType::Simple,
        color,
    });
}

impl ChartEngine {
    fn drawing_level_style(style: &str) -> LineStyle {
        match style {
            "dotted" | "sparse_dotted" => LineStyle::Dotted,
            "dashed" | "large_dashed" => LineStyle::Dashed,
            _ => LineStyle::Solid,
        }
    }

    fn drawing_level_fill(level: &crate::DrawingLevel, fallback: Color) -> Color {
        let base = Color::parse_css(&level.color).unwrap_or(fallback);
        level
            .fill_color
            .as_deref()
            .and_then(Color::parse_css)
            .unwrap_or(Color::rgba(base.r(), base.g(), base.b(), 35))
    }

    fn drawing_level_price_at(&self, drawing: &Drawing, y: f64, vpr: f64) -> Option<f64> {
        let scale = self.drawing_scale_for(drawing.pane_index, drawing.price_scale)?;
        let price = scale.coordinate_to_price(
            y / vpr,
            self.drawing_scale_base_for(drawing.pane_index, drawing.price_scale),
        );
        price.is_finite().then_some(price)
    }

    pub(crate) fn drawing_level_label(
        &self,
        drawing: &Drawing,
        value: f64,
        price: Option<f64>,
    ) -> Option<String> {
        let mut label = String::new();
        if drawing.level_show_values {
            write!(label, "{value}").expect("formatting a String cannot fail");
        }
        if drawing.level_show_percents {
            if !label.is_empty() {
                label.push_str(" · ");
            }
            write!(label, "{:.1}%", value * 100.0).expect("formatting a String cannot fail");
        }
        if drawing.level_show_prices
            && let Some(price) = price.filter(|price| price.is_finite())
        {
            if !label.is_empty() {
                label.push_str(" · ");
            }
            label.push_str(&self.format_drawing_price(drawing, price));
        }
        (!label.is_empty()).then_some(label)
    }

    fn drawing_level_align(drawing: &Drawing) -> TextAlign {
        match drawing.level_label_align.as_str() {
            "left" => TextAlign::Left,
            "center" => TextAlign::Center,
            _ => TextAlign::Right,
        }
    }

    /// The label of the Fibonacci arm's level `value` whose line runs `(a, b)` (caller px at
    /// `vpr` caller px per CSS px, in a pane `pane_w` caller px wide): upstream's top row (see
    /// [`Self::top_level_label_anchor`]), below the line's `level_label_align` point, or beside
    /// the line's end (`label_v_align`). Frame and hit testing (`vpr` 1) resolve it alike.
    pub(crate) fn fibonacci_level_label(
        &self,
        drawing: &Drawing,
        (a, b): ((f64, f64), (f64, f64)),
        value: f64,
        vpr: f64,
        pane_w: f64,
    ) -> Option<fibonacci::LevelLabel> {
        let ((x0, _), (x1, y1)) = (a, b);
        let price = self.drawing_level_price_at(drawing, y1, vpr);
        let text = self.drawing_level_label(drawing, value, price)?;
        let label_x = match drawing.level_label_align.as_str() {
            "left" => x0,
            "center" => (x0 + x1) / 2.0,
            _ => x1,
        };
        let (x, y, align) = match fibonacci::label_v_align(drawing) {
            FibonacciLabelVAlign::Top => {
                self.top_level_label_anchor(drawing, (a, b), label_x, &text, vpr, pane_w)
            }
            FibonacciLabelVAlign::Bottom => {
                (label_x, y1 + 8.0 * vpr, Self::drawing_level_align(drawing))
            }
            FibonacciLabelVAlign::Middle => fibonacci::middle_label_anchor(
                drawing,
                (a, b),
                (drawing.extend_left, drawing.extend_right),
                fibonacci::label_gap(vpr),
            ),
        };
        Some(fibonacci::LevelLabel { x, y, text, align })
    }

    /// Upstream's top-row level label placement, which never sits on its own line: above a
    /// horizontal level at its `level_label_align` point; past a sloped level's end, beside it
    /// (every parallel stops at the same x extent, so the label never crosses a neighbor), or
    /// clear of the line at its center. An end on the pane edge (a Gann fan's ray, an extended
    /// channel) keeps the label inside the pane, lifted above the line under its run.
    fn top_level_label_anchor(
        &self,
        drawing: &Drawing,
        ((x0, y0), (x1, y1)): ((f64, f64), (f64, f64)),
        label_x: f64,
        text: &str,
        vpr: f64,
        pane_w: f64,
    ) -> (f64, f64, TextAlign) {
        let layout = &self.options.get().layout;
        let size = layout.font_size * vpr;
        let gap = POINT_LABEL_GAP_CSS * vpr;
        let line_y = |x: f64| {
            if (x1 - x0).abs() <= f64::EPSILON {
                y0.min(y1)
            } else {
                y0 + (y1 - y0) * (x - x0) / (x1 - x0)
            }
        };
        if (y1 - y0).abs() <= vpr {
            return (
                label_x,
                y0.min(y1) - gap - size * 0.6,
                Self::drawing_level_align(drawing),
            );
        }
        let width = || {
            self.measure_text_run(
                text,
                size,
                &layout.font_family,
                drawing.text_weight.unwrap_or(400),
                drawing.text_italic,
            )
        };
        let on_edge = |x: f64| x <= 0.5 || x >= pane_w - 0.5;
        let above = |left: f64, right: f64| line_y(left).min(line_y(right)) - gap - size * 0.6;
        let (left_end, right_end) = if x0 <= x1 {
            ((x0, y0), (x1, y1))
        } else {
            ((x1, y1), (x0, y0))
        };
        match drawing.level_label_align.as_str() {
            "left" if on_edge(left_end.0) => {
                let x = left_end.0 + gap;
                (x, above(x, x + width()), TextAlign::Left)
            }
            "left" => (left_end.0 - gap, left_end.1, TextAlign::Right),
            "center" => {
                let (x, y) = clear_label_center(
                    (label_x, line_y(label_x)),
                    &[(x0, y0), (x1, y1)],
                    width(),
                    size * 1.2,
                    gap,
                    (0.0, -1.0),
                );
                (x, y, TextAlign::Center)
            }
            _ if on_edge(right_end.0) => {
                let x = right_end.0 - gap;
                (x, above(x - width(), x), TextAlign::Right)
            }
            _ => (right_end.0 + gap, right_end.1, TextAlign::Left),
        }
    }

    /// The label of the time-level arm's level `value` on its vertical line at `x` (caller px at
    /// `vpr`): upstream's 4 CSS px beside the line by `level_label_align`, at the pane's top,
    /// middle or bottom (`label_v_align`).
    pub(crate) fn time_level_label(
        &self,
        drawing: &Drawing,
        time: TimeLevelGeometry,
        x: f64,
        value: f64,
        vpr: f64,
    ) -> Option<fibonacci::LevelLabel> {
        let text = self.drawing_level_label(drawing, value, None)?;
        let offset = match drawing.level_label_align.as_str() {
            "left" => 4.0 * vpr,
            "right" => -4.0 * vpr,
            _ => 0.0,
        };
        let y = match fibonacci::label_v_align(drawing) {
            FibonacciLabelVAlign::Top => time.pane_top + 14.0 * vpr,
            FibonacciLabelVAlign::Middle => (time.pane_top + time.pane_bottom) / 2.0,
            FibonacciLabelVAlign::Bottom => time.pane_bottom - 14.0 * vpr,
        };
        Some(fibonacci::LevelLabel {
            x: x + offset,
            y,
            text,
            align: Self::drawing_level_align(drawing),
        })
    }

    /// The label of the Fibonacci-arc arm's level `value` (caller px at `vpr`): upstream's, where
    /// the level's radius runs closest to vertical ([`FibonacciArcGeometry::label_t`]), outside
    /// the arc and never on its stroke, centered.
    pub(crate) fn arc_level_label(
        &self,
        drawing: &Drawing,
        arcs: FibonacciArcGeometry,
        value: f64,
        vpr: f64,
    ) -> Option<fibonacci::LevelLabel> {
        let level = drawing.level_value(value);
        let label_t = arcs.label_t();
        let (x, y) = arcs.point(level, label_t);
        let price = self.drawing_level_price_at(drawing, y, vpr);
        let text = self.drawing_level_label(drawing, value, price)?;
        let layout = &self.options.get().layout;
        let size = layout.font_size * vpr;
        let width = self.measure_text_run(
            &text,
            size,
            &layout.font_family,
            drawing.text_weight.unwrap_or(400),
            drawing.text_italic,
        );
        let radial = (x - arcs.center.0, y - arcs.center.1);
        let length = radial.0.hypot(radial.1).max(f64::EPSILON);
        let (x, y) = clear_label_center(
            (x, y),
            &[
                arcs.point(level, (label_t - 0.01).max(0.0)),
                arcs.point(level, (label_t + 0.01).min(1.0)),
            ],
            width,
            size * 1.2,
            POINT_LABEL_GAP_CSS * vpr,
            (radial.0 / length, radial.1 / length),
        );
        Some(fibonacci::LevelLabel {
            x,
            y,
            text,
            align: TextAlign::Center,
        })
    }

    /// Paint a level arm's `label` in `color` at the layout font (`vpr` caller px per CSS px).
    fn push_level_label(
        &self,
        drawing: &Drawing,
        label: fibonacci::LevelLabel,
        color: Color,
        vpr: f64,
        out: &mut Vec<Prim>,
    ) {
        let layout = &self.options.get().layout;
        out.push(Prim::Text {
            x: label.x as f32,
            y: label.y as f32,
            text: label.text,
            color,
            size: (layout.font_size * vpr) as f32,
            family: layout.font_family.clone(),
            align: label.align,
            weight: drawing.text_weight.unwrap_or(400),
            italic: drawing.text_italic,
        });
    }
    fn drawing_frame_text<'a>(&self, drawing: &'a Drawing) -> Option<(&'a str, bool)> {
        if drawing.kind == DrawingKind::PriceLabel {
            return None;
        }
        if !drawing.text.is_empty() {
            return Some((drawing.display_text(), false));
        }
        (drawing.kind == DrawingKind::TrendLine
            && self.hovered_text == Some(drawing.id)
            && self.editing_drawing() != Some(drawing.id))
        .then_some((TREND_TEXT_PLACEHOLDER, true))
    }

    /// Width source for the middle-line cutout. Hover reserves the full prompt; once editing
    /// begins (on any drawing with a segment-following label), an empty value uses the editor's
    /// one-em caret opening and measured text expands it.
    fn drawing_frame_gap_text<'a>(&self, drawing: &'a Drawing) -> Option<&'a str> {
        if self.editing_drawing() == Some(drawing.id) {
            return Some(drawing.display_text());
        }
        if !drawing.text.is_empty() {
            return Some(drawing.display_text());
        }
        (drawing.kind == DrawingKind::TrendLine && self.hovered_text == Some(drawing.id))
            .then_some(TREND_TEXT_PLACEHOLDER)
    }

    fn measure_drawing_frame_text(&self, drawing: &Drawing, text: &str, size: f64) -> f64 {
        let layout = &self.options.get().layout;
        self.measure_text_run(
            text,
            size,
            &layout.font_family,
            drawing.text_weight.unwrap_or(400),
            drawing.text_italic,
        )
    }

    #[cfg(test)]
    pub(crate) fn build_drawings_frame_reference(
        &self,
        pane_index: usize,
        pane_w_px: i32,
        hpr: f64,
        vpr: f64,
        out: &mut Vec<Prim>,
        points: &mut Vec<[f32; 2]>,
    ) {
        for drawing in &self.drawings {
            if drawing.pane_index != pane_index
                || !drawing.visible
                || !drawing.interval_visibility.allows(self.drawing_interval)
                || !self.drawing_viewport_candidate_reference(drawing)
            {
                continue;
            }
            let Some(px) = self.drawing_render_px(drawing) else {
                continue;
            };
            let px = px
                .into_iter()
                .map(|(x, y)| (x * hpr, y * vpr))
                .collect::<Vec<_>>();
            self.build_drawing_prims(drawing, &px, pane_w_px, vpr, out, points);
            self.build_drawing_text(drawing, &px, pane_w_px, vpr, out, points);
            self.build_drawing_text_caret(drawing, &px, pane_w_px, vpr, out, points);
            self.build_drawing_labels(drawing, &px, vpr, out);
        }
    }

    /// Segmented committed build for retained reassembly: stable z-order committed drawings,
    /// recording each emitted drawing's prim/point range in `parts` (stable z-order) and the
    /// trailing preview (brush + pending) start in `preview_start` (prim, point). Previews
    /// always trail committed so assembly can place them topmost among chart content.
    /// Ordering.rs reassembles these parts idle-below / active-above without rebuilding
    /// geometry on hover/selection. Drawings on stale panes draw nowhere.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn build_drawings_frame_segmented(
        &self,
        pane_index: usize,
        pane_w_px: i32,
        hpr: f64,
        vpr: f64,
        out: &mut Vec<Prim>,
        points: &mut Vec<[f32; 2]>,
        parts: &mut Vec<super::RetainedDrawingPart>,
        preview_start: &mut (usize, usize),
    ) {
        parts.clear();
        if self.drawing_runtime.borrow().pane_count(pane_index) <= 20 {
            for drawing in &self.drawings {
                if drawing.pane_index != pane_index
                    || !drawing.visible
                    || !drawing.interval_visibility.allows(self.drawing_interval)
                {
                    continue;
                }
                let px = if drawing.kind == DrawingKind::RegressionTrend {
                    self.drawing_coordinate_key(drawing).and_then(|key| {
                        let mut runtime = self.drawing_runtime.borrow_mut();
                        self.drawing_px_cached(drawing, &mut runtime, key)
                            .map(<[(f64, f64)]>::to_vec)
                    })
                } else {
                    self.drawing_render_px(drawing)
                };
                let Some(px) = px else {
                    continue;
                };
                let px = px
                    .into_iter()
                    .map(|(x, y)| (x * hpr, y * vpr))
                    .collect::<Vec<_>>();
                let prim_start = out.len();
                let point_start = points.len();
                self.build_drawing_prims(drawing, &px, pane_w_px, vpr, out, points);
                self.build_drawing_text(drawing, &px, pane_w_px, vpr, out, points);
                self.build_drawing_text_caret(drawing, &px, pane_w_px, vpr, out, points);
                self.build_drawing_labels(drawing, &px, vpr, out);
                parts.push(super::RetainedDrawingPart {
                    id: drawing.id,
                    prim_start,
                    prim_end: out.len(),
                    point_start,
                    point_end: points.len(),
                });
            }
        } else {
            let candidates = self.take_drawing_candidates(pane_index, None);
            let mut runtime = self.drawing_runtime.borrow_mut();
            for &id in &candidates {
                let Some(position) = runtime.position(id) else {
                    continue;
                };
                let Some(drawing) = self.drawings.get(position) else {
                    continue;
                };
                if !drawing.visible || !drawing.interval_visibility.allows(self.drawing_interval) {
                    continue;
                }
                let Some(key) = self.drawing_coordinate_key(drawing) else {
                    continue;
                };
                let Some(px) = self.drawing_px_cached(drawing, &mut runtime, key) else {
                    continue;
                };
                let px = px
                    .iter()
                    .map(|&(x, y)| (x * hpr, y * vpr))
                    .collect::<Vec<_>>();
                let prim_start = out.len();
                let point_start = points.len();
                self.build_drawing_prims(drawing, &px, pane_w_px, vpr, out, points);
                self.build_drawing_text(drawing, &px, pane_w_px, vpr, out, points);
                self.build_drawing_text_caret(drawing, &px, pane_w_px, vpr, out, points);
                self.build_drawing_labels(drawing, &px, vpr, out);
                parts.push(super::RetainedDrawingPart {
                    id: drawing.id,
                    prim_start,
                    prim_end: out.len(),
                    point_start,
                    point_end: points.len(),
                });
                runtime.record_visible();
            }
            drop(runtime);
            self.recycle_drawing_candidates(candidates);
        }
        *preview_start = (out.len(), points.len());
        // Live brush stroke: the decimated points so far paint as the same smooth curve the
        // commit will store, so what the user sees while dragging is what they get.
        if let Some(capture) = self.brush_capture()
            && capture.pane_index == pane_index
            && capture.points.len() >= 2
        {
            let px: Option<Vec<(f64, f64)>> = capture
                .points
                .iter()
                .map(|&point| self.drawing_to_px(pane_index, point))
                .collect();
            if let Some(px) = px {
                let px: Vec<(f64, f64)> = px.into_iter().map(|(x, y)| (x * hpr, y * vpr)).collect();
                self.build_drawing_prims(&capture.options, &px, pane_w_px, vpr, out, points);
            }
        }
        // Interactive creation: committed anchors plus the preview point render as a tentative
        // drawing, with handles on the committed anchors (the reference rectangle-drawing-tool's
        // PreviewRectangle — same geometry, shown while placing).
        if let Some(pending) = self.pending_drawing()
            && pending.drawing.pane_index == pane_index
        {
            let mut anchors = pending.drawing.points.clone();
            let is_sequence = pending.drawing.kind.spec().placement.is_sequence();
            if let Some(preview) = pending.preview
                && (is_sequence || anchors.len() < pending.drawing.kind.anchor_count())
            {
                anchors.push(preview);
                // The preview already keeps a linked coordinate shared, as the result will.
                let last = anchors.len() - 1;
                pending
                    .drawing
                    .kind
                    .spec()
                    .anchor_link
                    .apply(&mut anchors, last);
            }
            // Families that resolve partial anchors preview from the second anchor on, and
            // so do the patterns and Elliott waves: their legs, labels, ratios and fills
            // resolve from any prefix of their anchors (owner decision P5).
            let kind = pending.drawing.kind;
            let partial = anchors.len() >= 2
                && (kind
                    .spec()
                    .family
                    .is_some_and(|family| family.partial_preview)
                    || kind.vertex_labels().is_some());
            let ready = if is_sequence {
                anchors.len() >= pending.drawing.kind.anchor_count()
            } else {
                anchors.len() == pending.drawing.kind.anchor_count() || partial
            };
            if ready {
                // Semantic statistics (measure direction, labels, angles) and derived
                // geometry read the full placed-plus-preview anchor set, exactly as the
                // commit will store it.
                let mut preview_drawing = pending.drawing.clone();
                preview_drawing.points.clone_from(&anchors);
                // A tool placed ends first (an arc, a curve) previews the anchors its clicks
                // will store, in stored order (start, the points on the curve, end); its
                // handles stay on the placed clicks.
                let through = shapes::places_through(kind) && anchors.len() == kind.anchor_count();
                if through {
                    preview_drawing.points =
                        shapes::placement_anchors(kind, &anchors).unwrap_or_default();
                }
                if preview_drawing.kind == DrawingKind::AnchoredText {
                    // Anchored text paints at its screen position, which follows the
                    // pointer while placing exactly as the commit will set it.
                    if let Some((sx, sy)) = anchors.last().and_then(|&point| {
                        self.anchored_text_screen_position(
                            pane_index,
                            preview_drawing.price_scale,
                            point,
                        )
                    }) {
                        preview_drawing.screen_x = sx;
                        preview_drawing.screen_y = sy;
                    }
                }
                if let Some(media) = self.drawing_render_px(&preview_drawing) {
                    let px: Vec<(f64, f64)> =
                        media.iter().map(|&(x, y)| (x * hpr, y * vpr)).collect();
                    if preview_drawing.kind.spec().handles == DrawingHandleMode::RectangleBounds
                        && let Some(fill) = preview_drawing.preview_fill_color.clone()
                    {
                        preview_drawing.fill_color = Some(fill);
                    }
                    self.build_drawing_prims(&preview_drawing, &px, pane_w_px, vpr, out, points);
                    if pending.drawing.kind.spec().handles == DrawingHandleMode::RectangleBounds {
                        // the public reference shows all eight anchors while the rectangle is being
                        // drawn (committed corner + live preview corner), not only after
                        // the commit.
                        let handles = handle_set(DrawingHandleMode::RectangleBounds, &px);
                        build_handles(&handles, vpr, self.anchor_fill(), out);
                    } else if through {
                        // Discs on the clicks placed so far, which the stored anchors reorder
                        // (ends first) until the last click.
                        let placed = pending
                            .drawing
                            .points
                            .iter()
                            .map(|&point| {
                                self.drawing_point_px(&pending.drawing, point)
                                    .map(|(x, y)| (x * hpr, y * vpr))
                            })
                            .collect::<Option<Vec<_>>>()
                            .unwrap_or_default();
                        build_anchor_handles(&placed, vpr, self.anchor_fill(), out);
                    } else {
                        // The placed anchors' handles, where the family paints them on the
                        // previewed geometry; derived handles wait for the committed
                        // drawing, and derived render points (a regression's fitted band)
                        // never become handles.
                        let placed = pending.drawing.points.len();
                        let anchor_px = &media[..preview_drawing.points.len().min(media.len())];
                        let handles: Vec<(f64, f64)> = self
                            .drawing_handle_set(&preview_drawing, anchor_px)
                            .into_iter()
                            .filter(|handle| {
                                matches!(
                                    handle.part,
                                    crate::DrawingDragPart::Anchor(index) if index < placed
                                )
                            })
                            .map(|handle| (handle.point.0 * hpr, handle.point.1 * vpr))
                            .collect();
                        build_anchor_handles(&handles, vpr, self.anchor_fill(), out);
                    }
                }
            } else if !anchors.is_empty() {
                // Fewer anchors than the tool needs: a two-anchor kind before the preview
                // resolves shows its first anchor as a handle alone; a tool of three or more
                // anchors between clicks also runs a guide polyline in the drawing's stroke
                // through its placed anchors to the pointer, so every click leaves visible
                // ink.
                let px: Option<Vec<(f64, f64)>> = anchors
                    .iter()
                    .map(|&point| {
                        self.drawing_to_px_for(pane_index, pending.drawing.price_scale, point)
                            .map(|(x, y)| (x * hpr, y * vpr))
                    })
                    .collect();
                if let (Some(px), Some(pane)) = (px, self.panes.get(pane_index)) {
                    if px.len() >= 2 && pending.drawing.kind == DrawingKind::BarsPattern {
                        // The bars pattern's first two anchors bound its source window, which
                        // shows as the dashed outline the selection uses.
                        push_dashed_outline(px[0], px[1], pending.drawing.stroke_color(), vpr, out);
                    } else if px.len() >= 2 {
                        let clip = aeris_charts_render::shape::Rect {
                            left: 0.0,
                            top: pane.top * vpr,
                            right: f64::from(pane_w_px),
                            bottom: (pane.top + pane.height) * vpr,
                        };
                        push_clipped_stroke(
                            out,
                            points,
                            &px,
                            clip,
                            (
                                (pending.drawing.width * vpr) as f32,
                                pending.drawing.style,
                                pending.drawing.stroke_color(),
                            ),
                            &mut Vec::new(),
                        );
                    }
                    let placed = pending.drawing.points.len().clamp(1, px.len());
                    build_anchor_handles(&px[..placed], vpr, self.anchor_fill(), out);
                }
            }
        }
        // The transient Shift-click measure paints the date-and-price range geometry without
        // handles; it is never a committed drawing.
        if let Some(session) = self.measure_session()
            && session.drawing.pane_index == pane_index
            && let Some(px) = self.drawing_px(&session.drawing)
        {
            let px: Vec<(f64, f64)> = px.into_iter().map(|(x, y)| (x * hpr, y * vpr)).collect();
            self.build_drawing_prims(&session.drawing, &px, pane_w_px, vpr, out, points);
        }
    }

    /// The selected/hovered drawing's converted bitmap-px anchor points (never its derived render
    /// points, such as a regression's fitted band, which are no handles), or `None` when the id is
    /// stale, on another pane, or off-screen.
    fn overlay_drawing_px(
        &self,
        pane_index: usize,
        id: DrawingId,
        hpr: f64,
        vpr: f64,
    ) -> Option<Vec<(f64, f64)>> {
        let drawing = self.drawing(id)?;
        if drawing.pane_index != pane_index
            || !drawing.visible
            || !drawing.interval_visibility.allows(self.drawing_interval)
        {
            return None;
        }
        let key = self.drawing_coordinate_key(drawing)?;
        let mut runtime = self.drawing_runtime.borrow_mut();
        let px = self.drawing_px_cached(drawing, &mut runtime, key)?;
        Some(
            px.iter()
                .take(drawing.points.len())
                .map(|&(x, y)| (x * hpr, y * vpr))
                .collect::<Vec<_>>(),
        )
    }

    /// Selection chrome is retained with the overlay, so selection-only changes do not
    /// invalidate or reconstruct unrelated drawing geometry. The text tool gets no anchor
    /// handles (the public reference: text has no drag points) — its selection affordance is the focus
    /// border alone. That border STAYS painted while the host typing-mode editor is open
    /// (the wrap is borderless; only the caret overlays), so entering/leaving edit cannot
    /// shift the outline.
    pub(super) fn build_selected_drawing_handles_frame(
        &self,
        pane_index: usize,
        pane_w_px: i32,
        hpr: f64,
        vpr: f64,
        out: &mut Vec<Prim>,
    ) {
        let Some(id) = self.selected_drawing else {
            return;
        };
        let Some(drawing) = self.drawing(id) else {
            return;
        };
        // A drawing without handles (the text tool, anchored text, a fork-form note, comment,
        // or price note) shows the focus border alone; box annotations show their handles.
        // (`requests_text_editor` only says placement opens the editor, so other tools that
        // request it keep their handles.)
        if self.drawing_handle_mode(drawing) == DrawingHandleMode::None {
            let Some(px) = self.overlay_drawing_px(pane_index, id, hpr, vpr) else {
                return;
            };
            self.push_text_chrome(drawing, &px, pane_w_px, vpr, ANCHOR_BORDER, out);
            return;
        }
        let Some(px) = self.overlay_drawing_px(pane_index, id, 1.0, 1.0) else {
            return;
        };
        if drawing.kind == DrawingKind::BarsPattern
            && !drawing.bars_pattern.is_empty()
            && px.len() >= 2
        {
            // The copied bars paint at the target; the source window the two range handles
            // edit only exists as this selection outline, so the handles sit on its corners.
            let corner = |(x, y): (f64, f64)| (x * hpr, y * vpr);
            push_dashed_outline(
                corner(px[0]),
                corner(px[1]),
                drawing.stroke_color(),
                vpr,
                out,
            );
        }
        let mut handles = self.drawing_handle_set(drawing, &px);
        for handle in &mut handles {
            handle.point = (handle.point.0 * hpr, handle.point.1 * vpr);
        }
        if self.drawing_handle_mode(drawing) == DrawingHandleMode::IconBox {
            let bitmap: Vec<_> = px.iter().map(|&(x, y)| (x * hpr, y * vpr)).collect();
            if let Some(icon) = self.icon_box(drawing, &bitmap, vpr) {
                // A crisp frame on the icon's whole-pixel square, its corner handles on the
                // frame's corners.
                let edge = icon.1.round().max(1.0);
                let (left, top) = (
                    (icon.0.0 - edge / 2.0).round(),
                    (icon.0.1 - edge / 2.0).round(),
                );
                out.push(Prim::RectFrame {
                    rect: IRect {
                        x: left as i32,
                        y: top as i32,
                        w: edge as i32,
                        h: edge as i32,
                    },
                    border: vpr.round().max(1.0) as i32,
                    color: ANCHOR_BORDER,
                });
                let corners =
                    ChartEngine::icon_box_corners(((left + edge / 2.0, top + edge / 2.0), edge));
                for handle in &mut handles {
                    if let crate::DrawingDragPart::Anchor(corner) = handle.part
                        && let Some(&point) = corners.get(corner)
                    {
                        handle.point = point;
                    }
                }
            }
        }
        build_handles(&handles, vpr, self.anchor_fill(), out);
    }

    /// The hovered text drawing's focus border at hover opacity (the public reference's hover ring):
    /// the same chrome box as selection, dimmed. Suppressed while the drawing is selected
    /// (the full-strength border already paints, including during typing mode).
    pub(super) fn build_hovered_text_frame(
        &self,
        pane_index: usize,
        pane_w_px: i32,
        hpr: f64,
        vpr: f64,
        out: &mut Vec<Prim>,
    ) {
        let Some(id) = self.hovered_text else {
            return;
        };
        if self.selected_drawing == Some(id) {
            return;
        }
        let Some(drawing) = self.drawing(id) else {
            return;
        };
        // The drawings whose selection is the focus border alone (the text tool, anchored text,
        // a fork-form note, comment, or price note) show it dimmed on hover.
        if self.drawing_handle_mode(drawing) != DrawingHandleMode::None {
            return;
        }
        let Some(px) = self.overlay_drawing_px(pane_index, id, hpr, vpr) else {
            return;
        };
        self.push_text_chrome(drawing, &px, pane_w_px, vpr, HOVER_BORDER, out);
    }

    /// One drawing's geometry prims at bitmap-px anchors `px`.
    fn build_drawing_prims(
        &self,
        drawing: &Drawing,
        px: &[(f64, f64)],
        pane_w_px: i32,
        vpr: f64,
        out: &mut Vec<Prim>,
        points: &mut Vec<[f32; 2]>,
    ) {
        if drawing.kind == DrawingKind::BarsPattern && !drawing.bars_pattern.is_empty() {
            self.build_bars_pattern_prims(drawing, pane_w_px, vpr, out, points);
            return;
        }
        if matches!(
            drawing.kind,
            DrawingKind::FixedRangeVolumeProfile
                | DrawingKind::AnchoredVolumeProfile
                | DrawingKind::AnchoredVwap
        ) && drawing.profile.is_some()
        {
            self.build_profile_drawing_prims(drawing, px, pane_w_px, vpr, out, points);
            return;
        }
        if drawing.kind.spec().family.is_some() {
            self.build_family_prims(drawing, px, pane_w_px, vpr, out, points);
            return;
        }
        let color = drawing.stroke_color();
        let crisp_width = (drawing.width * vpr).round().max(1.0) as i32;
        let Some(pane) = self.panes.get(drawing.pane_index) else {
            return;
        };
        // Dashed and dotted strokes lower to solid dash runs clipped to the pane.
        let stroke = (
            ((drawing.width * vpr) as f32, drawing.style, color),
            aeris_charts_render::shape::Rect {
                left: 0.0,
                top: pane.top * vpr,
                right: f64::from(pane_w_px),
                bottom: (pane.top + pane.height) * vpr,
            },
        );
        // A channel's 1 CSS px dashed middle line, in its `middle_color` or the stroke color.
        let channel_middle_stroke = |middle_color: Option<Color>| {
            (
                (
                    (channels::MIDDLE_WIDTH * vpr) as f32,
                    LineStyle::Dashed,
                    middle_color.unwrap_or(color),
                ),
                stroke.1,
            )
        };
        let Some(geometry) = resolve_drawing_geometry(
            drawing.kind,
            px,
            f64::from(pane_w_px),
            pane.top * vpr,
            pane.height * vpr,
            DrawingGeometryOptions::for_drawing(drawing, vpr),
        ) else {
            return;
        };
        // Upstream's construction guides paint under the body (a pitchfork's base, the trend
        // legs of a Fibonacci extension or trend-based time without a stored block), one leg at
        // a time through the stroke path, so each dashed leg starts drawn at its own anchor.
        if fibonacci::draws_guides(drawing) {
            for guide in geometry.guides.into_iter().flatten() {
                let style = if guide.dashed {
                    LineStyle::Dashed
                } else {
                    drawing.style
                };
                let leg = ((stroke.0.0, style, color), stroke.1);
                push_segment(guide.a, guide.b, leg, out, points);
            }
        }
        match geometry.body {
            DrawingBodyGeometry::Annotation => {
                self.build_annotation_prims(drawing, px, color, vpr, out, points);
            }
            DrawingBodyGeometry::Segment { a, b } => {
                if let Some(context) = self
                    .frame_part_context(drawing, px, pane_w_px, vpr)
                    .filter(|_| lines::fork_presentation(drawing))
                {
                    // The `line` block's presentation strokes and caps the resolved segment
                    // through the parts layer, then adds its decorations and stats box.
                    self.push_parts(&context, pane_w_px, vpr, out, points, |c, parts| {
                        lines::upstream_line_parts(c, Some((a, b)), parts);
                    });
                } else {
                    let label_gap = self.segment_label_gap(drawing, px, pane_w_px, vpr, a, b);
                    if let Some((gap_start, gap_end)) = label_gap {
                        push_segment(a, point_on_segment(a, b, gap_start), stroke, out, points);
                        push_segment(point_on_segment(a, b, gap_end), b, stroke, out, points);
                    } else {
                        push_segment(a, b, stroke, out, points);
                    }
                    push_drawing_cap(
                        drawing.stroke_start,
                        a,
                        b,
                        drawing.width,
                        vpr,
                        color,
                        out,
                        points,
                    );
                    push_drawing_cap(
                        drawing.stroke_end,
                        b,
                        a,
                        drawing.width,
                        vpr,
                        color,
                        out,
                        points,
                    );
                    // Without a `line` block the trend angle and the info line keep upstream's
                    // dotted reference and arc, and statistics card.
                    match drawing.kind {
                        DrawingKind::TrendAngle => {
                            self.build_trend_angle_prims(drawing, px, color, vpr, out, points);
                        }
                        DrawingKind::InfoLine => {
                            self.build_info_line_prims(drawing, px, pane_w_px, vpr, out, points);
                        }
                        _ => {}
                    }
                }
            }
            DrawingBodyGeometry::Horizontal { y, x0, x1 } => {
                let x0 = (x0.round() as i32).clamp(0, pane_w_px);
                let x1 = (x1.round() as i32).clamp(0, pane_w_px);
                if x0 != x1 {
                    out.push(Prim::HLine {
                        y: y.round() as i32,
                        x0: x0.min(x1),
                        x1: x0.max(x1),
                        width: crisp_width,
                        style: drawing.style,
                        color,
                    });
                }
            }
            DrawingBodyGeometry::Vertical { x, y0, y1 } => {
                out.push(Prim::VLine {
                    x: x.round() as i32,
                    y0: y0.round().max(0.0) as i32,
                    y1: y1.round().max(0.0) as i32,
                    width: crisp_width,
                    style: drawing.style,
                    color,
                });
            }
            DrawingBodyGeometry::Cross {
                x,
                y,
                pane_w,
                pane_top,
                pane_bottom,
            } => {
                out.push(Prim::HLine {
                    y: y.round() as i32,
                    x0: 0,
                    x1: pane_w.round() as i32,
                    width: crisp_width,
                    style: drawing.style,
                    color,
                });
                out.push(Prim::VLine {
                    x: x.round() as i32,
                    y0: pane_top.round().max(0.0) as i32,
                    y1: pane_bottom.round().max(0.0) as i32,
                    width: crisp_width,
                    style: drawing.style,
                    color,
                });
                if let Some(context) = self
                    .frame_part_context(drawing, px, pane_w_px, vpr)
                    .filter(|_| lines::fork_presentation(drawing))
                {
                    self.push_parts(&context, pane_w_px, vpr, out, points, |c, parts| {
                        lines::upstream_line_parts(c, None, parts);
                    });
                }
            }
            DrawingBodyGeometry::Channel { first, second } => {
                // The fill and the middle line pair the lines' ends by side, so a disjoint whose
                // second line runs opposite to its first fills its whole quad, and lines that
                // cross fill two lobes meeting at the crossing on every executor; a concave
                // disjoint quad fills through its exact ribbon instead.
                let paired = channels::paired_second(first, second);
                if drawing.fill_enabled {
                    let fill = drawing
                        .fill_color
                        .as_deref()
                        .and_then(Color::parse_css)
                        .unwrap_or(Color::rgba(color.r(), color.g(), color.b(), 51));
                    let upper_first = points.len() as u32;
                    let point_count = match channels::channel_fill_ribbon(first, paired) {
                        Some(ribbon) => {
                            points.extend(ribbon.iter().map(|&(x, y)| [x as f32, y as f32]));
                            (ribbon.len() / 2) as u32
                        }
                        None => {
                            points.extend(first.map(|(x, y)| [x as f32, y as f32]));
                            points.extend(paired.map(|(x, y)| [x as f32, y as f32]));
                            2
                        }
                    };
                    if point_count >= 2 {
                        out.push(Prim::BandFill {
                            upper_first,
                            lower_first: upper_first + point_count,
                            point_count,
                            line_type: LineType::Simple,
                            fill,
                        });
                    }
                }
                if let Some((middle, middle_color)) =
                    channels::channel_middle(drawing, first, paired)
                {
                    let middle_stroke = channel_middle_stroke(middle_color);
                    push_segment(middle[0], middle[1], middle_stroke, out, points);
                }
                push_segment(first[0], first[1], stroke, out, points);
                push_segment(second[0], second[1], stroke, out, points);
            }
            DrawingBodyGeometry::Regression {
                center,
                upper,
                lower,
            } => {
                let band = channels::regression_band(drawing);
                if let Some((first, second)) = channels::regression_zone(band, center, upper, lower)
                    .filter(|_| drawing.fill_enabled)
                {
                    let fill = drawing
                        .fill_color
                        .as_deref()
                        .and_then(Color::parse_css)
                        .unwrap_or(Color::rgba(color.r(), color.g(), color.b(), 35));
                    let upper_first = points.len() as u32;
                    points.extend(first.map(|(x, y)| [x as f32, y as f32]));
                    let lower_first = points.len() as u32;
                    points.extend(second.map(|(x, y)| [x as f32, y as f32]));
                    out.push(Prim::BandFill {
                        upper_first,
                        lower_first,
                        point_count: 2,
                        line_type: LineType::Simple,
                        fill,
                    });
                }
                // A side that is switched off has no line.
                for (enabled, segment) in [(band.lower, lower), (band.upper, upper)] {
                    if enabled.is_some() {
                        push_segment(segment[0], segment[1], stroke, out, points);
                    }
                }
                // `middle_line` draws the centre as the thin dashed middle line.
                let center_stroke =
                    channels::middle_line(drawing).map_or(stroke, channel_middle_stroke);
                push_segment(center[0], center[1], center_stroke, out, points);
                if let Some(context) = self
                    .frame_part_context(drawing, px, pane_w_px, vpr)
                    .filter(|_| band.show_pearsons)
                {
                    self.push_parts(&context, pane_w_px, vpr, out, points, |c, parts| {
                        channels::regression_parts(c, parts);
                    });
                }
            }
            DrawingBodyGeometry::RegressionWindow { a, b } => {
                let ((width, _, color), pane) = stroke;
                push_segment(a, b, ((width, LineStyle::Dashed, color), pane), out, points);
            }
            DrawingBodyGeometry::Fibonacci(fib) => {
                for (prior, level) in level_band_pairs(drawing, &drawing.levels, false) {
                    let (prior_a, prior_b) =
                        self.drawing_fibonacci_level_segment(drawing, fib, prior, vpr);
                    let ((x0, y0), (x1, y1)) =
                        self.drawing_fibonacci_level_segment(drawing, fib, level.value, vpr);
                    let fill = Self::drawing_level_fill(level, color);
                    if (y0 - y1).abs() <= f64::EPSILON {
                        out.push(Prim::Rect {
                            rect: IRect {
                                x: x0.min(x1).round() as i32,
                                y: y0.min(prior_a.1).round() as i32,
                                w: (x1 - x0).abs().round().max(1.0) as i32,
                                h: (y0 - prior_a.1).abs().round().max(1.0) as i32,
                            },
                            color: fill,
                        });
                    } else {
                        let upper_first = points.len() as u32;
                        points.extend([
                            [prior_a.0 as f32, prior_a.1 as f32],
                            [prior_b.0 as f32, prior_b.1 as f32],
                        ]);
                        let lower_first = points.len() as u32;
                        points.extend([[x0 as f32, y0 as f32], [x1 as f32, y1 as f32]]);
                        out.push(Prim::BandFill {
                            upper_first,
                            lower_first,
                            point_count: 2,
                            line_type: LineType::Simple,
                            fill,
                        });
                    }
                }
                if fibonacci::draws_grid(drawing) {
                    let (top, bottom) = (stroke.1.top, stroke.1.bottom);
                    let right = f64::from(pane_w_px);
                    for level in drawing.levels.iter().filter(|level| level.visible) {
                        let [(h0, h1), (v0, v1)] = fib.grid_lines(drawing.level_value(level.value));
                        if (top..=bottom).contains(&h0.1) {
                            out.push(Prim::HLine {
                                y: h0.1.round() as i32,
                                x0: h0.0.min(h1.0).clamp(0.0, right).round() as i32,
                                x1: h0.0.max(h1.0).clamp(0.0, right).round() as i32,
                                width: crisp_width,
                                style: drawing.style,
                                color,
                            });
                        }
                        if (0.0..=right).contains(&v0.0) {
                            out.push(Prim::VLine {
                                x: v0.0.round() as i32,
                                y0: v0.1.min(v1.1).clamp(top, bottom).round() as i32,
                                y1: v0.1.max(v1.1).clamp(top, bottom).round() as i32,
                                width: crisp_width,
                                style: drawing.style,
                                color,
                            });
                        }
                    }
                }
                push_fibonacci_trend_line(drawing, px, vpr, stroke.1, out, points);
                for level in &drawing.levels {
                    if !level.visible {
                        continue;
                    }
                    let ((x0, y0), (x1, y1)) =
                        self.drawing_fibonacci_level_segment(drawing, fib, level.value, vpr);
                    let level_color = Color::parse_css(&level.color).unwrap_or(color);
                    let style = match level.style.as_str() {
                        "dotted" | "sparse_dotted" => LineStyle::Dotted,
                        "dashed" | "large_dashed" => LineStyle::Dashed,
                        _ => LineStyle::Solid,
                    };
                    if (y0 - y1).abs() <= f64::EPSILON {
                        out.push(Prim::HLine {
                            y: y0.round() as i32,
                            x0: x0.min(x1).round() as i32,
                            x1: x0.max(x1).round() as i32,
                            width: crisp_width,
                            style,
                            color: level_color,
                        });
                    } else {
                        let first_point = points.len() as u32;
                        points.extend([[x0 as f32, y0 as f32], [x1 as f32, y1 as f32]]);
                        out.push(Prim::Polyline {
                            first_point,
                            point_count: 2,
                            width: (drawing.width * vpr) as f32,
                            style,
                            line_type: LineType::Simple,
                            color: level_color,
                        });
                    }
                    if level.label_visible
                        && let Some(label) = self.fibonacci_level_label(
                            drawing,
                            ((x0, y0), (x1, y1)),
                            level.value,
                            vpr,
                            f64::from(pane_w_px),
                        )
                    {
                        self.push_level_label(drawing, label, level_color, vpr, out);
                    }
                }
            }
            DrawingBodyGeometry::TimeLevels(time) => {
                for (prior, level) in level_band_pairs(drawing, &drawing.levels, false) {
                    let prior_x = time.x(drawing.level_value(prior));
                    let x = time.x(drawing.level_value(level.value));
                    let left = x.min(prior_x).max(0.0);
                    let right = x.max(prior_x).min(f64::from(pane_w_px));
                    if right > left {
                        out.push(Prim::Rect {
                            rect: IRect {
                                x: left.round() as i32,
                                y: time.pane_top.round().max(0.0) as i32,
                                w: (right - left).round().max(1.0) as i32,
                                h: (time.pane_bottom - time.pane_top).round().max(1.0) as i32,
                            },
                            color: Self::drawing_level_fill(level, color),
                        });
                    }
                }
                push_fibonacci_trend_line(drawing, px, vpr, stroke.1, out, points);
                for level in &drawing.levels {
                    if !level.visible {
                        continue;
                    }
                    let x = time.x(drawing.level_value(level.value));
                    if !(0.0..=f64::from(pane_w_px)).contains(&x) {
                        continue;
                    }
                    let level_color = Color::parse_css(&level.color).unwrap_or(color);
                    let style = match level.style.as_str() {
                        "dotted" | "sparse_dotted" => LineStyle::Dotted,
                        "dashed" | "large_dashed" => LineStyle::Dashed,
                        _ => LineStyle::Solid,
                    };
                    out.push(Prim::VLine {
                        x: x.round() as i32,
                        y0: time.pane_top.round().max(0.0) as i32,
                        y1: time.pane_bottom.round().max(0.0) as i32,
                        width: crisp_width,
                        style,
                        color: level_color,
                    });
                    if level.label_visible
                        && let Some(label) =
                            self.time_level_label(drawing, time, x, level.value, vpr)
                    {
                        self.push_level_label(drawing, label, level_color, vpr, out);
                    }
                }
            }
            DrawingBodyGeometry::FibonacciArcs(arcs) => {
                if arcs.kind == DrawingKind::FibonacciWedge {
                    for &side in &px[1..3] {
                        push_segment(px[0], side, stroke, out, points);
                    }
                }
                let segments = arcs.segments(fibonacci::largest_level(drawing));
                // The fork's precise rings: every ring and band over the part the pane shows,
                // within a tenth of a pixel (`geometry::Rings`); upstream's whole rings otherwise.
                let precise = fibonacci::precise_rings(drawing);
                let rings = precise
                    .then(|| {
                        arcs.rings(
                            stroke.1,
                            drawing.width * vpr,
                            arcs.radius * fibonacci::largest_level(drawing),
                        )
                    })
                    .flatten();
                let (mut chain, mut scratch) = (Vec::new(), Vec::new());
                for (prior, level) in level_band_pairs(drawing, &drawing.levels, true) {
                    let fill = Self::drawing_level_fill(level, color);
                    let upper_first = points.len() as u32;
                    let count = if precise {
                        // A ring tool whose arc misses the pane paints no band.
                        let Some(rings) = rings else {
                            continue;
                        };
                        let radii = [prior, level.value]
                            .map(|value| arcs.radius * drawing.level_value(value));
                        if !rings.reaches(radii[0].min(radii[1]), radii[0].max(radii[1])) {
                            continue;
                        }
                        for radius in radii {
                            rings.chain(radius, &mut chain);
                            points.extend(chain.iter().map(|&(x, y)| [x as f32, y as f32]));
                        }
                        chain.len() as u32
                    } else {
                        for value in [prior, level.value] {
                            for step in 0..=segments {
                                let (x, y) = arcs.point(
                                    drawing.level_value(value),
                                    step as f64 / f64::from(segments),
                                );
                                points.push([x as f32, y as f32]);
                            }
                        }
                        segments + 1
                    };
                    out.push(Prim::BandFill {
                        upper_first,
                        lower_first: upper_first + count,
                        point_count: count,
                        line_type: LineType::Simple,
                        fill,
                    });
                }
                if fibonacci::phi_spiral(drawing) {
                    let width = drawing.width * vpr;
                    arcs.phi_spiral(
                        fibonacci::options(drawing).reverse,
                        stroke.1,
                        (width, vpr, fibonacci::dash_period(drawing.style, width)),
                        |run| {
                            push_clipped_stroke(out, points, run, stroke.1, stroke.0, &mut scratch)
                        },
                    );
                }
                push_fibonacci_trend_line(drawing, px, vpr, stroke.1, out, points);
                for level in &drawing.levels {
                    if !level.visible || drawing.level_value(level.value) <= 0.0 {
                        continue;
                    }
                    let level_color = Color::parse_css(&level.color).unwrap_or(color);
                    let style = match level.style.as_str() {
                        "dotted" | "sparse_dotted" => LineStyle::Dotted,
                        "dashed" | "large_dashed" => LineStyle::Dashed,
                        _ => LineStyle::Solid,
                    };
                    let width = drawing.width * vpr;
                    if precise {
                        let radius = arcs.radius * drawing.level_value(level.value);
                        if let Some(rings) = rings.filter(|rings| rings.reaches(radius, radius)) {
                            let period = fibonacci::dash_period(style, width);
                            rings.stroke(radius, period, &mut chain, |run| {
                                push_clipped_stroke(
                                    out,
                                    points,
                                    run,
                                    stroke.1,
                                    (width as f32, style, level_color),
                                    &mut scratch,
                                );
                            });
                        }
                    } else {
                        let first_point = points.len() as u32;
                        for step in 0..=segments {
                            let (x, y) = arcs.point(
                                drawing.level_value(level.value),
                                step as f64 / f64::from(segments),
                            );
                            points.push([x as f32, y as f32]);
                        }
                        out.push(Prim::Polyline {
                            first_point,
                            point_count: segments + 1,
                            width: width as f32,
                            style,
                            line_type: LineType::Simple,
                            color: level_color,
                        });
                    }
                    if level.label_visible
                        && let Some(label) = self.arc_level_label(drawing, arcs, level.value, vpr)
                    {
                        self.push_level_label(drawing, label, level_color, vpr, out);
                    }
                }
            }
            DrawingBodyGeometry::Pitchfork(fork) => {
                for (prior, level) in level_band_pairs(drawing, &drawing.levels, false) {
                    let upper_first = points.len() as u32;
                    for value in [prior, level.value] {
                        let (a, b) = fork.segment(drawing.level_value(value));
                        points.extend([[a.0 as f32, a.1 as f32], [b.0 as f32, b.1 as f32]]);
                    }
                    out.push(Prim::BandFill {
                        upper_first,
                        lower_first: upper_first + 2,
                        point_count: 2,
                        line_type: LineType::Simple,
                        fill: Self::drawing_level_fill(level, color),
                    });
                }
                for level in &drawing.levels {
                    if !level.visible {
                        continue;
                    }
                    let (a, b) = fork.segment(drawing.level_value(level.value));
                    let level_color = Color::parse_css(&level.color).unwrap_or(color);
                    let style = match level.style.as_str() {
                        "dotted" | "sparse_dotted" => LineStyle::Dotted,
                        "dashed" | "large_dashed" => LineStyle::Dashed,
                        _ => LineStyle::Solid,
                    };
                    let first_point = points.len() as u32;
                    points.extend([[a.0 as f32, a.1 as f32], [b.0 as f32, b.1 as f32]]);
                    out.push(Prim::Polyline {
                        first_point,
                        point_count: 2,
                        width: (drawing.width * vpr) as f32,
                        style,
                        line_type: LineType::Simple,
                        color: level_color,
                    });
                    if level.label_visible {
                        let anchor = fork.anchor(drawing.level_value(level.value));
                        let price = self.drawing_level_price_at(drawing, anchor.1, vpr);
                        if let Some(text) = self.drawing_level_label(drawing, level.value, price) {
                            // Clear of the tine (both ways: the median runs back to the pivot)
                            // and of the base line it starts on.
                            let layout = &self.options.get().layout;
                            let size = layout.font_size * vpr;
                            let width = self.measure_text_run(
                                &text,
                                size,
                                &layout.font_family,
                                drawing.text_weight.unwrap_or(400),
                                drawing.text_italic,
                            );
                            let (x, y) = clear_label_center(
                                anchor,
                                &[
                                    fork.segment(drawing.level_value(level.value)).0,
                                    fork.segment(drawing.level_value(level.value)).1,
                                    fork.anchor(0.0),
                                    fork.anchor(1.0),
                                ],
                                width,
                                size * 1.2,
                                POINT_LABEL_GAP_CSS * vpr,
                                (0.0, -1.0),
                            );
                            out.push(Prim::Text {
                                x: x as f32,
                                y: y as f32,
                                text,
                                color: level_color,
                                size: size as f32,
                                family: layout.font_family.clone(),
                                align: TextAlign::Center,
                                weight: drawing.text_weight.unwrap_or(400),
                                italic: drawing.text_italic,
                            });
                        }
                    }
                }
            }
            DrawingBodyGeometry::Cycles(cycles) => {
                let fill = drawing
                    .fill_color
                    .as_deref()
                    .and_then(Color::parse_css)
                    .unwrap_or(Color::rgba(color.r(), color.g(), color.b(), 24));
                let mut previous_x: Option<f64> = None;
                cycles.for_each_visible_line(|index, x| {
                    if drawing.fill_enabled
                        && index % 2 != 0
                        && let Some(prior_x) = previous_x
                    {
                        out.push(Prim::Rect {
                            rect: IRect {
                                x: prior_x.min(x).round() as i32,
                                y: cycles.pane_top.round().max(0.0) as i32,
                                w: (x - prior_x).abs().round().max(1.0) as i32,
                                h: (cycles.pane_bottom - cycles.pane_top).round().max(1.0) as i32,
                            },
                            color: fill,
                        });
                    }
                    previous_x = Some(x);
                });
                cycles.for_each_visible_line(|index, x| {
                    out.push(Prim::VLine {
                        x: x.round() as i32,
                        y0: cycles.pane_top.round().max(0.0) as i32,
                        y1: cycles.pane_bottom.round().max(0.0) as i32,
                        width: crisp_width,
                        style: drawing.style,
                        color,
                    });
                    if drawing.kind == DrawingKind::TimeCycles {
                        out.push(Prim::Text {
                            x: (x + 4.0 * vpr) as f32,
                            y: (cycles.pane_top + 14.0 * vpr) as f32,
                            text: index.to_string(),
                            color,
                            size: (self.options.get().layout.font_size * vpr) as f32,
                            family: self.options.get().layout.font_family.clone(),
                            align: TextAlign::Left,
                            weight: drawing.text_weight.unwrap_or(400),
                            italic: drawing.text_italic,
                        });
                    }
                });
            }
            DrawingBodyGeometry::Sine(sine) => {
                if let Some((left, right)) = sine.visible_x() {
                    let count = sine.sample_count();
                    let first_point = points.len() as u32;
                    for step in 0..=count {
                        let x = left + (right - left) * f64::from(step) / f64::from(count);
                        points.push([x as f32, sine.y(x) as f32]);
                    }
                    out.push(Prim::Polyline {
                        first_point,
                        point_count: count + 1,
                        width: (drawing.width * vpr) as f32,
                        style: drawing.style,
                        line_type: LineType::Simple,
                        color,
                    });
                }
            }
            DrawingBodyGeometry::Marker(marker) => {
                let (a, b) = marker.stem();
                push_segment(a, b, stroke, out, points);
                let [a, b, c] = marker.triangle();
                out.push(Prim::Triangle {
                    a: [a.0 as f32, a.1 as f32],
                    b: [b.0 as f32, b.1 as f32],
                    c: [c.0 as f32, c.1 as f32],
                    color,
                });
            }
            DrawingBodyGeometry::PriceLabel { x, y } => {
                let label = self.price_label_layout(drawing, (x, y), vpr);
                let [left, top, width, height] = label.rect;
                let bottom = top + height;
                let radius = (3.0 * vpr).round().max(1.0) as f32;
                out.push(Prim::RoundRect {
                    x: left as f32,
                    y: top as f32,
                    w: width as f32,
                    h: height as f32,
                    radii: [radius, radius, radius, 0.0],
                    fill: color,
                    border_width: 0.0,
                    border_color: color,
                });
                out.push(Prim::Triangle {
                    a: [x as f32, y as f32],
                    b: [left as f32, bottom as f32],
                    c: [(left + label.tail * 1.5) as f32, bottom as f32],
                    color,
                });
                let layout = &self.options.get().layout;
                out.push(Prim::Text {
                    x: (left + label.padding) as f32,
                    y: (top + height / 2.0) as f32,
                    text: label.text,
                    color: drawing
                        .text_color
                        .as_deref()
                        .and_then(Color::parse_css)
                        .unwrap_or_else(|| color.contrast_text()),
                    size: label.size as f32,
                    family: layout.font_family.clone(),
                    align: TextAlign::Left,
                    weight: drawing.text_weight.unwrap_or(400),
                    italic: drawing.text_italic,
                });
            }
            DrawingBodyGeometry::IconStamp { center, size } => {
                let rect = [
                    (center.0 - size / 2.0) as f32,
                    (center.1 - size / 2.0) as f32,
                    size as f32,
                    size as f32,
                ];
                // An arrow marker paints its built-in arrow. A stamp's name resolves to a
                // host-registered image first; then a built-in solid icon, saved stamps
                // included (owner question Q1, answer A); then the fork's vector glyph of a
                // name the suite lacks (check, cross, triangle_up, triangle_down).
                let name = match arrow_marker_icon(drawing.kind) {
                    Some((arrow, _)) => Some(arrow),
                    None => drawing.icon_name.as_deref(),
                };
                let stamp_name = name.filter(|_| drawing.kind == DrawingKind::IconStamp);
                let image = stamp_name.and_then(|name| self.drawing_icons.get(name));
                let builtin = name.and_then(builtin_icon);
                let fork_glyph = stamp_name.and_then(ForkGlyph::from_name);
                if let Some(image) = image {
                    out.push(Prim::Image {
                        image: image.clone(),
                        rect,
                        opacity: 1.0,
                    });
                } else if let Some(icon) = builtin {
                    // A solid icon rasterizes at its whole-device-px size on a pixel-aligned
                    // square, so executors draw it 1:1 without resampling. While a corner drag
                    // resizes it, each sample's size paints exactly but stays out of the cache;
                    // the frame after the release caches the final size.
                    let edge = size.round().max(1.0);
                    let resizing = self.drawing_drag.as_ref().is_some_and(|drag| {
                        drag.id == drawing.id
                            && matches!(drag.part, crate::DrawingDragPart::Anchor(_))
                            && self.drawing_handle_mode(drawing) == DrawingHandleMode::IconBox
                    });
                    let image =
                        self.icon_rasters
                            .borrow_mut()
                            .raster(icon, edge as u32, color, !resizing);
                    out.push(Prim::Image {
                        rect: [
                            (center.0 - edge / 2.0).round() as f32,
                            (center.1 - edge / 2.0).round() as f32,
                            image.width as f32,
                            image.height as f32,
                        ],
                        image,
                        opacity: 1.0,
                    });
                } else if let Some((glyph, context)) = fork_glyph.and_then(|glyph| {
                    Some((glyph, self.frame_part_context(drawing, px, pane_w_px, vpr)?))
                }) {
                    self.push_parts(&context, pane_w_px, vpr, out, points, |c, parts| {
                        fork_glyph_parts(c, glyph, center, size, parts);
                    });
                } else {
                    out.push(Prim::Rect {
                        rect: IRect {
                            x: rect[0].round() as i32,
                            y: rect[1].round() as i32,
                            w: size.round().max(1.0) as i32,
                            h: size.round().max(1.0) as i32,
                        },
                        color,
                    });
                }
            }
            DrawingBodyGeometry::GannGrid(grid) => {
                let box_bounds = grid.bounds();
                // A box's own time levels split the axes: price levels draw horizontally, time
                // levels vertically, and the zones fill as overlapping per-axis bands.
                let time_levels = pitchforks_gann::box_time_levels(drawing);
                let x_at = |value: f64| grid.start.0 + (grid.end.0 - grid.start.0) * value;
                if drawing.fill_enabled {
                    let fill = drawing
                        .fill_color
                        .as_deref()
                        .and_then(Color::parse_css)
                        .unwrap_or(Color::rgba(color.r(), color.g(), color.b(), 24));
                    out.push(Prim::Rect {
                        rect: IRect {
                            x: box_bounds.left.round() as i32,
                            y: box_bounds.top.round() as i32,
                            w: (box_bounds.right - box_bounds.left).round().max(1.0) as i32,
                            h: (box_bounds.bottom - box_bounds.top).round().max(1.0) as i32,
                        },
                        color: fill,
                    });
                    for ((x0, x1), (y0, y1), level) in pitchforks_gann::grid_bands(drawing, grid) {
                        out.push(Prim::Rect {
                            rect: IRect {
                                x: x0.min(x1).round() as i32,
                                y: y0.min(y1).round() as i32,
                                w: (x1 - x0).abs().round().max(1.0) as i32,
                                h: (y1 - y0).abs().round().max(1.0) as i32,
                            },
                            color: Self::drawing_level_fill(level, color),
                        });
                    }
                    let mut prior_fan: Option<(f64, f64)> = None;
                    for level in drawing.gann_fans.iter().filter(|level| level.visible) {
                        let (fan_start, end) = grid.fan_segment(level.value, drawing.level_reverse);
                        if level.fill_between
                            && let Some(previous) = prior_fan
                        {
                            out.push(Prim::Triangle {
                                a: [fan_start.0 as f32, fan_start.1 as f32],
                                b: [previous.0 as f32, previous.1 as f32],
                                c: [end.0 as f32, end.1 as f32],
                                color: Self::drawing_level_fill(level, color),
                            });
                        }
                        prior_fan = Some(end);
                    }
                    let gann_segments = gann_arc_segments(grid, drawing);
                    let mut prior_arc: Option<f64> = None;
                    for level in drawing.gann_arcs.iter().filter(|level| level.visible) {
                        if level.fill_between
                            && let Some(previous) = prior_arc
                        {
                            let upper_first = points.len() as u32;
                            for step in 0..=gann_segments {
                                let p = grid.arc_point(
                                    previous,
                                    f64::from(step) / f64::from(gann_segments),
                                    drawing.level_reverse,
                                );
                                points.push([p.0 as f32, p.1 as f32]);
                            }
                            let lower_first = points.len() as u32;
                            for step in 0..=gann_segments {
                                let p = grid.arc_point(
                                    level.value,
                                    f64::from(step) / f64::from(gann_segments),
                                    drawing.level_reverse,
                                );
                                points.push([p.0 as f32, p.1 as f32]);
                            }
                            out.push(Prim::BandFill {
                                upper_first,
                                lower_first,
                                point_count: gann_segments + 1,
                                line_type: LineType::Simple,
                                fill: Self::drawing_level_fill(level, color),
                            });
                        }
                        prior_arc = Some(level.value);
                    }
                }
                let corners = [
                    (box_bounds.left, box_bounds.top),
                    (box_bounds.right, box_bounds.top),
                    (box_bounds.right, box_bounds.bottom),
                    (box_bounds.left, box_bounds.bottom),
                ];
                for index in 0..4 {
                    push_segment(
                        corners[index],
                        corners[(index + 1) % 4],
                        stroke,
                        out,
                        points,
                    );
                }
                // Border labels already placed; a label that would overlap one is skipped, so
                // a small box keeps its labels readable. The fork's time-level labels above a
                // box and a square's stats box (painted after the levels) hold their places
                // first.
                let mut placed_labels: Vec<[f64; 4]> = Vec::new();
                let stats = self
                    .frame_part_context(drawing, px, pane_w_px, vpr)
                    .filter(|_| pitchforks_gann::shows_stats(drawing))
                    .map(|context| {
                        let mut parts = DrawingParts::default();
                        pitchforks_gann::square_stats(&context, grid, &mut parts);
                        (context, parts)
                    });
                for label in stats.iter().flat_map(|(_, parts)| &parts.labels) {
                    let rect = label
                        .layout(|line| self.measure_part_label(label, line))
                        .rect;
                    placed_labels.push([rect.left, rect.top, rect.right, rect.bottom]);
                }
                for level in time_levels
                    .into_iter()
                    .flatten()
                    .filter(|level| level.visible && level.label_visible)
                {
                    if let Some(text) = self.drawing_level_label(drawing, level.value, None) {
                        let layout = &self.options.get().layout;
                        let size = layout.font_size * vpr;
                        let width = self.measure_text_run(
                            &text,
                            size,
                            &layout.font_family,
                            drawing.text_weight.unwrap_or(400),
                            drawing.text_italic,
                        );
                        let (x, y) = (
                            x_at(drawing.level_value(level.value)),
                            box_bounds.top - 8.0 * vpr,
                        );
                        placed_labels.push([
                            x - width / 2.0,
                            y - size * 0.6,
                            x + width / 2.0,
                            y + size * 0.6,
                        ]);
                    }
                }
                for level in drawing.levels.iter().filter(|level| level.visible) {
                    let level_color = Color::parse_css(&level.color).unwrap_or(color);
                    let style = match level.style.as_str() {
                        "dotted" | "sparse_dotted" => LineStyle::Dotted,
                        "dashed" | "large_dashed" => LineStyle::Dashed,
                        _ => LineStyle::Solid,
                    };
                    let lines = grid.level_lines(drawing.level_value(level.value));
                    let lines = if time_levels.is_some() {
                        &lines[1..]
                    } else {
                        &lines[..]
                    };
                    for &(a, b) in lines {
                        let first_point = points.len() as u32;
                        points.extend([[a.0 as f32, a.1 as f32], [b.0 as f32, b.1 as f32]]);
                        out.push(Prim::Polyline {
                            first_point,
                            point_count: 2,
                            width: (drawing.width * vpr) as f32,
                            style,
                            line_type: LineType::Simple,
                            color: level_color,
                        });
                    }
                    if level.label_visible {
                        let x = grid.start.0
                            + (grid.end.0 - grid.start.0) * drawing.level_value(level.value);
                        let y = grid.start.1
                            + (grid.end.1 - grid.start.1) * drawing.level_value(level.value);
                        let price = self
                            .drawing_scale_for(drawing.pane_index, drawing.price_scale)
                            .map(|scale| {
                                scale.coordinate_to_price(
                                    y / vpr,
                                    self.drawing_scale_base_for(
                                        drawing.pane_index,
                                        drawing.price_scale,
                                    ),
                                )
                            });
                        // Levels label on the box border, outside the grid that the fans and arcs
                        // fill: the price level left of the left edge (with its price), the time
                        // level under the bottom edge.
                        let layout = &self.options.get().layout;
                        let size = layout.font_size * vpr;
                        let gap = POINT_LABEL_GAP_CSS * vpr;
                        let edges = grid.bounds();
                        let mut push_label = |text: String, x: f64, y: f64, align: TextAlign| {
                            let width = self.measure_text_run(
                                &text,
                                size,
                                &layout.font_family,
                                drawing.text_weight.unwrap_or(400),
                                drawing.text_italic,
                            );
                            let left = match align {
                                TextAlign::Left => x,
                                TextAlign::Center => x - width / 2.0,
                                TextAlign::Right => x - width,
                            };
                            let rect = [left, y - size * 0.6, left + width, y + size * 0.6];
                            if placed_labels.iter().any(|other| {
                                rect[0] < other[2] + gap
                                    && other[0] < rect[2] + gap
                                    && rect[1] < other[3]
                                    && other[1] < rect[3]
                            }) {
                                return;
                            }
                            placed_labels.push(rect);
                            out.push(Prim::Text {
                                x: x as f32,
                                y: y as f32,
                                text,
                                color: level_color,
                                size: size as f32,
                                family: layout.font_family.clone(),
                                align,
                                weight: drawing.text_weight.unwrap_or(400),
                                italic: drawing.text_italic,
                            });
                        };
                        if let Some(text) = self.drawing_level_label(drawing, level.value, price) {
                            push_label(text, edges.left - gap, y, TextAlign::Right);
                        }
                        // A box's own time levels label above it (the fork's `time_levels`),
                        // so a price level never also labels a time under it.
                        if time_levels.is_none()
                            && let Some(text) = self.drawing_level_label(drawing, level.value, None)
                        {
                            push_label(text, x, edges.bottom + gap + size * 0.6, TextAlign::Center);
                        }
                    }
                }
                for level in time_levels
                    .into_iter()
                    .flatten()
                    .filter(|level| level.visible)
                {
                    let x = x_at(drawing.level_value(level.value));
                    let level_color = Color::parse_css(&level.color).unwrap_or(color);
                    let first_point = points.len() as u32;
                    points.extend([
                        [x as f32, grid.start.1 as f32],
                        [x as f32, grid.end.1 as f32],
                    ]);
                    out.push(Prim::Polyline {
                        first_point,
                        point_count: 2,
                        width: (drawing.width * vpr) as f32,
                        style: Self::drawing_level_style(&level.style),
                        line_type: LineType::Simple,
                        color: level_color,
                    });
                    if level.label_visible
                        && let Some(text) = self.drawing_level_label(drawing, level.value, None)
                    {
                        out.push(Prim::Text {
                            x: x as f32,
                            y: (box_bounds.top - 8.0 * vpr) as f32,
                            text,
                            color: level_color,
                            size: (self.options.get().layout.font_size * vpr) as f32,
                            family: self.options.get().layout.font_family.clone(),
                            align: TextAlign::Center,
                            weight: drawing.text_weight.unwrap_or(400),
                            italic: drawing.text_italic,
                        });
                    }
                }
                // A box strokes its `tool_options.gann.angles` while `show_angles` is on; a
                // square its `gann_fans`.
                for level in pitchforks_gann::angle_levels(drawing)
                    .iter()
                    .filter(|level| level.visible)
                {
                    let (a, b) = grid.fan_segment(level.value, drawing.level_reverse);
                    let level_color = Color::parse_css(&level.color).unwrap_or(color);
                    let first_point = points.len() as u32;
                    points.extend([[a.0 as f32, a.1 as f32], [b.0 as f32, b.1 as f32]]);
                    out.push(Prim::Polyline {
                        first_point,
                        point_count: 2,
                        width: (drawing.width * vpr) as f32,
                        style: Self::drawing_level_style(&level.style),
                        line_type: LineType::Simple,
                        color: level_color,
                    });
                }
                if grid.kind != DrawingKind::GannBox {
                    let gann_segments = gann_arc_segments(grid, drawing);
                    for level in drawing.gann_arcs.iter().filter(|level| level.visible) {
                        let level_color = Color::parse_css(&level.color).unwrap_or(color);
                        let first_point = points.len() as u32;
                        for step in 0..=gann_segments {
                            let point = grid.arc_point(
                                level.value,
                                f64::from(step) / f64::from(gann_segments),
                                drawing.level_reverse,
                            );
                            points.push([point.0 as f32, point.1 as f32]);
                        }
                        out.push(Prim::Polyline {
                            first_point,
                            point_count: gann_segments + 1,
                            width: (drawing.width * vpr) as f32,
                            style: Self::drawing_level_style(&level.style),
                            line_type: LineType::Simple,
                            color: level_color,
                        });
                    }
                }
                // A square's stats box (`tool_options.gann.show_stats`) paints last.
                if let Some((context, parts)) = &stats {
                    self.push_drawing_parts(context, parts, pane_w_px, vpr, out, points);
                }
            }
            DrawingBodyGeometry::Quad { corners } => {
                if drawing.fill_enabled {
                    let fill = drawing
                        .fill_color
                        .as_deref()
                        .and_then(Color::parse_css)
                        .unwrap_or(Color::rgba(color.r(), color.g(), color.b(), 51));
                    let a = [corners[0].0 as f32, corners[0].1 as f32];
                    let b = [corners[1].0 as f32, corners[1].1 as f32];
                    let c = [corners[2].0 as f32, corners[2].1 as f32];
                    let d = [corners[3].0 as f32, corners[3].1 as f32];
                    out.push(Prim::Triangle {
                        a,
                        b,
                        c,
                        color: fill,
                    });
                    out.push(Prim::Triangle {
                        a,
                        b: c,
                        c: d,
                        color: fill,
                    });
                }
                // One seamless outline run (owner decision S7): every corner is a join.
                push_closed_outline(&corners, stroke, out, points);
            }
            DrawingBodyGeometry::Ellipse { center, rx, ry } => {
                let clip = curve_clip(stroke.1, drawing.width, vpr);
                if rx > 0.0 && ry > 0.0 && box_meets(center, rx, ry, clip) {
                    let mut outline = Vec::new();
                    ellipse_outline(center, rx, ry, clip, &mut outline);
                    if drawing.fill_enabled {
                        let fill = drawing
                            .fill_color
                            .as_deref()
                            .and_then(Color::parse_css)
                            .unwrap_or(Color::rgba(color.r(), color.g(), color.b(), 51));
                        // The closing point repeats the first; the fill polygon needs it once.
                        let mut ribbon = Vec::new();
                        let count = aeris_charts_render::shape::convex_ribbon(
                            &outline[..outline.len().saturating_sub(1)],
                            &mut ribbon,
                        ) as u32;
                        if count > 0 {
                            let upper_first = points.len() as u32;
                            points.extend(ribbon.iter().map(|&(x, y)| [x as f32, y as f32]));
                            out.push(Prim::BandFill {
                                upper_first,
                                lower_first: upper_first + count,
                                point_count: count,
                                line_type: LineType::Simple,
                                fill,
                            });
                        }
                    }
                    push_styled_stroke(out, points, &outline, LineType::Simple, stroke.0, stroke.1);
                }
            }
            DrawingBodyGeometry::Circle { center, radius } => {
                let clip = curve_clip(stroke.1, drawing.width, vpr);
                if box_meets(center, radius, radius, clip) {
                    if drawing.fill_enabled {
                        let fill = drawing
                            .fill_color
                            .as_deref()
                            .and_then(Color::parse_css)
                            .unwrap_or(Color::rgba(color.r(), color.g(), color.b(), 51));
                        out.push(Prim::Circle {
                            cx: center.0 as f32,
                            cy: center.1 as f32,
                            radius: radius as f32,
                            fill,
                            stroke_width: 0.0,
                            stroke: color,
                        });
                    }
                    let mut outline = Vec::new();
                    ellipse_outline(center, radius, radius, clip, &mut outline);
                    push_styled_stroke(out, points, &outline, LineType::Simple, stroke.0, stroke.1);
                }
            }
            DrawingBodyGeometry::Triangle { corners } => {
                if drawing.fill_enabled {
                    let fill = drawing
                        .fill_color
                        .as_deref()
                        .and_then(Color::parse_css)
                        .unwrap_or(Color::rgba(color.r(), color.g(), color.b(), 51));
                    out.push(Prim::Triangle {
                        a: [corners[0].0 as f32, corners[0].1 as f32],
                        b: [corners[1].0 as f32, corners[1].1 as f32],
                        c: [corners[2].0 as f32, corners[2].1 as f32],
                        color: fill,
                    });
                }
                if drawing.kind == DrawingKind::Triangle {
                    // One seamless outline run (owner decision S7): every corner is a join.
                    push_closed_outline(&corners, stroke, out, points);
                } else {
                    for index in 0..3 {
                        push_segment(
                            corners[index],
                            corners[(index + 1) % 3],
                            stroke,
                            out,
                            points,
                        );
                    }
                }
            }
            DrawingBodyGeometry::Sector(arc) => {
                // A fork-form projection: the sector's fill (a fan from the pivot) under its
                // outline from the pivot along the arc and back, then its stats box.
                let mut arc_points = Vec::new();
                arc.flatten(curve_clip(stroke.1, drawing.width, vpr), &mut arc_points);
                if drawing.fill_enabled && arc_points.len() >= 2 {
                    let count = arc_points.len();
                    let mut ribbon = arc_points.clone();
                    ribbon.resize(2 * count, arc.center);
                    push_ribbon(&ribbon, count, drawing.fill_or_wash(51), out, points);
                }
                let mut outline = Vec::with_capacity(arc_points.len() + 2);
                outline.push(arc.center);
                outline.extend_from_slice(&arc_points);
                outline.push(arc.center);
                push_styled_stroke(out, points, &outline, LineType::Simple, stroke.0, stroke.1);
                if let (Some(context), Some(&target)) = (
                    self.frame_part_context(drawing, px, pane_w_px, vpr),
                    px.get(1),
                ) {
                    self.push_parts(&context, pane_w_px, vpr, out, points, |c, parts| {
                        projection_annotations::projection_stats(c, arc.center, target, parts);
                    });
                }
            }
            DrawingBodyGeometry::NotePin(pin) => {
                // A fork-form note: the pin in the stroke color with a contrasting dot in its
                // head; its box is the drawing's text (`build_drawing_text`).
                let mut outline = Vec::new();
                pin.outline(&mut outline);
                let mut ribbon = Vec::new();
                let count = aeris_charts_render::shape::convex_ribbon(&outline, &mut ribbon);
                push_ribbon(&ribbon, count, color, out, points);
                let dot = color.solid().contrast_text();
                out.push(Prim::Circle {
                    cx: pin.head.0 as f32,
                    cy: pin.head.1 as f32,
                    radius: pin.dot_radius as f32,
                    fill: dot,
                    stroke_width: 0.0,
                    stroke: dot,
                });
            }
            DrawingBodyGeometry::SpeechTail { corners } => {
                // A fork-form comment's or price label's tail in the bubble's stroke color; the
                // bubble is the drawing's text (`build_drawing_text`).
                let mut ribbon = Vec::new();
                let count = aeris_charts_render::shape::convex_ribbon(&corners, &mut ribbon);
                push_ribbon(&ribbon, count, color, out, points);
            }
            DrawingBodyGeometry::Arc(_) | DrawingBodyGeometry::Curve(_) => {
                // The chord fill under the stroke; with an end cap, the shared capped stroke.
                let clip = curve_clip(stroke.1, drawing.width, vpr);
                if let Some(curve) = shapes::CurveStroke::resolve(geometry.body, clip) {
                    let run = curve.run();
                    if curve.meets_clip(&run) {
                        if drawing.fill_enabled {
                            let mut ribbon = Vec::new();
                            let count = curve.chord_fill(&mut ribbon);
                            push_ribbon(&ribbon, count, drawing.fill_or_wash(51), out, points);
                        }
                        match self
                            .frame_part_context(drawing, px, pane_w_px, vpr)
                            .filter(|_| shapes::capped(drawing))
                        {
                            Some(context) => self.push_parts(
                                &context,
                                pane_w_px,
                                vpr,
                                out,
                                points,
                                |c, parts| curve.capped_parts(&run, c.drawing, c.scale, parts),
                            ),
                            None => push_styled_stroke(
                                out,
                                points,
                                &run,
                                LineType::Simple,
                                stroke.0,
                                stroke.1,
                            ),
                        }
                    }
                }
            }
            DrawingBodyGeometry::Polygon { points: vertices } => {
                // A closed polyline: its nonzero fill under one outline run from mid-edge, no
                // caps (they stay stored for reopening it).
                if drawing.fill_enabled {
                    let mut ribbon = Vec::new();
                    let count = aeris_charts_render::shape::nonzero_ribbon(vertices, &mut ribbon);
                    push_ribbon(&ribbon, count, drawing.fill_or_wash(51), out, points);
                }
                push_closed_outline(vertices, stroke, out, points);
            }
            DrawingBodyGeometry::Rectangle {
                left,
                right,
                top,
                bottom,
            } => {
                let left = left.round() as i32;
                let right = right.round() as i32;
                let top = top.round() as i32;
                let bottom = bottom.round() as i32;
                // Official `positionsBox`: both endpoint pixels belong to the box, so an
                // equal-point preview still occupies one bitmap pixel.
                let width = (right - left).abs() + 1;
                let height = (bottom - top).abs() + 1;
                // reference rectangle-drawing-tool default: the fill is the border color washed
                // out (its `previewFillColor`/`fillColor` alpha pattern) — 20% here.
                let fill = drawing.fill_or_wash(51);
                if drawing.fill_enabled {
                    out.push(Prim::Rect {
                        rect: IRect {
                            x: left,
                            y: top,
                            w: width,
                            h: height,
                        },
                        color: fill,
                    });
                }
                if !drawing.border_visible {
                    return;
                }
                if drawing.style == LineStyle::Solid {
                    out.push(Prim::RectFrame {
                        rect: IRect {
                            x: left,
                            y: top,
                            w: width,
                            h: height,
                        },
                        border: crisp_width,
                        color,
                    });
                } else {
                    // Dotted/dashed border: four crisp line prims sharing the dash pattern,
                    // centered on the frame's inner edge (where RectFrame paints).
                    let half = (crisp_width as f64 / 2.0) as i32;
                    for y in [top + half, top + height - half] {
                        out.push(Prim::HLine {
                            y,
                            x0: left,
                            x1: left + width,
                            width: crisp_width,
                            style: drawing.style,
                            color,
                        });
                    }
                    for x in [left + half, left + width - half] {
                        out.push(Prim::VLine {
                            x,
                            y0: top,
                            y1: top + height,
                            width: crisp_width,
                            style: drawing.style,
                            color,
                        });
                    }
                }
            }
            DrawingBodyGeometry::Position(position) => {
                let reward = Color::parse_css(aeris_charts_core::style::MARKET_UP_CSS)
                    .unwrap_or(Color::rgb(8, 153, 129));
                let risk = Color::parse_css(aeris_charts_core::style::MARKET_DOWN_CSS)
                    .unwrap_or(Color::rgb(247, 82, 95));
                push_position_zone(out, position.reward_zone(), reward);
                push_position_zone(out, position.risk_zone(), risk);

                let left_px = position.left.round() as i32;
                let right_px = position.right.round() as i32;
                if left_px != right_px {
                    let entry_path = [
                        [left_px as f32, position.entry_y as f32],
                        [right_px as f32, position.entry_y as f32],
                    ];
                    push_line_stroke(
                        out,
                        points,
                        &entry_path,
                        (POSITION_ENTRY_WIDTH_CSS * vpr) as f32,
                        drawing.style,
                        LineType::Simple,
                        POSITION_ENTRY,
                    );
                }
            }
            // The text tool's geometry is its label (emitted by `build_drawing_text`).
            DrawingBodyGeometry::Empty => {}
            DrawingBodyGeometry::Polyline {
                points: line_points,
                line_type,
                terminal,
            } => {
                // A highlighter is the same stroke at a translucent alpha; its caps and labels
                // follow the translucent color.
                let highlighter = drawing.kind == DrawingKind::Highlighter;
                let color = if highlighter {
                    Color::rgba(
                        color.r(),
                        color.g(),
                        color.b(),
                        ((color.a() as u16 * 64) / 255) as u8,
                    )
                } else {
                    color
                };
                // A pattern layers its fills under the zigzag and its sides, ratio connectors
                // and ratios over it (`kinds::patterns_elliott_cycles`).
                let pattern = if patterns_elliott_cycles::layers_parts(drawing.kind) {
                    self.frame_part_context(drawing, px, pane_w_px, vpr)
                } else {
                    None
                };
                if let Some(context) = &pattern {
                    self.push_parts(context, pane_w_px, vpr, out, points, |c, parts| {
                        patterns_elliott_cycles::pattern_parts(c, PatternLayer::Under, parts);
                    });
                }
                // An Elliott wave without `show_wave` paints only its labels.
                let wave = patterns_elliott_cycles::draws_wave(drawing);
                // A solid highlighter paints as the region its stroke covers (the tube around
                // the same expanded path the stroke would take, round joins and caps), filled
                // once per pixel: where a wide translucent stroke overlaps or crosses itself, the
                // GPU executors would blend its overlapping triangles twice. Beyond the fill
                // bounds even when coarsened, it falls back to the stroke.
                let mut tube = Vec::new();
                let tube_count = if highlighter && drawing.style == LineStyle::Solid {
                    let path = line_points
                        .iter()
                        .map(|&(x, y)| aeris_charts_render::line::LinePoint { x, y })
                        .collect::<Vec<_>>();
                    let mut expanded = Vec::new();
                    aeris_charts_render::line::expand_line_into(
                        &path,
                        line_type,
                        1.0,
                        1.0,
                        &mut expanded,
                    );
                    let expanded = expanded
                        .iter()
                        .map(|point| (point.x, point.y))
                        .collect::<Vec<_>>();
                    aeris_charts_render::shape::tube_ribbon(
                        &expanded,
                        f64::from(stroke.0.0) / 2.0,
                        HIGHLIGHTER_TUBE_TOLERANCE,
                        stroke.1,
                        &mut tube,
                    )
                } else {
                    0
                };
                if tube_count > 0 {
                    let upper_first = points.len() as u32;
                    points.extend(tube.iter().map(|&(x, y)| [x as f32, y as f32]));
                    out.push(Prim::BandFill {
                        upper_first,
                        lower_first: upper_first + tube_count as u32,
                        point_count: tube_count as u32,
                        line_type: LineType::Simple,
                        fill: color,
                    });
                } else if wave {
                    push_styled_stroke(
                        out,
                        points,
                        line_points,
                        line_type,
                        (stroke.0.0, stroke.0.1, color),
                        stroke.1,
                    );
                }
                if let (Some(first), Some(last)) = (line_points.first(), line_points.last())
                    && line_points.len() >= 2
                    && wave
                {
                    push_drawing_cap(
                        drawing.stroke_start,
                        *first,
                        line_points[1],
                        drawing.width,
                        vpr,
                        color,
                        out,
                        points,
                    );
                    push_drawing_cap(
                        drawing.stroke_end,
                        *last,
                        line_points[line_points.len() - 2],
                        drawing.width,
                        vpr,
                        color,
                        out,
                        points,
                    );
                }
                if let Some(terminal) = terminal {
                    let first_point = points.len() as u32;
                    for (x, y) in terminal {
                        points.push([x as f32, y as f32]);
                    }
                    out.push(Prim::Polyline {
                        first_point,
                        point_count: 3,
                        width: (drawing.width * vpr) as f32,
                        style: LineStyle::Solid,
                        line_type: LineType::Simple,
                        color,
                    });
                }
                if let Some(context) = &pattern {
                    self.push_parts(context, pane_w_px, vpr, out, points, |c, parts| {
                        patterns_elliott_cycles::pattern_parts(c, PatternLayer::Over, parts);
                    });
                }
                if drawing.kind.vertex_labels().is_some() {
                    let label_color = drawing
                        .text_color
                        .as_deref()
                        .and_then(Color::parse_css)
                        .unwrap_or(color);
                    // Upstream's placement, shared with hit testing and the culling pad; an
                    // Elliott degree's ring (1 CSS px) paints before its label.
                    let (size, labels) =
                        patterns_elliott_cycles::vertex_labels(self, drawing, line_points, vpr);
                    let clip = aeris_charts_render::shape::Rect {
                        left: 0.0,
                        top: pane.top * vpr,
                        right: f64::from(pane_w_px),
                        bottom: (pane.top + pane.height) * vpr,
                    };
                    let (mut ring, mut scratch) = (Vec::new(), Vec::new());
                    for label in labels {
                        patterns_elliott_cycles::ring_points(&label, &mut ring);
                        push_clipped_stroke(
                            out,
                            points,
                            &ring,
                            clip,
                            (
                                (patterns_elliott_cycles::DECORATION_WIDTH * vpr) as f32,
                                LineStyle::Solid,
                                label_color,
                            ),
                            &mut scratch,
                        );
                        out.push(Prim::Text {
                            x: label.center.0 as f32,
                            y: label.center.1 as f32,
                            text: label.text,
                            color: label_color,
                            size: size as f32,
                            family: self.options.get().layout.font_family.clone(),
                            align: TextAlign::Center,
                            weight: drawing.text_weight.unwrap_or(400),
                            italic: drawing.text_italic,
                        });
                    }
                }
            }
        }
        if projection_annotations::draws_forecast_boxes(drawing) {
            // The fork form's dot and boxes take the place of upstream's outcome label.
            if let Some(context) = self.frame_part_context(drawing, px, pane_w_px, vpr) {
                self.push_parts(
                    &context,
                    pane_w_px,
                    vpr,
                    out,
                    points,
                    projection_annotations::forecast_parts,
                );
            }
        } else if drawing.kind == DrawingKind::Forecast
            && let (Some(entry), Some(target)) = (drawing.points.first(), px.get(1))
        {
            let change = drawing.points[1].price - entry.price;
            let percent = if entry.price.abs() > f64::EPSILON {
                change / entry.price.abs() * 100.0
            } else {
                0.0
            };
            let result = match self.forecast_result(drawing) {
                Some(true) => "target reached",
                Some(false) => "expired",
                None => "pending",
            };
            let size = self.options.get().layout.font_size * vpr;
            let label_y = target.1 - 10.0 * vpr;
            out.push(Prim::Text {
                x: target.0 as f32,
                y: label_y as f32,
                text: format!("{:+.1}% · {}", percent, result),
                color,
                size: size as f32,
                family: self.options.get().layout.font_family.clone(),
                align: TextAlign::Center,
                weight: drawing.text_weight.unwrap_or(400),
                italic: drawing.text_italic,
            });
            // The fork's target time, kept on upstream's label: the target bar's time through
            // the bar label (`bar_time_label`), one line above, while the axis has time.
            if let Some(time) = self.drawing_anchor_time_of(drawing, 1) {
                out.push(Prim::Text {
                    x: target.0 as f32,
                    y: (label_y - size * 1.25) as f32,
                    text: self.format_crosshair_ts(time.round() as i64),
                    color,
                    size: size as f32,
                    family: self.options.get().layout.font_family.clone(),
                    align: TextAlign::Center,
                    weight: drawing.text_weight.unwrap_or(400),
                    italic: drawing.text_italic,
                });
            }
        }
    }

    fn build_bars_pattern_prims(
        &self,
        drawing: &Drawing,
        pane_w_px: i32,
        vpr: f64,
        out: &mut Vec<Prim>,
        points: &mut Vec<[f32; 2]>,
    ) {
        let color = Color::parse_css(&drawing.color).unwrap_or(PRIMARY);
        let ghost = Color::rgba(color.r(), color.g(), color.b(), 160);
        let hpr = f64::from(pane_w_px) / self.pane_w.max(1.0);
        let tick = (self.time_scale.bar_spacing() * hpr * 0.25)
            .round()
            .clamp(2.0, 6.0) as i32;
        let mode = drawing.bars_pattern_mode.as_str();
        let first_point = points.len() as u32;
        let mut line_count = 0_u32;
        for &bar in &drawing.bars_pattern {
            let projected = bar.project(drawing);
            let mut encoded = [(0.0, 0.0); 4];
            let mut valid = true;
            for (slot, point) in encoded.iter_mut().zip(projected) {
                if let Some((x, y)) =
                    self.drawing_to_px_for(drawing.pane_index, drawing.price_scale, point)
                {
                    *slot = (x * hpr, y * vpr);
                } else {
                    valid = false;
                    break;
                }
            }
            if !valid {
                continue;
            }
            let x = encoded[0].0.round() as i32;
            if mode == "oc_bars" {
                // One open-close stick per bar, no ticks; a doji still paints one device pixel.
                if x < -tick || x > pane_w_px + tick {
                    continue;
                }
                let y0 = encoded[0].1.min(encoded[3].1).round() as i32;
                let y1 = encoded[0].1.max(encoded[3].1).round() as i32;
                out.push(Prim::VLine {
                    x,
                    y0,
                    y1: y1.max(y0 + 1),
                    width: (drawing.width * vpr).round().max(1.0) as i32,
                    style: drawing.style,
                    color: ghost,
                });
            } else if matches!(mode, "bars" | "hl_bars") {
                // `hl_bars` is the fork's spelling of the high-low bars.
                if x < -tick || x > pane_w_px + tick {
                    continue;
                }
                out.push(Prim::VLine {
                    x,
                    y0: encoded[1].1.min(encoded[2].1).round() as i32,
                    y1: encoded[1].1.max(encoded[2].1).round() as i32,
                    width: (drawing.width * vpr).round().max(1.0) as i32,
                    style: drawing.style,
                    color: ghost,
                });
                out.push(Prim::HLine {
                    y: encoded[0].1.round() as i32,
                    x0: x - tick,
                    x1: x,
                    width: 1,
                    style: drawing.style,
                    color: ghost,
                });
                out.push(Prim::HLine {
                    y: encoded[3].1.round() as i32,
                    x0: x,
                    x1: x + tick,
                    width: 1,
                    style: drawing.style,
                    color: ghost,
                });
            } else {
                let index = match mode {
                    "line_open" => 0,
                    "line_high" => 1,
                    "line_low" => 2,
                    _ => 3,
                };
                points.push([encoded[index].0 as f32, encoded[index].1 as f32]);
                line_count += 1;
            }
        }
        if line_count >= 2 {
            out.push(Prim::Polyline {
                first_point,
                point_count: line_count,
                width: (drawing.width * vpr) as f32,
                style: drawing.style,
                line_type: LineType::Simple,
                color: ghost,
            });
        }
    }

    /// The `[start, end]` fraction of segment `a → b` a middle segment-layout label cuts out of
    /// the stroke: the measured run (the host editor's one-em minimum while editing) plus the
    /// text pad on each side, projected onto the segment.
    fn segment_label_gap(
        &self,
        drawing: &Drawing,
        px: &[(f64, f64)],
        pane_w_px: i32,
        vpr: f64,
        a: (f64, f64),
        b: (f64, f64),
    ) -> Option<(f64, f64)> {
        self.drawing_frame_gap_text(drawing)
            .filter(|_| {
                drawing.kind.spec().text_layout == DrawingTextLayout::Segment
                    && drawing.text_v_align == crate::drawings::DrawingTextVAlign::Middle
            })
            .and_then(|text| {
                let (size, x, y, align, angle) =
                    self.text_run_geometry(drawing, px, pane_w_px, vpr);
                let mut width = self.measure_drawing_frame_text(drawing, text, size);
                if self.editing_drawing() == Some(drawing.id) {
                    // Match the host editor's one-em empty/minimum width. This leaves a
                    // compact caret slot and then grows from actual shaped advance.
                    width = width.max(size);
                }
                let gap = TEXT_PAD * vpr;
                let (local_start, local_end) = match align {
                    DrawingTextHAlign::Left => (-gap, width + gap),
                    DrawingTextHAlign::Center => (-width / 2.0 - gap, width / 2.0 + gap),
                    DrawingTextHAlign::Right => (-width - gap, gap),
                };
                let length_sq = (b.0 - a.0).powi(2) + (b.1 - a.1).powi(2);
                if length_sq <= f64::EPSILON {
                    return None;
                }
                let project = |distance: f64| {
                    let px = x + angle.cos() * distance;
                    let py = y + angle.sin() * distance;
                    ((px - a.0) * (b.0 - a.0) + (py - a.1) * (b.1 - a.1)) / length_sq
                };
                let first = project(local_start);
                let second = project(local_end);
                let start = first.min(second).clamp(0.0, 1.0);
                let end = first.max(second).clamp(0.0, 1.0);
                (start < end).then_some((start, end))
            })
    }

    /// The color every text run of `drawing` resolves to: the explicit `text_color`, the stroke
    /// for segment-layout labels, then the chart foreground.
    pub(crate) fn drawing_label_color(&self, drawing: &Drawing) -> Color {
        let layout = &self.options.get().layout;
        drawing
            .text_color
            .as_deref()
            .and_then(Color::parse_css)
            .or_else(|| {
                (drawing.kind.spec().text_layout == DrawingTextLayout::Segment)
                    .then(|| Color::parse_css(&drawing.color))
                    .flatten()
            })
            .or_else(|| Color::parse_css(&layout.text_color))
            .unwrap_or_else(|| {
                let fallback = aeris_charts_core::style::DEFAULT_FOREGROUND_RGB;
                Color::rgb(fallback.0, fallback.1, fallback.2)
            })
    }

    /// Lower a family tool's shared parts (`drawings/parts.rs`: the own-line tools and the
    /// ranges; every other catalog tool renders through `build_drawing_prims`' geometry arms)
    /// into the ordered frame. The family resolves them in bitmap px, and `push_drawing_parts`
    /// is the only place parts become `Prim`s.
    fn build_family_prims(
        &self,
        drawing: &Drawing,
        px: &[(f64, f64)],
        pane_w_px: i32,
        vpr: f64,
        out: &mut Vec<Prim>,
        points: &mut Vec<[f32; 2]>,
    ) {
        let (Some(family), Some(context)) = (
            drawing.kind.spec().family,
            self.frame_part_context(drawing, px, pane_w_px, vpr),
        ) else {
            return;
        };
        debug_assert!(
            drawing.points.len() != px.len()
                || drawing.points.iter().zip(px).all(|(&point, &anchor)| {
                    context.point_px(point).is_none_or(|mapped| {
                        (mapped.0 - anchor.0).abs() <= 1e-6 * anchor.0.abs().max(1.0)
                            && (mapped.1 - anchor.1).abs() <= 1e-6 * anchor.1.abs().max(1.0)
                    })
                }),
            "derived family points share the anchors' caller-px space"
        );
        self.push_parts(&context, pane_w_px, vpr, out, points, family.build_parts);
    }

    /// The bitmap-px part context of `drawing` with anchors at `px`; `None` for a stale pane.
    fn frame_part_context<'a>(
        &'a self,
        drawing: &'a Drawing,
        px: &'a [(f64, f64)],
        pane_w_px: i32,
        vpr: f64,
    ) -> Option<PartContext<'a>> {
        let pane = self.panes.get(drawing.pane_index)?;
        // The frame's horizontal ratio: `pane_w_px` is the rounded bitmap width it derives from.
        let hpr = f64::from(pane_w_px) / self.pane_w.max(1.0);
        Some(PartContext {
            engine: self,
            drawing,
            px,
            pane: aeris_charts_render::shape::Rect {
                left: 0.0,
                top: pane.top * vpr,
                right: f64::from(pane_w_px),
                bottom: (pane.top + pane.height) * vpr,
            },
            scale: vpr,
            x_scale: hpr,
            text_editing: self.editing_drawing() == Some(drawing.id),
        })
    }

    /// Resolve the parts `build` describes for `context` (from [`Self::frame_part_context`]) and
    /// lower them with [`Self::push_drawing_parts`]: a family's whole drawing, or the parts an
    /// upstream-rendered drawing layers over its arm (the vector glyph of an icon stamp, the
    /// presentation an upstream line tool's `line` block selects). Hit testing resolves the same
    /// parts (`parts_hit`), and the culling pad covers their reach (a family's
    /// `decoration_extent`, `kinds::upstream_decoration_extent`).
    fn push_parts(
        &self,
        context: &PartContext<'_>,
        pane_w_px: i32,
        vpr: f64,
        out: &mut Vec<Prim>,
        points: &mut Vec<[f32; 2]>,
        build: impl FnOnce(&PartContext<'_>, &mut DrawingParts),
    ) {
        let mut parts = DrawingParts::default();
        build(context, &mut parts);
        self.push_drawing_parts(context, &parts, pane_w_px, vpr, out, points);
    }

    /// Lower `parts` resolved for `context` into frame primitives: dashed strokes as solid dash
    /// runs clipped to the pane, crisp axis-aligned lines, fills, discs, tubes, and boxed labels
    /// with the open native session's caret. The family path and the built-in icon glyphs share
    /// this one lowering.
    fn push_drawing_parts(
        &self,
        context: &PartContext<'_>,
        parts: &DrawingParts,
        pane_w_px: i32,
        vpr: f64,
        out: &mut Vec<Prim>,
        points: &mut Vec<[f32; 2]>,
    ) {
        let (drawing, px) = (context.drawing, context.px);
        let mut scratch = Vec::new();
        let color = drawing.stroke_color();
        let layout = &self.options.get().layout;
        for part in &parts.items {
            match *part {
                DrawingPart::Stroke {
                    start,
                    end,
                    stroke,
                    label_gap,
                } => {
                    let line = &parts.points[start..end];
                    let width = (stroke.width_css(drawing) * vpr) as f32;
                    let style = stroke.line_style(drawing);
                    let stroke_color = stroke.color.unwrap_or(color);
                    let gap = if label_gap && line.len() == 2 {
                        self.segment_label_gap(drawing, px, pane_w_px, vpr, line[0], line[1])
                    } else {
                        None
                    };
                    // Dashed styles split into solid dash runs (the series' dash contract), so
                    // every executor paints the same dashes over the stroke's visible reach.
                    let mut push = |run: &[(f64, f64)]| {
                        push_clipped_stroke(
                            out,
                            points,
                            run,
                            context.pane,
                            (width, style, stroke_color),
                            &mut scratch,
                        );
                    };
                    match gap {
                        Some((gap_start, gap_end)) => {
                            let (a, b) = (line[0], line[1]);
                            push(&[a, point_on_segment(a, b, gap_start)]);
                            push(&[point_on_segment(a, b, gap_end), b]);
                        }
                        None => push(line),
                    }
                }
                DrawingPart::HLine { y, x0, x1, stroke } => {
                    let width = (stroke.width_css(drawing) * vpr).round().max(1.0) as i32;
                    let style = stroke.line_style(drawing);
                    let span = crisp_span(x0, x1, (0.0, f64::from(pane_w_px)), width, style);
                    if let Some((x0, x1)) = span.filter(|(x0, x1)| x0 != x1) {
                        out.push(Prim::HLine {
                            y: y.round() as i32,
                            x0,
                            x1,
                            width,
                            style,
                            color: stroke.color.unwrap_or(color),
                        });
                    }
                }
                DrawingPart::VLine { x, y0, y1, stroke } => {
                    let width = (stroke.width_css(drawing) * vpr).round().max(1.0) as i32;
                    let style = stroke.line_style(drawing);
                    let pane_span = (context.pane.top.floor(), context.pane.bottom.ceil());
                    if let Some((y0, y1)) = crisp_span(y0, y1, pane_span, width, style) {
                        out.push(Prim::VLine {
                            x: x.round() as i32,
                            y0,
                            y1,
                            width,
                            style,
                            color: stroke.color.unwrap_or(color),
                        });
                    }
                }
                DrawingPart::Fill {
                    upper,
                    lower,
                    count,
                    color: fill,
                    ..
                } => {
                    let upper_first = points.len() as u32;
                    points.extend(
                        parts.points[upper..upper + count]
                            .iter()
                            .map(|&(x, y)| [x as f32, y as f32]),
                    );
                    let lower_first = points.len() as u32;
                    points.extend(
                        parts.points[lower..lower + count]
                            .iter()
                            .map(|&(x, y)| [x as f32, y as f32]),
                    );
                    out.push(Prim::BandFill {
                        upper_first,
                        lower_first,
                        point_count: count as u32,
                        line_type: LineType::Simple,
                        fill: fill.unwrap_or(color),
                    });
                }
                DrawingPart::Disc {
                    center,
                    radius,
                    color: fill,
                } => {
                    let fill = fill.unwrap_or(color);
                    out.push(Prim::Circle {
                        cx: center.0 as f32,
                        cy: center.1 as f32,
                        radius: radius as f32,
                        fill,
                        stroke_width: 0.0,
                        stroke: fill,
                    });
                }
                DrawingPart::Label { index } => {
                    let label = &parts.labels[index];
                    let box_layout = label.layout(|line| {
                        self.measure_text_run(
                            line,
                            label.size,
                            &layout.font_family,
                            label.weight,
                            label.italic,
                        )
                    });
                    // Boxes off the pane (level labels of far tines) emit nothing.
                    if !box_layout.rect.intersects(&context.pane) {
                        continue;
                    }
                    let rect = IRect {
                        x: box_layout.rect.left.round() as i32,
                        y: box_layout.rect.top.round() as i32,
                        w: (box_layout.rect.right - box_layout.rect.left)
                            .round()
                            .max(1.0) as i32,
                        h: (box_layout.rect.bottom - box_layout.rect.top)
                            .round()
                            .max(1.0) as i32,
                    };
                    if let Some(background) = label.background {
                        out.push(Prim::Rect {
                            rect,
                            color: background,
                        });
                    }
                    if let Some(border) = label.border {
                        out.push(Prim::RectFrame {
                            rect,
                            border: vpr.round().max(1.0) as i32,
                            color: border,
                        });
                    }
                    let text_color = label
                        .color
                        .unwrap_or_else(|| self.drawing_label_color(drawing));
                    for (line_index, text) in label.lines.iter().enumerate() {
                        out.push(Prim::Text {
                            x: box_layout.text_x as f32,
                            y: (box_layout.first_y + line_index as f64 * box_layout.line_height)
                                as f32,
                            text: text.clone(),
                            color: text_color,
                            size: label.size as f32,
                            family: layout.font_family.clone(),
                            align: TextAlign::Left,
                            weight: label.weight,
                            italic: label.italic,
                        });
                    }
                    // The open native session's caret in a family or fork-form text box, in its
                    // shown blink phase: a 1 CSS px bar on the caret's own line, measured like the
                    // glyphs just painted. An empty box edits as one empty line, so row 0 at the
                    // line start is its caret slot.
                    if let (Some(text_part), Some(session)) = (
                        parts.text.filter(|text| text.label == index),
                        self.drawing_text_edit.as_ref().filter(|session| {
                            session.id == drawing.id && session.paint_caret && session.caret_shown
                        }),
                    ) {
                        let prefix: String = session.text.chars().take(session.caret).collect();
                        let row = prefix.matches('\n').count();
                        let line = prefix.rsplit('\n').next().unwrap_or("");
                        // Browser caret parity: the next whole CSS px after the prefix run.
                        let advance_css = self.measure_text_run(
                            line,
                            label.size,
                            &layout.font_family,
                            label.weight,
                            label.italic,
                        ) / vpr;
                        push_caret_bar(
                            out,
                            points,
                            CaretBar {
                                x: box_layout.text_x,
                                y: box_layout.first_y
                                    + (text_part.first_line + row) as f64 * box_layout.line_height,
                                local_x: advance_css.ceil() * vpr,
                                half_height: label.size * 0.6,
                                angle: 0.0,
                            },
                            vpr,
                            text_color,
                        );
                    }
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn build_profile_drawing_prims(
        &self,
        drawing: &Drawing,
        px: &[(f64, f64)],
        pane_w_px: i32,
        vpr: f64,
        out: &mut Vec<Prim>,
        points: &mut Vec<[f32; 2]>,
    ) {
        let Ok(snapshot) = self.profile_drawing_snapshot_for_frame(drawing.id) else {
            return;
        };
        match snapshot {
            crate::ProfileDrawingSnapshot::Volume(profile) => {
                let Some(options) = drawing.profile.as_ref() else {
                    return;
                };
                let left = px
                    .first()
                    .map_or(0.0, |point| point.0)
                    .clamp(0.0, f64::from(pane_w_px));
                let right = match drawing.kind {
                    DrawingKind::FixedRangeVolumeProfile => px
                        .get(1)
                        .map_or(left, |point| point.0)
                        .clamp(0.0, f64::from(pane_w_px)),
                    _ => f64::from(pane_w_px),
                };
                let range_left = left.min(right);
                let range_right = left.max(right);
                let available = (range_right - range_left).max(1.0) * options.width_percent / 100.0;
                let max_volume = profile
                    .rows
                    .iter()
                    .map(|row| row.total_volume)
                    .fold(0.0_f64, f64::max);
                if max_volume <= 0.0 {
                    return;
                }
                let bid = Color::rgba(247, 82, 95, 150);
                let ask = Color::rgba(8, 153, 129, 150);
                let unknown = Color::rgba(120, 130, 145, 130);
                for row in &profile.rows {
                    let Some((_, y0)) = self.drawing_to_px_for(
                        drawing.pane_index,
                        drawing.price_scale,
                        crate::DrawingPoint {
                            logical: drawing.points[0].logical,
                            price: row.low,
                        },
                    ) else {
                        continue;
                    };
                    let Some((_, y1)) = self.drawing_to_px_for(
                        drawing.pane_index,
                        drawing.price_scale,
                        crate::DrawingPoint {
                            logical: drawing.points[0].logical,
                            price: row.high,
                        },
                    ) else {
                        continue;
                    };
                    let width = available * row.total_volume / max_volume;
                    let x0 = range_right - width;
                    let height = ((y0 - y1).abs() * vpr).round().max(1.0) as i32;
                    let y = (y0.min(y1) * vpr).round() as i32;
                    let mut cursor = x0;
                    for (volume, color) in [
                        (row.bid_volume, bid),
                        (row.unknown_volume, unknown),
                        (row.ask_volume, ask),
                    ] {
                        if volume <= 0.0 {
                            continue;
                        }
                        let segment = width * volume / row.total_volume;
                        out.push(Prim::Rect {
                            rect: IRect {
                                x: (cursor * vpr).round() as i32,
                                y,
                                w: (segment * vpr).round().max(1.0) as i32,
                                h: height,
                            },
                            color,
                        });
                        cursor += segment;
                    }
                }
                if let Some(poc) = profile
                    .poc
                    .and_then(|price| {
                        self.drawing_to_px_for(
                            drawing.pane_index,
                            drawing.price_scale,
                            crate::DrawingPoint {
                                logical: drawing.points[0].logical,
                                price,
                            },
                        )
                    })
                    .map(|(_, y)| y * vpr)
                {
                    out.push(Prim::HLine {
                        y: poc.round() as i32,
                        x0: ((range_right - available) * vpr).round() as i32,
                        x1: (range_right * vpr).round() as i32,
                        width: vpr.round().max(1.0) as i32,
                        style: LineStyle::Solid,
                        color: Color::rgb(245, 166, 35),
                    });
                }
            }
            crate::ProfileDrawingSnapshot::Vwap(values) => {
                let mut center = Vec::with_capacity(values.len());
                let mut upper = Vec::with_capacity(values.len());
                let mut lower = Vec::with_capacity(values.len());
                for value in values {
                    let seconds = value.timestamp_micros.div_euclid(1_000_000) as f64;
                    let Some(logical) = self.time_to_index(seconds, true).map(|index| index as f64)
                    else {
                        continue;
                    };
                    for (price, target) in [
                        (value.vwap, &mut center),
                        (value.upper_band, &mut upper),
                        (value.lower_band, &mut lower),
                    ] {
                        if let Some((x, y)) = self.drawing_to_px_for(
                            drawing.pane_index,
                            drawing.price_scale,
                            crate::DrawingPoint { logical, price },
                        ) {
                            target.push([x as f32 * vpr as f32, y as f32 * vpr as f32]);
                        }
                    }
                }
                let color = drawing.stroke_color();
                push_line_stroke(
                    out,
                    points,
                    &center,
                    (drawing.width * vpr) as f32,
                    drawing.style,
                    LineType::Simple,
                    color,
                );
                let band = Color::rgba(color.r(), color.g(), color.b(), color.a().min(150));
                for path in [&upper, &lower] {
                    push_line_stroke(
                        out,
                        points,
                        path,
                        vpr.max(1.0) as f32,
                        LineStyle::Dashed,
                        LineType::Simple,
                        band,
                    );
                }
            }
        }
    }

    /// The engine-painted caret of an open drawing text session (native hosts). It sits on the
    /// label's own transform: the run's measured advance places it, and it rotates with a trend
    /// label. An empty run uses the one-em editing slot the middle-line cutout reserves.
    fn build_drawing_text_caret(
        &self,
        drawing: &Drawing,
        px: &[(f64, f64)],
        pane_w_px: i32,
        vpr: f64,
        out: &mut Vec<Prim>,
        points: &mut Vec<[f32; 2]>,
    ) {
        let Some(session) = self.drawing_text_edit.as_ref().filter(|session| {
            session.id == drawing.id && session.paint_caret && session.caret_shown
        }) else {
            return;
        };
        // A family text box paints its caret with its label (`build_family_prims`): falling
        // through to the run placement would put the bar where the generic label would sit.
        if !drawing.kind.paints_generic_text() || projection_annotations::fork_text_owner(drawing) {
            return;
        }
        let text = drawing.display_text();
        if let Some(layout) = self.annotation_layout(drawing, px, vpr) {
            // A box annotation: the caret's own row of the shared layout, after its prefix.
            let prefix: String = text.chars().take(session.caret).collect();
            let (row, line) = if layout.lines > 1 {
                let row = prefix.matches('\n').count();
                (row, prefix.rsplit('\n').next().unwrap_or(""))
            } else {
                (0, prefix.as_str())
            };
            let prefix_px = self.measure_text_run(
                line,
                layout.size,
                &self.options.get().layout.font_family,
                annotation_text_weight(drawing),
                drawing.text_italic,
            );
            push_caret_bar(
                out,
                points,
                CaretBar {
                    x: layout.text_x,
                    y: layout.text_y + row as f64 * layout.line_height,
                    local_x: (prefix_px / vpr).ceil() * vpr,
                    half_height: layout.size * 0.6,
                    angle: 0.0,
                },
                vpr,
                self.annotation_paint(drawing).text_color(drawing, false),
            );
            return;
        }
        let (size, x, y, align, angle) = self.text_run_geometry(drawing, px, pane_w_px, vpr);
        if drawing.text_block_lines() > 1 {
            // Upstream's text block: the caret's own line, measured from the line start.
            let block = TextBlock::new(
                (x, y, align),
                drawing.text_v_align,
                self.measure_drawing_text(drawing, size),
                size,
                drawing.text_block_lines(),
            );
            let prefix: String = text.chars().take(session.caret).collect();
            let row = prefix.matches('\n').count();
            let line = prefix.rsplit('\n').next().unwrap_or("");
            let prefix_css = self.measure_drawing_frame_text(drawing, line, size) / vpr;
            let color = self.drawing_label_color(drawing);
            push_caret_bar(
                out,
                points,
                CaretBar {
                    x: block.left,
                    y: block.line_y(row),
                    local_x: prefix_css.ceil() * vpr,
                    half_height: size * 0.6,
                    angle: 0.0,
                },
                vpr,
                color,
            );
            return;
        }
        let advance = if text.is_empty() {
            size
        } else {
            self.measure_drawing_frame_text(drawing, text, size)
        };
        let prefix: String = text.chars().take(session.caret).collect();
        let start = match align {
            DrawingTextHAlign::Left => 0.0,
            DrawingTextHAlign::Center => -advance / 2.0,
            DrawingTextHAlign::Right => -advance,
        };
        // Browser caret parity: a 1 CSS px bar at the next whole CSS px after the prefix run,
        // spanning the label's 1.2em line box, rotated with the label.
        let prefix_css = self.measure_drawing_frame_text(drawing, &prefix, size) / vpr;
        let local_x = start + prefix_css.ceil() * vpr;
        let half_height = size * 0.6;
        let color = self.drawing_label_color(drawing);
        push_caret_bar(
            out,
            points,
            CaretBar {
                x,
                y,
                local_x,
                half_height,
                angle,
            },
            vpr,
            color,
        );
    }

    /// The text run's resolved glyph size (bitmap px, placeholder floor included), aligned
    /// anchor point, and horizontal alignment — shared by the label prim, the container box,
    /// and the focus/hover chrome so every consumer draws the same geometry.
    fn text_run_geometry(
        &self,
        drawing: &Drawing,
        px: &[(f64, f64)],
        pane_w_px: i32,
        vpr: f64,
    ) -> (f64, f64, f64, DrawingTextHAlign, f64) {
        if let Some(annotation) = self.annotation_layout(drawing, px, vpr) {
            return (
                annotation.size,
                annotation.text_x,
                annotation.text_y,
                DrawingTextHAlign::Left,
                0.0,
            );
        }
        let layout = &self.options.get().layout;
        let size = drawing.resolved_text_size(layout.font_size) * vpr;
        let pane = &self.panes[drawing.pane_index];
        let (x, y, align, angle) = ChartEngine::drawing_text_placement(
            drawing,
            px,
            f64::from(pane_w_px),
            pane.top * vpr,
            pane.height * vpr,
            size,
            vpr,
        );
        (size, x, y, align, angle)
    }

    /// The painted box of a fork-form annotation's text (`fork_text_box`) at bitmap-px anchors
    /// `px`; `None` while it paints none (a fork-form note's box outside focus).
    fn fork_text_rect(
        &self,
        drawing: &Drawing,
        px: &[(f64, f64)],
        pane_w_px: i32,
        vpr: f64,
    ) -> Option<aeris_charts_render::shape::Rect> {
        let context = self.frame_part_context(drawing, px, pane_w_px, vpr)?;
        let mut parts = DrawingParts::default();
        projection_annotations::fork_text_box(&context, &mut parts);
        let label = parts.labels.get(parts.text?.label)?;
        let family = &self.options.get().layout.font_family;
        let layout = label.layout(|line| {
            self.measure_text_run(line, label.size, family, label.weight, label.italic)
        });
        Some(layout.rect)
    }

    /// The text tool's interaction chrome (hover ring, focus border): a crisp integer-snapped
    /// hollow frame on the SAME box the host's editing wrap draws — the label run (advance ×
    /// 1.2·size, the hit test's line-height convention) padded by the editing chrome's
    /// 2 px border + 4 px padding (drawings.rs `TEXT_CHROME_PAD`). Selection, hover, and
    /// typing mode land on one outline, so entering/leaving the editor moves nothing.
    fn push_text_chrome(
        &self,
        drawing: &Drawing,
        px: &[(f64, f64)],
        pane_w_px: i32,
        vpr: f64,
        color: Color,
        out: &mut Vec<Prim>,
    ) {
        let rect = if projection_annotations::fork_text_owner(drawing) {
            // A fork-form box: its painted box grown by the editing border.
            let Some(rect) = self.fork_text_rect(drawing, px, pane_w_px, vpr) else {
                return;
            };
            let grow = (TEXT_CHROME_PAD - TEXT_PAD) * vpr;
            IRect {
                x: (rect.left - grow).round() as i32,
                y: (rect.top - grow).round() as i32,
                w: (rect.right - rect.left + 2.0 * grow).round().max(1.0) as i32,
                h: (rect.bottom - rect.top + 2.0 * grow).round().max(1.0) as i32,
            }
        } else {
            let (size, x, y, align, _) = self.text_run_geometry(drawing, px, pane_w_px, vpr);
            let block = TextBlock::new(
                (x, y, align),
                drawing.text_v_align,
                self.measure_drawing_text(drawing, size),
                size,
                drawing.text_block_lines(),
            );
            let pad = TEXT_CHROME_PAD * vpr;
            IRect {
                x: (block.left - pad).round() as i32,
                y: (block.top() - pad).round() as i32,
                w: (block.width + 2.0 * pad).round().max(1.0) as i32,
                h: (block.height() + 2.0 * pad).round().max(1.0) as i32,
            }
        };
        out.push(Prim::RectFrame {
            rect,
            border: (2.0 * vpr).round().max(1.0) as i32,
            color,
        });
    }

    /// One drawing's text label (every tool can carry one): the placement resolves the 3×3
    /// alignment against the tool's reference box, except trend lines, whose slots follow the
    /// actual segment and whose middle slot opens a measured stroke gap. Trend labels emit
    /// `Prim::RotatedText`; other drawing text emits `Prim::Text`. In either contract x is the
    /// aligned edge and y is the vertical center (the IR's middle-baseline convention). Empty
    /// standalone text paints nothing; an empty hovered trend label paints its dedicated
    /// prompt at the canonical label transform. A text tool
    /// with a `box_color`/`box_border_color` gets its container (crisp integer-snapped
    /// `Rect`/`RectFrame` prims behind the run — the public reference's text-box background/border).
    fn build_drawing_text(
        &self,
        drawing: &Drawing,
        px: &[(f64, f64)],
        pane_w_px: i32,
        vpr: f64,
        out: &mut Vec<Prim>,
        points: &mut Vec<[f32; 2]>,
    ) {
        // Empty text paints nothing. While the host typing-mode editor is open the LABEL and
        // the focus border still paint — the editor wrap is borderless with transparent glyphs,
        // so entering edit cannot lift the text or shift the outline (the public reference's
        // overlay-caret model). Families that own their text lay it out in their parts.
        if drawing
            .kind
            .spec()
            .family
            .is_some_and(|family| family.owns_text)
        {
            return;
        }
        // A fork-form annotation's text is its box (`fork_text_box`), with the open native
        // session's caret.
        if projection_annotations::fork_text_owner(drawing) {
            if let Some(context) = self.frame_part_context(drawing, px, pane_w_px, vpr) {
                self.push_parts(
                    &context,
                    pane_w_px,
                    vpr,
                    out,
                    points,
                    projection_annotations::fork_text_box,
                );
            }
            return;
        }
        // Box annotations paint their own text with their box (`build_annotation_prims`).
        if drawing.kind.is_annotation() {
            return;
        }
        let Some((text, placeholder)) = self.drawing_frame_text(drawing) else {
            return;
        };
        let is_text_tool = drawing.kind == DrawingKind::Text || drawing.text_annotation();
        let (size, x, y, align, angle) = self.text_run_geometry(drawing, px, pane_w_px, vpr);
        let layout = &self.options.get().layout;
        let mut color = self.drawing_label_color(drawing);
        if placeholder {
            color = Color::rgba(
                color.r(),
                color.g(),
                color.b(),
                color.a().min(TREND_TEXT_PLACEHOLDER_ALPHA),
            );
        }

        // The container (text tool with a background/border): a box wrapping the run, emitted
        // as the rectangle tool's crisp integer-snapped prims (`Rect` fill + `RectFrame`
        // border) — strong-color thin geometry at fractional positions AA-phases differently
        // between the backends, so the box snaps to whole device px (the public reference's boxes are
        // crisp the same way).
        let box_fill = drawing.box_color.as_deref().and_then(Color::parse_css);
        let box_border = drawing
            .box_border_color
            .as_deref()
            .and_then(Color::parse_css);
        // A text annotation's text of several lines stacks into upstream's text block.
        let lines = drawing.text_block_lines();
        let block = (lines > 1 || (is_text_tool && (box_fill.is_some() || box_border.is_some())))
            .then(|| {
                TextBlock::new(
                    (x, y, align),
                    drawing.text_v_align,
                    self.measure_drawing_text(drawing, size),
                    size,
                    lines,
                )
            });
        if let (true, Some(block)) = (
            is_text_tool && (box_fill.is_some() || box_border.is_some()),
            block,
        ) {
            let pad = 4.0 * vpr;
            let rect = IRect {
                x: (block.left - pad).round() as i32,
                y: (block.top() - pad).round() as i32,
                w: (block.width + 2.0 * pad).round().max(1.0) as i32,
                h: (block.height() + 2.0 * pad).round().max(1.0) as i32,
            };
            if let Some(fill) = box_fill {
                out.push(Prim::Rect { rect, color: fill });
            }
            if let Some(border) = box_border {
                out.push(Prim::RectFrame {
                    rect,
                    // Browser border semantics: whole device pixels, rounded down, at least one.
                    border: (drawing.box_border_width * vpr).floor().max(1.0) as i32,
                    color: border,
                });
            }
        }

        if let Some(block) = block.filter(|block| block.lines > 1) {
            // One left-aligned run per line, at the left edge the multi-line editor shares.
            for (index, line) in text.split('\n').enumerate() {
                out.push(Prim::Text {
                    x: block.left as f32,
                    y: block.line_y(index) as f32,
                    text: line.trim_end_matches('\r').to_string(),
                    color,
                    size: size as f32,
                    family: layout.font_family.clone(),
                    align: TextAlign::Left,
                    weight: drawing.text_weight.unwrap_or(400),
                    italic: drawing.text_italic,
                });
            }
            return;
        }
        let text_prim = Prim::RotatedText {
            x: x as f32,
            y: y as f32,
            text: text.to_string(),
            color,
            size: size as f32,
            family: layout.font_family.clone(),
            align: match align {
                DrawingTextHAlign::Left => TextAlign::Left,
                DrawingTextHAlign::Center => TextAlign::Center,
                DrawingTextHAlign::Right => TextAlign::Right,
            },
            weight: drawing.text_weight.unwrap_or(400),
            italic: drawing.text_italic,
            angle: angle as f32,
        };
        if drawing.kind.spec().text_layout == DrawingTextLayout::Segment {
            out.push(text_prim);
        } else if let Prim::RotatedText {
            x,
            y,
            text,
            color,
            size,
            family,
            align,
            weight,
            italic,
            ..
        } = text_prim
        {
            out.push(Prim::Text {
                x,
                y,
                text,
                color,
                size,
                family,
                align,
                weight,
                italic,
            });
        }
    }

    fn build_drawing_labels(
        &self,
        drawing: &Drawing,
        px: &[(f64, f64)],
        vpr: f64,
        out: &mut Vec<Prim>,
    ) {
        let Some(anchor) = px.first().copied() else {
            return;
        };
        // These kinds lay their metric labels out with their geometry (build_drawing_prims).
        if drawing.labels.is_empty()
            || drawing
                .kind
                .spec()
                .family
                .is_some_and(|family| family.owns_labels)
            || lines::fork_presentation(drawing)
            || projection_annotations::draws_sector(drawing)
            || matches!(
                drawing.kind,
                DrawingKind::TrendAngle | DrawingKind::InfoLine
            )
        {
            return;
        }
        let color = drawing
            .text_color
            .as_deref()
            .and_then(Color::parse_css)
            .or_else(|| Color::parse_css(&drawing.color))
            .unwrap_or_else(|| Color::rgb(255, 255, 255));
        let size = drawing.resolved_text_size(self.options.get().layout.font_size) * vpr;
        for (index, label) in drawing.labels.iter().enumerate() {
            if !label.visible {
                continue;
            }
            let value = label.text.clone().unwrap_or_else(|| {
                let first = drawing.points.first().map_or(0.0, |point| point.price);
                let second = drawing.points.get(1).map(|point| point.price);
                match label.metric {
                    crate::DrawingLabelMetric::Price => self.format_drawing_price(drawing, first),
                    crate::DrawingLabelMetric::PriceChange => second
                        .map(|value| {
                            crate::drawings::unsigned_zero(
                                self.format_drawing_price(drawing, value - first),
                            )
                        })
                        .unwrap_or_default(),
                    crate::DrawingLabelMetric::PercentChange => second
                        .filter(|_| first.abs() > f64::EPSILON)
                        .map(|value| format!("{:.2}%", (value - first) / first * 100.0))
                        .unwrap_or_default(),
                    crate::DrawingLabelMetric::Ticks => second
                        .map(|value| format!("{:.4}", value - first))
                        .unwrap_or_default(),
                    crate::DrawingLabelMetric::BarCount => drawing
                        .points
                        .get(1)
                        .map(|value| {
                            format!(
                                "{} bars",
                                (value.logical - drawing.points[0].logical).abs().round() as i64
                            )
                        })
                        .unwrap_or_default(),
                    // The anchors' times through the bar label, as the own line's stats box
                    // printed them; upstream's placeholder text where the axis has no time.
                    crate::DrawingLabelMetric::DateTimeRange => self
                        .drawing_metric_text(drawing, label.metric, 0, 1)
                        .unwrap_or_else(|| "range".to_string()),
                    crate::DrawingLabelMetric::Duration => self
                        .drawing_metric_text(drawing, label.metric, 0, 1)
                        .or_else(|| {
                            drawing.points.get(1).map(|value| {
                                format!(
                                    "{:.2} bars",
                                    (value.logical - drawing.points[0].logical).abs()
                                )
                            })
                        })
                        .unwrap_or_default(),
                    crate::DrawingLabelMetric::Angle => px
                        .get(1)
                        .map(|value| {
                            let dx = value.0 - px[0].0;
                            let dy = px[0].1 - value.1;
                            format!("{:.1}°", dy.atan2(dx).to_degrees())
                        })
                        .unwrap_or_default(),
                    crate::DrawingLabelMetric::Distance => drawing
                        .points
                        .get(1)
                        .map(|value| {
                            format!(
                                "{:.2}",
                                (value.logical - drawing.points[0].logical)
                                    .hypot(value.price - first)
                            )
                        })
                        .unwrap_or_default(),
                    crate::DrawingLabelMetric::VolumeInRange => "volume".to_string(),
                }
            });
            if value.is_empty() {
                continue;
            }
            let offset = (index as f64 + 1.0) * size * 1.25;
            let y = match label.position {
                crate::DrawingLabelPosition::Above => anchor.1 - offset,
                crate::DrawingLabelPosition::Below => anchor.1 + offset,
                crate::DrawingLabelPosition::Inside | crate::DrawingLabelPosition::On => anchor.1,
                crate::DrawingLabelPosition::Outside => anchor.1 + offset,
            };
            out.push(Prim::Text {
                x: anchor.0 as f32,
                y: y as f32,
                text: value,
                color,
                size: size as f32,
                family: self.options.get().layout.font_family.clone(),
                align: TextAlign::Left,
                weight: drawing.text_weight.unwrap_or(400),
                italic: drawing.text_italic,
            });
        }
    }

    /// The visible label of `metric`: `Some(custom text)` when shown, `None` when hidden/absent.
    fn visible_label(
        drawing: &Drawing,
        metric: crate::DrawingLabelMetric,
    ) -> Option<Option<String>> {
        drawing
            .labels
            .iter()
            .find(|label| label.metric == metric && label.visible)
            .map(|label| label.text.clone())
    }

    /// Trend Angle without a `line` block: a dotted horizontal reference from the first anchor,
    /// a dotted arc sweeping from it to the line, and the signed angle (counter-clockwise
    /// positive, as on a y-up chart; the shared metric text) beside the reference's end.
    fn build_trend_angle_prims(
        &self,
        drawing: &Drawing,
        px: &[(f64, f64)],
        color: Color,
        vpr: f64,
        out: &mut Vec<Prim>,
        points: &mut Vec<[f32; 2]>,
    ) {
        let (Some(&start), Some(&end)) = (px.first(), px.get(1)) else {
            return;
        };
        let (dx, dy) = (end.0 - start.0, end.1 - start.1);
        if dx.hypot(dy) <= f64::EPSILON {
            return;
        }
        let radius = lines::TREND_ANGLE_RADIUS_CSS * vpr;
        let width = vpr.round().max(1.0) as f32;
        let sweep = dy.atan2(dx);
        let first_point = points.len() as u32;
        points.extend([
            [start.0 as f32, start.1 as f32],
            [(start.0 + radius) as f32, start.1 as f32],
        ]);
        out.push(Prim::Polyline {
            first_point,
            point_count: 2,
            width,
            style: LineStyle::Dotted,
            line_type: LineType::Simple,
            color,
        });
        let steps = arc_segments(radius, sweep);
        let first_point = points.len() as u32;
        for step in 0..=steps {
            let angle = sweep * f64::from(step) / f64::from(steps);
            points.push([
                (start.0 + radius * angle.cos()) as f32,
                (start.1 + radius * angle.sin()) as f32,
            ]);
        }
        out.push(Prim::Polyline {
            first_point,
            point_count: steps + 1,
            width,
            style: LineStyle::Dotted,
            line_type: LineType::Simple,
            color,
        });
        let Some(text) =
            Self::visible_label(drawing, crate::DrawingLabelMetric::Angle).and_then(|custom| {
                custom.or_else(|| {
                    self.drawing_metric_text(drawing, crate::DrawingLabelMetric::Angle, 0, 1)
                })
            })
        else {
            return;
        };
        let layout = &self.options.get().layout;
        out.push(Prim::Text {
            x: (start.0 + radius + lines::TREND_ANGLE_LABEL_GAP_CSS * vpr) as f32,
            y: start.1 as f32,
            text,
            color,
            size: (self.drawing_stats_size() * vpr) as f32,
            family: layout.font_family.clone(),
            align: TextAlign::Left,
            weight: drawing.text_weight.unwrap_or(400),
            italic: drawing.text_italic,
        });
    }

    /// The Info Line's statistics card rows, each listing only its visible metrics in the shared
    /// metric text (`ChartEngine::drawing_metric_text`): price change (percent), ticks; bars
    /// (elapsed), pixel distance; angle. A label's own `text` replaces its metric value; a
    /// missing percent or tick count prints "—" and an elapsed time without time data is
    /// omitted. A visible `DateTimeRange` label stands in for a hidden `Duration` one.
    fn info_line_rows(&self, drawing: &Drawing) -> Vec<(InfoLineIcon, String)> {
        use crate::DrawingLabelMetric as Metric;
        let metric = |metric| self.drawing_metric_text(drawing, metric, 0, 1);
        let mut rows = Vec::with_capacity(3);

        let mut price = String::new();
        if let Some(custom) = Self::visible_label(drawing, Metric::PriceChange) {
            price = custom
                .or_else(|| metric(Metric::PriceChange))
                .unwrap_or_default();
        }
        if let Some(custom) = Self::visible_label(drawing, Metric::PercentChange) {
            let percent = custom
                .or_else(|| metric(Metric::PercentChange))
                .unwrap_or_else(|| "—".to_string());
            price = if price.is_empty() {
                percent
            } else {
                format!("{price} ({percent})")
            };
        }
        if let Some(custom) = Self::visible_label(drawing, Metric::Ticks) {
            let ticks = custom
                .or_else(|| metric(Metric::Ticks))
                .unwrap_or_else(|| "—".to_string());
            price = if price.is_empty() {
                ticks
            } else {
                format!("{price}, {ticks}")
            };
        }
        if !price.is_empty() {
            rows.push((InfoLineIcon::Price, price));
        }

        let mut time = String::new();
        if let Some(custom) = Self::visible_label(drawing, Metric::BarCount) {
            time = custom
                .or_else(|| metric(Metric::BarCount))
                .unwrap_or_default();
        }
        let duration = Self::visible_label(drawing, Metric::Duration)
            .or_else(|| Self::visible_label(drawing, Metric::DateTimeRange));
        if let Some(custom) = duration
            && let Some(elapsed) = custom.or_else(|| metric(Metric::Duration))
        {
            time = if time.is_empty() {
                elapsed
            } else {
                format!("{time} ({elapsed})")
            };
        }
        if let Some(custom) = Self::visible_label(drawing, Metric::Distance)
            && let Some(distance) =
                custom.or_else(|| metric(Metric::Distance).map(|text| format!("distance: {text}")))
        {
            time = if time.is_empty() {
                distance
            } else {
                format!("{time}, {distance}")
            };
        }
        if !time.is_empty() {
            rows.push((InfoLineIcon::Time, time));
        }

        if let Some(custom) = Self::visible_label(drawing, Metric::Angle)
            && let Some(angle) = custom.or_else(|| metric(Metric::Angle))
        {
            rows.push((InfoLineIcon::Angle, angle));
        }
        rows
    }

    /// A box annotation from the shared layout (annotations.rs): its connector, the rounded box
    /// (the callout's box and tail as one outlined shape), then the text, one run per line, in
    /// the shared colors (`ChartEngine::annotation_paint`).
    fn build_annotation_prims(
        &self,
        drawing: &Drawing,
        px: &[(f64, f64)],
        color: Color,
        vpr: f64,
        out: &mut Vec<Prim>,
        points: &mut Vec<[f32; 2]>,
    ) {
        let Some(layout) = self.annotation_layout(drawing, px, vpr) else {
            return;
        };
        let paint = self.annotation_paint(drawing);
        let (fill, border) = (paint.fill, paint.border);
        let border_width = if drawing.kind == DrawingKind::Callout {
            (2.0 * vpr).round().max(1.0)
        } else {
            (drawing.box_border_width * vpr).floor().max(1.0)
        };
        let mut stroke = |a: (f64, f64), b: (f64, f64), width: f64, color: Color| {
            let first_point = points.len() as u32;
            points.extend([[a.0 as f32, a.1 as f32], [b.0 as f32, b.1 as f32]]);
            out.push(Prim::Polyline {
                first_point,
                point_count: 2,
                width: width as f32,
                style: drawing.style,
                line_type: LineType::Simple,
                color,
            });
        };
        if let Some((a, b)) = layout.connector {
            stroke(a, b, drawing.width * vpr, color);
        }
        let [left, top, width, height] = layout.rect;
        if let Some(outline) = &layout.outline {
            // The callout is one shape: the box fill, the tail fill tucked into the box so the two
            // surfaces overlap instead of meeting at an anti-aliased seam, then one closed
            // outline stroked around both, so the border runs into the tail without a break.
            out.push(Prim::RoundRect {
                x: left as f32,
                y: top as f32,
                w: width as f32,
                h: height as f32,
                radii: layout.radii.map(|radius| radius as f32),
                fill,
                border_width: 0.0,
                border_color: fill,
            });
            if let Some([tip, base_a, base_b]) = layout.tail {
                let center = (left + width / 2.0, top + height / 2.0);
                let tuck = |(x, y): (f64, f64)| {
                    let (dx, dy) = (center.0 - x, center.1 - y);
                    let distance = dx.hypot(dy).max(f64::EPSILON);
                    let inset = border_width + vpr;
                    [
                        (x + dx / distance * inset) as f32,
                        (y + dy / distance * inset) as f32,
                    ]
                };
                out.push(Prim::Triangle {
                    a: [tip.0 as f32, tip.1 as f32],
                    b: tuck(base_a),
                    c: tuck(base_b),
                    color: fill,
                });
            }
            let first_point = points.len() as u32;
            points.extend(outline.iter().map(|&(x, y)| [x as f32, y as f32]));
            out.push(Prim::Polyline {
                first_point,
                point_count: outline.len() as u32,
                width: border_width as f32,
                style: LineStyle::Solid,
                line_type: LineType::Simple,
                color: border.unwrap_or(color),
            });
        } else {
            // The box snaps to whole device pixels so its edges stay crisp on every backend.
            let (x0, y0) = (left.round(), top.round());
            let (x1, y1) = ((left + width).round(), (top + height).round());
            out.push(Prim::RoundRect {
                x: x0 as f32,
                y: y0 as f32,
                w: (x1 - x0).max(1.0) as f32,
                h: (y1 - y0).max(1.0) as f32,
                radii: layout.radii.map(|radius| radius.round() as f32),
                fill,
                border_width: if border.is_some() {
                    border_width as f32
                } else {
                    0.0
                },
                border_color: border.unwrap_or(fill),
            });
        }
        let text_color = paint.text_color(drawing, layout.placeholder);
        // One run per line, left-aligned at the box's text start (owner decision R9).
        let family = &self.options.get().layout.font_family;
        for (y, line) in layout.rows() {
            out.push(Prim::Text {
                x: layout.text_x as f32,
                y: y as f32,
                text: line.to_string(),
                color: text_color,
                size: layout.size as f32,
                family: family.clone(),
                align: TextAlign::Left,
                weight: annotation_text_weight(drawing),
                italic: drawing.text_italic,
            });
        }
    }

    /// The Info Line's statistics card without a `line` block: a rounded panel beside the
    /// segment's midpoint, on the side the line leaves free (above-right of a falling line,
    /// below-right of a rising one), kept inside the pane.
    fn build_info_line_prims(
        &self,
        drawing: &Drawing,
        px: &[(f64, f64)],
        pane_w_px: i32,
        vpr: f64,
        out: &mut Vec<Prim>,
        points: &mut Vec<[f32; 2]>,
    ) {
        let (Some(&a), Some(&b)) = (px.first(), px.get(1)) else {
            return;
        };
        let Some(pane) = self.panes.get(drawing.pane_index) else {
            return;
        };
        let rows = self.info_line_rows(drawing);
        if rows.is_empty() {
            return;
        }
        let layout = &self.options.get().layout;
        let card = ChromeTokens::for_theme(self.surface_theme());
        let size = INFO_LINE_FONT_CSS * vpr;
        let line_height = INFO_LINE_LINE_HEIGHT_CSS * vpr;
        let row_gap = INFO_LINE_ROW_GAP_CSS * vpr;
        let pad_x = INFO_LINE_PAD_X_CSS * vpr;
        let (pad_top, pad_bottom) = (INFO_LINE_PAD_TOP_CSS * vpr, INFO_LINE_PAD_BOTTOM_CSS * vpr);
        let icon = INFO_LINE_ICON_CSS * vpr;
        let text_inset = pad_x + icon + INFO_LINE_ICON_GAP_CSS * vpr;
        let text_width = rows
            .iter()
            .map(|(_, text)| self.measure_text_run(text, size, &layout.font_family, 400, false))
            .fold(0.0_f64, f64::max);
        let width = (text_inset + text_width + pad_x).round();
        let height = (pad_top
            + rows.len() as f64 * line_height
            + (rows.len() - 1) as f64 * row_gap
            + pad_bottom)
            .round();
        let mid = point_on_segment(a, b, 0.5);
        let gap_x = 12.0 * vpr;
        let gap_y = 16.0 * vpr;
        let pane_top = pane.top * vpr;
        let pane_bottom = (pane.top + pane.height) * vpr;
        let left = (mid.0 + gap_x)
            .min(f64::from(pane_w_px) - width)
            .max(0.0)
            .round();
        let top = if (b.0 - a.0) * (b.1 - a.1) >= 0.0 {
            mid.1 - gap_y - height
        } else {
            mid.1 + gap_y
        }
        .min(pane_bottom - height)
        .max(pane_top)
        .round();
        out.push(Prim::RoundRect {
            x: left as f32,
            y: top as f32,
            w: width as f32,
            h: height as f32,
            radii: [(aeris_charts_core::style::RADIUS_DEFAULT * vpr).round() as f32; 4],
            fill: card.surface,
            border_width: aeris_charts_core::style::border_width_device_px(vpr) as f32,
            border_color: card.border,
        });
        for (index, (kind, text)) in rows.into_iter().enumerate() {
            let center_y =
                top + pad_top + (line_height + row_gap) * index as f64 + line_height / 2.0;
            push_info_line_icon(
                kind,
                (left + pad_x + icon / 2.0, center_y),
                card.muted,
                vpr,
                out,
                points,
            );
            out.push(Prim::Text {
                x: (left + text_inset) as f32,
                y: center_y as f32,
                text,
                color: card.foreground,
                size: size as f32,
                family: layout.font_family.clone(),
                align: TextAlign::Left,
                weight: 400,
                italic: false,
            });
        }
    }

    /// A price-valued drawing text in the bound scale's price format through
    /// [`ChartEngine::format_scale_price`]: the one owner for every price a drawing prints
    /// (level and price labels, metrics, stats boxes, positions, ranges, Fibonacci, Gann).
    pub(crate) fn format_drawing_price(&self, drawing: &Drawing, value: f64) -> String {
        self.format_scale_price(drawing.pane_index, drawing.price_scale.target(), value)
    }

    /// The price label's bubble in the anchor's px basis (`scale` is the bitmap/media ratio;
    /// 1.0 at hit-test). The bubble rises above-right of the anchor and its tail's tip touches
    /// the anchor, so the label and its handle are one attached shape.
    pub(crate) fn price_label_layout(
        &self,
        drawing: &Drawing,
        anchor: (f64, f64),
        scale: f64,
    ) -> PriceLabelLayout {
        let text = if drawing.text.is_empty() {
            self.format_drawing_price(drawing, drawing.points[0].price)
        } else {
            drawing.text.clone()
        };
        let layout = &self.options.get().layout;
        let size = drawing.resolved_text_size(layout.font_size) * scale;
        let text_width = self.measure_text_run(
            &text,
            size,
            &layout.font_family,
            drawing.text_weight.unwrap_or(400),
            drawing.text_italic,
        );
        let (padding, tail, height) = projection_annotations::price_label_bubble(size, scale);
        PriceLabelLayout {
            rect: [
                anchor.0,
                anchor.1 - tail - height,
                text_width + 2.0 * padding,
                height,
            ],
            text,
            size,
            padding,
            tail,
        }
    }

    fn build_position_labels(
        &self,
        drawing: &Drawing,
        position: PositionGeometry,
        vpr: f64,
        reward: Color,
        risk: Color,
        out: &mut Vec<Prim>,
    ) {
        let (Some(entry), Some(target), Some(stop)) = (
            drawing.points.first(),
            drawing.points.get(1),
            drawing.points.get(2),
        ) else {
            return;
        };
        let reward_distance = (target.price - entry.price).abs();
        let risk_distance = (entry.price - stop.price).abs();
        let base = entry.price.abs();
        let reward_percent = if base > f64::EPSILON {
            reward_distance / base * 100.0
        } else {
            0.0
        };
        let risk_percent = if base > f64::EPSILON {
            risk_distance / base * 100.0
        } else {
            0.0
        };
        let scale_target = match drawing.price_scale {
            crate::DrawingPriceScale::Right => crate::PriceScaleTarget::Right,
            crate::DrawingPriceScale::Left => crate::PriceScaleTarget::Left,
            crate::DrawingPriceScale::Overlay => crate::PriceScaleTarget::Overlay,
        };
        let format_price = |value: f64| self.format_drawing_price(drawing, value);
        let ticks = |from: f64, to: f64| {
            self.position_price_ticks_between(drawing.pane_index, drawing.price_scale, from, to)
                .map_or_else(|| "—".to_string(), |ticks| position_stat_number(ticks, 0))
        };
        let point_value = self.trading_state.instrument.point_value.unwrap_or(1.0);
        let risk_amount = drawing.position_account_size * drawing.position_risk_percent / 100.0;
        let quantity = if risk_distance > 0.0 {
            risk_amount / risk_distance / point_value
        } else {
            f64::NAN
        };
        let quantity_precision = self
            .trading_state
            .instrument
            .quantity_precision
            .unwrap_or(3) as usize;
        let target_amount =
            drawing.position_account_size + reward_distance * quantity * point_value;
        let stop_amount = drawing.position_account_size - risk_distance * quantity * point_value;
        let target_text = format!(
            "Target: {} ({reward_percent:.3}%) {}, Amount: {}",
            format_price(reward_distance),
            ticks(entry.price, target.price),
            position_stat_number(target_amount, 2),
        );
        let stop_text = format!(
            "Stop: {} ({risk_percent:.3}%) {}, Amount: {}",
            format_price(risk_distance),
            ticks(entry.price, stop.price),
            position_stat_number(stop_amount, 2),
        );
        let run = self.position_run_progress(drawing);
        // Unfilled/historical/future drawings still show the right-edge or latest close.
        // This is an estimate, not a broker execution; preserve the run's exact frozen exit.
        let current = run.map(|run| run.point.price).or_else(|| {
            let series = self.scale_formatter_source(drawing.pane_index, scale_target)?;
            if series.kind == crate::SeriesKind::Custom {
                return None;
            }
            let plot = self.data.plot(series.id);
            let row = plot.last_non_whitespace_row(target.logical.floor() as i64)?;
            let close = plot.value_at(row, PlotValueIndex::Close);
            close.is_finite().then_some(close)
        });
        let direction = if drawing.kind == DrawingKind::LongPosition {
            1.0
        } else {
            -1.0
        };
        let pnl = current.map(|price| (price - entry.price) * direction);
        let status = if run.is_some_and(|run| run.closed) {
            "Closed"
        } else {
            "Open"
        };
        let middle = [
            format!(
                "{status} P&L: {}, Qty: {}",
                pnl.map_or_else(|| "—".to_string(), format_price),
                position_stat_number(quantity, quantity_precision)
            ),
            format!(
                "Risk/reward ratio: {}",
                position_stat_number(reward_distance / risk_distance, 2)
            ),
        ];
        let center_x = (position.left + position.right) / 2.0;
        let label_offset = 14.0 * vpr;
        let target_label_y = if position.target_y < position.entry_y {
            position.target_y - label_offset
        } else {
            position.target_y + label_offset
        };
        let stop_label_y = if position.stop_y < position.entry_y {
            position.stop_y - label_offset
        } else {
            position.stop_y + label_offset
        };
        self.push_stat_label_block(out, (center_x, target_label_y), &[target_text], reward, vpr);
        self.push_stat_label_block(out, (center_x, stop_label_y), &[stop_text], risk, vpr);
        let pnl_color = if pnl.is_some_and(|value| value < 0.0) {
            risk
        } else {
            reward
        };
        self.push_stat_label_block(out, (center_x, position.entry_y), &middle, pnl_color, vpr);
    }

    /// Dynamic position progress is rebuilt with pane chrome rather than retained drawing
    /// geometry: series updates already invalidate chrome, so the darker traversed fill and
    /// terminal-candle trend can follow data without rebuilding every drawing on each tick.
    /// `parts` records each position's range so reassembly paints it in its drawing's z-slot.
    pub(super) fn build_position_progress_frame(
        &self,
        pane_index: usize,
        out: &mut Vec<Prim>,
        points: &mut Vec<[f32; 2]>,
        parts: &mut Vec<super::RetainedDrawingPart>,
        hpr: f64,
        vpr: f64,
    ) {
        for drawing in self.drawings.iter().filter(|drawing| {
            drawing.pane_index == pane_index
                && drawing.visible
                && drawing.interval_visibility.allows(self.drawing_interval)
                && matches!(
                    drawing.kind,
                    DrawingKind::LongPosition | DrawingKind::ShortPosition
                )
                && drawing.points.len() == 3
        }) {
            let Some(run) = self.position_run_progress(drawing) else {
                continue;
            };
            let run_point = run.point;
            let run_start = run.start;
            let entry = drawing.points[0].price;
            if !run_point.price.is_finite() || !entry.is_finite() {
                continue;
            }

            let Some(px) = self.drawing_px(drawing) else {
                continue;
            };
            let px = px
                .into_iter()
                .map(|(x, y)| (x * hpr, y * vpr))
                .collect::<Vec<_>>();
            let Some(geometry) = resolve_drawing_geometry(
                drawing.kind,
                &px,
                self.pane_w * hpr,
                self.panes[pane_index].top * vpr,
                self.panes[pane_index].height * vpr,
                DrawingGeometryOptions::for_drawing(drawing, vpr),
            ) else {
                continue;
            };
            let DrawingBodyGeometry::Position(position) = geometry.body else {
                continue;
            };

            // The progress pivot is the first post-placement candle that actually reaches/crosses
            // the entry. A position that has not filled emits no progress geometry at all.
            let Some((start_x, _)) =
                self.drawing_to_px_for(pane_index, drawing.price_scale, run_start)
            else {
                continue;
            };
            let Some((run_x, run_y)) =
                self.drawing_to_px_for(pane_index, drawing.price_scale, run_point)
            else {
                continue;
            };
            let start_x = (start_x * hpr).clamp(position.left, position.right);
            let run_x = (run_x * hpr).clamp(position.left, position.right);
            let run_y = run_y * vpr;
            let semantic = match run.side {
                PositionRunSide::Reward => {
                    Color::parse_css(aeris_charts_core::style::MARKET_UP_CSS)
                        .unwrap_or(Color::rgb(8, 153, 129))
                }
                PositionRunSide::Risk => {
                    Color::parse_css(aeris_charts_core::style::MARKET_DOWN_CSS)
                        .unwrap_or(Color::rgb(247, 82, 95))
                }
            };

            // Stronger opacity represents only the price/time space actually travelled since the
            // fill: first-fill x -> current/terminal x, entry y -> current/terminal y. It never
            // darkens the untouched remainder of either TP/SL zone.
            let (prim_start, point_start) = (out.len(), points.len());
            let travel_left = start_x.min(run_x);
            let travel_right = start_x.max(run_x);
            if travel_right > travel_left && (run_y - position.entry_y).abs() > f64::EPSILON {
                push_position_zone_with_alpha(
                    out,
                    PositionZone {
                        left: travel_left,
                        right: travel_right,
                        y0: position.entry_y,
                        y1: run_y,
                    },
                    semantic,
                    POSITION_PROGRESS_ALPHA,
                );
            }
            let progress_path = [
                [start_x as f32, position.entry_y as f32],
                [run_x as f32, run_y as f32],
            ];
            if progress_path[0] != progress_path[1] {
                push_line_stroke(
                    out,
                    points,
                    &progress_path,
                    vpr.max(1.0) as f32,
                    LineStyle::Dashed,
                    LineType::Simple,
                    POSITION_ENTRY,
                );
            }
            if out.len() != prim_start {
                parts.push(super::RetainedDrawingPart {
                    id: drawing.id,
                    prim_start,
                    prim_end: out.len(),
                    point_start,
                    point_end: points.len(),
                });
            }
        }
    }

    pub(super) fn position_run_progress(&self, drawing: &Drawing) -> Option<PositionRunProgress> {
        let target = match drawing.price_scale {
            crate::DrawingPriceScale::Right => crate::PriceScaleTarget::Right,
            crate::DrawingPriceScale::Left => crate::PriceScaleTarget::Left,
            crate::DrawingPriceScale::Overlay => crate::PriceScaleTarget::Overlay,
        };
        let series = self.series.iter().find(|series| {
            series.visible
                && !series.removed
                && series.pane_index == drawing.pane_index
                && super::series_scale_target(series) == target
        })?;
        // Custom-series frame values expose only their current value, not historical OHLC
        // extrema. Fabricating a "run" endpoint from that current value would violate the
        // position contract, so only canonical plot-backed series participate here.
        if series.kind == crate::SeriesKind::Custom {
            return None;
        }
        let plot = self.data.plot(series.id);
        let entry = drawing.points.first()?;
        let extent = drawing.points.get(1)?;
        let target_price = extent.price;
        let stop_price = drawing.points.get(2)?.price;
        if !entry.logical.is_finite()
            || !entry.price.is_finite()
            || !extent.logical.is_finite()
            || !target_price.is_finite()
            || !stop_price.is_finite()
            || extent.logical < entry.logical
        {
            return None;
        }
        let first_index = entry.logical.ceil();
        let last_index = extent.logical.floor();
        if first_index < i64::MIN as f64
            || first_index > i64::MAX as f64
            || last_index < i64::MIN as f64
            || last_index > i64::MAX as f64
            || first_index > last_index
        {
            return None;
        }
        let first_row = plot.first_non_whitespace_row(first_index as i64)?;
        let last_row = plot.last_non_whitespace_row(last_index as i64)?;
        if first_row > last_row {
            return None;
        }

        let first_high = plot.value_at(first_row, PlotValueIndex::High);
        let first_low = plot.value_at(first_row, PlotValueIndex::Low);
        if !first_high.is_finite() || !first_low.is_finite() {
            return None;
        }

        // Before fill, entry is approached from whichever side contains the first post-placement
        // candle. A candle already spanning entry fills immediately. Otherwise a one-sided extrema
        // predicate (High >= entry from below, Low <= entry from above) lets the LOD hierarchy find
        // the first touch/cross without scanning every historical candle. A gap across entry is a
        // deterministic OHLC "cross" and is anchored visually at the exact entry level.
        let starts_below = first_high < entry.price;
        let starts_above = first_low > entry.price;
        let fill_row = if !starts_below && !starts_above {
            first_row
        } else {
            let range_crosses_entry = |start: usize, end: usize| {
                let mut crossed = false;
                let mut inspect = |row: usize| {
                    if crossed || plot.is_whitespace_row(row) {
                        return;
                    }
                    let value_index = if starts_below {
                        PlotValueIndex::High
                    } else {
                        PlotValueIndex::Low
                    };
                    let value = plot.value_at(row, value_index);
                    crossed |= value.is_finite()
                        && if starts_below {
                            value >= entry.price
                        } else {
                            value <= entry.price
                        };
                };
                if let Some(lod) = plot.lod() {
                    let (rows, _) = lod.rows_on_range(start..end, usize::MAX);
                    for row in rows.iter() {
                        inspect(row);
                    }
                } else {
                    for row in start..end {
                        inspect(row);
                    }
                }
                crossed
            };
            if !range_crosses_entry(first_row, last_row + 1) {
                return None;
            }
            let mut lo = first_row;
            let mut hi = last_row;
            while lo < hi {
                let mid = lo + (hi - lo) / 2;
                if range_crosses_entry(first_row, mid + 1) {
                    hi = mid;
                } else {
                    lo = mid + 1;
                }
            }
            lo
        };

        let range_hits = |start: usize, end: usize| {
            let mut target_hit = false;
            let mut stop_hit = false;
            let mut inspect = |row: usize| {
                if plot.is_whitespace_row(row) {
                    return;
                }
                let high = plot.value_at(row, PlotValueIndex::High);
                let low = plot.value_at(row, PlotValueIndex::Low);
                match drawing.kind {
                    DrawingKind::LongPosition => {
                        target_hit |= high.is_finite() && high >= target_price;
                        stop_hit |= low.is_finite() && low <= stop_price;
                    }
                    DrawingKind::ShortPosition => {
                        target_hit |= low.is_finite() && low <= target_price;
                        stop_hit |= high.is_finite() && high >= stop_price;
                    }
                    _ => {}
                }
            };
            if let Some(lod) = plot.lod() {
                let (rows, _) = lod.rows_on_range(start..end, usize::MAX);
                for row in rows.iter() {
                    inspect(row);
                }
            } else {
                for row in start..end {
                    inspect(row);
                }
            }
            (target_hit, stop_hit)
        };

        let (any_target, any_stop) = range_hits(fill_row, last_row + 1);
        let (terminal_row, side, closed) = if any_target || any_stop {
            // Prefix boundary-hit is monotonic, so binary search finds the first candle touching
            // either target or stop without rescanning a long-lived position on every frame.
            let mut lo = fill_row;
            let mut hi = last_row;
            while lo < hi {
                let mid = lo + (hi - lo) / 2;
                let (target_hit, stop_hit) = range_hits(fill_row, mid + 1);
                if target_hit || stop_hit {
                    hi = mid;
                } else {
                    lo = mid + 1;
                }
            }
            let (target_hit, stop_hit) = range_hits(lo, lo + 1);
            // OHLC cannot tell intrabar order when both boundaries are touched by one candle.
            // Resolve that ambiguity conservatively as stop-first.
            let side = if stop_hit {
                PositionRunSide::Risk
            } else if target_hit {
                PositionRunSide::Reward
            } else {
                return None;
            };
            (lo, side, true)
        } else {
            let current = plot.value_at(last_row, PlotValueIndex::Close);
            if !current.is_finite() {
                return None;
            }
            let current = current.clamp(target_price.min(stop_price), target_price.max(stop_price));
            let side = match drawing.kind {
                DrawingKind::LongPosition => {
                    if current >= entry.price {
                        PositionRunSide::Reward
                    } else {
                        PositionRunSide::Risk
                    }
                }
                DrawingKind::ShortPosition => {
                    if current <= entry.price {
                        PositionRunSide::Reward
                    } else {
                        PositionRunSide::Risk
                    }
                }
                _ => return None,
            };
            (last_row, side, false)
        };

        let logical = plot.index_at(terminal_row)?;
        let price = if closed {
            match side {
                PositionRunSide::Reward => target_price,
                PositionRunSide::Risk => stop_price,
            }
        } else {
            plot.value_at(terminal_row, PlotValueIndex::Close)
                .clamp(target_price.min(stop_price), target_price.max(stop_price))
        };
        let start_logical = plot.index_at(fill_row)?;
        price.is_finite().then_some(PositionRunProgress {
            start: crate::drawings::DrawingPoint {
                logical: start_logical as f64,
                price: entry.price,
            },
            point: crate::drawings::DrawingPoint {
                logical: logical as f64,
                price,
            },
            side,
            closed,
        })
    }

    /// Position information labels paint in pane chrome, which composes above every drawing
    /// segment and so above each position's progress overlay (painted in its drawing's z-slot).
    /// This keeps the dashed run line visually behind the label chips instead of striking
    /// through their text.
    pub(super) fn build_position_labels_frame(
        &self,
        pane_index: usize,
        out: &mut Vec<Prim>,
        hpr: f64,
        vpr: f64,
    ) {
        let reward = Color::parse_css(aeris_charts_core::style::MARKET_UP_CSS)
            .unwrap_or(Color::rgb(8, 153, 129));
        let risk = Color::parse_css(aeris_charts_core::style::MARKET_DOWN_CSS)
            .unwrap_or(Color::rgb(247, 82, 95));
        // Statistic labels belong to the active position only; idle positions show their zones.
        for drawing in self.drawings.iter().filter(|drawing| {
            self.selected_drawing == Some(drawing.id)
                && drawing.pane_index == pane_index
                && drawing.visible
                && drawing.interval_visibility.allows(self.drawing_interval)
                && matches!(
                    drawing.kind,
                    DrawingKind::LongPosition | DrawingKind::ShortPosition
                )
                && drawing.points.len() == 3
        }) {
            let Some(px) = self.drawing_px(drawing) else {
                continue;
            };
            let px = px
                .into_iter()
                .map(|(x, y)| (x * hpr, y * vpr))
                .collect::<Vec<_>>();
            let Some(geometry) = resolve_drawing_geometry(
                drawing.kind,
                &px,
                self.pane_w * hpr,
                self.panes[pane_index].top * vpr,
                self.panes[pane_index].height * vpr,
                DrawingGeometryOptions::for_drawing(drawing, vpr),
            ) else {
                continue;
            };
            let DrawingBodyGeometry::Position(position) = geometry.body else {
                continue;
            };
            self.build_position_labels(drawing, position, vpr, reward, risk, out);
        }
    }

    fn push_stat_label_block(
        &self,
        out: &mut Vec<Prim>,
        center: (f64, f64),
        lines: &[String],
        background: Color,
        vpr: f64,
    ) {
        let (x, y) = center;
        if lines.is_empty() {
            return;
        }
        let layout = &self.options.get().layout;
        let size = self.drawing_stats_size() * vpr;
        let line_height = size * 1.25;
        let pad_x = 6.0 * vpr;
        let pad_y = 3.0 * vpr;
        let width = lines
            .iter()
            .map(|line| self.measure_text_run(line, size, &layout.font_family, 400, false))
            .fold(0.0_f64, f64::max)
            + 2.0 * pad_x;
        let height = lines.len() as f64 * line_height + 2.0 * pad_y;
        let rect = IRect {
            x: (x - width / 2.0).round() as i32,
            y: (y - height / 2.0).round() as i32,
            w: width.round().max(1.0) as i32,
            h: height.round().max(1.0) as i32,
        };
        // The solid semantic fill under contrast text already separates the label from the
        // chart, so it carries no outline.
        out.push(Prim::RoundRect {
            x: rect.x as f32,
            y: rect.y as f32,
            w: rect.w as f32,
            h: rect.h as f32,
            radii: [(3.0 * vpr).round().max(1.0) as f32; 4],
            fill: background.solid(),
            border_width: 0.0,
            border_color: background.solid(),
        });
        let text_color = background.contrast_text();
        let first_y = y - ((lines.len() as f64 - 1.0) * line_height) / 2.0;
        for (index, text) in lines.iter().enumerate() {
            out.push(Prim::Text {
                x: x as f32,
                y: (first_y + index as f64 * line_height) as f32,
                text: text.clone(),
                color: text_color,
                size: size as f32,
                family: layout.font_family.clone(),
                align: TextAlign::Center,
                weight: 400,
                italic: false,
            });
        }
    }

    /// The anchor-handle fill for the current theme (white on light backgrounds, black on dark —
    /// the series selection anchors' luminance rule, series_geometry.rs).
    fn anchor_fill(&self) -> Color {
        match self.surface_theme() {
            crate::ChartTheme::Light => Color::rgb(0xff, 0xff, 0xff),
            crate::ChartTheme::Dark => Color::rgb(0, 0, 0),
        }
    }

    /// The token theme matching the painted chart background, so in-chart chrome follows the
    /// surface hosts actually show even when they restyle `layout` without `set_theme`.
    pub(crate) fn surface_theme(&self) -> crate::ChartTheme {
        let fallback = aeris_charts_core::style::DEFAULT_SURFACE_RGB;
        let background = Color::parse_css(&self.options.get().layout.background.color)
            .unwrap_or(Color::rgb(fallback.0, fallback.1, fallback.2));
        if background.luminance() > 160.0 {
            crate::ChartTheme::Light
        } else {
            crate::ChartTheme::Dark
        }
    }
}

/// `ChartEngine::price_label_layout`: bubble `[left, top, width, height]`, label text and
/// glyph size, the text inset, and the tail height below the bubble.
pub(crate) struct PriceLabelLayout {
    pub(crate) rect: [f64; 4],
    pub(crate) text: String,
    pub(crate) size: f64,
    pub(crate) padding: f64,
    pub(crate) tail: f64,
}

/// Info Line card metrics in CSS px, following the package's `.aeris_charts-tooltip` panel
/// (12/16 px type, 10 px side and 6/7 px vertical padding, 12 px column gap). Rows get a little
/// more spacing than the tooltip's 2 px so the 14 px icons do not touch.
const INFO_LINE_FONT_CSS: f64 = 12.0;
const INFO_LINE_LINE_HEIGHT_CSS: f64 = 16.0;
const INFO_LINE_ROW_GAP_CSS: f64 = 4.0;
const INFO_LINE_PAD_X_CSS: f64 = 10.0;
const INFO_LINE_PAD_TOP_CSS: f64 = 6.0;
const INFO_LINE_PAD_BOTTOM_CSS: f64 = 7.0;
const INFO_LINE_ICON_CSS: f64 = 16.0;
const INFO_LINE_ICON_GAP_CSS: f64 = 12.0;

/// In-chart chrome colors from the design tokens of the painted theme (the package CSS's
/// `--surface`, `--border`, `--text-primary`, `--text-secondary`, and accent surface), shared by
/// the Info Line card and the box annotations.
pub(crate) struct ChromeTokens {
    pub(crate) surface: Color,
    pub(crate) accent: Color,
    pub(crate) border: Color,
    pub(crate) foreground: Color,
    pub(crate) muted: Color,
}

impl ChromeTokens {
    pub(crate) fn for_theme(theme: crate::ChartTheme) -> Self {
        use aeris_charts_core::style::*;
        let (surface, accent, border, foreground, muted) = match theme {
            crate::ChartTheme::Light => (
                LIGHT_SURFACE_RGB,
                LIGHT_ACCENT_RGB,
                LIGHT_BORDER_RGB,
                LIGHT_FOREGROUND_RGB,
                LIGHT_MUTED_FOREGROUND_RGB,
            ),
            crate::ChartTheme::Dark => (
                DARK_SURFACE_RGB,
                DARK_ACCENT_RGB,
                DARK_BORDER_RGB,
                DARK_FOREGROUND_RGB,
                DARK_MUTED_FOREGROUND_RGB,
            ),
        };
        let rgb = |(r, g, b): (u8, u8, u8)| Color::rgb(r, g, b);
        Self {
            surface: rgb(surface),
            accent: rgb(accent),
            border: rgb(border),
            foreground: rgb(foreground),
            muted: rgb(muted),
        }
    }
}

/// The glyph leading each Info Line card row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InfoLineIcon {
    /// Price range: a double arrow between two horizontal bars.
    Price,
    /// Time range: a double arrow between two candles.
    Time,
    /// Angle: a ray rising from a baseline with its arc.
    Angle,
}

/// Strokes `kind`'s glyph centered at `center`, drawn on a 16 CSS px grid.
fn push_info_line_icon(
    kind: InfoLineIcon,
    center: (f64, f64),
    color: Color,
    vpr: f64,
    out: &mut Vec<Prim>,
    points: &mut Vec<[f32; 2]>,
) {
    let (cx, cy) = center;
    let at = |x: f64, y: f64| [(cx + x * vpr) as f32, (cy + y * vpr) as f32];
    let mut stroke = |path: &[[f32; 2]]| {
        let first_point = points.len() as u32;
        points.extend_from_slice(path);
        out.push(Prim::Polyline {
            first_point,
            point_count: path.len() as u32,
            width: vpr.round().max(1.0) as f32,
            style: LineStyle::Solid,
            line_type: LineType::Simple,
            color,
        });
    };
    match kind {
        InfoLineIcon::Price => {
            stroke(&[at(-5.0, -7.0), at(5.0, -7.0)]);
            stroke(&[at(-5.0, 7.0), at(5.0, 7.0)]);
            stroke(&[at(0.0, -5.0), at(0.0, 5.0)]);
            stroke(&[at(-3.0, -2.0), at(0.0, -5.0), at(3.0, -2.0)]);
            stroke(&[at(-3.0, 2.0), at(0.0, 5.0), at(3.0, 2.0)]);
        }
        InfoLineIcon::Time => {
            for side in [-1.0, 1.0] {
                let x = 7.0 * side;
                stroke(&[at(x, -7.0), at(x, -3.0)]);
                stroke(&[at(x, 3.0), at(x, 7.0)]);
                stroke(&[
                    at(x - 1.5, -3.0),
                    at(x + 1.5, -3.0),
                    at(x + 1.5, 3.0),
                    at(x - 1.5, 3.0),
                    at(x - 1.5, -3.0),
                ]);
                stroke(&[
                    at(-1.5 * side, -2.0),
                    at(-3.5 * side, 0.0),
                    at(-1.5 * side, 2.0),
                ]);
            }
            stroke(&[at(-3.5, 0.0), at(3.5, 0.0)]);
        }
        InfoLineIcon::Angle => {
            stroke(&[at(-7.0, 6.0), at(7.0, 6.0)]);
            stroke(&[at(-7.0, 6.0), at(2.0, -7.0)]);
            let ray = (-13.0_f64).atan2(9.0);
            let arc: Vec<[f32; 2]> = (0..=8)
                .map(|step| {
                    let angle = ray * f64::from(step) / 8.0;
                    at(-7.0 + 8.0 * angle.cos(), 6.0 + 8.0 * angle.sin())
                })
                .collect();
            stroke(&arc);
        }
    }
}

fn push_position_zone(out: &mut Vec<Prim>, zone: PositionZone, color: Color) {
    push_position_zone_with_alpha(out, zone, color, POSITION_ZONE_ALPHA);
}

fn push_position_zone_with_alpha(out: &mut Vec<Prim>, zone: PositionZone, color: Color, alpha: u8) {
    let left = zone.left.round() as i32;
    let right = zone.right.round() as i32;
    let top = zone.y0.min(zone.y1).round() as i32;
    let bottom = zone.y0.max(zone.y1).round() as i32;
    let width = (right - left).abs() + 1;
    let height = (bottom - top).abs() + 1;
    let rect = IRect {
        x: left,
        y: top,
        w: width,
        h: height,
    };
    out.push(Prim::Rect {
        rect,
        color: Color::rgba(color.r(), color.g(), color.b(), alpha),
    });
}

/// Every drawing handle, round or square: one pixel-aligned shape that owns both its fill and
/// its inside border, so the ring is the same whole-device-pixel width on every side, tool, and
/// backend (separately rounded fill and border shapes drifted apart at fractional positions).
fn push_handle(center: (f64, f64), square: bool, vpr: f64, fill: Color, out: &mut Vec<Prim>) {
    let side = (2.0 * (ANCHOR_RADIUS + ANCHOR_BORDER_WIDTH) * vpr)
        .round()
        .max(3.0);
    let radius = if square {
        (ANCHOR_SQUARE_RADIUS * vpr).round().max(1.0)
    } else {
        side / 2.0
    };
    out.push(Prim::RoundRect {
        x: (center.0 - side / 2.0).round() as f32,
        y: (center.1 - side / 2.0).round() as f32,
        w: side as f32,
        h: side as f32,
        radii: [radius as f32; 4],
        fill,
        border_width: (ANCHOR_BORDER_WIDTH * vpr).floor().max(1.0) as f32,
        border_color: ANCHOR_BORDER,
    });
}

/// One round handle per anchor.
fn build_anchor_handles(px: &[(f64, f64)], vpr: f64, fill: Color, out: &mut Vec<Prim>) {
    for &center in px {
        push_handle(center, false, vpr, fill, out);
    }
}

/// Paint a handle set (`drawings/handles.rs`) in its order, each in its shape.
fn build_handles(handles: &[DrawingHandle], vpr: f64, fill: Color, out: &mut Vec<Prim>) {
    for handle in handles {
        push_handle(
            handle.point,
            handle.shape == HandleShape::Square,
            vpr,
            fill,
            out,
        );
    }
}

fn position_stat_number(value: f64, precision: usize) -> String {
    if !value.is_finite() {
        return "—".to_string();
    }
    let text = format!("{value:.precision$}");
    let text = if precision > 0 {
        text.trim_end_matches('0').trim_end_matches('.')
    } else {
        &text
    };
    if text == "-0" {
        "0".to_string()
    } else {
        text.to_string()
    }
}
