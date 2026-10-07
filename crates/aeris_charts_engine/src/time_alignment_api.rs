//! Multi-calendar overlay alignment (`series_options.time_alignment`). The data layer owns the
//! as-of mapping (`TimeAlignment`); this module validates which series may use it and keeps the
//! chart's time state, drawings, and retained frames synchronized when the policy changes.

use super::*;

impl ChartEngine {
    /// Choose how an ordinary time-based series lands on the shared time axis.
    ///
    /// [`TimeAlignment::Union`] (the default) adds the series' timestamps to the chart's time
    /// points, as the reference does. [`TimeAlignment::AsOf`] is for an overlay from another
    /// market calendar: it adds no time point, and every point up to the last real bar of the
    /// union-timed series shows the overlay's last row at or before that point's time (rows
    /// between two points collapse into the later one; a point without a newer row repeats the
    /// previous one unless it is older than `max_staleness` seconds). Studies bound to the series
    /// compute on its own rows and are shown the same way.
    ///
    /// Only line, area, baseline, histogram, bar, and candlestick series that own their rows
    /// may be as-of: indicator outputs follow their source, and custom, advanced, footprint,
    /// trade-bound, trade-study, and synthetic series, like any series on a non-time bar axis,
    /// are refused with `unsupported_operation`. A negative staleness is `invalid_options`.
    /// Unchanged requests are no-ops. Converting an as-of series to a custom, advanced, or
    /// footprint series, binding it to a trade stream, or configuring it as a synthetic
    /// transform returns it to the union.
    pub fn set_series_time_alignment(
        &mut self,
        id: SeriesId,
        alignment: TimeAlignment,
    ) -> Result<(), ChartError> {
        match self.data.validate_series_id(id) {
            Ok(()) => {}
            Err(SeriesIdError::Stale(_)) => {
                return Err(ChartError::new(
                    ErrorCode::StaleHandle,
                    format!("series {id} was removed"),
                ));
            }
            Err(SeriesIdError::Unknown(_)) => {
                return Err(ChartError::new(
                    ErrorCode::InvalidHandle,
                    format!("series {id} does not exist"),
                ));
            }
        }
        if let TimeAlignment::AsOf {
            max_staleness: Some(max),
        } = alignment
            && max < 0
        {
            return Err(ChartError::new(
                ErrorCode::InvalidOptions,
                "as_of_max_staleness must be a non-negative number of seconds",
            ));
        }
        if self.data.time_alignment(id) == Some(alignment) {
            return Ok(());
        }
        if let Some(reason) = self.time_alignment_refusal(id) {
            return Err(ChartError::new(ErrorCode::UnsupportedOperation, reason));
        }
        if alignment.is_as_of()
            && self.indicators.iter().any(|binding| {
                binding.structure.is_some()
                    && self.data.time_alignment_owner(binding.source) == Some(id)
            })
        {
            // Structure annotations name canonical rows, which an as-of plot repeats or skips.
            // A study on an indicator output follows the output's source series, so a structure
            // study anywhere down a chain keeps that series off the as-of alignment.
            return Err(ChartError::new(
                ErrorCode::UnsupportedOperation,
                "a series with a structure study cannot become an as-of overlay",
            ));
        }
        self.apply_time_alignment(id, alignment);
        Ok(())
    }

    /// Return a series to the union before it becomes source-owned in a domain without its own
    /// calendar (a footprint, a trade-bound bar series, or a synthetic transform).
    pub(crate) fn rejoin_time_union(&mut self, id: SeriesId) {
        self.apply_time_alignment(id, TimeAlignment::Union);
    }

    fn apply_time_alignment(&mut self, id: SeriesId, alignment: TimeAlignment) {
        if !self.data.set_time_alignment(id, alignment) {
            return;
        }
        // Positions of every series may move with the union; a same-union staleness change moves
        // only this series and its studies.
        self.reset_series_scale_stabilization(id);
        self.sync_time_points();
        self.invalidate_frame_scene();
        self.invalidate_frame_layout_and_axis();
    }

    /// The alignment a series follows (an indicator output reports its source's). `None` for an
    /// unknown or removed id.
    pub fn series_time_alignment(&self, id: SeriesId) -> Option<TimeAlignment> {
        self.data.time_alignment(id)
    }

    fn time_alignment_refusal(&self, id: SeriesId) -> Option<String> {
        let series = self.series_entry(id)?;
        if !matches!(
            series.kind,
            SeriesKind::Line
                | SeriesKind::Area
                | SeriesKind::Baseline
                | SeriesKind::Histogram
                | SeriesKind::Bar
                | SeriesKind::Candlestick
        ) {
            return Some(format!(
                "time_alignment applies to line, area, baseline, histogram, bar, and candlestick series, not {:?}",
                series.kind
            ));
        }
        if self
            .indicators
            .iter()
            .any(|binding| binding.outputs.contains(&id))
        {
            return Some("an indicator output follows its source series' time alignment".into());
        }
        if matches!(
            self.series_owner(id),
            Some(SeriesOwner::Synthetic | SeriesOwner::TradeBars | SeriesOwner::TradeStudy)
        ) {
            return Some(
                "a trade-bound, trade-study, or synthetic series has no calendar of its own".into(),
            );
        }
        // Row keys of a non-time (tick, volume, range, or synthetic) axis are not times.
        self.sequence_points
            .is_some()
            .then(|| "a non-time bar axis has no calendar to align to".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aeris_charts_render::draw_list::Prim;

    const DAY: f64 = 86_400.0;

    fn days(list: &[f64]) -> Vec<f64> {
        list.iter().map(|day| day * DAY).collect()
    }

    const AS_OF: TimeAlignment = TimeAlignment::AsOf {
        max_staleness: None,
    };

    /// HK trades d1 d2 d3 d5 d6 (d4 an HK holiday); the US index trades d1 d2 d4 d5 d7.
    fn hk_with_us_overlay() -> (ChartEngine, SeriesId) {
        let mut chart = ChartEngine::new(800.0, 400.0, 1.0);
        let hk = days(&[1.0, 2.0, 3.0, 5.0, 6.0]);
        let close = [100.0, 102.0, 101.0, 105.0, 104.0];
        chart
            .set_series_data(0, &hk, &close, &close, &close, &close)
            .unwrap();
        let us = chart.add_series(SeriesKind::Line);
        let us_times = days(&[1.0, 2.0, 4.0, 5.0, 7.0]);
        let us_close = [4000.0, 4040.0, 4100.0, 4120.0, 4200.0];
        chart
            .set_series_data(us, &us_times, &us_close, &us_close, &us_close, &us_close)
            .unwrap();
        (chart, us)
    }

    fn snapshot(chart: &ChartEngine, id: SeriesId, index: Option<i64>) -> SeriesValueSnapshot {
        chart
            .value_snapshot(index)
            .into_iter()
            .find(|snapshot| snapshot.series_id == id)
            .unwrap()
    }

    fn snapshot_value(chart: &ChartEngine, id: SeriesId, index: i64) -> Option<f64> {
        let snapshot = snapshot(chart, id, Some(index));
        snapshot.value.or(snapshot.close)
    }

    #[test]
    fn as_of_overlay_keeps_the_primary_calendar_gapless() {
        let (mut chart, us) = hk_with_us_overlay();
        // The union inserts the HK holiday d4 and the US-only d7.
        assert_eq!(chart.data_layer().merged_times().len(), 7);
        assert_eq!(chart.series_time_alignment(us), Some(TimeAlignment::Union));

        chart.set_series_time_alignment(us, AS_OF).unwrap();
        assert_eq!(chart.series_time_alignment(us), Some(AS_OF));
        let merged = chart.data_layer().merged_times();
        assert_eq!(
            merged,
            days(&[1.0, 2.0, 3.0, 5.0, 6.0])
                .iter()
                .map(|&t| t as i64)
                .collect::<Vec<_>>()
        );
        // Every HK slot holds a bar: no empty slots were inserted into the main series.
        let hk = chart.data_layer().plot(0);
        assert_eq!(hk.indices().collect::<Vec<_>>(), vec![0, 1, 2, 3, 4]);
        // The overlay shows its last close at or before each HK day.
        let values = (0..5)
            .map(|index| snapshot_value(&chart, us, index))
            .collect::<Vec<_>>();
        assert_eq!(
            values,
            vec![
                Some(4000.0),
                Some(4040.0),
                Some(4040.0), // US holiday d3 repeats d2
                Some(4120.0), // d4 collapses into d5
                Some(4120.0),
            ]
        );
        // The snapshot names the HK slot's time; data() keeps the overlay's own rows.
        assert_eq!(snapshot(&chart, us, Some(2)).time, Some(3 * 86_400));
        assert_eq!(
            chart
                .series_data(us)
                .iter()
                .map(|point| point.time)
                .collect::<Vec<_>>(),
            [1, 2, 4, 5, 7].map(|day| day * 86_400).to_vec()
        );
        // Back to the union restores the reference axis.
        chart
            .set_series_time_alignment(us, TimeAlignment::Union)
            .unwrap();
        assert_eq!(chart.data_layer().merged_times().len(), 7);
    }

    #[test]
    fn as_of_tips_on_both_sides_update_the_overlay() {
        let (mut chart, us) = hk_with_us_overlay();
        chart.set_series_time_alignment(us, AS_OF).unwrap();
        // HK trades d8: the overlay extends with its last row at or before d8 (US d7).
        assert!(chart.update_series_bar(0, 8.0 * DAY, [106.0; 4]));
        assert_eq!(chart.data_layer().merged_times().len(), 6);
        assert_eq!(snapshot_value(&chart, us, 5), Some(4200.0));
        // A US tick on d7 repaints the HK d8 point it backs.
        assert!(chart.update_series_bar(us, 7.0 * DAY, [4210.0; 4]));
        assert_eq!(snapshot_value(&chart, us, 5), Some(4210.0));
        // US d9 is ahead of HK: it adds no slot and shows once HK reaches d9.
        assert!(chart.update_series_bar(us, 9.0 * DAY, [4300.0; 4]));
        assert_eq!(chart.data_layer().merged_times().len(), 6);
        assert_eq!(snapshot_value(&chart, us, 5), Some(4210.0));
        assert!(chart.update_series_bar(0, 9.0 * DAY, [107.0; 4]));
        assert_eq!(snapshot_value(&chart, us, 6), Some(4300.0));
        // Latest-value queries follow the as-of point as well.
        let latest = snapshot(&chart, us, None);
        assert_eq!(latest.logical_index, Some(6));
        assert_eq!(latest.value, Some(4300.0));
        assert!(!chart.build_frame().panes.is_empty());
    }

    #[test]
    fn as_of_comparison_base_and_legend_use_the_as_of_values() {
        let (mut chart, us) = hk_with_us_overlay();
        chart.set_series_time_alignment(us, AS_OF).unwrap();
        // Anchor at HK d3, a US holiday: the US base is its d2 close.
        assert!(chart.set_comparison_anchor(Some(3.0 * DAY)));
        let legend = chart.comparison_legend_snapshot();
        let entry = legend.iter().find(|entry| entry.series_id == us).unwrap();
        assert_eq!(entry.anchor_value, Some(4040.0));
        assert_eq!(entry.latest_value, Some(4120.0));
        assert_eq!(entry.latest_time, Some(6 * 86_400));
        let percent = entry.percent_change.unwrap();
        assert!((percent - (4120.0 - 4040.0) / 4040.0 * 100.0).abs() < 1e-9);
        assert_eq!(chart.series_base_value(us, 0), Some(4040.0));
    }

    #[test]
    fn studies_on_an_as_of_overlay_compute_on_its_own_rows() {
        let (mut chart, us) = hk_with_us_overlay();
        chart.set_series_time_alignment(us, AS_OF).unwrap();
        let sma = chart.add_sma(us, 2).unwrap();
        assert_eq!(chart.series_time_alignment(sma), Some(AS_OF));
        assert!(matches!(
            chart.set_series_time_alignment(sma, TimeAlignment::Union),
            Err(error) if error.code() == ErrorCode::UnsupportedOperation
        ));
        // SMA(2) over the US rows (4000, 4040, 4100, 4120, 4200): 4020, 4070, 4110, 4160, shown
        // as of each HK day (US d4's 4070 collapses into d5).
        let values = (0..5)
            .map(|index| snapshot_value(&chart, sma, index))
            .collect::<Vec<_>>();
        assert_eq!(
            values,
            vec![None, Some(4020.0), Some(4020.0), Some(4110.0), Some(4110.0)]
        );
        // A US tick recomputes the study on US rows; HK d8 then shows it.
        assert!(chart.update_series_bar(us, 7.0 * DAY, [4300.0; 4]));
        assert!(chart.update_series_bar(0, 8.0 * DAY, [106.0; 4]));
        assert_eq!(snapshot_value(&chart, sma, 5), Some(4210.0));
        assert_eq!(chart.data_layer().merged_times().len(), 6);
    }

    /// Pane-0 polyline points (device px, DPR 1) of the only line series.
    fn overlay_line_points(chart: &mut ChartEngine) -> Vec<[f32; 2]> {
        let frame = chart.build_frame();
        let pane = &frame.panes[0];
        let lines = pane
            .main
            .iter()
            .filter_map(|primitive| match primitive {
                Prim::Polyline {
                    first_point,
                    point_count,
                    ..
                } => Some((*first_point as usize, *point_count as usize)),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(lines.len(), 1, "one overlay stroke");
        let (first, count) = lines[0];
        pane.points[first..first + count].to_vec()
    }

    #[test]
    fn as_of_overlay_geometry_hits_and_markers_sit_on_primary_slots() {
        let (mut chart, us) = hk_with_us_overlay();
        chart.set_series_time_alignment(us, AS_OF).unwrap();
        chart.time_scale.set_width(800.0);
        chart.fit_content();
        let color = Color::rgb(222, 17, 99);
        let waiting = Color::rgb(17, 222, 99);
        let marker = |day: i64, color: Color| Marker {
            time: day * 86_400,
            position: marker_pos::ABOVE,
            shape: marker_shape::CIRCLE,
            color,
            text: String::new(),
            id: String::new(),
            size: 1.0,
            price: None,
        };
        // A marker on US d4 (an HK holiday) lands on the HK d5 slot that shows that row's
        // successor, the first point at or after its time. US d7 is newer than every HK point, so
        // its marker waits with its row instead of sitting on HK d6.
        chart.set_series_markers(us, vec![marker(4, color), marker(7, waiting)]);
        let points = overlay_line_points(&mut chart);
        // One point per HK slot, at the slot's x; the US holiday d3 repeats d2's level and d6
        // repeats d5's.
        assert_eq!(points.len(), 5);
        for (index, point) in points.iter().enumerate() {
            let x = chart.time_scale.index_to_coordinate(index as i64) as f32;
            assert!(
                (point[0] - x).abs() < 1e-3,
                "point {index} at {point:?}, slot {x}"
            );
        }
        assert_eq!(points[1][1], points[2][1]);
        assert_eq!(points[3][1], points[4][1]);
        assert_ne!(points[2][1], points[3][1]);

        // The repeated point hit-tests as the overlay.
        let top = chart.panes[0].top as f32;
        let hit = chart.hit_test_one_series(us, points[2][0] as f64, (points[2][1] + top) as f64);
        assert!(hit.is_some());

        let frame = chart.build_frame();
        let marker_x = chart.time_scale.index_to_coordinate(3).round() as f32 + 0.5;
        assert!(frame.panes[0].main.iter().any(|primitive| matches!(
            primitive,
            Prim::Circle { cx, fill, .. } if *fill == color && (*cx - marker_x).abs() < 1e-3
        )));
        let drawn = |frame: &ChartFrame, wanted: Color| {
            frame.panes[0]
                .main
                .iter()
                .filter_map(|primitive| match primitive {
                    Prim::Circle { cx, fill, .. } if *fill == wanted => Some(*cx),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        assert!(drawn(&frame, waiting).is_empty());
        // HK d8 shows US d7, and the marker with it.
        assert!(chart.update_series_bar(0, 8.0 * DAY, [106.0; 4]));
        chart.fit_content();
        let frame = chart.build_frame();
        let marker_x = chart.time_scale.index_to_coordinate(5).round() as f32 + 0.5;
        assert_eq!(drawn(&frame, waiting), vec![marker_x]);
    }

    #[test]
    fn primary_tips_extend_the_overlay_and_invalidate_its_retained_layer() {
        let (mut chart, us) = hk_with_us_overlay();
        chart.set_series_time_alignment(us, AS_OF).unwrap();
        chart.time_scale.set_width(800.0);
        chart.fit_content();
        assert_eq!(overlay_line_points(&mut chart).len(), 5);
        // Only the primary series changes; the overlay's retained layer must still rebuild.
        assert!(chart.update_series_bar(0, 8.0 * DAY, [106.0; 4]));
        chart.fit_content();
        let points = overlay_line_points(&mut chart);
        assert_eq!(points.len(), 6);
        let x = chart.time_scale.index_to_coordinate(5) as f32;
        assert!((points[5][0] - x).abs() < 1e-3);
    }

    #[test]
    fn overlay_follows_trades_into_preinstalled_session_slots_without_a_new_time_point() {
        let mut chart = ChartEngine::new(800.0, 400.0, 1.0);
        let nan = f64::NAN;
        // A six-minute session installed as whitespace slots; two minutes have traded.
        let slots = [60.0, 120.0, 180.0, 240.0, 300.0, 360.0];
        let close = [10.0, 11.0, nan, nan, nan, nan];
        chart
            .set_series_data(0, &slots, &close, &close, &close, &close)
            .unwrap();
        let index = chart.add_series(SeriesKind::Line);
        chart
            .set_series_data(
                index,
                &[0.0, 150.0],
                &[50.0; 2],
                &[50.0; 2],
                &[50.0; 2],
                &[50.0; 2],
            )
            .unwrap();
        chart.set_series_time_alignment(index, AS_OF).unwrap();
        // A flat overlay on its own scale in the fixed full-session view.
        assert!(chart.try_set_series_pane_and_scale(index, 0, 1.0, "left"));
        chart.time_scale.set_width(800.0);
        chart.fit_content();
        chart.set_lock_visible_logical_range(true);
        // The overlay stops at the last traded minute instead of running through the session.
        assert_eq!(overlay_line_points(&mut chart).len(), 2);
        let generation = chart.data_layer().time_points_generation();
        // The third minute trades: no time point changes, yet the overlay's layer repaints.
        assert!(chart.update_series_bar(0, 180.0, [12.0; 4]));
        assert_eq!(chart.data_layer().time_points_generation(), generation);
        assert_eq!(overlay_line_points(&mut chart).len(), 3);
        assert_eq!(snapshot_value(&chart, index, 2), Some(50.0));
    }

    #[test]
    fn primary_reinstalls_and_pops_inside_session_slots_repaint_the_overlay() {
        let mut chart = ChartEngine::new(800.0, 400.0, 1.0);
        let nan = f64::NAN;
        let slots = [60.0, 120.0, 180.0, 240.0, 300.0, 360.0];
        let close = [10.0, 11.0, nan, nan, nan, nan];
        chart
            .set_series_data(0, &slots, &close, &close, &close, &close)
            .unwrap();
        let index = chart.add_series(SeriesKind::Line);
        let flat = [50.0; 3];
        chart
            .set_series_data(index, &[0.0, 150.0, 250.0], &flat, &flat, &flat, &flat)
            .unwrap();
        chart.set_series_time_alignment(index, AS_OF).unwrap();
        assert!(chart.try_set_series_pane_and_scale(index, 0, 1.0, "left"));
        chart.time_scale.set_width(800.0);
        // The fixed full-session view.
        chart.set_lock_visible_logical_range(true);
        chart.set_visible_logical_range(0.0, 5.0);
        assert_eq!(overlay_line_points(&mut chart).len(), 2);
        let generation = chart.data_layer().time_points_generation();
        // The host reinstalls the session with a fourth traded minute: the same time points,
        // but the overlay now reaches that minute and its retained layer must rebuild.
        // Inside the primary's price range, so no autoscale change repaints the pane.
        let close = [10.0, 11.0, 10.5, 10.5, nan, nan];
        chart
            .set_series_data(0, &slots, &close, &close, &close, &close)
            .unwrap();
        assert_eq!(chart.data_layer().time_points_generation(), generation);
        assert_eq!(overlay_line_points(&mut chart).len(), 4);
        // A trailing whitespace slot popped with a traded bar pulls it back; the slot stays
        // because the union still reaches it only through the popped series.
        assert_eq!(chart.series_pop(0, 3), Some(3));
        assert_eq!(overlay_line_points(&mut chart).len(), 3);
    }

    #[test]
    fn session_highlighting_on_an_as_of_overlay_colors_each_point_as_the_row_it_shows() {
        let (mut chart, us) = hk_with_us_overlay();
        chart.set_series_time_alignment(us, AS_OF).unwrap();
        chart.time_scale.set_width(800.0);
        chart.fit_content();
        let weekday = Color::rgba(10, 20, 30, 60);
        let weekend = Color::rgba(200, 20, 30, 60);
        chart
            .add_session_highlighting(
                us,
                SessionHighlightingOptions {
                    weekday_color: weekday,
                    weekend_color: weekend,
                    ..SessionHighlightingOptions::default()
                },
            )
            .unwrap();
        // Epoch day 1 is a Friday. The HK points d1 d2 d3 d5 d6 show the US rows d1 (Fri),
        // d2 (Sat), d2 again, d5 (Tue), and d5 again: one weekday slot, two weekend slots, and
        // two weekday slots, one bar spacing each.
        let spacing = chart.time_scale.bar_spacing();
        let widths = |chart: &mut ChartEngine, wanted: Color| {
            chart.build_frame().panes[0]
                .under
                .iter()
                .filter_map(|primitive| match primitive {
                    Prim::Rect { rect, color } if *color == wanted => Some(rect.w as f64),
                    _ => None,
                })
                .sum::<f64>()
        };
        assert!((widths(&mut chart, weekend) - 2.0 * spacing).abs() <= 2.0);
        assert!((widths(&mut chart, weekday) - 3.0 * spacing).abs() <= 2.0);

        // Host callback colors are keyed by the overlay's own rows as well: only US d2 is marked,
        // and it backs two HK slots.
        let primitive = chart
            .add_session_highlighting(us, SessionHighlightingOptions::default())
            .unwrap();
        let marked = Color::rgba(1, 222, 3, 90);
        let records = chart
            .series_data(us)
            .iter()
            .map(|point| SessionHighlightingData {
                time: point.time,
                color: if point.time == 2 * 86_400 {
                    marked
                } else {
                    Color::rgba(0, 0, 0, 0)
                },
            })
            .collect();
        assert!(chart.set_session_highlighting_data(primitive, records));
        assert!((widths(&mut chart, marked) - 2.0 * spacing).abs() <= 2.0);
    }

    #[test]
    fn accessibility_focus_on_an_as_of_row_rings_the_first_point_showing_it() {
        let mut chart = ChartEngine::new(800.0, 400.0, 1.0);
        let hk = days(&[1.0, 2.0, 3.0, 5.0]);
        let close = [100.0; 4];
        chart
            .set_series_data(0, &hk, &close, &close, &close, &close)
            .unwrap();
        let us = chart.add_series(SeriesKind::Line);
        let us_close = [4000.0, 4040.0, 4100.0];
        chart
            .set_series_data(
                us,
                &days(&[1.0, 2.0, 4.0]),
                &us_close,
                &us_close,
                &us_close,
                &us_close,
            )
            .unwrap();
        chart.set_series_time_alignment(us, AS_OF).unwrap();
        chart.time_scale.set_width(800.0);
        chart.fit_content();
        let options = AccessibilityFocusOptions {
            color: Color::rgb(12, 34, 56),
            size: 14.0,
            high_contrast: false,
        };
        let primitive = chart.add_accessibility_focus(us, options).unwrap();
        // US d4 (an HK holiday) is first shown at the HK d5 slot.
        assert!(chart.set_accessibility_focus(primitive, Some(4 * 86_400), options));
        let frame = chart.build_frame();
        let x = chart.time_scale.index_to_coordinate(3) as f32;
        assert!(frame.panes[0].main.iter().any(|primitive| matches!(
            primitive,
            Prim::Circle { cx, stroke, .. } if *stroke == options.color && (*cx - x).abs() < 1e-3
        )));
    }

    #[test]
    fn source_owned_conversions_return_an_as_of_series_to_the_union() {
        let (mut chart, us) = hk_with_us_overlay();
        chart.set_series_time_alignment(us, AS_OF).unwrap();
        let hk_axis = chart.data_layer().merged_times().to_vec();

        // Binding a candle series to a trade stream hands its rows to the stream's domain.
        let candles = chart.add_series(SeriesKind::Candlestick);
        chart.set_series_time_alignment(candles, AS_OF).unwrap();
        let stream = chart
            .add_trade_stream("US:tape", FootprintAggregationOptions::default())
            .unwrap();
        chart
            .bind_trade_bar_series_to_stream(candles, stream)
            .unwrap();
        assert_eq!(
            chart.series_time_alignment(candles),
            Some(TimeAlignment::Union)
        );
        assert!(matches!(
            chart.set_series_time_alignment(candles, AS_OF),
            Err(error) if error.code() == ErrorCode::UnsupportedOperation
        ));
        // Trade studies are stream-owned as well.
        let delta = chart.add_delta_series(stream, 1).unwrap();
        assert!(matches!(
            chart.set_series_time_alignment(delta, AS_OF),
            Err(error) if error.code() == ErrorCode::UnsupportedOperation
        ));

        // A footprint conversion does the same.
        let footprint = chart.add_series(SeriesKind::Line);
        chart.set_series_time_alignment(footprint, AS_OF).unwrap();
        chart
            .configure_footprint_series(footprint, FootprintSeriesOptions::default())
            .unwrap();
        assert_eq!(
            chart.series_time_alignment(footprint),
            Some(TimeAlignment::Union)
        );
        // The overlay itself is untouched, and the empty converted series added no point.
        assert_eq!(chart.series_time_alignment(us), Some(AS_OF));
        assert_eq!(chart.data_layer().merged_times(), hk_axis.as_slice());
    }

    #[test]
    fn a_synthetic_transform_rejoins_the_union_and_its_sequence_axis_refuses_as_of() {
        let mut chart = ChartEngine::new(800.0, 400.0, 1.0);
        chart
            .set_series_time_alignment(0, AS_OF)
            .expect("a candle series may be as-of before it is configured");
        chart
            .configure_synthetic_bar_series(0, SyntheticBarOptions::RenkoFixed { box_size: 1.0 })
            .unwrap();
        assert_eq!(chart.series_time_alignment(0), Some(TimeAlignment::Union));
        let source = [100.0, 103.0, 99.0]
            .iter()
            .enumerate()
            .map(|(index, &close)| SyntheticSourceBar {
                timestamp_micros: (index as i64 + 1) * 1_000_000,
                open: close,
                high: close,
                low: close,
                close,
            })
            .collect();
        chart.set_synthetic_bar_source(0, source).unwrap();
        assert!(chart.sequence_points().is_some());
        // Logical row keys of the Renko axis are not times, so no overlay can join them as-of.
        let line = chart.add_series(SeriesKind::Line);
        assert!(matches!(
            chart.set_series_time_alignment(line, AS_OF),
            Err(error) if error.code() == ErrorCode::UnsupportedOperation
        ));
        assert_eq!(
            chart.series_time_alignment(line),
            Some(TimeAlignment::Union)
        );
    }

    #[test]
    fn time_alignment_refuses_series_without_an_own_calendar() {
        let (mut chart, _) = hk_with_us_overlay();
        let custom = chart.add_series(SeriesKind::Custom);
        assert!(matches!(
            chart.set_series_time_alignment(custom, AS_OF),
            Err(error) if error.code() == ErrorCode::UnsupportedOperation
        ));
        assert!(matches!(
            chart.set_series_time_alignment(
                0,
                TimeAlignment::AsOf {
                    max_staleness: Some(-1)
                }
            ),
            Err(error) if error.code() == ErrorCode::InvalidOptions
        ));
        assert!(matches!(
            chart.set_series_time_alignment(999, AS_OF),
            Err(error) if error.code() == ErrorCode::InvalidHandle
        ));
        chart.remove_series(custom);
        assert!(matches!(
            chart.set_series_time_alignment(custom, AS_OF),
            Err(error) if error.code() == ErrorCode::StaleHandle
        ));
    }

    #[test]
    fn switching_alignment_keeps_drawings_on_their_moments() {
        let (mut chart, us) = hk_with_us_overlay();
        chart.time_scale.set_width(800.0);
        chart.fit_content();
        chart.build_frame();
        // Union axis d1 d2 d3 d4 d5 d6 d7: a trend line from HK d3 (logical 2) to d6 (5).
        let id = chart
            .add_drawing(
                DrawingKind::TrendLine,
                0,
                vec![
                    DrawingPoint {
                        logical: 2.0,
                        price: 101.0,
                    },
                    DrawingPoint {
                        logical: 5.0,
                        price: 104.0,
                    },
                ],
                None,
            )
            .unwrap();
        chart.set_series_time_alignment(us, AS_OF).unwrap();
        // Without the US-only d4 slot d6 is logical 4; d3 stays at 2.
        let points = &chart.drawing(id).unwrap().points;
        assert_eq!((points[0].logical, points[1].logical), (2.0, 4.0));
        chart
            .set_series_time_alignment(us, TimeAlignment::Union)
            .unwrap();
        let points = &chart.drawing(id).unwrap().points;
        assert_eq!((points[0].logical, points[1].logical), (2.0, 5.0));
    }

    #[test]
    fn alignment_chosen_before_data_never_adds_time_points() {
        let mut chart = ChartEngine::new(640.0, 360.0, 1.0);
        let times = (0..600)
            .map(|i| 1_577_836_800.0 + i as f64 * 60.0)
            .collect::<Vec<_>>();
        let values = vec![100.0; 600];
        chart
            .set_series_data(0, &times, &values, &values, &values, &values)
            .unwrap();
        chart.time_scale.set_width(600.0);
        chart.fit_content();
        let before = chart.visible_logical_range();
        // A host picks the alignment at creation, then installs and streams data.
        let id = chart.add_series(SeriesKind::Line);
        chart.set_series_time_alignment(id, AS_OF).unwrap();
        let overlay = (0..85).map(|i| times[i * 7] + 30.0).collect::<Vec<_>>();
        let values = vec![100.0; 85];
        chart
            .set_series_data(id, &overlay, &values, &values, &values, &values)
            .unwrap();
        assert!(chart.update_series_bar(id, overlay[84] + 60.0, [101.0; 4]));
        chart.fit_content();
        assert_eq!(chart.data_layer().merged_times().len(), 600);
        assert_eq!(chart.visible_logical_range(), before);
        // Every minute from the overlay's first row on shows a value.
        assert_eq!(chart.data_layer().plot(id).size(), 599);
    }
}
