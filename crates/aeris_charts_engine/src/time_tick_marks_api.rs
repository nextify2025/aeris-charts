//! Host-supplied time-axis tick marks.
//!
//! Automatic time ticks are chosen by boundary weight and label spacing. Session charts (A-share
//! time-sharing 09:30/10:30/11:30|13:00/14:00/15:00, one mark per day of a multi-day intraday
//! chart) instead need marks at fixed anchor times. An explicit list replaces the automatic
//! selection for both the axis labels and the vertical grid until it is cleared. Anchors resolve
//! to the time point with exactly their time, so they work on whitespace slots reserved for bars
//! that have not traded yet; an anchor without a time point is not drawn.

use aeris_charts_core::format::time_formatter::{format_tick_label_in, weight_to_tick_mark_type};
use aeris_charts_core::model::data_validation::validate_timestamp;
use aeris_charts_core::scale::time_tick_marks::{TickMarkWeight, weight_by_time_shifted};
use serde::{Deserialize, Serialize};

use crate::axis_metrics::AXIS_FONT_SCALE;
use crate::{AxisFrame, AxisLabel, AxisLabelCorners, AxisTextAlign, AxisTextMidpoint, ChartEngine};

/// At most this many explicit time-axis marks.
pub const MAX_TIME_TICK_MARKS: usize = 512;
/// Longest explicit label, in UTF-8 bytes.
pub const MAX_TIME_TICK_LABEL_BYTES: usize = 64;
/// Minimum horizontal clearance between two explicit labels, in CSS px.
const LABEL_GAP: f64 = 4.0;

/// One explicit time-axis mark: a canonical UTC-seconds time point and optional label text.
/// Without a label the built-in label for that time point is used (exchange time, the host
/// tick-mark formatter first); an empty label keeps the grid line and draws no text.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimeTickMark {
    pub time: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// Rejected explicit tick marks; nothing is changed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TimeTickMarksError {
    TooMany { count: usize },
    InvalidTime { index: usize },
    Unordered { index: usize },
    LabelTooLong { index: usize },
}

impl std::fmt::Display for TimeTickMarksError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooMany { count } => write!(
                f,
                "{count} time tick marks; at most {MAX_TIME_TICK_MARKS} are supported"
            ),
            Self::InvalidTime { index } => {
                write!(f, "time tick mark {index} needs a whole UTC-seconds time")
            }
            Self::Unordered { index } => write!(
                f,
                "time tick mark {index} is not strictly after the previous mark"
            ),
            Self::LabelTooLong { index } => write!(
                f,
                "time tick mark {index} label exceeds {MAX_TIME_TICK_LABEL_BYTES} bytes"
            ),
        }
    }
}

impl std::error::Error for TimeTickMarksError {}

/// An explicit mark resolved against the current time points.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ResolvedTimeTickMark {
    pub index: i64,
    pub weight: u8,
    /// Position in the configured list (for its label).
    pub mark: usize,
}

fn validate(marks: &[TimeTickMark]) -> Result<(), TimeTickMarksError> {
    if marks.len() > MAX_TIME_TICK_MARKS {
        return Err(TimeTickMarksError::TooMany { count: marks.len() });
    }
    for (index, mark) in marks.iter().enumerate() {
        if validate_timestamp(mark.time as f64).ok() != Some(mark.time) {
            return Err(TimeTickMarksError::InvalidTime { index });
        }
        if index > 0 && mark.time <= marks[index - 1].time {
            return Err(TimeTickMarksError::Unordered { index });
        }
        if mark
            .label
            .as_ref()
            .is_some_and(|label| label.len() > MAX_TIME_TICK_LABEL_BYTES)
        {
            return Err(TimeTickMarksError::LabelTooLong { index });
        }
    }
    Ok(())
}

/// Parse the `timeScale.tickMarks` key of an options patch: absent → `None`, `null` → clear,
/// otherwise a validated list.
pub(crate) fn parse_time_tick_marks_patch(
    patch: &serde_json::Value,
) -> Result<Option<Option<Vec<TimeTickMark>>>, String> {
    let Some(value) = patch
        .get("timeScale")
        .and_then(|scale| scale.get("tickMarks"))
    else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(Some(None));
    }
    let marks: Vec<TimeTickMark> = serde_json::from_value(value.clone())
        .map_err(|error| format!("invalid timeScale.tickMarks: {error}"))?;
    validate(&marks).map_err(|error| error.to_string())?;
    Ok(Some(Some(marks)))
}

impl ChartEngine {
    /// Replace the automatic time-axis tick selection with explicit marks (or restore it with
    /// `None`). Marks must be strictly ascending whole UTC seconds, at most
    /// [`MAX_TIME_TICK_MARKS`], with labels of at most [`MAX_TIME_TICK_LABEL_BYTES`] bytes. The
    /// axis labels and the vertical grid follow the marks that match a time point, including
    /// whitespace slots. The list is mirrored into the options store, so option snapshots and
    /// V2 persistence carry it.
    pub fn set_time_tick_marks(
        &mut self,
        marks: Option<Vec<TimeTickMark>>,
    ) -> Result<(), TimeTickMarksError> {
        if let Some(marks) = &marks {
            validate(marks)?;
        }
        self.time_tick_marks = marks.map(|mut marks| {
            marks.shrink_to_fit();
            marks
        });
        self.mirror_time_tick_marks_option();
        self.invalidate_frame_all();
        Ok(())
    }

    /// The explicit time-axis marks, or `None` while ticks are selected automatically.
    pub fn time_tick_marks(&self) -> Option<&[TimeTickMark]> {
        self.time_tick_marks.as_deref()
    }

    pub(crate) fn time_tick_marks_json(&self) -> serde_json::Value {
        self.time_tick_marks
            .as_ref()
            .map_or(serde_json::Value::Null, |marks| serde_json::json!(marks))
    }

    pub(crate) fn mirror_time_tick_marks_option(&mut self) {
        let marks = self.time_tick_marks_json();
        self.options
            .apply(&serde_json::json!({ "timeScale": { "tickMarks": marks } }));
    }

    /// Retained heap payload of the explicit marks (memory attribution).
    pub(crate) fn time_tick_marks_bytes(&self) -> (usize, usize) {
        self.time_tick_marks.as_ref().map_or((0, 0), |marks| {
            let labels = |capacity: bool| {
                marks
                    .iter()
                    .filter_map(|mark| mark.label.as_ref())
                    .map(|label| {
                        if capacity {
                            label.capacity()
                        } else {
                            label.len()
                        }
                    })
                    .sum::<usize>()
            };
            let size = std::mem::size_of::<TimeTickMark>();
            (
                marks.len() * size + labels(false),
                marks.capacity() * size + labels(true),
            )
        })
    }

    /// Explicit marks resolved to time-point indices, in ascending order. `None` while ticks are
    /// selected automatically. `O(marks × log points)`.
    pub(crate) fn resolved_time_tick_marks(&self) -> Option<Vec<ResolvedTimeTickMark>> {
        let marks = self.time_tick_marks.as_ref()?;
        Some(
            marks
                .iter()
                .enumerate()
                .filter_map(|(mark, tick)| {
                    let index = self.time_to_index(tick.time as f64, false)?;
                    Some(ResolvedTimeTickMark {
                        index,
                        weight: self.time_point_weight(index),
                        mark,
                    })
                })
                .collect(),
        )
    }

    /// Boundary weight of one time point, as the automatic selection computes it: against the
    /// previous point, or for the first point against one average spacing before it. On a chart
    /// spanning several trading days the first point also opens its day, so a mark there weighs
    /// at least a day like every later day-open mark (one label style for all of them).
    fn time_point_weight(&self, index: i64) -> u8 {
        let Ok(index) = usize::try_from(index) else {
            return TickMarkWeight::LessThanSecond as u8;
        };
        let Some(time) = self.axis_time_key_at(index) else {
            return TickMarkWeight::LessThanSecond as u8;
        };
        let shift = self.tick_label_shift();
        if index > 0 {
            return self
                .axis_time_key_at(index - 1)
                .map_or(TickMarkWeight::LessThanSecond as u8, |previous| {
                    weight_by_time_shifted(time, previous, shift, &self.exchange_time) as u8
                });
        }
        let count = self
            .sequence_points()
            .map_or(self.data.merged_times().len(), <[_]>::len);
        let Some(last) = (count > 1)
            .then(|| self.axis_time_key_at(count - 1))
            .flatten()
        else {
            return TickMarkWeight::LessThanSecond as u8;
        };
        let average = ((last - time) as f64 / (count as f64 - 1.0)).ceil() as i64;
        let weight = weight_by_time_shifted(time, time - average, shift, &self.exchange_time) as u8;
        if self.exchange_time.trading_day(time) != self.exchange_time.trading_day(last) {
            weight.max(TickMarkWeight::Day as u8)
        } else {
            weight
        }
    }

    /// Axis labels for explicit marks inside `[from, to]`. Labels are kept inside the time-axis
    /// strip (the reference edge alignment of `_alignTickMarkLabelCoordinate`) and a label that
    /// would overlap the previous one is skipped; its grid line and tick stay.
    pub(crate) fn append_explicit_time_labels<F>(
        &self,
        out: &mut AxisFrame,
        marks: &[ResolvedTimeTickMark],
        (from, to): (i64, i64),
        measure: &F,
    ) where
        F: Fn(&str, bool) -> f64,
    {
        let Some(configured) = self.time_tick_marks.as_ref() else {
            return;
        };
        let y = self.pane_h + self.axis_metrics().time_text_dy();
        let color = self.primary_text_color();
        let left = self.pane_left;
        let right = self.pane_left + self.pane_w;
        let mut previous_right = f64::NEG_INFINITY;
        for mark in marks {
            if mark.index < from || mark.index > to {
                continue;
            }
            let Some(ts) = usize::try_from(mark.index)
                .ok()
                .and_then(|index| self.axis_time_key_at(index))
            else {
                continue;
            };
            let x = self.pane_left + self.time_scale.index_to_coordinate(mark.index);
            if self.time_ticks_visible {
                out.time_ticks.push(x);
            }
            let text = match configured[mark.mark].label.as_ref() {
                Some(label) => label.clone(),
                None => {
                    let kind = weight_to_tick_mark_type(
                        mark.weight,
                        self.time_visible,
                        self.seconds_visible,
                    );
                    // The mark sits on the bar with this identity time; its default text prints
                    // the bar's label time (the close under a close-time label).
                    let printed = self.bar_label_time(ts);
                    self.tick_mark_formatter_fn
                        .as_ref()
                        .and_then(|formatter| formatter(printed, kind as u8))
                        .unwrap_or_else(|| {
                            format_tick_label_in(
                                printed,
                                kind,
                                &self.month_names,
                                &self.exchange_time,
                            )
                        })
                }
            };
            if text.is_empty() {
                continue;
            }
            // Explicit marks have no weight competition: day (and longer) boundaries are the
            // major labels.
            let bold = self.time_scale.options().allow_bold_labels
                && mark.weight >= TickMarkWeight::Day as u8;
            let half = measure(&text, bold) / 2.0;
            let center = if right - left <= 2.0 * half {
                (left + right) / 2.0
            } else {
                x.clamp(left + half, right - half)
            };
            if center - half < previous_right + LABEL_GAP {
                continue;
            }
            previous_right = center + half;
            out.labels.push(AxisLabel {
                text,
                x: center,
                y,
                color,
                align: AxisTextAlign::Center,
                midpoint: AxisTextMidpoint::None,
                font_scale: AXIS_FONT_SCALE,
                bold,
                background: None,
                background_corners: AxisLabelCorners::NONE,
                measure_extra: 0.0,
                attach_group: None,
                border: None,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use aeris_charts_core::scale::exchange_time::ExchangeTime;
    use aeris_charts_render::draw_list::Prim;

    use super::{MAX_TIME_TICK_MARKS, TimeTickMark, TimeTickMarksError};
    use crate::{
        AxisTextAlign, ChartEngine, SeriesKind, SessionSlotConvention, SessionWindow,
        UtcOffsetSchedule, parse_iso_date, parse_wall_clock, session_slot_times,
    };

    const WIDTH: f64 = 800.0;

    fn shanghai() -> UtcOffsetSchedule {
        UtcOffsetSchedule::fixed(8 * 3_600).unwrap()
    }

    fn slots(date: &str) -> Vec<i64> {
        let window = |start: &str, end: &str| SessionWindow {
            start_seconds: parse_wall_clock(start, false).unwrap(),
            end_seconds: parse_wall_clock(end, true).unwrap(),
        };
        session_slot_times(
            parse_iso_date(date).unwrap(),
            &[window("09:30", "11:30"), window("13:00", "15:00")],
            60,
            &ExchangeTime::new(shanghai(), 0).unwrap(),
            SessionSlotConvention::BarCloseWithOpen,
        )
        .unwrap()
    }

    fn at(date: &str, hour: i64, minute: i64) -> i64 {
        parse_iso_date(date).unwrap() * 86_400 + (hour - 8) * 3_600 + minute * 60
    }

    /// A full A-share session of whitespace slots, traded through `traded` minutes, held fixed.
    fn session_chart(times: &[i64], traded: usize) -> ChartEngine {
        let mut chart = ChartEngine::new(WIDTH, 500.0, 1.0);
        chart.series[0].kind = SeriesKind::Line;
        let times_f: Vec<f64> = times.iter().map(|&time| time as f64).collect();
        let values: Vec<f64> = (0..times.len())
            .map(|i| {
                if i < traded {
                    10.0 + i as f64 * 0.01
                } else {
                    f64::NAN
                }
            })
            .collect();
        chart
            .set_series_data(0, &times_f, &values, &values, &values, &values)
            .unwrap();
        chart.time_scale.set_width(WIDTH);
        chart.set_exchange_offsets(shanghai());
        chart.set_time_visible(true);
        chart.set_lock_visible_logical_range(true);
        chart.set_visible_logical_range(0.0, times.len() as f64 - 1.0);
        // The canonical chart ships without a grid; these tests assert that the vertical grid
        // follows the explicit marks, so they turn it on.
        chart
            .apply_options(r#"{"grid":{"vertLines":{"visible":true}}}"#)
            .unwrap();
        chart
    }

    fn time_labels(chart: &mut ChartEngine) -> Vec<(String, f64, bool)> {
        let frame = chart.build_axis_frame(
            80.0,
            |text, _bold| text.len() as f64 * 7.0,
            |text, _bold| text.len() as f64 * 6.0,
        );
        frame
            .labels
            .iter()
            .filter(|label| label.align == AxisTextAlign::Center && label.y > chart.pane_h)
            .map(|label| (label.text.clone(), label.x, label.bold))
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

    fn mark(time: i64, label: Option<&str>) -> TimeTickMark {
        TimeTickMark {
            time,
            label: label.map(str::to_string),
        }
    }

    fn a_share_marks(date: &str) -> Vec<TimeTickMark> {
        vec![
            mark(at(date, 9, 30), None),
            mark(at(date, 10, 30), None),
            mark(at(date, 11, 30), Some("11:30/13:00")),
            mark(at(date, 14, 0), None),
            mark(at(date, 15, 0), None),
        ]
    }

    #[test]
    fn explicit_marks_replace_automatic_labels_and_grid_on_future_slots() {
        let date = "2026-09-25";
        let times = slots(date);
        assert_eq!(times.len(), 241);
        // Only the opening minutes have traded: every later anchor sits on a whitespace slot.
        let mut chart = session_chart(&times, 3);
        let automatic = time_labels(&mut chart);
        assert!(automatic.iter().any(|(text, _, _)| text == "10:00"));

        chart
            .set_time_tick_marks(Some(a_share_marks(date)))
            .unwrap();
        let labels = time_labels(&mut chart);
        let texts: Vec<&str> = labels.iter().map(|(text, _, _)| text.as_str()).collect();
        assert_eq!(texts, ["09:30", "10:30", "11:30/13:00", "14:00", "15:00"]);
        assert!(labels.iter().all(|(_, _, bold)| !bold));
        // Labels stay inside the time-axis strip at both session edges; interior ones centre on
        // their slot.
        let half = |text: &str| text.len() as f64 * 7.0 / 2.0;
        assert_eq!(labels[0].1, chart.pane_left + half("09:30"));
        assert_eq!(labels[4].1, chart.pane_left + chart.pane_w - half("15:00"));
        for (label, index) in labels[1..4].iter().zip([60, 120, 180]) {
            assert_eq!(
                label.1,
                chart.pane_left + chart.time_scale.index_to_coordinate(index)
            );
        }
        // The vertical grid follows the anchors, not the automatic marks.
        let expected: Vec<i32> = [0, 60, 120, 180, 240]
            .into_iter()
            .map(|index| chart.time_scale.index_to_coordinate(index).round() as i32)
            .collect();
        assert_eq!(grid_columns(&mut chart), expected);

        // Clearing restores the automatic selection.
        chart.set_time_tick_marks(None).unwrap();
        assert_eq!(time_labels(&mut chart), automatic);
    }

    #[test]
    fn marks_without_a_slot_are_skipped_and_crowded_labels_keep_their_grid_line() {
        let date = "2026-09-25";
        let mut chart = session_chart(&slots(date), 241);
        // Unordered marks reject the whole list and change nothing.
        assert_eq!(
            chart.set_time_tick_marks(Some(vec![
                mark(at(date, 9, 30), None),
                mark(at(date, 9, 30), None),
            ])),
            Err(TimeTickMarksError::Unordered { index: 1 })
        );
        assert_eq!(chart.time_tick_marks(), None);
        chart
            .set_time_tick_marks(Some(vec![
                mark(at(date, 9, 30), None),
                mark(at(date, 11, 30), Some("11:30")),
                // Inside the lunch break: no slot, so neither label nor grid line.
                mark(at(date, 12, 0), Some("12:00")),
                // The next slot after 11:30: its label would overlap; the grid line stays.
                mark(at(date, 13, 1), Some("13:01")),
                // An empty label keeps only the grid line.
                mark(at(date, 15, 0), Some("")),
            ]))
            .unwrap();
        let texts: Vec<String> = time_labels(&mut chart)
            .into_iter()
            .map(|(text, _, _)| text)
            .collect();
        assert_eq!(texts, ["09:30", "11:30"]);
        assert_eq!(grid_columns(&mut chart).len(), 4);
    }

    #[test]
    fn day_open_marks_label_each_day_of_a_multi_day_chart() {
        let days = [
            "2026-09-21",
            "2026-09-22",
            "2026-09-23",
            "2026-09-24",
            "2026-09-25",
        ];
        let times: Vec<i64> = days.iter().flat_map(|day| slots(day)).collect();
        let mut chart = session_chart(&times, times.len());
        chart
            .set_time_tick_marks(Some(
                days.iter().map(|day| mark(at(day, 9, 30), None)).collect(),
            ))
            .unwrap();
        let labels = time_labels(&mut chart);
        // Default labels name each day's trading date in bold. The chart's first point opens its
        // trading day too, so its mark matches the later day opens instead of showing the time
        // the automatic first-point guess would give it.
        let texts: Vec<&str> = labels.iter().map(|(text, _, _)| text.as_str()).collect();
        assert_eq!(texts, ["21", "22", "23", "24", "25"]);
        assert!(labels.iter().all(|(_, _, bold)| *bold));
        // Host labels name every day explicitly, in one style.
        chart
            .set_time_tick_marks(Some(
                days.iter()
                    .map(|day| mark(at(day, 9, 30), Some(&day[5..])))
                    .collect(),
            ))
            .unwrap();
        let labels = time_labels(&mut chart);
        let texts: Vec<&str> = labels.iter().map(|(text, _, _)| text.as_str()).collect();
        assert_eq!(texts, ["09-21", "09-22", "09-23", "09-24", "09-25"]);
        assert!(labels.iter().all(|(_, _, bold)| *bold));

        // A single-day chart keeps the first-point guess: its open mark reads as a time.
        let mut one_day = session_chart(&slots(days[0]), 241);
        one_day
            .set_time_tick_marks(Some(vec![mark(at(days[0], 9, 30), None)]))
            .unwrap();
        assert_eq!(
            time_labels(&mut one_day),
            [("09:30".to_string(), one_day.pane_left + 17.5, false)]
        );
    }

    #[test]
    fn options_route_validates_mirrors_and_persists_the_marks() {
        let date = "2026-09-25";
        let mut chart = session_chart(&slots(date), 10);
        let marks = a_share_marks(date);
        let json = serde_json::json!({ "timeScale": { "tickMarks": marks } }).to_string();
        chart.apply_options(&json).unwrap();
        assert_eq!(chart.time_tick_marks(), Some(marks.as_slice()));
        let options: serde_json::Value =
            serde_json::from_str(&chart.time_scale_options_json()).unwrap();
        assert_eq!(options["tick_marks"][2]["label"], "11:30/13:00");
        assert!(options["tick_marks"][0].get("label").is_none());

        // Invalid patches reject atomically, other keys included.
        let long = "x".repeat(super::MAX_TIME_TICK_LABEL_BYTES + 1);
        for bad in [
            r#"{"timeScale":{"tickMarks":[{"time":2},{"time":1}]},"grid":{"vertLines":{"visible":false}}}"#.to_string(),
            r#"{"timeScale":{"tickMarks":[{"time":1.5}]}}"#.to_string(),
            format!(r#"{{"timeScale":{{"tickMarks":[{{"time":1,"label":"{long}"}}]}}}}"#),
            r#"{"timeScale":{"tickMarks":"09:30"}}"#.to_string(),
        ] {
            assert!(chart.apply_options(&bad).is_err(), "{bad}");
        }
        assert!(chart.options.get().grid.vert_lines.visible);
        assert_eq!(chart.time_tick_marks(), Some(marks.as_slice()));
        let too_many = (0..=MAX_TIME_TICK_MARKS as i64)
            .map(|time| mark(time, None))
            .collect();
        assert_eq!(
            chart.set_time_tick_marks(Some(too_many)),
            Err(TimeTickMarksError::TooMany {
                count: MAX_TIME_TICK_MARKS + 1
            })
        );
        assert!(chart.memory_usage().tick_payload_bytes > 0);

        // V2 persistence carries the marks in the options store.
        chart
            .add_pane_with_domain(
                true,
                crate::HorizontalDomain::Category {
                    scale: crate::CategoryScaleType::Band,
                },
            )
            .unwrap();
        let document = chart.export_state_json().unwrap();
        let mut restored = ChartEngine::new(WIDTH, 500.0, 1.0);
        restored.import_state_json(&document).unwrap();
        assert_eq!(restored.time_tick_marks(), Some(marks.as_slice()));
        chart
            .apply_options(r#"{"timeScale":{"tickMarks":null}}"#)
            .unwrap();
        assert_eq!(chart.time_tick_marks(), None);
        assert!(chart.options.value()["timeScale"]["tickMarks"].is_null());
    }
}
