//! Line runs end at period boundaries: indicator resets (session and amount-weighted VWAP, VWAP
//! bands, pivots) and the host `break_on_trading_day` option split line, area, and baseline
//! geometry and hit testing into independent runs, identically after full installs, incremental
//! updates, retention trims, and conflated (sub-pixel) row selection.

use std::collections::BTreeSet;

use super::*;
use crate::synthetic_bars::{SyntheticBar, SyntheticBarOptions, SyntheticSourceBar};
use crate::{PivotKind, SeriesHitKind, UtcOffsetSchedule, VwapReset};

const HOUR: i64 = 3_600;
const DAY: i64 = 86_400;
/// 2026-09-21 00:00 UTC, a Monday.
const MONDAY: i64 = 1_789_948_800;
/// 2024-01-01 00:00 UTC, a Monday.
const MONDAY_2024: i64 = 1_704_067_200;
const AVERAGE: Color = Color::rgb(0x12, 0x34, 0x56);
const TYPICAL: Color = Color::rgb(0x34, 0x56, 0x12);
const PRICE: Color = Color::rgb(0x65, 0x43, 0x21);

/// `days` sessions of `minutes` one-minute rows from 09:30 Asia/Shanghai (01:30 UTC). Prices
/// step up by 2 between days, so a connector between days would be long and steep.
struct Session {
    minutes: usize,
    times: Vec<f64>,
    price: Vec<f64>,
    volume: Vec<f64>,
    amount: Vec<f64>,
}

fn session(days: usize, minutes: usize) -> Session {
    let mut data = Session {
        minutes,
        times: Vec::new(),
        price: Vec::new(),
        volume: Vec::new(),
        amount: Vec::new(),
    };
    for day in 0..days {
        for minute in 0..minutes {
            let price = 10.0 + 2.0 * day as f64 + 0.05 * ((minute * 7) % 5) as f64;
            let volume = 100.0 + (minute % 3) as f64 * 50.0;
            data.times
                .push((MONDAY + day as i64 * DAY + 90 * 60 + minute as i64 * 60) as f64);
            data.price.push(price);
            data.volume.push(volume);
            data.amount.push(volume * price);
        }
    }
    data
}

fn install(chart: &mut ChartEngine, id: SeriesId, times: &[f64], values: &[f64]) {
    chart
        .set_series_data(id, times, values, values, values, values)
        .unwrap();
}

fn apply(chart: &mut ChartEngine, id: SeriesId, json: &str) {
    assert!(chart.series_apply_options_json(id, json), "{json}");
}

struct Intraday {
    chart: ChartEngine,
    volume: SeriesId,
    amount: SeriesId,
    average: SeriesId,
}

/// A Shanghai time-sharing chart over the first `rows` rows: a line price series (optionally
/// broken per trading day), hidden volume/turnover inputs, and the amount-weighted average.
fn intraday(data: &Session, rows: usize, price_breaks: bool) -> Intraday {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    chart.series[0].kind = SeriesKind::Line;
    chart.set_time_zone(UtcOffsetSchedule::fixed(8 * HOUR as i32).unwrap());
    let volume = chart.add_series(SeriesKind::Histogram);
    let amount = chart.add_series(SeriesKind::Line);
    chart.set_series_visible(volume, false);
    chart.set_series_visible(amount, false);
    install(&mut chart, 0, &data.times[..rows], &data.price[..rows]);
    install(
        &mut chart,
        volume,
        &data.times[..rows],
        &data.volume[..rows],
    );
    install(
        &mut chart,
        amount,
        &data.times[..rows],
        &data.amount[..rows],
    );
    let average = chart.add_vwap_with_amount(0, volume, amount).unwrap();
    apply(&mut chart, average, r##"{"color":"#123456"}"##);
    apply(
        &mut chart,
        0,
        &format!(r##"{{"color":"#654321","break_on_trading_day":{price_breaks}}}"##),
    );
    Intraday {
        chart,
        volume,
        amount,
        average,
    }
}

/// Settle layout, fit every row into view, and build the frame.
fn settle(chart: &mut ChartEngine) -> ChartFrame {
    chart.time_scale.set_width(800.0);
    chart.build_frame();
    chart.fit_content();
    chart.build_frame()
}

/// Point lists of the price pane's polylines stroked in `color`, in paint order.
fn strokes(frame: &ChartFrame, color: Color) -> Vec<Vec<[f32; 2]>> {
    let pane = &frame.panes[0];
    pane.main
        .iter()
        .filter_map(|prim| match prim {
            Prim::Polyline {
                first_point,
                point_count,
                color: stroke,
                ..
            } if *stroke == color => Some(
                pane.points[*first_point as usize..(*first_point + *point_count) as usize].to_vec(),
            ),
            _ => None,
        })
        .collect()
}

/// Point lists of the price pane's area fills, with their gradients.
fn fills(frame: &ChartFrame) -> Vec<(Vec<[f32; 2]>, Gradient)> {
    let pane = &frame.panes[0];
    pane.main
        .iter()
        .filter_map(|prim| match prim {
            Prim::AreaFill {
                first_point,
                point_count,
                gradient,
                ..
            } => Some((
                pane.points[*first_point as usize..(*first_point + *point_count) as usize].to_vec(),
                *gradient,
            )),
            _ => None,
        })
        .collect()
}

/// Fractional logical position of a frame x (device px, dpr 1).
fn logical_at(chart: &ChartEngine, x: f32) -> f64 {
    let hpr = chart.pane_w.round().max(1.0) / chart.pane_w.max(1.0);
    let slot_zero = chart.time_scale.index_to_coordinate(0);
    (f64::from(x) / hpr - slot_zero) / chart.time_scale.bar_spacing()
}

/// The day (group of `rows_per_day` logical rows) every point of `path` belongs to, or `None`
/// when some segment joins two days. A lone row's one-bar segment spans half a bar to each side
/// of its row and stays inside that row's day.
fn path_day(chart: &ChartEngine, path: &[[f32; 2]], rows_per_day: usize) -> Option<usize> {
    let per_day = rows_per_day as f64;
    let logicals: Vec<f64> = path
        .iter()
        .map(|point| logical_at(chart, point[0]))
        .collect();
    let (first, last) = (logicals[0], *logicals.last().unwrap());
    let day = ((first + last) / 2.0 + 0.5).div_euclid(per_day);
    let (start, end) = (day * per_day - 0.5, day * per_day + per_day - 0.5);
    logicals
        .iter()
        .all(|&logical| logical >= start - 0.01 && logical <= end + 0.01)
        .then_some(day as usize)
}

fn days_of(chart: &ChartEngine, paths: &[Vec<[f32; 2]>], rows_per_day: usize) -> Vec<usize> {
    paths
        .iter()
        .map(|path| {
            path_day(chart, path, rows_per_day)
                .unwrap_or_else(|| panic!("a run joins two trading days: {path:?}"))
        })
        .collect()
}

#[test]
fn session_averages_restart_their_line_every_trading_day() {
    let data = session(3, 20);
    let Intraday {
        mut chart,
        volume,
        average,
        ..
    } = intraday(&data, data.times.len(), false);
    let typical = chart.add_vwap(0, Some(volume)).unwrap();
    apply(&mut chart, typical, r##"{"color":"#345612"}"##);
    let frame = settle(&mut chart);

    for color in [AVERAGE, TYPICAL] {
        let runs = strokes(&frame, color);
        assert_eq!(days_of(&chart, &runs, data.minutes), vec![0, 1, 2]);
        assert!(runs.iter().all(|run| run.len() == data.minutes));
    }
    // Each day's run starts at its reset value: the day's first minute alone.
    let values = chart.data.series_data(average).unwrap().1[3].to_vec();
    for day in 0..3 {
        let first = day * data.minutes;
        assert_eq!(values[first], data.amount[first] / data.volume[first]);
    }
    // The price series keeps the default: one run through every day.
    assert_eq!(strokes(&frame, PRICE).len(), 1);
}

#[test]
fn run_breaks_after_incremental_updates_match_full_installs() {
    let data = session(3, 12);
    let n = data.times.len();
    let start = data.minutes + 1;
    let mut live = intraday(&data, start, true);
    for row in start..n {
        for (id, values) in [
            (0, &data.price),
            (live.volume, &data.volume),
            (live.amount, &data.amount),
        ] {
            assert!(live
                .chart
                .update_series_bar(id, data.times[row], [values[row]; 4]));
        }
        let live_frame = settle(&mut live.chart);
        let mut full = intraday(&data, row + 1, true);
        let full_frame = settle(&mut full.chart);
        for color in [AVERAGE, PRICE] {
            let runs = strokes(&live_frame, color);
            assert_eq!(runs, strokes(&full_frame, color), "row {row}");
            let expected: Vec<usize> = (0..=row / data.minutes).collect();
            assert_eq!(days_of(&live.chart, &runs, data.minutes), expected);
        }
    }
    assert_eq!(
        live.chart.data.series_data(live.average).unwrap().1[3],
        intraday(&data, n, true)
            .chart
            .data
            .series_data(live.average)
            .unwrap()
            .1[3]
    );
}

#[test]
fn a_lone_first_row_of_a_period_draws_a_one_bar_segment() {
    let data = session(2, 10);
    let mut chart = intraday(&data, 11, true).chart;
    let frame = settle(&mut chart);
    let hpr = chart.pane_w.round().max(1.0) / chart.pane_w.max(1.0);
    let x = chart.time_scale.index_to_coordinate(10) * hpr;
    let half = chart.time_scale.bar_spacing() * hpr / 2.0;
    for color in [AVERAGE, PRICE] {
        let runs = strokes(&frame, color);
        assert_eq!(days_of(&chart, &runs, data.minutes), vec![0, 1]);
        let lone = &runs[1];
        assert_eq!(lone.len(), 2);
        assert_eq!(lone[0][1], lone[1][1], "horizontal");
        assert!((f64::from(lone[0][0]) - (x - half)).abs() < 1e-3);
        assert!((f64::from(lone[1][0]) - (x + half)).abs() < 1e-3);
    }
}

#[test]
fn an_edge_neighbour_across_a_break_draws_nothing_into_view() {
    // The view shows only the middle day; the neighbours just beyond each edge belong to the
    // days before and after, so each would be a lone run whose real segments lie off-screen.
    let data = session(3, 10);
    let mut chart = intraday(&data, data.times.len(), true).chart;
    settle(&mut chart);
    chart.set_visible_logical_range(10.0, 19.0);
    let frame = chart.build_frame();
    let (from, to) = chart.visible_range_for_frame().unwrap();
    assert!(
        from > 9 && to < 20,
        "neighbour rows 9 and 20 are off-screen"
    );
    for color in [AVERAGE, PRICE] {
        let runs = strokes(&frame, color);
        assert_eq!(days_of(&chart, &runs, data.minutes), vec![1], "{color:?}");
        assert_eq!(runs[0].len(), 10);
    }
}

#[test]
fn trading_day_option_splits_area_fills_and_keeps_one_gradient() {
    let data = session(3, 8);
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    chart.series[0].kind = SeriesKind::Area;
    install(&mut chart, 0, &data.times, &data.price);
    apply(
        &mut chart,
        0,
        r##"{"color":"#654321","area_top_color":"#ff0000","area_bottom_color":"#0000ff"}"##,
    );
    let top = Color::rgb(0xff, 0, 0);
    let bottom = Color::rgb(0, 0, 0xff);
    let frame = settle(&mut chart);
    let unbroken = fills(&frame);
    assert_eq!(unbroken.len(), 1, "default area: one fill");
    assert_eq!(unbroken[0].1, Gradient { top, bottom });
    assert!(path_day(&chart, &unbroken[0].0, data.minutes).is_none());

    apply(&mut chart, 0, r#"{"break_on_trading_day":true}"#);
    let frame = settle(&mut chart);
    let broken = fills(&frame);
    let paths: Vec<_> = broken.iter().map(|(points, _)| points.clone()).collect();
    assert_eq!(days_of(&chart, &paths, data.minutes), vec![0, 1, 2]);
    assert_eq!(
        days_of(&chart, &strokes(&frame, PRICE), data.minutes),
        vec![0, 1, 2]
    );
    // Every run fills down to the pane bottom in the bottom color; the run holding the highest
    // price starts at the top color, lower runs at their slice of the same gradient.
    assert!(broken.iter().all(|(_, gradient)| gradient.bottom == bottom));
    assert_eq!(broken[2].1.top, top);
    assert!(broken[0].1.top != top && broken[0].1.top != bottom);
}

#[test]
fn trading_day_option_splits_baseline_quadrants_and_round_trips() {
    let data = session(3, 8);
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    chart.series[0].kind = SeriesKind::Baseline;
    chart.series[0].baseline = Some(12.1);
    install(&mut chart, 0, &data.times, &data.price);
    let red = Color::rgb(0xaa, 0, 0);
    let green = Color::rgb(0, 0xaa, 0);
    apply(
        &mut chart,
        0,
        r##"{"top_line_color":"#aa0000","bottom_line_color":"#00aa00"}"##,
    );
    let quadrant_paths = |frame: &ChartFrame| {
        let mut paths = strokes(frame, red);
        paths.extend(strokes(frame, green));
        paths.extend(fills(frame).into_iter().map(|(points, _)| points));
        paths
    };
    let frame = settle(&mut chart);
    assert!(
        quadrant_paths(&frame)
            .iter()
            .any(|path| path_day(&chart, path, data.minutes).is_none()),
        "without the option a quadrant run joins two days"
    );

    apply(&mut chart, 0, r#"{"break_on_trading_day":true}"#);
    let frame = settle(&mut chart);
    let days: BTreeSet<usize> = days_of(&chart, &quadrant_paths(&frame), data.minutes)
        .into_iter()
        .collect();
    assert_eq!(days, BTreeSet::from([0, 1, 2]));

    // The option reads back, and survives a style reset: it describes sessions, not styling.
    let options = || -> serde_json::Value {
        serde_json::from_str(&chart.series_options_json(0).unwrap()).unwrap()
    };
    assert_eq!(options()["break_on_trading_day"], true);
    chart.reset_style_to_defaults();
    assert!(chart.series[0].break_on_trading_day);
    apply(&mut chart, 0, r#"{"break_on_trading_day":false}"#);
    assert!(!chart.series[0].break_on_trading_day);
    let default = ChartEngine::new(800.0, 500.0, 1.0);
    let value: serde_json::Value =
        serde_json::from_str(&default.series_options_json(0).unwrap()).unwrap();
    assert_eq!(value["break_on_trading_day"], false);
}

#[test]
fn hit_testing_skips_the_connector_the_frame_omits() {
    let data = session(2, 10);
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    chart.series[0].kind = SeriesKind::Line;
    install(&mut chart, 0, &data.times, &data.price);
    settle(&mut chart);
    let point = |chart: &ChartEngine, row: usize| {
        (
            chart.logical_to_coordinate(row as f64).unwrap(),
            chart
                .series_price_to_coordinate(0, data.price[row])
                .unwrap(),
        )
    };
    let middle = |chart: &ChartEngine, a: usize, b: usize| {
        let (a, b) = (point(chart, a), point(chart, b));
        ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0)
    };
    let (x, y) = middle(&chart, 9, 10);
    let connector = chart.hit_test_one_series(0, x, y).expect("unbroken hit");
    assert_eq!(connector.kind, SeriesHitKind::Line);

    apply(&mut chart, 0, r#"{"break_on_trading_day":true}"#);
    settle(&mut chart);
    let (x, y) = middle(&chart, 9, 10);
    assert_eq!(chart.hit_test_one_series(0, x, y), None);
    for (a, b) in [(3, 4), (12, 13)] {
        let (x, y) = middle(&chart, a, b);
        let hit = chart.hit_test_one_series(0, x, y).expect("run segment hit");
        assert_eq!(hit.kind, SeriesHitKind::Line);
    }
}

#[test]
fn vwap_bands_and_pivots_break_on_their_own_reset_keys() {
    // Daily rows 2024-01-01 (Mon) .. 2024-02-06: weeks start on Jan 8, 15, 22, 29 and Feb 5.
    let times: Vec<f64> = (0..37)
        .map(|day| (MONDAY_2024 + day * DAY) as f64)
        .collect();
    let close: Vec<f64> = (0..37).map(|day| 10.0 + day as f64 * 0.25).collect();
    let run_counts = |reset: VwapReset| {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        install(&mut chart, 0, &times, &close);
        let outputs = chart.add_vwap_bands(0, None, reset, 1.0, 5.0);
        for (index, &output) in outputs.iter().enumerate() {
            apply(
                &mut chart,
                output,
                &format!(r##"{{"color":"#0000{:02x}"}}"##, index + 1),
            );
        }
        let frame = settle(&mut chart);
        (0..outputs.len())
            .map(|index| strokes(&frame, Color::rgb(0, 0, index as u8 + 1)))
            .map(|runs| runs.iter().map(Vec::len).collect::<Vec<_>>())
            .collect::<Vec<_>>()
    };
    // Weekly: Jan 1-7 and four more Monday weeks, the last two days long.
    let weekly = vec![7, 7, 7, 7, 7, 2];
    assert_eq!(run_counts(VwapReset::Weekly), vec![weekly; 5]);
    // Monthly: January (31 days) and February's first six.
    assert_eq!(run_counts(VwapReset::Monthly), vec![vec![31, 6]; 5]);
    // Session VWAP on daily bars: every bar is its own period, drawn as a one-bar segment.
    assert_eq!(run_counts(VwapReset::Session), vec![vec![2; 37]; 5]);

    // Pivots on minute rows: the first session has no prior range, later sessions hold their
    // level for the whole session and never connect to the next one.
    let data = session(3, 6);
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    install(&mut chart, 0, &data.times, &data.price);
    let pivot = chart.add_pivot_points(0, PivotKind::Standard)[0];
    apply(&mut chart, pivot, r##"{"color":"#123456"}"##);
    let frame = settle(&mut chart);
    let runs = strokes(&frame, AVERAGE);
    assert_eq!(days_of(&chart, &runs, data.minutes), vec![1, 2]);
    assert!(runs
        .iter()
        .all(|run| run.iter().all(|point| point[1] == run[0][1])));
}

#[test]
fn session_start_moves_the_breaks_with_the_trading_day() {
    // Chinese futures in Asia/Shanghai: Monday and Tuesday night sessions (21:00, 21:15, ...)
    // and Tuesday and Wednesday day sessions (09:00, 09:15, ...), four rows each.
    let local = |day: i64, hour: i64, minute: i64| {
        (MONDAY + day * DAY + hour * HOUR + minute * 60 - 8 * HOUR) as f64
    };
    let mut times = Vec::new();
    for (day, hour) in [(0, 21), (1, 9), (1, 21), (2, 9)] {
        times.extend((0..4).map(|quarter| local(day, hour, quarter * 15)));
    }
    let close: Vec<f64> = (0..times.len()).map(|row| 10.0 + row as f64).collect();
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    chart.series[0].kind = SeriesKind::Line;
    chart.set_time_zone(UtcOffsetSchedule::fixed(8 * HOUR as i32).unwrap());
    install(&mut chart, 0, &times, &close);
    apply(
        &mut chart,
        0,
        r##"{"color":"#654321","break_on_trading_day":true}"##,
    );
    let average = chart.add_vwap(0, None).unwrap();
    apply(&mut chart, average, r##"{"color":"#123456"}"##);
    let lengths = |chart: &mut ChartEngine| {
        let frame = settle(chart);
        [PRICE, AVERAGE].map(|color| {
            strokes(&frame, color)
                .iter()
                .map(Vec::len)
                .collect::<Vec<_>>()
        })
    };
    // Midnight trading days: each night session is its own day's evening.
    assert_eq!(lengths(&mut chart), [vec![4, 8, 4], vec![4, 8, 4]]);
    // A 21:00 session start opens the next trading day with the night session.
    chart.set_session_start_seconds(-3 * HOUR as i32).unwrap();
    assert_eq!(lengths(&mut chart), [vec![8, 8], vec![8, 8]]);
}

#[test]
fn conflated_rows_break_at_the_same_period_boundaries() {
    let data = session(3, 600);
    let mut chart = intraday(&data, data.times.len(), true).chart;
    chart.set_min_bar_spacing(0.05);
    let frame = settle(&mut chart);
    assert!(
        chart.time_scale.bar_spacing() < 1.0,
        "sub-pixel spacing selects conflated rows"
    );
    for color in [AVERAGE, PRICE] {
        let runs = strokes(&frame, color);
        assert_eq!(days_of(&chart, &runs, data.minutes), vec![0, 1, 2]);
    }
    let x = (chart.logical_to_coordinate(599.0).unwrap()
        + chart.logical_to_coordinate(600.0).unwrap())
        / 2.0;
    let y = (chart
        .series_price_to_coordinate(0, data.price[599])
        .unwrap()
        + chart
            .series_price_to_coordinate(0, data.price[600])
            .unwrap())
        / 2.0;
    assert_eq!(chart.hit_test_one_series(0, x, y), None);
}

#[test]
fn trading_day_option_on_a_non_time_axis_follows_bar_open_times() {
    // Renko bricks from 15-minute source bars over three UTC days, one brick per source bar. The
    // data layer keys the bricks 0, 1, 2, ... (not UTC times), so the trading day must come from
    // each brick's open time, as the time-axis marks do.
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    chart
        .configure_synthetic_bar_series(0, SyntheticBarOptions::RenkoFixed { box_size: 1.0 })
        .unwrap();
    let source: Vec<SyntheticSourceBar> = (0..3 * 96)
        .map(|index| {
            let close = 100.0 + index as f64;
            SyntheticSourceBar {
                timestamp_micros: (MONDAY + index * 900) * 1_000_000,
                open: close,
                high: close,
                low: close,
                close,
            }
        })
        .collect();
    chart.set_synthetic_bar_source(0, source).unwrap();
    assert_eq!(chart.data.merged_times()[..3], [0, 1, 2]);
    let bricks = chart.synthetic_bars(0).unwrap().to_vec();
    let day_of = |brick: &SyntheticBar| brick.open_timestamp_micros.div_euclid(1_000_000 * DAY);
    let mut expected = Vec::new();
    for brick in &bricks {
        match expected.last_mut() {
            Some((day, count)) if *day == day_of(brick) => *count += 1,
            _ => expected.push((day_of(brick), 1)),
        }
    }
    assert_eq!(expected.len(), 3, "the bricks span three trading days");
    let line = chart.add_sma(0, 1).unwrap();
    apply(
        &mut chart,
        line,
        r##"{"color":"#654321","break_on_trading_day":true}"##,
    );
    let frame = settle(&mut chart);
    let lengths: Vec<usize> = strokes(&frame, PRICE).iter().map(Vec::len).collect();
    assert_eq!(
        lengths,
        expected.iter().map(|&(_, count)| count).collect::<Vec<_>>()
    );
}

#[test]
fn whitespace_rows_neither_break_a_day_nor_delay_its_break() {
    let data = session(3, 10);
    let mut close = data.price.clone();
    // The second day opens with two blank minutes and has a blank minute mid-session; the third
    // day's first minute is blank.
    for row in [10, 11, 15, 20] {
        close[row] = f64::NAN;
    }
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    chart.series[0].kind = SeriesKind::Line;
    install(&mut chart, 0, &data.times, &close);
    apply(
        &mut chart,
        0,
        r##"{"color":"#654321","break_on_trading_day":true}"##,
    );
    let frame = settle(&mut chart);
    let runs = strokes(&frame, PRICE);
    assert_eq!(days_of(&chart, &runs, data.minutes), vec![0, 1, 2]);
    // Each day is one run from its first drawn row, across the blank minute inside it.
    assert_eq!(
        runs.iter().map(Vec::len).collect::<Vec<_>>(),
        vec![10, 7, 9]
    );
    let hpr = chart.pane_w.round().max(1.0) / chart.pane_w.max(1.0);
    for (run, first_row) in runs.iter().zip([0, 12, 21]) {
        let x = chart.time_scale.index_to_coordinate(first_row) * hpr;
        assert!((f64::from(run[0][0]) - x).abs() < 1e-3);
    }
}

#[test]
fn retention_trims_while_streaming_break_like_a_fresh_install_of_the_kept_rows() {
    let data = session(4, 10);
    let mut live = intraday(&data, 5, true);
    for id in [0, live.volume, live.amount] {
        assert!(live.chart.set_series_max_points(id, Some(17)));
    }
    for row in 5..data.times.len() {
        for (id, values) in [
            (0, &data.price),
            (live.volume, &data.volume),
            (live.amount, &data.amount),
        ] {
            assert!(live
                .chart
                .update_series_bar(id, data.times[row], [values[row]; 4]));
        }
        let live_frame = settle(&mut live.chart);
        let kept_from = live.chart.data.series_data(0).unwrap().0[0];
        let start = data
            .times
            .iter()
            .position(|&time| time as i64 == kept_from)
            .unwrap();
        assert!(start > 0 || row < 17, "retention trimmed by row {row}");
        let kept = Session {
            minutes: data.minutes,
            times: data.times[start..=row].to_vec(),
            price: data.price[start..=row].to_vec(),
            volume: data.volume[start..=row].to_vec(),
            amount: data.amount[start..=row].to_vec(),
        };
        let mut full = intraday(&kept, kept.times.len(), true);
        let full_frame = settle(&mut full.chart);
        for color in [AVERAGE, PRICE] {
            assert_eq!(
                strokes(&live_frame, color),
                strokes(&full_frame, color),
                "row {row}"
            );
        }
    }
}

#[test]
fn a_day_without_a_break_keeps_the_unbroken_geometry_primitive_for_primitive() {
    // One trading day: the option finds no boundary, so every kind emits exactly the frame it
    // emits without the option (the baseline crosses its reference several times).
    let data = session(1, 30);
    for kind in [SeriesKind::Line, SeriesKind::Area, SeriesKind::Baseline] {
        let frame = |breaks: bool| {
            let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
            chart.series[0].kind = kind;
            chart.series[0].baseline = Some(10.1);
            install(&mut chart, 0, &data.times, &data.price);
            apply(
                &mut chart,
                0,
                &format!(r#"{{"break_on_trading_day":{breaks}}}"#),
            );
            let frame = settle(&mut chart);
            (frame.panes[0].main.clone(), frame.panes[0].points.clone())
        };
        assert_eq!(frame(true), frame(false), "{kind:?}");
    }
}

#[test]
fn an_as_of_vwap_breaks_on_the_row_it_resets_on() {
    use aeris_charts_core::model::data_layer::TimeAlignment;
    let d = MONDAY;
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    chart.series[0].kind = SeriesKind::Line;
    chart.set_time_zone(UtcOffsetSchedule::fixed(0).unwrap());
    let main_times = [
        d + 10 * HOUR,
        d + 14 * HOUR,
        d + DAY + 2 * HOUR,
        d + DAY + 10 * HOUR,
    ];
    install(
        &mut chart,
        0,
        &main_times.map(|time| time as f64),
        &[1.0, 2.0, 3.0, 4.0],
    );
    // The overlay trades before each main bar; its D+1 row lands between the axis's D+1 points.
    let overlay = chart.add_series(SeriesKind::Line);
    let overlay_times = [d + 9 * HOUR, d + 13 * HOUR, d + DAY + 9 * HOUR + 1_800];
    install(
        &mut chart,
        overlay,
        &overlay_times.map(|time| time as f64),
        &[10.0, 20.0, 50.0],
    );
    chart
        .set_series_time_alignment(
            overlay,
            TimeAlignment::AsOf {
                max_staleness: None,
            },
        )
        .unwrap();
    let vwap = chart.add_vwap(overlay, None).unwrap();
    apply(&mut chart, vwap, r##"{"color":"#123456"}"##);
    let frame = settle(&mut chart);
    let plot = chart.data_layer().plot(vwap);
    let rows = (0..plot.size()).collect::<Vec<_>>();
    // The D+1 02:00 point still shows session D's row (its VWAP 15); the reset row is the last.
    assert_eq!(
        rows.iter()
            .map(|&row| plot.source_row(row))
            .collect::<Vec<_>>(),
        [0, 1, 1, 2]
    );
    assert_eq!(chart.line_run_breaks(vwap, plot, &rows), [3]);
    assert_eq!(
        strokes(&frame, AVERAGE)
            .iter()
            .map(Vec::len)
            .collect::<Vec<_>>(),
        [3, 2],
        "session D as one run, then the reset row's one-bar segment"
    );
    let point = |row: usize, value: f64| {
        (
            chart.logical_to_coordinate(row as f64).unwrap(),
            chart.series_price_to_coordinate(vwap, value).unwrap(),
        )
    };
    let middle = |a: (f64, f64), b: (f64, f64)| ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0);
    let (x, y) = middle(point(2, 15.0), point(3, 50.0));
    assert_eq!(
        chart.hit_test_one_series(vwap, x, y),
        None,
        "no connector across the reset"
    );
    let (x, y) = middle(point(1, 15.0), point(2, 15.0));
    assert_eq!(
        chart.hit_test_one_series(vwap, x, y).map(|hit| hit.kind),
        Some(SeriesHitKind::Line),
        "session D stays one run"
    );
}
