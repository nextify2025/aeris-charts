//! `MACD`. Ported from KLineChart
//! `src/extension/indicator/movingAverageConvergenceDivergence.ts`.

use super::Column;
use super::stepper::{Out, Window, fold, seeded_ema_step};

/// MACD outputs in KLineChart figure order.
#[derive(Clone, Debug, PartialEq)]
pub struct Macd {
    /// `DIF = EMA(CLOSE, SHORT) - EMA(CLOSE, LONG)`.
    pub dif: Column,
    /// `DEA`: the EMA of `DIF` over `SIGNAL` rows, seeded with the average of the first `SIGNAL`
    /// DIF values.
    pub dea: Column,
    /// The histogram, `(DIF - DEA) * 2` (the Chinese-software convention).
    pub macd: Column,
}

/// The two seeded EMAs of close (they advance on every row, warm-up included), the running sum of
/// the DIF values that seeds `DEA`, and `DEA` itself.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct State {
    short_sum: f64,
    short_ema: f64,
    long_sum: f64,
    long_ema: f64,
    dif_sum: f64,
    dea: f64,
}

/// Every read is the current close: the recursions live in the state.
pub(super) fn lookback(_short: usize, _long: usize, _signal: usize) -> usize {
    0
}

/// Advances the formula by the valid row `i`. A zero period leaves every output unset.
pub(super) fn step(
    short: usize,
    long: usize,
    signal: usize,
    st: &mut State,
    w: &Window<'_>,
    i: usize,
    out: &mut Out,
) {
    if short == 0 || long == 0 || signal == 0 {
        return;
    }
    let fast = seeded_ema_step(&mut st.short_sum, &mut st.short_ema, i, short, w.c(i));
    let slow = seeded_ema_step(&mut st.long_sum, &mut st.long_ema, i, long, w.c(i));
    // Both averages exist exactly from row `max(short, long) - 1`, where DIF starts.
    let (Some(fast), Some(slow)) = (fast, slow) else {
        return;
    };
    let max_period = short.max(long);
    let m = signal as f64;
    let dif = fast - slow;
    out[0] = Some(dif);
    st.dif_sum += dif;
    if i + 2 >= max_period + signal {
        st.dea = if i + 2 > max_period + signal {
            (dif * 2.0 + st.dea * (m - 1.0)) / (m + 1.0)
        } else {
            st.dif_sum / m
        };
        out[2] = Some((dif - st.dea) * 2.0);
        out[1] = Some(st.dea);
    }
}

/// KLineChart default: `short = 12`, `long = 26`, `signal = 9`.
///
/// A fold of the same `step` the chart runtime executes; it assumes finite input.
pub fn macd(close: &[f64], short: usize, long: usize, signal: usize) -> Macd {
    let window = Window {
        close,
        ..Window::EMPTY
    };
    let [dif, dea, macd]: [Column; 3] = fold::<State>(close.len(), 3, |st, i, out| {
        step(short, long, signal, st, &window, i, out);
    })
    .try_into()
    .expect("a fold of three outputs has three columns");
    Macd { dif, dea, macd }
}
