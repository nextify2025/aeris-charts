//! `EMV` (简易波动指标). Ported from KLineChart
//! `src/extension/indicator/easeOfMovementValue.ts`.

use super::{empty, Column};

/// EMV outputs in KLineChart figure order.
#[derive(Clone, Debug, PartialEq)]
pub struct Emv {
    /// `EMV = ((HIGH + LOW) / 2 - REF((HIGH + LOW) / 2, 1)) / (VOLUME / 100000000 / (HIGH - LOW))`,
    /// 0 when the volume or the range is 0. Unset on the first row.
    pub emv: Column,
    /// The mean of the last `period` EMV values.
    pub ma_emv: Column,
}

/// KLineChart default parameters are `[14, 9]`, but its implementation averages EMV over the first
/// parameter and never reads the second, so only `period` (14) matters here.
pub fn emv(high: &[f64], low: &[f64], volume: &[f64], period: usize) -> Emv {
    let len = high.len();
    let mut out = Emv {
        emv: empty(len),
        ma_emv: empty(len),
    };
    if period == 0 {
        return out;
    }
    let mut values: Vec<f64> = Vec::with_capacity(len.saturating_sub(1));
    let mut sum = 0.0;
    for i in 1..len {
        let distance_moved = (high[i] + low[i]) / 2.0 - (high[i - 1] + low[i - 1]) / 2.0;
        let value = if volume[i] == 0.0 || high[i] - low[i] == 0.0 {
            0.0
        } else {
            let ratio = volume[i] / 100_000_000.0 / (high[i] - low[i]);
            distance_moved / ratio
        };
        out.emv[i] = Some(value);
        sum += value;
        values.push(value);
        if i >= period {
            out.ma_emv[i] = Some(sum / period as f64);
            sum -= values[i - period];
        }
    }
    out
}
