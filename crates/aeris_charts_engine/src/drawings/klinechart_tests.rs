//! The drawing tools ported from KLineChart's overlays: catalog identity, geometry against the
//! KLineChart templates, linked anchors, creation, hit-testing, frame output, axis tags, and
//! persistence.

use aeris_charts_render::draw_list::Prim;

use super::*;

const OVERLAY_KINDS: [DrawingKind; 11] = [
    DrawingKind::StraightLine,
    DrawingKind::RayLine,
    DrawingKind::HorizontalSegment,
    DrawingKind::VerticalRay,
    DrawingKind::VerticalSegment,
    DrawingKind::PriceLine,
    DrawingKind::ParallelLine,
    DrawingKind::PriceChannel,
    DrawingKind::FibonacciLine,
    DrawingKind::SimpleAnnotation,
    DrawingKind::SimpleTag,
];

/// One candle series over 10 hourly bars with a settled 800×500 layout at dpr 1.
fn settled_chart() -> ChartEngine {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    let times = (0..10).map(|i| (i * 3600) as f64).collect::<Vec<_>>();
    let values = [11.0, 12.0, 11.0, 10.0, 11.0, 12.0, 13.0, 12.0, 11.0, 10.0];
    chart
        .set_series_data(0, &times, &values, &values, &values, &values)
        .unwrap();
    chart.time_scale.set_width(800.0);
    chart.fit_content();
    chart.build_frame();
    chart
}

fn point(logical: f64, price: f64) -> DrawingPoint {
    DrawingPoint { logical, price }
}

fn px(chart: &ChartEngine, logical: f64, price: f64) -> (f64, f64) {
    (
        chart.logical_to_coordinate(logical).unwrap(),
        chart.series_price_to_coordinate(0, price).unwrap(),
    )
}

/// Representative anchors for each kind.
fn anchors(kind: DrawingKind) -> Vec<DrawingPoint> {
    match kind.anchor_count() {
        1 => vec![point(4.0, 11.5)],
        2 => vec![point(2.0, 10.5), point(6.0, 12.5)],
        _ => vec![point(2.0, 10.5), point(6.0, 12.0), point(3.0, 12.5)],
    }
}

fn geometry(kind: DrawingKind, px: &[(f64, f64)]) -> DrawingBodyGeometry<'_> {
    resolve_drawing_geometry(
        kind,
        px,
        100.0,
        0.0,
        200.0,
        DrawingGeometryOptions {
            line_width: 1.0,
            device_scale: 2.0,
            ..Default::default()
        },
    )
    .expect("geometry")
    .body
}

#[test]
fn overlay_tools_have_unique_contiguous_wire_ids_and_names() {
    let mut names = std::collections::HashSet::new();
    for (index, spec) in DRAWING_TOOL_SPECS.iter().enumerate() {
        assert_eq!(usize::from(spec.wire_id), index, "{}", spec.name);
        assert_eq!(spec.kind.spec().wire_id, spec.wire_id);
        assert!(names.insert(spec.name), "duplicate name {}", spec.name);
        assert_eq!(DrawingKind::from_u8(spec.wire_id), Some(spec.kind));
        assert_eq!(DrawingKind::from_name(spec.name), Some(spec.kind));
        assert!(usize::from(spec.preview_points) <= spec.placement.minimum_points());
    }
    for kind in OVERLAY_KINDS {
        let json = serde_json::to_string(&kind).unwrap();
        assert_eq!(json, format!("\"{}\"", kind.name()));
    }
}

#[test]
fn line_geometry_follows_the_klinechart_templates() {
    let segment = |body: DrawingBodyGeometry| match body {
        DrawingBodyGeometry::Segment { a, b } => (a, b),
        other => panic!("expected a segment, got {other:?}"),
    };
    // straightLine: across the pane, or the full height when vertical.
    let (a, b) = segment(geometry(
        DrawingKind::StraightLine,
        &[(10.0, 20.0), (30.0, 40.0)],
    ));
    assert_eq!((a, b), ((0.0, 10.0), (100.0, 110.0)));
    let (a, b) = segment(geometry(
        DrawingKind::StraightLine,
        &[(50.0, 20.0), (50.0, 80.0)],
    ));
    assert_eq!((a, b), ((50.0, 0.0), (50.0, 200.0)));

    // rayLine: from the first anchor through the second to the edge on its side.
    let ray = |a: (f64, f64), b: (f64, f64)| segment(geometry(DrawingKind::RayLine, &[a, b])).1;
    assert_eq!(ray((10.0, 20.0), (30.0, 40.0)), (100.0, 110.0));
    assert_eq!(ray((30.0, 40.0), (10.0, 20.0)), (0.0, 10.0));
    assert_eq!(ray((50.0, 20.0), (50.0, 80.0)), (50.0, 200.0));
    assert_eq!(ray((50.0, 80.0), (50.0, 20.0)), (50.0, 0.0));

    // Segments and rays that stay horizontal or vertical.
    assert!(matches!(
        geometry(DrawingKind::HorizontalSegment, &[(60.0, 20.0), (10.0, 40.0)]),
        DrawingBodyGeometry::Horizontal { y, x0, x1 } if (y, x0, x1) == (40.0, 10.0, 60.0)
    ));
    assert!(matches!(
        geometry(DrawingKind::VerticalRay, &[(50.0, 20.0), (50.0, 80.0)]),
        DrawingBodyGeometry::Vertical { x, y0, y1 } if (x, y0, y1) == (50.0, 20.0, 200.0)
    ));
    assert!(matches!(
        geometry(DrawingKind::VerticalRay, &[(50.0, 80.0), (50.0, 20.0)]),
        DrawingBodyGeometry::Vertical { y1, .. } if y1 == 0.0
    ));
    assert!(matches!(
        geometry(DrawingKind::VerticalSegment, &[(50.0, 80.0), (50.0, 20.0)]),
        DrawingBodyGeometry::Vertical { x, y0, y1 } if (x, y0, y1) == (50.0, 80.0, 20.0)
    ));
    assert!(matches!(
        geometry(DrawingKind::PriceLine, &[(40.0, 30.0)]),
        DrawingBodyGeometry::Horizontal { y, x0, x1 } if (y, x0, x1) == (30.0, 40.0, 100.0)
    ));
    assert!(matches!(
        geometry(DrawingKind::SimpleTag, &[(40.0, 30.0)]),
        DrawingBodyGeometry::Horizontal { y, x0, x1 } if (y, x0, x1) == (30.0, 0.0, 100.0)
    ));
}

#[test]
fn channel_fibonacci_and_annotation_geometry_follow_the_klinechart_templates() {
    let lines = |kind, px: &[(f64, f64)]| match geometry(kind, px) {
        DrawingBodyGeometry::Lines { lines, count } => lines[..count].to_vec(),
        other => panic!("expected lines, got {other:?}"),
    };
    let three = [(0.0, 10.0), (10.0, 20.0), (0.0, 30.0)];
    // getParallelLines: y = x + 10 and its parallel through (0, 30).
    assert_eq!(
        lines(DrawingKind::ParallelLine, &three),
        [((0.0, 10.0), (100.0, 110.0)), ((0.0, 30.0), (100.0, 130.0))]
    );
    // The channel mirrors the parallel on the far side: intercept 10 + (10 - 30).
    assert_eq!(
        lines(DrawingKind::PriceChannel, &three)[2],
        ((0.0, -10.0), (100.0, 90.0))
    );
    // Two anchors already draw the first line while the third is being placed.
    assert_eq!(lines(DrawingKind::PriceChannel, &three[..2]).len(), 1);
    // Vertical channels step horizontally.
    assert_eq!(
        lines(
            DrawingKind::PriceChannel,
            &[(50.0, 10.0), (50.0, 90.0), (40.0, 30.0)]
        ),
        [
            ((50.0, 0.0), (50.0, 200.0)),
            ((40.0, 0.0), (40.0, 200.0)),
            ((60.0, 0.0), (60.0, 200.0))
        ]
    );

    let DrawingBodyGeometry::Fibonacci { x0, x1, y100, y0 } =
        geometry(DrawingKind::FibonacciLine, &[(20.0, 150.0), (60.0, 50.0)])
    else {
        panic!("fibonacci geometry");
    };
    assert_eq!((x0, x1), (0.0, 100.0));
    let levels = FIBONACCI_LEVELS.map(|level| DrawingBodyGeometry::fibonacci_y(y100, y0, level));
    assert_eq!(levels[0], 150.0);
    assert_eq!(levels[3], 100.0);
    assert_eq!(levels[6], 50.0);

    // simpleAnnotation at device scale 2: stem 6..56 px above the anchor, 5 px head, 8 px wide.
    let DrawingBodyGeometry::Annotation(annotation) =
        geometry(DrawingKind::SimpleAnnotation, &[(40.0, 150.0)])
    else {
        panic!("annotation geometry");
    };
    assert_eq!(annotation.stem_bottom, 138.0);
    assert_eq!(annotation.stem_top, 38.0);
    assert_eq!(annotation.head_top, 28.0);
    assert_eq!(annotation.head_half_width, 8.0);
}

#[test]
fn linked_anchors_share_their_price_or_bar() {
    let mut chart = settled_chart();
    let horizontal = chart
        .add_drawing(
            DrawingKind::HorizontalSegment,
            0,
            vec![point(2.0, 10.5), point(6.0, 12.5)],
            None,
        )
        .unwrap();
    // The newest anchor sets the shared price.
    let points = chart.drawing(horizontal).unwrap().points.clone();
    assert_eq!(points, vec![point(2.0, 12.5), point(6.0, 12.5)]);

    // Dragging either anchor moves the price of both.
    let (x, y) = px(&chart, 2.0, 12.5);
    chart.set_selected_drawing(Some(horizontal));
    assert!(chart.drawing_drag_start_at(x, y));
    chart.drawing_drag_to(x, y + 40.0, DrawingModifiers::default());
    chart.drawing_drag_end();
    let points = &chart.drawing(horizontal).unwrap().points;
    assert_eq!(points[0].price, points[1].price);
    assert!(points[0].price < 12.5);
    assert_eq!(points[1].logical, 6.0);

    let vertical = chart
        .add_drawing(
            DrawingKind::VerticalSegment,
            0,
            vec![point(2.0, 10.5), point(6.0, 12.5)],
            None,
        )
        .unwrap();
    let points = &chart.drawing(vertical).unwrap().points;
    assert_eq!(points[0].logical, 6.0);
    assert_eq!(points[1].logical, 6.0);
    assert_eq!(points[0].price, 10.5);
}

#[test]
fn every_overlay_is_created_by_clicking_its_anchors() {
    for kind in OVERLAY_KINDS {
        let mut chart = settled_chart();
        assert!(chart.drawing_create_begin(kind, None), "{kind:?}");
        let anchors = anchors(kind);
        let mut committed = 0;
        for anchor in &anchors {
            let (x, y) = px(&chart, anchor.logical, anchor.price);
            chart.drawing_create_move(x, y, DrawingModifiers::default());
            chart.build_frame();
            committed = chart.drawing_create_click(x, y, DrawingModifiers::default());
        }
        assert!(committed > 0, "{kind:?} commits after its last anchor");
        let drawing = chart.drawing(committed as DrawingId).unwrap();
        assert_eq!(drawing.kind, kind);
        assert_eq!(drawing.points.len(), anchors.len());
        chart.build_frame();
    }
}

/// A pane-wide straight stroke in a frame pane: the lines of channels and straight lines.
fn has_pane_wide_line(frame: &crate::ChartFrame) -> bool {
    let pane = &frame.panes[0];
    pane.main.iter().any(|prim| match prim {
        Prim::Polyline {
            first_point,
            point_count: 2,
            ..
        } => {
            let first = pane.points[*first_point as usize];
            let second = pane.points[*first_point as usize + 1];
            first[0] == 0.0 && second[0] >= 790.0
        }
        _ => false,
    })
}

#[test]
fn channels_preview_their_first_line_before_the_third_anchor() {
    for kind in [DrawingKind::ParallelLine, DrawingKind::PriceChannel] {
        let mut chart = settled_chart();
        assert!(chart.drawing_create_begin(kind, None));
        let (x, y) = px(&chart, 2.0, 10.5);
        chart.drawing_create_move(x, y, DrawingModifiers::default());
        assert!(!has_pane_wide_line(&chart.build_frame()), "{kind:?}");
        chart.drawing_create_click(x, y, DrawingModifiers::default());
        let (x, y) = px(&chart, 6.0, 12.0);
        chart.drawing_create_move(x, y, DrawingModifiers::default());
        assert!(has_pane_wide_line(&chart.build_frame()), "{kind:?}");
    }
}

#[test]
fn every_overlay_body_is_hit_where_it_is_drawn() {
    let chart_with = |kind: DrawingKind| {
        let mut chart = settled_chart();
        let id = chart.add_drawing(kind, 0, anchors(kind), None).unwrap();
        chart.build_frame();
        (chart, id)
    };
    let hit = |chart: &ChartEngine, (x, y): (f64, f64)| {
        chart.hit_test_drawing(x, y).map(|hit| (hit.id, hit.part))
    };
    for kind in OVERLAY_KINDS {
        let (chart, id) = chart_with(kind);
        let a = px(&chart, 2.0, 10.5);
        let b = px(&chart, 6.0, 12.5);
        let on_body = match kind {
            // Beyond both anchors, still on the extended line.
            DrawingKind::StraightLine | DrawingKind::RayLine => {
                let x = b.0 + 100.0;
                (x, a.1 + (x - a.0) * (b.1 - a.1) / (b.0 - a.0))
            }
            DrawingKind::HorizontalSegment => ((a.0 + b.0) / 2.0, b.1),
            DrawingKind::VerticalRay => (b.0, 5.0),
            DrawingKind::VerticalSegment => (b.0, (a.1 + b.1) / 2.0),
            DrawingKind::PriceLine | DrawingKind::SimpleTag => {
                let (x, y) = px(&chart, 4.0, 11.5);
                (x + 150.0, y)
            }
            DrawingKind::ParallelLine | DrawingKind::PriceChannel => {
                // On the parallel through the third anchor, away from it.
                let c = px(&chart, 3.0, 12.5);
                let second = px(&chart, 6.0, 12.0);
                let slope = (second.1 - a.1) / (second.0 - a.0);
                let x = c.0 + 200.0;
                (x, c.1 + (x - c.0) * slope)
            }
            DrawingKind::FibonacciLine => (700.0, (a.1 + b.1) / 2.0),
            DrawingKind::SimpleAnnotation => {
                let (x, y) = px(&chart, 4.0, 11.5);
                (x, y - 30.0)
            }
            _ => unreachable!(),
        };
        assert_eq!(
            hit(&chart, on_body),
            Some((id, DrawingDragPart::Body)),
            "{kind:?} at {on_body:?}"
        );
        // Step off the body across it: sideways from vertical strokes, vertically otherwise.
        let off_body = match kind {
            DrawingKind::VerticalRay
            | DrawingKind::VerticalSegment
            | DrawingKind::SimpleAnnotation => (on_body.0 + 25.0, on_body.1),
            // Between the 50% and 61.8% levels.
            DrawingKind::FibonacciLine => (on_body.0, b.1 + (a.1 - b.1) * 0.559),
            _ => (on_body.0, on_body.1 + 25.0),
        };
        assert_eq!(hit(&chart, off_body), None, "{kind:?} off the body");
    }
}

#[test]
fn fibonacci_levels_and_price_lines_print_their_prices() {
    let mut chart = settled_chart();
    chart
        .add_drawing(
            DrawingKind::FibonacciLine,
            0,
            vec![point(2.0, 12.0), point(6.0, 10.0)],
            None,
        )
        .unwrap();
    chart
        .add_drawing(DrawingKind::PriceLine, 0, vec![point(4.0, 11.25)], None)
        .unwrap();
    let frame = chart.build_frame();
    let texts = frame.panes[0]
        .main
        .iter()
        .chain(&frame.panes[0].top_prims)
        .filter_map(|prim| match prim {
            Prim::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>();
    for expected in [
        "12.00 (100.0%)",
        "11.57 (78.6%)",
        "11.24 (61.8%)",
        "11.00 (50.0%)",
        "10.76 (38.2%)",
        "10.47 (23.6%)",
        "10.00 (0.0%)",
        "11.25",
    ] {
        assert!(texts.contains(&expected), "{expected} in {texts:?}");
    }
}

#[test]
fn tags_and_price_lines_label_the_price_axis() {
    let mut chart = settled_chart();
    let tag = chart
        .add_drawing(DrawingKind::SimpleTag, 0, vec![point(4.0, 12.0)], None)
        .unwrap();
    chart
        .add_drawing(DrawingKind::PriceLine, 0, vec![point(4.0, 11.0)], None)
        .unwrap();
    let axis_texts = |chart: &mut ChartEngine| {
        chart
            .build_axis_frame(
                80.0,
                |t, _bold| t.len() as f64 * 7.0,
                |t, _bold| t.len() as f64 * 6.0,
            )
            .labels
            .into_iter()
            .filter(|label| label.background.is_some())
            .map(|label| label.text)
            .collect::<Vec<_>>()
    };
    let texts = axis_texts(&mut chart);
    assert!(texts.contains(&"12.00".to_owned()), "{texts:?}");
    assert!(texts.contains(&"11.00".to_owned()), "{texts:?}");

    // A tag with text shows the text instead of its price, on the axis only.
    assert!(chart.drawing_apply_options(tag, r#"{"text":"Support"}"#));
    let texts = axis_texts(&mut chart);
    assert!(texts.contains(&"Support".to_owned()), "{texts:?}");
    assert!(!texts.contains(&"12.00".to_owned()), "{texts:?}");
    let frame = chart.build_frame();
    assert!(!frame.panes[0].main.iter().any(
        |prim| matches!(prim, Prim::Text { text, .. } | Prim::RotatedText { text, .. } if text == "Support")
    ));
}

#[test]
fn overlay_drawings_round_trip_through_persistence() {
    let mut chart = settled_chart();
    for kind in OVERLAY_KINDS {
        chart
            .add_drawing(kind, 0, anchors(kind), Some(r#"{"text":"note"}"#))
            .unwrap();
    }
    let document = chart.export_state_json().unwrap();
    let mut restored = settled_chart();
    restored.import_state_json(&document).unwrap();
    let kinds = restored
        .drawings()
        .iter()
        .map(|drawing| (drawing.kind, drawing.points.clone(), drawing.style))
        .collect::<Vec<_>>();
    let expected = chart
        .drawings()
        .iter()
        .map(|drawing| (drawing.kind, drawing.points.clone(), drawing.style))
        .collect::<Vec<_>>();
    assert_eq!(kinds, expected);
}

#[test]
fn upward_vertical_rays_and_segments_paint_top_to_bottom() {
    let mut chart = settled_chart();
    chart
        .add_drawing(
            DrawingKind::VerticalSegment,
            0,
            vec![point(4.0, 10.5), point(4.0, 12.5)],
            None,
        )
        .unwrap();
    let frame = chart.build_frame();
    let x = chart.logical_to_coordinate(4.0).unwrap().round() as i32;
    let (top, bottom) = (
        chart.series_price_to_coordinate(0, 12.5).unwrap().round() as i32,
        chart.series_price_to_coordinate(0, 10.5).unwrap().round() as i32,
    );
    assert!(frame.panes[0].main.iter().any(|prim| matches!(
        prim,
        Prim::VLine { x: line_x, y0, y1, .. } if *line_x == x && (*y0, *y1) == (top, bottom)
    )));
}
