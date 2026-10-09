//! Lines-family engine tests: catalog defaults, armed placement, shared-part frames and hit
//! testing (indexed and brute force), drags, keyboard nudges, magnet, time identity, schema and
//! kind options, patches with history, persistence, clipboard, and sync.

use aeris_charts_render::color::Color;
use aeris_charts_render::draw_list::{LineStyle, Prim};

use super::super::super::tools::DrawingAnchorLink;
use super::super::super::{
    DrawingPlacement, DrawingStraightenMode, DrawingTextHAlign, DrawingTextLayout,
    DrawingTextVAlign,
};
use crate::{
    ChartEngine, DrawingAnchor, DrawingId, DrawingKind, DrawingLineCap, DrawingMagnetMode,
    DrawingModifiers, DrawingPoint, DrawingStatsPosition,
};

const LINE_KINDS: [DrawingKind; 6] = [
    DrawingKind::Ray,
    DrawingKind::ExtendedLine,
    DrawingKind::InfoLine,
    DrawingKind::TrendAngle,
    DrawingKind::CrossLine,
    DrawingKind::ArrowLine,
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

fn add(
    chart: &mut ChartEngine,
    kind: DrawingKind,
    points: Vec<DrawingPoint>,
    options: &str,
) -> DrawingId {
    chart.add_drawing(kind, 0, points, Some(options)).unwrap()
}

fn two_points(kind: DrawingKind) -> Vec<DrawingPoint> {
    if kind == DrawingKind::CrossLine {
        vec![p(12.0, 102.0)]
    } else {
        vec![p(10.0, 101.0), p(20.0, 104.0)]
    }
}

fn anchor(chart: &ChartEngine, id: DrawingId, index: usize) -> (f64, f64) {
    chart.drawing_point_to_coordinate(id, index).unwrap()
}

fn ink() -> Color {
    Color::parse_css(INK).unwrap()
}

/// One drawing-colored polyline of the first pane: points, width, and style.
type InkLine = (Vec<(f64, f64)>, f32, LineStyle);

fn ink_polylines(chart: &mut ChartEngine) -> Vec<InkLine> {
    let frame = chart.build_frame();
    let pane = &frame.panes[0];
    pane.main
        .iter()
        .filter_map(|prim| match prim {
            Prim::Polyline {
                first_point,
                point_count,
                width,
                style,
                color,
                ..
            } if *color == ink() => Some((
                pane.points[*first_point as usize..(*first_point + *point_count) as usize]
                    .iter()
                    .map(|point| (f64::from(point[0]), f64::from(point[1])))
                    .collect(),
                *width,
                *style,
            )),
            _ => None,
        })
        .collect()
}

fn texts(chart: &mut ChartEngine) -> Vec<(String, f32, f32)> {
    let frame = chart.build_frame();
    frame.panes[0]
        .main
        .iter()
        .filter_map(|prim| match prim {
            Prim::Text { text, x, y, .. } => Some((text.clone(), *x, *y)),
            _ => None,
        })
        .collect()
}

fn close(a: (f64, f64), b: (f64, f64), tolerance: f64) -> bool {
    (a.0 - b.0).abs() <= tolerance && (a.1 - b.1).abs() <= tolerance
}

/// The fork's pre-merge defaults of the upstream line tools, which documents it wrote omitted,
/// come back through `apply_legacy_fork_defaults` (upstream renders these tools now).
#[test]
fn catalog_defaults_follow_each_tool() {
    for kind in LINE_KINDS {
        let mut drawing = crate::Drawing::new(1, kind, 0, Vec::new());
        super::super::apply_legacy_fork_defaults(&mut drawing);
        let spec = kind.spec();
        assert!(spec.family.is_none(), "{kind:?} is upstream-rendered");
        assert_eq!(spec.axis_price_label, kind == DrawingKind::CrossLine);
        if kind == DrawingKind::CrossLine {
            assert_eq!(spec.placement, DrawingPlacement::ClickAnchors { count: 1 });
            assert_eq!(spec.text_layout, DrawingTextLayout::Box);
            assert_eq!(drawing.text_h_align, DrawingTextHAlign::Center);
        } else {
            assert_eq!(spec.placement, DrawingPlacement::ClickAnchors { count: 2 });
            assert_eq!(spec.text_layout, DrawingTextLayout::Segment);
            assert_eq!(drawing.width, 2.0);
            assert_eq!(
                (drawing.text_h_align, drawing.text_v_align),
                (DrawingTextHAlign::Right, DrawingTextVAlign::Top),
                "segment labels default to the top-right slot"
            );
        }
        assert_eq!(
            (drawing.extend_left, drawing.extend_right),
            match kind {
                DrawingKind::Ray => (false, true),
                DrawingKind::ExtendedLine => (true, true),
                _ => (false, false),
            }
        );
        assert_eq!(
            drawing.stroke_end,
            if kind == DrawingKind::ArrowLine {
                DrawingLineCap::Arrow
            } else {
                DrawingLineCap::None
            }
        );
        assert_eq!(
            drawing.labels.len(),
            if kind == DrawingKind::InfoLine { 5 } else { 0 }
        );
        assert!(drawing.tool_options.is_empty());
    }
}

#[test]
fn armed_tools_place_every_lines_kind() {
    let mut chart = chart();
    for kind in LINE_KINDS {
        assert!(chart.set_drawing_tool(Some(kind), Some(r##"{"color":"#123456"}"##), None));
        let clicks = if kind == DrawingKind::CrossLine {
            vec![(300.0, 200.0)]
        } else {
            vec![(200.0, 260.0), (420.0, 150.0)]
        };
        let mut created = None;
        for (x, y) in clicks {
            created = chart
                .drawing_tool_activate(x, y, DrawingModifiers::default())
                .created;
        }
        let id = created.unwrap_or_else(|| panic!("{kind:?} committed"));
        let drawing = chart.drawing(id).unwrap();
        assert_eq!(drawing.kind, kind);
        assert_eq!(drawing.points.len(), kind.anchor_count());
        assert_eq!(drawing.color, INK);
        assert_eq!(chart.active_drawing_tool(), None, "one-shot tools disarm");
        assert_eq!(chart.selected_drawing(), Some(id));
    }
}

#[test]
fn cross_lines_span_both_axes_and_tag_their_price() {
    let mut chart = chart();
    let id = add(
        &mut chart,
        DrawingKind::CrossLine,
        vec![p(12.0, 102.0)],
        r##"{"color":"#123456"}"##,
    );
    let (x, y) = anchor(&chart, id, 0);
    let frame = chart.build_frame();
    let pane_w = chart.pane_w.round() as i32;
    assert!(frame.panes[0].main.iter().any(|prim| matches!(
        prim,
        Prim::HLine { x0: 0, x1, color, .. } if *x1 == pane_w && *color == ink()
    )));
    assert!(frame.panes[0].main.iter().any(|prim| matches!(
        prim,
        Prim::VLine { x: vx, color, .. } if *vx == x.round() as i32 && *color == ink()
    )));
    let axis = chart.build_axis_frame(
        200.0,
        |text, _| text.len() as f64 * 6.0,
        |text, _| text.len() as f64 * 5.0,
    );
    assert!(axis.labels.iter().any(|label| label.text == "102.00"));
    // Either line is a body target far from the anchor.
    assert!(chart.hit_test_drawing(20.0, y).is_some());
    assert!(chart
        .hit_test_drawing(x, chart.panes[0].top + 5.0)
        .is_some());
    // A body drag moves both coordinates.
    assert!(chart.drawing_drag_start_at(20.0, y));
    chart.drawing_drag_to(60.0, y - 40.0, DrawingModifiers::default());
    chart.drawing_drag_end();
    let moved = anchor(&chart, id, 0);
    assert!(close(moved, (x + 40.0, y - 40.0), 1e-6));
}

#[test]
fn indexed_hit_testing_matches_brute_force_including_extensions() {
    let mut chart = chart();
    for copy in 0..6 {
        let shift = copy as f64 * 0.7;
        for kind in LINE_KINDS {
            let points = two_points(kind)
                .into_iter()
                .map(|point| p(point.logical + shift, point.price + shift * 0.3))
                .collect();
            add(&mut chart, kind, points, "{}");
        }
    }
    assert!(
        chart.drawings().len() > 20,
        "exercises the culled candidate path"
    );
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
    assert!(hits > 100, "the grid meets the drawings ({hits} hits)");
}

#[test]
fn extended_trend_lines_hit_beyond_their_anchors() {
    let mut chart = chart();
    let id = add(
        &mut chart,
        DrawingKind::TrendLine,
        vec![p(2.0, 103.0), p(4.0, 103.05)],
        r#"{"extend_right":true}"#,
    );
    let (a, b) = (anchor(&chart, id, 0), anchor(&chart, id, 1));
    let x = chart.pane_w - 20.0;
    let y = a.1 + (b.1 - a.1) * (x - a.0) / (b.0 - a.0);
    assert!(y > chart.panes[0].top && y < chart.panes[0].top + chart.panes[0].height);
    assert_eq!(chart.hit_test_drawing(x, y).map(|hit| hit.id), Some(id));
}

#[test]
fn drags_nudges_and_straighten_edit_line_tools_as_single_history_entries() {
    let mut chart = chart();
    let id = add(
        &mut chart,
        DrawingKind::Ray,
        vec![p(10.0, 101.0), p(20.0, 104.0)],
        "{}",
    );
    let before = chart.drawing(id).unwrap().points.clone();
    chart.set_selected_drawing(Some(id));
    let (bx, by) = anchor(&chart, id, 1);
    assert!(chart.drawing_drag_start_at(bx, by));
    chart.drawing_drag_to(
        bx + 30.0,
        by + 7.0,
        DrawingModifiers {
            magnet: false,
            straighten: true,
        },
    );
    chart.drawing_drag_end();
    let (ax, ay) = anchor(&chart, id, 0);
    let (nx, ny) = anchor(&chart, id, 1);
    assert_eq!(
        chart.drawing(id).unwrap().points[0],
        before[0],
        "only the dragged anchor moves"
    );
    let angle = (ay - ny).atan2(nx - ax).to_degrees();
    assert!(
        (angle / 45.0 - (angle / 45.0).round()).abs() < 1e-6,
        "Shift snaps to 45° steps"
    );
    assert!(chart.undo_drawing());
    assert_eq!(chart.drawing(id).unwrap().points, before);

    // Keyboard handles: two anchors on segment tools, one on the cross line.
    assert_eq!(chart.drawing_handle_count(id), Some(2));
    let cross = add(
        &mut chart,
        DrawingKind::CrossLine,
        vec![p(12.0, 102.0)],
        "{}",
    );
    assert_eq!(chart.drawing_handle_count(cross), Some(1));
    chart.set_selected_drawing(Some(id));
    let (x0, y0) = anchor(&chart, id, 0);
    assert!(chart.nudge_selected_drawing(10.0, 0.0, Some(0)));
    let (x1, y1) = anchor(&chart, id, 0);
    assert!((x1 - x0 - 10.0).abs() < 1e-6 && (y1 - y0).abs() < 1e-6);
    assert!(chart.undo_drawing());
    assert_eq!(chart.drawing(id).unwrap().points, before);
}

#[test]
fn chart_magnet_snaps_cross_line_placement_to_bar_values() {
    let mut chart = chart();
    chart.set_drawing_magnet_mode(DrawingMagnetMode::Strong);
    assert!(chart.set_drawing_tool(Some(DrawingKind::CrossLine), None, None));
    let x = chart.logical_to_coordinate(12.0).unwrap() + 3.0;
    let y = chart.series_price_to_coordinate(0, 102.4).unwrap();
    let id = chart
        .drawing_tool_activate(x, y, DrawingModifiers::default())
        .created
        .unwrap();
    let point = chart.drawing(id).unwrap().points[0];
    assert_eq!(point.logical, 12.0);
    assert_eq!(point.price, 100.0 + (12 % 7) as f64);
}

#[test]
fn line_anchors_resolve_by_time_across_an_interval_switch() {
    let mut chart = chart();
    let id = chart
        .add_drawing_anchors(
            DrawingKind::Ray,
            0,
            &[
                DrawingAnchor {
                    logical: None,
                    price: 101.0,
                    time: Some(10.0 * HOUR),
                },
                DrawingAnchor {
                    logical: None,
                    price: 103.0,
                    time: Some(20.5 * HOUR),
                },
            ],
            None,
        )
        .unwrap();
    let half_hourly = (0..80)
        .map(|index| index as f64 * HOUR / 2.0)
        .collect::<Vec<_>>();
    let values = vec![100.0; half_hourly.len()];
    chart
        .set_series_data(0, &half_hourly, &values, &values, &values, &values)
        .unwrap();
    let anchors = chart.drawing_anchors(id).unwrap();
    assert_eq!(anchors[0].logical, Some(20.0));
    assert_eq!(anchors[1].logical, Some(41.0));
    assert_eq!(anchors[1].time, Some(20.5 * HOUR));
}

#[test]
fn schema_kind_options_and_tool_option_patches_are_typed_and_atomic() {
    let schema = crate::drawing_property_schema(DrawingKind::VerticalRay);
    let default_of = |name: &str| {
        schema
            .properties
            .iter()
            .find(|property| property.name == name)
            .unwrap_or_else(|| panic!("{name} descriptor"))
            .default
            .clone()
    };
    assert_eq!(default_of("extend_right"), serde_json::json!(true));
    assert_eq!(default_of("text_h_align"), serde_json::json!("right"));
    let stats = schema
        .properties
        .iter()
        .find(|property| property.name == "tool_options.line.stats_position")
        .unwrap();
    assert_eq!(stats.default, serde_json::json!("end"));
    assert_eq!(stats.enum_values, ["start", "middle", "end"]);
    let arrow = crate::drawing_property_schema(DrawingKind::ArrowLine);
    assert!(arrow
        .properties
        .iter()
        .any(|property| property.name == "stroke_end" && property.default == "arrow"));
    // Core tools keep their historical schema.
    assert!(!crate::drawing_property_schema(DrawingKind::TrendLine)
        .properties
        .iter()
        .any(|property| property.name.starts_with("tool_options")));

    let mut chart = chart();
    let id = add(
        &mut chart,
        DrawingKind::HorizontalSegment,
        two_points(DrawingKind::HorizontalSegment),
        "{}",
    );
    let kind_options = || {
        serde_json::from_str::<serde_json::Value>(&chart.drawing_kind_options_json(id).unwrap())
            .unwrap()
    };
    assert_eq!(
        kind_options(),
        serde_json::json!({"kind": "line", "stats_position": "end"})
    );
    assert!(chart.drawing_apply_options(
        id,
        r#"{"tool_options":{"line":{"stats_position":"middle"}},"width":3}"#
    ));
    assert_eq!(
        chart
            .drawing(id)
            .unwrap()
            .tool_options
            .line
            .unwrap()
            .stats_position,
        DrawingStatsPosition::Middle
    );
    // An invalid block rejects the whole patch.
    let before = chart.drawing(id).unwrap().clone();
    for invalid in [
        r#"{"tool_options":{"line":{"stats_position":"sideways"}},"width":9}"#,
        r#"{"tool_options":7,"width":9}"#,
        r#"{"tool_options":{"line":7}}"#,
    ] {
        assert!(!chart.drawing_apply_options(id, invalid), "{invalid}");
        assert_eq!(chart.drawing(id).unwrap(), &before);
    }
    // Absent keys keep their values, `null` resets the block, and each change is one undo step.
    assert!(chart.drawing_apply_options(id, r#"{"tool_options":{}}"#));
    assert_eq!(chart.drawing(id).unwrap(), &before);
    assert!(chart.drawing_apply_options(id, r#"{"tool_options":{"line":null}}"#));
    assert!(chart.drawing(id).unwrap().tool_options.is_empty());
    assert!(chart.undo_drawing());
    assert_eq!(chart.drawing(id).unwrap().tool_options, before.tool_options);
    let options: serde_json::Value =
        serde_json::from_str(&chart.drawing_options_json(id).unwrap()).unwrap();
    assert_eq!(
        options["tool_options"],
        serde_json::json!({"line": {"stats_position": "middle"}})
    );
}

#[test]
fn persistence_round_trips_line_tools_and_omits_kind_defaults() {
    let mut chart = chart();
    let defaults = LINE_KINDS
        .into_iter()
        .map(|kind| add(&mut chart, kind, two_points(kind), "{}"))
        .collect::<Vec<_>>();
    let exported: serde_json::Value =
        serde_json::from_str(&chart.export_state_json().unwrap()).unwrap();
    for drawing in exported["drawings"].as_array().unwrap() {
        let style = &drawing["style"];
        for field in [
            "extend_left",
            "extend_right",
            "stroke_end",
            "labels",
            "tool_options",
        ] {
            assert!(
                style.get(field).is_none(),
                "{} writes its default {field}",
                drawing["kind"]
            );
        }
    }
    let customized = [
        (
            DrawingKind::Ray,
            r#"{"extend_right":false,"extend_left":true}"#,
        ),
        (DrawingKind::ExtendedLine, r#"{"extend_left":false}"#),
        (
            DrawingKind::InfoLine,
            r#"{"labels":[],"tool_options":{"line":{"stats_position":"start"}}}"#,
        ),
        (
            DrawingKind::TrendAngle,
            r#"{"text":"angle","style":"dashed"}"#,
        ),
        (DrawingKind::CrossLine, r#"{"width":3}"#),
        (
            DrawingKind::ArrowLine,
            r#"{"stroke_end":"none","stroke_start":"arrow"}"#,
        ),
    ];
    for (kind, options) in customized {
        add(&mut chart, kind, two_points(kind), options);
    }
    let document = chart.export_state_json().unwrap();
    let mut restored = ChartEngine::new(800.0, 500.0, 1.0);
    restored.import_state_json(&document).unwrap();
    assert_eq!(restored.export_state_json().unwrap(), document);
    let original = chart.drawings().iter().map(|drawing| {
        let mut drawing = drawing.clone();
        drawing.pending_times.clear();
        drawing
    });
    for (restored, original) in restored.drawings().iter().zip(original) {
        assert_eq!(restored.kind, original.kind);
        assert_eq!(restored.extend_left, original.extend_left);
        assert_eq!(restored.extend_right, original.extend_right);
        assert_eq!(restored.stroke_start, original.stroke_start);
        assert_eq!(restored.stroke_end, original.stroke_end);
        assert_eq!(restored.labels, original.labels);
        assert_eq!(restored.tool_options, original.tool_options);
    }
    assert_eq!(defaults.len(), 6);
}

#[test]
fn clipboard_and_sync_payloads_carry_line_tool_options() {
    let mut source = chart();
    let id = add(
        &mut source,
        DrawingKind::InfoLine,
        two_points(DrawingKind::InfoLine),
        r#"{"tool_options":{"line":{"stats_position":"middle"}},"extend_left":true}"#,
    );
    let copied = source.copy_drawings_json(&[id]).unwrap();
    let mut target = chart();
    let pasted = target.paste_drawings_json(&copied, 0, 0.0, 0.0).unwrap();
    let drawing = target.drawing(pasted[0]).unwrap();
    assert_eq!(drawing.kind, DrawingKind::InfoLine);
    assert!(drawing.extend_left);
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
    assert_eq!(
        mirror.drawings()[0].labels,
        source.drawing(id).unwrap().labels
    );
}

#[test]
fn frames_scale_family_geometry_with_the_device_pixel_ratio() {
    // 1.5 makes the horizontal and vertical bitmap ratios differ (odd pane sizes round apart).
    for dpr in [1.0, 1.5, 2.0] {
        let mut chart = chart_with(&hourly(40), dpr);
        let id = add(
            &mut chart,
            DrawingKind::HorizontalSegment,
            vec![p(10.0, 105.0), p(20.0, 105.0)],
            r##"{"color":"#123456","labels":[{"metric":"bar_count","visible":true,"position":"on"}]}"##,
        );
        let b = anchor(&chart, id, 1);
        let stroke = ink_polylines(&mut chart).remove(0);
        let hpr = (chart.pane_w * dpr).round() / chart.pane_w;
        let vpr = (chart.pane_h * dpr).round() / chart.pane_h;
        assert!((f64::from(stroke.1) - 2.0 * vpr).abs() < 1e-5);
        assert!((stroke.0[1].0 - b.0 * hpr).abs() < 1e-3);
        assert!((stroke.0[1].1 - b.1 * vpr).abs() < 1e-3);
        let (_, x, _) = texts(&mut chart)
            .into_iter()
            .find(|(text, ..)| text == "10 bars")
            .unwrap();
        assert!(f64::from(x) > (b.0 + 8.0) * dpr - 1.0);
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
fn lines_tools_tolerate_charts_without_data_and_degenerate_anchors() {
    let mut empty = ChartEngine::new(800.0, 500.0, 1.0);
    for kind in LINE_KINDS {
        assert!(empty.add_drawing(kind, 0, two_points(kind), None).is_some());
    }
    empty.build_frame();
    assert_eq!(empty.hit_test_drawing(100.0, 100.0), None);

    let mut chart = chart();
    assert!(chart
        .add_drawing(
            DrawingKind::Ray,
            0,
            vec![p(f64::NAN, 1.0), p(2.0, 3.0)],
            None
        )
        .is_none());
    for kind in LINE_KINDS {
        let point = p(12.0, 102.0);
        let points = vec![point; kind.anchor_count()];
        assert!(chart.add_drawing(kind, 0, points, None).is_some());
    }
    let frame = chart.build_frame();
    assert!(frame.panes[0]
        .points
        .iter()
        .all(|point| point[0].is_finite() && point[1].is_finite()));
}

#[test]
fn cross_lines_paint_and_hit_with_their_anchor_scrolled_away() {
    let mut chart = chart();
    crowd(&mut chart);
    let id = add(
        &mut chart,
        DrawingKind::CrossLine,
        vec![p(-60.0, 104.0)],
        r##"{"color":"#123456"}"##,
    );
    let frame = chart.build_frame();
    let (_, y) = anchor(&chart, id, 0);
    assert!(frame.panes[0].main.iter().any(|prim| matches!(
        prim,
        Prim::HLine { y: line_y, color, .. } if *color == ink() && *line_y == y.round() as i32
    )));
    assert_eq!(chart.hit_test_drawing(400.0, y).map(|hit| hit.id), Some(id));
}

#[test]
fn derived_points_share_the_anchor_space_when_bitmap_ratios_differ() {
    // 801 × 1.5 rounds to 1202 bitmap px, so x scales by 1202/801 while y scales by 1.5. Frame
    // construction debug-asserts that `PartContext::point_px` maps every anchor onto its px.
    let mut chart = ChartEngine::new(801.0, 500.0, 1.5);
    let times = hourly(40);
    let values = vec![100.0; times.len()];
    chart
        .set_series_data(0, &times, &values, &values, &values, &values)
        .unwrap();
    chart.time_scale.set_width(801.0);
    chart.fit_content();
    chart.build_frame();
    let id = add(
        &mut chart,
        DrawingKind::InfoLine,
        // Inside the flat series' narrow price range, so the frame does not clip the stroke.
        vec![p(10.0, 99.97), p(20.0, 100.03)],
        r##"{"color":"#123456"}"##,
    );
    let hpr = (chart.pane_w * 1.5).round() / chart.pane_w;
    let vpr = (chart.pane_h * 1.5).round() / chart.pane_h;
    assert!(
        (hpr - vpr).abs() > 1e-4,
        "the ratios differ: {hpr} vs {vpr}"
    );
    let b = anchor(&chart, id, 1);
    let stroke = ink_polylines(&mut chart).remove(0).0;
    assert!((stroke[1].0 - b.0 * hpr).abs() < 1e-3);
    assert!((stroke[1].1 - b.1 * vpr).abs() < 1e-3);
}

/// Axis-locked segments (KLineChart `horizontalSegment`, `verticalRayLine`, `verticalSegment`):
/// the anchors share one price or one bar, taken from the anchor placed last.
const LOCKED_KINDS: [DrawingKind; 3] = [
    DrawingKind::HorizontalSegment,
    DrawingKind::VerticalRay,
    DrawingKind::VerticalSegment,
];

#[test]
fn axis_locked_segments_have_their_own_catalog_entries() {
    for (kind, wire, name, link, extends) in [
        (
            DrawingKind::HorizontalSegment,
            240,
            "horizontal_segment",
            DrawingAnchorLink::SamePrice,
            (false, false),
        ),
        (
            DrawingKind::VerticalRay,
            241,
            "vertical_ray",
            DrawingAnchorLink::SameLogical,
            (false, true),
        ),
        (
            DrawingKind::VerticalSegment,
            242,
            "vertical_segment",
            DrawingAnchorLink::SameLogical,
            (false, false),
        ),
    ] {
        let spec = kind.spec();
        assert_eq!((spec.wire_id, spec.name), (wire, name));
        assert_eq!(DrawingKind::from_u8(wire), Some(kind));
        assert_eq!(DrawingKind::from_name(name), Some(kind));
        assert_eq!(spec.placement, DrawingPlacement::ClickAnchors { count: 2 });
        assert_eq!(spec.anchor_link, link);
        // The lock leaves nothing for Shift to straighten.
        assert_eq!(spec.straighten, DrawingStraightenMode::None);
        assert!(spec.family.is_some(), "{kind:?} is a lines-family tool");
        let drawing = crate::Drawing::new(1, kind, 0, Vec::new());
        assert_eq!((drawing.extend_left, drawing.extend_right), extends);
    }
    // Every other tool leaves its anchors free.
    for spec in super::super::super::DRAWING_TOOL_SPECS {
        assert_eq!(
            spec.anchor_link != DrawingAnchorLink::None,
            LOCKED_KINDS.contains(&spec.kind),
            "{}",
            spec.name
        );
    }
}

#[test]
fn axis_locked_segments_repair_supplied_anchors_to_the_last_one() {
    let mut chart = chart();
    let horizontal = add(
        &mut chart,
        DrawingKind::HorizontalSegment,
        vec![p(5.0, 101.0), p(15.0, 104.0)],
        "{}",
    );
    assert_eq!(
        chart.drawing(horizontal).unwrap().points,
        vec![p(5.0, 104.0), p(15.0, 104.0)]
    );
    let vertical = add(
        &mut chart,
        DrawingKind::VerticalRay,
        vec![p(5.0, 101.0), p(9.0, 104.0)],
        "{}",
    );
    assert_eq!(
        chart.drawing(vertical).unwrap().points,
        vec![p(9.0, 101.0), p(9.0, 104.0)]
    );
    // Replacing the anchors later goes through the same repair, as one undo step.
    assert!(chart.drawing_set_points(
        horizontal,
        r#"[{"logical":6,"price":102},{"logical":16,"price":107}]"#
    ));
    assert_eq!(
        chart.drawing(horizontal).unwrap().points,
        vec![p(6.0, 107.0), p(16.0, 107.0)]
    );
    assert!(chart.undo_drawing());
    assert_eq!(
        chart.drawing(horizontal).unwrap().points,
        vec![p(5.0, 104.0), p(15.0, 104.0)]
    );
}

#[test]
fn placing_an_axis_locked_segment_keeps_its_axis_in_the_preview_and_the_result() {
    for kind in LOCKED_KINDS {
        let mut chart = chart();
        assert!(chart.set_drawing_tool(Some(kind), Some(r##"{"color":"#123456"}"##), None));
        let start = (200.0, 260.0);
        let end = (420.0, 150.0);
        chart.drawing_tool_activate(start.0, start.1, DrawingModifiers::default());
        chart.drawing_tool_pointer_move(end.0, end.1, DrawingModifiers::default(), false);
        let preview = ink_polylines(&mut chart);
        let line = &preview.last().expect("the pending segment previews").0;
        let (first, last) = (line[0], line[line.len() - 1]);
        if kind == DrawingKind::HorizontalSegment {
            assert!(
                (first.1 - last.1).abs() < 0.01,
                "{kind:?} previews flat: {line:?}"
            );
        } else {
            assert!(
                (first.0 - last.0).abs() < 0.01,
                "{kind:?} previews upright: {line:?}"
            );
        }
        let created = chart
            .drawing_tool_activate(end.0, end.1, DrawingModifiers::default())
            .created
            .unwrap_or_else(|| panic!("{kind:?} committed"));
        let points = &chart.drawing(created).unwrap().points;
        if kind == DrawingKind::HorizontalSegment {
            assert_eq!(points[0].price, points[1].price);
            assert_ne!(points[0].logical, points[1].logical);
        } else {
            assert_eq!(points[0].logical, points[1].logical);
            assert_ne!(points[0].price, points[1].price);
        }
    }
}

#[test]
fn dragging_one_anchor_of_an_axis_locked_segment_moves_the_shared_coordinate() {
    let mut chart = chart();
    let horizontal = add(
        &mut chart,
        DrawingKind::HorizontalSegment,
        vec![p(10.0, 102.0), p(20.0, 102.0)],
        "{}",
    );
    chart.set_selected_drawing(Some(horizontal));
    let (bx, by) = anchor(&chart, horizontal, 1);
    assert!(chart.drawing_drag_start_at(bx, by));
    chart.drawing_drag_to(bx + 30.0, by + 25.0, DrawingModifiers::default());
    chart.drawing_drag_end();
    let points = chart.drawing(horizontal).unwrap().points.clone();
    assert_eq!(points[0].price, points[1].price, "the other anchor follows");
    assert_ne!(points[1].price, 102.0, "the price moved");
    assert_eq!(
        points[0].logical, 10.0,
        "only the free coordinate stays per anchor"
    );
    assert!(points[1].logical > 20.0);
    assert!(chart.undo_drawing());
    assert_eq!(
        chart.drawing(horizontal).unwrap().points,
        vec![p(10.0, 102.0), p(20.0, 102.0)]
    );

    let vertical = add(
        &mut chart,
        DrawingKind::VerticalSegment,
        vec![p(12.0, 101.0), p(12.0, 105.0)],
        "{}",
    );
    chart.set_selected_drawing(Some(vertical));
    let (ax, ay) = anchor(&chart, vertical, 0);
    assert!(chart.drawing_drag_start_at(ax, ay));
    chart.drawing_drag_to(ax + 40.0, ay - 10.0, DrawingModifiers::default());
    chart.drawing_drag_end();
    let points = chart.drawing(vertical).unwrap().points.clone();
    assert_eq!(
        points[0].logical, points[1].logical,
        "the other anchor follows"
    );
    assert!(points[0].logical > 12.0);
    assert_eq!(points[1].price, 105.0, "each anchor keeps its own price");
}

#[test]
fn axis_locked_segments_paint_and_extend_like_klinechart() {
    let mut chart = chart();
    let pane_top = chart.panes[0].top;
    let pane_bottom = chart.panes[0].top + chart.panes[0].height;
    let near = |a: f64, b: f64| (a - b).abs() < 0.5;

    let horizontal = add(
        &mut chart,
        DrawingKind::HorizontalSegment,
        vec![p(10.0, 102.0), p(20.0, 107.0)],
        r##"{"color":"#123456"}"##,
    );
    let (a, b) = (anchor(&chart, horizontal, 0), anchor(&chart, horizontal, 1));
    let line = ink_polylines(&mut chart).remove(0).0;
    assert!(
        near(line[0].1, line[1].1) && near(line[0].1, b.1),
        "flat at the last price"
    );
    assert!(
        near(line[0].0, a.0) && near(line[1].0, b.0),
        "between the anchors"
    );
    chart.remove_drawing(horizontal);

    let segment = add(
        &mut chart,
        DrawingKind::VerticalSegment,
        vec![p(12.0, 101.0), p(12.0, 106.0)],
        r##"{"color":"#123456"}"##,
    );
    let (a, b) = (anchor(&chart, segment, 0), anchor(&chart, segment, 1));
    let line = ink_polylines(&mut chart).remove(0).0;
    assert!(near(line[0].0, line[1].0));
    assert!(
        near(line[0].1, a.1) && near(line[1].1, b.1),
        "between the anchors only"
    );
    chart.remove_drawing(segment);

    // A vertical ray runs from its first anchor through the second to the pane edge on the
    // second anchor's side.
    let downward = add(
        &mut chart,
        DrawingKind::VerticalRay,
        vec![p(12.0, 106.0), p(12.0, 103.0)],
        r##"{"color":"#123456"}"##,
    );
    let (a, b) = (anchor(&chart, downward, 0), anchor(&chart, downward, 1));
    assert!(b.1 > a.1, "the second anchor sits below the first");
    let line = ink_polylines(&mut chart).remove(0).0;
    assert!(
        near(line[0].1, a.1) && near(line[1].1, pane_bottom),
        "{line:?}"
    );
    chart.remove_drawing(downward);
    let upward = add(
        &mut chart,
        DrawingKind::VerticalRay,
        vec![p(12.0, 103.0), p(12.0, 106.0)],
        r##"{"color":"#123456"}"##,
    );
    let a = anchor(&chart, upward, 0);
    let line = ink_polylines(&mut chart).remove(0).0;
    assert!(
        near(line[0].1, a.1) && near(line[1].1, pane_top),
        "{line:?}"
    );
}

#[test]
fn axis_locked_segments_are_hit_on_their_body_and_nowhere_else() {
    let mut chart = chart();
    for kind in LOCKED_KINDS {
        let id = add(&mut chart, kind, vec![p(10.0, 102.0), p(20.0, 106.0)], "{}");
        let (a, b) = (anchor(&chart, id, 0), anchor(&chart, id, 1));
        let middle = ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0);
        assert_eq!(chart.drawing_at(middle.0, middle.1), Some(id), "{kind:?}");
        assert_eq!(
            chart.drawing_at(middle.0 + 120.0, middle.1 + 120.0),
            None,
            "{kind:?}"
        );
        chart.remove_drawing(id);
    }
}

#[test]
fn axis_locked_segments_round_trip_and_documents_repair_unlocked_anchors() {
    let mut chart = chart();
    for kind in LOCKED_KINDS {
        add(&mut chart, kind, vec![p(10.0, 102.0), p(20.0, 106.0)], "{}");
    }
    let document = chart.export_state_json().unwrap();
    let mut restored = ChartEngine::new(800.0, 500.0, 1.0);
    restored.import_state_json(&document).unwrap();
    assert_eq!(restored.export_state_json().unwrap(), document);
    for (restored, original) in restored.drawings().iter().zip(chart.drawings()) {
        assert_eq!(restored.kind, original.kind);
        assert_eq!(restored.points, original.points);
    }

    // A document whose anchors disagree on the locked coordinate loads repaired, like the
    // programmatic path.
    let mut tampered: serde_json::Value = serde_json::from_str(&document).unwrap();
    for drawing in tampered["drawings"].as_array_mut().unwrap() {
        let anchors = drawing["anchors"].as_array_mut().unwrap();
        anchors[0]["price"] = serde_json::json!(101.0);
        anchors[0]["logical"] = serde_json::json!(8.0);
    }
    let mut repaired = ChartEngine::new(800.0, 500.0, 1.0);
    repaired.import_state_json(&tampered.to_string()).unwrap();
    for drawing in repaired.drawings() {
        let [first, second] = drawing.points[..] else {
            panic!("two anchors");
        };
        if drawing.kind == DrawingKind::HorizontalSegment {
            assert_eq!(first.price, second.price);
        } else {
            assert_eq!(first.logical, second.logical);
        }
    }
}

/// Texts of the price-axis tags (labels with a background) in the axis frame.
fn axis_tags(chart: &mut ChartEngine) -> Vec<String> {
    chart.axis_w = 80.0;
    chart.build_frame();
    chart
        .build_axis_frame(
            80.0,
            |text, _bold| text.len() as f64 * 7.0,
            |text, _bold| text.len() as f64 * 6.0,
        )
        .labels
        .into_iter()
        .filter(|label| label.background.is_some())
        .map(|label| label.text)
        .collect()
}

#[test]
fn price_lines_run_right_from_one_anchor_with_their_price_on_the_line_and_the_axis() {
    let kind = DrawingKind::PriceLine;
    let spec = kind.spec();
    assert_eq!((spec.wire_id, spec.name), (243, "price_line"));
    assert_eq!(DrawingKind::from_u8(243), Some(kind));
    assert_eq!(DrawingKind::from_name("price_line"), Some(kind));
    assert_eq!(spec.placement, DrawingPlacement::ClickAnchors { count: 1 });
    assert!(spec.axis_price_label && !spec.axis_tag_text);
    assert!(spec.family.is_some(), "a lines-family tool");

    let mut chart = chart();
    let id = add(
        &mut chart,
        kind,
        vec![p(10.0, 102.5)],
        r##"{"color":"#123456"}"##,
    );
    let a = anchor(&chart, id, 0);
    let pane_right = chart.pane_w;
    // The line is a crisp horizontal from the anchor to the pane's right edge, level with it.
    let frame = chart.build_frame();
    let hlines = frame.panes[0]
        .main
        .iter()
        .filter_map(|prim| match prim {
            Prim::HLine {
                y, x0, x1, color, ..
            } if *color == ink() => Some((*y, *x0, *x1)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(hlines.len(), 1, "{hlines:?}");
    let (y, x0, x1) = hlines[0];
    assert!(
        (f64::from(y) - a.1).abs() <= 1.0,
        "level with the anchor: {hlines:?}"
    );
    assert!(
        (f64::from(x0) - a.0).abs() <= 1.0 && x1 == pane_right.round() as i32,
        "{hlines:?}"
    );
    // The price is printed above the line, starting at the anchor.
    let runs = texts(&mut chart);
    let (_, x, y) = runs
        .iter()
        .find(|(text, ..)| text == "102.50")
        .unwrap_or_else(|| panic!("the price on the line: {runs:?}"));
    assert!(
        f64::from(*x) >= a.0 - 0.5 && f64::from(*x) < a.0 + 8.0,
        "starts at the anchor"
    );
    assert!(f64::from(*y) < a.1, "above the line");
    // It is also tagged on the price axis.
    assert!(axis_tags(&mut chart).contains(&"102.50".to_string()));
    // The body is the ray: hit to the right of the anchor, not to its left.
    assert_eq!(chart.drawing_at(a.0 + 150.0, a.1), Some(id));
    assert_eq!(chart.drawing_at(a.0 - 60.0, a.1), None);
}

#[test]
fn a_price_lines_own_text_does_not_cover_its_price() {
    let mut chart = chart();
    add(
        &mut chart,
        DrawingKind::PriceLine,
        vec![p(10.0, 102.5)],
        r##"{"color":"#123456","text":"entry"}"##,
    );
    let runs = texts(&mut chart);
    let at = |wanted: &str| {
        runs.iter()
            .find(|(text, ..)| text == wanted)
            .unwrap_or_else(|| panic!("{wanted} in {runs:?}"))
    };
    let (_, price_x, price_y) = at("102.50");
    let (_, text_x, text_y) = at("entry");
    assert!(
        (price_x - text_x).abs() > 4.0 || (price_y - text_y).abs() > 4.0,
        "the two runs sit apart: price ({price_x}, {price_y}), text ({text_x}, {text_y})"
    );
}

/// The stats box's culling reach is measured text: installing the host's text measurer (GPUI and
/// the browser do so after the drawings exist) re-measures it, so a box that the real glyphs
/// widen into the pane paints and hits although its anchors lie left of the pane.
#[test]
fn installing_a_text_measurer_re_measures_the_stats_box_reach() {
    let mut chart = chart();
    crowd(&mut chart);
    let first = chart.coordinate_to_logical(-400.0).unwrap();
    let second = chart.coordinate_to_logical(-200.0).unwrap();
    let id = add(
        &mut chart,
        DrawingKind::HorizontalSegment,
        vec![p(first, 104.0), p(second, 104.0)],
        r##"{"color":"#123456","labels":[{"metric":"bar_count","visible":true,"position":"on","text":"MMMMMMMMMMMM"}]}"##,
    );
    let painted =
        |chart: &mut ChartEngine| texts(chart).iter().any(|(text, ..)| text == "MMMMMMMMMMMM");
    // Measured by the fallback estimate, the box ends left of the pane.
    assert!(!painted(&mut chart));
    // The host's glyphs are much wider: the box now reaches across the left half of the pane.
    chart.set_text_measure(Some(Box::new(|text, size, _family, _weight, _italic| {
        text.chars().count() as f64 * size * 4.0
    })));
    assert!(painted(&mut chart), "the re-measured box paints");
    let y = anchor(&chart, id, 1).1;
    assert_eq!(chart.hit_test_drawing(100.0, y).map(|hit| hit.id), Some(id));
    assert_eq!(
        chart.hit_test_drawing(100.0, y),
        chart.hit_test_drawing_bruteforce(100.0, y)
    );
}

// --- The `line` block's presentation on the upstream line tools ---------------------------------

/// The fork's info-line stats (the five `on` labels) in the drawing color, with the `line` block
/// that draws them as one box.
const FORK_INFO: &str = r##"{"color":"#123456","labels":[{"metric":"price_change","visible":true,"position":"on"},{"metric":"percent_change","visible":true,"position":"on"},{"metric":"bar_count","visible":true,"position":"on"},{"metric":"duration","visible":true,"position":"on"},{"metric":"angle","visible":true,"position":"on"}],"tool_options":{"line":{}}}"##;

fn texts_of(chart: &mut ChartEngine) -> Vec<String> {
    texts(chart).into_iter().map(|(text, ..)| text).collect()
}

fn collinear(a: (f64, f64), b: (f64, f64), c: (f64, f64)) -> bool {
    ((b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0)).abs()
        <= 1e-6 * (b.0 - a.0).hypot(b.1 - a.1) * (c.0 - a.0).hypot(c.1 - a.1) + 1e-3
}

/// Ink-colored arrowhead fills (`Prim::BandFill`) and upstream arrowheads (`Prim::Triangle`).
fn ink_heads(chart: &mut ChartEngine) -> (Vec<(f64, f64)>, usize) {
    let frame = chart.build_frame();
    let pane = &frame.panes[0];
    let fills = pane
        .main
        .iter()
        .filter_map(|prim| match prim {
            Prim::BandFill {
                upper_first, fill, ..
            } if *fill == ink() => {
                let tip = pane.points[*upper_first as usize];
                Some((f64::from(tip[0]), f64::from(tip[1])))
            }
            _ => None,
        })
        .collect();
    let triangles = pane
        .main
        .iter()
        .filter(|prim| matches!(prim, Prim::Triangle { color, .. } if *color == ink()))
        .count();
    (fills, triangles)
}

fn dashed_ink_hline(chart: &mut ChartEngine) -> Option<(i32, i32, i32)> {
    let frame = chart.build_frame();
    frame.panes[0].main.iter().find_map(|prim| match prim {
        Prim::HLine {
            y,
            x0,
            x1,
            style: LineStyle::Dashed,
            color,
            ..
        } if *color == ink() => Some((*y, *x0, *x1)),
        _ => None,
    })
}

#[test]
fn a_line_block_draws_an_info_lines_labels_as_one_hittable_stats_box() {
    let mut chart = chart();
    let id = add(
        &mut chart,
        DrawingKind::InfoLine,
        vec![p(10.0, 100.0), p(20.0, 105.0)],
        FORK_INFO,
    );
    let (a, b) = (anchor(&chart, id, 0), anchor(&chart, id, 1));
    let runs = texts(&mut chart);
    let lines = runs
        .iter()
        .map(|(text, ..)| text.as_str())
        .collect::<Vec<_>>();
    assert!(lines.contains(&"+5.00  +5.00%"), "{lines:?}");
    assert!(lines.contains(&"10 bars  10h"), "{lines:?}");
    let angle = lines
        .iter()
        .find(|line| line.ends_with('°'))
        .expect("angle line");
    let (degrees, _) = chart
        .drawing_screen_vector(chart.drawing(id).unwrap(), 0, 1)
        .unwrap();
    assert_eq!(*angle, format!("{degrees:.2}°"));
    assert_eq!(lines.len(), 3, "one box and no per-label text: {lines:?}");
    // The box sits beyond the second anchor, on its far side, in the drawing color.
    let (_, box_x, box_y) = runs
        .iter()
        .find(|(text, ..)| text == "+5.00  +5.00%")
        .cloned()
        .unwrap();
    assert!(f64::from(box_x) > b.0 + 8.0);
    let background = Color::rgba(0x12, 0x34, 0x56, 224);
    let frame = chart.build_frame();
    assert!(frame.panes[0]
        .main
        .iter()
        .any(|prim| matches!(prim, Prim::Rect { color, .. } if *color == background)));
    let probe = (f64::from(box_x) + 10.0, f64::from(box_y));
    assert_eq!(
        chart
            .hit_test_drawing(probe.0, probe.1)
            .map(|hit| (hit.id, hit.part)),
        Some((id, crate::DrawingDragPart::Body)),
        "the box is a body target"
    );

    // Stats position moves the box.
    assert!(chart.drawing_apply_options(
        id,
        r#"{"tool_options":{"line":{"stats_position":"start"}}}"#
    ));
    let (_, start_x, _) = texts(&mut chart)
        .into_iter()
        .find(|(text, ..)| text == "+5.00  +5.00%")
        .unwrap();
    assert!(f64::from(start_x) < a.0 - 8.0);
    assert!(chart.drawing_apply_options(
        id,
        r#"{"tool_options":{"line":{"stats_position":"middle"}}}"#
    ));
    let (_, _, middle_y) = texts(&mut chart)
        .into_iter()
        .find(|(text, ..)| text == "+5.00  +5.00%")
        .unwrap();
    assert!(f64::from(middle_y) > (a.1 + b.1) / 2.0 + 8.0);

    // Removing the block returns to upstream's per-label text at the first anchor.
    assert!(chart.drawing_apply_options(id, r#"{"tool_options":{"line":null}}"#));
    let runs = texts(&mut chart);
    assert_eq!(runs.len(), 5, "{runs:?}");
    assert!(runs
        .iter()
        .all(|(_, x, _)| (f64::from(*x) - a.0).abs() < 1e-3));
    assert_eq!(chart.hit_test_drawing(probe.0, probe.1), None);
}

#[test]
fn upstream_line_tools_keep_upstreams_rendering_without_a_line_block() {
    let mut chart = chart();
    let id = add(
        &mut chart,
        DrawingKind::InfoLine,
        vec![p(10.0, 100.0), p(20.0, 105.0)],
        r##"{"color":"#123456"}"##,
    );
    let a = anchor(&chart, id, 0);
    let runs = texts(&mut chart);
    assert_eq!(runs.len(), 4, "upstream's four labels: {runs:?}");
    assert!(runs
        .iter()
        .all(|(_, x, _)| (f64::from(*x) - a.0).abs() < 1e-3));
    let background = Color::rgba(0x12, 0x34, 0x56, 224);
    let frame = chart.build_frame();
    assert!(!frame.panes[0]
        .main
        .iter()
        .any(|prim| matches!(prim, Prim::Rect { color, .. } if *color == background)));
    chart.remove_drawing(id);

    add(
        &mut chart,
        DrawingKind::TrendAngle,
        vec![p(10.0, 101.0), p(20.0, 104.0)],
        r##"{"color":"#123456"}"##,
    );
    assert_eq!(dashed_ink_hline(&mut chart), None);
    assert_eq!(ink_polylines(&mut chart).len(), 1, "the segment alone");
}

#[test]
fn a_line_block_draws_the_trend_angles_reference_arc_and_folded_angle() {
    let mut chart = chart();
    let a = p(10.0, 101.0);
    let (ax, ay) = chart
        .drawing_to_px_for(0, crate::DrawingPriceScale::Right, a)
        .unwrap();
    let b = chart
        .drawing_from_px_for(0, crate::DrawingPriceScale::Right, ax + 90.0, ay - 90.0)
        .unwrap();
    let id = add(
        &mut chart,
        DrawingKind::TrendAngle,
        vec![a, b],
        r##"{"color":"#123456","labels":[],"tool_options":{"line":{}}}"##,
    );
    assert_eq!(texts_of(&mut chart), ["45.00°"]);
    let (reference_y, reference_x0, reference_x1) =
        dashed_ink_hline(&mut chart).expect("crisp dashed horizontal reference");
    assert_eq!(reference_y, ay.round() as i32);
    assert_eq!(reference_x0, ax.round() as i32);
    assert!(
        f64::from(reference_x1) > ax + 90.0,
        "as long as the segment"
    );
    let lines = ink_polylines(&mut chart);
    let arc = &lines
        .iter()
        .find(|(points, ..)| points.len() > 2)
        .expect("arc")
        .0;
    let radius = (arc[0].0 - ax).hypot(arc[0].1 - ay);
    assert!((16.0..=48.0).contains(&radius));
    assert!(arc
        .iter()
        .all(|point| ((point.0 - ax).hypot(point.1 - ay) - radius).abs() < 1e-3));
    assert!(
        (arc[0].1 - ay).abs() < 1e-3,
        "the arc starts on the reference"
    );
    let end = arc[arc.len() - 1];
    assert!(
        ((end.0 - ax) + (end.1 - ay)).abs() < 1e-3,
        "and ends on the 45° segment"
    );

    // The reference and the arc are body targets; the angle text is not.
    let hit = |chart: &ChartEngine, x: f64, y: f64| chart.hit_test_drawing(x, y).map(|hit| hit.id);
    assert_eq!(hit(&chart, ax + 120.0, ay), Some(id), "reference");
    let bisector = -std::f64::consts::FRAC_PI_8;
    assert_eq!(
        hit(
            &chart,
            ax + radius * bisector.cos(),
            ay + radius * bisector.sin()
        ),
        Some(id),
        "arc"
    );
    let label = radius + 6.0;
    assert_eq!(
        hit(
            &chart,
            ax + label * bisector.cos() + 10.0,
            ay + label * bisector.sin()
        ),
        None,
        "angle text"
    );

    // Falling segments read negative, and a segment drawn leftward folds into [-90°, 90°].
    let falling = chart
        .drawing_from_px_for(0, crate::DrawingPriceScale::Right, ax + 90.0, ay + 30.0)
        .unwrap();
    assert!(chart
        .set_drawing_anchors(id, &[a.into(), falling.into()])
        .is_ok());
    assert_eq!(texts_of(&mut chart), ["-18.43°"]);
    let leftward = chart
        .drawing_from_px_for(0, crate::DrawingPriceScale::Right, ax - 90.0, ay - 90.0)
        .unwrap();
    assert!(chart
        .set_drawing_anchors(id, &[a.into(), leftward.into()])
        .is_ok());
    assert_eq!(texts_of(&mut chart), ["45.00°"]);
    let (_, x0, x1) = dashed_ink_hline(&mut chart).unwrap();
    assert!(f64::from(x0) < ax - 90.0 && x1 == ax.round() as i32);
}

#[test]
fn the_trend_angle_arc_scales_with_the_device_pixel_ratio() {
    for dpr in [1.0, 1.5, 2.0] {
        let mut chart = chart_with(&hourly(40), dpr);
        let id = add(
            &mut chart,
            DrawingKind::TrendAngle,
            vec![p(10.0, 101.0), p(30.0, 104.0)],
            r##"{"color":"#123456","labels":[],"tool_options":{"line":{}}}"##,
        );
        let (a, b) = (anchor(&chart, id, 0), anchor(&chart, id, 1));
        let hpr = (chart.pane_w * dpr).round() / chart.pane_w;
        let vpr = (chart.pane_h * dpr).round() / chart.pane_h;
        let (ax, ay) = (a.0 * hpr, a.1 * vpr);
        let length = ((b.0 - a.0) * hpr).hypot((b.1 - a.1) * vpr);
        let lines = ink_polylines(&mut chart);
        let arc = &lines
            .iter()
            .find(|(points, ..)| points.len() > 2)
            .expect("arc")
            .0;
        let radius = (arc[0].0 - ax).hypot(arc[0].1 - ay);
        let expected = (length / 3.0).clamp(16.0 * vpr, 48.0 * vpr);
        assert!(
            (radius - expected).abs() < 1e-3,
            "{dpr}: {radius} {expected}"
        );
        let width = lines
            .iter()
            .find(|(points, ..)| points.len() > 2)
            .unwrap()
            .1;
        assert!((f64::from(width) - vpr).abs() < 1e-5, "1 CSS px");
    }
}

/// A steep trend angle's reference runs far past its anchors' box: it paints and hits through
/// the culled index although both anchors lie left of the pane.
#[test]
fn a_steep_trend_angles_reference_is_culled_by_its_screen_reach() {
    let mut chart = chart();
    crowd(&mut chart);
    // Both anchors lie far left of the pane, and beyond the stats and angle pad; the second one
    // also above it.
    let left = chart.coordinate_to_logical(-300.0).unwrap();
    let a = p(left, 100.5);
    let (ax, ay) = chart
        .drawing_to_px_for(0, crate::DrawingPriceScale::Right, a)
        .unwrap();
    let b = chart
        .drawing_from_px_for(0, crate::DrawingPriceScale::Right, ax + 8.0, ay - 600.0)
        .unwrap();
    let id = add(
        &mut chart,
        DrawingKind::TrendAngle,
        vec![a, b],
        r##"{"color":"#123456","labels":[],"tool_options":{"line":{}}}"##,
    );
    assert!(chart.drawings().len() > 20, "the culled candidate path");
    let (_, _, x1) = dashed_ink_hline(&mut chart).expect("the reference paints");
    assert!(f64::from(x1) > 150.0);
    let probe = (150.0, ay);
    assert_eq!(
        chart.hit_test_drawing(probe.0, probe.1).map(|hit| hit.id),
        Some(id)
    );
    assert_eq!(
        chart.hit_test_drawing(probe.0, probe.1),
        chart.hit_test_drawing_bruteforce(probe.0, probe.1)
    );
}

#[test]
fn a_line_block_trims_arrow_strokes_under_their_heads_and_caps_only_unextended_ends() {
    let mut chart = chart();
    let id = add(
        &mut chart,
        DrawingKind::ArrowLine,
        vec![p(10.0, 101.0), p(20.0, 101.0)],
        r##"{"color":"#123456","width":2,"tool_options":{"line":{}}}"##,
    );
    let (a, b) = (anchor(&chart, id, 0), anchor(&chart, id, 1));
    let (heads, triangles) = ink_heads(&mut chart);
    assert_eq!(heads.len(), 1);
    assert!(close(heads[0], b, 0.01), "the tip stays on the anchor");
    assert_eq!(triangles, 0);
    let stroke = ink_polylines(&mut chart).remove(0).0;
    assert!(close(stroke[0], a, 0.01));
    assert!(
        (b.0 - stroke[1].0 - 2.0).abs() < 1e-3,
        "trimmed by one stroke width"
    );

    // An extended end carries no head.
    assert!(chart.drawing_apply_options(id, r#"{"extend_right":true}"#));
    assert_eq!(ink_heads(&mut chart), (Vec::new(), 0));

    // Without the block the arrow line is upstream's: an untrimmed stroke under a triangle head,
    // on the pane edge when extended.
    assert!(
        chart.drawing_apply_options(id, r#"{"extend_right":false,"tool_options":{"line":null}}"#)
    );
    let stroke = ink_polylines(&mut chart).remove(0).0;
    assert!(close(stroke[1], b, 0.01));
    assert_eq!(ink_heads(&mut chart).1, 1);
    chart.remove_drawing(id);

    // A ray's arrow end reaches the pane edge, so it carries no head.
    add(
        &mut chart,
        DrawingKind::Ray,
        vec![p(10.0, 101.0), p(20.0, 102.0)],
        r##"{"color":"#123456","stroke_end":"arrow","stroke_start":"arrow","tool_options":{"line":{}}}"##,
    );
    let (heads, triangles) = ink_heads(&mut chart);
    assert_eq!((heads.len(), triangles), (1, 0), "only the anchored start");
}

#[test]
fn a_coincident_rays_caps_follow_its_resolved_ends() {
    let mut chart = chart();
    // Coincident anchors have no direction: the ray runs up from its anchor only, whatever
    // `extend_left` says, so its anchored start keeps its arrowhead.
    add(
        &mut chart,
        DrawingKind::Ray,
        vec![p(15.0, 101.0), p(15.0, 101.0)],
        r##"{"color":"#123456","extend_left":true,"stroke_end":"arrow","stroke_start":"arrow","tool_options":{"line":{}}}"##,
    );
    let (heads, triangles) = ink_heads(&mut chart);
    assert_eq!((heads.len(), triangles), (1, 0), "only the anchored start");
}

#[test]
fn a_line_blocks_caps_are_body_targets() {
    let mut chart = chart();
    let id = add(
        &mut chart,
        DrawingKind::ArrowLine,
        vec![p(10.0, 101.0), p(20.0, 101.0)],
        r##"{"color":"#123456","width":6,"stroke_start":"circle","tool_options":{"line":{}}}"##,
    );
    let (a, b) = (anchor(&chart, id, 0), anchor(&chart, id, 1));
    // 7.5 px off the segment (beyond its 3 + 3 px reach) but inside the 10.5 px head.
    let head = (b.0 - 16.0, b.1 + 7.5);
    let disc = (a.0 - 10.0, a.1);
    let hit =
        |chart: &ChartEngine, (x, y): (f64, f64)| chart.hit_test_drawing(x, y).map(|hit| hit.id);
    assert_eq!(hit(&chart, head), Some(id), "arrowhead");
    assert_eq!(hit(&chart, disc), Some(id), "circle cap");
    for (x, y) in [head, disc] {
        assert_eq!(
            chart.hit_test_drawing(x, y),
            chart.hit_test_drawing_bruteforce(x, y)
        );
    }
    assert!(chart.drawing_apply_options(id, r#"{"tool_options":{"line":null}}"#));
    assert_eq!(hit(&chart, head), None, "upstream hits the stroke only");
}

#[test]
fn a_ray_extends_back_through_its_first_anchor_by_extend_left() {
    let mut chart = chart();
    let pane_right = chart.pane_w;
    let id = add(
        &mut chart,
        DrawingKind::Ray,
        vec![p(10.0, 101.0), p(20.0, 102.0)],
        r##"{"color":"#123456","extend_left":true}"##,
    );
    let (a, b) = (anchor(&chart, id, 0), anchor(&chart, id, 1));
    let line = ink_polylines(&mut chart).remove(0).0;
    assert!(line[0].0.abs() < 0.5, "reaches the left edge: {line:?}");
    assert!((line[1].0 - pane_right).abs() < 0.5);
    assert!(collinear(a, b, line[0]) && collinear(a, b, line[1]));
    let x = a.0 / 2.0;
    let y = a.1 + (b.1 - a.1) * (x - a.0) / (b.0 - a.0);
    assert_eq!(chart.hit_test_drawing(x, y).map(|hit| hit.id), Some(id));

    // Turning `extend_right` off does not make a segment of a ray (not restored).
    assert!(chart.drawing_apply_options(id, r#"{"extend_left":false,"extend_right":false}"#));
    let line = ink_polylines(&mut chart).remove(0).0;
    assert!(close(line[0], a, 0.01));
    assert!((line[1].0 - pane_right).abs() < 0.5);
    assert_eq!(chart.hit_test_drawing(x, y), None);
}

#[test]
fn vertical_segment_tools_extend_to_the_pane_edges_by_their_flags() {
    let mut chart = chart();
    let (top, bottom) = (
        chart.panes[0].top,
        chart.panes[0].top + chart.panes[0].height,
    );
    for kind in [
        DrawingKind::InfoLine,
        DrawingKind::TrendAngle,
        DrawingKind::ArrowLine,
        DrawingKind::TrendLine,
    ] {
        let id = add(
            &mut chart,
            kind,
            vec![p(15.0, 101.0), p(15.0, 105.0)],
            r##"{"color":"#123456","labels":[],"stroke_end":"none","extend_right":true}"##,
        );
        let (a, b) = (anchor(&chart, id, 0), anchor(&chart, id, 1));
        let line = ink_polylines(&mut chart).remove(0).0;
        assert!(close(line[0], a, 0.01), "{kind:?}");
        if kind == DrawingKind::TrendLine {
            assert!(close(line[1], b, 0.01), "a trend line stays its segment");
        } else {
            // The second anchor is above the first: extending past it reaches the top.
            assert!((line[1].1 - top).abs() < 0.5, "{kind:?}: {line:?}");
            assert!((line[1].0 - b.0).abs() < 0.01);
        }
        assert!(chart.drawing_apply_options(id, r#"{"extend_left":true,"extend_right":false}"#));
        let line = ink_polylines(&mut chart).remove(0).0;
        if kind == DrawingKind::TrendLine {
            assert!(close(line[0], a, 0.01));
        } else {
            assert!((line[0].1 - bottom).abs() < 0.5, "{kind:?}: {line:?}");
            assert!(close(line[1], b, 0.01));
        }
        chart.remove_drawing(id);
    }
}

/// Coincident anchors have no direction: the segment tools that extend by their flags draw and
/// hit nothing beyond the anchor, with or without the `line` block.
#[test]
fn coincident_anchors_do_not_extend_into_a_vertical_line() {
    let mut chart = chart();
    for kind in [
        DrawingKind::InfoLine,
        DrawingKind::TrendAngle,
        DrawingKind::ArrowLine,
    ] {
        for block in [r#""tool_options":{}"#, r#""tool_options":{"line":{}}"#] {
            for extend in [r#""extend_right":true"#, r#""extend_left":true"#] {
                let options = format!(r##"{{"color":"#123456","labels":[],{extend},{block}}}"##);
                let id = add(
                    &mut chart,
                    kind,
                    vec![p(15.0, 103.0), p(15.0, 103.0)],
                    &options,
                );
                let a = anchor(&chart, id, 0);
                let lines = ink_polylines(&mut chart);
                assert!(
                    lines
                        .iter()
                        .all(|(line, ..)| line.iter().all(|point| close(*point, a, 0.01))),
                    "{kind:?} {options}: {lines:?}"
                );
                for probe in [(a.0, a.1 - 60.0), (a.0, a.1 + 60.0)] {
                    assert_eq!(
                        chart.hit_test_drawing(probe.0, probe.1),
                        None,
                        "{kind:?} {options}"
                    );
                }
                chart.remove_drawing(id);
            }
        }
    }
}

/// A middle segment-layout `text` splits the stroke the same way with the `line` block as
/// without it.
#[test]
fn a_line_block_keeps_the_stroke_gap_under_a_middle_text_label() {
    let mut chart = chart();
    for kind in [DrawingKind::InfoLine, DrawingKind::ArrowLine] {
        let pieces = |chart: &mut ChartEngine, block: &str| {
            let options = format!(
                r##"{{"color":"#123456","labels":[],"stroke_end":"none","text":"Breakout",{block}}}"##
            );
            let id = add(chart, kind, vec![p(10.0, 101.0), p(30.0, 105.0)], &options);
            let drawing = chart.drawing(id).unwrap();
            assert_eq!(
                (drawing.text_h_align, drawing.text_v_align),
                (DrawingTextHAlign::Center, DrawingTextVAlign::Middle)
            );
            let lines = ink_polylines(chart);
            chart.remove_drawing(id);
            lines
        };
        let upstream = pieces(&mut chart, r#""tool_options":{}"#);
        let layered = pieces(&mut chart, r#""tool_options":{"line":{}}"#);
        assert_eq!(upstream.len(), 2, "{kind:?}: {upstream:?}");
        assert_eq!(layered.len(), 2, "{kind:?}: {layered:?}");
        for ((up, ..), (with, ..)) in upstream.iter().zip(&layered) {
            assert_eq!(up.len(), with.len());
            assert!(
                up.iter().zip(with).all(|(a, b)| close(*a, *b, 0.01)),
                "{kind:?}: {up:?} vs {with:?}"
            );
        }
    }
}

#[test]
fn indexed_hit_testing_matches_brute_force_under_line_blocks() {
    let mut chart = chart();
    let options = FORK_INFO;
    for copy in 0..5 {
        let shift = copy as f64 * 0.7;
        for kind in LINE_KINDS {
            let points = two_points(kind)
                .into_iter()
                .map(|point| p(point.logical + shift, point.price + shift * 0.3))
                .collect();
            add(&mut chart, kind, points, options);
        }
        // A steep trend angle, whose reference reaches far past its anchors.
        add(
            &mut chart,
            DrawingKind::TrendAngle,
            vec![p(4.0 + shift, 100.2), p(4.2 + shift, 106.0)],
            options,
        );
    }
    // An info line left of the pane whose box reaches into it.
    let left = chart.coordinate_to_logical(-60.0).unwrap();
    let edge = chart.coordinate_to_logical(-20.0).unwrap();
    add(
        &mut chart,
        DrawingKind::InfoLine,
        vec![p(left, 103.0), p(edge, 103.5)],
        options,
    );
    assert!(chart.drawings().len() > 20);
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
    assert!(hits > 100, "the grid meets the drawings ({hits} hits)");
}

#[test]
fn a_line_blocks_stats_box_scales_with_the_device_pixel_ratio() {
    for dpr in [1.0, 1.5, 2.0] {
        let mut chart = chart_with(&hourly(40), dpr);
        let id = add(
            &mut chart,
            DrawingKind::InfoLine,
            vec![p(10.0, 100.0), p(20.0, 105.0)],
            FORK_INFO,
        );
        let b = anchor(&chart, id, 1);
        let stroke = ink_polylines(&mut chart).remove(0);
        let hpr = (chart.pane_w * dpr).round() / chart.pane_w;
        let vpr = (chart.pane_h * dpr).round() / chart.pane_h;
        assert!((f64::from(stroke.1) - 2.0 * vpr).abs() < 1e-5);
        assert!((stroke.0[1].0 - b.0 * hpr).abs() < 1e-3);
        assert!((stroke.0[1].1 - b.1 * vpr).abs() < 1e-3);
        let (_, x, _) = texts(&mut chart)
            .into_iter()
            .find(|(text, ..)| text == "+5.00  +5.00%")
            .unwrap();
        assert!(f64::from(x) > (b.0 + 8.0) * dpr - 1.0);
    }
}

#[test]
fn creation_previews_draw_the_armed_line_blocks_stats() {
    let mut chart = chart();
    assert!(chart.set_drawing_tool(Some(DrawingKind::InfoLine), Some(FORK_INFO), None));
    let start = chart
        .drawing_to_px_for(0, crate::DrawingPriceScale::Right, p(10.0, 100.0))
        .unwrap();
    let end = chart
        .drawing_to_px_for(0, crate::DrawingPriceScale::Right, p(20.0, 105.0))
        .unwrap();
    chart.drawing_tool_activate(start.0, start.1, DrawingModifiers::default());
    chart.drawing_tool_pointer_move(end.0, end.1, DrawingModifiers::default(), false);
    assert!(
        texts_of(&mut chart)
            .iter()
            .any(|text| text == "10 bars  10h"),
        "the pending info line shows its stats box before the second click"
    );
}

#[test]
fn line_block_stats_omit_durations_on_non_time_bars() {
    use crate::{
        AggressorSide, FootprintAggregationOptions, FootprintBarAggregation,
        FootprintSeriesOptions, FootprintTrade,
    };
    let trade = |index: i64| FootprintTrade {
        timestamp_micros: 2_000_001 + index * 1_000_000,
        price: 100.0 + (index % 3) as f64,
        volume: 1.0,
        aggressor: AggressorSide::Buy,
        bid: None,
        ask: None,
        sequence: None,
        trade_id: None,
        conditions: 0,
        session_id: Some(1),
    };
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    let footprint = chart
        .add_footprint_series(FootprintSeriesOptions {
            aggregation: FootprintAggregationOptions {
                tick_size: 1.0,
                bars: FootprintBarAggregation::Trades { trades_per_bar: 1 },
                ..FootprintAggregationOptions::default()
            },
            ..FootprintSeriesOptions::default()
        })
        .unwrap();
    chart
        .set_footprint_trades(footprint, (0..20).map(trade).collect())
        .unwrap();
    chart.time_scale.set_width(800.0);
    chart.fit_content();
    chart.build_frame();
    let id = add(
        &mut chart,
        DrawingKind::InfoLine,
        vec![p(2.0, 100.0), p(6.0, 102.0)],
        FORK_INFO,
    );
    let runs = texts_of(&mut chart);
    assert!(runs.iter().any(|text| text == "+2.00  +2.00%"), "{runs:?}");
    assert!(
        runs.iter().any(|text| text == "4 bars"),
        "row keys are never read as a duration: {runs:?}"
    );
    let b = anchor(&chart, id, 1);
    assert_eq!(
        chart.hit_test_drawing(b.0 + 20.0, b.1).map(|hit| hit.id),
        Some(id)
    );
}

#[test]
fn line_block_stats_paint_and_hit_on_a_lower_pane() {
    let mut chart = chart();
    let pane = chart.add_pane(true).unwrap();
    let series = chart.add_series(crate::SeriesKind::Line);
    let values = (0..40)
        .map(|index| 10.0 + index as f64 * 0.1)
        .collect::<Vec<_>>();
    chart
        .set_series_data(series, &hourly(40), &values, &values, &values, &values)
        .unwrap();
    chart.set_series_pane(series, pane, 1.0);
    chart.build_frame();
    let id = chart
        .add_drawing(
            DrawingKind::InfoLine,
            pane,
            vec![p(10.0, 10.5), p(20.0, 11.5)],
            Some(FORK_INFO),
        )
        .unwrap();
    let frame = chart.build_frame();
    let (a, b) = (anchor(&chart, id, 0), anchor(&chart, id, 1));
    assert!(a.1 > chart.panes[pane].top && b.1 > chart.panes[pane].top);
    let stats = frame.panes[pane]
        .main
        .iter()
        .find_map(|prim| match prim {
            Prim::Text { text, y, .. } if text.contains("bars") => Some(f64::from(*y)),
            _ => None,
        })
        .expect("stats on the drawing's own pane");
    assert!(stats > chart.panes[pane].top);
    assert!(!frame.panes[0]
        .main
        .iter()
        .any(|prim| matches!(prim, Prim::Text { text, .. } if text.contains("bars"))));
    let middle = ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0);
    assert_eq!(
        chart.hit_test_drawing(middle.0, middle.1).map(|hit| hit.id),
        Some(id)
    );
    assert_eq!(
        chart.hit_test_drawing(b.0 + 20.0, b.1).map(|hit| hit.id),
        Some(id)
    );
}

#[test]
fn upstream_line_tools_list_the_stats_position_their_line_block_reads() {
    for kind in LINE_KINDS {
        let schema = crate::drawing_property_schema(kind);
        let stats = schema
            .properties
            .iter()
            .find(|property| property.name == "tool_options.line.stats_position")
            .unwrap_or_else(|| panic!("{kind:?}"));
        assert_eq!(stats.default, serde_json::json!("end"));
        assert_eq!(stats.enum_values, ["start", "middle", "end"]);
    }
    // Writing the descriptor's default back creates the block: the stats box appears at the
    // end. `null` on the block removes it again; the kind options stay generic.
    let mut chart = chart();
    let id = add(
        &mut chart,
        DrawingKind::InfoLine,
        vec![p(10.0, 100.0), p(20.0, 105.0)],
        r##"{"color":"#123456"}"##,
    );
    assert_eq!(texts(&mut chart).len(), 4);
    assert!(
        chart.drawing_apply_options(id, r#"{"tool_options":{"line":{"stats_position":"end"}}}"#)
    );
    assert_eq!(
        chart.drawing(id).unwrap().tool_options.line,
        Some(Default::default())
    );
    assert_eq!(
        texts_of(&mut chart)[..2],
        ["+5.00  +5.00%".to_string(), "10 bars".to_string()]
    );
    assert_eq!(
        chart.drawing_kind_options_json(id).as_deref(),
        Some(r#"{"kind":"generic"}"#)
    );
    assert!(chart.drawing_apply_options(id, r#"{"tool_options":{"line":null}}"#));
    assert_eq!(texts(&mut chart).len(), 4);
}

#[test]
fn fork_documents_restore_the_info_lines_stats_box() {
    let document = serde_json::json!({
        "schema": "aeris_charts-state",
        "schema_version": 1,
        "panes": [{"id": "pane-1"}],
        "drawings": [{"id": 1, "kind": "info_line", "pane_id": "pane-1", "anchors": [
            {"logical": 10.0, "price": 100.0, "time": 10.0 * HOUR},
            {"logical": 20.0, "price": 105.0, "time": 20.0 * HOUR}
        ]}],
    })
    .to_string();
    let mut chart = chart();
    chart.import_state_json(&document).unwrap();
    let drawing = chart.drawings()[0].clone();
    assert!(crate::drawings::kinds::lines::is_legacy_info_stats(
        &drawing.labels
    ));
    assert_eq!(drawing.tool_options.line, Some(Default::default()));
    let runs = texts_of(&mut chart);
    assert!(runs.iter().any(|text| text == "10 bars  10h"), "{runs:?}");
    let exported: serde_json::Value =
        serde_json::from_str(&chart.export_state_json().unwrap()).unwrap();
    assert_eq!(
        exported["drawings"][0]["style"]["tool_options"],
        serde_json::json!({"line": {"stats_position": "end"}})
    );
    let mut again = chart_with(&hourly(40), 1.0);
    again.import_state_json(&exported.to_string()).unwrap();
    assert_eq!(again.drawings()[0], drawing);
}

/// A fork-era info line pasted from the clipboard or applied from a sync payload (its five
/// default stats, no `line` key) paints the one stats box.
#[test]
fn fork_payloads_restore_the_info_lines_stats_box() {
    let stats = [
        "price_change",
        "percent_change",
        "bar_count",
        "duration",
        "angle",
    ]
    .map(|metric| serde_json::json!({"metric": metric, "visible": true, "position": "on"}));
    let items = serde_json::json!([{"id": 1, "kind": "info_line", "pane_index": 0,
        "options": {"labels": stats, "tool_options": {}},
        "points": [{"logical": 10.0, "price": 100.0}, {"logical": 20.0, "price": 105.0}]}]);
    let check = |chart: &mut ChartEngine| {
        let runs = texts_of(chart);
        assert!(runs.iter().any(|text| text == "10 bars  10h"), "{runs:?}");
        assert_eq!(runs.len(), 3, "one box and no per-label text: {runs:?}");
    };
    let clipboard = serde_json::json!({
        "schema": "aeris_charts-drawings",
        "revision": 1,
        "drawings": items,
    })
    .to_string();
    let mut pasted = chart();
    pasted.paste_drawings_json(&clipboard, 0, 0.0, 0.0).unwrap();
    check(&mut pasted);
    let sync = serde_json::json!({
        "schema": "aeris_charts-drawing-sync",
        "source": "fork-peer",
        "revision": 3,
        "drawings": items,
    })
    .to_string();
    let mut peer = chart();
    assert!(peer.apply_drawing_sync_payload_json(&sync));
    check(&mut peer);
}
