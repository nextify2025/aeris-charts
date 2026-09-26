//! `TRIX` (三重指数平滑平均线). Ported from KLineChart
//! `src/extension/indicator/tripleExponentiallySmoothedAverage.ts`.

use super::{empty, Column};

/// TRIX outputs in KLineChart figure order.
#[derive(Clone, Debug, PartialEq)]
pub struct Trix {
    /// `TRIX = (TR - REF(TR, 1)) / REF(TR, 1) * 100`, where `TR = EMA(EMA(EMA(CLOSE, N), N), N)`
    /// and each EMA is seeded with the simple average of its first `N` inputs. KLineChart reports 0
    /// on the first `TR` row.
    pub trix: Column,
    /// `MATRIX = MA(TRIX, M)`.
    pub ma_trix: Column,
}

/// KLineChart default: `period = 12`, `ma_period = 9`.
pub fn trix(close: &[f64], period: usize, ma_period: usize) -> Trix {
    let len = close.len();
    let mut out = Trix {
        trix: empty(len),
        ma_trix: empty(len),
    };
    if period == 0 || ma_period == 0 {
        return out;
    }
    let n = period as f64;
    let mut close_sum = 0.0;
    let mut ema1 = 0.0;
    let mut ema2 = 0.0;
    let mut old_tr = 0.0;
    let mut ema1_sum = 0.0;
    let mut ema2_sum = 0.0;
    let mut trix_sum = 0.0;
    for (i, &c) in close.iter().enumerate() {
        close_sum += c;
        if i + 1 < period {
            continue;
        }
        ema1 = if i + 1 > period {
            (2.0 * c + (n - 1.0) * ema1) / (n + 1.0)
        } else {
            close_sum / n
        };
        ema1_sum += ema1;
        if i + 2 < period * 2 {
            continue;
        }
        ema2 = if i + 2 > period * 2 {
            (2.0 * ema1 + (n - 1.0) * ema2) / (n + 1.0)
        } else {
            ema1_sum / n
        };
        ema2_sum += ema2;
        if i + 3 < period * 3 {
            continue;
        }
        let (tr, value) = if i + 3 > period * 3 {
            let tr = (2.0 * ema2 + (n - 1.0) * old_tr) / (n + 1.0);
            (tr, ((tr - old_tr) / old_tr) * 100.0)
        } else {
            (ema2_sum / n, 0.0)
        };
        old_tr = tr;
        out.trix[i] = Some(value);
        trix_sum += value;
        if i + 4 >= period * 3 + ma_period {
            out.ma_trix[i] = Some(trix_sum / ma_period as f64);
            trix_sum -= out.trix[i + 1 - ma_period].unwrap_or(0.0);
        }
    }
    out
}
