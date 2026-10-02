//! `WR` (威廉指标). Ported from KLineChart `src/extension/indicator/williamsR.ts`.

use super::{empty, highest_high_lowest_low, Column};

/// `WR = (CLOSE - HHV(HIGH, N)) / (HHV(HIGH, N) - LLV(LOW, N)) * 100` (from -100 to 0), one column
/// per period; 0 when the range is 0. KLineChart default periods: `[6, 10, 14]`.
pub fn wr(high: &[f64], low: &[f64], close: &[f64], periods: &[usize]) -> Vec<Column> {
    periods
        .iter()
        .map(|&period| {
            let mut out = empty(close.len());
            if period == 0 {
                return out;
            }
            for i in period - 1..close.len() {
                let start = i + 1 - period;
                let (hn, ln) = highest_high_lowest_low(&high[start..=i], &low[start..=i]);
                let range = hn - ln;
                out[i] = Some(if range == 0.0 {
                    0.0
                } else {
                    ((close[i] - hn) / range) * 100.0
                });
            }
            out
        })
        .collect()
}
