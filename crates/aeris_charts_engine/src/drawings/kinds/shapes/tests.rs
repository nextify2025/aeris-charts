//! Shapes-family engine tests: catalog defaults, armed placement for every placement class,
//! shared-part frames and fills, hit testing (indexed and brute force), drags, keyboard nudges,
//! magnet, time identity, schema and kind options, patches with history, persistence, clipboard,
//! sync, device-pixel ratios, and bounded work at extreme zoom.

use aeris_charts_render::color::Color;
use aeris_charts_render::draw_list::Prim;
use aeris_charts_render::shape::{self, Point, MAX_FLATTEN_POINTS};

use super::super::super::{DrawingHandleMode, DrawingPlacement, DrawingTextLayout};
use super::ShapeToolOptions;
use crate::{
    ChartEngine, DrawingAnchor, DrawingDragPart, DrawingId, DrawingKind, DrawingKindOptions,
    DrawingLineCap, DrawingMagnetMode, DrawingModifiers, DrawingPoint, DrawingPriceScale,
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

fn px(chart: &ChartEngine, point: DrawingPoint) -> Point {
    chart
        .drawing_to_px_for(0, DrawingPriceScale::Right, point)
        .unwrap()
}

fn ink() -> Color {
    Color::parse_css(INK).unwrap()
}

fn wash() -> Color {
    Color::rgba(0x12, 0x34, 0x56, 51)
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

fn hit(chart: &ChartEngine, point: Point) -> Option<DrawingId> {
    chart.hit_test_drawing(point.0, point.1).map(|hit| hit.id)
}

#[test]
fn catalog_defaults_follow_each_tool() {
    for (index, kind) in SHAPE_KINDS.into_iter().enumerate() {
        let spec = kind.spec();
        let drawing = crate::Drawing::new(1, kind, 0, Vec::new());
        assert!(spec.family.is_some(), "{kind:?} is a family tool");
        assert_eq!(usize::from(spec.wire_id), 192 + index);
        assert_eq!(spec.text_layout, DrawingTextLayout::Box);
        assert_eq!(
            spec.placement,
            match kind {
                DrawingKind::Ellipse | DrawingKind::Circle => {
                    DrawingPlacement::ClickAnchors { count: 2 }
                }
                DrawingKind::DoubleCurve => DrawingPlacement::ClickAnchors { count: 4 },
                DrawingKind::Polyline => DrawingPlacement::MultiClick { minimum: 2 },
                DrawingKind::Highlighter => DrawingPlacement::Freehand { minimum: 2 },
                _ => DrawingPlacement::ClickAnchors { count: 3 },
            }
        );
        assert_eq!(
            spec.handles,
            match kind {
                DrawingKind::Ellipse => DrawingHandleMode::RectangleBounds,
                DrawingKind::Highlighter => DrawingHandleMode::Endpoints,
                _ => DrawingHandleMode::Anchors,
            }
        );
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
        assert_eq!(
            drawing.kind_options(),
            DrawingKindOptions::Shape { closed: false }
        );
    }
    let highlighter = crate::Drawing::new(1, DrawingKind::Highlighter, 0, Vec::new());
    let color = Color::parse_css(&highlighter.color).expect("a CSS color");
    assert_eq!((color.r(), color.g(), color.b()), (0xf5, 0x9e, 0x0a));
    assert_eq!(color.a(), 102, "40% opacity");
    // The shared hook defaults to the anchors' box for families that do not override it.
    fn no_parts(_: &super::PartContext<'_>, _: &mut super::DrawingParts) {}
    let neutral = super::DrawingFamily::new(no_parts, |_| DrawingKindOptions::Generic);
    assert_eq!(
        (neutral.text_box)(DrawingKind::Circle, &[(0.0, 0.0), (1.0, 1.0)]),
        None
    );
}

#[test]
fn armed_tools_place_every_shapes_kind() {
    let mut chart = chart();
    let template = Some(r##"{"color":"#123456"}"##);
    for kind in SHAPE_KINDS {
        assert!(chart.set_drawing_tool(Some(kind), template, None));
        let clicks = [
            (200.0, 260.0),
            (420.0, 150.0),
            (300.0, 120.0),
            (380.0, 300.0),
        ];
        let id = match kind.spec().placement {
            DrawingPlacement::ClickAnchors { count } => {
                let mut created = None;
                for &(x, y) in &clicks[..usize::from(count)] {
                    created = chart
                        .drawing_tool_activate(x, y, DrawingModifiers::default())
                        .created;
                }
                created
            }
            DrawingPlacement::MultiClick { .. } => {
                for &(x, y) in &clicks[..3] {
                    let update = chart.drawing_tool_activate(x, y, DrawingModifiers::default());
                    assert_eq!(update.created, None, "a sequence waits for its finish");
                }
                chart.drawing_tool_finish().created
            }
            DrawingPlacement::Freehand { .. } => {
                let modifiers = DrawingModifiers::default();
                assert!(
                    chart
                        .drawing_tool_pointer_down(200.0, 260.0, modifiers)
                        .pointer_capture
                );
                for step in 1..20 {
                    let x = 200.0 + f64::from(step) * 8.0;
                    chart.drawing_tool_pointer_move(x, 260.0 - f64::from(step), modifiers, true);
                }
                chart
                    .drawing_tool_pointer_up(360.0, 240.0, modifiers)
                    .created
            }
            placement => panic!("unexpected placement {placement:?}"),
        }
        .unwrap_or_else(|| panic!("{kind:?} committed"));
        let drawing = chart.drawing(id).unwrap();
        assert_eq!(drawing.kind, kind);
        match kind {
            DrawingKind::Polyline => assert_eq!(drawing.points.len(), 3),
            DrawingKind::Highlighter => assert!(drawing.points.len() > 10),
            _ => assert_eq!(drawing.points.len(), kind.anchor_count()),
        }
        assert_eq!(drawing.color, INK, "{kind:?} takes the armed template");
        assert_eq!(chart.active_drawing_tool(), None, "one-shot tools disarm");
        assert_eq!(chart.selected_drawing(), Some(id));
    }
    // Without a template the highlighter keeps its translucent marker color.
    assert!(chart.set_drawing_tool(Some(DrawingKind::Highlighter), None, None));
    let modifiers = DrawingModifiers::default();
    chart.drawing_tool_pointer_down(100.0, 100.0, modifiers);
    chart.drawing_tool_pointer_move(140.0, 110.0, modifiers, true);
    let id = chart
        .drawing_tool_pointer_up(180.0, 100.0, modifiers)
        .created
        .unwrap();
    assert!(chart.drawing(id).unwrap().color.starts_with("rgba("));
}

#[test]
fn rotated_rectangles_are_symmetric_right_angled_boxes_over_their_fill() {
    let mut chart = chart();
    let id = add(
        &mut chart,
        DrawingKind::RotatedRectangle,
        points_for(DrawingKind::RotatedRectangle),
        r##"{"color":"#123456"}"##,
    );
    let (a, b, c) = (
        anchor(&chart, id, 0),
        anchor(&chart, id, 1),
        anchor(&chart, id, 2),
    );
    let outline = ink_line(&mut chart);
    assert_eq!(
        outline.len(),
        6,
        "mid-edge start, four corners, mid-edge end"
    );
    assert_eq!(outline[0], outline[5]);
    let corners = &outline[1..5];
    for index in 0..4 {
        let (p0, p1, p2) = (
            corners[index],
            corners[(index + 1) % 4],
            corners[(index + 2) % 4],
        );
        let (u, v) = ((p1.0 - p0.0, p1.1 - p0.1), (p2.0 - p1.0, p2.1 - p1.1));
        let cosine = (u.0 * v.0 + u.1 * v.1) / (u.0.hypot(u.1) * v.0.hypot(v.1));
        assert!(
            cosine.abs() < 1e-5,
            "right angle at corner {index}: {cosine}"
        );
    }
    // The axis joins the short sides' midpoints and the third anchor lies on a long side.
    let midpoints = (0..4)
        .map(|index| {
            let (p0, p1) = (corners[index], corners[(index + 1) % 4]);
            ((p0.0 + p1.0) / 2.0, (p0.1 + p1.1) / 2.0)
        })
        .collect::<Vec<_>>();
    assert!(midpoints.iter().any(|&m| close(m, a, 1e-3)));
    assert!(midpoints.iter().any(|&m| close(m, b, 1e-3)));
    assert!((0..4).any(|index| {
        let (p0, p1) = (corners[index], corners[(index + 1) % 4]);
        let cross = (p1.0 - p0.0) * (c.1 - p0.1) - (p1.1 - p0.1) * (c.0 - p0.0);
        cross.abs() / (p1.0 - p0.0).hypot(p1.1 - p0.1) < 1e-3
    }));
    let fill = fills(&mut chart, wash());
    assert_eq!(fill.len(), 1, "the 20% wash of the stroke color");
    // The outline hits unselected; the interior only once selected.
    let center = ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0);
    chart.set_selected_drawing(None);
    assert_eq!(hit(&chart, center), None);
    assert_eq!(hit(&chart, corners[0]), Some(id));
    chart.set_selected_drawing(Some(id));
    assert_eq!(hit(&chart, center), Some(id));
}

#[test]
fn ellipses_circles_and_triangles_trace_their_true_outlines() {
    let mut chart = chart();
    let ellipse = add(
        &mut chart,
        DrawingKind::Ellipse,
        points_for(DrawingKind::Ellipse),
        r##"{"color":"#123456"}"##,
    );
    let (a, b) = (anchor(&chart, ellipse, 0), anchor(&chart, ellipse, 1));
    let (cx, cy) = ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0);
    let (rx, ry) = ((b.0 - a.0).abs() / 2.0, (b.1 - a.1).abs() / 2.0);
    let outline = ink_line(&mut chart);
    assert!(outline.len() > 16);
    for point in &outline {
        let value = ((point.0 - cx) / rx).powi(2) + ((point.1 - cy) / ry).powi(2);
        assert!(
            (value - 1.0).abs() < 1e-3,
            "{point:?} on the inscribed ellipse"
        );
    }
    let (fill_upper, _) = fills(&mut chart, wash()).remove(0);
    assert!(fill_upper.len() > 8);
    chart.remove_drawing(ellipse);

    let circle = add(
        &mut chart,
        DrawingKind::Circle,
        points_for(DrawingKind::Circle),
        r##"{"color":"#123456"}"##,
    );
    let (center, rim) = (anchor(&chart, circle, 0), anchor(&chart, circle, 1));
    let radius = (rim.0 - center.0).hypot(rim.1 - center.1);
    let outline = ink_line(&mut chart);
    for pair in outline.windows(2) {
        assert!(((pair[0].0 - center.0).hypot(pair[0].1 - center.1) - radius).abs() < 1e-3);
        let middle = ((pair[0].0 + pair[1].0) / 2.0, (pair[0].1 + pair[1].1) / 2.0);
        let sagitta = radius - (middle.0 - center.0).hypot(middle.1 - center.1);
        assert!(sagitta <= 0.25 + 1e-9, "chords within the curve tolerance");
    }
    assert_eq!(hit(&chart, rim), Some(circle), "the rim is the body");
    chart.remove_drawing(circle);

    let triangle = add(
        &mut chart,
        DrawingKind::Triangle,
        points_for(DrawingKind::Triangle),
        r##"{"color":"#123456"}"##,
    );
    let vertices = (0..3)
        .map(|index| anchor(&chart, triangle, index))
        .collect::<Vec<_>>();
    let outline = ink_line(&mut chart);
    assert_eq!(outline.len(), 5);
    for vertex in &vertices {
        assert!(outline.iter().any(|point| close(*point, *vertex, 1e-3)));
    }
    let (upper, lower) = fills(&mut chart, wash()).remove(0);
    let centroid = (
        vertices.iter().map(|v| v.0).sum::<f64>() / 3.0,
        vertices.iter().map(|v| v.1).sum::<f64>() / 3.0,
    );
    assert!(shape::point_in_ribbon(centroid, &upper, &lower));
}

#[test]
fn open_shapes_pass_through_every_anchor() {
    let mut chart = chart();
    for kind in [
        DrawingKind::Arc,
        DrawingKind::Curve,
        DrawingKind::DoubleCurve,
        DrawingKind::Polyline,
    ] {
        let id = add(
            &mut chart,
            kind,
            points_for(kind),
            r##"{"color":"#123456"}"##,
        );
        let anchors = (0..kind_points(&chart, id))
            .map(|index| anchor(&chart, id, index))
            .collect::<Vec<_>>();
        // Frames clip strokes to the pane, so a curve dipping below it (the double curve's
        // overshoot past its last anchor) arrives as several runs.
        let runs = polylines(&mut chart, ink());
        let (line, width) = runs[0].clone();
        assert!(
            close(line[0], anchors[0], 1e-3),
            "{kind:?} starts on anchor 0"
        );
        let end = match kind {
            DrawingKind::Polyline => *anchors.last().unwrap(),
            _ => anchors[1],
        };
        let last = &runs.last().unwrap().0;
        assert!(close(*last.last().unwrap(), end, 1e-3), "{kind:?} ends");
        for anchor in &anchors {
            assert!(
                runs.iter()
                    .any(|(run, _)| shape::distance_to_polyline(*anchor, run) <= 0.26),
                "{kind:?} passes through {anchor:?}"
            );
        }
        assert!(runs.iter().all(|(_, run_width)| *run_width == width));
        if kind == DrawingKind::Arc {
            let (center, radius) =
                shape::circle_through(anchors[0], anchors[1], anchors[2]).expect("a proper arc");
            assert!(line
                .iter()
                .all(
                    |point| ((point.0 - center.0).hypot(point.1 - center.1) - radius).abs() < 1e-3
                ));
            assert_eq!(fills(&mut chart, wash()).len(), 1, "circular segment fill");
        }
        assert_eq!(width, 2.0);
        if matches!(kind, DrawingKind::Curve | DrawingKind::DoubleCurve) {
            assert!(
                fills(&mut chart, wash()).is_empty(),
                "curves start unfilled"
            );
        }
        if kind == DrawingKind::Polyline {
            assert!(
                fills(&mut chart, wash()).is_empty(),
                "an open polyline never fills"
            );
        }
        chart.remove_drawing(id);
    }
    // Collinear arc anchors degrade to the straight chord.
    let id = add(
        &mut chart,
        DrawingKind::Arc,
        vec![p(10.0, 101.0), p(20.0, 101.0), p(15.0, 101.0)],
        r##"{"color":"#123456"}"##,
    );
    let line = ink_line(&mut chart);
    assert_eq!(line.len(), 2);
    assert!(close(line[1], anchor(&chart, id, 1), 1e-3));
}

fn kind_points(chart: &ChartEngine, id: DrawingId) -> usize {
    chart.drawing(id).unwrap().points.len()
}

/// How many quads of a ribbon cover `point` (each executor blends every quad once).
fn quad_coverage(point: Point, upper: &[Point], lower: &[Point]) -> usize {
    (1..upper.len().min(lower.len()))
        .filter(|&index| {
            shape::point_in_polygon(
                point,
                &[
                    upper[index - 1],
                    lower[index - 1],
                    lower[index],
                    upper[index],
                ],
            )
        })
        .count()
}

#[test]
fn highlighters_paint_their_wide_stroke_once_per_pixel() {
    let mut chart = chart();
    // A scribble that doubles back over itself with sub-width jitter.
    let points = (0..120)
        .map(|index| {
            let t = f64::from(index);
            p(
                10.0 + (t / 8.0).sin() * 4.0 + t * 0.05,
                102.0 + (t / 5.0).cos() * 1.5 + f64::from(index % 3) * 0.02,
            )
        })
        .collect::<Vec<_>>();
    let id = add(&mut chart, DrawingKind::Highlighter, points, "{}");
    let highlighter = chart.drawing(id).unwrap().clone();
    let marker = Color::parse_css(&highlighter.color).unwrap();
    assert!(
        polylines(&mut chart, marker).is_empty(),
        "painted as a region"
    );
    let mut regions = fills(&mut chart, marker);
    assert_eq!(regions.len(), 1, "one region in the marker color");
    let (upper, lower) = regions.remove(0);
    let anchors = (0..highlighter.points.len())
        .map(|index| anchor(&chart, id, index))
        .collect::<Vec<_>>();
    let bounds = shape::Rect::bounding(&anchors).unwrap().inflate(12.0);
    let radius = highlighter.width / 2.0;
    let mut inside = 0;
    for gy in 0..60 {
        for gx in 0..60 {
            let point = (
                bounds.left + (bounds.right - bounds.left) * (f64::from(gx) + 0.5) / 60.0,
                bounds.top + (bounds.bottom - bounds.top) * (f64::from(gy) + 0.5) / 60.0,
            );
            let coverage = quad_coverage(point, &upper, &lower);
            assert!(coverage <= 1, "{point:?} blends {coverage} times");
            let distance = shape::distance_to_polyline(point, &anchors);
            if (distance - radius).abs() > 0.6 {
                assert_eq!(coverage == 1, distance < radius, "{point:?}");
            }
            inside += coverage;
        }
    }
    assert!(inside > 200, "{inside} samples inside");
    // The round cap reaches half the width beyond the last sample, and the whole wide stroke is
    // the body target.
    let (last, before) = (anchors[anchors.len() - 1], anchors[anchors.len() - 2]);
    let away = {
        let (dx, dy) = (last.0 - before.0, last.1 - before.1);
        let length = dx.hypot(dy);
        (
            last.0 + dx / length * radius * 0.8,
            last.1 + dy / length * radius * 0.8,
        )
    };
    assert_eq!(quad_coverage(away, &upper, &lower), 1, "round cap");
    chart.set_selected_drawing(None);
    let side = (anchors[40].0, anchors[40].1 + radius - 1.0);
    if shape::distance_to_polyline(side, &anchors) < radius - 0.5 {
        assert_eq!(hit(&chart, side), Some(id));
    }
    assert_eq!(hit(&chart, anchors[60]), Some(id));
    // A dashed style does not break the marker into dashes.
    assert!(chart.drawing_apply_options(id, r#"{"style":"dashed"}"#));
    assert_eq!(fills(&mut chart, marker).len(), 1);
    assert!(polylines(&mut chart, marker).is_empty());
    chart.remove_drawing(id);

    // A scribble of 300 pane-wide jumps crossing each other everywhere exceeds the fill bounds
    // even coarsened: it still paints, as a plain stroke.
    let zigzag = (0..300_u32)
        .map(|index| {
            p(
                1.0 + f64::from(index * 7_919 % 97) * 0.38,
                100.2 + f64::from(index * 104_729 % 89) * 0.065,
            )
        })
        .collect::<Vec<_>>();
    add(&mut chart, DrawingKind::Highlighter, zigzag, "{}");
    assert!(fills(&mut chart, marker).is_empty());
    assert_eq!(polylines(&mut chart, marker).len(), 1);
}

#[test]
fn curves_fill_their_chord_region_and_extend_along_their_tangents() {
    let mut chart = chart();
    // An S-shaped cubic crosses its chord: both lobes fill by the nonzero rule.
    let id = add(
        &mut chart,
        DrawingKind::DoubleCurve,
        vec![
            p(10.0, 102.0),
            p(22.0, 102.0),
            p(14.0, 105.0),
            p(18.0, 99.0),
        ],
        r##"{"color":"#123456","fill_enabled":true}"##,
    );
    chart.set_selected_drawing(Some(id));
    let lobe = |chart: &ChartEngine, logical: f64, price: f64| px(chart, p(logical, price));
    assert_eq!(
        hit(&chart, lobe(&chart, 14.0, 103.5)),
        Some(id),
        "upper lobe"
    );
    assert_eq!(
        hit(&chart, lobe(&chart, 18.0, 100.5)),
        Some(id),
        "lower lobe"
    );
    assert_eq!(hit(&chart, lobe(&chart, 14.0, 100.0)), None, "outside");
    chart.remove_drawing(id);

    // A curve extended to the right continues its end tangent to the pane edge; the end cap only
    // paints on the unextended start.
    let id = add(
        &mut chart,
        DrawingKind::Curve,
        points_for(DrawingKind::Curve),
        r##"{"color":"#123456","extend_right":true,"stroke_start":"arrow","stroke_end":"arrow"}"##,
    );
    let (a, b, m) = (
        anchor(&chart, id, 0),
        anchor(&chart, id, 1),
        anchor(&chart, id, 2),
    );
    let control = (2.0 * m.0 - (a.0 + b.0) / 2.0, 2.0 * m.1 - (a.1 + b.1) / 2.0);
    let line = ink_line(&mut chart);
    let edge = *line.last().unwrap();
    let pane_bottom = chart.panes[0].top + chart.panes[0].height;
    assert!(
        (edge.0 - chart.pane_w).abs() < 0.5
            || (edge.1 - pane_bottom).abs() < 0.5
            || (edge.1 - chart.panes[0].top).abs() < 0.5,
        "{edge:?} on the pane edge"
    );
    let cross = (b.0 - control.0) * (edge.1 - control.1) - (b.1 - control.1) * (edge.0 - control.0);
    assert!(cross.abs() / (edge.0 - control.0).hypot(edge.1 - control.1) < 1e-2);
    let frame = chart.build_frame();
    let arrowheads = frame.panes[0]
        .main
        .iter()
        .filter(|prim| matches!(prim, Prim::BandFill { fill, .. } if *fill == ink()))
        .count();
    assert_eq!(arrowheads, 1, "only the start carries its arrow");
    let trimmed = (line[0].0 - a.0).hypot(line[0].1 - a.1);
    assert!(
        (trimmed - 2.0).abs() < 0.05,
        "the arrow start trims the stroke by one width ({trimmed})"
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
        l_shape,
        r##"{"color":"#123456","tool_options":{"shape":{"closed":true}}}"##,
    );
    let outline = ink_line(&mut chart);
    assert_eq!(
        outline.len(),
        8,
        "every vertex plus the mid-edge seam twice"
    );
    assert_eq!(outline[0], outline[7]);
    assert_eq!(fills(&mut chart, wash()).len(), 1);
    chart.set_selected_drawing(Some(id));
    assert_eq!(hit(&chart, px(&chart, p(12.5, 102.0))), Some(id));
    assert_eq!(hit(&chart, px(&chart, p(12.5, 104.0))), Some(id));
    assert_eq!(hit(&chart, px(&chart, p(17.5, 104.0))), None, "the notch");
    // Opening it removes the fill and the closing edge.
    assert!(chart.drawing_apply_options(id, r#"{"tool_options":{"shape":{"closed":false}}}"#));
    assert!(fills(&mut chart, wash()).is_empty());
    assert_eq!(ink_line(&mut chart).len(), 6);
    assert!(chart.undo_drawing());
    assert_eq!(fills(&mut chart, wash()).len(), 1);
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

#[test]
fn closed_polylines_beyond_the_fill_vertex_bound_keep_only_their_outline() {
    let closed = r##"{"color":"#123456","tool_options":{"shape":{"closed":true}}}"##;
    let middle = p(20.0, 103.0);

    // The largest polygon fills and its interior selects it.
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

    // One vertex more paints its outline and nothing else, on the shared frame every backend
    // executes: no fill part, so no interior target even while selected.
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
    let vertex = px(&beyond, convex_anchors(count)[0]);
    assert_eq!(
        hit(&beyond, vertex),
        Some(id),
        "the stroke still selects it"
    );
}

#[test]
fn fills_follow_fill_enabled_and_fill_color() {
    let mut chart = chart();
    let id = add(
        &mut chart,
        DrawingKind::Circle,
        points_for(DrawingKind::Circle),
        r##"{"color":"#123456"}"##,
    );
    let (center, rim) = (anchor(&chart, id, 0), anchor(&chart, id, 1));
    // Inside, away from the center and rim handles.
    let inside = (
        center.0,
        center.1 + (rim.0 - center.0).hypot(rim.1 - center.1) / 2.0,
    );
    assert_eq!(
        hit(&chart, inside),
        None,
        "unselected, the interior pans the chart"
    );
    chart.set_selected_drawing(Some(id));
    assert_eq!(hit(&chart, inside), Some(id));
    assert!(chart.drawing_apply_options(id, r#"{"fill_color":"rgba(255, 0, 0, 0.5)"}"#));
    assert!(fills(&mut chart, wash()).is_empty());
    assert_eq!(fills(&mut chart, Color::rgba(255, 0, 0, 128)).len(), 1);
    assert!(chart.drawing_apply_options(id, r#"{"fill_enabled":false}"#));
    assert!(fills(&mut chart, Color::rgba(255, 0, 0, 128)).is_empty());
    assert_eq!(hit(&chart, inside), None, "an unfilled interior never hits");
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
fn handles_drags_and_nudges_edit_shapes_as_single_history_entries() {
    let mut chart = chart();
    // The rotated rectangle's axis ends plus its two derived width handles.
    let expected_handles = [4, 8, 2, 3, 3, 3, 4, 4, 2];
    let ids = SHAPE_KINDS
        .into_iter()
        .map(|kind| add(&mut chart, kind, points_for(kind), "{}"))
        .collect::<Vec<_>>();
    for (id, expected) in ids.iter().zip(expected_handles) {
        assert_eq!(chart.drawing_handle_count(*id), Some(expected));
    }
    for id in ids {
        chart.remove_drawing(id);
    }

    // Dragging the rim handle resizes a circle around its fixed center.
    let circle = add(
        &mut chart,
        DrawingKind::Circle,
        points_for(DrawingKind::Circle),
        "{}",
    );
    let before = chart.drawing(circle).unwrap().points.clone();
    chart.set_selected_drawing(Some(circle));
    let rim = anchor(&chart, circle, 1);
    assert_eq!(
        chart.hit_test_drawing(rim.0, rim.1).map(|hit| hit.part),
        Some(DrawingDragPart::Anchor(1)),
        "the rim handle wins over the ring"
    );
    assert!(chart.drawing_drag_start_at(rim.0, rim.1));
    chart.drawing_drag_to(rim.0 + 40.0, rim.1 - 30.0, DrawingModifiers::default());
    chart.drawing_drag_end();
    let after = chart.drawing(circle).unwrap().points.clone();
    assert_eq!(after[0], before[0], "the center stays");
    assert!(close(
        anchor(&chart, circle, 1),
        (rim.0 + 40.0, rim.1 - 30.0),
        1e-6
    ));
    assert!(chart.undo_drawing());
    assert_eq!(chart.drawing(circle).unwrap().points, before);

    // A body drag moves every anchor of a rotated rectangle rigidly on screen.
    let rotated = add(
        &mut chart,
        DrawingKind::RotatedRectangle,
        points_for(DrawingKind::RotatedRectangle),
        "{}",
    );
    let starts = (0..3)
        .map(|index| anchor(&chart, rotated, index))
        .collect::<Vec<_>>();
    let grab = (
        (starts[0].0 + starts[1].0) / 2.0,
        (starts[0].1 + starts[1].1) / 2.0,
    );
    chart.set_selected_drawing(Some(rotated));
    assert!(chart.drawing_drag_start_at(grab.0, grab.1));
    chart.drawing_drag_to(grab.0 + 25.0, grab.1 + 15.0, DrawingModifiers::default());
    chart.drawing_drag_end();
    for (index, start) in starts.iter().enumerate() {
        assert!(close(
            anchor(&chart, rotated, index),
            (start.0 + 25.0, start.1 + 15.0),
            1e-6
        ));
    }

    // Shift on an ellipse corner makes its box square: a circle.
    let ellipse = add(
        &mut chart,
        DrawingKind::Ellipse,
        points_for(DrawingKind::Ellipse),
        "{}",
    );
    chart.set_selected_drawing(Some(ellipse));
    let (a, b) = (anchor(&chart, ellipse, 0), anchor(&chart, ellipse, 1));
    let bottom_right = (a.0.max(b.0), a.1.max(b.1));
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
    let (a, b) = (anchor(&chart, ellipse, 0), anchor(&chart, ellipse, 1));
    assert!(
        ((b.0 - a.0).abs() - (b.1 - a.1).abs()).abs() < 1e-3,
        "{a:?} {b:?} from {bottom_right:?}"
    );

    // Keyboard handles and the whole drawing move through the same history path.
    let triangle = add(
        &mut chart,
        DrawingKind::Triangle,
        points_for(DrawingKind::Triangle),
        "{}",
    );
    let before = chart.drawing(triangle).unwrap().points.clone();
    chart.set_selected_drawing(Some(triangle));
    let vertex = anchor(&chart, triangle, 2);
    assert!(chart.nudge_selected_drawing(0.0, -10.0, Some(2)));
    assert!(close(
        anchor(&chart, triangle, 2),
        (vertex.0, vertex.1 - 10.0),
        1e-6
    ));
    assert_eq!(chart.drawing(triangle).unwrap().points[..2], before[..2]);
    assert!(chart.nudge_selected_drawing(5.0, 0.0, None));
    assert!(chart.undo_drawing() && chart.undo_drawing());
    assert_eq!(chart.drawing(triangle).unwrap().points, before);

    // Locked shapes select but never drag.
    assert!(chart.set_drawing_locked(triangle, true));
    let vertex = anchor(&chart, triangle, 0);
    assert!(!chart.drawing_drag_start_at(vertex.0, vertex.1));
    assert!(!chart.nudge_selected_drawing(5.0, 0.0, None));
}

/// The rotated rectangle's half width on screen: the third anchor's distance from the axis.
fn half_width(chart: &ChartEngine, id: DrawingId) -> f64 {
    let (a, b, c) = (
        anchor(chart, id, 0),
        anchor(chart, id, 1),
        anchor(chart, id, 2),
    );
    let normal = shape::segment_normal(a, b).unwrap();
    ((c.0 - a.0) * normal.0 + (c.1 - a.1) * normal.1).abs()
}

fn part_at(chart: &ChartEngine, point: Point) -> Option<DrawingDragPart> {
    chart.hit_test_drawing(point.0, point.1).map(|hit| hit.part)
}

#[test]
fn rotated_rectangle_width_handles_and_axis_drags_keep_its_width() {
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
    let half = (c.0 - a.0) * normal.0 + (c.1 - a.1) * normal.1;
    let middle = shape::midpoint(a, b);
    let side = (middle.0 + normal.0 * half, middle.1 + normal.1 * half);
    let opposite = (middle.0 - normal.0 * half, middle.1 - normal.1 * half);

    // The axis ends and the long sides' midpoints are the handles; the third anchor, placed off
    // its side's midpoint, has none of its own.
    assert_eq!(chart.drawing_handle_count(id), Some(4));
    assert_eq!(part_at(&chart, a), Some(DrawingDragPart::Anchor(0)));
    assert_eq!(part_at(&chart, side), Some(DrawingDragPart::Handle(0)));
    assert_eq!(part_at(&chart, opposite), Some(DrawingDragPart::Handle(1)));
    assert!(
        !close(c, side, 6.5),
        "the fixture's width point is off the midpoint"
    );
    assert_ne!(part_at(&chart, c), Some(DrawingDragPart::Anchor(2)));

    // Pulling the opposite width handle 15 px outward widens the rectangle symmetrically in
    // one undo step, keeping the axis.
    assert!(chart.drawing_drag_start_at(opposite.0, opposite.1));
    chart.drawing_drag_to(
        opposite.0 - normal.0 * 15.0 + normal.1 * 9.0,
        opposite.1 - normal.1 * 15.0 - normal.0 * 9.0,
        DrawingModifiers::default(),
    );
    chart.drawing_drag_end();
    assert!((half_width(&chart, id) - (half.abs() + 15.0)).abs() < 1e-3);
    assert!(close(anchor(&chart, id, 0), a, 1e-9) && close(anchor(&chart, id, 1), b, 1e-9));
    assert!(chart.undo_drawing());
    assert_eq!(chart.drawing(id).unwrap().points, before);

    // Turning the axis a quarter turn around its first end keeps the width; passing through a
    // zero-length axis on the way does not lose it either.
    let quarter = (a.0 - (b.1 - a.1), a.1 + (b.0 - a.0));
    assert!(chart.drawing_drag_start_at(b.0, b.1));
    chart.drawing_drag_to(a.0, a.1, DrawingModifiers::default());
    chart.drawing_drag_to(quarter.0, quarter.1, DrawingModifiers::default());
    chart.drawing_drag_end();
    assert!(close(anchor(&chart, id, 1), quarter, 1e-3));
    assert!((half_width(&chart, id) - half.abs()).abs() < 1e-3);
    assert!(chart.undo_drawing());

    // Keyboard: the third handle (the first width handle) nudges only the width; an axis end
    // nudge keeps it.
    assert!(chart.nudge_selected_drawing(0.0, -10.0, Some(2)));
    let expected = (half + normal.1 * -10.0).abs();
    assert!((half_width(&chart, id) - expected).abs() < 1e-3);
    assert!(close(anchor(&chart, id, 0), a, 1e-9));
    assert!(chart.nudge_selected_drawing(0.0, 30.0, Some(1)));
    assert!((half_width(&chart, id) - expected).abs() < 1e-3);
    assert!(chart.undo_drawing() && chart.undo_drawing());
    assert_eq!(chart.drawing(id).unwrap().points, before);

    // The magnet snaps a width handle like an anchor: the long side then passes through the bar
    // value under the pointer.
    chart.set_drawing_magnet_mode(DrawingMagnetMode::Strong);
    let grab = side;
    let to = (grab.0 + 4.0, grab.1 - 20.0);
    assert!(chart.drawing_drag_start_at(grab.0, grab.1));
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

#[test]
fn clicking_the_first_vertex_closes_a_polyline() {
    let mut chart = chart();
    let modifiers = DrawingModifiers::default();
    let vertices = [(200.0, 250.0), (300.0, 150.0), (380.0, 260.0)];
    let place = |chart: &mut ChartEngine, kind: DrawingKind| {
        assert!(chart.set_drawing_tool(Some(kind), Some(r##"{"color":"#123456"}"##), None));
        for (x, y) in vertices {
            assert!(chart
                .drawing_tool_activate(x, y, modifiers)
                .created
                .is_none());
        }
    };
    let near_first = (vertices[0].0 + 3.0, vertices[0].1 - 2.0);

    // Hovering the first vertex closes the preview onto it; clicking there commits the three
    // vertices closed (filled) and disarms the tool, as one undo step.
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

    // Enter and double-click still finish open, and Escape still cancels.
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

    // The core path has no close: a click on its first vertex places another vertex.
    place(&mut chart, DrawingKind::Path);
    assert!(chart
        .drawing_tool_activate(near_first.0, near_first.1, modifiers)
        .created
        .is_none());
    let id = chart.drawing_tool_finish().created.unwrap();
    assert_eq!(chart.drawing(id).unwrap().points.len(), 4);
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
fn schema_kind_options_and_tool_option_patches_are_typed_and_atomic() {
    let property = |kind: DrawingKind, name: &str| {
        crate::drawing_property_schema(kind)
            .properties
            .into_iter()
            .find(|property| property.name == name)
    };
    let closed = property(DrawingKind::Polyline, "tool_options.shape.closed").unwrap();
    assert_eq!(closed.default, serde_json::json!(false));
    assert_eq!(closed.property_type, crate::DrawingPropertyType::Boolean);
    assert!(property(DrawingKind::Circle, "tool_options.shape.closed").is_none());
    assert_eq!(
        property(DrawingKind::Circle, "fill_enabled")
            .unwrap()
            .default,
        serde_json::json!(true)
    );
    assert_eq!(
        property(DrawingKind::Curve, "fill_enabled")
            .unwrap()
            .default,
        serde_json::json!(false)
    );
    assert_eq!(
        property(DrawingKind::Highlighter, "width").unwrap().default,
        serde_json::json!(20.0)
    );

    let mut chart = chart();
    let id = add(
        &mut chart,
        DrawingKind::Polyline,
        points_for(DrawingKind::Polyline),
        "{}",
    );
    let kind_options = |chart: &ChartEngine| {
        serde_json::from_str::<serde_json::Value>(&chart.drawing_kind_options_json(id).unwrap())
            .unwrap()
    };
    assert_eq!(
        kind_options(&chart),
        serde_json::json!({"kind": "shape", "closed": false})
    );
    assert!(chart.drawing_apply_options(
        id,
        r#"{"tool_options":{"shape":{"closed":true}},"width":3}"#
    ));
    assert_eq!(
        chart.drawing(id).unwrap().tool_options.shape,
        Some(ShapeToolOptions { closed: true })
    );
    assert_eq!(
        kind_options(&chart),
        serde_json::json!({"kind": "shape", "closed": true})
    );
    let before = chart.drawing(id).unwrap().clone();
    for invalid in [
        r#"{"tool_options":{"shape":{"closed":"yes"}},"width":9}"#,
        r#"{"tool_options":{"shape":7}}"#,
    ] {
        assert!(!chart.drawing_apply_options(id, invalid), "{invalid}");
        assert_eq!(chart.drawing(id).unwrap(), &before);
    }
    assert!(chart.drawing_apply_options(id, r#"{"tool_options":{"shape":null}}"#));
    assert!(chart.drawing(id).unwrap().tool_options.is_empty());
    assert!(chart.undo_drawing());
    assert_eq!(chart.drawing(id).unwrap().tool_options, before.tool_options);
    let options: serde_json::Value =
        serde_json::from_str(&chart.drawing_options_json(id).unwrap()).unwrap();
    assert_eq!(
        options["tool_options"],
        serde_json::json!({"shape": {"closed": true}})
    );
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

#[test]
fn frames_scale_shape_geometry_with_the_device_pixel_ratio() {
    // 801 × 1.5 rounds the bitmap width apart from the height ratio.
    for (width, dpr) in [(800.0, 1.0), (801.0, 1.5), (800.0, 2.0)] {
        let mut chart = ChartEngine::new(width, 500.0, dpr);
        let times = hourly(40);
        let values = (0..times.len())
            .map(|index| 100.0 + (index % 7) as f64)
            .collect::<Vec<_>>();
        chart
            .set_series_data(0, &times, &values, &values, &values, &values)
            .unwrap();
        chart.time_scale.set_width(width);
        chart.fit_content();
        chart.build_frame();
        let id = add(
            &mut chart,
            DrawingKind::Triangle,
            points_for(DrawingKind::Triangle),
            r##"{"color":"#123456"}"##,
        );
        let hpr = (chart.pane_w * dpr).round() / chart.pane_w;
        let vpr = (chart.pane_h * dpr).round() / chart.pane_h;
        let (line, stroke_width) = polylines(&mut chart, ink()).remove(0);
        assert!((f64::from(stroke_width) - 2.0 * vpr).abs() < 1e-5);
        for index in 0..3 {
            let vertex = anchor(&chart, id, index);
            let expected = (vertex.0 * hpr, vertex.1 * vpr);
            assert!(
                line.iter().any(|point| close(*point, expected, 1e-3)),
                "dpr {dpr}: vertex {index}"
            );
        }
        // The circle's derived outline shares the anchors' space.
        let circle = add(
            &mut chart,
            DrawingKind::Circle,
            points_for(DrawingKind::Circle),
            r##"{"color":"#654321"}"##,
        );
        let (center, rim) = (anchor(&chart, circle, 0), anchor(&chart, circle, 1));
        let ring = polylines(&mut chart, Color::parse_css("#654321").unwrap())
            .remove(0)
            .0;
        let rim_px = (rim.0 * hpr, rim.1 * vpr);
        assert!(close(ring[0], rim_px, 1e-3), "{:?} vs {rim_px:?}", ring[0]);
        let radius = (rim_px.0 - center.0 * hpr).hypot(rim_px.1 - center.1 * vpr);
        assert!(ring.iter().all(|point| {
            ((point.0 - center.0 * hpr).hypot(point.1 - center.1 * vpr) - radius).abs() < 1e-3
        }));
    }
}

#[test]
fn huge_zoomed_circles_stay_round_on_screen_with_bounded_work() {
    let mut chart = chart();
    // A center far below the pane: the visible rim of a radius-thousands-of-px circle.
    let id = add(
        &mut chart,
        DrawingKind::Circle,
        vec![p(20.0, 0.0), p(20.0, 103.0)],
        r##"{"color":"#123456"}"##,
    );
    let (center, rim) = (anchor(&chart, id, 0), anchor(&chart, id, 1));
    let radius = (rim.0 - center.0).hypot(rim.1 - center.1);
    assert!(radius > 3_000.0, "{radius}");
    let ring = ink_line(&mut chart);
    assert!(
        ring.len() <= MAX_FLATTEN_POINTS + 1,
        "{} points",
        ring.len()
    );
    let pane = (
        0.0,
        chart.panes[0].top,
        chart.pane_w,
        chart.panes[0].top + chart.panes[0].height,
    );
    let visible = |point: Point| {
        point.0 >= pane.0 && point.0 <= pane.2 && point.1 >= pane.1 && point.1 <= pane.3
    };
    let mut on_screen = 0;
    for pair in ring.windows(2) {
        if visible(pair[0]) && visible(pair[1]) {
            on_screen += 1;
            let middle = ((pair[0].0 + pair[1].0) / 2.0, (pair[0].1 + pair[1].1) / 2.0);
            let sagitta = radius - (middle.0 - center.0).hypot(middle.1 - center.1);
            assert!(sagitta <= 0.25 + 1e-6, "{sagitta}");
        }
    }
    // Uniform 256-chord tessellation would miss the curve by r·(1 − cos(π/256)) > 0.25 px here.
    assert!(radius * (1.0 - (std::f64::consts::PI / 256.0).cos()) > 0.25);
    assert!(
        on_screen >= 4,
        "the visible rim is refined ({on_screen} chords)"
    );
    // Its rim hits across the pane even though the anchors' box barely touches it.
    assert_eq!(hit(&chart, rim), Some(id));
    // A circle entirely off screen paints nothing.
    chart.remove_drawing(id);
    add(
        &mut chart,
        DrawingKind::Circle,
        vec![p(-200.0, 103.0), p(-199.0, 103.0)],
        r##"{"color":"#123456"}"##,
    );
    assert!(polylines(&mut chart, ink()).is_empty());
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

#[test]
fn creation_previews_draw_multi_anchor_shapes_before_the_last_click() {
    let mut chart = chart();
    let modifiers = DrawingModifiers::default();
    // Until only the last anchor of a three- or four-anchor tool remains, the anchors placed so
    // far and the pointer join as one polyline in the drawing's stroke, with handles on the
    // placed anchors.
    for kind in [
        DrawingKind::Triangle,
        DrawingKind::RotatedRectangle,
        DrawingKind::Arc,
        DrawingKind::Curve,
        DrawingKind::DoubleCurve,
    ] {
        assert!(chart.set_drawing_tool(Some(kind), Some(r##"{"color":"#123456"}"##), None));
        chart.drawing_tool_activate(200.0, 250.0, modifiers);
        assert!(chart.drawings().is_empty());
        chart.drawing_tool_pointer_move(400.0, 250.0, modifiers, false);
        let line = ink_line(&mut chart);
        assert!(
            close(line[0], (200.0, 250.0), 1e-3) && close(line[1], (400.0, 250.0), 1e-3),
            "{kind:?}: {line:?}"
        );
        assert_eq!(handle_centers(&mut chart), vec![(200.0, 250.0)], "{kind:?}");
        if kind == DrawingKind::DoubleCurve {
            chart.drawing_tool_activate(400.0, 250.0, modifiers);
            chart.drawing_tool_pointer_move(260.0, 180.0, modifiers, false);
            assert_eq!(
                ink_line(&mut chart),
                vec![(200.0, 250.0), (400.0, 250.0), (260.0, 180.0)]
            );
            assert_eq!(handle_centers(&mut chart).len(), 2);
        }
        assert!(chart.set_drawing_tool(None, None, None));
        assert!(
            polylines(&mut chart, ink()).is_empty(),
            "disarming drops it"
        );
    }

    assert!(chart.set_drawing_tool(
        Some(DrawingKind::Arc),
        Some(r##"{"color":"#123456"}"##),
        None
    ));
    chart.drawing_tool_activate(200.0, 250.0, modifiers);
    chart.drawing_tool_activate(400.0, 250.0, modifiers);
    chart.drawing_tool_pointer_move(300.0, 180.0, modifiers, false);
    let arc = ink_line(&mut chart);
    assert!(arc.len() > 8, "the pending arc bends through the pointer");
    assert!(shape::distance_to_polyline((300.0, 180.0), &arc) < 1.0);

    // A closed polyline template previews its fill while vertices are placed.
    assert!(chart.set_drawing_tool(
        Some(DrawingKind::Polyline),
        Some(r##"{"color":"#123456","tool_options":{"shape":{"closed":true}}}"##),
        None
    ));
    chart.drawing_tool_activate(200.0, 250.0, modifiers);
    chart.drawing_tool_activate(300.0, 150.0, modifiers);
    chart.drawing_tool_pointer_move(380.0, 260.0, modifiers, false);
    assert_eq!(fills(&mut chart, wash()).len(), 1);
    chart.drawing_tool_activate(380.0, 260.0, modifiers);
    let id = chart.drawing_tool_finish().created.unwrap();
    assert_eq!(chart.drawing(id).unwrap().points.len(), 3);
    assert!(
        chart
            .drawing(id)
            .unwrap()
            .tool_options
            .shape
            .unwrap()
            .closed,
        "the template's closed flag is committed"
    );
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

#[test]
fn open_shape_end_caps_follow_the_stroke_ends() {
    let mut chart = chart();
    let id = add(
        &mut chart,
        DrawingKind::Polyline,
        vec![p(10.0, 101.0), p(14.0, 104.0), p(20.0, 102.0)],
        r##"{"color":"#123456","stroke_end":"arrow","stroke_start":"circle"}"##,
    );
    let last = anchor(&chart, id, 2);
    let frame = chart.build_frame();
    let tip = frame.panes[0]
        .main
        .iter()
        .find_map(|prim| match prim {
            Prim::BandFill {
                upper_first, fill, ..
            } if *fill == ink() => Some(frame.panes[0].points[*upper_first as usize]),
            _ => None,
        })
        .expect("arrowhead");
    assert!(close((f64::from(tip[0]), f64::from(tip[1])), last, 1e-3));
    assert!(frame.panes[0]
        .main
        .iter()
        .any(|prim| matches!(prim, Prim::Circle { fill, .. } if *fill == ink())));
    assert_eq!(
        hit(&chart, (last.0 - 4.0, last.1 - 1.0)),
        Some(id),
        "the arrowhead is a body target"
    );
    // Caps never paint on closed shapes.
    assert!(chart.drawing_apply_options(id, r#"{"tool_options":{"shape":{"closed":true}}}"#));
    let frame = chart.build_frame();
    assert!(!frame.panes[0]
        .main
        .iter()
        .any(|prim| matches!(prim, Prim::Circle { fill, .. } if *fill == ink())));
    assert_eq!(
        chart.drawing(id).unwrap().stroke_end,
        DrawingLineCap::Arrow,
        "the cap setting is kept for reopening"
    );
}
