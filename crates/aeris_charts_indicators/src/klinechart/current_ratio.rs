//! `CR` (带状能量线). Ported from KLineChart `src/extension/indicator/currentRatio.ts`.

use super::Column;
use super::stepper::{Out, Window, fold};

/// CR outputs in KLineChart figure order.
#[derive(Clone, Debug, PartialEq)]
pub struct Cr {
    /// `CR = SUM(MAX(0, HIGH - MID), N) / SUM(MAX(0, MID - LOW), N) * 100`, where
    /// `MID = REF(HIGH + LOW, 1) / 2`; 0 when the divisor is 0.
    pub cr: Column,
    /// `MAi = REF(MA(CR, Mi), CEIL(Mi / 2.5 + 1))` for the four moving-average periods.
    pub ma: [Column; 4],
}

/// The two running sums that produce CR. A moving average of CR, shifted forward, reads CR from
/// earlier rows, so each of those readers keeps its own copy of this stage running that many rows
/// behind the lead: every copy consumes the same rows through the same recursion, so it yields the
/// lead's earlier CR bit for bit, with a size independent of every period.
#[derive(Clone, Copy, Debug, Default)]
struct Stage {
    high_sub_sum: f64,
    mid_sub_sum: f64,
}

/// One shifted moving average of CR.
#[derive(Clone, Copy, Debug, Default)]
struct MaStage {
    /// CR `forward_shift(m)` rows behind the lead: the row whose moving average is shown now.
    shifted: Stage,
    /// CR `forward_shift(m) + m - 1` rows behind the lead: the value leaving that moving average.
    leaving: Stage,
    /// The running sum of the shifted CR.
    sum: f64,
}

/// The lead stage and the four moving averages.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct State {
    lead: Stage,
    ma: [MaStage; 4],
}

/// The deepest read is the leaving row of a shifted moving average: its stage runs
/// `forward_shift(m) + m - 1` rows behind the lead, and the stage reads the `MID` before its own
/// leaving row, `period + 1` rows further back.
pub(super) fn lookback(period: usize, ma_periods: [usize; 4]) -> usize {
    ma_periods
        .iter()
        .map(|&m| forward_shift(m) + m)
        .max()
        .unwrap_or(0)
        + period
}

/// Advances `stage` by the valid row `j` and returns CR for it once a full window of `period` rows
/// exists.
fn stage_step(stage: &mut Stage, w: &Window<'_>, j: usize, period: usize) -> Option<f64> {
    let mid = |i: usize| {
        let prev = i.saturating_sub(1);
        (w.h(prev) + w.l(prev)) / 2.0
    };
    let prev_mid = mid(j);
    stage.high_sub_sum += 0f64.max(w.h(j) - prev_mid);
    stage.mid_sub_sum += 0f64.max(prev_mid - w.l(j));
    if j >= period {
        let out_row = j - period;
        // KLineChart reads `dataList[i - N - 1] ?? dataList[i - N]` for the leaving row's MID.
        let out_prev_mid = mid(out_row);
        stage.high_sub_sum -= 0f64.max(w.h(out_row) - out_prev_mid);
        stage.mid_sub_sum -= 0f64.max(out_prev_mid - w.l(out_row));
    }
    if j + 1 < period {
        return None;
    }
    Some(if stage.mid_sub_sum != 0.0 {
        (stage.high_sub_sum / stage.mid_sub_sum) * 100.0
    } else {
        0.0
    })
}

/// Advances the formula by the valid row `i`: `out[0]` is CR, `out[1..=4]` its shifted moving
/// averages. A zero period leaves every output unset.
pub(super) fn step(
    period: usize,
    ma_periods: [usize; 4],
    st: &mut State,
    w: &Window<'_>,
    i: usize,
    out: &mut Out,
) {
    if period == 0 || ma_periods.contains(&0) {
        return;
    }
    out[0] = stage_step(&mut st.lead, w, i, period);
    for ((ma, &m), slot) in st.ma.iter_mut().zip(&ma_periods).zip(&mut out[1..]) {
        let forward = forward_shift(m);
        if i < forward {
            continue;
        }
        // The moving average shown at row `i` is the one KLineChart computed at row `shifted`.
        // Both copies consume every row from the first they may read, whether or not it is used.
        let shifted = i - forward;
        let cr = stage_step(&mut ma.shifted, w, shifted, period);
        let leaving =
            (shifted + 1 >= m).then(|| stage_step(&mut ma.leaving, w, shifted + 1 - m, period));
        if let Some(cr) = cr {
            ma.sum += cr;
            if shifted + 2 >= period + m {
                *slot = Some(ma.sum / m as f64);
                ma.sum -= leaving.flatten().unwrap_or(0.0);
            }
        }
    }
}

/// KLineChart default: `period = 26`, `ma_periods = [10, 20, 40, 60]`.
///
/// As in KLineChart, each shifted moving average starts one row after its first full window plus
/// shift, because the reference reads one element before the start of its history on that row.
///
/// A fold of the same `step` the chart runtime executes; it assumes finite input.
pub fn cr(high: &[f64], low: &[f64], period: usize, ma_periods: [usize; 4]) -> Cr {
    let window = Window {
        high,
        low,
        ..Window::EMPTY
    };
    let [cr, ma1, ma2, ma3, ma4] =
        <[Column; 5]>::try_from(fold::<State>(high.len(), 5, |st, i, out| {
            step(period, ma_periods, st, &window, i, out);
        }))
        .expect("cr folds five columns");
    Cr {
        cr,
        ma: [ma1, ma2, ma3, ma4],
    }
}

/// How many rows a CR moving average is shifted by: `CEIL(M / 2.5 + 1)`.
pub(super) fn forward_shift(ma_period: usize) -> usize {
    (ma_period as f64 / 2.5 + 1.0).ceil() as usize
}
