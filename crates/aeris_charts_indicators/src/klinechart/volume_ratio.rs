//! `VR` (成交量变异率). Ported from KLineChart `src/extension/indicator/volumeRatio.ts`.

use super::Column;
use super::stepper::{Out, Window, fold};

/// VR outputs in KLineChart figure order.
#[derive(Clone, Debug, PartialEq)]
pub struct Vr {
    /// `VR = (UVS + PVS / 2) / (DVS + PVS / 2) * 100` over `N` rows, where UVS, DVS, and PVS sum the
    /// volume of up, down, and unchanged closes; 0 when the divisor is 0.
    pub vr: Column,
    /// `MAVR = MA(VR, M)`.
    pub ma_vr: Column,
}

/// The three volume sums of one VR window: up, down and unchanged closes.
#[derive(Clone, Copy, Debug, Default)]
struct Stage {
    uvs: f64,
    dvs: f64,
    pvs: f64,
}

/// The VR recursion, its lagging copy and the sum the moving average divides.
///
/// `MAVR` subtracts the VR value that leaves its window `M - 1` rows back. VR is a recursion of the
/// input rows, so a second set of sums advanced over the rows `M - 1` behind the current one
/// yields exactly that value, bit for bit, without a ring and in a size independent of `N` and `M`.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct State {
    lead: Stage,
    shadow: Stage,
    sum: f64,
}

/// The lagging recursion at row `i + 1 - M` reads the previous close of the row `N` before it.
pub(super) fn lookback(period: usize, ma_period: usize) -> usize {
    if period == 0 || ma_period == 0 {
        0
    } else {
        period + ma_period - 1
    }
}

/// One row of the VR recursion: adds row `j` to the sum of its direction and, once the window is
/// full, returns VR and drops the row leaving the window.
fn vr_stage(stage: &mut Stage, w: &Window<'_>, j: usize, period: usize) -> Option<f64> {
    let prev_close = w.c(j.saturating_sub(1));
    if w.c(j) > prev_close {
        stage.uvs += w.v(j);
    } else if w.c(j) < prev_close {
        stage.dvs += w.v(j);
    } else {
        stage.pvs += w.v(j);
    }
    if j + 1 < period {
        return None;
    }
    let half_pvs = stage.pvs / 2.0;
    let value = if stage.dvs + half_pvs == 0.0 {
        0.0
    } else {
        ((stage.uvs + half_pvs) / (stage.dvs + half_pvs)) * 100.0
    };
    let ago = j + 1 - period;
    // KLineChart reads `dataList[i - N] ?? dataList[i - (N - 1)]` for the leaving row's
    // previous close.
    let ago_prev_close = if j >= period {
        w.c(j - period)
    } else {
        w.c(ago)
    };
    if w.c(ago) > ago_prev_close {
        stage.uvs -= w.v(ago);
    } else if w.c(ago) < ago_prev_close {
        stage.dvs -= w.v(ago);
    } else {
        stage.pvs -= w.v(ago);
    }
    Some(value)
}

/// Advances VR and its moving average by the valid row `i`. A zero period leaves both unset.
pub(super) fn step(
    period: usize,
    ma_period: usize,
    st: &mut State,
    w: &Window<'_>,
    i: usize,
    out: &mut Out,
) {
    if period == 0 || ma_period == 0 {
        return;
    }
    let value = vr_stage(&mut st.lead, w, i, period);
    // The row `i + 1 - M` is the first the average drops; the copy consumes every row from it,
    // whether or not this row reads its value.
    let leaving = if i + 1 >= ma_period {
        vr_stage(&mut st.shadow, w, i + 1 - ma_period, period)
    } else {
        None
    };
    let Some(value) = value else {
        return;
    };
    out[0] = Some(value);
    st.sum += value;
    if i + 2 >= period + ma_period {
        out[1] = Some(st.sum / ma_period as f64);
        st.sum -= leaving.unwrap_or(0.0);
    }
}

/// KLineChart default: `period = 26`, `ma_period = 6`. The first row counts as unchanged.
///
/// A fold of the same `step` the chart runtime executes; it assumes finite input.
pub fn vr(close: &[f64], volume: &[f64], period: usize, ma_period: usize) -> Vr {
    let window = Window {
        close,
        volume,
        ..Window::EMPTY
    };
    let [vr, ma_vr]: [Column; 2] = fold::<State>(close.len(), 2, |st, i, out| {
        step(period, ma_period, st, &window, i, out);
    })
    .try_into()
    .expect("VR has two outputs");
    Vr { vr, ma_vr }
}
