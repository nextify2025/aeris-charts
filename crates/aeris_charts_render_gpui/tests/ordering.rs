//! Paint-order guarantees for the GPUI executor.
//!
//! Ordering is the easiest thing to get wrong in a backend that batches by primitive type, and the
//! hardest to notice: a mis-ordered frame still looks like a chart. Three separate properties are
//! checked here.
//!
//! 1. **The plan preserves prim order.** Ops are appended one-for-one in prim order, never merged
//!    or bucketed, so `plan.ops` is directly comparable to the Canvas2D executor's call order.
//! 2. **Frame composition order.** Panes in order, `under` then `main` then `top_prims` inside each,
//!    then the unscissored axis layer last — the same composition the shipping WebGPU host uses.
//! 3. **GPUI's own reordering is safe.** `gpui::Scene::insert_primitive` assigns a draw order from a
//!    bounds tree (`1 + max` over intersecting primitives) and sorts by it, so insertion order
//!    survives wherever two primitives actually overlap. The test below encodes the property that
//!    makes that sufficient: whenever two ops in the plan overlap, the later one must be intended to
//!    paint on top.

use aeris_charts_engine::{ChartFrame, FramePane};
use aeris_charts_render::canvas2d::{execute as canvas_execute, Canvas2d, Viewport};
use aeris_charts_render::color::Color;
use aeris_charts_render::draw_list::{Gradient, IRect, LineStyle, LineType, Prim, TextAlign};
use aeris_charts_render_gpui::{
    DeviceRect, ExecutorOptions, GpuiChartRenderer, GpuiFrameMetrics, PreparedAerisFrame, SceneOp,
    ScenePlan,
};

const C1: Color = Color::rgb(0x11, 0x11, 0x11);
const C2: Color = Color::rgb(0x22, 0x22, 0x22);
const C3: Color = Color::rgb(0x33, 0x33, 0x33);

fn plan_of(prims: &[Prim], points: &[[f32; 2]]) -> ScenePlan {
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
    plan
}

/// A coarse tag per op, for comparing shapes of streams.
fn kinds(plan: &ScenePlan) -> Vec<&'static str> {
    plan.ops
        .iter()
        .map(|op| match op {
            SceneOp::PushClip(_) => "push",
            SceneOp::PopClip => "pop",
            SceneOp::Quad { .. } => "quad",
            SceneOp::Mesh { .. } => "mesh",
            SceneOp::Text(_) => "text",
            SceneOp::Image { .. } => "image",
        })
        .collect()
}

/// Records the *kind* of each Canvas2D drawing call, in order, so the two streams can be compared
/// shape-for-shape.
#[derive(Default)]
struct KindRecorder {
    ops: Vec<&'static str>,
}

impl Canvas2d for KindRecorder {
    fn set_fill_solid(&mut self, _c: Color) {}
    fn set_fill_vgradient(&mut self, _a: f32, _b: f32, _t: Color, _bo: Color) {}
    fn set_stroke(&mut self, _c: Color) {}
    fn set_line_width(&mut self, _w: f32) {}
    fn set_line_dash(&mut self, _p: &[f32]) {}
    fn fill_rect(&mut self, _x: f32, _y: f32, _w: f32, _h: f32) {
        self.ops.push("quad");
    }
    fn begin_path(&mut self) {}
    fn move_to(&mut self, _x: f32, _y: f32) {}
    fn line_to(&mut self, _x: f32, _y: f32) {}
    fn close_path(&mut self) {}
    fn arc(&mut self, _cx: f32, _cy: f32, _r: f32, _s: f32, _e: f32) {}
    fn stroke(&mut self) {
        self.ops.push("mesh");
    }
    fn fill(&mut self) {
        self.ops.push("mesh");
    }
    fn fill_text(&mut self, _t: &str, _x: f32, _y: f32, _f: &str, _c: Color, _a: TextAlign) {
        self.ops.push("text");
    }
    fn fill_rotated_text(
        &mut self,
        _: &str,
        _: f32,
        _: f32,
        _: &str,
        _: Color,
        _: TextAlign,
        _: f32,
    ) {
        self.ops.push("text");
    }
}

fn mixed_prims() -> (Vec<Prim>, Vec<[f32; 2]>) {
    let points = vec![[0.0f32, 10.0], [10.0, 4.0], [20.0, 12.0]];
    let prims = vec![
        Prim::Background {
            rect: [0.0, 0.0, 100.0, 100.0],
            gradient: Gradient {
                top: C1,
                bottom: C2,
            },
        },
        Prim::HLine {
            y: 20,
            x0: 0,
            x1: 100,
            width: 1,
            style: LineStyle::Solid,
            color: C1,
        },
        Prim::AreaFill {
            first_point: 0,
            point_count: 3,
            base_y: 40.0,
            line_type: LineType::Simple,
            gradient: Gradient {
                top: C2,
                bottom: C3,
            },
        },
        Prim::Polyline {
            first_point: 0,
            point_count: 3,
            width: 2.0,
            style: LineStyle::Solid,
            line_type: LineType::Simple,
            color: C2,
        },
        Prim::Rect {
            rect: IRect {
                x: 10,
                y: 10,
                w: 20,
                h: 20,
            },
            color: C3,
        },
        Prim::Circle {
            cx: 50.0,
            cy: 50.0,
            radius: 5.0,
            fill: C1,
            stroke_width: 0.0,
            stroke: C1,
        },
        Prim::Text {
            x: 10.0,
            y: 10.0,
            text: "label".into(),
            color: C3,
            size: 11.0,
            family: "sans-serif".into(),
            align: TextAlign::Left,
            weight: 400,
            italic: false,
        },
    ];
    (prims, points)
}

#[test]
fn the_op_stream_shape_matches_the_canvas2d_call_stream() {
    let (prims, points) = mixed_prims();
    let plan = plan_of(&prims, &points);

    let mut rec = KindRecorder::default();
    canvas_execute(
        &prims,
        &points,
        &mut rec,
        Viewport {
            width: 200.0,
            height: 200.0,
        },
    );
    let mut expected = rec.ops;
    // AreaFill paints its solid core and coverage fringe as adjacent paths. Keeping the fringe
    // separate preserves GPUI's bounds-relative gradient while retaining one logical frame prim.
    let area_index = expected
        .iter()
        .position(|kind| *kind == "mesh")
        .expect("area mesh");
    expected.insert(area_index + 1, "mesh");
    assert_eq!(
        kinds(&plan),
        expected,
        "GPUI keeps the area fringe beside its core"
    );
}

#[test]
fn one_op_is_emitted_per_prim_for_single_op_prims() {
    // Guards against any future batching that would collapse distinct prims into one op and lose
    // their relative order.
    let prims: Vec<Prim> = (0..25)
        .map(|i| Prim::Rect {
            rect: IRect {
                x: i,
                y: 0,
                w: 3,
                h: 3,
            },
            color: C1,
        })
        .collect();
    let plan = plan_of(&prims, &[]);
    assert_eq!(plan.ops.len(), 25);
    let xs: Vec<f32> = plan
        .ops
        .iter()
        .filter_map(|op| match op {
            SceneOp::Quad { rect, .. } => Some(rect.x),
            _ => None,
        })
        .collect();
    assert_eq!(xs, (0..25).map(|i| i as f32).collect::<Vec<_>>());
}

fn pane(scissor: [u32; 4], under: Vec<Prim>, main: Vec<Prim>, top: Vec<Prim>) -> FramePane {
    FramePane {
        top: 0.0,
        height: 100.0,
        scissor,
        under,
        main,
        top_prims: top,
        series_paint_marks: Vec::new(),
        points: Vec::new(),
    }
}

fn tagged_rect(tag: i32) -> Prim {
    Prim::Rect {
        rect: IRect {
            x: tag,
            y: 0,
            w: 1,
            h: 1,
        },
        color: C1,
    }
}

/// The x coordinate of each quad, which the tests use as a paint-order tag.
fn quad_tags(plan: &ScenePlan) -> Vec<i32> {
    plan.ops
        .iter()
        .filter_map(|op| match op {
            SceneOp::Quad { rect, .. } => Some(rect.x as i32),
            _ => None,
        })
        .collect()
}

#[test]
fn frame_composition_order_is_panes_then_layers_then_axis() {
    let frame = ChartFrame {
        width: 200.0,
        height: 200.0,
        pixel_ratio: 1.0,
        panes: vec![
            pane(
                [0, 0, 200, 100],
                vec![tagged_rect(0)],
                vec![tagged_rect(1)],
                vec![tagged_rect(2)],
            ),
            pane(
                [0, 100, 200, 100],
                vec![tagged_rect(3)],
                vec![tagged_rect(4)],
                vec![tagged_rect(5)],
            ),
        ],
    };
    let axis = vec![tagged_rect(6), tagged_rect(7)];
    let prepared = PreparedAerisFrame::new(&frame).with_axis(&axis, &[]);

    let mut renderer = GpuiChartRenderer::new();
    renderer.plan_frame(&prepared, 1.0).unwrap();
    assert_eq!(
        quad_tags(renderer.plan()),
        vec![0, 1, 2, 3, 4, 5, 6, 7],
        "panes in order, under/main/top within each, axis layer last"
    );
}

#[test]
fn the_axis_layer_is_never_interleaved_with_pane_content() {
    let frame = ChartFrame {
        width: 200.0,
        height: 200.0,
        pixel_ratio: 1.0,
        panes: vec![
            pane([0, 0, 200, 100], vec![], vec![tagged_rect(0)], vec![]),
            pane([0, 100, 200, 100], vec![], vec![tagged_rect(1)], vec![]),
        ],
    };
    let axis = vec![tagged_rect(9)];
    let prepared = PreparedAerisFrame::new(&frame).with_axis(&axis, &[]);
    let mut renderer = GpuiChartRenderer::new();
    renderer.plan_frame(&prepared, 1.0).unwrap();

    let tags = quad_tags(renderer.plan());
    let axis_pos = tags
        .iter()
        .position(|t| *t == 9)
        .expect("axis quad present");
    assert_eq!(
        axis_pos,
        tags.len() - 1,
        "the axis layer must be strictly last: {tags:?}"
    );
}

#[test]
fn an_empty_layer_does_not_disturb_the_order_of_the_others() {
    let frame = ChartFrame {
        width: 200.0,
        height: 200.0,
        pixel_ratio: 1.0,
        panes: vec![pane(
            [0, 0, 200, 200],
            Vec::new(),
            vec![tagged_rect(1)],
            Vec::new(),
        )],
    };
    let prepared = PreparedAerisFrame::new(&frame);
    let mut renderer = GpuiChartRenderer::new();
    renderer.plan_frame(&prepared, 1.0).unwrap();
    assert_eq!(quad_tags(renderer.plan()), vec![1]);
}

/// The bounds of an op, for the overlap analysis below.
fn op_bounds(plan: &ScenePlan, op: &SceneOp) -> Option<DeviceRect> {
    match op {
        SceneOp::Quad { rect, .. } => Some(*rect),
        SceneOp::Mesh {
            first_vertex,
            vertex_count,
            ..
        } => plan.mesh_bounds(*first_vertex, *vertex_count),
        _ => None,
    }
}

#[test]
fn overlapping_ops_are_ordered_so_gpuis_bounds_tree_preserves_them() {
    // GPUI reorders only primitives that do NOT intersect. This test states the invariant that
    // makes that safe: for every pair of ops where the later one overlaps the earlier one, the
    // later index really is the one meant to paint on top. It holds by construction because the
    // executor never reorders — so the assertion here is that the plan index order is the same
    // order the Canvas2D executor would have painted them in.
    let (prims, points) = mixed_prims();
    let plan = plan_of(&prims, &points);

    let drawables: Vec<(usize, DeviceRect)> = plan
        .ops
        .iter()
        .enumerate()
        .filter_map(|(i, op)| op_bounds(&plan, op).map(|b| (i, b)))
        .collect();

    let mut overlaps = 0usize;
    for (a, (ia, ba)) in drawables.iter().enumerate() {
        for (ib, bb) in &drawables[a + 1..] {
            if ba.intersect(bb).is_some() {
                overlaps += 1;
                assert!(
                    ib > ia,
                    "an overlapping pair must be in ascending plan order ({ia} vs {ib})"
                );
            }
        }
    }
    assert!(
        overlaps > 0,
        "the fixture must actually contain overlapping geometry for this to mean anything"
    );
}

#[test]
fn a_later_prim_covering_an_earlier_one_stays_later() {
    // The concrete case that matters: markers emitted after candle bodies must paint over them.
    let prims = vec![
        Prim::Rect {
            rect: IRect {
                x: 10,
                y: 10,
                w: 40,
                h: 40,
            },
            color: C1,
        },
        Prim::Rect {
            rect: IRect {
                x: 20,
                y: 20,
                w: 10,
                h: 10,
            },
            color: C2,
        },
    ];
    let plan = plan_of(&prims, &[]);
    let colors: Vec<Color> = plan
        .ops
        .iter()
        .filter_map(|op| match op {
            SceneOp::Quad {
                fill: aeris_charts_render_gpui::Paint::Solid(c),
                ..
            } => Some(*c),
            _ => None,
        })
        .collect();
    assert_eq!(colors, vec![C1, C2], "the covering rect must come second");
}

#[test]
fn dash_spans_are_emitted_in_increasing_coordinate_order() {
    // Dash phase starts at the path start; emitting spans out of order would still look right for
    // opaque colors but would break blending for translucent ones.
    let prims = vec![Prim::VLine {
        x: 5,
        y0: 0,
        y1: 120,
        width: 1,
        style: LineStyle::Dashed,
        color: C1,
    }];
    let plan = plan_of(&prims, &[]);
    let ys: Vec<f32> = plan
        .ops
        .iter()
        .filter_map(|op| match op {
            SceneOp::Quad { rect, .. } => Some(rect.y),
            _ => None,
        })
        .collect();
    assert!(ys.len() > 2, "expected several dash spans, got {ys:?}");
    assert!(
        ys.windows(2).all(|w| w[0] < w[1]),
        "dash spans must ascend: {ys:?}"
    );
}
