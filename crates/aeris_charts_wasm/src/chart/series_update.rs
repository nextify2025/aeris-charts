//! Partial merges and sequence-guarded streaming updates (`series.merge`, the `sequence` option of
//! `update`/`update_typed`/`set_data`). The engine owns the merge and guard semantics; this layer
//! only decodes the JS boundary encoding and reports ingestion diagnostics.

use super::*;
use aeris_charts_core::model::data_validation::{sanitize_ohlc_owned, SanitizedOhlc};
use aeris_charts_engine::{SeriesBarPatch, SeriesUpdateOutcome, SeriesUpdateRejection};

/// JS numbers are exact only up to 2^53 - 1, so a sequence beyond that cannot be monotonic.
const MAX_SAFE_SEQUENCE: f64 = 9_007_199_254_740_991.0;

/// `NaN` encodes "no sequence". Anything else must be a non-negative safe integer.
fn decode_sequence(sequence: f64) -> Result<Option<u64>, String> {
    if sequence.is_nan() {
        return Ok(None);
    }
    if sequence.fract() != 0.0 || !(0.0..=MAX_SAFE_SEQUENCE).contains(&sequence) {
        return Err(rejected_diagnostics_json(
            "sequence must be a non-negative safe integer",
        ));
    }
    Ok(Some(sequence as u64))
}

/// `None` for an applied update; otherwise the rejected ingestion record. Stale deliveries carry
/// a machine-readable `code` and the last applied sequence so hosts can resync without parsing.
fn outcome_json(outcome: SeriesUpdateOutcome, sequence: Option<u64>) -> Option<String> {
    let (reason, code) = match outcome {
        SeriesUpdateOutcome::Applied => return None,
        SeriesUpdateOutcome::StaleSequence { last_applied } => {
            let mut json = serde_json::json!({
                "status": "rejected",
                "accepted": 0,
                "dropped_invalid": 0,
                "dropped_non_finite": 0,
                "dropped_out_of_range": 0,
                "deduplicated": 0,
                "reordered": false,
                "semantic_anomalies": 0,
                "code": "stale_sequence",
                "last_sequence": last_applied,
            });
            json["reason"] = format!(
                "stale sequence {} is not newer than the last applied {last_applied}",
                sequence.unwrap_or(last_applied)
            )
            .into();
            return Some(json.to_string());
        }
        SeriesUpdateOutcome::Rejected(SeriesUpdateRejection::UnknownSeries) => {
            ("unknown or stale series id", None)
        }
        SeriesUpdateOutcome::Rejected(SeriesUpdateRejection::UnsupportedSeries) => (
            "custom, advanced, and footprint series, and engine-derived synthetic or resampled \
             bars, do not accept host OHLC writes",
            None,
        ),
        SeriesUpdateOutcome::Rejected(SeriesUpdateRejection::EmptyPatch) => (
            "merge needs a price field: open/high/low/close on candlestick and bar series, \
             value (or close) on line, area, baseline, and histogram series; merge volume into \
             its own series",
            Some("empty_merge"),
        ),
        SeriesUpdateOutcome::Rejected(SeriesUpdateRejection::InvalidData) => {
            ("invalid timestamp or non-finite/out-of-range values", None)
        }
    };
    let mut json: serde_json::Value =
        serde_json::from_str(&rejected_diagnostics_json(reason)).unwrap_or_default();
    if let Some(code) = code {
        json["code"] = code.into();
    }
    Some(json.to_string())
}

/// Sanitize a typed OHLC batch at the boundary (sort, dedupe last-wins, drop invalid rows) and warn
/// about any repair, shared by the plain and sequence-guarded typed updates. `Err` carries the
/// rejection diagnostics when the batch cannot be used; `Ok` carries the repair diagnostics.
pub(super) fn sanitize_typed_batch(
    times: &Float64Array,
    open: &Float64Array,
    high: &Float64Array,
    low: &Float64Array,
    close: &Float64Array,
) -> Result<(SanitizedOhlc, Option<String>), Option<String>> {
    let s = match sanitize_ohlc_owned(
        times.to_vec(),
        open.to_vec(),
        high.to_vec(),
        low.to_vec(),
        close.to_vec(),
    ) {
        Ok(s) => s,
        Err(e) => {
            web_sys::console::warn_1(&format!("aeris_charts: update_typed rejected — {e}").into());
            return Err(Some(rejected_validation_diagnostics_json(e)));
        }
    };
    if !s.report.is_clean() {
        web_sys::console::warn_1(
            &format!(
                "aeris_charts: update_typed sanitized batch — accepted {}, dropped {} invalid, {} duplicate{}",
                s.report.accepted,
                s.report.dropped_invalid,
                s.report.dropped_duplicate,
                if s.report.reordered { ", reordered" } else { "" }
            )
            .into(),
        );
    }
    let diagnostics = validation_diagnostics_json(&s.report);
    if s.report.accepted == 0 && s.report.dropped_invalid > 0 {
        return Err(diagnostics);
    }
    Ok((s, diagnostics))
}

impl ChartInner {
    /// Field-wise merge into the bar at `time`. `NaN` channels are absent and keep the existing
    /// bar's values; `sequence` is `NaN` for an unguarded merge.
    #[allow(clippy::too_many_arguments)] // the OHLC channels, three color slots, and the guard
    pub fn merge_series_bar(
        &mut self,
        id: u32,
        time: f64,
        open: f64,
        high: f64,
        low: f64,
        close: f64,
        body: Option<u32>,
        wick: Option<u32>,
        border: Option<u32>,
        sequence: f64,
    ) -> Option<String> {
        let sequence = match decode_sequence(sequence) {
            Ok(sequence) => sequence,
            Err(json) => return Some(json),
        };
        let present = |value: f64| (!value.is_nan()).then_some(value);
        let patch = SeriesBarPatch {
            open: present(open),
            high: present(high),
            low: present(low),
            close: present(close),
            colors: [body, wick, border],
        };
        let outcome = self
            .engine
            .merge_series_bar(id as SeriesId, time, patch, sequence);
        outcome_json(outcome, sequence)
    }

    /// `update_series_bar_styled` behind the sequence guard.
    #[allow(clippy::too_many_arguments)] // mirrors update_series_bar_styled plus the guard
    pub fn update_series_bar_sequenced(
        &mut self,
        id: u32,
        time: f64,
        open: f64,
        high: f64,
        low: f64,
        close: f64,
        body: Option<u32>,
        wick: Option<u32>,
        border: Option<u32>,
        sequence: f64,
    ) -> Option<String> {
        let sequence = match decode_sequence(sequence) {
            Ok(sequence) => sequence,
            Err(json) => return Some(json),
        };
        let outcome = self.engine.update_series_bar_sequenced(
            id as SeriesId,
            time,
            [open, high, low, close],
            [body, wick, border],
            sequence,
        );
        outcome_json(outcome, sequence)
    }

    /// `update_series_bars_typed` behind the sequence guard. A stale batch is rejected before
    /// its columns are copied or repaired.
    #[allow(clippy::too_many_arguments)] // the typed OHLC columns plus the guard
    pub fn update_series_bars_typed_sequenced(
        &mut self,
        id: u32,
        times: &Float64Array,
        open: &Float64Array,
        high: &Float64Array,
        low: &Float64Array,
        close: &Float64Array,
        sequence: f64,
    ) -> Option<String> {
        let sequence = match decode_sequence(sequence) {
            Ok(sequence) => sequence,
            Err(json) => return Some(json),
        };
        if let (Some(sequence), Some(last_applied)) =
            (sequence, self.engine.series_update_sequence(id as SeriesId))
        {
            if sequence <= last_applied {
                return outcome_json(
                    SeriesUpdateOutcome::StaleSequence { last_applied },
                    Some(sequence),
                );
            }
        }
        let (s, diagnostics) = match sanitize_typed_batch(times, open, high, low, close) {
            Ok(batch) => batch,
            Err(rejected) => return rejected,
        };
        let outcome = self.engine.update_series_bars_sanitized_sequenced(
            id as SeriesId,
            s.times,
            s.open,
            s.high,
            s.low,
            s.close,
            sequence,
        );
        outcome_json(outcome, sequence).or(diagnostics)
    }

    /// Typed partial merge (`series.merge_typed`): row `i` merges like `merge_series_bar` with
    /// the `NaN` entries (or an omitted column) absent, applied in input order with one time and
    /// indicator synchronization. Rejected atomically; `None` when every row applied.
    #[allow(clippy::too_many_arguments)] // the optional OHLC columns plus the guard
    pub fn merge_series_bars_typed(
        &mut self,
        id: u32,
        times: &Float64Array,
        open: Option<Float64Array>,
        high: Option<Float64Array>,
        low: Option<Float64Array>,
        close: Option<Float64Array>,
        sequence: f64,
    ) -> Option<String> {
        let sequence = match decode_sequence(sequence) {
            Ok(sequence) => sequence,
            Err(json) => return Some(json),
        };
        let times = times.to_vec();
        let columns = [open, high, low, close].map(|column| column.map(|column| column.to_vec()));
        if let Some(column) = columns
            .iter()
            .flatten()
            .find(|column| column.len() != times.len())
        {
            return Some(rejected_diagnostics_json(format!(
                "merge_typed columns must match times: {} times, a column of {}",
                times.len(),
                column.len()
            )));
        }
        let channel = |column: &Option<Vec<f64>>, row: usize| {
            column
                .as_ref()
                .map(|values| values[row])
                .filter(|value| !value.is_nan())
        };
        let rows = times
            .iter()
            .enumerate()
            .map(|(row, &time)| {
                (
                    time,
                    SeriesBarPatch {
                        open: channel(&columns[0], row),
                        high: channel(&columns[1], row),
                        low: channel(&columns[2], row),
                        close: channel(&columns[3], row),
                        colors: [None; 3],
                    },
                )
            })
            .collect::<Vec<_>>();
        let outcome = self
            .engine
            .merge_series_bars(id as SeriesId, &rows, sequence);
        outcome_json(outcome, sequence)
    }

    /// Install (finite) or clear (`NaN`) the series' sequence baseline.
    pub fn set_series_update_sequence(&mut self, id: u32, sequence: f64) -> bool {
        match decode_sequence(sequence) {
            Ok(sequence) => self
                .engine
                .set_series_update_sequence(id as SeriesId, sequence),
            Err(_) => false,
        }
    }
}
