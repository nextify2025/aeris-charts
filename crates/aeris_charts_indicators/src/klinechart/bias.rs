//! `BIAS` (乖离率). Ported from KLineChart `src/extension/indicator/bias.ts`.

use super::{empty, Column};

/// `BIAS = (CLOSE - MA(CLOSE, N)) / MA(CLOSE, N) * 100`, one column per period. KLineChart default
/// periods: `[6, 12, 24]`.
pub fn bias(close: &[f64], periods: &[usize]) -> Vec<Column> {
    periods
        .iter()
        .map(|&period| bias_column(close, period))
        .collect()
}

fn bias_column(close: &[f64], period: usize) -> Column {
    let mut out = empty(close.len());
    if period == 0 {
        return out;
    }
    let n = period as f64;
    let mut sum = 0.0;
    for (i, &c) in close.iter().enumerate() {
        sum += c;
        if i + 1 >= period {
            let mean = sum / n;
            out[i] = Some(((c - mean) / mean) * 100.0);
            sum -= close[i + 1 - period];
        }
    }
    out
}
