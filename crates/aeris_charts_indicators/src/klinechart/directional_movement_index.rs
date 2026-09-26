//! `DMI` (趋向指标). Ported from KLineChart
//! `src/extension/indicator/directionalMovementIndex.ts`.

use super::{empty, Column};

/// DMI outputs in KLineChart figure order.
#[derive(Clone, Debug, PartialEq)]
pub struct Dmi {
    /// `PDI = DMP * 100 / MTR`.
    pub pdi: Column,
    /// `MDI = DMM * 100 / MTR`.
    pub mdi: Column,
    /// `ADX`: the Wilder average of `DX = |MDI - PDI| / (MDI + PDI) * 100` over `N` rows.
    pub adx: Column,
    /// `ADXR = (ADX + REF(ADX, MM - 1)) / 2`.
    pub adxr: Column,
}

/// KLineChart default: `period = 14`, `adxr_period = 6`.
///
/// `MTR`, `DMP`, and `DMM` start as `N`-row sums and then follow the Wilder recursion
/// `X = X' - X' / N + value`. The first row uses itself as the previous row.
pub fn dmi(high: &[f64], low: &[f64], close: &[f64], period: usize, adxr_period: usize) -> Dmi {
    let len = close.len();
    let mut out = Dmi {
        pdi: empty(len),
        mdi: empty(len),
        adx: empty(len),
        adxr: empty(len),
    };
    if period == 0 || adxr_period == 0 {
        return out;
    }
    let n = period as f64;
    let mut tr_sum = 0.0;
    let mut h_sum = 0.0;
    let mut l_sum = 0.0;
    let mut mtr = 0.0;
    let mut dmp = 0.0;
    let mut dmm = 0.0;
    let mut dx_sum = 0.0;
    let mut adx = 0.0;
    for i in 0..len {
        let prev = i.saturating_sub(1);
        let prev_close = close[prev];
        let hl = high[i] - low[i];
        let hcy = (high[i] - prev_close).abs();
        let lcy = (prev_close - low[i]).abs();
        let hhy = high[i] - high[prev];
        let lyl = low[prev] - low[i];
        let tr = hl.max(hcy).max(lcy);
        let h = if hhy > 0.0 && hhy > lyl { hhy } else { 0.0 };
        let l = if lyl > 0.0 && lyl > hhy { lyl } else { 0.0 };
        tr_sum += tr;
        h_sum += h;
        l_sum += l;
        if i + 1 < period {
            continue;
        }
        if i + 1 > period {
            mtr = mtr - mtr / n + tr;
            dmp = dmp - dmp / n + h;
            dmm = dmm - dmm / n + l;
        } else {
            mtr = tr_sum;
            dmp = h_sum;
            dmm = l_sum;
        }
        let (pdi, mdi) = if mtr != 0.0 {
            ((dmp * 100.0) / mtr, (dmm * 100.0) / mtr)
        } else {
            (0.0, 0.0)
        };
        out.pdi[i] = Some(pdi);
        out.mdi[i] = Some(mdi);
        let dx = if mdi + pdi != 0.0 {
            ((mdi - pdi).abs() / (mdi + pdi)) * 100.0
        } else {
            0.0
        };
        dx_sum += dx;
        if i + 2 >= period * 2 {
            adx = if i + 2 > period * 2 {
                (adx * (n - 1.0) + dx) / n
            } else {
                dx_sum / n
            };
            out.adx[i] = Some(adx);
            if i + 3 >= period * 2 + adxr_period {
                let lagged = out.adx[i + 1 - adxr_period].unwrap_or(0.0);
                out.adxr[i] = Some((lagged + adx) / 2.0);
            }
        }
    }
    out
}
