//! `ROC` (变动率). Ported from KLineChart `src/extension/indicator/rateOfChange.ts`.

use super::{empty, Column};

/// ROC outputs in KLineChart figure order.
#[derive(Clone, Debug, PartialEq)]
pub struct Roc {
    /// `ROC = (CLOSE - REF(CLOSE, N)) / REF(CLOSE, N) * 100`, 0 when the reference close is 0.
    pub roc: Column,
    /// `MAROC = MA(ROC, M)`.
    pub ma_roc: Column,
}

/// KLineChart default: `period = 12`, `ma_period = 6`.
pub fn roc(close: &[f64], period: usize, ma_period: usize) -> Roc {
    let len = close.len();
    let mut out = Roc {
        roc: empty(len),
        ma_roc: empty(len),
    };
    if period == 0 || ma_period == 0 {
        return out;
    }
    let mut sum = 0.0;
    for i in period..len {
        let ago = close[i - period];
        let value = if ago != 0.0 {
            ((close[i] - ago) / ago) * 100.0
        } else {
            0.0
        };
        out.roc[i] = Some(value);
        sum += value;
        if i + 1 >= period + ma_period {
            out.ma_roc[i] = Some(sum / ma_period as f64);
            sum -= out.roc[i + 1 - ma_period].unwrap_or(0.0);
        }
    }
    out
}
