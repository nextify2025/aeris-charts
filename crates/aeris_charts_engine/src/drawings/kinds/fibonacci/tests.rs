//! Fibonacci-family engine tests: catalog defaults, armed placement and partial previews, level
//! geometry per tool (price, time, and screen-space levels), bands and labels, hit testing
//! (indexed and brute force), culling beyond the anchors, drags, keyboard nudges, magnet, time
//! identity, schema and kind options, level-list and option patches with history, templates,
//! persistence with default omission, clipboard, sync, and device-pixel scaling.

use aeris_charts_render::color::Color;
use aeris_charts_render::draw_list::{LineStyle, Prim};

use super::super::super::DrawingTextLayout;
use super::{FibonacciLabelHAlign, FibonacciToolOptions};
use crate::{
    ChartEngine, DrawingId, DrawingKind, DrawingMagnetMode, DrawingModifiers, DrawingPoint,
};

const FIB_KINDS: [DrawingKind; 10] = [
    DrawingKind::FibonacciRetracement,
    DrawingKind::FibonacciExtension,
    DrawingKind::FibonacciChannel,
    DrawingKind::FibonacciTimeZones,
    DrawingKind::FibonacciTrendTime,
    DrawingKind::FibonacciSpeedFan,
    DrawingKind::FibonacciSpeedArcs,
    DrawingKind::FibonacciCircles,
    DrawingKind::FibonacciSpiral,
    DrawingKind::FibonacciWedge,
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

/// Anchors inside the default viewport: a rising first leg and, for three-anchor tools, a
/// pullback.
fn anchors_for(kind: DrawingKind) -> Vec<DrawingPoint> {
    match kind.anchor_count() {
        3 => vec![p(10.0, 101.0), p(16.0, 105.0), p(20.0, 103.0)],
        _ => vec![p(10.0, 101.0), p(18.0, 105.0)],
    }
}

fn y_of(chart: &ChartEngine, price: f64) -> f64 {
    chart.series_price_to_coordinate(0, price).unwrap()
}

/// Media x of a (possibly fractional) logical position.
fn x_of(chart: &ChartEngine, logical: f64) -> f64 {
    chart.time_scale.logical_to_coordinate(logical)
}

fn css(color: &str) -> Color {
    Color::parse_css(color).unwrap()
}

fn close(a: (f64, f64), b: (f64, f64), tolerance: f64) -> bool {
    (a.0 - b.0).abs() <= tolerance && (a.1 - b.1).abs() <= tolerance
}

/// Custom level list JSON: `(value, color)` pairs, visible, solid, labeled, filled.
fn levels_json(levels: &[(f64, &str)]) -> String {
    let items = levels
        .iter()
        .map(|(value, color)| {
            format!(
                r#"{{"value":{value},"color":"{color}","visible":true,"style":"solid","fill_between":true,"label_visible":true}}"#
            )
        })
        .collect::<Vec<_>>();
    format!("[{}]", items.join(","))
}

/// The first pane's primitives and point pool of one frame.
struct Scene {
    prims: Vec<Prim>,
    points: Vec<[f32; 2]>,
}

impl Scene {
    fn of(chart: &mut ChartEngine) -> Self {
        let frame = chart.build_frame();
        Self {
            prims: frame.panes[0].main.clone(),
            points: frame.panes[0].points.clone(),
        }
    }

    fn run(&self, first: u32, count: u32) -> Vec<(f64, f64)> {
        self.points[first as usize..(first + count) as usize]
            .iter()
            .map(|point| (f64::from(point[0]), f64::from(point[1])))
            .collect()
    }

    /// `(points, style)` of every anti-aliased polyline in `color`.
    fn polylines(&self, color: Color) -> Vec<(Vec<(f64, f64)>, LineStyle)> {
        self.prims
            .iter()
            .filter_map(|prim| match prim {
                Prim::Polyline {
                    first_point,
                    point_count,
                    style,
                    color: line,
                    ..
                } if *line == color => Some((self.run(*first_point, *point_count), *style)),
                _ => None,
            })
            .collect()
    }

    /// The runs of every polyline in `color`; a dashed stroke lowers to one solid run per dash.
    fn runs(&self, color: Color) -> Vec<Vec<(f64, f64)>> {
        self.polylines(color)
            .into_iter()
            .map(|(points, style)| {
                assert_eq!(style, LineStyle::Solid, "dashes lower to solid runs");
                points
            })
            .collect()
    }
}

/// Whether `runs` stroke `path`: every point on it, the first run starting at its first point,
/// and the last ending within one dash period (12 stroke widths) of its end. A dashed stroke
/// lowers to one solid run per dash, so this holds for solid and dashed strokes alike.
fn strokes_along(runs: &[Vec<(f64, f64)>], path: &[(f64, f64)], width: f64) -> bool {
    let tail = path[path.len() - 1];
    runs.iter()
        .flatten()
        .all(|&point| aeris_charts_render::shape::distance_to_polyline(point, path) < 0.01)
        && runs.first().is_some_and(|run| close(run[0], path[0], 0.01))
        && runs
            .last()
            .and_then(|run| run.last())
            .is_some_and(|end| (end.0 - tail.0).hypot(end.1 - tail.1) <= 12.0 * width + 0.01)
}

/// The fork's pre-merge Fibonacci defaults, which documents it wrote omitted, come back through
/// `apply_legacy_fork_defaults` (upstream renders the Fibonacci tools now).
#[test]
fn catalog_defaults_follow_each_tool() {
    let legacy = |kind| {
        let mut drawing = crate::Drawing::new(1, kind, 0, Vec::new());
        super::super::apply_legacy_fork_defaults(&mut drawing);
        drawing
    };
    for kind in FIB_KINDS {
        let drawing = legacy(kind);
        let spec = kind.spec();
        assert!(spec.family.is_none(), "{kind:?} is upstream-rendered");
        assert_eq!(spec.text_layout, DrawingTextLayout::Box);
        assert!(!spec.axis_price_label);
        assert_eq!(drawing.width, 1.0);
        assert!(drawing.tool_options.is_empty());
        assert!(!drawing.extend_left && !drawing.extend_right);
        let spiral = kind == DrawingKind::FibonacciSpiral;
        assert_eq!(drawing.levels.is_empty(), spiral, "{kind:?} levels");
        assert_eq!(
            drawing.fill_enabled,
            !matches!(
                kind,
                DrawingKind::FibonacciTimeZones | DrawingKind::FibonacciSpiral
            )
        );
        assert_eq!(drawing.color == "#787b86", !spiral);
        assert_eq!(
            drawing.style,
            if matches!(
                kind,
                DrawingKind::FibonacciSpiral | DrawingKind::FibonacciWedge
            ) {
                LineStyle::Solid
            } else {
                LineStyle::Dashed
            }
        );
        assert!(drawing
            .levels
            .iter()
            .all(|level| level.visible && level.fill_between && level.label_visible));
    }
    // TradingView's retracement levels and palette; time zones follow the Fibonacci sequence.
    let retracement = legacy(DrawingKind::FibonacciRetracement);
    let table = retracement
        .levels
        .iter()
        .map(|level| (level.value, level.color.as_str()))
        .collect::<Vec<_>>();
    assert_eq!(
        table,
        [
            (0.0, "#787b86"),
            (0.236, "#f23645"),
            (0.382, "#ff9800"),
            (0.5, "#4caf50"),
            (0.618, "#089981"),
            (0.786, "#00bcd4"),
            (1.0, "#787b86"),
            (1.618, "#2962ff"),
            (2.618, "#f23645"),
            (3.618, "#9c27b0"),
            (4.236, "#e91e63"),
        ]
    );
    let zones = legacy(DrawingKind::FibonacciTimeZones);
    assert_eq!(
        zones
            .levels
            .iter()
            .map(|level| level.value)
            .collect::<Vec<_>>(),
        [0.0, 1.0, 2.0, 3.0, 5.0, 8.0, 13.0, 21.0, 34.0, 55.0, 89.0]
    );
    let fan = legacy(DrawingKind::FibonacciSpeedFan);
    assert_eq!(
        fan.levels
            .iter()
            .map(|level| level.value)
            .collect::<Vec<_>>(),
        [0.0, 0.25, 0.382, 0.5, 0.618, 0.75, 1.0]
    );
}

#[test]
fn armed_tools_place_every_fibonacci_kind() {
    let mut chart = chart();
    for kind in FIB_KINDS {
        assert!(chart.set_drawing_tool(Some(kind), Some(r##"{"color":"#123456"}"##), None));
        let clicks = [(200.0, 260.0), (420.0, 150.0), (520.0, 220.0)];
        let mut created = None;
        for &(x, y) in &clicks[..kind.anchor_count()] {
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
    let frame = chart.build_frame();
    assert!(frame.panes[0]
        .points
        .iter()
        .all(|point| point[0].is_finite() && point[1].is_finite()));
}

#[test]
fn three_anchor_previews_show_their_first_leg_before_the_second_click() {
    for kind in [
        DrawingKind::FibonacciExtension,
        DrawingKind::FibonacciChannel,
        DrawingKind::FibonacciTrendTime,
        DrawingKind::FibonacciWedge,
    ] {
        let mut chart = chart();
        assert!(chart.set_drawing_tool(Some(kind), Some(r##"{"color":"#123456"}"##), None));
        let (a, b) = ((200.0, 260.0), (420.0, 150.0));
        chart.drawing_tool_activate(a.0, a.1, DrawingModifiers::default());
        chart.drawing_tool_pointer_move(b.0, b.1, DrawingModifiers::default(), false);
        let lines = Scene::of(&mut chart).runs(css(INK));
        assert!(
            strokes_along(&lines, &[a, b], 1.0),
            "{kind:?} previews its first leg: {lines:?}"
        );
        // The third anchor's preview resolves the whole tool.
        chart.drawing_tool_activate(b.0, b.1, DrawingModifiers::default());
        chart.drawing_tool_pointer_move(520.0, 220.0, DrawingModifiers::default(), false);
        assert!(Scene::of(&mut chart).prims.len() > lines.len());
    }
}

#[test]
fn indexed_hit_testing_matches_brute_force() {
    let mut chart = chart();
    for copy in 0..3 {
        let shift = copy as f64 * 1.3;
        for kind in FIB_KINDS {
            let points = anchors_for(kind)
                .into_iter()
                .map(|point| p(point.logical + shift, point.price + shift * 0.2))
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
fn chart_magnet_snaps_fibonacci_placement_to_bar_values() {
    let mut chart = chart();
    chart.set_drawing_magnet_mode(DrawingMagnetMode::Strong);
    assert!(chart.set_drawing_tool(Some(DrawingKind::FibonacciRetracement), None, None));
    let at = |chart: &ChartEngine, logical: f64, price: f64| {
        (x_of(chart, logical) + 3.0, y_of(chart, price))
    };
    let (x, y) = at(&chart, 12.0, 102.4);
    chart.drawing_tool_activate(x, y, DrawingModifiers::default());
    let (x, y) = at(&chart, 15.0, 104.3);
    let id = chart
        .drawing_tool_activate(x, y, DrawingModifiers::default())
        .created
        .unwrap();
    let points = &chart.drawing(id).unwrap().points;
    assert_eq!(points[0], p(12.0, 100.0 + (12 % 7) as f64));
    assert_eq!(points[1], p(15.0, 100.0 + (15 % 7) as f64));
}

#[test]
fn persistence_round_trips_fibonacci_tools_and_omits_kind_defaults() {
    let mut chart = chart();
    for kind in FIB_KINDS {
        add(&mut chart, kind, anchors_for(kind), "{}");
    }
    let exported: serde_json::Value =
        serde_json::from_str(&chart.export_state_json().unwrap()).unwrap();
    for drawing in exported["drawings"].as_array().unwrap() {
        let style = &drawing["style"];
        for field in ["levels", "fill_enabled", "extend_left", "tool_options"] {
            assert!(
                style.get(field).is_none(),
                "{} writes its default {field}",
                drawing["kind"]
            );
        }
    }
    let customized = [
        (
            DrawingKind::FibonacciRetracement,
            r#"{"extend_right":true,"tool_options":{"fibonacci":{"reverse":true,"label_h_align":"right"}}}"#.to_string(),
        ),
        (DrawingKind::FibonacciChannel, r#"{"levels":[]}"#.to_string()),
        (DrawingKind::FibonacciTimeZones, r#"{"fill_enabled":true}"#.to_string()),
        (
            DrawingKind::FibonacciSpeedFan,
            format!(r#"{{"levels":{}}}"#, levels_json(&[(0.5, "#abcdef")])),
        ),
        (
            DrawingKind::FibonacciSpiral,
            r#"{"tool_options":{"fibonacci":{"reverse":true}},"style":"dotted"}"#.to_string(),
        ),
    ];
    for (kind, options) in &customized {
        add(&mut chart, *kind, anchors_for(*kind), options);
    }
    let document = chart.export_state_json().unwrap();
    let mut restored = ChartEngine::new(800.0, 500.0, 1.0);
    restored.import_state_json(&document).unwrap();
    assert_eq!(restored.export_state_json().unwrap(), document);
    assert_eq!(
        restored.drawings().len(),
        FIB_KINDS.len() + customized.len()
    );
    for (restored, original) in restored.drawings().iter().zip(chart.drawings()) {
        assert_eq!(restored.kind, original.kind);
        assert_eq!(restored.levels, original.levels);
        assert_eq!(restored.fill_enabled, original.fill_enabled);
        assert_eq!(restored.extend_right, original.extend_right);
        assert_eq!(restored.tool_options, original.tool_options);
        assert_eq!(restored.style, original.style);
        assert_eq!(restored.color, original.color);
    }
    let cleared = &restored.drawings()[FIB_KINDS.len() + 1];
    assert_eq!(cleared.kind, DrawingKind::FibonacciChannel);
    assert!(
        cleared.levels.is_empty(),
        "a user-cleared level list stays cleared"
    );
}

#[test]
fn clipboard_and_sync_payloads_carry_levels_and_tool_options() {
    let mut source = chart();
    let id = add(
        &mut source,
        DrawingKind::FibonacciExtension,
        anchors_for(DrawingKind::FibonacciExtension),
        &format!(
            r#"{{"levels":{},"tool_options":{{"fibonacci":{{"log_scale":true}}}},"extend_left":true}}"#,
            levels_json(&[(1.0, "#111111"), (1.618, "#222222")])
        ),
    );
    let copied = source.copy_drawings_json(&[id]).unwrap();
    let mut target = chart();
    let pasted = target.paste_drawings_json(&copied, 0, 0.0, 0.0).unwrap();
    let drawing = target.drawing(pasted[0]).unwrap();
    let original = source.drawing(id).unwrap();
    assert_eq!(drawing.kind, DrawingKind::FibonacciExtension);
    assert!(drawing.extend_left);
    assert_eq!(drawing.levels, original.levels);
    assert_eq!(drawing.tool_options, original.tool_options);

    let payload = source.drawing_sync_payload_json("cell-a").unwrap();
    let mut mirror = chart();
    assert!(mirror.apply_drawing_sync_payload_json(&payload));
    assert_eq!(mirror.drawings()[0].levels, original.levels);
    assert_eq!(mirror.drawings()[0].tool_options, original.tool_options);
}

#[test]
fn fibonacci_tools_tolerate_charts_without_data_and_degenerate_anchors() {
    let mut empty = ChartEngine::new(800.0, 500.0, 1.0);
    for kind in FIB_KINDS {
        assert!(empty
            .add_drawing(kind, 0, anchors_for(kind), None)
            .is_some());
    }
    empty.build_frame();
    assert_eq!(empty.hit_test_drawing(100.0, 100.0), None);

    let mut chart = chart();
    for kind in FIB_KINDS {
        let points = vec![p(12.0, 102.0); kind.anchor_count()];
        assert!(chart.add_drawing(kind, 0, points, None).is_some());
        // Non-positive prices fall back to linear levels under log scale.
        let points =
            vec![p(12.0, -5.0), p(14.0, 0.0), p(16.0, 3.0)][..kind.anchor_count()].to_vec();
        assert!(chart
            .add_drawing(
                kind,
                0,
                points,
                Some(r#"{"tool_options":{"fibonacci":{"log_scale":true}}}"#)
            )
            .is_some());
    }
    let frame = chart.build_frame();
    assert!(frame.panes[0]
        .points
        .iter()
        .all(|point| point[0].is_finite() && point[1].is_finite()));
    chart.hit_test_drawing(400.0, 200.0);
}

#[test]
fn family_hooks_added_here_default_to_neutral() {
    // The retired family's `bounds` hook is gone with its renderer; its option block keeps
    // deserializing to the neutral defaults from an empty object.
    let options: FibonacciToolOptions = serde_json::from_str("{}").unwrap();
    assert_eq!(options, FibonacciToolOptions::default());
    assert_eq!(options.label_h_align, None::<FibonacciLabelHAlign>);
}
