//! Timeline-mark lane frame tests: token primitives and layering, pixel-ratio parity, the
//! autoscale reservation, the hover ring and dwell tooltip, and the retained == clean invariant.

use super::*;
use crate::timeline_marks::TIMELINE_LANE_RESERVATION_CSS as LANE;
use crate::{
    PointerInput, TRADING_TOOLTIP_DWELL_MS, TimelineGlyphShape, TimelineMark, TimelineMarkGlyph,
    TimelineMarkGroup, TimelineMarksSnapshot,
};

const BARS: usize = 80;
const HOUR: i64 = 3_600;
const START: i64 = 1_700_000_000;

fn bar_time(index: usize) -> i64 {
    START + index as i64 * HOUR
}

fn chart_at(dpr: f64) -> ChartEngine {
    let mut chart = ChartEngine::new(800.0, 500.0, dpr);
    let times: Vec<f64> = (0..BARS).map(|i| bar_time(i) as f64).collect();
    let open: Vec<f64> = (0..BARS).map(|i| 100.0 + (i % 7) as f64).collect();
    let high: Vec<f64> = open.iter().map(|value| value + 3.0).collect();
    let low: Vec<f64> = open.iter().map(|value| value - 3.0).collect();
    let close: Vec<f64> = open.iter().map(|value| value + 1.0).collect();
    chart
        .set_series_data(0, &times, &open, &high, &low, &close)
        .unwrap();
    chart.recompute_layout_with_measure(true, |_, _| 48.0, |_, _| 48.0);
    chart.fit_content();
    chart
}

fn mark(
    id: &str,
    index: usize,
    group: &str,
    shape: TimelineGlyphShape,
    color: &str,
    letter: &str,
) -> TimelineMark {
    TimelineMark {
        id: id.to_string(),
        time: bar_time(index),
        group: group.to_string(),
        glyph: TimelineMarkGlyph {
            shape,
            color: color.to_string(),
            letter: letter.to_string(),
        },
        title: format!("{id} title"),
    }
}

fn snapshot(marks: Vec<TimelineMark>) -> TimelineMarksSnapshot {
    TimelineMarksSnapshot {
        marks,
        groups: vec![TimelineMarkGroup {
            id: "earnings".into(),
            label: "Earnings".into(),
        }],
    }
}

fn chrome_range(chart: &ChartEngine) -> std::ops::Range<usize> {
    let segment = chart
        .frame_series_segments(0)
        .iter()
        .find(|segment| segment.series_id.is_none())
        .copied()
        .expect("the chrome segment");
    segment.start..segment.end
}

fn overlay_range(chart: &ChartEngine) -> std::ops::Range<usize> {
    let segments = chart.frame_pane_segments(0).unwrap();
    segments.trading_end..segments.overlay_end
}

fn token_center(chart: &ChartEngine, index: usize) -> (f64, f64) {
    let pane = &chart.panes[0];
    (
        chart.time_scale.index_to_coordinate(index as i64),
        pane.top + pane.height - 15.0,
    )
}

fn glyph_texts(prims: &[Prim]) -> Vec<String> {
    prims
        .iter()
        .filter_map(|prim| match prim {
            Prim::Text {
                text, weight: 600, ..
            } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

fn assert_retained_matches_clean(chart: &mut ChartEngine) {
    let incremental = chart.build_frame();
    chart.retained_frame = RetainedFrame::default();
    chart.frame_invalidation.all();
    let rebuilt = chart.build_frame();
    assert_eq!(incremental, rebuilt);
}

#[test]
fn tokens_letters_and_counts_land_in_the_chrome_layer() {
    let mut chart = chart_at(1.0);
    let red = Color::rgb(0xff, 0, 0);
    chart
        .set_timeline_marks(snapshot(vec![
            mark(
                "e",
                30,
                "earnings",
                TimelineGlyphShape::Circle,
                "#ff0000",
                "E",
            ),
            mark(
                "s",
                50,
                "earnings",
                TimelineGlyphShape::Square,
                "#00ff00",
                "S",
            ),
            mark(
                "s2",
                50,
                "earnings",
                TimelineGlyphShape::Diamond,
                "#0000ff",
                "D",
            ),
            mark("n", 60, "news", TimelineGlyphShape::Pin, "#123456", "N"),
            mark(
                "m",
                61,
                "earnings",
                TimelineGlyphShape::Circle,
                "#654321",
                "M",
            ),
            mark("p", 70, "news", TimelineGlyphShape::Pin, "#abcdef", "P"),
            mark("q", 10, "news", TimelineGlyphShape::Diamond, "#fedcba", ""),
        ]))
        .unwrap();
    let frame = chart.build_frame();
    let chrome = chrome_range(&chart);
    let segments = chart.frame_pane_segments(0).unwrap();
    assert!(
        chrome.end <= segments.trading_regions_end,
        "chrome sits below trading"
    );
    let prims = &frame.panes[0].main[chrome.clone()];

    // The single circle token: 16 px disc at the bar's x, 15 px above the pane bottom.
    let (x, y) = token_center(&chart, 30);
    let circle = prims
        .iter()
        .position(|prim| {
            matches!(prim, Prim::Circle { cx, cy, radius, fill, .. }
                if *fill == red && (*cx - x.round() as f32).abs() < 1e-4
                    && (*cy - y.round() as f32).abs() < 1e-4 && (*radius - 8.0).abs() < 1e-4)
        })
        .expect("the earnings disc");
    let letter = prims
        .iter()
        .position(|prim| {
            matches!(prim, Prim::Text { text, weight: 600, align: TextAlign::Center, color, .. }
                if text == "E" && *color == red.contrast_text())
        })
        .expect("the letter rides the disc");
    assert!(letter > circle, "the letter paints above its disc");

    // Same-slot pair of one group: the earliest mark's glyph (square, green) with the count.
    let (x50, _) = token_center(&chart, 50);
    assert!(
        prims
            .iter()
            .any(|prim| matches!(prim, Prim::RoundRect { x, w, fill, .. }
        if *fill == Color::rgb(0, 0xff, 0) && (*w - 18.0).abs() < 1e-4
            && ((*x + *w / 2.0) - x50.round() as f32).abs() < 1e-4))
    );
    // Mixed cluster of two groups one bar apart (10 px): a neutral bordered square with the count.
    let (x60, _) = token_center(&chart, 60);
    let text_color = chart.primary_text_color();
    assert!(prims.iter().any(
        |prim| matches!(prim, Prim::RoundRect { x, w, border_color, border_width, .. }
        if *border_color == text_color && *border_width > 0.0 && (*w - 18.0).abs() < 1e-4
            && ((*x + *w / 2.0) - x60.round() as f32).abs() < 1e-4)
    ));
    // A pin is a disc over a triangle; a diamond is two triangles; a letterless mark prints none.
    let (x70, _) = token_center(&chart, 70);
    assert!(
        prims
            .iter()
            .any(|prim| matches!(prim, Prim::Triangle { c, color, .. }
        if *color == Color::rgb(0xab, 0xcd, 0xef) && (c[0] - x70.round() as f32).abs() < 1e-4))
    );
    let triangles = prims
        .iter()
        .filter(|prim| matches!(prim, Prim::Triangle { color, .. } if *color == Color::rgb(0xfe, 0xdc, 0xba)))
        .count();
    assert_eq!(triangles, 2, "a diamond is two triangles");
    let mut texts = glyph_texts(prims);
    texts.sort();
    assert_eq!(texts, ["2", "2", "E", "P"]);
    assert_retained_matches_clean(&mut chart);
}

#[test]
fn counts_fold_at_ninety_nine_plus() {
    let mut chart = chart_at(1.0);
    let marks = (0..120)
        .map(|i| {
            mark(
                &format!("m{i}"),
                40,
                "earnings",
                TimelineGlyphShape::Circle,
                "#ff0000",
                "X",
            )
        })
        .collect();
    chart.set_timeline_marks(snapshot(marks)).unwrap();
    let frame = chart.build_frame();
    assert_eq!(
        glyph_texts(&frame.panes[0].main[chrome_range(&chart)]),
        ["99+"]
    );
}

#[test]
fn lane_geometry_is_identical_across_pixel_ratios_and_matches_a_clean_rebuild() {
    let tokens = |dpr: f64| {
        let mut chart = chart_at(dpr);
        chart
            .set_timeline_marks(snapshot(vec![
                mark(
                    "a",
                    10,
                    "earnings",
                    TimelineGlyphShape::Circle,
                    "#ff0000",
                    "A",
                ),
                mark(
                    "b",
                    11,
                    "earnings",
                    TimelineGlyphShape::Circle,
                    "#ff0000",
                    "B",
                ),
                mark("c", 12, "news", TimelineGlyphShape::Square, "#00ff00", "C"),
                mark("d", 40, "news", TimelineGlyphShape::Square, "#00ff00", "D"),
            ]))
            .unwrap();
        let frame = chart.build_frame();
        let texts = glyph_texts(&frame.panes[0].main[chrome_range(&chart)]);
        assert_retained_matches_clean(&mut chart);
        texts
    };
    let one = tokens(1.0);
    assert_eq!(one, tokens(1.5));
    assert_eq!(one, tokens(2.0));
    assert_eq!(one, ["3", "D"]);
}

#[test]
fn the_lane_reserves_its_height_independent_of_view_and_hidden_groups() {
    let mut chart = chart_at(1.0);
    chart.build_frame();
    assert_eq!(chart.panes[0].marker_margin_below, 0.0);
    chart
        .set_timeline_marks(snapshot(vec![mark(
            "a",
            40,
            "earnings",
            TimelineGlyphShape::Circle,
            "#ff0000",
            "A",
        )]))
        .unwrap();
    chart.build_frame();
    assert_eq!(
        chart.frame_build_stats().autoscale_runs,
        1,
        "marks re-run autoscale"
    );
    assert_eq!(chart.panes[0].marker_margin_below, LANE);
    assert_eq!(
        chart.panes[0].overlay_marker_margin_below, LANE,
        "the volume overlay lifts too"
    );
    assert_eq!(chart.panes[0].marker_margin_above, 0.0);
    let scale_range = chart.panes[0].price_scale.price_range().cloned().unwrap();

    // Hiding the only group removes the token but keeps the reservation without autoscaling.
    chart.set_timeline_group_hidden("earnings", true).unwrap();
    let frame = chart.build_frame();
    assert!(glyph_texts(&frame.panes[0].main[chrome_range(&chart)]).is_empty());
    assert_eq!(chart.frame_build_stats().autoscale_runs, 0);
    assert_eq!(chart.panes[0].marker_margin_below, LANE);
    assert_eq!(
        chart.panes[0].price_scale.price_range().cloned().unwrap(),
        scale_range
    );
    chart.set_timeline_group_hidden("earnings", false).unwrap();

    // Scrolling the token out of view keeps the reservation.
    chart.set_visible_logical_range(0.0, 20.0);
    let frame = chart.build_frame();
    assert!(glyph_texts(&frame.panes[0].main[chrome_range(&chart)]).is_empty());
    assert_eq!(chart.panes[0].marker_margin_below, LANE);

    // Turning the lane off releases it; an empty snapshot reserves nothing.
    chart.set_timeline_marks_visible(false);
    chart.build_frame();
    assert_eq!(chart.panes[0].marker_margin_below, 0.0);
    chart.set_timeline_marks_visible(true);
    chart
        .set_timeline_marks(TimelineMarksSnapshot::default())
        .unwrap();
    chart.build_frame();
    assert_eq!(chart.panes[0].marker_margin_below, 0.0);
    assert_eq!(LANE, 27.0);
}

#[test]
fn a_short_pane_hides_the_lane_and_its_reservation() {
    let mut chart = ChartEngine::new(800.0, 90.0, 1.0);
    let times: Vec<f64> = (0..BARS).map(|i| bar_time(i) as f64).collect();
    let values: Vec<f64> = (0..BARS).map(|i| 100.0 + i as f64).collect();
    chart
        .set_series_data(0, &times, &values, &values, &values, &values)
        .unwrap();
    chart.recompute_layout_with_measure(true, |_, _| 48.0, |_, _| 48.0);
    chart.fit_content();
    assert!(
        chart.panes[0].height < 96.0,
        "pane {} px",
        chart.panes[0].height
    );
    chart
        .set_timeline_marks(snapshot(vec![mark(
            "a",
            40,
            "earnings",
            TimelineGlyphShape::Circle,
            "#ff0000",
            "A",
        )]))
        .unwrap();
    let frame = chart.build_frame();
    assert!(glyph_texts(&frame.panes[0].main[chrome_range(&chart)]).is_empty());
    assert_eq!(chart.panes[0].marker_margin_below, 0.0);
    let (x, y) = token_center(&chart, 40);
    assert_eq!(chart.timeline_mark_hit_at(x, y), None);
}

#[test]
fn projected_marks_draw_in_the_whitespace_only_while_a_bar_is_visible() {
    let mut chart = chart_at(1.0);
    let future = TimelineMark {
        id: "fut".into(),
        time: bar_time(BARS - 1) + 5 * HOUR + 10,
        group: "earnings".into(),
        glyph: TimelineMarkGlyph {
            letter: "F".into(),
            ..TimelineMarkGlyph::default()
        },
        title: "future".into(),
    };
    chart.set_timeline_marks(snapshot(vec![future])).unwrap();
    // Fit content leaves no right whitespace: the projected slot is outside the view.
    let frame = chart.build_frame();
    assert!(glyph_texts(&frame.panes[0].main[chrome_range(&chart)]).is_empty());
    // Ten bars of whitespace: the token sits five slots past the last bar.
    chart.set_right_offset(10.0);
    let frame = chart.build_frame();
    let prims = &frame.panes[0].main[chrome_range(&chart)];
    assert_eq!(glyph_texts(prims), ["F"]);
    let expected_x = chart
        .time_scale
        .logical_to_coordinate((BARS - 1) as f64 + 5.0)
        .round() as f32;
    assert!(
        prims
            .iter()
            .any(|prim| matches!(prim, Prim::Circle { cx, .. } if (*cx - expected_x).abs() < 1e-4))
    );
    let hit = chart
        .timeline_mark_hit_at(f64::from(expected_x), token_center(&chart, 0).1)
        .expect("the projected token hits");
    assert!(hit.projected);
    assert_eq!(hit.logical, (BARS - 1) as i64 + 5);
    // Chrome builds only while a bar is visible. The time scale never scrolls the last bars out
    // of view (the right offset clamps), so a projected token inside the clamped window still
    // draws; one past the window does not.
    chart.set_visible_logical_range(100.0, 180.0);
    let range = chart.time_scale.visible_logical_range().unwrap();
    assert!(
        chart.visible_range().is_some(),
        "the scale keeps the last bars in view"
    );
    let frame = chart.build_frame();
    let in_window = (range.left()..=range.right()).contains(&((BARS - 1) as f64 + 5.0));
    assert_eq!(
        glyph_texts(&frame.panes[0].main[chrome_range(&chart)]).is_empty(),
        !in_window
    );
    assert_retained_matches_clean(&mut chart);
}

#[test]
fn the_hover_ring_and_armed_tooltip_live_in_the_overlay() {
    let mut chart = chart_at(1.0);
    chart
        .set_timeline_marks(snapshot(vec![mark(
            "a",
            40,
            "earnings",
            TimelineGlyphShape::Circle,
            "#ff0000",
            "A",
        )]))
        .unwrap();
    chart.build_frame();
    let (x, y) = token_center(&chart, 40);
    let text_color = chart.primary_text_color();
    let ring = |frame: &ChartFrame, chart: &ChartEngine| {
        frame.panes[0].main[overlay_range(chart)]
            .iter()
            .any(|prim| matches!(prim, Prim::RoundRect { fill, border_color, w, .. }
                if *fill == Color::rgba(0, 0, 0, 0) && *border_color == text_color && (*w - 20.0).abs() < 1e-4))
    };
    let tooltip = |frame: &ChartFrame, chart: &ChartEngine| {
        frame.panes[0].main[overlay_range(chart)]
            .iter()
            .any(|prim| matches!(prim, Prim::Text { text, .. } if text == "Earnings · a title"))
    };
    chart.input_pointer_move(
        PointerInput {
            x,
            y,
            timestamp_ms: 1_000.0,
            ..PointerInput::default()
        },
        false,
    );
    assert_eq!(
        chart.input_wake_deadline_ms(),
        Some(1_000.0 + TRADING_TOOLTIP_DWELL_MS)
    );
    let frame = chart.build_frame();
    assert!(ring(&frame, &chart), "the hovered token wears a ring");
    assert!(!tooltip(&frame, &chart), "contact alone shows no tooltip");
    assert_eq!(chart.frame_build_stats().series_rebuilds, 0);
    assert_eq!(chart.frame_build_stats().autoscale_runs, 0);
    assert_eq!(chart.frame_build_stats().overlay_rebuilds, 1);

    assert!(!chart.input_tick(1_000.0 + TRADING_TOOLTIP_DWELL_MS - 1.0));
    assert!(chart.input_tick(1_000.0 + TRADING_TOOLTIP_DWELL_MS));
    let frame = chart.build_frame();
    assert!(tooltip(&frame, &chart), "the dwell reveals the title");
    assert_retained_matches_clean(&mut chart);

    chart.input_pointer_move(
        PointerInput {
            x: x + 60.0,
            y: y - 100.0,
            timestamp_ms: 2_000.0,
            ..PointerInput::default()
        },
        false,
    );
    assert_eq!(chart.input_wake_deadline_ms(), None);
    let frame = chart.build_frame();
    assert!(!ring(&frame, &chart));
    assert!(!tooltip(&frame, &chart));
}
