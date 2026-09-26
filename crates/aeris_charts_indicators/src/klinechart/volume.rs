//! `VOL` (成交量). Ported from KLineChart `src/extension/indicator/volume.ts`.

use super::{rolling_mean, Column};

/// Rolling means of `volume`, one column per period. KLineChart default periods: `[5, 10, 20]`.
///
/// KLineChart's VOL also draws the volume itself as bars colored by the candle's direction; that
/// figure is the input column, so it is not repeated here.
pub fn vol(volume: &[f64], periods: &[usize]) -> Vec<Column> {
    periods
        .iter()
        .map(|&period| rolling_mean(volume, period))
        .collect()
}
