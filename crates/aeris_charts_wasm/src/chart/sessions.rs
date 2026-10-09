//! Browser translation for exchange-session bars: session-anchored trade streams, the stream
//! volume histogram, and engine-owned OHLCV resampling. Request parsing lives in the
//! host-testable `session_slots` boundary; the engine owns every calculation.

use super::*;
use crate::session_slots::{resample_series_json, trade_sessions_json};

impl ChartInner {
    /// Anchor a trade stream's time bars to session windows in the chart's exchange time
    /// (`"null"` restores the plain anchor grid). Returns the rejection reason, or `""`.
    pub(super) fn set_trade_stream_sessions(
        &mut self,
        stream_id: u32,
        sessions_json: &str,
    ) -> String {
        let sessions = match trade_sessions_json(sessions_json) {
            Ok(sessions) => sessions,
            Err(error) => return error,
        };
        self.engine
            .set_trade_stream_sessions(u64::from(stream_id), sessions)
            .map_or_else(|error| error.to_string(), |()| String::new())
    }

    pub(super) fn add_trade_volume_series(&mut self, stream_id: u32, pane_index: usize) -> u32 {
        self.engine
            .add_trade_volume_series(u64::from(stream_id), pane_index)
            .unwrap_or(u32::MAX)
    }

    /// Bind `target` (and an optional volume target) to resampled bars of a source series.
    /// Returns the rejection reason, or `""`.
    pub(super) fn configure_resampled_series(&mut self, target: u32, options_json: &str) -> String {
        let (source, volume_source, volume_target, options) =
            match resample_series_json(options_json) {
                Ok(request) => request,
                Err(error) => return error,
            };
        self.engine
            .configure_resampled_series(source, volume_source, target, volume_target, options)
            .map_or_else(|error| error.to_string(), |()| String::new())
    }

    pub(super) fn resample_stats_json(&self, target: u32) -> String {
        self.engine
            .resample_stats(target)
            .and_then(|stats| serde_json::to_string(&stats).ok())
            .unwrap_or_else(|| "null".to_string())
    }
}
