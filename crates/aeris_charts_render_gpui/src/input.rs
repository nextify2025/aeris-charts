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
    point, App, Bounds, ClipboardItem, CursorStyle, KeyDownEvent, KeyUpEvent, Modifiers,
    ModifiersChangedEvent, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PinchEvent, Pixels, Point,
    ScrollDelta, ScrollWheelEvent, Window,
};

use crate::backend::{text_cap_centerer, text_measurer};

#[cfg(test)]
mod tests;

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

    /// Prepaint step with GPUI's current application motion preference.
    pub fn prepare_frame_with_motion(
        &self,
        engine: &mut ChartEngine,
        reduced_motion: bool,
    ) -> bool {
        Self::sync_motion_policy(engine, reduced_motion);
        self.prepare_frame(engine)
    }

    /// Apply the host's current motion preference before processing an input frame.
    /// GPUI exposes this through `App::reduce_motion()`; chart policy remains engine-owned.
    fn sync_motion_policy(engine: &mut ChartEngine, reduced_motion: bool) {
        let mut options = engine.interaction_options();
        if options.reduced_motion != reduced_motion {
            options.reduced_motion = reduced_motion;
            engine.set_interaction_options(options);
        }
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
            id: 1,
            device: aeris_charts_engine::InputDevice::Mouse,
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
        // GPUI reports wheel-up as positive y but its horizontal sign is opposite the browser's.
        // Preserve raw line/pixel units for the engine's shared normalization rule.
        let (delta_x, delta_y, delta_mode) = match event.delta {
            ScrollDelta::Pixels(delta) => {
                let x: f32 = delta.x.into();
                let y: f32 = delta.y.into();
                (x, y, WheelDeltaMode::Pixel)
            }
            ScrollDelta::Lines(delta) => {
                // Windows GPUI has already multiplied notch counts by the system's line setting
                // (three by default). Express those native lines in DOM line units so the default
                // three-line notch resolves to 100 px while the configured speed stays relative.
                const NATIVE_TO_DOM_LINES: f32 = 25.0 / 24.0;
                (
                    delta.x * NATIVE_TO_DOM_LINES,
                    delta.y * NATIVE_TO_DOM_LINES,
                    WheelDeltaMode::Line,
                )
            }
        };
        engine.input_wheel(WheelSample {
            x,
            y,
            delta_x: WheelSample::normalize_delta(-f64::from(delta_x), delta_mode, 1.0),
            delta_y: WheelSample::normalize_delta(f64::from(delta_y), delta_mode, 1.0),
            delta_mode,
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
        let Some(key) = chart_key_down(event) else {
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

fn chart_key_down(event: &KeyDownEvent) -> Option<ChartKey> {
    if command_shortcut(event) && event.keystroke.key == "z" {
        Some(if event.keystroke.modifiers.shift {
            ChartKey::Redo
        } else {
            ChartKey::Undo
        })
    } else {
        chart_key(&event.keystroke.key)
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
/// GPUI has no four-way move cursor, so [`ChartCursor::Move`] (CSS `move` in browsers) maps to
/// the open hand, the closest grab affordance GPUI offers.
///
/// GPUI's Windows backend draws `OpenHand`/`ClosedHand` as the arrow, so on Windows movable
/// drawings use the pointing hand and vertically dragged trading lines use the native
/// vertical-resize cursor instead of looking inert.
pub fn cursor_style(cursor: ChartCursor) -> CursorStyle {
    match cursor {
        ChartCursor::Crosshair => CursorStyle::Crosshair,
        ChartCursor::Default => CursorStyle::Arrow,
        ChartCursor::Pointer => CursorStyle::PointingHand,
        ChartCursor::Text => CursorStyle::IBeam,
        ChartCursor::Move if cfg!(target_os = "windows") => CursorStyle::PointingHand,
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
        ChartCursor::HostPrimitive => CursorStyle::Crosshair,
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

#[cfg(test)]
mod wheel_tests {
    use super::*;
    use gpui::px;

    #[test]
    fn movable_drawings_never_show_an_inert_arrow() {
        let expected = if cfg!(target_os = "windows") {
            CursorStyle::PointingHand
        } else {
            CursorStyle::OpenHand
        };
        assert_eq!(cursor_style(ChartCursor::Move), expected);
    }

    #[test]
    fn native_motion_policy_tracks_host_setting() {
        let adapter = GpuiChartInput::default();
        let mut chart = ChartEngine::new(640.0, 360.0, 1.0);
        let times: Vec<f64> = (0..200).map(|i| 1_000.0 + i as f64 * 60.0).collect();
        chart
            .set_series_data(
                0,
                &times,
                &[100.0; 200],
                &[102.0; 200],
                &[99.0; 200],
                &[101.0; 200],
            )
            .unwrap();
        chart.fit_content();
        chart.scroll_to_position(-30.0);
        assert!(!chart.interaction_options().reduced_motion);
        assert!(chart.input_key_down(
            ChartKey::ArrowRight,
            InputModifiers::default(),
            false,
            adapter.now_ms(),
        ));
        assert!(chart.input_animating());
        adapter.prepare_frame_with_motion(&mut chart, true);
        assert!(chart.interaction_options().reduced_motion);
        assert!(!chart.input_animating());
        let start = chart.scroll_position();
        assert!(chart.input_key_down(
            ChartKey::ArrowRight,
            InputModifiers::default(),
            true,
            adapter.now_ms(),
        ));
        assert_eq!(chart.scroll_position(), start + 1.0);
        assert!(!chart.input_animating());
        adapter.prepare_frame_with_motion(&mut chart, false);
        assert!(!chart.interaction_options().reduced_motion);
    }

    #[test]
    fn gpui_second_click_can_drag_through_the_shared_pointer_controller() {
        let adapter = GpuiChartInput::default();
        let mut chart = ChartEngine::new(640.0, 360.0, 1.0);
        let times: Vec<f64> = (0..200).map(|i| 1_000.0 + i as f64 * 60.0).collect();
        chart
            .set_series_data(
                0,
                &times,
                &[100.0; 200],
                &[102.0; 200],
                &[99.0; 200],
                &[101.0; 200],
            )
            .unwrap();
        chart.recompute_layout_with_measure(true, |_, _| 48.0, |_, _| 48.0);
        chart.fit_content();
        let start = chart.scroll_position();
        let at = |x| point(px(x), px(150.0));
        adapter.mouse_down(
            &mut chart,
            &MouseDownEvent {
                position: at(250.0),
                click_count: 1,
                ..MouseDownEvent::default()
            },
        );
        adapter.mouse_up(
            &mut chart,
            &MouseUpEvent {
                position: at(250.0),
                click_count: 1,
                ..MouseUpEvent::default()
            },
        );
        chart.take_input_events();
        adapter.mouse_down(
            &mut chart,
            &MouseDownEvent {
                position: at(250.0),
                click_count: 2,
                ..MouseDownEvent::default()
            },
        );
        adapter.mouse_move(
            &mut chart,
            &MouseMoveEvent {
                position: at(275.0),
                pressed_button: Some(gpui::MouseButton::Left),
                ..MouseMoveEvent::default()
            },
        );
        adapter.mouse_move(
            &mut chart,
            &MouseMoveEvent {
                position: at(310.0),
                pressed_button: Some(gpui::MouseButton::Left),
                ..MouseMoveEvent::default()
            },
        );
        adapter.mouse_up(
            &mut chart,
            &MouseUpEvent {
                position: at(310.0),
                click_count: 2,
                ..MouseUpEvent::default()
            },
        );
        assert_ne!(chart.scroll_position(), start);
        assert!(chart.take_input_events().is_empty());

        adapter.mouse_down(
            &mut chart,
            &MouseDownEvent {
                position: at(300.0),
                click_count: 2,
                ..MouseDownEvent::default()
            },
        );
        adapter.mouse_up(
            &mut chart,
            &MouseUpEvent {
                position: at(300.0),
                click_count: 2,
                ..MouseUpEvent::default()
            },
        );
        assert_eq!(
            chart.take_input_events(),
            vec![aeris_charts_engine::ChartInputEvent::DoubleClick { x: 300.0, y: 150.0 }]
        );
    }

    #[test]
    fn gpui_shift_and_horizontal_wheels_match_browser_pan_direction() {
        let adapter = GpuiChartInput::default();
        // A GPUI rightward swipe reports positive x; the browser reports the same gesture as
        // negative deltaX. Expected values are the browser's normalized notches, written out so
        // the test cannot share a sign error with the adapter.
        for (delta, browser_notches) in [
            (ScrollDelta::Lines(point(1.0, 0.0)), -1.0 / 3.0),
            (ScrollDelta::Pixels(point(px(32.0), px(0.0))), -0.32),
        ] {
            let mut actual = ChartEngine::new(400.0, 200.0, 1.0);
            let mut expected = ChartEngine::new(400.0, 200.0, 1.0);
            for chart in [&mut actual, &mut expected] {
                chart
                    .set_series_data(
                        0,
                        &[1.0, 2.0, 3.0, 4.0],
                        &[10.0; 4],
                        &[11.0; 4],
                        &[9.0; 4],
                        &[10.0; 4],
                    )
                    .unwrap();
                chart.time_scale.set_width(400.0);
                chart.fit_content();
            }
            let event = ScrollWheelEvent {
                position: point(px(200.0), px(100.0)),
                delta,
                modifiers: Modifiers {
                    shift: true,
                    ..Modifiers::default()
                },
                ..ScrollWheelEvent::default()
            };
            let start = actual.time_scale.right_offset();
            assert!(adapter.scroll_wheel(&mut actual, &event));
            assert_ne!(actual.time_scale.right_offset(), start);
            let delta_mode = match delta {
                ScrollDelta::Lines(_) => WheelDeltaMode::Line,
                ScrollDelta::Pixels(_) => WheelDeltaMode::Pixel,
            };
            assert!(expected.input_wheel(WheelSample {
                x: 200.0,
                y: 100.0,
                delta_x: browser_notches,
                delta_mode,
                modifiers: InputModifiers {
                    shift: true,
                    ..InputModifiers::default()
                },
                ..WheelSample::default()
            }));
            assert!(
                (actual.time_scale.right_offset() - expected.time_scale.right_offset()).abs()
                    < 1e-6
            );
        }
    }

    #[test]
    fn three_native_wheel_lines_match_one_browser_notch_at_each_dpr() {
        let adapter = GpuiChartInput::default();
        for dpr in [1.0, 1.5, 2.0] {
            let mut native = ChartEngine::new(400.0, 200.0, dpr);
            let mut browser = ChartEngine::new(400.0, 200.0, dpr);
            for chart in [&mut native, &mut browser] {
                chart
                    .set_series_data(
                        0,
                        &[1.0, 2.0, 3.0, 4.0],
                        &[10.0; 4],
                        &[11.0; 4],
                        &[9.0; 4],
                        &[10.0; 4],
                    )
                    .unwrap();
                chart.time_scale.set_width(400.0);
                chart.fit_content();
            }
            let native_event = ScrollWheelEvent {
                position: point(px(200.0), px(100.0)),
                delta: ScrollDelta::Lines(point(0.0, 3.0)),
                ..ScrollWheelEvent::default()
            };
            assert!(adapter.scroll_wheel(&mut native, &native_event));
            assert!(browser.input_wheel(WheelSample {
                x: 200.0,
                y: 100.0,
                delta_y: 1.0,
                delta_mode: WheelDeltaMode::Pixel,
                ..WheelSample::default()
            }));
            assert_eq!(
                native.time_scale.bar_spacing(),
                browser.time_scale.bar_spacing()
            );
        }
    }
}

#[cfg(test)]
mod key_tests {
    use super::*;
    use aeris_charts_engine::{
        ChartInputEvent, DrawingKind, DrawingModifiers, DrawingPoint, InteractionOptions,
    };
    use gpui::Keystroke;

    fn press(chart: &mut ChartEngine, name: &str, modifiers: Modifiers) -> bool {
        let event = KeyDownEvent {
            keystroke: Keystroke {
                key: name.into(),
                modifiers,
                ..Keystroke::default()
            },
            is_held: false,
            prefer_character_input: false,
        };
        let Some(key) = chart_key_down(&event) else {
            return false;
        };
        chart.input_key_down(key, input_modifiers(&event.keystroke.modifiers), false, 0.0)
    }

    #[test]
    fn gpui_key_adapter_drives_shared_navigation_and_edit_bindings() {
        let bindings = [
            ("left", ChartKey::ArrowLeft),
            ("right", ChartKey::ArrowRight),
            ("pageup", ChartKey::PageUp),
            ("pagedown", ChartKey::PageDown),
            ("+", ChartKey::ZoomIn),
            ("-", ChartKey::ZoomOut),
            ("home", ChartKey::Home),
            ("end", ChartKey::End),
            ("enter", ChartKey::Enter),
            ("backspace", ChartKey::Backspace),
            ("delete", ChartKey::Delete),
            ("escape", ChartKey::Escape),
        ];
        for (name, expected) in bindings {
            assert_eq!(chart_key(name), Some(expected), "{name}");
        }

        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        let times: Vec<f64> = (0..200).map(|i| 1_000.0 + i as f64 * 60.0).collect();
        let open = vec![100.0; 200];
        let high = vec![102.0; 200];
        let low = vec![99.0; 200];
        let close = vec![101.0; 200];
        chart
            .set_series_data(0, &times, &open, &high, &low, &close)
            .unwrap();
        chart.recompute_layout_with_measure(true, |_, _| 48.0, |_, _| 48.0);
        chart.fit_content();
        chart.autoscale_visible();
        chart.scroll_to_position(-30.0);
        let start = chart.scroll_position();
        assert!(press(&mut chart, "pageup", Modifiers::default()));
        assert!(chart.scroll_position() < start);
        let older = chart.scroll_position();
        assert!(press(&mut chart, "pagedown", Modifiers::default()));
        assert!(chart.scroll_position() > older);
        assert!(press(&mut chart, "end", Modifiers::default()));
        assert!(
            chart.is_at_latest(),
            "End offset {} configured {}",
            chart.scroll_position(),
            chart.time_scale.options().right_offset
        );
        chart.set_interaction_options(InteractionOptions {
            reduced_motion: true,
            ..InteractionOptions::default()
        });
        chart.scroll_to_position(-30.0);
        assert!(press(&mut chart, "left", Modifiers::default()));
        assert_eq!(chart.scroll_position(), -31.0);
        assert!(press(&mut chart, "right", Modifiers::default()));
        assert_eq!(chart.scroll_position(), -30.0);

        let spacing = chart.bar_spacing();
        assert!(press(&mut chart, "+", Modifiers::default()));
        assert!(chart.bar_spacing() > spacing);
        let zoomed = chart.bar_spacing();
        assert!(press(&mut chart, "-", Modifiers::default()));
        assert!(chart.bar_spacing() < zoomed);
        chart.set_bar_spacing(20.0);
        assert!(press(&mut chart, "home", Modifiers::default()));
        assert_eq!(chart.bar_spacing(), 6.0);

        assert!(chart.set_drawing_tool(Some(DrawingKind::Rectangle), None, None));
        assert!(press(&mut chart, "escape", Modifiers::default()));
        assert_eq!(chart.active_drawing_tool(), None);
        assert_eq!(
            chart.take_input_events(),
            vec![ChartInputEvent::CrosshairLeft]
        );
        for key in ["delete", "backspace"] {
            let id = chart
                .add_drawing(
                    DrawingKind::Text,
                    0,
                    vec![DrawingPoint {
                        logical: 0.0,
                        price: 100.0,
                    }],
                    Some(r#"{"text":"note"}"#),
                )
                .unwrap();
            chart.set_selected_drawing(Some(id));
            assert!(press(&mut chart, key, Modifiers::default()));
            assert!(chart.drawing(id).is_none(), "{key}");
        }
        assert!(chart.set_drawing_tool(Some(DrawingKind::Path), None, None));
        chart.drawing_tool_activate(120.0, 120.0, DrawingModifiers::default());
        chart.drawing_tool_activate(200.0, 160.0, DrawingModifiers::default());
        assert!(press(&mut chart, "enter", Modifiers::default()));
        let [ChartInputEvent::DrawingCreated(id)] = chart.take_input_events()[..] else {
            panic!("GPUI Enter must report a committed path");
        };
        assert!(chart.drawing(id).is_some());
        chart.set_selected_drawing(None);
        chart.set_selected_series(Some(0));
        assert!(press(&mut chart, "delete", Modifiers::default()));
        assert_eq!(
            chart.take_input_events(),
            vec![ChartInputEvent::RemoveSeries(0)]
        );
    }

    #[test]
    fn gpui_shortcuts_use_the_engine_drawing_history() {
        let mut chart = ChartEngine::new(320.0, 200.0, 1.0);
        let id = chart
            .add_drawing(
                DrawingKind::Text,
                0,
                vec![DrawingPoint {
                    logical: 0.0,
                    price: 10.0,
                }],
                Some(r#"{"text":"note"}"#),
            )
            .unwrap();
        for (shift, expected_key) in [(false, ChartKey::Undo), (true, ChartKey::Redo)] {
            let event = KeyDownEvent {
                keystroke: Keystroke {
                    key: "z".into(),
                    modifiers: Modifiers {
                        control: true,
                        shift,
                        ..Modifiers::default()
                    },
                    ..Keystroke::default()
                },
                is_held: false,
                prefer_character_input: false,
            };
            let key = chart_key_down(&event).unwrap();
            assert_eq!(key, expected_key);
            assert!(chart.input_key_down(
                key,
                input_modifiers(&event.keystroke.modifiers),
                false,
                0.0
            ));
            assert_eq!(chart.drawing(id).is_some(), shift);
        }
    }
}

#[cfg(test)]
mod swipe_tests {
    use super::*;
    use gpui::{px, TouchPhase};

    fn engine() -> ChartEngine {
        let mut engine = ChartEngine::new(800.0, 400.0, 2.0);
        let bars = 600;
        let times: Vec<f64> = (0..bars)
            .map(|i| 1_600_000_000.0 + i as f64 * 60.0)
            .collect();
        let close: Vec<f64> = (0..bars)
            .map(|i| 100.0 + (i as f64 * 0.05).sin() * 12.0)
            .collect();
        let high: Vec<f64> = close.iter().map(|c| c + 1.0).collect();
        let low: Vec<f64> = close.iter().map(|c| c - 1.0).collect();
        engine
            .set_series_data(0, &times, &close, &high, &low, &close)
            .expect("series loads");
        let content_h = (400.0 - engine.time_axis_height()).max(1.0);
        engine.layout_panes(content_h);
        engine.time_scale.set_width(800.0);
        // Mid-history, so a wheel can move the view either way.
        engine.set_visible_logical_range(300.0, 400.0);
        engine
    }

    /// One swipe must move a GPUI chart exactly as it moves the same chart in a browser, which
    /// receives it as a `WheelEvent` with both signs reversed (`deltaX = -scrollingDeltaX`) and
    /// normalizes it to `(deltaX / 100, -deltaY / 100)`. A swipe that reveals the left must show
    /// earlier bars, as it shows the left of every native scroll view.
    #[test]
    fn a_wheel_or_swipe_moves_the_chart_as_the_browser_does() {
        let input = GpuiChartInput::default();
        let position = point(px(300.0), px(200.0));
        for (dx, dy) in [(40.0, 0.0), (-40.0, 0.0), (0.0, 40.0), (0.0, -40.0)] {
            let mut gpui = engine();
            let mut browser = engine();
            let start = gpui.visible_logical_range().expect("visible range");
            let event = ScrollWheelEvent {
                position,
                delta: ScrollDelta::Pixels(point(px(dx), px(dy))),
                modifiers: Modifiers::default(),
                touch_phase: TouchPhase::Moved,
            };
            assert!(
                input.scroll_wheel(&mut gpui, &event),
                "({dx}, {dy}) consumed"
            );
            let (x, y) = input.pane_point(&browser, position);
            let (dom_x, dom_y) = (-f64::from(dx), -f64::from(dy));
            assert!(browser.input_wheel(WheelSample {
                x,
                y,
                delta_x: dom_x / 100.0,
                delta_y: -dom_y / 100.0,
                delta_mode: WheelDeltaMode::Pixel,
                ..WheelSample::default()
            }));
            let moved = gpui.visible_logical_range().expect("visible range");
            assert_eq!(
                moved,
                browser.visible_logical_range().expect("visible range"),
                "({dx}, {dy})"
            );
            if dx > 0.0 {
                assert!(
                    moved.0 < start.0,
                    "revealing the left shows earlier bars: {start:?} -> {moved:?}"
                );
            }
        }
    }
}
