//! Chart chrome through the controller: interaction switches on real pointer input, axis resets,
//! cursor resolution, and region edges.

use super::tests::*;
use super::*;

fn hover(chart: &mut ChartEngine, x: f64, y: f64) -> ChartCursor {
    chart.input_pointer_move(at(x, y), false);
    chart.input_cursor()
}

fn options(change: impl FnOnce(&mut InteractionOptions)) -> InteractionOptions {
    let mut options = InteractionOptions::default();
    change(&mut options);
    options
}

/// The fixture with a preserved empty second pane below it.
fn two_panes() -> ChartEngine {
    let mut chart = chart();
    chart.add_pane(true).unwrap();
    relayout(&mut chart);
    chart
}

fn stretch(chart: &ChartEngine) -> (f64, f64) {
    (chart.panes[0].stretch_factor, chart.panes[1].stretch_factor)
}

// --- interaction switches ---

#[test]
fn pan_off_keeps_the_view_still_but_a_click_still_selects() {
    let mut chart = chart();
    chart.set_interaction_options(options(|o| o.pan = false));
    chart.set_price_scale_auto_scale(0, false, false);
    let (x, y) = empty_pane_point(&chart);
    let position = chart.scroll_position();
    let range = chart.price_scale_visible_range(0, false);

    chart.input_pointer_down(at(x, y), 1);
    chart.input_pointer_move(at(x + 30.0, y + 30.0), true);
    chart.input_pointer_move(at(x + 60.0, y + 60.0), true);
    assert_ne!(chart.input_cursor(), ChartCursor::Grabbing);
    chart.input_pointer_up(at(x + 60.0, y + 60.0));
    assert_eq!(chart.scroll_position(), position);
    assert_eq!(
        chart.price_scale_visible_range(0, false),
        range,
        "a manual price scale stays put too"
    );

    let (sx, sy) = series_point(&chart);
    click(&mut chart, sx, sy);
    assert_eq!(chart.selected_series(), Some(0));
}

#[test]
fn panes_resize_off_hides_the_separator_affordance_and_keeps_the_layout() {
    let mut chart = two_panes();
    chart.set_interaction_options(options(|o| o.panes_resize = false));
    let separator = chart.panes[1].top;
    assert_ne!(hover(&mut chart, 200.0, separator), ChartCursor::ResizeRow);
    assert_eq!(chart.separator_hover(), None);

    let before = stretch(&chart);
    let position = chart.scroll_position();
    chart.input.take_frame_invalidation();
    // The drag also runs sideways, so a press that fell through to a pan would move the view.
    chart.input_pointer_down(at(200.0, separator), 1);
    chart.input_pointer_move(at(210.0, separator + 10.0), true);
    chart.input_pointer_move(at(240.0, separator + 40.0), true);
    assert_ne!(
        chart.input_cursor(),
        ChartCursor::Grabbing,
        "a gated separator never pans"
    );
    chart.input_pointer_up(at(240.0, separator + 40.0));
    assert_eq!(stretch(&chart), before);
    assert_eq!(
        chart.input.take_frame_invalidation(),
        (true, false),
        "no pane geometry changed"
    );
    assert_eq!(chart.scroll_position(), position, "not a pan");
}

#[test]
fn price_axis_scaling_off_makes_the_axis_inert_chrome() {
    let mut chart = chart();
    chart.set_interaction_options(options(|o| o.axis_scale_price = false));
    let axis_x = chart.pane_w + 10.0;
    assert_eq!(hover(&mut chart, axis_x, 100.0), ChartCursor::Default);
    let range = chart.price_scale_visible_range(0, false);

    chart.input_pointer_down(at(axis_x, 100.0), 1);
    chart.input_pointer_move(at(axis_x, 130.0), true);
    chart.input_pointer_move(at(axis_x, 160.0), true);
    assert_eq!(chart.input_cursor(), ChartCursor::Default);
    chart.input_pointer_up(at(axis_x, 160.0));
    assert_eq!(chart.price_scale_auto_scale(0, false), Some(true));
    assert_eq!(chart.price_scale_visible_range(0, false), range);
}

#[test]
fn time_axis_scaling_off_makes_the_axis_inert_chrome() {
    let mut chart = chart();
    chart.set_interaction_options(options(|o| o.axis_scale_time = false));
    let time_y = chart.pane_h + 4.0;
    assert_eq!(hover(&mut chart, 200.0, time_y), ChartCursor::Default);
    let (spacing, position) = (chart.bar_spacing(), chart.scroll_position());

    chart.input_pointer_down(at(200.0, time_y), 1);
    chart.input_pointer_move(at(160.0, time_y), true);
    chart.input_pointer_move(at(120.0, time_y), true);
    assert_eq!(chart.input_cursor(), ChartCursor::Default);
    chart.input_pointer_up(at(120.0, time_y));
    assert_eq!(chart.bar_spacing(), spacing);
    assert_eq!(chart.scroll_position(), position);
}

#[test]
fn axis_reset_switches_keep_manual_axes_on_double_click() {
    let mut chart = chart();
    chart.set_interaction_options(options(|o| {
        o.axis_double_click_reset_price = false;
        o.axis_double_click_reset_time = false;
    }));
    let axis_x = chart.pane_w + 10.0;
    let time_y = chart.pane_h + 4.0;
    drag(&mut chart, (axis_x, 100.0), (axis_x, 160.0));
    drag(&mut chart, (200.0, time_y), (120.0, time_y));
    assert_eq!(chart.price_scale_auto_scale(0, false), Some(false));
    let range = chart.price_scale_visible_range(0, false);
    let (spacing, position) = (chart.bar_spacing(), chart.scroll_position());

    click(&mut chart, axis_x, 100.0);
    double_click(&mut chart, axis_x, 100.0);
    click(&mut chart, 200.0, time_y);
    double_click(&mut chart, 200.0, time_y);
    assert_eq!(chart.price_scale_auto_scale(0, false), Some(false));
    assert_eq!(chart.price_scale_visible_range(0, false), range);
    assert_eq!(chart.bar_spacing(), spacing);
    assert_eq!(chart.scroll_position(), position);
}

// --- axis resets and non-scalable axes ---

/// A time-axis double-click is the axis-local twin of `reset_time_scale`: it restores the
/// configured default spacing and offset, not the spacing `fit_content` chose.
#[test]
fn time_axis_double_click_restores_the_default_time_scale() {
    let mut chart = chart();
    let time_y = chart.pane_h + 4.0;
    drag(&mut chart, (200.0, time_y), (120.0, time_y));
    // Pan through real input too, so the offset is off its default before the reset.
    let (x, y) = empty_pane_point(&chart);
    drag(&mut chart, (x, y), (x - 60.0, y));
    let options = chart.time_scale.options();
    let (default_spacing, default_offset) = (options.bar_spacing, options.right_offset);
    assert_ne!(chart.bar_spacing(), default_spacing);
    assert_ne!(chart.scroll_position(), default_offset);

    click(&mut chart, 200.0, time_y);
    double_click(&mut chart, 200.0, time_y);
    assert_eq!(chart.bar_spacing(), default_spacing);
    assert_eq!(chart.scroll_position(), default_offset);
}

#[test]
fn a_percentage_price_axis_cannot_be_drag_scaled() {
    let mut chart = chart();
    chart.set_price_scale_mode(0, false, PriceScaleMode::Percentage);
    chart.autoscale_visible();
    let axis_x = chart.pane_w + 10.0;
    assert_eq!(hover(&mut chart, axis_x, 100.0), ChartCursor::Default);

    chart.input_pointer_down(at(axis_x, 100.0), 1);
    chart.input_pointer_move(at(axis_x, 130.0), true);
    chart.input_pointer_move(at(axis_x, 160.0), true);
    assert_eq!(chart.input_cursor(), ChartCursor::Default, "inert press");
    chart.input_pointer_up(at(axis_x, 160.0));
    assert_eq!(chart.price_scale_auto_scale(0, false), Some(true));
}

// --- cursors ---

#[test]
fn hover_cursors_name_the_gesture_under_the_pointer() {
    let mut chart = chart();
    let (x, y) = empty_pane_point(&chart);
    assert_eq!(hover(&mut chart, x, y), ChartCursor::Crosshair);
    let axis_x = chart.pane_w + 10.0;
    let axis_cursor = hover(&mut chart, axis_x, 100.0);
    assert_eq!(axis_cursor, ChartCursor::ResizeVertical);
    assert_eq!(chart.crosshair, None, "axis chrome");

    // A selected rectangle resizes diagonally from its corners and straight from its edges.
    let id = chart
        .add_drawing(
            DrawingKind::Rectangle,
            0,
            vec![
                DrawingPoint {
                    logical: 20.0,
                    price: 101.0,
                },
                DrawingPoint {
                    logical: 35.0,
                    price: 106.0,
                },
            ],
            None,
        )
        .unwrap();
    chart.build_frame();
    chart.set_selected_drawing(Some(id));
    let a = chart.drawing_point_to_coordinate(id, 0).unwrap();
    let b = chart.drawing_point_to_coordinate(id, 1).unwrap();
    let (left, right) = (a.0.min(b.0), a.0.max(b.0));
    let (top, bottom) = (a.1.min(b.1), a.1.max(b.1));
    for ((x, y), cursor) in [
        ((left, top), ChartCursor::ResizeNwse),
        ((right, top), ChartCursor::ResizeNesw),
        ((right, bottom), ChartCursor::ResizeNwse),
        ((left, bottom), ChartCursor::ResizeNesw),
        (((left + right) / 2.0, top), ChartCursor::ResizeVertical),
        ((right, (top + bottom) / 2.0), ChartCursor::ResizeHorizontal),
    ] {
        assert_eq!(hover(&mut chart, x, y), cursor, "({x}, {y})");
    }
}

#[test]
fn a_drawing_body_shows_move_then_grabbing_while_dragged() {
    let mut chart = chart();
    let (id, (x, y)) = trend_line_body(&mut chart);
    assert_eq!(hover(&mut chart, x, y), ChartCursor::Move);
    let before = chart.drawing(id).unwrap().points.clone();

    chart.input_pointer_down(at(x, y), 1);
    chart.input_pointer_move(at(x + 20.0, y + 10.0), true);
    chart.input_pointer_move(at(x + 40.0, y + 20.0), true);
    assert_eq!(chart.input_cursor(), ChartCursor::Grabbing);
    chart.input_pointer_up(at(x + 40.0, y + 20.0));
    assert_ne!(chart.drawing(id).unwrap().points, before);
    assert_eq!(
        chart.input_cursor(),
        ChartCursor::Move,
        "the body followed the pointer"
    );
}

// --- region edges ---

#[test]
fn the_left_price_axis_strip_scales_the_left_scale() {
    let mut chart = chart();
    let target = PriceScaleTarget::Left;
    assert!(chart.set_price_scale_visible_for(0, target, true));
    chart.set_series_price_scale(0, target);
    chart.autoscale_visible();
    relayout(&mut chart);
    chart.autoscale_visible();
    assert!(chart.pane_left > 10.0);

    // Pane x is measured from the plot's left edge, so the left strip is negative.
    let strip_x = -10.0;
    assert_eq!(
        chart.region_at(strip_x, 100.0),
        ChartRegion::PriceAxis { pane: 0, target }
    );
    let strip_cursor = hover(&mut chart, strip_x, 100.0);
    assert_eq!(strip_cursor, ChartCursor::ResizeVertical);
    assert_eq!(chart.crosshair, None);
    drag(&mut chart, (strip_x, 100.0), (strip_x, 160.0));
    assert_eq!(chart.price_scale_auto_scale_for(0, target), Some(false));
    assert_eq!(chart.price_scale_auto_scale(0, false), Some(true));
}

#[test]
fn the_separator_grabs_only_within_its_hit_tolerance() {
    let mut chart = two_panes();
    let separator = chart.panes[1].top;
    let inside = PANE_SEPARATOR_HIT - 0.25;
    let outside = PANE_SEPARATOR_HIT + 0.25;
    for y in [separator - inside, separator + inside] {
        assert_eq!(hover(&mut chart, 200.0, y), ChartCursor::ResizeRow, "{y}");
        assert_eq!(chart.separator_hover(), Some(0));
    }
    for y in [separator - outside, separator + outside] {
        assert_ne!(hover(&mut chart, 200.0, y), ChartCursor::ResizeRow, "{y}");
        assert_eq!(chart.separator_hover(), None);
    }

    let before = stretch(&chart);
    drag(
        &mut chart,
        (200.0, separator + outside),
        (200.0, separator + outside + 40.0),
    );
    assert_eq!(stretch(&chart), before, "just outside");
    drag(
        &mut chart,
        (200.0, separator + inside),
        (200.0, separator + inside + 40.0),
    );
    assert!(chart.panes[0].stretch_factor > before.0);
}
