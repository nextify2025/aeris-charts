//! Shapes-family engine tests: catalog defaults, armed placement for every placement class,
//! shared-part frames and fills, hit testing (indexed and brute force), drags, keyboard nudges,
//! magnet, time identity, schema and kind options, patches with history, persistence, clipboard,
//! sync, device-pixel ratios, and bounded work at extreme zoom.

use aeris_charts_render::color::Color;
use aeris_charts_render::draw_list::{LineStyle, Prim};
use aeris_charts_render::shape::{self, Point, Rect, MAX_FLATTEN_POINTS};

use super::super::super::DrawingTextLayout;
use super::ShapeToolOptions;
use crate::drawings::{resolve_drawing_geometry, DrawingBodyGeometry, DrawingGeometryOptions};
use crate::{
    ChartEngine, DrawingAnchor, DrawingId, DrawingKind, DrawingMagnetMode, DrawingModifiers,
    DrawingPoint,
};

const SHAPE_KINDS: [DrawingKind; 9] = [
    DrawingKind::RotatedRectangle,
    DrawingKind::Ellipse,
    DrawingKind::Circle,
    DrawingKind::Triangle,
    DrawingKind::Arc,
    DrawingKind::Curve,
    DrawingKind::DoubleCurve,
    DrawingKind::Polyline,
    DrawingKind::Highlighter,
];
const INK: &str = "#123456";
const HOUR: f64 = 3_600.0;

fn chart_with(times: &[f64], dpr: f64) -> ChartEngine {
    let mut chart = ChartEngine::new(800.0, 500.0, dpr);
    let values = (0..times.len())
        .map(|index| 100.0 + (index % 7) as f64)
        .collect::<Vec<_>>();
    chart
        .set_series_data(0, times, &values, &values, &values, &values)
        .unwrap();
    chart.time_scale.set_width(800.0);
    chart.fit_content();
    chart.build_frame();
    chart
}

fn hourly(count: usize) -> Vec<f64> {
    (0..count).map(|index| index as f64 * HOUR).collect()
}

fn chart() -> ChartEngine {
    chart_with(&hourly(40), 1.0)
}

fn p(logical: f64, price: f64) -> DrawingPoint {
    DrawingPoint { logical, price }
}

fn points_for(kind: DrawingKind) -> Vec<DrawingPoint> {
    match kind {
        DrawingKind::RotatedRectangle => vec![p(10.0, 101.0), p(20.0, 103.0), p(14.0, 104.0)],
        DrawingKind::Ellipse => vec![p(10.0, 101.0), p(20.0, 105.0)],
        DrawingKind::Circle => vec![p(15.0, 103.0), p(19.0, 103.0)],
        DrawingKind::Triangle => vec![p(10.0, 101.0), p(20.0, 101.0), p(15.0, 105.0)],
        DrawingKind::Arc => vec![p(10.0, 101.0), p(20.0, 101.0), p(15.0, 104.0)],
        DrawingKind::Curve => vec![p(10.0, 101.0), p(20.0, 101.0), p(15.0, 105.0)],
        DrawingKind::DoubleCurve => {
            vec![
                p(10.0, 101.0),
                p(22.0, 101.0),
                p(14.0, 105.0),
                p(18.0, 100.0),
            ]
        }
        DrawingKind::Polyline => {
            vec![
                p(10.0, 101.0),
                p(14.0, 105.0),
                p(18.0, 102.0),
                p(22.0, 104.0),
            ]
        }
        DrawingKind::Highlighter => {
            vec![
                p(10.0, 101.0),
                p(12.0, 102.0),
                p(14.0, 101.5),
                p(16.0, 103.0),
            ]
        }
        _ => unreachable!("not a shapes tool"),
    }
}

fn add(
    chart: &mut ChartEngine,
    kind: DrawingKind,
    points: Vec<DrawingPoint>,
    options: &str,
) -> DrawingId {
    chart.add_drawing(kind, 0, points, Some(options)).unwrap()
}

fn anchor(chart: &ChartEngine, id: DrawingId, index: usize) -> Point {
    chart.drawing_point_to_coordinate(id, index).unwrap()
}

fn ink() -> Color {
    Color::parse_css(INK).unwrap()
}

fn pool(points: &[[f32; 2]], first: u32, count: u32) -> Vec<Point> {
    points[first as usize..(first + count) as usize]
        .iter()
        .map(|point| (f64::from(point[0]), f64::from(point[1])))
        .collect()
}

/// Every polyline of `color` on the first pane, with its width.
fn polylines(chart: &mut ChartEngine, color: Color) -> Vec<(Vec<Point>, f32)> {
    let frame = chart.build_frame();
    let pane = &frame.panes[0];
    pane.main
        .iter()
        .filter_map(|prim| match prim {
            Prim::Polyline {
                first_point,
                point_count,
                width,
                color: stroke,
                ..
            } if *stroke == color => Some((pool(&pane.points, *first_point, *point_count), *width)),
            _ => None,
        })
        .collect()
}

fn hit(chart: &ChartEngine, point: Point) -> Option<DrawingId> {
    chart.hit_test_drawing(point.0, point.1).map(|hit| hit.id)
}

/// The fork's pre-merge defaults, which documents it wrote omitted, come back for every shape
/// through `apply_legacy_fork_defaults` (upstream renders the shapes now).
#[test]
fn catalog_defaults_follow_each_tool() {
    for kind in SHAPE_KINDS {
        let spec = kind.spec();
        assert!(spec.family.is_none(), "{kind:?} is upstream-rendered");
        assert_eq!(spec.text_layout, DrawingTextLayout::Box);
        let mut drawing = crate::Drawing::new(1, kind, 0, Vec::new());
        super::super::apply_legacy_fork_defaults(&mut drawing);
        assert_eq!(
            drawing.fill_enabled,
            !matches!(
                kind,
                DrawingKind::Curve | DrawingKind::DoubleCurve | DrawingKind::Highlighter
            ),
            "{kind:?} fill default"
        );
        assert_eq!(
            drawing.width,
            if kind == DrawingKind::Highlighter {
                20.0
            } else {
                2.0
            }
        );
        assert!(drawing.tool_options.is_empty());
    }
    let mut highlighter = crate::Drawing::new(1, DrawingKind::Highlighter, 0, Vec::new());
    super::super::apply_legacy_fork_defaults(&mut highlighter);
    let color = Color::parse_css(&highlighter.color).expect("a CSS color");
    assert_eq!((color.r(), color.g(), color.b()), (0xf5, 0x9e, 0x0a));
    assert_eq!(color.a(), 102, "40% opacity");
    // The option block stays a public, serializable type.
    assert_eq!(
        serde_json::to_value(ShapeToolOptions { closed: true }).unwrap(),
        serde_json::json!({ "closed": true })
    );
}

#[test]
fn dashed_outlines_split_into_solid_runs_for_every_executor() {
    let mut chart = chart();
    let id = add(
        &mut chart,
        DrawingKind::Triangle,
        points_for(DrawingKind::Triangle),
        r##"{"color":"#123456","style":"dashed"}"##,
    );
    let frame = chart.build_frame();
    let pane = &frame.panes[0];
    let runs = pane
        .main
        .iter()
        .filter_map(|prim| match prim {
            Prim::Polyline {
                first_point,
                point_count,
                style,
                color,
                ..
            } if *color == ink() => Some((pool(&pane.points, *first_point, *point_count), *style)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(runs.len() > 10, "{} dash runs", runs.len());
    assert!(runs
        .iter()
        .all(|(_, style)| *style == aeris_charts_render::draw_list::LineStyle::Solid));
    // Six-width dashes and gaps: the runs cover half the perimeter.
    let length = |line: &[Point]| {
        line.windows(2)
            .map(|pair| (pair[1].0 - pair[0].0).hypot(pair[1].1 - pair[0].1))
            .sum::<f64>()
    };
    let painted: f64 = runs.iter().map(|(line, _)| length(line)).sum();
    let vertices = (0..3)
        .map(|index| anchor(&chart, id, index))
        .collect::<Vec<_>>();
    let perimeter = length(&[vertices[0], vertices[1], vertices[2], vertices[0]]);
    assert!(
        (painted / perimeter - 0.5).abs() < 0.05,
        "{painted} of {perimeter}"
    );
}

#[test]
fn indexed_hit_testing_matches_brute_force() {
    let mut chart = chart();
    for copy in 0..3 {
        let shift = f64::from(copy) * 0.9;
        for kind in SHAPE_KINDS {
            let points = points_for(kind)
                .into_iter()
                .map(|point| p(point.logical + shift * 3.0, point.price + shift))
                .collect();
            add(&mut chart, kind, points, "{}");
        }
    }
    assert!(chart.drawings().len() > 20, "exercises the culled path");
    chart.set_selected_drawing(Some(chart.drawings()[2].id));
    chart.build_frame();
    let mut hits = 0;
    for gy in 0..48 {
        for gx in 0..78 {
            let (x, y) = (f64::from(gx) * 10.0 + 3.0, f64::from(gy) * 10.0 + 4.0);
            let indexed = chart.hit_test_drawing(x, y);
            assert_eq!(
                indexed,
                chart.hit_test_drawing_bruteforce(x, y),
                "({x}, {y})"
            );
            hits += usize::from(indexed.is_some());
        }
    }
    assert!(hits > 100, "the grid meets the shapes ({hits} hits)");
}

#[test]
fn chart_magnet_snaps_shape_placement_to_bar_values() {
    let mut chart = chart();
    chart.set_drawing_magnet_mode(DrawingMagnetMode::Strong);
    assert!(chart.set_drawing_tool(Some(DrawingKind::Circle), None, None));
    let x = chart.logical_to_coordinate(12.0).unwrap() + 3.0;
    let y = chart.series_price_to_coordinate(0, 102.4).unwrap();
    chart.drawing_tool_activate(x, y, DrawingModifiers::default());
    let rim_x = chart.logical_to_coordinate(16.0).unwrap();
    let id = chart
        .drawing_tool_activate(rim_x, y, DrawingModifiers::default())
        .created
        .unwrap();
    let center = chart.drawing(id).unwrap().points[0];
    assert_eq!(center.logical, 12.0);
    assert_eq!(center.price, 100.0 + (12 % 7) as f64);
}

#[test]
fn shape_anchors_resolve_by_time_across_an_interval_switch() {
    let mut chart = chart();
    let anchors =
        [(10.0, 101.0), (20.5, 101.0), (15.0, 104.0)].map(|(hour, price)| DrawingAnchor {
            logical: None,
            price,
            time: Some(hour * HOUR),
        });
    let id = chart
        .add_drawing_anchors(DrawingKind::Triangle, 0, &anchors, None)
        .unwrap();
    let half_hourly = (0..80)
        .map(|index| index as f64 * HOUR / 2.0)
        .collect::<Vec<_>>();
    let values = vec![100.0; half_hourly.len()];
    chart
        .set_series_data(0, &half_hourly, &values, &values, &values, &values)
        .unwrap();
    let resolved = chart.drawing_anchors(id).unwrap();
    assert_eq!(resolved[0].logical, Some(20.0));
    assert_eq!(resolved[1].logical, Some(41.0));
    assert_eq!(resolved[2].logical, Some(30.0));
    assert_eq!(resolved[1].time, Some(20.5 * HOUR));
}

#[test]
fn persistence_round_trips_shapes_and_omits_kind_defaults() {
    let mut chart = chart();
    for kind in SHAPE_KINDS {
        add(&mut chart, kind, points_for(kind), "{}");
    }
    let exported: serde_json::Value =
        serde_json::from_str(&chart.export_state_json().unwrap()).unwrap();
    for drawing in exported["drawings"].as_array().unwrap() {
        for field in ["fill_enabled", "tool_options", "stroke_end", "extend_right"] {
            assert!(
                drawing["style"].get(field).is_none(),
                "{} writes its default {field}",
                drawing["kind"]
            );
        }
    }
    let customized = [
        (DrawingKind::Circle, r#"{"fill_enabled":false}"#),
        (
            DrawingKind::Polyline,
            r#"{"tool_options":{"shape":{"closed":true}},"fill_color":"rgba(1, 2, 3, 0.5)"}"#,
        ),
        (
            DrawingKind::Curve,
            r#"{"fill_enabled":true,"extend_right":true,"stroke_start":"arrow"}"#,
        ),
        (
            DrawingKind::Highlighter,
            r##"{"width":12,"color":"#ff00ff"}"##,
        ),
        (
            DrawingKind::RotatedRectangle,
            r#"{"text":"zone","style":"dashed"}"#,
        ),
    ];
    for (kind, options) in customized {
        add(&mut chart, kind, points_for(kind), options);
    }
    let document = chart.export_state_json().unwrap();
    let mut restored = ChartEngine::new(800.0, 500.0, 1.0);
    restored.import_state_json(&document).unwrap();
    assert_eq!(restored.export_state_json().unwrap(), document);
    assert_eq!(restored.drawings().len(), chart.drawings().len());
    for (restored, original) in restored.drawings().iter().zip(chart.drawings()) {
        assert_eq!(restored.kind, original.kind);
        assert_eq!(restored.points, original.points);
        assert_eq!(restored.fill_enabled, original.fill_enabled);
        assert_eq!(restored.fill_color, original.fill_color);
        assert_eq!(restored.tool_options, original.tool_options);
        assert_eq!(restored.color, original.color);
        assert_eq!(restored.width, original.width);
        assert_eq!(restored.extend_right, original.extend_right);
        assert_eq!(restored.stroke_start, original.stroke_start);
        assert_eq!(restored.text, original.text);
    }
}

#[test]
fn clipboard_and_sync_payloads_carry_shape_options() {
    let mut source = chart();
    let id = add(
        &mut source,
        DrawingKind::Polyline,
        points_for(DrawingKind::Polyline),
        r#"{"tool_options":{"shape":{"closed":true}},"fill_enabled":false}"#,
    );
    let copied = source.copy_drawings_json(&[id]).unwrap();
    let mut target = chart();
    let pasted = target.paste_drawings_json(&copied, 0, 0.0, 0.0).unwrap();
    let drawing = target.drawing(pasted[0]).unwrap();
    assert_eq!(drawing.kind, DrawingKind::Polyline);
    assert_eq!(drawing.points.len(), 4);
    assert!(!drawing.fill_enabled);
    assert_eq!(
        drawing.tool_options,
        source.drawing(id).unwrap().tool_options
    );

    let payload = source.drawing_sync_payload_json("cell-a").unwrap();
    let mut mirror = chart();
    assert!(mirror.apply_drawing_sync_payload_json(&payload));
    assert_eq!(
        mirror.drawings()[0].tool_options,
        source.drawing(id).unwrap().tool_options
    );
    assert!(!mirror.drawings()[0].fill_enabled);

    // A named template carries the typed block to another polyline, never to another kind.
    let template = source.drawing_template_json(id, "closed outline").unwrap();
    let open = add(
        &mut target,
        DrawingKind::Polyline,
        points_for(DrawingKind::Polyline),
        "{}",
    );
    assert!(target.apply_drawing_template_json(open, &template));
    let restyled = target.drawing(open).unwrap();
    assert_eq!(
        restyled.tool_options.shape,
        Some(ShapeToolOptions { closed: true })
    );
    assert!(!restyled.fill_enabled);
    let triangle = add(
        &mut target,
        DrawingKind::Triangle,
        points_for(DrawingKind::Triangle),
        "{}",
    );
    assert!(!target.apply_drawing_template_json(triangle, &template));
    assert!(target.drawing(triangle).unwrap().tool_options.is_empty());
}

#[test]
fn hand_written_documents_restore_shapes_with_kind_defaults() {
    // Minimal V1 documents as a host or an older export writes them: absent style fields take
    // each shape's own defaults, and a wrong anchor count or a malformed option block fails
    // without mutation.
    let document = r#"{"schema":"aeris_charts-state","schema_version":1,"panes":[{"id":"pane-1"}],
        "drawings":[
          {"id":1,"kind":"triangle","pane_id":"pane-1","anchors":[{"logical":1,"price":10},{"logical":5,"price":10},{"logical":3,"price":12}]},
          {"id":2,"kind":"highlighter","pane_id":"pane-1","anchors":[{"logical":1,"price":10},{"logical":2,"price":11},{"logical":3,"price":10.5}],"style":{}},
          {"id":3,"kind":"polyline","pane_id":"pane-1","anchors":[{"logical":1,"price":9},{"logical":2,"price":12},{"logical":4,"price":10}],"style":{"tool_options":{"shape":{"closed":true}}}}
        ]}"#;
    let mut chart = chart();
    chart.import_state_json(document).unwrap();
    let drawings = chart.drawings();
    assert_eq!(drawings.len(), 3);
    assert!(drawings[0].fill_enabled, "a triangle fills by default");
    assert_eq!(drawings[1].width, 20.0);
    assert!(drawings[1].color.starts_with("rgba("));
    assert_eq!(
        drawings[2].tool_options.shape,
        Some(ShapeToolOptions { closed: true })
    );
    assert!(drawings[2].fill_enabled);
    chart.build_frame();

    let before = chart.export_state_json().unwrap();
    for broken in [
        r#"{"schema":"aeris_charts-state","schema_version":1,"panes":[{"id":"pane-1"}],"drawings":[{"id":1,"kind":"triangle","pane_id":"pane-1","anchors":[{"logical":1,"price":10},{"logical":5,"price":10}]}]}"#,
        r#"{"schema":"aeris_charts-state","schema_version":1,"panes":[{"id":"pane-1"}],"drawings":[{"id":1,"kind":"polyline","pane_id":"pane-1","anchors":[{"logical":1,"price":10},{"logical":5,"price":10}],"style":{"tool_options":{"shape":{"closed":1}}}}]}"#,
    ] {
        assert!(chart.import_state_json(broken).is_err(), "{broken}");
        assert_eq!(chart.export_state_json().unwrap(), before);
    }
}

/// Twenty-two trend lines past the anchors under test, so candidate queries take the culled path.
fn crowd(chart: &mut ChartEngine) {
    for index in 0..22 {
        add(
            chart,
            DrawingKind::TrendLine,
            vec![p(30.0 + index as f64 * 0.1, 100.0), p(31.0, 100.5)],
            "{}",
        );
    }
}

#[test]
fn screen_derived_shapes_paint_and_hit_with_their_anchors_scrolled_away() {
    let mut chart = chart();
    crowd(&mut chart);
    // A circle centered left of the pane whose rim reaches into it.
    let id = add(
        &mut chart,
        DrawingKind::Circle,
        vec![p(-12.0, 103.0), p(8.0, 103.0)],
        r##"{"color":"#123456"}"##,
    );
    let rim = anchor(&chart, id, 1);
    assert!(rim.0 > 0.0 && rim.0 < chart.pane_w);
    let candidates = chart.take_drawing_candidates(0, None);
    assert!(candidates.contains(&id));
    chart.recycle_drawing_candidates(candidates);
    assert!(!polylines(&mut chart, ink()).is_empty());
    assert_eq!(hit(&chart, rim), Some(id));
    // Their own box culls them exactly: a small circle beside the pane is no candidate, and one
    // under the pointer is the only full-extent shape a hit test examines.
    let away = add(
        &mut chart,
        DrawingKind::Circle,
        vec![p(-30.0, 103.0), p(-29.0, 103.0)],
        "{}",
    );
    let candidates = chart.take_drawing_candidates(0, None);
    assert!(candidates.contains(&id) && !candidates.contains(&away));
    chart.recycle_drawing_candidates(candidates);
    let candidates = chart.take_drawing_candidates(0, Some(rim));
    assert!(candidates.contains(&id) && !candidates.contains(&away));
    chart.recycle_drawing_candidates(candidates);
    // Curves cull by their padded logical span once it leaves the viewport.
    let curve = add(
        &mut chart,
        DrawingKind::Curve,
        vec![p(-80.0, 101.0), p(-60.0, 101.0), p(-70.0, 104.0)],
        "{}",
    );
    let candidates = chart.take_drawing_candidates(0, None);
    assert!(!candidates.contains(&curve));
    chart.recycle_drawing_candidates(candidates);
}

#[test]
fn box_text_centers_on_the_shape() {
    let mut chart = chart();
    let id = add(
        &mut chart,
        DrawingKind::Circle,
        points_for(DrawingKind::Circle),
        r##"{"text":"zone"}"##,
    );
    let center = anchor(&chart, id, 0);
    let frame = chart.build_frame();
    let (x, y) = frame.panes[0]
        .main
        .iter()
        .find_map(|prim| match prim {
            Prim::Text { text, x, y, .. } if text == "zone" => Some((*x, *y)),
            _ => None,
        })
        .expect("the label");
    assert!((f64::from(x) - center.0).abs() < 1e-3);
    assert!((f64::from(y) - center.1).abs() < 1e-3);
}

#[test]
fn shapes_tolerate_charts_without_data_and_degenerate_anchors() {
    let mut empty = ChartEngine::new(800.0, 500.0, 1.0);
    for kind in SHAPE_KINDS {
        assert!(empty.add_drawing(kind, 0, points_for(kind), None).is_some());
    }
    empty.build_frame();
    assert_eq!(empty.hit_test_drawing(100.0, 100.0), None);

    let mut chart = chart();
    for kind in SHAPE_KINDS {
        let point = p(12.0, 102.0);
        let count = points_for(kind).len();
        assert!(chart
            .add_drawing(
                kind,
                0,
                vec![point; count],
                Some(r#"{"fill_enabled":true}"#)
            )
            .is_some());
    }
    let frame = chart.build_frame();
    assert!(frame.panes[0]
        .points
        .iter()
        .all(|point| point[0].is_finite() && point[1].is_finite()));
    assert!(chart
        .add_drawing(
            DrawingKind::Circle,
            0,
            vec![p(f64::NAN, 1.0), p(2.0, 3.0)],
            None
        )
        .is_none());
    let _ = chart.hit_test_drawing(300.0, 200.0);
}

/// A chart whose prices sit far above zero, so anchors tens of thousands of px away keep
/// positive prices.
fn deep_chart(width: f64, dpr: f64) -> ChartEngine {
    let mut chart = ChartEngine::new(width, 500.0, dpr);
    let times = hourly(40);
    let values = (0..times.len())
        .map(|index| 10_000.0 + (index % 7) as f64)
        .collect::<Vec<_>>();
    chart
        .set_series_data(0, &times, &values, &values, &values, &values)
        .unwrap();
    chart.time_scale.set_width(width);
    chart.fit_content();
    chart.build_frame();
    chart
}

/// The anchor at media px `(x, y)`.
fn at(chart: &ChartEngine, (x, y): Point) -> DrawingPoint {
    p(
        chart.time_scale.coordinate_to_float_index(x),
        chart.series_coordinate_to_price(0, y).unwrap(),
    )
}

/// Horizontal and vertical bitmap px per media px.
fn ratios(chart: &ChartEngine, dpr: f64) -> (f64, f64) {
    (
        (chart.pane_w * dpr).round() / chart.pane_w,
        (chart.pane_h * dpr).round() / chart.pane_h,
    )
}

/// The drawing's body geometry resolved from its anchors scaled by `(hpr, vpr)`, the frame's
/// own resolution (bitmap px for the frame ratios, media px for `(1, 1)`).
fn body_at(
    chart: &ChartEngine,
    id: DrawingId,
    (hpr, vpr): (f64, f64),
) -> DrawingBodyGeometry<'static> {
    let drawing = chart.drawing(id).unwrap();
    let px: &'static [Point] = Vec::leak(
        (0..drawing.points.len())
            .map(|index| {
                let (x, y) = anchor(chart, id, index);
                (x * hpr, y * vpr)
            })
            .collect(),
    );
    let pane = &chart.panes[0];
    resolve_drawing_geometry(
        drawing.kind,
        px,
        chart.pane_w * hpr,
        pane.top * vpr,
        pane.height * vpr,
        DrawingGeometryOptions::for_drawing(drawing, vpr),
    )
    .unwrap()
    .body
}

/// The true curve of a curved body at `t` in `[0, 1]`.
fn curve_point(body: DrawingBodyGeometry<'_>, t: f64) -> Point {
    let turn = std::f64::consts::TAU * t;
    match body {
        DrawingBodyGeometry::Circle { center, radius } => (
            center.0 + radius * turn.cos(),
            center.1 + radius * turn.sin(),
        ),
        DrawingBodyGeometry::Ellipse { center, rx, ry } => {
            (center.0 + rx * turn.cos(), center.1 + ry * turn.sin())
        }
        DrawingBodyGeometry::Arc(arc) => arc.point(t),
        DrawingBodyGeometry::Curve(curve) => curve.point(t),
        _ => panic!("not a curved body"),
    }
}

/// The one huge curve each case places through `target` (media px): a radius of 20 000 px, or a
/// parabola 32 768 px deep, positioned so `target` lies on the true curve midway between two of
/// the vertices of a 64-chord tessellation (`t` = 32.5 / 64, or 15.5 / 64 of the ellipse's turn),
/// where those chords stray 8 to 24 px from it.
fn huge_curve(chart: &ChartEngine, kind: DrawingKind, target: Point) -> Vec<DrawingPoint> {
    const RADIUS: f64 = 20_000.0;
    let polar = |center: Point, angle: f64| {
        (
            center.0 + RADIUS * angle.cos(),
            center.1 + RADIUS * angle.sin(),
        )
    };
    let points: Vec<Point> = match kind {
        // The bottom of a circle centered above the pane.
        DrawingKind::Circle => vec![(target.0, target.1 - RADIUS), target],
        DrawingKind::Ellipse => {
            let angle = std::f64::consts::TAU * 15.5 / 64.0;
            let center = (
                target.0 - RADIUS * angle.cos(),
                target.1 - RADIUS * angle.sin(),
            );
            vec![
                (center.0 - RADIUS, center.1 - RADIUS),
                (center.0 + RADIUS, center.1 + RADIUS),
            ]
        }
        // A 6-radian arc whose t = 32.5 / 64 is the circle's bottom: start, through, end.
        DrawingKind::Arc => {
            let center = (target.0, target.1 - RADIUS);
            let through = std::f64::consts::FRAC_PI_2 - 3.0 / 64.0;
            vec![
                polar(center, through - 3.0),
                polar(center, through),
                polar(center, through + 3.0),
            ]
        }
        // y = 4·depth·t(1 − t) downward over x = width·(2t − 1), shifted so t = 32.5 / 64 is
        // `target`; the cubic is the same parabola raised to degree three.
        DrawingKind::Curve | DrawingKind::DoubleCurve => {
            let (width, depth) = (100_000.0, 32_768.0);
            let t: f64 = 32.5 / 64.0;
            let base = (
                target.0 - width * (2.0 * t - 1.0),
                target.1 + 4.0 * depth * t * (1.0 - t),
            );
            let start = (base.0 - width, base.1);
            let end = (base.0 + width, base.1);
            let control = (base.0, base.1 - 2.0 * depth);
            if kind == DrawingKind::Curve {
                vec![start, control, end]
            } else {
                let toward = |from: Point| {
                    (
                        from.0 + (control.0 - from.0) * 2.0 / 3.0,
                        from.1 + (control.1 - from.1) * 2.0 / 3.0,
                    )
                };
                vec![start, toward(start), toward(end), end]
            }
        }
        _ => unreachable!(),
    };
    points.into_iter().map(|point| at(chart, point)).collect()
}

/// Huge zoomed-in curves (a 20 000 px radius, a parabola 32 768 px deep) stay within a quarter
/// device pixel of the true curve on screen with bounded work, at every device-pixel ratio, and
/// hit on the true curve where a uniform 64-chord tessellation strays several pixels from it.
#[test]
fn huge_zoomed_curves_stay_within_a_quarter_pixel_on_screen_with_bounded_work() {
    for dpr in [1.0, 1.5, 2.0] {
        for kind in [
            DrawingKind::Circle,
            DrawingKind::Ellipse,
            DrawingKind::Arc,
            DrawingKind::Curve,
            DrawingKind::DoubleCurve,
        ] {
            let mut chart = deep_chart(800.0, dpr);
            let target = (400.0, 250.0);
            let points = huge_curve(&chart, kind, target);
            let id = add(
                &mut chart,
                kind,
                points,
                r##"{"color":"#123456","fill_enabled":false}"##,
            );
            let label = format!("{kind:?} at dpr {dpr}");
            let media = body_at(&chart, id, (1.0, 1.0));
            let on_curve = (0..=64_000)
                .map(|step| curve_point(media, f64::from(step) / 64_000.0))
                .map(|point| (point.0 - target.0).hypot(point.1 - target.1))
                .fold(f64::INFINITY, f64::min);
            assert!(on_curve < 1.0, "{label}: the target is on the curve");

            // Bounded work: one stroke of at most MAX_FLATTEN_POINTS + 1 points.
            let strokes = polylines(&mut chart, ink());
            assert_eq!(strokes.len(), 1, "{label}");
            let stroke = &strokes[0].0;
            assert!(
                stroke.len() <= MAX_FLATTEN_POINTS + 1,
                "{label}: {}",
                stroke.len()
            );

            // Every true-curve point on screen lies within 0.25 device px (plus the f32 storage
            // of the frame's points) of the painted chords.
            let (hpr, vpr) = ratios(&chart, dpr);
            let pane = &chart.panes[0];
            let screen = Rect {
                left: 0.0,
                top: pane.top * vpr,
                right: chart.pane_w * hpr,
                bottom: (pane.top + pane.height) * vpr,
            };
            let near = screen.inflate(4.0);
            let chords = stroke
                .windows(2)
                .filter(|pair| Rect::bounding(pair).is_some_and(|bounds| bounds.intersects(&near)))
                .collect::<Vec<_>>();
            let bitmap = body_at(&chart, id, (hpr, vpr));
            let inner = screen.inflate(-2.0);
            let mut visible = 0;
            for step in 0..=400_000 {
                let point = curve_point(bitmap, f64::from(step) / 400_000.0);
                if !inner.contains(point) {
                    continue;
                }
                visible += 1;
                let deviation = chords
                    .iter()
                    .map(|pair| shape::distance_to_segment(point, pair[0], pair[1]))
                    .fold(f64::INFINITY, f64::min);
                assert!(deviation <= 0.25 + 2e-3, "{label}: {deviation} px");
            }
            assert!(
                visible > 100,
                "{label}: the curve crosses the pane ({visible})"
            );

            // The true curve hits where the 64-chord polygon strays beyond the hit tolerance.
            let tolerance = chart.drawing(id).unwrap().width / 2.0 + 3.0;
            let chord = match (kind, media) {
                (DrawingKind::Ellipse, DrawingBodyGeometry::Ellipse { .. }) => {
                    [15.0 / 64.0, 16.0 / 64.0].map(|t| curve_point(media, t))
                }
                (DrawingKind::Circle, _) => [target, target],
                _ => [32.0 / 64.0, 33.0 / 64.0].map(|t| curve_point(media, t)),
            };
            if kind != DrawingKind::Circle {
                let stray = shape::distance_to_segment(target, chord[0], chord[1]);
                assert!(
                    stray > tolerance + 2.0,
                    "{label}: the 64-gon strays {stray} px"
                );
            }
            assert_eq!(hit(&chart, target), Some(id), "{label}");
            assert_eq!(
                chart.hit_test_drawing(target.0, target.1),
                chart.hit_test_drawing_bruteforce(target.0, target.1),
                "{label}"
            );
        }
    }
}

/// A curve that is a frame candidate but whose stroke lies wholly off screen paints nothing, at
/// every device-pixel ratio: the left side of a huge circle around the pane's middle (an arc), a
/// quadratic curve above the pane whose control point reaches below it, and an ellipse and a circle
/// just left of the pane (inside the candidate pad, outside the curve clip).
#[test]
fn off_screen_curves_paint_nothing() {
    for (width, dpr) in [(800.0, 1.0), (801.0, 1.5), (800.0, 2.0)] {
        let mut chart = deep_chart(width, dpr);
        let center = (400.0, 250.0);
        let polar = |angle: f64| {
            (
                center.0 + 20_000.0 * angle.cos(),
                center.1 + 20_000.0 * angle.sin(),
            )
        };
        let left_side = [
            polar(std::f64::consts::PI - 0.3),
            polar(std::f64::consts::PI),
            polar(std::f64::consts::PI + 0.3),
        ]
        .map(|point| at(&chart, point))
        .to_vec();
        // The candidate pad (22 px) reaches past the curve clip (16.5 px) beside the pane. `at`'s
        // anchor lands half a bar left of `x`; `exact` puts it on `x`, as the loop checks.
        let gap = -19.5;
        let exact = |point: Point| {
            let anchor = at(&chart, point);
            p(anchor.logical + 0.5, anchor.price)
        };
        let cases = [
            (DrawingKind::Arc, left_side),
            // Peaks at y = -500: the curve stays above the pane.
            (
                DrawingKind::Curve,
                [(-200.0, -3_000.0), (400.0, 2_000.0), (1_000.0, -3_000.0)]
                    .map(|point| at(&chart, point))
                    .to_vec(),
            ),
            (
                DrawingKind::Ellipse,
                [(gap - 200.0, 100.0), (gap, 300.0)].map(exact).to_vec(),
            ),
            (
                DrawingKind::Circle,
                [(gap - 60.0, 250.0), (gap, 250.0)].map(exact).to_vec(),
            ),
        ];
        for (kind, points) in cases {
            let id = add(&mut chart, kind, points, r##"{"color":"#123456"}"##);
            if matches!(kind, DrawingKind::Ellipse | DrawingKind::Circle) {
                assert!((anchor(&chart, id, 1).0 - gap).abs() < 1e-3, "{kind:?}");
            }
            let candidates = chart.take_drawing_candidates(0, None);
            assert!(
                candidates.contains(&id),
                "{kind:?} at dpr {dpr} is a candidate"
            );
            chart.recycle_drawing_candidates(candidates);
            assert!(
                polylines(&mut chart, ink()).is_empty(),
                "{kind:?} at dpr {dpr} paints nothing"
            );
            chart.remove_drawing(id);
        }
    }
}

/// The curved outlines resolve in the anchors' bitmap space at every device-pixel ratio (801 ×
/// 1.5 rounds the bitmap width apart from the height ratio): the circle's outline starts on its
/// rim anchor and every point sits on the scaled radius.
#[test]
fn curved_outlines_scale_with_the_device_pixel_ratio() {
    for (width, dpr) in [(800.0, 1.0), (801.0, 1.5), (800.0, 2.0)] {
        let mut chart = deep_chart(width, dpr);
        let id = add(
            &mut chart,
            DrawingKind::Circle,
            vec![p(15.0, 10_003.0), p(19.0, 10_003.0)],
            r##"{"color":"#123456"}"##,
        );
        let (hpr, vpr) = ratios(&chart, dpr);
        let (center, rim) = (anchor(&chart, id, 0), anchor(&chart, id, 1));
        let (ring, stroke_width) = polylines(&mut chart, ink()).remove(0);
        let width = chart.drawing(id).unwrap().width;
        assert!((f64::from(stroke_width) - width * vpr).abs() < 1e-5);
        let rim_px = (rim.0 * hpr, rim.1 * vpr);
        assert!(
            (ring[0].0 - rim_px.0).abs() < 1e-3 && (ring[0].1 - rim_px.1).abs() < 1e-3,
            "dpr {dpr}: {:?} vs {rim_px:?}",
            ring[0]
        );
        let center_px = (center.0 * hpr, center.1 * vpr);
        let radius = (rim_px.0 - center_px.0).hypot(rim_px.1 - center_px.1);
        assert!(ring.iter().all(|point| {
            ((point.0 - center_px.0).hypot(point.1 - center_px.1) - radius).abs() < 1e-3
        }));
        // A whole on-screen circle takes the uniform chords within a quarter pixel.
        let half_step = std::f64::consts::PI / (ring.len() - 1) as f64;
        assert!(radius * (1.0 - half_step.cos()) <= 0.25 + 1e-9, "dpr {dpr}");
    }
}

/// Dashed and dotted curved outlines reach every executor as solid dash runs clipped to the pane,
/// like the straight-edged shapes.
#[test]
fn dashed_curved_outlines_lower_to_solid_runs() {
    for kind in [
        DrawingKind::Ellipse,
        DrawingKind::Circle,
        DrawingKind::Arc,
        DrawingKind::Curve,
        DrawingKind::DoubleCurve,
    ] {
        let mut chart = chart();
        add(
            &mut chart,
            kind,
            points_for(kind),
            r##"{"color":"#123456","style":"dashed","fill_enabled":true}"##,
        );
        let frame = chart.build_frame();
        let styles = frame.panes[0]
            .main
            .iter()
            .filter_map(|prim| match prim {
                Prim::Polyline { style, color, .. } if *color == ink() => Some(*style),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(styles.len() > 4, "{kind:?}: {} dash runs", styles.len());
        assert!(
            styles.iter().all(|style| *style == LineStyle::Solid),
            "{kind:?}"
        );
    }
}
