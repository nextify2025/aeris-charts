//! Exchange time zone, trading-day session start, and calendar-date axis state.
//!
//! The chart keeps canonical UTC seconds everywhere. These settings only change how instants are
//! grouped into trading days and presented: tick weights, built-in time labels, VWAP/pivot resets,
//! session highlighting, and the countdown window all read the same [`ExchangeTime`]. Hosts supply
//! an explicit UTC-offset schedule; the engine never consults a platform time zone.

use aeris_charts_core::scale::exchange_time::{ExchangeTime, UtcOffsetSchedule};
use aeris_charts_core::scale::time_tick_marks::fill_weights_for_points_in;

use crate::{ChartEngine, ExchangeTimeError, UtcOffsetTransition};

/// Parsed, validated `timeScale.timeZone` / `timeScale.sessionStart` keys of an options patch.
#[derive(Debug, Default)]
pub(crate) struct ExchangeTimePatch {
    offsets: Option<UtcOffsetSchedule>,
    session_start_seconds: Option<i32>,
}

fn parse_time_zone(value: &serde_json::Value) -> Result<UtcOffsetSchedule, String> {
    if let Some(name) = value.as_str() {
        return if matches!(name, "UTC" | "Etc/UTC") {
            Ok(UtcOffsetSchedule::utc())
        } else {
            Err(format!(
                "timeScale.timeZone {name:?} must be \"UTC\" or an explicit offset schedule; hosts resolve IANA names"
            ))
        };
    }
    let transitions: Vec<UtcOffsetTransition> = serde_json::from_value(value.clone())
        .map_err(|error| format!("invalid timeScale.timeZone schedule: {error}"))?;
    UtcOffsetSchedule::new(transitions).map_err(|error| error.to_string())
}

impl ChartEngine {
    /// The chart's exchange time: offset schedule, session start, and calendar-date flag.
    pub fn exchange_time(&self) -> &ExchangeTime {
        &self.exchange_time
    }

    /// Install the exchange time zone as a validated UTC-offset schedule (UTC by default). Tick
    /// weights, built-in labels, trading-day indicator resets, session highlighting, and the
    /// countdown follow it. The schedule is mirrored into the options store so option snapshots
    /// and V2 persistence round-trip it.
    pub fn set_time_zone(&mut self, offsets: UtcOffsetSchedule) {
        if self.exchange_time.offsets() != &offsets {
            self.exchange_time.set_offsets(offsets);
            self.exchange_time_changed();
        }
        self.mirror_exchange_time_options();
    }

    /// Seconds from local midnight at which the exchange trading day begins (default 0). Negative
    /// values assign an evening session to the next trading day. Rejects values outside ±1 day.
    pub fn set_session_start_seconds(&mut self, seconds: i32) -> Result<(), ExchangeTimeError> {
        let previous = self.exchange_time.session_start_seconds();
        self.exchange_time.set_session_start_seconds(seconds)?;
        if previous != seconds {
            self.exchange_time_changed();
        }
        self.mirror_exchange_time_options();
        Ok(())
    }

    /// Declare that the chart's time points are calendar dates (business days / `YYYY-MM-DD`
    /// input taken at UTC midnight). Calendar dates are never shifted by the time zone or the
    /// session start, so daily bars keep their date in every zone. Browser hosts derive this flag
    /// from the input form of their financial series; Rust hosts set it with their data.
    pub fn set_calendar_date_axis(&mut self, calendar_dates: bool) {
        if self.exchange_time.calendar_dates() == calendar_dates {
            return;
        }
        self.exchange_time.set_calendar_dates(calendar_dates);
        if self.exchange_time.is_utc_identity() {
            // UTC midnights are already their own trading days: weights and period keys are
            // unchanged, so only per-frame state (the countdown window) needs a new frame.
            self.invalidate_frame_all();
        } else {
            self.exchange_time_changed();
        }
    }

    /// Exchange-local wall-clock seconds for a canonical timestamp (identity for calendar dates).
    pub fn exchange_local_seconds(&self, time: i64) -> i64 {
        self.exchange_time.local_seconds(time)
    }

    pub(crate) fn time_zone_json(&self) -> serde_json::Value {
        time_zone_json(self.exchange_time.offsets())
    }

    /// Write the live time zone and session start into the options store, so option snapshots
    /// and V2 persistence always describe the exchange time the chart actually uses (including
    /// after importing a document that predates these keys).
    pub(crate) fn mirror_exchange_time_options(&mut self) {
        self.options.apply(&serde_json::json!({
            "timeScale": {
                "timeZone": self.time_zone_json(),
                "sessionStart": self.exchange_time.session_start_seconds(),
            }
        }));
    }

    /// Validate the exchange-time keys of an options patch without mutating anything.
    pub(crate) fn parse_exchange_time_patch(
        patch: &serde_json::Value,
    ) -> Result<ExchangeTimePatch, String> {
        let Some(time_scale) = patch
            .get("timeScale")
            .and_then(serde_json::Value::as_object)
        else {
            return Ok(ExchangeTimePatch::default());
        };
        let offsets = time_scale
            .get("timeZone")
            .map(parse_time_zone)
            .transpose()?;
        let session_start_seconds = time_scale
            .get("sessionStart")
            .map(|value| {
                let seconds = value
                    .as_f64()
                    .filter(|seconds| seconds.is_finite() && seconds.fract() == 0.0)
                    .ok_or_else(|| {
                        "timeScale.sessionStart must be a whole number of seconds".to_string()
                    })?;
                let seconds = seconds.clamp(i32::MIN as f64, i32::MAX as f64) as i32;
                ExchangeTime::default()
                    .set_session_start_seconds(seconds)
                    .map(|()| seconds)
                    .map_err(|error| error.to_string())
            })
            .transpose()?;
        Ok(ExchangeTimePatch {
            offsets,
            session_start_seconds,
        })
    }

    pub(crate) fn apply_exchange_time_patch(&mut self, patch: ExchangeTimePatch) {
        if let Some(offsets) = patch.offsets {
            self.set_time_zone(offsets);
        }
        if let Some(seconds) = patch.session_start_seconds {
            self.set_session_start_seconds(seconds)
                .expect("session start was validated before the options patch applied");
        }
    }

    fn exchange_time_changed(&mut self) {
        self.rebuild_tick_weights();
        self.rebuild_trading_day_indicators();
        self.refresh_trade_stream_sessions();
        self.invalidate_frame_all();
    }

    /// Recompute every axis weight in the current exchange time without touching view state.
    fn rebuild_tick_weights(&mut self) {
        let sequence_times = self.sequence_points().map(|points| {
            points
                .iter()
                .map(|point| point.open_timestamp_micros.div_euclid(1_000_000))
                .collect::<Vec<_>>()
        });
        let times = sequence_times
            .as_deref()
            .unwrap_or_else(|| self.data.merged_times());
        let mut weights = vec![0u8; times.len()];
        fill_weights_for_points_in(times, &mut weights, 0, &self.exchange_time);
        self.tick_marks.set_weights(&weights);
    }
}

fn time_zone_json(offsets: &UtcOffsetSchedule) -> serde_json::Value {
    if offsets.is_utc() {
        serde_json::Value::String("UTC".to_string())
    } else {
        serde_json::to_value(offsets.transitions()).expect("offset transitions serialize")
    }
}

#[cfg(test)]
mod tests {
    use aeris_charts_core::scale::time_tick_marks::{days_from_civil, TickMarkWeight};

    use crate::{
        AxisTextAlign, AxisTextMidpoint, ChartEngine, SeriesKind, UtcOffsetSchedule,
        UtcOffsetTransition,
    };

    const HOUR: i64 = 3_600;

    fn shanghai() -> UtcOffsetSchedule {
        UtcOffsetSchedule::fixed(8 * HOUR as i32).unwrap()
    }

    /// America/New_York around 2024 (EST, EDT from 2024-03-10 07:00 UTC, EST from 2024-11-03).
    fn new_york() -> UtcOffsetSchedule {
        let at = |y, m, d, h: i64| days_from_civil(y, m, d).unwrap() * 86_400 + h * HOUR;
        UtcOffsetSchedule::new(vec![
            UtcOffsetTransition {
                from_utc_seconds: at(2023, 11, 5, 6),
                offset_seconds: -5 * HOUR as i32,
            },
            UtcOffsetTransition {
                from_utc_seconds: at(2024, 3, 10, 7),
                offset_seconds: -4 * HOUR as i32,
            },
            UtcOffsetTransition {
                from_utc_seconds: at(2024, 11, 3, 6),
                offset_seconds: -5 * HOUR as i32,
            },
        ])
        .unwrap()
    }

    /// UTC instants of exchange-local bars from `from` to `to` (inclusive) every `step` minutes.
    fn bars(
        zone: &UtcOffsetSchedule,
        date: (i64, u32, u32),
        from: (i64, i64),
        to: (i64, i64),
        step: i64,
    ) -> Vec<i64> {
        let day = days_from_civil(date.0, date.1, date.2).unwrap() * 86_400;
        (from.0 * 60 + from.1..=to.0 * 60 + to.1)
            .step_by(step as usize)
            .map(|minute| zone.to_utc(day + minute * 60))
            .collect()
    }

    fn line_chart(times: &[i64]) -> ChartEngine {
        let mut chart = ChartEngine::new(1_600.0, 500.0, 1.0);
        chart.series[0].kind = SeriesKind::Line;
        let times: Vec<f64> = times.iter().map(|&time| time as f64).collect();
        let values: Vec<f64> = (0..times.len()).map(|i| 100.0 + i as f64).collect();
        chart
            .set_series_data(0, &times, &values, &values, &values, &values)
            .unwrap();
        chart.time_scale.set_width(1_600.0);
        chart.fit_content();
        chart.set_time_visible(true);
        chart
    }

    fn crosshair_label(chart: &mut ChartEngine, index: usize) -> String {
        let x = chart.time_scale.index_to_coordinate(index as i64);
        chart.set_crosshair_at(x, 100.0);
        chart
            .build_axis_frame(
                80.0,
                |t, _| t.len() as f64 * 7.0,
                |t, _| t.len() as f64 * 6.0,
            )
            .labels
            .into_iter()
            .find(|label| label.midpoint == AxisTextMidpoint::StableTime)
            .map(|label| label.text)
            .expect("crosshair time label")
    }

    fn tick_labels(chart: &mut ChartEngine) -> Vec<String> {
        chart.clear_crosshair_at();
        chart
            .build_axis_frame(
                80.0,
                |t, _| t.len() as f64 * 7.0,
                |t, _| t.len() as f64 * 6.0,
            )
            .labels
            .into_iter()
            .filter(|label| {
                label.align == AxisTextAlign::Center
                    && label.midpoint == AxisTextMidpoint::None
                    && label.background.is_none()
            })
            .map(|label| label.text)
            .collect()
    }

    fn day_marks(chart: &mut ChartEngine) -> Vec<usize> {
        let mut marks: Vec<usize> = chart
            .time_marks(0.001)
            .into_iter()
            .filter(|&(index, weight)| index > 0 && weight >= TickMarkWeight::Day as u8)
            .map(|(index, _)| index as usize)
            .collect();
        marks.sort_unstable();
        marks
    }

    fn output(chart: &ChartEngine, id: crate::SeriesId) -> Vec<f64> {
        chart.data.series_data(id).unwrap().1[3].to_vec()
    }

    #[test]
    fn a_share_lunch_break_is_gapless_with_shanghai_labels_and_day_marks() {
        let zone = shanghai();
        let mut times = Vec::new();
        for day in [2, 3] {
            times.extend(bars(&zone, (2024, 1, day), (9, 30), (11, 30), 1));
            times.extend(bars(&zone, (2024, 1, day), (13, 0), (15, 0), 1));
        }
        let mut chart = line_chart(&times);
        // Installing the zone after the data rebuilds weights and labels.
        chart.set_time_zone(zone);
        let lunch_close = 120; // 11:30
        let afternoon_open = 121; // 13:00
        let spacing = chart.time_scale.bar_spacing();
        let x = |time: i64| chart.time_to_coordinate(time as f64).unwrap();
        assert!(
            (x(times[afternoon_open]) - x(times[lunch_close]) - spacing).abs() < 1e-9,
            "11:30 -> 13:00 must be one bar apart"
        );
        assert_eq!(crosshair_label(&mut chart, 0), "02 Jan '24   09:30");
        assert_eq!(
            crosshair_label(&mut chart, lunch_close),
            "02 Jan '24   11:30"
        );
        assert_eq!(
            crosshair_label(&mut chart, afternoon_open),
            "02 Jan '24   13:00"
        );
        assert_eq!(crosshair_label(&mut chart, 241), "02 Jan '24   15:00");
        let per_day = times.len() / 2;
        assert_eq!(day_marks(&mut chart), vec![per_day]);
        let ticks = tick_labels(&mut chart);
        assert!(!ticks.is_empty());
        for text in ticks {
            if let Some((hour, minute)) = text.split_once(':') {
                let minutes = hour.parse::<i64>().unwrap() * 60 + minute.parse::<i64>().unwrap();
                assert!(
                    (9 * 60 + 30..=15 * 60).contains(&minutes),
                    "tick {text} is outside the Shanghai session"
                );
            }
        }
        // Resetting to UTC restores the historical labels.
        chart.set_time_zone(UtcOffsetSchedule::utc());
        assert_eq!(crosshair_label(&mut chart, 0), "02 Jan '24   01:30");
    }

    #[test]
    fn countdown_anchors_on_the_forming_bar_when_future_session_slots_are_whitespace() {
        // 分时 layout: every minute of the session is installed up front and the minutes that
        // have not traded yet are whitespace. The forming bar is the last traded minute, not the
        // final whitespace slot at the close.
        let zone = shanghai();
        let mut times = bars(&zone, (2024, 1, 2), (9, 30), (11, 30), 1);
        times.extend(bars(&zone, (2024, 1, 2), (13, 1), (15, 0), 1));
        let traded = 31; // 09:30 through 10:00
        let mut chart = ChartEngine::new(1_600.0, 500.0, 1.0);
        chart.series[0].kind = SeriesKind::Line;
        let slot_times: Vec<f64> = times.iter().map(|&time| time as f64).collect();
        let values: Vec<f64> = (0..times.len())
            .map(|i| {
                if i < traded {
                    100.0 + i as f64
                } else {
                    f64::NAN
                }
            })
            .collect();
        chart
            .set_series_data(0, &slot_times, &values, &values, &values, &values)
            .unwrap();
        chart.set_time_zone(zone);
        chart.series[0].countdown_visible = true;
        let forming = times[traded - 1];
        chart.now_override = Some((forming + 45) as f64);
        assert_eq!(chart.series_countdown_text(0).as_deref(), Some("00:15"));
        // After the forming minute ends with no new trade the countdown hides, as for any bar.
        chart.now_override = Some((forming + 90) as f64);
        assert_eq!(chart.series_countdown_text(0), None);
        // Pre-open (every slot whitespace): no forming bar, no countdown.
        let empty = vec![f64::NAN; times.len()];
        chart
            .set_series_data(0, &slot_times, &empty, &empty, &empty, &empty)
            .unwrap();
        chart.now_override = Some((times[0] + 30) as f64);
        assert_eq!(chart.series_countdown_text(0), None);
    }

    #[test]
    fn hk_half_day_countdown_hides_after_the_early_close() {
        let zone = shanghai(); // Hong Kong shares UTC+8.
        let mut times = bars(&zone, (2024, 12, 23), (9, 30), (15, 59), 1);
        let half_day = bars(&zone, (2024, 12, 24), (9, 30), (11, 59), 1);
        let last = *half_day.last().unwrap();
        times.extend(half_day);
        let mut chart = line_chart(&times);
        chart.set_time_zone(zone);
        chart.series[0].countdown_visible = true;
        chart.now_override = Some((last + 30) as f64);
        assert_eq!(chart.series_countdown_text(0).as_deref(), Some("00:30"));
        // 12:00 early close and later: no forming bar, no countdown.
        for after in [60, 1_800, 4 * HOUR] {
            chart.now_override = Some((last + after) as f64);
            assert_eq!(chart.series_countdown_text(0), None);
        }
        assert_eq!(
            crosshair_label(&mut chart, times.len() - 1),
            "24 Dec '24   11:59"
        );
    }

    #[test]
    fn us_dst_week_opens_at_0930_eastern_on_both_sides() {
        let zone = new_york();
        let mut times = bars(&zone, (2024, 3, 8), (9, 30), (16, 0), 30);
        let monday = times.len();
        times.extend(bars(&zone, (2024, 3, 11), (9, 30), (16, 0), 30));
        let mut chart = line_chart(&times);
        chart.set_time_zone(zone);
        assert_eq!(crosshair_label(&mut chart, 0), "08 Mar '24   09:30");
        assert_eq!(crosshair_label(&mut chart, monday), "11 Mar '24   09:30");
        assert_eq!(times[0] % 86_400, 14 * HOUR + 30 * 60);
        assert_eq!(times[monday] % 86_400, 13 * HOUR + 30 * 60);
        assert_eq!(day_marks(&mut chart), vec![monday]);
    }

    #[test]
    fn us_extended_hours_in_winter_keep_one_session_for_ticks_and_vwap() {
        let zone = new_york();
        let mut times = bars(&zone, (2024, 1, 8), (4, 0), (20, 0), 60);
        let next = times.len();
        times.extend(bars(&zone, (2024, 1, 9), (4, 0), (20, 0), 60));
        let mut chart = line_chart(&times);
        let vwap = chart.add_vwap(0, None).unwrap();
        // UTC boundaries split the 19:00 ET bar (00:00 UTC) into a new day.
        let utc_vwap = output(&chart, vwap);
        assert_eq!(utc_vwap[15], 115.0);
        assert!(day_marks(&mut chart).contains(&15));

        chart.set_time_zone(zone);
        assert_eq!(day_marks(&mut chart), vec![next]);
        let eastern = output(&chart, vwap);
        // Unit weights: the session average of 100..=115 and a fresh session at 04:00 ET.
        assert_eq!(eastern[15], 107.5);
        assert_eq!(eastern[next], 100.0 + next as f64);
    }

    #[test]
    fn china_futures_night_session_starts_the_trading_day() {
        let zone = shanghai();
        let mut times = bars(&zone, (2024, 1, 2), (13, 30), (15, 0), 30);
        let night = times.len();
        times.extend(bars(&zone, (2024, 1, 2), (21, 0), (23, 0), 30));
        let day_session = times.len();
        times.extend(bars(&zone, (2024, 1, 3), (9, 0), (11, 30), 30));
        let mut chart = line_chart(&times);
        let vwap = chart.add_vwap(0, None).unwrap();
        chart.set_time_zone(zone);
        chart.set_session_start_seconds(-3 * HOUR as i32).unwrap();
        assert_eq!(day_marks(&mut chart), vec![night]);
        let values = output(&chart, vwap);
        // The session resets at 21:00, not at the 09:00 day session.
        assert_eq!(values[night], 100.0 + night as f64);
        assert_ne!(values[day_session], 100.0 + day_session as f64);
        // The crosshair keeps the wall clock of the night bar.
        assert_eq!(crosshair_label(&mut chart, night), "02 Jan '24   21:00");
        assert!(chart.set_session_start_seconds(86_400).is_err());
        assert_eq!(
            chart.exchange_time().session_start_seconds(),
            -3 * HOUR as i32
        );
    }

    #[test]
    fn weekly_vwap_bands_reset_on_monday() {
        // Calendar dates Thu 2024-01-04 .. Tue 2024-01-09.
        let first = days_from_civil(2024, 1, 4).unwrap() * 86_400;
        let times: Vec<i64> = (0..6).map(|index| first + index * 86_400).collect();
        let mut chart = line_chart(&times);
        chart.set_calendar_date_axis(true);
        chart.set_time_zone(new_york());
        let outputs = chart.add_vwap_bands(
            0,
            None,
            aeris_charts_indicators::VwapReset::Weekly,
            1.0,
            1.0,
        );
        let basis = output(&chart, outputs[0]);
        assert_eq!(basis, vec![100.0, 100.5, 101.0, 101.5, 104.0, 104.5]);
    }

    #[test]
    fn calendar_dates_keep_their_date_and_trading_day_countdown() {
        let first = days_from_civil(2024, 1, 8).unwrap() * 86_400;
        let times: Vec<i64> = (0..3).map(|index| first + index * 86_400).collect();
        let mut chart = line_chart(&times);
        chart.set_time_visible(false);
        chart.set_time_zone(new_york());
        // Treated as instants, UTC midnight is the previous evening in New York.
        assert_eq!(crosshair_label(&mut chart, 0), "07 Jan '24");
        chart.set_calendar_date_axis(true);
        assert_eq!(crosshair_label(&mut chart, 0), "08 Jan '24");
        // The Jan 10 daily bar forms during the New York trading day.
        chart.series[0].countdown_visible = true;
        let jan10 = times[2];
        chart.now_override = Some((jan10 + 20 * HOUR) as f64); // 15:00 ET
        assert_eq!(chart.series_countdown_text(0).as_deref(), Some("09:00:00"));
        chart.now_override = Some((jan10 + 30 * HOUR) as f64); // Jan 11 01:00 ET
        assert_eq!(chart.series_countdown_text(0), None);
        chart.now_override = Some((jan10 + 2 * HOUR) as f64); // Jan 9 21:00 ET
        assert_eq!(chart.series_countdown_text(0), None);
    }

    #[test]
    fn calendar_countdown_covers_friday_night_sessions_and_whole_months() {
        // China futures daily bars: Monday's trading day opens with Friday's 21:00 night session.
        let tuesday = days_from_civil(2024, 1, 2).unwrap() * 86_400;
        let times: Vec<i64> = [0, 1, 2, 3, 6] // Tue..Fri, Mon
            .iter()
            .map(|day| tuesday + day * 86_400)
            .collect();
        let mut chart = line_chart(&times);
        chart.set_time_visible(false);
        chart.set_time_zone(shanghai());
        chart.set_session_start_seconds(-3 * HOUR as i32).unwrap();
        chart.set_calendar_date_axis(true);
        chart.series[0].countdown_visible = true;
        let friday_night = bars(&shanghai(), (2024, 1, 5), (21, 30), (21, 30), 1)[0];
        chart.now_override = Some(friday_night as f64);
        // Monday 21:00 CST closes Monday's trading day: 3 days minus 30 minutes away.
        assert_eq!(chart.series_countdown_text(0).as_deref(), Some("2d 23h"));

        // Monthly bars (Jan..May 1, median spacing 30.5 days): May's bar forms until June 1.
        let months: Vec<i64> = (1..=5)
            .map(|month| days_from_civil(2024, month, 1).unwrap() * 86_400)
            .collect();
        let mut monthly = line_chart(&months);
        monthly.set_calendar_date_axis(true);
        monthly.series[0].countdown_visible = true;
        let june = days_from_civil(2024, 6, 1).unwrap() * 86_400;
        monthly.now_override = Some((june - 11 * HOUR) as f64); // May 31 13:00 UTC
        assert_eq!(
            monthly.series_countdown_text(0).as_deref(),
            Some("11:00:00")
        );
        monthly.now_override = Some(june as f64);
        assert_eq!(monthly.series_countdown_text(0), None);
    }

    #[test]
    fn time_zone_options_round_trip_and_reject_atomically() {
        let mut chart = line_chart(&[0, 60, 120]);
        chart
            .apply_options(
                r#"{"timeScale":{"timeZone":[{"from_utc_seconds":0,"offset_seconds":28800}],"sessionStart":-10800}}"#,
            )
            .unwrap();
        assert_eq!(chart.exchange_time().offsets(), &shanghai());
        assert_eq!(chart.exchange_time().session_start_seconds(), -10_800);
        let options: serde_json::Value =
            serde_json::from_str(&chart.time_scale_options_json()).unwrap();
        assert_eq!(options["session_start"], -10_800);
        assert_eq!(options["time_zone"][0]["offset_seconds"], 28_800);
        assert_eq!(
            chart.options.value()["timeScale"]["timeZone"][0]["offset_seconds"],
            28_800
        );

        // Invalid exchange-time keys reject the whole patch before anything mutates.
        let before = chart.options.value().clone();
        for invalid in [
            r##"{"timeScale":{"timeZone":"Asia/Shanghai","borderColor":"#123456"}}"##,
            r##"{"timeScale":{"sessionStart":90000,"borderColor":"#123456"}}"##,
            r##"{"timeScale":{"sessionStart":1.5}}"##,
            r##"{"timeScale":{"timeZone":[{"from_utc_seconds":10,"offset_seconds":0},{"from_utc_seconds":5,"offset_seconds":3600}]}}"##,
        ] {
            assert!(chart.apply_options(invalid).is_err(), "{invalid}");
        }
        assert_eq!(chart.options.value(), &before);
        assert_eq!(chart.exchange_time().offsets(), &shanghai());

        chart
            .apply_options(r#"{"timeScale":{"timeZone":"UTC","sessionStart":0}}"#)
            .unwrap();
        assert!(chart.exchange_time().is_utc_identity());
        assert_eq!(chart.options.value()["timeScale"]["timeZone"], "UTC");
    }

    #[test]
    fn sequence_axis_weights_follow_exchange_time() {
        use crate::{
            AggressorSide, FootprintAggregationOptions, FootprintBarAggregation,
            FootprintSeriesOptions, FootprintTrade,
        };
        let mut chart = ChartEngine::new(800.0, 420.0, 1.0);
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
        // One-trade bars at 18:00, 19:00 (00:00 UTC) and 20:00 ET on a winter evening.
        let zone = new_york();
        let trades = bars(&zone, (2024, 1, 8), (18, 0), (20, 0), 60)
            .into_iter()
            .map(|time| FootprintTrade {
                timestamp_micros: time * 1_000_000,
                price: 100.0,
                volume: 1.0,
                aggressor: AggressorSide::Buy,
                bid: None,
                ask: None,
                sequence: None,
                trade_id: None,
                conditions: 0,
                session_id: Some(1),
            })
            .collect();
        chart.set_footprint_trades(id, trades).unwrap();
        assert!(chart.sequence_points().is_some());
        chart.time_scale.set_width(800.0);
        assert!(
            day_marks(&mut chart).contains(&1),
            "UTC puts a Day mark at 00:00 UTC"
        );
        chart.set_time_zone(zone);
        assert_eq!(day_marks(&mut chart), Vec::<usize>::new());
    }

    #[test]
    fn v2_persistence_round_trips_the_exchange_time_options() {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        chart
            .add_pane_with_domain(
                true,
                crate::HorizontalDomain::Category {
                    scale: crate::CategoryScaleType::Band,
                },
            )
            .unwrap();
        chart.set_time_zone(new_york());
        chart.set_session_start_seconds(-6 * HOUR as i32).unwrap();
        let document = chart.export_state_json().unwrap();
        let mut restored = ChartEngine::new(800.0, 500.0, 1.0);
        restored.import_state_json(&document).unwrap();
        assert_eq!(restored.exchange_time().offsets(), &new_york());
        assert_eq!(
            restored.exchange_time().session_start_seconds(),
            -6 * HOUR as i32
        );

        // A document from before these keys existed: a fresh chart stays UTC, and a chart whose
        // host already installed a zone keeps it and still reports it in its options and exports.
        let mut legacy: serde_json::Value = serde_json::from_str(&document).unwrap();
        let time_scale = legacy["chart_options"]["timeScale"]
            .as_object_mut()
            .unwrap();
        time_scale.remove("timeZone");
        time_scale.remove("sessionStart");
        let legacy = legacy.to_string();
        let mut fresh = ChartEngine::new(800.0, 500.0, 1.0);
        fresh.import_state_json(&legacy).unwrap();
        assert!(fresh.exchange_time().is_utc_identity());
        let mut zoned = ChartEngine::new(800.0, 500.0, 1.0);
        zoned.set_time_zone(shanghai());
        zoned.import_state_json(&legacy).unwrap();
        assert_eq!(zoned.exchange_time().offsets(), &shanghai());
        assert_eq!(
            zoned.options.value()["timeScale"]["timeZone"][0]["offset_seconds"],
            28_800
        );
        let exported: serde_json::Value =
            serde_json::from_str(&zoned.export_state_json().unwrap()).unwrap();
        assert_eq!(
            exported["chart_options"]["timeScale"]["timeZone"][0]["offset_seconds"],
            28_800
        );
    }

    #[test]
    fn host_time_formatter_overrides_built_in_exchange_time_text() {
        let zone = shanghai();
        let times = bars(&zone, (2024, 1, 2), (9, 30), (10, 30), 1);
        let mut chart = line_chart(&times);
        chart.set_time_zone(zone);
        chart.set_time_formatter(Some(Box::new(|time| Some(format!("H{time}")))));
        assert_eq!(crosshair_label(&mut chart, 0), format!("H{}", times[0]));
        chart.set_time_formatter(None);
        assert_eq!(crosshair_label(&mut chart, 0), "02 Jan '24   09:30");
    }

    #[test]
    fn rectangle_axis_tags_use_exchange_dates_and_the_host_formatter() {
        // 2024-01-01 20:00 UTC is already Jan 2 in Shanghai.
        let first = days_from_civil(2024, 1, 1).unwrap() * 86_400 + 20 * HOUR;
        let times: Vec<i64> = (0..20).map(|index| first + index * 60).collect();
        let mut chart = line_chart(&times);
        let rectangle = chart
            .add_drawing(
                crate::DrawingKind::Rectangle,
                0,
                vec![
                    crate::DrawingPoint {
                        logical: 2.0,
                        price: 105.0,
                    },
                    crate::DrawingPoint {
                        logical: 8.0,
                        price: 110.0,
                    },
                ],
                Some("{\"show_labels\":true}"),
            )
            .unwrap();
        chart.set_selected_drawing(Some(rectangle));
        let tags = |chart: &mut ChartEngine| -> Vec<String> {
            chart.clear_crosshair_at();
            chart
                .build_axis_frame(
                    80.0,
                    |t, _| t.len() as f64 * 7.0,
                    |t, _| t.len() as f64 * 6.0,
                )
                .labels
                .into_iter()
                .filter(|label| {
                    label.midpoint == AxisTextMidpoint::None && label.background.is_some()
                })
                .map(|label| label.text)
                .collect()
        };
        assert!(tags(&mut chart).contains(&"1/1/2024".to_string()));
        chart.set_time_zone(shanghai());
        let exchange = tags(&mut chart);
        assert!(exchange.contains(&"1/2/2024".to_string()), "{exchange:?}");
        chart.set_time_formatter(Some(Box::new(|time| Some(format!("H{time}")))));
        let hosted = tags(&mut chart);
        assert!(hosted.contains(&format!("H{}", times[2])), "{hosted:?}");
    }
}
