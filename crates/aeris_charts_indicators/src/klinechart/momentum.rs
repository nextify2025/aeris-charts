//! `MTM` (动量指标). Ported from KLineChart `src/extension/indicator/momentum.ts`.

use super::Column;
use super::stepper::{Out, Window, fold, rolling_mean_step};

/// MTM outputs in KLineChart figure order.
#[derive(Clone, Debug, PartialEq)]
pub struct Mtm {
    /// `MTM = CLOSE - REF(CLOSE, N)`.
    pub mtm: Column,
    /// `MAMTM = MA(MTM, M)`.
    pub ma_mtm: Column,
}

/// The running sum of the momentum values inside the average's window. A running add/subtract sum
/// cannot be recomputed from its window bit for bit, so it lives in the checkpointed state.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct State {
    sum: f64,
}

/// The value leaving the average is `ma_period - 1` rows back and reads the close `period` rows
/// before that one.
pub(super) fn lookback(period: usize, ma_period: usize) -> usize {
    (period + ma_period).saturating_sub(1)
}

/// `CLOSE[row] - CLOSE[row - period]`. A pure function of two window rows, so the value the average
/// drops is recomputed from the window instead of being kept.
fn momentum_at(w: &Window<'_>, row: usize, period: usize) -> f64 {
    w.c(row) - w.c(row - period)
}

/// Advances the formula by the valid row `i`. A zero period leaves both outputs unset.
pub(super) fn step(
    period: usize,
    ma_period: usize,
    st: &mut State,
    w: &Window<'_>,
    i: usize,
    out: &mut Out,
) {
    if period == 0 || ma_period == 0 || i < period {
        return;
    }
    let value = momentum_at(w, i, period);
    out[0] = Some(value);
    // The average counts rows from the first momentum row, `period`.
    out[1] = rolling_mean_step(&mut st.sum, i - period, ma_period, value, || {
        momentum_at(w, i + 1 - ma_period, period)
    });
}

/// KLineChart default: `period = 12`, `ma_period = 6`.
///
/// A fold of the same `step` the chart runtime executes; it assumes finite input.
pub fn mtm(close: &[f64], period: usize, ma_period: usize) -> Mtm {
    let window = Window {
        close,
        ..Window::EMPTY
    };
    let [mtm, ma_mtm]: [Column; 2] = fold::<State>(close.len(), 2, |st, i, out| {
        step(period, ma_period, st, &window, i, out);
    })
    .try_into()
    .expect("a fold of two outputs has two columns");
    Mtm { mtm, ma_mtm }
}
