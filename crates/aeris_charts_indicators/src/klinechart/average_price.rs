//! `AVP` (均价). Ported from KLineChart `src/extension/indicator/averagePrice.ts`.

use super::Column;

/// `AVP = SUM(TURNOVER) / SUM(VOLUME)` from the first row: the running volume-weighted average
/// traded price. Unset until the cumulative volume is non-zero.
pub fn avp(volume: &[f64], turnover: &[f64]) -> Column {
    let mut total_turnover = 0.0;
    let mut total_volume = 0.0;
    volume
        .iter()
        .zip(turnover)
        .map(|(&v, &t)| {
            total_turnover += t;
            total_volume += v;
            (total_volume != 0.0).then(|| total_turnover / total_volume)
        })
        .collect()
}
