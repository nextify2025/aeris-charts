//! Browser boundary for session slot generation. The package resolves the exchange time zone to
//! an explicit schedule; the engine owns validation and the slot arithmetic.

use aeris_charts_engine::{
    parse_iso_date, parse_wall_clock, session_slot_times, ExchangeTime, SessionSlotConvention,
    SessionWindow, UtcOffsetSchedule, UtcOffsetTransition,
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

/// UTC seconds of every bar slot described by a JSON request, or the rejection reason.
pub(crate) fn session_slot_times_json(request: &str) -> Result<Vec<i64>, String> {
    let request: SessionSlotRequest =
        serde_json::from_str(request).map_err(|error| format!("invalid session: {error}"))?;
    let day = parse_iso_date(&request.date)
        .ok_or_else(|| format!("date {:?} must be a YYYY-MM-DD calendar date", request.date))?;
    let windows = request
        .windows
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
        .collect::<Result<Vec<_>, _>>()?;
    let offsets = UtcOffsetSchedule::new(request.time_zone).map_err(|error| error.to_string())?;
    let time = ExchangeTime::new(offsets, request.session_start).map_err(|e| e.to_string())?;
    session_slot_times(
        day,
        &windows,
        request.interval_seconds,
        &time,
        request.convention,
    )
    .map_err(|error| error.to_string())
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
}

#[cfg(test)]
mod tests {
    use super::session_slot_times_json;

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
}
