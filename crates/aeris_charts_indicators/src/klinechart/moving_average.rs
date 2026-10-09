//! `MA` (移动平均). Ported from KLineChart `src/extension/indicator/movingAverage.ts`.
//!
//! This file is the template of the row-stepper contract (see `stepper.rs`): a `State`, a
//! `lookback`, one `step` over one valid row, and a public whole-series function that is a fold of
//! that same `step`.

use super::Column;
use super::stepper::{Out, Window, fold, rolling_mean_step};
use crate::MAX_OUTPUTS;

/// One running sum per period. A running add/subtract sum cannot be recomputed from its window
/// bit for bit, so it lives in the checkpointed state.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct State {
    sum: [f64; MAX_OUTPUTS],
}

/// The leaving value of the longest window is `period - 1` rows back.
pub(super) fn lookback(periods: &[usize]) -> usize {
    periods
        .iter()
        .take(MAX_OUTPUTS)
        .copied()
        .max()
        .unwrap_or(0)
        .saturating_sub(1)
}

/// Advances every mean by the valid row `i`. A zero period leaves its output unset.
pub(super) fn step(periods: &[usize], st: &mut State, w: &Window<'_>, i: usize, out: &mut Out) {
    for ((slot, sum), &period) in out.iter_mut().zip(&mut st.sum).zip(periods) {
        if period == 0 {
            continue;
        }
        *slot = rolling_mean_step(sum, i, period, w.c(i), || w.c(i + 1 - period));
    }
}

/// Rolling means of `close`, one column per period. KLineChart default periods: `[5, 10, 30, 60]`.
///
/// A fold of the same `step` the chart runtime executes; it assumes finite input.
pub fn ma(close: &[f64], periods: &[usize]) -> Vec<Column> {
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
