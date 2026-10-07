//! Price-scale mode conversions: percent, indexed-to-100, logarithmic.
//! Port of `src/model/price-scale-conversions.ts`.

use crate::model::price_range::PriceRange;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LogFormula {
    pub logical_offset: i32,
    pub coord_offset: f64,
}

pub const DEF_LOG_FORMULA: LogFormula = LogFormula {
    logical_offset: 4,
    coord_offset: 0.0001,
};

pub fn from_percent(value: f64, base_value: f64) -> f64 {
    let v = if base_value < 0.0 { -value } else { value };
    (v / 100.0) * base_value + base_value
}

pub fn to_percent(value: f64, base_value: f64) -> f64 {
    let result = 100.0 * (value - base_value) / base_value;
    if base_value < 0.0 { -result } else { result }
}

pub fn to_percent_range(range: &PriceRange, base_value: f64) -> PriceRange {
    PriceRange::new(
        to_percent(range.min_value(), base_value),
        to_percent(range.max_value(), base_value),
    )
}

pub fn from_indexed_to_100(value: f64, base_value: f64) -> f64 {
    let mut v = value - 100.0;
    if base_value < 0.0 {
        v = -v;
    }
    (v / 100.0) * base_value + base_value
}

pub fn to_indexed_to_100(value: f64, base_value: f64) -> f64 {
    let result = 100.0 * (value - base_value) / base_value + 100.0;
    if base_value < 0.0 { -result } else { result }
}

pub fn to_indexed_to_100_range(range: &PriceRange, base_value: f64) -> PriceRange {
    PriceRange::new(
        to_indexed_to_100(range.min_value(), base_value),
        to_indexed_to_100(range.max_value(), base_value),
    )
}

pub fn to_log(price: f64, formula: &LogFormula) -> f64 {
    let m = price.abs();
    if m < 1e-15 {
        return 0.0;
    }

    let res = (m + formula.coord_offset).log10() + formula.logical_offset as f64;
    if price < 0.0 { -res } else { res }
}

pub fn from_log(logical: f64, formula: &LogFormula) -> f64 {
    let m = logical.abs();
    if m < 1e-15 {
        return 0.0;
    }

    let res = 10f64.powf(m - formula.logical_offset as f64) - formula.coord_offset;
    if logical < 0.0 { -res } else { res }
}

pub fn convert_price_range_to_log(range: &PriceRange, formula: &LogFormula) -> PriceRange {
    PriceRange::new(
        to_log(range.min_value(), formula),
        to_log(range.max_value(), formula),
    )
}

pub fn can_convert_price_range_from_log(range: &PriceRange, formula: &LogFormula) -> bool {
    let min = from_log(range.min_value(), formula);
    let max = from_log(range.max_value(), formula);
    min.is_finite() && max.is_finite()
}

pub fn convert_price_range_from_log(range: &PriceRange, formula: &LogFormula) -> PriceRange {
    PriceRange::new(
        from_log(range.min_value(), formula),
        from_log(range.max_value(), formula),
    )
}

/// Picks a log formula adapted to the raw (non-log) price range: small ranges (< 1) get more
/// logical resolution.
pub fn log_formula_for_price_range(range: Option<&PriceRange>) -> LogFormula {
    let Some(range) = range else {
        return DEF_LOG_FORMULA;
    };

    let diff = (range.max_value() - range.min_value()).abs();
    if !(1e-15..1.0).contains(&diff) {
        return DEF_LOG_FORMULA;
    }

    let digits = diff.log10().abs().ceil() as i32;
    let logical_offset = DEF_LOG_FORMULA.logical_offset + digits;
    let coord_offset = 1.0 / 10f64.powi(logical_offset);

    LogFormula {
        logical_offset,
        coord_offset,
    }
}

pub fn log_formulas_are_same(f1: &LogFormula, f2: &LogFormula) -> bool {
    f1.logical_offset == f2.logical_offset && f1.coord_offset == f2.coord_offset
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_roundtrip() {
        assert_eq!(to_percent(110.0, 100.0), 10.0);
        assert_eq!(from_percent(10.0, 100.0), 110.0);
        // negative base flips sign
        assert_eq!(to_percent(-110.0, -100.0), -10.0);
        let v = from_percent(to_percent(-90.0, -100.0), -100.0);
        assert!((v - -90.0).abs() < 1e-12);
    }

    #[test]
    fn indexed_to_100_roundtrip() {
        assert_eq!(to_indexed_to_100(110.0, 100.0), 110.0);
        assert_eq!(to_indexed_to_100(100.0, 100.0), 100.0);
        let v = from_indexed_to_100(to_indexed_to_100(123.45, 100.0), 100.0);
        assert!((v - 123.45).abs() < 1e-12);
    }

    #[test]
    fn log_roundtrip() {
        let f = DEF_LOG_FORMULA;
        for &p in &[0.00001, 0.5, 1.0, 123.456, 98765.0, -42.0] {
            let v = from_log(to_log(p, &f), &f);
            assert!((v - p).abs() < 1e-9 * p.abs().max(1.0), "p={p} got {v}");
        }
        assert_eq!(to_log(0.0, &f), 0.0);
        assert_eq!(from_log(0.0, &f), 0.0);
    }

    #[test]
    fn log_formula_adapts_to_small_ranges() {
        // diff = 0.5 -> digits = ceil(|log10(0.5)|) = ceil(0.301) = 1
        let r = PriceRange::new(1.0, 1.5);
        let f = log_formula_for_price_range(Some(&r));
        assert_eq!(f.logical_offset, 5);
        assert!((f.coord_offset - 1e-5).abs() < 1e-18);

        // diff >= 1 -> default
        let r2 = PriceRange::new(0.0, 10.0);
        assert_eq!(log_formula_for_price_range(Some(&r2)), DEF_LOG_FORMULA);
        assert_eq!(log_formula_for_price_range(None), DEF_LOG_FORMULA);
    }
}
