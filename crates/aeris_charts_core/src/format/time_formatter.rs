//! Time-axis label formatting. Ports of
//! `src/model/horz-scale-behavior-time/default-tick-mark-formatter.ts` (en-US behavior;
//! an `Intl`-backed locale hook comes with the JS API) and the crosshair date-time format
//! (`DateTimeFormatter` with the reference's default `dd MMM 'yy` date format).
//!
//! The date-format pattern language is tokenized here (reference `localization.dateFormat`,
//! formatters/format-date.ts): `dd`/`d` day, `MM`/`M`/`MMM`/`MMMM` month, `yy`/`yyyy` year,
//! with ICU-style `'…'` quoted literals (`''` is an escaped quote). Month names come from a
//! per-locale table (reference `localization.locale`); the headless default is English and hosts
//! inject `Intl`-derived names.

use std::sync::LazyLock;

use crate::scale::exchange_time::ExchangeTime;
use crate::scale::time_tick_marks::{civil_from_timestamp, TickMarkWeight};

const MONTHS_SHORT: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

const MONTHS_LONG: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// the reference's default `localization.dateFormat` (chart-options-defaults.ts:36).
pub const DEFAULT_DATE_FORMAT: &str = "dd MMM 'yy";

/// Per-locale month-name tables backing the `MMM`/`MMMM` date-format tokens and the month
/// tick labels (reference `localization.locale`). The engine stays headless: hosts generate the
/// names (browser builds use `Intl.DateTimeFormat`) and inject them; the default is English.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MonthNames {
    pub short: [String; 12],
    pub long: [String; 12],
}

impl MonthNames {
    /// The built-in English tables (the reference's en-US `toLocaleString` output).
    pub fn english() -> Self {
        Self {
            short: MONTHS_SHORT.map(String::from),
            long: MONTHS_LONG.map(String::from),
        }
    }
}

impl Default for MonthNames {
    fn default() -> Self {
        Self::english()
    }
}

/// The shared English tables (allocated once) for callers that never install a locale.
pub fn english_months() -> &'static MonthNames {
    static ENGLISH: LazyLock<MonthNames> = LazyLock::new(MonthNames::english);
    &ENGLISH
}

/// Port of `TickMarkType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TickMarkType {
    Year,
    Month,
    DayOfMonth,
    Time,
    TimeWithSeconds,
}

/// Port of `weightToTickMarkType` (`horz-scale-behavior-time.ts`).
pub fn weight_to_tick_mark_type(
    weight: u8,
    time_visible: bool,
    seconds_visible: bool,
) -> TickMarkType {
    let w = weight;
    if w <= TickMarkWeight::Second as u8 {
        if time_visible {
            if seconds_visible {
                TickMarkType::TimeWithSeconds
            } else {
                TickMarkType::Time
            }
        } else {
            TickMarkType::DayOfMonth
        }
    } else if w < TickMarkWeight::Day as u8 {
        // minutes and hours
        if time_visible {
            TickMarkType::Time
        } else {
            TickMarkType::DayOfMonth
        }
    } else if w < TickMarkWeight::Month as u8 {
        TickMarkType::DayOfMonth
    } else if w < TickMarkWeight::Year as u8 {
        TickMarkType::Month
    } else {
        TickMarkType::Year
    }
}

fn hms(ts: i64) -> (i64, i64, i64) {
    let secs_of_day = ts.rem_euclid(86_400);
    (
        secs_of_day / 3600,
        (secs_of_day % 3600) / 60,
        secs_of_day % 60,
    )
}

/// Tick label for a UTC timestamp — matches `defaultTickMarkFormatter` output for en-US.
pub fn format_tick_label(ts: i64, mark_type: TickMarkType) -> String {
    format_tick_label_with(ts, mark_type, english_months())
}

/// [`format_tick_label`] with per-locale month names for the `Month` mark (reference
/// `localization.locale` applied to the time axis).
pub fn format_tick_label_with(ts: i64, mark_type: TickMarkType, months: &MonthNames) -> String {
    format_tick_label_in(ts, mark_type, months, &ExchangeTime::UTC)
}

/// [`format_tick_label_with`] in exchange time: Year/Month/DayOfMonth marks name the exchange
/// trading date (the local date shifted by the session start), and time marks show exchange
/// wall-clock time.
pub fn format_tick_label_in(
    ts: i64,
    mark_type: TickMarkType,
    months: &MonthNames,
    time: &ExchangeTime,
) -> String {
    match mark_type {
        TickMarkType::Year | TickMarkType::Month | TickMarkType::DayOfMonth => {
            let (year, month, day) = time.trading_date(ts);
            match mark_type {
                TickMarkType::Year => format!("{year}"),
                TickMarkType::Month => months.short[(month - 1) as usize].clone(),
                _ => format!("{day}"),
            }
        }
        TickMarkType::Time => {
            let (h, m, _) = hms(time.local_seconds(ts));
            format!("{h:02}:{m:02}")
        }
        TickMarkType::TimeWithSeconds => {
            let (h, m, s) = hms(time.local_seconds(ts));
            format!("{h:02}:{m:02}:{s:02}")
        }
    }
}

/// Format a UTC timestamp through an reference `localization.dateFormat` pattern
/// (formatters/format-date.ts). Tokens: `d`/`dd` day of month, `M`/`MM` month number,
/// `MMM` month short name, `MMMM` month long name, `yy` two-digit year, `yyyy` four-digit
/// year. ICU-style `'…'` spans are literal text (`''` is an escaped quote). reference implements
/// the replacement naively, so its default `dd MMM 'yy` (an *unterminated* quote) keeps the
/// apostrophe and still replaces `yy` — an unterminated quote here is likewise literal and
/// tokenization continues after it. Token runs longer than the forms above degrade
/// gracefully: `d`/`M` runs pad to two digits at `dd`/`MM`, name forms cap at `MMMM`, and
/// `y` runs other than 2/4 pass through literally.
pub fn format_date_pattern(ts: i64, pattern: &str, months: &MonthNames) -> String {
    let (year, month, day) = civil_from_timestamp(ts);
    let chars: Vec<char> = pattern.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '\'' {
            if chars.get(i + 1) == Some(&'\'') {
                out.push('\'');
                i += 2;
                continue;
            }
            // Find the closing quote, skipping escaped '' pairs.
            let mut j = i + 1;
            let mut closing = None;
            while j < chars.len() {
                if chars[j] == '\'' {
                    if chars.get(j + 1) == Some(&'\'') {
                        j += 2;
                        continue;
                    }
                    closing = Some(j);
                    break;
                }
                j += 1;
            }
            match closing {
                Some(end) => {
                    // Quoted literal span ('' inside is an escaped quote).
                    let mut k = i + 1;
                    while k < end {
                        if chars[k] == '\'' {
                            out.push('\'');
                            k += 2;
                        } else {
                            out.push(chars[k]);
                            k += 1;
                        }
                    }
                    i = end + 1;
                }
                None => {
                    // Unterminated quote: literal apostrophe (reference naive-replace parity).
                    out.push('\'');
                    i += 1;
                }
            }
            continue;
        }
        // maximal-munch token run of the same letter
        let mut run = 1;
        while chars.get(i + run) == Some(&c) {
            run += 1;
        }
        match c {
            'd' => {
                if run == 1 {
                    out.push_str(&day.to_string());
                } else {
                    out.push_str(&format!("{day:02}"));
                    out.extend(std::iter::repeat_n('d', run - 2));
                }
            }
            'M' => match run {
                1 => out.push_str(&month.to_string()),
                2 => out.push_str(&format!("{month:02}")),
                3 => out.push_str(&months.short[(month - 1) as usize]),
                _ => {
                    out.push_str(&months.long[(month - 1) as usize]);
                    out.extend(std::iter::repeat_n('M', run - 4));
                }
            },
            'y' => match run {
                2 => out.push_str(&format!("{:02}", year.rem_euclid(100))),
                4 => out.push_str(&format!("{year:04}")),
                _ => out.extend(std::iter::repeat_n('y', run)),
            },
            _ => out.extend(std::iter::repeat_n(c, run)),
        }
        i += run;
    }
    out
}

/// Crosshair time label: the reference's default `dd MMM 'yy` date format, plus `HH:MM` when the
/// time is visible (`DateTimeFormatter` joins them with spaces).
pub fn format_crosshair_time(ts: i64, time_visible: bool, seconds_visible: bool) -> String {
    format_crosshair_time_with(
        ts,
        time_visible,
        seconds_visible,
        DEFAULT_DATE_FORMAT,
        english_months(),
    )
}

/// [`format_crosshair_time`] with an explicit `localization.dateFormat` pattern and
/// per-locale month names.
pub fn format_crosshair_time_with(
    ts: i64,
    time_visible: bool,
    seconds_visible: bool,
    date_format: &str,
    months: &MonthNames,
) -> String {
    format_crosshair_time_in(
        ts,
        time_visible,
        seconds_visible,
        date_format,
        months,
        &ExchangeTime::UTC,
    )
}

/// [`format_crosshair_time_with`] in exchange time: the instant's exchange wall-clock date and
/// time. Calendar-date rows keep their own date.
pub fn format_crosshair_time_in(
    ts: i64,
    time_visible: bool,
    seconds_visible: bool,
    date_format: &str,
    months: &MonthNames,
    time: &ExchangeTime,
) -> String {
    let ts = time.local_seconds(ts);
    let date = format_date_pattern(ts, date_format, months);
    if !time_visible {
        return date;
    }
    let (h, m, s) = hms(ts);
    if seconds_visible {
        format!("{date}   {h:02}:{m:02}:{s:02}")
    } else {
        format!("{date}   {h:02}:{m:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 2018-06-25T14:30:45Z
    const TS: i64 = 1_529_937_045;

    #[test]
    fn tick_labels() {
        assert_eq!(format_tick_label(TS, TickMarkType::Year), "2018");
        assert_eq!(format_tick_label(TS, TickMarkType::Month), "Jun");
        assert_eq!(format_tick_label(TS, TickMarkType::DayOfMonth), "25");
        assert_eq!(format_tick_label(TS, TickMarkType::Time), "14:30");
        assert_eq!(
            format_tick_label(TS, TickMarkType::TimeWithSeconds),
            "14:30:45"
        );
    }

    #[test]
    fn time_zone_changes_tick_and_crosshair_calendar_text() {
        use crate::time_zone::ChartTimeZone;
        let new_york = ExchangeTime::new(
            ChartTimeZone::parse("America/New_York")
                .unwrap()
                .offset_schedule()
                .unwrap(),
            0,
        )
        .unwrap();
        // 2026-01-02 04:30 UTC = 2026-01-01 23:30 EST.
        let ts = 1_767_328_200;
        assert_eq!(
            format_tick_label_in(ts, TickMarkType::Time, english_months(), &new_york),
            "23:30"
        );
        assert_eq!(
            format_date_pattern(new_york.local_seconds(ts), "dd MMM yyyy", english_months()),
            "01 Jan 2026"
        );
        assert_eq!(
            format_crosshair_time_in(ts, true, false, "dd MMM yyyy", english_months(), &new_york),
            "01 Jan 2026   23:30"
        );
    }

    #[test]
    fn weight_mapping() {
        // intraday weights show time when timeVisible
        assert_eq!(
            weight_to_tick_mark_type(TickMarkWeight::Hour1 as u8, true, false),
            TickMarkType::Time
        );
        assert_eq!(
            weight_to_tick_mark_type(TickMarkWeight::Hour1 as u8, false, false),
            TickMarkType::DayOfMonth
        );
        assert_eq!(
            weight_to_tick_mark_type(TickMarkWeight::Day as u8, true, true),
            TickMarkType::DayOfMonth
        );
        assert_eq!(
            weight_to_tick_mark_type(TickMarkWeight::Month as u8, true, true),
            TickMarkType::Month
        );
        assert_eq!(
            weight_to_tick_mark_type(TickMarkWeight::Year as u8, false, false),
            TickMarkType::Year
        );
        assert_eq!(
            weight_to_tick_mark_type(TickMarkWeight::Second as u8, true, true),
            TickMarkType::TimeWithSeconds
        );
    }

    #[test]
    fn crosshair_format() {
        assert_eq!(format_crosshair_time(TS, false, false), "25 Jun '18");
        assert_eq!(format_crosshair_time(TS, true, false), "25 Jun '18   14:30");
        assert_eq!(
            format_crosshair_time(TS, true, true),
            "25 Jun '18   14:30:45"
        );
    }

    #[test]
    fn date_pattern_tokens() {
        let months = MonthNames::english();
        // the reference's documented token set (localization-options.ts dateFormat).
        assert_eq!(format_date_pattern(TS, "yyyy-MM-dd", &months), "2018-06-25");
        assert_eq!(format_date_pattern(TS, "dd MMM 'yy", &months), "25 Jun '18");
        assert_eq!(format_date_pattern(TS, "d M yy", &months), "25 6 18");
        assert_eq!(
            format_date_pattern(TS, "MMMM d, yyyy", &months),
            "June 25, 2018"
        );
        // Literal quoting: a closed '…' span passes through untouched, '' escapes a quote.
        assert_eq!(
            format_date_pattern(TS, "dd 'of' MMMM", &months),
            "25 of June"
        );
        assert_eq!(format_date_pattern(TS, "dd 'MM'", &months), "25 MM");
        assert_eq!(format_date_pattern(TS, "dd ''dd''", &months), "25 '25'");
        // Non-token letters are literal; an unterminated quote is a literal apostrophe and
        // tokenization continues (reference naive-replace parity — the default pattern relies on it).
        assert_eq!(format_date_pattern(TS, "yyyy/MM/dd", &months), "2018/06/25");
        assert_eq!(format_date_pattern(TS, "dd 'MM", &months), "25 '06");
        // Degenerate runs: single y is literal, long M runs cap at the long name.
        assert_eq!(format_date_pattern(TS, "y", &months), "y");
        assert_eq!(format_date_pattern(TS, "MMMMM", &months), "JuneM");
    }

    #[test]
    fn exchange_time_labels_use_local_wall_clock_and_trading_dates() {
        use crate::scale::exchange_time::UtcOffsetSchedule;
        let months = MonthNames::english();
        let shanghai = ExchangeTime::new(UtcOffsetSchedule::fixed(8 * 3_600).unwrap(), 0).unwrap();
        // 2024-01-02 01:30 UTC = 09:30 CST.
        let open = 1_704_159_000;
        assert_eq!(
            format_tick_label_in(open, TickMarkType::Time, &months, &shanghai),
            "09:30"
        );
        assert_eq!(
            format_tick_label_in(open, TickMarkType::DayOfMonth, &months, &shanghai),
            "2"
        );
        assert_eq!(
            format_crosshair_time_in(open, true, false, DEFAULT_DATE_FORMAT, &months, &shanghai),
            "02 Jan '24   09:30"
        );
        // 2024-01-01 16:30 UTC is already Jan 2 in Shanghai.
        assert_eq!(
            format_crosshair_time_in(
                open - 9 * 3_600,
                true,
                false,
                DEFAULT_DATE_FORMAT,
                &months,
                &shanghai
            ),
            "02 Jan '24   00:30"
        );
        // A 21:00 night session names the next trading day on its Day mark while the crosshair
        // keeps the wall-clock date.
        let futures =
            ExchangeTime::new(UtcOffsetSchedule::fixed(8 * 3_600).unwrap(), -3 * 3_600).unwrap();
        let night = 1_704_200_400; // 2024-01-02 13:00 UTC = 21:00 CST
        assert_eq!(
            format_tick_label_in(night, TickMarkType::DayOfMonth, &months, &futures),
            "3"
        );
        assert_eq!(
            format_crosshair_time_in(night, true, false, "yyyy-MM-dd", &months, &futures),
            "2024-01-02   21:00"
        );
        // Calendar dates never shift.
        let mut dates =
            ExchangeTime::new(UtcOffsetSchedule::fixed(-5 * 3_600).unwrap(), 0).unwrap();
        dates.set_calendar_dates(true);
        let midnight = 1_704_153_600; // 2024-01-02 00:00 UTC
        assert_eq!(
            format_crosshair_time_in(midnight, false, false, "yyyy-MM-dd", &months, &dates),
            "2024-01-02"
        );
        assert_eq!(
            format_tick_label_in(midnight, TickMarkType::DayOfMonth, &months, &dates),
            "2"
        );
    }

    #[test]
    fn locale_month_names_drive_tokens_and_ticks() {
        let mut months = MonthNames::english();
        months.short[5] = "Jun.".to_string();
        months.long[5] = "Juni".to_string();
        assert_eq!(
            format_date_pattern(TS, "dd MMM yyyy", &months),
            "25 Jun. 2018"
        );
        assert_eq!(format_date_pattern(TS, "MMMM", &months), "Juni");
        assert_eq!(
            format_tick_label_with(TS, TickMarkType::Month, &months),
            "Jun."
        );
        // The default stays English.
        assert_eq!(format_date_pattern(TS, "MMMM", english_months()), "June");
    }
}
