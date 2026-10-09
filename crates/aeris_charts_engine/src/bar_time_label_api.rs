//! Close-time display labels for bar times.
//!
//! Every bar keeps its OPEN time as its identity: rows, merged times, countdown, replay, sessions,
//! trading days, drawings, resampling, and every time a host passes in or reads back are stamped
//! by the second the bar opens. [`BarTimeLabel::Close`] changes only the time TEXT the chart
//! prints for a bar (crosshair, automatic and default explicit tick labels, drawing axis tags,
//! drawing statistics, the delta tooltip): the bar opened 09:30 with a one-minute interval prints
//! 09:31. The close is the open plus the interval, or the end of the session window that contains
//! the open when the bar is that window's short last bar (an hourly 15:30 bar of a 16:00 session
//! closes 16:00). Session windows are optional; without them every bar prints open plus interval.
//!
//! Hour and minute tick weights follow the printed time so an hour tick lands on the bar that
//! closes on the hour, while day, month and year weights keep following the identity trading day.
//! Non-time sequence axes and calendar-date axes print their own times and ignore the option.
//!
//! The windows and the trading-day start form a pair that is validated whenever either changes:
//! a label, an options patch, a V2 import, or `set_session_start_seconds` that would leave
//! windows the start cannot place is rejected before anything mutates. The chart therefore never
//! holds a state its own validation refuses, and every exported document imports again.

use aeris_charts_core::scale::exchange_time::{ExchangeTime, UtcOffsetSchedule};
use aeris_charts_core::scale::session_slots::{
    parse_wall_clock, OutOfSessionPolicy, SessionBarGrid, SessionSlotError, SessionWindow,
};
use serde::Deserialize;

use crate::ChartEngine;

/// One interval is at most one second short of a day: a daily bar has no close-time label.
const MAX_LABEL_INTERVAL_SECONDS: u32 = 86_399;

/// Which instant of a bar its time text prints.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum BarTimeLabel {
    /// The bar's open time, the canonical bar time.
    #[default]
    Open,
    /// The bar's close time. `interval_seconds` is the chart's primary bar interval (1 second to
    /// just under a day). `windows` are the exchange-local session windows (at most
    /// [`crate::MAX_SESSION_WINDOWS`]) that end each window's last bar exactly; without them a
    /// short last bar prints its open plus the interval.
    Close {
        interval_seconds: u32,
        windows: Vec<SessionWindow>,
    },
}

impl BarTimeLabel {
    /// Seconds the printed time lies after the identity time, for the hour and minute tick
    /// weights. Constant per interval: a short last bar weighs as if it printed open + interval.
    pub(crate) fn weight_shift(&self) -> i64 {
        match self {
            Self::Open => 0,
            Self::Close {
                interval_seconds, ..
            } => i64::from(*interval_seconds),
        }
    }
}

/// The window grid of a close-time label. `None` for an open label and for a close label without
/// windows; an error for an interval outside 1 second..1 day or windows that cannot be placed.
fn label_grid(
    label: &BarTimeLabel,
    time: &ExchangeTime,
) -> Result<Option<SessionBarGrid>, SessionSlotError> {
    let BarTimeLabel::Close {
        interval_seconds,
        windows,
    } = label
    else {
        return Ok(None);
    };
    if !(1..=MAX_LABEL_INTERVAL_SECONDS).contains(interval_seconds) {
        return Err(SessionSlotError::InvalidInterval {
            seconds: *interval_seconds,
        });
    }
    if windows.is_empty() {
        return Ok(None);
    }
    // Only bar closes are asked of the grid, so the out-of-session policy is never consulted.
    SessionBarGrid::new(
        windows.clone(),
        *interval_seconds,
        time,
        OutOfSessionPolicy::Fold,
    )
    .map(Some)
}

/// `"HH:MM"` (or `"HH:MM:SS"` when the seconds are not zero) of exchange-local seconds after
/// midnight; 86,400 is `"24:00"`.
fn wall_clock_text(seconds: u32) -> String {
    let (hour, minute, second) = (seconds / 3_600, seconds % 3_600 / 60, seconds % 60);
    if second == 0 {
        format!("{hour:02}:{minute:02}")
    } else {
        format!("{hour:02}:{minute:02}:{second:02}")
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CloseLabelJson {
    anchor: String,
    interval_seconds: u32,
    #[serde(default)]
    windows: Vec<[String; 2]>,
}

fn parse_close_label(value: &serde_json::Value) -> Result<BarTimeLabel, String> {
    let json: CloseLabelJson = serde_json::from_value(value.clone())
        .map_err(|error| format!("invalid timeScale.barTimeLabel: {error}"))?;
    if json.anchor != "close" {
        return Err(format!(
            "timeScale.barTimeLabel anchor {:?} must be \"close\"",
            json.anchor
        ));
    }
    let windows = json
        .windows
        .iter()
        .enumerate()
        .map(|(index, [start, end])| {
            let start_seconds = parse_wall_clock(start, false);
            let end_seconds = parse_wall_clock(end, true);
            match (start_seconds, end_seconds) {
                (Some(start_seconds), Some(end_seconds)) => Ok(SessionWindow {
                    start_seconds,
                    end_seconds,
                }),
                _ => Err(format!(
                    "timeScale.barTimeLabel window {index} needs \"HH:MM\" or \"HH:MM:SS\" times"
                )),
            }
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(BarTimeLabel::Close {
        interval_seconds: json.interval_seconds,
        windows,
    })
}

/// Check that `label` can be placed on trading days that begin `session_start_seconds` after
/// local midnight. Placement is structural, so the time zone never changes the outcome.
fn validate_label(
    label: &BarTimeLabel,
    session_start_seconds: i32,
) -> Result<(), SessionSlotError> {
    let time = ExchangeTime::new(UtcOffsetSchedule::utc(), session_start_seconds)
        .map_err(|_| SessionSlotError::OutOfRange)?;
    label_grid(label, &time).map(|_| ())
}

impl ChartEngine {
    /// Parse and validate the `timeScale.barTimeLabel` key of an options patch: absent gives
    /// `None`; `null` and `"open"` give [`BarTimeLabel::Open`]; `{anchor: "close",
    /// interval_seconds, windows?: [["09:30", "11:30"], ...]}` gives a close label. The label
    /// that will be installed, the patch's or, when the patch only moves the session start, the
    /// installed one, is validated against the trading-day start in force after the patch, so a
    /// patch never leaves the chart with windows its own validation rejects.
    pub(crate) fn parse_bar_time_label_patch(
        &self,
        patch: &serde_json::Value,
        patch_session_start_seconds: Option<i32>,
    ) -> Result<Option<BarTimeLabel>, String> {
        let installed_start = self.exchange_time.session_start_seconds();
        let session_start_seconds = patch_session_start_seconds.unwrap_or(installed_start);
        let label = match patch
            .get("timeScale")
            .and_then(|scale| scale.get("barTimeLabel"))
        {
            None => None,
            Some(value) if value.is_null() || value.as_str() == Some("open") => {
                Some(BarTimeLabel::Open)
            }
            Some(value) => Some(parse_close_label(value)?),
        };
        match &label {
            Some(label) => validate_label(label, session_start_seconds)
                .map_err(|error| format!("invalid timeScale.barTimeLabel: {error}"))?,
            None if session_start_seconds != installed_start => {
                validate_label(&self.bar_time_label, session_start_seconds).map_err(|error| {
                    format!(
                        "timeScale.sessionStart does not fit the installed timeScale.barTimeLabel windows: {error}"
                    )
                })?;
            }
            None => {}
        }
        Ok(label)
    }

    /// Choose which instant of a bar its time text prints (see the module docs). Rejects an
    /// interval outside 1 second..1 day and windows that are unordered, more than
    /// [`crate::MAX_SESSION_WINDOWS`], or invalid for the chart's session start; a rejection
    /// changes nothing. The label is mirrored into the options store, so option snapshots and V2
    /// persistence carry it while it is not [`BarTimeLabel::Open`]. While a label is set the
    /// chart also rejects a session start its windows do not fit
    /// ([`crate::ExchangeTimeError::BarTimeLabelWindows`]).
    pub fn set_bar_time_label(&mut self, label: BarTimeLabel) -> Result<(), SessionSlotError> {
        validate_label(&label, self.exchange_time.session_start_seconds())?;
        let changed = self.replace_bar_time_label(label);
        self.bar_time_label_replaced(changed);
        Ok(())
    }

    /// Store a label the caller validated against the session start that will be in force, and
    /// report whether it differs. The window grid, options mirror, and tick weights follow in
    /// [`Self::bar_time_label_replaced`], after any exchange-time change of the same step.
    pub(crate) fn replace_bar_time_label(&mut self, label: BarTimeLabel) -> bool {
        let changed = self.bar_time_label != label;
        self.bar_time_label = label;
        changed
    }

    /// Finish [`Self::replace_bar_time_label`]: place the windows, mirror the label, and, when
    /// it changed, redo the weights and the frame.
    pub(crate) fn bar_time_label_replaced(&mut self, changed: bool) {
        self.rebuild_bar_label_grid();
        self.mirror_bar_time_label_option();
        if changed {
            self.rebuild_tick_weights();
            self.invalidate_frame_all();
        }
    }

    /// The configured bar time label.
    pub fn bar_time_label(&self) -> &BarTimeLabel {
        &self.bar_time_label
    }

    /// The instant a bar identified by `time` prints: `time` itself under an open label, on a
    /// calendar-date axis, and on a non-time sequence axis; otherwise its close. Total: an
    /// instant that lies in no window (the pre-open minute, an extrapolated drawing anchor) or
    /// whose windows a DST transition collapsed prints `time` plus the interval.
    pub fn bar_label_time(&self, time: i64) -> i64 {
        let BarTimeLabel::Close {
            interval_seconds, ..
        } = &self.bar_time_label
        else {
            return time;
        };
        if self.exchange_time.calendar_dates() || self.sequence_points().is_some() {
            return time;
        }
        self.bar_label_grid
            .as_ref()
            .and_then(|grid| grid.bar_close(time).ok().flatten())
            .unwrap_or_else(|| time.saturating_add(i64::from(*interval_seconds)))
    }

    /// Seconds the tick weights shift the identity times by: zero on a non-time sequence axis,
    /// which prints and weighs its own open times. Independent of the calendar-date flag (a
    /// calendar row's neighbours differ in trading day, which returns before the intraday
    /// boundaries), so flipping that flag never leaves stale weights.
    pub(crate) fn tick_label_shift(&self) -> i64 {
        if self.sequence_points().is_some() {
            0
        } else {
            self.bar_time_label.weight_shift()
        }
    }

    /// Re-place the window grid for the current label and exchange time. Every path that
    /// installs a label or a session start validates the pair first ([`Self::set_bar_time_label`],
    /// `set_session_start_seconds`, and the options patch), and windows are placed independently
    /// of the offset schedule, so placement cannot fail here.
    pub(crate) fn rebuild_bar_label_grid(&mut self) {
        self.bar_label_grid = label_grid(&self.bar_time_label, &self.exchange_time)
            .expect("the bar time label was validated for the installed session start");
    }

    /// Reject a session start that the installed label's windows cannot be placed on.
    pub(crate) fn check_bar_time_label_fits(
        &self,
        session_start_seconds: i32,
    ) -> Result<(), SessionSlotError> {
        validate_label(&self.bar_time_label, session_start_seconds)
    }

    /// Retained heap of the configured windows and the derived grid: `(payload, capacity)`.
    pub(crate) fn bar_time_label_bytes(&self) -> (usize, usize) {
        let size = core::mem::size_of::<SessionWindow>();
        let (configured, capacity) = match &self.bar_time_label {
            BarTimeLabel::Open => (0, 0),
            BarTimeLabel::Close { windows, .. } => {
                (windows.len() * size, windows.capacity() * size)
            }
        };
        let grid = self.bar_label_grid.as_ref().map_or(0, |grid| {
            core::mem::size_of_val(grid.windows()) + grid.exchange_time().capacity_bytes()
        });
        (configured + grid, capacity + grid)
    }

    pub(crate) fn bar_time_label_json(&self) -> serde_json::Value {
        match &self.bar_time_label {
            BarTimeLabel::Open => serde_json::Value::String("open".to_string()),
            BarTimeLabel::Close {
                interval_seconds,
                windows,
            } => serde_json::json!({
                "anchor": "close",
                "interval_seconds": interval_seconds,
                "windows": windows
                    .iter()
                    .map(|window| [
                        wall_clock_text(window.start_seconds),
                        wall_clock_text(window.end_seconds),
                    ])
                    .collect::<Vec<_>>(),
            }),
        }
    }

    /// Write a close label into the options store so option snapshots and V2 persistence describe
    /// it. An open label leaves a chart that never had one byte-identical to a default document
    /// and clears the key (to `null`) when a close label was mirrored before.
    pub(crate) fn mirror_bar_time_label_option(&mut self) {
        let value = match &self.bar_time_label {
            BarTimeLabel::Open => {
                let mirrored = self
                    .options
                    .value()
                    .get("timeScale")
                    .and_then(|scale| scale.get("barTimeLabel"))
                    .is_some_and(|value| !value.is_null());
                if !mirrored {
                    return;
                }
                serde_json::Value::Null
            }
            BarTimeLabel::Close { .. } => self.bar_time_label_json(),
        };
        self.options
            .apply(&serde_json::json!({ "timeScale": { "barTimeLabel": value } }));
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use aeris_charts_core::scale::time_tick_marks::{days_from_civil, TickMarkWeight};
    use aeris_charts_render::color::Color;
    use aeris_charts_render::draw_list::Prim;

    use crate::native_primitives::{DeltaTooltipOptions, VerticalLineOptions};
    use crate::{
        parse_iso_date, parse_wall_clock, session_slot_times, AxisTextAlign, AxisTextMidpoint,
        BarTimeLabel, ChartEngine, DrawingKind, DrawingPoint, ExchangeTime, SeriesKind,
        SessionSlotConvention, SessionWindow, TimeTickMark, UtcOffsetSchedule, UtcOffsetTransition,
    };

    const HOUR: i64 = 3_600;
    const DAY: i64 = 86_400;

    fn window(start: &str, end: &str) -> SessionWindow {
        SessionWindow {
            start_seconds: parse_wall_clock(start, false).unwrap(),
            end_seconds: parse_wall_clock(end, true).unwrap(),
        }
    }

    fn a_share() -> Vec<SessionWindow> {
        vec![window("09:30", "11:30"), window("13:00", "15:00")]
    }

    fn close(interval_seconds: u32, windows: Vec<SessionWindow>) -> BarTimeLabel {
        BarTimeLabel::Close {
            interval_seconds,
            windows,
        }
    }

    fn shanghai() -> UtcOffsetSchedule {
        UtcOffsetSchedule::fixed(8 * HOUR as i32).unwrap()
    }

    /// America/New_York around 2024 (EST, EDT from 2024-03-10 07:00 UTC, EST from 2024-11-03).
    fn new_york() -> UtcOffsetSchedule {
        let at = |y, m, d, h: i64| days_from_civil(y, m, d).unwrap() * DAY + h * HOUR;
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

    /// UTC open times of one trading date's bars.
    fn slots(
        zone: &UtcOffsetSchedule,
        date: &str,
        windows: &[SessionWindow],
        interval: u32,
    ) -> Vec<i64> {
        session_slot_times(
            parse_iso_date(date).unwrap(),
            windows,
            interval,
            &ExchangeTime::new(zone.clone(), 0).unwrap(),
            SessionSlotConvention::BarOpen,
        )
        .unwrap()
    }

    /// UTC seconds of an exchange-local `"YYYY-MM-DD HH:MM"`.
    fn at(zone: &UtcOffsetSchedule, text: &str) -> i64 {
        let (date, clock) = text.split_once(' ').unwrap();
        zone.to_utc(
            parse_iso_date(date).unwrap() * DAY
                + i64::from(parse_wall_clock(clock, false).unwrap()),
        )
    }

    fn line_chart(times: &[i64]) -> ChartEngine {
        let mut chart = ChartEngine::new(1_600.0, 500.0, 1.0);
        chart.series[0].kind = SeriesKind::Line;
        let times: Vec<f64> = times.iter().map(|&time| time as f64).collect();
        let values: Vec<f64> = (0..times.len()).map(|i| 100.0 + (i % 9) as f64).collect();
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

    /// `(text, x)` of the automatic tick labels.
    fn tick_labels(chart: &mut ChartEngine) -> Vec<(String, f64)> {
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
            .map(|label| (label.text, label.x))
            .collect()
    }

    fn installed_weights(chart: &mut ChartEngine) -> Vec<u8> {
        let mut marks: Vec<(i64, u8)> = chart
            .tick_marks
            .build(1.0, 0.0)
            .iter()
            .map(|mark| (mark.index, mark.weight))
            .collect();
        marks.sort_unstable();
        marks.into_iter().map(|(_, weight)| weight).collect()
    }

    /// A one-day A-share chart of one-minute open-stamped bars in Shanghai time.
    fn a_share_chart() -> (ChartEngine, Vec<i64>) {
        let zone = shanghai();
        let times = slots(&zone, "2024-01-02", &a_share(), 60);
        assert_eq!(times.len(), 240);
        let mut chart = line_chart(&times);
        chart.set_exchange_offsets(zone);
        (chart, times)
    }

    #[test]
    fn a_share_minute_bars_label_by_close_with_open_identity() {
        let (mut chart, times) = a_share_chart();
        chart.set_bar_time_label(close(60, a_share())).unwrap();
        assert_eq!(crosshair_label(&mut chart, 0), "02 Jan '24   09:31");
        assert_eq!(crosshair_label(&mut chart, 119), "02 Jan '24   11:30");
        assert_eq!(crosshair_label(&mut chart, 120), "02 Jan '24   13:01");
        assert_eq!(crosshair_label(&mut chart, 239), "02 Jan '24   15:00");
        // Identity stays the open second.
        assert_eq!(chart.data_layer().merged_times(), times.as_slice());
        assert_eq!(chart.time_to_index(times[0] as f64, false), Some(0));
        assert_eq!(chart.time_to_index(times[0] as f64 + 60.0, false), Some(1));
        assert_eq!(chart.time_to_index(times[119] as f64 + 60.0, false), None);
        // Without windows every bar prints open plus interval.
        chart.set_bar_time_label(close(60, Vec::new())).unwrap();
        assert_eq!(crosshair_label(&mut chart, 119), "02 Jan '24   11:30");
        chart.set_bar_time_label(BarTimeLabel::Open).unwrap();
        assert_eq!(crosshair_label(&mut chart, 0), "02 Jan '24   09:30");
        assert_eq!(crosshair_label(&mut chart, 239), "02 Jan '24   14:59");
    }

    #[test]
    fn host_fed_241_bar_feed_shifted_back_labels_0930_to_1500() {
        // A host that draws the 09:25 auction print as its own 09:30 point shifts every row back
        // one interval: the auction row lands at 09:29, outside every window, and prints 09:30
        // through the open-plus-interval fallback while the real bars print their closes.
        let (zone, windows) = (shanghai(), a_share());
        let slot_times = slots(&zone, "2024-01-02", &windows, 60);
        let mut times = vec![slot_times[0] - 60];
        times.extend(&slot_times);
        assert_eq!(times.len(), 241);
        let mut chart = line_chart(&times);
        chart.set_exchange_offsets(zone);
        chart.set_bar_time_label(close(60, windows)).unwrap();
        assert_eq!(chart.data_layer().merged_times(), times.as_slice());
        assert_eq!(chart.time_to_index(times[0] as f64, false), Some(0));
        let printed: Vec<String> = [0, 1, 2, 120, 121, 240]
            .into_iter()
            .map(|index| crosshair_label(&mut chart, index))
            .collect();
        assert_eq!(
            printed,
            [
                "02 Jan '24   09:30",
                "02 Jan '24   09:31",
                "02 Jan '24   09:32",
                "02 Jan '24   11:30",
                "02 Jan '24   13:01",
                "02 Jan '24   15:00",
            ]
        );
        assert_eq!(chart.bar_label_time(times[0]), times[0] + 60);
    }

    #[test]
    fn automatic_hour_ticks_sit_on_the_bar_that_closes_on_the_hour() {
        let (mut chart, times) = a_share_chart();
        let x_of = |chart: &ChartEngine, index: i64| {
            chart.pane_left + chart.time_scale.index_to_coordinate(index)
        };
        // By open time the 10:00 tick sits on the bar opened 10:00 (index 30).
        let open = tick_labels(&mut chart);
        assert_eq!(
            open.iter().find(|(text, _)| text == "10:00").map(|t| t.1),
            Some(x_of(&chart, 30))
        );

        chart.set_bar_time_label(close(60, a_share())).unwrap();
        let closed = tick_labels(&mut chart);
        assert_eq!(
            closed.iter().find(|(text, _)| text == "10:00").map(|t| t.1),
            Some(x_of(&chart, 29)),
            "the bar opened 09:59 closes at 10:00: {closed:?}"
        );
        assert!(closed.iter().all(|(text, _)| text != "10:01"));

        // Host formatters receive the printed instant for ticks and the crosshair.
        let ticks = Rc::new(RefCell::new(Vec::new()));
        let seen = Rc::clone(&ticks);
        chart.set_tick_mark_formatter(Some(Box::new(move |time, _| {
            seen.borrow_mut().push(time);
            None
        })));
        chart.set_time_formatter(Some(Box::new(|time| Some(format!("H{time}")))));
        tick_labels(&mut chart);
        let ticks = ticks.borrow().clone();
        assert!(!ticks.is_empty());
        assert!(
            ticks.iter().all(|time| times.contains(&(time - 60))),
            "tick formatter times must be bar closes: {ticks:?}"
        );
        assert_eq!(
            crosshair_label(&mut chart, 3),
            format!("H{}", times[3] + 60)
        );

        // A native vertical line keeps its host text and its identity slot.
        chart.set_time_formatter(None);
        chart
            .add_vertical_line(
                0,
                times[100],
                VerticalLineOptions {
                    label_text: "Event".into(),
                    show_label: true,
                    ..VerticalLineOptions::default()
                },
            )
            .unwrap();
        chart.clear_crosshair_at();
        let axis = chart.build_axis_frame(
            80.0,
            |t, _| t.len() as f64 * 7.0,
            |t, _| t.len() as f64 * 6.0,
        );
        let event = axis
            .labels
            .iter()
            .find(|label| label.text == "Event")
            .unwrap();
        assert_eq!(event.x, x_of(&chart, 100));
    }

    fn a_share_marks_chart() -> (ChartEngine, Vec<i64>) {
        let (mut chart, times) = a_share_chart();
        chart
            .apply_options(r#"{"grid":{"vertLines":{"visible":true}}}"#)
            .unwrap();
        chart.set_lock_visible_logical_range(true);
        chart.set_visible_logical_range(0.0, 239.0);
        chart.set_bar_time_label(close(60, a_share())).unwrap();
        (chart, times)
    }

    fn strip_labels(chart: &mut ChartEngine) -> Vec<String> {
        let frame = chart.build_axis_frame(
            80.0,
            |text, _| text.len() as f64 * 7.0,
            |text, _| text.len() as f64 * 6.0,
        );
        frame
            .labels
            .iter()
            .filter(|label| label.align == AxisTextAlign::Center && label.y > chart.pane_h)
            .map(|label| label.text.clone())
            .collect()
    }

    fn grid_columns(chart: &mut ChartEngine) -> Vec<i32> {
        chart.build_frame().panes[0]
            .under
            .iter()
            .filter_map(|prim| match prim {
                Prim::VLine { x, .. } => Some(*x),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn explicit_marks_keep_identity_time_and_default_to_the_label() {
        let (mut chart, times) = a_share_marks_chart();
        let mark = |time: i64, label: Option<&str>| TimeTickMark {
            time,
            label: label.map(str::to_string),
        };
        // A mark names its bar by identity: 11:29 is the bar closing 11:30 and draws that text,
        // 09:30 draws 09:31. 11:30 is a label instant no bar opens at, so it has no slot.
        let identity = |text: &str| at(&shanghai(), &format!("2024-01-02 {text}"));
        chart
            .set_time_tick_marks(Some(vec![
                mark(identity("09:30"), None),
                mark(identity("11:29"), None),
                mark(identity("11:30"), None),
            ]))
            .unwrap();
        assert_eq!(strip_labels(&mut chart), ["09:31", "11:30"]);
        let expected: Vec<i32> = [0, 119]
            .into_iter()
            .map(|index| chart.time_scale.index_to_coordinate(index).round() as i32)
            .collect();
        assert_eq!(grid_columns(&mut chart), expected);
        assert_eq!(times[119], identity("11:29"));

        // Host label text, empty labels, and the host formatter keep their behaviour.
        chart
            .set_time_tick_marks(Some(vec![
                mark(identity("09:30"), Some("Open")),
                mark(identity("10:30"), Some("")),
                mark(identity("14:59"), None),
            ]))
            .unwrap();
        assert_eq!(strip_labels(&mut chart), ["Open", "15:00"]);
        assert_eq!(grid_columns(&mut chart).len(), 3);
        chart.set_tick_mark_formatter(Some(Box::new(|time, _| Some(format!("T{time}")))));
        assert_eq!(
            strip_labels(&mut chart),
            ["Open".to_string(), format!("T{}", identity("15:00"))]
        );
    }

    fn label_drawing_tags(chart: &mut ChartEngine) -> Vec<String> {
        chart.clear_crosshair_at();
        chart
            .build_axis_frame(
                80.0,
                |t, _| t.len() as f64 * 7.0,
                |t, _| t.len() as f64 * 6.0,
            )
            .labels
            .into_iter()
            .filter(|label| label.midpoint == AxisTextMidpoint::None && label.background.is_some())
            .map(|label| label.text)
            .collect()
    }

    fn text_prims(chart: &mut ChartEngine) -> Vec<String> {
        chart.build_frame().panes[0]
            .main
            .iter()
            .filter_map(|prim| match prim {
                Prim::Text { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn every_bar_time_surface_uses_the_label() {
        let (mut chart, times) = a_share_chart();
        chart.set_bar_time_label(close(60, a_share())).unwrap();
        // The host formatter shows exactly which instant each surface prints.
        chart.set_time_formatter(Some(Box::new(|time| Some(format!("H{time}")))));
        let printed = |time: i64| format!("H{}", time + 60);

        // Crosshair.
        assert_eq!(crosshair_label(&mut chart, 7), printed(times[7]));

        // Drawing axis tags, including an anchor extrapolated beyond the data.
        chart.set_visible_logical_range(0.0, 260.0);
        let beyond = 245.0;
        let rectangle = chart
            .add_drawing(
                DrawingKind::Rectangle,
                0,
                vec![
                    DrawingPoint {
                        logical: 20.0,
                        price: 101.0,
                    },
                    DrawingPoint {
                        logical: beyond,
                        price: 104.0,
                    },
                ],
                Some(r#"{"show_labels":true}"#),
            )
            .unwrap();
        chart.set_selected_drawing(Some(rectangle));
        let extrapolated = chart.anchor_time_at_logical(beyond).unwrap().floor() as i64;
        assert!(extrapolated > times[239], "anchor lies past the data");
        let tags = label_drawing_tags(&mut chart);
        assert!(tags.contains(&printed(times[20])), "{tags:?}");
        // The 15:05 anchor lies in no window and prints open plus interval.
        assert!(tags.contains(&printed(extrapolated)), "{tags:?}");
        chart.set_selected_drawing(None);

        // DateTimeRange statistics.
        let range = chart
            .add_drawing(
                DrawingKind::TrendLine,
                0,
                vec![
                    DrawingPoint {
                        logical: 10.0,
                        price: 101.0,
                    },
                    DrawingPoint {
                        logical: 30.0,
                        price: 103.0,
                    },
                ],
                Some(r#"{"labels":[{"metric":"date_time_range","visible":true,"position":"on"}]}"#),
            )
            .unwrap();
        let drawing = chart.drawing(range).unwrap().clone();
        assert_eq!(
            chart.drawing_stat_lines(&drawing, 0, 1),
            [format!("{} – {}", printed(times[10]), printed(times[30]))]
        );
        // Upstream's label path prints the same engine-formatted range on the frame.
        assert!(text_prims(&mut chart).contains(&format!(
            "{} – {}",
            printed(times[10]),
            printed(times[30])
        )));
        chart.remove_drawing(range);

        // Forecast target time (the fork's line above upstream's outcome label).
        chart
            .add_drawing(
                DrawingKind::Forecast,
                0,
                vec![
                    DrawingPoint {
                        logical: 40.0,
                        price: 101.0,
                    },
                    DrawingPoint {
                        logical: 60.0,
                        price: 106.0,
                    },
                ],
                Some("{}"),
            )
            .unwrap();
        assert!(text_prims(&mut chart).contains(&printed(times[60])));

        // The delta tooltip's time line.
        let tooltip = chart
            .add_delta_tooltip(
                0,
                DeltaTooltipOptions {
                    show_time: true,
                    ..DeltaTooltipOptions::default()
                },
            )
            .unwrap();
        let (x2, x9) = (
            chart.time_scale.index_to_coordinate(2),
            chart.time_scale.index_to_coordinate(9),
        );
        assert!(chart.delta_tooltip_mouse_down(x2));
        assert!(chart.delta_tooltip_mouse_move(x9));
        assert!(chart.delta_tooltip_active_range(tooltip).is_some());
        let texts = text_prims(&mut chart);
        assert!(texts.contains(&printed(times[2])), "{texts:?}");
        assert!(texts.contains(&printed(times[9])), "{texts:?}");

        // The built-in text prints the close as well: the tooltip's time line reads 09:33.
        chart.set_time_formatter(None);
        let texts = text_prims(&mut chart);
        assert!(texts.contains(&"09:33".to_string()), "{texts:?}");
        assert!(!texts.contains(&"09:32".to_string()), "{texts:?}");
    }

    #[test]
    fn host_events_and_readbacks_stay_identity_under_a_label() {
        fn readbacks(chart: &mut ChartEngine, x: f64) -> String {
            chart.build_frame();
            chart.set_crosshair_at(x, 100.0);
            let context = chart.chart_context_at(x, 100.0).unwrap();
            let index = chart.time_scale.coordinate_to_index(x);
            format!(
                "{context:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}",
                chart.coordinate_to_time(x),
                chart.value_snapshot(Some(index)),
                chart.series_data(0),
                chart.series_bars_in_logical_range(0, 10.0, 200.0),
                chart.visible_time_range(),
                chart.crosshair_sync_position(),
                chart.time_to_index(chart.data_layer().merged_times()[5] as f64, false),
            )
        }
        let (mut open, times) = a_share_chart();
        let (mut closed, _) = a_share_chart();
        closed.set_bar_time_label(close(60, a_share())).unwrap();
        let x = open.time_scale.index_to_coordinate(17);
        let before = readbacks(&mut open, x);
        assert_eq!(readbacks(&mut closed, x), before);
        assert!(
            before.contains(&format!("Some({}.0)", times[17])),
            "{before}"
        );
        assert_eq!(closed.data_layer().merged_times(), times.as_slice());
    }

    #[test]
    fn countdown_replay_and_highlighting_ignore_the_label_convention() {
        let shaded = |chart: &mut ChartEngine, color: Color| -> i32 {
            chart.build_frame().panes[0]
                .under
                .iter()
                .filter_map(|prim| match prim {
                    Prim::Rect { rect, color: c } if *c == color => Some(rect.w),
                    _ => None,
                })
                .sum()
        };
        let gate = Color::rgba(9, 8, 7, 40);
        let mut widths = Vec::new();
        for label in [BarTimeLabel::Open, close(60, a_share())] {
            let (mut chart, times) = a_share_chart();
            chart.set_bar_time_label(label).unwrap();
            // The forming bar is the last bar; its countdown counts to open plus one interval.
            chart.series[0].countdown_visible = true;
            chart.now_override = Some((times[239] + 45) as f64);
            assert_eq!(chart.series_countdown_text(0).as_deref(), Some("00:15"));
            chart.now_override = Some((times[239] + 75) as f64);
            assert_eq!(chart.series_countdown_text(0), None);
            // Replay reveals the bar that opened before the clock, whichever way it prints.
            chart
                .set_replay_clock_micros(Some((times[0] + 40) * 1_000_000))
                .unwrap();
            assert_eq!(chart.data_layer().merged_times(), &times[..1]);
            chart.set_replay_clock_micros(None).unwrap();
            assert_eq!(chart.data_layer().merged_times().len(), 240);
            // Session highlighting [14:54, 15:00) covers the bars that OPEN in it.
            chart
                .add_session_highlighting(
                    0,
                    crate::SessionHighlightingOptions {
                        start_hour: Some(14.9),
                        end_hour: Some(15.0),
                        weekday_color: gate,
                        weekend_color: gate,
                    },
                )
                .unwrap();
            widths.push(shaded(&mut chart, gate));
        }
        assert!(widths[0] > 0);
        assert_eq!(widths[0], widths[1]);
    }

    #[test]
    fn midnight_window_keeps_the_last_bar_in_its_trading_day() {
        // The window ends at midnight, the trading-day start: the last bar opens 23:59 and
        // prints 00:00 of the next date but still belongs to its own trading day.
        let utc = UtcOffsetSchedule::utc();
        let windows = vec![window("23:30", "24:00")];
        let mut times = slots(&utc, "2024-01-08", &windows, 60);
        times.extend(slots(&utc, "2024-01-09", &windows, 60));
        assert_eq!(times.len(), 60);
        let run = |label: BarTimeLabel| {
            let mut chart = line_chart(&times);
            chart.set_bar_time_label(label).unwrap();
            let vwap = chart.add_vwap(0, None).unwrap();
            chart.series_apply_options_json(
                0,
                r##"{"color":"#654321","break_on_trading_day":true}"##,
            );
            let output = chart.data.series_data(vwap).unwrap().1[3].to_vec();
            let runs: Vec<u32> = chart.build_frame().panes[0]
                .main
                .iter()
                .filter_map(|prim| match prim {
                    Prim::Polyline {
                        point_count, color, ..
                    } if *color == Color::rgb(0x65, 0x43, 0x21) => Some(*point_count),
                    _ => None,
                })
                .collect();
            let last = crosshair_label(&mut chart, 29);
            let next_first = crosshair_label(&mut chart, 30);
            (output, runs, last, next_first)
        };
        let (open_vwap, open_runs, open_last, _) = run(BarTimeLabel::Open);
        let (close_vwap, close_runs, close_last, close_next) = run(close(60, windows.clone()));
        assert_eq!(open_last, "08 Jan '24   23:59");
        assert_eq!(close_last, "09 Jan '24   00:00");
        assert_eq!(close_next, "09 Jan '24   23:31");
        // Trading-day resets follow the identity: VWAP restarts on the second date's first bar.
        assert_eq!(open_vwap, close_vwap);
        assert_ne!(close_vwap[29], 100.0 + 29.0 % 9.0);
        assert_eq!(close_vwap[30], 100.0 + 30.0 % 9.0);
        assert_eq!(open_runs, close_runs);
        assert_eq!(close_runs, [30, 30]);
    }

    #[test]
    fn windows_give_exact_labels_for_short_last_bars_across_dst() {
        let zone = new_york();
        let windows = vec![window("09:30", "16:00")];
        let mut times = slots(&zone, "2024-03-08", &windows, 3_600);
        times.extend(slots(&zone, "2024-03-11", &windows, 3_600));
        assert_eq!(times.len(), 14);
        let mut chart = line_chart(&times);
        chart.set_exchange_offsets(zone.clone());
        chart
            .set_bar_time_label(close(3_600, windows.clone()))
            .unwrap();
        // The short 15:30 bar closes with the session at 16:00 on both sides of the DST change.
        assert_eq!(crosshair_label(&mut chart, 0), "08 Mar '24   10:30");
        assert_eq!(crosshair_label(&mut chart, 5), "08 Mar '24   15:30");
        assert_eq!(crosshair_label(&mut chart, 6), "08 Mar '24   16:00");
        assert_eq!(crosshair_label(&mut chart, 7), "11 Mar '24   10:30");
        assert_eq!(crosshair_label(&mut chart, 13), "11 Mar '24   16:00");
        // Without windows the short bar prints open plus the interval (documented behaviour).
        chart.set_bar_time_label(close(3_600, Vec::new())).unwrap();
        assert_eq!(crosshair_label(&mut chart, 6), "08 Mar '24   16:30");
        assert_eq!(crosshair_label(&mut chart, 13), "11 Mar '24   16:30");

        // Hong Kong: the 11:30 bar of the 09:30-12:00 morning window closes at 12:00.
        let hong_kong = vec![window("09:30", "12:00"), window("13:00", "16:00")];
        let zone = shanghai();
        let times = slots(&zone, "2024-01-02", &hong_kong, 3_600);
        let mut chart = line_chart(&times);
        chart.set_exchange_offsets(zone);
        chart.set_bar_time_label(close(3_600, hong_kong)).unwrap();
        assert_eq!(crosshair_label(&mut chart, 2), "02 Jan '24   12:00");
        assert_eq!(crosshair_label(&mut chart, 5), "02 Jan '24   16:00");
    }

    #[test]
    fn calendar_dates_and_sequence_axes_ignore_the_label_convention() {
        // Calendar-date daily rows print the date whatever the label says.
        let first = days_from_civil(2024, 1, 8).unwrap() * DAY;
        let days: Vec<i64> = (0..6).map(|index| first + index * DAY).collect();
        let daily = |label: BarTimeLabel| {
            let mut chart = line_chart(&days);
            chart.set_time_visible(false);
            chart.set_exchange_offsets(new_york());
            chart.set_calendar_date_axis(true);
            chart.set_bar_time_label(label).unwrap();
            let texts: Vec<String> = (0..6).map(|i| crosshair_label(&mut chart, i)).collect();
            (texts, chart.bar_label_time(days[0]))
        };
        let (open_text, open_time) = daily(BarTimeLabel::Open);
        let (close_text, close_time) = daily(close(60, Vec::new()));
        assert_eq!(open_text, close_text);
        assert_eq!(open_text[0], "08 Jan '24");
        assert_eq!((open_time, close_time), (days[0], days[0]));

        // A non-time sequence axis (one-trade footprint bars) prints and weighs its own open
        // times: weights equal the unshifted ones with or without a close label, and the
        // crosshair text is unchanged.
        let sequence = |label: BarTimeLabel| {
            use crate::{
                AggressorSide, FootprintAggregationOptions, FootprintBarAggregation,
                FootprintSeriesOptions, FootprintTrade,
            };
            let mut chart = ChartEngine::new(800.0, 420.0, 1.0);
            chart.set_time_visible(true);
            chart.set_exchange_offsets(new_york());
            chart.set_bar_time_label(label).unwrap();
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
            // One-minute-spaced prints 18:58..19:02 New York time: 19:00 is an hour boundary.
            let trades = (0..5)
                .map(|minute| FootprintTrade {
                    timestamp_micros: at(&new_york(), "2024-01-08 18:58") * 1_000_000
                        + minute * 60_000_000,
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
                .collect::<Vec<_>>();
            chart.set_footprint_trades(id, trades.clone()).unwrap();
            chart.time_scale.set_width(800.0);
            assert!(chart.sequence_points().is_some());
            assert_eq!(
                chart.bar_label_time(trades[0].timestamp_micros / 1_000_000),
                trades[0].timestamp_micros / 1_000_000
            );
            // The append path weighs the same way as the rebuild.
            let mut late = trades[4].clone();
            late.timestamp_micros += 60_000_000;
            chart.update_footprint_trade(id, late).unwrap();
            (
                installed_weights(&mut chart),
                (0..6)
                    .map(|index| crosshair_label(&mut chart, index))
                    .collect::<Vec<_>>(),
            )
        };
        let (open_weights, open_texts) = sequence(BarTimeLabel::Open);
        let (close_weights, close_texts) = sequence(close(60, Vec::new()));
        assert_eq!(open_weights[2], TickMarkWeight::Hour1 as u8);
        assert_eq!(open_weights, close_weights);
        assert_eq!(open_texts, close_texts);
        assert_eq!(open_texts[2], "08 Jan '24   19:00");
    }

    #[test]
    fn calendar_flag_flip_needs_no_weight_rebuild() {
        // The weight shift depends on the configured label only, so flipping the calendar-date
        // flag under a UTC exchange time (which never rebuilds weights) leaves them current.
        let utc = UtcOffsetSchedule::utc();
        let times = slots(&utc, "2024-01-08", &[window("00:00", "24:00")], 60);
        let mut chart = line_chart(&times[..300]);
        chart.set_bar_time_label(close(60, Vec::new())).unwrap();
        let installed = installed_weights(&mut chart);
        for calendar in [true, false] {
            chart.set_calendar_date_axis(calendar);
            assert_eq!(installed_weights(&mut chart), installed);
            chart.rebuild_tick_weights();
            assert_eq!(
                installed_weights(&mut chart),
                installed,
                "calendar {calendar}"
            );
        }
        // The shift is in force: by open time the weights differ.
        chart.set_bar_time_label(BarTimeLabel::Open).unwrap();
        assert_ne!(installed_weights(&mut chart), installed);
    }

    /// An empty one-series line chart with the label already configured: the order a host uses
    /// when it creates the chart with `timeScale.barTimeLabel` and loads its bars afterwards.
    fn label_first_chart(zone: UtcOffsetSchedule, label: BarTimeLabel) -> ChartEngine {
        let mut chart = ChartEngine::new(1_600.0, 500.0, 1.0);
        chart.series[0].kind = SeriesKind::Line;
        chart.time_scale.set_width(1_600.0);
        chart.set_time_visible(true);
        chart.set_exchange_offsets(zone);
        chart.set_bar_time_label(label).unwrap();
        chart
    }

    fn install_bars(chart: &mut ChartEngine, times: &[i64]) {
        let times: Vec<f64> = times.iter().map(|&time| time as f64).collect();
        let values: Vec<f64> = (0..times.len()).map(|i| 100.0 + (i % 9) as f64).collect();
        chart
            .set_series_data(0, &times, &values, &values, &values, &values)
            .unwrap();
    }

    /// The weights a one-minute close label must install for `times`: those of the same bars
    /// stamped at their printed instants (open plus one minute, as no A-share bar is short) and
    /// weighed by the unshifted default path. An oracle that shares no shift code with the label.
    fn printed_instant_weights(times: &[i64]) -> Vec<u8> {
        let printed: Vec<i64> = times.iter().map(|time| time + 60).collect();
        let mut chart = line_chart(&printed);
        chart.set_exchange_offsets(shanghai());
        installed_weights(&mut chart)
    }

    /// The installed weights match the oracle for whatever rows the chart holds now.
    fn assert_weights_follow_data(chart: &mut ChartEngine, what: &str) {
        let held = chart.data_layer().merged_times().to_vec();
        assert!(!held.is_empty(), "{what}: no rows");
        assert_eq!(
            installed_weights(chart),
            printed_instant_weights(&held),
            "{what}"
        );
    }

    #[test]
    fn weights_follow_the_label_when_bars_load_and_stream_after_it() {
        let zone = shanghai();
        let times = slots(&zone, "2024-01-02", &a_share(), 60);
        let expected = printed_instant_weights(&times);
        // The bar opened 09:59 closes 10:00 and owns the hour; so do the bars opened 13:59 and
        // 14:59 for 14:00 and 15:00. The bars opened on the hour do not.
        let hour = TickMarkWeight::Hour1 as u8;
        for (owner, bar_opened_on_the_hour) in [(29, 30), (179, 180), (239, 240)] {
            assert!(expected[owner] >= hour, "{owner}: {expected:?}");
            assert!(
                expected
                    .get(bar_opened_on_the_hour)
                    .is_none_or(|w| *w < hour),
                "{bar_opened_on_the_hour}: {expected:?}"
            );
        }

        // Bulk reference: every bar installed in one call, label set first.
        let mut bulk = label_first_chart(zone.clone(), close(60, a_share()));
        install_bars(&mut bulk, &times);
        bulk.fit_content();
        assert_eq!(installed_weights(&mut bulk), expected, "bulk load");
        let bulk_ticks = tick_labels(&mut bulk);

        // Production order: label first, then the history (the first install rebuilds the
        // weights), then the live tail one bar at a time (each append extends them in place).
        let mut chart = label_first_chart(zone, close(60, a_share()));
        install_bars(&mut chart, &times[..150]);
        assert_eq!(
            installed_weights(&mut chart),
            expected[..150],
            "history after the label"
        );
        for &time in &times[150..] {
            assert!(chart.update_series_bar(0, time as f64, [100.0; 4]));
        }
        assert_eq!(installed_weights(&mut chart), expected, "streamed tail");
        chart.fit_content();
        let streamed_ticks = tick_labels(&mut chart);
        assert_eq!(streamed_ticks, bulk_ticks);
        let x_of = |chart: &ChartEngine, index: i64| {
            chart.pane_left + chart.time_scale.index_to_coordinate(index)
        };
        for (text, index) in [("10:00", 29), ("14:00", 179)] {
            assert_eq!(
                streamed_ticks
                    .iter()
                    .find(|(label, _)| label == text)
                    .map(|tick| tick.1),
                Some(x_of(&chart, index)),
                "{text} sits on the bar that closes at {text}: {streamed_ticks:?}"
            );
        }

        // A replacement that cannot extend the old weights (front rows dropped) rebuilds them.
        install_bars(&mut chart, &times[30..200]);
        assert_eq!(chart.data_layer().merged_times(), &times[30..200]);
        assert_weights_follow_data(&mut chart, "replacement");

        // A retention trim inside a live append rebuilds them too, and later appends extend the
        // rebuilt weights again: the cap holds 100 bars, the 101st evicts the oldest rows.
        install_bars(&mut chart, &times[..100]);
        assert!(chart.set_series_max_points(0, Some(100)));
        assert_weights_follow_data(&mut chart, "at the cap");
        assert!(chart.update_series_bar(0, times[100] as f64, [100.0; 4]));
        let kept = chart.data_layer().merged_times().to_vec();
        assert!(
            kept.len() < 100,
            "the append over the cap trimmed: {}",
            kept.len()
        );
        assert_eq!(kept.last(), Some(&times[100]));
        assert_weights_follow_data(&mut chart, "trim and append");
        for &time in &times[101..] {
            assert!(chart.update_series_bar(0, time as f64, [100.0; 4]));
        }
        assert_eq!(chart.data_layer().merged_times().last(), times.last());
        assert_weights_follow_data(&mut chart, "appends after the trim");
    }

    #[test]
    fn exchange_time_changes_replace_the_label_windows() {
        // 45-minute bars end each window with a short bar: 09:45-10:00 and 20:45-21:00.
        let windows = vec![window("09:00", "10:00"), window("20:00", "21:00")];
        let label = close(2_700, windows.clone());
        let utc = UtcOffsetSchedule::utc();
        let short_bar = at(&utc, "2024-01-09 09:45");
        let mut chart = line_chart(&[short_bar - 2_700, short_bar, short_bar + 2_700]);
        chart.set_bar_time_label(label.clone()).unwrap();
        assert_eq!(chart.bar_label_time(short_bar), short_bar + 900);

        // A session start that the installed windows do not fit is rejected and changes nothing:
        // the chart never holds a label its own validation refuses.
        let error = chart
            .set_session_start_seconds(12 * HOUR as i32)
            .expect_err("windows do not fit start 12:00");
        assert_eq!(
            error,
            crate::ExchangeTimeError::BarTimeLabelWindows(
                crate::SessionSlotError::UnorderedWindow { index: 1 }
            )
        );
        assert!(error.to_string().contains("bar time label"), "{error}");
        // A start out of range stays the plain range error, not a label conflict.
        assert!(matches!(
            chart.set_session_start_seconds(86_400),
            Err(crate::ExchangeTimeError::SessionStartOutOfRange { .. })
        ));
        assert_eq!(chart.exchange_time().session_start_seconds(), 0);
        assert_eq!(chart.bar_time_label(), &label);
        assert_eq!(chart.bar_label_time(short_bar), short_bar + 900);
        // Once the label is cleared the session start moves, and windows that fit it install.
        chart.set_bar_time_label(BarTimeLabel::Open).unwrap();
        chart.set_session_start_seconds(12 * HOUR as i32).unwrap();
        assert!(chart.set_bar_time_label(label.clone()).is_err());
        chart.set_session_start_seconds(0).unwrap();
        chart.set_bar_time_label(label.clone()).unwrap();
        assert_eq!(chart.bar_label_time(short_bar), short_bar + 900);

        // A time-zone change re-places the windows: the New York 15:30 bar of a 09:30-16:00
        // session closes at 16:00 in New York and prints open plus interval under UTC.
        let mut chart = line_chart(&[0, 60]);
        let ny_windows = vec![window("09:30", "16:00")];
        chart
            .set_bar_time_label(close(3_600, ny_windows.clone()))
            .unwrap();
        let short = at(&new_york(), "2024-03-08 15:30");
        assert_eq!(chart.bar_label_time(short), short + 3_600);
        chart.set_exchange_offsets(new_york());
        assert_eq!(chart.bar_label_time(short), short + 1_800);
        chart.set_exchange_offsets(utc.clone());
        assert_eq!(chart.bar_label_time(short), short + 3_600);

        // One patch that changes the session start and the windows validates against the new
        // start: these windows are unordered at start 0 and fit start 12:00.
        let mut chart = line_chart(&[short_bar, short_bar + 60]);
        let patch = r#"{"timeScale":{"sessionStart":43200,"barTimeLabel":{"anchor":"close","interval_seconds":2700,"windows":[["20:00","21:00"],["09:00","10:00"]]}}}"#;
        chart.apply_options(patch).unwrap();
        assert_eq!(chart.exchange_time().session_start_seconds(), 43_200);
        assert_eq!(chart.bar_label_time(short_bar), short_bar + 900);
        // The same windows alone are rejected against the installed start, and the patch is
        // atomic.
        let rejected = r#"{"grid":{"vertLines":{"visible":true}},"timeScale":{"sessionStart":0,"barTimeLabel":{"anchor":"close","interval_seconds":2700,"windows":[["20:00","21:00"],["09:00","10:00"]]}}}"#;
        let before = chart.options.value().clone();
        assert!(chart.apply_options(rejected).is_err());
        assert_eq!(chart.options.value(), &before);
        assert_eq!(chart.exchange_time().session_start_seconds(), 43_200);
    }

    /// A chart whose export is a V2 document (a general pane makes the export V2). An import
    /// needs a fresh chart instead (no general handles issued): `ChartEngine::new`.
    fn v2_chart() -> ChartEngine {
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
    }

    /// Windows ordered at session start 00:00 (22:00-02:00 crosses midnight) and unordered at
    /// start -03:00, where the 22:00 window opens on the evening before the trading day and so
    /// starts before the 08:00 window ends.
    fn evening_and_morning() -> Vec<SessionWindow> {
        vec![window("08:00", "12:00"), window("22:00", "02:00")]
    }

    /// Everything an import may not change when it fails.
    fn snapshot(chart: &ChartEngine) -> (serde_json::Value, String, BarTimeLabel, i32) {
        (
            chart.options.value().clone(),
            chart.time_scale_options_json(),
            chart.bar_time_label().clone(),
            chart.exchange_time().session_start_seconds(),
        )
    }

    fn fresh_chart() -> ChartEngine {
        ChartEngine::new(800.0, 500.0, 1.0)
    }

    #[test]
    fn v2_import_rejects_a_label_that_does_not_fit_the_installed_session_start() {
        // Chart A: UTC, start 00:00; its document mirrors no timeScale.sessionStart key.
        let mut source = v2_chart();
        source
            .set_bar_time_label(close(3_600, evening_and_morning()))
            .unwrap();
        let document = source.export_state_json().unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&document).unwrap();
        assert!(parsed["chart_options"]["timeScale"]
            .get("sessionStart")
            .is_none());

        // Chart B keeps its own session start (-03:00) for a document without the key, and at
        // that start the 22:00 window opens before the 08:00 window ends: an error, not a
        // panic, and nothing of the chart changes.
        let mut target = fresh_chart();
        target.set_session_start_seconds(-3 * HOUR as i32).unwrap();
        let before = snapshot(&target);
        let error = target
            .import_state_json(&document)
            .expect_err("windows that do not fit the installed session start");
        assert!(error.message().contains("barTimeLabel"), "{error:?}");
        assert_eq!(snapshot(&target), before);
        assert_eq!(target.bar_time_label(), &BarTimeLabel::Open);
        assert_eq!(target.exchange_time().session_start_seconds(), -3 * 3_600);

        // The same document imports into a chart at the start it was exported for.
        let mut same = fresh_chart();
        same.import_state_json(&document).unwrap();
        assert_eq!(same.bar_time_label(), &close(3_600, evening_and_morning()));
    }

    #[test]
    fn v2_import_accepts_a_label_that_fits_the_installed_session_start() {
        // Windows that fit start -03:00 only (at start 00:00 they are unordered): a document
        // whose sessionStart key is absent still imports into a chart installed at -03:00.
        let windows = vec![window("22:00", "23:00"), window("08:00", "09:00")];
        let mut source = v2_chart();
        source.set_session_start_seconds(-3 * HOUR as i32).unwrap();
        source
            .set_bar_time_label(close(1_800, windows.clone()))
            .unwrap();
        let mut document: serde_json::Value =
            serde_json::from_str(&source.export_state_json().unwrap()).unwrap();
        assert_eq!(
            document["chart_options"]["timeScale"]["sessionStart"],
            -3 * 3_600
        );
        document["chart_options"]["timeScale"]
            .as_object_mut()
            .unwrap()
            .remove("sessionStart");
        let document = document.to_string();

        let mut target = fresh_chart();
        target.set_session_start_seconds(-3 * HOUR as i32).unwrap();
        target.import_state_json(&document).unwrap();
        assert_eq!(target.bar_time_label(), &close(1_800, windows));
        // At start 00:00 the same document is rejected.
        let mut default = fresh_chart();
        assert!(default.import_state_json(&document).is_err());
        assert_eq!(default.bar_time_label(), &BarTimeLabel::Open);
    }

    #[test]
    fn v2_import_keeps_the_installed_label_valid_for_the_document_session_start() {
        // The document carries a session start (12:00) but no label; the installed windows
        // [09:00-10:00, 20:00-21:00] do not fit it, so the import is rejected whole.
        let mut source = v2_chart();
        source.set_session_start_seconds(12 * HOUR as i32).unwrap();
        let document = source.export_state_json().unwrap();
        let mut target = fresh_chart();
        target
            .set_bar_time_label(close(
                2_700,
                vec![window("09:00", "10:00"), window("20:00", "21:00")],
            ))
            .unwrap();
        let before = snapshot(&target);
        let error = target
            .import_state_json(&document)
            .expect_err("installed windows do not fit the document session start");
        assert!(error.message().contains("session"), "{error:?}");
        assert_eq!(snapshot(&target), before);
        assert_eq!(target.exchange_time().session_start_seconds(), 0);
    }

    #[test]
    fn session_start_changes_that_orphan_the_label_windows_are_rejected_atomically() {
        let windows = vec![window("09:00", "10:00"), window("20:00", "21:00")];
        let label = close(2_700, windows);
        let mut chart = v2_chart();
        chart.set_bar_time_label(label.clone()).unwrap();
        let before_options = chart.options.value().clone();
        let before_document = chart.export_state_json().unwrap();

        // The engine option path rejects a start change alone and mutates nothing.
        let error = chart
            .apply_options(
                r#"{"grid":{"vertLines":{"visible":true}},"timeScale":{"sessionStart":43200}}"#,
            )
            .expect_err("session start orphans the installed windows")
            .to_string();
        assert!(error.contains("sessionStart"), "{error}");
        assert!(error.contains("barTimeLabel"), "{error}");
        assert_eq!(chart.options.value(), &before_options);
        assert_eq!(chart.exchange_time().session_start_seconds(), 0);
        assert_eq!(chart.bar_time_label(), &label);
        assert_eq!(chart.export_state_json().unwrap(), before_document);

        // The same patch that also replaces the label is judged against the label it installs.
        chart
            .apply_options(r#"{"timeScale":{"sessionStart":43200,"barTimeLabel":"open"}}"#)
            .unwrap();
        assert_eq!(chart.bar_time_label(), &BarTimeLabel::Open);
        assert_eq!(chart.exchange_time().session_start_seconds(), 43_200);
        chart
            .apply_options(r#"{"timeScale":{"sessionStart":0,"barTimeLabel":{"anchor":"close","interval_seconds":2700,"windows":[["09:00","10:00"],["20:00","21:00"]]}}}"#)
            .unwrap();
        assert_eq!(chart.bar_time_label(), &label);

        // A zone change never orphans windows: placement is structural, independent of offsets.
        chart.set_exchange_offsets(new_york());
        assert_eq!(chart.bar_time_label(), &label);

        // One patch may move the zone, the session start, and the windows together: the label
        // is validated for the start the patch installs and placed once, in the patch's zone.
        // These windows are unordered at start 00:00 and fit start -03:00.
        let mut combined = v2_chart();
        let evening = vec![window("22:00", "23:00"), window("08:00", "09:00")];
        combined
            .apply_options(
                r#"{"timeScale":{"timeZone":[{"from_utc_seconds":0,"offset_seconds":28800}],"sessionStart":-10800,"barTimeLabel":{"anchor":"close","interval_seconds":2700,"windows":[["22:00","23:00"],["08:00","09:00"]]}}}"#,
            )
            .unwrap();
        assert_eq!(combined.bar_time_label(), &close(2_700, evening));
        assert_eq!(combined.exchange_time().session_start_seconds(), -10_800);
        let short_bar = at(&shanghai(), "2024-01-09 22:45");
        assert_eq!(combined.bar_label_time(short_bar), short_bar + 900);
        assert_eq!(combined.bar_label_time(short_bar - 2_700), short_bar);

        // Every reachable state exports a document that imports again.
        let document = chart.export_state_json().unwrap();
        let mut restored = fresh_chart();
        restored.import_state_json(&document).unwrap();
        assert_eq!(restored.bar_time_label(), &label);
    }

    #[test]
    fn every_engine_reachable_label_state_round_trips_through_v2() {
        let label_windows = [
            Vec::new(),
            vec![window("09:30", "11:30"), window("13:00", "15:00")],
            evening_and_morning(),
            vec![window("22:00", "23:00"), window("08:00", "09:00")],
        ];
        let starts = [0, -3 * HOUR as i32, 12 * HOUR as i32, 6 * HOUR as i32];
        for windows in &label_windows {
            for &first in &starts {
                for &second in &starts {
                    let mut chart = v2_chart();
                    if chart.set_session_start_seconds(first).is_err() {
                        continue;
                    }
                    if chart
                        .set_bar_time_label(close(1_800, windows.clone()))
                        .is_err()
                    {
                        continue;
                    }
                    // Whether the second start is accepted, the chart holds a state that its
                    // own validation accepts, and that state round-trips.
                    let _ = chart.set_session_start_seconds(second);
                    let document = chart.export_state_json().unwrap();
                    let mut restored = fresh_chart();
                    restored
                        .import_state_json(&document)
                        .unwrap_or_else(|error| {
                            panic!("windows {windows:?} start {first} then {second}: {error:?}")
                        });
                    assert_eq!(restored.bar_time_label(), chart.bar_time_label());
                    assert_eq!(
                        restored.exchange_time().session_start_seconds(),
                        chart.exchange_time().session_start_seconds()
                    );
                }
            }
        }
    }

    #[test]
    fn bar_time_label_options_validate_mirror_and_persist() {
        let (mut chart, _) = a_share_chart();
        let valid = r#"{"timeScale":{"barTimeLabel":{"anchor":"close","interval_seconds":60,"windows":[["09:30","11:30"],["13:00","15:00"]]}}}"#;
        chart.apply_options(valid).unwrap();
        assert_eq!(chart.bar_time_label(), &close(60, a_share()));
        // The windows and their grid count toward the tick-state memory attribution.
        let without = {
            let (plain, _) = a_share_chart();
            plain.memory_usage().tick_payload_bytes
        };
        assert!(chart.memory_usage().tick_payload_bytes > without);
        let options: serde_json::Value =
            serde_json::from_str(&chart.time_scale_options_json()).unwrap();
        assert_eq!(options["bar_time_label"]["anchor"], "close");
        assert_eq!(options["bar_time_label"]["interval_seconds"], 60);
        assert_eq!(options["bar_time_label"]["windows"][1][1], "15:00");
        assert_eq!(
            chart.options.value()["timeScale"]["barTimeLabel"]["windows"][0][0],
            "09:30"
        );

        // Invalid labels reject the whole patch before anything mutates.
        let many = vec![["09:30", "10:00"]; crate::MAX_SESSION_WINDOWS + 1];
        let cases = [
            r#"{"timeScale":{"barTimeLabel":{"anchor":"close","interval_seconds":0}}}"#.to_string(),
            r#"{"timeScale":{"barTimeLabel":{"anchor":"close","interval_seconds":86400}}}"#.to_string(),
            r#"{"timeScale":{"barTimeLabel":{"anchor":"close","interval_seconds":1.5}}}"#.to_string(),
            r#"{"timeScale":{"barTimeLabel":{"anchor":"open","interval_seconds":60}}}"#.to_string(),
            r#"{"timeScale":{"barTimeLabel":{"anchor":"close"}}}"#.to_string(),
            r#"{"timeScale":{"barTimeLabel":{"anchor":"close","interval_seconds":60,"extra":1}}}"#.to_string(),
            r#"{"timeScale":{"barTimeLabel":{"anchor":"close","interval_seconds":60,"windows":[["13:00","15:00"],["09:30","11:30"]]}}}"#.to_string(),
            r#"{"timeScale":{"barTimeLabel":{"anchor":"close","interval_seconds":60,"windows":[["9:3","11:30"]]}}}"#.to_string(),
            r#"{"timeScale":{"barTimeLabel":{"anchor":"close","interval_seconds":60,"windows":[["09:30","09:30"]]}}}"#.to_string(),
            r#"{"timeScale":{"barTimeLabel":"close"}}"#.to_string(),
            format!(
                r#"{{"timeScale":{{"barTimeLabel":{{"anchor":"close","interval_seconds":60,"windows":{}}}}}}}"#,
                serde_json::json!(many)
            ),
        ];
        let before = chart.options.value().clone();
        for bad in cases {
            let patch = bad.replacen('{', r#"{"grid":{"vertLines":{"visible":true}},"#, 1);
            assert!(chart.apply_options(&patch).is_err(), "{bad}");
            assert_eq!(chart.options.value(), &before, "{bad}");
            assert_eq!(chart.bar_time_label(), &close(60, a_share()), "{bad}");
        }
        assert_eq!(
            chart.set_bar_time_label(close(0, Vec::new())),
            Err(crate::SessionSlotError::InvalidInterval { seconds: 0 })
        );
        assert_eq!(chart.bar_time_label(), &close(60, a_share()));

        // V2 persistence carries the label; a document without the key keeps the installed one.
        chart
            .add_pane_with_domain(
                true,
                crate::HorizontalDomain::Category {
                    scale: crate::CategoryScaleType::Band,
                },
            )
            .unwrap();
        let document = chart.export_state_json().unwrap();
        let mut restored = ChartEngine::new(800.0, 500.0, 1.0);
        restored.import_state_json(&document).unwrap();
        assert_eq!(restored.bar_time_label(), &close(60, a_share()));
        let mut legacy: serde_json::Value = serde_json::from_str(&document).unwrap();
        legacy["chart_options"]["timeScale"]
            .as_object_mut()
            .unwrap()
            .remove("barTimeLabel");
        let legacy = legacy.to_string();
        let mut fresh = ChartEngine::new(800.0, 500.0, 1.0);
        fresh.import_state_json(&legacy).unwrap();
        assert_eq!(fresh.bar_time_label(), &BarTimeLabel::Open);
        let mut labelled = ChartEngine::new(800.0, 500.0, 1.0);
        labelled.set_bar_time_label(close(300, Vec::new())).unwrap();
        labelled.import_state_json(&legacy).unwrap();
        assert_eq!(labelled.bar_time_label(), &close(300, Vec::new()));
        let exported: serde_json::Value =
            serde_json::from_str(&labelled.export_state_json().unwrap()).unwrap();
        assert_eq!(
            exported["chart_options"]["timeScale"]["barTimeLabel"]["interval_seconds"],
            300
        );

        // A chart that never had a label exports the default document byte for byte, and
        // switching back to Open clears the mirrored key.
        let export_of = |configure: &dyn Fn(&mut ChartEngine)| {
            let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
            chart
                .add_pane_with_domain(
                    true,
                    crate::HorizontalDomain::Category {
                        scale: crate::CategoryScaleType::Band,
                    },
                )
                .unwrap();
            configure(&mut chart);
            chart.export_state_json().unwrap()
        };
        let default = export_of(&|_| {});
        assert_eq!(
            default,
            export_of(&|chart| chart.set_bar_time_label(BarTimeLabel::Open).unwrap())
        );
        assert!(!default.contains("barTimeLabel"));
        chart
            .apply_options(r#"{"timeScale":{"barTimeLabel":null}}"#)
            .unwrap();
        assert_eq!(chart.bar_time_label(), &BarTimeLabel::Open);
        assert!(chart.options.value()["timeScale"]["barTimeLabel"].is_null());
        let options: serde_json::Value =
            serde_json::from_str(&chart.time_scale_options_json()).unwrap();
        assert_eq!(options["bar_time_label"], "open");
        chart.apply_options(valid).unwrap();
        chart
            .apply_options(r#"{"timeScale":{"barTimeLabel":"open"}}"#)
            .unwrap();
        assert!(chart.options.value()["timeScale"]["barTimeLabel"].is_null());
    }
}
