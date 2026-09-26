//! `MA` (移动平均). Ported from KLineChart `src/extension/indicator/movingAverage.ts`.

use super::{rolling_mean, Column};

/// Rolling means of `close`, one column per period. KLineChart default periods: `[5, 10, 30, 60]`.
pub fn ma(close: &[f64], periods: &[usize]) -> Vec<Column> {
    periods
        .iter()
        .map(|&period| rolling_mean(close, period))
        .collect()
}
