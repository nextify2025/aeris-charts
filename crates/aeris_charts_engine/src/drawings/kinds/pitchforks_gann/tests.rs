//! Pitchforks & Gann family engine tests: catalog defaults, armed placement (with the partial
//! placement guide), shared-part geometry per tool at several device-pixel ratios, hit testing
//! (indexed against brute force, paint-bounded culling), drags, straighten, magnet, keyboard
//! nudges, time identity, schema and kind options, atomic patches with history, persistence with
//! default omission, clipboard, and sync.

use aeris_charts_render::color::Color;
use aeris_charts_render::draw_list::{LineStyle, Prim};

use super::super::super::DrawingTextLayout;
use super::{GannToolOptions, MEDIAN_COLOR};
use crate::{ChartEngine, DrawingAnchor, DrawingId, DrawingKind, DrawingModifiers, DrawingPoint};

const KINDS: [DrawingKind; 9] = [
    DrawingKind::AndrewsPitchfork,
    DrawingKind::SchiffPitchfork,
    DrawingKind::ModifiedSchiffPitchfork,
    DrawingKind::InsidePitchfork,
    DrawingKind::Pitchfan,
    DrawingKind::GannBox,
    DrawingKind::GannSquare,
    DrawingKind::GannSquareFixed,
    DrawingKind::GannFan,
];
const INK: &str = "#123456";
const HOUR: f64 = 3_600.0;

fn chart_with(width: f64, times: &[f64], dpr: f64) -> ChartEngine {
    let mut chart = ChartEngine::new(width, 500.0, dpr);
    let values = (0..times.len())
        .map(|index| 100.0 + (index % 7) as f64)
        .collect::<Vec<_>>();
    chart
        .set_series_data(0, times, &values, &values, &values, &values)
        .unwrap();
    chart.time_scale.set_width(width);
    chart.fit_content();
    chart.build_frame();
    chart
}

fn hourly(count: usize) -> Vec<f64> {
    (0..count).map(|index| index as f64 * HOUR).collect()
}

fn chart() -> ChartEngine {
    chart_with(800.0, &hourly(40), 1.0)
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

/// Deterministic anchors for each kind (a rising swing then a pullback for the pitchforks).
fn anchors(kind: DrawingKind) -> Vec<DrawingPoint> {
    match kind.anchor_count() {
        1 => vec![p(12.0, 101.0)],
        2 => vec![p(10.0, 101.0), p(20.0, 105.0)],
        _ => vec![p(8.0, 101.0), p(16.0, 105.0), p(20.0, 102.0)],
    }
}

fn anchor(chart: &ChartEngine, id: DrawingId, index: usize) -> (f64, f64) {
    chart.drawing_point_to_coordinate(id, index).unwrap()
}

fn color(css: &str) -> Color {
    Color::parse_css(css).unwrap()
}

type Polyline = (Vec<(f64, f64)>, f32, LineStyle);

fn polylines(chart: &mut ChartEngine, css: &str) -> Vec<Polyline> {
    let target = color(css);
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
            } if *color == target => Some((
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

fn close(a: (f64, f64), b: (f64, f64), tolerance: f64) -> bool {
    (a.0 - b.0).abs() <= tolerance && (a.1 - b.1).abs() <= tolerance
}

/// The fork's pre-merge pitchfork and Gann defaults, which documents it wrote omitted, come back
/// through `apply_legacy_fork_defaults` (upstream renders these tools now).
#[test]
fn catalog_defaults_follow_each_tool() {
    for kind in KINDS {
        let mut drawing = crate::Drawing::new(1, kind, 0, Vec::new());
        super::super::apply_legacy_fork_defaults(&mut drawing);
        let spec = kind.spec();
        assert!(spec.family.is_none(), "{kind:?} is upstream-rendered");
        assert_eq!(spec.text_layout, DrawingTextLayout::Box);
        assert!(drawing.fill_enabled, "{kind:?} fills its zones by default");
        assert!(drawing.tool_options.is_empty());
        let visible = |drawing: &crate::Drawing| {
            drawing
                .levels
                .iter()
                .filter(|level| level.visible)
                .map(|level| level.value)
                .collect::<Vec<_>>()
        };
        match kind {
            DrawingKind::GannBox => {
                assert_eq!(visible(&drawing), [0.0, 0.25, 0.382, 0.5, 0.618, 0.75, 1.0]);
            }
            DrawingKind::GannSquare | DrawingKind::GannSquareFixed => {
                assert_eq!(visible(&drawing), [0.0, 0.2, 0.4, 0.6, 0.8, 1.0]);
            }
            DrawingKind::GannFan => {
                assert!(drawing.extend_right, "Gann fan lines are rays");
                assert_eq!(
                    visible(&drawing),
                    [0.125, 0.25, 1.0 / 3.0, 0.5, 1.0, 2.0, 3.0, 4.0, 8.0]
                );
            }
            _ => {
                assert_eq!(drawing.color, MEDIAN_COLOR);
                assert_eq!(drawing.levels.len(), 9);
                assert_eq!(
                    visible(&drawing),
                    [0.5, 1.0],
                    "TradingView's visible levels"
                );
                assert!(!drawing.extend_left && !drawing.extend_right);
            }
        }
        assert_eq!(drawing.width, 1.0);
        // The fork's option defaults its documents never stored: the box's time levels and the
        // squares' stats box, merged under what a document stored when it is restored.
        let mut stored = serde_json::json!({"gann": {"show_stats": false}});
        super::super::merge_legacy_fork_tool_options(kind, &mut stored);
        let restored = serde_json::from_value::<crate::DrawingToolOptions>(stored)
            .unwrap()
            .gann
            .unwrap();
        assert!(!restored.show_stats, "{kind:?}: a stored key wins");
        let mut blank = serde_json::json!({});
        super::super::merge_legacy_fork_tool_options(kind, &mut blank);
        let fork = serde_json::from_value::<crate::DrawingToolOptions>(blank)
            .unwrap()
            .gann
            .unwrap_or_default();
        assert!(fork.validate());
        match kind {
            DrawingKind::GannBox => assert_eq!(fork.time_levels.len(), 7),
            DrawingKind::GannSquare | DrawingKind::GannSquareFixed => assert!(fork.show_stats),
            _ => assert_eq!(fork, GannToolOptions::default(), "{kind:?}"),
        }
    }
    // The option type's own defaults are upstream's look, so a block a patch creates for one key
    // switches nothing else on.
    let options = GannToolOptions::default();
    assert!(options.validate());
    assert!(options.time_levels.is_empty());
    assert!(!options.show_stats);
    assert_eq!(options.angles.len(), 9);
    assert_eq!(options.arcs.len(), 5);
    assert_eq!(options.size_bars, 20.0);
    assert_eq!(options.scale_ratio, None);
}

/// A pitchfork the fork wrote converts its levels to upstream's meaning: upstream's renderer then
/// draws the fork's tines (each fork level on both sides of the median) and band fills (each
/// band in the fill of the fork level outside it, mirrored on both sides).
#[test]
fn fork_pitchfork_levels_keep_their_tines_and_band_fills() {
    for kind in [
        DrawingKind::AndrewsPitchfork,
        DrawingKind::SchiffPitchfork,
        DrawingKind::InsidePitchfork,
        DrawingKind::Pitchfan,
    ] {
        let mut chart = chart();
        let id = add(&mut chart, kind, anchors(kind), "{}");
        let mut fork = crate::Drawing::new(0, kind, 0, Vec::new());
        super::super::apply_legacy_fork_defaults(&mut fork);
        super::legacy_levels_to_upstream(&mut fork);
        let options = serde_json::json!({"levels": fork.levels, "color": MEDIAN_COLOR});
        assert!(chart.drawing_apply_options(id, &options.to_string()));
        let frame = chart.build_frame();
        let fills = frame.panes[0]
            .main
            .iter()
            .filter_map(|prim| match prim {
                Prim::BandFill { fill, .. } => Some(*fill),
                _ => None,
            })
            .collect::<Vec<_>>();
        // The fork's defaults: the 0.5 tines' band in green inside, the 1 tines' in blue
        // outside, on both sides of the median.
        let (green, blue) = (
            Color::rgba(0x08, 0x99, 0x81, 35),
            Color::rgba(0x29, 0x62, 0xff, 35),
        );
        assert_eq!(fills, [blue, green, green, blue], "{kind:?}");
        // The median and the four tines, in the drawing's and the levels' colors.
        assert_eq!(polylines(&mut chart, MEDIAN_COLOR).len(), 1, "{kind:?}");
        assert_eq!(polylines(&mut chart, "#089981").len(), 2, "{kind:?}");
        assert_eq!(polylines(&mut chart, "#2962ff").len(), 2, "{kind:?}");
    }
    // Other kinds keep their levels.
    let mut gann = crate::Drawing::new(0, DrawingKind::GannBox, 0, Vec::new());
    let levels = gann.levels.clone();
    super::legacy_levels_to_upstream(&mut gann);
    assert_eq!(gann.levels, levels);
}

/// Past the level cap, a fork pitchfork keeps its visible tines before any hidden level.
#[test]
fn fork_pitchfork_level_cap_keeps_the_visible_tines() {
    let mut fork = crate::Drawing::new(0, DrawingKind::AndrewsPitchfork, 0, Vec::new());
    fork.levels = (1..=35)
        .map(|index| crate::DrawingLevel {
            visible: false,
            ..crate::DrawingLevel::at(f64::from(index) / 100.0, "#787b86")
        })
        .chain([0.5, 1.0].map(|value| crate::DrawingLevel::at(value, "#2962ff")))
        .collect();
    super::legacy_levels_to_upstream(&mut fork);
    assert!(fork.levels.len() <= crate::MAX_DRAWING_LEVELS);
    let visible = fork
        .levels
        .iter()
        .filter(|level| level.visible)
        .map(|level| level.value)
        .collect::<Vec<_>>();
    assert_eq!(visible, [0.0, 0.25, 0.5, 0.75, 1.0]);
}

#[test]
fn armed_tools_place_every_kind_with_a_guide_between_clicks() {
    let mut chart = chart();
    for kind in KINDS {
        assert!(chart.set_drawing_tool(Some(kind), Some(r##"{"color":"#123456"}"##), None));
        let clicks = match kind.anchor_count() {
            1 => vec![(300.0, 200.0)],
            2 => vec![(200.0, 260.0), (420.0, 150.0)],
            _ => vec![(160.0, 300.0), (360.0, 140.0), (460.0, 240.0)],
        };
        let mut created = None;
        for (index, &(x, y)) in clicks.iter().enumerate() {
            if index == 1 && clicks.len() == 3 {
                // Between the first and second click of a three-anchor tool the placed anchor
                // and the preview join in a guide in the drawing's stroke.
                chart.drawing_tool_pointer_move(x, y, DrawingModifiers::default(), false);
                let guide = polylines(&mut chart, INK);
                assert!(
                    guide.iter().any(|(points, ..)| points.len() == 2
                        && close(points[0], clicks[0], 1e-3)
                        && close(points[1], (x, y), 1e-3)),
                    "{kind:?} guide: {guide:?}"
                );
            }
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
fn indexed_hit_testing_matches_brute_force() {
    let mut chart = chart();
    for copy in 0..3 {
        let shift = copy as f64 * 0.9;
        for kind in KINDS {
            let points = anchors(kind)
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
fn anchors_resolve_by_time_across_an_interval_switch() {
    let mut chart = chart();
    let id = chart
        .add_drawing_anchors(
            DrawingKind::SchiffPitchfork,
            0,
            &[
                DrawingAnchor {
                    logical: None,
                    price: 101.0,
                    time: Some(8.0 * HOUR),
                },
                DrawingAnchor {
                    logical: None,
                    price: 105.0,
                    time: Some(16.0 * HOUR),
                },
                DrawingAnchor {
                    logical: None,
                    price: 102.0,
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
    assert_eq!(anchors[0].logical, Some(16.0));
    assert_eq!(anchors[1].logical, Some(32.0));
    assert_eq!(anchors[2].logical, Some(41.0));
    assert_eq!(anchors[2].time, Some(20.5 * HOUR));
}

#[test]
fn persistence_round_trips_every_tool_and_omits_kind_defaults() {
    let mut chart = chart();
    for kind in KINDS {
        add(&mut chart, kind, anchors(kind), "{}");
    }
    let exported: serde_json::Value =
        serde_json::from_str(&chart.export_state_json().unwrap()).unwrap();
    for drawing in exported["drawings"].as_array().unwrap() {
        let style = &drawing["style"];
        for field in ["levels", "fill_enabled", "extend_right", "tool_options"] {
            assert!(
                style.get(field).is_none(),
                "{} writes its default {field}",
                drawing["kind"]
            );
        }
    }
    let customized = [
        (
            DrawingKind::AndrewsPitchfork,
            r#"{"extend_right":true,"fill_enabled":false}"#,
        ),
        (DrawingKind::SchiffPitchfork, r#"{"levels":[]}"#),
        (
            DrawingKind::GannBox,
            r#"{"tool_options":{"gann":{"show_angles":true,"reverse":true}}}"#,
        ),
        (
            DrawingKind::GannSquareFixed,
            r#"{"tool_options":{"gann":{"size_bars":35,"scale_ratio":0.4}}}"#,
        ),
        (
            DrawingKind::GannFan,
            r#"{"extend_right":false,"style":"dashed"}"#,
        ),
    ];
    for (kind, options) in customized {
        add(&mut chart, kind, anchors(kind), options);
    }
    let document = chart.export_state_json().unwrap();
    let mut restored = ChartEngine::new(800.0, 500.0, 1.0);
    restored.import_state_json(&document).unwrap();
    assert_eq!(restored.export_state_json().unwrap(), document);
    for (restored, source) in restored.drawings().iter().zip(chart.drawings()) {
        assert_eq!(restored.kind, source.kind);
        assert_eq!(restored.levels, source.levels);
        assert_eq!(restored.fill_enabled, source.fill_enabled);
        assert_eq!(restored.extend_right, source.extend_right);
        assert_eq!(restored.tool_options, source.tool_options);
        assert_eq!(restored.color, source.color);
    }
    let cleared = restored
        .drawings()
        .iter()
        .find(|drawing| drawing.kind == DrawingKind::SchiffPitchfork && drawing.levels.is_empty());
    assert!(cleared.is_some(), "a cleared level list stays cleared");
}

#[test]
fn clipboard_and_sync_payloads_carry_family_options() {
    let mut source = chart();
    let id = add(
        &mut source,
        DrawingKind::GannSquare,
        anchors(DrawingKind::GannSquare),
        r#"{"tool_options":{"gann":{"reverse":true,"show_stats":false}},"fill_enabled":false}"#,
    );
    let copied = source.copy_drawings_json(&[id]).unwrap();
    let mut target = chart();
    let pasted = target.paste_drawings_json(&copied, 0, 0.0, 0.0).unwrap();
    let drawing = target.drawing(pasted[0]).unwrap();
    assert_eq!(drawing.kind, DrawingKind::GannSquare);
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
    assert_eq!(
        mirror.drawings()[0].levels,
        source.drawing(id).unwrap().levels
    );
}

#[test]
fn tools_tolerate_charts_without_data_and_degenerate_anchors() {
    let mut empty = ChartEngine::new(800.0, 500.0, 1.0);
    for kind in KINDS {
        assert!(empty.add_drawing(kind, 0, anchors(kind), None).is_some());
    }
    empty.build_frame();
    assert_eq!(empty.hit_test_drawing(100.0, 100.0), None);

    let mut chart = chart();
    for kind in KINDS {
        let points = vec![p(12.0, 102.0); kind.anchor_count()];
        assert!(chart.add_drawing(kind, 0, points, None).is_some());
        // Every level and arc value at once, including out-of-range ones.
        let id = chart.add_drawing(kind, 0, anchors(kind), None).unwrap();
        assert!(chart.drawing_apply_options(
            id,
            r##"{"extend_left":true,"extend_right":true,"levels":[{"value":-3,"color":"#fff","visible":true,"style":"dotted","fill_between":true,"label_visible":true},{"value":40,"color":"#fff","visible":true,"style":"dashed","fill_between":true,"label_visible":true}]}"##
        ));
    }
    let frame = chart.build_frame();
    assert!(frame.panes[0]
        .points
        .iter()
        .all(|point| point[0].is_finite() && point[1].is_finite()));
}

#[test]
fn z_order_and_templates_follow_the_common_contract() {
    let mut chart = chart();
    let lower = add(
        &mut chart,
        DrawingKind::GannBox,
        anchors(DrawingKind::GannBox),
        "{}",
    );
    let upper = add(
        &mut chart,
        DrawingKind::GannBox,
        anchors(DrawingKind::GannBox),
        r##"{"color":"#123456"}"##,
    );
    let (a, b) = (anchor(&chart, lower, 0), anchor(&chart, lower, 1));
    let level = (a.0 + (b.0 - a.0) * 0.4, a.1 + (b.1 - a.1) * 0.5);
    chart.build_frame();
    let top = |chart: &ChartEngine| chart.hit_test_drawing(level.0, level.1).map(|hit| hit.id);
    assert_eq!(top(&chart), Some(upper));
    assert!(chart.move_drawing_z_order(upper, -1));
    assert_eq!(top(&chart), Some(lower));
    assert!(chart.undo_drawing());
    assert_eq!(top(&chart), Some(upper));

    // A named template carries the typed Gann block and the level list to another Gann box only.
    assert!(chart.drawing_apply_options(
        upper,
        r#"{"tool_options":{"gann":{"show_angles":true,"reverse":true}},"fill_enabled":false}"#
    ));
    let template = chart.drawing_template_json(upper, "reversed").unwrap();
    assert!(chart.apply_drawing_template_json(lower, &template));
    let (source, target) = (
        chart.drawing(upper).unwrap().clone(),
        chart.drawing(lower).unwrap(),
    );
    assert_eq!(target.tool_options, source.tool_options);
    assert_eq!(target.levels, source.levels);
    assert!(!target.fill_enabled);
    let fork = add(
        &mut chart,
        DrawingKind::AndrewsPitchfork,
        anchors(DrawingKind::AndrewsPitchfork),
        "{}",
    );
    assert!(!chart.apply_drawing_template_json(fork, &template));
}
