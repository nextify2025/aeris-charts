//! Pure session and calendar level calculations on borrowed OHLC columns.

use crate::{IndicatorInput, SessionSpan, month_key};

/// Calendar UTC days, or ordered disjoint host intervals (start inclusive, end exclusive).
/// Adjacent intervals with the same identity form one continuous session.
#[derive(Clone, Copy, Debug)]
pub enum SessionSource<'a> {
    Utc,
    Host(&'a [SessionSpan]),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreviousPeriod {
    Day,
    Week,
    Month,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionStudy {
    /// Running high and low, reset at each new UTC day or host session.
    SessionLevels,
    /// Completed period levels become available only on the first row of the next period.
    PreviousPeriodLevels(PreviousPeriod),
    /// Running high and low until the elapsed wall-clock duration expires, then fixed.
    OpeningRange { duration_seconds: i64 },
}

/// Columns aligned with the borrowed input's shared OHLC/time prefix. `close` is
/// populated only by previous-period levels. Missing/whitespace rows have no output.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SessionStudyPoint {
    pub high: Option<f64>,
    pub low: Option<f64>,
    pub close: Option<f64>,
}

#[derive(Clone, Copy, Debug)]
struct Aggregate {
    high: f64,
    low: f64,
    close: f64,
}

impl Aggregate {
    fn update(&mut self, high: f64, low: f64, close: f64) {
        self.high = self.high.max(high);
        self.low = self.low.min(low);
        self.close = close;
    }

    fn levels(self, include_close: bool) -> SessionStudyPoint {
        SessionStudyPoint {
            high: Some(self.high),
            low: Some(self.low),
            close: include_close.then_some(self.close),
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Group {
    start: i64,
    end: i64,
    ordinal: usize,
}

#[derive(Clone, Copy, Default, Debug)]
struct HostCursor {
    next: usize,
    current: Option<Group>,
}

impl HostCursor {
    fn at(&mut self, spans: &[SessionSpan], time: i64) -> Option<Group> {
        // A host may append another adjacent piece of the active session. Keep
        // the original ordinal, rather than treating the appended piece as a
        // new session. `from` must cover any earlier rows whose trading day
        // changes when a host extends a span across a calendar boundary.
        if let Some(group) = self.current.as_mut() {
            while let Some(next) = spans.get(self.next) {
                let Some(first) = spans.get(group.ordinal) else {
                    break;
                };
                if next.start != group.end
                    || next.session_id != first.session_id
                    || next.end <= next.start
                {
                    break;
                }
                group.end = next.end;
                self.next += 1;
            }
        }
        while self.current.is_none_or(|group| time >= group.end) {
            let first = *spans.get(self.next)?;
            let ordinal = self.next;
            self.next += 1;
            if first.start >= first.end {
                continue;
            }
            let mut end = first.end;
            while let Some(next) = spans.get(self.next) {
                if next.start != end
                    || next.session_id != first.session_id
                    || next.end <= next.start
                {
                    break;
                }
                end = next.end;
                self.next += 1;
            }
            self.current = Some(Group {
                start: first.start,
                end,
                ordinal,
            });
        }
        self.current.filter(|group| time >= group.start)
    }
}

#[derive(Clone, Copy, Default, Debug)]
struct Runtime {
    host: HostCursor,
    active_key: Option<i64>,
    aggregate: Option<Aggregate>,
    previous: Option<Aggregate>,
}

impl Runtime {
    fn row(
        &mut self,
        input: IndicatorInput<'_>,
        source: SessionSource<'_>,
        kind: SessionStudy,
        row: usize,
    ) -> SessionStudyPoint {
        let time = input.times[row];
        let (session_key, start, day) = match source {
            SessionSource::Host(spans) => {
                let Some(group) = self.host.at(spans, time) else {
                    return SessionStudyPoint::default();
                };
                (
                    group.ordinal as i64,
                    i128::from(group.start),
                    (group.end - 1).div_euclid(86_400),
                )
            }
            SessionSource::Utc => {
                let day = time.div_euclid(86_400);
                (day, i128::from(day) * 86_400, day)
            }
        };
        let key = match kind {
            SessionStudy::PreviousPeriodLevels(period) => period_key(day, period),
            _ => session_key,
        };
        if self.active_key != Some(key) {
            if self.active_key.is_some() {
                // A wholly blank period has no levels to publish. Keep the
                // last observed period until another contributes valid bars.
                if let Some(aggregate) = self.aggregate.take() {
                    self.previous = Some(aggregate);
                }
            }
            self.active_key = Some(key);
            self.aggregate = None;
        }
        let (open, high, low, close) = (
            input.open[row],
            input.high[row],
            input.low[row],
            input.close[row],
        );
        if !open.is_finite()
            || !high.is_finite()
            || !low.is_finite()
            || !close.is_finite()
            || high < low
        {
            return SessionStudyPoint::default();
        }
        let include = match kind {
            SessionStudy::OpeningRange { duration_seconds } => {
                duration_seconds > 0 && i128::from(time) - start < i128::from(duration_seconds)
            }
            _ => true,
        };
        if include {
            if let Some(value) = self.aggregate.as_mut() {
                value.update(high, low, close);
            } else {
                self.aggregate = Some(Aggregate { high, low, close });
            }
        }
        match kind {
            SessionStudy::SessionLevels | SessionStudy::OpeningRange { .. } => self
                .aggregate
                .map(|value| value.levels(false))
                .unwrap_or_default(),
            SessionStudy::PreviousPeriodLevels(_) => self
                .previous
                .map(|value| value.levels(true))
                .unwrap_or_default(),
        }
    }
}

const CHECKPOINT_INTERVAL: usize = 1024;

#[derive(Clone, Copy, Debug)]
struct Checkpoint {
    row: usize,
    runtime: Runtime,
}

/// Retained output and sparse replay state for one fixed study configuration.
///
/// Append and current-tip replacement process one row. Historical correction
/// replays from the nearest preceding 1024-row checkpoint to the new tip.
/// Memory beyond output is O(rows / 1024); neither a source slice nor a host
/// session slice is retained.
#[derive(Clone, Debug)]
pub struct SessionStudyState {
    kind: SessionStudy,
    host_mode: Option<bool>,
    outputs: Vec<SessionStudyPoint>,
    checkpoints: Vec<Checkpoint>,
    tail: Runtime,
    before_tail: Runtime,
    /// Rows the most recent [`Self::update`] evaluated (work telemetry).
    last_work_rows: usize,
}

impl SessionStudyState {
    pub fn new(kind: SessionStudy) -> Self {
        Self {
            kind,
            host_mode: None,
            outputs: Vec::new(),
            checkpoints: Vec::new(),
            tail: Runtime::default(),
            before_tail: Runtime::default(),
            last_work_rows: 0,
        }
    }

    pub fn outputs(&self) -> &[SessionStudyPoint] {
        &self.outputs
    }

    /// Rows the most recent [`Self::update`] evaluated, including a checkpoint replay.
    pub fn last_work_rows(&self) -> usize {
        self.last_work_rows
    }

    pub fn capacity_bytes(&self) -> usize {
        self.outputs.capacity() * std::mem::size_of::<SessionStudyPoint>()
            + self.checkpoints.capacity() * std::mem::size_of::<Checkpoint>()
    }

    /// Replace/replay from the earliest changed source row. Input columns must
    /// retain the unchanged prefix and ascending times. Host spans must remain
    /// ordered/disjoint; when their boundaries or identity change, `from` must
    /// include every affected earlier bar. Changing UTC/host mode rebuilds all.
    /// Host cursor checkpoints are row-local and do not retain the spans.
    pub fn update(&mut self, input: IndicatorInput<'_>, source: SessionSource<'_>, from: usize) {
        let n = input_len(input);
        let host_mode = matches!(source, SessionSource::Host(_));
        let from = if self.host_mode != Some(host_mode) {
            self.host_mode = Some(host_mode);
            0
        } else {
            from.min(n).min(self.outputs.len())
        };
        // A pure truncation still needs the state before its new last row for
        // the next tip replacement, so replay that row as well.
        let from = if n < self.outputs.len() && n > 0 {
            from.min(n - 1)
        } else {
            from
        };
        let (start, mut runtime) = if from == 0 {
            (0, Runtime::default())
        } else if from == self.outputs.len() {
            (from, self.tail)
        } else if from + 1 == self.outputs.len() {
            (from, self.before_tail)
        } else {
            let checkpoint = self.checkpoints.partition_point(|entry| entry.row <= from);
            if checkpoint == 0 {
                (0, Runtime::default())
            } else {
                let entry = self.checkpoints[checkpoint - 1];
                (entry.row, entry.runtime)
            }
        };
        self.outputs.truncate(start);
        // The normal append/tip path already ends at the latest checkpoint:
        // do not binary-search an ever-growing history on every live update.
        if let Some(last) = self.checkpoints.last() {
            if last.row == start + 1 {
                self.checkpoints.pop();
            } else if last.row > start {
                self.checkpoints
                    .truncate(self.checkpoints.partition_point(|entry| entry.row <= start));
            }
        }
        for row in start..n {
            if row + 1 == n {
                self.before_tail = runtime;
            }
            self.outputs
                .push(runtime.row(input, source, self.kind, row));
            if (row + 1) % CHECKPOINT_INTERVAL == 0 {
                self.checkpoints.push(Checkpoint {
                    row: row + 1,
                    runtime,
                });
            }
        }
        self.tail = runtime;
        self.last_work_rows = n.saturating_sub(start);
        if n == 0 {
            self.before_tail = Runtime::default();
        }
    }
}

fn input_len(input: IndicatorInput<'_>) -> usize {
    input
        .times
        .len()
        .min(input.open.len())
        .min(input.high.len())
        .min(input.low.len())
        .min(input.close.len())
}

fn period_key(day: i64, period: PreviousPeriod) -> i64 {
    match period {
        PreviousPeriod::Day => day,
        // Unix day zero was Thursday; adding three makes Monday the week boundary.
        PreviousPeriod::Week => (day + 3).div_euclid(7),
        PreviousPeriod::Month => month_key(day),
    }
}

/// Recompute one study over ascending UTC-second bars without retaining source data.
///
/// Only rows covered by a host interval emit in host mode; intervals must be ordered and
/// disjoint. Adjacent pieces with the same session ID merge even if no bar falls at the
/// join. In UTC mode only input timestamps create rows (never synthetic calendar gaps).
/// Nonfinite or incomplete OHLC rows emit no point and do not affect accumulated levels.
/// A nonpositive opening-range duration produces no levels.
pub fn session_study(
    input: IndicatorInput<'_>,
    source: SessionSource<'_>,
    kind: SessionStudy,
) -> Vec<SessionStudyPoint> {
    let n = input_len(input);
    let mut output = Vec::with_capacity(n);
    let mut runtime = Runtime::default();
    for row in 0..n {
        output.push(runtime.row(input, source, kind, row));
    }
    output
}
