//! `CCI` (顺势指标). Ported from KLineChart `src/extension/indicator/commodityChannelIndex.ts`.

use super::Column;
use super::stepper::{Out, Window, fold};

/// The running sum of the window's typical prices. A running add/subtract sum cannot be recomputed
/// from its window bit for bit, so it lives in the checkpointed state.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct State {
    tp_sum: f64,
}

/// The window starts `period - 1` rows back.
pub(super) fn lookback(period: usize) -> usize {
    period.saturating_sub(1)
}

/// `TP = (HIGH + LOW + CLOSE) / 3`.
fn typical(high: f64, low: f64, close: f64) -> f64 {
    (high + low + close) / 3.0
}

/// Advances the formula by the valid row `i`. A zero period leaves the output unset.
pub(super) fn step(period: usize, st: &mut State, w: &Window<'_>, i: usize, out: &mut Out) {
    if period == 0 {
        return;
    }
    let n = period as f64;
    let tp = typical(w.h(i), w.l(i), w.c(i));
    st.tp_sum += tp;
    if i + 1 >= period {
        let start = i + 1 - period;
        let ma_tp = st.tp_sum / n;
        let mut abs_sum = 0.0;
        let window = w
            .highs(start, i + 1)
            .iter()
            .zip(w.lows(start, i + 1))
            .zip(w.cs(start, i + 1));
        for ((&high, &low), &close) in window {
            abs_sum += (typical(high, low, close) - ma_tp).abs();
        }
        let md = abs_sum / n;
        out[0] = Some(if md != 0.0 {
            (tp - ma_tp) / md / 0.015
        } else {
            0.0
        });
        st.tp_sum -= typical(w.h(start), w.l(start), w.c(start));
    }
}

/// `CCI = (TP - MA(TP, N)) / MD / 0.015`, where `TP = (HIGH + LOW + CLOSE) / 3` and `MD` is the mean
/// absolute deviation of the window's `TP` around `MA(TP, N)`; 0 when `MD` is 0. KLineChart default:
/// `period = 20`.
///
/// A fold of the same `step` the chart runtime executes; it assumes finite input.
pub fn cci(high: &[f64], low: &[f64], close: &[f64], period: usize) -> Column {
    let window = Window {
        high,
        low,
        close,
        ..Window::EMPTY
    };
    fold::<State>(close.len(), 1, |st, i, out| {
        step(period, st, &window, i, out);
    })
    .swap_remove(0)
}
