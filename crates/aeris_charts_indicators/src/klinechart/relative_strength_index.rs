//! `RSI` (相对强弱指标). Ported from KLineChart
//! `src/extension/indicator/relativeStrengthIndex.ts`.

use super::{empty, Column};

/// Wilder RSI of `close`, one column per period. KLineChart default periods: `[6, 12, 24]`.
///
/// `RSI = 100 - 100 / (1 + RMA(MAX(CHANGE, 0), N) / RMA(MAX(-CHANGE, 0), N))`. The averages are
/// seeded with the plain mean of the first `N` changes, so the first value is on row `N` (the first
/// row has no change). A zero loss average gives 100, then a zero gain average gives 0.
pub fn rsi(close: &[f64], periods: &[usize]) -> Vec<Column> {
    periods
        .iter()
        .map(|&period| rsi_column(close, period))
        .collect()
}

fn rsi_column(close: &[f64], period: usize) -> Column {
    let mut out = empty(close.len());
    if period == 0 {
        return out;
    }
    let n = period as f64;
    let mut gain_sum = 0.0;
    let mut loss_sum = 0.0;
    let mut averages: Option<(f64, f64)> = None;
    for (i, &c) in close.iter().enumerate() {
        let change = if i == 0 { 0.0 } else { c - close[i - 1] };
        let gain = change.max(0.0);
        let loss = (-change).max(0.0);
        gain_sum += gain;
        loss_sum += loss;
        if i < period {
            continue;
        }
        let (avg_gain, avg_loss) = match averages {
            None => (gain_sum / n, loss_sum / n),
            Some((g, l)) => ((g * (n - 1.0) + gain) / n, (l * (n - 1.0) + loss) / n),
        };
        averages = Some((avg_gain, avg_loss));
        out[i] = Some(if avg_loss == 0.0 {
            100.0
        } else if avg_gain == 0.0 {
            0.0
        } else {
            100.0 - 100.0 / (1.0 + avg_gain / avg_loss)
        });
    }
    out
}
