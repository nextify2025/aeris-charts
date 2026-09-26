//! `VR` (成交量变异率). Ported from KLineChart `src/extension/indicator/volumeRatio.ts`.

use super::{empty, Column};

/// VR outputs in KLineChart figure order.
#[derive(Clone, Debug, PartialEq)]
pub struct Vr {
    /// `VR = (UVS + PVS / 2) / (DVS + PVS / 2) * 100` over `N` rows, where UVS, DVS, and PVS sum the
    /// volume of up, down, and unchanged closes; 0 when the divisor is 0.
    pub vr: Column,
    /// `MAVR = MA(VR, M)`.
    pub ma_vr: Column,
}

/// KLineChart default: `period = 26`, `ma_period = 6`. The first row counts as unchanged.
pub fn vr(close: &[f64], volume: &[f64], period: usize, ma_period: usize) -> Vr {
    let len = close.len();
    let mut out = Vr {
        vr: empty(len),
        ma_vr: empty(len),
    };
    if period == 0 || ma_period == 0 {
        return out;
    }
    let mut uvs = 0.0;
    let mut dvs = 0.0;
    let mut pvs = 0.0;
    let mut sum = 0.0;
    for i in 0..len {
        let prev_close = close[i.saturating_sub(1)];
        if close[i] > prev_close {
            uvs += volume[i];
        } else if close[i] < prev_close {
            dvs += volume[i];
        } else {
            pvs += volume[i];
        }
        if i + 1 < period {
            continue;
        }
        let half_pvs = pvs / 2.0;
        let value = if dvs + half_pvs == 0.0 {
            0.0
        } else {
            ((uvs + half_pvs) / (dvs + half_pvs)) * 100.0
        };
        out.vr[i] = Some(value);
        sum += value;
        if i + 2 >= period + ma_period {
            out.ma_vr[i] = Some(sum / ma_period as f64);
            sum -= out.vr[i + 1 - ma_period].unwrap_or(0.0);
        }
        let ago = i + 1 - period;
        // KLineChart reads `dataList[i - N] ?? dataList[i - (N - 1)]` for the leaving row's
        // previous close.
        let ago_prev_close = if i >= period {
            close[i - period]
        } else {
            close[ago]
        };
        if close[ago] > ago_prev_close {
            uvs -= volume[ago];
        } else if close[ago] < ago_prev_close {
            dvs -= volume[ago];
        } else {
            pvs -= volume[ago];
        }
    }
    out
}
