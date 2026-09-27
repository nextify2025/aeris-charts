//! Exchange time for the financial time axis.
//!
//! Canonical chart time stays whole UTC seconds. An [`ExchangeTime`] describes how the chart
//! presents and groups those instants: a validated UTC-offset schedule (the exchange time zone,
//! resolved by the host — browsers derive it from an IANA name with `Intl`, Rust hosts pass it
//! directly), a trading-day start relative to local midnight, and whether the time points are
//! calendar dates (`business_day` / `"YYYY-MM-DD"` input) rather than instants.
//!
//! The engine never consults a platform time zone. The default is UTC with a midnight session
//! start, which reproduces the historical UTC-only behavior exactly.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::model::data_validation::{MAX_TIMESTAMP, MIN_TIMESTAMP};
use crate::scale::time_tick_marks::{civil_from_timestamp, days_from_civil};

/// Upper bound on offset transitions. Browser hosts resolve IANA zones over 1970..2100, which
/// needs at most two DST transitions per year (about 262) plus historical rule changes.
pub const MAX_UTC_OFFSET_TRANSITIONS: usize = 1024;
/// ISO 8601 offset bound (±18:00). Real exchange offsets are within ±14:00.
pub const MAX_UTC_OFFSET_SECONDS: i32 = 18 * 3_600;
/// A trading day may begin at most one day before or after local midnight (exclusive).
pub const MAX_SESSION_START_SECONDS: i32 = 86_399;

const DAY: i64 = 86_400;
/// Mean Gregorian month (365.2425 / 12 days) in seconds.
const AVERAGE_MONTH_SECONDS: f64 = 2_629_746.0;

/// One entry of a UTC-offset schedule: from `from_utc_seconds` (inclusive) until the next
/// transition, local wall-clock time is `utc + offset_seconds`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UtcOffsetTransition {
    pub from_utc_seconds: i64,
    pub offset_seconds: i32,
}

/// Validation failures for exchange-time configuration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExchangeTimeError {
    TooManyTransitions { count: usize },
    TransitionOutOfRange { index: usize },
    UnorderedTransition { index: usize },
    OffsetOutOfRange { index: usize, offset_seconds: i32 },
    SessionStartOutOfRange { seconds: i64 },
}

impl fmt::Display for ExchangeTimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooManyTransitions { count } => write!(
                f,
                "time zone schedule has {count} transitions; at most {MAX_UTC_OFFSET_TRANSITIONS} are supported"
            ),
            Self::TransitionOutOfRange { index } => write!(
                f,
                "time zone transition {index} is not a whole UTC second in {MIN_TIMESTAMP}..{MAX_TIMESTAMP}"
            ),
            Self::UnorderedTransition { index } => write!(
                f,
                "time zone transition {index} does not start strictly after the previous transition"
            ),
            Self::OffsetOutOfRange {
                index,
                offset_seconds,
            } => write!(
                f,
                "time zone transition {index} has offset {offset_seconds}s outside ±{MAX_UTC_OFFSET_SECONDS}s"
            ),
            Self::SessionStartOutOfRange { seconds } => write!(
                f,
                "session start {seconds}s is outside ±{MAX_SESSION_START_SECONDS}s of local midnight"
            ),
        }
    }
}

impl std::error::Error for ExchangeTimeError {}

/// A validated, sorted UTC-offset schedule. Before the first transition the first offset applies,
/// so a single entry is a fixed offset; an empty schedule is UTC. Redundant transitions (same
/// offset as their predecessor) are dropped so equal zones compare equal.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UtcOffsetSchedule {
    transitions: Vec<UtcOffsetTransition>,
}

impl UtcOffsetSchedule {
    /// UTC (no transitions).
    pub const fn utc() -> Self {
        Self {
            transitions: Vec::new(),
        }
    }

    /// A fixed offset from UTC.
    pub fn fixed(offset_seconds: i32) -> Result<Self, ExchangeTimeError> {
        Self::new(vec![UtcOffsetTransition {
            from_utc_seconds: MIN_TIMESTAMP,
            offset_seconds,
        }])
    }

    /// Validate and normalize a sorted transition list.
    pub fn new(transitions: Vec<UtcOffsetTransition>) -> Result<Self, ExchangeTimeError> {
        if transitions.len() > MAX_UTC_OFFSET_TRANSITIONS {
            return Err(ExchangeTimeError::TooManyTransitions {
                count: transitions.len(),
            });
        }
        let mut normalized: Vec<UtcOffsetTransition> = Vec::with_capacity(transitions.len());
        let mut previous_from = None;
        for (index, transition) in transitions.into_iter().enumerate() {
            if !(MIN_TIMESTAMP..=MAX_TIMESTAMP).contains(&transition.from_utc_seconds) {
                return Err(ExchangeTimeError::TransitionOutOfRange { index });
            }
            if previous_from.is_some_and(|from| transition.from_utc_seconds <= from) {
                return Err(ExchangeTimeError::UnorderedTransition { index });
            }
            if transition.offset_seconds.abs() > MAX_UTC_OFFSET_SECONDS {
                return Err(ExchangeTimeError::OffsetOutOfRange {
                    index,
                    offset_seconds: transition.offset_seconds,
                });
            }
            previous_from = Some(transition.from_utc_seconds);
            if normalized
                .last()
                .is_none_or(|last| last.offset_seconds != transition.offset_seconds)
            {
                normalized.push(transition);
            }
        }
        if normalized.iter().all(|t| t.offset_seconds == 0) {
            normalized.clear();
        }
        // The first offset also applies before its transition, so its start is irrelevant.
        if let Some(first) = normalized.first_mut() {
            first.from_utc_seconds = MIN_TIMESTAMP;
        }
        normalized.shrink_to_fit();
        Ok(Self {
            transitions: normalized,
        })
    }

    pub fn transitions(&self) -> &[UtcOffsetTransition] {
        &self.transitions
    }

    pub fn is_utc(&self) -> bool {
        self.transitions.is_empty()
    }

    /// Retained heap payload in bytes (memory attribution).
    pub fn capacity_bytes(&self) -> usize {
        self.transitions.capacity() * std::mem::size_of::<UtcOffsetTransition>()
    }

    /// Offset in seconds at a UTC instant. `O(log transitions)`.
    pub fn offset_at(&self, utc_seconds: i64) -> i32 {
        let Some(first) = self.transitions.first() else {
            return 0;
        };
        let index = self
            .transitions
            .partition_point(|t| t.from_utc_seconds <= utc_seconds);
        if index == 0 {
            first.offset_seconds
        } else {
            self.transitions[index - 1].offset_seconds
        }
    }

    /// Local wall-clock seconds for a UTC instant.
    pub fn to_local(&self, utc_seconds: i64) -> i64 {
        utc_seconds.saturating_add(i64::from(self.offset_at(utc_seconds)))
    }

    /// The UTC instant of a local wall-clock time. A wall time skipped by a forward transition
    /// resolves with the offset in force before the transition; a repeated wall time resolves
    /// deterministically to one of its instants.
    pub fn to_utc(&self, local_seconds: i64) -> i64 {
        let guess = local_seconds.saturating_sub(i64::from(self.offset_at(local_seconds)));
        let offset = self.offset_at(guess);
        let utc = local_seconds.saturating_sub(i64::from(offset));
        let settled = self.offset_at(utc);
        if settled == offset {
            utc
        } else {
            local_seconds.saturating_sub(i64::from(settled))
        }
    }
}

/// The exchange clock used by tick weights, built-in time labels, session/weekly/monthly
/// indicator resets, session highlighting, and the candle-close countdown.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExchangeTime {
    offsets: UtcOffsetSchedule,
    session_start_seconds: i32,
    calendar_dates: bool,
}

/// Weekday index with Monday = 0 for days since the Unix epoch (1970-01-01 was a Thursday).
pub fn weekday_from_monday(day: i64) -> u8 {
    (day + 3).rem_euclid(7) as u8
}

impl ExchangeTime {
    /// UTC exchange time with the trading day starting at midnight (the historical behavior).
    pub const UTC: ExchangeTime = ExchangeTime {
        offsets: UtcOffsetSchedule::utc(),
        session_start_seconds: 0,
        calendar_dates: false,
    };

    pub fn new(
        offsets: UtcOffsetSchedule,
        session_start_seconds: i32,
    ) -> Result<Self, ExchangeTimeError> {
        let mut time = Self {
            offsets,
            ..Self::default()
        };
        time.set_session_start_seconds(session_start_seconds)?;
        Ok(time)
    }

    pub fn offsets(&self) -> &UtcOffsetSchedule {
        &self.offsets
    }

    pub fn set_offsets(&mut self, offsets: UtcOffsetSchedule) {
        self.offsets = offsets;
    }

    /// Seconds from local midnight at which a trading day begins. Negative values assign an
    /// evening session to the next trading day (e.g. `-3 * 3600` makes 21:00 the day start).
    pub fn session_start_seconds(&self) -> i32 {
        self.session_start_seconds
    }

    pub fn set_session_start_seconds(&mut self, seconds: i32) -> Result<(), ExchangeTimeError> {
        if seconds.abs() > MAX_SESSION_START_SECONDS {
            return Err(ExchangeTimeError::SessionStartOutOfRange {
                seconds: i64::from(seconds),
            });
        }
        self.session_start_seconds = seconds;
        Ok(())
    }

    /// Whether the chart's time points are calendar dates (`business_day`/`"YYYY-MM-DD"` input
    /// at UTC midnight). Calendar dates are already exchange-local trading days: they are never
    /// shifted by the offset schedule or the session start.
    pub fn calendar_dates(&self) -> bool {
        self.calendar_dates
    }

    pub fn set_calendar_dates(&mut self, calendar_dates: bool) {
        self.calendar_dates = calendar_dates;
    }

    /// True when every mapping is the identity (UTC offsets and a midnight session start).
    pub fn is_utc_identity(&self) -> bool {
        self.offsets.is_utc() && self.session_start_seconds == 0
    }

    /// Exchange-local wall-clock seconds for a canonical timestamp.
    pub fn local_seconds(&self, time: i64) -> i64 {
        if self.calendar_dates {
            time
        } else {
            self.offsets.to_local(time)
        }
    }

    /// Exchange trading day (days since 1970-01-01 of its calendar date) for a timestamp.
    ///
    /// Local time is shifted by the session start. With a negative session start (an evening
    /// session that opens the following trading day), a day that would fall on Saturday or
    /// Sunday rolls forward to Monday, so a Friday-night session belongs to Monday's trading
    /// day. Calendar-date rows are their own trading day.
    pub fn trading_day(&self, time: i64) -> i64 {
        if self.calendar_dates {
            return time.div_euclid(DAY);
        }
        let day = self
            .offsets
            .to_local(time)
            .saturating_sub(i64::from(self.session_start_seconds))
            .div_euclid(DAY);
        if self.session_start_seconds < 0 {
            let weekday = weekday_from_monday(day);
            if weekday >= 5 {
                return day + i64::from(7 - weekday);
            }
        }
        day
    }

    /// `trading_day * 86_400`: the UTC-midnight seconds of the trading date. Period-keyed
    /// indicators consume this form so their day/week/month arithmetic stays calendar-only.
    pub fn trading_day_seconds(&self, time: i64) -> i64 {
        self.trading_day(time).saturating_mul(DAY)
    }

    /// (year, month 1-12, day 1-31) of the trading day.
    pub fn trading_date(&self, time: i64) -> (i64, u32, u32) {
        civil_from_timestamp(self.trading_day_seconds(time))
    }

    /// Local weekday (Monday = 0) of the wall-clock date.
    pub fn local_weekday(&self, time: i64) -> u8 {
        weekday_from_monday(self.local_seconds(time).div_euclid(DAY))
    }

    /// Seconds after local midnight of the wall-clock time.
    pub fn local_seconds_of_day(&self, time: i64) -> i64 {
        self.local_seconds(time).rem_euclid(DAY)
    }

    /// UTC instant at which the trading day of calendar date `day` (days since the epoch) begins,
    /// the inverse of [`Self::trading_day`]. With a negative session start Monday's trading day
    /// begins on the preceding Friday evening, because the weekend days roll forward to Monday.
    pub fn trading_day_start_utc(&self, day: i64) -> i64 {
        self.offsets.to_utc(self.trading_day_start_local(day))
    }

    /// Exchange-local wall-clock seconds at which the trading day of calendar date `day` begins
    /// (see [`Self::trading_day_start_utc`]).
    pub fn trading_day_start_local(&self, day: i64) -> i64 {
        let start_day = if self.session_start_seconds < 0 && weekday_from_monday(day) == 0 {
            day.saturating_sub(2)
        } else {
            day
        };
        start_day
            .saturating_mul(DAY)
            .saturating_add(i64::from(self.session_start_seconds))
    }

    /// UTC window `[start, end)` during which a calendar-date bar dated `day` forms, given the
    /// typical spacing of its bars. Daily and weekly bars span whole trading days
    /// (`interval_seconds` rounded to days); bars at least 28 days apart span calendar months, so
    /// monthly, quarterly, and yearly bars end with their month rather than after an average
    /// month length.
    pub fn calendar_bar_window_utc(&self, day: i64, interval_seconds: f64) -> (i64, i64) {
        let days = (interval_seconds / DAY as f64).round().clamp(1.0, 1e9) as i64;
        let next = if days >= 28 {
            let months = (interval_seconds / AVERAGE_MONTH_SECONDS)
                .round()
                .clamp(1.0, 1e7) as i64;
            let (year, month, _) = civil_from_timestamp(day.saturating_mul(DAY));
            let index = year * 12 + i64::from(month - 1) + months;
            days_from_civil(index.div_euclid(12), index.rem_euclid(12) as u32 + 1, 1)
                .unwrap_or_else(|| day.saturating_add(days))
        } else {
            day.saturating_add(days)
        };
        (
            self.trading_day_start_utc(day),
            self.trading_day_start_utc(next),
        )
    }

    /// Retained heap payload in bytes (memory attribution).
    pub fn capacity_bytes(&self) -> usize {
        self.offsets.capacity_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scale::time_tick_marks::days_from_civil;

    const HOUR: i64 = 3_600;

    fn ts(year: i64, month: u32, day: u32, hour: i64, minute: i64) -> i64 {
        days_from_civil(year, month, day).unwrap() * DAY + hour * HOUR + minute * 60
    }

    /// America/New_York 2024: EDT from 2024-03-10 07:00 UTC, EST from 2024-11-03 06:00 UTC.
    pub(crate) fn new_york_2024() -> UtcOffsetSchedule {
        UtcOffsetSchedule::new(vec![
            UtcOffsetTransition {
                from_utc_seconds: ts(2023, 11, 5, 6, 0),
                offset_seconds: -5 * 3_600,
            },
            UtcOffsetTransition {
                from_utc_seconds: ts(2024, 3, 10, 7, 0),
                offset_seconds: -4 * 3_600,
            },
            UtcOffsetTransition {
                from_utc_seconds: ts(2024, 11, 3, 6, 0),
                offset_seconds: -5 * 3_600,
            },
        ])
        .unwrap()
    }

    #[test]
    fn schedule_validation_and_normalization() {
        assert!(UtcOffsetSchedule::utc().is_utc());
        assert!(UtcOffsetSchedule::fixed(0).unwrap().is_utc());
        assert_eq!(
            UtcOffsetSchedule::fixed(19 * 3_600),
            Err(ExchangeTimeError::OffsetOutOfRange {
                index: 0,
                offset_seconds: 19 * 3_600
            })
        );
        let unordered = UtcOffsetSchedule::new(vec![
            UtcOffsetTransition {
                from_utc_seconds: 10,
                offset_seconds: 0,
            },
            UtcOffsetTransition {
                from_utc_seconds: 10,
                offset_seconds: 3_600,
            },
        ]);
        assert_eq!(
            unordered,
            Err(ExchangeTimeError::UnorderedTransition { index: 1 })
        );
        let too_many = (0..=MAX_UTC_OFFSET_TRANSITIONS as i64)
            .map(|index| UtcOffsetTransition {
                from_utc_seconds: index,
                offset_seconds: 0,
            })
            .collect();
        assert!(matches!(
            UtcOffsetSchedule::new(too_many),
            Err(ExchangeTimeError::TooManyTransitions { .. })
        ));
        let redundant = UtcOffsetSchedule::new(vec![
            UtcOffsetTransition {
                from_utc_seconds: 0,
                offset_seconds: 3_600,
            },
            UtcOffsetTransition {
                from_utc_seconds: 100,
                offset_seconds: 3_600,
            },
        ])
        .unwrap();
        assert_eq!(redundant, UtcOffsetSchedule::fixed(3_600).unwrap());
    }

    #[test]
    fn offsets_follow_dst_transitions_in_both_directions() {
        let new_york = new_york_2024();
        assert_eq!(new_york.offset_at(ts(2024, 3, 8, 14, 30)), -5 * 3_600);
        assert_eq!(new_york.offset_at(ts(2024, 3, 11, 13, 30)), -4 * 3_600);
        // Before the first transition the first offset applies.
        assert_eq!(new_york.offset_at(0), -5 * 3_600);
        // 09:30 ET maps to 14:30 UTC in winter and 13:30 UTC in summer.
        assert_eq!(
            new_york.to_utc(ts(2024, 3, 8, 9, 30)),
            ts(2024, 3, 8, 14, 30)
        );
        assert_eq!(
            new_york.to_utc(ts(2024, 3, 11, 9, 30)),
            ts(2024, 3, 11, 13, 30)
        );
        // A skipped wall time still resolves deterministically.
        let skipped = new_york.to_utc(ts(2024, 3, 10, 2, 30));
        assert_eq!(new_york.to_local(skipped), ts(2024, 3, 10, 3, 30));
    }

    #[test]
    fn trading_days_follow_exchange_time_and_session_start() {
        let shanghai = ExchangeTime::new(UtcOffsetSchedule::fixed(8 * 3_600).unwrap(), 0).unwrap();
        // 2024-01-02 09:30 CST is 01:30 UTC on the same date.
        let open = ts(2024, 1, 2, 1, 30);
        assert_eq!(shanghai.trading_date(open), (2024, 1, 2));
        // 2024-01-02 20:00 UTC is already Jan 3 in Shanghai.
        assert_eq!(shanghai.trading_date(ts(2024, 1, 2, 20, 0)), (2024, 1, 3));

        // China futures: a 21:00 night session belongs to the next trading day, and Friday night
        // belongs to Monday.
        let futures =
            ExchangeTime::new(UtcOffsetSchedule::fixed(8 * 3_600).unwrap(), -3 * 3_600).unwrap();
        // Tuesday 2024-01-02 21:00 CST = 13:00 UTC -> Wednesday.
        assert_eq!(futures.trading_date(ts(2024, 1, 2, 13, 0)), (2024, 1, 3));
        // Tuesday 20:59 CST stays on Tuesday.
        assert_eq!(futures.trading_date(ts(2024, 1, 2, 12, 59)), (2024, 1, 2));
        // Friday 2024-01-05 21:00 CST and Saturday 01:00 CST -> Monday 2024-01-08.
        assert_eq!(futures.trading_date(ts(2024, 1, 5, 13, 0)), (2024, 1, 8));
        assert_eq!(futures.trading_date(ts(2024, 1, 5, 17, 0)), (2024, 1, 8));
        // Monday's day session stays on Monday.
        assert_eq!(futures.trading_date(ts(2024, 1, 8, 1, 0)), (2024, 1, 8));

        // Calendar dates ignore the zone and session start.
        let mut dates = futures.clone();
        dates.set_calendar_dates(true);
        assert_eq!(dates.trading_date(ts(2024, 1, 5, 0, 0)), (2024, 1, 5));
        assert_eq!(
            dates.local_seconds(ts(2024, 1, 5, 0, 0)),
            ts(2024, 1, 5, 0, 0)
        );

        assert!(ExchangeTime::default().is_utc_identity());
        assert_eq!(ExchangeTime::UTC, ExchangeTime::default());
        assert_eq!(
            ExchangeTime::default().set_session_start_seconds(86_400),
            Err(ExchangeTimeError::SessionStartOutOfRange { seconds: 86_400 })
        );
    }

    #[test]
    fn trading_day_start_and_weekdays() {
        let new_york = ExchangeTime::new(new_york_2024(), 0).unwrap();
        let day = days_from_civil(2024, 3, 11).unwrap();
        assert_eq!(new_york.trading_day_start_utc(day), ts(2024, 3, 11, 4, 0));
        assert_eq!(weekday_from_monday(0), 3);
        assert_eq!(weekday_from_monday(day), 0);
        // Friday 19:30 ET in winter is Saturday 00:30 UTC but still Friday locally.
        assert_eq!(new_york.local_weekday(ts(2024, 1, 6, 0, 30)), 4);
        assert_eq!(
            new_york.local_seconds_of_day(ts(2024, 1, 6, 0, 30)),
            19 * HOUR + 30 * 60
        );
    }

    #[test]
    fn trading_day_starts_invert_the_weekend_roll_and_calendar_windows_follow_months() {
        let futures =
            ExchangeTime::new(UtcOffsetSchedule::fixed(8 * 3_600).unwrap(), -3 * 3_600).unwrap();
        let monday = days_from_civil(2024, 1, 8).unwrap();
        // Monday's trading day opens with the Friday 21:00 CST night session (13:00 UTC).
        let monday_start = futures.trading_day_start_utc(monday);
        assert_eq!(monday_start, ts(2024, 1, 5, 13, 0));
        assert_eq!(futures.trading_day(monday_start), monday);
        assert_eq!(futures.trading_day(monday_start - 1), monday - 3);
        // Tuesday opens Monday 21:00 CST.
        assert_eq!(
            futures.trading_day_start_utc(monday + 1),
            ts(2024, 1, 8, 13, 0)
        );
        // Friday's daily bar ends where Monday's trading day begins.
        assert_eq!(
            futures.calendar_bar_window_utc(monday - 3, 86_400.0),
            (ts(2024, 1, 4, 13, 0), ts(2024, 1, 5, 13, 0))
        );

        // Bars at least 28 days apart end with their calendar month(s), whatever the average.
        let utc = ExchangeTime::default();
        let may = days_from_civil(2024, 5, 1).unwrap();
        let june = days_from_civil(2024, 6, 1).unwrap() * DAY;
        assert_eq!(
            utc.calendar_bar_window_utc(may, 30.5 * 86_400.0),
            (may * DAY, june)
        );
        let january = days_from_civil(2024, 1, 2).unwrap();
        assert_eq!(
            utc.calendar_bar_window_utc(january, 91.0 * 86_400.0).1,
            days_from_civil(2024, 4, 1).unwrap() * DAY
        );
        assert_eq!(
            utc.calendar_bar_window_utc(january, 365.0 * 86_400.0).1,
            days_from_civil(2025, 1, 1).unwrap() * DAY
        );
        // Weekly bars span seven trading days.
        assert_eq!(
            utc.calendar_bar_window_utc(monday, 7.0 * 86_400.0),
            (monday * DAY, (monday + 7) * DAY)
        );
    }
}
