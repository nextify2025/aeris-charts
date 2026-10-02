//! `OBV` (能量潮). Ported from KLineChart `src/extension/indicator/onBalanceVolume.ts`.

use super::{empty, Column};

/// OBV outputs in KLineChart figure order.
#[derive(Clone, Debug, PartialEq)]
pub struct Obv {
    /// Cumulative volume, added on up closes and subtracted on down closes; 0 on the first row.
    pub obv: Column,
    /// `MAOBV = MA(OBV, N)`.
    pub ma_obv: Column,
}

/// KLineChart default: `ma_period = 30`.
pub fn obv(close: &[f64], volume: &[f64], ma_period: usize) -> Obv {
    let len = close.len();
    let mut out = Obv {
        obv: empty(len),
        ma_obv: empty(len),
    };
    if ma_period == 0 {
        return out;
    }
    let mut running = 0.0;
    let mut sum = 0.0;
    for i in 0..len {
        let prev_close = close[i.saturating_sub(1)];
        if close[i] < prev_close {
            running -= volume[i];
        } else if close[i] > prev_close {
            running += volume[i];
        }
        out.obv[i] = Some(running);
        sum += running;
        if i + 1 >= ma_period {
            out.ma_obv[i] = Some(sum / ma_period as f64);
            sum -= out.obv[i + 1 - ma_period].unwrap_or(0.0);
        }
    }
    out
}
