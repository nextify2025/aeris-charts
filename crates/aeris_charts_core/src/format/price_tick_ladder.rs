//! Price-band tick-size ladder: an exchange spread table such as the HKEX table or the US
//! sub-dollar rule (`< $1 -> 0.0001`, `>= $1 -> 0.01`). One ladder drives label rounding and
//! per-band precision, the axis tick grid, and trading price snapping, so every surface agrees on
//! which prices are tradable.

use crate::format::price_formatter::{precision_by_min_move, PriceFormatter};
use crate::scale::price_tick_span_calculator::is_multiple_of;

/// Upper bound on bands per ladder; real spread tables have at most a dozen.
pub const MAX_PRICE_TICK_BANDS: usize = 64;

/// One band of a [`PriceTickLadder`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PriceTickBand {
    /// Inclusive lower bound of the band's absolute price. The first band also covers every
    /// lower price.
    pub from: f64,
    /// Tick size of the band.
    pub min_move: f64,
    /// Label decimals for prices in the band (derived from `min_move` when the host omits it).
    pub precision: u32,
}

impl PriceTickBand {
    /// A band whose precision derives from its tick size.
    pub fn new(from: f64, min_move: f64) -> Self {
        Self {
            from,
            min_move,
            precision: precision_by_min_move(min_move),
        }
    }
}

/// Validated, ascending price bands. Band boundaries lie on the grids of both adjacent bands
/// (true for every exchange spread table), so snapping inside a band can at most land exactly on
/// its boundary, which is tradable on both sides.
#[derive(Clone, Debug, PartialEq)]
pub struct PriceTickLadder {
    bands: Vec<PriceTickBand>,
    /// Cumulative tick index at each band's `from`.
    base_index: Vec<f64>,
}

impl PriceTickLadder {
    pub fn new(bands: Vec<PriceTickBand>) -> Result<Self, &'static str> {
        if bands.is_empty() {
            return Err("a tick ladder needs at least one band");
        }
        if bands.len() > MAX_PRICE_TICK_BANDS {
            return Err("a tick ladder holds at most 64 bands");
        }
        let mut base_index = Vec::with_capacity(bands.len());
        for (index, band) in bands.iter().enumerate() {
            if !band.from.is_finite() || band.from < 0.0 {
                return Err("tick ladder band bounds must be finite and non-negative");
            }
            if !band.min_move.is_finite() || band.min_move <= 0.0 {
                return Err("tick ladder tick sizes must be finite and positive");
            }
            if band.precision > 15 {
                return Err("tick ladder precision must be in 0..=15");
            }
            if band.from != 0.0 && !is_multiple_of(band.from, band.min_move) {
                return Err("a tick ladder band must start on its own tick grid");
            }
            let base = match index {
                0 => (band.from / band.min_move).round(),
                _ => {
                    let previous = bands[index - 1];
                    if band.from <= previous.from {
                        return Err("tick ladder bands must be strictly ascending");
                    }
                    if !is_multiple_of(band.from - previous.from, previous.min_move) {
                        return Err("a tick ladder boundary must lie on the lower band's grid");
                    }
                    base_index[index - 1]
                        + ((band.from - previous.from) / previous.min_move).round()
                }
            };
            base_index.push(base);
        }
        Ok(Self { bands, base_index })
    }

    pub fn bands(&self) -> &[PriceTickBand] {
        &self.bands
    }

    fn band_index(&self, price: f64) -> usize {
        let magnitude = price.abs();
        self.bands
            .partition_point(|band| band.from <= magnitude)
            .saturating_sub(1)
    }

    /// The band that owns `price` (by absolute value).
    pub fn band(&self, price: f64) -> &PriceTickBand {
        &self.bands[self.band_index(price)]
    }

    /// Tick size at `price`.
    pub fn min_move_at(&self, price: f64) -> f64 {
        self.band(price).min_move
    }

    /// Nearest tradable price.
    pub fn snap(&self, price: f64) -> f64 {
        if !price.is_finite() {
            return price;
        }
        let tick = self.min_move_at(price);
        (price / tick).round() * tick
    }

    /// Label for `price`: rounded to its band tick and printed with the precision of the band
    /// that owns the rounded price.
    pub fn format(&self, price: f64) -> String {
        let snapped = self.snap(price);
        let band = self.band(snapped);
        PriceFormatter::from_precision(band.precision, band.min_move).format(snapped)
    }

    /// Exact cumulative tick index of an on-grid price, counting every band's ticks from zero.
    /// `None` for off-grid or non-finite prices.
    pub fn tick_index(&self, price: f64) -> Option<i64> {
        if !price.is_finite() {
            return None;
        }
        let index = self.band_index(price);
        let band = self.bands[index];
        let magnitude = price.abs();
        let offset = (magnitude - band.from) / band.min_move;
        let rounded = offset.round();
        if (offset - rounded).abs() > 1e-6 {
            return None;
        }
        let total = self.base_index[index] + rounded;
        if !total.is_finite() || total.abs() > i64::MAX as f64 / 2.0 {
            return None;
        }
        Some(if price < 0.0 {
            -(total as i64)
        } else {
            total as i64
        })
    }

    /// Price at a cumulative tick index (inverse of [`Self::tick_index`]).
    pub fn price_at_index(&self, index: i64) -> f64 {
        let magnitude = index.unsigned_abs() as f64;
        let band_index = self
            .base_index
            .partition_point(|base| *base <= magnitude)
            .saturating_sub(1);
        let band = self.bands[band_index];
        let price = band.from + (magnitude - self.base_index[band_index]) * band.min_move;
        let price = (price / band.min_move).round() * band.min_move;
        if index < 0 {
            -price
        } else {
            price
        }
    }

    /// Move an on-grid (or snapped) price by `ticks` exact band ticks, crossing band boundaries.
    pub fn step(&self, price: f64, ticks: i64) -> Option<f64> {
        let index = self.tick_index(self.snap(price))?;
        Some(self.price_at_index(index.checked_add(ticks)?))
    }

    /// The grid every tick mark over `[low, high]` must lie on: the least common multiple of the
    /// tick sizes of every band the range touches (the coarsest tick when bands nest, e.g. HKEX
    /// 0.02 and 0.05 need 0.1). Falls back to the coarsest tick when no exact decimal LCM exists.
    pub fn grid_step(&self, low: f64, high: f64) -> f64 {
        let (low, high) = (low.min(high), low.max(high));
        let (min_abs, max_abs) = if low <= 0.0 && high >= 0.0 {
            (0.0, low.abs().max(high.abs()))
        } else {
            (low.abs().min(high.abs()), low.abs().max(high.abs()))
        };
        let first = self.band_index(min_abs);
        let last = self.band_index(max_abs);
        let ticks = &self.bands[first..=last];
        let coarsest = ticks
            .iter()
            .map(|band| band.min_move)
            .fold(0.0_f64, f64::max);
        decimal_lcm(ticks.iter().map(|band| band.min_move)).unwrap_or(coarsest)
    }
}

fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

/// LCM of decimal tick sizes, computed on integers after scaling by the smallest power of ten
/// that makes every tick integral (at most 12 decimals).
fn decimal_lcm(ticks: impl Iterator<Item = f64> + Clone) -> Option<f64> {
    let mut scale = 1.0_f64;
    for _ in 0..=12 {
        if ticks
            .clone()
            .all(|tick| is_multiple_of(tick * scale, 1.0) && tick * scale >= 1.0 - 1e-9)
        {
            let mut lcm = 1_u64;
            for tick in ticks.clone() {
                let integer = (tick * scale).round() as u64;
                lcm = (lcm / gcd(lcm, integer)).checked_mul(integer)?;
            }
            return Some(lcm as f64 / scale);
        }
        scale *= 10.0;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// HKEX spread table (Part A), the canonical multi-band ladder.
    fn hkex() -> PriceTickLadder {
        PriceTickLadder::new(
            [
                (0.0, 0.001),
                (0.25, 0.005),
                (0.5, 0.01),
                (10.0, 0.02),
                (20.0, 0.05),
                (100.0, 0.1),
                (200.0, 0.2),
                (500.0, 0.5),
                (1000.0, 1.0),
                (2000.0, 2.0),
                (5000.0, 5.0),
            ]
            .into_iter()
            .map(|(from, tick)| PriceTickBand::new(from, tick))
            .collect(),
        )
        .unwrap()
    }

    fn us() -> PriceTickLadder {
        PriceTickLadder::new(vec![
            PriceTickBand::new(0.0, 0.0001),
            PriceTickBand::new(1.0, 0.01),
        ])
        .unwrap()
    }

    #[test]
    fn bands_resolve_by_lower_bound_and_derive_precision() {
        let ladder = hkex();
        assert_eq!(ladder.min_move_at(9.99), 0.01);
        assert_eq!(ladder.min_move_at(10.0), 0.02);
        assert_eq!(ladder.min_move_at(15.0), 0.02);
        assert_eq!(ladder.min_move_at(0.1), 0.001);
        assert_eq!(ladder.band(0.3).precision, 3);
        assert_eq!(ladder.band(250.0).precision, 1);
        assert_eq!(us().band(0.5).precision, 4);
        assert_eq!(us().band(1.0).precision, 2);
    }

    #[test]
    fn snapping_and_labels_follow_the_owning_band() {
        let ladder = hkex();
        assert!((ladder.snap(15.251) - 15.26).abs() < 1e-12);
        assert!((ladder.snap(15.249) - 15.24).abs() < 1e-12);
        assert!((ladder.snap(10.013) - 10.02).abs() < 1e-12);
        assert!((ladder.snap(9.996) - 10.0).abs() < 1e-12);
        assert_eq!(ladder.format(9.876), "9.88");
        assert_eq!(ladder.format(10.031), "10.04");
        assert_eq!(ladder.format(0.3012), "0.300");
        assert_eq!(us().format(0.123456), "0.1235");
        assert_eq!(us().format(12.3456), "12.35");
        assert_eq!(us().format(0.99999), "1.00");
    }

    #[test]
    fn tick_index_counts_ticks_across_bands_and_round_trips() {
        let ladder = hkex();
        // 0..0.25 has 250 ticks, 0.25..0.5 has 50, 0.5..10 has 950.
        assert_eq!(ladder.tick_index(0.25), Some(250));
        assert_eq!(ladder.tick_index(0.5), Some(300));
        assert_eq!(ladder.tick_index(10.0), Some(1250));
        assert_eq!(ladder.tick_index(10.02), Some(1251));
        assert_eq!(ladder.tick_index(10.01), None);
        for index in [0, 1, 249, 250, 251, 1249, 1250, 1251, 5000, 9000] {
            let price = ladder.price_at_index(index);
            assert_eq!(
                ladder.tick_index(price),
                Some(index),
                "index {index} price {price}"
            );
        }
        assert!((ladder.step(9.99, 1).unwrap() - 10.0).abs() < 1e-12);
        assert!((ladder.step(10.0, 1).unwrap() - 10.02).abs() < 1e-12);
        assert!((ladder.step(10.02, -2).unwrap() - 9.99).abs() < 1e-12);
    }

    #[test]
    fn grid_step_is_the_lcm_of_the_touched_bands() {
        let ladder = hkex();
        assert!((ladder.grid_step(9.8, 10.4) - 0.02).abs() < 1e-12);
        assert!((ladder.grid_step(15.0, 25.0) - 0.1).abs() < 1e-12);
        assert!((ladder.grid_step(12.0, 14.0) - 0.02).abs() < 1e-12);
        assert!((ladder.grid_step(0.2, 0.3) - 0.005).abs() < 1e-12);
        assert!((ladder.grid_step(4000.0, 6000.0) - 10.0).abs() < 1e-12);
        assert!((us().grid_step(0.9, 1.1) - 0.01).abs() < 1e-12);
        assert!((us().grid_step(0.2, 0.3) - 0.0001).abs() < 1e-15);
    }

    #[test]
    fn invalid_ladders_are_rejected() {
        assert!(PriceTickLadder::new(Vec::new()).is_err());
        assert!(PriceTickLadder::new(vec![
            PriceTickBand::new(1.0, 0.01),
            PriceTickBand::new(0.5, 0.01),
        ])
        .is_err());
        assert!(PriceTickLadder::new(vec![PriceTickBand::new(0.0, 0.0)]).is_err());
        assert!(PriceTickLadder::new(vec![
            PriceTickBand::new(0.0, 0.02),
            PriceTickBand::new(10.01, 0.01),
        ])
        .is_err());
        assert!(PriceTickLadder::new(vec![PriceTickBand::new(0.0, 0.01); 65]).is_err());
    }
}
