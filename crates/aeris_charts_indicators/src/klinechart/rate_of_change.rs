//! `ROC` (变动率). Ported from KLineChart `src/extension/indicator/rateOfChange.ts`.

use super::Column;
use super::stepper::{Out, Window, fold, rolling_mean_step};

/// ROC outputs in KLineChart figure order.
#[derive(Clone, Debug, PartialEq)]
pub struct Roc {
    /// `ROC = (CLOSE - REF(CLOSE, N)) / REF(CLOSE, N) * 100`, 0 when the reference close is 0.
    pub roc: Column,
    /// `MAROC = MA(ROC, M)`.
    pub ma_roc: Column,
}

/// The running sum of the rate-of-change values inside the average's window. A running
/// add/subtract sum cannot be recomputed from its window bit for bit, so it lives in the
/// checkpointed state.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct State {
    sum: f64,
}

/// The value leaving the average is `ma_period - 1` rows back and reads the close `period` rows
/// before that one.
pub(super) fn lookback(period: usize, ma_period: usize) -> usize {
    (period + ma_period).saturating_sub(1)
}

/// `(CLOSE[row] - CLOSE[row - period]) / CLOSE[row - period] * 100`, 0 when the reference close is
/// 0. A pure function of two window rows, so the value the average drops is recomputed from the
/// window instead of being kept.
fn change_at(w: &Window<'_>, row: usize, period: usize) -> f64 {
    let ago = w.c(row - period);
    if ago != 0.0 {
        ((w.c(row) - ago) / ago) * 100.0
    } else {
        0.0
    }
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
    let value = change_at(w, i, period);
    out[0] = Some(value);
    // The average counts rows from the first rate-of-change row, `period`.
    out[1] = rolling_mean_step(&mut st.sum, i - period, ma_period, value, || {
        change_at(w, i + 1 - ma_period, period)
    });
}

/// KLineChart default: `period = 12`, `ma_period = 6`.
///
/// A fold of the same `step` the chart runtime executes; it assumes finite input.
pub fn roc(close: &[f64], period: usize, ma_period: usize) -> Roc {
    let window = Window {
        close,
        ..Window::EMPTY
    };
    let [roc, ma_roc]: [Column; 2] = fold::<State>(close.len(), 2, |st, i, out| {
        step(period, ma_period, st, &window, i, out);
    })
    .try_into()
    .expect("a fold of two outputs has two columns");
    Roc { roc, ma_roc }
}
