//! Host-neutral Cartesian scales for non-financial series.
//!
//! These scales deliberately operate on scalar or category-index domains. Category labels remain
//! caller-owned, so constructing a scale does not duplicate host strings or allocate category
//! state. They are not wired into the financial pane path.

/// Hard limit for one linear-axis tick calculation.
pub const MAX_GENERAL_TICKS: usize = 512;
pub const DEFAULT_SYMLOG_CONSTANT: f64 = 1.0;

const MAX_EXACT_INTEGER: u128 = 9_007_199_254_740_991;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScaleError {
    NonFinite,
    DegenerateDomain,
    NonPositiveDomain,
    InvalidConstant,
    InvalidPadding,
    InvalidAlign,
    CategoryCountTooLarge,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LogScale {
    domain_from: f64,
    domain_to: f64,
    range_from: f64,
    range_to: f64,
}

impl LogScale {
    pub fn new(
        domain_from: f64,
        domain_to: f64,
        range_from: f64,
        range_to: f64,
    ) -> Result<Self, ScaleError> {
        validate_domain(domain_from, domain_to)?;
        validate_range(range_from, range_to)?;
        if domain_from <= 0.0 || domain_to <= 0.0 {
            return Err(ScaleError::NonPositiveDomain);
        }
        Ok(Self {
            domain_from,
            domain_to,
            range_from,
            range_to,
        })
    }

    pub fn coordinate(&self, value: f64) -> Option<f64> {
        if !value.is_finite() || value <= 0.0 {
            return None;
        }
        let from = self.domain_from.ln();
        let to = self.domain_to.ln();
        let unit = (value.ln() - from) / (to - from);
        let coordinate = self.range_from + unit * (self.range_to - self.range_from);
        coordinate.is_finite().then_some(normalize_zero(coordinate))
    }

    pub fn invert(&self, coordinate: f64) -> Option<f64> {
        if !coordinate.is_finite() || self.range_from == self.range_to {
            return None;
        }
        let unit = (coordinate - self.range_from) / (self.range_to - self.range_from);
        let from = self.domain_from.ln();
        let to = self.domain_to.ln();
        let value = (from + unit * (to - from)).exp();
        (value.is_finite() && value > 0.0).then_some(value)
    }

    pub fn ticks(&self, target_count: usize) -> Vec<f64> {
        if target_count == 0 {
            return Vec::new();
        }
        let low = self.domain_from.min(self.domain_to);
        let high = self.domain_from.max(self.domain_to);
        let first_exp = low.log10().floor() as i32;
        let last_exp = high.log10().ceil() as i32;
        let candidates = (last_exp - first_exp + 1).max(1) as usize * 3;
        let stride = candidates
            .div_ceil(target_count.min(MAX_GENERAL_TICKS))
            .max(1);
        let mut ticks = Vec::with_capacity(target_count.min(MAX_GENERAL_TICKS));
        let mut candidate_index = 0usize;
        for exp in first_exp..=last_exp {
            let power = 10.0_f64.powi(exp);
            if !power.is_finite() || power <= 0.0 {
                continue;
            }
            for factor in [1.0, 2.0, 5.0] {
                let value = factor * power;
                if value >= low
                    && value <= high
                    && candidate_index.is_multiple_of(stride)
                    && ticks.len() < MAX_GENERAL_TICKS
                {
                    ticks.push(value);
                }
                candidate_index = candidate_index.saturating_add(1);
            }
        }
        if ticks.is_empty() {
            ticks.push(low);
            if high != low && ticks.len() < target_count.min(MAX_GENERAL_TICKS) {
                ticks.push(high);
            }
        }
        if self.domain_from > self.domain_to {
            ticks.reverse();
        }
        ticks
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SymLogScale {
    domain_from: f64,
    domain_to: f64,
    range_from: f64,
    range_to: f64,
    constant: f64,
}

impl SymLogScale {
    pub fn new(
        domain_from: f64,
        domain_to: f64,
        range_from: f64,
        range_to: f64,
        constant: f64,
    ) -> Result<Self, ScaleError> {
        validate_domain(domain_from, domain_to)?;
        validate_range(range_from, range_to)?;
        if !constant.is_finite() || constant <= 0.0 {
            return Err(ScaleError::InvalidConstant);
        }
        Ok(Self {
            domain_from,
            domain_to,
            range_from,
            range_to,
            constant,
        })
    }

    fn transform(&self, value: f64) -> Option<f64> {
        if !value.is_finite() {
            return None;
        }
        Some(value.signum() * (1.0 + value.abs() / self.constant).ln())
    }

    fn untransform(&self, value: f64) -> Option<f64> {
        if !value.is_finite() {
            return None;
        }
        let result = value.signum() * self.constant * (value.abs().exp() - 1.0);
        result.is_finite().then_some(normalize_zero(result))
    }

    pub fn coordinate(&self, value: f64) -> Option<f64> {
        let from = self.transform(self.domain_from)?;
        let to = self.transform(self.domain_to)?;
        let transformed = self.transform(value)?;
        let unit = (transformed - from) / (to - from);
        let coordinate = self.range_from + unit * (self.range_to - self.range_from);
        coordinate.is_finite().then_some(normalize_zero(coordinate))
    }

    pub fn invert(&self, coordinate: f64) -> Option<f64> {
        if !coordinate.is_finite() || self.range_from == self.range_to {
            return None;
        }
        let from = self.transform(self.domain_from)?;
        let to = self.transform(self.domain_to)?;
        let unit = (coordinate - self.range_from) / (self.range_to - self.range_from);
        self.untransform(from + unit * (to - from))
    }

    pub fn ticks(&self, target_count: usize) -> Vec<f64> {
        if target_count == 0 {
            return Vec::new();
        }
        let Some(from) = self.transform(self.domain_from) else {
            return Vec::new();
        };
        let Some(to) = self.transform(self.domain_to) else {
            return Vec::new();
        };
        let Ok(linear) = LinearScale::new(from, to, 0.0, 1.0) else {
            return Vec::new();
        };
        linear
            .ticks(target_count)
            .into_iter()
            .filter_map(|tick| self.untransform(tick))
            .collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LinearScale {
    domain_from: f64,
    domain_to: f64,
    range_from: f64,
    range_to: f64,
}

impl LinearScale {
    pub fn new(
        domain_from: f64,
        domain_to: f64,
        range_from: f64,
        range_to: f64,
    ) -> Result<Self, ScaleError> {
        validate_domain(domain_from, domain_to)?;
        validate_range(range_from, range_to)?;
        Ok(Self {
            domain_from,
            domain_to,
            range_from,
            range_to,
        })
    }

    pub fn domain(&self) -> (f64, f64) {
        (self.domain_from, self.domain_to)
    }

    pub fn range(&self) -> (f64, f64) {
        (self.range_from, self.range_to)
    }

    /// Replace both domain endpoints atomically.
    pub fn set_domain(&mut self, from: f64, to: f64) -> Result<(), ScaleError> {
        validate_domain(from, to)?;
        self.domain_from = from;
        self.domain_to = to;
        Ok(())
    }

    /// Replace both output-range endpoints atomically. A zero-length range is valid during layout.
    pub fn set_range(&mut self, from: f64, to: f64) -> Result<(), ScaleError> {
        validate_range(from, to)?;
        self.range_from = from;
        self.range_to = to;
        Ok(())
    }

    pub fn coordinate(&self, value: f64) -> Option<f64> {
        let unit = normalized_unit(value, self.domain_from, self.domain_to)?;
        let coordinate = self.range_from + unit * (self.range_to - self.range_from);
        coordinate.is_finite().then_some(normalize_zero(coordinate))
    }

    pub fn coordinate_clamped(&self, value: f64) -> Option<f64> {
        let low = self.domain_from.min(self.domain_to);
        let high = self.domain_from.max(self.domain_to);
        self.coordinate(value.clamp(low, high))
    }

    /// Convert a coordinate back into the domain. A collapsed layout range has no inverse.
    pub fn invert(&self, coordinate: f64) -> Option<f64> {
        if !coordinate.is_finite() || self.range_from == self.range_to {
            return None;
        }
        let unit = normalized_unit(coordinate, self.range_from, self.range_to)?;
        interpolate_unit(self.domain_from, self.domain_to, unit)
    }

    /// Generate deterministic 1/2/5 ticks. `target_count` is a density hint, not a promise.
    pub fn ticks(&self, target_count: usize) -> Vec<f64> {
        if target_count == 0 {
            return Vec::new();
        }

        let low = self.domain_from.min(self.domain_to);
        let high = self.domain_from.max(self.domain_to);
        // Leave room for both ends when the chosen step divides the domain exactly.
        let count = target_count.min(MAX_GENERAL_TICKS - 1);
        let span = high - low;
        let raw_step = if span.is_finite() {
            span / count as f64
        } else {
            let scaled_step = high / count as f64 - low / count as f64;
            if scaled_step.is_finite() {
                scaled_step
            } else {
                f64::MAX
            }
        };
        let step = nice_step(raw_step);
        if !step.is_finite() || step <= 0.0 {
            return Vec::new();
        }

        let quotient = low / step;
        let mut tick = quotient.ceil() * step;
        if !tick.is_finite() {
            return Vec::new();
        }

        let tolerance = step.abs() * 1e-12;
        let mut ticks = Vec::with_capacity(count.saturating_add(1).min(MAX_GENERAL_TICKS));
        while ticks.len() < MAX_GENERAL_TICKS {
            if tick > high && tick - high > tolerance {
                break;
            }
            let value = normalize_zero(tick);
            if ticks.last().is_none_or(|previous| *previous != value) {
                ticks.push(value);
            }
            let next = tick + step;
            if !next.is_finite() || next <= tick {
                break;
            }
            tick = next;
        }

        if self.domain_from > self.domain_to {
            ticks.reverse();
        }
        ticks
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BandScale {
    count: usize,
    range_from: f64,
    range_to: f64,
    padding_inner: f64,
    padding_outer: f64,
    align: f64,
}

impl BandScale {
    pub fn new(
        count: usize,
        range_from: f64,
        range_to: f64,
        padding_inner: f64,
        padding_outer: f64,
        align: f64,
    ) -> Result<Self, ScaleError> {
        validate_category_inputs(
            count,
            range_from,
            range_to,
            padding_inner,
            padding_outer,
            align,
        )?;
        Ok(Self {
            count,
            range_from,
            range_to,
            padding_inner,
            padding_outer,
            align,
        })
    }

    pub fn count(&self) -> usize {
        self.count
    }

    pub fn step(&self) -> f64 {
        if self.count == 0 {
            return 0.0;
        }
        let span = (self.range_to - self.range_from).abs();
        let denominator =
            (self.count as f64 - self.padding_inner + 2.0 * self.padding_outer).max(1.0);
        span / denominator
    }

    pub fn bandwidth(&self) -> f64 {
        self.step() * (1.0 - self.padding_inner)
    }

    pub fn center(&self, index: usize) -> Option<f64> {
        if index >= self.count {
            return None;
        }
        let step = self.step();
        let bandwidth = self.bandwidth();
        let span = (self.range_to - self.range_from).abs();
        let occupied = step * (self.count as f64 - self.padding_inner);
        let offset = (span - occupied) * self.align + bandwidth * 0.5 + index as f64 * step;
        Some(normalize_zero(project_from_start(
            self.range_from,
            self.range_to,
            offset,
        )))
    }

    /// Return ascending coordinate bounds regardless of range direction.
    pub fn bounds(&self, index: usize) -> Option<(f64, f64)> {
        let center = self.center(index)?;
        let half = self.bandwidth() * 0.5;
        Some((normalize_zero(center - half), normalize_zero(center + half)))
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PointScale {
    count: usize,
    range_from: f64,
    range_to: f64,
    padding: f64,
    align: f64,
}

impl PointScale {
    pub fn new(
        count: usize,
        range_from: f64,
        range_to: f64,
        padding: f64,
        align: f64,
    ) -> Result<Self, ScaleError> {
        validate_category_inputs(count, range_from, range_to, 1.0, padding, align)?;
        Ok(Self {
            count,
            range_from,
            range_to,
            padding,
            align,
        })
    }

    pub fn count(&self) -> usize {
        self.count
    }

    pub fn step(&self) -> f64 {
        if self.count == 0 {
            return 0.0;
        }
        let span = (self.range_to - self.range_from).abs();
        let denominator = (self.count.saturating_sub(1) as f64 + 2.0 * self.padding).max(1.0);
        span / denominator
    }

    pub fn coordinate(&self, index: usize) -> Option<f64> {
        if index >= self.count {
            return None;
        }
        let step = self.step();
        let span = (self.range_to - self.range_from).abs();
        let occupied = step * self.count.saturating_sub(1) as f64;
        let offset = (span - occupied) * self.align + index as f64 * step;
        Some(normalize_zero(project_from_start(
            self.range_from,
            self.range_to,
            offset,
        )))
    }
}

fn validate_domain(from: f64, to: f64) -> Result<(), ScaleError> {
    if !from.is_finite() || !to.is_finite() {
        return Err(ScaleError::NonFinite);
    }
    if from == to {
        return Err(ScaleError::DegenerateDomain);
    }
    Ok(())
}

fn validate_range(from: f64, to: f64) -> Result<(), ScaleError> {
    if !from.is_finite() || !to.is_finite() || !(to - from).is_finite() {
        return Err(ScaleError::NonFinite);
    }
    Ok(())
}

fn validate_category_inputs(
    count: usize,
    range_from: f64,
    range_to: f64,
    padding_inner: f64,
    padding_outer: f64,
    align: f64,
) -> Result<(), ScaleError> {
    validate_range(range_from, range_to)?;
    if count as u128 > MAX_EXACT_INTEGER {
        return Err(ScaleError::CategoryCountTooLarge);
    }
    if !padding_inner.is_finite()
        || !padding_outer.is_finite()
        || !(0.0..=1.0).contains(&padding_inner)
        || !(0.0..=1.0).contains(&padding_outer)
    {
        return Err(ScaleError::InvalidPadding);
    }
    if !align.is_finite() || !(0.0..=1.0).contains(&align) {
        return Err(ScaleError::InvalidAlign);
    }
    Ok(())
}

fn nice_step(raw_step: f64) -> f64 {
    if !raw_step.is_finite() || raw_step <= 0.0 {
        return f64::NAN;
    }
    let power = 10.0_f64.powf(raw_step.log10().floor());
    if !power.is_finite() || power <= 0.0 {
        return raw_step;
    }
    let error = raw_step / power;
    // Round upward so the density hint can never expand beyond the hard output bound.
    let factor = if error <= 1.0 {
        1.0
    } else if error <= 2.0 {
        2.0
    } else if error <= 5.0 {
        5.0
    } else {
        10.0
    };
    let step = factor * power;
    if step.is_finite() { step } else { raw_step }
}

fn normalized_unit(value: f64, from: f64, to: f64) -> Option<f64> {
    if !value.is_finite() {
        return None;
    }
    let span = to - from;
    let offset = value - from;
    let unit = if span.is_finite() && offset.is_finite() {
        offset / span
    } else {
        (value * 0.5 - from * 0.5) / (to * 0.5 - from * 0.5)
    };
    unit.is_finite().then_some(unit)
}

fn interpolate_unit(from: f64, to: f64, unit: f64) -> Option<f64> {
    let value = if unit == 0.0 {
        from
    } else if unit == 1.0 {
        to
    } else {
        from * (1.0 - unit) + to * unit
    };
    value.is_finite().then_some(normalize_zero(value))
}

fn project_from_start(range_from: f64, range_to: f64, offset: f64) -> f64 {
    if range_to >= range_from {
        range_from + offset
    } else {
        range_from - offset
    }
}

fn normalize_zero(value: f64) -> f64 {
    if value == 0.0 { 0.0 } else { value }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() < 1e-10,
            "expected {expected}, got {actual}"
        );
    }

    #[test]
    fn linear_maps_inverts_and_supports_reversed_domains() {
        let scale = LinearScale::new(10.0, 20.0, 0.0, 100.0).unwrap();
        close(scale.coordinate(12.5).unwrap(), 25.0);
        close(scale.invert(75.0).unwrap(), 17.5);

        let reversed = LinearScale::new(20.0, 10.0, -50.0, 50.0).unwrap();
        close(reversed.coordinate(15.0).unwrap(), 0.0);
        close(reversed.coordinate(10.0).unwrap(), 50.0);
        close(reversed.invert(25.0).unwrap(), 12.5);
    }

    #[test]
    fn linear_maps_the_complete_finite_domain_without_overflow() {
        let scale = LinearScale::new(-f64::MAX, f64::MAX, -1.0, 1.0).unwrap();
        assert_eq!(scale.coordinate(-f64::MAX), Some(-1.0));
        assert_eq!(scale.coordinate(0.0), Some(0.0));
        assert_eq!(scale.coordinate(f64::MAX), Some(1.0));
        assert_eq!(scale.invert(-1.0), Some(-f64::MAX));
        assert_eq!(scale.invert(0.0), Some(0.0));
        assert_eq!(scale.invert(1.0), Some(f64::MAX));
        let ticks = scale.ticks(10);
        assert!(!ticks.is_empty());
        assert!(ticks.len() <= MAX_GENERAL_TICKS);
        assert!(ticks.iter().all(|value| value.is_finite()));
    }

    #[test]
    fn linear_clamps_and_collapsed_ranges_have_no_inverse() {
        let mut scale = LinearScale::new(-1.0, 1.0, 10.0, 20.0).unwrap();
        assert_eq!(scale.coordinate_clamped(-4.0), Some(10.0));
        assert_eq!(scale.coordinate_clamped(4.0), Some(20.0));
        scale.set_range(7.0, 7.0).unwrap();
        assert_eq!(scale.coordinate(0.0), Some(7.0));
        assert_eq!(scale.invert(7.0), None);
        assert_eq!(scale.coordinate(f64::NAN), None);
    }

    #[test]
    fn linear_updates_reject_invalid_input_atomically() {
        let mut scale = LinearScale::new(0.0, 10.0, 0.0, 100.0).unwrap();
        assert_eq!(
            scale.set_domain(4.0, 4.0),
            Err(ScaleError::DegenerateDomain)
        );
        assert_eq!(scale.domain(), (0.0, 10.0));
        assert_eq!(
            scale.set_range(0.0, f64::INFINITY),
            Err(ScaleError::NonFinite)
        );
        assert_eq!(scale.range(), (0.0, 100.0));
    }

    #[test]
    fn linear_ticks_are_nice_bounded_and_follow_domain_direction() {
        let scale = LinearScale::new(-0.7, 9.3, 0.0, 100.0).unwrap();
        assert_eq!(scale.ticks(5), vec![0.0, 2.0, 4.0, 6.0, 8.0]);
        assert!(scale.ticks(usize::MAX).len() <= MAX_GENERAL_TICKS);

        let reversed = LinearScale::new(9.3, -0.7, 0.0, 100.0).unwrap();
        assert_eq!(reversed.ticks(5), vec![8.0, 6.0, 4.0, 2.0, 0.0]);
        assert!(reversed.ticks(0).is_empty());
    }

    #[test]
    fn log_scale_maps_inverts_and_emits_bounded_decade_ticks() {
        let scale = LogScale::new(1.0, 1_000.0, 0.0, 300.0).unwrap();
        close(scale.coordinate(10.0).unwrap(), 100.0);
        close(scale.coordinate(100.0).unwrap(), 200.0);
        close(scale.invert(150.0).unwrap(), 10.0_f64.powf(1.5));
        let ticks = scale.ticks(8);
        assert!(ticks.contains(&1.0));
        assert!(ticks.iter().all(|value| *value > 0.0));
        assert!(ticks.len() <= MAX_GENERAL_TICKS);
        assert_eq!(
            LogScale::new(3.0, 4.0, 0.0, 1.0).unwrap().ticks(6),
            vec![3.0, 4.0]
        );
        assert_eq!(
            LogScale::new(0.0, 10.0, 0.0, 1.0),
            Err(ScaleError::NonPositiveDomain)
        );
    }

    #[test]
    fn symlog_scale_round_trips_through_zero_and_validates_constant() {
        let scale = SymLogScale::new(-100.0, 100.0, 0.0, 200.0, 1.0).unwrap();
        close(scale.coordinate(0.0).unwrap(), 100.0);
        for value in [-100.0, -4.0, 0.0, 3.5, 100.0] {
            close(
                scale.invert(scale.coordinate(value).unwrap()).unwrap(),
                value,
            );
        }
        assert!(scale.ticks(7).len() <= MAX_GENERAL_TICKS);
        assert_eq!(
            SymLogScale::new(-1.0, 1.0, 0.0, 1.0, 0.0),
            Err(ScaleError::InvalidConstant)
        );
    }

    #[test]
    fn band_scale_respects_padding_alignment_and_reverse_direction() {
        let scale = BandScale::new(3, 0.0, 120.0, 0.2, 0.1, 0.5).unwrap();
        close(scale.step(), 40.0);
        close(scale.bandwidth(), 32.0);
        assert_eq!(scale.bounds(0), Some((4.0, 36.0)));
        assert_eq!(scale.center(1), Some(60.0));
        assert_eq!(scale.bounds(2), Some((84.0, 116.0)));
        assert_eq!(scale.center(3), None);

        let reversed = BandScale::new(3, 120.0, 0.0, 0.2, 0.1, 0.5).unwrap();
        assert_eq!(reversed.bounds(0), Some((84.0, 116.0)));
        assert_eq!(reversed.center(2), Some(20.0));
    }

    #[test]
    fn point_scale_handles_endpoints_singletons_and_reverse_direction() {
        let scale = PointScale::new(3, 0.0, 100.0, 0.0, 0.5).unwrap();
        assert_eq!(scale.step(), 50.0);
        assert_eq!(scale.coordinate(0), Some(0.0));
        assert_eq!(scale.coordinate(1), Some(50.0));
        assert_eq!(scale.coordinate(2), Some(100.0));

        let singleton = PointScale::new(1, 0.0, 100.0, 0.0, 0.25).unwrap();
        assert_eq!(singleton.coordinate(0), Some(25.0));

        let reversed = PointScale::new(3, 100.0, 0.0, 0.0, 0.5).unwrap();
        assert_eq!(reversed.coordinate(0), Some(100.0));
        assert_eq!(reversed.coordinate(2), Some(0.0));
        assert_eq!(reversed.coordinate(3), None);
    }

    #[test]
    fn category_scales_validate_without_allocating_category_state() {
        assert_eq!(
            BandScale::new(2, 0.0, 10.0, 1.1, 0.0, 0.5),
            Err(ScaleError::InvalidPadding)
        );
        assert_eq!(
            PointScale::new(2, 0.0, 10.0, 0.0, -0.1),
            Err(ScaleError::InvalidAlign)
        );
        assert_eq!(
            BandScale::new(0, 0.0, 10.0, 0.0, 0.0, 0.5)
                .unwrap()
                .center(0),
            None
        );
        assert_eq!(
            PointScale::new(0, 0.0, 10.0, 0.0, 0.5)
                .unwrap()
                .coordinate(0),
            None
        );
    }
}
