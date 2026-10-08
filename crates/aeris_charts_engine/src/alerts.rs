//! Host-authoritative price-alert presentation and creation requests.
//!
//! Aeris renders alert lines and the crosshair create chip, but it does not evaluate alert
//! conditions or deliver notifications. Those operations require the host's live price stream,
//! persistence, account limits, and background/server lifecycle.

use std::collections::{HashSet, VecDeque};

use aeris_charts_render::draw_list::RasterImage;
use serde::{Deserialize, Serialize};

use crate::{ChartEngine, ChartError, ErrorCode, PANELESS, PriceScaleTarget};

pub const MAX_ALERT_LINES: usize = 4_096;
const MAX_ALERT_REQUESTS: usize = 256;
const MAX_ALERT_ID_BYTES: usize = 128;
const MAX_ALERT_LABEL_BYTES: usize = 256;

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AlertId(String);

impl AlertId {
    pub fn new(value: impl Into<String>) -> Result<Self, ChartError> {
        let value = value.into();
        if value.is_empty() || value.len() > MAX_ALERT_ID_BYTES {
            return Err(invalid(format!(
                "AlertId must contain 1..={MAX_ALERT_ID_BYTES} UTF-8 bytes"
            )));
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn heap_bytes(&self) -> usize {
        self.0.capacity()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlertPriceScale {
    #[default]
    Right,
    Left,
    Overlay,
}

impl From<AlertPriceScale> for PriceScaleTarget {
    fn from(value: AlertPriceScale) -> Self {
        match value {
            AlertPriceScale::Right => Self::Right,
            AlertPriceScale::Left => Self::Left,
            AlertPriceScale::Overlay => Self::Overlay,
        }
    }
}

impl TryFrom<PriceScaleTarget> for AlertPriceScale {
    type Error = ();

    fn try_from(value: PriceScaleTarget) -> Result<Self, Self::Error> {
        match value {
            PriceScaleTarget::Right => Ok(Self::Right),
            PriceScaleTarget::Left => Ok(Self::Left),
            PriceScaleTarget::Overlay => Ok(Self::Overlay),
            PriceScaleTarget::Named(_) => Err(()),
        }
    }
}

/// Price comparison selected by the host's alert dialog.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlertCondition {
    #[default]
    Crossing,
    CrossingUp,
    CrossingDown,
    GreaterThan,
    LessThan,
}

/// Trigger frequency retained for indicator fidelity. The host owns actual evaluation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlertFrequency {
    #[default]
    OnlyOnce,
    EveryTime,
    OncePerBar,
    OncePerBarClose,
    OncePerMinute,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlertLineStatus {
    #[default]
    Active,
    Triggered,
    Expired,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AlertLine {
    pub id: AlertId,
    #[serde(default)]
    pub pane_index: usize,
    #[serde(default)]
    pub price_scale: AlertPriceScale,
    pub price: f64,
    #[serde(default)]
    pub condition: AlertCondition,
    #[serde(default)]
    pub frequency: AlertFrequency,
    #[serde(default)]
    pub status: AlertLineStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AlertSnapshot {
    pub lines: Vec<AlertLine>,
}

/// A click on the crosshair's plus chip. The host opens its dialog from this exact price/scale.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AlertCreateRequest {
    pub sequence: u32,
    pub pane_index: usize,
    pub price_scale: AlertPriceScale,
    pub price: f64,
    pub condition: AlertCondition,
    pub frequency: AlertFrequency,
}

#[derive(Clone, Debug)]
pub(crate) struct AlertState {
    pub lines: Vec<AlertLine>,
    pub create_button_visible: bool,
    requests: VecDeque<AlertCreateRequest>,
    next_request_sequence: u32,
    pub(crate) create_icon: Option<RasterImage>,
}

impl Default for AlertState {
    fn default() -> Self {
        Self {
            lines: Vec::new(),
            create_button_visible: true,
            requests: VecDeque::new(),
            next_request_sequence: 0,
            create_icon: None,
        }
    }
}

impl AlertState {
    fn next_sequence(&mut self) -> u32 {
        self.next_request_sequence = self.next_request_sequence.wrapping_add(1).max(1);
        self.next_request_sequence
    }

    fn push_request(&mut self, request: AlertCreateRequest) {
        if self.requests.len() == MAX_ALERT_REQUESTS {
            self.requests.pop_front();
        }
        self.requests.push_back(request);
    }

    pub(crate) fn estimated_bytes(&self) -> usize {
        self.lines.capacity() * std::mem::size_of::<AlertLine>()
            + self.requests.capacity() * std::mem::size_of::<AlertCreateRequest>()
            + self
                .lines
                .iter()
                .map(|line| {
                    line.id.heap_bytes() + line.label.as_ref().map_or(0, |label| label.capacity())
                })
                .sum::<usize>()
            + self
                .create_icon
                .as_ref()
                .map_or(0, |image| image.pixels.len())
    }
}

fn invalid(message: impl Into<String>) -> ChartError {
    ChartError::new(ErrorCode::InvalidData, message)
}

fn validate_line(line: &AlertLine) -> Result<(), ChartError> {
    AlertId::new(line.id.as_str())?;
    if !line.price.is_finite() {
        return Err(invalid("alert price must be finite"));
    }
    if line
        .label
        .as_ref()
        .is_some_and(|label| label.len() > MAX_ALERT_LABEL_BYTES)
    {
        return Err(invalid(format!(
            "alert label exceeds {MAX_ALERT_LABEL_BYTES} UTF-8 bytes"
        )));
    }
    Ok(())
}

impl ChartEngine {
    pub fn alert_snapshot(&self) -> AlertSnapshot {
        AlertSnapshot {
            lines: self.alert_state.lines.clone(),
        }
    }

    pub fn set_alert_snapshot(&mut self, snapshot: AlertSnapshot) -> Result<(), ChartError> {
        if snapshot.lines.len() > MAX_ALERT_LINES {
            return Err(ChartError::new(
                ErrorCode::ResourceLimit,
                format!("alert snapshot exceeds {MAX_ALERT_LINES} lines"),
            ));
        }
        let mut ids = HashSet::with_capacity(snapshot.lines.len());
        for line in &snapshot.lines {
            validate_line(line)?;
            if !ids.insert(line.id.as_str()) {
                return Err(invalid(format!(
                    "duplicate alert id '{}'",
                    line.id.as_str()
                )));
            }
        }
        self.alert_state.lines = snapshot.lines;
        self.invalidate_frame_trading();
        Ok(())
    }

    pub fn update_alert_line(&mut self, line: AlertLine) -> Result<(), ChartError> {
        validate_line(&line)?;
        if let Some(index) = self
            .alert_state
            .lines
            .iter()
            .position(|candidate| candidate.id == line.id)
        {
            self.alert_state.lines[index] = line;
        } else {
            if self.alert_state.lines.len() == MAX_ALERT_LINES {
                return Err(ChartError::new(
                    ErrorCode::ResourceLimit,
                    format!("alert state exceeds {MAX_ALERT_LINES} lines"),
                ));
            }
            self.alert_state.lines.push(line);
        }
        self.invalidate_frame_trading();
        Ok(())
    }

    pub fn remove_alert_line(&mut self, id: &AlertId) -> bool {
        let before = self.alert_state.lines.len();
        self.alert_state.lines.retain(|line| &line.id != id);
        let changed = before != self.alert_state.lines.len();
        if changed {
            self.invalidate_frame_trading();
        }
        changed
    }

    pub fn set_alert_create_button_visible(&mut self, visible: bool) -> bool {
        if self.alert_state.create_button_visible == visible {
            return false;
        }
        self.alert_state.create_button_visible = visible;
        self.invalidate_frame_axis();
        true
    }

    pub fn alert_create_button_visible(&self) -> bool {
        self.alert_state.create_button_visible
    }

    pub fn take_alert_create_requests(&mut self) -> Vec<AlertCreateRequest> {
        self.alert_state.requests.drain(..).collect()
    }

    pub(crate) fn queue_alert_create_request(
        &mut self,
        pane_index: usize,
        price_scale: AlertPriceScale,
        price: f64,
    ) -> AlertCreateRequest {
        let request = AlertCreateRequest {
            sequence: self.alert_state.next_sequence(),
            pane_index,
            price_scale,
            price,
            condition: AlertCondition::Crossing,
            // the public reference regular-price alerts offer Only Once and Every Time. A host may change
            // this default in its dialog, or use the interval-dependent modes represented above.
            frequency: AlertFrequency::OnlyOnce,
        };
        self.alert_state.push_request(request.clone());
        request
    }

    pub(crate) fn remove_alert_pane(&mut self, index: usize) {
        for line in &mut self.alert_state.lines {
            if line.pane_index == index {
                line.pane_index = PANELESS;
            } else if line.pane_index != PANELESS && line.pane_index > index {
                line.pane_index -= 1;
            }
        }
    }

    pub(crate) fn swap_alert_panes(&mut self, first: usize, second: usize) {
        for line in &mut self.alert_state.lines {
            if line.pane_index == first {
                line.pane_index = second;
            } else if line.pane_index == second {
                line.pane_index = first;
            }
        }
    }

    pub(crate) fn move_alert_pane(&mut self, from: usize, to: usize) {
        for line in &mut self.alert_state.lines {
            let pane = line.pane_index;
            if pane == PANELESS {
                continue;
            }
            line.pane_index = if pane == from {
                to
            } else if from < to && pane > from && pane <= to {
                pane - 1
            } else if to < from && pane >= to && pane < from {
                pane + 1
            } else {
                pane
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aeris_charts_render::{color::Color, draw_list::Prim};

    fn chart_with_market() -> ChartEngine {
        let mut chart = ChartEngine::new(400.0, 240.0, 1.0);
        chart
            .set_series_data(
                0,
                &[10.0, 20.0, 30.0],
                &[99.0, 100.0, 101.0],
                &[102.0, 103.0, 104.0],
                &[98.0, 99.0, 100.0],
                &[101.0, 102.0, 103.0],
            )
            .unwrap();
        chart.time_scale.set_width(400.0);
        chart.fit_content();
        chart.build_frame();
        chart
    }

    fn line(id: &str, status: AlertLineStatus) -> AlertLine {
        AlertLine {
            id: AlertId::new(id).unwrap(),
            pane_index: 0,
            price_scale: AlertPriceScale::Right,
            price: 102.0,
            condition: AlertCondition::Crossing,
            frequency: AlertFrequency::EveryTime,
            status,
            label: None,
        }
    }

    #[test]
    fn snapshot_is_transactional_and_alerts_remain_host_authoritative() {
        let mut chart = chart_with_market();
        chart
            .set_alert_snapshot(AlertSnapshot {
                lines: vec![line("active", AlertLineStatus::Active)],
            })
            .unwrap();
        let before = chart.alert_snapshot();
        let mut invalid = line("bad", AlertLineStatus::Triggered);
        invalid.price = f64::NAN;
        assert!(
            chart
                .set_alert_snapshot(AlertSnapshot {
                    lines: vec![invalid],
                })
                .is_err()
        );
        assert_eq!(chart.alert_snapshot(), before);

        let mut negative = line("negative", AlertLineStatus::Active);
        negative.price = -1.0;
        assert!(chart.update_alert_line(negative).is_ok());
    }

    #[test]
    fn alert_lines_and_axis_indicators_use_the_shared_frame() {
        let mut chart = chart_with_market();
        let mut active = line("active", AlertLineStatus::Active);
        active.label = Some("Demo alert".to_string());
        chart
            .set_alert_snapshot(AlertSnapshot {
                lines: vec![active],
            })
            .unwrap();
        let frame = chart.build_frame();
        let segments = chart.frame_pane_segments(0).unwrap();
        let actionable = &frame.panes[0].main[segments.drawings_end..segments.trading_end];
        assert!(
            actionable
                .iter()
                .any(|primitive| matches!(primitive, Prim::HLine { .. }))
        );
        // The line is named by an attached bell badge — drawn geometry, not a glyph — so the
        // axis tag itself carries nothing but the price, like every other tag.
        let color = chart.alert_color(AlertLineStatus::Active);
        assert!(actionable.iter().any(|primitive| matches!(
            primitive,
            Prim::RoundRect { fill, radii, .. }
                if *fill == color && radii[0] > 0.0 && radii[1] == 0.0
        )));
        assert!(
            actionable
                .iter()
                .all(|primitive| !matches!(primitive, Prim::Text { text, .. } if text == "A"))
        );
        let axis = chart.build_axis_frame(
            100.0,
            |text, _bold| text.len() as f64 * 7.0,
            |text, _bold| text.len() as f64 * 6.0,
        );
        assert!(axis.labels.iter().any(|label| label.text == "102.00"));
        assert!(axis.labels.iter().all(|label| label.text != "Demo alert"));
    }

    #[test]
    fn active_alerts_use_the_theme_muted_text_color() {
        let mut chart = chart_with_market();
        chart
            .options
            .apply_str(r##"{"layout":{"mutedTextColor":"#8a8f98"}}"##)
            .unwrap();
        assert_eq!(
            chart.alert_color(AlertLineStatus::Active),
            Color::rgb(0x8a, 0x8f, 0x98)
        );
        assert_ne!(
            chart.alert_color(AlertLineStatus::Active),
            Color::rgb(0x3e, 0x63, 0xdd)
        );
    }

    #[test]
    fn alert_lines_follow_pane_move_swap_and_removal() {
        let mut chart = chart_with_market();
        assert_eq!(chart.add_pane(false), Some(1));
        assert_eq!(chart.add_pane(false), Some(2));
        let mut alert = line("active", AlertLineStatus::Active);
        alert.pane_index = 2;
        chart
            .set_alert_snapshot(AlertSnapshot { lines: vec![alert] })
            .unwrap();

        assert!(chart.move_pane(2, 0));
        assert_eq!(chart.alert_state.lines[0].pane_index, 0);
        assert!(chart.swap_panes(0, 1));
        assert_eq!(chart.alert_state.lines[0].pane_index, 1);
        assert!(chart.remove_pane(1));
        assert_eq!(chart.alert_state.lines[0].pane_index, PANELESS);
    }

    #[test]
    fn alert_axis_tag_keeps_its_price_coordinate_unless_the_live_price_holds_it() {
        let mut chart = chart_with_market();
        chart
            .set_series_data(
                0,
                &[10.0, 20.0, 30.0],
                &[99.0, 100.0, 101.0],
                &[102.0, 103.0, 104.0],
                &[98.0, 99.0, 100.0],
                &[101.0, 102.0, 102.0],
            )
            .unwrap();
        chart.fit_content();
        let mut away = line("away", AlertLineStatus::Active);
        away.price = 99.0;
        chart
            .set_alert_snapshot(AlertSnapshot {
                lines: vec![line("active", AlertLineStatus::Active), away],
            })
            .unwrap();

        let axis = chart.build_axis_frame(
            100.0,
            |text, _bold| text.len() as f64 * 7.0,
            |text, _bold| text.len() as f64 * 6.0,
        );
        let color = chart.alert_color(AlertLineStatus::Active);
        let tag = |text: &str| {
            axis.labels
                .iter()
                .find(|label| {
                    label.text == text && label.background.is_some_and(|bg| bg.4 == color)
                })
                .expect("alert axis tag")
        };
        let coordinate = |price: f64| {
            chart
                .runtime_price_coordinate(0, PriceScaleTarget::Right, price)
                .expect("populated right scale")
        };
        assert!((tag("99.00").y - coordinate(99.0)).abs() <= f64::EPSILON);
        // The live price owns its slot; the alert at the same price stacks beside it.
        let at_live = tag("102.00").background.unwrap();
        let live = axis
            .labels
            .iter()
            .find(|label| {
                label.text == "102.00" && label.background.is_some_and(|bg| bg.4 != color)
            })
            .and_then(|label| label.background)
            .expect("live price tag");
        assert!(at_live.1 + at_live.3 <= live.1 + 1e-9 || live.1 + live.3 <= at_live.1 + 1e-9);
        assert!((tag("102.00").y - coordinate(102.0)).abs() <= at_live.3 + 1e-9);
    }

    #[test]
    fn crosshair_plus_chip_queues_exact_default_create_request() {
        let mut chart = chart_with_market();
        chart
            .options
            .apply_str(r##"{"crosshair":{"horzLine":{"labelBackgroundColor":"#123456"}}}"##)
            .unwrap();
        let y = chart
            .runtime_price_coordinate(0, PriceScaleTarget::Right, 102.0)
            .expect("populated right scale");
        chart.crosshair = Some((200.0, y));
        let chip = chart
            .alert_create_chip()
            .expect("visible crosshair alert chip");
        let axis = chart.build_axis_frame(
            100.0,
            |text, _bold| text.len() as f64 * 7.0,
            |text, _bold| text.len() as f64 * 6.0,
        );
        // The container fill stays separate from the shared vector icon.
        let container = axis
            .labels
            .iter()
            .find(|label| {
                label.text.is_empty()
                    && label.background.is_some_and(|(bg_x, _, w, h, _)| {
                        (w - chip.size).abs() < 1e-9
                            && (h - chip.size).abs() < 1e-9
                            && (bg_x - (chip.x + chart.pane_left)).abs() < 1e-9
                    })
            })
            .expect("round plus container");
        let (plus_x, _, plus_width, _, plus_color) = container.background.unwrap();
        assert_eq!(plus_color, Color::rgb(0x12, 0x34, 0x56));
        assert_eq!(container.background_corners, crate::AxisLabelCorners::LEFT);
        assert_eq!(container.border, None);
        let icon = axis.crosshair_action_icon.as_ref().unwrap();
        assert_eq!(
            icon.x + icon.side / 2.0,
            chip.x + chart.pane_left + chip.size / 2.0
        );
        assert_eq!(icon.y + icon.side / 2.0, chip.y);
        // The crosshair sits mid-pane here, off the chip: idle styling above.
        // Parking it on the chip lifts the fill a step with no blue anywhere,
        // and the button keeps its geometry and hit rect.
        chart.crosshair = Some((chip.x + chip.size / 2.0, chip.y));
        let hovered_axis = chart.build_axis_frame(
            100.0,
            |text, _bold| text.len() as f64 * 7.0,
            |text, _bold| text.len() as f64 * 6.0,
        );
        let lifted = Color::rgb(0x12, 0x34, 0x56).lighten(0.3);
        let hovered_container = hovered_axis
            .labels
            .iter()
            .find(|label| {
                label.text.is_empty()
                    && label.background.is_some_and(|(_, _, w, h, color)| {
                        color == lifted
                            && (w - chip.size).abs() < 1e-9
                            && (h - chip.size).abs() < 1e-9
                    })
            })
            .expect("hovered plus container lifts with no blue");
        assert_eq!(hovered_container.border, None);
        assert_eq!(
            hovered_container.background_corners,
            crate::AxisLabelCorners::LEFT
        );
        assert_eq!(
            hovered_axis.crosshair_action_icon,
            axis.crosshair_action_icon
        );
        let price_chip = axis
            .labels
            .iter()
            .find(|label| {
                !label.text.is_empty()
                    && label.y == chip.y
                    && label
                        .background
                        .is_some_and(|(_, _, _, _, color)| color == plus_color)
            })
            .expect("primary crosshair price chip");
        assert!(
            (plus_x + plus_width - price_chip.background.unwrap().0).abs() < 1e-9,
            "the plus chip meets the price chip at the same border seam as an attached name chip"
        );
        assert!(chart.alert_create_hit_at(chip.x + chip.size / 2.0, chip.y));
        assert!(chart.activate_alert_create_at(chip.x + chip.size / 2.0, chip.y));
        let requests = chart.take_alert_create_requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].pane_index, 0);
        assert_eq!(requests[0].price_scale, AlertPriceScale::Right);
        assert_eq!(requests[0].condition, AlertCondition::Crossing);
        assert_eq!(requests[0].frequency, AlertFrequency::OnlyOnce);
        assert!((requests[0].price - 102.0).abs() < 1e-9);
    }

    #[test]
    fn crosshair_action_retains_original_svg_pixels_across_frames_and_dpr_changes() {
        let mut chart = chart_with_market();
        let y = chart
            .runtime_price_coordinate(0, PriceScaleTarget::Right, 102.0)
            .unwrap();
        chart.crosshair = Some((200.0, y));
        for dpr in [1.0, 1.25, 1.5, 2.0, 3.0] {
            chart.dpr = dpr;
            let axis = chart.build_axis_frame(
                100.0,
                |text, _| text.len() as f64 * 7.0,
                |text, _| text.len() as f64 * 6.0,
            );
            let icon = axis.crosshair_action_icon.as_ref().unwrap();
            let expected = aeris_charts_render::crosshair_icon::crosshair_icon(
                (19.0 * 0.9 * dpr).round() as u32,
            );
            assert_eq!(icon.image.pixels, expected.pixels);
            let mut prims = Vec::new();
            chart.build_axis_primitives_into(&axis, &mut prims);
            let Some(Prim::Image {
                image,
                rect,
                opacity,
            }) = prims.last()
            else {
                panic!("shared SVG image must paint above the chip")
            };
            assert_eq!(image.pixels, expected.pixels);
            assert!(rect.iter().all(|v| v.fract() == 0.0));
            assert_eq!(*opacity, 1.0);
            let next = chart.build_axis_frame(
                100.0,
                |text, _| text.len() as f64 * 7.0,
                |text, _| text.len() as f64 * 6.0,
            );
            assert!(std::sync::Arc::ptr_eq(
                &icon.image.pixels,
                &next.crosshair_action_icon.unwrap().image.pixels
            ));
        }
    }
}
