//! Chart display-time-zone support.
//!
//! Canonical chart data remains UTC. This module converts UTC instants to local civil time only
//! for presentation semantics: time-axis tick weighting, labels, crosshair labels, and host clocks.

use chrono::{Datelike, Duration, LocalResult, Offset, TimeZone, Timelike, Utc};
use chrono_tz::Tz;

use crate::scale::exchange_time::{ExchangeTimeError, UtcOffsetSchedule, UtcOffsetTransition};

pub const DEFAULT_TIME_ZONE: &str = "Etc/UTC";

/// End of the span resolved into an offset schedule: 2100-01-01T00:00:00Z (the browser's `Intl`
/// path resolves the same 1970..2100 span).
const SCHEDULE_END: i64 = 4_102_444_800;
/// Sampling step of the schedule scan. No listed zone changes offset twice within 12 hours.
const SCHEDULE_STEP: i64 = 43_200;

/// The exact built-in time-zone identifiers documented by TradingView Advanced Charts.
///
/// Hosts can expose this list directly without maintaining a second copy. The chart display-time
/// parser accepts this parity surface exactly, including TradingView's retained `Asia/Astana`
/// identifier, which is mapped internally to current tzdb rules.
pub const TRADINGVIEW_TIME_ZONES: &[&str] = &[
    "Etc/UTC",
    "Africa/Cairo",
    "Africa/Casablanca",
    "Africa/Johannesburg",
    "Africa/Lagos",
    "Africa/Nairobi",
    "Africa/Tunis",
    "America/Anchorage",
    "America/Argentina/Buenos_Aires",
    "America/Bogota",
    "America/Caracas",
    "America/Chicago",
    "America/El_Salvador",
    "America/Halifax",
    "America/Juneau",
    "America/Lima",
    "America/Los_Angeles",
    "America/Mexico_City",
    "America/New_York",
    "America/Phoenix",
    "America/Santiago",
    "America/Sao_Paulo",
    "America/Toronto",
    "America/Vancouver",
    "Asia/Astana",
    "Asia/Ashkhabad",
    "Asia/Bahrain",
    "Asia/Bangkok",
    "Asia/Chongqing",
    "Asia/Colombo",
    "Asia/Dhaka",
    "Asia/Dubai",
    "Asia/Ho_Chi_Minh",
    "Asia/Hong_Kong",
    "Asia/Jakarta",
    "Asia/Jerusalem",
    "Asia/Karachi",
    "Asia/Kabul",
    "Asia/Kathmandu",
    "Asia/Kolkata",
    "Asia/Kuala_Lumpur",
    "Asia/Kuwait",
    "Asia/Manila",
    "Asia/Muscat",
    "Asia/Nicosia",
    "Asia/Qatar",
    "Asia/Riyadh",
    "Asia/Seoul",
    "Asia/Shanghai",
    "Asia/Singapore",
    "Asia/Taipei",
    "Asia/Tehran",
    "Asia/Tokyo",
    "Asia/Yangon",
    "Atlantic/Azores",
    "Atlantic/Reykjavik",
    "Australia/Adelaide",
    "Australia/Brisbane",
    "Australia/Perth",
    "Australia/Sydney",
    "Europe/Amsterdam",
    "Europe/Athens",
    "Europe/Belgrade",
    "Europe/Berlin",
    "Europe/Bratislava",
    "Europe/Brussels",
    "Europe/Bucharest",
    "Europe/Budapest",
    "Europe/Copenhagen",
    "Europe/Dublin",
    "Europe/Helsinki",
    "Europe/Istanbul",
    "Europe/Lisbon",
    "Europe/Ljubljana",
    "Europe/London",
    "Europe/Luxembourg",
    "Europe/Madrid",
    "Europe/Malta",
    "Europe/Moscow",
    "Europe/Oslo",
    "Europe/Paris",
    "Europe/Prague",
    "Europe/Riga",
    "Europe/Rome",
    "Europe/Sofia",
    "Europe/Stockholm",
    "Europe/Tallinn",
    "Europe/Vienna",
    "Europe/Vilnius",
    "Europe/Warsaw",
    "Europe/Zagreb",
    "Europe/Zurich",
    "Pacific/Auckland",
    "Pacific/Chatham",
    "Pacific/Fakaofo",
    "Pacific/Honolulu",
    "Pacific/Norfolk",
    "US/Mountain",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChartTimeZone {
    id: &'static str,
    inner: Tz,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocalTimeParts {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
    pub offset_seconds: i32,
}

impl Default for ChartTimeZone {
    fn default() -> Self {
        Self {
            id: DEFAULT_TIME_ZONE,
            inner: chrono_tz::Etc::UTC,
        }
    }
}

impl ChartTimeZone {
    fn resolve_local(self, naive: chrono::NaiveDateTime) -> Option<chrono::DateTime<Tz>> {
        match self.inner.from_local_datetime(&naive) {
            LocalResult::Single(value) => Some(value),
            LocalResult::Ambiguous(first, second) => {
                Some(if first.timestamp_millis() <= second.timestamp_millis() {
                    first
                } else {
                    second
                })
            }
            LocalResult::None => {
                // A civil boundary can land in a DST gap. Advance to the first representable
                // local minute rather than dropping a whole day/month/year tick.
                (1..=180).find_map(|minutes| {
                    let candidate = naive.checked_add_signed(Duration::minutes(minutes))?;
                    match self.inner.from_local_datetime(&candidate) {
                        LocalResult::Single(value) => Some(value),
                        LocalResult::Ambiguous(first, second) => {
                            Some(if first.timestamp_millis() <= second.timestamp_millis() {
                                first
                            } else {
                                second
                            })
                        }
                        LocalResult::None => None,
                    }
                })
            }
        }
    }

    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        let id = TRADINGVIEW_TIME_ZONES
            .iter()
            .copied()
            .find(|candidate| *candidate == value)?;
        // TradingView retains this historical identifier while current tzdb releases use
        // Asia/Almaty for the same Kazakhstan civil-time rules.
        let canonical = if id == "Asia/Astana" {
            "Asia/Almaty"
        } else {
            id
        };
        canonical.parse::<Tz>().ok().map(|inner| Self { id, inner })
    }

    #[must_use]
    pub fn id(self) -> &'static str {
        self.id
    }

    #[must_use]
    pub fn is_tradingview_supported(value: &str) -> bool {
        TRADINGVIEW_TIME_ZONES.contains(&value)
    }

    #[must_use]
    pub fn local_parts(self, utc_seconds: i64) -> Option<LocalTimeParts> {
        let local = self.inner.timestamp_opt(utc_seconds, 0).single()?;
        Some(LocalTimeParts {
            year: local.year(),
            month: local.month(),
            day: local.day(),
            hour: local.hour(),
            minute: local.minute(),
            second: local.second(),
            offset_seconds: local.offset().fix().local_minus_utc(),
        })
    }

    #[must_use]
    pub fn local_epoch_seconds(self, utc_seconds: i64) -> i64 {
        self.local_parts(utc_seconds).map_or(utc_seconds, |parts| {
            utc_seconds.saturating_add(i64::from(parts.offset_seconds))
        })
    }

    /// Resolve a pseudo-epoch millisecond value whose civil fields represent local wall time into
    /// the corresponding UTC instant. Ambiguous fall-back times choose the earlier occurrence;
    /// nonexistent spring-forward times advance to the first representable local minute.
    #[must_use]
    pub fn utc_millis_from_local_epoch_millis(self, local_millis: i64) -> Option<i64> {
        if self == Self::default() {
            return Some(local_millis);
        }
        let naive = chrono::DateTime::<Utc>::from_timestamp_millis(local_millis)?.naive_utc();
        Some(self.resolve_local(naive)?.timestamp_millis())
    }

    #[must_use]
    pub fn abbreviation(self, utc_seconds: i64) -> String {
        // The offset's own display: the database abbreviation ("EDT"), or its numeric form
        // ("+04") for zones that have none. Identical to strftime's `%Z` without linking it.
        self.inner
            .timestamp_opt(utc_seconds, 0)
            .single()
            .map_or_else(|| "UTC".to_string(), |local| local.offset().to_string())
    }

    fn offset_seconds_at(self, utc_seconds: i64) -> i32 {
        self.inner
            .timestamp_opt(utc_seconds, 0)
            .single()
            .map_or(0, |local| local.offset().fix().local_minus_utc())
    }

    /// This zone's UTC-offset schedule over 1970..2100, for the one clock the chart runs on
    /// (`ExchangeTime`): the offset in force at 1970-01-01 also applies before it, and every later
    /// change is one transition. Resolving a name is a one-time cost (2.6–5.9 ms natively), paid
    /// when a zone is selected, never per frame. The embedded database carries DST through 2099, so
    /// the schedule is complete up to its 2100 end; the offset of its last transition holds beyond.
    pub fn offset_schedule(self) -> Result<UtcOffsetSchedule, ExchangeTimeError> {
        let first = self.offset_seconds_at(0);
        let mut transitions = vec![UtcOffsetTransition {
            from_utc_seconds: 0,
            offset_seconds: first,
        }];
        let (mut previous, mut low) = (first, 0_i64);
        while low < SCHEDULE_END {
            let high = (low + SCHEDULE_STEP).min(SCHEDULE_END);
            if self.offset_seconds_at(high) == previous {
                low = high;
                continue;
            }
            // The offset changed inside (low, high]: bisect to the first second that has the new one.
            let (mut before, mut after) = (low, high);
            while after - before > 1 {
                let middle = before + (after - before) / 2;
                if self.offset_seconds_at(middle) == previous {
                    before = middle;
                } else {
                    after = middle;
                }
            }
            previous = self.offset_seconds_at(after);
            transitions.push(UtcOffsetTransition {
                from_utc_seconds: after,
                offset_seconds: previous,
            });
            low = after;
        }
        UtcOffsetSchedule::new(transitions)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tradingview_zone_resolves_to_a_schedule_that_matches_the_database() {
        use crate::scale::exchange_time::MAX_UTC_OFFSET_TRANSITIONS;
        let mut busiest = 0;
        for id in TRADINGVIEW_TIME_ZONES {
            let zone = ChartTimeZone::parse(id).unwrap();
            let schedule = zone.offset_schedule().expect(id);
            busiest = busiest.max(schedule.transitions().len());
            assert!(
                schedule.transitions().len() <= MAX_UTC_OFFSET_TRANSITIONS,
                "{id}"
            );
            let expect = |utc: i64| {
                assert_eq!(
                    schedule.offset_at(utc),
                    zone.local_parts(utc).unwrap().offset_seconds,
                    "{id} at {utc}"
                );
            };
            // 1970..2100 on a coprime stride, then one second either side of every transition.
            let mut utc = 0;
            while utc < SCHEDULE_END {
                expect(utc);
                utc += 86_399 * 31 + 7;
            }
            for transition in schedule.transitions().iter().skip(1) {
                for utc in [-1, 0, 1] {
                    expect(transition.from_utc_seconds + utc);
                }
            }
        }
        assert!(
            busiest > 100,
            "a DST zone resolves its transitions: {busiest}"
        );
    }

    #[test]
    fn named_zone_schedules_match_known_offsets() {
        let new_york = ChartTimeZone::parse("America/New_York")
            .unwrap()
            .offset_schedule()
            .unwrap();
        // 2024-01-15 12:00 UTC is EST, 2024-07-15 12:00 UTC is EDT.
        assert_eq!(new_york.offset_at(1_705_320_000), -5 * 3_600);
        assert_eq!(new_york.offset_at(1_721_044_800), -4 * 3_600);
        assert!(ChartTimeZone::default().offset_schedule().unwrap().is_utc());
        let shanghai = ChartTimeZone::parse("Asia/Shanghai")
            .unwrap()
            .offset_schedule()
            .unwrap();
        assert_eq!(shanghai.offset_at(1_705_320_000), 8 * 3_600);
        assert_eq!(shanghai.offset_at(0), 8 * 3_600);
        // Asia/Astana keeps its identifier and resolves through Asia/Almaty.
        assert!(
            ChartTimeZone::parse("Asia/Astana")
                .unwrap()
                .offset_schedule()
                .is_ok()
        );
    }

    #[test]
    fn every_tradingview_zone_parses() {
        for zone in TRADINGVIEW_TIME_ZONES {
            assert!(
                ChartTimeZone::parse(zone).is_some(),
                "unsupported zone: {zone}"
            );
        }
    }

    #[test]
    fn new_york_observes_dst() {
        let zone = ChartTimeZone::parse("America/New_York").unwrap();
        // 2026-01-15 12:00 UTC and 2026-07-15 12:00 UTC.
        let winter = zone.local_parts(1_768_478_400).unwrap();
        let summer = zone.local_parts(1_784_116_800).unwrap();
        assert_eq!(winter.offset_seconds, -18_000);
        assert_eq!(summer.offset_seconds, -14_400);
    }

    #[test]
    fn abbreviations_are_database_names_or_numeric_offsets() {
        let abbreviation = |id: &str, utc| ChartTimeZone::parse(id).unwrap().abbreviation(utc);
        assert_eq!(abbreviation("America/New_York", 1_784_116_800), "EDT");
        assert_eq!(abbreviation("America/New_York", 1_768_478_400), "EST");
        assert_eq!(abbreviation("Asia/Dubai", 1_784_116_800), "+04");
        assert_eq!(abbreviation("America/Sao_Paulo", 1_784_116_800), "-03");
        assert_eq!(abbreviation("Etc/UTC", 1_784_116_800), "UTC");
    }
}
