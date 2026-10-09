//! Trading interactions driven through the input controller: order-line drags, the close and
//! protection controls, execution arrows, and the close-tooltip dwell the host wakes it for.

use super::tests::*;
use super::*;
use crate::trading::tests::{cancel_center, id, order, position, protection_button_x};
use crate::{
    ExecutionId, ExecutionKind, ExecutionMarkerShape, InstrumentMetadata, OrderId, OrderRole,
    OrderSide, PositionId, PositionSide, TradingExecution, TradingHitKind, TradingIntent,
    TradingIntentAction, TradingObjectId, TradingPosition, TradingPriceScale, TradingSnapshot,
    WorkingOrder,
};
use aeris_charts_render::draw_list::Prim;

const TICK: f64 = 0.25;
/// A tick of well under one pixel on the fixture (about 30 px per price unit), so any pointer
/// offset from a line lands on a different snapped price.
const FINE_TICK: f64 = 0.01;

/// The interactive fixture carrying a trading snapshot with a 0.25 instrument tick.
fn market(positions: Vec<TradingPosition>, orders: Vec<WorkingOrder>) -> ChartEngine {
    market_with_tick(TICK, positions, orders)
}

fn market_with_tick(
    tick: f64,
    positions: Vec<TradingPosition>,
    orders: Vec<WorkingOrder>,
) -> ChartEngine {
    let mut chart = chart();
    chart
        .set_trading_snapshot(TradingSnapshot {
            instrument: InstrumentMetadata {
                tick_size: Some(tick),
                ..InstrumentMetadata::default()
            },
            positions,
            orders,
            ..TradingSnapshot::default()
        })
        .unwrap();
    chart.build_frame();
    chart
}

/// A take profit at 103 protecting the long position at 101.
fn take_profit_market() -> ChartEngine {
    market(
        vec![position(PositionSide::Long)],
        vec![order("tp-1", OrderRole::TakeProfit, 103.0)],
    )
}

/// A working entry order at 104; its close cell cancels it.
fn working_order_market() -> ChartEngine {
    market(
        Vec::new(),
        vec![order("order-1", OrderRole::Working, 104.0)],
    )
}

fn price_y(chart: &ChartEngine, price: f64) -> f64 {
    chart
        .trading_price_coordinate(0, TradingPriceScale::Right, price)
        .unwrap()
}

/// The tick-snapped price an order dragged to `y` previews, on the chart's instrument tick.
fn snapped_price_at(chart: &ChartEngine, y: f64) -> f64 {
    let tick = chart.trading_state.instrument.tick_size.unwrap();
    let raw = chart
        .trading_coordinate_to_price(0, TradingPriceScale::Right, y)
        .unwrap();
    (raw / tick).round() * tick
}

fn device_at(device: InputDevice, x: f64, y: f64) -> PointerInput {
    PointerInput { device, ..at(x, y) }
}

fn only_intent(chart: &mut ChartEngine) -> TradingIntent {
    let mut intents = chart.take_trading_intents();
    assert_eq!(intents.len(), 1, "exactly one intent: {intents:?}");
    intents.pop().unwrap()
}

/// A host frame loop: prepares frames into retained buffers like a native host does.
#[derive(Default)]
struct Host {
    frame: ChartFrame,
    axis: Vec<Prim>,
}

impl Host {
    /// Prepare the next frame; returns whether it was rebuilt.
    fn prepare(&mut self, chart: &mut ChartEngine) -> bool {
        chart
            .prepare_financial_frame_with_measure(
                FinancialFrameRequest {
                    width: 800.0,
                    height: 500.0,
                    dpr: 1.0,
                    force_layout: false,
                    allow_axis_shrink: false,
                    force_frame: false,
                    force_axis: false,
                    layout_only: false,
                    fit_content: false,
                    frame: &mut self.frame,
                    axis_frame: None,
                    axis_primitives: Some(&mut self.axis),
                },
                |_, _| 48.0,
                |_, _| 48.0,
            )
            .frame_built
    }

    fn shows(&self, label: &str) -> bool {
        self.frame
            .panes
            .iter()
            .flat_map(|pane| &pane.main)
            .any(|primitive| matches!(primitive, Prim::Text { text, .. } if text == label))
    }
}

#[test]
fn hovering_and_dragging_an_order_line_previews_snapped_prices_and_releases_one_modify() {
    let mut chart = take_profit_market();
    let x = chart.trading_marker_start() + 20.0;
    let y0 = price_y(&chart, 103.0);
    let scroll = chart.scroll_position();

    chart.input_pointer_move(at(x, y0), false);
    assert_eq!(chart.input_cursor(), ChartCursor::VerticalGrab);

    chart.input_pointer_down(at(x, y0), 1);
    for y in [y0 - 15.0, y0 - 30.0] {
        chart.input_pointer_move(at(x, y), true);
        assert_eq!(
            chart.trading_preview().map(|preview| preview.price),
            Some(snapped_price_at(&chart, y))
        );
    }
    let target = snapped_price_at(&chart, y0 - 30.0);
    assert!(target > 103.0, "{target}");
    assert_eq!(chart.input_cursor(), ChartCursor::VerticalGrabbing);
    assert_eq!(
        chart.trading_snapshot().orders[0].price,
        103.0,
        "the order moves only on release"
    );
    assert!(chart.take_trading_intents().is_empty());

    chart.input_pointer_up(at(x, y0 - 30.0));
    let intent = only_intent(&mut chart);
    assert_eq!(intent.action, TradingIntentAction::ModifyOrder);
    assert_eq!(intent.order_id.as_ref().map(OrderId::as_str), Some("tp-1"));
    assert_eq!(intent.price, Some(target));
    assert!(chart.take_trading_intents().is_empty());
    assert_eq!(chart.trading_preview(), None);
    assert_eq!(chart.trading_snapshot().orders[0].price, target);
    assert_eq!(chart.scroll_position(), scroll, "the line owns the drag");
    // While the host has not answered, the moved line under the resting pointer is inert; once
    // the host accepts, it is grabbable again at once, without another pointer motion.
    assert_eq!(chart.input_cursor(), ChartCursor::Default);
    assert!(chart.resolve_trading_intent(intent.sequence, true));
    assert_eq!(chart.input_cursor(), ChartCursor::VerticalGrab);
    chart.input_pointer_move(at(x, y0 - 30.0), false);
    assert_eq!(chart.input_cursor(), ChartCursor::VerticalGrab);
}

#[test]
fn a_released_order_drag_holds_the_line_until_the_host_resolves_it() {
    let mut chart = take_profit_market();
    let x = chart.trading_marker_start() + 20.0;
    let y0 = price_y(&chart, 103.0);
    drag(&mut chart, (x, y0), (x, y0 - 30.0));
    let first = only_intent(&mut chart);
    let moved = first.price.unwrap();
    let y1 = price_y(&chart, moved);
    let scroll = chart.scroll_position();

    // Until the host answers, the moved line reads as inert, and a press on it neither drags it
    // nor pans the chart.
    chart.input_pointer_move(at(x, y1), false);
    assert_eq!(chart.input_cursor(), ChartCursor::Default);
    chart.input_pointer_down(at(x, y1), 1);
    assert_eq!(chart.input_cursor(), ChartCursor::Default);
    chart.input_pointer_move(at(x, y1 - 15.0), true);
    chart.input_pointer_move(at(x, y1 - 30.0), true);
    assert_eq!(chart.trading_preview(), None);
    assert_eq!(chart.input_cursor(), ChartCursor::Default);
    chart.input_pointer_up(at(x, y1 - 30.0));
    assert!(chart.take_trading_intents().is_empty());
    assert_eq!(chart.trading_snapshot().orders[0].price, moved);
    assert_eq!(chart.scroll_position(), scroll);

    assert!(chart.resolve_trading_intent(first.sequence, true));
    let target = snapped_price_at(&chart, y1 - 30.0);
    drag(&mut chart, (x, y1), (x, y1 - 30.0));
    let second = only_intent(&mut chart);
    assert_eq!(second.action, TradingIntentAction::ModifyOrder);
    assert_eq!(second.price, Some(target));
    assert_ne!(second.sequence, first.sequence);
}

/// The trading layer the next frame paints: lines, markers, buttons, their hover and press
/// surfaces, endpoint nodes, and action tooltips.
fn trading_layer(chart: &mut ChartEngine) -> Vec<Prim> {
    let frame = chart.build_frame();
    let segments = chart.frame_pane_segments(0).unwrap();
    frame.panes[0].main[segments.drawings_end..segments.trading_end].to_vec()
}

/// Whether the next frame marks the trading line at `y` as hovered: an endpoint node in the line's
/// own color. The bracket connector a protection move raises puts nodes in the position color at
/// the same spot, so those do not count.
fn line_highlighted(chart: &mut ChartEngine, y: f64) -> bool {
    let end_x = chart.pane_w - 8.0;
    let connector = chart.trading_state.style.position;
    trading_layer(chart).iter().any(|primitive| {
        matches!(primitive, Prim::Circle { cx, cy, stroke, .. }
            if (f64::from(*cx) - end_x).abs() < 0.5
                && (f64::from(*cy) - y).abs() < 1.0
                && *stroke != connector)
    })
}

/// The trading layer with no pointer over the chart, the baseline an inert control's hover and
/// press must leave untouched.
fn resting_trading_layer(chart: &mut ChartEngine) -> Vec<Prim> {
    chart.input_pointer_leave();
    trading_layer(chart)
}

/// The cursor the controller holds is the one the next motion at the same spot resolves, so a
/// host presenting `input_cursor()` after its own call shows the right cursor without motion.
fn assert_cursor_is_current(chart: &mut ChartEngine, context: &str) {
    let held = chart.input_cursor();
    let (x, y) = chart.input.pointer.expect("a resting pointer");
    chart.input_pointer_move(at(x, y), false);
    assert_eq!(held, chart.input_cursor(), "{context}");
}

/// A long position at 101 protected by the take profit at 103, beside one buy fill on bar 30.
fn take_profit_market_with_fill() -> ChartEngine {
    let mut chart = chart();
    chart
        .set_trading_snapshot(TradingSnapshot {
            instrument: InstrumentMetadata {
                tick_size: Some(TICK),
                ..InstrumentMetadata::default()
            },
            positions: vec![position(PositionSide::Long)],
            orders: vec![order("tp-1", OrderRole::TakeProfit, 103.0)],
            executions: vec![buy_fill_at_bar_30()],
            ..TradingSnapshot::default()
        })
        .unwrap();
    chart.build_frame();
    chart
}

/// Drag the take profit at 103 by 30 px and release it without a host answer. Returns the intent
/// and the line body x; the pointer rests on the moved line.
fn move_take_profit_unanswered(chart: &mut ChartEngine) -> (TradingIntent, f64) {
    let x = chart.trading_marker_start() + 20.0;
    let y0 = price_y(chart, 103.0);
    drag(chart, (x, y0), (x, y0 - 30.0));
    let intent = only_intent(chart);
    assert_eq!(intent.action, TradingIntentAction::ModifyOrder);
    (intent, x)
}

/// While a released change awaits the host, every trading control on the chart is inert, not only
/// the moved line: the arrow cursor, no hover highlight, pressed surface, or close tooltip, and a
/// press that neither drags, pans, nor requests anything. An execution arrow still answers, since
/// revealing a fill requests nothing. The host's answer frees the controls under a resting pointer
/// at once, without another pointer motion.
#[test]
fn a_request_awaiting_the_host_leaves_every_control_inert_until_it_answers() {
    let mut chart = take_profit_market_with_fill();
    let position_id = TradingObjectId::Position(id("position-1", PositionId::new));
    let x = chart.trading_marker_start() + 20.0;
    let y0 = price_y(&chart, 103.0);
    chart.input_pointer_move(at(x, y0), false);
    assert_eq!(chart.input_cursor(), ChartCursor::VerticalGrab);
    assert!(line_highlighted(&mut chart, y0), "an idle line highlights");

    let (intent, x) = move_take_profit_unanswered(&mut chart);
    let y1 = price_y(&chart, intent.price.unwrap());
    let scroll = chart.scroll_position();
    assert_eq!(chart.input_cursor(), ChartCursor::Default, "inert at once");
    assert_eq!(chart.trading_cursor_at(x, y1), None);
    assert!(!line_highlighted(&mut chart, y1));

    let rest = resting_trading_layer(&mut chart);
    let (order_close_x, order_close_y) =
        cancel_center(&mut chart, TradingObjectId::Order(id("tp-1", OrderId::new)));
    let position_y = price_y(&chart, 101.0);
    let sl_x = protection_button_x(&chart, position_y, TradingHitKind::StopLossButton);
    let (close_x, close_y) = cancel_center(&mut chart, position_id);
    for (cx, cy, name) in [
        (x, y1, "moved line"),
        (order_close_x, order_close_y, "order close"),
        (sl_x, position_y, "SL button"),
        (close_x, close_y, "position close"),
    ] {
        chart.input_pointer_move(at_ms(cx, cy, 1_000.0), false);
        assert_eq!(chart.input_cursor(), ChartCursor::Default, "{name}");
        assert_eq!(chart.trading_cursor_at(cx, cy), None, "{name}");
        assert_eq!(
            chart.input_wake_deadline_ms(),
            None,
            "{name}: no tooltip dwell"
        );
        assert!(
            !chart.input_tick(1_000.0 + TRADING_TOOLTIP_DWELL_MS),
            "{name}"
        );
        assert_eq!(
            trading_layer(&mut chart),
            rest,
            "{name}: no hover highlight"
        );

        // A press there is absorbed: no pressed surface, no preview, no pan, no request.
        chart.input_pointer_down(at_ms(cx, cy, 1_100.0), 1);
        assert_eq!(chart.input_cursor(), ChartCursor::Default, "{name}");
        assert_eq!(
            trading_layer(&mut chart),
            rest,
            "{name}: no pressed surface"
        );
        chart.input_pointer_move(at_ms(cx - 30.0, cy - 30.0, 1_150.0), true);
        assert_eq!(chart.trading_preview(), None, "{name}");
        assert_eq!(chart.input_cursor(), ChartCursor::Default, "{name}");
        chart.input_pointer_up(at_ms(cx - 30.0, cy - 30.0, 1_200.0));
        click(&mut chart, cx, cy);
        assert!(chart.take_trading_intents().is_empty(), "{name}");
        assert_eq!(chart.scroll_position(), scroll, "{name}");
        assert_eq!(chart.trading_snapshot().positions.len(), 1, "{name}");
        assert_eq!(chart.trading_snapshot().orders.len(), 1, "{name}");
    }

    // Revealing a fill requests nothing, so the execution arrow keeps answering.
    let layout = chart.trading_execution_layout(0);
    let (fill_x, fill_y) = (layout.marks[0].x, layout.marks[0].y);
    chart.input_pointer_move(at(fill_x, fill_y), false);
    assert_eq!(chart.input_cursor(), ChartCursor::Pointer);
    assert_ne!(trading_layer(&mut chart), rest, "the fill detail shows");

    // The host accepts while the pointer rests on the moved line: it grabs and highlights again.
    chart.input_pointer_move(at(x, y1), false);
    assert!(!line_highlighted(&mut chart, y1));
    assert!(chart.resolve_trading_intent(intent.sequence, true));
    assert_eq!(chart.input_cursor(), ChartCursor::VerticalGrab);
    assert!(
        line_highlighted(&mut chart, y1),
        "the line highlights again"
    );
    chart.input_pointer_move(at_ms(close_x, close_y, 2_000.0), false);
    assert_eq!(chart.input_cursor(), ChartCursor::Pointer);
    assert_eq!(
        chart.input_wake_deadline_ms(),
        Some(2_000.0 + TRADING_TOOLTIP_DWELL_MS)
    );
}

/// A rejected move puts the line back; the resting pointer it left reads what is under it now,
/// and the line grabs again where it returned.
#[test]
fn a_rejected_move_frees_the_line_where_it_returns() {
    let mut chart = take_profit_market();
    let (intent, x) = move_take_profit_unanswered(&mut chart);
    assert_eq!(chart.input_cursor(), ChartCursor::Default);
    assert!(chart.resolve_trading_intent(intent.sequence, false));
    assert_eq!(chart.trading_snapshot().orders[0].price, 103.0);
    assert_ne!(chart.input_cursor(), ChartCursor::Default);
    assert_cursor_is_current(&mut chart, "after the rejection");
    chart.input_pointer_move(at(x, price_y(&chart, 103.0)), false);
    assert_eq!(chart.input_cursor(), ChartCursor::VerticalGrab);
}

/// The host's own word settles a request even when it never answers the intent: an update of the
/// moved order, or a snapshot. The chart then shows the host's state, so a late rejection has
/// nothing left to roll back, and the line drags again at once. An update of another object (a
/// P&L tick on the position) is no answer and leaves the request waiting.
#[test]
fn the_hosts_update_of_the_moved_order_settles_its_request() {
    type Answer = fn(&mut ChartEngine, f64);
    let answers: [(&str, Answer); 2] = [
        ("order update", |chart, price| {
            let mut confirmed = order("tp-1", OrderRole::TakeProfit, price);
            confirmed.revision = 2;
            chart.update_working_order(confirmed).unwrap();
        }),
        ("snapshot", |chart, _| {
            let snapshot = chart.trading_snapshot();
            chart.set_trading_snapshot(snapshot).unwrap();
        }),
    ];
    for (name, answer) in answers {
        let mut chart = take_profit_market();
        let (intent, x) = move_take_profit_unanswered(&mut chart);
        let moved = intent.price.unwrap();
        let mut tick = position(PositionSide::Long);
        tick.display_pnl = Some(24.0);
        chart.update_trading_position(tick).unwrap();
        assert_eq!(
            chart.input_cursor(),
            ChartCursor::Default,
            "{name}: still waiting"
        );

        answer(&mut chart, moved);
        assert_eq!(chart.input_cursor(), ChartCursor::VerticalGrab, "{name}");
        assert!(
            !chart.resolve_trading_intent(intent.sequence, false),
            "{name}"
        );
        assert_eq!(chart.trading_snapshot().orders[0].price, moved, "{name}");

        let y1 = price_y(&chart, moved);
        drag(&mut chart, (x, y1), (x, y1 - 30.0));
        assert_eq!(
            only_intent(&mut chart).action,
            TradingIntentAction::ModifyOrder,
            "{name}"
        );
    }
}

/// A close answered by the host's own state settles too: the host's removal of the cancelled order
/// or the closed position, even though the chart already removed it. A plain re-report is no
/// answer. A P&L tick on the position being closed, or a snapshot that still holds it, puts it
/// back on the chart, but its close cell stays inert, so no second close can go out while the
/// first is in flight (a host that closes by sending an opposing order would otherwise flip the
/// position). A late rejection keeps the host's one copy.
#[test]
fn the_hosts_update_of_a_closed_object_settles_its_request() {
    let mut chart = take_profit_market();
    let order_id = id("tp-1", OrderId::new);
    let position_id = id("position-1", PositionId::new);
    let (close_x, close_y) =
        cancel_center(&mut chart, TradingObjectId::Position(position_id.clone()));
    let (order_x, order_y) = cancel_center(&mut chart, TradingObjectId::Order(order_id.clone()));
    click(&mut chart, order_x, order_y);
    let cancel = only_intent(&mut chart);
    assert_eq!(cancel.action, TradingIntentAction::CancelOrder);
    chart.input_pointer_move(at(close_x, close_y), false);
    assert_eq!(chart.input_cursor(), ChartCursor::Default);
    // The order is already gone from the chart; the host's removal still answers the request.
    assert!(!chart.remove_working_order(&order_id));
    assert_eq!(chart.input_cursor(), ChartCursor::Pointer);
    assert!(!chart.resolve_trading_intent(cancel.sequence, false));
    assert!(chart.trading_snapshot().orders.is_empty());

    click(&mut chart, close_x, close_y);
    let close = only_intent(&mut chart);
    assert_eq!(close.action, TradingIntentAction::ClosePosition);
    assert!(chart.trading_snapshot().positions.is_empty());
    let mut tick = position(PositionSide::Long);
    tick.display_pnl = Some(24.0);
    chart.update_trading_position(tick.clone()).unwrap();
    assert_eq!(
        chart.trading_snapshot().positions.len(),
        1,
        "shown as reported"
    );
    assert_eq!(
        chart.input_cursor(),
        ChartCursor::Default,
        "a P&L tick keeps the close cell inert"
    );
    click(&mut chart, close_x, close_y);
    assert!(chart.take_trading_intents().is_empty(), "no second close");
    let snapshot = chart.trading_snapshot();
    chart.set_trading_snapshot(snapshot).unwrap();
    assert_eq!(
        chart.input_cursor(),
        ChartCursor::Default,
        "a snapshot that still holds the position is no answer"
    );
    assert!(chart.resolve_trading_intent(close.sequence, false));
    assert_eq!(
        chart.trading_snapshot().positions.len(),
        1,
        "a rejection inserts no second copy"
    );
    assert_eq!(chart.input_cursor(), ChartCursor::Pointer);

    // Closed again, a tick still waits and the host's removal answers.
    click(&mut chart, close_x, close_y);
    let close = only_intent(&mut chart);
    chart.update_trading_position(tick).unwrap();
    assert_eq!(chart.input_cursor(), ChartCursor::Default);
    assert!(chart.remove_trading_position(&position_id));
    assert!(!chart.resolve_trading_intent(close.sequence, false));
    assert!(chart.trading_snapshot().positions.is_empty());
}

/// A position whose side the host flips has been closed through, so the flip answers the close,
/// and the new position's close cell acts.
#[test]
fn a_position_flip_answers_its_close() {
    let mut chart = take_profit_market();
    let position_id = TradingObjectId::Position(id("position-1", PositionId::new));
    let (close_x, close_y) = cancel_center(&mut chart, position_id);
    click(&mut chart, close_x, close_y);
    let close = only_intent(&mut chart);
    assert_eq!(close.action, TradingIntentAction::ClosePosition);
    chart
        .update_trading_position(position(PositionSide::Short))
        .unwrap();
    assert!(!chart.resolve_trading_intent(close.sequence, false));
    assert_eq!(chart.input_cursor(), ChartCursor::Pointer);
}

/// A cancel or a move is answered only by order state that shows the broker acted on it. An
/// update still at the revision the request saw (one already in flight when the trader acted, or
/// a plain re-report) and a stale snapshot keep it waiting: the order shows as reported, and
/// nothing on it acts, so no second cancel or modify goes out. A pending or final status, a new
/// revision, or for a move the requested price, answers it.
#[test]
fn an_order_re_report_at_the_requests_revision_keeps_it_waiting() {
    use crate::OrderStatus;
    let mut chart = working_order_market();
    let order_id = id("order-1", OrderId::new);
    let (x, y) = cancel_center(&mut chart, TradingObjectId::Order(order_id));
    click(&mut chart, x, y);
    let cancel = only_intent(&mut chart);
    assert_eq!(cancel.action, TradingIntentAction::CancelOrder);
    chart
        .update_working_order(order("order-1", OrderRole::Working, 104.0))
        .unwrap();
    assert_eq!(chart.trading_snapshot().orders.len(), 1);
    assert_eq!(chart.input_cursor(), ChartCursor::Default);
    click(&mut chart, x, y);
    assert!(chart.take_trading_intents().is_empty(), "no second cancel");
    let snapshot = chart.trading_snapshot();
    chart.set_trading_snapshot(snapshot).unwrap();
    assert_eq!(chart.input_cursor(), ChartCursor::Default);
    let mut cancelling = order("order-1", OrderRole::Working, 104.0);
    cancelling.status = OrderStatus::PendingCancel;
    chart.update_working_order(cancelling).unwrap();
    assert!(!chart.resolve_trading_intent(cancel.sequence, false));
    assert_eq!(
        chart.trading_snapshot().orders[0].status,
        OrderStatus::PendingCancel
    );

    let mut chart = take_profit_market();
    let (intent, x) = move_take_profit_unanswered(&mut chart);
    let moved = intent.price.unwrap();
    let y0 = price_y(&chart, 103.0);
    // An update sent before the broker saw the move still shows the old price.
    chart
        .update_working_order(order("tp-1", OrderRole::TakeProfit, 103.0))
        .unwrap();
    assert_eq!(chart.trading_snapshot().orders[0].price, 103.0);
    drag(&mut chart, (x, y0), (x, y0 - 30.0));
    assert!(chart.take_trading_intents().is_empty(), "no second modify");
    assert_eq!(chart.trading_snapshot().orders[0].price, 103.0);
    let mut confirmed = order("tp-1", OrderRole::TakeProfit, moved);
    confirmed.revision = 2;
    chart.update_working_order(confirmed).unwrap();
    assert!(!chart.resolve_trading_intent(intent.sequence, false));
    assert_eq!(chart.trading_snapshot().orders[0].price, moved);
}

/// A protection request is answered by the protection order the host adds, not by updates of the
/// position it protects: a P&L tick on the position leaves the chart waiting, so the `TP` button
/// cannot send a second request before the first one materializes.
#[test]
fn a_protection_request_waits_for_its_order_not_for_position_updates() {
    let mut chart = market(vec![position(PositionSide::Long)], Vec::new());
    let position_id = TradingObjectId::Position(id("position-1", PositionId::new));
    let line_y = price_y(&chart, 101.0);
    let tp_x = protection_button_x(&chart, line_y, TradingHitKind::TakeProfitButton);
    let target_y = price_y(&chart, 103.0);
    drag(&mut chart, (tp_x, line_y), (tp_x, target_y));
    let request = only_intent(&mut chart);
    assert_eq!(request.action, TradingIntentAction::CreateTakeProfit);

    let (close_x, close_y) = cancel_center(&mut chart, position_id);
    chart.input_pointer_move(at(close_x, close_y), false);
    assert_eq!(chart.input_cursor(), ChartCursor::Default);
    let mut tick = position(PositionSide::Long);
    tick.display_pnl = Some(24.0);
    chart.update_trading_position(tick).unwrap();
    assert_eq!(
        chart.input_cursor(),
        ChartCursor::Default,
        "a P&L tick is no answer"
    );
    chart.input_pointer_move(at(tp_x, line_y), false);
    assert_eq!(chart.input_cursor(), ChartCursor::Default);
    drag(&mut chart, (tp_x, line_y), (tp_x, target_y - 30.0));
    assert!(chart.take_trading_intents().is_empty(), "no second request");

    chart.input_pointer_move(at(close_x, close_y), false);
    chart
        .update_working_order(order("tp-1", OrderRole::TakeProfit, 103.0))
        .unwrap();
    assert_eq!(chart.input_cursor(), ChartCursor::Pointer);
    assert!(!chart.resolve_trading_intent(request.sequence, true));
}

/// An entry's protection request waits through the entry's transient broker states. A pending
/// modify (a broker attaching the leg by modifying the entry) is no answer, so once the entry is
/// working again its `TP` button still sends nothing until the protection order arrives.
#[test]
fn an_entry_protection_request_waits_through_a_pending_entry_state() {
    use crate::OrderStatus;
    let mut chart = working_order_market();
    let line_y = price_y(&chart, 104.0);
    let tp_x = protection_button_x(&chart, line_y, TradingHitKind::TakeProfitButton);
    let target_y = price_y(&chart, 102.0);
    drag(&mut chart, (tp_x, line_y), (tp_x, target_y));
    let request = only_intent(&mut chart);
    assert_eq!(request.action, TradingIntentAction::CreateTakeProfit);

    let mut modifying = order("order-1", OrderRole::Working, 104.0);
    modifying.status = OrderStatus::PendingModify;
    chart.update_working_order(modifying).unwrap();
    chart
        .update_working_order(order("order-1", OrderRole::Working, 104.0))
        .unwrap();
    assert_eq!(
        chart.trading_hit_at(tp_x, line_y).map(|hit| hit.kind),
        Some(TradingHitKind::TakeProfitButton)
    );
    drag(&mut chart, (tp_x, line_y), (tp_x, target_y));
    assert!(chart.take_trading_intents().is_empty(), "no second request");

    let mut take_profit = order("tp-1", OrderRole::TakeProfit, 102.0);
    take_profit.parent_order_id = Some(id("order-1", OrderId::new));
    chart.update_working_order(take_profit).unwrap();
    assert!(!chart.resolve_trading_intent(request.sequence, true));
}

/// Switching the visible account ends a hover or drag but never a request awaiting the host: the
/// lock is chart-wide, and the rollback survives, so a later rejection still restores the order.
#[test]
fn switching_the_visible_account_keeps_a_request_waiting() {
    let mut chart = take_profit_market();
    let (intent, x) = move_take_profit_unanswered(&mut chart);
    let y = price_y(&chart, intent.price.unwrap());
    chart.set_trading_visible_account(None);
    drag(&mut chart, (x, y), (x, y - 30.0));
    assert!(chart.take_trading_intents().is_empty());
    assert!(chart.resolve_trading_intent(intent.sequence, false));
    assert_eq!(chart.trading_snapshot().orders[0].price, 103.0);
}

/// The host can change an order while its line or button is held. A drag the host's state now
/// rules out ends as a cancel at once, and its release emits nothing: the dragged protection
/// order filled or entered a pending state, or the entry gained the protection being placed
/// through any link the host uses (here its bracket, with no parent id).
#[test]
fn a_held_drag_the_host_rules_out_ends_and_emits_nothing() {
    use crate::OrderStatus;
    for status in [OrderStatus::Filled, OrderStatus::PendingCancel] {
        let mut chart = take_profit_market();
        let x = chart.trading_marker_start() + 20.0;
        let y0 = price_y(&chart, 103.0);
        chart.input_pointer_down(at(x, y0), 1);
        chart.input_pointer_move(at(x, y0 - 30.0), true);
        assert!(chart.trading_preview().is_some(), "{status:?}");
        let mut changed = order("tp-1", OrderRole::TakeProfit, 103.0);
        changed.status = status;
        chart.update_working_order(changed).unwrap();
        assert_eq!(chart.trading_preview(), None, "{status:?}");
        chart.input_pointer_move(at(x, y0 - 40.0), true);
        chart.input_pointer_up(at(x, y0 - 40.0));
        assert!(chart.take_trading_intents().is_empty(), "{status:?}");
        assert_eq!(
            chart.trading_snapshot().orders[0].price,
            103.0,
            "{status:?}"
        );
    }

    let mut chart = working_order_market();
    let line_y = price_y(&chart, 104.0);
    let tp_x = protection_button_x(&chart, line_y, TradingHitKind::TakeProfitButton);
    let target_y = price_y(&chart, 102.0);
    chart.input_pointer_down(at(tp_x, line_y), 1);
    chart.input_pointer_move(at(tp_x, target_y), true);
    assert!(chart.trading_preview().is_some());
    let mut attached = order("tp-9", OrderRole::TakeProfit, 101.0);
    attached.position_id = None;
    assert!(attached.bracket_id.is_some() && attached.parent_order_id.is_none());
    chart.update_working_order(attached).unwrap();
    assert_eq!(chart.trading_preview(), None);
    chart.input_pointer_up(at(tp_x, target_y));
    assert!(chart.take_trading_intents().is_empty());
}

/// A click cancels or closes exactly the control its press began on. When the host removes the
/// pressed order mid-click and an identical order at the same price puts its close cell under the
/// pointer, nothing is cancelled. A press on a close cell that could not act (a request was in
/// flight) stays absorbed even when the host answers before the release.
#[test]
fn a_close_click_acts_only_on_the_control_its_press_began_on() {
    let mut chart = market(
        Vec::new(),
        vec![
            order("order-1", OrderRole::Working, 104.0),
            order("order-2", OrderRole::Working, 104.0),
        ],
    );
    let top = TradingObjectId::Order(id("order-2", OrderId::new));
    let twin = TradingObjectId::Order(id("order-1", OrderId::new));
    let (x, y) = cancel_center(&mut chart, top.clone());
    assert_eq!(
        chart.trading_hit_at(x, y).map(|hit| (hit.object, hit.kind)),
        Some((top, TradingHitKind::CancelButton))
    );
    chart.input_pointer_down(at(x, y), 1);
    assert_eq!(chart.input_cursor(), ChartCursor::Pointer);
    assert!(chart.remove_working_order(&id("order-2", OrderId::new)));
    assert_eq!(
        chart.trading_hit_at(x, y).map(|hit| (hit.object, hit.kind)),
        Some((twin, TradingHitKind::CancelButton)),
        "the twin's close cell is under the pointer now"
    );
    assert_eq!(
        chart.input_cursor(),
        ChartCursor::Default,
        "the pressed control is gone"
    );
    chart.input_pointer_up(at(x, y));
    assert!(chart.take_trading_intents().is_empty());
    assert_eq!(chart.trading_snapshot().orders.len(), 1);

    let mut chart = take_profit_market();
    let position_id = TradingObjectId::Position(id("position-1", PositionId::new));
    let (close_x, close_y) = cancel_center(&mut chart, position_id);
    let (intent, _) = move_take_profit_unanswered(&mut chart);
    chart.input_pointer_down(at(close_x, close_y), 1);
    assert!(chart.resolve_trading_intent(intent.sequence, true));
    assert_eq!(
        chart.input_cursor(),
        ChartCursor::Default,
        "the release will not act"
    );
    chart.input_pointer_up(at(close_x, close_y));
    assert!(
        chart.take_trading_intents().is_empty(),
        "the press was absorbed"
    );
    assert_eq!(chart.trading_snapshot().positions.len(), 1);
    click(&mut chart, close_x, close_y);
    assert_eq!(
        only_intent(&mut chart).action,
        TradingIntentAction::ClosePosition
    );
}

/// A control whose object's status rules its action out reads as inert on every path, with no
/// request in flight: the close cell of an order the broker is still submitting, modifying, or
/// cancelling, or one already filled, cancelled, rejected, or expired, and the line of a
/// protection order in those states. The arrow cursor, no hover surface or tooltip dwell, and a
/// press that requests nothing, drags nothing, and never pans.
#[test]
fn controls_whose_order_status_rules_the_action_out_read_inert() {
    use crate::OrderStatus;
    for status in [
        OrderStatus::PendingSubmit,
        OrderStatus::PendingModify,
        OrderStatus::PendingCancel,
        OrderStatus::Filled,
        OrderStatus::Cancelled,
        OrderStatus::Rejected,
        OrderStatus::Expired,
    ] {
        let mut protection = order("tp-1", OrderRole::TakeProfit, 103.0);
        protection.status = status;
        let mut chart = market(vec![position(PositionSide::Long)], vec![protection]);
        let (cx, cy) = cancel_center(&mut chart, TradingObjectId::Order(id("tp-1", OrderId::new)));
        let x = chart.trading_marker_start() + 20.0;
        let y0 = price_y(&chart, 103.0);
        let scroll = chart.scroll_position();
        let rest = resting_trading_layer(&mut chart);
        for (px, py, name) in [(cx, cy, "close"), (x, y0, "line")] {
            chart.input_pointer_move(at_ms(px, py, 1_000.0), false);
            assert_eq!(
                chart.input_cursor(),
                ChartCursor::Default,
                "{status:?} {name}"
            );
            assert_eq!(chart.trading_cursor_at(px, py), None, "{status:?} {name}");
            assert_eq!(chart.input_wake_deadline_ms(), None, "{status:?} {name}");
            assert_eq!(trading_layer(&mut chart), rest, "{status:?} {name}");
            chart.input_pointer_down(at(px, py), 1);
            assert_eq!(
                chart.input_cursor(),
                ChartCursor::Default,
                "{status:?} {name}"
            );
            assert_eq!(trading_layer(&mut chart), rest, "{status:?} {name}");
            chart.input_pointer_up(at(px, py));
            drag(&mut chart, (px, py), (px, py - 30.0));
            assert_eq!(chart.trading_preview(), None, "{status:?} {name}");
            assert!(chart.take_trading_intents().is_empty(), "{status:?} {name}");
            assert_eq!(chart.scroll_position(), scroll, "{status:?} {name}");
            assert_eq!(chart.trading_snapshot().orders[0].price, 103.0);
        }
    }
}

/// A press that opens no drag shows what its release does where it began: the click cursor on a
/// close cell that acts, and the pane's crosshair on a readout such as the position line, whose
/// press only keeps the chart from panning.
#[test]
fn a_trading_press_without_a_drag_shows_what_its_release_does() {
    let mut chart = take_profit_market();
    let position_y = price_y(&chart, 101.0);
    let body_x = (40..chart.trading_marker_start() as i32)
        .map(f64::from)
        .find(|&x| {
            chart.hit_test_series(x, position_y).is_none()
                && chart.trading_hit_at(x, position_y).map(|hit| hit.kind)
                    == Some(TradingHitKind::PositionLine)
        })
        .expect("bare position line");
    let scroll = chart.scroll_position();
    chart.input_pointer_move(at(body_x, position_y), false);
    let hover = chart.input_cursor();
    assert_eq!(hover, ChartCursor::Crosshair);
    chart.input_pointer_down(at(body_x, position_y), 1);
    assert_eq!(
        chart.input_cursor(),
        hover,
        "a readout press keeps the pane cursor"
    );
    chart.input_pointer_move(at(body_x - 40.0, position_y - 20.0), true);
    assert_eq!(chart.input_cursor(), hover);
    chart.input_pointer_up(at(body_x - 40.0, position_y - 20.0));
    assert_eq!(
        chart.scroll_position(),
        scroll,
        "the line keeps the chart from panning"
    );
    assert!(chart.take_trading_intents().is_empty());

    let (cx, cy) = cancel_center(
        &mut chart,
        TradingObjectId::Position(id("position-1", PositionId::new)),
    );
    chart.input_pointer_down(at(cx, cy), 1);
    assert_eq!(chart.input_cursor(), ChartCursor::Pointer);
    chart.input_pointer_up(at(cx, cy));
    assert_eq!(
        only_intent(&mut chart).action,
        TradingIntentAction::ClosePosition
    );
}

#[test]
fn escape_or_cancel_mid_order_drag_emits_nothing_and_frees_the_line() {
    type Interrupt = fn(&mut ChartEngine);
    let interrupts: [(&str, Interrupt); 2] = [
        ("Escape", |chart| {
            assert!(chart.input_key_down(ChartKey::Escape, InputModifiers::default(), false, 0.0));
        }),
        ("cancel", ChartEngine::input_cancel),
    ];
    for (name, interrupt) in interrupts {
        let mut chart = take_profit_market();
        let x = chart.trading_marker_start() + 20.0;
        let y0 = price_y(&chart, 103.0);
        chart.input_pointer_down(at(x, y0), 1);
        chart.input_pointer_move(at(x, y0 - 30.0), true);
        assert!(chart.trading_preview().is_some(), "{name}");

        interrupt(&mut chart);
        assert_eq!(chart.trading_preview(), None, "{name}");
        assert_eq!(chart.input_cursor(), ChartCursor::Crosshair, "{name}");
        // The button comes up after the interruption; that release commits nothing.
        chart.input_pointer_up(at(x, y0 - 30.0));
        assert!(chart.take_trading_intents().is_empty(), "{name}");
        assert_eq!(chart.trading_snapshot().orders[0].price, 103.0, "{name}");

        // The line is idle again rather than waiting on a host answer.
        drag(&mut chart, (x, y0), (x, y0 - 30.0));
        assert_eq!(
            only_intent(&mut chart).action,
            TradingIntentAction::ModifyOrder,
            "{name}"
        );
    }
}

/// A keyboard adjustment has no pointer press, so Escape must discard the preview itself.
#[test]
fn escape_discards_a_keyboard_order_preview_that_has_no_pointer_press() {
    let none = InputModifiers::default();
    let mut chart = take_profit_market();
    let (x, y) = empty_pane_point(&chart);
    assert!(chart.trading_hit_at(x, y).is_none());
    chart.input_pointer_move(at(x, y), false);
    assert_eq!(chart.input_cursor(), ChartCursor::Crosshair);

    assert!(chart.trading_keyboard_start_order(&id("tp-1", OrderId::new)));
    assert!(chart.trading_keyboard_adjust(4));
    assert_eq!(
        chart.trading_preview().map(|preview| preview.price),
        Some(104.0)
    );
    chart.input_pointer_move(at(x, y), false);
    assert_eq!(
        chart.input_cursor(),
        ChartCursor::VerticalGrabbing,
        "a live preview owns the cursor"
    );

    assert!(chart.input_key_down(ChartKey::Escape, none, false, 0.0));
    assert_eq!(chart.trading_preview(), None);
    assert_eq!(chart.input_cursor(), ChartCursor::Crosshair);
    assert!(chart.trading_keyboard_commit().is_none());
    assert!(chart.take_trading_intents().is_empty());
    assert_eq!(chart.trading_snapshot().orders[0].price, 103.0);
}

#[test]
fn the_close_cell_cancels_on_click_but_not_after_the_press_drags_off() {
    let order_id = id("order-1", OrderId::new);
    let mut chart = working_order_market();
    let (cx, cy) = cancel_center(&mut chart, TradingObjectId::Order(order_id.clone()));
    let scroll = chart.scroll_position();

    chart.input_pointer_move(at(cx, cy), false);
    assert_eq!(chart.input_cursor(), ChartCursor::Pointer);

    // A press that drags off the cell is not a click, even when it is released back on the cell.
    chart.input_pointer_down(at(cx, cy), 1);
    chart.input_pointer_move(at(cx - 40.0, cy), true);
    chart.input_pointer_move(at(cx, cy), true);
    chart.input_pointer_up(at(cx, cy));
    assert!(chart.take_trading_intents().is_empty());
    assert_eq!(chart.trading_snapshot().orders.len(), 1);

    click(&mut chart, cx, cy);
    let intent = only_intent(&mut chart);
    assert_eq!(intent.action, TradingIntentAction::CancelOrder);
    assert_eq!(intent.order_id, Some(order_id));
    assert!(chart.take_trading_intents().is_empty());
    assert!(chart.trading_snapshot().orders.is_empty());
    assert_eq!(chart.scroll_position(), scroll);
}

/// The engine owns no clock: the hover reports a wake deadline, and the host's tick at that
/// deadline reveals the tooltip in the next prepared frame.
#[test]
fn the_close_tooltip_arms_through_input_tick_exactly_at_the_dwell_deadline() {
    let mut chart = working_order_market();
    let (cx, cy) = cancel_center(
        &mut chart,
        TradingObjectId::Order(id("order-1", OrderId::new)),
    );
    let mut host = Host::default();
    assert!(host.prepare(&mut chart));

    chart.input_pointer_move(at_ms(cx, cy, 1_000.0), false);
    let deadline = 1_000.0 + TRADING_TOOLTIP_DWELL_MS;
    assert_eq!(chart.input_wake_deadline_ms(), Some(deadline));
    host.prepare(&mut chart);
    assert!(!host.shows("Cancel order"), "contact alone shows nothing");

    assert!(!chart.input_tick(deadline - 1.0));
    assert!(!host.prepare(&mut chart), "an early tick changes nothing");
    assert!(!host.shows("Cancel order"));

    assert!(chart.input_tick(deadline));
    assert_eq!(chart.input_wake_deadline_ms(), None);
    assert!(host.prepare(&mut chart));
    assert!(host.shows("Cancel order"));
    assert!(!chart.input_tick(deadline + 500.0), "the dwell fires once");

    // A shown tooltip stays while the hand wobbles inside the cell: no blink, no new wake.
    for (dx, dy, ms) in [
        (0.0, 1.0, 1_600.0),
        (1.0, -3.0, 1_650.0),
        (-2.0, 2.0, 1_700.0),
    ] {
        chart.input_pointer_move(at_ms(cx + dx, cy + dy, ms), false);
        assert_eq!(chart.input_wake_deadline_ms(), None, "{dx},{dy}");
        host.prepare(&mut chart);
        assert!(host.shows("Cancel order"), "{dx},{dy}");
    }
}

/// A different control restarts the dwell even when one motion sample carries the pointer straight
/// from one close cell to another, with no gap between them: the second tooltip waits its own full
/// dwell, and the first control's deadline arms nothing.
#[test]
fn moving_straight_from_one_close_cell_to_another_restarts_the_dwell() {
    let mut chart = take_profit_market();
    let (order_x, order_y) =
        cancel_center(&mut chart, TradingObjectId::Order(id("tp-1", OrderId::new)));
    let (position_x, position_y) = cancel_center(
        &mut chart,
        TradingObjectId::Position(id("position-1", PositionId::new)),
    );
    chart.input_pointer_move(at_ms(order_x, order_y, 1_000.0), false);
    assert_eq!(
        chart.input_wake_deadline_ms(),
        Some(1_000.0 + TRADING_TOOLTIP_DWELL_MS)
    );
    chart.input_pointer_move(at_ms(position_x, position_y, 1_200.0), false);
    assert_eq!(
        chart
            .trading_state
            .feedback_hover
            .as_ref()
            .map(|hit| hit.kind),
        Some(TradingHitKind::CancelButton)
    );
    assert_eq!(
        chart.input_wake_deadline_ms(),
        Some(1_200.0 + TRADING_TOOLTIP_DWELL_MS)
    );
    assert!(!chart.input_tick(1_000.0 + TRADING_TOOLTIP_DWELL_MS));
    assert!(!chart.trading_state.tooltip_armed);
    assert!(chart.input_tick(1_200.0 + TRADING_TOOLTIP_DWELL_MS));
}

/// The tooltip belongs to the control: once a press or a host snapshot hides it while the pointer
/// stays on the close cell, the next motion there dwells again, like the platform's own tooltips.
#[test]
fn the_close_tooltip_dwells_again_after_a_press_or_a_host_snapshot_hides_it() {
    let mut chart = working_order_market();
    let (cx, cy) = cancel_center(
        &mut chart,
        TradingObjectId::Order(id("order-1", OrderId::new)),
    );
    let mut host = Host::default();
    let dwell = |ms: f64| ms + TRADING_TOOLTIP_DWELL_MS;

    // A press that drags off the cell and returns is no click, and its release dwells again.
    chart.input_pointer_move(at_ms(cx, cy, 1_000.0), false);
    chart.input_pointer_down(at_ms(cx, cy, 1_100.0), 1);
    assert_eq!(chart.input_wake_deadline_ms(), None);
    chart.input_pointer_move(at_ms(cx - 40.0, cy, 1_150.0), true);
    chart.input_pointer_move(at_ms(cx, cy, 1_200.0), true);
    chart.input_pointer_up(at_ms(cx, cy, 1_250.0));
    assert!(chart.take_trading_intents().is_empty());
    assert_eq!(chart.input_wake_deadline_ms(), Some(dwell(1_250.0)));
    chart.input_pointer_move(at_ms(cx, cy + 1.0, 1_300.0), false);
    assert_eq!(chart.input_wake_deadline_ms(), Some(dwell(1_250.0)));
    assert!(chart.input_tick(dwell(1_250.0)));
    host.prepare(&mut chart);
    assert!(host.shows("Cancel order"));

    // A host snapshot replaces the order under the resting pointer; the next motion on the cell
    // brings its tooltip back after one dwell.
    let snapshot = chart.trading_snapshot();
    chart.set_trading_snapshot(snapshot).unwrap();
    chart.input_pointer_move(at_ms(cx + 1.0, cy, 2_000.0), false);
    chart.input_tick(dwell(2_000.0));
    host.prepare(&mut chart);
    assert!(host.shows("Cancel order"));
    assert_eq!(chart.input_wake_deadline_ms(), None);
}

#[test]
fn the_close_tooltip_dwell_survives_holding_still_and_any_gesture_clears_it() {
    let mut chart = working_order_market();
    let (cx, cy) = cancel_center(
        &mut chart,
        TradingObjectId::Order(id("order-1", OrderId::new)),
    );
    let body_x = chart.trading_marker_start() + 20.0;
    let dwell = |ms: f64| Some(ms + TRADING_TOOLTIP_DWELL_MS);

    // Holding still, or wobbling sideways or vertically within the cell, lets the first dwell
    // elapse: the dwell belongs to the control under the pointer, not to its exact pixel.
    chart.input_pointer_move(at_ms(cx, cy, 1_000.0), false);
    chart.input_pointer_move(at_ms(cx, cy, 1_200.0), false);
    chart.input_pointer_move(at_ms(cx + 2.0, cy, 1_300.0), false);
    chart.input_pointer_move(at_ms(cx + 2.0, cy + 1.0, 1_310.0), false);
    assert_eq!(chart.input_wake_deadline_ms(), dwell(1_000.0));
    chart.input_pointer_move(at_ms(cx, cy - 3.0, 1_320.0), false);
    assert_eq!(chart.input_wake_deadline_ms(), dwell(1_000.0));
    // The order body carries no tooltip.
    chart.input_pointer_move(at_ms(body_x, cy, 1_350.0), false);
    assert_eq!(chart.input_wake_deadline_ms(), None);
    assert!(!chart.input_tick(dwell(1_000.0).unwrap()));

    chart.input_pointer_move(at_ms(cx, cy, 2_000.0), false);
    assert_eq!(chart.input_wake_deadline_ms(), dwell(2_000.0));
    chart.input_pointer_leave();
    assert_eq!(chart.input_wake_deadline_ms(), None);
    assert!(!chart.input_tick(dwell(2_000.0).unwrap()));

    chart.input_pointer_move(at_ms(cx, cy, 3_000.0), false);
    assert_eq!(chart.input_wake_deadline_ms(), dwell(3_000.0));
    chart.input_cancel();
    assert_eq!(chart.input_wake_deadline_ms(), None);

    chart.input_pointer_move(at_ms(cx, cy, 4_000.0), false);
    assert_eq!(chart.input_wake_deadline_ms(), dwell(4_000.0));
    chart.input_pointer_down(at_ms(cx, cy, 4_100.0), 1);
    assert_eq!(chart.input_wake_deadline_ms(), None);
    assert!(
        !chart.input_tick(dwell(4_000.0).unwrap()),
        "a press never arms it"
    );
    chart.input_pointer_move(at_ms(cx - 40.0, cy, 4_200.0), true);
    chart.input_pointer_up(at_ms(cx - 40.0, cy, 4_300.0));
    assert!(chart.take_trading_intents().is_empty());
}

#[test]
fn a_protection_button_press_drag_creates_one_protection_request() {
    let mut chart = market(vec![position(PositionSide::Long)], Vec::new());
    let line_y = price_y(&chart, 101.0);
    let target_y = price_y(&chart, 103.0);
    let x = protection_button_x(&chart, line_y, TradingHitKind::TakeProfitButton);

    chart.input_pointer_move(at(x, line_y), false);
    assert_eq!(chart.input_cursor(), ChartCursor::Pointer);

    chart.input_pointer_down(at(x, line_y), 1);
    chart.input_pointer_move(at(x, (line_y + target_y) / 2.0), true);
    chart.input_pointer_move(at(x, target_y), true);
    assert_eq!(chart.input_cursor(), ChartCursor::VerticalGrabbing);
    let preview = chart.trading_preview().expect("take-profit preview");
    assert_eq!(preview.role, OrderRole::TakeProfit);
    assert_eq!(preview.price, 103.0);
    assert!(chart.take_trading_intents().is_empty());

    chart.input_pointer_up(at(x, target_y));
    let intent = only_intent(&mut chart);
    assert_eq!(intent.action, TradingIntentAction::CreateTakeProfit);
    assert_eq!(
        intent.position_id.as_ref().map(PositionId::as_str),
        Some("position-1")
    );
    assert_eq!(intent.side, Some(OrderSide::Sell));
    assert_eq!(intent.price, Some(103.0));
    assert!(chart.take_trading_intents().is_empty());
    assert_eq!(chart.trading_preview(), None);
}

/// The take profit at 103 on a tick finer than a pixel, and the x of its line body.
fn fine_take_profit_market() -> (ChartEngine, f64) {
    let chart = market_with_tick(
        FINE_TICK,
        vec![position(PositionSide::Long)],
        vec![order("tp-1", OrderRole::TakeProfit, 103.0)],
    );
    let x = chart.trading_marker_start() + 20.0;
    (chart, x)
}

/// The price the order line shows: the live preview's, else the order's own.
fn shown_price(chart: &ChartEngine) -> f64 {
    chart
        .trading_preview()
        .map_or(chart.trading_snapshot().orders[0].price, |preview| {
            preview.price
        })
}

/// A press that never travels the shared 5 px slop is a click on every device. The press lands
/// 4 px off the line, where the tick is so fine that the press row is another price; the line
/// still keeps its price, and no request reaches the host.
#[test]
fn a_wobbling_click_on_an_order_line_never_moves_it_or_requests_a_change() {
    for device in [InputDevice::Mouse, InputDevice::Pen, InputDevice::Touch] {
        let (mut chart, x) = fine_take_profit_market();
        let y0 = price_y(&chart, 103.0) + 4.0;
        assert_eq!(
            chart.trading_hit_at(x, y0).map(|hit| hit.kind),
            Some(TradingHitKind::OrderLine)
        );
        assert_ne!(snapped_price_at(&chart, y0), 103.0);
        let scroll = chart.scroll_position();

        chart.input_pointer_down(device_at(device, x, y0), 1);
        // 2, 3, and 4 px (Manhattan) from the press.
        for (dx, dy) in [(1.0, 1.0), (2.0, -1.0), (-1.0, 3.0)] {
            chart.input_pointer_move(device_at(device, x + dx, y0 + dy), true);
            assert_eq!(shown_price(&chart), 103.0, "{device:?} {dx},{dy}");
        }
        chart.input_pointer_up(device_at(device, x - 1.0, y0 + 3.0));
        assert!(chart.take_trading_intents().is_empty(), "{device:?}");
        assert_eq!(chart.trading_preview(), None, "{device:?}");
        assert_eq!(
            chart.trading_snapshot().orders[0].price,
            103.0,
            "{device:?}"
        );
        assert_eq!(chart.scroll_position(), scroll, "{device:?}");
    }
}

/// A wobbling click on a `TP` or `SL` button creates nothing, even 8 px toward the side where a
/// drag would make a valid protection.
#[test]
fn a_wobbling_click_on_a_protection_button_creates_no_protection() {
    for (kind, toward) in [
        (TradingHitKind::TakeProfitButton, -1.0),
        (TradingHitKind::StopLossButton, 1.0),
    ] {
        let mut chart = market_with_tick(FINE_TICK, vec![position(PositionSide::Long)], Vec::new());
        let line_y = price_y(&chart, 101.0);
        let x = protection_button_x(&chart, line_y, kind);
        let y0 = line_y + 8.0 * toward;
        assert_eq!(chart.trading_hit_at(x, y0).map(|hit| hit.kind), Some(kind));

        chart.input_pointer_down(at(x, y0), 1);
        chart.input_pointer_move(at(x, y0 + 2.0 * toward), true);
        chart.input_pointer_move(at(x + 1.0, y0 + 3.0 * toward), true);
        chart.input_pointer_up(at(x + 1.0, y0 + 3.0 * toward));
        assert!(chart.take_trading_intents().is_empty(), "{kind:?}");
        assert_eq!(chart.trading_preview(), None, "{kind:?}");
        assert!(chart.trading_snapshot().orders.is_empty(), "{kind:?}");
    }
}

/// Past the slop the order catches up with the pointer at once and then follows it exactly, back
/// inside the slop too: a change smaller than the slop is a pull away and back.
#[test]
fn an_order_drag_past_the_slop_follows_the_pointer_back_near_its_start() {
    let (mut chart, x) = fine_take_profit_market();
    let y0 = price_y(&chart, 103.0);

    chart.input_pointer_down(at(x, y0), 1);
    chart.input_pointer_move(at(x, y0 - 4.0), true);
    assert_eq!(shown_price(&chart), 103.0, "inside the slop the line stays");
    chart.input_pointer_move(at(x, y0 - 12.0), true);
    assert_eq!(
        shown_price(&chart),
        snapped_price_at(&chart, y0 - 12.0),
        "crossing the slop catches up at once"
    );
    chart.input_pointer_move(at(x, y0 - 2.0), true);
    let target = snapped_price_at(&chart, y0 - 2.0);
    assert!(target > 103.0, "{target}");
    assert_eq!(shown_price(&chart), target);

    chart.input_pointer_up(at(x, y0 - 2.0));
    let intent = only_intent(&mut chart);
    assert_eq!(intent.action, TradingIntentAction::ModifyOrder);
    assert_eq!(intent.price, Some(target));
}

/// A release past the slop is a drag even when no motion sample crossed it first: the order lands
/// where the pointer lets go.
#[test]
fn an_order_released_past_the_slop_without_crossing_motion_lands_at_the_release() {
    let mut chart = take_profit_market();
    let x = chart.trading_marker_start() + 20.0;
    let y0 = price_y(&chart, 103.0);
    let target = snapped_price_at(&chart, y0 - 30.0);
    assert!(target > 103.0, "{target}");

    chart.input_pointer_down(at(x, y0), 1);
    chart.input_pointer_move(at(x, y0 - 2.0), true);
    chart.input_pointer_up(at(x, y0 - 30.0));
    let intent = only_intent(&mut chart);
    assert_eq!(intent.action, TradingIntentAction::ModifyOrder);
    assert_eq!(intent.price, Some(target));
}

/// One buy fill at 101 on bar 30 of the fixture.
fn buy_fill_at_bar_30() -> TradingExecution {
    TradingExecution {
        id: id("fill-1", ExecutionId::new),
        account_id: None,
        pane_index: 0,
        price_scale: TradingPriceScale::Right,
        side: OrderSide::Buy,
        kind: ExecutionKind::Entry,
        time: 2_800,
        price: 101.0,
        quantity: 1.0,
        order_id: None,
        position_id: None,
        marker_shape: ExecutionMarkerShape::default(),
        size_by_quantity: false,
    }
}

#[test]
fn an_execution_arrow_owns_its_click_without_selecting_or_emitting() {
    let mut chart = chart();
    let (sx, sy) = series_point(&chart);
    click(&mut chart, sx, sy);
    assert_eq!(chart.selected_series(), Some(0));
    chart
        .set_trading_snapshot(TradingSnapshot {
            executions: vec![buy_fill_at_bar_30()],
            ..TradingSnapshot::default()
        })
        .unwrap();
    chart.build_frame();
    let layout = chart.trading_execution_layout(0);
    let (x, y) = (layout.marks[0].x, layout.marks[0].y);
    // A click on bare pane space here would clear the series selection.
    assert_eq!(chart.hit_test_series(x, y), None);

    chart.input_pointer_move(at(x, y), false);
    assert_eq!(chart.input_cursor(), ChartCursor::Pointer);
    click(&mut chart, x, y);
    assert_eq!(chart.selected_series(), Some(0));
    assert!(chart.take_trading_intents().is_empty());
}

/// Arm a trend line, the click-anchor tool every armed-tool scenario below uses.
fn arm_trend_line(chart: &mut ChartEngine) {
    assert!(chart.set_drawing_tool(Some(DrawingKind::TrendLine), None, None));
}

fn drawings_created(chart: &mut ChartEngine) -> usize {
    chart
        .take_input_events()
        .iter()
        .filter(|event| matches!(event, ChartInputEvent::DrawingCreated(_)))
        .count()
}

/// While a drawing tool is armed the tool owns order and position line bodies, the order chip's
/// drag area included: the pointer reads as the tool's crosshair there, the line neither
/// highlights nor hides the crosshair, and a press belongs to the tool, so nothing financial moves
/// and no request reaches the host. Once the one-shot tool commits and disarms, the line drags
/// again.
#[test]
fn an_armed_drawing_tool_owns_order_and_position_line_bodies() {
    let mut chart = take_profit_market();
    let order_y = price_y(&chart, 103.0);
    let position_y = price_y(&chart, 101.0);
    let chip_x = chart.trading_marker_start() + 20.0;
    let [first_x, second_x, free_x] = [120.0, 260.0, 60.0];
    assert!(second_x < chart.trading_marker_start() - 20.0);
    let bodies = [
        (first_x, order_y, TradingHitKind::OrderLine),
        (chip_x, order_y, TradingHitKind::OrderLine),
        (second_x, position_y, TradingHitKind::PositionLine),
    ];
    for (x, y, kind) in bodies {
        assert_eq!(chart.trading_hit_at(x, y).map(|hit| hit.kind), Some(kind));
    }
    let scroll = chart.scroll_position();
    arm_trend_line(&mut chart);

    for (x, y, kind) in bodies {
        chart.input_pointer_move(at(x, y), false);
        assert_eq!(chart.input_cursor(), ChartCursor::Crosshair, "{kind:?} {x}");
        assert_eq!(chart.trading_state.feedback_hover, None, "{kind:?} {x}");
        assert!(
            !chart.crosshair_suppressed_by_interaction(),
            "{kind:?} {x}: the tool's crosshair shows over the line"
        );
    }

    // A press-drag-release on the line or its chip drags no order: the press is the tool's, and a
    // click-anchor tool places on clicks only.
    for x in [first_x, chip_x] {
        chart.input_pointer_down(at(x, order_y), 1);
        assert_eq!(chart.input_cursor(), ChartCursor::Crosshair, "{x}");
        for dy in [-15.0, -30.0] {
            chart.input_pointer_move(at(x, order_y + dy), true);
            assert_eq!(chart.trading_preview(), None, "{x}");
            assert_eq!(chart.input_cursor(), ChartCursor::Crosshair, "{x}");
        }
        chart.input_pointer_up(at(x, order_y - 30.0));
    }
    assert!(chart.take_trading_intents().is_empty());
    assert_eq!(chart.trading_snapshot().orders[0].price, 103.0);
    assert_eq!(chart.scroll_position(), scroll);
    assert_eq!(drawings_created(&mut chart), 0);

    // A click on the order line places the first anchor; with that anchor pending, the position
    // line still reads as the tool's and a click there places the second.
    click(&mut chart, first_x, order_y);
    chart.input_pointer_move(at(second_x, position_y), false);
    assert_eq!(chart.input_cursor(), ChartCursor::Crosshair);
    assert_eq!(chart.trading_state.feedback_hover, None);
    click(&mut chart, second_x, position_y);
    assert_eq!(drawings_created(&mut chart), 1);
    assert_eq!(chart.drawings().len(), 1);
    assert_eq!(chart.active_drawing_tool(), None, "one-shot tools disarm");
    assert!(chart.take_trading_intents().is_empty());
    assert_eq!(chart.trading_snapshot().orders[0].price, 103.0);
    assert_eq!(chart.trading_snapshot().positions[0].average_price, 101.0);

    // Disarmed, the order line reads as draggable again and drags.
    chart.input_pointer_move(at(free_x, order_y), false);
    assert_eq!(chart.input_cursor(), ChartCursor::VerticalGrab);
    assert!(chart.trading_state.feedback_hover.is_some());
    let target = snapped_price_at(&chart, order_y - 30.0);
    drag(&mut chart, (free_x, order_y), (free_x, order_y - 30.0));
    let intent = only_intent(&mut chart);
    assert_eq!(intent.action, TradingIntentAction::ModifyOrder);
    assert_eq!(intent.price, Some(target));
}

/// Marker buttons keep their press and read as buttons while a drawing tool is armed: the close
/// cell cancels, a protection button still press-drags a protection, and an execution arrow keeps
/// its click. None of them places an anchor, and the tool stays armed. The crosshair's alert chip
/// also wins a press over the tool, so it reads as a button too.
#[test]
fn marker_buttons_keep_their_press_and_pointer_cursor_while_a_tool_is_armed() {
    let order_id = id("order-1", OrderId::new);
    let mut chart = working_order_market();
    let (cx, cy) = cancel_center(&mut chart, TradingObjectId::Order(order_id.clone()));
    arm_trend_line(&mut chart);
    chart.input_pointer_move(at(cx, cy), false);
    assert_eq!(chart.input_cursor(), ChartCursor::Pointer);
    click(&mut chart, cx, cy);
    let intent = only_intent(&mut chart);
    assert_eq!(intent.action, TradingIntentAction::CancelOrder);
    assert_eq!(intent.order_id, Some(order_id));
    assert_eq!(chart.active_drawing_tool(), Some(DrawingKind::TrendLine));
    assert!(
        chart.pending_drawing().is_none(),
        "the click placed no anchor"
    );

    let mut chart = market(vec![position(PositionSide::Long)], Vec::new());
    let line_y = price_y(&chart, 101.0);
    let target_y = price_y(&chart, 103.0);
    let x = protection_button_x(&chart, line_y, TradingHitKind::TakeProfitButton);
    arm_trend_line(&mut chart);
    chart.input_pointer_move(at(x, line_y), false);
    assert_eq!(chart.input_cursor(), ChartCursor::Pointer);
    chart.input_pointer_down(at(x, line_y), 1);
    chart.input_pointer_move(at(x, (line_y + target_y) / 2.0), true);
    chart.input_pointer_move(at(x, target_y), true);
    assert_eq!(chart.input_cursor(), ChartCursor::VerticalGrabbing);
    chart.input_pointer_up(at(x, target_y));
    let intent = only_intent(&mut chart);
    assert_eq!(intent.action, TradingIntentAction::CreateTakeProfit);
    assert_eq!(intent.price, Some(103.0));
    assert_eq!(chart.active_drawing_tool(), Some(DrawingKind::TrendLine));
    assert!(chart.pending_drawing().is_none());
    assert_eq!(drawings_created(&mut chart), 0);

    let mut arrows = super::tests::chart();
    arrows
        .set_trading_snapshot(TradingSnapshot {
            executions: vec![buy_fill_at_bar_30()],
            ..TradingSnapshot::default()
        })
        .unwrap();
    arrows.build_frame();
    let layout = arrows.trading_execution_layout(0);
    let (x, y) = (layout.marks[0].x, layout.marks[0].y);
    arm_trend_line(&mut arrows);
    arrows.input_pointer_move(at(x, y), false);
    assert_eq!(arrows.input_cursor(), ChartCursor::Pointer);
    click(&mut arrows, x, y);
    assert_eq!(arrows.active_drawing_tool(), Some(DrawingKind::TrendLine));
    assert!(arrows.pending_drawing().is_none());

    let mut chart = super::tests::chart();
    let (x, y) = (chart.pane_w - 4.0, 200.0);
    arm_trend_line(&mut chart);
    chart.input_pointer_move(at(x, y), false);
    assert!(chart.alert_create_hit_at(x, y));
    assert_eq!(chart.input_cursor(), ChartCursor::Pointer);
    click(&mut chart, x, y);
    assert_eq!(chart.take_alert_create_requests().len(), 1);
    assert_eq!(chart.active_drawing_tool(), Some(DrawingKind::TrendLine));
    assert!(chart.pending_drawing().is_none());
}

/// Annotation chips are readouts on the marker, so the armed tool owns them as it owns the
/// marker's drag area: the tool's crosshair over them, no hover feedback, and a click there places
/// the tool's anchor. Disarmed, the chip answers its hover again.
#[test]
fn an_armed_drawing_tool_owns_annotation_chips() {
    use crate::{TradingAnnotation, TradingAnnotationPlacement, TradingAnnotationTone};
    let mut annotated = position(PositionSide::Long);
    annotated.annotations = vec![TradingAnnotation {
        id: "queue".to_string(),
        text: "Q 12".to_string(),
        tone: TradingAnnotationTone::Info,
        tooltip: Some("queue position".to_string()),
        placement: TradingAnnotationPlacement::Inline,
    }];
    let mut chart = market(vec![annotated], Vec::new());
    let (x, y) = (chart.trading_marker_start() + 12.0, price_y(&chart, 101.0));
    assert_eq!(
        chart.trading_hit_at(x, y).map(|hit| hit.kind),
        Some(TradingHitKind::Annotation)
    );
    chart.input_pointer_move(at(x, y), false);
    assert!(
        chart.trading_state.feedback_hover.is_some(),
        "an idle chip hovers"
    );

    arm_trend_line(&mut chart);
    assert_eq!(chart.trading_state.feedback_hover, None);
    assert_eq!(chart.input_cursor(), ChartCursor::Crosshair);
    click(&mut chart, x, y);
    assert!(chart.pending_drawing().is_some(), "the click is the tool's");
    assert!(chart.take_trading_intents().is_empty());

    chart.cancel_drawing_tool();
    assert!(chart.trading_state.feedback_hover.is_some());
}

/// The armed tool owns order lines for every pointer device: a mouse, pen, or touch press-drag
/// along the line moves no order and requests nothing, and a click or tap there places the tool's
/// first anchor instead of acting on the order.
#[test]
fn every_device_press_on_an_order_line_belongs_to_the_armed_tool() {
    for device in [InputDevice::Mouse, InputDevice::Pen, InputDevice::Touch] {
        let mut chart = take_profit_market();
        let (x, y) = (120.0, price_y(&chart, 103.0));
        let scroll = chart.scroll_position();
        arm_trend_line(&mut chart);

        chart.input_pointer_down(device_at(device, x, y), 1);
        for dy in [-15.0, -30.0] {
            chart.input_pointer_move(device_at(device, x, y + dy), true);
            assert_eq!(chart.trading_preview(), None, "{device:?}");
        }
        chart.input_pointer_up(device_at(device, x, y - 30.0));
        assert!(chart.take_trading_intents().is_empty(), "{device:?}");
        assert_eq!(
            chart.trading_snapshot().orders[0].price,
            103.0,
            "{device:?}"
        );
        assert_eq!(chart.scroll_position(), scroll, "{device:?}");
        assert!(
            chart.pending_drawing().is_none(),
            "{device:?}: a drag places nothing"
        );

        chart.input_pointer_down(device_at(device, x, y), 1);
        chart.input_pointer_up(device_at(device, x, y));
        assert!(
            chart.pending_drawing().is_some(),
            "{device:?}: the click is the tool's"
        );
        assert_eq!(chart.active_drawing_tool(), Some(DrawingKind::TrendLine));
        assert!(chart.take_trading_intents().is_empty(), "{device:?}");
        assert_eq!(
            chart.trading_snapshot().orders[0].price,
            103.0,
            "{device:?}"
        );
    }
}

/// A commit that disarms the one-shot tool hands the line under a resting pointer back to trading
/// at once, however the commit arrived: Enter finishing a path whose last vertex sits on the line,
/// or a host placing the anchors through the direct tool API.
#[test]
fn a_commit_that_disarms_the_tool_frees_the_line_under_a_resting_pointer() {
    let mut chart = take_profit_market();
    let (x, y) = (120.0, price_y(&chart, 103.0));
    assert!(chart.set_drawing_tool(Some(DrawingKind::Path), None, None));
    click(&mut chart, 60.0, 80.0);
    click(&mut chart, x, y);
    assert_eq!(chart.input_cursor(), ChartCursor::Crosshair);
    assert_eq!(chart.trading_state.feedback_hover, None);
    assert!(chart.input_key_down(ChartKey::Enter, InputModifiers::default(), false, 0.0));
    assert_eq!(drawings_created(&mut chart), 1);
    assert_eq!(chart.active_drawing_tool(), None);
    assert_eq!(chart.input_cursor(), ChartCursor::VerticalGrab);
    assert_eq!(
        chart.trading_state.feedback_hover.map(|hit| hit.kind),
        Some(TradingHitKind::OrderLine)
    );

    let mut chart = take_profit_market();
    chart.input_pointer_move(at(x, y), false);
    arm_trend_line(&mut chart);
    assert_eq!(chart.trading_state.feedback_hover, None);
    let modifiers = DrawingModifiers::default();
    assert!(
        chart
            .drawing_tool_activate(60.0, 80.0, modifiers)
            .created
            .is_none()
    );
    assert!(
        chart
            .drawing_tool_activate(200.0, 60.0, modifiers)
            .created
            .is_some()
    );
    assert_eq!(chart.active_drawing_tool(), None);
    assert_eq!(chart.input_cursor(), ChartCursor::VerticalGrab);
    assert_eq!(
        chart.trading_state.feedback_hover.map(|hit| hit.kind),
        Some(TradingHitKind::OrderLine)
    );
}

/// Arming or disarming under a resting pointer updates the line's affordance at once, without
/// waiting for motion: through the host API, and through Escape, after which the line drags.
#[test]
fn arming_or_disarming_under_a_resting_pointer_updates_the_line_affordance_at_once() {
    let mut chart = take_profit_market();
    let (x, y) = (120.0, price_y(&chart, 103.0));
    chart.input_pointer_move(at(x, y), false);
    assert_eq!(chart.input_cursor(), ChartCursor::VerticalGrab);
    assert!(chart.trading_state.feedback_hover.is_some());

    arm_trend_line(&mut chart);
    assert_eq!(chart.input_cursor(), ChartCursor::Crosshair);
    assert_eq!(chart.trading_state.feedback_hover, None);
    assert!(!chart.crosshair_suppressed_by_interaction());

    chart.cancel_drawing_tool();
    assert_eq!(chart.input_cursor(), ChartCursor::VerticalGrab);
    assert!(chart.trading_state.feedback_hover.is_some());

    arm_trend_line(&mut chart);
    assert!(chart.input_key_down(ChartKey::Escape, InputModifiers::default(), false, 0.0));
    assert_eq!(chart.active_drawing_tool(), None);
    assert_eq!(chart.input_cursor(), ChartCursor::VerticalGrab);
    drag(&mut chart, (x, y), (x, y - 30.0));
    assert_eq!(
        only_intent(&mut chart).action,
        TradingIntentAction::ModifyOrder
    );
}
