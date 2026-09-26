//! `SMA` — the weighted `SMA(X, N, M)` smoothing used by Chinese charting software, not the plain
//! rolling mean (that is [`super::ma`]). Ported from KLineChart
//! `src/extension/indicator/simpleMovingAverage.ts`.

use super::{empty, Column};

/// KLineChart default: `period = 12`, `weight = 2`.
///
/// Seeded with the simple average of the first `period` closes, then
/// `SMA = (CLOSE * M + SMA' * (N - M + 1)) / (N + 1)`.
pub fn sma(close: &[f64], period: usize, weight: f64) -> Column {
    let mut out = empty(close.len());
    if period == 0 {
        return out;
    }
    let n = period as f64;
    let mut close_sum = 0.0;
    let mut value = 0.0;
    for (i, &c) in close.iter().enumerate() {
        close_sum += c;
        if i + 1 >= period {
            value = if i + 1 > period {
                (c * weight + value * (n - weight + 1.0)) / (n + 1.0)
            } else {
                close_sum / n
            };
            out[i] = Some(value);
        }
    }
    out
}
