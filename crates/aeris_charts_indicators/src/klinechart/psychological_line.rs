//! `PSY` (心理线). Ported from KLineChart `src/extension/indicator/psychologicalLine.ts`.

use super::stepper::{fold, Out, Window};
use super::Column;

/// PSY outputs in KLineChart figure order.
#[derive(Clone, Debug, PartialEq)]
pub struct Psy {
    /// `PSY = COUNT(CLOSE > REF(CLOSE, 1), N) / N * 100`.
    pub psy: Column,
    /// `MAPSY = MA(PSY, M)`.
    pub ma_psy: Column,
}

/// The up-count, its lagging copy and the sum the moving average divides.
///
/// `MAPSY` subtracts the PSY value that leaves its window `M - 1` rows back. PSY is a recursion of
/// the input rows, so a second up-count advanced over the rows `M - 1` behind the current one
/// yields exactly that value, bit for bit, without a ring and in a size independent of `N` and `M`.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct State {
    up_count: f64,
    shadow_up_count: f64,
    sum: f64,
}

/// The lagging up-count at row `i + 1 - M` drops the row `N` before it, whose previous close is one
/// further back.
pub(super) fn lookback(period: usize, ma_period: usize) -> usize {
    if period == 0 || ma_period == 0 {
        0
    } else {
        period + ma_period - 1
    }
}

/// `1` when valid row `k` closes above the row before it (the first row compares with itself).
fn up(w: &Window<'_>, k: usize) -> f64 {
    if w.c(k) - w.c(k.saturating_sub(1)) > 0.0 {
        1.0
    } else {
        0.0
    }
}

/// One row of the PSY recursion: adds row `j` to the up-count and, once the window is full,
/// returns PSY and drops the row leaving the window.
fn psy_stage(up_count: &mut f64, w: &Window<'_>, j: usize, period: usize) -> Option<f64> {
    *up_count += up(w, j);
    if j + 1 < period {
        return None;
    }
    let value = (*up_count / period as f64) * 100.0;
    *up_count -= up(w, j + 1 - period);
    Some(value)
}

/// Advances PSY and its moving average by the valid row `i`. A zero period leaves both unset.
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
    let value = psy_stage(&mut st.up_count, w, i, period);
    // The row `i + 1 - M` is the first the average drops; the copy consumes every row from it,
    // whether or not this row reads its value.
    let leaving = if i + 1 >= ma_period {
        psy_stage(&mut st.shadow_up_count, w, i + 1 - ma_period, period)
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

/// KLineChart default: `period = 12`, `ma_period = 6`.
///
/// A fold of the same `step` the chart runtime executes; it assumes finite input.
pub fn psy(close: &[f64], period: usize, ma_period: usize) -> Psy {
    let window = Window {
        close,
        ..Window::EMPTY
    };
    let [psy, ma_psy]: [Column; 2] = fold::<State>(close.len(), 2, |st, i, out| {
        step(period, ma_period, st, &window, i, out);
    })
    .try_into()
    .expect("PSY has two outputs");
    Psy { psy, ma_psy }
}
