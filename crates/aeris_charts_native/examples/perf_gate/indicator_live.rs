//! Target V (upstream's Target Q; the fork's letter Q is the timeline-lane target): live indicator
//! updates for every `IndicatorKind` through the real engine API.
//!
//! Each binding is attached alone to a fixed-length history, timed for tip appends and tip
//! replacements, then removed; the appended rows are popped so every binding sees the same history.
//! The same procedure runs at a small and a large history, so a live path whose cost depends on
//! history length shows up as a p99 ratio, independent of the machine's absolute speed.
//!
//! The fork's cases add KDJ (both seeds), the first-value seed and China MACD conventions, the
//! amount-weighted VWAP and all 27 KLineChart templates (AVP on a turnover series) to upstream's
//! default-parameter case per kind.

use std::time::Instant;

use aeris_charts_engine::klinechart::{Indicator, NAMES};
use aeris_charts_engine::{
    ChartEngine, CustomStudyDefinition, CustomStudyFault, CustomStudyInput, CustomStudyOutput,
    CustomStudyPane, CustomStudyPlot, CustomStudyRuntime, IndicatorInputSource, IndicatorKind,
    IndicatorOutputStyle, IndicatorSeed, KdjSeed, SeriesKind,
};

pub const SMALL_ROWS: usize = 10_000;
pub const LARGE_ROWS: usize = 1_000_000;
/// p99 of 1,000 samples is the tenth slowest, so the one-off capacity growth of each output column
/// (at most five per binding) cannot set it, while any per-tick history-length cost does.
const SAMPLES: usize = 1_000;
const WARMUP: usize = 8;
/// Absolute p99 budget for one binding's tip append or tip replacement at LARGE_ROWS.
pub const KIND_P99_BUDGET_MS: f64 = 0.5;
/// Absolute p99 budget for one tip append or tip replacement with every case attached together
/// at LARGE_ROWS.
pub const ALL_ATTACHED_P99_BUDGET_MS: f64 = 4.0;
/// Largest allowed LARGE_ROWS p99 over SMALL_ROWS p99. A cost linear in history would show
/// as about 100.
pub const SCALING_LIMIT: f64 = 10.0;
/// SMALL_ROWS p99s below this are timer and cache noise, so the ratio's denominator is floored
/// here.
pub const SCALING_FLOOR_MS: f64 = 0.01;
const BAR_SECONDS: f64 = 60.0;
/// Every tenth history row has no volume sample, so that binding aligns volume by timestamp.
const SPARSE_VOLUME_STRIDE: usize = 10;
const CUSTOM_TYPE: &str = "perf.close";
/// `IndicatorKind` variants, including `Custom`, the fork's `Kdj` and `KLineChart`.
const VARIANTS: usize = 73;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Volume {
    None,
    Aligned,
    OwnTimeline,
    /// Aligned volume plus the turnover series as the amount column (amount-weighted VWAP).
    AlignedWithAmount,
}

#[derive(Clone)]
pub struct Case {
    pub label: String,
    kind: IndicatorKind,
    input: IndicatorInputSource,
    volume: Volume,
}

impl Case {
    /// KLineChart AVP reads turnover as the value of its scalar source series.
    fn reads_turnover_source(&self) -> bool {
        matches!(self.kind, IndicatorKind::KLineChart(Indicator::Avp))
    }
}

pub struct CaseTiming {
    pub label: String,
    /// p99 in milliseconds at `[SMALL_ROWS, LARGE_ROWS]`.
    pub append_p99_ms: [f64; 2],
    pub replace_p99_ms: [f64; 2],
}

fn uses_volume(kind: &IndicatorKind) -> bool {
    matches!(
        kind,
        IndicatorKind::Vwap
            | IndicatorKind::VwapBands { .. }
            | IndicatorKind::Vwma { .. }
            | IndicatorKind::Obv
            | IndicatorKind::AccumulationDistribution
            | IndicatorKind::PriceVolumeTrend
            | IndicatorKind::ChaikinOscillator { .. }
            | IndicatorKind::Klinger { .. }
            | IndicatorKind::RelativeVolume { .. }
            | IndicatorKind::VolumeOscillator { .. }
            | IndicatorKind::ElderForce { .. }
            | IndicatorKind::EaseOfMovement { .. }
            | IndicatorKind::Cmf { .. }
            | IndicatorKind::Mfi { .. }
            | IndicatorKind::Volume { .. }
    ) || matches!(kind, IndicatorKind::KLineChart(indicator) if indicator.needs_volume())
}

/// Distinct slot per variant. Adding a variant fails to compile here until it is given a slot,
/// and `cases` then fails at run time until a case covers it.
fn variant_slot(kind: &IndicatorKind) -> usize {
    match kind {
        IndicatorKind::Aroon { .. } => 0,
        IndicatorKind::AwesomeOscillator => 1,
        IndicatorKind::Dpo { .. } => 2,
        IndicatorKind::ChandeMomentum { .. } => 3,
        IndicatorKind::Sma { .. } => 4,
        IndicatorKind::Ema { .. } => 5,
        IndicatorKind::Dema { .. } => 6,
        IndicatorKind::Tema { .. } => 7,
        IndicatorKind::Smma { .. } => 8,
        IndicatorKind::Hma { .. } => 9,
        IndicatorKind::Vwma { .. } => 10,
        IndicatorKind::StandardDeviation { .. } => 11,
        IndicatorKind::Cci { .. } => 12,
        IndicatorKind::WilliamsR { .. } => 13,
        IndicatorKind::StochasticRsi { .. } => 14,
        IndicatorKind::Momentum { .. } => 15,
        IndicatorKind::RateOfChange { .. } => 16,
        IndicatorKind::Donchian { .. } => 17,
        IndicatorKind::PivotPoints { .. } => 18,
        IndicatorKind::ZigZag { .. } => 19,
        IndicatorKind::Keltner { .. } => 20,
        IndicatorKind::AdxDmi { .. } => 21,
        IndicatorKind::ParabolicSar => 22,
        IndicatorKind::SuperTrend { .. } => 23,
        IndicatorKind::Ichimoku => 24,
        IndicatorKind::EmaRibbon { .. } => 25,
        IndicatorKind::Bollinger { .. } => 26,
        IndicatorKind::BollingerMetrics { .. } => 27,
        IndicatorKind::Envelopes { .. } => 28,
        IndicatorKind::Alma { .. } => 29,
        IndicatorKind::Rsi { .. } => 30,
        IndicatorKind::Macd { .. } => 31,
        IndicatorKind::Stochastic { .. } => 32,
        IndicatorKind::Atr { .. } => 33,
        IndicatorKind::Vwap => 34,
        IndicatorKind::Obv => 35,
        IndicatorKind::AccumulationDistribution => 36,
        IndicatorKind::PriceVolumeTrend => 37,
        IndicatorKind::ChaikinOscillator { .. } => 38,
        IndicatorKind::Klinger { .. } => 39,
        IndicatorKind::Kama { .. } => 40,
        IndicatorKind::McGinley { .. } => 41,
        IndicatorKind::LinearRegression { .. } => 42,
        IndicatorKind::Choppiness { .. } => 43,
        IndicatorKind::AtrBands { .. } => 44,
        IndicatorKind::RelativeVolume { .. } => 45,
        IndicatorKind::VolumeOscillator { .. } => 46,
        IndicatorKind::ElderForce { .. } => 47,
        IndicatorKind::EaseOfMovement { .. } => 48,
        IndicatorKind::HistoricalVolatility { .. } => 49,
        IndicatorKind::Trix { .. } => 50,
        IndicatorKind::Kst { .. } => 51,
        IndicatorKind::Tsi { .. } => 52,
        IndicatorKind::MassIndex { .. } => 53,
        IndicatorKind::Vortex { .. } => 54,
        IndicatorKind::CoppockCurve { .. } => 55,
        IndicatorKind::FisherTransform { .. } => 56,
        IndicatorKind::UltimateOscillator { .. } => 57,
        IndicatorKind::Cmf { .. } => 58,
        IndicatorKind::Mfi { .. } => 59,
        IndicatorKind::Volume { .. } => 60,
        IndicatorKind::VwapBands { .. } => 61,
        IndicatorKind::Wma { .. } => 62,
        IndicatorKind::SwingPoints { .. } => 63,
        IndicatorKind::MarketStructure { .. } => 64,
        IndicatorKind::FairValueGaps { .. } => 65,
        IndicatorKind::OrderBlocks { .. } => 66,
        IndicatorKind::SessionLevels { .. } => 67,
        IndicatorKind::PreviousPeriodLevels { .. } => 68,
        IndicatorKind::OpeningRange { .. } => 69,
        IndicatorKind::Custom { .. } => 70,
        IndicatorKind::Kdj { .. } => 71,
        IndicatorKind::KLineChart(_) => 72,
    }
}

/// Engine schema names. `schema_definition(name, 14, 2.0)` is the documented canonical-default
/// query; pivot points select their variant by index instead, and 1 is the default (standard).
const DEFAULT_KIND_NAMES: [&str; 71] = [
    "aroon",
    "awesome_oscillator",
    "dpo",
    "chande_momentum",
    "sma",
    "ema",
    "dema",
    "tema",
    "smma",
    "hma",
    "vwma",
    "standard_deviation",
    "cci",
    "williams_r",
    "stochastic_rsi",
    "momentum",
    "roc",
    "donchian",
    "pivot_points",
    "zigzag",
    "keltner",
    "adx_dmi",
    "parabolic_sar",
    "supertrend",
    "ichimoku",
    "ema_ribbon",
    "bollinger",
    "bollinger_metrics",
    "envelopes",
    "alma",
    "rsi",
    "macd",
    "stochastic",
    "atr",
    "vwap",
    "obv",
    "accumulation_distribution",
    "price_volume_trend",
    "chaikin_oscillator",
    "klinger",
    "kama",
    "mcginley",
    "linear_regression",
    "choppiness",
    "atr_bands",
    "relative_volume",
    "volume_oscillator",
    "elder_force",
    "ease_of_movement",
    "historical_volatility",
    "trix",
    "kst",
    "tsi",
    "mass_index",
    "vortex",
    "coppock_curve",
    "fisher_transform",
    "ultimate_oscillator",
    "cmf",
    "mfi",
    "volume",
    "vwap_bands",
    "wma",
    "swing_points",
    "market_structure",
    "fair_value_gaps",
    "order_blocks",
    "session_levels",
    "previous_period_levels",
    "opening_range",
    "kdj",
];

fn case(label: &str, kind: IndicatorKind, input: IndicatorInputSource, volume: Volume) -> Case {
    Case {
        label: label.to_string(),
        kind,
        input,
        volume,
    }
}

/// One default-parameter case per built-in kind, the exponential Envelopes mode, derived input
/// sources, a volume series on its own timeline, and one Rust custom study.
pub fn cases() -> Vec<Case> {
    let mut cases = DEFAULT_KIND_NAMES
        .iter()
        .map(|&name| {
            let kind = IndicatorKind::schema_definition(
                name,
                if name == "pivot_points" { 1 } else { 14 },
                2.0,
            )
            .unwrap_or_else(|| panic!("{name} has a default definition"));
            let volume = if uses_volume(&kind) {
                Volume::Aligned
            } else {
                Volume::None
            };
            case(name, kind, IndicatorInputSource::Close, volume)
        })
        .collect::<Vec<_>>();
    cases.extend([
        case(
            "envelopes (exponential)",
            IndicatorKind::Envelopes {
                period: 20,
                percent: 2.0,
                exponential: true,
            },
            IndicatorInputSource::Close,
            Volume::None,
        ),
        case(
            "sma (hlc3 input)",
            IndicatorKind::Sma { period: 14 },
            IndicatorInputSource::Hlc3,
            Volume::None,
        ),
        case(
            "rsi (ohlc4 input)",
            IndicatorKind::Rsi {
                period: 14,
                seed: IndicatorSeed::Sma,
            },
            IndicatorInputSource::Ohlc4,
            Volume::None,
        ),
        case(
            "obv (volume on its own timeline)",
            IndicatorKind::Obv,
            IndicatorInputSource::Close,
            Volume::OwnTimeline,
        ),
        case(
            "vwap (volume on its own timeline)",
            IndicatorKind::Vwap,
            IndicatorInputSource::Close,
            Volume::OwnTimeline,
        ),
        case(
            "ema (first-value seed)",
            IndicatorKind::Ema {
                period: 14,
                seed: IndicatorSeed::FirstValue,
            },
            IndicatorInputSource::Close,
            Volume::None,
        ),
        case(
            "rsi (first-value seed)",
            IndicatorKind::Rsi {
                period: 14,
                seed: IndicatorSeed::FirstValue,
            },
            IndicatorInputSource::Close,
            Volume::None,
        ),
        case(
            "macd (China convention)",
            IndicatorKind::Macd {
                fast: 12,
                slow: 26,
                signal: 9,
                seed: IndicatorSeed::FirstValue,
                histogram_multiplier: 2.0,
            },
            IndicatorInputSource::Close,
            Volume::None,
        ),
        case(
            "kdj (first-value seed)",
            IndicatorKind::Kdj {
                period: 9,
                k_smoothing: 3,
                d_smoothing: 3,
                seed: KdjSeed::FirstValue,
            },
            IndicatorInputSource::Close,
            Volume::None,
        ),
        case(
            "vwap (amount-weighted)",
            IndicatorKind::Vwap,
            IndicatorInputSource::Close,
            Volume::AlignedWithAmount,
        ),
        case(
            "custom (Rust close passthrough, hl2 input)",
            IndicatorKind::Custom {
                type_id: CUSTOM_TYPE.into(),
                version: 1,
                parameters: Default::default(),
                output_count: 1,
            },
            IndicatorInputSource::Hl2,
            Volume::None,
        ),
    ]);
    cases.extend(NAMES.iter().map(|&name| {
        let indicator = Indicator::from_name(name).expect("every listed name is a template");
        let kind = IndicatorKind::KLineChart(indicator);
        let volume = if uses_volume(&kind) {
            Volume::Aligned
        } else {
            Volume::None
        };
        case(
            &format!("KLineChart {name}"),
            kind,
            IndicatorInputSource::Close,
            volume,
        )
    }));
    let mut covered = [false; VARIANTS];
    for case in &cases {
        covered[variant_slot(&case.kind)] = true;
    }
    assert!(
        covered.iter().all(|&covered| covered),
        "Target V must cover every IndicatorKind variant"
    );
    cases
}

struct ClosePassthrough;

impl CustomStudyRuntime for ClosePassthrough {
    fn compute(
        &mut self,
        input: CustomStudyInput<'_>,
        out: &mut [Vec<f64>],
    ) -> Result<(), CustomStudyFault> {
        out[0].extend_from_slice(&input.close[input.from..]);
        Ok(())
    }
}

fn register_custom_study(chart: &mut ChartEngine) {
    chart
        .register_custom_study(
            CustomStudyDefinition {
                type_id: CUSTOM_TYPE.into(),
                version: 1,
                title: "Close".into(),
                parameters: Vec::new(),
                outputs: vec![CustomStudyOutput {
                    name: "Close".into(),
                    plot: CustomStudyPlot::Line,
                    pane: CustomStudyPane::Price,
                    default_style: IndicatorOutputStyle {
                        visible: true,
                        ..IndicatorOutputStyle::default()
                    },
                }],
                uses_volume: false,
            },
            Box::new(|_| Ok(Box::new(ClosePassthrough))),
        )
        .expect("valid custom study definition");
}

fn bar(row: usize) -> [f64; 4] {
    let x = row as f64;
    let close =
        100.0 + (x * 0.0031).sin() * 25.0 + (x * 0.17).sin() * 1.5 + (x * 0.011).cos() * 4.0;
    let open = close - (x * 0.53).sin() * 0.6;
    let high = close.max(open) + 0.3 + (row % 5) as f64 * 0.05;
    let low = close.min(open) - 0.3 - (row % 3) as f64 * 0.05;
    [open, high, low, close]
}

fn volume(row: usize) -> f64 {
    100.0 + (row % 37) as f64 * 7.0
}

/// Traded value of a bar: its volume at the close.
fn turnover(values: [f64; 4], volume: f64) -> f64 {
    values[3] * volume
}

/// One chart with the source bars, a volume series on the same timeline, a volume series that
/// skips rows, and a turnover series on the source timeline. Live rows are appended to all four,
/// as a host feeding one stream does.
pub struct Fixture {
    pub chart: ChartEngine,
    rows: usize,
    aligned_volume: u32,
    sparse_volume: u32,
    turnover: u32,
}

impl Fixture {
    pub fn new(rows: usize) -> Self {
        let mut chart = ChartEngine::new(1600.0, 800.0, 1.0);
        register_custom_study(&mut chart);
        let aligned_volume = chart.add_series(SeriesKind::Histogram);
        let sparse_volume = chart.add_series(SeriesKind::Histogram);
        let turnover_series = chart.add_series(SeriesKind::Line);
        let times = (0..rows)
            .map(|row| row as f64 * BAR_SECONDS)
            .collect::<Vec<_>>();
        let bars = (0..rows).map(bar).collect::<Vec<_>>();
        let column = |index: usize| bars.iter().map(|bar| bar[index]).collect::<Vec<_>>();
        chart
            .set_series_data(0, &times, &column(0), &column(1), &column(2), &column(3))
            .expect("valid indicator fixture");
        let volumes = (0..rows).map(volume).collect::<Vec<_>>();
        chart
            .set_series_data(
                aligned_volume,
                &times,
                &volumes,
                &volumes,
                &volumes,
                &volumes,
            )
            .expect("valid aligned volume");
        let sparse_rows = (0..rows)
            .filter(|row| row % SPARSE_VOLUME_STRIDE != SPARSE_VOLUME_STRIDE - 1)
            .collect::<Vec<_>>();
        let sparse_times = sparse_rows
            .iter()
            .map(|&row| row as f64 * BAR_SECONDS)
            .collect::<Vec<_>>();
        let sparse = sparse_rows
            .iter()
            .map(|&row| volume(row))
            .collect::<Vec<_>>();
        chart
            .set_series_data(
                sparse_volume,
                &sparse_times,
                &sparse,
                &sparse,
                &sparse,
                &sparse,
            )
            .expect("valid sparse volume");
        let turnovers = bars
            .iter()
            .zip(&volumes)
            .map(|(&bar, &volume)| turnover(bar, volume))
            .collect::<Vec<_>>();
        chart
            .set_series_data(
                turnover_series,
                &times,
                &turnovers,
                &turnovers,
                &turnovers,
                &turnovers,
            )
            .expect("valid turnover");
        Self {
            chart,
            rows,
            aligned_volume,
            sparse_volume,
            turnover: turnover_series,
        }
    }

    fn volume_series(&self, volume: Volume) -> Option<u32> {
        match volume {
            Volume::None => None,
            Volume::Aligned | Volume::AlignedWithAmount => Some(self.aligned_volume),
            Volume::OwnTimeline => Some(self.sparse_volume),
        }
    }

    pub fn attach(&mut self, case: &Case) -> Vec<u32> {
        let volume = self.volume_series(case.volume);
        let source = if case.reads_turnover_source() {
            self.turnover
        } else {
            0
        };
        let amount = (case.volume == Volume::AlignedWithAmount).then_some(self.turnover);
        let outputs = if let IndicatorKind::Custom { .. } = case.kind {
            self.chart
                .add_custom_study(CUSTOM_TYPE, source, case.input, volume, Default::default())
                .expect("valid custom binding")
        } else {
            self.chart.add_indicator_kind_with_sources(
                source,
                case.input,
                case.kind.clone(),
                volume,
                amount,
            )
        };
        assert!(!outputs.is_empty(), "{} binding is created", case.label);
        outputs
    }

    /// Write `values` at `row` (appending when `row` is one past the last row) to the source, to
    /// both volume series and to the turnover series.
    fn write(&mut self, row: usize, values: [f64; 4], volume: f64) {
        let time = row as f64 * BAR_SECONDS;
        assert!(self.chart.update_series_bar(0, time, values));
        for series in [self.aligned_volume, self.sparse_volume] {
            assert!(
                self.chart
                    .update_series_bar(series, time, [volume, volume, volume, volume])
            );
        }
        let traded = turnover(values, volume);
        assert!(
            self.chart
                .update_series_bar(self.turnover, time, [traded; 4])
        );
    }

    pub fn append(&mut self) {
        let row = self.rows;
        self.write(row, bar(row), volume(row));
        self.rows += 1;
    }

    pub fn replace_tip(&mut self, step: usize) {
        let row = self.rows - 1;
        let [open, high, low, close] = bar(row);
        let wobble = ((step as f64) * 0.37).sin() * 0.2;
        self.write(
            row,
            [
                open,
                high + wobble.abs(),
                low - wobble.abs(),
                close + wobble,
            ],
            volume(row) + step as f64,
        );
    }

    fn pop(&mut self, count: usize) {
        for series in [0, self.aligned_volume, self.sparse_volume, self.turnover] {
            self.chart
                .series_pop(series, count)
                .expect("live rows can be popped");
        }
        self.rows -= count;
    }

    /// Source bars and per-row volume as they stand now, to load a fresh engine with.
    pub fn snapshot(&self) -> Self {
        let mut fresh = Self::new(self.rows);
        let rows = (0..self.rows)
            .map(|row| row as f64 * BAR_SECONDS)
            .collect::<Vec<_>>();
        let mut columns: [Vec<f64>; 4] = Default::default();
        for point in self.chart.series_data(0) {
            columns[0].push(point.open);
            columns[1].push(point.high);
            columns[2].push(point.low);
            columns[3].push(point.close);
        }
        fresh
            .chart
            .set_series_data(0, &rows, &columns[0], &columns[1], &columns[2], &columns[3])
            .expect("valid fresh source");
        for series in [self.aligned_volume, self.sparse_volume, self.turnover] {
            let points = self.chart.series_data(series);
            let times = points
                .iter()
                .map(|point| point.time as f64)
                .collect::<Vec<_>>();
            let values = points.iter().map(|point| point.close).collect::<Vec<_>>();
            fresh
                .chart
                .set_series_data(series, &times, &values, &values, &values, &values)
                .expect("valid fresh volume");
        }
        fresh
    }
}

fn p99(samples: &mut [f64]) -> f64 {
    samples.sort_unstable_by(f64::total_cmp);
    samples[((samples.len() - 1) as f64 * 0.99).round() as usize]
}

/// Time `SAMPLES` tip appends and tip replacements, after `WARMUP` untimed pairs. Returns the
/// append and replace p99 in milliseconds. Leaves the appended rows in place.
pub fn time_live_updates(fixture: &mut Fixture) -> (f64, f64) {
    for step in 0..WARMUP {
        fixture.append();
        fixture.replace_tip(step);
    }
    let mut appends = Vec::with_capacity(SAMPLES);
    let mut replaces = Vec::with_capacity(SAMPLES);
    for step in 0..SAMPLES {
        let started = Instant::now();
        fixture.append();
        appends.push(started.elapsed().as_secs_f64() * 1000.0);
        let started = Instant::now();
        fixture.replace_tip(step);
        replaces.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    (p99(&mut appends), p99(&mut replaces))
}

/// Per-case p99 timings at both history sizes, each binding attached alone.
pub fn measure_each_kind(cases: &[Case]) -> Vec<CaseTiming> {
    let mut timings = cases
        .iter()
        .map(|case| CaseTiming {
            label: case.label.clone(),
            append_p99_ms: [0.0; 2],
            replace_p99_ms: [0.0; 2],
        })
        .collect::<Vec<_>>();
    for (size, rows) in [SMALL_ROWS, LARGE_ROWS].into_iter().enumerate() {
        let mut fixture = Fixture::new(rows);
        for (case, timing) in cases.iter().zip(&mut timings) {
            let outputs = fixture.attach(case);
            let (append, replace) = time_live_updates(&mut fixture);
            timing.append_p99_ms[size] = append;
            timing.replace_p99_ms[size] = replace;
            assert!(fixture.chart.remove_series(outputs[0]));
            assert!(fixture.chart.indicator_bindings().is_empty());
            fixture.pop(WARMUP + SAMPLES);
        }
    }
    timings
}

/// Compare the live outputs with a fresh engine loaded with the same bars, within 1e-9 relative.
/// NaN rows must match exactly.
pub fn compare_with_fresh_engine(
    fixture: &Fixture,
    cases: &[Case],
    outputs: &[Vec<u32>],
) -> Result<(), String> {
    let mut fresh = fixture.snapshot();
    for (case, live) in cases.iter().zip(outputs) {
        let expected = fresh.attach(case);
        if expected.len() != live.len() {
            return Err(format!("{}: output count differs", case.label));
        }
        for (&live_id, &fresh_id) in live.iter().zip(&expected) {
            let actual = fixture.chart.series_data(live_id);
            let reference = fresh.chart.series_data(fresh_id);
            if actual.len() != reference.len() {
                return Err(format!(
                    "{} output {live_id}: {} rows live, {} fresh",
                    case.label,
                    actual.len(),
                    reference.len()
                ));
            }
            for (row, (a, b)) in actual.iter().zip(&reference).enumerate() {
                let same = a.time == b.time
                    && ((a.close.is_nan() && b.close.is_nan())
                        || (a.close - b.close).abs() <= 1e-9 * b.close.abs().max(1.0));
                if !same {
                    return Err(format!(
                        "{} output {live_id} row {row}: live {:?} fresh {:?}",
                        case.label, a, b
                    ));
                }
            }
        }
    }
    Ok(())
}

fn verdict(pass: bool) -> &'static str {
    if pass { "PASS" } else { "FAIL" }
}

/// `LARGE_ROWS` p99 over the floored `SMALL_ROWS` p99.
fn scaling(p99_ms: [f64; 2]) -> f64 {
    p99_ms[1] / p99_ms[0].max(SCALING_FLOOR_MS)
}

/// Target V: measure, print, and return whether every check passed.
pub fn run() -> bool {
    let cases = cases();
    let started = Instant::now();
    let timings = measure_each_kind(&cases);
    println!(
        "Target V — live indicator updates, {} cases covering all {VARIANTS} IndicatorKind variants at default parameters, \
         {SMALL_ROWS} and {LARGE_ROWS} rows ({SAMPLES} tip appends + {SAMPLES} tip replacements per case, {:.1} s):",
        cases.len(),
        started.elapsed().as_secs_f64(),
    );
    let mut pass = true;
    for (operation, p99) in [
        (
            "tip append",
            (|t: &CaseTiming| t.append_p99_ms) as fn(&CaseTiming) -> [f64; 2],
        ),
        ("tip replace", |t: &CaseTiming| t.replace_p99_ms),
    ] {
        let mut over_budget = timings
            .iter()
            .filter(|timing| p99(timing)[1] > KIND_P99_BUDGET_MS)
            .peekable();
        let budget_pass = over_budget.peek().is_none();
        for timing in over_budget {
            println!(
                "    {} {operation} p99 {:.4} ms",
                timing.label,
                p99(timing)[1]
            );
        }
        let slowest = timings
            .iter()
            .max_by(|a, b| p99(a)[1].total_cmp(&p99(b)[1]))
            .expect("Target V has cases");
        println!(
            "  [{}] slowest {operation} p99 at {LARGE_ROWS} rows: {:.4} ms, {} (budget {KIND_P99_BUDGET_MS:.2} ms per binding)",
            verdict(budget_pass),
            p99(slowest)[1],
            slowest.label,
        );
        let mut over_limit = timings
            .iter()
            .filter(|timing| scaling(p99(timing)) > SCALING_LIMIT)
            .peekable();
        let scaling_pass = over_limit.peek().is_none();
        for timing in over_limit {
            let [small, large] = p99(timing);
            println!(
                "    {} {operation} p99 {small:.4} ms -> {large:.4} ms (x{:.1})",
                timing.label,
                scaling(p99(timing))
            );
        }
        let steepest = timings
            .iter()
            .max_by(|a, b| scaling(p99(a)).total_cmp(&scaling(p99(b))))
            .expect("Target V has cases");
        println!(
            "  [{}] steepest {operation} p99 growth {SMALL_ROWS} -> {LARGE_ROWS} rows: x{:.1}, {} \
             ({:.4} -> {:.4} ms; limit x{SCALING_LIMIT:.0} over max(p99, {SCALING_FLOOR_MS} ms))",
            verdict(scaling_pass),
            scaling(p99(steepest)),
            steepest.label,
            p99(steepest)[0],
            p99(steepest)[1],
        );
        pass &= budget_pass && scaling_pass;
    }

    let mut together = [[0.0; 2]; 2];
    for (size, rows) in [SMALL_ROWS, LARGE_ROWS].into_iter().enumerate() {
        let mut fixture = Fixture::new(rows);
        let outputs = cases
            .iter()
            .map(|case| fixture.attach(case))
            .collect::<Vec<_>>();
        let (append, replace) = time_live_updates(&mut fixture);
        together[0][size] = append;
        together[1][size] = replace;
        if rows == LARGE_ROWS {
            let matches = compare_with_fresh_engine(&fixture, &cases, &outputs);
            println!(
                "  [{}] every case attached together at {} rows matches a fresh engine{}",
                verdict(matches.is_ok()),
                fixture.rows,
                matches
                    .as_ref()
                    .err()
                    .map_or(String::new(), |error| format!(": {error}")),
            );
            pass &= matches.is_ok();
        }
    }
    for (operation, p99) in [("tip append", together[0]), ("tip replace", together[1])] {
        let budget_pass = p99[1] <= ALL_ATTACHED_P99_BUDGET_MS;
        let scaling_pass = scaling(p99) <= SCALING_LIMIT;
        println!(
            "  [{}] every case attached together, {operation} p99 at {LARGE_ROWS} rows: {:.4} ms (budget {ALL_ATTACHED_P99_BUDGET_MS:.2} ms)",
            verdict(budget_pass),
            p99[1],
        );
        println!(
            "  [{}] every case attached together, {operation} p99 growth: x{:.1} ({:.4} -> {:.4} ms; limit x{SCALING_LIMIT:.0})",
            verdict(scaling_pass),
            scaling(p99),
            p99[0],
            p99[1],
        );
        pass &= budget_pass && scaling_pass;
    }
    pass
}
