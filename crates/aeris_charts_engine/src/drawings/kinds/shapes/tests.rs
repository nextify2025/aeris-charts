//! Shapes-family engine tests: catalog defaults, armed placement for every placement class,
//! shared-part frames and fills, hit testing (indexed and brute force), drags, keyboard nudges,
//! magnet, time identity, schema and kind options, patches with history, persistence, clipboard,
//! sync, device-pixel ratios, and bounded work at extreme zoom.

use aeris_charts_render::color::Color;
use aeris_charts_render::draw_list::{LineStyle, Prim};
use aeris_charts_render::shape::{self, MAX_FLATTEN_POINTS, Point, Rect};

use super::super::super::DrawingTextLayout;
use super::ShapeToolOptions;
use crate::drawings::geometry::CURVE_TOLERANCE;
use crate::drawings::{DrawingBodyGeometry, DrawingGeometryOptions, resolve_drawing_geometry};
use crate::{
    ChartEngine, ChartFocusTarget, ChartKey, DrawingAnchor, DrawingDragPart, DrawingId,
    DrawingKind, DrawingMagnetMode, DrawingModifiers, DrawingPoint, InputModifiers,
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
    assert!(
        runs.iter()
            .all(|(_, style)| *style == aeris_charts_render::draw_list::LineStyle::Solid)
    );
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
        assert!(
            chart
                .add_drawing(
                    kind,
                    0,
                    vec![point; count],
                    Some(r#"{"fill_enabled":true}"#)
                )
                .is_some()
        );
    }
    let frame = chart.build_frame();
    assert!(
        frame.panes[0]
            .points
            .iter()
            .all(|point| point[0].is_finite() && point[1].is_finite())
    );
    assert!(
        chart
            .add_drawing(
                DrawingKind::Circle,
                0,
                vec![p(f64::NAN, 1.0), p(2.0, 3.0)],
                None
            )
            .is_none()
    );
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
        // `target`; the cubic is the same parabola raised to degree three. The anchors are the
        // curve's points: its ends and its points at t = 1/2 (at 1/3 and 2/3 for the cubic).
        DrawingKind::Curve | DrawingKind::DoubleCurve => {
            let (width, depth) = (100_000.0, 32_768.0);
            let t: f64 = 32.5 / 64.0;
            let base = (
                target.0 - width * (2.0 * t - 1.0),
                target.1 + 4.0 * depth * t * (1.0 - t),
            );
            let start = (base.0 - width, base.1);
            let end = (base.0 + width, base.1);
            let on_parabola = |t: f64| {
                (
                    base.0 + width * (2.0 * t - 1.0),
                    base.1 - 4.0 * depth * t * (1.0 - t),
                )
            };
            if kind == DrawingKind::Curve {
                vec![start, on_parabola(0.5), end]
            } else {
                vec![start, on_parabola(1.0 / 3.0), on_parabola(2.0 / 3.0), end]
            }
        }
        _ => unreachable!(),
    };
    points.into_iter().map(|point| at(chart, point)).collect()
}

/// Huge zoomed-in curves (a 20 000 px radius, a parabola 32 768 px deep) stay within a tenth of
/// a device pixel of the true curve on screen with bounded work, at every device-pixel ratio, and
/// hit on the true curve where a uniform 64-chord tessellation strays several pixels from it.
#[test]
fn huge_zoomed_curves_stay_within_a_tenth_of_a_pixel_on_screen_with_bounded_work() {
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

            // Every true-curve point on screen lies within 0.1 device px (plus the f32 storage
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
                assert!(
                    deviation <= CURVE_TOLERANCE + 2e-3,
                    "{label}: {deviation} px"
                );
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
            // Peaks at y = -500: the curve stays above the pane (its derived control point at
            // y = 2,000 reaches below it).
            (
                DrawingKind::Curve,
                [(-200.0, -3_000.0), (400.0, -500.0), (1_000.0, -3_000.0)]
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

/// A curve passes through its anchors and bulges past their box (it stays inside the box of the
/// Bézier controls it derives from them): with every anchor just off the pane, the bulge that
/// swings into it still paints and hits, through the candidate index. A double curve on a linear
/// scale bulges down from anchors above the pane; a quadratic on a log scale spanning three
/// decades rises from anchors below it, where only the ln-space controls reach the bulge (the
/// linear ones stop about 20 px short of it, past the candidate pad).
#[test]
fn curve_bulges_past_their_anchors_paint_and_hit_with_the_anchors_off_screen() {
    for log in [false, true] {
        let mut chart = if log {
            let times = hourly(40);
            let values = (0..times.len())
                .map(|index| 10f64.powf(4.0 + index as f64 * 3.0 / 39.0))
                .collect::<Vec<_>>();
            let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
            chart
                .set_series_data(0, &times, &values, &values, &values, &values)
                .unwrap();
            chart.time_scale.set_width(800.0);
            chart.set_price_scale_mode_for(
                0,
                crate::PriceScaleTarget::Right,
                crate::PriceScaleMode::Logarithmic,
            );
            chart.fit_content();
            chart.build_frame();
            chart
        } else {
            chart()
        };
        let bottom = chart.panes[0].top + chart.panes[0].height;
        let (kind, anchors): (DrawingKind, Vec<Point>) = if log {
            (
                DrawingKind::Curve,
                vec![
                    (200.0, bottom + 500.0),
                    (260.0, bottom + 2.0),
                    (500.0, bottom + 2.0),
                ],
            )
        } else {
            (
                DrawingKind::DoubleCurve,
                vec![(200.0, -4.0), (260.0, -1.0), (320.0, -200.0), (380.0, -4.0)],
            )
        };
        let points = anchors.iter().map(|&point| at(&chart, point)).collect();
        let id = add(&mut chart, kind, points, r##"{"color":"#123456"}"##);
        // How far into the pane a point is, from the side the anchors sit beyond.
        let depth = |point: Point| if log { bottom - point.1 } else { point.1 };
        for index in 0..anchors.len() {
            assert!(
                depth(anchor(&chart, id, index)) < 0.0,
                "{kind:?}: anchors off the pane"
            );
        }
        let candidates = chart.take_drawing_candidates(0, None);
        assert!(candidates.contains(&id), "{kind:?} is a candidate");
        chart.recycle_drawing_candidates(candidates);
        let line = ink_line(&mut chart);
        let bulge = line
            .iter()
            .copied()
            .max_by(|a, b| depth(*a).total_cmp(&depth(*b)))
            .unwrap();
        assert!(
            depth(bulge) > 40.0,
            "{kind:?} swings into the pane: {bulge:?}"
        );
        assert_eq!(hit(&chart, bulge), Some(id), "{kind:?}");
        assert_eq!(
            chart
                .hit_test_drawing_bruteforce(bulge.0, bulge.1)
                .map(|hit| hit.id),
            Some(id),
            "{kind:?}"
        );
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
        // A whole on-screen circle takes the uniform chords within a tenth of a pixel.
        let half_step = std::f64::consts::PI / (ring.len() - 1) as f64;
        assert!(
            radius * (1.0 - half_step.cos()) <= CURVE_TOLERANCE + 1e-9,
            "dpr {dpr}"
        );
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

// --- Re-applied fork features (R8): chord fills, tangent extension, caps, closed polylines,
// --- close-on-click, ellipse bounds handles, rotated-rectangle width handles, on-curve editing,
// --- and seamless outlines.

fn wash() -> Color {
    Color::rgba(0x12, 0x34, 0x56, 51)
}

/// Media px of `point` on the first pane's right price scale.
fn px(chart: &ChartEngine, point: DrawingPoint) -> Point {
    chart
        .drawing_to_px_for(0, crate::DrawingPriceScale::Right, point)
        .unwrap()
}

/// The one ink stroke on the first pane.
fn ink_line(chart: &mut ChartEngine) -> Vec<Point> {
    let mut lines = polylines(chart, ink());
    assert_eq!(lines.len(), 1, "one ink stroke");
    lines.remove(0).0
}

/// Every band fill of `color` on the first pane as (upper, lower) chains.
fn fills(chart: &mut ChartEngine, color: Color) -> Vec<(Vec<Point>, Vec<Point>)> {
    let frame = chart.build_frame();
    let pane = &frame.panes[0];
    pane.main
        .iter()
        .filter_map(|prim| match prim {
            Prim::BandFill {
                upper_first,
                lower_first,
                point_count,
                fill,
                ..
            } if *fill == color => Some((
                pool(&pane.points, *upper_first, *point_count),
                pool(&pane.points, *lower_first, *point_count),
            )),
            _ => None,
        })
        .collect()
}

fn close(a: Point, b: Point, tolerance: f64) -> bool {
    (a.0 - b.0).abs() <= tolerance && (a.1 - b.1).abs() <= tolerance
}

fn part_at(chart: &ChartEngine, point: Point) -> Option<DrawingDragPart> {
    chart.hit_test_drawing(point.0, point.1).map(|hit| hit.part)
}

/// Anchor handles in the frame: the bordered discs at the handle radius.
fn handle_centers(chart: &mut ChartEngine) -> Vec<Point> {
    let frame = chart.build_frame();
    frame.panes[0]
        .main
        .iter()
        .filter_map(|prim| match prim {
            Prim::Circle { cx, cy, radius, .. } if (*radius - 5.5).abs() < 1e-3 => {
                Some((f64::from(*cx), f64::from(*cy)))
            }
            _ => None,
        })
        .collect()
}

/// The ink arrowheads (`Prim::BandFill` in the stroke color, a triangle's convex ribbon) on the
/// first pane, each as its tip (the first point) and its base's midpoint (the second point of
/// each chain).
fn arrowheads(chart: &mut ChartEngine) -> Vec<(Point, Point)> {
    fills(chart, ink())
        .into_iter()
        .map(|(upper, lower)| (upper[0], shape::midpoint(upper[1], lower[1])))
        .collect()
}

/// An arc fills the circular segment between it and its chord, and a curve the region between it
/// and its chord (a cubic crossing its chord in both lobes), in the drawing's fill color under the
/// stroke; the region is a body target only while the drawing is selected.
#[test]
fn arcs_and_curves_fill_their_chord_regions() {
    let mut chart = chart();
    // Upstream's arc anchors: its start, a point it passes through, its end.
    let id = add(
        &mut chart,
        DrawingKind::Arc,
        vec![p(10.0, 101.0), p(15.0, 104.0), p(20.0, 101.0)],
        r##"{"color":"#123456","fill_enabled":true}"##,
    );
    let segment = fills(&mut chart, wash());
    assert_eq!(segment.len(), 1, "the circular segment fills");
    let (inside, beyond_chord) = (px(&chart, p(15.0, 102.5)), px(&chart, p(15.0, 100.0)));
    assert!(shape::point_in_ribbon(inside, &segment[0].0, &segment[0].1));
    assert!(!shape::point_in_ribbon(
        beyond_chord,
        &segment[0].0,
        &segment[0].1
    ));
    // The stroke paints over its fill.
    let frame = chart.build_frame();
    let position = |wanted: fn(&Prim) -> bool| frame.panes[0].main.iter().position(wanted);
    let fill_at = position(|prim| matches!(prim, Prim::BandFill { fill, .. } if *fill == wash()));
    let stroke_at =
        position(|prim| matches!(prim, Prim::Polyline { color, .. } if *color == ink()));
    assert!(fill_at.unwrap() < stroke_at.unwrap());
    assert_eq!(hit(&chart, inside), None, "unselected, the interior pans");
    chart.set_selected_drawing(Some(id));
    assert_eq!(hit(&chart, inside), Some(id));
    assert_eq!(hit(&chart, beyond_chord), None);
    assert!(chart.drawing_apply_options(id, r#"{"fill_color":"rgba(255, 0, 0, 0.5)"}"#));
    assert!(fills(&mut chart, wash()).is_empty());
    assert_eq!(fills(&mut chart, Color::rgba(255, 0, 0, 128)).len(), 1);
    assert!(chart.drawing_apply_options(id, r#"{"fill_enabled":false}"#));
    assert!(fills(&mut chart, Color::rgba(255, 0, 0, 128)).is_empty());
    assert_eq!(hit(&chart, inside), None, "an unfilled interior never hits");
    chart.remove_drawing(id);

    // A quadratic and its chord bound a convex region.
    let id = add(
        &mut chart,
        DrawingKind::Curve,
        vec![p(10.0, 101.0), p(15.0, 105.0), p(20.0, 101.0)],
        r##"{"color":"#123456","fill_enabled":true}"##,
    );
    let region = fills(&mut chart, wash());
    assert_eq!(region.len(), 1);
    assert!(shape::point_in_ribbon(
        px(&chart, p(15.0, 102.0)),
        &region[0].0,
        &region[0].1
    ));
    chart.remove_drawing(id);

    // An S-shaped cubic crosses its chord: both lobes fill (nonzero rule) and hit while selected.
    let id = add(
        &mut chart,
        DrawingKind::DoubleCurve,
        vec![
            p(10.0, 102.0),
            p(14.0, 106.0),
            p(18.0, 98.0),
            p(22.0, 102.0),
        ],
        r##"{"color":"#123456","fill_enabled":true}"##,
    );
    assert_eq!(fills(&mut chart, wash()).len(), 1);
    let DrawingBodyGeometry::Curve(curve) = body_at(&chart, id, (1.0, 1.0)) else {
        panic!("a curve body");
    };
    let [start, end] = curve.ends();
    let chord = |t: f64| {
        (
            start.0 + (end.0 - start.0) * t,
            start.1 + (end.1 - start.1) * t,
        )
    };
    let lobe = |t: f64| shape::midpoint(curve.point(t), chord(t));
    let mirrored = (
        2.0 * chord(0.25).0 - lobe(0.25).0,
        2.0 * chord(0.25).1 - lobe(0.25).1,
    );
    assert_eq!(hit(&chart, lobe(0.25)), None, "unselected, the lobes pan");
    chart.set_selected_drawing(Some(id));
    assert_eq!(hit(&chart, lobe(0.25)), Some(id), "the upper lobe");
    assert_eq!(hit(&chart, lobe(0.75)), Some(id), "the lower lobe");
    assert_eq!(hit(&chart, mirrored), None, "beyond the chord");
}

/// `extend_right` continues a curve's end tangent (toward its derived control point) to the pane
/// edge and `extend_left` its start's; the extension is stroke and body target, never fill or
/// cap.
#[test]
fn curves_extend_along_their_end_tangents() {
    let mut chart = chart();
    let pane = &chart.panes[0];
    let (pane_top, pane_bottom) = (pane.top, pane.top + pane.height);
    let on_edge = |point: Point, chart: &ChartEngine| {
        point.0.abs() < 0.5
            || (point.0 - chart.pane_w).abs() < 0.5
            || (point.1 - pane_top).abs() < 0.5
            || (point.1 - pane_bottom).abs() < 0.5
    };
    let collinear = |a: Point, b: Point, c: Point| {
        let cross = (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0);
        cross.abs() / (c.0 - a.0).hypot(c.1 - a.1) < 1e-2
    };
    let id = add(
        &mut chart,
        DrawingKind::Curve,
        vec![p(10.0, 101.0), p(15.0, 105.0), p(20.0, 101.0)],
        r##"{"color":"#123456","extend_right":true,"fill_enabled":true}"##,
    );
    let (a, b) = (anchor(&chart, id, 0), anchor(&chart, id, 2));
    let DrawingBodyGeometry::Curve(curve) = body_at(&chart, id, (1.0, 1.0)) else {
        panic!("a curve body");
    };
    let control = curve.points[1];
    let line = ink_line(&mut chart);
    let edge = *line.last().unwrap();
    assert!(close(line[0], a, 1e-3), "the start stays");
    assert!(on_edge(edge, &chart), "{edge:?} on the pane edge");
    assert!(collinear(control, b, edge));
    assert!((edge.0 - b.0) * (b.0 - control.0) > 0.0, "beyond the end");
    // The fill keeps to the curve and its chord.
    let region = fills(&mut chart, wash());
    assert!(
        region[0]
            .0
            .iter()
            .chain(&region[0].1)
            .all(|point| point.0 <= b.0 + 1e-3)
    );
    // The extension selects the drawing as its body.
    let on_extension = shape::midpoint(b, edge);
    let hit_part = chart.hit_test_drawing(on_extension.0, on_extension.1);
    assert_eq!(
        hit_part.map(|hit| (hit.id, hit.part)),
        Some((id, DrawingDragPart::Body))
    );
    // With caps, only the unextended start carries its arrowhead.
    assert!(chart.drawing_apply_options(
        id,
        r#"{"fill_enabled":false,"stroke_start":"arrow","stroke_end":"arrow"}"#
    ));
    let heads = arrowheads(&mut chart);
    assert_eq!(heads.len(), 1, "only the start carries its arrow");
    assert!(close(heads[0].0, a, 1e-3));
    let line = ink_line(&mut chart);
    assert!(on_edge(*line.last().unwrap(), &chart));
    chart.remove_drawing(id);

    // A double curve extends from its start away from its first control point.
    let id = add(
        &mut chart,
        DrawingKind::DoubleCurve,
        vec![
            p(10.0, 101.0),
            p(14.0, 105.0),
            p(18.0, 100.0),
            p(22.0, 101.0),
        ],
        r##"{"color":"#123456","extend_left":true}"##,
    );
    let start = anchor(&chart, id, 0);
    let DrawingBodyGeometry::Curve(curve) = body_at(&chart, id, (1.0, 1.0)) else {
        panic!("a curve body");
    };
    let first_control = curve.points[1];
    let line = ink_line(&mut chart);
    assert!(on_edge(line[0], &chart), "{:?} on the pane edge", line[0]);
    assert!(close(line[1], start, 1e-3));
    assert!(collinear(first_control, start, line[0]));
    assert!(close(*line.last().unwrap(), anchor(&chart, id, 3), 1e-3));
    // Without the flags nothing reaches past the anchors.
    assert!(chart.drawing_apply_options(id, r#"{"extend_left":false}"#));
    assert!(close(ink_line(&mut chart)[0], start, 1e-3));
}

/// `stroke_start` and `stroke_end` cap an arc's and a curve's ends along their exact end
/// tangents (owner decision S6): the arrow trims the stroke under it, and the caps are body
/// targets.
#[test]
fn arc_and_curve_end_caps_follow_their_exact_tangents() {
    let mut chart = chart();
    let id = add(
        &mut chart,
        DrawingKind::Arc,
        vec![p(10.0, 101.0), p(15.0, 104.0), p(20.0, 101.0)],
        r##"{"color":"#123456","width":6,"stroke_start":"circle","stroke_end":"arrow"}"##,
    );
    let (start, end) = (anchor(&chart, id, 0), anchor(&chart, id, 2));
    let DrawingBodyGeometry::Arc(arc) = body_at(&chart, id, (1.0, 1.0)) else {
        panic!("an arc body");
    };
    let heads = arrowheads(&mut chart);
    assert_eq!(heads.len(), 1);
    let (tip, base) = heads[0];
    assert!(close(tip, end, 1e-3), "{tip:?} at {end:?}");
    // The arrow's axis is the tangent: perpendicular to the radius at the end, into the arc.
    let axis = (base.0 - tip.0, base.1 - tip.1);
    let length = axis.0.hypot(axis.1);
    let radial = (end.0 - arc.center.0, end.1 - arc.center.1);
    let radial_length = radial.0.hypot(radial.1);
    assert!((axis.0 * radial.0 + axis.1 * radial.1).abs() / (length * radial_length) < 1e-4);
    assert!((length - 2.0 * super::super::super::parts::cap_radius(6.0)).abs() < 1e-3);
    let line = ink_line(&mut chart);
    let trimmed = shape::distance_to_polyline(end, &line);
    assert!(
        (trimmed - 6.0).abs() < 0.05,
        "trimmed by one width: {trimmed}"
    );
    let frame = chart.build_frame();
    assert!(frame.panes[0].main.iter().any(|prim| matches!(
        prim,
        Prim::Circle { cx, cy, fill, .. }
            if *fill == ink() && close((f64::from(*cx), f64::from(*cy)), start, 1e-3)
    )));
    // A point inside the arrowhead but beyond the stroke's reach (outside the circle).
    let (ux, uy) = (axis.0 / length, axis.1 / length);
    let (rx, ry) = (radial.0 / radial_length, radial.1 / radial_length);
    let probe = (tip.0 + ux * 18.0 + rx * 7.5, tip.1 + uy * 18.0 + ry * 7.5);
    assert!(shape::distance_to_polyline(probe, &line) > 6.0 / 2.0 + 3.0);
    assert_eq!(
        hit(&chart, probe),
        Some(id),
        "the arrowhead is a body target"
    );
    assert!(chart.drawing_apply_options(id, r#"{"stroke_end":"none"}"#));
    assert_eq!(hit(&chart, probe), None);
    assert!(arrowheads(&mut chart).is_empty());
    chart.remove_drawing(id);

    // A curve's end cap points along its end tangent, toward the last derived control point.
    for (kind, points) in [
        (
            DrawingKind::Curve,
            vec![p(10.0, 101.0), p(15.0, 105.0), p(20.0, 101.0)],
        ),
        (
            DrawingKind::DoubleCurve,
            vec![
                p(10.0, 101.0),
                p(14.0, 105.0),
                p(18.0, 100.0),
                p(22.0, 101.0),
            ],
        ),
    ] {
        let last = points.len() - 1;
        let id = add(
            &mut chart,
            kind,
            points,
            r##"{"color":"#123456","width":2,"stroke_end":"arrow"}"##,
        );
        let end = anchor(&chart, id, last);
        let DrawingBodyGeometry::Curve(curve) = body_at(&chart, id, (1.0, 1.0)) else {
            panic!("a curve body");
        };
        let control = curve.points[if curve.cubic { 2 } else { 1 }];
        let heads = arrowheads(&mut chart);
        assert_eq!(heads.len(), 1, "{kind:?}");
        let (tip, base) = heads[0];
        assert!(close(tip, end, 1e-3), "{kind:?}");
        let axis = (base.0 - tip.0, base.1 - tip.1);
        let toward = (control.0 - end.0, control.1 - end.1);
        let cross = (axis.0 * toward.1 - axis.1 * toward.0) / toward.0.hypot(toward.1);
        assert!(cross.abs() < 1e-3, "{kind:?}: {cross}");
        assert!(axis.0 * toward.0 + axis.1 * toward.1 > 0.0, "{kind:?}");
        chart.remove_drawing(id);
    }
}

/// A closed polyline joins its last vertex to its first as one outline run from mid-edge, fills
/// the enclosed region by the nonzero rule while `fill_enabled`, and hits inside only while
/// selected; opening it again removes both, in one undo step.
#[test]
fn closed_polylines_fill_by_the_nonzero_rule() {
    let mut chart = chart();
    // An L: the notch at the upper right is outside.
    let l_shape = vec![
        p(10.0, 101.0),
        p(20.0, 101.0),
        p(20.0, 103.0),
        p(15.0, 103.0),
        p(15.0, 105.0),
        p(10.0, 105.0),
    ];
    let id = add(
        &mut chart,
        DrawingKind::Polyline,
        l_shape.clone(),
        r##"{"color":"#123456","fill_enabled":true,"tool_options":{"shape":{"closed":true}}}"##,
    );
    let outline = ink_line(&mut chart);
    assert_eq!(
        outline.len(),
        8,
        "every vertex plus the mid-edge seam twice"
    );
    assert_eq!(outline[0], outline[7]);
    assert!(close(
        outline[0],
        shape::midpoint(px(&chart, l_shape[0]), px(&chart, l_shape[1])),
        1e-3
    ));
    assert_eq!(fills(&mut chart, wash()).len(), 1);
    let (lower_arm, upper_arm, notch) = (
        px(&chart, p(12.5, 102.0)),
        px(&chart, p(12.5, 104.0)),
        px(&chart, p(17.5, 104.0)),
    );
    assert_eq!(
        hit(&chart, lower_arm),
        None,
        "unselected, the interior pans"
    );
    chart.set_selected_drawing(Some(id));
    assert_eq!(hit(&chart, lower_arm), Some(id));
    assert_eq!(hit(&chart, upper_arm), Some(id));
    assert_eq!(hit(&chart, notch), None, "the notch");
    // The closing edge is a body target.
    chart.set_selected_drawing(None);
    let closing = shape::midpoint(px(&chart, l_shape[5]), px(&chart, l_shape[0]));
    assert_eq!(hit(&chart, closing), Some(id));
    // Opening it removes the fill and the closing edge.
    assert!(chart.drawing_apply_options(id, r#"{"tool_options":{"shape":{"closed":false}}}"#));
    assert!(fills(&mut chart, wash()).is_empty());
    assert_eq!(ink_line(&mut chart).len(), 6);
    assert_eq!(hit(&chart, closing), None);
    assert!(chart.undo_drawing());
    assert_eq!(fills(&mut chart, wash()).len(), 1);
    chart.remove_drawing(id);

    // A new polyline keeps upstream's unfilled default (owner decision S1): closed, it paints its
    // closed outline only.
    add(
        &mut chart,
        DrawingKind::Polyline,
        l_shape,
        r##"{"color":"#123456","tool_options":{"shape":{"closed":true}}}"##,
    );
    assert!(fills(&mut chart, wash()).is_empty());
    assert_eq!(ink_line(&mut chart).len(), 8);
}

/// `count` anchors of a convex polygon (an ellipse in logical and price space, so a convex one
/// on screen) around the middle of the test data.
fn convex_anchors(count: usize) -> Vec<DrawingPoint> {
    (0..count)
        .map(|index| {
            let angle = index as f64 * std::f64::consts::TAU / count as f64;
            p(20.0 + 8.0 * angle.cos(), 103.0 + 2.0 * angle.sin())
        })
        .collect()
}

/// The fill is bounded work: a closed polyline of more than `MAX_FILL_VERTICES` vertices paints
/// its outline only and has no interior target, while its stroke still selects it.
#[test]
fn closed_polylines_beyond_the_fill_vertex_bound_keep_only_their_outline() {
    let closed =
        r##"{"color":"#123456","fill_enabled":true,"tool_options":{"shape":{"closed":true}}}"##;
    let middle = p(20.0, 103.0);

    let mut within = chart();
    let count = shape::MAX_FILL_VERTICES;
    let id = add(
        &mut within,
        DrawingKind::Polyline,
        convex_anchors(count),
        closed,
    );
    assert_eq!(fills(&mut within, wash()).len(), 1);
    assert_eq!(ink_line(&mut within).len(), count + 2);
    within.set_selected_drawing(Some(id));
    assert_eq!(hit(&within, px(&within, middle)), Some(id));

    let mut beyond = chart();
    let count = shape::MAX_FILL_VERTICES + 1;
    let id = add(
        &mut beyond,
        DrawingKind::Polyline,
        convex_anchors(count),
        closed,
    );
    assert!(fills(&mut beyond, wash()).is_empty());
    assert_eq!(ink_line(&mut beyond).len(), count + 2, "the outline stays");
    beyond.set_selected_drawing(Some(id));
    assert_eq!(
        hit(&beyond, px(&beyond, middle)),
        None,
        "no interior target"
    );
    // Unselected, only the body answers: the stroke at a vertex and mid-edge selects it.
    let anchors = convex_anchors(count);
    let vertex = px(&beyond, anchors[0]);
    let mid_edge = shape::midpoint(px(&beyond, anchors[100]), px(&beyond, anchors[101]));
    beyond.set_selected_drawing(None);
    for point in [vertex, mid_edge] {
        let stroke = beyond
            .hit_test_drawing(point.0, point.1)
            .expect("the stroke still selects it");
        assert_eq!((stroke.id, stroke.part), (id, DrawingDragPart::Body));
    }
}

/// A closed polyline paints no caps, and keeps them stored for reopening it.
#[test]
fn closed_polylines_paint_no_caps_and_keep_them_stored() {
    let mut chart = chart();
    let id = add(
        &mut chart,
        DrawingKind::Polyline,
        vec![p(10.0, 101.0), p(14.0, 104.0), p(20.0, 102.0)],
        r##"{"color":"#123456","stroke_end":"arrow","stroke_start":"circle"}"##,
    );
    let has_caps = |chart: &mut ChartEngine| {
        let frame = chart.build_frame();
        frame.panes[0].main.iter().any(|prim| {
            matches!(prim, Prim::Circle { fill, .. } if *fill == ink())
                || matches!(prim, Prim::Triangle { color, .. } if *color == ink())
        })
    };
    assert!(has_caps(&mut chart), "an open polyline caps its ends");
    assert!(chart.drawing_apply_options(id, r#"{"tool_options":{"shape":{"closed":true}}}"#));
    assert!(!has_caps(&mut chart));
    assert_eq!(
        chart.drawing(id).unwrap().stroke_end,
        crate::DrawingLineCap::Arrow,
        "the cap setting is kept for reopening"
    );
}

/// The schema lists `tool_options.shape.closed` for the polyline only.
#[test]
fn schema_lists_the_closed_flag_for_polylines_only() {
    let rows = |kind| {
        crate::drawing_property_schema(kind)
            .properties
            .into_iter()
            .filter(|property| property.name.starts_with("tool_options.shape."))
            .map(|property| (property.name, property.default))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        rows(DrawingKind::Polyline),
        [(
            "tool_options.shape.closed".to_string(),
            serde_json::json!(false)
        )]
    );
    for kind in SHAPE_KINDS
        .into_iter()
        .chain([DrawingKind::Path, DrawingKind::PatternXabcd])
        .filter(|&kind| kind != DrawingKind::Polyline)
    {
        assert!(rows(kind).is_empty(), "{kind:?}");
    }
}

/// Clicking the first vertex of a polyline once three are placed commits it closed and disarms
/// the tool in one undo step (owner decision S2); hovering there closes the preview onto it.
#[test]
fn clicking_the_first_vertex_closes_a_polyline() {
    let mut chart = chart();
    let modifiers = DrawingModifiers::default();
    let vertices = [(200.0, 250.0), (300.0, 150.0), (380.0, 260.0)];
    let place = |chart: &mut ChartEngine, kind: DrawingKind| {
        assert!(chart.set_drawing_tool(
            Some(kind),
            Some(r##"{"color":"#123456","fill_enabled":true}"##),
            None
        ));
        for (x, y) in vertices {
            assert!(
                chart
                    .drawing_tool_activate(x, y, modifiers)
                    .created
                    .is_none()
            );
        }
    };
    let near_first = (vertices[0].0 + 3.0, vertices[0].1 - 2.0);

    place(&mut chart, DrawingKind::Polyline);
    chart.drawing_tool_pointer_move(near_first.0, near_first.1, modifiers, false);
    let first = chart.pending_drawing().unwrap().drawing.points[0];
    assert_eq!(chart.pending_drawing().unwrap().preview, Some(first));
    let id = chart
        .drawing_tool_activate(near_first.0, near_first.1, modifiers)
        .created
        .expect("the closing click commits");
    let drawing = chart.drawing(id).unwrap();
    assert_eq!(drawing.points.len(), 3);
    assert!(drawing.tool_options.shape.unwrap().closed);
    assert_eq!(chart.active_drawing_tool(), None);
    assert_eq!(chart.selected_drawing(), Some(id));
    assert_eq!(fills(&mut chart, wash()).len(), 1);
    assert!(chart.undo_drawing());
    assert!(chart.drawings().is_empty());

    // Two vertices enclose nothing: the first vertex is no closing target yet.
    assert!(chart.set_drawing_tool(Some(DrawingKind::Polyline), None, None));
    chart.drawing_tool_activate(vertices[0].0, vertices[0].1, modifiers);
    chart.drawing_tool_activate(vertices[1].0, vertices[1].1, modifiers);
    chart.drawing_tool_pointer_move(near_first.0, near_first.1, modifiers, false);
    let first = chart.pending_drawing().unwrap().drawing.points[0];
    assert_ne!(chart.pending_drawing().unwrap().preview, Some(first));
    chart.cancel_drawing_tool();

    // Enter still finishes open, and Escape still cancels.
    place(&mut chart, DrawingKind::Polyline);
    let id = chart.drawing_tool_finish().created.unwrap();
    assert!(
        !chart
            .drawing(id)
            .unwrap()
            .tool_options
            .shape
            .unwrap_or_default()
            .closed
    );
    place(&mut chart, DrawingKind::Polyline);
    chart.cancel_drawing_tool();
    assert_eq!(chart.drawings().len(), 1);

    // The path has no close: a click on its first vertex places another vertex.
    place(&mut chart, DrawingKind::Path);
    assert!(
        chart
            .drawing_tool_activate(near_first.0, near_first.1, modifiers)
            .created
            .is_none()
    );
    let id = chart.drawing_tool_finish().created.unwrap();
    assert_eq!(chart.drawing(id).unwrap().points.len(), 4);
}

/// A closed polyline template previews its fill while vertices are placed and commits closed.
#[test]
fn closed_polyline_templates_preview_their_fill() {
    let mut chart = chart();
    let modifiers = DrawingModifiers::default();
    assert!(chart.set_drawing_tool(
        Some(DrawingKind::Polyline),
        Some(
            r##"{"color":"#123456","fill_enabled":true,"tool_options":{"shape":{"closed":true}}}"##
        ),
        None
    ));
    chart.drawing_tool_activate(200.0, 250.0, modifiers);
    chart.drawing_tool_activate(300.0, 150.0, modifiers);
    chart.drawing_tool_pointer_move(380.0, 260.0, modifiers, false);
    assert_eq!(fills(&mut chart, wash()).len(), 1);
    chart.drawing_tool_activate(380.0, 260.0, modifiers);
    let id = chart.drawing_tool_finish().created.unwrap();
    let drawing = chart.drawing(id).unwrap();
    assert_eq!(drawing.points.len(), 3);
    assert!(drawing.tool_options.shape.unwrap().closed);
}

/// An ellipse edits with the rectangle's eight bounds handles of its box, and Shift squares the
/// box into a circle while placing and dragging (owner decision S3).
#[test]
fn ellipses_edit_with_the_rectangle_bounds_handles() {
    let mut chart = chart();
    let id = add(
        &mut chart,
        DrawingKind::Ellipse,
        points_for(DrawingKind::Ellipse),
        "{}",
    );
    assert_eq!(chart.drawing_handle_count(id), Some(8));
    chart.set_selected_drawing(Some(id));
    let before = chart.drawing(id).unwrap().points.clone();
    let (a, b) = (anchor(&chart, id, 0), anchor(&chart, id, 1));
    let bottom_right = (a.0.max(b.0), a.1.max(b.1));
    assert_eq!(
        part_at(&chart, bottom_right),
        Some(DrawingDragPart::Anchor(4))
    );
    assert!(chart.drawing_drag_start_at(bottom_right.0, bottom_right.1));
    chart.drawing_drag_to(
        bottom_right.0 + 30.0,
        bottom_right.1 + 5.0,
        DrawingModifiers {
            magnet: false,
            straighten: true,
        },
    );
    chart.drawing_drag_end();
    let (a2, b2) = (anchor(&chart, id, 0), anchor(&chart, id, 1));
    assert!(
        ((b2.0 - a2.0).abs() - (b2.1 - a2.1).abs()).abs() < 1e-3,
        "{a2:?} {b2:?}"
    );
    assert!(chart.undo_drawing());
    assert_eq!(chart.drawing(id).unwrap().points, before);
    // The top edge's midpoint moves the top edge only.
    let top = (a.0.min(b.0) + (a.0 - b.0).abs() / 2.0, a.1.min(b.1));
    assert_eq!(part_at(&chart, top), Some(DrawingDragPart::Anchor(1)));
    assert!(chart.drawing_drag_start_at(top.0, top.1));
    chart.drawing_drag_to(top.0 + 17.0, top.1 - 20.0, DrawingModifiers::default());
    chart.drawing_drag_end();
    let (a3, b3) = (anchor(&chart, id, 0), anchor(&chart, id, 1));
    assert!(close(
        (a3.0.min(b3.0), a3.0.max(b3.0)),
        (a.0.min(b.0), a.0.max(b.0)),
        1e-6
    ));
    assert!((a3.1.min(b3.1) - (top.1 - 20.0)).abs() < 1e-6);
    assert!((a3.1.max(b3.1) - a.1.max(b.1)).abs() < 1e-6);
    assert!(chart.undo_drawing());
    assert_eq!(chart.drawing(id).unwrap().points, before);

    // Shift on the second placement click squares the box.
    let modifiers = DrawingModifiers {
        magnet: false,
        straighten: true,
    };
    assert!(chart.set_drawing_tool(Some(DrawingKind::Ellipse), None, None));
    chart.drawing_tool_activate(200.0, 250.0, modifiers);
    let id = chart
        .drawing_tool_activate(330.0, 290.0, modifiers)
        .created
        .unwrap();
    // Both clicks land on their bar slots first; the square takes the snapped width.
    let (a, b) = (anchor(&chart, id, 0), anchor(&chart, id, 1));
    let slot_x = |x: f64| {
        chart
            .logical_to_coordinate(chart.coordinate_to_logical(x).unwrap())
            .unwrap()
    };
    assert!(close(a, (slot_x(200.0), 250.0), 1e-6), "{a:?}");
    assert!(((b.0 - a.0).abs() - (b.1 - a.1).abs()).abs() < 1e-3);
    assert!(((b.0 - a.0) - (slot_x(330.0) - slot_x(200.0))).abs() < 1e-3);
}

/// The rotated rectangle's on-screen depth: its depth point's distance from the edge's line.
fn depth(chart: &ChartEngine, id: DrawingId) -> f64 {
    let (a, b, c) = (
        anchor(chart, id, 0),
        anchor(chart, id, 1),
        anchor(chart, id, 2),
    );
    let normal = shape::segment_normal(a, b).unwrap();
    ((c.0 - a.0) * normal.0 + (c.1 - a.1) * normal.1).abs()
}

/// The rotated rectangle's third handle sits on its far side's midpoint (upstream's projection of
/// the depth anchor) and a width handle on the near side's; dragging either changes only the
/// width, and an edge corner drag keeps the on-screen width (owner decision S4, the width handles
/// only). The far handle drags its anchor bar by bar like every anchor; the near width handle
/// encodes a perpendicular distance and stays continuous.
#[test]
fn rotated_rectangle_width_handles_sit_on_its_long_sides() {
    let mut chart = chart();
    let id = add(
        &mut chart,
        DrawingKind::RotatedRectangle,
        points_for(DrawingKind::RotatedRectangle),
        "{}",
    );
    let before = chart.drawing(id).unwrap().points.clone();
    chart.set_selected_drawing(Some(id));
    let (a, b, c) = (
        anchor(&chart, id, 0),
        anchor(&chart, id, 1),
        anchor(&chart, id, 2),
    );
    let normal = shape::segment_normal(a, b).unwrap();
    let offset = (c.0 - a.0) * normal.0 + (c.1 - a.1) * normal.1;
    let middle = shape::midpoint(a, b);
    let far = (middle.0 + normal.0 * offset, middle.1 + normal.1 * offset);
    assert_eq!(chart.drawing_handle_count(id), Some(4));
    assert!(
        !close(c, far, 6.5),
        "the fixture's depth point is off the midpoint"
    );
    assert_eq!(part_at(&chart, far), Some(DrawingDragPart::Anchor(2)));
    assert_eq!(part_at(&chart, middle), Some(DrawingDragPart::Handle(0)));
    assert_ne!(part_at(&chart, c), Some(DrawingDragPart::Anchor(2)));

    // Pulling the far handle 15 px outward, with 9 px of jitter along the side, moves the depth
    // anchor by the pointer's bar steps from its own slot and by the raw vertical delta, in one
    // undo step, keeping the edge.
    let out = offset.signum();
    let delta = (
        normal.0 * 15.0 * out + normal.1 * 9.0,
        normal.1 * 15.0 * out - normal.0 * 9.0,
    );
    assert!(chart.drawing_drag_start_at(far.0, far.1));
    chart.drawing_drag_to(
        far.0 + delta.0,
        far.1 + delta.1,
        DrawingModifiers::default(),
    );
    chart.drawing_drag_end();
    let slot = |chart: &ChartEngine, x: f64| chart.coordinate_to_logical(x).unwrap();
    let steps = slot(&chart, far.0 + delta.0) - slot(&chart, far.0);
    assert_eq!(
        chart.drawing(id).unwrap().points[2].logical,
        before[2].logical + steps
    );
    let moved = (
        chart
            .logical_to_coordinate(before[2].logical + steps)
            .unwrap(),
        c.1 + delta.1,
    );
    let across = (moved.0 - a.0) * normal.0 + (moved.1 - a.1) * normal.1;
    assert!(close(anchor(&chart, id, 2), moved, 1e-6));
    assert!((depth(&chart, id) - across.abs()).abs() < 1e-6);
    assert!(close(anchor(&chart, id, 0), a, 1e-9) && close(anchor(&chart, id, 1), b, 1e-9));
    assert!(chart.undo_drawing());
    assert_eq!(chart.drawing(id).unwrap().points, before);

    // The near side's handle narrows the rectangle from its side by exactly the pointer's
    // perpendicular travel, 10 px, whatever its 13 px (over half a bar) of jitter along the side;
    // the far side stays.
    assert!(chart.drawing_drag_start_at(middle.0, middle.1));
    chart.drawing_drag_to(
        middle.0 + normal.0 * 10.0 * out + normal.1 * 13.0,
        middle.1 + normal.1 * 10.0 * out - normal.0 * 13.0,
        DrawingModifiers::default(),
    );
    chart.drawing_drag_end();
    assert!((depth(&chart, id) - (offset.abs() - 10.0)).abs() < 1e-3);
    let far_side = |chart: &ChartEngine| {
        let (a, b, c) = (
            anchor(chart, id, 0),
            anchor(chart, id, 1),
            anchor(chart, id, 2),
        );
        let normal = shape::segment_normal(a, b).unwrap();
        let offset = (c.0 - a.0) * normal.0 + (c.1 - a.1) * normal.1;
        (a.0 + normal.0 * offset, a.1 + normal.1 * offset)
    };
    assert!(close(
        far_side(&chart),
        (a.0 + normal.0 * offset, a.1 + normal.1 * offset),
        1e-3
    ));
    assert_eq!(chart.drawing(id).unwrap().points[2], before[2]);
    assert!(chart.undo_drawing());
    assert_eq!(chart.drawing(id).unwrap().points, before);

    // A (bar-snapped) quarter turn of the edge about its first corner keeps the width, also
    // through a zero-length edge on the way.
    let quarter = (a.0 - (b.1 - a.1), a.1 + (b.0 - a.0));
    assert!(chart.drawing_drag_start_at(b.0, b.1));
    chart.drawing_drag_to(a.0, a.1, DrawingModifiers::default());
    chart.drawing_drag_to(quarter.0, quarter.1, DrawingModifiers::default());
    chart.drawing_drag_end();
    let quarter = (
        chart
            .logical_to_coordinate(slot(&chart, quarter.0))
            .unwrap(),
        quarter.1,
    );
    assert!(close(anchor(&chart, id, 1), quarter, 1e-3));
    assert!((depth(&chart, id) - offset.abs()).abs() < 1e-3);
    assert!(chart.undo_drawing());
    assert_eq!(chart.drawing(id).unwrap().points, before);

    // Keyboard: the far handle (third) and the width handle (fourth) nudge only the width.
    assert!(chart.nudge_selected_drawing(0.0, -10.0, Some(2)));
    let widened = (offset + normal.1 * -10.0).abs();
    assert!((depth(&chart, id) - widened).abs() < 1e-3);
    assert!(close(anchor(&chart, id, 0), a, 1e-9));
    // The same step on the near side moves the edge with it: the width is back.
    assert!(chart.nudge_selected_drawing(0.0, -10.0, Some(3)));
    assert!((depth(&chart, id) - offset.abs()).abs() < 1e-3);
    let shifted = (
        a.0 - normal.0 * normal.1 * 10.0,
        a.1 - normal.1 * normal.1 * 10.0,
    );
    assert!(close(anchor(&chart, id, 0), shifted, 1e-3));
    assert!(chart.undo_drawing() && chart.undo_drawing());
    assert_eq!(chart.drawing(id).unwrap().points, before);

    // A horizontal step on the width handle stays continuous: one px (ten with Shift) moves the
    // edge by that px's part across it, never by a whole bar spacing.
    assert!(normal.0.abs() > 0.1 && chart.bar_spacing() > 2.0);
    let across = |dx: f64| (offset.abs() - dx * normal.0 * out).abs();
    assert!(chart.nudge_selected_drawing(1.0, 0.0, Some(3)));
    assert!((depth(&chart, id) - across(1.0)).abs() < 1e-3);
    assert!(chart.undo_drawing());
    assert_eq!(chart.drawing(id).unwrap().points, before);
    let press = |chart: &mut ChartEngine, key: ChartKey, shift: bool| {
        let modifiers = InputModifiers {
            shift,
            ..InputModifiers::default()
        };
        let target = ChartFocusTarget::Drawing(id);
        assert!(
            chart.input_target_key_down(target, key, modifiers),
            "{key:?}"
        );
    };
    press(&mut chart, ChartKey::Enter, false);
    for _ in 0..4 {
        press(&mut chart, ChartKey::Tab, false);
    }
    press(&mut chart, ChartKey::ArrowRight, true);
    press(&mut chart, ChartKey::Enter, false);
    assert!((depth(&chart, id) - across(10.0)).abs() < 1e-3);
    assert!(chart.undo_drawing());
    assert_eq!(chart.drawing(id).unwrap().points, before);

    // The magnet snaps the far handle like an anchor: the far side then passes through the bar
    // value under the pointer.
    chart.set_drawing_magnet_mode(DrawingMagnetMode::Strong);
    let to = (far.0 + 4.0, far.1 - 20.0);
    assert!(chart.drawing_drag_start_at(far.0, far.1));
    chart.drawing_drag_to(to.0, to.1, DrawingModifiers::default());
    chart.drawing_drag_end();
    chart.set_drawing_magnet_mode(DrawingMagnetMode::Off);
    let bar = chart
        .drawing_from_px(0, to.0, to.1)
        .unwrap()
        .logical
        .round();
    let snapped = px(&chart, p(bar, 100.0 + (bar as usize % 7) as f64));
    let (a, b, c) = (
        anchor(&chart, id, 0),
        anchor(&chart, id, 1),
        anchor(&chart, id, 2),
    );
    let normal = shape::segment_normal(a, b).unwrap();
    let across = |point: Point| (point.0 - a.0) * normal.0 + (point.1 - a.1) * normal.1;
    assert!(
        (across(snapped) - across(c)).abs() < 1e-3,
        "{snapped:?} {c:?}"
    );
}

/// The bar slot under media `x` (every anchor but anchored text lands on one), at `y`.
fn on_slot(chart: &ChartEngine, (x, y): Point) -> Point {
    let logical = chart.coordinate_to_logical(x).unwrap();
    (chart.logical_to_coordinate(logical).unwrap(), y)
}

/// Curves are placed and edited through points on the curve: a curve's anchors are its start,
/// the points it passes through (a curve's at t = 1/2, a double curve's at 1/3 and 2/3) and its
/// end (upstream's catalog contract, revision 3), and every anchor is a handle on the curve.
/// Placement keeps owner decision S5's click order: both ends first, then the points it passes
/// through. An arc is placed the same way.
#[test]
fn curves_place_and_edit_through_points_on_the_curve() {
    let mut chart = chart();
    let modifiers = DrawingModifiers::default();
    let place = |chart: &mut ChartEngine, kind, clicks: &[Point]| {
        assert!(chart.set_drawing_tool(Some(kind), Some(r##"{"color":"#123456"}"##), None));
        let mut created = None;
        for &(x, y) in clicks {
            created = chart.drawing_tool_activate(x, y, modifiers).created;
        }
        created.expect("committed")
    };
    let cases: [(DrawingKind, &[Point]); 3] = [
        (
            DrawingKind::Arc,
            &[(200.0, 250.0), (420.0, 260.0), (300.0, 150.0)],
        ),
        (
            DrawingKind::Curve,
            &[(200.0, 250.0), (420.0, 260.0), (300.0, 150.0)],
        ),
        (
            DrawingKind::DoubleCurve,
            &[
                (200.0, 250.0),
                (420.0, 260.0),
                (270.0, 150.0),
                (350.0, 320.0),
            ],
        ),
    ];
    for (kind, clicks) in cases {
        let id = place(&mut chart, kind, clicks);
        // Each click lands on its bar slot, at the raw price.
        let clicks = clicks
            .iter()
            .map(|&click| on_slot(&chart, click))
            .collect::<Vec<_>>();
        let line = ink_line(&mut chart);
        for &click in &clicks {
            let distance = shape::distance_to_polyline(click, &line);
            assert!(
                distance <= 0.11,
                "{kind:?} passes through {click:?} ({distance})"
            );
        }
        // Stored in curve order: start, the points it passes through, end.
        let last = clicks.len() - 1;
        let mut stored = vec![clicks[0]];
        stored.extend_from_slice(&clicks[2..]);
        stored.push(clicks[1]);
        for (index, &expected) in stored.iter().enumerate() {
            assert!(
                close(anchor(&chart, id, index), expected, 1e-6),
                "{kind:?} anchor {index} of {last}"
            );
        }
        chart.remove_drawing(id);
    }

    // A curve's handles: its anchors, all on the curve, never the derived control point.
    let id = add(
        &mut chart,
        DrawingKind::Curve,
        vec![p(10.0, 101.0), p(15.0, 105.0), p(20.0, 101.0)],
        r##"{"color":"#123456"}"##,
    );
    chart.set_selected_drawing(Some(id));
    let before = chart.drawing(id).unwrap().points.clone();
    let (a, middle, b) = (
        anchor(&chart, id, 0),
        anchor(&chart, id, 1),
        anchor(&chart, id, 2),
    );
    let control = (
        2.0 * middle.0 - (a.0 + b.0) / 2.0,
        2.0 * middle.1 - (a.1 + b.1) / 2.0,
    );
    assert_eq!(chart.drawing_handle_count(id), Some(3));
    assert!(shape::distance_to_polyline(middle, &ink_line(&mut chart)) <= 0.11);
    assert_eq!(part_at(&chart, middle), Some(DrawingDragPart::Anchor(1)));
    assert_eq!(
        part_at(&chart, control),
        None,
        "the control point is no handle"
    );
    // Dragging it moves that anchor alone, a whole bar for 25 px, and the curve passes through
    // it with its ends fixed, in one step.
    assert_eq!(chart.bar_spacing(), 20.0);
    let to = (middle.0 + 25.0, middle.1 - 30.0);
    assert!(chart.drawing_drag_start_at(middle.0, middle.1));
    chart.drawing_drag_to(to.0, to.1, DrawingModifiers::default());
    chart.drawing_drag_end();
    let moved = (middle.0 + 20.0, to.1);
    assert!(close(anchor(&chart, id, 1), moved, 1e-6));
    assert_eq!(chart.drawing(id).unwrap().points[1].logical, 16.0);
    assert!(shape::distance_to_polyline(moved, &ink_line(&mut chart)) <= 0.11);
    assert!(close(anchor(&chart, id, 0), a, 1e-9) && close(anchor(&chart, id, 2), b, 1e-9));
    assert!(chart.undo_drawing());
    assert_eq!(chart.drawing(id).unwrap().points, before);
    // Dragging an end (two whole bars) keeps the point on the curve.
    assert!(chart.drawing_drag_start_at(a.0, a.1));
    chart.drawing_drag_to(a.0 - 40.0, a.1 + 20.0, DrawingModifiers::default());
    chart.drawing_drag_end();
    assert!(close(anchor(&chart, id, 0), (a.0 - 40.0, a.1 + 20.0), 1e-6));
    assert!(close(anchor(&chart, id, 1), middle, 1e-9));
    assert!(chart.undo_drawing());
    // Keyboard: the second handle is the on-curve anchor.
    assert!(chart.nudge_selected_drawing(0.0, -10.0, Some(1)));
    assert!(close(
        anchor(&chart, id, 1),
        (middle.0, middle.1 - 10.0),
        1e-6
    ));
    assert!(chart.undo_drawing());
    chart.remove_drawing(id);

    // A double curve's anchors at t = 1/3 and 2/3; dragging one keeps the other on the curve.
    let id = add(
        &mut chart,
        DrawingKind::DoubleCurve,
        vec![
            p(10.0, 101.0),
            p(14.0, 105.0),
            p(18.0, 100.0),
            p(22.0, 101.0),
        ],
        r##"{"color":"#123456"}"##,
    );
    chart.set_selected_drawing(Some(id));
    assert_eq!(chart.drawing_handle_count(id), Some(4));
    let DrawingBodyGeometry::Curve(curve) = body_at(&chart, id, (1.0, 1.0)) else {
        panic!("a curve body");
    };
    let (third, two_thirds) = (anchor(&chart, id, 1), anchor(&chart, id, 2));
    assert!(close(curve.point(1.0 / 3.0), third, 1e-6));
    assert!(close(curve.point(2.0 / 3.0), two_thirds, 1e-6));
    assert_eq!(part_at(&chart, third), Some(DrawingDragPart::Anchor(1)));
    assert_eq!(
        part_at(&chart, two_thirds),
        Some(DrawingDragPart::Anchor(2))
    );
    let end = anchor(&chart, id, 3);
    let to = (two_thirds.0 + 25.0, two_thirds.1 + 35.0);
    assert!(chart.drawing_drag_start_at(two_thirds.0, two_thirds.1));
    chart.drawing_drag_to(to.0, to.1, DrawingModifiers::default());
    chart.drawing_drag_end();
    let moved = (two_thirds.0 + 20.0, to.1);
    assert!(close(anchor(&chart, id, 2), moved, 1e-6));
    assert!(close(anchor(&chart, id, 1), third, 1e-9));
    assert!(close(anchor(&chart, id, 3), end, 1e-9));
    let line = ink_line(&mut chart);
    for point in [third, moved] {
        assert!(shape::distance_to_polyline(point, &line) <= 0.11);
    }
    assert!(chart.undo_drawing());
    // Keyboard: the second and third handles are the on-curve anchors, each nudge one undo step.
    let before = chart.drawing(id).unwrap().points.clone();
    let ends = (anchor(&chart, id, 0), anchor(&chart, id, 3));
    for (moved, kept) in [(1, 2), (2, 1)] {
        let (from, other) = (anchor(&chart, id, moved), anchor(&chart, id, kept));
        assert!(chart.nudge_selected_drawing(0.0, -10.0, Some(moved)));
        assert!(close(
            anchor(&chart, id, moved),
            (from.0, from.1 - 10.0),
            1e-6
        ));
        assert!(close(anchor(&chart, id, kept), other, 1e-9));
        assert!(close(anchor(&chart, id, 0), ends.0, 1e-9));
        assert!(close(anchor(&chart, id, 3), ends.1, 1e-9));
        assert!(chart.undo_drawing());
        assert_eq!(
            chart.drawing(id).unwrap().points,
            before,
            "keyboard {moved}"
        );
    }
}

/// While a through-point tool is placed, the preview resolves the anchors its clicks will store
/// (the curve bends through the pointer) and paints its handle discs on the clicks placed so far,
/// each on its bar slot.
#[test]
fn through_point_previews_bend_through_the_pointer_with_discs_on_the_clicks() {
    let mut chart = chart();
    let modifiers = DrawingModifiers::default();
    for kind in [
        DrawingKind::Arc,
        DrawingKind::Curve,
        DrawingKind::DoubleCurve,
    ] {
        assert!(chart.set_drawing_tool(Some(kind), Some(r##"{"color":"#123456"}"##), None));
        let slot = |chart: &ChartEngine, point| on_slot(chart, point);
        let (start, end) = (slot(&chart, (200.0, 250.0)), slot(&chart, (400.0, 250.0)));
        chart.drawing_tool_activate(200.0, 250.0, modifiers);
        // Until only the last click remains, a guide runs from the placed clicks to the pointer.
        chart.drawing_tool_pointer_move(400.0, 250.0, modifiers, false);
        let guide = ink_line(&mut chart);
        assert!(close(guide[0], start, 1e-3) && close(guide[1], end, 1e-3));
        chart.drawing_tool_activate(400.0, 250.0, modifiers);
        let mut placed = vec![start, end];
        if kind == DrawingKind::DoubleCurve {
            let through = slot(&chart, (260.0, 180.0));
            chart.drawing_tool_pointer_move(260.0, 180.0, modifiers, false);
            assert_eq!(
                ink_line(&mut chart),
                vec![start, end, through],
                "the guide through the ends and the pointer"
            );
            chart.drawing_tool_activate(260.0, 180.0, modifiers);
            placed.push(through);
        }
        let pointer = slot(&chart, (330.0, 300.0));
        chart.drawing_tool_pointer_move(330.0, 300.0, modifiers, false);
        let preview = ink_line(&mut chart);
        assert!(preview.len() > 8, "{kind:?} bends");
        for point in placed.iter().chain([&pointer]) {
            assert!(
                shape::distance_to_polyline(*point, &preview) < 0.11,
                "{kind:?} passes through {point:?}"
            );
        }
        assert_eq!(
            handle_centers(&mut chart),
            placed,
            "{kind:?} discs on the clicks"
        );
        assert!(chart.set_drawing_tool(None, None, None));
        assert!(polylines(&mut chart, ink()).is_empty());
    }
}

/// The rotated rectangle's and the triangle's outlines are one seamless run from mid-edge
/// (owner decision S7); the projection keeps upstream's triangle edges.
#[test]
fn rotated_rectangles_and_triangles_outline_as_one_seamless_run() {
    for (kind, runs, points) in [
        (DrawingKind::RotatedRectangle, 1, 6),
        (DrawingKind::Triangle, 1, 5),
    ] {
        let mut chart = chart();
        let id = add(
            &mut chart,
            kind,
            points_for(kind),
            r##"{"color":"#123456","width":3}"##,
        );
        let strokes = polylines(&mut chart, ink());
        assert_eq!(strokes.len(), runs, "{kind:?}");
        let outline = &strokes[0].0;
        assert_eq!(outline.len(), points, "{kind:?}");
        assert_eq!(outline[0], outline[points - 1]);
        let corner = anchor(&chart, id, 0);
        let next = anchor(&chart, id, 1);
        assert!(
            close(outline[0], shape::midpoint(corner, next), 1e-3),
            "{kind:?} starts mid-edge"
        );
    }
    let mut chart = chart();
    add(
        &mut chart,
        DrawingKind::Projection,
        vec![p(10.0, 101.0), p(20.0, 104.0)],
        r##"{"color":"#123456"}"##,
    );
    assert!(
        polylines(&mut chart, ink())
            .iter()
            .all(|(line, _)| line.len() == 2)
    );
}

/// The culled hit path agrees with brute force for the re-applied options: caps reaching past the
/// anchors, tangent extensions to the pane edge, chord fills and closed polylines, selected.
#[test]
fn indexed_hit_testing_matches_brute_force_with_the_shape_options() {
    let mut chart = chart();
    // Wide caps reach past the anchors' culling pad (the decoration extent covers them); the
    // other set extends, fills and closes.
    let option_sets = [
        r#"{"width":20,"stroke_start":"arrow","stroke_end":"circle"}"#,
        r#"{"width":5,"fill_enabled":true,"extend_left":true,"extend_right":true,"tool_options":{"shape":{"closed":true}}}"#,
    ];
    for copy in 0..3 {
        let shift = f64::from(copy) * 0.9;
        for kind in SHAPE_KINDS {
            let points = points_for(kind)
                .into_iter()
                .map(|point| p(point.logical + shift * 3.0, point.price + shift))
                .collect::<Vec<_>>();
            for options in option_sets {
                add(&mut chart, kind, points.clone(), options);
            }
        }
    }
    crowd(&mut chart);
    let ids = chart
        .drawings()
        .iter()
        .map(|drawing| drawing.id)
        .collect::<Vec<_>>();
    let mut hits = 0;
    // A fine grid unselected (the caps' reach), a coarser one with each filled arc, curve,
    // double curve and closed polyline selected.
    for (selected, step) in [
        (None, 7.0),
        (Some(ids[9]), 12.0),
        (Some(ids[11]), 12.0),
        (Some(ids[13]), 12.0),
        (Some(ids[15]), 12.0),
    ] {
        chart.set_selected_drawing(selected);
        chart.build_frame();
        for gy in 0..(480.0 / step) as u32 {
            for gx in 0..(780.0 / step) as u32 {
                let (x, y) = (f64::from(gx) * step + 3.0, f64::from(gy) * step + 4.0);
                let indexed = chart.hit_test_drawing(x, y);
                assert_eq!(
                    indexed,
                    chart.hit_test_drawing_bruteforce(x, y),
                    "({x}, {y}) with {selected:?} selected"
                );
                hits += usize::from(indexed.is_some());
            }
        }
    }
    assert!(hits > 400, "the grid meets the shapes ({hits} hits)");
}

/// The re-applied options resolve in bitmap px at every device-pixel ratio (801 × 1.5 rounds the
/// bitmap width apart from the height ratio): an arc's arrowhead sits on its end at the scaled
/// length, a curve's tangent extension reaches the bitmap pane edge, a closed polyline's outline
/// starts on its first edge's midpoint, and the chord fill stays on the arc's chord.
#[test]
fn shape_options_scale_with_the_device_pixel_ratio() {
    for (width, dpr) in [(800.0, 1.0), (801.0, 1.5), (800.0, 2.0)] {
        let mut chart = deep_chart(width, dpr);
        let (hpr, vpr) = ratios(&chart, dpr);
        let bitmap = |(x, y): Point| (x * hpr, y * vpr);

        let id = add(
            &mut chart,
            DrawingKind::Arc,
            vec![p(10.0, 10_001.0), p(15.0, 10_004.0), p(20.0, 10_001.0)],
            r##"{"color":"#123456","width":6,"stroke_end":"arrow","fill_enabled":true}"##,
        );
        let (start, end) = (bitmap(anchor(&chart, id, 0)), bitmap(anchor(&chart, id, 2)));
        let heads = arrowheads(&mut chart);
        assert_eq!(heads.len(), 1, "dpr {dpr}");
        let (tip, base) = heads[0];
        assert!(close(tip, end, 1e-3), "dpr {dpr}: {tip:?} at {end:?}");
        let length = (base.0 - tip.0).hypot(base.1 - tip.1);
        let expected = 2.0 * super::super::super::parts::cap_radius(6.0 * vpr);
        assert!((length - expected).abs() < 1e-3, "dpr {dpr}: {length}");
        let region = fills(&mut chart, wash());
        assert_eq!(region.len(), 1, "dpr {dpr}");
        let chord = [start, end];
        assert!(
            region[0]
                .0
                .iter()
                .chain(&region[0].1)
                .any(|&point| shape::distance_to_polyline(point, &chord) < 1e-2),
            "dpr {dpr}: the fill closes on the chord"
        );
        chart.remove_drawing(id);

        let id = add(
            &mut chart,
            DrawingKind::Curve,
            vec![p(10.0, 10_001.0), p(15.0, 10_005.0), p(20.0, 10_001.0)],
            r##"{"color":"#123456","extend_right":true}"##,
        );
        // The end tangent runs toward the derived control point (the anchors pass through it).
        let DrawingBodyGeometry::Curve(curve) = body_at(&chart, id, (hpr, vpr)) else {
            panic!("a curve body");
        };
        let (control, b) = (curve.points[1], bitmap(anchor(&chart, id, 2)));
        let edge = *ink_line(&mut chart).last().unwrap();
        let pane = &chart.panes[0];
        let (right, top, bottom) = (
            chart.pane_w * hpr,
            pane.top * vpr,
            (pane.top + pane.height) * vpr,
        );
        assert!(
            (edge.0 - right).abs() < 0.5
                || (edge.1 - top).abs() < 0.5
                || (edge.1 - bottom).abs() < 0.5,
            "dpr {dpr}: {edge:?} on the bitmap pane edge"
        );
        let cross =
            (b.0 - control.0) * (edge.1 - control.1) - (b.1 - control.1) * (edge.0 - control.0);
        assert!(
            cross.abs() / (edge.0 - control.0).hypot(edge.1 - control.1) < 1e-2,
            "dpr {dpr}: along the end tangent"
        );
        assert!((edge.0 - b.0) * (b.0 - control.0) > 0.0, "dpr {dpr}");
        chart.remove_drawing(id);

        let id = add(
            &mut chart,
            DrawingKind::Polyline,
            vec![p(10.0, 10_001.0), p(20.0, 10_001.0), p(15.0, 10_005.0)],
            r##"{"color":"#123456","tool_options":{"shape":{"closed":true}}}"##,
        );
        let outline = ink_line(&mut chart);
        assert_eq!(outline.len(), 5, "dpr {dpr}");
        assert_eq!(outline[0], outline[4]);
        let seam = shape::midpoint(bitmap(anchor(&chart, id, 0)), bitmap(anchor(&chart, id, 1)));
        assert!(close(outline[0], seam, 1e-3), "dpr {dpr}: {:?}", outline[0]);
    }
}

/// On an inverted price scale an on-curve anchor still drags the curve through the pointer's
/// bar slot and price, and a rotated rectangle's edge corner drag still keeps its on-screen width.
#[test]
fn derived_handles_follow_the_pointer_on_an_inverted_scale() {
    let mut chart = chart();
    chart.set_price_scale_inverted(0, false, true);
    chart.build_frame();
    let id = add(
        &mut chart,
        DrawingKind::Curve,
        vec![p(10.0, 101.0), p(15.0, 105.0), p(20.0, 101.0)],
        r##"{"color":"#123456"}"##,
    );
    chart.set_selected_drawing(Some(id));
    let (a, middle, b) = (
        anchor(&chart, id, 0),
        anchor(&chart, id, 1),
        anchor(&chart, id, 2),
    );
    assert!(middle.1 > a.1, "the inverted curve bows down");
    let to = (middle.0 + 25.0, middle.1 + 30.0);
    assert!(chart.drawing_drag_start_at(middle.0, middle.1));
    chart.drawing_drag_to(to.0, to.1, DrawingModifiers::default());
    chart.drawing_drag_end();
    let moved = on_slot(&chart, to);
    assert!(close(anchor(&chart, id, 1), moved, 1e-6));
    assert!(shape::distance_to_polyline(moved, &ink_line(&mut chart)) <= 0.11);
    assert!(close(anchor(&chart, id, 0), a, 1e-9) && close(anchor(&chart, id, 2), b, 1e-9));
    chart.remove_drawing(id);

    let id = add(
        &mut chart,
        DrawingKind::RotatedRectangle,
        points_for(DrawingKind::RotatedRectangle),
        "{}",
    );
    chart.set_selected_drawing(Some(id));
    let (a, b) = (anchor(&chart, id, 0), anchor(&chart, id, 1));
    let width = depth(&chart, id);
    let quarter = (a.0 - (b.1 - a.1), a.1 + (b.0 - a.0));
    assert!(chart.drawing_drag_start_at(b.0, b.1));
    chart.drawing_drag_to(quarter.0, quarter.1, DrawingModifiers::default());
    chart.drawing_drag_end();
    assert!(close(anchor(&chart, id, 1), on_slot(&chart, quarter), 1e-3));
    assert!(
        (depth(&chart, id) - width).abs() < 1e-3,
        "{} vs {width}",
        depth(&chart, id)
    );
}

/// Documents the fork wrote regain its shapes look on upstream's arms: arcs fill their circular
/// segment and closed polylines their region through the fork's fill defaults, and the caps the
/// fork painted come back.
#[test]
fn fork_documents_regain_the_fork_shapes_look() {
    let at = |logical: f64, price: f64| serde_json::json!({"logical": logical, "price": price, "time": logical * HOUR});
    let document = serde_json::json!({
        "schema": "aeris_charts-state",
        "schema_version": 1,
        "panes": [{"id": "pane-1"}],
        "drawings": [
            // The fork's arc order: start, end, then the point it passes through.
            {"id": 1, "kind": "arc", "pane_id": "pane-1",
             "anchors": [at(10.0, 101.0), at(20.0, 101.0), at(15.0, 104.0)],
             "style": {"color": INK, "stroke_end": "arrow"}},
            {"id": 2, "kind": "polyline", "pane_id": "pane-1",
             "anchors": [at(22.0, 101.0), at(30.0, 101.0), at(26.0, 105.0)],
             "style": {"color": INK, "tool_options": {"shape": {"closed": true}}}},
        ]
    })
    .to_string();
    let mut chart = chart();
    chart.import_state_json(&document).unwrap();
    assert_eq!(
        chart.drawing(1).unwrap().points,
        [p(10.0, 101.0), p(15.0, 104.0), p(20.0, 101.0)]
    );
    assert!(chart.drawing(1).unwrap().fill_enabled && chart.drawing(2).unwrap().fill_enabled);
    let regions = fills(&mut chart, Color::rgba(0x12, 0x34, 0x56, 51));
    assert_eq!(
        regions.len(),
        2,
        "the arc's segment and the polyline's region"
    );
    assert!(regions.iter().any(|(upper, lower)| shape::point_in_ribbon(
        px(&chart, p(15.0, 102.5)),
        upper,
        lower
    )));
    assert!(regions.iter().any(|(upper, lower)| shape::point_in_ribbon(
        px(&chart, p(26.0, 102.0)),
        upper,
        lower
    )));
    let heads = arrowheads(&mut chart);
    assert_eq!(heads.len(), 1);
    assert!(close(heads[0].0, px(&chart, p(20.0, 101.0)), 1e-3));
}
