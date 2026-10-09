//! Sliding-window exactness on every computation path.
//!
//! Each sliding-window kind is computed five ways (dense batch, incremental full rebuild, one-row
//! live appends, live appends that each replace a provisional tip, and a historical repair from
//! the 1,021-row checkpoint neighbourhood) and every path is compared with two references built
//! here:
//!
//! - a hand-written per-window formula using compensated (Neumaier) summation, for the summation
//!   kinds. It shares no code with production. Its tolerance adds the forward error bound of a
//!   naive per-window sum (`1e-14 * scale`, where `scale` is the magnitude of the summed terms in
//!   output units), because a naive sum of $1M prices cannot be exact to `1e-12` absolute even
//!   without any history.
//! - a fresh-window oracle: the kind recomputed on only the rows its value depends on. It uses the
//!   strict `1e-9` relative / `1e-12` absolute tolerance, so any value that remembers rows outside
//!   its window (running-sum cancellation residue) fails.
//!
//! Fork rule (owner decision Q-G): both references are the deleted-rows oracle. They are evaluated
//! on the series with its whitespace rows removed and scattered back to source rows, so a window
//! that meets a gap spans the last valid rows and is compared like any other; every path must be
//! NaN (or not yet emitted) at the gap rows themselves. Upstream defines its references only on
//! whitespace-free windows, because its windows stay blank while a gap is inside them.
//!
//! The fixtures put large moves (or $1M prices) before long exact flat runs, near-flat runs,
//! $1M prices with tiny moves, moves around 1e-4, prices around 1e-4 and alternating series, with
//! non-constant ranges and volumes, with and without single- and multi-row whitespace gaps.
use super::*;

const N: usize = 1_300;
const REPAIR_FROM: usize = 1_021;

/// Kinds whose value at a row depends only on a bounded trailing window of source rows.
const WINDOW_KINDS: [TestKind; 32] = [
    TestKind::Sma,
    TestKind::Wma,
    TestKind::Hma,
    TestKind::Vwma,
    TestKind::StandardDeviation,
    TestKind::Cci,
    TestKind::WilliamsR,
    TestKind::Momentum,
    TestKind::RateOfChange,
    TestKind::Donchian,
    TestKind::Ichimoku,
    TestKind::Bollinger,
    TestKind::BollingerMetrics,
    TestKind::EnvelopesSma,
    TestKind::Alma,
    TestKind::Stochastic,
    TestKind::Aroon,
    TestKind::AwesomeOscillator,
    TestKind::Dpo,
    TestKind::ChandeMomentum,
    TestKind::RelativeVolume,
    TestKind::EaseOfMovement,
    TestKind::HistoricalVolatility,
    TestKind::Kst,
    TestKind::LinearRegression,
    TestKind::Choppiness,
    TestKind::CoppockCurve,
    TestKind::UltimateOscillator,
    TestKind::Vortex,
    TestKind::Cmf,
    TestKind::Mfi,
    TestKind::Volume,
];

/// Window kinds fed by a recursive stage (RSI, EMA or adaptive smoothing). Their value depends on
/// all history, so no finite-window reference exists; every path must still match dense.
const RECURSIVE_WINDOW_KINDS: [TestKind; 4] = [
    TestKind::StochasticRsi,
    TestKind::MassIndex,
    TestKind::Kama,
    TestKind::FisherTransform,
];

struct Bars {
    times: Vec<i64>,
    close: Vec<f64>,
    high: Vec<f64>,
    low: Vec<f64>,
    volume: Vec<f64>,
}

impl Bars {
    fn input(&self, range: std::ops::RangeInclusive<usize>) -> IndicatorInput<'_> {
        IndicatorInput {
            times: &self.times[range.clone()],
            open: &self.close[range.clone()],
            high: &self.high[range.clone()],
            low: &self.low[range.clone()],
            close: &self.close[range.clone()],
            volume: &self.volume[range],
            amount: &[],
        }
    }

    fn clear(&self, start: usize, row: usize) -> bool {
        self.close[start..=row]
            .iter()
            .all(|value| value.is_finite())
    }
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, bound: u64) -> u64 {
        self.next() % bound
    }

    fn signed_unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1_u64 << 53) as f64 * 2.0 - 1.0
    }
}

#[derive(Clone, Copy)]
enum Segment {
    /// Random walk with steps up to `step` and dyadic half-ranges up to `range`.
    Walk { step: f64, range: f64 },
    /// The previous close repeated exactly; symmetric dyadic half-ranges keep every midpoint equal.
    Flat,
    /// `level + k * tick`, `k` in -5..=5.
    Ticks { level: f64, tick: f64 },
    /// `level + tick` and `level - tick` on alternating rows.
    Alternate { level: f64, tick: f64 },
}

/// Builds a series from `(first_row, segment)` pieces. Half-ranges are multiples of `half_tick`
/// (a power of two), so `close ± half_range` is exact and a flat close has an exactly flat
/// midpoint even though its range changes on every row.
fn build(seed: u64, start: f64, half_tick: f64, pieces: &[(usize, Segment)]) -> Bars {
    let mut rng = Rng(seed);
    let mut close = Vec::with_capacity(N);
    let mut high = Vec::with_capacity(N);
    let mut low = Vec::with_capacity(N);
    let mut volume = Vec::with_capacity(N);
    let mut last = start;
    for row in 0..N {
        let segment = pieces
            .iter()
            .rev()
            .find(|(first, _)| *first <= row)
            .map(|(_, segment)| *segment)
            .unwrap();
        let (value, half_range) = match segment {
            Segment::Walk { step, range } => {
                let value = (last + rng.signed_unit() * step).max(start * 0.25);
                let ticks = (range / half_tick).max(1.0) as u64;
                (value, half_tick * (1 + rng.below(ticks)) as f64)
            }
            Segment::Flat => (last, half_tick * (1 + rng.below(8)) as f64),
            Segment::Ticks { level, tick } => (
                level + (rng.below(11) as f64 - 5.0) * tick,
                half_tick * (1 + rng.below(8)) as f64,
            ),
            Segment::Alternate { level, tick } => (
                level + if row % 2 == 0 { tick } else { -tick },
                half_tick * (1 + rng.below(8)) as f64,
            ),
        };
        last = value;
        close.push(value);
        high.push(value + half_range);
        low.push(value - half_range);
        volume.push(if row % 29 == 0 {
            0.0
        } else if rng.below(50) == 0 {
            1e6
        } else {
            (1 + rng.below(1_000)) as f64
        });
    }
    Bars {
        times: (0..N as i64).map(|row| row * 3_600).collect(),
        close,
        high,
        low,
        volume,
    }
}

fn shapes() -> Vec<(&'static str, Bars)> {
    use Segment::*;
    let large = Walk {
        step: 5.0,
        range: 10.0,
    };
    let million = Walk {
        step: 900.0,
        range: 400.0,
    };
    let million_large = Walk {
        step: 5_000.0,
        range: 900.0,
    };
    vec![
        (
            "flat after large moves",
            build(
                0x51ab_77e1,
                100.0,
                0.25,
                &[
                    (0, large),
                    (300, Flat),
                    (420, large),
                    (980, Flat),
                    (1_110, large),
                    (1_180, Flat),
                ],
            ),
        ),
        (
            "near-flat after large moves",
            build(
                0x2c6e_0f13,
                100.0,
                0.25,
                &[
                    (0, large),
                    (
                        300,
                        Ticks {
                            level: 100.0,
                            tick: 1e-8,
                        },
                    ),
                ],
            ),
        ),
        (
            "$1M tiny moves",
            build(
                0x7d41_9a05,
                1e6,
                0.25,
                &[
                    (0, million),
                    (
                        300,
                        Ticks {
                            level: 1e6,
                            tick: 1e-7,
                        },
                    ),
                ],
            ),
        ),
        (
            "moves around 1e-4",
            build(
                0x0b3f_c2d9,
                100.0,
                2f64.powi(-12),
                &[
                    (
                        0,
                        Walk {
                            step: 5.0,
                            range: 10.0,
                        },
                    ),
                    (
                        300,
                        Ticks {
                            level: 100.0,
                            tick: 1e-4,
                        },
                    ),
                ],
            ),
        ),
        (
            "prices around 1e-4",
            build(
                0x93e5_4471,
                100.0,
                2f64.powi(-24),
                &[
                    (
                        0,
                        Walk {
                            step: 5.0,
                            range: 2f64.powi(-21),
                        },
                    ),
                    (
                        300,
                        Ticks {
                            level: 1e-4,
                            tick: 1e-7,
                        },
                    ),
                ],
            ),
        ),
        (
            "alternating after large moves",
            build(
                0x6a09_e667,
                100.0,
                0.25,
                &[
                    (0, large),
                    (
                        300,
                        Alternate {
                            level: 100.0,
                            tick: 1.0,
                        },
                    ),
                ],
            ),
        ),
        (
            "$1M alternating",
            build(
                0xbb67_ae85,
                1e6,
                0.25,
                &[
                    (0, million_large),
                    (
                        300,
                        Alternate {
                            level: 1e6,
                            tick: 0.001,
                        },
                    ),
                ],
            ),
        ),
        (
            "$1M then flat",
            build(
                0x3c6e_f372,
                1e6,
                0.25,
                &[
                    (0, million_large),
                    (980, Flat),
                    (1_110, million_large),
                    (1_180, Flat),
                ],
            ),
        ),
    ]
}

fn gap_sets() -> [(&'static str, Vec<usize>); 3] {
    let mut multi = vec![2, 3];
    multi.extend(600..602);
    multi.extend(1_019..1_024);
    multi.extend(1_040..1_048);
    multi.extend(1_190..1_193);
    [
        ("no gaps", vec![]),
        ("single-row gaps", vec![457, 1_000, 1_023, 1_200]),
        ("multi-row gaps", multi),
    ]
}

fn with_gaps(bars: &Bars, rows: &[usize]) -> Bars {
    let blank = |values: &[f64]| {
        let mut values = values.to_vec();
        for &row in rows {
            values[row] = f64::NAN;
        }
        values
    };
    Bars {
        times: bars.times.clone(),
        close: blank(&bars.close),
        high: blank(&bars.high),
        low: blank(&bars.low),
        volume: bars.volume.clone(),
    }
}

/// The deleted-rows series: `bars` without its whitespace rows, and the source row of each kept
/// row.
fn without_gaps(bars: &Bars) -> (Bars, Vec<usize>) {
    let rows = (0..N)
        .filter(|&row| bars.close[row].is_finite())
        .collect::<Vec<_>>();
    let pick = |values: &[f64]| rows.iter().map(|&row| values[row]).collect::<Vec<_>>();
    let compacted = Bars {
        times: rows.iter().map(|&row| bars.times[row]).collect(),
        close: pick(&bars.close),
        high: pick(&bars.high),
        low: pick(&bars.low),
        volume: pick(&bars.volume),
    };
    (compacted, rows)
}

/// Deleted-rows references scattered back to the `N` source rows; gap rows have no reference.
fn scatter(columns: Vec<Column>, rows: &[usize]) -> Vec<Column> {
    columns
        .into_iter()
        .map(|column| {
            let mut source = vec![None; N];
            for (value, &row) in column.into_iter().zip(rows) {
                source[row] = value;
            }
            source
        })
        .collect()
}

fn neumaier(values: impl IntoIterator<Item = f64>) -> f64 {
    let (mut sum, mut compensation) = (0.0_f64, 0.0_f64);
    for value in values {
        let total = sum + value;
        compensation += if sum.abs() >= value.abs() {
            (sum - total) + value
        } else {
            (value - total) + sum
        };
        sum = total;
    }
    sum + compensation
}

fn exact_mean(values: &[f64]) -> f64 {
    let mean = neumaier(values.iter().copied()) / values.len() as f64;
    mean + neumaier(values.iter().map(|value| value - mean)) / values.len() as f64
}

fn mean_abs(values: impl Iterator<Item = f64>) -> f64 {
    let values = values.collect::<Vec<_>>();
    values.iter().map(|value| value.abs()).sum::<f64>() / values.len() as f64
}

#[derive(Clone, Copy)]
struct Reference {
    value: f64,
    scale: f64,
}

type Column = Vec<Option<Reference>>;

fn exact(value: f64, scale: f64) -> Reference {
    Reference { value, scale }
}

/// Hand-written compensated references for the summation kinds. Each value is defined only when
/// its `span` rows are whitespace-free; [`audit`] passes the deleted-rows series, where they are.
fn handwritten(kind: TestKind, bars: &Bars) -> Option<Vec<Column>> {
    let (c, h, l, v) = (&bars.close, &bars.high, &bars.low, &bars.volume);
    let window = |span: usize, value: &dyn Fn(usize) -> Reference| -> Column {
        (0..bars.close.len())
            .map(|row| (row + 1 >= span && bars.clear(row + 1 - span, row)).then(|| value(row)))
            .collect()
    };
    let mean_of = |values: &[f64]| exact(exact_mean(values), mean_abs(values.iter().copied()));
    Some(match kind {
        TestKind::Sma => vec![window(5, &|row| mean_of(&c[row - 4..=row]))],
        TestKind::Wma => vec![window(5, &|row| {
            exact(
                neumaier((0..5).map(|i| (i + 1) as f64 * c[row - 4 + i])) / 15.0,
                mean_abs(c[row - 4..=row].iter().copied()),
            )
        })],
        TestKind::Vwma => vec![window(5, &|row| {
            let rows = row - 4..=row;
            let volume = neumaier(rows.clone().map(|i| v[i].max(0.0)));
            let value = if volume > 0.0 {
                neumaier(rows.clone().map(|i| c[i] * v[i].max(0.0))) / volume
            } else {
                exact_mean(&c[rows.clone()])
            };
            exact(value, mean_abs(c[rows].iter().copied()))
        })],
        TestKind::StandardDeviation => vec![window(5, &|row| {
            let values = &c[row - 4..=row];
            let mean = exact_mean(values);
            exact(
                (neumaier(values.iter().map(|value| (value - mean).powi(2))) / 5.0).sqrt(),
                mean.abs(),
            )
        })],
        TestKind::Dpo => vec![window(5, &|row| {
            let mean = mean_of(&c[row - 4..=row]);
            exact(c[row - 3] - mean.value, mean.scale)
        })],
        TestKind::ChandeMomentum => vec![window(6, &|row| {
            let deltas = (row - 4..=row).map(|i| c[i] - c[i - 1]).collect::<Vec<_>>();
            let signed = neumaier(deltas.iter().copied());
            let absolute = neumaier(deltas.iter().map(|delta| delta.abs()));
            exact(
                if absolute == 0.0 {
                    0.0
                } else {
                    100.0 * signed / absolute
                },
                100.0,
            )
        })],
        TestKind::EaseOfMovement => vec![window(6, &|row| {
            let raws = (row - 4..=row)
                .map(|i| {
                    if v[i].max(0.0) == 0.0 {
                        f64::NAN
                    } else {
                        ((h[i] + l[i]) * 0.5 - (h[i - 1] + l[i - 1]) * 0.5) * (h[i] - l[i]) * 100.0
                            / v[i]
                    }
                })
                .collect::<Vec<_>>();
            if raws.iter().all(|raw| raw.is_finite()) {
                exact(
                    neumaier(raws.iter().copied()) / 5.0,
                    mean_abs(raws.into_iter()),
                )
            } else {
                exact(f64::NAN, 0.0)
            }
        })],
        TestKind::HistoricalVolatility => vec![window(6, &|row| {
            let returns = (row - 4..=row)
                .map(|i| c[i].ln() - c[i - 1].ln())
                .collect::<Vec<_>>();
            let mean = exact_mean(&returns);
            let squares = neumaier(returns.iter().map(|value| (value - mean).powi(2)));
            let factor = 252.0_f64.sqrt() * 100.0;
            exact(
                (squares / 4.0).max(0.0).sqrt() * factor,
                mean.abs() * factor,
            )
        })],
        TestKind::Cmf => vec![window(5, &|row| {
            let rows = row - 4..=row;
            let flow = neumaier(rows.clone().map(|i| {
                let range = h[i] - l[i];
                let location = if range != 0.0 {
                    ((c[i] - l[i]) - (h[i] - c[i])) / range
                } else {
                    0.0
                };
                location * v[i].max(0.0)
            }));
            let volume = neumaier(rows.map(|i| v[i].max(0.0)));
            exact(if volume > 0.0 { flow / volume } else { 0.0 }, 1.0)
        })],
        TestKind::Mfi => vec![window(6, &|row| {
            let typical = |i: usize| (h[i] + l[i] + c[i]) / 3.0;
            let (mut positive, mut negative) = (Vec::new(), Vec::new());
            for (i, volume) in v.iter().enumerate().take(row + 1).skip(row - 4) {
                let flow = typical(i) * volume.max(0.0);
                if typical(i) > typical(i - 1) {
                    positive.push(flow);
                } else if typical(i) < typical(i - 1) {
                    negative.push(flow);
                }
            }
            let (positive, negative) = (neumaier(positive), neumaier(negative));
            exact(
                if negative == 0.0 {
                    if positive == 0.0 { 50.0 } else { 100.0 }
                } else {
                    100.0 - 100.0 / (1.0 + positive / negative)
                },
                100.0,
            )
        })],
        TestKind::Choppiness => vec![window(6, &|row| {
            let rows = row - 4..=row;
            let highest = rows.clone().map(|i| h[i]).fold(f64::NEG_INFINITY, f64::max);
            let lowest = rows.clone().map(|i| l[i]).fold(f64::INFINITY, f64::min);
            let true_range = neumaier(rows.map(|i| {
                (h[i] - l[i])
                    .max((h[i] - c[i - 1]).abs())
                    .max((l[i] - c[i - 1]).abs())
            }));
            let span = highest - lowest;
            exact(
                if span <= 0.0 || true_range <= 0.0 {
                    f64::NAN
                } else {
                    (100.0 * (true_range / span).log10() / 5.0_f64.log10()).clamp(0.0, 100.0)
                },
                100.0,
            )
        })],
        TestKind::UltimateOscillator => vec![window(8, &|row| {
            let mut averages = [0.0; 3];
            for (average, period) in averages.iter_mut().zip([3, 5, 7]) {
                let rows = row + 1 - period..=row;
                let bottom = |i: usize| l[i].min(c[i - 1]);
                let pressure = neumaier(rows.clone().map(|i| c[i] - bottom(i)));
                let range = neumaier(rows.map(|i| h[i].max(c[i - 1]) - bottom(i)));
                if range <= 0.0 {
                    return exact(f64::NAN, 0.0);
                }
                *average = pressure / range;
            }
            exact(
                100.0 * (4.0 * averages[0] + 2.0 * averages[1] + averages[2]) / 7.0,
                100.0,
            )
        })],
        TestKind::Vortex => {
            let parts = |row: usize| {
                let rows = row - 4..=row;
                let plus = neumaier(rows.clone().map(|i| (h[i] - l[i - 1]).abs()));
                let minus = neumaier(rows.clone().map(|i| (l[i] - h[i - 1]).abs()));
                let range = neumaier(rows.map(|i| {
                    (h[i] - l[i])
                        .max((h[i] - c[i - 1]).abs())
                        .max((l[i] - c[i - 1]).abs())
                }));
                if range <= 0.0 {
                    (f64::NAN, f64::NAN)
                } else {
                    (plus / range, minus / range)
                }
            };
            vec![
                window(6, &|row| exact(parts(row).0, 1.0)),
                window(6, &|row| exact(parts(row).1, 1.0)),
            ]
        }
        TestKind::Volume => vec![
            window(1, &|row| exact(v[row].max(0.0), 0.0)),
            window(5, &|row| {
                let volumes = v[row - 4..=row]
                    .iter()
                    .map(|value| value.max(0.0))
                    .collect::<Vec<_>>();
                mean_of(&volumes)
            }),
        ],
        TestKind::RelativeVolume => vec![window(6, &|row| {
            let previous = neumaier(v[row - 5..row].iter().map(|value| value.max(0.0)));
            let value = if previous > 0.0 {
                v[row].max(0.0) * 5.0 / previous
            } else {
                f64::NAN
            };
            exact(value, value.abs())
        })],
        _ => return None,
    })
}

/// Number of source rows each output depends on: its warm-up length on whitespace-free data.
fn spans(kind: TestKind) -> Vec<usize> {
    let bars = build(
        0x1234_5678,
        100.0,
        0.25,
        &[(
            0,
            Segment::Walk {
                step: 1.0,
                range: 2.0,
            },
        )],
    );
    // The first Ultimate value substitutes the first close for the missing prior close, so its
    // warm-up is one row shorter than the span every later value depends on.
    let extra = usize::from(kind == TestKind::UltimateOscillator);
    expected(kind, bars.input(0..=N - 1))
        .iter()
        .map(|column| column.iter().position(Option::is_some).unwrap() + 1 + extra)
        .collect()
}

/// The kind recomputed from only the trailing rows each value depends on.
fn fresh_window(kind: TestKind, bars: &Bars) -> Vec<Column> {
    let rows = bars.close.len();
    let spans = spans(kind);
    let mut columns = vec![vec![None; rows]; spans.len()];
    let mut distinct = spans.clone();
    distinct.sort_unstable();
    distinct.dedup();
    for span in distinct {
        for start in 0..=rows.saturating_sub(span) {
            let row = start + span - 1;
            if !bars.clear(start, row) {
                continue;
            }
            let values = expected(kind, bars.input(start..=row));
            for (column, _) in spans.iter().enumerate().filter(|(_, s)| **s == span) {
                columns[column][row] = values[column][span - 1].map(|value| exact(value, 0.0));
            }
        }
    }
    columns
}

type Path = Vec<Vec<Option<f64>>>;

fn fresh_state(kind: TestKind) -> IncrementalState {
    all_test_states()
        .into_iter()
        .find(|(candidate, _)| *candidate == kind)
        .unwrap()
        .1
}

fn record(state: &IncrementalState, path: &mut Path, rows: std::ops::Range<usize>) {
    for (column, values) in path.iter_mut().enumerate() {
        let from = state.output_from(column);
        for row in rows.clone().filter(|&row| row >= from) {
            values[row] = Some(state.output(column)[row - from]);
        }
    }
}

fn paths(kind: TestKind, bars: &Bars) -> Vec<(&'static str, Path)> {
    let dense = expected(kind, bars.input(0..=N - 1));
    let columns = dense.len();
    let empty = || vec![vec![None; N]; columns];

    let mut state = fresh_state(kind);
    state.rebuild_from(bars.input(0..=N - 1), 0);
    let mut rebuild = empty();
    record(&state, &mut rebuild, 0..N);

    state.rebuild_from(bars.input(0..=N - 1), REPAIR_FROM);
    let mut repair = empty();
    record(&state, &mut repair, REPAIR_FROM..N);

    let mut live_state = fresh_state(kind);
    let mut live = empty();
    for row in 0..N {
        live_state.rebuild_from(bars.input(0..=row), row);
        record(&live_state, &mut live, row..row + 1);
    }

    // Every row first arrives as a provisional tip (a different bar, or whitespace) and is then
    // replaced by its final value, so each append follows a tip replacement.
    let mut tip_state = fresh_state(kind);
    let mut tip = empty();
    let mut scratch = with_gaps(bars, &[]);
    for row in 0..N {
        set_provisional_tip(&mut scratch, bars, row);
        tip_state.rebuild_from(scratch.input(0..=row), row);
        scratch.close[row] = bars.close[row];
        scratch.high[row] = bars.high[row];
        scratch.low[row] = bars.low[row];
        scratch.volume[row] = bars.volume[row];
        tip_state.rebuild_from(scratch.input(0..=row), row);
        record(&tip_state, &mut tip, row..row + 1);
    }
    vec![
        ("dense", dense),
        ("incremental rebuild", rebuild),
        ("historical repair", repair),
        ("live append", live),
        ("live tip replace", tip),
    ]
}

/// A provisional bar that differs from the final one: whitespace on every third row, otherwise a
/// wider bar around a moved close based on the latest finite close.
fn set_provisional_tip(scratch: &mut Bars, bars: &Bars, row: usize) {
    if row.is_multiple_of(3) {
        scratch.close[row] = f64::NAN;
        scratch.high[row] = f64::NAN;
        scratch.low[row] = f64::NAN;
        return;
    }
    let base = bars.close[..=row]
        .iter()
        .rev()
        .copied()
        .find(|value| value.is_finite())
        .unwrap_or(1.0);
    let close = base * 1.015_625;
    scratch.close[row] = close;
    scratch.high[row] = close * 1.031_25;
    scratch.low[row] = close * 0.968_75;
    scratch.volume[row] = bars.volume[row] * 2.0 + 7.0;
}

/// Kinds whose update may rewrite rows before the changed one: ZigZag repaints its unconfirmed
/// leg, and PivotPoints carries the current UTC day and the last completed day across rows.
const REPAINT_KINDS: [TestKind; 2] = [TestKind::PivotPoints, TestKind::ZigZag];

/// Apply one update's emitted suffix to engine-style output columns: rows before `output_from`
/// are kept, later rows are replaced.
fn apply(state: &IncrementalState, columns: &mut [Vec<f64>], rows: usize) {
    for (column, values) in columns.iter_mut().enumerate() {
        let from = state.output_from(column);
        values.truncate(from);
        values.extend_from_slice(state.output(column));
        assert_eq!(
            values.len(),
            rows,
            "column {column} emitted through row {rows}"
        );
    }
}

/// Every update path, applied to maintained columns, must equal dense on the current prefix after
/// each step. ZigZag must never re-emit a row before its last confirmed turning point.
fn repaint_audit(kind: TestKind, bars: &Bars, fail: &mut dyn FnMut(&str, String)) {
    let dense_prefix = |rows: usize, bars: &Bars| -> Vec<Vec<f64>> {
        expected(kind, bars.input(0..=rows - 1))
            .into_iter()
            .map(|column| column.into_iter().map(|v| v.unwrap_or(f64::NAN)).collect())
            .collect()
    };
    let compare = |step: usize, actual: &[Vec<f64>], bars: &Bars| -> Option<String> {
        let rows = actual[0].len();
        for (column, (actual, expected)) in actual.iter().zip(dense_prefix(rows, bars)).enumerate()
        {
            for (row, (&actual, &expected)) in actual.iter().zip(&expected).enumerate() {
                let same = if kind == TestKind::ZigZag {
                    actual.to_bits() == expected.to_bits()
                } else {
                    within(actual, expected, 0.0)
                };
                if !same {
                    return Some(format!(
                        "after row {step}: col {column} row {row}: {actual:e} != {expected:e}"
                    ));
                }
            }
        }
        None
    };
    let check = |fail: &mut dyn FnMut(&str, String),
                 path: &str,
                 step: usize,
                 actual: &[Vec<f64>],
                 bars: &Bars| {
        if let Some(detail) = compare(step, actual, bars) {
            fail(path, detail);
        }
    };
    let last_confirmed = |columns: &[Vec<f64>]| {
        let mut turns = columns[0]
            .iter()
            .enumerate()
            .filter(|(_, value)| value.is_finite())
            .map(|(row, _)| row);
        let provisional = turns.next_back();
        provisional.and(turns.next_back())
    };
    let empty = || vec![Vec::new(); dense_prefix(N, bars).len()];

    let mut state = fresh_state(kind);
    let mut rebuilt = empty();
    state.rebuild_from(bars.input(0..=N - 1), 0);
    apply(&state, &mut rebuilt, N);
    check(fail, "incremental rebuild", N - 1, &rebuilt, bars);
    state.rebuild_from(bars.input(0..=N - 1), REPAIR_FROM);
    apply(&state, &mut rebuilt, N);
    check(fail, "historical repair", N - 1, &rebuilt, bars);

    let mut live_state = fresh_state(kind);
    let mut live = empty();
    let mut tip_state = fresh_state(kind);
    let mut tip = empty();
    let mut scratch = with_gaps(bars, &[]);
    for row in 0..N {
        let confirmed = last_confirmed(&live);
        live_state.rebuild_from(bars.input(0..=row), row);
        if kind == TestKind::ZigZag && confirmed.is_some_and(|c| live_state.output_from(0) < c) {
            fail(
                "live append",
                format!("row {row} re-emitted before {confirmed:?}"),
            );
        }
        apply(&live_state, &mut live, row + 1);
        check(fail, "live append", row, &live, bars);

        set_provisional_tip(&mut scratch, bars, row);
        tip_state.rebuild_from(scratch.input(0..=row), row);
        apply(&tip_state, &mut tip, row + 1);
        check(fail, "live provisional tip", row, &tip, &scratch);
        scratch.close[row] = bars.close[row];
        scratch.high[row] = bars.high[row];
        scratch.low[row] = bars.low[row];
        scratch.volume[row] = bars.volume[row];
        let confirmed = last_confirmed(&tip);
        tip_state.rebuild_from(scratch.input(0..=row), row);
        if kind == TestKind::ZigZag && confirmed.is_some_and(|c| tip_state.output_from(0) < c) {
            fail(
                "live tip replace",
                format!("row {row} re-emitted before {confirmed:?}"),
            );
        }
        apply(&tip_state, &mut tip, row + 1);
        check(fail, "live tip replace", row, &tip, bars);
    }
}

fn within(actual: f64, expected: f64, extra: f64) -> bool {
    if expected.is_nan() {
        return actual.is_nan();
    }
    actual.is_finite() && (actual - expected).abs() <= 1e-12_f64.max(1e-9 * expected.abs()) + extra
}

/// Checks every window kind on one fixture shape with each gap set. Shapes run as separate tests
/// so they execute in parallel.
fn audit(shape: usize) {
    let (shape, base) = shapes().swap_remove(shape);
    // (kind, path, reference) -> (mismatching rows, first mismatch)
    let mut failures = std::collections::BTreeMap::<String, (usize, String)>::new();
    let mut fail = |key: String, detail: String| {
        failures.entry(key).or_insert((0, detail)).0 += 1;
    };
    let mut compared = std::collections::BTreeMap::<String, usize>::new();
    {
        for (gaps, rows) in gap_sets() {
            let bars = with_gaps(&base, &rows);
            let (deleted, kept) = without_gaps(&bars);
            for kind in WINDOW_KINDS.into_iter().chain(RECURSIVE_WINDOW_KINDS) {
                let paths = paths(kind, &bars);
                let mut references = Vec::new();
                if let Some(columns) = handwritten(kind, &deleted) {
                    references.push(("compensated formula", scatter(columns, &kept), 1e-14));
                }
                if WINDOW_KINDS.contains(&kind) {
                    references.push((
                        "fresh window",
                        scatter(fresh_window(kind, &deleted), &kept),
                        0.0,
                    ));
                }
                let dense = paths[0].1.clone();
                for (path, values) in &paths {
                    // A gap row carries no sample on any path.
                    for (column, values) in values.iter().enumerate() {
                        for &row in &rows {
                            if values[row].is_some_and(|value| !value.is_nan()) {
                                fail(
                                    format!("{kind:?} {path} at a gap row"),
                                    format!(
                                        "[{shape}, {gaps}] col {column} row {row}: {:?}",
                                        values[row]
                                    ),
                                );
                            }
                        }
                    }
                    // Every path also follows the approved incremental == dense rule on every row.
                    for (column, expected) in dense.iter().enumerate() {
                        for (row, &expected) in expected.iter().enumerate() {
                            let (Some(expected), Some(actual)) = (expected, values[column][row])
                            else {
                                continue;
                            };
                            if !within(actual, expected, 0.0) {
                                fail(
                                    format!("{kind:?} {path} vs dense"),
                                    format!(
                                        "[{shape}, {gaps}] col {column} row {row}: {actual:e} != {expected:e}"
                                    ),
                                );
                            }
                        }
                    }
                    for (reference, columns, bound) in &references {
                        for (column, expected) in columns.iter().enumerate() {
                            for (row, expected) in expected.iter().enumerate() {
                                let Some(expected) = expected else { continue };
                                if *path != "historical repair" || row >= REPAIR_FROM {
                                    *compared
                                        .entry(format!("{kind:?} {reference}"))
                                        .or_default() += 1;
                                }
                                let actual = values[column][row];
                                let ok = match actual {
                                    Some(actual) => {
                                        within(actual, expected.value, bound * expected.scale)
                                    }
                                    None => *path == "historical repair" && row < REPAIR_FROM,
                                };
                                if !ok {
                                    fail(
                                        format!("{kind:?} {path} vs {reference}"),
                                        format!(
                                            "[{shape}, {gaps}] col {column} row {row}: {actual:?} != {:e}",
                                            expected.value
                                        ),
                                    );
                                }
                            }
                        }
                    }
                }
            }
            for kind in REPAINT_KINDS {
                repaint_audit(kind, &bars, &mut |path, detail| {
                    fail(
                        format!("{kind:?} {path}"),
                        format!("[{shape}, {gaps}] {detail}"),
                    );
                });
            }
        }
    }
    for kind in WINDOW_KINDS {
        assert!(
            compared
                .get(&format!("{kind:?} fresh window"))
                .is_some_and(|&rows| rows > 3 * N),
            "{kind:?} [{shape}] was not compared with its fresh-window reference"
        );
    }
    assert!(
        failures.is_empty(),
        "[{shape}] window values differ from their exact references:\n{}",
        failures
            .iter()
            .map(|(key, (rows, first))| format!("{key}: {rows} rows, first {first}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
fn window_kinds_are_exact_on_flat_runs_after_large_moves() {
    audit(0);
}

#[test]
fn window_kinds_are_exact_on_near_flat_runs_after_large_moves() {
    audit(1);
}

#[test]
fn window_kinds_are_exact_on_million_dollar_prices_with_tiny_moves() {
    audit(2);
}

#[test]
fn window_kinds_are_exact_on_moves_around_one_ten_thousandth() {
    audit(3);
}

#[test]
fn window_kinds_are_exact_on_prices_around_one_ten_thousandth() {
    audit(4);
}

#[test]
fn window_kinds_are_exact_on_alternating_moves_after_large_moves() {
    audit(5);
}

#[test]
fn window_kinds_are_exact_on_alternating_million_dollar_prices() {
    audit(6);
}

#[test]
fn window_kinds_are_exact_on_flat_runs_after_million_dollar_moves() {
    audit(7);
}

/// A flat window has exactly zero movement, so ChandeMomentum, EaseOfMovement and
/// HistoricalVolatility must be exactly 0 on every path, however large the earlier moves were.
#[test]
fn flat_windows_after_moves_larger_than_the_price_are_exactly_zero_on_every_path() {
    let mut rng = Rng(0x0dd_ba11);
    let levels = [1.0_f64, 100.0, 1e6];
    for level in levels.into_iter().cycle().take(48) {
        let mut close = Vec::new();
        for row in 0..120 {
            // Moves larger than the price make the window sums coarser than the price grid, so
            // an add/subtract sum cannot return to exactly zero once they leave the window.
            close.push(if row < 60 {
                level * (1.0 + 2.0 * (row % 2) as f64 + rng.below(1_000) as f64 * 1e-3)
            } else {
                level * 1.234_567
            });
        }
        let high = close
            .iter()
            .enumerate()
            .map(|(row, close)| close + 0.25 * (1 + row % 3) as f64)
            .collect::<Vec<_>>();
        let low = close
            .iter()
            .enumerate()
            .map(|(row, close)| close - 0.25 * (1 + row % 3) as f64)
            .collect::<Vec<_>>();
        let volume = (0..120).map(|row| (row % 7 + 1) as f64).collect::<Vec<_>>();
        let times = (0..120).collect::<Vec<i64>>();
        let input = IndicatorInput {
            times: &times,
            open: &close,
            high: &high,
            low: &low,
            close: &close,
            volume: &volume,
            amount: &[],
        };
        for kind in [
            TestKind::ChandeMomentum,
            TestKind::EaseOfMovement,
            TestKind::HistoricalVolatility,
        ] {
            let dense = expected(kind, input).remove(0);
            let mut state = fresh_state(kind);
            for (path, from) in [("rebuild", 0), ("repair", 62)] {
                state.rebuild_from(input, from);
                for (row, value) in dense.iter().enumerate().skip(70) {
                    let incremental = state.output(0)[row - state.output_from(0)];
                    assert_eq!(*value, Some(0.0), "{kind:?} level {level} dense row {row}");
                    assert_eq!(incremental, 0.0, "{kind:?} level {level} {path} row {row}");
                }
            }
            for row in 70..120 {
                // Install the rows before `row`, then append `row` as a live update.
                let prefix = |end: usize| IndicatorInput {
                    times: &times[..end],
                    open: &close[..end],
                    high: &high[..end],
                    low: &low[..end],
                    close: &close[..end],
                    volume: &volume[..end],
                    amount: &[],
                };
                let mut live = fresh_state(kind);
                live.rebuild_from(prefix(row), 0);
                live.rebuild_from(prefix(row + 1), row);
                let value = live.output(0)[row - live.output_from(0)];
                assert_eq!(value, 0.0, "{kind:?} level {level} live row {row}");
            }
        }
    }
}

#[test]
fn window_exactness_covers_every_fixture_shape() {
    assert_eq!(shapes().len(), 8);
}
