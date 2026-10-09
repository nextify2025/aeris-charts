//! `RSI` (相对强弱指标). Ported from KLineChart
//! `src/extension/indicator/relativeStrengthIndex.ts`.

use super::Column;
use super::stepper::{Out, Window, fold};
use crate::MAX_OUTPUTS;

/// The Wilder state of one period: the running sums that seed the averages on row `N`, then the
/// smoothed gain and loss.
#[derive(Clone, Copy, Debug, Default)]
struct Wilder {
    gain_sum: f64,
    loss_sum: f64,
    avg_gain: f64,
    avg_loss: f64,
}

/// One Wilder state per period (four scalars each).
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct State {
    wilder: [Wilder; MAX_OUTPUTS],
}

/// Every period reads the previous close only, whatever its length.
pub(super) fn lookback(_periods: &[usize]) -> usize {
    1
}

/// Advances every Wilder average by the valid row `i`. A zero period leaves its output unset.
pub(super) fn step(periods: &[usize], st: &mut State, w: &Window<'_>, i: usize, out: &mut Out) {
    let change = if i == 0 { 0.0 } else { w.c(i) - w.c(i - 1) };
    let gain = change.max(0.0);
    let loss = (-change).max(0.0);
    for ((slot, wilder), &period) in out.iter_mut().zip(&mut st.wilder).zip(periods) {
        if period == 0 {
            continue;
        }
        let n = period as f64;
        wilder.gain_sum += gain;
        wilder.loss_sum += loss;
        if i < period {
            continue;
        }
        let (avg_gain, avg_loss) = if i == period {
            (wilder.gain_sum / n, wilder.loss_sum / n)
        } else {
            (
                (wilder.avg_gain * (n - 1.0) + gain) / n,
                (wilder.avg_loss * (n - 1.0) + loss) / n,
            )
        };
        wilder.avg_gain = avg_gain;
        wilder.avg_loss = avg_loss;
        *slot = Some(if avg_loss == 0.0 {
            100.0
        } else if avg_gain == 0.0 {
            0.0
        } else {
            100.0 - 100.0 / (1.0 + avg_gain / avg_loss)
        });
    }
}

/// Wilder RSI of `close`, one column per period. KLineChart default periods: `[6, 12, 24]`.
///
/// `RSI = 100 - 100 / (1 + RMA(MAX(CHANGE, 0), N) / RMA(MAX(-CHANGE, 0), N))`. The averages are
/// seeded with the plain mean of the first `N` changes, so the first value is on row `N` (the first
/// row has no change). A zero loss average gives 100, then a zero gain average gives 0.
///
/// A fold of the same `step` the chart runtime executes; it assumes finite input.
pub fn rsi(close: &[f64], periods: &[usize]) -> Vec<Column> {
    let window = Window {
        close,
        ..Window::EMPTY
    };
    periods
        .chunks(MAX_OUTPUTS)
        .flat_map(|periods| {
            fold::<State>(close.len(), periods.len(), |st, i, out| {
                step(periods, st, &window, i, out);
            })
        })
        .collect()
}
