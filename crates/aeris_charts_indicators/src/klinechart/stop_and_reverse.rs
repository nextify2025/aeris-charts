//! `SAR` (抛物线指标). Ported from KLineChart `src/extension/indicator/stopAndReverse.ts`.

use super::Column;

/// Parabolic SAR with KLineChart's percent-unit parameters. KLineChart default: `start = 2`,
/// `step = 2`, `max = 20`, meaning an acceleration factor starting at 0.02, stepping by 0.02, and
/// capped at 0.20.
///
/// Every row has a value. KLineChart colors a dot with its up color when `SAR < (HIGH + LOW) / 2`,
/// otherwise with its down color; that is presentation and is left to the renderer.
pub fn sar(high: &[f64], low: &[f64], start: f64, step: f64, max: f64) -> Column {
    const UNSET: f64 = -100.0;
    let start_af = start / 100.0;
    let step = step / 100.0;
    let max_af = max / 100.0;
    let mut af = start_af;
    let mut ep = UNSET;
    let mut is_increasing = false;
    let mut sar = 0.0;
    (0..high.len())
        .map(|i| {
            let prev_sar = sar;
            let prev = i.max(1) - 1;
            if is_increasing {
                if ep == UNSET || ep < high[i] {
                    ep = high[i];
                    af = (af + step).min(max_af);
                }
                sar = prev_sar + af * (ep - prev_sar);
                let low_min = low[prev].min(low[i]);
                if sar > low[i] {
                    sar = ep;
                    af = start_af;
                    ep = UNSET;
                    is_increasing = !is_increasing;
                } else if sar > low_min {
                    sar = low_min;
                }
            } else {
                if ep == UNSET || ep > low[i] {
                    ep = low[i];
                    af = (af + step).min(max_af);
                }
                sar = prev_sar + af * (ep - prev_sar);
                let high_max = high[prev].max(high[i]);
                if sar < high[i] {
                    sar = ep;
                    af = start_af;
                    ep = UNSET;
                    is_increasing = !is_increasing;
                } else if sar < high_max {
                    sar = high_max;
                }
            }
            Some(sar)
        })
        .collect()
}
