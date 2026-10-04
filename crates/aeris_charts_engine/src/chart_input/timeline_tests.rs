//! Timeline-mark lane scenarios through the public `input_*` API: click ownership and the
//! activation outcome, drag suppression, double-click editor guarding, hover, cursor, the dwell
//! tooltip and the active trading group.

use super::tests::{at, at_ms, chart, click, drag, relayout, BARS};
use super::*;
use crate::{
    ChartFrame, FinancialFrameRequest, OrderRole, PositionSide, TimelineGlyphShape, TimelineMark,
    TimelineMarkGlyph, TimelineMarkGroup, TimelineMarksSnapshot, TradingPriceScale,
    TradingSnapshot,
};
use aeris_charts_render::draw_list::Prim;

fn mark(id: &str, bar: usize, group: &str) -> TimelineMark {
    TimelineMark {
        id: id.to_string(),
        time: 1_000 + bar as i64 * 60,
        group: group.to_string(),
        glyph: TimelineMarkGlyph {
            shape: TimelineGlyphShape::Circle,
            color: "#ff0000".into(),
            letter: "E".into(),
        },
        title: format!("{id} title"),
    }
}

fn marked_chart(marks: Vec<TimelineMark>) -> ChartEngine {
    let mut chart = chart();
    chart
        .set_timeline_marks(TimelineMarksSnapshot {
            marks,
            groups: vec![TimelineMarkGroup {
                id: "earnings".into(),
                label: "Earnings".into(),
            }],
        })
        .unwrap();
    chart.build_frame();
    chart
}

fn token_center(chart: &ChartEngine, bar: usize) -> (f64, f64) {
    let pane = &chart.panes[0];
    (
        chart.time_scale.index_to_coordinate(bar as i64),
        pane.top + pane.height - 15.0,
    )
}

fn activations(chart: &mut ChartEngine) -> Vec<u32> {
    chart
        .take_input_events()
        .into_iter()
        .filter_map(|event| match event {
            ChartInputEvent::TimelineMarkActivated(seq) => Some(seq),
            _ => None,
        })
        .collect()
}

#[test]
fn a_token_click_emits_one_activation_whose_hit_resolves() {
    let mut chart = marked_chart(vec![mark("a", 30, "earnings")]);
    let (x, y) = token_center(&chart, 30);
    assert!(
        chart.hit_test_series(x, y).is_none(),
        "the lane sits below the data"
    );
    click(&mut chart, x, y);
    let seqs = activations(&mut chart);
    assert_eq!(seqs.len(), 1, "exactly one outcome per click");
    let hit = chart
        .timeline_mark_activation(seqs[0])
        .expect("the hit is retained");
    assert_eq!(hit.mark_ids, ["a"]);
    assert_eq!(hit.logical, 30);
    assert_eq!(hit.label, "Earnings");
    assert_eq!(
        chart.selected_series(),
        None,
        "a token click selects nothing"
    );
    assert_eq!(chart.timeline_mark_activation(seqs[0] + 1), None);
    // A plain pane click emits a click, never an activation.
    click(&mut chart, x, y - 120.0);
    let events = chart.take_input_events();
    assert!(events
        .iter()
        .any(|event| matches!(event, ChartInputEvent::Click { .. })));
    assert!(!events
        .iter()
        .any(|event| matches!(event, ChartInputEvent::TimelineMarkActivated(_))));
}

#[test]
fn a_drag_from_a_token_emits_nothing_and_does_not_pan() {
    let mut chart = marked_chart(vec![mark("a", 30, "earnings")]);
    let (x, y) = token_center(&chart, 30);
    let scroll = chart.scroll_position();
    drag(&mut chart, (x, y), (x + 40.0, y));
    assert!(activations(&mut chart).is_empty());
    assert_eq!(
        chart.scroll_position(),
        scroll,
        "a token press is a control, not a pan"
    );
}

#[test]
fn a_release_on_another_token_emits_nothing() {
    let mut chart = marked_chart(vec![mark("a", 30, "earnings"), mark("b", 33, "earnings")]);
    let (ax, y) = token_center(&chart, 30);
    let (bx, _) = token_center(&chart, 33);
    assert!(chart.timeline_mark_hit_at(ax, y).is_some());
    assert!(chart.timeline_mark_hit_at(bx, y).is_some());
    chart.input_pointer_down(at(ax, y), 1);
    chart.input_pointer_up(at(bx, y));
    assert!(activations(&mut chart).is_empty());
}

#[test]
fn hovering_a_token_answers_the_pointer_cursor_and_the_lane_hover() {
    let mut chart = marked_chart(vec![mark("a", 30, "earnings")]);
    let (x, y) = token_center(&chart, 30);
    chart.input_pointer_move(at(x, y), false);
    assert_eq!(chart.input_cursor(), ChartCursor::Pointer);
    assert_eq!(chart.input_hover(), ChartHover::TimelineMark);
    chart.input_pointer_move(at(x, y - 60.0), false);
    assert_ne!(chart.input_hover(), ChartHover::TimelineMark);
    assert_eq!(chart.input_cursor(), ChartCursor::Crosshair);
    chart.input_pointer_leave();
    assert_eq!(chart.input_hover(), ChartHover::None);
}

#[test]
fn the_title_tooltip_arms_through_input_tick_at_the_dwell_deadline() {
    let mut chart = marked_chart(vec![mark("a", 30, "earnings")]);
    let (x, y) = token_center(&chart, 30);
    let mut frame = ChartFrame::default();
    let mut axis = Vec::new();
    let mut prepare = |chart: &mut ChartEngine, frame: &mut ChartFrame| {
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
                    frame,
                    axis_frame: None,
                    axis_primitives: Some(&mut axis),
                },
                |_, _| 48.0,
                |_, _| 48.0,
            )
            .frame_built
    };
    let shows = |frame: &ChartFrame| {
        frame.panes[0]
            .main
            .iter()
            .any(|prim| matches!(prim, Prim::Text { text, .. } if text == "Earnings · a title"))
    };
    assert!(prepare(&mut chart, &mut frame));
    chart.input_pointer_move(at_ms(x, y, 1_000.0), false);
    let deadline = 1_000.0 + TRADING_TOOLTIP_DWELL_MS;
    assert_eq!(chart.input_wake_deadline_ms(), Some(deadline));
    prepare(&mut chart, &mut frame);
    assert!(!shows(&frame), "contact alone shows nothing");
    assert!(!chart.input_tick(deadline - 1.0));
    assert!(
        !prepare(&mut chart, &mut frame),
        "an early tick changes nothing"
    );
    assert!(chart.input_tick(deadline));
    assert_eq!(chart.input_wake_deadline_ms(), None);
    assert!(prepare(&mut chart, &mut frame));
    assert!(shows(&frame));
    assert!(!chart.input_tick(deadline + 500.0), "the dwell fires once");
    // Holding still keeps it; moving off the token disarms it.
    chart.input_pointer_move(at_ms(x + 1.0, y, deadline + 600.0), false);
    assert!(prepare(&mut chart, &mut frame) || shows(&frame));
    assert!(shows(&frame));
    chart.input_pointer_move(at_ms(x, y - 80.0, deadline + 700.0), false);
    assert!(prepare(&mut chart, &mut frame));
    assert!(!shows(&frame));
    assert_eq!(chart.input_wake_deadline_ms(), None);
}

/// An order line crossing the lane changes the trading hover while the token under the pointer
/// stays the same; the token's pending dwell must survive that change.
#[test]
fn a_trading_hover_change_under_the_same_token_keeps_the_dwell() {
    let mut chart = marked_chart(vec![mark("a", 30, "earnings")]);
    let (x, y) = token_center(&chart, 30);
    // A protection order whose plain line (no control under the token's x) runs through the
    // lower half of the token box.
    let price = chart
        .trading_coordinate_to_price(0, TradingPriceScale::Right, y + 4.0)
        .unwrap();
    chart
        .set_trading_snapshot(TradingSnapshot {
            positions: vec![crate::trading::tests::position(PositionSide::Long)],
            orders: vec![crate::trading::tests::order(
                "tp-1",
                OrderRole::TakeProfit,
                price,
            )],
            ..TradingSnapshot::default()
        })
        .unwrap();
    chart.build_frame();
    assert!(
        chart.trading_hit_at(x, y + 4.0).is_some(),
        "the line crosses the token"
    );
    assert!(
        chart.trading_hit_at(x, y - 6.0).is_none(),
        "the upper half is lane only"
    );
    assert!(chart.timeline_mark_hit_at(x, y - 6.0).is_some());
    assert!(chart.timeline_mark_hit_at(x, y + 4.0).is_some());

    chart.input_pointer_move(at_ms(x, y - 6.0, 1_000.0), false);
    let deadline = 1_000.0 + TRADING_TOOLTIP_DWELL_MS;
    assert_eq!(chart.input_wake_deadline_ms(), Some(deadline));
    assert_eq!(chart.input_hover(), ChartHover::TimelineMark);
    // Onto the order line without leaving the token: the trading hover changes, the token does
    // not, and the dwell keeps its deadline.
    chart.input_pointer_move(at_ms(x, y + 4.0, 1_100.0), false);
    assert!(chart.trading_state.feedback_hover.is_some());
    assert_eq!(chart.input_hover(), ChartHover::TimelineMark);
    assert_eq!(chart.input_wake_deadline_ms(), Some(deadline));
    assert!(
        chart.input_tick(deadline),
        "the title tooltip arms at the deadline"
    );
    assert!(chart.timeline_marks.tooltip_armed);
    // Back off the line, still on the token: the armed tooltip stays and no new dwell starts.
    chart.input_pointer_move(at_ms(x, y - 6.0, deadline + 50.0), false);
    assert!(chart.timeline_marks.tooltip_armed);
    assert_eq!(chart.input_wake_deadline_ms(), None);
    // Leaving the token clears both.
    chart.input_pointer_move(at_ms(x, y - 60.0, deadline + 100.0), false);
    assert!(!chart.timeline_marks.tooltip_armed);
    assert_eq!(chart.input_wake_deadline_ms(), None);
}

#[test]
fn a_double_click_on_a_token_never_opens_the_editor_under_it() {
    let mut chart = marked_chart(vec![mark("a", 30, "earnings")]);
    let (x, y) = token_center(&chart, 30);
    let id = chart
        .add_drawing(
            DrawingKind::Rectangle,
            0,
            vec![
                DrawingPoint {
                    logical: 2.0,
                    price: 80.0,
                },
                DrawingPoint {
                    logical: 58.0,
                    price: 120.0,
                },
            ],
            Some(r#"{"text":"guarded"}"#),
        )
        .unwrap();
    chart.set_selected_drawing(Some(id));
    chart.build_frame();
    assert_eq!(
        chart.drawing_at(x, y),
        Some(id),
        "the text box lies under the token"
    );
    let sample = |timestamp_ms| PointerInput {
        timestamp_ms,
        ..at(x, y)
    };
    chart.input_pointer_down(sample(100.0), 1);
    chart.input_pointer_up(sample(101.0));
    assert_eq!(activations(&mut chart).len(), 1);
    chart.input_pointer_down(sample(200.0), 2);
    chart.input_pointer_up(sample(201.0));
    assert_eq!(chart.editing_drawing(), None);
    assert!(!chart
        .take_input_events()
        .iter()
        .any(|event| matches!(event, ChartInputEvent::TextEditorOpened(_))));
    assert_eq!(
        chart.selected_drawing(),
        Some(id),
        "the selection survives the pair"
    );
}

#[test]
fn a_token_click_keeps_the_active_trading_group() {
    use crate::trading::TradingGroupVisualState;
    let mut chart = marked_chart(vec![mark("a", 30, "earnings")]);
    chart
        .set_trading_snapshot(TradingSnapshot {
            positions: vec![crate::trading::tests::position(PositionSide::Long)],
            orders: vec![crate::trading::tests::order(
                "tp-1",
                OrderRole::TakeProfit,
                103.0,
            )],
            ..TradingSnapshot::default()
        })
        .unwrap();
    let group = chart
        .trading_group_key_for_order(&chart.trading_snapshot().orders[0])
        .expect("the protection order belongs to a group");
    chart.trading_state.group_visual = TradingGroupVisualState::Active(group.clone());
    let (x, y) = token_center(&chart, 30);
    click(&mut chart, x, y);
    assert_eq!(activations(&mut chart).len(), 1);
    assert_eq!(
        chart.trading_state.group_visual,
        TradingGroupVisualState::Active(group),
        "an engine control never clears the active order group"
    );
    // A bare pane click does.
    click(&mut chart, x, y - 150.0);
    assert_eq!(
        chart.trading_state.group_visual,
        TradingGroupVisualState::Inactive
    );
}

#[test]
fn a_hidden_lane_leaves_every_input_to_the_pane() {
    let mut chart = marked_chart(vec![mark("a", 30, "earnings")]);
    chart.set_timeline_marks_visible(false);
    relayout(&mut chart);
    let (x, y) = token_center(&chart, 30);
    chart.input_pointer_move(at(x, y), false);
    assert_ne!(chart.input_hover(), ChartHover::TimelineMark);
    click(&mut chart, x, y);
    let events = chart.take_input_events();
    assert!(events
        .iter()
        .any(|event| matches!(event, ChartInputEvent::Click { .. })));
    assert!(!events
        .iter()
        .any(|event| matches!(event, ChartInputEvent::TimelineMarkActivated(_))));
    assert_eq!(BARS, 60);
}

#[test]
fn a_touch_press_widens_the_token_box_through_the_real_input_path() {
    let mut chart = marked_chart(vec![mark("a", 30, "earnings")]);
    let (x, y) = token_center(&chart, 30);
    // 20 px above the center is outside the 16 px token box but inside the touch slack.
    let above = y - 20.0;
    let touch = |x: f64, y: f64| PointerInput {
        device: InputDevice::Touch,
        ..at(x, y)
    };
    chart.input_pointer_down(touch(x, above), 1);
    chart.input_pointer_up(touch(x, above));
    assert_eq!(
        activations(&mut chart).len(),
        1,
        "a touch tap inside the widened box activates the token"
    );
    // The same geometry with a mouse stays a plain pane click: the precision box ends 8 px from
    // the center, so no activation is emitted and the pane reports the click.
    click(&mut chart, x, above);
    let events = chart.take_input_events();
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, ChartInputEvent::TimelineMarkActivated(_))),
        "a mouse press 20 px above the token never arbitrates to the lane"
    );
    assert!(
        events
            .iter()
            .any(|event| matches!(event, ChartInputEvent::Click { .. })),
        "the pane answers the mouse click instead"
    );
}
