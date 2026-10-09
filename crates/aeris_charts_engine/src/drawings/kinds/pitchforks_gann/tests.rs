//! Pitchforks & Gann family engine tests: catalog defaults, armed placement (with the partial
//! placement guide), shared-part geometry per tool at several device-pixel ratios, hit testing
//! (indexed against brute force, paint-bounded culling), drags, straighten, magnet, keyboard
//! nudges, time identity, schema and kind options, atomic patches with history, persistence with
//! default omission, clipboard, and sync.

use aeris_charts_render::color::Color;
use aeris_charts_render::draw_list::{IRect, LineStyle, Prim};
use aeris_charts_render::shape;

use super::super::super::DrawingTextLayout;
use super::{GannToolOptions, MEDIAN_COLOR};
use crate::{
    ChartEngine, DrawingAnchor, DrawingDragPart, DrawingId, DrawingKind, DrawingMagnetMode,
    DrawingModifiers, DrawingPoint, DrawingPriceScale,
};

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
    assert!(
        frame.panes[0]
            .points
            .iter()
            .all(|point| point[0].is_finite() && point[1].is_finite())
    );
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

// --- re-applied fork features ---------------------------------------------------------------

const PITCHFORKS: [DrawingKind; 5] = [
    DrawingKind::AndrewsPitchfork,
    DrawingKind::SchiffPitchfork,
    DrawingKind::ModifiedSchiffPitchfork,
    DrawingKind::InsidePitchfork,
    DrawingKind::Pitchfan,
];

fn set_points(chart: &mut ChartEngine, id: DrawingId, points: &[DrawingPoint]) -> bool {
    let json = points
        .iter()
        .map(|point| serde_json::json!({"logical": point.logical, "price": point.price}))
        .collect::<Vec<_>>();
    chart.drawing_set_points(id, &serde_json::Value::Array(json).to_string())
}

fn mid(a: (f64, f64), b: (f64, f64)) -> (f64, f64) {
    ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0)
}

fn at_x(chart: &ChartEngine, logical: f64) -> f64 {
    chart.logical_to_coordinate(logical).unwrap()
}

fn at_y(chart: &ChartEngine, price: f64) -> f64 {
    chart.series_price_to_coordinate(0, price).unwrap()
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

fn rects(chart: &mut ChartEngine, keep: impl Fn(Color) -> bool) -> Vec<IRect> {
    let frame = chart.build_frame();
    frame.panes[0]
        .main
        .iter()
        .filter_map(|prim| match prim {
            Prim::Rect { rect, color } if keep(*color) => Some(*rect),
            _ => None,
        })
        .collect()
}

fn part_at(chart: &ChartEngine, (x, y): (f64, f64)) -> Option<(DrawingId, DrawingDragPart)> {
    let hit = chart.hit_test_drawing(x, y);
    assert_eq!(hit, chart.hit_test_drawing_bruteforce(x, y), "({x}, {y})");
    hit.map(|hit| (hit.id, hit.part))
}

fn on_polyline(line: &[(f64, f64)], point: (f64, f64), tolerance: f64) -> bool {
    shape::distance_to_polyline(point, line) <= tolerance
}

fn level_json(value: f64, color: &str, fill_between: bool, fill: &str) -> serde_json::Value {
    let mut level = serde_json::json!({
        "value": value, "color": color, "visible": true, "style": "solid",
        "fill_between": fill_between, "label_visible": true,
    });
    if !fill.is_empty() {
        level["fill_color"] = serde_json::json!(fill);
    }
    level
}

/// The pane-clipped centroids of the band fills the frame paints for the drawings.
fn band_probes(chart: &mut ChartEngine) -> Vec<(f64, f64)> {
    let frame = chart.build_frame();
    let pane = &frame.panes[0];
    let point = |index: u32| {
        let p = pane.points[index as usize];
        (f64::from(p[0]), f64::from(p[1]))
    };
    let area = shape::Rect {
        left: 0.0,
        top: chart.panes[0].top,
        right: chart.pane_w,
        bottom: chart.panes[0].top + chart.panes[0].height,
    };
    pane.main
        .iter()
        .filter_map(|prim| match prim {
            Prim::BandFill {
                upper_first,
                lower_first,
                point_count: 2,
                ..
            } => {
                let quad = [
                    point(*upper_first),
                    point(upper_first + 1),
                    point(lower_first + 1),
                    point(*lower_first),
                ];
                let mut clipped = Vec::new();
                shape::clip_polygon_to_rect(&quad, area, &mut clipped);
                let count = clipped.len() as f64;
                (count >= 3.0).then(|| {
                    let sum = clipped
                        .iter()
                        .fold((0.0, 0.0), |sum, p| (sum.0 + p.0, sum.1 + p.1));
                    (sum.0 / count, sum.1 / count)
                })
            }
            _ => None,
        })
        .collect()
}

/// While selected, every band a pitchfork or the pitchfan fills between its tines is a body
/// target where the frame paints it (probed inside the pane), and a Gann grid's level cells or
/// per-axis bands past its box are too; unselected or unfilled they let the pointer through.
#[test]
fn zone_fills_hit_only_while_selected() {
    let levels = serde_json::json!({
        "fill_enabled": true,
        "levels": [
            level_json(0.0, INK, false, ""),
            level_json(0.5, INK, true, ""),
            level_json(1.0, INK, true, ""),
        ],
    })
    .to_string();
    for kind in PITCHFORKS {
        let mut chart = chart();
        let id = add(&mut chart, kind, anchors(kind), &levels);
        let probes = band_probes(&mut chart);
        assert_eq!(probes.len(), 2, "{kind:?}");
        for &probe in &probes {
            assert_eq!(part_at(&chart, probe), None, "{kind:?} unselected");
            chart.set_selected_drawing(Some(id));
            assert_eq!(
                part_at(&chart, probe),
                Some((id, DrawingDragPart::Body)),
                "{kind:?} selected"
            );
            chart.set_selected_drawing(None);
        }
        assert!(chart.drawing_apply_options(id, r#"{"fill_enabled":false}"#));
        chart.set_selected_drawing(Some(id));
        for &probe in &probes {
            assert_eq!(part_at(&chart, probe), None, "{kind:?} unfilled");
        }
    }

    // Gann grids: a level beyond the box paints a cell (or, with split axes, a band) outside it.
    for (kind, options) in [
        (
            DrawingKind::GannBox,
            serde_json::json!({"fill_enabled": true, "levels": [
                level_json(0.0, INK, false, ""), level_json(1.0, INK, true, ""),
                level_json(1.25, INK, true, ""),
            ]}),
        ),
        (
            DrawingKind::GannBox,
            serde_json::json!({"fill_enabled": true, "levels": [
                level_json(0.0, INK, false, ""), level_json(1.0, INK, true, ""),
                level_json(1.25, INK, true, ""),
            ], "tool_options": {"gann": {"time_levels": [level_json(0.0, INK, false, "")]}}}),
        ),
        (
            DrawingKind::GannSquare,
            serde_json::json!({"fill_enabled": true, "gann_fans": [], "gann_arcs": [], "levels": [
                level_json(0.0, INK, false, ""), level_json(1.0, INK, true, ""),
                level_json(1.25, INK, true, ""),
            ]}),
        ),
    ] {
        let mut chart = chart();
        let id = add(
            &mut chart,
            kind,
            vec![p(10.0, 101.0), p(14.0, 102.0)],
            &options.to_string(),
        );
        let (a, b) = (anchor(&chart, id, 0), anchor(&chart, id, 1));
        // Past the far corner, inside the 1–1.25 cell; with split axes, past the far price
        // edge inside the price band across the box's width.
        let probe = if options["tool_options"].is_object() {
            (a.0 + (b.0 - a.0) * 0.3, a.1 + (b.1 - a.1) * 1.1)
        } else {
            (a.0 + (b.0 - a.0) * 1.1, a.1 + (b.1 - a.1) * 1.1)
        };
        assert_eq!(part_at(&chart, probe), None, "{kind:?} unselected");
        chart.set_selected_drawing(Some(id));
        assert_eq!(
            part_at(&chart, probe),
            Some((id, DrawingDragPart::Body)),
            "{kind:?} {options}"
        );
        assert!(chart.drawing_apply_options(id, r#"{"fill_enabled":false}"#));
        assert_eq!(part_at(&chart, probe), None, "{kind:?} unfilled");
    }
}

/// A Gann box with its own time levels draws them as its only vertical lines, labelled above the
/// box, with its price levels as horizontal lines only, and fills overlapping per-axis bands;
/// without them it renders exactly as upstream does.
#[test]
fn gann_box_time_levels_split_the_axes() {
    let mut chart = chart();
    let eighths = (0..=8)
        .map(|step| level_json(f64::from(step) / 8.0, "#aa0000", true, "#ff000040"))
        .collect::<Vec<_>>();
    let options = serde_json::json!({
        "fill_enabled": true, "levels": eighths, "level_show_values": true,
    });
    let id = add(
        &mut chart,
        DrawingKind::GannBox,
        vec![p(10.0, 101.0), p(20.0, 105.0)],
        &options.to_string(),
    );
    let (a, b) = (anchor(&chart, id, 0), anchor(&chart, id, 1));
    let x_at = |value: f64| a.0 + (b.0 - a.0) * value;
    let y_at = |value: f64| a.1 + (b.1 - a.1) * value;
    let split = |chart: &mut ChartEngine, css: &str| {
        let lines = polylines(chart, css);
        let vertical = lines
            .iter()
            .filter(|(points, ..)| (points[0].0 - points[1].0).abs() < 1e-3)
            .map(|(points, ..)| points[0].0)
            .collect::<Vec<_>>();
        (vertical, lines.len())
    };
    // Upstream: every level draws both lines, and the zones are diagonal cells.
    let (vertical, total) = split(&mut chart, "#aa0000");
    assert_eq!((vertical.len(), total), (9, 18));
    let upstream = chart.build_frame().panes[0].main.clone();
    // The default block changes nothing.
    assert!(chart.drawing_apply_options(id, r#"{"tool_options":{"gann":{"show_angles":false}}}"#));
    assert_eq!(chart.build_frame().panes[0].main, upstream);

    let time = serde_json::json!({"tool_options": {"gann": {"time_levels": [
        level_json(0.0, "#00aa00", true, "#0000ff40"),
        level_json(0.5, "#00aa00", true, "#0000ff40"),
        level_json(1.0, "#00aa00", true, "#0000ff40"),
    ]}}});
    assert!(chart.drawing_apply_options(id, &time.to_string()));
    let (vertical, total) = split(&mut chart, "#aa0000");
    assert!(vertical.is_empty(), "price levels draw horizontally only");
    assert_eq!(total, 9);
    let (vertical, total) = split(&mut chart, "#00aa00");
    assert_eq!(total, 3);
    for value in [0.0, 0.5, 1.0] {
        assert!(
            vertical.iter().any(|x| (x - x_at(value)).abs() < 1e-3),
            "{value}"
        );
    }
    let red = color("#ff000040");
    let blue = color("#0000ff40");
    assert_eq!(rects(&mut chart, |color| color == red).len(), 8);
    assert_eq!(rects(&mut chart, |color| color == blue).len(), 2);
    // Time labels sit above the box, centred on their line.
    let top = a.1.min(b.1);
    let labels = texts(&mut chart);
    assert!(labels.iter().any(|(text, x, y)| text.starts_with("0.5 ")
        && (f64::from(*x) - x_at(0.5)).abs() < 1e-3
        && (f64::from(*y) - (top - 8.0)).abs() < 1e-3));
    // A box just left of the view whose time label alone reaches into it stays a culling
    // candidate.
    let left = add(
        &mut chart,
        DrawingKind::GannBox,
        vec![p(-12.0, 101.0), p(-2.0, 105.0)],
        &serde_json::json!({"levels": [], "level_show_values": true, "tool_options": {"gann": {
            "time_levels": [level_json(1.0, "#00aa00", false, "")]}}})
        .to_string(),
    );
    assert!(at_x(&chart, -2.0) < -25.0);
    assert!(chart.take_drawing_candidates(0, None).contains(&left));
    chart.remove_drawing(left);
    // A split box's price level past [0, 1] keeps upstream's label at the diagonal point, and
    // the box stays a candidate while only that label is in view.
    let mut hidden_label = level_json(0.0, "#00aa00", false, "");
    hidden_label["label_visible"] = serde_json::json!(false);
    let beyond = add(
        &mut chart,
        DrawingKind::GannBox,
        vec![p(-20.0, 101.0), p(-4.0, 105.0)],
        &serde_json::json!({"levels": [level_json(1.5, "#0000aa", false, "")],
            "level_show_values": true,
            "tool_options": {"gann": {"time_levels": [hidden_label]}}})
        .to_string(),
    );
    let label_x = at_x(&chart, 4.0);
    assert!(at_x(&chart, -4.0) < -25.0 && label_x > 25.0);
    assert!(
        texts(&mut chart)
            .iter()
            .any(|(text, x, _)| text.starts_with("1.5 ") && (f64::from(*x) - label_x).abs() < 1e-3)
    );
    assert!(chart.take_drawing_candidates(0, None).contains(&beyond));
    chart.remove_drawing(beyond);
    // Hits: the time line at 0.5 and every price line; no vertical line at 0.25 any more.
    let between = y_at(0.5625);
    assert_eq!(
        part_at(&chart, (x_at(0.5), between)),
        Some((id, DrawingDragPart::Body))
    );
    assert_eq!(part_at(&chart, (x_at(0.25), between)), None);
    assert_eq!(
        part_at(&chart, (x_at(0.3), y_at(0.25))),
        Some((id, DrawingDragPart::Body))
    );
    // Reversed, the time levels count from the second anchor.
    let reversed = serde_json::json!({"level_reverse": true, "tool_options": {"gann": {
        "time_levels": [level_json(0.0, "#00aa00", true, ""), level_json(0.25, "#00aa00", true, "")]
    }}});
    assert!(chart.drawing_apply_options(id, &reversed.to_string()));
    let (vertical, _) = split(&mut chart, "#00aa00");
    for value in [1.0, 0.75] {
        assert!(
            vertical.iter().any(|x| (x - x_at(value)).abs() < 1e-3),
            "{value}: {vertical:?}"
        );
    }
}

/// A Gann box strokes its `tool_options.gann.angles` from the pivot corner while `show_angles` is
/// on: the 1×1 is the box diagonal and steeper angles end on the far price edge.
#[test]
fn gann_box_angles_follow_show_angles() {
    let mut chart = chart();
    let id = add(
        &mut chart,
        DrawingKind::GannBox,
        vec![p(10.0, 101.0), p(20.0, 105.0)],
        r#"{"levels":[]}"#,
    );
    let (a, b) = (anchor(&chart, id, 0), anchor(&chart, id, 1));
    let diagonal = |chart: &mut ChartEngine, from, to| {
        polylines(chart, "#787b86")
            .iter()
            .any(|(points, ..)| close(points[0], from, 1e-3) && close(points[1], to, 1e-3))
    };
    assert!(!diagonal(&mut chart, a, b), "off by default");
    assert!(chart.drawing_apply_options(id, r#"{"tool_options":{"gann":{"show_angles":true}}}"#));
    assert!(diagonal(&mut chart, a, b));
    let steep_end = (a.0 + (b.0 - a.0) / 2.0, b.1);
    assert!(
        polylines(&mut chart, "#2962ff")
            .iter()
            .any(|(points, ..)| close(points[0], a, 1e-3) && close(points[1], steep_end, 1e-3))
    );
    // Unfilled, and a body target along the line.
    let on_steep = mid(a, steep_end);
    assert_eq!(part_at(&chart, on_steep), Some((id, DrawingDragPart::Body)));
    assert!(chart.drawing_apply_options(id, r#"{"tool_options":{"gann":{"show_angles":false}}}"#));
    assert_eq!(part_at(&chart, on_steep), None);
    // Reversed, the angles grow from the second anchor.
    assert!(chart.drawing_apply_options(
        id,
        r#"{"level_reverse":true,"tool_options":{"gann":{"show_angles":true}}}"#
    ));
    assert!(diagonal(&mut chart, b, a));
}

/// A Gann square's stats box (`show_stats`, off for new squares) prints the price range, the
/// bars and the price per bar beside the far corner in the shared translucent stats box, which is
/// a body target and pads culling; a square without width reports 0 bars and no per-bar line.
#[test]
fn gann_square_stats_box_measures_range_bars_and_ratio() {
    let mut chart = chart();
    let id = add(
        &mut chart,
        DrawingKind::GannSquare,
        vec![p(10.0, 101.0), p(20.0, 106.0)],
        r##"{"color":"#123456"}"##,
    );
    let lines = |chart: &mut ChartEngine| {
        texts(chart)
            .into_iter()
            .map(|(text, ..)| text)
            .collect::<Vec<_>>()
    };
    assert!(!lines(&mut chart).iter().any(|text| text.ends_with("bars")));
    assert!(chart.drawing_apply_options(id, r#"{"tool_options":{"gann":{"show_stats":true}}}"#));
    let stats = lines(&mut chart);
    for line in ["5.00", "10 bars", "0.5000/bar"] {
        assert!(stats.contains(&line.to_string()), "{line}: {stats:?}");
    }
    // The translucent box (the stroke at STATS_ALPHA) paints last, right of the far corner.
    let ink = color(INK);
    let wash = Color::rgba(
        ink.r(),
        ink.g(),
        ink.b(),
        crate::drawings::parts::STATS_ALPHA,
    );
    let frame = chart.build_frame();
    let main = &frame.panes[0].main;
    let index = main
        .iter()
        .position(|prim| matches!(prim, Prim::Rect { color, .. } if *color == wash))
        .expect("stats box");
    let after = main[index + 1..]
        .iter()
        .take_while(|prim| matches!(prim, Prim::Text { .. }))
        .count();
    assert_eq!(after, 3, "its three lines follow the box");
    assert!(
        main[..index]
            .iter()
            .any(|prim| matches!(prim, Prim::Polyline { .. }))
            && !main[index..]
                .iter()
                .any(|prim| matches!(prim, Prim::Polyline { color, .. } if *color == ink)),
        "the box paints after the grid"
    );
    let Prim::Rect { rect, .. } = main[index] else {
        unreachable!()
    };
    let b = anchor(&chart, id, 1);
    assert!(f64::from(rect.x) >= b.0 + 7.0, "{rect:?} right of {b:?}");
    let center = (
        f64::from(rect.x) + f64::from(rect.w) / 2.0,
        f64::from(rect.y) + f64::from(rect.h) / 2.0,
    );
    assert_eq!(part_at(&chart, center), Some((id, DrawingDragPart::Body)));
    assert!(chart.drawing_apply_options(id, r#"{"tool_options":{"gann":{"show_stats":false}}}"#));
    assert_eq!(part_at(&chart, center), None);
    assert!(!lines(&mut chart).iter().any(|text| text.ends_with("bars")));

    // A square without width: 0 bars and no per-bar line.
    let flat = add(
        &mut chart,
        DrawingKind::GannSquare,
        vec![p(12.0, 102.0), p(12.0, 104.0)],
        r#"{"tool_options":{"gann":{"show_stats":true}}}"#,
    );
    let stats = lines(&mut chart);
    assert!(stats.contains(&"0 bars".to_string()), "{stats:?}");
    assert!(
        !stats.iter().any(|text| text.ends_with("/bar")),
        "{stats:?}"
    );
    chart.remove_drawing(flat);

    // The fixed square with a scale ratio measures its ratio corner.
    let fixed = add(
        &mut chart,
        DrawingKind::GannSquareFixed,
        vec![p(10.0, 101.0), p(30.0, 140.0)],
        r#"{"tool_options":{"gann":{"show_stats":true,"scale_ratio":0.25}}}"#,
    );
    let stats = lines(&mut chart);
    for line in ["5.00", "20 bars", "0.2500/bar"] {
        assert!(stats.contains(&line.to_string()), "{line}: {stats:?}");
    }
    chart.remove_drawing(fixed);

    // A square left of the view whose stats box alone reaches into it stays a candidate.
    let left = add(
        &mut chart,
        DrawingKind::GannSquare,
        vec![p(-12.0, 101.0), p(-2.0, 106.0)],
        r#"{"tool_options":{"gann":{"show_stats":true}}}"#,
    );
    assert!(at_x(&chart, -2.0) < 0.0);
    let frame = chart.build_frame();
    let ink = chart.drawing(left).unwrap().stroke_color();
    let wash = Color::rgba(
        ink.r(),
        ink.g(),
        ink.b(),
        crate::drawings::parts::STATS_ALPHA,
    );
    let rect = frame.panes[0]
        .main
        .iter()
        .find_map(|prim| match prim {
            Prim::Rect { rect, color } if *color == wash => Some(*rect),
            _ => None,
        })
        .expect("the stats box paints");
    let inside = (
        f64::from(rect.x + rect.w) - 3.0,
        f64::from(rect.y) + f64::from(rect.h) / 2.0,
    );
    assert!(inside.0 > 0.0);
    assert_eq!(part_at(&chart, inside), Some((left, DrawingDragPart::Body)));
}

/// A Gann fan's `scale_ratio` sets its 1×1 slope in price per bar through the derived render
/// point: paint, hit testing and culling follow it, and clearing it restores the anchor slope.
#[test]
fn gann_fan_scale_ratio_sets_the_one_by_one_slope() {
    let mut chart = chart();
    let one =
        serde_json::json!({"fill_enabled": false, "levels": [level_json(1.0, INK, false, "")]});
    let id = add(
        &mut chart,
        DrawingKind::GannFan,
        vec![p(10.0, 101.0), p(20.0, 104.0)],
        &one.to_string(),
    );
    let through = |chart: &mut ChartEngine, point: (f64, f64)| {
        polylines(chart, INK)
            .iter()
            .any(|(line, ..)| on_polyline(line, point, 1e-3))
    };
    let px = |chart: &ChartEngine, logical, price| {
        chart
            .drawing_to_px_for(0, DrawingPriceScale::Right, p(logical, price))
            .unwrap()
    };
    assert!({
        let point = px(&chart, 20.0, 104.0);
        through(&mut chart, point)
    });
    assert!(chart.drawing_apply_options(id, r#"{"tool_options":{"gann":{"scale_ratio":0.2}}}"#));
    let target = px(&chart, 20.0, 103.0);
    assert!(through(&mut chart, target));
    assert!(!{
        let point = px(&chart, 20.0, 104.0);
        through(&mut chart, point)
    });
    let on_line = mid(anchor(&chart, id, 0), target);
    assert_eq!(part_at(&chart, on_line), Some((id, DrawingDragPart::Body)));
    // A downward second anchor turns the ratio's slope down.
    assert!(set_points(
        &mut chart,
        id,
        &[p(10.0, 104.0), p(20.0, 101.0)]
    ));
    assert!({
        let point = px(&chart, 20.0, 102.0);
        through(&mut chart, point)
    });
    // A flat fan reaches each multiple's own slope.
    let flat = serde_json::json!({"fill_enabled": false, "levels": [level_json(4.0, "#9c27b0", false, "")],
        "tool_options": {"gann": {"scale_ratio": 0.25}}});
    let fan = add(
        &mut chart,
        DrawingKind::GannFan,
        vec![p(10.0, 101.0), p(14.0, 101.0)],
        &flat.to_string(),
    );
    let end = px(&chart, 14.0, 105.0);
    let steep = polylines(&mut chart, "#9c27b0");
    assert!(steep.iter().any(|(line, ..)| on_polyline(line, end, 1e-3)));
    chart.remove_drawing(fan);
    // Clearing the ratio restores the anchor slope.
    assert!(chart.drawing_apply_options(id, r#"{"tool_options":{"gann":{"scale_ratio":null}}}"#));
    assert!({
        let point = px(&chart, 20.0, 101.0);
        through(&mut chart, point)
    });
    // On a logarithmic scale the geometry stays finite and both hit paths agree.
    assert!(chart.drawing_apply_options(id, r#"{"tool_options":{"gann":{"scale_ratio":50}}}"#));
    chart.set_price_scale_mode(0, false, crate::PriceScaleMode::Logarithmic);
    let frame = chart.build_frame();
    assert!(
        frame.panes[0]
            .points
            .iter()
            .all(|point| point[0].is_finite() && point[1].is_finite())
    );
    for gy in 0..25 {
        for gx in 0..40 {
            part_at(
                &chart,
                (f64::from(gx) * 20.0 + 3.0, f64::from(gy) * 20.0 + 4.0),
            );
        }
    }
}

/// A fixed square's `scale_ratio` fixes its price side (the bars between the anchors times the
/// ratio, toward the second anchor's price), and its culling bounds cover the ratio corner past
/// the anchors' box; without a ratio it stays upstream's screen square.
#[test]
fn fixed_gann_square_scale_ratio_fixes_the_price_side() {
    use aeris_charts_core::model::price_range::PriceRange;
    let mut chart = chart();
    let edges = serde_json::json!({"fill_enabled": false, "gann_fans": [], "gann_arcs": [],
        "levels": [level_json(0.0, INK, false, ""), level_json(1.0, INK, false, "")]});
    let id = add(
        &mut chart,
        DrawingKind::GannSquareFixed,
        vec![p(10.0, 101.0), p(30.0, 140.0)],
        &edges.to_string(),
    );
    let upstream = chart.build_frame().panes[0].main.clone();
    let far_edge = |chart: &mut ChartEngine, x: f64| {
        polylines(chart, INK)
            .into_iter()
            .find(|(line, ..)| (line[0].0 - x).abs() < 1e-3 && (line[1].0 - x).abs() < 1e-3)
            .map(|(line, ..)| (line[0].1.min(line[1].1), line[0].1.max(line[1].1)))
    };
    assert!(chart.drawing_apply_options(id, r#"{"tool_options":{"gann":{"scale_ratio":0.25}}}"#));
    let x = at_x(&chart, 30.0);
    let (top, bottom) = far_edge(&mut chart, x).expect("far edge at the second anchor's bar");
    assert!((top - at_y(&chart, 106.0)).abs() < 1e-3);
    assert!((bottom - at_y(&chart, 101.0)).abs() < 1e-3);
    // Growing down with the second anchor below.
    assert!(set_points(&mut chart, id, &[p(10.0, 101.0), p(30.0, 90.0)]));
    let (top, _) = far_edge(&mut chart, x).unwrap();
    assert!((top - at_y(&chart, 101.0)).abs() < 1e-3);
    let (_, bottom) = far_edge(&mut chart, x).unwrap();
    assert!((bottom - at_y(&chart, 96.0)).abs() < 1e-3);
    // Without a ratio: upstream's square, prim for prim.
    assert!(set_points(
        &mut chart,
        id,
        &[p(10.0, 101.0), p(30.0, 140.0)]
    ));
    assert!(chart.drawing_apply_options(id, r#"{"tool_options":{"gann":{"scale_ratio":null}}}"#));
    assert_eq!(chart.build_frame().panes[0].main, upstream);

    // Culling: the anchors' box sits below the view while the ratio square's top edge shows.
    let mut chart = chart_with(800.0, &hourly(40), 1.0);
    let id = add(
        &mut chart,
        DrawingKind::GannSquareFixed,
        vec![p(10.0, 101.0), p(30.0, 102.0)],
        &serde_json::json!({"fill_enabled": false, "gann_fans": [], "gann_arcs": [],
            "levels": [level_json(0.0, INK, false, ""), level_json(1.0, INK, false, "")],
            "tool_options": {"gann": {"scale_ratio": 0.25}}})
        .to_string(),
    );
    chart.panes[0].price_scale.set_auto_scale(false);
    chart.panes[0]
        .price_scale
        .set_price_range(Some(PriceRange::new(103.5, 110.0)));
    chart.build_frame();
    assert!(at_y(&chart, 102.0) > chart.panes[0].top + chart.panes[0].height);
    let probe = (at_x(&chart, 30.0), at_y(&chart, 105.0));
    assert_eq!(part_at(&chart, probe), Some((id, DrawingDragPart::Body)));
}

/// A pitchfork's (and the pitchfan's) fourth handle sits on its base midpoint between B and C,
/// paints with the anchors' discs, and moves B and C together by pointer (one undo step, magnet
/// snapped) and by keyboard.
#[test]
fn a_pitchfork_base_midpoint_handle_moves_both_handle_anchors() {
    let mut chart = chart();
    for kind in PITCHFORKS {
        let id = add(&mut chart, kind, anchors(kind), "{}");
        chart.set_selected_drawing(Some(id));
        let before = chart.drawing(id).unwrap().points.clone();
        let (a, b, c) = (
            anchor(&chart, id, 0),
            anchor(&chart, id, 1),
            anchor(&chart, id, 2),
        );
        let base = mid(b, c);
        assert_eq!(chart.drawing_handle_count(id), Some(4), "{kind:?}");
        let frame = chart.build_frame();
        assert!(
            frame.panes[0].main.iter().any(|prim| matches!(
                prim,
                Prim::Circle { cx, cy, .. }
                    if close((f64::from(*cx), f64::from(*cy)), base, 1e-3)
            )),
            "{kind:?} paints the base handle"
        );
        let hit = chart.hit_test_drawing(base.0, base.1).unwrap();
        assert_eq!(
            (hit.part, hit.cursor),
            (DrawingDragPart::Handle(0), "pointer"),
            "{kind:?}"
        );
        // A pointer drag translates B and C by the midpoint's move, as one undo step.
        assert!(chart.drawing_drag_start_at(base.0, base.1));
        chart.drawing_drag_to(base.0 + 20.0, base.1 - 12.0, DrawingModifiers::default());
        chart.drawing_drag_end();
        assert!(close(anchor(&chart, id, 0), a, 1e-9), "{kind:?}");
        assert!(close(anchor(&chart, id, 1), (b.0 + 20.0, b.1 - 12.0), 1e-3));
        assert!(close(anchor(&chart, id, 2), (c.0 + 20.0, c.1 - 12.0), 1e-3));
        assert!(chart.undo_drawing());
        assert_eq!(chart.drawing(id).unwrap().points, before);
        // Keyboard: the fourth handle nudges the base the same way.
        assert!(chart.nudge_selected_drawing(0.0, 10.0, Some(3)));
        assert!(close(anchor(&chart, id, 1), (b.0, b.1 + 10.0), 1e-3));
        assert!(close(anchor(&chart, id, 2), (c.0, c.1 + 10.0), 1e-3));
        assert!(close(anchor(&chart, id, 0), a, 1e-9));
        assert!(chart.undo_drawing());
        // A cancelled drag restores the anchors.
        assert!(chart.drawing_drag_start_at(base.0, base.1));
        chart.drawing_drag_to(base.0 + 30.0, base.1, DrawingModifiers::default());
        chart.drawing_drag_cancel();
        assert_eq!(chart.drawing(id).unwrap().points, before);
        chart.remove_drawing(id);
    }

    // The magnet snaps the midpoint to the bar value under the pointer; the base follows it.
    let id = add(
        &mut chart,
        DrawingKind::AndrewsPitchfork,
        anchors(DrawingKind::AndrewsPitchfork),
        "{}",
    );
    chart.set_selected_drawing(Some(id));
    let base = mid(anchor(&chart, id, 1), anchor(&chart, id, 2));
    let to = (at_x(&chart, 24.0) + 2.0, base.1 + 7.0);
    chart.set_drawing_magnet_mode(DrawingMagnetMode::Strong);
    assert!(chart.drawing_drag_start_at(base.0, base.1));
    chart.drawing_drag_to(to.0, to.1, DrawingModifiers::default());
    chart.drawing_drag_end();
    chart.set_drawing_magnet_mode(DrawingMagnetMode::Off);
    let snapped = chart
        .drawing_to_px_for(0, DrawingPriceScale::Right, p(24.0, 103.0))
        .unwrap();
    assert!(close(
        mid(anchor(&chart, id, 1), anchor(&chart, id, 2)),
        snapped,
        1e-3
    ));
}

/// With `snap_time_to_data` the base midpoint translates B and C by one whole-bar shift: a base
/// an odd number of bars wide (its midpoint half a bar off the grid) keeps its width at
/// fractional bar spacing, and a drag under a bar either way leaves the bars where they were.
#[test]
fn a_time_snapped_pitchfork_base_keeps_its_width_on_small_drags() {
    let mut chart = chart();
    chart.set_bar_spacing(13.37);
    chart.set_right_offset(2.31);
    chart.build_frame();
    let id = add(
        &mut chart,
        DrawingKind::AndrewsPitchfork,
        vec![p(8.0, 101.0), p(16.0, 105.0), p(21.0, 102.0)],
        r#"{"snap_time_to_data":true}"#,
    );
    chart.set_selected_drawing(Some(id));
    let spacing = at_x(&chart, 1.0) - at_x(&chart, 0.0);
    for dx in [
        1.0,
        -1.0,
        0.4 * spacing,
        -0.4 * spacing,
        0.9 * spacing,
        -0.9 * spacing,
    ] {
        let base = mid(anchor(&chart, id, 1), anchor(&chart, id, 2));
        assert!(chart.drawing_drag_start_at(base.0, base.1));
        chart.drawing_drag_to(base.0 + dx, base.1 - 3.0, DrawingModifiers::default());
        chart.drawing_drag_end();
        let points = chart.drawing(id).unwrap().points.clone();
        assert_eq!((points[1].logical, points[2].logical), (16.0, 21.0), "{dx}");
        assert!(
            points[1].price > 105.0,
            "{dx}: the price follows the pointer"
        );
        assert!(chart.undo_drawing());
    }
    // A drag past a bar shifts both by whole bars.
    for (bars, expected) in [(1.6, 1.0), (-1.6, -1.0), (3.2, 3.0)] {
        let base = mid(anchor(&chart, id, 1), anchor(&chart, id, 2));
        assert!(chart.drawing_drag_start_at(base.0, base.1));
        chart.drawing_drag_to(base.0 + bars * spacing, base.1, DrawingModifiers::default());
        chart.drawing_drag_end();
        let points = chart.drawing(id).unwrap().points.clone();
        assert_eq!(
            (points[1].logical, points[2].logical),
            (16.0 + expected, 21.0 + expected),
            "{bars}"
        );
        assert!(chart.undo_drawing());
    }
}

/// A time-snapped derived-handle sample that would put an anchor past the data is rejected
/// whole and the drag keeps its last valid sample: the pitchfork base never moves B without C,
/// and the fixed square's corner never sizes its second anchor past the last bar.
#[test]
fn a_time_snapped_derived_handle_drag_rejects_samples_past_the_data() {
    let mut chart = chart();
    let fork = add(
        &mut chart,
        DrawingKind::AndrewsPitchfork,
        vec![p(20.0, 101.0), p(30.0, 105.0), p(36.0, 102.0)],
        r#"{"snap_time_to_data":true}"#,
    );
    chart.set_selected_drawing(Some(fork));
    let spacing = at_x(&chart, 1.0) - at_x(&chart, 0.0);
    let base = mid(anchor(&chart, fork, 1), anchor(&chart, fork, 2));
    let logicals = |chart: &ChartEngine| {
        let points = &chart.drawing(fork).unwrap().points;
        (points[1].logical, points[2].logical)
    };
    // C would land on bar 40 (the data ends at 39): nothing moves.
    assert!(chart.drawing_drag_start_at(base.0, base.1));
    chart.drawing_drag_to(base.0 + 4.2 * spacing, base.1, DrawingModifiers::default());
    assert_eq!(logicals(&chart), (30.0, 36.0));
    chart.drawing_drag_end();
    assert_eq!(logicals(&chart), (30.0, 36.0));
    // A valid sample stands when a later one is rejected.
    assert!(chart.drawing_drag_start_at(base.0, base.1));
    chart.drawing_drag_to(base.0 + 2.2 * spacing, base.1, DrawingModifiers::default());
    assert_eq!(logicals(&chart), (32.0, 38.0));
    chart.drawing_drag_to(base.0 + 4.2 * spacing, base.1, DrawingModifiers::default());
    chart.drawing_drag_end();
    assert_eq!(logicals(&chart), (32.0, 38.0));
    chart.remove_drawing(fork);

    let square = add(
        &mut chart,
        DrawingKind::GannSquareFixed,
        vec![p(30.0, 101.0), p(34.0, 200.0)],
        r#"{"snap_time_to_data":true}"#,
    );
    chart.set_selected_drawing(Some(square));
    let pivot = anchor(&chart, square, 0);
    let corner_at = |chart: &ChartEngine, bars: f64| {
        let x = at_x(chart, 30.0 + bars);
        (x, pivot.1 - (x - pivot.0))
    };
    let corner = corner_at(&chart, 4.0);
    assert_eq!(
        chart
            .hit_test_drawing(corner.0, corner.1)
            .map(|hit| hit.part),
        Some(DrawingDragPart::Handle(0))
    );
    let second = |chart: &ChartEngine| chart.drawing(square).unwrap().points[1];
    // A mostly vertical drag sizes the side past bar 39: rejected.
    assert!(chart.drawing_drag_start_at(corner.0, corner.1));
    chart.drawing_drag_to(
        at_x(&chart, 35.0),
        corner.1 - 300.0,
        DrawingModifiers::default(),
    );
    chart.drawing_drag_end();
    assert_eq!(second(&chart), p(34.0, 200.0));
    // A square drag to bar 36 stands when the next sample would pass the data.
    assert!(chart.drawing_drag_start_at(corner.0, corner.1));
    let to = corner_at(&chart, 6.0);
    chart.drawing_drag_to(to.0, to.1, DrawingModifiers::default());
    assert_eq!(second(&chart), p(36.0, 200.0));
    chart.drawing_drag_to(
        at_x(&chart, 35.0),
        corner.1 - 300.0,
        DrawingModifiers::default(),
    );
    chart.drawing_drag_end();
    assert_eq!(second(&chart), p(36.0, 200.0));
}

fn gann(chart: &ChartEngine, id: DrawingId) -> GannToolOptions {
    chart
        .drawing(id)
        .unwrap()
        .tool_options
        .gann
        .clone()
        .unwrap_or_default()
}

/// The fixed square's second handle sits on its painted far corner and resizes it in whole bars:
/// without a ratio by the corner's larger distance, the second anchor kept beyond the corner
/// (its own price when it already is, else far beyond) so the square keeps its bars when the
/// price axis zooms out moderately; with one through the corner's price
/// (Shift keeps the press ratio). Undo and cancel restore the anchors and the options.
#[test]
fn a_fixed_gann_square_resizes_from_its_corner_handle() {
    use aeris_charts_core::model::price_range::PriceRange;
    let mut chart = chart();
    let id = add(
        &mut chart,
        DrawingKind::GannSquareFixed,
        vec![p(10.0, 101.0), p(20.0, 105.5)],
        "{}",
    );
    chart.set_selected_drawing(Some(id));
    assert_eq!(chart.drawing_handle_count(id), Some(2));
    let pivot = anchor(&chart, id, 0);
    let corner_at = |chart: &ChartEngine, bars: f64| {
        let x = at_x(chart, 10.0 + bars);
        (x, pivot.1 - (x - pivot.0))
    };
    let raw = anchor(&chart, id, 1);
    let corner = corner_at(&chart, 10.0);
    assert!(raw.1 < corner.1 - 20.0, "the time side is the smaller one");
    let hit = chart.hit_test_drawing(corner.0, corner.1).unwrap();
    assert_eq!(
        (hit.part, hit.cursor),
        (DrawingDragPart::Handle(0), "nesw-resize")
    );
    assert!(
        !matches!(
            chart.hit_test_drawing(raw.0, raw.1).map(|hit| hit.part),
            Some(DrawingDragPart::Anchor(_) | DrawingDragPart::Handle(_))
        ),
        "the raw second anchor is no handle"
    );

    // The corner's larger distance sets the side in whole bars.
    let before = chart.drawing(id).unwrap().points.clone();
    let x24 = at_x(&chart, 24.0);
    let to = (x24 + 3.0, pivot.1 - (x24 - pivot.0) * 0.5);
    assert!(chart.drawing_drag_start_at(corner.0, corner.1));
    chart.drawing_drag_to(to.0, to.1, DrawingModifiers::default());
    chart.drawing_drag_end();
    let points = chart.drawing(id).unwrap().points.clone();
    assert_eq!(points[0], before[0]);
    assert_eq!(points[1].logical, 24.0);
    assert!(
        at_y(&chart, 105.5) > corner_at(&chart, 14.0).1,
        "the press price lies inside the new side"
    );
    assert!(points[1].price > 105.5, "moved far above the corner");
    let corner = corner_at(&chart, 14.0);
    assert_eq!(
        chart
            .hit_test_drawing(corner.0, corner.1)
            .map(|hit| hit.part),
        Some(DrawingDragPart::Handle(0)),
        "the handle follows the new corner"
    );
    // Zooming the price axis out keeps the square 14 bars wide.
    chart.panes[0].price_scale.set_auto_scale(false);
    chart.panes[0]
        .price_scale
        .set_price_range(Some(PriceRange::new(80.0, 130.0)));
    chart.build_frame();
    let pivot_now = anchor(&chart, id, 0);
    let x = at_x(&chart, 24.0);
    let handle = (x, pivot_now.1 - (x - pivot_now.0));
    assert_eq!(
        chart
            .hit_test_drawing(handle.0, handle.1)
            .map(|hit| hit.part),
        Some(DrawingDragPart::Handle(0)),
        "still a 14-bar square"
    );
    chart.panes[0].price_scale.set_auto_scale(true);
    chart.fit_content();
    chart.build_frame();
    assert!(chart.undo_drawing());
    assert_eq!(chart.drawing(id).unwrap().points, before);

    // A press price already beyond the new corner stays: only the bars change.
    let corner = corner_at(&chart, 10.0);
    let x18 = at_x(&chart, 18.0);
    assert!(chart.drawing_drag_start_at(corner.0, corner.1));
    chart.drawing_drag_to(x18, pivot.1 - (x18 - pivot.0), DrawingModifiers::default());
    chart.drawing_drag_end();
    assert_eq!(
        chart.drawing(id).unwrap().points,
        vec![before[0], p(18.0, 105.5)]
    );
    assert!(chart.undo_drawing());

    // Below the anchor it grows downward; a cancelled drag restores the anchors.
    let corner = corner_at(&chart, 10.0);
    assert!(chart.drawing_drag_start_at(corner.0, corner.1));
    chart.drawing_drag_to(
        at_x(&chart, 12.0),
        pivot.1 + 30.0,
        DrawingModifiers::default(),
    );
    let dragged = chart.drawing(id).unwrap().points.clone();
    assert_eq!(dragged[1].logical, 12.0);
    assert!(dragged[1].price < 101.0 && dragged[1].price > 0.0);
    let side = at_x(&chart, 12.0) - pivot.0;
    let below = (at_x(&chart, 12.0), pivot.1 + side);
    assert_eq!(
        chart
            .hit_test_drawing(below.0, below.1)
            .map(|hit| hit.cursor),
        Some("nwse-resize")
    );
    chart.drawing_drag_cancel();
    assert_eq!(chart.drawing(id).unwrap().points, before);

    // With a ratio the corner's price sets it; Shift keeps the press ratio.
    assert!(chart.drawing_apply_options(id, r#"{"tool_options":{"gann":{"scale_ratio":0.25}}}"#));
    assert!(set_points(
        &mut chart,
        id,
        &[p(10.0, 101.0), p(30.0, 140.0)]
    ));
    let corner = (at_x(&chart, 30.0), at_y(&chart, 106.0));
    assert_eq!(
        chart
            .hit_test_drawing(corner.0, corner.1)
            .map(|hit| hit.part),
        Some(DrawingDragPart::Handle(0))
    );
    assert!(chart.drawing_drag_start_at(corner.0, corner.1));
    chart.drawing_drag_to(
        at_x(&chart, 26.0),
        at_y(&chart, 104.0),
        DrawingModifiers::default(),
    );
    chart.drawing_drag_end();
    let points = chart.drawing(id).unwrap().points.clone();
    assert_eq!(points[1].logical, 26.0);
    let ratio = gann(&chart, id).scale_ratio.unwrap();
    assert!((ratio - 3.0 / 16.0).abs() < 1e-6, "{ratio}");
    assert!((points[1].price - 104.0).abs() < 1e-6);
    // Undo restores the ratio with the anchors.
    assert!(chart.undo_drawing());
    assert_eq!(gann(&chart, id).scale_ratio, Some(0.25));
    assert!(chart.redo_drawing());
    let corner = (at_x(&chart, 26.0), at_y(&chart, 104.0));
    assert!(chart.drawing_drag_start_at(corner.0, corner.1));
    chart.drawing_drag_to(
        at_x(&chart, 28.0),
        at_y(&chart, 109.0),
        DrawingModifiers::default(),
    );
    assert!((gann(&chart, id).scale_ratio.unwrap() - 8.0 / 18.0).abs() < 1e-6);
    chart.drawing_drag_to(
        at_x(&chart, 30.0),
        at_y(&chart, 110.0),
        DrawingModifiers {
            magnet: false,
            straighten: true,
        },
    );
    chart.drawing_drag_end();
    assert_eq!(chart.drawing(id).unwrap().points[1].logical, 30.0);
    assert!((gann(&chart, id).scale_ratio.unwrap() - 3.0 / 16.0).abs() < 1e-6);

    // A document square whose second anchor lies far off its corner shows the handle on the
    // painted corner.
    let far = add(
        &mut chart,
        DrawingKind::GannSquareFixed,
        vec![p(10.0, 101.0), p(14.0, 2_000.0)],
        "{}",
    );
    chart.set_selected_drawing(Some(far));
    let x = at_x(&chart, 14.0);
    let corner = (x, pivot.1 - (x - pivot.0));
    assert_eq!(
        chart
            .hit_test_drawing(corner.0, corner.1)
            .map(|hit| (hit.id, hit.part)),
        Some((far, DrawingDragPart::Handle(0)))
    );
}

/// Keyboard steps of the fixed square's corner (its second keyboard handle) cross a whole bar
/// each way: without a ratio by the axis they moved, with one a horizontal step changes the bars
/// and a vertical one the ratio; a step that cannot shrink below one bar records nothing.
#[test]
fn keyboard_steps_resize_a_fixed_gann_square_across_whole_bars() {
    let mut chart = chart();
    let id = add(
        &mut chart,
        DrawingKind::GannSquareFixed,
        vec![p(10.0, 101.0), p(20.0, 2_000.0)],
        "{}",
    );
    chart.set_selected_drawing(Some(id));
    let bars = |chart: &ChartEngine| {
        let points = &chart.drawing(id).unwrap().points;
        points[1].logical - points[0].logical
    };
    let spacing = at_x(&chart, 1.0) - at_x(&chart, 0.0);
    assert!(spacing > 2.0, "a one-pixel step is well under a bar");
    for expected in [11.0, 12.0, 13.0] {
        assert!(chart.nudge_selected_drawing(1.0, 0.0, Some(1)));
        assert_eq!(bars(&chart), expected);
    }
    assert!(chart.nudge_selected_drawing(-1.0, 0.0, Some(1)));
    assert_eq!(bars(&chart), 12.0);
    // A vertical step sizes by the axis it moved: up grows an upward square, down shrinks it.
    assert!(chart.nudge_selected_drawing(0.0, -1.0, Some(1)));
    assert_eq!(bars(&chart), 13.0);
    assert!(chart.nudge_selected_drawing(0.0, 1.0, Some(1)));
    assert_eq!(bars(&chart), 12.0);
    assert!(chart.undo_drawing());
    assert_eq!(bars(&chart), 13.0);

    // With a ratio a vertical step keeps the bars and edits the ratio; a horizontal one adds a
    // bar.
    assert!(chart.drawing_apply_options(id, r#"{"tool_options":{"gann":{"scale_ratio":0.25}}}"#));
    assert!(chart.nudge_selected_drawing(0.0, -1.0, Some(1)));
    assert_eq!(bars(&chart), 13.0);
    assert!(gann(&chart, id).scale_ratio.unwrap() > 0.25);
    assert!(chart.nudge_selected_drawing(1.0, 0.0, Some(1)));
    assert_eq!(bars(&chart), 14.0);

    // A vertical ratio step keeps the second anchor's logical bit-identical, also off the whole
    // bars and at fractional bar spacing (the px round trip keeps six decimals).
    chart.set_bar_spacing(13.37);
    chart.set_right_offset(2.31);
    chart.build_frame();
    assert!(set_points(
        &mut chart,
        id,
        &[p(10.0, 101.0), p(24.123_456_789, 104.0)]
    ));
    for step in [-1.0, 1.0] {
        assert!(chart.nudge_selected_drawing(0.0, step, Some(1)));
        assert_eq!(chart.drawing(id).unwrap().points[1].logical, 24.123_456_789);
    }
    chart.fit_content();
    chart.build_frame();

    // One bar is the minimum: a shrinking step changes nothing and reports it.
    assert!(chart.drawing_apply_options(id, r#"{"tool_options":{"gann":{"scale_ratio":null}}}"#));
    assert!(set_points(
        &mut chart,
        id,
        &[p(10.0, 101.0), p(11.0, 2_000.0)]
    ));
    let before = chart.drawing(id).unwrap().clone();
    assert!(!chart.nudge_selected_drawing(-1.0, 0.0, Some(1)));
    assert_eq!(chart.drawing(id).unwrap(), &before);
}

/// A price-basis rescale scales the fan's and the fixed square's price-per-bar ratios with their
/// anchors (live drawings, history, an in-flight drag's restore point) and rejects a ratio it
/// would push past the value range, atomically.
#[test]
fn a_price_basis_rescale_scales_gann_price_per_bar_ratios_with_the_anchors() {
    let times = hourly(40);
    let mut chart = chart_with(800.0, &times, 1.0);
    let edges = serde_json::json!({"fill_enabled": false, "gann_fans": [], "gann_arcs": [],
        "levels": [level_json(0.0, INK, false, ""), level_json(1.0, INK, false, "")],
        "tool_options": {"gann": {"scale_ratio": 0.25}}});
    let square = add(
        &mut chart,
        DrawingKind::GannSquareFixed,
        vec![p(10.0, 100.0), p(30.0, 140.0)],
        &edges.to_string(),
    );
    let fan = add(
        &mut chart,
        DrawingKind::GannFan,
        vec![p(10.0, 101.0), p(20.0, 104.0)],
        &serde_json::json!({"fill_enabled": false,
            "levels": [level_json(1.0, "#787b86", false, "")],
            "tool_options": {"gann": {"scale_ratio": 0.2}}})
        .to_string(),
    );
    let ratio = |chart: &ChartEngine, id| gann(chart, id).scale_ratio.unwrap();
    let geometry = |chart: &mut ChartEngine| (polylines(chart, INK), polylines(chart, "#787b86"));
    let before = geometry(&mut chart);
    // A committed edit whose undo snapshot must follow the basis too.
    assert!(
        chart.drawing_apply_options(square, r#"{"tool_options":{"gann":{"scale_ratio":0.3}}}"#)
    );
    assert!(chart.undo_drawing());

    // A 2:1 split: the host swaps in halved data and rescales the drawings by 0.5.
    let halved = (0..times.len())
        .map(|index| (100.0 + (index % 7) as f64) * 0.5)
        .collect::<Vec<_>>();
    chart
        .set_series_data(0, &times, &halved, &halved, &halved, &halved)
        .unwrap();
    let all_time = [crate::DrawingPriceSegment {
        from_time: None,
        to_time: None,
        factor: 0.5,
    }];
    assert_eq!(chart.rescale_drawing_prices(&all_time, None).unwrap(), 2);
    chart.fit_content();
    assert_eq!(ratio(&chart, square), 0.125);
    assert_eq!(ratio(&chart, fan), 0.1);
    let after = geometry(&mut chart);
    for (after, before) in [(&after.0, &before.0), (&after.1, &before.1)] {
        assert_eq!(after.len(), before.len());
        for ((a, ..), (b, ..)) in after.iter().zip(before) {
            for (a, b) in a.iter().zip(b) {
                assert!(close(*a, *b, 1e-3), "{a:?} vs {b:?}");
            }
        }
    }
    // Redo replays the edit in the new basis.
    assert!(chart.redo_drawing());
    assert!((ratio(&chart, square) - 0.15).abs() < 1e-12);
    assert!(chart.undo_drawing());

    // An in-flight corner drag keeps its restore point in the new basis.
    chart.set_selected_drawing(Some(square));
    let corner = (at_x(&chart, 30.0), at_y(&chart, 52.5));
    assert_eq!(
        chart
            .hit_test_drawing(corner.0, corner.1)
            .map(|hit| hit.part),
        Some(DrawingDragPart::Handle(0))
    );
    assert!(chart.drawing_drag_start_at(corner.0, corner.1));
    chart.drawing_drag_to(
        corner.0 + 40.0,
        corner.1 - 30.0,
        DrawingModifiers::default(),
    );
    let double = [crate::DrawingPriceSegment {
        from_time: None,
        to_time: None,
        factor: 2.0,
    }];
    chart.rescale_drawing_prices(&double, None).unwrap();
    chart.drawing_drag_cancel();
    assert_eq!(ratio(&chart, square), 0.25);

    // A ratio the factor would push past the value range rejects the whole rescale.
    assert!(chart.drawing_apply_options(fan, r#"{"tool_options":{"gann":{"scale_ratio":1e8}}}"#));
    let snapshot = chart.drawings().to_vec();
    let huge = [crate::DrawingPriceSegment {
        from_time: None,
        to_time: None,
        factor: 1e6,
    }];
    assert!(chart.rescale_drawing_prices(&huge, None).is_err());
    assert_eq!(chart.drawings(), &snapshot[..]);
    // The option accepts only ratios a rescale can keep, so a stored one never vetoes it.
    assert!(!chart.drawing_apply_options(fan, r#"{"tool_options":{"gann":{"scale_ratio":1e14}}}"#));
    assert_eq!(chart.drawings(), &snapshot[..]);
}

/// The schema lists the `tool_options.gann` rows each tool renders, at upstream-neutral
/// defaults, and no numeric fixed-square size (G3).
#[test]
fn schema_lists_the_gann_options_each_tool_renders() {
    let names = |kind| {
        crate::drawing_property_schema(kind)
            .properties
            .into_iter()
            .filter(|property| property.name.starts_with("tool_options.gann."))
            .map(|property| (property.name, property.default))
            .collect::<Vec<_>>()
    };
    let row =
        |name: &str, default: serde_json::Value| (format!("tool_options.gann.{name}"), default);
    let box_rows = names(DrawingKind::GannBox);
    assert_eq!(box_rows[0], row("time_levels", serde_json::json!([])));
    assert_eq!(box_rows[1].0, "tool_options.gann.angles");
    assert_eq!(box_rows[1].1.as_array().unwrap().len(), 9);
    assert_eq!(box_rows[2], row("show_angles", serde_json::json!(false)));
    assert_eq!(box_rows.len(), 3);
    assert_eq!(
        names(DrawingKind::GannSquare),
        [row("show_stats", serde_json::json!(false))]
    );
    assert_eq!(
        names(DrawingKind::GannSquareFixed),
        [
            row("show_stats", serde_json::json!(false)),
            row("scale_ratio", serde_json::Value::Null)
        ]
    );
    assert_eq!(
        names(DrawingKind::GannFan),
        [row("scale_ratio", serde_json::Value::Null)]
    );
    for kind in PITCHFORKS {
        assert!(names(kind).is_empty());
    }
}

/// Documents the fork wrote regain its Gann look on upstream's arms: the box's own time levels
/// with per-axis bands, the squares' stats box, and a ratio square's exact corner.
#[test]
fn fork_documents_regain_the_fork_gann_look() {
    let at = |logical: f64, price: f64| serde_json::json!({"logical": logical, "price": price, "time": logical * HOUR});
    let document = serde_json::json!({
        "schema": "aeris_charts-state",
        "schema_version": 1,
        "panes": [{"id": "pane-1"}],
        "drawings": [
            {"id": 1, "kind": "gann_box", "pane_id": "pane-1",
             "anchors": [at(10.0, 101.0), at(20.0, 105.0)]},
            {"id": 2, "kind": "gann_square", "pane_id": "pane-1",
             "anchors": [at(22.0, 101.0), at(30.0, 105.0)]},
            {"id": 3, "kind": "gann_square_fixed", "pane_id": "pane-1",
             "anchors": [at(3.0, 101.0)],
             "style": {"tool_options": {"gann": {"size_bars": 10.0, "scale_ratio": 0.5}}}},
        ]
    })
    .to_string();
    let mut chart = chart();
    chart.import_state_json(&document).unwrap();
    let only = |chart: &mut ChartEngine, id: DrawingId| {
        for drawing in chart.drawings().to_vec() {
            let visible = drawing.id == id;
            assert!(chart.drawing_apply_options(
                drawing.id,
                &serde_json::json!({"visible": visible}).to_string()
            ));
        }
    };
    only(&mut chart, 1);
    // Six price bands and six time bands, each in its outer level's zone colour.
    assert_eq!(rects(&mut chart, |color| color.a() == 35).len(), 12);
    let box_px = (anchor(&chart, 1, 0), anchor(&chart, 1, 1));
    let labels = texts(&mut chart);
    let top = box_px.0.1.min(box_px.1.1);
    assert!(
        labels
            .iter()
            .any(|(_, _, y)| (f64::from(*y) - (top - 8.0)).abs() < 1e-3)
    );
    only(&mut chart, 2);
    let stats = texts(&mut chart)
        .into_iter()
        .map(|(text, ..)| text)
        .collect::<Vec<_>>();
    assert!(stats.contains(&"8 bars".to_string()), "{stats:?}");
    only(&mut chart, 3);
    assert_eq!(
        chart.drawing(3).unwrap().points[1],
        p(13.0, 106.0),
        "the fork's ratio corner"
    );
    let stats = texts(&mut chart)
        .into_iter()
        .map(|(text, ..)| text)
        .collect::<Vec<_>>();
    for line in ["5.00", "10 bars", "0.5000/bar"] {
        assert!(stats.contains(&line.to_string()), "{line}: {stats:?}");
    }
}

/// The restored options keep the indexed hit test equal to brute force, selected or not, at
/// every device pixel ratio's media geometry.
#[test]
fn restored_options_keep_indexed_hits_equal_to_brute_force() {
    let mut chart = chart();
    let mut ids = Vec::new();
    for copy in 0..3 {
        let shift = f64::from(copy) * 7.0;
        let at = |logical: f64, price: f64| p(logical + shift, price + shift * 0.1);
        ids.push(add(
            &mut chart,
            DrawingKind::GannBox,
            vec![at(2.0, 101.0), at(8.0, 104.0)],
            &serde_json::json!({"fill_enabled": true, "tool_options": {"gann": {
                "show_angles": true,
                "time_levels": [level_json(0.0, INK, true, ""), level_json(1.5, INK, true, "")]}}})
            .to_string(),
        ));
        ids.push(add(
            &mut chart,
            DrawingKind::GannSquareFixed,
            vec![at(4.0, 102.0), at(9.0, 102.5)],
            r#"{"fill_enabled":true,"tool_options":{"gann":{"show_stats":true,"scale_ratio":0.3}}}"#,
        ));
        ids.push(add(
            &mut chart,
            DrawingKind::GannSquare,
            vec![at(1.0, 100.0), at(6.0, 103.0)],
            r#"{"fill_enabled":true,"tool_options":{"gann":{"show_stats":true}}}"#,
        ));
        ids.push(add(
            &mut chart,
            DrawingKind::GannFan,
            vec![at(3.0, 101.0), at(7.0, 101.0)],
            r#"{"tool_options":{"gann":{"scale_ratio":0.25}}}"#,
        ));
        ids.push(add(
            &mut chart,
            DrawingKind::Pitchfan,
            vec![at(1.0, 101.0), at(5.0, 104.0), at(7.0, 102.0)],
            &serde_json::json!({"fill_enabled": true, "levels": [
                level_json(0.0, INK, false, ""), level_json(0.5, INK, true, ""),
                level_json(1.0, INK, true, "")]})
            .to_string(),
        ));
    }
    for selected in [None, Some(ids[1]), Some(ids[4])] {
        chart.set_selected_drawing(selected);
        chart.build_frame();
        let mut hits = 0;
        for gy in 0..48 {
            for gx in 0..78 {
                let (x, y) = (f64::from(gx) * 10.0 + 3.0, f64::from(gy) * 10.0 + 4.0);
                hits += usize::from(part_at(&chart, (x, y)).is_some());
            }
        }
        assert!(hits > 100, "{hits}");
    }
}

/// The restored geometry scales with the device pixel ratio like upstream's arms: the ratio
/// corner, the stats box (which media-px hit testing finds where the frame paints it), the time
/// levels, and the base-midpoint handle.
#[test]
fn restored_geometry_scales_with_the_device_pixel_ratio() {
    // 801 wide at 1.5 makes the horizontal and vertical bitmap ratios differ.
    for (width, dpr) in [(800.0, 1.0), (800.0, 2.0), (801.0, 1.5)] {
        let mut chart = chart_with(width, &hourly(40), dpr);
        let hpr = (chart.pane_w * dpr).round() / chart.pane_w;
        let vpr = (chart.pane_h * dpr).round() / chart.pane_h;
        let square = add(
            &mut chart,
            DrawingKind::GannSquareFixed,
            vec![p(10.0, 101.0), p(30.0, 140.0)],
            &serde_json::json!({"gann_fans": [], "gann_arcs": [],
                "levels": [level_json(1.0, INK, false, "")],
                "tool_options": {"gann": {"scale_ratio": 0.25, "show_stats": true}}})
            .to_string(),
        );
        let far = (at_x(&chart, 30.0) * hpr, at_y(&chart, 106.0) * vpr);
        let edges = polylines(&mut chart, INK);
        assert!(
            edges
                .iter()
                .any(|(line, ..)| close(line[0], (far.0, line[0].1), 1e-3)
                    && (line[0].1.min(line[1].1) - far.1).abs() < 1e-3),
            "dpr {dpr}: {edges:?}"
        );
        let stroke = chart.drawing(square).unwrap().stroke_color();
        let wash = Color::rgba(
            stroke.r(),
            stroke.g(),
            stroke.b(),
            crate::drawings::parts::STATS_ALPHA,
        );
        let rect = rects(&mut chart, |color| color == wash)[0];
        assert!(f64::from(rect.x) > far.0, "dpr {dpr}: right of the corner");
        let center = (
            (f64::from(rect.x) + f64::from(rect.w) / 2.0) / hpr,
            (f64::from(rect.y) + f64::from(rect.h) / 2.0) / vpr,
        );
        assert_eq!(
            part_at(&chart, center),
            Some((square, DrawingDragPart::Body)),
            "dpr {dpr}"
        );
        chart.remove_drawing(square);

        let fork = add(
            &mut chart,
            DrawingKind::AndrewsPitchfork,
            anchors(DrawingKind::AndrewsPitchfork),
            "{}",
        );
        chart.set_selected_drawing(Some(fork));
        let base = mid(anchor(&chart, fork, 1), anchor(&chart, fork, 2));
        let frame = chart.build_frame();
        assert!(
            frame.panes[0].main.iter().any(|prim| matches!(
                prim,
                Prim::Circle { cx, cy, .. }
                    if close((f64::from(*cx), f64::from(*cy)), (base.0 * hpr, base.1 * vpr), 1e-2)
            )),
            "dpr {dpr}: the base handle"
        );
        chart.remove_drawing(fork);

        let gann_box = add(
            &mut chart,
            DrawingKind::GannBox,
            vec![p(10.0, 101.0), p(20.0, 105.0)],
            &serde_json::json!({"levels": [], "tool_options": {"gann": {
                "time_levels": [level_json(0.5, "#00aa00", false, "")]}}})
            .to_string(),
        );
        let x = (at_x(&chart, 10.0) + at_x(&chart, 20.0)) / 2.0 * hpr;
        let lines = polylines(&mut chart, "#00aa00");
        assert_eq!(lines.len(), 1, "dpr {dpr}");
        assert!((lines[0].0[0].0 - x).abs() < 1e-3 && (lines[0].1 - vpr as f32).abs() < 1e-5);
        chart.remove_drawing(gann_box);
    }
}
