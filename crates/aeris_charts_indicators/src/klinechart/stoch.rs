//! `KDJ` (随机指标). Ported from KLineChart `src/extension/indicator/stoch.ts`.

use super::stepper::{Out, Window, fold};
use super::{Column, highest_high_lowest_low};

/// KDJ outputs in KLineChart figure order.
#[derive(Clone, Debug, PartialEq)]
pub struct Kdj {
    /// `K = ((M1 - 1) * K' + RSV) / M1`, starting from `K' = 50`.
    pub k: Column,
    /// `D = ((M2 - 1) * D' + K) / M2`, starting from `D' = 50`.
    pub d: Column,
    /// `J = 3K - 2D`.
    pub j: Column,
}

/// The last `K` and `D`, the only recursion KDJ carries between rows.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct State {
    k: f64,
    d: f64,
}

/// The range scan covers the last `period` rows, the current one included.
pub(super) fn lookback(period: usize, _k_smoothing: usize, _d_smoothing: usize) -> usize {
    period.saturating_sub(1)
}

/// Advances KDJ by the valid row `i`. A zero period or smoothing leaves every output unset, and so
/// do the first `period - 1` rows.
pub(super) fn step(
    period: usize,
    k_smoothing: usize,
    d_smoothing: usize,
    st: &mut State,
    w: &Window<'_>,
    i: usize,
    out: &mut Out,
) {
    if period == 0 || k_smoothing == 0 || d_smoothing == 0 || i + 1 < period {
        return;
    }
    let m1 = k_smoothing as f64;
    let m2 = d_smoothing as f64;
    let start = i + 1 - period;
    let (hn, ln) = highest_high_lowest_low(w.highs(start, i + 1), w.lows(start, i + 1));
    let range = hn - ln;
    let rsv = ((w.c(i) - ln) / if range == 0.0 { 1.0 } else { range }) * 100.0;
    // The first computed row starts both recursions from 50.
    let (prev_k, prev_d) = if i + 1 == period {
        (50.0, 50.0)
    } else {
        (st.k, st.d)
    };
    let k = ((m1 - 1.0) * prev_k + rsv) / m1;
    let d = ((m2 - 1.0) * prev_d + k) / m2;
    st.k = k;
    st.d = d;
    out[0] = Some(k);
    out[1] = Some(d);
    out[2] = Some(3.0 * k - 2.0 * d);
}

/// KLineChart default: `period = 9`, `k_smoothing = 3`, `d_smoothing = 3`.
///
/// `RSV = (CLOSE - LLV(LOW, N)) / (HHV(HIGH, N) - LLV(LOW, N)) * 100`, with the range replaced by 1
/// when it is zero.
///
/// A fold of the same `step` the chart runtime executes; it assumes finite input.
pub fn kdj(
    high: &[f64],
    low: &[f64],
    close: &[f64],
    period: usize,
    k_smoothing: usize,
    d_smoothing: usize,
) -> Kdj {
    let window = Window {
        high,
        low,
        close,
        ..Window::EMPTY
    };
    let [k, d, j]: [Column; 3] = fold::<State>(close.len(), 3, |st, i, out| {
        step(period, k_smoothing, d_smoothing, st, &window, i, out);
    })
    .try_into()
    .expect("KDJ has three outputs");
    Kdj { k, d, j }
}
