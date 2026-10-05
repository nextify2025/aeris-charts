//! View motion through the controller: kinetic coasting, wheel routing, pinch guards, and manual
//! price panning.

use super::tests::*;
use super::*;

#[track_caller]
fn assert_close(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 1e-9, "{actual} != {expected}");
}

/// A rightward pan from `(x, y)`: the press at `t0`, a 10 px sample that crosses the slop and
/// anchors the pan, then three 30 px samples, each `step_ms` after the previous one. The release
/// comes `release_lag_ms` after the last sample; its time is returned.
fn timed_pan(
    chart: &mut ChartEngine,
    (x, y): (f64, f64),
    t0: f64,
    step_ms: f64,
    release_lag_ms: f64,
) -> f64 {
    chart.input_pointer_down(at_ms(x, y, t0), 1);
    let mut t = t0;
    for dx in [10.0, 40.0, 70.0, 100.0] {
        t += step_ms;
        chart.input_pointer_move(at_ms(x + dx, y, t), true);
    }
    let release = t + release_lag_ms;
    chart.input_pointer_up(at_ms(x + 100.0, y, release));
    release
}

/// Where the engine's kinetic model puts a coast launched from `position` at `speed` (bars per
/// ms) after `elapsed_ms`.
fn coast(position: f64, speed: f64, elapsed_ms: f64) -> f64 {
    position + speed * (KINETIC_DUMPING.powf(elapsed_ms) - 1.0) / KINETIC_DUMPING.ln()
}

fn wheel(x: f64, y: f64, delta_x: f64, delta_y: f64) -> WheelSample {
    WheelSample {
        x,
        y,
        delta_x,
        delta_y,
        ..WheelSample::default()
    }
}

/// A vertical pane drag from `(x, y)`: a 10 px sample crosses the slop and anchors the pan, and
/// the pointer then travels `dy` further before the release.
fn vertical_pan(chart: &mut ChartEngine, (x, y): (f64, f64), dy: f64) {
    chart.input_pointer_down(at(x, y), 1);
    chart.input_pointer_move(at(x, y + 10.0), true);
    chart.input_pointer_move(at(x, y + 10.0 + dy / 2.0), true);
    chart.input_pointer_move(at(x, y + 10.0 + dy), true);
    chart.input_pointer_up(at(x, y + 10.0 + dy));
}

/// The fixture with a line series moved into a second pane, laid out and autoscaled.
fn two_pane_chart() -> (ChartEngine, SeriesId) {
    let mut chart = chart();
    let line = chart.add_series(SeriesKind::Line);
    let times: Vec<f64> = (0..BARS).map(|i| 1_000.0 + i as f64 * 60.0).collect();
    let values: Vec<f64> = (0..BARS).map(|i| 50.0 + (i % 5) as f64).collect();
    chart
        .set_series_data(line, &times, &values, &values, &values, &values)
        .unwrap();
    chart.set_series_pane(line, 1, 1.0);
    relayout(&mut chart);
    chart.autoscale_visible();
    assert_eq!(chart.panes.len(), 2);
    (chart, line)
}

/// Empty plot space in `pane` with room for a 90 px downward drag below it.
fn empty_point_in_pane(chart: &ChartEngine, pane: usize) -> (f64, f64) {
    let top = chart.panes[pane].top as i32;
    let rows = (top + 12)..(top + chart.panes[pane].height as i32 - 100);
    (40..chart.pane_w as i32)
        .step_by(17)
        .flat_map(|x| rows.clone().step_by(7).map(move |y| (x, y)))
        .map(|(x, y)| (f64::from(x), f64::from(y)))
        .find(|&(x, y)| {
            chart.hit_test_series(x, y).is_none()
                && chart.region_at(x, y) == ChartRegion::Pane
                && chart.pane_index_at_y(y) == pane
        })
        .expect("the pane has empty space")
}

// --- kinetic coast ---

#[test]
fn a_released_flick_coasts_on_input_tick_in_the_drag_direction_until_it_settles() {
    let mut chart = kinetic_chart();
    let start = empty_pane_point(&chart);
    let (before, spacing) = (chart.scroll_position(), chart.bar_spacing());
    let release = timed_pan(&mut chart, start, 1_000.0, 8.0, 4.0);
    let released = chart.scroll_position();
    // The threshold sample anchors the pan, so the 90 px after it scroll the view.
    assert_close(released, before - 90.0 / spacing);
    assert!(chart.input_animating(), "the flick coasts");

    // Three equal 30 px / 8 ms segments launch the coast at that speed; it then decays with the
    // kinetic damping from the release.
    let speed = -30.0 / spacing / 8.0;
    assert!(chart.input_tick(release + 16.0));
    let first = chart.scroll_position();
    assert_close(first, coast(released, speed, 16.0));
    assert!(first < released, "toward earlier bars, like the drag");
    assert!(chart.input_tick(release + 64.0));
    let second = chart.scroll_position();
    assert_close(second, coast(released, speed, 64.0));
    assert!(second < first);
    assert!(chart.input_animating());

    // Long after the coast ran out, the next tick ends it and later ticks move nothing.
    assert!(chart.input_tick(release + 100_000.0));
    assert!(!chart.input_animating());
    let settled = chart.scroll_position();
    assert!(!chart.input_tick(release + 100_016.0));
    assert_eq!(chart.scroll_position(), settled);
}

#[test]
fn a_new_press_stops_the_coast_where_it_is() {
    let mut chart = kinetic_chart();
    let (x, y) = empty_pane_point(&chart);
    let release = timed_pan(&mut chart, (x, y), 1_000.0, 8.0, 4.0);
    assert!(chart.input_tick(release + 16.0));
    let frozen = chart.scroll_position();

    chart.input_pointer_down(at_ms(x, y, release + 24.0), 1);
    assert!(!chart.input_animating());
    assert_eq!(chart.scroll_position(), frozen);
    chart.input_pointer_up(at_ms(x, y, release + 30.0));
    assert!(!chart.input_tick(release + 48.0));
    assert!(!chart.input_tick(release + 400.0));
    assert_eq!(chart.scroll_position(), frozen);
}

#[test]
fn a_wheel_stops_the_coast_and_pans_exactly_its_own_step() {
    let mut chart = kinetic_chart();
    let start = empty_pane_point(&chart);
    let release = timed_pan(&mut chart, start, 1_000.0, 8.0, 4.0);
    assert!(chart.input_tick(release + 16.0));
    let frozen = chart.scroll_position();

    assert!(chart.input_wheel(wheel(300.0, 100.0, 0.5, 0.0)));
    assert!(!chart.input_animating());
    // The coast's scroll session closed, so the wheel's own scroll starts from where it stopped.
    let panned = frozen - WHEEL_SCROLL_PX_PER_DELTA * 0.5 / chart.bar_spacing();
    assert_close(chart.scroll_position(), panned);
    let after_wheel = chart.scroll_position();
    assert!(!chart.input_tick(release + 48.0));
    assert_eq!(chart.scroll_position(), after_wheel);
}

#[test]
fn a_navigation_key_stops_the_coast_and_applies_its_own_step() {
    let mut chart = kinetic_chart();
    let start = empty_pane_point(&chart);
    let release = timed_pan(&mut chart, start, 1_000.0, 8.0, 4.0);
    assert!(chart.input_tick(release + 16.0));
    let frozen = chart.scroll_position();

    let none = InputModifiers::default();
    assert!(chart.input_key_down(ChartKey::PageDown, none, false, release + 20.0));
    assert!(!chart.input_animating());
    let page = chart.pane_w / chart.bar_spacing() * KEYBOARD_PAGE_FRACTION;
    assert_close(chart.scroll_position(), frozen + page);
    let after_key = chart.scroll_position();
    assert!(!chart.input_tick(release + 48.0));
    assert_eq!(chart.scroll_position(), after_key);
}

#[test]
fn slow_or_late_releases_and_the_default_options_never_coast() {
    let start = empty_pane_point(&chart());

    // 30 px every 300 ms is 0.1 px/ms, below the coast's minimum speed.
    let mut slow = kinetic_chart();
    let release = timed_pan(&mut slow, start, 1_000.0, 300.0, 4.0);
    assert!(!slow.input_animating());
    let settled = slow.scroll_position();
    assert!(!slow.input_tick(release + 16.0));
    assert_eq!(slow.scroll_position(), settled);

    // A fast flick released after the pointer rested is a placement, not a throw.
    let mut late = kinetic_chart();
    let release = timed_pan(&mut late, start, 1_000.0, 8.0, 200.0);
    assert!(!late.input_animating());
    let settled = late.scroll_position();
    assert!(!late.input_tick(release + 16.0));
    assert_eq!(late.scroll_position(), settled);

    // Mouse kinetics are opt-in: the same fast flick stops dead by default.
    let mut plain = chart();
    assert!(!plain.interaction_options().kinetic_mouse);
    let release = timed_pan(&mut plain, start, 1_000.0, 8.0, 4.0);
    assert!(!plain.input_animating());
    let settled = plain.scroll_position();
    assert!(!plain.input_tick(release + 16.0));
    assert_eq!(plain.scroll_position(), settled);
}

// --- wheel ---

#[test]
fn a_horizontal_wheel_pans_toward_later_bars_without_zooming() {
    let mut chart = chart();
    let (spacing, position) = (chart.bar_spacing(), chart.scroll_position());
    let (from, to) = chart.visible_logical_range().unwrap();
    assert!(chart.input_wheel(wheel(300.0, 100.0, 0.5, 0.0)));
    assert_eq!(chart.bar_spacing(), spacing, "no zoom");
    // The reference scrolls 80 px per normalized step; a positive deltaX reveals later bars, as
    // in the browser host.
    let shift = -WHEEL_SCROLL_PX_PER_DELTA * 0.5 / spacing;
    assert!(shift > 0.0);
    assert_close(chart.scroll_position(), position + shift);
    let (after_from, after_to) = chart.visible_logical_range().unwrap();
    assert_close(after_from, from + shift);
    assert_close(after_to, to + shift);

    // A negative deltaX goes back by the same amount.
    assert!(chart.input_wheel(wheel(300.0, 100.0, -0.5, 0.0)));
    assert_close(chart.scroll_position(), position);
}

#[test]
fn disabled_wheel_scroll_leaves_the_wheel_to_the_page() {
    let mut chart = chart();
    for wheel_behavior in [WheelBehavior::Auto, WheelBehavior::Pan] {
        chart.set_interaction_options(InteractionOptions {
            wheel_scroll: false,
            wheel_behavior,
            ..InteractionOptions::default()
        });
        let (spacing, position) = (chart.bar_spacing(), chart.scroll_position());
        assert!(
            !chart.input_wheel(wheel(300.0, 100.0, 0.5, 0.0)),
            "{wheel_behavior:?}"
        );
        assert_eq!(chart.bar_spacing(), spacing);
        assert_eq!(chart.scroll_position(), position);
    }
}

#[test]
fn pan_mode_pans_along_the_dominant_wheel_axis_and_never_zooms() {
    let mut chart = chart();
    chart.set_interaction_options(InteractionOptions {
        wheel_behavior: WheelBehavior::Pan,
        ..InteractionOptions::default()
    });
    let (spacing, position) = (chart.bar_spacing(), chart.scroll_position());
    let step = -WHEEL_SCROLL_PX_PER_DELTA / spacing;

    // A vertical wheel pans; wheel up (positive) goes back in time.
    assert!(chart.input_wheel(wheel(300.0, 100.0, 0.0, 1.0)));
    assert_eq!(chart.bar_spacing(), spacing);
    assert_close(chart.scroll_position(), position - step);

    // A mostly horizontal sample pans by its horizontal delta only.
    assert!(chart.input_wheel(wheel(300.0, 100.0, 0.5, 0.2)));
    assert_eq!(chart.bar_spacing(), spacing);
    assert_close(chart.scroll_position(), position - step + 0.5 * step);
}

#[test]
fn zoom_mode_zooms_the_price_scale_under_the_axis_strip_without_the_axis_switch() {
    let mut chart = chart();
    chart.set_interaction_options(InteractionOptions {
        wheel_behavior: WheelBehavior::Zoom,
        ..InteractionOptions::default()
    });
    assert!(!chart.interaction_options().price_axis_wheel_zoom);
    let axis_x = chart.pane_w + 10.0;
    let y = 100.0;
    let (min, max) = chart.price_scale_visible_range(0, false).unwrap();
    let anchor = chart.series_coordinate_to_price(0, y).unwrap();
    let (spacing, position) = (chart.bar_spacing(), chart.scroll_position());

    assert!(chart.input_wheel(wheel(axis_x, y, 0.0, 1.0)));
    let (after_min, after_max) = chart.price_scale_visible_range(0, false).unwrap();
    // One full notch narrows the range by 10% around the price under the pointer.
    assert_close(after_max - after_min, (max - min) * 0.9);
    assert_close(chart.series_coordinate_to_price(0, y).unwrap(), anchor);
    assert_eq!(chart.price_scale_auto_scale(0, false), Some(false));
    assert_eq!(chart.bar_spacing(), spacing, "the time scale stays put");
    assert_eq!(chart.scroll_position(), position);

    // Over the plot the same wheel zooms time and leaves the price range alone.
    let range = chart.price_scale_visible_range(0, false);
    assert!(chart.input_wheel(wheel(300.0, y, 0.0, 1.0)));
    assert!(chart.bar_spacing() > spacing);
    assert_eq!(chart.price_scale_visible_range(0, false), range);
}

#[test]
fn a_wheel_without_deltas_is_never_consumed() {
    let mut chart = chart();
    for wheel_behavior in [WheelBehavior::Auto, WheelBehavior::Pan, WheelBehavior::Zoom] {
        chart.set_interaction_options(InteractionOptions {
            wheel_behavior,
            price_axis_wheel_zoom: true,
            ..InteractionOptions::default()
        });
        let range = chart.price_scale_visible_range(0, false);
        let (spacing, position) = (chart.bar_spacing(), chart.scroll_position());
        for x in [300.0, chart.pane_w + 10.0] {
            assert!(
                !chart.input_wheel(wheel(x, 100.0, 0.0, 0.0)),
                "{wheel_behavior:?} at {x}"
            );
        }
        assert_eq!(chart.bar_spacing(), spacing);
        assert_eq!(chart.scroll_position(), position);
        assert_eq!(chart.price_scale_visible_range(0, false), range);
    }
}

#[test]
fn wheel_zoom_saturates_at_one_normalized_step() {
    let zoomed = |delta_y: f64| {
        let mut chart = chart();
        assert!(chart.input_wheel(wheel(300.0, 100.0, 0.0, delta_y)));
        chart.bar_spacing()
    };
    assert_eq!(zoomed(3.7), zoomed(1.0));
    assert_eq!(zoomed(-3.7), zoomed(-1.0));
    // Below one step the zoom stays proportional.
    assert!(zoomed(0.5) < zoomed(1.0));
    assert!(zoomed(-0.5) > zoomed(-1.0));
}

#[test]
fn a_price_axis_wheel_zooms_only_the_pane_under_it() {
    let (mut chart, _) = two_pane_chart();
    chart.set_interaction_options(InteractionOptions {
        price_axis_wheel_zoom: true,
        ..InteractionOptions::default()
    });
    let pane0 = chart.price_scale_visible_range(0, false);
    let (min, max) = chart.price_scale_visible_range(1, false).unwrap();
    let spacing = chart.bar_spacing();
    let y = chart.panes[1].top + chart.panes[1].height / 2.0;
    let axis_x = chart.pane_w + 10.0;
    let target = PriceScaleTarget::Right;
    assert_eq!(
        chart.region_at(axis_x, y),
        ChartRegion::PriceAxis { pane: 1, target }
    );

    assert!(chart.input_wheel(wheel(axis_x, y, 0.0, 1.0)));
    let (after_min, after_max) = chart.price_scale_visible_range(1, false).unwrap();
    assert_close(after_max - after_min, (max - min) * 0.9);
    assert_eq!(chart.price_scale_auto_scale(1, false), Some(false));
    assert_eq!(chart.price_scale_visible_range(0, false), pane0);
    assert_eq!(chart.price_scale_auto_scale(0, false), Some(true));
    assert_eq!(chart.bar_spacing(), spacing);
}

// --- pinch ---

#[test]
fn degenerate_pinch_steps_are_not_consumed() {
    let mut chart = chart();
    let (spacing, position) = (chart.bar_spacing(), chart.scroll_position());
    for delta in [0.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(!chart.input_pinch(300.0, 100.0, delta, 0.0), "{delta}");
    }
    assert_eq!(chart.bar_spacing(), spacing);
    assert_eq!(chart.scroll_position(), position);
}

// --- manual price pan ---

#[test]
fn a_pane_drag_moves_a_manual_price_scale_with_the_pointer() {
    let mut chart = chart();
    chart.set_price_scale_auto_scale(0, false, false);
    let (x, y) = empty_pane_point(&chart);
    let (min, max) = chart.price_scale_visible_range(0, false).unwrap();
    // The pan anchors on the threshold sample 10 px below the press.
    let grabbed = chart.series_coordinate_to_price(0, y + 10.0).unwrap();
    let dropped_on = chart.series_coordinate_to_price(0, y + 70.0).unwrap();
    let position = chart.scroll_position();

    vertical_pan(&mut chart, (x, y), 60.0);
    let shift = grabbed - dropped_on;
    assert!(shift > 0.0, "dragging down reveals higher prices");
    let (after_min, after_max) = chart.price_scale_visible_range(0, false).unwrap();
    assert_close(after_min, min + shift);
    assert_close(after_max, max + shift);
    // The grabbed price stays under the pointer.
    let under_pointer = chart.series_coordinate_to_price(0, y + 70.0).unwrap();
    assert_close(under_pointer, grabbed);
    assert_eq!(chart.price_scale_auto_scale(0, false), Some(false));
    assert_eq!(chart.scroll_position(), position, "no time pan");
}

#[test]
fn a_pane_drag_never_unlocks_an_autoscaled_price_scale() {
    let mut chart = chart();
    let range = chart.price_scale_visible_range(0, false);
    let point = empty_pane_point(&chart);
    vertical_pan(&mut chart, point, 60.0);
    assert_eq!(chart.price_scale_auto_scale(0, false), Some(true));
    assert_eq!(chart.price_scale_visible_range(0, false), range);
}

#[test]
fn a_pane_drag_pans_only_the_price_scale_of_the_pane_it_started_in() {
    let (mut chart, line) = two_pane_chart();
    chart.set_price_scale_auto_scale(0, false, false);
    chart.set_price_scale_auto_scale(1, false, false);
    let pane0 = chart.price_scale_visible_range(0, false);
    let (min, max) = chart.price_scale_visible_range(1, false).unwrap();
    let (x, y) = empty_point_in_pane(&chart, 1);
    let grabbed = chart.series_coordinate_to_price(line, y + 10.0).unwrap();
    let dropped_on = chart.series_coordinate_to_price(line, y + 70.0).unwrap();

    vertical_pan(&mut chart, (x, y), 60.0);
    let shift = grabbed - dropped_on;
    assert!(shift > 0.0);
    let (after_min, after_max) = chart.price_scale_visible_range(1, false).unwrap();
    assert_close(after_min, min + shift);
    assert_close(after_max, max + shift);
    assert_eq!(chart.price_scale_visible_range(0, false), pane0);
}
