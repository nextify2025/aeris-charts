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

/// The interactive fixture carrying a trading snapshot with a 0.25 instrument tick.
fn market(positions: Vec<TradingPosition>, orders: Vec<WorkingOrder>) -> ChartEngine {
    let mut chart = chart();
    chart
        .set_trading_snapshot(TradingSnapshot {
            instrument: InstrumentMetadata {
                tick_size: Some(TICK),
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

/// The tick-snapped price an order dragged to `y` previews.
fn snapped_price_at(chart: &ChartEngine, y: f64) -> f64 {
    let raw = chart
        .trading_coordinate_to_price(0, TradingPriceScale::Right, y)
        .unwrap();
    (raw / TICK).round() * TICK
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
    // The cursor over a line still waiting on the host is not pinned here; once the host
    // accepts, the moved line is idle and grabbable under the pointer again.
    assert!(chart.resolve_trading_intent(intent.sequence, true));
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

    // Until the host answers, a press on the moved line neither drags it nor pans the chart.
    chart.input_pointer_down(at(x, y1), 1);
    chart.input_pointer_move(at(x, y1 - 15.0), true);
    chart.input_pointer_move(at(x, y1 - 30.0), true);
    assert_eq!(chart.trading_preview(), None);
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

    // Holding still, or moving sideways within the cell, lets the first dwell elapse. (A vertical
    // wobble inside the cell currently restarts it, a known defect this test does not pin.)
    chart.input_pointer_move(at_ms(cx, cy, 1_000.0), false);
    chart.input_pointer_move(at_ms(cx, cy, 1_200.0), false);
    chart.input_pointer_move(at_ms(cx + 2.0, cy, 1_300.0), false);
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

#[test]
fn an_execution_arrow_owns_its_click_without_selecting_or_emitting() {
    let mut chart = chart();
    let (sx, sy) = series_point(&chart);
    click(&mut chart, sx, sy);
    assert_eq!(chart.selected_series(), Some(0));
    chart
        .set_trading_snapshot(TradingSnapshot {
            executions: vec![TradingExecution {
                id: id("fill-1", ExecutionId::new),
                account_id: None,
                pane_index: 0,
                price_scale: TradingPriceScale::Right,
                side: OrderSide::Buy,
                kind: ExecutionKind::Entry,
                // Bar 30.
                time: 2_800,
                price: 101.0,
                quantity: 1.0,
                order_id: None,
                position_id: None,
                marker_shape: ExecutionMarkerShape::default(),
                size_by_quantity: false,
            }],
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
