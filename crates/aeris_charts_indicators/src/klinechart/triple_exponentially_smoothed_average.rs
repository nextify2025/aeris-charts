//! `TRIX` (三重指数平滑平均线). Ported from KLineChart
//! `src/extension/indicator/tripleExponentiallySmoothedAverage.ts`.

use super::stepper::{fold, rolling_mean_step, Out, Window};
use super::Column;

/// TRIX outputs in KLineChart figure order.
#[derive(Clone, Debug, PartialEq)]
pub struct Trix {
    /// `TRIX = (TR - REF(TR, 1)) / REF(TR, 1) * 100`, where `TR = EMA(EMA(EMA(CLOSE, N), N), N)`
    /// and each EMA is seeded with the simple average of its first `N` inputs. KLineChart reports 0
    /// on the first `TR` row.
    pub trix: Column,
    /// `MATRIX = MA(TRIX, M)`.
    pub ma_trix: Column,
}

/// The three-stage EMA chain that produces TRIX: the running sum of close, the three seeded EMAs
/// (`ema1`, `ema2` and `TR` itself, kept as `old_tr`) and the sums that seed the second and third.
#[derive(Clone, Copy, Debug, Default)]
struct Stage {
    close_sum: f64,
    ema1: f64,
    ema2: f64,
    old_tr: f64,
    ema1_sum: f64,
    ema2_sum: f64,
}

/// `lead` produces the TRIX of the current row. The average of TRIX drops the value `ma_period - 1`
/// rows back, so `shadow` is a second copy of the same chain run that far behind: it consumes the
/// same rows with the same arithmetic and hands back that value bit for bit (an `inf` or NaN
/// included), whatever the period. `sum` is the running sum of the TRIX values inside the average's
/// window.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct State {
    lead: Stage,
    shadow: Stage,
    sum: f64,
}

/// Only the shadow reads the window: the close `ma_period - 1` rows back.
pub(super) fn lookback(_period: usize, ma_period: usize) -> usize {
    ma_period.saturating_sub(1)
}

/// Advances the chain by the valid row `row` and returns that row's TRIX, unset until the third EMA
/// exists.
fn trix_stage(stage: &mut Stage, w: &Window<'_>, row: usize, period: usize) -> Option<f64> {
    let n = period as f64;
    let c = w.c(row);
    stage.close_sum += c;
    if row + 1 < period {
        return None;
    }
    stage.ema1 = if row + 1 > period {
        (2.0 * c + (n - 1.0) * stage.ema1) / (n + 1.0)
    } else {
        stage.close_sum / n
    };
    stage.ema1_sum += stage.ema1;
    if row + 2 < period * 2 {
        return None;
    }
    stage.ema2 = if row + 2 > period * 2 {
        (2.0 * stage.ema1 + (n - 1.0) * stage.ema2) / (n + 1.0)
    } else {
        stage.ema1_sum / n
    };
    stage.ema2_sum += stage.ema2;
    if row + 3 < period * 3 {
        return None;
    }
    let (tr, value) = if row + 3 > period * 3 {
        let tr = (2.0 * stage.ema2 + (n - 1.0) * stage.old_tr) / (n + 1.0);
        (tr, ((tr - stage.old_tr) / stage.old_tr) * 100.0)
    } else {
        (stage.ema2_sum / n, 0.0)
    };
    stage.old_tr = tr;
    Some(value)
}

/// Advances the formula by the valid row `i`. A zero period leaves both outputs unset.
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
    let value = trix_stage(&mut st.lead, w, i, period);
    // The shadow consumes one row every tick, read or not, so it never misses a row.
    let leaving = if i + 1 >= ma_period {
        trix_stage(&mut st.shadow, w, i + 1 - ma_period, period)
    } else {
        None
    };
    let Some(value) = value else {
        return;
    };
    out[0] = Some(value);
    // The average counts rows from the first TRIX row, `3 * period - 3`.
    out[1] = rolling_mean_step(&mut st.sum, i + 3 - period * 3, ma_period, value, || {
        leaving.unwrap_or(0.0)
    });
}

/// KLineChart default: `period = 12`, `ma_period = 9`.
///
/// A fold of the same `step` the chart runtime executes; it assumes finite input.
pub fn trix(close: &[f64], period: usize, ma_period: usize) -> Trix {
    let window = Window {
        close,
        ..Window::EMPTY
    };
    let [trix, ma_trix]: [Column; 2] = fold::<State>(close.len(), 2, |st, i, out| {
        step(period, ma_period, st, &window, i, out);
    })
    .try_into()
    .expect("a fold of two outputs has two columns");
    Trix { trix, ma_trix }
}
