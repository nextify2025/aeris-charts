//! `CR` (带状能量线). Ported from KLineChart `src/extension/indicator/currentRatio.ts`.

use super::{empty, Column};

/// CR outputs in KLineChart figure order.
#[derive(Clone, Debug, PartialEq)]
pub struct Cr {
    /// `CR = SUM(MAX(0, HIGH - MID), N) / SUM(MAX(0, MID - LOW), N) * 100`, where
    /// `MID = REF(HIGH + LOW, 1) / 2`; 0 when the divisor is 0.
    pub cr: Column,
    /// `MAi = REF(MA(CR, Mi), CEIL(Mi / 2.5 + 1))` for the four moving-average periods.
    pub ma: [Column; 4],
}

/// KLineChart default: `period = 26`, `ma_periods = [10, 20, 40, 60]`.
///
/// As in KLineChart, each shifted moving average starts one row after its first full window plus
/// shift, because the reference reads one element before the start of its history on that row.
pub fn cr(high: &[f64], low: &[f64], period: usize, ma_periods: [usize; 4]) -> Cr {
    let len = high.len();
    let mut out = Cr {
        cr: empty(len),
        ma: [empty(len), empty(len), empty(len), empty(len)],
    };
    if period == 0 || ma_periods.contains(&0) {
        return out;
    }
    let mid = |i: usize| {
        let prev = i.saturating_sub(1);
        (high[prev] + low[prev]) / 2.0
    };
    let forward = ma_periods.map(forward_shift);
    let mut ma_sums = [0.0; 4];
    let mut ma_lists: [Vec<f64>; 4] = Default::default();
    let mut high_sub_sum = 0.0;
    let mut mid_sub_sum = 0.0;
    for i in 0..len {
        let prev_mid = mid(i);
        high_sub_sum += 0f64.max(high[i] - prev_mid);
        mid_sub_sum += 0f64.max(prev_mid - low[i]);
        if i >= period {
            let out_row = i - period;
            // KLineChart reads `dataList[i - N - 1] ?? dataList[i - N]` for the leaving row's MID.
            let out_prev_mid = mid(out_row);
            high_sub_sum -= 0f64.max(high[out_row] - out_prev_mid);
            mid_sub_sum -= 0f64.max(out_prev_mid - low[out_row]);
        }
        if i + 1 < period {
            continue;
        }
        let cr = if mid_sub_sum != 0.0 {
            (high_sub_sum / mid_sub_sum) * 100.0
        } else {
            0.0
        };
        out.cr[i] = Some(cr);
        for k in 0..4 {
            let m = ma_periods[k];
            ma_sums[k] += cr;
            if i + 2 >= period + m {
                ma_lists[k].push(ma_sums[k] / m as f64);
                let list = &ma_lists[k];
                if i + 3 >= period + m + forward[k] && list.len() > forward[k] {
                    out.ma[k][i] = Some(list[list.len() - 1 - forward[k]]);
                }
                ma_sums[k] -= out.cr[i + 1 - m].unwrap_or(0.0);
            }
        }
    }
    out
}

/// How many rows a CR moving average is shifted by: `CEIL(M / 2.5 + 1)`.
pub(super) fn forward_shift(ma_period: usize) -> usize {
    (ma_period as f64 / 2.5 + 1.0).ceil() as usize
}
