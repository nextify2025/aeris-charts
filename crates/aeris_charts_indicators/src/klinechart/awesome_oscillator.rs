//! `AO` (动量震荡指标). Ported from KLineChart `src/extension/indicator/awesomeOscillator.ts`.

use super::stepper::{fold, rolling_mean_step, Out, Window};
use super::Column;

/// The running sums of the short and the long window over the median price. A running
/// add/subtract sum cannot be recomputed from its window bit for bit, so they live in the
/// checkpointed state.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct State {
    short: f64,
    long: f64,
}

/// The leaving median of the longer window is `max(short, long) - 1` rows back.
pub(super) fn lookback(short: usize, long: usize) -> usize {
    short.max(long).saturating_sub(1)
}

/// Advances both means by the valid row `i`. A zero period leaves the output unset.
pub(super) fn step(
    short: usize,
    long: usize,
    st: &mut State,
    w: &Window<'_>,
    i: usize,
    out: &mut Out,
) {
    if short == 0 || long == 0 {
        return;
    }
    let median = |k: usize| (w.l(k) + w.h(k)) / 2.0;
    let ma_short = rolling_mean_step(&mut st.short, i, short, median(i), || median(i + 1 - short));
    let ma_long = rolling_mean_step(&mut st.long, i, long, median(i), || median(i + 1 - long));
    if i + 1 >= short.max(long) {
        out[0] = Some(ma_short.unwrap_or(0.0) - ma_long.unwrap_or(0.0));
    }
}

/// `AO = MA(MEDIAN, SHORT) - MA(MEDIAN, LONG)` with `MEDIAN = (HIGH + LOW) / 2`. KLineChart default:
/// `short = 5`, `long = 34`.
///
/// KLineChart draws AO as bars, hollow when rising and filled when falling; that is presentation
/// and is left to the renderer.
///
/// A fold of the same `step` the chart runtime executes; it assumes finite input.
pub fn ao(high: &[f64], low: &[f64], short: usize, long: usize) -> Column {
    let window = Window {
        high,
        low,
        ..Window::EMPTY
    };
    fold::<State>(high.len(), 1, |st, i, out| {
        step(short, long, st, &window, i, out);
    })
    .swap_remove(0)
}
