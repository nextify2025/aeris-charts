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
