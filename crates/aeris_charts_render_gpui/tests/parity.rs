//! Cross-backend parity: the GPUI executor against the Canvas2D executor, on the same frame.
//!
//! # What this proves, and what it does not
//!
//! It does **not** compare rasterized pixels because GPUI exposes no GPU readback. It compares the
//! *draw stream*: for every
//! primitive, the exact geometry and color each backend is asked to fill.
//!
//! For the crisp-rect subset (`Rect`, `RectFrame`, `HLine`, `VLine`, `Background`) that is a
//! stronger statement than an image diff at one DPR, because both backends resolve to
//! "fill this axis-aligned rectangle with this color". If the rectangle lists and the color lists
//! are identical, the rasterized output is identical for any solid axis-aligned filler — there is
//! no antialiasing, no shaping, and no tessellation freedom left to differ over. That subset is the
//! large majority of a candlestick chart's primitives.
//!
//! For the tessellated subset the comparison is structural (same path, same closing edges, same
//! gradient extents), because Canvas2D describes curves analytically while GPUI and WebGPU both
//! consume triangles. Those are exactly the prims the existing WebGPU-vs-Canvas2D reference already
//! records a bounded residual for.

use aeris_charts_engine::{
    AxisDimension, CategoryScaleType, ChartEngine, ContinuousScaleType, GeneralAxisDomain,
    GeneralAxisOptions, GeneralReferenceOptions, GeneralReferenceValue, GeneralScaleType,
    GeneralSeriesOptions, GeneralXyInput, HorizontalDomain, IndicatorInputSource, IndicatorKind,
    IndicatorOutputStyle, OrderId, OrderKind, OrderRole, OrderSide, OrderStatus, PositionId,
    PositionSide, SeriesKind, TradingPosition, TradingPriceScale, WorkingOrder,
};
use aeris_charts_render::canvas2d::{execute as canvas_execute, Canvas2d, Viewport};
use aeris_charts_render::color::Color;
use aeris_charts_render::draw_list::{Gradient, IRect, LineStyle, LineType, Prim, TextAlign};
use aeris_charts_render_gpui::{
    fixtures, ExecutorOptions, GpuiChartRenderer, GpuiFrameMetrics, Paint, PreparedAerisFrame,
    SceneOp, ScenePlan,
};

/// A Canvas2D target that records only what the crisp-rect subset does: the current fill style and
/// every `fill_rect`. Path calls are counted so a test can assert a prim went down the path route.
#[derive(Default)]
struct RectRecorder {
    fill: Option<Paint>,
    /// `(x, y, w, h, fill)` in call order.
    rects: Vec<(f32, f32, f32, f32, Paint)>,
    path_fills: usize,
    path_strokes: usize,
    text_runs: Vec<(String, f32, f32, String, Color, TextAlign)>,
    images: Vec<([f32; 4], f32, u32, u32)>,
    gradient_extents: Vec<(f32, f32, Color, Color)>,
}

impl Canvas2d for RectRecorder {
    fn set_fill_solid(&mut self, color: Color) {
        self.fill = Some(Paint::Solid(color));
    }
    fn set_fill_vgradient(&mut self, y_top: f32, y_bottom: f32, top: Color, bottom: Color) {
        self.fill = Some(Paint::VGradient { top, bottom });
        self.gradient_extents.push((y_top, y_bottom, top, bottom));
    }
    fn set_stroke(&mut self, _color: Color) {}
    fn set_line_width(&mut self, _width: f32) {}
    fn set_line_dash(&mut self, _pattern: &[f32]) {}
    fn fill_rect(&mut self, x: f32, y: f32, w: f32, h: f32) {
        let fill = self
            .fill
            .expect("a fill style is set before every fill_rect");
        self.rects.push((x, y, w, h, fill));
    }
    fn begin_path(&mut self) {}
    fn move_to(&mut self, _x: f32, _y: f32) {}
    fn line_to(&mut self, _x: f32, _y: f32) {}
    fn close_path(&mut self) {}
    fn arc(&mut self, _cx: f32, _cy: f32, _r: f32, _s: f32, _e: f32) {}
    fn stroke(&mut self) {
        self.path_strokes += 1;
    }
    fn fill(&mut self) {
        self.path_fills += 1;
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
        self.text_runs
            .push((text.into(), x, y, font.into(), color, align));
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
        self.text_runs
            .push((format!("{text}@{angle}"), x, y, font.into(), color, align));
    }
    fn draw_raster_image(
        &mut self,
        image: &aeris_charts_render::draw_list::RasterImage,
        rect: [f32; 4],
        opacity: f32,
    ) {
        self.images.push((rect, opacity, image.width, image.height));
    }
}

fn canvas_rects(prims: &[Prim], points: &[[f32; 2]]) -> RectRecorder {
    let mut r = RectRecorder::default();
    canvas_execute(
        prims,
        points,
        &mut r,
        Viewport {
            width: 2000.0,
            height: 2000.0,
        },
    );
    r
}

fn gpui_plan(prims: &[Prim], points: &[[f32; 2]]) -> (ScenePlan, GpuiFrameMetrics) {
    let mut plan = ScenePlan::default();
    let mut metrics = GpuiFrameMetrics::default();
    aeris_charts_render_gpui::executor::execute_layer(
        prims,
        points,
        ExecutorOptions::default(),
        &mut aeris_charts_render_gpui::geometry::Scratch::default(),
        &mut plan,
        &mut metrics,
    );
    (plan, metrics)
}

/// The GPUI plan's quads as `(x, y, w, h, fill)`, in emission order.
fn gpui_quads(plan: &ScenePlan) -> Vec<(f32, f32, f32, f32, Paint)> {
    plan.ops
        .iter()
        .filter_map(|op| match op {
            SceneOp::Quad { rect, fill, .. } => Some((rect.x, rect.y, rect.w, rect.h, *fill)),
            _ => None,
        })
        .collect()
}

/// Every rect-family prim, with the awkward cases (even/odd widths, dashes, degenerate extents,
/// negative coordinates) included deliberately.
fn rect_family_cases() -> Vec<(&'static str, Prim)> {
    let c = Color::rgba(0x26, 0xa6, 0x9a, 0xc0);
    vec![
        (
            "Rect",
            Prim::Rect {
                rect: IRect {
                    x: 3,
                    y: 4,
                    w: 10,
                    h: 6,
                },
                color: c,
            },
        ),
        (
            "Rect at a negative offset",
            Prim::Rect {
                rect: IRect {
                    x: -12,
                    y: -5,
                    w: 30,
                    h: 9,
                },
                color: c,
            },
        ),
        (
            "Rect with a zero width",
            Prim::Rect {
                rect: IRect {
                    x: 0,
                    y: 0,
                    w: 0,
                    h: 6,
                },
                color: c,
            },
        ),
        (
            "Rect with a negative height",
            Prim::Rect {
                rect: IRect {
                    x: 0,
                    y: 0,
                    w: 6,
                    h: -3,
                },
                color: c,
            },
        ),
        (
            "RectFrame border 1",
            Prim::RectFrame {
                rect: IRect {
                    x: 10,
                    y: 20,
                    w: 8,
                    h: 6,
                },
                border: 1,
                color: c,
            },
        ),
        (
            "RectFrame border 3",
            Prim::RectFrame {
                rect: IRect {
                    x: 10,
                    y: 20,
                    w: 41,
                    h: 27,
                },
                border: 3,
                color: c,
            },
        ),
        (
            "RectFrame thicker than half its width",
            Prim::RectFrame {
                rect: IRect {
                    x: 0,
                    y: 0,
                    w: 4,
                    h: 4,
                },
                border: 3,
                color: c,
            },
        ),
        (
            "HLine width 1",
            Prim::HLine {
                y: 50,
                x0: 10,
                x1: 40,
                width: 1,
                style: LineStyle::Solid,
                color: c,
            },
        ),
        (
            "HLine width 2 (even)",
            Prim::HLine {
                y: 50,
                x0: 10,
                x1: 40,
                width: 2,
                style: LineStyle::Solid,
                color: c,
            },
        ),
        (
            "HLine width 3 (odd)",
            Prim::HLine {
                y: 51,
                x0: -7,
                x1: 40,
                width: 3,
                style: LineStyle::Solid,
                color: c,
            },
        ),
        (
            "HLine dotted",
            Prim::HLine {
                y: 7,
                x0: 0,
                x1: 97,
                width: 1,
                style: LineStyle::Dotted,
                color: c,
            },
        ),
        (
            "HLine dashed",
            Prim::HLine {
                y: 7,
                x0: 0,
                x1: 97,
                width: 2,
                style: LineStyle::Dashed,
                color: c,
            },
        ),
        (
            "VLine width 1",
            Prim::VLine {
                x: 5,
                y0: 0,
                y1: 24,
                width: 1,
                style: LineStyle::Solid,
                color: c,
            },
        ),
        (
            "VLine width 4 (even)",
            Prim::VLine {
                x: 5,
                y0: 0,
                y1: 24,
                width: 4,
                style: LineStyle::Solid,
                color: c,
            },
        ),
        (
            "VLine dashed",
            Prim::VLine {
                x: 5,
                y0: 0,
                y1: 97,
                width: 1,
                style: LineStyle::Dashed,
                color: c,
            },
        ),
        (
            "VLine dotted odd width",
            Prim::VLine {
                x: 15,
                y0: 3,
                y1: 61,
                width: 3,
                style: LineStyle::Dotted,
                color: c,
            },
        ),
        (
            "Background gradient",
            Prim::Background {
                rect: [20.0, 10.0, 160.0, 60.0],
                gradient: Gradient {
                    top: Color::rgb(1, 2, 3),
                    bottom: Color::rgb(4, 5, 6),
                },
            },
        ),
    ]
}

#[test]
fn crisp_rect_subset_is_draw_call_identical_to_canvas2d() {
    for (name, prim) in rect_family_cases() {
        let prims = [prim];
        let canvas = canvas_rects(&prims, &[]);
        let (plan, _) = gpui_plan(&prims, &[]);
        assert_eq!(
            gpui_quads(&plan),
            canvas.rects,
            "{name}: the GPUI quad stream must match Canvas2D's fill_rect stream exactly"
        );
        assert_eq!(
            canvas.path_fills + canvas.path_strokes,
            0,
            "{name} should not have taken a path route on Canvas2D"
        );
    }
}

#[test]
fn crisp_rect_subset_matches_across_the_whole_dpr_matrix() {
    // The engine bakes the DPR into the prim coordinates, so parity has to hold at every DPR the
    // validation matrix names — including the fractional ones.
    for dpr in [1.0f64, 1.25, 1.5, 2.0, 2.5] {
        let scaled: Vec<Prim> = rect_family_cases()
            .into_iter()
            .map(|(_, prim)| scale_prim(prim, dpr))
            .collect();
        let canvas = canvas_rects(&scaled, &[]);
        let (plan, _) = gpui_plan(&scaled, &[]);
        assert_eq!(
            gpui_quads(&plan),
            canvas.rects,
            "DPR {dpr}: the crisp-rect draw streams diverged"
        );
    }
}

/// Apply a DPR the way the engine would, i.e. by scaling the already-integer device coordinates.
fn scale_prim(prim: Prim, dpr: f64) -> Prim {
    let s = |v: i32| (v as f64 * dpr).round() as i32;
    let sf = |v: f32| (v as f64 * dpr) as f32;
    match prim {
        Prim::Rect { rect, color } => Prim::Rect {
            rect: IRect {
                x: s(rect.x),
                y: s(rect.y),
                w: s(rect.w),
                h: s(rect.h),
            },
            color,
        },
        Prim::RectFrame {
            rect,
            border,
            color,
        } => Prim::RectFrame {
            rect: IRect {
                x: s(rect.x),
                y: s(rect.y),
                w: s(rect.w),
                h: s(rect.h),
            },
            border: s(border).max(1),
            color,
        },
        Prim::HLine {
            y,
            x0,
            x1,
            width,
            style,
            color,
        } => Prim::HLine {
            y: s(y),
            x0: s(x0),
            x1: s(x1),
            width: s(width).max(1),
            style,
            color,
        },
        Prim::VLine {
            x,
            y0,
            y1,
            width,
            style,
            color,
        } => Prim::VLine {
            x: s(x),
            y0: s(y0),
            y1: s(y1),
            width: s(width).max(1),
            style,
            color,
        },
        Prim::Background { rect, gradient } => Prim::Background {
            rect: [sf(rect[0]), sf(rect[1]), sf(rect[2]), sf(rect[3])],
            gradient,
        },
        other => other,
    }
}

#[test]
fn tessellated_prims_take_the_path_route_on_both_backends() {
    let points = vec![
        [0.0f32, 10.0],
        [10.0, 4.0],
        [20.0, 12.0],
        [0.0, 30.0],
        [10.0, 28.0],
        [20.0, 33.0],
    ];
    let c = Color::rgba(0x21, 0x96, 0xf3, 0xa0);
    let cases: Vec<(&str, Prim)> = vec![
        (
            "Polyline",
            Prim::Polyline {
                first_point: 0,
                point_count: 3,
                width: 2.0,
                style: LineStyle::Solid,
                line_type: LineType::Simple,
                color: c,
            },
        ),
        (
            "Segments",
            Prim::Segments {
                first_point: 0,
                segment_count: 2,
                width: 2.0,
                color: c,
            },
        ),
        (
            "Dashed Polyline",
            Prim::Polyline {
                first_point: 0,
                point_count: 3,
                width: 2.0,
                style: LineStyle::Dashed,
                line_type: LineType::Simple,
                color: c,
            },
        ),
        (
            "Dotted stepped Polyline",
            Prim::Polyline {
                first_point: 0,
                point_count: 3,
                width: 2.0,
                style: LineStyle::Dotted,
                line_type: LineType::WithSteps,
                color: c,
            },
        ),
        (
            "AreaFill",
            Prim::AreaFill {
                first_point: 0,
                point_count: 3,
                base_y: 40.0,
                line_type: LineType::Simple,
                gradient: Gradient {
                    top: c,
                    bottom: Color::rgba(0x21, 0x96, 0xf3, 0),
                },
            },
        ),
        (
            "BandFill",
            Prim::BandFill {
                line_type: LineType::Curved,
                upper_first: 0,
                lower_first: 3,
                point_count: 3,
                fill: c,
            },
        ),
        (
            "Circle",
            Prim::Circle {
                cx: 10.0,
                cy: 10.0,
                radius: 4.0,
                fill: c,
                stroke_width: 0.0,
                stroke: c,
            },
        ),
        (
            "Stroked Circle",
            Prim::Circle {
                cx: 10.0,
                cy: 10.0,
                radius: 4.0,
                fill: c,
                stroke_width: 2.0,
                stroke: Color::rgb(220, 30, 20),
            },
        ),
        (
            "Triangle",
            Prim::Triangle {
                a: [0.0, 0.0],
                b: [8.0, 0.0],
                c: [8.0, 8.0],
                color: c,
            },
        ),
        (
            "RoundRect",
            Prim::RoundRect {
                x: 1.0,
                y: 1.0,
                w: 20.0,
                h: 10.0,
                radii: [2.0; 4],
                fill: c,
                border_width: 0.0,
                border_color: c,
            },
        ),
        (
            "Bordered RoundRect",
            Prim::RoundRect {
                x: 1.0,
                y: 1.0,
                w: 20.0,
                h: 10.0,
                radii: [2.0; 4],
                fill: c,
                border_width: 2.0,
                border_color: Color::rgb(220, 30, 20),
            },
        ),
    ];
    for (name, prim) in cases {
        let prims = [prim];
        let canvas = canvas_rects(&prims, &points);
        let (plan, metrics) = gpui_plan(&prims, &points);
        assert!(
            canvas.path_fills + canvas.path_strokes > 0,
            "{name}: Canvas2D should have taken a path route"
        );
        assert!(
            metrics.paths > 0,
            "{name}: GPUI should have emitted at least one triangle mesh"
        );
        assert!(
            canvas.rects.is_empty() && metrics.quads == 0,
            "{name}: neither backend should emit an axis-aligned quad"
        );
        // Every mesh must be a whole number of triangles, or GPUI's `push_triangle` loop would
        // silently drop vertices.
        for op in &plan.ops {
            if let SceneOp::Mesh { vertex_count, .. } = op {
                assert_eq!(vertex_count % 3, 0, "{name}: partial triangle in the mesh");
            }
        }
    }
}

#[test]
fn gpui_meshes_contain_the_webgpu_contract_vertices_for_each_shape() {
    let points = [
        [2.0, 8.0],
        [18.0, 4.0],
        [34.0, 12.0],
        [2.0, 22.0],
        [18.0, 20.0],
        [34.0, 27.0],
    ];
    let c = Color::rgb(30, 90, 150);
    let cases = [
        (
            "Polyline",
            Prim::Polyline {
                first_point: 0,
                point_count: 3,
                width: 2.0,
                style: LineStyle::Solid,
                line_type: LineType::WithSteps,
                color: c,
            },
        ),
        (
            "AreaFill",
            Prim::AreaFill {
                first_point: 0,
                point_count: 3,
                base_y: 35.0,
                line_type: LineType::Simple,
                gradient: Gradient { top: c, bottom: c },
            },
        ),
        (
            "BandFill",
            Prim::BandFill {
                upper_first: 0,
                lower_first: 3,
                point_count: 3,
                line_type: LineType::Simple,
                fill: c,
            },
        ),
        (
            "RoundRect",
            Prim::RoundRect {
                x: 4.0,
                y: 5.0,
                w: 30.0,
                h: 20.0,
                radii: [4.0; 4],
                fill: c,
                border_width: 2.0,
                border_color: Color::rgb(80, 20, 20),
            },
        ),
        (
            "Circle",
            Prim::Circle {
                cx: 20.0,
                cy: 20.0,
                radius: 8.0,
                fill: c,
                stroke_width: 2.0,
                stroke: Color::rgb(80, 20, 20),
            },
        ),
        (
            "Triangle",
            Prim::Triangle {
                a: [3.0, 4.0],
                b: [20.0, 6.0],
                c: [12.0, 25.0],
                color: c,
            },
        ),
    ];
    for (name, prim) in cases {
        let (plan, _) = gpui_plan(std::slice::from_ref(&prim), &points);
        let mut gpu = Vec::new();
        aeris_charts_render_wgpu::geom_prim_to_tris(&prim, &points, &mut gpu);
        assert!(!gpu.is_empty(), "{name}: WebGPU emitted no vertices");
        if name == "Circle" {
            // GPUI's explicit one-pixel coverage fringe straddles the WebGPU/MSAA nominal
            // radius, so those AA vertices intentionally differ by half a pixel.
            let has_x = |x: f32| {
                plan.vertices
                    .iter()
                    .any(|v| (v.x - x).abs() <= 1e-3 && (v.y - 20.0).abs() <= 1e-3)
            };
            assert!(gpu.iter().any(|v| (v.pos[0] - 28.0).abs() <= 1e-3));
            assert!(
                has_x(27.5) && has_x(28.5),
                "GPUI coverage must straddle the 8 px nominal radius"
            );
            continue;
        }
        let mut available: Vec<_> = plan
            .vertices
            .iter()
            .map(|vertex| [vertex.x, vertex.y])
            .collect();
        for expected in &gpu {
            let Some(index) = available.iter().position(|actual| {
                (actual[0] - expected.pos[0]).abs() <= 1e-3
                    && (actual[1] - expected.pos[1]).abs() <= 1e-3
            }) else {
                panic!(
                    "{name}: WebGPU vertex {:?} has no GPUI counterpart",
                    expected.pos
                );
            };
            available.swap_remove(index);
        }
    }
}

#[test]
fn curved_brush_fixture_lowers_sparse_dense_and_scaled_widths_without_drops() {
    for dpr in [1.0f32, 1.25, 1.5, 2.0, 2.5] {
        let fixture = fixtures::curved_brushes(dpr);
        let stroke_count = fixture
            .prims
            .iter()
            .filter(|prim| {
                matches!(
                    prim,
                    Prim::Polyline {
                        line_type: LineType::Curved,
                        ..
                    }
                )
            })
            .count();
        let (plan, metrics) = gpui_plan(&fixture.prims, &fixture.points);
        let meshes: Vec<_> = plan
            .ops
            .iter()
            .filter_map(|op| match op {
                SceneOp::Mesh { vertex_count, .. } => Some(*vertex_count),
                _ => None,
            })
            .collect();

        assert_eq!(stroke_count, 4, "DPR {dpr}: fixture lost curved strokes");
        assert_eq!(metrics.dropped_prims, 0, "DPR {dpr}: a stroke was dropped");
        assert_eq!(meshes.len(), stroke_count, "DPR {dpr}: wrong mesh count");
        assert!(
            meshes.iter().all(|count| *count >= 3 && *count % 3 == 0),
            "DPR {dpr}: every curved brush must produce complete triangles"
        );
    }
}

#[test]
fn tessellated_fixture_segments_reach_both_backends_without_drops() {
    for dpr in [1.0f32, 1.25, 1.5, 2.0, 2.5] {
        let fixture = fixtures::tessellated(dpr);
        let (first_point, segment_count) = fixture
            .prims
            .iter()
            .find_map(|prim| match prim {
                Prim::Segments {
                    first_point,
                    segment_count,
                    ..
                } => Some((*first_point, *segment_count)),
                _ => None,
            })
            .expect("the tessellated fixture carries a segment batch");
        assert_eq!(segment_count, 10, "DPR {dpr}");
        assert!(
            first_point as usize + 2 * segment_count as usize <= fixture.points.len(),
            "DPR {dpr}: the batch stays inside the pool"
        );
        let (_, metrics) = gpui_plan(&fixture.prims, &fixture.points);
        assert_eq!(metrics.dropped_prims, 0, "DPR {dpr}: a prim was dropped");
        // One Canvas2D stroke for the whole batch beside the fixture's other strokes.
        let canvas = canvas_rects(&fixture.prims, &fixture.points);
        let polylines = fixture
            .prims
            .iter()
            .filter(|prim| matches!(prim, Prim::Polyline { .. }))
            .count();
        assert!(
            canvas.path_strokes > polylines,
            "DPR {dpr}: the segment batch takes the Canvas2D stroke route"
        );
    }
}

#[test]
fn area_fill_gradient_extent_matches_the_canvas2d_ramp() {
    // Canvas2D spans the ramp over [min point y, base_y]; GPUI's ramp is bounds-relative, so the
    // mesh bounds must equal that interval or the two shade differently.
    let points = vec![[0.0f32, 10.0], [10.0, 4.0], [20.0, 12.0]];
    let prims = [Prim::AreaFill {
        first_point: 0,
        point_count: 3,
        base_y: 40.0,
        line_type: LineType::Simple,
        gradient: Gradient {
            top: Color::rgb(0, 0, 0xff),
            bottom: Color::rgba(0, 0, 0xff, 0),
        },
    }];
    let (plan, _) = gpui_plan(&prims, &points);
    let SceneOp::Mesh {
        first_vertex,
        vertex_count,
        fill,
    } = plan.ops[0]
    else {
        panic!("expected one mesh, got {:?}", plan.ops);
    };
    let bounds = plan
        .mesh_bounds(first_vertex, vertex_count)
        .expect("the mesh has vertices");
    assert_eq!(
        (bounds.y, bounds.y + bounds.h),
        (4.0, 40.0),
        "the mesh must span exactly the Canvas2D gradient extent"
    );
    assert_eq!(
        fill,
        Paint::VGradient {
            top: Color::rgb(0, 0, 0xff),
            bottom: Color::rgba(0, 0, 0xff, 0),
        },
        "a full-height fill keeps the prim's own stops"
    );
}

#[test]
fn image_and_background_fixture_preserve_rects_pixels_and_gradient_extent() {
    let image = fixtures::colored_image(1.0);
    let canvas = canvas_rects(&image.prims, &image.points);
    let (plan, _) = gpui_plan(&image.prims, &image.points);
    assert_eq!(canvas.images.len(), 1);
    let image_ops: Vec<_> = plan
        .ops
        .iter()
        .filter_map(|op| match op {
            SceneOp::Image {
                image,
                rect,
                opacity,
            } => Some((image, rect, opacity)),
            _ => None,
        })
        .collect();
    assert_eq!(image_ops.len(), 1);
    let (gpui_image, rect, opacity) = image_ops[0];
    let (canvas_rect, canvas_opacity, width, height) = canvas.images[0];
    assert_eq!([rect.x, rect.y, rect.w, rect.h], canvas_rect);
    assert_eq!(*opacity, canvas_opacity);
    assert_eq!((gpui_image.width, gpui_image.height), (width, height));
    let Prim::Image { image: source, .. } = &image.prims[0] else {
        panic!("colored image fixture starts with the image");
    };
    assert_eq!(gpui_image.pixels, source.pixels);

    let gradients = fixtures::gradients(1.0);
    let background = &gradients.prims[..1];
    let canvas = canvas_rects(background, &[]);
    let (plan, _) = gpui_plan(background, &[]);
    let SceneOp::Quad { rect, fill, .. } = plan.ops[0] else {
        panic!("background must become a gradient quad");
    };
    let (top_y, bottom_y, top, bottom) = canvas.gradient_extents[0];
    assert_eq!((rect.y, rect.y + rect.h), (top_y, bottom_y));
    assert_eq!(fill, Paint::VGradient { top, bottom });
}

#[test]
fn native_golden_scene_reaches_canvas_and_gpui_with_no_dropped_primitives() {
    let scene = aeris_charts_native::scene::demo_scene();
    let canvas = canvas_rects(&scene.prims, &scene.points);
    let (plan, metrics) = gpui_plan(&scene.prims, &scene.points);
    assert_eq!(metrics.dropped_prims, 0);
    assert!(!canvas.rects.is_empty() && !canvas.gradient_extents.is_empty());
    assert!(canvas.path_fills > 0 && canvas.path_strokes > 0);
    assert_eq!(canvas.images.len(), 1);
    assert_eq!(canvas.text_runs.len(), 1);
    assert!(plan
        .ops
        .iter()
        .any(|op| matches!(op, SceneOp::Image { .. })));
    assert!(plan.ops.iter().any(|op| matches!(op, SceneOp::Text(_))));
    assert!(plan.ops.iter().any(|op| matches!(op, SceneOp::Mesh { .. })));
}

#[test]
fn text_runs_reach_both_backends_with_the_same_font_and_anchor() {
    let prims = [Prim::Text {
        x: 100.5,
        y: 30.0,
        text: "42.50".into(),
        color: Color::rgb(0x13, 0x17, 0x22),
        size: 12.0,
        family: "sans-serif".into(),
        align: TextAlign::Right,
        weight: 400,
        italic: false,
    }];
    let canvas = canvas_rects(&prims, &[]);
    let (plan, metrics) = gpui_plan(&prims, &[]);
    assert_eq!(canvas.text_runs.len(), 1);
    assert_eq!(metrics.text_runs, 1);

    let (text, x, y, font, color, align) = &canvas.text_runs[0];
    let SceneOp::Text(run) = &plan.ops[0] else {
        panic!("expected a text op");
    };
    assert_eq!(run.text, *text);
    assert_eq!(run.x, *x);
    assert_eq!(run.y, *y);
    assert_eq!(run.color, *color);
    assert_eq!(run.align, *align);
    // The GPUI adapter derives the same CSS shorthand Canvas2D is given.
    assert_eq!(
        aeris_charts_render::draw_list::text_font_spec(
            run.size,
            &run.family,
            run.weight,
            run.italic
        ),
        *font
    );
}

#[test]
fn rotated_text_reaches_canvas_and_gpui_with_the_same_transform() {
    let prims = [Prim::RotatedText {
        x: 100.5,
        y: 30.0,
        text: "trend".into(),
        color: Color::rgb(0x13, 0x17, 0x22),
        size: 12.0,
        family: "sans-serif".into(),
        align: TextAlign::Center,
        weight: 500,
        italic: true,
        angle: -0.625,
    }];
    let canvas = canvas_rects(&prims, &[]);
    let (plan, metrics) = gpui_plan(&prims, &[]);
    assert_eq!(canvas.text_runs.len(), 1);
    assert_eq!(metrics.text_runs, 1);
    assert_eq!(canvas.text_runs[0].0, "trend@-0.625");
    let SceneOp::Text(run) = &plan.ops[0] else {
        panic!("expected a text op");
    };
    assert_eq!((run.x, run.y, run.angle), (100.5, 30.0, -0.625));
    assert_eq!(run.color, canvas.text_runs[0].4);
    assert_eq!(run.align, canvas.text_runs[0].5);
}

/// A real multi-series, multi-pane engine frame — not a synthetic prim list.
fn real_engine_frame(dpr: f64) -> ChartEngine {
    let mut engine = ChartEngine::new(900.0, 520.0, dpr);
    let n = 180usize;
    let times: Vec<f64> = (0..n).map(|i| 1_600_000_000.0 + i as f64 * 60.0).collect();
    let close: Vec<f64> = (0..n)
        .map(|i| 100.0 + (i as f64 * 0.13).sin() * 8.0 + (i as f64 * 0.02).cos() * 5.0)
        .collect();
    let open: Vec<f64> = close
        .iter()
        .enumerate()
        .map(|(i, c)| if i == 0 { *c } else { close[i - 1] })
        .collect();
    let high: Vec<f64> = open
        .iter()
        .zip(&close)
        .map(|(o, c)| o.max(*c) + 2.0)
        .collect();
    let low: Vec<f64> = open
        .iter()
        .zip(&close)
        .map(|(o, c)| o.min(*c) - 2.0)
        .collect();

    engine
        .set_series_data(0, &times, &open, &high, &low, &close)
        .expect("candles load");
    engine.series[0].kind = SeriesKind::Candlestick;

    let rsi = engine
        .add_indicator_kind_with_input(
            0,
            IndicatorInputSource::Hlc3,
            IndicatorKind::Rsi {
                period: 3,
                seed: aeris_charts_engine::IndicatorSeed::Sma,
            },
            None,
        )
        .into_iter()
        .next()
        .expect("RSI output");
    let sma = engine.add_sma(rsi, 2).expect("SMA output");
    let bands = engine.add_bollinger(sma, 2, 2.0);
    assert!(engine.set_indicator_output_style(
        bands[0],
        IndicatorOutputStyle {
            area_top_color: Some("rgba(20, 120, 220, 0.24)".into()),
            area_bottom_color: Some("rgba(20, 120, 220, 0.04)".into()),
            ..IndicatorOutputStyle::default()
        }
    ));

    // A line, an area and a histogram, so the tessellated and gradient routes are all populated.
    for kind in [SeriesKind::Line, SeriesKind::Area, SeriesKind::Histogram] {
        let id = engine.add_series(kind);
        engine
            .set_series_data(id, &times, &close, &close, &close, &close)
            .expect("line-ish series loads");
    }

    engine.css_width = 900.0;
    engine.css_height = 520.0;
    engine.dpr = dpr;
    let content_h = (520.0 - engine.time_axis_height()).max(1.0);
    engine.layout_panes(content_h);
    engine.time_scale.set_width(900.0);
    engine.fit_content();
    engine.crosshair = Some((450.0, 260.0));
    let position_id = PositionId::new("gpui-position").unwrap();
    engine
        .update_trading_position(TradingPosition {
            id: position_id.clone(),
            account_id: None,
            pane_index: 0,
            price_scale: TradingPriceScale::Right,
            side: PositionSide::Long,
            average_price: 105.0,
            quantity: 2.0,
            display_pnl: Some(14.0),
            currency: Some("USD".into()),
            annotations: Vec::new(),
        })
        .unwrap();
    for (id, role, kind, price) in [
        ("gpui-tp", OrderRole::TakeProfit, OrderKind::Limit, 112.0),
        ("gpui-sl", OrderRole::StopLoss, OrderKind::Stop, 96.0),
    ] {
        engine
            .update_working_order(WorkingOrder {
                id: OrderId::new(id).unwrap(),
                account_id: None,
                pane_index: 0,
                price_scale: TradingPriceScale::Right,
                side: OrderSide::Sell,
                kind,
                role,
                status: OrderStatus::Working,
                price,
                stop_price: None,
                trailing_trigger_price: None,
                break_even_trigger_price: None,
                quantity: 2.0,
                filled_quantity: 0.0,
                position_id: Some(position_id.clone()),
                parent_order_id: None,
                bracket_id: None,
                oco_group_id: None,
                revision: 1,
                annotations: Vec::new(),
            })
            .unwrap();
    }
    engine
}

#[test]
fn a_real_engine_frame_has_draw_call_identical_quads_on_both_backends() {
    for dpr in [1.0f64, 1.25, 1.5, 2.0, 2.5] {
        let mut engine = real_engine_frame(dpr);
        let frame = engine.build_frame();
        assert!(!frame.panes.is_empty(), "the frame should have panes");

        let mut total = 0usize;
        for pane in &frame.panes {
            for layer in [&pane.under, &pane.main, &pane.top_prims] {
                if layer.is_empty() {
                    continue;
                }
                let canvas = canvas_rects(layer, &pane.points);
                let (plan, _) = gpui_plan(layer, &pane.points);
                assert_eq!(
                    gpui_quads(&plan),
                    canvas.rects,
                    "DPR {dpr}: a real frame's quad stream diverged from Canvas2D"
                );
                total += canvas.rects.len();
            }
        }
        assert!(
            total > 50,
            "DPR {dpr}: expected a substantial quad stream, got {total}"
        );
    }
}

#[test]
fn category_column_engine_frame_has_identical_canvas_and_gpui_quads() {
    for dpr in [1.0f64, 1.5, 2.0] {
        let mut engine = ChartEngine::new(420.0, 260.0, dpr);
        let pane = engine
            .add_pane_with_domain(
                true,
                HorizontalDomain::Category {
                    scale: CategoryScaleType::Band,
                },
            )
            .unwrap();
        engine
            .add_general_axis(GeneralAxisOptions::new(
                "x",
                pane,
                AxisDimension::X,
                GeneralScaleType::Band,
            ))
            .unwrap();
        engine
            .add_general_axis(GeneralAxisOptions::new(
                "y",
                pane,
                AxisDimension::Y,
                GeneralScaleType::Linear,
            ))
            .unwrap();
        let dataset = engine
            .create_general_xy_dataset(GeneralXyInput::Category {
                ids: None,
                categories: vec!["A".into(), "B".into(), "C".into()],
                category_indices: vec![0, 1, 2],
                y: vec![-4.0, 8.0, 99.0],
                y_valid: Some(vec![1, 1, 0]),
            })
            .unwrap();
        let mut options = GeneralSeriesOptions::column(pane, dataset, "x", "y");
        options.color = Some("#4f6b8a".into());
        engine.add_general_series(options).unwrap();
        engine.recompute_layout_with_measure(true, |text, _| text.len() as f64 * 7.0, |_, _| 0.0);

        let frame = engine.build_frame();
        let pane_frame = &frame.panes[pane];
        let canvas = canvas_rects(&pane_frame.main, &pane_frame.points);
        let (plan, metrics) = gpui_plan(&pane_frame.main, &pane_frame.points);
        assert_eq!(canvas.rects.len(), 2, "DPR {dpr}: two columns expected");
        assert_eq!(metrics.dropped_prims, 0, "DPR {dpr}: no column may drop");
        assert_eq!(
            gpui_quads(&plan),
            canvas.rects,
            "DPR {dpr}: category-column quads diverged"
        );
    }
}

#[test]
fn horizontal_bar_engine_frame_has_identical_canvas_and_gpui_quads() {
    for dpr in [1.0f64, 1.5, 2.0] {
        let mut engine = ChartEngine::new(420.0, 260.0, dpr);
        let pane = engine
            .add_pane_with_domain(
                true,
                HorizontalDomain::Continuous {
                    scale: ContinuousScaleType::Linear,
                },
            )
            .unwrap();
        engine
            .add_general_axis(GeneralAxisOptions::new(
                "x",
                pane,
                AxisDimension::X,
                GeneralScaleType::Linear,
            ))
            .unwrap();
        engine
            .add_general_axis(GeneralAxisOptions::new(
                "y",
                pane,
                AxisDimension::Y,
                GeneralScaleType::Band,
            ))
            .unwrap();
        let dataset = engine
            .create_general_xy_dataset(GeneralXyInput::Category {
                ids: None,
                categories: vec!["A".into(), "B".into(), "C".into()],
                category_indices: vec![0, 1, 2],
                y: vec![-4.0, 8.0, 99.0],
                y_valid: Some(vec![1, 1, 0]),
            })
            .unwrap();
        let mut options = GeneralSeriesOptions::horizontal_bar(pane, dataset, "x", "y");
        options.color = Some("#4f6b8a".into());
        engine.add_general_series(options).unwrap();
        engine.recompute_layout_with_measure(true, |text, _| text.len() as f64 * 7.0, |_, _| 0.0);

        let frame = engine.build_frame();
        let pane_frame = &frame.panes[pane];
        let canvas = canvas_rects(&pane_frame.main, &pane_frame.points);
        let (plan, metrics) = gpui_plan(&pane_frame.main, &pane_frame.points);
        assert_eq!(
            canvas.rects.len(),
            2,
            "DPR {dpr}: two horizontal bars expected"
        );
        assert_eq!(
            metrics.dropped_prims, 0,
            "DPR {dpr}: no horizontal bar may drop"
        );
        assert_eq!(
            gpui_quads(&plan),
            canvas.rects,
            "DPR {dpr}: horizontal-bar quads diverged"
        );
    }
}

#[test]
fn xy_scatter_engine_frame_reaches_canvas_and_gpui_path_routes() {
    for dpr in [1.0f64, 1.5, 2.0] {
        let mut engine = ChartEngine::new(420.0, 260.0, dpr);
        let pane = engine
            .add_pane_with_domain(
                true,
                HorizontalDomain::Continuous {
                    scale: ContinuousScaleType::Linear,
                },
            )
            .unwrap();
        engine
            .add_general_axis(GeneralAxisOptions::new(
                "x",
                pane,
                AxisDimension::X,
                GeneralScaleType::Linear,
            ))
            .unwrap();
        engine
            .add_general_axis(GeneralAxisOptions::new(
                "y",
                pane,
                AxisDimension::Y,
                GeneralScaleType::Linear,
            ))
            .unwrap();
        let dataset = engine
            .create_general_xy_dataset(GeneralXyInput::Numeric {
                ids: None,
                x: vec![1.0, 2.0, 3.0, 4.0],
                y: vec![-2.0, 1.0, 5.0, 99.0],
                y_valid: Some(vec![1, 1, 1, 0]),
            })
            .unwrap();
        let mut options = GeneralSeriesOptions::scatter(pane, dataset, "x", "y");
        options.color = Some("#725c9f".into());
        engine.add_general_series(options).unwrap();
        engine.recompute_layout_with_measure(true, |text, _| text.len() as f64 * 7.0, |_, _| 0.0);

        let frame = engine.build_frame();
        let pane_frame = &frame.panes[pane];
        let circles = pane_frame
            .main
            .iter()
            .filter(|primitive| matches!(primitive, Prim::Circle { .. }))
            .count();
        assert_eq!(circles, 3, "DPR {dpr}: three scatter circles expected");
        let canvas = canvas_rects(&pane_frame.main, &pane_frame.points);
        let (_plan, metrics) = gpui_plan(&pane_frame.main, &pane_frame.points);
        assert_eq!(
            canvas.path_fills, 3,
            "DPR {dpr}: Canvas must fill each point"
        );
        assert_eq!(
            metrics.dropped_prims, 0,
            "DPR {dpr}: no scatter point may drop"
        );
        assert!(
            metrics.paths >= 3,
            "DPR {dpr}: GPUI must lower scatter circles to paths ({metrics:?})"
        );
    }
}

#[test]
fn error_bar_engine_frame_reaches_canvas_and_gpui_stroke_and_point_routes() {
    for dpr in [1.0f64, 1.5, 2.0] {
        let mut engine = ChartEngine::new(420.0, 260.0, dpr);
        let pane = engine
            .add_pane_with_domain(
                true,
                HorizontalDomain::Continuous {
                    scale: ContinuousScaleType::Linear,
                },
            )
            .unwrap();
        engine
            .add_general_axis(GeneralAxisOptions::new(
                "x",
                pane,
                AxisDimension::X,
                GeneralScaleType::Linear,
            ))
            .unwrap();
        engine
            .add_general_axis(GeneralAxisOptions::new(
                "y",
                pane,
                AxisDimension::Y,
                GeneralScaleType::Linear,
            ))
            .unwrap();
        let dataset = engine
            .create_general_xy_dataset(GeneralXyInput::ErrorNumeric {
                ids: None,
                x: vec![1.0, 2.0, 3.0],
                y: vec![2.0, 3.0, 100.0],
                y_valid: Some(vec![1, 1, 0]),
                x_low: vec![0.5, 0.0, 0.0],
                x_low_valid: Some(vec![1, 0, 0]),
                x_high: vec![1.5, 2.5, 0.0],
                x_high_valid: Some(vec![1, 1, 0]),
                y_low: vec![1.5, 2.5, 0.0],
                y_low_valid: Some(vec![1, 1, 0]),
                y_high: vec![2.5, 0.0, 0.0],
                y_high_valid: Some(vec![1, 0, 0]),
            })
            .unwrap();
        engine
            .add_general_series(GeneralSeriesOptions::error_bar(pane, dataset, "x", "y"))
            .unwrap();
        engine.recompute_layout_with_measure(true, |text, _| text.len() as f64 * 7.0, |_, _| 0.0);
        let frame = engine.build_frame();
        let pane_frame = &frame.panes[pane];
        assert_eq!(
            pane_frame
                .main
                .iter()
                .filter(|primitive| matches!(primitive, Prim::Circle { .. }))
                .count(),
            2,
            "DPR {dpr}: missing observation must not emit a mark"
        );
        assert!(pane_frame
            .main
            .iter()
            .any(|primitive| matches!(primitive, Prim::HLine { .. })));
        assert!(pane_frame
            .main
            .iter()
            .any(|primitive| matches!(primitive, Prim::VLine { .. })));
        let canvas = canvas_rects(&pane_frame.main, &pane_frame.points);
        let (_plan, metrics) = gpui_plan(&pane_frame.main, &pane_frame.points);
        assert_eq!(
            canvas.path_fills, 2,
            "DPR {dpr}: two center points expected"
        );
        assert!(
            !canvas.rects.is_empty(),
            "DPR {dpr}: error stems and caps must be drawn"
        );
        assert_eq!(
            metrics.dropped_prims, 0,
            "DPR {dpr}: GPUI must retain every primitive"
        );
        assert!(
            metrics.paths >= 2,
            "DPR {dpr}: GPUI center points must lower to paths"
        );
    }
}

#[test]
fn category_error_bar_frame_reaches_canvas_and_gpui_on_band_and_point_axes() {
    for (category_scale, axis_scale) in [
        (CategoryScaleType::Band, GeneralScaleType::Band),
        (CategoryScaleType::Point, GeneralScaleType::Point),
    ] {
        for dpr in [1.0f64, 1.5, 2.0] {
            let mut engine = ChartEngine::new(420.0, 260.0, dpr);
            let pane = engine
                .add_pane_with_domain(
                    true,
                    HorizontalDomain::Category {
                        scale: category_scale,
                    },
                )
                .unwrap();
            engine
                .add_general_axis(GeneralAxisOptions::new(
                    "x",
                    pane,
                    AxisDimension::X,
                    axis_scale,
                ))
                .unwrap();
            engine
                .add_general_axis(GeneralAxisOptions::new(
                    "y",
                    pane,
                    AxisDimension::Y,
                    GeneralScaleType::Linear,
                ))
                .unwrap();
            let dataset = engine
                .create_general_xy_dataset(GeneralXyInput::ErrorCategory {
                    ids: None,
                    categories: vec!["A".into(), "B".into()],
                    category_indices: vec![0, 1],
                    y: vec![2.0, 3.0],
                    y_valid: None,
                    y_low: vec![1.5, 2.5],
                    y_low_valid: None,
                    y_high: vec![2.5, 0.0],
                    y_high_valid: Some(vec![1, 0]),
                })
                .unwrap();
            engine
                .add_general_series(GeneralSeriesOptions::error_bar(pane, dataset, "x", "y"))
                .unwrap();
            engine.recompute_layout_with_measure(
                true,
                |text, _| text.len() as f64 * 7.0,
                |_, _| 0.0,
            );
            let frame = engine.build_frame();
            let pane_frame = &frame.panes[pane];
            let canvas = canvas_rects(&pane_frame.main, &pane_frame.points);
            let (_plan, metrics) = gpui_plan(&pane_frame.main, &pane_frame.points);
            assert_eq!(
                canvas.path_fills, 2,
                "DPR {dpr}: two category centers expected"
            );
            assert!(
                !canvas.rects.is_empty(),
                "DPR {dpr}: Y stems and caps expected"
            );
            assert_eq!(
                metrics.dropped_prims, 0,
                "DPR {dpr}: GPUI must retain all marks"
            );
            assert_eq!(
                metrics.quads as usize,
                canvas.rects.len(),
                "DPR {dpr}: stems/caps must match Canvas2D"
            );
            assert!(
                metrics.paths >= 2,
                "DPR {dpr}: GPUI must lower center circles"
            );
        }
    }
}

#[test]
fn category_box_plot_frame_reaches_canvas_and_gpui_without_dropped_primitives() {
    for dpr in [1.0f64, 1.5, 2.0] {
        let mut engine = ChartEngine::new(420.0, 260.0, dpr);
        let pane = engine
            .add_pane_with_domain(
                true,
                HorizontalDomain::Category {
                    scale: CategoryScaleType::Band,
                },
            )
            .unwrap();
        engine
            .add_general_axis(GeneralAxisOptions::new(
                "x",
                pane,
                AxisDimension::X,
                GeneralScaleType::Band,
            ))
            .unwrap();
        engine
            .add_general_axis(GeneralAxisOptions::new(
                "y",
                pane,
                AxisDimension::Y,
                GeneralScaleType::Linear,
            ))
            .unwrap();
        let dataset = engine
            .create_general_xy_dataset(GeneralXyInput::BoxCategory {
                ids: None,
                categories: vec!["A".into(), "B".into(), "Missing".into()],
                category_indices: vec![0, 1, 2],
                min: vec![1.0, 2.0, -100.0],
                min_valid: None,
                q1: vec![2.0, 3.0, -50.0],
                q1_valid: None,
                median: vec![3.0, 4.0, 0.0],
                median_valid: Some(vec![1, 1, 0]),
                q3: vec![4.0, 5.0, 50.0],
                q3_valid: None,
                max: vec![5.0, 7.0, 100.0],
                max_valid: None,
            })
            .unwrap();
        engine
            .add_general_series(GeneralSeriesOptions::box_plot(pane, dataset, "x", "y"))
            .unwrap();
        engine.recompute_layout_with_measure(true, |text, _| text.len() as f64 * 7.0, |_, _| 0.0);
        let frame = engine.build_frame();
        let pane_frame = &frame.panes[pane];
        assert_eq!(
            pane_frame
                .main
                .iter()
                .filter(|primitive| matches!(primitive, Prim::Rect { .. }))
                .count(),
            2,
            "DPR {dpr}: only complete rows emit IQR boxes"
        );
        assert!(pane_frame
            .main
            .iter()
            .any(|primitive| matches!(primitive, Prim::HLine { .. })));
        assert!(pane_frame
            .main
            .iter()
            .any(|primitive| matches!(primitive, Prim::VLine { .. })));

        let canvas = canvas_rects(&pane_frame.main, &pane_frame.points);
        let (_plan, metrics) = gpui_plan(&pane_frame.main, &pane_frame.points);
        assert!(
            !canvas.rects.is_empty(),
            "DPR {dpr}: box fills, medians, caps, and whiskers must reach Canvas2D"
        );
        assert_eq!(
            metrics.dropped_prims, 0,
            "DPR {dpr}: GPUI must retain every box-plot primitive"
        );
        assert_eq!(
            metrics.quads as usize,
            canvas.rects.len(),
            "DPR {dpr}: Canvas2D and GPUI rectangle routes must agree"
        );
    }
}

#[test]
fn category_heatmap_grid_frame_reaches_canvas_and_gpui_as_rect_cells() {
    for dpr in [1.0f64, 1.5, 2.0] {
        let mut engine = ChartEngine::new(420.0, 260.0, dpr);
        let pane = engine
            .add_pane_with_domain(
                true,
                HorizontalDomain::Category {
                    scale: CategoryScaleType::Band,
                },
            )
            .unwrap();
        engine
            .add_general_axis(GeneralAxisOptions::new(
                "x",
                pane,
                AxisDimension::X,
                GeneralScaleType::Band,
            ))
            .unwrap();
        engine
            .add_general_axis(GeneralAxisOptions::new(
                "y",
                pane,
                AxisDimension::Y,
                GeneralScaleType::Band,
            ))
            .unwrap();
        let dataset = engine
            .create_general_xy_dataset(GeneralXyInput::HeatmapCategoryCategory {
                ids: None,
                x_categories: vec!["A".into(), "B".into()],
                x_category_indices: vec![0, 0, 1, 1],
                y_categories: vec!["North".into(), "South".into()],
                y_category_indices: vec![0, 1, 0, 1],
                value: vec![10.0, 20.0, 30.0, 0.0],
                value_valid: Some(vec![1, 1, 1, 0]),
            })
            .unwrap();
        let mut options = GeneralSeriesOptions::heatmap_grid(pane, dataset, "x", "y");
        options.color = Some("#3568a8".into());
        engine.add_general_series(options).unwrap();
        engine.recompute_layout_with_measure(true, |text, _| text.len() as f64 * 7.0, |_, _| 0.0);
        let frame = engine.build_frame();
        let pane_frame = &frame.panes[pane];
        assert_eq!(
            pane_frame
                .main
                .iter()
                .filter(|primitive| matches!(primitive, Prim::Rect { .. }))
                .count(),
            3,
            "DPR {dpr}: only valid heatmap values emit cells"
        );
        let canvas = canvas_rects(&pane_frame.main, &pane_frame.points);
        let (_plan, metrics) = gpui_plan(&pane_frame.main, &pane_frame.points);
        assert_eq!(
            metrics.dropped_prims, 0,
            "DPR {dpr}: GPUI must retain every heatmap cell"
        );
        assert_eq!(
            metrics.quads as usize,
            canvas.rects.len(),
            "DPR {dpr}: Canvas2D and GPUI heatmap rect routes must agree"
        );
    }
}

#[test]
fn numeric_and_temporal_heatmap_frames_reach_canvas_and_gpui_as_rect_cells() {
    for dpr in [1.0f64, 1.5, 2.0] {
        let mut engine = ChartEngine::new(520.0, 420.0, dpr);
        let numeric_pane = engine
            .add_pane_with_domain(
                true,
                HorizontalDomain::Continuous {
                    scale: ContinuousScaleType::Linear,
                },
            )
            .unwrap();
        engine
            .add_general_axis(GeneralAxisOptions::new(
                "numeric-x",
                numeric_pane,
                AxisDimension::X,
                GeneralScaleType::Linear,
            ))
            .unwrap();
        engine
            .add_general_axis(GeneralAxisOptions::new(
                "numeric-y",
                numeric_pane,
                AxisDimension::Y,
                GeneralScaleType::Linear,
            ))
            .unwrap();
        let numeric_data = engine
            .create_general_xy_dataset(GeneralXyInput::HeatmapNumericNumeric {
                ids: None,
                x: vec![0.0, 0.0, 10.0, 10.0],
                y_coordinate: vec![10.0, 20.0, 10.0, 20.0],
                value: vec![1.0, 2.0, 3.0, 0.0],
                value_valid: Some(vec![1, 1, 1, 0]),
            })
            .unwrap();
        engine
            .add_general_series(GeneralSeriesOptions::heatmap_grid(
                numeric_pane,
                numeric_data,
                "numeric-x",
                "numeric-y",
            ))
            .unwrap();

        let temporal_pane = engine
            .add_pane_with_domain(true, HorizontalDomain::Temporal)
            .unwrap();
        engine
            .add_general_axis(GeneralAxisOptions::new(
                "temporal-x",
                temporal_pane,
                AxisDimension::X,
                GeneralScaleType::Temporal,
            ))
            .unwrap();
        engine
            .add_general_axis(GeneralAxisOptions::new(
                "temporal-y",
                temporal_pane,
                AxisDimension::Y,
                GeneralScaleType::Linear,
            ))
            .unwrap();
        let temporal_data = engine
            .create_general_xy_dataset(GeneralXyInput::HeatmapTemporalNumeric {
                ids: None,
                x_epoch_ms: vec![
                    1_700_000_000_000,
                    1_700_000_000_000,
                    1_700_000_060_000,
                    1_700_000_060_000,
                ],
                y_coordinate: vec![5.0, 15.0, 5.0, 15.0],
                value: vec![10.0, 20.0, 30.0, 40.0],
                value_valid: None,
            })
            .unwrap();
        engine
            .add_general_series(GeneralSeriesOptions::heatmap_grid(
                temporal_pane,
                temporal_data,
                "temporal-x",
                "temporal-y",
            ))
            .unwrap();

        engine.recompute_layout_with_measure(true, |text, _| text.len() as f64 * 7.0, |_, _| 0.0);
        let frame = engine.build_frame();
        for (pane, expected_cells) in [(numeric_pane, 3usize), (temporal_pane, 4usize)] {
            let pane_frame = &frame.panes[pane];
            assert_eq!(
                pane_frame
                    .main
                    .iter()
                    .filter(|primitive| matches!(primitive, Prim::Rect { .. }))
                    .count(),
                expected_cells,
                "DPR {dpr}: valid continuous/temporal heatmap values emit one cell each"
            );
            let canvas = canvas_rects(&pane_frame.main, &pane_frame.points);
            let (_plan, metrics) = gpui_plan(&pane_frame.main, &pane_frame.points);
            assert_eq!(
                metrics.dropped_prims, 0,
                "DPR {dpr}: GPUI must retain heatmap cells"
            );
            assert_eq!(
                metrics.quads as usize,
                canvas.rects.len(),
                "DPR {dpr}: Canvas2D and GPUI continuous heatmap rect routes must agree"
            );
        }
    }
}

#[test]
fn general_reference_components_reach_canvas_and_gpui_without_dropped_primitives() {
    for dpr in [1.0f64, 1.5, 2.0] {
        let mut engine = ChartEngine::new(520.0, 320.0, dpr);
        let pane = engine
            .add_pane_with_domain(
                true,
                HorizontalDomain::Continuous {
                    scale: ContinuousScaleType::Linear,
                },
            )
            .unwrap();
        let mut x = GeneralAxisOptions::new(
            "reference-x",
            pane,
            AxisDimension::X,
            GeneralScaleType::Linear,
        );
        x.domain = GeneralAxisDomain::Numeric([0.0, 10.0]);
        engine.add_general_axis(x).unwrap();
        let mut y = GeneralAxisOptions::new(
            "reference-y",
            pane,
            AxisDimension::Y,
            GeneralScaleType::Linear,
        );
        y.domain = GeneralAxisDomain::Numeric([0.0, 10.0]);
        engine.add_general_axis(y).unwrap();
        engine
            .add_general_reference(GeneralReferenceOptions::Region {
                pane,
                x_axis_id: "reference-x".into(),
                y_axis_id: "reference-y".into(),
                x_from: GeneralReferenceValue::Numeric(2.0),
                x_to: GeneralReferenceValue::Numeric(8.0),
                y_from: GeneralReferenceValue::Numeric(2.0),
                y_to: GeneralReferenceValue::Numeric(8.0),
                fill_color: Some("rgba(10,20,30,0.25)".into()),
                extend_domain: false,
            })
            .unwrap();
        engine
            .add_general_reference(GeneralReferenceOptions::Line {
                pane,
                axis_id: "reference-x".into(),
                value: GeneralReferenceValue::Numeric(5.0),
                color: Some("#112233".into()),
                line_width: 2.0,
                extend_domain: false,
            })
            .unwrap();
        engine
            .add_general_reference(GeneralReferenceOptions::Dot {
                pane,
                x_axis_id: "reference-x".into(),
                y_axis_id: "reference-y".into(),
                x: GeneralReferenceValue::Numeric(4.0),
                y: GeneralReferenceValue::Numeric(6.0),
                color: Some("#445566".into()),
                radius: 5.0,
                extend_domain: false,
            })
            .unwrap();

        engine.recompute_layout_with_measure(true, |text, _| text.len() as f64 * 7.0, |_, _| 0.0);
        let frame = engine.build_frame();
        let pane_frame = &frame.panes[pane];
        assert_eq!(
            pane_frame
                .main
                .iter()
                .filter(|primitive| matches!(primitive, Prim::Rect { .. }))
                .count(),
            1
        );
        assert_eq!(
            pane_frame
                .main
                .iter()
                .filter(|primitive| matches!(primitive, Prim::VLine { .. }))
                .count(),
            1
        );
        assert_eq!(
            pane_frame
                .main
                .iter()
                .filter(|primitive| matches!(primitive, Prim::Circle { .. }))
                .count(),
            1
        );
        let canvas = canvas_rects(&pane_frame.main, &pane_frame.points);
        let (_plan, metrics) = gpui_plan(&pane_frame.main, &pane_frame.points);
        assert_eq!(
            metrics.dropped_prims, 0,
            "DPR {dpr}: no reference primitive may drop"
        );
        assert!(
            canvas.rects.len() >= 2,
            "DPR {dpr}: region and reference line reach Canvas2D"
        );
        assert!(
            canvas.path_fills >= 1,
            "DPR {dpr}: reference dot reaches Canvas2D"
        );
        assert!(metrics.quads >= 2, "DPR {dpr}: region and line reach GPUI");
        assert!(metrics.paths >= 1, "DPR {dpr}: reference dot reaches GPUI");
    }
}

#[test]
fn temporal_error_bar_frame_reaches_canvas_and_gpui_with_xy_stems() {
    for dpr in [1.0f64, 1.5, 2.0] {
        let mut engine = ChartEngine::new(420.0, 260.0, dpr);
        let pane = engine
            .add_pane_with_domain(true, HorizontalDomain::Temporal)
            .unwrap();
        engine
            .add_general_axis(GeneralAxisOptions::new(
                "x",
                pane,
                AxisDimension::X,
                GeneralScaleType::Temporal,
            ))
            .unwrap();
        engine
            .add_general_axis(GeneralAxisOptions::new(
                "y",
                pane,
                AxisDimension::Y,
                GeneralScaleType::Linear,
            ))
            .unwrap();
        let dataset = engine
            .create_general_xy_dataset(GeneralXyInput::ErrorTemporal {
                ids: None,
                x_epoch_ms: vec![1_700_000_000_000, 1_700_000_060_000],
                y: vec![2.0, 3.0],
                y_valid: None,
                x_low_epoch_ms: vec![1_699_999_970_000.0, 0.0],
                x_low_valid: Some(vec![1, 0]),
                x_high_epoch_ms: vec![1_700_000_030_000.0, 1_700_000_090_000.0],
                x_high_valid: None,
                y_low: vec![1.5, 2.5],
                y_low_valid: None,
                y_high: vec![2.5, 0.0],
                y_high_valid: Some(vec![1, 0]),
            })
            .unwrap();
        engine
            .add_general_series(GeneralSeriesOptions::error_bar(pane, dataset, "x", "y"))
            .unwrap();
        engine.recompute_layout_with_measure(true, |text, _| text.len() as f64 * 7.0, |_, _| 0.0);
        let frame = engine.build_frame();
        let pane_frame = &frame.panes[pane];
        let canvas = canvas_rects(&pane_frame.main, &pane_frame.points);
        let (_plan, metrics) = gpui_plan(&pane_frame.main, &pane_frame.points);
        assert_eq!(
            canvas.path_fills, 2,
            "DPR {dpr}: two temporal centers expected"
        );
        assert!(
            !canvas.rects.is_empty(),
            "DPR {dpr}: temporal XY stems and caps expected"
        );
        assert_eq!(
            metrics.dropped_prims, 0,
            "DPR {dpr}: GPUI must retain all temporal error marks"
        );
        assert_eq!(
            metrics.quads as usize,
            canvas.rects.len(),
            "DPR {dpr}: temporal stems/caps must match Canvas2D"
        );
        assert!(
            metrics.paths >= 2,
            "DPR {dpr}: GPUI must lower temporal center circles"
        );
    }
}

#[test]
fn xy_line_engine_frame_reaches_canvas_and_gpui_stroke_routes() {
    for dpr in [1.0f64, 1.5, 2.0] {
        let mut engine = ChartEngine::new(420.0, 260.0, dpr);
        let pane = engine
            .add_pane_with_domain(
                true,
                HorizontalDomain::Continuous {
                    scale: ContinuousScaleType::Linear,
                },
            )
            .unwrap();
        engine
            .add_general_axis(GeneralAxisOptions::new(
                "x",
                pane,
                AxisDimension::X,
                GeneralScaleType::Linear,
            ))
            .unwrap();
        engine
            .add_general_axis(GeneralAxisOptions::new(
                "y",
                pane,
                AxisDimension::Y,
                GeneralScaleType::Linear,
            ))
            .unwrap();
        let dataset = engine
            .create_general_xy_dataset(GeneralXyInput::Numeric {
                ids: None,
                x: vec![0.0, 1.0, 2.0, 3.0, 4.0],
                y: vec![0.0, 1.0, 99.0, 3.0, 4.0],
                y_valid: Some(vec![1, 1, 0, 1, 1]),
            })
            .unwrap();
        let mut options = GeneralSeriesOptions::xy_line(pane, dataset, "x", "y");
        options.color = Some("#365f91".into());
        engine.add_general_series(options).unwrap();
        engine.recompute_layout_with_measure(true, |text, _| text.len() as f64 * 7.0, |_, _| 0.0);

        let frame = engine.build_frame();
        let pane_frame = &frame.panes[pane];
        let expected = Color::parse_css("#365f91").unwrap();
        assert_eq!(
            pane_frame
                .main
                .iter()
                .filter(|primitive| matches!(primitive, Prim::Polyline { color, point_count: 2, .. } if *color == expected))
                .count(),
            2,
            "DPR {dpr}: missing Y must split XY line into two stroke runs"
        );
        let canvas = canvas_rects(&pane_frame.main, &pane_frame.points);
        let (_plan, metrics) = gpui_plan(&pane_frame.main, &pane_frame.points);
        assert_eq!(
            canvas.path_strokes, 2,
            "DPR {dpr}: Canvas must stroke both line runs"
        );
        assert_eq!(metrics.dropped_prims, 0, "DPR {dpr}: no XY line may drop");
        assert!(
            metrics.paths >= 2,
            "DPR {dpr}: GPUI must lower both XY line runs to paths ({metrics:?})"
        );
    }
}

#[test]
fn xy_area_engine_frame_reaches_canvas_and_gpui_fill_routes() {
    for dpr in [1.0f64, 1.5, 2.0] {
        let mut engine = ChartEngine::new(420.0, 260.0, dpr);
        let pane = engine
            .add_pane_with_domain(
                true,
                HorizontalDomain::Continuous {
                    scale: ContinuousScaleType::Linear,
                },
            )
            .unwrap();
        engine
            .add_general_axis(GeneralAxisOptions::new(
                "x",
                pane,
                AxisDimension::X,
                GeneralScaleType::Linear,
            ))
            .unwrap();
        engine
            .add_general_axis(GeneralAxisOptions::new(
                "y",
                pane,
                AxisDimension::Y,
                GeneralScaleType::Linear,
            ))
            .unwrap();
        let dataset = engine
            .create_general_xy_dataset(GeneralXyInput::Numeric {
                ids: None,
                x: vec![0.0, 1.0, 2.0, 3.0, 4.0],
                y: vec![1.0, 2.0, 99.0, 3.0, 1.0],
                y_valid: Some(vec![1, 1, 0, 1, 1]),
            })
            .unwrap();
        engine
            .add_general_series(GeneralSeriesOptions::xy_area(pane, dataset, "x", "y"))
            .unwrap();
        engine.recompute_layout_with_measure(true, |text, _| text.len() as f64 * 7.0, |_, _| 0.0);

        let frame = engine.build_frame();
        let pane_frame = &frame.panes[pane];
        assert_eq!(
            pane_frame
                .main
                .iter()
                .filter(|primitive| matches!(primitive, Prim::AreaFill { point_count: 2, .. }))
                .count(),
            2,
            "DPR {dpr}: missing Y must split XY area into two fill runs"
        );
        let canvas = canvas_rects(&pane_frame.main, &pane_frame.points);
        let (_plan, metrics) = gpui_plan(&pane_frame.main, &pane_frame.points);
        assert_eq!(
            canvas.path_fills, 2,
            "DPR {dpr}: Canvas must fill both area runs"
        );
        assert!(
            canvas.path_strokes >= 2,
            "DPR {dpr}: Canvas must stroke both area runs"
        );
        assert_eq!(metrics.dropped_prims, 0, "DPR {dpr}: no XY area may drop");
        assert!(
            metrics.paths >= 4,
            "DPR {dpr}: GPUI must lower area fill/stroke paths ({metrics:?})"
        );
    }
}

#[test]
fn stacked_xy_area_engine_frame_reaches_canvas_and_gpui_band_routes() {
    for dpr in [1.0f64, 1.5, 2.0] {
        let mut engine = ChartEngine::new(420.0, 260.0, dpr);
        let pane = engine
            .add_pane_with_domain(
                true,
                HorizontalDomain::Continuous {
                    scale: ContinuousScaleType::Linear,
                },
            )
            .unwrap();
        engine
            .add_general_axis(GeneralAxisOptions::new(
                "x",
                pane,
                AxisDimension::X,
                GeneralScaleType::Linear,
            ))
            .unwrap();
        engine
            .add_general_axis(GeneralAxisOptions::new(
                "y",
                pane,
                AxisDimension::Y,
                GeneralScaleType::Linear,
            ))
            .unwrap();
        let first_dataset = engine
            .create_general_xy_dataset(GeneralXyInput::Numeric {
                ids: None,
                x: vec![0.0, 1.0, 2.0],
                y: vec![1.0, 2.0, 3.0],
                y_valid: None,
            })
            .unwrap();
        let second_dataset = engine
            .create_general_xy_dataset(GeneralXyInput::Numeric {
                ids: None,
                x: vec![0.0, 1.0, 2.0],
                y: vec![2.0, 1.0, 2.0],
                y_valid: None,
            })
            .unwrap();
        let mut first = GeneralSeriesOptions::xy_area(pane, first_dataset, "x", "y");
        first.stack_id = Some("total".into());
        engine.add_general_series(first).unwrap();
        let mut second = GeneralSeriesOptions::xy_area(pane, second_dataset, "x", "y");
        second.stack_id = Some("total".into());
        engine.add_general_series(second).unwrap();
        engine.recompute_layout_with_measure(true, |text, _| text.len() as f64 * 7.0, |_, _| 0.0);

        let frame = engine.build_frame();
        let pane_frame = &frame.panes[pane];
        assert_eq!(
            pane_frame
                .main
                .iter()
                .filter(|primitive| matches!(primitive, Prim::BandFill { point_count: 3, .. }))
                .count(),
            2,
            "DPR {dpr}: each stacked area must emit one variable-bound band"
        );
        let canvas = canvas_rects(&pane_frame.main, &pane_frame.points);
        let (_plan, metrics) = gpui_plan(&pane_frame.main, &pane_frame.points);
        assert_eq!(
            canvas.path_fills, 2,
            "DPR {dpr}: Canvas must fill both stacked area bands"
        );
        assert_eq!(
            canvas.path_strokes, 2,
            "DPR {dpr}: Canvas must stroke both stacked upper boundaries"
        );
        assert_eq!(
            metrics.dropped_prims, 0,
            "DPR {dpr}: GPUI must retain every stacked area primitive"
        );
        assert!(
            metrics.paths >= 4,
            "DPR {dpr}: GPUI must lower both stacked fills and strokes ({metrics:?})"
        );
    }
}

#[test]
fn range_area_engine_frame_reaches_canvas_and_gpui_fill_routes() {
    for dpr in [1.0f64, 1.5, 2.0] {
        let mut engine = ChartEngine::new(420.0, 260.0, dpr);
        let pane = engine
            .add_pane_with_domain(
                true,
                HorizontalDomain::Continuous {
                    scale: ContinuousScaleType::Linear,
                },
            )
            .unwrap();
        engine
            .add_general_axis(GeneralAxisOptions::new(
                "x",
                pane,
                AxisDimension::X,
                GeneralScaleType::Linear,
            ))
            .unwrap();
        engine
            .add_general_axis(GeneralAxisOptions::new(
                "y",
                pane,
                AxisDimension::Y,
                GeneralScaleType::Linear,
            ))
            .unwrap();
        let dataset = engine
            .create_general_xy_dataset(GeneralXyInput::RangeNumeric {
                ids: None,
                x: vec![0.0, 1.0, 2.0, 3.0, 4.0],
                low: vec![1.0, 2.0, 0.0, 3.0, 1.0],
                low_valid: Some(vec![1, 1, 0, 1, 1]),
                high: vec![4.0, 5.0, 6.0, 7.0, 5.0],
                high_valid: None,
            })
            .unwrap();
        engine
            .add_general_series(GeneralSeriesOptions::range_area(pane, dataset, "x", "y"))
            .unwrap();
        engine.recompute_layout_with_measure(true, |text, _| text.len() as f64 * 7.0, |_, _| 0.0);

        let frame = engine.build_frame();
        let pane_frame = &frame.panes[pane];
        assert_eq!(
            pane_frame
                .main
                .iter()
                .filter(|primitive| matches!(primitive, Prim::BandFill { point_count: 2, .. }))
                .count(),
            2,
            "DPR {dpr}: missing bounds must split the range band into two fill runs"
        );
        assert_eq!(
            pane_frame
                .main
                .iter()
                .filter(|primitive| matches!(primitive, Prim::Polyline { point_count: 2, .. }))
                .count(),
            4,
            "DPR {dpr}: both bounds of each range run must be stroked"
        );
        let canvas = canvas_rects(&pane_frame.main, &pane_frame.points);
        let (_plan, metrics) = gpui_plan(&pane_frame.main, &pane_frame.points);
        assert_eq!(
            canvas.path_fills, 2,
            "DPR {dpr}: Canvas must fill both range runs"
        );
        assert_eq!(
            canvas.path_strokes, 4,
            "DPR {dpr}: Canvas must stroke both bounds of both runs"
        );
        assert_eq!(
            metrics.dropped_prims, 0,
            "DPR {dpr}: no range-area primitive may drop"
        );
        assert!(
            metrics.paths >= 6,
            "DPR {dpr}: GPUI must lower range fill/stroke paths ({metrics:?})"
        );
    }
}

#[test]
fn measure_tools_reach_canvas_and_gpui_with_identical_quads_strokes_and_text() {
    use aeris_charts_engine::{DrawingKind, DrawingModifiers, DrawingPoint};
    for dpr in [1.0f64, 1.25, 1.5, 2.0, 2.5] {
        let mut engine = real_engine_frame(dpr);
        engine.build_frame();
        // A rising price range, a backward date range, a falling date-and-price range, and the
        // transient Shift-drag measure (a falling pull, painted market-down): every arrow
        // direction and both the drawing and the pull colors.
        for (kind, from, to) in [
            (DrawingKind::PriceRange, (20.0, 94.0), (45.0, 107.5)),
            (DrawingKind::DateRange, (110.0, 96.0), (80.0, 104.0)),
            (DrawingKind::DatePriceRange, (125.0, 108.0), (160.0, 93.25)),
        ] {
            engine
                .add_drawing(
                    kind,
                    0,
                    vec![
                        DrawingPoint {
                            logical: from.0,
                            price: from.1,
                        },
                        DrawingPoint {
                            logical: to.0,
                            price: to.1,
                        },
                    ],
                    None,
                )
                .expect("measure drawing");
        }
        let at = |engine: &ChartEngine, logical: f64, price: f64| {
            (
                engine.logical_to_coordinate(logical).unwrap(),
                engine.series_price_to_coordinate(0, price).unwrap(),
            )
        };
        let start = at(&engine, 50.0, 99.0);
        let end = at(&engine, 75.0, 92.0);
        let modifiers = DrawingModifiers::default();
        assert!(engine.measure_pointer_down(start.0, start.1, true, modifiers));
        engine.measure_pointer_move(end.0, end.1, modifiers);
        assert!(engine.measure_pointer_up(end.0, end.1, modifiers));

        let frame = engine.build_frame();
        let pane = &frame.panes[0];
        let labels = pane
            .main
            .iter()
            .filter_map(|prim| match prim {
                Prim::Text { text, .. } if text.contains(" bars") || text.contains('%') => {
                    Some(text.clone())
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(
            labels.len() >= 6,
            "DPR {dpr}: every measure must label its statistics ({labels:?})"
        );
        // Every arrow ends in the drawing's cap: an opaque filled head (the area wash is
        // translucent). Price range 1 + date range 1 + date-and-price range 2 + quick measure 2.
        let arrowheads = pane
            .main
            .iter()
            .filter(|prim| matches!(prim, Prim::BandFill { fill, .. } if fill.a() == 255))
            .count();
        assert!(arrowheads >= 6, "DPR {dpr}: missing measure arrowheads");

        let canvas = canvas_rects(&pane.main, &pane.points);
        let (plan, metrics) = gpui_plan(&pane.main, &pane.points);
        assert_eq!(
            gpui_quads(&plan),
            canvas.rects,
            "DPR {dpr}: measure fills, rules, and shafts must be draw-call identical"
        );
        assert_eq!(metrics.dropped_prims, 0, "DPR {dpr}: {metrics:?}");
        assert!(canvas.path_fills >= arrowheads && metrics.paths as usize >= arrowheads);
        let gpui_text = plan
            .ops
            .iter()
            .filter_map(|op| match op {
                SceneOp::Text(run) => Some((run.text.clone(), run.x, run.y)),
                _ => None,
            })
            .collect::<Vec<_>>();
        let canvas_text = canvas
            .text_runs
            .iter()
            .map(|(text, x, y, ..)| (text.clone(), *x, *y))
            .collect::<Vec<_>>();
        assert_eq!(
            gpui_text, canvas_text,
            "DPR {dpr}: measure labels must reach both backends at the same anchors"
        );
    }
}

#[test]
fn a_real_engine_frame_lowers_every_prim_it_contains() {
    let mut engine = real_engine_frame(1.5);
    let frame = engine.build_frame();
    let prepared = PreparedAerisFrame::new(&frame);
    let mut renderer = GpuiChartRenderer::new();
    let metrics = renderer.plan_frame(&prepared, 1.5).expect("frame plans");

    assert!(metrics.prims > 0, "the frame should carry prims");
    assert_eq!(
        metrics.dropped_prims, 0,
        "no prim in a real frame should lower to nothing ({metrics:?})"
    );
    assert!(
        metrics.quads > 0 && metrics.paths > 0,
        "a mixed candle/line/area/histogram frame must exercise both routes ({metrics:?})"
    );
}

#[test]
fn planning_is_deterministic_for_an_unchanged_frame() {
    let mut engine = real_engine_frame(2.0);
    let frame = engine.build_frame();
    let prepared = PreparedAerisFrame::new(&frame);

    let mut a = GpuiChartRenderer::new();
    let ma = a.plan_frame(&prepared, 2.0).unwrap();
    let ops_a = a.plan().ops.clone();
    let verts_a = a.plan().vertices.clone();

    let mut b = GpuiChartRenderer::new();
    let mb = b.plan_frame(&prepared, 2.0).unwrap();

    assert_eq!(ops_a, b.plan().ops, "the op stream must be reproducible");
    assert_eq!(
        verts_a,
        b.plan().vertices,
        "the vertex pool must be reproducible"
    );
    assert_eq!(
        (ma.prims, ma.quads, ma.paths),
        (mb.prims, mb.quads, mb.paths)
    );
}

#[test]
fn odd_even_and_fractional_viewport_geometry_stays_draw_call_identical() {
    // Covers odd/even viewport sizes, fractional pane sizes, and small and large chart
    // bounds". Odd sizes at a fractional DPR are where half-pixel rounding diverges if a backend
    // recomputes geometry instead of consuming the engine's.
    let sizes = [
        (321.0f64, 199.0f64), // small and odd
        (322.0, 200.0),       // small and even
        (1001.0, 601.0),      // odd
        (1000.0, 600.0),      // even
        (1279.5, 719.5),      // fractional
        (2560.0, 1440.0),     // large
    ];
    for (w, h) in sizes {
        for dpr in [1.0f64, 1.25, 1.5, 2.0, 2.5] {
            let mut engine = ChartEngine::new(w, h, dpr);
            let n = 120usize;
            let times: Vec<f64> = (0..n).map(|i| 1_600_000_000.0 + i as f64 * 60.0).collect();
            let close: Vec<f64> = (0..n)
                .map(|i| 100.0 + (i as f64 * 0.17).sin() * 9.0)
                .collect();
            let open: Vec<f64> = close
                .iter()
                .enumerate()
                .map(|(i, c)| if i == 0 { *c } else { close[i - 1] })
                .collect();
            let high: Vec<f64> = open
                .iter()
                .zip(&close)
                .map(|(o, c)| o.max(*c) + 1.0)
                .collect();
            let low: Vec<f64> = open
                .iter()
                .zip(&close)
                .map(|(o, c)| o.min(*c) - 1.0)
                .collect();
            engine
                .set_series_data(0, &times, &open, &high, &low, &close)
                .expect("series loads");
            engine.series[0].kind = SeriesKind::Candlestick;
            engine.css_width = w;
            engine.css_height = h;
            engine.dpr = dpr;
            let content_h = (h - engine.time_axis_height()).max(1.0);
            engine.layout_panes(content_h);
            engine.time_scale.set_width(w);
            engine.fit_content();

            let frame = engine.build_frame();
            for pane in &frame.panes {
                for layer in [&pane.under, &pane.main, &pane.top_prims] {
                    if layer.is_empty() {
                        continue;
                    }
                    let canvas = canvas_rects(layer, &pane.points);
                    let (plan, _) = gpui_plan(layer, &pane.points);
                    assert_eq!(
                        gpui_quads(&plan),
                        canvas.rects,
                        "{w}x{h} @ DPR {dpr}: quad streams diverged"
                    );
                }
            }

            // The frame must also plan cleanly at this geometry, with nothing silently dropped.
            let prepared = PreparedAerisFrame::new(&frame);
            let mut renderer = GpuiChartRenderer::new();
            let m = renderer
                .plan_frame(&prepared, dpr as f32)
                .unwrap_or_else(|e| panic!("{w}x{h} @ DPR {dpr}: {e}"));
            assert_eq!(m.dropped_prims, 0, "{w}x{h} @ DPR {dpr} dropped a prim");
        }
    }
}
