//! `PSY` (心理线). Ported from KLineChart `src/extension/indicator/psychologicalLine.ts`.

use super::{empty, Column};

/// PSY outputs in KLineChart figure order.
#[derive(Clone, Debug, PartialEq)]
pub struct Psy {
    /// `PSY = COUNT(CLOSE > REF(CLOSE, 1), N) / N * 100`.
    pub psy: Column,
    /// `MAPSY = MA(PSY, M)`.
    pub ma_psy: Column,
}

/// KLineChart default: `period = 12`, `ma_period = 6`.
pub fn psy(close: &[f64], period: usize, ma_period: usize) -> Psy {
    let len = close.len();
    let mut out = Psy {
        psy: empty(len),
        ma_psy: empty(len),
    };
    if period == 0 || ma_period == 0 {
        return out;
    }
    let up = |i: usize| -> f64 {
        if close[i] - close[i.saturating_sub(1)] > 0.0 {
            1.0
        } else {
            0.0
        }
    };
    let mut up_count = 0.0;
    let mut sum = 0.0;
    for i in 0..len {
        up_count += up(i);
        if i + 1 < period {
            continue;
        }
        let value = (up_count / period as f64) * 100.0;
        out.psy[i] = Some(value);
        sum += value;
        if i + 2 >= period + ma_period {
            out.ma_psy[i] = Some(sum / ma_period as f64);
            sum -= out.psy[i + 1 - ma_period].unwrap_or(0.0);
        }
        up_count -= up(i + 1 - period);
    }
    out
}
