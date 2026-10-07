//! Canvas2D fallback executor for the [`Prim`] draw-list IR (roadmap Phase D2).
//!
//! The WebGPU backend rasterizes the IR through instanced quads + tessellated triangles. This
//! module is the *other* backend the roadmap calls for: a straight translation of the same IR
//! into `CanvasRenderingContext2D`-style calls, issued against an abstract [`Canvas2d`] target.
//! It is pure (no gpu, no dom) and lives here so it can be unit-tested and reused by two concrete
//! targets later — web-sys `CanvasRenderingContext2d` in `aeris_charts_wasm` (browsers without WebGPU) and
//! a native rasterizer in `aeris_charts_native` (server PNGs + golden-image tests).
//!
//! The crisp-rect subset (`Rect`, `RectFrame`, `HLine`, `VLine`) reproduces the exact integer
//! math of the wgpu quad executor — same pixel coverage, so the two backends agree pixel-for-pixel
//! on rects. The anti-aliased prims (`Polyline`, `AreaFill`, `Circle`, `RoundRect`, `Background`)
//! map onto native 2D path/gradient calls, which the wgpu path approximates with tessellation.

use crate::color::Color;
use crate::draw_list::{
    IRect, LineStyle, LineType, Prim, RasterImage, TextAlign, positive_finite_extent,
    segment_points, text_font_spec,
};
use crate::line::{LinePoint, expand_band, expand_line};

/// Abstract 2D drawing target: the subset of `CanvasRenderingContext2D` this executor needs.
/// Coordinates are bitmap-space (device px), matching the IR. Concrete impls wrap web-sys or a
/// native rasterizer; the unit tests use a recording impl.
pub trait Canvas2d {
    /// Save the target state before applying a backend-specific clip.
    fn save(&mut self) {}
    /// Restore the target state after drawing a clipped layer.
    fn restore(&mut self) {}
    /// Clip subsequent drawing to an integer bitmap-space rectangle.
    fn clip_rect(&mut self, _x: f32, _y: f32, _w: f32, _h: f32) {}

    /// Set the fill style to a solid color.
    fn set_fill_solid(&mut self, color: Color);
    /// Set the fill style to a vertical linear gradient from `y_top` (`top`) to `y_bottom`.
    fn set_fill_vgradient(&mut self, y_top: f32, y_bottom: f32, top: Color, bottom: Color);
    /// Set the stroke color.
    fn set_stroke(&mut self, color: Color);
    /// Set the stroke line width.
    fn set_line_width(&mut self, width: f32);
    /// Set the dash pattern (empty = solid).
    fn set_line_dash(&mut self, pattern: &[f32]);

    /// Fill an axis-aligned rectangle with the current fill style.
    fn fill_rect(&mut self, x: f32, y: f32, w: f32, h: f32);

    fn begin_path(&mut self);
    fn move_to(&mut self, x: f32, y: f32);
    fn line_to(&mut self, x: f32, y: f32);
    fn close_path(&mut self);
    /// Add a circular arc (radians) to the current path.
    fn arc(&mut self, cx: f32, cy: f32, r: f32, start: f32, end: f32);
    /// Stroke the current path with the current stroke color / width / dash, using round joins
    /// and butt caps (the reference line renderer's `lineJoin = 'round'`): a miter join spikes
    /// past every sharp turn of a price line.
    fn stroke(&mut self);
    /// Fill the current path with the current fill style.
    fn fill(&mut self);

    /// Draw a text run with the browser's text engine: fully-resolved CSS `font` shorthand
    /// ([`text_font_spec`]), `textBaseline: "middle"` (y is the vertical center), `textAlign`
    /// from `align`. Default no-op: targets without a glyph engine (e.g. recorders, native
    /// rasterizers not yet text-capable) silently drop text.
    fn fill_text(
        &mut self,
        _text: &str,
        _x: f32,
        _y: f32,
        _font: &str,
        _color: Color,
        _align: TextAlign,
    ) {
    }

    /// Draw a run rotated clockwise around its aligned anchor. Every executor must implement the
    /// transform explicitly; there is intentionally no angle-dropping fallback.
    #[allow(clippy::too_many_arguments)]
    fn fill_rotated_text(
        &mut self,
        text: &str,
        x: f32,
        y: f32,
        font: &str,
        color: Color,
        align: TextAlign,
        angle: f32,
    );

    /// Draw immutable straight-alpha RGBA8 pixels into a bitmap-space rectangle. Default no-op
    /// keeps command recorders lightweight; shipping browser/native targets implement it.
    fn draw_raster_image(&mut self, _image: &RasterImage, _rect: [f32; 4], _opacity: f32) {}
}

/// The viewport (bitmap px). Carried for target context; every prim now owns its own extent
/// (the `Background` gradient included), so the executor no longer consults it.
#[derive(Clone, Copy, Debug)]
pub struct Viewport {
    pub width: f32,
    pub height: f32,
}

/// Emits filled dash segments over `[from, to)` — identical to the wgpu executor so rect coverage
/// matches pixel-for-pixel.
fn dash_segments(style: LineStyle, width: i32, from: i32, to: i32, mut emit: impl FnMut(i32, i32)) {
    let pattern = style.dash_pattern(width as f32);
    if pattern.is_empty() {
        emit(from, to);
        return;
    }
    let mut pos = from as f32;
    let mut i = 0usize;
    let mut on = true;
    while pos < to as f32 {
        let seg = pattern[i % pattern.len()];
        if on {
            let a = pos.round() as i32;
            let b = ((pos + seg).min(to as f32)).round() as i32;
            if b > a {
                emit(a, b);
            }
        }
        pos += seg;
        i += 1;
        on = !on;
    }
}

/// Fill an integer rect via the target, dropping degenerate sizes (matches the wgpu executor).
fn fill_irect(target: &mut impl Canvas2d, rect: IRect) {
    if rect.w <= 0 || rect.h <= 0 {
        return;
    }
    target.fill_rect(rect.x as f32, rect.y as f32, rect.w as f32, rect.h as f32);
}

/// Port of `fillRectInnerBorder` — four edge rects (same order as the wgpu executor).
fn fill_rect_frame(target: &mut impl Canvas2d, rect: IRect, border: i32) {
    let IRect { x, y, w, h } = rect;
    fill_irect(
        target,
        IRect {
            x: x + border,
            y,
            w: w - border * 2,
            h: border,
        },
    );
    fill_irect(
        target,
        IRect {
            x: x + border,
            y: y + h - border,
            w: w - border * 2,
            h: border,
        },
    );
    fill_irect(target, IRect { x, y, w: border, h });
    fill_irect(
        target,
        IRect {
            x: x + w - border,
            y,
            w: border,
            h,
        },
    );
}

/// Trace `points` as a path (`move_to` then `line_to`), applying the `line_type` expansion.
fn trace_polyline(target: &mut impl Canvas2d, points: &[LinePoint], line_type: LineType) {
    let pts = expand_line(points, line_type);
    for (i, p) in pts.iter().enumerate() {
        if i == 0 {
            target.move_to(p.x as f32, p.y as f32);
        } else {
            target.line_to(p.x as f32, p.y as f32);
        }
    }
}

/// Slice a `[first, first+count)` window of the shared point pool into `LinePoint`s.
fn pool_slice(points: &[[f32; 2]], first: u32, count: u32) -> Vec<LinePoint> {
    let (a, b) = (first as usize, (first + count) as usize);
    points
        .get(a..b)
        .unwrap_or(&[])
        .iter()
        .map(|p| LinePoint {
            x: p[0] as f64,
            y: p[1] as f64,
        })
        .collect()
}

/// Execute one layer of prims against a 2D target. `points` is the layer's shared point pool
/// (referenced by `Polyline`/`Segments`/`AreaFill`/`BandFill`). `Text` runs go through the target's text engine in
/// prim order — the browser target rasterizes the same glyphs the WebGPU host samples into its
/// atlas, keeping the two backends pixel-identical.
pub fn execute(
    prims: &[Prim],
    points: &[[f32; 2]],
    target: &mut impl Canvas2d,
    _viewport: Viewport,
) {
    for prim in prims {
        match prim {
            Prim::Rect { rect, color } => {
                target.set_fill_solid(*color);
                fill_irect(target, *rect);
            }
            Prim::RectFrame {
                rect,
                border,
                color,
            } => {
                target.set_fill_solid(*color);
                fill_rect_frame(target, *rect, *border);
            }
            Prim::HLine {
                y,
                x0,
                x1,
                width,
                style,
                color,
            } => {
                target.set_fill_solid(*color);
                let top = y - width / 2;
                dash_segments(*style, *width, *x0, *x1, |a, b| {
                    fill_irect(
                        target,
                        IRect {
                            x: a,
                            y: top,
                            w: b - a,
                            h: *width,
                        },
                    );
                });
            }
            Prim::VLine {
                x,
                y0,
                y1,
                width,
                style,
                color,
            } => {
                target.set_fill_solid(*color);
                let left = x - width / 2;
                dash_segments(*style, *width, *y0, *y1, |a, b| {
                    fill_irect(
                        target,
                        IRect {
                            x: left,
                            y: a,
                            w: *width,
                            h: b - a,
                        },
                    );
                });
            }
            Prim::Polyline {
                first_point,
                point_count,
                width,
                style,
                line_type,
                color,
            } => {
                let pts = pool_slice(points, *first_point, *point_count);
                if pts.len() < 2 || !positive_finite_extent(*width) {
                    continue;
                }
                target.set_stroke(*color);
                target.set_line_width(*width);
                target.set_line_dash(&style.dash_pattern(*width));
                target.begin_path();
                trace_polyline(target, &pts, *line_type);
                target.stroke();
                target.set_line_dash(&[]);
            }
            Prim::Segments {
                first_point,
                segment_count,
                width,
                color,
            } => {
                let Some(pairs) = segment_points(points, *first_point, *segment_count) else {
                    continue;
                };
                if pairs.is_empty() {
                    continue;
                }
                // One path of independent pairs, one stroke. The dash reset stays explicit: a host
                // plugin sharing the context may have left a pattern behind.
                target.set_stroke(*color);
                target.set_line_width(*width);
                target.set_line_dash(&[]);
                target.begin_path();
                for [from, to] in pairs.as_chunks::<2>().0 {
                    target.move_to(from[0], from[1]);
                    target.line_to(to[0], to[1]);
                }
                target.stroke();
            }
            Prim::AreaFill {
                first_point,
                point_count,
                base_y,
                line_type,
                gradient,
            } => {
                let pts = pool_slice(points, *first_point, *point_count);
                if pts.len() < 2 {
                    continue;
                }
                let expanded = expand_line(&pts, *line_type);
                let (y_top, y_bottom, first_x, last_x) = area_extent(&expanded, *base_y);
                target.set_fill_vgradient(y_top, y_bottom, gradient.top, gradient.bottom);
                target.begin_path();
                trace_polyline(target, &pts, *line_type);
                target.line_to(last_x, *base_y);
                target.line_to(first_x, *base_y);
                target.close_path();
                target.fill();
            }
            Prim::BandFill {
                upper_first,
                lower_first,
                point_count,
                line_type,
                fill,
            } => {
                let upper = pool_slice(points, *upper_first, *point_count);
                let lower = pool_slice(points, *lower_first, *point_count);
                if upper.len() < 2 || lower.len() < 2 {
                    continue;
                }
                let (upper, lower) = expand_band(&upper, &lower, *line_type);
                target.set_fill_solid(*fill);
                target.begin_path();
                target.move_to(upper[0].x as f32, upper[0].y as f32);
                for p in &upper[1..] {
                    target.line_to(p.x as f32, p.y as f32);
                }
                for p in lower.iter().rev() {
                    target.line_to(p.x as f32, p.y as f32);
                }
                target.close_path();
                target.fill();
            }
            Prim::Circle {
                cx,
                cy,
                radius,
                fill,
                stroke_width,
                stroke,
            } => {
                if !positive_finite_extent(*radius) {
                    continue;
                }
                target.set_fill_solid(*fill);
                target.begin_path();
                target.arc(*cx, *cy, *radius, 0.0, std::f32::consts::TAU);
                target.fill();
                if positive_finite_extent(*stroke_width) {
                    target.set_stroke(*stroke);
                    target.set_line_width(*stroke_width);
                    target.stroke();
                }
            }
            Prim::Triangle { a, b, c, color } => {
                target.set_fill_solid(*color);
                target.begin_path();
                target.move_to(a[0], a[1]);
                target.line_to(b[0], b[1]);
                target.line_to(c[0], c[1]);
                target.close_path();
                target.fill();
            }
            Prim::RoundRect {
                x,
                y,
                w,
                h,
                radii,
                fill,
                border_width,
                border_color,
            } => {
                if *w <= 0.0 || *h <= 0.0 {
                    continue;
                }
                // `border_width` is an INSIDE border, exactly like the WebGPU/GPUI tessellation:
                // the fill covers only the inner rect and the stroke is centred half a border
                // inside the edge. A stroke centred on the edge would spill half outside the
                // shape and, on whole-pixel geometry, smear a hairline across two pixel rows.
                let radii = crate::line::normalized_round_rect_radii(*w, *h, *radii);
                let inset = border_width.max(0.0).min(*w / 2.0).min(*h / 2.0);
                let inner = |by: f32| radii.map(|r| (r - by).max(0.0));
                round_rect_path(
                    target,
                    x + inset,
                    y + inset,
                    w - inset * 2.0,
                    h - inset * 2.0,
                    inner(inset),
                );
                target.set_fill_solid(*fill);
                target.fill();
                if inset > 0.0 {
                    let half = inset / 2.0;
                    round_rect_path(
                        target,
                        x + half,
                        y + half,
                        w - inset,
                        h - inset,
                        inner(half),
                    );
                    target.set_stroke(*border_color);
                    target.set_line_width(inset);
                    target.stroke();
                }
            }
            Prim::Background { rect, gradient } => {
                // reference pane-widget.ts `_drawBackground`: the two-stop ramp spans the pane rect.
                let [x, y, w, h] = *rect;
                target.set_fill_vgradient(y, y + h, gradient.top, gradient.bottom);
                target.fill_rect(x, y, w, h);
            }
            Prim::Text {
                x,
                y,
                text,
                color,
                size,
                family,
                align,
                weight,
                italic,
            } => {
                if !text.is_empty() {
                    target.fill_text(
                        text,
                        *x,
                        *y,
                        &text_font_spec(*size, family, *weight, *italic),
                        *color,
                        *align,
                    );
                }
            }
            Prim::RotatedText {
                x,
                y,
                text,
                color,
                size,
                family,
                align,
                weight,
                italic,
                angle,
            } => {
                if !text.is_empty() {
                    target.fill_rotated_text(
                        text,
                        *x,
                        *y,
                        &text_font_spec(*size, family, *weight, *italic),
                        *color,
                        *align,
                        *angle,
                    );
                }
            }
            Prim::Image {
                image,
                rect,
                opacity,
            } => {
                if image.width > 0
                    && image.height > 0
                    && *opacity > 0.0
                    && let Some(rect) = crate::draw_list::snap_image_rect(*rect)
                {
                    target.draw_raster_image(image, rect, opacity.clamp(0.0, 1.0));
                }
            }
        }
    }
}

/// The fill's vertical gradient extent and its first/last x. The gradient runs from the
/// geometrically highest edge (`y_top`) to the lowest (`y_bottom`): normally
/// [topmost point, base_y], but inverted (reference `invertFilledArea` / baseline segments under the
/// base level) it runs [base_y, lowest point] — matching the per-vertex shading of the wgpu
/// tessellator stop-for-stop.
fn area_extent(pts: &[LinePoint], base_y: f32) -> (f32, f32, f32, f32) {
    let mut y_top = base_y;
    let mut y_bottom = base_y;
    for p in pts {
        y_top = y_top.min(p.y as f32);
        y_bottom = y_bottom.max(p.y as f32);
    }
    (
        y_top,
        y_bottom,
        pts[0].x as f32,
        pts[pts.len() - 1].x as f32,
    )
}

/// Build a rounded-rect path (left-top, right-top, right-bottom, left-bottom radii).
fn round_rect_path(target: &mut impl Canvas2d, x: f32, y: f32, w: f32, h: f32, radii: [f32; 4]) {
    use std::f32::consts::PI;
    let [lt, rt, rb, lb] = crate::line::normalized_round_rect_radii(w, h, radii);
    target.begin_path();
    target.move_to(x + lt, y);
    target.line_to(x + w - rt, y);
    target.arc(x + w - rt, y + rt, rt, -PI / 2.0, 0.0);
    target.line_to(x + w, y + h - rb);
    target.arc(x + w - rb, y + h - rb, rb, 0.0, PI / 2.0);
    target.line_to(x + lb, y + h);
    target.arc(x + lb, y + h - lb, lb, PI / 2.0, PI);
    target.line_to(x, y + lt);
    target.arc(x + lt, y + lt, lt, PI, 3.0 * PI / 2.0);
    target.close_path();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::draw_list::Gradient;

    #[test]
    fn oversized_round_rect_radii_match_the_shared_polygon() {
        let radii = [30.0, 30.0, 5.0, 5.0];
        let mut target = Recorder::default();
        round_rect_path(&mut target, 0.0, 0.0, 40.0, 20.0, radii);
        let top_left = 30.0 * 4.0 / 7.0;
        let lower_left = 5.0 * 4.0 / 7.0;
        let polygon = crate::line::round_rect_polygon(0.0, 0.0, 40.0, 20.0, radii);
        assert!((polygon[0][0] - top_left).abs() < 1e-4);
        assert!((polygon[1][0] - (40.0 - top_left)).abs() < 1e-4);
        let arc_radii: Vec<f32> = target
            .ops
            .iter()
            .filter_map(|op| {
                let words: Vec<&str> = op.split_whitespace().collect();
                (words.first() == Some(&"arc")).then(|| words[3].parse().unwrap())
            })
            .collect();
        for (actual, expected) in arc_radii
            .into_iter()
            .zip([top_left, lower_left, lower_left, top_left])
        {
            assert!((actual - expected).abs() < 1e-4, "{actual} != {expected}");
        }
    }

    const C: Color = Color::rgb(0x10, 0x20, 0x30);

    /// Records the target calls as tagged strings so tests can assert the command stream.
    #[derive(Default)]
    struct Recorder {
        ops: Vec<String>,
    }
    /// Full 0xRRGGBBAA hex (keeps alpha, unlike `Color::to_hex`) so tests can assert gradients.
    fn hx(c: Color) -> String {
        format!("{:08x}", c.0)
    }
    impl Canvas2d for Recorder {
        fn set_fill_solid(&mut self, c: Color) {
            self.ops.push(format!("fill_solid {}", hx(c)));
        }
        fn set_fill_vgradient(&mut self, y0: f32, y1: f32, t: Color, b: Color) {
            self.ops
                .push(format!("fill_grad {y0} {y1} {} {}", hx(t), hx(b)));
        }
        fn set_stroke(&mut self, c: Color) {
            self.ops.push(format!("stroke_color {}", hx(c)));
        }
        fn set_line_width(&mut self, w: f32) {
            self.ops.push(format!("line_width {w}"));
        }
        fn set_line_dash(&mut self, p: &[f32]) {
            self.ops.push(format!("line_dash {p:?}"));
        }
        fn fill_rect(&mut self, x: f32, y: f32, w: f32, h: f32) {
            self.ops.push(format!("fill_rect {x} {y} {w} {h}"));
        }
        fn begin_path(&mut self) {
            self.ops.push("begin".into());
        }
        fn move_to(&mut self, x: f32, y: f32) {
            self.ops.push(format!("move {x} {y}"));
        }
        fn line_to(&mut self, x: f32, y: f32) {
            self.ops.push(format!("line {x} {y}"));
        }
        fn close_path(&mut self) {
            self.ops.push("close".into());
        }
        fn arc(&mut self, cx: f32, cy: f32, r: f32, s: f32, e: f32) {
            self.ops.push(format!("arc {cx} {cy} {r} {s} {e}"));
        }
        fn stroke(&mut self) {
            self.ops.push("stroke".into());
        }
        fn fill(&mut self) {
            self.ops.push("fill".into());
        }
        fn fill_text(
            &mut self,
            text: &str,
            x: f32,
            y: f32,
            font: &str,
            color: Color,
            align: TextAlign,
        ) {
            self.ops.push(format!(
                "text {text:?} {x} {y} {font:?} {} {}",
                hx(color),
                align.canvas_keyword()
            ));
        }
        fn fill_rotated_text(
            &mut self,
            text: &str,
            x: f32,
            y: f32,
            font: &str,
            color: Color,
            align: TextAlign,
            angle: f32,
        ) {
            self.ops.push(format!(
                "rotated_text {text:?} {x} {y} {font:?} {} {} {angle}",
                hx(color),
                align.canvas_keyword()
            ));
        }
    }

    fn run(prims: &[Prim], points: &[[f32; 2]]) -> Vec<String> {
        let mut r = Recorder::default();
        execute(
            prims,
            points,
            &mut r,
            Viewport {
                width: 200.0,
                height: 100.0,
            },
        );
        r.ops
    }

    #[test]
    fn rect_sets_color_then_fills() {
        let ops = run(
            &[Prim::Rect {
                rect: IRect {
                    x: 3,
                    y: 4,
                    w: 10,
                    h: 6,
                },
                color: C,
            }],
            &[],
        );
        assert_eq!(
            ops,
            vec![
                "fill_solid 102030ff".to_string(),
                "fill_rect 3 4 10 6".into()
            ]
        );
    }

    #[test]
    fn round_rect_border_is_inside_like_the_tessellated_executors() {
        let ops = run(
            &[Prim::RoundRect {
                x: 10.0,
                y: 20.0,
                w: 40.0,
                h: 24.0,
                radii: [12.0; 4],
                fill: C,
                border_width: 2.0,
                border_color: Color::rgb(0xff, 0, 0),
            }],
            &[],
        );
        // The fill covers only the inner rect, never the border ring.
        assert_eq!(ops[1], "move 22 22");
        assert!(ops.contains(&"arc 22 32 10 1.5707964 3.1415927".to_string()));
        // The stroke is centred one half-border inside the edge, so it ends exactly on it.
        let stroke_path = ops.iter().position(|op| op == "fill").unwrap() + 1;
        assert_eq!(ops[stroke_path], "begin");
        assert_eq!(ops[stroke_path + 1], "move 22 21");
        assert!(ops.contains(&"line_width 2".to_string()));
        assert_eq!(ops.last().unwrap(), "stroke");
    }

    #[test]
    fn degenerate_rect_dropped_but_color_still_set() {
        let ops = run(
            &[Prim::Rect {
                rect: IRect {
                    x: 0,
                    y: 0,
                    w: 0,
                    h: 6,
                },
                color: C,
            }],
            &[],
        );
        assert_eq!(ops, vec!["fill_solid 102030ff".to_string()]);
    }

    #[test]
    fn rect_frame_expands_to_four_edges() {
        let ops = run(
            &[Prim::RectFrame {
                rect: IRect {
                    x: 10,
                    y: 20,
                    w: 8,
                    h: 6,
                },
                border: 1,
                color: C,
            }],
            &[],
        );
        assert_eq!(
            ops,
            vec![
                "fill_solid 102030ff".to_string(),
                "fill_rect 11 20 6 1".into(), // top
                "fill_rect 11 25 6 1".into(), // bottom
                "fill_rect 10 20 1 6".into(), // left
                "fill_rect 17 20 1 6".into(), // right
            ]
        );
    }

    #[test]
    fn hline_centers_odd_width_and_fills_span() {
        // width 1 centered on y=50 -> top = 50 - 0 = 50
        let ops = run(
            &[Prim::HLine {
                y: 50,
                x0: 10,
                x1: 40,
                width: 1,
                style: LineStyle::Solid,
                color: C,
            }],
            &[],
        );
        assert_eq!(
            ops,
            vec![
                "fill_solid 102030ff".to_string(),
                "fill_rect 10 50 30 1".into()
            ]
        );
    }

    #[test]
    fn large_dashed_vline_emits_on_segments_only() {
        // pattern 6-on/6-off over [0,24): segments [0,6) and [12,18)
        let ops = run(
            &[Prim::VLine {
                x: 5,
                y0: 0,
                y1: 24,
                width: 1,
                style: LineStyle::Dashed,
                color: C,
            }],
            &[],
        );
        assert_eq!(
            ops,
            vec![
                "fill_solid 102030ff".to_string(),
                "fill_rect 5 0 1 6".into(),
                "fill_rect 5 12 1 6".into(),
            ]
        );
    }

    #[test]
    fn rotated_text_is_forwarded_without_dropping_its_angle() {
        let ops = run(
            &[Prim::RotatedText {
                x: 10.0,
                y: 20.0,
                text: "trend".into(),
                color: C,
                size: 12.0,
                family: "sans-serif".into(),
                align: TextAlign::Center,
                weight: 500,
                italic: true,
                angle: -0.75,
            }],
            &[],
        );
        assert_eq!(
            ops,
            ["rotated_text \"trend\" 10 20 \"italic 500 12px sans-serif\" 102030ff center -0.75"]
        );
    }

    #[test]
    fn polyline_strokes_path_with_dash_reset() {
        let points = [[0.0f32, 0.0], [10.0, 5.0], [20.0, 0.0]];
        let ops = run(
            &[Prim::Polyline {
                first_point: 0,
                point_count: 3,
                width: 2.0,
                style: LineStyle::Solid,
                line_type: LineType::Simple,
                color: C,
            }],
            &points,
        );
        assert_eq!(
            ops,
            vec![
                "stroke_color 102030ff".to_string(),
                "line_width 2".into(),
                "line_dash []".into(),
                "begin".into(),
                "move 0 0".into(),
                "line 10 5".into(),
                "line 20 0".into(),
                "stroke".into(),
                "line_dash []".into(),
            ]
        );
    }

    #[test]
    fn segments_stroke_as_one_path() {
        let points = [
            [9.0f32, 9.0],
            [0.0, 5.0],
            [10.0, 5.0],
            [12.0, 7.0],
            [22.0, 7.0],
        ];
        let segments = |first_point, segment_count| Prim::Segments {
            first_point,
            segment_count,
            width: 2.0,
            color: C,
        };
        // Two independent pairs share one path and one stroke, with the dash reset up front.
        assert_eq!(
            run(&[segments(1, 2)], &points),
            vec![
                "stroke_color 102030ff".to_string(),
                "line_width 2".into(),
                "line_dash []".into(),
                "begin".into(),
                "move 0 5".into(),
                "line 10 5".into(),
                "move 12 7".into(),
                "line 22 7".into(),
                "stroke".into(),
            ]
        );
        // A window that leaves the pool, or holds no pair, draws nothing and touches no state.
        assert!(run(&[segments(3, 2)], &points).is_empty());
        assert!(run(&[segments(1, 0)], &points).is_empty());
        assert!(run(&[segments(u32::MAX, 1)], &points).is_empty());
    }

    #[test]
    fn area_fill_closes_down_to_base_with_gradient() {
        let points = [[0.0f32, 10.0], [20.0, 4.0]];
        let g = Gradient {
            top: Color::rgb(0, 0, 0xFF),
            bottom: Color::rgba(0, 0, 0xFF, 0),
        };
        let ops = run(
            &[Prim::AreaFill {
                first_point: 0,
                point_count: 2,
                base_y: 40.0,
                line_type: LineType::Simple,
                gradient: g,
            }],
            &points,
        );
        // y_top is the min point y (4); gradient spans 4..40
        assert_eq!(ops[0], "fill_grad 4 40 0000ffff 0000ff00");
        assert_eq!(ops[1], "begin");
        assert_eq!(ops[2], "move 0 10");
        assert_eq!(ops[3], "line 20 4");
        assert_eq!(ops[4], "line 20 40"); // down to base at last x
        assert_eq!(ops[5], "line 0 40"); // back to base at first x
        assert_eq!(ops[6], "close");
        assert_eq!(ops[7], "fill");
    }

    #[test]
    fn circle_fills_and_optionally_strokes() {
        let no_stroke = run(
            &[Prim::Circle {
                cx: 5.0,
                cy: 6.0,
                radius: 3.0,
                fill: C,
                stroke_width: 0.0,
                stroke: C,
            }],
            &[],
        );
        assert_eq!(no_stroke.iter().filter(|o| *o == "stroke").count(), 0);
        assert!(no_stroke.contains(&"fill".to_string()));

        let with_stroke = run(
            &[Prim::Circle {
                cx: 5.0,
                cy: 6.0,
                radius: 3.0,
                fill: C,
                stroke_width: 1.5,
                stroke: C,
            }],
            &[],
        );
        assert_eq!(with_stroke.iter().filter(|o| *o == "stroke").count(), 1);
    }

    #[test]
    fn degenerate_circles_and_polyline_widths_emit_no_canvas_commands() {
        for radius in [-3.0, 0.0, f32::NAN, f32::INFINITY] {
            assert!(
                run(
                    &[Prim::Circle {
                        cx: 5.0,
                        cy: 6.0,
                        radius,
                        fill: C,
                        stroke_width: 2.0,
                        stroke: C,
                    }],
                    &[]
                )
                .is_empty(),
                "radius {radius}"
            );
        }
        let points = [[0.0, 0.0], [10.0, 10.0]];
        for width in [-2.0, 0.0, f32::NAN, f32::INFINITY] {
            assert!(
                run(
                    &[Prim::Polyline {
                        first_point: 0,
                        point_count: 2,
                        width,
                        style: LineStyle::Solid,
                        line_type: LineType::Simple,
                        color: C,
                    }],
                    &points
                )
                .is_empty(),
                "width {width}"
            );
        }
    }

    #[test]
    fn background_fills_its_rect_with_a_gradient_spanning_it() {
        let g = Gradient {
            top: Color::rgb(1, 2, 3),
            bottom: Color::rgb(4, 5, 6),
        };
        let ops = run(
            &[Prim::Background {
                rect: [20.0, 10.0, 160.0, 60.0],
                gradient: g,
            }],
            &[],
        );
        assert_eq!(ops[0], "fill_grad 10 70 010203ff 040506ff");
        assert_eq!(ops[1], "fill_rect 20 10 160 60");
    }

    #[test]
    fn text_prim_draws_with_font_spec_and_empty_text_is_skipped() {
        let ops = run(
            &[
                Prim::Text {
                    x: 10.5,
                    y: 20.0,
                    text: "hello".into(),
                    color: C,
                    size: 24.0,
                    family: "Roboto".into(),
                    align: TextAlign::Center,
                    weight: 700,
                    italic: false,
                },
                Prim::Text {
                    x: 0.0,
                    y: 0.0,
                    text: String::new(),
                    color: C,
                    size: 12.0,
                    family: "Roboto".into(),
                    align: TextAlign::Left,
                    weight: 400,
                    italic: false,
                },
            ],
            &[],
        );
        assert_eq!(
            ops,
            vec![r#"text "hello" 10.5 20 "700 24px Roboto" 102030ff center"#.to_string()]
        );
    }

    #[test]
    fn font_spec_uses_numeric_weight_and_plain_size() {
        assert_eq!(
            text_font_spec(12.0, "sans-serif", 400, false),
            "400 12px sans-serif"
        );
        assert_eq!(
            text_font_spec(11.5, "sans-serif", 700, false),
            "700 11.5px sans-serif"
        );
        assert_eq!(
            text_font_spec(11.5, "sans-serif", 600, true),
            "italic 600 11.5px sans-serif"
        );
    }
}
