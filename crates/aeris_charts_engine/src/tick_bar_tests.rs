//! Ordinary time-axis candles built from ticks: exchange-session anchoring, auction and lunch
//! prints, live roll-over, the stream volume histogram, replay, and retention.

use aeris_charts_core::scale::session_slots::{parse_iso_date, parse_wall_clock};

use crate::{
    AggressorSide, ChartEngine, FootprintAggregationOptions, FootprintAggregator,
    FootprintBarAggregation, FootprintError, FootprintSeriesOptions, FootprintTrade,
    FootprintUpdateKind, OutOfSessionPolicy, SeriesId, SeriesKind, SessionSlotError, SessionWindow,
    TradeSessionOptions, UtcOffsetSchedule, UtcOffsetTransition,
};

const DAY: i64 = 86_400;
const HOUR: i64 = 3_600;
const MICROS: i64 = 1_000_000;

fn shanghai() -> UtcOffsetSchedule {
    UtcOffsetSchedule::fixed(8 * 3_600).unwrap()
}

fn new_york() -> UtcOffsetSchedule {
    let ts = |date: &str, hour: i64| parse_iso_date(date).unwrap() * DAY + hour * HOUR;
    UtcOffsetSchedule::new(vec![
        UtcOffsetTransition {
            from_utc_seconds: ts("2023-11-05", 6),
            offset_seconds: -5 * 3_600,
        },
        UtcOffsetTransition {
            from_utc_seconds: ts("2024-03-10", 7),
            offset_seconds: -4 * 3_600,
        },
    ])
    .unwrap()
}

/// UTC seconds of an exchange-local `"YYYY-MM-DD HH:MM[:SS]"` in `zone`.
fn at(zone: &UtcOffsetSchedule, text: &str) -> i64 {
    let (date, clock) = text.split_once(' ').unwrap();
    zone.to_utc(
        parse_iso_date(date).unwrap() * DAY + i64::from(parse_wall_clock(clock, false).unwrap()),
    )
}

fn window(start: &str, end: &str) -> SessionWindow {
    SessionWindow {
        start_seconds: parse_wall_clock(start, false).unwrap(),
        end_seconds: parse_wall_clock(end, true).unwrap(),
    }
}

fn a_share_sessions(outside: OutOfSessionPolicy) -> TradeSessionOptions {
    TradeSessionOptions {
        windows: vec![window("09:30", "11:30"), window("13:00", "15:00")],
        outside,
    }
}

fn trade(zone: &UtcOffsetSchedule, text: &str, price: f64, volume: f64) -> FootprintTrade {
    FootprintTrade {
        timestamp_micros: at(zone, text) * MICROS,
        price,
        volume,
        aggressor: AggressorSide::Buy,
        bid: None,
        ask: None,
        sequence: None,
        trade_id: None,
        conditions: 0,
        session_id: Some(1),
    }
}

fn time_bars(interval_seconds: i64, anchor_seconds: i64) -> FootprintAggregationOptions {
    FootprintAggregationOptions {
        tick_size: 0.01,
        ticks_per_row: 1,
        bars: FootprintBarAggregation::Time {
            interval_micros: (interval_seconds * MICROS) as u64,
            anchor_micros: anchor_seconds * MICROS,
        },
        ..FootprintAggregationOptions::default()
    }
}

/// One A-share day of prints: the opening auction, continuous trading around every 60-minute
/// boundary, the 11:30 print, and the closing auction.
fn a_share_day(date: &str) -> Vec<FootprintTrade> {
    let zone = shanghai();
    [
        ("09:25:00", 10.00, 500.0),
        ("09:30:01", 10.02, 100.0),
        ("10:29:59", 10.05, 100.0),
        ("10:30:00", 10.04, 100.0),
        ("11:29:59", 10.03, 100.0),
        ("11:30:00", 10.01, 50.0),
        ("13:00:02", 10.06, 100.0),
        ("14:00:00", 10.07, 100.0),
        ("14:59:59", 10.08, 100.0),
        ("15:00:00", 10.10, 800.0),
    ]
    .into_iter()
    .map(|(clock, price, volume)| trade(&zone, &format!("{date} {clock}"), price, volume))
    .collect()
}

struct TickChart {
    chart: ChartEngine,
    stream: u64,
    candles: SeriesId,
    volume: SeriesId,
}

fn tick_chart(zone: UtcOffsetSchedule, options: FootprintAggregationOptions) -> TickChart {
    let mut chart = ChartEngine::new(800.0, 400.0, 1.0);
    chart.set_time_zone(zone);
    let stream = chart.add_trade_stream("SSE:600000", options).unwrap();
    let candles = chart.add_series(SeriesKind::Candlestick);
    chart
        .bind_trade_bar_series_to_stream(candles, stream)
        .unwrap();
    let volume = chart.add_trade_volume_series(stream, 1).unwrap();
    TickChart {
        chart,
        stream,
        candles,
        volume,
    }
}

type Row = (i64, [f64; 4]);

fn rows(chart: &ChartEngine, id: SeriesId) -> Vec<Row> {
    let (times, columns) = chart.data_layer().series_data(id).unwrap();
    times
        .iter()
        .enumerate()
        .map(|(row, &time)| {
            (
                time,
                [
                    columns[0][row],
                    columns[1][row],
                    columns[2][row],
                    columns[3][row],
                ],
            )
        })
        .collect()
}

fn local_times(zone: &UtcOffsetSchedule, rows: &[Row]) -> Vec<String> {
    rows.iter()
        .map(|&(time, _)| {
            let local = zone.to_local(time).rem_euclid(DAY);
            format!("{:02}:{:02}", local / HOUR, local % HOUR / 60)
        })
        .collect()
}

fn closes(rows: &[Row]) -> Vec<f64> {
    rows.iter().map(|&(_, values)| values[3]).collect()
}

#[test]
fn a_share_hour_candles_from_ticks_open_at_each_session_window() {
    let zone = shanghai();
    let anchor = at(&zone, "2026-09-25 09:30:00");
    let mut tick = tick_chart(zone.clone(), time_bars(HOUR, anchor));
    tick.chart
        .set_trade_stream_trades(tick.stream, a_share_day("2026-09-25"))
        .unwrap();
    // The plain anchor grid stays the default: one 09:30 anchor puts the auction in an 08:30
    // bar and shifts the whole afternoon to 12:30/13:30/14:30.
    assert_eq!(
        local_times(&zone, &rows(&tick.chart, tick.candles)),
        ["08:30", "09:30", "10:30", "11:30", "12:30", "13:30", "14:30"]
    );

    tick.chart
        .set_trade_stream_sessions(
            tick.stream,
            Some(a_share_sessions(OutOfSessionPolicy::Fold)),
        )
        .unwrap();
    let candles = rows(&tick.chart, tick.candles);
    assert_eq!(
        local_times(&zone, &candles),
        ["09:30", "10:30", "13:00", "14:00"]
    );
    let ohlc = candles
        .iter()
        .map(|&(_, values)| values)
        .collect::<Vec<_>>();
    // The opening auction opens the first bar; the 11:30 and 15:00 prints close their window.
    assert_eq!(ohlc[0], [10.00, 10.05, 10.00, 10.05]);
    assert_eq!(ohlc[1], [10.04, 10.04, 10.01, 10.01]);
    assert_eq!(ohlc[2], [10.06, 10.06, 10.06, 10.06]);
    assert_eq!(ohlc[3], [10.07, 10.10, 10.07, 10.10]);
    let volume = rows(&tick.chart, tick.volume);
    assert_eq!(
        volume.iter().map(|&(time, _)| time).collect::<Vec<_>>(),
        candles.iter().map(|&(time, _)| time).collect::<Vec<_>>()
    );
    assert_eq!(closes(&volume), [700.0, 250.0, 100.0, 1_000.0]);
    let series = tick.chart.series_entry(tick.volume).unwrap();
    assert!(series.histogram_updown);
    assert_eq!(series.price_format.kind, crate::PriceFormatKind::Volume);

    // Clearing the sessions restores the anchor grid exactly.
    tick.chart
        .set_trade_stream_sessions(tick.stream, None)
        .unwrap();
    assert_eq!(rows(&tick.chart, tick.candles).len(), 7);
}

#[test]
fn a_share_minute_candles_skip_the_lunch_break_and_fold_closing_prints() {
    let zone = shanghai();
    let mut tick = tick_chart(zone.clone(), time_bars(60, 0));
    tick.chart
        .set_trade_stream_sessions(
            tick.stream,
            Some(a_share_sessions(OutOfSessionPolicy::Fold)),
        )
        .unwrap();
    tick.chart
        .set_trade_stream_trades(tick.stream, a_share_day("2026-09-25"))
        .unwrap();
    let candles = rows(&tick.chart, tick.candles);
    assert_eq!(
        local_times(&zone, &candles),
        ["09:30", "10:29", "10:30", "11:29", "13:00", "14:00", "14:59"]
    );
    // 11:29 and 13:00 are neighbours on the ordinal time axis: the lunch break takes no width.
    assert_eq!(
        tick.chart.data_layer().merged_times(),
        candles.iter().map(|&(time, _)| time).collect::<Vec<_>>()
    );
    assert_eq!(closes(&candles)[3], 10.01);
    assert_eq!(closes(&candles)[6], 10.10);
    assert_eq!(
        closes(&rows(&tick.chart, tick.volume)),
        [600.0, 100.0, 100.0, 150.0, 100.0, 100.0, 900.0]
    );
}

#[test]
fn live_ticks_roll_into_the_next_session_bar_incrementally() {
    let zone = shanghai();
    let mut tick = tick_chart(zone.clone(), time_bars(HOUR, 0));
    tick.chart
        .set_trade_stream_sessions(
            tick.stream,
            Some(a_share_sessions(OutOfSessionPolicy::Fold)),
        )
        .unwrap();
    let day = a_share_day("2026-09-25");
    tick.chart
        .set_trade_stream_trades(tick.stream, day[..3].to_vec())
        .unwrap();
    let rebuilds = tick
        .chart
        .trade_stream_stats(tick.stream)
        .unwrap()
        .dependent_rebuilds;
    let mut counts = Vec::new();
    for print in &day[3..] {
        assert_eq!(
            tick.chart
                .update_trade_stream_trade(tick.stream, print.clone())
                .unwrap(),
            FootprintUpdateKind::Tip
        );
        counts.push(rows(&tick.chart, tick.candles).len());
    }
    // 10:30:00 opens the second bar, 11:29:59 and the 11:30 print stay in it, 13:00:02 and
    // 14:00:00 open the afternoon bars, and the closing prints stay in the 14:00 bar.
    assert_eq!(counts, [2, 2, 2, 3, 4, 4, 4]);
    let stats = tick.chart.trade_stream_stats(tick.stream).unwrap();
    assert_eq!(stats.dependent_rebuilds, rebuilds);
    assert!(stats.dependent_incremental_updates >= 7);
    // The live result equals a fresh load of the same tape.
    let mut fresh = tick_chart(zone, time_bars(HOUR, 0));
    fresh
        .chart
        .set_trade_stream_sessions(
            fresh.stream,
            Some(a_share_sessions(OutOfSessionPolicy::Fold)),
        )
        .unwrap();
    fresh
        .chart
        .set_trade_stream_trades(fresh.stream, day)
        .unwrap();
    assert_eq!(
        rows(&tick.chart, tick.candles),
        rows(&fresh.chart, fresh.candles)
    );
    assert_eq!(
        rows(&tick.chart, tick.volume),
        rows(&fresh.chart, fresh.volume)
    );
}

#[test]
fn session_candles_replay_like_a_fresh_load_at_every_clock() {
    let zone = shanghai();
    let tape = [a_share_day("2026-09-24"), a_share_day("2026-09-25")].concat();
    let mut tick = tick_chart(zone.clone(), time_bars(HOUR, 0));
    tick.chart
        .set_trade_stream_sessions(
            tick.stream,
            Some(a_share_sessions(OutOfSessionPolicy::Fold)),
        )
        .unwrap();
    tick.chart
        .set_trade_stream_trades(tick.stream, tape.clone())
        .unwrap();
    for clock in [
        "2026-09-25 11:00:00",
        "2026-09-25 15:00:00",
        "2026-09-24 09:28:00",
        "2026-09-25 13:30:00",
        "2026-09-24 14:00:00",
    ] {
        let clock_micros = at(&zone, clock) * MICROS;
        tick.chart
            .set_replay_clock_micros(Some(clock_micros))
            .unwrap();
        let mut fresh = tick_chart(zone.clone(), time_bars(HOUR, 0));
        fresh
            .chart
            .set_trade_stream_sessions(
                fresh.stream,
                Some(a_share_sessions(OutOfSessionPolicy::Fold)),
            )
            .unwrap();
        fresh
            .chart
            .set_trade_stream_trades(
                fresh.stream,
                tape.iter()
                    .filter(|print| print.timestamp_micros <= clock_micros)
                    .cloned()
                    .collect(),
            )
            .unwrap();
        // The chart clock also masks time-domain rows after it, so a bar that a folded
        // opening-auction print opens ahead of the clock appears when the clock reaches it.
        fresh
            .chart
            .set_replay_clock_micros(Some(clock_micros))
            .unwrap();
        assert_eq!(
            rows(&tick.chart, tick.candles),
            rows(&fresh.chart, fresh.candles),
            "{clock}"
        );
        assert_eq!(
            rows(&tick.chart, tick.volume),
            rows(&fresh.chart, fresh.volume),
            "{clock}"
        );
    }
    tick.chart
        .set_replay_clock_micros(Some(at(&zone, "2026-09-24 09:28:00") * MICROS))
        .unwrap();
    assert!(rows(&tick.chart, tick.candles).is_empty());
    tick.chart
        .set_replay_clock_micros(Some(at(&zone, "2026-09-24 09:30:00") * MICROS))
        .unwrap();
    assert_eq!(
        rows(&tick.chart, tick.candles),
        [(
            at(&zone, "2026-09-24 09:30:00"),
            [10.00, 10.00, 10.00, 10.00]
        )]
    );
    tick.chart.set_replay_clock_micros(None).unwrap();
    assert_eq!(rows(&tick.chart, tick.candles).len(), 8);
}

#[test]
fn exchange_time_changes_re_place_the_session_windows() {
    let zone = shanghai();
    let mut tick = tick_chart(UtcOffsetSchedule::utc(), time_bars(HOUR, 0));
    tick.chart
        .set_trade_stream_sessions(
            tick.stream,
            Some(a_share_sessions(OutOfSessionPolicy::Fold)),
        )
        .unwrap();
    tick.chart
        .set_trade_stream_trades(tick.stream, a_share_day("2026-09-25"))
        .unwrap();
    // In UTC every Shanghai print precedes the 09:30 UTC window and folds into its first bar.
    assert_eq!(rows(&tick.chart, tick.candles).len(), 1);
    tick.chart.set_time_zone(zone.clone());
    assert_eq!(
        local_times(&zone, &rows(&tick.chart, tick.candles)),
        ["09:30", "10:30", "13:00", "14:00"]
    );
    assert_eq!(
        closes(&rows(&tick.chart, tick.volume)),
        [700.0, 250.0, 100.0, 1_000.0]
    );
}

#[test]
fn excluded_extended_hours_keep_regular_bars_across_dst() {
    let zone = new_york();
    let mut tick = tick_chart(zone.clone(), time_bars(HOUR, 0));
    tick.chart
        .set_trade_stream_sessions(
            tick.stream,
            Some(TradeSessionOptions {
                windows: vec![window("09:30", "16:00")],
                outside: OutOfSessionPolicy::Exclude,
            }),
        )
        .unwrap();
    let tape = ["2024-03-08", "2024-03-11"]
        .into_iter()
        .flat_map(|date| {
            [
                ("08:00:00", 50.00, 7.0),
                ("09:30:00", 50.10, 1.0),
                ("15:59:00", 50.20, 1.0),
                ("16:00:00", 50.30, 5.0),
                ("17:00:00", 50.40, 3.0),
            ]
            .map(|(clock, price, volume)| trade(&zone, &format!("{date} {clock}"), price, volume))
        })
        .collect::<Vec<_>>();
    tick.chart
        .set_trade_stream_trades(tick.stream, tape)
        .unwrap();
    let candles = rows(&tick.chart, tick.candles);
    assert_eq!(
        local_times(&zone, &candles),
        ["09:30", "15:30", "09:30", "15:30"]
    );
    // 09:30 Eastern is 14:30 UTC before the DST change and 13:30 UTC after it.
    assert_eq!(
        candles[0].0,
        parse_iso_date("2024-03-08").unwrap() * DAY + 14 * HOUR + 1_800
    );
    assert_eq!(
        candles[2].0,
        parse_iso_date("2024-03-11").unwrap() * DAY + 13 * HOUR + 1_800
    );
    // Pre- and after-market prints join no bar; the 16:00 closing cross closes the day.
    assert_eq!(closes(&candles), [50.10, 50.30, 50.10, 50.30]);
    assert_eq!(
        closes(&rows(&tick.chart, tick.volume)),
        [1.0, 6.0, 1.0, 6.0]
    );
}

#[test]
fn retention_keeps_a_folded_auction_print_with_its_bar() {
    let zone = shanghai();
    let mut stream = FootprintAggregator::new(time_bars(HOUR, 0)).unwrap();
    let mut exchange = crate::ExchangeTime::default();
    exchange.set_offsets(zone.clone());
    stream
        .set_sessions(Some(&a_share_sessions(OutOfSessionPolicy::Fold)), &exchange)
        .unwrap();
    stream
        .set_trades(vec![
            trade(&zone, "2026-09-24 09:31:00", 10.00, 1.0),
            trade(&zone, "2026-09-24 14:10:00", 10.20, 2.0),
            trade(&zone, "2026-09-25 09:25:00", 10.40, 300.0),
            trade(&zone, "2026-09-25 09:31:00", 10.50, 4.0),
        ])
        .unwrap();
    assert_eq!(stream.bars().len(), 3);
    stream.retain_last_bars(1);
    let bars = stream.bars();
    assert_eq!(bars.len(), 1);
    assert_eq!(
        bars[0].start_timestamp_micros,
        at(&zone, "2026-09-25 09:30:00") * MICROS
    );
    assert_eq!((bars[0].open, bars[0].total_volume), (10.40, 304.0));
    assert_eq!(stream.trades().len(), 2);
}

/// Local `HH:MM` of every bubble marker on `series`.
fn bubble_times(chart: &ChartEngine, zone: &UtcOffsetSchedule, series: SeriesId) -> Vec<String> {
    let markers = &chart.series_entry(series).unwrap().markers;
    let rows = markers
        .iter()
        .map(|marker| (marker.time, [0.0; 4]))
        .collect::<Vec<_>>();
    local_times(zone, &rows)
}

#[test]
fn trade_bubbles_sit_on_the_bar_holding_their_print() {
    let zone = shanghai();
    let bubbles = crate::TradeBubbleOptions {
        minimum_volume: 0.0,
        max_markers: 64,
        aggregation_window_micros: 0,
    };
    // Plain grid: a print inside a bar used to carry its own second, which markers snap to the
    // NEXT bar.
    let mut plain = tick_chart(
        zone.clone(),
        time_bars(HOUR, at(&zone, "2026-09-25 09:30:00")),
    );
    plain
        .chart
        .set_trade_stream_trades(plain.stream, a_share_day("2026-09-25")[1..5].to_vec())
        .unwrap();
    plain
        .chart
        .add_trade_bubbles(plain.stream, plain.candles, bubbles)
        .unwrap();
    assert_eq!(
        bubble_times(&plain.chart, &zone, plain.candles),
        ["09:30", "09:30", "10:30", "10:30"]
    );
    let candle_times = rows(&plain.chart, plain.candles)
        .iter()
        .map(|&(time, _)| time)
        .collect::<Vec<_>>();
    for marker in &plain.chart.series_entry(plain.candles).unwrap().markers {
        assert!(candle_times.contains(&marker.time), "{}", marker.time);
    }
    // Session anchoring: the auction print sits on the 09:30 bar and the 11:30 print on the
    // 10:30 bar; excluded pre-open and lunch prints have no bar and no bubble.
    let mut tape = a_share_day("2026-09-25");
    tape.insert(6, trade(&zone, "2026-09-25 12:10:00", 10.00, 5.0));
    for (outside, expected) in [
        (
            OutOfSessionPolicy::Fold,
            &[
                "09:30", "09:30", "09:30", "10:30", "10:30", "10:30", "10:30", "13:00", "14:00",
                "14:00", "14:00",
            ][..],
        ),
        (
            OutOfSessionPolicy::Exclude,
            &[
                "09:30", "09:30", "10:30", "10:30", "10:30", "13:00", "14:00", "14:00", "14:00",
            ][..],
        ),
    ] {
        let mut tick = tick_chart(zone.clone(), time_bars(HOUR, 0));
        tick.chart
            .set_trade_stream_sessions(tick.stream, Some(a_share_sessions(outside)))
            .unwrap();
        tick.chart
            .set_trade_stream_trades(tick.stream, tape.clone())
            .unwrap();
        tick.chart
            .add_trade_bubbles(tick.stream, tick.candles, bubbles)
            .unwrap();
        assert_eq!(
            bubble_times(&tick.chart, &zone, tick.candles),
            expected,
            "{outside:?}"
        );
    }
}

#[test]
fn sessions_require_whole_second_time_bars_and_valid_windows() {
    let zone = shanghai();
    let mut chart = ChartEngine::new(800.0, 400.0, 1.0);
    chart.set_time_zone(zone.clone());
    let ticks = chart
        .add_trade_stream(
            "SSE:600000:ticks",
            FootprintAggregationOptions {
                bars: FootprintBarAggregation::Trades { trades_per_bar: 10 },
                ..time_bars(60, 0)
            },
        )
        .unwrap();
    assert_eq!(
        chart.set_trade_stream_sessions(ticks, Some(a_share_sessions(OutOfSessionPolicy::Fold))),
        Err(FootprintError::UnsupportedChartAggregation)
    );
    // Clearing sessions a stream never had is a no-op, whatever its bar type.
    assert_eq!(chart.set_trade_stream_sessions(ticks, None), Ok(()));
    let minutes = chart
        .add_trade_stream("SSE:600000:1m", time_bars(60, 0))
        .unwrap();
    assert_eq!(
        chart.set_trade_stream_sessions(
            minutes,
            Some(TradeSessionOptions {
                windows: vec![window("13:00", "15:00"), window("09:30", "11:30")],
                outside: OutOfSessionPolicy::Fold,
            })
        ),
        Err(FootprintError::InvalidSessions(
            SessionSlotError::UnorderedWindow { index: 1 }
        ))
    );
    assert_eq!(
        chart.set_trade_stream_sessions(99, None),
        Err(FootprintError::UnknownTradeStream(99))
    );
    let mut sub_second = FootprintAggregator::new(FootprintAggregationOptions {
        bars: FootprintBarAggregation::Time {
            interval_micros: 500_000,
            anchor_micros: 0,
        },
        ..time_bars(60, 0)
    })
    .unwrap();
    assert_eq!(
        sub_second.set_sessions(
            Some(&a_share_sessions(OutOfSessionPolicy::Fold)),
            &crate::ExchangeTime::default()
        ),
        Err(FootprintError::UnsupportedChartAggregation)
    );
}

/// A footprint over a Fold-anchored stream, loaded with `tape` and clocked to `clock`.
fn session_footprint(
    aggregation: FootprintAggregationOptions,
    tape: Vec<FootprintTrade>,
    clock: Option<i64>,
) -> (ChartEngine, u64, SeriesId) {
    let mut chart = ChartEngine::new(800.0, 400.0, 1.0);
    chart.set_time_zone(shanghai());
    let stream = chart.add_trade_stream("SSE:600000", aggregation).unwrap();
    chart
        .set_trade_stream_sessions(stream, Some(a_share_sessions(OutOfSessionPolicy::Fold)))
        .unwrap();
    let footprint = chart.add_series(SeriesKind::Footprint);
    chart
        .configure_footprint_series(
            footprint,
            FootprintSeriesOptions {
                aggregation,
                ..FootprintSeriesOptions::default()
            },
        )
        .unwrap();
    chart
        .bind_footprint_series_to_stream(footprint, stream)
        .unwrap();
    chart.set_trade_stream_trades(stream, tape).unwrap();
    chart.set_replay_clock_micros(clock).unwrap();
    (chart, stream, footprint)
}

/// Changing a footprint's aggregation re-aggregates its stream in place: the session anchoring
/// (re-placed on the new interval), the replay clock, and the prints it hides all survive.
#[test]
fn footprint_option_changes_keep_sessions_the_replay_clock_and_hidden_prints() {
    let zone = shanghai();
    let tape = [a_share_day("2024-01-02"), a_share_day("2024-01-03")].concat();
    let clock = Some(at(&zone, "2024-01-03 10:30:00") * MICROS);
    let (mut chart, stream, footprint) = session_footprint(time_bars(HOUR, 0), tape.clone(), clock);
    let regrouped = FootprintAggregationOptions {
        ticks_per_row: 2,
        ..time_bars(30 * 60, 0)
    };
    chart
        .apply_footprint_series_options(
            footprint,
            FootprintSeriesOptions {
                aggregation: regrouped,
                ..FootprintSeriesOptions::default()
            },
        )
        .unwrap();
    let (mut reference, _, reference_footprint) = session_footprint(regrouped, tape, clock);
    assert!(chart.trade_stream(stream).unwrap().session_grid().is_some());
    assert_eq!(chart.replay_clock_micros(), clock);
    assert_eq!(
        chart.footprint_bars(footprint),
        reference.footprint_bars(reference_footprint)
    );
    let footprint_rows = rows(&chart, footprint);
    assert_eq!(footprint_rows, rows(&reference, reference_footprint));
    // Half-hour bars restart at each window open; the auction folds into 09:30 and the closing
    // prints into 11:00 and 14:30.
    assert_eq!(
        local_times(&zone, &footprint_rows[..7]),
        ["09:30", "10:00", "10:30", "11:00", "13:00", "14:00", "14:30"]
    );
    for chart in [&mut chart, &mut reference] {
        chart.set_replay_clock_micros(None).unwrap();
    }
    assert_eq!(
        chart.footprint_bars(footprint),
        reference.footprint_bars(reference_footprint)
    );
    assert_eq!(
        rows(&chart, footprint),
        rows(&reference, reference_footprint)
    );

    // Sessions need time bars: a switch to trade-count bars is refused and changes nothing.
    let before = chart.footprint_bars(footprint);
    assert_eq!(
        chart.apply_footprint_series_options(
            footprint,
            FootprintSeriesOptions {
                aggregation: FootprintAggregationOptions {
                    bars: FootprintBarAggregation::Trades { trades_per_bar: 3 },
                    ..regrouped
                },
                ..FootprintSeriesOptions::default()
            },
        ),
        Err(FootprintError::UnsupportedChartAggregation)
    );
    assert_eq!(chart.footprint_bars(footprint), before);
}

/// Folding a lunch print into the morning's last bar would give that bar two sessions. The
/// session change is refused although the replay clock still hides the lunch print.
#[test]
fn a_session_change_that_folds_a_hidden_print_into_another_session_is_refused() {
    let zone = shanghai();
    let mut tick = tick_chart(zone.clone(), time_bars(300, 0));
    let mut lunch = trade(&zone, "2024-01-02 12:10:00", 10.02, 100.0);
    lunch.session_id = Some(2);
    tick.chart
        .set_trade_stream_trades(
            tick.stream,
            vec![trade(&zone, "2024-01-02 11:26:00", 10.01, 100.0), lunch],
        )
        .unwrap();
    tick.chart
        .set_replay_clock_micros(Some(at(&zone, "2024-01-02 11:27:00") * MICROS))
        .unwrap();
    let before = rows(&tick.chart, tick.candles);
    assert_eq!(
        tick.chart.set_trade_stream_sessions(
            tick.stream,
            Some(a_share_sessions(OutOfSessionPolicy::Fold))
        ),
        Err(FootprintError::ProjectionTimeCollision)
    );
    assert!(tick
        .chart
        .trade_stream(tick.stream)
        .unwrap()
        .session_grid()
        .is_none());
    assert_eq!(rows(&tick.chart, tick.candles), before);
    // Excluding the lunch print leaves nothing to collide.
    tick.chart
        .set_trade_stream_sessions(
            tick.stream,
            Some(a_share_sessions(OutOfSessionPolicy::Exclude)),
        )
        .unwrap();
    tick.chart.set_replay_clock_micros(None).unwrap();
    assert_eq!(
        local_times(&zone, &rows(&tick.chart, tick.candles)),
        ["11:25"]
    );
}

/// The retention seed survives an aggregation change: the retained bars keep the session delta
/// their evicted history established, exactly like a capped load under the new options.
#[test]
fn footprint_option_changes_keep_the_retention_seed() {
    let tape = [a_share_day("2024-01-02"), a_share_day("2024-01-03")].concat();
    let capped = |aggregation| {
        let (mut chart, stream, footprint) = session_footprint(aggregation, Vec::new(), None);
        assert!(chart.set_series_max_points(footprint, Some(6)));
        chart.set_trade_stream_trades(stream, tape.clone()).unwrap();
        (chart, footprint)
    };
    let (mut chart, footprint) = capped(time_bars(HOUR, 0));
    let regrouped = FootprintAggregationOptions {
        ticks_per_row: 2,
        ..time_bars(HOUR, 0)
    };
    chart
        .apply_footprint_series_options(
            footprint,
            FootprintSeriesOptions {
                aggregation: regrouped,
                ..FootprintSeriesOptions::default()
            },
        )
        .unwrap();
    let (reference, reference_footprint) = capped(regrouped);
    let bars = chart.footprint_bars(footprint).unwrap();
    assert_eq!(bars.len(), 6);
    // Every print buys in one session, so the first retained bar's session delta carries the
    // two evicted bars.
    assert!(bars[0].session_delta > bars[0].delta);
    assert_eq!(Some(bars), reference.footprint_bars(reference_footprint));
}

/// Candles bound to the stream present the same bars, so they follow a footprint's aggregation
/// change, onto the sequence axis too; that axis is refused while resampling shares the chart.
#[test]
fn footprint_option_changes_move_the_bound_candles_with_the_stream() {
    let zone = shanghai();
    let tape = [a_share_day("2024-01-02"), a_share_day("2024-01-03")].concat();
    let (mut chart, stream, footprint) = session_footprint(time_bars(HOUR, 0), tape, None);
    let candles = chart.add_series(SeriesKind::Candlestick);
    chart
        .bind_trade_bar_series_to_stream(candles, stream)
        .unwrap();
    let apply = |chart: &mut ChartEngine, aggregation| {
        chart.apply_footprint_series_options(
            footprint,
            FootprintSeriesOptions {
                aggregation,
                ..FootprintSeriesOptions::default()
            },
        )
    };
    apply(&mut chart, time_bars(30 * 60, 0)).unwrap();
    let footprint_rows = rows(&chart, footprint);
    assert_eq!(rows(&chart, candles), footprint_rows);
    assert_eq!(
        local_times(&zone, &footprint_rows[..4]),
        ["09:30", "10:00", "10:30", "11:00"]
    );

    let trade_bars = FootprintAggregationOptions {
        bars: FootprintBarAggregation::Trades { trades_per_bar: 3 },
        ..time_bars(30 * 60, 0)
    };
    chart.set_trade_stream_sessions(stream, None).unwrap();
    let resampled = chart.add_series(SeriesKind::Candlestick);
    chart
        .configure_resampled_series(
            candles,
            None,
            resampled,
            None,
            crate::ResampleOptions {
                interval_seconds: HOUR as u32,
                boundaries: vec![crate::ResampleBoundary {
                    start_time: at(&zone, "2024-01-02 00:00"),
                    end_time: at(&zone, "2024-01-04 00:00"),
                    session_id: 1,
                }],
            },
        )
        .unwrap();
    let before = rows(&chart, candles);
    assert_eq!(
        apply(&mut chart, trade_bars),
        Err(FootprintError::SequenceDomainInUse)
    );
    assert_eq!(rows(&chart, candles), before);
    chart.remove_series(resampled);
    apply(&mut chart, trade_bars).unwrap();
    let keys = rows(&chart, footprint);
    assert_eq!(keys.len(), 7);
    assert_eq!(rows(&chart, candles), keys);
    assert_eq!(chart.sequence_points().map(<[_]>::len), Some(7));
}
