//! Ordered `Prim` dispatch: lowers one layer of the backend-neutral draw list into
//! [`SceneOp`]s, appended to a [`ScenePlan`] in strict paint order.
//!
//! This is the GPUI counterpart of `aeris_charts_render::canvas2d::execute` and
//! `aeris_charts_render_wgpu`'s quad/tri executors, and it deliberately mirrors their control flow arm
//! for arm so the three stay comparable under review.
//!
//! Ordering guarantee: ops are appended in prim order and never reordered or merged.
//! GPUI's `Scene::insert_primitive` assigns each primitive a draw order of
//! `1 + max(order of every primitive it intersects)` (see `gpui::bounds_tree`), then sorts by that
//! order — so insertion order is preserved for anything that overlaps, which is exactly the
//! guarantee a chart needs. Non-overlapping primitives may be batched out of order, which is
//! unobservable. The adapter therefore must *not* wrap the chart in `Window::paint_layer`: a layer
//! forces every primitive inside it to share one order, which would flatten the chart's z-order.

use aeris_charts_render::draw_list::{LineStyle, Prim};

use crate::geometry::{
    area_fill_mesh, band_fill_mesh, dash_spans, dashed_polyline_meshes, disc_mesh, fill_polygon,
    irect, line_span_start, polyline_mesh, rect_frame_edges, ring_mesh, round_rect_polygon,
    segments_mesh, Scratch,
};
use crate::metrics::GpuiFrameMetrics;
use crate::scene::{DeviceRect, Paint, SceneOp, ScenePlan, TextRun};

/// Knobs that change *how* a prim is expressed in GPUI without changing what it means.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ExecutorOptions {
    /// Express `Prim::RoundRect` as a single GPUI rounded `paint_quad` instead of Aeris's
    /// tessellated polygon.
    ///
    /// `false` (the default) reproduces `aeris_charts_render_wgpu`'s `round_rect_polygon` triangle-for-
    /// triangle, so GPUI matches the current WebGPU output by construction. `true` hands the
    /// rounding to GPUI's analytic rounded-rect shader, which is smoother and GPUI-idiomatic but
    /// is *not* pixel-identical to the current output — opt in only where that is wanted.
    pub native_round_rects: bool,
}

/// Lower one layer of prims into `plan`, appending to whatever is already there.
///
/// `points` is the layer's shared point pool (indexed by `Polyline`/`Segments`/`AreaFill`/`BandFill`).
/// `metrics` accumulates across every call in a frame.
pub fn execute_layer(
    prims: &[Prim],
    points: &[[f32; 2]],
    options: ExecutorOptions,
    scratch: &mut Scratch,
    plan: &mut ScenePlan,
    metrics: &mut GpuiFrameMetrics,
) {
    for prim in prims {
        metrics.prims += 1;
        let before = plan.ops.len();
        lower_prim(prim, points, options, scratch, plan, metrics);
        if plan.ops.len() == before {
            metrics.dropped_prims += 1;
        }
    }
}

fn lower_prim(
    prim: &Prim,
    points: &[[f32; 2]],
    options: ExecutorOptions,
    scratch: &mut Scratch,
    plan: &mut ScenePlan,
    metrics: &mut GpuiFrameMetrics,
) {
    match prim {
        Prim::Rect { rect, color } => {
            if let Some(rect) = irect(*rect) {
                push_quad(plan, metrics, rect, Paint::Solid(*color));
            }
        }

        Prim::RectFrame {
            rect,
            border,
            color,
        } => {
            for edge in rect_frame_edges(*rect, *border) {
                if let Some(rect) = irect(edge) {
                    push_quad(plan, metrics, rect, Paint::Solid(*color));
                }
            }
        }

        Prim::HLine {
            y,
            x0,
            x1,
            width,
            style,
            color,
        } => {
            let top = line_span_start(*y, *width);
            let (w, c) = (*width, *color);
            // The span callback writes straight into the plan. Collecting spans into a temporary
            // `Vec` first would allocate once per line prim — and candle wicks alone are thousands
            // of `VLine`s per frame, which showed up directly in the scene-construction p99.
            dash_spans(*style, w, *x0, *x1, |a, b| {
                if let Some(rect) = irect(aeris_charts_render::draw_list::IRect {
                    x: a,
                    y: top,
                    w: b - a,
                    h: w,
                }) {
                    push_quad(plan, metrics, rect, Paint::Solid(c));
                }
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
            let left = line_span_start(*x, *width);
            let (w, c) = (*width, *color);
            dash_spans(*style, w, *y0, *y1, |a, b| {
                if let Some(rect) = irect(aeris_charts_render::draw_list::IRect {
                    x: left,
                    y: a,
                    w,
                    h: b - a,
                }) {
                    push_quad(plan, metrics, rect, Paint::Solid(c));
                }
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
            // A solid style tessellates the whole run; a dashed one is split into solid runs
            // first, exactly as `aeris_charts_render::line::push_line_stroke` does for the strokes
            // the engine and the browser decoder produce. Doing it here too means a dashed
            // `Polyline` arriving from a host plugin (Terminal-side GPUI plugins) still dashes.
            // The wgpu tri executor still ignores `style`, which is why every producer feeding it
            // must pre-lower dashes and this path stays only for GPUI-native hosts.
            let pattern = style.dash_pattern(*width);
            if pattern.is_empty() {
                let range = polyline_mesh(
                    scratch,
                    &mut plan.vertices,
                    points,
                    *first_point,
                    *point_count,
                    *width,
                    *line_type,
                );
                push_mesh(plan, metrics, range, Paint::Solid(*color));
            } else {
                dashed_polyline_meshes(
                    scratch,
                    &mut plan.vertices,
                    points,
                    *first_point,
                    *point_count,
                    *width,
                    *line_type,
                    &pattern,
                );
                // Indexing rather than iterating keeps `scratch` unborrowed across `push_mesh`.
                for i in 0..scratch.ranges.len() {
                    let range = scratch.ranges[i];
                    push_mesh(plan, metrics, range, Paint::Solid(*color));
                }
            }
        }

        Prim::Segments {
            first_point,
            segment_count,
            width,
            color,
        } => {
            // One contiguous mesh for the batch: `push_mesh` chunks it at the path-buffer bound, and
            // an out-of-pool range yields an empty mesh that `execute_layer` counts as dropped.
            let range = segments_mesh(
                &mut plan.vertices,
                points,
                *first_point,
                *segment_count,
                *width,
            );
            push_mesh(plan, metrics, range, Paint::Solid(*color));
        }

        Prim::AreaFill {
            first_point,
            point_count,
            base_y,
            line_type,
            gradient,
        } => {
            let (range, paint) = area_fill_mesh(
                scratch,
                &mut plan.vertices,
                points,
                *first_point,
                *point_count,
                *base_y,
                *line_type,
                gradient.top,
                gradient.bottom,
            );
            push_mesh(plan, metrics, range, paint);
        }

        Prim::BandFill {
            upper_first,
            lower_first,
            point_count,
            line_type,
            fill,
        } => {
            let range = band_fill_mesh(
                scratch,
                &mut plan.vertices,
                points,
                *upper_first,
                *lower_first,
                *point_count,
                *line_type,
            );
            push_mesh(plan, metrics, range, Paint::Solid(*fill));
        }

        Prim::Circle {
            cx,
            cy,
            radius,
            fill,
            stroke_width,
            stroke,
        } => {
            if *radius > 0.0 {
                let range = disc_mesh(&mut plan.vertices, *cx, *cy, *radius);
                push_mesh(plan, metrics, range, Paint::Solid(*fill));
            }
            // Stroke after fill, matching Canvas2D, native, and the WebGPU annulus tessellation.
            if *stroke_width > 0.0 {
                let range = ring_mesh(&mut plan.vertices, *cx, *cy, *radius, *stroke_width);
                push_mesh(plan, metrics, range, Paint::Solid(*stroke));
            }
        }

        Prim::Triangle { a, b, c, color } => {
            let range = crate::geometry::push_vertices(&mut plan.vertices, [*a, *b, *c]);
            push_mesh(plan, metrics, range, Paint::Solid(*color));
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
                return;
            }
            if options.native_round_rects {
                plan.ops.push(SceneOp::Quad {
                    rect: DeviceRect::new(*x, *y, *w, *h),
                    fill: Paint::Solid(*fill),
                    corner_radii: *radii,
                    border_width: *border_width,
                    border_color: *border_color,
                });
                metrics.quads += 1;
                metrics.ops += 1;
                return;
            }
            // Aeris-generated strips: the wgpu executor's exact construction — the border as the
            // outer polygon, the fill as an inset polygon on top.
            let outer = round_rect_polygon(*x, *y, *w, *h, *radii);
            if *border_width > 0.0 {
                let range = fill_polygon(&mut plan.vertices, &outer);
                push_mesh(plan, metrics, range, Paint::Solid(*border_color));
                let inset = border_width.min(*w / 2.0).min(*h / 2.0);
                let inner_radii = radii.map(|r| (r - inset).max(0.0));
                let inner = round_rect_polygon(
                    x + inset,
                    y + inset,
                    w - inset * 2.0,
                    h - inset * 2.0,
                    inner_radii,
                );
                let range = fill_polygon(&mut plan.vertices, &inner);
                push_mesh(plan, metrics, range, Paint::Solid(*fill));
            } else {
                let range = fill_polygon(&mut plan.vertices, &outer);
                push_mesh(plan, metrics, range, Paint::Solid(*fill));
            }
        }

        Prim::Background { rect, gradient } => {
            let [x, y, w, h] = *rect;
            let rect = DeviceRect::new(x, y, w, h);
            if rect.is_empty() {
                return;
            }
            // The Canvas2D executor spans the ramp over `[y, y + h]` — the rect itself — so a
            // bounds-relative GPUI gradient over the same quad is exact.
            push_quad(
                plan,
                metrics,
                rect,
                Paint::VGradient {
                    top: gradient.top,
                    bottom: gradient.bottom,
                },
            );
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
            if text.is_empty() || *size <= 0.0 || color.a() == 0 {
                return;
            }
            plan.ops.push(SceneOp::Text(TextRun {
                x: *x,
                y: *y,
                text: text.clone(),
                color: *color,
                size: *size,
                family: family.clone(),
                align: *align,
                weight: *weight,
                italic: *italic,
                angle: 0.0,
            }));
            metrics.text_runs += 1;
            metrics.ops += 1;
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
            if text.is_empty() || *size <= 0.0 || color.a() == 0 {
                return;
            }
            plan.ops.push(SceneOp::Text(TextRun {
                x: *x,
                y: *y,
                text: text.clone(),
                color: *color,
                size: *size,
                family: family.clone(),
                align: *align,
                weight: *weight,
                italic: *italic,
                angle: *angle,
            }));
            metrics.text_runs += 1;
            metrics.ops += 1;
        }
        Prim::Image {
            image,
            rect,
            opacity,
        } => {
            let rect = DeviceRect::new(rect[0], rect[1], rect[2], rect[3]);
            if rect.is_empty()
                || image.width == 0
                || image.height == 0
                || *opacity <= 0.0
                || image.pixels.len() != (image.width * image.height * 4) as usize
            {
                return;
            }
            plan.ops.push(SceneOp::Image {
                image: image.clone(),
                rect,
                opacity: opacity.clamp(0.0, 1.0),
            });
            metrics.image_runs += 1;
            metrics.ops += 1;
        }
    }
}

fn push_quad(plan: &mut ScenePlan, metrics: &mut GpuiFrameMetrics, rect: DeviceRect, fill: Paint) {
    if rect.is_empty() {
        return;
    }
    plan.ops.push(SceneOp::Quad {
        rect,
        fill,
        corner_radii: [0.0; 4],
        border_width: 0.0,
        border_color: aeris_charts_render::color::Color::rgba(0, 0, 0, 0),
    });
    metrics.quads += 1;
    metrics.ops += 1;
}

/// The largest mesh handed to one GPUI `Path`. GPUI's wgpu renderer copies every path vertex
/// (position, `st`, a fat `Background`, bounds) into a fixed 2 MiB instance buffer and, on
/// overflow, grows the buffer and re-encodes the whole frame — so one unbounded stroke can stall
/// the window in a grow-and-redraw loop. Chunking keeps each path upload bounded; the mesh is a
/// triangle soup, so splitting it changes neither coverage nor paint order.
const MAX_MESH_VERTICES: u32 = 12_288;

fn push_mesh(
    plan: &mut ScenePlan,
    metrics: &mut GpuiFrameMetrics,
    (first_vertex, vertex_count): (u32, u32),
    fill: Paint,
) {
    if vertex_count < 3 {
        return;
    }
    let mut offset = first_vertex;
    let mut remaining = vertex_count;
    while remaining > 0 {
        let chunk = remaining.min(MAX_MESH_VERTICES);
        plan.ops.push(SceneOp::Mesh {
            first_vertex: offset,
            vertex_count: chunk,
            fill,
        });
        metrics.paths += 1;
        metrics.ops += 1;
        offset += chunk;
        remaining -= chunk;
    }
    metrics.triangles += vertex_count / 3;
}

/// Push a clip rect, returning `false` when it is degenerate (nothing was pushed).
pub(crate) fn push_clip(
    plan: &mut ScenePlan,
    metrics: &mut GpuiFrameMetrics,
    rect: DeviceRect,
) -> bool {
    if rect.is_empty() {
        return false;
    }
    plan.ops.push(SceneOp::PushClip(rect));
    metrics.clips += 1;
    metrics.ops += 1;
    true
}

pub(crate) fn pop_clip(plan: &mut ScenePlan, metrics: &mut GpuiFrameMetrics) {
    plan.ops.push(SceneOp::PopClip);
    metrics.ops += 1;
}

/// Whether `style` dashes at `width` (exposed for tests and host diagnostics).
pub fn dashes(style: LineStyle, width: f32) -> bool {
    !style.dash_pattern(width).is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;
    use aeris_charts_render::color::Color;
    use aeris_charts_render::draw_list::{Gradient, IRect, LineType, RasterImage, TextAlign};
    use std::sync::Arc;

    const C: Color = Color::rgb(0x10, 0x20, 0x30);

    fn run(prims: &[Prim], points: &[[f32; 2]]) -> (ScenePlan, GpuiFrameMetrics) {
        let mut plan = ScenePlan::default();
        let mut metrics = GpuiFrameMetrics::default();
        execute_layer(
            prims,
            points,
            ExecutorOptions::default(),
            &mut Scratch::default(),
            &mut plan,
            &mut metrics,
        );
        (plan, metrics)
    }

    fn quads(plan: &ScenePlan) -> Vec<(DeviceRect, Paint)> {
        plan.ops
            .iter()
            .filter_map(|op| match op {
                SceneOp::Quad { rect, fill, .. } => Some((*rect, *fill)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn rect_lowers_to_one_quad() {
        let (plan, metrics) = run(
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
            quads(&plan),
            vec![(DeviceRect::new(3.0, 4.0, 10.0, 6.0), Paint::Solid(C))]
        );
        assert_eq!(metrics.quads, 1);
        assert_eq!(metrics.dropped_prims, 0);
    }

    #[test]
    fn degenerate_rect_is_counted_as_dropped() {
        let (plan, metrics) = run(
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
        assert!(plan.ops.is_empty());
        assert_eq!(metrics.prims, 1);
        assert_eq!(metrics.dropped_prims, 1);
    }

    #[test]
    fn rect_frame_lowers_to_four_quads_in_edge_order() {
        let (plan, _) = run(
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
        let rects: Vec<_> = quads(&plan).into_iter().map(|(r, _)| r).collect();
        assert_eq!(
            rects,
            vec![
                DeviceRect::new(11.0, 20.0, 6.0, 1.0),
                DeviceRect::new(11.0, 25.0, 6.0, 1.0),
                DeviceRect::new(10.0, 20.0, 1.0, 6.0),
                DeviceRect::new(17.0, 20.0, 1.0, 6.0),
            ]
        );
    }

    #[test]
    fn hline_centers_odd_width_on_its_coordinate() {
        let (plan, _) = run(
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
            quads(&plan).into_iter().map(|(r, _)| r).collect::<Vec<_>>(),
            vec![DeviceRect::new(10.0, 50.0, 30.0, 1.0)]
        );
    }

    #[test]
    fn dashed_vline_emits_only_the_on_spans() {
        let (plan, _) = run(
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
            quads(&plan).into_iter().map(|(r, _)| r).collect::<Vec<_>>(),
            vec![
                DeviceRect::new(5.0, 0.0, 1.0, 6.0),
                DeviceRect::new(5.0, 12.0, 1.0, 6.0),
            ]
        );
    }

    #[test]
    fn background_uses_a_bounds_relative_gradient() {
        let g = Gradient {
            top: Color::rgb(1, 2, 3),
            bottom: Color::rgb(4, 5, 6),
        };
        let (plan, _) = run(
            &[Prim::Background {
                rect: [20.0, 10.0, 160.0, 60.0],
                gradient: g,
            }],
            &[],
        );
        assert_eq!(
            quads(&plan),
            vec![(
                DeviceRect::new(20.0, 10.0, 160.0, 60.0),
                Paint::VGradient {
                    top: g.top,
                    bottom: g.bottom
                }
            )]
        );
    }

    #[test]
    fn polyline_tessellates_to_a_single_mesh() {
        let points = [[0.0f32, 0.0], [10.0, 5.0], [20.0, 0.0]];
        let (plan, metrics) = run(
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
        assert_eq!(metrics.paths, 1);
        assert!(metrics.triangles >= 4, "two segments plus a round join");
        let SceneOp::Mesh { fill, .. } = &plan.ops[0] else {
            panic!("expected a mesh, got {:?}", plan.ops[0]);
        };
        assert_eq!(*fill, Paint::Solid(C));
    }

    #[test]
    fn an_oversized_mesh_is_chunked_into_bounded_paths() {
        // 1100 collinear points: 1099 segments of 12 vertices plus caps, past MAX_MESH_VERTICES.
        let points: Vec<[f32; 2]> = (0..1100).map(|i| [i as f32, 0.0]).collect();
        let (plan, metrics) = run(
            &[Prim::Polyline {
                first_point: 0,
                point_count: points.len() as u32,
                width: 2.0,
                style: LineStyle::Solid,
                line_type: LineType::Simple,
                color: C,
            }],
            &points,
        );
        assert_eq!(metrics.paths, 2, "the mesh splits into bounded chunks");
        let total: u32 = plan
            .ops
            .iter()
            .map(|op| match op {
                SceneOp::Mesh { vertex_count, .. } => *vertex_count,
                _ => 0,
            })
            .sum();
        assert_eq!(total, metrics.triangles * 3, "chunking drops no triangles");
        assert!(
            plan.ops.iter().all(
                |op| matches!(op, SceneOp::Mesh { vertex_count, .. } if *vertex_count <= 12_288)
            ),
            "every chunk fits one bounded GPUI path upload"
        );
    }

    #[test]
    fn segments_lower_to_bounded_meshes() {
        // 3,000 touching one-bar pairs, far past one path upload: the single batch must chunk.
        const PAIRS: u32 = 3_000;
        let points: Vec<[f32; 2]> = (0..PAIRS)
            .flat_map(|pair| {
                let x = pair as f32 * 4.0;
                [[x, 5.0], [x + 4.0, 5.0]]
            })
            .collect();
        let (plan, metrics) = run(
            &[Prim::Segments {
                first_point: 0,
                segment_count: PAIRS,
                width: 2.0,
                color: C,
            }],
            &points,
        );
        let separate: Vec<Prim> = (0..PAIRS)
            .map(|pair| Prim::Polyline {
                first_point: 2 * pair,
                point_count: 2,
                width: 2.0,
                style: LineStyle::Solid,
                line_type: LineType::Simple,
                color: C,
            })
            .collect();
        let (_, reference) = run(&separate, &points);
        assert_eq!(reference.paths, PAIRS, "separate strokes are a path each");
        assert_eq!(metrics.dropped_prims, 0);
        assert_eq!(
            metrics.triangles, reference.triangles,
            "the batch tessellates exactly the separate strokes' triangles"
        );
        assert_eq!(
            metrics.paths,
            (metrics.triangles * 3).div_ceil(12_288),
            "one mesh, chunked at the bounded path size"
        );
        assert!(
            plan.ops.iter().all(
                |op| matches!(op, SceneOp::Mesh { vertex_count, .. } if *vertex_count <= 12_288)
            ),
            "every chunk fits one bounded GPUI path upload"
        );

        // A window that leaves the pool lowers to nothing and counts as dropped.
        let (plan, metrics) = run(
            &[Prim::Segments {
                first_point: 2 * PAIRS - 2,
                segment_count: 2,
                width: 2.0,
                color: C,
            }],
            &points,
        );
        assert!(plan.ops.is_empty());
        assert_eq!(metrics.dropped_prims, 1);
    }

    #[test]
    fn dashed_polyline_splits_into_several_meshes() {
        let points: Vec<[f32; 2]> = (0..2).map(|i| [i as f32 * 200.0, 0.0]).collect();
        let (_, metrics) = run(
            &[Prim::Polyline {
                first_point: 0,
                point_count: 2,
                width: 2.0,
                style: LineStyle::Dashed,
                line_type: LineType::Simple,
                color: C,
            }],
            &points,
        );
        assert!(
            metrics.paths > 1,
            "a 200px dashed run must break into multiple solid runs, got {}",
            metrics.paths
        );
    }

    #[test]
    fn short_polyline_is_dropped() {
        let (plan, metrics) = run(
            &[Prim::Polyline {
                first_point: 0,
                point_count: 1,
                width: 2.0,
                style: LineStyle::Solid,
                line_type: LineType::Simple,
                color: C,
            }],
            &[[0.0, 0.0]],
        );
        assert!(plan.ops.is_empty());
        assert_eq!(metrics.dropped_prims, 1);
    }

    #[test]
    fn circle_fills_then_strokes() {
        let (plan, metrics) = run(
            &[Prim::Circle {
                cx: 10.0,
                cy: 10.0,
                radius: 6.0,
                fill: C,
                stroke_width: 2.0,
                stroke: Color::rgb(0xff, 0xff, 0xff),
            }],
            &[],
        );
        assert_eq!(metrics.paths, 2, "disc then ring");
        let fills: Vec<_> = plan
            .ops
            .iter()
            .filter_map(|op| match op {
                SceneOp::Mesh { fill, .. } => Some(*fill),
                _ => None,
            })
            .collect();
        assert_eq!(
            fills,
            vec![Paint::Solid(C), Paint::Solid(Color::rgb(0xff, 0xff, 0xff))]
        );
    }

    #[test]
    fn circle_without_stroke_emits_only_the_disc() {
        let (_, metrics) = run(
            &[Prim::Circle {
                cx: 10.0,
                cy: 10.0,
                radius: 6.0,
                fill: C,
                stroke_width: 0.0,
                stroke: C,
            }],
            &[],
        );
        assert_eq!(metrics.paths, 1);
    }

    #[test]
    fn round_rect_defaults_to_base_tessellation_not_a_native_quad() {
        let prim = Prim::RoundRect {
            x: 1.0,
            y: 2.0,
            w: 20.0,
            h: 10.0,
            radii: [2.0; 4],
            fill: C,
            border_width: 1.0,
            border_color: Color::rgb(9, 9, 9),
        };
        let prims = [prim];
        let (plan, metrics) = run(&prims, &[]);
        assert_eq!(metrics.quads, 0);
        assert_eq!(metrics.paths, 2, "border polygon then inset fill polygon");
        assert!(plan.ops.iter().all(|op| matches!(op, SceneOp::Mesh { .. })));

        let mut native_plan = ScenePlan::default();
        let mut native_metrics = GpuiFrameMetrics::default();
        execute_layer(
            &prims,
            &[],
            ExecutorOptions {
                native_round_rects: true,
            },
            &mut Scratch::default(),
            &mut native_plan,
            &mut native_metrics,
        );
        assert_eq!(native_metrics.quads, 1);
        assert_eq!(native_metrics.paths, 0);
        let SceneOp::Quad {
            corner_radii,
            border_width,
            ..
        } = &native_plan.ops[0]
        else {
            panic!("expected a quad");
        };
        assert_eq!(*corner_radii, [2.0; 4]);
        assert_eq!(*border_width, 1.0);
    }

    #[test]
    fn text_carries_the_prim_anchor_verbatim() {
        let (plan, metrics) = run(
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
        assert_eq!(metrics.text_runs, 1, "the empty run is dropped");
        assert_eq!(metrics.dropped_prims, 1);
        let SceneOp::Text(run) = &plan.ops[0] else {
            panic!("expected text");
        };
        assert_eq!((run.x, run.y, run.align), (10.5, 20.0, TextAlign::Center));
        assert_eq!(run.weight, 700);
    }

    #[test]
    fn rotated_text_carries_the_complete_transform_verbatim() {
        let (plan, metrics) = run(
            &[Prim::RotatedText {
                x: 10.5,
                y: 20.0,
                text: "trend".into(),
                color: C,
                size: 14.0,
                family: "Roboto".into(),
                align: TextAlign::Right,
                weight: 600,
                italic: true,
                angle: -0.75,
            }],
            &[],
        );
        assert_eq!(metrics.text_runs, 1);
        let SceneOp::Text(run) = &plan.ops[0] else {
            panic!("expected text");
        };
        assert_eq!((run.x, run.y, run.angle), (10.5, 20.0, -0.75));
        assert_eq!(
            (run.align, run.weight, run.italic),
            (TextAlign::Right, 600, true)
        );
    }

    #[test]
    fn image_carries_shared_pixels_and_placement_into_the_scene() {
        let (plan, metrics) = run(
            &[Prim::Image {
                image: RasterImage {
                    key: 9,
                    width: 1,
                    height: 1,
                    pixels: Arc::<[u8]>::from([1, 2, 3, 255]),
                },
                rect: [10.0, 20.0, 30.0, 40.0],
                opacity: 0.25,
            }],
            &[],
        );
        assert_eq!(metrics.image_runs, 1);
        let SceneOp::Image {
            image,
            rect,
            opacity,
        } = &plan.ops[0]
        else {
            panic!("expected image");
        };
        assert_eq!(image.key, 9);
        assert_eq!(*rect, DeviceRect::new(10.0, 20.0, 30.0, 40.0));
        assert_eq!(*opacity, 0.25);
    }

    #[test]
    fn fully_transparent_text_is_dropped() {
        let (plan, _) = run(
            &[Prim::Text {
                x: 0.0,
                y: 0.0,
                text: "invisible".into(),
                color: Color::rgba(0, 0, 0, 0),
                size: 12.0,
                family: "sans-serif".into(),
                align: TextAlign::Left,
                weight: 400,
                italic: false,
            }],
            &[],
        );
        assert!(plan.ops.is_empty());
    }

    #[test]
    fn ops_follow_prim_order_exactly() {
        let prims = vec![
            Prim::Background {
                rect: [0.0, 0.0, 100.0, 100.0],
                gradient: Gradient { top: C, bottom: C },
            },
            Prim::Rect {
                rect: IRect {
                    x: 1,
                    y: 1,
                    w: 2,
                    h: 2,
                },
                color: C,
            },
            Prim::Triangle {
                a: [0.0, 0.0],
                b: [1.0, 0.0],
                c: [1.0, 1.0],
                color: C,
            },
            Prim::Text {
                x: 0.0,
                y: 0.0,
                text: "t".into(),
                color: C,
                size: 10.0,
                family: "sans-serif".into(),
                align: TextAlign::Left,
                weight: 400,
                italic: false,
            },
        ];
        let (plan, _) = run(&prims, &[]);
        let kinds: Vec<&str> = plan
            .ops
            .iter()
            .map(|op| match op {
                SceneOp::Quad { .. } => "quad",
                SceneOp::Mesh { .. } => "mesh",
                SceneOp::Text(_) => "text",
                SceneOp::Image { .. } => "image",
                SceneOp::PushClip(_) => "push",
                SceneOp::PopClip => "pop",
            })
            .collect();
        assert_eq!(kinds, vec!["quad", "quad", "mesh", "text"]);
    }

    #[test]
    fn every_prim_variant_lowers_to_at_least_one_op() {
        // Guards the invariant that every current `Prim` variant has a GPUI implementation: if a
        // variant is added to the IR, this fails until it is mapped here.
        let points = vec![
            [0.0f32, 0.0],
            [10.0, 5.0],
            [20.0, 0.0],
            [0.0, 20.0],
            [10.0, 25.0],
            [20.0, 20.0],
        ];
        let cases: Vec<(&str, Prim)> = vec![
            (
                "Rect",
                Prim::Rect {
                    rect: IRect {
                        x: 0,
                        y: 0,
                        w: 4,
                        h: 4,
                    },
                    color: C,
                },
            ),
            (
                "RectFrame",
                Prim::RectFrame {
                    rect: IRect {
                        x: 0,
                        y: 0,
                        w: 8,
                        h: 8,
                    },
                    border: 1,
                    color: C,
                },
            ),
            (
                "HLine",
                Prim::HLine {
                    y: 5,
                    x0: 0,
                    x1: 10,
                    width: 1,
                    style: LineStyle::Solid,
                    color: C,
                },
            ),
            (
                "VLine",
                Prim::VLine {
                    x: 5,
                    y0: 0,
                    y1: 10,
                    width: 1,
                    style: LineStyle::Solid,
                    color: C,
                },
            ),
            (
                "Polyline",
                Prim::Polyline {
                    first_point: 0,
                    point_count: 3,
                    width: 2.0,
                    style: LineStyle::Solid,
                    line_type: LineType::Simple,
                    color: C,
                },
            ),
            (
                "AreaFill",
                Prim::AreaFill {
                    first_point: 0,
                    point_count: 3,
                    base_y: 40.0,
                    line_type: LineType::Simple,
                    gradient: Gradient { top: C, bottom: C },
                },
            ),
            (
                "BandFill",
                Prim::BandFill {
                    line_type: LineType::Simple,
                    upper_first: 0,
                    lower_first: 3,
                    point_count: 3,
                    fill: C,
                },
            ),
            (
                "RoundRect",
                Prim::RoundRect {
                    x: 0.0,
                    y: 0.0,
                    w: 10.0,
                    h: 6.0,
                    radii: [1.0; 4],
                    fill: C,
                    border_width: 0.0,
                    border_color: C,
                },
            ),
            (
                "Circle",
                Prim::Circle {
                    cx: 5.0,
                    cy: 5.0,
                    radius: 3.0,
                    fill: C,
                    stroke_width: 0.0,
                    stroke: C,
                },
            ),
            (
                "Triangle",
                Prim::Triangle {
                    a: [0.0, 0.0],
                    b: [4.0, 0.0],
                    c: [4.0, 4.0],
                    color: C,
                },
            ),
            (
                "Background",
                Prim::Background {
                    rect: [0.0, 0.0, 10.0, 10.0],
                    gradient: Gradient { top: C, bottom: C },
                },
            ),
            (
                "Text",
                Prim::Text {
                    x: 0.0,
                    y: 0.0,
                    text: "x".into(),
                    color: C,
                    size: 10.0,
                    family: "sans-serif".into(),
                    align: TextAlign::Left,
                    weight: 400,
                    italic: false,
                },
            ),
        ];
        for (name, prim) in cases {
            let (plan, metrics) = run(&[prim], &points);
            assert!(
                !plan.ops.is_empty(),
                "{name} lowered to nothing (metrics: {metrics:?})"
            );
        }
    }

    #[test]
    fn dashes_reports_the_shared_pattern_rule() {
        assert!(!dashes(LineStyle::Solid, 1.0));
        assert!(dashes(LineStyle::Dotted, 1.0));
        assert!(dashes(LineStyle::Dashed, 1.0));
    }
}
