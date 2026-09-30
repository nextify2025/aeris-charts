//! Retention trim cost. Report-only (no thresholds): the repository's `perf_gate` Target D covers
//! one small shape, while the trim is shaped by the series count and the retained rows together.
//!
//!   Matrix  — one retention applied to a standalone data layer of `S` aligned OHLC series holding
//!             `N` rows each, `EVICTED` rows leaving every series, one row per call per series.
//!   Engine  — the footprint, bound candles, CVD, delta and bubbles of one trade stream (the
//!             `perf_gate` Target D shape) with the retention cap at `N` bars: the trim tips' own
//!             cost and their timestamp-union rebuild count.
//!
//! Run: `cargo run -p aeris_charts_native --example retention_perf --release`

use std::time::Instant;

use aeris_charts_core::model::data_layer::DataLayer;
use aeris_charts_engine::{
    AggressorSide, ChartEngine, FootprintAggregationOptions, FootprintBarAggregation,
    FootprintSeriesOptions, FootprintTrade, FootprintVisualOptions, SeriesKind, TradeBubbleOptions,
    TradeStudyOptions,
};

const EVICTED: usize = 80;
const MATRIX_RUNS: usize = 15;
const SERIES_COUNTS: [usize; 3] = [1, 4, 8];
const ROW_COUNTS: [usize; 4] = [2_500, 10_000, 28_800, 40_000];
const ENGINE_CAPS: [usize; 3] = [2_500, 10_000, 28_800];
const ENGINE_TRADES_PER_BAR: usize = 4;
const ENGINE_TRIMS: usize = 3;

fn median(samples: &mut [f64]) -> f64 {
    samples.sort_by(f64::total_cmp);
    samples[samples.len() / 2]
}

/// `series` aligned OHLC series of `rows` rows each on one shared timeline.
fn layer(series: usize, rows: usize) -> (DataLayer, Vec<u32>) {
    let times = (0..rows as i64).collect::<Vec<_>>();
    let mut price = 100.0;
    let columns = (0..rows)
        .map(|row| {
            let next = price + (row as f64 * 0.017).sin() * 0.8;
            let bar = [price, price.max(next) + 0.4, price.min(next) - 0.4, next];
            price = next;
            bar
        })
        .collect::<Vec<_>>();
    let column = |index: usize| columns.iter().map(|bar| bar[index]).collect::<Vec<_>>();
    let mut data = DataLayer::new();
    let ids = (0..series)
        .map(|_| {
            let id = data.add_series();
            assert!(data.set_data(
                id,
                times.clone(),
                column(0),
                column(1),
                column(2),
                column(3)
            ));
            id
        })
        .collect();
    (data, ids)
}

fn matrix() {
    println!("Matrix — {EVICTED} evicted rows per series, median of {MATRIX_RUNS} runs:");
    for series in SERIES_COUNTS {
        for rows in ROW_COUNTS {
            let mut sequential = Vec::with_capacity(MATRIX_RUNS);
            let mut batched = Vec::with_capacity(MATRIX_RUNS);
            let (mut sequential_passes, mut batched_passes) = (0, 0);
            for _ in 0..MATRIX_RUNS {
                let (mut data, ids) = layer(series, rows);
                let passes = data.index_rebuilds();
                let start = Instant::now();
                for &id in &ids {
                    data.trim_front(id, rows - EVICTED);
                }
                sequential.push(start.elapsed().as_secs_f64() * 1000.0);
                sequential_passes = data.index_rebuilds() - passes;

                let (mut data, ids) = layer(series, rows);
                let trims = ids
                    .iter()
                    .map(|&id| (id, rows - EVICTED))
                    .collect::<Vec<_>>();
                let passes = data.index_rebuilds();
                let start = Instant::now();
                data.trim_fronts(&trims);
                batched.push(start.elapsed().as_secs_f64() * 1000.0);
                batched_passes = data.index_rebuilds() - passes;
            }
            println!(
                "  S={series} N={rows:>6}: sequential trim_front x S {:>8.3} ms ({sequential_passes:>2} union passes) | trim_fronts {:>8.3} ms ({batched_passes} union passes)",
                median(&mut sequential),
                median(&mut batched),
            );
        }
    }
}

fn gen_trades(start_bar: usize, bars: usize) -> Vec<FootprintTrade> {
    let mut trades = Vec::with_capacity(bars * ENGINE_TRADES_PER_BAR);
    for bar in start_bar..start_bar + bars {
        let bar_time = bar as i64 * 60_000_000;
        for tick in 0..ENGINE_TRADES_PER_BAR {
            let level = ((bar + tick * 7) % 21) as i64 - 10;
            trades.push(FootprintTrade {
                timestamp_micros: bar_time + tick as i64 * 1_000,
                price: 100.0 + level as f64 * 0.25,
                volume: (tick % 17 + 1) as f64,
                aggressor: if (bar + tick) % 2 == 0 {
                    AggressorSide::Buy
                } else {
                    AggressorSide::Sell
                },
                bid: None,
                ask: None,
                sequence: Some(tick as u64),
                trade_id: Some((bar * ENGINE_TRADES_PER_BAR + tick) as u64),
                conditions: 0,
                session_id: Some((bar / 1_440) as u64),
            });
        }
    }
    trades
}

fn engine(cap: usize) {
    let aggregation = FootprintAggregationOptions {
        tick_size: 0.25,
        ticks_per_row: 1,
        bars: FootprintBarAggregation::Trades {
            trades_per_bar: ENGINE_TRADES_PER_BAR as u32,
        },
        ..FootprintAggregationOptions::default()
    };
    let mut chart = ChartEngine::new(1600.0, 800.0, 1.0);
    chart
        .configure_footprint_series(
            0,
            FootprintSeriesOptions {
                aggregation,
                visual: FootprintVisualOptions::default(),
            },
        )
        .expect("valid footprint options");
    let stream = chart
        .add_trade_stream("PERF:ES", aggregation)
        .expect("valid shared footprint stream");
    chart
        .bind_footprint_series_to_stream(0, stream)
        .expect("bind footprint stream");
    let candles = chart.add_series(SeriesKind::Candlestick);
    chart
        .bind_trade_bar_series_to_stream(candles, stream)
        .expect("bind trade candle stream");
    chart
        .add_cvd_series(stream, 1, TradeStudyOptions::default())
        .expect("add CVD dependent");
    chart
        .add_delta_series(stream, 1)
        .expect("add delta dependent");
    chart
        .add_trade_bubbles(
            stream,
            0,
            TradeBubbleOptions {
                minimum_volume: 10.0,
                max_markers: 2_048,
                aggregation_window_micros: 0,
            },
        )
        .expect("add bubble dependent");
    chart
        .set_footprint_trades(0, gen_trades(0, cap))
        .expect("valid footprint history");
    assert!(chart.set_series_max_points(0, Some(cap)));
    let first_row_key = |chart: &ChartEngine| {
        chart
            .data_layer()
            .series_data(0)
            .and_then(|(times, _)| times.first().copied())
    };
    // The hysteresis margin is cap / 32 bars; run until a few trims have happened.
    let mut trim_ms = Vec::new();
    let mut trim_passes = Vec::new();
    let mut tips = 0usize;
    let mut next_bar = cap;
    while trim_ms.len() < ENGINE_TRIMS {
        for trade in gen_trades(next_bar, 1) {
            let first_key = first_row_key(&chart);
            let passes = chart.data_layer().index_rebuilds();
            let start = Instant::now();
            chart
                .update_footprint_trades(0, vec![trade])
                .expect("valid footprint tip");
            let ms = start.elapsed().as_secs_f64() * 1000.0;
            if first_row_key(&chart) != first_key {
                trim_ms.push(ms);
                trim_passes.push(chart.data_layer().index_rebuilds() - passes);
            }
            tips += 1;
        }
        next_bar += 1;
    }
    println!(
        "  cap {cap:>6} bars: trim tip {:>8.3} ms median ({:>8.3} ms max) over {ENGINE_TRIMS} trims / {tips} tips, union passes per trim tip {trim_passes:?}",
        median(&mut trim_ms.clone()),
        trim_ms.iter().copied().fold(0.0, f64::max),
    );
}

fn main() {
    println!("aeris_charts retention trim (release build required, report-only)\n");
    matrix();
    println!("Engine — Target D shape (footprint, candles, CVD, delta, bubbles), {ENGINE_TRADES_PER_BAR} trades per bar:");
    for cap in ENGINE_CAPS {
        engine(cap);
    }
}
