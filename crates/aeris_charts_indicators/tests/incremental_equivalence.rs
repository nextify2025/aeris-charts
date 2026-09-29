//! Every incremental runtime must agree with a clean full rebuild after any sequence of the
//! mutations a chart applies (appends, last-row replacement, replacement plus append in one
//! rebuild, historical corrections, truncations, and rebuilds with nothing changed), with
//! whitespace rows appearing and disappearing, and
//! must equal the same formula evaluated with the whitespace rows removed.

use aeris_charts_indicators::{
    pivot_points_by_trading_day, stochastic_rsi, zigzag, DeviationEstimator, IncrementalState,
    IndicatorInput, IndicatorSeed, KdjSeed, PivotKind, VwapReset,
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

#[derive(Clone, Default)]
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

    /// Insert a bar before `row`, one second after its predecessor (a late historical bar).
    fn insert(&mut self, row: usize, rng: &mut Rng) {
        let time = self.times[row - 1] + 1;
        self.times.insert(row, time);
        for column in self.columns_mut() {
            column.insert(row, f64::NAN);
        }
        self.write(row, rng, false);
    }

    /// Drop every row from `len` on, like a host `pop`.
    fn truncate(&mut self, len: usize) {
        self.times.truncate(len);
        for column in self.columns_mut() {
            column.truncate(len);
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

/// The public dense formula of a path-dependent runtime evaluated over the non-whitespace rows and
/// scattered back to source rows, or `None` for other runtimes. The runtimes carry their own
/// incremental state, so this pins them to the documented formulas rather than to themselves.
fn dense_formula(label: &str, columns: &Columns) -> Option<Vec<Vec<f64>>> {
    let (compact, kept) = columns.without_whitespace();
    let values: Vec<Vec<Option<f64>>> = match label {
        "stochastic_rsi" => vec![stochastic_rsi(&compact.close, 5, 5)],
        "zigzag" => vec![zigzag(&compact.high, &compact.low, 2.0)],
        "pivot_points" => {
            let points = pivot_points_by_trading_day(
                &compact.times,
                &compact.open,
                &compact.high,
                &compact.low,
                &compact.close,
                PivotKind::Standard,
                &|seconds| seconds,
            );
            vec![
                points.iter().map(|point| point.pivot).collect(),
                points.iter().map(|point| point.resistance_1).collect(),
                points.iter().map(|point| point.support_1).collect(),
                points.iter().map(|point| point.resistance_2).collect(),
                points.iter().map(|point| point.support_2).collect(),
            ]
        }
        _ => return None,
    };
    Some(
        values
            .into_iter()
            .map(|values| {
                let mut scattered = vec![f64::NAN; columns.close.len()];
                for (value, &row) in values.into_iter().zip(&kept) {
                    scattered[row] = value.unwrap_or(f64::NAN);
                }
                scattered
            })
            .collect(),
    )
}

fn run_mutations(seed: u64, with_amount: bool, target_rows: usize) {
    run_mutations_for(&|_| true, seed, with_amount, target_rows, 1);
}

/// [`run_mutations`] over the runtimes `include` selects, comparing with a clean full rebuild every
/// `check_every` steps (and after the last one).
fn run_mutations_for(
    include: &dyn Fn(&str) -> bool,
    seed: u64,
    with_amount: bool,
    target_rows: usize,
    check_every: usize,
) {
    for index in 0..runtimes().len() {
        let fresh = || runtimes().swap_remove(index).1;
        let label = runtimes()[index].0;
        if !include(label) {
            continue;
        }
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
            let from = match rng.below(13) {
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
                // Truncation (a host pop), sometimes with a new last row in the same rebuild. A
                // later replacement of the new last row must still see its full window.
                10 => {
                    let keep = len - 1 - rng.below(3.min(len - 1));
                    columns.truncate(keep);
                    if rng.below(2) == 0 {
                        let whitespace = rng.below(4) == 0;
                        columns.write(keep, &mut rng, whitespace);
                    }
                    keep
                }
                // A late historical bar shifts every later row.
                11 if len > 1 => {
                    let row = 1 + rng.below(len - 1);
                    if columns.times[row] - columns.times[row - 1] > 1 {
                        columns.insert(row, &mut rng);
                        row
                    } else {
                        len
                    }
                }
                // A rebuild that reports no changed row must leave every tail state reusable.
                _ => len,
            };
            state.rebuild_from(columns.input(with_amount), from);
            apply_suffix(&mut retained, &state);
            if step % check_every != 0 && columns.close.len() < target_rows {
                continue;
            }
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
            if let Some(dense) = dense_formula(label, &columns) {
                for (output, (actual, dense)) in retained.iter().zip(&dense).enumerate() {
                    for (row, (&actual, &dense)) in actual.iter().zip(dense).enumerate() {
                        assert!(
                            same(actual, dense),
                            "{label} seed {seed} step {step} output {output} row {row}: \
                             incremental {actual} != dense formula {dense}"
                        );
                    }
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

#[test]
fn truncation_just_past_a_checkpoint_keeps_tail_windows_complete() {
    // A truncation resumes from the sparse checkpoint at row 1023 when the new length is just past
    // it. Runtimes with a retained tail window (Stochastic %D, Stochastic RSI) must rebuild that
    // window with every sample the next replacement of the new last row needs, not just the
    // samples before the truncation point.
    let mut rng = Rng(0x7c0a);
    let mut base = Columns::default();
    for row in 0..1_060 {
        base.write(row, &mut rng, false);
    }
    for index in 0..runtimes().len() {
        let fresh = || runtimes().swap_remove(index).1;
        let label = runtimes()[index].0;
        for len in 1_024..1_050 {
            let mut columns = base.clone();
            let mut state = fresh();
            state.rebuild_from(columns.input(false), 0);
            let mut retained = expanded(&state, columns.close.len());
            columns.truncate(len);
            state.rebuild_from(columns.input(false), len);
            apply_suffix(&mut retained, &state);
            columns.write(len - 1, &mut rng, false);
            state.rebuild_from(columns.input(false), len - 1);
            apply_suffix(&mut retained, &state);
            columns.write(len, &mut rng, false);
            state.rebuild_from(columns.input(false), len);
            apply_suffix(&mut retained, &state);

            let mut reference = fresh();
            reference.rebuild_from(columns.input(false), 0);
            let expected = expanded(&reference, len + 1);
            for (output, (actual, expected)) in retained.iter().zip(&expected).enumerate() {
                for row in len - 30..=len {
                    assert!(
                        same(actual[row], expected[row]),
                        "{label} truncated to {len} output {output} row {row}: incremental {} \
                         != full {}",
                        actual[row],
                        expected[row]
                    );
                }
            }
        }
    }
}

#[test]
fn path_dependent_runtimes_repair_across_checkpoints_like_a_full_rebuild() {
    // Stochastic RSI, pivots and ZigZag carry recursive state with sparse checkpoints every 1,024
    // rows; long sequences make historical corrections resume from real checkpoints.
    let include = |label: &str| {
        matches!(
            label,
            "stochastic_rsi" | "pivot_points" | "zigzag" | "stochastic"
        )
    };
    for seed in 1..=3_u64 {
        run_mutations_for(&include, seed * 6_151, false, 2_600, 97);
    }
}

/// Rows every built-in runtime may evaluate for one tail mutation: its longest window (21, the
/// EMA ribbon's slowest period) plus whitespace it skips, or for ZigZag the rows since its
/// provisional endpoint (the fixture completes a swing every ~17 rows).
const TAIL_WORK_ROWS: usize = 64;

#[test]
fn tail_mutations_on_100k_rows_do_work_bounded_by_the_window_not_the_history() {
    const ROWS: usize = 100_000;
    let mut rng = Rng(0x7a11);
    let mut base = Columns::default();
    for row in 0..ROWS {
        // Whitespace slots throughout history exercise the compaction and in-state skip paths.
        base.write(row, &mut rng, row % 997 == 0);
    }
    for index in 0..runtimes().len() {
        let fresh = || runtimes().swap_remove(index).1;
        let label = runtimes()[index].0;
        let mut columns = base.clone();
        let mut state = fresh();
        state.rebuild_from(columns.input(false), 0);
        let mut retained = expanded(&state, columns.close.len());
        let mut tick = |columns: &Columns, from: usize, bound: usize, mutation: &str| {
            state.rebuild_from(columns.input(false), from);
            apply_suffix(&mut retained, &state);
            assert!(
                state.last_work_rows() <= bound,
                "{label} {mutation}: {} work rows over {} source rows (bound {bound})",
                state.last_work_rows(),
                columns.close.len()
            );
        };

        let last = columns.close.len() - 1;
        columns.write(last, &mut rng, false);
        tick(&columns, last, TAIL_WORK_ROWS, "current-bar replacement");
        columns.write(last + 1, &mut rng, false);
        tick(&columns, last + 1, TAIL_WORK_ROWS, "append");
        columns.write(last + 2, &mut rng, true);
        tick(&columns, last + 2, TAIL_WORK_ROWS, "whitespace append");
        columns.write(last + 2, &mut rng, false);
        tick(&columns, last + 2, TAIL_WORK_ROWS, "whitespace slot filled");
        for revision in 0..8 {
            columns.write(last + 2, &mut rng, revision % 3 == 2);
            tick(&columns, last + 2, TAIL_WORK_ROWS, "repeated revision");
        }
        // Closing the current bar and opening the next in one rebuild resumes recursive state
        // from before the replaced row, not from the nearest sparse checkpoint (the EMA ribbon
        // reports the summed work of its five recursive states).
        columns.write(last + 2, &mut rng, false);
        columns.write(last + 3, &mut rng, false);
        tick(
            &columns,
            last + 2,
            TAIL_WORK_ROWS,
            "replacement plus append",
        );

        let rows = columns.close.len();
        let mut reference = fresh();
        reference.rebuild_from(columns.input(false), 0);
        let expected = expanded(&reference, rows);
        for (output, (actual, expected)) in retained.iter().zip(&expected).enumerate() {
            for row in rows - 256..rows {
                assert!(
                    same(actual[row], expected[row]),
                    "{label} output {output} row {row}: incremental {} != full {}",
                    actual[row],
                    expected[row]
                );
            }
        }
    }
}

/// Bars whose high and low sit 0.1 around `close`.
fn bar_columns(closes: &[f64]) -> Columns {
    Columns {
        times: (0..closes.len() as i64).map(|row| row * 60).collect(),
        open: closes.to_vec(),
        high: closes.iter().map(|close| close + 0.1).collect(),
        low: closes.iter().map(|close| close - 0.1).collect(),
        close: closes.to_vec(),
        volume: vec![1.0; closes.len()],
        amount: vec![1.0; closes.len()],
    }
}

/// Stream `revisions` of the last bar (NaN = whitespace) through a 5% ZigZag, checking every
/// revision against a full rebuild and returning each revision's work rows.
fn zigzag_revisions(columns: &mut Columns, revisions: &[f64]) -> Vec<usize> {
    let mut state = IncrementalState::zigzag(5.0);
    state.rebuild_from(columns.input(false), 0);
    let mut retained = expanded(&state, columns.close.len());
    let last = columns.close.len() - 1;
    let mut work = Vec::new();
    for &close in revisions {
        columns.open[last] = close;
        columns.high[last] = close + 0.1;
        columns.low[last] = close - 0.1;
        columns.close[last] = close;
        state.rebuild_from(columns.input(false), last);
        apply_suffix(&mut retained, &state);
        work.push(state.last_work_rows());
        let mut reference = IncrementalState::zigzag(5.0);
        reference.rebuild_from(columns.input(false), 0);
        let expected = expanded(&reference, columns.close.len());
        for (row, (&actual, &expected)) in retained[0].iter().zip(&expected[0]).enumerate() {
            assert!(
                same(actual, expected),
                "revision to {close} row {row}: incremental {actual} != full {expected}"
            );
        }
    }
    work
}

#[test]
fn zigzag_long_legs_cost_only_the_rows_whose_turning_points_change() {
    // A 20% rally to a high at row 999, then 99,000 sideways rows within 1% below it: a 5% ZigZag
    // neither extends nor reverses that leg, so its provisional endpoint stays 99,000 rows back.
    let rows = 100_000;
    let closes = (0..rows)
        .map(|row| {
            if row < 1_000 {
                100.0 + row as f64 * 0.02
            } else {
                119.4 + (row as f64 * 0.1).sin() * 0.5
            }
        })
        .collect::<Vec<_>>();
    let mut columns = bar_columns(&closes);
    let inside = 119.5;
    let work = zigzag_revisions(
        &mut columns,
        &[
            inside,
            121.0,
            inside,
            119.0,
            110.0,
            inside,
            f64::NAN,
            inside,
        ],
    );
    // Revisions that leave the endpoint's value alone (inside the range, a confirmed reversal
    // that keeps the high as a turning point, whitespace) touch only the forming bar. Moving the
    // endpoint to the forming bar, and moving it back, rewrite every row back to the old high.
    assert_eq!(work, [1, rows - 999, rows - 999, 1, 1, 1, 1, 1]);

    // A flat market never moves 5% from its first bar, so no direction exists: ticks stay O(1)
    // until one establishes the first direction, which writes the anchor at row 0.
    let flat = (0..rows)
        .map(|row| 100.0 + (row as f64 * 0.1).sin())
        .collect::<Vec<_>>();
    let mut columns = bar_columns(&flat);
    let work = zigzag_revisions(&mut columns, &[100.5, 99.5, 107.0, 100.5, f64::NAN, 100.2]);
    assert_eq!(work, [1, 1, rows, rows, 1, 1]);
}

#[test]
fn zigzag_tail_work_is_bounded_by_the_rows_since_the_last_confirmed_turning_point() {
    // A random walk with occasional jumps streams current-bar revisions (some whitespace) and new
    // bars through ZigZag at several deviations. Each tail rebuild may rewrite at most the rows
    // after the last confirmed turning point of the retained path, and the path must stay equal
    // to a full rebuild.
    for (seed, deviation) in [(0x2192, 0.5), (0x5e1f, 2.0), (0x0b7d, 5.0)] {
        let mut rng = Rng(seed);
        let mut columns = Columns::default();
        let mut close = 100.0_f64;
        let write = |columns: &mut Columns, row: usize, rng: &mut Rng, base: f64| {
            let whitespace = rng.below(10) == 0;
            let jump = if rng.below(20) == 0 { 0.15 } else { 0.02 };
            let next = (base * (1.0 + (rng.below(1_000) as f64 / 1_000.0 - 0.5) * jump)).max(1.0);
            let bar = if whitespace {
                [f64::NAN; 4]
            } else {
                [base, base.max(next) * 1.002, base.min(next) * 0.998, next]
            };
            if row == columns.close.len() {
                columns.times.push(row as i64 * 60);
                columns.volume.push(1.0);
                columns.amount.push(1.0);
                columns.open.push(bar[0]);
                columns.high.push(bar[1]);
                columns.low.push(bar[2]);
                columns.close.push(bar[3]);
            } else {
                columns.open[row] = bar[0];
                columns.high[row] = bar[1];
                columns.low[row] = bar[2];
                columns.close[row] = bar[3];
            }
            if whitespace {
                base
            } else {
                next
            }
        };
        for row in 0..5_000 {
            close = write(&mut columns, row, &mut rng, close);
        }
        let mut state = IncrementalState::zigzag(deviation);
        state.rebuild_from(columns.input(false), 0);
        let mut retained = expanded(&state, columns.close.len());
        let mut previous_close = close;
        for step in 0..1_500 {
            let len = columns.close.len();
            let values = &retained[0];
            let endpoint = (0..len).rev().find(|&row| !values[row].is_nan());
            let confirmed = endpoint
                .and_then(|endpoint| (0..endpoint).rev().find(|&row| !values[row].is_nan()))
                .unwrap_or(0);
            let from = if rng.below(3) == 0 {
                previous_close = close;
                close = write(&mut columns, len, &mut rng, close);
                len
            } else {
                close = write(&mut columns, len - 1, &mut rng, previous_close);
                len - 1
            };
            state.rebuild_from(columns.input(false), from);
            apply_suffix(&mut retained, &state);
            assert!(
                state.last_work_rows() <= columns.close.len() - confirmed,
                "deviation {deviation} step {step}: {} work rows, last confirmed turning point \
                 {confirmed} of {} rows",
                state.last_work_rows(),
                columns.close.len()
            );
        }
        let mut reference = IncrementalState::zigzag(deviation);
        reference.rebuild_from(columns.input(false), 0);
        let expected = expanded(&reference, columns.close.len());
        for (row, (&actual, &expected)) in retained[0].iter().zip(&expected[0]).enumerate() {
            assert!(
                same(actual, expected),
                "deviation {deviation} row {row}: incremental {actual} != full {expected}"
            );
        }
    }
}

/// Full and row-by-row streamed outputs of `fresh` over `columns` with explicit weight columns.
fn weighted_outputs(
    fresh: &dyn Fn() -> IncrementalState,
    columns: &Columns,
    volume: &[f64],
    amount: &[f64],
) -> (Vec<Vec<f64>>, Vec<Vec<f64>>) {
    let rows = columns.close.len();
    let input = |len: usize| IndicatorInput {
        times: &columns.times[..len],
        open: &columns.open[..len],
        high: &columns.high[..len],
        low: &columns.low[..len],
        close: &columns.close[..len],
        volume,
        amount,
    };
    let mut full = fresh();
    full.rebuild_from(input(rows), 0);
    let mut streamed = fresh();
    streamed.rebuild_from(input(24), 0);
    let mut retained = expanded(&streamed, 24);
    for len in 25..=rows {
        streamed.rebuild_from(input(len), len - 1);
        apply_suffix(&mut retained, &streamed);
    }
    (expanded(&full, rows), retained)
}

#[test]
fn short_weight_columns_use_each_formula_missing_weight_fallback() {
    // An engine borrows a volume or turnover column whose timestamps are a prefix of the source
    // timestamps (a candle that streamed its new bar before its volume) instead of re-aligning the
    // whole history. Every row past the column's end must equal the documented timestamp-alignment
    // fallback: unit weight for VWAP/VWMA/VWAP bands, zero for OBV/CMF/MFI/volume, and "no trade"
    // for amount-weighted VWAP.
    let mut rng = Rng(0x5107);
    let mut columns = Columns::default();
    for row in 0..48 {
        columns.write(row, &mut rng, matches!(row, 9 | 33 | 36 | 44));
    }
    let known = 31;
    let padded = |values: &[f64], fallback: f64| {
        let mut values = values[..known].to_vec();
        values.resize(columns.close.len(), fallback);
        values
    };
    let assert_same =
        |label: &str, short: (Vec<Vec<f64>>, Vec<Vec<f64>>), expected: &[Vec<f64>]| {
            for (path, actual) in [("full", &short.0), ("streamed", &short.1)] {
                for (output, expected) in expected.iter().enumerate() {
                    for (row, &expected) in expected.iter().enumerate() {
                        assert!(
                            same(actual[output][row], expected),
                            "{label} {path} output {output} row {row}: short column {} != \
                         fallback-padded {expected}",
                            actual[output][row]
                        );
                    }
                }
            }
        };
    for (label, fallback) in [
        ("vwma", 1.0),
        ("vwap", 1.0),
        ("vwap_bands", 1.0),
        ("obv", 0.0),
        ("cmf", 0.0),
        ("mfi", 0.0),
        ("volume", 0.0),
    ] {
        let index = runtimes()
            .iter()
            .position(|(name, _)| *name == label)
            .expect("weighted runtime");
        let fresh = || runtimes().swap_remove(index).1;
        let volume = padded(&columns.volume, fallback);
        let expected = weighted_outputs(&fresh, &columns, &volume, &[]).0;
        let short = weighted_outputs(&fresh, &columns, &columns.volume[..known], &[]);
        assert_same(label, short, &expected);
    }
    let volume = padded(&columns.volume, f64::NAN);
    let amount = padded(&columns.amount, f64::NAN);
    let expected = weighted_outputs(&IncrementalState::vwap, &columns, &volume, &amount).0;
    let short = weighted_outputs(
        &IncrementalState::vwap,
        &columns,
        &columns.volume[..known],
        &columns.amount[..known],
    );
    assert_same("amount_vwap", short, &expected);
}
