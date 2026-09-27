//! Session slots: the UTC bar times of one trading day's session windows.
//!
//! Intraday (time-sharing, 分时) charts reserve a slot for every bar of the session before it
//! trades, so the time axis spans the whole session from the open. Hosts own exchange calendars
//! (which days trade, half days, holidays); this module only turns one trading date plus its
//! exchange-local session windows into canonical UTC bar times, with the offset in force on that
//! date, so the same windows stay on exchange hours across DST.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::model::data_validation::{MAX_TIMESTAMP, MIN_TIMESTAMP};
use crate::scale::exchange_time::ExchangeTime;
use crate::scale::time_tick_marks::days_from_civil;

/// At most this many session windows per trading day.
pub const MAX_SESSION_WINDOWS: usize = 32;
/// At most this many slots per call (a 24-hour day of one-second bars is 86,400).
pub const MAX_SESSION_SLOTS: usize = 100_000;

const DAY: i64 = 86_400;

/// One trading window in exchange-local wall-clock seconds after midnight. An `end_seconds` at
/// or before `start_seconds` ends on the following day (a night session crossing midnight), and
/// `86_400` (`"24:00"`) ends at midnight.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SessionWindow {
    pub start_seconds: u32,
    pub end_seconds: u32,
}

/// Which instant of each bar names its slot.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionSlotConvention {
    /// Each bar at its open time, the canonical Aeris and TradingView bar time. An A-share day
    /// (09:30-11:30, 13:00-15:00, one-minute bars) has 240 slots: 09:30..11:29 and 13:00..14:59.
    #[default]
    BarOpen,
    /// Each bar at its close time: 09:31..11:30 and 13:01..15:00 (240 slots).
    BarClose,
    /// [`Self::BarClose`] plus the first window's opening instant as its own slot, for the
    /// opening-auction print: 09:30, 09:31..11:30, 13:01..15:00. These are the 241 one-minute
    /// points 同花顺 and 富途 show for an A-share day.
    BarCloseWithOpen,
}

/// Rejected session-slot input.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionSlotError {
    NoWindows,
    TooManyWindows {
        count: usize,
    },
    /// A start at or after 24:00, an end after 24:00, or a zero-length window.
    InvalidWindow {
        index: usize,
    },
    /// A window that starts before the previous window ends.
    UnorderedWindow {
        index: usize,
    },
    InvalidInterval {
        seconds: u32,
    },
    TooManySlots {
        count: u64,
    },
    /// The trading date or its slots fall outside the supported timestamp range.
    OutOfRange,
}

impl fmt::Display for SessionSlotError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoWindows => write!(f, "a session needs at least one window"),
            Self::TooManyWindows { count } => write!(
                f,
                "{count} session windows; at most {MAX_SESSION_WINDOWS} are supported"
            ),
            Self::InvalidWindow { index } => write!(
                f,
                "session window {index} needs a start before 24:00, an end at or before 24:00, and a nonzero length"
            ),
            Self::UnorderedWindow { index } => write!(
                f,
                "session window {index} starts before the previous window ends"
            ),
            Self::InvalidInterval { seconds } => write!(
                f,
                "bar interval {seconds}s must be between 1 second and 1 day"
            ),
            Self::TooManySlots { count } => write!(
                f,
                "the session has {count} slots; at most {MAX_SESSION_SLOTS} are supported"
            ),
            Self::OutOfRange => write!(
                f,
                "the trading date is outside the supported time range (years 0000..9999)"
            ),
        }
    }
}

impl std::error::Error for SessionSlotError {}

/// Parse `"HH:MM"` or `"HH:MM:SS"` (hour 0-23 with one or two digits) into seconds after
/// midnight. `"24:00"` is accepted only when `end_of_day` is set.
pub fn parse_wall_clock(text: &str, end_of_day: bool) -> Option<u32> {
    let mut fields = text.split(':');
    let hour = fields.next()?;
    let minute = fields.next()?;
    let second = fields.next();
    if fields.next().is_some() || hour.is_empty() || hour.len() > 2 {
        return None;
    }
    let digits = |field: &str| {
        (field.len() == 2 && field.bytes().all(|byte| byte.is_ascii_digit()))
            .then(|| field.parse::<u32>().ok())
            .flatten()
    };
    let hour = hour
        .bytes()
        .all(|byte| byte.is_ascii_digit())
        .then(|| hour.parse::<u32>().ok())
        .flatten()?;
    let minute = digits(minute)?;
    let second = match second {
        Some(second) => digits(second)?,
        None => 0,
    };
    if minute > 59 || second > 59 {
        return None;
    }
    match hour {
        0..=23 => Some(hour * 3_600 + minute * 60 + second),
        24 if end_of_day && minute == 0 && second == 0 => Some(86_400),
        _ => None,
    }
}

/// Parse a strict `"YYYY-MM-DD"` calendar date into days since 1970-01-01.
pub fn parse_iso_date(text: &str) -> Option<i64> {
    let bytes = text.as_bytes();
    if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        return None;
    }
    let number = |range: std::ops::Range<usize>| {
        text.get(range.clone())
            .filter(|field| field.bytes().all(|byte| byte.is_ascii_digit()))
            .and_then(|field| field.parse::<u32>().ok())
    };
    days_from_civil(i64::from(number(0..4)?), number(5..7)?, number(8..10)?)
}

/// UTC seconds of every bar slot of trading date `day` (days since 1970-01-01).
///
/// Each window is placed inside the trading day defined by `time`: with the default midnight
/// session start every window starts on `day` (a window starting before a positive session start
/// runs after midnight, on the next calendar day); with a negative session start a window
/// starting at or after the session-start time of day belongs to the preceding evening (Friday
/// evening for a Monday, as [`ExchangeTime::trading_day`] rolls weekends forward). Windows are
/// converted with the offset in force at each boundary, so they stay on exchange hours across DST,
/// and must be listed chronologically without overlapping. Slots step by elapsed
/// `interval_seconds`; a window whose length is not a multiple of the interval ends with a
/// shorter bar (its close slot is the window end). The work and the result are bounded by
/// [`MAX_SESSION_SLOTS`], checked before anything is allocated.
pub fn session_slot_times(
    day: i64,
    windows: &[SessionWindow],
    interval_seconds: u32,
    time: &ExchangeTime,
    convention: SessionSlotConvention,
) -> Result<Vec<i64>, SessionSlotError> {
    if windows.is_empty() {
        return Err(SessionSlotError::NoWindows);
    }
    if windows.len() > MAX_SESSION_WINDOWS {
        return Err(SessionSlotError::TooManyWindows {
            count: windows.len(),
        });
    }
    if !(1..=86_400).contains(&interval_seconds) {
        return Err(SessionSlotError::InvalidInterval {
            seconds: interval_seconds,
        });
    }
    // Days reachable from the supported timestamp range, with a day of margin for placement.
    if !(MIN_TIMESTAMP / DAY - 1..=MAX_TIMESTAMP / DAY + 1).contains(&day) {
        return Err(SessionSlotError::OutOfRange);
    }
    let session_start = i64::from(time.session_start_seconds());
    let evening_day = time.trading_day_start_local(day).div_euclid(DAY);
    let interval = i64::from(interval_seconds);
    let mut bounds = Vec::with_capacity(windows.len());
    let mut count = u64::from(convention == SessionSlotConvention::BarCloseWithOpen);
    let mut previous_end: Option<i64> = None;
    for (index, window) in windows.iter().enumerate() {
        let (start, end) = (
            i64::from(window.start_seconds),
            i64::from(window.end_seconds),
        );
        if start >= DAY || end > DAY || start == end {
            return Err(SessionSlotError::InvalidWindow { index });
        }
        let base_day = if session_start < 0 {
            if start >= DAY + session_start {
                evening_day
            } else {
                day
            }
        } else if start < session_start {
            day + 1
        } else {
            day
        };
        let local_start = base_day * DAY + start;
        let local_end = if end > start {
            base_day * DAY + end
        } else {
            (base_day + 1) * DAY + end
        };
        let utc_start = time.offsets().to_utc(local_start);
        let utc_end = time.offsets().to_utc(local_end);
        if utc_end <= utc_start {
            return Err(SessionSlotError::InvalidWindow { index });
        }
        if previous_end.is_some_and(|previous| utc_start < previous) {
            return Err(SessionSlotError::UnorderedWindow { index });
        }
        previous_end = Some(utc_end);
        count += (utc_end - utc_start).div_euclid(interval) as u64
            + u64::from((utc_end - utc_start).rem_euclid(interval) != 0);
        if count > MAX_SESSION_SLOTS as u64 {
            return Err(SessionSlotError::TooManySlots { count });
        }
        bounds.push((utc_start, utc_end));
    }
    let first = bounds[0].0;
    let last = bounds[bounds.len() - 1].1;
    if first < MIN_TIMESTAMP || last > MAX_TIMESTAMP {
        return Err(SessionSlotError::OutOfRange);
    }
    let mut slots = Vec::with_capacity(count as usize);
    if convention == SessionSlotConvention::BarCloseWithOpen {
        slots.push(first);
    }
    for (start, end) in bounds {
        let mut open = start;
        while open < end {
            let close = (open + interval).min(end);
            slots.push(match convention {
                SessionSlotConvention::BarOpen => open,
                SessionSlotConvention::BarClose | SessionSlotConvention::BarCloseWithOpen => close,
            });
            open = close;
        }
    }
    Ok(slots)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scale::exchange_time::{UtcOffsetSchedule, UtcOffsetTransition};
    use crate::scale::time_tick_marks::weight_by_time_in;

    const HOUR: i64 = 3_600;

    fn window(start: &str, end: &str) -> SessionWindow {
        SessionWindow {
            start_seconds: parse_wall_clock(start, false).unwrap(),
            end_seconds: parse_wall_clock(end, true).unwrap(),
        }
    }

    fn a_share() -> [SessionWindow; 2] {
        [window("09:30", "11:30"), window("13:00", "15:00")]
    }

    fn shanghai(session_start: i32) -> ExchangeTime {
        ExchangeTime::new(UtcOffsetSchedule::fixed(8 * 3_600).unwrap(), session_start).unwrap()
    }

    fn day(text: &str) -> i64 {
        parse_iso_date(text).unwrap()
    }

    fn local(time: &ExchangeTime, utc: i64) -> String {
        let seconds = time.local_seconds(utc);
        let (year, month, date) =
            crate::scale::time_tick_marks::civil_from_timestamp(seconds.div_euclid(DAY) * DAY);
        let of_day = seconds.rem_euclid(DAY);
        format!(
            "{year:04}-{month:02}-{date:02} {:02}:{:02}",
            of_day / HOUR,
            of_day % HOUR / 60
        )
    }

    #[test]
    fn wall_clock_and_date_parsing_is_strict() {
        assert_eq!(parse_wall_clock("09:30", false), Some(34_200));
        assert_eq!(parse_wall_clock("9:30", false), Some(34_200));
        assert_eq!(parse_wall_clock("21:00:30", false), Some(75_630));
        assert_eq!(parse_wall_clock("24:00", true), Some(86_400));
        for bad in [
            "24:00",
            "9:3",
            "09:60",
            "09:30:60",
            "25:00",
            "",
            "09",
            "a9:30",
            "09:30:00:00",
        ] {
            assert_eq!(parse_wall_clock(bad, false), None, "{bad}");
        }
        assert_eq!(parse_wall_clock("24:01", true), None);
        assert_eq!(parse_iso_date("1970-01-02"), Some(1));
        for bad in [
            "2026-02-30",
            "2026-13-01",
            "2026-1-01",
            "26-01-01",
            "2026/01/01",
        ] {
            assert_eq!(parse_iso_date(bad), None, "{bad}");
        }
    }

    #[test]
    fn a_share_day_has_240_open_slots_and_241_tonghuashun_points() {
        let time = shanghai(0);
        let date = day("2026-09-25");
        let open = session_slot_times(date, &a_share(), 60, &time, SessionSlotConvention::BarOpen)
            .unwrap();
        assert_eq!(open.len(), 240);
        assert_eq!(local(&time, open[0]), "2026-09-25 09:30");
        assert_eq!(local(&time, open[119]), "2026-09-25 11:29");
        assert_eq!(local(&time, open[120]), "2026-09-25 13:00");
        assert_eq!(local(&time, open[239]), "2026-09-25 14:59");

        let close =
            session_slot_times(date, &a_share(), 60, &time, SessionSlotConvention::BarClose)
                .unwrap();
        assert_eq!(close.len(), 240);
        assert_eq!(local(&time, close[0]), "2026-09-25 09:31");
        assert_eq!(local(&time, close[239]), "2026-09-25 15:00");

        let points = session_slot_times(
            date,
            &a_share(),
            60,
            &time,
            SessionSlotConvention::BarCloseWithOpen,
        )
        .unwrap();
        assert_eq!(points.len(), 241);
        assert_eq!(local(&time, points[0]), "2026-09-25 09:30");
        assert_eq!(local(&time, points[1]), "2026-09-25 09:31");
        // The lunch break takes no slot: 11:30 and the first afternoon minute are neighbours.
        assert_eq!(local(&time, points[120]), "2026-09-25 11:30");
        assert_eq!(local(&time, points[121]), "2026-09-25 13:01");
        assert_eq!(local(&time, points[240]), "2026-09-25 15:00");
        assert!(points.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(points.iter().all(|&slot| time.trading_day(slot) == date));
    }

    #[test]
    fn uneven_intervals_end_each_window_with_a_short_bar() {
        let time = shanghai(0);
        let slots = session_slot_times(
            day("2026-09-25"),
            &[window("09:30", "10:00")],
            7 * 60,
            &time,
            SessionSlotConvention::BarClose,
        )
        .unwrap();
        let labels: Vec<_> = slots.iter().map(|&slot| local(&time, slot)).collect();
        assert_eq!(
            labels,
            [
                "2026-09-25 09:37",
                "2026-09-25 09:44",
                "2026-09-25 09:51",
                "2026-09-25 09:58",
                "2026-09-25 10:00"
            ]
        );
    }

    #[test]
    fn half_days_and_bar_sized_intervals_keep_the_session_shape() {
        let time = shanghai(0);
        let date = day("2026-09-30");
        // An A-share half day trades the morning window only: 120 bars, or 121 points with the
        // opening print.
        let morning = [window("09:30", "11:30")];
        for (convention, count, last) in [
            (SessionSlotConvention::BarOpen, 120, "11:29"),
            (SessionSlotConvention::BarCloseWithOpen, 121, "11:30"),
        ] {
            let slots = session_slot_times(date, &morning, 60, &time, convention).unwrap();
            assert_eq!(slots.len(), count);
            assert_eq!(local(&time, slots[count - 1]), format!("2026-09-30 {last}"));
        }
        // A one-day interval gives each window a single (short) bar.
        let daily = session_slot_times(
            date,
            &a_share(),
            86_400,
            &time,
            SessionSlotConvention::BarClose,
        )
        .unwrap();
        let labels: Vec<_> = daily.iter().map(|&slot| local(&time, slot)).collect();
        assert_eq!(labels, ["2026-09-30 11:30", "2026-09-30 15:00"]);
    }

    #[test]
    fn windows_spanning_a_dst_change_step_by_elapsed_time() {
        let ts = |date: &str, hour: i64| day(date) * DAY + hour * HOUR;
        let new_york = ExchangeTime::new(
            UtcOffsetSchedule::new(vec![
                UtcOffsetTransition {
                    from_utc_seconds: ts("2023-11-05", 6),
                    offset_seconds: -5 * 3_600,
                },
                UtcOffsetTransition {
                    from_utc_seconds: ts("2024-03-10", 7),
                    offset_seconds: -4 * 3_600,
                },
                UtcOffsetTransition {
                    from_utc_seconds: ts("2024-11-03", 6),
                    offset_seconds: -5 * 3_600,
                },
            ])
            .unwrap(),
            0,
        )
        .unwrap();
        let overnight = [window("01:00", "04:00")];
        // Spring forward: 02:00-03:00 does not exist, so the window lasts two hours and its
        // slots jump from 01:59 EST to 03:00 EDT.
        let spring = session_slot_times(
            day("2024-03-10"),
            &overnight,
            60,
            &new_york,
            SessionSlotConvention::BarOpen,
        )
        .unwrap();
        assert_eq!(spring.len(), 120);
        assert_eq!(local(&new_york, spring[59]), "2024-03-10 01:59");
        assert_eq!(local(&new_york, spring[60]), "2024-03-10 03:00");
        assert!(spring.windows(2).all(|pair| pair[1] - pair[0] == 60));
        // Fall back: 01:00-02:00 happens twice, so the window lasts four hours of elapsed time.
        let fall = session_slot_times(
            day("2024-11-03"),
            &overnight,
            60,
            &new_york,
            SessionSlotConvention::BarOpen,
        )
        .unwrap();
        assert_eq!(fall.len(), 240);
        assert_eq!(fall[0], ts("2024-11-03", 5));
        assert_eq!(fall[239], ts("2024-11-03", 9) - 60);
        assert!(fall.windows(2).all(|pair| pair[1] - pair[0] == 60));
    }

    #[test]
    fn new_york_session_stays_on_exchange_hours_across_dst() {
        let ts = |date: &str, hour: i64| day(date) * DAY + hour * HOUR;
        let new_york = ExchangeTime::new(
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
            .unwrap(),
            0,
        )
        .unwrap();
        let session = [window("09:30", "16:00")];
        for (date, utc_open) in [
            ("2024-03-08", 14 * HOUR + 1_800),
            ("2024-03-11", 13 * HOUR + 1_800),
        ] {
            let slots = session_slot_times(
                day(date),
                &session,
                60,
                &new_york,
                SessionSlotConvention::BarOpen,
            )
            .unwrap();
            assert_eq!(slots.len(), 390, "{date}");
            assert_eq!(slots[0] - day(date) * DAY, utc_open, "{date}");
            assert_eq!(local(&new_york, slots[389]), format!("{date} 15:59"));
        }
    }

    #[test]
    fn night_sessions_cross_midnight_inside_the_trading_day() {
        let time = shanghai(-3 * 3_600);
        let session = [
            window("21:00", "02:30"),
            window("09:00", "10:15"),
            window("10:30", "11:30"),
            window("13:30", "15:00"),
        ];
        // Tuesday's trading day opens on Monday evening.
        let tuesday = day("2026-09-29");
        let slots =
            session_slot_times(tuesday, &session, 60, &time, SessionSlotConvention::BarOpen)
                .unwrap();
        assert_eq!(slots.len(), 330 + 75 + 60 + 90);
        assert_eq!(local(&time, slots[0]), "2026-09-28 21:00");
        assert_eq!(local(&time, slots[329]), "2026-09-29 02:29");
        assert_eq!(local(&time, slots[330]), "2026-09-29 09:00");
        assert!(slots.iter().all(|&slot| time.trading_day(slot) == tuesday));
        // The night session crosses midnight without a Day mark; the day session carries none
        // either, because the trading day started the evening before.
        assert!(slots
            .windows(2)
            .all(|pair| { (weight_by_time_in(pair[1], pair[0], &time) as u8) < 50 }));
        // Monday's night session trades on Friday evening.
        let monday = day("2026-09-28");
        let slots = session_slot_times(monday, &session, 60, &time, SessionSlotConvention::BarOpen)
            .unwrap();
        assert_eq!(local(&time, slots[0]), "2026-09-25 21:00");
        assert_eq!(local(&time, slots[330]), "2026-09-28 09:00");
        assert!(slots.iter().all(|&slot| time.trading_day(slot) == monday));
    }

    #[test]
    fn a_positive_session_start_places_early_windows_after_midnight() {
        let time = shanghai(6 * 3_600);
        let slots = session_slot_times(
            day("2026-09-25"),
            &[window("08:00", "20:00"), window("22:00", "02:00")],
            HOUR as u32,
            &time,
            SessionSlotConvention::BarOpen,
        )
        .unwrap();
        assert_eq!(slots.len(), 12 + 4);
        assert_eq!(local(&time, slots[15]), "2026-09-26 01:00");
        let slots = session_slot_times(
            day("2026-09-25"),
            &[window("08:00", "12:00"), window("01:00", "03:00")],
            HOUR as u32,
            &time,
            SessionSlotConvention::BarOpen,
        )
        .unwrap();
        assert_eq!(local(&time, slots[4]), "2026-09-26 01:00");
    }

    #[test]
    fn invalid_input_is_rejected_before_allocation() {
        let time = shanghai(0);
        let date = day("2026-09-25");
        let open = SessionSlotConvention::BarOpen;
        assert_eq!(
            session_slot_times(date, &[], 60, &time, open),
            Err(SessionSlotError::NoWindows)
        );
        assert_eq!(
            session_slot_times(date, &a_share(), 0, &time, open),
            Err(SessionSlotError::InvalidInterval { seconds: 0 })
        );
        assert_eq!(
            session_slot_times(date, &a_share(), 86_401, &time, open),
            Err(SessionSlotError::InvalidInterval { seconds: 86_401 })
        );
        assert_eq!(
            session_slot_times(date, &[window("09:30", "09:30")], 60, &time, open),
            Err(SessionSlotError::InvalidWindow { index: 0 })
        );
        assert_eq!(
            session_slot_times(
                date,
                &[window("13:00", "15:00"), window("09:30", "11:30")],
                60,
                &time,
                open
            ),
            Err(SessionSlotError::UnorderedWindow { index: 1 })
        );
        assert_eq!(
            session_slot_times(
                date,
                &[window("09:30", "11:30"), window("11:00", "15:00")],
                60,
                &time,
                open
            ),
            Err(SessionSlotError::UnorderedWindow { index: 1 })
        );
        assert_eq!(
            session_slot_times(
                date,
                &[window("00:00", "24:00"), window("00:00", "24:00")],
                1,
                &time,
                open
            ),
            Err(SessionSlotError::UnorderedWindow { index: 1 })
        );
        let full_day = [window("00:00", "24:00")];
        assert_eq!(
            session_slot_times(date, &full_day, 1, &time, open).map(|slots| slots.len()),
            Ok(86_400)
        );
        assert!(matches!(
            session_slot_times(
                date,
                &[window("00:00", "12:00"), window("12:00", "24:00")],
                1,
                &time,
                SessionSlotConvention::BarCloseWithOpen
            ),
            Ok(slots) if slots.len() == 86_401
        ));
        let many = vec![window("09:30", "11:30"); MAX_SESSION_WINDOWS + 1];
        assert_eq!(
            session_slot_times(date, &many, 60, &time, open),
            Err(SessionSlotError::TooManyWindows {
                count: MAX_SESSION_WINDOWS + 1
            })
        );
        assert_eq!(
            session_slot_times(
                parse_iso_date("9999-12-31").unwrap(),
                &[window("22:00", "02:00")],
                60,
                &ExchangeTime::default(),
                open
            ),
            Err(SessionSlotError::OutOfRange)
        );
    }

    #[test]
    fn slot_count_is_bounded() {
        let time = ExchangeTime::default();
        let date = day("2026-09-25");
        let night = [window("12:00", "11:59")];
        assert!(matches!(
            session_slot_times(date, &night, 1, &time, SessionSlotConvention::BarOpen),
            Ok(slots) if slots.len() == 86_340
        ));
        let long = [window("00:00", "23:59"), window("23:59", "23:58")];
        assert!(matches!(
            session_slot_times(date, &long, 1, &time, SessionSlotConvention::BarOpen),
            Err(SessionSlotError::TooManySlots { .. })
        ));
    }
}
