//! Browser boundary for exchange-session requests: session slot generation, session-anchored
//! trade-stream bars, and resampling boundaries/configuration. The package resolves the exchange
//! time zone to an explicit schedule; the engine owns validation and the session arithmetic.

use aeris_charts_engine::{
    ExchangeTime, OutOfSessionPolicy, ResampleBoundary, ResampleOptions, ResampleSpan,
    SessionSlotConvention, SessionWindow, TradeSessionOptions, UtcOffsetSchedule,
    UtcOffsetTransition, parse_iso_date, parse_wall_clock, resample_boundaries, session_slot_times,
};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SessionSlotRequest {
    date: String,
    windows: Vec<(String, String)>,
    interval_seconds: u32,
    /// Explicit offset schedule; empty is UTC.
    #[serde(default)]
    time_zone: Vec<UtcOffsetTransition>,
    #[serde(default)]
    session_start: i32,
    #[serde(default)]
    convention: SessionSlotConvention,
}

fn parse_date(text: &str) -> Result<i64, String> {
    parse_iso_date(text).ok_or_else(|| format!("date {text:?} must be a YYYY-MM-DD calendar date"))
}

fn parse_windows(windows: &[(String, String)]) -> Result<Vec<SessionWindow>, String> {
    windows
        .iter()
        .enumerate()
        .map(|(index, (start, end))| {
            match (parse_wall_clock(start, false), parse_wall_clock(end, true)) {
                (Some(start_seconds), Some(end_seconds)) => Ok(SessionWindow {
                    start_seconds,
                    end_seconds,
                }),
                _ => Err(format!(
                    "session window {index} needs \"HH:MM\" or \"HH:MM:SS\" times (end may be \"24:00\")"
                )),
            }
        })
        .collect()
}

fn exchange_time(
    time_zone: Vec<UtcOffsetTransition>,
    session_start: i32,
) -> Result<ExchangeTime, String> {
    let offsets = UtcOffsetSchedule::new(time_zone).map_err(|error| error.to_string())?;
    ExchangeTime::new(offsets, session_start).map_err(|error| error.to_string())
}

/// UTC seconds of every bar slot described by a JSON request, or the rejection reason.
pub(crate) fn session_slot_times_json(request: &str) -> Result<Vec<i64>, String> {
    let request: SessionSlotRequest =
        serde_json::from_str(request).map_err(|error| format!("invalid session: {error}"))?;
    let day = parse_date(&request.date)?;
    let windows = parse_windows(&request.windows)?;
    let time = exchange_time(request.time_zone, request.session_start)?;
    session_slot_times(
        day,
        &windows,
        request.interval_seconds,
        &time,
        request.convention,
    )
    .map_err(|error| error.to_string())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResampleBoundaryRequest {
    dates: Vec<String>,
    windows: Vec<(String, String)>,
    #[serde(default)]
    time_zone: Vec<UtcOffsetTransition>,
    #[serde(default)]
    session_start: i32,
    #[serde(default)]
    span: ResampleSpan,
}

/// Resampling boundaries for a JSON request of trading dates and session windows.
pub(crate) fn resample_boundaries_json(request: &str) -> Result<Vec<ResampleBoundary>, String> {
    let request: ResampleBoundaryRequest = serde_json::from_str(request)
        .map_err(|error| format!("invalid resample sessions: {error}"))?;
    let days = request
        .dates
        .iter()
        .map(|date| parse_date(date))
        .collect::<Result<Vec<_>, _>>()?;
    let windows = parse_windows(&request.windows)?;
    let time = exchange_time(request.time_zone, request.session_start)?;
    resample_boundaries(&days, &windows, &time, request.span).map_err(|error| error.to_string())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TradeSessionRequest {
    windows: Vec<(String, String)>,
    #[serde(default)]
    outside: OutOfSessionPolicy,
}

/// Session anchoring for a trade stream (`null` restores the plain anchor grid). The windows are
/// placed in the chart's own exchange time by the engine.
pub(crate) fn trade_sessions_json(request: &str) -> Result<Option<TradeSessionOptions>, String> {
    let request: Option<TradeSessionRequest> = serde_json::from_str(request)
        .map_err(|error| format!("invalid trade sessions: {error}"))?;
    request
        .map(|request| {
            Ok(TradeSessionOptions {
                windows: parse_windows(&request.windows)?,
                outside: request.outside,
            })
        })
        .transpose()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BoundaryRow {
    start_time: i64,
    end_time: i64,
    session_id: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResampleSeriesRequest {
    source: u32,
    #[serde(default)]
    volume_source: Option<u32>,
    #[serde(default)]
    volume_target: Option<u32>,
    interval_seconds: u32,
    boundaries: Vec<BoundaryRow>,
}

/// A parsed `configure_resampled_series` request: source, volume source, volume target, and
/// options (the target is the call's series).
pub(crate) type ResampleSeriesConfig = (u32, Option<u32>, Option<u32>, ResampleOptions);

pub(crate) fn resample_series_json(request: &str) -> Result<ResampleSeriesConfig, String> {
    let request: ResampleSeriesRequest = serde_json::from_str(request)
        .map_err(|error| format!("invalid resample options: {error}"))?;
    Ok((
        request.source,
        request.volume_source,
        request.volume_target,
        ResampleOptions {
            interval_seconds: request.interval_seconds,
            boundaries: request
                .boundaries
                .into_iter()
                .map(|row| ResampleBoundary {
                    start_time: row.start_time,
                    end_time: row.end_time,
                    session_id: row.session_id,
                })
                .collect(),
        },
    ))
}

#[cfg(target_arch = "wasm32")]
mod bindings {
    use wasm_bindgen::prelude::*;

    /// UTC seconds of every bar slot of one trading date's session windows (see the engine's
    /// `session_slot_times`). Throws the rejection reason as a string.
    #[wasm_bindgen]
    pub fn session_slot_times(request_json: &str) -> Result<Vec<f64>, JsValue> {
        super::session_slot_times_json(request_json)
            .map(|slots| slots.into_iter().map(|slot| slot as f64).collect())
            .map_err(|reason| JsValue::from_str(&reason))
    }

    /// Resampling boundaries as flat `[start_time, end_time, session_id]` triples (see the
    /// engine's `resample_boundaries`). Throws the rejection reason as a string.
    #[wasm_bindgen]
    pub fn resample_boundaries(request_json: &str) -> Result<Vec<f64>, JsValue> {
        super::resample_boundaries_json(request_json)
            .map(|boundaries| {
                boundaries
                    .into_iter()
                    .flat_map(|boundary| {
                        [
                            boundary.start_time as f64,
                            boundary.end_time as f64,
                            boundary.session_id as f64,
                        ]
                    })
                    .collect()
            })
            .map_err(|reason| JsValue::from_str(&reason))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_requests_validate_at_the_boundary() {
        let slots = session_slot_times_json(
            r#"{"date":"2026-09-25","windows":[["09:30","11:30"],["13:00","15:00"]],
                "interval_seconds":60,"time_zone":[{"from_utc_seconds":0,"offset_seconds":28800}],
                "convention":"bar_close_with_open"}"#,
        )
        .unwrap();
        assert_eq!(slots.len(), 241);
        // 09:30 Shanghai is 01:30 UTC.
        assert_eq!(slots[0].rem_euclid(86_400), 3_600 + 1_800);
        for (request, reason) in [
            (
                r#"{"date":"2026-02-30","windows":[["09:30","11:30"]],"interval_seconds":60}"#,
                "YYYY-MM-DD",
            ),
            (
                r#"{"date":"2026-09-25","windows":[["9h30","11:30"]],"interval_seconds":60}"#,
                "HH:MM",
            ),
            (
                r#"{"date":"2026-09-25","windows":[["09:30","11:30"]],"interval_seconds":0}"#,
                "interval",
            ),
            (
                r#"{"date":"2026-09-25","windows":[["09:30","11:30"]],"interval_seconds":60,"session_start":90000}"#,
                "session start",
            ),
            (
                r#"{"date":"2026-09-25","windows":[["09:30","11:30"]],"interval_seconds":60,"zone":"UTC"}"#,
                "unknown field",
            ),
        ] {
            let error = session_slot_times_json(request).unwrap_err();
            assert!(error.contains(reason), "{error}");
        }
    }

    #[test]
    fn a_request_session_start_is_its_own_and_defaults_to_midnight() {
        // The request's `session_start` is independent of any chart: omitted, it is 0, so a
        // Globex-style 17:00-16:00 session keyed by its evening date opens that evening.
        const CST: &str = r#""time_zone":[{"from_utc_seconds":0,"offset_seconds":-21600}]"#;
        let sunday = session_slot_times_json(&format!(
            r#"{{"date":"2024-01-07","windows":[["17:00","16:00"]],"interval_seconds":60,{CST}}}"#
        ))
        .unwrap();
        assert_eq!(sunday.len(), 23 * 60);
        assert_eq!(sunday[0], 1_704_668_400, "Sunday 2024-01-07 17:00 CST");
        // An explicit -7 h start places Monday's date on the preceding Friday evening instead.
        let monday = session_slot_times_json(&format!(
            r#"{{"date":"2024-01-08","windows":[["17:00","16:00"]],"interval_seconds":60,
                "session_start":-25200,{CST}}}"#
        ))
        .unwrap();
        assert_eq!(monday[0], 1_704_495_600, "Friday 2024-01-05 17:00 CST");

        // The same holds for a China futures night window: without `session_start: -10800` it is
        // placed on the calendar date (Monday 21:00), not on Friday evening.
        const CHINA: &str = r#""time_zone":[{"from_utc_seconds":0,"offset_seconds":28800}]"#;
        let night = |session_start: &str| {
            session_slot_times_json(&format!(
                r#"{{"date":"2024-01-08","windows":[["21:00","02:30"]],"interval_seconds":60,
                    {session_start}{CHINA}}}"#
            ))
            .unwrap()[0]
        };
        let monday_21_00_utc = 1_704_672_000 + 13 * 3_600;
        assert_eq!(night(""), monday_21_00_utc);
        assert_eq!(
            night(r#""session_start":-10800,"#),
            monday_21_00_utc - 3 * 86_400
        );

        // resample_boundaries takes its own `session_start` with the same default.
        let boundary_start = |session_start: &str| {
            resample_boundaries_json(&format!(
                r#"{{"dates":["2024-01-08"],"windows":[["17:00","16:00"]],{session_start}{CST}}}"#
            ))
            .unwrap()[0]
                .start_time
        };
        assert_eq!(boundary_start(""), 1_704_754_800, "Monday 17:00 CST");
        assert_eq!(boundary_start(r#""session_start":-25200,"#), 1_704_495_600);
    }

    #[test]
    fn resample_boundary_requests_place_sessions_per_date() {
        let boundaries = resample_boundaries_json(
            r#"{"dates":["2026-09-24","2026-09-25"],"windows":[["09:30","11:30"],["13:00","15:00"]],
                "time_zone":[{"from_utc_seconds":0,"offset_seconds":28800}]}"#,
        )
        .unwrap();
        assert_eq!(boundaries.len(), 4);
        assert_eq!(boundaries[0].start_time.rem_euclid(86_400), 3_600 + 1_800);
        assert_eq!(boundaries[1].end_time.rem_euclid(86_400), 7 * 3_600);
        assert_eq!(boundaries[3].session_id, 20_260_925);
        let daily = resample_boundaries_json(
            r#"{"dates":["2026-09-25"],"windows":[["09:30","11:30"],["13:00","15:00"]],"span":"day"}"#,
        )
        .unwrap();
        assert_eq!(daily.len(), 1);
        assert_eq!(daily[0].end_time - daily[0].start_time, 5 * 3_600 + 1_800);
        for (request, reason) in [
            (
                r#"{"dates":["2026-9-25"],"windows":[["09:30","11:30"]]}"#,
                "YYYY-MM-DD",
            ),
            (
                r#"{"dates":["2026-09-25","2026-09-24"],"windows":[["09:30","11:30"]]}"#,
                "boundaries",
            ),
            (
                r#"{"dates":["2026-09-25"],"windows":[["09:30","11:30"]],"span":"week"}"#,
                "unknown variant",
            ),
            (
                r#"{"dates":["2026-09-25"],"windows":[["13:00","15:00"],["09:30","11:30"]]}"#,
                "starts before the previous window ends",
            ),
        ] {
            let error = resample_boundaries_json(request).unwrap_err();
            assert!(error.contains(reason), "{error}");
        }
    }

    #[test]
    fn trade_session_and_resample_series_requests_are_strict() {
        assert_eq!(trade_sessions_json("null").unwrap(), None);
        let sessions = trade_sessions_json(
            r#"{"windows":[["09:30","11:30"],["13:00","15:00"]],"outside":"exclude"}"#,
        )
        .unwrap()
        .unwrap();
        assert_eq!(sessions.windows.len(), 2);
        assert_eq!(sessions.outside, OutOfSessionPolicy::Exclude);
        assert_eq!(
            trade_sessions_json(r#"{"windows":[["09:30","11:30"]]}"#)
                .unwrap()
                .unwrap()
                .outside,
            OutOfSessionPolicy::Fold
        );
        assert!(
            trade_sessions_json(r#"{"windows":[["09:30","11:30"]],"outside":"drop"}"#)
                .unwrap_err()
                .contains("unknown variant")
        );
        let (source, volume_source, volume_target, options) = resample_series_json(
            r#"{"source":1,"volume_source":2,"volume_target":4,"interval_seconds":300,
                "boundaries":[{"start_time":0,"end_time":600,"session_id":20260925}]}"#,
        )
        .unwrap();
        assert_eq!(
            (source, volume_source, volume_target),
            (1, Some(2), Some(4))
        );
        assert_eq!(options.interval_seconds, 300);
        assert_eq!(options.boundaries[0].end_time, 600);
        assert!(resample_series_json(
            r#"{"source":1,"interval_seconds":300,"boundaries":[{"start_time":0.5,"end_time":600,"session_id":1}]}"#
        )
        .is_err());
    }
}
