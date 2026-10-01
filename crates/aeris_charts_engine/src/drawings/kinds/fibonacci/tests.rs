//! Fibonacci-family engine tests: catalog defaults, armed placement and partial previews, level
//! geometry per tool (price, time, and screen-space levels), bands and labels, hit testing
//! (indexed and brute force), culling beyond the anchors, drags, keyboard nudges, magnet, time
//! identity, schema and kind options, level-list and option patches with history, templates,
//! persistence with default omission, clipboard, sync, and device-pixel scaling.

use aeris_charts_render::color::Color;
use aeris_charts_render::draw_list::{LineStyle, Prim};

use super::super::super::{DrawingPlacement, DrawingTextLayout};
use super::{
    golden_ratio, trimmed, visible_arc, FibonacciLabelHAlign, FibonacciLabelVAlign,
    FibonacciToolOptions,
};
use crate::{
    ChartEngine, DrawingAnchor, DrawingDragPart, DrawingId, DrawingKind, DrawingMagnetMode,
    DrawingModifiers, DrawingPoint, DrawingPriceScale,
};

const FIB_KINDS: [DrawingKind; 10] = [
    DrawingKind::FibRetracement,
    DrawingKind::TrendBasedFibExtension,
    DrawingKind::FibChannel,
    DrawingKind::FibTimeZone,
    DrawingKind::TrendBasedFibTime,
    DrawingKind::FibSpeedResistanceFan,
    DrawingKind::FibSpeedResistanceArcs,
    DrawingKind::FibCircles,
    DrawingKind::FibSpiral,
    DrawingKind::FibWedge,
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

fn anchor(chart: &ChartEngine, id: DrawingId, index: usize) -> (f64, f64) {
    chart.drawing_point_to_coordinate(id, index).unwrap()
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

/// One band fill: its paired upper and lower chains and its color.
type Band = (Vec<(f64, f64)>, Vec<(f64, f64)>, Color);

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

    /// `(y, x0, x1, style)` of every crisp horizontal line in `color`.
    fn hlines(&self, color: Color) -> Vec<(i32, i32, i32, LineStyle)> {
        self.prims
            .iter()
            .filter_map(|prim| match prim {
                Prim::HLine {
                    y,
                    x0,
                    x1,
                    style,
                    color: line,
                    ..
                } if *line == color => Some((*y, *x0, *x1, *style)),
                _ => None,
            })
            .collect()
    }

    /// `x` of every crisp vertical line in `color`.
    fn vlines(&self, color: Color) -> Vec<i32> {
        self.prims
            .iter()
            .filter_map(|prim| match prim {
                Prim::VLine { x, color: line, .. } if *line == color => Some(*x),
                _ => None,
            })
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

    /// `(upper, lower, fill)` of every band fill.
    fn fills(&self) -> Vec<Band> {
        self.prims
            .iter()
            .filter_map(|prim| match prim {
                Prim::BandFill {
                    upper_first,
                    lower_first,
                    point_count,
                    fill,
                    ..
                } => Some((
                    self.run(*upper_first, *point_count),
                    self.run(*lower_first, *point_count),
                    *fill,
                )),
                _ => None,
            })
            .collect()
    }

    /// `(text, x, y, color)` of every text run.
    fn texts(&self) -> Vec<(String, f64, f64, Color)> {
        self.prims
            .iter()
            .filter_map(|prim| match prim {
                Prim::Text {
                    text, x, y, color, ..
                } => Some((text.clone(), f64::from(*x), f64::from(*y), *color)),
                _ => None,
            })
            .collect()
    }

    fn text(&self, wanted: &str) -> Option<(f64, f64, Color)> {
        self.texts()
            .into_iter()
            .find(|(text, ..)| text == wanted)
            .map(|(_, x, y, color)| (x, y, color))
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

#[test]
fn catalog_defaults_follow_each_tool() {
    for kind in FIB_KINDS {
        let drawing = crate::Drawing::new(1, kind, 0, Vec::new());
        let spec = kind.spec();
        assert!(spec.family.is_some(), "{kind:?} is a family tool");
        assert_eq!(spec.text_layout, DrawingTextLayout::Box);
        assert!(!spec.axis_price_label);
        let count = match kind {
            DrawingKind::TrendBasedFibExtension
            | DrawingKind::FibChannel
            | DrawingKind::TrendBasedFibTime
            | DrawingKind::FibWedge => 3,
            _ => 2,
        };
        assert_eq!(spec.placement, DrawingPlacement::ClickAnchors { count });
        assert_eq!(drawing.width, 1.0);
        assert!(drawing.tool_options.is_empty());
        assert!(!drawing.extend_left && !drawing.extend_right);
        let spiral = kind == DrawingKind::FibSpiral;
        assert_eq!(drawing.levels.is_empty(), spiral, "{kind:?} levels");
        assert_eq!(
            drawing.fill_enabled,
            !matches!(kind, DrawingKind::FibTimeZone | DrawingKind::FibSpiral)
        );
        assert_eq!(drawing.color == "#787b86", !spiral);
        assert_eq!(
            drawing.style,
            if matches!(kind, DrawingKind::FibSpiral | DrawingKind::FibWedge) {
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
    let retracement = crate::Drawing::new(1, DrawingKind::FibRetracement, 0, Vec::new());
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
    let zones = crate::Drawing::new(1, DrawingKind::FibTimeZone, 0, Vec::new());
    assert_eq!(
        zones
            .levels
            .iter()
            .map(|level| level.value)
            .collect::<Vec<_>>(),
        [0.0, 1.0, 2.0, 3.0, 5.0, 8.0, 13.0, 21.0, 34.0, 55.0, 89.0]
    );
    let fan = crate::Drawing::new(1, DrawingKind::FibSpeedResistanceFan, 0, Vec::new());
    assert_eq!(
        fan.levels
            .iter()
            .map(|level| level.value)
            .collect::<Vec<_>>(),
        [0.0, 0.25, 0.382, 0.5, 0.618, 0.75, 1.0]
    );
}

#[test]
fn labels_format_values_percents_and_prices() {
    assert_eq!(trimmed(0.618, 4), "0.618");
    assert_eq!(trimmed(1.0, 4), "1");
    assert_eq!(trimmed(89.0, 4), "89");
    assert_eq!(trimmed(0.1 + 0.2, 4), "0.3");
    assert_eq!(trimmed(-0.0, 4), "0");
    assert_eq!(trimmed(61.8, 2), "61.8");
    let mut chart = chart();
    let id = add(
        &mut chart,
        DrawingKind::FibRetracement,
        vec![p(10.0, 101.0), p(20.0, 105.0)],
        "{}",
    );
    let scene = Scene::of(&mut chart);
    // 105 + (101 - 105) × 0.618 = 102.528.
    assert!(
        scene.text("0.618 (102.53)").is_some(),
        "{:?}",
        scene.texts()
    );
    assert!(chart.drawing_apply_options(
        id,
        r#"{"tool_options":{"fibonacci":{"levels_as_percent":true,"show_prices":false}}}"#
    ));
    let scene = Scene::of(&mut chart);
    assert!(scene.text("61.8%").is_some(), "{:?}", scene.texts());
    assert!(chart.drawing_apply_options(
        id,
        r#"{"tool_options":{"fibonacci":{"show_levels":false,"show_prices":true}}}"#
    ));
    assert!(Scene::of(&mut chart).text("102.53").is_some());
    assert!(chart.drawing_apply_options(
        id,
        r#"{"tool_options":{"fibonacci":{"show_levels":false,"show_prices":false}}}"#
    ));
    assert!(Scene::of(&mut chart).texts().is_empty());
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
        DrawingKind::TrendBasedFibExtension,
        DrawingKind::FibChannel,
        DrawingKind::TrendBasedFibTime,
        DrawingKind::FibWedge,
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
fn retracement_levels_sit_on_ratio_prices_with_bands_and_labels() {
    let mut chart = chart();
    let id = add(
        &mut chart,
        DrawingKind::FibRetracement,
        vec![p(10.0, 101.0), p(20.0, 105.0)],
        r##"{"color":"#123456"}"##,
    );
    let (a, b) = (anchor(&chart, id, 0), anchor(&chart, id, 1));
    let scene = Scene::of(&mut chart);
    // Level 0 on the second anchor, level 1 on the first, the rest between and beyond.
    for (value, color) in [
        (0.0, "#787b86"),
        (0.382, "#ff9800"),
        (0.5, "#4caf50"),
        (0.618, "#089981"),
        (1.0, "#787b86"),
    ] {
        let price = 105.0 + (101.0 - 105.0) * value;
        let y = y_of(&chart, price).round() as i32;
        assert!(
            scene.hlines(css(color)).contains(&(
                y,
                a.0.round() as i32,
                b.0.round() as i32,
                LineStyle::Solid
            )),
            "level {value} at {y}: {:?}",
            scene.hlines(css(color))
        );
    }
    // Bands between neighbouring levels take the upper level's color at 20% alpha.
    let fills = scene.fills();
    let golden = fills
        .iter()
        .find(|(.., fill)| *fill == Color::rgba(0x08, 0x99, 0x81, 51))
        .expect("0.5 → 0.618 band");
    let (y0, y1) = (y_of(&chart, 103.0), y_of(&chart, 102.528));
    assert!(close(golden.0[0], (a.0, y0), 1e-3) && close(golden.1[1], (b.0, y1), 1e-3));
    // The dashed trend line through both anchors in the drawing's stroke.
    let trend = scene.runs(css(INK));
    assert!(
        trend.len() > 1 && strokes_along(&trend, &[a, b], 1.0),
        "{trend:?}"
    );
    // Labels sit left of the levels in the level color.
    let (x, y, color) = scene.text("0.618 (102.53)").unwrap();
    assert!(x < a.0 - 6.0 && (y - y1).abs() < 1.0);
    assert_eq!(color, css("#089981"));

    // Reverse puts level 0 on the first anchor; log scale interpolates in log space.
    assert!(chart.drawing_apply_options(id, r#"{"tool_options":{"fibonacci":{"reverse":true}}}"#));
    let y = y_of(&chart, 101.0).round() as i32;
    assert!(Scene::of(&mut chart)
        .hlines(css("#787b86"))
        .iter()
        .any(|line| line.0 == y));
    assert!(chart.drawing_apply_options(
        id,
        r#"{"tool_options":{"fibonacci":{"reverse":false,"log_scale":true}}}"#
    ));
    let mid = (105.0f64.ln() + (101.0f64.ln() - 105.0f64.ln()) * 0.5).exp();
    let y = y_of(&chart, mid).round() as i32;
    assert!(Scene::of(&mut chart)
        .hlines(css("#4caf50"))
        .iter()
        .any(|line| line.0 == y));

    // Extensions reach the pane edges; labels then move inside the extended end.
    assert!(chart.drawing_apply_options(
        id,
        r#"{"extend_left":true,"extend_right":true,"tool_options":{"fibonacci":{"log_scale":false}}}"#
    ));
    let scene = Scene::of(&mut chart);
    let pane_w = chart.pane_w.round() as i32;
    assert!(scene
        .hlines(css("#4caf50"))
        .iter()
        .all(|line| line.1 == 0 && line.2 == pane_w));
    let (x, ..) = scene.text("0.5 (103.00)").unwrap();
    assert!((x - 6.0).abs() < 1e-3, "left-aligned inside the pane: {x}");

    // The trend line toggles off; hidden levels paint nothing.
    assert!(chart.drawing_apply_options(
        id,
        r#"{"tool_options":{"fibonacci":{"trend_line":false}},"fill_enabled":false}"#
    ));
    let scene = Scene::of(&mut chart);
    assert!(scene.polylines(css(INK)).is_empty());
    assert!(scene.fills().is_empty());
}

#[test]
fn label_placements_follow_the_alignment_options() {
    let mut chart = chart();
    let id = add(
        &mut chart,
        DrawingKind::FibRetracement,
        vec![p(10.0, 101.0), p(20.0, 105.0)],
        "{}",
    );
    let (a, b) = (anchor(&chart, id, 0), anchor(&chart, id, 1));
    let y = y_of(&chart, 103.0);
    for (h, v, check) in [
        ("right", "middle", (b.0 + 6.0, 0.0)),
        ("center", "top", ((a.0 + b.0) / 2.0, -1.0)),
        ("left", "bottom", (a.0 - 6.0, 1.0)),
    ] {
        assert!(chart.drawing_apply_options(
            id,
            &format!(
                r#"{{"tool_options":{{"fibonacci":{{"label_h_align":"{h}","label_v_align":"{v}"}}}}}}"#
            )
        ));
        let (x, label_y, _) = Scene::of(&mut chart).text("0.5 (103.00)").unwrap();
        match h {
            "right" => assert!((x - check.0).abs() < 1e-3),
            "center" => assert!(x < check.0 && x > a.0),
            _ => assert!(x < check.0),
        }
        let side = if (label_y - y).abs() < 0.01 {
            0.0
        } else {
            (label_y - y).signum()
        };
        assert_eq!(side, check.1, "{h}/{v}");
    }
}

#[test]
fn extension_projects_the_first_leg_from_the_third_anchor() {
    let mut chart = chart();
    let id = add(
        &mut chart,
        DrawingKind::TrendBasedFibExtension,
        vec![p(10.0, 101.0), p(16.0, 104.0), p(20.0, 102.0)],
        r##"{"color":"#123456"}"##,
    );
    let (a, b, c) = (
        anchor(&chart, id, 0),
        anchor(&chart, id, 1),
        anchor(&chart, id, 2),
    );
    let scene = Scene::of(&mut chart);
    let span = (c.0.round() as i32, (c.0 + b.0 - a.0).round() as i32);
    for (value, color) in [(0.0, "#787b86"), (0.618, "#089981"), (1.0, "#787b86")] {
        let y = y_of(&chart, 102.0 + 3.0 * value).round() as i32;
        assert!(
            scene
                .hlines(css(color))
                .contains(&(y, span.0, span.1, LineStyle::Solid)),
            "level {value}: {:?}",
            scene.hlines(css(color))
        );
    }
    assert!(
        strokes_along(&scene.runs(css(INK)), &[a, b, c], 1.0),
        "the trend line joins all three anchors"
    );
    assert!(scene.text("1 (105.00)").is_some(), "{:?}", scene.texts());
    // Reverse swaps the projection's ends.
    assert!(chart.drawing_apply_options(id, r#"{"tool_options":{"fibonacci":{"reverse":true}}}"#));
    assert!(Scene::of(&mut chart).text("0 (105.00)").is_some());
}

#[test]
fn channel_levels_run_parallel_to_the_first_leg() {
    let mut chart = chart();
    let id = add(
        &mut chart,
        DrawingKind::FibChannel,
        vec![p(10.0, 102.0), p(20.0, 104.0), p(15.0, 101.0)],
        r#"{"fill_enabled":false}"#,
    );
    assert!(chart.drawing_apply_options(
        id,
        &format!(
            r#"{{"levels":{}}}"#,
            levels_json(&[(0.0, "#111111"), (0.5, "#222222"), (1.0, "#333333")])
        )
    ));
    let scene = Scene::of(&mut chart);
    // At the third anchor's time the first leg is at 103, so level 1 sits 2 lower and level 0.5
    // 1 lower, through both ends.
    for (color, shift) in [("#111111", 0.0), ("#222222", -1.0), ("#333333", -2.0)] {
        let line = &scene.polylines(css(color))[0].0;
        let start = (x_of(&chart, 10.0), y_of(&chart, 102.0 + shift));
        let end = (x_of(&chart, 20.0), y_of(&chart, 104.0 + shift));
        assert!(
            close(line[0], start, 1e-3) && close(line[1], end, 1e-3),
            "{color}: {line:?}"
        );
    }
    let (x, ..) = scene.text("0.5").unwrap();
    assert!(x < x_of(&chart, 10.0));
    // Extensions reach the pane edges along each level's own slope.
    assert!(chart.drawing_apply_options(id, r#"{"extend_left":true,"extend_right":true}"#));
    let line = &Scene::of(&mut chart).polylines(css("#222222"))[0].0;
    let (top, bottom) = (
        chart.panes[0].top,
        chart.panes[0].top + chart.panes[0].height,
    );
    let on_edge = |point: (f64, f64)| {
        point.0 < 0.5
            || point.0 > chart.pane_w - 0.5
            || point.1 < top + 0.5
            || point.1 > bottom - 0.5
    };
    assert!(on_edge(line[0]) && on_edge(line[1]), "{line:?}");
    assert!(line[0].0 < x_of(&chart, 10.0) && line[1].0 > x_of(&chart, 20.0));
    let slope =
        (y_of(&chart, 104.0) - y_of(&chart, 102.0)) / (x_of(&chart, 20.0) - x_of(&chart, 10.0));
    assert!(((line[1].1 - line[0].1) / (line[1].0 - line[0].0) - slope).abs() < 1e-6);
}

#[test]
fn time_zones_mark_fibonacci_multiples_of_the_anchor_distance() {
    let mut chart = chart();
    let id = add(
        &mut chart,
        DrawingKind::FibTimeZone,
        vec![p(4.0, 101.0), p(6.0, 103.0)],
        "{}",
    );
    let scene = Scene::of(&mut chart);
    let pane_w = chart.pane_w;
    let expected = [0.0, 1.0, 2.0, 3.0, 5.0, 8.0, 13.0, 21.0, 34.0]
        .iter()
        .map(|zone| x_of(&chart, 4.0 + 2.0 * zone))
        .filter(|&x| x <= pane_w)
        .map(|x| x.round() as i32)
        .collect::<Vec<_>>();
    let mut painted = super::PALETTE
        .iter()
        .flat_map(|color| scene.vlines(css(color)))
        .collect::<Vec<_>>();
    painted.sort_unstable();
    painted.dedup();
    let mut wanted = expected.clone();
    wanted.sort_unstable();
    assert_eq!(painted, wanted, "zones beyond the pane are not emitted");
    assert!(
        scene.fills().is_empty(),
        "time zones have no background by default"
    );
    // Labels sit right of each line at the pane bottom.
    let (x, y, _) = scene.text("13").unwrap();
    assert!((x - (x_of(&chart, 30.0) + 6.0)).abs() < 1e-3);
    assert!(y > chart.panes[0].top + chart.panes[0].height - 20.0);
    // Reverse projects backward from the first anchor; the background switch fills zones.
    assert!(chart.drawing_apply_options(
        id,
        r#"{"fill_enabled":true,"tool_options":{"fibonacci":{"reverse":true}}}"#
    ));
    let scene = Scene::of(&mut chart);
    assert!(scene.text("2").unwrap().0 < x_of(&chart, 4.0));
    assert!(!scene.fills().is_empty());
}

#[test]
fn trend_based_time_projects_the_first_leg_from_the_third_anchor() {
    let mut chart = chart();
    let id = add(
        &mut chart,
        DrawingKind::TrendBasedFibTime,
        vec![p(2.0, 101.0), p(6.0, 104.0), p(10.0, 102.0)],
        r##"{"color":"#123456"}"##,
    );
    let anchors = [0, 1, 2].map(|index| anchor(&chart, id, index));
    let scene = Scene::of(&mut chart);
    for (value, color) in [(0.0, "#787b86"), (0.618, "#089981"), (1.618, "#2962ff")] {
        let x = x_of(&chart, 10.0 + 4.0 * value).round() as i32;
        assert!(
            scene.vlines(css(color)).contains(&x),
            "{value}: {:?}",
            scene.vlines(css(color))
        );
    }
    assert!(strokes_along(&scene.runs(css(INK)), &anchors, 1.0));
    assert!(!scene.fills().is_empty());
}

#[test]
fn speed_resistance_fans_cast_price_and_time_rays_with_a_grid() {
    let mut chart = chart();
    let id = add(
        &mut chart,
        DrawingKind::FibSpeedResistanceFan,
        vec![p(10.0, 101.0), p(18.0, 105.0)],
        r##"{"color":"#123456"}"##,
    );
    let (a, b) = (anchor(&chart, id, 0), anchor(&chart, id, 1));
    let scene = Scene::of(&mut chart);
    // The 0.5 rays: through half the box height at the second anchor's time, and through half
    // its width at the second anchor's price, each to the pane edge.
    let rays = scene.polylines(css("#4caf50"));
    assert_eq!(rays.len(), 2);
    let price = (b.0, b.1 + (a.1 - b.1) * 0.5);
    let time = (b.0 + (a.0 - b.0) * 0.5, b.1);
    for (ray, through) in rays.iter().zip([price, time]) {
        assert!(close(ray.0[0], a, 1e-3));
        let (end, cross) = (
            ray.0[1],
            (through.0 - a.0) * (ray.0[1].1 - a.1) - (through.1 - a.1) * (ray.0[1].0 - a.0),
        );
        assert!(cross.abs() < 1e-6 * (end.0 - a.0).hypot(end.1 - a.1) * 100.0);
        let edge = end.0 >= chart.pane_w - 0.5 || end.1 <= chart.panes[0].top + 0.5;
        assert!(edge, "ray reaches the pane edge: {end:?}");
    }
    // Level 0 of both fans is one ray through the second anchor; level 1 casts the horizontal
    // price ray and the vertical time ray.
    let neutral = scene.polylines(css("#787b86"));
    assert_eq!(neutral.len(), 3, "level 0 once plus level 1's two rays");
    assert!(neutral.iter().any(|(ray, _)| {
        let cross = (b.0 - a.0) * (ray[1].1 - a.1) - (b.1 - a.1) * (ray[1].0 - a.0);
        cross.abs() < 1e-3 * (ray[1].0 - a.0).hypot(ray[1].1 - a.1) * (b.0 - a.0).hypot(b.1 - a.1)
    }));
    assert!(neutral.iter().any(|(ray, _)| (ray[1].1 - a.1).abs() < 1e-3));
    assert!(neutral.iter().any(|(ray, _)| (ray[1].0 - a.0).abs() < 1e-3));
    // The grid is the drawing stroke: seven horizontal and seven vertical lines in the box.
    assert_eq!(scene.hlines(css(INK)).len(), 7);
    assert_eq!(scene.vlines(css(INK)).len(), 7);
    assert_eq!(scene.fills().len(), 12, "six price and six time bands");
    assert!(chart.drawing_apply_options(id, r#"{"tool_options":{"fibonacci":{"grid":false}}}"#));
    assert!(Scene::of(&mut chart).hlines(css(INK)).is_empty());
}

#[test]
fn arcs_and_circles_are_concentric_ratio_curves() {
    let mut chart = chart();
    let arcs = add(
        &mut chart,
        DrawingKind::FibSpeedResistanceArcs,
        vec![p(10.0, 101.0), p(14.0, 104.0)],
        "{}",
    );
    let (a, b) = (anchor(&chart, arcs, 0), anchor(&chart, arcs, 1));
    let unit = (b.0 - a.0).hypot(b.1 - a.1);
    let scene = Scene::of(&mut chart);
    let arc = &scene.polylines(css("#4caf50"))[0].0;
    assert!(
        arc.iter().all(|point| {
            ((point.0 - a.0).hypot(point.1 - a.1) - unit * 0.5).abs() < 1e-3
                && point.1 <= a.1 + 1e-3
        }),
        "the half toward the second anchor, radius 0.5 × the anchor distance"
    );
    let level_one = &scene.polylines(css("#787b86"))[0].0;
    assert!(
        aeris_charts_render::shape::distance_to_polyline(b, level_one) <= 0.25 + 1e-6,
        "level 1 passes through the second anchor"
    );
    assert!(scene.text("0.5").is_some());
    assert!(chart.drawing_apply_options(
        arcs,
        r#"{"tool_options":{"fibonacci":{"full_circles":true}}}"#
    ));
    let arc = &Scene::of(&mut chart).polylines(css("#4caf50"))[0].0;
    assert!(arc.iter().any(|point| point.1 > a.1 + unit * 0.4));
    chart.remove_drawing(arcs);

    let circles = add(
        &mut chart,
        DrawingKind::FibCircles,
        vec![p(10.0, 101.0), p(14.0, 104.0)],
        "{}",
    );
    let (a, b) = (anchor(&chart, circles, 0), anchor(&chart, circles, 1));
    let center = ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0);
    let radius = (b.0 - a.0).hypot(b.1 - a.1) / 2.0;
    let scene = Scene::of(&mut chart);
    let one = &scene.polylines(css("#787b86"))[0].0;
    assert!(one
        .iter()
        .all(|point| ((point.0 - center.0).hypot(point.1 - center.1) - radius).abs() < 1e-3));
    assert!(close(one[0], *one.last().unwrap(), 1e-6), "full circles");
    // Annulus bands pair points on neighbouring circles.
    let band = scene
        .fills()
        .into_iter()
        .find(|(.., fill)| *fill == Color::rgba(0x08, 0x99, 0x81, 51))
        .expect("0.5 → 0.618 annulus");
    assert_eq!(band.0.len(), band.1.len());
    assert!(((band.0[3].0 - center.0).hypot(band.0[3].1 - center.1) - radius * 0.618).abs() < 1e-3);
    assert!(((band.1[3].0 - center.0).hypot(band.1[3].1 - center.1) - radius * 0.5).abs() < 1e-3);
}

#[test]
fn visible_arcs_keep_the_part_of_an_arc_the_pane_subtends() {
    use aeris_charts_render::shape::Rect;
    use std::f64::consts::{FRAC_PI_2, PI, TAU};
    let pane = Rect {
        left: 0.0,
        top: 0.0,
        right: 100.0,
        bottom: 50.0,
    };
    let near = |got: Option<(f64, f64)>, want: (f64, f64)| {
        got.is_some_and(|got| (got.0 - want.0).abs() < 1e-9 && (got.1 - want.1).abs() < 1e-9)
    };
    // A center inside the pane keeps the whole arc.
    assert_eq!(
        visible_arc(pane, (50.0, 25.0), (0.3, TAU)),
        Some((0.3, TAU))
    );
    // From straight below, the pane spans ±atan(50 / 950) around straight up.
    let spread = (50.0_f64 / 950.0).atan();
    let below = (50.0, 1_000.0);
    assert!(near(
        visible_arc(pane, below, (0.0, TAU)),
        (-FRAC_PI_2 - spread, 2.0 * spread)
    ));
    // The upper half (π → 2π) meets the window one turn up; the lower half misses it.
    assert!(near(
        visible_arc(pane, below, (PI, PI)),
        (3.0 * FRAC_PI_2 - spread, 2.0 * spread)
    ));
    assert_eq!(visible_arc(pane, below, (0.0, PI)), None);
    // A negative sweep keeps its direction.
    assert!(near(
        visible_arc(pane, below, (0.0, -PI)),
        (-FRAC_PI_2 + spread, -2.0 * spread)
    ));
}

#[test]
fn circles_far_beyond_the_pane_stay_within_the_curve_tolerance() {
    let mut chart = chart();
    // The center sits about 8,000 px above the pane.
    let id = add(
        &mut chart,
        DrawingKind::FibCircles,
        vec![p(10.0, 200.0), p(12.0, 300.0)],
        r#"{"fill_enabled":false}"#,
    );
    let (a, b) = (anchor(&chart, id, 0), anchor(&chart, id, 1));
    let center = ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0);
    let unit = (b.0 - a.0).hypot(b.1 - a.1) / 2.0;
    let (top, bottom) = (
        chart.panes[0].top,
        chart.panes[0].top + chart.panes[0].height,
    );
    assert!(top - center.1 > 5_000.0, "{center:?}");
    // Two circles that cross the pane 100 and 300 px below its top.
    let radii = [top - center.1 + 100.0, top - center.1 + 300.0];
    let levels = levels_json(&[(radii[0] / unit, "#111111"), (radii[1] / unit, "#222222")]);
    assert!(chart.drawing_apply_options(id, &format!(r#"{{"levels":{levels}}}"#)));
    let scene = Scene::of(&mut chart);
    for (radius, color) in radii.into_iter().zip(["#111111", "#222222"]) {
        let arcs = scene.polylines(css(color));
        assert_eq!(arcs.len(), 1, "{radius}");
        let points = &arcs[0].0;
        assert!(points.len() <= 257, "bounded: {}", points.len());
        // A whole circle at the capped segment count would sag about 0.6 px per chord.
        let sag = points
            .windows(2)
            .map(|pair| {
                let middle = ((pair[0].0 + pair[1].0) / 2.0, (pair[0].1 + pair[1].1) / 2.0);
                radius - (middle.0 - center.0).hypot(middle.1 - center.1)
            })
            .fold(0.0_f64, f64::max);
        assert!(sag <= 0.25 + 0.02, "{radius}: chords sag {sag} px");
        // The visible window runs edge to edge: the ends sit outside or on the pane boundary.
        let outside = |point: (f64, f64)| {
            point.0 <= 0.01 || point.0 >= chart.pane_w - 0.01 || point.1 <= top + 0.01
        };
        assert!(outside(points[0]) && outside(*points.last().unwrap()));
        assert!(points.iter().any(|point| point.1 > top && point.1 < bottom));
    }
    // It still hits along the visible arc.
    let y = center.1 + radii[0];
    assert_eq!(
        chart.hit_test_drawing(center.0, y).map(|hit| hit.id),
        Some(id)
    );
}

#[test]
fn the_golden_ratio_satisfies_its_defining_equation() {
    // φ is computed rather than written as a literal; it is the positive root of φ² = φ + 1, and
    // the spiral tests below take it as their expected growth per quarter turn.
    let phi = golden_ratio();
    assert!((phi * phi - phi - 1.0).abs() < 1e-15, "{phi}");
    assert!(phi > 1.0 && (phi - 1.618).abs() < 1e-3, "{phi}");
}

#[test]
fn spirals_grow_by_phi_every_quarter_turn_through_the_second_anchor() {
    let mut chart = chart();
    let id = add(
        &mut chart,
        DrawingKind::FibSpiral,
        vec![p(20.0, 103.0), p(22.0, 103.0)],
        r##"{"color":"#123456"}"##,
    );
    let (a, b) = (anchor(&chart, id, 0), anchor(&chart, id, 1));
    let r0 = (b.0 - a.0).hypot(b.1 - a.1);
    let scene = Scene::of(&mut chart);
    let spiral = scene
        .polylines(css(INK))
        .into_iter()
        .find(|(_, style)| *style == LineStyle::Solid)
        .expect("the spiral stroke")
        .0;
    assert!(
        aeris_charts_render::shape::distance_to_polyline(b, &spiral) < 0.5,
        "through the second anchor"
    );
    // Every chord follows r = r0 · φ^(turn / 90°): the radius ratio of neighbouring points
    // matches the angle between them, which turns clockwise on screen (y down).
    let growth = |spiral: &[(f64, f64)], clockwise: bool| {
        let radius = |point: (f64, f64)| (point.0 - a.0).hypot(point.1 - a.1);
        let angle = |point: (f64, f64)| (point.1 - a.1).atan2(point.0 - a.0);
        let pi = std::f64::consts::PI;
        spiral.windows(2).all(|pair| {
            let turn = (angle(pair[1]) - angle(pair[0]) + pi).rem_euclid(2.0 * pi) - pi;
            let expected = golden_ratio().powf(turn.abs() / std::f64::consts::FRAC_PI_2);
            // Frame points are f32, so the innermost sub-pixel turns are too coarse to compare.
            radius(pair[0]) < 2.0
                || ((turn > 0.0) == clockwise
                    && (radius(pair[1]) / radius(pair[0]) / expected - 1.0).abs() < 1e-3)
        })
    };
    assert!(growth(&spiral, true));
    // Strokes are clipped to the pane grown by the stroke's reach (`push_clipped_stroke`), so the
    // spiral grows until it leaves the pane: its painted points stay inside that reach and at
    // least one of them reaches the pane border.
    let (top, bottom) = (
        chart.panes[0].top,
        chart.panes[0].top + chart.panes[0].height,
    );
    let reach = 8.0;
    assert!(
        spiral.iter().all(|point| point.0 >= -reach
            && point.0 <= chart.pane_w + reach
            && point.1 >= top - reach
            && point.1 <= bottom + reach),
        "painted points stay inside the pane plus the stroke reach"
    );
    let border_distance = spiral
        .iter()
        .map(|point| {
            point
                .0
                .min(chart.pane_w - point.0)
                .min(point.1 - top)
                .min(bottom - point.1)
        })
        .fold(f64::INFINITY, f64::min);
    assert!(
        border_distance <= 1.0,
        "it grows until it leaves the pane: nearest border distance {border_distance}"
    );
    assert!(
        spiral.len() < 4_000,
        "bounded tessellation: {}",
        spiral.len()
    );
    assert!(r0 > 0.0);
    // Counterclockwise with reverse.
    assert!(chart.drawing_apply_options(id, r#"{"tool_options":{"fibonacci":{"reverse":true}}}"#));
    let spiral = Scene::of(&mut chart)
        .polylines(css(INK))
        .into_iter()
        .find(|(_, style)| *style == LineStyle::Solid)
        .unwrap()
        .0;
    assert!(growth(&spiral, false));
}

#[test]
fn wedges_span_ratio_arcs_between_their_edges() {
    let mut chart = chart();
    let id = add(
        &mut chart,
        DrawingKind::FibWedge,
        vec![p(10.0, 102.0), p(18.0, 105.0), p(18.0, 101.0)],
        r##"{"color":"#123456"}"##,
    );
    let (a, b, c) = (
        anchor(&chart, id, 0),
        anchor(&chart, id, 1),
        anchor(&chart, id, 2),
    );
    let unit = (b.0 - a.0).hypot(b.1 - a.1);
    let scene = Scene::of(&mut chart);
    let arc = &scene.polylines(css("#089981"))[0].0;
    assert!(arc
        .iter()
        .all(|point| ((point.0 - a.0).hypot(point.1 - a.1) - unit * 0.618).abs() < 1e-3));
    let (start, end) = (arc[0], *arc.last().unwrap());
    let direction = |point: (f64, f64), toward: (f64, f64)| {
        let (u, v) = (
            (point.0 - a.0, point.1 - a.1),
            (toward.0 - a.0, toward.1 - a.1),
        );
        (u.0 * v.1 - u.1 * v.0).abs() / (u.0.hypot(u.1) * v.0.hypot(v.1))
    };
    assert!(
        direction(start, b) < 1e-6 && direction(end, c) < 1e-6,
        "arcs run edge to edge"
    );
    let edges = scene.polylines(css(INK));
    assert_eq!(edges.len(), 2);
    assert!(edges.iter().all(|edge| close(edge.0[0], a, 1e-3)));
    assert_eq!(scene.fills().len(), 5);
}

#[test]
fn hit_testing_follows_levels_labels_and_selected_bands() {
    let mut chart = chart();
    let id = add(
        &mut chart,
        DrawingKind::FibRetracement,
        vec![p(10.0, 101.0), p(20.0, 105.0)],
        "{}",
    );
    let (a, b) = (anchor(&chart, id, 0), anchor(&chart, id, 1));
    let x = a.0 + (b.0 - a.0) * 0.3;
    let level = y_of(&chart, 103.0);
    assert_eq!(
        chart.hit_test_drawing(x, level + 3.0).map(|hit| hit.id),
        Some(id)
    );
    let band = (level + y_of(&chart, 102.528)) / 2.0;
    assert_eq!(
        chart.hit_test_drawing(x, band),
        None,
        "bands pan the chart while unselected"
    );
    chart.set_selected_drawing(Some(id));
    assert_eq!(
        chart.hit_test_drawing(x, band).map(|hit| hit.part),
        Some(DrawingDragPart::Body),
        "and drag the drawing once selected"
    );
    chart.set_selected_drawing(None);
    let (label_x, label_y, _) = Scene::of(&mut chart).text("0.5 (103.00)").unwrap();
    assert_eq!(
        chart
            .hit_test_drawing(label_x + 4.0, label_y)
            .map(|hit| hit.id),
        Some(id),
        "labels are body targets"
    );
    assert_eq!(chart.hit_test_drawing(b.0 + 60.0, level), None);

    // Time zones hit far from their anchors along the full-height lines; fans along their rays.
    let zones = add(
        &mut chart,
        DrawingKind::FibTimeZone,
        vec![p(24.0, 101.0), p(26.0, 101.0)],
        "{}",
    );
    let zone = x_of(&chart, 30.0);
    assert_eq!(
        chart
            .hit_test_drawing(zone + 1.0, chart.panes[0].top + 20.0)
            .map(|hit| hit.id),
        Some(zones)
    );
    let fan = add(
        &mut chart,
        DrawingKind::FibSpeedResistanceFan,
        vec![p(2.0, 100.5), p(4.0, 101.0)],
        "{}",
    );
    let (fa, fb) = (anchor(&chart, fan, 0), anchor(&chart, fan, 1));
    let far = fa.0 + (fb.0 - fa.0) * 3.0;
    let on_ray = (far, fa.1 + (fb.1 - fa.1) * 3.0);
    assert_eq!(
        chart.hit_test_drawing(on_ray.0, on_ray.1).map(|hit| hit.id),
        Some(fan)
    );
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

fn viewport_candidate(chart: &ChartEngine, id: DrawingId) -> bool {
    let candidates = chart.take_drawing_candidates(0, None);
    let found = candidates.contains(&id);
    chart.recycle_drawing_candidates(candidates);
    found
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
fn culling_bounds_cover_levels_beyond_the_anchors() {
    let mut chart = chart();
    crowd(&mut chart);
    // Anchors far above the pane; the 4.236 extension level lands inside it.
    let high = add(
        &mut chart,
        DrawingKind::FibRetracement,
        vec![p(10.0, 150.0), p(20.0, 162.0)],
        r#"{"fill_enabled":false}"#,
    );
    // 162 + (150 - 162) × 4.236 = 111.168: still above; pick a level list that lands inside.
    assert!(chart.drawing_apply_options(
        high,
        &format!(
            r#"{{"levels":{}}}"#,
            levels_json(&[(0.0, "#111111"), (5.0, "#222222")])
        )
    ));
    // Level 5: 162 - 12 × 5 = 102.
    let y = y_of(&chart, 102.0);
    assert!(y > chart.panes[0].top && y < chart.panes[0].top + chart.panes[0].height);
    chart.build_frame();
    assert!(
        viewport_candidate(&chart, high),
        "the level inside the pane keeps it a candidate"
    );
    assert_eq!(
        chart
            .hit_test_drawing(x_of(&chart, 15.0), y)
            .map(|hit| hit.id),
        Some(high)
    );
    assert!(Scene::of(&mut chart)
        .hlines(css("#222222"))
        .iter()
        .any(|line| line.0 == y.round() as i32));

    // Time zones anchored left of the viewport still paint the zones that reach it.
    let zones = add(
        &mut chart,
        DrawingKind::FibTimeZone,
        vec![p(-30.0, 101.0), p(-27.0, 101.0)],
        "{}",
    );
    chart.build_frame();
    assert!(viewport_candidate(&chart, zones));
    let x = x_of(&chart, -30.0 + 3.0 * 13.0);
    assert_eq!(
        chart
            .hit_test_drawing(x, chart.panes[0].top + 30.0)
            .map(|hit| hit.id),
        Some(zones)
    );

    // A retracement entirely left of the viewport culls; its extension brings it back.
    let away = add(
        &mut chart,
        DrawingKind::FibRetracement,
        vec![p(-40.0, 101.0), p(-30.0, 105.0)],
        "{}",
    );
    chart.build_frame();
    assert!(!viewport_candidate(&chart, away));
    assert!(chart.drawing_apply_options(away, r#"{"extend_right":true}"#));
    chart.build_frame();
    assert!(viewport_candidate(&chart, away));
    assert_eq!(
        chart
            .hit_test_drawing(400.0, y_of(&chart, 103.0))
            .map(|hit| hit.id),
        Some(away)
    );
}

#[test]
fn ring_tools_cull_and_hit_test_by_their_paint_box() {
    let mut chart = chart();
    crowd(&mut chart);
    let rings = [
        DrawingKind::FibSpeedResistanceArcs,
        DrawingKind::FibCircles,
        DrawingKind::FibWedge,
    ];
    // Thousands of px left of the pane, far beyond the largest ring's radius.
    let far = rings.map(|kind| {
        let points = anchors_for(kind)
            .into_iter()
            .map(|point| p(point.logical - 300.0, point.price))
            .collect();
        add(&mut chart, kind, points, "{}")
    });
    chart.build_frame();
    for (kind, id) in rings.iter().zip(far) {
        assert!(!viewport_candidate(&chart, id), "{kind:?} culls");
    }
    chart.reset_drawing_work_stats();
    assert_eq!(chart.hit_test_drawing(50.0, 50.0), None);
    assert_eq!(
        chart.drawing_work_stats().precise_hit_tests,
        0,
        "no far ring is tested precisely"
    );

    // Centered left of the pane, the outer rings still reach into it: they paint and hit.
    let reaching = add(
        &mut chart,
        DrawingKind::FibCircles,
        vec![p(-14.0, 103.0), p(-2.0, 103.0)],
        &format!(r#"{{"levels":{}}}"#, levels_json(&[(4.236, "#abcdef")])),
    );
    chart.build_frame();
    assert!(viewport_candidate(&chart, reaching));
    let (a, b) = (anchor(&chart, reaching, 0), anchor(&chart, reaching, 1));
    let radius = 4.236 * (b.0 - a.0) / 2.0;
    let rim = ((a.0 + b.0) / 2.0 + radius, a.1);
    assert!(rim.0 > 0.0 && rim.0 < chart.pane_w);
    assert_eq!(
        chart.hit_test_drawing(rim.0, rim.1).map(|hit| hit.id),
        Some(reaching)
    );
    assert!(!Scene::of(&mut chart).runs(css("#abcdef")).is_empty());

    // A ring just below the pane whose large label reaches into it keeps the drawing painted.
    let bottom = chart.panes[0].top + chart.panes[0].height;
    let (x, unit) = (400.0, 60.0);
    let center_y = bottom + unit + 30.0;
    let ends = vec![
        at(&chart, x - unit, center_y),
        at(&chart, x + unit, center_y),
    ];
    let labeled = add(
        &mut chart,
        DrawingKind::FibCircles,
        ends,
        &format!(
            r#"{{"text_size":40,"tool_options":{{"fibonacci":{{"trend_line":false}}}},"levels":{}}}"#,
            levels_json(&[(1.0, "#fedcba")])
        ),
    );
    chart.build_frame();
    assert!(
        viewport_candidate(&chart, labeled),
        "its label pads the box"
    );
    assert!(Scene::of(&mut chart)
        .texts()
        .iter()
        .any(|(.., color)| *color == css("#fedcba")));
}

#[test]
fn drags_nudges_and_straighten_edit_fibonacci_tools_as_single_history_entries() {
    let mut chart = chart();
    let id = add(
        &mut chart,
        DrawingKind::FibRetracement,
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
    assert_eq!(chart.drawing(id).unwrap().points[0], before[0]);
    let angle = (ay - ny).atan2(nx - ax).to_degrees();
    assert!((angle / 45.0 - (angle / 45.0).round()).abs() < 1e-6);
    assert!(chart.undo_drawing());
    assert_eq!(chart.drawing(id).unwrap().points, before);

    // A body drag on a level line moves every anchor of a three-anchor tool.
    let extension = add(
        &mut chart,
        DrawingKind::TrendBasedFibExtension,
        vec![p(24.0, 101.0), p(28.0, 104.0), p(32.0, 102.0)],
        "{}",
    );
    let original = chart.drawing(extension).unwrap().points.clone();
    let y = y_of(&chart, 102.0 + 3.0 * 0.5);
    let x = x_of(&chart, 33.0);
    assert!(chart.drawing_drag_start_at(x, y));
    chart.drawing_drag_to(x + 20.0, y - 15.0, DrawingModifiers::default());
    chart.drawing_drag_end();
    let moved = chart.drawing(extension).unwrap().points.clone();
    for (index, point) in moved.iter().enumerate() {
        assert!(point.logical > original[index].logical && point.price > original[index].price);
        let (px, py) = anchor(&chart, extension, index);
        let before = chart
            .drawing_to_px_for(0, DrawingPriceScale::Right, original[index])
            .unwrap();
        assert!(close((px, py), (before.0 + 20.0, before.1 - 15.0), 1e-6));
    }
    assert!(chart.undo_drawing());
    assert_eq!(chart.drawing(extension).unwrap().points, original);

    // Keyboard handles: every anchor.
    assert_eq!(chart.drawing_handle_count(id), Some(2));
    assert_eq!(chart.drawing_handle_count(extension), Some(3));
    chart.set_selected_drawing(Some(extension));
    let (x0, y0) = anchor(&chart, extension, 2);
    assert!(chart.nudge_selected_drawing(0.0, -10.0, Some(2)));
    let (x1, y1) = anchor(&chart, extension, 2);
    assert!((x1 - x0).abs() < 1e-6 && (y1 - y0 + 10.0).abs() < 1e-6);
    assert!(chart.undo_drawing());
    assert_eq!(chart.drawing(extension).unwrap().points, original);

    // Locked drawings stay selectable but never drag; hidden ones neither paint nor hit.
    assert!(chart.set_drawing_locked(extension, true));
    assert!(!chart.drawing_drag_start_at(x, y));
    assert_eq!(chart.selected_drawing(), Some(extension));
    assert!(chart.set_drawing_visibility(extension, false));
    assert_eq!(chart.hit_test_drawing(x, y), None);
    assert!(Scene::of(&mut chart).text("0.5 (103.50)").is_none());
    // Z-order: the retracement moves above the extension.
    assert!(chart.set_drawing_visibility(extension, true));
    assert!(chart.move_drawing_z_order(id, 1));
    assert_eq!(chart.drawings().last().unwrap().id, id);
}

#[test]
fn chart_magnet_snaps_fibonacci_placement_to_bar_values() {
    let mut chart = chart();
    chart.set_drawing_magnet_mode(DrawingMagnetMode::Strong);
    assert!(chart.set_drawing_tool(Some(DrawingKind::FibRetracement), None, None));
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
fn fibonacci_anchors_resolve_by_time_across_an_interval_switch() {
    let mut chart = chart();
    let at = |time: f64, price: f64| DrawingAnchor {
        logical: None,
        price,
        time: Some(time),
    };
    let id = chart
        .add_drawing_anchors(
            DrawingKind::TrendBasedFibTime,
            0,
            &[
                at(4.0 * HOUR, 101.0),
                at(8.0 * HOUR, 104.0),
                at(10.5 * HOUR, 102.0),
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
    chart.fit_content();
    let anchors = chart.drawing_anchors(id).unwrap();
    assert_eq!(
        anchors
            .iter()
            .map(|anchor| anchor.logical)
            .collect::<Vec<_>>(),
        [Some(8.0), Some(16.0), Some(21.0)]
    );
    assert_eq!(anchors[2].time, Some(10.5 * HOUR));
    // Time levels follow: level 1 lands a first-leg duration (4h = 8 bars) after the third.
    let x = x_of(&chart, 29.0).round() as i32;
    assert!(Scene::of(&mut chart).vlines(css("#787b86")).contains(&x));
}

#[test]
fn schema_kind_options_and_tool_option_patches_are_typed_and_atomic() {
    let names = |kind: DrawingKind| {
        crate::drawing_property_schema(kind)
            .properties
            .into_iter()
            .filter(|property| property.name.starts_with("tool_options."))
            .map(|property| (property.name, property.default, property.enum_values))
            .collect::<Vec<_>>()
    };
    let retracement = names(DrawingKind::FibRetracement);
    assert_eq!(
        retracement
            .iter()
            .map(|(name, ..)| name.trim_start_matches("tool_options.fibonacci."))
            .collect::<Vec<_>>(),
        [
            "reverse",
            "show_levels",
            "show_prices",
            "levels_as_percent",
            "log_scale",
            "trend_line",
            "label_h_align",
            "label_v_align"
        ]
    );
    let (_, default, values) = &retracement[6];
    assert_eq!(
        (default, values.as_slice()),
        (
            &serde_json::json!("left"),
            ["left", "center", "right"].map(String::from).as_slice()
        )
    );
    let zones = names(DrawingKind::FibTimeZone);
    assert!(zones
        .iter()
        .any(|(name, default, _)| name.ends_with("label_v_align") && default == "bottom"));
    assert!(names(DrawingKind::FibSpiral)
        .iter()
        .all(|(name, ..)| !name.ends_with("show_levels")));
    for kind in FIB_KINDS {
        assert!(!names(kind).is_empty(), "{kind:?} lists its options");
        let schema = crate::drawing_property_schema(kind);
        let levels = schema
            .properties
            .iter()
            .find(|property| property.name == "levels")
            .unwrap();
        let template = crate::Drawing::new(0, kind, 0, Vec::new());
        assert_eq!(
            levels.default,
            serde_json::to_value(&template.levels).unwrap()
        );
    }

    let mut chart = chart();
    let id = add(
        &mut chart,
        DrawingKind::FibTimeZone,
        anchors_for(DrawingKind::FibTimeZone),
        "{}",
    );
    let kind_options = |chart: &ChartEngine| {
        serde_json::from_str::<serde_json::Value>(&chart.drawing_kind_options_json(id).unwrap())
            .unwrap()
    };
    assert_eq!(
        kind_options(&chart),
        serde_json::json!({
            "kind": "fibonacci",
            "reverse": false,
            "show_levels": true,
            "show_prices": true,
            "levels_as_percent": false,
            "log_scale": false,
            "trend_line": true,
            "grid": true,
            "full_circles": false,
            "label_h_align": "right",
            "label_v_align": "bottom",
        })
    );
    assert!(chart.drawing_apply_options(
        id,
        r#"{"tool_options":{"fibonacci":{"label_v_align":"top"}},"width":2}"#
    ));
    let options = chart.drawing(id).unwrap().tool_options.fibonacci.unwrap();
    assert_eq!(options.label_v_align, Some(FibonacciLabelVAlign::Top));
    assert_eq!(
        options.label_h_align, None,
        "absent keys keep the kind default"
    );
    assert_eq!(kind_options(&chart)["label_h_align"], "right");
    let before = chart.drawing(id).unwrap().clone();
    for invalid in [
        r#"{"tool_options":{"fibonacci":{"label_v_align":"sideways"}},"width":9}"#,
        r#"{"tool_options":{"fibonacci":{"reverse":1}},"width":9}"#,
        r#"{"tool_options":{"fibonacci":7}}"#,
        r#"{"levels":[{"value":1}],"width":9}"#,
    ] {
        assert!(!chart.drawing_apply_options(id, invalid), "{invalid}");
        assert_eq!(chart.drawing(id).unwrap(), &before);
    }
    let too_many = levels_json(&vec![(0.5, "#111111"); crate::MAX_DRAWING_LEVELS + 1]);
    assert!(!chart.drawing_apply_options(id, &format!(r#"{{"levels":{too_many}}}"#)));
    assert!(chart.drawing_apply_options(id, r#"{"tool_options":{"fibonacci":null}}"#));
    assert!(chart.drawing(id).unwrap().tool_options.is_empty());
    assert!(chart.undo_drawing());
    assert_eq!(chart.drawing(id).unwrap().tool_options, before.tool_options);
}

#[test]
fn level_list_patches_edit_values_visibility_colors_styles_and_fills() {
    let mut chart = chart();
    let id = add(
        &mut chart,
        DrawingKind::FibRetracement,
        vec![p(10.0, 101.0), p(20.0, 105.0)],
        "{}",
    );
    let (a, b) = (anchor(&chart, id, 0), anchor(&chart, id, 1));
    let levels = serde_json::json!([
        {"value": 0.0, "color": "#111111", "visible": true, "style": "solid", "fill_between": false, "label_visible": true},
        {"value": 0.25, "color": "#222222", "visible": true, "style": "dashed", "fill_between": true, "fill_color": "rgba(1, 2, 3, 0.5)", "label_visible": false},
        {"value": 0.75, "color": "#333333", "visible": false, "style": "solid", "fill_between": true, "label_visible": true},
        {"value": 1.0, "color": "#444444", "visible": true, "style": "dotted", "fill_between": true, "label_visible": true},
    ]);
    assert!(chart.drawing_apply_options(id, &serde_json::json!({ "levels": levels }).to_string()));
    let scene = Scene::of(&mut chart);
    let quarter = y_of(&chart, 104.0).round() as i32;
    assert_eq!(
        scene.hlines(css("#222222")),
        [(
            quarter,
            a.0.round() as i32,
            b.0.round() as i32,
            LineStyle::Dashed
        )]
    );
    assert!(
        scene.hlines(css("#333333")).is_empty(),
        "hidden levels paint nothing"
    );
    assert_eq!(scene.hlines(css("#444444"))[0].3, LineStyle::Dotted);
    let fills = scene.fills();
    assert_eq!(
        fills.len(),
        2,
        "0 → 0.25 and 0.25 → 1 (skipping the hidden level)"
    );
    assert_eq!(fills[0].2, Color::rgba(1, 2, 3, 128), "explicit fill color");
    assert_eq!(fills[1].2, Color::rgba(0x44, 0x44, 0x44, 51));
    let texts = scene.texts();
    assert!(texts.iter().any(|(text, ..)| text == "0 (105.00)"));
    assert!(
        !texts.iter().any(|(text, ..)| text.starts_with("0.25")),
        "label hidden"
    );
    assert!(chart.undo_drawing());
    assert_eq!(chart.drawing(id).unwrap().levels.len(), 11);
}

#[test]
fn templates_carry_levels_and_tool_options() {
    let mut chart = chart();
    let source = add(
        &mut chart,
        DrawingKind::FibRetracement,
        anchors_for(DrawingKind::FibRetracement),
        &format!(
            r#"{{"levels":{},"tool_options":{{"fibonacci":{{"reverse":true}}}}}}"#,
            levels_json(&[(0.0, "#111111"), (0.5, "#222222")])
        ),
    );
    let template = chart.drawing_template_json(source, "mine").unwrap();
    let target = add(
        &mut chart,
        DrawingKind::FibRetracement,
        vec![p(24.0, 101.0), p(30.0, 104.0)],
        "{}",
    );
    assert!(chart.apply_drawing_template_json(target, &template));
    let (source, target) = (
        chart.drawing(source).unwrap(),
        chart.drawing(target).unwrap(),
    );
    assert_eq!(target.levels, source.levels);
    assert_eq!(target.tool_options, source.tool_options);
    // Templates are per kind.
    let zones = add(
        &mut chart,
        DrawingKind::FibTimeZone,
        anchors_for(DrawingKind::FibTimeZone),
        "{}",
    );
    assert!(!chart.apply_drawing_template_json(zones, &template));
    // A tool armed with options creates drawings from them.
    assert!(chart.set_drawing_tool(
        Some(DrawingKind::FibCircles),
        Some(r#"{"tool_options":{"fibonacci":{"levels_as_percent":true}}}"#),
        None
    ));
    chart.drawing_tool_activate(300.0, 200.0, DrawingModifiers::default());
    let id = chart
        .drawing_tool_activate(360.0, 240.0, DrawingModifiers::default())
        .created
        .unwrap();
    assert_eq!(
        chart.drawing(id).unwrap().tool_options.fibonacci,
        Some(FibonacciToolOptions {
            levels_as_percent: true,
            ..FibonacciToolOptions::default()
        })
    );
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
            DrawingKind::FibRetracement,
            r#"{"extend_right":true,"tool_options":{"fibonacci":{"reverse":true,"label_h_align":"right"}}}"#.to_string(),
        ),
        (DrawingKind::FibChannel, r#"{"levels":[]}"#.to_string()),
        (DrawingKind::FibTimeZone, r#"{"fill_enabled":true}"#.to_string()),
        (
            DrawingKind::FibSpeedResistanceFan,
            format!(r#"{{"levels":{}}}"#, levels_json(&[(0.5, "#abcdef")])),
        ),
        (
            DrawingKind::FibSpiral,
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
    assert_eq!(cleared.kind, DrawingKind::FibChannel);
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
        DrawingKind::TrendBasedFibExtension,
        anchors_for(DrawingKind::TrendBasedFibExtension),
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
    assert_eq!(drawing.kind, DrawingKind::TrendBasedFibExtension);
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
fn frames_scale_family_geometry_with_the_device_pixel_ratio() {
    // 1.5 makes the horizontal and vertical bitmap ratios differ (odd pane sizes round apart).
    for dpr in [1.0, 1.5, 2.0] {
        let mut chart = chart_with(&hourly(40), dpr);
        let id = add(
            &mut chart,
            DrawingKind::FibRetracement,
            vec![p(10.0, 101.0), p(20.0, 105.0)],
            r##"{"color":"#123456"}"##,
        );
        let (a, b) = (anchor(&chart, id, 0), anchor(&chart, id, 1));
        let hpr = (chart.pane_w * dpr).round() / chart.pane_w;
        let vpr = (chart.pane_h * dpr).round() / chart.pane_h;
        let scene = Scene::of(&mut chart);
        let y = (y_of(&chart, 103.0) * vpr).round() as i32;
        assert!(
            scene.hlines(css("#4caf50")).contains(&(
                y,
                (a.0 * hpr).round() as i32,
                (b.0 * hpr).round() as i32,
                LineStyle::Solid
            )),
            "dpr {dpr}: {:?}",
            scene.hlines(css("#4caf50"))
        );
        let path = [(a.0 * hpr, a.1 * vpr), (b.0 * hpr, b.1 * vpr)];
        assert!(strokes_along(&scene.runs(css(INK)), &path, dpr));
        let (x, ..) = scene.text("0.5 (103.00)").unwrap();
        assert!(x < (a.0 - 6.0) * hpr + 1e-3);
    }
    // Screen-space curves keep their radius ratio at a fractional ratio.
    let mut chart = ChartEngine::new(801.0, 500.0, 1.5);
    let times = hourly(40);
    let values = (0..times.len())
        .map(|index| 100.0 + (index % 7) as f64)
        .collect::<Vec<_>>();
    chart
        .set_series_data(0, &times, &values, &values, &values, &values)
        .unwrap();
    chart.time_scale.set_width(801.0);
    chart.fit_content();
    chart.build_frame();
    let id = add(
        &mut chart,
        DrawingKind::FibCircles,
        vec![p(10.0, 101.0), p(14.0, 103.0)],
        "{}",
    );
    let scene = Scene::of(&mut chart);
    assert!(!scene.polylines(css("#787b86")).is_empty());
    assert!(chart
        .hit_test_drawing(anchor(&chart, id, 1).0, anchor(&chart, id, 1).1)
        .is_some());
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
    fn no_parts(_: &super::PartContext<'_>, _: &mut super::DrawingParts) {}
    let family = super::DrawingFamily::new(no_parts, |_| crate::DrawingKindOptions::Generic);
    let drawing = crate::Drawing::new(1, DrawingKind::TrendLine, 0, vec![p(1.0, 2.0)]);
    assert_eq!((family.bounds)(&drawing), None);
    assert!(!family.partial_preview);
    let options: FibonacciToolOptions = serde_json::from_str("{}").unwrap();
    assert_eq!(options, FibonacciToolOptions::default());
    assert_eq!(options.label_h_align, None::<FibonacciLabelHAlign>);
}

/// The logical/price point at media px `(x, y)` on the first pane.
fn at(chart: &ChartEngine, x: f64, y: f64) -> DrawingPoint {
    chart
        .drawing_from_px_for(0, DrawingPriceScale::Right, x, y)
        .unwrap()
}

#[test]
fn extreme_level_values_keep_every_tool_finite_bounded_and_precise() {
    let mut chart = chart();
    let levels = levels_json(&[
        (0.5, "#111111"),
        (1.0, "#222222"),
        (1e30, "#333333"),
        (-1e30, "#444444"),
        (1e300, "#555555"),
        (-1e300, "#666666"),
        (1.7e308, "#777777"),
    ]);
    // Every emitted coordinate stays finite and within a pane's reach: far-off level lines, rays,
    // arcs, and labels are clipped or skipped instead of reaching 1e30 px or overflowing.
    let bound = 10.0 * chart.pane_w.max(chart.pane_h);
    let near = |x: f32, y: f32| {
        x.is_finite() && y.is_finite() && x.abs() < bound as f32 && y.abs() < bound as f32
    };
    for kind in FIB_KINDS {
        let id = add(
            &mut chart,
            kind,
            anchors_for(kind),
            &format!(r#"{{"levels":{levels}}}"#),
        );
        for selected in [None, Some(id)] {
            chart.set_selected_drawing(selected);
            let frame = chart.build_frame();
            let pane = &frame.panes[0];
            assert!(
                pane.points.iter().all(|point| near(point[0], point[1])),
                "{kind:?}: {:?}",
                pane.points
                    .iter()
                    .filter(|point| !near(point[0], point[1]))
                    .collect::<Vec<_>>()
            );
            for prim in &pane.main {
                if let Prim::Text { text, x, y, .. } = prim {
                    assert!(near(*x, *y), "{kind:?} label {text} at ({x}, {y})");
                }
            }
            chart.hit_test_drawing(400.0, 200.0);
        }
        // An overflowing radius must not coarsen the tessellation of the arcs that do show.
        if matches!(
            kind,
            DrawingKind::FibSpeedResistanceArcs | DrawingKind::FibCircles | DrawingKind::FibWedge
        ) {
            let scene = Scene::of(&mut chart);
            let arc = &scene.polylines(css("#222222"))[0].0;
            assert!(
                arc.len() > 8,
                "{kind:?} level 1 keeps its chords: {}",
                arc.len()
            );
        }
        chart.remove_drawing(id);
    }
}

#[test]
fn extended_channel_bands_cover_the_pane_corners_their_lines_leave_through() {
    let mut chart = chart();
    let (w, top) = (chart.pane_w, chart.panes[0].top);
    // Parallel lines of slope −½ px: level 0 leaves through the top edge 40 px left of the
    // right edge, level 0.236 (47 px lower) through the right edge.
    let base = |x: f64| top + (w - 40.0 - x) / 2.0;
    let points = vec![
        at(&chart, 100.0, base(100.0)),
        at(&chart, 300.0, base(300.0)),
        at(&chart, 300.0, base(300.0) + 200.0),
    ];
    let id = add(
        &mut chart,
        DrawingKind::FibChannel,
        points,
        r#"{"extend_right":true}"#,
    );
    chart.set_selected_drawing(Some(id));
    chart.build_frame();
    // The corner lies between the two lines, so the selected band covers it.
    assert_eq!(
        chart
            .hit_test_drawing(w - 2.0, top + 2.0)
            .map(|hit| (hit.id, hit.part)),
        Some((id, DrawingDragPart::Body))
    );
    let scene = Scene::of(&mut chart);
    let corner = (w - 2.0, top + 2.0);
    assert!(
        scene.fills().iter().any(|(upper, lower, _)| {
            aeris_charts_render::shape::point_in_ribbon(corner, upper, lower)
        }),
        "a painted band covers the corner"
    );
    // Without the extension the band closes at the second anchor's time.
    assert!(chart.drawing_apply_options(id, r#"{"extend_right":false}"#));
    chart.build_frame();
    assert_eq!(chart.hit_test_drawing(w - 2.0, top + 2.0), None);
    let inside = (200.0, base(200.0) + 20.0);
    assert_eq!(
        chart.hit_test_drawing(inside.0, inside.1).map(|hit| hit.id),
        Some(id)
    );
}

#[test]
fn labels_of_levels_just_outside_the_pane_still_paint() {
    let mut chart = chart();
    let top = chart.panes[0].top;
    // Level 0 sits 4 px above the pane; its label hangs below the line, inside the pane.
    let points = vec![at(&chart, 200.0, top + 200.0), at(&chart, 400.0, top - 4.0)];
    let id = add(
        &mut chart,
        DrawingKind::FibRetracement,
        points,
        r#"{"tool_options":{"fibonacci":{"label_v_align":"bottom"}}}"#,
    );
    let scene = Scene::of(&mut chart);
    let (_, y, _) = scene
        .texts()
        .into_iter()
        .find(|(text, ..)| text.starts_with("0 ("))
        .map(|(_, x, y, color)| (x, y, color))
        .unwrap_or_else(|| panic!("level 0's label: {:?}", scene.texts()));
    assert!(y > top, "{y}");
    assert!(
        scene
            .hlines(css("#787b86"))
            .iter()
            .all(|line| f64::from(line.0) > top - 2.0),
        "the line itself stays culled"
    );
    // Its label is a body target where it shows.
    assert_eq!(
        chart
            .hit_test_drawing(200.0 - 30.0, y + 2.0)
            .map(|hit| hit.id),
        Some(id)
    );
    chart.remove_drawing(id);

    // A time zone 3 px left of the pane labels its right side inside it.
    let points = vec![at(&chart, -3.0, top + 100.0), at(&chart, 97.0, top + 100.0)];
    let zones = add(&mut chart, DrawingKind::FibTimeZone, points, "{}");
    assert!(
        Scene::of(&mut chart).text("0").is_some(),
        "the zone at -3 px keeps its label"
    );
    assert!(chart.drawing(zones).is_some());
}

/// Dash runs of `color` whose points all lie inside the pane, translated by `shift`.
fn dashes_inside(chart: &mut ChartEngine, color: &str, shift: (f64, f64)) -> Vec<Vec<(f64, f64)>> {
    let (w, top, bottom) = (
        chart.pane_w,
        chart.panes[0].top,
        chart.panes[0].top + chart.panes[0].height,
    );
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

/// Whether every dash of `after` repeats one of `before` (same ends within `tolerance`).
fn dashes_stay_put(before: &[Vec<(f64, f64)>], after: &[Vec<(f64, f64)>], tolerance: f64) -> bool {
    !after.is_empty()
        && after.iter().all(|dash| {
            before.iter().any(|other| {
                close(dash[0], other[0], tolerance)
                    && close(*dash.last().unwrap(), *other.last().unwrap(), tolerance)
            })
        })
}

#[test]
fn dashed_curves_keep_their_dash_phase_while_the_pane_scrolls() {
    let dashed = |value: f64| {
        format!(
            r##"{{"value":{value},"color":"#111111","visible":true,"style":"dashed","fill_between":false,"label_visible":false}}"##
        )
    };
    for (kind, options) in [
        // A full circle centered left of the pane at its middle height: the visible window
        // straddles the angle where the circle's dash pattern restarts.
        (DrawingKind::FibCircles, format!(r#"{{"levels":[{}]}}"#, dashed(2.4))),
        (
            DrawingKind::FibSpeedResistanceArcs,
            format!(r#"{{"levels":[{}],"tool_options":{{"fibonacci":{{"trend_line":false}}}}}}"#, dashed(1.6)),
        ),
        (
            DrawingKind::FibSpiral,
            r##"{"color":"#111111","style":"dashed","tool_options":{"fibonacci":{"trend_line":false}}}"##.to_string(),
        ),
    ] {
        let mut chart = chart();
        let middle = chart.panes[0].top + chart.panes[0].height / 2.0;
        let points = match kind {
            DrawingKind::FibCircles => vec![at(&chart, -520.0, middle), at(&chart, -120.0, middle)],
            DrawingKind::FibSpeedResistanceArcs => {
                vec![at(&chart, -250.0, middle + 150.0), at(&chart, -130.0, middle)]
            }
            _ => vec![at(&chart, -60.0, middle), at(&chart, -30.0, middle)],
        };
        let id = add(&mut chart, kind, points, &options);
        let center = |chart: &ChartEngine| {
            let (a, b) = (anchor(chart, id, 0), anchor(chart, id, 1));
            if kind == DrawingKind::FibCircles {
                ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0)
            } else {
                a
            }
        };
        assert!(center(&chart).0 < 0.0, "{kind:?} is centered off the pane");
        let before_center = center(&chart);
        let before = dashes_inside(&mut chart, "#111111", (0.0, 0.0));
        // Scroll by a fraction of a bar: the center moves a few px left, the window with it.
        let range = chart.time_scale.visible_logical_range().unwrap();
        chart.set_visible_logical_range(range.left() + 0.37, range.right() + 0.37);
        chart.build_frame();
        let after_center = center(&chart);
        let shift = (before_center.0 - after_center.0, before_center.1 - after_center.1);
        assert!(shift.0 > 3.0 && shift.1.abs() < 1e-6, "{kind:?} scrolled: {shift:?}");
        let after = dashes_inside(&mut chart, "#111111", shift);
        assert!(
            dashes_stay_put(&before, &after, 0.3),
            "{kind:?} dashes follow the curve, not the pane: {before:?} vs {after:?}"
        );
    }
}

#[test]
fn spirals_centered_far_off_the_pane_stay_within_the_curve_tolerance() {
    let mut chart = chart();
    let bottom = chart.panes[0].top + chart.panes[0].height;
    // The center sits 100,000 px below the pane; the second anchor straight above it, inside.
    let points = vec![
        at(&chart, 300.0, bottom + 100_000.0),
        at(&chart, 300.0, bottom - 200.0),
    ];
    let id = add(
        &mut chart,
        DrawingKind::FibSpiral,
        points,
        r##"{"color":"#123456","tool_options":{"fibonacci":{"trend_line":false}}}"##,
    );
    let a = anchor(&chart, id, 0);
    let scene = Scene::of(&mut chart);
    let runs = scene.runs(css("#123456"));
    let top = chart.panes[0].top;
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
}

#[test]
fn price_levels_sit_on_exact_prices_in_every_price_scale_mode() {
    use crate::PriceScaleMode;
    for mode in [
        PriceScaleMode::Normal,
        PriceScaleMode::Logarithmic,
        PriceScaleMode::Percentage,
        PriceScaleMode::IndexedTo100,
    ] {
        let mut chart = chart();
        chart.set_price_scale_mode_for(0, crate::PriceScaleTarget::Right, mode);
        chart.build_frame();
        add(
            &mut chart,
            DrawingKind::FibRetracement,
            vec![p(10.0, 101.0), p(20.0, 105.0)],
            "{}",
        );
        add(
            &mut chart,
            DrawingKind::FibChannel,
            vec![p(24.0, 102.0), p(30.0, 104.0), p(27.0, 101.0)],
            &format!(
                r#"{{"levels":{},"fill_enabled":false}}"#,
                levels_json(&[(0.5, "#222222")])
            ),
        );
        let scene = Scene::of(&mut chart);
        // Retracement 0.5 at 103; labels keep prices, not the axis mode's percentages.
        let y = y_of(&chart, 103.0).round() as i32;
        assert!(
            scene.hlines(css("#4caf50")).iter().any(|line| line.0 == y),
            "{mode:?}: {:?} vs {y}",
            scene.hlines(css("#4caf50"))
        );
        assert!(scene.text("0.5 (103.00)").is_some(), "{mode:?}");
        // Channel 0.5: the first leg's line (103 at the third anchor's time) moved half the way
        // to the third anchor's 101.
        let line = &scene.polylines(css("#222222"))[0].0;
        let (start, end) = (
            (x_of(&chart, 24.0), y_of(&chart, 101.0)),
            (x_of(&chart, 30.0), y_of(&chart, 103.0)),
        );
        assert!(
            close(line[0], start, 1e-3) && close(line[1], end, 1e-3),
            "{mode:?}: {line:?}"
        );
    }
}
