//! `PVT` (价量趋势). Ported from KLineChart `src/extension/indicator/priceAndVolumeTrend.ts`.

use super::Column;

/// `PVT = SUM((CLOSE - REF(CLOSE, 1)) / REF(CLOSE, 1) * VOLUME)` from the first row, skipping rows
/// whose previous close is 0. KLineChart treats a missing volume as 1; callers pass 1 for such rows.
pub fn pvt(close: &[f64], volume: &[f64]) -> Column {
    let mut sum = 0.0;
    close
        .iter()
        .enumerate()
        .map(|(i, &c)| {
            let prev_close = close[i.saturating_sub(1)];
            if prev_close != 0.0 {
                sum += ((c - prev_close) / prev_close) * volume[i];
            } else {
                sum += 0.0;
            }
            Some(sum)
        })
        .collect()
}
