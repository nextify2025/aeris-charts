//! Interactive tests of the GPUI adapter: real GPUI event structs go through [`GpuiChartInput`]
//! into a live engine, and where the browser host normalizes the same gesture, a twin engine fed
//! the browser's formula must end in the same state.

use std::time::Duration;

use aeris_charts_engine::{
    ChartInputEvent, ChartRegion, DrawingId, DrawingKind, DrawingPoint, InteractionOptions,
    OrderId, OrderKind, OrderRole, OrderSide, OrderStatus, PriceScaleTarget, TradingHitKind,
    TradingPriceScale, TradingSnapshot, WorkingOrder, TRADING_TOOLTIP_DWELL_MS,
};
use aeris_charts_render::draw_list::Prim;
use gpui::{bounds, size, Keystroke, MouseButton};

use super::*;

const BARS: usize = 60;
/// Window position of the chart canvas reported by prepaint.
const CANVAS: (f32, f32) = (100.0, 40.0);

/// The engine controller fixture (60 bars on 800x500) with the left price scale shown, so the
/// plot starts right of the canvas edge and every pane point differs from its canvas point.
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
    assert!(chart.set_price_scale_visible_for(0, PriceScaleTarget::Left, true));
    chart.recompute_layout_with_measure(true, |_, _| 48.0, |_, _| 48.0);
    chart.fit_content();
    chart.autoscale_visible();
    assert!(chart.pane_left > 0.0, "the left scale offsets the plot");
    chart
}

fn input() -> GpuiChartInput {
    let mut input = GpuiChartInput::default();
    input.set_canvas_bounds(bounds(
        point(px(CANVAS.0), px(CANVAS.1)),
        size(px(800.0), px(500.0)),
    ));
    input
}

/// The window position of pane point `(x, y)` for a canvas at [`CANVAS`].
fn window(chart: &ChartEngine, x: f64, y: f64) -> Point<Pixels> {
    point(
        px((f64::from(CANVAS.0) + chart.pane_left + x) as f32),
        px((f64::from(CANVAS.1) + y) as f32),
    )
}

fn down(position: Point<Pixels>, click_count: usize) -> MouseDownEvent {
    MouseDownEvent {
        button: MouseButton::Left,
        position,
        click_count,
        ..MouseDownEvent::default()
    }
}

fn up(position: Point<Pixels>) -> MouseUpEvent {
    MouseUpEvent {
        button: MouseButton::Left,
        position,
        click_count: 1,
        ..MouseUpEvent::default()
    }
}

fn hover(position: Point<Pixels>) -> MouseMoveEvent {
    MouseMoveEvent {
        position,
        ..MouseMoveEvent::default()
    }
}

fn pressed(position: Point<Pixels>) -> MouseMoveEvent {
    MouseMoveEvent {
        position,
        pressed_button: Some(MouseButton::Left),
        ..MouseMoveEvent::default()
    }
}

fn wheel(position: Point<Pixels>, delta: ScrollDelta, modifiers: Modifiers) -> ScrollWheelEvent {
    ScrollWheelEvent {
        position,
        delta,
        modifiers,
        ..ScrollWheelEvent::default()
    }
}

fn key(key: &str, key_char: Option<&str>, modifiers: Modifiers) -> KeyDownEvent {
    KeyDownEvent {
        keystroke: Keystroke {
            modifiers,
            key: key.into(),
            key_char: key_char.map(str::to_string),
        },
        is_held: false,
        prefer_character_input: false,
    }
}

fn click(input: &GpuiChartInput, chart: &mut ChartEngine, at: (f64, f64), click_count: usize) {
    let position = window(chart, at.0, at.1);
    input.mouse_down(chart, &down(position, click_count));
    input.mouse_up(chart, &up(position));
}

fn drag(input: &GpuiChartInput, chart: &mut ChartEngine, from: (f64, f64), to: (f64, f64)) {
    let start = window(chart, from.0, from.1);
    input.mouse_down(chart, &down(start, 1));
    for step in 1..=4 {
        let t = f64::from(step) / 4.0;
        let position = window(
            chart,
            from.0 + (to.0 - from.0) * t,
            from.1 + (to.1 - from.1) * t,
        );
        input.mouse_move(chart, &pressed(position));
    }
    let end = window(chart, to.0, to.1);
    input.mouse_up(chart, &up(end));
}

fn assert_near(actual: (f64, f64), expected: (f64, f64)) {
    assert!(
        (actual.0 - expected.0).abs() < 1e-3 && (actual.1 - expected.1).abs() < 1e-3,
        "{actual:?} != {expected:?}"
    );
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

/// An open typing session on a text drawing that reads `text`, with the caret at its end.
fn text_session(chart: &mut ChartEngine, text: &str) -> DrawingId {
    let options = format!(r#"{{"text":"{text}"}}"#);
    let id = chart
        .add_drawing(
            DrawingKind::Text,
            0,
            vec![DrawingPoint {
                logical: 20.0,
                price: 103.0,
            }],
            Some(&options),
        )
        .unwrap();
    chart.build_frame();
    assert!(chart.begin_drawing_text_edit(id, true));
    assert_eq!(
        chart.drawing_text_edit(),
        Some((id, text, text.chars().count()))
    );
    id
}

#[test]
fn pane_point_subtracts_the_canvas_corner_and_the_left_price_scale() {
    let mut chart = chart();
    let mut input = GpuiChartInput::default();
    let left = chart.pane_left;

    // Before prepaint reports bounds the canvas sits at the window's top-left corner.
    let position = point(px((left + 200.0) as f32), px(150.0));
    input.mouse_move(&mut chart, &hover(position));
    assert_near(chart.crosshair.unwrap(), (200.0, 150.0));

    input.set_canvas_bounds(bounds(
        point(px(CANVAS.0), px(CANVAS.1)),
        size(px(800.0), px(500.0)),
    ));
    let position = window(&chart, 260.0, 210.0);
    input.mouse_move(&mut chart, &hover(position));
    assert_near(chart.crosshair.unwrap(), (260.0, 210.0));
    assert_near(
        input.pane_point(&chart, point(px(CANVAS.0), px(CANVAS.1))),
        (-left, 0.0),
    );

    // A secondary press resolves its region and context at the same pane point.
    let (sx, sy) = series_point(&chart);
    let position = window(&chart, sx, sy);
    input.context_menu(
        &mut chart,
        &MouseDownEvent {
            button: MouseButton::Right,
            position,
            click_count: 1,
            ..MouseDownEvent::default()
        },
    );
    let events = chart.take_input_events();
    let [ChartInputEvent::ContextMenu(menu)] = events[..] else {
        panic!("one context menu: {events:?}");
    };
    assert_eq!(menu.region, ChartRegion::Pane);
    assert_near((menu.x, menu.y), (sx, sy));
    assert_eq!(menu.context.and_then(|context| context.series), Some(0));
}

#[test]
fn mouse_events_pan_past_the_slop_select_on_click_and_abandon_a_lost_release() {
    let mut chart = chart();
    let input = input();
    let series = series_point(&chart);
    click(&input, &mut chart, series, 1);
    assert_eq!(chart.selected_series(), Some(0));

    let (x, y) = empty_pane_point(&chart);
    let start = chart.scroll_position();
    let spacing = chart.bar_spacing();
    let positions: Vec<_> = [0.0, 2.0, 30.0, 60.0]
        .into_iter()
        .map(|dx| window(&chart, x + dx, y))
        .collect();
    input.mouse_down(&mut chart, &down(positions[0], 1));
    input.mouse_move(&mut chart, &pressed(positions[1]));
    assert_eq!(
        chart.scroll_position(),
        start,
        "below the 5 px slop nothing pans"
    );
    input.mouse_move(&mut chart, &pressed(positions[2]));
    assert_eq!(
        chart.scroll_position(),
        start,
        "the threshold sample opens the pan"
    );
    assert_eq!(cursor_style(chart.input_cursor()), CursorStyle::ClosedHand);
    input.mouse_move(&mut chart, &pressed(positions[3]));
    input.mouse_up(&mut chart, &up(positions[3]));
    // The view moves from the sample after the threshold: 30 px reveal 30 / spacing older bars.
    let panned = chart.scroll_position();
    assert!(
        (panned - (start - 30.0 / spacing)).abs() < 1e-6,
        "{start} -> {panned}"
    );
    assert_eq!(
        chart.selected_series(),
        Some(0),
        "a pan never clicks empty space"
    );

    let position = window(&chart, series.0, series.1);
    input.mouse_move(&mut chart, &hover(position));
    assert_eq!(
        cursor_style(chart.input_cursor()),
        CursorStyle::PointingHand
    );

    // A release outside the window never arrives: the next motion reports no held button.
    let (id, body) = trend_line_body(&mut chart);
    let before = chart.drawing(id).unwrap().points.clone();
    let press = window(&chart, body.0, body.1);
    input.mouse_down(&mut chart, &down(press, 1));
    let dragged = window(&chart, body.0 + 50.0, body.1 - 40.0);
    input.mouse_move(&mut chart, &pressed(dragged));
    assert!(chart.drawing_drag_active());
    assert_ne!(chart.drawing(id).unwrap().points, before);
    let away = window(&chart, 400.0, 300.0);
    input.mouse_move(&mut chart, &hover(away));
    assert_eq!(chart.drawing(id).unwrap().points, before);
    assert!(!chart.drawing_drag_active());
    assert_near(chart.crosshair.unwrap(), (400.0, 300.0));
    assert_ne!(cursor_style(chart.input_cursor()), CursorStyle::ClosedHand);
}

#[test]
fn click_count_two_resets_the_price_axis_and_the_time_axis() {
    let mut chart = chart();
    let input = input();
    let right = PriceScaleTarget::Right;

    let axis = (chart.pane_w + 10.0, 100.0);
    assert_eq!(
        chart.region_at(axis.0, axis.1),
        ChartRegion::PriceAxis {
            pane: 0,
            target: right
        }
    );
    drag(&input, &mut chart, axis, (axis.0, axis.1 + 60.0));
    assert_eq!(chart.price_scale_auto_scale_for(0, right), Some(false));
    click(&input, &mut chart, axis, 1);
    assert_eq!(
        chart.price_scale_auto_scale_for(0, right),
        Some(false),
        "a single click is not a reset"
    );
    click(&input, &mut chart, axis, 2);
    assert_eq!(chart.price_scale_auto_scale_for(0, right), Some(true));

    let time = (200.0, chart.pane_h + 4.0);
    assert_eq!(chart.region_at(time.0, time.1), ChartRegion::TimeAxis);
    drag(&input, &mut chart, time, (time.0 - 80.0, time.1));
    let mut reset = self::chart();
    reset.reset_time_scale();
    let dragged = (chart.bar_spacing(), chart.scroll_position());
    assert_ne!(dragged.0, reset.bar_spacing());
    click(&input, &mut chart, time, 1);
    assert_eq!((chart.bar_spacing(), chart.scroll_position()), dragged);
    click(&input, &mut chart, time, 2);
    assert_eq!(chart.bar_spacing(), reset.bar_spacing());
    assert_eq!(chart.scroll_position(), reset.scroll_position());
}

/// The browser host normalizes a DOM wheel as `delta_y = -(speed * deltaY) / 100` with a 32 px
/// line speed (`packages/charts/src/gestures.ts` `route_wheel`), so a DOM wheel-up is positive.
#[test]
fn vertical_wheel_lines_and_pixels_match_the_browser_normalization() {
    let input = input();
    for (delta, dom_delta_y, speed, delta_mode) in [
        (
            ScrollDelta::Lines(point(0.0, 1.0)),
            -1.0,
            32.0,
            WheelDeltaMode::Line,
        ),
        (
            ScrollDelta::Pixels(point(px(0.0), px(100.0))),
            -100.0,
            1.0,
            WheelDeltaMode::Pixel,
        ),
    ] {
        let mut chart = chart();
        let mut twin = self::chart();
        let spacing = chart.bar_spacing();
        let position = window(&chart, 300.0, 200.0);
        assert!(input.scroll_wheel(&mut chart, &wheel(position, delta, Modifiers::default())));

        let (x, y) = input.pane_point(&twin, position);
        assert!(twin.input_wheel(WheelSample {
            x,
            y,
            delta_y: -(speed * dom_delta_y) / 100.0,
            delta_mode,
            ..WheelSample::default()
        }));
        assert_eq!(chart.bar_spacing(), twin.bar_spacing(), "{delta:?}");
        assert_eq!(chart.scroll_position(), twin.scroll_position(), "{delta:?}");
        assert!(
            chart.bar_spacing() > spacing,
            "wheel-up zooms in: {delta:?}"
        );
    }
}

/// GPUI's horizontal delta is the negation of the DOM's `deltaX` (a swipe the DOM reports as
/// scrolling right arrives negative), while the browser host passes `+(speed * deltaX) / 100`.
#[test]
fn horizontal_wheel_pans_in_the_browser_direction() {
    let input = input();
    for (delta, dom_delta_x, speed, delta_mode) in [
        (
            ScrollDelta::Pixels(point(px(-100.0), px(0.0))),
            100.0,
            1.0,
            WheelDeltaMode::Pixel,
        ),
        (
            ScrollDelta::Lines(point(-1.0, 0.0)),
            1.0,
            32.0,
            WheelDeltaMode::Line,
        ),
    ] {
        let mut chart = chart();
        let mut twin = self::chart();
        let start = chart.scroll_position();
        let position = window(&chart, 300.0, 200.0);
        assert!(input.scroll_wheel(&mut chart, &wheel(position, delta, Modifiers::default())));

        let (x, y) = input.pane_point(&twin, position);
        assert!(twin.input_wheel(WheelSample {
            x,
            y,
            delta_x: speed * dom_delta_x / 100.0,
            delta_mode,
            ..WheelSample::default()
        }));
        assert_ne!(chart.scroll_position(), start, "{delta:?}");
        assert_eq!(chart.scroll_position(), twin.scroll_position(), "{delta:?}");
        assert_eq!(chart.bar_spacing(), twin.bar_spacing(), "{delta:?}");
    }
}

#[test]
fn an_unconsumed_wheel_or_pinch_reports_false_and_a_pinch_zooms_at_the_pointer() {
    let input = input();
    let mut chart = chart();
    chart.set_interaction_options(InteractionOptions {
        wheel_zoom: false,
        wheel_scroll: false,
        ..InteractionOptions::default()
    });
    let view = (chart.bar_spacing(), chart.scroll_position());
    let position = window(&chart, 300.0, 200.0);
    for delta in [
        ScrollDelta::Pixels(point(px(0.0), px(100.0))),
        ScrollDelta::Pixels(point(px(-100.0), px(0.0))),
        ScrollDelta::Lines(point(1.0, 1.0)),
    ] {
        assert!(
            !input.scroll_wheel(&mut chart, &wheel(position, delta, Modifiers::default())),
            "{delta:?} is left to the host"
        );
    }
    assert_eq!((chart.bar_spacing(), chart.scroll_position()), view);

    let mut chart = self::chart();
    let x = 300.0;
    let position = window(&chart, x, 200.0);
    let index = chart.time_scale.coordinate_to_float_index(x);
    let spacing = chart.bar_spacing();
    let pinch = |delta| PinchEvent {
        position,
        delta,
        ..PinchEvent::default()
    };
    assert!(input.pinch(&mut chart, &pinch(0.1)));
    assert!(chart.bar_spacing() > spacing);
    let after = chart.time_scale.coordinate_to_float_index(x);
    assert!((after - index).abs() < 1e-6, "{index} -> {after}");
}

#[test]
fn input_modifiers_map_the_platform_key_to_meta_and_ignore_function() {
    let cases = [
        (
            Modifiers {
                shift: true,
                ..Modifiers::default()
            },
            InputModifiers {
                shift: true,
                ..InputModifiers::default()
            },
        ),
        (
            Modifiers {
                control: true,
                ..Modifiers::default()
            },
            InputModifiers {
                control: true,
                ..InputModifiers::default()
            },
        ),
        (
            Modifiers {
                alt: true,
                ..Modifiers::default()
            },
            InputModifiers {
                alt: true,
                ..InputModifiers::default()
            },
        ),
        (
            Modifiers {
                platform: true,
                function: true,
                ..Modifiers::default()
            },
            InputModifiers {
                meta: true,
                ..InputModifiers::default()
            },
        ),
        (
            Modifiers {
                function: true,
                ..Modifiers::default()
            },
            InputModifiers::default(),
        ),
    ];
    for (gpui, engine) in cases {
        assert_eq!(input_modifiers(&gpui), engine, "{gpui:?}");
    }
}

#[test]
fn command_or_control_wheel_zooms_at_the_pointer_and_a_plain_wheel_pins_the_right_edge() {
    let input = input();
    let delta = ScrollDelta::Pixels(point(px(0.0), px(100.0)));
    let mut plain = chart();
    let x = 200.0;
    let position = window(&plain, x, 200.0);
    let offset = plain.time_scale.right_offset();
    let index = plain.time_scale.coordinate_to_float_index(x);

    assert!(input.scroll_wheel(&mut plain, &wheel(position, delta, Modifiers::default())));
    assert_eq!(plain.time_scale.right_offset(), offset);
    let moved = plain.time_scale.coordinate_to_float_index(x);
    assert!(
        (moved - index).abs() > 0.5,
        "a plain wheel keeps the newest bar, not the pointer: {index} -> {moved}"
    );

    for modifiers in [
        Modifiers {
            platform: true,
            ..Modifiers::default()
        },
        Modifiers {
            control: true,
            ..Modifiers::default()
        },
    ] {
        let mut focused = chart();
        assert!(input.scroll_wheel(&mut focused, &wheel(position, delta, modifiers)));
        let after = focused.time_scale.coordinate_to_float_index(x);
        assert!(
            (after - index).abs() < 1e-6,
            "{modifiers:?}: {index} -> {after}"
        );
        assert_ne!(focused.time_scale.right_offset(), offset, "{modifiers:?}");
    }
}

#[test]
fn modifier_changes_toggle_the_ohlc_magnet_while_a_tool_is_armed() {
    let mut chart = chart();
    let input = input();
    assert!(chart.set_drawing_tool(Some(DrawingKind::TrendLine), None, None));
    let position = window(&chart, 300.0, 200.0);
    input.mouse_move(&mut chart, &hover(position));
    assert!(!chart.crosshair_ohlc_magnet);

    let changed = |modifiers| ModifiersChangedEvent {
        modifiers,
        ..ModifiersChangedEvent::default()
    };
    input.modifiers_changed(
        &mut chart,
        &changed(Modifiers {
            control: true,
            ..Modifiers::default()
        }),
    );
    assert!(chart.crosshair_ohlc_magnet);
    input.modifiers_changed(&mut chart, &changed(Modifiers::default()));
    assert!(!chart.crosshair_ohlc_magnet);
    input.modifiers_changed(
        &mut chart,
        &changed(Modifiers {
            platform: true,
            ..Modifiers::default()
        }),
    );
    assert!(
        chart.crosshair_ohlc_magnet,
        "the platform key is meta, the macOS magnet modifier"
    );
    input.modifiers_changed(
        &mut chart,
        &changed(Modifiers {
            function: true,
            ..Modifiers::default()
        }),
    );
    assert!(!chart.crosshair_ohlc_magnet);
}

#[test]
fn chart_key_maps_exactly_the_bound_gpui_key_names() {
    for (name, expected) in [
        ("left", ChartKey::ArrowLeft),
        ("right", ChartKey::ArrowRight),
        ("pageup", ChartKey::PageUp),
        ("pagedown", ChartKey::PageDown),
        ("+", ChartKey::ZoomIn),
        ("=", ChartKey::ZoomIn),
        ("-", ChartKey::ZoomOut),
        ("_", ChartKey::ZoomOut),
        ("home", ChartKey::Home),
        ("end", ChartKey::End),
        ("enter", ChartKey::Enter),
        ("f2", ChartKey::EditText),
        ("backspace", ChartKey::Backspace),
        ("delete", ChartKey::Delete),
        ("escape", ChartKey::Escape),
    ] {
        assert_eq!(chart_key(name), Some(expected), "{name}");
    }
    for name in ["up", "down", "tab", "space", "a", "f3", ""] {
        assert_eq!(chart_key(name), None, "{name}");
    }
}

#[test]
fn key_up_stops_only_the_held_arrow_pan() {
    let mut chart = chart();
    let input = input();
    let left = chart_key("left").unwrap();
    let none = input_modifiers(&Modifiers::default());
    assert!(chart.input_key_down(left, none, false, input.now_ms()));
    assert!(chart.input_animating());

    let release = |name: &str| KeyUpEvent {
        keystroke: Keystroke {
            key: name.into(),
            ..Keystroke::default()
        },
    };
    assert!(!input.key_up(&mut chart, &release("right")));
    assert!(!input.key_up(&mut chart, &release("a")));
    assert!(
        chart.input_animating(),
        "another key leaves the pan running"
    );
    assert!(input.key_up(&mut chart, &release("left")));
    assert!(!chart.input_animating());
}

#[test]
fn cursor_style_maps_every_chart_cursor() {
    let windows = cfg!(target_os = "windows");
    for (cursor, expected) in [
        (ChartCursor::Crosshair, CursorStyle::Crosshair),
        (ChartCursor::Default, CursorStyle::Arrow),
        (ChartCursor::Pointer, CursorStyle::PointingHand),
        (ChartCursor::Text, CursorStyle::IBeam),
        (ChartCursor::Move, CursorStyle::OpenHand),
        (ChartCursor::Grabbing, CursorStyle::ClosedHand),
        (
            ChartCursor::VerticalGrab,
            if windows {
                CursorStyle::ResizeUpDown
            } else {
                CursorStyle::OpenHand
            },
        ),
        (
            ChartCursor::VerticalGrabbing,
            if windows {
                CursorStyle::ResizeUpDown
            } else {
                CursorStyle::ClosedHand
            },
        ),
        (ChartCursor::ResizeVertical, CursorStyle::ResizeUpDown),
        (ChartCursor::ResizeHorizontal, CursorStyle::ResizeLeftRight),
        (ChartCursor::ResizeNwse, CursorStyle::ResizeUpLeftDownRight),
        (ChartCursor::ResizeNesw, CursorStyle::ResizeUpRightDownLeft),
        (ChartCursor::ResizeRow, CursorStyle::ResizeRow),
    ] {
        assert_eq!(cursor_style(cursor), expected, "{cursor:?}");
    }
}

/// Wayland delivers pressed motion per HID report, often one axis per event. The prepaint step
/// forwards only the newest captured sample, so a stroke gains one knot per frame, not per event.
#[test]
fn prepare_frame_forwards_one_coalesced_brush_knot_per_prepaint() {
    let mut chart = chart();
    let input = input();
    assert!(chart.set_drawing_tool(Some(DrawingKind::Brush), None, None));
    let start = window(&chart, 100.0, 100.0);
    input.mouse_down(&mut chart, &down(start, 1));

    let mut at = (100.0, 100.0);
    let mut knots = Vec::new();
    for _ in 0..2 {
        for step in 0..12 {
            if step % 2 == 0 {
                at.0 += 1.5;
            } else {
                at.1 += 1.5;
            }
            let position = window(&chart, at.0, at.1);
            input.mouse_move(&mut chart, &pressed(position));
        }
        assert!(
            input.prepare_frame(&mut chart),
            "the newest sample is a knot"
        );
        assert!(
            !input.prepare_frame(&mut chart),
            "an idle prepaint adds nothing"
        );
        knots.push(input.pane_point(&chart, window(&chart, at.0, at.1)));
    }
    let release = window(&chart, 130.0, 125.0);
    input.mouse_up(&mut chart, &up(release));

    let events = chart.take_input_events();
    let [ChartInputEvent::DrawingCreated(id)] = events[..] else {
        panic!("the brush commits: {events:?}");
    };
    // The press, one knot per prepaint, and the release point. Each knot is the newest sample of
    // its frame; the samples before it lie at least 1.5 px away.
    assert_eq!(chart.drawing(id).unwrap().points.len(), 4);
    for (index, knot) in knots.into_iter().enumerate() {
        assert_near(
            chart.drawing_point_to_coordinate(id, index + 1).unwrap(),
            knot,
        );
    }
}

#[test]
fn wake_delay_schedules_the_close_button_tooltip_dwell() {
    let mut chart = chart();
    let mut input = input();
    assert_eq!(input.wake_delay(&chart), None);

    let price = 106.0;
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
    chart.build_frame();
    let y = chart.series_price_to_coordinate(0, price).unwrap();
    let close_x = (0..=(chart.pane_w * 2.0) as usize)
        .map(|step| step as f64 / 2.0)
        .find(|&x| {
            chart
                .trading_hit_at(x, y)
                .is_some_and(|hit| hit.kind == TradingHitKind::CancelButton)
        })
        .expect("the order shows a close button");
    let tooltip_shown = |chart: &mut ChartEngine| {
        chart.build_frame().panes.iter().any(|pane| {
            pane.main
                .iter()
                .any(|prim| matches!(prim, Prim::Text { text, .. } if text == "Cancel order"))
        })
    };

    let close = window(&chart, close_x, y);
    let before = input.now_ms();
    input.mouse_move(&mut chart, &hover(close));
    let after = input.now_ms();
    let deadline = chart
        .input_wake_deadline_ms()
        .expect("hovering the close button starts the dwell");
    assert!(
        (before + TRADING_TOOLTIP_DWELL_MS..=after + TRADING_TOOLTIP_DWELL_MS).contains(&deadline),
        "the dwell runs on the adapter clock: {before}..{after} -> {deadline}"
    );
    let delay = input.wake_delay(&chart).expect("the host schedules a wake");
    assert!(delay <= Duration::from_millis(450));
    assert!(!tooltip_shown(&mut chart));

    // Leaving the button before the dwell elapses cancels the pending wake.
    let away = window(&chart, close_x, y + 60.0);
    input.mouse_move(&mut chart, &hover(away));
    assert_eq!(input.wake_delay(&chart), None);
    input.mouse_move(&mut chart, &hover(close));
    assert!(
        input.wake_delay(&chart).is_some(),
        "re-entering restarts the dwell"
    );

    // The host's wake repaints once the deadline passes, and that prepaint tick reveals the
    // tooltip. Back-dating the adapter clock by the dwell stands in for the wait.
    input.epoch = input
        .epoch
        .checked_sub(Duration::from_millis(TRADING_TOOLTIP_DWELL_MS as u64))
        .expect("the monotonic clock can be back-dated by the dwell");
    assert_eq!(
        input.wake_delay(&chart),
        Some(Duration::ZERO),
        "the wake is due"
    );
    assert!(
        input.prepare_frame(&mut chart),
        "the prepaint tick arms the tooltip"
    );
    assert!(tooltip_shown(&mut chart));
    assert_eq!(input.wake_delay(&chart), None);
    assert!(
        !input.prepare_frame(&mut chart),
        "an idle prepaint changes nothing"
    );

    // A press on the button drops a pending wake.
    input.mouse_move(&mut chart, &hover(away));
    input.mouse_move(&mut chart, &hover(close));
    assert!(input.wake_delay(&chart).is_some());
    input.mouse_down(&mut chart, &down(close, 1));
    assert_eq!(input.wake_delay(&chart), None);
}

#[test]
fn text_edit_word_keys_follow_the_platform_word_modifier() {
    let mut chart = chart();
    let word = if cfg!(target_os = "macos") {
        Modifiers {
            alt: true,
            ..Modifiers::default()
        }
    } else {
        Modifiers {
            control: true,
            ..Modifiers::default()
        }
    };
    let id = text_session(&mut chart, "one two three");
    text_edit_key(&mut chart, &key("left", None, word));
    assert_eq!(chart.drawing_text_edit(), Some((id, "one two three", 8)));
    text_edit_key(&mut chart, &key("backspace", None, word));
    assert_eq!(chart.drawing_text_edit(), Some((id, "one three", 4)));
    text_edit_key(&mut chart, &key("delete", None, word));
    assert_eq!(chart.drawing_text_edit(), Some((id, "one ", 4)));
    text_edit_key(&mut chart, &key("left", None, word));
    assert_eq!(chart.drawing_text_edit(), Some((id, "one ", 0)));
    text_edit_key(&mut chart, &key("right", None, word));
    assert_eq!(chart.drawing_text_edit(), Some((id, "one ", 4)));

    // Shift with the word modifier selects whole words.
    let select = Modifiers {
        shift: true,
        ..word
    };
    text_edit_key(&mut chart, &key("left", None, select));
    assert_eq!(chart.drawing_text_edit_selection(), Some("one "));

    if cfg!(target_os = "macos") {
        let command = Modifiers {
            platform: true,
            ..Modifiers::default()
        };
        text_edit_key(&mut chart, &key("right", None, command));
        assert_eq!(chart.drawing_text_edit(), Some((id, "one ", 4)));
        text_edit_key(&mut chart, &key("left", None, command));
        assert_eq!(chart.drawing_text_edit(), Some((id, "one ", 0)));
    }
}

#[test]
fn text_edit_deletes_forward_selects_with_shift_and_types_only_characters() {
    let mut chart = chart();
    let none = Modifiers::default();
    let shift = Modifiers {
        shift: true,
        ..Modifiers::default()
    };
    let id = text_session(&mut chart, "hello world");
    text_edit_key(&mut chart, &key("up", None, none));
    assert_eq!(chart.drawing_text_edit(), Some((id, "hello world", 0)));
    text_edit_key(&mut chart, &key("delete", None, none));
    assert_eq!(chart.drawing_text_edit(), Some((id, "ello world", 0)));

    text_edit_key(&mut chart, &key("right", None, shift));
    text_edit_key(&mut chart, &key("right", None, shift));
    assert_eq!(chart.drawing_text_edit_selection(), Some("el"));
    // A space key without a committed character still types a space, replacing the selection.
    text_edit_key(&mut chart, &key("space", None, none));
    assert_eq!(chart.drawing_text_edit(), Some((id, " lo world", 1)));

    let function = Modifiers {
        function: true,
        ..Modifiers::default()
    };
    text_edit_key(&mut chart, &key("x", Some("x"), function));
    text_edit_key(&mut chart, &key("space", None, function));
    assert_eq!(chart.drawing_text_edit(), Some((id, " lo world", 1)));

    text_edit_key(&mut chart, &key("down", None, none));
    assert_eq!(chart.drawing_text_edit(), Some((id, " lo world", 9)));
}
