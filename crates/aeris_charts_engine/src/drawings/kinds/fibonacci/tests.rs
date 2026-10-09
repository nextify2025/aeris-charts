//! Fibonacci-family engine tests: catalog defaults, armed placement and partial previews, level
//! geometry per tool (price, time, and screen-space levels), bands and labels, hit testing
//! (indexed and brute force), culling beyond the anchors, drags, keyboard nudges, magnet, time
//! identity, schema and kind options, level-list and option patches with history, templates,
//! persistence with default omission, clipboard, sync, and device-pixel scaling.

use aeris_charts_render::color::Color;
use aeris_charts_render::draw_list::{LineStyle, Prim, TextAlign};

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
        assert!(
            drawing
                .levels
                .iter()
                .all(|level| level.visible && level.fill_between && level.label_visible)
        );
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
    assert!(
        frame.panes[0]
            .points
            .iter()
            .all(|point| point[0].is_finite() && point[1].is_finite())
    );
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
    // The fork presentation: trend lines, the fan grid (a level past the anchors), labels beside
    // the lines' ends, full circles, precise rings, filled bands of a selected drawing, and the
    // golden spiral.
    let banded = levels_json(&[
        (0.0, "#111111"),
        (0.5, "#222222"),
        (1.0, "#333333"),
        (1.618, "#444444"),
    ]);
    let fork = format!(
        r#"{{"levels":{banded},"level_label_align":"left","tool_options":{{"fibonacci":{{"trend_line":true,"grid":true,"label_v_align":"middle","full_circles":true}}}}}}"#
    );
    let mut selected = None;
    for kind in FIB_KINDS {
        let points = anchors_for(kind)
            .into_iter()
            .map(|point| p(point.logical + 9.0, point.price - 2.0))
            .collect();
        let id = add(&mut chart, kind, points, &fork);
        selected.get_or_insert(id);
    }
    add(
        &mut chart,
        DrawingKind::FibonacciSpiral,
        vec![p(30.0, 103.0), p(31.5, 103.5)],
        r#"{"levels":[]}"#,
    );
    chart.set_selected_drawing(selected);
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
        assert!(
            empty
                .add_drawing(kind, 0, anchors_for(kind), None)
                .is_some()
        );
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
        assert!(
            chart
                .add_drawing(
                    kind,
                    0,
                    points,
                    Some(r#"{"tool_options":{"fibonacci":{"log_scale":true}}}"#)
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

// --- The fork presentation on upstream's arms (R5) ---------------------------------------------

/// Anchor `index` of drawing `id` in media px.
fn anchor(chart: &ChartEngine, id: DrawingId, index: usize) -> (f64, f64) {
    chart.drawing_point_to_coordinate(id, index).unwrap()
}

/// The logical/price point at media px `(x, y)` on the first pane.
fn at(chart: &ChartEngine, x: f64, y: f64) -> DrawingPoint {
    chart
        .drawing_from_px_for(0, crate::DrawingPriceScale::Right, x, y)
        .unwrap()
}

fn pane_box(chart: &ChartEngine) -> (f64, f64) {
    (
        chart.panes[0].top,
        chart.panes[0].top + chart.panes[0].height,
    )
}

impl Scene {
    /// `(y, x0, x1, style)` of every horizontal line in `color`.
    fn hlines(&self, color: Color) -> Vec<(i32, i32, i32, LineStyle)> {
        self.prims
            .iter()
            .filter_map(|prim| match *prim {
                Prim::HLine {
                    y,
                    x0,
                    x1,
                    style,
                    color: line,
                    ..
                } if line == color => Some((y, x0, x1, style)),
                _ => None,
            })
            .collect()
    }

    /// `(x, y0, y1)` of every vertical line in `color`.
    fn vlines(&self, color: Color) -> Vec<(i32, i32, i32)> {
        self.prims
            .iter()
            .filter_map(|prim| match *prim {
                Prim::VLine {
                    x,
                    y0,
                    y1,
                    color: line,
                    ..
                } if line == color => Some((x, y0, y1)),
                _ => None,
            })
            .collect()
    }

    /// `(text, x, y, align)` of every text run.
    fn texts(&self) -> Vec<(String, f64, f64, TextAlign)> {
        self.prims
            .iter()
            .filter_map(|prim| match prim {
                Prim::Text {
                    x, y, text, align, ..
                } => Some((text.clone(), f64::from(*x), f64::from(*y), *align)),
                _ => None,
            })
            .collect()
    }

    /// The text run that starts with `prefix`.
    fn text(&self, prefix: &str) -> Option<(String, f64, f64, TextAlign)> {
        self.texts()
            .into_iter()
            .find(|(text, ..)| text.starts_with(prefix))
    }
}

/// The center of a painted level label's box (its run measured like the hit test's).
fn label_center(
    chart: &ChartEngine,
    (text, x, y, align): &(String, f64, f64, TextAlign),
) -> (f64, f64) {
    let layout = &chart.options.get().layout;
    let width = chart.measure_text_run(text, layout.font_size, &layout.font_family, 400, false);
    let left = match align {
        TextAlign::Left => *x,
        TextAlign::Center => x - width / 2.0,
        TextAlign::Right => x - width,
    };
    (left + width / 2.0, *y)
}

fn body(chart: &ChartEngine, x: f64, y: f64) -> Option<(DrawingId, crate::DrawingDragPart)> {
    chart.hit_test_drawing(x, y).map(|hit| (hit.id, hit.part))
}

/// A document the fork wrote (its anchors carry the time it stored), with `drawings` as
/// `(kind, anchors (logical, price), style)`.
/// One stored fork drawing: its kind name, anchors `(logical, price)`, and style (`null` for
/// none).
type ForkDrawing<'a> = (&'a str, &'a [(f64, f64)], serde_json::Value);

fn fork_document(drawings: &[ForkDrawing<'_>]) -> String {
    let drawings = drawings
        .iter()
        .enumerate()
        .map(|(index, (kind, anchors, style))| {
            let anchors = anchors
                .iter()
                .map(|&(logical, price)| {
                    serde_json::json!({"logical": logical, "price": price, "time": logical * HOUR})
                })
                .collect::<Vec<_>>();
            let mut drawing = serde_json::json!({
                "id": index + 1, "kind": kind, "pane_id": "pane-1", "anchors": anchors,
            });
            if !style.is_null() {
                drawing["style"] = style.clone();
            }
            drawing
        })
        .collect::<Vec<_>>();
    serde_json::json!({
        "schema": "aeris_charts-state",
        "schema_version": 1,
        "panes": [{"id": "pane-1"}],
        "drawings": drawings,
    })
    .to_string()
}

/// Every level of every new Fibonacci tool is uncolored and paints in the drawing color; the
/// fork's TradingView palette and bands reach only the documents it wrote, through their levels
/// (`legacy_defaults`), which upstream's arms paint in each level's own color.
#[test]
fn new_drawings_paint_uncolored_levels_and_fork_documents_their_palette() {
    let mut chart = chart();
    for kind in FIB_KINDS {
        let id = add(
            &mut chart,
            kind,
            anchors_for(kind),
            r##"{"color":"#123456"}"##,
        );
        let drawing = chart.drawing(id).unwrap();
        assert!(
            drawing
                .levels
                .iter()
                .all(|level| level.color.is_empty() && !level.fill_between),
            "{kind:?}"
        );
        assert_eq!(drawing.tool_options.fibonacci, None, "{kind:?}");
        chart.remove_drawing(id);
    }
    let id = add(
        &mut chart,
        DrawingKind::FibonacciRetracement,
        vec![p(10.0, 101.0), p(18.0, 105.0)],
        r##"{"color":"#123456"}"##,
    );
    let scene = Scene::of(&mut chart);
    assert_eq!(
        scene.hlines(css(INK)).len(),
        7,
        "every level in the drawing color"
    );
    chart.remove_drawing(id);

    let document = fork_document(&[(
        "fib_retracement",
        &[(10.0, 101.0), (18.0, 105.0)],
        serde_json::Value::Null,
    )]);
    let mut chart = super::tests::chart();
    chart.import_state_json(&document).unwrap();
    let scene = Scene::of(&mut chart);
    // The fork's level 0 sat on the second anchor.
    for (ratio, color) in [(0.236, "#f23645"), (0.382, "#ff9800"), (0.618, "#089981")] {
        let y = y_of(&chart, 105.0 - 4.0 * ratio);
        assert!(
            scene
                .hlines(css(color))
                .iter()
                .any(|line| (f64::from(line.0) - y).abs() <= 1.0),
            "{ratio} in {color}"
        );
        let base = css(color);
        let band = Color::rgba(base.r(), base.g(), base.b(), 35);
        assert!(
            scene
                .prims
                .iter()
                .any(|prim| matches!(prim, Prim::Rect { color, .. } if *color == band)),
            "the band below {ratio} in its level color"
        );
    }
}

/// The trend line: off for new drawings; on, it strokes the anchors in the drawing's own stroke
/// (dashes as solid runs), both legs on the extension and trend-based time, the level-1 diameter
/// on the circles, and a 1 CSS px dashed line on the spiral.
#[test]
fn trend_line_strokes_the_anchors_when_enabled() {
    let levels = levels_json(&[(0.5, "#abcdef"), (1.0, "#abcdef")]);
    for kind in FIB_KINDS {
        let mut chart = chart();
        let style = |trend_line: bool| {
            format!(
                r##"{{"color":"{INK}","style":"dashed","levels":{levels},"tool_options":{{"fibonacci":{{"trend_line":{trend_line}}}}}}}"##
            )
        };
        // The circles' diameter runs past the center by the radius: keep it on the pane.
        let anchors = if kind == DrawingKind::FibonacciCircles {
            vec![p(14.0, 101.5), p(18.0, 102.5)]
        } else {
            anchors_for(kind)
        };
        let id = add(&mut chart, kind, anchors, &style(false));
        let without = Scene::of(&mut chart).polylines(css(INK));
        let edges = usize::from(kind == DrawingKind::FibonacciWedge) * 2;
        let off = Scene::of(&mut chart).runs(css(INK)).len();
        assert!(chart.drawing_apply_options(id, &style(true)));
        let runs = Scene::of(&mut chart).runs(css(INK));
        let px = (0..kind.anchor_count())
            .map(|index| anchor(&chart, id, index))
            .collect::<Vec<_>>();
        let path = match kind {
            DrawingKind::FibonacciRetracement
            | DrawingKind::FibonacciTimeZones
            | DrawingKind::FibonacciSpeedArcs
            | DrawingKind::FibonacciSpiral => vec![px[0], px[1]],
            DrawingKind::FibonacciExtension | DrawingKind::FibonacciTrendTime => {
                vec![px[0], px[1], px[2]]
            }
            DrawingKind::FibonacciCircles => {
                vec![px[0], (2.0 * px[1].0 - px[0].0, 2.0 * px[1].1 - px[0].1)]
            }
            _ => {
                // The channel, fan and wedge draw none.
                assert_eq!(runs.len(), off, "{kind:?}");
                assert!(without.len() >= edges);
                continue;
            }
        };
        assert!(
            without.is_empty(),
            "{kind:?} draws none by default: {without:?}"
        );
        assert!(
            runs.len() > 1 && strokes_along(&runs, &path, 1.0),
            "{kind:?} dashes along {path:?}: {runs:?}"
        );
    }
    // The spiral's line is a 1 CSS px decoration whatever the drawing's width.
    let mut chart = chart_with(&hourly(40), 2.0);
    let kind = DrawingKind::FibonacciSpiral;
    add(
        &mut chart,
        kind,
        anchors_for(kind),
        &format!(
            r##"{{"color":"{INK}","width":3,"levels":{levels},"tool_options":{{"fibonacci":{{"trend_line":true}}}}}}"##
        ),
    );
    let frame = chart.build_frame();
    let widths = frame.panes[0]
        .main
        .iter()
        .filter_map(|prim| match prim {
            Prim::Polyline { width, color, .. } if *color == css(INK) => Some(*width),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(
        !widths.is_empty() && widths.iter().all(|&width| width == 2.0),
        "{widths:?}"
    );
}

/// The trend line is a body target away from every level: at t = 0.1 along the retracement's
/// anchors (between levels 0 and 0.236; the midpoint lies on level 0.5), and between explicit
/// levels 0 and 1 on the other tools.
#[test]
fn trend_line_is_a_body_target() {
    let on = r#"{"tool_options":{"fibonacci":{"trend_line":true}}}"#;
    let off = r#"{"tool_options":{"fibonacci":{"trend_line":false}}}"#;
    let along =
        |a: (f64, f64), b: (f64, f64), t: f64| (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
    let mut chart = chart();
    let kind = DrawingKind::FibonacciRetracement;
    let id = add(&mut chart, kind, anchors_for(kind), on);
    let probe = along(anchor(&chart, id, 0), anchor(&chart, id, 1), 0.1);
    assert_eq!(
        body(&chart, probe.0, probe.1),
        Some((id, crate::DrawingDragPart::Body))
    );
    assert!(chart.drawing_apply_options(id, off));
    assert_eq!(body(&chart, probe.0, probe.1), None);
    chart.remove_drawing(id);

    let levels = levels_json(&[(0.0, "#abcdef"), (1.0, "#abcdef")]);
    for kind in [
        DrawingKind::FibonacciExtension,
        DrawingKind::FibonacciTimeZones,
        DrawingKind::FibonacciTrendTime,
        DrawingKind::FibonacciSpeedArcs,
        DrawingKind::FibonacciCircles,
        DrawingKind::FibonacciSpiral,
    ] {
        let style = |options: &str| format!(r#"{{"levels":{levels},{}"#, &options[1..]);
        let id = add(&mut chart, kind, anchors_for(kind), &style(on));
        let (a, b) = (anchor(&chart, id, 0), anchor(&chart, id, 1));
        // The circles' line runs through the center (the second anchor) on.
        let probe = if kind == DrawingKind::FibonacciCircles {
            along(a, b, 1.3)
        } else {
            along(a, b, 0.5)
        };
        assert_eq!(
            body(&chart, probe.0, probe.1),
            Some((id, crate::DrawingDragPart::Body)),
            "{kind:?}"
        );
        assert!(chart.drawing_apply_options(id, &style(off)));
        assert_eq!(body(&chart, probe.0, probe.1), None, "{kind:?}");
        chart.remove_drawing(id);
    }
}

/// The fan's grid: each visible level's horizontal and vertical line at its ratio of the anchors'
/// box in the drawing's stroke, mirrored with `level_reverse`, a body target; none by default.
#[test]
fn fan_grid_draws_the_level_box_when_enabled() {
    let mut chart = chart();
    let kind = DrawingKind::FibonacciSpeedFan;
    let levels = levels_json(&[
        (0.0, "#abcdef"),
        (0.25, "#abcdef"),
        (0.382, "#abcdef"),
        (1.0, "#abcdef"),
    ]);
    let id = add(
        &mut chart,
        kind,
        vec![p(10.0, 101.0), p(18.0, 105.0)],
        &format!(r##"{{"color":"{INK}","levels":{levels}}}"##),
    );
    let scene = Scene::of(&mut chart);
    assert!(scene.hlines(css(INK)).is_empty() && scene.vlines(css(INK)).is_empty());
    let (a, b) = (anchor(&chart, id, 0), anchor(&chart, id, 1));
    let probe = (a.0 + (b.0 - a.0) * 0.25, a.1 + (b.1 - a.1) * 0.75);
    assert_eq!(body(&chart, probe.0, probe.1), None);
    for reverse in [false, true] {
        assert!(chart.drawing_apply_options(
            id,
            &format!(
                r#"{{"level_reverse":{reverse},"tool_options":{{"fibonacci":{{"grid":true}}}}}}"#
            )
        ));
        let scene = Scene::of(&mut chart);
        let (mut ys, mut xs) = (
            scene
                .hlines(css(INK))
                .iter()
                .map(|line| line.0)
                .collect::<Vec<_>>(),
            scene
                .vlines(css(INK))
                .iter()
                .map(|line| line.0)
                .collect::<Vec<_>>(),
        );
        ys.sort_unstable();
        xs.sort_unstable();
        let ratio = |value: f64| if reverse { 1.0 - value } else { value };
        let mut expected_ys = [0.0, 0.25, 0.382, 1.0]
            .map(|value| (a.1 + (b.1 - a.1) * ratio(value)).round() as i32)
            .to_vec();
        let mut expected_xs = [0.0, 0.25, 0.382, 1.0]
            .map(|value| (a.0 + (b.0 - a.0) * ratio(value)).round() as i32)
            .to_vec();
        expected_ys.sort_unstable();
        expected_xs.sort_unstable();
        assert_eq!((ys, xs), (expected_ys, expected_xs), "reverse {reverse}");
        for (_, x0, x1, _) in scene.hlines(css(INK)) {
            assert_eq!((x0, x1), (a.0.round() as i32, b.0.round() as i32));
        }
    }
    assert!(chart.drawing_apply_options(id, r#"{"level_reverse":false}"#));
    assert_eq!(
        body(&chart, probe.0, probe.1),
        Some((id, crate::DrawingDragPart::Body))
    );
}

/// A level beyond 1 puts a grid line past the fan's anchors: the culling bounds cover it, so it
/// paints (and hits) while the anchors are off the pane.
#[test]
fn fan_grid_lines_beyond_the_anchors_stay_candidates() {
    let mut chart = chart();
    let levels = levels_json(&[(0.0, "#abcdef"), (1.0, "#abcdef"), (1.618, "#abcdef")]);
    let left = chart.time_scale.coordinate_to_float_index(0.0);
    // The anchors sit 30 and 10 bars left of the pane; level 1.618 lands inside it.
    let (start, end) = (left - 30.0, left - 10.0);
    let id = add(
        &mut chart,
        DrawingKind::FibonacciSpeedFan,
        vec![p(start, 101.0), p(end, 105.0)],
        &format!(
            r##"{{"color":"{INK}","levels":{levels},"tool_options":{{"fibonacci":{{"grid":true}}}}}}"##
        ),
    );
    for index in 0..22 {
        add(
            &mut chart,
            DrawingKind::TrendLine,
            vec![p(30.0 + f64::from(index) * 0.1, 100.0), p(31.0, 100.5)],
            "{}",
        );
    }
    let x = x_of(&chart, start + (end - start) * 1.618);
    assert!(x > 0.0 && x < chart.pane_w, "{x}");
    let lines = Scene::of(&mut chart).vlines(css(INK));
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!((f64::from(lines[0].0) - x).abs() <= 1.0);
    let y = (y_of(&chart, 101.0) + y_of(&chart, 105.0)) / 2.0;
    assert_eq!(chart.hit_test_drawing(x, y).map(|hit| hit.id), Some(id));
    assert_eq!(
        chart.hit_test_drawing_bruteforce(x, y).map(|hit| hit.id),
        Some(id)
    );
}

/// Full circles turn the speed arcs into whole rings around the second anchor.
#[test]
fn full_circles_turn_speed_arcs_into_rings() {
    let mut chart = chart();
    let levels = levels_json(&[(1.0, "#abcdef")]);
    let id = add(
        &mut chart,
        DrawingKind::FibonacciSpeedArcs,
        vec![p(14.0, 101.5), p(20.0, 103.0)],
        &format!(r#"{{"levels":{levels}}}"#),
    );
    let (a, b) = (anchor(&chart, id, 0), anchor(&chart, id, 1));
    let radius = (a.0 - b.0).hypot(a.1 - b.1);
    let opposite = (2.0 * b.0 - a.0, 2.0 * b.1 - a.1);
    let far_side = |points: &[(f64, f64)]| {
        points.iter().any(|p| {
            (p.0 - b.0) * (a.0 - b.0) + (p.1 - b.1) * (a.1 - b.1) < -0.99 * radius * radius
        })
    };
    let arc = Scene::of(&mut chart).polylines(css("#abcdef"));
    assert_eq!(arc.len(), 1);
    assert!(!far_side(&arc[0].0), "a half arc faces the first anchor");
    assert_eq!(body(&chart, opposite.0, opposite.1), None);
    assert!(chart.drawing_apply_options(
        id,
        r#"{"tool_options":{"fibonacci":{"full_circles":true}}}"#
    ));
    let ring = Scene::of(&mut chart).polylines(css("#abcdef"));
    assert_eq!(ring.len(), 1);
    let points = &ring[0].0;
    assert!(
        far_side(points),
        "the ring passes opposite the first anchor"
    );
    assert!(
        points
            .iter()
            .all(|p| ((p.0 - b.0).hypot(p.1 - b.1) - radius).abs() < 0.3)
    );
    assert!(close(points[0], *points.last().unwrap(), 0.01), "closed");
    assert_eq!(
        body(&chart, opposite.0, opposite.1).map(|hit| hit.0),
        Some(id)
    );
}

/// `label_v_align` moves the price labels above, onto (beside the line's end) or below their
/// lines, and the time labels to the pane's top, middle or bottom; unset is upstream's top row.
#[test]
fn label_v_align_places_level_labels() {
    let mut chart = chart();
    let id = add(
        &mut chart,
        DrawingKind::FibonacciRetracement,
        vec![p(10.0, 101.0), p(18.0, 105.0)],
        "{}",
    );
    let (x0, x1) = (anchor(&chart, id, 0).0, anchor(&chart, id, 1).0);
    let y = y_of(&chart, 103.0);
    let half = |chart: &mut ChartEngine| Scene::of(chart).text("50.0%").unwrap();
    let (_, x, top_y, align) = half(&mut chart);
    assert!((top_y - (y - 8.0)).abs() < 0.5 && (x - x1).abs() < 0.5 && align == TextAlign::Right);
    for (options, expected) in [
        (
            r#"{"tool_options":{"fibonacci":{"label_v_align":"top"}}}"#,
            (x1, y - 8.0, TextAlign::Right),
        ),
        (
            r#"{"tool_options":{"fibonacci":{"label_v_align":"bottom"}}}"#,
            (x1, y + 8.0, TextAlign::Right),
        ),
        (
            r#"{"level_label_align":"left","tool_options":{"fibonacci":{"label_v_align":"middle"}}}"#,
            (x0 - 6.0, y, TextAlign::Right),
        ),
        (r#"{"extend_left":true}"#, (6.0, y, TextAlign::Left)),
        (
            r#"{"extend_left":false,"level_label_align":"right"}"#,
            (x1 + 6.0, y, TextAlign::Left),
        ),
        (
            r#"{"level_label_align":"center"}"#,
            ((x0 + x1) / 2.0, y, TextAlign::Center),
        ),
    ] {
        assert!(chart.drawing_apply_options(id, options));
        let (_, x, label_y, align) = half(&mut chart);
        assert!(
            (x - expected.0).abs() < 0.5
                && (label_y - expected.1).abs() < 0.5
                && align == expected.2,
            "{options}: {:?} vs {expected:?}",
            (x, label_y, align)
        );
    }
    chart.remove_drawing(id);

    let (top, bottom) = pane_box(&chart);
    let id = add(
        &mut chart,
        DrawingKind::FibonacciTimeZones,
        vec![p(10.0, 101.0), p(12.0, 101.0)],
        "{}",
    );
    for (align, expected) in [
        ("top", top + 14.0),
        ("middle", (top + bottom) / 2.0),
        ("bottom", bottom - 14.0),
    ] {
        assert!(chart.drawing_apply_options(
            id,
            &format!(r#"{{"tool_options":{{"fibonacci":{{"label_v_align":"{align}"}}}}}}"#)
        ));
        let (_, _, y, _) = Scene::of(&mut chart).text("1").unwrap();
        assert!((y - expected).abs() < 0.5, "{align}: {y} vs {expected}");
    }
}

/// Documents the fork wrote label their price levels left of the lines, centered on them, and
/// their time levels at the pane's bottom, right of the lines.
#[test]
fn fork_documents_restore_their_label_placement() {
    let document = fork_document(&[
        (
            "fib_retracement",
            &[(10.0, 101.0), (18.0, 105.0)],
            serde_json::Value::Null,
        ),
        (
            "fib_time_zone",
            &[(22.0, 101.0), (24.0, 101.0)],
            serde_json::Value::Null,
        ),
    ]);
    let mut chart = chart();
    chart.import_state_json(&document).unwrap();
    let scene = Scene::of(&mut chart);
    let x0 = anchor(&chart, 1, 0).0;
    // The fork showed level values, not percents.
    let (_, x, y, align) = scene.text("0.5 ").unwrap();
    assert!(x < x0 - 5.0 && align == TextAlign::Right, "{x} vs {x0}");
    assert!((y - y_of(&chart, 103.0)).abs() < 0.5);
    let (_, bottom) = pane_box(&chart);
    let line = anchor(&chart, 2, 1).0;
    let (_, x, y, align) = scene
        .texts()
        .into_iter()
        .find(|(text, ..)| text == "1")
        .unwrap();
    assert!((y - (bottom - 14.0)).abs() < 0.5, "{y}");
    assert!(
        x > line && align == TextAlign::Left,
        "right of the line: {x} vs {line}"
    );
}

/// A middle-left label hangs past its anchors: with the anchors just past the pane's right edge
/// it still paints, and it hits on the indexed path like the brute-force one.
#[test]
fn labels_beyond_the_anchors_stay_painted_and_hit() {
    let mut chart = chart();
    for index in 0..22 {
        add(
            &mut chart,
            DrawingKind::TrendLine,
            vec![p(1.0 + f64::from(index) * 0.1, 100.0), p(2.0, 100.5)],
            "{}",
        );
    }
    let mut start = 39.0;
    while x_of(&chart, start) < chart.pane_w + 50.0 {
        start += 0.5;
    }
    let id = add(
        &mut chart,
        DrawingKind::FibonacciRetracement,
        vec![p(start, 101.0), p(start + 3.0, 105.0)],
        r#"{"level_label_align":"left","tool_options":{"fibonacci":{"label_v_align":"middle"}}}"#,
    );
    assert!(anchor(&chart, id, 0).0 > chart.pane_w + 40.0);
    let scene = Scene::of(&mut chart);
    let label = scene
        .text("50.0%")
        .expect("the label left of the anchors paints");
    let center = label_center(&chart, &label);
    let probe = (center.0.min(chart.pane_w - 5.0), center.1);
    assert!(probe.0 > center.0 - 40.0);
    assert_eq!(
        chart.hit_test_drawing(probe.0, probe.1).map(|hit| hit.id),
        Some(id)
    );
    assert_eq!(
        chart
            .hit_test_drawing_bruteforce(probe.0, probe.1)
            .map(|hit| hit.id),
        Some(id)
    );
}

/// Level labels are body targets on every level arm: the retracement's, the time zones' and the
/// circles' painted runs hit, and just beyond their boxes nothing does.
#[test]
fn level_labels_are_body_targets() {
    for (kind, points, prefix) in [
        (
            DrawingKind::FibonacciRetracement,
            vec![p(10.0, 101.0), p(18.0, 105.0)],
            "50.0%",
        ),
        (
            DrawingKind::FibonacciTimeZones,
            vec![p(10.0, 101.0), p(12.0, 101.0)],
            "3",
        ),
        (
            DrawingKind::FibonacciCircles,
            vec![p(14.0, 101.5), p(20.0, 103.0)],
            "61.8%",
        ),
    ] {
        let mut chart = chart();
        let options = if kind == DrawingKind::FibonacciCircles {
            format!(r#"{{"levels":{}}}"#, levels_json(&[(0.618, "#111111")]))
        } else {
            "{}".to_string()
        };
        let id = add(&mut chart, kind, points, &options);
        let label = Scene::of(&mut chart).text(prefix).unwrap();
        let center = label_center(&chart, &label);
        assert_eq!(
            body(&chart, center.0, center.1),
            Some((id, crate::DrawingDragPart::Body)),
            "{kind:?}"
        );
        // Above the line box: 1.25 × 12 px tall around the run's center.
        assert_eq!(body(&chart, center.0, center.1 - 8.5), None, "{kind:?}");
    }
}

/// Bands are drag surfaces while the drawing is selected (the rectangle's convention), on every
/// level arm: price bands, time bands, ring bands (precise or not) and the Gann fan's ray bands.
#[test]
fn selected_bands_drag_the_drawing() {
    let mut chart = chart();
    let price_levels = levels_json(&[(0.0, "#111111"), (0.5, "#222222"), (1.0, "#333333")]);
    let retracement = add(
        &mut chart,
        DrawingKind::FibonacciRetracement,
        vec![p(4.0, 101.0), p(12.0, 105.0)],
        &format!(r#"{{"levels":{price_levels}}}"#),
    );
    let in_band = (x_of(&chart, 4.0 + 8.0 * 0.2), y_of(&chart, 102.0));
    let time_levels = levels_json(&[(0.0, "#111111"), (1.0, "#222222"), (2.0, "#333333")]);
    let zones = add(
        &mut chart,
        DrawingKind::FibonacciTimeZones,
        vec![p(14.0, 101.0), p(16.0, 101.0)],
        &format!(r#"{{"levels":{time_levels},"fill_enabled":true}}"#),
    );
    let (top, bottom) = pane_box(&chart);
    let in_zone = (x_of(&chart, 15.0), (top + bottom) / 2.0 + 30.0);
    let ring_levels = levels_json(&[(0.5, "#111111"), (1.0, "#222222")]);
    let circles = add(
        &mut chart,
        DrawingKind::FibonacciCircles,
        vec![p(22.0, 101.0), p(26.0, 101.5)],
        &format!(r#"{{"levels":{ring_levels}}}"#),
    );
    let precise = add(
        &mut chart,
        DrawingKind::FibonacciCircles,
        vec![p(30.0, 104.0), p(34.0, 104.5)],
        &format!(
            r#"{{"levels":{ring_levels},"tool_options":{{"fibonacci":{{"trend_line":false}}}}}}"#
        ),
    );
    let in_ring = |chart: &ChartEngine, id: DrawingId| {
        let (a, b) = (anchor(chart, id, 0), anchor(chart, id, 1));
        // Below the center, three quarters of the way out (between levels 0.5 and 1).
        let radius = (a.0 - b.0).hypot(a.1 - b.1);
        (b.0, b.1 + radius * 0.75)
    };
    let fan = add(
        &mut chart,
        DrawingKind::GannFan,
        vec![p(2.0, 103.0), p(6.0, 104.0)],
        &format!(
            r#"{{"levels":{}}}"#,
            levels_json(&[(1.0, "#111111"), (2.0, "#222222")])
        ),
    );
    let in_fan = {
        let (a, b) = (anchor(&chart, fan, 0), anchor(&chart, fan, 1));
        // Between the 1x1 ray (through the second anchor) and the 2x ray, a bar past it.
        let x = b.0 + 15.0;
        let slope = (b.1 - a.1) / (b.0 - a.0);
        (x, a.1 + (x - a.0) * slope * 1.5)
    };
    let probes = [
        (retracement, in_band),
        (zones, in_zone),
        (circles, in_ring(&chart, circles)),
        (precise, in_ring(&chart, precise)),
        (fan, in_fan),
    ];
    for (id, (x, y)) in probes {
        assert_eq!(body(&chart, x, y), None, "{id} unselected pans the chart");
        chart.set_selected_drawing(Some(id));
        assert_eq!(
            body(&chart, x, y),
            Some((id, crate::DrawingDragPart::Body)),
            "{id} selected"
        );
        assert_eq!(
            chart.hit_test_drawing_bruteforce(x, y).map(|hit| hit.id),
            Some(id)
        );
        chart.set_selected_drawing(None);
        assert_eq!(body(&chart, x, y), None, "{id} deselected");
    }
}

/// A selected band hits only where the frame paints it: on a reversed speed fan the last level
/// is the horizontal ray through the first anchor, whose band the frame paints as the 1 px box
/// from that ray to the prior ray's start, so the triangle above it is no drag surface, while
/// the band between two slanted rays is.
#[test]
fn selected_fan_bands_hit_where_the_frame_paints_them() {
    let mut chart = chart();
    let levels = levels_json(&[(0.25, "#111111"), (0.5, "#222222"), (1.0, "#333333")]);
    let id = add(
        &mut chart,
        DrawingKind::FibonacciSpeedFan,
        vec![p(10.0, 101.0), p(18.0, 105.0)],
        &format!(r#"{{"levels":{levels},"fill_enabled":true,"level_reverse":true}}"#),
    );
    chart.set_selected_drawing(Some(id));
    let (a, b) = (anchor(&chart, id, 0), anchor(&chart, id, 1));
    let x = a.0 + (b.0 - a.0) * 0.6;
    // At 0.6 of the way the rays of values 0.75 and 0.5 sit at 0.45 and 0.3 of the rise.
    let at_rise = |share: f64| (x, a.1 + (b.1 - a.1) * share);
    let frame = chart.build_frame();
    let fills = frame.panes[0]
        .main
        .iter()
        .filter(|prim| matches!(prim, Prim::BandFill { .. }))
        .count();
    let boxes = frame.panes[0]
        .main
        .iter()
        .filter_map(|prim| match prim {
            Prim::Rect { rect, .. }
                if rect.x == a.0.round() as i32 && rect.w == (b.0 - a.0).round() as i32 =>
            {
                Some(rect.h)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!((fills, boxes.as_slice()), (1, [1].as_slice()), "{boxes:?}");
    let (x, y) = at_rise(0.375);
    assert_eq!(body(&chart, x, y), Some((id, crate::DrawingDragPart::Body)));
    let (x, y) = at_rise(0.15);
    assert_eq!(body(&chart, x, y), None, "unpainted triangle");
    assert_eq!(
        chart.hit_test_drawing_bruteforce(x, y).map(|hit| hit.id),
        None
    );
}

/// Fork ring-tool documents repaint the fork's rings after their anchor migrations: full-circle
/// speed arcs ring the fork's first anchor (upstream's second after the swap), the circles' trend
/// line is the fork's a→b diameter through the migrated center, and every ring tool (the wedge
/// with its block written empty) paints precise rings within a quarter pixel instead of
/// upstream's 33-point polylines.
#[test]
fn fork_ring_documents_repaint_their_rings_around_the_fork_anchors() {
    let fork_arcs = [(14.0, 103.0), (15.0, 103.2)];
    let fork_circles = [(24.0, 102.5), (25.0, 103.0)];
    let wedge = [(30.0, 102.0), (33.0, 103.0), (33.0, 101.5)];
    let document = fork_document(&[
        (
            "fib_speed_resistance_arcs",
            &fork_arcs,
            serde_json::json!({"tool_options": {"fibonacci": {"full_circles": true}}}),
        ),
        ("fib_circles", &fork_circles, serde_json::Value::Null),
        ("fib_wedge", &wedge, serde_json::Value::Null),
    ]);
    let mut chart = chart();
    chart.import_state_json(&document).unwrap();
    chart.build_frame();
    let wedge_points = wedge.iter().map(|&(logical, price)| p(logical, price));
    let upstream_wedge = add(
        &mut chart,
        DrawingKind::FibonacciWedge,
        wedge_points.collect(),
        "{}",
    );
    let only = |chart: &mut ChartEngine, id: DrawingId| {
        for other in [1, 2, 3, upstream_wedge] {
            assert!(
                chart.drawing_apply_options(other, &format!(r#"{{"visible":{}}}"#, other == id))
            );
        }
    };
    // Every polyline of the visible drawing that keeps one distance from `center`.
    let rings = |chart: &mut ChartEngine, center: (f64, f64)| {
        let scene = Scene::of(chart);
        scene
            .prims
            .iter()
            .filter_map(|prim| match prim {
                Prim::Polyline {
                    first_point,
                    point_count,
                    ..
                } if *point_count > 2 => Some(scene.run(*first_point, *point_count)),
                _ => None,
            })
            .filter(|points| {
                let radius = (points[0].0 - center.0).hypot(points[0].1 - center.1);
                points.iter().all(|point| {
                    ((point.0 - center.0).hypot(point.1 - center.1) - radius).abs() < 0.5
                })
            })
            .collect::<Vec<_>>()
    };
    let precise = |points: &[(f64, f64)], center: (f64, f64)| {
        let radius = (points[0].0 - center.0).hypot(points[0].1 - center.1);
        points.len() != 33
            && points.windows(2).all(|pair| {
                let middle = ((pair[0].0 + pair[1].0) / 2.0, (pair[0].1 + pair[1].1) / 2.0);
                radius - (middle.0 - center.0).hypot(middle.1 - center.1) <= 0.26
            })
    };

    // Speed arcs: closed rings around the fork's first anchor, now upstream's second.
    only(&mut chart, 1);
    let center = (x_of(&chart, fork_arcs[0].0), y_of(&chart, fork_arcs[0].1));
    assert!(close(anchor(&chart, 1, 1), center, 0.01));
    let arcs = rings(&mut chart, center);
    assert!(arcs.len() >= 3, "{arcs:?}");
    for ring in &arcs {
        assert!(close(ring[0], *ring.last().unwrap(), 0.01), "closed");
        assert!(precise(ring, center), "{} points", ring.len());
    }

    // Circles: the trend line runs along the fork's a→b, the diameter through the midpoint.
    only(&mut chart, 2);
    let color = css(&chart.drawing(2).unwrap().color);
    let path = fork_circles.map(|(logical, price)| (x_of(&chart, logical), y_of(&chart, price)));
    // A level ring in the drawing's color is not part of the line.
    let runs = Scene::of(&mut chart)
        .runs(color)
        .into_iter()
        .filter(|run| {
            run.iter()
                .all(|&point| aeris_charts_render::shape::distance_to_polyline(point, &path) < 0.01)
        })
        .collect::<Vec<_>>();
    assert!(
        runs.len() > 1 && strokes_along(&runs, &path, 1.0),
        "{path:?}: {runs:?}"
    );
    let center = ((path[0].0 + path[1].0) / 2.0, (path[0].1 + path[1].1) / 2.0);
    let circles = rings(&mut chart, center);
    assert!(!circles.is_empty());
    assert!(circles.iter().all(|ring| precise(ring, center)));

    // Wedge: the fork's empty block selects the precise rings; a new wedge keeps upstream's.
    let center = (x_of(&chart, wedge[0].0), y_of(&chart, wedge[0].1));
    only(&mut chart, upstream_wedge);
    let upstream = rings(&mut chart, center);
    assert!(!upstream.is_empty() && upstream.iter().all(|ring| ring.len() == 33));
    only(&mut chart, 3);
    let fork = rings(&mut chart, center);
    assert_eq!(fork.len(), upstream.len());
    assert!(fork.iter().all(|ring| precise(ring, center)));
}

/// An empty spiral paints the golden spiral: through the second anchor, growing by φ every
/// quarter turn, clockwise (counterclockwise with `reverse`), until it leaves the pane; bounded.
#[test]
fn spirals_without_levels_grow_by_phi_every_quarter_turn_through_the_second_anchor() {
    let mut chart = chart();
    let id = add(
        &mut chart,
        DrawingKind::FibonacciSpiral,
        vec![p(20.0, 103.0), p(22.0, 103.0)],
        &format!(r##"{{"color":"{INK}","levels":[]}}"##),
    );
    let (a, b) = (anchor(&chart, id, 0), anchor(&chart, id, 1));
    let spiral = |chart: &mut ChartEngine| {
        let runs = Scene::of(chart).runs(css(INK));
        assert_eq!(runs.len(), 1, "one solid run");
        runs.into_iter().next().unwrap()
    };
    let points = spiral(&mut chart);
    assert!(aeris_charts_render::shape::distance_to_polyline(b, &points) < 0.5);
    let phi = (1.0 + 5.0_f64.sqrt()) / 2.0;
    let growth = |points: &[(f64, f64)], clockwise: bool| {
        let radius = |point: (f64, f64)| (point.0 - a.0).hypot(point.1 - a.1);
        let angle = |point: (f64, f64)| (point.1 - a.1).atan2(point.0 - a.0);
        let pi = std::f64::consts::PI;
        points.windows(2).all(|pair| {
            let turn = (angle(pair[1]) - angle(pair[0]) + pi).rem_euclid(2.0 * pi) - pi;
            let expected = phi.powf(turn.abs() / std::f64::consts::FRAC_PI_2);
            // Frame points are f32: the innermost sub-pixel turns are too coarse to compare.
            radius(pair[0]) < 2.0
                || ((turn > 0.0) == clockwise
                    && (radius(pair[1]) / radius(pair[0]) / expected - 1.0).abs() < 1e-3)
        })
    };
    assert!(growth(&points, true));
    // It hits along the curve: a quarter turn inside the second anchor, above the center.
    let r = (b.0 - a.0).abs();
    let probe = (a.0, a.1 - r / phi);
    assert_eq!(
        body(&chart, probe.0, probe.1).map(|hit| hit.0),
        Some(id),
        "{probe:?}"
    );
    let (top, bottom) = pane_box(&chart);
    let border = points
        .iter()
        .map(|point| {
            point
                .0
                .min(chart.pane_w - point.0)
                .min(point.1 - top)
                .min(bottom - point.1)
        })
        .fold(f64::INFINITY, f64::min);
    assert!(border <= 1.0, "it grows until it leaves the pane: {border}");
    assert!(points.len() < 4_000, "bounded: {}", points.len());
    assert!(chart.drawing_apply_options(id, r#"{"tool_options":{"fibonacci":{"reverse":true}}}"#));
    assert!(growth(&spiral(&mut chart), false));
    assert_eq!(body(&chart, probe.0, probe.1), None, "turned the other way");
    assert_eq!(
        body(&chart, probe.0, a.1 + r / phi).map(|hit| hit.0),
        Some(id)
    );
}

/// A golden spiral centered far off the pane stays within the curve tolerance where it shows,
/// and stays a candidate (the whole pane is its screen box).
#[test]
fn spirals_centered_far_off_the_pane_stay_within_the_curve_tolerance() {
    let mut chart = chart();
    let (top, bottom) = pane_box(&chart);
    // The center sits 100,000 px below the pane; the second anchor straight above it, inside.
    let points = vec![
        at(&chart, 300.0, bottom + 100_000.0),
        at(&chart, 300.0, bottom - 200.0),
    ];
    let id = add(
        &mut chart,
        DrawingKind::FibonacciSpiral,
        points,
        &format!(r##"{{"color":"{INK}","levels":[]}}"##),
    );
    for index in 0..22 {
        add(
            &mut chart,
            DrawingKind::TrendLine,
            vec![p(30.0 + f64::from(index) * 0.1, 100.0), p(31.0, 100.5)],
            "{}",
        );
    }
    let a = anchor(&chart, id, 0);
    let runs = Scene::of(&mut chart).runs(css(INK));
    let mut chords = 0;
    for run in &runs {
        assert!(run.len() <= 257, "bounded: {}", run.len());
        for pair in run.windows(2) {
            let middle = ((pair[0].0 + pair[1].0) / 2.0, (pair[0].1 + pair[1].1) / 2.0);
            if middle.0 < 0.0 || middle.0 > chart.pane_w || middle.1 < top || middle.1 > bottom {
                continue;
            }
            chords += 1;
            // A logarithmic spiral's radius at the middle angle is the ends' geometric mean.
            let radius = |point: (f64, f64)| (point.0 - a.0).hypot(point.1 - a.1);
            let sag = (radius(pair[0]) * radius(pair[1])).sqrt() - radius(middle);
            assert!(sag <= 0.25 + 0.05, "chord sags {sag} px");
        }
    }
    assert!(chords > 0, "the spiral crosses the pane");
    let candidates = chart.take_drawing_candidates(0, None);
    assert!(candidates.contains(&id));
    chart.recycle_drawing_candidates(candidates);
    let crossing = runs
        .iter()
        .flatten()
        .copied()
        .find(|point| {
            point.0 > 50.0
                && point.0 < chart.pane_w - 50.0
                && point.1 > top + 50.0
                && point.1 < bottom - 50.0
        })
        .expect("a point inside the pane");
    assert_eq!(
        chart
            .hit_test_drawing(crossing.0, crossing.1)
            .map(|hit| hit.id),
        Some(id)
    );
    assert_eq!(
        chart
            .hit_test_drawing_bruteforce(crossing.0, crossing.1)
            .map(|hit| hit.id),
        Some(id)
    );
}

/// Dash runs of `color` whose points all lie inside the pane, translated by `shift`.
fn dashes_inside(chart: &mut ChartEngine, color: &str, shift: (f64, f64)) -> Vec<Vec<(f64, f64)>> {
    let (top, bottom) = pane_box(chart);
    let w = chart.pane_w;
    Scene::of(chart)
        .runs(css(color))
        .into_iter()
        .filter(|run| {
            run.iter().all(|point| {
                point.0 > 1.0 && point.0 < w - 1.0 && point.1 > top + 1.0 && point.1 < bottom - 1.0
            })
        })
        .map(|run| {
            run.iter()
                .map(|point| (point.0 + shift.0, point.1 + shift.1))
                .collect()
        })
        .collect()
}

/// The dashes of a golden spiral and of precise rings centered off the pane follow the curve,
/// not the pane, while it scrolls.
#[test]
fn dashed_spirals_and_precise_rings_keep_their_dash_phase_while_the_pane_scrolls() {
    let ring = r##"{"value":2.4,"color":"#111111","visible":true,"style":"dashed","fill_between":false,"label_visible":false}"##;
    for (kind, options) in [
        (
            DrawingKind::FibonacciCircles,
            format!(
                r#"{{"levels":[{ring}],"tool_options":{{"fibonacci":{{"trend_line":false}}}}}}"#
            ),
        ),
        (
            DrawingKind::FibonacciSpiral,
            r##"{"color":"#111111","style":"dashed","levels":[]}"##.to_string(),
        ),
    ] {
        let mut chart = chart();
        let (top, bottom) = pane_box(&chart);
        let middle = (top + bottom) / 2.0;
        let points = match kind {
            // Upstream's circles center on the second anchor.
            DrawingKind::FibonacciCircles => {
                vec![at(&chart, -520.0, middle), at(&chart, -320.0, middle)]
            }
            _ => vec![at(&chart, -60.0, middle), at(&chart, -30.0, middle)],
        };
        let id = add(&mut chart, kind, points, &options);
        let center = |chart: &ChartEngine| {
            anchor(
                chart,
                id,
                usize::from(kind == DrawingKind::FibonacciCircles),
            )
        };
        assert!(center(&chart).0 < 0.0, "{kind:?} is centered off the pane");
        let before_center = center(&chart);
        let before = dashes_inside(&mut chart, "#111111", (0.0, 0.0));
        let (from, to) = chart.visible_logical_range().unwrap();
        chart.set_visible_logical_range(from + 0.37, to + 0.37);
        chart.build_frame();
        let after_center = center(&chart);
        let shift = (
            before_center.0 - after_center.0,
            before_center.1 - after_center.1,
        );
        assert!(
            shift.0 > 3.0 && shift.1.abs() < 1e-6,
            "{kind:?} scrolled: {shift:?}"
        );
        let after = dashes_inside(&mut chart, "#111111", shift);
        assert!(
            !after.is_empty()
                && after.iter().all(|dash| before.iter().any(|other| {
                    close(dash[0], other[0], 0.3)
                        && close(*dash.last().unwrap(), *other.last().unwrap(), 0.3)
                })),
            "{kind:?} dashes follow the curve, not the pane"
        );
    }
}

/// A ring tool with a stored block (every fork document) tessellates within a quarter pixel over
/// the part of the ring the pane shows, bounded; without one it keeps upstream's 33 points.
#[test]
fn precise_rings_stay_within_a_quarter_pixel_where_they_show() {
    let mut chart = chart();
    let (top, bottom) = pane_box(&chart);
    // The center sits about 8,000 px above the pane: upstream's 32 chords sag by tens of px.
    let (center, edge) = (
        at(&chart, 300.0, top - 8_000.0),
        at(&chart, 300.0, top - 7_900.0),
    );
    let unit = 100.0;
    let radii = [8_000.0 + 100.0, 8_000.0 + 300.0];
    let levels = levels_json(&[(radii[0] / unit, "#111111"), (radii[1] / unit, "#222222")]);
    let id = add(
        &mut chart,
        DrawingKind::FibonacciCircles,
        vec![edge, center],
        &format!(r#"{{"levels":{levels},"fill_enabled":false}}"#),
    );
    let c = anchor(&chart, id, 1);
    let unit = (anchor(&chart, id, 0).1 - c.1).abs();
    assert_eq!(
        Scene::of(&mut chart).polylines(css("#111111"))[0].0.len(),
        33
    );
    // Any stored key keeps the block (an empty patch block is dropped).
    assert!(
        chart.drawing_apply_options(id, r#"{"tool_options":{"fibonacci":{"trend_line":false}}}"#)
    );
    let scene = Scene::of(&mut chart);
    for (value, color) in [(radii[0] / 100.0, "#111111"), (radii[1] / 100.0, "#222222")] {
        let radius = unit * value;
        let arcs = scene.polylines(css(color));
        assert_eq!(arcs.len(), 1, "{color}");
        let points = &arcs[0].0;
        assert!(points.len() <= 257, "bounded: {}", points.len());
        let sag = points
            .windows(2)
            .map(|pair| {
                let middle = ((pair[0].0 + pair[1].0) / 2.0, (pair[0].1 + pair[1].1) / 2.0);
                radius - (middle.0 - c.0).hypot(middle.1 - c.1)
            })
            .fold(0.0_f64, f64::max);
        assert!(sag <= 0.25 + 0.05, "{color}: chords sag {sag} px");
        assert!(points.iter().any(|point| point.1 > top && point.1 < bottom));
    }
    // It hits along the visible ring.
    let y = c.1 + unit * radii[0] / 100.0;
    assert_eq!(chart.hit_test_drawing(c.0, y).map(|hit| hit.id), Some(id));
}

/// The fib channel's `extend_left`/`extend_right` run every level (and so its bands and labels)
/// to the pane edges on that side of the screen.
#[test]
fn channel_levels_follow_extend_left_and_right() {
    let mut chart = chart();
    let levels = levels_json(&[(0.0, "#111111"), (1.0, "#222222")]);
    let id = add(
        &mut chart,
        DrawingKind::FibonacciChannel,
        vec![p(12.0, 101.0), p(20.0, 103.0), p(16.0, 104.0)],
        &format!(r#"{{"levels":{levels}}}"#),
    );
    let (a, b) = (anchor(&chart, id, 0), anchor(&chart, id, 1));
    let slope = (b.1 - a.1) / (b.0 - a.0);
    let base = |chart: &mut ChartEngine| Scene::of(chart).polylines(css("#111111"))[0].0.clone();
    assert!(close(base(&mut chart)[0], a, 0.01) && close(base(&mut chart)[1], b, 0.01));
    let w = chart.pane_w;
    for (options, left, right) in [
        (r#"{"extend_left":true}"#, true, false),
        (r#"{"extend_left":false,"extend_right":true}"#, false, true),
        (r#"{"extend_left":true,"extend_right":true}"#, true, true),
    ] {
        assert!(chart.drawing_apply_options(id, options));
        let line = base(&mut chart);
        let expected_start = if left { (0.0, a.1 - a.0 * slope) } else { a };
        let expected_end = if right {
            (w, b.1 + (w - b.0) * slope)
        } else {
            b
        };
        assert!(close(line[0], expected_start, 0.05), "{options}: {line:?}");
        assert!(close(line[1], expected_end, 0.05), "{options}: {line:?}");
    }
    // The extended base line hits beyond the anchors.
    let x = a.0 / 2.0;
    assert_eq!(
        body(&chart, x, a.1 + (x - a.0) * slope).map(|hit| hit.0),
        Some(id)
    );
}

/// Documents the fork wrote keep its look: the dashed trend line in the neutral gray (written
/// back explicitly), the fan grid, and the golden spiral (in the stored direction).
#[test]
fn fork_documents_restore_their_trend_lines_grid_and_spirals() {
    let document = fork_document(&[
        (
            "fib_retracement",
            &[(10.0, 101.0), (18.0, 105.0)],
            serde_json::Value::Null,
        ),
        (
            "fib_retracement",
            &[(22.0, 101.0), (30.0, 105.0)],
            serde_json::json!({"tool_options": {"fibonacci": {"reverse": true}}}),
        ),
        (
            "fib_speed_resistance_fan",
            &[(4.0, 100.5), (9.0, 102.5)],
            serde_json::Value::Null,
        ),
        (
            "fib_spiral",
            &[(20.0, 103.0), (22.0, 103.0)],
            serde_json::Value::Null,
        ),
        (
            "fib_spiral",
            &[(30.0, 103.0), (32.0, 103.0)],
            serde_json::json!({"tool_options": {"fibonacci": {"reverse": true}}}),
        ),
    ]);
    let mut chart = chart();
    chart.import_state_json(&document).unwrap();
    // The fork's defaults are written back explicitly.
    let exported: serde_json::Value =
        serde_json::from_str(&chart.export_state_json().unwrap()).unwrap();
    let written = |index: usize, key: &str| {
        exported["drawings"][index]["style"]["tool_options"]["fibonacci"][key].clone()
    };
    assert_eq!(written(0, "trend_line"), true);
    assert_eq!(written(2, "grid"), true);
    let only = |chart: &mut ChartEngine, id: DrawingId| {
        for other in 1..=5 {
            assert!(
                chart.drawing_apply_options(other, &format!(r#"{{"visible":{}}}"#, other == id))
            );
        }
    };
    let gray = css("#787b86");
    for id in [1, 2] {
        only(&mut chart, id);
        let runs = Scene::of(&mut chart).runs(gray);
        let path = [anchor(&chart, id, 0), anchor(&chart, id, 1)];
        assert!(
            runs.len() > 1 && strokes_along(&runs, &path, 1.0),
            "{id}: {runs:?}"
        );
    }
    only(&mut chart, 3);
    let scene = Scene::of(&mut chart);
    // Seven grid lines each way; level 0's horizontal ray (gray too) lies on the bottom one.
    assert_eq!((scene.hlines(gray).len(), scene.vlines(gray).len()), (8, 7));
    assert!(chart.drawing_apply_options(3, r#"{"tool_options":{"fibonacci":{"grid":false}}}"#));
    let scene = Scene::of(&mut chart);
    assert_eq!((scene.hlines(gray).len(), scene.vlines(gray).len()), (1, 0));
    for (id, clockwise) in [(4, true), (5, false)] {
        only(&mut chart, id);
        let a = anchor(&chart, id, 0);
        let spiral = Scene::of(&mut chart)
            .polylines(css(crate::DRAWING_DEFAULT_COLOR))
            .into_iter()
            .map(|(points, _)| points)
            .max_by_key(Vec::len)
            .expect("the golden spiral");
        let angle = |point: (f64, f64)| (point.1 - a.1).atan2(point.0 - a.0);
        let middle = spiral.len() / 2;
        let turn = (angle(spiral[middle + 1]) - angle(spiral[middle]) + std::f64::consts::PI)
            .rem_euclid(std::f64::consts::TAU)
            - std::f64::consts::PI;
        assert_eq!(turn > 0.0, clockwise, "{id}");
    }
}

/// The schema lists the options each Fibonacci tool reads on upstream's arms, at upstream's
/// defaults.
#[test]
fn schema_lists_the_fibonacci_tool_options_each_tool_reads() {
    let rows = |kind: DrawingKind| {
        crate::drawing_property_schema(kind)
            .properties
            .into_iter()
            .filter(|property| property.name.starts_with("tool_options.fibonacci."))
            .map(|property| {
                (
                    property
                        .name
                        .trim_start_matches("tool_options.fibonacci.")
                        .to_string(),
                    property.default,
                )
            })
            .collect::<Vec<_>>()
    };
    let no = serde_json::json!(false);
    let top = serde_json::json!("top");
    assert_eq!(
        rows(DrawingKind::FibonacciRetracement),
        [
            ("trend_line".to_string(), no.clone()),
            ("label_v_align".to_string(), top.clone())
        ]
    );
    assert_eq!(
        rows(DrawingKind::FibonacciChannel),
        [("label_v_align".to_string(), top)]
    );
    assert_eq!(
        rows(DrawingKind::FibonacciSpeedFan),
        [("grid".to_string(), no.clone())]
    );
    assert_eq!(
        rows(DrawingKind::FibonacciSpeedArcs),
        [
            ("trend_line".to_string(), no.clone()),
            ("full_circles".to_string(), no.clone())
        ]
    );
    assert_eq!(
        rows(DrawingKind::FibonacciSpiral),
        [
            ("trend_line".to_string(), no.clone()),
            ("reverse".to_string(), no)
        ]
    );
    assert!(rows(DrawingKind::FibonacciWedge).is_empty());
    assert!(rows(DrawingKind::GannFan).is_empty());
    let align = crate::drawing_property_schema(DrawingKind::FibonacciTimeZones)
        .properties
        .into_iter()
        .find(|property| property.name == "tool_options.fibonacci.label_v_align")
        .unwrap();
    assert_eq!(align.enum_values, ["top", "middle", "bottom"]);
}

/// The fork presentation scales with the device pixel ratio like upstream's arms: label gaps,
/// grid lines and the golden spiral sit on the anchors' device px.
#[test]
fn fork_presentation_scales_with_the_device_pixel_ratio() {
    for dpr in [1.0, 1.5, 2.0] {
        let mut chart = chart_with(&hourly(40), dpr);
        let retracement = add(
            &mut chart,
            DrawingKind::FibonacciRetracement,
            vec![p(10.0, 101.0), p(18.0, 105.0)],
            r#"{"level_label_align":"left","tool_options":{"fibonacci":{"label_v_align":"middle"}}}"#,
        );
        let fan = add(
            &mut chart,
            DrawingKind::FibonacciSpeedFan,
            vec![p(22.0, 101.0), p(30.0, 105.0)],
            &format!(
                r##"{{"color":"{INK}","levels":{},"tool_options":{{"fibonacci":{{"grid":true}}}}}}"##,
                levels_json(&[(0.5, "#abcdef")])
            ),
        );
        let spiral = add(
            &mut chart,
            DrawingKind::FibonacciSpiral,
            vec![p(20.0, 103.0), p(22.0, 103.0)],
            r##"{"color":"#7b1fa2","levels":[]}"##,
        );
        let scene = Scene::of(&mut chart);
        let x0 = anchor(&chart, retracement, 0).0;
        let (_, x, y, _) = scene.text("50.0%").unwrap();
        assert!((x - (x0 - 6.0) * dpr).abs() < 0.6, "{dpr}: {x}");
        assert!((y - y_of(&chart, 103.0) * dpr).abs() < 0.6, "{dpr}: {y}");
        let (a, b) = (anchor(&chart, fan, 0), anchor(&chart, fan, 1));
        let grid = scene.vlines(css(INK));
        assert_eq!(grid.len(), 1);
        assert!(
            (f64::from(grid[0].0) - (a.0 + b.0) / 2.0 * dpr).abs() <= 1.0,
            "{dpr}"
        );
        let b = anchor(&chart, spiral, 1);
        let runs = scene.runs(css("#7b1fa2"));
        assert!(
            runs.iter()
                .any(|run| aeris_charts_render::shape::distance_to_polyline(
                    (b.0 * dpr, b.1 * dpr),
                    run
                ) < 0.5 * dpr),
            "{dpr}"
        );
    }
}
