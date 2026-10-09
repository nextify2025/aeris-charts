//! `PVT` (价量趋势). Ported from KLineChart `src/extension/indicator/priceAndVolumeTrend.ts`.

use super::Column;
use super::stepper::{Out, Window, fold};

/// The running total.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct State {
    sum: f64,
}

/// A row reads the close one valid row before it.
pub(super) fn lookback() -> usize {
    1
}

/// Adds the valid row `i` to the running total. The first row uses itself as the previous row.
pub(super) fn step(st: &mut State, w: &Window<'_>, i: usize, out: &mut Out) {
    let prev_close = w.c(i.saturating_sub(1));
    if prev_close != 0.0 {
        st.sum += ((w.c(i) - prev_close) / prev_close) * w.v(i);
    } else {
        st.sum += 0.0;
    }
    out[0] = Some(st.sum);
}

/// `PVT = SUM((CLOSE - REF(CLOSE, 1)) / REF(CLOSE, 1) * VOLUME)` from the first row, skipping rows
/// whose previous close is 0. KLineChart treats a missing volume as 1; callers pass 1 for such rows.
///
/// A fold of the same `step` the chart runtime executes; it assumes finite input. Rows past the end
/// of a `volume` shorter than `close` read 1, the missing-volume value.
pub fn pvt(close: &[f64], volume: &[f64]) -> Column {
    let window = Window {
        close,
        volume,
        missing_volume: 1.0,
        ..Window::EMPTY
    };
    fold::<State>(close.len(), 1, |st, i, out| step(st, &window, i, out)).swap_remove(0)
}
