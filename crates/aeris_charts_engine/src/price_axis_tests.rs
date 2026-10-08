//! Price-axis semantics through the public engine API: min-move tick grids, tick-size ladders,
//! derived precision, explicit/symmetric bases, stable autoscale, and replacing autoscale hooks.

use super::*;
use aeris_charts_core::scale::price_tick_span_calculator::is_multiple_of;

/// HKEX spread table (Part A) as the price-format JSON ladder.
const HKEX_LADDER: &str = r#"[
    {"from": 0, "min_move": 0.001}, {"from": 0.25, "min_move": 0.005},
    {"from": 0.5, "min_move": 0.01}, {"from": 10, "min_move": 0.02},
    {"from": 20, "min_move": 0.05}, {"from": 100, "min_move": 0.1},
    {"from": 200, "min_move": 0.2}, {"from": 500, "min_move": 0.5},
    {"from": 1000, "min_move": 1}, {"from": 2000, "min_move": 2},
    {"from": 5000, "min_move": 5}
]"#;

fn measure(text: &str, _bold: bool) -> f64 {
    text.len() as f64 * 7.0
}

/// A chart whose series 0 holds `bars` OHLC bars oscillating around `center`.
fn chart_with_bars(
    width: f64,
    height: f64,
    center: f64,
    amplitude: f64,
    bars: usize,
) -> ChartEngine {
    let mut chart = ChartEngine::new(width, height, 1.0);
    let times: Vec<f64> = (0..bars)
        .map(|index| 1_000.0 + index as f64 * 60.0)
        .collect();
    let wave =
        |index: usize, phase: f64| center + amplitude * ((index as f64 * 0.37 + phase).sin());
    let open: Vec<f64> = (0..bars).map(|index| wave(index, 0.0)).collect();
    let close: Vec<f64> = (0..bars).map(|index| wave(index, 0.5)).collect();
    let high: Vec<f64> = (0..bars)
        .map(|index| open[index].max(close[index]) + amplitude * 0.1)
        .collect();
    let low: Vec<f64> = (0..bars)
        .map(|index| open[index].min(close[index]) - amplitude * 0.1)
        .collect();
    chart
        .set_series_data(0, &times, &open, &high, &low, &close)
        .unwrap();
    chart.time_scale.set_width(width);
    chart.fit_content();
    chart
}

/// Right-scale tick label texts (plain, unboxed, left-aligned labels on the right strip). A
/// tick whose text a boxed tag covers keeps its slot with empty text and is not a label.
fn tick_labels(chart: &mut ChartEngine) -> Vec<String> {
    chart.build_frame();
    chart
        .build_axis_frame(80.0, measure, measure)
        .labels
        .iter()
        .filter(|label| {
            label.background.is_none()
                && label.align == AxisTextAlign::Left
                && !label.text.is_empty()
        })
        .map(|label| label.text.clone())
        .collect()
}

fn parse_label(label: &str) -> f64 {
    label.replace(',', "").parse().unwrap()
}

fn right_range(chart: &ChartEngine) -> (f64, f64) {
    chart
        .price_scale_visible_range_for(0, PriceScaleTarget::Right)
        .unwrap()
}

#[test]
fn hk_min_move_ticks_are_tradable_prices_at_every_pane_height() {
    // The audit's HK$15 case: a 0.02 tick used to label 15.25 as "15.26".
    for (min_move, center) in [
        (0.02, 15.0),
        (0.05, 25.0),
        (0.005, 0.4),
        (0.2, 300.0),
        (5.0, 7_000.0),
    ] {
        for height in [240.0, 400.0, 555.0, 900.0] {
            let mut chart = chart_with_bars(800.0, height, center, center * 0.04, 60);
            assert!(chart.series_apply_price_format_json(
                0,
                &format!(r#"{{"type":"price","min_move":{min_move}}}"#)
            ));
            let labels = tick_labels(&mut chart);
            assert!(labels.len() >= 3, "{min_move} @ {height}: {labels:?}");
            for label in &labels {
                assert!(
                    is_multiple_of(parse_label(label), min_move),
                    "min_move {min_move} height {height}: off-grid label {label} in {labels:?}"
                );
            }
            let unique: std::collections::BTreeSet<_> = labels.iter().collect();
            assert_eq!(unique.len(), labels.len(), "duplicate labels {labels:?}");
        }
    }
}

#[test]
fn precision_derives_from_min_move_when_omitted() {
    let mut chart = chart_with_bars(800.0, 400.0, 0.5, 0.01, 40);
    assert!(chart.series_apply_price_format_json(0, r#"{"type":"price","min_move":0.0001}"#));
    let options: serde_json::Value =
        serde_json::from_str(&chart.series_options_json(0).unwrap()).unwrap();
    assert_eq!(options["price_format"]["precision"], 4);
    let labels = tick_labels(&mut chart);
    assert!(
        labels
            .iter()
            .all(|label| label.split('.').nth(1).map(str::len) == Some(4))
    );
    // An explicit precision still wins, and a precision-only patch keeps the move.
    assert!(
        chart
            .series_apply_price_format_json(0, r#"{"type":"price","precision":3,"min_move":0.05}"#)
    );
    let options: serde_json::Value =
        serde_json::from_str(&chart.series_options_json(0).unwrap()).unwrap();
    assert_eq!(options["price_format"]["precision"], 3);
    assert_eq!(options["price_format"]["min_move"], 0.05);
}

#[test]
fn tick_ladder_rounds_each_label_to_its_band_and_crossing_views_use_the_common_grid() {
    // A view crossing the HK$10 boundary: 0.01 below, 0.02 above.
    let mut chart = ChartEngine::new(800.0, 400.0, 1.0);
    let times: Vec<f64> = (0..40).map(|index| 1_000.0 + index as f64 * 60.0).collect();
    let close: Vec<f64> = (0..40).map(|index| 9.8 + index as f64 * 0.015).collect();
    chart
        .set_series_data(0, &times, &close, &close, &close, &close)
        .unwrap();
    chart.time_scale.set_width(800.0);
    chart.fit_content();
    assert!(chart.series_apply_price_format_json(
        0,
        &format!(r#"{{"type":"price","tick_ladder":{HKEX_LADDER}}}"#)
    ));
    let labels = tick_labels(&mut chart);
    assert!(labels.len() >= 3, "{labels:?}");
    let values: Vec<f64> = labels.iter().map(|label| parse_label(label)).collect();
    assert!(values.iter().any(|value| *value < 10.0) && values.iter().any(|value| *value > 10.0));
    for (label, value) in labels.iter().zip(&values) {
        // Every tick is on the 0.02 grid, which is tradable on both sides of HK$10.
        assert!(is_multiple_of(*value, 0.02), "{label} in {labels:?}");
        assert_eq!(label.split('.').nth(1).map(str::len), Some(2), "{label}");
    }
    // Last-value label: the final close 10.385 rounds to the 0.02 band tick.
    let axis = chart.build_axis_frame(80.0, measure, measure);
    assert!(
        axis.labels
            .iter()
            .any(|label| label.background.is_some() && label.text == "10.38"),
        "{:?}",
        axis.labels
            .iter()
            .map(|label| &label.text)
            .collect::<Vec<_>>()
    );
    // The ladder round-trips through the options JSON and `null` clears it.
    let options: serde_json::Value =
        serde_json::from_str(&chart.series_options_json(0).unwrap()).unwrap();
    assert_eq!(
        options["price_format"]["tick_ladder"]
            .as_array()
            .unwrap()
            .len(),
        11
    );
    assert!(!chart.series_apply_price_format_json(
        0,
        r#"{"type":"price","tick_ladder":[{"from":1,"min_move":0.01},{"from":0.5,"min_move":0.01}]}"#
    ));
    assert!(chart.series_apply_price_format_json(0, r#"{"type":"price","tick_ladder":null}"#));
    let options: serde_json::Value =
        serde_json::from_str(&chart.series_options_json(0).unwrap()).unwrap();
    assert!(options["price_format"].get("tick_ladder").is_none());
}

#[test]
fn us_sub_dollar_ladder_prints_four_decimals_below_one_dollar() {
    let mut chart = chart_with_bars(800.0, 400.0, 0.52, 0.01, 40);
    assert!(chart.series_apply_price_format_json(
        0,
        r#"{"type":"price","tick_ladder":[{"from":0,"min_move":0.0001},{"from":1,"min_move":0.01}]}"#
    ));
    let labels = tick_labels(&mut chart);
    assert!(!labels.is_empty());
    assert!(
        labels
            .iter()
            .all(|label| label.split('.').nth(1).map(str::len) == Some(4))
    );
}

#[test]
fn log_scale_ladder_keeps_fine_ticks_in_the_low_bands() {
    // Decade-spanning history (HK$0.30 -> HK$3,000) on a log scale: the whole view's common grid
    // is HK$10, but every lower band must still be labelled on its own finer tick.
    let mut chart = ChartEngine::new(800.0, 600.0, 1.0);
    let bars = 200;
    let times: Vec<f64> = (0..bars)
        .map(|index| 1_000.0 + index as f64 * 60.0)
        .collect();
    let close: Vec<f64> = (0..bars)
        .map(|index| 0.3 * 10_000f64.powf(index as f64 / (bars - 1) as f64))
        .collect();
    chart
        .set_series_data(0, &times, &close, &close, &close, &close)
        .unwrap();
    chart.time_scale.set_width(800.0);
    chart.fit_content();
    chart.set_price_scale_mode_for(0, PriceScaleTarget::Right, PriceScaleMode::Logarithmic);
    assert!(chart.series_apply_price_format_json(
        0,
        &format!(r#"{{"type":"price","tick_ladder":{HKEX_LADDER}}}"#)
    ));
    let ladder = chart
        .series_entry(0)
        .unwrap()
        .price_format
        .tick_ladder
        .clone();
    let ladder = ladder.unwrap();
    let labels = tick_labels(&mut chart);
    let values: Vec<f64> = labels.iter().map(|label| parse_label(label)).collect();
    assert!(
        values.iter().filter(|value| **value < 10.0).count() >= 3,
        "the low bands lost their ticks: {labels:?}"
    );
    for (label, value) in labels.iter().zip(&values) {
        assert!(
            is_multiple_of(*value, ladder.min_move_at(*value)),
            "{label} is not a tradable price in {labels:?}"
        );
    }
    let unique: std::collections::BTreeSet<_> = labels.iter().collect();
    assert_eq!(unique.len(), labels.len(), "{labels:?}");
}

#[test]
fn trading_snaps_and_steps_on_the_series_tick_ladder() {
    let mut chart = ChartEngine::new(400.0, 240.0, 1.0);
    chart
        .set_series_data(
            0,
            &[10.0, 20.0, 30.0],
            &[9.9, 10.0, 10.1],
            &[10.2, 10.3, 10.4],
            &[9.7, 9.8, 9.9],
            &[10.0, 10.1, 10.2],
        )
        .unwrap();
    chart.time_scale.set_width(400.0);
    // A scalar instrument tick of 0.01 is configured, but the ladder owns band ticks.
    chart
        .set_instrument_metadata(InstrumentMetadata {
            tick_size: Some(0.01),
            ..InstrumentMetadata::default()
        })
        .unwrap();
    assert!(chart.series_apply_price_format_json(
        0,
        &format!(r#"{{"type":"price","tick_ladder":{HKEX_LADDER}}}"#)
    ));
    let order_id = OrderId::new("working-1").unwrap();
    chart
        .set_trading_snapshot(TradingSnapshot {
            orders: vec![WorkingOrder {
                id: order_id.clone(),
                account_id: None,
                pane_index: 0,
                price_scale: TradingPriceScale::Right,
                side: OrderSide::Buy,
                kind: OrderKind::Limit,
                role: OrderRole::Working,
                status: OrderStatus::Working,
                price: 9.99,
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
            }],
            ..TradingSnapshot::default()
        })
        .unwrap();
    chart.build_frame();
    assert!(chart.trading_keyboard_start_order(&order_id));
    // 9.99 -> 10.00 (0.01 band) -> 10.02 (0.02 band): exact band ticks across the boundary.
    assert!(chart.trading_keyboard_adjust(1));
    assert!(chart.trading_keyboard_adjust(1));
    let intent = chart
        .trading_keyboard_commit()
        .expect("moved order emits an intent");
    assert!((intent.price.unwrap() - 10.02).abs() < 1e-12);
    // 0..0.25: 250 ticks, 0.25..0.5: 50, 0.5..10: 950, then one 0.02 tick.
    assert_eq!(intent.price_tick_index, Some(1251));
}

#[test]
fn explicit_percentage_base_replaces_the_first_visible_bar_for_series_and_drawings() {
    let mut chart = chart_with_bars(800.0, 400.0, 105.0, 3.0, 60);
    chart.set_price_scale_mode_for(0, PriceScaleTarget::Right, PriceScaleMode::Percentage);
    assert!(chart.price_scale_apply_options_json(
        0,
        PriceScaleTarget::Right,
        r#"{"base_value":100}"#
    ));
    chart.build_frame();
    let (low, high) = right_range(&chart);
    // Data spans roughly 101.7..108.3 => +1.7%..+8.3% against the explicit 100 base.
    assert!(
        low > 0.5 && low < 3.0 && high > 7.0 && high < 10.0,
        "{low}..{high}"
    );
    // Panning horizontally no longer re-bases the scale.
    let before = chart.series_base_value(0, 0).unwrap();
    chart.set_visible_logical_range(20.0, 50.0);
    chart.build_frame();
    assert_eq!(before, 100.0);
    assert_eq!(chart.series_base_value(0, 20).unwrap(), 100.0);
    assert_eq!(
        chart.drawing_scale_base_for(0, DrawingPriceScale::Right),
        100.0
    );
    // A drawing at 110 sits exactly where +10% sits on the axis.
    let scale = chart.price_scale_for(0, PriceScaleTarget::Right).unwrap();
    let y_drawing = scale.price_to_coordinate(
        110.0,
        chart.drawing_scale_base_for(0, DrawingPriceScale::Right),
    );
    assert!((y_drawing - scale.logical_to_coordinate(10.0)).abs() < 1e-9);
    let options: serde_json::Value = serde_json::from_str(
        &chart
            .price_scale_options_json(0, PriceScaleTarget::Right)
            .unwrap(),
    )
    .unwrap();
    assert_eq!(options["base_value"], 100.0);
    // Zero and non-finite bases are rejected; `null` clears.
    assert!(!chart.set_price_scale_base_value_for(0, PriceScaleTarget::Right, Some(0.0)));
    assert!(chart.price_scale_apply_options_json(
        0,
        PriceScaleTarget::Right,
        r#"{"base_value":null}"#
    ));
    assert_eq!(
        chart
            .price_scale_for(0, PriceScaleTarget::Right)
            .unwrap()
            .options()
            .base_value,
        None
    );
}

#[test]
fn symmetric_autoscale_centers_normal_and_percentage_scales() {
    let mut chart = chart_with_bars(800.0, 400.0, 102.5, 1.0, 40);
    assert!(chart.set_price_scale_autoscale_center_for(0, PriceScaleTarget::Right, Some(100.0)));
    chart.build_frame();
    let (low, high) = right_range(&chart);
    assert!(((low + high) / 2.0 - 100.0).abs() < 1e-9, "{low}..{high}");
    assert!(high > 103.0);
    // Percentage mode against the previous close: symmetric around 0%.
    chart.set_price_scale_mode_for(0, PriceScaleTarget::Right, PriceScaleMode::Percentage);
    assert!(chart.set_price_scale_base_value_for(0, PriceScaleTarget::Right, Some(100.0)));
    chart.build_frame();
    let (low, high) = right_range(&chart);
    assert!(((low + high) / 2.0).abs() < 1e-9, "{low}..{high}");
    assert!(high > 3.0);
}

/// A flat market with one spike bar at `spike`; returns the chart in stable or exact mode.
fn spike_chart(stable: bool) -> ChartEngine {
    let bars = 80;
    let spike = 50;
    let mut chart = ChartEngine::new(600.0, 400.0, 1.0);
    let times: Vec<f64> = (0..bars)
        .map(|index| 1_000.0 + index as f64 * 60.0)
        .collect();
    let base: Vec<f64> = (0..bars).map(|index| 100.0 + (index % 3) as f64).collect();
    let high: Vec<f64> = (0..bars)
        .map(|index| {
            if index == spike {
                140.0
            } else {
                base[index] + 1.0
            }
        })
        .collect();
    let low: Vec<f64> = base.iter().map(|value| value - 1.0).collect();
    chart
        .set_series_data(0, &times, &base, &high, &low, &base)
        .unwrap();
    chart.time_scale.set_width(600.0);
    if stable {
        assert!(chart.price_scale_apply_options_json(
            0,
            PriceScaleTarget::Right,
            r#"{"stable_auto_scale":true}"#
        ));
    }
    chart
}

/// Pan so the visible logical range ends at `right` (20 px bars: 30 bars in view), like a drag or
/// kinetic coast moving the right offset by fractional bars.
fn pan_to(chart: &mut ChartEngine, right: f64) {
    chart.set_bar_spacing(20.0);
    chart.set_right_offset(right - 79.0);
}

/// Pan back and forth by sub-bar amounts across the spike bar's right edge and record the
/// distinct autoscaled ranges.
fn ranges_while_panning(chart: &mut ChartEngine) -> Vec<(f64, f64)> {
    let mut ranges = Vec::new();
    for step in 0..24 {
        // Right edge alternates between 48.8 (spike excluded) and 49.2 (spike bar enters the
        // strict range), drifting by a hundredth of a bar per round trip.
        let right = if step % 2 == 0 { 48.8 } else { 49.2 } + (step / 2) as f64 * 0.01;
        pan_to(chart, right);
        chart.build_frame();
        let range = right_range(chart);
        if ranges.last() != Some(&range) {
            ranges.push(range);
        }
    }
    ranges
}

#[test]
fn stable_autoscale_does_not_oscillate_on_sub_bar_pans() {
    // Exact reference behavior flips on every sub-bar crossing.
    let mut exact = spike_chart(false);
    let exact_ranges = ranges_while_panning(&mut exact);
    assert!(
        exact_ranges.len() > 10,
        "exact mode should flip: {exact_ranges:?}"
    );
    // Stable mode grows once when the spike first shows and then holds.
    let mut stable = spike_chart(true);
    let stable_ranges = ranges_while_panning(&mut stable);
    assert!(
        stable_ranges.len() <= 2,
        "stable mode oscillated: {stable_ranges:?}"
    );
    let (_, high) = *stable_ranges.last().unwrap();
    assert!(high >= 140.0);
    // Panning the spike far out of view shrinks back to the flat market.
    pan_to(&mut stable, 35.0);
    stable.build_frame();
    assert!(right_range(&stable).1 < 110.0);
    // Data replacement restarts from the exact range.
    pan_to(&mut stable, 49.2);
    stable.build_frame();
    assert!(right_range(&stable).1 >= 140.0);
    let times: Vec<f64> = (0..80).map(|index| 1_000.0 + index as f64 * 60.0).collect();
    let flat = vec![100.0; 80];
    stable
        .set_series_data(0, &times, &flat, &flat, &flat, &flat)
        .unwrap();
    pan_to(&mut stable, 49.2);
    stable.build_frame();
    assert!(right_range(&stable).1 < 101.0, "{:?}", right_range(&stable));
}

#[test]
fn stable_autoscale_refits_exactly_on_structural_changes() {
    // Series 0 spans 99..103 and a second source reaches 103.5: without it the range leaves only
    // 10% unused, inside the hysteresis band, so only a structural restart refits it.
    let mut chart = spike_chart_without_spike();
    let times: Vec<f64> = (0..80).map(|index| 1_000.0 + index as f64 * 60.0).collect();
    let second = chart.add_series(SeriesKind::Line);
    let level = vec![103.5; 80];
    chart
        .set_series_data(second, &times, &level, &level, &level, &level)
        .unwrap();
    chart.fit_content();
    chart.build_frame();
    assert_eq!(right_range(&chart), (99.0, 103.5));
    chart.set_series_visible(second, false);
    chart.build_frame();
    assert_eq!(right_range(&chart), (99.0, 103.0), "hiding a source refits");
    chart.set_series_visible(second, true);
    chart.build_frame();
    assert_eq!(right_range(&chart), (99.0, 103.5));
    chart.set_series_price_scale(second, PriceScaleTarget::Left);
    chart.build_frame();
    assert_eq!(
        right_range(&chart),
        (99.0, 103.0),
        "rebinding a source refits"
    );
    chart.set_series_price_scale(second, PriceScaleTarget::Right);
    chart.build_frame();
    assert_eq!(right_range(&chart), (99.0, 103.5));
    assert!(chart.remove_series(second));
    chart.build_frame();
    assert_eq!(
        right_range(&chart),
        (99.0, 103.0),
        "removing a source refits"
    );

    // Replacing a source restarts every indicator output rebuilt from it, even on another pane.
    let ramp =
        |step: f64| -> Vec<f64> { (0..80).map(|index| 100.0 + index as f64 * step).collect() };
    let close = ramp(0.1);
    chart
        .set_series_data(0, &times, &close, &close, &close, &close)
        .unwrap();
    let sma = chart.add_sma(0, 3).unwrap();
    assert!(chart.try_set_series_pane(sma, 1, 1.0));
    assert!(chart.price_scale_apply_options_json(
        1,
        PriceScaleTarget::Right,
        r#"{"stable_auto_scale":true}"#
    ));
    chart.fit_content();
    chart.build_frame();
    let before = chart
        .price_scale_visible_range_for(1, PriceScaleTarget::Right)
        .unwrap();
    let close = ramp(0.095);
    chart
        .set_series_data(0, &times, &close, &close, &close, &close)
        .unwrap();
    chart.build_frame();
    let after = chart
        .price_scale_visible_range_for(1, PriceScaleTarget::Right)
        .unwrap();
    // The new SMA tops out 0.4 lower (a 5% shrink the hysteresis alone would keep).
    assert!(after.1 < before.1 - 0.3, "{before:?} -> {after:?}");
}

#[test]
fn stable_autoscale_refits_an_amount_weighted_vwap_when_its_turnover_is_replaced() {
    // The turnover (amount) column is a third indicator input next to the price and volume
    // sources: replacing it rebuilds the 分时 average price, so its stable scale must restart.
    let mut chart = spike_chart_without_spike();
    let times: Vec<f64> = (0..80).map(|index| 1_000.0 + index as f64 * 60.0).collect();
    let ramp =
        |step: f64| -> Vec<f64> { (0..80).map(|index| 100.0 + index as f64 * step).collect() };
    let close = ramp(0.1);
    chart
        .set_series_data(0, &times, &close, &close, &close, &close)
        .unwrap();
    let volume = chart.add_series(SeriesKind::Histogram);
    let ones = vec![1.0; 80];
    chart
        .set_series_data(volume, &times, &ones, &ones, &ones, &ones)
        .unwrap();
    let amount = chart.add_series(SeriesKind::Line);
    chart
        .set_series_data(amount, &times, &close, &close, &close, &close)
        .unwrap();
    let average = chart.add_vwap_with_amount(0, volume, amount).unwrap();
    assert!(chart.try_set_series_pane(average, 1, 1.0));
    assert!(chart.price_scale_apply_options_json(
        1,
        PriceScaleTarget::Right,
        r#"{"stable_auto_scale":true}"#
    ));
    chart.fit_content();
    chart.build_frame();
    let before = chart
        .price_scale_visible_range_for(1, PriceScaleTarget::Right)
        .unwrap();
    let turnover = ramp(0.095);
    chart
        .set_series_data(amount, &times, &turnover, &turnover, &turnover, &turnover)
        .unwrap();
    chart.build_frame();
    let after = chart
        .price_scale_visible_range_for(1, PriceScaleTarget::Right)
        .unwrap();
    // The average price now tops out about 0.2 lower: a 5% shrink the hysteresis alone keeps.
    assert!(after.1 < before.1 - 0.15, "{before:?} -> {after:?}");
}

/// [`spike_chart`] in stable mode with the spike bar flattened (99..103).
fn spike_chart_without_spike() -> ChartEngine {
    let mut chart = spike_chart(true);
    let times: Vec<f64> = (0..80).map(|index| 1_000.0 + index as f64 * 60.0).collect();
    let base: Vec<f64> = (0..80).map(|index| 100.0 + (index % 3) as f64).collect();
    let high: Vec<f64> = base.iter().map(|value| value + 1.0).collect();
    let low: Vec<f64> = base.iter().map(|value| value - 1.0).collect();
    chart
        .set_series_data(0, &times, &base, &high, &low, &base)
        .unwrap();
    chart
}

#[test]
fn autoscale_info_provider_replaces_the_series_range() {
    let mut chart = chart_with_bars(800.0, 400.0, 105.0, 3.0, 40);
    chart.build_frame();
    let own = right_range(&chart);
    let seen = std::rc::Rc::new(std::cell::Cell::new(None));
    let record = seen.clone();
    assert!(chart.set_series_autoscale_info_provider(
        0,
        Some(Box::new(move |base: Option<AutoscaleInfo>| {
            record.set(base.and_then(|info| info.price_range));
            Some(AutoscaleInfo {
                price_range: Some((0.0, 100.0)),
                margins: None,
            })
        })),
    ));
    chart.build_frame();
    // The base implementation saw the series' own data range...
    let (min, max) = seen.get().expect("provider received the series' own range");
    assert!(min > 100.0 && max < 110.0);
    // ...and the provider's answer replaced it rather than expanding it.
    let (low, high) = right_range(&chart);
    assert_eq!((low, high), (0.0, 100.0));
    assert!(own.0 > 90.0);
    // A pass-through provider reproduces the default range exactly.
    assert!(chart.set_series_autoscale_info_provider(0, Some(Box::new(|base| base))));
    chart.build_frame();
    assert_eq!(right_range(&chart), own);
    assert!(chart.set_series_autoscale_info_provider(0, None));
    assert!(!chart.series_has_autoscale_info_provider(0));
}

#[test]
fn indexed_to_100_labels_use_the_reference_fixed_formatter() {
    let mut chart = chart_with_bars(800.0, 400.0, 105.0, 0.3, 40);
    // An integer series format must not floor the 0.5-spaced indexed ticks.
    assert!(
        chart.series_apply_price_format_json(0, r#"{"type":"price","precision":0,"min_move":1}"#)
    );
    chart.set_price_scale_mode_for(0, PriceScaleTarget::Right, PriceScaleMode::IndexedTo100);
    let labels = tick_labels(&mut chart);
    assert!(labels.len() >= 3);
    assert!(
        labels
            .iter()
            .all(|label| label.split('.').nth(1).map(str::len) == Some(2)),
        "{labels:?}"
    );
    let unique: std::collections::BTreeSet<_> = labels.iter().collect();
    assert_eq!(unique.len(), labels.len(), "{labels:?}");
}

#[test]
fn chart_level_tick_options_reach_existing_and_new_panes() {
    let mut chart = chart_with_bars(800.0, 400.0, 105.0, 3.0, 40);
    chart
        .apply_options(
            r#"{"rightPriceScale":{"tickMarkDensity":4,"ensureEdgeTickMarksVisible":true}}"#,
        )
        .unwrap();
    let options = |chart: &ChartEngine, pane: usize| {
        let scale = chart
            .price_scale_for(pane, PriceScaleTarget::Right)
            .unwrap();
        (
            scale.options().tick_mark_density,
            scale.options().ensure_edge_tick_marks_visible,
        )
    };
    assert_eq!(options(&chart, 0), (4.0, true));
    let pane = chart.add_pane(true).unwrap();
    assert_eq!(options(&chart, pane), (4.0, true));
    // The left scale keeps the reference defaults.
    let left = chart.price_scale_for(0, PriceScaleTarget::Left).unwrap();
    assert_eq!(left.options().tick_mark_density, 2.5);
    assert!(!left.options().ensure_edge_tick_marks_visible);
}

#[test]
fn tick_density_and_edge_marks_are_price_scale_options() {
    let mut chart = chart_with_bars(800.0, 400.0, 105.0, 3.0, 40);
    let dense = {
        assert!(chart.price_scale_apply_options_json(
            0,
            PriceScaleTarget::Right,
            r#"{"tick_mark_density":1.5}"#
        ));
        tick_labels(&mut chart).len()
    };
    let sparse = {
        assert!(chart.set_price_scale_tick_mark_density_for(0, PriceScaleTarget::Right, 5.0));
        tick_labels(&mut chart).len()
    };
    assert!(dense > sparse, "{dense} vs {sparse}");
    assert!(!chart.set_price_scale_tick_mark_density_for(0, PriceScaleTarget::Right, 0.0));
    assert!(chart.price_scale_apply_options_json(
        0,
        PriceScaleTarget::Right,
        r#"{"tick_mark_density":2.5,"ensure_edge_tick_marks_visible":true,"scale_margins":{"top":0,"bottom":0}}"#
    ));
    chart.build_frame();
    let marks = chart.scale_tick_marks(0, PriceScaleTarget::Right, 0.0);
    assert!(marks.first().unwrap().edge && marks.last().unwrap().edge);
    let options: serde_json::Value = serde_json::from_str(
        &chart
            .price_scale_options_json(0, PriceScaleTarget::Right)
            .unwrap(),
    )
    .unwrap();
    assert_eq!(options["tick_mark_density"], 2.5);
    assert_eq!(options["ensure_edge_tick_marks_visible"], true);
    assert_eq!(options["stable_auto_scale"], false);
    assert!(options["autoscale_center"].is_null());
}
