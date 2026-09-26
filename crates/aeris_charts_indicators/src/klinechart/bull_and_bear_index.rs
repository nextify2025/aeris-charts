//! `BBI` (多空指标). Ported from KLineChart `src/extension/indicator/bullAndBearIndex.ts`.

use super::{empty, rolling_mean, Column};

/// `BBI = (MA(CLOSE, P1) + MA(CLOSE, P2) + MA(CLOSE, P3) + MA(CLOSE, P4)) / 4`. KLineChart default
/// periods: `[3, 6, 12, 24]`.
///
/// Like KLineChart, the sum is always divided by 4, so the result is only meaningful with exactly
/// four periods.
pub fn bbi(close: &[f64], periods: &[usize]) -> Column {
    let mut out = empty(close.len());
    let Some(&max_period) = periods.iter().max() else {
        return out;
    };
    if periods.contains(&0) {
        return out;
    }
    let means: Vec<Column> = periods
        .iter()
        .map(|&period| rolling_mean(close, period))
        .collect();
    for (i, slot) in out.iter_mut().enumerate().skip(max_period - 1) {
        let mut sum = 0.0;
        for mean in &means {
            sum += mean[i].unwrap_or(0.0);
        }
        *slot = Some(sum / 4.0);
    }
    out
}
