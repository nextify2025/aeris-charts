//! `BIAS` (乖离率). Ported from KLineChart `src/extension/indicator/bias.ts`.

use super::stepper::{fold, rolling_mean_step, Out, Window};
use super::Column;
use crate::MAX_OUTPUTS;

/// One running sum per period. A running add/subtract sum cannot be recomputed from its window
/// bit for bit, so it lives in the checkpointed state.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct State {
    sum: [f64; MAX_OUTPUTS],
}

/// The leaving close of the longest window is `period - 1` rows back.
pub(super) fn lookback(periods: &[usize]) -> usize {
    periods
        .iter()
        .take(MAX_OUTPUTS)
        .copied()
        .max()
        .unwrap_or(0)
        .saturating_sub(1)
}

/// Advances every period by the valid row `i`. A zero period leaves its output unset.
pub(super) fn step(periods: &[usize], st: &mut State, w: &Window<'_>, i: usize, out: &mut Out) {
    for ((slot, sum), &period) in out.iter_mut().zip(&mut st.sum).zip(periods) {
        if period == 0 {
            continue;
        }
        let close = w.c(i);
        let mean = rolling_mean_step(sum, i, period, close, || w.c(i + 1 - period));
        *slot = mean.map(|mean| ((close - mean) / mean) * 100.0);
    }
}

/// `BIAS = (CLOSE - MA(CLOSE, N)) / MA(CLOSE, N) * 100`, one column per period. KLineChart default
/// periods: `[6, 12, 24]`.
///
/// A fold of the same `step` the chart runtime executes; it assumes finite input.
pub fn bias(close: &[f64], periods: &[usize]) -> Vec<Column> {
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
