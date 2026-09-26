//! `EMA` (指数移动平均). Ported from KLineChart
//! `src/extension/indicator/exponentialMovingAverage.ts`.

use super::{empty, Column};

/// Exponential moving averages of `close`, one column per period. KLineChart default periods:
/// `[6, 12, 20]`.
///
/// Each EMA is seeded with the simple average of its first `N` closes, then follows
/// `EMA = (2 * CLOSE + (N - 1) * EMA') / (N + 1)`.
pub fn ema(close: &[f64], periods: &[usize]) -> Vec<Column> {
    periods
        .iter()
        .map(|&period| seeded_ema(close, period))
        .collect()
}

/// One KLineChart EMA column; shared with MACD and TRIX, which use the same recursion.
pub(super) fn seeded_ema(values: &[f64], period: usize) -> Column {
    let mut out = empty(values.len());
    if period == 0 {
        return out;
    }
    let n = period as f64;
    let mut sum = 0.0;
    let mut ema = 0.0;
    for (i, &value) in values.iter().enumerate() {
        sum += value;
        if i + 1 >= period {
            ema = if i + 1 > period {
                (2.0 * value + (n - 1.0) * ema) / (n + 1.0)
            } else {
                sum / n
            };
            out[i] = Some(ema);
        }
    }
    out
}
