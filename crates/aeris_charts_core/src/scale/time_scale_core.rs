//! Time scale layout state machine. Port of the coordinate/zoom/scroll math from
//! `src/model/time-scale.ts` (tick mark generation and formatting live elsewhere).
//!
//! State: `(width, bar_spacing, right_offset, base_index)` over a point list of length
//! `points_len`. All coordinates are media (CSS) pixels; indices are integer bar positions,
//! logical values are float bar positions (integers at bar centers).

use crate::helpers::mathex::clamp;
use crate::model::range::{LogicalRange, StrictRange};
use crate::{Coordinate, TimePointIndex};

/// `Constants.MinVisibleBarsCount` in reference.
const MIN_VISIBLE_BARS_COUNT: f64 = 2.0;

#[derive(Clone, Debug)]
pub struct TimeScaleOptions {
    pub right_offset: f64,
    pub bar_spacing: f64,
    pub min_bar_spacing: f64,
    /// 0 disables the option (default max = half the width).
    pub max_bar_spacing: f64,
    pub fix_left_edge: bool,
    pub fix_right_edge: bool,
    pub lock_visible_time_range_on_resize: bool,
    pub right_bar_stays_on_scroll: bool,
    /// reference `timeScale.shiftVisibleRangeOnNewBar` (default true): shift the visible range
    /// right with new bars when the last bar is visible (time-scale.ts:165-171).
    pub shift_visible_range_on_new_bar: bool,
    /// reference `timeScale.allowShiftVisibleRangeOnWhitespaceReplacement` (default false): also
    /// shift when the "new bar" replaces an existing whitespace time point
    /// (time-scale.ts:173-181).
    pub allow_shift_visible_range_on_whitespace_replacement: bool,
    /// reference `timeScale.allowBoldLabels` (default true): draw the major (highest-weight)
    /// time tick labels in the bold font (time-axis-widget.ts `_baseBoldFont`).
    pub allow_bold_labels: bool,
    /// When set, overrides `right_offset` and is preserved in pixels across zoom.
    pub right_offset_pixels: Option<f64>,
    /// Aeris fixed-view option (default false; no reference equivalent): hold the visible logical
    /// range exactly across data synchronization and resizes. Data changes neither follow new
    /// bars nor compensate, a resize rescales the bar spacing, and a range set through
    /// [`TimeScaleCore::set_logical_range`] is applied without the reference scroll clamps, so a
    /// host can show a complete session of `N` slots as `[0, N - 1]` even before two bars have
    /// traded. Explicit scroll/zoom mutations still apply (clamped) and re-lock the result.
    pub lock_visible_logical_range: bool,
}

impl Default for TimeScaleOptions {
    fn default() -> Self {
        Self {
            right_offset: 0.0,
            bar_spacing: 6.0,
            min_bar_spacing: 0.5,
            max_bar_spacing: 0.0,
            fix_left_edge: false,
            fix_right_edge: false,
            lock_visible_time_range_on_resize: false,
            // Measured TradingView behavior: ordinary wheel zoom keeps the right edge pinned (the
            // right offset in bars is preserved), so the latest bars stay put while history
            // compresses or expands. Ctrl/Cmd wheel zoom and pinch remain anchored at the pointer;
            // hosts may opt back into cursor-anchored ordinary zoom by setting this to false.
            right_bar_stays_on_scroll: true,
            // reference defaults (time-scale-options-defaults.ts:17-18).
            shift_visible_range_on_new_bar: true,
            allow_shift_visible_range_on_whitespace_replacement: false,
            allow_bold_labels: true,
            right_offset_pixels: None,
            lock_visible_logical_range: false,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct TransitionState {
    bar_spacing: f64,
    right_offset: f64,
}

#[derive(Clone, Debug)]
pub struct TimeScaleCore {
    options: TimeScaleOptions,
    width: f64,
    base_index: Option<TimePointIndex>,
    right_offset: f64,
    bar_spacing: f64,
    points_len: usize,
    scroll_start_point: Option<Coordinate>,
    scale_start_point: Option<Coordinate>,
    common_transition_start_state: Option<TransitionState>,
    /// Host-pushed "all scaling and scrolling disabled" aggregate (reference
    /// `_isAllScalingAndScrollingDisabled`, time-scale.ts:975-986): label alignment only —
    /// never consulted by the spacing/offset math, which reads the raw fix-edge options.
    interaction_disabled: bool,
    /// The held range while `lock_visible_logical_range` is on (`None` until a range exists).
    locked_range: Option<LogicalRange>,
    /// Canonical source revision for every value that can affect time coordinates or time-axis
    /// presentation. Mutators advance this intrinsically so hosts cannot change scale state while
    /// leaving retained geometry on an older transform.
    revision: u64,
}

impl TimeScaleCore {
    pub fn new(options: TimeScaleOptions) -> Self {
        let right_offset = options.right_offset;
        let bar_spacing = options.bar_spacing;
        Self {
            options,
            width: 0.0,
            base_index: None,
            right_offset,
            bar_spacing,
            points_len: 0,
            scroll_start_point: None,
            scale_start_point: None,
            common_transition_start_state: None,
            interaction_disabled: false,
            locked_range: None,
            revision: 1,
        }
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    fn changed(&mut self) {
        self.revision = self.revision.wrapping_add(1).max(1);
    }

    pub fn options(&self) -> &TimeScaleOptions {
        &self.options
    }

    pub fn width(&self) -> f64 {
        self.width
    }

    pub fn bar_spacing(&self) -> f64 {
        self.bar_spacing
    }

    pub fn right_offset(&self) -> f64 {
        self.right_offset
    }

    pub fn points_len(&self) -> usize {
        self.points_len
    }

    /// Base index or 0 when unset (matches the reference's `baseIndex()` getter).
    pub fn base_index(&self) -> TimePointIndex {
        self.base_index.unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.width == 0.0 || self.points_len == 0 || self.base_index.is_none()
    }

    pub fn set_points_len(&mut self, len: usize) {
        let before = (self.points_len, self.bar_spacing, self.right_offset);
        self.points_len = len;
        if self.locked_range.is_some() {
            self.apply_locked_range();
        } else {
            self.correct_offset();
            self.relock();
        }
        if before != (self.points_len, self.bar_spacing, self.right_offset) {
            self.changed();
        }
    }

    pub fn set_base_index(&mut self, base_index: Option<TimePointIndex>) {
        let before = (self.base_index, self.bar_spacing, self.right_offset);
        self.base_index = base_index;
        if self.locked_range.is_some() {
            self.apply_locked_range();
        } else {
            self.correct_offset();
            self.do_fix_left_edge();
            self.relock();
        }
        if before != (self.base_index, self.bar_spacing, self.right_offset) {
            self.changed();
        }
    }

    /// Land one data synchronization atomically: the new point count and base index together
    /// with the right offset that keeps the intended view (`None` keeps the offset relative to
    /// the new base index, i.e. the view follows the latest bar). Offsets are corrected once
    /// against the final state. A locked logical range ignores `right_offset` and is re-applied
    /// exactly, which rebases the offset by the base-index move.
    ///
    /// Returns that view-preserving rebase of the right offset (0 when the offset stays relative
    /// to the base). An active drag's start snapshot moves by the same amount so the gesture
    /// continues on the bars it grabbed; the owner applies it to its own in-flight motion.
    pub fn sync_points(
        &mut self,
        points_len: usize,
        base_index: Option<TimePointIndex>,
        right_offset: Option<f64>,
    ) -> f64 {
        let before = (
            self.points_len,
            self.base_index,
            self.bar_spacing,
            self.right_offset,
        );
        self.points_len = points_len;
        self.base_index = base_index;
        let rebase = if self.locked_range.is_some() {
            let previous = self.right_offset;
            self.apply_locked_range();
            self.right_offset - previous
        } else {
            let rebase = right_offset
                .filter(|offset| offset.is_finite())
                .map_or(0.0, |offset| {
                    let shift = offset - self.right_offset;
                    self.right_offset = offset;
                    shift
                });
            self.correct_offset();
            self.do_fix_left_edge();
            self.relock();
            rebase
        };
        if let Some(state) = self.common_transition_start_state.as_mut() {
            state.right_offset += rebase;
        }
        if before
            != (
                self.points_len,
                self.base_index,
                self.bar_spacing,
                self.right_offset,
            )
        {
            self.changed();
        }
        rebase
    }

    fn first_index(&self) -> Option<TimePointIndex> {
        if self.points_len == 0 { None } else { Some(0) }
    }

    fn last_index(&self) -> Option<TimePointIndex> {
        if self.points_len == 0 {
            None
        } else {
            Some(self.points_len as i64 - 1)
        }
    }

    // --- coordinate conversion ---

    pub fn index_to_coordinate(&self, index: TimePointIndex) -> Coordinate {
        self.logical_to_coordinate(index as f64)
    }

    /// Convert a possibly fractional logical index to a media-coordinate X position.
    pub fn logical_to_coordinate(&self, logical: f64) -> Coordinate {
        if self.is_empty() {
            return 0.0;
        }
        let base_index = self.base_index();
        let delta_from_right = base_index as f64 + self.right_offset - logical;
        self.width - (delta_from_right + 0.5) * self.bar_spacing - 1.0
    }

    /// Batch conversion: writes x for each index. The hot path for series rendering.
    pub fn indexes_to_coordinates(&self, indices: &[TimePointIndex], out_x: &mut [Coordinate]) {
        debug_assert_eq!(indices.len(), out_x.len());
        let base = self.base_index() as f64 + self.right_offset;
        for (i, &index) in indices.iter().enumerate() {
            let delta_from_right = base - index as f64;
            out_x[i] = self.width - (delta_from_right + 0.5) * self.bar_spacing - 1.0;
        }
    }

    fn right_offset_for_coordinate(&self, x: Coordinate) -> f64 {
        (self.width - 1.0 - x) / self.bar_spacing
    }

    pub fn coordinate_to_float_index(&self, x: Coordinate) -> f64 {
        let delta_from_right = self.right_offset_for_coordinate(x);
        let base_index = self.base_index();
        let index = base_index as f64 + self.right_offset - delta_from_right;
        // JS-compatible fp-noise cleanup
        (index * 1_000_000.0).round() / 1_000_000.0
    }

    pub fn coordinate_to_index(&self, x: Coordinate) -> TimePointIndex {
        self.coordinate_to_float_index(x).ceil() as TimePointIndex
    }

    // --- visible range ---

    pub fn visible_logical_range(&self) -> Option<LogicalRange> {
        if self.is_empty() {
            return None;
        }
        // A locked range reports the exact host range while the spacing honors it; derived
        // borders would carry floating-point noise from `width / (width / count)`.
        if let Some(locked) = self.locked_range
            && self.bar_spacing == self.width / (locked.right() - locked.left() + 1.0)
        {
            return Some(locked);
        }
        self.derived_visible_logical_range()
    }

    fn derived_visible_logical_range(&self) -> Option<LogicalRange> {
        if self.is_empty() {
            return None;
        }

        let base_index = self.base_index();
        let new_bars_length = self.width / self.bar_spacing;
        let right_border = self.right_offset + base_index as f64;
        let left_border = right_border - new_bars_length + 1.0;

        Some(LogicalRange::new(left_border, right_border))
    }

    pub fn visible_strict_range(&self) -> Option<StrictRange> {
        self.visible_logical_range().map(|r| r.to_strict())
    }

    // --- sizing ---

    pub fn set_width(&mut self, new_width: f64) {
        if !new_width.is_finite() || new_width <= 0.0 || self.width == new_width {
            return;
        }

        // capture the previous visible range before mutating (needed for fix_left_edge)
        let previous_visible_range = self.visible_logical_range();

        let old_width = self.width;
        self.width = new_width;

        if self.locked_range.is_some() {
            self.apply_locked_range();
            self.changed();
            return;
        }

        if self.options.lock_visible_time_range_on_resize && old_width != 0.0 {
            self.bar_spacing = self.bar_spacing * new_width / old_width;
        }

        if self.options.fix_left_edge
            && let Some(prev) = previous_visible_range
            && prev.left() <= 0.0
        {
            let delta = old_width - new_width;
            // reducing right_offset means moving right
            self.right_offset -= (delta / self.bar_spacing).round() + 1.0;
        }

        // bar spacing first: right offset correction depends on it
        self.correct_bar_spacing();
        self.correct_offset();
        self.relock();
        self.changed();
    }

    // --- bar spacing / offset mutation ---

    pub fn set_bar_spacing(&mut self, new_bar_spacing: f64) {
        let before = (self.bar_spacing, self.right_offset);
        let old_bar_spacing = self.bar_spacing;
        self.set_bar_spacing_internal(new_bar_spacing);
        if self.options.right_offset_pixels.is_some() && old_bar_spacing != 0.0 {
            // pixel mode: keep the pixel offset by rescaling the bar offset
            self.right_offset = self.right_offset * old_bar_spacing / self.bar_spacing;
        }
        self.correct_offset();
        self.relock();
        if before != (self.bar_spacing, self.right_offset) {
            self.changed();
        }
    }

    fn set_bar_spacing_internal(&mut self, new_bar_spacing: f64) {
        self.bar_spacing = new_bar_spacing;
        self.correct_bar_spacing();
    }

    pub fn set_right_offset(&mut self, offset: f64) {
        let before = self.right_offset;
        self.right_offset = offset;
        self.correct_offset();
        self.relock();
        if before != self.right_offset {
            self.changed();
        }
    }

    // --- option mutation (reference `applyOptions({ timeScale })`) ---

    /// Minimum bar spacing in CSS px (reference `minBarSpacing`, default 0.5). Ignored if non-positive.
    /// Re-clamps the current spacing/offset against the new floor.
    pub fn set_min_bar_spacing(&mut self, min_bar_spacing: f64) {
        if min_bar_spacing.is_finite() && min_bar_spacing > 0.0 {
            let before = (
                self.options.min_bar_spacing,
                self.bar_spacing,
                self.right_offset,
            );
            self.options.min_bar_spacing = min_bar_spacing;
            self.correct_bar_spacing();
            self.correct_offset();
            self.relock();
            if before
                != (
                    self.options.min_bar_spacing,
                    self.bar_spacing,
                    self.right_offset,
                )
            {
                self.changed();
            }
        }
    }

    /// reference `applyOptions({ barSpacing })`: write the option *and* apply it live. Zoom/scroll
    /// gestures use `set_bar_spacing`, which deliberately leaves the configured option alone.
    pub fn apply_bar_spacing_option(&mut self, bar_spacing: f64) {
        if bar_spacing.is_finite() && bar_spacing > 0.0 {
            let changed = self.options.bar_spacing != bar_spacing;
            self.options.bar_spacing = bar_spacing;
            self.set_bar_spacing(bar_spacing);
            if changed {
                self.changed();
            }
        }
    }

    /// reference `applyOptions({ rightOffset })`: write the option *and* apply it live.
    pub fn apply_right_offset_option(&mut self, offset: f64) {
        if offset.is_finite() {
            let changed = self.options.right_offset != offset;
            self.options.right_offset = offset;
            self.set_right_offset(offset);
            if changed {
                self.changed();
            }
        }
    }

    /// Maximum bar spacing in CSS px (reference `maxBarSpacing`, default 0 = the half-width cap,
    /// time-scale.ts:1052-1059). Ignored if negative or non-finite. Re-clamps the current
    /// spacing/offset against the new cap.
    pub fn set_max_bar_spacing(&mut self, max_bar_spacing: f64) {
        if max_bar_spacing.is_finite() && max_bar_spacing >= 0.0 {
            let before = (
                self.options.max_bar_spacing,
                self.bar_spacing,
                self.right_offset,
            );
            self.options.max_bar_spacing = max_bar_spacing;
            self.correct_bar_spacing();
            self.correct_offset();
            self.relock();
            if before
                != (
                    self.options.max_bar_spacing,
                    self.bar_spacing,
                    self.right_offset,
                )
            {
                self.changed();
            }
        }
    }

    /// reference `rightOffsetPixels`: pin the right offset in pixels. The pixel value converts to a
    /// bar offset through the current bar spacing and is then preserved across zoom, exactly
    /// like `_checkRightOffsetPixels` (time-scale.ts:1247-1252). Ignored if non-finite.
    pub fn set_right_offset_pixels(&mut self, pixels: f64) {
        if !pixels.is_finite() {
            return;
        }
        let changed = self.options.right_offset_pixels != Some(pixels);
        self.options.right_offset_pixels = Some(pixels);
        if self.bar_spacing > 0.0 {
            self.set_right_offset(pixels / self.bar_spacing);
        }
        if changed {
            self.changed();
        }
    }

    /// reference `fixLeftEdge`: prevent scrolling past the first data point on the left.
    pub fn set_fix_left_edge(&mut self, fix: bool) {
        let before = (
            self.options.fix_left_edge,
            self.bar_spacing,
            self.right_offset,
        );
        self.options.fix_left_edge = fix;
        self.do_fix_left_edge();
        self.correct_bar_spacing();
        self.correct_offset();
        self.relock();
        if before
            != (
                self.options.fix_left_edge,
                self.bar_spacing,
                self.right_offset,
            )
        {
            self.changed();
        }
    }

    /// reference `fixRightEdge`: prevent scrolling past the last data point on the right.
    pub fn set_fix_right_edge(&mut self, fix: bool) {
        let before = (self.options.fix_right_edge, self.right_offset);
        self.options.fix_right_edge = fix;
        self.correct_offset();
        self.relock();
        if before != (self.options.fix_right_edge, self.right_offset) {
            self.changed();
        }
    }

    /// reference `lockVisibleTimeRangeOnResize`: keep the visible range constant across width changes by
    /// rescaling bar spacing (applied on the next `set_width`).
    pub fn set_lock_visible_time_range_on_resize(&mut self, lock: bool) {
        if self.options.lock_visible_time_range_on_resize != lock {
            self.options.lock_visible_time_range_on_resize = lock;
            self.changed();
        }
    }

    /// Aeris `lock_visible_logical_range`: enabling captures the current visible logical range
    /// (or the next one once the scale has a range); disabling returns to the reference behavior
    /// from the current view.
    pub fn set_lock_visible_logical_range(&mut self, lock: bool) {
        if self.options.lock_visible_logical_range != lock {
            self.options.lock_visible_logical_range = lock;
            self.locked_range = None;
            self.relock();
            self.changed();
        }
    }

    /// Capture the current view as the locked range after an explicit mutation.
    fn relock(&mut self) {
        if self.options.lock_visible_logical_range {
            self.locked_range = self.derived_visible_logical_range();
        }
    }

    /// Re-derive spacing and offset from the locked range for the current width and base index.
    /// Offset clamps are deliberately skipped: the locked range is the host's explicit view.
    fn apply_locked_range(&mut self) {
        let Some(range) = self.locked_range else {
            return;
        };
        if self.width > 0.0 {
            self.set_bar_spacing_internal(self.width / (range.right() - range.left() + 1.0));
        }
        self.right_offset = range.right() - self.base_index() as f64;
    }

    /// reference `rightBarStaysOnScroll`.
    pub fn set_right_bar_stays_on_scroll(&mut self, stays: bool) {
        if self.options.right_bar_stays_on_scroll != stays {
            self.options.right_bar_stays_on_scroll = stays;
            self.changed();
        }
    }

    /// reference `shiftVisibleRangeOnNewBar` (time-scale.ts:165-171). Read by the data-sync
    /// compensation in the engine (chart-model.ts:968-983).
    pub fn set_shift_visible_range_on_new_bar(&mut self, shift: bool) {
        if self.options.shift_visible_range_on_new_bar != shift {
            self.options.shift_visible_range_on_new_bar = shift;
            self.changed();
        }
    }

    /// reference `allowShiftVisibleRangeOnWhitespaceReplacement` (time-scale.ts:173-181).
    pub fn set_allow_shift_visible_range_on_whitespace_replacement(&mut self, allow: bool) {
        if self
            .options
            .allow_shift_visible_range_on_whitespace_replacement
            != allow
        {
            self.options
                .allow_shift_visible_range_on_whitespace_replacement = allow;
            self.changed();
        }
    }

    /// reference `timeScale.allowBoldLabels` (default true): bold the major time tick labels.
    pub fn set_allow_bold_labels(&mut self, allow: bool) {
        if self.options.allow_bold_labels != allow {
            self.options.allow_bold_labels = allow;
            self.changed();
        }
    }

    /// Host-pushed "all scaling and scrolling disabled" flag (reference
    /// `_isAllScalingAndScrollingDisabled`, time-scale.ts:975-986). In the reference this
    /// aggregate only feeds tick-mark label alignment (time-scale.ts:657-690); the scale math
    /// (spacing floor, offset clamps, edge snapping) always reads the raw fixLeftEdge /
    /// fixRightEdge options. The flag is stored for the label path and must not alter spacing
    /// or offsets — a non-interactive chart must not react to resizes.
    pub fn set_interaction_disabled(&mut self, disabled: bool) {
        if self.interaction_disabled != disabled {
            self.interaction_disabled = disabled;
            self.changed();
        }
    }

    /// Whether the host has disabled every scroll/scale gesture (the reference's
    /// label-alignment input, time-scale.ts:658-659). Scale math reads the raw options instead.
    pub fn interaction_disabled(&self) -> bool {
        self.interaction_disabled
    }

    fn max_bar_spacing(&self) -> f64 {
        if self.options.max_bar_spacing > 0.0 {
            self.options.max_bar_spacing
        } else {
            self.width * 0.5
        }
    }

    fn min_bar_spacing(&self) -> f64 {
        if self.options.fix_left_edge && self.options.fix_right_edge && self.points_len != 0 {
            self.width / self.points_len as f64
        } else {
            self.options.min_bar_spacing
        }
    }

    fn correct_bar_spacing(&mut self) {
        let bar_spacing = clamp(
            self.bar_spacing,
            self.min_bar_spacing(),
            self.max_bar_spacing(),
        );
        if self.bar_spacing != bar_spacing {
            self.bar_spacing = bar_spacing;
        }
    }

    fn min_right_offset(&self) -> Option<f64> {
        let first_index = self.first_index()?;
        let base_index = self.base_index?;

        let bars_estimation = if self.options.fix_left_edge {
            self.width / self.bar_spacing
        } else {
            MIN_VISIBLE_BARS_COUNT.min(self.points_len as f64)
        };

        Some(first_index as f64 - base_index as f64 - 1.0 + bars_estimation)
    }

    fn max_right_offset(&self) -> f64 {
        if self.options.fix_right_edge {
            0.0
        } else {
            (self.width / self.bar_spacing) - MIN_VISIBLE_BARS_COUNT.min(self.points_len as f64)
        }
    }

    fn correct_offset(&mut self) {
        // block scrolling into the past
        if let Some(min_right_offset) = self.min_right_offset()
            && self.right_offset < min_right_offset
        {
            self.right_offset = min_right_offset;
        }

        // block scrolling into the future
        let max_right_offset = self.max_right_offset();
        if self.right_offset > max_right_offset {
            self.right_offset = max_right_offset;
        }
    }

    fn do_fix_left_edge(&mut self) {
        if !self.options.fix_left_edge {
            return;
        }
        let Some(first_index) = self.first_index() else {
            return;
        };
        let Some(visible_range) = self.visible_strict_range() else {
            return;
        };

        let delta = visible_range.left() - first_index;
        if delta < 0 {
            let left_edge_offset = self.right_offset - delta as f64 - 1.0;
            self.set_right_offset(left_edge_offset);
        }
        self.correct_bar_spacing();
    }

    // --- zoom ---

    /// `scale` is in 1/10 parts of the current bar spacing; negative zooms out. Ordinary wheel
    /// zoom follows the configured right-bar pin policy.
    pub fn zoom(&mut self, zoom_point: Coordinate, scale: f64) {
        self.zoom_impl(zoom_point, scale, !self.options.right_bar_stays_on_scroll);
    }

    /// Focused zoom always keeps the logical point under `zoom_point` fixed, whatever
    /// `right_bar_stays_on_scroll` says: Ctrl/Cmd wheel zoom and pinch use it on every host.
    pub fn zoom_focused(&mut self, zoom_point: Coordinate, scale: f64) {
        self.zoom_impl(zoom_point, scale, true);
    }

    fn zoom_impl(&mut self, zoom_point: Coordinate, scale: f64, keep_zoom_point: bool) {
        let float_index_at_zoom_point = self.coordinate_to_float_index(zoom_point);

        let bar_spacing = self.bar_spacing;
        let new_bar_spacing = bar_spacing + scale * (bar_spacing / 10.0);

        self.set_bar_spacing(new_bar_spacing);

        if keep_zoom_point {
            // move the index under zoom_point back to its coordinate
            let new_offset = self.right_offset
                + (float_index_at_zoom_point - self.coordinate_to_float_index(zoom_point));
            self.set_right_offset(new_offset);
        }
    }

    // --- axis-drag scale ---

    pub fn start_scale(&mut self, x: Coordinate) {
        if self.scroll_start_point.is_some() {
            self.end_scroll();
        }
        if self.scale_start_point.is_some() || self.common_transition_start_state.is_some() {
            return;
        }
        if self.is_empty() {
            return;
        }
        self.scale_start_point = Some(x);
        self.save_common_transitions_start_state();
    }

    pub fn scale_to(&mut self, x: Coordinate) {
        // Both are set together in `start_scale`; bail if the pair desynced.
        let (Some(start_state), Some(scale_start_point)) =
            (self.common_transition_start_state, self.scale_start_point)
        else {
            return;
        };

        let start_length_from_right = clamp(self.width - x, 0.0, self.width);
        let current_length_from_right = clamp(self.width - scale_start_point, 0.0, self.width);
        if start_length_from_right == 0.0 || current_length_from_right == 0.0 {
            return;
        }

        self.set_bar_spacing(
            start_state.bar_spacing * start_length_from_right / current_length_from_right,
        );
    }

    pub fn end_scale(&mut self) {
        if self.scale_start_point.is_none() {
            return;
        }
        self.scale_start_point = None;
        self.clear_common_transitions_start_state();
    }

    // --- drag scroll ---

    pub fn start_scroll(&mut self, x: Coordinate) {
        if self.scroll_start_point.is_some() || self.common_transition_start_state.is_some() {
            return;
        }
        if self.is_empty() {
            return;
        }
        self.scroll_start_point = Some(x);
        self.save_common_transitions_start_state();
    }

    pub fn scroll_to(&mut self, x: Coordinate) {
        // Both are set together in `start_scroll`; bail if the pair desynced.
        let (Some(scroll_start_point), Some(start_state)) =
            (self.scroll_start_point, self.common_transition_start_state)
        else {
            return;
        };

        let shift_in_logical = (scroll_start_point - x) / self.bar_spacing;
        let before = self.right_offset;
        self.right_offset = start_state.right_offset + shift_in_logical;

        self.correct_offset();
        self.relock();
        if before != self.right_offset {
            self.changed();
        }
    }

    pub fn end_scroll(&mut self) {
        if self.scroll_start_point.is_none() {
            return;
        }
        self.scroll_start_point = None;
        self.clear_common_transitions_start_state();
    }

    // --- range setting ---

    /// Port of `setVisibleRange` (without the invalidation side effects).
    pub fn set_visible_range(&mut self, range: StrictRange, apply_default_offset: bool) {
        self.set_visible_bounds(
            range.left() as f64,
            range.right() as f64,
            apply_default_offset,
        );
    }

    /// `setVisibleRange` over possibly fractional borders: the reference `RangeImpl` count is
    /// `right - left + 1` for fractional logical bounds as well (range-impl.ts:22-24).
    fn set_visible_bounds(&mut self, left: f64, right: f64, apply_default_offset: bool) {
        let before = (self.bar_spacing, self.right_offset);
        let length = right - left + 1.0;
        let pixel_offset = if apply_default_offset {
            self.options.right_offset_pixels.unwrap_or(0.0)
        } else {
            0.0
        };
        self.set_bar_spacing_internal((self.width - pixel_offset) / length);
        self.right_offset = right - self.base_index() as f64;
        if apply_default_offset {
            self.right_offset = if pixel_offset != 0.0 {
                pixel_offset / self.bar_spacing
            } else {
                self.options.right_offset
            };
        }
        self.correct_offset();
        self.relock();
        if before != (self.bar_spacing, self.right_offset) {
            self.changed();
        }
    }

    pub fn fit_content(&mut self) {
        let (Some(first), Some(last)) = (self.first_index(), self.last_index()) else {
            return;
        };

        // include the user-defined right offset in the range so scaling reserves space for it
        let right_offset_bars = if self.options.right_offset_pixels.is_none() {
            self.options.right_offset
        } else {
            0.0
        };
        self.set_visible_range(
            StrictRange::new(first, last + right_offset_bars as i64),
            true,
        );
    }

    /// reference `setLogicalRange` (time-scale.ts:907-913): the fractional borders pass straight
    /// through, so reading the visible logical range and setting it back is the identity. With
    /// `lock_visible_logical_range` the range becomes the held view and skips the scroll clamps.
    pub fn set_logical_range(&mut self, range: LogicalRange) {
        if self.options.lock_visible_logical_range {
            let before = (self.locked_range, self.bar_spacing, self.right_offset);
            self.locked_range = Some(range);
            self.apply_locked_range();
            if before != (self.locked_range, self.bar_spacing, self.right_offset) {
                self.changed();
            }
            return;
        }
        self.set_visible_bounds(range.left(), range.right(), false);
    }

    pub fn restore_default(&mut self) {
        self.set_bar_spacing(self.options.bar_spacing);
        let new_offset = match self.options.right_offset_pixels {
            Some(px) => px / self.bar_spacing,
            None => self.options.right_offset,
        };
        self.set_right_offset(new_offset);
    }

    fn save_common_transitions_start_state(&mut self) {
        self.common_transition_start_state = Some(TransitionState {
            bar_spacing: self.bar_spacing,
            right_offset: self.right_offset,
        });
    }

    fn clear_common_transitions_start_state(&mut self) {
        self.common_transition_start_state = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scale(
        width: f64,
        bar_spacing: f64,
        right_offset: f64,
        points: usize,
        base: i64,
    ) -> TimeScaleCore {
        let mut s = TimeScaleCore::new(TimeScaleOptions {
            bar_spacing,
            right_offset,
            ..Default::default()
        });
        s.set_width(width);
        s.points_len = points;
        s.base_index = Some(base);
        s
    }

    #[test]
    fn index_to_coordinate_formula() {
        // x = width - (base + rightOffset - index + 0.5) * barSpacing - 1
        let s = scale(100.0, 6.0, 0.0, 20, 10);
        assert_eq!(s.index_to_coordinate(10), 100.0 - 0.5 * 6.0 - 1.0); // 96
        assert_eq!(s.index_to_coordinate(9), 100.0 - 1.5 * 6.0 - 1.0); // 90
    }

    #[test]
    fn coordinate_roundtrip() {
        // coordinate_to_float_index(index_to_coordinate(i)) == i - 0.5 by construction
        // (float index measures the logical cell boundary; ceil snaps back to the bar index)
        let s = scale(100.0, 6.0, 2.5, 50, 30);
        for index in [0i64, 5, 29, 30] {
            let x = s.index_to_coordinate(index);
            let f = s.coordinate_to_float_index(x);
            assert!(
                (f - (index as f64 - 0.5)).abs() < 1e-6,
                "index {index} -> x {x} -> {f}"
            );
            assert_eq!(s.coordinate_to_index(x), index);
        }
    }

    #[test]
    fn coordinate_to_index_cell_boundaries() {
        let s = scale(100.0, 6.0, 0.0, 20, 10);
        let x_of_9 = s.index_to_coordinate(9);
        // bar 9 owns roughly (center - spacing/2, center + spacing/2]
        assert_eq!(s.coordinate_to_index(x_of_9), 9);
        assert_eq!(s.coordinate_to_index(x_of_9 + 3.0), 9); // exactly on the boundary
        assert_eq!(s.coordinate_to_index(x_of_9 + 3.1), 10); // past it
    }

    #[test]
    fn visible_logical_range_formula() {
        let s = scale(120.0, 6.0, 0.0, 100, 50);
        let r = s.visible_logical_range().unwrap();
        // rightBorder = rightOffset + baseIndex = 50; barsLength = 120/6 = 20
        assert_eq!(r.right(), 50.0);
        assert_eq!(r.left(), 50.0 - 20.0 + 1.0);
        let strict = r.to_strict();
        assert_eq!(strict.left(), 31);
        assert_eq!(strict.right(), 50);
    }

    #[test]
    fn ordinary_zoom_pins_the_right_edge_by_default_like_tradingview() {
        // Measured on TradingView: one wheel notch scales bar spacing by exactly 1.1 / 0.9 while
        // the right offset in bars (the gap after the latest bar) stays constant.
        let mut s = scale(400.0, 6.0, 7.0, 500, 300);
        assert!(s.options().right_bar_stays_on_scroll);
        let cursor = 250.0;
        let before_index = s.coordinate_to_float_index(cursor);
        let before_offset = s.right_offset();
        s.zoom(cursor, 1.0); // zoom in 10%
        assert!((s.bar_spacing() - 6.6).abs() < 1e-12);
        assert_eq!(s.right_offset(), before_offset);
        assert_ne!(s.coordinate_to_float_index(cursor), before_index);
        s.zoom(cursor, -1.0); // zoom out 10%
        assert!((s.bar_spacing() - 5.94).abs() < 1e-12);
        assert_eq!(s.right_offset(), before_offset);
    }

    #[test]
    fn ordinary_zoom_is_cursor_anchored_when_the_right_pin_is_disabled() {
        let mut s = scale(400.0, 6.0, 0.0, 500, 300);
        s.set_right_bar_stays_on_scroll(false);
        let cursor = 250.0;
        let before = s.coordinate_to_float_index(cursor);
        s.zoom(cursor, 1.0); // zoom in 10%
        assert!((s.bar_spacing() - 6.6).abs() < 1e-12);
        let after = s.coordinate_to_float_index(cursor);
        assert!(
            (before - after).abs() < 1e-6,
            "point drifted: {before} -> {after}"
        );
    }

    #[test]
    fn focused_zoom_keeps_point_under_cursor_even_with_right_bar_pin_enabled() {
        let mut s = scale(400.0, 6.0, 0.0, 500, 300);
        // The pin is what focused zoom must override; the test previously left it disabled.
        s.set_right_bar_stays_on_scroll(true);
        let cursor = 250.0;
        let before = s.coordinate_to_float_index(cursor);
        s.zoom_focused(cursor, 1.0);
        let after = s.coordinate_to_float_index(cursor);
        assert!(
            (before - after).abs() < 1e-6,
            "point drifted: {before} -> {after}"
        );
        assert_ne!(s.right_offset(), 0.0);
    }

    #[test]
    fn zoom_out_clamps_to_min_bar_spacing() {
        let mut s = scale(400.0, 0.55, 0.0, 500, 300);
        s.zoom(200.0, -5.0); // massive zoom out
        assert!(s.bar_spacing() >= 0.5);
    }

    #[test]
    fn scroll_moves_by_bars() {
        let mut s = scale(400.0, 8.0, 10.0, 500, 300);
        s.start_scroll(200.0);
        s.scroll_to(160.0); // dragged 40px left -> content moves left -> offset += 40/8
        assert!((s.right_offset() - 15.0).abs() < 1e-12);
        s.end_scroll();
    }

    #[test]
    fn scroll_clamped_to_future_limit() {
        let mut s = scale(400.0, 8.0, 0.0, 500, 300);
        s.start_scroll(400.0);
        s.scroll_to(0.0); // drag 400px left -> +50 bars, but max = width/spacing - 2 = 48
        assert!((s.right_offset() - 48.0).abs() < 1e-12);
    }

    #[test]
    fn scroll_clamped_to_past_limit() {
        let mut s = scale(400.0, 8.0, 0.0, 100, 99);
        s.start_scroll(0.0);
        s.scroll_to(4000.0); // drag far right -> past; min = 0 - 99 - 1 + 2 = -98
        assert!((s.right_offset() - -98.0).abs() < 1e-12);
    }

    #[test]
    fn axis_drag_scale_formula() {
        // new = start_spacing * (width - x) / (width - start_x)
        let mut s = scale(400.0, 6.0, 0.0, 500, 300);
        s.start_scale(300.0); // 100px from right
        s.scale_to(350.0); // 50px from right -> 6 * 50/100 = 3 (zoom out)
        assert!((s.bar_spacing() - 3.0).abs() < 1e-12);
        s.end_scale();

        // dragging away from the right edge zooms in
        let mut s2 = scale(400.0, 6.0, 0.0, 500, 300);
        s2.start_scale(300.0);
        s2.scale_to(200.0); // 200px from right -> 6 * 200/100 = 12
        assert!((s2.bar_spacing() - 12.0).abs() < 1e-12);
    }

    #[test]
    fn fit_content_shows_all_bars() {
        let mut s = scale(500.0, 6.0, 0.0, 100, 99);
        s.fit_content();
        let r = s.visible_strict_range().unwrap();
        assert!(r.left() <= 0);
        assert!(r.right() >= 99);
    }

    #[test]
    fn max_bar_spacing_defaults_to_half_width() {
        let mut s = scale(400.0, 6.0, 0.0, 500, 300);
        s.set_bar_spacing(10_000.0);
        assert_eq!(s.bar_spacing(), 200.0);
    }

    #[test]
    fn max_bar_spacing_option_caps_zoom_and_zero_restores_default() {
        let mut s = scale(400.0, 6.0, 0.0, 500, 300);
        // reference `maxBarSpacing` overrides the half-width default cap.
        s.set_max_bar_spacing(10.0);
        s.set_bar_spacing(50.0);
        assert_eq!(s.bar_spacing(), 10.0);
        // 0 disables the option (reference default), restoring the half-width cap.
        s.set_max_bar_spacing(0.0);
        s.set_bar_spacing(10_000.0);
        assert_eq!(s.bar_spacing(), 200.0);
        // Negative or non-finite input is ignored, keeping the current option.
        s.set_max_bar_spacing(-1.0);
        s.set_max_bar_spacing(f64::NAN);
        assert_eq!(s.options.max_bar_spacing, 0.0);
    }

    #[test]
    fn right_offset_pixels_converts_through_current_bar_spacing() {
        // reference time-scale.ts:1247-1252: the pixel offset becomes a bar offset via the current
        // bar spacing, and a later spacing change rescales it to hold the pixels constant.
        let mut s = scale(400.0, 6.0, 0.0, 500, 300);
        s.set_right_offset_pixels(60.0);
        assert_eq!(s.right_offset(), 10.0);
        s.set_bar_spacing(12.0);
        assert_eq!(s.right_offset(), 5.0);
        assert_eq!(s.options.right_offset_pixels, Some(60.0));
        // Non-finite input is ignored, keeping the current option.
        s.set_right_offset_pixels(f64::NAN);
        s.set_right_offset_pixels(f64::INFINITY);
        assert_eq!(s.options.right_offset_pixels, Some(60.0));
        assert_eq!(s.right_offset(), 5.0);
    }

    #[test]
    fn interaction_disabled_leaves_scale_math_untouched() {
        // reference time-scale.ts:975-986: the all-interactions-disabled aggregate only feeds
        // tick-label alignment (time-scale.ts:657-690); the spacing/offset math keeps reading
        // the raw fixLeftEdge/fixRightEdge options, so a non-interactive chart never reacts to
        // resizes or offset clamps as if its edges were fixed.
        let mut s = scale(400.0, 8.0, 0.0, 500, 300);
        s.set_interaction_disabled(true);
        // no forced fixRightEdge: future whitespace is still allowed
        s.set_right_offset(5.0);
        assert_eq!(s.right_offset(), 5.0);
        // no forced fixLeftEdge: the past clamp stays the plain two-bar one
        s.set_right_offset(-280.0);
        assert_eq!(s.right_offset(), -280.0);
        // no width/points spacing floor: fit-content spacing survives a width change
        let mut s = scale(500.0, 500.0 / 300.0, 0.0, 300, 299);
        s.set_interaction_disabled(true);
        s.set_width(700.0);
        assert_eq!(s.bar_spacing(), 500.0 / 300.0);
        // the flag is readable for the label path and toggles freely
        assert!(s.interaction_disabled());
        s.set_interaction_disabled(false);
        assert!(!s.interaction_disabled());
    }

    #[test]
    fn logical_range_keeps_fractional_borders_like_the_reference() {
        // reference setLogicalRange (time-scale.ts:907-913) does not round; RangeImpl.count is
        // right - left + 1 (range-impl.ts:22-24).
        let mut s = scale(400.0, 6.0, 0.0, 500, 300);
        s.set_logical_range(LogicalRange::new(100.25, 179.75));
        assert_eq!(s.bar_spacing(), 400.0 / 80.5);
        let r = s.visible_logical_range().unwrap();
        assert!((r.left() - 100.25).abs() < 1e-9 && (r.right() - 179.75).abs() < 1e-9);
    }

    #[test]
    fn sync_points_rebases_right_offset_and_an_active_drag_snapshot() {
        let mut s = scale(400.0, 8.0, -20.0, 500, 300);
        s.start_scroll(200.0);
        s.scroll_to(160.0);
        let grabbed = s.coordinate_to_float_index(160.0);
        // Two bars appended while scrolled back: the view keeps its absolute right border.
        let rebase = s.sync_points(502, Some(302), Some(s.right_offset() - 2.0));
        assert_eq!(
            rebase, -2.0,
            "the owner rebases its own motion by the same amount"
        );
        assert!((s.coordinate_to_float_index(160.0) - grabbed).abs() < 1e-9);
        s.scroll_to(160.0);
        assert!(
            (s.coordinate_to_float_index(160.0) - grabbed).abs() < 1e-9,
            "the drag continues from the rebased snapshot"
        );
        // `None` keeps the offset relative to the base (follow-latest).
        let offset = s.right_offset();
        s.end_scroll();
        assert_eq!(s.sync_points(503, Some(303), None), 0.0);
        assert_eq!(s.right_offset(), offset);

        // A locked range rebases by the base move and carries the drag snapshot with it.
        s.set_lock_visible_logical_range(true);
        s.start_scroll(200.0);
        s.scroll_to(190.0);
        let grabbed = s.coordinate_to_float_index(190.0);
        assert_eq!(s.sync_points(503, Some(305), None), -2.0);
        s.scroll_to(190.0);
        assert!(
            (s.coordinate_to_float_index(190.0) - grabbed).abs() < 1e-9,
            "a locked drag continues from the rebased snapshot"
        );
    }

    #[test]
    fn locked_logical_range_holds_across_resize_and_base_moves_without_clamps() {
        let mut s = scale(800.0, 6.0, 0.0, 241, 0);
        s.set_lock_visible_logical_range(true);
        s.set_logical_range(LogicalRange::new(0.0, 240.0));
        // The reference clamp would force index -1 into view while the base is slot 0.
        assert_eq!(
            s.visible_logical_range(),
            Some(LogicalRange::new(0.0, 240.0))
        );
        s.sync_points(241, Some(5), None);
        s.set_width(333.0);
        assert_eq!(
            s.visible_logical_range(),
            Some(LogicalRange::new(0.0, 240.0))
        );
        assert_eq!(s.bar_spacing(), 333.0 / 241.0);
        // An explicit scroll applies the ordinary clamps and becomes the held range.
        s.set_right_offset(s.right_offset() - 10.0);
        let moved = s.visible_logical_range().unwrap();
        assert!((moved.right() - 230.0).abs() < 1e-9);
        s.sync_points(241, Some(6), None);
        assert!((s.visible_logical_range().unwrap().right() - 230.0).abs() < 1e-9);
        // Unlocking returns to the reference clamps from the current view.
        s.set_lock_visible_logical_range(false);
        s.sync_points(241, Some(0), None);
        s.set_logical_range(LogicalRange::new(0.0, 240.0));
        let clamped = s.visible_logical_range().unwrap();
        assert!(
            (clamped.left() + 1.0).abs() < 1e-9,
            "reference clamp: {clamped:?}"
        );
    }

    #[test]
    fn interaction_disabled_defaults_off_and_is_idempotent() {
        let mut s = scale(400.0, 8.0, 0.0, 500, 300);
        assert!(!s.interaction_disabled());
        s.set_right_offset(5.0);
        // flag off: the future clamp is width/spacing - 2 bars, not 0
        assert_eq!(s.right_offset(), 5.0);
        s.set_interaction_disabled(false);
        assert_eq!(s.right_offset(), 5.0);
    }
}
