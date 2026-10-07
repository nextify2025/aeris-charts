//! `WR` (威廉指标). Ported from KLineChart `src/extension/indicator/williamsR.ts`.

use super::stepper::{Out, Window, fold};
use super::{Column, highest_high_lowest_low};
use crate::MAX_OUTPUTS;

/// Stateless: every value is a scan of the window's highs and lows.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct State;

/// The window of the longest period starts `period - 1` rows back.
pub(super) fn lookback(periods: &[usize]) -> usize {
    periods
        .iter()
        .take(MAX_OUTPUTS)
        .copied()
        .max()
        .unwrap_or(0)
        .saturating_sub(1)
}

/// Evaluates every period at the valid row `i`. A zero period leaves its output unset, and so does
/// a period longer than the rows seen so far.
pub(super) fn step(periods: &[usize], _st: &mut State, w: &Window<'_>, i: usize, out: &mut Out) {
    for (slot, &period) in out.iter_mut().zip(periods) {
        if period == 0 || i + 1 < period {
            continue;
        }
        let start = i + 1 - period;
        let (hn, ln) = highest_high_lowest_low(w.highs(start, i + 1), w.lows(start, i + 1));
        let range = hn - ln;
        *slot = Some(if range == 0.0 {
            0.0
        } else {
            ((w.c(i) - hn) / range) * 100.0
        });
    }
}

/// `WR = (CLOSE - HHV(HIGH, N)) / (HHV(HIGH, N) - LLV(LOW, N)) * 100` (from -100 to 0), one column
/// per period; 0 when the range is 0. KLineChart default periods: `[6, 10, 14]`.
///
/// A fold of the same `step` the chart runtime executes; it assumes finite input.
pub fn wr(high: &[f64], low: &[f64], close: &[f64], periods: &[usize]) -> Vec<Column> {
    let window = Window {
        high,
        low,
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
