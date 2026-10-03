//! `AVP` (均价). Ported from KLineChart `src/extension/indicator/averagePrice.ts`.

use super::stepper::{fold, Out, Window};
use super::Column;

/// The cumulative turnover and volume.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct State {
    total_turnover: f64,
    total_volume: f64,
}

/// A row reads only itself.
pub(super) fn lookback() -> usize {
    0
}

/// Adds the valid row `i` to the totals. The output stays unset while the cumulative volume is 0.
pub(super) fn step(st: &mut State, w: &Window<'_>, i: usize, out: &mut Out) {
    st.total_turnover += w.t(i);
    st.total_volume += w.v(i);
    if st.total_volume != 0.0 {
        out[0] = Some(st.total_turnover / st.total_volume);
    }
}

/// `AVP = SUM(TURNOVER) / SUM(VOLUME)` from the first row: the running volume-weighted average
/// traded price. Unset until the cumulative volume is non-zero.
///
/// A fold of the same `step` the chart runtime executes; it assumes finite input. The result has
/// as many rows as the shorter of the two columns.
pub fn avp(volume: &[f64], turnover: &[f64]) -> Column {
    let window = Window {
        close: turnover,
        volume,
        ..Window::EMPTY
    };
    fold::<State>(volume.len().min(turnover.len()), 1, |st, i, out| {
        step(st, &window, i, out);
    })
    .swap_remove(0)
}
