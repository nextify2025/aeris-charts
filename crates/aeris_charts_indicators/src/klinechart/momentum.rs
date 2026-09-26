//! `MTM` (动量指标). Ported from KLineChart `src/extension/indicator/momentum.ts`.

use super::{empty, Column};

/// MTM outputs in KLineChart figure order.
#[derive(Clone, Debug, PartialEq)]
pub struct Mtm {
    /// `MTM = CLOSE - REF(CLOSE, N)`.
    pub mtm: Column,
    /// `MAMTM = MA(MTM, M)`.
    pub ma_mtm: Column,
}

/// KLineChart default: `period = 12`, `ma_period = 6`.
pub fn mtm(close: &[f64], period: usize, ma_period: usize) -> Mtm {
    let len = close.len();
    let mut out = Mtm {
        mtm: empty(len),
        ma_mtm: empty(len),
    };
    if period == 0 || ma_period == 0 {
        return out;
    }
    let mut sum = 0.0;
    for i in period..len {
        let value = close[i] - close[i - period];
        out.mtm[i] = Some(value);
        sum += value;
        if i + 1 >= period + ma_period {
            out.ma_mtm[i] = Some(sum / ma_period as f64);
            sum -= out.mtm[i + 1 - ma_period].unwrap_or(0.0);
        }
    }
    out
}
