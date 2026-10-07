//! `EMA` (指数移动平均). Ported from KLineChart
//! `src/extension/indicator/exponentialMovingAverage.ts`.

use super::Column;
use super::stepper::{Out, Window, fold, seeded_ema_step};
use crate::MAX_OUTPUTS;

/// Per period: the sum that seeds the first value and the recursion's previous value.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct State {
    sum: [f64; MAX_OUTPUTS],
    ema: [f64; MAX_OUTPUTS],
}

/// The recursion reads only the current close.
pub(super) fn lookback(_periods: &[usize]) -> usize {
    0
}

/// Advances every average by the valid row `i`. A zero period leaves its output unset.
pub(super) fn step(periods: &[usize], st: &mut State, w: &Window<'_>, i: usize, out: &mut Out) {
    for (((slot, sum), ema), &period) in out
        .iter_mut()
        .zip(&mut st.sum)
        .zip(&mut st.ema)
        .zip(periods)
    {
        if period == 0 {
            continue;
        }
        *slot = seeded_ema_step(sum, ema, i, period, w.c(i));
    }
}

/// Exponential moving averages of `close`, one column per period. KLineChart default periods:
/// `[6, 12, 20]`.
///
/// Each EMA is seeded with the simple average of its first `N` closes, then follows
/// `EMA = (2 * CLOSE + (N - 1) * EMA') / (N + 1)`.
///
/// A fold of the same `step` the chart runtime executes; it assumes finite input.
pub fn ema(close: &[f64], periods: &[usize]) -> Vec<Column> {
    let window = Window {
        close,
        ..Window::EMPTY
    };
    periods
        .chunks(MAX_OUTPUTS)
        .flat_map(|periods| {
            fold::<State>(close.len(), periods.len(), |st, i, out| {
                step(periods, st, &window, i, out);
            })
        })
        .collect()
}
