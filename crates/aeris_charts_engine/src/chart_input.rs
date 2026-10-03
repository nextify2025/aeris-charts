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
const CLICK_SLOP_MANHATTAN: f64 = 5.0;
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

/// Host-configurable interaction switches (the reference `handleScroll`/`handleScale` family).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InteractionOptions {
    /// Primary drag on empty pane space pans the time scale (and a manual price scale).
    pub pan: bool,
    pub wheel_scroll: bool,
    pub wheel_zoom: bool,
    pub wheel_behavior: WheelBehavior,
    /// The wheel over a price axis zooms that scale in `Auto` mode too, not only in `Zoom` mode.
    pub price_axis_wheel_zoom: bool,
    pub axis_double_click_reset_time: bool,
    pub axis_double_click_reset_price: bool,
    pub axis_scale_price: bool,
    pub axis_scale_time: bool,
    /// A released mouse pan coasts with the engine's kinetic model.
    pub kinetic_mouse: bool,
    pub panes_resize: bool,
}

impl Default for InteractionOptions {
    fn default() -> Self {
        Self {
            pan: true,
            wheel_scroll: true,
            wheel_zoom: true,
            wheel_behavior: WheelBehavior::Auto,
            price_axis_wheel_zoom: false,
            axis_double_click_reset_time: true,
            axis_double_click_reset_price: true,
            axis_scale_price: true,
            axis_scale_time: true,
            kinetic_mouse: false,
            panes_resize: true,
        }
    }
}

/// Semantic pointer feedback. Each host maps it to one platform cursor in one place.
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
    /// A drawing tool committed a drawing. One-shot tools are already disarmed.
    DrawingCreated(DrawingId),
    /// Delete was pressed on a selected series the engine does not own (a host-installed series or
    /// an external study). The host decides how its own objects are removed.
    RemoveSeries(SeriesId),
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
            Self::Pane { .. } | Self::DeltaTooltip | Self::Inert => InputTarget::Pane,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Press {
    mode: PressMode,
    start: (f64, f64),
    moved: bool,
    /// The drawing selected when the press began: a selected text drawing opens its editor on
    /// the next click (two-step select-then-type).
    text_press_selected: Option<DrawingId>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct KeyboardPan {
    key: ChartKey,
    delta: f64,
}

/// Runtime-only input state. Never persisted; [`ChartEngine::input_cancel`] resets gestures.
#[derive(Default)]
pub(crate) struct InputController {
    options: InteractionOptions,
    resolver: GestureResolver,
    press: Option<Press>,
    pointer: Option<(f64, f64)>,
    modifiers: InputModifiers,
    cursor: ChartCursor,
    kinetic_active: bool,
    keyboard_pan: Option<KeyboardPan>,
    /// Newest captured freehand sample since the last frame. Native platforms may deliver motion
    /// at device cadence (Wayland: per HID report); the stroke samples display cadence instead.
    pending_capture: Option<(f64, f64, DrawingModifiers)>,
    tooltip_deadline_ms: Option<f64>,
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
        self.input.options = options;
    }

    /// The cursor the platform should present for the current pointer state.
    pub fn input_cursor(&self) -> ChartCursor {
        self.input.cursor
    }

    /// Drain host requests produced by input since the last drain.
    pub fn take_input_events(&mut self) -> Vec<ChartInputEvent> {
        self.input.events.drain(..).collect()
    }

    /// Whether an input-owned animation (kinetic coast, held keyboard pan, animated scroll) needs
    /// another frame. Hosts request animation frames only while this holds.
    pub fn input_animating(&self) -> bool {
        self.input.kinetic_active || self.keyboard_scroll_active() || self.scroll_animation_active()
    }

    /// Earliest host-clock time at which [`Self::input_tick`] has deferred work, if any.
    pub fn input_wake_deadline_ms(&self) -> Option<f64> {
        self.input.tooltip_deadline_ms
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
        if changed {
            self.input.frame_dirty = true;
        }
        changed
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
        if let Some(index) = self.pane_separator_at(y, PANE_SEPARATOR_HIT) {
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
        let (x, y) = (input.x, input.y);
        self.input.pointer = Some((x, y));
        self.input.modifiers = input.modifiers;
        self.input.tooltip_deadline_ms = None;
        self.input.frame_dirty = true;
        self.end_press_without_commit();
        self.stop_input_motion();

        let text_press_selected = self.selected_drawing();
        let press = |mode| Press {
            mode,
            start: (x, y),
            moved: false,
            text_press_selected,
        };
        if self.drawing_text_edit().is_some() {
            if self.point_on_edited_text(x, y) {
                self.drawing_text_edit_caret_at(x, y);
                self.input.press = Some(press(PressMode::TextEditor));
                self.refresh_input_cursor();
                return;
            }
            self.commit_drawing_text_edit();
        }
        self.apply_input_magnet();

        let region = self.region_at(x, y);
        let sequence_or_idle =
            self.active_drawing_tool().is_none() || self.drawing_tool_sequence_active();
        let mode = if click_count >= 2 && sequence_or_idle {
            self.input_double_click(region, input, text_press_selected);
            PressMode::Inert
        } else {
            self.begin_press(region, input)
        };
        self.input
            .resolver
            .pointer_down(Self::mouse_sample(input, mode.target()));
        self.input.press = Some(press(mode));
        if matches!(mode, PressMode::Separator { .. }) {
            self.clear_pointer_hover();
        } else {
            self.refresh_pointer_hover(x, y, input.timestamp_ms);
        }
        self.refresh_input_cursor();
    }

    /// Pointer motion. `primary_pressed` reports whether the primary button is held.
    pub fn input_pointer_move(&mut self, input: PointerInput, primary_pressed: bool) {
        let (x, y) = (input.x, input.y);
        self.input.pointer = Some((x, y));
        self.input.modifiers = input.modifiers;
        self.input.frame_dirty = true;
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
        if self.delta_tooltip_mouse_move(x) {
            self.sync_brushable_areas();
        }
        self.apply_input_magnet();
        let modifiers = drawing_modifiers(input.modifiers);
        if let Some(press) = self.input.press.as_mut() {
            press.moved |=
                (x - press.start.0).abs() + (y - press.start.1).abs() >= CLICK_SLOP_MANHATTAN;
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
        let (x, y) = (input.x, input.y);
        self.input.pointer = Some((x, y));
        self.input.modifiers = input.modifiers;
        self.input.frame_dirty = true;
        if self.delta_tooltip_mouse_up() {
            self.sync_brushable_areas();
        }
        self.input
            .resolver
            .pointer_up(Self::mouse_sample(input, InputTarget::Pane));
        self.clear_trading_pressed();
        let Some(press) = self.input.press.take() else {
            self.refresh_pointer_hover(x, y, input.timestamp_ms);
            self.refresh_input_cursor();
            return;
        };
        let moved = press.moved
            || (x - press.start.0).abs() + (y - press.start.1).abs() >= CLICK_SLOP_MANHATTAN;
        let modifiers = drawing_modifiers(input.modifiers);
        match press.mode {
            PressMode::TextEditor | PressMode::Separator { .. } | PressMode::Inert => {}
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
                self.note_drawing_created(update.created);
                if !capture && !moved && !committed_on_press && update.created.is_none() {
                    let update = self.drawing_tool_activate(x, y, modifiers);
                    self.note_drawing_created(update.created);
                }
            }
            PressMode::Trading { dragging } => {
                if dragging {
                    self.trading_drag_end();
                }
                if !moved {
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
                }
            }
            PressMode::Pane { price_pan, panning } => {
                if let Some((pane, target)) = price_pan {
                    self.price_axis_end_scroll(pane, target);
                }
                if panning {
                    self.end_pan(input.timestamp_ms);
                }
                if !moved {
                    self.input_primary_click(x, y, press.text_press_selected);
                }
            }
        }
        self.apply_input_magnet();
        self.refresh_pointer_hover(x, y, input.timestamp_ms);
        self.refresh_input_cursor();
    }

    /// The pointer left the chart without a held button. Captured gestures are unaffected; a live
    /// measure stays on screen.
    pub fn input_pointer_leave(&mut self) {
        if self.input.press.is_some() {
            return;
        }
        self.input.pointer = None;
        self.input.tooltip_deadline_ms = None;
        self.delta_tooltip_leave();
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
        if !self.input.options.wheel_zoom || !scale_delta.is_finite() || scale_delta == 0.0 {
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
        let handled = match key {
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
            // Home refits the time axis, as the browser and worker hosts do. The time-axis reset
            // switch gates it, so it never touches a price scale: a manual price range is the
            // user's until they reset that axis.
            ChartKey::Home if options.axis_double_click_reset_time => {
                self.fit_content();
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
                self.clear_brushable_ranges();
                self.clear_pointer_hover();
                true
            }
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

    // --- internals ---

    fn mouse_sample(input: PointerInput, target: InputTarget) -> PointerSample {
        PointerSample {
            id: 1,
            device: InputDevice::Mouse,
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
            self.sync_brushable_areas();
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
                && self
                    .drawing(editing)
                    .is_some_and(|drawing| drawing.kind == DrawingKind::Text)
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
            .pan
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
                // alert chip under it keeps the click.
                let selected = self.selected_drawing();
                let owned = selected.is_some()
                    && selected == text_press_selected
                    && self.trading_hit_at(x, y).is_none()
                    && !self.alert_create_hit_at(x, y)
                    && self.drawing_at(x, y) == selected;
                let Some(id) = selected.filter(|_| owned) else {
                    self.clear_brushable_ranges();
                    return;
                };
                // The text tool and a trend line's own label re-run click activation (two-step
                // focus, trend prompt); every other text-bearing drawing, a trend line's body
                // included, opens its editor. A drawing without text, a locked or hidden one, or
                // one with no layout refuses and falls back to clearing the brush ranges.
                let kind = self.drawing(id).map(|drawing| drawing.kind);
                let on_trend_label = kind == Some(DrawingKind::TrendLine)
                    && self.drawing_text_hit_at(x, y) == Some(id);
                if kind == Some(DrawingKind::Text) || on_trend_label {
                    self.input_primary_click(x, y, text_press_selected);
                } else if !self.begin_drawing_text_edit(id, true) {
                    self.clear_brushable_ranges();
                }
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
            drawing.kind == DrawingKind::Text
                && (drawing.text.trim().is_empty() || text_press_selected == Some(id))
        };
        if open_editor {
            self.begin_drawing_text_edit(id, true);
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
            GestureUpdateKind::DragStarted if !panning && self.input.options.pan => {
                self.time_scale_start_scroll(x);
                self.kinetic_begin_sampling(
                    self.input.options.kinetic_mouse,
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
    fn end_pan(&mut self, now_ms: f64) {
        let coasting = self.input.options.kinetic_mouse
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
        self.selected_drawing()
            .is_some_and(|id| self.begin_drawing_text_edit(id, true))
    }

    /// Delete the selection: a drawing or an engine indicator binding is removed here; any other
    /// selected series is host-owned and is requested through [`ChartInputEvent::RemoveSeries`].
    fn delete_selection(&mut self) -> bool {
        if self.remove_selected_drawing() {
            return true;
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
        self.set_hovered_series(None);
        self.set_hovered_text(None);
        self.set_hovered_drawing(None);
        self.clear_general_hover();
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
        // A drawing hit wins over series hits so overlaps stay selectable.
        if self.update_drawing_hover(x, y).is_some() {
            self.clear_general_hover();
            self.set_hovered_series(None);
        } else {
            let hovered = self.hit_test_series(x, y);
            self.set_hovered_series(hovered);
            if let Some(pane) = self.pane_at_y(y) {
                self.update_general_hover(pane, x, y);
            }
        }
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
        if let Some((_, name)) = self.drawing_hover_at(x, y) {
            return ChartCursor::from_hit_name(name);
        }
        if self.hovered_series().is_some() {
            return ChartCursor::Pointer;
        }
        ChartCursor::Crosshair
    }
}

// Interactive scenarios that drive the controller only through its public `input_*` API, grouped
// by the behavior they pin. They share the fixtures in `tests`.
#[cfg(test)]
mod chrome_tests;
#[cfg(test)]
mod drawing_tests;
#[cfg(test)]
mod lifecycle_tests;
#[cfg(test)]
mod motion_tests;
#[cfg(test)]
mod trading_tests;

#[cfg(test)]
mod tests {
    use super::*;

    pub(super) const BARS: usize = 60;

    pub(super) fn chart() -> ChartEngine {
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

    pub(super) fn relayout(chart: &mut ChartEngine) {
        chart.recompute_layout_with_measure(true, |_, _| 48.0, |_, _| 48.0);
    }

    pub(super) fn at(x: f64, y: f64) -> PointerInput {
        PointerInput {
            x,
            y,
            ..PointerInput::default()
        }
    }

    pub(super) fn shifted(x: f64, y: f64) -> PointerInput {
        PointerInput {
            modifiers: InputModifiers {
                shift: true,
                ..InputModifiers::default()
            },
            ..at(x, y)
        }
    }

    pub(super) fn click(chart: &mut ChartEngine, x: f64, y: f64) {
        chart.input_pointer_down(at(x, y), 1);
        chart.input_pointer_up(at(x, y));
    }

    pub(super) fn drag(chart: &mut ChartEngine, from: (f64, f64), to: (f64, f64)) {
        chart.input_pointer_down(at(from.0, from.1), 1);
        for step in 1..=4 {
            let t = f64::from(step) / 4.0;
            let x = from.0 + (to.0 - from.0) * t;
            let y = from.1 + (to.1 - from.1) * t;
            chart.input_pointer_move(at(x, y), true);
        }
        chart.input_pointer_up(at(to.0, to.1));
    }

    pub(super) fn empty_pane_point(chart: &ChartEngine) -> (f64, f64) {
        (40..chart.pane_w as i32)
            .step_by(17)
            .flat_map(|x| (20..chart.pane_h as i32).step_by(13).map(move |y| (x, y)))
            .map(|(x, y)| (f64::from(x), f64::from(y)))
            .find(|&(x, y)| {
                chart.hit_test_series(x, y).is_none() && chart.region_at(x, y) == ChartRegion::Pane
            })
            .expect("the pane has empty space")
    }

    pub(super) fn series_point(chart: &ChartEngine) -> (f64, f64) {
        let x = chart.time_scale.index_to_coordinate(30);
        let y = chart.series_price_to_coordinate(0, 101.0).unwrap();
        assert_eq!(chart.hit_test_series(x, y), Some(0));
        (x, y)
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
    fn keyboard_bindings_navigate_and_escape_disarms() {
        let mut chart = chart();
        let none = InputModifiers::default();
        chart.set_drawing_tool(Some(DrawingKind::Rectangle), None, None);
        assert!(chart.input_key_down(ChartKey::Escape, none, false, 0.0));
        assert_eq!(chart.active_drawing_tool(), None);

        let start = chart.scroll_position();
        assert!(chart.input_key_down(ChartKey::PageUp, none, false, 0.0));
        assert!(chart.scroll_position() < start);
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

        let none = InputModifiers::default();
        assert!(chart.input_key_down(ChartKey::Escape, none, false, 0.0));
        assert!(chart.series_entry(0).unwrap().area_brush.is_none());
        assert!(chart.set_brushable_area(0, None));
        assert!(!chart.is_brushable_area(0));
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
                        fit_content: false,
                        frame: &mut frame,
                        axis_primitives: &mut axis,
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
        assert!(prepare(&mut chart), "pointer hover rebuilds");
        assert!(!prepare(&mut chart));

        chart.set_hovered_series(None);
        assert!(prepare(&mut chart), "an overlay-only invalidation rebuilds");
        assert!(!prepare(&mut chart));
    }

    #[test]
    fn interaction_defaults_match_the_browser_host() {
        let options = InteractionOptions::default();
        assert!(options.pan && options.wheel_scroll && options.wheel_zoom);
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
    pub(super) fn trend_line_body(chart: &mut ChartEngine) -> (DrawingId, (f64, f64)) {
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

    pub(super) fn double_click(chart: &mut ChartEngine, x: f64, y: f64) {
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

        // A family text box opens it from anywhere on its body too.
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
    fn home_resets_the_time_axis_only_like_the_browser_and_worker_hosts() {
        let mut chart = chart();
        let none = InputModifiers::default();
        // A manual price range is the user's, and the time-axis reset switch does not own it.
        chart.set_price_scale_auto_scale(0, false, false);
        assert_eq!(chart.price_scale_auto_scale(0, false), Some(false));
        chart.set_bar_spacing(chart.bar_spacing() * 3.0);
        let zoomed = chart.bar_spacing();
        assert!(chart.input_key_down(ChartKey::Home, none, false, 0.0));
        assert_ne!(chart.bar_spacing(), zoomed, "Home refits the time axis");
        assert_eq!(
            chart.price_scale_auto_scale(0, false),
            Some(false),
            "Home keeps a manual price scale"
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
}
