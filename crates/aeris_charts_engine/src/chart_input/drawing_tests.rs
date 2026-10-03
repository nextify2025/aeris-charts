//! Drawing and annotation interactions driven end to end through the input controller: text
//! placement and editing, path sequences, the Shift measure, drag cancellation, anchor edits,
//! magnet placement, and Delete routing by selection owner.

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

    // Later clicks, a double-click included, are ordinary clicks again.
    click(&mut chart, 320.0, 160.0);
    double_click(&mut chart, 320.0, 160.0);
    assert!(chart.drawings().is_empty());
    assert!(chart.take_input_events().is_empty());
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
