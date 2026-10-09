//! Viewport preservation across time-point changes.
//!
//! The reference (`ChartModel.updateTimeScale`, chart-model.ts:953-984) decides compensation
//! with a heuristic: it compensates only for points "added to the right" when the first time did
//! not move left. Aeris accepts out-of-order inserts, historical batches, prepend-plus-append
//! replacements, and retention trims, so the heuristic would move a scrolled-back view by every
//! inserted or trimmed bar. The engine therefore maps the view's right border through the exact
//! old-to-new logical mapping it already computes for drawings (common timestamps, or the bar
//! sequence identity on a non-time axis). The follow-latest rule and the reference results for
//! appends are unchanged.

use aeris_charts_core::model::data_layer::MergedTimeMapping;

use crate::ChartEngine;

impl ChartEngine {
    /// The right offset that keeps the intended view for the data synchronization in progress,
    /// decided against the scale state before the new points land. `None` keeps the offset
    /// relative to the latest bar: the view follows new bars (reference `shiftVisibleRangeOnNewBar`
    /// at the live edge), the scale has no view yet, or the logical range is locked.
    pub(crate) fn viewport_right_offset_after_sync(
        &self,
        merged: Option<&MergedTimeMapping>,
    ) -> Option<f64> {
        let options = self.time_scale.options();
        if options.lock_visible_logical_range {
            return None;
        }
        let visible = self.time_scale.visible_strict_range()?;
        let new_base = self.data.base_index()?;
        let old_base = self.time_scale.base_index();
        let right_offset = self.time_scale.right_offset();
        let sequence = self.pending_sequence_mapping.as_ref();
        // The reference's `replacedExistingWhitespace` is "the time points did not change". On a
        // non-time sequence axis the row keys can stay identical while bar identities move
        // (retention plus append), so a pending sequence rebuild counts as changed points.
        let points_changed = self.data.time_points_generation()
            != self.synced_time_points_generation
            || sequence.is_some();
        let follow = visible.contains(old_base)
            && options.shift_visible_range_on_new_bar
            && (points_changed || options.allow_shift_visible_range_on_whitespace_replacement);
        if follow {
            return None;
        }
        let right_border = old_base as f64 + right_offset;
        let mapped = match (sequence, merged) {
            (Some(mapping), _) if !mapping.is_empty() => Some(mapping.map_logical(right_border)),
            (None, Some(mapping)) if mapping.has_common_time() => {
                Some(mapping.map_logical(right_border))
            }
            // Tail appends, value-only updates and whitespace replacements keep every index.
            (None, None) => Some(right_border),
            // No common bar identity: nothing exact to anchor on, keep the reference heuristic.
            _ => None,
        };
        match mapped {
            Some(right_border) => Some(right_border - new_base as f64),
            None => {
                let old_first = self.synced_first_time?;
                let new_first = *self.data.merged_times().first()?;
                (new_base > old_base && old_first <= new_first)
                    .then(|| right_offset - (new_base - old_base) as f64)
            }
        }
    }

    /// Shift in-flight scroll motion by the data-sync rebase so a kinetic coast, held keyboard
    /// pan, or animated scroll continues over the same content instead of overwriting the
    /// compensation with its pre-rebase absolute positions.
    pub(crate) fn rebase_view_motion(&mut self, shift: f64) {
        if shift == 0.0 || !shift.is_finite() {
            return;
        }
        if let Some(kinetic) = self.kinetic.as_mut() {
            kinetic.shift_positions(shift);
        }
        if let Some(keyboard) = self.keyboard_scroll_animation.as_mut() {
            keyboard.position += shift;
        }
        if let Some(animation) = self.scroll_animation.as_mut() {
            animation.start_position += shift;
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::ChartEngine;
    use crate::footprint::{
        AggressorSide, FootprintAggregationOptions, FootprintBarAggregation,
        FootprintSeriesOptions, FootprintTrade,
    };

    const EPS: f64 = 1e-9;

    fn bars(times: &[i64]) -> (Vec<f64>, Vec<f64>) {
        let times = times.iter().map(|&time| time as f64).collect::<Vec<_>>();
        let values = times.iter().map(|time| 100.0 + time).collect::<Vec<_>>();
        (times, values)
    }

    fn install(chart: &mut ChartEngine, times: &[i64]) {
        let (times, values) = bars(times);
        chart
            .set_series_data(0, &times, &values, &values, &values, &values)
            .unwrap();
    }

    fn chart_with(times: &[i64]) -> ChartEngine {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        chart.time_scale.set_width(800.0);
        install(&mut chart, times);
        // 50 visible bars keep scrolled-back windows inside the data in every scenario.
        chart.set_bar_spacing(16.0);
        chart
    }

    fn range(chart: &ChartEngine) -> (f64, f64) {
        chart.visible_logical_range().unwrap()
    }

    /// Timestamp at a logical position, interpolated between merged time points so a fractional
    /// edge compares by content rather than by index.
    fn time_at(chart: &ChartEngine, logical: f64) -> f64 {
        let times = chart.data_layer().merged_times();
        let index = logical.floor();
        let fraction = logical - index;
        assert!(
            index >= 0.0 && (index as usize) < times.len(),
            "logical {logical} outside the data"
        );
        let at = |i: f64| times[i as usize] as f64;
        if fraction == 0.0 {
            return at(index);
        }
        at(index) + fraction * (at(index + 1.0) - at(index))
    }

    fn visible_times(chart: &ChartEngine) -> (f64, f64) {
        let (from, to) = range(chart);
        (time_at(chart, from), time_at(chart, to))
    }

    fn assert_close(actual: (f64, f64), expected: (f64, f64), message: &str) {
        assert!(
            (actual.0 - expected.0).abs() < EPS && (actual.1 - expected.1).abs() < EPS,
            "{message}: {actual:?} != {expected:?}"
        );
    }

    fn step(times: std::ops::RangeInclusive<i64>, by: usize) -> Vec<i64> {
        times.step_by(by).collect()
    }

    #[test]
    fn set_data_with_more_left_history_keeps_a_scrolled_back_view() {
        let mut chart = chart_with(&step(1000..=1099, 1));
        chart.set_right_offset(-30.25);
        let before = visible_times(&chart);
        let (from, to) = range(&chart);

        install(&mut chart, &step(950..=1099, 1));

        assert_close(visible_times(&chart), before, "same bars after prepend");
        assert_close(range(&chart), (from + 50.0, to + 50.0), "range rebased");
    }

    #[test]
    fn prepend_and_append_in_one_replacement_keep_a_scrolled_back_view() {
        let mut chart = chart_with(&step(1000..=1099, 1));
        chart.set_right_offset(-30.0);
        let before = visible_times(&chart);

        install(&mut chart, &step(980..=1104, 1));

        assert_close(
            visible_times(&chart),
            before,
            "history prepend plus tail append in one set_data",
        );
    }

    #[test]
    fn historical_backfill_left_of_the_view_keeps_a_scrolled_back_view() {
        // Every other second is missing; backfilling gaps left of the view inserts time points.
        let mut chart = chart_with(&step(1000..=1198, 2));
        chart.set_right_offset(-20.0);
        let before = visible_times(&chart);

        assert!(chart.update_series_bar(0, 1003.0, [1.0, 1.0, 1.0, 1.0]));
        assert_close(visible_times(&chart), before, "single out-of-order insert");

        let values = vec![1.0; 3];
        let accepted = chart.update_series_bars_sanitized(
            0,
            vec![1005, 1007, 1009],
            values.clone(),
            values.clone(),
            values.clone(),
            values,
        );
        assert_eq!(accepted, 3);
        assert_close(visible_times(&chart), before, "historical batch backfill");
    }

    #[test]
    fn insert_inside_a_scrolled_back_view_keeps_both_edges() {
        let mut chart = chart_with(&step(1000..=1198, 2));
        chart.set_right_offset(-20.0);
        let before = visible_times(&chart);
        let (from, _) = range(&chart);
        let inside = time_at(&chart, from.ceil() + 5.0) as i64 + 1;

        assert!(chart.update_series_bar(0, inside as f64, [1.0, 1.0, 1.0, 1.0]));

        let after = visible_times(&chart);
        assert!((after.1 - before.1).abs() < EPS, "right edge keeps its bar");
    }

    #[test]
    fn retention_trim_with_an_append_keeps_a_scrolled_back_view() {
        let mut chart = chart_with(&step(1000..=1127, 1));
        assert!(chart.set_series_max_points(0, Some(128)));
        chart.set_right_offset(-40.0);
        let before = visible_times(&chart);

        // The 129th row exceeds the cap and trims the margin from the front.
        assert!(chart.update_series_bar(0, 1128.0, [1.0, 1.0, 1.0, 1.0]));
        assert!(chart.data_layer().merged_times().len() < 128);

        assert_close(visible_times(&chart), before, "trim plus append");
    }

    #[test]
    fn retention_trim_at_the_edge_still_follows_the_latest_bar() {
        let mut chart = chart_with(&step(1000..=1127, 1));
        assert!(chart.set_series_max_points(0, Some(128)));
        assert_eq!(chart.right_offset(), 0.0);

        assert!(chart.update_series_bar(0, 1128.0, [1.0, 1.0, 1.0, 1.0]));

        assert_eq!(chart.right_offset(), 0.0, "live edge keeps following");
        let (_, to) = range(&chart);
        assert_eq!(time_at(&chart, to), 1128.0);
    }

    #[test]
    fn follow_latest_and_scrolled_back_append_keep_reference_results() {
        let mut chart = chart_with(&step(1..=10, 1));
        chart.update_series_bar(0, 11.0, [1.0, 1.0, 1.0, 1.0]);
        assert_eq!(chart.right_offset(), 0.0, "edge follows");

        chart.set_right_offset(-5.0);
        chart.update_series_bar(0, 12.0, [1.0, 1.0, 1.0, 1.0]);
        assert_eq!(chart.right_offset(), -6.0, "scrolled back compensates");

        chart.set_right_offset(0.0);
        chart.set_shift_visible_range_on_new_bar(false);
        chart.update_series_bar(0, 13.0, [1.0, 1.0, 1.0, 1.0]);
        assert_eq!(chart.right_offset(), -1.0, "edge without shift compensates");
    }

    #[test]
    fn disjoint_replacement_keeps_reference_behavior() {
        // No common timestamp: the exact mapping has no anchor, so the reference heuristic keeps
        // deciding. A later first time with more bars compensates to the same logical window.
        let mut chart = chart_with(&step(1000..=1099, 1));
        chart.set_right_offset(-30.0);
        let (from, to) = range(&chart);
        install(&mut chart, &step(5000..=5119, 1));
        assert_close(range(&chart), (from, to), "absolute logical window kept");

        // An earlier first time keeps the offset from the latest bar.
        let mut chart = chart_with(&step(1000..=1099, 1));
        chart.set_right_offset(-30.0);
        install(&mut chart, &step(1..=120, 1));
        assert_eq!(chart.right_offset(), -30.0);
    }

    #[test]
    fn active_drag_keeps_the_grabbed_bars_when_bars_arrive() {
        let mut chart = chart_with(&step(1000..=1099, 1));
        chart.set_right_offset(-30.0);
        chart.time_scale_start_scroll(400.0);
        chart.time_scale_scroll_to(380.0);
        let before = visible_times(&chart);

        chart.update_series_bar(0, 1100.0, [1.0, 1.0, 1.0, 1.0]);
        chart.time_scale_scroll_to(380.0);

        assert_close(
            visible_times(&chart),
            before,
            "drag continues on the same bars",
        );
        chart.time_scale_end_scroll();
    }

    #[test]
    fn in_flight_motion_continues_over_the_same_bars_when_bars_arrive() {
        // Held keyboard pan: the next tick continues from the compensated position.
        let mut chart = chart_with(&step(1000..=1099, 1));
        chart.set_right_offset(-30.0);
        chart.start_keyboard_scroll(-1.0, 0.0);
        chart.keyboard_scroll_tick(16.0);
        let before = visible_times(&chart);
        chart.update_series_bar(0, 1100.0, [1.0; 4]);
        chart.keyboard_scroll_tick(17.0);
        let after = visible_times(&chart);
        assert!(
            (after.1 - before.1).abs() < 0.5,
            "keyboard pan jumped: {before:?} -> {after:?}"
        );
        chart.cancel_keyboard_scroll();

        // Kinetic coast positions move with the rebase.
        chart.set_right_offset(-30.0);
        chart.kinetic_begin_sampling(true, -30.0, 0.0);
        chart.kinetic_add_sample(-29.0, 10.0);
        chart.kinetic_add_sample(-27.0, 20.0);
        assert!(chart.kinetic_release(-27.0, 25.0));
        let coast = chart.kinetic_position(40.0).unwrap();
        chart.update_series_bar(0, 1101.0, [1.0; 4]);
        assert!((chart.kinetic_position(40.0).unwrap() - (coast - 1.0)).abs() < EPS);
        chart.kinetic_stop();
    }

    #[test]
    fn fractional_logical_range_round_trips_exactly() {
        let mut chart = chart_with(&step(1..=200, 1));
        chart.set_visible_logical_range(20.25, 79.75);
        assert_close(range(&chart), (20.25, 79.75), "fractional range restored");
        let spacing = chart.bar_spacing();
        assert!(
            (spacing - 800.0 / 60.5).abs() < EPS,
            "count = to - from + 1"
        );

        // A saved range restores without the half-bar jump.
        chart.set_right_offset(-17.4);
        let saved = range(&chart);
        chart.set_visible_logical_range(10.0, 50.0);
        chart.set_visible_logical_range(saved.0, saved.1);
        assert_close(range(&chart), saved, "save/restore is the identity");
    }

    #[test]
    fn scroll_to_real_time_uses_the_configured_right_offset() {
        let mut chart = chart_with(&step(1..=200, 1));
        chart.apply_right_offset_option(5.0);
        chart.set_right_offset(-40.0);
        chart.scroll_to_real_time();
        assert_eq!(chart.right_offset(), 5.0);

        // The animated form eases to the same target and clears itself.
        chart.set_right_offset(-40.0);
        chart.start_real_time_scroll_animation(400.0, 1_000.0);
        let mid = chart.scroll_animation_tick(1_200.0).unwrap();
        assert!(mid > -40.0 && mid < 5.0, "mid-animation position {mid}");
        assert!(chart.scroll_animation_tick(1_400.0).is_none());
        assert_eq!(chart.right_offset(), 5.0);
    }

    // ---- fixed full-session views (intraday time-sharing charts) ----

    const SESSION_SLOTS: i64 = 241;

    /// One trading session of `SESSION_SLOTS` one-minute slots starting at `open`, every slot
    /// a whitespace row until it trades.
    fn install_session(chart: &mut ChartEngine, open: i64) {
        let times = (0..SESSION_SLOTS)
            .map(|slot| (open + slot * 60) as f64)
            .collect::<Vec<_>>();
        let values = vec![f64::NAN; times.len()];
        chart
            .set_series_data(0, &times, &values, &values, &values, &values)
            .unwrap();
    }

    fn trade_minute(chart: &mut ChartEngine, open: i64, slot: i64) {
        let price = 10.0 + slot as f64 * 0.01;
        assert!(chart.update_series_bar(0, (open + slot * 60) as f64, [price; 4]));
    }

    const OPEN: i64 = 1_700_000_000;
    const FULL: (f64, f64) = (0.0, (SESSION_SLOTS - 1) as f64);

    #[test]
    fn reference_clamp_shifts_a_full_session_before_two_bars_trade() {
        // Evidence for the opt-in lock: the reference `maxRightOffset` keeps
        // `min(2, points)` bars left of the base index visible (time-scale.ts:1130-1134). With
        // the base at slot 0 (pre-open or first minute) `[0, N - 1]` is clamped one bar left,
        // and compensation then preserves that shifted window for the whole session.
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        chart.time_scale.set_width(800.0);
        install_session(&mut chart, OPEN);
        chart.set_visible_logical_range(FULL.0, FULL.1);
        assert_close(range(&chart), (-1.0, FULL.1 - 1.0), "pre-open clamp");
        trade_minute(&mut chart, OPEN, 0);
        trade_minute(&mut chart, OPEN, 1);
        assert_close(range(&chart), (-1.0, FULL.1 - 1.0), "kept for the session");
    }

    #[test]
    fn locked_session_view_shows_every_slot_from_pre_open_through_resizes() {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        chart.time_scale.set_width(800.0);
        chart
            .apply_options(r#"{"timeScale":{"lockVisibleLogicalRange":true}}"#)
            .unwrap();
        install_session(&mut chart, OPEN);
        chart.set_visible_logical_range(FULL.0, FULL.1);
        assert_eq!(range(&chart), FULL, "all-whitespace pre-open session");

        trade_minute(&mut chart, OPEN, 0);
        assert_eq!(range(&chart), FULL, "first traded minute");
        for slot in 1..120 {
            trade_minute(&mut chart, OPEN, slot);
        }
        // A current-minute replacement and a tail value update leave it untouched too.
        trade_minute(&mut chart, OPEN, 119);
        assert_eq!(range(&chart), FULL, "midday");

        chart.time_scale.set_width(537.0);
        assert_eq!(range(&chart), FULL, "narrow resize");
        assert_eq!(chart.bar_spacing(), 537.0 / SESSION_SLOTS as f64);
        chart.time_scale.set_width(1_280.0);
        assert_eq!(range(&chart), FULL, "wide resize");

        for slot in 120..SESSION_SLOTS {
            trade_minute(&mut chart, OPEN, slot);
        }
        assert_eq!(range(&chart), FULL, "session close");

        // The next session replaces every slot with new whitespace times.
        install_session(&mut chart, OPEN + 86_400);
        assert_eq!(range(&chart), FULL, "next session pre-open");
        let x_first = chart.time_scale.index_to_coordinate(0);
        let x_last = chart.time_scale.index_to_coordinate(SESSION_SLOTS - 1);
        let spacing = chart.bar_spacing();
        assert!(
            (x_first - (spacing / 2.0 - 1.0)).abs() < 1e-6,
            "slot 0 at the left edge"
        );
        assert!((x_last - (1_280.0 - spacing / 2.0 - 1.0)).abs() < 1e-6);
    }

    #[test]
    fn locked_view_rejects_follow_latest_but_still_accepts_explicit_scrolls() {
        let mut chart = chart_with(&step(1..=100, 1));
        chart.set_lock_visible_logical_range(true);
        let locked = range(&chart);
        chart.update_series_bar(0, 101.0, [1.0; 4]);
        assert_eq!(range(&chart), locked, "no follow at the edge");

        chart.set_right_offset(-10.0);
        let moved = range(&chart);
        assert_ne!(moved, locked);
        chart.update_series_bar(0, 102.0, [1.0; 4]);
        assert_close(range(&chart), moved, "explicit scroll re-locks");

        chart.set_lock_visible_logical_range(false);
        let options: serde_json::Value =
            serde_json::from_str(&chart.time_scale_options_json()).unwrap();
        assert_eq!(options["lock_visible_logical_range"], false);
        chart.set_right_offset(0.0);
        chart.update_series_bar(0, 103.0, [1.0; 4]);
        assert_eq!(chart.right_offset(), 0.0, "reference follow restored");
    }

    /// A locked session whose first `traded` slots have a price; the rest are whitespace.
    fn locked_partial_session(slots: i64, traded: usize, from: f64, to: f64) -> ChartEngine {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        chart.time_scale.set_width(800.0);
        let times = (0..slots)
            .map(|slot| (OPEN + slot * 60) as f64)
            .collect::<Vec<_>>();
        let mut values = vec![f64::NAN; times.len()];
        values[..traded].fill(10.0);
        chart
            .set_series_data(0, &times, &values, &values, &values, &values)
            .unwrap();
        chart.set_lock_visible_logical_range(true);
        chart.set_visible_logical_range(from, to);
        chart
    }

    #[test]
    fn locked_view_keeps_an_active_drag_and_coast_when_a_slot_trades() {
        // Filling a whitespace slot moves the base index while the locked range stays put, so
        // the drag snapshot and the coast must rebase with it instead of jumping one bar.
        let mut chart = locked_partial_session(100, 50, 0.0, 99.0);
        chart.time_scale_start_scroll(400.0);
        chart.time_scale_scroll_to(390.0);
        let before = range(&chart);
        trade_minute(&mut chart, OPEN, 50);
        assert_close(range(&chart), before, "lock holds on the sync");
        chart.time_scale_scroll_to(390.0);
        assert_close(range(&chart), before, "drag continues on the same bars");
        chart.time_scale_end_scroll();

        let mut chart = locked_partial_session(300, 250, 100.0, 199.0);
        let position = chart.right_offset();
        chart.kinetic_begin_sampling(true, position, 0.0);
        chart.kinetic_add_sample(position + 1.0, 10.0);
        chart.kinetic_add_sample(position + 3.0, 20.0);
        assert!(chart.kinetic_release(position + 3.0, 25.0));
        chart.scroll_to_position(chart.kinetic_position(40.0).unwrap());
        let before = range(&chart);
        trade_minute(&mut chart, OPEN, 250);
        assert_close(range(&chart), before, "lock holds on the sync");
        chart.scroll_to_position(chart.kinetic_position(40.0).unwrap());
        assert_close(range(&chart), before, "coast continues on the same bars");
    }

    #[test]
    fn multi_series_history_and_fixed_left_edge_keep_a_scrolled_back_view() {
        // A second series' prepend reindexes the merged union; the view keeps its bars.
        let mut chart = chart_with(&step(1000..=1099, 1));
        let id = chart.add_series(crate::SeriesKind::Line);
        let sparse = |from: i64| {
            step(from..=1098, 2)
                .into_iter()
                .map(|time| time as f64)
                .collect::<Vec<_>>()
        };
        let times = sparse(1000);
        chart
            .set_series_data(id, &times, &times, &times, &times, &times)
            .unwrap();
        chart.set_right_offset(-30.0);
        let before = visible_times(&chart);
        let times = sparse(950);
        chart
            .set_series_data(id, &times, &times, &times, &times, &times)
            .unwrap();
        assert_close(visible_times(&chart), before, "second-series prepend");

        // `fix_left_edge` clamps only the left border; a prepend moves that border away.
        let mut chart = chart_with(&step(1000..=1099, 1));
        chart.set_fix_left_edge(true);
        chart.set_right_offset(-30.0);
        let before = visible_times(&chart);
        install(&mut chart, &step(900..=1099, 1));
        assert_close(visible_times(&chart), before, "prepend under fix_left_edge");
    }

    fn trade(timestamp_micros: i64, price: f64) -> FootprintTrade {
        FootprintTrade {
            timestamp_micros,
            price,
            volume: 1.0,
            aggressor: AggressorSide::Buy,
            bid: None,
            ask: None,
            sequence: None,
            trade_id: None,
            conditions: 0,
            session_id: None,
        }
    }

    fn sequence_chart(count: i64) -> (ChartEngine, u32) {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        chart.time_scale.set_width(800.0);
        let id = chart
            .add_footprint_series(FootprintSeriesOptions {
                aggregation: FootprintAggregationOptions {
                    tick_size: 1.0,
                    bars: FootprintBarAggregation::Trades { trades_per_bar: 1 },
                    ..FootprintAggregationOptions::default()
                },
                ..FootprintSeriesOptions::default()
            })
            .unwrap();
        let trades = (0..count)
            .map(|i| trade(1_000_000_000 + i * 1_000_000, 100.0 + (i % 5) as f64))
            .collect();
        chart.set_footprint_trades(id, trades).unwrap();
        chart.set_bar_spacing(16.0);
        (chart, id)
    }

    #[test]
    fn sequence_axis_append_keeps_a_scrolled_back_view() {
        let (mut chart, id) = sequence_chart(200);
        chart.set_right_offset(-50.0);
        let before = range(&chart);

        for i in 200..203 {
            chart
                .update_footprint_trade(id, trade(1_000_000_000 + i * 1_000_000, 101.0))
                .unwrap();
        }

        assert_close(range(&chart), before, "tick bars do not drift the view");
    }

    #[test]
    fn sequence_axis_follows_at_the_edge_and_honors_shift_disabled() {
        let (mut chart, id) = sequence_chart(200);
        assert_eq!(chart.right_offset(), 0.0);
        chart
            .update_footprint_trade(id, trade(1_000_000_000 + 200 * 1_000_000, 101.0))
            .unwrap();
        assert_eq!(chart.right_offset(), 0.0, "edge follows new tick bars");

        chart.set_shift_visible_range_on_new_bar(false);
        let before = range(&chart);
        chart
            .update_footprint_trade(id, trade(1_000_000_000 + 201 * 1_000_000, 101.0))
            .unwrap();
        assert_eq!(chart.right_offset(), -1.0, "shift disabled compensates");
        assert_close(range(&chart), before, "same bars stay");
    }

    #[test]
    fn sequence_axis_trim_and_append_rebuilds_tick_weights() {
        // One trade per bar, one second apart: minute boundaries give some bars a higher tick
        // weight. A retention trim in the same sync as an append shifts every surviving bar to a
        // lower index, so extending the old weights incrementally would misplace those marks.
        // 100 bars, then one batch of 50 new bars: 150 rows trim to 124, which is still more
        // rows than were synced, so the sync looks like a tail append although the front moved.
        let (mut chart, id) = sequence_chart(100);
        assert!(chart.set_series_max_points(id, Some(128)));
        let batch = (100..150)
            .map(|i| trade(1_000_000_000 + i * 1_000_000, 101.0))
            .collect::<Vec<_>>();
        chart.update_footprint_trades(id, batch).unwrap();
        assert_eq!(chart.sequence_points().unwrap().len(), 124);
        let times = chart
            .sequence_points()
            .unwrap()
            .iter()
            .map(|point| point.open_timestamp_micros.div_euclid(1_000_000))
            .collect::<Vec<_>>();
        let mut expected = vec![0u8; times.len()];
        aeris_charts_core::scale::time_tick_marks::fill_weights_for_points_in(
            &times,
            &mut expected,
            0,
            &chart.exchange_time,
        );
        let mut installed = chart
            .tick_marks
            .build(1.0, 0.0)
            .iter()
            .map(|mark| (mark.index as usize, mark.weight))
            .collect::<Vec<_>>();
        installed.sort_unstable_by_key(|(index, _)| *index);
        let installed = installed
            .into_iter()
            .map(|(_, weight)| weight)
            .collect::<Vec<_>>();
        assert_eq!(installed, expected);
    }

    #[test]
    fn sequence_axis_weights_ignore_a_close_time_label() {
        // A close-time label prints an interval after a bar's identity, but a non-time sequence
        // axis prints and weighs its own open times: every weight path stays unshifted, the
        // plain append included and a retention trim in the same sync as an append.
        let close_label = crate::BarTimeLabel::Close {
            interval_seconds: 60,
            windows: Vec::new(),
        };
        let installed_and_expected = |chart: &mut ChartEngine| {
            let times = chart
                .sequence_points()
                .unwrap()
                .iter()
                .map(|point| point.open_timestamp_micros.div_euclid(1_000_000))
                .collect::<Vec<_>>();
            let mut expected = vec![0u8; times.len()];
            aeris_charts_core::scale::time_tick_marks::fill_weights_for_points_in(
                &times,
                &mut expected,
                0,
                &chart.exchange_time,
            );
            let mut installed = chart
                .tick_marks
                .build(1.0, 0.0)
                .iter()
                .map(|mark| (mark.index as usize, mark.weight))
                .collect::<Vec<_>>();
            installed.sort_unstable_by_key(|(index, _)| *index);
            (
                installed
                    .into_iter()
                    .map(|(_, weight)| weight)
                    .collect::<Vec<_>>(),
                expected,
            )
        };
        let (mut chart, id) = sequence_chart(100);
        chart.set_bar_time_label(close_label).unwrap();
        let (installed, expected) = installed_and_expected(&mut chart);
        assert_eq!(installed, expected, "label set after the data");
        // Ten appended bars extend the weights incrementally.
        for i in 100..110 {
            chart
                .update_footprint_trade(id, trade(1_000_000_000 + i * 1_000_000, 101.0))
                .unwrap();
        }
        let (installed, expected) = installed_and_expected(&mut chart);
        assert_eq!(installed, expected, "incremental append");
        // A batch that trims and appends in one sync rebuilds them.
        assert!(chart.set_series_max_points(id, Some(128)));
        let batch = (110..150)
            .map(|i| trade(1_000_000_000 + i * 1_000_000, 101.0))
            .collect::<Vec<_>>();
        chart.update_footprint_trades(id, batch).unwrap();
        assert_eq!(chart.sequence_points().unwrap().len(), 124);
        let (installed, expected) = installed_and_expected(&mut chart);
        assert_eq!(installed, expected, "trim and append");
        // Clearing the label rebuilds them again without changing a sequence axis.
        chart.set_bar_time_label(crate::BarTimeLabel::Open).unwrap();
        let (installed, expected) = installed_and_expected(&mut chart);
        assert_eq!(installed, expected, "label cleared");
    }

    #[test]
    fn sequence_axis_retention_keeps_a_scrolled_back_view() {
        let (mut chart, id) = sequence_chart(128);
        assert!(chart.set_series_max_points(id, Some(128)));
        chart.set_right_offset(-40.0);
        let before = range(&chart);
        let first_visible = chart.sequence_points().unwrap()[before.0.ceil() as usize];

        chart
            .update_footprint_trade(id, trade(1_000_000_000 + 128 * 1_000_000, 101.0))
            .unwrap();

        let after = range(&chart);
        let trimmed = before.0 - after.0;
        assert!(
            trimmed > 0.0,
            "front rows were trimmed: {before:?} -> {after:?}"
        );
        assert!((after.1 - after.0 - (before.1 - before.0)).abs() < EPS);
        let same = chart.sequence_points().unwrap()[after.0.ceil() as usize];
        assert_eq!(
            same.open_timestamp_micros, first_visible.open_timestamp_micros,
            "the same tick bar stays at the left edge"
        );
    }
}
