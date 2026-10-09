//! Patterns, Elliott waves, and cycles engine tests: catalog defaults, armed and progressive
//! placement, shared-part frames (ratios, fills, necklines, apexes, degree labels, repeats) and
//! hit testing (indexed and brute force, on log and percentage scales and lower panes), culling
//! that never drops a painted part, drags, keyboard nudges, magnet, time identity, schema, kind
//! options, templates, patches with history, persistence (old documents included), clipboard,
//! sync, and bounded repeats at extreme zoom.

use aeris_charts_render::color::Color;
use aeris_charts_render::draw_list::{LineStyle, Prim};

use super::super::super::DrawingTextLayout;
use super::{ElliottWaveDegree, WaveMark};
use crate::{
    ChartEngine, DrawingAnchor, DrawingId, DrawingKind, DrawingMagnetMode, DrawingModifiers,
    DrawingPoint, DrawingPriceScale, HitProfile,
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

fn ink_fill() -> Color {
    Color::rgba(0x12, 0x34, 0x56, super::FILL_ALPHA)
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

fn ink_fills(chart: &mut ChartEngine) -> usize {
    let frame = chart.build_frame();
    frame.panes[0]
        .main
        .iter()
        .filter(|prim| matches!(prim, Prim::BandFill { fill, .. } if *fill == ink_fill()))
        .count()
}

fn text_y(chart: &mut ChartEngine, text: &str) -> f64 {
    texts(chart)
        .into_iter()
        .find(|(run, ..)| run == text)
        .unwrap_or_else(|| panic!("{text} is painted"))
        .2
}

/// Total length of the thin (1 CSS px at DPR 1) solid runs lying on segment `a → b`: a dashed
/// connector reaches the frame pre-split into its dashes.
fn dashes_on(lines: &[InkLine], a: (f64, f64), b: (f64, f64)) -> f64 {
    lines
        .iter()
        .filter(|(points, width, style)| {
            *width == 1.0
                && *style == LineStyle::Solid
                && points.len() == 2
                && points.iter().all(|&point| {
                    aeris_charts_render::shape::distance_to_segment(point, a, b) < 0.01
                })
        })
        .map(|(points, ..)| (points[1].0 - points[0].0).hypot(points[1].1 - points[0].1))
        .sum()
}

/// Whether `a → b` is painted as a dashed connector: about half its length in dashes.
fn dashed_between(lines: &[InkLine], a: (f64, f64), b: (f64, f64)) -> bool {
    let coverage = dashes_on(lines, a, b) / (b.0 - a.0).hypot(b.1 - a.1);
    (0.35..=0.65).contains(&coverage)
}

fn px(chart: &ChartEngine, logical: f64, price: f64) -> (f64, f64) {
    chart
        .drawing_to_px_for(0, DrawingPriceScale::Right, p(logical, price))
        .unwrap()
}

/// Enough unrelated drawings that hit testing takes the culled candidate path.
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

fn hit(chart: &ChartEngine, (x, y): (f64, f64)) -> Option<DrawingId> {
    let indexed = chart.hit_test_drawing(x, y);
    assert_eq!(
        indexed,
        chart.hit_test_drawing_bruteforce(x, y),
        "({x}, {y})"
    );
    indexed.map(|hit| hit.id)
}

const HEAD_AND_SHOULDERS: [(f64, f64); 7] = [
    (4.0, 101.0),
    (8.0, 104.0),
    (12.0, 102.0),
    (16.0, 106.0),
    (20.0, 102.0),
    (24.0, 104.0),
    (28.0, 101.0),
];

fn points_of(anchors: &[(f64, f64)]) -> Vec<DrawingPoint> {
    anchors
        .iter()
        .map(|&(logical, price)| p(logical, price))
        .collect()
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
        // The fork drew a triangle pattern's apex sides; the extend flags select them.
        let apex = kind == DrawingKind::PatternTriangle;
        assert_eq!(
            (drawing.extend_left, drawing.extend_right),
            (apex, apex),
            "{kind:?}"
        );
        assert!(drawing.labels.is_empty() && drawing.tool_options.is_empty());
        assert_eq!(
            super::super::legacy_fork_tool_options(kind),
            None,
            "{kind:?}"
        );
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

/// The Frost-Prechter notation of every degree (owner decision P4).
#[test]
fn elliott_degrees_label_with_their_notation() {
    let impulse = |degree: ElliottWaveDegree| {
        (1..=5)
            .map(|number| degree.label(WaveMark::Number(number)))
            .collect::<Vec<_>>()
    };
    let owned = |labels: [&str; 5], ring: bool| {
        labels
            .into_iter()
            .map(|label| (label.to_string(), ring))
            .collect::<Vec<_>>()
    };
    use ElliottWaveDegree::*;
    assert_eq!(
        impulse(Supermillennium),
        owned(["{I}", "{II}", "{III}", "{IV}", "{V}"], false)
    );
    assert_eq!(
        impulse(Millennium),
        owned(["[I]", "[II]", "[III]", "[IV]", "[V]"], false)
    );
    assert_eq!(
        impulse(Submillennium),
        owned(["<I>", "<II>", "<III>", "<IV>", "<V>"], false)
    );
    assert_eq!(
        impulse(GrandSupercycle),
        owned(["I", "II", "III", "IV", "V"], true)
    );
    assert_eq!(
        impulse(Supercycle),
        owned(["(I)", "(II)", "(III)", "(IV)", "(V)"], false)
    );
    assert_eq!(impulse(Cycle), owned(["I", "II", "III", "IV", "V"], false));
    assert_eq!(impulse(Primary), owned(["1", "2", "3", "4", "5"], true));
    assert_eq!(
        impulse(Intermediate),
        owned(["(1)", "(2)", "(3)", "(4)", "(5)"], false)
    );
    assert_eq!(impulse(Minor), owned(["1", "2", "3", "4", "5"], false));
    assert_eq!(impulse(Minute), owned(["i", "ii", "iii", "iv", "v"], true));
    assert_eq!(
        impulse(Minuette),
        owned(["(i)", "(ii)", "(iii)", "(iv)", "(v)"], false)
    );
    assert_eq!(
        impulse(Subminuette),
        owned(["i", "ii", "iii", "iv", "v"], false)
    );
    let letter = |degree: ElliottWaveDegree, letter| degree.label(WaveMark::Letter(letter));
    assert_eq!(letter(Primary, 'a'), ("A".to_string(), true));
    assert_eq!(letter(Intermediate, 'W'), ("(W)".to_string(), false));
    assert_eq!(letter(Minor, 'z'), ("Z".to_string(), false));
    assert_eq!(letter(Cycle, 'B'), ("b".to_string(), false));
    assert_eq!(letter(Minuette, 'X'), ("(x)".to_string(), false));
    assert_eq!(letter(Supermillennium, 'C'), ("{c}".to_string(), false));
    // Every degree labels distinctly, and each is named by its upstream `wave_degree` value.
    let impulses = ElliottWaveDegree::ALL.map(impulse);
    for (index, labels) in impulses.iter().enumerate() {
        assert!(impulses[index + 1..].iter().all(|other| other != labels));
    }
    for degree in ElliottWaveDegree::ALL {
        let name = serde_json::to_value(degree).unwrap();
        assert!(DrawingKind::valid_wave_degree(name.as_str().unwrap()));
        assert_eq!(
            ElliottWaveDegree::from_name(name.as_str().unwrap()),
            Some(degree)
        );
    }
}

/// Harmonic ratio connectors and their boxed ratios paint by default (owner decision P6:
/// `show_ratios` defaults to true) after the zigzag, every connector before every ratio, and
/// `show_ratios: false` removes both.
#[test]
fn xabcd_draws_dashed_ratio_connectors_and_ratios_by_default() {
    let mut chart = chart();
    crowd(&mut chart);
    let id = add(
        &mut chart,
        DrawingKind::PatternXabcd,
        vec![
            p(6.0, 100.0),
            p(10.0, 105.0),
            p(14.0, 101.91),
            p(18.0, 104.0),
            p(22.0, 100.5),
        ],
        r##"{"color":"#123456","width":2}"##,
    );
    let runs = texts_of(&mut chart);
    for ratio in ["0.618", "0.676", "1.675", "0.900"] {
        assert!(runs.iter().any(|run| run == ratio), "{ratio} in {runs:?}");
    }
    let lines = ink_polylines(&mut chart);
    assert_eq!(lines[0].0.len(), 5, "the zigzag passes every anchor");
    assert_eq!((lines[0].1, lines[0].2), (2.0, LineStyle::Solid));
    // Connectors XB, AC, BD, and XD, each pre-split into solid dashes.
    let [x, a, b, c, d] = [0, 1, 2, 3, 4].map(|index| anchor(&chart, id, index));
    for (from, to) in [(x, b), (a, c), (b, d), (x, d)] {
        assert!(dashed_between(&lines, from, to), "{from:?} → {to:?}");
    }
    // Z-order: zigzag, connectors, ratio boxes, then the vertex labels.
    let frame = chart.build_frame();
    let main = &frame.panes[0].main;
    let last_connector = main.iter().rposition(
        |prim| matches!(prim, Prim::Polyline { width, color, .. } if *width == 1.0 && *color == ink()),
    );
    let first_box = main
        .iter()
        .position(|prim| matches!(prim, Prim::Rect { color, .. } if *color == ink()));
    let first_vertex_label = main
        .iter()
        .position(|prim| matches!(prim, Prim::Text { text, .. } if text == "X"));
    assert!(last_connector.is_some() && last_connector < first_box);
    assert!(first_box < first_vertex_label);
    // The XD connector beyond the zigzag and a ratio box are body targets.
    let on_xd = (x.0 + (d.0 - x.0) * 0.3, x.1 + (d.1 - x.1) * 0.3);
    assert_eq!(hit(&chart, on_xd), Some(id));
    let (_, left, y) = texts(&mut chart)
        .into_iter()
        .find(|(text, ..)| text == "0.900")
        .unwrap();
    assert_eq!(hit(&chart, (left + 2.0, y)), Some(id));

    assert!(
        chart.drawing_apply_options(id, r#"{"tool_options":{"pattern":{"show_ratios":false}}}"#)
    );
    assert!(!texts_of(&mut chart).iter().any(|run| run == "0.618"));
    assert!(ink_polylines(&mut chart)
        .iter()
        .all(|(_, width, _)| *width == 2.0));
    assert!(!main_has_rect(&mut chart));
    assert_eq!(hit(&chart, on_xd), None);
}

fn main_has_rect(chart: &mut ChartEngine) -> bool {
    chart.build_frame().panes[0]
        .main
        .iter()
        .any(|prim| matches!(prim, Prim::Rect { color, .. } if *color == ink()))
}

/// Connectors stay dashed under a solid drawing style and reach the executors as solid runs.
#[test]
fn ratio_connectors_reach_executors_as_solid_dash_runs() {
    let mut chart = chart();
    let id = add(
        &mut chart,
        DrawingKind::PatternAbcd,
        vec![p(6.0, 105.0), p(10.0, 101.0), p(14.0, 103.5), p(18.0, 99.5)],
        r##"{"color":"#123456","width":2}"##,
    );
    let frame = chart.build_frame();
    assert!(frame.panes[0].main.iter().all(|prim| !matches!(
        prim,
        Prim::Polyline { style, .. } if *style != LineStyle::Solid
    )));
    let lines = ink_polylines(&mut chart);
    let (a, c) = (anchor(&chart, id, 0), anchor(&chart, id, 2));
    let coverage = dashes_on(&lines, a, c) / (c.0 - a.0).hypot(c.1 - a.1);
    assert!((0.35..=0.65).contains(&coverage), "{coverage}");
    assert!(
        lines.iter().filter(|(_, width, _)| *width == 1.0).count() > 4,
        "the connectors are split into dash runs"
    );
}

#[test]
fn cypher_abcd_and_three_drives_measure_their_conventional_ratios() {
    let mut chart = chart();
    add(
        &mut chart,
        DrawingKind::PatternCypher,
        vec![
            p(6.0, 100.0),
            p(9.0, 105.0),
            p(12.0, 102.0),
            p(15.0, 106.0),
            p(18.0, 101.284),
        ],
        r##"{"color":"#123456"}"##,
    );
    let runs = texts_of(&mut chart);
    for ratio in ["0.600", "1.200", "0.786"] {
        assert!(runs.iter().any(|run| run == ratio), "{ratio} in {runs:?}");
    }
    chart.clear_drawings();

    add(
        &mut chart,
        DrawingKind::PatternAbcd,
        vec![p(6.0, 105.0), p(10.0, 101.0), p(14.0, 103.5), p(18.0, 99.5)],
        r##"{"color":"#123456"}"##,
    );
    let runs = texts_of(&mut chart);
    // BC/AB = 2.5/4, CD/BC = 4/2.5.
    for label in ["0.625", "1.600", "A", "B", "C", "D"] {
        assert!(runs.iter().any(|run| run == label), "{label} in {runs:?}");
    }
    chart.clear_drawings();

    // Upstream's six anchors 0, 1, A, 2, B, 3: each retracement against its drive and each
    // drive against its retracement, drawn across both legs.
    let drives = add(
        &mut chart,
        DrawingKind::PatternThreeDrives,
        vec![
            p(4.0, 100.0),
            p(8.0, 102.0),
            p(11.0, 101.0),
            p(15.0, 103.5),
            p(18.0, 102.5),
            p(22.0, 105.0),
        ],
        r##"{"color":"#123456","width":2}"##,
    );
    let runs = texts_of(&mut chart);
    for label in ["0.500", "2.500", "0.400", "1", "2", "3"] {
        assert!(runs.iter().any(|run| run == label), "{label} in {runs:?}");
    }
    assert_eq!(runs.iter().filter(|run| *run == "2.500").count(), 2);
    let lines = ink_polylines(&mut chart);
    for index in 1..=4 {
        let (from, to) = (
            anchor(&chart, drives, index - 1),
            anchor(&chart, drives, index + 1),
        );
        assert!(dashed_between(&lines, from, to), "connector {index}");
    }
}

/// XABCD and cypher shade X-A-B and B-C-D with `fill_enabled` (upstream's default stays off);
/// the shading is a body target only while the drawing is selected.
#[test]
fn xabcd_and_cypher_shade_their_triangles_with_fill_enabled() {
    for kind in [DrawingKind::PatternXabcd, DrawingKind::PatternCypher] {
        let mut chart = chart();
        let points = vec![
            p(6.0, 100.0),
            p(10.0, 105.0),
            p(14.0, 101.91),
            p(18.0, 104.0),
            p(22.0, 100.5),
        ];
        let id = add(&mut chart, kind, points, r##"{"color":"#123456"}"##);
        assert_eq!(ink_fills(&mut chart), 0, "{kind:?}: upstream's default");
        assert!(chart.drawing_apply_options(id, r#"{"fill_enabled":true}"#));
        assert_eq!(ink_fills(&mut chart), 2, "{kind:?}");
        // Inside X-A-B, clear of every stroke and label.
        let [x, a, b] = [0, 1, 2].map(|index| anchor(&chart, id, index));
        let inside = ((x.0 + a.0 + b.0) / 3.0, (x.1 + 2.0 * a.1 + b.1) / 4.0 + 6.0);
        assert_eq!(hit(&chart, inside), None, "{kind:?}: unselected");
        chart.set_selected_drawing(Some(id));
        assert_eq!(hit(&chart, inside), Some(id), "{kind:?}: selected");
    }
}

/// Every head and shoulders draws its neckline (owner decision P2) from where it meets the
/// first leg to where it meets the last; `fill_enabled` shades the shoulders and the head.
#[test]
fn head_and_shoulders_draws_the_neckline_between_the_outer_legs() {
    let mut chart = chart();
    let id = add(
        &mut chart,
        DrawingKind::PatternHeadShoulders,
        points_of(&HEAD_AND_SHOULDERS),
        r##"{"color":"#123456","width":2}"##,
    );
    let lines = ink_polylines(&mut chart);
    let neckline = lines
        .iter()
        .find(|(points, ..)| points.len() == 2)
        .expect("neckline")
        .0
        .clone();
    let start = px(&chart, 4.0 + 4.0 / 3.0, 102.0);
    let end = px(&chart, 24.0 + 8.0 / 3.0, 102.0);
    assert!(close(neckline[0], start, 0.01), "{neckline:?} vs {start:?}");
    assert!(close(neckline[1], end, 0.01));
    assert_eq!(ink_fills(&mut chart), 0);
    // The neckline beyond the neck anchors is a body target.
    let beyond = ((start.0 + px(&chart, 12.0, 102.0).0) / 2.0, start.1);
    assert_eq!(hit(&chart, beyond), Some(id));
    assert!(chart.drawing_apply_options(id, r#"{"fill_enabled":true}"#));
    assert_eq!(ink_fills(&mut chart), 3, "both shoulders and the head");
    // The inverse pattern draws its neckline too.
    let mirrored =
        HEAD_AND_SHOULDERS.map(|(logical, price)| DrawingAnchor::from(p(logical, 206.0 - price)));
    assert!(chart.set_drawing_anchors(id, &mirrored).is_ok());
    let start = px(&chart, 4.0 + 4.0 / 3.0, 104.0);
    assert!(ink_polylines(&mut chart)
        .iter()
        .any(|(points, ..)| points.len() == 2 && close(points[0], start, 0.01)));
}

/// The triangle pattern's sides reach their apex only with the extend flag of their direction
/// (owner decision P3); fork documents set both.
#[test]
fn triangle_pattern_extends_converging_sides_to_their_apex_when_extended() {
    let mut chart = chart();
    let converging = vec![
        p(5.0, 106.0),
        p(8.0, 100.0),
        p(15.0, 104.0),
        p(18.0, 102.0),
        p(21.0, 103.0),
    ];
    let id = add(
        &mut chart,
        DrawingKind::PatternTriangle,
        converging,
        r##"{"color":"#123456","width":2}"##,
    );
    let apex = px(&chart, 21.5, 102.7);
    let (a, b) = (anchor(&chart, id, 0), anchor(&chart, id, 1));
    let to_apex = |chart: &mut ChartEngine, from: (f64, f64)| {
        ink_polylines(chart)
            .iter()
            .any(|(line, ..)| close(line[0], from, 0.01) && close(line[1], apex, 0.01))
    };
    let beyond = ((a.0 + 9.0 * apex.0) / 10.0, (a.1 + 9.0 * apex.1) / 10.0);
    // Upstream's default: the zigzag alone.
    assert_eq!(ink_polylines(&mut chart).len(), 1);
    assert_eq!(hit(&chart, beyond), None);
    // A left extension does not reach forward.
    assert!(chart.drawing_apply_options(id, r#"{"extend_left":true}"#));
    assert_eq!(ink_polylines(&mut chart).len(), 1);
    assert!(chart.drawing_apply_options(
        id,
        r#"{"extend_left":false,"extend_right":true,"fill_enabled":true}"#
    ));
    assert!(to_apex(&mut chart, a) && to_apex(&mut chart, b));
    assert_eq!(ink_fills(&mut chart), 1);
    assert!(beyond.0 > anchor(&chart, id, 3).0);
    assert_eq!(hit(&chart, beyond), Some(id));

    // Diverging sides stay between their anchors and shade the convex quad.
    let diverging = [
        p(5.0, 106.0),
        p(8.0, 100.0),
        p(15.0, 107.0),
        p(18.0, 99.5),
        p(21.0, 108.0),
    ]
    .map(DrawingAnchor::from);
    assert!(chart.set_drawing_anchors(id, &diverging).is_ok());
    let (c, d) = (anchor(&chart, id, 2), anchor(&chart, id, 3));
    let lines = ink_polylines(&mut chart);
    assert!(lines
        .iter()
        .any(|(line, ..)| line.len() == 2 && close(line[0], a, 0.01) && close(line[1], c, 0.01)));
    assert!(lines
        .iter()
        .any(|(line, ..)| line.len() == 2 && close(line[0], b, 0.01) && close(line[1], d, 0.01)));
    assert_eq!(ink_fills(&mut chart), 1);
    // Without the flag the fill alone shades the quad.
    assert!(chart.drawing_apply_options(id, r#"{"extend_right":false}"#));
    assert_eq!(ink_polylines(&mut chart).len(), 1);
    assert_eq!(ink_fills(&mut chart), 1);
}

/// An extended triangle pattern keeps time culling: its logical bounds pad by one pattern
/// width (the apex's farthest reach), so a triangle left of the pane still paints and hits its
/// apex region while one far outside the window is no candidate.
#[test]
fn triangle_apexes_keep_time_culling_and_their_visible_reach() {
    let mut chart = chart();
    crowd(&mut chart);
    let triangle = add(
        &mut chart,
        DrawingKind::PatternTriangle,
        vec![
            p(-14.0, 106.0),
            p(-11.0, 100.0),
            p(-4.0, 104.0),
            p(-1.0, 102.0),
            p(2.0, 103.0),
        ],
        r#"{"extend_right":true}"#,
    );
    chart.set_visible_logical_range(0.5, 39.0);
    chart.build_frame();
    assert!(viewport_candidate(&chart, triangle));
    let apex = px(&chart, 2.5, 102.7);
    let a = anchor(&chart, triangle, 0);
    assert!(a.0 < 0.0 && apex.0 > 0.0);
    let near_apex = ((a.0 + 9.0 * apex.0) / 10.0, (a.1 + 9.0 * apex.1) / 10.0);
    assert!(near_apex.0 > 0.0);
    assert_eq!(hit(&chart, near_apex), Some(triangle));
    let far = add(
        &mut chart,
        DrawingKind::PatternTriangle,
        vec![
            p(-214.0, 106.0),
            p(-211.0, 100.0),
            p(-204.0, 104.0),
            p(-201.0, 102.0),
            p(-198.0, 103.0),
        ],
        r#"{"extend_left":true,"extend_right":true}"#,
    );
    chart.build_frame();
    assert!(!viewport_candidate(&chart, far));
}

/// Vertex labels (upstream's placement, owner decision P1) are body targets, and the culling pad
/// covers them, so indexed hits equal brute force at every label.
#[test]
fn vertex_labels_are_body_targets_on_the_culled_path() {
    let mut chart = chart();
    crowd(&mut chart);
    let head = add(
        &mut chart,
        DrawingKind::PatternHeadShoulders,
        points_of(&HEAD_AND_SHOULDERS),
        "{}",
    );
    chart.build_frame();
    let head_x = anchor(&chart, head, 3).0;
    let label_y = text_y(&mut chart, "H");
    assert!(label_y < anchor(&chart, head, 3).1);
    assert_eq!(hit(&chart, (head_x, label_y)), Some(head));
    assert_eq!(hit(&chart, (head_x, label_y - 20.0)), None);
    chart.clear_drawings();
    crowd(&mut chart);
    for kind in &KINDS[..11] {
        let id = add(&mut chart, *kind, zigzag_points(*kind), "{}");
        chart.build_frame();
        let points = zigzag_points(*kind);
        for (index, point) in points.iter().enumerate() {
            let (x, y) = px(&chart, point.logical, point.price);
            let Some((_, _, label_y)) = texts(&mut chart)
                .into_iter()
                .find(|(_, tx, ty)| (tx - x).abs() < 0.5 && *ty < y && y - ty < 30.0)
            else {
                assert!(kind.is_elliott() && index == 0, "{kind:?} label {index}");
                continue;
            };
            assert_eq!(
                hit(&chart, (x, label_y)),
                Some(id),
                "{kind:?} label {index}"
            );
        }
        assert!(chart.remove_drawing(id));
    }
}

/// A ratio label of any width keeps its drawing a candidate past the anchors' box.
#[test]
fn wide_ratio_labels_keep_their_drawing_visible_past_the_anchors() {
    let mut chart = chart();
    crowd(&mut chart);
    // A near-flat AB leg makes BC/AB a ten-digit ratio. Its A-C connector is vertical on the
    // anchors' left edge, so half of that label reaches left of every anchor.
    let id = add(
        &mut chart,
        DrawingKind::PatternAbcd,
        vec![
            p(20.0, 100.0),
            p(24.0, 100.0 + 1e-9),
            p(20.0, 104.0),
            p(24.0, 101.0),
        ],
        r##"{"color":"#123456"}"##,
    );
    // The anchors sit past the right edge; only the label reaches into the pane.
    chart.set_visible_logical_range(-21.5, 17.5);
    chart.build_frame();
    let a = anchor(&chart, id, 0);
    assert!(a.0 > chart.pane_w + 30.0, "{a:?}");
    let (text, left, y) = texts(&mut chart)
        .into_iter()
        .find(|(text, ..)| text.parse::<f64>().is_ok_and(|value| value > 1e9))
        .expect("the ten-digit ratio is painted");
    assert!(left < chart.pane_w - 5.0, "{text} starts inside the pane");
    assert!(viewport_candidate(&chart, id));
    assert_eq!(hit(&chart, (chart.pane_w - 3.0, y)), Some(id));
}

/// Elliott waves label in the notation of their `wave_degree` without the start label (owner
/// decision P4), ring the ringed degrees, and `show_wave: false` keeps only the labels, which
/// stay body targets.
#[test]
fn elliott_waves_label_each_wave_in_its_degree() {
    let mut chart = chart();
    crowd(&mut chart);
    let id = add(
        &mut chart,
        DrawingKind::ElliottImpulse,
        vec![
            p(5.0, 100.0),
            p(9.0, 103.0),
            p(12.0, 101.5),
            p(18.0, 106.0),
            p(21.0, 104.0),
            p(26.0, 105.5),
        ],
        r##"{"color":"#123456","width":2}"##,
    );
    let wave_texts = texts_of;
    assert_eq!(wave_texts(&mut chart), ["1", "2", "3", "4", "5"]);
    assert_eq!(ink_polylines(&mut chart).len(), 1);
    assert!(chart.drawing_apply_options(id, r#"{"wave_degree":"intermediate"}"#));
    assert_eq!(wave_texts(&mut chart), ["(1)", "(2)", "(3)", "(4)", "(5)"]);

    // Ringed degrees add a 1 px ring around each label, clear of its vertex.
    assert!(chart.drawing_apply_options(id, r#"{"wave_degree":"primary"}"#));
    assert_eq!(wave_texts(&mut chart), ["1", "2", "3", "4", "5"]);
    let rings = ink_polylines(&mut chart)
        .into_iter()
        .filter(|(points, width, _)| *width == 1.0 && points.len() > 8)
        .collect::<Vec<_>>();
    assert_eq!(rings.len(), 5);
    let (_, x, y) = texts(&mut chart).remove(0);
    let ring = &rings[0].0;
    let center = (
        ring.iter().map(|point| point.0).sum::<f64>() / ring.len() as f64,
        ring.iter().map(|point| point.1).sum::<f64>() / ring.len() as f64,
    );
    assert!(close(center, (x, y), 1.0), "the ring wraps the label");
    let lowest = ring.iter().map(|point| point.1).fold(f64::MIN, f64::max);
    assert!(
        lowest < anchor(&chart, id, 1).1,
        "the ring clears its vertex"
    );
    // Inside the ring, off the glyphs (a one-digit run is narrower than the ring), hits.
    let radius = ring
        .iter()
        .map(|point| (point.0 - center.0).hypot(point.1 - center.1))
        .fold(0.0_f64, f64::max);
    let inside = (x + 0.8 * radius, y);
    assert!(
        !chart.text_run_hit(
            chart.drawing(id).unwrap(),
            "1",
            (x, y),
            aeris_charts_render::draw_list::TextAlign::Center,
            chart.drawing_text_size(chart.drawing(id).unwrap()),
            inside,
        ),
        "the probe is off the glyph run"
    );
    assert_eq!(hit(&chart, inside), Some(id));

    // Without the wave only the labels paint, and they remain body targets.
    assert!(chart.drawing_apply_options(
        id,
        r#"{"wave_degree":"minor","tool_options":{"pattern":{"show_wave":false}}}"#
    ));
    assert!(ink_polylines(&mut chart).is_empty());
    let (_, x, y) = texts(&mut chart).remove(0);
    assert_eq!(hit(&chart, (x, y)), Some(id));
    let (w1, w2) = (anchor(&chart, id, 2), anchor(&chart, id, 3));
    assert_eq!(
        hit(&chart, ((w1.0 + w2.0) / 2.0, (w1.1 + w2.1) / 2.0)),
        None
    );
    assert!(chart.drawing_apply_options(id, r#"{"tool_options":{"pattern":{"show_wave":true}}}"#));
    assert_eq!(
        hit(&chart, ((w1.0 + w2.0) / 2.0, (w1.1 + w2.1) / 2.0)),
        Some(id)
    );

    // The other waves use their letters, capital on the minor degree.
    let letters = [
        (DrawingKind::ElliottCorrection, vec!["A", "B", "C"]),
        (DrawingKind::ElliottTriangle, vec!["A", "B", "C", "D", "E"]),
        (DrawingKind::ElliottDoubleCombination, vec!["W", "X", "Y"]),
        (
            DrawingKind::ElliottTripleCombination,
            vec!["W", "X", "Y", "X", "Z"],
        ),
    ];
    for (kind, expected) in letters {
        chart.clear_drawings();
        add(&mut chart, kind, zigzag_points(kind), "{}");
        assert_eq!(texts_of(&mut chart), expected, "{kind:?}");
        // Lower-case letters in parentheses on the minuette degree.
        add(
            &mut chart,
            kind,
            zigzag_points(kind),
            r#"{"wave_degree":"minuette"}"#,
        );
        let minuette = expected
            .iter()
            .map(|letter| format!("({})", letter.to_lowercase()));
        assert_eq!(
            texts_of(&mut chart),
            expected
                .iter()
                .map(|letter| letter.to_string())
                .chain(minuette)
                .collect::<Vec<_>>(),
            "{kind:?}"
        );
    }
}

/// A degree ring hits up to the stroke tolerance past its edge, and the culling pad covers that
/// band, so indexed hits equal brute force above the highest ring on either hit profile.
#[test]
fn ring_tolerance_band_stays_on_the_culled_path() {
    let mut chart = chart();
    crowd(&mut chart);
    let id = add(
        &mut chart,
        DrawingKind::ElliottImpulse,
        vec![
            p(5.0, 100.0),
            p(9.0, 103.0),
            p(12.0, 101.5),
            p(18.0, 106.0),
            p(21.0, 104.0),
            p(26.0, 105.5),
        ],
        r##"{"color":"#123456","wave_degree":"primary"}"##,
    );
    chart.build_frame();
    // Wave 3 tops the impulse; its ring is the highest paint.
    let top = anchor(&chart, id, 3);
    let (_, x, y) = texts(&mut chart)
        .into_iter()
        .find(|(text, tx, _)| text == "3" && (tx - top.0).abs() < 0.5)
        .expect("wave 3 label");
    let ring = ink_polylines(&mut chart)
        .into_iter()
        .map(|(points, ..)| points)
        .find(|points| points.len() > 8 && close(points[0], (x, y), 20.0))
        .expect("wave 3 ring");
    let radius = ring
        .iter()
        .map(|point| (point.0 - x).hypot(point.1 - y))
        .fold(0.0_f64, f64::max);
    for profile in [HitProfile::PRECISION, HitProfile::TOUCH] {
        let tolerance = profile.drawing_stroke_tolerance;
        let probe = |point: (f64, f64)| {
            let indexed = chart.hit_test_drawing_impl(point.0, point.1, true, profile);
            assert_eq!(
                indexed,
                chart.hit_test_drawing_impl(point.0, point.1, false, profile),
                "{point:?} {profile:?}"
            );
            indexed.map(|hit| hit.id)
        };
        assert_eq!(probe((x, y - radius - tolerance + 0.5)), Some(id));
        let mut dy = 0.0;
        while dy <= tolerance + 2.0 {
            let mut dx = -radius;
            while dx <= radius {
                probe((x + dx, y - radius - dy));
                dx += 1.0;
            }
            dy += 0.5;
        }
    }
}

/// While a pattern is placed it previews as the drawing it will commit (owner decision P5):
/// legs, labels, ratios and fills from the placed anchors and the pointer.
#[test]
fn placement_previews_the_legs_labels_ratios_and_fills_placed_so_far() {
    let mut chart = chart();
    assert!(chart.set_drawing_tool(
        Some(DrawingKind::PatternXabcd),
        Some(r##"{"color":"#123456","fill_enabled":true}"##),
        None
    ));
    let (x, a) = (px(&chart, 6.0, 101.0), px(&chart, 10.0, 105.0));
    let b = px(&chart, 14.0, 102.0);
    chart.drawing_tool_activate(x.0, x.1, DrawingModifiers::default());
    chart.drawing_tool_pointer_move(a.0, a.1, DrawingModifiers::default(), false);
    let legs = ink_polylines(&mut chart);
    assert_eq!(legs.len(), 1, "X to the pointer after one click");
    assert!(close(legs[0].0[0], x, 0.01) && close(legs[0].0[1], a, 0.01));
    chart.drawing_tool_activate(a.0, a.1, DrawingModifiers::default());
    chart.drawing_tool_pointer_move(b.0, b.1, DrawingModifiers::default(), false);
    let zigzag = ink_polylines(&mut chart)
        .into_iter()
        .find(|(points, ..)| points.len() == 3)
        .expect("X-A-B preview");
    assert!(close(zigzag.0[2], b, 0.01));
    let runs = texts_of(&mut chart);
    // AB/XA = 3/4.
    for label in ["X", "A", "B", "0.750"] {
        assert!(runs.iter().any(|run| run == label), "{label} in {runs:?}");
    }
    assert_eq!(ink_fills(&mut chart), 1, "the XAB triangle shades");
    assert!(
        chart.drawings().is_empty(),
        "nothing commits before five anchors"
    );

    // Every kind previews every prefix with finite geometry.
    for kind in KINDS {
        assert!(chart.set_drawing_tool(Some(kind), None, None));
        for index in 0..kind.anchor_count().saturating_sub(1) {
            let (x, y) = (
                150.0 + index as f64 * 60.0,
                280.0 - (index % 2) as f64 * 100.0,
            );
            chart.drawing_tool_activate(x, y, DrawingModifiers::default());
            chart.drawing_tool_pointer_move(x + 30.0, y - 40.0, DrawingModifiers::default(), false);
            let frame = chart.build_frame();
            assert!(frame.panes[0]
                .points
                .iter()
                .all(|point| point[0].is_finite() && point[1].is_finite()));
        }
        chart.set_drawing_tool(None, None, None);
    }
}

/// The schema lists the rendered pattern options: `show_ratios` on the harmonic patterns and
/// `show_wave` on the Elliott waves, both on by default.
#[test]
fn schema_lists_the_rendered_pattern_options() {
    for kind in &KINDS[..11] {
        let schema = crate::drawing_property_schema(*kind);
        let row = |name: &str| {
            schema
                .properties
                .iter()
                .find(|property| property.name == name)
                .map(|property| property.default.clone())
        };
        let harmonic = matches!(
            kind,
            DrawingKind::PatternXabcd
                | DrawingKind::PatternCypher
                | DrawingKind::PatternAbcd
                | DrawingKind::PatternThreeDrives
        );
        assert_eq!(
            row("tool_options.pattern.show_ratios"),
            harmonic.then_some(serde_json::Value::Bool(true)),
            "{kind:?}"
        );
        assert_eq!(
            row("tool_options.pattern.show_wave"),
            kind.is_elliott().then_some(serde_json::Value::Bool(true)),
            "{kind:?}"
        );
    }
    // The options persist.
    let mut chart = chart();
    let id = add(
        &mut chart,
        DrawingKind::ElliottImpulse,
        zigzag_points(DrawingKind::ElliottImpulse),
        r#"{"tool_options":{"pattern":{"show_wave":false}}}"#,
    );
    let saved = chart.export_state_json().unwrap();
    let mut restored = chart_with(&hourly(40), 1.0);
    restored.import_state_json(&saved).unwrap();
    assert!(
        !restored
            .drawing(id)
            .unwrap()
            .tool_options
            .pattern
            .unwrap()
            .show_wave
    );
    assert!(!super::draws_wave(restored.drawing(id).unwrap()));
}

/// Documents the fork wrote regain its pattern look: shaded harmonic triangles, the neckline
/// with shaded shoulders, the triangle's sides to the apex, and intermediate Elliott labels.
#[test]
fn fork_documents_regain_the_fork_pattern_look() {
    let at = |logical: f64, price: f64| serde_json::json!({"logical": logical, "price": price});
    let drawing = |id: u32, kind: &str, anchors: &[(f64, f64)]| {
        serde_json::json!({
            "id": id,
            "kind": kind,
            "pane_id": "pane-1",
            "anchors": anchors.iter().map(|&(l, p)| at(l, p)).collect::<Vec<_>>(),
        })
    };
    let document = serde_json::json!({
        "schema": "aeris_charts-state",
        "schema_version": 1,
        "panes": [{"id": "pane-1"}],
        "drawings": [
            drawing(1, "triangle_pattern", &[(5.0, 106.0), (8.0, 100.0), (15.0, 104.0), (18.0, 102.0)]),
            drawing(2, "head_and_shoulders", &HEAD_AND_SHOULDERS),
            drawing(3, "xabcd_pattern", &[(6.0, 100.0), (10.0, 105.0), (14.0, 101.91), (18.0, 104.0), (22.0, 100.5)]),
            drawing(4, "elliott_impulse_wave", &[(5.0, 100.0), (9.0, 103.0), (12.0, 101.5), (18.0, 106.0), (21.0, 104.0), (26.0, 105.5)]),
        ]
    })
    .to_string();
    let fills = |chart: &mut ChartEngine, color: &str| {
        let ink = Color::parse_css(color).unwrap();
        let wash = Color::rgba(ink.r(), ink.g(), ink.b(), super::FILL_ALPHA);
        let frame = chart.build_frame();
        frame.panes[0]
            .main
            .iter()
            .filter(|prim| matches!(prim, Prim::BandFill { fill, .. } if *fill == wash))
            .count()
    };
    let only = |chart: &mut ChartEngine, keep: u32| {
        for id in 1..=4 {
            chart.set_drawing_visibility(id, id == keep);
        }
    };
    let mut chart = chart();
    chart.import_state_json(&document).unwrap();
    chart.build_frame();
    let triangle = chart.drawing(1).unwrap();
    assert!(triangle.extend_left && triangle.extend_right && triangle.fill_enabled);
    only(&mut chart, 1);
    let apex = px(&chart, 21.5, 102.7);
    let frame = chart.build_frame();
    let purple = Color::parse_css("#673AB7").unwrap();
    let pane = &frame.panes[0];
    let reaches_apex = pane.main.iter().any(|prim| match prim {
        Prim::Polyline {
            first_point,
            point_count: 2,
            color,
            ..
        } if *color == purple => {
            let end = pane.points[*first_point as usize + 1];
            close((f64::from(end[0]), f64::from(end[1])), apex, 0.01)
        }
        _ => false,
    });
    assert!(reaches_apex);
    assert_eq!(fills(&mut chart, "#673AB7"), 1);
    only(&mut chart, 2);
    assert_eq!(fills(&mut chart, "#089981"), 3);
    only(&mut chart, 3);
    assert_eq!(fills(&mut chart, "#2962FF"), 2);
    assert!(texts_of(&mut chart).contains(&"0.618".to_string()));
    only(&mut chart, 4);
    assert_eq!(texts_of(&mut chart), ["(1)", "(2)", "(3)", "(4)", "(5)"]);
}

/// Pattern decorations scale with the device pixel ratio: 1 CSS px connectors and rings, ratio
/// boxes and glyphs at the ratio's size.
#[test]
fn pattern_decorations_scale_with_the_device_pixel_ratio() {
    for dpr in [1.0, 2.0] {
        let mut chart = chart_with(&hourly(40), dpr);
        add(
            &mut chart,
            DrawingKind::PatternAbcd,
            vec![p(6.0, 105.0), p(10.0, 101.0), p(14.0, 103.5), p(18.0, 99.5)],
            r##"{"color":"#123456","width":2}"##,
        );
        add(
            &mut chart,
            DrawingKind::ElliottCorrection,
            zigzag_points(DrawingKind::ElliottCorrection),
            r##"{"color":"#123456","width":2,"wave_degree":"minute"}"##,
        );
        let frame = chart.build_frame();
        let main = &frame.panes[0].main;
        let thin = main
            .iter()
            .filter(|prim| matches!(prim, Prim::Polyline { width, color, .. } if f64::from(*width) == dpr && *color == ink()))
            .count();
        assert!(thin > 3 + 3, "connector dashes and three rings at {dpr}");
        let ratio = main
            .iter()
            .find_map(|prim| match prim {
                Prim::Text { text, size, .. } if text == "0.625" => Some(f64::from(*size)),
                _ => None,
            })
            .unwrap();
        assert_eq!(ratio, 12.0 * dpr);
        assert!(main
            .iter()
            .any(|prim| matches!(prim, Prim::Text { text, .. } if text == "a")));
    }
}
