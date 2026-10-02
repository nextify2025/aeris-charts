//! `EMV` (简易波动指标). Ported from KLineChart
//! `src/extension/indicator/easeOfMovementValue.ts`.

use super::stepper::{fold, Out, Window};
use super::Column;

/// EMV outputs in KLineChart figure order.
#[derive(Clone, Debug, PartialEq)]
pub struct Emv {
    /// `EMV = ((HIGH + LOW) / 2 - REF((HIGH + LOW) / 2, 1)) / (VOLUME / 100000000 / (HIGH - LOW))`,
    /// 0 when the volume or the range is 0. Unset on the first row.
    pub emv: Column,
    /// The mean of the last `period` EMV values.
    pub ma_emv: Column,
}

/// The running sum of the last `period` EMV values. A running add/subtract sum cannot be recomputed
/// from its window bit for bit, so it lives in the checkpointed state.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct State {
    sum: f64,
}

/// The leaving EMV value is the one of row `i + 1 - period`, which reads the row before it.
pub(super) fn lookback(period: usize) -> usize {
    period
}

/// The EMV value of valid row `row` (at least 1): a pure function of that row and the one before
/// it, so the value leaving the average is recomputed from the window instead of being kept.
fn emv_value(w: &Window<'_>, row: usize) -> f64 {
    let distance_moved = (w.h(row) + w.l(row)) / 2.0 - (w.h(row - 1) + w.l(row - 1)) / 2.0;
    if w.v(row) == 0.0 || w.h(row) - w.l(row) == 0.0 {
        0.0
    } else {
        let ratio = w.v(row) / 100_000_000.0 / (w.h(row) - w.l(row));
        distance_moved / ratio
    }
}

/// Advances the formula by the valid row `i`. A zero period leaves both outputs unset, and so does
/// the first row.
pub(super) fn step(period: usize, st: &mut State, w: &Window<'_>, i: usize, out: &mut Out) {
    if period == 0 || i == 0 {
        return;
    }
    let value = emv_value(w, i);
    out[0] = Some(value);
    st.sum += value;
    if i >= period {
        out[1] = Some(st.sum / period as f64);
        st.sum -= emv_value(w, i + 1 - period);
    }
}

/// KLineChart default parameters are `[14, 9]`, but its implementation averages EMV over the first
/// parameter and never reads the second, so only `period` (14) matters here.
///
/// A fold of the same `step` the chart runtime executes; it assumes finite input. A `volume` shorter
/// than `high` reads 0 for the missing rows, the missing-volume value.
pub fn emv(high: &[f64], low: &[f64], volume: &[f64], period: usize) -> Emv {
    let window = Window {
        high,
        low,
        volume,
        ..Window::EMPTY
    };
    let [emv, ma_emv]: [Column; 2] = fold::<State>(high.len(), 2, |st, i, out| {
        step(period, st, &window, i, out);
    })
    .try_into()
    .expect("a fold of two outputs has two columns");
    Emv { emv, ma_emv }
}
