//! GPUI platform adapter for the engine's input controller.
//!
//! The engine owns every interaction decision (`ChartEngine::input_*`). This module only
//! translates GPUI events into the engine's normalized vocabulary, applies GPUI's platform
//! conventions (wheel line height, text-editing keys, clipboard), and maps the engine's semantic
//! [`ChartCursor`] onto one GPUI cursor. Every GPUI chart host uses it unchanged, so a host binds
//! listeners with one call each and never re-implements chart routing.

use std::time::{Duration, Instant};

use aeris_charts_engine::{
    ChartCursor, ChartEngine, ChartKey, DrawingTextEditKey, InputModifiers, PointerInput,
    WheelDeltaMode, WheelSample,
};
use gpui::{
    point, px, App, Bounds, ClipboardItem, CursorStyle, KeyDownEvent, KeyUpEvent, Modifiers,
    ModifiersChangedEvent, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PinchEvent, Pixels, Point,
    ScrollDelta, ScrollWheelEvent, Window,
};

use crate::backend::{text_cap_centerer, text_measurer};

#[cfg(test)]
mod tests;

/// Browser-equivalent pixels per wheel line (the reference `DOM_DELTA_LINE` adjustment).
pub const WHEEL_LINE_HEIGHT: f32 = 32.0;

/// Per-chart GPUI input state: the chart canvas's top-left window position and a monotonic
/// clock. Everything else lives in the engine.
#[derive(Clone, Copy, Debug)]
pub struct GpuiChartInput {
    canvas_corner: Point<Pixels>,
    epoch: Instant,
}

impl Default for GpuiChartInput {
    fn default() -> Self {
        Self {
            canvas_corner: Point::default(),
            epoch: Instant::now(),
        }
    }
}

impl GpuiChartInput {
    /// Record the chart canvas position from its prepaint bounds.
    pub fn set_canvas_bounds(&mut self, bounds: Bounds<Pixels>) {
        self.canvas_corner = point(bounds.origin.x, bounds.origin.y);
    }

    /// Milliseconds on the clock every input timestamp and [`ChartEngine::input_tick`] share.
    pub fn now_ms(&self) -> f64 {
        self.epoch.elapsed().as_secs_f64() * 1_000.0
    }

    /// Prepaint step: advance input animations and forward the newest coalesced capture sample.
    /// Returns whether chart state changed.
    pub fn prepare_frame(&self, engine: &mut ChartEngine) -> bool {
        let ticked = engine.input_tick(self.now_ms());
        engine.flush_coalesced_input() || ticked
    }

    /// Delay until the engine's next deferred input work (the trading-tooltip dwell). Hosts
    /// schedule one wake and then call [`Self::prepare_frame`] by repainting.
    pub fn wake_delay(&self, engine: &ChartEngine) -> Option<Duration> {
        engine
            .input_wake_deadline_ms()
            .map(|deadline| Duration::from_secs_f64((deadline - self.now_ms()).max(0.0) / 1_000.0))
    }

    /// A window position in the engine's pane space.
    pub fn pane_point(&self, engine: &ChartEngine, position: Point<Pixels>) -> (f64, f64) {
        let x: f32 = (position.x - self.canvas_corner.x).into();
        let y: f32 = (position.y - self.canvas_corner.y).into();
        (f64::from(x) - engine.pane_left, f64::from(y))
    }

    fn pointer(
        &self,
        engine: &ChartEngine,
        position: Point<Pixels>,
        modifiers: &Modifiers,
    ) -> PointerInput {
        let (x, y) = self.pane_point(engine, position);
        PointerInput {
            x,
            y,
            modifiers: input_modifiers(modifiers),
            timestamp_ms: self.now_ms(),
        }
    }

    /// Primary-button press.
    pub fn mouse_down(&self, engine: &mut ChartEngine, event: &MouseDownEvent) {
        let input = self.pointer(engine, event.position, &event.modifiers);
        let click_count = u32::try_from(event.click_count).unwrap_or(u32::MAX);
        engine.input_pointer_down(input, click_count);
    }

    pub fn mouse_move(&self, engine: &mut ChartEngine, event: &MouseMoveEvent) {
        let input = self.pointer(engine, event.position, &event.modifiers);
        engine.input_pointer_move(input, event.dragging());
    }

    /// Primary-button release, including a release outside the chart.
    pub fn mouse_up(&self, engine: &mut ChartEngine, event: &MouseUpEvent) {
        let input = self.pointer(engine, event.position, &event.modifiers);
        engine.input_pointer_up(input);
    }

    /// Secondary-button press: queues an engine context-menu request.
    pub fn context_menu(&self, engine: &mut ChartEngine, event: &MouseDownEvent) {
        let (x, y) = self.pane_point(engine, event.position);
        engine.input_context_menu(x, y);
    }

    /// Returns whether the chart consumed the wheel.
    pub fn scroll_wheel(&self, engine: &mut ChartEngine, event: &ScrollWheelEvent) -> bool {
        let (x, y) = self.pane_point(engine, event.position);
        // GPUI reports wheel-up as a positive delta, which is already the engine's normalized
        // sign; lines convert with the browser's 32 px adjustment.
        let delta = event.delta.pixel_delta(px(WHEEL_LINE_HEIGHT));
        let delta_x: f32 = delta.x.into();
        let delta_y: f32 = delta.y.into();
        engine.input_wheel(WheelSample {
            x,
            y,
            delta_x: f64::from(delta_x) / 100.0,
            delta_y: f64::from(delta_y) / 100.0,
            delta_mode: match event.delta {
                ScrollDelta::Pixels(_) => WheelDeltaMode::Pixel,
                ScrollDelta::Lines(_) => WheelDeltaMode::Line,
            },
            modifiers: input_modifiers(&event.modifiers),
            timestamp_ms: self.now_ms(),
        })
    }

    /// Returns whether the chart consumed the trackpad pinch.
    pub fn pinch(&self, engine: &mut ChartEngine, event: &PinchEvent) -> bool {
        let (x, y) = self.pane_point(engine, event.position);
        engine.input_pinch(x, y, f64::from(event.delta), self.now_ms())
    }

    pub fn modifiers_changed(&self, engine: &mut ChartEngine, event: &ModifiersChangedEvent) {
        engine.input_modifiers_changed(input_modifiers(&event.modifiers));
    }

    /// A key press while the chart has focus. An active typing session owns the keyboard first;
    /// otherwise chart key bindings apply. Returns whether the chart consumed the key.
    pub fn key_down(&self, engine: &mut ChartEngine, event: &KeyDownEvent, cx: &mut App) -> bool {
        if engine.drawing_text_edit().is_some() {
            if !clipboard_key(engine, event, cx) {
                text_edit_key(engine, event);
            }
            return true;
        }
        let Some(key) = chart_key(&event.keystroke.key) else {
            return false;
        };
        engine.input_key_down(
            key,
            input_modifiers(&event.keystroke.modifiers),
            event.is_held,
            self.now_ms(),
        )
    }

    pub fn key_up(&self, engine: &mut ChartEngine, event: &KeyUpEvent) -> bool {
        chart_key(&event.keystroke.key).is_some_and(|key| engine.input_key_up(key))
    }
}

pub fn input_modifiers(modifiers: &Modifiers) -> InputModifiers {
    InputModifiers {
        shift: modifiers.shift,
        control: modifiers.control,
        alt: modifiers.alt,
        meta: modifiers.platform,
    }
}

/// GPUI key names the chart binds.
pub fn chart_key(key: &str) -> Option<ChartKey> {
    Some(match key {
        "left" => ChartKey::ArrowLeft,
        "right" => ChartKey::ArrowRight,
        "pageup" => ChartKey::PageUp,
        "pagedown" => ChartKey::PageDown,
        "+" | "=" => ChartKey::ZoomIn,
        "-" | "_" => ChartKey::ZoomOut,
        "home" => ChartKey::Home,
        "end" => ChartKey::End,
        "enter" => ChartKey::Enter,
        "f2" => ChartKey::EditText,
        "backspace" => ChartKey::Backspace,
        "delete" => ChartKey::Delete,
        "escape" => ChartKey::Escape,
        _ => return None,
    })
}

/// The one GPUI cursor for each engine cursor.
///
/// GPUI's Windows backend draws `OpenHand`/`ClosedHand` as the arrow, so vertically dragged
/// trading lines use the native vertical-resize cursor there instead of looking inert.
pub fn cursor_style(cursor: ChartCursor) -> CursorStyle {
    match cursor {
        ChartCursor::Crosshair => CursorStyle::Crosshair,
        ChartCursor::Default => CursorStyle::Arrow,
        ChartCursor::Pointer => CursorStyle::PointingHand,
        ChartCursor::Text => CursorStyle::IBeam,
        ChartCursor::Move => CursorStyle::OpenHand,
        ChartCursor::Grabbing => CursorStyle::ClosedHand,
        ChartCursor::VerticalGrab if cfg!(target_os = "windows") => CursorStyle::ResizeUpDown,
        ChartCursor::VerticalGrab => CursorStyle::OpenHand,
        ChartCursor::VerticalGrabbing if cfg!(target_os = "windows") => CursorStyle::ResizeUpDown,
        ChartCursor::VerticalGrabbing => CursorStyle::ClosedHand,
        ChartCursor::ResizeVertical => CursorStyle::ResizeUpDown,
        ChartCursor::ResizeHorizontal => CursorStyle::ResizeLeftRight,
        ChartCursor::ResizeNwse => CursorStyle::ResizeUpLeftDownRight,
        ChartCursor::ResizeNesw => CursorStyle::ResizeUpRightDownLeft,
        ChartCursor::ResizeRow => CursorStyle::ResizeRow,
    }
}

/// Install the window's native text measurement and glyph metric so drawing labels, text hit
/// boxes, and trading controls measure exactly as they paint. Call once per prepaint.
pub fn install_text_metrics(engine: &mut ChartEngine, window: &Window) {
    engine.set_text_measure(Some(Box::new(text_measurer(window))));
    engine.set_text_cap_center(Some(Box::new(text_cap_centerer(window))));
}

const fn command_shortcut(event: &KeyDownEvent) -> bool {
    let modifiers = event.keystroke.modifiers;
    (modifiers.control || modifiers.platform) && !modifiers.alt && !event.prefer_character_input
}

/// Clipboard shortcuts inside the typing session; run labels flatten pasted text to one line and
/// family text boxes keep its line breaks. Returns whether the key was a clipboard command.
fn clipboard_key(engine: &mut ChartEngine, event: &KeyDownEvent, cx: &mut App) -> bool {
    if !command_shortcut(event) {
        return false;
    }
    match event.keystroke.key.as_str() {
        "v" => {
            if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                engine.drawing_text_edit_insert(&text);
            }
        }
        key @ ("c" | "x") => {
            if let Some(selected) = engine.drawing_text_edit_selection() {
                cx.write_to_clipboard(ClipboardItem::new_string(selected.to_string()));
                if key == "x" {
                    engine.drawing_text_edit_key(DrawingTextEditKey::Backspace, false);
                }
            }
        }
        _ => return false,
    }
    true
}

/// Route one key into the engine's typing session using the platform's editing conventions:
/// word motion on Ctrl (Option on macOS) and line ends on Cmd+arrows on macOS.
pub fn text_edit_key(engine: &mut ChartEngine, event: &KeyDownEvent) {
    let keystroke = &event.keystroke;
    let modifiers = keystroke.modifiers;
    match keystroke.key.as_str() {
        "enter" => {
            engine.commit_drawing_text_edit();
            return;
        }
        "escape" => {
            engine.cancel_drawing_text_edit();
            return;
        }
        "a" if command_shortcut(event) => {
            engine.drawing_text_edit_select_all();
            return;
        }
        _ => {}
    }
    let word = if cfg!(target_os = "macos") {
        modifiers.alt
    } else {
        modifiers.control && !event.prefer_character_input
    };
    let line = cfg!(target_os = "macos") && modifiers.platform;
    let key = match keystroke.key.as_str() {
        "backspace" if word => Some(DrawingTextEditKey::DeleteWordBackward),
        "delete" if word => Some(DrawingTextEditKey::DeleteWordForward),
        "backspace" => Some(DrawingTextEditKey::Backspace),
        "delete" => Some(DrawingTextEditKey::Delete),
        "left" if line => Some(DrawingTextEditKey::Home),
        "right" if line => Some(DrawingTextEditKey::End),
        "left" if word => Some(DrawingTextEditKey::WordLeft),
        "right" if word => Some(DrawingTextEditKey::WordRight),
        "left" => Some(DrawingTextEditKey::Left),
        "right" => Some(DrawingTextEditKey::Right),
        "home" | "up" => Some(DrawingTextEditKey::Home),
        "end" | "down" => Some(DrawingTextEditKey::End),
        _ => None,
    };
    if let Some(key) = key {
        engine.drawing_text_edit_key(key, modifiers.shift);
        return;
    }
    // AltGr characters arrive with Ctrl+Alt on Windows; GPUI marks them as character input.
    let typed = !modifiers.function
        && (event.prefer_character_input
            || (!modifiers.control && !modifiers.platform && !modifiers.alt));
    if let Some(text) = keystroke.key_char.as_deref().filter(|_| typed) {
        engine.drawing_text_edit_insert(text);
    } else if typed && keystroke.key == "space" {
        engine.drawing_text_edit_insert(" ");
    }
}
