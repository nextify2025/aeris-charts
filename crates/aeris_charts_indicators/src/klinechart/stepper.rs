//! Row steppers: how a bound KLineChart indicator advances one non-whitespace row at a time.
//!
//! Every one of the 27 formula files exports a `State` (`Copy`, scalars and fixed-size arrays only)
//! and a `step` that consumes one valid row. [`KlRuntime`] replays rows through [`step_row`] from
//! the checkpointed state held in the same sparse [`RecursiveHistory`] the built-in studies use, so
//! a tick costs the formula's window, not the history. Whitespace rows (NaN close, high or low) are
//! skipped in place: they emit NaN and never reach a step. The public whole-series functions are
//! folds of the same steps over finite input, which keeps the arithmetic the runtime executes
//! pinned by the KLineChart parity fixtures.
//!
//! The invariant everything rests on: the state after source row `c` is a pure function of the
//! valid (non-whitespace) rows `0..=c`. A step reads only the [`Window`] (rows re-read from the
//! current input) and the checkpointed state, so replaying from any checkpoint equals a fresh run
//! after any edit. A formula that needs one of its own earlier outputs (a moving average of its
//! primary line) keeps a lagging copy of the stage that produces it inside its own `State`; it never
//! reads a ring or an engine output.
//!
//! This file is Aeris code, not a KLineChart translation; the formula files keep the attribution.

use super::{
    average_price, awesome_oscillator, bias, bollinger_bands, brar, bull_and_bear_index,
    commodity_channel_index, current_ratio, different_of_moving_average,
    directional_movement_index, ease_of_movement_value, exponential_moving_average, momentum,
    moving_average, moving_average_convergence_divergence, on_balance_volume,
    price_and_volume_trend, psychological_line, rate_of_change, relative_strength_index,
    simple_moving_average, stoch, stop_and_reverse, triple_exponentially_smoothed_average, volume,
    volume_ratio, williams_r, Column, Indicator,
};
use crate::{
    valid_lookback_start, whitespace_row, IndicatorInput, RecursiveHistory, MAX_OUTPUTS,
    MAX_RETAINED_ROWS,
};

/// Source rows per replay chunk. A chunk is the unit of the compacted whitespace window, so the
/// transient memory of a replay is bounded by one chunk plus the formula's lookback per column.
pub(super) const REPLAY_CHUNK: usize = 4096;

/// One row's result: a step sets `out[k] = Some(value)` only when output `k` has a value on this
/// row.
pub(super) type Out = [Option<f64>; MAX_OUTPUTS];

/// A read-only view of valid (non-whitespace) rows addressed by VALID-row index `k`: the index of a
/// row among the rows that carry a sample, so a formula never sees whitespace. `base` is the valid
/// index of element 0 of every slice.
///
/// Every accessor subtracts `base` with `checked_sub(..).expect("klinechart lookback")`, so a read
/// before the formula's declared `lookback` panics in every build profile instead of wrapping to a
/// wrong row in release. A reader of rows `[a, b)` passes the half-open range, like a slice.
pub(super) struct Window<'a> {
    pub open: &'a [f64],
    pub high: &'a [f64],
    pub low: &'a [f64],
    pub close: &'a [f64],
    /// May be shorter than the rows: a missing row reads `missing_volume`.
    pub volume: &'a [f64],
    pub base: usize,
    /// `Indicator::missing_volume`: 1.0 for PVT, else 0.0.
    pub missing_volume: f64,
}

impl Window<'_> {
    /// A window over nothing, so a public function builds the one it reads with
    /// `Window { close, ..Window::EMPTY }`.
    pub const EMPTY: Window<'static> = Window {
        open: &[],
        high: &[],
        low: &[],
        close: &[],
        volume: &[],
        base: 0,
        missing_volume: 0.0,
    };

    #[inline]
    fn at(&self, k: usize) -> usize {
        k.checked_sub(self.base).expect("klinechart lookback")
    }

    #[inline]
    pub fn o(&self, k: usize) -> f64 {
        self.open[self.at(k)]
    }

    #[inline]
    pub fn h(&self, k: usize) -> f64 {
        self.high[self.at(k)]
    }

    #[inline]
    pub fn l(&self, k: usize) -> f64 {
        self.low[self.at(k)]
    }

    #[inline]
    pub fn c(&self, k: usize) -> f64 {
        self.close[self.at(k)]
    }

    /// Volume of valid row `k`, or the template's missing-volume value past the end of a column
    /// shorter than the rows.
    #[inline]
    pub fn v(&self, k: usize) -> f64 {
        self.volume
            .get(self.at(k))
            .copied()
            .unwrap_or(self.missing_volume)
    }

    /// Turnover of valid row `k`: an AVP binding's source series carries turnover as its value, so
    /// it is the close column.
    #[inline]
    pub fn t(&self, k: usize) -> f64 {
        self.c(k)
    }

    /// Highs of valid rows `[a, b)`.
    #[inline]
    pub fn highs(&self, a: usize, b: usize) -> &[f64] {
        &self.high[self.at(a)..self.at(b)]
    }

    /// Lows of valid rows `[a, b)`.
    #[inline]
    pub fn lows(&self, a: usize, b: usize) -> &[f64] {
        &self.low[self.at(a)..self.at(b)]
    }

    /// Closes of valid rows `[a, b)`.
    #[inline]
    pub fn cs(&self, a: usize, b: usize) -> &[f64] {
        &self.close[self.at(a)..self.at(b)]
    }
}

/// The whole-series fold every public formula function uses: `S::default()` before row 0, one
/// `step` per row, one column per output. Rows are valid rows, so the public functions assume
/// finite input; the runtime is what skips whitespace.
pub(super) fn fold<S: Default>(
    len: usize,
    outputs: usize,
    mut step: impl FnMut(&mut S, usize, &mut Out),
) -> Vec<Column> {
    assert!(outputs <= MAX_OUTPUTS, "klinechart outputs per fold");
    let mut state = S::default();
    let mut columns: Vec<Column> = (0..outputs).map(|_| Vec::with_capacity(len)).collect();
    for i in 0..len {
        let mut out: Out = [None; MAX_OUTPUTS];
        step(&mut state, i, &mut out);
        for (column, value) in columns.iter_mut().zip(out) {
            column.push(value);
        }
    }
    columns
}

/// KLineChart's rolling mean: add `value`, divide the running sum by `period`, THEN subtract
/// `leaving()`. Returns `Some(sum / period)` when `i + 1 >= period`; `leaving` is only called
/// then. Keeping this exact order reproduces KLineChart's rounding bit for bit.
#[inline]
pub(super) fn rolling_mean_step(
    sum: &mut f64,
    i: usize,
    period: usize,
    value: f64,
    leaving: impl FnOnce() -> f64,
) -> Option<f64> {
    *sum += value;
    if i + 1 >= period {
        let mean = *sum / period as f64;
        *sum -= leaving();
        Some(mean)
    } else {
        None
    }
}

/// KLineChart's seeded EMA, shared by EMA and MACD: `sum` accumulates the first `period` values,
/// then `(2v + (n - 1) e) / (n + 1)`.
#[inline]
pub(super) fn seeded_ema_step(
    sum: &mut f64,
    ema: &mut f64,
    i: usize,
    period: usize,
    value: f64,
) -> Option<f64> {
    let n = period as f64;
    *sum += value;
    if i + 1 >= period {
        *ema = if i + 1 > period {
            (2.0 * value + (n - 1.0) * *ema) / (n + 1.0)
        } else {
            *sum / n
        };
        Some(*ema)
    } else {
        None
    }
}

/// The state of one bound formula between rows. `Idle` is what the history hands back before the
/// first row; the driver replaces it with the formula's own variant.
#[derive(Clone, Copy, Debug, Default)]
pub(super) enum State {
    #[default]
    Idle,
    Ma(moving_average::State),
    Ema(exponential_moving_average::State),
    Sma(simple_moving_average::State),
    Bbi(bull_and_bear_index::State),
    Vol(volume::State),
    Macd(moving_average_convergence_divergence::State),
    Boll(bollinger_bands::State),
    Kdj(stoch::State),
    Rsi(relative_strength_index::State),
    Bias(bias::State),
    Brar(brar::State),
    Cci(commodity_channel_index::State),
    Cr(current_ratio::State),
    Dma(different_of_moving_average::State),
    Dmi(directional_movement_index::State),
    Emv(ease_of_movement_value::State),
    Mtm(momentum::State),
    Obv(on_balance_volume::State),
    Pvt(price_and_volume_trend::State),
    Psy(psychological_line::State),
    Roc(rate_of_change::State),
    Sar(stop_and_reverse::State),
    Trix(triple_exponentially_smoothed_average::State),
    Vr(volume_ratio::State),
    Wr(williams_r::State),
    Ao(awesome_oscillator::State),
    Avp(average_price::State),
}

/// What a checkpoint stores: the formula state and the number of valid rows folded so far, which
/// is also the valid index of the next valid row.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Slot {
    pub rows: usize,
    pub state: State,
}

impl State {
    /// The initial state of `indicator`'s formula (`Default` of the right variant).
    pub(super) fn for_indicator(indicator: &Indicator) -> State {
        match indicator {
            Indicator::Ma { .. } => State::Ma(Default::default()),
            Indicator::Ema { .. } => State::Ema(Default::default()),
            Indicator::Sma { .. } => State::Sma(Default::default()),
            Indicator::Bbi { .. } => State::Bbi(Default::default()),
            Indicator::Vol { .. } => State::Vol(Default::default()),
            Indicator::Macd { .. } => State::Macd(Default::default()),
            Indicator::Boll { .. } => State::Boll(Default::default()),
            Indicator::Kdj { .. } => State::Kdj(Default::default()),
            Indicator::Rsi { .. } => State::Rsi(Default::default()),
            Indicator::Bias { .. } => State::Bias(Default::default()),
            Indicator::Brar { .. } => State::Brar(Default::default()),
            Indicator::Cci { .. } => State::Cci(Default::default()),
            Indicator::Cr { .. } => State::Cr(Default::default()),
            Indicator::Dma { .. } => State::Dma(Default::default()),
            Indicator::Dmi { .. } => State::Dmi(Default::default()),
            Indicator::Emv { .. } => State::Emv(Default::default()),
            Indicator::Mtm { .. } => State::Mtm(Default::default()),
            Indicator::Obv { .. } => State::Obv(Default::default()),
            Indicator::Pvt => State::Pvt(Default::default()),
            Indicator::Psy { .. } => State::Psy(Default::default()),
            Indicator::Roc { .. } => State::Roc(Default::default()),
            Indicator::Sar { .. } => State::Sar(Default::default()),
            Indicator::Trix { .. } => State::Trix(Default::default()),
            Indicator::Vr { .. } => State::Vr(Default::default()),
            Indicator::Wr { .. } => State::Wr(Default::default()),
            Indicator::Ao { .. } => State::Ao(Default::default()),
            Indicator::Avp => State::Avp(Default::default()),
        }
    }
}

/// Advances `slot` by one valid row: dispatches to the formula's `step` with the parameters taken
/// from `indicator`, then counts the row. `out` is all `None` on entry. VOL's volume bar is the
/// input column, so the arm writes it to `out[0]` itself and the formula fills the averages after it.
pub(super) fn step_row(indicator: &Indicator, slot: &mut Slot, w: &Window<'_>, out: &mut Out) {
    let i = slot.rows;
    match (indicator, &mut slot.state) {
        (Indicator::Ma { periods }, State::Ma(st)) => moving_average::step(periods, st, w, i, out),
        (Indicator::Ema { periods }, State::Ema(st)) => {
            exponential_moving_average::step(periods, st, w, i, out);
        }
        (Indicator::Sma { period, weight }, State::Sma(st)) => {
            simple_moving_average::step(*period, *weight, st, w, i, out);
        }
        (Indicator::Bbi { periods }, State::Bbi(st)) => {
            bull_and_bear_index::step(periods, st, w, i, out);
        }
        (Indicator::Vol { periods }, State::Vol(st)) => {
            out[0] = Some(w.v(i));
            volume::step(periods, st, w, i, &mut out[1..]);
        }
        (
            Indicator::Macd {
                short,
                long,
                signal,
            },
            State::Macd(st),
        ) => moving_average_convergence_divergence::step(*short, *long, *signal, st, w, i, out),
        (Indicator::Boll { period, multiplier }, State::Boll(st)) => {
            bollinger_bands::step(*period, *multiplier, st, w, i, out);
        }
        (
            Indicator::Kdj {
                period,
                k_smoothing,
                d_smoothing,
            },
            State::Kdj(st),
        ) => stoch::step(*period, *k_smoothing, *d_smoothing, st, w, i, out),
        (Indicator::Rsi { periods }, State::Rsi(st)) => {
            relative_strength_index::step(periods, st, w, i, out);
        }
        (Indicator::Bias { periods }, State::Bias(st)) => bias::step(periods, st, w, i, out),
        (Indicator::Brar { period }, State::Brar(st)) => brar::step(*period, st, w, i, out),
        (Indicator::Cci { period }, State::Cci(st)) => {
            commodity_channel_index::step(*period, st, w, i, out);
        }
        (Indicator::Cr { period, ma_periods }, State::Cr(st)) => {
            current_ratio::step(*period, *ma_periods, st, w, i, out);
        }
        (
            Indicator::Dma {
                short,
                long,
                signal,
            },
            State::Dma(st),
        ) => different_of_moving_average::step(*short, *long, *signal, st, w, i, out),
        (
            Indicator::Dmi {
                period,
                adxr_period,
            },
            State::Dmi(st),
        ) => directional_movement_index::step(*period, *adxr_period, st, w, i, out),
        (Indicator::Emv { period }, State::Emv(st)) => {
            ease_of_movement_value::step(*period, st, w, i, out);
        }
        (Indicator::Mtm { period, ma_period }, State::Mtm(st)) => {
            momentum::step(*period, *ma_period, st, w, i, out);
        }
        (Indicator::Obv { ma_period }, State::Obv(st)) => {
            on_balance_volume::step(*ma_period, st, w, i, out);
        }
        (Indicator::Pvt, State::Pvt(st)) => price_and_volume_trend::step(st, w, i, out),
        (Indicator::Psy { period, ma_period }, State::Psy(st)) => {
            psychological_line::step(*period, *ma_period, st, w, i, out);
        }
        (Indicator::Roc { period, ma_period }, State::Roc(st)) => {
            rate_of_change::step(*period, *ma_period, st, w, i, out);
        }
        (Indicator::Sar { start, step, max }, State::Sar(st)) => {
            stop_and_reverse::step(*start, *step, *max, st, w, i, out);
        }
        (Indicator::Trix { period, ma_period }, State::Trix(st)) => {
            triple_exponentially_smoothed_average::step(*period, *ma_period, st, w, i, out);
        }
        (Indicator::Vr { period, ma_period }, State::Vr(st)) => {
            volume_ratio::step(*period, *ma_period, st, w, i, out);
        }
        (Indicator::Wr { periods }, State::Wr(st)) => williams_r::step(periods, st, w, i, out),
        (Indicator::Ao { short, long }, State::Ao(st)) => {
            awesome_oscillator::step(*short, *long, st, w, i, out);
        }
        (Indicator::Avp, State::Avp(st)) => average_price::step(st, w, i, out),
        _ => unreachable!("klinechart state does not belong to its indicator"),
    }
    slot.rows += 1;
}

/// The farthest any read of any step of `indicator` reaches back, in valid rows, lagging shadow
/// copies included. An upper bound is always safe; a bound that is too small panics in the
/// whitespace window.
pub(super) fn lookback(indicator: &Indicator) -> usize {
    match indicator {
        Indicator::Ma { periods } => moving_average::lookback(periods),
        Indicator::Ema { periods } => exponential_moving_average::lookback(periods),
        Indicator::Sma { period, weight } => simple_moving_average::lookback(*period, *weight),
        Indicator::Bbi { periods } => bull_and_bear_index::lookback(periods),
        Indicator::Vol { periods } => volume::lookback(periods),
        Indicator::Macd {
            short,
            long,
            signal,
        } => moving_average_convergence_divergence::lookback(*short, *long, *signal),
        Indicator::Boll { period, multiplier } => bollinger_bands::lookback(*period, *multiplier),
        Indicator::Kdj {
            period,
            k_smoothing,
            d_smoothing,
        } => stoch::lookback(*period, *k_smoothing, *d_smoothing),
        Indicator::Rsi { periods } => relative_strength_index::lookback(periods),
        Indicator::Bias { periods } => bias::lookback(periods),
        Indicator::Brar { period } => brar::lookback(*period),
        Indicator::Cci { period } => commodity_channel_index::lookback(*period),
        Indicator::Cr { period, ma_periods } => current_ratio::lookback(*period, *ma_periods),
        Indicator::Dma {
            short,
            long,
            signal,
        } => different_of_moving_average::lookback(*short, *long, *signal),
        Indicator::Dmi {
            period,
            adxr_period,
        } => directional_movement_index::lookback(*period, *adxr_period),
        Indicator::Emv { period } => ease_of_movement_value::lookback(*period),
        Indicator::Mtm { period, ma_period } => momentum::lookback(*period, *ma_period),
        Indicator::Obv { ma_period } => on_balance_volume::lookback(*ma_period),
        Indicator::Pvt => price_and_volume_trend::lookback(),
        Indicator::Psy { period, ma_period } => psychological_line::lookback(*period, *ma_period),
        Indicator::Roc { period, ma_period } => rate_of_change::lookback(*period, *ma_period),
        Indicator::Sar { start, step, max } => stop_and_reverse::lookback(*start, *step, *max),
        Indicator::Trix { period, ma_period } => {
            triple_exponentially_smoothed_average::lookback(*period, *ma_period)
        }
        Indicator::Vr { period, ma_period } => volume_ratio::lookback(*period, *ma_period),
        Indicator::Wr { periods } => williams_r::lookback(periods),
        Indicator::Ao { short, long } => awesome_oscillator::lookback(*short, *long),
        Indicator::Avp => average_price::lookback(),
    }
}

/// Reusable columns for the compacted whitespace window. Transient: a clone starts empty.
#[derive(Debug, Default)]
struct Scratch {
    open: Vec<f64>,
    high: Vec<f64>,
    low: Vec<f64>,
    close: Vec<f64>,
    volume: Vec<f64>,
}

impl Clone for Scratch {
    fn clone(&self) -> Self {
        Self::default()
    }
}

impl Scratch {
    /// Gathers the valid rows of `[from, to)` into the columns (the volume column only up to the
    /// end of the input's own volume) and returns how many of them precede `chunk_start`.
    // ponytail: all five columns are gathered although a formula reads two or three of them;
    // per-formula column masks would shrink the scratch by up to 5x and are not needed yet.
    fn gather(
        &mut self,
        input: &IndicatorInput<'_>,
        open: &[f64],
        volume: &[f64],
        (from, chunk_start, to): (usize, usize, usize),
    ) -> usize {
        self.open.clear();
        self.high.clear();
        self.low.clear();
        self.close.clear();
        self.volume.clear();
        let mut preroll = 0;
        for row in from..to {
            if whitespace_row(input, row) {
                continue;
            }
            if row < chunk_start {
                preroll += 1;
            }
            self.open.push(open[row]);
            self.high.push(input.high[row]);
            self.low.push(input.low[row]);
            self.close.push(input.close[row]);
            // Valid rows ascend, so the rows with a volume are a prefix of the gathered rows.
            if row < volume.len() {
                self.volume.push(volume[row]);
            }
        }
        preroll
    }

    fn columns(&mut self) -> [&mut Vec<f64>; 5] {
        [
            &mut self.open,
            &mut self.high,
            &mut self.low,
            &mut self.close,
            &mut self.volume,
        ]
    }

    fn bytes(&self) -> usize {
        [&self.open, &self.high, &self.low, &self.close, &self.volume]
            .iter()
            .map(|column| column.capacity() * std::mem::size_of::<f64>())
            .sum()
    }
}

/// The incremental state of one bound KLineChart indicator: sparse checkpoints of [`Slot`] plus the
/// reusable whitespace scratch.
#[derive(Clone, Debug)]
pub(crate) struct KlRuntime {
    history: RecursiveHistory<Slot>,
    scratch: Scratch,
    chunk: usize,
}

impl KlRuntime {
    pub(crate) fn new() -> Self {
        Self {
            history: RecursiveHistory::new(),
            scratch: Scratch::default(),
            chunk: REPLAY_CHUNK,
        }
    }

    /// A runtime that replays in chunks of `chunk` source rows, so tests hit chunk boundaries at
    /// odd offsets.
    #[cfg(test)]
    fn with_chunk(chunk: usize) -> Self {
        assert!(chunk > 0, "klinechart replay chunk");
        Self {
            chunk,
            ..Self::new()
        }
    }

    /// Replays the source rows of `requested..n` through the formula from the nearest checkpoint
    /// and appends every output's rows from its `output_from` on (`NaN` for whitespace rows and
    /// rows the formula leaves unset). Returns the source rows evaluated: the replayed rows plus
    /// the rows a whitespace-compacted window re-reads before each chunk.
    ///
    /// A source row carrying whitespace (NaN close, high or low) emits `NaN` and changes no state;
    /// windows and lags count valid rows only, so every value equals the value computed with the
    /// whitespace rows removed. Whitespace is handled in global source-row coordinates so the
    /// checkpoint rows stay valid, which is why the generic window compaction cannot be reused.
    pub(crate) fn rebuild(
        &mut self,
        indicator: &Indicator,
        input: &IndicatorInput<'_>,
        requested: usize,
        n: usize,
        output_from: &[usize; MAX_OUTPUTS],
        outputs: &mut [Vec<f64>; MAX_OUTPUTS],
    ) -> usize {
        let output_count = indicator.output_count().min(MAX_OUTPUTS);
        // The whole open column falls back to close when it is shorter than the source (BRAR is
        // the only reader). A volume column shorter than the source stays short: the window
        // answers `missing_volume` past its end, so no padded copy is made.
        let open = if input.open.len() >= n {
            &input.open[..n]
        } else {
            &input.close[..n]
        };
        let volume = &input.volume[..input.volume.len().min(n)];
        let missing_volume = indicator.missing_volume();
        let lookback = lookback(indicator);

        let Self {
            history,
            scratch,
            chunk,
        } = self;
        let (start, mut slot) = history.begin(n, requested);
        if matches!(slot.state, State::Idle) {
            slot = Slot {
                rows: 0,
                state: State::for_indicator(indicator),
            };
        }
        let mut work = n - start;
        let mut tail = None;
        let mut before_tail = None;
        let mut chunk_start = start;
        while chunk_start < n {
            let chunk_end = (chunk_start + *chunk).min(n);
            // No whitespace before the chunk is known from the checkpointed counter alone; the rows
            // of the chunk themselves are scanned once. A whitespace-free chart reads the input in
            // place for any lookback.
            let clean = chunk_start == slot.rows
                && !(chunk_start..chunk_end).any(|row| whitespace_row(input, row));
            let window = if clean {
                Window {
                    open,
                    high: &input.high[..n],
                    low: &input.low[..n],
                    close: &input.close[..n],
                    volume,
                    base: 0,
                    missing_volume,
                }
            } else {
                let window_start = if lookback == 0 {
                    chunk_start
                } else {
                    valid_lookback_start(input, chunk_start, lookback)
                };
                work += chunk_start - window_start;
                let preroll =
                    scratch.gather(input, open, volume, (window_start, chunk_start, chunk_end));
                Window {
                    open: &scratch.open,
                    high: &scratch.high,
                    low: &scratch.low,
                    close: &scratch.close,
                    volume: &scratch.volume,
                    base: slot
                        .rows
                        .checked_sub(preroll)
                        .expect("klinechart valid-row count"),
                    missing_volume,
                }
            };
            for row in chunk_start..chunk_end {
                let last = row + 1 == n;
                // Only the last row needs the state before it (for a later replacement of that row).
                let before = if last { Some(slot) } else { None };
                let mut out: Out = [None; MAX_OUTPUTS];
                if clean || !whitespace_row(input, row) {
                    step_row(indicator, &mut slot, &window, &mut out);
                }
                history.checkpoint(row, slot);
                for ((column, &from), value) in outputs
                    .iter_mut()
                    .zip(output_from)
                    .zip(out)
                    .take(output_count)
                {
                    if row >= from {
                        column.push(value.filter(|value| value.is_finite()).unwrap_or(f64::NAN));
                    }
                }
                if last {
                    tail = Some(slot);
                    before_tail = if row > 0 { before } else { None };
                }
            }
            chunk_start = chunk_end;
        }
        history.finish(n, tail, before_tail);
        work
    }

    /// Bytes held: the checkpoints and the whitespace scratch.
    pub(crate) fn bytes(&self) -> usize {
        self.history.bytes() + self.scratch.bytes()
    }

    /// Drops the scratch a long whitespace window grew past the transfer-buffer cap.
    pub(crate) fn release_scratch(&mut self) {
        for column in self.scratch.columns() {
            column.clear();
            if column.capacity() > MAX_RETAINED_ROWS {
                column.shrink_to(MAX_RETAINED_ROWS);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::klinechart::{Bars, NAMES};

    /// Deterministic xorshift.
    struct Rng(u64);

    impl Rng {
        fn unit(&mut self) -> f64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 >> 11) as f64 / (1u64 << 53) as f64
        }
    }

    #[derive(Clone, Default)]
    struct Cols {
        open: Vec<f64>,
        high: Vec<f64>,
        low: Vec<f64>,
        close: Vec<f64>,
        volume: Vec<f64>,
    }

    impl Cols {
        fn walk(rows: usize, seed: u64) -> Self {
            let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
            let mut cols = Self::default();
            let mut close = 100.0;
            for _ in 0..rows {
                let open = close + (rng.unit() - 0.5) * 2.0;
                close = (open + (rng.unit() - 0.5) * 4.0).max(1.0);
                cols.open.push(open);
                cols.high.push(open.max(close) + rng.unit());
                cols.low.push(open.min(close) - rng.unit());
                cols.close.push(close);
                cols.volume.push(500.0 + (rng.unit() * 1500.0).floor());
            }
            cols
        }

        fn blank(&mut self, row: usize) {
            self.open[row] = f64::NAN;
            self.high[row] = f64::NAN;
            self.low[row] = f64::NAN;
            self.close[row] = f64::NAN;
            self.volume[row] = f64::NAN;
        }

        fn len(&self) -> usize {
            self.close.len()
        }

        fn columns_mut(&mut self) -> [&mut Vec<f64>; 5] {
            [
                &mut self.open,
                &mut self.high,
                &mut self.low,
                &mut self.close,
                &mut self.volume,
            ]
        }

        fn input<'a>(&'a self, times: &'a [i64]) -> IndicatorInput<'a> {
            IndicatorInput {
                times: &times[..self.len()],
                open: &self.open,
                high: &self.high,
                low: &self.low,
                close: &self.close,
                volume: &self.volume,
                amount: &[],
            }
        }

        /// Every output over the rows that carry a sample, scattered back with NaN elsewhere.
        fn expected(&self, indicator: &Indicator) -> Vec<Vec<f64>> {
            let valid: Vec<usize> = (0..self.len())
                .filter(|&row| {
                    !(self.close[row].is_nan() || self.high[row].is_nan() || self.low[row].is_nan())
                })
                .collect();
            let pick = |column: &[f64]| valid.iter().map(|&row| column[row]).collect::<Vec<_>>();
            let close = pick(&self.close);
            let columns = indicator.compute(&Bars {
                open: &pick(&self.open),
                high: &pick(&self.high),
                low: &pick(&self.low),
                close: &close,
                volume: &pick(&self.volume),
                turnover: &close,
            });
            (0..indicator.output_count())
                .map(|output| {
                    let mut values = vec![f64::NAN; self.len()];
                    for (compact, &row) in valid.iter().enumerate() {
                        values[row] = columns[output][compact]
                            .filter(|value| value.is_finite())
                            .unwrap_or(f64::NAN);
                    }
                    values
                })
                .collect()
        }
    }

    /// Every template with KLineChart's default parameters, plus extra moving-average parameter
    /// sets (one period, adjacent periods, five periods in no particular order).
    fn templates() -> Vec<Indicator> {
        let mut all: Vec<Indicator> = NAMES
            .iter()
            .map(|name| Indicator::from_name(name).expect("template"))
            .collect();
        all.extend([
            Indicator::Ma { periods: vec![1] },
            Indicator::Ma {
                periods: vec![2, 3],
            },
            Indicator::Ma {
                periods: vec![7, 40, 1, 3, 11],
            },
        ]);
        all
    }

    /// Rebuilds `runtime` from `requested` the way `IncrementalState::rebuild_rows` drives it and
    /// stitches the suffix into `canonical`. Returns the rows evaluated.
    fn rebuild(
        runtime: &mut KlRuntime,
        indicator: &Indicator,
        cols: &Cols,
        requested: usize,
        canonical: &mut [Vec<f64>],
    ) -> usize {
        let times: Vec<i64> = (0..cols.len() as i64).collect();
        let input = cols.input(&times);
        let n = cols.len();
        let mut output_from = [0; MAX_OUTPUTS];
        let mut outputs: [Vec<f64>; MAX_OUTPUTS] = Default::default();
        for (from, start) in output_from.iter_mut().zip(indicator.output_starts()) {
            *from = requested.max(start).min(n);
        }
        let work = runtime.rebuild(indicator, &input, requested, n, &output_from, &mut outputs);
        for (output, column) in canonical.iter_mut().enumerate() {
            column.truncate(output_from[output]);
            column.resize(output_from[output], f64::NAN);
            column.extend_from_slice(&outputs[output]);
            assert_eq!(column.len(), n, "stitched length of output {output}");
        }
        work
    }

    fn assert_same(context: &str, got: &[Vec<f64>], want: &[Vec<f64>]) {
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

    #[test]
    #[should_panic(expected = "klinechart lookback")]
    fn a_read_before_the_window_base_panics_in_every_profile() {
        let close = [1.0, 2.0, 3.0];
        let window = Window {
            close: &close,
            base: 5,
            ..Window::EMPTY
        };
        let _ = window.c(4);
    }

    #[test]
    #[should_panic(expected = "klinechart lookback")]
    fn a_slice_starting_before_the_window_base_panics() {
        let close = [1.0, 2.0, 3.0];
        let window = Window {
            close: &close,
            base: 5,
            ..Window::EMPTY
        };
        let _ = window.cs(4, 6);
    }

    #[test]
    fn a_window_addresses_valid_rows_and_half_open_ranges() {
        let open = [1.0, 2.0, 3.0, 4.0];
        let high = [5.0, 6.0, 7.0, 8.0];
        let low = [0.5, 1.5, 2.5, 3.5];
        let close = [10.0, 11.0, 12.0, 13.0];
        let volume = [20.0, 21.0];
        let window = Window {
            open: &open,
            high: &high,
            low: &low,
            close: &close,
            volume: &volume,
            base: 3,
            missing_volume: 7.0,
        };
        assert_eq!(
            (window.o(3), window.h(4), window.l(5), window.c(6)),
            (1.0, 6.0, 2.5, 13.0)
        );
        assert_eq!(window.cs(4, 6), [11.0, 12.0]);
        assert_eq!(window.highs(3, 7), high);
        assert_eq!(window.lows(5, 6), [2.5]);
        assert!(window.cs(5, 5).is_empty());
        // A volume column shorter than the rows answers the template's missing volume.
        assert_eq!((window.v(3), window.v(4), window.v(5)), (20.0, 21.0, 7.0));
        assert_eq!(window.t(5), window.c(5));
    }

    #[test]
    fn the_gathered_window_keeps_valid_rows_in_order_and_volume_as_a_prefix() {
        let rows = 12;
        let mut cols = Cols::walk(rows, 3);
        cols.blank(3);
        cols.blank(7);
        cols.volume.truncate(9);
        // Valid rows 2, 4, 5 | 6, 8, 9, 10, 11; five valid rows (0, 1, 2, 4, 5) precede row 6.
        let times: Vec<i64> = (0..rows as i64).collect();
        let input = IndicatorInput {
            volume: &cols.volume,
            ..cols.input(&times)
        };
        let mut scratch = Scratch::default();
        let preroll = scratch.gather(&input, &cols.open, &cols.volume, (2, 6, rows));
        assert_eq!(preroll, 3);
        assert_eq!(scratch.close.len(), 8);
        assert_eq!(
            scratch.volume.len(),
            5,
            "rows 2, 4, 5, 6 and 8 have a volume"
        );
        let window = Window {
            open: &scratch.open,
            high: &scratch.high,
            low: &scratch.low,
            close: &scratch.close,
            volume: &scratch.volume,
            base: 5 - preroll,
            missing_volume: 7.0,
        };
        let source_rows = [2, 4, 5, 6, 8, 9, 10, 11];
        for (offset, &row) in source_rows.iter().enumerate() {
            let k = 2 + offset;
            assert_eq!(
                window.c(k),
                cols.close[row],
                "valid row {k} is source row {row}"
            );
            let volume = if row < 9 { cols.volume[row] } else { 7.0 };
            assert_eq!(window.v(k), volume, "volume of valid row {k}");
        }
    }

    #[test]
    fn every_template_has_its_own_state_and_dispatch_arms() {
        for indicator in templates() {
            let state = State::for_indicator(&indicator);
            let name = indicator.name();
            assert!(
                format!("{state:?}").to_ascii_uppercase().starts_with(name),
                "{name} starts as {state:?}"
            );
            let _ = lookback(&indicator);
        }
    }

    #[test]
    fn a_declared_lookback_covers_every_read() {
        let rows = 220;
        for indicator in templates() {
            let name = format!("{} {:?}", indicator.name(), indicator.calc_params());
            let cols = Cols::walk(rows, 9);
            let missing_volume = indicator.missing_volume();
            let lookback = lookback(&indicator);
            let window = |base: usize| Window {
                open: &cols.open[base..],
                high: &cols.high[base..],
                low: &cols.low[base..],
                close: &cols.close[base..],
                volume: &cols.volume[base..],
                base,
                missing_volume,
            };
            // One pass over the whole series, remembering the state and outputs after each row.
            let whole = window(0);
            let mut slot = Slot {
                rows: 0,
                state: State::for_indicator(&indicator),
            };
            let (mut slots, mut outs) = (Vec::new(), Vec::new());
            for _ in 0..rows {
                let mut out: Out = [None; MAX_OUTPUTS];
                step_row(&indicator, &mut slot, &whole, &mut out);
                slots.push(slot);
                outs.push(out);
            }
            // Resume after any row with a window holding exactly `lookback` valid rows before the
            // next one: a read further back panics, and every following row must still agree.
            for (resume, &after) in slots.iter().enumerate().take(rows - 1) {
                let next = resume + 1;
                let window = window(next.saturating_sub(lookback));
                let mut slot = after;
                for (offset, want) in outs[next..rows.min(next + 60)].iter().enumerate() {
                    let mut out: Out = [None; MAX_OUTPUTS];
                    step_row(&indicator, &mut slot, &window, &mut out);
                    let bits = |out: &Out| out.map(|value| value.map(f64::to_bits));
                    assert_eq!(
                        bits(&out),
                        bits(want),
                        "{name}: row {} resumed after row {resume}",
                        next + offset
                    );
                }
            }
        }
    }

    #[test]
    fn chunked_replay_matches_compute_across_checkpoints_and_whitespace() {
        let rows = 2_600;
        for indicator in templates() {
            for chunk in [5, 4_096] {
                let name = format!("{} chunk {chunk}", indicator.name());
                let mut cols = Cols::walk(rows, 4);
                for row in [0, 1, 40, 41, 42, 1_023, 1_024, 2_000, rows - 1] {
                    cols.blank(row);
                }
                let mut runtime = KlRuntime::with_chunk(chunk);
                let mut canonical = vec![Vec::new(); indicator.output_count()];
                rebuild(&mut runtime, &indicator, &cols, 0, &mut canonical);
                assert_same(
                    &format!("{name} build"),
                    &canonical,
                    &cols.expected(&indicator),
                );

                // Replace the last row, then append two rows.
                let mut fill = Cols::walk(rows + 2, 5);
                for (column, fill) in cols.columns_mut().into_iter().zip(fill.columns_mut()) {
                    column[rows - 1] = fill[rows - 1];
                    column.extend_from_slice(&fill[rows..]);
                }
                rebuild(&mut runtime, &indicator, &cols, rows - 1, &mut canonical);
                assert_same(
                    &format!("{name} tick"),
                    &canonical,
                    &cols.expected(&indicator),
                );

                // Historical repairs on both sides of a checkpoint, one making a row whitespace.
                cols.close[1_030] += 1.5;
                rebuild(&mut runtime, &indicator, &cols, 1_030, &mut canonical);
                assert_same(
                    &format!("{name} repair"),
                    &canonical,
                    &cols.expected(&indicator),
                );
                cols.blank(1_500);
                rebuild(&mut runtime, &indicator, &cols, 1_500, &mut canonical);
                assert_same(
                    &format!("{name} gap"),
                    &canonical,
                    &cols.expected(&indicator),
                );

                // Truncate to a checkpoint boundary and past it, replacing the row at the cut.
                for keep in [2_048, 1_100] {
                    for column in cols.columns_mut() {
                        column.truncate(keep);
                        column.push(100.0);
                    }
                    rebuild(&mut runtime, &indicator, &cols, keep, &mut canonical);
                    assert_same(
                        &format!("{name} truncate to {keep}"),
                        &canonical,
                        &cols.expected(&indicator),
                    );
                }

                // Grow again by a few chunks and a checkpoint.
                let before = cols.len();
                let mut more = Cols::walk(1_200, 7);
                for (column, more) in cols.columns_mut().into_iter().zip(more.columns_mut()) {
                    column.extend_from_slice(more);
                }
                rebuild(&mut runtime, &indicator, &cols, before, &mut canonical);
                assert_same(
                    &format!("{name} grow"),
                    &canonical,
                    &cols.expected(&indicator),
                );
            }
        }
    }

    #[test]
    fn a_tick_costs_the_window_not_the_history() {
        let rows = 3_000;
        for indicator in templates() {
            let name = format!("{} {:?}", indicator.name(), indicator.calc_params());
            let mut cols = Cols::walk(rows, 6);
            let mut runtime = KlRuntime::new();
            let mut canonical = vec![Vec::new(); indicator.output_count()];
            assert_eq!(
                rebuild(&mut runtime, &indicator, &cols, 0, &mut canonical),
                rows,
                "{name}: a full build evaluates every row"
            );
            cols.close[rows - 1] += 1.0;
            assert_eq!(
                rebuild(&mut runtime, &indicator, &cols, rows - 1, &mut canonical),
                1,
                "{name}: a whitespace-free tick evaluates one row"
            );
            // One whitespace row far back: a tick re-reads the lookback behind it, no more.
            cols.blank(100);
            let mut runtime = KlRuntime::new();
            rebuild(&mut runtime, &indicator, &cols, 0, &mut canonical);
            cols.close[rows - 1] += 1.0;
            assert_eq!(
                rebuild(&mut runtime, &indicator, &cols, rows - 1, &mut canonical),
                lookback(&indicator) + 1,
                "{name}: a tick over a whitespace history re-reads exactly its lookback"
            );
            assert!(runtime.bytes() > 0);
        }
    }
}
