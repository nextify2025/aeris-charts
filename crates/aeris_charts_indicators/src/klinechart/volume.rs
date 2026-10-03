//! `VOL` (成交量). Ported from KLineChart `src/extension/indicator/volume.ts`.

use super::stepper::{fold, rolling_mean_step, Window};
use super::Column;
use crate::MAX_OUTPUTS;

/// One running sum per period. The first output of a VOL binding is the volume bar itself, so
/// the averages take the other `MAX_OUTPUTS - 1` slots.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct State {
    sum: [f64; MAX_OUTPUTS - 1],
}

/// The leaving volume of the longest window is `period - 1` rows back.
pub(super) fn lookback(periods: &[usize]) -> usize {
    periods
        .iter()
        .take(MAX_OUTPUTS - 1)
        .copied()
        .max()
        .unwrap_or(0)
        .saturating_sub(1)
}

/// Advances every volume mean by the valid row `i`; `out[k]` is the mean of `periods[k]`, so the
/// caller passes the outputs after the volume bar. A zero period leaves its output unset.
pub(super) fn step(
    periods: &[usize],
    st: &mut State,
    w: &Window<'_>,
    i: usize,
    out: &mut [Option<f64>],
) {
    for ((slot, sum), &period) in out.iter_mut().zip(&mut st.sum).zip(periods) {
        if period == 0 {
            continue;
        }
        *slot = rolling_mean_step(sum, i, period, w.v(i), || w.v(i + 1 - period));
    }
}

/// Rolling means of `volume`, one column per period. KLineChart default periods: `[5, 10, 20]`.
///
/// KLineChart's VOL also draws the volume itself as bars colored by the candle's direction; that
/// figure is the input column, so it is not repeated here.
///
/// A fold of the same `step` the chart runtime executes; it assumes finite input.
pub fn vol(volume: &[f64], periods: &[usize]) -> Vec<Column> {
    let window = Window {
        volume,
        ..Window::EMPTY
    };
    periods
        .chunks(MAX_OUTPUTS - 1)
        .flat_map(|periods| {
            fold::<State>(volume.len(), periods.len(), |st, i, out| {
                step(periods, st, &window, i, out);
            })
        })
        .collect()
}
