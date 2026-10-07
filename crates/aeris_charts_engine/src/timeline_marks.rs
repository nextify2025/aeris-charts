//! Engine-owned timeline marks: a bounded lane of glyph tokens along the bottom of the primary
//! series' pane, one token per bar slot (or per cluster of near slots), separate from the trading
//! host overlay (`HostOverlaySnapshot`) and from series markers.
//!
//! Ownership: the snapshot, hidden groups, lane visibility, hover, the dwell tooltip, the
//! activation ring and the lazily rebuilt lane layout all live in [`TimelineMarksState`] on
//! `ChartEngine`. Hosts supply marks and groups and receive behavior: slot mapping (time → bar
//! slot with the gap and future-projection rules), deterministic clustering in CSS media space,
//! autoscale reservation, hover ring, tooltip, cursor and the click outcome all come from the
//! engine so every host inherits them unchanged.

use std::cell::{Cell, Ref, RefCell};
use std::collections::{BTreeSet, HashSet, VecDeque};

use serde::{Deserialize, Serialize};

use crate::drawings::time_anchor::prevailing_interval;
use crate::interaction::HitProfile;
use crate::{ChartEngine, ChartError, ErrorCode};
use aeris_charts_core::style::DEFAULT_PRIMARY_RGB;
use aeris_charts_render::color::Color;

/// Most marks one snapshot may carry.
pub const MAX_TIMELINE_MARKS: usize = 4_096;
/// Most distinct group ids across `groups` and the groups marks name, and most hidden groups.
pub const MAX_TIMELINE_GROUPS: usize = 64;
const MAX_TIMELINE_ID_BYTES: usize = 128;
const MAX_TIMELINE_TITLE_BYTES: usize = 128;
const MAX_TIMELINE_LABEL_BYTES: usize = 64;
const MAX_TIMELINE_LETTER_CHARS: usize = 2;
/// Resolved click outcomes retained for `timeline_mark_activation`, matching the input-event queue.
pub(crate) const MAX_TIMELINE_ACTIVATIONS: usize = 32;

/// Single-mark token side (CSS px).
pub(crate) const TIMELINE_TOKEN_CSS: f64 = 16.0;
/// Cluster token side (CSS px).
pub(crate) const TIMELINE_CLUSTER_TOKEN_CSS: f64 = 18.0;
/// Gap between tokens (CSS px); a token joins a cluster while its center is within
/// `TIMELINE_TOKEN_CSS + TIMELINE_TOKEN_GAP_CSS` of the cluster anchor.
pub(crate) const TIMELINE_TOKEN_GAP_CSS: f64 = 4.0;
/// Lane height (CSS px).
pub(crate) const TIMELINE_LANE_HEIGHT_CSS: f64 = 24.0;
/// Gap between the lane and the pane bottom (CSS px).
pub(crate) const TIMELINE_LANE_BOTTOM_GAP_CSS: f64 = 3.0;
/// Autoscale margin below the data reserved for the lane (CSS px): lane height plus bottom gap.
pub(crate) const TIMELINE_LANE_RESERVATION_CSS: f64 =
    TIMELINE_LANE_HEIGHT_CSS + TIMELINE_LANE_BOTTOM_GAP_CSS;
/// The lane hides when the pane is shorter than this many lane heights ...
const LANE_HIDE_RATIO: f64 = 4.0;
/// ... and shows again only above this many, so a separator drag cannot toggle the reservation.
const LANE_SHOW_RATIO: f64 = 4.5;
/// Letter and count glyph text size (CSS px); fixed so token geometry never depends on host text.
pub(crate) const TIMELINE_GLYPH_TEXT_CSS: f64 = 10.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimelineGlyphShape {
    #[default]
    Circle,
    Square,
    Diamond,
    Pin,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TimelineMarkGlyph {
    pub shape: TimelineGlyphShape,
    /// CSS color of the token fill.
    pub color: String,
    /// At most two characters printed inside a single-mark token; may be empty.
    pub letter: String,
}

impl Default for TimelineMarkGlyph {
    fn default() -> Self {
        Self {
            shape: TimelineGlyphShape::Circle,
            color: Color::rgb(
                DEFAULT_PRIMARY_RGB.0,
                DEFAULT_PRIMARY_RGB.1,
                DEFAULT_PRIMARY_RGB.2,
            )
            .to_css(),
            letter: String::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TimelineMark {
    pub id: String,
    /// Unix seconds, like bar times.
    pub time: i64,
    pub group: String,
    #[serde(default)]
    pub glyph: TimelineMarkGlyph,
    #[serde(default)]
    pub title: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TimelineMarkGroup {
    pub id: String,
    #[serde(default)]
    pub label: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TimelineMarksSnapshot {
    pub marks: Vec<TimelineMark>,
    pub groups: Vec<TimelineMarkGroup>,
}

/// The token under a point or behind a click: the slot it anchors and every mark it folds.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimelineMarkHit {
    /// Anchor bar slot (logical index; past the last bar when `projected`).
    pub logical: i64,
    /// Time of the earliest mark in the token.
    pub time: i64,
    /// The anchor slot lies in the right-side whitespace (a future time).
    pub projected: bool,
    pub count: usize,
    /// Distinct group ids in mark order.
    pub groups: Vec<String>,
    pub mark_ids: Vec<String>,
    /// Title of the earliest mark.
    pub title: String,
    /// Group label of a single-group token (the id when the group has no label), or
    /// `"N marks"` for a mixed token.
    pub label: String,
}

impl TimelineMarkHit {
    /// The dwell tooltip text: `label · title` for one mark, `label · N` for a single-group
    /// cluster, `N marks` for a mixed one.
    pub fn tooltip_text(&self) -> String {
        if self.count == 1 {
            if self.title.is_empty() {
                self.label.clone()
            } else {
                format!("{} · {}", self.label, self.title)
            }
        } else if self.groups.len() == 1 {
            format!("{} · {}", self.label, self.count)
        } else {
            self.label.clone()
        }
    }
}

/// A mark's bar slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TimelineSlot {
    pub(crate) logical: i64,
    pub(crate) projected: bool,
}

/// One lane token: a single mark or a cluster, positioned in CSS media space.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TimelineToken {
    /// Center x (CSS px, pane-left relative) of the anchor slot.
    pub(crate) x: f64,
    pub(crate) logical: i64,
    pub(crate) projected: bool,
    /// Indices into the sorted marks, earliest first.
    pub(crate) marks: Vec<usize>,
    /// Distinct groups in mark order.
    pub(crate) groups: Vec<String>,
}

impl TimelineToken {
    pub(crate) fn count(&self) -> usize {
        self.marks.len()
    }

    pub(crate) fn size_css(&self) -> f64 {
        if self.count() > 1 {
            TIMELINE_CLUSTER_TOKEN_CSS
        } else {
            TIMELINE_TOKEN_CSS
        }
    }

    pub(crate) fn mixed(&self) -> bool {
        self.groups.len() > 1
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct LaneLayoutKey {
    time_scale_revision: u64,
    time_points_generation: u64,
    time_cutoff: Option<i64>,
    replay_clock_micros: Option<i64>,
    options_generation: u64,
    projection_revision: u64,
    marks_generation: u64,
    layout_generation: u64,
    pane: Option<usize>,
    pane_top_bits: u64,
    pane_height_bits: u64,
    pane_w_bits: u64,
    lane_shown: bool,
}

/// The lane layout shared by the frame builder, hover, cursor and hit testing.
#[derive(Default)]
pub(crate) struct TimelineLaneLayout {
    key: Option<LaneLayoutKey>,
    /// The primary series' pane while the lane is shown there.
    pub(crate) pane: Option<usize>,
    /// Token center y (CSS px).
    pub(crate) center_y: f64,
    /// Tokens ascending by x.
    pub(crate) tokens: Vec<TimelineToken>,
}

impl TimelineLaneLayout {
    fn capacity_bytes(&self) -> usize {
        self.tokens.capacity() * std::mem::size_of::<TimelineToken>()
            + self
                .tokens
                .iter()
                .map(|token| {
                    token.marks.capacity() * std::mem::size_of::<usize>()
                        + token.groups.iter().map(String::capacity).sum::<usize>()
                })
                .sum::<usize>()
    }
}

/// Runtime state of the lane. Only `hidden_groups` persists (V1/V2/V3 `hidden_mark_groups`).
pub(crate) struct TimelineMarksState {
    /// Marks sorted by `(time, group, id)`.
    snapshot: TimelineMarksSnapshot,
    pub(crate) hidden_groups: BTreeSet<String>,
    visible: bool,
    /// Bumped by every marks/visibility/hidden-group mutation; keys the layout cache.
    pub(crate) generation: u64,
    /// Anchor logical of the hovered token.
    hovered: Option<i64>,
    pub(crate) tooltip_armed: bool,
    /// Last tall-enough decision (hysteresis memory); `None` before the first decision.
    lane_shown: Cell<Option<bool>>,
    layout: RefCell<TimelineLaneLayout>,
    activations: VecDeque<(u32, TimelineMarkHit)>,
    next_activation_seq: u32,
}

impl Default for TimelineMarksState {
    fn default() -> Self {
        Self {
            snapshot: TimelineMarksSnapshot::default(),
            hidden_groups: BTreeSet::new(),
            visible: true,
            generation: 1,
            hovered: None,
            tooltip_armed: false,
            lane_shown: Cell::new(None),
            layout: RefCell::new(TimelineLaneLayout::default()),
            activations: VecDeque::new(),
            next_activation_seq: 1,
        }
    }
}

impl TimelineMarksState {
    pub(crate) fn capacity_bytes(&self) -> usize {
        let marks = self.snapshot.marks.capacity() * std::mem::size_of::<TimelineMark>()
            + self
                .snapshot
                .marks
                .iter()
                .map(|mark| {
                    mark.id.capacity()
                        + mark.group.capacity()
                        + mark.title.capacity()
                        + mark.glyph.color.capacity()
                        + mark.glyph.letter.capacity()
                })
                .sum::<usize>();
        let groups = self.snapshot.groups.capacity() * std::mem::size_of::<TimelineMarkGroup>()
            + self
                .snapshot
                .groups
                .iter()
                .map(|group| group.id.capacity() + group.label.capacity())
                .sum::<usize>();
        let hidden = self
            .hidden_groups
            .iter()
            .map(String::capacity)
            .sum::<usize>();
        let activations = self.activations.capacity()
            * std::mem::size_of::<(u32, TimelineMarkHit)>()
            + self
                .activations
                .iter()
                .map(|(_, hit)| {
                    hit.title.capacity()
                        + hit.label.capacity()
                        + hit.groups.iter().map(String::capacity).sum::<usize>()
                        + hit.mark_ids.iter().map(String::capacity).sum::<usize>()
                })
                .sum::<usize>();
        marks + groups + hidden + activations + self.layout.borrow().capacity_bytes()
    }

    fn bump(&mut self) {
        self.generation = self.generation.wrapping_add(1).max(1);
        self.hovered = None;
        self.tooltip_armed = false;
    }

    /// Replace the hidden set with an already-validated one (persistence installers).
    pub(crate) fn install_hidden_groups(&mut self, groups: impl IntoIterator<Item = String>) {
        self.hidden_groups = groups.into_iter().collect();
        self.bump();
    }

    pub(crate) fn hovered(&self) -> Option<i64> {
        self.hovered
    }
}

fn invalid(message: impl Into<String>) -> ChartError {
    ChartError::new(ErrorCode::InvalidData, message)
}

fn validate_group_id(kind: &str, value: &str) -> Result<(), ChartError> {
    if value.is_empty() || value.len() > MAX_TIMELINE_ID_BYTES {
        return Err(invalid(format!(
            "{kind} must contain 1..={MAX_TIMELINE_ID_BYTES} UTF-8 bytes"
        )));
    }
    Ok(())
}

fn validate_snapshot(snapshot: &TimelineMarksSnapshot) -> Result<(), ChartError> {
    if snapshot.marks.len() > MAX_TIMELINE_MARKS {
        return Err(ChartError::new(
            ErrorCode::ResourceLimit,
            format!("timeline marks exceed {MAX_TIMELINE_MARKS} entries"),
        ));
    }
    let mut group_ids = HashSet::new();
    for group in &snapshot.groups {
        validate_group_id("TimelineMarkGroup.id", &group.id)?;
        if group.label.len() > MAX_TIMELINE_LABEL_BYTES {
            return Err(invalid("timeline mark group label is too long"));
        }
        if !group_ids.insert(group.id.as_str()) {
            return Err(invalid(format!(
                "duplicate timeline mark group id '{}'",
                group.id
            )));
        }
    }
    let mut mark_ids = HashSet::new();
    for mark in &snapshot.marks {
        validate_group_id("TimelineMark.id", &mark.id)?;
        validate_group_id("TimelineMark.group", &mark.group)?;
        if mark.title.len() > MAX_TIMELINE_TITLE_BYTES {
            return Err(invalid("timeline mark title is too long"));
        }
        if mark.glyph.letter.chars().count() > MAX_TIMELINE_LETTER_CHARS {
            return Err(invalid("timeline mark letter exceeds two characters"));
        }
        if Color::parse_css(&mark.glyph.color).is_none() {
            return Err(invalid(format!(
                "timeline mark color '{}' is not a CSS color",
                mark.glyph.color
            )));
        }
        if !mark_ids.insert(mark.id.as_str()) {
            return Err(invalid(format!("duplicate timeline mark id '{}'", mark.id)));
        }
        group_ids.insert(mark.group.as_str());
    }
    if group_ids.len() > MAX_TIMELINE_GROUPS {
        return Err(ChartError::new(
            ErrorCode::ResourceLimit,
            format!("timeline marks name more than {MAX_TIMELINE_GROUPS} groups"),
        ));
    }
    Ok(())
}

/// Validate a persisted hidden-group list (the same bounds as the live setter).
pub(crate) fn validate_hidden_groups(groups: &[String]) -> Result<(), ChartError> {
    if groups.len() > MAX_TIMELINE_GROUPS {
        return Err(ChartError::new(
            ErrorCode::ResourceLimit,
            format!("hidden mark groups exceed {MAX_TIMELINE_GROUPS} entries"),
        ));
    }
    for group in groups {
        validate_group_id("hidden_mark_groups entry", group)?;
    }
    Ok(())
}

impl ChartEngine {
    /// Replace the marks and groups atomically. Hidden groups are independent of the snapshot and
    /// stay as they are. Rebuilds chrome, autoscale (the lane reservation), axis and overlay.
    pub fn set_timeline_marks(
        &mut self,
        mut snapshot: TimelineMarksSnapshot,
    ) -> Result<(), ChartError> {
        validate_snapshot(&snapshot)?;
        snapshot
            .marks
            .sort_by(|a, b| (a.time, &a.group, &a.id).cmp(&(b.time, &b.group, &b.id)));
        self.timeline_marks.snapshot = snapshot;
        self.timeline_marks.bump();
        self.invalidate_frame_chrome();
        Ok(())
    }

    #[must_use]
    pub fn timeline_marks(&self) -> &TimelineMarksSnapshot {
        &self.timeline_marks.snapshot
    }

    /// Show or hide the whole lane (runtime, default shown). Hiding releases the reservation.
    pub fn set_timeline_marks_visible(&mut self, visible: bool) -> bool {
        if self.timeline_marks.visible == visible {
            return false;
        }
        self.timeline_marks.visible = visible;
        self.timeline_marks.bump();
        self.invalidate_frame_chrome();
        true
    }

    #[must_use]
    pub fn timeline_marks_visible(&self) -> bool {
        self.timeline_marks.visible
    }

    /// Hide or show one group's marks. The set is independent of the snapshot — hosts import a
    /// document first and set marks later — and persists. `Ok(changed)`.
    pub fn set_timeline_group_hidden(
        &mut self,
        group: &str,
        hidden: bool,
    ) -> Result<bool, ChartError> {
        validate_group_id("timeline mark group", group)?;
        let changed = if hidden {
            if !self.timeline_marks.hidden_groups.contains(group) {
                if self.timeline_marks.hidden_groups.len() >= MAX_TIMELINE_GROUPS {
                    return Err(ChartError::new(
                        ErrorCode::ResourceLimit,
                        format!("hidden mark groups exceed {MAX_TIMELINE_GROUPS} entries"),
                    ));
                }
                self.timeline_marks.hidden_groups.insert(group.to_string())
            } else {
                false
            }
        } else {
            self.timeline_marks.hidden_groups.remove(group)
        };
        if changed {
            self.timeline_marks.bump();
            // The reservation is independent of hidden groups: no autoscale run.
            self.invalidate_frame_chrome_presentation();
        }
        Ok(changed)
    }

    #[must_use]
    pub fn hidden_timeline_groups(&self) -> Vec<String> {
        self.timeline_marks.hidden_groups.iter().cloned().collect()
    }

    /// The token under `(x_css, y_css)` with the pointer's precision tolerance.
    #[must_use]
    pub fn timeline_mark_hit_at(&self, x_css: f64, y_css: f64) -> Option<TimelineMarkHit> {
        self.timeline_mark_hit_at_with_profile(x_css, y_css, HitProfile::PRECISION)
    }

    /// [`Self::timeline_mark_hit_at`] with a device hit profile (touch widens the token box).
    #[must_use]
    pub fn timeline_mark_hit_at_with_profile(
        &self,
        x_css: f64,
        y_css: f64,
        profile: HitProfile,
    ) -> Option<TimelineMarkHit> {
        let token = self.timeline_token_at(x_css, y_css, profile)?;
        let layout = self.lane_layout();
        Some(self.timeline_hit_for_token(&layout.tokens[token]))
    }

    /// Whether a lane token lies under the pointer: the allocation-free answer hover
    /// arbitration, cursor choice and click guards need on every pointer move.
    pub(crate) fn timeline_token_under(&self, x_css: f64, y_css: f64) -> bool {
        self.timeline_token_at(x_css, y_css, HitProfile::PRECISION)
            .is_some()
    }

    /// The resolved outcome of the click that emitted `ChartInputEvent::TimelineMarkActivated(seq)`.
    #[must_use]
    pub fn timeline_mark_activation(&self, seq: u32) -> Option<&TimelineMarkHit> {
        self.timeline_marks
            .activations
            .iter()
            .find_map(|(stored, hit)| (*stored == seq).then_some(hit))
    }

    /// The token folding the mark `id` while it is in view (keyboard activation).
    #[must_use]
    pub fn timeline_mark_hit_for_id(&self, id: &str) -> Option<TimelineMarkHit> {
        if self.timeline_marks.snapshot.marks.is_empty() || !self.timeline_marks.visible {
            return None;
        }
        let layout = self.lane_layout();
        let marks = &self.timeline_marks.snapshot.marks;
        let token = layout
            .tokens
            .iter()
            .find(|token| token.marks.iter().any(|&index| marks[index].id == id))?;
        Some(self.timeline_hit_for_token(token))
    }

    pub(crate) fn push_timeline_activation(&mut self, hit: TimelineMarkHit) -> u32 {
        let seq = self.timeline_marks.next_activation_seq;
        self.timeline_marks.next_activation_seq = seq.wrapping_add(1).max(1);
        if self.timeline_marks.activations.len() == MAX_TIMELINE_ACTIVATIONS {
            self.timeline_marks.activations.pop_front();
        }
        self.timeline_marks.activations.push_back((seq, hit));
        seq
    }

    /// Hover promotion at the pointer: returns whether the hovered token changed. A change
    /// disarms the tooltip; the controller restarts the dwell.
    pub(crate) fn set_timeline_hover(&mut self, x_css: f64, y_css: f64) -> bool {
        let next = self
            .timeline_token_at(x_css, y_css, HitProfile::PRECISION)
            .map(|index| self.lane_layout().tokens[index].logical);
        if next == self.timeline_marks.hovered {
            return false;
        }
        self.timeline_marks.hovered = next;
        self.timeline_marks.tooltip_armed = false;
        self.invalidate_frame_overlay();
        true
    }

    /// Whether the hovered token's title tooltip is not shown yet. The shared hover dwell runs
    /// while this (or the trading counterpart) holds, and only then can it arm the tooltip.
    pub(crate) fn timeline_tooltip_pending(&self) -> bool {
        self.timeline_marks.hovered().is_some() && !self.timeline_marks.tooltip_armed
    }

    pub(crate) fn clear_timeline_hover(&mut self) -> bool {
        if self.timeline_marks.hovered.is_none() && !self.timeline_marks.tooltip_armed {
            return false;
        }
        self.timeline_marks.hovered = None;
        self.timeline_marks.tooltip_armed = false;
        self.invalidate_frame_overlay();
        true
    }

    /// Reveal the hovered token's title tooltip once the host's dwell elapsed (`input_tick`).
    pub(crate) fn arm_timeline_tooltip(&mut self) -> bool {
        if self.timeline_marks.hovered.is_none() || self.timeline_marks.tooltip_armed {
            return false;
        }
        self.timeline_marks.tooltip_armed = true;
        self.invalidate_frame_overlay();
        true
    }

    /// The hovered token, if it is still part of the current layout.
    pub(crate) fn hovered_timeline_token(&self) -> Option<TimelineToken> {
        let hovered = self.timeline_marks.hovered?;
        let layout = self.lane_layout();
        let index = layout
            .tokens
            .binary_search_by(|token| token.logical.cmp(&hovered))
            .ok()?;
        Some(layout.tokens[index].clone())
    }

    /// The index into the lane layout of the token under `(x_css, y_css)`, without resolving
    /// the hit. A token answers across its own box (16 CSS px, 18 for a cluster), widened by
    /// the profile's slack for touch.
    pub(crate) fn timeline_token_at(
        &self,
        x_css: f64,
        y_css: f64,
        profile: HitProfile,
    ) -> Option<usize> {
        if self.timeline_marks.snapshot.marks.is_empty() || !self.timeline_marks.visible {
            return None;
        }
        if !x_css.is_finite() || !y_css.is_finite() {
            return None;
        }
        let pane = self.pane_at_y(y_css)?;
        let layout = self.lane_layout();
        if layout.pane != Some(pane) || layout.tokens.is_empty() {
            return None;
        }
        // Touch answers across a finger-sized box; precision across the token's own box. The
        // largest token bounds the early exit; each candidate then checks its own size.
        let slack = (profile.trading_line_tolerance - HitProfile::PRECISION.trading_line_tolerance)
            .max(0.0);
        let dy = (y_css - layout.center_y).abs();
        if dy > TIMELINE_CLUSTER_TOKEN_CSS / 2.0 + slack {
            return None;
        }
        // Tokens ascend by x: the candidate is the last token whose center is at or left of the
        // pointer, or the first one right of it.
        let right = layout.tokens.partition_point(|token| token.x <= x_css);
        let candidates = [
            right.checked_sub(1),
            (right < layout.tokens.len()).then_some(right),
        ];
        candidates
            .into_iter()
            .flatten()
            .filter(|&index| {
                let token = &layout.tokens[index];
                let half = token.size_css() / 2.0 + slack;
                (x_css - token.x).abs() <= half && dy <= half
            })
            .min_by(|&a, &b| {
                (x_css - layout.tokens[a].x)
                    .abs()
                    .total_cmp(&(x_css - layout.tokens[b].x).abs())
            })
    }

    fn timeline_hit_for_token(&self, token: &TimelineToken) -> TimelineMarkHit {
        let marks = &self.timeline_marks.snapshot.marks;
        let first = &marks[token.marks[0]];
        let label = if token.mixed() {
            format!("{} marks", token.count())
        } else {
            self.timeline_group_label(&first.group)
        };
        TimelineMarkHit {
            logical: token.logical,
            time: first.time,
            projected: token.projected,
            count: token.count(),
            groups: token.groups.clone(),
            mark_ids: token
                .marks
                .iter()
                .map(|&index| marks[index].id.clone())
                .collect(),
            title: first.title.clone(),
            label,
        }
    }

    pub(crate) fn timeline_group_label(&self, group: &str) -> String {
        self.timeline_marks
            .snapshot
            .groups
            .iter()
            .find(|candidate| candidate.id == group)
            .filter(|candidate| !candidate.label.is_empty())
            .map_or_else(|| group.to_string(), |candidate| candidate.label.clone())
    }

    /// The projection step (seconds): an installed session bar grid, then the future-time
    /// projection cadence, then the prevailing bar interval.
    fn timeline_step_seconds(&self, times: &[i64]) -> Option<i64> {
        if let Some(grid) = self.bar_label_grid.as_ref() {
            let interval = i64::from(grid.interval_seconds());
            if interval > 0 {
                return Some(interval);
            }
        }
        if let Some((step, _)) = self.future_time_projection
            && step > 0
        {
            return Some(step);
        }
        prevailing_interval(times, true).map(|step| step as i64)
    }

    /// The projection step of the current axis, or `None` on a sequence axis (which maps by
    /// open..close spans) and when no cadence is known. Constant for a whole lane layout.
    fn timeline_time_step(&self) -> Option<i64> {
        if self.sequence_points().is_some() {
            return None;
        }
        self.timeline_step_seconds(self.data.merged_times())
    }

    /// The bar slot that holds `time`: the bar whose span (`open .. open + min(step, next_open -
    /// open)`) contains it; a time in a gap (a weekend) lands on the next bar; a time past the
    /// last bar's span projects into the right-side whitespace by whole steps. `None` before the
    /// first bar, after the replay cutoff, or past the last bar when no step is known. `step` is
    /// [`Self::timeline_time_step`], resolved once by the caller because the lane layout maps
    /// every visible mark with the same step.
    fn timeline_slot_for_time(&self, time: i64, step: Option<i64>) -> Option<TimelineSlot> {
        if !self.replay_time_is_visible(time) {
            return None;
        }
        if let Some(points) = self.sequence_points() {
            // Sequence axes (tick/volume/range bars): the open..close span, no projection.
            let micros = time.checked_mul(1_000_000)?;
            let index = points
                .partition_point(|point| point.open_timestamp_micros <= micros)
                .checked_sub(1)?;
            let point = &points[index];
            let inside =
                micros < point.close_timestamp_micros || micros == point.open_timestamp_micros;
            let slot = if inside {
                index
            } else if index + 1 < points.len() {
                index + 1
            } else {
                return None;
            };
            return Some(TimelineSlot {
                logical: slot as i64,
                projected: false,
            });
        }
        let times = self.data.merged_times();
        let &first = times.first()?;
        if time < first {
            return None;
        }
        let n = times.len();
        let index = times.partition_point(|&open| open <= time) - 1;
        let open = times[index];
        let Some(step) = step else {
            return (time == open).then_some(TimelineSlot {
                logical: index as i64,
                projected: false,
            });
        };
        let span = if index + 1 < n {
            step.min(times[index + 1] - open)
        } else {
            step
        };
        if time < open.saturating_add(span) {
            return Some(TimelineSlot {
                logical: index as i64,
                projected: false,
            });
        }
        if index + 1 < n {
            return Some(TimelineSlot {
                logical: (index + 1) as i64,
                projected: false,
            });
        }
        let steps = (time - open).div_euclid(step);
        Some(TimelineSlot {
            logical: (n as i64 - 1).checked_add(steps)?,
            projected: true,
        })
    }

    /// Whether the lane fits the pane, with hysteresis: it hides below `4 × lane height` and
    /// shows again only above `4.5 ×`, so a separator drag cannot toggle the reservation per frame.
    fn timeline_lane_shown(&self, pane_height: f64) -> bool {
        let hide_below = LANE_HIDE_RATIO * TIMELINE_LANE_HEIGHT_CSS;
        let show_above = LANE_SHOW_RATIO * TIMELINE_LANE_HEIGHT_CSS;
        let shown = match self.timeline_marks.lane_shown.get() {
            None => pane_height >= hide_below,
            Some(true) => pane_height >= hide_below,
            Some(false) => pane_height > show_above,
        };
        self.timeline_marks.lane_shown.set(Some(shown));
        shown
    }

    /// The pane that shows the lane now: the primary series' pane while the lane is enabled,
    /// the snapshot is non-empty and the pane is tall enough. Independent of the view and of
    /// hidden groups, so panning or toggling a group never moves the autoscale reservation.
    /// Hosts attach keyboard focus targets for marks to this pane and drop them while it is
    /// `None`.
    #[must_use]
    pub fn timeline_lane_pane(&self) -> Option<usize> {
        if !self.timeline_marks.visible || self.timeline_marks.snapshot.marks.is_empty() {
            return None;
        }
        let pane = self.primary_series()?.pane_index;
        let height = self.panes.get(pane)?.height;
        self.timeline_lane_shown(height).then_some(pane)
    }

    fn lane_layout_key(&self) -> LaneLayoutKey {
        let pane = self.timeline_lane_pane();
        let (top, height) = pane
            .and_then(|pane| self.panes.get(pane))
            .map_or((0.0, 0.0), |pane| (pane.top, pane.height));
        LaneLayoutKey {
            time_scale_revision: self.time_scale.revision(),
            time_points_generation: self.data.time_points_generation(),
            time_cutoff: self.data.time_cutoff(),
            replay_clock_micros: self.replay_clock_micros,
            options_generation: self.options.generation(),
            projection_revision: self.future_time_projection_revision,
            marks_generation: self.timeline_marks.generation,
            layout_generation: self.frame_layout_generation(),
            pane,
            pane_top_bits: top.to_bits(),
            pane_height_bits: height.to_bits(),
            pane_w_bits: self.pane_w.to_bits(),
            lane_shown: pane.is_some(),
        }
    }

    /// The lane layout for the current view, rebuilt lazily by whichever caller comes first
    /// (frame builder, hit test, hover or cursor — hover and cursor run on every pointer move
    /// before any frame is built).
    pub(crate) fn lane_layout(&self) -> Ref<'_, TimelineLaneLayout> {
        let key = self.lane_layout_key();
        if self.timeline_marks.layout.borrow().key != Some(key) {
            let layout = self.build_lane_layout(key);
            *self.timeline_marks.layout.borrow_mut() = layout;
        }
        self.timeline_marks.layout.borrow()
    }

    fn build_lane_layout(&self, key: LaneLayoutKey) -> TimelineLaneLayout {
        let mut layout = TimelineLaneLayout {
            key: Some(key),
            pane: key.pane,
            center_y: 0.0,
            tokens: Vec::new(),
        };
        let Some(pane_index) = key.pane else {
            return layout;
        };
        let Some(pane) = self.panes.get(pane_index) else {
            return layout;
        };
        layout.center_y =
            pane.top + pane.height - TIMELINE_LANE_BOTTOM_GAP_CSS - TIMELINE_LANE_HEIGHT_CSS / 2.0;
        let Some(range) = self.time_scale.visible_logical_range() else {
            return layout;
        };
        let left = range.left().floor() as i64;
        let right = range.right().ceil() as i64;
        // Lower time bound: a gap mark lands on the NEXT bar, so start one bar before the window.
        let lower_time = if let Some(points) = self.sequence_points() {
            if points.is_empty() {
                return layout;
            }
            let index = usize::try_from(left - 1).unwrap_or(0).min(points.len() - 1);
            points[index].open_timestamp_micros.div_euclid(1_000_000)
        } else {
            let times = self.data.merged_times();
            if times.is_empty() {
                return layout;
            }
            let index = usize::try_from(left - 1).unwrap_or(0).min(times.len() - 1);
            times[index]
        };
        let marks = &self.timeline_marks.snapshot.marks;
        let start = marks.partition_point(|mark| mark.time < lower_time);
        let join_distance = TIMELINE_TOKEN_CSS + TIMELINE_TOKEN_GAP_CSS;
        let step = self.timeline_time_step();
        for (index, mark) in marks.iter().enumerate().skip(start) {
            if !self.replay_time_is_visible(mark.time) {
                break;
            }
            if self.timeline_marks.hidden_groups.contains(&mark.group) {
                continue;
            }
            let Some(slot) = self.timeline_slot_for_time(mark.time, step) else {
                continue;
            };
            if slot.logical < left {
                continue;
            }
            if slot.logical > right {
                break;
            }
            let x = self.time_scale.logical_to_coordinate(slot.logical as f64);
            match layout.tokens.last_mut() {
                Some(token) if x - token.x <= join_distance => {
                    token.marks.push(index);
                    if !token.groups.contains(&mark.group) {
                        token.groups.push(mark.group.clone());
                    }
                }
                _ => layout.tokens.push(TimelineToken {
                    x,
                    logical: slot.logical,
                    projected: slot.projected,
                    marks: vec![index],
                    groups: vec![mark.group.clone()],
                }),
            }
        }
        layout
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SeriesKind;

    fn mark(id: &str, time: i64, group: &str) -> TimelineMark {
        TimelineMark {
            id: id.to_string(),
            time,
            group: group.to_string(),
            glyph: TimelineMarkGlyph::default(),
            title: format!("{id} title"),
        }
    }

    fn snapshot(marks: Vec<TimelineMark>) -> TimelineMarksSnapshot {
        TimelineMarksSnapshot {
            marks,
            groups: vec![TimelineMarkGroup {
                id: "earnings".into(),
                label: "Earnings".into(),
            }],
        }
    }

    /// Hourly bars over Mon..Fri 09:00-16:00 with a weekend gap between day 5 and day 8.
    fn weekly_chart() -> (ChartEngine, Vec<i64>) {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        chart.series[0].kind = SeriesKind::Line;
        let mut times = Vec::new();
        for day in [0i64, 1, 2, 3, 4, 7, 8, 9, 10, 11] {
            for hour in 9..=16 {
                times.push(1_700_000_000 + day * 86_400 + hour * 3_600);
            }
        }
        let t: Vec<f64> = times.iter().map(|&t| t as f64).collect();
        let v: Vec<f64> = (0..t.len()).map(|i| 100.0 + i as f64).collect();
        chart.set_series_data(0, &t, &v, &v, &v, &v).unwrap();
        chart.recompute_layout_with_measure(true, |_, _| 48.0, |_, _| 48.0);
        chart.fit_content();
        (chart, times)
    }

    #[test]
    fn slot_mapping_follows_the_span_gap_and_projection_rules() {
        let (chart, times) = weekly_chart();
        let slot = |time: i64| chart.timeline_slot_for_time(time, chart.timeline_time_step());
        let n = times.len() as i64;
        // Inside a span and exactly on an open.
        assert_eq!(slot(times[3]).unwrap().logical, 3);
        assert_eq!(slot(times[3] + 1_799).unwrap().logical, 3);
        // The overnight gap (Mon 17:00 .. Tue 09:00) belongs to the next bar, not the 16:00 bar.
        assert_eq!(slot(times[7] + 3_600 + 10).unwrap().logical, 8);
        // Saturday lands on Monday 09:00 (index 40), never on Friday's last bar (index 39).
        let saturday = 1_700_000_000 + 5 * 86_400 + 12 * 3_600;
        assert_eq!(slot(saturday).unwrap().logical, 40);
        assert!(!slot(saturday).unwrap().projected);
        // Future times project by whole prevailing steps (one hour) past the last bar.
        let last = *times.last().unwrap();
        let projected = slot(last + 3 * 3_600 + 5).unwrap();
        assert_eq!(projected.logical, n - 1 + 3);
        assert!(projected.projected);
        assert!(!slot(last + 3_599).unwrap().projected);
        // Before the first bar nothing draws.
        assert_eq!(slot(times[0] - 1), None);
    }

    #[test]
    fn slot_mapping_without_a_step_only_matches_the_last_open() {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        chart.series[0].kind = SeriesKind::Line;
        chart
            .set_series_data(0, &[1_000.0], &[1.0], &[1.0], &[1.0], &[1.0])
            .unwrap();
        let slot = |chart: &ChartEngine, time: i64| {
            chart.timeline_slot_for_time(time, chart.timeline_time_step())
        };
        assert_eq!(slot(&chart, 1_000).unwrap().logical, 0);
        assert_eq!(slot(&chart, 1_001), None);
        assert_eq!(slot(&chart, 999), None);
        // A projection cadence supplies the step a single bar cannot.
        chart.set_future_time_projection(Some(60), 10);
        let projected = slot(&chart, 1_125).unwrap();
        assert_eq!((projected.logical, projected.projected), (2, true));
    }

    #[test]
    fn replay_cutoff_hides_marks_before_they_can_project() {
        let (mut chart, times) = weekly_chart();
        chart
            .set_replay_clock_micros(Some(times[20] * 1_000_000))
            .unwrap();
        let slot = |time: i64| chart.timeline_slot_for_time(time, chart.timeline_time_step());
        assert_eq!(slot(times[20]).unwrap().logical, 20);
        assert_eq!(slot(times[20] + 1), None);
        assert_eq!(slot(times[70] + 86_400), None);
    }

    #[test]
    fn validation_is_atomic_and_bounded() {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        chart
            .set_timeline_marks(snapshot(vec![mark("a", 1_000, "earnings")]))
            .unwrap();
        let before = chart.timeline_marks().clone();
        let too_many: Vec<_> = (0..=MAX_TIMELINE_MARKS)
            .map(|i| mark(&format!("m{i}"), 1_000 + i as i64, "g"))
            .collect();
        let error = chart.set_timeline_marks(snapshot(too_many)).unwrap_err();
        assert_eq!(error.code(), ErrorCode::ResourceLimit);
        let too_many_groups: Vec<_> = (0..=MAX_TIMELINE_GROUPS)
            .map(|i| mark(&format!("m{i}"), 1_000, &format!("g{i}")))
            .collect();
        assert_eq!(
            chart
                .set_timeline_marks(snapshot(too_many_groups))
                .unwrap_err()
                .code(),
            ErrorCode::ResourceLimit
        );
        let duplicate = vec![mark("a", 1, "g"), mark("a", 2, "g")];
        assert_eq!(
            chart
                .set_timeline_marks(snapshot(duplicate))
                .unwrap_err()
                .code(),
            ErrorCode::InvalidData
        );
        let mut long_letter = mark("b", 1, "g");
        long_letter.glyph.letter = "abc".into();
        assert_eq!(
            chart
                .set_timeline_marks(snapshot(vec![long_letter]))
                .unwrap_err()
                .code(),
            ErrorCode::InvalidData
        );
        let mut bad_color = mark("c", 1, "g");
        bad_color.glyph.color = "not-a-color".into();
        assert_eq!(
            chart
                .set_timeline_marks(snapshot(vec![bad_color]))
                .unwrap_err()
                .code(),
            ErrorCode::InvalidData
        );
        let mut long_id = mark("d", 1, "g");
        long_id.id = "x".repeat(MAX_TIMELINE_ID_BYTES + 1);
        assert_eq!(
            chart
                .set_timeline_marks(snapshot(vec![long_id]))
                .unwrap_err()
                .code(),
            ErrorCode::InvalidData
        );
        assert_eq!(
            chart.timeline_marks(),
            &before,
            "a rejected snapshot changes nothing"
        );
        assert!(
            serde_json::from_str::<TimelineMarksSnapshot>(
                r#"{"marks":[{"id":"a","time":1,"group":"g","extra":1}]}"#
            )
            .is_err()
        );
    }

    #[test]
    fn marks_sort_by_time_group_and_id_and_a_mark_may_name_an_unlisted_group() {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        chart
            .set_timeline_marks(snapshot(vec![
                mark("z", 2_000, "news"),
                mark("b", 1_000, "earnings"),
                mark("a", 1_000, "earnings"),
                mark("c", 1_000, "dividends"),
            ]))
            .unwrap();
        let ids: Vec<_> = chart
            .timeline_marks()
            .marks
            .iter()
            .map(|mark| mark.id.as_str())
            .collect();
        assert_eq!(ids, ["c", "a", "b", "z"]);
        assert_eq!(chart.timeline_group_label("news"), "news");
        assert_eq!(chart.timeline_group_label("earnings"), "Earnings");
    }

    #[test]
    fn hidden_groups_are_independent_of_the_snapshot() {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        assert!(chart.set_timeline_group_hidden("news", true).unwrap());
        assert!(!chart.set_timeline_group_hidden("news", true).unwrap());
        assert_eq!(chart.hidden_timeline_groups(), ["news"]);
        chart
            .set_timeline_marks(snapshot(vec![mark("a", 1_000, "earnings")]))
            .unwrap();
        assert_eq!(
            chart.hidden_timeline_groups(),
            ["news"],
            "set_timeline_marks never prunes"
        );
        assert!(chart.set_timeline_group_hidden("news", false).unwrap());
        assert!(chart.hidden_timeline_groups().is_empty());
        for i in 0..MAX_TIMELINE_GROUPS {
            chart
                .set_timeline_group_hidden(&format!("g{i}"), true)
                .unwrap();
        }
        assert_eq!(
            chart
                .set_timeline_group_hidden("one-too-many", true)
                .unwrap_err()
                .code(),
            ErrorCode::ResourceLimit
        );
        assert_eq!(
            chart
                .set_timeline_group_hidden(&"x".repeat(MAX_TIMELINE_ID_BYTES + 1), true)
                .unwrap_err()
                .code(),
            ErrorCode::InvalidData
        );
    }

    #[test]
    fn clustering_is_deterministic_across_pixel_ratios_and_anchors_on_the_earliest_slot() {
        let tokens_at = |dpr: f64| {
            let (mut chart, times) = weekly_chart();
            chart.dpr = dpr;
            chart
                .set_timeline_marks(snapshot(vec![
                    mark("a", times[10], "earnings"),
                    mark("b", times[11], "earnings"),
                    mark("c", times[12], "news"),
                    mark("d", times[40], "earnings"),
                    mark("e", times[40], "earnings"),
                ]))
                .unwrap();
            let layout = chart.lane_layout();
            layout
                .tokens
                .iter()
                .map(|token| (token.logical, token.count(), token.mixed()))
                .collect::<Vec<_>>()
        };
        let at_one = tokens_at(1.0);
        assert_eq!(at_one, tokens_at(1.5));
        assert_eq!(at_one, tokens_at(2.0));
        // 80 bars across 800 px: 10 px per bar, so slots 10..12 fold into one mixed cluster
        // anchored on slot 10, and the same-slot pair folds into a single-group cluster.
        assert_eq!(at_one, vec![(10, 3, true), (40, 2, false)]);
        // A cluster token is 18 px, so its hit box is one pixel larger on every side.
        let (mut chart, times) = weekly_chart();
        chart
            .set_timeline_marks(snapshot(vec![
                mark("d", times[40], "earnings"),
                mark("e", times[40], "earnings"),
            ]))
            .unwrap();
        let x = chart.time_scale.index_to_coordinate(40);
        let y = chart.panes[0].top + chart.panes[0].height - 15.0;
        assert_eq!(
            chart.timeline_mark_hit_at(x, y).map(|hit| hit.count),
            Some(2)
        );
        assert!(chart.timeline_mark_hit_at(x + 9.0, y - 9.0).is_some());
        assert_eq!(chart.timeline_mark_hit_at(x, y - 10.0), None);
        assert_eq!(chart.timeline_mark_hit_at(x + 10.0, y), None);
    }

    #[test]
    fn cluster_anchor_stays_stable_while_zooming_out() {
        let (mut chart, times) = weekly_chart();
        chart
            .set_timeline_marks(snapshot(vec![
                mark("a", times[10], "earnings"),
                mark("b", times[14], "earnings"),
            ]))
            .unwrap();
        let anchors = |chart: &ChartEngine| {
            chart
                .lane_layout()
                .tokens
                .iter()
                .map(|token| (token.logical, token.count()))
                .collect::<Vec<_>>()
        };
        // Four bars apart at 10 px/bar: two tokens.
        assert_eq!(anchors(&chart), vec![(10, 1), (14, 1)]);
        chart.time_scale.set_bar_spacing(4.0);
        // 16 px apart: they fold, anchored on the earliest slot.
        assert_eq!(anchors(&chart), vec![(10, 2)]);
        chart.time_scale.set_bar_spacing(6.0);
        assert_eq!(anchors(&chart), vec![(10, 1), (14, 1)]);
    }

    #[test]
    fn the_lane_hides_and_shows_with_hysteresis() {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        chart
            .set_timeline_marks(snapshot(vec![mark("a", 1_000, "earnings")]))
            .unwrap();
        chart.panes[0].height = 95.0;
        assert_eq!(chart.timeline_lane_pane(), None, "below 4 × lane height");
        chart.panes[0].height = 100.0;
        assert_eq!(
            chart.timeline_lane_pane(),
            None,
            "inside the dead band it stays hidden"
        );
        chart.panes[0].height = 109.0;
        assert_eq!(chart.timeline_lane_pane(), Some(0));
        chart.panes[0].height = 100.0;
        assert_eq!(
            chart.timeline_lane_pane(),
            Some(0),
            "inside the dead band it stays shown"
        );
        chart.panes[0].height = 95.0;
        assert_eq!(chart.timeline_lane_pane(), None);
        chart.set_timeline_marks_visible(false);
        chart.panes[0].height = 400.0;
        assert_eq!(
            chart.timeline_lane_pane(),
            None,
            "a hidden lane reserves nothing"
        );
    }

    #[test]
    fn the_hit_query_resolves_before_any_frame_and_follows_a_pan() {
        let (mut chart, times) = weekly_chart();
        chart
            .set_timeline_marks(snapshot(vec![mark("a", times[30], "earnings")]))
            .unwrap();
        let x = chart.time_scale.index_to_coordinate(30);
        let y = chart.panes[0].top + chart.panes[0].height - 15.0;
        let hit = chart
            .timeline_mark_hit_at(x, y)
            .expect("token under the pointer");
        assert_eq!(hit.logical, 30);
        assert_eq!(hit.mark_ids, ["a"]);
        assert_eq!(hit.label, "Earnings");
        assert_eq!(hit.tooltip_text(), "Earnings · a title");
        assert_eq!(
            chart.timeline_mark_hit_at(x + 9.0, y),
            None,
            "outside the 16 px token"
        );
        assert!(
            chart.timeline_mark_hit_at(x, y - 8.0).is_some()
                && chart.timeline_mark_hit_at(x, y + 8.0).is_some(),
            "the token's own 16 px box answers vertically"
        );
        assert_eq!(
            chart.timeline_mark_hit_at(x, y - 9.0),
            None,
            "the lane above the token box belongs to the pane"
        );
        assert_eq!(
            chart.timeline_mark_hit_at(x, y + 9.0),
            None,
            "the lane below the token box belongs to the pane"
        );
        assert!(
            chart
                .timeline_mark_hit_at_with_profile(x, y - 20.0, HitProfile::TOUCH)
                .is_some(),
            "touch widens the box by its tolerance"
        );
        // Pan one bar through the controller (the threshold sample anchors the pan, the next
        // ten pixels scroll): the next hit uses the moved layout before any frame is built.
        let at = |x: f64| crate::PointerInput {
            x,
            y: 200.0,
            ..Default::default()
        };
        chart.input_pointer_down(at(400.0), 1);
        chart.input_pointer_move(at(406.0), true);
        chart.input_pointer_move(at(416.0), true);
        chart.input_pointer_up(at(416.0));
        let moved_x = chart.time_scale.index_to_coordinate(30);
        assert!(
            (moved_x - x - 10.0).abs() < 1e-6,
            "the pan moved the bar from {x} to {moved_x}"
        );
        assert!(chart.timeline_mark_hit_at(moved_x, y).is_some());
        assert_eq!(chart.timeline_mark_hit_at(x, y), None);
        // A replay cutoff before the mark stops the hit immediately.
        chart
            .set_replay_clock_micros(Some(times[29] * 1_000_000))
            .unwrap();
        assert_eq!(chart.timeline_mark_hit_at(moved_x, y), None);
    }

    #[test]
    fn the_hit_query_is_cheap_when_the_lane_is_empty_or_off() {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        assert_eq!(chart.timeline_mark_hit_at(100.0, 100.0), None);
        chart
            .set_timeline_marks(snapshot(vec![mark("a", 1_000, "earnings")]))
            .unwrap();
        chart.set_timeline_marks_visible(false);
        assert_eq!(chart.timeline_mark_hit_at(100.0, 100.0), None);
        assert_eq!(chart.timeline_mark_hit_for_id("a"), None);
    }

    #[test]
    fn activations_are_a_bounded_ring() {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        let hit = TimelineMarkHit {
            logical: 1,
            time: 1,
            projected: false,
            count: 1,
            groups: vec!["g".into()],
            mark_ids: vec!["a".into()],
            title: String::new(),
            label: "g".into(),
        };
        let first = chart.push_timeline_activation(hit.clone());
        for _ in 0..MAX_TIMELINE_ACTIVATIONS {
            chart.push_timeline_activation(hit.clone());
        }
        assert_eq!(chart.timeline_mark_activation(first), None);
        assert_eq!(
            chart.timeline_marks.activations.len(),
            MAX_TIMELINE_ACTIVATIONS
        );
        let last = chart.timeline_marks.activations.back().unwrap().0;
        assert_eq!(chart.timeline_mark_activation(last), Some(&hit));
    }
}
