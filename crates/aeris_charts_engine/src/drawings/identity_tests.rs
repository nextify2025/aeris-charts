//! Drawing anchor time identity (interval switches, clear-then-set, pending anchors, sync and
//! clipboard between charts), chart/per-drawing magnet modes, keyboard handle nudges, undo during
//! a drag, add_drawing option errors, and price-basis rescaling.

use super::tests::on_tick;
use super::*;

const MINUTE: f64 = 60.0;
const HOUR: f64 = 3_600.0;
const DAY: f64 = 86_400.0;
/// 2024-01-01T00:00:00Z, a Monday.
const BASE: f64 = 1_704_067_200.0;

fn spaced(from: f64, step: f64, count: usize) -> Vec<f64> {
    (0..count).map(|index| from + index as f64 * step).collect()
}

fn set_bars(chart: &mut ChartEngine, times: &[f64]) {
    let values = (0..times.len())
        .map(|index| 100.0 + (index % 7) as f64)
        .collect::<Vec<_>>();
    chart
        .set_series_data(0, times, &values, &values, &values, &values)
        .unwrap();
}

fn chart_with(times: &[f64]) -> ChartEngine {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    set_bars(&mut chart, times);
    chart.time_scale.set_width(800.0);
    chart.fit_content();
    chart.build_frame();
    chart
}

fn point(logical: f64, price: f64) -> DrawingPoint {
    DrawingPoint { logical, price }
}

fn at_time(time: f64, price: f64) -> DrawingAnchor {
    DrawingAnchor {
        logical: None,
        price,
        time: Some(time),
    }
}

fn logicals(chart: &ChartEngine, id: DrawingId) -> Vec<f64> {
    chart
        .drawing(id)
        .unwrap()
        .points
        .iter()
        .map(|point| point.logical)
        .collect()
}

fn prices(chart: &ChartEngine, id: DrawingId) -> Vec<f64> {
    chart
        .drawing(id)
        .unwrap()
        .points
        .iter()
        .map(|point| point.price)
        .collect()
}

fn assert_close(actual: f64, expected: f64, message: &str) {
    assert!(
        (actual - expected).abs() < 1e-6,
        "{message}: {actual} != {expected}"
    );
}

fn trend(chart: &mut ChartEngine, first: DrawingPoint, second: DrawingPoint) -> DrawingId {
    chart
        .add_drawing(DrawingKind::TrendLine, 0, vec![first, second], None)
        .unwrap()
}

#[test]
fn anchors_report_time_identity_and_accept_time_inputs() {
    let mut chart = chart_with(&spaced(BASE, HOUR, 10));
    let id = trend(&mut chart, point(2.5, 101.0), point(12.0, 102.0));
    let anchors = chart.drawing_anchors(id).unwrap();
    assert_eq!(anchors[0].time, Some(BASE + 2.5 * HOUR));
    assert_eq!(
        anchors[1].time,
        Some(BASE + 12.0 * HOUR),
        "a future anchor extrapolates with the prevailing bar interval"
    );
    let json: serde_json::Value =
        serde_json::from_str(&chart.drawing_points_json(id).unwrap()).unwrap();
    assert_eq!(json[0]["time"].as_f64(), Some(BASE + 2.5 * HOUR));
    assert_eq!(json[0]["logical"].as_f64(), Some(2.5));

    let by_time = chart
        .add_drawing_anchors(
            DrawingKind::TrendLine,
            0,
            &[
                at_time(BASE + 4.25 * HOUR, 101.0),
                at_time(BASE + 15.0 * HOUR, 102.0),
            ],
            None,
        )
        .unwrap();
    assert_eq!(logicals(&chart, by_time), [4.25, 15.0]);

    // `time` wins over a disagreeing `logical`; the JSON path accepts either form.
    assert!(chart.drawing_set_points(
        by_time,
        &format!(
            r#"[{{"time":{},"price":101}},{{"logical":3,"time":{},"price":102}}]"#,
            BASE + HOUR,
            BASE + 6.0 * HOUR
        )
    ));
    assert_eq!(logicals(&chart, by_time), [1.0, 6.0]);

    // Feeding points() back unchanged is exact and records no history step.
    let anchors = chart.drawing_anchors(id).unwrap();
    chart.set_drawing_anchors(id, &anchors).unwrap();
    assert_eq!(logicals(&chart, id), [2.5, 12.0]);
    assert!(chart.undo_drawing());
    assert_eq!(
        logicals(&chart, by_time),
        [4.25, 15.0],
        "the no-op round trip must not have recorded an undo step"
    );
}

#[test]
fn interval_switches_resolve_anchors_by_time() {
    // Monday 10:00..11:59 one-minute bars; anchors at 10:37 and a future 12:10.
    let start = BASE + 10.0 * HOUR;
    let mut chart = chart_with(&spaced(start, MINUTE, 120));
    let id = trend(&mut chart, point(37.0, 101.0), point(130.0, 102.0));

    // Hourly bars 06:00..15:00 share only the 10:00 and 11:00 stamps with the minute axis.
    set_bars(&mut chart, &spaced(BASE + 6.0 * HOUR, HOUR, 10));
    let rebased = logicals(&chart, id);
    assert_close(
        rebased[0],
        4.0 + 37.0 / 60.0,
        "10:37 lands 37/60 past the 10:00 bar",
    );
    assert_close(
        rebased[1],
        6.0 + 10.0 / 60.0,
        "a future 12:10 anchor resolves by time instead of 70 hourly bars",
    );

    // Daily bars stamped at midnight share no timestamp with the hourly axis.
    set_bars(&mut chart, &spaced(BASE - 5.0 * DAY, DAY, 10));
    let daily = logicals(&chart, id);
    assert_close(
        daily[0],
        5.0 + (10.0 * HOUR + 37.0 * MINUTE) / DAY,
        "10:37 on daily bars",
    );
    assert!(
        (4.5..5.5).contains(&daily[0]),
        "10:37 stays inside that day's bar"
    );

    // Back to the original minute window: the anchors return exactly.
    set_bars(&mut chart, &spaced(start, MINUTE, 120));
    let restored = logicals(&chart, id);
    assert_close(restored[0], 37.0, "minute anchor after the round trip");
    assert_close(restored[1], 130.0, "future anchor after the round trip");
}

#[test]
fn clear_then_set_parks_anchor_times_for_live_and_history_snapshots() {
    let mut chart = chart_with(&spaced(BASE, HOUR, 48));
    let id = trend(&mut chart, point(10.0, 101.0), point(20.0, 102.0));
    chart
        .set_drawing_anchors(id, &[point(12.0, 101.0).into(), point(20.0, 102.0).into()])
        .unwrap();

    // A symbol reload: clear the series, then load a window starting eight hours later.
    set_bars(&mut chart, &[]);
    assert_eq!(
        chart.drawing_anchors(id).unwrap()[0].time,
        Some(BASE + 12.0 * HOUR),
        "a parked anchor still reports its time identity"
    );
    set_bars(&mut chart, &spaced(BASE + 8.0 * HOUR, HOUR, 48));
    assert_eq!(logicals(&chart, id), [4.0, 12.0]);

    // The undo stack was parked and resolved in the same basis.
    assert!(chart.undo_drawing());
    assert_eq!(logicals(&chart, id), [2.0, 12.0]);
}

#[test]
fn time_anchors_added_before_data_resolve_when_data_arrives() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    let id = chart
        .add_drawing_anchors(
            DrawingKind::TrendLine,
            0,
            &[
                at_time(BASE + 3.0 * HOUR, 101.0),
                at_time(BASE + 5.5 * HOUR, 102.0),
            ],
            None,
        )
        .unwrap();
    assert_eq!(
        chart.drawing_anchors(id).unwrap()[1].time,
        Some(BASE + 5.5 * HOUR)
    );
    set_bars(&mut chart, &spaced(BASE, HOUR, 10));
    assert_eq!(logicals(&chart, id), [3.0, 5.5]);
    assert_eq!(
        chart.drawing_anchors(id).unwrap()[1].time,
        Some(BASE + 5.5 * HOUR)
    );
}

#[test]
fn sync_and_clipboard_payloads_resolve_by_time_in_the_receiving_chart() {
    let mut hourly = chart_with(&spaced(BASE, HOUR, 48));
    let id = trend(&mut hourly, point(10.0, 101.0), point(30.5, 102.0));
    hourly.set_drawing_price_basis(Some("raw")).unwrap();
    let payload = hourly.drawing_sync_payload_json("cell-a").unwrap();
    assert!(payload.contains("\"time\""));
    assert!(payload.contains("\"price_basis\":\"raw\""));

    // A daily cell showing the same symbol receives the drawing on the same moments.
    let mut daily = chart_with(&spaced(BASE - 3.0 * DAY, DAY, 10));
    assert!(daily.apply_drawing_sync_payload_json(&payload));
    let synced = logicals(&daily, id);
    assert_close(synced[0], 3.0 + 10.0 / 24.0, "synced first anchor");
    assert_close(synced[1], 3.0 + 30.5 / 24.0, "synced second anchor");
    assert_eq!(daily.drawing_price_basis(), Some("raw"));

    // Copy into a minute chart whose window starts at 09:00.
    let copied = hourly.copy_drawings_json(&[id]).unwrap();
    let mut minute = chart_with(&spaced(BASE + 9.0 * HOUR, MINUTE, 180));
    let pasted = minute.paste_drawings_json(&copied, 0, 0.0, 0.0).unwrap();
    let pasted = logicals(&minute, pasted[0]);
    assert_close(pasted[0], 60.0, "10:00 on the minute axis");
    assert_close(
        pasted[1],
        179.0 + (30.5 * HOUR - (11.0 * HOUR + 59.0 * MINUTE)) / MINUTE,
        "a later anchor extrapolates with the one-minute interval",
    );

    // A same-chart clone stays exact and applies its logical offset after time resolution.
    let clone = hourly.clone_drawing(id, 2.0, 1.0).unwrap();
    assert_eq!(logicals(&hourly, clone), [12.0, 32.5]);
    assert_eq!(prices(&hourly, clone), [102.0, 103.0]);
}

fn ohlc() -> ChartEngine {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    let times = (0..10).map(|i| (i * 3600) as f64).collect::<Vec<_>>();
    let open = [10.0, 11.0, 12.0, 11.0, 10.0, 11.0, 12.0, 13.0, 12.0, 11.0];
    let high = [11.0, 12.0, 13.0, 12.0, 11.0, 12.0, 13.0, 14.0, 13.0, 12.0];
    let low = [9.0, 10.0, 11.0, 10.0, 9.0, 10.0, 11.0, 12.0, 11.0, 10.0];
    let close = [11.0, 12.0, 11.0, 10.0, 11.0, 12.0, 13.0, 12.0, 11.0, 10.0];
    chart
        .set_series_data(0, &times, &open, &high, &low, &close)
        .unwrap();
    chart.time_scale.set_width(800.0);
    chart.fit_content();
    chart.build_frame();
    chart
}

fn x_at(chart: &ChartEngine, logical: f64) -> f64 {
    chart.logical_to_coordinate(logical).unwrap()
}

fn y_at(chart: &ChartEngine, price: f64) -> f64 {
    chart.series_price_to_coordinate(0, price).unwrap()
}

const NO_KEYS: DrawingModifiers = DrawingModifiers {
    magnet: false,
    straighten: false,
};
const TOGGLE: DrawingModifiers = DrawingModifiers {
    magnet: true,
    straighten: false,
};

fn first_placed(
    chart: &mut ChartEngine,
    x: f64,
    y: f64,
    modifiers: DrawingModifiers,
) -> DrawingPoint {
    chart.drawing_create_cancel();
    assert!(chart.drawing_create_begin(DrawingKind::TrendLine, None));
    assert_eq!(chart.drawing_create_click(x, y, modifiers), -1);
    chart.pending_drawing().unwrap().drawing.points[0]
}

#[test]
fn chart_magnet_modes_snap_without_a_modifier_and_ctrl_toggles_them() {
    let mut chart = ohlc();
    let x = (x_at(&chart, 2.0) + x_at(&chart, 3.0)) / 2.0;
    let near = y_at(&chart, 12.0) - 4.0;
    let far = (y_at(&chart, 12.0) + y_at(&chart, 13.0)) / 2.0;
    assert!((far - y_at(&chart, 12.0)).abs() > DRAWING_WEAK_MAGNET_DISTANCE);

    // Default chart mode is off: the historical Ctrl-only strong magnet is unchanged.
    assert_eq!(chart.drawing_magnet_mode(), DrawingMagnetMode::Off);
    assert_ne!(first_placed(&mut chart, x, near, NO_KEYS).price, 12.0);
    assert_eq!(first_placed(&mut chart, x, near, TOGGLE), point(2.0, 12.0));

    chart.set_drawing_magnet_mode(DrawingMagnetMode::Strong);
    assert_eq!(first_placed(&mut chart, x, far, NO_KEYS).logical, 2.0);
    assert!(
        (first_placed(&mut chart, x, near, TOGGLE).price - 12.0).abs() > 1e-9,
        "Ctrl temporarily turns an active magnet off"
    );

    chart.set_drawing_magnet_mode(DrawingMagnetMode::Weak);
    assert_eq!(first_placed(&mut chart, x, near, NO_KEYS), point(2.0, 12.0));
    // Free of the magnet, the anchor still lands on the bar slot under the pointer (bar 2), at
    // the raw price.
    let free = first_placed(&mut chart, x, far, NO_KEYS);
    assert!(
        (free.price - 12.5).abs() < 0.05 && free.logical == 2.0,
        "weak leaves a pointer beyond the capture distance free: {free:?}"
    );
}

#[test]
fn a_drawings_own_magnet_mode_applies_to_its_edits() {
    let mut chart = ohlc();
    let id = chart
        .add_drawing(
            DrawingKind::TrendLine,
            0,
            vec![point(2.0, 12.5), point(7.0, 13.5)],
            Some(r#"{"magnet":"strong"}"#),
        )
        .unwrap();
    chart.set_selected_drawing(Some(id));
    let (x, y) = chart.drawing_point_to_coordinate(id, 0).unwrap();
    assert!(chart.drawing_drag_start_at(x, y));
    chart.drawing_drag_to(x_at(&chart, 5.0) + 3.0, y_at(&chart, 11.8), NO_KEYS);
    chart.drawing_drag_end();
    assert_eq!(chart.drawing(id).unwrap().points[0], point(5.0, 12.0));
    assert_eq!(
        chart.armed_drawing_magnet(false),
        DrawingMagnetMode::Off,
        "nothing is armed"
    );
    assert!(chart.set_drawing_tool(
        Some(DrawingKind::TrendLine),
        Some(r#"{"magnet":"weak"}"#),
        None
    ));
    assert_eq!(chart.armed_drawing_magnet(false), DrawingMagnetMode::Weak);
    assert_eq!(
        chart.armed_drawing_magnet(true),
        DrawingMagnetMode::Strong,
        "the held modifier upgrades a drawing's own magnet to strong"
    );
}

fn settled() -> ChartEngine {
    chart_with(&(0..10).map(|i| (i * 3600) as f64).collect::<Vec<_>>())
}

fn px(chart: &ChartEngine, id: DrawingId, index: usize) -> (f64, f64) {
    chart.drawing_point_to_coordinate(id, index).unwrap()
}

/// Apply `sender`'s full sync payload to `receiver`; true when the receiver accepted it.
fn sync(sender: &ChartEngine, receiver: &mut ChartEngine) -> bool {
    receiver.apply_drawing_sync_payload_json(&sender.drawing_sync_payload_json("cell-a").unwrap())
}

/// The receiver shows `id` where the sender does (time resolution may differ in the last ulp).
fn assert_synced(sender: &ChartEngine, receiver: &ChartEngine, id: DrawingId) {
    for (index, (a, b)) in logicals(sender, id)
        .into_iter()
        .zip(logicals(receiver, id))
        .enumerate()
    {
        assert_close(b, a, &format!("synced logical {index}"));
    }
    for (index, (a, b)) in prices(sender, id)
        .into_iter()
        .zip(prices(receiver, id))
        .enumerate()
    {
        assert_close(b, a, &format!("synced price {index}"));
    }
}

#[test]
fn interactive_placement_and_drag_commits_reach_already_synced_cells() {
    let mut a = settled();
    let mut b = settled();
    let line = trend(&mut a, point(2.0, 101.0), point(7.0, 103.0));
    assert!(sync(&a, &mut b), "the first payload syncs");

    // Click placement of a two-anchor catalog tool.
    assert!(a.drawing_create_begin(DrawingKind::FibonacciRetracement, None));
    let (x0, y0) = (x_at(&a, 1.0), y_at(&a, 102.0));
    let (x1, y1) = (x_at(&a, 5.0), y_at(&a, 105.0));
    assert_eq!(a.drawing_create_click(x0, y0, NO_KEYS), -1);
    assert!(a.drawing_create_click(x1, y1, NO_KEYS) > 0);
    assert_eq!(a.drawings().len(), 2);
    assert!(
        sync(&a, &mut b),
        "a click-placed drawing advances the sync revision"
    );
    assert_eq!(b.drawings().len(), 2);

    // A freehand stroke.
    assert!(a.brush_create_start(None, x0, y0));
    assert!(a.brush_create_add(x0 + 20.0, y0 + 10.0));
    assert!(a.brush_create_add(x0 + 40.0, y0 - 10.0));
    assert!(a.brush_create_end() > 0);
    assert!(
        sync(&a, &mut b),
        "a committed stroke advances the sync revision"
    );
    assert_eq!(b.drawings().len(), 3);

    // A pointer drag of the trend line's first anchor.
    a.set_selected_drawing(Some(line));
    let revision = a.drawing(line).unwrap().revision;
    let (x, y) = px(&a, line, 0);
    assert!(a.drawing_drag_start_at(x, y));
    a.drawing_drag_to(x + 40.0, y + 30.0, NO_KEYS);
    a.drawing_drag_end();
    assert!(
        a.drawing(line).unwrap().revision > revision,
        "a committed drag advances the drawing's own revision"
    );
    assert!(
        sync(&a, &mut b),
        "a committed drag advances the sync revision"
    );
    assert_synced(&a, &b, line);
    // Undo restores the pre-drag revision along with the geometry.
    assert!(a.undo_drawing());
    assert_eq!(a.drawing(line).unwrap().revision, revision);
    assert!(a.redo_drawing());
    assert!(sync(&a, &mut b));

    // A keyboard nudge.
    let before = a.drawing(line).unwrap().points.clone();
    assert!(a.nudge_selected_drawing(0.0, -5.0, None));
    assert_ne!(a.drawing(line).unwrap().points, before);
    assert!(
        sync(&a, &mut b),
        "a keyboard nudge advances the sync revision"
    );
    assert_synced(&a, &b, line);

    // A drag that ends where it started and a cancelled drag commit nothing.
    let settled_revision = a.drawing_sync_revision;
    let drawing_revision = a.drawing(line).unwrap().revision;
    let (x, y) = px(&a, line, 0);
    assert!(a.drawing_drag_start_at(x, y));
    a.drawing_drag_end();
    assert!(a.drawing_drag_start_at(x, y));
    a.drawing_drag_to(x + 40.0, y + 30.0, NO_KEYS);
    a.drawing_drag_cancel();
    assert_eq!(a.drawing_sync_revision, settled_revision);
    assert_eq!(a.drawing(line).unwrap().revision, drawing_revision);
    assert!(
        !sync(&a, &mut b),
        "an unchanged payload is still a stale echo"
    );
}

#[test]
fn keyboard_nudge_moves_rectangle_and_position_handles_by_the_delta() {
    let mut chart = settled();
    let rectangle = chart
        .add_drawing(
            DrawingKind::Rectangle,
            0,
            vec![point(2.0, 101.0), point(6.0, 104.0)],
            None,
        )
        .unwrap();
    chart.set_selected_drawing(Some(rectangle));
    assert_eq!(chart.drawing_handle_count(rectangle), Some(8));
    let (top_left, bottom_right) = (px(&chart, rectangle, 1), px(&chart, rectangle, 0));
    // Handle 1 is the top edge; handle 3 the right edge. Horizontal nudges step whole bars, at
    // least one: 2 px moves the right edge one bar.
    assert!(chart.nudge_selected_drawing(0.0, 3.0, Some(1)));
    assert!(chart.nudge_selected_drawing(2.0, 0.0, Some(3)));
    let (moved_top, moved_bottom) = (px(&chart, rectangle, 1), px(&chart, rectangle, 0));
    assert_close(
        moved_top.1,
        on_tick(&chart, top_left.1 + 3.0),
        "top edge moved by the nudge onto the price tick",
    );
    assert_close(
        moved_top.0,
        top_left.0 + chart.bar_spacing(),
        "right edge moved by one bar",
    );
    assert_eq!(logicals(&chart, rectangle), vec![2.0, 7.0]);
    assert_close(moved_bottom.1, bottom_right.1, "bottom edge unchanged");
    assert_close(moved_bottom.0, bottom_right.0, "left edge unchanged");

    let position = chart
        .add_drawing(
            DrawingKind::LongPosition,
            0,
            vec![point(2.0, 102.0), point(7.0, 104.0), point(2.0, 101.0)],
            None,
        )
        .unwrap();
    chart.set_selected_drawing(Some(position));
    assert_eq!(chart.drawing_handle_count(position), Some(4));
    let before = [0, 1, 2].map(|index| px(&chart, position, index));
    // A position's levels sit on the price grid (the 0.01 display tick) and its width on whole
    // bar slots, like a pointer drag, so a nudge lands on the grid. It still moves: one that
    // rounds back to where it started steps one tick or slot the way the key points.
    let slot = (before[1].0 - before[0].0) / 5.0;
    let tick = (before[2].1 - before[0].1) * 0.01;
    // Handle 3 (the stop) is reachable from the keyboard and moves only the stop level.
    assert!(chart.nudge_selected_drawing(0.0, 4.0, Some(3)));
    // Handle 2 (the width) moves only the target's x: five px is under a slot, so it takes one.
    assert!(chart.nudge_selected_drawing(5.0, 0.0, Some(2)));
    let after = [0, 1, 2].map(|index| px(&chart, position, index));
    assert_close(after[0].0, before[0].0, "entry x");
    assert_close(after[0].1, before[0].1, "entry price");
    assert_close(after[1].1, before[1].1, "target price");
    assert_close(
        after[1].0,
        before[1].0 + slot,
        "width moved by one whole slot",
    );
    assert_eq!(logicals(&chart, position), vec![2.0, 8.0, 2.0]);
    let stop = prices(&chart, position)[2];
    assert_close(
        stop,
        (stop * 100.0).round() / 100.0,
        "stop level on the price tick",
    );
    assert!(
        (after[2].1 - (before[2].1 + 4.0)).abs() <= tick / 2.0 + 1e-6,
        "stop moved by the nudge to the nearest tick: {} vs {}",
        after[2].1,
        before[2].1 + 4.0
    );
    // A nudge far below one tick still steps one tick the key's way (down is a lower price).
    let before_tick = prices(&chart, position)[2];
    assert!(chart.nudge_selected_drawing(0.0, tick / 10.0, Some(3)));
    assert_close(
        prices(&chart, position)[2],
        before_tick - 0.01,
        "a sub-tick nudge steps one tick down",
    );
    assert!(chart.nudge_selected_drawing(0.0, -tick / 10.0, Some(3)));
    assert_close(
        prices(&chart, position)[2],
        before_tick,
        "and one tick back up",
    );
    assert!(!chart.nudge_selected_drawing(0.0, 1.0, Some(4)));

    // Keyboard handles never jump to the pane's top-left corner even with a chart magnet.
    chart.set_drawing_magnet_mode(DrawingMagnetMode::Strong);
    assert!(chart.nudge_selected_drawing(0.0, 1.0, Some(3)));
    assert!(
        (px(&chart, position, 2).1 - (after[2].1 + 1.0)).abs() <= tick / 2.0 + 1e-6,
        "an unsnapped stop nudge stays on the tick grid near the key's delta"
    );
    // Locked drawings stay put.
    assert!(chart.set_drawing_locked(position, true));
    assert!(!chart.nudge_selected_drawing(0.0, 1.0, None));
}

/// A projected handle can sit on screen while the anchor it drives is off it (a channel's width
/// anchor far right of the pane). Dragging the handle moves that anchor by the bars the pointer
/// crossed from the handle, never onto the slot under the pointer, which the crosshair clamps to
/// the visible range.
#[test]
fn a_projected_handle_drags_its_off_screen_anchor_by_the_bars_it_crosses() {
    let mut chart = settled();
    // The second line runs half a unit under the base, through the width anchor 14 bars past the
    // base's end and well right of the pane; its handle sits on bar 4, between the base anchors.
    let id = chart
        .add_drawing(
            DrawingKind::ParallelChannel,
            0,
            vec![point(2.0, 101.0), point(6.0, 103.0), point(20.0, 109.5)],
            None,
        )
        .unwrap();
    chart.set_selected_drawing(Some(id));
    chart.build_frame();
    assert!(
        px(&chart, id, 2).0 > chart.pane_w,
        "the anchor is off screen"
    );
    let handle = chart
        .drawing_handle_px(chart.drawing(id).unwrap(), DrawingDragPart::Anchor(2))
        .unwrap();
    assert!(handle.0 > 0.0 && handle.0 < chart.pane_w, "{handle:?}");
    let before = chart.drawing(id).unwrap().points.clone();
    let y = px(&chart, id, 2).1;
    // 25 px is under half of an 80 px bar: the anchor keeps its bar and takes the price tick
    // under the pointer.
    assert_eq!(chart.bar_spacing(), 80.0);
    for (dx, bars) in [(25.0, 0.0), (100.0, 1.0), (-130.0, -2.0)] {
        assert!(chart.drawing_drag_start_at(handle.0, handle.1));
        chart.drawing_drag_to(handle.0 + dx, handle.1 + 6.0, NO_KEYS);
        chart.drawing_drag_end();
        let points = chart.drawing(id).unwrap().points.clone();
        assert_eq!(points[..2], before[..2], "{dx}");
        assert_eq!(points[2].logical, 20.0 + bars, "{dx}");
        assert_close(
            px(&chart, id, 2).1,
            on_tick(&chart, y + 6.0),
            &format!("{dx}: price tick"),
        );
        assert!(chart.undo_drawing());
    }
}

/// A time-snapped channel's width handle sits on its odd base's midpoint, half a bar off the slot
/// grid, so a sub-bar drag can cross a slot at the handle while the width anchor's own px still
/// rounds to its bar. The data clamp checks the anchor's final slot, so an anchor on the first or
/// last data bar never steps off the data.
#[test]
fn a_projected_handle_never_steps_a_time_snapped_anchor_off_the_data() {
    let mut chart = settled();
    let last = (chart.data.merged_times().len() - 1) as f64;
    let mut crossed = false;
    for (edge, dx) in [(last, 0.3), (0.0, -0.3)] {
        let id = chart
            .add_drawing(
                DrawingKind::ParallelChannel,
                0,
                // The second line runs 1.5 under the base, so the handle stays on the pane.
                vec![
                    point(3.0, 101.0),
                    point(6.0, 103.0),
                    point(edge, 99.5 + (edge - 3.0) * 2.0 / 3.0),
                ],
                Some(r#"{"snap_time_to_data":true}"#),
            )
            .unwrap();
        chart.set_selected_drawing(Some(id));
        chart.build_frame();
        let handle = chart
            .drawing_handle_px(chart.drawing(id).unwrap(), DrawingDragPart::Anchor(2))
            .unwrap();
        let dx = dx * chart.bar_spacing();
        crossed |=
            chart.snapped_crosshair_index(handle.0 + dx) != chart.snapped_crosshair_index(handle.0);
        assert!(chart.drawing_drag_start_at(handle.0, handle.1));
        chart.drawing_drag_to(handle.0 + dx, handle.1, NO_KEYS);
        chart.drawing_drag_end();
        let logicals = logicals(&chart, id);
        assert!(
            logicals
                .iter()
                .all(|&logical| (0.0..=last).contains(&logical)),
            "{edge}: {logicals:?}"
        );
        chart.remove_drawing(id);
    }
    assert!(crossed, "one drag crosses a slot at the handle");
}

/// Keyboard nudges step handles in logical space: an off-screen anchor, rectangle bounds corner
/// or edge, or position handle moves exactly one bar per horizontal key step instead of jumping
/// to the slot at the pane edge.
#[test]
fn keyboard_nudges_step_off_screen_handles_by_exactly_one_bar() {
    let mut chart = settled();
    let line = trend(&mut chart, point(2.0, 101.0), point(20.0, 104.0));
    chart.set_selected_drawing(Some(line));
    assert!(px(&chart, line, 1).0 > chart.pane_w);
    assert!(chart.nudge_selected_drawing(1.0, 0.0, Some(1)));
    assert_eq!(logicals(&chart, line), [2.0, 21.0]);
    assert_eq!(prices(&chart, line), [101.0, 104.0]);
    assert!(chart.nudge_selected_drawing(-1.0, 0.0, Some(1)));
    assert_eq!(logicals(&chart, line), [2.0, 20.0]);

    let rectangle = chart
        .add_drawing(
            DrawingKind::Rectangle,
            0,
            vec![point(2.0, 101.0), point(20.0, 104.0)],
            None,
        )
        .unwrap();
    chart.set_selected_drawing(Some(rectangle));
    // Handle 2 is the top-right corner, 3 the right edge, both 11 bars right of the pane.
    for (handle, right) in [(2, 21.0), (3, 22.0)] {
        assert!(chart.nudge_selected_drawing(1.0, 0.0, Some(handle)));
        let points = chart.drawing(rectangle).unwrap().points.clone();
        let mut bars = logicals(&chart, rectangle);
        bars.sort_by(f64::total_cmp);
        assert_eq!(bars, [2.0, right], "handle {handle}");
        let mut levels = points.iter().map(|point| point.price).collect::<Vec<_>>();
        levels.sort_by(f64::total_cmp);
        assert_eq!(levels, [101.0, 104.0], "handle {handle}");
    }

    let position = chart
        .add_drawing(
            DrawingKind::LongPosition,
            0,
            vec![point(2.0, 102.0), point(20.0, 104.0), point(2.0, 101.0)],
            None,
        )
        .unwrap();
    chart.set_selected_drawing(Some(position));
    // Handle 2 is the width, on the target's off-screen right edge.
    assert!(chart.nudge_selected_drawing(1.0, 0.0, Some(2)));
    assert_eq!(logicals(&chart, position), [2.0, 21.0, 2.0]);
    assert_eq!(prices(&chart, position), [102.0, 104.0, 101.0]);
}

/// A keyboard body nudge adds whole bars to every anchor without rounding them, so an anchor
/// between slots (a Shift-straightened end, a time-identity anchor) keeps its fraction.
#[test]
fn keyboard_body_nudges_keep_fractional_anchors_between_their_slots() {
    let mut chart = settled();
    let line = trend(&mut chart, point(2.25, 101.0), point(6.6, 103.0));
    chart.set_selected_drawing(Some(line));
    assert!(chart.nudge_selected_drawing(1.0, 0.0, None));
    assert_eq!(logicals(&chart, line), [3.25, 7.6]);
    assert_eq!(prices(&chart, line), [101.0, 103.0]);
    assert!(chart.nudge_selected_drawing(-1.0, 0.0, None));
    assert_eq!(logicals(&chart, line), [2.25, 6.6]);
}

#[test]
fn keyboard_nudges_that_move_nothing_report_false_and_record_nothing() {
    let mut chart = settled();
    let recolored = trend(&mut chart, point(1.0, 101.0), point(4.0, 103.0));
    assert!(chart.drawing_apply_options(recolored, r##"{"color":"#00ff00"}"##));
    let recolor = chart.drawing(recolored).unwrap().clone();
    // A time-only kind nudged vertically, an anchored VWAP (time-only body) nudged vertically, and
    // an anchored text clamped at the pane's left edge.
    let cases = [
        (
            DrawingKind::VerticalLine,
            vec![point(5.0, 101.0)],
            (0.0, -1.0),
        ),
        (
            DrawingKind::AnchoredVwap,
            vec![point(2.0, 101.0)],
            (0.0, -1.0),
        ),
        (
            DrawingKind::AnchoredText,
            vec![
                chart
                    .drawing_from_px_for(0, crate::DrawingPriceScale::Right, 0.0, 120.0)
                    .unwrap(),
            ],
            (-1.0, 0.0),
        ),
    ];
    for (kind, points, (dx, dy)) in cases {
        let id = chart.add_drawing(kind, 0, points, None).unwrap();
        if kind == DrawingKind::AnchoredText {
            assert_eq!(
                chart.drawing(id).unwrap().screen_x,
                0.0,
                "placed on the left edge"
            );
        }
        chart.set_selected_drawing(Some(id));
        let before = chart.drawing(id).unwrap().clone();
        let undo_depth = chart.drawing_history.undo.len();
        let sync_revision = chart.drawing_sync_revision;
        assert!(!chart.nudge_selected_drawing(dx, dy, None), "{kind:?}");
        assert!(!chart.drawing_drag_active());
        assert_eq!(chart.drawing(id).unwrap(), &before, "{kind:?}");
        assert_eq!(chart.drawing_history.undo.len(), undo_depth, "{kind:?}");
        assert_eq!(chart.drawing_sync_revision, sync_revision, "{kind:?}");
    }
    // The newest undo step is still the last creation, and the earlier recolor survives it.
    assert!(chart.undo_drawing());
    assert_eq!(chart.drawings().len(), 3);
    assert_eq!(chart.drawing(recolored).unwrap(), &recolor);
}

#[test]
fn undo_during_a_drag_cancels_the_drag_first() {
    let mut chart = settled();
    let id = trend(&mut chart, point(2.0, 101.0), point(7.0, 103.0));
    let original = chart.drawing(id).unwrap().points.clone();
    assert!(chart.drawing_apply_options(id, r##"{"color":"#ff0000"}"##));
    let (x0, y0) = px(&chart, id, 0);
    let (x1, y1) = px(&chart, id, 1);
    let (x, y) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
    assert!(chart.drawing_drag_start_at(x, y));
    chart.drawing_drag_to(x + 40.0, y + 20.0, NO_KEYS);
    assert_ne!(chart.drawing(id).unwrap().points, original);

    assert!(chart.undo_drawing());
    assert!(!chart.drawing_drag_active());
    assert_eq!(chart.drawing(id).unwrap().points, original);
    assert_ne!(chart.drawing(id).unwrap().color, "#ff0000");
    // Late pointer samples and the pointer-up no longer mutate or record anything.
    chart.drawing_drag_to(x + 80.0, y + 40.0, NO_KEYS);
    chart.drawing_drag_end();
    assert_eq!(chart.drawing(id).unwrap().points, original);
    assert!(chart.redo_drawing());
    assert_eq!(chart.drawing(id).unwrap().color, "#ff0000");
}

#[test]
fn add_drawing_reports_invalid_options_instead_of_dropping_them() {
    let mut chart = settled();
    let points = vec![point(2.0, 101.0), point(7.0, 103.0)];
    assert!(
        chart
            .add_drawing(
                DrawingKind::TrendLine,
                0,
                points.clone(),
                Some(r#"{"width":"thick"}"#)
            )
            .is_none()
    );
    let anchors = points
        .iter()
        .copied()
        .map(DrawingAnchor::from)
        .collect::<Vec<_>>();
    let error = chart
        .add_drawing_anchors(DrawingKind::TrendLine, 0, &anchors, Some("{not json"))
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::InvalidOptions);
    let oversized = format!(r#"{{"name":"{}"}}"#, "x".repeat(MAX_DRAWING_NAME_BYTES + 1));
    let error = chart
        .add_drawing_anchors(DrawingKind::TrendLine, 0, &anchors, Some(&oversized))
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::InvalidOptions);
    let error = chart
        .add_drawing_anchors(DrawingKind::TrendLine, 0, &anchors[..1], None)
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::InvalidData);
    assert!(chart.drawings().is_empty());
    assert!(!chart.can_undo_drawing());
    let id = chart
        .add_drawing(DrawingKind::TrendLine, 0, points, Some(r#"{"width":3}"#))
        .unwrap();
    assert_eq!(id, 1, "rejected adds do not consume drawing identities");
    assert_eq!(chart.drawing(id).unwrap().width, 3.0);
}

#[test]
fn price_basis_rescale_is_one_atomic_data_basis_change() {
    let mut chart = chart_with(&spaced(BASE, DAY, 20));
    let ex_date = BASE + 10.0 * DAY;
    let before = trend(&mut chart, point(2.0, 100.0), point(8.0, 110.0));
    let spans = trend(&mut chart, point(5.0, 100.0), point(15.0, 120.0));
    let locked = chart
        .add_drawing(
            DrawingKind::HorizontalLine,
            0,
            vec![point(3.0, 90.0)],
            Some(r#"{"locked":true}"#),
        )
        .unwrap();
    let position = chart
        .add_drawing(
            DrawingKind::LongPosition,
            0,
            vec![point(9.0, 100.0), point(12.0, 110.0), point(9.0, 95.0)],
            None,
        )
        .unwrap();
    // A user edit, so the undo stack holds a pre-switch price snapshot.
    chart
        .set_drawing_anchors(
            before,
            &[point(2.0, 102.0).into(), point(8.0, 110.0).into()],
        )
        .unwrap();

    let segments = [DrawingPriceSegment {
        from_time: None,
        to_time: Some(ex_date),
        factor: 0.5,
    }];
    assert_eq!(
        chart
            .rescale_drawing_prices(&segments, Some("qfq"))
            .unwrap(),
        4
    );
    assert_eq!(prices(&chart, before), [51.0, 55.0]);
    assert_eq!(
        prices(&chart, spans),
        [50.0, 120.0],
        "only anchors before the ex-date"
    );
    assert_eq!(
        prices(&chart, locked),
        [45.0],
        "a basis change applies to locked drawings"
    );
    assert_eq!(
        prices(&chart, position),
        [50.0, 55.0, 47.5],
        "position levels share the entry's factor"
    );
    assert_eq!(chart.drawing_price_basis(), Some("qfq"));

    // No undo step was recorded, and the history is expressed in the new basis.
    assert!(chart.undo_drawing());
    assert_eq!(prices(&chart, before), [50.0, 55.0]);
    assert!(chart.redo_drawing());

    // Invalid segments fail atomically.
    let overlapping = [
        DrawingPriceSegment {
            from_time: None,
            to_time: Some(ex_date),
            factor: 2.0,
        },
        DrawingPriceSegment {
            from_time: Some(ex_date - DAY),
            to_time: None,
            factor: 2.0,
        },
    ];
    assert!(chart.rescale_drawing_prices(&overlapping, None).is_err());
    assert_eq!(prices(&chart, before), [51.0, 55.0]);

    // The batch rewrite API records exactly one undo step for many drawings.
    let changed = chart
        .set_drawings_anchors(&[
            (
                spans,
                vec![point(5.0, 60.0).into(), point(15.0, 70.0).into()],
            ),
            (locked, vec![point(3.0, 40.0).into()]),
        ])
        .unwrap();
    assert_eq!(changed, 2);
    assert!(chart.undo_drawing());
    assert_eq!(prices(&chart, spans), [50.0, 120.0]);
    assert_eq!(prices(&chart, locked), [45.0]);
    assert_eq!(prices(&chart, before), [51.0, 55.0]);
}

#[test]
fn future_rectangle_anchors_show_their_extrapolated_time_tag() {
    let mut chart = chart_with(&spaced(BASE, DAY, 20));
    chart.axis_w = 80.0;
    chart.pane_w = 720.0;
    chart.time_scale.set_width(720.0);
    chart.fit_content();
    chart.time_scale.set_right_offset(8.0);
    chart.build_frame();
    chart
        .add_drawing(
            DrawingKind::Rectangle,
            0,
            vec![point(12.0, 101.0), point(25.0, 104.0)],
            Some(r#"{"show_labels":true}"#),
        )
        .unwrap();
    chart.set_selected_drawing(None);
    let axis = chart.build_axis_frame(
        80.0,
        |text, _| text.len() as f64 * 7.0,
        |text, _| text.len() as f64 * 6.0,
    );
    let dates = axis
        .labels
        .iter()
        .map(|label| label.text.as_str())
        .collect::<Vec<_>>();
    assert!(dates.contains(&"1/13/2024"), "{dates:?}");
    assert!(
        dates.contains(&"1/26/2024"),
        "the future anchor's extrapolated date: {dates:?}"
    );
}

/// Weekday-only daily bars starting on the Monday `BASE`.
fn weekdays(count: usize) -> Vec<f64> {
    (0..)
        .map(|day| day as f64)
        .filter(|day| (*day as i64) % 7 < 5)
        .take(count)
        .map(|day| BASE + day * DAY)
        .collect()
}

/// Seven 09:30..15:30 hourly bars per trading day in `days`.
fn session_hours(days: &[f64]) -> Vec<f64> {
    days.iter()
        .flat_map(|day| (0..7).map(move |hour| day + 9.5 * HOUR + hour as f64 * HOUR))
        .collect()
}

#[test]
fn prepended_history_keeps_the_times_an_interval_switch_resolved() {
    // A daily trendline starting 120 trading days back; the hourly axis covers only the last ten.
    let days = weekdays(130);
    let mut chart = chart_with(&days);
    let id = trend(&mut chart, point(10.0, 101.0), point(129.0, 102.0));
    let saved = chart.drawing_anchors(id).unwrap()[0].time.unwrap();

    set_bars(&mut chart, &session_hours(&days[120..]));
    let on_hours = logicals(&chart, id)[0];
    assert!(
        on_hours < 0.0,
        "the old anchor sits left of the hourly data"
    );
    assert_eq!(chart.drawing_anchors(id).unwrap()[0].time, Some(saved));

    // Scrolling back prepends twenty more trading days of hourly bars (a pure translation). The
    // anchor keeps its time instead of shifting by the prepended bar count, which would ignore
    // every overnight and weekend gap inside the new history.
    set_bars(&mut chart, &session_hours(&days[100..]));
    let time = chart.drawing_anchors(id).unwrap()[0].time.unwrap();
    assert_close(time, saved, "time after the prepend");

    // Back on daily bars the anchor returns to its original day.
    set_bars(&mut chart, &days);
    assert_close(
        logicals(&chart, id)[0],
        10.0,
        "daily anchor after the round trip",
    );
    assert_close(
        logicals(&chart, id)[1],
        129.0,
        "latest anchor after the round trip",
    );
}

#[test]
fn retention_trims_keep_bar_count_shifts_for_anchors_left_of_the_data() {
    // Trims are the per-tick retention path: anchors on or left of the trimmed bars shift by the
    // trimmed row count, so streaming never makes a drawing jump across a session gap.
    let hours = session_hours(&weekdays(4));
    let mut chart = chart_with(&hours[..21]);
    assert!(chart.set_series_max_points(0, Some(21)));
    let id = trend(&mut chart, point(-3.0, 101.0), point(5.0, 102.0));
    for (row, &time) in hours[21..].iter().enumerate() {
        assert!(chart.update_series_bar(0, time, [100.0; 4]));
        let expected = [-4.0 - row as f64, 4.0 - row as f64];
        assert_eq!(
            logicals(&chart, id),
            expected,
            "after trimming {} rows",
            row + 1
        );
    }
}

#[test]
fn restoring_into_the_saved_window_is_bit_exact() {
    // Fractional anchors between gapped bars: re-deriving the logical from the saved time would
    // perturb it by an ULP (and the re-exported time with it) for most positions.
    let times = weekdays(10)
        .into_iter()
        .map(|day| day + 0.37 * HOUR)
        .collect::<Vec<_>>();
    let mut source = chart_with(&times);
    trend(&mut source, point(0.001, 101.0), point(4.003, 102.0));
    trend(&mut source, point(6.006, 101.5), point(11.25, 102.5));
    let document = source.export_state_json().unwrap();

    let mut after_data = chart_with(&times);
    after_data.import_state_json(&document).unwrap();
    assert_eq!(after_data.export_state_json().unwrap(), document);

    let mut before_data = ChartEngine::new(800.0, 500.0, 1.0);
    before_data.import_state_json(&document).unwrap();
    set_bars(&mut before_data, &times);
    assert_eq!(before_data.export_state_json().unwrap(), document);
}

#[test]
fn derived_times_stay_inside_the_persisted_value_range() {
    // An anchor far beyond the data would extrapolate to a time outside the persisted range; it
    // reports no time instead, so the exported document still imports.
    let mut chart = chart_with(&spaced(BASE, DAY, 20));
    let id = trend(&mut chart, point(2.0, 100.0), point(8.0e12, 110.0));
    let anchors = chart.drawing_anchors(id).unwrap();
    assert!(anchors[0].time.is_some());
    assert_eq!(anchors[1].time, None);
    assert_eq!(chart.anchor_time_at_logical(8.0e12), None);
    let document = chart.export_state_json().unwrap();
    let mut restored = ChartEngine::new(800.0, 500.0, 1.0);
    restored.import_state_json(&document).unwrap();
    assert_eq!(restored.drawing(id).unwrap().points[1].logical, 8.0e12);
}

#[test]
fn a_rescale_that_would_overflow_the_value_range_changes_nothing() {
    let mut chart = chart_with(&spaced(BASE, DAY, 20));
    let small = trend(&mut chart, point(2.0, 100.0), point(8.0, 110.0));
    let huge = trend(&mut chart, point(3.0, 1.0e8), point(9.0, 1.0e2));
    // A pre-edit snapshot on the undo stack is validated too.
    chart
        .set_drawing_anchors(huge, &[point(3.0, 1.0).into(), point(9.0, 1.0).into()])
        .unwrap();
    chart.set_drawing_price_basis(Some("raw")).unwrap();
    let everywhere = |factor| DrawingPriceSegment {
        from_time: None,
        to_time: None,
        factor,
    };
    let error = chart
        .rescale_drawing_prices(&[everywhere(1.0e6)], Some("hfq"))
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::InvalidData);
    assert_eq!(prices(&chart, small), [100.0, 110.0]);
    assert_eq!(prices(&chart, huge), [1.0, 1.0]);
    assert_eq!(chart.drawing_price_basis(), Some("raw"));
    assert!(chart.undo_drawing());
    assert_eq!(prices(&chart, huge), [1.0e8, 1.0e2]);
    assert!(chart.redo_drawing());

    // Many segments resolve by binary search; the one containing each anchor applies.
    let segments = (0..20)
        .map(|day| DrawingPriceSegment {
            from_time: Some(BASE + day as f64 * DAY),
            to_time: Some(BASE + (day + 1) as f64 * DAY),
            factor: 1.0 + day as f64,
        })
        .collect::<Vec<_>>();
    assert_eq!(chart.rescale_drawing_prices(&segments, None).unwrap(), 2);
    assert_eq!(prices(&chart, small), [300.0, 990.0]);
    assert_eq!(prices(&chart, huge), [4.0, 10.0]);
    let document = chart.export_state_json().unwrap();
    ChartEngine::new(800.0, 500.0, 1.0)
        .import_state_json(&document)
        .unwrap();
}

#[test]
fn non_time_bar_anchors_rescale_by_the_open_time_of_their_bar() {
    use crate::{
        AggressorSide, FootprintAggregationOptions, FootprintBarAggregation,
        FootprintSeriesOptions, FootprintTrade,
    };
    let trade = |timestamp_micros, price| FootprintTrade {
        timestamp_micros,
        price,
        volume: 1.0,
        aggressor: AggressorSide::Buy,
        bid: None,
        ask: None,
        sequence: None,
        trade_id: None,
        conditions: 0,
        session_id: Some(1),
    };
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    let footprint = chart
        .add_footprint_series(FootprintSeriesOptions {
            aggregation: FootprintAggregationOptions {
                tick_size: 1.0,
                bars: FootprintBarAggregation::Trades { trades_per_bar: 1 },
                ..FootprintAggregationOptions::default()
            },
            ..FootprintSeriesOptions::default()
        })
        .unwrap();
    chart
        .set_footprint_trades(
            footprint,
            vec![trade(2_000_001, 100.0), trade(3_000_001, 101.0)],
        )
        .unwrap();
    let id = trend(&mut chart, point(0.0, 100.0), point(1.0, 101.0));
    assert_eq!(
        chart.drawing_anchors(id).unwrap()[0].time,
        None,
        "row keys are not reported as times"
    );
    let segments = [DrawingPriceSegment {
        from_time: None,
        to_time: Some(2.5),
        factor: 0.5,
    }];
    assert_eq!(chart.rescale_drawing_prices(&segments, None).unwrap(), 1);
    assert_eq!(prices(&chart, id), [50.0, 101.0]);
}
