//! Partial (merge) and sequence-guarded streaming updates for built-in series.
//!
//! The plain [`ChartEngine::update_series_bar`] keeps reference `series.update` semantics: the
//! point replaces the whole bar at its time. Real-time feeds often deliver partial ticks (a new
//! close, a revised high) and can deliver them late or twice. This module adds the two engine-owned
//! answers without changing `update`: a field-wise merge into the existing bar, and an optional
//! monotonic per-series sequence that rejects stale deliveries in O(1).

use super::*;
use aeris_charts_core::model::data_validation::{MAX_SAFE_VALUE, MIN_SAFE_VALUE};

/// Fields of a partial streaming update. `None` keeps the existing bar's value.
///
/// For candlestick/bar series an existing bar keeps every absent field, and the merged result is
/// normalized so `high >= max(open, close)` and `low <= min(open, close)`; a close-only patch
/// therefore extends the bar's range to the new close. A patch for a time with no bar (or a
/// whitespace slot) creates one whose absent fields follow the supplied close (or open/high/low),
/// so a close-only patch creates `O = H = L = C`. Scalar series (line/area/baseline/histogram)
/// take `close` as their value. Volume and turnover are not bar fields: merge them into their own
/// series.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SeriesBarPatch {
    pub open: Option<f64>,
    pub high: Option<f64>,
    pub low: Option<f64>,
    /// Close price, or the value of a scalar series.
    pub close: Option<f64>,
    /// Body/wick/border color overrides (packed RGBA). `None` keeps the bar's current override.
    pub colors: [Option<u32>; 3],
}

impl SeriesBarPatch {
    fn is_empty(&self) -> bool {
        self.open.is_none() && self.high.is_none() && self.low.is_none() && self.close.is_none()
    }
}

/// Why a merge or sequence-guarded update changed nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SeriesUpdateRejection {
    /// Unknown or removed series identity.
    UnknownSeries,
    /// Custom, advanced (feature), footprint, and engine-derived series own their payloads
    /// elsewhere. Engine-derived means trade-bound candles and bars, CVD, delta, and volume
    /// studies, and synthetic or resampled targets: their stream, source, or resampler is the
    /// only writer. [`ChartEngine::series_is_source_owned`] tells the derived case apart from a
    /// custom, feature, or footprint series.
    UnsupportedSeries,
    /// The patch carried no price the series stores: none of open/high/low/close, or no close
    /// (value) for a line/area/baseline/histogram series.
    EmptyPatch,
    /// Invalid timestamp or non-finite/out-of-range values.
    InvalidData,
}

/// Result of [`ChartEngine::merge_series_bars`] and the sequence-guarded update entry points.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SeriesUpdateOutcome {
    Applied,
    /// The supplied sequence was not newer than the last one applied to this series. The update
    /// was discarded and the series is unchanged.
    StaleSequence {
        last_applied: u64,
    },
    Rejected(SeriesUpdateRejection),
}

impl ChartEngine {
    /// Last sequence applied to this series by a sequence-guarded update, if any.
    pub fn series_update_sequence(&self, id: SeriesId) -> Option<u64> {
        self.series_entry(id)?.update_sequence
    }

    /// Install (or with `None`, clear) the series' sequence baseline, for example the sequence of
    /// a snapshot the host just installed. A full data install clears the baseline itself.
    pub fn set_series_update_sequence(&mut self, id: SeriesId, sequence: Option<u64>) -> bool {
        let Some(series) = self.series_entry_mut(id) else {
            return false;
        };
        series.update_sequence = sequence;
        true
    }

    /// [`Self::update_series_bar_styled`] behind the optional sequence guard: with `Some(sequence)`
    /// an update whose sequence is not newer than the last applied one is discarded as stale.
    pub fn update_series_bar_sequenced(
        &mut self,
        id: SeriesId,
        time: f64,
        values: [f64; 4],
        colors: [Option<u32>; 3],
        sequence: Option<u64>,
    ) -> SeriesUpdateOutcome {
        if let Err(rejection) = self.check_streaming_series(id, sequence) {
            return rejection;
        }
        if !self.update_series_bar_styled(id, time, values, colors) {
            return SeriesUpdateOutcome::Rejected(SeriesUpdateRejection::InvalidData);
        }
        self.record_update_sequence(id, sequence)
    }

    /// Sequence-guarded form of [`Self::update_series_bars_sanitized`]: one sequence covers the
    /// whole ascending batch.
    #[allow(clippy::too_many_arguments)] // the typed OHLC columns plus the guard
    pub fn update_series_bars_sanitized_sequenced(
        &mut self,
        id: SeriesId,
        times: Vec<i64>,
        open: Vec<f64>,
        high: Vec<f64>,
        low: Vec<f64>,
        close: Vec<f64>,
        sequence: Option<u64>,
    ) -> SeriesUpdateOutcome {
        if let Err(rejection) = self.check_streaming_series(id, sequence) {
            return rejection;
        }
        // An empty batch is a valid delivery that changes nothing; it still advances the guard.
        if !times.is_empty()
            && self.update_series_bars_sanitized(id, times, open, high, low, close) == 0
        {
            return SeriesUpdateOutcome::Rejected(SeriesUpdateRejection::InvalidData);
        }
        self.record_update_sequence(id, sequence)
    }

    /// Merge a partial bar into the series at `time` (see [`SeriesBarPatch`]). The result is
    /// applied through the ordinary streaming path, so a tail bar stays O(1) and a historical bar
    /// is corrected in place.
    pub fn merge_series_bar(
        &mut self,
        id: SeriesId,
        time: f64,
        patch: SeriesBarPatch,
        sequence: Option<u64>,
    ) -> SeriesUpdateOutcome {
        self.merge_series_bars(id, &[(time, patch)], sequence)
    }

    /// Merge a batch of partial bars as consecutive [`Self::merge_series_bar`] calls in input
    /// order (a later patch for the same time merges into the earlier result), then synchronize
    /// time state and dependent indicators once. The batch is validated atomically: one empty
    /// patch, invalid timestamp, or non-finite/out-of-range value rejects every row. One
    /// `sequence` guards the whole batch.
    pub fn merge_series_bars(
        &mut self,
        id: SeriesId,
        rows: &[(f64, SeriesBarPatch)],
        sequence: Option<u64>,
    ) -> SeriesUpdateOutcome {
        if let Err(rejection) = self.check_streaming_series(id, sequence) {
            return rejection;
        }
        let scalar = self
            .series_entry(id)
            .is_some_and(|series| series.kind.stores_scalar_values());
        let mut timestamps = Vec::with_capacity(rows.len());
        for &(time, patch) in rows {
            // Scalar series store one value; a patch without it has nothing to merge.
            if patch.is_empty() || (scalar && patch.close.is_none()) {
                return SeriesUpdateOutcome::Rejected(SeriesUpdateRejection::EmptyPatch);
            }
            let in_range = [patch.open, patch.high, patch.low, patch.close]
                .into_iter()
                .flatten()
                .all(|value| (MIN_SAFE_VALUE..=MAX_SAFE_VALUE).contains(&value));
            match validate_timestamp(time) {
                Ok(timestamp) if in_range => timestamps.push(timestamp),
                _ => return SeriesUpdateOutcome::Rejected(SeriesUpdateRejection::InvalidData),
            }
        }
        if rows.is_empty() {
            return self.record_update_sequence(id, sequence);
        }

        self.invalidate_frame_series(id);
        let previous_generation = self.data.series_generation(id).unwrap_or(0);
        let mut from = usize::MAX;
        for (&timestamp, &(_, patch)) in timestamps.iter().zip(rows) {
            let (row, existing, existing_colors) = self.existing_bar(id, timestamp);
            let values = match patch.close {
                Some(value) if scalar => [value; 4],
                _ => merge_ohlc(existing, patch),
            };
            let colors =
                std::array::from_fn(|channel| patch.colors[channel].or(existing_colors[channel]));
            from = from.min(row);
            self.data.update_styled(id, timestamp, values, colors);
        }
        // Same synchronization as a streaming update: retention first, then one time sync and
        // one indicator pass from the earliest merged row.
        let trimmed = self.enforce_series_cap(id);
        self.sync_time_points();
        self.update_indicators_after_change(
            id,
            IndicatorChange {
                from: if trimmed { 0 } else { from },
                previous_generation,
                full_replace: trimmed,
            },
        );
        self.record_update_sequence(id, sequence)
    }

    /// Validate a streaming target and the optional sequence guard without mutating anything.
    fn check_streaming_series(
        &self,
        id: SeriesId,
        sequence: Option<u64>,
    ) -> Result<(), SeriesUpdateOutcome> {
        let Some(series) = self.series_entry(id) else {
            return Err(SeriesUpdateOutcome::Rejected(
                SeriesUpdateRejection::UnknownSeries,
            ));
        };
        // Engine-derived series (trade-bound candles and studies, synthetic and resampled bars)
        // change only through their owner; a merge written straight into their rows would
        // diverge from it.
        if matches!(
            series.kind,
            SeriesKind::Custom | SeriesKind::Feature | SeriesKind::Footprint
        ) || self.is_source_owned_series(id)
        {
            return Err(SeriesUpdateOutcome::Rejected(
                SeriesUpdateRejection::UnsupportedSeries,
            ));
        }
        match (sequence, series.update_sequence) {
            (Some(sequence), Some(last_applied)) if sequence <= last_applied => {
                Err(SeriesUpdateOutcome::StaleSequence { last_applied })
            }
            _ => Ok(()),
        }
    }

    fn record_update_sequence(
        &mut self,
        id: SeriesId,
        sequence: Option<u64>,
    ) -> SeriesUpdateOutcome {
        if let (Some(sequence), Some(series)) = (sequence, self.series_entry_mut(id)) {
            series.update_sequence = Some(sequence);
        }
        SeriesUpdateOutcome::Applied
    }

    /// The row `time` occupies (or would be inserted at), plus the non-whitespace bar stored at
    /// exactly `time` and its color overrides. O(log n).
    fn existing_bar(&self, id: SeriesId, time: i64) -> (usize, Option<[f64; 4]>, [Option<u32>; 3]) {
        let Some((times, columns)) = self.data.series_data(id) else {
            return (0, None, [None; 3]);
        };
        let row = match times.binary_search(&time) {
            Ok(row) => row,
            Err(row) => return (row, None, [None; 3]),
        };
        let values = [
            columns[0][row],
            columns[1][row],
            columns[2][row],
            columns[3][row],
        ];
        if values.iter().any(|value| value.is_nan()) {
            return (row, None, [None; 3]);
        }
        let colors = [
            PointColorChannel::Body,
            PointColorChannel::Wick,
            PointColorChannel::Border,
        ]
        .map(|channel| self.data.point_color(id, channel, row));
        (row, Some(values), colors)
    }
}

/// Field-wise OHLC merge with envelope normalization (see [`SeriesBarPatch`]).
fn merge_ohlc(existing: Option<[f64; 4]>, patch: SeriesBarPatch) -> [f64; 4] {
    let (open, high, low, close) = match existing {
        Some([open, high, low, close]) => (
            patch.open.unwrap_or(open),
            patch.high.unwrap_or(high),
            patch.low.unwrap_or(low),
            patch.close.unwrap_or(close),
        ),
        None => {
            let close = patch
                .close
                .or(patch.open)
                .or(patch.high)
                .or(patch.low)
                .unwrap_or(f64::NAN);
            let open = patch.open.unwrap_or(close);
            (
                open,
                patch.high.unwrap_or(open.max(close)),
                patch.low.unwrap_or(open.min(close)),
                close,
            )
        }
    };
    [
        open,
        high.max(open).max(close),
        low.min(open).min(close),
        close,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bars(chart: &mut ChartEngine, id: SeriesId, rows: usize) {
        let times = (0..rows).map(|row| row as f64 * 60.0).collect::<Vec<_>>();
        let close = (0..rows).map(|row| 100.0 + row as f64).collect::<Vec<_>>();
        let open = close.iter().map(|close| close - 0.5).collect::<Vec<_>>();
        let high = close.iter().map(|close| close + 1.0).collect::<Vec<_>>();
        let low = close.iter().map(|close| close - 1.0).collect::<Vec<_>>();
        chart
            .set_series_data(id, &times, &open, &high, &low, &close)
            .unwrap();
    }

    fn row(chart: &ChartEngine, id: SeriesId, row: usize) -> [f64; 4] {
        let (_, columns) = chart.data.series_data(id).unwrap();
        [
            columns[0][row],
            columns[1][row],
            columns[2][row],
            columns[3][row],
        ]
    }

    fn close(value: f64) -> SeriesBarPatch {
        SeriesBarPatch {
            close: Some(value),
            ..SeriesBarPatch::default()
        }
    }

    #[test]
    fn merge_keeps_absent_fields_and_normalizes_the_envelope() {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        bars(&mut chart, 0, 5);
        let last = 4.0 * 60.0;
        assert_eq!(row(&chart, 0, 4), [103.5, 105.0, 103.0, 104.0]);

        // A close-only tick above the high keeps open and low and extends the high.
        assert_eq!(
            chart.merge_series_bar(0, last, close(110.0), None),
            SeriesUpdateOutcome::Applied
        );
        assert_eq!(row(&chart, 0, 4), [103.5, 110.0, 103.0, 110.0]);
        // A close-only tick below the low extends the low and keeps the extended high.
        chart.merge_series_bar(0, last, close(90.0), None);
        assert_eq!(row(&chart, 0, 4), [103.5, 110.0, 90.0, 90.0]);
        // An explicit high below the body is normalized up to the body.
        chart.merge_series_bar(
            0,
            last,
            SeriesBarPatch {
                high: Some(95.0),
                ..SeriesBarPatch::default()
            },
            None,
        );
        assert_eq!(row(&chart, 0, 4), [103.5, 103.5, 90.0, 90.0]);

        // A new bar from a close-only tick is flat; open-only follows the open.
        assert_eq!(
            chart.merge_series_bar(0, 5.0 * 60.0, close(120.0), None),
            SeriesUpdateOutcome::Applied
        );
        assert_eq!(row(&chart, 0, 5), [120.0; 4]);
        chart.merge_series_bar(
            0,
            6.0 * 60.0,
            SeriesBarPatch {
                open: Some(121.0),
                high: Some(125.0),
                ..SeriesBarPatch::default()
            },
            None,
        );
        assert_eq!(row(&chart, 0, 6), [121.0, 125.0, 121.0, 121.0]);
    }

    #[test]
    fn merge_corrects_history_in_place_and_keeps_point_colors() {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        bars(&mut chart, 0, 50_000);
        assert!(chart.set_series_point_colors(0, Some(vec![0; 50_000]), None, None));
        assert!(chart.update_series_bar_styled(
            0,
            49_997.0 * 60.0,
            [1.0, 3.0, 0.5, 2.0],
            [Some(0xff00_00ff), None, None],
        ));
        let passes = chart.data.index_rebuilds();
        let generation = chart.data.time_points_generation();

        // A late close revision of N-3 changes only that bar's close/high; its color survives.
        assert_eq!(
            chart.merge_series_bar(0, 49_997.0 * 60.0, close(4.0), None),
            SeriesUpdateOutcome::Applied
        );
        assert_eq!(row(&chart, 0, 49_997), [1.0, 4.0, 0.5, 4.0]);
        assert_eq!(
            chart.data.point_color(0, PointColorChannel::Body, 49_997),
            Some(0xff00_00ff)
        );
        assert_eq!(chart.data.index_rebuilds(), passes);
        assert_eq!(chart.data.time_points_generation(), generation);
        assert_eq!(chart.series_data(0).len(), 50_000);
    }

    #[test]
    fn scalar_series_merge_sets_the_value_and_rejects_empty_patches() {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        let volume = chart.add_series(SeriesKind::Histogram);
        chart
            .set_series_data(
                volume,
                &[0.0, 60.0],
                &[5.0, 6.0],
                &[5.0, 6.0],
                &[5.0, 6.0],
                &[5.0, 6.0],
            )
            .unwrap();
        assert_eq!(
            chart.merge_series_bar(volume, 60.0, close(1_200.0), None),
            SeriesUpdateOutcome::Applied
        );
        assert_eq!(row(&chart, volume, 1), [1_200.0; 4]);
        assert_eq!(
            chart.merge_series_bar(volume, 60.0, SeriesBarPatch::default(), None),
            SeriesUpdateOutcome::Rejected(SeriesUpdateRejection::EmptyPatch)
        );
        assert_eq!(
            chart.merge_series_bar(
                volume,
                60.0,
                SeriesBarPatch {
                    high: Some(9.0),
                    ..SeriesBarPatch::default()
                },
                None,
            ),
            SeriesUpdateOutcome::Rejected(SeriesUpdateRejection::EmptyPatch)
        );
        assert_eq!(
            chart.merge_series_bar(0, 60.0, close(f64::INFINITY), None),
            SeriesUpdateOutcome::Rejected(SeriesUpdateRejection::InvalidData)
        );
        assert_eq!(
            chart.merge_series_bar(0, 60.5, close(1.0), None),
            SeriesUpdateOutcome::Rejected(SeriesUpdateRejection::InvalidData)
        );
        assert_eq!(row(&chart, volume, 1), [1_200.0; 4]);
    }

    #[test]
    fn stale_sequences_are_rejected_without_mutation() {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        bars(&mut chart, 0, 3);
        let last = 2.0 * 60.0;
        assert_eq!(chart.series_update_sequence(0), None);
        assert_eq!(
            chart.update_series_bar_sequenced(0, last, [1.0, 2.0, 0.5, 1.5], [None; 3], Some(7)),
            SeriesUpdateOutcome::Applied
        );
        assert_eq!(chart.series_update_sequence(0), Some(7));
        let before = chart.data.series_generation(0);

        // Equal and older sequences are stale for every guarded entry point.
        for stale in [7, 3] {
            assert_eq!(
                chart.update_series_bar_sequenced(0, last, [9.0; 4], [None; 3], Some(stale)),
                SeriesUpdateOutcome::StaleSequence { last_applied: 7 }
            );
            assert_eq!(
                chart.merge_series_bar(0, last, close(9.0), Some(stale)),
                SeriesUpdateOutcome::StaleSequence { last_applied: 7 }
            );
            assert_eq!(
                chart.update_series_bars_sanitized_sequenced(
                    0,
                    vec![180],
                    vec![9.0],
                    vec![9.0],
                    vec![9.0],
                    vec![9.0],
                    Some(stale),
                ),
                SeriesUpdateOutcome::StaleSequence { last_applied: 7 }
            );
        }
        assert_eq!(chart.data.series_generation(0), before);
        assert_eq!(row(&chart, 0, 2), [1.0, 2.0, 0.5, 1.5]);

        // Unsequenced updates always apply and leave the guard untouched.
        assert!(chart.update_series_bar(0, last, [2.0; 4]));
        assert_eq!(chart.series_update_sequence(0), Some(7));
        assert_eq!(
            chart.merge_series_bar(0, last, close(3.0), Some(8)),
            SeriesUpdateOutcome::Applied
        );
        assert_eq!(
            chart.update_series_bars_sanitized_sequenced(
                0,
                vec![180, 240],
                vec![4.0, 5.0],
                vec![4.0, 5.0],
                vec![4.0, 5.0],
                vec![4.0, 5.0],
                Some(9),
            ),
            SeriesUpdateOutcome::Applied
        );
        assert_eq!(chart.series_update_sequence(0), Some(9));

        // A rejected payload does not consume its sequence.
        assert_eq!(
            chart.update_series_bar_sequenced(0, 1.5, [1.0; 4], [None; 3], Some(10)),
            SeriesUpdateOutcome::Rejected(SeriesUpdateRejection::InvalidData)
        );
        assert_eq!(chart.series_update_sequence(0), Some(9));

        // A full data install is a resync and clears the guard; hosts may install a baseline.
        bars(&mut chart, 0, 3);
        assert_eq!(chart.series_update_sequence(0), None);
        assert!(chart.set_series_update_sequence(0, Some(100)));
        assert_eq!(
            chart.merge_series_bar(0, last, close(1.0), Some(100)),
            SeriesUpdateOutcome::StaleSequence { last_applied: 100 }
        );
        assert!(!chart.set_series_update_sequence(99, Some(1)));
    }

    /// A merge batch equals the same patches merged one by one (a later patch for the same time
    /// folds into the earlier result), synchronizes once, and rejects atomically.
    #[test]
    fn merge_batch_equals_consecutive_merges_and_rejects_atomically() {
        let last = 4.0 * 60.0;
        let next = 5.0 * 60.0;
        let patches = [
            (60.0, close(50.0)),
            (last, close(110.0)),
            (last, close(90.0)),
            (next, close(120.0)),
            (
                next,
                SeriesBarPatch {
                    high: Some(125.0),
                    ..SeriesBarPatch::default()
                },
            ),
        ];
        let mut batched = ChartEngine::new(800.0, 500.0, 1.0);
        bars(&mut batched, 0, 5);
        let batched_sma = batched.add_sma(0, 3).unwrap();
        let mut single = ChartEngine::new(800.0, 500.0, 1.0);
        bars(&mut single, 0, 5);
        let single_sma = single.add_sma(0, 3).unwrap();

        assert_eq!(
            batched.merge_series_bars(0, &patches, Some(4)),
            SeriesUpdateOutcome::Applied
        );
        for &(time, patch) in &patches {
            assert_eq!(
                single.merge_series_bar(0, time, patch, None),
                SeriesUpdateOutcome::Applied
            );
        }
        assert_eq!(row(&batched, 0, 4), [103.5, 110.0, 90.0, 90.0]);
        assert_eq!(row(&batched, 0, 5), [120.0, 125.0, 120.0, 120.0]);
        assert_eq!(batched.series_data(0), single.series_data(0));
        assert_eq!(
            batched.series_data(batched_sma),
            single.series_data(single_sma)
        );
        assert_eq!(batched.series_update_sequence(0), Some(4));

        // One bad row rejects the whole batch before any row changes.
        let before = batched.series_data(0);
        for (bad, rejection) in [
            (
                (next, close(f64::INFINITY)),
                SeriesUpdateRejection::InvalidData,
            ),
            ((next + 0.5, close(1.0)), SeriesUpdateRejection::InvalidData),
            (
                (next, close(MAX_SAFE_VALUE * 2.0)),
                SeriesUpdateRejection::InvalidData,
            ),
            (
                (next, SeriesBarPatch::default()),
                SeriesUpdateRejection::EmptyPatch,
            ),
        ] {
            assert_eq!(
                batched.merge_series_bars(0, &[(last, close(1.0)), bad], Some(9)),
                SeriesUpdateOutcome::Rejected(rejection)
            );
        }
        assert_eq!(batched.series_data(0), before);
        assert_eq!(batched.series_update_sequence(0), Some(4));
    }

    /// A merged color creates the series' color channel like the same item in a full install,
    /// and an empty sequenced batch is a valid delivery rather than invalid data.
    #[test]
    fn merge_colors_create_channels_and_empty_sequenced_batches_apply() {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        bars(&mut chart, 0, 3);
        assert!(!chart.data.has_point_colors(0));
        let colored = SeriesBarPatch {
            close: Some(1.0),
            colors: [Some(0x1122_33ff), None, None],
            ..SeriesBarPatch::default()
        };
        assert_eq!(
            chart.merge_series_bar(0, 60.0, colored, None),
            SeriesUpdateOutcome::Applied
        );
        assert_eq!(
            chart.data.point_color(0, PointColorChannel::Body, 1),
            Some(0x1122_33ff)
        );
        assert_eq!(chart.data.point_color(0, PointColorChannel::Body, 2), None);

        assert_eq!(
            chart.update_series_bars_sanitized_sequenced(
                0,
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Some(3),
            ),
            SeriesUpdateOutcome::Applied
        );
        assert_eq!(chart.series_update_sequence(0), Some(3));
        assert_eq!(
            chart.merge_series_bars(0, &[], Some(3)),
            SeriesUpdateOutcome::StaleSequence { last_applied: 3 }
        );
    }

    #[test]
    fn custom_feature_and_footprint_series_reject_merges() {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        let custom = chart.add_series(SeriesKind::Custom);
        assert_eq!(
            chart.merge_series_bar(custom, 60.0, close(1.0), None),
            SeriesUpdateOutcome::Rejected(SeriesUpdateRejection::UnsupportedSeries)
        );
        assert_eq!(
            chart.merge_series_bar(1_000, 60.0, close(1.0), None),
            SeriesUpdateOutcome::Rejected(SeriesUpdateRejection::UnknownSeries)
        );
    }

    /// An out-of-order single update that corrects bar N-2 must leave every dependent indicator
    /// identical to a clean recomputation while doing only local data-layer work.
    #[test]
    fn out_of_order_correction_of_n_minus_2_matches_a_clean_rebuild() {
        let rows = 20_000;
        let mut live = ChartEngine::new(800.0, 500.0, 1.0);
        bars(&mut live, 0, rows);
        let live_sma = live.add_sma(0, 14).unwrap();
        let live_ema = live.add_ema(0, 14).unwrap();
        let passes = live.data.index_rebuilds();
        let time = (rows - 3) as f64 * 60.0;
        assert!(live.update_series_bar(0, time, [90.0, 130.0, 80.0, 125.0]));
        assert_eq!(live.data.index_rebuilds(), passes);

        let mut clean = ChartEngine::new(800.0, 500.0, 1.0);
        bars(&mut clean, 0, rows);
        assert!(clean.update_series_bar(0, time, [90.0, 130.0, 80.0, 125.0]));
        let expected = clean.series_data(0);
        let mut rebuilt = ChartEngine::new(800.0, 500.0, 1.0);
        let times = expected
            .iter()
            .map(|bar| bar.time as f64)
            .collect::<Vec<_>>();
        let pick = |f: fn(&SeriesDataPoint) -> f64| expected.iter().map(f).collect::<Vec<_>>();
        rebuilt
            .set_series_data(
                0,
                &times,
                &pick(|bar| bar.open),
                &pick(|bar| bar.high),
                &pick(|bar| bar.low),
                &pick(|bar| bar.close),
            )
            .unwrap();
        let rebuilt_sma = rebuilt.add_sma(0, 14).unwrap();
        let rebuilt_ema = rebuilt.add_ema(0, 14).unwrap();
        let values = |chart: &ChartEngine, id| {
            chart
                .series_data(id)
                .iter()
                .map(|bar| bar.close.to_bits())
                .collect::<Vec<_>>()
        };
        assert_eq!(live.series_data(0).len(), rows);
        assert_eq!(values(&live, live_sma), values(&rebuilt, rebuilt_sma));
        assert_eq!(values(&live, live_ema), values(&rebuilt, rebuilt_ema));
    }
}
