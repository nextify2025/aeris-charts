//! Every incremental runtime must agree with a clean full rebuild after any sequence of the
//! mutations a chart applies (appends, last-row replacement, replacement plus append in one
//! rebuild, historical corrections, and rebuilds with nothing changed), with whitespace rows appearing and disappearing, and
//! must equal the same formula evaluated with the whitespace rows removed.

use aeris_charts_indicators::{
    DeviationEstimator, IncrementalState, IndicatorInput, IndicatorSeed, KdjSeed, PivotKind,
    VwapReset,
};

fn runtimes() -> Vec<(&'static str, IncrementalState)> {
    vec![
        ("sma", IncrementalState::sma(5)),
        ("ema", IncrementalState::ema(5)),
        ("dema", IncrementalState::dema(4)),
        ("tema", IncrementalState::tema(3)),
        ("smma", IncrementalState::smma(5)),
        ("hma", IncrementalState::hma(5)),
        ("vwma", IncrementalState::vwma(5)),
        ("stddev", IncrementalState::standard_deviation(5)),
        ("cci", IncrementalState::cci(5)),
        ("williams_r", IncrementalState::williams_r(5)),
        ("stochastic_rsi", IncrementalState::stochastic_rsi(5, 5)),
        ("momentum", IncrementalState::momentum(5)),
        ("roc", IncrementalState::rate_of_change(5)),
        ("donchian", IncrementalState::donchian(5)),
        (
            "pivot_points",
            IncrementalState::pivot_points(PivotKind::Standard),
        ),
        ("zigzag", IncrementalState::zigzag(2.0)),
        ("keltner", IncrementalState::keltner(5, 2.0)),
        ("adx_dmi", IncrementalState::adx_dmi(5)),
        ("parabolic_sar", IncrementalState::parabolic_sar()),
        ("supertrend", IncrementalState::supertrend(5, 3.0)),
        ("ichimoku", IncrementalState::ichimoku()),
        (
            "ema_ribbon",
            IncrementalState::ema_ribbon([3, 5, 8, 13, 21]),
        ),
        ("bollinger", IncrementalState::bollinger(5, 2.0)),
        ("rsi", IncrementalState::rsi(5)),
        ("macd", IncrementalState::macd(3, 6, 4)),
        ("macd_fast_above_slow", IncrementalState::macd(6, 3, 4)),
        ("stochastic", IncrementalState::stochastic(5, 3)),
        ("atr", IncrementalState::atr(5)),
        ("vwap", IncrementalState::vwap()),
        (
            "vwap_bands",
            IncrementalState::vwap_bands(VwapReset::Session, 1.0, 5.0),
        ),
        ("obv", IncrementalState::obv()),
        ("cmf", IncrementalState::cmf(5)),
        ("mfi", IncrementalState::mfi(5)),
        ("volume", IncrementalState::volume(5)),
        ("wma", IncrementalState::wma(5)),
        ("kdj", IncrementalState::kdj(5, 3, 3)),
        (
            "kdj_first_value",
            IncrementalState::kdj_with_seed(5, 3, 3, KdjSeed::FirstValue),
        ),
        (
            "ema_first_value",
            IncrementalState::ema_with_seed(5, IndicatorSeed::FirstValue),
        ),
        (
            "dema_first_value",
            IncrementalState::dema_with_seed(4, IndicatorSeed::FirstValue),
        ),
        (
            "tema_first_value",
            IncrementalState::tema_with_seed(3, IndicatorSeed::FirstValue),
        ),
        (
            "rsi_first_value",
            IncrementalState::rsi_with_seed(5, IndicatorSeed::FirstValue),
        ),
        (
            "macd_china",
            IncrementalState::macd_with(3, 6, 4, IndicatorSeed::FirstValue, 2.0),
        ),
        (
            "bollinger_sample",
            IncrementalState::bollinger_with(5, 2.0, DeviationEstimator::Sample),
        ),
    ]
}

/// Deterministic xorshift, so every failure names a reproducible seed.
struct Rng(u64);

impl Rng {
    fn below(&mut self, bound: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % bound as u64) as usize
    }
}

#[derive(Default)]
struct Columns {
    times: Vec<i64>,
    open: Vec<f64>,
    high: Vec<f64>,
    low: Vec<f64>,
    close: Vec<f64>,
    volume: Vec<f64>,
    amount: Vec<f64>,
}

impl Columns {
    /// Write row `row` (appending when it is the next row); whitespace rows are all-NaN.
    fn write(&mut self, row: usize, rng: &mut Rng, whitespace: bool) {
        let close = 100.0 + (row as f64 * 0.37).sin() * 5.0 + (rng.below(100) as f64 - 50.0) * 0.05;
        let volume = (rng.below(9) + 1) as f64 * 10.0;
        let value = |sample: f64| if whitespace { f64::NAN } else { sample };
        let values = [
            value(close + 0.3),
            value(close + 1.5),
            value(close - 1.4),
            value(close),
            value(volume),
            value(close * volume),
        ];
        if row == self.close.len() {
            self.times.push(row as i64 * 3_600);
            for (column, value) in self.columns_mut().into_iter().zip(values) {
                column.push(value);
            }
        } else {
            for (column, value) in self.columns_mut().into_iter().zip(values) {
                column[row] = value;
            }
        }
    }

    fn columns_mut(&mut self) -> [&mut Vec<f64>; 6] {
        [
            &mut self.open,
            &mut self.high,
            &mut self.low,
            &mut self.close,
            &mut self.volume,
            &mut self.amount,
        ]
    }

    fn input(&self, with_amount: bool) -> IndicatorInput<'_> {
        IndicatorInput {
            times: &self.times,
            open: &self.open,
            high: &self.high,
            low: &self.low,
            close: &self.close,
            volume: &self.volume,
            amount: if with_amount { &self.amount } else { &[] },
        }
    }

    fn without_whitespace(&self) -> (Self, Vec<usize>) {
        let kept = (0..self.close.len())
            .filter(|&row| !self.close[row].is_nan())
            .collect::<Vec<_>>();
        let pick = |column: &[f64]| kept.iter().map(|&row| column[row]).collect();
        (
            Self {
                times: kept.iter().map(|&row| self.times[row]).collect(),
                open: pick(&self.open),
                high: pick(&self.high),
                low: pick(&self.low),
                close: pick(&self.close),
                volume: pick(&self.volume),
                amount: pick(&self.amount),
            },
            kept,
        )
    }
}

/// One value per source row for every output (NaN before `output_from`).
fn expanded(state: &IncrementalState, rows: usize) -> Vec<Vec<f64>> {
    (0..state.output_count())
        .map(|output| {
            let mut values = vec![f64::NAN; state.output_from(output)];
            values.extend_from_slice(state.output(output));
            assert_eq!(values.len(), rows, "output {output} covers every row");
            values
        })
        .collect()
}

/// Apply a rebuild's suffix to the values a host retained from earlier rebuilds.
fn apply_suffix(retained: &mut [Vec<f64>], state: &IncrementalState) {
    for (output, values) in retained.iter_mut().enumerate() {
        let from = state.output_from(output);
        values.truncate(from.min(values.len()));
        values.resize(from, f64::NAN);
        values.extend_from_slice(state.output(output));
    }
}

fn same(left: f64, right: f64) -> bool {
    (left.is_nan() && right.is_nan()) || (left - right).abs() <= 1e-9 * left.abs().max(1.0)
}

fn run_mutations(seed: u64, with_amount: bool, target_rows: usize) {
    for index in 0..runtimes().len() {
        let fresh = || runtimes().swap_remove(index).1;
        let label = runtimes()[index].0;
        let mut rng = Rng(seed);
        let mut columns = Columns::default();
        for row in 0..8 {
            columns.write(row, &mut rng, false);
        }
        let mut state = fresh();
        state.rebuild_from(columns.input(with_amount), 0);
        let mut retained = expanded(&state, columns.close.len());
        let mut step = 0;
        while columns.close.len() < target_rows {
            step += 1;
            let len = columns.close.len();
            let from = match rng.below(11) {
                0..=3 => {
                    for _ in 0..=rng.below(3) {
                        let whitespace = rng.below(6) == 0;
                        columns.write(columns.close.len(), &mut rng, whitespace);
                    }
                    len
                }
                4 | 5 => {
                    let whitespace = rng.below(4) == 0;
                    columns.write(len - 1, &mut rng, whitespace);
                    len - 1
                }
                6 | 7 => {
                    let whitespace = rng.below(4) == 0;
                    columns.write(len - 1, &mut rng, whitespace);
                    let whitespace = rng.below(5) == 0;
                    columns.write(len, &mut rng, whitespace);
                    len - 1
                }
                8 => {
                    let row = len - 1 - rng.below(12.min(len));
                    let whitespace = rng.below(3) == 0;
                    columns.write(row, &mut rng, whitespace);
                    if rng.below(2) == 0 {
                        columns.write(columns.close.len(), &mut rng, false);
                    }
                    row
                }
                9 => {
                    let row = rng.below(len);
                    let whitespace = rng.below(3) == 0;
                    columns.write(row, &mut rng, whitespace);
                    row
                }
                // A rebuild that reports no changed row must leave every tail state reusable.
                _ => len,
            };
            state.rebuild_from(columns.input(with_amount), from);
            apply_suffix(&mut retained, &state);
            let mut reference = fresh();
            reference.rebuild_from(columns.input(with_amount), 0);
            let expected = expanded(&reference, columns.close.len());
            for (output, (actual, expected)) in retained.iter().zip(&expected).enumerate() {
                for (row, (&actual, &expected)) in actual.iter().zip(expected).enumerate() {
                    assert!(
                        same(actual, expected),
                        "{label} seed {seed} step {step} from {from} output {output} row {row}: \
                         incremental {actual} != full {expected}"
                    );
                }
            }
        }

        let (compact, kept) = columns.without_whitespace();
        let mut reference = fresh();
        reference.rebuild_from(compact.input(with_amount), 0);
        let expected = expanded(&reference, kept.len());
        for (output, (actual, expected)) in retained.iter().zip(&expected).enumerate() {
            let mut compact_row = 0;
            for (row, &actual) in actual.iter().enumerate() {
                if kept.get(compact_row) == Some(&row) {
                    assert!(
                        same(actual, expected[compact_row]),
                        "{label} seed {seed} output {output} row {row}: {actual} != {} \
                         without whitespace",
                        expected[compact_row]
                    );
                    compact_row += 1;
                } else {
                    assert!(
                        actual.is_nan(),
                        "{label} seed {seed} output {output} whitespace row {row} emitted {actual}"
                    );
                }
            }
        }
    }
}

#[test]
fn random_mutation_sequences_match_a_full_rebuild_and_ignore_whitespace() {
    for seed in 1..=24_u64 {
        run_mutations(seed * 7_919, false, 90);
    }
    for seed in 1..=6_u64 {
        run_mutations(seed * 104_729, true, 90);
    }
}

/// Rebuild `state` from `from` over `columns` and compare its rows from `from` with a clean
/// full rebuild.
fn assert_stochastic_resumes_like_a_full_rebuild(
    state: &mut IncrementalState,
    columns: &Columns,
    from: usize,
) {
    state.rebuild_from(columns.input(false), from);
    let mut reference = IncrementalState::stochastic(5, 3);
    reference.rebuild_from(columns.input(false), 0);
    let rows = columns.close.len();
    for output in 0..2 {
        let actual = &expanded(state, rows)[output][from.min(rows)..];
        let expected = &expanded(&reference, rows)[output][from.min(rows)..];
        for (actual, expected) in actual.iter().zip(expected) {
            assert!(
                same(*actual, *expected),
                "output {output} from {from}: {actual} != {expected}"
            );
        }
    }
}

#[test]
fn stochastic_tail_window_is_reused_only_where_the_tail_state_resumes() {
    // Row 1023 is a sparse checkpoint, so these rebuilds can resume exactly at row 1024 from the
    // checkpoint instead of a tail state. The retained %K tail window may then still hold row
    // 1024's old %K and must not be reused for %D.
    let mut rng = Rng(0x5eed);
    let mut columns = Columns::default();
    for row in 0..1025 {
        columns.write(row, &mut rng, false);
    }
    let mut state = IncrementalState::stochastic(5, 3);
    state.rebuild_from(columns.input(false), 0);

    // Replace the last row and append another in one rebuild.
    columns.write(1024, &mut rng, false);
    columns.high[1024] += 40.0;
    columns.write(1025, &mut rng, false);
    assert_stochastic_resumes_like_a_full_rebuild(&mut state, &columns, 1024);

    // A rebuild with nothing changed must keep the tail states, so the following last-row
    // replacement still resumes from them rather than from the checkpoint.
    let mut columns = Columns::default();
    let mut rng = Rng(0xfeed);
    for row in 0..1025 {
        columns.write(row, &mut rng, false);
    }
    let mut state = IncrementalState::stochastic(5, 3);
    state.rebuild_from(columns.input(false), 0);
    assert_stochastic_resumes_like_a_full_rebuild(&mut state, &columns, 1025);
    columns.write(1024, &mut rng, false);
    columns.low[1024] -= 40.0;
    assert_stochastic_resumes_like_a_full_rebuild(&mut state, &columns, 1024);
    assert_eq!(
        state.last_work_rows(),
        1,
        "the replacement resumed from the tail state"
    );
}
