//! Engine-owned OHLCV resampling.
//!
//! The host supplies UTC session/period boundaries. The engine deliberately has no calendar or
//! timezone fallback: a row outside those boundaries is omitted, making policy explicit and
//! deterministic across browser and native executors. [`resample_boundaries`] derives those
//! boundaries from exchange-local session windows and the exchange time for the host's trading
//! dates (which dates trade stays host calendar data).
//!
//! A source mutation refreshes only the derived bars whose buckets can contain a changed row:
//! the unchanged prefix is kept and the tail is rebuilt from the first affected bucket, so a
//! live tick costs the rows of one bucket rather than the whole history.

use std::collections::HashSet;

use aeris_charts_core::scale::exchange_time::ExchangeTime;
use aeris_charts_core::scale::session_slots::{
    session_window_bounds, SessionSlotError, SessionWindow,
};
use aeris_charts_core::scale::time_tick_marks::civil_from_timestamp;
use serde::{Deserialize, Serialize};

use crate::{ChartEngine, IndicatorChange, SeriesId, SeriesKind, SeriesOwner};

pub const MAX_RESAMPLE_BOUNDARIES: usize = 20_000;
pub const MAX_RESAMPLED_SERIES: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResampleBoundary {
    /// Inclusive UTC timestamp in seconds.
    pub start_time: i64,
    /// Exclusive UTC timestamp in seconds.
    pub end_time: i64,
    /// Opaque host identity shared by periods belonging to the same session.
    pub session_id: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResampleOptions {
    /// Width of one derived bar. Buckets restart at each supplied boundary.
    pub interval_seconds: u32,
    pub boundaries: Vec<ResampleBoundary>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResampledBar {
    pub timestamp: i64,
    pub session_id: u64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
    pub source_rows: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResampleError {
    UnknownSeries(SeriesId),
    UnsupportedSource(SeriesId),
    UnsupportedTarget(SeriesId),
    InvalidInterval,
    InvalidBoundaries,
    TooManyBoundaries,
    TooManySeries,
    DependencyCycle,
    /// [`resample_boundaries`] could not place the session windows on a trading date.
    InvalidSessions(SessionSlotError),
    /// The chart axis is a non-time bar sequence (a trade-count, volume, or range stream, or
    /// synthetic bars), whose rows are logical keys rather than UTC seconds.
    TimeAxisRequired,
}

/// How [`resample_boundaries`] groups each trading date's session windows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResampleSpan {
    /// One boundary per session window: intraday bars restart at every window open (A-share
    /// 60-minute bars open at 09:30, 10:30, 13:00 and 14:00).
    #[default]
    Window,
    /// One boundary per trading date, from its first window's open to its last window's close:
    /// with an interval of at least that span, one daily bar per date.
    Day,
}

/// Resampling boundaries for the host's trading dates (days since 1970-01-01, strictly
/// ascending): each date's session windows are placed in `time` exactly as
/// [`crate::session_slot_times`] places them, so the boundaries stay on exchange hours across
/// DST and around night sessions. Every boundary of a date carries the session id `YYYYMMDD`.
/// Dates without data produce no bars, so a host may include future dates it will stream.
pub fn resample_boundaries(
    days: &[i64],
    windows: &[SessionWindow],
    time: &ExchangeTime,
    span: ResampleSpan,
) -> Result<Vec<ResampleBoundary>, ResampleError> {
    if days.is_empty() || days.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err(ResampleError::InvalidBoundaries);
    }
    let per_day = match span {
        ResampleSpan::Window => windows.len().max(1),
        ResampleSpan::Day => 1,
    };
    if days.len().saturating_mul(per_day) > MAX_RESAMPLE_BOUNDARIES {
        return Err(ResampleError::TooManyBoundaries);
    }
    let mut boundaries = Vec::with_capacity(days.len() * per_day);
    for &day in days {
        let bounds =
            session_window_bounds(day, windows, time).map_err(ResampleError::InvalidSessions)?;
        let (year, month, date) = civil_from_timestamp(day.saturating_mul(86_400));
        let session_id = (year.max(0) as u64) * 10_000 + u64::from(month) * 100 + u64::from(date);
        let boundary = |start_time, end_time| ResampleBoundary {
            start_time,
            end_time,
            session_id,
        };
        match span {
            ResampleSpan::Window => boundaries.extend(
                bounds
                    .iter()
                    .map(|&(start_time, end_time)| boundary(start_time, end_time)),
            ),
            ResampleSpan::Day => {
                boundaries.push(boundary(bounds[0].0, bounds[bounds.len() - 1].1));
            }
        }
    }
    // Dates whose sessions overlap (a weekend date under a negative session start shares
    // Monday's evening) cannot form disjoint periods.
    let options = ResampleOptions {
        interval_seconds: 1,
        boundaries,
    };
    validate_options(&options)?;
    Ok(options.boundaries)
}

/// Lifetime work counters of one resampled series, like [`crate::TradeStreamStats`]: complete
/// rebuilds (configuration, full source replacement, a refresh that lost a bar), bounded tail
/// refreshes, and the source rows both advanced over.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct ResampleStats {
    pub rebuilds: u64,
    pub tail_refreshes: u64,
    pub rows_scanned: u64,
}

impl std::fmt::Display for ResampleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownSeries(id) => write!(f, "unknown series {id}"),
            Self::UnsupportedSource(id) => write!(f, "series {id} is not an OHLC source"),
            Self::UnsupportedTarget(id) => write!(f, "series {id} is not a compatible target"),
            Self::InvalidInterval => write!(f, "resample interval must be greater than zero"),
            Self::InvalidBoundaries => write!(
                f,
                "resample boundaries must be ordered, disjoint, and non-empty"
            ),
            Self::TooManyBoundaries => write!(f, "resample boundary limit exceeded"),
            Self::TooManySeries => write!(f, "resampled-series limit exceeded"),
            Self::DependencyCycle => {
                write!(f, "resampling dependencies may not be chained or cyclic")
            }
            Self::InvalidSessions(error) => write!(f, "invalid resampling sessions: {error}"),
            Self::TimeAxisRequired => write!(
                f,
                "resampling needs a time axis; the chart axis is a non-time bar sequence"
            ),
        }
    }
}

impl std::error::Error for ResampleError {}

#[derive(Clone, Debug)]
pub(crate) struct ResampleBinding {
    pub(crate) source: SeriesId,
    pub(crate) volume_source: Option<SeriesId>,
    pub(crate) target: SeriesId,
    pub(crate) volume_target: Option<SeriesId>,
    pub(crate) options: ResampleOptions,
    pub(crate) bars: Vec<ResampledBar>,
    pub(crate) work: ResampleStats,
    /// Visible source rows the bars were built from, and one past the last of them with a price
    /// (the rows after it are whitespace, such as pre-installed session slots).
    pub(crate) source_rows: usize,
    pub(crate) data_end: usize,
}

impl ChartEngine {
    pub fn configure_resampled_series(
        &mut self,
        source: SeriesId,
        volume_source: Option<SeriesId>,
        target: SeriesId,
        volume_target: Option<SeriesId>,
        options: ResampleOptions,
    ) -> Result<(), ResampleError> {
        validate_options(&options)?;
        if self.sequence_domain_in_use() {
            return Err(ResampleError::TimeAxisRequired);
        }
        if self.resampled_series.len() >= MAX_RESAMPLED_SERIES
            && !self.resampled_series.contains_key(&target)
        {
            return Err(ResampleError::TooManySeries);
        }
        let kind = |id| self.series_entry(id).map(|entry| entry.kind);
        if !matches!(
            kind(source),
            Some(SeriesKind::Candlestick | SeriesKind::Bar)
        ) {
            return Err(if kind(source).is_none() {
                ResampleError::UnknownSeries(source)
            } else {
                ResampleError::UnsupportedSource(source)
            });
        }
        if !matches!(
            kind(target),
            Some(SeriesKind::Candlestick | SeriesKind::Bar)
        ) {
            return Err(if kind(target).is_none() {
                ResampleError::UnknownSeries(target)
            } else {
                ResampleError::UnsupportedTarget(target)
            });
        }
        for id in [volume_source, volume_target].into_iter().flatten() {
            if !matches!(kind(id), Some(SeriesKind::Histogram)) {
                return Err(if kind(id).is_none() {
                    ResampleError::UnknownSeries(id)
                } else {
                    ResampleError::UnsupportedTarget(id)
                });
            }
        }
        // One writer per series. A target may already be this binding's own output (a
        // reconfigure), never a footprint, a trade-bound candle or study, or synthetic bars: the
        // resampler installs through the unguarded internals, so nothing else would stop two
        // writers from fighting over its rows. Sources are only read, so they may be trade-derived.
        // Another binding's output is a chain, refused as a dependency cycle below.
        for id in [Some(target), volume_target].into_iter().flatten() {
            if !matches!(self.series_owner(id), None | Some(SeriesOwner::Resampled)) {
                return Err(ResampleError::UnsupportedTarget(id));
            }
        }
        // The binding a reconfigure replaces (keyed at `target`) is not a conflict with itself:
        // only the other bindings' outputs and inputs are. Bindings never chain, in either
        // configuration order: a binding reads no other binding's output, and writes nothing
        // another binding reads. A binding never writes its own volume source either, which
        // would refresh itself without end.
        let others = || {
            self.resampled_series
                .values()
                .filter(move |binding| binding.target != target)
        };
        let owned = others()
            .flat_map(|binding| [Some(binding.target), binding.volume_target])
            .flatten()
            .collect::<HashSet<_>>();
        let read = others()
            .flat_map(|binding| [Some(binding.source), binding.volume_source])
            .flatten()
            .collect::<HashSet<_>>();
        let writes = [Some(target), volume_target];
        if source == target
            || volume_source == Some(target)
            || volume_source.is_some() && volume_source == volume_target
            || self
                .resampled_series
                .get(&target)
                .is_some_and(|binding| binding.source != source)
            || owned.contains(&source)
            || volume_source.is_some_and(|id| owned.contains(&id))
            || writes
                .into_iter()
                .flatten()
                .any(|id| owned.contains(&id) || read.contains(&id))
        {
            return Err(ResampleError::DependencyCycle);
        }
        self.resampled_series.insert(
            target,
            ResampleBinding {
                source,
                volume_source,
                target,
                volume_target,
                options,
                bars: Vec::new(),
                work: ResampleStats::default(),
                source_rows: 0,
                data_end: 0,
            },
        );
        self.refresh_resampled_target(target)
    }

    /// Whether a non-time bar sequence (a trade-count, volume, or range stream, or synthetic
    /// bars) owns the chart axis. Resampling buckets UTC seconds, so it cannot share that axis.
    pub(crate) fn sequence_domain_in_use(&self) -> bool {
        !self.synthetic_series.is_empty()
            || self.trade_streams.values().any(|stream| {
                !matches!(
                    stream.options().bars,
                    crate::FootprintBarAggregation::Time { .. }
                )
            })
    }

    pub fn resampled_bars(&self, target: SeriesId) -> Option<&[ResampledBar]> {
        Some(&self.resampled_series.get(&target)?.bars)
    }

    /// Work counters of a resampled target series.
    pub fn resample_stats(&self, target: SeriesId) -> Option<ResampleStats> {
        Some(self.resampled_series.get(&target)?.work)
    }

    /// Rebuild every binding reading `source` (a complete replacement of the source).
    pub(crate) fn refresh_resampled_dependents(&mut self, source: SeriesId) {
        for target in self.resampled_targets_of(source) {
            let _ = self.refresh_resampled_target(target);
        }
    }

    /// Refresh the bindings reading `dependency` after a data change reported like an indicator
    /// change: rows before `change.from` are unchanged, so only the bars whose buckets can hold
    /// a later row are rebuilt. A full replacement (or a change from the first row) rebuilds.
    pub(crate) fn refresh_resampled_after_change(
        &mut self,
        dependency: SeriesId,
        change: IndicatorChange,
    ) {
        if !self.resampled_series.values().any(|binding| {
            binding.source == dependency || binding.volume_source == Some(dependency)
        }) {
            return;
        }
        let unchanged_through = if change.full_replace {
            None
        } else {
            change.from.checked_sub(1).and_then(|row| {
                self.data
                    .series_data(dependency)
                    .and_then(|(times, _)| times.get(row).copied())
            })
        };
        for target in self.resampled_targets_of(dependency) {
            let _ = match unchanged_through {
                Some(time) => self.refresh_resampled_tail(target, time),
                None => self.refresh_resampled_target(target),
            };
        }
    }

    /// The replay cutoff moved from `previous` to `current` (inclusive whole seconds, `None` =
    /// everything visible): rows up to the earlier cutoff kept their visibility, so only the
    /// bars after it are refreshed.
    pub(crate) fn refresh_resampled_after_cutoff(
        &mut self,
        previous: Option<i64>,
        current: Option<i64>,
    ) {
        if previous == current || self.resampled_series.is_empty() {
            return;
        }
        let unchanged_through = match (previous, current) {
            (Some(previous), Some(current)) => previous.min(current),
            (Some(cutoff), None) | (None, Some(cutoff)) => cutoff,
            (None, None) => return,
        };
        let mut targets = self.resampled_series.keys().copied().collect::<Vec<_>>();
        targets.sort_unstable();
        for target in targets {
            let _ = self.refresh_resampled_tail(target, unchanged_through);
        }
    }

    fn resampled_targets_of(&self, dependency: SeriesId) -> Vec<SeriesId> {
        let mut targets = self
            .resampled_series
            .values()
            .filter(|binding| {
                binding.source == dependency || binding.volume_source == Some(dependency)
            })
            .map(|binding| binding.target)
            .collect::<Vec<_>>();
        targets.sort_unstable();
        targets
    }

    /// Keep every derived bar whose bucket closes at or before `unchanged_through + 1` (all of
    /// its source rows are unchanged and no changed row can fall into it) and rebuild the rest
    /// from the source rows after the last kept bucket. The target series takes the rebuilt tail
    /// through the ordinary tail-update path; a tail that lost a bar reinstalls the target.
    fn refresh_resampled_tail(
        &mut self,
        target: SeriesId,
        unchanged_through: i64,
    ) -> Result<(), ResampleError> {
        let (keep, replaced, tail, rows_scanned, lost_bar, volume_target, rows, data_end) = {
            let binding = self
                .resampled_series
                .get(&target)
                .ok_or(ResampleError::UnknownSeries(target))?;
            let (times, columns) = self
                .data
                .series_data(binding.source)
                .ok_or(ResampleError::UnknownSeries(binding.source))?;
            let volume = binding
                .volume_source
                .and_then(|id| self.data.series_data(id));
            let options = &binding.options;
            let keep = binding.bars.partition_point(|bar| {
                bar_close(options, bar) <= unchanged_through.saturating_add(1)
            });
            let resume = keep
                .checked_sub(1)
                .map(|last| bar_close(options, &binding.bars[last]));
            // With the same source rows, rows after the last priced row before and after the
            // change are whitespace both times (pre-installed session slots), so their buckets
            // are unchanged: the scan stops at that data end and keeps the later bars. Rows
            // appended or removed rebuild through the end.
            let rows = times.len();
            let data_end = self.source_data_end(binding.source, rows);
            let through = if rows == binding.source_rows {
                data_end.max(binding.data_end).min(rows)
            } else {
                rows
            };
            let (tail, rows_scanned) =
                resample_rows(times, columns, volume, options, resume, through);
            let replaced = if through < rows {
                tail.last().map_or(0, |last| {
                    binding.bars[keep..].partition_point(|bar| bar.timestamp <= last.timestamp)
                })
            } else {
                binding.bars.len() - keep
            };
            let lost_bar = if through < rows {
                replaced != tail.len()
                    || binding.bars[keep..keep + replaced]
                        .iter()
                        .zip(&tail)
                        .any(|(old, new)| old.timestamp != new.timestamp)
            } else {
                binding.bars[keep..].iter().any(|bar| {
                    tail.binary_search_by_key(&bar.timestamp, |rebuilt| rebuilt.timestamp)
                        .is_err()
                })
            };
            (
                keep,
                replaced,
                tail,
                rows_scanned,
                lost_bar,
                binding.volume_target,
                rows,
                data_end,
            )
        };
        if lost_bar {
            return self.refresh_resampled_target(target);
        }
        {
            let binding = self
                .resampled_series
                .get_mut(&target)
                .expect("binding exists");
            binding.work.tail_refreshes = binding.work.tail_refreshes.saturating_add(1);
            binding.work.rows_scanned = binding.work.rows_scanned.saturating_add(rows_scanned);
            binding.source_rows = rows;
            binding.data_end = data_end;
            if tail.is_empty() {
                return Ok(());
            }
            binding
                .bars
                .splice(keep..keep + replaced, tail.iter().copied());
        }
        let times = tail.iter().map(|bar| bar.timestamp).collect::<Vec<_>>();
        self.update_series_bars_sanitized_inner(
            target,
            times.clone(),
            tail.iter().map(|bar| bar.open).collect(),
            tail.iter().map(|bar| bar.high).collect(),
            tail.iter().map(|bar| bar.low).collect(),
            tail.iter().map(|bar| bar.close).collect(),
        );
        if let Some(volume_target) = volume_target {
            let values = tail.iter().map(|bar| bar.volume).collect::<Vec<_>>();
            self.update_series_bars_sanitized_inner(
                volume_target,
                times,
                values.clone(),
                values.clone(),
                values.clone(),
                values,
            );
        }
        Ok(())
    }

    fn refresh_resampled_target(&mut self, target: SeriesId) -> Result<(), ResampleError> {
        let (bars, rows_scanned, volume_target, rows, data_end) = {
            let binding = self
                .resampled_series
                .get(&target)
                .ok_or(ResampleError::UnknownSeries(target))?;
            let (times, columns) = self
                .data
                .series_data(binding.source)
                .ok_or(ResampleError::UnknownSeries(binding.source))?;
            let volume = binding
                .volume_source
                .and_then(|id| self.data.series_data(id));
            let rows = times.len();
            let (bars, rows_scanned) =
                resample_rows(times, columns, volume, &binding.options, None, rows);
            let data_end = self.source_data_end(binding.source, rows);
            (bars, rows_scanned, binding.volume_target, rows, data_end)
        };
        let out_times = bars.iter().map(|bar| bar.timestamp).collect::<Vec<_>>();
        let open = bars.iter().map(|bar| bar.open).collect::<Vec<_>>();
        let high = bars.iter().map(|bar| bar.high).collect::<Vec<_>>();
        let low = bars.iter().map(|bar| bar.low).collect::<Vec<_>>();
        let close = bars.iter().map(|bar| bar.close).collect::<Vec<_>>();
        self.install_series_columns(target, out_times.clone(), open, high, low, close);
        // ponytail: the target and its volume target each trim the whole layer on their own (a
        // union merge and reindex per trim) and recompute their indicators one by one, the pattern
        // `trim_stream_rows_front` batches for a stream. `DataLayer::trim_fronts` would trim both
        // together; deferred because a resample refresh trims at most two series.
        self.enforce_series_cap(target);
        self.recompute_indicators_for(target);
        if let Some(volume_target) = volume_target {
            let values = bars.iter().map(|bar| bar.volume).collect::<Vec<_>>();
            self.install_series_columns(
                volume_target,
                out_times,
                values.clone(),
                values.clone(),
                values.clone(),
                values,
            );
            self.enforce_series_cap(volume_target);
            self.recompute_indicators_for(volume_target);
        }
        let binding = self
            .resampled_series
            .get_mut(&target)
            .expect("binding exists");
        binding.bars = bars;
        binding.source_rows = rows;
        binding.data_end = data_end;
        binding.work.rebuilds = binding.work.rebuilds.saturating_add(1);
        binding.work.rows_scanned = binding.work.rows_scanned.saturating_add(rows_scanned);
        self.sync_time_points();
        self.invalidate_frame_scene();
        Ok(())
    }

    pub(crate) fn resampling_capacity_bytes(&self) -> usize {
        self.resampled_series
            .values()
            .map(|binding| {
                binding.options.boundaries.capacity() * std::mem::size_of::<ResampleBoundary>()
                    + binding.bars.capacity() * std::mem::size_of::<ResampledBar>()
            })
            .sum()
    }
}

fn validate_options(options: &ResampleOptions) -> Result<(), ResampleError> {
    if options.interval_seconds == 0 {
        return Err(ResampleError::InvalidInterval);
    }
    if options.boundaries.len() > MAX_RESAMPLE_BOUNDARIES {
        return Err(ResampleError::TooManyBoundaries);
    }
    if options.boundaries.is_empty()
        || options
            .boundaries
            .iter()
            .any(|boundary| boundary.start_time >= boundary.end_time)
        || options
            .boundaries
            .windows(2)
            .any(|pair| pair[0].end_time > pair[1].start_time)
    {
        return Err(ResampleError::InvalidBoundaries);
    }
    Ok(())
}

/// Exclusive close of the bucket that produced `bar`.
fn bar_close(options: &ResampleOptions, bar: &ResampledBar) -> i64 {
    let boundary = options
        .boundaries
        .partition_point(|boundary| boundary.start_time <= bar.timestamp)
        .checked_sub(1)
        .map_or(bar.timestamp, |index| options.boundaries[index].end_time);
    boundary.min(
        bar.timestamp
            .saturating_add(i64::from(options.interval_seconds)),
    )
}

/// Aggregate the source rows at or after `resume` (every row when `None`) into bars, returning
/// the bars and the number of source rows advanced over. Whitespace rows reserve their bucket
/// but never contribute prices: a bucket of whitespace rows only is a whitespace bar. Rows at or
/// past `through` start no bar; the bar holding the row before them still takes the rest of its
/// bucket.
fn resample_rows(
    times: &[i64],
    columns: [&[f64]; 4],
    volume: Option<(&[i64], [&[f64]; 4])>,
    options: &ResampleOptions,
    resume: Option<i64>,
    through: usize,
) -> (Vec<ResampledBar>, u64) {
    let through = through.min(times.len());
    let mut output = Vec::new();
    let (mut row, first_boundary) = resume.map_or((0, 0), |time| {
        (
            times.partition_point(|&row_time| row_time < time),
            options
                .boundaries
                .partition_point(|boundary| boundary.end_time <= time),
        )
    });
    let start_row = row;
    let interval = i64::from(options.interval_seconds);
    for boundary in &options.boundaries[first_boundary..] {
        // Boundaries past the last source row (future dates a live host configured ahead) hold
        // no rows: a tail refresh stops here instead of searching every one of them.
        if row >= through {
            break;
        }
        row = row.max(times.partition_point(|&time| time < boundary.start_time));
        while row < through && times[row] < boundary.end_time {
            let bucket = (times[row] - boundary.start_time) / interval;
            let timestamp = boundary
                .start_time
                .saturating_add(bucket.saturating_mul(interval));
            let end = boundary.end_time.min(timestamp.saturating_add(interval));
            let first = row;
            let (mut open, mut high, mut low, mut close) =
                (f64::NAN, f64::NEG_INFINITY, f64::INFINITY, f64::NAN);
            while row < times.len() && times[row] < end {
                if !columns[3][row].is_nan() {
                    if open.is_nan() {
                        open = columns[0][row];
                    }
                    high = high.max(columns[1][row]);
                    low = low.min(columns[2][row]);
                    close = columns[3][row];
                }
                row += 1;
            }
            let traded = !close.is_nan();
            let volume_sum = volume.map_or(0.0, |(volume_times, volume_columns)| {
                let start = volume_times.partition_point(|&time| time < times[first]);
                let finish = volume_times.partition_point(|&time| time <= times[row - 1]);
                volume_columns[3][start..finish]
                    .iter()
                    .copied()
                    .filter(|value| !value.is_nan())
                    .sum()
            });
            output.push(ResampledBar {
                timestamp,
                session_id: boundary.session_id,
                open,
                high: if traded { high } else { f64::NAN },
                low: if traded { low } else { f64::NAN },
                close,
                volume: if traded { volume_sum } else { f64::NAN },
                source_rows: u32::try_from(row - first).unwrap_or(u32::MAX),
            });
        }
    }
    (output, (row - start_row) as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boundaries_restart_buckets_and_exclude_out_of_session_rows() {
        let times = vec![0, 60, 120, 1_000, 1_060, 2_000];
        let columns = [
            vec![10.0, 11.0, 12.0, 20.0, 21.0, 99.0],
            vec![11.0, 13.0, 14.0, 21.0, 23.0, 100.0],
            vec![9.0, 10.0, 11.0, 19.0, 20.0, 98.0],
            vec![10.5, 12.0, 13.0, 20.5, 22.0, 99.5],
        ];
        let volume_times = times.clone();
        let volume = [
            vec![1.0; 6],
            vec![1.0; 6],
            vec![1.0; 6],
            vec![2.0, 3.0, 5.0, 7.0, 11.0, 13.0],
        ];
        let (bars, _) = resample_rows(
            &times,
            columns.each_ref().map(Vec::as_slice),
            Some((&volume_times, volume.each_ref().map(Vec::as_slice))),
            &ResampleOptions {
                interval_seconds: 120,
                boundaries: vec![
                    ResampleBoundary {
                        start_time: 0,
                        end_time: 180,
                        session_id: 7,
                    },
                    ResampleBoundary {
                        start_time: 1_000,
                        end_time: 1_120,
                        session_id: 8,
                    },
                ],
            },
            None,
            times.len(),
        );
        assert_eq!(bars.len(), 3);
        assert_eq!(
            (
                bars[0].open,
                bars[0].high,
                bars[0].low,
                bars[0].close,
                bars[0].volume
            ),
            (10.0, 13.0, 9.0, 12.0, 5.0)
        );
        assert_eq!(
            (bars[2].timestamp, bars[2].session_id, bars[2].volume),
            (1_000, 8, 18.0)
        );
    }

    use crate::{
        parse_iso_date, parse_wall_clock, session_slot_times, SessionSlotConvention,
        UtcOffsetSchedule, UtcOffsetTransition,
    };

    const DAY: i64 = 86_400;
    const HOUR: i64 = 3_600;

    fn window(start: &str, end: &str) -> SessionWindow {
        SessionWindow {
            start_seconds: parse_wall_clock(start, false).unwrap(),
            end_seconds: parse_wall_clock(end, true).unwrap(),
        }
    }

    fn a_share() -> Vec<SessionWindow> {
        vec![window("09:30", "11:30"), window("13:00", "15:00")]
    }

    fn day(text: &str) -> i64 {
        parse_iso_date(text).unwrap()
    }

    fn shanghai() -> UtcOffsetSchedule {
        UtcOffsetSchedule::fixed(8 * 3_600).unwrap()
    }

    fn new_york() -> UtcOffsetSchedule {
        let ts = |date: &str, hour: i64| day(date) * DAY + hour * HOUR;
        UtcOffsetSchedule::new(vec![
            UtcOffsetTransition {
                from_utc_seconds: ts("2023-11-05", 6),
                offset_seconds: -5 * 3_600,
            },
            UtcOffsetTransition {
                from_utc_seconds: ts("2024-03-10", 7),
                offset_seconds: -4 * 3_600,
            },
        ])
        .unwrap()
    }

    /// Deterministic one-minute bars (open-stamped) for every session slot of `dates`.
    fn minutes(
        zone: &UtcOffsetSchedule,
        windows: &[SessionWindow],
        dates: &[&str],
    ) -> (Vec<i64>, [Vec<f64>; 4], Vec<f64>) {
        let time = ExchangeTime::new(zone.clone(), 0).unwrap();
        let times = dates
            .iter()
            .flat_map(|date| {
                session_slot_times(
                    day(date),
                    windows,
                    60,
                    &time,
                    SessionSlotConvention::BarOpen,
                )
                .unwrap()
            })
            .collect::<Vec<_>>();
        let price = |index: usize| 10.0 + ((index * 7) % 23) as f64 * 0.01;
        let open = (0..times.len()).map(price).collect::<Vec<_>>();
        let high = open.iter().map(|value| value + 0.05).collect();
        let low = open.iter().map(|value| value - 0.03).collect();
        let close = open.iter().map(|value| value + 0.01).collect();
        let volume = (0..times.len())
            .map(|index| 100.0 + (index % 7) as f64)
            .collect();
        (times, [open, high, low, close], volume)
    }

    struct Resampled {
        chart: ChartEngine,
        source: SeriesId,
        volume: SeriesId,
        target: SeriesId,
        volume_target: SeriesId,
    }

    fn resampled(
        zone: &UtcOffsetSchedule,
        times: &[i64],
        columns: &[Vec<f64>; 4],
        volume: &[f64],
        interval_seconds: u32,
        boundaries: Vec<ResampleBoundary>,
    ) -> Resampled {
        let mut chart = ChartEngine::new(800.0, 400.0, 1.0);
        chart.set_time_zone(zone.clone());
        let source = chart.add_series(SeriesKind::Candlestick);
        let volume_series = chart.add_series(SeriesKind::Histogram);
        let seconds = times.iter().map(|&time| time as f64).collect::<Vec<_>>();
        chart
            .set_series_data(
                source,
                &seconds,
                &columns[0],
                &columns[1],
                &columns[2],
                &columns[3],
            )
            .unwrap();
        chart
            .set_series_data(volume_series, &seconds, volume, volume, volume, volume)
            .unwrap();
        let target = chart.add_series(SeriesKind::Candlestick);
        let volume_target = chart.add_series(SeriesKind::Histogram);
        chart
            .configure_resampled_series(
                source,
                Some(volume_series),
                target,
                Some(volume_target),
                ResampleOptions {
                    interval_seconds,
                    boundaries,
                },
            )
            .unwrap();
        Resampled {
            chart,
            source,
            volume: volume_series,
            target,
            volume_target,
        }
    }

    type Row = (i64, [f64; 4]);

    fn rows(chart: &ChartEngine, id: SeriesId) -> Vec<Row> {
        let (times, columns) = chart.data_layer().series_data(id).unwrap();
        times
            .iter()
            .enumerate()
            .map(|(row, &time)| {
                (
                    time,
                    [
                        columns[0][row],
                        columns[1][row],
                        columns[2][row],
                        columns[3][row],
                    ],
                )
            })
            .collect()
    }

    /// A fresh chart resampling the live chart's current source and volume rows.
    fn fresh_copy(live: &Resampled) -> Resampled {
        let source = rows(&live.chart, live.source);
        let volume = rows(&live.chart, live.volume);
        let times = source.iter().map(|&(time, _)| time).collect::<Vec<_>>();
        let columns: [Vec<f64>; 4] =
            std::array::from_fn(|column| source.iter().map(|row| row.1[column]).collect());
        assert_eq!(
            volume.iter().map(|&(time, _)| time).collect::<Vec<_>>(),
            times
        );
        let binding = &live.chart.resampled_series[&live.target];
        resampled(
            &shanghai(),
            &times,
            &columns,
            &volume.iter().map(|row| row.1[3]).collect::<Vec<_>>(),
            binding.options.interval_seconds,
            binding.options.boundaries.clone(),
        )
    }

    /// Rows with values as bit patterns, so whitespace (NaN) rows compare equal.
    fn row_bits(chart: &ChartEngine, id: SeriesId) -> Vec<(i64, [u64; 4])> {
        rows(chart, id)
            .into_iter()
            .map(|(time, values)| (time, values.map(f64::to_bits)))
            .collect()
    }

    fn assert_matches_fresh(live: &Resampled) {
        let fresh = fresh_copy(live);
        assert_eq!(
            row_bits(&live.chart, live.target),
            row_bits(&fresh.chart, fresh.target)
        );
        assert_eq!(
            row_bits(&live.chart, live.volume_target),
            row_bits(&fresh.chart, fresh.volume_target)
        );
        assert_eq!(
            format!("{:?}", live.chart.resampled_bars(live.target)),
            format!("{:?}", fresh.chart.resampled_bars(fresh.target))
        );
    }

    fn local_clock(zone: &UtcOffsetSchedule, time: i64) -> String {
        let local = zone.to_local(time).rem_euclid(DAY);
        format!("{:02}:{:02}", local / HOUR, local % HOUR / 60)
    }

    #[test]
    fn a_share_hour_bars_from_minutes_open_at_each_session_window() {
        let zone = shanghai();
        let dates = ["2026-09-24", "2026-09-25"];
        let (times, columns, volume) = minutes(&zone, &a_share(), &dates);
        assert_eq!(times.len(), 480);
        let time = ExchangeTime::new(zone.clone(), 0).unwrap();
        let boundaries =
            resample_boundaries(&dates.map(day), &a_share(), &time, ResampleSpan::Window).unwrap();
        assert_eq!(boundaries.len(), 4);
        let chart = resampled(&zone, &times, &columns, &volume, 3_600, boundaries);
        let bars = chart.chart.resampled_bars(chart.target).unwrap();
        assert_eq!(
            bars.iter()
                .map(|bar| local_clock(&zone, bar.timestamp))
                .collect::<Vec<_>>(),
            ["09:30", "10:30", "13:00", "14:00", "09:30", "10:30", "13:00", "14:00"]
        );
        assert!(bars.iter().all(|bar| bar.source_rows == 60));
        assert_eq!(
            bars.iter().map(|bar| bar.session_id).collect::<Vec<_>>(),
            [
                20_260_924, 20_260_924, 20_260_924, 20_260_924, 20_260_925, 20_260_925, 20_260_925,
                20_260_925
            ]
        );
        for (index, bar) in bars.iter().enumerate() {
            let rows = index * 60..(index + 1) * 60;
            assert_eq!(bar.open, columns[0][rows.start]);
            assert_eq!(bar.close, columns[3][rows.end - 1]);
            assert_eq!(
                bar.high,
                columns[1][rows.clone()]
                    .iter()
                    .copied()
                    .fold(f64::NEG_INFINITY, f64::max)
            );
            assert_eq!(
                bar.low,
                columns[2][rows.clone()]
                    .iter()
                    .copied()
                    .fold(f64::INFINITY, f64::min)
            );
            assert_eq!(bar.volume, volume[rows].iter().sum::<f64>());
        }
        assert_eq!(rows(&chart.chart, chart.target).len(), 8);
        assert_eq!(
            rows(&chart.chart, chart.volume_target)
                .iter()
                .map(|row| row.1[3])
                .collect::<Vec<_>>(),
            bars.iter().map(|bar| bar.volume).collect::<Vec<_>>()
        );
    }

    #[test]
    fn resampled_hour_bars_label_by_close() {
        let zone = shanghai();
        let dates = ["2026-09-24", "2026-09-25"];
        let (times, columns, volume) = minutes(&zone, &a_share(), &dates);
        let time = ExchangeTime::new(zone.clone(), 0).unwrap();
        let boundaries =
            resample_boundaries(&dates.map(day), &a_share(), &time, ResampleSpan::Window).unwrap();
        let mut chart = resampled(&zone, &times, &columns, &volume, 3_600, boundaries);
        chart
            .chart
            .set_bar_time_label(crate::BarTimeLabel::Close {
                interval_seconds: 3_600,
                windows: a_share(),
            })
            .unwrap();
        let bars = chart.chart.resampled_bars(chart.target).unwrap();
        // Bars keep their open identities; the labels print the closes of the same windows.
        assert_eq!(
            bars.iter()
                .map(|bar| local_clock(&zone, bar.timestamp))
                .collect::<Vec<_>>(),
            ["09:30", "10:30", "13:00", "14:00", "09:30", "10:30", "13:00", "14:00"]
        );
        assert_eq!(
            bars.iter()
                .map(|bar| local_clock(&zone, chart.chart.bar_label_time(bar.timestamp)))
                .collect::<Vec<_>>(),
            ["10:30", "11:30", "14:00", "15:00", "10:30", "11:30", "14:00", "15:00"]
        );
        assert_eq!(rows(&chart.chart, chart.target).len(), 8);
    }

    #[test]
    fn a_host_fed_241_bar_feed_shifted_back_drops_its_auction_row_from_resampling() {
        // The shifted auction row sits at 09:29, before the first window opens: resampling skips
        // it like any out-of-session row, so the first hour bar still holds exactly the 60
        // regular minutes and opens at the 09:30 row.
        let zone = shanghai();
        let date = ["2026-09-25"];
        let (mut times, mut columns, mut volume) = minutes(&zone, &a_share(), &date);
        times.insert(0, times[0] - 60);
        for column in &mut columns {
            column.insert(0, 999.0);
        }
        volume.insert(0, 1.0);
        assert_eq!(times.len(), 241);
        let time = ExchangeTime::new(zone.clone(), 0).unwrap();
        let boundaries =
            resample_boundaries(&date.map(day), &a_share(), &time, ResampleSpan::Window).unwrap();
        let chart = resampled(&zone, &times, &columns, &volume, 3_600, boundaries);
        let bars = chart.chart.resampled_bars(chart.target).unwrap();
        assert_eq!(bars.len(), 4);
        assert_eq!(bars[0].source_rows, 60);
        assert_eq!(bars[0].open, columns[0][1]);
        assert_eq!(bars[0].volume, volume[1..61].iter().sum::<f64>());
    }

    #[test]
    fn us_daily_bars_from_extended_hours_minutes_respect_dst() {
        let zone = new_york();
        let windows = [window("04:00", "20:00")];
        let dates = ["2024-03-08", "2024-03-11"];
        let (times, columns, volume) = minutes(&zone, &windows, &dates);
        assert_eq!(times.len(), 2 * 960);
        let time = ExchangeTime::new(zone.clone(), 0).unwrap();
        let boundaries =
            resample_boundaries(&dates.map(day), &windows, &time, ResampleSpan::Day).unwrap();
        let chart = resampled(&zone, &times, &columns, &volume, 86_400, boundaries);
        let bars = chart.chart.resampled_bars(chart.target).unwrap();
        assert_eq!(bars.len(), 2);
        assert!(bars.iter().all(|bar| bar.source_rows == 960));
        // 04:00 Eastern: 09:00 UTC in winter, 08:00 UTC after the change.
        assert_eq!(bars[0].timestamp, day("2024-03-08") * DAY + 9 * HOUR);
        assert_eq!(bars[1].timestamp, day("2024-03-11") * DAY + 8 * HOUR);
        assert_eq!(bars[0].close, columns[3][959]);
        assert_eq!(bars[1].open, columns[0][960]);
        // Friday's winter session runs past UTC midnight (20:00 EST is 01:00 UTC): calendar
        // UTC days would split it into two bars.
        let utc_days = resample_rows(
            &times,
            columns.each_ref().map(Vec::as_slice),
            None,
            &ResampleOptions {
                interval_seconds: 86_400,
                boundaries: ["2024-03-08", "2024-03-09", "2024-03-11"]
                    .map(|date| ResampleBoundary {
                        start_time: day(date) * DAY,
                        end_time: (day(date) + 1) * DAY,
                        session_id: 0,
                    })
                    .to_vec(),
            },
            None,
            times.len(),
        )
        .0;
        assert_eq!(utc_days.len(), 3);
    }

    #[test]
    fn live_source_updates_refresh_only_the_affected_tail() {
        let zone = shanghai();
        let dates = [
            "2026-09-21",
            "2026-09-22",
            "2026-09-23",
            "2026-09-24",
            "2026-09-25",
        ];
        let (times, columns, volume) = minutes(&zone, &a_share(), &dates);
        let time = ExchangeTime::new(zone.clone(), 0).unwrap();
        let boundaries =
            resample_boundaries(&dates.map(day), &a_share(), &time, ResampleSpan::Window).unwrap();
        let loaded = times.len() - 30;
        let truncated: [Vec<f64>; 4] =
            std::array::from_fn(|column| columns[column][..loaded].to_vec());
        let mut live = resampled(
            &zone,
            &times[..loaded],
            &truncated,
            &volume[..loaded],
            300,
            boundaries,
        );
        let stats = live.chart.resample_stats(live.target).unwrap();
        assert_eq!((stats.rebuilds, stats.tail_refreshes), (1, 0));
        assert_eq!(stats.rows_scanned, loaded as u64);

        // Streaming the last half hour: a source or volume minute re-reads at most the rows of
        // its 5-minute bucket, never the history.
        for row in loaded..times.len() {
            let before = live.chart.resample_stats(live.target).unwrap();
            let values = [
                columns[0][row],
                columns[1][row],
                columns[2][row],
                columns[3][row],
            ];
            assert!(live
                .chart
                .update_series_bar(live.source, times[row] as f64, values));
            assert!(live
                .chart
                .update_series_bar(live.volume, times[row] as f64, [volume[row]; 4]));
            let after = live.chart.resample_stats(live.target).unwrap();
            assert_eq!(after.rebuilds, before.rebuilds);
            assert_eq!(after.tail_refreshes, before.tail_refreshes + 2);
            assert!(after.rows_scanned - before.rows_scanned <= 2 * 6, "{row}");
            // Replacing the forming minute is bounded the same way.
            let revised = [values[0], values[1] + 0.5, values[2], values[3] + 0.2];
            assert!(live
                .chart
                .update_series_bar(live.source, times[row] as f64, revised));
            let revised_stats = live.chart.resample_stats(live.target).unwrap();
            assert!(revised_stats.rows_scanned - after.rows_scanned <= 6);
        }
        assert_matches_fresh(&live);
        assert_eq!(rows(&live.chart, live.target).len(), 5 * 48);

        // A historical correction rebuilds only from its own bucket onward.
        let early = times[37];
        let before = live.chart.resample_stats(live.target).unwrap();
        assert_eq!(
            live.chart.update_series_bars_sanitized(
                live.source,
                vec![early],
                vec![9.0],
                vec![12.0],
                vec![8.5],
                vec![11.0],
            ),
            1
        );
        let after = live.chart.resample_stats(live.target).unwrap();
        assert_eq!(after.rebuilds, before.rebuilds);
        assert_eq!(
            after.rows_scanned - before.rows_scanned,
            (times.len() - 35) as u64
        );
        assert_matches_fresh(&live);

        // Removing rows loses bars, which rebuilds once.
        let before = live.chart.resample_stats(live.target).unwrap();
        assert_eq!(live.chart.series_pop(live.source, 7), Some(times.len() - 7));
        assert_eq!(
            live.chart.resample_stats(live.target).unwrap().rebuilds,
            before.rebuilds + 1
        );
        assert_eq!(rows(&live.chart, live.target).len(), 5 * 48 - 1);
        assert_eq!(
            rows(&live.chart, live.target).last().unwrap().1[3],
            columns[3][times.len() - 8] + 0.2
        );
    }

    #[test]
    fn replay_clock_masks_rows_inside_the_forming_bar() {
        let zone = shanghai();
        let dates = ["2026-09-25"];
        let (times, columns, volume) = minutes(&zone, &a_share(), &dates);
        let time = ExchangeTime::new(zone.clone(), 0).unwrap();
        let boundaries =
            resample_boundaries(&dates.map(day), &a_share(), &time, ResampleSpan::Window).unwrap();
        let mut live = resampled(&zone, &times, &columns, &volume, 1_800, boundaries);
        let clock = |minute: usize| Some(times[minute] * 1_000_000);
        let rebuilds = live.chart.resample_stats(live.target).unwrap().rebuilds;
        for (minute, lost_bar) in [
            (17, true),
            (22, false),
            (35, false),
            (10, true),
            (40, false),
        ] {
            let before = live.chart.resample_stats(live.target).unwrap();
            live.chart.set_replay_clock_micros(clock(minute)).unwrap();
            let after = live.chart.resample_stats(live.target).unwrap();
            assert_eq!(
                after.rebuilds - before.rebuilds,
                u64::from(lost_bar),
                "{minute}"
            );
            let visible = rows(&live.chart, live.target);
            assert_eq!(visible.len(), minute / 30 + 1, "{minute}");
            let last = visible.last().unwrap().1;
            // The forming bar closes at the clock's minute, never at a future one.
            assert_eq!(last[3], columns[3][minute], "{minute}");
            assert_eq!(
                last[1],
                columns[1][minute / 30 * 30..=minute]
                    .iter()
                    .copied()
                    .fold(f64::NEG_INFINITY, f64::max)
            );
        }
        live.chart.set_replay_clock_micros(None).unwrap();
        assert_eq!(rows(&live.chart, live.target).len(), 8);
        assert!(live.chart.resample_stats(live.target).unwrap().rebuilds > rebuilds);
        assert_matches_fresh(&live);
    }

    #[test]
    fn whitespace_slots_reserve_buckets_without_prices() {
        let zone = shanghai();
        let dates = ["2026-09-25"];
        let (times, mut columns, mut volume) = minutes(&zone, &a_share(), &dates);
        // Only the first twelve minutes traded; the rest of the session is reserved whitespace.
        for column in &mut columns {
            column[12..].fill(f64::NAN);
        }
        volume[12..].fill(f64::NAN);
        let time = ExchangeTime::new(zone.clone(), 0).unwrap();
        let boundaries =
            resample_boundaries(&dates.map(day), &a_share(), &time, ResampleSpan::Window).unwrap();
        let chart = resampled(&zone, &times, &columns, &volume, 300, boundaries);
        let bars = chart.chart.resampled_bars(chart.target).unwrap();
        assert_eq!(bars.len(), 48);
        // The third bucket traded two minutes: it closes at the last traded minute.
        assert_eq!(bars[2].close, columns[3][11]);
        assert_eq!(bars[2].volume, volume[10] + volume[11]);
        assert!(bars[3..]
            .iter()
            .all(|bar| bar.close.is_nan() && bar.open.is_nan() && bar.volume.is_nan()));
        let target = rows(&chart.chart, chart.target);
        assert_eq!(target.len(), 48);
        assert!(target[3..]
            .iter()
            .all(|row| row.1.iter().all(|value| value.is_nan())));
    }

    #[test]
    fn live_ticks_filling_pre_installed_slots_scan_one_bucket() {
        let zone = shanghai();
        let dates = ["2026-09-24", "2026-09-25"];
        let (times, mut columns, mut volume) = minutes(&zone, &a_share(), &dates);
        // The last day is pre-installed as whitespace slots after its first five minutes.
        let traded = times.len() / 2 + 5;
        for column in &mut columns {
            column[traded..].fill(f64::NAN);
        }
        volume[traded..].fill(f64::NAN);
        let time = ExchangeTime::new(zone.clone(), 0).unwrap();
        let boundaries =
            resample_boundaries(&dates.map(day), &a_share(), &time, ResampleSpan::Window).unwrap();
        let mut live = resampled(&zone, &times, &columns, &volume, 300, boundaries);
        let sma = live.chart.add_sma(live.target, 3).unwrap();
        let price = |row: usize, revision: usize| {
            let value = 10.0 + ((row * 7 + revision) % 23) as f64 * 0.01;
            [value, value + 0.05, value - 0.03, value + 0.01]
        };
        for row in traded..traded + 12 {
            for revision in 0..3 {
                let before = live.chart.resample_stats(live.target).unwrap();
                assert!(live.chart.update_series_bar(
                    live.source,
                    times[row] as f64,
                    price(row, revision)
                ));
                assert!(live.chart.update_series_bar(
                    live.volume,
                    times[row] as f64,
                    [100.0 + revision as f64; 4]
                ));
                let after = live.chart.resample_stats(live.target).unwrap();
                assert_eq!(after.rebuilds, before.rebuilds, "{row}");
                // Source and volume each re-read at most the rows of the forming 5-minute bucket
                // and the one before it, never the reserved slots after them.
                assert!(
                    after.rows_scanned - before.rows_scanned <= 2 * 2 * 5,
                    "row {row}: scanned {}",
                    after.rows_scanned - before.rows_scanned
                );
                // A study on the target recomputes its forming bar, not the reserved buckets.
                let binding = live
                    .chart
                    .indicators
                    .iter()
                    .find(|binding| binding.outputs[0] == sma)
                    .unwrap();
                assert!(
                    binding.last_work_rows() <= 8,
                    "{}",
                    binding.last_work_rows()
                );
            }
            // Only the forming bucket changed: the reserved buckets stay whitespace.
            let bars = live.chart.resampled_bars(live.target).unwrap();
            assert_eq!(bars.len(), 96);
            assert!(bars[(row - times.len() / 2) / 5 + 49..]
                .iter()
                .all(|bar| bar.close.is_nan()));
        }
        assert_matches_fresh(&live);
    }

    #[test]
    fn tail_refreshes_match_a_rebuild_through_gaps_whitespace_and_retention() {
        let zone = shanghai();
        let dates = ["2026-09-24", "2026-09-25"];
        let (times, columns, volume) = minutes(&zone, &a_share(), &dates);
        let time = ExchangeTime::new(zone.clone(), 0).unwrap();
        let boundaries =
            resample_boundaries(&dates.map(day), &a_share(), &time, ResampleSpan::Window).unwrap();
        // Every third five-minute bucket of the history is missing, so later corrections insert
        // derived bars into the middle of the target.
        let keep = (0..times.len())
            .filter(|&row| (row / 5) % 3 != 1 && row < times.len() - 40)
            .collect::<Vec<_>>();
        let pick = |values: &[f64]| keep.iter().map(|&row| values[row]).collect::<Vec<_>>();
        let partial: [Vec<f64>; 4] = std::array::from_fn(|column| pick(&columns[column]));
        let mut live = resampled(
            &zone,
            &keep.iter().map(|&row| times[row]).collect::<Vec<_>>(),
            &partial,
            &pick(&volume),
            300,
            boundaries,
        );
        let nan = f64::NAN;
        let bar = |row: usize| {
            [
                columns[0][row],
                columns[1][row],
                columns[2][row],
                columns[3][row],
            ]
        };
        // Stream the next minutes, alternating traded rows and whitespace reservations.
        for row in times.len() - 40..times.len() - 20 {
            let (values, size) = if row % 4 == 0 {
                ([nan; 4], [nan; 4])
            } else {
                (bar(row), [volume[row]; 4])
            };
            assert!(live
                .chart
                .update_series_bar(live.source, times[row] as f64, values));
            assert!(live
                .chart
                .update_series_bar(live.volume, times[row] as f64, size));
            assert_matches_fresh(&live);
        }
        // Fill a missing historical bucket (a bar inserted mid-target), then blank a traded
        // minute and trade into a whitespace one.
        for row in [6, 7, 8] {
            assert!(live
                .chart
                .update_series_bar(live.source, times[row] as f64, bar(row)));
            assert!(live
                .chart
                .update_series_bar(live.volume, times[row] as f64, [volume[row]; 4]));
            assert_matches_fresh(&live);
        }
        let blank = times.len() - 39;
        assert!(live
            .chart
            .update_series_bar(live.source, times[blank] as f64, [nan; 4]));
        assert_matches_fresh(&live);
        let whitespace = times.len() - 40;
        assert!(live.chart.update_series_bar(
            live.source,
            times[whitespace] as f64,
            bar(whitespace)
        ));
        assert_matches_fresh(&live);
        // A retention cap trims the head of both sources; the target follows a fresh load.
        assert!(live.chart.set_series_max_points(live.source, Some(200)));
        assert!(live.chart.set_series_max_points(live.volume, Some(200)));
        for row in times.len() - 20..times.len() {
            assert!(live
                .chart
                .update_series_bar(live.source, times[row] as f64, bar(row)));
            assert!(live
                .chart
                .update_series_bar(live.volume, times[row] as f64, [volume[row]; 4]));
        }
        assert!(rows(&live.chart, live.source).len() <= 200);
        assert_matches_fresh(&live);
    }

    #[test]
    fn reconfiguring_a_volume_binding_adds_trading_dates() {
        let zone = shanghai();
        let dates = ["2026-09-24", "2026-09-25"];
        let (times, columns, volume) = minutes(&zone, &a_share(), &dates);
        let time = ExchangeTime::new(zone.clone(), 0).unwrap();
        let first =
            resample_boundaries(&[day(dates[0])], &a_share(), &time, ResampleSpan::Window).unwrap();
        let mut live = resampled(&zone, &times, &columns, &volume, 3_600, first);
        assert_eq!(rows(&live.chart, live.target).len(), 4);
        // The documented way to add a new trading date: the same call with extended boundaries.
        let both =
            resample_boundaries(&dates.map(day), &a_share(), &time, ResampleSpan::Window).unwrap();
        let options = ResampleOptions {
            interval_seconds: 3_600,
            boundaries: both,
        };
        live.chart
            .configure_resampled_series(
                live.source,
                Some(live.volume),
                live.target,
                Some(live.volume_target),
                options.clone(),
            )
            .unwrap();
        assert_eq!(rows(&live.chart, live.target).len(), 8);
        assert_eq!(rows(&live.chart, live.volume_target).len(), 8);
        assert_matches_fresh(&live);
        // A reconfigure still keeps its source.
        let other = live.chart.add_series(SeriesKind::Candlestick);
        assert_eq!(
            live.chart.configure_resampled_series(
                other,
                Some(live.volume),
                live.target,
                Some(live.volume_target),
                options,
            ),
            Err(ResampleError::DependencyCycle)
        );
        assert_matches_fresh(&live);
    }

    #[test]
    fn a_binding_never_writes_its_own_volume_source_or_chains() {
        let zone = shanghai();
        let dates = ["2026-09-25"];
        let (times, columns, volume) = minutes(&zone, &a_share(), &dates);
        let time = ExchangeTime::new(zone.clone(), 0).unwrap();
        let boundaries =
            resample_boundaries(&dates.map(day), &a_share(), &time, ResampleSpan::Window).unwrap();
        let options = |interval_seconds| ResampleOptions {
            interval_seconds,
            boundaries: boundaries.clone(),
        };
        let mut live = resampled(&zone, &times, &columns, &volume, 3_600, boundaries.clone());
        let chart = &mut live.chart;
        // The same histogram as volume source and volume target used to recurse without end;
        // it is refused on a first configure and on a reconfigure, leaving the chart usable.
        let fresh = chart.add_series(SeriesKind::Candlestick);
        assert_eq!(
            chart.configure_resampled_series(
                live.source,
                Some(live.volume),
                fresh,
                Some(live.volume),
                options(120),
            ),
            Err(ResampleError::DependencyCycle)
        );
        assert!(chart.resampled_bars(fresh).is_none());
        assert_eq!(
            chart.configure_resampled_series(
                live.source,
                Some(live.volume_target),
                live.target,
                Some(live.volume_target),
                options(3_600),
            ),
            Err(ResampleError::DependencyCycle)
        );
        let fresh_volume = chart.add_series(SeriesKind::Histogram);
        chart
            .configure_resampled_series(
                live.source,
                Some(live.volume),
                fresh,
                Some(fresh_volume),
                options(1_800),
            )
            .unwrap();
        assert!(!chart.resampled_bars(fresh).unwrap().is_empty());

        // Chaining is refused in both configuration orders, for bars and for volume: here the
        // hour binding reads what a new binding would write.
        let minute = chart.add_series(SeriesKind::Candlestick);
        let minute_volume = chart.add_series(SeriesKind::Histogram);
        assert_eq!(
            chart.configure_resampled_series(minute, None, live.source, None, options(60),),
            Err(ResampleError::DependencyCycle)
        );
        let other = chart.add_series(SeriesKind::Candlestick);
        assert_eq!(
            chart.configure_resampled_series(
                minute,
                Some(minute_volume),
                other,
                Some(live.volume),
                options(60),
            ),
            Err(ResampleError::DependencyCycle)
        );
        assert_matches_fresh(&live);
    }

    #[test]
    fn engine_derived_targets_reject_every_host_write() {
        use crate::{SeriesBarPatch, SeriesUpdateOutcome, SeriesUpdateRejection};

        let zone = shanghai();
        let dates = ["2026-09-25"];
        let (times, columns, volume) = minutes(&zone, &a_share(), &dates);
        let time = ExchangeTime::new(zone.clone(), 0).unwrap();
        let boundaries =
            resample_boundaries(&dates.map(day), &a_share(), &time, ResampleSpan::Window).unwrap();
        let mut live = resampled(&zone, &times, &columns, &volume, 3_600, boundaries);
        let open = times[0] as f64;
        for target in [live.target, live.volume_target] {
            let before = rows(&live.chart, target);
            let bars = live.chart.resampled_bars(live.target).unwrap().to_vec();
            let chart = &mut live.chart;
            assert!(chart
                .set_series_data(target, &[open], &[1.0], &[2.0], &[0.5], &[1.5])
                .is_err());
            assert!(!chart.install_series_data(
                target,
                vec![times[0]],
                vec![1.0],
                vec![2.0],
                vec![0.5],
                vec![1.5]
            ));
            assert!(!chart.update_series_bar(target, open, [1.0, 2.0, 0.5, 1.5]));
            assert_eq!(
                chart.update_series_bars_sanitized(
                    target,
                    vec![times[0]],
                    vec![1.0],
                    vec![2.0],
                    vec![0.5],
                    vec![1.5]
                ),
                0
            );
            assert_eq!(chart.series_pop(target, 1), None);
            let patch = SeriesBarPatch {
                open: None,
                high: Some(1_000.0),
                low: None,
                close: Some(999.0),
                colors: [None; 3],
            };
            let unsupported =
                SeriesUpdateOutcome::Rejected(SeriesUpdateRejection::UnsupportedSeries);
            // A merge used to write straight into the derived rows.
            assert_eq!(
                chart.merge_series_bar(target, open, patch, None),
                unsupported
            );
            assert_eq!(
                chart.merge_series_bars(target, &[(open, patch)], Some(7)),
                unsupported
            );
            assert_eq!(
                chart.update_series_bar_sequenced(target, open, [1.0; 4], [None; 3], Some(7)),
                unsupported
            );
            assert_eq!(rows(&live.chart, target), before);
            assert_eq!(
                live.chart.resampled_bars(live.target).unwrap(),
                bars.as_slice()
            );
        }
        assert_matches_fresh(&live);
    }

    /// A trade-bound candle or trade volume study is written by its stream. A resampler cannot take
    /// it as a target (two writers would fight over its rows), but may read it as a source.
    #[test]
    fn trade_derived_series_are_resample_sources_never_targets() {
        use crate::{AggressorSide, FootprintAggregationOptions, FootprintTrade};

        let print = |second: i64, price: f64| FootprintTrade {
            timestamp_micros: second * 1_000_000,
            price,
            volume: 2.0,
            aggressor: AggressorSide::Buy,
            bid: None,
            ask: None,
            sequence: None,
            trade_id: None,
            conditions: 0,
            session_id: Some(1),
        };
        let options = || ResampleOptions {
            interval_seconds: 300,
            boundaries: vec![ResampleBoundary {
                start_time: 0,
                end_time: 30 * 86_400,
                session_id: 1,
            }],
        };
        let mut chart = ChartEngine::new(800.0, 400.0, 1.0);
        let stream = chart
            .add_trade_stream("K:1m", FootprintAggregationOptions::default())
            .unwrap();
        let candles = chart.add_series(SeriesKind::Candlestick);
        chart
            .bind_trade_bar_series_to_stream(candles, stream)
            .unwrap();
        let volume = chart.add_trade_volume_series(stream, 1).unwrap();
        chart
            .set_trade_stream_trades(
                stream,
                (0..40)
                    .map(|index| print(60 + index * 20, 100.0 + (index % 7) as f64 * 0.25))
                    .collect(),
            )
            .unwrap();
        let plain = chart.add_series(SeriesKind::Candlestick);
        let plain_volume = chart.add_series(SeriesKind::Histogram);
        let (candle_rows, volume_rows) = (rows(&chart, candles), rows(&chart, volume));
        assert!(!candle_rows.is_empty());

        assert_eq!(
            chart.configure_resampled_series(plain, None, candles, None, options()),
            Err(ResampleError::UnsupportedTarget(candles))
        );
        assert_eq!(
            chart.configure_resampled_series(
                plain,
                Some(plain_volume),
                candles,
                Some(volume),
                options()
            ),
            Err(ResampleError::UnsupportedTarget(candles))
        );
        let target = chart.add_series(SeriesKind::Candlestick);
        assert_eq!(
            chart.configure_resampled_series(
                plain,
                Some(plain_volume),
                target,
                Some(volume),
                options()
            ),
            Err(ResampleError::UnsupportedTarget(volume))
        );
        assert!(chart.resampled_bars(candles).is_none());
        assert!(chart.resampled_bars(target).is_none());
        assert!(!chart.series_is_source_owned(target));
        assert_eq!(
            (rows(&chart, candles), rows(&chart, volume)),
            (candle_rows, volume_rows)
        );

        // Reading them is the supported direction: the stream keeps feeding the resampled bars.
        let resampled = chart.add_series(SeriesKind::Candlestick);
        let resampled_volume = chart.add_series(SeriesKind::Histogram);
        chart
            .configure_resampled_series(
                candles,
                Some(volume),
                resampled,
                Some(resampled_volume),
                options(),
            )
            .unwrap();
        assert!(!chart.resampled_bars(resampled).unwrap().is_empty());
        chart
            .update_trade_stream_trades(stream, vec![print(900, 101.0)])
            .unwrap();
        assert!(chart.series_is_source_owned(resampled));
        assert!(chart.series_is_source_owned(candles));
    }

    #[test]
    fn resampling_and_non_time_bar_sequences_do_not_share_an_axis() {
        use crate::{
            FootprintAggregationOptions, FootprintBarAggregation, FootprintError,
            SyntheticBarError, SyntheticBarOptions,
        };

        let zone = shanghai();
        let dates = ["2026-09-25"];
        let (times, columns, volume) = minutes(&zone, &a_share(), &dates);
        let time = ExchangeTime::new(zone.clone(), 0).unwrap();
        let boundaries =
            resample_boundaries(&dates.map(day), &a_share(), &time, ResampleSpan::Window).unwrap();
        let mut live = resampled(&zone, &times, &columns, &volume, 3_600, boundaries.clone());
        // Non-time rows are logical keys, not UTC seconds: no sequence axis joins a resampled
        // chart, and no resampling joins a sequence-axis chart.
        let tick_bars = FootprintAggregationOptions {
            bars: FootprintBarAggregation::Trades { trades_per_bar: 10 },
            ..FootprintAggregationOptions::default()
        };
        assert_eq!(
            live.chart.add_trade_stream("SSE:600000:ticks", tick_bars),
            Err(FootprintError::SequenceDomainInUse)
        );
        let renko = live.chart.add_series(SeriesKind::Candlestick);
        assert_eq!(
            live.chart.configure_synthetic_bar_series(
                renko,
                SyntheticBarOptions::RenkoFixed { box_size: 0.1 }
            ),
            Err(SyntheticBarError::SequenceDomainInUse)
        );
        // Time-bar streams share the time axis.
        let minute_bars = FootprintAggregationOptions {
            bars: FootprintBarAggregation::Time {
                interval_micros: 60_000_000,
                anchor_micros: 0,
            },
            ..FootprintAggregationOptions::default()
        };
        assert!(live
            .chart
            .add_trade_stream("SSE:600000:1m", minute_bars)
            .is_ok());

        let mut sequence = ChartEngine::new(800.0, 400.0, 1.0);
        let source = sequence.add_series(SeriesKind::Candlestick);
        let target = sequence.add_series(SeriesKind::Candlestick);
        sequence
            .add_trade_stream("SSE:600000:ticks", tick_bars)
            .unwrap();
        assert_eq!(
            sequence.configure_resampled_series(
                source,
                None,
                target,
                None,
                ResampleOptions {
                    interval_seconds: 3_600,
                    boundaries,
                }
            ),
            Err(ResampleError::TimeAxisRequired)
        );
    }

    #[test]
    fn resample_boundaries_validate_dates_and_sessions() {
        let time = ExchangeTime::new(shanghai(), 0).unwrap();
        assert_eq!(
            resample_boundaries(&[], &a_share(), &time, ResampleSpan::Window),
            Err(ResampleError::InvalidBoundaries)
        );
        assert_eq!(
            resample_boundaries(
                &[day("2026-09-25"), day("2026-09-24")],
                &a_share(),
                &time,
                ResampleSpan::Window
            ),
            Err(ResampleError::InvalidBoundaries)
        );
        let many = (0..10_001).collect::<Vec<_>>();
        assert_eq!(
            resample_boundaries(&many, &a_share(), &time, ResampleSpan::Window),
            Err(ResampleError::TooManyBoundaries)
        );
        assert_eq!(
            resample_boundaries(&many, &a_share(), &time, ResampleSpan::Day).map(|b| b.len()),
            Ok(10_001)
        );
        assert_eq!(
            resample_boundaries(
                &[day("2026-09-25")],
                &[window("13:00", "15:00"), window("09:30", "11:30")],
                &time,
                ResampleSpan::Window
            ),
            Err(ResampleError::InvalidSessions(
                SessionSlotError::UnorderedWindow { index: 1 }
            ))
        );
        // Under a negative session start a Saturday shares Monday's Friday-evening session.
        let night = ExchangeTime::new(shanghai(), -3 * 3_600).unwrap();
        assert_eq!(
            resample_boundaries(
                &[day("2026-09-26"), day("2026-09-28")],
                &[window("21:00", "02:30"), window("09:00", "15:00")],
                &night,
                ResampleSpan::Window
            ),
            Err(ResampleError::InvalidBoundaries)
        );
        let daily = resample_boundaries(&[day("2026-09-25")], &a_share(), &time, ResampleSpan::Day)
            .unwrap();
        assert_eq!(
            daily,
            [ResampleBoundary {
                start_time: day("2026-09-25") * DAY + HOUR + 1_800,
                end_time: day("2026-09-25") * DAY + 7 * HOUR,
                session_id: 20_260_925,
            }]
        );
    }
}
