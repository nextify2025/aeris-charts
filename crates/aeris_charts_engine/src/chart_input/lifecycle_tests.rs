//! Gesture lifecycles through the input controller: cancellation, lost releases, secondary
//! clicks during a gesture, pointer leave, and the bounded host-request queue.

use super::tests::*;
use super::*;

/// A fast rightward flick from `(x, y)` that is still held: the press at 1000 ms, then 40 px
/// every 8 ms, ending at `x + 140` at 1032 ms.
fn held_flick(chart: &mut ChartEngine, (x, y): (f64, f64)) {
    chart.input_pointer_down(at_ms(x, y, 1_000.0), 1);
    for step in 1..=4 {
        let step = f64::from(step);
        let sample = at_ms(x + 40.0 * step - 20.0, y, 1_000.0 + 8.0 * step);
        chart.input_pointer_move(sample, true);
    }
}

fn price_range(chart: &ChartEngine) -> (f64, f64) {
    chart
        .price_scale_visible_range_for(0, PriceScaleTarget::Right)
        .unwrap()
}

#[test]
fn cancel_mid_pan_closes_the_session_without_a_coast() {
    let mut released = kinetic_chart();
    let start = empty_pane_point(&released);
    held_flick(&mut released, start);
    released.input_pointer_up(at_ms(start.0 + 140.0, start.1, 1_036.0));
    assert!(
        released.input_animating(),
        "the same flick coasts when it is released"
    );

    let mut chart = kinetic_chart();
    held_flick(&mut chart, start);
    chart.input_cancel();
    // The button comes up after the cancel; that release does not start a coast.
    chart.input_pointer_up(at_ms(start.0 + 140.0, start.1, 1_036.0));
    assert!(!chart.input_animating());
    let position = chart.scroll_position();
    assert!(!chart.input_tick(1_100.0));
    assert_eq!(chart.scroll_position(), position);

    // The next drag opens its own scroll session from its own press.
    let mut fresh = kinetic_chart();
    let fresh_start = fresh.scroll_position();
    drag(&mut fresh, start, (start.0 + 40.0, start.1));
    drag(&mut chart, start, (start.0 + 40.0, start.1));
    let delta = chart.scroll_position() - position;
    let fresh_delta = fresh.scroll_position() - fresh_start;
    assert!(fresh_delta != 0.0);
    assert!(
        (delta - fresh_delta).abs() < 1e-9,
        "{delta} vs {fresh_delta}"
    );
}

/// A scale session left open would ignore the next press and keep scaling from the first
/// press's snapshot. A closed one scales from the new press by the same factor a fresh chart
/// applies.
#[test]
fn cancel_mid_price_axis_drag_ends_the_scale_session() {
    let mut fresh = chart();
    let mut chart = chart();
    let axis_x = chart.pane_w + 10.0;
    chart.input_pointer_down(at(axis_x, 100.0), 1);
    chart.input_pointer_move(at(axis_x, 130.0), true);
    chart.input_pointer_move(at(axis_x, 160.0), true);
    chart.input_cancel();

    let span = |(low, high): (f64, f64)| high - low;
    let before = price_range(&chart);
    let fresh_before = price_range(&fresh);
    drag(&mut chart, (axis_x, 200.0), (axis_x, 260.0));
    drag(&mut fresh, (axis_x, 200.0), (axis_x, 260.0));
    let factor = span(price_range(&chart)) / span(before);
    let fresh_factor = span(price_range(&fresh)) / span(fresh_before);
    assert!((fresh_factor - 1.0).abs() > 1e-3, "{fresh_factor}");
    assert!(
        (factor - fresh_factor).abs() < 1e-9,
        "{factor} vs {fresh_factor}"
    );
}

#[test]
fn cancel_mid_freehand_stroke_discards_it_and_keeps_the_brush_armed() {
    let mut chart = chart();
    assert!(chart.set_drawing_tool(Some(DrawingKind::Brush), None, None));
    chart.input_pointer_down(at(100.0, 100.0), 1);
    chart.input_pointer_move(at(140.0, 120.0), true);
    assert!(chart.flush_coalesced_input());
    chart.input_pointer_move(at(180.0, 140.0), true);
    assert!(chart.drawing_tool_capture_active());

    chart.input_cancel();
    assert!(!chart.drawing_tool_capture_active());
    assert!(
        !chart.flush_coalesced_input(),
        "the pending sample goes with the stroke"
    );
    chart.input_pointer_up(at(180.0, 140.0));
    assert!(chart.drawings().is_empty());
    assert!(chart.take_input_events().is_empty());
    assert_eq!(chart.active_drawing_tool(), Some(DrawingKind::Brush));

    // The next stroke starts clean: none of the abandoned samples leak into it.
    chart.input_pointer_down(at(300.0, 200.0), 1);
    chart.input_pointer_move(at(340.0, 220.0), true);
    assert!(chart.flush_coalesced_input());
    chart.input_pointer_up(at(380.0, 240.0));
    let events = chart.take_input_events();
    let [ChartInputEvent::DrawingCreated(id)] = events[..] else {
        panic!("the second stroke commits: {events:?}");
    };
    assert_eq!(chart.drawing(id).unwrap().points.len(), 3);
    let first = chart.drawing_point_to_coordinate(id, 0).unwrap();
    assert!(
        (first.0 - 300.0).abs() < 0.5 && (first.1 - 200.0).abs() < 0.5,
        "{first:?}"
    );
}

#[test]
fn a_press_after_a_lost_release_restores_the_open_drawing_drag_and_pans_normally() {
    let mut twin = chart();
    trend_line_body(&mut twin);
    let mut chart = chart();
    let (id, body) = trend_line_body(&mut chart);
    let before = chart.drawing(id).unwrap().points.clone();
    let start = empty_pane_point(&chart);
    assert_eq!(chart.drawing_at(start.0, start.1), None);
    let initial = chart.scroll_position();

    chart.input_pointer_down(at(body.0, body.1), 1);
    chart.input_pointer_move(at(body.0 + 40.0, body.1 - 30.0), true);
    assert_ne!(chart.drawing(id).unwrap().points, before);
    // The release never arrived; the next thing the chart sees is a new press that pans.
    drag(&mut chart, start, (start.0 + 40.0, start.1));
    assert_eq!(chart.drawing(id).unwrap().points, before);
    assert!(!chart.drawing_drag_active());

    drag(&mut twin, start, (start.0 + 40.0, start.1));
    assert_ne!(chart.scroll_position(), initial);
    assert_eq!(chart.scroll_position(), twin.scroll_position());
    // The abandoned drag recorded nothing: the creation is the only history entry.
    assert!(chart.undo_drawing());
    assert!(chart.drawing(id).is_none());
}

#[test]
fn a_press_after_a_lost_release_shows_the_cursor_of_its_own_gesture() {
    let mut twin = chart();
    let mut chart = chart();
    let (_, body) = trend_line_body(&mut chart);
    let axis = (chart.pane_w + 10.0, 120.0);
    twin.input_pointer_down(at(axis.0, axis.1), 1);
    assert_eq!(twin.input_cursor(), ChartCursor::ResizeVertical);

    chart.input_pointer_down(at(body.0, body.1), 1);
    chart.input_pointer_move(at(body.0 + 40.0, body.1 - 30.0), true);
    // The release never arrived; abandoning that drag must not forget where the new press is.
    chart.input_pointer_down(at(axis.0, axis.1), 1);
    assert_eq!(chart.input_cursor(), ChartCursor::ResizeVertical);
}

#[test]
fn a_context_menu_mid_drawing_drag_restores_the_drawing_and_still_opens() {
    let mut chart = chart();
    let (id, body) = trend_line_body(&mut chart);
    let before = chart.drawing(id).unwrap().points.clone();
    let (x, y) = (body.0 + 40.0, body.1 - 30.0);
    chart.input_pointer_down(at(body.0, body.1), 1);
    chart.input_pointer_move(at(x, y), true);
    assert_ne!(chart.drawing(id).unwrap().points, before);

    chart.input_context_menu(x, y);
    assert_eq!(chart.drawing(id).unwrap().points, before);
    assert!(!chart.drawing_drag_active());
    let events = chart.take_input_events();
    let [ChartInputEvent::ContextMenu(menu)] = events[..] else {
        panic!("one context menu: {events:?}");
    };
    assert_eq!((menu.x, menu.y, menu.region), (x, y, ChartRegion::Pane));

    // The primary button comes up after the menu; that release commits nothing.
    chart.input_pointer_up(at(x, y));
    assert_eq!(chart.drawing(id).unwrap().points, before);
    assert!(chart.undo_drawing());
    assert!(chart.drawing(id).is_none());
}

#[test]
fn a_context_menu_commits_the_open_text_edit() {
    let mut chart = chart();
    let (id, body) = trend_line_body(&mut chart);
    let original = chart.drawing(id).unwrap().text.clone();
    chart.set_selected_drawing(Some(id));
    assert!(chart.input_key_down(ChartKey::EditText, InputModifiers::default(), false, 0.0));
    assert_eq!(
        chart.take_input_events(),
        vec![ChartInputEvent::TextEditorOpened(id)]
    );
    assert!(chart.drawing_text_edit_insert("note"));

    chart.input_context_menu(body.0, body.1);
    assert_eq!(chart.editing_drawing(), None);
    assert_eq!(chart.drawing(id).unwrap().text, "note");
    let events = chart.take_input_events();
    assert!(
        matches!(events[..], [ChartInputEvent::ContextMenu(_)]),
        "{events:?}"
    );
    // A commit is one undoable edit; a cancel would have recorded nothing.
    assert!(chart.undo_drawing());
    assert_eq!(chart.drawing(id).unwrap().text, original);
}

#[test]
fn the_host_event_queue_keeps_only_the_newest_requests() {
    let mut chart = chart();
    let sent = MAX_PENDING_INPUT_EVENTS + 8;
    for i in 0..sent {
        chart.input_context_menu(10.0 + i as f64, 100.0);
    }
    let retained: Vec<f64> = chart
        .take_input_events()
        .into_iter()
        .map(|event| match event {
            ChartInputEvent::ContextMenu(menu) => menu.x,
            other => panic!("unexpected event {other:?}"),
        })
        .collect();
    let newest: Vec<f64> = (8..sent).map(|i| 10.0 + i as f64).collect();
    assert_eq!(retained, newest);
    assert!(chart.take_input_events().is_empty());
}

#[test]
fn a_press_on_the_alert_chip_that_drags_away_requests_no_alert() {
    let mut chart = chart();
    let (x, y) = (chart.pane_w - 4.0, 200.0);
    chart.input_pointer_move(at(x, y), false);
    assert!(chart.alert_create_hit_at(x, y));
    // A plain click on the chip requests one alert, so only the drag below can suppress it.
    click(&mut chart, x, y);
    assert_eq!(chart.take_alert_create_requests().len(), 1);
    chart.input_pointer_move(at(x, y), false);

    chart.input_pointer_down(at(x, y), 1);
    chart.input_pointer_move(at(x, y + 15.0), true);
    chart.input_pointer_move(at(x, y + 30.0), true);
    // The chip follows the crosshair, so it is under the pointer again at the release: only the
    // drag keeps this from being a click.
    assert!(chart.alert_create_hit_at(x, y + 30.0));
    chart.input_pointer_up(at(x, y + 30.0));
    assert!(chart.take_alert_create_requests().is_empty());
}

#[test]
fn leaving_mid_pan_is_ignored_and_the_pan_finishes_like_an_uninterrupted_drag() {
    let mut twin = chart();
    let mut chart = chart();
    let (x, y) = empty_pane_point(&chart);
    drag(&mut twin, (x, y), (x + 80.0, y));

    chart.input_pointer_down(at(x, y), 1);
    chart.input_pointer_move(at(x + 20.0, y), true);
    chart.input_pointer_move(at(x + 40.0, y), true);
    chart.input_pointer_leave();
    assert!(chart.crosshair.is_some(), "a captured drag keeps tracking");
    chart.input_pointer_move(at(x + 60.0, y), true);
    chart.input_pointer_move(at(x + 80.0, y), true);
    chart.input_pointer_up(at(x + 80.0, y));
    assert_eq!(chart.scroll_position(), twin.scroll_position());
}

#[test]
fn leaving_from_a_pane_separator_clears_its_highlight() {
    let mut chart = chart();
    chart.add_pane(true).unwrap();
    relayout(&mut chart);
    let separator = chart.panes[1].top;
    chart.input_pointer_move(at(200.0, separator), false);
    assert_eq!(chart.separator_hover(), Some(0));

    chart.input_pointer_leave();
    assert_eq!(chart.separator_hover(), None);
    assert_eq!(chart.input_cursor(), ChartCursor::Crosshair);
}

/// A finger has no hover: it pans the chart, and the crosshair appears only once a long press
/// starts tracking (the reference's `touchStartEvent` and `touchMoveEvent` against its
/// `longTapEvent`).
#[test]
fn a_finger_shows_the_crosshair_only_once_a_long_press_starts_tracking() {
    let mut chart = chart();
    let (x, y) = empty_pane_point(&chart);
    let touch = |x, timestamp_ms| PointerInput {
        device: InputDevice::Touch,
        timestamp_ms,
        ..at(x, y)
    };

    chart.input_pointer_down(touch(x, 100.0), 1);
    assert_eq!(chart.crosshair, None, "a touch-down shows no crosshair");
    assert!(!chart.input_tick(339.0));
    assert_eq!(chart.crosshair, None);
    assert!(chart.input_tick(340.0));
    assert_eq!(
        chart.crosshair,
        Some((x, y)),
        "the long press starts tracking"
    );
    chart.input_pointer_up(touch(x, 350.0));
    // The next tap ends tracking.
    chart.input_pointer_down(touch(x, 400.0), 1);
    chart.input_pointer_up(touch(x, 410.0));
    assert_eq!(chart.crosshair, None);

    // A finger drag pans and never carries a crosshair along.
    let scroll = chart.scroll_position();
    chart.input_pointer_down(touch(x, 1_000.0), 1);
    for step in 1..=4 {
        let step = f64::from(step);
        chart.input_pointer_move(touch(x + 20.0 * step, 1_000.0 + 10.0 * step), true);
        assert_eq!(chart.crosshair, None, "step {step}");
    }
    chart.input_pointer_up(touch(x + 80.0, 1_050.0));
    assert_ne!(chart.scroll_position(), scroll);
    assert_eq!(chart.crosshair, None);
}

/// A finger dragging a drawing shows the crosshair at the drop point, as a mouse drag does; the
/// touch-down that starts the drag shows none, and the lift clears it.
#[test]
fn a_finger_dragging_a_drawing_shows_the_crosshair_at_the_drop_point() {
    let mut chart = chart();
    let (id, body) = trend_line_body(&mut chart);
    chart.set_selected_drawing(Some(id));
    let touch = |x: f64, timestamp_ms| PointerInput {
        device: InputDevice::Touch,
        timestamp_ms,
        ..at(x, body.1)
    };
    let before = chart.drawing(id).unwrap().points.clone();

    chart.input_pointer_down(touch(body.0, 100.0), 1);
    assert_eq!(chart.crosshair, None, "the touch-down shows no crosshair");
    for step in 1..=4 {
        let x = body.0 + 10.0 * f64::from(step);
        chart.input_pointer_move(touch(x, 100.0 + 10.0 * f64::from(step)), true);
        assert_eq!(chart.crosshair, Some((x, body.1)), "step {step}");
    }
    chart.input_pointer_up(touch(body.0 + 40.0, 150.0));
    assert_ne!(
        chart.drawing(id).unwrap().points,
        before,
        "the drawing moved"
    );
    assert_eq!(chart.crosshair, None, "the lift clears the crosshair");
}

/// A double tap tolerates a finger's wobble: its taps may land up to 30 px apart (the
/// reference's `DoubleTapManhattanDistance`), while a double-click keeps the mouse's 5 px.
#[test]
fn a_double_tap_tolerates_a_finger_wobble_that_a_double_click_does_not() {
    for (device, apart, doubled) in [
        (InputDevice::Touch, 10.0, true),
        (InputDevice::Touch, 30.0, false),
        (InputDevice::Mouse, 4.0, true),
        (InputDevice::Mouse, 10.0, false),
    ] {
        let mut chart = chart();
        let (x, y) = (40..chart.pane_w as i32 - 40)
            .step_by(7)
            .flat_map(|x| (20..chart.pane_h as i32).step_by(13).map(move |y| (x, y)))
            .map(|(x, y)| (f64::from(x), f64::from(y)))
            .find(|&(x, y)| {
                [x, x + apart].iter().all(|&x| {
                    chart.hit_test_series(x, y).is_none()
                        && chart.region_at(x, y) == ChartRegion::Pane
                })
            })
            .expect("the pane has empty space");
        let sample = |x, timestamp_ms| PointerInput {
            device,
            timestamp_ms,
            ..at(x, y)
        };
        chart.input_pointer_down(sample(x, 100.0), 1);
        chart.input_pointer_up(sample(x, 110.0));
        chart.input_pointer_down(sample(x + apart, 200.0), 1);
        chart.input_pointer_up(sample(x + apart, 210.0));
        let second = if doubled {
            ChartInputEvent::DoubleClick { x: x + apart, y }
        } else {
            ChartInputEvent::Click { x: x + apart, y }
        };
        assert_eq!(
            chart.take_input_events(),
            vec![ChartInputEvent::Click { x, y }, second],
            "{device:?} taps {apart} px apart"
        );
    }
}
