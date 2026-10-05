//! Host-neutral chart input controller.
//!
//! Interactive hosts translate platform events into [`PointerInput`], [`WheelSample`] and
//! [`ChartKey`] values and forward them to the `input_*` methods below. The engine owns the
//! complete interaction policy: which chart object a press belongs to, the shared 5 px drag
//! threshold, drag lifecycles, kinetic coasting, click selection, text-edit activation, drawing
//! placement, keyboard bindings, wheel routing, hover promotion, trading-tooltip dwell, and the
//! [`ChartCursor`] the platform should present.
//!
//! Hosts keep only genuine platform duties: event translation, pointer capture, applying the
//! cursor, frame and timer scheduling, menus, and clipboard. A new interaction therefore lands
//! here once and every native host inherits it without wiring.

use super::*;

/// Manhattan distance before a press becomes a drag (reference CancelClickManhattanDistance).
/// Hosts that arbitrate page scrolling before the engine sees a gesture wait for this distance.
pub const CLICK_SLOP_MANHATTAN: f64 = 5.0;
const DOUBLE_CLICK_WINDOW_MS: f64 = 500.0;
/// A stationary pane touch enters crosshair inspection after this host-clock interval.
pub const TOUCH_LONG_PRESS_MS: f64 = 240.0;
/// CSS px hit tolerance around a pane boundary.
pub const PANE_SEPARATOR_HIT: f64 = 4.0;
/// Hover dwell before a trading control reveals its action tooltip. The engine owns no clock, so
/// the host wakes it through [`ChartEngine::input_tick`] once this deadline passes.
pub const TRADING_TOOLTIP_DWELL_MS: f64 = 450.0;
/// Host requests retained between drains; the oldest is dropped when a host stops draining.
const MAX_PENDING_INPUT_EVENTS: usize = 32;
/// Keyboard `+`/`-` zoom step. It anchors at the plot center, or keeps the newest bar in place
/// while `right_bar_stays_on_scroll` (the default) pins the right edge.
const KEYBOARD_ZOOM_STEP: f64 = 0.5;
/// Share of the plot width one PageUp/PageDown scrolls.
const KEYBOARD_PAGE_FRACTION: f64 = 0.8;
/// Half-span multipliers for one focused price-axis zoom step (5% in, 5% out).
const PRICE_AXIS_KEY_ZOOM_IN: f64 = 0.475;
const PRICE_AXIS_KEY_ZOOM_OUT: f64 = 0.525;
/// CSS px one focused separator key moves the divider.
const SEPARATOR_KEY_STEP: f64 = 10.0;

/// Host-configurable interaction switches (the reference `handleScroll`/`handleScale` family).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InteractionOptions {
    /// Primary drag on empty pane space pans the time scale (and a manual price scale).
    pub pan: bool,
    /// One-finger touch drag pans after the host's page-scroll arbitration.
    pub touch_pan: bool,
    pub wheel_scroll: bool,
    pub wheel_zoom: bool,
    pub pinch_zoom: bool,
    pub wheel_behavior: WheelBehavior,
    /// The wheel over a price axis zooms that scale in `Auto` mode too, not only in `Zoom` mode.
    pub price_axis_wheel_zoom: bool,
    pub axis_double_click_reset_time: bool,
    pub axis_double_click_reset_price: bool,
    pub axis_scale_price: bool,
    pub axis_scale_time: bool,
    /// A released mouse pan coasts with the engine's kinetic model.
    pub kinetic_mouse: bool,
    /// A released touch pan coasts with the same bounded engine kinetic model.
    pub kinetic_touch: bool,
    /// Release the touch crosshair on the same touch end instead of the next stationary tap.
    pub touch_tracking_exit_on_end: bool,
    /// Suppress kinetic coasting and held-key animation when the host requests reduced motion.
    pub reduced_motion: bool,
    pub panes_resize: bool,
}

impl Default for InteractionOptions {
    fn default() -> Self {
        Self {
            pan: true,
            touch_pan: true,
            wheel_scroll: true,
            wheel_zoom: true,
            pinch_zoom: true,
            wheel_behavior: WheelBehavior::Auto,
            price_axis_wheel_zoom: false,
            axis_double_click_reset_time: true,
            axis_double_click_reset_price: true,
            axis_scale_price: true,
            axis_scale_time: true,
            kinetic_mouse: false,
            kinetic_touch: true,
            touch_tracking_exit_on_end: false,
            reduced_motion: false,
            panes_resize: true,
        }
    }
}

impl InteractionOptions {
    fn pan_for(self, device: InputDevice) -> bool {
        if device == InputDevice::Touch {
            self.touch_pan
        } else {
            self.pan
        }
    }

    fn kinetic_for(self, device: InputDevice) -> bool {
        if device == InputDevice::Touch {
            self.kinetic_touch
        } else {
            self.kinetic_mouse
        }
    }
}

/// Semantic pointer feedback. Each host maps it to one platform cursor in one place.
#[repr(u8)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ChartCursor {
    #[default]
    Crosshair,
    Default,
    Pointer,
    Text,
    /// Hovering an object that moves freely.
    Move,
    /// Dragging an object or panning the chart.
    Grabbing,
    /// Hovering a line that drags only vertically (trading orders).
    VerticalGrab,
    VerticalGrabbing,
    /// `ns-resize`.
    ResizeVertical,
    /// `ew-resize`.
    ResizeHorizontal,
    /// `nwse-resize`: up-left to down-right.
    ResizeNwse,
    /// `nesw-resize`: up-right to down-left.
    ResizeNesw,
    /// Pane separator (`row-resize`).
    ResizeRow,
    /// A host primitive supplied a platform cursor; the engine chose its priority.
    HostPrimitive,
}

impl ChartCursor {
    /// Map an engine hit-test cursor name to its typed form.
    fn from_hit_name(name: &str) -> Self {
        match name {
            "text" => Self::Text,
            "pointer" => Self::Pointer,
            "move" => Self::Move,
            "ns-resize" => Self::ResizeVertical,
            "ew-resize" => Self::ResizeHorizontal,
            "nwse-resize" => Self::ResizeNwse,
            "nesw-resize" => Self::ResizeNesw,
            _ => Self::Crosshair,
        }
    }
}

/// One primary-button pointer sample in pane coordinates (x from the plot's left edge, y from
/// the chart's top), the same space every engine query uses.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PointerInput {
    pub id: u32,
    pub device: InputDevice,
    pub x: f64,
    pub y: f64,
    pub modifiers: InputModifiers,
    /// Host monotonic clock in milliseconds.
    pub timestamp_ms: f64,
}

/// Keys the chart binds. Hosts translate platform key names; the bindings live here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChartKey {
    ArrowLeft,
    ArrowRight,
    /// Bound only on focused targets ([`ChartEngine::input_target_key_down`]).
    ArrowUp,
    /// Bound only on focused targets ([`ChartEngine::input_target_key_down`]).
    ArrowDown,
    /// Cycles drawing anchors during a focused drawing edit; never consumed chart-wide.
    Tab,
    PageUp,
    PageDown,
    ZoomIn,
    ZoomOut,
    Home,
    End,
    Enter,
    /// F2: edit the selected drawing's text.
    EditText,
    Backspace,
    Delete,
    Escape,
    Undo,
    Redo,
}

/// The chart region under a pane-space point.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChartRegion {
    Pane,
    PriceAxis {
        pane: usize,
        target: PriceScaleTarget,
    },
    TimeAxis,
    /// The divider below pane `index`.
    Separator(usize),
}

/// A keyboard focus target inside the chart. Accessibility hosts expose these as focusable
/// proxies; the bindings for each live in [`ChartEngine::input_target_key_down`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChartFocusTarget {
    PriceAxis {
        pane: usize,
        target: PriceScaleTarget,
    },
    TimeAxis,
    /// The divider below pane `index`.
    Separator(usize),
    Drawing(DrawingId),
}

/// Paint layer of a host primitive hit, lowest first (reference `zOrder`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum HostPrimitiveLayer {
    Bottom,
    Normal,
    Top,
}

/// The best host-owned primitive hit at a hover point. Hosts gather it from their primitive
/// objects; the engine arbitrates it against drawings and series.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HostPrimitiveHit {
    /// The owning series of a series primitive; `None` for a pane primitive.
    pub series: Option<SeriesId>,
    pub layer: HostPrimitiveLayer,
    /// The primitive supplied its own platform cursor.
    pub cursor: bool,
}

/// The single winner of hover arbitration at the controller's hover point.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum ChartHover {
    #[default]
    None,
    HostPrimitive(HostPrimitiveHit),
    Drawing {
        id: DrawingId,
        /// Engine hit-test cursor name of the hovered part.
        cursor: &'static str,
    },
    Series(SeriesId),
    /// An engine volume-profile indicator (a series primitive, not an output series).
    VolumeProfile(NativePrimitiveId),
    /// A general-series item; read it through [`ChartEngine::general_hovered_hit`].
    General,
}

/// A secondary click resolved by the engine. Menus, clipboard, and order UI stay host-owned.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChartContextMenu {
    pub x: f64,
    pub y: f64,
    pub region: ChartRegion,
    /// Pane context for a pane click; `None` over chrome or outside the plot.
    pub context: Option<ChartContext>,
}

/// Work the host performs on the chart's behalf after input.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ChartInputEvent {
    ContextMenu(ChartContextMenu),
    /// Notify pane click subscribers after the engine has applied the click.
    Click {
        x: f64,
        y: f64,
    },
    /// Notify pane double-click subscribers after the engine has applied the action.
    DoubleClick {
        x: f64,
        y: f64,
    },
    /// Open the platform text-entry surface for an engine-owned editing session.
    TextEditorOpened(DrawingId),
    /// The host must notify crosshair subscribers that a keyboard cancel cleared hover.
    CrosshairLeft,
    /// A drawing tool committed a drawing. One-shot tools are already disarmed.
    DrawingCreated(DrawingId),
    /// Delete was pressed on a selected series the engine does not own (a host-installed series or
    /// an external study). The host decides how its own objects are removed.
    RemoveSeries(SeriesId),
    /// Delta-tooltip comparison state changed; the host notifies its range subscribers.
    DeltaTooltipChanged,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum PressMode {
    /// Pane press that becomes a pan at the drag threshold.
    Pane {
        price_pan: Option<(usize, PriceScaleTarget)>,
        panning: bool,
    },
    TimeAxis,
    PriceAxis {
        pane: usize,
        target: PriceScaleTarget,
    },
    Separator {
        index: usize,
        grab_offset_y: f64,
    },
    Measure,
    Trading {
        dragging: bool,
    },
    Alert,
    DrawingCreation {
        capture: bool,
        committed_on_press: bool,
    },
    DrawingDrag,
    TouchTracking,
    DeltaTooltip,
    /// A press inside the label being edited; the typing session keeps it.
    TextEditor,
    /// A press with no gesture: inert chrome or an already-handled double-click.
    Inert,
}

impl PressMode {
    const fn target(self) -> InputTarget {
        match self {
            Self::TimeAxis => InputTarget::TimeAxis,
            Self::PriceAxis { .. } => InputTarget::PriceAxis,
            Self::Separator { .. } => InputTarget::Separator,
            Self::Trading { .. } => InputTarget::Trading,
            Self::Alert => InputTarget::Alert,
            Self::Measure | Self::DrawingCreation { .. } | Self::DrawingDrag | Self::TextEditor => {
                InputTarget::Drawing
            }
            Self::Pane { .. } | Self::TouchTracking | Self::DeltaTooltip | Self::Inert => {
                InputTarget::Pane
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Press {
    id: u32,
    mode: PressMode,
    double_candidate: bool,
    start: (f64, f64),
    moved: bool,
    /// The drawing selected when the press began: a selected text drawing opens its editor on
    /// the next click (two-step select-then-type).
    text_press_selected: Option<DrawingId>,
    /// A trading control or the alert chip took this press or the click before it in the same
    /// double-click pair, so the pair's double-click never reaches the drawing underneath, even
    /// when the host has already removed the control (a cancelled order's line is gone by the
    /// second click).
    control_pair: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct KeyboardPan {
    key: ChartKey,
    delta: f64,
}

/// A keyboard drawing edit: every nudge applies live, and the whole edit commits as one undo entry
/// or cancels back to `before`.
#[derive(Clone, Debug)]
struct DrawingEditSession {
    before: Drawing,
    anchor: Option<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct TouchTracking {
    anchor: (f64, f64),
    base_anchor: (f64, f64),
    start: (f64, f64),
    active_id: Option<u32>,
    exit_on_release: bool,
}

/// Runtime-only input state. Never persisted; [`ChartEngine::input_cancel`] resets gestures.
#[derive(Default)]
pub(crate) struct InputController {
    options: InteractionOptions,
    resolver: GestureResolver,
    press: Option<Press>,
    last_click: Option<(f64, f64, f64)>,
    /// The last completed click was a trading-control or alert-chip press (see
    /// [`Press::control_pair`]); the next press reads it when it pairs into a double-click.
    control_click: bool,
    pointer: Option<(f64, f64)>,
    modifiers: InputModifiers,
    cursor: ChartCursor,
    host_primitive_cursor: bool,
    kinetic_active: bool,
    keyboard_pan: Option<KeyboardPan>,
    /// Newest captured freehand sample since the last frame. Native platforms may deliver motion
    /// at device cadence (Wayland: per HID report); the stroke samples display cadence instead.
    pending_capture: Option<(f64, f64, DrawingModifiers)>,
    tooltip_deadline_ms: Option<f64>,
    touch_longpress_deadline_ms: Option<f64>,
    touch_tracking: Option<TouchTracking>,
    hover: ChartHover,
    drawing_edit: Option<DrawingEditSession>,
    events: VecDeque<ChartInputEvent>,
    /// Input changed chart state since the last prepared frame.
    frame_dirty: bool,
    /// Input changed pane geometry (a separator drag) since the last prepared frame.
    layout_dirty: bool,
}

impl InputController {
    /// Consume `(frame, layout)` invalidation recorded by input since the last prepared frame.
    pub(crate) fn take_frame_invalidation(&mut self) -> (bool, bool) {
        (
            std::mem::take(&mut self.frame_dirty),
            std::mem::take(&mut self.layout_dirty),
        )
    }
}

const fn drawing_modifiers(modifiers: InputModifiers) -> DrawingModifiers {
    DrawingModifiers {
        magnet: modifiers.control || modifiers.meta,
        straighten: modifiers.shift,
    }
}

impl ChartEngine {
    // --- configuration and host-facing state ---

    pub fn interaction_options(&self) -> InteractionOptions {
        self.input.options
    }

    pub fn set_interaction_options(&mut self, options: InteractionOptions) {
        if options.reduced_motion && !self.input.options.reduced_motion {
            self.stop_input_motion();
        }
        self.input.options = options;
    }

    /// The cursor the platform should present for the current pointer state.
    pub fn input_cursor(&self) -> ChartCursor {
        self.input.cursor
    }

    /// Release every hover promotion without moving the crosshair.
    pub fn clear_hover(&mut self) {
        self.input.host_primitive_cursor = false;
        self.input.hover = ChartHover::None;
        self.set_hovered_series(None);
        self.set_hovered_volume_profile(None);
        self.set_hovered_text(None);
        self.set_hovered_drawing(None);
        self.clear_general_hover();
        self.refresh_input_cursor();
    }

    /// Whether a held pane touch may yield to the browser page's scroll direction. Other chart
    /// targets and an active tracking gesture retain the touch regardless of page policy.
    pub fn input_touch_page_scroll_candidate(&self, id: u32) -> bool {
        self.input
            .press
            .is_some_and(|press| press.id == id && matches!(press.mode, PressMode::Pane { .. }))
    }

    /// Drain host requests produced by input since the last drain.
    pub fn take_input_events(&mut self) -> Vec<ChartInputEvent> {
        self.input.events.drain(..).collect()
    }

    /// Whether the next prepared frame would rebuild: input, layout, axis, or any layer changed
    /// since the last prepared frame. Hosts render after input only while this holds.
    pub fn frame_pending(&self) -> bool {
        self.input.frame_dirty
            || self.input.layout_dirty
            || self.input.pending_capture.is_some()
            || self.frame_invalidated_since_prepare()
            || self.frame_requires_layout()
            || self.frame_requires_axis()
    }

    /// The current hover winner at the controller's hover point.
    pub fn input_hover(&self) -> ChartHover {
        self.input.hover
    }

    /// Re-run hover arbitration at `(x, y)` with the host's best primitive hit there. Hosts with
    /// their own primitive objects call this after input when a primitive hit exists; the engine
    /// already resolved every built-in candidate for the same point.
    pub fn resolve_pointer_hover(
        &mut self,
        x: f64,
        y: f64,
        primitive: Option<HostPrimitiveHit>,
    ) -> ChartHover {
        let hover = self.arbitrate_hover(x, y, primitive);
        self.input.hover = hover;
        self.input.host_primitive_cursor =
            matches!(hover, ChartHover::HostPrimitive(hit) if hit.cursor);
        self.refresh_input_cursor();
        hover
    }

    /// The open keyboard drawing edit: the drawing and its focused anchor (`None` moves the whole
    /// drawing).
    pub fn drawing_edit_session(&self) -> Option<(DrawingId, Option<usize>)> {
        self.input
            .drawing_edit
            .as_ref()
            .map(|session| (session.before.id, session.anchor))
    }

    /// Whether an input-owned animation (kinetic coast, held keyboard pan, animated scroll) needs
    /// another frame. Hosts request animation frames only while this holds.
    pub fn input_animating(&self) -> bool {
        self.input.kinetic_active || self.keyboard_scroll_active() || self.scroll_animation_active()
    }

    /// Earliest host-clock time at which [`Self::input_tick`] has deferred work, if any.
    pub fn input_wake_deadline_ms(&self) -> Option<f64> {
        match (
            self.input.tooltip_deadline_ms,
            self.input.touch_longpress_deadline_ms,
        ) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (Some(a), None) | (None, Some(a)) => Some(a),
            (None, None) => None,
        }
    }

    /// Advance time-driven input state: animations and the trading-tooltip dwell. Hosts call this
    /// once per prepared frame and when a wake deadline passes. Returns whether state changed.
    pub fn input_tick(&mut self, now_ms: f64) -> bool {
        let mut changed = false;
        if self.input.kinetic_active {
            if self.kinetic_finished(now_ms) {
                self.time_scale_end_scroll();
                self.kinetic_stop();
                self.input.kinetic_active = false;
            } else if let Some(position) = self.kinetic_position(now_ms) {
                self.scroll_to_position(position);
            }
            changed = true;
        }
        if self.keyboard_scroll_active() {
            self.keyboard_scroll_tick(now_ms);
            changed = true;
        }
        if self.scroll_animation_active() {
            self.scroll_animation_tick(now_ms);
            changed = true;
        }
        if self
            .input
            .tooltip_deadline_ms
            .is_some_and(|deadline| now_ms >= deadline)
        {
            self.input.tooltip_deadline_ms = None;
            changed |= self.arm_trading_tooltip();
        }
        if self
            .input
            .touch_longpress_deadline_ms
            .is_some_and(|deadline| now_ms >= deadline)
        {
            self.input.touch_longpress_deadline_ms = None;
            changed |= self.arm_touch_tracking();
        }
        if changed {
            self.input.frame_dirty = true;
        }
        changed
    }

    fn arm_touch_tracking(&mut self) -> bool {
        let Some(press) = self.input.press.as_mut() else {
            return false;
        };
        if !matches!(press.mode, PressMode::Pane { panning: false, .. }) || press.moved {
            return false;
        }
        let update = self.input.resolver.long_press(press.id);
        if update.kind != GestureUpdateKind::LongPress {
            return false;
        }
        let anchor = press.start;
        press.mode = PressMode::TouchTracking;
        press.moved = true;
        self.input.last_click = None;
        self.input.control_click = false;
        self.input.touch_tracking = Some(TouchTracking {
            anchor,
            base_anchor: anchor,
            start: anchor,
            active_id: Some(press.id),
            exit_on_release: false,
        });
        self.refresh_pointer_hover(anchor.0, anchor.1, 0.0);
        true
    }

    /// Forward the newest coalesced captured-drawing sample. Hosts that receive raw device-rate
    /// motion call this once per prepared frame; hosts that already coalesce never need it.
    pub fn flush_coalesced_input(&mut self) -> bool {
        let Some((x, y, modifiers)) = self.input.pending_capture.take() else {
            return false;
        };
        let changed = self
            .drawing_tool_pointer_move(x, y, modifiers, true)
            .changed;
        self.input.frame_dirty |= changed;
        changed
    }

    /// Resolve the chart region under a pane-space point.
    pub fn region_at(&self, x: f64, y: f64) -> ChartRegion {
        self.region_at_with_profile(x, y, HitProfile::PRECISION)
    }

    fn region_at_with_profile(&self, x: f64, y: f64, profile: HitProfile) -> ChartRegion {
        if let Some(index) = self.pane_separator_at(y, profile.separator_tolerance) {
            return ChartRegion::Separator(index);
        }
        let pane = self.pane_index_at_y(y);
        if let Some(target) = self.price_axis_target_at(pane, x) {
            return ChartRegion::PriceAxis { pane, target };
        }
        if y > self.pane_h {
            return ChartRegion::TimeAxis;
        }
        ChartRegion::Pane
    }

    /// Scroll to the configured real-time edge, keeping the chart's configured right offset.
    pub fn scroll_to_latest(&mut self) {
        let offset = self.time_scale.options().right_offset;
        self.scroll_to_position(offset);
    }

    /// Whether the newest bar sits at the configured real-time edge.
    pub fn is_at_latest(&self) -> bool {
        (self.scroll_position() - self.time_scale.options().right_offset).abs() < f64::EPSILON
    }

    // --- pointer ---

    /// Primary-button press. `click_count` is the platform's multi-click count.
    pub fn input_pointer_down(&mut self, input: PointerInput, click_count: u32) {
        // A pinch has exactly two members. After its primary leaves, ignore new touches until
        // the remaining member lifts too; it cannot silently become a fresh press.
        if input.device == InputDevice::Touch
            && self.input.press.is_none()
            && self.input.resolver.active_pointer_count() > 0
        {
            return;
        }
        if input.device == InputDevice::Touch
            && self.input.press.is_some_and(|press| press.id != input.id)
        {
            let update = self
                .input
                .resolver
                .pointer_down(Self::mouse_sample(input, InputTarget::Pane));
            if update.kind == GestureUpdateKind::PinchStarted {
                self.abandon_press_state();
                self.input.touch_longpress_deadline_ms = None;
                self.input.last_click = None;
                self.input.control_click = false;
                self.input.pointer = Some((update.x, update.y));
                self.input.frame_dirty = true;
                self.clear_pointer_hover();
                self.refresh_input_cursor();
            }
            return;
        }
        if self.input.press.is_some_and(|press| press.id != input.id) {
            return;
        }
        let (x, y) = (input.x, input.y);
        if input.device == InputDevice::Touch {
            if let Some(mut tracking) = self
                .input
                .touch_tracking
                .filter(|state| state.active_id.is_none())
            {
                self.stop_input_motion();
                self.input
                    .resolver
                    .pointer_down(Self::mouse_sample(input, InputTarget::Pane));
                self.input.resolver.long_press(input.id);
                tracking.base_anchor = tracking.anchor;
                tracking.start = (x, y);
                tracking.active_id = Some(input.id);
                tracking.exit_on_release = true;
                self.input.touch_tracking = Some(tracking);
                self.input.press = Some(Press {
                    id: input.id,
                    mode: PressMode::TouchTracking,
                    double_candidate: false,
                    start: (x, y),
                    moved: true,
                    text_press_selected: None,
                    control_pair: false,
                });
                self.input.pointer = Some((x, y));
                self.input.last_click = None;
                self.input.control_click = false;
                self.input.frame_dirty = true;
                self.refresh_input_cursor();
                return;
            }
        } else {
            self.input.touch_tracking = None;
        }
        self.input.pointer = Some((x, y));
        self.input.modifiers = input.modifiers;
        self.input.tooltip_deadline_ms = None;
        self.input.frame_dirty = true;
        self.end_press_without_commit();
        self.stop_input_motion();
        self.commit_drawing_edit_session();

        let text_press_selected = self.selected_drawing();
        let control_click = self.input.control_click;
        let press = |mode, double_candidate| Press {
            id: input.id,
            mode,
            double_candidate,
            start: (x, y),
            moved: false,
            text_press_selected,
            control_pair: double_candidate
                && (control_click || matches!(mode, PressMode::Trading { .. } | PressMode::Alert)),
        };
        if self.drawing_text_edit().is_some() {
            if self.point_on_edited_text(x, y) {
                self.drawing_text_edit_caret_at(x, y);
                self.input.press = Some(press(PressMode::TextEditor, false));
                self.refresh_input_cursor();
                return;
            }
            self.commit_drawing_text_edit();
        }
        self.apply_input_magnet();

        let region = self.region_at_with_profile(x, y, HitProfile::for_device(input.device));
        let sequence_or_idle =
            self.active_drawing_tool().is_none() || self.drawing_tool_sequence_active();
        let repeated = self
            .input
            .last_click
            .is_some_and(|(last_x, last_y, last_ms)| {
                input.timestamp_ms > 0.0
                    && last_ms > 0.0
                    && (0.0..=DOUBLE_CLICK_WINDOW_MS).contains(&(input.timestamp_ms - last_ms))
                    && (x - last_x).abs() + (y - last_y).abs() < CLICK_SLOP_MANHATTAN
            });
        if click_count >= 2 || repeated {
            self.input.last_click = None;
        }
        let mode = self.begin_press(region, input);
        self.input
            .resolver
            .pointer_down(Self::mouse_sample(input, mode.target()));
        self.input.press = Some(press(
            mode,
            (click_count >= 2 || repeated) && sequence_or_idle,
        ));
        self.input.touch_longpress_deadline_ms = (input.device == InputDevice::Touch
            && matches!(mode, PressMode::Pane { .. })
            && input.timestamp_ms.is_finite())
        .then_some(input.timestamp_ms + TOUCH_LONG_PRESS_MS);
        if matches!(mode, PressMode::Separator { .. }) {
            self.clear_pointer_hover();
        } else {
            self.refresh_pointer_hover(x, y, input.timestamp_ms);
        }
        self.refresh_input_cursor();
    }

    /// Pointer motion. `primary_pressed` reports whether the primary button is held.
    pub fn input_pointer_move(&mut self, input: PointerInput, primary_pressed: bool) {
        if input.device == InputDevice::Touch
            && self.input.resolver.state() == GestureState::Pinching
        {
            if !self.input.resolver.contains_pointer(input.id) {
                return;
            }
            let update = self
                .input
                .resolver
                .pointer_move(Self::mouse_sample(input, InputTarget::Pane));
            if update.kind == GestureUpdateKind::PinchMoved {
                self.input.pointer = Some((update.x, update.y));
                self.input_pinch(update.x, update.y, update.scale_delta, input.timestamp_ms);
            }
            self.update_touch_delta_tooltip();
            return;
        }
        if self.input.press.is_some_and(|press| press.id != input.id) {
            return;
        }
        let (x, y) = (input.x, input.y);
        self.input.pointer = Some((x, y));
        self.input.modifiers = input.modifiers;
        // Plain hover changes only state whose setters invalidate the frame themselves, so an
        // identical hover sample leaves the prepared frame valid. Gestures and armed tools may
        // mutate geometry through paths without that bookkeeping and always rebuild.
        if self.input.press.is_some()
            || self.measure_active()
            || self.active_drawing_tool().is_some()
        {
            self.input.frame_dirty = true;
        }
        // Motion with the button already up means the release happened where the host could not
        // see it. Like lost pointer capture, that abandons the gesture; the move is then a hover.
        if !primary_pressed && self.input.press.is_some() {
            self.abandon_press();
        }
        let target = self
            .input
            .press
            .map_or(InputTarget::Pane, |press| press.mode.target());
        let update = self
            .input
            .resolver
            .pointer_move(Self::mouse_sample(input, target));
        if input.device == InputDevice::Touch {
            self.update_touch_delta_tooltip();
        } else if self.delta_tooltip_mouse_move(x) {
            self.note_delta_tooltip_changed();
        }
        self.apply_input_magnet();
        let modifiers = drawing_modifiers(input.modifiers);
        if let Some(press) = self.input.press.as_mut() {
            press.moved |=
                (x - press.start.0).abs() + (y - press.start.1).abs() >= CLICK_SLOP_MANHATTAN;
            if input.device == InputDevice::Touch && press.moved {
                self.input.touch_longpress_deadline_ms = None;
            }
        }
        if matches!(
            self.input.press.map(|press| press.mode),
            Some(PressMode::TouchTracking)
        ) {
            if let Some(mut tracking) = self
                .input
                .touch_tracking
                .filter(|state| state.active_id == Some(input.id))
            {
                if (x, y) != tracking.start {
                    tracking.exit_on_release = false;
                }
                tracking.anchor = (
                    (tracking.base_anchor.0 + x - tracking.start.0).clamp(0.0, self.pane_w),
                    (tracking.base_anchor.1 + y - tracking.start.1).clamp(0.0, self.pane_h),
                );
                self.input.touch_tracking = Some(tracking);
                self.refresh_pointer_hover(
                    tracking.anchor.0,
                    tracking.anchor.1,
                    input.timestamp_ms,
                );
                self.refresh_input_cursor();
            }
            return;
        }
        // A live measure follows with or without a held button: press-drag-release and
        // click-move-click both measure.
        if self.measure_active() {
            self.measure_pointer_move(x, y, modifiers);
        }
        let dragging = matches!(
            update.kind,
            GestureUpdateKind::DragStarted | GestureUpdateKind::DragMoved
        );
        match self.input.press.map(|press| press.mode) {
            // Axis sessions snapshot on press but mutate only once the shared threshold is crossed.
            Some(PressMode::TimeAxis) => {
                if dragging {
                    self.time_axis_scale_to(x);
                }
                self.refresh_input_cursor();
                return;
            }
            Some(PressMode::PriceAxis { pane, target }) => {
                if dragging {
                    self.price_axis_scale_to(pane, target, y);
                }
                self.refresh_input_cursor();
                return;
            }
            Some(PressMode::Separator {
                index,
                grab_offset_y,
            }) => {
                if dragging {
                    if let Some(pane_below) = self.panes.get(index + 1) {
                        let delta = y - grab_offset_y - pane_below.top;
                        self.drag_pane_separator(index, delta);
                        self.input.layout_dirty = true;
                    }
                }
                // The separator is chrome: the crosshair hides during the resize drag.
                self.clear_pointer_hover();
                self.refresh_input_cursor();
                return;
            }
            Some(PressMode::Trading { dragging: true }) => {
                self.trading_drag_to(y);
            }
            Some(PressMode::DrawingCreation { capture: true, .. }) => {
                if primary_pressed {
                    self.input.pending_capture = Some((x, y, modifiers));
                }
            }
            Some(PressMode::DrawingCreation { capture: false, .. }) => {
                self.drawing_tool_pointer_move(x, y, modifiers, primary_pressed);
            }
            Some(PressMode::DrawingDrag) if primary_pressed => {
                self.drawing_drag_to(x, y, modifiers);
            }
            Some(PressMode::Pane { price_pan, panning }) if primary_pressed => {
                self.continue_pan(input, update.kind, price_pan, panning);
            }
            None => {
                if self.active_drawing_tool().is_some() {
                    self.drawing_tool_pointer_move(x, y, modifiers, false);
                }
                let hover = self
                    .input
                    .options
                    .panes_resize
                    .then(|| self.pane_separator_at(y, PANE_SEPARATOR_HIT))
                    .flatten();
                if self.separator_hover() != hover {
                    self.set_separator_hover(hover);
                }
            }
            _ => {}
        }
        self.refresh_pointer_hover(x, y, input.timestamp_ms);
        self.refresh_input_cursor();
    }

    /// Primary-button release.
    pub fn input_pointer_up(&mut self, input: PointerInput) {
        if input.device == InputDevice::Touch
            && self.input.resolver.state() == GestureState::Pinching
        {
            if !self.input.resolver.contains_pointer(input.id) {
                return;
            }
            let update = self
                .input
                .resolver
                .pointer_up(Self::mouse_sample(input, InputTarget::Pane));
            self.input.frame_dirty = true;
            if update.kind == GestureUpdateKind::RebasedSinglePointer {
                let rebased = PointerInput {
                    id: update.pointer_id,
                    x: update.x,
                    y: update.y,
                    ..input
                };
                let price_pan = self.pane_index_at_y(update.y);
                let price_pan = self
                    .price_pan_target_at(price_pan, update.x, update.y)
                    .map(|target| (price_pan, target));
                self.input.press = Some(Press {
                    id: update.pointer_id,
                    mode: PressMode::Pane {
                        price_pan,
                        panning: false,
                    },
                    double_candidate: false,
                    start: (update.x, update.y),
                    moved: true,
                    text_press_selected: None,
                    control_pair: false,
                });
                self.input.pointer = Some((rebased.x, rebased.y));
                self.refresh_pointer_hover(rebased.x, rebased.y, input.timestamp_ms);
                self.refresh_input_cursor();
            } else {
                self.input.pointer = None;
                self.clear_pointer_hover();
                self.refresh_input_cursor();
            }
            return;
        }
        if self.input.press.is_some_and(|press| press.id != input.id) {
            return;
        }
        let (x, y) = (input.x, input.y);
        if input.device == InputDevice::Touch {
            self.input.touch_longpress_deadline_ms = None;
        }
        self.input.pointer = Some((x, y));
        self.input.modifiers = input.modifiers;
        self.input.frame_dirty = true;
        if self.delta_tooltip_mouse_up() {
            self.note_delta_tooltip_changed();
        }
        self.input
            .resolver
            .pointer_up(Self::mouse_sample(input, InputTarget::Pane));
        self.clear_trading_pressed();
        let Some(press) = self.input.press.take() else {
            if input.device == InputDevice::Touch && self.input.touch_tracking.is_none() {
                self.input.pointer = None;
                self.clear_pointer_hover();
            } else {
                self.refresh_pointer_hover(x, y, input.timestamp_ms);
            }
            self.refresh_input_cursor();
            return;
        };
        if press.mode == PressMode::TouchTracking {
            if let Some(mut tracking) = self
                .input
                .touch_tracking
                .filter(|state| state.active_id == Some(input.id))
            {
                self.input.pointer = None;
                if self.input.options.touch_tracking_exit_on_end || tracking.exit_on_release {
                    self.input.touch_tracking = None;
                    self.clear_pointer_hover();
                } else {
                    tracking.active_id = None;
                    self.input.touch_tracking = Some(tracking);
                    self.refresh_pointer_hover(
                        tracking.anchor.0,
                        tracking.anchor.1,
                        input.timestamp_ms,
                    );
                }
                self.refresh_input_cursor();
            }
            return;
        }
        let moved = press.moved
            || (x - press.start.0).abs() + (y - press.start.1).abs() >= CLICK_SLOP_MANHATTAN;
        if press.double_candidate && !moved {
            // The second press opened a gesture session like any press; it closes here without
            // the release action, which the double-click replaces.
            match press.mode {
                PressMode::DrawingDrag => self.drawing_drag_end(),
                PressMode::TimeAxis => self.time_axis_end_scale(),
                PressMode::PriceAxis { pane, target } => {
                    self.price_axis_end_scale(pane, target);
                }
                PressMode::Measure => {
                    self.measure_pointer_up(x, y, drawing_modifiers(input.modifiers));
                }
                PressMode::Trading { dragging: true } => {
                    self.cancel_trading_drag();
                }
                PressMode::Pane { price_pan, panning } => {
                    if let Some((pane, target)) = price_pan {
                        self.price_axis_end_scroll(pane, target);
                    }
                    if panning {
                        self.kinetic_stop();
                        self.time_scale_end_scroll();
                    }
                }
                PressMode::DrawingCreation { capture: true, .. } => self.cancel_drawing_creation(),
                PressMode::DrawingCreation { capture: false, .. }
                | PressMode::Trading { dragging: false }
                | PressMode::Alert
                | PressMode::Separator { .. }
                | PressMode::TouchTracking
                | PressMode::DeltaTooltip
                | PressMode::TextEditor
                | PressMode::Inert => {}
            }
            self.input.last_click = None;
            self.input.control_click = false;
            self.input_double_click(
                self.region_at(x, y),
                input,
                press.text_press_selected,
                press.control_pair,
            );
            self.apply_input_magnet();
            if input.device == InputDevice::Touch {
                self.input.pointer = None;
                self.clear_pointer_hover();
            } else {
                self.refresh_pointer_hover(x, y, input.timestamp_ms);
            }
            self.refresh_input_cursor();
            return;
        }
        self.input.last_click = (!moved
            && input.timestamp_ms > 0.0
            && input.timestamp_ms.is_finite())
        .then_some((x, y, input.timestamp_ms));
        self.input.control_click =
            !moved && matches!(press.mode, PressMode::Trading { .. } | PressMode::Alert);
        let modifiers = drawing_modifiers(input.modifiers);
        let mut created_on_release = false;
        match press.mode {
            PressMode::TextEditor
            | PressMode::Separator { .. }
            | PressMode::Inert
            | PressMode::TouchTracking => {}
            PressMode::DeltaTooltip => {}
            PressMode::TimeAxis => self.time_axis_end_scale(),
            PressMode::PriceAxis { pane, target } => self.price_axis_end_scale(pane, target),
            PressMode::Measure => {
                self.measure_pointer_up(x, y, modifiers);
            }
            PressMode::DrawingCreation {
                capture,
                committed_on_press,
            } => {
                if capture {
                    self.flush_coalesced_input();
                }
                let update = self.drawing_tool_pointer_up(x, y, modifiers);
                created_on_release = committed_on_press || update.created.is_some();
                self.note_drawing_created(update.created);
                if !capture && !moved && !committed_on_press && update.created.is_none() {
                    let update = self.drawing_tool_activate(x, y, modifiers);
                    created_on_release |= update.created.is_some();
                    self.note_drawing_created(update.created);
                }
            }
            PressMode::Trading { dragging } => {
                if dragging {
                    self.trading_drag_end();
                }
                if !moved
                    && self
                        .trading_hit_at(press.start.0, press.start.1)
                        .zip(self.trading_hit_at(x, y))
                        .is_some_and(|(start, end)| {
                            start.object == end.object
                                && start.kind == end.kind
                                && start.annotation_id == end.annotation_id
                        })
                {
                    self.trading_activate_at(x, y);
                }
            }
            PressMode::Alert => {
                if !moved && self.alert_create_hit_at(x, y) {
                    self.activate_alert_create_at(x, y);
                }
            }
            PressMode::DrawingDrag => {
                self.drawing_drag_end();
                if !moved {
                    self.input_primary_click(x, y, press.text_press_selected);
                    self.push_input_event(ChartInputEvent::Click { x, y });
                }
            }
            PressMode::Pane { price_pan, panning } => {
                if let Some((pane, target)) = price_pan {
                    self.price_axis_end_scroll(pane, target);
                }
                if panning {
                    self.end_pan(input.timestamp_ms, input.device);
                }
                if !moved {
                    self.input_primary_click(x, y, press.text_press_selected);
                    self.push_input_event(ChartInputEvent::Click { x, y });
                }
            }
        }
        self.apply_input_magnet();
        self.refresh_pointer_hover(x, y, input.timestamp_ms);
        if input.device == InputDevice::Touch {
            self.input.pointer = None;
            self.clear_pointer_hover();
        }
        if created_on_release {
            // The new drawing appeared under a stationary pointer. Promote it after the next
            // motion, so committing an empty label does not show its hover-only placeholder.
            self.set_hovered_drawing(None);
            self.set_hovered_text(None);
        }
        self.refresh_input_cursor();
    }

    /// The pointer left the chart without a held button. Captured gestures are unaffected; a live
    /// measure stays on screen.
    pub fn input_pointer_leave(&mut self) {
        if self.input.press.is_some() || self.input.touch_tracking.is_some() {
            return;
        }
        self.input.pointer = None;
        self.input.tooltip_deadline_ms = None;
        if self.delta_tooltip_leave() {
            self.note_delta_tooltip_changed();
        }
        self.set_separator_hover(None);
        self.set_crosshair_ohlc_magnet(false);
        self.clear_trading_hover();
        self.clear_pointer_hover();
        self.input.cursor = ChartCursor::Crosshair;
        self.input.frame_dirty = true;
    }

    /// Abandon every in-flight gesture without committing it: capture loss, focus loss, a host
    /// modal occluding the chart, resize, or disposal.
    pub fn input_cancel(&mut self) {
        self.abandon_press();
        self.input.touch_longpress_deadline_ms = None;
        self.input.touch_tracking = None;
        self.stop_input_motion();
        self.input.tooltip_deadline_ms = None;
        self.cancel_measure();
        self.clear_trading_hover();
        self.set_crosshair_ohlc_magnet(false);
        self.set_separator_hover(None);
        self.clear_pointer_hover();
        self.input.pointer = None;
        self.input.cursor = ChartCursor::Crosshair;
        self.input.frame_dirty = true;
    }

    /// Cancel one captured pointer without disturbing a different active pointer.
    pub fn input_cancel_pointer(&mut self, id: u32) {
        if self.input.press.is_some_and(|press| press.id == id)
            || (self.input.resolver.state() == GestureState::Pinching
                && self.input.resolver.contains_pointer(id))
        {
            self.input_cancel();
        }
    }

    /// Modifier keys changed without pointer movement: refresh the drawing magnet live.
    pub fn input_modifiers_changed(&mut self, modifiers: InputModifiers) {
        self.input.modifiers = modifiers;
        if self.apply_input_magnet() {
            self.input.frame_dirty = true;
        }
    }

    /// Secondary click. Cancels any gesture and queues a [`ChartInputEvent::ContextMenu`].
    pub fn input_context_menu(&mut self, x: f64, y: f64) {
        self.commit_drawing_text_edit();
        self.input_cancel();
        let region = self.region_at(x, y);
        let context = matches!(region, ChartRegion::Pane)
            .then(|| self.chart_context_at(x, y))
            .flatten();
        self.push_input_event(ChartInputEvent::ContextMenu(ChartContextMenu {
            x,
            y,
            region,
            context,
        }));
    }

    // --- wheel ---

    /// One wheel sample in pane coordinates with browser-normalized deltas (`delta / 100`, wheel
    /// up positive). Returns whether the chart consumed it; an unconsumed wheel may scroll the
    /// host page.
    pub fn input_wheel(&mut self, sample: WheelSample) -> bool {
        let options = self.input.options;
        let intent = sample.intent(options.wheel_behavior);
        // Auto pans only on the horizontal axis; explicit modes pan along the dominant axis.
        let pan_delta = if options.wheel_behavior != WheelBehavior::Auto
            && sample.delta_x.abs() < sample.delta_y.abs()
        {
            -sample.delta_y
        } else {
            sample.delta_x
        };
        let zoom = matches!(intent, WheelIntent::Zoom | WheelIntent::PanAndZoom)
            && sample.delta_y != 0.0
            && options.wheel_zoom;
        let scroll = matches!(intent, WheelIntent::Pan | WheelIntent::PanAndZoom)
            && pan_delta != 0.0
            && options.wheel_scroll;
        if !zoom && !scroll {
            return false;
        }
        self.stop_input_motion();
        self.input.pointer = Some((sample.x, sample.y));
        self.input.modifiers = sample.modifiers;
        if zoom {
            let scale = wheel_zoom_scale(sample.delta_y);
            let pane = self.pane_index_at_y(sample.y);
            let price_target = (sample.y <= self.pane_h)
                .then(|| self.price_axis_target_at(pane, sample.x))
                .flatten()
                .filter(|_| {
                    options.wheel_behavior == WheelBehavior::Zoom || options.price_axis_wheel_zoom
                });
            if let Some(target) = price_target {
                self.price_axis_wheel_zoom(pane, target, sample.y, scale);
            } else {
                self.wheel_zoom_time_scale(sample.x, scale, sample.modifiers);
            }
        }
        if scroll {
            self.time_scale_start_scroll(0.0);
            self.time_scale_scroll_to(WHEEL_SCROLL_PX_PER_DELTA * pan_delta);
            self.time_scale_end_scroll();
        }
        self.input.frame_dirty = true;
        self.refresh_pointer_hover(sample.x, sample.y, sample.timestamp_ms);
        self.refresh_input_cursor();
        true
    }

    /// Wheel zoom of the time scale at pane x `x`. Ctrl/Cmd zooms around the pointer; otherwise the
    /// time scale's right-bar policy decides the anchor (by default the right edge stays pinned, as
    /// measured on TradingView). Every host's wheel path resolves the anchor here.
    pub fn wheel_zoom_time_scale(&mut self, x: f64, scale: f64, modifiers: InputModifiers) {
        let x = self.clamp_zoom_anchor(x);
        if modifiers.control || modifiers.meta {
            self.time_scale_zoom_focused(x, scale);
        } else {
            self.time_scale_zoom(x, scale);
        }
    }

    /// A trackpad or touch pinch step (`scale_delta` is the ratio change since the previous step).
    /// Pinching is direct manipulation, so it always zooms around the pinch point regardless of the
    /// wheel's right-edge policy. Returns whether the chart consumed it.
    pub fn input_pinch(&mut self, x: f64, y: f64, scale_delta: f64, timestamp_ms: f64) -> bool {
        if !self.input.options.pinch_zoom || !scale_delta.is_finite() || scale_delta == 0.0 {
            return false;
        }
        self.stop_input_motion();
        let anchor = self.clamp_zoom_anchor(x);
        self.time_scale_zoom_focused(anchor, pinch_zoom_scale(scale_delta));
        self.input.frame_dirty = true;
        self.refresh_pointer_hover(x, y, timestamp_ms);
        self.refresh_input_cursor();
        true
    }

    /// A zoom anchor stays inside the plot: a wheel or pinch over the price-axis strip anchors at
    /// the plot edge, never beyond it.
    fn clamp_zoom_anchor(&self, x: f64) -> f64 {
        x.max(1.0).min(self.time_scale.width())
    }

    // --- keyboard ---

    /// A key press while the chart has focus. Text editing is routed by the host first because its
    /// key conventions are platform-specific. Returns whether the chart consumed the key.
    pub fn input_key_down(
        &mut self,
        key: ChartKey,
        modifiers: InputModifiers,
        repeat: bool,
        now_ms: f64,
    ) -> bool {
        let options = self.input.options;
        let step = if modifiers.control || modifiers.shift {
            10.0
        } else {
            1.0
        };
        // Keys follow the same gesture switches as the pointer: a view fixed with `pan`,
        // `wheel_scroll`, and `wheel_zoom` off (an intraday session lock) cannot be moved from the
        // keyboard either, and a gated key stays unconsumed so the platform keeps its default.
        let scroll = options.pan || options.wheel_scroll;
        if key == ChartKey::Escape {
            self.cancel_drawing_edit_session();
        } else {
            self.commit_drawing_edit_session();
        }
        let handled = match key {
            ChartKey::ArrowUp | ChartKey::ArrowDown | ChartKey::Tab => false,
            ChartKey::ArrowLeft | ChartKey::ArrowRight if scroll => {
                let delta = if key == ChartKey::ArrowLeft {
                    -step
                } else {
                    step
                };
                self.begin_keyboard_pan(key, delta, repeat, now_ms);
                true
            }
            ChartKey::PageUp | ChartKey::PageDown if scroll => {
                let page = (self.pane_w / self.bar_spacing() * KEYBOARD_PAGE_FRACTION).max(1.0);
                let direction = if key == ChartKey::PageUp { -1.0 } else { 1.0 };
                self.scroll_to_position(self.scroll_position() + direction * page);
                true
            }
            ChartKey::ZoomIn | ChartKey::ZoomOut if options.wheel_zoom => {
                let step = if key == ChartKey::ZoomIn {
                    KEYBOARD_ZOOM_STEP
                } else {
                    -KEYBOARD_ZOOM_STEP
                };
                self.time_scale_zoom(self.pane_w / 2.0, step);
                true
            }
            ChartKey::ZoomIn | ChartKey::ZoomOut => false,
            // Home resets the whole view on every host: the time axis refits and every price
            // scale returns to autoscale. The time-axis reset switch gates it, so a view a host
            // fixed stays fixed from the keyboard.
            ChartKey::Home if options.axis_double_click_reset_time => {
                self.reset_view();
                self.input.layout_dirty = true;
                true
            }
            ChartKey::End if scroll => {
                self.scroll_to_latest();
                true
            }
            ChartKey::ArrowLeft
            | ChartKey::ArrowRight
            | ChartKey::PageUp
            | ChartKey::PageDown
            | ChartKey::Home
            | ChartKey::End => false,
            // Enter finishes a variable-length placement; with none under way it edits the
            // selected drawing's text, like F2.
            ChartKey::Enter => {
                let update = self.drawing_tool_finish();
                self.note_drawing_created(update.created);
                update.created.is_some() || self.edit_selected_drawing_text()
            }
            ChartKey::EditText => self.edit_selected_drawing_text(),
            ChartKey::Backspace
                if self.drawing_tool_sequence_active() && self.drawing_create_active() =>
            {
                self.drawing_tool_pop_anchor()
            }
            ChartKey::Backspace | ChartKey::Delete => self.delete_selection(),
            ChartKey::Escape => {
                self.abandon_press();
                self.discard_trading_interaction();
                self.cancel_drawing_tool();
                self.set_selected_drawing(None);
                self.clear_volume_profile_selection();
                self.clear_brushable_ranges();
                self.clear_pointer_hover();
                self.push_input_event(ChartInputEvent::CrosshairLeft);
                true
            }
            ChartKey::Undo => self.undo_drawing(),
            ChartKey::Redo => self.redo_drawing(),
        };
        if handled {
            if !matches!(key, ChartKey::ArrowLeft | ChartKey::ArrowRight) {
                self.stop_kinetic_coast();
            }
            self.input.frame_dirty = true;
            self.refresh_input_cursor();
        }
        handled
    }

    /// Key release. Only a held arrow pan reacts.
    pub fn input_key_up(&mut self, key: ChartKey) -> bool {
        if self.input.keyboard_pan.is_none_or(|pan| pan.key != key) {
            return false;
        }
        self.input.keyboard_pan = None;
        self.cancel_keyboard_scroll();
        self.input.frame_dirty = true;
        true
    }

    /// Cancel held-key and gesture motion when a host loses focus or supersedes an old adapter.
    pub fn input_cancel_motion(&mut self) {
        self.stop_input_motion();
    }

    /// A key press while a chart focus target has keyboard focus. Returns whether it was consumed.
    ///
    /// - Price axis: Home restores autoscale; ArrowUp/ArrowDown zoom the range 5% around its
    ///   center.
    /// - Time axis: Home resets the time scale (needs the time-axis reset switch);
    ///   ArrowLeft/ArrowRight scroll one bar (need `pan` or `wheel_scroll`).
    /// - Separator: Home splits the two adjacent panes evenly; ArrowUp/ArrowDown move the divider
    ///   10 px.
    /// - Drawing: the target is selected. Enter opens or commits an edit; while editing, arrows
    ///   nudge 1 px (10 px with Shift), Tab/Shift+Tab cycle the anchor, and Escape restores the
    ///   start. A committed edit is one undo entry. Delete/Backspace remove the drawing, and F2
    ///   opens the inline text editor of a drawing that paints text.
    pub fn input_target_key_down(
        &mut self,
        target: ChartFocusTarget,
        key: ChartKey,
        modifiers: InputModifiers,
    ) -> bool {
        if !matches!(target, ChartFocusTarget::Drawing(_)) {
            self.commit_drawing_edit_session();
        }
        let handled = match target {
            ChartFocusTarget::PriceAxis { pane, target } => match key {
                ChartKey::Home => {
                    let known = self.price_scale_auto_scale_for(pane, target).is_some();
                    if known {
                        self.set_price_scale_auto_scale_for(pane, target, true);
                    }
                    known
                }
                ChartKey::ArrowUp | ChartKey::ArrowDown => {
                    match self.price_scale_visible_range_for(pane, target) {
                        Some((from, to)) => {
                            let center = (from + to) / 2.0;
                            let half = (to - from)
                                * if key == ChartKey::ArrowUp {
                                    PRICE_AXIS_KEY_ZOOM_IN
                                } else {
                                    PRICE_AXIS_KEY_ZOOM_OUT
                                };
                            self.set_price_scale_visible_range_for(
                                pane,
                                target,
                                center - half,
                                center + half,
                            );
                            true
                        }
                        None => false,
                    }
                }
                _ => false,
            },
            // Keys follow the same gesture switches as the pointer and `input_key_down`: a view
            // fixed with `pan` and `wheel_scroll` off cannot be scrolled from the axis, and a
            // reset needs the time axis's own reset switch. A gated key stays unconsumed.
            ChartFocusTarget::TimeAxis => {
                let options = self.input.options;
                match key {
                    ChartKey::Home if options.axis_double_click_reset_time => {
                        self.reset_time_scale();
                        true
                    }
                    ChartKey::ArrowLeft | ChartKey::ArrowRight
                        if options.pan || options.wheel_scroll =>
                    {
                        let delta = if key == ChartKey::ArrowLeft {
                            -1.0
                        } else {
                            1.0
                        };
                        self.scroll_to_position(self.scroll_position() + delta);
                        true
                    }
                    _ => false,
                }
            }
            ChartFocusTarget::Separator(index) if index + 1 < self.panes.len() => {
                let handled = match key {
                    ChartKey::Home => {
                        let share = (self.panes[index].stretch_factor
                            + self.panes[index + 1].stretch_factor)
                            / 2.0;
                        self.panes[index].stretch_factor = share;
                        self.panes[index + 1].stretch_factor = share;
                        true
                    }
                    ChartKey::ArrowUp => {
                        self.drag_pane_separator(index, -SEPARATOR_KEY_STEP);
                        true
                    }
                    ChartKey::ArrowDown => {
                        self.drag_pane_separator(index, SEPARATOR_KEY_STEP);
                        true
                    }
                    _ => false,
                };
                self.input.layout_dirty |= handled;
                handled
            }
            ChartFocusTarget::Separator(_) => false,
            ChartFocusTarget::Drawing(id) => self.drawing_target_key(id, key, modifiers),
        };
        if handled {
            self.stop_input_motion();
            self.input.frame_dirty = true;
            self.refresh_input_cursor();
        }
        handled
    }

    // --- internals ---

    fn mouse_sample(input: PointerInput, target: InputTarget) -> PointerSample {
        PointerSample {
            id: input.id,
            device: input.device,
            target,
            modifiers: input.modifiers,
            x: input.x,
            y: input.y,
            timestamp_ms: input.timestamp_ms,
            pressure: 0.5,
            tilt_x: 0.0,
            tilt_y: 0.0,
        }
    }

    fn push_input_event(&mut self, event: ChartInputEvent) {
        if self.input.events.len() == MAX_PENDING_INPUT_EVENTS {
            self.input.events.pop_front();
        }
        self.input.events.push_back(event);
    }

    /// Hover motion can change the comparison on every sample; one pending notice is enough.
    fn note_delta_tooltip_changed(&mut self) {
        self.sync_brushable_areas();
        if !self
            .input
            .events
            .contains(&ChartInputEvent::DeltaTooltipChanged)
        {
            self.push_input_event(ChartInputEvent::DeltaTooltipChanged);
        }
    }

    /// Touch comparisons follow the first two retained touches: one finger previews a point, two
    /// commit a range.
    fn update_touch_delta_tooltip(&mut self) {
        let (xs, count) = self.input.resolver.touch_xs();
        if count > 0 && self.delta_tooltip_touch_move(&xs[..count]) {
            self.note_delta_tooltip_changed();
        }
    }

    fn drawing_target_key(
        &mut self,
        id: DrawingId,
        key: ChartKey,
        modifiers: InputModifiers,
    ) -> bool {
        let Some(drawing) = self.drawing(id).cloned() else {
            self.input.drawing_edit = None;
            return false;
        };
        if self
            .input
            .drawing_edit
            .as_ref()
            .is_some_and(|session| session.before.id != id)
        {
            self.commit_drawing_edit_session();
        }
        if self.selected_drawing() != Some(id) {
            self.set_selected_drawing(Some(id));
        }
        let editing = self.input.drawing_edit.is_some();
        match key {
            ChartKey::Enter if editing => {
                self.commit_drawing_edit_session();
                true
            }
            ChartKey::Enter => {
                self.input.drawing_edit = Some(DrawingEditSession {
                    before: drawing,
                    anchor: None,
                });
                true
            }
            ChartKey::Escape if editing => {
                self.cancel_drawing_edit_session();
                true
            }
            ChartKey::Delete | ChartKey::Backspace => {
                self.commit_drawing_edit_session();
                self.input.last_click = None;
                self.input.control_click = false;
                self.remove_drawing(id)
            }
            // Edit the drawing's own text in the inline editor, which announces itself and
            // returns focus here when it closes.
            ChartKey::EditText => {
                self.commit_drawing_edit_session();
                self.edit_selected_drawing_text()
            }
            ChartKey::Tab if editing => {
                // The engine's editable handles (every anchor, a rectangle's eight bounds handles,
                // a position's target/entry/width/stop controls, or a family's derived handles),
                // not the raw anchor list: `nudge_selected_drawing` takes the same index.
                let count = self.drawing_handle_count(id).unwrap_or(0);
                let Some(session) = self.input.drawing_edit.as_mut().filter(|_| count > 0) else {
                    return false;
                };
                session.anchor = Some(match session.anchor {
                    _ if modifiers.shift && session.anchor.is_none_or(|anchor| anchor == 0) => {
                        count - 1
                    }
                    Some(anchor) if modifiers.shift => anchor - 1,
                    Some(anchor) => (anchor + 1) % count,
                    None => 0,
                });
                true
            }
            ChartKey::ArrowLeft
            | ChartKey::ArrowRight
            | ChartKey::ArrowUp
            | ChartKey::ArrowDown
                if editing =>
            {
                let step = if modifiers.shift { 10.0 } else { 1.0 };
                let (dx, dy) = match key {
                    ChartKey::ArrowLeft => (-step, 0.0),
                    ChartKey::ArrowRight => (step, 0.0),
                    ChartKey::ArrowUp => (0.0, -step),
                    _ => (0.0, step),
                };
                let anchor = self
                    .input
                    .drawing_edit
                    .as_ref()
                    .and_then(|session| session.anchor);
                self.nudge_selected_drawing_with_history(dx, dy, anchor, false)
            }
            _ => false,
        }
    }

    /// Record an open keyboard drawing edit as one undo entry.
    fn commit_drawing_edit_session(&mut self) {
        if let Some(session) = self.input.drawing_edit.take() {
            self.record_drawing_edit(session.before);
        }
    }

    /// Discard an open keyboard drawing edit, restoring its start.
    fn cancel_drawing_edit_session(&mut self) {
        if let Some(session) = self.input.drawing_edit.take() {
            self.restore_drawing_points(session.before);
        }
    }

    /// A committed placement. Tools that request typing open the engine session directly.
    fn note_drawing_created(&mut self, created: Option<DrawingId>) {
        let Some(id) = created else {
            return;
        };
        if self.drawing_requests_text_edit(id) {
            self.begin_drawing_text_edit(id, true);
        }
        self.push_input_event(ChartInputEvent::DrawingCreated(id));
    }

    /// Stop every motion owned by an earlier gesture: kinetic coast, held keyboard pan, animated
    /// scroll, and any stale scroll snapshot.
    fn stop_input_motion(&mut self) {
        self.stop_kinetic_coast();
        self.input.keyboard_pan = None;
        self.cancel_keyboard_scroll();
        self.cancel_scroll_animation();
    }

    fn stop_kinetic_coast(&mut self) {
        self.kinetic_stop();
        if std::mem::take(&mut self.input.kinetic_active) {
            self.time_scale_end_scroll();
        }
    }

    /// A press that arrives while an earlier one is still open (a lost release) closes the old
    /// gesture without committing it.
    fn end_press_without_commit(&mut self) {
        if self.input.press.is_some() {
            self.input_cancel();
        }
    }

    /// Close the open press without committing it: drags restore their start, captures and
    /// previews are discarded, scale sessions end, and a delta-tooltip gesture settles.
    fn abandon_press(&mut self) {
        self.input.resolver.cancel();
        self.abandon_press_state();
    }

    fn abandon_press_state(&mut self) {
        self.input.pending_capture = None;
        if let Some(press) = self.input.press.take() {
            match press.mode {
                PressMode::Pane { price_pan, panning } => {
                    if let Some((pane, target)) = price_pan {
                        self.price_axis_end_scroll(pane, target);
                    }
                    if panning {
                        self.kinetic_stop();
                        self.time_scale_end_scroll();
                    }
                }
                PressMode::TimeAxis => self.time_axis_end_scale(),
                PressMode::PriceAxis { pane, target } => self.price_axis_end_scale(pane, target),
                PressMode::Trading { dragging: true } => {
                    self.cancel_trading_drag();
                }
                PressMode::DrawingDrag => self.drawing_drag_cancel(),
                PressMode::DrawingCreation { capture: true, .. } => self.cancel_drawing_creation(),
                _ => {}
            }
        }
        self.clear_trading_pressed();
        if self.delta_tooltip_mouse_up() {
            self.note_delta_tooltip_changed();
        }
    }

    /// The temporary Ctrl/Cmd OHLC magnet mirrors the drawing magnet the engine will apply, and only
    /// for drawing work: an armed tool whose effective magnet (chart mode, tool mode, Ctrl/Cmd as
    /// the temporary toggle) is strong, or a drawing drag with Ctrl/Cmd held. Returns whether it
    /// changed.
    fn apply_input_magnet(&mut self) -> bool {
        let modifiers = self.input.modifiers;
        let toggle = modifiers.control || modifiers.meta;
        let armed = self.armed_drawing_magnet(toggle) == DrawingMagnetMode::Strong;
        let dragging = toggle && self.drawing_drag_active();
        self.set_crosshair_ohlc_magnet(armed || dragging)
    }

    /// Whether a press lands on the text being edited: its label, a text tool's body, or the box
    /// of any family text (the layout rect the caret is placed in), so a click inside a note or
    /// callout positions the caret instead of committing.
    fn point_on_edited_text(&self, x: f64, y: f64) -> bool {
        let Some(editing) = self.editing_drawing() else {
            return false;
        };
        if self.drawing_text_hit_at(x, y) == Some(editing) {
            return true;
        }
        if self
            .drawing_text_edit_layout(editing)
            .is_some_and(|layout| {
                let [left, top, right, bottom] = layout.rect;
                (left..=right).contains(&x) && (top..=bottom).contains(&y)
            })
        {
            return true;
        }
        self.hit_test_drawing(x, y).is_some_and(|hit| {
            hit.id == editing
                && self.drawing(editing).is_some_and(|drawing| {
                    drawing.kind == DrawingKind::Text || drawing.kind.is_text_annotation()
                })
        })
    }

    fn begin_press(&mut self, region: ChartRegion, input: PointerInput) -> PressMode {
        let (x, y) = (input.x, input.y);
        let options = self.input.options;
        match region {
            ChartRegion::Separator(index) if options.panes_resize => {
                self.set_separator_hover(None);
                self.panes
                    .get(index + 1)
                    .map_or(PressMode::Inert, |below| PressMode::Separator {
                        index,
                        grab_offset_y: y - below.top,
                    })
            }
            ChartRegion::Separator(_) => PressMode::Inert,
            ChartRegion::PriceAxis { pane, target } => {
                if options.axis_scale_price && self.price_axis_scalable(pane, target) {
                    self.price_axis_start_scale(pane, target, y);
                    PressMode::PriceAxis { pane, target }
                } else {
                    PressMode::Inert
                }
            }
            ChartRegion::TimeAxis => {
                if options.axis_scale_time {
                    self.time_axis_start_scale(x);
                    PressMode::TimeAxis
                } else {
                    PressMode::Inert
                }
            }
            ChartRegion::Pane => self.begin_pane_press(input),
        }
    }

    /// Pane-press arbitration, topmost owner first: a live measure, trading controls, the
    /// crosshair action chip, an armed drawing tool, an existing drawing, the delta tooltip, a
    /// Shift measure, then pan.
    fn begin_pane_press(&mut self, input: PointerInput) -> PressMode {
        let (x, y) = (input.x, input.y);
        let modifiers = drawing_modifiers(input.modifiers);
        let magnet_only = DrawingModifiers {
            magnet: modifiers.magnet,
            straighten: false,
        };
        if self.measure_active() && self.measure_pointer_down(x, y, false, magnet_only) {
            return PressMode::Measure;
        }
        if self.trading_hit_at(x, y).is_some() {
            self.set_trading_pressed(x, y);
            let dragging = self.trading_drag_start_at(x, y);
            return PressMode::Trading { dragging };
        }
        if self.alert_create_hit_at(x, y) {
            return PressMode::Alert;
        }
        self.deactivate_trading_group();
        if self.active_drawing_tool().is_some() {
            let update = self.drawing_tool_pointer_down(x, y, modifiers);
            self.note_drawing_created(update.created);
            return PressMode::DrawingCreation {
                capture: update.pointer_capture,
                committed_on_press: update.created.is_some(),
            };
        }
        if self.drawing_drag_start_at(x, y) {
            return PressMode::DrawingDrag;
        }
        if self.delta_tooltip_mouse_down_with_shift(x, input.modifiers.shift) {
            self.sync_brushable_areas();
            return PressMode::DeltaTooltip;
        }
        if input.modifiers.shift && self.measure_pointer_down(x, y, true, magnet_only) {
            return PressMode::Measure;
        }
        let price_pan = self
            .input
            .options
            .pan_for(input.device)
            .then(|| {
                let pane = self.pane_index_at_y(y);
                self.price_pan_target_at(pane, x, y)
                    .map(|target| (pane, target))
            })
            .flatten();
        PressMode::Pane {
            price_pan,
            panning: false,
        }
    }

    fn input_double_click(
        &mut self,
        region: ChartRegion,
        input: PointerInput,
        text_press_selected: Option<DrawingId>,
        control_pair: bool,
    ) {
        let (x, y) = (input.x, input.y);
        let options = self.input.options;
        match region {
            ChartRegion::TimeAxis if options.axis_double_click_reset_time => {
                self.reset_time_scale();
            }
            ChartRegion::PriceAxis { pane, target } if options.axis_double_click_reset_price => {
                self.reset_price_scale(pane, target);
            }
            ChartRegion::Pane => {
                if self.drawing_tool_sequence_active() {
                    let modifiers = drawing_modifiers(input.modifiers);
                    let update = self.drawing_tool_activate(x, y, modifiers);
                    self.note_drawing_created(update.created);
                    let update = self.drawing_tool_finish();
                    self.note_drawing_created(update.created);
                    return;
                }
                // The second click acts on the drawing it landed on, never on a bare selection:
                // the point must belong to the selected drawing, and a trading control or the
                // alert chip under it, or one that took either click of the pair, keeps the click.
                let selected = self.selected_drawing();
                let owned = selected.is_some()
                    && selected == text_press_selected
                    && !control_pair
                    && self.trading_hit_at(x, y).is_none()
                    && !self.alert_create_hit_at(x, y)
                    && self.drawing_at(x, y) == selected;
                match selected.filter(|_| owned) {
                    None => self.clear_brushable_ranges(),
                    Some(id) => {
                        // The text tool and a trend line's own label re-run click activation
                        // (two-step focus, trend prompt); every other text-bearing drawing, a
                        // trend line's body included, opens its editor. A drawing without text,
                        // a locked or hidden one, or one with no layout refuses and falls back
                        // to clearing the brush ranges.
                        let kind = self.drawing(id).map(|drawing| drawing.kind);
                        let on_trend_label = kind == Some(DrawingKind::TrendLine)
                            && self.drawing_text_hit_at(x, y) == Some(id);
                        if kind == Some(DrawingKind::Text) || on_trend_label {
                            self.input_primary_click(x, y, text_press_selected);
                        } else if self.begin_drawing_text_edit(id, true) {
                            self.push_input_event(ChartInputEvent::TextEditorOpened(id));
                        } else {
                            self.clear_brushable_ranges();
                        }
                    }
                }
                // A double-click always reaches the host, whatever it landed on.
                self.push_input_event(ChartInputEvent::DoubleClick { x, y });
            }
            _ => {}
        }
    }

    /// Click-to-select, drawings first: a trend label or drawing hit selects it and clears the
    /// series selection; a miss falls through to the series under the click (or clears it).
    fn input_primary_click(&mut self, x: f64, y: f64, text_press_selected: Option<DrawingId>) {
        let trend_text_hit = self.drawing_text_hit_at(x, y);
        let drawing_hit = trend_text_hit.is_some() || self.select_drawing_at(x, y);
        if let Some(id) = trend_text_hit {
            self.set_selected_drawing(Some(id));
        }
        if let Some(id) = (!drawing_hit)
            .then(|| self.volume_profile_indicator_at(x, y))
            .flatten()
        {
            self.set_selected_volume_profile_indicator(Some(id));
            return;
        }
        self.clear_volume_profile_selection();
        let general_hit = !drawing_hit
            && self
                .pane_at_y(y)
                .and_then(|pane| self.update_general_hover(pane, x, y))
                .is_some();
        if general_hit {
            self.select_general_hovered();
        } else {
            self.clear_general_selection();
        }
        let series = (!drawing_hit && !general_hit)
            .then(|| self.hit_test_series(x, y))
            .flatten();
        self.set_selected_series(series);
        if !drawing_hit {
            return;
        }
        let Some(drawing) = self.selected_drawing().and_then(|id| self.drawing(id)) else {
            return;
        };
        let id = drawing.id;
        let open_editor = if drawing.kind == DrawingKind::TrendLine {
            trend_text_hit == Some(id)
        } else {
            (drawing.kind == DrawingKind::Text || drawing.kind.is_text_annotation())
                && (drawing.text.trim().is_empty() || text_press_selected == Some(id))
        };
        if open_editor {
            self.begin_drawing_text_edit(id, true);
            self.push_input_event(ChartInputEvent::TextEditorOpened(id));
        }
    }

    fn continue_pan(
        &mut self,
        input: PointerInput,
        update: GestureUpdateKind,
        price_pan: Option<(usize, PriceScaleTarget)>,
        panning: bool,
    ) {
        let (x, y) = (input.x, input.y);
        match update {
            // Pane panning opens on the threshold sample; movement begins on the next sample.
            GestureUpdateKind::DragStarted
                if !panning && self.input.options.pan_for(input.device) =>
            {
                self.time_scale_start_scroll(x);
                self.kinetic_begin_sampling(
                    self.input.options.kinetic_for(input.device)
                        && !self.input.options.reduced_motion,
                    self.scroll_position(),
                    input.timestamp_ms,
                );
                let price_pan = price_pan.filter(|&(pane, target)| {
                    self.price_scale_auto_scale_for(pane, target) == Some(false)
                });
                if let Some((pane, target)) = price_pan {
                    self.price_axis_start_scroll(pane, target, y);
                }
                if let Some(press) = self.input.press.as_mut() {
                    press.mode = PressMode::Pane {
                        price_pan,
                        panning: true,
                    };
                }
            }
            GestureUpdateKind::DragMoved if panning => {
                self.time_scale_scroll_to(x);
                self.kinetic_add_sample(self.scroll_position(), input.timestamp_ms);
                if let Some((pane, target)) = price_pan {
                    self.price_axis_scroll_to(pane, target, y);
                }
            }
            _ => {}
        }
    }

    /// End a pan: coast when the flick qualifies, otherwise close the scroll session.
    fn end_pan(&mut self, now_ms: f64, device: InputDevice) {
        let coasting = self.input.options.kinetic_for(device)
            && !self.input.options.reduced_motion
            && self.kinetic_release(self.scroll_position(), now_ms);
        if coasting {
            self.input.kinetic_active = true;
        } else {
            self.kinetic_stop();
            self.time_scale_end_scroll();
        }
    }

    fn begin_keyboard_pan(&mut self, key: ChartKey, delta: f64, repeat: bool, now_ms: f64) {
        self.stop_kinetic_coast();
        self.cancel_scroll_animation();
        if self.input.options.reduced_motion {
            self.input.keyboard_pan = None;
            self.cancel_keyboard_scroll();
            self.scroll_to_position(self.scroll_position() + delta);
            return;
        }
        // The engine owns the repeat cadence, so an unchanged OS repeat must not reset velocity;
        // only a new direction or modifier speed retunes the live session.
        let current = self.input.keyboard_pan;
        if !repeat || current != Some(KeyboardPan { key, delta }) {
            self.start_keyboard_scroll(delta, now_ms);
            self.input.keyboard_pan = Some(KeyboardPan { key, delta });
        }
    }

    /// Enter/F2: open the selected drawing's text editor (the engine paints the caret). Refused,
    /// and left unconsumed, for a drawing with no text, a locked or hidden one, or one scrolled out
    /// of view.
    fn edit_selected_drawing_text(&mut self) -> bool {
        let Some(id) = self.selected_drawing() else {
            return false;
        };
        let opened = self.begin_drawing_text_edit(id, true);
        if opened {
            // A host with its own text surface (the browser) presents it for the open session.
            self.push_input_event(ChartInputEvent::TextEditorOpened(id));
        }
        opened
    }

    /// Delete the selection: a drawing or an engine indicator binding is removed here; any other
    /// selected series is host-owned and is requested through [`ChartInputEvent::RemoveSeries`].
    fn delete_selection(&mut self) -> bool {
        if self.remove_selected_drawing() {
            self.input.last_click = None;
            self.input.control_click = false;
            return true;
        }
        if let Some(id) = self.selected_volume_profile_indicator() {
            self.clear_volume_profile_selection();
            return self.remove_native_primitive(id);
        }
        let Some(series) = self.selected_series() else {
            return false;
        };
        if self.external_study_for_series(series).is_none()
            && self.remove_indicator_for_series(series)
        {
            self.set_selected_series(None);
            return true;
        }
        self.push_input_event(ChartInputEvent::RemoveSeries(series));
        true
    }

    fn clear_pointer_hover(&mut self) {
        self.clear_crosshair_at();
        self.clear_hover();
    }

    /// Hover arbitration, topmost owner first (reference `hitTestPane`): a `Top` host primitive,
    /// a drawing, then series in paint order where a series' own non-bottom primitive blocks it
    /// and every series below; then a `Normal` pane primitive, a general-series item, and finally
    /// a `Bottom` or pane primitive that survives only without a series hit.
    fn arbitrate_hover(
        &mut self,
        x: f64,
        y: f64,
        primitive: Option<HostPrimitiveHit>,
    ) -> ChartHover {
        self.set_hovered_volume_profile(None);
        let Some(pane) = self.pane_at_y(y) else {
            self.set_hovered_series(None);
            self.set_hovered_text(None);
            self.set_hovered_drawing(None);
            self.clear_general_hover();
            return ChartHover::None;
        };
        let general = self.update_general_hover(pane, x, y).is_some();
        if let Some(hit) = primitive.filter(|hit| hit.layer == HostPrimitiveLayer::Top) {
            self.clear_general_hover();
            self.set_hovered_series(hit.series);
            self.set_hovered_text(None);
            self.set_hovered_drawing(None);
            return ChartHover::HostPrimitive(hit);
        }
        if let Some((id, cursor)) = self.update_drawing_hover(x, y) {
            self.clear_general_hover();
            self.set_hovered_series(None);
            return ChartHover::Drawing { id, cursor };
        }
        // Profile bars paint over their source series, so they win against series below them.
        if let Some(id) = self.volume_profile_indicator_at(x, y) {
            self.clear_general_hover();
            self.set_hovered_series(None);
            self.set_hovered_volume_profile(Some(id));
            return ChartHover::VolumeProfile(id);
        }
        let mut best: Option<SeriesHit> = None;
        for index in (0..self.series_order.len()).rev() {
            let id = self.series_order[index];
            if let Some(hit) = primitive
                .filter(|hit| hit.series == Some(id) && hit.layer != HostPrimitiveLayer::Bottom)
            {
                self.clear_general_hover();
                return match best {
                    Some(best) => {
                        self.set_hovered_series(Some(best.series));
                        ChartHover::Series(best.series)
                    }
                    None => {
                        self.set_hovered_series(hit.series);
                        ChartHover::HostPrimitive(hit)
                    }
                };
            }
            if self
                .series_entry(id)
                .is_none_or(|series| series.pane_index != pane)
            {
                continue;
            }
            let Some(candidate) = self.hit_test_one_series(id, x, y) else {
                continue;
            };
            if best.is_none_or(|current| candidate.is_better_than(&current)) {
                best = Some(candidate);
            }
        }
        if let Some(hit) = best {
            self.clear_general_hover();
            self.set_hovered_series(Some(hit.series));
            return ChartHover::Series(hit.series);
        }
        if let Some(hit) =
            primitive.filter(|hit| hit.layer == HostPrimitiveLayer::Normal && hit.series.is_none())
        {
            self.clear_general_hover();
            self.set_hovered_series(None);
            return ChartHover::HostPrimitive(hit);
        }
        if general {
            self.set_hovered_series(None);
            return ChartHover::General;
        }
        if let Some(hit) = primitive {
            self.set_hovered_series(hit.series);
            return ChartHover::HostPrimitive(hit);
        }
        self.set_hovered_series(None);
        ChartHover::None
    }

    /// Crosshair and hover promotion for a pointer position. Over axis strips or a separator the
    /// crosshair hides; during a captured drag it keeps tracking, clamped into the plot.
    fn refresh_pointer_hover(&mut self, x: f64, y: f64, now_ms: f64) {
        let captured = self.input.press.is_some();
        if matches!(
            self.input.press.map(|press| press.mode),
            Some(PressMode::Separator { .. })
        ) {
            self.clear_pointer_hover();
            return;
        }
        let outside_plot = x < 0.0 || x > self.pane_w || y < 0.0 || y > self.pane_h;
        if !captured && (outside_plot || !matches!(self.region_at(x, y), ChartRegion::Pane)) {
            self.clear_pointer_hover();
            return;
        }
        let x = x.clamp(0.0, self.pane_w.max(0.0));
        let y = y.clamp(0.0, self.pane_h.max(0.0));
        self.set_crosshair_at(x, y);
        // Host primitives are re-arbitrated by hosts that own them through
        // `resolve_pointer_hover`; the built-in candidates resolve here.
        self.input.host_primitive_cursor = false;
        self.input.hover = self.arbitrate_hover(x, y, None);
        // Only a changed trading hover restarts the dwell; holding still lets it elapse.
        if !captured && self.set_trading_hover(x, y) {
            let on_close = self
                .trading_hit_at(x, y)
                .is_some_and(|hit| hit.kind == TradingHitKind::CancelButton);
            self.input.tooltip_deadline_ms = on_close.then_some(now_ms + TRADING_TOOLTIP_DWELL_MS);
        }
    }

    fn refresh_input_cursor(&mut self) {
        self.input.cursor = self.resolve_input_cursor();
    }

    fn resolve_input_cursor(&self) -> ChartCursor {
        let Some((x, y)) = self.input.pointer else {
            return ChartCursor::Crosshair;
        };
        let options = self.input.options;
        if let Some(press) = self.input.press {
            match press.mode {
                PressMode::Pane { panning: true, .. } | PressMode::DrawingDrag => {
                    return ChartCursor::Grabbing;
                }
                PressMode::TimeAxis => return ChartCursor::ResizeHorizontal,
                PressMode::PriceAxis { .. } => return ChartCursor::ResizeVertical,
                PressMode::Separator { .. } => return ChartCursor::ResizeRow,
                PressMode::Trading { dragging: true } => return ChartCursor::VerticalGrabbing,
                PressMode::Trading { dragging: false } | PressMode::Alert => {
                    return ChartCursor::Pointer;
                }
                PressMode::TextEditor => return ChartCursor::Text,
                PressMode::Measure
                | PressMode::DrawingCreation { .. }
                | PressMode::TouchTracking
                | PressMode::DeltaTooltip => {
                    return ChartCursor::Crosshair;
                }
                PressMode::Pane { panning: false, .. } | PressMode::Inert => {}
            }
        }
        match self.region_at(x, y) {
            ChartRegion::Separator(_) if options.panes_resize => return ChartCursor::ResizeRow,
            ChartRegion::Separator(_) | ChartRegion::Pane => {}
            ChartRegion::PriceAxis { pane, target } => {
                return if options.axis_scale_price && self.price_axis_scalable(pane, target) {
                    ChartCursor::ResizeVertical
                } else {
                    ChartCursor::Default
                };
            }
            ChartRegion::TimeAxis => {
                return if options.axis_scale_time {
                    ChartCursor::ResizeHorizontal
                } else {
                    ChartCursor::Default
                };
            }
        }
        // A live measure or an armed tool keeps the crosshair over every chart object.
        if self.measure_active() || self.active_drawing_tool().is_some() {
            return ChartCursor::Crosshair;
        }
        if self.trading_preview().is_some() {
            return ChartCursor::VerticalGrabbing;
        }
        if let Some(cursor) = self.trading_cursor_at(x, y) {
            return match cursor {
                TradingCursor::Grab => ChartCursor::VerticalGrab,
                TradingCursor::Pointer => ChartCursor::Pointer,
            };
        }
        if self.alert_create_hit_at(x, y) {
            return ChartCursor::Pointer;
        }
        if self.input.host_primitive_cursor {
            return ChartCursor::HostPrimitive;
        }
        if let Some((_, name)) = self.drawing_hover_at(x, y) {
            return ChartCursor::from_hit_name(name);
        }
        if self.hovered_series().is_some() || self.hovered_volume_profile_indicator().is_some() {
            return ChartCursor::Pointer;
        }
        ChartCursor::Crosshair
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BARS: usize = 60;

    fn chart() -> ChartEngine {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        let times: Vec<f64> = (0..BARS).map(|i| 1_000.0 + i as f64 * 60.0).collect();
        let open: Vec<f64> = (0..BARS).map(|i| 100.0 + (i % 7) as f64).collect();
        let high: Vec<f64> = open.iter().map(|value| value + 3.0).collect();
        let low: Vec<f64> = open.iter().map(|value| value - 3.0).collect();
        let close: Vec<f64> = open.iter().map(|value| value + 1.0).collect();
        chart
            .set_series_data(0, &times, &open, &high, &low, &close)
            .unwrap();
        relayout(&mut chart);
        chart.fit_content();
        chart.autoscale_visible();
        chart
    }

    fn relayout(chart: &mut ChartEngine) {
        chart.recompute_layout_with_measure(true, |_, _| 48.0, |_, _| 48.0);
    }

    fn at(x: f64, y: f64) -> PointerInput {
        PointerInput {
            x,
            y,
            ..PointerInput::default()
        }
    }

    fn shifted(x: f64, y: f64) -> PointerInput {
        PointerInput {
            modifiers: InputModifiers {
                shift: true,
                ..InputModifiers::default()
            },
            ..at(x, y)
        }
    }

    fn click(chart: &mut ChartEngine, x: f64, y: f64) {
        chart.input_pointer_down(at(x, y), 1);
        chart.input_pointer_up(at(x, y));
    }

    fn drag(chart: &mut ChartEngine, from: (f64, f64), to: (f64, f64)) {
        chart.input_pointer_down(at(from.0, from.1), 1);
        for step in 1..=4 {
            let t = f64::from(step) / 4.0;
            let x = from.0 + (to.0 - from.0) * t;
            let y = from.1 + (to.1 - from.1) * t;
            chart.input_pointer_move(at(x, y), true);
        }
        chart.input_pointer_up(at(to.0, to.1));
    }

    fn empty_pane_point(chart: &ChartEngine) -> (f64, f64) {
        (40..chart.pane_w as i32)
            .step_by(17)
            .flat_map(|x| (20..chart.pane_h as i32).step_by(13).map(move |y| (x, y)))
            .map(|(x, y)| (f64::from(x), f64::from(y)))
            .find(|&(x, y)| {
                chart.hit_test_series(x, y).is_none() && chart.region_at(x, y) == ChartRegion::Pane
            })
            .expect("the pane has empty space")
    }

    fn series_point(chart: &ChartEngine) -> (f64, f64) {
        let x = chart.time_scale.index_to_coordinate(30);
        let y = chart.series_price_to_coordinate(0, 101.0).unwrap();
        assert_eq!(chart.hit_test_series(x, y), Some(0));
        (x, y)
    }

    fn volume_profile_chart() -> (ChartEngine, SeriesId, crate::NativePrimitiveId) {
        let mut chart = chart();
        let volume = chart.add_series(SeriesKind::Histogram);
        let times: Vec<f64> = (0..BARS).map(|i| 1_000.0 + i as f64 * 60.0).collect();
        let values: Vec<f64> = (0..BARS).map(|i| 10.0 + (i % 5) as f64 * 20.0).collect();
        chart
            .set_series_data(volume, &times, &values, &values, &values, &values)
            .unwrap();
        chart.series_entry_mut(volume).unwrap().visible = false;
        let id = chart
            .add_volume_profile_indicator(
                0,
                volume,
                crate::VolumeProfileIndicatorOptions::default(),
            )
            .unwrap();
        chart.build_frame();
        (chart, volume, id)
    }

    /// A point inside the profile's POC bar, clear of the crosshair action chip at the pane's
    /// right edge.
    fn volume_profile_point(chart: &mut ChartEngine, id: crate::NativePrimitiveId) -> (f64, f64) {
        let profile = chart
            .volume_profile_indicator_snapshot(id)
            .unwrap()
            .profile
            .clone();
        let poc = &profile.rows[profile.poc_index.unwrap()];
        let y = chart
            .series_price_to_coordinate(0, (poc.low + poc.high) / 2.0)
            .unwrap();
        (chart.pane_w - 60.0, y)
    }

    fn overlay_anchor_discs(chart: &mut ChartEngine) -> usize {
        use aeris_charts_core::style::DEFAULT_PRIMARY_RGB;
        use aeris_charts_render::{color::Color, draw_list::Prim};
        let border = Color::rgb(
            DEFAULT_PRIMARY_RGB.0,
            DEFAULT_PRIMARY_RGB.1,
            DEFAULT_PRIMARY_RGB.2,
        );
        chart.build_frame().panes[0]
            .main
            .iter()
            .filter(|prim| matches!(prim, Prim::Circle { fill, .. } if *fill == border))
            .count()
    }

    #[test]
    fn volume_profile_indicator_hovers_selects_and_deletes_like_other_indicators() {
        let (mut chart, _, id) = volume_profile_chart();
        let (x, y) = volume_profile_point(&mut chart, id);
        assert_eq!(chart.volume_profile_indicator_at(x, y), Some(id));

        chart.input_pointer_move(at(x, y), false);
        assert_eq!(chart.input_hover(), ChartHover::VolumeProfile(id));
        assert_eq!(chart.input_cursor(), ChartCursor::Pointer);
        assert_eq!(overlay_anchor_discs(&mut chart), 0);

        click(&mut chart, x, y);
        assert_eq!(chart.selected_volume_profile_indicator(), Some(id));
        assert_eq!(
            chart.selected_series(),
            None,
            "the profile, not its candles, is selected"
        );
        assert!(
            overlay_anchor_discs(&mut chart) >= 3,
            "a selected profile paints selection anchors"
        );

        // Selecting the source series moves the selection off the profile.
        let (sx, sy) = series_point(&chart);
        click(&mut chart, sx, sy);
        assert_eq!(chart.selected_series(), Some(0));
        assert_eq!(chart.selected_volume_profile_indicator(), None);

        click(&mut chart, x, y);
        assert!(chart.input_key_down(ChartKey::Escape, InputModifiers::default(), false, 0.0));
        assert_eq!(chart.selected_volume_profile_indicator(), None);
        assert_eq!(overlay_anchor_discs(&mut chart), 0);

        click(&mut chart, x, y);
        assert!(chart.input_key_down(ChartKey::Delete, InputModifiers::default(), false, 0.0));
        assert!(chart.volume_profile_indicator_options(id).is_none());
        assert_eq!(chart.selected_volume_profile_indicator(), None);
        assert!(
            chart
                .take_input_events()
                .iter()
                .all(|event| !matches!(event, ChartInputEvent::RemoveSeries(_))),
            "the engine owns the profile, so Delete never asks the host to remove its candles"
        );
        chart.input_pointer_move(at(x, y), false);
        assert_ne!(chart.input_hover(), ChartHover::VolumeProfile(id));
    }

    #[test]
    fn hidden_volume_profile_indicator_is_not_hittable() {
        let (mut chart, _, id) = volume_profile_chart();
        let (x, y) = volume_profile_point(&mut chart, id);
        let options = chart.volume_profile_indicator_options(id).unwrap().clone();
        assert!(chart.set_volume_profile_indicator_options(
            id,
            crate::VolumeProfileIndicatorOptions {
                visible: false,
                ..options
            }
        ));
        assert_eq!(chart.volume_profile_indicator_at(x, y), None);
    }

    #[test]
    fn host_primitive_cursor_obeys_engine_press_priority_and_clears_on_cancel() {
        let mut chart = chart();
        let (x, y) = empty_pane_point(&chart);
        chart.input_pointer_move(at(x, y), false);
        assert_eq!(chart.input_cursor(), ChartCursor::Crosshair);
        let primitive = HostPrimitiveHit {
            series: None,
            layer: HostPrimitiveLayer::Normal,
            cursor: true,
        };
        assert_eq!(
            chart.resolve_pointer_hover(x, y, Some(primitive)),
            ChartHover::HostPrimitive(primitive)
        );
        assert_eq!(chart.input_cursor(), ChartCursor::HostPrimitive);
        chart.input_pointer_down(at(x, y), 1);
        chart.input_pointer_move(at(x + 30.0, y), true);
        assert_eq!(chart.input_cursor(), ChartCursor::Grabbing);
        chart.input_cancel();
        assert_eq!(chart.input_cursor(), ChartCursor::Crosshair);
    }

    #[test]
    fn pane_drag_pans_only_past_the_shared_threshold_and_a_click_selects() {
        let mut chart = chart();
        let (x, y) = empty_pane_point(&chart);
        let start = chart.scroll_position();

        chart.input_pointer_down(at(x, y), 1);
        chart.input_pointer_move(at(x + 2.0, y), true);
        assert_eq!(
            chart.scroll_position(),
            start,
            "below the 5 px slop nothing pans"
        );
        chart.input_pointer_move(at(x + 30.0, y), true);
        chart.input_pointer_move(at(x + 60.0, y), true);
        assert_ne!(chart.scroll_position(), start);
        assert_eq!(chart.input_cursor(), ChartCursor::Grabbing);
        chart.input_pointer_up(at(x + 60.0, y));
        assert_eq!(chart.selected_series(), None, "a drag is not a click");

        let (sx, sy) = series_point(&chart);
        click(&mut chart, sx, sy);
        assert_eq!(chart.selected_series(), Some(0));
        chart.input_pointer_move(at(sx, sy), false);
        assert_eq!(chart.input_cursor(), ChartCursor::Pointer);
        let (ex, ey) = empty_pane_point(&chart);
        click(&mut chart, ex, ey);
        assert_eq!(chart.selected_series(), None);
    }

    #[test]
    fn repeated_platform_pointer_presses_resolve_double_click_without_dom_click_policy() {
        let mut chart = chart();
        let (x, y) = empty_pane_point(&chart);
        let sample = |timestamp_ms| PointerInput {
            timestamp_ms,
            ..at(x, y)
        };
        chart.input_pointer_down(sample(100.0), 1);
        chart.input_pointer_up(sample(101.0));
        assert_eq!(
            chart.take_input_events(),
            vec![ChartInputEvent::Click { x, y }]
        );
        chart.input_pointer_down(sample(200.0), 1);
        chart.input_pointer_up(sample(201.0));
        assert_eq!(
            chart.take_input_events(),
            vec![ChartInputEvent::DoubleClick { x, y }]
        );
    }

    #[test]
    fn a_second_press_at_the_same_anchor_can_drag_on_every_host() {
        let mut chart = chart();
        chart.set_drawing_tool(Some(DrawingKind::TrendLine), None, None);
        click(&mut chart, 120.0, 120.0);
        click(&mut chart, 300.0, 220.0);
        let id = chart.drawings()[0].id;
        let sample = |x, y, timestamp_ms| PointerInput {
            timestamp_ms,
            ..at(x, y)
        };
        chart.input_pointer_down(sample(120.0, 120.0, 100.0), 1);
        chart.input_pointer_up(sample(120.0, 120.0, 101.0));
        let before = chart.drawing(id).unwrap().points.clone();
        chart.input_pointer_down(sample(120.0, 120.0, 200.0), 2);
        chart.input_pointer_move(sample(150.0, 90.0, 210.0), true);
        chart.input_pointer_up(sample(150.0, 90.0, 220.0));
        assert_ne!(chart.drawing(id).unwrap().points, before);
    }

    #[test]
    fn completing_a_drawing_waits_for_pointer_motion_before_hover_promotion() {
        let mut chart = chart();
        chart.set_drawing_tool(Some(DrawingKind::TrendLine), None, None);
        click(&mut chart, 120.0, 120.0);
        click(&mut chart, 300.0, 220.0);
        let id = chart.drawings()[0].id;
        assert_ne!(chart.hovered_drawing(), Some(id));
        chart.input_pointer_move(at(300.0, 220.0), false);
        assert_eq!(chart.hovered_drawing(), Some(id));
    }

    #[test]
    fn chrome_regions_own_their_cursor_gesture_and_double_click_reset() {
        let mut chart = chart();
        chart.add_pane(true).unwrap();
        relayout(&mut chart);
        let separator = chart.panes[1].top;

        chart.input_pointer_move(at(200.0, separator), false);
        assert_eq!(chart.input_cursor(), ChartCursor::ResizeRow);
        assert_eq!(chart.separator_hover(), Some(0));
        assert_eq!(chart.crosshair, None, "the divider is chrome, not a price");
        let stretch = chart.panes[0].stretch_factor;
        drag(&mut chart, (200.0, separator), (200.0, separator + 40.0));
        assert!(chart.panes[0].stretch_factor > stretch);
        assert_eq!(chart.input.take_frame_invalidation(), (true, true));

        let time_y = chart.pane_h + 4.0;
        chart.input_pointer_move(at(200.0, time_y), false);
        assert_eq!(chart.input_cursor(), ChartCursor::ResizeHorizontal);
        let spacing = chart.bar_spacing();
        drag(&mut chart, (200.0, time_y), (120.0, time_y));
        assert_ne!(chart.bar_spacing(), spacing);

        let axis_x = chart.pane_w + 10.0;
        assert!(matches!(
            chart.region_at(axis_x, 100.0),
            ChartRegion::PriceAxis { pane: 0, .. }
        ));
        drag(&mut chart, (axis_x, 100.0), (axis_x, 160.0));
        assert_eq!(
            chart.price_scale_auto_scale_for(0, PriceScaleTarget::Right),
            Some(false)
        );
        chart.input_pointer_down(at(axis_x, 100.0), 2);
        chart.input_pointer_up(at(axis_x, 100.0));
        assert_eq!(
            chart.price_scale_auto_scale_for(0, PriceScaleTarget::Right),
            Some(true)
        );
    }

    #[test]
    fn separator_reversal_uses_the_original_grab_point_after_clamping() {
        let mut chart = chart();
        chart.add_pane(true).unwrap();
        relayout(&mut chart);
        let start = chart.panes[1].top;
        chart.input_pointer_down(at(200.0, start), 1);
        chart.input_pointer_move(at(200.0, start + 300.0), true);
        relayout(&mut chart);
        let clamped = chart.panes[1].top;
        chart.input_pointer_move(at(200.0, start + 100.0), true);
        relayout(&mut chart);
        assert!(clamped > start + 100.0);
        assert!((chart.panes[1].top - (start + 100.0)).abs() < 1e-6);
        chart.input_pointer_up(at(200.0, start + 100.0));
    }

    #[test]
    fn touch_separator_hit_uses_the_shared_hit_profile() {
        let mut chart = chart();
        chart.add_pane(true).unwrap();
        relayout(&mut chart);
        let separator = chart.panes[1].top;
        let touch = PointerInput {
            device: InputDevice::Touch,
            x: 200.0,
            y: separator + 10.0,
            ..PointerInput::default()
        };
        assert_eq!(chart.region_at(touch.x, touch.y), ChartRegion::Pane);
        chart.input_pointer_down(touch, 1);
        assert!(matches!(
            chart.input.press,
            Some(Press {
                mode: PressMode::Separator { .. },
                ..
            })
        ));
        chart.input_cancel();
    }

    #[test]
    fn drawing_tools_place_drag_and_delete_through_the_engine() {
        let mut chart = chart();
        assert!(chart.set_drawing_tool(Some(DrawingKind::TrendLine), None, None));
        let revision = chart.drawing_revision();
        click(&mut chart, 120.0, 120.0);
        chart.input_pointer_move(at(300.0, 220.0), false);
        assert_eq!(chart.input_cursor(), ChartCursor::Crosshair);
        click(&mut chart, 300.0, 220.0);
        let events = chart.take_input_events();
        let [ChartInputEvent::DrawingCreated(id)] = events[..] else {
            panic!("one drawing was created: {events:?}");
        };
        assert_eq!(chart.active_drawing_tool(), None, "one-shot tools disarm");
        assert!(chart.drawing_revision() > revision);

        let before = chart.drawing(id).unwrap().points.clone();
        chart.input_pointer_move(at(210.0, 170.0), false);
        assert_ne!(chart.input_cursor(), ChartCursor::Crosshair);
        let revision = chart.drawing_revision();
        drag(&mut chart, (210.0, 170.0), (250.0, 120.0));
        assert_ne!(chart.drawing(id).unwrap().points, before);
        assert!(chart.drawing_revision() > revision);

        chart.set_selected_drawing(Some(id));
        assert!(chart.input_key_down(ChartKey::Delete, InputModifiers::default(), false, 0.0));
        assert!(chart.drawing(id).is_none());
        assert!(chart.can_undo_drawing());
    }

    #[test]
    fn locked_drawings_select_but_never_drag() {
        let mut chart = chart();
        chart.set_drawing_tool(Some(DrawingKind::TrendLine), None, None);
        click(&mut chart, 120.0, 120.0);
        click(&mut chart, 300.0, 220.0);
        let id = chart.drawings()[0].id;
        assert!(chart.set_drawing_locked(id, true));
        let before = chart.drawing(id).unwrap().points.clone();
        drag(&mut chart, (210.0, 170.0), (250.0, 120.0));
        assert_eq!(chart.drawing(id).unwrap().points, before);
        assert_eq!(chart.selected_drawing(), Some(id));
    }

    #[test]
    fn interrupted_drawing_drag_restores_its_start() {
        let mut chart = chart();
        chart.set_drawing_tool(Some(DrawingKind::TrendLine), None, None);
        click(&mut chart, 120.0, 120.0);
        click(&mut chart, 300.0, 220.0);
        let id = chart.drawings()[0].id;
        let before = chart.drawing(id).unwrap().points.clone();
        chart.input_pointer_down(at(210.0, 170.0), 1);
        chart.input_pointer_move(at(260.0, 100.0), true);
        chart.input_cancel();
        assert_eq!(chart.drawing(id).unwrap().points, before);
        assert_eq!(chart.input_cursor(), ChartCursor::Crosshair);
    }

    #[test]
    fn motion_after_a_lost_release_abandons_the_gesture_and_hovers() {
        let mut chart = chart();
        chart.set_drawing_tool(Some(DrawingKind::TrendLine), None, None);
        click(&mut chart, 120.0, 120.0);
        click(&mut chart, 300.0, 220.0);
        let id = chart.drawings()[0].id;
        let before = chart.drawing(id).unwrap().points.clone();
        chart.input_pointer_down(at(210.0, 170.0), 1);
        chart.input_pointer_move(at(260.0, 100.0), true);
        // The release happened outside the window; the next sample arrives with no button held.
        chart.input_pointer_move(at(400.0, 300.0), false);
        assert_eq!(chart.drawing(id).unwrap().points, before);
        assert!(!chart.drawing_drag_active());
        assert_eq!(chart.crosshair, Some((400.0, 300.0)));
        assert_ne!(chart.input_cursor(), ChartCursor::Grabbing);
    }

    #[test]
    fn a_foreign_pointer_cannot_move_release_or_cancel_the_captured_press() {
        let mut chart = chart();
        let start = chart.scroll_position();
        chart.input_pointer_down(
            PointerInput {
                id: 7,
                ..at(200.0, 200.0)
            },
            1,
        );
        chart.input_pointer_move(
            PointerInput {
                id: 9,
                ..at(230.0, 200.0)
            },
            false,
        );
        chart.input_pointer_up(PointerInput {
            id: 9,
            ..at(230.0, 200.0)
        });
        chart.input_cancel_pointer(9);
        chart.input_pointer_move(
            PointerInput {
                id: 7,
                ..at(220.0, 200.0)
            },
            true,
        );
        chart.input_pointer_move(
            PointerInput {
                id: 7,
                ..at(250.0, 200.0)
            },
            true,
        );
        assert_ne!(chart.scroll_position(), start);
        chart.input_cancel_pointer(7);
        assert_ne!(chart.input_cursor(), ChartCursor::Grabbing);
    }

    #[test]
    fn keyboard_bindings_navigate_and_escape_disarms() {
        let mut chart = chart();
        let none = InputModifiers::default();
        chart.set_drawing_tool(Some(DrawingKind::Rectangle), None, None);
        assert!(chart.input_key_down(ChartKey::Escape, none, false, 0.0));
        assert_eq!(chart.active_drawing_tool(), None);

        let start = chart.scroll_position();
        assert!(chart.input_key_down(ChartKey::PageUp, none, false, 0.0));
        assert!(chart.scroll_position() < start);
        let older = chart.scroll_position();
        assert!(chart.input_key_down(ChartKey::PageDown, none, false, 0.0));
        assert!(chart.scroll_position() > older);
        assert!(chart.input_key_down(ChartKey::End, none, false, 0.0));
        assert!(chart.is_at_latest());

        assert!(chart.input_key_down(ChartKey::ArrowLeft, none, false, 0.0));
        assert!(chart.input_animating());
        chart.input_tick(500.0);
        assert!(chart.scroll_position() < 0.0);
        assert!(chart.input_key_up(ChartKey::ArrowLeft));
        assert!(!chart.input_animating());

        let spacing = chart.bar_spacing();
        assert!(chart.input_key_down(ChartKey::ZoomIn, none, false, 0.0));
        assert!(chart.bar_spacing() > spacing);
        let zoomed = chart.bar_spacing();
        assert!(chart.input_key_down(ChartKey::ZoomOut, none, false, 0.0));
        assert!(chart.bar_spacing() < zoomed);
        chart.set_bar_spacing(20.0);
        chart.set_price_scale_visible_range_for(0, PriceScaleTarget::Right, 80.0, 120.0);
        assert_eq!(
            chart.price_scale_auto_scale_for(0, PriceScaleTarget::Right),
            Some(false)
        );
        assert!(chart.input_key_down(ChartKey::Home, none, false, 0.0));
        assert_eq!(chart.bar_spacing(), 6.0);
        assert_eq!(
            chart.price_scale_auto_scale_for(0, PriceScaleTarget::Right),
            Some(true),
            "Home resets the price scales as well as the time scale"
        );
        for key in [ChartKey::ArrowUp, ChartKey::ArrowDown, ChartKey::Tab] {
            assert!(
                !chart.input_key_down(key, none, false, 0.0),
                "{key:?} is bound only on focused targets"
            );
        }
    }

    #[test]
    fn escape_cancels_every_transient_interaction_and_reports_the_crosshair_leaving() {
        let mut chart = chart();
        let none = InputModifiers::default();
        chart.set_drawing_tool(Some(DrawingKind::TrendLine), None, None);
        click(&mut chart, 120.0, 120.0);
        click(&mut chart, 300.0, 220.0);
        let id = chart.drawings()[0].id;
        chart.set_selected_drawing(Some(id));
        let (x, y) = series_point(&chart);
        chart.input_pointer_move(at(x, y), false);
        assert_eq!(chart.hovered_series(), Some(0));
        assert!(chart.crosshair.is_some());
        chart.set_drawing_tool(Some(DrawingKind::Rectangle), None, None);
        chart.take_input_events();

        assert!(chart.input_key_down(ChartKey::Escape, none, false, 0.0));
        assert_eq!(chart.active_drawing_tool(), None);
        assert_eq!(chart.selected_drawing(), None);
        assert_eq!(chart.hovered_series(), None);
        assert_eq!(chart.input_hover(), ChartHover::None);
        assert_eq!(chart.crosshair, None);
        assert!(chart.trading_preview().is_none());
        assert_eq!(
            chart.take_input_events(),
            vec![ChartInputEvent::CrosshairLeft]
        );
        assert!(chart.drawing(id).is_some(), "Escape never deletes");
    }

    #[test]
    fn a_double_click_on_a_trading_order_closes_the_drag_its_second_press_opened() {
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
        let snapshot: TradingSnapshot = serde_json::from_str(
            r#"{"instrument":{"tick_size":0.25},
                "positions":[{"id":"position-1","pane_index":0,"price_scale":"right",
                    "side":"long","average_price":101.0,"quantity":12.0}],
                "orders":[{"id":"tp-1","pane_index":0,"price_scale":"right","side":"sell",
                    "kind":"limit","role":"take_profit","status":"working","price":103.0,
                    "quantity":12.0,"filled_quantity":0.0,"position_id":"position-1",
                    "revision":1}]}"#,
        )
        .unwrap();
        chart.set_trading_snapshot(snapshot).unwrap();
        chart.build_frame();
        let x = chart.trading_marker_start() + 20.0;
        let y = chart
            .trading_price_coordinate(0, TradingPriceScale::Right, 103.0)
            .unwrap();
        // The platform reports the second press of a double-click directly.
        chart.input_pointer_down(at(x, y), 2);
        assert_eq!(
            chart.input.press.map(|press| press.mode),
            Some(PressMode::Trading { dragging: true })
        );
        chart.input_pointer_up(at(x, y));
        assert!(chart.input.press.is_none());
        assert!(
            chart.trading_preview().is_none(),
            "the order drag must not outlive the double-click"
        );
        assert!(chart.take_trading_intents().is_empty());
        chart.input_pointer_move(at(x, y), false);
        assert_ne!(chart.input_cursor(), ChartCursor::VerticalGrabbing);
        assert!(chart.trading_drag_start_at(x, y), "a fresh drag can start");
    }

    #[test]
    fn a_double_click_closes_measure_and_pan_sessions() {
        let mut chart = chart();
        let (x, y) = empty_pane_point(&chart);
        chart.input_pointer_down(shifted(x, y), 1);
        chart.input_pointer_up(shifted(x, y));
        assert!(chart.measure_active());
        chart.input_pointer_down(shifted(x, y), 2);
        chart.input_pointer_up(shifted(x, y));
        assert!(chart.input.press.is_none());

        let start = chart.scroll_position();
        chart.input_pointer_down(at(x, y), 1);
        chart.input_pointer_up(at(x, y));
        chart.input_pointer_down(at(x, y), 2);
        chart.input_pointer_up(at(x, y));
        drag(&mut chart, (x, y), (x + 40.0, y));
        assert_ne!(
            chart.scroll_position(),
            start,
            "the next pan starts from a closed session"
        );
    }

    fn area_chart_with_delta_tooltip() -> (ChartEngine, NativePrimitiveId) {
        let mut chart = chart();
        chart.series[0].kind = SeriesKind::Area;
        let primitive = chart
            .add_delta_tooltip(0, DeltaTooltipOptions::default())
            .unwrap();
        (chart, primitive)
    }

    #[test]
    fn two_touches_commit_a_delta_tooltip_range_inside_the_controller() {
        let (mut chart, primitive) = area_chart_with_delta_tooltip();
        let (_, y) = empty_pane_point(&chart);
        let touch = |id, x, timestamp_ms| PointerInput {
            id,
            device: InputDevice::Touch,
            x,
            y,
            timestamp_ms,
            ..PointerInput::default()
        };
        let x10 = chart.time_scale.index_to_coordinate(10);
        let x40 = chart.time_scale.index_to_coordinate(40);
        chart.input_pointer_down(touch(1, x10, 10.0), 1);
        chart.input_pointer_down(touch(2, x40, 20.0), 1);
        assert_eq!(chart.delta_tooltip_active_range(primitive), None);
        chart.take_input_events();
        chart.input_pointer_move(touch(2, x40, 30.0), true);
        assert!(
            chart.delta_tooltip_active_range(primitive).is_some(),
            "two touches commit a comparison without a host bridge"
        );
        assert_eq!(
            chart.take_input_events(),
            vec![ChartInputEvent::DeltaTooltipChanged]
        );
        chart.input_pointer_up(touch(2, x40, 40.0));
        chart.input_pointer_up(touch(1, x10, 50.0));
        assert!(chart.delta_tooltip_active_range(primitive).is_some());
    }

    #[test]
    fn mouse_delta_tooltip_changes_queue_one_notice_per_drain() {
        let (mut chart, primitive) = area_chart_with_delta_tooltip();
        let (_, y) = empty_pane_point(&chart);
        let x10 = chart.time_scale.index_to_coordinate(10);
        let x40 = chart.time_scale.index_to_coordinate(40);
        chart.input_pointer_down(at(x10, y), 1);
        chart.input_pointer_move(at(x10 + 20.0, y), true);
        chart.input_pointer_move(at(x40, y), true);
        chart.input_pointer_up(at(x40, y));
        assert!(chart.delta_tooltip_active_range(primitive).is_some());
        let events = chart.take_input_events();
        assert_eq!(
            events
                .iter()
                .filter(|event| **event == ChartInputEvent::DeltaTooltipChanged)
                .count(),
            1,
            "{events:?}"
        );
    }

    #[test]
    fn hover_arbitration_ranks_host_primitives_against_drawings_and_series() {
        let mut chart = chart();
        let (x, y) = series_point(&chart);
        chart.input_pointer_move(at(x, y), false);
        assert_eq!(chart.input_hover(), ChartHover::Series(0));
        let hit = |series, layer| HostPrimitiveHit {
            series,
            layer,
            cursor: false,
        };
        let top = hit(None, HostPrimitiveLayer::Top);
        assert_eq!(
            chart.resolve_pointer_hover(x, y, Some(top)),
            ChartHover::HostPrimitive(top)
        );
        assert_eq!(chart.hovered_series(), None);
        let own = hit(Some(0), HostPrimitiveLayer::Normal);
        assert_eq!(
            chart.resolve_pointer_hover(x, y, Some(own)),
            ChartHover::HostPrimitive(own),
            "a series' normal primitive blocks its own built-in hit"
        );
        assert_eq!(chart.hovered_series(), Some(0));
        let bottom = hit(None, HostPrimitiveLayer::Bottom);
        assert_eq!(
            chart.resolve_pointer_hover(x, y, Some(bottom)),
            ChartHover::Series(0),
            "a bottom primitive survives only without a series hit"
        );
        let (ex, ey) = empty_pane_point(&chart);
        chart.input_pointer_move(at(ex, ey), false);
        assert_eq!(chart.input_hover(), ChartHover::None);
        assert_eq!(
            chart.resolve_pointer_hover(ex, ey, Some(bottom)),
            ChartHover::HostPrimitive(bottom)
        );
        chart.clear_hover();
        assert_eq!(chart.input_hover(), ChartHover::None);
    }

    #[test]
    fn focused_axis_and_separator_targets_own_their_key_bindings() {
        let mut chart = chart();
        chart.add_pane(true).unwrap();
        relayout(&mut chart);
        let none = InputModifiers::default();
        let price = ChartFocusTarget::PriceAxis {
            pane: 0,
            target: PriceScaleTarget::Right,
        };
        chart.set_price_scale_visible_range_for(0, PriceScaleTarget::Right, 90.0, 110.0);
        assert!(chart.input_target_key_down(price, ChartKey::ArrowUp, none));
        let (from, to) = chart
            .price_scale_visible_range_for(0, PriceScaleTarget::Right)
            .unwrap();
        assert!((to - from - 19.0).abs() < 1e-9, "{from}..{to}");
        assert!(((from + to) / 2.0 - 100.0).abs() < 1e-9);
        assert!(chart.input_target_key_down(price, ChartKey::ArrowDown, none));
        let (from, to) = chart
            .price_scale_visible_range_for(0, PriceScaleTarget::Right)
            .unwrap();
        assert!((to - from - 19.0 * 1.05).abs() < 1e-9);
        assert!(chart.input_target_key_down(price, ChartKey::Home, none));
        assert_eq!(
            chart.price_scale_auto_scale_for(0, PriceScaleTarget::Right),
            Some(true)
        );
        assert!(!chart.input_target_key_down(price, ChartKey::PageUp, none));

        let start = chart.scroll_position();
        assert!(chart.input_target_key_down(ChartFocusTarget::TimeAxis, ChartKey::ArrowLeft, none));
        assert_eq!(chart.scroll_position(), start - 1.0);
        assert!(!chart.input_animating(), "a focused step is discrete");
        chart.set_bar_spacing(20.0);
        assert!(chart.input_target_key_down(ChartFocusTarget::TimeAxis, ChartKey::Home, none));
        assert_eq!(chart.bar_spacing(), 6.0);

        let separator = ChartFocusTarget::Separator(0);
        let height = chart.panes[0].height;
        assert!(chart.input_target_key_down(separator, ChartKey::ArrowUp, none));
        assert_eq!(chart.input.take_frame_invalidation(), (true, true));
        relayout(&mut chart);
        assert!((chart.panes[0].height - (height - 10.0)).abs() < 1.0);
        assert!(chart.input_target_key_down(separator, ChartKey::Home, none));
        relayout(&mut chart);
        assert!((chart.panes[0].height - chart.panes[1].height).abs() < 1.0);
        assert!(!chart.input_target_key_down(ChartFocusTarget::Separator(1), ChartKey::Home, none));
    }

    #[test]
    fn focused_drawing_edits_commit_atomically_or_cancel_to_their_start() {
        let mut chart = chart();
        let none = InputModifiers::default();
        chart.set_drawing_tool(Some(DrawingKind::TrendLine), None, None);
        click(&mut chart, 120.0, 120.0);
        click(&mut chart, 300.0, 220.0);
        let id = chart.drawings()[0].id;
        let target = ChartFocusTarget::Drawing(id);
        let original = chart.drawing(id).unwrap().points.clone();

        assert!(
            !chart.input_target_key_down(target, ChartKey::ArrowRight, none),
            "arrows nudge only inside an edit"
        );
        assert_eq!(
            chart.selected_drawing(),
            Some(id),
            "focus selects the drawing"
        );
        assert!(chart.input_target_key_down(target, ChartKey::Enter, none));
        assert_eq!(chart.drawing_edit_session(), Some((id, None)));
        assert!(chart.input_target_key_down(target, ChartKey::ArrowRight, none));
        assert!(chart.input_target_key_down(target, ChartKey::ArrowDown, none));
        assert_ne!(chart.drawing(id).unwrap().points, original);
        assert!(chart.input_target_key_down(target, ChartKey::Escape, none));
        assert_eq!(chart.drawing_edit_session(), None);
        assert_eq!(chart.drawing(id).unwrap().points, original);

        assert!(chart.input_target_key_down(target, ChartKey::Enter, none));
        assert!(chart.input_target_key_down(target, ChartKey::Tab, none));
        assert_eq!(chart.drawing_edit_session(), Some((id, Some(0))));
        let shift = InputModifiers {
            shift: true,
            ..InputModifiers::default()
        };
        assert!(chart.input_target_key_down(target, ChartKey::Tab, shift));
        assert_eq!(chart.drawing_edit_session(), Some((id, Some(1))));
        assert!(chart.input_target_key_down(target, ChartKey::Tab, none));
        assert!(chart.input_target_key_down(target, ChartKey::ArrowRight, shift));
        assert!(chart.input_target_key_down(target, ChartKey::ArrowRight, shift));
        let edited = chart.drawing(id).unwrap().points.clone();
        assert_eq!(edited[1], original[1], "only the focused anchor moves");
        assert_ne!(edited[0], original[0]);
        assert!(chart.input_target_key_down(target, ChartKey::Enter, none));
        assert_eq!(chart.drawing_edit_session(), None);
        assert!(chart.undo_drawing());
        assert_eq!(
            chart.drawing(id).unwrap().points,
            original,
            "one undo reverts the whole committed edit"
        );

        assert!(chart.input_target_key_down(target, ChartKey::Delete, none));
        assert!(chart.drawing(id).is_none());
        assert!(!chart.input_target_key_down(target, ChartKey::Enter, none));
    }

    #[test]
    fn keyboard_sequence_backspace_and_enter_share_drawing_creation() {
        let mut chart = chart();
        let none = InputModifiers::default();
        assert!(chart.set_drawing_tool(Some(DrawingKind::Path), None, None));
        chart.drawing_tool_activate(120.0, 120.0, DrawingModifiers::default());
        chart.drawing_tool_activate(200.0, 160.0, DrawingModifiers::default());
        assert!(chart.drawing_create_active());
        assert!(chart.input_key_down(ChartKey::Backspace, none, false, 0.0));
        assert!(chart.drawing_create_active());
        chart.drawing_tool_activate(240.0, 200.0, DrawingModifiers::default());
        assert!(chart.input_key_down(ChartKey::Enter, none, false, 0.0));
        let [ChartInputEvent::DrawingCreated(id)] = chart.take_input_events()[..] else {
            panic!("Enter must report the committed path");
        };
        assert!(chart.drawing(id).is_some());
    }

    #[test]
    fn keyboard_delete_requests_host_owned_series_removal() {
        let mut chart = chart();
        chart.set_selected_series(Some(0));
        assert!(chart.input_key_down(ChartKey::Delete, InputModifiers::default(), false, 0.0));
        assert_eq!(
            chart.take_input_events(),
            vec![ChartInputEvent::RemoveSeries(0)]
        );
    }

    #[test]
    fn keyboard_undo_and_redo_share_the_drawing_history() {
        let mut chart = chart();
        let none = InputModifiers::default();
        assert!(chart.set_drawing_tool(Some(DrawingKind::Rectangle), None, None));
        click(&mut chart, 120.0, 120.0);
        click(&mut chart, 300.0, 220.0);
        let id = chart.drawings()[0].id;
        assert!(chart.input_key_down(ChartKey::Undo, none, false, 0.0));
        assert!(chart.drawing(id).is_none());
        assert!(chart.input_key_down(ChartKey::Redo, none, false, 0.0));
        assert!(chart.drawing(id).is_some());
    }

    #[test]
    fn reduced_motion_uses_discrete_arrow_steps_without_animation() {
        let mut chart = chart();
        let mut options = chart.interaction_options();
        options.reduced_motion = true;
        chart.set_interaction_options(options);
        let start = chart.scroll_position();
        assert!(chart.input_key_down(ChartKey::ArrowRight, InputModifiers::default(), false, 0.0));
        assert_eq!(chart.scroll_position(), start + 1.0);
        assert!(!chart.input_animating());
        assert!(chart.input_key_down(ChartKey::ArrowRight, InputModifiers::default(), true, 25.0));
        assert_eq!(chart.scroll_position(), start + 2.0);
        assert!(!chart.input_animating());
    }

    #[test]
    fn wheel_routing_follows_interaction_options() {
        let mut chart = chart();
        let wheel = |x, y, delta_y| WheelSample {
            x,
            y,
            delta_y,
            ..WheelSample::default()
        };
        let spacing = chart.bar_spacing();
        assert!(chart.input_wheel(wheel(300.0, 100.0, 1.0)));
        assert_ne!(chart.bar_spacing(), spacing);

        let axis_x = chart.pane_w + 10.0;
        let range = chart.price_scale_visible_range_for(0, PriceScaleTarget::Right);
        let spacing = chart.bar_spacing();
        assert!(chart.input_wheel(wheel(axis_x, 100.0, 1.0)));
        assert_ne!(
            chart.bar_spacing(),
            spacing,
            "auto mode zooms time everywhere"
        );
        assert_eq!(
            chart.price_scale_visible_range_for(0, PriceScaleTarget::Right),
            range
        );

        chart.set_interaction_options(InteractionOptions {
            price_axis_wheel_zoom: true,
            ..InteractionOptions::default()
        });
        assert!(chart.input_wheel(wheel(axis_x, 100.0, 1.0)));
        assert_ne!(
            chart.price_scale_visible_range_for(0, PriceScaleTarget::Right),
            range
        );

        chart.set_interaction_options(InteractionOptions {
            wheel_zoom: false,
            ..InteractionOptions::default()
        });
        assert!(!chart.input_wheel(wheel(300.0, 100.0, 1.0)));
        let spacing = chart.bar_spacing();
        assert!(chart.input_pinch(300.0, 100.0, 0.1, 0.0));
        assert_ne!(chart.bar_spacing(), spacing);
        chart.set_interaction_options(InteractionOptions {
            wheel_zoom: false,
            pinch_zoom: false,
            ..InteractionOptions::default()
        });
        assert!(!chart.input_pinch(300.0, 100.0, 0.1, 0.0));
    }

    #[test]
    fn consumed_wheel_promotes_its_position_to_the_input_cursor() {
        let mut chart = chart();
        chart.input_pointer_move(at(300.0, 100.0), false);
        let axis_x = chart.pane_w + 10.0;
        assert!(chart.input_wheel(WheelSample {
            x: axis_x,
            y: 100.0,
            delta_y: 1.0,
            ..WheelSample::default()
        }));
        assert_eq!(chart.input_cursor(), ChartCursor::ResizeVertical);
    }

    #[test]
    fn touch_pinch_keeps_membership_and_rebases_the_primary_finger() {
        let mut chart = chart();
        let (x, y) = empty_pane_point(&chart);
        let touch = |id, x, timestamp_ms| PointerInput {
            id,
            device: InputDevice::Touch,
            x,
            y,
            timestamp_ms,
            ..PointerInput::default()
        };
        let spacing = chart.bar_spacing();
        chart.input_pointer_down(touch(1, x, 10.0), 1);
        chart.input_pointer_down(touch(2, x + 60.0, 20.0), 1);
        chart.input_pointer_down(touch(3, x + 120.0, 25.0), 1);
        chart.input_cancel_pointer(3);
        chart.input_pointer_move(touch(2, x + 90.0, 30.0), true);
        assert!(
            chart.bar_spacing() > spacing,
            "two touch members zoom together"
        );
        chart.input_pointer_up(touch(2, x + 90.0, 40.0));
        let position = chart.scroll_position();
        chart.input_pointer_move(touch(1, x + 20.0, 50.0), true);
        chart.input_pointer_move(touch(1, x + 40.0, 60.0), true);
        assert_ne!(
            chart.scroll_position(),
            position,
            "primary touch continues as a pan"
        );
        chart.input_pointer_up(touch(1, x + 40.0, 70.0));
        assert!(
            chart.take_input_events().is_empty(),
            "pinch release is never a click"
        );
    }

    #[test]
    fn touch_long_press_tracks_crosshair_and_exits_on_the_next_tap() {
        let mut chart = chart();
        let (x, y) = empty_pane_point(&chart);
        let touch = |id, x, timestamp_ms| PointerInput {
            id,
            device: InputDevice::Touch,
            x,
            y,
            timestamp_ms,
            ..PointerInput::default()
        };
        chart.input_pointer_down(touch(1, x, 100.0), 1);
        assert!(chart.input_touch_page_scroll_candidate(1));
        assert_eq!(chart.input_wake_deadline_ms(), Some(340.0));
        assert!(!chart.input_tick(339.0));
        assert!(chart.input_tick(340.0));
        assert!(!chart.input_touch_page_scroll_candidate(1));
        let scroll = chart.scroll_position();
        chart.input_pointer_move(touch(1, x + 20.0, 350.0), true);
        assert_eq!(chart.scroll_position(), scroll);
        assert_eq!(chart.crosshair, Some((x + 20.0, y)));
        chart.input_pointer_up(touch(1, x + 20.0, 360.0));
        assert_eq!(chart.crosshair, Some((x + 20.0, y)));
        assert!(chart.take_input_events().is_empty());
        chart.input_pointer_down(touch(2, x + 30.0, 400.0), 1);
        chart.input_pointer_up(touch(2, x + 30.0, 410.0));
        assert_eq!(chart.crosshair, None);
        assert!(chart.take_input_events().is_empty());
    }

    #[test]
    fn touch_tracking_exit_on_end_and_drag_cancellation_use_engine_policy() {
        let mut chart = chart();
        let (x, y) = empty_pane_point(&chart);
        chart.set_interaction_options(InteractionOptions {
            touch_tracking_exit_on_end: true,
            ..chart.interaction_options()
        });
        let touch = |x, timestamp_ms| PointerInput {
            id: 7,
            device: InputDevice::Touch,
            x,
            y,
            timestamp_ms,
            ..PointerInput::default()
        };
        chart.input_pointer_down(touch(x, 100.0), 1);
        chart.input_pointer_move(touch(x + 8.0, 120.0), true);
        assert_eq!(chart.input_wake_deadline_ms(), None);
        assert!(!chart.input_tick(340.0));
        chart.input_pointer_up(touch(x + 8.0, 350.0));
        chart.input_pointer_down(touch(x, 400.0), 1);
        assert!(chart.input_tick(640.0));
        chart.input_pointer_up(touch(x, 650.0));
        assert_eq!(chart.crosshair, None);
    }

    #[test]
    fn wheel_zoom_pins_the_right_edge_while_modifiers_and_pinch_zoom_at_the_pointer() {
        let mut chart = chart();
        let cursor = 300.0;
        let wheel = |delta_y, modifiers| WheelSample {
            x: cursor,
            y: 100.0,
            delta_y,
            modifiers,
            ..WheelSample::default()
        };
        // Plain wheel (measured TradingView behavior): exact 10% steps with the right offset
        // in bars (the gap after the latest bar) unchanged, wherever the pointer is.
        let spacing = chart.bar_spacing();
        let offset = chart.time_scale.right_offset();
        assert!(chart.input_wheel(wheel(-1.0, InputModifiers::default())));
        assert!((chart.bar_spacing() - spacing * 0.9).abs() < 1e-9);
        assert_eq!(chart.time_scale.right_offset(), offset);

        // Ctrl (and macOS Cmd) wheel and pinch keep the point under the pointer fixed.
        for modifiers in [
            InputModifiers {
                control: true,
                ..InputModifiers::default()
            },
            InputModifiers {
                meta: true,
                ..InputModifiers::default()
            },
        ] {
            let before = chart.time_scale.coordinate_to_float_index(cursor);
            assert!(chart.input_wheel(wheel(1.0, modifiers)));
            let after = chart.time_scale.coordinate_to_float_index(cursor);
            assert!(
                (before - after).abs() < 1e-6,
                "{modifiers:?}: {before} -> {after}"
            );
        }
        let before = chart.time_scale.coordinate_to_float_index(cursor);
        assert!(chart.input_pinch(cursor, 100.0, 0.1, 0.0));
        let after = chart.time_scale.coordinate_to_float_index(cursor);
        assert!((before - after).abs() < 1e-6, "pinch: {before} -> {after}");

        chart.set_interaction_options(InteractionOptions {
            wheel_zoom: false,
            pinch_zoom: false,
            ..InteractionOptions::default()
        });
        assert!(!chart.input_pinch(cursor, 100.0, 0.1, 0.0));
    }

    #[test]
    fn context_menu_reports_region_and_pane_context() {
        let mut chart = chart();
        let (x, y) = series_point(&chart);
        chart.input_context_menu(x, y);
        let axis_x = chart.pane_w + 10.0;
        chart.input_context_menu(axis_x, 100.0);
        let events = chart.take_input_events();
        let [ChartInputEvent::ContextMenu(pane), ChartInputEvent::ContextMenu(axis)] = events[..]
        else {
            panic!("two context menus: {events:?}");
        };
        assert_eq!(pane.region, ChartRegion::Pane);
        assert_eq!(pane.context.and_then(|context| context.series), Some(0));
        assert!(matches!(
            axis.region,
            ChartRegion::PriceAxis { pane: 0, .. }
        ));
        assert_eq!(axis.context, None);
    }

    #[test]
    fn brushable_area_styles_its_range_and_escape_clears_it() {
        let mut chart = chart();
        chart.series[0].kind = SeriesKind::Area;
        assert!(chart.set_brushable_area(
            0,
            Some(DeltaTooltipOptions {
                requires_shift_drag: true,
                ..DeltaTooltipOptions::default()
            })
        ));
        let (x, y) = empty_pane_point(&chart);
        let start = chart.scroll_position();
        chart.input_pointer_down(shifted(x, y), 1);
        chart.input_pointer_move(shifted(x + 80.0, y), true);
        chart.input_pointer_up(shifted(x + 80.0, y));
        assert_eq!(chart.scroll_position(), start, "the brush owns the drag");
        assert!(chart.series_entry(0).unwrap().area_brush.is_some());

        chart.input_pointer_down(at(x, y), 2);
        chart.input_pointer_up(at(x, y));
        assert!(chart.series_entry(0).unwrap().area_brush.is_none());

        chart.input_pointer_down(shifted(x, y), 1);
        chart.input_pointer_move(shifted(x + 80.0, y), true);
        chart.input_pointer_up(shifted(x + 80.0, y));
        assert!(chart.series_entry(0).unwrap().area_brush.is_some());

        let none = InputModifiers::default();
        assert!(chart.input_key_down(ChartKey::Escape, none, false, 0.0));
        assert!(chart.series_entry(0).unwrap().area_brush.is_none());
        assert!(chart.set_brushable_area(0, None));
        assert!(!chart.is_brushable_area(0));
    }

    #[test]
    fn brushable_area_applies_explicit_styles_inside_engine() {
        let mut chart = chart();
        chart.series[0].kind = SeriesKind::Area;
        let red = Color::rgba(210, 30, 40, 255);
        let blue = Color::rgba(20, 60, 220, 255);
        let green = Color::rgba(20, 180, 70, 255);
        assert!(chart.set_brushable_area_with_styles(
            0,
            Some(DeltaTooltipOptions::default()),
            AreaBrushOverrides {
                outside: BrushStyleOverride {
                    line_color: Some(red),
                    line_width: Some(3.5),
                    ..Default::default()
                },
                positive: BrushStyleOverride {
                    line_color: Some(blue),
                    top_color: Some(green),
                    ..Default::default()
                },
                negative: BrushStyleOverride {
                    line_color: Some(green),
                    ..Default::default()
                },
            },
        ));
        let (x, y) = empty_pane_point(&chart);
        chart.input_pointer_down(shifted(x, y), 1);
        chart.input_pointer_move(shifted(x + 80.0, y), true);
        chart.input_pointer_up(shifted(x + 80.0, y));
        let brush = chart.series_entry(0).unwrap().area_brush.as_ref().unwrap();
        assert_eq!(brush.outside.line_color, red);
        assert_eq!(brush.outside.line_width, 3.5);
        if chart.brushable_area_range(0).unwrap().positive {
            assert_eq!(brush.ranges[0].style.line_color, blue);
            assert_eq!(brush.ranges[0].style.top_color, green);
        } else {
            assert_eq!(brush.ranges[0].style.line_color, green);
        }
    }

    #[test]
    fn frame_preparation_rebuilds_after_any_input_or_invalidation_and_only_then() {
        let mut chart = chart();
        let mut frame = ChartFrame::default();
        let mut axis = Vec::new();
        let mut prepare = |chart: &mut ChartEngine| {
            chart
                .prepare_financial_frame_with_measure(
                    FinancialFrameRequest {
                        width: 800.0,
                        height: 500.0,
                        dpr: 1.0,
                        force_layout: false,
                        allow_axis_shrink: false,
                        force_frame: false,
                        force_axis: false,
                        layout_only: false,
                        fit_content: false,
                        frame: &mut frame,
                        axis_frame: None,
                        axis_primitives: Some(&mut axis),
                    },
                    |_, _| 48.0,
                    |_, _| 48.0,
                )
                .frame_built
        };
        assert!(prepare(&mut chart));
        assert!(!prepare(&mut chart), "an unchanged chart reuses its frame");

        let (x, y) = series_point(&chart);
        chart.input_pointer_move(at(x, y), false);
        assert!(chart.frame_pending());
        assert!(prepare(&mut chart), "pointer hover rebuilds");
        assert!(!chart.frame_pending());
        assert!(!prepare(&mut chart));
        chart.input_pointer_move(at(x, y), false);
        assert!(
            !chart.frame_pending(),
            "an identical hover sample changes nothing"
        );
        assert!(!prepare(&mut chart));

        chart.set_hovered_series(None);
        assert!(prepare(&mut chart), "an overlay-only invalidation rebuilds");
        assert!(!prepare(&mut chart));
    }

    #[test]
    fn interaction_defaults_match_the_browser_host() {
        let options = InteractionOptions::default();
        assert!(options.pan && options.wheel_scroll && options.wheel_zoom && options.pinch_zoom);
        assert_eq!(options.wheel_behavior, WheelBehavior::Auto);
        assert!(options.axis_double_click_reset_time && options.axis_double_click_reset_price);
        assert!(options.axis_scale_price && options.axis_scale_time && options.panes_resize);
        assert!(!options.kinetic_mouse && !options.price_axis_wheel_zoom);
    }

    /// Native platforms may deliver motion per HID report (Wayland: often one axis per event). A
    /// captured freehand stream forwards only the newest sample per frame, so device cadence never
    /// becomes stroke knots.
    #[test]
    fn captured_freehand_motion_coalesces_to_one_knot_per_frame() {
        let mut chart = chart();
        chart.set_drawing_tool(Some(DrawingKind::Brush), None, None);
        chart.input_pointer_down(at(100.0, 100.0), 1);
        for (x, y) in [
            (101.5, 100.0),
            (101.5, 101.5),
            (103.0, 101.5),
            (103.0, 103.0),
        ] {
            chart.input_pointer_move(at(x, y), true);
        }
        assert!(
            chart.flush_coalesced_input(),
            "the newest sample becomes a knot"
        );
        assert!(
            !chart.flush_coalesced_input(),
            "an idle frame captures nothing"
        );
        chart.input_pointer_up(at(103.0, 103.0));
        let events = chart.take_input_events();
        let [ChartInputEvent::DrawingCreated(id)] = events[..] else {
            panic!("the brush commits: {events:?}");
        };
        assert_eq!(chart.drawing(id).unwrap().points.len(), 2);
    }

    #[test]
    fn a_click_after_a_pan_keeps_its_position_and_the_next_pan_starts_there() {
        let mut chart = chart();
        let (x, y) = empty_pane_point(&chart);
        let initial = chart.scroll_position();
        drag(&mut chart, (x, y), (x + 40.0, y));
        let after_pan = chart.scroll_position();
        let first_delta = after_pan - initial;
        assert!(first_delta != 0.0);
        click(&mut chart, x, y);
        assert_eq!(chart.scroll_position(), after_pan);
        drag(&mut chart, (x, y), (x + 40.0, y));
        assert!((chart.scroll_position() - (after_pan + first_delta)).abs() < 1e-9);
    }

    #[test]
    fn leaving_clears_hover_but_keeps_a_captured_drag() {
        let mut chart = chart();
        let (x, y) = series_point(&chart);
        chart.input_pointer_move(at(x, y), false);
        assert!(chart.crosshair.is_some());
        assert_eq!(chart.hovered_series(), Some(0));
        chart.input_pointer_leave();
        assert_eq!(chart.crosshair, None);
        assert_eq!(chart.hovered_series(), None);

        chart.input_pointer_down(at(x, y), 1);
        chart.input_pointer_move(at(x + 40.0, y), true);
        chart.input_pointer_leave();
        assert_eq!(chart.input_cursor(), ChartCursor::Grabbing);
        chart.input_pointer_up(at(x + 40.0, y));
    }

    /// A trend line from bar 10 to bar 40 and the point halfway along its body.
    fn trend_line_body(chart: &mut ChartEngine) -> (DrawingId, (f64, f64)) {
        let id = chart
            .add_drawing(
                DrawingKind::TrendLine,
                0,
                vec![
                    DrawingPoint {
                        logical: 10.0,
                        price: 102.0,
                    },
                    DrawingPoint {
                        logical: 40.0,
                        price: 104.0,
                    },
                ],
                None,
            )
            .unwrap();
        chart.build_frame();
        let a = chart.drawing_point_to_coordinate(id, 0).unwrap();
        let b = chart.drawing_point_to_coordinate(id, 1).unwrap();
        (id, ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0))
    }

    fn double_click(chart: &mut ChartEngine, x: f64, y: f64) {
        chart.input_pointer_down(at(x, y), 2);
        chart.input_pointer_up(at(x, y));
    }

    #[test]
    fn double_click_opens_the_editor_of_the_selected_drawing_under_the_pointer() {
        let mut chart = chart();
        let (id, body) = trend_line_body(&mut chart);
        chart.set_selected_drawing(Some(id));

        // A trend line's body opens its editor, not only its label.
        double_click(&mut chart, body.0, body.1);
        assert_eq!(chart.editing_drawing(), Some(id));
        assert!(chart.cancel_drawing_text_edit());

        // An upstream text annotation (the callout's leader and box) opens it from its body too.
        let callout = chart
            .add_drawing(
                DrawingKind::Callout,
                0,
                vec![
                    DrawingPoint {
                        logical: 30.0,
                        price: 101.0,
                    },
                    DrawingPoint {
                        logical: 45.0,
                        price: 106.0,
                    },
                ],
                Some(r#"{"text":"note"}"#),
            )
            .unwrap();
        chart.build_frame();
        chart.set_selected_drawing(Some(callout));
        let on_callout = (0..80)
            .flat_map(|gx| {
                (0..50).map(move |gy| (f64::from(gx) * 10.0 + 2.0, f64::from(gy) * 10.0 + 3.0))
            })
            .find(|&(x, y)| chart.drawing_at(x, y) == Some(callout))
            .expect("the callout has a body target");
        double_click(&mut chart, on_callout.0, on_callout.1);
        assert_eq!(chart.editing_drawing(), Some(callout));
        assert!(chart.cancel_drawing_text_edit());

        // A family text box (the simple annotation's, which may span lines) opens it from
        // anywhere on its body too.
        let annotation = chart
            .add_drawing(
                DrawingKind::SimpleAnnotation,
                0,
                vec![DrawingPoint {
                    logical: 20.0,
                    price: 100.0,
                }],
                Some(r#"{"text":"note"}"#),
            )
            .unwrap();
        chart.build_frame();
        chart.set_selected_drawing(Some(annotation));
        let on_annotation = (0..80)
            .flat_map(|gx| {
                (0..50).map(move |gy| (f64::from(gx) * 10.0 + 2.0, f64::from(gy) * 10.0 + 3.0))
            })
            .find(|&(x, y)| chart.drawing_at(x, y) == Some(annotation))
            .expect("the simple annotation has a body target");
        double_click(&mut chart, on_annotation.0, on_annotation.1);
        assert_eq!(chart.editing_drawing(), Some(annotation));
        assert!(chart.cancel_drawing_text_edit());

        // The point must belong to the selected drawing: a selection alone is not enough.
        chart.set_selected_drawing(Some(id));
        let (x, y) = empty_pane_point(&chart);
        assert_ne!(chart.drawing_at(x, y), Some(id));
        double_click(&mut chart, x, y);
        assert_eq!(chart.editing_drawing(), None);

        // A locked drawing refuses and leaves everything as it was.
        assert!(chart.set_drawing_locked(id, true));
        double_click(&mut chart, body.0, body.1);
        assert_eq!(chart.editing_drawing(), None);
    }

    #[test]
    fn a_press_inside_an_edited_family_text_box_places_the_caret_instead_of_committing() {
        let mut chart = chart();
        let callout = chart
            .add_drawing(
                DrawingKind::Callout,
                0,
                vec![
                    DrawingPoint {
                        logical: 30.0,
                        price: 101.0,
                    },
                    DrawingPoint {
                        logical: 45.0,
                        price: 106.0,
                    },
                ],
                Some(r#"{"text":"a note that spans the box"}"#),
            )
            .unwrap();
        chart.build_frame();
        assert!(chart.begin_drawing_text_edit(callout, true));
        let rect = chart.drawing_text_edit_layout(callout).unwrap().rect;
        let (x, y) = ((rect[0] + rect[2]) / 2.0, (rect[1] + rect[3]) / 2.0);
        // The press is on the box being edited: it positions the caret and the session stays.
        click(&mut chart, x, y);
        assert_eq!(chart.editing_drawing(), Some(callout));
        // A press outside the box still commits it.
        let (outside_x, outside_y) = empty_pane_point(&chart);
        assert!(
            outside_x < rect[0]
                || outside_x > rect[2]
                || outside_y < rect[1]
                || outside_y > rect[3]
        );
        click(&mut chart, outside_x, outside_y);
        assert_eq!(chart.editing_drawing(), None);

        // The same holds for a family text box that spans lines (a simple annotation's).
        let annotation = chart
            .add_drawing(
                DrawingKind::SimpleAnnotation,
                0,
                vec![DrawingPoint {
                    logical: 20.0,
                    price: 100.0,
                }],
                Some(r#"{"text":"a note that spans the box"}"#),
            )
            .unwrap();
        chart.build_frame();
        assert!(chart.begin_drawing_text_edit(annotation, true));
        let rect = chart.drawing_text_edit_layout(annotation).unwrap().rect;
        let (x, y) = ((rect[0] + rect[2]) / 2.0, (rect[1] + rect[3]) / 2.0);
        click(&mut chart, x, y);
        assert_eq!(chart.editing_drawing(), Some(annotation));
        let (outside_x, outside_y) = empty_pane_point(&chart);
        assert!(
            outside_x < rect[0]
                || outside_x > rect[2]
                || outside_y < rect[1]
                || outside_y > rect[3]
        );
        click(&mut chart, outside_x, outside_y);
        assert_eq!(chart.editing_drawing(), None);
    }

    #[test]
    fn a_trading_object_under_the_second_click_keeps_it() {
        use crate::{
            OrderId, OrderKind, OrderRole, OrderSide, OrderStatus, TradingPriceScale,
            TradingSnapshot, WorkingOrder,
        };
        let mut chart = chart();
        let (id, body) = trend_line_body(&mut chart);
        chart.set_selected_drawing(Some(id));
        // A working order line through the trend line's body.
        let price = chart.series_coordinate_to_price(0, body.1).unwrap();
        chart
            .set_trading_snapshot(TradingSnapshot {
                orders: vec![WorkingOrder {
                    id: OrderId::new("order-1".to_string()).unwrap(),
                    account_id: None,
                    pane_index: 0,
                    price_scale: TradingPriceScale::Right,
                    side: OrderSide::Sell,
                    kind: OrderKind::Limit,
                    role: OrderRole::Working,
                    status: OrderStatus::Working,
                    price,
                    stop_price: None,
                    trailing_trigger_price: None,
                    break_even_trigger_price: None,
                    quantity: 1.0,
                    filled_quantity: 0.0,
                    position_id: None,
                    parent_order_id: None,
                    bracket_id: None,
                    oco_group_id: None,
                    revision: 1,
                    annotations: Vec::new(),
                }],
                ..TradingSnapshot::default()
            })
            .unwrap();
        assert!(chart.trading_hit_at(body.0, body.1).is_some());
        double_click(&mut chart, body.0, body.1);
        assert_eq!(chart.editing_drawing(), None);
    }

    /// A double-click whose first click used a trading control never reaches the drawing under
    /// it, even once the host has removed the control: a cancelled order's line is gone by the
    /// second click, and the selection that was valid when the pair began is not a request to edit.
    #[test]
    fn a_click_pair_that_began_on_a_trading_control_never_opens_the_editor() {
        use crate::{
            OrderId, OrderKind, OrderRole, OrderSide, OrderStatus, TradingPriceScale,
            TradingSnapshot, WorkingOrder,
        };
        let mut chart = chart();
        let id = chart
            .add_drawing(
                DrawingKind::Rectangle,
                0,
                vec![
                    DrawingPoint {
                        logical: 2.0,
                        price: 98.0,
                    },
                    DrawingPoint {
                        logical: 70.0,
                        price: 108.0,
                    },
                ],
                Some(r#"{"text":"guarded"}"#),
            )
            .unwrap();
        chart.set_selected_drawing(Some(id));
        chart.build_frame();
        let price = 103.0;
        chart
            .set_trading_snapshot(TradingSnapshot {
                orders: vec![WorkingOrder {
                    id: OrderId::new("order-1".to_string()).unwrap(),
                    account_id: None,
                    pane_index: 0,
                    price_scale: TradingPriceScale::Right,
                    side: OrderSide::Sell,
                    kind: OrderKind::Limit,
                    role: OrderRole::Working,
                    status: OrderStatus::Working,
                    price,
                    stop_price: None,
                    trailing_trigger_price: None,
                    break_even_trigger_price: None,
                    quantity: 1.0,
                    filled_quantity: 0.0,
                    position_id: None,
                    parent_order_id: None,
                    bracket_id: None,
                    oco_group_id: None,
                    revision: 1,
                    annotations: Vec::new(),
                }],
                ..TradingSnapshot::default()
            })
            .unwrap();
        let y = chart
            .trading_price_coordinate(0, TradingPriceScale::Right, price)
            .unwrap();
        let x = (0..chart.pane_w as i32)
            .rev()
            .map(f64::from)
            .find(|&x| {
                chart
                    .trading_hit_at(x, y)
                    .is_some_and(|hit| hit.kind == TradingHitKind::CancelButton)
            })
            .expect("the order has a cancel control");
        assert_eq!(
            chart.drawing_at(x, y),
            Some(id),
            "the text box lies under it"
        );

        let sample = |timestamp_ms| PointerInput {
            timestamp_ms,
            ..at(x, y)
        };
        chart.input_pointer_down(sample(100.0), 1);
        chart.input_pointer_up(sample(101.0));
        assert!(
            !chart.take_trading_intents().is_empty(),
            "the control still cancels"
        );
        // The host answers the cancel intent by removing the order.
        chart
            .set_trading_snapshot(TradingSnapshot::default())
            .unwrap();
        assert!(chart.trading_hit_at(x, y).is_none());
        assert_eq!(chart.selected_drawing(), Some(id));
        chart.take_input_events();

        chart.input_pointer_down(sample(200.0), 2);
        chart.input_pointer_up(sample(201.0));
        assert_eq!(chart.editing_drawing(), None);
        assert!(!chart
            .take_input_events()
            .iter()
            .any(|event| matches!(event, ChartInputEvent::TextEditorOpened(_))));

        // Once the pair is spent, a later double-click on the drawing edits it again.
        let (inside_x, inside_y) = (x - 60.0, y);
        assert_eq!(chart.drawing_at(inside_x, inside_y), Some(id));
        let inside = |timestamp_ms| PointerInput {
            timestamp_ms,
            ..at(inside_x, inside_y)
        };
        chart.input_pointer_down(inside(1_000.0), 1);
        chart.input_pointer_up(inside(1_001.0));
        chart.input_pointer_down(inside(1_100.0), 2);
        chart.input_pointer_up(inside(1_101.0));
        assert_eq!(chart.editing_drawing(), Some(id));
    }

    #[test]
    fn enter_and_f2_edit_the_selected_drawing_but_enter_still_finishes_a_sequence() {
        let mut chart = chart();
        let none = InputModifiers::default();
        let (id, _) = trend_line_body(&mut chart);
        // Nothing selected: nothing to edit, and the keys stay unconsumed.
        assert!(!chart.input_key_down(ChartKey::Enter, none, false, 0.0));
        assert!(!chart.input_key_down(ChartKey::EditText, none, false, 0.0));

        chart.set_selected_drawing(Some(id));
        assert!(chart.input_key_down(ChartKey::Enter, none, false, 0.0));
        assert_eq!(chart.editing_drawing(), Some(id));
        assert!(chart.cancel_drawing_text_edit());
        assert!(chart.input_key_down(ChartKey::EditText, none, false, 0.0));
        assert_eq!(chart.editing_drawing(), Some(id));
        assert!(chart.cancel_drawing_text_edit());

        // A locked drawing refuses.
        assert!(chart.set_drawing_locked(id, true));
        assert!(!chart.input_key_down(ChartKey::EditText, none, false, 0.0));
        assert_eq!(chart.editing_drawing(), None);
        assert!(chart.set_drawing_locked(id, false));

        // A variable-length tool under way finishes on Enter instead of editing the selection.
        chart.set_selected_drawing(Some(id));
        assert!(chart.set_drawing_tool(Some(DrawingKind::Path), None, None));
        click(&mut chart, 120.0, 120.0);
        click(&mut chart, 200.0, 180.0);
        click(&mut chart, 280.0, 120.0);
        let before = chart.drawings().len();
        assert!(chart.input_key_down(ChartKey::Enter, none, false, 0.0));
        assert_eq!(chart.drawings().len(), before + 1);
        assert_eq!(chart.editing_drawing(), None);
    }

    #[test]
    fn a_drawing_whose_text_is_scrolled_out_of_view_opens_no_editor() {
        let mut chart = chart();
        let none = InputModifiers::default();
        let (id, _) = trend_line_body(&mut chart);
        chart.set_selected_drawing(Some(id));
        // Pan until the whole line, and with it the label, is left of the plot: the selection
        // persists, but an editor opened there would be invisible and swallow the keys.
        chart.set_right_offset(200.0);
        chart.build_frame();
        let layout = chart.drawing_text_edit_layout(id).unwrap();
        assert!(layout.rect[2] <= 0.0, "{:?}", layout.rect);
        assert_eq!(chart.selected_drawing(), Some(id));
        assert!(!chart.input_key_down(ChartKey::Enter, none, false, 0.0));
        assert!(!chart.input_key_down(ChartKey::EditText, none, false, 0.0));
        assert_eq!(chart.editing_drawing(), None);

        // Back in view, the same keys edit it.
        chart.set_right_offset(0.0);
        chart.build_frame();
        assert!(chart.input_key_down(ChartKey::Enter, none, false, 0.0));
        assert_eq!(chart.editing_drawing(), Some(id));
        assert!(chart.cancel_drawing_text_edit());
        assert!(chart.input_key_down(ChartKey::EditText, none, false, 0.0));
        assert_eq!(chart.editing_drawing(), Some(id));
    }

    #[test]
    fn double_click_on_a_visible_body_whose_label_is_out_of_view_opens_no_editor() {
        let mut chart = chart();
        // A line running far beyond the right edge: its body crosses the plot while its
        // label, at the midpoint, is off-screen.
        let id = chart
            .add_drawing(
                DrawingKind::TrendLine,
                0,
                vec![
                    DrawingPoint {
                        logical: 10.0,
                        price: 102.0,
                    },
                    DrawingPoint {
                        logical: 300.0,
                        price: 104.0,
                    },
                ],
                None,
            )
            .unwrap();
        chart.build_frame();
        let layout = chart.drawing_text_edit_layout(id).unwrap();
        assert!(layout.rect[0] >= chart.pane_w, "{:?}", layout.rect);
        let a = chart.drawing_point_to_coordinate(id, 0).unwrap();
        let slope = (chart.drawing_point_to_coordinate(id, 1).unwrap().1 - a.1)
            / (chart.drawing_point_to_coordinate(id, 1).unwrap().0 - a.0);
        let x = a.0 + 150.0;
        let y = a.1 + slope * 150.0;
        chart.set_selected_drawing(Some(id));
        assert_eq!(chart.drawing_at(x, y), Some(id));
        double_click(&mut chart, x, y);
        assert_eq!(chart.editing_drawing(), None);
        assert_eq!(chart.selected_drawing(), Some(id));
    }

    #[test]
    fn keyboard_keys_follow_the_gesture_switches() {
        let mut chart = chart();
        let none = InputModifiers::default();
        let fixed = InteractionOptions {
            pan: false,
            wheel_scroll: false,
            wheel_zoom: false,
            axis_double_click_reset_time: false,
            ..InteractionOptions::default()
        };
        chart.set_interaction_options(fixed);
        let (position, spacing) = (chart.scroll_position(), chart.bar_spacing());
        for key in [
            ChartKey::ArrowLeft,
            ChartKey::ArrowRight,
            ChartKey::PageUp,
            ChartKey::PageDown,
            ChartKey::Home,
            ChartKey::End,
            ChartKey::ZoomIn,
            ChartKey::ZoomOut,
        ] {
            assert!(
                !chart.input_key_down(key, none, false, 0.0),
                "{key:?} is gated and stays unconsumed"
            );
        }
        assert_eq!(chart.scroll_position(), position);
        assert_eq!(chart.bar_spacing(), spacing);

        // Wheel scrolling alone enables the scroll keys; zoom and the time-axis reset stay off.
        chart.set_interaction_options(InteractionOptions {
            wheel_scroll: true,
            ..fixed
        });
        assert!(chart.input_key_down(ChartKey::PageUp, none, false, 0.0));
        assert!(chart.scroll_position() < position);
        assert!(!chart.input_key_down(ChartKey::ZoomIn, none, false, 0.0));
        assert!(!chart.input_key_down(ChartKey::Home, none, false, 0.0));
        assert_eq!(chart.bar_spacing(), spacing);

        // The time-axis reset switch gates Home; zoom follows the wheel-zoom switch.
        chart.set_interaction_options(InteractionOptions {
            axis_double_click_reset_time: true,
            wheel_zoom: true,
            ..fixed
        });
        assert!(chart.input_key_down(ChartKey::ZoomIn, none, false, 0.0));
        assert!(chart.bar_spacing() > spacing);
        assert!(chart.input_key_down(ChartKey::Home, none, false, 0.0));
        assert!(!chart.input_key_down(ChartKey::PageUp, none, false, 0.0));
    }

    #[test]
    fn home_resets_the_whole_view_on_every_host() {
        let mut chart = chart();
        let none = InputModifiers::default();
        chart.set_price_scale_auto_scale(0, false, false);
        assert_eq!(chart.price_scale_auto_scale(0, false), Some(false));
        chart.set_bar_spacing(chart.bar_spacing() * 3.0);
        let zoomed = chart.bar_spacing();
        assert!(chart.input_key_down(ChartKey::Home, none, false, 0.0));
        assert_ne!(chart.bar_spacing(), zoomed, "Home refits the time axis");
        assert_eq!(
            chart.price_scale_auto_scale(0, false),
            Some(true),
            "Home returns the price scale to autoscale"
        );
    }

    #[test]
    fn ctrl_magnet_follows_the_effective_drawing_magnet() {
        let mut chart = chart();
        let ctrl = |x, y| PointerInput {
            modifiers: InputModifiers {
                control: true,
                ..InputModifiers::default()
            },
            ..at(x, y)
        };
        // No tool armed: Ctrl alone never turns the OHLC magnet on.
        chart.input_pointer_move(ctrl(300.0, 200.0), false);
        assert!(!chart.crosshair_ohlc_magnet);

        // Chart magnet off: Ctrl is the temporary strong magnet while a tool is armed.
        assert!(chart.set_drawing_tool(Some(DrawingKind::TrendLine), None, None));
        chart.input_pointer_move(at(300.0, 200.0), false);
        assert!(!chart.crosshair_ohlc_magnet);
        chart.input_pointer_move(ctrl(300.0, 200.0), false);
        assert!(chart.crosshair_ohlc_magnet);

        // Chart magnet strong: it snaps without Ctrl, and Ctrl turns it off for this placement.
        chart.set_drawing_magnet_mode(DrawingMagnetMode::Strong);
        chart.input_pointer_move(at(300.0, 200.0), false);
        assert!(chart.crosshair_ohlc_magnet);
        chart.input_pointer_move(ctrl(300.0, 200.0), false);
        assert!(!chart.crosshair_ohlc_magnet);

        // Chart magnet weak: Ctrl makes it off as well, and no Ctrl keeps the crosshair free.
        chart.set_drawing_magnet_mode(DrawingMagnetMode::Weak);
        chart.input_pointer_move(at(300.0, 200.0), false);
        assert!(!chart.crosshair_ohlc_magnet);
    }

    #[test]
    fn wheel_and_pinch_anchors_clamp_into_the_plot() {
        let mut chart = chart();
        let axis_x = chart.pane_w + 30.0;
        let edge = chart.pane_w;
        let ctrl = InputModifiers {
            control: true,
            ..InputModifiers::default()
        };
        // Over the price-axis strip the wheel anchors at the plot edge: the bar under the edge
        // stays under it instead of the zoom pivoting around a point outside the plot.
        let before = chart.time_scale.coordinate_to_float_index(edge);
        chart.wheel_zoom_time_scale(axis_x, 1.0, ctrl);
        let after = chart.time_scale.coordinate_to_float_index(edge);
        assert!((before - after).abs() < 1e-6, "wheel: {before} -> {after}");
        let before = chart.time_scale.coordinate_to_float_index(edge);
        assert!(chart.input_pinch(axis_x, 100.0, 0.1, 0.0));
        let after = chart.time_scale.coordinate_to_float_index(edge);
        assert!((before - after).abs() < 1e-6, "pinch: {before} -> {after}");
    }

    #[test]
    fn text_annotation_creation_and_reedit_use_the_engine_controller() {
        let mut chart = chart();
        assert!(chart.set_drawing_tool(Some(DrawingKind::Note), None, None));
        click(&mut chart, 250.0, 180.0);
        let events = chart.take_input_events();
        let [ChartInputEvent::DrawingCreated(id)] = events[..] else {
            panic!("note creation must notify the host: {events:?}");
        };
        assert_eq!(chart.drawing_text_edit().map(|session| session.0), Some(id));
        assert!(chart.set_drawing_text_edit("Remember", 8));
        assert!(chart.commit_drawing_text_edit());
        assert_eq!(chart.drawing(id).unwrap().text, "Remember");
        click(&mut chart, 250.0, 180.0);
        assert_eq!(chart.drawing_text_edit().map(|session| session.0), Some(id));
        assert!(chart.cancel_drawing_text_edit());

        assert!(chart.set_drawing_tool(Some(DrawingKind::Callout), None, None));
        click(&mut chart, 300.0, 200.0);
        assert!(!chart
            .take_input_events()
            .iter()
            .any(|event| matches!(event, ChartInputEvent::DrawingCreated(_))));
        click(&mut chart, 380.0, 170.0);
        let events = chart.take_input_events();
        let [ChartInputEvent::DrawingCreated(callout)] = events[..] else {
            panic!("callout creation must notify the host: {events:?}");
        };
        assert_eq!(
            chart.drawing_text_edit().map(|session| session.0),
            Some(callout)
        );
        chart.commit_drawing_text_edit();
        assert!(chart.set_drawing_tool(Some(DrawingKind::AnchoredText), None, None));
        click(&mut chart, 210.0, 145.0);
        let events = chart.take_input_events();
        let [ChartInputEvent::DrawingCreated(anchored)] = events[..] else {
            panic!("anchored text creation must notify the host: {events:?}");
        };
        let point = chart.drawing_px(chart.drawing(anchored).unwrap()).unwrap()[0];
        assert!((point.0 - 210.0).abs() < 1e-4, "{point:?}");
        assert!((point.1 - 145.0).abs() < 1e-4, "{point:?}");
    }
}
