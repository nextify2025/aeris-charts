//! Drawing and annotation interactions driven end to end through the input controller: text
//! placement and editing, path sequences, the Shift measure, drag cancellation, anchor edits,
//! magnet placement, Delete routing by selection owner and by the drawing being placed, and the
//! click placement contract of click-placed tools.

use super::tests::*;
use super::*;

fn key(chart: &mut ChartEngine, key: ChartKey) -> bool {
    chart.input_key_down(key, InputModifiers::default(), false, 0.0)
}

fn held(x: f64, y: f64, modifiers: InputModifiers) -> PointerInput {
    PointerInput {
        modifiers,
        ..at(x, y)
    }
}

/// Anchor positions round-trip through the time scale's float index, so they match the pointer
/// to well within a thousandth of a pixel rather than exactly.
fn assert_near(actual: (f64, f64), expected: (f64, f64), what: &str) {
    assert!(
        (actual.0 - expected.0).abs() < 1e-3 && (actual.1 - expected.1).abs() < 1e-3,
        "{what}: {actual:?} != {expected:?}"
    );
}

fn anchor_px(chart: &ChartEngine, id: DrawingId) -> Vec<(f64, f64)> {
    (0..chart.drawing(id).unwrap().points.len())
        .map(|index| chart.drawing_point_to_coordinate(id, index).unwrap())
        .collect()
}

/// Arm a trend line and place it with two clicks that carry the given modifiers.
fn place_trend_line(chart: &mut ChartEngine, clicks: [PointerInput; 2]) -> DrawingId {
    assert!(chart.set_drawing_tool(Some(DrawingKind::TrendLine), None, None));
    for input in clicks {
        chart.input_pointer_down(input, 1);
        chart.input_pointer_up(input);
    }
    let events = chart.take_input_events();
    let [ChartInputEvent::DrawingCreated(id)] = events[..] else {
        panic!("one trend line was created: {events:?}");
    };
    id
}

fn text_drawing(chart: &mut ChartEngine, text: &str) -> DrawingId {
    let options = serde_json::json!({ "text": text }).to_string();
    let id = chart
        .add_drawing(
            DrawingKind::Text,
            0,
            vec![DrawingPoint {
                logical: 30.0,
                price: 104.0,
            }],
            Some(options.as_str()),
        )
        .unwrap();
    chart.build_frame();
    id
}

fn text_center(chart: &ChartEngine, id: DrawingId) -> (f64, f64) {
    let [left, top, right, bottom] = chart.drawing_text_edit_layout(id).unwrap().rect;
    ((left + right) / 2.0, (top + bottom) / 2.0)
}

#[test]
fn the_text_tool_places_on_press_opens_its_editor_and_an_outside_click_commits_one_undo_step() {
    let mut chart = chart();
    assert!(chart.set_drawing_tool(Some(DrawingKind::Text), None, None));
    let (x, y) = (400.0, 250.0);

    chart.input_pointer_down(at(x, y), 1);
    let events = chart.take_input_events();
    let [ChartInputEvent::DrawingCreated(id)] = events[..] else {
        panic!("the press places the text: {events:?}");
    };
    assert_eq!(chart.drawing(id).unwrap().kind, DrawingKind::Text);
    assert_eq!(chart.drawing_text_edit(), Some((id, "", 0)));
    assert_eq!(chart.active_drawing_tool(), None, "a one-shot tool");

    chart.input_pointer_up(at(x, y));
    assert_eq!(chart.drawings().len(), 1);
    assert!(
        chart.take_input_events().is_empty(),
        "the release places nothing more"
    );
    assert_eq!(chart.editing_drawing(), Some(id), "typing continues");

    let placed = chart.drawing_revision();
    for input in ["a", "b", "c"] {
        assert!(chart.drawing_text_edit_insert(input));
    }
    assert_eq!(chart.drawing_revision(), placed, "typing records nothing");

    let (ex, ey) = empty_pane_point(&chart);
    let [left, top, right, bottom] = chart.drawing_text_edit_layout(id).unwrap().rect;
    assert!(ex < left || ex > right || ey < top || ey > bottom);
    assert!(chart.hit_test_drawing(ex, ey).is_none());
    click(&mut chart, ex, ey);
    assert_eq!(chart.editing_drawing(), None);
    assert_eq!(chart.drawing(id).unwrap().text, "abc");
    assert_eq!(chart.drawing_revision(), placed + 1);
    assert_eq!(chart.selected_drawing(), None, "the click also deselects");

    // The whole typing session is one undo step on top of the placement.
    assert!(chart.undo_drawing());
    assert_eq!(chart.drawing(id).unwrap().text, "");
    assert!(chart.undo_drawing());
    assert!(chart.drawing(id).is_none());
}

#[test]
fn a_second_click_on_the_selected_text_opens_its_editor() {
    let mut chart = chart();
    let id = text_drawing(&mut chart, "note");
    let (x, y) = text_center(&chart, id);
    assert_eq!(chart.hit_test_drawing(x, y).map(|hit| hit.id), Some(id));

    click(&mut chart, x, y);
    assert_eq!(chart.selected_drawing(), Some(id));
    assert_eq!(chart.editing_drawing(), None, "the first click selects");
    click(&mut chart, x, y);
    assert_eq!(chart.drawing_text_edit(), Some((id, "note", 4)));
}

#[test]
fn a_click_on_an_unlabeled_trend_lines_add_text_prompt_opens_its_label_editor() {
    let mut chart = chart();
    let (id, _) = trend_line_body(&mut chart);
    assert!(chart.drawing(id).unwrap().text.is_empty());
    let (tx, ty, _) = chart.drawing_text_transform(id).unwrap();
    let prompt = (-60..=20)
        .flat_map(|dx| {
            (-15..=15).map(move |dy| (tx + f64::from(dx) * 2.0, ty + f64::from(dy) * 2.0))
        })
        .find(|&(x, y)| {
            chart.drawing_text_hit_at(x, y) == Some(id) && chart.hit_test_drawing(x, y).is_none()
        })
        .expect("the prompt has a target off the line");

    click(&mut chart, prompt.0, prompt.1);
    assert_eq!(chart.selected_drawing(), Some(id));
    assert_eq!(chart.drawing_text_edit(), Some((id, "", 0)));
    assert!(chart.drawing_text_edit_insert("breakout"));
    let (ex, ey) = empty_pane_point(&chart);
    click(&mut chart, ex, ey);
    assert_eq!(chart.editing_drawing(), None);
    assert_eq!(chart.drawing(id).unwrap().text, "breakout");
}

#[test]
fn path_anchors_placed_by_clicks_pop_on_backspace_and_a_double_click_finishes_one_drawing() {
    let mut chart = chart();
    assert!(chart.set_drawing_tool(Some(DrawingKind::Path), None, None));
    let revision = chart.drawing_revision();
    for (x, y) in [(120.0, 140.0), (220.0, 260.0), (320.0, 160.0)] {
        click(&mut chart, x, y);
    }
    assert!(chart.drawing_create_active());
    assert!(chart.drawings().is_empty(), "a path waits for its finish");
    assert!(chart.take_input_events().is_empty());

    assert!(key(&mut chart, ChartKey::Backspace));
    assert!(chart.drawing_create_active(), "only the latest anchor");

    // A platform double-click delivers its point as a click, then again with click count 2.
    click(&mut chart, 420.0, 220.0);
    double_click(&mut chart, 420.0, 220.0);
    let events = chart.take_input_events();
    let [ChartInputEvent::DrawingCreated(id)] = events[..] else {
        panic!("the double-click finishes one path: {events:?}");
    };
    assert_eq!(chart.drawings().len(), 1);
    assert_eq!(chart.drawing(id).unwrap().kind, DrawingKind::Path);
    let anchors = anchor_px(&chart, id);
    let expected = [(120.0, 140.0), (220.0, 260.0), (420.0, 220.0)];
    assert_eq!(anchors.len(), expected.len(), "{anchors:?}");
    for (index, (actual, expected)) in anchors.into_iter().zip(expected).enumerate() {
        assert_near(actual, expected, &format!("anchor {index}"));
    }
    assert_eq!(chart.active_drawing_tool(), None);
    assert!(!chart.drawing_create_active());
    assert_eq!(chart.selected_drawing(), Some(id));
    assert_eq!(chart.drawing_revision(), revision + 1, "one create step");
}

#[test]
fn escape_discards_a_pending_path_and_disarms_the_tool() {
    let mut chart = chart();
    assert!(chart.set_drawing_tool(Some(DrawingKind::Path), None, None));
    let revision = chart.drawing_revision();
    click(&mut chart, 120.0, 140.0);
    click(&mut chart, 220.0, 260.0);
    assert!(chart.drawing_create_active());

    assert!(key(&mut chart, ChartKey::Escape));
    assert!(!chart.drawing_create_active());
    assert_eq!(chart.active_drawing_tool(), None);
    // The vertex clicks notify nothing; the keyboard cancel clears the hover crosshair.
    assert_eq!(
        chart.take_input_events(),
        vec![ChartInputEvent::CrosshairLeft]
    );

    // Later clicks, a double-click included, are ordinary clicks again.
    click(&mut chart, 320.0, 160.0);
    double_click(&mut chart, 320.0, 160.0);
    assert!(chart.drawings().is_empty());
    assert_eq!(
        chart.take_input_events(),
        vec![
            ChartInputEvent::Click { x: 320.0, y: 160.0 },
            ChartInputEvent::DoubleClick { x: 320.0, y: 160.0 },
        ]
    );
    assert_eq!(chart.drawing_revision(), revision);
}

#[test]
fn shift_drag_on_empty_pane_freezes_a_measure_without_panning_and_a_plain_click_dismisses_it() {
    let mut chart = chart();
    let (x, y) = empty_pane_point(&chart);
    let end = (x + 90.0, y + 60.0);
    let scroll = chart.scroll_position();

    chart.input_pointer_down(shifted(x, y), 1);
    for step in 1..=3 {
        let t = f64::from(step) / 3.0;
        chart.input_pointer_move(shifted(x + 90.0 * t, y + 60.0 * t), true);
        assert_eq!(chart.input_cursor(), ChartCursor::Crosshair);
    }
    chart.input_pointer_up(shifted(end.0, end.1));

    let mut twin = super::tests::chart();
    assert!(twin.measure_pointer_down(x, y, true, DrawingModifiers::default()));
    assert!(twin.measure_pointer_up(end.0, end.1, DrawingModifiers::default()));
    assert!(chart.measure_points().is_some());
    assert_eq!(chart.measure_points(), twin.measure_points());
    assert!(!chart.measure_following(), "the release freezes it");
    assert_eq!(chart.scroll_position(), scroll, "nothing pans");

    // A frozen measure ignores hover.
    chart.input_pointer_move(at(end.0 + 50.0, end.1), false);
    assert_eq!(chart.measure_points(), twin.measure_points());

    // The next plain click only dismisses it: the series under it is not selected.
    let (sx, sy) = series_point(&chart);
    click(&mut chart, sx, sy);
    assert!(!chart.measure_active());
    assert_eq!(chart.selected_series(), None);
    assert_eq!(chart.scroll_position(), scroll);
    click(&mut chart, sx, sy);
    assert_eq!(chart.selected_series(), Some(0), "the next click selects");
}

#[test]
fn a_following_measure_survives_the_pointer_leaving_and_a_press_freezes_it_on_return() {
    let mut chart = chart();
    let (x, y) = empty_pane_point(&chart);
    chart.input_pointer_down(shifted(x, y), 1);
    chart.input_pointer_up(shifted(x, y));
    assert!(chart.measure_following());

    chart.input_pointer_move(at(x + 80.0, y + 40.0), false);
    let live = chart.measure_points();
    chart.input_pointer_leave();
    assert_eq!(chart.crosshair, None);
    assert_eq!(chart.measure_points(), live);
    assert!(chart.measure_following());

    let back = (x + 120.0, y + 70.0);
    chart.input_pointer_move(at(back.0, back.1), false);
    let mut twin = super::tests::chart();
    assert!(twin.measure_pointer_down(x, y, true, DrawingModifiers::default()));
    assert!(twin.measure_pointer_move(back.0, back.1, DrawingModifiers::default()));
    assert_eq!(chart.measure_points(), twin.measure_points());
    assert_ne!(chart.measure_points(), live);

    click(&mut chart, back.0, back.1);
    assert!(chart.measure_active());
    assert!(!chart.measure_following());
    assert_eq!(chart.measure_points(), twin.measure_points());
}

#[test]
fn a_shift_press_on_a_drawing_drags_it_along_the_dominant_axis_instead_of_measuring() {
    let mut chart = chart();
    let (id, body) = trend_line_body(&mut chart);
    let start = anchor_px(&chart, id);
    let revision = chart.drawing_revision();
    let scroll = chart.scroll_position();

    chart.input_pointer_down(shifted(body.0, body.1), 1);
    chart.input_pointer_move(shifted(body.0 + 20.0, body.1 + 5.0), true);
    chart.input_pointer_move(shifted(body.0 + 40.0, body.1 + 10.0), true);
    assert!(!chart.measure_active());
    assert!(chart.drawing_drag_active());
    assert_eq!(chart.input_cursor(), ChartCursor::Grabbing);
    chart.input_pointer_up(shifted(body.0 + 40.0, body.1 + 10.0));

    assert!(!chart.measure_active());
    assert!(!chart.drawing_drag_active());
    // Shift keeps a body move on its dominant axis: horizontal here, so prices are unchanged.
    for (index, (actual, before)) in anchor_px(&chart, id).into_iter().zip(start).enumerate() {
        let expected = (before.0 + 40.0, before.1);
        assert_near(actual, expected, &format!("anchor {index}"));
    }
    assert_eq!(chart.drawing_revision(), revision + 1);
    assert_eq!(chart.scroll_position(), scroll);
}

#[test]
fn escape_during_a_drawing_drag_restores_it_without_an_undo_step_and_ends_the_gesture() {
    let mut chart = chart();
    let (id, body) = trend_line_body(&mut chart);
    let before = chart.drawing(id).unwrap().points.clone();
    let revision = chart.drawing_revision();
    let scroll = chart.scroll_position();

    chart.input_pointer_down(at(body.0, body.1), 1);
    chart.input_pointer_move(at(body.0 + 20.0, body.1 - 10.0), true);
    chart.input_pointer_move(at(body.0 + 40.0, body.1 - 20.0), true);
    assert_ne!(chart.drawing(id).unwrap().points, before);
    assert_eq!(chart.input_cursor(), ChartCursor::Grabbing);

    assert!(key(&mut chart, ChartKey::Escape));
    assert_eq!(chart.drawing(id).unwrap().points, before);
    assert!(!chart.drawing_drag_active());
    assert_eq!(chart.drawing_revision(), revision);
    assert_ne!(chart.input_cursor(), ChartCursor::Grabbing);

    // The button is still held: further motion and the release neither move nor pan.
    chart.input_pointer_move(at(body.0 + 60.0, body.1 - 30.0), true);
    chart.input_pointer_up(at(body.0 + 60.0, body.1 - 30.0));
    assert_eq!(chart.drawing(id).unwrap().points, before);
    assert_eq!(chart.drawing_revision(), revision);
    assert_eq!(chart.scroll_position(), scroll);
    assert_eq!(chart.selected_drawing(), None);
}

#[test]
fn dragging_a_selected_drawings_anchor_moves_only_that_anchor_as_one_undo_step() {
    let mut chart = chart();
    let (id, body) = trend_line_body(&mut chart);
    click(&mut chart, body.0, body.1);
    assert_eq!(chart.selected_drawing(), Some(id));
    let before = chart.drawing(id).unwrap().points.clone();
    let a = chart.drawing_point_to_coordinate(id, 0).unwrap();
    assert_eq!(
        chart.hit_test_drawing(a.0, a.1).map(|hit| hit.part),
        Some(DrawingDragPart::Anchor(0))
    );
    let revision = chart.drawing_revision();

    let target = (a.0 + 30.0, a.1 - 25.0);
    drag(&mut chart, a, target);
    assert_near(
        chart.drawing_point_to_coordinate(id, 0).unwrap(),
        target,
        "dragged anchor",
    );
    assert_eq!(chart.drawing(id).unwrap().points[1], before[1]);
    assert_eq!(chart.drawing_revision(), revision + 1);
    assert_eq!(chart.selected_drawing(), Some(id));

    assert!(chart.undo_drawing());
    assert_eq!(chart.drawing(id).unwrap().points, before);
}

/// Assert every anchor of `id` sits `delta` px from where it was in `start`.
fn assert_shifted(
    chart: &ChartEngine,
    id: DrawingId,
    start: &[(f64, f64)],
    delta: (f64, f64),
    what: &str,
) {
    let anchors = anchor_px(chart, id);
    assert_eq!(anchors.len(), start.len(), "{what}");
    for (index, (actual, before)) in anchors.into_iter().zip(start).enumerate() {
        let expected = (before.0 + delta.0, before.1 + delta.1);
        assert_near(actual, expected, &format!("{what}: anchor {index}"));
    }
}

/// A press that never travels the shared 5 px slop is a click on every device: it selects the
/// drawing, moves nothing, and records neither an undo step nor a revision.
#[test]
fn a_wobbling_click_on_a_drawing_selects_it_without_moving_it_or_recording_an_undo_step() {
    for device in [InputDevice::Mouse, InputDevice::Pen, InputDevice::Touch] {
        let mut chart = chart();
        let (id, body) = trend_line_body(&mut chart);
        let before = chart.drawing(id).unwrap().points.clone();
        let revision = chart.drawing_revision();
        let sample = |dx: f64, dy: f64| PointerInput {
            device,
            ..at(body.0 + dx, body.1 + dy)
        };

        chart.input_pointer_down(sample(0.0, 0.0), 1);
        // 2, 3, and 4 px (Manhattan) from the press.
        for (dx, dy) in [(1.0, -1.0), (2.0, 1.0), (1.0, 3.0)] {
            chart.input_pointer_move(sample(dx, dy), true);
            assert_eq!(
                chart.drawing(id).unwrap().points,
                before,
                "{device:?} {dx},{dy}"
            );
        }
        chart.input_pointer_up(sample(1.0, 3.0));
        assert_eq!(chart.drawing(id).unwrap().points, before, "{device:?}");
        assert_eq!(chart.drawing_revision(), revision, "{device:?}");
        assert_eq!(chart.selected_drawing(), Some(id), "{device:?}");
        let click = ChartInputEvent::Click {
            x: body.0 + 1.0,
            y: body.1 + 3.0,
        };
        assert!(
            chart.take_input_events().contains(&click),
            "{device:?}: the press is a click"
        );
        assert!(chart.undo_drawing());
        assert!(
            chart.drawing(id).is_none(),
            "{device:?}: the only undo step is the placement"
        );
    }
}

/// Past the slop a drawing catches up with the pointer at once and then follows it exactly, back
/// inside the slop too, for a body and an anchor alike; each drag is one undo step.
#[test]
fn a_drawing_drag_past_the_slop_follows_the_pointer_back_near_its_start_as_one_undo_step() {
    let mut chart = chart();
    let (id, body) = trend_line_body(&mut chart);
    let start = anchor_px(&chart, id);
    let revision = chart.drawing_revision();

    chart.input_pointer_down(at(body.0, body.1), 1);
    chart.input_pointer_move(at(body.0 + 2.0, body.1 - 2.0), true);
    assert_shifted(&chart, id, &start, (0.0, 0.0), "body inside the slop");
    chart.input_pointer_move(at(body.0 + 12.0, body.1 - 6.0), true);
    assert_shifted(&chart, id, &start, (12.0, -6.0), "body crossing the slop");
    chart.input_pointer_move(at(body.0 + 2.0, body.1 - 1.0), true);
    chart.input_pointer_up(at(body.0 + 2.0, body.1 - 1.0));
    assert_shifted(&chart, id, &start, (2.0, -1.0), "body released");
    assert_eq!(chart.drawing_revision(), revision + 1);

    // The grab selected the drawing, so its first anchor is now a handle.
    let a = chart.drawing_point_to_coordinate(id, 0).unwrap();
    let b = chart.drawing_point_to_coordinate(id, 1).unwrap();
    assert_eq!(
        chart.hit_test_drawing(a.0, a.1).map(|hit| hit.part),
        Some(DrawingDragPart::Anchor(0))
    );
    let anchor = |chart: &ChartEngine| chart.drawing_point_to_coordinate(id, 0).unwrap();
    chart.input_pointer_down(at(a.0, a.1), 1);
    chart.input_pointer_move(at(a.0 - 3.0, a.1 + 1.0), true);
    assert_near(anchor(&chart), a, "anchor inside the slop");
    chart.input_pointer_move(at(a.0 - 15.0, a.1 + 5.0), true);
    assert_near(
        anchor(&chart),
        (a.0 - 15.0, a.1 + 5.0),
        "anchor crossing the slop",
    );
    chart.input_pointer_move(at(a.0 - 2.0, a.1 + 1.0), true);
    chart.input_pointer_up(at(a.0 - 2.0, a.1 + 1.0));
    assert_near(anchor(&chart), (a.0 - 2.0, a.1 + 1.0), "anchor released");
    assert_near(
        chart.drawing_point_to_coordinate(id, 1).unwrap(),
        b,
        "the other anchor stays",
    );
    assert_eq!(chart.drawing_revision(), revision + 2);

    // One undo step per drag, on top of the placement.
    assert!(chart.undo_drawing());
    assert_near(anchor(&chart), a, "the anchor drag undone");
    assert!(chart.undo_drawing());
    assert_shifted(&chart, id, &start, (0.0, 0.0), "the body drag undone");
    assert!(chart.undo_drawing());
    assert!(chart.drawing(id).is_none());
}

/// A release past the slop is a drag even when no motion sample crossed it first: the drawing
/// lands where the pointer lets go, and the release is no click.
#[test]
fn a_drawing_released_past_the_slop_without_crossing_motion_lands_at_the_release() {
    let mut chart = chart();
    let (id, body) = trend_line_body(&mut chart);
    let start = anchor_px(&chart, id);
    let revision = chart.drawing_revision();

    chart.input_pointer_down(at(body.0, body.1), 1);
    chart.input_pointer_move(at(body.0 + 1.0, body.1), true);
    chart.input_pointer_up(at(body.0 + 30.0, body.1 - 20.0));
    assert_shifted(&chart, id, &start, (30.0, -20.0), "released");
    assert_eq!(chart.drawing_revision(), revision + 1);
    assert!(!chart
        .take_input_events()
        .iter()
        .any(|event| matches!(event, ChartInputEvent::Click { .. })));
}

/// A wobbling click never lets the strong magnet pull a drawing onto a bar's price; the magnet
/// applies once the drag has started.
#[test]
fn a_wobbling_click_never_magnet_snaps_a_drawing() {
    let mut chart = chart();
    chart.set_drawing_magnet_mode(DrawingMagnetMode::Strong);
    // Bar 30 is O 102 H 105 L 99 C 103; the line sits between its close and high.
    let id = chart
        .add_drawing(
            DrawingKind::HorizontalLine,
            0,
            vec![DrawingPoint {
                logical: 30.0,
                price: 104.4,
            }],
            None,
        )
        .unwrap();
    chart.build_frame();
    let x = chart.time_scale.index_to_coordinate(30);
    let y = chart.drawing_point_to_coordinate(id, 0).unwrap().1;
    assert_eq!(chart.hit_test_drawing(x, y).map(|hit| hit.id), Some(id));
    let before = chart.drawing(id).unwrap().points.clone();
    let revision = chart.drawing_revision();

    chart.input_pointer_down(at(x, y), 1);
    chart.input_pointer_move(at(x + 1.0, y + 1.0), true);
    chart.input_pointer_move(at(x, y + 2.0), true);
    chart.input_pointer_up(at(x, y + 2.0));
    assert_eq!(chart.drawing(id).unwrap().points, before);
    assert_eq!(chart.drawing_revision(), revision);

    // About 20 px down the pointer is nearest the bar's close.
    drag(&mut chart, (x, y), (x, y + 20.0));
    let price = chart.drawing(id).unwrap().points[0].price;
    assert!((price - 103.0).abs() < 1e-6, "{price}");
    assert_eq!(chart.drawing_revision(), revision + 1);
}

/// Once a drag has moved, the drawing commits as last shown. Letting go of the magnet key
/// (Ctrl/Cmd) or the straighten key (Shift) before the button changes nothing, even when no
/// motion follows the key release: a line pulled onto a bar's close stays there, and a
/// Shift-straightened body move stays horizontal.
#[test]
fn a_modifier_let_go_before_the_button_keeps_the_drag_as_shown() {
    let control = InputModifiers {
        control: true,
        ..InputModifiers::default()
    };
    let mut chart = chart();
    // Bar 30 is O 102 H 105 L 99 C 103; the line sits between its close and high.
    let id = chart
        .add_drawing(
            DrawingKind::HorizontalLine,
            0,
            vec![DrawingPoint {
                logical: 30.0,
                price: 104.4,
            }],
            None,
        )
        .unwrap();
    chart.build_frame();
    let x = chart.time_scale.index_to_coordinate(30);
    let y = chart.drawing_point_to_coordinate(id, 0).unwrap().1;
    chart.input_pointer_down(held(x, y, control), 1);
    chart.input_pointer_move(held(x, y + 10.0, control), true);
    chart.input_pointer_move(held(x, y + 20.0, control), true);
    let shown = chart.drawing(id).unwrap().points[0].price;
    assert!(
        (shown - 103.0).abs() < 1e-6,
        "the magnet pulls it to the close: {shown}"
    );
    chart.input_modifiers_changed(InputModifiers::default());
    chart.input_pointer_up(at(x, y + 20.0));
    assert_eq!(chart.drawing(id).unwrap().points[0].price, shown);

    let mut chart = super::tests::chart();
    let (id, body) = trend_line_body(&mut chart);
    let start = anchor_px(&chart, id);
    chart.input_pointer_down(shifted(body.0, body.1), 1);
    chart.input_pointer_move(shifted(body.0 + 20.0, body.1 + 5.0), true);
    chart.input_pointer_move(shifted(body.0 + 40.0, body.1 + 10.0), true);
    assert_shifted(&chart, id, &start, (40.0, 0.0), "straightened while held");
    chart.input_modifiers_changed(InputModifiers::default());
    chart.input_pointer_up(at(body.0 + 40.0, body.1 + 10.0));
    assert_shifted(&chart, id, &start, (40.0, 0.0), "committed as shown");
}

/// A wobbling click on the selected text opens its editor and leaves the text where it was.
#[test]
fn a_wobbling_click_on_the_selected_text_opens_its_editor_without_moving_it() {
    let mut chart = chart();
    let id = text_drawing(&mut chart, "note");
    let (x, y) = text_center(&chart, id);
    click(&mut chart, x, y);
    assert_eq!(chart.selected_drawing(), Some(id));
    let before = chart.drawing(id).unwrap().points.clone();
    let revision = chart.drawing_revision();

    chart.input_pointer_down(at(x, y), 1);
    chart.input_pointer_move(at(x + 2.0, y - 1.0), true);
    chart.input_pointer_up(at(x + 2.0, y - 1.0));
    assert_eq!(chart.editing_drawing(), Some(id));
    assert_eq!(chart.drawing(id).unwrap().points, before);
    assert_eq!(chart.drawing_revision(), revision);
}

/// The second press of a double-click may wobble too: it opens the editor and moves nothing.
#[test]
fn a_wobbling_double_click_on_a_selected_drawing_opens_its_editor_without_moving_it() {
    let mut chart = chart();
    let (id, body) = trend_line_body(&mut chart);
    click(&mut chart, body.0, body.1);
    assert_eq!(chart.selected_drawing(), Some(id));
    let before = chart.drawing(id).unwrap().points.clone();
    let revision = chart.drawing_revision();

    chart.input_pointer_down(at(body.0, body.1), 2);
    chart.input_pointer_move(at(body.0 + 2.0, body.1 + 1.0), true);
    chart.input_pointer_up(at(body.0 + 2.0, body.1 + 1.0));
    assert_eq!(chart.editing_drawing(), Some(id));
    assert_eq!(chart.drawing(id).unwrap().points, before);
    assert_eq!(chart.drawing_revision(), revision);
    assert!(chart.cancel_drawing_text_edit());
    assert!(chart.undo_drawing());
    assert!(
        chart.drawing(id).is_none(),
        "the only undo step is the placement"
    );
}

#[test]
fn ctrl_or_cmd_held_while_placing_snaps_each_anchor_to_the_bars_ohlc() {
    let mut chart = chart();
    // Bar 20 is O 106 H 109 L 103 C 107; bar 40 is O 105 H 108 L 102 C 106. Each click lands
    // just off its bar's center, nearest that bar's high or low.
    let high = (
        chart.time_scale.index_to_coordinate(20) + 1.0,
        chart.series_price_to_coordinate(0, 108.6).unwrap(),
    );
    let low = (
        chart.time_scale.index_to_coordinate(40) - 1.0,
        chart.series_price_to_coordinate(0, 102.4).unwrap(),
    );
    let none = InputModifiers::default();

    let free = place_trend_line(
        &mut chart,
        [held(high.0, high.1, none), held(low.0, low.1, none)],
    );
    let free = chart.drawing(free).unwrap().points.clone();
    assert!((free[0].price - 108.6).abs() < 1e-6, "{free:?}");
    assert!((free[1].price - 102.4).abs() < 1e-6, "{free:?}");
    assert!((free[0].logical - 20.0).abs() > 1e-3, "{free:?}");

    let control = InputModifiers {
        control: true,
        ..InputModifiers::default()
    };
    let meta = InputModifiers {
        meta: true,
        ..InputModifiers::default()
    };
    let snapped = place_trend_line(
        &mut chart,
        [held(high.0, high.1, control), held(low.0, low.1, meta)],
    );
    let snapped = chart.drawing(snapped).unwrap().points.clone();
    assert_eq!(snapped[0].logical, 20.0, "{snapped:?}");
    assert!((snapped[0].price - 109.0).abs() < 1e-6, "{snapped:?}");
    assert_eq!(snapped[1].logical, 40.0, "{snapped:?}");
    assert!((snapped[1].price - 102.0).abs() < 1e-6, "{snapped:?}");
}

#[test]
fn backspace_deletes_the_clicked_drawing_as_one_undoable_step() {
    let mut chart = chart();
    let (id, body) = trend_line_body(&mut chart);
    click(&mut chart, body.0, body.1);
    assert_eq!(chart.selected_drawing(), Some(id));
    // The selecting click notifies pane click subscribers; only the key's requests matter below.
    chart.take_input_events();
    let revision = chart.drawing_revision();

    assert!(key(&mut chart, ChartKey::Backspace));
    assert!(chart.drawing(id).is_none());
    assert_eq!(chart.drawing_revision(), revision + 1);
    assert!(chart.take_input_events().is_empty(), "no host request");
    assert!(chart.undo_drawing());
    assert!(chart.drawing(id).is_some());
}

#[test]
fn delete_on_a_clicked_indicator_line_removes_its_binding_without_a_host_request() {
    let mut chart = chart();
    let sma = chart.add_sma(0, 5).expect("SMA output");
    chart.autoscale_visible();
    chart.build_frame();
    let on_line = (5..BARS as i64)
        .map(|index| chart.time_scale.index_to_coordinate(index))
        .flat_map(|x| (0..chart.pane_h as i32).map(move |y| (x, f64::from(y))))
        .find(|&(x, y)| chart.hit_test_series(x, y) == Some(sma))
        .expect("the SMA line has a click target");

    click(&mut chart, on_line.0, on_line.1);
    assert_eq!(chart.selected_series(), Some(sma));
    chart.take_input_events();
    assert!(key(&mut chart, ChartKey::Delete));
    assert!(!chart.has_indicator_bindings());
    assert!(chart.series_entry(sma).is_none());
    assert!(chart.series_entry(0).is_some(), "the source series stays");
    assert_eq!(chart.selected_series(), None);
    assert!(
        chart.take_input_events().is_empty(),
        "the engine removes its own indicators"
    );
}

#[test]
fn delete_on_a_clicked_host_series_asks_the_host_to_remove_it() {
    let mut chart = chart();
    let (x, y) = series_point(&chart);
    click(&mut chart, x, y);
    assert_eq!(chart.selected_series(), Some(0));
    chart.take_input_events();

    assert!(key(&mut chart, ChartKey::Delete));
    assert_eq!(
        chart.take_input_events(),
        vec![ChartInputEvent::RemoveSeries(0)]
    );
    assert!(chart.series_entry(0).is_some(), "the host decides");
}

#[test]
fn delete_and_backspace_with_nothing_selected_stay_unconsumed() {
    let mut chart = chart();
    let (id, _) = trend_line_body(&mut chart);
    let revision = chart.drawing_revision();
    assert_eq!(chart.selected_drawing(), None);
    assert_eq!(chart.selected_series(), None);

    assert!(!key(&mut chart, ChartKey::Delete));
    assert!(!key(&mut chart, ChartKey::Backspace));
    assert!(chart.drawing(id).is_some());
    assert_eq!(chart.drawing_revision(), revision);
    assert!(chart.take_input_events().is_empty());
}

/// The points placed so far by the drawing being placed, or `None` with no placement under way.
fn placed_points(chart: &ChartEngine) -> Option<usize> {
    chart
        .pending_drawing()
        .map(|pending| pending.drawing.points.len())
}

/// Select the fixture trend line with a click, as a trader would before picking a tool.
fn selected_trend_line(chart: &mut ChartEngine) -> DrawingId {
    let (id, body) = trend_line_body(chart);
    click(chart, body.0, body.1);
    assert_eq!(chart.selected_drawing(), Some(id));
    // The selecting click notifies pane click subscribers; only the keys' requests matter below.
    chart.take_input_events();
    id
}

/// Once a path's first point is placed, Backspace and Delete alike step back that path one point
/// at a time and keep the live preview; past its last point a held key does nothing, and the
/// drawing selected before the tool was picked survives every press.
#[test]
fn once_a_path_is_under_way_backspace_and_delete_step_back_only_that_path() {
    let mut chart = chart();
    let old = selected_trend_line(&mut chart);
    assert!(chart.set_drawing_tool(Some(DrawingKind::Path), None, None));
    let revision = chart.drawing_revision();
    for (x, y) in [(120.0, 140.0), (220.0, 260.0)] {
        click(&mut chart, x, y);
    }
    chart.input_pointer_move(at(300.0, 200.0), false);
    let preview = chart.pending_drawing().unwrap().preview;
    assert!(preview.is_some());

    assert!(key(&mut chart, ChartKey::Delete));
    assert_eq!(placed_points(&chart), Some(1), "Delete steps back the path");
    assert_eq!(
        chart.pending_drawing().unwrap().preview,
        preview,
        "the live preview stays"
    );
    assert!(key(&mut chart, ChartKey::Backspace));
    assert_eq!(placed_points(&chart), Some(0));
    // A held key past the last point does nothing until Escape or the next click.
    for held in [ChartKey::Backspace, ChartKey::Delete] {
        for _ in 0..3 {
            assert!(chart.input_key_down(held, InputModifiers::default(), true, 0.0));
        }
    }
    assert_eq!(placed_points(&chart), Some(0));
    assert!(
        chart.drawing(old).is_some(),
        "the selected drawing survives"
    );
    assert_eq!(chart.selected_drawing(), Some(old));
    assert_eq!(chart.drawing_revision(), revision);
    assert_eq!(chart.active_drawing_tool(), Some(DrawingKind::Path));
    assert!(chart.take_input_events().is_empty());

    // The next clicks place the path afresh.
    click(&mut chart, 140.0, 300.0);
    click(&mut chart, 360.0, 180.0);
    double_click(&mut chart, 360.0, 180.0);
    let events = chart.take_input_events();
    let [ChartInputEvent::DrawingCreated(id)] = events[..] else {
        panic!("the double-click finishes one path: {events:?}");
    };
    let anchors = anchor_px(&chart, id);
    assert_eq!(anchors.len(), 2, "{anchors:?}");
    assert_near(anchors[0], (140.0, 300.0), "first anchor");
    assert_near(anchors[1], (360.0, 180.0), "second anchor");
    assert!(chart.drawing(old).is_some());
    assert_eq!(chart.drawing_revision(), revision + 1, "one create step");
}

/// Fixed-count tools step back too: after the first click of a trend line or the third of an
/// XABCD pattern, Backspace removes the latest point and keeps the preview, the next clicks
/// finish the drawing exactly as if the removed point had never been placed, and the drawing
/// selected before the tool was picked survives. Once the new drawing is committed and selected,
/// Backspace deletes it as usual.
#[test]
fn backspace_steps_back_a_fixed_count_tool_and_never_deletes_the_selected_drawing() {
    let clicks = [
        (100.0, 300.0),
        (180.0, 120.0),
        (260.0, 320.0),
        (340.0, 140.0),
        (420.0, 300.0),
        (500.0, 150.0),
    ];
    for (kind, placed) in [(DrawingKind::TrendLine, 1), (DrawingKind::XabcdPattern, 3)] {
        let count = kind.anchor_count();
        // The clicks that finish the drawing after the latest placed point is stepped back.
        let kept: Vec<(f64, f64)> = clicks[..placed - 1]
            .iter()
            .chain(&clicks[placed..placed + count - (placed - 1)])
            .copied()
            .collect();
        // The same chart, scales included, placing those clicks with no step back.
        let mut reference = chart();
        selected_trend_line(&mut reference);
        assert!(reference.set_drawing_tool(Some(kind), None, None));
        for &(x, y) in &kept {
            click(&mut reference, x, y);
        }
        let [ChartInputEvent::DrawingCreated(reference_id)] = reference.take_input_events()[..]
        else {
            panic!("{kind:?}: the reference placement commits");
        };
        let expected = reference.drawing(reference_id).unwrap().points.clone();

        let mut chart = chart();
        let old = selected_trend_line(&mut chart);
        let revision = chart.drawing_revision();
        assert!(chart.set_drawing_tool(Some(kind), None, None));
        for &(x, y) in &clicks[..placed] {
            click(&mut chart, x, y);
        }
        chart.input_pointer_move(at(450.0, 400.0), false);
        let preview = chart.pending_drawing().unwrap().preview;

        assert!(key(&mut chart, ChartKey::Backspace), "{kind:?}");
        assert_eq!(placed_points(&chart), Some(placed - 1), "{kind:?}");
        assert_eq!(
            chart.pending_drawing().unwrap().preview,
            preview,
            "{kind:?}"
        );
        assert!(chart.drawing(old).is_some(), "{kind:?}");
        assert_eq!(chart.selected_drawing(), Some(old), "{kind:?}");
        assert_eq!(chart.drawing_revision(), revision, "{kind:?}");

        for &(x, y) in &clicks[placed..placed + count - (placed - 1)] {
            click(&mut chart, x, y);
        }
        let events = chart.take_input_events();
        let [ChartInputEvent::DrawingCreated(id)] = events[..] else {
            panic!("{kind:?}: the last click commits one drawing: {events:?}");
        };
        assert_eq!(chart.drawing(id).unwrap().points, expected, "{kind:?}");
        assert_eq!(chart.drawing_revision(), revision + 1, "{kind:?}");

        assert_eq!(chart.selected_drawing(), Some(id), "{kind:?}");
        assert!(key(&mut chart, ChartKey::Backspace), "{kind:?}");
        assert!(chart.drawing(id).is_none(), "{kind:?}: the new drawing");
        assert!(chart.drawing(old).is_some(), "{kind:?}");
    }
}

/// A trend line stepped back to no placed points draws nothing at the pointer yet, so the
/// crosshair shows to aim the next click, as before the first one. With a point placed, the
/// placement preview owns the pointer again.
#[test]
fn a_placement_stepped_back_to_no_points_aims_with_the_crosshair() {
    let mut chart = chart();
    assert!(chart.set_drawing_tool(Some(DrawingKind::TrendLine), None, None));
    chart.input_pointer_move(at(200.0, 200.0), false);
    assert!(
        !chart.crosshair_suppressed_by_interaction(),
        "before the first click"
    );
    click(&mut chart, 120.0, 140.0);
    chart.input_pointer_move(at(220.0, 260.0), false);
    assert!(
        chart.crosshair_suppressed_by_interaction(),
        "the preview follows the pointer"
    );

    assert!(key(&mut chart, ChartKey::Backspace));
    assert_eq!(placed_points(&chart), Some(0));
    assert!(
        !chart.crosshair_suppressed_by_interaction(),
        "stepped back to no points"
    );
    chart.input_pointer_move(at(240.0, 280.0), false);
    assert!(!chart.crosshair_suppressed_by_interaction());

    click(&mut chart, 140.0, 300.0);
    assert_eq!(placed_points(&chart), Some(1));
    assert!(chart.crosshair_suppressed_by_interaction());
}

/// A selected indicator line or host series is never removed, or its removal requested, while a
/// drawing is being placed.
#[test]
fn keys_mid_placement_never_remove_a_selected_indicator_or_host_series() {
    let mut chart = chart();
    let sma = chart.add_sma(0, 5).expect("SMA output");
    chart.autoscale_visible();
    chart.build_frame();
    let on_line = (5..BARS as i64)
        .map(|index| chart.time_scale.index_to_coordinate(index))
        .flat_map(|x| (0..chart.pane_h as i32).map(move |y| (x, f64::from(y))))
        .find(|&(x, y)| chart.hit_test_series(x, y) == Some(sma))
        .expect("the SMA line has a click target");
    click(&mut chart, on_line.0, on_line.1);
    assert_eq!(chart.selected_series(), Some(sma));
    chart.take_input_events();

    assert!(chart.set_drawing_tool(Some(DrawingKind::TrendLine), None, None));
    let (x, y) = empty_pane_point(&chart);
    click(&mut chart, x, y);
    assert!(key(&mut chart, ChartKey::Delete));
    assert!(key(&mut chart, ChartKey::Backspace));
    assert!(chart.has_indicator_bindings());
    assert!(chart.series_entry(sma).is_some());
    assert_eq!(chart.selected_series(), Some(sma));
    assert!(chart.take_input_events().is_empty());

    assert!(key(&mut chart, ChartKey::Escape));
    let (sx, sy) = series_point(&chart);
    click(&mut chart, sx, sy);
    assert_eq!(chart.selected_series(), Some(0));
    chart.take_input_events();
    assert!(chart.set_drawing_tool(Some(DrawingKind::Path), None, None));
    click(&mut chart, x, y);
    for pressed in [ChartKey::Delete, ChartKey::Backspace, ChartKey::Delete] {
        assert!(key(&mut chart, pressed));
    }
    assert!(
        chart.take_input_events().is_empty(),
        "no series removal request"
    );
    assert_eq!(chart.selected_series(), Some(0));
}

/// A freehand stroke being drawn owns the keys as well: nothing else is deleted mid-stroke.
#[test]
fn keys_during_a_freehand_stroke_never_delete_the_selected_drawing() {
    let mut chart = chart();
    let old = selected_trend_line(&mut chart);
    assert!(chart.set_drawing_tool(Some(DrawingKind::Brush), None, None));
    chart.input_pointer_down(at(100.0, 100.0), 1);
    chart.input_pointer_move(at(140.0, 120.0), true);
    assert!(chart.flush_coalesced_input());
    assert!(chart.drawing_tool_capture_active());

    assert!(key(&mut chart, ChartKey::Delete));
    assert!(key(&mut chart, ChartKey::Backspace));
    assert!(chart.drawing(old).is_some());
    assert!(chart.drawing_tool_capture_active(), "the stroke continues");

    chart.input_pointer_up(at(180.0, 140.0));
    let events = chart.take_input_events();
    let [ChartInputEvent::DrawingCreated(id)] = events[..] else {
        panic!("the stroke commits: {events:?}");
    };
    assert_eq!(chart.drawing(id).unwrap().kind, DrawingKind::Brush);
    assert!(chart.drawing(old).is_some());
}

/// Before the first click a picked tool leaves the keys to the selection a trader can see: the
/// selected drawing is deleted as one undoable step, and the tool stays armed.
#[test]
fn with_a_tool_armed_and_nothing_placed_the_keys_act_on_the_visible_selection() {
    let mut chart = chart();
    let old = selected_trend_line(&mut chart);
    assert!(chart.set_drawing_tool(Some(DrawingKind::Path), None, None));
    let revision = chart.drawing_revision();

    assert!(key(&mut chart, ChartKey::Backspace));
    assert!(chart.drawing(old).is_none());
    assert_eq!(chart.drawing_revision(), revision + 1);
    assert_eq!(chart.active_drawing_tool(), Some(DrawingKind::Path));
    assert!(
        !key(&mut chart, ChartKey::Delete),
        "with nothing selected the key stays unconsumed"
    );
    assert!(chart.undo_drawing());
    assert!(chart.drawing(old).is_some());
}

// Click placement is the decided creation contract for every click-placed tool (fixed-count,
// multi-click, and single-click preset): anchors come only from clicks, and a press that travels
// the shared 5 px slop places nothing. Freehand tools capture a press-drag and Text places on press
// (covered above).

/// A press-drag-release with a click-placed tool armed places nothing on any device: no anchor,
/// no preview, no drawing or undo step, no pan, and no click notification. The tool stays armed,
/// and a click then places as usual.
#[test]
fn a_press_drag_release_with_a_click_placed_tool_armed_places_nothing() {
    let (from, to) = ((150.0, 150.0), (450.0, 300.0));
    // Whether one click commits the drawing (one anchor, or a single-click preset).
    let kinds = [
        (DrawingKind::HorizontalLine, true),
        (DrawingKind::TrendLine, false),
        (DrawingKind::ParallelChannel, false),
        (DrawingKind::Path, false),
        (DrawingKind::LongPosition, true),
    ];
    for device in [InputDevice::Mouse, InputDevice::Pen, InputDevice::Touch] {
        for (kind, one_click) in kinds {
            let what = format!("{device:?} {kind:?}");
            let mut chart = chart();
            assert!(chart.set_drawing_tool(Some(kind), None, None), "{what}");
            let scroll = chart.scroll_position();
            let price = chart
                .price_scale_visible_range_for(0, PriceScaleTarget::Right)
                .unwrap();
            let revision = chart.drawing_revision();
            let sample = |x: f64, y: f64| PointerInput { device, ..at(x, y) };

            chart.input_pointer_down(sample(from.0, from.1), 1);
            for step in 1..=6 {
                let t = f64::from(step) / 6.0;
                let (x, y) = (from.0 + (to.0 - from.0) * t, from.1 + (to.1 - from.1) * t);
                chart.input_pointer_move(sample(x, y), true);
                assert_eq!(placed_points(&chart), None, "{what}: step {step}");
            }
            chart.input_pointer_up(sample(to.0, to.1));

            assert!(chart.drawings().is_empty(), "{what}: no drawing");
            assert_eq!(placed_points(&chart), None, "{what}: no anchor or preview");
            assert!(
                chart.take_input_events().is_empty(),
                "{what}: no creation or click"
            );
            assert_eq!(chart.drawing_revision(), revision, "{what}");
            assert_eq!(chart.scroll_position(), scroll, "{what}: no pan");
            assert_eq!(
                chart.price_scale_visible_range_for(0, PriceScaleTarget::Right),
                Some(price),
                "{what}: no price pan"
            );
            assert_eq!(
                chart.active_drawing_tool(),
                Some(kind),
                "{what}: still armed"
            );

            chart.input_pointer_down(sample(from.0, from.1), 1);
            chart.input_pointer_up(sample(from.0, from.1));
            if one_click {
                let events = chart.take_input_events();
                assert!(
                    matches!(events[..], [ChartInputEvent::DrawingCreated(_)]),
                    "{what}: the click places it: {events:?}"
                );
            } else {
                assert_eq!(placed_points(&chart), Some(1), "{what}: the click places");
            }
        }
    }
}

/// The shared slop decides between a click and a drag at the release as well: a release under
/// 5 px (Manhattan) from the press is a wobbly click that places its anchor where the button comes
/// up, while a release past it is a drag that places nothing, even with no motion in between.
#[test]
fn the_click_slop_alone_decides_whether_a_click_placed_tool_places_an_anchor() {
    let (x, y) = (200.0, 200.0);

    let mut wobbly = chart();
    assert!(wobbly.set_drawing_tool(Some(DrawingKind::TrendLine), None, None));
    wobbly.input_pointer_down(at(x, y), 1);
    wobbly.input_pointer_move(at(x + 2.0, y - 1.0), true);
    wobbly.input_pointer_up(at(x + 3.0, y + 1.0));
    assert_eq!(placed_points(&wobbly), Some(1), "a 4 px click places");
    click(&mut wobbly, 420.0, 320.0);
    let [ChartInputEvent::DrawingCreated(id)] = wobbly.take_input_events()[..] else {
        panic!("the second click commits the line");
    };
    assert_near(
        anchor_px(&wobbly, id)[0],
        (x + 3.0, y + 1.0),
        "at the release",
    );

    let mut dragged = chart();
    assert!(dragged.set_drawing_tool(Some(DrawingKind::TrendLine), None, None));
    dragged.input_pointer_down(at(x, y), 1);
    dragged.input_pointer_up(at(x + 4.0, y + 2.0));
    assert_eq!(placed_points(&dragged), None, "a 6 px release is a drag");
    assert!(dragged.take_input_events().is_empty());
    assert_eq!(dragged.active_drawing_tool(), Some(DrawingKind::TrendLine));
}

/// Once the first anchor is placed, a press-drag-release only moves the live preview, exactly as
/// hovering there would: the release places no anchor, and a cancelled drag leaves the placement
/// as it was. The next click then commits the drawing from the first anchor to that click as one
/// undo step.
#[test]
fn after_the_first_click_a_press_drag_release_only_moves_the_preview() {
    let first = (150.0, 320.0);
    let (from, to) = ((300.0, 300.0), (420.0, 160.0));

    // The preview a plain hover over `to` shows after the first click.
    let mut reference = chart();
    assert!(reference.set_drawing_tool(Some(DrawingKind::TrendLine), None, None));
    click(&mut reference, first.0, first.1);
    reference.input_pointer_move(at(to.0, to.1), false);
    let hovered = reference.pending_drawing().unwrap().preview;
    assert!(hovered.is_some());

    let mut chart = chart();
    assert!(chart.set_drawing_tool(Some(DrawingKind::TrendLine), None, None));
    let revision = chart.drawing_revision();
    click(&mut chart, first.0, first.1);
    drag(&mut chart, from, to);
    assert_eq!(placed_points(&chart), Some(1), "the release places nothing");
    assert_eq!(
        chart.pending_drawing().unwrap().preview,
        hovered,
        "the preview followed the drag"
    );
    assert!(chart.drawings().is_empty());
    assert!(chart.take_input_events().is_empty());

    // A drag the host cancels part-way leaves the placement as it was.
    chart.input_pointer_down(at(from.0, from.1), 1);
    chart.input_pointer_move(at(from.0 + 40.0, from.1 - 30.0), true);
    chart.input_cancel();
    assert_eq!(placed_points(&chart), Some(1));
    assert_eq!(chart.active_drawing_tool(), Some(DrawingKind::TrendLine));

    let last = (500.0, 200.0);
    click(&mut chart, last.0, last.1);
    let events = chart.take_input_events();
    let [ChartInputEvent::DrawingCreated(id)] = events[..] else {
        panic!("the next click commits the line: {events:?}");
    };
    let anchors = anchor_px(&chart, id);
    assert_eq!(anchors.len(), 2, "{anchors:?}");
    assert_near(anchors[0], first, "first anchor");
    assert_near(anchors[1], last, "second anchor");
    assert_eq!(chart.drawing_revision(), revision + 1, "one create step");
    assert_eq!(chart.active_drawing_tool(), None, "a one-shot tool");
    assert!(chart.undo_drawing());
    assert!(chart.drawings().is_empty());
}
