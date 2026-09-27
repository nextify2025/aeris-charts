//! Financial price formatter based on `src/formatters/price-formatter.ts`, with grouped integer
//! digits as the Aeris presentation default.
//!
//! Note: reference uses U+2212 (minus sign) instead of '-' because it has the same advance width as
//! '+', keeping axis labels stable when values flip sign.

pub const MINUS_SIGN: char = '\u{2212}';

/// Pads `value` with leading zeros to `length` digits. Port of `numberToStringWithLeadingZero`.
fn number_to_string_with_leading_zero(value: u64, length: usize) -> String {
    assert!(length <= 16, "invalid length");
    if length == 0 {
        return value.to_string();
    }
    let s = value.to_string();
    if s.len() >= length {
        s[s.len() - length..].to_string()
    } else {
        format!("{}{}", "0".repeat(length - s.len()), s)
    }
}

/// Groups an integer price with the conventional financial thousands separator.
fn format_integer(value: u64) -> String {
    let digits = value.to_string();
    let separators = digits.len().saturating_sub(1) / 3;
    let mut grouped = String::with_capacity(digits.len() + separators);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    grouped
}

/// Decimal digits needed to print every multiple of `min_move` (reference
/// `precisionByMinMove`, series-options.ts): `0.0001 -> 4`, `0.05 -> 2`, `1 -> 0`. The reference
/// compares against an absolute `1e-8`, which returns 0 digits for moves below `1e-8`; here the
/// scaled move must reach a whole unit first, so `1e-9 -> 9` (capped at the 15-digit limit).
pub fn precision_by_min_move(min_move: f64) -> u32 {
    if !min_move.is_finite() || min_move <= 0.0 || min_move >= 1.0 {
        return 0;
    }
    let mut value = min_move;
    for digits in 0..15 {
        let whole = value.round();
        if whole >= 1.0 && (whole - value).abs() < 1e-8 * whole {
            return digits;
        }
        value *= 10.0;
    }
    15
}

#[derive(Clone, Debug)]
pub struct PriceFormatter {
    price_scale: i64,
    min_move: f64,
    fractional_length: usize,
}

impl Default for PriceFormatter {
    fn default() -> Self {
        Self::new(100, 1.0)
    }
}

impl PriceFormatter {
    /// `price_scale` = 10^precision (e.g. 100 for 2 decimals), `min_move` = minimal price step
    /// in scaled units (usually 1).
    pub fn new(price_scale: i64, min_move: f64) -> Self {
        let min_move = if min_move == 0.0 { 1.0 } else { min_move };
        let price_scale = if price_scale < 0 { 100 } else { price_scale };

        // fractional length = number of decimal digits of price_scale
        let mut fractional_length = 0usize;
        if price_scale > 0 && min_move > 0.0 {
            let mut base = price_scale as f64;
            while base > 1.0 {
                base /= 10.0;
                fractional_length += 1;
            }
        }

        Self {
            price_scale,
            min_move,
            fractional_length,
        }
    }

    /// Constructs from a priceFormat option: precision + minMove (e.g. precision 2, minMove 0.01).
    pub fn from_precision(precision: u32, min_move: f64) -> Self {
        let price_scale = 10i64.pow(precision);
        // reference computes priceScale = round(1/minMove-ish) via series options; this helper covers
        // the common case where min_move = 10^-precision.
        let scaled_min_move = (min_move * price_scale as f64).round();
        Self::new(price_scale, scaled_min_move.max(1.0))
    }

    pub fn format(&self, price: f64) -> String {
        let sign = if price < 0.0 {
            MINUS_SIGN.to_string()
        } else {
            String::new()
        };
        format!("{}{}", sign, self.format_as_decimal(price.abs()))
    }

    fn format_as_decimal(&self, price: f64) -> String {
        let base = self.price_scale as f64 / self.min_move;

        let mut int_part = price.floor();
        let mut frac_string = String::new();
        let frac_length = self.fractional_length;

        if base > 1.0 {
            let mut frac_part = (price * base).round() - int_part * base;
            // fixed-point cleanup, port of toFixed(fractionalLength) roundtrip
            let fixup = 10f64.powi(frac_length as i32);
            frac_part = (frac_part * fixup).round() / fixup;

            if frac_part >= base {
                frac_part -= base;
                int_part += 1.0;
            }

            let scaled = ((frac_part * fixup).round() / fixup * self.min_move).round() as u64;
            frac_string = format!(
                ".{}",
                number_to_string_with_leading_zero(scaled, frac_length)
            );
        } else {
            // round int part to min move
            int_part = (int_part * base).round() / base;
            if frac_length > 0 {
                frac_string = format!(".{}", number_to_string_with_leading_zero(0, frac_length));
            }
        }

        format!("{}{}", format_integer(int_part as u64), frac_string)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_two_decimals() {
        let f = PriceFormatter::new(100, 1.0);
        assert_eq!(f.format(1.5), "1.50");
        assert_eq!(f.format(0.0), "0.00");
        assert_eq!(f.format(123.456), "123.46");
        assert_eq!(f.format(123.454), "123.45");
        assert_eq!(f.format(62_000.0), "62,000.00");
        assert_eq!(f.format(1_234_567.89), "1,234,567.89");
    }

    #[test]
    fn negative_uses_unicode_minus() {
        let f = PriceFormatter::new(100, 1.0);
        assert_eq!(f.format(-1.5), "\u{2212}1.50");
        assert_eq!(f.format(-62_000.0), "\u{2212}62,000.00");
    }

    #[test]
    fn integer_scale() {
        let f = PriceFormatter::new(1, 1.0);
        assert_eq!(f.format(5.2), "5");
        assert_eq!(f.format(5.7), "5"); // floor of int part; matches reference
    }

    #[test]
    fn three_decimals() {
        let f = PriceFormatter::new(1000, 1.0);
        assert_eq!(f.format(0.1234), "0.123");
        assert_eq!(f.format(0.0005), "0.001"); // wait: 0.0005*1000=0.5 -> round -> 1 (ties away)
    }

    #[test]
    fn carry_into_integer_part() {
        let f = PriceFormatter::new(100, 1.0);
        assert_eq!(f.format(1.999), "2.00");
    }

    #[test]
    fn from_precision_helper() {
        let f = PriceFormatter::from_precision(2, 0.01);
        assert_eq!(f.format(10.5), "10.50");
    }

    #[test]
    fn precision_follows_min_move_like_the_reference() {
        for (min_move, precision) in [
            (0.0001, 4),
            (0.01, 2),
            (0.02, 2),
            (0.05, 2),
            (0.005, 3),
            (0.25, 2),
            (0.03125, 5),
            (0.5, 1),
            (1.0, 0),
            (5.0, 0),
            (1e-9, 9),
        ] {
            assert_eq!(
                precision_by_min_move(min_move),
                precision,
                "min_move {min_move}"
            );
        }
    }
}
