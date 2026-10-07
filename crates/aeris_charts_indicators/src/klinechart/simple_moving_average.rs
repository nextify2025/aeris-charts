//! `SMA` — the weighted `SMA(X, N, M)` smoothing used by Chinese charting software, not the plain
//! rolling mean (that is [`super::ma`]). Ported from KLineChart
//! `src/extension/indicator/simpleMovingAverage.ts`.

use super::Column;
use super::stepper::{Out, Window, fold};

/// The running sum that seeds the first value, and the recursion's previous value.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct State {
    close_sum: f64,
    value: f64,
}

/// The recursion reads only the current close.
pub(super) fn lookback(_period: usize, _weight: f64) -> usize {
    0
}

/// Advances the smoothing by the valid row `i`. A zero period leaves the output unset.
pub(super) fn step(
    period: usize,
    weight: f64,
    st: &mut State,
    w: &Window<'_>,
    i: usize,
    out: &mut Out,
) {
    if period == 0 {
        return;
    }
    let n = period as f64;
    let c = w.c(i);
    st.close_sum += c;
    if i + 1 >= period {
        st.value = if i + 1 > period {
            (c * weight + st.value * (n - weight + 1.0)) / (n + 1.0)
        } else {
            st.close_sum / n
        };
        out[0] = Some(st.value);
    }
}

/// KLineChart default: `period = 12`, `weight = 2`.
///
/// Seeded with the simple average of the first `period` closes, then
/// `SMA = (CLOSE * M + SMA' * (N - M + 1)) / (N + 1)`.
///
/// A fold of the same `step` the chart runtime executes; it assumes finite input.
pub fn sma(close: &[f64], period: usize, weight: f64) -> Column {
    let window = Window {
        close,
        ..Window::EMPTY
    };
    fold::<State>(close.len(), 1, |st, i, out| {
        step(period, weight, st, &window, i, out);
    })
    .swap_remove(0)
}
