//! `AO` (动量震荡指标). Ported from KLineChart `src/extension/indicator/awesomeOscillator.ts`.

use super::{empty, rolling_mean, Column};

/// `AO = MA(MEDIAN, SHORT) - MA(MEDIAN, LONG)` with `MEDIAN = (HIGH + LOW) / 2`. KLineChart default:
/// `short = 5`, `long = 34`.
///
/// KLineChart draws AO as bars, hollow when rising and filled when falling; that is presentation
/// and is left to the renderer.
pub fn ao(high: &[f64], low: &[f64], short: usize, long: usize) -> Column {
    let len = high.len();
    let mut out = empty(len);
    if short == 0 || long == 0 {
        return out;
    }
    let median: Vec<f64> = (0..len).map(|i| (low[i] + high[i]) / 2.0).collect();
    let ma_short = rolling_mean(&median, short);
    let ma_long = rolling_mean(&median, long);
    for i in short.max(long) - 1..len {
        out[i] = Some(ma_short[i].unwrap_or(0.0) - ma_long[i].unwrap_or(0.0));
    }
    out
}
