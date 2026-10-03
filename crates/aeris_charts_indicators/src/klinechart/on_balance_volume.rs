//! `OBV` (能量潮). Ported from KLineChart `src/extension/indicator/onBalanceVolume.ts`.

use super::stepper::{fold, Out, Window};
use super::Column;

/// OBV outputs in KLineChart figure order.
#[derive(Clone, Debug, PartialEq)]
pub struct Obv {
    /// Cumulative volume, added on up closes and subtracted on down closes; 0 on the first row.
    pub obv: Column,
    /// `MAOBV = MA(OBV, N)`.
    pub ma_obv: Column,
}

/// The running total, its lagging copy and the sum the moving average divides.
///
/// `MAOBV` subtracts the OBV value that leaves its window `N - 1` rows back. OBV is a recursion of
/// the input rows, so a second running total advanced over the rows `N - 1` behind the current one
/// holds exactly that value, bit for bit, without a ring and in a size independent of `N`.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct State {
    run: f64,
    shadow: f64,
    sum: f64,
}

/// The lagging total reads the previous close of the row `N` back.
pub(super) fn lookback(ma_period: usize) -> usize {
    ma_period
}

/// One row of the OBV recursion: adds the volume on an up close, subtracts it on a down close.
fn obv_stage(run: &mut f64, w: &Window<'_>, j: usize) {
    let prev_close = w.c(j.saturating_sub(1));
    if w.c(j) < prev_close {
        *run -= w.v(j);
    } else if w.c(j) > prev_close {
        *run += w.v(j);
    }
}

/// Advances OBV and its moving average by the valid row `i`. A zero period leaves both unset.
pub(super) fn step(ma_period: usize, st: &mut State, w: &Window<'_>, i: usize, out: &mut Out) {
    if ma_period == 0 {
        return;
    }
    obv_stage(&mut st.run, w, i);
    out[0] = Some(st.run);
    st.sum += st.run;
    if i + 1 >= ma_period {
        // The row `i + 1 - N` is the first the window drops; the copy consumes every row from it.
        obv_stage(&mut st.shadow, w, i + 1 - ma_period);
        out[1] = Some(st.sum / ma_period as f64);
        st.sum -= st.shadow;
    }
}

/// KLineChart default: `ma_period = 30`.
///
/// A fold of the same `step` the chart runtime executes; it assumes finite input.
pub fn obv(close: &[f64], volume: &[f64], ma_period: usize) -> Obv {
    let window = Window {
        close,
        volume,
        ..Window::EMPTY
    };
    let [obv, ma_obv]: [Column; 2] = fold::<State>(close.len(), 2, |st, i, out| {
        step(ma_period, st, &window, i, out);
    })
    .try_into()
    .expect("OBV has two outputs");
    Obv { obv, ma_obv }
}
