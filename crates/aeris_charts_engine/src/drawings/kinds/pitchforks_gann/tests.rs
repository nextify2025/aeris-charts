//! Pitchforks & Gann family engine tests: catalog defaults, armed placement (with the partial
//! placement guide), shared-part geometry per tool at several device-pixel ratios, hit testing
//! (indexed against brute force, paint-bounded culling), drags, straighten, magnet, keyboard
//! nudges, time identity, schema and kind options, atomic patches with history, persistence with
//! default omission, clipboard, and sync.

use aeris_charts_render::color::Color;
use aeris_charts_render::draw_list::{LineStyle, Prim};

use super::super::super::{DrawingHandleMode, DrawingPlacement, DrawingTextLayout};
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
const PITCHFORKS: [DrawingKind; 4] = [
    DrawingKind::AndrewsPitchfork,
    DrawingKind::SchiffPitchfork,
    DrawingKind::ModifiedSchiffPitchfork,
    DrawingKind::InsidePitchfork,
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

fn fills(chart: &mut ChartEngine) -> Vec<Color> {
    let frame = chart.build_frame();
    frame.panes[0]
        .main
        .iter()
        .filter_map(|prim| match prim {
            Prim::BandFill { fill, .. } => Some(*fill),
            _ => None,
        })
        .collect()
}

/// `point` lies inside the main pane (the frame clips strokes a few px past it).
fn chart_contains(chart: &ChartEngine, point: (f64, f64)) -> bool {
    let pane = &chart.panes[0];
    (0.0..=chart.pane_w).contains(&point.0)
        && (pane.top..=pane.top + pane.height).contains(&point.1)
}

fn close(a: (f64, f64), b: (f64, f64), tolerance: f64) -> bool {
    (a.0 - b.0).abs() <= tolerance && (a.1 - b.1).abs() <= tolerance
}

fn parallel(a: (f64, f64), b: (f64, f64)) -> bool {
    (a.0 * b.1 - a.1 * b.0).abs() <= 1e-5 * a.0.hypot(a.1) * b.0.hypot(b.1)
}

fn sub(a: (f64, f64), b: (f64, f64)) -> (f64, f64) {
    (a.0 - b.0, a.1 - b.1)
}

fn mid(a: (f64, f64), b: (f64, f64)) -> (f64, f64) {
    ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0)
}

/// Only the given levels visible, all in `INK` with no fills: isolates line geometry.
fn ink_levels(values: &[f64]) -> String {
    let levels = values
        .iter()
        .map(|value| {
            serde_json::json!({
                "value": value, "color": INK, "visible": true, "style": "solid",
                "fill_between": false, "label_visible": false
            })
        })
        .collect::<Vec<_>>();
    serde_json::json!({ "levels": levels, "fill_enabled": false, "color": "#ff00ff" }).to_string()
}

#[test]
fn catalog_defaults_follow_each_tool() {
    for kind in KINDS {
        let drawing = crate::Drawing::new(1, kind, 0, Vec::new());
        let spec = kind.spec();
        assert!(spec.family.is_some(), "{kind:?} is a family tool");
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
            DrawingKind::GannBox | DrawingKind::GannSquare => {
                assert_eq!(spec.placement, DrawingPlacement::ClickAnchors { count: 2 });
                assert_eq!(spec.handles, DrawingHandleMode::RectangleBounds);
            }
            DrawingKind::GannSquareFixed => {
                assert_eq!(spec.placement, DrawingPlacement::ClickAnchors { count: 1 });
                assert_eq!(spec.handles, DrawingHandleMode::Anchors);
            }
            DrawingKind::GannFan => {
                assert_eq!(spec.placement, DrawingPlacement::ClickAnchors { count: 2 });
                assert!(drawing.extend_right, "Gann fan lines are rays");
                assert_eq!(
                    visible(&drawing),
                    [0.125, 0.25, 1.0 / 3.0, 0.5, 1.0, 2.0, 3.0, 4.0, 8.0]
                );
            }
            _ => {
                assert_eq!(spec.placement, DrawingPlacement::ClickAnchors { count: 3 });
                assert_eq!(spec.handles, DrawingHandleMode::Anchors);
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
        if kind == DrawingKind::GannBox {
            assert_eq!(visible(&drawing), [0.0, 0.25, 0.382, 0.5, 0.618, 0.75, 1.0]);
        }
        if matches!(kind, DrawingKind::GannSquare | DrawingKind::GannSquareFixed) {
            assert_eq!(visible(&drawing), [0.0, 0.2, 0.4, 0.6, 0.8, 1.0]);
        }
        assert_eq!(spec.default_width, 1.0);
        assert_eq!(drawing.width, 1.0);
    }
    let options = GannToolOptions::default();
    assert!(options.validate());
    assert_eq!(options.time_levels.len(), 7);
    assert_eq!(options.angles.len(), 9);
    assert_eq!(options.arcs.len(), 5);
    assert_eq!(options.size_bars, 20.0);
    assert_eq!(options.scale_ratio, None);
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
fn pitchfork_pivots_bases_and_tines_follow_each_variant() {
    for kind in PITCHFORKS {
        let mut chart = chart();
        // The inside pitchfork's reflected tine stays in view from a flatter swing.
        let points = if kind == DrawingKind::InsidePitchfork {
            vec![p(8.0, 102.0), p(16.0, 104.5), p(20.0, 103.0)]
        } else {
            anchors(kind)
        };
        let id = add(&mut chart, kind, points, &ink_levels(&[1.0]));
        let (a, b, c) = (
            anchor(&chart, id, 0),
            anchor(&chart, id, 1),
            anchor(&chart, id, 2),
        );
        let (pivot, center, half) = match kind {
            DrawingKind::AndrewsPitchfork => (a, mid(b, c), sub(c, mid(b, c))),
            DrawingKind::SchiffPitchfork => {
                ((a.0, (a.1 + b.1) / 2.0), mid(b, c), sub(c, mid(b, c)))
            }
            DrawingKind::ModifiedSchiffPitchfork => (mid(a, b), mid(b, c), sub(c, mid(b, c))),
            _ => (mid(a, b), c, sub(b, c)),
        };
        let direction = sub(center, pivot);
        let median = polylines(&mut chart, "#ff00ff");
        let median_line = median
            .iter()
            .find(|(points, ..)| close(points[0], pivot, 1e-3))
            .unwrap_or_else(|| panic!("{kind:?} median from its pivot: {median:?}"));
        assert!(
            close(
                median_line.0[1],
                (center.0 + direction.0, center.1 + direction.1),
                1e-3
            ),
            "{kind:?} median reaches one median length past the base"
        );
        let tines = polylines(&mut chart, INK);
        assert_eq!(tines.len(), 2, "{kind:?}: level 1 on both sides");
        for sign in [-1.0, 1.0] {
            let base = (center.0 + sign * half.0, center.1 + sign * half.1);
            let tine = tines
                .iter()
                .find(|(points, ..)| close(points[0], base, 1e-3))
                .unwrap_or_else(|| panic!("{kind:?} tine through {base:?}: {tines:?}"));
            assert!(parallel(sub(tine.0[1], tine.0[0]), direction));
            assert!(close(
                tine.0[1],
                (base.0 + direction.0, base.1 + direction.1),
                1e-3
            ));
        }
        if kind == DrawingKind::InsidePitchfork {
            // The tines pass through B and B's reflection about C.
            assert!(tines.iter().any(|(points, ..)| close(points[0], b, 1e-3)));
        } else {
            assert!(tines.iter().any(|(points, ..)| close(points[0], b, 1e-3)));
            assert!(tines.iter().any(|(points, ..)| close(points[0], c, 1e-3)));
        }
        // The shifted-pivot variants connect their first anchor with a dashed swing guide: solid
        // dash runs along A–B from A, split by the frame so every executor paints the same gaps.
        let guide = median
            .iter()
            .filter(|(points, ..)| points.iter().all(|&point| on_segment(point, a, b)))
            .collect::<Vec<_>>();
        if kind == DrawingKind::AndrewsPitchfork {
            assert!(guide.is_empty(), "{guide:?}");
        } else {
            assert!(guide.len() > 2, "{kind:?} dash runs: {guide:?}");
            assert!(close(guide[0].0[0], a, 1e-3));
            assert!(guide
                .iter()
                .all(|(_, width, style)| *width == 1.0 && *style == LineStyle::Solid));
        }
    }
}

/// `point` lies on the segment `a`–`b` (within float tolerance).
fn on_segment(point: (f64, f64), a: (f64, f64), b: (f64, f64)) -> bool {
    let (ab, ap) = (sub(b, a), sub(point, a));
    let length = ab.0.hypot(ab.1);
    let t = (ap.0 * ab.0 + ap.1 * ab.1) / (length * length);
    (ab.0 * ap.1 - ab.1 * ap.0).abs() / length <= 1e-3 && (-1e-6..=1.0 + 1e-6).contains(&t)
}

#[test]
fn dashed_and_dotted_family_strokes_reach_executors_as_solid_dash_runs() {
    let mut chart = chart();
    let id = add(
        &mut chart,
        DrawingKind::AndrewsPitchfork,
        anchors(DrawingKind::AndrewsPitchfork),
        &ink_levels(&[1.0]),
    );
    let solid = polylines(&mut chart, INK).len();
    for style in ["dashed", "dotted"] {
        let patch = serde_json::json!({
            "levels": [{
                "value": 1, "color": INK, "visible": true, "style": style,
                "fill_between": false, "label_visible": false
            }],
            "style": style
        });
        assert!(chart.drawing_apply_options(id, &patch.to_string()));
        let tines = polylines(&mut chart, INK);
        assert!(tines.len() > 2 * solid, "{style}: {} runs", tines.len());
        assert!(tines.iter().all(|(_, _, style)| *style == LineStyle::Solid));
        assert!(polylines(&mut chart, "#ff00ff")
            .iter()
            .all(|(_, _, style)| *style == LineStyle::Solid));
    }
    // The between-click guide of a three-anchor tool splits the same way.
    chart.remove_drawing(id);
    assert!(chart.set_drawing_tool(
        Some(DrawingKind::SchiffPitchfork),
        Some(r##"{"color":"#123456","style":"dashed"}"##),
        None
    ));
    chart.drawing_tool_activate(100.0, 300.0, DrawingModifiers::default());
    chart.drawing_tool_pointer_move(400.0, 150.0, DrawingModifiers::default(), false);
    let guide = polylines(&mut chart, INK)
        .into_iter()
        .filter(|(points, ..)| {
            points
                .iter()
                .all(|&point| on_segment(point, (100.0, 300.0), (400.0, 150.0)))
        })
        .collect::<Vec<_>>();
    assert!(guide.len() > 2, "{guide:?}");
    assert!(guide.iter().all(|(_, _, style)| *style == LineStyle::Solid));
}

#[test]
fn pitchfork_levels_fill_zones_and_extend_to_the_pane_edge() {
    let mut chart = chart();
    let id = add(
        &mut chart,
        DrawingKind::AndrewsPitchfork,
        anchors(DrawingKind::AndrewsPitchfork),
        "{}",
    );
    // Defaults: median plus the visible 0.5 and 1 levels on both sides, zones filled at 20%.
    assert_eq!(polylines(&mut chart, "#089981").len(), 2);
    assert_eq!(polylines(&mut chart, "#2962ff").len(), 2);
    let zones = fills(&mut chart);
    assert!(
        zones.contains(&Color::rgba(0x08, 0x99, 0x81, 51)),
        "{zones:?}"
    );
    assert!(
        zones.contains(&Color::rgba(0x29, 0x62, 0xff, 51)),
        "{zones:?}"
    );
    assert_eq!(zones.len(), 4, "two zones per side");
    assert!(chart.drawing_apply_options(id, r#"{"fill_enabled":false}"#));
    assert!(fills(&mut chart).is_empty());

    // Extending forward runs every line to the pane edge.
    assert!(chart.drawing_apply_options(id, r#"{"extend_right":true,"fill_enabled":true}"#));
    let (pane_top, pane_bottom) = (
        chart.panes[0].top,
        chart.panes[0].top + chart.panes[0].height,
    );
    let pane_w = chart.pane_w;
    let on_edge = |point: (f64, f64)| {
        point.0.abs() < 0.5
            || (point.0 - pane_w).abs() < 0.5
            || (point.1 - pane_top).abs() < 0.5
            || (point.1 - pane_bottom).abs() < 0.5
    };
    let lines = polylines(&mut chart, "#2962ff");
    assert!(
        lines.iter().all(|(points, ..)| on_edge(points[1])),
        "{lines:?}"
    );
    // Extended zones are clipped to the pane.
    let frame = chart.build_frame();
    let pane = &frame.panes[0];
    for prim in &pane.main {
        if let Prim::BandFill {
            upper_first,
            point_count,
            ..
        } = prim
        {
            for point in &pane.points[*upper_first as usize..(*upper_first + *point_count) as usize]
            {
                assert!(f64::from(point[0]) >= -1e-3 && f64::from(point[0]) <= pane_w + 1e-3);
            }
        }
    }
    // Level labels are off by default and paint the ratio at each tine end when enabled.
    assert!(!texts(&mut chart).iter().any(|(text, ..)| text == "1"));
    let mut levels = chart.drawing(id).unwrap().levels.clone();
    levels[5].label_visible = true;
    let patch = serde_json::json!({ "levels": levels }).to_string();
    assert!(chart.drawing_apply_options(id, &patch));
    let labels = texts(&mut chart);
    assert_eq!(
        labels.iter().filter(|(text, ..)| text == "1").count(),
        2,
        "{labels:?}"
    );
}

#[test]
fn pitchfan_rays_pass_through_the_level_points_of_the_base() {
    let mut chart = chart();
    let id = add(
        &mut chart,
        DrawingKind::Pitchfan,
        anchors(DrawingKind::Pitchfan),
        &ink_levels(&[0.5, 1.0]),
    );
    let (a, b, c) = (
        anchor(&chart, id, 0),
        anchor(&chart, id, 1),
        anchor(&chart, id, 2),
    );
    let center = mid(b, c);
    let rays = polylines(&mut chart, INK);
    assert_eq!(rays.len(), 4);
    for base in [b, c, mid(b, center), mid(center, c)] {
        let ray = rays
            .iter()
            .find(|(points, ..)| parallel(sub(points[1], points[0]), sub(base, a)))
            .unwrap_or_else(|| panic!("ray through {base:?}: {rays:?}"));
        assert!(close(ray.0[0], a, 1e-3), "rays start at the apex");
        // Twice as far as the base, or where the frame clips it just past the pane.
        let end = (2.0 * base.0 - a.0, 2.0 * base.1 - a.1);
        assert!(
            close(ray.0[1], end, 1e-3) || !chart_contains(&chart, end),
            "{ray:?} ends at {end:?}"
        );
    }
    let median = polylines(&mut chart, "#ff00ff");
    assert!(median.iter().any(|(points, ..)| close(points[0], a, 1e-3)
        && close(
            points[1],
            (2.0 * center.0 - a.0, 2.0 * center.1 - a.1),
            1e-3
        )));
    // Filled sectors between rays; extended rays reach the edge and their sectors clip.
    assert!(chart.drawing_apply_options(
        id,
        r##"{"fill_enabled":true,"extend_right":true,"levels":[{"value":1,"color":"#123456","visible":true,"style":"solid","fill_between":true,"label_visible":false}]}"##
    ));
    let zones = fills(&mut chart);
    assert_eq!(zones, vec![Color::rgba(0x12, 0x34, 0x56, 51); 2]);
    let rays = polylines(&mut chart, INK);
    assert!(rays
        .iter()
        .all(|(points, ..)| points[1].0 >= chart.pane_w - 0.5
            || points[1].1 <= chart.panes[0].top + 0.5
            || points[1].1 >= chart.panes[0].top + chart.panes[0].height - 0.5));
}

#[test]
fn gann_box_levels_zones_labels_angles_and_reverse() {
    let mut chart = chart();
    let id = add(
        &mut chart,
        DrawingKind::GannBox,
        anchors(DrawingKind::GannBox),
        "{}",
    );
    let (a, b) = (anchor(&chart, id, 0), anchor(&chart, id, 1));
    let (hlines, vlines) = crisp_lines(&mut chart, &super::GANN_BOX_LEVELS.map(|(_, css)| css));
    assert_eq!(hlines.len(), 7, "{hlines:?}");
    assert_eq!(vlines.len(), 7, "{vlines:?}");
    for (value, css) in super::GANN_BOX_LEVELS {
        let y = (a.1 + (b.1 - a.1) * value).round() as i32;
        assert!(
            hlines
                .iter()
                .any(|line| line.0 == y && line.3 == color(css)),
            "price level {value} at {y}"
        );
        let x = (a.0 + (b.0 - a.0) * value).round() as i32;
        assert!(
            vlines
                .iter()
                .any(|line| line.0 == x && line.3 == color(css)),
            "time level {value} at {x}"
        );
    }
    assert!(hlines
        .iter()
        .all(|&(_, x0, x1, _)| x0 == a.0.round() as i32 && x1 == b.0.round() as i32));
    // Six zones on each axis, each in its outer level's color at 20%.
    let zones = fills(&mut chart);
    assert_eq!(zones.len(), 12);
    assert!(zones.contains(&Color::rgba(0xff, 0x98, 0x00, 51)));
    // Ratio labels on all four sides.
    let labels = texts(&mut chart);
    assert_eq!(
        labels.iter().filter(|(text, ..)| text == "0.382").count(),
        4,
        "{labels:?}"
    );
    let left_label = labels
        .iter()
        .filter(|(text, ..)| text == "0.5")
        .map(|(_, x, _)| f64::from(*x))
        .fold(f64::INFINITY, f64::min);
    assert!(left_label < a.0.min(b.0), "left labels sit outside the box");

    // Gann angles from the pivot corner: the 1×1 is the box diagonal.
    assert!(chart.drawing_apply_options(id, r#"{"tool_options":{"gann":{"show_angles":true}}}"#));
    let diagonal = polylines(&mut chart, "#787b86");
    assert!(diagonal
        .iter()
        .any(|(points, ..)| close(points[0], a, 1e-3) && close(points[1], b, 1e-3)));
    // A 1×2 angle meets the far price edge halfway across.
    let steep = polylines(&mut chart, "#2962ff");
    assert!(steep.iter().any(|(points, ..)| close(
        points[1],
        (a.0 + (b.0 - a.0) / 2.0, b.1),
        1e-3
    )));
    // Reverse measures from the second anchor.
    assert!(chart.drawing_apply_options(id, r#"{"tool_options":{"gann":{"reverse":true}}}"#));
    let reversed = polylines(&mut chart, "#787b86");
    assert!(reversed
        .iter()
        .any(|(points, ..)| close(points[0], b, 1e-3) && close(points[1], a, 1e-3)));
    let frame = chart.build_frame();
    let quarter = (b.1 + (a.1 - b.1) * 0.25).round() as i32;
    assert!(frame.panes[0].main.iter().any(|prim| matches!(
        prim,
        Prim::HLine { y, color, .. } if *y == quarter && *color == color_of("#ff9800")
    )));
}

fn color_of(css: &str) -> Color {
    color(css)
}

type CrispLine = (i32, i32, i32, Color);

/// Crisp horizontal `(y, x0, x1, color)` and vertical `(x, y0, y1, color)` lines in the given
/// colors (the chart grid paints its own crisp lines, and the series' last-price line ends at the
/// pane's right edge).
fn crisp_lines(chart: &mut ChartEngine, colors: &[&str]) -> (Vec<CrispLine>, Vec<CrispLine>) {
    let colors = colors.iter().map(|css| color(css)).collect::<Vec<_>>();
    let right = chart.pane_w.round() as i32;
    let frame = chart.build_frame();
    let mut horizontal = Vec::new();
    let mut vertical = Vec::new();
    for prim in &frame.panes[0].main {
        match prim {
            Prim::HLine {
                y, x0, x1, color, ..
            } if colors.contains(color) && *x1 < right - 1 => {
                horizontal.push((*y, *x0, *x1, *color));
            }
            Prim::VLine {
                x, y0, y1, color, ..
            } if colors.contains(color) => vertical.push((*x, *y0, *y1, *color)),
            _ => {}
        }
    }
    (horizontal, vertical)
}

#[test]
fn gann_box_interior_drags_only_while_selected() {
    let mut chart = chart();
    let id = add(
        &mut chart,
        DrawingKind::GannBox,
        anchors(DrawingKind::GannBox),
        "{}",
    );
    let (a, b) = (anchor(&chart, id, 0), anchor(&chart, id, 1));
    // Inside a zone, clear of level lines.
    let inside = (a.0 + (b.0 - a.0) * 0.13, a.1 + (b.1 - a.1) * 0.13);
    chart.build_frame();
    assert_eq!(chart.hit_test_drawing(inside.0, inside.1), None);
    chart.set_selected_drawing(Some(id));
    assert_eq!(
        chart
            .hit_test_drawing(inside.0, inside.1)
            .map(|hit| hit.part),
        Some(DrawingDragPart::Body)
    );
    // Level lines are body targets either way.
    chart.set_selected_drawing(None);
    let level = (a.0 + (b.0 - a.0) * 0.4, a.1 + (b.1 - a.1) * 0.5);
    assert_eq!(
        chart.hit_test_drawing(level.0, level.1).map(|hit| hit.id),
        Some(id)
    );
}

#[test]
fn gann_squares_draw_grid_arcs_fans_and_measurements() {
    let mut chart = chart();
    let id = add(
        &mut chart,
        DrawingKind::GannSquare,
        vec![p(10.0, 101.0), p(20.0, 106.0)],
        "{}",
    );
    let (a, b) = (anchor(&chart, id, 0), anchor(&chart, id, 1));
    let (dx, dy) = sub(b, a);
    let (hlines, vlines) = crisp_lines(&mut chart, &super::GANN_SQUARE_GRID.map(|(_, css)| css));
    assert_eq!((hlines.len(), vlines.len()), (6, 6), "grid in fifths");
    // Each arc is a quarter ellipse around the pivot corner.
    let arcs = polylines(&mut chart, "#ff9800");
    let arc = arcs
        .iter()
        .find(|(points, ..)| points.len() > 3)
        .expect("the 0.2 arc");
    for &(x, y) in &arc.0 {
        let (u, v) = ((x - a.0) / (0.2 * dx), (y - a.1) / (0.2 * dy));
        assert!((u.hypot(v) - 1.0).abs() < 1e-3, "on the ellipse: {u} {v}");
    }
    assert!(close(arc.0[0], (a.0 + 0.2 * dx, a.1), 1e-3));
    assert!(close(*arc.0.last().unwrap(), (a.0, a.1 + 0.2 * dy), 1e-3));
    // The 1×1 fan line is the diagonal; the flatter 2×1 ends on the far time edge.
    assert!(polylines(&mut chart, "#787b86")
        .iter()
        .any(|(points, ..)| points.len() == 2 && close(points[1], b, 1e-3)));
    assert!(polylines(&mut chart, "#00bcd4")
        .iter()
        .any(|(points, ..)| points.len() == 2 && close(points[1], (b.0, a.1 + dy / 2.0), 1e-3)));
    // Arc zones fill.
    assert!(fills(&mut chart).contains(&Color::rgba(0xff, 0x98, 0x00, 51)));
    // Measurements: price range, bars, and price per bar.
    let labels = texts(&mut chart)
        .into_iter()
        .map(|(text, ..)| text)
        .collect::<Vec<_>>();
    assert!(labels.contains(&"5.00".to_string()), "{labels:?}");
    assert!(labels.contains(&"10 bars".to_string()), "{labels:?}");
    assert!(labels.contains(&"0.5000/bar".to_string()), "{labels:?}");
    assert!(chart.drawing_apply_options(id, r#"{"tool_options":{"gann":{"show_stats":false}}}"#));
    assert!(!texts(&mut chart)
        .iter()
        .any(|(text, ..)| text.ends_with("bars")));
}

#[test]
fn fixed_gann_squares_keep_their_size_and_scale_ratio() {
    let mut chart = chart();
    let id = add(
        &mut chart,
        DrawingKind::GannSquareFixed,
        vec![p(10.0, 101.0)],
        r#"{"tool_options":{"gann":{"size_bars":10}}}"#,
    );
    let pivot = anchor(&chart, id, 0);
    let right = chart.logical_to_coordinate(20.0).unwrap();
    // Without a ratio it is a square on screen, `size_bars` wide, growing upward.
    let border = |chart: &mut ChartEngine| {
        crisp_lines(chart, &["#787b86"])
            .1
            .into_iter()
            .map(|(x, y0, y1, _)| (x, y0, y1))
            .collect::<Vec<_>>()
    };
    let lines = border(&mut chart);
    let far = lines
        .iter()
        .find(|line| line.0 == right.round() as i32)
        .expect("far edge");
    let side = right - pivot.0;
    assert!(
        ((far.1 - far.2).abs() as f64 - side).abs() <= 1.0,
        "square: {far:?} vs {side}"
    );
    assert!(f64::from(far.1.min(far.2)) < pivot.1 - side + 1.5);
    // A ratio fixes the price side: 20 bars × 0.25 = 5 price units.
    assert!(chart.drawing_apply_options(
        id,
        r#"{"tool_options":{"gann":{"scale_ratio":0.25,"size_bars":20}}}"#
    ));
    let right = chart.logical_to_coordinate(30.0).unwrap();
    let top = chart.series_price_to_coordinate(0, 106.0).unwrap();
    let lines = border(&mut chart);
    assert!(lines
        .iter()
        .any(|line| line.0 == right.round() as i32 && line.1.min(line.2) == top.round() as i32));
    let labels = texts(&mut chart)
        .into_iter()
        .map(|(text, ..)| text)
        .collect::<Vec<_>>();
    assert!(labels.contains(&"5.00".to_string()) && labels.contains(&"20 bars".to_string()));
    assert!(labels.contains(&"0.2500/bar".to_string()), "{labels:?}");
    // Reverse grows downward; the size option resizes it (4 bars × 0.25 stays in view).
    assert!(chart.drawing_apply_options(
        id,
        r#"{"tool_options":{"gann":{"reverse":true,"size_bars":4}}}"#
    ));
    let bottom = chart.series_price_to_coordinate(0, 100.0).unwrap();
    assert!(bottom < chart.panes[0].top + chart.panes[0].height);
    let right = chart.logical_to_coordinate(14.0).unwrap();
    assert!(border(&mut chart)
        .iter()
        .any(|line| line.0 == right.round() as i32 && line.1.max(line.2) == bottom.round() as i32));
    // Two keyboard handles: the anchor and the derived resize corner.
    assert_eq!(chart.drawing_handle_count(id), Some(2));
}

#[test]
fn gann_fans_label_angles_and_honor_a_scale_ratio() {
    let mut chart = chart();
    let id = add(
        &mut chart,
        DrawingKind::GannFan,
        vec![p(10.0, 101.0), p(20.0, 104.0)],
        r#"{"extend_right":false}"#,
    );
    let (o, b) = (anchor(&chart, id, 0), anchor(&chart, id, 1));
    // The 1×1 runs to the second anchor; 2×1 meets the far time edge halfway up; 1×2 meets the
    // far price edge halfway across.
    assert!(polylines(&mut chart, "#787b86")
        .iter()
        .any(|(points, ..)| close(points[0], o, 1e-3) && close(points[1], b, 1e-3)));
    assert!(polylines(&mut chart, "#00bcd4")
        .iter()
        .any(|(points, ..)| close(points[1], (b.0, o.1 + (b.1 - o.1) / 2.0), 1e-3)));
    assert!(polylines(&mut chart, "#2962ff")
        .iter()
        .any(|(points, ..)| close(points[1], (o.0 + (b.0 - o.0) / 2.0, b.1), 1e-3)));
    let labels = texts(&mut chart)
        .into_iter()
        .map(|(text, ..)| text)
        .collect::<Vec<_>>();
    for name in [
        "8x1", "4x1", "3x1", "2x1", "1x1", "1x2", "1x3", "1x4", "1x8",
    ] {
        assert!(labels.contains(&name.to_string()), "{name}: {labels:?}");
    }
    assert_eq!(
        fills(&mut chart).len(),
        8,
        "one sector between each pair of lines"
    );

    // A scale ratio fixes the 1×1 at 0.2 price per bar: over 10 bars it rises 2 price units.
    assert!(chart.drawing_apply_options(id, r#"{"tool_options":{"gann":{"scale_ratio":0.2}}}"#));
    let target = chart
        .drawing_to_px_for(0, DrawingPriceScale::Right, p(20.0, 103.0))
        .unwrap();
    assert!(polylines(&mut chart, "#787b86")
        .iter()
        .any(|(points, ..)| close(points[1], target, 1e-3)));
    // Extended (the default) the lines are rays to the pane edge.
    assert!(chart.drawing_apply_options(id, r#"{"extend_right":true}"#));
    let rays = polylines(&mut chart, "#787b86");
    assert!(rays
        .iter()
        .any(|(points, ..)| points[1].0 >= chart.pane_w - 0.5
            || points[1].1 <= chart.panes[0].top + 0.5));
}

#[test]
fn a_price_basis_rescale_scales_gann_price_per_bar_ratios_with_the_anchors() {
    let times = hourly(40);
    let mut chart = chart_with(800.0, &times, 1.0);
    let square = add(
        &mut chart,
        DrawingKind::GannSquareFixed,
        vec![p(10.0, 100.0)],
        r#"{"tool_options":{"gann":{"scale_ratio":0.25,"size_bars":20}}}"#,
    );
    let fan = add(
        &mut chart,
        DrawingKind::GannFan,
        vec![p(10.0, 101.0), p(20.0, 104.0)],
        r#"{"extend_right":false,"tool_options":{"gann":{"scale_ratio":0.2}}}"#,
    );
    let ratio = |chart: &ChartEngine, id: DrawingId| {
        chart
            .drawing(id)
            .unwrap()
            .tool_options
            .gann
            .as_ref()
            .unwrap()
            .scale_ratio
            .unwrap()
    };
    // The square's border and the fan's 1×1 in px, and the square's stats.
    let geometry = |chart: &mut ChartEngine| {
        let border = crisp_lines(chart, &["#787b86"]).1;
        let pivot = anchor(chart, fan, 0);
        let one_to_one = polylines(chart, "#787b86")
            .into_iter()
            .filter(|(points, ..)| close(points[0], pivot, 1e-3))
            .map(|(points, ..)| points)
            .collect::<Vec<_>>();
        (border, one_to_one)
    };
    let labels = |chart: &mut ChartEngine| {
        texts(chart)
            .into_iter()
            .map(|(text, ..)| text)
            .collect::<Vec<_>>()
    };
    let before = geometry(&mut chart);
    assert!(!before.0.is_empty() && !before.1.is_empty());
    // A committed resize whose undo snapshot must follow the basis too.
    assert!(chart.drawing_apply_options(square, r#"{"tool_options":{"gann":{"size_bars":24}}}"#));
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
    chart.build_frame();
    assert_eq!(ratio(&chart, square), 0.125);
    assert_eq!(ratio(&chart, fan), 0.1);
    let after = geometry(&mut chart);
    assert_eq!(
        after.0, before.0,
        "the square keeps measuring the same bars"
    );
    assert_eq!(after.1.len(), before.1.len());
    for (a, b) in after.1.iter().zip(&before.1) {
        for (a, b) in a.iter().zip(b) {
            assert!(close(*a, *b, 1e-3), "the fan's 1×1: {a:?} vs {b:?}");
        }
    }
    let stats = labels(&mut chart);
    assert!(
        stats.contains(&"2.50".to_string()) && stats.contains(&"0.1250/bar".to_string()),
        "{stats:?}"
    );
    // Redo replays the resize in the new basis.
    assert!(chart.redo_drawing());
    assert_eq!(ratio(&chart, square), 0.125);
    assert!(chart.undo_drawing());

    // An in-flight corner drag keeps its restore snapshot in the new basis.
    chart.set_selected_drawing(Some(square));
    let corner = (
        chart.logical_to_coordinate(30.0).unwrap(),
        chart.series_price_to_coordinate(0, 52.5).unwrap(),
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
}

#[test]
fn frames_scale_family_geometry_with_the_device_pixel_ratio() {
    // 801 wide at 1.5 makes the horizontal and vertical bitmap ratios differ.
    for (width, dpr) in [(800.0, 1.0), (800.0, 2.0), (801.0, 1.5)] {
        let mut chart = chart_with(width, &hourly(40), dpr);
        let hpr = (chart.pane_w * dpr).round() / chart.pane_w;
        let vpr = (chart.pane_h * dpr).round() / chart.pane_h;
        let fork = add(
            &mut chart,
            DrawingKind::AndrewsPitchfork,
            anchors(DrawingKind::AndrewsPitchfork),
            &ink_levels(&[1.0]),
        );
        let (b, c) = (anchor(&chart, fork, 1), anchor(&chart, fork, 2));
        let tines = polylines(&mut chart, INK);
        assert!(
            (f64::from(tines[0].1) - vpr).abs() < 1e-5,
            "width scales with vpr"
        );
        for base in [b, c] {
            let scaled = (base.0 * hpr, base.1 * vpr);
            assert!(
                tines
                    .iter()
                    .any(|(points, ..)| close(points[0], scaled, 1e-3)),
                "dpr {dpr}: {scaled:?} in {tines:?}"
            );
        }
        chart.remove_drawing(fork);
        // The fixed square derives its far corner through `point_px` in the anchors' space.
        let square = add(
            &mut chart,
            DrawingKind::GannSquareFixed,
            vec![p(10.0, 101.0)],
            r#"{"tool_options":{"gann":{"scale_ratio":0.25}}}"#,
        );
        let far_x = chart.logical_to_coordinate(30.0).unwrap() * hpr;
        let far_y = chart.series_price_to_coordinate(0, 106.0).unwrap() * vpr;
        let frame = chart.build_frame();
        assert!(frame.panes[0].main.iter().any(|prim| matches!(
            prim,
            Prim::VLine { x, y0, y1, .. } if *x == far_x.round() as i32
                && (*y0).min(*y1) == far_y.round() as i32
        )));
        chart.remove_drawing(square);
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

/// Twenty-two trend lines past the drawings under test, so candidate queries take the culled path.
fn crowd(chart: &mut ChartEngine) {
    for index in 0..22 {
        add(
            chart,
            DrawingKind::TrendLine,
            vec![p(36.0 + index as f64 * 0.1, 100.0), p(37.0, 100.5)],
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
fn geometry_past_the_anchors_stays_visible_and_hittable_while_far_drawings_cull() {
    let mut chart = chart();
    crowd(&mut chart);
    // Anchors left of the viewport; the tines reach into it.
    let fork = add(
        &mut chart,
        DrawingKind::AndrewsPitchfork,
        vec![p(-30.0, 101.0), p(-12.0, 104.0), p(-4.0, 102.0)],
        &ink_levels(&[1.0]),
    );
    // A fixed square anchored off-screen whose body is on screen.
    let square = add(
        &mut chart,
        DrawingKind::GannSquareFixed,
        vec![p(-5.0, 100.0)],
        r#"{"tool_options":{"gann":{"scale_ratio":0.1}}}"#,
    );
    // A pitchfork far away in time stays culled.
    let far = add(
        &mut chart,
        DrawingKind::AndrewsPitchfork,
        vec![p(-300.0, 101.0), p(-290.0, 104.0), p(-285.0, 102.0)],
        "{}",
    );
    chart.build_frame();
    assert!(viewport_candidate(&chart, fork));
    assert!(viewport_candidate(&chart, square));
    assert!(!viewport_candidate(&chart, far));
    let tines = polylines(&mut chart, INK);
    let visible_end = tines
        .iter()
        .map(|(points, ..)| points[1])
        .find(|&(x, y)| {
            x > 5.0 && y > chart.panes[0].top && y < chart.panes[0].top + chart.panes[0].height
        })
        .expect("a tine end on screen");
    // Hit a point on that tine just before its end.
    let start = tines
        .iter()
        .find(|(points, ..)| points[1] == visible_end)
        .unwrap()
        .0[0];
    let probe = (
        visible_end.0 - (visible_end.0 - start.0) * 0.05,
        visible_end.1 - (visible_end.1 - start.1) * 0.05,
    );
    assert!(probe.0 > 0.0);
    assert_eq!(
        chart.hit_test_drawing(probe.0, probe.1).map(|hit| hit.id),
        Some(fork)
    );
    let square_edge = chart.logical_to_coordinate(15.0).unwrap();
    let mid_price = chart.series_price_to_coordinate(0, 101.0).unwrap();
    assert_eq!(
        chart
            .hit_test_drawing(square_edge, mid_price)
            .map(|hit| hit.id),
        Some(square)
    );
}

#[test]
fn a_flat_gann_fan_with_a_scale_ratio_bounds_the_lines_it_paints() {
    let mut chart = chart();
    crowd(&mut chart);
    // Both anchors at one price: every line runs to the time edge at its own slope, far above
    // the anchors' flat box.
    let fan = add(
        &mut chart,
        DrawingKind::GannFan,
        vec![p(10.0, 101.0), p(14.0, 101.0)],
        r#"{"extend_right":false,"fill_enabled":false,"tool_options":{"gann":{"scale_ratio":0.25}}}"#,
    );
    let pivot = anchor(&chart, fan, 0);
    // The 1×4 line rises 4 × 0.25 × 4 bars = 4 price units by the second anchor's bar.
    let end = chart
        .drawing_to_px_for(0, DrawingPriceScale::Right, p(14.0, 105.0))
        .unwrap();
    let steep = polylines(&mut chart, "#9c27b0");
    assert!(
        steep
            .iter()
            .any(|(points, ..)| close(points[0], pivot, 1e-3) && close(points[1], end, 1e-3)),
        "{steep:?}"
    );
    let probe = (
        pivot.0 + (end.0 - pivot.0) * 0.9,
        pivot.1 + (end.1 - pivot.1) * 0.9,
    );
    assert_eq!(
        chart.hit_test_drawing(probe.0, probe.1).map(|hit| hit.id),
        Some(fan)
    );
    assert_eq!(
        chart.hit_test_drawing(probe.0, probe.1),
        chart.hit_test_drawing_bruteforce(probe.0, probe.1)
    );
}

#[test]
fn drags_nudges_straighten_and_magnet_edit_as_single_history_entries() {
    let mut chart = chart();
    let id = add(
        &mut chart,
        DrawingKind::AndrewsPitchfork,
        anchors(DrawingKind::AndrewsPitchfork),
        "{}",
    );
    let before = chart.drawing(id).unwrap().points.clone();
    chart.set_selected_drawing(Some(id));
    // Every anchor plus the derived base midpoint.
    assert_eq!(chart.drawing_handle_count(id), Some(4));
    // Drag the third anchor: only it moves.
    let (cx, cy) = anchor(&chart, id, 2);
    assert!(chart.drawing_drag_start_at(cx, cy));
    chart.drawing_drag_to(cx + 30.0, cy + 12.0, DrawingModifiers::default());
    chart.drawing_drag_end();
    let after = chart.drawing(id).unwrap().points.clone();
    assert_eq!(after[..2], before[..2]);
    assert!(close(anchor(&chart, id, 2), (cx + 30.0, cy + 12.0), 1e-3));
    assert!(chart.undo_drawing());
    assert_eq!(chart.drawing(id).unwrap().points, before);
    // Body drag from the median moves every anchor rigidly.
    let (a, b, c) = (
        anchor(&chart, id, 0),
        anchor(&chart, id, 1),
        anchor(&chart, id, 2),
    );
    let center = mid(b, c);
    let on_median = (a.0 + (center.0 - a.0) * 0.4, a.1 + (center.1 - a.1) * 0.4);
    chart.set_selected_drawing(None);
    assert!(chart.drawing_drag_start_at(on_median.0, on_median.1));
    chart.drawing_drag_to(
        on_median.0 + 20.0,
        on_median.1 - 15.0,
        DrawingModifiers::default(),
    );
    chart.drawing_drag_end();
    for (index, start) in [a, b, c].into_iter().enumerate() {
        assert!(close(
            anchor(&chart, id, index),
            (start.0 + 20.0, start.1 - 15.0),
            1e-3
        ));
    }
    assert!(chart.undo_drawing());
    // Keyboard: an anchor handle's nudge moves one anchor as one undo step.
    chart.set_selected_drawing(Some(id));
    assert!(chart.nudge_selected_drawing(10.0, 0.0, Some(1)));
    assert!(close(anchor(&chart, id, 1), (b.0 + 10.0, b.1), 1e-3));
    assert!(chart.undo_drawing());
    assert_eq!(chart.drawing(id).unwrap().points, before);

    // Gann box: eight bounds handles and Shift-square corners; Gann fan: 45° straighten.
    let gann_box = add(
        &mut chart,
        DrawingKind::GannBox,
        anchors(DrawingKind::GannBox),
        "{}",
    );
    assert_eq!(chart.drawing_handle_count(gann_box), Some(8));
    chart.set_selected_drawing(Some(gann_box));
    let (ba, bb) = (anchor(&chart, gann_box, 0), anchor(&chart, gann_box, 1));
    let top_right = (ba.0.max(bb.0), ba.1.min(bb.1));
    assert!(chart.drawing_drag_start_at(top_right.0, top_right.1));
    chart.drawing_drag_to(
        top_right.0 + 40.0,
        top_right.1 - 3.0,
        DrawingModifiers {
            magnet: false,
            straighten: true,
        },
    );
    chart.drawing_drag_end();
    let (na, nb) = (anchor(&chart, gann_box, 0), anchor(&chart, gann_box, 1));
    assert!(
        ((na.0 - nb.0).abs() - (na.1 - nb.1).abs()).abs() < 1e-6,
        "squared"
    );

    let fan = add(
        &mut chart,
        DrawingKind::GannFan,
        anchors(DrawingKind::GannFan),
        "{}",
    );
    chart.set_selected_drawing(Some(fan));
    assert_eq!(chart.drawing_handle_count(fan), Some(2));
    let (fx, fy) = anchor(&chart, fan, 1);
    assert!(chart.drawing_drag_start_at(fx, fy));
    chart.drawing_drag_to(
        fx + 25.0,
        fy + 4.0,
        DrawingModifiers {
            magnet: false,
            straighten: true,
        },
    );
    chart.drawing_drag_end();
    let (o, n) = (anchor(&chart, fan, 0), anchor(&chart, fan, 1));
    let angle = (o.1 - n.1).atan2(n.0 - o.0).to_degrees();
    assert!(
        (angle / 45.0 - (angle / 45.0).round()).abs() < 1e-6,
        "{angle}"
    );

    // Strong magnet snaps a placed anchor to the bar's value.
    chart.set_drawing_magnet_mode(DrawingMagnetMode::Strong);
    assert!(chart.set_drawing_tool(Some(DrawingKind::GannSquareFixed), None, None));
    let x = chart.logical_to_coordinate(12.0).unwrap() + 3.0;
    let y = chart.series_price_to_coordinate(0, 102.4).unwrap();
    let snapped = chart
        .drawing_tool_activate(x, y, DrawingModifiers::default())
        .created
        .unwrap();
    let point = chart.drawing(snapped).unwrap().points[0];
    assert_eq!(point.logical, 12.0);
    assert_eq!(point.price, 100.0 + (12 % 7) as f64);
}

#[test]
fn a_pitchfork_base_midpoint_handle_moves_both_handle_anchors() {
    let mut chart = chart();
    for kind in PITCHFORKS.into_iter().chain([DrawingKind::Pitchfan]) {
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
        assert_eq!(
            chart.hit_test_drawing(base.0, base.1).map(|hit| hit.part),
            Some(DrawingDragPart::Handle(0)),
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
        assert!(chart.undo_drawing());
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
    let to = (
        chart.logical_to_coordinate(24.0).unwrap() + 2.0,
        base.1 + 7.0,
    );
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

fn at_x(chart: &ChartEngine, logical: f64) -> f64 {
    chart.logical_to_coordinate(logical).unwrap()
}

fn at_y(chart: &ChartEngine, price: f64) -> f64 {
    chart.series_price_to_coordinate(0, price).unwrap()
}

#[test]
fn a_fixed_gann_square_resizes_from_its_corner_handle() {
    let mut chart = chart();
    let id = add(
        &mut chart,
        DrawingKind::GannSquareFixed,
        vec![p(10.0, 101.0)],
        r#"{"tool_options":{"gann":{"size_bars":10}}}"#,
    );
    chart.set_selected_drawing(Some(id));
    let gann = |chart: &ChartEngine| {
        chart
            .drawing(id)
            .unwrap()
            .tool_options
            .gann
            .clone()
            .unwrap()
    };
    let pivot = anchor(&chart, id, 0);
    let corner = (at_x(&chart, 20.0), pivot.1 - (at_x(&chart, 20.0) - pivot.0));
    let hit = chart.hit_test_drawing(corner.0, corner.1).unwrap();
    assert_eq!(
        (hit.part, hit.cursor),
        (DrawingDragPart::Handle(0), "nesw-resize")
    );

    // Square on screen: the corner's larger distance sets the side in whole bars.
    let to = (
        at_x(&chart, 24.0) + 3.0,
        pivot.1 - (at_x(&chart, 24.0) - pivot.0) * 0.5,
    );
    assert!(chart.drawing_drag_start_at(corner.0, corner.1));
    chart.drawing_drag_to(to.0, to.1, DrawingModifiers::default());
    chart.drawing_drag_end();
    assert_eq!(
        (
            gann(&chart).size_bars,
            gann(&chart).reverse,
            gann(&chart).scale_ratio
        ),
        (14.0, false, None)
    );
    let corner = (at_x(&chart, 24.0), pivot.1 - (at_x(&chart, 24.0) - pivot.0));
    assert_eq!(
        chart
            .hit_test_drawing(corner.0, corner.1)
            .map(|hit| hit.part),
        Some(DrawingDragPart::Handle(0)),
        "the handle follows the new corner"
    );
    // Undo restores the options with the anchors; a cancelled drag leaves them untouched.
    assert!(chart.undo_drawing());
    assert_eq!(gann(&chart).size_bars, 10.0);
    let corner = (at_x(&chart, 20.0), pivot.1 - (at_x(&chart, 20.0) - pivot.0));
    assert!(chart.drawing_drag_start_at(corner.0, corner.1));
    chart.drawing_drag_to(
        at_x(&chart, 30.0),
        pivot.1 + 80.0,
        DrawingModifiers::default(),
    );
    assert_eq!(gann(&chart).size_bars, 20.0);
    assert!(gann(&chart).reverse, "below the anchor it grows downward");
    chart.drawing_drag_cancel();
    assert_eq!(
        (gann(&chart).size_bars, gann(&chart).reverse),
        (10.0, false)
    );

    // With a ratio the corner's price sets the ratio; Shift keeps it.
    assert!(chart.drawing_apply_options(
        id,
        r#"{"tool_options":{"gann":{"scale_ratio":0.25,"size_bars":20}}}"#
    ));
    let corner = (at_x(&chart, 30.0), at_y(&chart, 106.0));
    assert!(chart.drawing_drag_start_at(corner.0, corner.1));
    chart.drawing_drag_to(
        at_x(&chart, 26.0),
        at_y(&chart, 104.0),
        DrawingModifiers::default(),
    );
    chart.drawing_drag_end();
    assert_eq!(gann(&chart).size_bars, 16.0);
    assert!((gann(&chart).scale_ratio.unwrap() - 3.0 / 16.0).abs() < 1e-6);
    let corner = (at_x(&chart, 26.0), at_y(&chart, 104.0));
    assert!(chart.drawing_drag_start_at(corner.0, corner.1));
    // Pressing Shift mid-drag restores the press ratio rather than the previous sample's.
    chart.drawing_drag_to(
        at_x(&chart, 28.0),
        at_y(&chart, 109.0),
        DrawingModifiers::default(),
    );
    assert!((gann(&chart).scale_ratio.unwrap() - 8.0 / 18.0).abs() < 1e-6);
    chart.drawing_drag_to(
        at_x(&chart, 30.0),
        at_y(&chart, 110.0),
        DrawingModifiers {
            magnet: false,
            straighten: true,
        },
    );
    chart.drawing_drag_end();
    assert_eq!(gann(&chart).size_bars, 20.0);
    assert!((gann(&chart).scale_ratio.unwrap() - 3.0 / 16.0).abs() < 1e-6);

    // Keyboard: the corner is the second handle; a one-bar nudge adds a bar.
    let spacing = at_x(&chart, 1.0) - at_x(&chart, 0.0);
    assert!(chart.nudge_selected_drawing(spacing, 0.0, Some(1)));
    assert_eq!(gann(&chart).size_bars, 21.0);
    assert!(chart.undo_drawing());
    assert_eq!(gann(&chart).size_bars, 20.0);
    // The anchor handle still moves the whole square.
    assert!(chart.nudge_selected_drawing(spacing, 0.0, Some(0)));
    assert_eq!(chart.drawing(id).unwrap().points[0].logical, 11.0);
}

#[test]
fn keyboard_steps_resize_a_fixed_gann_square_across_whole_bars() {
    let mut chart = chart();
    let id = add(
        &mut chart,
        DrawingKind::GannSquareFixed,
        vec![p(10.0, 101.0)],
        r#"{"tool_options":{"gann":{"size_bars":10}}}"#,
    );
    chart.set_selected_drawing(Some(id));
    let gann = |chart: &ChartEngine| {
        chart
            .drawing(id)
            .unwrap()
            .tool_options
            .gann
            .clone()
            .unwrap()
    };
    let spacing = at_x(&chart, 1.0) - at_x(&chart, 0.0);
    assert!(spacing > 2.0, "a one-pixel step is well under a bar");

    // Sub-bar steps of the corner (keyboard handle 1) cross a whole bar each, both ways, and
    // accumulate across presses.
    for expected in [11.0, 12.0, 13.0] {
        assert!(chart.nudge_selected_drawing(1.0, 0.0, Some(1)));
        assert_eq!(gann(&chart).size_bars, expected);
    }
    assert!(chart.nudge_selected_drawing(-1.0, 0.0, Some(1)));
    assert_eq!(gann(&chart).size_bars, 12.0);
    // Square on screen, a vertical step sizes by the axis it moved: up grows, down shrinks.
    assert!(chart.nudge_selected_drawing(0.0, -1.0, Some(1)));
    assert_eq!(gann(&chart).size_bars, 13.0);
    assert!(chart.nudge_selected_drawing(0.0, 1.0, Some(1)));
    assert_eq!(
        (gann(&chart).size_bars, gann(&chart).reverse),
        (12.0, false)
    );
    // Each step is its own undo entry.
    assert!(chart.undo_drawing());
    assert_eq!(gann(&chart).size_bars, 13.0);

    // With a ratio a vertical step keeps the side and edits the ratio through the corner's
    // price; a horizontal step still crosses a whole bar.
    assert!(chart.drawing_apply_options(
        id,
        r#"{"tool_options":{"gann":{"scale_ratio":0.25,"size_bars":20}}}"#
    ));
    assert!(chart.nudge_selected_drawing(0.0, -1.0, Some(1)));
    assert_eq!(gann(&chart).size_bars, 20.0);
    assert!(gann(&chart).scale_ratio.unwrap() > 0.25);
    assert!(chart.nudge_selected_drawing(1.0, 0.0, Some(1)));
    assert_eq!(gann(&chart).size_bars, 21.0);

    // A step below the one-bar minimum changes nothing and reports it.
    assert!(chart.drawing_apply_options(
        id,
        r#"{"tool_options":{"gann":{"scale_ratio":null,"size_bars":1}}}"#
    ));
    let before = chart.drawing(id).unwrap().clone();
    assert!(!chart.nudge_selected_drawing(-1.0, 0.0, Some(1)));
    assert_eq!(chart.drawing(id).unwrap(), &before);
}

#[test]
fn locked_hidden_and_reordered_drawings_follow_the_common_contract() {
    let mut chart = chart();
    let id = add(
        &mut chart,
        DrawingKind::InsidePitchfork,
        anchors(DrawingKind::InsidePitchfork),
        &ink_levels(&[1.0]),
    );
    let (b, c) = (anchor(&chart, id, 1), anchor(&chart, id, 2));
    let probe = mid(b, c);
    assert!(chart.drawing_apply_options(id, r#"{"locked":true}"#));
    assert!(
        !chart.drawing_drag_start_at(probe.0, probe.1),
        "locked drawings do not move"
    );
    assert_eq!(chart.selected_drawing(), Some(id), "but still select");
    assert!(chart.drawing_apply_options(id, r#"{"locked":false,"visible":false}"#));
    assert!(polylines(&mut chart, INK).is_empty());
    assert_eq!(chart.hit_test_drawing(probe.0, probe.1), None);
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
fn schema_kind_options_and_tool_option_patches_are_typed_and_atomic() {
    let names = |kind| {
        crate::drawing_property_schema(kind)
            .properties
            .into_iter()
            .filter(|property| property.name.starts_with("tool_options."))
            .map(|property| property.name)
            .collect::<Vec<_>>()
    };
    assert!(names(DrawingKind::AndrewsPitchfork).is_empty());
    assert_eq!(
        names(DrawingKind::GannBox),
        [
            "tool_options.gann.time_levels",
            "tool_options.gann.angles",
            "tool_options.gann.reverse",
            "tool_options.gann.show_angles"
        ]
    );
    assert_eq!(
        names(DrawingKind::GannFan),
        ["tool_options.gann.scale_ratio"]
    );
    let fixed = crate::drawing_property_schema(DrawingKind::GannSquareFixed);
    let size = fixed
        .properties
        .iter()
        .find(|property| property.name == "tool_options.gann.size_bars")
        .unwrap();
    assert_eq!(size.default, serde_json::json!(20.0));
    assert_eq!(
        (size.min, size.max),
        (Some(1.0), Some(super::MAX_GANN_SQUARE_BARS))
    );
    let schema = crate::drawing_property_schema(DrawingKind::Pitchfan);
    let default_of = |name: &str| {
        schema
            .properties
            .iter()
            .find(|property| property.name == name)
            .unwrap()
            .default
            .clone()
    };
    assert_eq!(default_of("color"), serde_json::json!(MEDIAN_COLOR));
    assert_eq!(default_of("fill_enabled"), serde_json::json!(true));
    assert_eq!(default_of("levels").as_array().unwrap().len(), 9);

    let mut chart = chart();
    let fork = add(
        &mut chart,
        DrawingKind::Pitchfan,
        anchors(DrawingKind::Pitchfan),
        "{}",
    );
    let kind_options = |chart: &ChartEngine, id| {
        serde_json::from_str::<serde_json::Value>(&chart.drawing_kind_options_json(id).unwrap())
            .unwrap()
    };
    let value = kind_options(&chart, fork);
    assert_eq!(value["kind"], "pitchfork");
    assert_eq!(value["levels"].as_array().unwrap().len(), 9);

    let id = add(
        &mut chart,
        DrawingKind::GannFan,
        anchors(DrawingKind::GannFan),
        "{}",
    );
    let value = kind_options(&chart, id);
    assert_eq!(value["kind"], "gann");
    assert_eq!(value["scale_ratio"], serde_json::Value::Null);
    assert_eq!(value["size_bars"], serde_json::json!(20.0));
    assert!(chart.drawing_apply_options(
        id,
        r#"{"tool_options":{"gann":{"scale_ratio":0.5}},"width":2}"#
    ));
    assert_eq!(
        kind_options(&chart, id)["scale_ratio"],
        serde_json::json!(0.5)
    );
    // An invalid block rejects the whole patch.
    let before = chart.drawing(id).unwrap().clone();
    for invalid in [
        r#"{"tool_options":{"gann":{"scale_ratio":-1}},"width":9}"#,
        r#"{"tool_options":{"gann":{"size_bars":0}},"width":9}"#,
        r##"{"tool_options":{"gann":{"angles":[{"value":0,"color":"#fff","visible":true,"style":"solid","fill_between":false,"label_visible":false}]}}}"##,
        r#"{"tool_options":{"gann":{"reverse":"yes"}}}"#,
        r#"{"tool_options":{"gann":7}}"#,
    ] {
        assert!(!chart.drawing_apply_options(id, invalid), "{invalid}");
        assert_eq!(chart.drawing(id).unwrap(), &before);
    }
    // Absent keys keep their values, `null` resets, and each change is one undo step.
    assert!(chart.drawing_apply_options(id, r#"{"tool_options":{"gann":{"reverse":true}}}"#));
    let block = chart
        .drawing(id)
        .unwrap()
        .tool_options
        .gann
        .clone()
        .unwrap();
    assert_eq!((block.scale_ratio, block.reverse), (Some(0.5), true));
    assert!(chart.drawing_apply_options(id, r#"{"tool_options":{"gann":{"scale_ratio":null}}}"#));
    assert_eq!(
        chart
            .drawing(id)
            .unwrap()
            .tool_options
            .gann
            .as_ref()
            .unwrap()
            .scale_ratio,
        None
    );
    assert!(chart.drawing_apply_options(id, r#"{"tool_options":{"gann":null}}"#));
    assert!(chart.drawing(id).unwrap().tool_options.is_empty());
    assert!(chart.undo_drawing());
    assert!(chart.drawing(id).unwrap().tool_options.gann.is_some());
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

fn frame_cost(chart: &mut ChartEngine) -> (usize, usize) {
    let frame = chart.build_frame();
    (frame.panes[0].main.len(), frame.panes[0].points.len())
}

fn level_json(value: f64, style: &str) -> serde_json::Value {
    serde_json::json!({
        "value": value, "color": INK, "visible": true, "style": style,
        "fill_between": true, "label_visible": true
    })
}

#[test]
fn extreme_levels_and_sizes_keep_frame_work_bounded_and_coordinates_finite() {
    let mut chart = chart();
    let (base_prims, base_points) = frame_cost(&mut chart);
    let (pane_top, pane_bottom) = (
        chart.panes[0].top,
        chart.panes[0].top + chart.panes[0].height,
    );
    // Dotted pitchfan rays and base 2000 half handles out reach ~10^5 px past the pane; only the
    // part near the pane may be dash-split.
    let fan = add(
        &mut chart,
        DrawingKind::Pitchfan,
        anchors(DrawingKind::Pitchfan),
        &serde_json::json!({ "style": "dotted", "levels": [level_json(2000.0, "dotted")] })
            .to_string(),
    );
    let (prims, points) = frame_cost(&mut chart);
    assert!(
        prims - base_prims < 2_000 && points - base_points < 4_000,
        "{} prims, {} points",
        prims - base_prims,
        points - base_points
    );
    chart.remove_drawing(fan);

    // A fixed square grown downward by 20 bars at a million price units per bar, with a dashed
    // grid and a dotted arc 3000 sides out: crisp lines stay inside the pane (executors dash them
    // pixel by pixel from their start) and the frame stays small.
    let square = add(
        &mut chart,
        DrawingKind::GannSquareFixed,
        vec![p(10.0, 101.0)],
        &serde_json::json!({
            "levels": [level_json(0.0, "dashed"), level_json(1.0, "dashed")],
            "tool_options": { "gann": {
                "reverse": true, "scale_ratio": 1e6, "arcs": [level_json(3000.0, "dotted")]
            } }
        })
        .to_string(),
    );
    let (prims, _) = frame_cost(&mut chart);
    assert!(prims - base_prims < 2_000, "{} prims", prims - base_prims);
    let frame = chart.build_frame();
    let ink = color(INK);
    let vlines = frame.panes[0]
        .main
        .iter()
        .filter_map(|prim| match prim {
            Prim::VLine { y0, y1, color, .. } if *color == ink => Some((*y0, *y1)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(vlines.len(), 2, "{vlines:?}");
    for (y0, y1) in vlines {
        assert!(
            f64::from(y0) >= pane_top - 12.0 && f64::from(y1) <= pane_bottom.ceil(),
            "({y0}, {y1}) inside [{pane_top}, {pane_bottom}]"
        );
    }
    chart.remove_drawing(square);

    // Levels whose geometry overflows even f64 leave no non-finite coordinate in the frame.
    for kind in [
        DrawingKind::AndrewsPitchfork,
        DrawingKind::Pitchfan,
        DrawingKind::GannBox,
        DrawingKind::GannFan,
    ] {
        add(
            &mut chart,
            kind,
            anchors(kind),
            &serde_json::json!({
                "extend_right": false,
                "levels": [level_json(0.5, "solid"), level_json(1e307, "dashed")]
            })
            .to_string(),
        );
    }
    add(
        &mut chart,
        DrawingKind::GannSquare,
        anchors(DrawingKind::GannSquare),
        &serde_json::json!({ "tool_options": { "gann": {
            "arcs": [level_json(0.5, "solid"), level_json(1e307, "dotted")]
        } } })
        .to_string(),
    );
    let frame = chart.build_frame();
    let pane = &frame.panes[0];
    assert!(pane
        .points
        .iter()
        .all(|point| point[0].is_finite() && point[1].is_finite()));
    assert!(pane.main.iter().all(|prim| match prim {
        Prim::Text { x, y, .. } => x.is_finite() && y.is_finite(),
        _ => true,
    }));
}

#[test]
fn clipped_dashed_lines_keep_their_dash_phase() {
    let mut chart = chart();
    // Anchors left of the viewport: the tines start off-screen and run into it.
    let id = add(
        &mut chart,
        DrawingKind::AndrewsPitchfork,
        vec![p(-30.0, 101.0), p(-12.0, 104.0), p(-4.0, 102.0)],
        &serde_json::json!({
            "fill_enabled": false,
            "color": "#ff00ff",
            "levels": [{
                "value": 1, "color": INK, "visible": true, "style": "dashed",
                "fill_between": false, "label_visible": false
            }]
        })
        .to_string(),
    );
    let (b, c) = (anchor(&chart, id, 1), anchor(&chart, id, 2));
    let a = anchor(&chart, id, 0);
    let direction = sub(mid(b, c), a);
    let length = direction.0.hypot(direction.1);
    let unit = (direction.0 / length, direction.1 / length);
    // Dashed at width 1: 6 on, 6 off.
    let mut checked = 0;
    for base in [b, c] {
        for (points, ..) in polylines(&mut chart, INK) {
            let start = points[0];
            let along = (start.0 - base.0) * unit.0 + (start.1 - base.1) * unit.1;
            let across = (start.0 - base.0) * unit.1 - (start.1 - base.1) * unit.0;
            if across.abs() > 0.5 || start.0 < 1.0 {
                continue;
            }
            let phase = along.rem_euclid(12.0);
            assert!(
                phase < 1e-3 || phase > 12.0 - 1e-3,
                "a dash inside the pane starts {along} px along its tine"
            );
            checked += 1;
        }
    }
    assert!(checked > 10, "{checked} dashes checked");

    // A dashed crisp level line starting left of the pane clamps to it a whole dash period from
    // its own start, so executors dash it in phase while it scrolls.
    chart.remove_drawing(id);
    let gann_box = add(
        &mut chart,
        DrawingKind::GannBox,
        vec![p(-20.3, 101.0), p(10.0, 105.0)],
        &serde_json::json!({
            "fill_enabled": false,
            "levels": [{
                "value": 0.5, "color": INK, "visible": true, "style": "dashed",
                "fill_between": false, "label_visible": false
            }]
        })
        .to_string(),
    );
    let left = anchor(&chart, gann_box, 0).0.round() as i32;
    assert!(left < -12);
    let (hlines, _) = crisp_lines(&mut chart, &[INK]);
    assert_eq!(hlines.len(), 1, "{hlines:?}");
    let (_, x0, x1, _) = hlines[0];
    assert!((-11..=0).contains(&x0), "{x0}");
    assert_eq!((x0 - left) % 12, 0, "{x0} is whole periods from {left}");
    assert_eq!(x1, anchor(&chart, gann_box, 1).0.round() as i32);
}

#[test]
fn zone_fills_are_drag_targets_only_while_selected() {
    let mut chart = chart();
    // A pitchfork's zone between the median and the 0.5 tines.
    let fork = add(
        &mut chart,
        DrawingKind::AndrewsPitchfork,
        anchors(DrawingKind::AndrewsPitchfork),
        "{}",
    );
    let (a, b, c) = (
        anchor(&chart, fork, 0),
        anchor(&chart, fork, 1),
        anchor(&chart, fork, 2),
    );
    let (center, half) = (mid(b, c), sub(c, mid(b, c)));
    let direction = sub(center, a);
    let inside = (
        center.0 + half.0 * 0.25 + direction.0 * 0.5,
        center.1 + half.1 * 0.25 + direction.1 * 0.5,
    );
    // A Gann fan's sector between its 1×1 and 1×2 lines.
    let fan = add(
        &mut chart,
        DrawingKind::GannFan,
        vec![p(24.0, 101.0), p(32.0, 104.0)],
        r#"{"extend_right":false}"#,
    );
    let (o, n) = (anchor(&chart, fan, 0), anchor(&chart, fan, 1));
    let sector = (o.0 + (n.0 - o.0) * 0.6, o.1 + (n.1 - o.1) * 0.9);
    chart.build_frame();
    for (id, probe) in [(fork, inside), (fan, sector)] {
        chart.set_selected_drawing(None);
        assert_eq!(chart.hit_test_drawing(probe.0, probe.1), None, "{probe:?}");
        chart.set_selected_drawing(Some(id));
        let hit = chart.hit_test_drawing(probe.0, probe.1);
        assert_eq!(
            hit.map(|hit| (hit.id, hit.part)),
            Some((id, DrawingDragPart::Body))
        );
        assert_eq!(hit, chart.hit_test_drawing_bruteforce(probe.0, probe.1));
    }
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

#[test]
fn anchors_in_the_future_area_paint_hit_and_keep_their_times() {
    let mut chart = chart();
    crowd(&mut chart);
    // The data ends at logical 39; the view shows a few bars of future area.
    chart.set_visible_logical_range(20.0, 50.0);
    let fork = add(
        &mut chart,
        DrawingKind::AndrewsPitchfork,
        vec![p(30.0, 101.0), p(38.0, 105.0), p(44.0, 102.0)],
        &ink_levels(&[1.0]),
    );
    let square = add(
        &mut chart,
        DrawingKind::GannSquareFixed,
        vec![p(41.0, 102.0)],
        r#"{"tool_options":{"gann":{"size_bars":6,"scale_ratio":0.5}}}"#,
    );
    chart.build_frame();
    assert!(viewport_candidate(&chart, fork) && viewport_candidate(&chart, square));
    let times = chart.drawing_anchors(fork).unwrap();
    assert_eq!(
        times[2].time,
        Some(44.0 * HOUR),
        "extrapolated at the bar interval"
    );
    assert!(!polylines(&mut chart, INK).is_empty());
    let mut hits = std::collections::HashSet::new();
    for gy in 0..48 {
        for gx in 0..78 {
            let (x, y) = (f64::from(gx) * 10.0 + 3.0, f64::from(gy) * 10.0 + 4.0);
            let hit = chart.hit_test_drawing(x, y);
            assert_eq!(hit, chart.hit_test_drawing_bruteforce(x, y), "({x}, {y})");
            hits.extend(hit.map(|hit| hit.id));
        }
    }
    assert!(hits.contains(&fork) && hits.contains(&square), "{hits:?}");
}
