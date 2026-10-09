//! Interaction models the engine owns outright: hosts forward
//! normalized pointer/wheel samples and schedule frames; every formula lives here so the
//! native headless harness exercises the exact code the browser runs.
//!
//! - kinetic (momentum) scroll — reference `model/kinetic-animation.ts`, sampled in pointer px
//! - axis drag-to-scale and vertical price pan routing onto the pane scales — reference
//!   `PriceAxisWidget`/`TimeAxisWidget` pressedMouseMove and `startScrollPrice`/`scrollPriceTo`
//! - wheel/pinch zoom increments — reference chart-widget.ts `_onMousewheel` and pane-widget.ts
//!   `pinchEvent`
//! - animated scroll-to-position — cubic ease-out progress (the host only schedules frames)

use aeris_charts_core::model::kinetic_animation::KineticAnimation;

use super::*;

/// Maximum simultaneous pointers retained by one chart. Chart gestures use at most two; extra
/// palm/stylus contacts are rejected instead of allocating or perturbing the active gesture.
pub const MAX_ACTIVE_POINTERS: usize = 2;

#[repr(u8)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum InputDevice {
    #[default]
    Mouse = 0,
    Touch = 1,
    Pen = 2,
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum InputTarget {
    #[default]
    Pane = 0,
    Drawing = 1,
    Trading = 2,
    PriceAxis = 3,
    TimeAxis = 4,
    Separator = 5,
    Alert = 6,
    TimelineMark = 7,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WheelBehavior {
    #[default]
    Auto,
    Pan,
    Zoom,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WheelDeltaMode {
    #[default]
    Pixel,
    Line,
    Page,
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WheelIntent {
    #[default]
    Ignore = 0,
    Pan = 1,
    Zoom = 2,
    PanAndZoom = 3,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WheelSample {
    pub x: f64,
    pub y: f64,
    /// Browser-normalized horizontal delta divided by 100; positive pans in the DOM deltaX
    /// direction. GPUI adapters reverse the native horizontal sign before constructing this.
    pub delta_x: f64,
    /// Browser-normalized vertical delta divided by 100; wheel-up is positive.
    pub delta_y: f64,
    pub delta_mode: WheelDeltaMode,
    pub modifiers: InputModifiers,
    pub timestamp_ms: f64,
}

impl WheelSample {
    /// Normalize one raw wheel axis into the controller's 100-pixel notch units. Pixel deltas
    /// that arrive in device pixels supply their DPR; logical-pixel sources supply `1.0`.
    /// DOM line-mode deltas use 32 px per line, while page mode retains 120 px per page.
    pub fn normalize_delta(raw: f64, mode: WheelDeltaMode, pixel_ratio: f64) -> f64 {
        if !raw.is_finite() {
            return 0.0;
        }
        let pixels = match mode {
            WheelDeltaMode::Pixel => {
                raw / if pixel_ratio.is_finite() && pixel_ratio > 0.0 {
                    pixel_ratio
                } else {
                    1.0
                }
            }
            WheelDeltaMode::Line => raw * 32.0,
            WheelDeltaMode::Page => raw * 120.0,
        };
        pixels / 100.0
    }

    pub fn intent(self, behavior: WheelBehavior) -> WheelIntent {
        match behavior {
            WheelBehavior::Pan => WheelIntent::Pan,
            WheelBehavior::Zoom => WheelIntent::Zoom,
            WheelBehavior::Auto => {
                // the public reference does not reinterpret modifiers: browsers remain free to
                // expose Shift+wheel as deltaX, and the two non-zero axes execute independently.
                match (self.delta_x != 0.0, self.delta_y != 0.0) {
                    (true, true) => WheelIntent::PanAndZoom,
                    (true, false) => WheelIntent::Pan,
                    (false, true) => WheelIntent::Zoom,
                    (false, false) => WheelIntent::Ignore,
                }
            }
        }
    }
}

#[cfg(test)]
mod wheel_normalization_tests {
    use super::{WheelDeltaMode, WheelSample};

    #[test]
    fn browser_and_gpui_raw_notches_normalize_identically_at_each_dpr() {
        for dpr in [1.0, 1.5, 2.0] {
            let browser = WheelSample::normalize_delta(100.0 * dpr, WheelDeltaMode::Pixel, dpr);
            let gpui = WheelSample::normalize_delta(3.0 * (25.0 / 24.0), WheelDeltaMode::Line, 1.0);
            assert_eq!(browser, 1.0);
            assert_eq!(gpui, browser);
            assert_eq!(
                WheelSample::normalize_delta(-100.0 * dpr, WheelDeltaMode::Pixel, dpr),
                -gpui
            );
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum InputEvent {
    PointerDown(PointerSample),
    PointerMove(PointerSample),
    PointerUp(PointerSample),
    LongPress(u32),
    CancelAll(CancelReason),
    Wheel(WheelSample),
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PointerSample {
    pub id: u32,
    pub device: InputDevice,
    pub target: InputTarget,
    pub modifiers: InputModifiers,
    pub x: f64,
    pub y: f64,
    pub timestamp_ms: f64,
    pub pressure: f64,
    pub tilt_x: f64,
    pub tilt_y: f64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CancelReason {
    #[default]
    PointerCancelled,
    LostPointerCapture,
    WindowBlur,
    DocumentHidden,
    Resize,
    BackendLoss,
    Disposed,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InputModifiers {
    pub shift: bool,
    pub control: bool,
    pub alt: bool,
    pub meta: bool,
}

/// Deterministic chart-space context for one intentional secondary click. Hosts own the menu or
/// action UI; the engine owns pane, time, logical-index, hit-series, and price-scale resolution.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChartContext {
    pub x: f64,
    pub y: f64,
    pub pane_index: usize,
    pub time: Option<f64>,
    pub logical: Option<f64>,
    pub price: f64,
    pub series: Option<SeriesId>,
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GestureState {
    #[default]
    Idle,
    Hovering,
    PendingSinglePointer,
    Panning,
    Pinching,
    Inspecting,
    DraggingObject,
    ScalingAxis,
    ResizingPane,
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GestureUpdateKind {
    #[default]
    None = 0,
    Hover = 1,
    Pressed = 2,
    DragStarted = 3,
    DragMoved = 4,
    PinchStarted = 5,
    PinchMoved = 6,
    RebasedSinglePointer = 7,
    Released = 8,
    Cancelled = 9,
    LongPress = 10,
    Rejected = 11,
}

/// Allocation-free result returned to a platform adapter for each normalized input sample.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GestureUpdate {
    pub kind: GestureUpdateKind,
    pub state: GestureState,
    pub pointer_id: u32,
    pub target: InputTarget,
    pub device: InputDevice,
    pub x: f64,
    pub y: f64,
    pub previous_x: f64,
    pub previous_y: f64,
    /// Incremental distance ratio (`current / previous - 1`) for a pinch sample.
    pub scale_delta: f64,
    pub active_pointers: u8,
    pub prevent_default: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct ActivePointer {
    start: PointerSample,
    current: PointerSample,
}

/// Shared pointer/gesture recognizer. Hosts retain one instance and execute the returned semantic
/// updates through `ChartEngine`; DOM/GPUI capture and scheduling stay at their platform boundary.
pub struct GestureResolver {
    pointers: [Option<ActivePointer>; MAX_ACTIVE_POINTERS],
    state: GestureState,
    primary_id: Option<u32>,
    pinch_ids: Option<(u32, u32)>,
    pinch_start_centroid: (f64, f64),
    pinch_start_distance: f64,
    pinch_previous_scale: f64,
    pinch_primary_id: Option<u32>,
    pinch_allowed: bool,
}

impl Default for GestureResolver {
    fn default() -> Self {
        Self {
            pointers: [None; MAX_ACTIVE_POINTERS],
            state: GestureState::Idle,
            primary_id: None,
            pinch_ids: None,
            pinch_start_centroid: (0.0, 0.0),
            pinch_start_distance: 0.0,
            pinch_previous_scale: 1.0,
            pinch_primary_id: None,
            pinch_allowed: true,
        }
    }
}

impl GestureResolver {
    pub fn state(&self) -> GestureState {
        self.state
    }

    pub fn active_pointer_count(&self) -> usize {
        self.pointers.iter().flatten().count()
    }

    pub(crate) fn contains_pointer(&self, id: u32) -> bool {
        self.find(id).is_some()
    }

    /// Current x of up to two retained touch pointers and how many were found.
    pub(crate) fn touch_xs(&self) -> ([f64; 2], usize) {
        let mut xs = [0.0; 2];
        let mut count = 0;
        for pointer in self.pointers.iter().flatten() {
            if pointer.current.device == InputDevice::Touch && count < xs.len() {
                xs[count] = pointer.current.x;
                count += 1;
            }
        }
        (xs, count)
    }

    pub fn pointer_down(&mut self, sample: PointerSample) -> GestureUpdate {
        if !sample.x.is_finite()
            || !sample.y.is_finite()
            || !sample.timestamp_ms.is_finite()
            || self.find(sample.id).is_some()
        {
            return self.update(GestureUpdateKind::Rejected, sample, sample.x, sample.y, 0.0);
        }
        let Some(slot_index) = self.pointers.iter().position(Option::is_none) else {
            return self.update(GestureUpdateKind::Rejected, sample, sample.x, sample.y, 0.0);
        };
        self.pointers[slot_index] = Some(ActivePointer {
            start: sample,
            current: sample,
        });

        let mut touch_ids = self
            .pointers
            .iter()
            .flatten()
            .filter(|pointer| pointer.current.device == InputDevice::Touch)
            .map(|pointer| pointer.current.id);
        let first_touch = touch_ids.next();
        let second_touch = touch_ids.next();
        let third_touch = touch_ids.next();
        if let (Some(first), Some(second), None) = (first_touch, second_touch, third_touch) {
            let pane_owned = [first, second].into_iter().all(|id| {
                self.find(id).is_some_and(|index| {
                    self.pointers[index]
                        .is_some_and(|pointer| pointer.current.target == InputTarget::Pane)
                })
            });
            if !self.pinch_allowed || self.state == GestureState::Inspecting || !pane_owned {
                self.pointers[slot_index] = None;
                return self.update(GestureUpdateKind::Rejected, sample, sample.x, sample.y, 0.0);
            }
            let (centroid, distance) = self.pinch_geometry(first, second).unwrap_or_default();
            self.pinch_primary_id = self.primary_id.or(Some(first));
            self.primary_id = None;
            self.pinch_ids = Some((first, second));
            self.pinch_start_centroid = centroid;
            self.pinch_start_distance = distance;
            self.pinch_previous_scale = 1.0;
            self.state = GestureState::Pinching;
            let mut update = self.update(
                GestureUpdateKind::PinchStarted,
                sample,
                centroid.0,
                centroid.1,
                0.0,
            );
            update.x = centroid.0;
            update.y = centroid.1;
            update.previous_x = centroid.0;
            update.previous_y = centroid.1;
            return update;
        }
        if self.active_pointer_count() == 1 {
            self.primary_id = Some(sample.id);
            self.pinch_allowed = true;
            self.state = GestureState::PendingSinglePointer;
            return self.update(GestureUpdateKind::Pressed, sample, sample.x, sample.y, 0.0);
        }
        self.pointers[slot_index] = None;
        self.update(GestureUpdateKind::Rejected, sample, sample.x, sample.y, 0.0)
    }

    pub fn pointer_move(&mut self, sample: PointerSample) -> GestureUpdate {
        let Some(index) = self.find(sample.id) else {
            self.state = GestureState::Hovering;
            return self.update(GestureUpdateKind::Hover, sample, sample.x, sample.y, 0.0);
        };
        let previous = self.pointers[index].expect("located pointer").current;
        self.pointers[index]
            .as_mut()
            .expect("located pointer")
            .current = sample;

        if let Some((first, second)) = self.pinch_ids {
            let Some((_, distance)) = self.pinch_geometry(first, second) else {
                return self.update(GestureUpdateKind::None, sample, previous.x, previous.y, 0.0);
            };
            let scale = if self.pinch_start_distance > f64::EPSILON {
                distance / self.pinch_start_distance
            } else {
                1.0
            };
            let scale_delta = scale - self.pinch_previous_scale;
            self.pinch_previous_scale = scale;
            self.state = GestureState::Pinching;
            let mut update = self.update(
                GestureUpdateKind::PinchMoved,
                sample,
                self.pinch_start_centroid.0,
                self.pinch_start_centroid.1,
                scale_delta,
            );
            update.x = self.pinch_start_centroid.0;
            update.y = self.pinch_start_centroid.1;
            return update;
        }

        if self.primary_id != Some(sample.id) {
            return self.update(GestureUpdateKind::None, sample, previous.x, previous.y, 0.0);
        }
        if self.state == GestureState::PendingSinglePointer {
            let start = self.pointers[index].expect("located pointer").start;
            if (sample.x - start.x).abs() + (sample.y - start.y).abs()
                < crate::chart_input::CLICK_SLOP_MANHATTAN
            {
                return self.update(GestureUpdateKind::None, sample, previous.x, previous.y, 0.0);
            }
            self.state = match sample.target {
                InputTarget::Pane => GestureState::Panning,
                InputTarget::Drawing
                | InputTarget::Trading
                | InputTarget::Alert
                | InputTarget::TimelineMark => GestureState::DraggingObject,
                InputTarget::PriceAxis | InputTarget::TimeAxis => GestureState::ScalingAxis,
                InputTarget::Separator => GestureState::ResizingPane,
            };
            self.pinch_allowed = false;
            return self.update(
                GestureUpdateKind::DragStarted,
                sample,
                previous.x,
                previous.y,
                0.0,
            );
        }
        self.update(
            GestureUpdateKind::DragMoved,
            sample,
            previous.x,
            previous.y,
            0.0,
        )
    }

    pub fn pointer_up(&mut self, sample: PointerSample) -> GestureUpdate {
        let Some(index) = self.find(sample.id) else {
            return self.update(GestureUpdateKind::Rejected, sample, sample.x, sample.y, 0.0);
        };
        self.pointers[index] = None;
        if self.pinch_ids.is_some() {
            self.pinch_ids = None;
            let pinch_primary_id = self.pinch_primary_id.take();
            if let Some(remaining) = self.pointers.iter_mut().flatten().next() {
                let current = remaining.current;
                if pinch_primary_id == Some(current.id) {
                    self.primary_id = Some(current.id);
                    self.state = GestureState::PendingSinglePointer;
                    self.pinch_allowed = false;
                    return self.update(
                        GestureUpdateKind::RebasedSinglePointer,
                        current,
                        current.x,
                        current.y,
                        0.0,
                    );
                }
                self.primary_id = None;
                self.state = GestureState::Idle;
                self.pinch_allowed = false;
                return self.update(GestureUpdateKind::Released, sample, sample.x, sample.y, 0.0);
            }
        }
        self.primary_id = None;
        self.pinch_primary_id = None;
        self.pinch_allowed = true;
        self.state = GestureState::Idle;
        self.update(GestureUpdateKind::Released, sample, sample.x, sample.y, 0.0)
    }

    pub fn long_press(&mut self, pointer_id: u32) -> GestureUpdate {
        let Some(pointer) = self.find(pointer_id).and_then(|index| self.pointers[index]) else {
            return GestureUpdate::default();
        };
        if self.state != GestureState::PendingSinglePointer {
            return GestureUpdate::default();
        }
        self.state = GestureState::Inspecting;
        self.pinch_allowed = false;
        self.update(
            GestureUpdateKind::LongPress,
            pointer.current,
            pointer.current.x,
            pointer.current.y,
            0.0,
        )
    }

    pub fn cancel(&mut self) -> GestureUpdate {
        let pointer = self
            .pointers
            .iter()
            .flatten()
            .next()
            .map(|pointer| pointer.current)
            .unwrap_or_default();
        self.pointers.fill(None);
        self.primary_id = None;
        self.pinch_ids = None;
        self.pinch_primary_id = None;
        self.pinch_allowed = true;
        self.state = GestureState::Idle;
        self.update(
            GestureUpdateKind::Cancelled,
            pointer,
            pointer.x,
            pointer.y,
            0.0,
        )
    }

    fn find(&self, id: u32) -> Option<usize> {
        self.pointers
            .iter()
            .position(|entry| entry.is_some_and(|pointer| pointer.current.id == id))
    }

    fn pinch_geometry(&self, first: u32, second: u32) -> Option<((f64, f64), f64)> {
        let first = self.pointers[self.find(first)?]?.current;
        let second = self.pointers[self.find(second)?]?.current;
        Some((
            ((first.x + second.x) * 0.5, (first.y + second.y) * 0.5),
            (first.x - second.x).hypot(first.y - second.y),
        ))
    }

    fn update(
        &self,
        kind: GestureUpdateKind,
        sample: PointerSample,
        previous_x: f64,
        previous_y: f64,
        scale_delta: f64,
    ) -> GestureUpdate {
        GestureUpdate {
            kind,
            state: self.state,
            pointer_id: sample.id,
            target: sample.target,
            device: sample.device,
            x: sample.x,
            y: sample.y,
            previous_x,
            previous_y,
            scale_delta,
            active_pointers: self.active_pointer_count() as u8,
            prevent_default: sample.device == InputDevice::Touch
                && self.state != GestureState::Hovering
                && kind != GestureUpdateKind::Rejected,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HitProfile {
    pub drawing_anchor_radius: f64,
    pub drawing_stroke_tolerance: f64,
    pub trading_line_tolerance: f64,
    pub control_half_size: f64,
    pub separator_tolerance: f64,
}

impl HitProfile {
    pub const PRECISION: Self = Self {
        drawing_anchor_radius: 6.5,
        drawing_stroke_tolerance: 3.0,
        trading_line_tolerance: 6.0,
        control_half_size: 10.0,
        separator_tolerance: 4.0,
    };
    pub const TOUCH: Self = Self {
        drawing_anchor_radius: 22.0,
        drawing_stroke_tolerance: 12.0,
        trading_line_tolerance: 22.0,
        control_half_size: 22.0,
        separator_tolerance: 12.0,
    };

    pub fn for_device(device: InputDevice) -> Self {
        if device == InputDevice::Touch {
            Self::TOUCH
        } else {
            Self::PRECISION
        }
    }
}

/// reference `KineticScrollConstants` (pane-widget.ts) in the px domain. The reference samples the
/// time scale's logical `rightOffset`, so these values are divided by the current bar spacing when
/// a drag begins. Keeping the sampler in rightOffset units makes the coast feel invariant across
/// zoom levels and matches the source implementation exactly.
pub const KINETIC_MIN_SPEED: f64 = 0.2;
pub const KINETIC_MAX_SPEED: f64 = 7.0;
pub const KINETIC_DUMPING: f64 = 0.997;
pub const KINETIC_MIN_MOVE: f64 = 15.0;
/// Keyboard navigation is a low-friction velocity model. A held key receives small internal
/// velocity kicks at a fixed cadence; between kicks velocity decays only slightly. This avoids the
/// sticky zero-velocity startup of a force/thrust model while still keeping motion bounded.
pub const KEYBOARD_KINETIC_DAMPING: f64 = 0.9985;
pub const KEYBOARD_KINETIC_IMPULSE_INTERVAL_MS: f64 = 70.0;
/// Logical bars / ms added by one internal keyboard impulse per arrow step.
pub const KEYBOARD_KINETIC_IMPULSE_SPEED_PER_STEP: f64 = 0.0045;
/// Hard velocity ceiling per arrow step. Ctrl/Shift retain the existing 10x relationship.
pub const KEYBOARD_KINETIC_MAX_SPEED_PER_STEP: f64 = 0.020;

/// reference pane-widget.ts `pinchEvent`: the incremental scale multiplier per pinch step.
pub const PINCH_ZOOM_INTENSITY: f64 = 5.0;

/// reference chart-widget.ts `_onMousewheel`: `scrollChart(deltaX * -80)` — "80 is a made up
/// coefficient, and minus is for the 'natural' scroll".
pub const WHEEL_SCROLL_PX_PER_DELTA: f64 = -80.0;

/// Convert a host-normalized wheel delta to the independently measured reference zoom increment. The input
/// saturates at one normalized wheel step so unusually large OS/browser deltas stay bounded.
pub fn wheel_zoom_scale(delta_y: f64) -> f64 {
    delta_y.signum() * delta_y.abs().min(1.0)
}

/// reference pane-widget.ts `pinchEvent`: the scale ratio delta since the previous step, times
/// the intensity (the engine clamps the resulting spacing).
pub fn pinch_zoom_scale(scale_delta: f64) -> f64 {
    scale_delta * PINCH_ZOOM_INTENSITY
}

/// An in-flight animated scroll (reference `scrollToPosition(position, animated)` semantics):
/// cubic ease-out from the position at `start` to `target` over `duration_ms`. The host's only
/// jobs are scheduling a frame per tick and cancelling on a newer scroll or user gesture.
#[derive(Clone, Copy, Debug)]
pub struct ScrollAnimation {
    pub start_position: f64,
    pub target_position: f64,
    pub start_time_ms: f64,
    pub duration_ms: f64,
}

impl ScrollAnimation {
    /// Cubic ease-out progress in [0, 1] (1 completes the animation).
    fn progress(&self, now_ms: f64) -> f64 {
        if self.duration_ms <= 0.0 {
            return 1.0;
        }
        ((now_ms - self.start_time_ms) / self.duration_ms).clamp(0.0, 1.0)
    }

    /// The eased position at `now_ms`.
    fn position(&self, now_ms: f64) -> f64 {
        let t = self.progress(now_ms);
        let eased = 1.0 - (1.0 - t).powi(3);
        self.start_position + (self.target_position - self.start_position) * eased
    }
}

/// Velocity-owned keyboard pan. It launches with an immediate velocity kick and receives further
/// engine-timed kicks while held. Light damping between kicks produces a slippery glide without
/// relying on OS key-repeat cadence. Key-up cancels this state immediately.
#[derive(Clone, Copy, Debug)]
pub struct KeyboardKineticScroll {
    pub position: f64,
    pub velocity: f64,
    pub impulse_velocity: f64,
    pub max_velocity: f64,
    pub last_time_ms: f64,
    pub next_impulse_ms: f64,
}

impl KeyboardKineticScroll {
    fn integrate_to(&mut self, target_time_ms: f64) {
        let elapsed = (target_time_ms - self.last_time_ms).max(0.0);
        if elapsed == 0.0 {
            return;
        }
        let rate = -KEYBOARD_KINETIC_DAMPING.ln();
        let decay = KEYBOARD_KINETIC_DAMPING.powf(elapsed);
        self.position += self.velocity * (1.0 - decay) / rate;
        self.velocity *= decay;
        self.last_time_ms = target_time_ms;
    }

    fn kick(&mut self) {
        self.velocity =
            (self.velocity + self.impulse_velocity).clamp(-self.max_velocity, self.max_velocity);
    }

    fn advance(&mut self, now_ms: f64) {
        if !now_ms.is_finite() {
            return;
        }
        while self.next_impulse_ms <= now_ms {
            self.integrate_to(self.next_impulse_ms);
            self.kick();
            self.next_impulse_ms += KEYBOARD_KINETIC_IMPULSE_INTERVAL_MS;
        }
        self.integrate_to(now_ms);
    }
}

impl ChartEngine {
    // --- canonical time-scale mutation boundary ---

    pub fn time_scale_zoom(&mut self, x: f64, scale: f64) {
        self.time_scale.zoom(x, scale);
    }

    /// Focused zoom: keep the logical point under `x` fixed even when ordinary wheel zoom keeps the
    /// right-most bar pinned. Ctrl/Cmd wheel zoom and pinch (see `wheel_zoom_time_scale`) use it.
    pub fn time_scale_zoom_focused(&mut self, x: f64, scale: f64) {
        self.time_scale.zoom_focused(x, scale);
    }

    pub fn time_scale_start_scroll(&mut self, x: f64) {
        self.time_scale.start_scroll(x);
    }

    pub fn time_scale_scroll_to(&mut self, x: f64) {
        self.time_scale.scroll_to(x);
    }

    pub fn time_scale_end_scroll(&mut self) {
        self.time_scale.end_scroll();
    }

    // --- kinetic (momentum) scroll ---

    /// Open a kinetic sampling session alongside a drag-scroll. The reference samples logical
    /// right-offset values and scales all px-domain thresholds by the current bar spacing.
    /// `enabled = false` mirrors its `_scrollXAnimation = null`. Seeds the first logical sample.
    pub fn kinetic_begin_sampling(&mut self, enabled: bool, position: f64, now_ms: f64) {
        let spacing = self.time_scale.bar_spacing().max(f64::EPSILON);
        self.kinetic = enabled.then(|| {
            let mut animation = KineticAnimation::new(
                KINETIC_MIN_SPEED / spacing,
                KINETIC_MAX_SPEED / spacing,
                KINETIC_DUMPING,
                KINETIC_MIN_MOVE / spacing,
            );
            animation.add_position(position, now_ms);
            animation
        });
    }

    /// Feed a drag-move logical right-offset sample (reference `addPosition`).
    pub fn kinetic_add_sample(&mut self, position: f64, now_ms: f64) {
        if let Some(animation) = self.kinetic.as_mut() {
            animation.add_position(position, now_ms);
        }
    }

    /// The drag was released: freeze the coast. Returns whether a coast engaged (the host then
    /// drives `kinetic_position` from its frame scheduler instead of ending the scroll session).
    pub fn kinetic_release(&mut self, position: f64, now_ms: f64) -> bool {
        let Some(animation) = self.kinetic.as_mut() else {
            return false;
        };
        animation.start(position, now_ms);
        !animation.finished(now_ms)
    }

    /// The coast's position at `now_ms`; `None` when no coast is engaged.
    pub fn kinetic_position(&self, now_ms: f64) -> Option<f64> {
        self.kinetic.as_ref().map(|a| a.position(now_ms))
    }

    /// Whether the coast has run its course (true when none is engaged).
    pub fn kinetic_finished(&self, now_ms: f64) -> bool {
        self.kinetic.as_ref().is_none_or(|a| a.finished(now_ms))
    }

    /// Drop the sampler/coast entirely (a fresh gesture supersedes any in-flight coast).
    pub fn kinetic_stop(&mut self) {
        self.kinetic = None;
    }

    // --- keyboard kinetic pan ---

    /// Start or retune one held keyboard-pan session. The engine owns the impulse cadence; browser
    /// key-repeat does not drive motion. The existing one/ten-step distinction controls kick
    /// strength and velocity ceiling.
    pub fn start_keyboard_scroll(&mut self, delta_bars: f64, now_ms: f64) {
        if !delta_bars.is_finite() || delta_bars == 0.0 || !now_ms.is_finite() {
            return;
        }
        let impulse_velocity = delta_bars * KEYBOARD_KINETIC_IMPULSE_SPEED_PER_STEP;
        let max_velocity = delta_bars.abs() * KEYBOARD_KINETIC_MAX_SPEED_PER_STEP;
        if let Some(animation) = self.keyboard_scroll_animation.as_mut() {
            animation.advance(now_ms);
            if animation.velocity != 0.0 && animation.velocity.signum() != delta_bars.signum() {
                animation.velocity = 0.0;
            }
            animation.impulse_velocity = impulse_velocity;
            animation.max_velocity = max_velocity;
            if animation.velocity == 0.0 {
                animation.kick();
            }
            animation.next_impulse_ms = now_ms + KEYBOARD_KINETIC_IMPULSE_INTERVAL_MS;
            let position = animation.position;
            self.scroll_to_position(position);
            return;
        }
        let mut animation = KeyboardKineticScroll {
            position: self.scroll_position(),
            velocity: 0.0,
            impulse_velocity,
            max_velocity,
            last_time_ms: now_ms,
            next_impulse_ms: now_ms + KEYBOARD_KINETIC_IMPULSE_INTERVAL_MS,
        };
        animation.kick();
        self.keyboard_scroll_animation = Some(animation);
    }

    /// Apply one held keyboard-pan tick. `None` means no key-owned kinetic session is active.
    pub fn keyboard_scroll_tick(&mut self, now_ms: f64) -> Option<f64> {
        let animation = self.keyboard_scroll_animation.as_mut()?;
        animation.advance(now_ms);
        let position = animation.position;
        self.scroll_to_position(position);
        Some(position)
    }

    pub fn cancel_keyboard_scroll(&mut self) {
        self.keyboard_scroll_animation = None;
    }

    pub fn keyboard_scroll_active(&self) -> bool {
        self.keyboard_scroll_animation.is_some()
    }

    // --- axis drag-to-scale ---

    /// reference `TimeAxisWidget` pressedMouseMove arm (`TimeScale.startScale`).
    pub fn time_axis_start_scale(&mut self, x: f64) {
        self.time_scale.start_scale(x);
    }

    /// reference `TimeScale.scaleTo` (bar spacing by the ratio of distances-from-right).
    pub fn time_axis_scale_to(&mut self, x: f64) {
        self.time_scale.scale_to(x);
        self.invalidate_frame_scene();
    }

    pub fn time_axis_end_scale(&mut self) {
        self.time_scale.end_scale();
    }

    /// Whether a drag on this price axis can scale it (reference `PriceScale.scaleTo` no-ops in
    /// percentage and indexed-to-100 modes; an empty scale has nothing to scale).
    pub fn price_axis_scalable(&self, pane: usize, target: PriceScaleTarget) -> bool {
        let Some(scale) = self.price_scale_for(pane, target) else {
            return false;
        };
        !scale.is_percentage() && !scale.is_indexed_to_100() && scale.price_range().is_some()
    }

    /// reference `PriceAxisWidget` pressedMouseMove arm (`PriceScale.startScale`); `y` is the
    /// chart-content coordinate (the scale crops itself to the pane via internal margins).
    pub fn price_axis_start_scale(&mut self, pane: usize, target: PriceScaleTarget, y: f64) {
        if let Some(scale) = self.price_scale_for_mut(pane, target) {
            scale.start_scale(y);
        }
    }

    /// reference `PriceScale.scaleTo` (the start range scaled around its center).
    pub fn price_axis_scale_to(&mut self, pane: usize, target: PriceScaleTarget, y: f64) {
        if let Some(scale) = self.price_scale_for_mut(pane, target) {
            scale.scale_to(y);
            self.invalidate_frame_scene();
        }
    }

    pub fn price_axis_end_scale(&mut self, pane: usize, target: PriceScaleTarget) {
        if let Some(scale) = self.price_scale_for_mut(pane, target) {
            scale.end_scale();
        }
    }

    /// industry-standard wheel zoom on a price axis (the reference has no price-axis wheel;
    /// the time axis wheel is `_onMousewheel` → `zoomTime`). `scale` is the same normalized
    /// increment the time-axis wheel consumes (`wheel_zoom_scale`), converted to a per-notch
    /// range factor: 10% per full notch, anchored at the cursor's price.
    pub fn price_axis_wheel_zoom(
        &mut self,
        pane: usize,
        target: PriceScaleTarget,
        y: f64,
        scale: f64,
    ) {
        let factor = (1.0 - scale * 0.1).clamp(0.05, 20.0);
        if let Some(price_scale) = self.price_scale_for_mut(pane, target) {
            price_scale.zoom(y, factor);
            self.invalidate_frame_scene();
        }
    }

    // --- vertical price pan ---

    /// reference chart-model.ts `startScrollPrice`: no-ops while the scale is in autoscale.
    pub fn price_axis_start_scroll(&mut self, pane: usize, target: PriceScaleTarget, y: f64) {
        if let Some(scale) = self.price_scale_for_mut(pane, target) {
            scale.start_scroll(y);
        }
    }

    /// reference chart-model.ts `scrollPriceTo`: the start range shifted by `dy * span/(h-1)`.
    pub fn price_axis_scroll_to(&mut self, pane: usize, target: PriceScaleTarget, y: f64) {
        if let Some(scale) = self.price_scale_for_mut(pane, target) {
            scale.scroll_to(y);
            self.invalidate_frame_scene();
        }
    }

    pub fn price_axis_end_scroll(&mut self, pane: usize, target: PriceScaleTarget) {
        if let Some(scale) = self.price_scale_for_mut(pane, target) {
            scale.end_scroll();
        }
    }

    /// Resolve a pane drag to the scale owned by the series the user intended to grab.
    /// A selected series wins when it is itself under the pointer; otherwise the canonical series
    /// hit arbitration chooses the target. Empty pane space may continue dragging an explicitly
    /// selected visible series, then falls back to the pane's canonical default price scale.
    pub fn price_pan_target_at(
        &self,
        pane: usize,
        x_css: f64,
        y_css: f64,
    ) -> Option<PriceScaleTarget> {
        if self.pane_at_y(y_css)? != pane {
            return None;
        }
        let selected = self.selected_series().and_then(|id| {
            let series = self.series_entry(id)?;
            (series.visible && series.pane_index == pane).then_some(id)
        });
        let series = selected
            .filter(|id| self.hit_test_one_series(*id, x_css, y_css).is_some())
            .or_else(|| self.hit_test_series(x_css, y_css))
            .or(selected);
        if let Some(series) = series
            && let Some((series_pane, target)) = self.series_price_scale(series)
            && series_pane == pane
        {
            return Some(target);
        }
        Some(self.pane_default_scale_target(pane))
    }

    /// Resolve and begin one pane price-pan session on the intended series' already-manual scale.
    /// Autoscaled scales stay locked; grabbing series geometry never changes that state.
    pub fn begin_price_pan_at(
        &mut self,
        pane: usize,
        x_css: f64,
        y_css: f64,
    ) -> Option<PriceScaleTarget> {
        let target = self.price_pan_target_at(pane, x_css, y_css)?;
        let scale = self.price_scale_for_mut(pane, target)?;
        scale.start_scroll(y_css);
        if scale.is_auto_scale() {
            return None;
        }
        Some(target)
    }

    /// Enable or disable the temporary modifier-driven OHLC magnet.
    pub fn set_crosshair_ohlc_magnet(&mut self, enabled: bool) -> bool {
        if self.crosshair_ohlc_magnet == enabled {
            return false;
        }
        self.crosshair_ohlc_magnet = enabled;
        self.invalidate_frame_overlay();
        true
    }

    /// Current pane-separator hover target.
    #[must_use]
    pub const fn separator_hover(&self) -> Option<usize> {
        self.separator_hover
    }

    /// Configured crosshair mode in the public wire representation.
    #[must_use]
    pub fn configured_crosshair_mode(&self) -> u8 {
        self.options.get().crosshair.mode
    }

    /// Set the configured crosshair mode from its stable public wire value.
    pub fn set_configured_crosshair_mode(&mut self, mode: u8) -> bool {
        if mode > 3 || self.configured_crosshair_mode() == mode {
            return false;
        }
        self.options
            .apply(&serde_json::json!({"crosshair": {"mode": mode}}));
        self.crosshair_mode = crosshair_mode_from_u8(mode);
        self.invalidate_frame_overlay();
        true
    }

    /// Resolve a pane separator using a host-selected interaction halo.
    #[must_use]
    pub fn pane_separator_at(&self, y: f64, hit_radius: f64) -> Option<usize> {
        if !y.is_finite() || !hit_radius.is_finite() || hit_radius < 0.0 {
            return None;
        }
        self.panes
            .iter()
            .skip(1)
            .position(|pane| (y - pane.top).abs() <= hit_radius)
    }

    // --- animated scroll-to-position ---

    /// Start an eased scroll to `target_position` (logical bars from the right edge), replacing
    /// any in-flight animation. The engine applies each tick itself; the host schedules frames
    /// and repaints.
    pub fn start_scroll_animation(&mut self, target_position: f64, duration_ms: f64, now_ms: f64) {
        if !target_position.is_finite() {
            return;
        }
        self.scroll_animation = Some(ScrollAnimation {
            start_position: self.scroll_position(),
            target_position,
            start_time_ms: now_ms,
            duration_ms: duration_ms.max(0.0),
        });
    }

    /// Apply the animation's eased position for `now_ms`. Returns the applied position, or
    /// `None` when no animation is running (finished animations self-clear after applying the
    /// final position).
    pub fn scroll_animation_tick(&mut self, now_ms: f64) -> Option<f64> {
        let animation = self.scroll_animation?;
        let position = animation.position(now_ms);
        self.scroll_to_position(position);
        if animation.progress(now_ms) >= 1.0 {
            self.scroll_animation = None;
            return None;
        }
        Some(position)
    }

    /// Invalidate any in-flight animated scroll (a new scroll call or user gesture supersedes).
    pub fn cancel_scroll_animation(&mut self) {
        self.scroll_animation = None;
    }

    pub fn scroll_animation_active(&self) -> bool {
        self.scroll_animation.is_some()
    }

    // --- pane hit-testing ---

    /// Index of the stacked pane containing content-y `y`. A pane owns everything above the next
    /// pane's top, so a separator between panes belongs to the pane above; a `y` above the content
    /// resolves to the first pane and one below it to the last, so coordinates outside the stack
    /// still resolve.
    pub fn pane_index_at_y(&self, y: f64) -> usize {
        self.panes
            .windows(2)
            .position(|pair| y < pair[1].top)
            .unwrap_or(self.panes.len().saturating_sub(1))
    }

    /// Resolve a secondary-click payload without mutating hover, selection, drawing, or trading
    /// state. A hit series supplies its exact pane scale; empty pane space uses the same canonical
    /// default scale as the crosshair.
    pub fn chart_context_at(&self, x: f64, y: f64) -> Option<ChartContext> {
        if !x.is_finite() || !y.is_finite() || !(0.0..=self.pane_w).contains(&x) {
            return None;
        }
        let pane_index = self.pane_at_y(y)?;
        let series = self.hit_test_series(x, y);
        let price = if let Some(series) = series {
            self.series_coordinate_to_price(series, y)?
        } else {
            let (from, _) = self.visible_range_for_frame()?;
            let (scale, base) = self.pane_default_scale(pane_index, from);
            if scale.is_empty() {
                return None;
            }
            scale.coordinate_to_price(y, base)
        };
        price.is_finite().then(|| ChartContext {
            x,
            y,
            pane_index,
            time: self.coordinate_to_time(x),
            logical: self.coordinate_to_logical(x),
            price,
            series,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pointer(id: u32, x: f64, y: f64) -> PointerSample {
        PointerSample {
            id,
            device: InputDevice::Touch,
            target: InputTarget::Pane,
            modifiers: InputModifiers::default(),
            x,
            y,
            timestamp_ms: f64::from(id),
            pressure: 0.5,
            tilt_x: 0.0,
            tilt_y: 0.0,
        }
    }

    #[test]
    fn crosshair_modes_and_magnet_are_engine_owned() {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        assert!(chart.set_crosshair_ohlc_magnet(true));
        assert!(!chart.set_crosshair_ohlc_magnet(true));
        assert!(chart.crosshair_ohlc_magnet);
        assert!(chart.set_configured_crosshair_mode(2));
        assert_eq!(chart.configured_crosshair_mode(), 2);
        assert!(!chart.set_configured_crosshair_mode(4));
    }

    #[test]
    fn pinch_uses_the_start_centroid_and_cumulative_scale_differences() {
        let mut input = GestureResolver::default();
        assert_eq!(
            input.pointer_down(pointer(1, 100.0, 100.0)).kind,
            GestureUpdateKind::Pressed
        );
        let start = input.pointer_down(pointer(2, 200.0, 100.0));
        assert_eq!(start.kind, GestureUpdateKind::PinchStarted);
        assert_eq!((start.x, start.y), (150.0, 100.0));

        let moved = input.pointer_move(pointer(2, 230.0, 120.0));
        assert_eq!(moved.kind, GestureUpdateKind::PinchMoved);
        assert_eq!((moved.previous_x, moved.previous_y), (150.0, 100.0));
        assert_eq!((moved.x, moved.y), (150.0, 100.0));
        assert!(moved.scale_delta > 0.0);

        let first_delta = moved.scale_delta;
        let moved_again = input.pointer_move(pointer(2, 260.0, 100.0));
        assert_eq!(moved_again.kind, GestureUpdateKind::PinchMoved);
        assert_eq!((moved_again.x, moved_again.y), (150.0, 100.0));
        assert!(moved_again.scale_delta > 0.0);
        assert!(moved_again.scale_delta < first_delta);

        let rebased = input.pointer_up(pointer(2, 260.0, 100.0));
        assert_eq!(rebased.kind, GestureUpdateKind::RebasedSinglePointer);
        assert_eq!(rebased.pointer_id, 1);
        assert_eq!((rebased.x, rebased.y), (100.0, 100.0));
        let continued = input.pointer_move(pointer(1, 110.0, 100.0));
        assert_eq!(continued.kind, GestureUpdateKind::DragStarted);
        assert_eq!((continued.previous_x, continued.previous_y), (100.0, 100.0));
    }

    #[test]
    fn five_pixel_manhattan_threshold_opens_drag_on_the_crossing_sample() {
        let mut input = GestureResolver::default();
        let mut down = pointer(1, 10.0, 10.0);
        down.device = InputDevice::Mouse;
        assert_eq!(input.pointer_down(down).kind, GestureUpdateKind::Pressed);
        let mut four = down;
        four.x = 12.0;
        four.y = 12.0;
        assert_eq!(input.pointer_move(four).kind, GestureUpdateKind::None);
        let mut five = four;
        five.x = 13.0;
        assert_eq!(
            input.pointer_move(five).kind,
            GestureUpdateKind::DragStarted
        );
        let mut six = five;
        six.x = 14.0;
        assert_eq!(input.pointer_move(six).kind, GestureUpdateKind::DragMoved);
    }

    #[test]
    fn pinch_is_rejected_after_single_touch_move_or_long_press() {
        let mut moved = GestureResolver::default();
        moved.pointer_down(pointer(1, 0.0, 0.0));
        moved.pointer_move(pointer(1, 5.0, 0.0));
        assert_eq!(
            moved.pointer_down(pointer(2, 20.0, 0.0)).kind,
            GestureUpdateKind::Rejected
        );

        let mut inspected = GestureResolver::default();
        inspected.pointer_down(pointer(1, 0.0, 0.0));
        assert_eq!(inspected.long_press(1).kind, GestureUpdateKind::LongPress);
        assert_eq!(
            inspected.pointer_down(pointer(2, 20.0, 0.0)).kind,
            GestureUpdateKind::Rejected
        );
    }

    #[test]
    fn extension_owned_touch_rejects_competing_pinch() {
        let mut input = GestureResolver::default();
        let mut drawing = pointer(1, 0.0, 0.0);
        drawing.target = InputTarget::Drawing;
        input.pointer_down(drawing);
        assert_eq!(
            input.pointer_down(pointer(2, 20.0, 0.0)).kind,
            GestureUpdateKind::Rejected
        );
        assert_eq!(input.state(), GestureState::PendingSinglePointer);
        assert_eq!(input.active_pointer_count(), 1);
    }

    #[test]
    fn lifting_primary_touch_ends_pinch_without_rebasing_secondary() {
        let mut input = GestureResolver::default();
        input.pointer_down(pointer(1, 0.0, 0.0));
        input.pointer_down(pointer(2, 20.0, 0.0));
        assert_eq!(
            input.pointer_up(pointer(1, 0.0, 0.0)).kind,
            GestureUpdateKind::Released
        );
        assert_eq!(
            input.pointer_move(pointer(2, 30.0, 0.0)).kind,
            GestureUpdateKind::None
        );
    }

    #[test]
    fn cancellation_and_pointer_capacity_are_bounded() {
        let mut input = GestureResolver::default();
        for id in 0..MAX_ACTIVE_POINTERS as u32 {
            assert_ne!(
                input.pointer_down(pointer(id, f64::from(id), 0.0)).kind,
                GestureUpdateKind::Rejected
            );
        }
        assert_eq!(
            input.pointer_down(pointer(99, 0.0, 0.0)).kind,
            GestureUpdateKind::Rejected
        );
        assert_eq!(input.cancel().kind, GestureUpdateKind::Cancelled);
        assert_eq!(input.active_pointer_count(), 0);
        assert_eq!(input.state(), GestureState::Idle);
    }

    #[test]
    fn rejected_non_touch_pointer_is_not_retained() {
        let mut input = GestureResolver::default();
        let mut first = pointer(1, 10.0, 10.0);
        first.device = InputDevice::Mouse;
        let mut second = pointer(2, 20.0, 10.0);
        second.device = InputDevice::Pen;
        assert_eq!(input.pointer_down(first).kind, GestureUpdateKind::Pressed);
        assert_eq!(input.pointer_down(second).kind, GestureUpdateKind::Rejected);
        assert_eq!(input.active_pointer_count(), 1);
        assert_eq!(input.pointer_up(first).kind, GestureUpdateKind::Released);
    }

    #[test]
    fn touch_and_precision_profiles_keep_visual_geometry_independent() {
        assert_eq!(
            HitProfile::for_device(InputDevice::Pen),
            HitProfile::PRECISION
        );
        assert_eq!(
            HitProfile::for_device(InputDevice::Mouse),
            HitProfile::PRECISION
        );
        assert_eq!(
            HitProfile::for_device(InputDevice::Touch),
            HitProfile::TOUCH
        );
        assert_eq!(HitProfile::TOUCH.control_half_size * 2.0, 44.0);
    }

    #[test]
    fn wheel_auto_matches_lightweight_charts_axis_and_modifier_semantics() {
        let mut sample = WheelSample {
            delta_y: -0.125,
            ..WheelSample::default()
        };
        assert_eq!(sample.intent(WheelBehavior::Auto), WheelIntent::Zoom);
        sample.modifiers.control = true;
        assert_eq!(sample.intent(WheelBehavior::Auto), WheelIntent::Zoom);
        sample.delta_x = 0.25;
        assert_eq!(sample.intent(WheelBehavior::Auto), WheelIntent::PanAndZoom);
        sample.delta_y = 0.0;
        assert_eq!(sample.intent(WheelBehavior::Auto), WheelIntent::Pan);
        assert_eq!(sample.intent(WheelBehavior::Pan), WheelIntent::Pan);

        sample.delta_x = 0.0;
        sample.delta_y = -0.25;
        sample.modifiers.shift = true;
        assert_eq!(sample.intent(WheelBehavior::Auto), WheelIntent::Zoom);
        assert_eq!(sample.intent(WheelBehavior::Zoom), WheelIntent::Zoom);
    }

    fn chart_with_data(width: f64, height: f64) -> ChartEngine {
        let mut chart = ChartEngine::new(width, height, 1.0);
        chart
            .set_series_data(
                0,
                &[1.0, 2.0, 3.0, 4.0, 5.0],
                &[10.0, 11.0, 12.0, 13.0, 14.0],
                &[11.0, 12.0, 13.0, 14.0, 15.0],
                &[9.0, 10.0, 11.0, 12.0, 13.0],
                &[10.5, 11.5, 12.5, 13.5, 14.5],
            )
            .unwrap();
        chart.time_scale.set_width(width);
        chart.layout_panes(height);
        chart.fit_content();
        chart.autoscale_visible();
        chart
    }

    #[test]
    fn kinetic_coast_samples_logical_offset_with_reference_bar_spacing_tuning() {
        let mut chart = chart_with_data(400.0, 300.0);
        chart.time_scale.set_bar_spacing(10.0);
        chart.scroll_to_position(0.0);
        chart.kinetic_begin_sampling(true, 0.0, 1000.0);
        chart.scroll_to_position(4.0);
        chart.kinetic_add_sample(4.0, 1020.0);
        chart.scroll_to_position(8.0);
        chart.kinetic_add_sample(8.0, 1040.0);
        assert!(
            chart.kinetic_release(8.0, 1040.0),
            "a fast flick engages the coast"
        );
        assert!(!chart.kinetic_finished(1040.0));
        let coast = chart.kinetic_position(1060.0).unwrap();
        assert!(coast > 8.0, "same-direction momentum advances rightOffset");
        assert!(chart.kinetic_finished(100_000.0));
        chart.kinetic_stop();
        assert!(chart.kinetic_position(1060.0).is_none());

        // Reference divides ScrollMinMove (15px) by bar spacing. At spacing 2 the same four-bar
        // change is below the 7.5-bar sampling threshold, so no release velocity can form.
        chart.time_scale.set_bar_spacing(2.0);
        chart.kinetic_begin_sampling(true, 0.0, 2000.0);
        chart.kinetic_add_sample(4.0, 2020.0);
        assert!(!chart.kinetic_release(4.0, 2020.0));
    }

    #[test]
    fn kinetic_disabled_sampler_never_engages() {
        let mut chart = chart_with_data(400.0, 300.0);
        chart.kinetic_begin_sampling(false, 0.0, 1000.0);
        chart.kinetic_add_sample(4.0, 1020.0);
        assert!(!chart.kinetic_release(4.0, 1020.0));
        assert!(chart.kinetic_finished(1020.0));
    }

    #[test]
    fn time_axis_drag_scales_bar_spacing_by_the_right_ratio() {
        let mut chart = chart_with_data(400.0, 300.0);
        let start_spacing = chart.bar_spacing();
        chart.time_axis_start_scale(300.0);
        chart.time_axis_scale_to(200.0);
        // reference TimeScale.scaleTo: start * (width - x) / (width - startX)
        let expected = start_spacing * (400.0 - 200.0) / (400.0 - 300.0);
        assert!((chart.bar_spacing() - expected).abs() < 1e-9);
        chart.time_axis_end_scale();
    }

    #[test]
    fn price_axis_drag_scales_the_range_around_its_center() {
        let mut chart = chart_with_data(400.0, 300.0);
        chart.set_price_scale_auto_scale(0, false, false);
        let (from, to) = chart.price_scale_visible_range(0, false).unwrap();
        assert!(chart.price_axis_scalable(0, PriceScaleTarget::Right));
        chart.price_axis_start_scale(0, PriceScaleTarget::Right, 250.0);
        chart.price_axis_scale_to(0, PriceScaleTarget::Right, 200.0);
        let (new_from, new_to) = chart.price_scale_visible_range(0, false).unwrap();
        // reference PriceScale.scaleTo with height-flipped coordinates (height = 300):
        // coeff = (50 + 299*0.2) / (100 + 299*0.2)
        let coeff: f64 = (50.0 + 299.0 * 0.2) / (100.0 + 299.0 * 0.2);
        let mid = (from + to) / 2.0;
        let half = (to - from) / 2.0 * coeff.max(0.1);
        assert!((new_from - (mid - half)).abs() < 1e-9);
        assert!((new_to - (mid + half)).abs() < 1e-9);
        chart.price_axis_end_scale(0, PriceScaleTarget::Right);
    }

    #[test]
    fn price_axis_drag_noops_in_percentage_mode() {
        let mut chart = chart_with_data(400.0, 300.0);
        chart.set_price_scale_mode(0, false, PriceScaleMode::Percentage);
        assert!(!chart.price_axis_scalable(0, PriceScaleTarget::Right));
    }

    #[test]
    fn price_pan_shifts_the_range_by_pixels() {
        let mut chart = chart_with_data(400.0, 300.0);
        chart.set_price_scale_auto_scale(0, false, false);
        let (from, to) = chart.price_scale_visible_range(0, false).unwrap();
        chart.price_axis_start_scroll(0, PriceScaleTarget::Right, 100.0);
        chart.price_axis_scroll_to(0, PriceScaleTarget::Right, 110.0);
        let (new_from, new_to) = chart.price_scale_visible_range(0, false).unwrap();
        // +10 px down shifts the range up by 10 * span/(internalHeight-1); the internal height
        // is the scale height minus the fractional scale margins.
        let (margin_top, margin_bottom) = chart.price_scale_margins(0, false).unwrap();
        let internal_h = 300.0 * (1.0 - margin_top - margin_bottom);
        let shift = 10.0 * (to - from) / (internal_h - 1.0);
        assert!((new_from - (from + shift)).abs() < 1e-9);
        assert!((new_to - (to + shift)).abs() < 1e-9);
        chart.price_axis_end_scroll(0, PriceScaleTarget::Right);
    }

    #[test]
    fn price_pan_preserves_the_autoscale_lock() {
        let mut chart = chart_with_data(400.0, 300.0);
        let (from, to) = chart.price_scale_visible_range(0, false).unwrap();
        chart.price_axis_start_scroll(0, PriceScaleTarget::Right, 100.0);
        assert_eq!(chart.price_scale_auto_scale(0, false), Some(true));
        chart.price_axis_scroll_to(0, PriceScaleTarget::Right, 120.0);
        let (new_from, new_to) = chart.price_scale_visible_range(0, false).unwrap();
        assert_eq!((new_from, new_to), (from, to));
        chart.price_axis_end_scroll(0, PriceScaleTarget::Right);
    }

    #[test]
    fn every_builtin_series_hit_preserves_its_autoscale_lock() {
        for kind in [
            SeriesKind::Candlestick,
            SeriesKind::Bar,
            SeriesKind::Line,
            SeriesKind::Area,
            SeriesKind::Histogram,
        ] {
            let mut chart = chart_with_data(400.0, 300.0);
            chart.convert_series_kind(0, kind);
            chart.autoscale_visible();
            let x = chart.time_scale.index_to_coordinate(4);
            let y = chart.series_price_to_coordinate(0, 14.5).unwrap();
            assert_eq!(chart.begin_price_pan_at(0, x, y), None, "{kind:?}");
            assert_eq!(chart.price_scale_auto_scale(0, false), Some(true));
        }
    }

    #[test]
    fn price_pan_target_follows_the_grabbed_series_scale_without_order_fallback() {
        let mut chart = chart_with_data(600.0, 300.0);
        let comparison_series = chart.add_series(SeriesKind::Candlestick);
        chart
            .set_series_data(
                comparison_series,
                &[1.0, 2.0, 3.0, 4.0, 5.0],
                &[1_000.0, 1_010.0, 1_020.0, 1_030.0, 1_040.0],
                &[1_005.0, 1_015.0, 1_025.0, 1_035.0, 1_045.0],
                &[995.0, 1_005.0, 1_015.0, 1_025.0, 1_035.0],
                &[1_002.0, 1_012.0, 1_022.0, 1_032.0, 1_042.0],
            )
            .unwrap();
        let comparison = chart
            .add_price_scale(0, "comparison-pan", PriceScaleSide::Left, Some(0), true)
            .unwrap();
        chart.set_series_price_scale(comparison_series, comparison);
        chart.set_price_scale_visible_range_for(0, PriceScaleTarget::Right, 0.0, 40.0);
        chart.set_price_scale_visible_range_for(0, comparison, 900.0, 1_100.0);

        let x = chart.time_scale.index_to_coordinate(4);
        let main_y = chart.series_price_to_coordinate(0, 14.5).unwrap();
        let comparison_y = chart
            .series_price_to_coordinate(comparison_series, 1_042.0)
            .unwrap();
        assert!((main_y - comparison_y).abs() > 20.0);
        assert_eq!(
            chart.price_pan_target_at(0, x, main_y),
            Some(PriceScaleTarget::Right)
        );
        assert_eq!(
            chart.price_pan_target_at(0, x, comparison_y),
            Some(comparison)
        );

        let empty = (0..600)
            .step_by(20)
            .flat_map(|x| {
                (0..300)
                    .step_by(20)
                    .map(move |y| (f64::from(x), f64::from(y)))
            })
            .find(|(x, y)| chart.hit_test_series(*x, *y).is_none())
            .expect("pane has empty space");
        assert_eq!(
            chart.price_pan_target_at(0, empty.0, empty.1),
            Some(PriceScaleTarget::Right)
        );

        chart.set_selected_series(Some(comparison_series));
        assert_eq!(
            chart.price_pan_target_at(0, empty.0, empty.1),
            Some(comparison)
        );

        chart.set_price_scale_auto_scale_for(0, PriceScaleTarget::Right, true);
        chart.set_price_scale_auto_scale_for(0, comparison, true);
        assert_eq!(
            chart.price_pan_target_at(0, x, comparison_y),
            Some(comparison)
        );
        assert_eq!(chart.begin_price_pan_at(0, x, comparison_y), None);
        assert_eq!(chart.price_scale_auto_scale_for(0, comparison), Some(true));
        assert_eq!(
            chart.price_scale_auto_scale_for(0, PriceScaleTarget::Right),
            Some(true)
        );
        chart.price_axis_end_scroll(0, comparison);
    }

    #[test]
    fn chart_context_uses_the_hit_series_scale_without_mutating_selection() {
        let mut chart = chart_with_data(600.0, 300.0);
        let comparison_series = chart.add_series(SeriesKind::Line);
        chart
            .set_series_data(
                comparison_series,
                &[1.0, 2.0, 3.0, 4.0, 5.0],
                &[1_000.0, 1_010.0, 1_020.0, 1_030.0, 1_040.0],
                &[1_000.0, 1_010.0, 1_020.0, 1_030.0, 1_040.0],
                &[1_000.0, 1_010.0, 1_020.0, 1_030.0, 1_040.0],
                &[1_000.0, 1_010.0, 1_020.0, 1_030.0, 1_040.0],
            )
            .unwrap();
        let comparison = chart
            .add_price_scale(0, "context", PriceScaleSide::Left, Some(0), true)
            .unwrap();
        chart.set_series_price_scale(comparison_series, comparison);
        chart.set_price_scale_visible_range_for(0, PriceScaleTarget::Right, 0.0, 40.0);
        chart.set_price_scale_visible_range_for(0, comparison, 900.0, 1_100.0);

        let x = chart.time_scale.index_to_coordinate(4);
        let y = chart
            .series_price_to_coordinate(comparison_series, 1_040.0)
            .unwrap();
        let context = chart.chart_context_at(x, y).expect("chart context");

        assert_eq!(context.pane_index, 0);
        assert_eq!(context.logical, Some(4.0));
        assert_eq!(context.time, Some(5.0));
        assert_eq!(context.series, Some(comparison_series));
        assert!((context.price - 1_040.0).abs() < 1e-9);
        assert_eq!(chart.selected_series(), None);
    }

    #[test]
    fn selected_overlapping_series_owns_price_pan_and_removal_is_safe() {
        let mut chart = chart_with_data(600.0, 300.0);
        let twin = chart.add_series(SeriesKind::Candlestick);
        chart
            .set_series_data(
                twin,
                &[1.0, 2.0, 3.0, 4.0, 5.0],
                &[10.0, 11.0, 12.0, 13.0, 14.0],
                &[11.0, 12.0, 13.0, 14.0, 15.0],
                &[9.0, 10.0, 11.0, 12.0, 13.0],
                &[10.5, 11.5, 12.5, 13.5, 14.5],
            )
            .unwrap();
        let twin_scale = chart
            .add_price_scale(0, "overlap", PriceScaleSide::Right, Some(0), true)
            .unwrap();
        chart.set_series_price_scale(twin, twin_scale);
        chart.set_price_scale_visible_range_for(0, PriceScaleTarget::Right, 0.0, 40.0);
        chart.set_price_scale_visible_range_for(0, twin_scale, 0.0, 40.0);

        let x = chart.time_scale.index_to_coordinate(4);
        let y = chart.series_price_to_coordinate(0, 14.5).unwrap();
        assert_eq!(chart.price_pan_target_at(0, x, y), Some(twin_scale));

        chart.set_selected_series(Some(0));
        assert_eq!(
            chart.price_pan_target_at(0, x, y),
            Some(PriceScaleTarget::Right)
        );
        assert!(chart.remove_series(0));
        assert_eq!(chart.price_pan_target_at(0, x, y), Some(twin_scale));
        chart.price_axis_start_scroll(0, twin_scale, y);
        assert!(chart.remove_series(twin));
        chart.price_axis_scroll_to(0, twin_scale, y + 20.0);
        chart.price_axis_end_scroll(0, twin_scale);
        assert_eq!(
            chart.price_pan_target_at(0, x, y),
            Some(PriceScaleTarget::Right)
        );
    }

    #[test]
    fn indicator_output_price_pan_uses_the_output_series_scale() {
        let mut chart = chart_with_data(600.0, 300.0);
        let sma = chart.add_sma(0, 2).expect("SMA output");
        let indicator_scale = chart
            .add_price_scale(0, "indicator-pan", PriceScaleSide::Left, Some(0), true)
            .unwrap();
        chart.set_series_price_scale(sma, indicator_scale);
        chart.set_price_scale_visible_range_for(0, indicator_scale, 0.0, 40.0);

        let x = chart.time_scale.index_to_coordinate(4);
        let y = chart.series_price_to_coordinate(sma, 14.0).unwrap();
        assert_eq!(chart.begin_price_pan_at(0, x, y), Some(indicator_scale));
        assert_eq!(
            chart.price_scale_auto_scale_for(0, indicator_scale),
            Some(false)
        );
    }

    #[test]
    fn named_axis_hit_testing_and_gestures_touch_only_the_selected_strip() {
        let mut chart = chart_with_data(600.0, 300.0);
        let comparison_series = chart.add_series(SeriesKind::Line);
        chart
            .set_series_data(
                comparison_series,
                &[0.0, 1.0, 2.0, 3.0],
                &[1_000.0, 1_010.0, 1_020.0, 1_030.0],
                &[1_000.0, 1_010.0, 1_020.0, 1_030.0],
                &[1_000.0, 1_010.0, 1_020.0, 1_030.0],
                &[1_000.0, 1_010.0, 1_020.0, 1_030.0],
            )
            .unwrap();
        let comparison = chart
            .add_price_scale(0, "comparison", PriceScaleSide::Right, Some(0), true)
            .unwrap();
        chart.set_series_price_scale(comparison_series, comparison);
        chart.set_price_scale_visible_range_for(0, PriceScaleTarget::Right, 5.0, 25.0);
        chart.set_price_scale_visible_range_for(0, comparison, 990.0, 1_040.0);
        chart.recompute_layout_with_measure(
            true,
            |text, _bold| text.len() as f64 * 7.0,
            |text, _bold| text.len() as f64 * 6.0,
        );

        let (_, comparison_x, comparison_width) = chart
            .price_scale_axis_geometry(0, comparison)
            .expect("named scale strip");
        let (_, right_x, right_width) = chart
            .price_scale_axis_geometry(0, PriceScaleTarget::Right)
            .expect("built-in strip");
        assert_eq!(comparison_x, chart.pane_left + chart.pane_w);
        assert_eq!(right_x, comparison_x + comparison_width);
        assert_eq!(
            chart.price_axis_target_at(0, comparison_x + comparison_width / 2.0 - chart.pane_left,),
            Some(comparison)
        );
        assert_eq!(
            chart.price_axis_target_at(0, right_x + right_width / 2.0 - chart.pane_left),
            Some(PriceScaleTarget::Right)
        );

        // Axis reset is local to the exact strip under the pointer.
        chart.set_price_scale_auto_scale_for(0, PriceScaleTarget::Right, false);
        chart.set_price_scale_auto_scale_for(0, comparison, false);
        chart.reset_price_scale(0, comparison);
        assert_eq!(chart.price_scale_auto_scale_for(0, comparison), Some(true));
        assert_eq!(
            chart.price_scale_auto_scale_for(0, PriceScaleTarget::Right),
            Some(false)
        );
        chart.set_price_scale_visible_range_for(0, PriceScaleTarget::Right, 5.0, 25.0);
        chart.set_price_scale_visible_range_for(0, comparison, 990.0, 1_040.0);

        let right_before = chart
            .price_scale_visible_range_for(0, PriceScaleTarget::Right)
            .unwrap();
        let named_before = chart.price_scale_visible_range_for(0, comparison).unwrap();
        chart.price_axis_start_scale(0, comparison, 250.0);
        chart.price_axis_scale_to(0, comparison, 200.0);
        chart.price_axis_end_scale(0, comparison);
        assert_ne!(
            chart.price_scale_visible_range_for(0, comparison),
            Some(named_before)
        );
        assert_eq!(
            chart.price_scale_visible_range_for(0, PriceScaleTarget::Right),
            Some(right_before)
        );

        chart.set_price_scale_visible_for(0, comparison, false);
        chart.recompute_layout_with_measure(
            true,
            |text, _bold| text.len() as f64 * 7.0,
            |text, _bold| text.len() as f64 * 6.0,
        );
        assert!(chart.price_scale_axis_geometry(0, comparison).is_none());
        assert!(chart.price_scale_visible_range_for(0, comparison).is_some());
        let (_, moved_right_x, _) = chart
            .price_scale_axis_geometry(0, PriceScaleTarget::Right)
            .expect("right strip remains");
        assert_eq!(moved_right_x, chart.pane_left + chart.pane_w);
    }

    #[test]
    fn wheel_pinch_and_scroll_use_reference_coefficients() {
        assert_eq!(wheel_zoom_scale(0.42), 0.42);
        assert_eq!(wheel_zoom_scale(-3.7), -1.0);
        assert_eq!(wheel_zoom_scale(0.0), 0.0);
        assert_eq!(pinch_zoom_scale(0.1), 0.5);
        assert_eq!(WHEEL_SCROLL_PX_PER_DELTA, -80.0);
        assert_eq!(PINCH_ZOOM_INTENSITY, 5.0);
    }

    #[test]
    fn scroll_animation_eases_to_the_target_and_clears() {
        let mut chart = chart_with_data(400.0, 300.0);
        chart.scroll_to_position(0.0);
        chart.start_scroll_animation(2.0, 200.0, 1000.0);
        assert!(chart.scroll_animation_active());
        // halfway: cubic ease-out(0.5) = 1 - 0.5^3 = 0.875 -> 0 + 2*0.875
        let mid = chart.scroll_animation_tick(1100.0).unwrap();
        assert!((mid - 1.75).abs() < 1e-9);
        assert!((chart.scroll_position() - 1.75).abs() < 1e-9);
        // completion applies the target and self-clears
        assert!(chart.scroll_animation_tick(1200.0).is_none());
        assert_eq!(chart.scroll_position(), 2.0);
        assert!(!chart.scroll_animation_active());
        assert!(chart.scroll_animation_tick(1300.0).is_none());
    }

    #[test]
    fn scroll_animation_cancel_and_zero_duration() {
        let mut chart = chart_with_data(400.0, 300.0);
        chart.start_scroll_animation(2.0, 200.0, 1000.0);
        chart.cancel_scroll_animation();
        assert!(!chart.scroll_animation_active());
        assert!(chart.scroll_animation_tick(1100.0).is_none());
        // zero duration jumps straight to the target on the first tick
        chart.start_scroll_animation(1.0, 0.0, 1000.0);
        assert!(chart.scroll_animation_tick(1000.0).is_none());
        assert_eq!(chart.scroll_position(), 1.0);
    }

    #[test]
    fn keyboard_scroll_is_fast_low_friction_and_stops_on_release() {
        let mut chart = chart_with_data(400.0, 300.0);
        chart.time_scale.set_bar_spacing(10.0);
        chart.scroll_to_position(0.0);

        chart.start_keyboard_scroll(10.0, 1000.0);
        let held_50 = chart.keyboard_scroll_tick(1050.0).unwrap();
        let held_100 = chart.keyboard_scroll_tick(1100.0).unwrap();
        let held_150 = chart.keyboard_scroll_tick(1150.0).unwrap();
        let held_200 = chart.keyboard_scroll_tick(1200.0).unwrap();
        let held_250 = chart.keyboard_scroll_tick(1250.0).unwrap();
        let held_300 = chart.keyboard_scroll_tick(1300.0).unwrap();
        let first_step = held_50;
        let second_step = held_100 - held_50;
        let third_step = held_150 - held_100;
        let late_step = held_300 - held_250;
        assert!(
            first_step > 1.5,
            "Ctrl+Arrow should launch with visible velocity instead of a sticky slow start"
        );
        assert!(
            second_step > first_step && third_step > second_step,
            "engine-timed impulses should build momentum smoothly while held"
        );
        assert!(held_200 > held_150 && held_250 > held_200 && held_300 > held_250);
        assert!(
            late_step > first_step,
            "low friction should retain more speed than launch"
        );

        chart.cancel_keyboard_scroll();
        assert!(!chart.keyboard_scroll_active());
        let stopped = chart.scroll_position();
        assert!(chart.keyboard_scroll_tick(2000.0).is_none());
        assert_eq!(chart.scroll_position(), stopped);
    }

    #[test]
    fn pane_index_at_y_uses_engine_pane_bounds() {
        let mut chart = chart_with_data(400.0, 300.0);
        chart.add_pane(true);
        chart.panes[1].stretch_factor = 0.5; // 2:1 split after the canonical separator slot
        chart.layout_panes(300.0);
        let first_h = chart.panes[0].height;
        assert_eq!(chart.pane_index_at_y(0.0), 0);
        assert_eq!(chart.pane_index_at_y(first_h - 1.0), 0);
        assert_eq!(chart.pane_index_at_y(first_h + 10.0), 1);
        assert_eq!(chart.pane_index_at_y(299.0), 1);
    }

    #[test]
    fn pane_index_at_y_resolves_separators_to_the_pane_above() {
        let mut chart = chart_with_data(400.0, 300.0);
        chart.add_pane(true);
        chart.add_pane(true);
        chart.layout_panes(300.0);
        assert_eq!(chart.panes.len(), 3);
        for index in 0..2 {
            let pane = &chart.panes[index];
            let separator = pane.top + pane.height + PANE_SEPARATOR / 2.0;
            assert_eq!(
                chart.pane_index_at_y(separator),
                index,
                "a separator below pane {index} belongs to that pane, not the last one"
            );
        }
        assert_eq!(chart.pane_index_at_y(-10.0), 0);
        assert_eq!(chart.pane_index_at_y(10_000.0), 2);
    }
}
