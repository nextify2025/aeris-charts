//! Exchange time zone, trading-day session start, and calendar-date axis state.
//!
//! The chart keeps canonical UTC seconds everywhere. These settings only change how instants are
//! grouped into trading days and presented: tick weights, built-in time labels, VWAP/pivot resets,
//! session highlighting, and the countdown window all read the same [`ExchangeTime`]. Hosts supply
//! an explicit UTC-offset schedule or name a zone from the TradingView parity list, which
//! `ChartTimeZone` resolves once into one; the engine never consults a platform time zone.

use aeris_charts_core::scale::exchange_time::{ExchangeTime, UtcOffsetSchedule};
use aeris_charts_core::scale::time_tick_marks::fill_weights_for_points_shifted_in;

use crate::{ChartEngine, ChartTimeZone, ExchangeTimeError, UtcOffsetTransition};

/// Parsed, validated `timeScale.timeZone` / `timeScale.sessionStart` keys and top-level `timezone`
/// name of an options patch.
#[derive(Debug, Default)]
pub(crate) struct ExchangeTimePatch {
    offsets: Option<UtcOffsetSchedule>,
    /// The `timezone` name (a TradingView parity id), validated.
    named: Option<ChartTimeZone>,
    /// The named zone's own schedule, resolved only when no explicit schedule travels with it.
    named_schedule: Option<UtcOffsetSchedule>,
    session_start_seconds: Option<i32>,
}

impl ExchangeTimePatch {
    /// The trading-day start this patch installs, if it carries one.
    pub(crate) fn session_start_seconds(&self) -> Option<i32> {
        self.session_start_seconds
    }
}

fn parse_time_zone(value: &serde_json::Value) -> Result<UtcOffsetSchedule, String> {
    if let Some(name) = value.as_str() {
        return if matches!(name, "UTC" | "Etc/UTC") {
            Ok(UtcOffsetSchedule::utc())
        } else {
            Err(format!(
                "timeScale.timeZone {name:?} must be \"UTC\" or an explicit offset schedule; name a zone through the top-level timezone option"
            ))
        };
    }
    let transitions: Vec<UtcOffsetTransition> = serde_json::from_value(value.clone())
        .map_err(|error| format!("invalid timeScale.timeZone schedule: {error}"))?;
    UtcOffsetSchedule::new(transitions).map_err(|error| error.to_string())
}

/// A saved document's `chart_options` as this engine can restore them. A top-level `timezone` that
/// does not name a supported zone (a raw value an earlier build stored, or a non-string) is
/// dropped so the rest of the layout still restores; a live patch keeps rejecting it atomically.
pub(crate) fn importable_chart_options(options: &serde_json::Value) -> serde_json::Value {
    let mut options = options.clone();
    let usable = match options.get("timezone") {
        None | Some(serde_json::Value::Null) => true,
        Some(serde_json::Value::String(id)) => ChartTimeZone::parse(id).is_some(),
        Some(_) => false,
    };
    if !usable && let Some(map) = options.as_object_mut() {
        map.remove("timezone");
    }
    options
}

impl ChartEngine {
    /// The chart's exchange time: offset schedule, session start, and calendar-date flag.
    pub fn exchange_time(&self) -> &ExchangeTime {
        &self.exchange_time
    }

    /// Install the exchange time zone as a validated UTC-offset schedule (UTC by default). Tick
    /// weights, built-in labels, trading-day indicator resets, session highlighting, and the
    /// countdown follow it. The schedule is mirrored into the options store so option snapshots
    /// and V2 persistence round-trip it. An explicit schedule clears the named zone: general
    /// temporal axes and the clock then stay UTC and [`Self::time_zone_id`] reports `custom`.
    pub fn set_exchange_offsets(&mut self, offsets: UtcOffsetSchedule) {
        self.install_time_zone(ChartTimeZone::default(), offsets);
    }

    /// Select one of [`crate::TRADINGVIEW_TIME_ZONES`] by IANA id. The zone is resolved once into
    /// the same offset schedule [`Self::set_exchange_offsets`] takes, so tick weights, labels,
    /// period resets, sessions and the countdown follow it exactly like the explicit schedule;
    /// general temporal axes and [`Self::time_zone_clock_text`] follow the name. `Ok(false)` when
    /// the zone is already installed.
    pub fn set_time_zone(&mut self, value: &str) -> Result<bool, String> {
        let zone = ChartTimeZone::parse(value)
            .ok_or_else(|| format!("unsupported IANA time zone: {value}"))?;
        if self.zone_installed(zone) {
            return Ok(false);
        }
        let offsets = zone.offset_schedule().map_err(|error| error.to_string())?;
        self.install_time_zone(zone, offsets);
        Ok(true)
    }

    /// Whether `zone` is the named zone already in force (the default zone counts only while the
    /// offsets are UTC, since an explicit schedule leaves the default name behind).
    fn zone_installed(&self, zone: ChartTimeZone) -> bool {
        self.time_zone == zone
            && (zone != ChartTimeZone::default() || self.exchange_time.offsets().is_utc())
    }

    /// The installed named zone (`Etc/UTC` by default), or `custom` when an explicit schedule that
    /// no named zone produced is installed. Canonical data and public timestamps remain UTC.
    pub fn time_zone_id(&self) -> &'static str {
        if self.time_zone == ChartTimeZone::default() && !self.exchange_time.offsets().is_utc() {
            "custom"
        } else {
            self.time_zone.id()
        }
    }

    /// Install a schedule together with the named zone that produced it (the default zone for an
    /// explicit schedule).
    fn install_time_zone(&mut self, zone: ChartTimeZone, offsets: UtcOffsetSchedule) {
        let zone_changed = self.time_zone != zone;
        self.time_zone = zone;
        if self.exchange_time.offsets() != &offsets {
            self.exchange_time.set_offsets(offsets);
            self.exchange_time_changed();
        } else if zone_changed {
            // General temporal axes and the clock follow the named zone.
            self.invalidate_frame_all();
        }
        self.mirror_exchange_time_options();
    }

    /// Seconds from local midnight at which the exchange trading day begins (default 0). Negative
    /// values assign an evening session to the next trading day. Rejects values outside ±1 day
    /// and, while a close-time bar label with session windows is installed, a start those
    /// windows cannot be placed on ([`ExchangeTimeError::BarTimeLabelWindows`]); a rejection
    /// changes nothing.
    pub fn set_session_start_seconds(&mut self, seconds: i32) -> Result<(), ExchangeTimeError> {
        if seconds != self.exchange_time.session_start_seconds() {
            ExchangeTime::new(UtcOffsetSchedule::utc(), seconds)?;
            self.check_bar_time_label_fits(seconds)
                .map_err(ExchangeTimeError::BarTimeLabelWindows)?;
        }
        self.install_session_start(seconds)
    }

    /// Install a session start that was range-checked and validated against the bar time label
    /// that will be in force (an options patch may replace the label in the same step).
    fn install_session_start(&mut self, seconds: i32) -> Result<(), ExchangeTimeError> {
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

    /// Write the live time zone, its name and the session start into the options store, so option
    /// snapshots and V2 persistence always describe the exchange time the chart actually uses
    /// (including after importing a document that predates these keys). The `timezone` name is
    /// written only while a named zone is installed and cleared when an explicit schedule
    /// replaces it, so a persisted name is never stale.
    pub(crate) fn mirror_exchange_time_options(&mut self) {
        let mut patch = serde_json::json!({
            "timeScale": {
                "timeZone": self.time_zone_json(),
                "sessionStart": self.exchange_time.session_start_seconds(),
            }
        });
        if self.time_zone != ChartTimeZone::default() {
            patch["timezone"] = serde_json::json!(self.time_zone.id());
        } else if self
            .options
            .value()
            .get("timezone")
            .is_some_and(|value| !value.is_null())
        {
            patch["timezone"] = serde_json::Value::Null;
        }
        self.options.apply(&patch);
    }

    /// Validate the exchange-time keys of an options patch without mutating anything. Judged against
    /// the state in force now, so a name that is already installed costs nothing.
    pub(crate) fn parse_exchange_time_patch(
        &self,
        patch: &serde_json::Value,
    ) -> Result<ExchangeTimePatch, String> {
        let time_scale = patch
            .get("timeScale")
            .and_then(serde_json::Value::as_object);
        let offsets = time_scale
            .and_then(|time_scale| time_scale.get("timeZone"))
            .map(parse_time_zone)
            .transpose()?;
        let session_start_seconds = time_scale
            .and_then(|time_scale| time_scale.get("sessionStart"))
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
        // The top-level `timezone` option names a TradingView parity zone. It is validated like
        // every other rejectable key; its own schedule is resolved only when `timeScale.timeZone`
        // does not carry one (a V2 document carries both, written together).
        let named = match patch.get("timezone") {
            None | Some(serde_json::Value::Null) => None,
            Some(serde_json::Value::String(id)) => Some(
                ChartTimeZone::parse(id)
                    .ok_or_else(|| format!("unsupported IANA time zone: {id}"))?,
            ),
            Some(_) => return Err("timezone must be an IANA time zone id string".to_string()),
        };
        let named_schedule = match (named, &offsets) {
            // A host may resend the installed name with every patch: resolve nothing for it.
            (Some(zone), None) if !self.zone_installed(zone) => {
                Some(zone.offset_schedule().map_err(|error| error.to_string())?)
            }
            _ => None,
        };
        Ok(ExchangeTimePatch {
            offsets,
            named,
            named_schedule,
            session_start_seconds,
        })
    }

    /// Install a validated patch. The session start goes first: the bar time label the patch
    /// installs was validated for it, so the label grid must never be placed at the old start.
    pub(crate) fn apply_exchange_time_patch(&mut self, patch: ExchangeTimePatch) {
        if let Some(seconds) = patch.session_start_seconds {
            self.install_session_start(seconds)
                .expect("session start was validated before the options patch applied");
        }
        match (patch.offsets, patch.named, patch.named_schedule) {
            (Some(offsets), Some(zone), _) => self.install_time_zone(zone, offsets),
            (Some(offsets), None, _) => self.set_exchange_offsets(offsets),
            (None, Some(zone), Some(offsets)) => self.install_time_zone(zone, offsets),
            (None, _, _) => {}
        }
    }

    fn exchange_time_changed(&mut self) {
        self.rebuild_bar_label_grid();
        self.rebuild_tick_weights();
        self.rebuild_trading_day_indicators();
        self.refresh_trade_stream_sessions();
        self.invalidate_frame_all();
    }

    /// Recompute every axis weight in the current exchange time without touching view state. The
    /// column covers the display-only projected labels on both sides of the data (negative
    /// logical indices for past labels) and the non-time sequence axis.
    pub(crate) fn rebuild_tick_weights(&mut self) {
        let (start_index, times) = self.axis_tick_times();
        let mut weights = vec![0u8; times.len()];
        fill_weights_for_points_shifted_in(
            &times,
            &mut weights,
            0,
            self.tick_label_shift(),
            &self.exchange_time,
        );
        self.tick_marks.set_weights_from(start_index, &weights);
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
    use aeris_charts_core::scale::time_tick_marks::{TickMarkWeight, days_from_civil};

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

    /// America/Chicago around 2024 (CST, CDT from 2024-03-10 08:00 UTC, CST from 2024-11-03).
    fn chicago() -> UtcOffsetSchedule {
        let at = |y, m, d, h: i64| days_from_civil(y, m, d).unwrap() * 86_400 + h * HOUR;
        UtcOffsetSchedule::new(vec![
            UtcOffsetTransition {
                from_utc_seconds: at(2023, 11, 5, 7),
                offset_seconds: -6 * HOUR as i32,
            },
            UtcOffsetTransition {
                from_utc_seconds: at(2024, 3, 10, 8),
                offset_seconds: -5 * HOUR as i32,
            },
            UtcOffsetTransition {
                from_utc_seconds: at(2024, 11, 3, 7),
                offset_seconds: -6 * HOUR as i32,
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
        chart.set_exchange_offsets(zone);
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
        chart.set_exchange_offsets(UtcOffsetSchedule::utc());
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
        chart.set_exchange_offsets(zone);
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
        chart.set_exchange_offsets(zone);
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
        chart.set_exchange_offsets(zone);
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

        chart.set_exchange_offsets(zone);
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
        chart.set_exchange_offsets(zone);
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
    fn china_futures_friday_night_and_holiday_reset() {
        let vwap_reset = |times: &[i64]| {
            let mut chart = line_chart(times);
            let vwap = chart.add_vwap(0, None).unwrap();
            chart.set_exchange_offsets(shanghai());
            chart.set_session_start_seconds(-3 * HOUR as i32).unwrap();
            let marks = day_marks(&mut chart);
            (marks, output(&chart, vwap))
        };

        // Friday's day session, then its 21:00 night session, then Monday's day session: the
        // night session opens Monday's trading day, so the only Day mark is at 21:00 and the
        // Monday morning continues the session.
        let mut times = bars(&shanghai(), (2024, 1, 5), (13, 30), (15, 0), 30);
        assert_eq!(times.len(), 4);
        times.extend(bars(&shanghai(), (2024, 1, 5), (21, 0), (23, 0), 30));
        times.extend(bars(&shanghai(), (2024, 1, 8), (9, 0), (10, 0), 30));
        let (marks, vwap) = vwap_reset(&times);
        assert_eq!(marks, vec![4]);
        assert_eq!(vwap[4], 104.0);
        // The session average of the night bars and the Monday morning: 104..=109.
        assert_eq!(vwap[9], 106.5);

        // A host calendar break: Sep 30 14:30-15:00, then the first date after the break opens
        // with its day session (no night session before it), then the night session of the next
        // trading day, then that day's session.
        let mut times = bars(&shanghai(), (2024, 9, 30), (14, 30), (15, 0), 30);
        times.extend(bars(&shanghai(), (2024, 10, 8), (9, 0), (10, 0), 30));
        times.extend(bars(&shanghai(), (2024, 10, 8), (21, 0), (22, 0), 30));
        times.extend(bars(&shanghai(), (2024, 10, 9), (9, 0), (9, 30), 30));
        let (marks, vwap) = vwap_reset(&times);
        assert_eq!(marks, vec![2, 5]);
        assert_eq!(vwap[2], 102.0);
        assert_eq!(vwap[5], 105.0);
        // The day session after the night session does not reset again.
        assert_eq!(vwap[8], 106.5);
    }

    #[test]
    fn cme_sunday_open_starts_monday_and_weekly_vwap_resets() {
        // Globex: the week opens Sunday 17:00 Central. Bars: Friday afternoon (idx 0-1), the
        // Sunday evening open (2-3), Monday early morning (4-5) and afternoon (6), and Monday
        // evening (7-8).
        let zone = chicago();
        let mut times = bars(&zone, (2024, 1, 5), (14, 0), (15, 0), 60);
        times.extend(bars(&zone, (2024, 1, 7), (17, 0), (18, 0), 60));
        times.extend(bars(&zone, (2024, 1, 8), (0, 0), (1, 0), 60));
        times.extend(bars(&zone, (2024, 1, 8), (15, 0), (15, 0), 60));
        times.extend(bars(&zone, (2024, 1, 8), (17, 0), (18, 0), 60));
        assert_eq!(times.len(), 9);
        let mut chart = line_chart(&times);
        let session = chart.add_vwap(0, None).unwrap();
        let weekly = chart.add_vwap_bands(
            0,
            None,
            aeris_charts_indicators::VwapReset::Weekly,
            1.0,
            1.0,
        )[0];
        chart.set_exchange_offsets(zone);

        // A -7 h start makes the Sunday 17:00 open Monday's trading day: one Day mark where the
        // Friday afternoon ends, one where Monday's evening (Tuesday's trading day) opens, and
        // the session and weekly VWAP reset at the open rather than at Monday midnight.
        chart.set_session_start_seconds(-7 * HOUR as i32).unwrap();
        assert_eq!(day_marks(&mut chart), vec![2, 7]);
        let values = output(&chart, session);
        assert_eq!(values[2], 102.0);
        assert_eq!(values[6], 104.0); // the average of 102..=106
        assert_eq!(values[7], 107.0);
        let basis = output(&chart, weekly);
        assert_eq!(basis[2], 102.0);
        assert_eq!(basis[8], 105.0); // 102..=108: Sunday open, Monday and Tuesday's evening
        assert_eq!(crosshair_label(&mut chart, 2), "07 Jan '24   17:00");

        // With a midnight start the Sunday block is its own trading day: a Day mark appears at
        // Monday midnight in the middle of the session, and the weekly reset waits for Monday's
        // calendar date (one session late).
        chart.set_session_start_seconds(0).unwrap();
        assert_eq!(day_marks(&mut chart), vec![2, 4]);
        let basis = output(&chart, weekly);
        assert_eq!(basis[2], 101.0, "Sunday stays in the week of Friday");
        assert_eq!(basis[4], 104.0);
    }

    #[test]
    fn weekly_vwap_bands_reset_on_monday() {
        // Calendar dates Thu 2024-01-04 .. Tue 2024-01-09.
        let first = days_from_civil(2024, 1, 4).unwrap() * 86_400;
        let times: Vec<i64> = (0..6).map(|index| first + index * 86_400).collect();
        let mut chart = line_chart(&times);
        chart.set_calendar_date_axis(true);
        chart.set_exchange_offsets(new_york());
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
        chart.set_exchange_offsets(new_york());
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
        chart.set_exchange_offsets(shanghai());
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
        chart.set_exchange_offsets(zone);
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
        chart.set_exchange_offsets(new_york());
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
        zoned.set_exchange_offsets(shanghai());
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
        chart.set_exchange_offsets(zone);
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
        chart.set_exchange_offsets(shanghai());
        let exchange = tags(&mut chart);
        assert!(exchange.contains(&"1/2/2024".to_string()), "{exchange:?}");
        chart.set_time_formatter(Some(Box::new(|time| Some(format!("H{time}")))));
        let hosted = tags(&mut chart);
        assert!(hosted.contains(&format!("H{}", times[2])), "{hosted:?}");
    }

    #[test]
    fn a_named_zone_installs_the_clock_its_explicit_schedule_would() {
        let zone = new_york();
        let mut times = bars(&zone, (2024, 3, 8), (9, 30), (16, 0), 30);
        let monday = times.len();
        times.extend(bars(&zone, (2024, 3, 11), (9, 30), (16, 0), 30));
        let mut named = line_chart(&times);
        let mut explicit = line_chart(&times);
        assert_eq!(named.set_time_zone("America/New_York"), Ok(true));
        explicit.set_exchange_offsets(zone.clone());
        assert_eq!(named.time_zone_id(), "America/New_York");
        assert_eq!(explicit.time_zone_id(), "custom");
        assert_eq!(day_marks(&mut named), vec![monday]);
        assert_eq!(day_marks(&mut explicit), vec![monday]);
        assert_eq!(tick_labels(&mut named), tick_labels(&mut explicit));
        assert_eq!(crosshair_label(&mut named, monday), "11 Mar '24   09:30");
        assert_eq!(crosshair_label(&mut explicit, monday), "11 Mar '24   09:30");
        for &time in &times {
            assert_eq!(
                named.exchange_time().offsets().offset_at(time),
                zone.offset_at(time)
            );
        }
    }

    #[test]
    fn a_raw_schedule_clears_the_named_zone_and_the_utc_name_restores_utc() {
        let mut chart = line_chart(&[0, 60, 120]);
        assert_eq!(chart.time_zone_id(), "Etc/UTC");
        assert_eq!(chart.set_time_zone("America/New_York"), Ok(true));
        assert_eq!(chart.options.value()["timezone"], "America/New_York");
        chart.set_exchange_offsets(shanghai());
        assert_eq!(chart.time_zone_id(), "custom");
        assert!(chart.options.value()["timezone"].is_null());
        assert_eq!(chart.set_time_zone("Etc/UTC"), Ok(true));
        assert!(chart.exchange_time().is_utc_identity());
        assert_eq!(chart.set_time_zone("Etc/UTC"), Ok(false));
        assert!(chart.set_time_zone("Mars/Olympus_Mons").is_err());
    }

    #[test]
    fn the_timezone_option_names_a_zone_validates_atomically_and_replays_through_v2() {
        // A general pane makes the export a V2 document, which carries the options store.
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        chart
            .add_pane_with_domain(
                true,
                crate::HorizontalDomain::Category {
                    scale: crate::CategoryScaleType::Band,
                },
            )
            .unwrap();
        chart.apply_options(r#"{"timezone":"Asia/Tokyo"}"#).unwrap();
        assert_eq!(chart.time_zone_id(), "Asia/Tokyo");
        assert_eq!(
            chart.exchange_time().offsets().offset_at(1_700_000_000),
            9 * HOUR as i32
        );
        // Resending the installed name resolves and changes nothing.
        let before = chart.options.value().clone();
        chart.apply_options(r#"{"timezone":"Asia/Tokyo"}"#).unwrap();
        assert_eq!(chart.options.value(), &before);
        for invalid in [
            r#"{"timezone":"Mars/Olympus_Mons","timeScale":{"sessionStart":3600}}"#,
            r#"{"timezone":5}"#,
        ] {
            assert!(chart.apply_options(invalid).is_err(), "{invalid}");
        }
        assert_eq!(chart.options.value(), &before);
        assert_eq!(chart.exchange_time().session_start_seconds(), 0);
        // V2: the name comes back through the options replay; a schedule installed later never
        // leaves a stale name in the exported document.
        let document = chart.export_state_json().unwrap();
        let mut restored = ChartEngine::new(800.0, 500.0, 1.0);
        restored.import_state_json(&document).unwrap();
        assert_eq!(restored.time_zone_id(), "Asia/Tokyo");
        assert_eq!(
            restored.exchange_time().offsets(),
            chart.exchange_time().offsets()
        );
        chart.set_exchange_offsets(shanghai());
        let document = chart.export_state_json().unwrap();
        let mut restored = ChartEngine::new(800.0, 500.0, 1.0);
        restored.import_state_json(&document).unwrap();
        assert_eq!(restored.time_zone_id(), "custom");
        assert_eq!(restored.exchange_time().offsets(), &shanghai());
    }

    #[test]
    fn a_document_naming_an_unresolvable_zone_still_imports_with_the_rest_of_its_options() {
        // Documents saved by builds that stored the raw `timezone` value can carry an id this
        // engine does not know (or a non-string). A live patch rejects it atomically, but a saved
        // layout must still restore: the zone is dropped, everything else applies.
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        chart
            .add_pane_with_domain(
                true,
                crate::HorizontalDomain::Category {
                    scale: crate::CategoryScaleType::Band,
                },
            )
            .unwrap();
        chart
            .apply_options(r#"{"timeScale":{"sessionStart":3600}}"#)
            .unwrap();
        let fresh_id = ChartEngine::new(800.0, 500.0, 1.0).time_zone_id();
        for unresolvable in [
            serde_json::json!("Mars/Olympus_Mons"),
            serde_json::json!("exchange"),
            serde_json::json!(5),
        ] {
            let mut document: serde_json::Value =
                serde_json::from_str(&chart.export_state_json().unwrap()).unwrap();
            document["chart_options"]["timezone"] = unresolvable.clone();
            let mut restored = ChartEngine::new(800.0, 500.0, 1.0);
            restored
                .import_state_json(&document.to_string())
                .unwrap_or_else(|error| panic!("{unresolvable}: {error:?}"));
            assert_eq!(restored.time_zone_id(), fresh_id, "{unresolvable}");
            assert_eq!(
                restored.exchange_time().session_start_seconds(),
                3600,
                "{unresolvable}: the rest of the options applied"
            );
            assert!(
                restored.options.value()["timezone"].is_null(),
                "{unresolvable}: the unusable value is not kept in the options store"
            );
        }
        // A resolvable id in the same position still installs.
        let mut document: serde_json::Value =
            serde_json::from_str(&chart.export_state_json().unwrap()).unwrap();
        document["chart_options"]["timezone"] = serde_json::json!("Asia/Tokyo");
        let mut restored = ChartEngine::new(800.0, 500.0, 1.0);
        restored.import_state_json(&document.to_string()).unwrap();
        assert_eq!(restored.time_zone_id(), "Asia/Tokyo");
    }

    #[test]
    fn projected_labels_weigh_like_real_bars_under_the_exchange_time_and_the_close_label() {
        // A projection's label points must carry the weights real bars at the same times would.
        let zone = shanghai();
        let times = bars(&zone, (2024, 1, 2), (9, 30), (11, 30), 1);
        let last = *times.last().unwrap();
        let mut extended = times.clone();
        extended.extend((1..=3).map(|step| last + 60 * step));
        let mut real = line_chart(&extended);
        let mut projected = line_chart(&times);
        for chart in [&mut real, &mut projected] {
            chart.set_exchange_offsets(zone.clone());
            chart
                .set_bar_time_label(crate::BarTimeLabel::Close {
                    interval_seconds: 60,
                    windows: Vec::new(),
                })
                .unwrap();
        }
        assert!(projected.set_future_time_projection(Some(60), 3));
        assert_eq!(projected.axis_time_key_at(times.len()), Some(last + 60));
        assert_eq!(projected.bar_label_time(last + 60), last + 120);
        let marks = |chart: &mut ChartEngine| {
            let mut marks = chart.time_marks(0.001);
            marks.sort_unstable();
            marks
        };
        assert_eq!(marks(&mut projected), marks(&mut real));
    }

    /// The axis weight column in index order (past labels at negative indices included), without
    /// its first mark: a tail append never re-guesses the extrapolated weight of the first point.
    fn axis_weights(chart: &mut ChartEngine) -> Vec<(i64, u8)> {
        let mut marks: Vec<(i64, u8)> = chart
            .tick_marks
            .build(1.0, 0.0)
            .iter()
            .map(|mark| (mark.index, mark.weight))
            .collect();
        marks.sort_unstable();
        marks.remove(0);
        marks
    }

    #[test]
    fn live_appends_and_trims_keep_the_axis_weights_of_a_clean_rebuild_under_projection() {
        // Four New York sessions across the 2024 spring-forward, 30-minute bars.
        let zone = new_york();
        let mut all = Vec::new();
        for date in [(2024, 3, 7), (2024, 3, 8), (2024, 3, 11), (2024, 3, 12)] {
            all.extend(bars(&zone, date, (9, 30), (16, 0), 30));
        }
        let zones = [UtcOffsetSchedule::utc(), zone, shanghai()];
        let label = |close: bool| {
            if close {
                crate::BarTimeLabel::Close {
                    interval_seconds: 1_800,
                    windows: Vec::new(),
                }
            } else {
                crate::BarTimeLabel::Open
            }
        };
        // The settings the live chart carries: zone, close label, future and past projections.
        let (mut zone_index, mut close) = (0_usize, false);
        let (mut future, mut past) = (0_usize, 0_usize);
        let mut live = line_chart(&all[..20]);
        assert!(live.set_series_max_points(0, Some(30)));
        let mut next = 20;
        let mut seed = 0x9e37_79b9_7f4a_7c15_u64;
        for step in 0..120 {
            seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            match (seed >> 33) % 8 {
                // Streaming dominates, so appends, trims and settings interleave.
                0..=3 if next < all.len() => {
                    assert!(live.update_series_bar(0, all[next] as f64, [100.0; 4]));
                    next += 1;
                }
                4 => {
                    zone_index = (zone_index + 1) % zones.len();
                    live.set_exchange_offsets(zones[zone_index].clone());
                }
                5 => {
                    close = !close;
                    live.set_bar_time_label(label(close)).unwrap();
                }
                6 => {
                    future = (future + 3) % 7;
                    live.set_future_time_projection(Some(1_800), future);
                }
                _ => {
                    past = (past + 2) % 5;
                    live.set_past_time_projection(Some(1_800), past);
                }
            }
            let retained = live.data_layer().merged_times().to_vec();
            let mut clean = line_chart(&retained);
            clean.set_exchange_offsets(zones[zone_index].clone());
            clean.set_bar_time_label(label(close)).unwrap();
            clean.set_future_time_projection(Some(1_800), future);
            clean.set_past_time_projection(Some(1_800), past);
            assert_eq!(
                axis_weights(&mut live),
                axis_weights(&mut clean),
                "step {step}: zone {zone_index}, close {close}, future {future}, past {past}, {} bars",
                retained.len()
            );
        }
        assert!(next > 40, "the run streamed past the cap: {next}");
    }
}
