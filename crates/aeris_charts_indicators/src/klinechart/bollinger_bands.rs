//! `BOLL` (布林线). Ported from KLineChart `src/extension/indicator/bollingerBands.ts`.

use super::Column;
use super::stepper::{Out, Window, fold, rolling_mean_step};

/// Bollinger bands in KLineChart figure order.
#[derive(Clone, Debug, PartialEq)]
pub struct Boll {
    /// `MID + K * MD`.
    pub up: Column,
    /// `MID`: the `N`-row rolling mean of the close.
    pub mid: Column,
    /// `MID - K * MD`.
    pub dn: Column,
}

/// The running sum of the closes inside the window. A running add/subtract sum cannot be recomputed
/// from its window bit for bit, so it lives in the checkpointed state; the deviation is a scan of
/// the window and needs no state.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct State {
    sum: f64,
}

/// The leaving close and the first close of the deviation scan are `period - 1` rows back.
pub(super) fn lookback(period: usize, _multiplier: f64) -> usize {
    period.saturating_sub(1)
}

/// KLineChart's `getBollMd`.
fn deviation(window: &[f64], mean: f64) -> f64 {
    let mut sum = 0.0;
    for &c in window {
        let diff = c - mean;
        sum += diff * diff;
    }
    (sum.abs() / window.len() as f64).sqrt()
}

/// Advances the formula by the valid row `i`. A zero period leaves every output unset.
pub(super) fn step(
    period: usize,
    multiplier: f64,
    st: &mut State,
    w: &Window<'_>,
    i: usize,
    out: &mut Out,
) {
    if period == 0 {
        return;
    }
    let Some(mid) = rolling_mean_step(&mut st.sum, i, period, w.c(i), || w.c(i + 1 - period))
    else {
        return;
    };
    let md = deviation(w.cs(i + 1 - period, i + 1), mid);
    out[1] = Some(mid);
    out[0] = Some(mid + multiplier * md);
    out[2] = Some(mid - multiplier * md);
}

/// KLineChart default: `period = 20`, `multiplier = 2`. `MD` is the population standard deviation
/// of the window's closes around `MID`.
///
/// A fold of the same `step` the chart runtime executes; it assumes finite input.
pub fn boll(close: &[f64], period: usize, multiplier: f64) -> Boll {
    let window = Window {
        close,
        ..Window::EMPTY
    };
    let [up, mid, dn]: [Column; 3] = fold::<State>(close.len(), 3, |st, i, out| {
        step(period, multiplier, st, &window, i, out);
    })
    .try_into()
    .expect("a fold of three outputs has three columns");
    Boll { up, mid, dn }
}
