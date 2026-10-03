//! The KLineChart row-stepping runtime through the public `IncrementalState` path.
//!
//! After every kind of edit a chart applies (appends, last-row replacement, replacement plus
//! append, historical repair, truncation, mid-series insertion, full rebuild, no change), with
//! whitespace rows appearing, disappearing, running together, leading the series and ending it, the
//! stitched output of every binding must equal, bit for bit (`to_bits`):
//!
//! 1. a fresh `IncrementalState` over the same series, and
//! 2. `Indicator::compute` over the series with its whitespace rows removed, scattered back with NaN
//!    at the whitespace rows (volume compacted by the same rule, a volume column shorter than the
//!    source padded with the template's missing-volume value).
//!
//! All 27 templates are row steppers and every one gets every check below; nothing is skipped. Each
//! template's test first binds it once, runs it over 3,000 rows and gives it one replace tick, which
//! must evaluate at most five rows (`assert_stepped`), so a template that replays its history on a
//! tick, or the runtime doing so for every template at once, is a hard failure in that template's
//! test and in `every_template_is_stepped`, never a skipped check.

use aeris_charts_indicators::klinechart::{Bars, Indicator, NAMES};
use aeris_charts_indicators::{IncrementalState, IndicatorInput};

/// Rows that a transfer buffer or the whitespace scratch may keep after `release_transfer_capacity`.
/// This integration test cannot import the crate's private `MAX_RETAINED_ROWS` (`src/lib.rs`), so it
/// keeps a copy that must equal it: the memory assertions below fail if the crate keeps more.
const MAX_RETAINED_ROWS: usize = 65_536;
/// Rows between two stored checkpoints (`CHECKPOINT_INTERVAL` of the crate).
const CHECKPOINT_INTERVAL: usize = 1_024;
/// Data regimes: walk, repeated closes with zero volume, flat, 1e-4..1e9 range, prices crossing
/// zero, overflow to infinity.
const REGIMES: u64 = 6;

/// Deterministic xorshift, so every failure names a reproducible seed.
struct Rng(u64);

impl Rng {
    fn new(seed: u64, salt: u64) -> Self {
        Self(0x9E37_79B9_7F4A_7C15 ^ seed.wrapping_mul(0xD6E8_FEB8_6659_FD93) ^ (salt << 32) | 1)
    }

    fn next_u64(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    fn below(&mut self, bound: usize) -> usize {
        (self.next_u64() % bound.max(1) as u64) as usize
    }
}

const WHITESPACE: [f64; 5] = [f64::NAN; 5];

/// One bar `[open, high, low, close, volume]` in the given data regime.
fn bar(rng: &mut Rng, regime: usize, previous_close: f64) -> [f64; 5] {
    let kind = rng.unit();
    match regime {
        1 => {
            // Repeated closes, flat bars and zero volume: every zero-divisor branch.
            let close = (previous_close + (rng.below(3) as f64 - 1.0)).max(1.0);
            let open = if rng.below(2) == 0 {
                close
            } else {
                close + 1.0
            };
            let high = open.max(close) + rng.below(2) as f64;
            let low = open.min(close) - rng.below(2) as f64;
            let volume = if rng.below(3) == 0 {
                0.0
            } else {
                rng.below(50) as f64
            };
            [open, high, low, close, volume]
        }
        2 => [10.0, 10.0, 10.0, 10.0, 100.0],
        3 => {
            let magnitude = 10f64.powi(rng.below(14) as i32 - 4);
            let close = magnitude * (0.5 + rng.unit());
            [
                close * 0.99,
                close * 1.02,
                close * 0.97,
                close,
                magnitude * rng.unit() * 1e3,
            ]
        }
        4 => {
            let open = previous_close;
            let close = open + (rng.unit() - 0.5) * 6.0;
            let high = open.max(close) + rng.unit();
            let low = open.min(close) - rng.unit();
            let volume = (rng.unit() * 1000.0).floor() - if rng.unit() < 0.1 { 300.0 } else { 0.0 };
            [open, high, low, close, volume]
        }
        5 => {
            // Squares and products overflow to infinity; both sides must agree on every bit.
            let scale = 1e160;
            let open = if previous_close.abs() < 1e100 {
                100.0 * scale
            } else {
                previous_close
            };
            let close = open * (1.0 + (rng.unit() - 0.5) * 0.4);
            let high = open.max(close) * (1.0 + rng.unit() * 0.1);
            let low = open.min(close) * (1.0 - rng.unit() * 0.1);
            [open, high, low, close, scale * 100.0 * rng.unit()]
        }
        _ => {
            let open = previous_close + (rng.unit() - 0.5) * 2.0;
            if kind < 0.06 {
                return [
                    open,
                    open,
                    open,
                    open,
                    if rng.unit() < 0.5 { 0.0 } else { 100.0 },
                ];
            }
            let close = (open + (rng.unit() - 0.5) * if kind < 0.12 { 20.0 } else { 4.0 }).max(1.0);
            let volume = if rng.unit() < 0.05 {
                0.0
            } else {
                (500.0 + rng.unit() * 1500.0).floor()
            };
            [
                open,
                open.max(close) + rng.unit(),
                open.min(close) - rng.unit(),
                close,
                volume,
            ]
        }
    }
}

/// The columns a chart hands to a binding.
#[derive(Clone, Default)]
struct Data {
    open: Vec<f64>,
    high: Vec<f64>,
    low: Vec<f64>,
    close: Vec<f64>,
    volume: Vec<f64>,
    /// The volume column handed to the runtime ends here (a column shorter than the source).
    volume_rows: Option<usize>,
    /// The runtime gets no open column, so BRAR falls back to close.
    no_open: bool,
}

impl Data {
    fn len(&self) -> usize {
        self.close.len()
    }

    fn push(&mut self, row: [f64; 5]) {
        self.open.push(row[0]);
        self.high.push(row[1]);
        self.low.push(row[2]);
        self.close.push(row[3]);
        self.volume.push(row[4]);
    }

    fn set(&mut self, at: usize, row: [f64; 5]) {
        self.open[at] = row[0];
        self.high[at] = row[1];
        self.low[at] = row[2];
        self.close[at] = row[3];
        self.volume[at] = row[4];
    }

    fn insert(&mut self, at: usize, row: [f64; 5]) {
        self.open.insert(at, row[0]);
        self.high.insert(at, row[1]);
        self.low.insert(at, row[2]);
        self.close.insert(at, row[3]);
        self.volume.insert(at, row[4]);
    }

    fn truncate(&mut self, rows: usize) {
        self.open.truncate(rows);
        self.high.truncate(rows);
        self.low.truncate(rows);
        self.close.truncate(rows);
        self.volume.truncate(rows);
    }

    /// A whitespace row: no sample, only a time slot.
    fn is_whitespace(&self, row: usize) -> bool {
        self.close[row].is_nan() || self.high[row].is_nan() || self.low[row].is_nan()
    }

    fn last_close(&self) -> f64 {
        self.close
            .iter()
            .rev()
            .find(|close| close.is_finite())
            .copied()
            .unwrap_or(100.0)
    }

    fn input<'a>(&'a self, times: &'a [i64]) -> IndicatorInput<'a> {
        let rows = self.len();
        IndicatorInput {
            times: &times[..rows],
            open: if self.no_open { &[] } else { &self.open },
            high: &self.high,
            low: &self.low,
            close: &self.close,
            volume: &self.volume[..self.volume_rows.map_or(rows, |limit| limit.min(rows))],
            amount: &[],
        }
    }
}

/// A series of `rows` bars in a regime, with a few whitespace rows when asked.
fn series(rng: &mut Rng, regime: usize, rows: usize, whitespace: bool) -> Data {
    let mut data = Data::default();
    let mut previous = 100.0;
    for _ in 0..rows {
        let row = bar(rng, regime, previous);
        previous = row[3];
        data.push(row);
    }
    if whitespace {
        for row in 0..rows {
            if rng.unit() < 0.03 {
                data.set(row, WHITESPACE);
            }
        }
        // Leading rows: the warm-up of a chained indicator source.
        if rng.unit() < 0.4 {
            for row in 0..rng.below(6).min(rows) {
                data.set(row, WHITESPACE);
            }
        }
        // A run of consecutive gaps.
        if rng.unit() < 0.3 && rows > 40 {
            let at = rng.below(rows - 30);
            for row in at..at + 1 + rng.below(25) {
                data.set(row, WHITESPACE);
            }
        }
    }
    data
}

/// Every output of `indicator` over `data`, computed on the non-whitespace rows only and scattered
/// back with NaN at whitespace.
fn expected(indicator: &Indicator, data: &Data) -> Vec<Vec<f64>> {
    let rows = data.len();
    let valid: Vec<usize> = (0..rows).filter(|&row| !data.is_whitespace(row)).collect();
    let pick = |column: &[f64]| valid.iter().map(|&row| column[row]).collect::<Vec<_>>();
    let close = pick(&data.close);
    let open = if data.no_open {
        close.clone()
    } else {
        pick(&data.open)
    };
    let (high, low) = (pick(&data.high), pick(&data.low));
    let volume_rows = data.volume_rows.map_or(rows, |limit| limit.min(rows));
    let missing = indicator.missing_volume();
    let volume = valid
        .iter()
        .map(|&row| {
            if row < volume_rows {
                data.volume[row]
            } else {
                missing
            }
        })
        .collect::<Vec<_>>();
    let columns = indicator.compute(&Bars {
        open: &open,
        high: &high,
        low: &low,
        close: &close,
        volume: &volume,
        // An AVP binding's source series carries turnover as its value.
        turnover: &close,
    });
    (0..indicator.output_count())
        .map(|output| {
            let mut values = vec![f64::NAN; rows];
            for (compact, &row) in valid.iter().enumerate() {
                values[row] = columns[output][compact]
                    .filter(|value| value.is_finite())
                    .unwrap_or(f64::NAN);
            }
            values
        })
        .collect()
}

/// Rebuild `state` from `from` and stitch the suffix into `canonical` the way the engine does.
/// Returns the rows the rebuild evaluated.
fn rebuild(
    state: &mut IncrementalState,
    data: &Data,
    times: &[i64],
    from: usize,
    canonical: &mut [Vec<f64>],
) -> usize {
    state.rebuild_from(data.input(times), from);
    for (output, column) in canonical.iter_mut().enumerate() {
        let start = state.output_from(output);
        column.truncate(start);
        column.resize(start, f64::NAN);
        column.extend_from_slice(state.output(output));
        assert_eq!(
            column.len(),
            data.len(),
            "stitched length of output {output}"
        );
    }
    state.last_work_rows()
}

/// The outputs of a fresh binding over `data`.
fn fresh(indicator: &Indicator, data: &Data, times: &[i64]) -> Vec<Vec<f64>> {
    let mut state = IncrementalState::klinechart(indicator.clone());
    let mut canonical = vec![Vec::new(); state.output_count()];
    rebuild(&mut state, data, times, 0, &mut canonical);
    canonical
}

fn assert_bits(context: &str, got: &[Vec<f64>], want: &[Vec<f64>]) {
    assert_eq!(got.len(), want.len(), "{context}: output count");
    for (output, (got, want)) in got.iter().zip(want).enumerate() {
        assert_eq!(got.len(), want.len(), "{context}: output {output} length");
        for (row, (got, want)) in got.iter().zip(want).enumerate() {
            assert_eq!(
                got.to_bits(),
                want.to_bits(),
                "{context}: output {output} row {row}: got {got}, want {want}"
            );
        }
    }
}

#[derive(Clone, Copy, Default)]
struct Flags {
    whitespace: bool,
    short_volume: bool,
    no_open: bool,
}

/// A bar, or a whitespace row one time in `odds` when whitespace is enabled.
fn next_row(rng: &mut Rng, regime: usize, whitespace: bool, data: &Data, odds: usize) -> [f64; 5] {
    if whitespace && rng.below(odds) == 0 {
        WHITESPACE
    } else {
        bar(rng, regime, data.last_close())
    }
}

/// One random edit sequence over one series; compares after every `check_every`th edit.
fn run(
    indicator: &Indicator,
    label: &str,
    seed: u64,
    rows: usize,
    ops: usize,
    check_every: usize,
    flags: Flags,
) {
    let regime = (seed % REGIMES) as usize;
    let mut rng = Rng::new(seed, label.len() as u64);
    let rows = rows + rng.below(rows / 3 + 1);
    let mut data = series(&mut rng, regime, rows, flags.whitespace);
    data.no_open = flags.no_open;
    data.volume_rows = flags.short_volume.then(|| rows * 2 / 3);
    let times: Vec<i64> = (0..rows as i64 + 10_000).collect();
    let mut state = IncrementalState::klinechart(indicator.clone());
    let mut canonical = vec![Vec::new(); state.output_count()];
    rebuild(&mut state, &data, &times, 0, &mut canonical);
    let compare = |canonical: &[Vec<f64>], data: &Data, what: &str| {
        let context = format!("{label} seed {seed} regime {regime} {what}");
        assert_bits(
            &format!("{context} vs fresh"),
            canonical,
            &fresh(indicator, data, &times),
        );
        assert_bits(
            &format!("{context} vs compute"),
            canonical,
            &expected(indicator, data),
        );
    };
    compare(&canonical, &data, "initial rebuild");
    for op in 0..ops {
        let n = data.len();
        let pick = rng.unit();
        let ws = flags.whitespace;
        let (what, from) = if n < 4 || pick < 0.17 {
            for _ in 0..1 + rng.below(4) {
                let row = next_row(&mut rng, regime, ws, &data, 16);
                data.push(row);
            }
            ("append", n)
        } else if pick < 0.28 {
            let row = next_row(&mut rng, regime, ws, &data, 4);
            data.set(n - 1, row);
            ("replace last", n - 1)
        } else if pick < 0.38 {
            let row = next_row(&mut rng, regime, ws, &data, 4);
            data.set(n - 1, row);
            for _ in 0..1 + rng.below(3) {
                let row = next_row(&mut rng, regime, ws, &data, 6);
                data.push(row);
            }
            ("replace last and append", n - 1)
        } else if pick < 0.46 {
            let at = n - 1 - rng.below(12.min(n));
            let row = next_row(&mut rng, regime, ws, &data, 3);
            data.set(at, row);
            if rng.below(2) == 0 {
                let row = next_row(&mut rng, regime, ws, &data, 6);
                data.push(row);
            }
            ("repair in the last 12 rows", at)
        } else if pick < 0.56 {
            let at = rng.below(n);
            let row = next_row(&mut rng, regime, ws, &data, 3);
            data.set(at, row);
            ("historical repair", at)
        } else if pick < 0.61 {
            let at = if n > CHECKPOINT_INTERVAL + 2 {
                (CHECKPOINT_INTERVAL - 2 + rng.below(4)).min(n - 1)
            } else {
                rng.below(n)
            };
            let row = next_row(&mut rng, regime, ws, &data, 3);
            data.set(at, row);
            ("repair around a checkpoint", at)
        } else if pick < 0.68 {
            let keep = n - 1 - rng.below(3.min(n - 1));
            data.truncate(keep);
            ("truncate", keep)
        } else if pick < 0.73 {
            let keep = n - 1 - rng.below(3.min(n - 1));
            data.truncate(keep);
            let row = next_row(&mut rng, regime, ws, &data, 4);
            data.push(row);
            ("truncate and new last row", keep)
        } else if pick < 0.77 {
            let keep = (n - 1 - rng.below((n - 1).min(3 * CHECKPOINT_INTERVAL))).max(1);
            data.truncate(keep);
            let row = next_row(&mut rng, regime, ws, &data, 4);
            data.push(row);
            ("deep truncation", keep)
        } else if pick < 0.83 {
            let at = 1 + rng.below(n - 1);
            let row = next_row(&mut rng, regime, ws, &data, 6);
            data.insert(at, row);
            ("mid-series insertion", at)
        } else if pick < 0.90 {
            ("no change", n - rng.below(2))
        } else {
            ("full rebuild", 0)
        };
        rebuild(&mut state, &data, &times, from, &mut canonical);
        if (op + 1) % check_every == 0 || op + 1 == ops {
            compare(
                &canonical,
                &data,
                &format!("after op {op} ({what} from row {from})"),
            );
        }
    }
}

/// Rows the probe binds a template over.
const PROBE_ROWS: usize = 3_000;

/// Rows a replace tick may evaluate on a stepped template: the replaced row itself, with slack.
const STEPPED_WORK_ROWS: usize = 5;

/// Probe: the rows a replace tick over [`PROBE_ROWS`] rows evaluates. The whole series (or any
/// replay of the history) means the template is not stepping.
fn probe_work(indicator: &Indicator) -> usize {
    let rows = PROBE_ROWS;
    let mut rng = Rng::new(11, 0);
    let mut data = series(&mut rng, 0, rows, false);
    let times: Vec<i64> = (0..rows as i64 + 10).collect();
    let mut state = IncrementalState::klinechart(indicator.clone());
    let mut canonical = vec![Vec::new(); state.output_count()];
    rebuild(&mut state, &data, &times, 0, &mut canonical);
    let row = bar(&mut rng, 0, data.close[rows - 2]);
    data.set(rows - 1, row);
    rebuild(&mut state, &data, &times, rows - 1, &mut canonical)
}

/// Fails unless `indicator` steps: a replace tick over [`PROBE_ROWS`] rows evaluates at most
/// [`STEPPED_WORK_ROWS`] rows.
fn assert_stepped(name: &str, indicator: &Indicator) {
    let work = probe_work(indicator);
    assert!(
        work <= STEPPED_WORK_ROWS,
        "{name} evaluated {work} of {PROBE_ROWS} rows on a replace tick; a stepped template \
         evaluates at most {STEPPED_WORK_ROWS}"
    );
}

/// Rows a step can read back, bounded from the public parameters: every lookback is at most twice
/// the sum of the template's periods (CR adds its shift, at most the period itself).
fn reach(indicator: &Indicator) -> usize {
    2 * indicator
        .params()
        .iter()
        .filter(|param| param.integer)
        .map(|param| param.value as usize)
        .sum::<usize>()
        + 8
}

/// Parameter sets per template: KLineChart's default, period 1, 2 and 3, swapped short/long where
/// both exist, a period above the series rows, one above the 1,024-row checkpoint interval, and
/// small custom lists.
fn variants(name: &str) -> Vec<Indicator> {
    let sets: &[&[f64]] = match name {
        "MA" | "EMA" | "RSI" | "BIAS" | "WR" => &[
            &[],
            &[1.0],
            &[2.0, 3.0],
            &[1.0, 2.0, 3.0, 4.0, 5.0],
            &[400.0],
            &[7.0, 1100.0],
        ],
        "VOL" => &[
            &[],
            &[1.0],
            &[2.0, 3.0],
            &[1.0, 2.0, 3.0, 4.0],
            &[400.0],
            &[3.0, 1100.0],
        ],
        "SMA" => &[
            &[12.0, 2.0],
            &[1.0, 1.0],
            &[2.0, 0.5],
            &[3.0, 3.5],
            &[400.0, 2.0],
            &[1100.0, 1.0],
        ],
        "BBI" => &[
            &[3.0, 6.0, 12.0, 24.0],
            &[1.0, 1.0, 1.0, 1.0],
            &[1.0, 2.0, 3.0, 4.0],
            &[24.0, 12.0, 6.0, 3.0],
            &[400.0, 2.0, 3.0, 5.0],
            &[1100.0, 1.0, 2.0, 3.0],
        ],
        "MACD" | "DMA" => &[
            &[],
            &[1.0, 1.0, 1.0],
            &[2.0, 3.0, 2.0],
            &[50.0, 10.0, 10.0],
            &[400.0, 5.0, 3.0],
            &[3.0, 1100.0, 4.0],
            &[3.0, 5.0, 1100.0],
        ],
        "BOLL" => &[
            &[20.0, 2.0],
            &[1.0, 2.0],
            &[2.0, 0.0],
            &[3.0, 1.5],
            &[400.0, 2.0],
            &[1100.0, 2.0],
        ],
        "KDJ" => &[
            &[9.0, 3.0, 3.0],
            &[1.0, 1.0, 1.0],
            &[2.0, 3.0, 1.0],
            &[3.0, 2.0, 2.0],
            &[400.0, 3.0, 3.0],
            &[1100.0, 3.0, 3.0],
        ],
        "BRAR" | "CCI" | "EMV" | "OBV" => &[&[], &[1.0], &[2.0], &[3.0], &[400.0], &[1100.0]],
        "CR" => &[
            &[26.0, 10.0, 20.0, 40.0, 60.0],
            &[1.0, 1.0, 1.0, 1.0, 1.0],
            &[2.0, 1.0, 2.0, 3.0, 4.0],
            &[3.0, 2.0, 3.0, 5.0, 7.0],
            &[40.0, 60.0, 40.0, 20.0, 10.0],
            &[400.0, 5.0, 6.0, 7.0, 8.0],
            &[2.0, 1100.0, 3.0, 1.0, 9.0],
        ],
        "DMI" | "MTM" | "PSY" | "ROC" | "TRIX" | "VR" | "AO" => &[
            &[],
            &[1.0, 1.0],
            &[2.0, 3.0],
            &[3.0, 2.0],
            &[400.0, 6.0],
            &[3.0, 1100.0],
            &[1100.0, 3.0],
        ],
        "SAR" => &[
            &[2.0, 2.0, 20.0],
            &[1.0, 1.0, 1.0],
            &[50.0, 50.0, 100.0],
            &[0.5, 0.5, 5.0],
            &[20.0, 20.0, 20.0],
        ],
        "PVT" | "AVP" => &[&[]],
        other => panic!("unknown template {other}"),
    };
    sets.iter()
        .map(|params| {
            if params.is_empty() {
                Indicator::from_name(name).expect("template")
            } else {
                Indicator::from_calc_params(name, params)
                    .unwrap_or_else(|| panic!("{name} {params:?} is not a valid binding"))
            }
        })
        .collect()
}

fn label(name: &str, indicator: &Indicator) -> String {
    format!("{name} {:?}", indicator.calc_params())
}

/// The work and memory a tick may cost on a long history.
fn work_bound(indicator: &Indicator, label: &str) {
    let rows = 100_000;
    let reach = reach(indicator);
    let mut rng = Rng::new(7, 3);
    let times: Vec<i64> = (0..rows as i64 + 8).collect();

    for gap in [None, Some(500)] {
        let mut data = series(&mut rng, 0, rows, false);
        if let Some(row) = gap {
            data.set(row, WHITESPACE);
        }
        // One tick costs the window (plus the rows a whitespace-compacted window re-reads), never
        // the history.
        let tick_limit = if gap.is_some() { reach + 2 } else { 2 };
        let context = format!(
            "{label} with {} whitespace row(s) in history",
            usize::from(gap.is_some())
        );
        let mut state = IncrementalState::klinechart(indicator.clone());
        let mut canonical = vec![Vec::new(); state.output_count()];
        let work = rebuild(&mut state, &data, &times, 0, &mut canonical);
        if gap.is_none() {
            assert_eq!(work, rows, "{context}: a full rebuild evaluates every row");
        } else {
            // Each replay chunk after the gap also re-reads the rows its window looks back over.
            let chunks = rows / 4_096 + 2;
            assert!(
                work >= rows && work <= rows + chunks * (reach + 1),
                "{context}: a full rebuild evaluated {work} rows"
            );
        }

        let row = bar(&mut rng, 0, data.close[rows - 2]);
        data.set(rows - 1, row);
        let work = rebuild(&mut state, &data, &times, rows - 1, &mut canonical);
        assert!(
            work <= tick_limit,
            "{context}: replace evaluated {work} rows"
        );

        let row = bar(&mut rng, 0, data.close[rows - 1]);
        data.push(row);
        let work = rebuild(&mut state, &data, &times, rows, &mut canonical);
        assert!(
            work <= tick_limit,
            "{context}: append evaluated {work} rows"
        );

        let row = bar(&mut rng, 0, data.close[rows]);
        data.set(rows, row);
        let row = bar(&mut rng, 0, data.close[rows]);
        data.push(row);
        let work = rebuild(&mut state, &data, &times, rows, &mut canonical);
        assert!(
            work <= tick_limit + 1,
            "{context}: replace and append evaluated {work} rows"
        );

        // A historical repair replays the changed rows plus at most one checkpoint interval.
        let at = rows / 2;
        let row = bar(&mut rng, 0, 100.0);
        data.set(at, row);
        let work = rebuild(&mut state, &data, &times, at, &mut canonical);
        let replayed = data.len() - at;
        let chunks = replayed / 4_096 + 2;
        let slack = if gap.is_some() {
            CHECKPOINT_INTERVAL + chunks * (reach + 1)
        } else {
            CHECKPOINT_INTERVAL
        };
        assert!(
            work >= replayed && work < replayed + slack,
            "{context}: repair of {replayed} rows evaluated {work}"
        );
        assert_bits(&context, &canonical, &expected(indicator, &data));

        // Checkpoints (at most 256 bytes each, a growing Vec holds at most twice its length) plus,
        // with whitespace in history, the compacted window of one replay chunk.
        let bytes = state.runtime_bytes();
        let checkpoints = 2 * (rows / CHECKPOINT_INTERVAL + 1) * 256;
        let scratch = if gap.is_some() {
            2 * 5 * 8 * (4_096 + reach)
        } else {
            0
        };
        assert!(
            bytes > 0 && bytes <= checkpoints + scratch,
            "{context}: {bytes} runtime bytes"
        );

        state.release_transfer_capacity();
        assert!(
            state.transfer_capacity_bytes() <= state.output_count() * MAX_RETAINED_ROWS * 8,
            "{context}: transfer capacity after release"
        );
    }
}

/// The whitespace scratch of a very long window is trimmed by `release_transfer_capacity`.
fn release_trims_scratch(indicator: &Indicator, label: &str) {
    let rows = 120_000;
    let mut rng = Rng::new(5, 4);
    let times: Vec<i64> = (0..rows as i64 + 8).collect();
    let mut data = series(&mut rng, 0, rows, false);
    data.set(10, WHITESPACE);
    let mut state = IncrementalState::klinechart(indicator.clone());
    let mut canonical = vec![Vec::new(); state.output_count()];
    rebuild(&mut state, &data, &times, 0, &mut canonical);
    let row = bar(&mut rng, 0, data.close[rows - 2]);
    data.set(rows - 1, row);
    rebuild(&mut state, &data, &times, rows - 1, &mut canonical);
    let before = state.runtime_bytes();
    assert!(
        before > 5 * MAX_RETAINED_ROWS * 8,
        "{label}: a {}-row window should grow the scratch past the cap, runtime {before} bytes",
        reach(indicator)
    );
    state.release_transfer_capacity();
    let history_bound = 2 * (rows / CHECKPOINT_INTERVAL + 1) * 256;
    let after = state.runtime_bytes();
    assert!(
        after <= history_bound + 5 * MAX_RETAINED_ROWS * 8,
        "{label}: runtime {after} bytes after release (was {before})"
    );
}

fn check(name: &str) {
    let default = Indicator::from_name(name).expect("template");
    assert_stepped(name, &default);
    for indicator in variants(name) {
        let label = label(name, &indicator);
        for seed in 1..=24 {
            run(&indicator, &label, seed, 90, 30, 1, Flags::default());
        }
        for seed in 1..=36 {
            let flags = Flags {
                whitespace: true,
                ..Flags::default()
            };
            run(&indicator, &label, seed, 90, 30, 1, flags);
        }
        // A series about 9,300 rows long with few edits, so repairs and truncations cross several
        // checkpoints and two replay chunks.
        if indicator == default || reach(&indicator) > 2_000 {
            for (seed, whitespace) in [(6, false), (12, true), (1, true), (5, false)] {
                let flags = Flags {
                    whitespace,
                    ..Flags::default()
                };
                run(&indicator, &label, seed, 9_300, 48, 16, flags);
            }
        }
    }
    let label = label(name, &default);
    if default.needs_volume() {
        for seed in 1..=6 {
            for whitespace in [false, true] {
                let flags = Flags {
                    whitespace,
                    short_volume: true,
                    ..Flags::default()
                };
                run(&default, &label, seed, 90, 30, 1, flags);
            }
        }
    }
    if name == "BRAR" {
        for seed in 1..=6 {
            let flags = Flags {
                whitespace: seed % 2 == 0,
                no_open: true,
                ..Flags::default()
            };
            run(&default, &label, seed, 90, 30, 1, flags);
        }
    }
    // An empty source.
    let empty = Data::default();
    let times = [0_i64; 1];
    let mut state = IncrementalState::klinechart(default.clone());
    let mut canonical = vec![Vec::new(); state.output_count()];
    rebuild(&mut state, &empty, &times, 0, &mut canonical);
    assert!(canonical.iter().all(Vec::is_empty), "{label}: empty source");
    work_bound(&default, &label);
}

macro_rules! templates {
    ($($test:ident => $name:literal),* $(,)?) => {
        $(#[test] fn $test() { check($name); })*

        /// Every KLineChart template has its line in this file.
        #[test]
        fn every_template_has_a_test() {
            let tested = [$($name),*];
            assert_eq!(tested.len(), NAMES.len());
            for name in NAMES {
                assert!(tested.contains(&name), "{name} has no test");
            }
        }
    };
}

templates! {
    ma => "MA", ema => "EMA", sma => "SMA", boll => "BOLL", sar => "SAR", bbi => "BBI",
    avp => "AVP", vol => "VOL", macd => "MACD", kdj => "KDJ", rsi => "RSI", bias => "BIAS",
    brar => "BRAR", cci => "CCI", dmi => "DMI", cr => "CR", psy => "PSY", dma => "DMA",
    trix => "TRIX", obv => "OBV", vr => "VR", wr => "WR", mtm => "MTM", emv => "EMV",
    roc => "ROC", pvt => "PVT", ao => "AO",
}

/// The scratch a long window keeps is bounded, for the one template whose period can be that long
/// on every formula (the moving average).
#[test]
fn a_long_whitespace_window_does_not_stay_resident() {
    assert_stepped("MA", &Indicator::from_name("MA").expect("template"));
    let indicator = Indicator::Ma {
        periods: vec![100_000],
    };
    release_trims_scratch(&indicator, "MA [100000]");
}

/// Every KLineChart template steps: a replace tick evaluates a handful of rows, not the history.
#[test]
fn every_template_is_stepped() {
    for name in NAMES {
        assert_stepped(name, &Indicator::from_name(name).expect("template"));
    }
}
