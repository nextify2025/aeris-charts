//! Live-tip equivalence and work-bound tests for chart-level trade streams.
//!
//! Every dependent of a stream (footprint projection, trade-bound candles, CVD/delta/volume
//! studies and large-trade bubbles) must end a live tip in exactly the state a clean rebuild
//! produces, while doing work proportional to the changed bar suffix and the newly appended trades
//! only, including on session-anchored time bars.

use super::*;
use crate::{ChartEngine, SeriesKind};

/// A minute-aligned Unix time in microseconds.
const BASE_MICROS: i64 = 1_699_999_980_000_000;

/// Deterministic tape: trades arrive in pairs 250 ms apart every 7 s (several per minute bar);
/// every third pair repeats side and price so bubble aggregation windows merge it; periodic
/// large prints move the peak bubble volume; sessions change only on minute boundaries.
fn tape_trade(index: usize) -> FootprintTrade {
    let pair = index / 2;
    let timestamp_micros = BASE_MICROS + pair as i64 * 7_000_000 + (index % 2) as i64 * 250_000;
    let key = if pair.is_multiple_of(3) { pair } else { index };
    let aggressor = match key % 5 {
        0 | 3 => AggressorSide::Buy,
        1 | 4 => AggressorSide::Sell,
        _ => AggressorSide::Unknown,
    };
    let bucket = timestamp_micros.div_euclid(60_000_000);
    FootprintTrade {
        timestamp_micros,
        price: 100.0 + ((key * 7) % 11) as f64 * 0.25,
        volume: 1.0 + ((index * 13) % 9) as f64 + if index.is_multiple_of(29) { 30.0 } else { 0.0 },
        aggressor,
        bid: None,
        ask: None,
        sequence: Some(index as u64),
        trade_id: Some(index as u64 + 1),
        conditions: 0,
        session_id: Some((bucket / 9) as u64),
    }
}

fn tape(range: core::ops::Range<usize>) -> Vec<FootprintTrade> {
    range.map(tape_trade).collect()
}

fn time_bars() -> FootprintAggregationOptions {
    FootprintAggregationOptions::default()
}

fn trade_bars() -> FootprintAggregationOptions {
    FootprintAggregationOptions {
        bars: FootprintBarAggregation::Trades { trades_per_bar: 3 },
        ..FootprintAggregationOptions::default()
    }
}

fn bubble_options() -> TradeBubbleOptions {
    TradeBubbleOptions {
        minimum_volume: 3.0,
        max_markers: 16,
        aggregation_window_micros: 500_000,
    }
}

/// One stream presented by every dependent kind.
struct Harness {
    chart: ChartEngine,
    stream: u64,
    footprint: SeriesId,
    candles: SeriesId,
    studies: [SeriesId; STUDIES],
}

/// Session, continuous and anchored CVD, delta, and volume.
const STUDIES: usize = 5;

impl Harness {
    fn new(aggregation: FootprintAggregationOptions, max_points: Option<usize>) -> Self {
        Self::with_bubbles(aggregation, max_points, bubble_options())
    }

    fn with_bubbles(
        aggregation: FootprintAggregationOptions,
        max_points: Option<usize>,
        bubbles: TradeBubbleOptions,
    ) -> Self {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        let stream = chart.add_trade_stream("TEST:TIP", aggregation).unwrap();
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
        let candles = chart.add_series(SeriesKind::Candlestick);
        chart
            .bind_trade_bar_series_to_stream(candles, stream)
            .unwrap();
        let session = chart
            .add_cvd_series(stream, 1, TradeStudyOptions::default())
            .unwrap();
        let continuous = chart
            .add_cvd_series(
                stream,
                1,
                TradeStudyOptions {
                    cumulative_delta_reset: CumulativeDeltaReset::Continuous,
                    anchor_timestamp_micros: None,
                },
            )
            .unwrap();
        let anchored = chart
            .add_cvd_series(
                stream,
                1,
                TradeStudyOptions {
                    cumulative_delta_reset: CumulativeDeltaReset::Anchored,
                    anchor_timestamp_micros: Some(tape_trade(40).timestamp_micros),
                },
            )
            .unwrap();
        let delta = chart.add_delta_series(stream, 2).unwrap();
        let volume = chart.add_trade_volume_series(stream, 2).unwrap();
        chart.add_trade_bubbles(stream, footprint, bubbles).unwrap();
        if let Some(max_points) = max_points {
            assert!(chart.set_series_max_points(footprint, Some(max_points)));
        }
        Self {
            chart,
            stream,
            footprint,
            candles,
            studies: [session, continuous, anchored, delta, volume],
        }
    }

    fn load(&mut self, trades: Vec<FootprintTrade>) {
        self.chart
            .set_trade_stream_trades(self.stream, trades)
            .unwrap();
    }

    fn tip(&mut self, trades: Vec<FootprintTrade>) {
        assert_eq!(
            self.chart
                .update_trade_stream_trades(self.stream, trades)
                .unwrap(),
            FootprintUpdateKind::Tip
        );
    }

    fn first_bar_open(&self) -> Option<i64> {
        self.chart
            .trade_stream(self.stream)
            .unwrap()
            .bars()
            .first()
            .map(|bar| bar.start_timestamp_micros)
    }

    fn last_bar_open(&self) -> Option<i64> {
        self.chart
            .trade_stream(self.stream)
            .unwrap()
            .bars()
            .last()
            .map(|bar| bar.start_timestamp_micros)
    }

    fn snapshot(&self) -> Snapshot {
        let chart = &self.chart;
        Snapshot {
            bars: chart.trade_stream(self.stream).unwrap().bars().to_vec(),
            footprint: rows(chart, self.footprint),
            candles: rows(chart, self.candles),
            studies: self.studies.map(|id| rows(chart, id)),
            markers: markers(chart, self.footprint),
            sequence: chart.sequence_points().map(<[BarSequencePoint]>::to_vec),
        }
    }

    /// Rebuild every dependent from the stream in place: the reference a live tip must equal.
    fn clean_rebuild_in_place(&mut self) -> Snapshot {
        self.chart.refresh_trade_dependents(self.stream).unwrap();
        self.chart
            .refresh_footprint_series_from_stream(self.stream, None)
            .unwrap();
        self.snapshot()
    }
}

/// Bit-exact data-layer rows plus the body color channel.
#[derive(Debug, PartialEq)]
struct Rows {
    times: Vec<i64>,
    values: Vec<[u64; 4]>,
    colors: Vec<Option<u32>>,
}

fn rows(chart: &ChartEngine, id: SeriesId) -> Rows {
    let (times, columns) = chart.data_layer().series_data(id).unwrap();
    let colors = chart.data_layer().point_colors(id).unwrap();
    Rows {
        times: times.to_vec(),
        values: (0..times.len())
            .map(|row| columns.map(|column| column[row].to_bits()))
            .collect(),
        colors: (0..times.len())
            .map(|row| colors.color(PointColorChannel::Body, row))
            .collect(),
    }
}

type MarkerRow = (i64, u8, u8, u32, String, String, u64, Option<u64>);

fn markers(chart: &ChartEngine, id: SeriesId) -> Vec<MarkerRow> {
    marker_rows(&chart.series_entry(id).unwrap().markers)
}

fn marker_rows(markers: &[Marker]) -> Vec<MarkerRow> {
    markers
        .iter()
        .map(|marker| {
            (
                marker.time,
                marker.position,
                marker.shape,
                marker.color.0,
                marker.text.clone(),
                marker.id.clone(),
                marker.size.to_bits(),
                marker.price.map(f64::to_bits),
            )
        })
        .collect()
}

#[derive(Debug, PartialEq)]
struct Snapshot {
    bars: Vec<FootprintBar>,
    footprint: Rows,
    candles: Rows,
    studies: [Rows; STUDIES],
    markers: Vec<MarkerRow>,
    sequence: Option<Vec<BarSequencePoint>>,
}

/// Field-by-field comparison so a failure names the dependent that diverged.
#[track_caller]
fn assert_same(actual: &Snapshot, expected: &Snapshot, context: &str) {
    assert_eq!(actual.bars, expected.bars, "bars: {context}");
    assert_eq!(
        actual.footprint, expected.footprint,
        "footprint rows: {context}"
    );
    assert_eq!(actual.candles, expected.candles, "candle rows: {context}");
    for (study, (actual, expected)) in actual.studies.iter().zip(&expected.studies).enumerate() {
        assert_eq!(actual, expected, "study {study} rows: {context}");
    }
    assert_eq!(
        actual.markers, expected.markers,
        "bubble markers: {context}"
    );
    assert_eq!(
        actual.sequence, expected.sequence,
        "sequence sidecar: {context}"
    );
}

#[test]
fn time_bar_live_tips_match_a_fresh_rebuild_for_every_dependent() {
    let mut live = Harness::new(time_bars(), None);
    live.load(tape(0..120));
    for index in 120..360 {
        live.tip(vec![tape_trade(index)]);
    }
    // A multi-trade tip batch that closes and opens bars in one step.
    live.tip(tape(360..380));
    let mut fresh = Harness::new(time_bars(), None);
    fresh.load(tape(0..380));
    assert!(!live.snapshot().markers.is_empty());
    assert_same(&live.snapshot(), &fresh.snapshot(), "final");
}

#[test]
fn non_time_live_tips_match_a_fresh_rebuild_for_every_dependent() {
    let mut live = Harness::new(trade_bars(), None);
    live.load(tape(0..120));
    for index in 120..360 {
        live.tip(vec![tape_trade(index)]);
    }
    live.tip(tape(360..380));
    let mut fresh = Harness::new(trade_bars(), None);
    fresh.load(tape(0..380));
    assert_same(&live.snapshot(), &fresh.snapshot(), "final");
}

/// Retention trims the stream from the front while tips stream in. Every tip must still equal an
/// in-place clean rebuild, and the retained rows must equal the unretained chart's tail: evicting
/// old bars never rewrites the cumulative-delta values of the bars that remain.
#[test]
fn time_bar_retention_tips_match_a_clean_rebuild_and_the_unretained_tail() {
    let mut live = Harness::new(time_bars(), Some(12));
    let mut unretained = Harness::new(time_bars(), None);
    live.load(tape(0..300));
    unretained.load(tape(0..300));
    let mut trims = 0;
    for index in 300..700 {
        let before = live.first_bar_open();
        live.tip(vec![tape_trade(index)]);
        unretained.tip(vec![tape_trade(index)]);
        let trimmed = live.first_bar_open() != before;
        trims += usize::from(trimmed);
        if index.is_multiple_of(23) || trimmed {
            let tipped = live.snapshot();
            assert_same(
                &tipped,
                &live.clean_rebuild_in_place(),
                &format!("tip {index}"),
            );
        }
        let retained = live.snapshot();
        let reference = unretained.snapshot();
        let tail = |rows: &Rows, len: usize| rows.values[rows.values.len() - len..].to_vec();
        let len = retained.footprint.values.len();
        assert_eq!(
            retained.footprint.times,
            reference.footprint.times[reference.footprint.times.len() - len..]
        );
        assert_eq!(retained.footprint.values, tail(&reference.footprint, len));
        assert_eq!(retained.candles.values, tail(&reference.candles, len));
        for (retained, reference) in retained.studies.iter().zip(&reference.studies) {
            assert_eq!(
                retained.times,
                reference.times[reference.times.len() - len..]
            );
            assert_eq!(retained.values, tail(reference, len), "tip {index}");
        }
    }
    assert!(
        trims >= 3,
        "the scenario must cross the retention ceiling repeatedly"
    );
    // A continuous CVD created after retention evicted history resumes from the same stream seed
    // as the one that watched the eviction.
    let late = live
        .chart
        .add_cvd_series(
            live.stream,
            1,
            TradeStudyOptions {
                cumulative_delta_reset: CumulativeDeltaReset::Continuous,
                anchor_timestamp_micros: None,
            },
        )
        .unwrap();
    assert_eq!(rows(&live.chart, late), rows(&live.chart, live.studies[1]));
}

#[test]
fn non_time_retention_tips_keep_rows_sidecar_studies_and_bubbles_aligned() {
    let mut live = Harness::new(trade_bars(), Some(40));
    live.load(tape(0..150));
    let mut trims = 0;
    for index in 150..500 {
        let before = live.first_bar_open();
        live.tip(vec![tape_trade(index)]);
        let trimmed = live.first_bar_open() != before;
        trims += usize::from(trimmed);
        let snapshot = live.snapshot();
        let keys = &snapshot.footprint.times;
        assert_eq!(snapshot.bars.len(), keys.len());
        assert_eq!(snapshot.sequence.as_ref().unwrap().len(), keys.len());
        assert_eq!(&snapshot.candles.times, keys, "tip {index}");
        for study in &snapshot.studies {
            assert_eq!(&study.times, keys, "tip {index}");
        }
        for marker in &snapshot.markers {
            assert!(keys.binary_search(&marker.0).is_ok(), "tip {index}");
        }
        if index.is_multiple_of(17) || trimmed {
            assert_same(
                &snapshot,
                &live.clean_rebuild_in_place(),
                &format!("tip {index}"),
            );
        }
    }
    assert!(trims >= 3);
}

/// Bubble merges, peak growth through a merge, eviction of the peak bubble, minimum-volume gaps and
/// equal-volume ties must all fold exactly like a rebuild from the full tape.
#[test]
fn bubble_fold_merges_evicts_and_rescales_like_a_fresh_rebuild() {
    let side = |index: usize| {
        if index % 4 < 2 {
            AggressorSide::Buy
        } else {
            AggressorSide::Sell
        }
    };
    let crafted = |index: usize| {
        let volume = match index % 11 {
            0 => 20.0,
            3 | 4 => 5.0,
            7 => 1.0,
            _ => 5.0 + (index % 3) as f64,
        };
        FootprintTrade {
            timestamp_micros: BASE_MICROS + index as i64 * 300_000,
            price: 100.0 + (index / 2 % 3) as f64 * 0.25,
            volume,
            aggressor: side(index),
            bid: None,
            ask: None,
            sequence: None,
            trade_id: None,
            conditions: 0,
            session_id: Some(1),
        }
    };
    let options = TradeBubbleOptions {
        minimum_volume: 2.0,
        max_markers: 3,
        aggregation_window_micros: 400_000,
    };
    let mut live = Harness::with_bubbles(time_bars(), None, options);
    live.load((0..4).map(crafted).collect());
    let mut peak_changes = 0;
    for index in 4..120 {
        let peak = live.bubble_peak();
        live.tip(vec![crafted(index)]);
        peak_changes += usize::from(live.bubble_peak() != peak);
        let mut fresh = Harness::with_bubbles(time_bars(), None, options);
        fresh.load((0..=index).map(crafted).collect());
        assert_eq!(
            live.snapshot().markers,
            fresh.snapshot().markers,
            "tip {index}"
        );
    }
    // The scenario exercises merges (fewer bubbles than qualifying prints) and peak moves.
    let qualifying = (0..120)
        .filter(|&index| crafted(index).volume >= options.minimum_volume)
        .count();
    let fold = live.chart.trade_bubbles[&live.stream][0]
        .fold
        .as_ref()
        .unwrap();
    assert!((fold.next_sequence as usize) < qualifying);
    assert!(peak_changes >= 2, "{peak_changes} peak changes");
}

#[test]
fn late_and_corrected_trades_between_tips_match_a_fresh_rebuild() {
    for aggregation in [time_bars(), trade_bars()] {
        let mut live = Harness::new(aggregation, None);
        live.load(tape(0..200));
        for index in 200..260 {
            live.tip(vec![tape_trade(index)]);
        }
        // A provider correction of an earlier print and a late print inside closed history.
        let mut corrected = tape_trade(150);
        corrected.volume += 17.0;
        let mut late = tape_trade(151);
        late.timestamp_micros += 1;
        late.trade_id = Some(1_000_000);
        late.sequence = None;
        assert_eq!(
            live.chart
                .update_trade_stream_trades(live.stream, vec![corrected.clone(), late.clone()])
                .unwrap(),
            FootprintUpdateKind::Historical
        );
        for index in 260..320 {
            live.tip(vec![tape_trade(index)]);
        }
        let mut final_tape = tape(0..320);
        final_tape[150] = corrected;
        final_tape.push(late);
        let mut fresh = Harness::new(aggregation, None);
        fresh.load(final_tape);
        assert_same(&live.snapshot(), &fresh.snapshot(), "final");
    }
}

#[test]
fn host_markers_on_a_bubble_series_are_replaced_by_the_next_tip() {
    let mut live = Harness::new(time_bars(), None);
    live.load(tape(0..100));
    let marker = live.chart.series_entry(live.footprint).unwrap().markers[0].clone();
    live.chart
        .set_series_markers(live.footprint, vec![marker.clone(), marker]);
    live.tip(vec![tape_trade(100)]);
    let mut fresh = Harness::new(time_bars(), None);
    fresh.load(tape(0..101));
    assert_eq!(live.snapshot().markers, fresh.snapshot().markers);
}

#[test]
fn replay_clock_tips_and_forward_seeks_match_a_fresh_rebuild() {
    let mut live = Harness::new(time_bars(), None);
    live.load(tape(0..200));
    let clock = tape_trade(150).timestamp_micros;
    live.chart.set_replay_clock_micros(Some(clock)).unwrap();
    // Future tips change only source truth; a forward seek then reveals them.
    for index in 200..240 {
        live.tip(vec![tape_trade(index)]);
    }
    live.chart
        .set_replay_clock_micros(Some(tape_trade(220).timestamp_micros))
        .unwrap();
    live.chart.set_replay_clock_micros(None).unwrap();
    for index in 240..260 {
        live.tip(vec![tape_trade(index)]);
    }
    let mut fresh = Harness::new(time_bars(), None);
    fresh.load(tape(0..260));
    assert_same(&live.snapshot(), &fresh.snapshot(), "final");
}

impl Harness {
    fn stats(&self) -> TradeStreamStats {
        self.chart.trade_stream_stats(self.stream).unwrap()
    }

    fn bar_opens(&self) -> Vec<i64> {
        self.chart
            .trade_stream(self.stream)
            .unwrap()
            .bars()
            .iter()
            .map(|bar| bar.start_timestamp_micros)
            .collect()
    }

    fn historical_rebuilds(&self) -> usize {
        self.chart
            .trade_stream(self.stream)
            .unwrap()
            .work_stats()
            .historical_rebuilds
    }

    /// Peak volume the bubble markers are currently sized against.
    fn bubble_peak(&self) -> u64 {
        self.chart.trade_bubbles[&self.stream][0]
            .fold
            .as_ref()
            .unwrap()
            .sized_peak
            .to_bits()
    }
}

/// Per-tip work of every dependent kind, from the stream's work counters.
struct TipWork {
    study_rows: u64,
    bar_rows: u64,
    bubble_trades: u64,
    bubble_sizes: u64,
    index_rebuilds: u64,
    /// Stream bars the tip changed: the previously active bar plus any bars it opened.
    changed_bars: u64,
    peak_changed: bool,
}

fn tip_work(harness: &mut Harness, trades: Vec<FootprintTrade>) -> TipWork {
    let before = harness.stats();
    let opens_before = harness.bar_opens();
    let peak_before = harness.bubble_peak();
    let rebuilds_before = harness.chart.data_layer().index_rebuilds();
    harness.tip(trades);
    let after = harness.stats();
    // The previously active bar plus every bar the tip opened, whether or not retention then
    // evicted older bars from the front.
    let last_before = opens_before.last().copied();
    let opened = harness
        .bar_opens()
        .iter()
        .filter(|&&open| last_before.is_none_or(|last| open > last))
        .count();
    TipWork {
        study_rows: after.dependent_rows_computed - before.dependent_rows_computed,
        bar_rows: after.bar_rows_projected - before.bar_rows_projected,
        bubble_trades: after.bubble_trades_scanned - before.bubble_trades_scanned,
        bubble_sizes: after.bubble_markers_sized - before.bubble_markers_sized,
        index_rebuilds: harness.chart.data_layer().index_rebuilds() - rebuilds_before,
        changed_bars: (opened + usize::from(last_before.is_some())) as u64,
        peak_changed: harness.bubble_peak() != peak_before,
    }
}

/// Architecture.md: a live tip updates only the active derived bar. Five studies, the footprint
/// and the bound candles each touch only the changed bars, bubbles fold only the new trade and
/// size only the bubble it created or grew (every retained marker only when the peak moves), and
/// a tip inside the active bar never reinstalls any series.
#[test]
fn live_tip_work_is_bounded_by_the_changed_suffix_and_new_trades() {
    for aggregation in [time_bars(), trade_bars()] {
        let mut live = Harness::new(aggregation, None);
        live.load(tape(0..3_000));
        let max_markers = bubble_options().max_markers as u64;
        let mut rescales = 0;
        for index in 3_000..3_300 {
            let work = tip_work(&mut live, vec![tape_trade(index)]);
            assert_eq!(
                work.study_rows,
                STUDIES as u64 * work.changed_bars,
                "tip {index}"
            );
            assert_eq!(work.bar_rows, 2 * work.changed_bars, "tip {index}");
            assert_eq!(work.bubble_trades, 1, "tip {index}");
            if work.peak_changed {
                rescales += 1;
                assert!(work.bubble_sizes <= max_markers, "tip {index}");
            } else {
                assert!(work.bubble_sizes <= 1, "tip {index}");
            }
            if work.changed_bars == 1 {
                assert_eq!(work.index_rebuilds, 0, "tip {index} reinstalled a series");
            }
        }
        assert!(rescales > 0, "the tape moves the peak bubble volume");
        assert!(rescales < 60, "rescaling stays the exception: {rescales}");
        let mut fresh = Harness::new(aggregation, None);
        fresh.load(tape(0..3_300));
        assert_same(&live.snapshot(), &fresh.snapshot(), "after bounded tips");
    }
}

/// Under a retention ceiling every tip stays suffix-bounded, including the tip that crosses the
/// ceiling: retention evicts the leading bars, their trades, and their bubbles in place, without
/// reconstructing the retained tape or refolding bubbles. Only the data layer's own hysteresis trim
/// of the affected rows remains, once per margin.
#[test]
fn retained_live_tip_work_is_bounded_including_retention_trims() {
    for aggregation in [time_bars(), trade_bars()] {
        let mut live = Harness::new(aggregation, Some(96));
        live.load(tape(0..1_500));
        let rebuilds = live.historical_rebuilds();
        let max_markers = bubble_options().max_markers as u64;
        let mut trims = 0;
        let mut opened = 0;
        for index in 1_500..2_700 {
            let before = live.first_bar_open();
            let active = live.last_bar_open();
            let work = tip_work(&mut live, vec![tape_trade(index)]);
            opened += usize::from(live.last_bar_open() != active);
            let trimmed = live.first_bar_open() != before;
            trims += usize::from(trimmed);
            assert_eq!(
                work.study_rows,
                STUDIES as u64 * work.changed_bars,
                "tip {index}"
            );
            assert_eq!(work.bar_rows, 2 * work.changed_bars, "tip {index}");
            assert_eq!(work.bubble_trades, 1, "tip {index}");
            let sizes = if work.peak_changed { max_markers } else { 1 };
            assert!(work.bubble_sizes <= sizes, "tip {index}");
            if work.changed_bars == 1 && !trimmed {
                assert_eq!(work.index_rebuilds, 0, "tip {index} reinstalled a series");
            }
        }
        assert_eq!(
            live.historical_rebuilds(),
            rebuilds,
            "retention reconstructed the retained tape"
        );
        // 96 / CAP_TRIM_MARGIN_DIVISOR = 3 bars of hysteresis between trims.
        assert!(trims >= 3, "the scenario crosses the ceiling: {trims}");
        assert!(
            trims <= opened / 3 + 1,
            "trims happen once per margin, not per bar: {trims} trims for {opened} new bars"
        );
        assert_same(
            &live.snapshot(),
            &live.clean_rebuild_in_place(),
            "after retained tips",
        );
    }
}

/// One retention is one data-layer transaction: every presentation of the stream (footprint,
/// bound candles, five studies) leaves the shared time axis in a single union rebuild plus a single
/// reindex, not one pair per presentation. The tips that are not trims never rebuild at all.
#[test]
fn retention_trim_runs_one_union_rebuild_for_every_presentation() {
    for aggregation in [time_bars(), trade_bars()] {
        let mut live = Harness::new(aggregation, Some(96));
        live.load(tape(0..1_500));
        let mut trims = 0;
        for index in 1_500..2_700 {
            let before = live.first_bar_open();
            let work = tip_work(&mut live, vec![tape_trade(index)]);
            if live.first_bar_open() != before {
                trims += 1;
                assert_eq!(
                    work.index_rebuilds, 2,
                    "{aggregation:?} trim tip {index}: one union rebuild and one reindex"
                );
            }
        }
        assert!(trims >= 3, "the scenario crosses the ceiling: {trims}");
        assert_same(
            &live.snapshot(),
            &live.clean_rebuild_in_place(),
            "after retained tips",
        );
    }
}

/// A non-time retention trim evicts bubbles in place, without refolding the retained tape. Eviction
/// addresses the bubbles against the first retained row key, so it must run after the data layer
/// has trimmed every presentation: before it, the key base still names an evicted row, the fold
/// refuses the in-place path and every trim tip rescans the whole retained tape.
#[test]
fn retention_trim_evicts_bubbles_in_place_on_the_trimmed_row_keys() {
    let mut live = Harness::new(trade_bars(), Some(96));
    live.load(tape(0..1_500));
    let mut trims = 0;
    for index in 1_500..2_700 {
        let before = live.first_bar_open();
        let work = tip_work(&mut live, vec![tape_trade(index)]);
        if live.first_bar_open() != before {
            trims += 1;
            assert_eq!(
                work.bubble_trades, 1,
                "trim tip {index}: bubbles refolded the retained tape"
            );
        }
    }
    assert!(trims >= 3, "the scenario crosses the ceiling: {trims}");
    let retained = live.snapshot();
    assert!(
        !retained.markers.is_empty(),
        "the tape leaves retained bubbles"
    );
    assert_same(
        &retained,
        &live.clean_rebuild_in_place(),
        "after retained tips",
    );
}

/// Every point's `(index, weight)` of the marks a clean rebuild gives the chart's current axis.
fn clean_tick_marks(
    chart: &ChartEngine,
) -> Vec<aeris_charts_core::scale::time_tick_marks::TickMark> {
    clean_tick_marks_shifted(chart, chart.tick_label_shift())
}

/// [`clean_tick_marks`] for time labels printed `label_shift` seconds after each bar's open.
fn clean_tick_marks_shifted(
    chart: &ChartEngine,
    label_shift: i64,
) -> Vec<aeris_charts_core::scale::time_tick_marks::TickMark> {
    use aeris_charts_core::scale::time_tick_marks::{
        fill_weights_for_points_shifted_in, TimeTickMarks,
    };
    let times = chart.sequence_points().map_or_else(
        || chart.data_layer().merged_times().to_vec(),
        |points| {
            points
                .iter()
                .map(|point| point.open_timestamp_micros.div_euclid(1_000_000))
                .collect()
        },
    );
    let mut weights = vec![0u8; times.len()];
    fill_weights_for_points_shifted_in(&times, &mut weights, 0, label_shift, &chart.exchange_time);
    let mut marks = TimeTickMarks::new();
    marks.set_weights(&weights);
    // A spacing wider than the label keeps every point, so the marks carry every weight.
    marks.build(1_000.0, 10.0).to_vec()
}

/// A retention trim re-weighs the axis: the marks after a trimming tip, on a time axis by dropping
/// the evicted points and re-weighing only the first, equal a clean rebuild of the retained axis.
#[test]
fn retention_trims_leave_the_axis_weights_of_a_clean_rebuild() {
    for aggregation in [time_bars(), trade_bars()] {
        let mut live = Harness::new(aggregation, Some(96));
        live.load(tape(0..1_500));
        let mut trims = 0;
        for index in 1_500..2_700 {
            let before = live.first_bar_open();
            live.tip(vec![tape_trade(index)]);
            if live.first_bar_open() == before {
                continue;
            }
            trims += 1;
            let marks = live.chart.tick_marks.build(1_000.0, 10.0).to_vec();
            assert_eq!(
                marks,
                clean_tick_marks(&live.chart),
                "{aggregation:?} trim tip {index}"
            );
        }
        assert!(trims >= 3, "the scenario crosses the ceiling: {trims}");
    }
}

/// A close-time label prints each time bar an interval after its open, and the hour and minute
/// marks follow the printed time. A retention trim re-weighs the first point and the appended tail
/// incrementally, so they must carry the same shift as a clean rebuild; a trade-count axis prints
/// its own open times and stays unshifted.
#[test]
fn retention_trims_keep_the_close_label_shift_in_the_axis_weights() {
    for aggregation in [time_bars(), trade_bars()] {
        let mut live = Harness::new(aggregation, Some(96));
        live.chart
            .set_bar_time_label(crate::BarTimeLabel::Close {
                interval_seconds: 60,
                windows: Vec::new(),
            })
            .unwrap();
        live.load(tape(0..1_500));
        let mut trims = 0;
        let mut shift_shows = 0;
        for index in 1_500..2_700 {
            let before = live.first_bar_open();
            live.tip(vec![tape_trade(index)]);
            if live.first_bar_open() == before {
                continue;
            }
            trims += 1;
            let marks = live.chart.tick_marks.build(1_000.0, 10.0).to_vec();
            assert_eq!(
                marks,
                clean_tick_marks(&live.chart),
                "{aggregation:?} trim tip {index}"
            );
            if marks != clean_tick_marks_shifted(&live.chart, 0) {
                shift_shows += 1;
            }
        }
        assert!(trims >= 3, "the scenario crosses the ceiling: {trims}");
        // On a time axis the shift moves the hour marks, so the comparison above is not vacuous;
        // a sequence axis ignores the label and the two rebuilds agree.
        assert_eq!(
            shift_shows > 0,
            live.chart.sequence_points().is_none(),
            "{aggregation:?}: the close label shifts the weights only on a time axis"
        );
    }
}

/// A refold over a long tape materializes only the retained markers, so the series never holds
/// capacity for every qualifying print it folded past.
#[test]
fn bubble_refold_holds_only_the_retained_markers() {
    let options = TradeBubbleOptions {
        minimum_volume: 0.0,
        max_markers: 16,
        aggregation_window_micros: 0,
    };
    let mut live = Harness::with_bubbles(time_bars(), None, options);
    live.load(tape(0..20_000));
    let markers = &live.chart.series_entry(live.footprint).unwrap().markers;
    assert_eq!(markers.len(), 16);
    assert!(markers.capacity() <= 2 * 16, "{}", markers.capacity());
}

/// Footprint geometry reads stream bar `i` for row `i`, so every footprint bound to a stream must
/// drop the rows retention evicts, not only the one whose ceiling triggered the trim.
#[test]
fn every_footprint_of_a_retained_stream_keeps_one_row_per_bar() {
    for aggregation in [time_bars(), trade_bars()] {
        let mut live = Harness::new(aggregation, Some(40));
        let second = live.chart.add_series(SeriesKind::Footprint);
        live.chart
            .configure_footprint_series(
                second,
                FootprintSeriesOptions {
                    aggregation,
                    ..FootprintSeriesOptions::default()
                },
            )
            .unwrap();
        live.chart
            .bind_footprint_series_to_stream(second, live.stream)
            .unwrap();
        live.load(tape(0..300));
        let mut trims = 0;
        for index in 300..1_000 {
            let before = live.first_bar_open();
            live.tip(vec![tape_trade(index)]);
            trims += usize::from(live.first_bar_open() != before);
            assert_eq!(
                rows(&live.chart, second),
                rows(&live.chart, live.footprint),
                "{aggregation:?} tip {index}"
            );
            assert_eq!(
                rows(&live.chart, second).times.len(),
                live.bar_opens().len()
            );
        }
        assert!(trims >= 3);
    }
}

/// Trade, volume, and range bars may split same-microsecond trades across a bar boundary.
/// Retention must evict exactly the trades the evicted bars aggregated, so the retained bars and
/// studies equal the unretained chart's tail and the footprint keeps one row per bar.
#[test]
fn non_time_retention_evicts_exactly_the_trades_of_the_evicted_bars() {
    let paired = |index: usize| FootprintTrade {
        timestamp_micros: BASE_MICROS + (index / 2) as i64 * 1_000_000,
        ..tape_trade(index)
    };
    let mut live = Harness::new(trade_bars(), Some(40));
    let mut unretained = Harness::new(trade_bars(), None);
    live.load((0..150).map(paired).collect());
    unretained.load((0..150).map(paired).collect());
    let mut trims = 0;
    for index in 150..500 {
        let before = live.first_bar_open();
        live.tip(vec![paired(index)]);
        unretained.tip(vec![paired(index)]);
        trims += usize::from(live.first_bar_open() != before);
        let retained = live.snapshot();
        let reference = unretained.snapshot();
        let len = retained.bars.len();
        assert_eq!(retained.footprint.times.len(), len, "tip {index}");
        let renumbered = reference.bars[reference.bars.len() - len..]
            .iter()
            .enumerate()
            .map(|(index, bar)| FootprintBar {
                logical_index: index as u64,
                ..bar.clone()
            })
            .collect::<Vec<_>>();
        assert_eq!(retained.bars, renumbered, "tip {index}");
        let tail = |rows: &Rows| rows.values[rows.values.len() - len..].to_vec();
        assert_eq!(retained.footprint.values, tail(&reference.footprint));
        assert_eq!(retained.candles.values, tail(&reference.candles));
        for (study, (retained, reference)) in
            retained.studies.iter().zip(&reference.studies).enumerate()
        {
            assert_eq!(
                retained.values,
                tail(reference),
                "study {study} tip {index}"
            );
        }
    }
    assert!(trims >= 3);
}

/// On a non-time axis, a candle, study, or footprint bound after retention trims continues the
/// stream's retained row keys instead of restarting at zero.
#[test]
fn non_time_presentations_bound_after_trims_continue_the_stream_keys() {
    let mut live = Harness::new(trade_bars(), Some(40));
    live.load(tape(0..150));
    for index in 150..500 {
        live.tip(vec![tape_trade(index)]);
    }
    let keys = rows(&live.chart, live.footprint).times;
    assert!(keys[0] > 0, "retention trimmed the axis");
    let bars = live.chart.add_series(SeriesKind::Bar);
    live.chart
        .bind_trade_bar_series_to_stream(bars, live.stream)
        .unwrap();
    let cvd = live
        .chart
        .add_cvd_series(live.stream, 1, TradeStudyOptions::default())
        .unwrap();
    let second = live.chart.add_series(SeriesKind::Footprint);
    live.chart
        .configure_footprint_series(
            second,
            FootprintSeriesOptions {
                aggregation: trade_bars(),
                ..FootprintSeriesOptions::default()
            },
        )
        .unwrap();
    live.chart
        .bind_footprint_series_to_stream(second, live.stream)
        .unwrap();
    for id in [bars, cvd, second] {
        assert_eq!(rows(&live.chart, id).times, keys, "late series {id}");
    }
    assert_eq!(live.chart.data_layer().merged_times(), keys.as_slice());
    for index in 500..560 {
        live.tip(vec![tape_trade(index)]);
        let keys = rows(&live.chart, live.footprint).times;
        for id in [bars, cvd, second, live.candles] {
            assert_eq!(rows(&live.chart, id).times, keys, "series {id} tip {index}");
        }
        assert_eq!(live.chart.data_layer().merged_times(), keys.as_slice());
    }
}

fn stream_state(stream: &FootprintAggregator) -> impl PartialEq + core::fmt::Debug {
    let ids = stream
        .trade_ids
        .iter()
        .map(|(&id, &position)| (id, position - stream.trade_id_base))
        .collect::<std::collections::BTreeMap<_, _>>();
    (
        stream.bars().to_vec(),
        stream
            .trades
            .iter()
            .map(|stored| (stored.event.clone(), stored.classified_side))
            .collect::<Vec<_>>(),
        ids,
        (
            stream.last_trade_price.map(f64::to_bits),
            stream.last_classified_side,
            stream.active_session,
            stream.session_delta.to_bits(),
        ),
    )
}

/// Retention evicts complete bars and exactly their trades without reconstructing the tape. The
/// result equals a reconstruction of the retained tape from its rebuild seed (the former
/// behavior): bars numbered from zero, classifications, running state, and trade-id lookups, and
/// every later backward or forward replay seek, provider correction, and tip.
#[test]
fn retention_without_reconstruction_equals_a_reconstruction_of_the_retained_tape() {
    // Same-microsecond pairs split across non-time bar boundaries; unknown sides use the tick rule.
    let trade = |index: usize| {
        let mut trade = tape_trade(index);
        trade.timestamp_micros = BASE_MICROS + (index / 2) as i64 * 400_000;
        if index % 7 == 3 {
            trade.aggressor = AggressorSide::Unknown;
        }
        trade
    };
    let aggregations = [
        time_bars(),
        trade_bars(),
        FootprintAggregationOptions {
            bars: FootprintBarAggregation::Volume {
                volume_per_bar: 40.0,
            },
            ..FootprintAggregationOptions::default()
        },
        FootprintAggregationOptions {
            bars: FootprintBarAggregation::Range { range_ticks: 4 },
            ..FootprintAggregationOptions::default()
        },
    ];
    for aggregation in aggregations {
        for clock in [None, Some(trade(2_900).timestamp_micros)] {
            let context = format!("{aggregation:?} clock {clock:?}");
            let mut retained = FootprintAggregator::new(aggregation).unwrap();
            retained
                .set_trades((0..3_000).map(trade).collect())
                .unwrap();
            retained.set_replay_clock_micros(clock).unwrap();
            for index in 3_000..3_100 {
                retained.update_trades(vec![trade(index)]).unwrap();
            }
            let keep = retained.bars().len() * 2 / 3;
            let mut reconstructed = retained.clone();
            let rebuilds = retained.work_stats().historical_rebuilds;
            let evicted = retained.retain_last_bars(keep).unwrap();
            assert!(evicted > 0, "{context}");
            assert_eq!(retained.work_stats().historical_rebuilds, rebuilds);
            assert_eq!(reconstructed.retain_last_bars(keep), Some(evicted));
            reconstructed.rebuild();
            assert_eq!(retained.bars().len(), keep, "{context}");
            assert_eq!(
                stream_state(&retained),
                stream_state(&reconstructed),
                "{context}"
            );
            let first = retained.bars()[0].start_timestamp_micros;
            let last = retained.bars().last().unwrap().end_timestamp_micros;
            for seek in [
                Some(first),
                Some(first + (last - first) / 3),
                Some(last - 1),
                clock,
                None,
            ] {
                retained.set_replay_clock_micros(seek).unwrap();
                reconstructed.set_replay_clock_micros(seek).unwrap();
                assert_eq!(
                    stream_state(&retained),
                    stream_state(&reconstructed),
                    "{context} seek {seek:?}"
                );
            }
            let mut corrected = trade(2_990);
            corrected.volume += 5.0;
            for stream in [&mut retained, &mut reconstructed] {
                assert_eq!(
                    stream.update_trades(vec![corrected.clone()]).unwrap(),
                    FootprintUpdateKind::Historical
                );
                stream.update_trades(vec![trade(3_100)]).unwrap();
            }
            assert_eq!(
                stream_state(&retained),
                stream_state(&reconstructed),
                "{context} after correction"
            );
        }
    }
}

/// Evicting bubbles in place leaves exactly the markers a refold of the retained tape builds, on
/// both axes. A merged bubble straddling the eviction boundary (possible only for sub-second time
/// buckets, which chart projections reject) refuses and the caller refolds.
#[test]
fn bubble_eviction_in_place_equals_a_refold_of_the_retained_tape() {
    let crafted = |index: usize| FootprintTrade {
        timestamp_micros: BASE_MICROS + index as i64 * 150_000,
        price: 100.0 + (index / 3 % 2) as f64 * 0.25,
        volume: 1.0 + (index * 7 % 5) as f64,
        aggressor: if (index / 3).is_multiple_of(3) {
            AggressorSide::Sell
        } else {
            AggressorSide::Buy
        },
        bid: None,
        ask: None,
        sequence: None,
        trade_id: Some(index as u64),
        conditions: 0,
        session_id: Some(1),
    };
    let half_second = FootprintAggregationOptions {
        bars: FootprintBarAggregation::Time {
            interval_micros: 500_000,
            anchor_micros: 0,
        },
        ..FootprintAggregationOptions::default()
    };
    let (mut in_place, mut refused) = (0, 0);
    // A six-marker window has usually dropped the bubbles near an early boundary; a window holding
    // the whole tape keeps every straddling bubble in play.
    let cases = [
        (half_second, false),
        (time_bars(), false),
        (trade_bars(), true),
    ]
    .into_iter()
    .flat_map(|case| [6, 64].map(|max_markers| (case, max_markers)));
    for ((aggregation, sequence_axis), max_markers) in cases {
        let options = TradeBubbleOptions {
            minimum_volume: 2.0,
            max_markers,
            aggregation_window_micros: 400_000,
        };
        let mut stream = FootprintAggregator::new(aggregation).unwrap();
        stream.set_trades((0..120).map(crafted).collect()).unwrap();
        for keep in 1..stream.bars().len() {
            let mut stream = stream.clone();
            let key_base = sequence_axis.then_some(7);
            let mut fold = BubbleFold::new(key_base);
            let mut markers = Vec::new();
            fold.advance(&mut markers, &stream, options);
            let bars = stream.bars().len() - keep;
            let trades = stream.retain_last_bars(keep).unwrap();
            let key_base = key_base.map(|base| base + bars as i64);
            let mut refold = BubbleFold::new(key_base);
            let mut expected = Vec::new();
            refold.advance(&mut expected, &stream, options);
            let before = marker_rows(&markers);
            if fold.evict_front(&mut markers, trades, bars, key_base) {
                fold.advance(&mut markers, &stream, options);
                assert_eq!(
                    marker_rows(&markers),
                    marker_rows(&expected),
                    "{aggregation:?} {max_markers} markers keep {keep}"
                );
                in_place += 1;
            } else {
                assert_eq!(marker_rows(&markers), before, "a refusal changes nothing");
                assert!(!sequence_axis && aggregation == half_second, "keep {keep}");
                refused += 1;
            }
        }
    }
    assert!(
        in_place > 20 && refused > 0,
        "{in_place} in place, {refused} refused"
    );
}

/// Replay under a retention ceiling: tips behind and across the clock, forward seeks that reveal
/// enough bars to trim inside the full reinstall, and a release back to live must leave every
/// dependent equal to an in-place clean rebuild, with bars equal to the unretained chart's tail.
#[test]
fn replay_seeks_and_tips_under_retention_match_a_clean_rebuild() {
    for aggregation in [time_bars(), trade_bars()] {
        let mut live = Harness::new(aggregation, Some(24));
        let mut unretained = Harness::new(aggregation, None);
        live.load(tape(0..200));
        unretained.load(tape(0..200));
        for harness in [&mut live, &mut unretained] {
            harness
                .chart
                .set_replay_clock_micros(Some(tape_trade(150).timestamp_micros))
                .unwrap();
        }
        let mut index = 200;
        for clock in [260, 420, 700] {
            while index < clock + 40 {
                let batch = tape(index..index + 5);
                live.tip(batch.clone());
                unretained.tip(batch);
                index += 5;
            }
            for harness in [&mut live, &mut unretained] {
                harness
                    .chart
                    .set_replay_clock_micros(Some(tape_trade(clock).timestamp_micros))
                    .unwrap();
            }
            let context = format!("{aggregation:?} clock {clock}");
            assert_same(&live.snapshot(), &live.clean_rebuild_in_place(), &context);
        }
        for harness in [&mut live, &mut unretained] {
            harness.chart.set_replay_clock_micros(None).unwrap();
        }
        for index in index..index + 60 {
            live.tip(vec![tape_trade(index)]);
            unretained.tip(vec![tape_trade(index)]);
        }
        assert_same(
            &live.snapshot(),
            &live.clean_rebuild_in_place(),
            &format!("{aggregation:?} live"),
        );
        let retained = live.snapshot();
        let reference = unretained.snapshot();
        let len = retained.bars.len();
        let tail = &reference.bars[reference.bars.len() - len..];
        for (retained, reference) in retained.bars.iter().zip(tail) {
            assert_eq!(
                FootprintBar {
                    logical_index: 0,
                    ..retained.clone()
                },
                FootprintBar {
                    logical_index: 0,
                    ..reference.clone()
                },
                "{aggregation:?}"
            );
        }
    }
}

/// A retention trim counts the rows a series exposes up to the data layer's replay cutoff, yet drops
/// the same leading bars from every presentation and keeps the rows past the cutoff in all of
/// them, in one data-layer transaction: what remains is the tail of the unretained chart.
#[test]
fn retention_trim_keeps_the_rows_past_the_cutoff_in_every_presentation() {
    const KEEP: usize = 30;
    const HIDDEN: usize = 10;
    for aggregation in [time_bars(), trade_bars()] {
        let mut live = Harness::new(aggregation, None);
        let mut unretained = Harness::new(aggregation, None);
        live.load(tape(0..1_200));
        unretained.load(tape(0..1_200));
        let reference = unretained.snapshot();
        let bars = reference.bars.len();
        assert!(bars > KEEP + HIDDEN);
        // Hide the newest rows of every presentation, as a replay clock behind the last bars does.
        let cutoff = reference.footprint.times[bars - HIDDEN - 1];
        live.chart.data.set_time_cutoff(Some(cutoff));
        let rebuilds = live.chart.data_layer().index_rebuilds();
        assert!(live.chart.set_series_max_points(live.footprint, Some(KEEP)));
        assert_eq!(
            live.chart.data_layer().index_rebuilds() - rebuilds,
            2,
            "{aggregation:?}: one union rebuild and one reindex"
        );
        let presentations = [live.footprint, live.candles]
            .into_iter()
            .chain(live.studies);
        for id in presentations {
            let data = live.chart.data_layer();
            assert_eq!(data.series_rows(id), Some(KEEP), "{aggregation:?} {id}");
            let exposed = data.series_data(id).unwrap().0.len();
            assert_eq!(
                exposed,
                KEEP - HIDDEN,
                "{aggregation:?} {id}: rows past the cutoff"
            );
        }
        live.chart.data.set_time_cutoff(None);
        let retained = live.snapshot();
        let tail = |rows: &Rows| Rows {
            times: rows.times[bars - KEEP..].to_vec(),
            values: rows.values[bars - KEEP..].to_vec(),
            colors: rows.colors[bars - KEEP..].to_vec(),
        };
        assert_eq!(
            retained.footprint,
            tail(&reference.footprint),
            "{aggregation:?}"
        );
        assert_eq!(
            retained.candles,
            tail(&reference.candles),
            "{aggregation:?}"
        );
        for (study, (retained, reference)) in
            retained.studies.iter().zip(&reference.studies).enumerate()
        {
            assert_eq!(*retained, tail(reference), "{aggregation:?} study {study}");
        }
    }
}

/// The data-layer trims a retention ran before its presentations were batched, as the reference a
/// batched trim must equal: the footprint down to `keep` rows on its own, then every other
/// presentation of the stream by the first key the footprint then exposes (all its exposed rows
/// when it exposes none), one `trim_front` and so one union rebuild and reindex per series.
fn sequential_retention_reference(harness: &mut Harness, keep: usize) {
    let chart = &mut harness.chart;
    chart.data.trim_front(harness.footprint, keep);
    let first_key = chart
        .data
        .series_data(harness.footprint)
        .and_then(|(times, _)| times.first().copied());
    let presentations = chart
        .stream_presentations(harness.stream)
        .filter(|&id| id != harness.footprint)
        .collect::<Vec<_>>();
    for id in presentations {
        let (times, _) = chart.data.series_data(id).unwrap();
        let evicted =
            first_key.map_or(times.len(), |key| times.partition_point(|&time| time < key));
        if evicted > 0 {
            let rows = chart.data.series_rows(id).unwrap();
            chart.data.trim_front(id, rows - evicted);
        }
    }
}

/// Every canonical row of every presentation (rows past the cutoff included) plus the shared axis.
fn canonical_rows(harness: &mut Harness) -> (Vec<Rows>, Vec<i64>) {
    harness.chart.data.set_time_cutoff(None);
    let ids = [harness.footprint, harness.candles]
        .into_iter()
        .chain(harness.studies);
    let presentations = ids.map(|id| rows(&harness.chart, id)).collect();
    (
        presentations,
        harness.chart.data_layer().merged_times().to_vec(),
    )
}

/// The retention trim of a stream whose presentations hold rows past the replay cutoff equals the
/// sequence of per-presentation trims it replaced, in every presentation's rows and in the shared
/// axis, while running one union rebuild and one reindex instead of one pair per presentation. The
/// cutoff is set on the data layer itself: a seek projects only the revealed bars into a stream's
/// presentations (the replay tests above cover those), so this is how rows past the clock reach
/// the trim. The first case leaves the footprint rows it exposes, the second none of them: its
/// `keep` retained rows are all past the cutoff, so every other presentation drops every row it
/// exposes and keeps the rows past the cutoff.
#[test]
fn retention_trim_under_a_cutoff_equals_the_sequential_trims() {
    const KEEP: usize = 30;
    for aggregation in [time_bars(), trade_bars()] {
        let mut probe = Harness::new(aggregation, None);
        probe.load(tape(0..1_200));
        let reference = probe.snapshot();
        let bars = reference.bars.len();
        for (exposed, exposed_after) in [(bars - 10, KEEP - 10), (KEEP + 5, 0)] {
            assert!(
                exposed > KEEP && exposed < bars,
                "{aggregation:?}: {bars} bars"
            );
            let cutoff = reference.footprint.times[exposed - 1];
            let context = format!("{aggregation:?} exposing {exposed} of {bars} bars");
            let mut batched = Harness::new(aggregation, None);
            let mut sequential = Harness::new(aggregation, None);
            for harness in [&mut batched, &mut sequential] {
                harness.load(tape(0..1_200));
                harness.chart.data.set_time_cutoff(Some(cutoff));
            }
            let rebuilds = |harness: &Harness| harness.chart.data_layer().index_rebuilds();
            let (before_batched, before_sequential) = (rebuilds(&batched), rebuilds(&sequential));
            assert!(batched
                .chart
                .set_series_max_points(batched.footprint, Some(KEEP)));
            sequential_retention_reference(&mut sequential, KEEP);
            assert_eq!(
                rebuilds(&batched) - before_batched,
                2,
                "{context}: one union rebuild and one reindex"
            );
            assert_eq!(
                rebuilds(&sequential) - before_sequential,
                2 * (2 + STUDIES) as u64,
                "{context}: the reference trims each presentation on its own"
            );
            let footprint_exposed = |harness: &Harness| {
                harness
                    .chart
                    .data_layer()
                    .series_data(harness.footprint)
                    .unwrap()
                    .0
                    .len()
            };
            assert_eq!(footprint_exposed(&batched), exposed_after, "{context}");
            assert_eq!(footprint_exposed(&sequential), exposed_after, "{context}");
            let (batched_rows, batched_axis) = canonical_rows(&mut batched);
            let (sequential_rows, sequential_axis) = canonical_rows(&mut sequential);
            let lengths =
                |rows: &[Rows]| rows.iter().map(|rows| rows.times.len()).collect::<Vec<_>>();
            assert_eq!(
                lengths(&batched_rows),
                lengths(&sequential_rows),
                "{context}: rows per presentation"
            );
            assert!(batched_rows == sequential_rows, "{context}: row contents");
            assert!(batched_axis == sequential_axis, "{context}: shared axis");
        }
    }
}

/// A host retention cap on a delta histogram keeps the sign palette of every retained row, on the
/// full install and across live tips, and tips still equal a clean rebuild.
#[test]
fn capped_delta_histogram_keeps_its_sign_palette() {
    let mut live = Harness::new(time_bars(), None);
    let delta = live.studies[3];
    assert!(live.chart.set_series_max_points(delta, Some(10)));
    live.load(tape(0..400));
    let installed = rows(&live.chart, delta);
    assert_eq!(installed.times.len(), 10);
    assert!(installed.colors.iter().all(Option::is_some), "full install");
    for index in 400..500 {
        live.tip(vec![tape_trade(index)]);
        let tipped = rows(&live.chart, delta);
        assert!(tipped.colors.iter().all(Option::is_some), "tip {index}");
    }
    let tipped = live.snapshot();
    assert_same(&tipped, &live.clean_rebuild_in_place(), "capped delta");
}

/// Trade-bound candles and the CVD, delta, and volume studies belong to their stream: every host
/// write path is refused with its documented refusal value, and nothing they hold, nor the
/// sequence keys every other presentation continues from, moves. A live tip afterwards still
/// equals a clean rebuild.
#[test]
fn trade_derived_series_reject_every_host_write() {
    use crate::{SeriesBarPatch, SeriesUpdateOutcome, SeriesUpdateRejection};
    use aeris_charts_core::model::data_validation::ValidationError;

    let unsupported = SeriesUpdateOutcome::Rejected(SeriesUpdateRejection::UnsupportedSeries);
    for aggregation in [time_bars(), trade_bars()] {
        let context = format!("{aggregation:?}");
        let mut live = Harness::new(aggregation, None);
        live.load(tape(0..120));
        let ordinary = live.chart.add_series(SeriesKind::Candlestick);
        assert!(!live.chart.series_is_source_owned(ordinary), "{context}");
        assert!(
            live.chart.series_is_source_owned(live.footprint),
            "{context}"
        );
        let before = live.snapshot();
        let key_base = live.chart.sequence_key_base(live.stream);
        let stream_bars = live.bar_opens();
        assert_eq!(key_base.is_some(), aggregation != time_bars(), "{context}");
        let time = 1_800_000_000.0;
        let seconds = 1_800_000_000;
        let ohlc = [1.0, 2.0, 0.5, 1.5];
        let patch = SeriesBarPatch {
            open: None,
            high: Some(1_000.0),
            low: None,
            close: Some(999.0),
            colors: [None; 3],
        };
        let mut owned = vec![live.candles];
        owned.extend(live.studies);
        for id in owned {
            let context = format!("{context} series {id}");
            assert!(live.chart.series_is_source_owned(id), "{context}");
            let held = rows(&live.chart, id).times.len();
            let chart = &mut live.chart;
            assert_eq!(
                chart.set_series_data(id, &[time], &[1.0], &[2.0], &[0.5], &[1.5]),
                Err(ValidationError::UnsupportedSeriesData(id)),
                "{context}"
            );
            assert_eq!(
                chart.set_series_data_styled(
                    id,
                    &[time],
                    &[1.0],
                    &[2.0],
                    &[0.5],
                    &[1.5],
                    [Some(vec![1]), None, None]
                ),
                Err(ValidationError::UnsupportedSeriesData(id)),
                "{context}"
            );
            assert!(
                !chart.install_series_data(
                    id,
                    vec![seconds],
                    vec![1.0],
                    vec![2.0],
                    vec![0.5],
                    vec![1.5]
                ),
                "{context}"
            );
            assert!(!chart.update_series_bar(id, time, ohlc), "{context}");
            assert!(
                !chart.update_series_bar_styled(id, time, ohlc, [Some(1), None, None]),
                "{context}"
            );
            assert_eq!(chart.update_series_bars(id, [(time, ohlc)]), 0, "{context}");
            assert_eq!(
                chart.update_series_bars_sanitized(
                    id,
                    vec![seconds],
                    vec![1.0],
                    vec![2.0],
                    vec![0.5],
                    vec![1.5]
                ),
                0,
                "{context}"
            );
            assert!(
                !chart.set_series_point_colors(id, Some(vec![7; held]), None, None),
                "{context}"
            );
            assert_eq!(chart.series_pop(id, 1), None, "{context}");
            assert_eq!(
                chart.merge_series_bar(id, time, patch, None),
                unsupported,
                "{context}"
            );
            assert_eq!(
                chart.merge_series_bars(id, &[(time, patch)], Some(7)),
                unsupported,
                "{context}"
            );
            assert_eq!(
                chart.update_series_bar_sequenced(id, time, ohlc, [None; 3], Some(7)),
                unsupported,
                "{context}"
            );
            assert_eq!(
                chart.update_series_bars_sanitized_sequenced(
                    id,
                    vec![seconds],
                    vec![1.0],
                    vec![2.0],
                    vec![0.5],
                    vec![1.5],
                    Some(7)
                ),
                unsupported,
                "{context}"
            );
            assert_eq!(chart.series_update_sequence(id), None, "{context}");
        }
        assert!(
            !live.chart.apply_momentum_histogram_colors(live.studies[3]),
            "{context}"
        );
        assert_same(&live.snapshot(), &before, &context);
        assert_eq!(live.chart.sequence_key_base(live.stream), key_base);
        assert_eq!(live.bar_opens(), stream_bars, "{context}");

        // The stream still feeds every presentation after the refused writes.
        live.tip(tape(120..140));
        let tipped = live.snapshot();
        assert_same(&tipped, &live.clean_rebuild_in_place(), &context);
        let mut fresh = Harness::new(aggregation, None);
        fresh.load(tape(0..140));
        assert_same(&tipped, &fresh.snapshot(), &context);
    }
}

/// A trade-bound candle converted to another chart type stays fed by its stream on both axes: the
/// tip still equals a clean rebuild and no sequence-axis writer assumes it is still a candle.
#[test]
fn converted_trade_bound_candles_keep_following_their_stream() {
    for aggregation in [time_bars(), trade_bars()] {
        for kind in [SeriesKind::Line, SeriesKind::Area, SeriesKind::Histogram] {
            let context = format!("{aggregation:?} {kind:?}");
            let mut live = Harness::new(aggregation, None);
            live.load(tape(0..60));
            live.chart.convert_series_kind(live.candles, kind);
            assert_eq!(live.chart.series_kind(live.candles), Some(kind));
            assert!(live.chart.series_is_source_owned(live.candles), "{context}");
            for index in 60..100 {
                live.tip(vec![tape_trade(index)]);
            }
            live.tip(tape(100..140));
            let tipped = live.snapshot();
            assert_same(&tipped, &live.clean_rebuild_in_place(), &context);
        }
    }
}

/// Tips into an empty stream, and retention ceilings of one bar (every new bar evicts one) and zero
/// bars (every trim clears the stream), stay equal to an in-place clean rebuild on both axes.
#[test]
fn tips_from_empty_and_degenerate_ceilings_match_a_clean_rebuild() {
    for aggregation in [time_bars(), trade_bars()] {
        for max_points in [None, Some(1), Some(0)] {
            let context = format!("{aggregation:?} {max_points:?}");
            let mut live = Harness::new(aggregation, max_points);
            live.load(Vec::new());
            assert!(live.snapshot().markers.is_empty(), "{context}");
            for index in 0..120 {
                live.tip(vec![tape_trade(index)]);
                if index % 13 == 0 {
                    let tipped = live.snapshot();
                    assert_same(&tipped, &live.clean_rebuild_in_place(), &context);
                }
            }
            let bars = live.bar_opens().len();
            match max_points {
                Some(max_points) => assert!(bars <= max_points, "{context}"),
                None => assert!(bars > 1, "{context}"),
            }
            let tipped = live.snapshot();
            assert_same(&tipped, &live.clean_rebuild_in_place(), &context);
        }
    }
}

// Session-anchored time bars (`set_trade_stream_sessions`) through the bounded tip paths.

const SESSION_BAR_SECONDS: i64 = 300;

fn shanghai() -> crate::UtcOffsetSchedule {
    crate::UtcOffsetSchedule::fixed(8 * 3_600).unwrap()
}

fn a_share_sessions(outside: OutOfSessionPolicy) -> TradeSessionOptions {
    use aeris_charts_core::scale::session_slots::parse_wall_clock;
    let window = |start: &str, end: &str| SessionWindow {
        start_seconds: parse_wall_clock(start, false).unwrap(),
        end_seconds: parse_wall_clock(end, true).unwrap(),
    };
    TradeSessionOptions {
        windows: vec![window("09:30", "11:30"), window("13:00", "15:00")],
        outside,
    }
}

/// Shanghai prints over consecutive days: pre-open and opening-auction prints, continuous trading
/// in both A-share windows, the 11:30:00 print, a lunch print, the 15:00:00 closing print, and an
/// after-hours print. Prints come in pairs 400 ms apart; every third pair repeats side and price so
/// bubble windows merge it. Out-of-window prints are large enough to qualify as bubbles, every
/// fifth print has no provider id, and each day is its own session.
fn session_tape(days: usize) -> Vec<FootprintTrade> {
    const HOUR: i64 = 3_600;
    let first_day = aeris_charts_core::scale::session_slots::parse_iso_date("2023-11-15").unwrap();
    let mut tape = Vec::new();
    for day in 0..days {
        let midnight = (first_day + day as i64) * 86_400 - 8 * HOUR;
        let mut slots = vec![(9 * HOUR + 15 * 60, true), (9 * HOUR + 25 * 60, true)];
        slots.extend(
            (9 * HOUR + 30 * 60..11 * HOUR + 30 * 60)
                .step_by(37)
                .map(|s| (s, false)),
        );
        slots.extend([(11 * HOUR + 30 * 60, true), (12 * HOUR + 10 * 60, true)]);
        slots.extend((13 * HOUR..15 * HOUR).step_by(37).map(|s| (s, false)));
        slots.extend([(15 * HOUR, true), (15 * HOUR + 30 * 60, true)]);
        for (slot, outside) in slots {
            for half in 0..2 {
                let index = tape.len();
                let pair = index / 2;
                let key = if pair.is_multiple_of(3) { pair } else { index };
                tape.push(FootprintTrade {
                    timestamp_micros: (midnight + slot) * MICROS_PER_SECOND + half * 400_000,
                    price: 100.0 + ((key * 7) % 11) as f64 * 0.25,
                    volume: 1.0
                        + ((index * 13) % 9) as f64
                        + if outside || index.is_multiple_of(29) {
                            40.0
                        } else {
                            0.0
                        },
                    aggressor: match key % 5 {
                        0 | 3 => AggressorSide::Buy,
                        1 | 4 => AggressorSide::Sell,
                        _ => AggressorSide::Unknown,
                    },
                    bid: None,
                    ask: None,
                    sequence: Some(index as u64),
                    trade_id: (index % 5 != 2).then_some(index as u64 + 1),
                    conditions: 0,
                    session_id: Some(day as u64),
                });
            }
        }
    }
    tape
}

fn session_bars() -> FootprintAggregationOptions {
    FootprintAggregationOptions {
        bars: FootprintBarAggregation::Time {
            interval_micros: (SESSION_BAR_SECONDS * MICROS_PER_SECOND) as u64,
            anchor_micros: 0,
        },
        ..FootprintAggregationOptions::default()
    }
}

fn session_harness(outside: OutOfSessionPolicy, max_points: Option<usize>) -> Harness {
    let mut harness = Harness::with_bubbles(session_bars(), max_points, bubble_options());
    harness.chart.set_time_zone(shanghai());
    harness
        .chart
        .set_trade_stream_sessions(harness.stream, Some(a_share_sessions(outside)))
        .unwrap();
    harness
}

type BubbleRow = (i64, String, u64, Option<u64>);

fn bubble_rows(markers: &[Marker]) -> Vec<BubbleRow> {
    markers
        .iter()
        .map(|marker| {
            (
                marker.time,
                marker.id.clone(),
                marker.size.to_bits(),
                marker.price.map(f64::to_bits),
            )
        })
        .collect()
}

/// Time-axis bubbles written as one plain pass over the visible tape, independently of
/// `BubbleFold`: a qualifying print sits on the bar holding it (`print_bar_time`; none when the
/// session policy excludes it), merges into the newest bubble on the same bar, side, and price
/// within the window, the newest `max_markers` bubbles stay, sizes follow the peak retained volume,
/// and an id-less print is named by its own second.
fn reference_time_bubbles(
    stream: &FootprintAggregator,
    options: TradeBubbleOptions,
) -> Vec<BubbleRow> {
    struct Bubble {
        time: i64,
        price: f64,
        volume: f64,
        aggressor: AggressorSide,
        last_timestamp_micros: i64,
        id: String,
    }
    let mut bubbles = VecDeque::<Bubble>::new();
    for trade in stream.trades() {
        if trade.volume < options.minimum_volume {
            continue;
        }
        let Some(time) = stream.print_bar_time(trade.timestamp_micros) else {
            continue;
        };
        if let Some(bubble) = bubbles.back_mut().filter(|bubble| {
            options.aggregation_window_micros > 0
                && bubble.time == time
                && bubble.aggressor == trade.aggressor
                && bubble.price.to_bits() == trade.price.to_bits()
                && (trade.timestamp_micros - bubble.last_timestamp_micros).abs()
                    <= options.aggregation_window_micros
        }) {
            bubble.volume += trade.volume;
            bubble.last_timestamp_micros = trade.timestamp_micros;
            continue;
        }
        if bubbles.len() == options.max_markers {
            bubbles.pop_front();
        }
        let second = trade.timestamp_micros.div_euclid(MICROS_PER_SECOND);
        bubbles.push_back(Bubble {
            time,
            price: trade.price,
            volume: trade.volume,
            aggressor: trade.aggressor,
            last_timestamp_micros: trade.timestamp_micros,
            id: trade
                .trade_id
                .map_or_else(|| format!("trade-{second}"), |id| format!("trade-{id}")),
        });
    }
    let peak = bubbles
        .iter()
        .map(|bubble| bubble.volume)
        .fold(0.0_f64, f64::max);
    bubbles
        .into_iter()
        .map(|bubble| {
            (
                bubble.time,
                bubble.id,
                trade_bubble_size(bubble.volume, peak).to_bits(),
                Some(bubble.price.to_bits()),
            )
        })
        .collect()
}

/// How the retained bubbles sit relative to their first print: markers whose print is stamped
/// outside the bar they sit on (folded auction, lunch, and after-hours prints), and id-less markers,
/// which must be named by their print's second rather than their bar's.
#[derive(Default)]
struct BubblePlacement {
    folded: usize,
    named_by_print: usize,
}

impl Harness {
    #[track_caller]
    fn assert_bubbles_follow_print_bars(
        &self,
        tape: &[FootprintTrade],
        placement: &mut BubblePlacement,
        context: &str,
    ) {
        let stream = self.chart.trade_stream(self.stream).unwrap();
        let markers = &self.chart.series_entry(self.footprint).unwrap().markers;
        assert_eq!(
            bubble_rows(markers),
            reference_time_bubbles(stream, bubble_options()),
            "bubbles: {context}"
        );
        let bars = stream.bars();
        for marker in markers {
            assert!(
                bars.iter()
                    .any(|bar| bar.start_timestamp_micros == marker.time * MICROS_PER_SECOND),
                "a bubble sits on a bar: {context}"
            );
            let number = marker.id["trade-".len()..].parse::<i64>().unwrap();
            // Provider ids are tape positions + 1; an id-less marker carries a UTC second.
            let second = if number > 1_000_000 {
                placement.named_by_print += usize::from(number != marker.time);
                number
            } else {
                tape[number as usize - 1]
                    .timestamp_micros
                    .div_euclid(MICROS_PER_SECOND)
            };
            if !(marker.time..marker.time + SESSION_BAR_SECONDS).contains(&second) {
                placement.folded += 1;
            }
        }
    }
}

/// Live tips on session-anchored time bars stay suffix-bounded and end equal to a fresh load for
/// every dependent, the volume study included. Bubbles resume incrementally yet sit exactly where
/// the plain print-bar pass puts them: folded prints on the bar holding them, excluded prints
/// nowhere, id-less prints named by their own second.
#[test]
fn session_anchored_live_tips_match_a_fresh_rebuild_and_the_print_bar_reference() {
    let tape = session_tape(2);
    for outside in [OutOfSessionPolicy::Fold, OutOfSessionPolicy::Exclude] {
        let mut live = session_harness(outside, None);
        live.load(tape[..300].to_vec());
        let mut placement = BubblePlacement::default();
        let max_markers = bubble_options().max_markers as u64;
        for index in 300..tape.len() - 20 {
            let context = format!("{outside:?} tip {index}");
            let work = tip_work(&mut live, vec![tape[index].clone()]);
            assert_eq!(
                work.study_rows,
                STUDIES as u64 * work.changed_bars,
                "{context}"
            );
            assert_eq!(work.bar_rows, 2 * work.changed_bars, "{context}");
            assert_eq!(work.bubble_trades, 1, "{context}");
            let sizes = if work.peak_changed { max_markers } else { 1 };
            assert!(work.bubble_sizes <= sizes, "{context}");
            if index.is_multiple_of(31) {
                live.assert_bubbles_follow_print_bars(&tape, &mut placement, &context);
            }
        }
        live.tip(tape[tape.len() - 20..].to_vec());
        live.assert_bubbles_follow_print_bars(&tape, &mut placement, "final");
        let mut fresh = session_harness(outside, None);
        fresh.load(tape.clone());
        assert_same(&live.snapshot(), &fresh.snapshot(), &format!("{outside:?}"));
        // 48 five-minute bars per day: the lunch break and the night take no bars.
        assert_eq!(live.bar_opens().len(), 96, "{outside:?}");
        assert!(placement.named_by_print > 0, "{outside:?}");
        match outside {
            OutOfSessionPolicy::Fold => assert!(placement.folded > 0),
            // A closing-second print stays in its window's last bar.
            OutOfSessionPolicy::Exclude => {}
        }
    }
}

/// Retention on session-anchored time bars evicts by counted trades: the retained tape, running
/// state, and trade-id lookups equal a reconstruction of the retained tape, every retained row
/// (the volume study included) equals the unretained chart's tail, every tip equals a clean
/// rebuild, and bubbles still follow the print-bar reference. Under the exclude policy the lunch,
/// after-hours, and pre-open prints between an evicted bar and the first retained bar leave with
/// the evicted history instead of stranding the evicted bar's last trades on the tape.
#[test]
fn session_anchored_retention_tips_match_a_clean_rebuild_and_the_unretained_tail() {
    let tape = session_tape(2);
    for outside in [OutOfSessionPolicy::Fold, OutOfSessionPolicy::Exclude] {
        let mut live = session_harness(outside, Some(20));
        let mut unretained = session_harness(outside, None);
        live.load(tape[..400].to_vec());
        unretained.load(tape[..400].to_vec());
        let mut placement = BubblePlacement::default();
        let mut trims = 0;
        for index in 400..tape.len() {
            let context = format!("{outside:?} tip {index}");
            let before = live.first_bar_open();
            live.tip(vec![tape[index].clone()]);
            unretained.tip(vec![tape[index].clone()]);
            let trimmed = live.first_bar_open() != before;
            trims += usize::from(trimmed);
            if index.is_multiple_of(37) || trimmed {
                let stream = live.chart.trade_stream(live.stream).unwrap();
                let first = stream.trades().next().unwrap();
                assert!(
                    !stream.excluded_from_bars(first.timestamp_micros),
                    "the tape starts at the first retained bar: {context}"
                );
                for (&id, &position) in &stream.trade_ids {
                    assert_eq!(
                        stream.trades[position - stream.trade_id_base]
                            .event
                            .trade_id,
                        Some(id),
                        "{context}"
                    );
                }
                let mut reconstructed = stream.clone();
                reconstructed.rebuild();
                assert_eq!(
                    stream_state(stream),
                    stream_state(&reconstructed),
                    "{context}"
                );
                live.assert_bubbles_follow_print_bars(&tape, &mut placement, &context);
                let tipped = live.snapshot();
                assert_same(&tipped, &live.clean_rebuild_in_place(), &context);
            }
            let retained = live.snapshot();
            let reference = unretained.snapshot();
            let len = retained.footprint.times.len();
            let tail = |rows: &Rows| {
                (
                    rows.times[rows.times.len() - len..].to_vec(),
                    rows.values[rows.values.len() - len..].to_vec(),
                )
            };
            let rows_of = |rows: &Rows| (rows.times.clone(), rows.values.clone());
            assert_eq!(
                rows_of(&retained.footprint),
                tail(&reference.footprint),
                "{context}"
            );
            assert_eq!(
                rows_of(&retained.candles),
                tail(&reference.candles),
                "{context}"
            );
            for (study, (retained, reference)) in
                retained.studies.iter().zip(&reference.studies).enumerate()
            {
                assert_eq!(
                    rows_of(retained),
                    tail(reference),
                    "study {study}: {context}"
                );
            }
        }
        assert!(trims >= 3, "{outside:?} crosses the ceiling: {trims}");
    }
}

// Consumers of stream presentations: indicators and resampled series.

/// An indicator whose warm-up completes inside one update that also rewrites an earlier source
/// row starts its output at the first computed value, on trade-bound candles and host candles.
#[test]
fn an_indicator_warmed_up_by_one_batch_starts_at_its_first_value() {
    let mut live = Harness::new(time_bars(), None);
    live.load(tape(0..20));
    assert_eq!(live.bar_opens().len(), 2);
    let average = live.chart.add_sma(live.candles, 4).unwrap();
    assert!(rows(&live.chart, average).times.is_empty());
    // One tip rewrites the active bar and opens seven more.
    live.tip(tape(20..150));
    assert_eq!(live.bar_opens().len(), 9);
    let mut fresh = Harness::new(time_bars(), None);
    fresh.load(tape(0..150));
    let reference = fresh.chart.add_sma(fresh.candles, 4).unwrap();
    assert_eq!(rows(&live.chart, average), rows(&fresh.chart, reference));
    assert_eq!(rows(&live.chart, average).times.len(), 6);

    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    let candles = chart.add_series(SeriesKind::Candlestick);
    let average = chart.add_sma(candles, 4).unwrap();
    let column = |rows: usize| (0..rows).map(|row| 100.0 + row as f64).collect::<Vec<_>>();
    assert!(chart.install_series_data(
        candles,
        vec![60, 120],
        column(2),
        column(2),
        column(2),
        column(2)
    ));
    let times = (2..10).map(|minute| minute * 60).collect::<Vec<_>>();
    assert_eq!(
        chart.update_series_bars_sanitized(
            candles,
            times,
            column(8),
            column(8),
            column(8),
            column(8)
        ),
        8
    );
    let updated = rows(&chart, average);
    assert_eq!(updated.times, [240, 300, 360, 420, 480, 540]);
    chart.recompute_indicators_for(candles);
    assert_eq!(updated, rows(&chart, average));
}

/// Retention trims the candles and studies of a stream outside their own write paths. The
/// indicators and resampled bars reading them must still equal a recomputation from the retained
/// rows, after a capped load and after every tip, including the tips that trim.
#[test]
fn retention_trims_recompute_indicators_and_resampled_bars_of_every_presentation() {
    let mut live = Harness::new(time_bars(), Some(12));
    let target = live.chart.add_series(SeriesKind::Candlestick);
    let volume_target = live.chart.add_series(SeriesKind::Histogram);
    let start = BASE_MICROS / MICROS_PER_SECOND - 3_600;
    live.chart
        .configure_resampled_series(
            live.candles,
            Some(live.studies[4]),
            target,
            Some(volume_target),
            crate::ResampleOptions {
                interval_seconds: 300,
                boundaries: vec![crate::ResampleBoundary {
                    start_time: start,
                    end_time: start + 30 * 86_400,
                    session_id: 1,
                }],
            },
        )
        .unwrap();
    let averages = [
        live.chart.add_sma(live.candles, 5).unwrap(),
        live.chart.add_sma(live.studies[1], 5).unwrap(),
    ];
    let state = |chart: &ChartEngine| {
        (
            chart.resampled_bars(target).unwrap().to_vec(),
            rows(chart, target),
            rows(chart, volume_target),
            averages.map(|id| rows(chart, id)),
        )
    };
    let recomputed = |live: &mut Harness| {
        live.chart.recompute_indicators_for(live.candles);
        live.chart.recompute_indicators_for(live.studies[1]);
        state(&live.chart)
    };
    live.load(tape(0..300));
    assert_eq!(state(&live.chart), recomputed(&mut live), "capped load");
    let mut trims = 0;
    for index in 300..700 {
        let before = live.first_bar_open();
        live.tip(vec![tape_trade(index)]);
        trims += usize::from(live.first_bar_open() != before);
        assert_eq!(state(&live.chart), recomputed(&mut live), "tip {index}");
    }
    assert!(
        trims >= 3,
        "the tips must cross the ceiling repeatedly: {trims}"
    );
}

/// A provider correction that moves a revealed print past the replay clock hides it: every
/// dependent follows at once, and the print reappears when the clock passes it.
#[test]
fn a_correction_past_the_replay_clock_hides_the_print_from_every_dependent() {
    for aggregation in [time_bars(), trade_bars()] {
        let clock = tape_trade(100).timestamp_micros;
        let mut moved = tape_trade(60);
        moved.timestamp_micros = clock + 1;
        moved.session_id = tape_trade(100).session_id;
        let mut corrected = tape(0..120);
        corrected[60] = moved.clone();
        let mut live = Harness::new(aggregation, None);
        live.load(tape(0..120));
        live.chart.set_replay_clock_micros(Some(clock)).unwrap();
        assert_eq!(
            live.chart
                .update_trade_stream_trades(live.stream, vec![moved])
                .unwrap(),
            FootprintUpdateKind::Historical
        );
        let mut fresh = Harness::new(aggregation, None);
        fresh.load(corrected);
        fresh.chart.set_replay_clock_micros(Some(clock)).unwrap();
        assert_same(
            &live.snapshot(),
            &fresh.snapshot(),
            "hidden by the correction",
        );
        for chart in [&mut live.chart, &mut fresh.chart] {
            chart.set_replay_clock_micros(None).unwrap();
        }
        assert_same(&live.snapshot(), &fresh.snapshot(), "revealed");
    }
}

// Adversarial combination: session-anchored and sequence-axis streams driven at random through
// live tips, late and corrected prints, replay seeks, retention trims, session and time-zone
// changes, and host marker writes. After every step a never-rebuilt chart must equal a twin that
// rebuilds every dependent in place, and (without retention) a fresh load of the same tape.

/// Deterministic xorshift64* generator for the scenario drivers.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, bound: usize) -> usize {
        (self.next() % bound as u64) as usize
    }

    fn percent(&mut self, percent: u64) -> bool {
        self.next() % 100 < percent
    }
}

fn random_print(
    rng: &mut Rng,
    timestamp_micros: i64,
    session_id: Option<u64>,
    large: bool,
    id: u64,
) -> FootprintTrade {
    let price = 99.0 + rng.below(12) as f64 * 0.25;
    let aggressor = match rng.below(3) {
        0 => AggressorSide::Buy,
        1 => AggressorSide::Sell,
        _ => AggressorSide::Unknown,
    };
    let (bid, ask) = if aggressor == AggressorSide::Unknown && rng.percent(40) {
        let bid = price - rng.below(2) as f64 * 0.25;
        (Some(bid), Some(bid + 0.25))
    } else {
        (None, None)
    };
    FootprintTrade {
        timestamp_micros,
        price,
        volume: 1.0 + rng.below(9) as f64 + if large || rng.percent(10) { 40.0 } else { 0.0 },
        aggressor,
        bid,
        ask,
        sequence: rng.percent(85).then_some(id),
        trade_id: rng.percent(80).then_some(id),
        conditions: 0,
        session_id,
    }
}

/// Shanghai days of prints in canonical order: pre-open and repeated opening-auction prints, the
/// 09:30:00 open, a bar's last microsecond, repeated 11:30:00 and 15:00:00 prints, lunch, 15:05
/// and after-hours prints, continuous trading in both windows, and same-microsecond duplicates.
fn random_session_universe(rng: &mut Rng, days: usize, session_ids: bool) -> Vec<FootprintTrade> {
    const HOUR: i64 = 3_600;
    let first_day = aeris_charts_core::scale::session_slots::parse_iso_date("2023-11-15").unwrap();
    let mut stamps = Vec::new();
    for day in 0..days {
        let midnight = (first_day + day as i64) * 86_400 - 8 * HOUR;
        for special in [
            8 * HOUR + 59 * 60 + 30,
            9 * HOUR + 15 * 60,
            9 * HOUR + 25 * 60,
            9 * HOUR + 25 * 60,
            9 * HOUR + 30 * 60,
            11 * HOUR + 30 * 60,
            11 * HOUR + 30 * 60,
            12 * HOUR + 10 * 60,
            13 * HOUR,
            15 * HOUR,
            15 * HOUR,
            15 * HOUR + 5 * 60,
            15 * HOUR + 30 * 60,
            20 * HOUR,
        ] {
            stamps.push(((midnight + special) * MICROS_PER_SECOND, day, true));
        }
        stamps.push((
            (midnight + 10 * HOUR + 29 * 60 + 59) * MICROS_PER_SECOND + 999_999,
            day,
            false,
        ));
        for _ in 0..140 {
            let (open, close) = if rng.percent(50) {
                (9 * HOUR + 30 * 60, 11 * HOUR + 30 * 60)
            } else {
                (13 * HOUR, 15 * HOUR)
            };
            let second = open + rng.below((close - open) as usize) as i64;
            let micros = if rng.percent(30) {
                0
            } else {
                rng.below(1_000_000) as i64
            };
            stamps.push(((midnight + second) * MICROS_PER_SECOND + micros, day, false));
        }
    }
    stamps.sort_by_key(|stamp| stamp.0);
    let mut trades = Vec::new();
    for (timestamp_micros, day, large) in stamps {
        let copies = if rng.percent(8) { 2 } else { 1 };
        for _ in 0..copies {
            let id = trades.len() as u64 + 1;
            trades.push(random_print(
                rng,
                timestamp_micros,
                session_ids.then_some(day as u64),
                large,
                id,
            ));
        }
    }
    trades.sort_by_key(|trade| (trade.timestamp_micros, trade.sequence.unwrap_or(u64::MAX)));
    trades
}

#[derive(Clone, Debug)]
enum ScenarioOp {
    Update(Vec<FootprintTrade>),
    Clock(Option<i64>),
    Sessions(Option<OutOfSessionPolicy>),
    Zone(i64),
    /// Host marker writes on both bubble series, then a live batch.
    HostMarkers(Vec<FootprintTrade>),
}

/// Bubbles on the bound candles too, with a different window, size, and threshold.
fn candle_bubble_options() -> TradeBubbleOptions {
    TradeBubbleOptions {
        minimum_volume: 0.0,
        max_markers: 7,
        aggregation_window_micros: 2_000_000,
    }
}

struct ScenarioChart {
    harness: Harness,
    sessions: Option<OutOfSessionPolicy>,
    zone_hours: i64,
    averages: [SeriesId; 2],
    resampled: Option<(SeriesId, SeriesId)>,
}

/// Rows of every derived consumer of the stream presentations.
type DerivedState = (Vec<Rows>, Option<(Vec<crate::ResampledBar>, Rows, Rows)>);

impl ScenarioChart {
    fn new(
        aggregation: FootprintAggregationOptions,
        max_points: Option<usize>,
        sessions: Option<OutOfSessionPolicy>,
        zone_hours: i64,
    ) -> Self {
        let mut harness = Harness::with_bubbles(aggregation, max_points, bubble_options());
        harness
            .chart
            .add_trade_bubbles(harness.stream, harness.candles, candle_bubble_options())
            .unwrap();
        // Derived consumers of the stream presentations: moving averages of the candles and the
        // continuous CVD, and (on a time axis) half-hour bars resampled from the candles and the
        // stream volume inside the A-share windows.
        let averages = [
            harness.chart.add_sma(harness.candles, 4).unwrap(),
            harness.chart.add_sma(harness.studies[1], 3).unwrap(),
        ];
        let resampled =
            matches!(aggregation.bars, FootprintBarAggregation::Time { .. }).then(|| {
                let chart = &mut harness.chart;
                let target = chart.add_series(SeriesKind::Candlestick);
                let volume_target = chart.add_series(SeriesKind::Histogram);
                let first_day =
                    aeris_charts_core::scale::session_slots::parse_iso_date("2023-11-15").unwrap();
                let exchange =
                    aeris_charts_core::scale::exchange_time::ExchangeTime::new(shanghai(), 0)
                        .unwrap();
                let boundaries = crate::resample_boundaries(
                    &(first_day..first_day + 4).collect::<Vec<_>>(),
                    &a_share_sessions(OutOfSessionPolicy::Fold).windows,
                    &exchange,
                    crate::ResampleSpan::Window,
                )
                .unwrap();
                chart
                    .configure_resampled_series(
                        harness.candles,
                        Some(harness.studies[4]),
                        target,
                        Some(volume_target),
                        crate::ResampleOptions {
                            interval_seconds: 1_800,
                            boundaries,
                        },
                    )
                    .unwrap();
                (target, volume_target)
            });
        let mut chart = Self {
            harness,
            sessions: None,
            zone_hours: 0,
            averages,
            resampled,
        };
        chart.apply(&ScenarioOp::Zone(zone_hours)).unwrap();
        chart.apply(&ScenarioOp::Sessions(sessions)).unwrap();
        chart
    }

    fn apply(&mut self, op: &ScenarioOp) -> Result<Option<FootprintUpdateKind>, FootprintError> {
        let chart = &mut self.harness.chart;
        let stream = self.harness.stream;
        match op {
            ScenarioOp::Update(trades) => chart
                .update_trade_stream_trades(stream, trades.clone())
                .map(Some),
            ScenarioOp::Clock(clock) => chart.set_replay_clock_micros(*clock).map(|_| None),
            ScenarioOp::Sessions(sessions) => {
                chart.set_trade_stream_sessions(stream, sessions.map(a_share_sessions))?;
                self.sessions = *sessions;
                Ok(None)
            }
            ScenarioOp::Zone(hours) => {
                chart
                    .set_time_zone(crate::UtcOffsetSchedule::fixed((*hours * 3_600) as _).unwrap());
                self.zone_hours = *hours;
                Ok(None)
            }
            ScenarioOp::HostMarkers(trades) => {
                let marker = Marker {
                    time: 1,
                    position: marker_pos::AT_PRICE_MIDDLE,
                    shape: marker_shape::CIRCLE,
                    color: Color::rgb(1, 2, 3),
                    text: String::from("host"),
                    id: String::from("host"),
                    size: 1.0,
                    price: None,
                };
                chart.set_series_markers(self.harness.footprint, vec![marker.clone(); 3]);
                chart.set_series_markers(self.harness.candles, vec![marker]);
                chart
                    .update_trade_stream_trades(stream, trades.clone())
                    .map(Some)
            }
        }
    }

    fn stream(&self) -> &FootprintAggregator {
        self.harness
            .chart
            .trade_stream(self.harness.stream)
            .unwrap()
    }

    fn candle_markers(&self) -> Vec<MarkerRow> {
        markers(&self.harness.chart, self.harness.candles)
    }

    fn derived(&self) -> DerivedState {
        let chart = &self.harness.chart;
        (
            self.averages.iter().map(|&id| rows(chart, id)).collect(),
            self.resampled.map(|(target, volume)| {
                (
                    chart.resampled_bars(target).unwrap().to_vec(),
                    rows(chart, target),
                    rows(chart, volume),
                )
            }),
        )
    }

    /// The whole retained tape, hidden trades included, in canonical order.
    fn tape(&self) -> Vec<FootprintTrade> {
        self.stream()
            .trades
            .iter()
            .map(|stored| stored.event.clone())
            .collect()
    }
}

#[derive(Clone, Copy, Debug)]
struct Scenario {
    aggregation: FootprintAggregationOptions,
    sessions: Option<OutOfSessionPolicy>,
    session_ids: bool,
    max_points: Option<usize>,
    seed: u64,
    steps: usize,
}

/// How often a scenario exercised each path, so a test can prove it is not vacuous.
#[derive(Debug, Default)]
struct Coverage {
    tips: usize,
    historical: usize,
    settings: usize,
    rejected: usize,
    trims: usize,
}

fn run_scenario(scenario: Scenario) -> Coverage {
    let time_axis = matches!(
        scenario.aggregation.bars,
        FootprintBarAggregation::Time { .. }
    );
    let mut rng = Rng(scenario.seed);
    let universe = random_session_universe(&mut rng, 3, scenario.session_ids);
    let sessions = if time_axis { scenario.sessions } else { None };
    let mut inc = ScenarioChart::new(scenario.aggregation, scenario.max_points, sessions, 8);
    let mut clean = ScenarioChart::new(scenario.aggregation, scenario.max_points, sessions, 8);
    let initial = universe.len() / 5;
    for chart in [&mut inc, &mut clean] {
        chart.harness.load(universe[..initial].to_vec());
    }
    let mut delivered = initial;
    let mut next_id = 50_000_000;
    let mut clock = None;
    let mut coverage = Coverage::default();
    let deliver = |count: usize, delivered: &mut usize| {
        let end = (*delivered + count).min(universe.len());
        let batch = universe[*delivered..end].to_vec();
        *delivered = end;
        batch
    };
    for step in 0..scenario.steps {
        let roll = rng.below(100);
        let tape = inc.tape();
        let op = match roll {
            0..=44 => {
                let count = 1 + rng.below(3);
                ScenarioOp::Update(deliver(count, &mut delivered))
            }
            45..=54 if !tape.is_empty() => {
                let base = &tape[rng.below(tape.len())];
                next_id += 1;
                let timestamp_micros = base.timestamp_micros + rng.below(3) as i64;
                let mut late =
                    random_print(&mut rng, timestamp_micros, base.session_id, false, next_id);
                late.sequence = rng.percent(50).then_some(next_id);
                ScenarioOp::Update(vec![late])
            }
            55..=64 if tape.iter().any(|trade| trade.trade_id.is_some()) => {
                let mut batch = Vec::new();
                for _ in 0..1 + rng.below(2) {
                    let candidates = tape
                        .iter()
                        .filter(|trade| {
                            trade.trade_id.is_some()
                                && batch
                                    .iter()
                                    .all(|other: &FootprintTrade| other.trade_id != trade.trade_id)
                        })
                        .collect::<Vec<_>>();
                    if candidates.is_empty() {
                        break;
                    }
                    let mut corrected = candidates[rng.below(candidates.len())].clone();
                    corrected.volume += 1.0 + rng.below(5) as f64;
                    if rng.percent(40) {
                        corrected.price = 99.0 + rng.below(12) as f64 * 0.25;
                    }
                    if rng.percent(30) {
                        corrected.aggressor = match rng.below(3) {
                            0 => AggressorSide::Buy,
                            1 => AggressorSide::Sell,
                            _ => AggressorSide::Unknown,
                        };
                        corrected.bid = None;
                        corrected.ask = None;
                    }
                    if rng.percent(30) {
                        corrected.timestamp_micros += rng.below(600_000) as i64 - 300_000;
                    }
                    batch.push(corrected);
                }
                if rng.percent(30) {
                    batch.extend(deliver(1, &mut delivered));
                }
                ScenarioOp::Update(batch)
            }
            65..=74 if delivered > 0 => {
                let target = universe[rng.below(delivered)].timestamp_micros;
                let jitter = rng.below(3) as i64 - 1;
                ScenarioOp::Clock(Some(match rng.below(4) {
                    // Between the opening auction and the 09:30 open of that print's day.
                    0 => {
                        let second = target.div_euclid(MICROS_PER_SECOND);
                        let local_day = (second + 8 * 3_600).div_euclid(86_400);
                        (local_day * 86_400 - 8 * 3_600 + 9 * 3_600 + 27 * 60) * MICROS_PER_SECOND
                    }
                    _ => target + jitter,
                }))
            }
            75..=79 => ScenarioOp::Clock(None),
            80..=84 if time_axis => ScenarioOp::Sessions(match rng.below(3) {
                0 => None,
                1 => Some(OutOfSessionPolicy::Fold),
                _ => Some(OutOfSessionPolicy::Exclude),
            }),
            85..=87 if time_axis => ScenarioOp::Zone(if inc.zone_hours == 8 { 9 } else { 8 }),
            // Host markers stay until the next visible refresh replaces them.
            88..=91 if clock.is_none() && delivered < universe.len() => {
                ScenarioOp::HostMarkers(deliver(1, &mut delivered))
            }
            // A provider correction moves a revealed print behind the replay clock.
            92..=95
                if clock.is_some_and(|clock| {
                    tape.iter()
                        .any(|trade| trade.trade_id.is_some() && trade.timestamp_micros <= clock)
                }) =>
            {
                let clock = clock.unwrap();
                let revealed = tape
                    .iter()
                    .filter(|trade| trade.trade_id.is_some() && trade.timestamp_micros <= clock)
                    .collect::<Vec<_>>();
                let mut moved = revealed[rng.below(revealed.len())].clone();
                moved.timestamp_micros = clock + 1 + rng.below(400_000) as i64;
                ScenarioOp::Update(vec![moved])
            }
            _ => {
                let count = 5 + rng.below(40);
                ScenarioOp::Update(deliver(count, &mut delivered))
            }
        };
        if let ScenarioOp::Clock(next) = op {
            clock = next;
        }
        let context = format!("{scenario:?} step {step}: {op:?}");
        let context = &context[..context.len().min(600)];
        let first_bar = inc
            .stream()
            .bars()
            .first()
            .map(|bar| bar.start_timestamp_micros);
        let applied = inc.apply(&op);
        assert_eq!(applied, clean.apply(&op), "{context}");
        match applied {
            Ok(Some(FootprintUpdateKind::Tip)) => coverage.tips += 1,
            Ok(Some(FootprintUpdateKind::Historical)) => coverage.historical += 1,
            Ok(None) => coverage.settings += 1,
            Err(_) => coverage.rejected += 1,
        }
        if first_bar.is_some()
            && inc
                .stream()
                .bars()
                .first()
                .map(|bar| bar.start_timestamp_micros)
                > first_bar
            && scenario.max_points.is_some()
        {
            coverage.trims += 1;
        }
        let expected = clean.harness.clean_rebuild_in_place();
        let actual = inc.harness.snapshot();
        assert_same(&actual, &expected, context);
        assert_eq!(
            inc.candle_markers(),
            clean.candle_markers(),
            "candle bubbles: {context}"
        );
        assert_eq!(inc.derived(), clean.derived(), "derived: {context}");
        assert_eq!(
            stream_state(inc.stream()),
            stream_state(clean.stream()),
            "stream: {context}"
        );
        let mut reconstructed = inc.stream().clone();
        reconstructed.rebuild();
        assert_eq!(
            stream_state(inc.stream()),
            stream_state(&reconstructed),
            "reconstruction: {context}"
        );
        if time_axis {
            assert_eq!(
                bubble_rows(
                    &inc.harness
                        .chart
                        .series_entry(inc.harness.footprint)
                        .unwrap()
                        .markers
                ),
                reference_time_bubbles(inc.stream(), bubble_options()),
                "print-bar bubbles: {context}"
            );
        }
        if scenario.max_points.is_none() {
            let mut fresh =
                ScenarioChart::new(scenario.aggregation, None, inc.sessions, inc.zone_hours);
            fresh.harness.load(inc.tape());
            fresh.apply(&ScenarioOp::Clock(clock)).unwrap();
            assert_same(
                &actual,
                &fresh.harness.snapshot(),
                &format!("fresh: {context}"),
            );
            assert_eq!(
                inc.candle_markers(),
                fresh.candle_markers(),
                "fresh candle bubbles: {context}"
            );
            assert_eq!(inc.derived(), fresh.derived(), "fresh derived: {context}");
        }
    }
    coverage
}

/// Assert that a scenario exercised live tips, historical rebuilds, settings and clock moves, and
/// (under a ceiling) retention trims.
fn assert_exercised(coverage: &Coverage, retained: bool, context: &str) {
    assert!(coverage.tips >= 50, "{context}: {coverage:?}");
    assert!(coverage.historical >= 20, "{context}: {coverage:?}");
    assert!(coverage.settings >= 20, "{context}: {coverage:?}");
    assert!(!retained || coverage.trims >= 5, "{context}: {coverage:?}");
}

#[test]
fn adversarial_session_streams_match_a_clean_rebuild_after_every_step() {
    let mut rejected = 0;
    for (seed, outside, session_ids, max_points) in [
        (1, Some(OutOfSessionPolicy::Fold), true, None),
        (2, Some(OutOfSessionPolicy::Exclude), false, None),
        (3, Some(OutOfSessionPolicy::Fold), true, Some(20)),
        (4, Some(OutOfSessionPolicy::Exclude), true, Some(20)),
        (5, None, false, Some(9)),
    ] {
        let coverage = run_scenario(Scenario {
            aggregation: session_bars(),
            sessions: outside,
            session_ids,
            max_points,
            seed,
            steps: 220,
        });
        assert_exercised(&coverage, max_points.is_some(), &format!("seed {seed}"));
        rejected += coverage.rejected;
    }
    // A correction that moves a print into another session's bar is rejected atomically.
    assert!(rejected > 0);
}

#[test]
fn adversarial_sequence_streams_match_a_clean_rebuild_after_every_step() {
    let volume_bars = FootprintAggregationOptions {
        bars: FootprintBarAggregation::Volume {
            volume_per_bar: 60.0,
        },
        ..FootprintAggregationOptions::default()
    };
    let range_bars = FootprintAggregationOptions {
        bars: FootprintBarAggregation::Range { range_ticks: 4 },
        ..FootprintAggregationOptions::default()
    };
    for (seed, aggregation, max_points) in [
        (11, trade_bars(), None),
        (12, trade_bars(), Some(30)),
        (13, volume_bars, Some(25)),
        (14, range_bars, None),
        (15, range_bars, Some(30)),
    ] {
        let coverage = run_scenario(Scenario {
            aggregation,
            sessions: None,
            session_ids: seed % 2 == 0,
            max_points,
            seed,
            steps: 220,
        });
        assert_exercised(&coverage, max_points.is_some(), &format!("seed {seed}"));
    }
}
