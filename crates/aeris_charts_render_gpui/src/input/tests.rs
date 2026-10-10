//! Interactive tests of the GPUI adapter: real GPUI event structs go through [`GpuiChartInput`]
//! into a live engine, and where the browser host normalizes the same gesture, a twin engine fed
//! the browser's formula must end in the same state.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use aeris_charts_engine::{
    ChartInputEvent, ChartRegion, DrawingId, DrawingKind, DrawingPoint, InstrumentMetadata,
    InteractionOptions, OrderId, OrderKind, OrderRole, OrderSide, OrderStatus, PriceScaleTarget,
    SeriesKind, TRADING_TOOLTIP_DWELL_MS, TradingHitKind, TradingIntentAction, TradingPriceScale,
    TradingSnapshot, WorkingOrder,
};
use aeris_charts_render::draw_list::Prim;
use gpui::{AppContext, Entity, Keystroke, MouseButton, TestAppContext, bounds, px, size};

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

/// [`input`] on the test app's executor clock, as a GPUI host constructs it.
fn input_on(cx: &mut TestAppContext) -> GpuiChartInput {
    let mut input = cx.update(|cx| GpuiChartInput::new(cx));
    input.set_canvas_bounds(bounds(
        point(px(CANVAS.0), px(CANVAS.1)),
        size(px(800.0), px(500.0)),
    ));
    input
}

/// A stand-in for the view that drew the chart, and a count of the notifies it receives.
fn chart_view(cx: &mut TestAppContext) -> (Entity<()>, Rc<Cell<usize>>, gpui::Subscription) {
    let view = cx.new(|_| ());
    let notifies = Rc::new(Cell::new(0));
    let count = notifies.clone();
    let observer = cx.update(|cx| cx.observe(&view, move |_, _| count.set(count.get() + 1)));
    (view, notifies, observer)
}

/// Move the executor clock and run whatever became due.
fn advance_ms(cx: &mut TestAppContext, ms: u64) {
    cx.executor().advance_clock(Duration::from_millis(ms));
    cx.run_until_parked();
}

/// [`chart`] with one working sell order, and the pane point of its close button.
fn trading_chart() -> (ChartEngine, (f64, f64)) {
    let mut chart = chart();
    let price = 106.0;
    chart
        .set_trading_snapshot(TradingSnapshot {
            orders: vec![working_order(price)],
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
    (chart, (close_x, y))
}

/// A hover listener's tail: the adapter call, then the refresh.
fn hover_and_refresh(
    cx: &mut TestAppContext,
    input: &mut GpuiChartInput,
    chart: &mut ChartEngine,
    at: Point<Pixels>,
) {
    input.mouse_move(chart, &hover(at));
    cx.update(|cx| input.refresh(chart, cx));
}

fn tooltip_shown(chart: &mut ChartEngine) -> bool {
    chart.build_frame().panes.iter().any(|pane| {
        pane.main
            .iter()
            .any(|prim| matches!(prim, Prim::Text { text, .. } if text == "Cancel order"))
    })
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

/// Every drawing anchor but anchored text lands on the bar slot under the pointer and on the
/// price tick nearest the pointer's price.
fn on_slot(chart: &ChartEngine, (x, y): (f64, f64)) -> (f64, f64) {
    let logical = chart.coordinate_to_logical(x).unwrap().round();
    (
        chart.logical_to_coordinate(logical).unwrap(),
        on_tick(chart, y),
    )
}

/// The y of the price tick nearest the series price under `y`: the fixture's series format
/// steps 0.01.
fn on_tick(chart: &ChartEngine, y: f64) -> f64 {
    const TICK: f64 = 0.01;
    let price = chart.series_coordinate_to_price(0, y).unwrap();
    chart
        .series_price_to_coordinate(0, (price / TICK).round() * TICK)
        .unwrap()
}

/// The whole bars a drawing body moves when the pointer goes from `from_x` to `to_x`.
fn slot_shift(chart: &ChartEngine, from_x: f64, to_x: f64) -> f64 {
    let slot = |x: f64| chart.coordinate_to_logical(x).unwrap().round();
    (slot(to_x) - slot(from_x)) * chart.bar_spacing()
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

/// A one-lot working sell limit at `price` on the right scale of pane 0.
fn working_order(price: f64) -> WorkingOrder {
    WorkingOrder {
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
    }
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

/// A press that wobbles under the shared 5 px slop on a drawing or an order line is a click: it
/// moves nothing and asks the host for nothing. A drag past the slop moves either object.
#[test]
fn a_wobbling_click_moves_no_drawing_or_order_and_a_drag_past_the_slop_does() {
    let mut chart = chart();
    let input = input();
    chart
        .set_trading_snapshot(TradingSnapshot {
            // Finer than a pixel, so any pointer offset from the line is another price.
            instrument: InstrumentMetadata {
                tick_size: Some(0.01),
                ..InstrumentMetadata::default()
            },
            // A protection order: its line is a drag target (an entry's line is a readout).
            orders: vec![WorkingOrder {
                role: OrderRole::TakeProfit,
                ..working_order(106.0)
            }],
            ..TradingSnapshot::default()
        })
        .unwrap();
    let (id, body) = trend_line_body(&mut chart);
    let points = chart.drawing(id).unwrap().points.clone();
    let anchors = [0, 1].map(|index| chart.drawing_point_to_coordinate(id, index).unwrap());
    let revision = chart.drawing_revision();

    let press = window(&chart, body.0, body.1);
    input.mouse_down(&mut chart, &down(press, 1));
    for (dx, dy) in [(2.0, -1.0), (1.0, 3.0)] {
        let wobble = window(&chart, body.0 + dx, body.1 + dy);
        input.mouse_move(&mut chart, &pressed(wobble));
        assert_eq!(chart.drawing(id).unwrap().points, points, "{dx},{dy}");
    }
    let release = window(&chart, body.0 + 1.0, body.1 + 3.0);
    input.mouse_up(&mut chart, &up(release));
    assert_eq!(chart.drawing(id).unwrap().points, points);
    assert_eq!(chart.drawing_revision(), revision);
    assert_eq!(chart.selected_drawing(), Some(id), "the press is a click");

    // Past the slop the body moves by whole bars horizontally and by price ticks vertically.
    let dx = slot_shift(&chart, body.0, body.0 + 30.0);
    drag(&input, &mut chart, body, (body.0 + 30.0, body.1 - 20.0));
    for (index, before) in anchors.into_iter().enumerate() {
        assert_near(
            chart.drawing_point_to_coordinate(id, index).unwrap(),
            (before.0 + dx, on_tick(&chart, before.1 - 20.0)),
        );
    }
    assert_eq!(chart.drawing_revision(), revision + 1);

    let line_y = chart.series_price_to_coordinate(0, 106.0).unwrap();
    let y = line_y + 4.0;
    let x = (80..chart.pane_w as usize)
        .map(|x| x as f64)
        .find(|&x| {
            chart
                .trading_hit_at(x, y)
                .is_some_and(|hit| hit.kind == TradingHitKind::OrderLine)
        })
        .expect("the order line is a drag target 4 px off its price");
    let press = window(&chart, x, y);
    input.mouse_down(&mut chart, &down(press, 1));
    for (dx, dy) in [(1.0, 1.0), (-1.0, 3.0)] {
        let wobble = window(&chart, x + dx, y + dy);
        input.mouse_move(&mut chart, &pressed(wobble));
    }
    let release = window(&chart, x - 1.0, y + 3.0);
    input.mouse_up(&mut chart, &up(release));
    assert!(chart.take_trading_intents().is_empty());
    assert_eq!(chart.trading_snapshot().orders[0].price, 106.0);

    drag(&input, &mut chart, (x, line_y), (x, line_y - 30.0));
    let intents = chart.take_trading_intents();
    assert_eq!(intents.len(), 1, "{intents:?}");
    assert_eq!(intents[0].action, TradingIntentAction::ModifyOrder);
}

/// Through real GPUI events, an armed drawing tool owns an order line: the pointer reads as the
/// tool's crosshair there, a drag along it moves no order, and two clicks place a trend line from
/// it. The order's close button keeps its pointer cursor and its click while a tool is armed.
#[test]
fn an_armed_tool_owns_order_lines_and_leaves_the_close_button_its_click() {
    let mut chart = chart();
    let input = input();
    chart
        .set_trading_snapshot(TradingSnapshot {
            // A protection order: its line drags whenever no tool is armed.
            orders: vec![WorkingOrder {
                role: OrderRole::TakeProfit,
                ..working_order(106.0)
            }],
            ..TradingSnapshot::default()
        })
        .unwrap();
    chart.build_frame();
    let line_y = chart.series_price_to_coordinate(0, 106.0).unwrap();
    let x = 80.0;
    assert_eq!(
        chart.trading_hit_at(x, line_y).map(|hit| hit.kind),
        Some(TradingHitKind::OrderLine)
    );
    let close_x = (0..=(chart.pane_w * 2.0) as usize)
        .map(|step| step as f64 / 2.0)
        .find(|&x| {
            chart
                .trading_hit_at(x, line_y)
                .is_some_and(|hit| hit.kind == TradingHitKind::CancelButton)
        })
        .expect("the order shows a close button");

    assert!(chart.set_drawing_tool(Some(DrawingKind::TrendLine), None, None));
    let on_line = window(&chart, x, line_y);
    input.mouse_move(&mut chart, &hover(on_line));
    assert_eq!(chart.input_cursor(), ChartCursor::Crosshair);
    drag(&input, &mut chart, (x, line_y), (x, line_y - 30.0));
    assert!(chart.take_trading_intents().is_empty());
    assert_eq!(chart.trading_snapshot().orders[0].price, 106.0);

    click(&input, &mut chart, (x, line_y), 1);
    click(&input, &mut chart, (x + 150.0, line_y + 40.0), 1);
    let created = chart
        .take_input_events()
        .into_iter()
        .filter(|event| matches!(event, ChartInputEvent::DrawingCreated(_)))
        .count();
    assert_eq!(created, 1, "the second click places the trend line");
    assert_eq!(chart.active_drawing_tool(), None, "one-shot tools disarm");
    assert!(chart.take_trading_intents().is_empty());
    assert_eq!(chart.trading_snapshot().orders[0].price, 106.0);
    input.mouse_move(&mut chart, &hover(on_line));
    assert_eq!(chart.input_cursor(), ChartCursor::VerticalGrab);

    assert!(chart.set_drawing_tool(Some(DrawingKind::TrendLine), None, None));
    let on_close = window(&chart, close_x, line_y);
    input.mouse_move(&mut chart, &hover(on_close));
    assert_eq!(chart.input_cursor(), ChartCursor::Pointer);
    click(&input, &mut chart, (close_x, line_y), 1);
    let intents = chart.take_trading_intents();
    assert_eq!(intents.len(), 1, "{intents:?}");
    assert_eq!(intents[0].action, TradingIntentAction::CancelOrder);
    assert_eq!(chart.active_drawing_tool(), Some(DrawingKind::TrendLine));
}

/// Through real GPUI events, an order change awaiting the host leaves the chart's trading controls
/// inert: the arrow over the moved line and over the close button, no tooltip wake, and a click
/// that cancels nothing. The host's answer, or its own update of the order when it never answers,
/// brings the grab cursor back under the resting pointer without another mouse move.
#[test]
fn an_order_change_awaiting_the_host_reads_inert_until_the_host_answers() {
    let mut chart = chart();
    let input = input();
    let take_profit = |price: f64, revision: u32| WorkingOrder {
        role: OrderRole::TakeProfit,
        revision,
        ..working_order(price)
    };
    chart
        .set_trading_snapshot(TradingSnapshot {
            orders: vec![take_profit(106.0, 1)],
            ..TradingSnapshot::default()
        })
        .unwrap();
    chart.build_frame();
    let line_y = chart.series_price_to_coordinate(0, 106.0).unwrap();
    let x = 80.0;
    let close_x = (0..=(chart.pane_w * 2.0) as usize)
        .map(|step| step as f64 / 2.0)
        .find(|&x| {
            chart
                .trading_hit_at(x, line_y)
                .is_some_and(|hit| hit.kind == TradingHitKind::CancelButton)
        })
        .expect("the order shows a close button");
    let grab = if cfg!(target_os = "windows") {
        CursorStyle::ResizeUpDown
    } else {
        CursorStyle::OpenHand
    };
    let on_line = window(&chart, x, line_y);
    input.mouse_move(&mut chart, &hover(on_line));
    assert_eq!(cursor_style(chart.input_cursor()), grab);

    drag(&input, &mut chart, (x, line_y), (x, line_y - 30.0));
    let intents = chart.take_trading_intents();
    assert_eq!(intents.len(), 1, "{intents:?}");
    assert_eq!(intents[0].action, TradingIntentAction::ModifyOrder);
    let moved_y = line_y - 30.0;
    assert_eq!(cursor_style(chart.input_cursor()), CursorStyle::Arrow);

    let on_close = window(&chart, close_x, moved_y);
    input.mouse_move(&mut chart, &hover(on_close));
    assert_eq!(cursor_style(chart.input_cursor()), CursorStyle::Arrow);
    assert_eq!(chart.input_wake_deadline_ms(), None, "no tooltip dwell");
    click(&input, &mut chart, (close_x, moved_y), 1);
    assert!(chart.take_trading_intents().is_empty());
    assert_eq!(chart.trading_snapshot().orders.len(), 1);

    let on_moved_line = window(&chart, x, moved_y);
    input.mouse_move(&mut chart, &hover(on_moved_line));
    assert_eq!(cursor_style(chart.input_cursor()), CursorStyle::Arrow);
    assert!(chart.resolve_trading_intent(intents[0].sequence, true));
    assert_eq!(cursor_style(chart.input_cursor()), grab);

    // A host that answers with its own order update instead of resolving the intent.
    drag(&input, &mut chart, (x, moved_y), (x, moved_y - 30.0));
    let second = chart.take_trading_intents();
    assert_eq!(second.len(), 1, "{second:?}");
    assert_eq!(cursor_style(chart.input_cursor()), CursorStyle::Arrow);
    chart
        .update_working_order(take_profit(second[0].price.unwrap(), 2))
        .unwrap();
    assert_eq!(cursor_style(chart.input_cursor()), grab);
    assert!(!chart.resolve_trading_intent(second[0].sequence, false));
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

/// The engine scale both hosts share is browser notches: a DOM wheel normalizes to `deltaY / 100`
/// pixels with a 32 px DOM line, and a DOM wheel-up is positive. GPUI's native lines are DOM lines
/// scaled by 25/24, so one native line is a third of a notch and the default three-line notch is
/// one. The expected notches are written out so the test cannot share a scale error with the
/// adapter.
#[test]
fn vertical_wheel_lines_and_pixels_match_the_browser_normalization() {
    let input = input();
    for (delta, browser_notches, delta_mode) in [
        (
            ScrollDelta::Lines(point(0.0, 1.0)),
            1.0 / 3.0,
            WheelDeltaMode::Line,
        ),
        (
            ScrollDelta::Pixels(point(px(0.0), px(100.0))),
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
            delta_y: browser_notches,
            delta_mode,
            ..WheelSample::default()
        }));
        assert_near(
            (chart.bar_spacing(), chart.scroll_position()),
            (twin.bar_spacing(), twin.scroll_position()),
        );
        assert!(
            chart.bar_spacing() > spacing,
            "wheel-up zooms in: {delta:?}"
        );
    }
}

/// GPUI's horizontal delta is the negation of the DOM's `deltaX` (a swipe the DOM reports as
/// scrolling right arrives negative), and the browser host passes `deltaX` through, so an unshifted
/// GPUI swipe of -100 px or one native line left pans as +1 or +1/3 browser notch.
#[test]
fn horizontal_wheel_pans_in_the_browser_direction() {
    let input = input();
    for (delta, browser_notches, delta_mode) in [
        (
            ScrollDelta::Pixels(point(px(-100.0), px(0.0))),
            1.0,
            WheelDeltaMode::Pixel,
        ),
        (
            ScrollDelta::Lines(point(-1.0, 0.0)),
            1.0 / 3.0,
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
            delta_x: browser_notches,
            delta_mode,
            ..WheelSample::default()
        }));
        assert_ne!(chart.scroll_position(), start, "{delta:?}");
        assert_near(
            (chart.scroll_position(), chart.bar_spacing()),
            (twin.scroll_position(), twin.bar_spacing()),
        );
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
        (
            ChartCursor::Move,
            if windows {
                CursorStyle::PointingHand
            } else {
                CursorStyle::OpenHand
            },
        ),
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
        (ChartCursor::HostPrimitive, CursorStyle::Crosshair),
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
            GpuiChartInput::advance(&mut chart, input.now_ms(), false),
            "the newest sample is a knot"
        );
        assert!(
            !GpuiChartInput::advance(&mut chart, input.now_ms(), false),
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

/// A GPUI host's adapter reads GPUI's executor clock, the clock its timers fire on, so the
/// tooltip dwell deadline and the wake armed for it share one time base, and
/// `TestAppContext::advance_clock` moves both.
#[gpui::test]
fn the_adapter_clock_is_the_gpui_executor_clock(cx: &mut TestAppContext) {
    let input = input_on(cx);
    let start = input.now_ms();
    advance_ms(cx, 250);
    assert_eq!(input.now_ms() - start, 250.0);

    let (mut chart, close) = trading_chart();
    let now = input.now_ms();
    let on_close = window(&chart, close.0, close.1);
    input.mouse_move(&mut chart, &hover(on_close));
    assert_eq!(
        chart.input_wake_deadline_ms(),
        Some(now + TRADING_TOOLTIP_DWELL_MS),
        "hovering the close button starts the dwell on the adapter clock"
    );

    // The dwell belongs to the hovered control: a wobble inside the button, later on the same
    // clock, keeps the deadline it already started.
    advance_ms(cx, 100);
    for dy in [1.0, -2.0] {
        let wobble = window(&chart, close.0, close.1 + dy);
        input.mouse_move(&mut chart, &hover(wobble));
        assert_eq!(
            chart.input_wake_deadline_ms(),
            Some(now + TRADING_TOOLTIP_DWELL_MS),
            "{dy}"
        );
    }
}

/// `refresh` notifies the view that drew the chart and keeps exactly one wake for the engine's
/// deadline: an unchanged deadline keeps the armed wake, a moved or cleared one replaces or drops
/// it, and a fired wake notifies the view once.
#[gpui::test]
fn refresh_notifies_the_chart_view_and_keeps_one_wake_per_deadline(cx: &mut TestAppContext) {
    let (view, notifies, _observer) = chart_view(cx);
    let (mut chart, close) = trading_chart();
    let mut input = input_on(cx);
    let on_close = window(&chart, close.0, close.1);
    let away = window(&chart, close.0, close.1 + 60.0);

    // Before a prepaint records the view there is nothing to notify and nothing to wake: the
    // first draw paints everything.
    hover_and_refresh(cx, &mut input, &mut chart, on_close);
    assert!(input.wake.is_none());
    advance_ms(cx, 1_000);
    assert_eq!(notifies.get(), 0);
    hover_and_refresh(cx, &mut input, &mut chart, away);

    input.view = Some(view.entity_id());
    hover_and_refresh(cx, &mut input, &mut chart, on_close);
    assert_eq!(
        notifies.get(),
        1,
        "refresh notifies the view that drew the chart"
    );
    let first = chart
        .input_wake_deadline_ms()
        .expect("hovering the close button starts the dwell");
    assert_eq!(input.wake.as_ref().map(|wake| wake.deadline), Some(first));

    // Holding still over the button keeps the deadline, and the refresh keeps the armed wake, so
    // the dwell still ends 450 ms after the hover rather than 450 ms after the last motion.
    advance_ms(cx, 300);
    hover_and_refresh(cx, &mut input, &mut chart, on_close);
    assert_eq!(chart.input_wake_deadline_ms(), Some(first));
    assert_eq!(notifies.get(), 2);
    advance_ms(cx, 149);
    assert_eq!(notifies.get(), 2, "the dwell has not elapsed");
    advance_ms(cx, 1);
    assert_eq!(
        notifies.get(),
        3,
        "the wake notifies the view at the deadline"
    );

    // A refresh after the wake fired, before a prepaint ticked the engine past the deadline,
    // keeps the fired wake instead of arming a second timer for the same deadline.
    cx.update(|cx| input.refresh(&chart, cx));
    advance_ms(cx, 1_000);
    assert_eq!(notifies.get(), 4, "only the refresh notified");

    // Leaving the button drops the wake; a dropped wake never fires.
    hover_and_refresh(cx, &mut input, &mut chart, away);
    assert!(input.wake.is_none());
    hover_and_refresh(cx, &mut input, &mut chart, on_close);
    let second = chart
        .input_wake_deadline_ms()
        .expect("re-entering restarts the dwell");
    assert!(second > first);
    assert_eq!(input.wake.as_ref().map(|wake| wake.deadline), Some(second));
    hover_and_refresh(cx, &mut input, &mut chart, away);
    assert!(input.wake.is_none());
    assert_eq!(notifies.get(), 7);
    advance_ms(cx, 1_000);
    assert_eq!(notifies.get(), 7, "the replaced wakes never fire");

    // Re-entering arms one wake, and exactly one notify follows its deadline.
    hover_and_refresh(cx, &mut input, &mut chart, on_close);
    assert_eq!(notifies.get(), 8);
    advance_ms(cx, 449);
    assert_eq!(notifies.get(), 8);
    advance_ms(cx, 1);
    assert_eq!(notifies.get(), 9);
    advance_ms(cx, 1_000);
    assert_eq!(notifies.get(), 9);

    // A press on the button clears the deadline, and the refresh drops the pending wake.
    hover_and_refresh(cx, &mut input, &mut chart, away);
    hover_and_refresh(cx, &mut input, &mut chart, on_close);
    assert!(input.wake.is_some());
    input.mouse_down(&mut chart, &down(on_close, 1));
    cx.update(|cx| input.refresh(&chart, cx));
    assert_eq!(chart.input_wake_deadline_ms(), None);
    assert!(input.wake.is_none());
}

/// A timer can elapse a hair before the `f64` deadline it was armed for (platform timer slack,
/// Windows' 100 ns timer truncation, or `f64` rounding of the adapter clock). The prepaint after a
/// fired wake still reaches the deadline, so the tooltip cannot stall until the next input.
#[gpui::test]
fn a_fired_wake_reaches_its_deadline_even_when_the_clock_reads_just_before_it(
    cx: &mut TestAppContext,
) {
    let (view, _notifies, _observer) = chart_view(cx);
    let (mut chart, close) = trading_chart();
    let mut input = input_on(cx);
    input.view = Some(view.entity_id());
    let on_close = window(&chart, close.0, close.1);
    input.mouse_move(&mut chart, &hover(on_close));
    let deadline = chart
        .input_wake_deadline_ms()
        .expect("hovering the close button starts the dwell");
    cx.update(|cx| input.refresh(&chart, cx));
    cx.executor()
        .advance_clock(Duration::from_nanos(449_999_500));
    assert!(input.now_ms() < deadline, "half a microsecond early");

    // While the wake is pending the deadline is still ahead.
    let elapsed = input
        .wake
        .as_ref()
        .expect("the dwell is armed")
        .elapsed
        .clone();
    assert!(!elapsed.get());
    assert!(!GpuiChartInput::advance(&mut chart, input.tick_ms(), false));
    assert!(!tooltip_shown(&mut chart));
    assert_eq!(chart.input_wake_deadline_ms(), Some(deadline));

    // The timer elapsing is the evidence that the deadline was reached.
    elapsed.set(true);
    assert_eq!(input.tick_ms(), deadline);
    assert!(
        GpuiChartInput::advance(&mut chart, input.tick_ms(), false),
        "the tick at the deadline arms the tooltip"
    );
    assert!(tooltip_shown(&mut chart));
    assert_eq!(chart.input_wake_deadline_ms(), None);
    cx.update(|cx| input.arm_wake(&chart, cx));
    assert!(input.wake.is_none(), "the spent wake is dropped");
}

/// The wake task holds only the view's id and belongs to the adapter: dropping the adapter (with
/// the view that owns it) cancels the timer, so nothing is notified afterwards.
#[gpui::test]
fn dropping_the_adapter_cancels_its_wake(cx: &mut TestAppContext) {
    let (view, notifies, _observer) = chart_view(cx);
    let (mut chart, close) = trading_chart();
    let mut input = input_on(cx);
    input.view = Some(view.entity_id());
    let on_close = window(&chart, close.0, close.1);
    input.mouse_move(&mut chart, &hover(on_close));
    cx.update(|cx| input.refresh(&chart, cx));
    assert_eq!(notifies.get(), 1);
    assert!(input.wake.is_some());
    drop(input);
    advance_ms(cx, 1_000);
    assert_eq!(notifies.get(), 1, "the cancelled wake never fires");
}

/// The last-price pulse runs on the adapter clock for line and area series (browser parity), and
/// every pulse step reports a change so the host rebuilds the overlay.
#[test]
fn the_pulse_clock_follows_the_tick_on_line_and_area_series_only() {
    let mut chart = chart();
    let id = chart.series[0].id;
    assert!(!chart.last_price_pulse_active(), "candles have no pulse");
    assert!(!GpuiChartInput::advance(&mut chart, 100.0, false));
    assert_eq!(chart.animation_time, 0.0);

    chart.convert_series_kind(id, SeriesKind::Line);
    assert!(chart.last_price_pulse_active());
    assert!(GpuiChartInput::advance(&mut chart, 250.0, false));
    assert_eq!(chart.animation_time, 250.0);
    chart.convert_series_kind(id, SeriesKind::Area);
    assert!(GpuiChartInput::advance(&mut chart, 300.0, false));
    assert_eq!(chart.animation_time, 300.0);

    assert!(chart.set_series_last_price_animation(id, false));
    assert!(
        !GpuiChartInput::advance(&mut chart, 400.0, false),
        "a host opt-out stops the pulse"
    );
    assert_eq!(chart.animation_time, 300.0);
}

/// `App::reduce_motion` removes the pulse (GPUI asks decorative motion to stop under it): the
/// changed preference is one change, so the host redraws without the ring, and later prepaints
/// leave the pulse clock alone until the preference clears.
#[test]
fn reduced_motion_stops_the_pulse_clock() {
    let mut chart = chart();
    let id = chart.series[0].id;
    chart.convert_series_kind(id, SeriesKind::Line);
    assert!(GpuiChartInput::advance(&mut chart, 100.0, false));
    assert!(
        GpuiChartInput::advance(&mut chart, 200.0, true),
        "the changed preference is a change"
    );
    assert!(!chart.last_price_pulse_active());
    assert_eq!(chart.animation_time, 100.0);
    assert!(!GpuiChartInput::advance(&mut chart, 300.0, true));
    assert_eq!(chart.animation_time, 100.0);
    assert!(GpuiChartInput::advance(&mut chart, 400.0, false));
    assert_eq!(chart.animation_time, 400.0);
}

/// A live-bar glide advances through the same clock step as the pulse: the first tick after the
/// in-place replace only stamps the glide's clock and changes nothing, later ticks move the drawn
/// close and report the change, and the frame predicate holds until the glide settles.
#[test]
fn the_live_bar_glide_advances_on_the_tick_and_keeps_frames_until_it_settles() {
    let mut chart = chart();
    assert!(chart.series_apply_options_json(0, r#"{"live_bar_easing_ms":200}"#));
    assert!(
        !chart.animation_frame_requested(),
        "an idle chart requests nothing"
    );
    let last = BARS - 1;
    let time = 1_000.0 + last as f64 * 60.0;
    let open = 100.0 + (last % 7) as f64;
    assert!(chart.update_series_bar(0, time, [open, open + 9.0, open - 3.0, open + 8.0]));
    assert!(chart.live_bar_easing_active());
    assert!(
        chart.animation_frame_requested(),
        "an unsettled glide keeps frames coming"
    );
    assert!(
        !GpuiChartInput::advance(&mut chart, 100.0, false),
        "the first tick only stamps the glide's clock"
    );
    assert_eq!(
        chart.animation_time, 0.0,
        "candles have no pulse, a stamp moves no clock"
    );
    assert!(chart.animation_frame_requested());
    assert!(
        GpuiChartInput::advance(&mut chart, 150.0, false),
        "a later tick moves the drawn close"
    );
    assert_eq!(
        chart.animation_time, 150.0,
        "a glide that advanced moves the animation clock"
    );
    assert!(chart.animation_frame_requested());
    assert!(
        GpuiChartInput::advance(&mut chart, 150.0 + 6.0 * 200.0, false),
        "the tick that settles the glide is a change"
    );
    assert!(!chart.live_bar_easing_active());
    assert!(
        !chart.animation_frame_requested(),
        "a settled glide requests nothing"
    );
    assert!(!GpuiChartInput::advance(&mut chart, 2_000.0, false));
    assert_eq!(chart.animation_time, 150.0 + 6.0 * 200.0);
}

/// While a countdown row shows, the one wake also fires on each whole second of the adapter
/// clock; an earlier engine deadline takes the wake first, and it stays a single wake.
#[gpui::test]
fn the_countdown_second_shares_the_one_wake_with_the_engine_deadline(cx: &mut TestAppContext) {
    let (view, notifies, _observer) = chart_view(cx);
    let (mut chart, close) = trading_chart();
    let mut input = input_on(cx);
    input.view = Some(view.entity_id());
    input.countdown = true;
    let deadline = |input: &GpuiChartInput| input.wake.as_ref().map(|wake| wake.deadline);

    advance_ms(cx, 250);
    cx.update(|cx| input.refresh(&chart, cx));
    assert_eq!(deadline(&input), Some(1_000.0), "the next whole second");
    let on_close = window(&chart, close.0, close.1);
    hover_and_refresh(cx, &mut input, &mut chart, on_close);
    assert_eq!(
        deadline(&input),
        Some(250.0 + TRADING_TOOLTIP_DWELL_MS),
        "the earlier dwell takes the wake"
    );
    let notified = notifies.get();
    advance_ms(cx, TRADING_TOOLTIP_DWELL_MS as u64);
    assert_eq!(notifies.get(), notified + 1, "the dwell fires");

    // The prepaint that follows ticks the dwell away and re-arms for the countdown second.
    assert!(GpuiChartInput::advance(&mut chart, input.tick_ms(), false));
    cx.update(|cx| input.arm_wake(&chart, cx));
    assert_eq!(deadline(&input), Some(1_000.0));
    advance_ms(cx, 300);
    assert_eq!(notifies.get(), notified + 2, "the countdown second fires");
    cx.update(|cx| input.arm_wake(&chart, cx));
    assert_eq!(deadline(&input), Some(2_000.0), "a fired second moves on");

    input.countdown = false;
    cx.update(|cx| input.arm_wake(&chart, cx));
    assert!(
        input.wake.is_none(),
        "no countdown and no deadline, no wake"
    );
    advance_ms(cx, 5_000);
    assert_eq!(notifies.get(), notified + 2);
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

/// Through real GPUI events, the drawing being placed owns Delete and Backspace: after a trend
/// line's first click they step back that click, and the drawing selected before the tool was
/// picked survives every press, key auto-repeat included. Two more clicks then place the line.
#[test]
fn delete_and_backspace_mid_placement_step_back_only_the_drawing_being_placed() {
    let mut chart = chart();
    let input = input();
    let (old, body) = trend_line_body(&mut chart);
    click(&input, &mut chart, body, 1);
    assert_eq!(chart.selected_drawing(), Some(old));
    chart.take_input_events();
    // `GpuiChartInput::key_down` with no typing session open, minus the `App` it needs only for
    // the clipboard.
    let press = |chart: &mut ChartEngine, name: &str, is_held: bool| {
        let event = KeyDownEvent {
            is_held,
            ..key(name, None, Modifiers::default())
        };
        let key = chart_key_down(&event).expect("a bound chart key");
        chart.input_key_down(
            key,
            input_modifiers(&event.keystroke.modifiers),
            event.is_held,
            input.now_ms(),
        )
    };

    assert!(chart.set_drawing_tool(Some(DrawingKind::TrendLine), None, None));
    click(&input, &mut chart, (200.0, 300.0), 1);
    assert!(chart.drawing_create_active());
    assert!(press(&mut chart, "delete", false));
    assert!(press(&mut chart, "backspace", false));
    assert!(press(&mut chart, "backspace", true));
    assert!(
        chart.drawing(old).is_some(),
        "the selected drawing survives"
    );
    assert_eq!(chart.selected_drawing(), Some(old));
    assert!(chart.take_input_events().is_empty());
    assert_eq!(chart.active_drawing_tool(), Some(DrawingKind::TrendLine));

    for at in [(300.0, 120.0), (450.0, 200.0)] {
        click(&input, &mut chart, at, 1);
    }
    let [ChartInputEvent::DrawingCreated(id)] = chart.take_input_events()[..] else {
        panic!("the second click after the step back places the line");
    };
    assert_near(
        chart.drawing_point_to_coordinate(id, 0).unwrap(),
        on_slot(&chart, (300.0, 120.0)),
    );
    assert_near(
        chart.drawing_point_to_coordinate(id, 1).unwrap(),
        on_slot(&chart, (450.0, 200.0)),
    );
    assert!(chart.drawing(old).is_some());
}

/// Through real GPUI events, an armed click-placed tool places by click only: a press-drag-release
/// places nothing, pans nothing, and leaves the tool armed, and two clicks then place the line.
#[test]
fn an_armed_trend_line_places_by_click_and_a_drag_places_nothing() {
    let mut chart = chart();
    let input = input();
    assert!(chart.set_drawing_tool(Some(DrawingKind::TrendLine), None, None));
    let scroll = chart.scroll_position();

    drag(&input, &mut chart, (150.0, 150.0), (450.0, 300.0));
    assert!(!chart.drawing_create_active(), "no anchor was placed");
    assert!(chart.drawings().is_empty());
    assert!(chart.take_input_events().is_empty());
    assert_eq!(chart.scroll_position(), scroll, "the drag did not pan");
    assert_eq!(chart.active_drawing_tool(), Some(DrawingKind::TrendLine));

    click(&input, &mut chart, (150.0, 150.0), 1);
    click(&input, &mut chart, (450.0, 300.0), 1);
    let [ChartInputEvent::DrawingCreated(id)] = chart.take_input_events()[..] else {
        panic!("two clicks place the line");
    };
    assert_near(
        chart.drawing_point_to_coordinate(id, 0).unwrap(),
        on_slot(&chart, (150.0, 150.0)),
    );
    assert_near(
        chart.drawing_point_to_coordinate(id, 1).unwrap(),
        on_slot(&chart, (450.0, 300.0)),
    );
}
