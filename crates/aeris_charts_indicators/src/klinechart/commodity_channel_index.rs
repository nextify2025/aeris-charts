//! `CCI` (顺势指标). Ported from KLineChart `src/extension/indicator/commodityChannelIndex.ts`.

use super::{empty, Column};

/// `CCI = (TP - MA(TP, N)) / MD / 0.015`, where `TP = (HIGH + LOW + CLOSE) / 3` and `MD` is the mean
/// absolute deviation of the window's `TP` around `MA(TP, N)`; 0 when `MD` is 0. KLineChart default:
/// `period = 20`.
pub fn cci(high: &[f64], low: &[f64], close: &[f64], period: usize) -> Column {
    let len = close.len();
    let mut out = empty(len);
    if period == 0 {
        return out;
    }
    let n = period as f64;
    let typical: Vec<f64> = (0..len)
        .map(|i| (high[i] + low[i] + close[i]) / 3.0)
        .collect();
    let mut tp_sum = 0.0;
    for (i, &tp) in typical.iter().enumerate() {
        tp_sum += tp;
        if i + 1 >= period {
            let start = i + 1 - period;
            let ma_tp = tp_sum / n;
            let mut abs_sum = 0.0;
            for &window_tp in &typical[start..=i] {
                abs_sum += (window_tp - ma_tp).abs();
            }
            let md = abs_sum / n;
            out[i] = Some(if md != 0.0 {
                (tp - ma_tp) / md / 0.015
            } else {
                0.0
            });
            tp_sum -= typical[start];
        }
    }
    out
}
