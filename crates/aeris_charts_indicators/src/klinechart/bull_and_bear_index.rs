//! `BBI` (多空指标). Ported from KLineChart `src/extension/indicator/bullAndBearIndex.ts`.

use super::Column;
use super::stepper::{Out, Window, fold, rolling_mean_step};
use crate::MAX_OUTPUTS;

/// One running sum per period. A running add/subtract sum cannot be recomputed from its window
/// bit for bit, so it lives in the checkpointed state.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct State {
    sum: [f64; MAX_OUTPUTS],
}

/// The leaving value of the longest window is `period - 1` rows back. At most `MAX_OUTPUTS`
/// periods are read; the template always has four.
pub(super) fn lookback(periods: &[usize]) -> usize {
    periods
        .iter()
        .take(MAX_OUTPUTS)
        .copied()
        .max()
        .unwrap_or(0)
        .saturating_sub(1)
}

/// Advances every mean by the valid row `i`, and sets the output once the longest window is full.
/// An empty list or a zero period leaves the output unset.
pub(super) fn step(periods: &[usize], st: &mut State, w: &Window<'_>, i: usize, out: &mut Out) {
    let periods = &periods[..periods.len().min(MAX_OUTPUTS)];
    let Some(&max_period) = periods.iter().max() else {
        return;
    };
    if periods.contains(&0) {
        return;
    }
    let mut total = 0.0;
    for (sum, &period) in st.sum.iter_mut().zip(periods) {
        let mean = rolling_mean_step(sum, i, period, w.c(i), || w.c(i + 1 - period));
        total += mean.unwrap_or(0.0);
    }
    if i + 1 >= max_period {
        out[0] = Some(total / 4.0);
    }
}

/// `BBI = (MA(CLOSE, P1) + MA(CLOSE, P2) + MA(CLOSE, P3) + MA(CLOSE, P4)) / 4`. KLineChart default
/// periods: `[3, 6, 12, 24]`.
///
/// Like KLineChart, the sum is always divided by 4, so the result is only meaningful with exactly
/// four periods.
///
/// A fold of the same `step` the chart runtime executes; it assumes finite input and at most
/// `MAX_OUTPUTS` periods.
pub fn bbi(close: &[f64], periods: &[usize]) -> Column {
    assert!(
        periods.len() <= MAX_OUTPUTS,
        "bbi takes at most MAX_OUTPUTS periods"
    );
    let window = Window {
        close,
        ..Window::EMPTY
    };
    fold::<State>(close.len(), 1, |st, i, out| {
        step(periods, st, &window, i, out);
    })
    .swap_remove(0)
}
