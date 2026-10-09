//! Patterns, Elliott waves, and cycles engine tests: catalog defaults, armed and progressive
//! placement, shared-part frames (ratios, fills, necklines, apexes, degree labels, repeats) and
//! hit testing (indexed and brute force, on log and percentage scales and lower panes), culling
//! that never drops a painted part, drags, keyboard nudges, magnet, time identity, schema, kind
//! options, templates, patches with history, persistence (old documents included), clipboard,
//! sync, and bounded repeats at extreme zoom.

use aeris_charts_render::color::Color;
use aeris_charts_render::draw_list::Prim;

use super::super::super::DrawingTextLayout;
use crate::{
    ChartEngine, DrawingAnchor, DrawingId, DrawingKind, DrawingMagnetMode, DrawingModifiers,
    DrawingPoint, DrawingPriceScale,
};

const KINDS: [DrawingKind; 14] = [
    DrawingKind::PatternXabcd,
    DrawingKind::PatternCypher,
    DrawingKind::PatternAbcd,
    DrawingKind::PatternHeadShoulders,
    DrawingKind::PatternTriangle,
    DrawingKind::PatternThreeDrives,
    DrawingKind::ElliottImpulse,
    DrawingKind::ElliottCorrection,
    DrawingKind::ElliottTriangle,
    DrawingKind::ElliottDoubleCombination,
    DrawingKind::ElliottTripleCombination,
    DrawingKind::CyclicLines,
    DrawingKind::TimeCycles,
    DrawingKind::SineLine,
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

/// A zigzag alternating between highs and lows, one anchor per defining point.
fn zigzag_points(kind: DrawingKind) -> Vec<DrawingPoint> {
    (0..kind.anchor_count())
        .map(|index| {
            let high = index % 2 == 1;
            p(
                6.0 + index as f64 * 3.0,
                if high { 104.5 } else { 101.0 } + index as f64 * 0.1,
            )
        })
        .collect()
}

fn anchor(chart: &ChartEngine, id: DrawingId, index: usize) -> (f64, f64) {
    chart.drawing_point_to_coordinate(id, index).unwrap()
}

fn ink() -> Color {
    Color::parse_css(INK).unwrap()
}

/// Every text run of the first pane: text, left x, and first-line center y.
fn texts(chart: &mut ChartEngine) -> Vec<(String, f64, f64)> {
    let frame = chart.build_frame();
    frame.panes[0]
        .main
        .iter()
        .filter_map(|prim| match prim {
            Prim::Text { text, x, y, .. } => Some((text.clone(), f64::from(*x), f64::from(*y))),
            _ => None,
        })
        .collect()
}

fn texts_of(chart: &mut ChartEngine) -> Vec<String> {
    texts(chart).into_iter().map(|(text, ..)| text).collect()
}

fn close(a: (f64, f64), b: (f64, f64), tolerance: f64) -> bool {
    (a.0 - b.0).abs() <= tolerance && (a.1 - b.1).abs() <= tolerance
}

/// The fork's pre-merge pattern, Elliott, and cycle defaults, which documents it wrote omitted,
/// come back through `apply_legacy_fork_defaults` (upstream renders these tools now).
#[test]
fn catalog_defaults_follow_each_tool() {
    for kind in KINDS {
        let mut drawing = crate::Drawing::new(1, kind, 0, Vec::new());
        super::super::apply_legacy_fork_defaults(&mut drawing);
        let spec = kind.spec();
        assert!(spec.family.is_none(), "{kind:?} is upstream-rendered");
        assert_eq!(spec.text_layout, DrawingTextLayout::Box);
        assert!(!spec.axis_price_label);
        let (color, fill, width) = match kind {
            DrawingKind::PatternXabcd | DrawingKind::PatternCypher => ("#2962FF", true, 2.0),
            DrawingKind::PatternAbcd => ("#089981", false, 2.0),
            DrawingKind::PatternHeadShoulders => ("#089981", true, 2.0),
            DrawingKind::PatternTriangle => ("#673AB7", true, 2.0),
            DrawingKind::PatternThreeDrives => ("#673AB7", false, 2.0),
            DrawingKind::ElliottImpulse | DrawingKind::ElliottCorrection => ("#3D85C6", false, 2.0),
            DrawingKind::ElliottTriangle => ("#FF9800", false, 2.0),
            DrawingKind::ElliottDoubleCombination | DrawingKind::ElliottTripleCombination => {
                ("#6AA84F", false, 2.0)
            }
            DrawingKind::CyclicLines => ("#80CCDB", false, 1.0),
            DrawingKind::TimeCycles => ("#159980", true, 2.0),
            _ => ("#159980", false, 2.0),
        };
        assert_eq!(
            (drawing.color.as_str(), drawing.fill_enabled, drawing.width),
            (color, fill, width),
            "{kind:?}"
        );
        assert!(!drawing.extend_left && !drawing.extend_right);
        assert!(drawing.labels.is_empty() && drawing.tool_options.is_empty());
    }
}

#[test]
fn armed_tools_place_every_family_kind() {
    let mut chart = chart();
    for kind in KINDS {
        assert!(chart.set_drawing_tool(Some(kind), Some(r##"{"color":"#123456"}"##), None));
        let mut created = None;
        for index in 0..kind.anchor_count() {
            let (x, y) = (
                120.0 + index as f64 * 70.0,
                300.0 - (index % 2) as f64 * 120.0,
            );
            let update = chart.drawing_tool_activate(x, y, DrawingModifiers::default());
            assert!(update.consumed);
            if index + 1 < kind.anchor_count() {
                assert_eq!(update.created, None, "{kind:?} waits for every anchor");
            }
            created = update.created;
        }
        let id = created.unwrap_or_else(|| panic!("{kind:?} committed"));
        let drawing = chart.drawing(id).unwrap();
        assert_eq!(drawing.kind, kind);
        assert_eq!(drawing.points.len(), kind.anchor_count());
        assert_eq!(drawing.color, INK);
        assert_eq!(chart.active_drawing_tool(), None, "one-shot tools disarm");
        assert_eq!(chart.selected_drawing(), Some(id));
        assert_eq!(chart.drawing_handle_count(id), Some(kind.anchor_count()));
    }
}

#[test]
fn indexed_hit_testing_matches_brute_force() {
    let mut chart = chart();
    for copy in 0..2 {
        let shift = copy as f64 * 0.7;
        for kind in KINDS {
            let points = zigzag_points(kind)
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
    for gy in 0..24 {
        for gx in 0..39 {
            let (x, y) = (f64::from(gx) * 20.0 + 3.0, f64::from(gy) * 20.0 + 4.0);
            let indexed = chart.hit_test_drawing(x, y);
            assert_eq!(
                indexed,
                chart.hit_test_drawing_bruteforce(x, y),
                "({x}, {y})"
            );
            hits += usize::from(indexed.is_some());
        }
    }
    assert!(hits > 50, "the grid meets the drawings ({hits} hits)");
}

#[test]
fn drags_nudges_and_undo_edit_patterns_as_single_history_entries() {
    let mut chart = chart();
    let id = add(
        &mut chart,
        DrawingKind::PatternXabcd,
        zigzag_points(DrawingKind::PatternXabcd),
        "{}",
    );
    let before = chart.drawing(id).unwrap().points.clone();
    chart.set_selected_drawing(Some(id));
    let (bx, by) = anchor(&chart, id, 2);
    assert!(chart.drawing_drag_start_at(bx, by));
    chart.drawing_drag_to(bx + 20.0, by + 10.0, DrawingModifiers::default());
    chart.drawing_drag_end();
    let after = chart.drawing(id).unwrap().points.clone();
    for index in [0, 1, 3, 4] {
        assert_eq!(after[index], before[index], "only anchor 2 moves");
    }
    assert!(close(anchor(&chart, id, 2), (bx + 20.0, by + 10.0), 1e-6));
    assert!(chart.undo_drawing());
    assert_eq!(chart.drawing(id).unwrap().points, before);

    // Body drag from the first leg moves every anchor rigidly.
    let (x0, x1) = (anchor(&chart, id, 0), anchor(&chart, id, 1));
    let grab = ((x0.0 + x1.0) / 2.0, (x0.1 + x1.1) / 2.0);
    chart.set_selected_drawing(None);
    assert!(chart.drawing_drag_start_at(grab.0, grab.1));
    chart.drawing_drag_to(grab.0 + 30.0, grab.1 - 12.0, DrawingModifiers::default());
    chart.drawing_drag_end();
    for (index, &point) in before.iter().enumerate() {
        let moved = anchor(&chart, id, index);
        let original = chart
            .drawing_to_px_for(0, DrawingPriceScale::Right, point)
            .unwrap();
        assert!(close(moved, (original.0 + 30.0, original.1 - 12.0), 1e-6));
    }
    assert!(chart.undo_drawing());

    // Keyboard: every anchor is a handle; a nudge moves one handle.
    assert_eq!(chart.drawing_handle_count(id), Some(5));
    chart.set_selected_drawing(Some(id));
    let (x3, y3) = anchor(&chart, id, 3);
    assert!(chart.nudge_selected_drawing(10.0, 0.0, Some(3)));
    let (nx, ny) = anchor(&chart, id, 3);
    assert!((nx - x3 - 10.0).abs() < 1e-6 && (ny - y3).abs() < 1e-6);
    assert!(chart.undo_drawing());
    assert_eq!(chart.drawing(id).unwrap().points, before);

    // Locked drawings stay selectable but do not drag; hidden ones neither paint nor hit.
    assert!(chart.set_drawing_locked(id, true));
    let (lx, ly) = anchor(&chart, id, 1);
    if chart.drawing_drag_start_at(lx, ly) {
        chart.drawing_drag_to(lx + 25.0, ly, DrawingModifiers::default());
        chart.drawing_drag_end();
    }
    assert_eq!(chart.drawing(id).unwrap().points, before);
    assert!(chart.set_drawing_visibility(id, false));
    chart.set_selected_drawing(None);
    assert!(texts_of(&mut chart).is_empty());
    assert_eq!(chart.hit_test_drawing(grab.0, grab.1), None);
}

#[test]
fn z_order_decides_which_overlapping_pattern_hits() {
    let mut chart = chart();
    let points = zigzag_points(DrawingKind::PatternAbcd);
    let below = add(&mut chart, DrawingKind::PatternAbcd, points.clone(), "{}");
    let above = add(&mut chart, DrawingKind::ElliottCorrection, points, "{}");
    let (a, b) = (anchor(&chart, below, 0), anchor(&chart, below, 1));
    let leg = ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0);
    assert_eq!(
        chart.hit_test_drawing(leg.0, leg.1).map(|hit| hit.id),
        Some(above)
    );
    assert!(chart.move_drawing_z_order(above, -1));
    assert_eq!(
        chart.hit_test_drawing(leg.0, leg.1).map(|hit| hit.id),
        Some(below)
    );
    assert!(chart.undo_drawing());
    assert_eq!(
        chart.hit_test_drawing(leg.0, leg.1).map(|hit| hit.id),
        Some(above)
    );
}

#[test]
fn the_chart_magnet_snaps_every_placed_anchor() {
    let mut chart = chart();
    chart.set_drawing_magnet_mode(DrawingMagnetMode::Strong);
    assert!(chart.set_drawing_tool(Some(DrawingKind::ElliottCorrection), None, None));
    let mut created = None;
    for (index, logical) in [8.0, 12.0, 16.0, 20.0].into_iter().enumerate() {
        let x = chart.logical_to_coordinate(logical).unwrap() + 3.0;
        let y = chart
            .series_price_to_coordinate(0, 102.4 + index as f64 * 0.3)
            .unwrap();
        created = chart
            .drawing_tool_activate(x, y, DrawingModifiers::default())
            .created;
    }
    let drawing = chart.drawing(created.unwrap()).unwrap();
    for (point, logical) in drawing.points.iter().zip([8.0, 12.0, 16.0, 20.0]) {
        assert_eq!(point.logical, logical);
        assert_eq!(point.price, 100.0 + (logical as usize % 7) as f64);
    }
}

#[test]
fn anchors_resolve_by_time_across_an_interval_switch() {
    let mut chart = chart();
    let anchors =
        [(10.0, 101.0), (14.5, 104.0), (18.0, 102.0), (22.0, 105.0)].map(|(hours, price)| {
            DrawingAnchor {
                logical: None,
                price,
                time: Some(hours * HOUR),
            }
        });
    let id = chart
        .add_drawing_anchors(DrawingKind::PatternAbcd, 0, &anchors, None)
        .unwrap();
    // A cycle anchored in the future area beyond the last bar keeps its times too.
    let future = add(
        &mut chart,
        DrawingKind::CyclicLines,
        vec![p(45.0, 101.0), p(50.0, 101.0)],
        "{}",
    );
    let half_hourly = (0..80)
        .map(|index| index as f64 * HOUR / 2.0)
        .collect::<Vec<_>>();
    let values = vec![100.0; half_hourly.len()];
    chart
        .set_series_data(0, &half_hourly, &values, &values, &values, &values)
        .unwrap();
    let resolved = chart.drawing_anchors(id).unwrap();
    let logicals = resolved
        .iter()
        .map(|anchor| anchor.logical)
        .collect::<Vec<_>>();
    assert_eq!(logicals, [Some(20.0), Some(29.0), Some(36.0), Some(44.0)]);
    assert_eq!(resolved[1].time, Some(14.5 * HOUR));
    let future = chart.drawing_anchors(future).unwrap();
    assert_eq!(
        future
            .iter()
            .map(|anchor| anchor.logical)
            .collect::<Vec<_>>(),
        [Some(90.0), Some(100.0)]
    );
    assert_eq!(future[1].time, Some(50.0 * HOUR));
}

#[test]
fn family_tools_paint_and_hit_on_a_lower_pane() {
    let mut chart = chart();
    let pane = chart.add_pane(true).unwrap();
    let series = chart.add_series(crate::SeriesKind::Line);
    let values = (0..40)
        .map(|index| 10.0 + (index % 5) as f64 * 0.5)
        .collect::<Vec<_>>();
    chart
        .set_series_data(series, &hourly(40), &values, &values, &values, &values)
        .unwrap();
    chart.set_series_pane(series, pane, 1.0);
    chart.build_frame();
    let (top, bottom) = (
        chart.panes[pane].top,
        chart.panes[pane].top + chart.panes[pane].height,
    );
    let points = vec![
        p(6.0, 10.2),
        p(10.0, 11.8),
        p(14.0, 10.6),
        p(18.0, 11.5),
        p(22.0, 10.4),
    ];
    let pattern = chart
        .add_drawing(
            DrawingKind::PatternXabcd,
            pane,
            points,
            Some(r##"{"color":"#123456"}"##),
        )
        .unwrap();
    let cycles = chart
        .add_drawing(
            DrawingKind::CyclicLines,
            pane,
            vec![p(9.0, 11.0), p(13.0, 11.0)],
            Some(r##"{"color":"#123456"}"##),
        )
        .unwrap();
    let frame = chart.build_frame();
    // Point labels and repeats paint on the drawing's own pane, and the repeats span only it.
    let label_y = frame.panes[pane]
        .main
        .iter()
        .find_map(|prim| match prim {
            Prim::Text { text, y, .. } if text == "A" => Some(f64::from(*y)),
            _ => None,
        })
        .expect("the A label on the lower pane");
    assert!(label_y > top && label_y < bottom);
    assert!(!frame.panes[0]
        .main
        .iter()
        .any(|prim| matches!(prim, Prim::Text { text, .. } if text == "A")));
    let repeats = frame.panes[pane]
        .main
        .iter()
        .filter_map(|prim| match prim {
            Prim::VLine { y0, y1, color, .. } if *color == ink() => Some((*y0, *y1)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(repeats.len() > 5);
    assert!(repeats
        .iter()
        .all(|&(y0, y1)| y0 == top.round() as i32 && y1 == bottom.round() as i32));
    // Legs, labels, and far repeats hit on that pane.
    let (a, b) = (anchor(&chart, pattern, 0), anchor(&chart, pattern, 1));
    assert!(a.1 > top && b.1 > top);
    let leg = ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0);
    assert_eq!(
        chart.hit_test_drawing(leg.0, leg.1).map(|hit| hit.id),
        Some(pattern)
    );
    let x = chart.logical_to_coordinate(33.0).unwrap();
    assert_eq!(
        chart
            .hit_test_drawing(x, (top + bottom) / 2.0)
            .map(|hit| hit.id),
        Some(cycles)
    );
    assert_eq!(chart.hit_test_drawing(x, top / 2.0), None);
}

#[test]
fn family_tools_tolerate_charts_without_data_and_degenerate_anchors() {
    let mut empty = ChartEngine::new(800.0, 500.0, 1.0);
    for kind in KINDS {
        assert!(empty
            .add_drawing(kind, 0, zigzag_points(kind), None)
            .is_some());
    }
    empty.build_frame();
    assert_eq!(empty.hit_test_drawing(100.0, 100.0), None);

    let mut chart = chart();
    assert!(chart
        .add_drawing(
            DrawingKind::SineLine,
            0,
            vec![p(f64::NAN, 1.0), p(2.0, 3.0)],
            None
        )
        .is_none());
    assert!(
        chart
            .add_drawing(
                DrawingKind::PatternXabcd,
                0,
                zigzag_points(DrawingKind::PatternAbcd),
                None
            )
            .is_none(),
        "a pattern needs its exact anchor count"
    );
    for kind in KINDS {
        let points = vec![p(12.0, 102.0); kind.anchor_count()];
        assert!(chart.add_drawing(kind, 0, points, None).is_some());
    }
    let frame = chart.build_frame();
    assert!(frame.panes[0]
        .points
        .iter()
        .all(|point| point[0].is_finite() && point[1].is_finite()));
}

#[test]
fn log_and_percentage_scales_keep_indexed_hits_equal_to_brute_force() {
    use crate::PriceScaleMode;
    for mode in [
        PriceScaleMode::Logarithmic,
        PriceScaleMode::Percentage,
        PriceScaleMode::IndexedTo100,
    ] {
        let mut chart = chart();
        for kind in KINDS {
            add(&mut chart, kind, zigzag_points(kind), "{}");
            // Non-positive prices clamp on a log scale instead of producing non-finite geometry.
            let points = (0..kind.anchor_count())
                .map(|index| p(20.0 + index as f64 * 2.0, -(index as f64)))
                .collect();
            add(&mut chart, kind, points, "{}");
        }
        chart.set_price_scale_mode(0, false, mode);
        let frame = chart.build_frame();
        assert!(frame.panes[0]
            .points
            .iter()
            .all(|point| point[0].is_finite() && point[1].is_finite()));
        for gy in 0..16 {
            for gx in 0..26 {
                let (x, y) = (f64::from(gx) * 30.0 + 3.0, f64::from(gy) * 30.0 + 4.0);
                assert_eq!(
                    chart.hit_test_drawing(x, y),
                    chart.hit_test_drawing_bruteforce(x, y),
                    "{mode:?} ({x}, {y})"
                );
            }
        }
    }
}
