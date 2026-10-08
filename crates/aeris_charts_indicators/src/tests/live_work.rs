//! A live append or tip replacement must read a number of source rows set by the indicator's
//! parameters, never by the history length.
use super::*;

/// Largest trailing lookback among the test parameters (Ichimoku's 52-row leading span).
const MAX_LOOKBACK_ROWS: usize = 52;

/// Rows an update may re-emit: the lookback for window kinds, or ZigZag's unconfirmed leg, which
/// the fixture's swings keep shorter than this.
const MAX_EMITTED_ROWS: usize = 64;

struct Columns {
    times: Vec<i64>,
    close: Vec<f64>,
    high: Vec<f64>,
    low: Vec<f64>,
    volume: Vec<f64>,
}

impl Columns {
    fn new(rows: usize) -> Self {
        let close = (0..rows)
            .map(|row| 100.0 + (row as f64 * 0.37).sin() * 5.0 + (row % 11) as f64 * 0.125)
            .collect::<Vec<_>>();
        Self {
            times: (0..rows as i64).map(|row| row * 3_600).collect(),
            high: close
                .iter()
                .enumerate()
                .map(|(row, close)| close + 0.25 * (1 + row % 4) as f64)
                .collect(),
            low: close
                .iter()
                .enumerate()
                .map(|(row, close)| close - 0.25 * (1 + row % 3) as f64)
                .collect(),
            volume: (0..rows).map(|row| (1 + row % 17) as f64 * 10.0).collect(),
            close,
        }
    }

    fn input(&self, rows: usize) -> IndicatorInput<'_> {
        IndicatorInput {
            times: &self.times[..rows],
            open: &self.close[..rows],
            high: &self.high[..rows],
            low: &self.low[..rows],
            close: &self.close[..rows],
            volume: &self.volume[..rows],
            amount: &[],
        }
    }
}

/// Rows re-emitted by the last update, over every output column.
fn emitted_rows(state: &IncrementalState, rows: usize) -> usize {
    (0..state.output_count())
        .map(|column| rows - state.output_from(column))
        .max()
        .unwrap_or(0)
}

/// Source rows read by an append of the last row and by a replacement of that tip.
fn live_work(kind: TestKind, mut state: IncrementalState, rows: usize) -> (usize, usize) {
    let mut columns = Columns::new(rows);
    state.rebuild_from(columns.input(rows - 1), 0);
    state.rebuild_from(columns.input(rows), rows - 1);
    let append = state.last_work_rows();
    let append_emitted = emitted_rows(&state, rows);
    let tip = rows - 1;
    columns.close[tip] += 1.5;
    columns.high[tip] += 2.0;
    columns.low[tip] -= 0.5;
    columns.volume[tip] += 3.0;
    state.rebuild_from(columns.input(rows), tip);
    let replace = state.last_work_rows();
    assert!(
        state.output_from(0) <= tip,
        "{kind:?} produced no tip output at {rows} rows"
    );
    let emitted = append_emitted.max(emitted_rows(&state, rows));
    assert!(
        emitted <= MAX_EMITTED_ROWS,
        "{kind:?} re-emitted {emitted} rows at {rows} rows"
    );
    (append, replace)
}

#[test]
fn live_append_and_tip_replace_work_does_not_grow_with_history() {
    for (kind, state) in all_test_states() {
        let short = live_work(kind, state.clone(), 2_048);
        let long = live_work(kind, state, 65_536);
        assert_eq!(
            short, long,
            "{kind:?} live (append, replace) source rows grow with history"
        );
        assert!(
            long.0 <= MAX_LOOKBACK_ROWS && long.1 <= MAX_LOOKBACK_ROWS,
            "{kind:?} live (append, replace) read {long:?} source rows"
        );
    }
}

/// A whitespace run that ends just before the tip, inside every kind's last `period` valid rows,
/// must not be walked on each update, and the tip must still equal a fresh rebuild.
///
/// Fork rule (owner decision Q-G): a window kind's last `period` valid rows lie before the run, so
/// this also proves the runtime finds them through its recorded valid rows (`ValidRows`) rather
/// than by walking the run; the counts then equal the whitespace-free counts of
/// `live_append_and_tip_replace_work_does_not_grow_with_history`. ZigZag's count adds the rows it
/// re-emits back to its unconfirmed leg's open row, which follows the price path before the run
/// (the two runs leave different bars there), so for ZigZag the fork asserts that bound
/// (`MAX_EMITTED_ROWS`) instead of equal counts.
#[test]
fn live_work_does_not_grow_with_a_whitespace_run_inside_the_window() {
    let rows = 70_000;
    let work = |kind: TestKind, state: &IncrementalState, run: usize| {
        let mut columns = Columns::new(rows);
        for row in rows - 3 - run..rows - 3 {
            columns.high[row] = f64::NAN;
            columns.low[row] = f64::NAN;
            columns.close[row] = f64::NAN;
        }
        let mut live = state.clone();
        live.rebuild_from(columns.input(rows - 1), 0);
        live.rebuild_from(columns.input(rows), rows - 1);
        let append = live.last_work_rows();
        columns.close[rows - 1] += 1.5;
        columns.high[rows - 1] += 2.0;
        columns.low[rows - 1] -= 0.5;
        live.rebuild_from(columns.input(rows), rows - 1);
        let replace = live.last_work_rows();
        let mut fresh = state.clone();
        fresh.rebuild_from(columns.input(rows), 0);
        for column in 0..fresh.output_count() {
            let fresh = fresh.output(column).last().copied();
            let live = live.output(column).last().copied();
            let same = match (fresh, live) {
                (Some(fresh), Some(live)) if fresh.is_nan() => live.is_nan(),
                (Some(fresh), Some(live)) => {
                    (fresh - live).abs() <= 1e-12_f64.max(1e-9 * fresh.abs())
                }
                (fresh, live) => fresh.is_none() && live.is_none(),
            };
            assert!(
                same,
                "{kind:?} column {column} tip after a {run}-row whitespace run: {live:?} != {fresh:?}"
            );
        }
        (append, replace)
    };
    let mut grown = Vec::new();
    for (kind, state) in all_test_states() {
        let short = work(kind, &state, 100);
        let long = work(kind, &state, 60_000);
        if kind == TestKind::ZigZag {
            if [short.0, short.1, long.0, long.1]
                .into_iter()
                .any(|rows| rows > MAX_EMITTED_ROWS)
            {
                grown.push(format!("{kind:?} {short:?} -> {long:?}"));
            }
        } else if short != long {
            grown.push(format!("{kind:?} {short:?} -> {long:?}"));
        }
    }
    assert!(
        grown.is_empty(),
        "live work grows with the whitespace run: {grown:#?}"
    );
}

/// Fork: a live update counts the source rows its windows read the same way whether or not a
/// whitespace row sits inside the window. The compacted path reads the recorded valid rows plus
/// the changed tail; the gap-free path reads the physical window. Both are the window's valid
/// rows, so a gap entering or leaving the window never moves the count (upstream's windows
/// stay blank there instead). ZigZag is excluded: its count follows the leg it re-emits.
#[test]
fn live_work_counts_the_same_rows_with_or_without_a_gap_inside_the_window() {
    let rows = 4_096;
    let work = |state: &IncrementalState, gap: bool| {
        let mut columns = Columns::new(rows);
        if gap {
            for column in [&mut columns.high, &mut columns.low, &mut columns.close] {
                column[rows - 3] = f64::NAN;
            }
        }
        live_work_with(state.clone(), &mut columns, rows)
    };
    let mut differ = Vec::new();
    for (kind, state) in all_test_states() {
        if kind == TestKind::ZigZag {
            continue;
        }
        let (plain, gapped) = (work(&state, false), work(&state, true));
        if plain != gapped {
            differ.push(format!(
                "{kind:?} {plain:?} without, {gapped:?} with the gap"
            ));
        }
    }
    assert!(
        differ.is_empty(),
        "live work counts move with a gap: {differ:#?}"
    );
}

/// Source rows read by an append of the last row of `columns` and by a replacement of that tip.
fn live_work_with(
    mut state: IncrementalState,
    columns: &mut Columns,
    rows: usize,
) -> (usize, usize) {
    state.rebuild_from(columns.input(rows - 1), 0);
    state.rebuild_from(columns.input(rows), rows - 1);
    let append = state.last_work_rows();
    columns.close[rows - 1] += 1.5;
    columns.high[rows - 1] += 2.0;
    columns.low[rows - 1] -= 0.5;
    state.rebuild_from(columns.input(rows), rows - 1);
    (append, state.last_work_rows())
}

/// Fork: the valid-row record is sized once by the rebuild that installs the history, so the
/// first live append (or the first fill of a pre-installed session slot) and a tip replacement
/// never grow it; a growth there is retained capacity on every binding (perf_gate Target M
/// slots checks the same on the engine path).
#[test]
fn live_updates_never_grow_the_valid_row_record() {
    let rows = 2_048;
    let mut grown = Vec::new();
    for (kind, mut state) in all_test_states() {
        let mut columns = Columns::new(rows);
        state.rebuild_from(columns.input(rows - 1), 0);
        let installed = state.valid_rows.bytes();
        state.rebuild_from(columns.input(rows), rows - 1);
        columns.close[rows - 1] += 1.5;
        state.rebuild_from(columns.input(rows), rows - 1);
        if state.valid_rows.bytes() != installed {
            grown.push(format!(
                "{kind:?} {installed} -> {} bytes",
                state.valid_rows.bytes()
            ));
        }
    }
    assert!(grown.is_empty(), "live updates grew the record: {grown:#?}");
}

/// A rebuild from the end of the source recomputes no row, so it reports no source rows read
/// (the engine clamps a volume or amount tick at or past the data end to the end).
#[test]
fn a_rebuild_from_the_end_reads_no_rows() {
    let rows = 2_048;
    let columns = Columns::new(rows);
    let mut read = Vec::new();
    for (kind, mut state) in all_test_states() {
        state.rebuild_from(columns.input(rows), 0);
        state.rebuild_from(columns.input(rows), rows);
        if state.last_work_rows() != 0 {
            read.push(format!("{kind:?} {}", state.last_work_rows()));
        }
    }
    assert!(
        read.is_empty(),
        "rebuilds from the end read rows: {read:#?}"
    );
}
