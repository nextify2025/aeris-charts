//! Performance gate for the repository's named production targets (roadmap). Headless: measures the
//! `aeris_charts_engine` CPU cost — frame construction and data ingestion — which is what governs whether
//! the browser can hit 60fps; GPU present time is a separate, backend-specific concern.
//!
//!   Target A — 60fps @ 10 series x 50k bars:  `build_frame` under 16.67 ms/frame
//!   Target B — 1M-bar load under 300 ms:      `set_series_data` of 1,000,000 bars
//!   Target C — canonical pointer sample:      fixed-capacity resolver under 0.01 ms/sample
//!   Target D — shared non-time candles/footprint history, live, correction, frame construction,
//!              and bounded single-trade live tips across every stream dependent
//!   Target D2 — report-only retention trim of the data layer across series counts and retained
//!              rows: one `trim_fronts` versus one `trim_front` per series
//!   Target E — 100k visible-bar volume profile refresh, periodic developing paths, and cached frame
//!   Target F — 100k-point general XY line frame + nearest-hit interaction
//!   Target G — mixed 100k-row general dashboard frame, hit interaction, and retained memory
//!   Target H — combined 50k-bar financial + 50k-point general frame and retained memory
//!   Target I — 100k-row numeric error bars frame, hit interaction, and retained memory
//!   Target K — 100x replay clock advance, shared projections, frame work, and flat retained memory
//!   Target L — sustained depth updates, bounded heatmap frame work, and live-edge upload size
//!   Target M — per-tick cost of every built-in indicator bound to a 1M-row source, and the
//!              bounded capacity of the aggregate price columns composite-input studies retain;
//!              the same per-tick cost, work rows, runtime bytes and a historical repair for
//!              all 27 KLineChart templates bound to a 1M-row source
//!   Target N — live ticks plus frame construction with regression trends anchored across a
//!              1M-row source (data-reading drawings follow ticks by the changed rows)
//!   Target O — daily-reset studies on daily bars: report-only frame, Canvas2D call, rasterizer,
//!              WebGPU scheduling, and hover cost of the per-bar segments a session-reset study
//!              draws when every bar is its own period
//!
//! Report-only by default (prints numbers + PASS/FAIL). Set `AERIS_CHARTS_PERF_STRICT=1` to exit non-zero
//! on any failure so CI can treat it as a hard gate; thresholds are machine-dependent, so the
//! strict mode is opt-in rather than the default. Only the value `1` enforces: unset, `0`, or
//! anything else stays report-only.
//!
//! Run: `cargo run -p aeris_charts_native --example perf_gate --release`

use std::process::ExitCode;
use std::time::Instant;

use aeris_charts_core::model::data_layer::{DataLayer, SeriesId};
use aeris_charts_engine::{
    AggressorSide, AxisDimension, ChartEngine, ChartFrame, ContinuousScaleType,
    DepthHeatmapOptions, DepthLevel, DepthOptions, DepthSide, DepthSnapshot, DepthUpdate,
    FootprintAggregationOptions, FootprintBarAggregation, FootprintSeriesOptions, FootprintTrade,
    FootprintVisualOptions, GeneralAxisOptions, GeneralHitMode, GeneralScaleType,
    GeneralSeriesOptions, GeneralXyInput, GestureResolver, HorizontalDomain, InputDevice,
    InputTarget, PeriodicProfilePresentationOptions, PeriodicProfilePresentationRequest,
    PointerSample, ProfileSource, ResampleBoundary, SeriesKind, TradeBubbleOptions,
    TradeStudyOptions,
};
use aeris_charts_native::render_prims;
use aeris_charts_render::canvas2d::{execute, Canvas2d, Viewport};
use aeris_charts_render::color::Color;
use aeris_charts_render::draw_list::Prim;
use aeris_charts_render_wgpu::{prims_to_group, DrawGroup, TexQuadInstance};

/// Parallel `(times, open, high, low, close)` columns.
type OhlcColumns = (Vec<f64>, Vec<f64>, Vec<f64>, Vec<f64>, Vec<f64>);

/// Deterministic OHLC columns for `bars` points, offset by `phase` so stacked series differ.
fn gen_series(bars: usize, phase: f64) -> OhlcColumns {
    let mut times = Vec::with_capacity(bars);
    let mut open = Vec::with_capacity(bars);
    let mut high = Vec::with_capacity(bars);
    let mut low = Vec::with_capacity(bars);
    let mut close = Vec::with_capacity(bars);
    let mut price = 100.0 + phase;
    for i in 0..bars {
        let next = price + ((i as f64) * 0.017 + phase).sin() * 0.8;
        times.push(i as f64);
        open.push(price);
        high.push(price.max(next) + 0.4);
        low.push(price.min(next) - 0.4);
        close.push(next);
        price = next;
    }
    (times, open, high, low, close)
}

fn report(label: &str, measured_ms: f64, budget_ms: f64) -> bool {
    let pass = measured_ms <= budget_ms;
    println!(
        "  [{}] {label}: {measured_ms:.2} ms (budget {budget_ms:.2} ms)",
        if pass { "PASS" } else { "FAIL" }
    );
    pass
}

fn report_bytes(label: &str, measured_bytes: usize, budget_bytes: usize) -> bool {
    let pass = measured_bytes <= budget_bytes;
    println!(
        "  [{}] {label}: {:.2} MiB (budget {:.2} MiB)",
        if pass { "PASS" } else { "FAIL" },
        measured_bytes as f64 / (1024.0 * 1024.0),
        budget_bytes as f64 / (1024.0 * 1024.0),
    );
    pass
}

fn report_check(label: &str, pass: bool, detail: &str) -> bool {
    println!(
        "  [{}] {label}: {detail}",
        if pass { "PASS" } else { "FAIL" }
    );
    pass
}

fn gen_footprint_trades(
    start_bar: usize,
    bars: usize,
    trades_per_bar: usize,
) -> Vec<FootprintTrade> {
    let mut trades = Vec::with_capacity(bars * trades_per_bar);
    for bar in start_bar..start_bar + bars {
        let bar_time = bar as i64 * 60_000_000;
        for tick in 0..trades_per_bar {
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
                trade_id: Some((bar * trades_per_bar + tick) as u64),
                conditions: 0,
                session_id: Some((bar / 1_440) as u64),
            });
        }
    }
    trades
}

/// Measure the WebGPU adapter's CPU-side primitive scheduling for a dense numbers bar. The real
/// atlas rasterizer and GPU present are host/device concerns; this gate covers the bounded frame
/// encoding work that runs before those submissions and keeps text runs in prim order.
fn dense_footprint_wgpu_scene_ms() -> (usize, usize, f64) {
    const BARS: usize = 20;
    const TRADES_PER_BAR: usize = 22;
    const RUNS: usize = 30;
    let mut chart = ChartEngine::new(1600.0, 800.0, 1.0);
    let footprint = chart
        .add_footprint_series(FootprintSeriesOptions::default())
        .expect("default footprint options are valid");
    chart.set_series_visible(0, false);
    chart
        .set_footprint_trades(footprint, gen_footprint_trades(0, BARS, TRADES_PER_BAR))
        .expect("dense footprint trades are valid");
    chart.time_scale.set_width(1600.0);
    chart.fit_content();
    chart.set_bar_spacing(72.0);
    let frame = chart.build_frame();
    let pane = &frame.panes[0];
    let text_count = pane
        .main
        .iter()
        .filter(|prim| matches!(prim, Prim::Text { .. } | Prim::RotatedText { .. }))
        .count();
    let mut group = DrawGroup::default();
    let mut resolve_text = |prim: &Prim| -> Option<TexQuadInstance> {
        let (x, y) = match prim {
            Prim::Text { x, y, .. } | Prim::RotatedText { x, y, .. } => (*x, *y),
            _ => return None,
        };
        Some(TexQuadInstance {
            rect: [x, y, 2.0, 2.0],
            uv: [0.0, 0.0, 0.5, 0.5],
            color: [0.0, 0.0, 0.0, 1.0],
        })
    };
    let mut resolve_image = |_prim: &Prim| None;
    for _ in 0..5 {
        group.clear();
        prims_to_group(
            &pane.main,
            &pane.points,
            &mut group,
            &mut resolve_text,
            &mut resolve_image,
        );
    }
    let mut samples = Vec::with_capacity(RUNS);
    let mut text_instances = 0;
    for _ in 0..RUNS {
        group.clear();
        let started = Instant::now();
        prims_to_group(
            &pane.main,
            &pane.points,
            &mut group,
            &mut resolve_text,
            &mut resolve_image,
        );
        samples.push(started.elapsed().as_nanos() as u64);
        text_instances = group.tex_quads.len();
    }
    samples.sort_unstable();
    let p99_index = ((samples.len() as f64 - 1.0) * 0.99).round() as usize;
    (
        text_count,
        text_instances,
        samples[p99_index] as f64 / 1_000_000.0,
    )
}

/// Per-tick indicator cost measured by Target M.
struct IndicatorTickCost {
    bindings: usize,
    /// `(mean, median, max)` milliseconds per current-bar replacement tick.
    replace_ms: (f64, f64, f64),
    /// `(mean, median, max)` milliseconds per new-bar append tick.
    append_ms: (f64, f64, f64),
    /// The excluded first append after the bulk install.
    first_append_ms: f64,
    /// Capacity the first append grew across the engine's data columns and indicator runtimes.
    first_append_growth_bytes: usize,
    /// Largest `last_indicator_work_rows` any measured tick reported.
    max_work_rows: usize,
    /// Indicator runtime bytes after the install and every measured tick.
    runtime_bytes: usize,
    /// `(mean, median, max)` milliseconds to revise a bar [`REPAIR_DEPTH`] rows before the newest
    /// on every series the studies read, measured for the KLineChart set only.
    repair_ms: Option<(f64, f64, f64)>,
}

/// Rows between the newest bar and the historical correction Target M (KLineChart) times.
const REPAIR_DEPTH: usize = 500;
/// Corrections timed for that sample.
const REPAIR_SAMPLES: usize = 20;

/// The studies one Target M measurement binds to its source.
#[derive(Clone, Copy, PartialEq)]
enum StudySet {
    /// Every built-in study kind plus the aggregate-input studies.
    BuiltIn,
    /// All 27 KLineChart templates with KLineChart's default parameters, the volume-reading ones
    /// on a volume series and AVP on a turnover series.
    KLineChart,
}

/// Deterministic `[open, high, low, close]` for `row`; `revision` moves the close so a tick
/// replacing the forming bar changes its values.
fn indicator_bar(row: usize, revision: usize) -> [f64; 4] {
    let base = 100.0 + (row as f64 * 0.0007).sin() * 12.0 + (row as f64 * 0.013).sin() * 1.5;
    let close = base + revision as f64 * 0.01;
    let open = base - (row as f64 * 0.31).cos() * 0.4;
    [open, open.max(close) + 0.35, open.min(close) - 0.3, close]
}

/// Binds every built-in study kind, and the aggregate-input studies, to the candle series 0 (the
/// volume-reading kinds to `volume`). Returns the number of bindings.
fn bind_builtin_studies(chart: &mut ChartEngine, volume: SeriesId) -> usize {
    use aeris_charts_engine::{
        DeviationEstimator, IndicatorInputSource, IndicatorKind, IndicatorSeed, KdjSeed, PivotKind,
        VwapReset,
    };

    let kinds = [
        IndicatorKind::Sma { period: 20 },
        IndicatorKind::Ema {
            period: 20,
            seed: IndicatorSeed::Sma,
        },
        IndicatorKind::Dema {
            period: 20,
            seed: IndicatorSeed::Sma,
        },
        IndicatorKind::Tema {
            period: 20,
            seed: IndicatorSeed::Sma,
        },
        IndicatorKind::Smma { period: 14 },
        IndicatorKind::Hma { period: 20 },
        IndicatorKind::Vwma { period: 20 },
        IndicatorKind::StandardDeviation { period: 20 },
        IndicatorKind::Cci { period: 20 },
        IndicatorKind::WilliamsR { period: 14 },
        IndicatorKind::StochasticRsi {
            rsi_period: 14,
            stochastic_period: 14,
        },
        IndicatorKind::Momentum { period: 10 },
        IndicatorKind::RateOfChange { period: 10 },
        IndicatorKind::Donchian { period: 20 },
        IndicatorKind::PivotPoints {
            variant: PivotKind::Standard,
        },
        IndicatorKind::ZigZag {
            deviation_percent: 5.0,
        },
        IndicatorKind::Keltner {
            period: 20,
            multiplier: 2.0,
        },
        IndicatorKind::AdxDmi { period: 14 },
        IndicatorKind::ParabolicSar,
        IndicatorKind::SuperTrend {
            period: 10,
            multiplier: 3.0,
        },
        IndicatorKind::Ichimoku,
        IndicatorKind::EmaRibbon {
            periods: [5, 10, 20, 50, 200],
        },
        IndicatorKind::Bollinger {
            period: 20,
            deviation: 2.0,
            estimator: DeviationEstimator::Population,
        },
        IndicatorKind::Rsi {
            period: 14,
            seed: IndicatorSeed::Sma,
        },
        IndicatorKind::Macd {
            fast: 12,
            slow: 26,
            signal: 9,
            seed: IndicatorSeed::Sma,
            histogram_multiplier: 1.0,
        },
        IndicatorKind::Stochastic {
            k_period: 14,
            d_period: 3,
        },
        IndicatorKind::Atr { period: 14 },
        IndicatorKind::Vwap,
        IndicatorKind::Obv,
        IndicatorKind::Cmf { period: 20 },
        IndicatorKind::Mfi { period: 14 },
        IndicatorKind::Volume { period: 20 },
        IndicatorKind::VwapBands {
            reset: VwapReset::Session,
            standard_deviation: 1.0,
            percent: 1.0,
        },
        IndicatorKind::Wma { period: 20 },
        IndicatorKind::Kdj {
            period: 9,
            k_smoothing: 3,
            d_smoothing: 3,
            seed: KdjSeed::Fifty,
        },
    ];
    let mut bindings = 0;
    for kind in kinds {
        let weighted = matches!(
            kind,
            IndicatorKind::Vwma { .. }
                | IndicatorKind::Vwap
                | IndicatorKind::VwapBands { .. }
                | IndicatorKind::Obv
                | IndicatorKind::Cmf { .. }
                | IndicatorKind::Mfi { .. }
                | IndicatorKind::Volume { .. }
        );
        let outputs = chart.add_indicator_kind(0, kind, weighted.then_some(volume));
        assert!(!outputs.is_empty(), "indicator binds");
        bindings += 1;
    }
    // Aggregate price inputs derive their scalar column from the source OHLC rows.
    for (input, kind) in [
        (
            IndicatorInputSource::Hlc3,
            IndicatorKind::Rsi {
                period: 14,
                seed: IndicatorSeed::Sma,
            },
        ),
        (
            IndicatorInputSource::Ohlc4,
            IndicatorKind::Sma { period: 20 },
        ),
        (
            IndicatorInputSource::Hl2,
            IndicatorKind::StochasticRsi {
                rsi_period: 14,
                stochastic_period: 14,
            },
        ),
    ] {
        assert!(!chart
            .add_indicator_kind_with_input(0, input, kind, None)
            .is_empty());
        bindings += 1;
    }
    bindings
}

/// Binds the 27 KLineChart templates with KLineChart's default parameters to the candle series 0:
/// the templates that read volume on `volume`, and AVP on the `turnover` series. Returns the
/// number of bindings.
fn bind_klinechart_templates(
    chart: &mut ChartEngine,
    volume: SeriesId,
    turnover: SeriesId,
) -> usize {
    use aeris_charts_engine::klinechart::{Indicator, NAMES};

    for name in NAMES {
        let indicator = Indicator::from_name(name).expect("every listed name is a template");
        let source = if matches!(indicator, Indicator::Avp) {
            turnover
        } else {
            0
        };
        let volume_source = indicator.needs_volume().then_some(volume);
        let outputs = chart.add_klinechart_indicator(source, indicator, volume_source);
        assert!(!outputs.is_empty(), "{name} binds");
    }
    NAMES.len()
}

/// Target M: bind a set of studies (every built-in study kind plus aggregate-input studies, or the
/// 27 KLineChart templates) to one `rows`-row minute candle source and its volume series (and, for
/// the KLineChart set, a turnover series), then time live ticks through the public engine path:
/// current-bar replacements, and appends in the usual candle-then-volume order. `whitespace_row`
/// makes one row of the history whitespace on every series.
fn indicator_tick_cost(
    studies: StudySet,
    rows: usize,
    slots: usize,
    whitespace_row: Option<usize>,
    replaces_per_append: usize,
    appends: usize,
) -> IndicatorTickCost {
    let volume_at = |row: usize, revision: usize| ((row * 37 + revision) % 900 + 100) as f64;
    // The turnover of a bar: its volume at its close.
    let turnover_at =
        |row: usize, revision: usize| volume_at(row, revision) * indicator_bar(row, revision)[3];
    // `slots` trailing whitespace rows model a pre-installed session: ticks then fill them in
    // place instead of appending.
    let mut times = Vec::with_capacity(rows + slots);
    let mut columns: [Vec<f64>; 4] = std::array::from_fn(|_| Vec::with_capacity(rows + slots));
    let mut volumes = Vec::with_capacity(rows + slots);
    let mut turnovers = Vec::with_capacity(rows + slots);
    for row in 0..rows + slots {
        times.push(row as f64 * 60.0);
        let (values, volume_value, turnover_value) = if row < rows && whitespace_row != Some(row) {
            (
                indicator_bar(row, 0),
                volume_at(row, 0),
                turnover_at(row, 0),
            )
        } else {
            ([f64::NAN; 4], f64::NAN, f64::NAN)
        };
        for (column, value) in columns.iter_mut().zip(values) {
            column.push(value);
        }
        volumes.push(volume_value);
        turnovers.push(turnover_value);
    }
    let mut chart = ChartEngine::new(1600.0, 800.0, 1.0);
    chart
        .set_series_data(
            0,
            &times,
            &columns[0],
            &columns[1],
            &columns[2],
            &columns[3],
        )
        .expect("valid indicator source");
    let volume = chart.add_series(SeriesKind::Histogram);
    chart
        .set_series_data(volume, &times, &volumes, &volumes, &volumes, &volumes)
        .expect("valid indicator volume");
    // AVP reads turnover as the value of a scalar source series.
    let turnover = (studies == StudySet::KLineChart).then(|| {
        let turnover = chart.add_series(SeriesKind::Line);
        chart
            .set_series_data(
                turnover, &times, &turnovers, &turnovers, &turnovers, &turnovers,
            )
            .expect("valid indicator turnover");
        turnover
    });
    let bindings = match studies {
        StudySet::BuiltIn => bind_builtin_studies(&mut chart, volume),
        StudySet::KLineChart => bind_klinechart_templates(
            &mut chart,
            volume,
            turnover.expect("the KLineChart set has a turnover series"),
        ),
    };

    // Writes `row` at `revision` to every series the studies read, in the usual live order: the
    // candle, then the volume, then (KLineChart set) the turnover.
    let write_row = |chart: &mut ChartEngine, row: usize, revision: usize| {
        let time = row as f64 * 60.0;
        chart.update_series_bar(0, time, indicator_bar(row, revision));
        chart.update_series_bar(volume, time, [volume_at(row, revision); 4]);
        if let Some(turnover) = turnover {
            chart.update_series_bar(turnover, time, [turnover_at(row, revision); 4]);
        }
    };

    // The first append after a bulk install grows every exact-capacity column the install created
    // (source, volume, and each study output) once. That amortized capacity growth is not per-tick
    // work, so it is reported separately and excluded from the tick statistics.
    let mut last = rows;
    let before = chart.memory_usage();
    let started = Instant::now();
    write_row(&mut chart, last, 0);
    let first_append_ms = started.elapsed().as_secs_f64() * 1000.0;
    let after = chart.memory_usage();
    let first_append_growth_bytes = (after.data.allocated_capacity_bytes
        + after.indicator_runtime_bytes)
        .saturating_sub(before.data.allocated_capacity_bytes + before.indicator_runtime_bytes);

    let mut replace_ms = Vec::new();
    let mut append_ms = Vec::new();
    let mut max_work_rows = 0;
    for _ in 0..appends {
        for revision in 1..=replaces_per_append {
            let started = Instant::now();
            write_row(&mut chart, last, revision);
            replace_ms.push(started.elapsed().as_secs_f64() * 1000.0);
            max_work_rows = max_work_rows.max(chart.last_indicator_work_rows());
        }
        last += 1;
        let started = Instant::now();
        write_row(&mut chart, last, 0);
        append_ms.push(started.elapsed().as_secs_f64() * 1000.0);
        max_work_rows = max_work_rows.max(chart.last_indicator_work_rows());
    }
    let runtime_bytes = chart.memory_usage().indicator_runtime_bytes;
    let summary = |mut samples: Vec<f64>| {
        samples.sort_by(f64::total_cmp);
        let mean = samples.iter().sum::<f64>() / samples.len() as f64;
        (mean, samples[samples.len() / 2], samples[samples.len() - 1])
    };
    // A correction `REPAIR_DEPTH` rows back replays from the nearest checkpoint to the newest row.
    let repair_ms = (studies == StudySet::KLineChart).then(|| {
        let row = last - REPAIR_DEPTH;
        summary(
            (1..=REPAIR_SAMPLES)
                .map(|revision| {
                    let started = Instant::now();
                    write_row(&mut chart, row, revision + replaces_per_append);
                    started.elapsed().as_secs_f64() * 1000.0
                })
                .collect(),
        )
    });
    IndicatorTickCost {
        bindings,
        replace_ms: summary(replace_ms),
        append_ms: summary(append_ms),
        first_append_ms,
        first_append_growth_bytes,
        max_work_rows,
        runtime_bytes,
        repair_ms,
    }
}

/// What Target M measures for studies on aggregate price inputs: the capacity their binding-private
/// price columns hold, as a difference of indicator runtime bytes against the same studies on a
/// canonical input.
struct CompositeInputCost {
    /// Composite minus canonical runtime bytes after the install, after the first append, and
    /// after the remaining appends.
    install_bytes: usize,
    first_append_bytes: usize,
    final_bytes: usize,
    /// First append after the install for the canonical and the composite chart (context only).
    first_append_ms: (f64, f64),
    /// Mean milliseconds per append after the first, on the composite chart.
    append_mean_ms: f64,
}

/// Target M (composite inputs): four studies on `rows` minute candles with no volume series, once
/// each on Close and once each on Hl2, Hlc3, Ohlc4 and Hlcc4. The two charts carry the same runtime
/// state except the aggregate price columns, so the byte difference is exactly those columns and
/// is deterministic where first-append time on a shared machine is not. Charts are built and
/// dropped one at a time to bound peak memory.
fn composite_input_cost(rows: usize, appends: usize) -> CompositeInputCost {
    use aeris_charts_engine::{IndicatorInputSource, IndicatorKind, IndicatorSeed};

    let run = |inputs: [IndicatorInputSource; 4]| {
        let mut times = Vec::with_capacity(rows);
        let mut columns: [Vec<f64>; 4] = std::array::from_fn(|_| Vec::with_capacity(rows));
        for row in 0..rows {
            times.push(row as f64 * 60.0);
            for (column, value) in columns.iter_mut().zip(indicator_bar(row, 0)) {
                column.push(value);
            }
        }
        let mut chart = ChartEngine::new(1600.0, 800.0, 1.0);
        chart
            .set_series_data(
                0,
                &times,
                &columns[0],
                &columns[1],
                &columns[2],
                &columns[3],
            )
            .expect("valid indicator source");
        let kinds = [
            IndicatorKind::Sma { period: 20 },
            IndicatorKind::Ema {
                period: 20,
                seed: IndicatorSeed::Sma,
            },
            IndicatorKind::Rsi {
                period: 14,
                seed: IndicatorSeed::Sma,
            },
            IndicatorKind::StochasticRsi {
                rsi_period: 14,
                stochastic_period: 14,
            },
        ];
        for (kind, input) in kinds.into_iter().zip(inputs) {
            assert!(!chart
                .add_indicator_kind_with_input(0, input, kind, None)
                .is_empty());
        }
        let runtime_bytes = |chart: &ChartEngine| chart.memory_usage().indicator_runtime_bytes;
        let install = runtime_bytes(&chart);
        let started = Instant::now();
        chart.update_series_bar(0, rows as f64 * 60.0, indicator_bar(rows, 0));
        let first_append_ms = started.elapsed().as_secs_f64() * 1000.0;
        let first_append = runtime_bytes(&chart);
        let mut append_ms = 0.0;
        for row in rows + 1..=rows + appends {
            let started = Instant::now();
            chart.update_series_bar(0, row as f64 * 60.0, indicator_bar(row, 0));
            append_ms += started.elapsed().as_secs_f64() * 1000.0;
        }
        (
            [install, first_append, runtime_bytes(&chart)],
            first_append_ms,
            append_ms / appends as f64,
        )
    };
    let (canonical, canonical_first_ms, _) = run([IndicatorInputSource::Close; 4]);
    let (composite, composite_first_ms, append_mean_ms) = run([
        IndicatorInputSource::Hl2,
        IndicatorInputSource::Hlc3,
        IndicatorInputSource::Ohlc4,
        IndicatorInputSource::Hlcc4,
    ]);
    let delta = |index: usize| composite[index].saturating_sub(canonical[index]);
    CompositeInputCost {
        install_bytes: delta(0),
        first_append_bytes: delta(1),
        final_bytes: delta(2),
        first_append_ms: (canonical_first_ms, composite_first_ms),
        append_mean_ms,
    }
}

/// Target N: `rows` minute candles with `regressions` regression trends anchored from the first
/// bar to the latest (one reaching into the future, so appends enter it), the view on the latest
/// 100 bars, then live ticks (current-bar replacements and appends) each followed by one frame.
/// Returns `(mean, median, max)` milliseconds per tick plus frame.
fn regression_tick_cost(rows: usize, regressions: usize, ticks: usize) -> (f64, f64, f64) {
    use aeris_charts_engine::{DrawingKind, DrawingPoint};

    let bar = |row: usize, revision: usize| {
        let close = 100.0 + (row as f64 * 0.0007).sin() * 12.0 + revision as f64 * 0.01;
        [close - 0.2, close + 0.4, close - 0.5, close]
    };
    let times = (0..rows).map(|row| row as f64 * 60.0).collect::<Vec<_>>();
    let columns: [Vec<f64>; 4] =
        std::array::from_fn(|column| (0..rows).map(|row| bar(row, 0)[column]).collect());
    let mut chart = ChartEngine::new(1600.0, 800.0, 1.0);
    chart
        .set_series_data(
            0,
            &times,
            &columns[0],
            &columns[1],
            &columns[2],
            &columns[3],
        )
        .expect("valid regression source");
    for index in 0..regressions {
        let second = if index == 0 {
            rows as f64 + 1_000.0
        } else {
            rows as f64 - 1.0
        };
        chart
            .add_drawing(
                DrawingKind::RegressionTrend,
                0,
                vec![
                    DrawingPoint {
                        logical: index as f64,
                        price: 100.0,
                    },
                    DrawingPoint {
                        logical: second,
                        price: 100.0,
                    },
                ],
                None,
            )
            .expect("regression trend");
    }
    chart.set_visible_logical_range(rows as f64 - 100.0, rows as f64);
    let mut frame = ChartFrame::default();
    chart.build_frame_into(&mut frame);
    let mut samples = Vec::with_capacity(ticks);
    let mut last = rows - 1;
    for tick in 0..ticks {
        // Four replacements of the forming bar, then the next bar.
        if tick % 5 == 4 {
            last += 1;
        }
        let started = Instant::now();
        chart.update_series_bar(0, last as f64 * 60.0, bar(last, tick % 5));
        chart.build_frame_into(&mut frame);
        samples.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    samples.sort_by(f64::total_cmp);
    let mean = samples.iter().sum::<f64>() / samples.len() as f64;
    (mean, samples[samples.len() / 2], samples[samples.len() - 1])
}

/// Counts every Canvas2D call one frame issues (the wasm host pays a JS call for each) and the
/// path-building calls separately.
#[derive(Default)]
struct CallCounter {
    calls: usize,
    strokes: usize,
}

impl Canvas2d for CallCounter {
    fn set_fill_solid(&mut self, _: Color) {
        self.calls += 1;
    }
    fn set_fill_vgradient(&mut self, _: f32, _: f32, _: Color, _: Color) {
        self.calls += 1;
    }
    fn set_stroke(&mut self, _: Color) {
        self.calls += 1;
    }
    fn set_line_width(&mut self, _: f32) {
        self.calls += 1;
    }
    fn set_line_dash(&mut self, _: &[f32]) {
        self.calls += 1;
    }
    fn fill_rect(&mut self, _: f32, _: f32, _: f32, _: f32) {
        self.calls += 1;
    }
    fn begin_path(&mut self) {
        self.calls += 1;
    }
    fn move_to(&mut self, _: f32, _: f32) {
        self.calls += 1;
    }
    fn line_to(&mut self, _: f32, _: f32) {
        self.calls += 1;
    }
    fn close_path(&mut self) {
        self.calls += 1;
    }
    fn arc(&mut self, _: f32, _: f32, _: f32, _: f32, _: f32) {
        self.calls += 1;
    }
    fn stroke(&mut self) {
        self.calls += 1;
        self.strokes += 1;
    }
    fn fill(&mut self) {
        self.calls += 1;
    }
    fn fill_rotated_text(
        &mut self,
        _: &str,
        _: f32,
        _: f32,
        _: &str,
        _: Color,
        _: aeris_charts_render::draw_list::TextAlign,
        _: f32,
    ) {
        self.calls += 1;
    }
}

/// Every layer of `frame` in paint order (under, main, top) with its pane's point pool.
fn for_each_layer(frame: &ChartFrame, mut visit: impl FnMut(&[Prim], &[[f32; 2]])) {
    for pane in &frame.panes {
        for layer in [&pane.under, &pane.main, &pane.top_prims] {
            visit(layer, &pane.points);
        }
    }
}

/// What Target O measures for one daily-bar chart.
struct DailyStudyCost {
    outputs: usize,
    prims: usize,
    lone_polylines: usize,
    batches: usize,
    batched_pairs: usize,
    pool_points: usize,
    rebuild_ms: f64,
    retained_ms: f64,
    canvas_calls: usize,
    canvas_strokes: usize,
    canvas_counting_ms: f64,
    raster_ms: f64,
    group_ms: f64,
    hit_ms: f64,
}

/// Target O: `rows` daily candles on a 1600 px chart, fit, optionally with the session-reset studies
/// that draw one bar-wide segment per bar (session VWAP, VWAP bands, standard pivots). Times one
/// full series rebuild, one crosshair-only (retained) frame, the Canvas2D call stream and its
/// tiny-skia rasterization, WebGPU group scheduling, and one pointer hover arbitration.
fn daily_reset_study_cost(rows: usize, min_bar_spacing: f64, studies: bool) -> DailyStudyCost {
    use aeris_charts_engine::{IndicatorKind, PivotKind, VwapReset};

    const DAY: f64 = 86_400.0;
    const START: f64 = 1_789_948_800.0;
    let times = (0..rows)
        .map(|row| START + row as f64 * DAY)
        .collect::<Vec<_>>();
    let columns: [Vec<f64>; 4] =
        std::array::from_fn(|column| (0..rows).map(|row| indicator_bar(row, 0)[column]).collect());
    let mut chart = ChartEngine::new(1600.0, 800.0, 1.0);
    chart
        .set_series_data(
            0,
            &times,
            &columns[0],
            &columns[1],
            &columns[2],
            &columns[3],
        )
        .expect("valid daily source");
    let mut outputs = 0;
    if studies {
        let volume = chart.add_series(SeriesKind::Histogram);
        let volumes = (0..rows)
            .map(|row| ((row * 37) % 900 + 100) as f64)
            .collect::<Vec<_>>();
        chart
            .set_series_data(volume, &times, &volumes, &volumes, &volumes, &volumes)
            .expect("valid daily volume");
        chart.set_series_visible(volume, false);
        for (kind, weighted) in [
            (IndicatorKind::Vwap, true),
            (
                IndicatorKind::VwapBands {
                    reset: VwapReset::Session,
                    standard_deviation: 1.0,
                    percent: 1.0,
                },
                true,
            ),
            (
                IndicatorKind::PivotPoints {
                    variant: PivotKind::Standard,
                },
                false,
            ),
        ] {
            outputs += chart
                .add_indicator_kind(0, kind, weighted.then_some(volume))
                .len();
        }
    }
    chart.time_scale.set_width(1600.0);
    chart.set_min_bar_spacing(min_bar_spacing);
    chart.fit_content();
    let mut frame = ChartFrame::default();
    chart.build_frame_into(&mut frame);

    // One full series rebuild: alternate the view by one bar so every coordinate changes.
    let time_of_last = times[rows - 1];
    let ranges = [(-0.5, rows as f64 - 0.5), (-1.5, rows as f64 - 1.5)];
    const REBUILDS: usize = 20;
    let mut samples = Vec::with_capacity(REBUILDS);
    for index in 0..REBUILDS {
        let (from, to) = ranges[index % 2];
        chart.set_visible_logical_range(from, to);
        let started = Instant::now();
        chart.build_frame_into(&mut frame);
        samples.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    samples.sort_by(f64::total_cmp);
    let rebuild_ms = samples[samples.len() / 2];
    chart.set_visible_logical_range(ranges[0].0, ranges[0].1);
    chart.build_frame_into(&mut frame);

    // Crosshair-only frames: the retained series layer is re-assembled, never rebuilt.
    const CURSOR_FRAMES: usize = 100;
    let mut samples = Vec::with_capacity(CURSOR_FRAMES);
    for index in 0..CURSOR_FRAMES {
        let time = time_of_last - (index % 2) as f64 * DAY;
        chart.set_crosshair_position(100.0, time, 0);
        let started = Instant::now();
        chart.build_frame_into(&mut frame);
        samples.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    samples.sort_by(f64::total_cmp);
    let retained_ms = samples[samples.len() / 2];

    let mut prims = 0;
    let mut lone_polylines = 0;
    let mut batches = 0;
    let mut batched_pairs = 0;
    let mut pool_points = 0;
    for pane in &frame.panes {
        pool_points += pane.points.len();
        for prim in pane.under.iter().chain(&pane.main).chain(&pane.top_prims) {
            prims += 1;
            match prim {
                Prim::Polyline { point_count: 2, .. } => lone_polylines += 1,
                Prim::Segments { segment_count, .. } => {
                    batches += 1;
                    batched_pairs += *segment_count as usize;
                }
                _ => {}
            }
        }
    }

    let viewport = Viewport {
        width: 1600.0,
        height: 800.0,
    };
    let mut counter = CallCounter::default();
    for_each_layer(&frame, |prims, points| {
        execute(prims, points, &mut counter, viewport);
    });
    const EXECUTIONS: usize = 20;
    let started = Instant::now();
    for _ in 0..EXECUTIONS {
        let mut counter = CallCounter::default();
        for_each_layer(&frame, |prims, points| {
            execute(prims, points, &mut counter, viewport);
        });
        std::hint::black_box(counter.calls);
    }
    let canvas_counting_ms = started.elapsed().as_secs_f64() * 1000.0 / EXECUTIONS as f64;

    // The same stream into a real rasterizer (tiny-skia): a CPU proxy for the browser's 2D stroker,
    // not a Chromium measurement.
    const RASTERS: usize = 5;
    let started = Instant::now();
    for _ in 0..RASTERS {
        for_each_layer(&frame, |prims, points| {
            std::hint::black_box(render_prims(1600, 800, Color::rgb(0, 0, 0), prims, points));
        });
    }
    let raster_ms = started.elapsed().as_secs_f64() * 1000.0 / RASTERS as f64;

    const GROUPS: usize = 20;
    let started = Instant::now();
    for _ in 0..GROUPS {
        let mut group = DrawGroup::default();
        for_each_layer(&frame, |prims, points| {
            prims_to_group(
                prims,
                points,
                &mut group,
                &mut |_: &Prim| None::<TexQuadInstance>,
                &mut |_: &Prim| None::<TexQuadInstance>,
            );
        });
        std::hint::black_box(group.tris.len());
    }
    let group_ms = started.elapsed().as_secs_f64() * 1000.0 / GROUPS as f64;

    const HOVERS: usize = 200;
    let started = Instant::now();
    for index in 0..HOVERS {
        let x = 20.0 + index as f64 * (1500.0 / HOVERS as f64);
        std::hint::black_box(chart.hit_test_series(x, 200.0));
    }
    let hit_ms = started.elapsed().as_secs_f64() * 1000.0 / HOVERS as f64;

    DailyStudyCost {
        outputs,
        prims,
        lone_polylines,
        batches,
        batched_pairs,
        pool_points,
        rebuild_ms,
        retained_ms,
        canvas_calls: counter.calls,
        canvas_strokes: counter.strokes,
        canvas_counting_ms,
        raster_ms,
        group_ms,
        hit_ms,
    }
}

/// Target D2 (report-only): the data layer's share of one retention trim, across the series count
/// `S` and the retained rows `N` that shape it together. One retention drops `evicted` rows from
/// each of the `S` presentations of a stream, which share one timestamp union. Each cell times
/// one `trim_fronts` against `S` separate `trim_front` calls on identical layers of `S` aligned
/// OHLC series, and counts the union merges and reindexes each ran. No threshold: the trim still
/// scales with the retained rows.
// ponytail: add a threshold with the O(evicted) trim `DataLayer::trim_fronts` defers; the bar is
// about 2 ms in the S >= 4, N = 28,800 cells.
fn retention_trim_matrix() {
    const EVICTED: usize = 80;
    const RUNS: usize = 7;
    let layer = |series: usize, rows: usize| {
        let (times, open, high, low, close) = gen_series(rows, 0.0);
        let times = times.iter().map(|&time| time as i64).collect::<Vec<_>>();
        let mut data = DataLayer::new();
        let ids = (0..series)
            .map(|_| {
                let id = data.add_series();
                assert!(data.set_data(
                    id,
                    times.clone(),
                    open.clone(),
                    high.clone(),
                    low.clone(),
                    close.clone()
                ));
                id
            })
            .collect::<Vec<_>>();
        (data, ids)
    };
    let median = |samples: &mut Vec<f64>| {
        samples.sort_by(f64::total_cmp);
        samples[samples.len() / 2]
    };
    println!("Target D2 — retention trim of the data layer, {EVICTED} rows evicted per series (report-only):");
    for series in [1, 4, 8] {
        for rows in [2_500, 10_000, 28_800, 40_000] {
            let mut sequential_ms = Vec::with_capacity(RUNS);
            let mut batched_ms = Vec::with_capacity(RUNS);
            let (mut sequential_passes, mut batched_passes) = (0, 0);
            for _ in 0..RUNS {
                let (mut data, ids) = layer(series, rows);
                let passes = data.index_rebuilds();
                let start = Instant::now();
                for &id in &ids {
                    data.trim_front(id, rows - EVICTED);
                }
                sequential_ms.push(start.elapsed().as_secs_f64() * 1000.0);
                sequential_passes = data.index_rebuilds() - passes;

                let (mut data, ids) = layer(series, rows);
                let trims = ids
                    .iter()
                    .map(|&id| (id, rows - EVICTED))
                    .collect::<Vec<_>>();
                let passes = data.index_rebuilds();
                let start = Instant::now();
                data.trim_fronts(&trims);
                batched_ms.push(start.elapsed().as_secs_f64() * 1000.0);
                batched_passes = data.index_rebuilds() - passes;
            }
            println!(
                "  S={series} N={rows:>6}: trim_front x S {:>7.3} ms ({sequential_passes:>2} union passes) | trim_fronts {:>7.3} ms ({batched_passes} union passes)",
                median(&mut sequential_ms),
                median(&mut batched_ms),
            );
        }
    }
}

/// Whether `AERIS_CHARTS_PERF_STRICT` asks for a non-zero exit on a failed target: exactly `1`,
/// the browser perf specs' parse, so `AERIS_CHARTS_PERF_STRICT=0` turns enforcement off.
fn strict_requested(value: Option<&str>) -> bool {
    value == Some("1")
}

/// The process status of a finished run: `main` returns it, so a failed target exits non-zero only
/// when strict mode is requested and every other run (all targets passing, unset or `0`) exits zero.
fn gate_exit(all_pass: bool, strict_value: Option<&str>) -> ExitCode {
    if !all_pass && strict_requested(strict_value) {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn main() -> ExitCode {
    const SERIES: usize = 10;
    const FRAME_BARS: usize = 50_000;
    const FRAME_BUDGET_MS: f64 = 1000.0 / 60.0;
    const FRAMES: usize = 60;
    const LOAD_BARS: usize = 1_000_000;
    const LOAD_BUDGET_MS: f64 = 300.0;
    const INPUT_SAMPLES: usize = 1_000_000;
    const INPUT_SAMPLE_BUDGET_MS: f64 = 0.01;
    const FOOTPRINT_HISTORY_BARS: usize = 2_500;
    const FOOTPRINT_TRADES_PER_BAR: usize = 100;
    const FOOTPRINT_LOAD_BUDGET_MS: f64 = 300.0;
    const FOOTPRINT_LIVE_BARS: usize = 100;
    const FOOTPRINT_LIVE_BUDGET_MS: f64 = 50.0;
    const FOOTPRINT_CORRECTION_BARS: usize = 10;
    const FOOTPRINT_CORRECTION_BUDGET_MS: f64 = 300.0;
    // Enough single-trade tips to cross the 2,500-bar retention ceiling's 78-bar hysteresis once.
    const FOOTPRINT_TIP_BARS: usize = 90;
    const FOOTPRINT_TIP_P99_BUDGET_MS: f64 = 0.25;
    const GENERAL_LINE_POINTS: usize = 100_000;
    const GENERAL_LINE_HIT_SAMPLES: usize = 100;
    const GENERAL_LINE_HIT_BUDGET_MS: f64 = 8.0;
    const GENERAL_MIX_POINTS_PER_SERIES: usize = 20_000;
    const GENERAL_MIX_MEMORY_BUDGET_BYTES: usize = 12 * 1024 * 1024;
    const COMBINED_FINANCIAL_BARS: usize = 50_000;
    const COMBINED_GENERAL_POINTS: usize = 50_000;
    const COMBINED_MEMORY_BUDGET_BYTES: usize = 16 * 1024 * 1024;
    const ERROR_BAR_POINTS: usize = 100_000;
    const ERROR_BAR_MEMORY_BUDGET_BYTES: usize = 16 * 1024 * 1024;
    const REPLAY_SPEED: usize = 100;
    const REPLAY_SECONDS: usize = 6_000;
    const REPLAY_FRAMES: usize = REPLAY_SECONDS / REPLAY_SPEED;
    const DEPTH_SOAK_UPDATES: usize = 1_200_000;
    const DEPTH_BATCH_UPDATES: usize = 100_000;
    const DEPTH_BATCH_BUDGET_MS: f64 = 150.0;
    const DEPTH_UPLOAD_BUDGET_BYTES: usize = 4_096 * 4;
    const INDICATOR_TICK_ROWS: usize = 1_000_000;
    const INDICATOR_TICK_APPENDS: usize = 60;
    const INDICATOR_TICK_BUDGET_MS: f64 = 1.0;
    // Target M (KLineChart): a tick evaluates one row per binding (two when a weight column is
    // realigned), so four per binding leaves headroom without admitting a replayed window.
    const KLINECHART_WORK_ROWS_PER_BINDING: usize = 4;
    const KLINECHART_RUNTIME_BUDGET_BYTES: usize = 8 * 1024 * 1024;
    const KLINECHART_REPAIR_BUDGET_MS: f64 = 5.0;

    println!("aeris_charts perf gate (release build recommended)\n");

    // ---- Target A: 60fps @ 10 series x 50k bars ---------------------------------------------
    let mut chart = ChartEngine::new(1600.0, 800.0, 1.0);
    // series[0] exists at construction; add the remaining nine on the shared time axis.
    let mut ids = vec![0u32];
    for _ in 1..SERIES {
        ids.push(chart.add_series(SeriesKind::Candlestick));
    }
    for (n, &id) in ids.iter().enumerate() {
        let (t, o, h, l, c) = gen_series(FRAME_BARS, n as f64 * 3.0);
        chart
            .set_series_data(id, &t, &o, &h, &l, &c)
            .expect("valid series fixture");
    }
    chart.time_scale.set_width(1600.0);
    chart.fit_content();

    let mut frame = ChartFrame::default();
    chart.build_frame_into(&mut frame); // warm up buffers + caches
    let start = Instant::now();
    for _ in 0..FRAMES {
        chart.build_frame_into(&mut frame);
    }
    let per_frame_ms = start.elapsed().as_secs_f64() * 1000.0 / FRAMES as f64;
    println!("Target A — 60fps @ {SERIES} series x {FRAME_BARS} bars:");
    let a_pass = report("build_frame", per_frame_ms, FRAME_BUDGET_MS);

    // ---- Target B: 1M-bar load under 300 ms -------------------------------------------------
    let (t, o, h, l, c) = gen_series(LOAD_BARS, 0.0);
    let mut load_chart = ChartEngine::new(1600.0, 800.0, 1.0);
    let start = Instant::now();
    load_chart
        .set_series_data(0, &t, &o, &h, &l, &c)
        .expect("valid load fixture");
    let load_ms = start.elapsed().as_secs_f64() * 1000.0;
    println!("Target B — {LOAD_BARS} bar load:");
    let b_pass = report("set_series_data", load_ms, LOAD_BUDGET_MS);

    // ---- Target C: allocation-free canonical pointer resolver -------------------------------
    // GestureResolver contains only a two-slot inline pointer array and scalar state: the move
    // loop has no heap owner or capacity growth path. Measure the release-mode sample latency.
    let mut input = GestureResolver::default();
    let mut sample = PointerSample {
        id: 1,
        device: InputDevice::Touch,
        target: InputTarget::Pane,
        modifiers: Default::default(),
        x: 100.0,
        y: 100.0,
        timestamp_ms: 0.0,
        pressure: 0.5,
        tilt_x: 0.0,
        tilt_y: 0.0,
    };
    input.pointer_down(sample);
    let start = Instant::now();
    for index in 0..INPUT_SAMPLES {
        sample.x = 100.0 + (index & 63) as f64;
        sample.timestamp_ms = index as f64;
        std::hint::black_box(input.pointer_move(sample));
    }
    let per_sample_ms = start.elapsed().as_secs_f64() * 1000.0 / INPUT_SAMPLES as f64;
    println!(
        "Target C — {INPUT_SAMPLES} canonical pointer samples ({}-byte fixed resolver):",
        std::mem::size_of::<GestureResolver>()
    );
    let c_pass = report("pointer_move", per_sample_ms, INPUT_SAMPLE_BUDGET_MS);

    // ---- Target D: tick-truth footprint history + one synchronized live batch ----------------
    let mut footprint = ChartEngine::new(1600.0, 800.0, 1.0);
    footprint
        .configure_footprint_series(
            0,
            FootprintSeriesOptions {
                aggregation: FootprintAggregationOptions {
                    tick_size: 0.25,
                    ticks_per_row: 1,
                    bars: FootprintBarAggregation::Trades {
                        trades_per_bar: FOOTPRINT_TRADES_PER_BAR as u32,
                    },
                    ..FootprintAggregationOptions::default()
                },
                visual: FootprintVisualOptions::default(),
            },
        )
        .expect("valid footprint options");
    let footprint_stream = footprint
        .add_trade_stream(
            "PERF:ES",
            FootprintAggregationOptions {
                tick_size: 0.25,
                ticks_per_row: 1,
                bars: FootprintBarAggregation::Trades {
                    trades_per_bar: FOOTPRINT_TRADES_PER_BAR as u32,
                },
                ..FootprintAggregationOptions::default()
            },
        )
        .expect("valid shared footprint stream");
    footprint
        .bind_footprint_series_to_stream(0, footprint_stream)
        .expect("bind footprint stream");
    let trade_candles = footprint.add_series(SeriesKind::Candlestick);
    footprint
        .bind_trade_bar_series_to_stream(trade_candles, footprint_stream)
        .expect("bind trade candle stream");
    let _cvd = footprint
        .add_cvd_series(footprint_stream, 1, TradeStudyOptions::default())
        .expect("add CVD dependent");
    let _delta = footprint
        .add_delta_series(footprint_stream, 1)
        .expect("add delta dependent");
    footprint
        .add_trade_bubbles(
            footprint_stream,
            0,
            TradeBubbleOptions {
                minimum_volume: 10.0,
                max_markers: 2_048,
                aggregation_window_micros: 0,
            },
        )
        .expect("add bubble dependent");
    let history = gen_footprint_trades(0, FOOTPRINT_HISTORY_BARS, FOOTPRINT_TRADES_PER_BAR);
    let start = Instant::now();
    footprint
        .set_footprint_trades(0, history)
        .expect("valid footprint history");
    let footprint_load_ms = start.elapsed().as_secs_f64() * 1000.0;
    assert!(footprint.set_series_max_points(0, Some(FOOTPRINT_HISTORY_BARS)));
    let footprint_memory_before = footprint.memory_usage().footprint_capacity_bytes;
    footprint.time_scale.set_width(1600.0);
    footprint.fit_content();
    let mut footprint_frame = ChartFrame::default();
    let start = Instant::now();
    footprint.build_frame_into(&mut footprint_frame);
    let footprint_frame_ms = start.elapsed().as_secs_f64() * 1000.0;
    let live = gen_footprint_trades(
        FOOTPRINT_HISTORY_BARS,
        FOOTPRINT_LIVE_BARS,
        FOOTPRINT_TRADES_PER_BAR,
    );
    let start = Instant::now();
    footprint
        .update_footprint_trades(0, live)
        .expect("valid footprint live batch");
    let footprint_live_ms = start.elapsed().as_secs_f64() * 1000.0;
    let footprint_stats = footprint
        .footprint_work_stats(0)
        .expect("footprint work stats");
    let mut corrections = gen_footprint_trades(
        FOOTPRINT_HISTORY_BARS + FOOTPRINT_LIVE_BARS - FOOTPRINT_CORRECTION_BARS,
        FOOTPRINT_CORRECTION_BARS,
        FOOTPRINT_TRADES_PER_BAR,
    );
    for trade in &mut corrections {
        trade.volume += 1.0;
    }
    let before_correction = footprint_stats;
    let start = Instant::now();
    footprint
        .update_footprint_trades(0, corrections)
        .expect("valid footprint correction batch");
    let footprint_correction_ms = start.elapsed().as_secs_f64() * 1000.0;
    let after_correction = footprint
        .footprint_work_stats(0)
        .expect("footprint work stats after correction");
    let correction_rebuilds =
        after_correction.historical_rebuilds - before_correction.historical_rebuilds;
    let correction_rebuilt_ticks = after_correction.rebuilt_ticks - before_correction.rebuilt_ticks;
    assert_eq!(
        correction_rebuilds, 1,
        "one correction batch must reconstruct once"
    );
    let footprint_retained_bars = footprint.footprint_bars(0).expect("footprint bars").len();
    let footprint_memory_after = footprint.memory_usage().footprint_capacity_bytes;
    let footprint_stream_stats = footprint
        .trade_stream_stats(footprint_stream)
        .expect("shared footprint stream stats");
    assert_eq!(footprint_stream_stats.dependent_count, 4);
    println!(
        "Target D — {} footprint trades / {} bars + {}-trade live batch ({} incremental ticks, {} historical rebuilds, {} dependent incremental updates, {} retained bars, {:.2}/{:.2} MiB footprint capacity):",
        FOOTPRINT_HISTORY_BARS * FOOTPRINT_TRADES_PER_BAR,
        FOOTPRINT_HISTORY_BARS,
        FOOTPRINT_LIVE_BARS * FOOTPRINT_TRADES_PER_BAR,
        footprint_stats.incremental_ticks,
        footprint_stats.historical_rebuilds,
        footprint_stream_stats.dependent_incremental_updates,
        footprint_retained_bars,
        footprint_memory_before as f64 / (1024.0 * 1024.0),
        footprint_memory_after as f64 / (1024.0 * 1024.0),
    );
    let d_load_pass = report(
        "set_footprint_trades",
        footprint_load_ms,
        FOOTPRINT_LOAD_BUDGET_MS,
    );
    let d_live_pass = report(
        "update_footprint_trades",
        footprint_live_ms,
        FOOTPRINT_LIVE_BUDGET_MS,
    );
    let d_correction_pass = report(
        &format!(
            "correct_footprint_trades ({} trades, {correction_rebuilds} rebuild, {correction_rebuilt_ticks} rebuilt ticks)",
            FOOTPRINT_CORRECTION_BARS * FOOTPRINT_TRADES_PER_BAR,
        ),
        footprint_correction_ms,
        FOOTPRINT_CORRECTION_BUDGET_MS,
    );
    let d_frame_pass = report("footprint build_frame", footprint_frame_ms, FRAME_BUDGET_MS);
    let d_retention_pass = footprint_retained_bars <= FOOTPRINT_HISTORY_BARS;
    println!(
        "  {:<24} {:>8}",
        "retention ceiling",
        if d_retention_pass { "PASS" } else { "FAIL" }
    );

    // Sustained single-trade live tips on the retained chart. Each tip must advance the footprint,
    // bound candles, CVD, delta, and bubble dependents by the changed bar suffix and the new trade
    // only. The tip crossing the retention ceiling also evicts the leading bars, their trades, and
    // their bubbles in place, without reconstructing the retained tape or refolding bubbles.
    let tip_trades = gen_footprint_trades(
        FOOTPRINT_HISTORY_BARS + FOOTPRINT_LIVE_BARS,
        FOOTPRINT_TIP_BARS,
        FOOTPRINT_TRADES_PER_BAR,
    );
    let tip_count = tip_trades.len() as u64;
    let first_row_key = |chart: &ChartEngine| {
        chart
            .data_layer()
            .series_data(0)
            .and_then(|(times, _)| times.first().copied())
    };
    let tip_work_before = footprint
        .trade_stream_stats(footprint_stream)
        .expect("footprint stream stats before tips");
    let tip_rebuilds_before = footprint
        .footprint_work_stats(0)
        .expect("footprint work stats before tips")
        .historical_rebuilds;
    let mut tip_samples = Vec::with_capacity(tip_trades.len());
    let mut tip_trims = 0u64;
    // The retention trim tip: its time, and the timestamp-union rebuild and reindex passes the
    // whole data layer ran for it (one union merge and one reindex for every presentation).
    let mut trim_tip_ms = 0.0f64;
    let mut trim_tip_passes = Vec::new();
    for trade in tip_trades {
        let first_key = first_row_key(&footprint);
        let passes_before = footprint.data_layer().index_rebuilds();
        let start = Instant::now();
        footprint
            .update_footprint_trades(0, vec![trade])
            .expect("valid footprint tip");
        let tip_ms = start.elapsed().as_secs_f64() * 1000.0;
        tip_samples.push(tip_ms);
        if first_row_key(&footprint) != first_key {
            tip_trims += 1;
            trim_tip_ms = trim_tip_ms.max(tip_ms);
            trim_tip_passes.push(footprint.data_layer().index_rebuilds() - passes_before);
        }
    }
    let tip_work = footprint
        .trade_stream_stats(footprint_stream)
        .expect("footprint stream stats after tips");
    let tip_rebuilds = footprint
        .footprint_work_stats(0)
        .expect("footprint work stats after tips")
        .historical_rebuilds
        - tip_rebuilds_before;
    tip_samples.sort_by(f64::total_cmp);
    let tip_percentile =
        |p: f64| tip_samples[((tip_samples.len() - 1) as f64 * p).round() as usize];
    let tip_study_rows = tip_work.dependent_rows_computed - tip_work_before.dependent_rows_computed;
    let tip_bar_rows = tip_work.bar_rows_projected - tip_work_before.bar_rows_projected;
    let tip_bubble_trades = tip_work.bubble_trades_scanned - tip_work_before.bubble_trades_scanned;
    let tip_bubble_sizes = tip_work.bubble_markers_sized - tip_work_before.bubble_markers_sized;
    println!(
        "  live tips: {tip_count} single-trade tips, {tip_trims} retention trim(s) (slowest {trim_tip_ms:.2} ms, {trim_tip_passes:?} union passes), {tip_rebuilds} tape reconstruction(s); p50 {:.4} ms, max {:.2} ms; per tip {:.2} study rows, {:.2} bar rows, {:.2} bubble trades, {:.2} bubble sizes",
        tip_percentile(0.5),
        tip_percentile(1.0),
        tip_study_rows as f64 / tip_count as f64,
        tip_bar_rows as f64 / tip_count as f64,
        tip_bubble_trades as f64 / tip_count as f64,
        tip_bubble_sizes as f64 / tip_count as f64,
    );
    // CVD and delta each recompute the changed suffix (the active bar, plus the bar a tip opens);
    // the footprint and bound candles project the same suffix; bubbles fold each new trade exactly
    // once, also across the retention trim, and nothing reconstructs the retained tape. A trim
    // runs one union merge and one reindex for all of its presentations, however many there are.
    let d_tip_work_pass = tip_study_rows <= 2 * (tip_count + FOOTPRINT_TIP_BARS as u64)
        && tip_bar_rows <= 2 * (tip_count + FOOTPRINT_TIP_BARS as u64)
        && tip_bubble_trades == tip_count
        && tip_rebuilds == 0
        && tip_trims >= 1
        && trim_tip_passes.iter().all(|&passes| passes == 2);
    println!(
        "  [{}] tip work bounded by the changed suffix and the new trade",
        if d_tip_work_pass { "PASS" } else { "FAIL" }
    );
    let d_tip_pass = report(
        "footprint live tip p99",
        tip_percentile(0.99),
        FOOTPRINT_TIP_P99_BUDGET_MS,
    );
    // The slowest tip is the one that crosses the retention ceiling; it must still fit a frame.
    let d_tip_max_pass = report(
        "footprint live tip max",
        tip_percentile(1.0),
        FRAME_BUDGET_MS,
    );
    retention_trim_matrix();
    let d_pass = d_load_pass
        && d_live_pass
        && d_correction_pass
        && d_frame_pass
        && d_retention_pass
        && d_tip_work_pass
        && d_tip_pass
        && d_tip_max_pass;

    let (dense_text_prims, dense_text_instances, dense_wgpu_p99_ms) =
        dense_footprint_wgpu_scene_ms();
    println!(
        "Target J — dense footprint WebGPU frame encoding ({} text prims, {} atlas instances):",
        dense_text_prims, dense_text_instances
    );
    let j_text_count = dense_text_prims == dense_text_instances;
    println!(
        "  [{}] text-run scheduling preserves all resolved runs",
        if j_text_count { "PASS" } else { "FAIL" }
    );
    let j_scene = report(
        "WebGPU dense text frame encoding p99",
        dense_wgpu_p99_ms,
        2.0,
    );

    // Profile work is measured through the real frame path, including timestamp matching.
    let volume = load_chart.add_series(SeriesKind::Histogram);
    let values = vec![1000.0; LOAD_BARS];
    load_chart
        .set_series_data(volume, &t, &values, &values, &values, &values)
        .unwrap();
    load_chart
        .series
        .iter_mut()
        .find(|series| series.id == volume)
        .unwrap()
        .visible = false;
    load_chart.time_scale.set_width(1600.0);
    load_chart.set_min_bar_spacing(0.001);
    load_chart.build_frame();
    load_chart.set_visible_logical_range(900_000.0, 999_999.0);
    let profile = load_chart
        .add_volume_profile_indicator(0, volume, Default::default())
        .unwrap();
    let start = Instant::now();
    load_chart.build_frame();
    let profile_ms = start.elapsed().as_secs_f64() * 1000.0;
    assert_eq!(
        load_chart
            .volume_profile_indicator_snapshot(profile)
            .unwrap()
            .profile
            .bar_count,
        100_000
    );
    let revision = load_chart
        .volume_profile_indicator_snapshot(profile)
        .unwrap()
        .calculation_revision;
    let start = Instant::now();
    for _ in 0..FRAMES {
        load_chart.build_frame();
    }
    let cached_ms = start.elapsed().as_secs_f64() * 1000.0 / FRAMES as f64;
    assert_eq!(
        load_chart
            .volume_profile_indicator_snapshot(profile)
            .unwrap()
            .calculation_revision,
        revision
    );
    println!("Target E — 100k visible-bar volume profile (48 rows):");
    let e_refresh = report("profile refresh + frame", profile_ms, FRAME_BUDGET_MS);
    let e_cached = report("cached profile frame", cached_ms, FRAME_BUDGET_MS);
    load_chart
        .add_periodic_profile_presentation(
            0,
            PeriodicProfilePresentationRequest {
                source: ProfileSource::Candles {
                    price_series: 0,
                    volume_series: volume,
                },
                boundaries: vec![ResampleBoundary {
                    start_time: 900_000,
                    end_time: 1_000_000,
                    session_id: 1,
                }],
                tick_size: 0.01,
                row_count: 48,
                value_area_percent: 70.0,
            },
            PeriodicProfilePresentationOptions {
                show_developing: true,
                ..Default::default()
            },
        )
        .unwrap();
    let started = Instant::now();
    load_chart.build_frame();
    let periodic_ms = started.elapsed().as_secs_f64() * 1000.0;
    let e_periodic = report(
        "periodic developing profile + frame",
        periodic_ms,
        FRAME_BUDGET_MS,
    );

    // ---- Target F: first Phase 2 general-only density gate -----------------------------------
    let mut general = ChartEngine::new(1600.0, 800.0, 1.0);
    let pane = general
        .add_pane_with_domain(
            true,
            HorizontalDomain::Continuous {
                scale: ContinuousScaleType::Linear,
            },
        )
        .expect("valid general pane");
    general
        .add_general_axis(GeneralAxisOptions::new(
            "general-x",
            pane,
            AxisDimension::X,
            GeneralScaleType::Linear,
        ))
        .expect("valid general X axis");
    general
        .add_general_axis(GeneralAxisOptions::new(
            "general-y",
            pane,
            AxisDimension::Y,
            GeneralScaleType::Linear,
        ))
        .expect("valid general Y axis");
    let mut x = Vec::with_capacity(GENERAL_LINE_POINTS);
    let mut y = Vec::with_capacity(GENERAL_LINE_POINTS);
    for index in 0..GENERAL_LINE_POINTS {
        x.push(index as f64);
        y.push(100.0 + (index as f64 * 0.013).sin() * 20.0);
    }
    let dataset = general
        .create_general_xy_dataset(GeneralXyInput::Numeric {
            ids: None,
            x,
            y,
            y_valid: None,
        })
        .expect("valid general line dataset");
    general
        .add_general_series(GeneralSeriesOptions::xy_line(
            pane,
            dataset,
            "general-x",
            "general-y",
        ))
        .expect("valid general XY line");
    general.recompute_layout_with_measure(true, |text, _| text.len() as f64 * 7.0, |_, _| 0.0);
    let mut general_frame = ChartFrame::default();
    general.build_frame_into(&mut general_frame);
    let start = Instant::now();
    for _ in 0..FRAMES {
        general.build_frame_into(&mut general_frame);
    }
    let general_frame_ms = start.elapsed().as_secs_f64() * 1000.0 / FRAMES as f64;
    let start = Instant::now();
    for sample in 0..GENERAL_LINE_HIT_SAMPLES {
        let x = 1600.0 * (sample as f64 + 0.5) / GENERAL_LINE_HIT_SAMPLES as f64;
        std::hint::black_box(general.general_hit_test(
            pane,
            x,
            400.0,
            GeneralHitMode::Nearest { max_distance: 32.0 },
        ));
    }
    let general_hit_ms = start.elapsed().as_secs_f64() * 1000.0 / GENERAL_LINE_HIT_SAMPLES as f64;
    println!("Target F — {GENERAL_LINE_POINTS} point general XY line:");
    let f_frame = report("xy_line build_frame", general_frame_ms, FRAME_BUDGET_MS);
    let f_hit = report(
        "xy_line nearest hit",
        general_hit_ms,
        GENERAL_LINE_HIT_BUDGET_MS,
    );

    // ---- Target G: mixed general-only dashboard ---------------------------------------------
    // Keep every current high-density geometry family in one coordinate region so this gate
    // catches accidental repeated walks, unbounded retained caches, and interaction regressions.
    let mut mixed = ChartEngine::new(1600.0, 800.0, 1.0);
    let mixed_pane = mixed
        .add_pane_with_domain(
            true,
            HorizontalDomain::Continuous {
                scale: ContinuousScaleType::Linear,
            },
        )
        .expect("valid mixed-general pane");
    mixed
        .add_general_axis(GeneralAxisOptions::new(
            "mixed-x",
            mixed_pane,
            AxisDimension::X,
            GeneralScaleType::Linear,
        ))
        .expect("valid mixed-general X axis");
    mixed
        .add_general_axis(GeneralAxisOptions::new(
            "mixed-y",
            mixed_pane,
            AxisDimension::Y,
            GeneralScaleType::Linear,
        ))
        .expect("valid mixed-general Y axis");
    let mixed_x: Vec<f64> = (0..GENERAL_MIX_POINTS_PER_SERIES)
        .map(|index| index as f64)
        .collect();
    let mixed_y: Vec<f64> = mixed_x
        .iter()
        .map(|x| 100.0 + (x * 0.013).sin() * 20.0)
        .collect();
    let line = mixed
        .create_general_xy_dataset(GeneralXyInput::Numeric {
            ids: None,
            x: mixed_x.clone(),
            y: mixed_y.clone(),
            y_valid: None,
        })
        .expect("valid mixed line dataset");
    mixed
        .add_general_series(GeneralSeriesOptions::xy_line(
            mixed_pane, line, "mixed-x", "mixed-y",
        ))
        .expect("valid mixed line");
    let area = mixed
        .create_general_xy_dataset(GeneralXyInput::Numeric {
            ids: None,
            x: mixed_x.clone(),
            y: mixed_y.iter().map(|value| value - 15.0).collect(),
            y_valid: None,
        })
        .expect("valid mixed area dataset");
    mixed
        .add_general_series(GeneralSeriesOptions::xy_area(
            mixed_pane, area, "mixed-x", "mixed-y",
        ))
        .expect("valid mixed area");
    let range = mixed
        .create_general_xy_dataset(GeneralXyInput::RangeNumeric {
            ids: None,
            x: mixed_x.clone(),
            low: mixed_y.iter().map(|value| value - 8.0).collect(),
            low_valid: None,
            high: mixed_y.iter().map(|value| value + 8.0).collect(),
            high_valid: None,
        })
        .expect("valid mixed range dataset");
    mixed
        .add_general_series(GeneralSeriesOptions::range_area(
            mixed_pane, range, "mixed-x", "mixed-y",
        ))
        .expect("valid mixed range area");
    let scatter = mixed
        .create_general_xy_dataset(GeneralXyInput::Numeric {
            ids: None,
            x: mixed_x.clone(),
            y: mixed_y.iter().map(|value| value + 25.0).collect(),
            y_valid: None,
        })
        .expect("valid mixed scatter dataset");
    mixed
        .add_general_series(GeneralSeriesOptions::scatter(
            mixed_pane, scatter, "mixed-x", "mixed-y",
        ))
        .expect("valid mixed scatter");
    let bubble = mixed
        .create_general_xy_dataset(GeneralXyInput::Bubble {
            ids: None,
            x: mixed_x,
            y: mixed_y.iter().map(|value| value - 30.0).collect(),
            y_valid: None,
            size: (0..GENERAL_MIX_POINTS_PER_SERIES)
                .map(|index| (index % 16 + 1) as f64)
                .collect(),
            size_valid: None,
        })
        .expect("valid mixed bubble dataset");
    mixed
        .add_general_series(GeneralSeriesOptions::bubble(
            mixed_pane, bubble, "mixed-x", "mixed-y",
        ))
        .expect("valid mixed bubble");
    mixed.recompute_layout_with_measure(true, |text, _| text.len() as f64 * 7.0, |_, _| 0.0);
    let mut mixed_frame = ChartFrame::default();
    mixed.build_frame_into(&mut mixed_frame);
    let start = Instant::now();
    for _ in 0..FRAMES {
        mixed.build_frame_into(&mut mixed_frame);
    }
    let mixed_frame_ms = start.elapsed().as_secs_f64() * 1000.0 / FRAMES as f64;
    let start = Instant::now();
    for sample in 0..GENERAL_LINE_HIT_SAMPLES {
        let x = 1600.0 * (sample as f64 + 0.5) / GENERAL_LINE_HIT_SAMPLES as f64;
        std::hint::black_box(mixed.general_hit_test(
            mixed_pane,
            x,
            400.0,
            GeneralHitMode::Nearest { max_distance: 32.0 },
        ));
    }
    let mixed_hit_ms = start.elapsed().as_secs_f64() * 1000.0 / GENERAL_LINE_HIT_SAMPLES as f64;
    let mixed_memory = mixed.memory_usage().estimated_live_bytes();
    println!(
        "Target G — 5-series mixed general dashboard ({} total rows):",
        GENERAL_MIX_POINTS_PER_SERIES * 5
    );
    let g_frame = report("mixed general build_frame", mixed_frame_ms, FRAME_BUDGET_MS);
    let g_hit = report(
        "mixed general nearest hit",
        mixed_hit_ms,
        GENERAL_LINE_HIT_BUDGET_MS,
    );
    let g_memory = report_bytes(
        "mixed general retained memory",
        mixed_memory,
        GENERAL_MIX_MEMORY_BUDGET_BYTES,
    );

    // ---- Target H: one engine with financial and general panes ------------------------------
    let mut combined = ChartEngine::new(1600.0, 800.0, 1.0);
    let (times, open, high, low, close) = gen_series(COMBINED_FINANCIAL_BARS, 0.0);
    combined
        .set_series_data(0, &times, &open, &high, &low, &close)
        .expect("valid combined financial fixture");
    combined.time_scale.set_width(1600.0);
    combined.fit_content();
    let combined_pane = combined
        .add_pane_with_domain(
            true,
            HorizontalDomain::Continuous {
                scale: ContinuousScaleType::Linear,
            },
        )
        .expect("valid combined general pane");
    combined
        .add_general_axis(GeneralAxisOptions::new(
            "combined-x",
            combined_pane,
            AxisDimension::X,
            GeneralScaleType::Linear,
        ))
        .expect("valid combined X axis");
    combined
        .add_general_axis(GeneralAxisOptions::new(
            "combined-y",
            combined_pane,
            AxisDimension::Y,
            GeneralScaleType::Linear,
        ))
        .expect("valid combined Y axis");
    let combined_x: Vec<f64> = (0..COMBINED_GENERAL_POINTS)
        .map(|index| index as f64)
        .collect();
    let combined_y: Vec<f64> = combined_x
        .iter()
        .map(|x| 50.0 + (x * 0.017).sin() * 12.0)
        .collect();
    let combined_dataset = combined
        .create_general_xy_dataset(GeneralXyInput::RangeNumeric {
            ids: None,
            x: combined_x,
            low: combined_y.iter().map(|value| value - 4.0).collect(),
            low_valid: None,
            high: combined_y.iter().map(|value| value + 4.0).collect(),
            high_valid: None,
        })
        .expect("valid combined range dataset");
    combined
        .add_general_series(GeneralSeriesOptions::range_area(
            combined_pane,
            combined_dataset,
            "combined-x",
            "combined-y",
        ))
        .expect("valid combined range area");
    combined.recompute_layout_with_measure(true, |text, _| text.len() as f64 * 7.0, |_, _| 0.0);
    let mut combined_frame = ChartFrame::default();
    combined.build_frame_into(&mut combined_frame);
    let start = Instant::now();
    for _ in 0..FRAMES {
        combined.build_frame_into(&mut combined_frame);
    }
    let combined_frame_ms = start.elapsed().as_secs_f64() * 1000.0 / FRAMES as f64;
    let combined_memory = combined.memory_usage().estimated_live_bytes();
    println!(
        "Target H — {COMBINED_FINANCIAL_BARS} financial bars + {COMBINED_GENERAL_POINTS} general points:"
    );
    let h_frame = report("combined build_frame", combined_frame_ms, FRAME_BUDGET_MS);
    let h_memory = report_bytes(
        "combined retained memory",
        combined_memory,
        COMBINED_MEMORY_BUDGET_BYTES,
    );

    // ---- Target I: dense numeric error bars -------------------------------------------------
    let mut errors = ChartEngine::new(1600.0, 800.0, 1.0);
    let error_pane = errors
        .add_pane_with_domain(
            true,
            HorizontalDomain::Continuous {
                scale: ContinuousScaleType::Linear,
            },
        )
        .expect("valid error-bar pane");
    errors
        .add_general_axis(GeneralAxisOptions::new(
            "error-x",
            error_pane,
            AxisDimension::X,
            GeneralScaleType::Linear,
        ))
        .expect("valid error-bar X axis");
    errors
        .add_general_axis(GeneralAxisOptions::new(
            "error-y",
            error_pane,
            AxisDimension::Y,
            GeneralScaleType::Linear,
        ))
        .expect("valid error-bar Y axis");
    let error_x: Vec<f64> = (0..ERROR_BAR_POINTS).map(|index| index as f64).collect();
    let error_y: Vec<f64> = error_x
        .iter()
        .map(|x| 100.0 + (x * 0.013).sin() * 20.0)
        .collect();
    let error_dataset = errors
        .create_general_xy_dataset(GeneralXyInput::ErrorNumeric {
            ids: None,
            x: error_x.clone(),
            y: error_y.clone(),
            y_valid: None,
            x_low: error_x.iter().map(|x| x - 0.25).collect(),
            x_low_valid: None,
            x_high: error_x.iter().map(|x| x + 0.25).collect(),
            x_high_valid: None,
            y_low: error_y.iter().map(|y| y - 3.0).collect(),
            y_low_valid: None,
            y_high: error_y.iter().map(|y| y + 3.0).collect(),
            y_high_valid: None,
        })
        .expect("valid dense error-bar dataset");
    errors
        .add_general_series(GeneralSeriesOptions::error_bar(
            error_pane,
            error_dataset,
            "error-x",
            "error-y",
        ))
        .expect("valid error-bar series");
    errors.recompute_layout_with_measure(true, |text, _| text.len() as f64 * 7.0, |_, _| 0.0);
    let mut error_frame = ChartFrame::default();
    errors.build_frame_into(&mut error_frame);
    let start = Instant::now();
    for _ in 0..FRAMES {
        errors.build_frame_into(&mut error_frame);
    }
    let error_frame_ms = start.elapsed().as_secs_f64() * 1000.0 / FRAMES as f64;
    let start = Instant::now();
    for sample in 0..GENERAL_LINE_HIT_SAMPLES {
        let x = 1600.0 * (sample as f64 + 0.5) / GENERAL_LINE_HIT_SAMPLES as f64;
        std::hint::black_box(errors.general_hit_test(
            error_pane,
            x,
            400.0,
            GeneralHitMode::Nearest { max_distance: 32.0 },
        ));
    }
    let error_hit_ms = start.elapsed().as_secs_f64() * 1000.0 / GENERAL_LINE_HIT_SAMPLES as f64;
    let error_memory = errors.memory_usage().estimated_live_bytes();
    println!("Target I — {ERROR_BAR_POINTS} numeric error bars:");
    let i_frame = report("error_bar build_frame", error_frame_ms, FRAME_BUDGET_MS);
    let i_hit = report(
        "error_bar nearest hit",
        error_hit_ms,
        GENERAL_LINE_HIT_BUDGET_MS,
    );
    let i_memory = report_bytes(
        "error_bar retained memory",
        error_memory,
        ERROR_BAR_MEMORY_BUDGET_BYTES,
    );

    // ---- Target K: 100x replay through the real chart clock and shared trade projections ------
    let mut replay = ChartEngine::new(1600.0, 800.0, 1.0);
    replay
        .configure_footprint_series(
            0,
            FootprintSeriesOptions {
                aggregation: FootprintAggregationOptions {
                    tick_size: 0.25,
                    ticks_per_row: 1,
                    bars: FootprintBarAggregation::Time {
                        interval_micros: 1_000_000,
                        anchor_micros: 0,
                    },
                    ..FootprintAggregationOptions::default()
                },
                visual: FootprintVisualOptions::default(),
            },
        )
        .expect("valid replay footprint options");
    let replay_stream = replay
        .add_trade_stream(
            "PERF:REPLAY",
            FootprintAggregationOptions {
                tick_size: 0.25,
                ticks_per_row: 1,
                bars: FootprintBarAggregation::Time {
                    interval_micros: 1_000_000,
                    anchor_micros: 0,
                },
                ..FootprintAggregationOptions::default()
            },
        )
        .expect("valid replay stream");
    replay
        .bind_footprint_series_to_stream(0, replay_stream)
        .expect("bind replay footprint");
    let replay_candles = replay.add_series(SeriesKind::Candlestick);
    replay
        .bind_trade_bar_series_to_stream(replay_candles, replay_stream)
        .expect("bind replay candles");
    let replay_trades = (1..=REPLAY_SECONDS)
        .map(|second| FootprintTrade {
            timestamp_micros: second as i64 * 1_000_000,
            price: 100.0 + ((second % 40) as f64 - 20.0) * 0.25,
            volume: 1.0,
            aggressor: if second % 2 == 0 {
                AggressorSide::Buy
            } else {
                AggressorSide::Sell
            },
            bid: None,
            ask: None,
            sequence: Some(second as u64),
            trade_id: Some(second as u64),
            conditions: 0,
            session_id: Some(1),
        })
        .collect();
    replay
        .set_trade_stream_trades(replay_stream, replay_trades)
        .expect("valid replay history");
    replay.time_scale.set_width(1600.0);
    replay.fit_content();
    let mut replay_frame = ChartFrame::default();
    let run_replay = |chart: &mut ChartEngine, frame: &mut ChartFrame| {
        chart
            .set_replay_clock_micros(Some(0))
            .expect("reset replay clock");
        let started = Instant::now();
        for frame_index in 1..=REPLAY_FRAMES {
            chart
                .set_replay_clock_micros(Some((frame_index * REPLAY_SPEED) as i64 * 1_000_000))
                .expect("advance replay clock");
            chart.build_frame_into(frame);
        }
        started.elapsed().as_secs_f64() * 1000.0 / REPLAY_FRAMES as f64
    };
    run_replay(&mut replay, &mut replay_frame);
    let replay_first_ms = run_replay(&mut replay, &mut replay_frame);
    let replay_memory_first = replay.memory_usage().estimated_live_bytes();
    let replay_second_ms = run_replay(&mut replay, &mut replay_frame);
    let replay_memory_second = replay.memory_usage().estimated_live_bytes();
    let replay_visible = replay
        .footprint_bars(0)
        .expect("replay footprint bars")
        .len();
    println!(
        "Target K — {REPLAY_SPEED}x replay over {REPLAY_SECONDS} seconds / {REPLAY_FRAMES} frames:"
    );
    let k_frame = report(
        "clock advance + shared frame",
        replay_first_ms.max(replay_second_ms),
        FRAME_BUDGET_MS,
    );
    let k_flat_memory = replay_memory_second <= replay_memory_first;
    println!(
        "  [{}] flat retained memory: {:.2} MiB -> {:.2} MiB; {replay_visible} visible bars",
        if k_flat_memory { "PASS" } else { "FAIL" },
        replay_memory_first as f64 / (1024.0 * 1024.0),
        replay_memory_second as f64 / (1024.0 * 1024.0),
    );

    // ---- Target L: bounded depth soak and shared image heatmap -------------------------------
    let mut depth = ChartEngine::new(1600.0, 800.0, 1.0);
    let (times, open, high, low, close) = gen_series(2_501, 0.0);
    depth
        .set_series_data(0, &times, &open, &high, &low, &close)
        .expect("valid depth time axis");
    let depth_stream = depth
        .add_depth_stream(
            "PERF:DEPTH",
            DepthOptions {
                tick_size: 0.25,
                max_levels_per_side: 256,
                history_bucket_micros: 100_000,
                max_history_buckets: 512,
                max_history_cells: 65_536,
                max_event_markers: 2_048,
            },
        )
        .expect("valid depth stream");
    let bids = (0..64)
        .map(|level| DepthLevel {
            price: 100.0 - level as f64 * 0.25,
            size: (level + 1) as f64,
            order_count: Some(level + 1),
        })
        .collect();
    let asks = (0..64)
        .map(|level| DepthLevel {
            price: 100.25 + level as f64 * 0.25,
            size: (level + 1) as f64,
            order_count: Some(level + 1),
        })
        .collect();
    depth
        .set_depth_snapshot(
            depth_stream,
            DepthSnapshot {
                timestamp_micros: 0,
                sequence: 1,
                bids,
                asks,
            },
        )
        .expect("valid depth snapshot");
    depth
        .add_depth_heatmap(
            depth_stream,
            DepthHeatmapOptions {
                price_min: 84.25,
                price_max: 116.0,
                maximum_size: 128.0,
                ..DepthHeatmapOptions::default()
            },
        )
        .expect("valid depth heatmap");
    depth.time_scale.set_width(1600.0);
    depth.fit_content();
    let run_depth_soak = |chart: &mut ChartEngine, next_sequence: &mut u64| {
        let mut worst_batch_ms = 0.0_f64;
        for _ in 0..DEPTH_SOAK_UPDATES / DEPTH_BATCH_UPDATES {
            let updates = (0..DEPTH_BATCH_UPDATES)
                .map(|offset| {
                    let sequence = next_sequence.saturating_add(offset as u64);
                    let side = if sequence.is_multiple_of(2) {
                        DepthSide::Bid
                    } else {
                        DepthSide::Ask
                    };
                    let level = (sequence as usize / 2) % 64;
                    DepthUpdate {
                        timestamp_micros: sequence as i64 * 1_000,
                        sequence,
                        previous_sequence: sequence - 1,
                        side,
                        level: DepthLevel {
                            price: match side {
                                DepthSide::Bid => 100.0 - level as f64 * 0.25,
                                DepthSide::Ask => 100.25 + level as f64 * 0.25,
                            },
                            size: (sequence % 127 + 1) as f64,
                            order_count: Some((sequence % 32 + 1) as u32),
                        },
                    }
                })
                .collect::<Vec<_>>();
            let started = Instant::now();
            chart
                .update_depth_batch(depth_stream, &updates)
                .expect("valid depth update batch");
            worst_batch_ms = worst_batch_ms.max(started.elapsed().as_secs_f64() * 1000.0);
            *next_sequence = next_sequence.saturating_add(DEPTH_BATCH_UPDATES as u64);
        }
        worst_batch_ms
    };
    let mut next_depth_sequence = 2_u64;
    let depth_first_ms = run_depth_soak(&mut depth, &mut next_depth_sequence);
    let depth_memory_first = depth.memory_usage().depth_capacity_bytes;
    let mut depth_frame = ChartFrame::default();
    let started = Instant::now();
    depth.build_frame_into(&mut depth_frame);
    let depth_frame_ms = started.elapsed().as_secs_f64() * 1000.0;
    let depth_images = depth_frame.panes[0]
        .under
        .iter()
        .filter_map(|primitive| match primitive {
            Prim::Image { image, .. } => Some(image),
            _ => None,
        })
        .collect::<Vec<_>>();
    let live_upload_bytes = depth_images
        .iter()
        .filter(|image| image.width == 1)
        .map(|image| image.pixels.len())
        .sum::<usize>();
    let depth_second_ms = run_depth_soak(&mut depth, &mut next_depth_sequence);
    let depth_memory_second = depth.memory_usage().depth_capacity_bytes;
    println!(
        "Target L — two {DEPTH_SOAK_UPDATES}-update depth soaks, {} heatmap images:",
        depth_images.len()
    );
    let l_update = report(
        "worst 100k depth batch",
        depth_first_ms.max(depth_second_ms),
        DEPTH_BATCH_BUDGET_MS,
    );
    let l_frame = report("depth heatmap build_frame", depth_frame_ms, FRAME_BUDGET_MS);
    let l_upload = report_bytes(
        "incremental live-edge image",
        live_upload_bytes,
        DEPTH_UPLOAD_BUDGET_BYTES,
    );
    let l_flat_memory = depth_memory_second <= depth_memory_first;
    println!(
        "  [{}] flat retained depth memory: {:.2} MiB -> {:.2} MiB",
        if l_flat_memory { "PASS" } else { "FAIL" },
        depth_memory_first as f64 / (1024.0 * 1024.0),
        depth_memory_second as f64 / (1024.0 * 1024.0),
    );

    // ---- Target M: bounded per-tick indicator work over a 1M-row source ---------------------
    // Every built-in kind is bound, so one tick advances 38 bindings (~60 outputs) plus the volume
    // series. Bounded rolling state makes a tick O(period) per binding, independent of history;
    // the budget keeps a tick (candle + volume update) under 1 ms, so a 60 fps host absorbs a
    // burst of ticks inside one frame with most of its 16.67 ms left for frame construction.
    let tick_cost = indicator_tick_cost(
        StudySet::BuiltIn,
        INDICATOR_TICK_ROWS,
        0,
        None,
        4,
        INDICATOR_TICK_APPENDS,
    );
    println!(
        "Target M — per-tick indicator cost, {} bindings over {INDICATOR_TICK_ROWS} rows (max {} work rows per tick; first append after install {:.2} ms, growing {:.2} MiB of column capacity):",
        tick_cost.bindings,
        tick_cost.max_work_rows,
        tick_cost.first_append_ms,
        tick_cost.first_append_growth_bytes as f64 / (1024.0 * 1024.0),
    );
    let (replace_mean, replace_median, replace_max) = tick_cost.replace_ms;
    let m_replace = report(
        &format!(
            "current-bar replace mean (median {replace_median:.3} ms, max {replace_max:.2} ms)"
        ),
        replace_mean,
        INDICATOR_TICK_BUDGET_MS,
    );
    let (append_mean, append_median, append_max) = tick_cost.append_ms;
    let m_append = report(
        &format!("new-bar append mean (median {append_median:.3} ms, max {append_max:.2} ms)"),
        append_mean,
        INDICATOR_TICK_BUDGET_MS,
    );

    // The same studies with a pre-installed one-second session (23,400 whitespace slots) after
    // the source: filling and revising the forming slot must cost the same bounded window.
    let slot_cost = indicator_tick_cost(
        StudySet::BuiltIn,
        INDICATOR_TICK_ROWS,
        23_400,
        None,
        4,
        INDICATOR_TICK_APPENDS,
    );
    println!(
        "Target M (slots) — the same ticks filling 23,400 pre-installed session slots (max {} work rows per tick; first fill after install {:.2} ms, growing {:.2} MiB of column capacity):",
        slot_cost.max_work_rows,
        slot_cost.first_append_ms,
        slot_cost.first_append_growth_bytes as f64 / (1024.0 * 1024.0),
    );
    // Filling the first slot extends every column the runtime retains, the aggregate price columns
    // among them, by one row inside capacity the install left. Unlike a live append past the
    // source, it grows no other column, so any growth here is a retained column reallocating.
    let m_slot_first_fill = report_check(
        "first slot fill grows no column capacity",
        slot_cost.first_append_growth_bytes == 0,
        &format!(
            "{:.2} MiB",
            slot_cost.first_append_growth_bytes as f64 / (1024.0 * 1024.0)
        ),
    );
    let (slot_replace_mean, slot_replace_median, slot_replace_max) = slot_cost.replace_ms;
    let m_slot_replace = report(
        &format!(
            "forming-slot replace mean (median {slot_replace_median:.3} ms, max {slot_replace_max:.2} ms)"
        ),
        slot_replace_mean,
        INDICATOR_TICK_BUDGET_MS,
    );
    let (slot_fill_mean, slot_fill_median, slot_fill_max) = slot_cost.append_ms;
    let m_slot_fill = report(
        &format!(
            "next-slot fill mean (median {slot_fill_median:.3} ms, max {slot_fill_max:.2} ms)"
        ),
        slot_fill_mean,
        INDICATOR_TICK_BUDGET_MS,
    );

    // Aggregate-input studies keep one derived price column each. It must be resident (re-deriving
    // the source per tick would be O(rows)), and it must carry bounded spare capacity so the first
    // append after a bulk install does not reallocate it and later growth stays within one eighth
    // plus a fixed floor of rows. The capacity is counted exactly, so these byte gates are not
    // noise-sensitive; the times are context.
    const COMPOSITE_COLUMNS: usize = 4;
    let composite = composite_input_cost(INDICATOR_TICK_ROWS, INDICATOR_TICK_APPENDS);
    let mib = |bytes: usize| bytes as f64 / (1024.0 * 1024.0);
    let resident_floor = COMPOSITE_COLUMNS * INDICATOR_TICK_ROWS * std::mem::size_of::<f64>();
    let headroom_bound = COMPOSITE_COLUMNS
        * (INDICATOR_TICK_ROWS + INDICATOR_TICK_ROWS / 8 + 4096)
        * std::mem::size_of::<f64>();
    println!(
        "Target M (composite inputs) — {COMPOSITE_COLUMNS} studies on hl2/hlc3/ohlc4/hlcc4 versus close over {INDICATOR_TICK_ROWS} rows (first append {:.2} ms canonical, {:.2} ms composite; context only):",
        composite.first_append_ms.0, composite.first_append_ms.1
    );
    let m_composite_resident = report_check(
        "price columns are resident after install",
        composite.install_bytes >= resident_floor,
        &format!(
            "{:.2} MiB (floor {:.2} MiB)",
            mib(composite.install_bytes),
            mib(resident_floor)
        ),
    );
    let m_composite_install = report_bytes(
        "price columns after install",
        composite.install_bytes,
        headroom_bound,
    );
    let m_composite_first = report_check(
        "first append does not grow the price columns",
        composite.first_append_bytes == composite.install_bytes,
        &format!(
            "{:.2} MiB -> {:.2} MiB",
            mib(composite.install_bytes),
            mib(composite.first_append_bytes)
        ),
    );
    let m_composite_final = report_bytes(
        &format!("price columns after {INDICATOR_TICK_APPENDS} more appends"),
        composite.final_bytes,
        headroom_bound,
    );
    let m_composite_append = report(
        "composite new-bar append mean",
        composite.append_mean_ms,
        INDICATOR_TICK_BUDGET_MS,
    );

    // ---- Target M (KLineChart): the 27 KLineChart templates over a 1M-row source ------------
    // Each template advances one row at a time from a checkpointed state, so a tick costs the
    // template's window however long the history is. The set is measured in its own block with the
    // same 1 ms per-tick budget, so its 27 bindings do not spend the headroom of the 38 built-in
    // bindings above, which already read close to theirs.
    let kline_cost = indicator_tick_cost(
        StudySet::KLineChart,
        INDICATOR_TICK_ROWS,
        0,
        None,
        4,
        INDICATOR_TICK_APPENDS,
    );
    println!(
        "Target M (KLineChart) — per-tick indicator cost, {} bindings over {INDICATOR_TICK_ROWS} rows (max {} work rows per tick; first append after install {:.2} ms, growing {:.2} MiB of column capacity):",
        kline_cost.bindings,
        kline_cost.max_work_rows,
        kline_cost.first_append_ms,
        kline_cost.first_append_growth_bytes as f64 / (1024.0 * 1024.0),
    );
    let (kline_replace_mean, kline_replace_median, kline_replace_max) = kline_cost.replace_ms;
    let m_kline_replace = report(
        &format!(
            "current-bar replace mean (median {kline_replace_median:.3} ms, max {kline_replace_max:.2} ms)"
        ),
        kline_replace_mean,
        INDICATOR_TICK_BUDGET_MS,
    );
    let (kline_append_mean, kline_append_median, kline_append_max) = kline_cost.append_ms;
    let m_kline_append = report(
        &format!(
            "new-bar append mean (median {kline_append_median:.3} ms, max {kline_append_max:.2} ms)"
        ),
        kline_append_mean,
        INDICATOR_TICK_BUDGET_MS,
    );
    // The work-row count is exact, so this bound is not noise-sensitive.
    let kline_work_bound = KLINECHART_WORK_ROWS_PER_BINDING * kline_cost.bindings;
    let m_kline_work = report_check(
        "rows evaluated per tick across all bindings",
        kline_cost.max_work_rows <= kline_work_bound,
        &format!(
            "{} (bound {kline_work_bound}: {KLINECHART_WORK_ROWS_PER_BINDING} per binding)",
            kline_cost.max_work_rows
        ),
    );
    let m_kline_bytes = report_bytes(
        "indicator runtime bytes after install and ticks",
        kline_cost.runtime_bytes,
        KLINECHART_RUNTIME_BUDGET_BYTES,
    );
    let (kline_repair_mean, kline_repair_median, kline_repair_max) = kline_cost
        .repair_ms
        .expect("the KLineChart set samples a historical repair");
    let m_kline_repair = report(
        &format!(
            "historical repair {REPAIR_DEPTH} rows back, all series, mean of {REPAIR_SAMPLES} (median {kline_repair_median:.3} ms, max {kline_repair_max:.2} ms)"
        ),
        kline_repair_mean,
        KLINECHART_REPAIR_BUDGET_MS,
    );

    // The same templates on a pre-installed one-second session (23,400 whitespace slots).
    let kline_slot_cost = indicator_tick_cost(
        StudySet::KLineChart,
        INDICATOR_TICK_ROWS,
        23_400,
        None,
        4,
        INDICATOR_TICK_APPENDS,
    );
    println!(
        "Target M (KLineChart, slots) — the same ticks filling 23,400 pre-installed session slots (max {} work rows per tick; first fill after install {:.2} ms):",
        kline_slot_cost.max_work_rows, kline_slot_cost.first_append_ms,
    );
    let (kline_slot_replace_mean, kline_slot_replace_median, kline_slot_replace_max) =
        kline_slot_cost.replace_ms;
    let m_kline_slot_replace = report(
        &format!(
            "forming-slot replace mean (median {kline_slot_replace_median:.3} ms, max {kline_slot_replace_max:.2} ms)"
        ),
        kline_slot_replace_mean,
        INDICATOR_TICK_BUDGET_MS,
    );
    let (kline_slot_fill_mean, kline_slot_fill_median, kline_slot_fill_max) =
        kline_slot_cost.append_ms;
    let m_kline_slot_fill = report(
        &format!(
            "next-slot fill mean (median {kline_slot_fill_median:.3} ms, max {kline_slot_fill_max:.2} ms)"
        ),
        kline_slot_fill_mean,
        INDICATOR_TICK_BUDGET_MS,
    );
    // Exact like the plain block's bound: the runtime evaluates the window of a tick, never the
    // whitespace slots after it, and the engine recolors only the rows the runtime rewrote.
    let m_kline_slot_work = report_check(
        "slots: rows evaluated per tick across all bindings",
        kline_slot_cost.max_work_rows <= kline_work_bound,
        &format!(
            "{} (bound {kline_work_bound}: {KLINECHART_WORK_ROWS_PER_BINDING} per binding)",
            kline_slot_cost.max_work_rows
        ),
    );

    // One whitespace row in the history (report-only): every later tick also re-reads the rows
    // its window spans, so a tick costs the window instead of one row, still not the history.
    let kline_gap_cost = indicator_tick_cost(
        StudySet::KLineChart,
        INDICATOR_TICK_ROWS,
        0,
        Some(INDICATOR_TICK_ROWS / 2),
        4,
        INDICATOR_TICK_APPENDS,
    );
    println!(
        "Target M (KLineChart, whitespace in history; report-only) — one whitespace row at row {}:",
        INDICATOR_TICK_ROWS / 2
    );
    println!(
        "  current-bar replace mean {:.3} ms (median {:.3}, max {:.2}), new-bar append mean {:.3} ms (median {:.3}, max {:.2}), max {} work rows per tick, historical repair mean {:.3} ms",
        kline_gap_cost.replace_ms.0,
        kline_gap_cost.replace_ms.1,
        kline_gap_cost.replace_ms.2,
        kline_gap_cost.append_ms.0,
        kline_gap_cost.append_ms.1,
        kline_gap_cost.append_ms.2,
        kline_gap_cost.max_work_rows,
        kline_gap_cost.repair_ms.map_or(f64::NAN, |repair| repair.0),
    );

    // ---- Target N: regression trends across a 1M-row source under live ticks ----------------
    // Each regression spans the whole history; a tick replacing or appending the latest bar must
    // extend the fits by the changed rows, so a tick plus its frame stays inside the per-tick
    // budget however long the anchored range is.
    let (regression_mean, regression_median, regression_max) =
        regression_tick_cost(INDICATOR_TICK_ROWS, 5, 200);
    println!("Target N — live ticks with 5 regression trends over {INDICATOR_TICK_ROWS} rows:");
    let n_tick = report(
        &format!(
            "tick + frame mean (median {regression_median:.3} ms, max {regression_max:.2} ms)"
        ),
        regression_mean,
        INDICATOR_TICK_BUDGET_MS,
    );

    // ---- Target O: daily-reset studies on daily bars (report-only) ----------------------------
    // A study that resets per session (session VWAP, VWAP bands, pivots) on daily bars makes every
    // drawn bar its own period, so each bar draws one bar-wide segment. The study-attributable
    // cost is the difference against the same chart without studies.
    println!(
        "Target O — daily-reset studies on daily bars (report-only, study cost = with - without):"
    );
    for (rows, spacing) in [(2_520, 0.5), (25_200, 0.01)] {
        let without = daily_reset_study_cost(rows, spacing, false);
        let with = daily_reset_study_cost(rows, spacing, true);
        println!(
            "  {rows} rows, min bar spacing {spacing} ({} study outputs): prims {} -> {} ({} two-point polylines, {} segment batches of {} pairs), pool {} -> {} points",
            with.outputs,
            without.prims,
            with.prims,
            with.lone_polylines,
            with.batches,
            with.batched_pairs,
            without.pool_points,
            with.pool_points,
        );
        println!(
            "    full rebuild {:.2} -> {:.2} ms, cursor-only frame {:.3} -> {:.3} ms",
            without.rebuild_ms, with.rebuild_ms, without.retained_ms, with.retained_ms
        );
        println!(
            "    Canvas2D: {} calls / {} strokes (was {} / {}), counting canvas {:.2} ms (was {:.2}), tiny-skia {:.2} ms (was {:.2})",
            with.canvas_calls,
            with.canvas_strokes,
            without.canvas_calls,
            without.canvas_strokes,
            with.canvas_counting_ms,
            without.canvas_counting_ms,
            with.raster_ms,
            without.raster_ms,
        );
        println!(
            "    prims_to_group {:.2} -> {:.2} ms, hit_test_series {:.3} -> {:.3} ms/sample",
            without.group_ms, with.group_ms, without.hit_ms, with.hit_ms
        );
    }

    let all_pass = a_pass
        && b_pass
        && c_pass
        && d_pass
        && j_text_count
        && j_scene
        && e_refresh
        && e_cached
        && e_periodic
        && f_frame
        && f_hit
        && g_frame
        && g_hit
        && g_memory
        && h_frame
        && h_memory
        && i_frame
        && i_hit
        && i_memory
        && k_frame
        && k_flat_memory
        && l_update
        && l_frame
        && l_upload
        && l_flat_memory
        && m_replace
        && m_append
        && m_slot_replace
        && m_slot_first_fill
        && m_slot_fill
        && m_composite_resident
        && m_composite_install
        && m_composite_first
        && m_composite_final
        && m_composite_append
        && m_kline_replace
        && m_kline_append
        && m_kline_work
        && m_kline_bytes
        && m_kline_repair
        && m_kline_slot_replace
        && m_kline_slot_fill
        && m_kline_slot_work
        && n_tick;
    println!(
        "\n{}",
        if all_pass {
            "ALL TARGETS PASS"
        } else {
            "SOME TARGETS FAILED"
        }
    );
    gate_exit(
        all_pass,
        std::env::var("AERIS_CHARTS_PERF_STRICT").ok().as_deref(),
    )
}

#[cfg(test)]
mod tests {
    use super::{gate_exit, strict_requested};
    use std::process::ExitCode;

    /// Only `1` enforces, matching the browser perf specs (`=== "1"`) and the release-gate guard,
    /// which simulates a disabled gate by setting the variable to `0`.
    #[test]
    fn only_one_enables_strict_mode() {
        assert!(strict_requested(Some("1")));
        assert!(!strict_requested(None));
        assert!(!strict_requested(Some("0")));
        assert!(!strict_requested(Some("")));
        assert!(!strict_requested(Some("true")));
    }

    /// `main` returns this value, so it is the process status: a failed target exits non-zero in
    /// strict mode, and exits zero when the variable is unset or `0` (report-only) or when every
    /// target passes.
    #[test]
    fn a_failed_target_exits_non_zero_only_in_strict_mode() {
        assert_eq!(gate_exit(false, Some("1")), ExitCode::FAILURE);
        assert_eq!(gate_exit(false, Some("0")), ExitCode::SUCCESS);
        assert_eq!(gate_exit(false, None), ExitCode::SUCCESS);
        assert_eq!(gate_exit(false, Some("true")), ExitCode::SUCCESS);
        assert_eq!(gate_exit(true, Some("1")), ExitCode::SUCCESS);
        assert_eq!(gate_exit(true, Some("0")), ExitCode::SUCCESS);
        assert_eq!(gate_exit(true, None), ExitCode::SUCCESS);
    }
}
