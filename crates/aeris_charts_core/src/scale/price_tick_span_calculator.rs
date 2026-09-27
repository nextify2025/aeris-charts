//! Price tick span calculation. Port of `src/model/price-tick-span-calculator.ts`.

use crate::helpers::mathex::{equal, greater_or_equal, is_base_decimal};

const TICK_SPAN_EPSILON: f64 = 1e-14;

pub struct PriceTickSpanCalculator {
    base: i64,
    integral_dividers: Vec<f64>,
    fractional_dividers: Vec<f64>,
}

impl PriceTickSpanCalculator {
    /// `base` is the price scale base (e.g. 100 for 2 decimals). Panics on unexpected bases,
    /// matching the reference's thrown errors.
    pub fn new(base: i64, integral_dividers: Vec<f64>) -> Self {
        let fractional_dividers = if is_base_decimal(base) {
            vec![2.0, 2.5, 2.0]
        } else {
            let mut dividers = Vec::new();
            let mut base_rest = base;
            // reference throws on a base with prime factors other than 2 and 5 (a user-supplied
            // `min_move` like 0.03 produces one). A JS throw is catchable; a wasm panic aborts
            // the whole chart, so stop decomposing instead and use the dividers found so far.
            while base_rest != 1 && dividers.len() <= 100 {
                if base_rest % 2 == 0 {
                    dividers.push(2.0);
                    base_rest /= 2;
                } else if base_rest % 5 == 0 {
                    dividers.push(2.0);
                    dividers.push(2.5);
                    base_rest /= 5;
                } else {
                    break;
                }
            }
            dividers
        };

        Self {
            base,
            integral_dividers,
            fractional_dividers,
        }
    }

    pub fn tick_span(&self, high: f64, low: f64, max_tick_span: f64) -> f64 {
        let min_movement = if self.base == 0 {
            0.0
        } else {
            1.0 / self.base as f64
        };

        let mut result_tick_span = 10f64.powf(0f64.max((high - low).log10().ceil()));

        let mut index = 0usize;
        let mut c = self.integral_dividers[0];

        loop {
            // the second condition matters for very small values like 1e-10 where
            // greater_or_equal alone fails
            let larger_min_movement =
                greater_or_equal(result_tick_span, min_movement, TICK_SPAN_EPSILON)
                    && result_tick_span > (min_movement + TICK_SPAN_EPSILON);
            let larger_max_tick_span =
                greater_or_equal(result_tick_span, max_tick_span * c, TICK_SPAN_EPSILON);
            let larger_1 = greater_or_equal(result_tick_span, 1.0, TICK_SPAN_EPSILON);

            if !(larger_min_movement && larger_max_tick_span && larger_1) {
                break;
            }

            result_tick_span /= c;
            index += 1;
            c = self.integral_dividers[index % self.integral_dividers.len()];
        }

        if result_tick_span <= min_movement + TICK_SPAN_EPSILON {
            result_tick_span = min_movement;
        }

        result_tick_span = result_tick_span.max(1.0);

        if !self.fractional_dividers.is_empty() && equal(result_tick_span, 1.0, TICK_SPAN_EPSILON) {
            index = 0;
            c = self.fractional_dividers[0];
            while greater_or_equal(result_tick_span, max_tick_span * c, TICK_SPAN_EPSILON)
                && result_tick_span > (min_movement + TICK_SPAN_EPSILON)
            {
                result_tick_span /= c;
                index += 1;
                c = self.fractional_dividers[index % self.fractional_dividers.len()];
            }
        }

        result_tick_span
    }
}

/// The price-scale tick base for a minimum price move: `round(1 / min_move)`, clamped to the
/// exactly representable integer range (reference series `base()`).
pub fn tick_base_for_min_move(min_move: f64) -> i64 {
    const MAX_EXACT_BASE: f64 = 1_000_000_000_000_000.0;
    if !min_move.is_finite() || min_move <= 0.0 {
        return 100;
    }
    min_move.recip().round().clamp(1.0, MAX_EXACT_BASE) as i64
}

/// `10^exponent` for `0 <= exponent <= 22`, built from exact integer products so every host
/// (native libm or wasm) derives bit-identical decades.
fn exact_pow10(exponent: u32) -> f64 {
    (0..exponent.min(22)).fold(1.0, |value, _| value * 10.0)
}

/// `mantissa × 10^exponent` with one correctly rounded operation, so `4 × 10^-1` is exactly the
/// double nearest `0.4` on every host.
fn scaled_decimal(mantissa: f64, exponent: i32) -> f64 {
    if exponent >= 0 {
        mantissa * exact_pow10(exponent.unsigned_abs())
    } else {
        mantissa / exact_pow10(exponent.unsigned_abs())
    }
}

/// Whether `value` is an integer multiple of `step`, tolerant of decimal floating-point error
/// (`0.4 / 0.02` is `20.000000000000004`). Ratios beyond `1e9` are treated as multiples: the
/// step is then far below any printable label resolution.
pub fn is_multiple_of(value: f64, step: f64) -> bool {
    if !value.is_finite() || !step.is_finite() || step <= 0.0 {
        return false;
    }
    let ratio = (value / step).abs();
    ratio > 1e9 || (ratio - ratio.round()).abs() <= 1e-6
}

/// Smallest tick span `>= span` that is an integer multiple of `min_move`.
///
/// A span that already lies on the grid is returned unchanged, which keeps every decimal
/// `min_move` (0.01, 0.1, 1e-4, ...) and binary `min_move` (0.25, 1/32) on the reference output.
/// Otherwise the candidates are the reference's own nice numbers (`{1, 2, 2.5, 4, 5} × 10^k`) that
/// lie on the grid, plus `min_move × {1, 2, 5} × 10^k` so that any positive `min_move` has an
/// answer. Growing the span never violates the tick-mark spacing the reference span satisfied.
pub fn align_span_to_min_move(span: f64, min_move: f64) -> f64 {
    if !span.is_finite() || span <= 0.0 || !min_move.is_finite() || min_move <= 0.0 {
        return span;
    }
    if is_multiple_of(span, min_move) {
        return span;
    }
    let floor = span * (1.0 - 1e-12);
    let mut best = f64::INFINITY;
    // Nice numbers on the grid. Mantissas are tenths so 2.5 stays an integer operand.
    let decade = span.log10().floor() as i32;
    for exponent in decade - 1..=decade + 3 {
        for mantissa in [10.0, 20.0, 25.0, 40.0, 50.0] {
            let candidate = scaled_decimal(mantissa, exponent - 1);
            if candidate >= floor && candidate < best && is_multiple_of(candidate, min_move) {
                best = candidate;
            }
        }
    }
    // Plain min-move multiples, ascending; the first one at or above `span` is this family's min.
    'multiples: for exponent in 0..=20 {
        for mantissa in [1.0, 2.0, 5.0] {
            let candidate = min_move * scaled_decimal(mantissa, exponent);
            if candidate >= floor {
                best = best.min(candidate);
                break 'multiples;
            }
        }
    }
    if best.is_finite() {
        best
    } else {
        span
    }
}

/// The composite span used by the tick mark builder: the minimum over the three divider cycles.
/// Port of `PriceTickMarkBuilder.tickSpan()` (`src/model/price-tick-mark-builder.ts`), with the
/// result aligned to the `min_move` price grid so every generated tick is a tradable price.
pub fn composite_tick_span(
    high: f64,
    low: f64,
    min_move: f64,
    scale_height: f64,
    tick_mark_height: f64,
) -> f64 {
    assert!(high >= low, "high < low");

    let base = tick_base_for_min_move(min_move);
    let max_tick_span = (high - low) * tick_mark_height / scale_height;

    let c1 = PriceTickSpanCalculator::new(base, vec![2.0, 2.5, 2.0]);
    let c2 = PriceTickSpanCalculator::new(base, vec![2.0, 2.0, 2.5]);
    let c3 = PriceTickSpanCalculator::new(base, vec![2.5, 2.0, 2.0]);

    let span = c1
        .tick_span(high, low, max_tick_span)
        .min(c2.tick_span(high, low, max_tick_span))
        .min(c3.tick_span(high, low, max_tick_span));
    align_span_to_min_move(span, min_move)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integral_spans() {
        // hand-traced from the algorithm:
        // base=100, dividers [2, 2.5, 2], high-low=100 -> start=100
        // maxTickSpan=5: 100/2=50 /2.5=20 /2=10 /2=5, 5 >= 12.5? no -> 5
        let calc = PriceTickSpanCalculator::new(100, vec![2.0, 2.5, 2.0]);
        assert_eq!(calc.tick_span(100.0, 0.0, 5.0), 5.0);
    }

    #[test]
    fn fractional_spans_go_below_one() {
        // range 0..1, generous space: span should drop below 1 using fractional dividers
        let calc = PriceTickSpanCalculator::new(100, vec![2.0, 2.5, 2.0]);
        let span = calc.tick_span(1.0, 0.0, 0.1);
        assert!(span < 1.0);
        assert!(span >= 0.01); // never below min movement
    }

    #[test]
    fn never_below_min_movement() {
        let calc = PriceTickSpanCalculator::new(100, vec![2.0, 2.5, 2.0]);
        let span = calc.tick_span(0.02, 0.0, 1e-9);
        assert!(span >= 0.01 - 1e-14);
    }

    #[test]
    fn composite_takes_min_of_cycles() {
        let a = composite_tick_span(100.0, 0.0, 0.01, 500.0, 30.0);
        // each individual cycle produces >= a
        for dividers in [
            vec![2.0, 2.5, 2.0],
            vec![2.0, 2.0, 2.5],
            vec![2.5, 2.0, 2.0],
        ] {
            let c = PriceTickSpanCalculator::new(100, dividers);
            assert!(c.tick_span(100.0, 0.0, 100.0 * 30.0 / 500.0) >= a);
        }
    }

    #[test]
    fn non_decimal_base_fractional_dividers() {
        // base 25 = 5*5 -> dividers [2, 2.5, 2, 2.5]
        let calc = PriceTickSpanCalculator::new(25, vec![2.0, 2.5, 2.0]);
        let span = calc.tick_span(1.0, 0.0, 0.2);
        assert!(span <= 1.0);
        assert!(span >= 1.0 / 25.0 - 1e-14);
    }

    /// The unaligned reference composite (`PriceTickMarkBuilder.tickSpan`).
    fn reference_span(high: f64, low: f64, base: i64, height: f64, mark_height: f64) -> f64 {
        let max_tick_span = (high - low) * mark_height / height;
        [
            vec![2.0, 2.5, 2.0],
            vec![2.0, 2.0, 2.5],
            vec![2.5, 2.0, 2.0],
        ]
        .into_iter()
        .map(|dividers| {
            PriceTickSpanCalculator::new(base, dividers).tick_span(high, low, max_tick_span)
        })
        .fold(f64::INFINITY, f64::min)
    }

    fn sweep() -> impl Iterator<Item = (f64, f64, f64)> {
        let centers = [0.37, 1.0, 9.9, 15.0, 25.0, 97.5, 250.0, 1234.5, 5_000.0];
        let widths = [0.01, 0.05, 0.13, 0.6, 1.43, 3.7, 12.0, 55.0, 240.0, 1_900.0];
        let heights = [90.0, 180.0, 300.0, 400.0, 517.0, 900.0];
        centers.into_iter().flat_map(move |center| {
            widths.into_iter().flat_map(move |width| {
                heights
                    .into_iter()
                    .map(move |height| (center + width / 2.0, center - width / 2.0, height))
            })
        })
    }

    #[test]
    fn decimal_and_binary_min_moves_keep_reference_spans() {
        for min_move in [0.01, 0.1, 0.0001, 1e-8, 0.25, 0.5, 0.03125] {
            let base = tick_base_for_min_move(min_move);
            for (high, low, height) in sweep() {
                let reference = reference_span(high, low, base, height, 30.0);
                assert_eq!(
                    composite_tick_span(high, low, min_move, height, 30.0),
                    reference,
                    "min_move {min_move} range {low}..{high} height {height}"
                );
            }
        }
    }

    #[test]
    fn spans_are_multiples_of_every_min_move() {
        for min_move in [
            0.02, 0.05, 0.005, 0.2, 1.0, 2.0, 5.0, 0.25, 0.03125, 0.03, 0.01,
        ] {
            for (high, low, height) in sweep() {
                let span = composite_tick_span(high, low, min_move, height, 30.0);
                assert!(
                    is_multiple_of(span, min_move) && span >= min_move * (1.0 - 1e-12),
                    "min_move {min_move} range {low}..{high} height {height}: span {span}"
                );
                // Alignment only ever widens the reference span, keeping label spacing.
                let reference =
                    reference_span(high, low, tick_base_for_min_move(min_move), height, 30.0);
                assert!(span >= reference * (1.0 - 1e-12));
            }
        }
    }

    #[test]
    fn alignment_picks_the_nearest_nice_grid_span() {
        // The audit's traced failures: HK$15 with a 0.02 tick got 0.25, 0.05 got 0.125, a
        // 1-unit tick got 2.5, and a 5-unit tick got 2.
        assert_eq!(align_span_to_min_move(0.25, 0.02), 0.4);
        assert_eq!(align_span_to_min_move(0.125, 0.05), 0.2);
        assert_eq!(align_span_to_min_move(0.0625, 0.005), 0.1);
        assert_eq!(align_span_to_min_move(0.5, 0.2), 1.0);
        assert_eq!(align_span_to_min_move(2.5, 1.0), 4.0);
        assert_eq!(align_span_to_min_move(2.0, 5.0), 5.0);
        assert_eq!(align_span_to_min_move(1.0, 0.03), 1.5);
        // Already on the grid: untouched.
        assert_eq!(align_span_to_min_move(0.25, 0.25), 0.25);
        assert_eq!(align_span_to_min_move(2.5, 0.01), 2.5);
    }
}
