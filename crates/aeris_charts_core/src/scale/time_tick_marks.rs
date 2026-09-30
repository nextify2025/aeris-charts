//! Time tick-mark weights and mark selection.
//! Ports of `src/model/horz-scale-behavior-time/time-scale-point-weight-generator.ts`
//! and `src/model/tick-marks.ts`.
//!
//! Weights are assigned per point by comparing consecutive timestamps in exchange time: the
//! largest trading-day/calendar/time boundary crossed between neighbors determines the weight.
//! The default [`ExchangeTime::UTC`] reproduces the reference's UTC boundaries exactly. Mark
//! selection keeps higher weights first and inserts lower-weight marks only where they fit.

use std::collections::BTreeMap;

use crate::scale::exchange_time::ExchangeTime;
use crate::TimePointIndex;

/// Exact values from the reference's `TickMarkWeight` (`horz-scale-behavior-time/types.ts`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum TickMarkWeight {
    LessThanSecond = 0,
    Second = 10,
    Minute1 = 20,
    Minute5 = 21,
    Minute30 = 22,
    Hour1 = 30,
    Hour3 = 31,
    Hour6 = 32,
    Hour12 = 33,
    Day = 50,
    Month = 60,
    Year = 70,
}

/// (year, month 1-12, day 1-31) from days since the Unix epoch.
/// Howard Hinnant's `civil_from_days` — exact for the proleptic Gregorian calendar,
/// matching JS `Date` UTC accessors.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Days since the Unix epoch for a proleptic-Gregorian civil date. Returns `None` for an invalid
/// month/day or arithmetic overflow. This is the inverse of [`civil_from_timestamp`] at UTC
/// midnight and keeps calendar-aligned temporal axes independent of platform date libraries.
pub fn days_from_civil(year: i64, month: u32, day: u32) -> Option<i64> {
    if !(1..=12).contains(&month) {
        return None;
    }
    let leap = year.rem_euclid(4) == 0 && (year.rem_euclid(100) != 0 || year.rem_euclid(400) == 0);
    let days_in_month = match month {
        2 if leap => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    if day == 0 || day > days_in_month {
        return None;
    }

    let adjusted_year = year.checked_sub(i64::from(month <= 2))?;
    let era = adjusted_year.div_euclid(400);
    let year_of_era = adjusted_year - era * 400;
    let shifted_month = i64::from(month) + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * shifted_month + 2) / 5 + i64::from(day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era.checked_mul(146_097)?
        .checked_add(day_of_era)?
        .checked_sub(719_468)
}

/// (year, month 1-12, day 1-31) of a UTC timestamp in seconds.
pub fn civil_from_timestamp(ts: i64) -> (i64, u32, u32) {
    civil_from_days(ts.div_euclid(86_400))
}

/// Intraday boundary divisors in seconds, smallest to largest (reference iterates largest first).
const INTRADAY_DIVISORS: [(i64, TickMarkWeight); 8] = [
    (1, TickMarkWeight::Second),
    (60, TickMarkWeight::Minute1),
    (300, TickMarkWeight::Minute5),
    (1800, TickMarkWeight::Minute30),
    (3600, TickMarkWeight::Hour1),
    (10_800, TickMarkWeight::Hour3),
    (21_600, TickMarkWeight::Hour6),
    (43_200, TickMarkWeight::Hour12),
];

/// Port of `weightByTime`: weight of `current` given the previous point's timestamp (UTC).
pub fn weight_by_time(current_ts: i64, prev_ts: i64) -> TickMarkWeight {
    weight_by_time_in(current_ts, prev_ts, &ExchangeTime::UTC)
}

/// [`weight_by_time`] in exchange time. Year/Month/Day boundaries compare exchange trading days
/// (local date shifted by the session start); intraday boundaries compare local wall-clock time,
/// so hour and minute marks stay on exchange hours across DST and non-hour offsets.
pub fn weight_by_time_in(current_ts: i64, prev_ts: i64, time: &ExchangeTime) -> TickMarkWeight {
    let current_day = time.trading_day(current_ts);
    let prev_day = time.trading_day(prev_ts);
    if current_day != prev_day {
        let (cy, cm, _) = civil_from_days(current_day);
        let (py, pm, _) = civil_from_days(prev_day);
        return if cy != py {
            TickMarkWeight::Year
        } else if cm != pm {
            TickMarkWeight::Month
        } else {
            TickMarkWeight::Day
        };
    }

    let current = time.local_seconds(current_ts);
    let prev = time.local_seconds(prev_ts);
    for &(divisor, weight) in INTRADAY_DIVISORS.iter().rev() {
        if prev.div_euclid(divisor) != current.div_euclid(divisor) {
            return weight;
        }
    }

    TickMarkWeight::LessThanSecond
}

/// Port of `fillWeightsForPoints`. `times` are UTC timestamps in seconds; writes
/// `weights[start_index..]`. The first point's weight is guessed by extrapolating the
/// average time diff backwards.
pub fn fill_weights_for_points(times: &[i64], weights: &mut [u8], start_index: usize) {
    fill_weights_for_points_in(times, weights, start_index, &ExchangeTime::UTC);
}

/// [`fill_weights_for_points`] in exchange time (see [`weight_by_time_in`]).
pub fn fill_weights_for_points_in(
    times: &[i64],
    weights: &mut [u8],
    start_index: usize,
    time: &ExchangeTime,
) {
    debug_assert_eq!(times.len(), weights.len());
    if times.is_empty() {
        return;
    }

    let mut prev_time: Option<i64> = if start_index == 0 {
        None
    } else {
        Some(times[start_index - 1])
    };
    let mut total_time_diff: i64 = 0;

    for index in start_index..times.len() {
        let current = times[index];
        if let Some(prev) = prev_time {
            weights[index] = weight_by_time_in(current, prev, time) as u8;
        }
        total_time_diff += current - prev_time.unwrap_or(current);
        prev_time = Some(current);
    }

    if start_index == 0 && times.len() > 1 {
        weights[0] = first_point_weight_in(times[0], total_time_diff, times.len(), time);
    }
}

/// The guessed weight of the first of `len > 1` points spanning `span` seconds: pretend the
/// previous point was the average time diff back in history.
pub fn first_point_weight_in(first: i64, span: i64, len: usize, time: &ExchangeTime) -> u8 {
    let average_time_diff = ((span as f64) / (len as f64 - 1.0)).ceil() as i64;
    weight_by_time_in(first, first - average_time_diff, time) as u8
}

/// `Some(k)` when `new` is `old` without its first `k >= 1` points, followed by any later points:
/// every surviving point keeps its timestamp, and with it the weight it was given against its
/// predecessor. Both sequences are ascending.
pub fn front_trim(old: &[i64], new: &[i64]) -> Option<usize> {
    let first = *new.first()?;
    let dropped = old.partition_point(|&time| time < first);
    let survivors = old.get(dropped..)?;
    (dropped > 0 && survivors.first() == Some(&first) && new.starts_with(survivors))
        .then_some(dropped)
}

/// A selectable tick mark: time-point index + weight.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TickMark {
    pub index: TimePointIndex,
    pub weight: u8,
}

/// Port of the `TickMarks` container: marks grouped by weight, selection by available space.
#[derive(Default)]
pub struct TimeTickMarks {
    marks_by_weight: BTreeMap<u8, Vec<TimePointIndex>>,
    cache: Option<(i64, Vec<TickMark>)>,
}

impl TimeTickMarks {
    pub fn new() -> Self {
        Self::default()
    }

    /// Estimated live vector payload retained by tick selection. Map/node allocator overhead is
    /// intentionally excluded; benchmark reports label this as structure payload, not heap truth.
    pub fn payload_bytes(&self) -> usize {
        let buckets = self
            .marks_by_weight
            .values()
            .map(|indices| indices.len() * std::mem::size_of::<TimePointIndex>())
            .sum::<usize>();
        let cache = self.cache.as_ref().map_or(0, |(_, marks)| {
            marks.len() * std::mem::size_of::<TickMark>()
        });
        buckets + cache
    }

    pub fn capacity_bytes(&self) -> usize {
        let buckets = self
            .marks_by_weight
            .values()
            .map(|indices| indices.capacity() * std::mem::size_of::<TimePointIndex>())
            .sum::<usize>();
        let cache = self.cache.as_ref().map_or(0, |(_, marks)| {
            marks.capacity() * std::mem::size_of::<TickMark>()
        });
        buckets + cache
    }

    /// Full rebuild from per-point weights (incremental `firstChangedPointIndex` variant
    /// comes with the data layer).
    pub fn set_weights(&mut self, weights: &[u8]) {
        self.marks_by_weight.clear();
        self.cache = None;
        for (index, &weight) in weights.iter().enumerate() {
            self.marks_by_weight
                .entry(weight)
                .or_default()
                .push(index as TimePointIndex);
        }
    }

    /// Drop the first `dropped` points: every other point keeps its weight at an index lowered by
    /// `dropped`, and the point that becomes the first takes `first_weight` (it has no
    /// predecessor, so it is guessed). Equals [`Self::set_weights`] over the surviving weights
    /// without re-weighing any point.
    pub fn drop_front(&mut self, dropped: usize, first_weight: u8) {
        self.cache = None;
        let first = dropped as TimePointIndex;
        self.marks_by_weight.retain(|_, indices| {
            indices.drain(..indices.partition_point(|&index| index <= first));
            indices.iter_mut().for_each(|index| *index -= first);
            !indices.is_empty()
        });
        self.marks_by_weight
            .entry(first_weight)
            .or_default()
            .insert(0, 0);
    }

    /// Append weights for newly-added points without rebuilding prior weight buckets.
    pub fn append_weights(&mut self, start_index: usize, weights: &[u8]) {
        self.cache = None;
        for (offset, &weight) in weights.iter().enumerate().skip(start_index) {
            self.marks_by_weight
                .entry(weight)
                .or_default()
                .push(offset as TimePointIndex);
        }
    }

    /// Streaming hot path: append a single point's weight without any O(n) scratch vector.
    /// Matches `fill_weights_for_points` semantics for one appended point with a known
    /// predecessor.
    pub fn push_weight(&mut self, index: TimePointIndex, weight: u8) {
        self.cache = None;
        self.marks_by_weight.entry(weight).or_default().push(index);
    }

    /// Port of `TickMarks.build`: `max_width` is the max label width in px
    /// (`(font_size + 4) * 5 / 8 * max_label_chars`), `spacing` the current bar spacing.
    pub fn build(&mut self, spacing: f64, max_width: f64) -> &[TickMark] {
        let max_indexes_per_mark = (max_width / spacing).ceil() as i64;
        if self
            .cache
            .as_ref()
            .is_none_or(|(cached, _)| *cached != max_indexes_per_mark)
        {
            let marks = self.build_impl(max_indexes_per_mark);
            self.cache = Some((max_indexes_per_mark, marks));
        }
        match self.cache.as_ref() {
            Some((_, marks)) => marks,
            None => &[],
        }
    }

    fn build_impl(&self, max_indexes_per_mark: i64) -> Vec<TickMark> {
        let mut marks: Vec<TickMark> = Vec::new();

        for (&weight, current_weight_marks) in self.marks_by_weight.iter().rev() {
            // built marks so far become prev_marks; marks restarts
            let prev_marks = marks;
            marks = Vec::with_capacity(prev_marks.len() + current_weight_marks.len());

            let mut prev_marks_pointer = 0usize;
            let mut right_index = i64::MAX;
            let mut left_index = i64::MIN;

            for &current_index in current_weight_marks {
                // move all prev marks strictly left of current into the result
                while prev_marks_pointer < prev_marks.len() {
                    let last_mark = prev_marks[prev_marks_pointer];
                    if last_mark.index < current_index {
                        prev_marks_pointer += 1;
                        marks.push(last_mark);
                        left_index = last_mark.index;
                        right_index = i64::MAX;
                    } else {
                        right_index = last_mark.index;
                        break;
                    }
                }

                // saturating: sentinels are i64::MAX/MIN (reference uses ±Infinity)
                if right_index.saturating_sub(current_index) >= max_indexes_per_mark
                    && current_index.saturating_sub(left_index) >= max_indexes_per_mark
                {
                    marks.push(TickMark {
                        index: current_index,
                        weight,
                    });
                    left_index = current_index;
                }
            }

            // append the unused prev marks
            for &m in &prev_marks[prev_marks_pointer..] {
                marks.push(m);
            }
        }

        marks
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_date_conversion() {
        // 2018-06-25T04:00:00Z (the reference's doc example timestamp)
        assert_eq!(civil_from_timestamp(1_529_899_200), (2018, 6, 25));
        // epoch
        assert_eq!(civil_from_timestamp(0), (1970, 1, 1));
        // leap day
        assert_eq!(civil_from_timestamp(1_582_934_400), (2020, 2, 29));
        // pre-epoch
        assert_eq!(civil_from_timestamp(-86_400), (1969, 12, 31));
        for &(year, month, day) in &[
            (1970, 1, 1),
            (1969, 12, 31),
            (2020, 2, 29),
            (-400, 3, 1),
            (285_000, 12, 31),
        ] {
            let days = days_from_civil(year, month, day).unwrap();
            assert_eq!(civil_from_timestamp(days * 86_400), (year, month, day));
        }
        assert_eq!(days_from_civil(2021, 2, 29), None);
    }

    #[test]
    fn weights_by_boundary() {
        let day = 86_400;
        // year boundary: 2019-12-31 -> 2020-01-01
        let y2020 = 1_577_836_800; // 2020-01-01T00:00:00Z
        assert_eq!(weight_by_time(y2020, y2020 - day), TickMarkWeight::Year);
        // month boundary: 2020-01-31 -> 2020-02-01
        let feb1 = 1_580_515_200;
        assert_eq!(weight_by_time(feb1, feb1 - day), TickMarkWeight::Month);
        // plain day boundary
        let jan15 = 1_579_046_400; // 2020-01-15
        assert_eq!(weight_by_time(jan15, jan15 - day), TickMarkWeight::Day);
        // intraday: crossing 12h boundary
        assert_eq!(
            weight_by_time(jan15 + 43_200, jan15 + 43_100),
            TickMarkWeight::Hour12
        );
        // crossing 1h but not 3h
        assert_eq!(
            weight_by_time(jan15 + 3600, jan15 + 3599),
            TickMarkWeight::Hour1
        );
        // crossing 1min but not 5min
        assert_eq!(
            weight_by_time(jan15 + 60, jan15 + 59),
            TickMarkWeight::Minute1
        );
        // same second
        assert_eq!(weight_by_time(jan15, jan15), TickMarkWeight::LessThanSecond);
    }

    #[test]
    fn fill_weights_guesses_first_point() {
        // hourly bars starting mid-day
        let times: Vec<i64> = (0..48).map(|i| 1_579_046_400 + i * 3600).collect();
        let mut weights = vec![0u8; times.len()];
        fill_weights_for_points(&times, &mut weights, 0);

        // first point: avg diff 3600 back -> crosses an hour boundary at minimum
        assert!(weights[0] >= TickMarkWeight::Hour1 as u8);
        // index 24 is the next midnight -> Day weight
        assert_eq!(weights[24], TickMarkWeight::Day as u8);
        // other intraday points are hour-weighted
        assert_eq!(weights[1], TickMarkWeight::Hour1 as u8);
        assert_eq!(weights[12], TickMarkWeight::Hour12 as u8);
    }

    /// Ascending times whose gaps reach every weight: seconds, minutes, hours, days, months.
    fn spread_times(rng: &mut u64, len: usize) -> Vec<i64> {
        let mut next = move || {
            *rng = rng
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            *rng >> 33
        };
        let mut time = 1_579_046_400 + (next() % 86_400) as i64;
        (0..len)
            .map(|_| {
                time += match next() % 8 {
                    0 => 1,
                    1 => 60,
                    2 => 300,
                    3 => 3_600,
                    4 => 21_600,
                    5 => 86_400,
                    6 => 2_678_400,
                    _ => 1 + (next() % 7_200) as i64,
                };
                time
            })
            .collect()
    }

    fn clean_marks(times: &[i64]) -> TimeTickMarks {
        let mut weights = vec![0u8; times.len()];
        fill_weights_for_points(times, &mut weights, 0);
        let mut marks = TimeTickMarks::new();
        marks.set_weights(&weights);
        marks
    }

    #[test]
    fn dropping_the_front_equals_a_clean_fill_of_the_surviving_points() {
        let mut rng = 0x2545_F491_4F6C_DD1D_u64;
        for round in 0..200 {
            let len = 2 + (round * 7) % 90;
            let times = spread_times(&mut rng, len + 12);
            let (old, new) = (&times[..len], &times[..len + 12]);
            for dropped in [1, len / 2, len - 1] {
                let new = &new[dropped..];
                assert_eq!(front_trim(old, new), Some(dropped), "round {round}");
                let mut marks = clean_marks(old);
                let span = new[new.len() - 1] - new[0];
                let first = first_point_weight_in(new[0], span, new.len(), &ExchangeTime::UTC);
                marks.drop_front(dropped, first);
                for index in len - dropped..new.len() {
                    let weight = weight_by_time(new[index], new[index - 1]) as u8;
                    marks.push_weight(index as TimePointIndex, weight);
                }
                let clean = clean_marks(new);
                assert_eq!(
                    marks.marks_by_weight, clean.marks_by_weight,
                    "round {round} dropped {dropped}"
                );
                assert_eq!(
                    marks.build(1_000.0, 10.0),
                    clean_marks(new).build(1_000.0, 10.0)
                );
            }
        }
    }

    #[test]
    fn a_front_trim_is_recognized_only_when_every_survivor_keeps_its_time() {
        let old = [10, 20, 30, 40];
        assert_eq!(front_trim(&old, &[30, 40]), Some(2));
        assert_eq!(front_trim(&old, &[20, 30, 40, 50, 60]), Some(1));
        // Nothing left of the old points, nothing dropped, or an empty side.
        assert_eq!(front_trim(&old, &[50, 60]), None);
        assert_eq!(front_trim(&old, &old), None);
        assert_eq!(front_trim(&old, &[10, 20, 30, 40, 50]), None);
        assert_eq!(front_trim(&old, &[]), None);
        assert_eq!(front_trim(&[], &[1, 2]), None);
        // The first survivor moved, a middle point was replaced, or the tail shrank.
        assert_eq!(front_trim(&old, &[25, 30, 40]), None);
        assert_eq!(front_trim(&old, &[20, 35, 40]), None);
        assert_eq!(front_trim(&old, &[20, 30]), None);
        // One surviving point followed by new ones.
        assert_eq!(front_trim(&old, &[40, 50, 60]), Some(3));
    }

    #[test]
    fn build_keeps_high_weights_and_spacing() {
        // 100 daily points; every 10th is Month weight, rest Day
        let mut weights = vec![TickMarkWeight::Day as u8; 100];
        for i in (0..100).step_by(10) {
            weights[i] = TickMarkWeight::Month as u8;
        }
        let mut tm = TimeTickMarks::new();
        tm.set_weights(&weights);

        // plenty of space: max_indexes_per_mark = ceil(80/40) = 2 -> months + days that fit
        let marks = tm.build(40.0, 80.0).to_vec();
        assert!(!marks.is_empty());
        // all month marks must be present
        let month_count = marks
            .iter()
            .filter(|m| m.weight == TickMarkWeight::Month as u8)
            .count();
        assert_eq!(month_count, 10);
        // result sorted by index
        assert!(marks.windows(2).all(|w| w[0].index < w[1].index));
        // no two marks closer than max_indexes_per_mark... except between two high-weight marks
        // (higher weights always win); day marks must respect spacing vs neighbors
        for w in marks.windows(2) {
            if w[0].weight != w[1].weight {
                assert!((w[1].index - w[0].index) >= 2, "{:?}", w);
            }
        }

        // tight space: only high-weight marks survive
        let tight = tm.build(4.0, 80.0).to_vec(); // max_indexes_per_mark = 20
        assert!(tight
            .iter()
            .all(|m| m.weight == TickMarkWeight::Month as u8));
        // and they respect the 20-index spacing (every other month mark dropped)
        assert!(tight.windows(2).all(|w| w[1].index - w[0].index >= 20));
    }

    #[test]
    fn build_cache_invalidates_on_spacing_change() {
        let weights = vec![TickMarkWeight::Day as u8; 50];
        let mut tm = TimeTickMarks::new();
        tm.set_weights(&weights);
        let wide = tm.build(80.0, 80.0).len(); // 1 index per mark -> all fit
        let narrow = tm.build(2.0, 80.0).len(); // 40 indexes per mark -> few fit
        assert!(wide > narrow);
    }

    use crate::scale::exchange_time::{UtcOffsetSchedule, UtcOffsetTransition};

    /// UTC instant of an exchange-local wall-clock minute.
    fn local(time: &ExchangeTime, year: i64, month: u32, day: u32, hour: i64, minute: i64) -> i64 {
        let wall = days_from_civil(year, month, day).unwrap() * 86_400 + hour * 3_600 + minute * 60;
        time.offsets().to_utc(wall)
    }

    fn weights_in(times: &[i64], time: &ExchangeTime) -> Vec<u8> {
        let mut weights = vec![0u8; times.len()];
        fill_weights_for_points_in(times, &mut weights, 0, time);
        weights
    }

    fn minute_bars(
        time: &ExchangeTime,
        date: (i64, u32, u32),
        from: (i64, i64),
        to: (i64, i64),
        step_minutes: i64,
    ) -> Vec<i64> {
        let start = from.0 * 60 + from.1;
        let end = to.0 * 60 + to.1;
        (start..=end)
            .step_by(step_minutes as usize)
            .map(|minute| local(time, date.0, date.1, date.2, minute / 60, minute % 60))
            .collect()
    }

    fn new_york() -> ExchangeTime {
        let at = |y, m, d, h: i64| days_from_civil(y, m, d).unwrap() * 86_400 + h * 3_600;
        ExchangeTime::new(
            UtcOffsetSchedule::new(vec![
                UtcOffsetTransition {
                    from_utc_seconds: at(2023, 11, 5, 6),
                    offset_seconds: -5 * 3_600,
                },
                UtcOffsetTransition {
                    from_utc_seconds: at(2024, 3, 10, 7),
                    offset_seconds: -4 * 3_600,
                },
            ])
            .unwrap(),
            0,
        )
        .unwrap()
    }

    #[test]
    fn utc_default_matches_the_reference_weights() {
        let times: Vec<i64> = (0..200).map(|i| 1_579_046_400 + i * 1_700).collect();
        let mut reference = vec![0u8; times.len()];
        fill_weights_for_points(&times, &mut reference, 0);
        assert_eq!(weights_in(&times, &ExchangeTime::UTC), reference);
    }

    #[test]
    fn a_share_lunch_break_weights_follow_shanghai_time() {
        let shanghai = ExchangeTime::new(UtcOffsetSchedule::fixed(8 * 3_600).unwrap(), 0).unwrap();
        let mut times = Vec::new();
        for day in [2, 3] {
            times.extend(minute_bars(&shanghai, (2024, 1, day), (9, 30), (11, 30), 1));
            times.extend(minute_bars(&shanghai, (2024, 1, day), (13, 0), (15, 0), 1));
        }
        let weights = weights_in(&times, &shanghai);
        let per_day = times.len() / 2;
        // The first bar of the second trading day carries the Day mark; nothing else does.
        let day_marks: Vec<usize> = weights
            .iter()
            .enumerate()
            .skip(1)
            .filter(|(_, &w)| w >= TickMarkWeight::Day as u8)
            .map(|(index, _)| index)
            .collect();
        assert_eq!(day_marks, vec![per_day]);
        // 11:30 -> 13:00 crosses local noon: Hour12 in exchange time (UTC would give Hour1).
        let afternoon_open = 121;
        assert_eq!(weights[afternoon_open], TickMarkWeight::Hour12 as u8);
        assert_eq!(
            weight_by_time(times[afternoon_open], times[afternoon_open - 1]),
            TickMarkWeight::Hour1
        );
        // 10:00 local is an hour boundary (02:00 UTC).
        assert_eq!(weights[30], TickMarkWeight::Hour1 as u8);
    }

    #[test]
    fn us_dst_week_keeps_day_marks_and_hours_on_eastern_time() {
        let eastern = new_york();
        let mut times = minute_bars(&eastern, (2024, 3, 8), (9, 30), (16, 0), 30);
        let friday = times.len();
        times.extend(minute_bars(&eastern, (2024, 3, 11), (9, 30), (16, 0), 30));
        let weights = weights_in(&times, &eastern);
        assert_eq!(weights[friday], TickMarkWeight::Day as u8);
        // 12:00 ET crosses local noon on both sides of the DST change.
        let noon = 5; // 09:30, 10:00, 10:30, 11:00, 11:30, 12:00
        assert_eq!(weights[noon], TickMarkWeight::Hour12 as u8);
        assert_eq!(weights[friday + noon], TickMarkWeight::Hour12 as u8);
        // 10:00 ET is the same Hour1 boundary on both days in exchange time, while UTC shifts it
        // from a Hour3 boundary (15:00 UTC) to a Hour1 boundary (14:00 UTC).
        assert_eq!(weights[1], TickMarkWeight::Hour1 as u8);
        assert_eq!(weights[friday + 1], TickMarkWeight::Hour1 as u8);
        let utc = weights_in(&times, &ExchangeTime::UTC);
        assert_ne!(utc[1], utc[friday + 1]);
    }

    #[test]
    fn us_extended_hours_in_winter_have_no_mid_session_day_mark() {
        let eastern = new_york();
        let mut times = minute_bars(&eastern, (2024, 1, 8), (4, 0), (20, 0), 60);
        let next = times.len();
        times.extend(minute_bars(&eastern, (2024, 1, 9), (4, 0), (20, 0), 60));
        let weights = weights_in(&times, &eastern);
        let day_marks: Vec<usize> = weights
            .iter()
            .enumerate()
            .skip(1)
            .filter(|(_, &w)| w >= TickMarkWeight::Day as u8)
            .map(|(index, _)| index)
            .collect();
        assert_eq!(day_marks, vec![next]);
        // UTC would put the Day mark on the 19:00 ET bar (00:00 UTC).
        let utc = weights_in(&times, &ExchangeTime::UTC);
        assert_eq!(utc[15], TickMarkWeight::Day as u8);
    }

    #[test]
    fn china_futures_night_session_starts_the_trading_day() {
        let futures =
            ExchangeTime::new(UtcOffsetSchedule::fixed(8 * 3_600).unwrap(), -3 * 3_600).unwrap();
        // Tuesday day session, Tuesday night session (Wednesday's trading day), Wednesday day.
        let mut times = minute_bars(&futures, (2024, 1, 2), (13, 30), (15, 0), 30);
        let night = times.len();
        times.extend(minute_bars(&futures, (2024, 1, 2), (21, 0), (23, 0), 30));
        let day = times.len();
        times.extend(minute_bars(&futures, (2024, 1, 3), (9, 0), (11, 30), 30));
        let weights = weights_in(&times, &futures);
        assert_eq!(weights[night], TickMarkWeight::Day as u8);
        assert!(weights[day] < TickMarkWeight::Day as u8);
        // Friday night belongs to Monday: the Monday day session is not a new trading day.
        let mut weekend = minute_bars(&futures, (2024, 1, 5), (21, 0), (23, 0), 60);
        let monday = weekend.len();
        weekend.extend(minute_bars(&futures, (2024, 1, 8), (9, 0), (11, 0), 60));
        let weights = weights_in(&weekend, &futures);
        assert!(weights[monday] < TickMarkWeight::Day as u8);
    }

    #[test]
    fn calendar_dates_ignore_the_exchange_offset() {
        let mut eastern = new_york();
        eastern.set_calendar_dates(true);
        let days: Vec<i64> = (0..40)
            .map(|index| (days_from_civil(2024, 1, 15).unwrap() + index) * 86_400)
            .collect();
        let mut reference = vec![0u8; days.len()];
        fill_weights_for_points(&days, &mut reference, 0);
        assert_eq!(weights_in(&days, &eastern), reference);
    }

    #[test]
    fn append_weights_keeps_existing_marks_and_adds_new_point() {
        let mut marks = TimeTickMarks::new();
        marks.set_weights(&[50]);
        marks.append_weights(1, &[50, 50]);
        let built = marks.build(10.0, 10.0);
        assert!(built.iter().any(|mark| mark.index == 0));
        assert!(built.iter().any(|mark| mark.index == 1));
    }
}
