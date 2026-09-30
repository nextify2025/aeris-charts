//! Headless-engine unit tests (extracted from `lib.rs`; `super` is the crate root).

use super::*;
use aeris_charts_render::canvas2d::{execute, Canvas2d, Viewport};
use aeris_charts_render::color::Color;

#[derive(Default)]
struct CountingCanvas {
    calls: usize,
}

impl Canvas2d for CountingCanvas {
    fn set_fill_solid(&mut self, _color: Color) {
        self.calls += 1;
    }
    fn set_fill_vgradient(&mut self, _y_top: f32, _y_bottom: f32, _top: Color, _bottom: Color) {
        self.calls += 1;
    }
    fn set_stroke(&mut self, _color: Color) {
        self.calls += 1;
    }
    fn set_line_width(&mut self, _width: f32) {
        self.calls += 1;
    }
    fn set_line_dash(&mut self, _pattern: &[f32]) {
        self.calls += 1;
    }
    fn fill_rect(&mut self, _x: f32, _y: f32, _w: f32, _h: f32) {
        self.calls += 1;
    }
    fn begin_path(&mut self) {
        self.calls += 1;
    }
    fn move_to(&mut self, _x: f32, _y: f32) {
        self.calls += 1;
    }
    fn line_to(&mut self, _x: f32, _y: f32) {
        self.calls += 1;
    }
    fn close_path(&mut self) {
        self.calls += 1;
    }
    fn arc(&mut self, _cx: f32, _cy: f32, _r: f32, _start: f32, _end: f32) {
        self.calls += 1;
    }
    fn stroke(&mut self) {
        self.calls += 1;
    }
    fn fill(&mut self) {
        self.calls += 1;
    }
    fn fill_rotated_text(
        &mut self,
        _: &str,
        _: f32,
        _: f32,
        _: &str,
        _: Color,
        _: aeris_charts_render::draw_list::TextAlign,
        _: f32,
    ) {
        self.calls += 1;
    }
}

#[test]
fn constructs_without_a_browser_or_gpu() {
    let chart = ChartEngine::new(800.0, 500.0, 2.0);
    assert_eq!(chart.series.len(), 1);
    assert_eq!(chart.panes.len(), 1);
    assert_eq!(chart.css_width, 800.0);
    assert_eq!(chart.dpr, 2.0);
}

#[test]
fn heikin_ashi_frame_projection_preserves_raw_queries() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    let times = [1.0, 2.0];
    let open = [10.0, 12.0];
    let high = [14.0, 16.0];
    let low = [8.0, 10.0];
    let close = [12.0, 14.0];
    chart
        .set_series_data(0, &times, &open, &high, &low, &close)
        .expect("valid candlestick data");
    assert!(chart.series_apply_options_json(0, r#"{"heikin_ashi":true}"#));
    chart.time_scale.fit_content();
    chart.build_frame();

    assert_eq!(chart.heikin_ashi_row(0, 0), Some([11.0, 14.0, 8.0, 11.0]));
    assert_eq!(chart.heikin_ashi_row(0, 1), Some([11.0, 16.0, 10.0, 13.0]));
    let (_, raw_columns) = chart.data.series_data(0).expect("raw series data");
    assert_eq!(raw_columns[0], &open);
    assert_eq!(raw_columns[1], &high);
    assert_eq!(raw_columns[2], &low);
    assert_eq!(raw_columns[3], &close);
}

/// Phase-0 compatibility sentinel for the all-in-one architecture. This deliberately uses the
/// real engine mutation and frame paths instead of testing future domain types in isolation: when
/// panes become domain-aware, the unchanged financial default must still compose the established
/// series, pane/scale, indicator, drawing, and interaction owners into one ordered frame.
#[test]
fn financial_product_compatibility_fixture_survives_shared_frame_mutations() {
    let mut chart = ChartEngine::new(960.0, 640.0, 1.0);
    let rows = 96;
    let times = (0..rows).map(|row| row as f64 * 60.0).collect::<Vec<_>>();
    let close = (0..rows)
        .map(|row| 100.0 + row as f64 * 0.1 + (row as f64 * 0.23).sin())
        .collect::<Vec<_>>();
    let open = close
        .iter()
        .enumerate()
        .map(|(row, value)| value - (row as f64 * 0.11).sin() * 0.4)
        .collect::<Vec<_>>();
    let high = open
        .iter()
        .zip(&close)
        .map(|(open, close)| open.max(*close) + 0.75)
        .collect::<Vec<_>>();
    let low = open
        .iter()
        .zip(&close)
        .map(|(open, close)| open.min(*close) - 0.75)
        .collect::<Vec<_>>();

    chart
        .set_series_data(0, &times, &open, &high, &low, &close)
        .expect("canonical candlestick fixture");
    let mut financial_series = vec![(0, SeriesKind::Candlestick)];
    for kind in [
        SeriesKind::Bar,
        SeriesKind::Line,
        SeriesKind::Area,
        SeriesKind::Histogram,
        SeriesKind::Baseline,
    ] {
        let id = chart.add_series(kind);
        chart
            .set_series_data(id, &times, &open, &high, &low, &close)
            .expect("valid financial series fixture");
        financial_series.push((id, kind));
    }

    let comparison_pane = chart.add_pane(true).expect("comparison pane identity");
    let comparison_scale = chart
        .add_price_scale(
            comparison_pane,
            "phase-zero-comparison",
            PriceScaleSide::Left,
            Some(0),
            true,
        )
        .expect("named comparison scale");
    let comparison = financial_series
        .iter()
        .find_map(|(id, kind)| (*kind == SeriesKind::Line).then_some(*id))
        .unwrap();
    assert!(chart.try_set_series_pane_and_scale(
        comparison,
        comparison_pane,
        0.6,
        "phase-zero-comparison",
    ));
    assert_eq!(
        chart.series_price_scale(comparison),
        Some((comparison_pane, comparison_scale))
    );

    let histogram = financial_series
        .iter()
        .find_map(|(id, kind)| (*kind == SeriesKind::Histogram).then_some(*id))
        .unwrap();
    assert!(chart.add_sma(0, 10).is_some());
    assert!(chart.add_ema(0, 12).is_some());
    assert_eq!(chart.add_ema_ribbon(0, [3, 5, 8, 13, 21]).len(), 5);
    assert_eq!(chart.add_bollinger(0, 20, 2.0).len(), 3);
    assert_eq!(chart.add_keltner(0, 20, 2.0).len(), 3);
    assert_eq!(chart.add_adx_dmi(0, 14).len(), 3);
    assert!(chart.add_parabolic_sar(0).is_some());
    assert!(chart.add_supertrend(0, 14, 3.0).is_some());
    assert_eq!(chart.add_ichimoku(0).len(), 5);
    assert!(chart.add_cci(0, 14).is_some());
    assert!(chart.add_rsi(0, 14).is_some());
    assert_eq!(chart.add_macd(0, 12, 26, 9).len(), 3);
    assert_eq!(chart.add_stochastic(0, 14, 3).len(), 2);
    assert!(chart.add_atr(0, 14).is_some());
    assert!(chart.add_vwap(0, Some(histogram)).is_some());
    assert!(chart.add_obv(0, histogram).is_some());
    assert!(chart.add_cmf(0, histogram, 14).is_some());
    assert!(chart.add_mfi(0, histogram, 14).is_some());
    assert_eq!(chart.add_volume(0, histogram, 14).len(), 2);
    assert!(chart.add_wma(0, 9).is_some());
    assert_eq!(
        chart
            .indicator_bindings()
            .into_iter()
            .map(|binding| binding.kind)
            .collect::<Vec<_>>(),
        vec![
            IndicatorKind::Sma { period: 10 },
            IndicatorKind::Ema {
                period: 12,
                seed: IndicatorSeed::Sma,
            },
            IndicatorKind::EmaRibbon {
                periods: [3, 5, 8, 13, 21],
            },
            IndicatorKind::Bollinger {
                period: 20,
                deviation: 2.0,
                estimator: DeviationEstimator::Population,
            },
            IndicatorKind::Keltner {
                period: 20,
                multiplier: 2.0,
            },
            IndicatorKind::AdxDmi { period: 14 },
            IndicatorKind::ParabolicSar,
            IndicatorKind::SuperTrend {
                period: 14,
                multiplier: 3.0,
            },
            IndicatorKind::Ichimoku,
            IndicatorKind::Cci { period: 14 },
            IndicatorKind::Rsi {
                period: 14,
                seed: IndicatorSeed::Sma,
            },
            IndicatorKind::Macd {
                fast: 12,
                slow: 26,
                signal: 9,
                seed: IndicatorSeed::Sma,
                histogram_multiplier: 1.0,
            },
            IndicatorKind::Stochastic {
                k_period: 14,
                d_period: 3,
            },
            IndicatorKind::Atr { period: 14 },
            IndicatorKind::Vwap,
            IndicatorKind::Obv,
            IndicatorKind::Cmf { period: 14 },
            IndicatorKind::Mfi { period: 14 },
            IndicatorKind::Volume { period: 14 },
            IndicatorKind::Wma { period: 9 },
        ]
    );

    let point = |logical, price| DrawingPoint { logical, price };
    let drawing_fixtures = [
        (
            DrawingKind::TrendLine,
            vec![point(8.0, 100.0), point(24.0, 106.0)],
        ),
        (DrawingKind::HorizontalLine, vec![point(0.0, 103.0)]),
        (DrawingKind::HorizontalRay, vec![point(20.0, 104.0)]),
        (DrawingKind::VerticalLine, vec![point(32.0, 0.0)]),
        (
            DrawingKind::Rectangle,
            vec![point(36.0, 101.0), point(48.0, 107.0)],
        ),
        (DrawingKind::Text, vec![point(52.0, 105.0)]),
        (
            DrawingKind::Brush,
            vec![point(56.0, 102.0), point(60.0, 104.0), point(64.0, 103.0)],
        ),
        (
            DrawingKind::Path,
            vec![point(66.0, 102.0), point(70.0, 105.0), point(74.0, 104.0)],
        ),
        (
            DrawingKind::LongPosition,
            vec![point(76.0, 104.0), point(76.0, 108.0), point(76.0, 101.0)],
        ),
        (
            DrawingKind::ShortPosition,
            vec![point(84.0, 105.0), point(84.0, 101.0), point(84.0, 108.0)],
        ),
    ];
    let drawing_kinds = drawing_fixtures.each_ref().map(|(kind, _)| *kind);
    for (kind, points) in drawing_fixtures {
        assert!(
            chart.add_drawing(kind, 0, points, None).is_some(),
            "{kind:?} fixture"
        );
    }

    chart.time_scale.set_width(960.0);
    chart.fit_content();
    let initial = chart.build_frame();
    assert_eq!(initial.panes.len(), chart.panes.len());
    assert!(initial.panes.iter().all(|pane| pane.height > 0.0));
    assert!(!initial.panes[0].main.is_empty());
    assert!(!initial.panes[comparison_pane].main.is_empty());

    chart.time_scale_start_scroll(400.0);
    chart.time_scale_scroll_to(430.0);
    chart.time_scale_end_scroll();
    chart.time_scale_zoom_focused(480.0, 1.1);
    chart.price_axis_start_scale(0, PriceScaleTarget::Right, 260.0);
    chart.price_axis_scale_to(0, PriceScaleTarget::Right, 230.0);
    chart.price_axis_end_scale(0, PriceScaleTarget::Right);
    assert!(chart.set_crosshair_position(close[48], times[48], 0));

    let mutated = chart.build_frame();
    assert_eq!(mutated.panes.len(), initial.panes.len());
    assert!(mutated.panes.iter().all(|pane| pane.height > 0.0));
    assert!(!mutated.panes[0].main.is_empty());
    assert!(!mutated.panes[comparison_pane].main.is_empty());
    assert_eq!(
        financial_series
            .iter()
            .map(|(id, expected)| {
                chart
                    .series
                    .iter()
                    .find(|series| series.id == *id)
                    .map(|series| (series.kind, *expected))
            })
            .collect::<Vec<_>>(),
        financial_series
            .iter()
            .map(|(_, expected)| Some((*expected, *expected)))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        chart
            .drawings
            .iter()
            .map(|drawing| drawing.kind)
            .collect::<Vec<_>>(),
        drawing_kinds
    );
}

#[test]
fn series_kind_selects_canonical_scalar_storage_without_collapsing_flat_ohlc() {
    let mut chart = ChartEngine::new(800.0, 600.0, 1.0);
    let times = [1.0, 2.0, 3.0];
    let values = [10.0, 10.0, 10.0];
    chart
        .set_series_data(0, &times, &values, &values, &values, &values)
        .unwrap();
    assert_eq!(
        chart
            .data_layer()
            .series_memory_usage(0)
            .unwrap()
            .canonical_value_bytes,
        3 * 4 * std::mem::size_of::<f64>()
    );

    let line = chart.add_series(SeriesKind::Line);
    chart
        .set_series_data(line, &times, &values, &values, &values, &values)
        .unwrap();
    assert_eq!(
        chart
            .data_layer()
            .series_memory_usage(line)
            .unwrap()
            .canonical_value_bytes,
        3 * std::mem::size_of::<f64>()
    );
}

#[test]
fn rsi_output_aliases_source_time_and_keeps_sparse_runtime() {
    let rows = 10_000;
    let times = (0..rows).map(|row| row as f64 * 60.0).collect::<Vec<_>>();
    let close = (0..rows)
        .map(|row| 100.0 + (row as f64 * 0.01).sin())
        .collect::<Vec<_>>();
    let open = close.clone();
    let high = close.iter().map(|value| value + 1.0).collect::<Vec<_>>();
    let low = close.iter().map(|value| value - 1.0).collect::<Vec<_>>();
    let mut chart = ChartEngine::new(800.0, 600.0, 1.0);
    chart
        .set_series_data(0, &times, &open, &high, &low, &close)
        .unwrap();
    let source_memory = chart.memory_usage();
    let output = chart.add_rsi(0, 14).unwrap();
    let memory = chart.memory_usage();
    let output_memory = chart.data_layer().series_memory_usage(output).unwrap();

    assert_eq!(output_memory.rows, rows - 14);
    assert_eq!(output_memory.owned_time_bytes, 0);
    assert_eq!(output_memory.plot_index_bytes, 0);
    assert_eq!(
        output_memory.canonical_value_bytes,
        (rows - 14) * std::mem::size_of::<f64>()
    );
    assert!(output_memory.aligned_time_view);
    assert_eq!(
        memory.data.merged_time_bytes,
        source_memory.data.merged_time_bytes
    );
    assert!(memory.indicator_runtime_bytes < 4 * 1024);
    assert_eq!(memory.indicator_transfer_capacity_bytes, 0);
}

#[test]
fn pane_layout_is_host_independent() {
    let mut pane = Pane::new();
    pane.top = 100.0;
    pane.height = 200.0;
    pane.layout();
    pane.price_scale.apply_autoscale_range(
        Some(aeris_charts_core::model::price_range::PriceRange::new(
            0.0, 2.0,
        )),
        0.01,
    );
    let y = pane.price_scale.price_to_coordinate(1.0, 1.0);
    assert!(y.is_finite() && (100.0..=300.0).contains(&y));
}

#[test]
fn ingests_data_without_a_host_runtime() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    let report = chart
        .set_series_data(
            0,
            &[3.0, 1.0, 2.0],
            &[12.0, 10.0, 11.0],
            &[13.0, 11.0, 12.0],
            &[9.0, 8.0, 10.0],
            &[11.0, 10.0, 11.5],
        )
        .unwrap();
    assert!(report.reordered);
    assert_eq!(chart.data.merged_times(), &[1, 2, 3]);
    chart.time_scale.set_width(800.0);
    chart.fit_content();
    assert!(chart.time_scale.visible_logical_range().is_some());
    let frame = chart.build_frame();
    assert_eq!(frame.panes.len(), 1);
    assert!(!frame.panes[0].main.is_empty());
}

fn installed_tick_weights(chart: &mut ChartEngine) -> Vec<u8> {
    let mut weights = chart
        .tick_marks
        .build(1.0, 0.0)
        .iter()
        .map(|mark| (mark.index as usize, mark.weight))
        .collect::<Vec<_>>();
    weights.sort_unstable_by_key(|(index, _)| *index);
    weights.into_iter().map(|(_, weight)| weight).collect()
}

#[test]
fn timestamp_replacement_rebuilds_weights_for_every_sequence_change() {
    use aeris_charts_core::scale::time_tick_marks::fill_weights_for_points;

    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    let values = [1.0; 4];
    let install = |chart: &mut ChartEngine, times: &[f64]| {
        chart
            .set_series_data(0, times, &values, &values, &values, &values)
            .unwrap();
    };
    let expected = |times: &[i64]| {
        let mut weights = vec![0; times.len()];
        fill_weights_for_points(times, &mut weights, 0);
        weights
    };

    install(&mut chart, &[0.0, 60.0, 120.0, 86_400.0]);
    assert_eq!(
        installed_tick_weights(&mut chart),
        expected(&[0, 60, 120, 86_400])
    );

    // Same length and endpoints, but different interior calendar boundaries.
    install(&mut chart, &[0.0, 3_600.0, 7_200.0, 86_400.0]);
    let interior = installed_tick_weights(&mut chart);
    assert_eq!(interior, expected(&[0, 3_600, 7_200, 86_400]));

    install(&mut chart, &[-60.0, 3_600.0, 7_200.0, 86_400.0]);
    assert_eq!(
        installed_tick_weights(&mut chart),
        expected(&[-60, 3_600, 7_200, 86_400])
    );

    install(&mut chart, &[-60.0, 3_600.0, 7_200.0, 172_800.0]);
    assert_eq!(
        installed_tick_weights(&mut chart),
        expected(&[-60, 3_600, 7_200, 172_800])
    );

    // Identical timestamps and a current-bar value replacement keep the time generation stable.
    let generation = chart.synced_time_points_generation;
    let before = installed_tick_weights(&mut chart);
    install(&mut chart, &[-60.0, 3_600.0, 7_200.0, 172_800.0]);
    assert_eq!(chart.synced_time_points_generation, generation);
    assert_eq!(installed_tick_weights(&mut chart), before);
    assert!(chart.update_series_bar(0, 172_800.0, [2.0; 4]));
    assert_eq!(chart.synced_time_points_generation, generation);
    assert_eq!(installed_tick_weights(&mut chart), before);

    assert!(chart.update_series_bar(0, 259_200.0, [3.0; 4]));
    assert_ne!(chart.synced_time_points_generation, generation);
    assert_eq!(
        installed_tick_weights(&mut chart),
        expected(&[-60, 3_600, 7_200, 172_800, 259_200])
    );
}

#[test]
fn series_primitive_autoscale_contribution_expands_the_owning_scale() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    chart
        .set_series_data(
            0,
            &[1.0, 2.0],
            &[5.0, 6.0],
            &[10.0, 9.0],
            &[0.0, 1.0],
            &[7.0, 8.0],
        )
        .unwrap();
    chart.time_scale.set_width(800.0);
    chart.fit_content();
    chart.autoscale_visible();
    let base = chart.panes[0].price_scale.price_range().unwrap();
    let base = (base.min_value(), base.max_value());
    assert_eq!(base, (0.0, 10.0));

    // A primitive on the series reaches past the data on both ends; the merged range unions in.
    chart.add_autoscale_contribution(PrimitiveAutoscaleContribution {
        series: 0,
        pane: 0,
        target: PriceScaleTarget::Right,
        min: -50.0,
        max: 60.0,
    });
    chart.autoscale_visible();
    let merged = chart.panes[0].price_scale.price_range().unwrap();
    assert_eq!((merged.min_value(), merged.max_value()), (-50.0, 60.0));

    // Contributions are per-frame: clearing them returns the scale to the data range.
    chart.clear_autoscale_contributions();
    chart.autoscale_visible();
    let restored = chart.panes[0].price_scale.price_range().unwrap();
    assert_eq!((restored.min_value(), restored.max_value()), base);
}

#[test]
fn series_primitive_autoscale_is_gated_on_owning_series_visibility() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    chart
        .set_series_data(
            0,
            &[1.0, 2.0],
            &[5.0, 6.0],
            &[10.0, 9.0],
            &[0.0, 1.0],
            &[7.0, 8.0],
        )
        .unwrap();
    chart.time_scale.set_width(800.0);
    chart.fit_content();

    // reference price-scale.ts `_recalculatePriceRangeImpl` skips invisible sources, and series.ts
    // merges primitive ranges into the series' own autoscale info — a hidden owning series
    // therefore silences its primitives' contributions.
    chart.set_series_visible(0, false);
    chart.add_autoscale_contribution(PrimitiveAutoscaleContribution {
        series: 0,
        pane: 0,
        target: PriceScaleTarget::Right,
        min: -50.0,
        max: 60.0,
    });
    chart.autoscale_visible();
    let range = chart.panes[0].price_scale.price_range();
    assert!(
        range.is_none_or(|r| r.max_value() <= 10.0),
        "hidden series must not contribute: {range:?}"
    );

    chart.set_series_visible(0, true);
    chart.add_autoscale_contribution(PrimitiveAutoscaleContribution {
        series: 0,
        pane: 0,
        target: PriceScaleTarget::Right,
        min: -50.0,
        max: 60.0,
    });
    chart.autoscale_visible();
    let merged = chart.panes[0].price_scale.price_range().unwrap();
    assert_eq!((merged.min_value(), merged.max_value()), (-50.0, 60.0));
}

#[test]
fn series_primitive_autoscale_routes_to_the_owning_scale() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    chart
        .set_series_data(
            0,
            &[1.0, 2.0],
            &[5.0, 6.0],
            &[10.0, 9.0],
            &[0.0, 1.0],
            &[7.0, 8.0],
        )
        .unwrap();
    let left = chart.add_series(SeriesKind::Line);
    chart
        .set_series_data(
            left,
            &[1.0, 2.0],
            &[100.0, 100.0],
            &[100.0, 100.0],
            &[100.0, 100.0],
            &[100.0, 100.0],
        )
        .unwrap();
    chart.set_series_price_scale(left, PriceScaleTarget::Left);
    chart.time_scale.set_width(800.0);
    chart.fit_content();

    // The left-bound series' primitive grows only the left scale; the right scale is untouched.
    chart.add_autoscale_contribution(PrimitiveAutoscaleContribution {
        series: left,
        pane: 0,
        target: PriceScaleTarget::Left,
        min: 0.0,
        max: 500.0,
    });
    chart.autoscale_visible();
    assert_eq!(
        chart
            .price_scale_visible_range_for(0, PriceScaleTarget::Left)
            .unwrap(),
        (0.0, 500.0)
    );
    assert_eq!(
        chart
            .price_scale_visible_range_for(0, PriceScaleTarget::Right)
            .unwrap(),
        (0.0, 10.0)
    );

    // A contribution recorded against a pane the series no longer occupies is stale and skipped.
    chart.clear_autoscale_contributions();
    chart.add_autoscale_contribution(PrimitiveAutoscaleContribution {
        series: left,
        pane: 3,
        target: PriceScaleTarget::Left,
        min: -999.0,
        max: 999.0,
    });
    chart.autoscale_visible();
    // (Flat 100 data yields the scale's degenerate ±0.05 range, not the stale contribution.)
    assert_eq!(
        chart
            .price_scale_visible_range_for(0, PriceScaleTarget::Left)
            .unwrap(),
        (99.95, 100.05)
    );
}

#[test]
fn series_primitive_autoscale_rejects_non_finite_bounds() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    chart
        .set_series_data(
            0,
            &[1.0, 2.0],
            &[5.0, 6.0],
            &[10.0, 9.0],
            &[0.0, 1.0],
            &[7.0, 8.0],
        )
        .unwrap();
    chart.time_scale.set_width(800.0);
    chart.fit_content();
    for (min, max) in [(f64::NAN, 10.0), (0.0, f64::INFINITY), (f64::NAN, f64::NAN)] {
        chart.add_autoscale_contribution(PrimitiveAutoscaleContribution {
            series: 0,
            pane: 0,
            target: PriceScaleTarget::Right,
            min,
            max,
        });
    }
    chart.autoscale_visible();
    assert_eq!(
        chart
            .price_scale_visible_range_for(0, PriceScaleTarget::Right)
            .unwrap(),
        (0.0, 10.0)
    );
}

#[test]
fn hidden_series_do_not_expand_autoscale() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    chart
        .set_series_data(
            0,
            &[1.0, 2.0],
            &[5.0, 6.0],
            &[10.0, 9.0],
            &[0.0, 1.0],
            &[7.0, 8.0],
        )
        .unwrap();
    let hidden = chart.add_series(SeriesKind::Line);
    chart
        .set_series_data(
            hidden,
            &[1.0, 2.0],
            &[1000.0, 1001.0],
            &[1000.0, 1001.0],
            &[1000.0, 1001.0],
            &[1000.0, 1001.0],
        )
        .unwrap();
    chart.set_series_visible(hidden, false);
    chart.time_scale.set_width(800.0);
    chart.fit_content();
    chart.autoscale_visible();
    assert_eq!(
        chart.panes[0]
            .price_scale
            .price_range()
            .unwrap()
            .max_value(),
        10.0
    );

    chart.set_series_visible(hidden, true);
    chart.autoscale_visible();
    assert_eq!(
        chart.panes[0]
            .price_scale
            .price_range()
            .unwrap()
            .max_value(),
        1001.0
    );
}

#[test]
fn marker_autoscale_margins_are_headless_and_can_be_disabled() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    chart
        .set_series_data(
            0,
            &[1.0, 2.0],
            &[100.0, 101.0],
            &[102.0, 103.0],
            &[99.0, 100.0],
            &[101.0, 102.0],
        )
        .unwrap();
    chart.set_series_markers(
        0,
        vec![Marker {
            time: 2,
            position: marker_pos::ABOVE,
            shape: marker_shape::CIRCLE,
            color: Color::rgb(0x21, 0x96, 0xf3),
            text: String::new(),
            id: String::new(),
            size: 1.0,
            price: None,
        }],
    );
    chart.time_scale.set_width(800.0);
    chart.fit_content();
    chart.build_frame();
    // Two fitted bars clamp marker geometry to the reference's maximum spacing bucket.
    assert_eq!(chart.panes[0].marker_margin_above, 48.0);
    assert_eq!(chart.panes[0].marker_margin_below, 0.0);

    chart.set_series_markers_auto_scale(0, false);
    chart.build_frame();
    assert_eq!(chart.panes[0].marker_margin_above, 0.0);
    assert_eq!(chart.panes[0].marker_margin_below, 0.0);
}

#[test]
fn public_time_scale_options_are_validated_and_headless() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    chart.time_scale.set_width(800.0);
    chart.set_bar_spacing(50.0);
    chart.set_right_offset(3.5);
    assert_eq!(chart.bar_spacing(), 50.0);
    assert_eq!(chart.right_offset(), 3.5);
    chart.set_bar_spacing(f64::NAN);
    chart.set_right_offset(f64::INFINITY);
    assert_eq!(chart.bar_spacing(), 50.0);
    assert_eq!(chart.right_offset(), 3.5);
}

#[test]
fn richer_time_scale_queries_and_mutations_are_headless() {
    let mut chart = ChartEngine::new(300.0, 200.0, 1.0);
    chart
        .set_series_data(
            0,
            &[10.0, 20.0, 30.0],
            &[1.0, 2.0, 3.0],
            &[1.0, 2.0, 3.0],
            &[1.0, 2.0, 3.0],
            &[1.0, 2.0, 3.0],
        )
        .unwrap();
    chart.time_scale.set_width(300.0);
    chart.fit_content();

    assert_eq!(chart.time_to_index(20.0, false), Some(1));
    assert_eq!(chart.time_to_index(15.0, false), None);
    assert_eq!(chart.time_to_index(15.0, true), Some(1));
    assert_eq!(chart.time_to_index(35.0, true), Some(2));
    let x = chart.logical_to_coordinate(1.0).unwrap();
    assert_eq!(chart.coordinate_to_logical(x), Some(1.0));
    assert_eq!(chart.logical_to_coordinate(1.25), Some(0.0));
    assert_eq!(
        chart.time_to_coordinate(20.0),
        chart.logical_to_coordinate(1.0)
    );
    assert_eq!(
        chart.coordinate_to_time(chart.logical_to_coordinate(2.0).unwrap()),
        Some(30.0)
    );

    chart.scroll_to_position(4.0);
    // The core clamps excessive future whitespace for a three-point data set.
    assert_eq!(chart.scroll_position(), 1.0);
    chart.scroll_to_real_time();
    assert_eq!(chart.scroll_position(), 0.0);
    chart.set_bar_spacing(20.0);
    chart.set_right_offset(2.0);
    chart.reset_time_scale();
    assert_eq!(chart.bar_spacing(), 6.0);
    assert_eq!(chart.right_offset(), 0.0);

    chart.set_visible_time_range(10.0, 20.0);
    assert_eq!(chart.visible_time_range(), Some((10.0, 20.0)));
}

#[test]
fn public_price_scale_state_is_headless_and_manual_ranges_survive_rendering() {
    let mut chart = ChartEngine::new(300.0, 200.0, 1.0);
    chart
        .set_series_data(
            0,
            &[10.0, 20.0, 30.0],
            &[100.0, 101.0, 102.0],
            &[101.0, 102.0, 103.0],
            &[99.0, 100.0, 101.0],
            &[100.5, 101.5, 102.5],
        )
        .unwrap();
    chart.time_scale.set_width(300.0);
    chart.layout_panes(172.0);
    chart.fit_content();
    chart.build_frame();
    assert_eq!(chart.price_scale_margins(0, false), Some((0.2, 0.1)));

    chart.set_price_scale_visible_range(0, false, 90.0, 110.0);
    assert_eq!(chart.price_scale_auto_scale(0, false), Some(false));
    chart.build_frame();
    assert_eq!(
        chart.price_scale_visible_range(0, false),
        Some((90.0, 110.0))
    );

    chart.set_price_scale_inverted(0, false, true);
    chart.set_price_scale_margins(0, false, 0.25, 0.15);
    assert_eq!(chart.price_scale_inverted(0, false), Some(true));
    assert_eq!(chart.price_scale_margins(0, false), Some((0.25, 0.15)));

    chart.set_price_scale_auto_scale(0, false, true);
    chart.build_frame();
    assert_eq!(chart.price_scale_auto_scale(0, false), Some(true));
    assert_ne!(
        chart.price_scale_visible_range(0, false),
        Some((90.0, 110.0))
    );
    assert_eq!(
        chart.series_price_scale(0),
        Some((0, PriceScaleTarget::Right))
    );

    chart.set_price_scale_mode(0, false, PriceScaleMode::Percentage);
    chart.build_frame();
    assert_eq!(
        chart.price_scale_mode(0, false),
        Some(PriceScaleMode::Percentage)
    );
    assert_eq!(chart.price_scale_auto_scale(0, false), Some(true));
    let coordinate = chart.series_price_to_coordinate(0, 101.5).unwrap();
    assert!((chart.series_coordinate_to_price(0, coordinate).unwrap() - 101.5).abs() < 1e-9);
    let axis = chart.build_axis_frame(
        80.0,
        |text, _bold| text.len() as f64 * 7.0,
        |text, _bold| text.len() as f64 * 6.0,
    );
    assert!(axis.labels.iter().any(|label| label.text.ends_with('%')));

    chart.set_price_scale_mode(0, false, PriceScaleMode::Logarithmic);
    chart.build_frame();
    let coordinate = chart.series_price_to_coordinate(0, 102.5).unwrap();
    assert!((chart.series_coordinate_to_price(0, coordinate).unwrap() - 102.5).abs() < 1e-8);
}

#[test]
fn reset_view_restores_time_defaults_and_reenables_autoscale() {
    let mut chart = ChartEngine::new(300.0, 200.0, 1.0);
    chart
        .set_series_data(
            0,
            &[10.0, 20.0, 30.0],
            &[100.0, 101.0, 102.0],
            &[101.0, 102.0, 103.0],
            &[99.0, 100.0, 101.0],
            &[100.5, 101.5, 102.5],
        )
        .unwrap();
    let comparison = chart
        .add_price_scale(0, "comparison", PriceScaleSide::Left, Some(0), false)
        .unwrap();
    chart.time_scale.set_width(300.0);
    chart.layout_panes(172.0);
    chart.fit_content();
    chart.build_frame();

    // Simulate a user zoom/pan: custom spacing+offset and a manual (contracted) price range.
    chart.set_bar_spacing(20.0);
    chart.set_right_offset(2.0);
    chart.set_price_scale_visible_range(0, false, 101.0, 101.2);
    chart.set_price_scale_visible_range_for(0, PriceScaleTarget::Left, 90.0, 91.0);
    chart.set_price_scale_visible_range_for(0, PriceScaleTarget::Overlay, 80.0, 81.0);
    chart.set_price_scale_visible_range_for(0, comparison, 70.0, 71.0);
    assert_eq!(chart.price_scale_auto_scale(0, false), Some(false));
    assert_eq!(
        chart.price_scale_auto_scale_for(0, PriceScaleTarget::Left),
        Some(false)
    );
    assert_eq!(
        chart.price_scale_auto_scale_for(0, PriceScaleTarget::Overlay),
        Some(false)
    );
    assert_eq!(chart.price_scale_auto_scale_for(0, comparison), Some(false));
    chart.build_frame();
    assert_eq!(
        chart.price_scale_visible_range(0, false),
        Some((101.0, 101.2))
    );

    chart.reset_view();
    // Time scale back to the configured defaults (the public resetTimeScale behavior)…
    assert_eq!(chart.bar_spacing(), 6.0);
    // …except that a view reset leaves a right margin worth 10% of the plot width instead of
    // pinning the newest bar to the axis under its own live-price cluster (industry-standard).
    // 300px plot / 6px bars => 5 empty bars.
    assert_eq!(chart.right_offset(), 5.0);
    assert!(
        chart.right_offset() * chart.bar_spacing() - chart.pane_w * 0.10 < 1e-9,
        "the margin is a fraction of the plot width, not a fixed bar count"
    );
    // The reference `resetTimeScale` itself keeps its zero-offset semantics for direct callers.
    chart.reset_time_scale();
    assert_eq!(chart.right_offset(), 0.0);
    // …and every pane's price scales autoscale again (the public pane resetPriceScale behavior); the next
    // frame recalculates the range, so the contracted data fits the pane once more.
    assert_eq!(chart.price_scale_auto_scale(0, false), Some(true));
    assert_eq!(
        chart.price_scale_auto_scale_for(0, PriceScaleTarget::Left),
        Some(true)
    );
    assert_eq!(
        chart.price_scale_auto_scale_for(0, PriceScaleTarget::Overlay),
        Some(true)
    );
    assert_eq!(chart.price_scale_auto_scale_for(0, comparison), Some(true));
    chart.build_frame();
    let (min, max) = chart.price_scale_visible_range(0, false).unwrap();
    assert!(
        min <= 99.0 && max >= 103.0,
        "range must fit the data, got ({min}, {max})"
    );
}

#[test]
fn reset_style_to_defaults_preserves_runtime_view_and_semantic_state() {
    let mut chart = ChartEngine::new(640.0, 400.0, 1.0);
    chart
        .set_series_data(
            0,
            &[1.0, 2.0, 3.0, 4.0, 5.0],
            &[100.0, 101.0, 102.0, 103.0, 104.0],
            &[101.0, 102.0, 103.0, 104.0, 105.0],
            &[99.0, 100.0, 101.0, 102.0, 103.0],
            &[100.5, 101.5, 102.5, 103.5, 104.5],
        )
        .unwrap();
    chart.set_theme(ChartTheme::Light);
    chart
        .apply_options(
            r##"{
                "layout":{"background":{"color":"#123456"},"fontSize":20},
                "grid":{"vertLines":{"visible":false,"color":"#abcdef"}},
                "crosshair":{"vertLine":{"color":"#abcdef","width":4}},
                "rightPriceScale":{"borderColor":"#abcdef","textColor":"#abcdef"},
                "timeScale":{"borderColor":"#abcdef"},
                "watermark":{"visible":true,"text":"KEEP","color":"#abcdef","fontSize":70}
            }"##,
        )
        .unwrap();

    chart.set_series_visible(0, false);
    assert!(chart.series_apply_options_json(
        0,
        r##"{
            "color":"#ff00ff",
            "up_color":"#00ff00",
            "line_width":7,
            "line_style":2,
            "price_line_color":"#ff00ff",
            "title":"Primary",
            "title_visible":false,
            "countdown_visible":false
        }"##,
    ));
    assert!(chart
        .series_apply_price_format_json(0, r#"{"type":"price","precision":4,"min_move":0.0001}"#,));

    let rsi = chart.add_rsi(0, 2).expect("valid RSI");
    assert!(chart.series_apply_options_json(
        rsi,
        r##"{
            "color":"#ff0000",
            "line_width":9,
            "countdown_visible":true,
            "title":"Custom RSI",
            "title_visible":false
        }"##,
    ));

    let feature = chart.add_feature_series(
        FeatureSeriesKind::PrettyHistogram,
        FeatureSeriesOptionsPatch {
            color: Some(Color::rgb(1, 2, 3)),
            line_width: Some(8.0),
            base_price: Some(42.0),
            width_percent: Some(37.0),
            low_value: Some(-10.0),
            high_value: Some(250.0),
            ..FeatureSeriesOptionsPatch::default()
        },
    );

    let mut footprint_options = FootprintSeriesOptions::default();
    footprint_options.aggregation.tick_size = 0.5;
    footprint_options.visual.cell_mode = FootprintCellMode::Delta;
    footprint_options.visual.font_size = 19.0;
    footprint_options.visual.text_color = Some(Color::rgb(1, 2, 3));
    footprint_options.visual.show_bar_summary = false;
    let footprint = chart
        .add_footprint_series(footprint_options)
        .expect("valid footprint options");

    let drawing = chart
        .add_drawing(
            DrawingKind::TrendLine,
            0,
            vec![
                DrawingPoint {
                    logical: 0.0,
                    price: 100.0,
                },
                DrawingPoint {
                    logical: 3.0,
                    price: 103.0,
                },
            ],
            Some(r##"{"color":"#ff0000","width":4}"##),
        )
        .expect("valid drawing");
    let drawing_before = chart.drawing_options_json(drawing).unwrap();

    let named = chart
        .add_price_scale(0, "custom", PriceScaleSide::Left, Some(0), true)
        .unwrap();
    assert!(chart.price_scale_apply_options_json(
        0,
        named,
        r##"{
            "mode":1,
            "auto_scale":false,
            "invert_scale":true,
            "scale_margins":{"top":0.3,"bottom":0.2},
            "align_labels":false,
            "ticks_visible":true,
            "entire_text_only":true,
            "minimum_width":91,
            "text_color":"#ff0000",
            "bold_round_labels":false
        }"##,
    ));
    chart.set_price_scale_visible_range_for(0, named, 90.0, 110.0);
    let named_range = chart
        .price_scale_visible_range_for(0, named)
        .expect("named range");

    chart.time_scale.set_width(640.0);
    chart.set_bar_spacing(17.0);
    chart.set_right_offset(4.25);

    let panes_before = chart.panes.len();
    let indicators_before = chart.indicators.len();
    let data_before = chart.series_data(0);

    chart.reset_style_to_defaults();

    assert_eq!(chart.bar_spacing(), 17.0);
    assert_eq!(chart.right_offset(), 4.25);
    assert_eq!(chart.panes.len(), panes_before);
    assert_eq!(chart.indicators.len(), indicators_before);
    assert_eq!(chart.drawings().len(), 1);
    assert_eq!(
        chart.drawing_options_json(drawing).as_deref(),
        Some(drawing_before.as_str())
    );
    assert_eq!(chart.series_data(0), data_before);

    let options = chart.options.get();
    assert_eq!(
        options.layout.background.color,
        aeris_charts_core::style::LIGHT_SURFACE_CSS
    );
    assert_eq!(options.layout.font_size, 12.0);
    assert!(!options.grid.vert_lines.visible);
    assert_eq!(
        options.grid.vert_lines.color,
        aeris_charts_core::style::LIGHT_BORDER_CSS
    );
    assert!(options.watermark.visible);
    assert_eq!(options.watermark.text, "KEEP");
    assert_eq!(options.watermark.color, "rgba(0, 0, 0, 0)");
    assert_eq!(options.watermark.font_size, 48.0);
    assert_eq!(options.right_price_scale.text_color, None);

    let primary: serde_json::Value =
        serde_json::from_str(&chart.series_options_json(0).unwrap()).unwrap();
    assert_eq!(primary["up_color"], "");
    assert_eq!(primary["line_width"], crate::frame::LINE_WIDTH);
    assert_eq!(primary["line_style"], 0);
    assert_eq!(primary["price_line_color"], "");
    assert_eq!(primary["title"], "Primary");
    assert_eq!(primary["visible"], false);
    assert_eq!(primary["price_format"]["precision"], 4);
    assert_eq!(primary["price_format"]["min_move"], 0.0001);
    assert_eq!(primary["countdown_visible"], false);

    let rsi_entry = chart.series_entry(rsi).unwrap();
    assert_eq!(rsi_entry.line_color, None);
    assert_eq!(rsi_entry.line_width, Some(2.0));
    assert!(!rsi_entry.countdown_visible);
    assert_eq!(rsi_entry.title, "Custom RSI");
    assert!(rsi_entry.title_visible);
    assert_eq!(
        rsi_entry.threshold_region,
        Some(SeriesThresholdRegion {
            lower: 30.0,
            upper: 70.0
        })
    );

    let feature_options: serde_json::Value =
        serde_json::from_str(&chart.feature_series_options_json(feature).unwrap()).unwrap();
    assert_eq!(feature_options["line_width"], 2.0);
    assert_ne!(feature_options["color"], "rgb(1, 2, 3)");
    assert_eq!(feature_options["base_price"], 42.0);
    assert_eq!(feature_options["width_percent"], 37.0);
    assert_eq!(feature_options["low_value"], -10.0);
    assert_eq!(feature_options["high_value"], 250.0);

    let footprint_options = chart.footprint_series_options(footprint).unwrap();
    assert_eq!(footprint_options.aggregation.tick_size, 0.5);
    assert_eq!(footprint_options.visual.cell_mode, FootprintCellMode::Delta);
    assert!(!footprint_options.visual.show_bar_summary);
    assert_eq!(footprint_options.visual.font_size, 11.0);
    assert_eq!(footprint_options.visual.text_color, None);
    assert_eq!(
        footprint_options.visual.bid_color,
        FootprintVisualOptions::default().bid_color
    );
    assert_eq!(
        chart.series_entry(footprint).unwrap().price_format.min_move,
        0.5
    );

    let named_options: serde_json::Value = serde_json::from_str(
        &chart
            .price_scale_options_json(0, named)
            .expect("named scale options"),
    )
    .unwrap();
    assert_eq!(named_options["mode"], 1);
    assert_eq!(named_options["auto_scale"], false);
    assert_eq!(named_options["invert_scale"], true);
    assert_eq!(named_options["scale_margins"]["top"], 0.3);
    assert_eq!(named_options["scale_margins"]["bottom"], 0.2);
    assert_eq!(named_options["align_labels"], false);
    assert_eq!(named_options["entire_text_only"], true);
    assert_eq!(named_options["minimum_width"], 91.0);
    assert_eq!(named_options["ticks_visible"], false);
    assert_eq!(named_options["text_color"], serde_json::Value::Null);
    assert_eq!(named_options["bold_round_labels"], true);
    assert_eq!(
        chart.price_scale_visible_range_for(0, named),
        Some(named_range)
    );
}

#[test]
fn only_the_canonical_primary_series_owns_countdown_by_default() {
    let mut chart = ChartEngine::new(640.0, 400.0, 1.0);
    assert!(chart.series_entry(0).unwrap().countdown_visible);

    let overlay = chart.add_series(SeriesKind::Line);
    assert!(!chart.series_entry(overlay).unwrap().countdown_visible);

    assert!(chart.series_apply_options_json(overlay, r#"{"countdown_visible":true}"#));
    chart.reset_style_to_defaults();
    assert!(
        chart.series_entry(overlay).unwrap().countdown_visible,
        "style reset must preserve explicit countdown ownership"
    );
}

#[test]
fn price_reset_restores_every_scale_across_panes_without_resetting_time() {
    let mut chart = ChartEngine::new(300.0, 240.0, 1.0);
    chart
        .set_series_data(
            0,
            &[10.0, 20.0, 30.0],
            &[100.0, 101.0, 102.0],
            &[101.0, 102.0, 103.0],
            &[99.0, 100.0, 101.0],
            &[100.5, 101.5, 102.5],
        )
        .unwrap();
    let named = chart
        .add_price_scale(0, "comparison-reset", PriceScaleSide::Left, Some(0), false)
        .unwrap();
    let pane = chart.add_pane(true).expect("second pane");
    let pane_named = chart
        .add_price_scale(pane, "pane-reset", PriceScaleSide::Right, Some(0), false)
        .unwrap();

    chart.set_bar_spacing(18.0);
    chart.set_right_offset(3.0);
    let spacing_before = chart.bar_spacing();
    let offset_before = chart.right_offset();
    for (pane, target, from) in [
        (0, PriceScaleTarget::Right, 100.0),
        (0, PriceScaleTarget::Left, 90.0),
        (0, PriceScaleTarget::Overlay, 80.0),
        (0, named, 70.0),
        (pane, PriceScaleTarget::Right, 60.0),
        (pane, PriceScaleTarget::Left, 50.0),
        (pane, PriceScaleTarget::Overlay, 40.0),
        (pane, pane_named, 30.0),
    ] {
        chart.set_price_scale_visible_range_for(pane, target, from, from + 1.0);
        assert_eq!(chart.price_scale_auto_scale_for(pane, target), Some(false));
    }
    chart.price_axis_start_scroll(0, PriceScaleTarget::Right, 100.0);

    chart.reset_price_scales();
    for (pane, target) in [
        (0, PriceScaleTarget::Right),
        (0, PriceScaleTarget::Left),
        (0, PriceScaleTarget::Overlay),
        (0, named),
        (pane, PriceScaleTarget::Right),
        (pane, PriceScaleTarget::Left),
        (pane, PriceScaleTarget::Overlay),
        (pane, pane_named),
    ] {
        assert_eq!(chart.price_scale_auto_scale_for(pane, target), Some(true));
    }
    assert_eq!(chart.bar_spacing(), spacing_before);
    assert_eq!(chart.right_offset(), offset_before);

    // Reset must discard the in-flight manual snapshot. Turning manual mode back on without
    // starting a new drag cannot resume the stale session.
    chart.build_frame();
    let reset_range = chart
        .price_scale_visible_range_for(0, PriceScaleTarget::Right)
        .unwrap();
    chart.set_price_scale_auto_scale_for(0, PriceScaleTarget::Right, false);
    chart.price_axis_scroll_to(0, PriceScaleTarget::Right, 140.0);
    assert_eq!(
        chart.price_scale_visible_range_for(0, PriceScaleTarget::Right),
        Some(reset_range)
    );
}

#[test]
fn left_price_scale_owns_range_axis_labels_and_pane_offset() {
    let mut chart = ChartEngine::new(300.0, 200.0, 1.0);
    chart
        .set_series_data(
            0,
            &[10.0, 20.0, 30.0],
            &[100.0, 101.0, 102.0],
            &[101.0, 102.0, 103.0],
            &[99.0, 100.0, 101.0],
            &[100.5, 101.5, 102.5],
        )
        .unwrap();
    chart.set_series_price_scale(0, PriceScaleTarget::Left);
    chart
        .options
        .apply_str(r#"{"leftPriceScale":{"visible":true},"rightPriceScale":{"visible":false}}"#)
        .unwrap();
    chart.pane_left = 58.0;
    chart.left_axis_w = 58.0;
    chart.pane_w = 242.0;
    chart.time_scale.set_width(242.0);
    chart.layout_panes(172.0);
    chart.fit_content();

    let frame = chart.build_frame();
    assert_eq!(
        chart.series_price_scale(0),
        Some((0, PriceScaleTarget::Left))
    );
    assert!(chart
        .price_scale_visible_range_for(0, PriceScaleTarget::Left)
        .is_some());
    assert!(chart
        .price_scale_visible_range_for(0, PriceScaleTarget::Right)
        .is_none());
    assert_eq!(frame.width, 300.0);
    assert_eq!(frame.panes[0].scissor[0], 58);
    assert!(frame.panes[0].main.iter().any(|prim| matches!(
        prim,
        aeris_charts_render::draw_list::Prim::Rect { rect, .. } if rect.x >= 58
    )));

    let axis = chart.build_axis_frame(
        80.0,
        |text, _bold| text.len() as f64 * 7.0,
        |text, _bold| text.len() as f64 * 6.0,
    );
    assert!(axis
        .labels
        .iter()
        .any(|label| label.align == AxisTextAlign::Right));
    assert!(!axis
        .labels
        .iter()
        .any(|label| label.align == AxisTextAlign::Left));
    let coordinate = chart.series_price_to_coordinate(0, 101.5).unwrap();
    assert!((chart.series_coordinate_to_price(0, coordinate).unwrap() - 101.5).abs() < 1e-9);
}

#[test]
fn series_data_and_logical_range_queries_match_reference_gap_semantics() {
    let mut chart = ChartEngine::new(300.0, 200.0, 1.0);
    let times = (0..=10).map(|time| time as f64 * 10.0).collect::<Vec<_>>();
    let values = (0..=10).map(|value| value as f64).collect::<Vec<_>>();
    chart
        .set_series_data(0, &times, &values, &values, &values, &values)
        .unwrap();
    let sparse = chart.add_series(SeriesKind::Line);
    chart
        .set_series_data(
            sparse,
            &[0.0, 100.0],
            &[5.0, 15.0],
            &[5.0, 15.0],
            &[5.0, 15.0],
            &[5.0, 15.0],
        )
        .unwrap();

    assert_eq!(chart.series_kind(sparse), Some(SeriesKind::Line));
    assert_eq!(chart.series_data(sparse).len(), 2);
    assert_eq!(
        chart.series_data_by_index(sparse, 5, MismatchDirection::NearestLeft),
        Some(SeriesDataPoint {
            time: 0,
            open: 5.0,
            high: 5.0,
            low: 5.0,
            close: 5.0,
        })
    );
    assert_eq!(
        chart
            .series_data_by_index(sparse, 5, MismatchDirection::NearestRight)
            .map(|point| point.time),
        Some(100)
    );
    assert_eq!(
        chart.series_bars_in_logical_range(sparse, 3.0, 7.0),
        Some(BarsInLogicalRange {
            bars_before: 3.0,
            bars_after: 3.0,
            from: None,
            to: None,
        })
    );
    assert_eq!(
        chart.series_bars_in_logical_range(sparse, -1.5, 5.25),
        Some(BarsInLogicalRange {
            bars_before: -1.5,
            bars_after: 10.0,
            from: Some(0),
            to: Some(0),
        })
    );
}

#[test]
fn value_snapshot_unifies_latest_exact_predecessor_format_and_placement() {
    let nan = f64::NAN;
    let mut chart = ChartEngine::new(300.0, 200.0, 1.0);
    chart
        .set_series_data(
            0,
            &[10.0, 20.0, 30.0],
            &[10.0, 20.0, nan],
            &[13.0, 23.0, nan],
            &[9.0, 19.0, nan],
            &[11.0, 21.0, nan],
        )
        .unwrap();
    assert!(chart.series_apply_price_format_json(0, r#"{"type":"price","precision":1}"#));
    let comparison = chart.add_series(SeriesKind::Line);
    chart
        .set_series_data(
            comparison,
            &[10.0, 30.0],
            &[100.0, 300.0],
            &[100.0, 300.0],
            &[100.0, 300.0],
            &[100.0, 300.0],
        )
        .unwrap();
    chart.convert_series_kind(comparison, SeriesKind::Area);
    chart.set_series_pane(comparison, 1, 0.5);
    chart.set_series_price_scale(comparison, PriceScaleTarget::Left);

    let latest = chart.value_snapshot(None);
    assert_eq!(latest.len(), 2);
    assert_eq!(
        (latest[0].logical_index, latest[0].time, latest[0].close),
        (Some(1), Some(20), Some(21.0))
    );
    assert_eq!(latest[0].previous_value, Some(11.0));
    assert_eq!(latest[0].formatted_close.as_deref(), Some("21.0"));
    assert_eq!(latest[0].formatted_previous_value.as_deref(), Some("11.0"));
    assert_eq!(
        (latest[1].logical_index, latest[1].time, latest[1].value),
        (Some(2), Some(30), Some(300.0))
    );
    assert_eq!(latest[1].previous_value, Some(100.0));
    assert_eq!(latest[1].kind, SeriesKind::Area);
    assert_eq!(latest[1].pane_index, 1);
    assert_eq!(latest[1].price_scale_id, "left");

    let exact_gap = chart.value_snapshot(Some(1));
    assert_eq!(exact_gap[0].close, Some(21.0));
    assert_eq!(exact_gap[0].previous_value, Some(11.0));
    assert_eq!(exact_gap[1].logical_index, Some(1));
    assert_eq!(exact_gap[1].time, Some(20));
    assert_eq!(exact_gap[1].value, None, "a gap never borrows a value");
    assert_eq!(exact_gap[1].previous_value, None);

    let exact_whitespace = chart.value_snapshot(Some(2));
    assert_eq!(exact_whitespace[0].time, Some(30));
    assert_eq!(exact_whitespace[0].close, None);
    assert_eq!(exact_whitespace[0].previous_value, None);
    assert_eq!(exact_whitespace[1].value, Some(300.0));
    assert_eq!(exact_whitespace[1].previous_value, Some(100.0));

    assert!(chart.update_series_bar(comparison, 40.0, [400.0; 4]));
    let fresh = chart.value_snapshot(None);
    assert_eq!(fresh[1].time, Some(40));
    assert_eq!(fresh[1].value, Some(400.0));
    assert_eq!(fresh[1].previous_value, Some(300.0));
}

#[test]
fn crosshair_geometry_is_host_independent() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    chart.series[0].kind = SeriesKind::Line;
    chart
        .set_series_data(
            0,
            &[1.0, 2.0, 3.0],
            &[10.0, 11.0, 12.0],
            &[11.0, 12.0, 13.0],
            &[9.0, 10.0, 11.0],
            &[10.5, 11.5, 12.5],
        )
        .unwrap();
    chart.time_scale.set_width(800.0);
    chart.fit_content();
    chart.series[0].crosshair_marker_visible = true;
    chart.crosshair = Some((200.0, 120.0));
    let frame = chart.build_frame();
    assert!(frame.panes[0]
        .main
        .iter()
        .any(|p| matches!(p, aeris_charts_render::draw_list::Prim::VLine { .. })));
    assert!(frame.panes[0]
        .main
        .iter()
        .any(|p| matches!(p, aeris_charts_render::draw_list::Prim::HLine { .. })));
    assert!(frame.panes[0]
        .main
        .iter()
        .any(|p| matches!(p, aeris_charts_render::draw_list::Prim::Circle { .. })));

    let mut canvas = CountingCanvas::default();
    for pane in &frame.panes {
        execute(
            &pane.under,
            &pane.points,
            &mut canvas,
            Viewport {
                width: 800.0,
                height: 500.0,
            },
        );
        execute(
            &pane.main,
            &pane.points,
            &mut canvas,
            Viewport {
                width: 800.0,
                height: 500.0,
            },
        );
    }
    assert!(
        canvas.calls > 0,
        "the shared frame must be executable by a Canvas2D backend"
    );
}

#[test]
fn indicators_are_engine_owned_series() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    chart
        .set_series_data(
            0,
            &[1.0, 2.0, 3.0, 4.0],
            &[1.0, 2.0, 3.0, 4.0],
            &[1.0, 2.0, 3.0, 4.0],
            &[1.0, 2.0, 3.0, 4.0],
            &[1.0, 2.0, 3.0, 4.0],
        )
        .unwrap();
    let sma = chart.add_sma(0, 2).expect("valid indicator");
    let rows = chart.data.series_data(sma).unwrap();
    assert_eq!(rows.0, &[2, 3, 4]);
    assert_eq!(rows.1[3], &[1.5, 2.5, 3.5]);

    chart.update_series_bar(0, 4.0, [4.0, 5.0, 3.0, 5.0]);
    let rows = chart.data.series_data(sma).unwrap();
    assert_eq!(rows.1[3], &[1.5, 2.5, 4.0]);

    let ema = chart.add_ema(0, 2).expect("valid indicator");
    let initial_ema = chart.data.series_data(ema).unwrap().1[3];
    assert_eq!(initial_ema.len(), 3);
    assert!((initial_ema[2] - 4.166666666666667).abs() < 1e-12);
    chart.update_series_bar(0, 5.0, [5.0, 6.0, 4.0, 6.0]);
    let ema_rows = chart.data.series_data(ema).unwrap();
    assert!((ema_rows.1[3].last().copied().unwrap() - 5.388888888888889).abs() < 1e-12);
}

#[test]
fn ema_ribbon_owns_five_colored_outputs_and_updates_periods_atomically() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    let times = (0..300).map(|index| index as f64).collect::<Vec<_>>();
    let values = (0..300)
        .map(|index| 100.0 + index as f64 * 0.1 + (index as f64 * 0.17).sin())
        .collect::<Vec<_>>();
    chart
        .set_series_data(0, &times, &values, &values, &values, &values)
        .unwrap();

    let outputs = chart.add_ema_ribbon(0, EMA_RIBBON_DEFAULT_PERIODS);
    assert_eq!(outputs.len(), 5);
    for (index, &output) in outputs.iter().enumerate() {
        let series = chart.series_entry(output).unwrap();
        assert_eq!(
            series.line_color.as_deref(),
            Some(EMA_RIBBON_DEFAULT_COLORS[index])
        );
        assert_eq!(
            series.title,
            format!("EMA {}", EMA_RIBBON_DEFAULT_PERIODS[index])
        );
        let info = chart.indicator_info(output).unwrap();
        assert_eq!(info.kind, "ema_ribbon");
        assert_eq!(info.binding_id, outputs[0]);
        assert_eq!(info.parameters.periods, Some(EMA_RIBBON_DEFAULT_PERIODS));
        assert_eq!(info.period, EMA_RIBBON_DEFAULT_PERIODS[index]);
        assert_eq!(info.output_index, index);
        assert_eq!(info.output_count, 5);
        assert_eq!(
            info.output_name,
            ["EMA 1", "EMA 2", "EMA 3", "EMA 4", "EMA 5"][index]
        );
    }
    assert_eq!(
        chart
            .series_entry(outputs[2])
            .unwrap()
            .line_color
            .as_deref(),
        Some("#7d52f4")
    );
    assert_indicator_binding_matches_full(&chart, 0);

    let dependent = chart.add_ema(outputs[0], 2).unwrap();
    chart.series_entry_mut(outputs[1]).unwrap().title = "Custom EMA".to_string();
    let before = chart.indicator_bindings();
    assert!(!chart.set_ema_ribbon_periods(outputs[4], [6, 12, 0, 60, 120]));
    assert_eq!(chart.indicator_bindings(), before);

    let updated = [6, 12, 24, 60, 120];
    assert!(chart.set_ema_ribbon_periods(outputs[2], updated));
    let binding = &chart.indicator_bindings()[0];
    assert_eq!(binding.outputs, outputs);
    assert_eq!(binding.kind, IndicatorKind::EmaRibbon { periods: updated });
    assert_eq!(chart.series_entry(outputs[0]).unwrap().title, "EMA 6");
    assert_eq!(chart.series_entry(outputs[1]).unwrap().title, "Custom EMA");
    assert_eq!(chart.series_entry(outputs[2]).unwrap().title, "EMA 24");
    assert_eq!(
        chart
            .series_entry(outputs[2])
            .unwrap()
            .line_color
            .as_deref(),
        Some("#7d52f4")
    );
    assert_indicator_binding_matches_full(&chart, 0);
    assert_indicator_binding_matches_full(&chart, 1);
    assert!(!chart.data.series_data(dependent).unwrap().0.is_empty());
}

#[test]
fn indicator_source_can_be_replaced_below_warmup_and_repopulated() {
    for replacement_rows in [0usize, 10] {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        let times = (0..20)
            .map(|index| (1_700_000_000 + index * 60) as f64)
            .collect::<Vec<_>>();
        let values = (0..20)
            .map(|index| 100.0 + index as f64)
            .collect::<Vec<_>>();
        chart
            .set_series_data(0, &times, &values, &values, &values, &values)
            .unwrap();
        let sma = chart.add_sma(0, 20).expect("valid indicator");

        chart
            .set_series_data(
                0,
                &times[..replacement_rows],
                &values[..replacement_rows],
                &values[..replacement_rows],
                &values[..replacement_rows],
                &values[..replacement_rows],
            )
            .unwrap();
        assert!(chart.series_data(sma).is_empty());

        for row in replacement_rows..20 {
            assert!(chart.update_series_bar(0, times[row], [values[row]; 4]));
        }
        let output = chart.series_data(sma);
        assert_eq!(output.len(), 1);
        assert_eq!(output[0].time, times[19] as i64);
        assert_eq!(output[0].close, 109.5);
    }
}

#[test]
fn indicator_source_can_stream_after_retention_trims_below_warmup() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    let times = (0..20).map(|index| index as f64).collect::<Vec<_>>();
    let values = (0..20).map(|index| index as f64).collect::<Vec<_>>();
    chart
        .set_series_data(0, &times, &values, &values, &values, &values)
        .unwrap();
    let sma = chart.add_sma(0, 20).expect("valid indicator");

    assert!(chart.set_series_max_points(0, Some(1)));
    assert!(chart.series_data(sma).is_empty());
    for time in 20..40 {
        assert!(chart.update_series_bar(0, time as f64, [time as f64; 4]));
        assert!(chart.series_data(sma).is_empty());
    }
}

fn add_test_indicator(
    chart: &mut ChartEngine,
    kind: &IndicatorKind,
    volume: Option<SeriesId>,
) -> Vec<SeriesId> {
    chart.add_indicator_kind(
        0,
        kind.clone(),
        matches!(
            kind,
            IndicatorKind::Vwap
                | IndicatorKind::Obv
                | IndicatorKind::Cmf { .. }
                | IndicatorKind::Mfi { .. }
                | IndicatorKind::Volume { .. }
        )
        .then_some(volume)
        .flatten(),
    )
}

fn assert_indicator_binding_matches_full(chart: &ChartEngine, binding_index: usize) {
    let binding = &chart.indicators[binding_index];
    let (times, source) = chart.data.series_data(binding.source).unwrap();
    let expected = match binding.kind {
        IndicatorKind::Sma { period } => vec![aeris_charts_indicators::sma(source[3], period)],
        IndicatorKind::Ema { period, seed } => vec![aeris_charts_indicators::ema_with_seed(
            source[3], period, seed,
        )],
        IndicatorKind::Dema { period, seed } => vec![aeris_charts_indicators::dema_with_seed(
            source[3], period, seed,
        )],
        IndicatorKind::Tema { period, seed } => vec![aeris_charts_indicators::tema_with_seed(
            source[3], period, seed,
        )],
        IndicatorKind::Smma { period } => vec![aeris_charts_indicators::smma(source[3], period)],
        IndicatorKind::Hma { period } => vec![aeris_charts_indicators::hma(source[3], period)],
        IndicatorKind::Vwma { period } => {
            vec![aeris_charts_indicators::vwma(source[3], &[], period)]
        }
        IndicatorKind::StandardDeviation { period } => {
            vec![aeris_charts_indicators::standard_deviation(
                source[3], period,
            )]
        }
        IndicatorKind::Cci { period } => {
            vec![aeris_charts_indicators::cci(
                source[1], source[2], source[3], period,
            )]
        }
        IndicatorKind::WilliamsR { period } => {
            vec![aeris_charts_indicators::williams_r(
                source[1], source[2], source[3], period,
            )]
        }
        IndicatorKind::StochasticRsi {
            rsi_period,
            stochastic_period,
        } => vec![aeris_charts_indicators::stochastic_rsi(
            source[3],
            rsi_period,
            stochastic_period,
        )],
        IndicatorKind::Momentum { period } => {
            vec![aeris_charts_indicators::momentum(source[3], period)]
        }
        IndicatorKind::RateOfChange { period } => {
            vec![aeris_charts_indicators::rate_of_change(source[3], period)]
        }
        IndicatorKind::Donchian { period } => {
            let points = aeris_charts_indicators::donchian(source[1], source[2], period);
            vec![
                points.iter().map(|point| point.upper).collect(),
                points.iter().map(|point| point.middle).collect(),
                points.iter().map(|point| point.lower).collect(),
            ]
        }
        IndicatorKind::PivotPoints { variant } => {
            let points = aeris_charts_indicators::pivot_points(
                times, source[0], source[1], source[2], source[3], variant,
            );
            vec![
                points.iter().map(|point| point.pivot).collect(),
                points.iter().map(|point| point.resistance_1).collect(),
                points.iter().map(|point| point.support_1).collect(),
                points.iter().map(|point| point.resistance_2).collect(),
                points.iter().map(|point| point.support_2).collect(),
            ]
        }
        IndicatorKind::ZigZag { deviation_percent } => {
            vec![aeris_charts_indicators::zigzag(
                source[1],
                source[2],
                deviation_percent,
            )]
        }
        IndicatorKind::Keltner { period, multiplier } => {
            let points = aeris_charts_indicators::keltner(
                source[1], source[2], source[3], period, multiplier,
            );
            vec![
                points.iter().map(|point| point.upper).collect(),
                points.iter().map(|point| point.middle).collect(),
                points.iter().map(|point| point.lower).collect(),
            ]
        }
        IndicatorKind::AdxDmi { period } => {
            let points = aeris_charts_indicators::adx_dmi(source[1], source[2], source[3], period);
            vec![
                points.iter().map(|point| point.plus_di).collect(),
                points.iter().map(|point| point.minus_di).collect(),
                points.iter().map(|point| point.adx).collect(),
            ]
        }
        IndicatorKind::ParabolicSar => {
            vec![aeris_charts_indicators::parabolic_sar(source[1], source[2])]
        }
        IndicatorKind::SuperTrend { period, multiplier } => {
            vec![aeris_charts_indicators::supertrend(
                source[1], source[2], source[3], period, multiplier,
            )]
        }
        IndicatorKind::Ichimoku => {
            let points = aeris_charts_indicators::ichimoku(source[1], source[2], source[3]);
            vec![
                points.iter().map(|point| point.conversion).collect(),
                points.iter().map(|point| point.base).collect(),
                points.iter().map(|point| point.leading_a).collect(),
                points.iter().map(|point| point.leading_b).collect(),
                points.iter().map(|point| point.lagging).collect(),
            ]
        }
        IndicatorKind::EmaRibbon { periods } => periods
            .into_iter()
            .map(|period| aeris_charts_indicators::ema(source[3], period))
            .collect(),
        IndicatorKind::Bollinger {
            period,
            deviation,
            estimator,
        } => {
            let points =
                aeris_charts_indicators::bollinger_with(source[3], period, deviation, estimator);
            vec![
                points.iter().map(|point| point.upper).collect(),
                points.iter().map(|point| point.middle).collect(),
                points.iter().map(|point| point.lower).collect(),
            ]
        }
        IndicatorKind::Rsi { period, seed } => vec![aeris_charts_indicators::rsi_with_seed(
            source[3], period, seed,
        )],
        IndicatorKind::Macd {
            fast,
            slow,
            signal,
            seed,
            histogram_multiplier,
        } => {
            let points = aeris_charts_indicators::macd_with(
                source[3],
                fast,
                slow,
                signal,
                seed,
                histogram_multiplier,
            );
            vec![
                points.iter().map(|point| point.macd).collect(),
                points.iter().map(|point| point.signal).collect(),
                points.iter().map(|point| point.histogram).collect(),
            ]
        }
        IndicatorKind::Stochastic { k_period, d_period } => {
            let points = aeris_charts_indicators::stochastic(
                source[1], source[2], source[3], k_period, d_period,
            );
            vec![
                points.iter().map(|point| point.k).collect(),
                points.iter().map(|point| point.d).collect(),
            ]
        }
        IndicatorKind::Atr { period } => vec![aeris_charts_indicators::atr(
            source[1], source[2], source[3], period,
        )],
        IndicatorKind::Vwap => {
            let volume = binding
                .volume_source
                .and_then(|id| chart.data.series_data(id))
                .map(|(volume_times, values)| {
                    let mut aligned = vec![1.0; times.len()];
                    let mut volume_row = 0;
                    for (source_row, &time) in times.iter().enumerate() {
                        while volume_row < volume_times.len() && volume_times[volume_row] < time {
                            volume_row += 1;
                        }
                        if volume_times.get(volume_row) == Some(&time) {
                            aligned[source_row] = values[3][volume_row];
                        }
                    }
                    aligned
                })
                .unwrap_or_default();
            vec![aeris_charts_indicators::vwap(
                times, source[1], source[2], source[3], &volume,
            )]
        }
        IndicatorKind::Obv => {
            let volume = binding
                .volume_source
                .and_then(|id| chart.data.series_data(id))
                .map(|(volume_times, values)| {
                    let mut aligned = vec![0.0; times.len()];
                    let mut volume_row = 0;
                    for (source_row, &time) in times.iter().enumerate() {
                        while volume_row < volume_times.len() && volume_times[volume_row] < time {
                            volume_row += 1;
                        }
                        if volume_times.get(volume_row) == Some(&time) {
                            aligned[source_row] = values[3][volume_row];
                        }
                    }
                    aligned
                })
                .unwrap_or_default();
            vec![aeris_charts_indicators::obv(source[3], &volume)]
        }
        IndicatorKind::Cmf { period } => {
            let volume = binding
                .volume_source
                .and_then(|id| chart.data.series_data(id))
                .map(|(volume_times, values)| {
                    let mut aligned = vec![0.0; times.len()];
                    let mut volume_row = 0;
                    for (source_row, &time) in times.iter().enumerate() {
                        while volume_row < volume_times.len() && volume_times[volume_row] < time {
                            volume_row += 1;
                        }
                        if volume_times.get(volume_row) == Some(&time) {
                            aligned[source_row] = values[3][volume_row];
                        }
                    }
                    aligned
                })
                .unwrap_or_default();
            vec![aeris_charts_indicators::cmf(
                source[1], source[2], source[3], &volume, period,
            )]
        }
        IndicatorKind::Mfi { period } => {
            let volume = binding
                .volume_source
                .and_then(|id| chart.data.series_data(id))
                .map(|(volume_times, values)| {
                    let mut aligned = vec![0.0; times.len()];
                    let mut volume_row = 0;
                    for (source_row, &time) in times.iter().enumerate() {
                        while volume_row < volume_times.len() && volume_times[volume_row] < time {
                            volume_row += 1;
                        }
                        if volume_times.get(volume_row) == Some(&time) {
                            aligned[source_row] = values[3][volume_row];
                        }
                    }
                    aligned
                })
                .unwrap_or_default();
            vec![aeris_charts_indicators::mfi(
                source[1], source[2], source[3], &volume, period,
            )]
        }
        IndicatorKind::Volume { period } => {
            let volume = binding
                .volume_source
                .and_then(|id| chart.data.series_data(id))
                .map(|(volume_times, values)| {
                    let mut aligned = vec![0.0; times.len()];
                    let mut volume_row = 0;
                    for (source_row, &time) in times.iter().enumerate() {
                        while volume_row < volume_times.len() && volume_times[volume_row] < time {
                            volume_row += 1;
                        }
                        if volume_times.get(volume_row) == Some(&time) {
                            aligned[source_row] = values[3][volume_row];
                        }
                    }
                    aligned
                })
                .unwrap_or_default();
            vec![
                volume.iter().copied().map(Some).collect(),
                aeris_charts_indicators::sma(&volume, period),
            ]
        }
        IndicatorKind::VwapBands {
            reset,
            standard_deviation,
            percent,
        } => {
            let volume = binding
                .volume_source
                .and_then(|id| chart.data.series_data(id))
                .map(|(volume_times, values)| {
                    let mut aligned = vec![1.0; times.len()];
                    let mut volume_row = 0;
                    for (source_row, &time) in times.iter().enumerate() {
                        while volume_row < volume_times.len() && volume_times[volume_row] < time {
                            volume_row += 1;
                        }
                        if volume_times.get(volume_row) == Some(&time) {
                            aligned[source_row] = values[3][volume_row];
                        }
                    }
                    aligned
                })
                .unwrap_or_default();
            let points = aeris_charts_indicators::vwap_bands(
                times,
                source[1],
                source[2],
                source[3],
                &volume,
                aeris_charts_indicators::VwapBandsOptions {
                    reset,
                    standard_deviation,
                    percent,
                },
            );
            vec![
                points.iter().map(|point| point.basis).collect(),
                points.iter().map(|point| point.standard_upper).collect(),
                points.iter().map(|point| point.standard_lower).collect(),
                points.iter().map(|point| point.percent_upper).collect(),
                points.iter().map(|point| point.percent_lower).collect(),
            ]
        }
        IndicatorKind::Wma { period } => vec![aeris_charts_indicators::wma(source[3], period)],
        IndicatorKind::Kdj {
            period,
            k_smoothing,
            d_smoothing,
            seed,
        } => {
            let points = aeris_charts_indicators::kdj_with_seed(
                source[1],
                source[2],
                source[3],
                period,
                k_smoothing,
                d_smoothing,
                seed,
            );
            vec![
                points.iter().map(|point| point.k).collect(),
                points.iter().map(|point| point.d).collect(),
                points.iter().map(|point| point.j).collect(),
            ]
        }
    };

    for (&output, expected) in binding.outputs.iter().zip(expected) {
        let expected = if matches!(
            binding.kind,
            IndicatorKind::PivotPoints { .. } | IndicatorKind::ZigZag { .. }
        ) {
            times
                .iter()
                .copied()
                .zip(expected)
                .map(|(time, value)| (time, value.unwrap_or(f64::NAN)))
                .collect::<Vec<_>>()
        } else {
            times
                .iter()
                .copied()
                .zip(expected)
                .filter_map(|(time, value)| value.map(|value| (time, value)))
                .collect::<Vec<_>>()
        };
        let (actual_times, actual) = chart.data.series_data(output).unwrap();
        assert_eq!(
            actual_times,
            expected.iter().map(|(time, _)| *time).collect::<Vec<_>>(),
            "{kind:?} output {output:?}",
            kind = binding.kind,
            output = output,
        );
        assert_eq!(actual[3].len(), expected.len());
        for (index, (&actual, (_, expected))) in actual[3].iter().zip(expected).enumerate() {
            if expected.is_nan() {
                assert!(
                    actual.is_nan(),
                    "{:?} row {index}: expected NaN, actual {actual}",
                    binding.kind,
                );
            } else {
                assert!(
                    (actual - expected).abs() < 1e-10,
                    "{:?} row {index}: {actual} != {expected}",
                    binding.kind,
                );
            }
        }
    }
}

#[test]
fn every_indicator_engine_path_matches_full_recomputation() {
    let kinds = [
        IndicatorKind::Sma { period: 5 },
        IndicatorKind::Ema {
            period: 5,
            seed: IndicatorSeed::Sma,
        },
        IndicatorKind::EmaRibbon {
            periods: [3, 5, 8, 13, 21],
        },
        IndicatorKind::Bollinger {
            period: 5,
            deviation: 2.0,
            estimator: DeviationEstimator::Population,
        },
        IndicatorKind::Rsi {
            period: 5,
            seed: IndicatorSeed::Sma,
        },
        IndicatorKind::Macd {
            fast: 3,
            slow: 6,
            signal: 4,
            seed: IndicatorSeed::Sma,
            histogram_multiplier: 1.0,
        },
        IndicatorKind::Stochastic {
            k_period: 5,
            d_period: 3,
        },
        IndicatorKind::Atr { period: 5 },
        IndicatorKind::Vwap,
        IndicatorKind::VwapBands {
            reset: aeris_charts_indicators::VwapReset::Monthly,
            standard_deviation: 1.0,
            percent: 5.0,
        },
        IndicatorKind::Obv,
        IndicatorKind::Cmf { period: 5 },
        IndicatorKind::Mfi { period: 5 },
        IndicatorKind::Volume { period: 5 },
        IndicatorKind::Wma { period: 5 },
        IndicatorKind::PivotPoints {
            variant: aeris_charts_indicators::PivotKind::Standard,
        },
        IndicatorKind::ZigZag {
            deviation_percent: 5.0,
        },
        IndicatorKind::Keltner {
            period: 5,
            multiplier: 2.0,
        },
        IndicatorKind::AdxDmi { period: 5 },
        IndicatorKind::ParabolicSar,
        IndicatorKind::SuperTrend {
            period: 5,
            multiplier: 3.0,
        },
        IndicatorKind::Ichimoku,
        IndicatorKind::Cci { period: 5 },
        IndicatorKind::WilliamsR { period: 5 },
        IndicatorKind::StochasticRsi {
            rsi_period: 5,
            stochastic_period: 5,
        },
        IndicatorKind::Momentum { period: 5 },
        IndicatorKind::RateOfChange { period: 5 },
    ];
    for kind in kinds {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        let volume = chart.add_series(SeriesKind::Histogram);
        let times = (0..40)
            .map(|index| index as f64 * 3_600.0)
            .collect::<Vec<_>>();
        let close = (0..40)
            .map(|index| 90.0 + index as f64 * 0.4)
            .collect::<Vec<_>>();
        let high = close.iter().map(|value| value + 2.0).collect::<Vec<_>>();
        let low = close.iter().map(|value| value - 1.0).collect::<Vec<_>>();
        let volumes = (0..40).map(|index| (index % 7) as f64).collect::<Vec<_>>();
        chart
            .set_series_data(0, &times, &close, &high, &low, &close)
            .unwrap();
        chart
            .set_series_data(volume, &times, &volumes, &volumes, &volumes, &volumes)
            .unwrap();
        let outputs = add_test_indicator(&mut chart, &kind, Some(volume));
        let binding = chart.indicators.len() - 1;
        assert_indicator_binding_matches_full(&chart, binding);

        chart.update_series_bar(0, 40.0 * 3_600.0, [106.0, 108.0, 105.0, 107.0]);
        assert_indicator_binding_matches_full(&chart, binding);

        let batch_times = (41..46).map(|index| index * 3_600).collect::<Vec<_>>();
        let batch_close = (41..46)
            .map(|index| 90.0 + index as f64 * 0.4)
            .collect::<Vec<_>>();
        let batch_high = batch_close
            .iter()
            .map(|value| value + 2.0)
            .collect::<Vec<_>>();
        let batch_low = batch_close
            .iter()
            .map(|value| value - 1.0)
            .collect::<Vec<_>>();
        chart.update_series_bars_sanitized(
            0,
            batch_times,
            batch_close.clone(),
            batch_high,
            batch_low,
            batch_close,
        );
        assert_indicator_binding_matches_full(&chart, binding);

        for replacement in 0..1_000 {
            let close = 111.0 + replacement as f64 * 0.001;
            chart.update_series_bar(0, 45.0 * 3_600.0, [close, close + 1.0, close - 1.0, close]);
        }
        assert_indicator_binding_matches_full(&chart, binding);

        chart.update_series_bar(0, 17.0 * 3_600.0, [99.0, 102.0, 97.0, 100.0]);
        assert_indicator_binding_matches_full(&chart, binding);
        chart.update_series_bar(0, 17.5 * 3_600.0, [98.0, 101.0, 96.0, 99.0]);
        assert_indicator_binding_matches_full(&chart, binding);

        chart.series_pop(0, 7).unwrap();
        assert_indicator_binding_matches_full(&chart, binding);

        let replacement_times = (0..31)
            .map(|index| 86_400.0 + index as f64 * 1_800.0)
            .collect::<Vec<_>>();
        let replacement = (0..31)
            .map(|index| 70.0 + index as f64 * 0.75)
            .collect::<Vec<_>>();
        let replacement_high = replacement
            .iter()
            .map(|value| value + 3.0)
            .collect::<Vec<_>>();
        let replacement_low = replacement
            .iter()
            .map(|value| value - 2.0)
            .collect::<Vec<_>>();
        chart
            .set_series_data(
                0,
                &replacement_times,
                &replacement,
                &replacement_high,
                &replacement_low,
                &replacement,
            )
            .unwrap();
        assert_indicator_binding_matches_full(&chart, binding);

        assert!(chart.remove_series(0));
        assert!(outputs
            .iter()
            .all(|&output| chart.series_kind(output).is_none()));
        assert!(chart.indicators.is_empty());
    }
}

/// Every built-in indicator kind with short periods, for engine-path equivalence checks.
fn every_indicator_kind() -> Vec<IndicatorKind> {
    use aeris_charts_indicators::{PivotKind, VwapReset};
    vec![
        IndicatorKind::Sma { period: 5 },
        IndicatorKind::Ema {
            period: 5,
            seed: IndicatorSeed::Sma,
        },
        IndicatorKind::Dema {
            period: 4,
            seed: IndicatorSeed::FirstValue,
        },
        IndicatorKind::Tema {
            period: 3,
            seed: IndicatorSeed::Sma,
        },
        IndicatorKind::Smma { period: 5 },
        IndicatorKind::Hma { period: 5 },
        IndicatorKind::Vwma { period: 5 },
        IndicatorKind::StandardDeviation { period: 5 },
        IndicatorKind::Cci { period: 5 },
        IndicatorKind::WilliamsR { period: 5 },
        IndicatorKind::StochasticRsi {
            rsi_period: 5,
            stochastic_period: 5,
        },
        IndicatorKind::Momentum { period: 5 },
        IndicatorKind::RateOfChange { period: 5 },
        IndicatorKind::Donchian { period: 5 },
        IndicatorKind::PivotPoints {
            variant: PivotKind::Camarilla,
        },
        IndicatorKind::ZigZag {
            deviation_percent: 3.0,
        },
        IndicatorKind::Keltner {
            period: 5,
            multiplier: 2.0,
        },
        IndicatorKind::AdxDmi { period: 5 },
        IndicatorKind::ParabolicSar,
        IndicatorKind::SuperTrend {
            period: 5,
            multiplier: 3.0,
        },
        IndicatorKind::Ichimoku,
        IndicatorKind::EmaRibbon {
            periods: [3, 5, 8, 13, 21],
        },
        IndicatorKind::Bollinger {
            period: 5,
            deviation: 2.0,
            estimator: DeviationEstimator::Sample,
        },
        IndicatorKind::Rsi {
            period: 5,
            seed: IndicatorSeed::Sma,
        },
        IndicatorKind::Macd {
            fast: 3,
            slow: 6,
            signal: 4,
            seed: IndicatorSeed::Sma,
            histogram_multiplier: 2.0,
        },
        IndicatorKind::Stochastic {
            k_period: 5,
            d_period: 3,
        },
        IndicatorKind::Atr { period: 5 },
        IndicatorKind::Vwap,
        IndicatorKind::Obv,
        IndicatorKind::Cmf { period: 5 },
        IndicatorKind::Mfi { period: 5 },
        IndicatorKind::Volume { period: 5 },
        IndicatorKind::VwapBands {
            reset: VwapReset::Session,
            standard_deviation: 1.0,
            percent: 5.0,
        },
        IndicatorKind::Wma { period: 5 },
        IndicatorKind::Kdj {
            period: 5,
            k_smoothing: 3,
            d_smoothing: 3,
            seed: aeris_charts_indicators::KdjSeed::Fifty,
        },
    ]
}

fn indicator_reads_volume(kind: &IndicatorKind) -> bool {
    matches!(
        kind,
        IndicatorKind::Vwma { .. }
            | IndicatorKind::Vwap
            | IndicatorKind::VwapBands { .. }
            | IndicatorKind::Obv
            | IndicatorKind::Cmf { .. }
            | IndicatorKind::Mfi { .. }
            | IndicatorKind::Volume { .. }
    )
}

/// One swinging bar per row, so ZigZag confirms reversals and pivots see several sessions.
fn swinging_bar(row: usize, revision: usize) -> [f64; 4] {
    let close = 100.0 + (row as f64 * 0.35).sin() * 8.0 + revision as f64 * 0.013;
    let open = close - (row as f64 * 0.9).cos() * 0.6;
    [open, open.max(close) + 0.7, open.min(close) - 0.5, close]
}

/// Compare every output of `binding` with the same study installed on a fresh chart from the
/// chart's current source, volume, and turnover data: incremental repairs must equal a full build.
fn assert_binding_matches_fresh_install(chart: &ChartEngine, binding: usize, label: &str) {
    let binding = &chart.indicators[binding];
    let mut fresh = ChartEngine::new(800.0, 500.0, 1.0);
    let copy = |fresh: &mut ChartEngine, from: SeriesId, into: SeriesId| {
        let (times, values) = chart.data.series_data(from).unwrap();
        let seconds = times.iter().map(|&time| time as f64).collect::<Vec<_>>();
        fresh
            .set_series_data(into, &seconds, values[0], values[1], values[2], values[3])
            .unwrap();
    };
    copy(&mut fresh, binding.source, 0);
    let mut weight = |column: Option<SeriesId>| {
        column.map(|id| {
            let fresh_id = fresh.add_series(SeriesKind::Histogram);
            copy(&mut fresh, id, fresh_id);
            fresh_id
        })
    };
    let volume = weight(binding.volume_source);
    let amount = weight(binding.amount_source);
    let outputs = fresh.add_indicator_kind_with_sources(
        0,
        binding.source_input,
        binding.kind.clone(),
        volume,
        amount,
    );
    assert_eq!(outputs.len(), binding.outputs.len(), "{label}");
    for (output, (&actual, &expected)) in binding.outputs.iter().zip(&outputs).enumerate() {
        let (actual_times, actual) = chart.data.series_data(actual).unwrap();
        let (expected_times, expected) = fresh.data.series_data(expected).unwrap();
        assert_eq!(
            actual_times, expected_times,
            "{label} output {output} times"
        );
        for (row, (&actual, &expected)) in actual[3].iter().zip(expected[3]).enumerate() {
            assert!(
                (actual.is_nan() && expected.is_nan()) || (actual - expected).abs() < 1e-10,
                "{label} output {output} row {row}: incremental {actual} != fresh {expected}"
            );
        }
    }
}

#[test]
fn streamed_aggregate_and_weight_inputs_match_a_fresh_install_for_every_indicator() {
    // Weight timelines: identical with the candle streaming first ("lagging" volume), identical
    // with the volume streaming first ("leading"), and genuinely different timestamps ("diverged":
    // missing every seventh bar plus bars the source lacks).
    let orders = ["lagging", "leading", "diverged"];
    let mut cases = Vec::new();
    for kind in every_indicator_kind() {
        let weighted = indicator_reads_volume(&kind);
        for input in [
            IndicatorInputSource::Close,
            IndicatorInputSource::Hl2,
            IndicatorInputSource::Hlc3,
            IndicatorInputSource::Ohlc4,
            IndicatorInputSource::Hlcc4,
        ] {
            for order in orders.iter().take(if weighted { 3 } else { 1 }) {
                cases.push((kind.clone(), input, *order, false));
            }
        }
    }
    for order in orders {
        cases.push((
            IndicatorKind::Vwap,
            IndicatorInputSource::Ohlc4,
            order,
            true,
        ));
    }
    for (kind, input, order, with_amount) in cases {
        let label = format!("{kind:?} {input:?} {order} amount={with_amount}");
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        let volume = chart.add_series(SeriesKind::Histogram);
        let amount = chart.add_series(SeriesKind::Histogram);
        let diverged = order == "diverged";
        let weight_rows = |rows: std::ops::Range<usize>| {
            rows.filter(|row| !diverged || row % 7 != 3)
                .map(|row| row as f64 * 3_600.0)
                .chain(
                    diverged
                        .then_some([10.5 * 3_600.0, 20.25 * 3_600.0])
                        .into_iter()
                        .flatten(),
                )
                .collect::<Vec<_>>()
        };
        let times = (0..40).map(|row| row as f64 * 3_600.0).collect::<Vec<_>>();
        let bars = (0..40).map(|row| swinging_bar(row, 0)).collect::<Vec<_>>();
        let column = |index: usize| bars.iter().map(|bar| bar[index]).collect::<Vec<_>>();
        chart
            .set_series_data(0, &times, &column(0), &column(1), &column(2), &column(3))
            .unwrap();
        let mut weight_times = weight_rows(0..40);
        weight_times.sort_by(f64::total_cmp);
        let volumes = weight_times
            .iter()
            .map(|time| (time / 3_600.0 * 7.0) % 11.0 + 1.0)
            .collect::<Vec<_>>();
        let amounts = volumes
            .iter()
            .map(|volume| volume * 101.0)
            .collect::<Vec<_>>();
        for (id, values) in [(volume, &volumes), (amount, &amounts)] {
            chart
                .set_series_data(id, &weight_times, values, values, values, values)
                .unwrap();
        }
        let outputs = chart.add_indicator_kind_with_sources(
            0,
            input,
            kind.clone(),
            indicator_reads_volume(&kind).then_some(volume),
            with_amount.then_some(amount),
        );
        assert!(!outputs.is_empty(), "{label}");
        let binding = chart.indicators.len() - 1;
        assert_binding_matches_fresh_install(&chart, binding, &label);

        let tick = |chart: &mut ChartEngine, row: usize, revision: usize| {
            let time = row as f64 * 3_600.0;
            let weight = ((row * 7 + revision) % 11 + 1) as f64;
            let candle = |chart: &mut ChartEngine| {
                chart.update_series_bar(0, time, swinging_bar(row, revision));
            };
            let weights = |chart: &mut ChartEngine| {
                if !diverged || row % 7 != 3 {
                    chart.update_series_bar(volume, time, [weight; 4]);
                    chart.update_series_bar(amount, time, [weight * 101.0; 4]);
                }
            };
            if order == "leading" {
                weights(chart);
                assert_binding_matches_fresh_install(chart, binding, &label);
                candle(chart);
            } else {
                candle(chart);
                assert_binding_matches_fresh_install(chart, binding, &label);
                weights(chart);
            }
            assert_binding_matches_fresh_install(chart, binding, &label);
        };
        for revision in 1..=3 {
            tick(&mut chart, 39, revision);
        }
        for row in 40..46 {
            tick(&mut chart, row, 0);
            tick(&mut chart, row, 1);
        }
        // Historical corrections of existing bars, then a bar at a time neither series had.
        tick(&mut chart, 17, 4);
        tick(&mut chart, 31, 2);
        chart.update_series_bar(0, 17.5 * 3_600.0, swinging_bar(17, 9));
        assert_binding_matches_fresh_install(&chart, binding, &label);
        chart.update_series_bar(volume, 17.5 * 3_600.0, [4.0; 4]);
        chart.update_series_bar(amount, 17.5 * 3_600.0, [404.0; 4]);
        assert_binding_matches_fresh_install(&chart, binding, &label);
        // Truncating the source leaves the weight series longer than the source.
        chart.series_pop(0, 3).unwrap();
        assert_binding_matches_fresh_install(&chart, binding, &label);
        tick(&mut chart, 43, 5);
        tick(&mut chart, 44, 0);
    }
}

/// Aggregate price columns the twin charts of `aggregate_twin_charts` retain.
const AGGREGATE_COLUMNS: usize = 4;

/// Bytes of capacity a chart of `rows` rows may keep in `AGGREGATE_COLUMNS` aggregate columns:
/// one eighth of the rows plus a fixed floor of spare rows each.
fn aggregate_column_bound(rows: usize) -> usize {
    AGGREGATE_COLUMNS * (rows + rows / 8 + 4096) * std::mem::size_of::<f64>()
}

/// Install `rows` real bars followed by `slots` whitespace rows on the primary series.
fn load_aggregate_source(chart: &mut ChartEngine, rows: usize, slots: usize) {
    let times = (0..rows + slots)
        .map(|row| row as f64 * 60.0)
        .collect::<Vec<_>>();
    let bars = (0..rows + slots)
        .map(|row| {
            if row < rows {
                swinging_bar(row, 0)
            } else {
                [f64::NAN; 4]
            }
        })
        .collect::<Vec<_>>();
    let column = |index: usize| bars.iter().map(|bar| bar[index]).collect::<Vec<_>>();
    chart
        .set_series_data(0, &times, &column(0), &column(1), &column(2), &column(3))
        .unwrap();
}

/// Two charts over the same source: four studies on a canonical input (no derived column) and the
/// same four on the four aggregate inputs. Every other runtime capacity cancels, so the
/// difference in indicator runtime bytes is the binding-private aggregate columns.
fn aggregate_twin_charts(rows: usize, slots: usize) -> (ChartEngine, ChartEngine) {
    let kinds = [
        IndicatorKind::Sma { period: 20 },
        IndicatorKind::Ema {
            period: 20,
            seed: IndicatorSeed::Sma,
        },
        IndicatorKind::Rsi {
            period: 14,
            seed: IndicatorSeed::Sma,
        },
        IndicatorKind::StochasticRsi {
            rsi_period: 14,
            stochastic_period: 14,
        },
    ];
    let aggregates = [
        IndicatorInputSource::Hl2,
        IndicatorInputSource::Hlc3,
        IndicatorInputSource::Ohlc4,
        IndicatorInputSource::Hlcc4,
    ];
    let mut canonical = ChartEngine::new(800.0, 500.0, 1.0);
    let mut composite = ChartEngine::new(800.0, 500.0, 1.0);
    load_aggregate_source(&mut canonical, rows, slots);
    load_aggregate_source(&mut composite, rows, slots);
    for (kind, input) in kinds.into_iter().zip(aggregates) {
        assert!(!canonical
            .add_indicator_kind_with_input(0, IndicatorInputSource::Close, kind.clone(), None)
            .is_empty());
        assert!(!composite
            .add_indicator_kind_with_input(0, input, kind, None)
            .is_empty());
    }
    (canonical, composite)
}

fn aggregate_columns_bytes(canonical: &ChartEngine, composite: &ChartEngine) -> usize {
    composite
        .memory_usage()
        .indicator_runtime_bytes
        .checked_sub(canonical.memory_usage().indicator_runtime_bytes)
        .expect("aggregate studies hold at least the canonical runtime state")
}

#[test]
fn aggregate_input_columns_keep_bounded_tail_headroom() {
    // Each aggregate column must be resident (an O(n) rebuild per tick is the alternative), must
    // not reallocate on the first live append, and must stay within one eighth plus a fixed floor
    // of spare rows however the source changes.
    const ROWS: usize = 100_000;
    let f64_bytes = std::mem::size_of::<f64>();
    let (mut canonical, mut composite) = aggregate_twin_charts(ROWS, 0);

    let installed = aggregate_columns_bytes(&canonical, &composite);
    assert!(
        installed >= AGGREGATE_COLUMNS * ROWS * f64_bytes,
        "aggregate columns are resident: {installed} bytes"
    );
    assert!(
        installed <= aggregate_column_bound(ROWS),
        "install keeps bounded headroom: {installed} > {}",
        aggregate_column_bound(ROWS)
    );

    // The first live append after the bulk install derives one row into spare capacity.
    let mut last = ROWS;
    for chart in [&mut canonical, &mut composite] {
        chart.update_series_bar(0, last as f64 * 60.0, swinging_bar(last, 0));
    }
    assert_eq!(
        aggregate_columns_bytes(&canonical, &composite),
        installed,
        "the first append reallocated an aggregate column"
    );
    for _ in 0..2_000 {
        last += 1;
        for chart in [&mut canonical, &mut composite] {
            chart.update_series_bar(0, last as f64 * 60.0, swinging_bar(last, 0));
        }
        assert!(
            aggregate_columns_bytes(&canonical, &composite) <= aggregate_column_bound(last + 1),
            "append {last} outgrew the headroom bound"
        );
    }

    // Replacing the source with far fewer rows releases the oversized columns.
    for chart in [&mut canonical, &mut composite] {
        load_aggregate_source(chart, 1_000, 0);
    }
    let replaced = aggregate_columns_bytes(&canonical, &composite);
    assert!(
        replaced >= AGGREGATE_COLUMNS * 1_000 * f64_bytes
            && replaced <= aggregate_column_bound(1_000),
        "data replacement kept {replaced} bytes of aggregate columns (bound {})",
        aggregate_column_bound(1_000)
    );
}

#[test]
fn filling_the_first_session_slot_keeps_aggregate_input_columns() {
    // A time-sharing chart installs the rest of the session as whitespace slots after its real
    // rows. The runtime covers the source through the last real row, so the first fill extends
    // each aggregate column by one row exactly like an append; it must not reallocate the column.
    const ROWS: usize = 100_000;
    const SLOTS: usize = 1_000;
    let (mut canonical, mut composite) = aggregate_twin_charts(ROWS, SLOTS);

    let installed = aggregate_columns_bytes(&canonical, &composite);
    assert!(
        installed >= AGGREGATE_COLUMNS * ROWS * std::mem::size_of::<f64>()
            && installed <= aggregate_column_bound(ROWS),
        "aggregate columns after the install: {installed} bytes"
    );
    for (filled, row) in (ROWS..ROWS + SLOTS).enumerate() {
        for chart in [&mut canonical, &mut composite] {
            chart.update_series_bar(0, row as f64 * 60.0, swinging_bar(row, 0));
        }
        let bytes = aggregate_columns_bytes(&canonical, &composite);
        if filled == 0 {
            assert_eq!(bytes, installed, "the first slot fill reallocated a column");
        }
        assert!(
            bytes <= aggregate_column_bound(row + 1),
            "filling slot {row} outgrew the headroom bound"
        );
    }
}

#[test]
fn indicator_ticks_over_100k_rows_do_bounded_engine_work() {
    // Every kind plus aggregate inputs, a volume stream in both update orders, and a diverged
    // turnover timeline: one tick must cost O(window) per binding, independent of history.
    const ROWS: usize = 100_000;
    const WORK_PER_BINDING: usize = 64;
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    let volume = chart.add_series(SeriesKind::Histogram);
    let gappy_volume = chart.add_series(SeriesKind::Histogram);
    let times = (0..ROWS).map(|row| row as f64 * 60.0).collect::<Vec<_>>();
    let bars = (0..ROWS)
        .map(|row| swinging_bar(row, 0))
        .collect::<Vec<_>>();
    let column = |index: usize| bars.iter().map(|bar| bar[index]).collect::<Vec<_>>();
    chart
        .set_series_data(0, &times, &column(0), &column(1), &column(2), &column(3))
        .unwrap();
    let volumes = (0..ROWS)
        .map(|row| (row % 13 + 1) as f64)
        .collect::<Vec<_>>();
    chart
        .set_series_data(volume, &times, &volumes, &volumes, &volumes, &volumes)
        .unwrap();
    let gappy_times = times
        .iter()
        .copied()
        .enumerate()
        .filter(|(row, _)| row % 50 != 7)
        .map(|(_, time)| time)
        .collect::<Vec<_>>();
    let gappy = vec![5.0; gappy_times.len()];
    chart
        .set_series_data(gappy_volume, &gappy_times, &gappy, &gappy, &gappy, &gappy)
        .unwrap();
    for kind in every_indicator_kind() {
        let weighted = indicator_reads_volume(&kind);
        assert!(!chart
            .add_indicator_kind(0, kind, weighted.then_some(volume))
            .is_empty());
    }
    for input in [
        IndicatorInputSource::Hl2,
        IndicatorInputSource::Hlc3,
        IndicatorInputSource::Ohlc4,
        IndicatorInputSource::Hlcc4,
    ] {
        assert!(!chart
            .add_indicator_kind_with_input(
                0,
                input,
                IndicatorKind::StochasticRsi {
                    rsi_period: 5,
                    stochastic_period: 5,
                },
                None,
            )
            .is_empty());
    }
    assert!(!chart
        .add_indicator_kind(0, IndicatorKind::Vwap, Some(gappy_volume))
        .is_empty());
    // Every binding's latest rebuild (formula rows plus derived input rows) stays within its
    // window, so a regression in one binding cannot hide behind the others' small ticks.
    let assert_bounded = |chart: &ChartEngine, mutation: &str| {
        for binding in &chart.indicators {
            assert!(
                binding.last_work_rows() <= WORK_PER_BINDING,
                "{mutation}: {:?} {:?} did {} work rows (bound {WORK_PER_BINDING})",
                binding.kind,
                binding.source_input,
                binding.last_work_rows()
            );
        }
    };

    let mut last = ROWS - 1;
    for (step, leading) in [false, true, false, true].into_iter().enumerate() {
        for revision in 1..=3 {
            let time = last as f64 * 60.0;
            chart.update_series_bar(0, time, swinging_bar(last, revision + step));
            assert_bounded(&chart, "replace candle");
            chart.update_series_bar(volume, time, [revision as f64; 4]);
            assert_bounded(&chart, "replace volume");
        }
        last += 1;
        let time = last as f64 * 60.0;
        let updates: [(SeriesId, [f64; 4]); 3] = [
            (volume, [3.0; 4]),
            (gappy_volume, [5.0; 4]),
            (0, swinging_bar(last, 0)),
        ];
        let order: Vec<_> = if leading {
            updates.to_vec()
        } else {
            updates.iter().rev().copied().collect()
        };
        for (series, values) in order {
            chart.update_series_bar(series, time, values);
            assert_bounded(&chart, &format!("append (volume leading: {leading})"));
        }
    }
    // Closing the current bar and opening the next one in a single batch.
    let bars = [swinging_bar(last, 9), swinging_bar(last + 1, 0)];
    let column = |index: usize| bars.iter().map(|bar| bar[index]).collect::<Vec<_>>();
    assert_eq!(
        chart.update_series_bars_sanitized(
            0,
            vec![last as i64 * 60, (last + 1) as i64 * 60],
            column(0),
            column(1),
            column(2),
            column(3),
        ),
        2
    );
    assert_bounded(&chart, "batch close and open");
    chart.update_series_bar(volume, (last + 1) as f64 * 60.0, [4.0; 4]);
    assert_bounded(&chart, "volume for the batch-opened bar");
    // The bounded paths still produce full-rebuild values: the path-dependent ZigZag, an
    // aggregate-input Stochastic RSI, and the VWAP over the diverged volume timeline.
    for binding in 0..chart.indicators.len() {
        let binding_info = &chart.indicators[binding];
        if matches!(binding_info.kind, IndicatorKind::ZigZag { .. })
            || binding_info.source_input == IndicatorInputSource::Hlc3
            || binding_info.volume_source == Some(gappy_volume)
        {
            let label = format!("{:?} {:?}", binding_info.kind, binding_info.source_input);
            assert_binding_matches_fresh_install(&chart, binding, &label);
        }
    }
}

#[test]
fn indicator_ticks_filling_pre_installed_session_slots_do_bounded_work() {
    // A time-sharing chart: real rows, then the rest of the session installed as whitespace
    // slots. Filling and revising the forming slot must cost the window, not the slots.
    const ROWS: usize = 20_000;
    const SLOTS: usize = 5_000;
    const WORK_PER_BINDING: usize = 64;
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    let volume = chart.add_series(SeriesKind::Histogram);
    let times = (0..ROWS + SLOTS)
        .map(|row| row as f64 * 60.0)
        .collect::<Vec<_>>();
    let bars = (0..ROWS + SLOTS)
        .map(|row| {
            if row < ROWS {
                swinging_bar(row, 0)
            } else {
                [f64::NAN; 4]
            }
        })
        .collect::<Vec<_>>();
    let column = |index: usize| bars.iter().map(|bar| bar[index]).collect::<Vec<_>>();
    chart
        .set_series_data(0, &times, &column(0), &column(1), &column(2), &column(3))
        .unwrap();
    let volumes = (0..ROWS + SLOTS)
        .map(|row| {
            if row < ROWS {
                (row % 13 + 1) as f64
            } else {
                f64::NAN
            }
        })
        .collect::<Vec<_>>();
    chart
        .set_series_data(volume, &times, &volumes, &volumes, &volumes, &volumes)
        .unwrap();
    for kind in every_indicator_kind() {
        let weighted = indicator_reads_volume(&kind);
        assert!(!chart
            .add_indicator_kind(0, kind, weighted.then_some(volume))
            .is_empty());
    }
    let rsi = chart.add_indicator_kind(
        0,
        IndicatorKind::Rsi {
            period: 5,
            seed: IndicatorSeed::Sma,
        },
        None,
    )[0];
    assert!(!chart
        .add_indicator_kind(
            rsi,
            IndicatorKind::Ema {
                period: 4,
                seed: IndicatorSeed::Sma
            },
            None
        )
        .is_empty());
    let assert_bounded = |chart: &ChartEngine, mutation: &str| {
        for binding in &chart.indicators {
            assert!(
                binding.last_work_rows() <= WORK_PER_BINDING,
                "{mutation}: {:?} did {} work rows (bound {WORK_PER_BINDING})",
                binding.kind,
                binding.last_work_rows()
            );
        }
    };
    for (step, row) in (ROWS..ROWS + 4).enumerate() {
        let time = row as f64 * 60.0;
        let leading = step % 2 == 1;
        if leading {
            chart.update_series_bar(volume, time, [3.0; 4]);
            assert_bounded(&chart, "volume fills its slot first");
        }
        for revision in 0..3 {
            chart.update_series_bar(0, time, swinging_bar(row, revision));
            assert_bounded(&chart, "fill or revise the forming slot");
        }
        if !leading {
            chart.update_series_bar(volume, time, [4.0; 4]);
            assert_bounded(&chart, "volume fills its slot after the candle");
        }
    }
    // A tick that clears the forming bar back to whitespace, then fills it again.
    let time = (ROWS + 3) as f64 * 60.0;
    chart.update_series_bar(0, time, [f64::NAN; 4]);
    assert_bounded(&chart, "clear the forming bar");
    chart.update_series_bar(0, time, swinging_bar(ROWS + 3, 9));
    assert_bounded(&chart, "refill the forming bar");
    for binding in 0..chart.indicators.len() {
        assert_binding_matches_fresh_install(&chart, binding, "after filling session slots");
    }
}

/// A chart of `rows` swinging one-minute bars with a volume and a turnover series, every
/// built-in study (weighted ones on the volume), a turnover-weighted VWAP, and a study chained on
/// an RSI output.
fn replay_study_chart(rows: usize) -> (ChartEngine, SeriesId) {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    let volume = chart.add_series(SeriesKind::Histogram);
    let amount = chart.add_series(SeriesKind::Histogram);
    let times = (0..rows).map(|row| row as f64 * 60.0).collect::<Vec<_>>();
    let bars = (0..rows)
        .map(|row| swinging_bar(row, 0))
        .collect::<Vec<_>>();
    let column = |index: usize| bars.iter().map(|bar| bar[index]).collect::<Vec<_>>();
    chart
        .set_series_data(0, &times, &column(0), &column(1), &column(2), &column(3))
        .unwrap();
    let volumes = (0..rows)
        .map(|row| (row % 13 + 1) as f64)
        .collect::<Vec<_>>();
    chart
        .set_series_data(volume, &times, &volumes, &volumes, &volumes, &volumes)
        .unwrap();
    let amounts = volumes
        .iter()
        .map(|volume| volume * 101.0)
        .collect::<Vec<_>>();
    chart
        .set_series_data(amount, &times, &amounts, &amounts, &amounts, &amounts)
        .unwrap();
    for kind in every_indicator_kind() {
        let weighted = indicator_reads_volume(&kind);
        assert!(!chart
            .add_indicator_kind(0, kind, weighted.then_some(volume))
            .is_empty());
    }
    assert!(!chart
        .add_indicator_kind_with_sources(
            0,
            IndicatorInputSource::Hlc3,
            IndicatorKind::Vwap,
            Some(volume),
            Some(amount),
        )
        .is_empty());
    let rsi = chart.add_indicator_kind(
        0,
        IndicatorKind::Rsi {
            period: 5,
            seed: IndicatorSeed::Sma,
        },
        None,
    )[0];
    assert!(!chart
        .add_indicator_kind(rsi, IndicatorKind::Sma { period: 3 }, None)
        .is_empty());
    (chart, volume)
}

#[test]
fn one_bar_replay_steps_do_bounded_indicator_work() {
    const ROWS: usize = 100_000;
    const WORK_PER_BINDING: usize = 64;
    let (mut chart, _) = replay_study_chart(ROWS);
    let clock = |row: usize| Some(row as i64 * 60 * 1_000_000);
    chart.set_replay_clock_micros(clock(90_000)).unwrap();
    for row in 90_001..90_006 {
        chart.set_replay_clock_micros(clock(row)).unwrap();
        for binding in &chart.indicators {
            assert!(
                binding.last_work_rows() <= WORK_PER_BINDING,
                "a one-bar step: {:?} did {} work rows (bound {WORK_PER_BINDING})",
                binding.kind,
                binding.last_work_rows()
            );
        }
    }
    for binding in 0..chart.indicators.len() {
        assert_binding_matches_fresh_install(&chart, binding, "after one-bar steps");
    }
}

#[test]
fn replay_seeks_in_both_directions_match_a_fresh_install_for_every_indicator() {
    let (mut chart, _) = replay_study_chart(240);
    let clock = |row: i64| Some(row * 60 * 1_000_000);
    // Forward, one bar and many; backward, one bar and many; before the data; cleared.
    for (step, target) in [
        clock(120),
        clock(121),
        clock(160),
        clock(159),
        clock(90),
        clock(91),
        clock(-5),
        clock(40),
        None,
        clock(200),
        None,
    ]
    .into_iter()
    .enumerate()
    {
        chart.set_replay_clock_micros(target).unwrap();
        for binding in 0..chart.indicators.len() {
            assert_binding_matches_fresh_install(
                &chart,
                binding,
                &format!("seek {step} to {target:?}"),
            );
        }
    }
}

#[test]
fn batch_and_single_updates_are_semantically_identical_for_every_indicator() {
    let kinds = [
        IndicatorKind::Sma { period: 5 },
        IndicatorKind::Ema {
            period: 5,
            seed: IndicatorSeed::Sma,
        },
        IndicatorKind::EmaRibbon {
            periods: [3, 5, 8, 13, 21],
        },
        IndicatorKind::Bollinger {
            period: 5,
            deviation: 2.0,
            estimator: DeviationEstimator::Population,
        },
        IndicatorKind::Rsi {
            period: 5,
            seed: IndicatorSeed::Sma,
        },
        IndicatorKind::Macd {
            fast: 3,
            slow: 6,
            signal: 4,
            seed: IndicatorSeed::Sma,
            histogram_multiplier: 1.0,
        },
        IndicatorKind::Stochastic {
            k_period: 5,
            d_period: 3,
        },
        IndicatorKind::Atr { period: 5 },
        IndicatorKind::Vwap,
        IndicatorKind::Wma { period: 5 },
    ];
    for kind in kinds {
        let setup = || {
            let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
            let volume = chart.add_series(SeriesKind::Histogram);
            let times = (0..40)
                .map(|index| index as f64 * 3_600.0)
                .collect::<Vec<_>>();
            let close = (0..40)
                .map(|index| 90.0 + index as f64 * 0.4)
                .collect::<Vec<_>>();
            let high = close.iter().map(|value| value + 2.0).collect::<Vec<_>>();
            let low = close.iter().map(|value| value - 1.0).collect::<Vec<_>>();
            let volumes = (0..40).map(|index| (index % 7) as f64).collect::<Vec<_>>();
            chart
                .set_series_data(0, &times, &close, &high, &low, &close)
                .unwrap();
            chart
                .set_series_data(volume, &times, &volumes, &volumes, &volumes, &volumes)
                .unwrap();
            add_test_indicator(&mut chart, &kind, Some(volume));
            chart
        };
        let mut singles = setup();
        let mut batch = setup();
        let rows = [
            (41.0 * 3_600.0, [106.0, 108.0, 105.0, 107.0]),
            (10.0 * 3_600.0, [95.0, 99.0, 94.0, 98.0]),
            (f64::NAN, [1.0; 4]),
            (10.0 * 3_600.0, [96.0, 100.0, 95.0, 99.0]),
            (42.0 * 3_600.0, [107.0, 109.0, 106.0, 108.0]),
        ];
        for (time, values) in rows {
            singles.update_series_bar(0, time, values);
        }
        assert_eq!(batch.update_series_bars(0, rows), 4);

        assert_eq!(
            singles.data.time_points_generation(),
            batch.data.time_points_generation()
        );
        assert_eq!(singles.data.merged_times(), batch.data.merged_times());
        assert_eq!(singles.series_order(), batch.series_order());
        for &series in singles.series_order() {
            let single = singles.data.series_data(series).unwrap();
            let batched = batch.data.series_data(series).unwrap();
            assert_eq!(single.0, batched.0);
            assert_eq!(single.1, batched.1);
        }

        singles.time_scale.set_width(800.0);
        batch.time_scale.set_width(800.0);
        singles.fit_content();
        batch.fit_content();
        assert_eq!(
            singles.tick_marks.build(6.0, 50.0),
            batch.tick_marks.build(6.0, 50.0)
        );
        assert_eq!(singles.build_frame(), batch.build_frame());
        for (single, batched) in singles.panes.iter().zip(&batch.panes) {
            assert_eq!(
                single.price_scale.price_range(),
                batched.price_scale.price_range()
            );
        }
    }
}

#[test]
fn vwap_volume_catchup_and_replacement_resume_at_the_affected_row() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    let volume = chart.add_series(SeriesKind::Histogram);
    let times = (0..10).map(|index| index as f64 * 60.0).collect::<Vec<_>>();
    let close = (0..10)
        .map(|index| 100.0 + index as f64)
        .collect::<Vec<_>>();
    let high = close.iter().map(|value| value + 1.0).collect::<Vec<_>>();
    let low = close.iter().map(|value| value - 1.0).collect::<Vec<_>>();
    chart
        .set_series_data(0, &times, &close, &high, &low, &close)
        .unwrap();
    chart
        .set_series_data(
            volume,
            &times[..5],
            &[1.0; 5],
            &[1.0; 5],
            &[1.0; 5],
            &[1.0; 5],
        )
        .unwrap();
    chart.add_vwap(0, Some(volume)).unwrap();
    let binding = chart.indicators.len() - 1;
    assert_indicator_binding_matches_full(&chart, binding);

    let added_times = (5..10).map(|index| index * 60).collect::<Vec<_>>();
    let added = [2.0, 0.0, 4.0, 5.0, 6.0];
    chart.update_series_bars_sanitized(
        volume,
        added_times,
        added.to_vec(),
        added.to_vec(),
        added.to_vec(),
        added.to_vec(),
    );
    assert_indicator_binding_matches_full(&chart, binding);
    chart.update_series_bar(volume, 9.0 * 60.0, [8.0; 4]);
    assert_indicator_binding_matches_full(&chart, binding);
}

#[test]
fn vwap_volume_input_aligns_by_timestamp_instead_of_row_position() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    let volume = chart.add_series(SeriesKind::Histogram);
    let source_times = [0.0, 60.0, 120.0, 180.0];
    let source = [10.0, 20.0, 30.0, 40.0];
    chart
        .set_series_data(0, &source_times, &source, &source, &source, &source)
        .unwrap();
    // The volume rows are deliberately shifted and sparse. Row-position pairing would apply
    // 5.0 to time 0 and 7.0 to time 60; timestamp pairing applies unit weight to both gaps.
    chart
        .set_series_data(
            volume,
            &[60.0, 180.0],
            &[5.0, 7.0],
            &[5.0, 7.0],
            &[5.0, 7.0],
            &[5.0, 7.0],
        )
        .unwrap();
    let vwap = chart.add_vwap(0, Some(volume)).unwrap();
    let actual = chart.data.series_data(vwap).unwrap().1[3].to_vec();
    let expected = aeris_charts_indicators::vwap(
        &[0, 60, 120, 180],
        &source,
        &source,
        &source,
        &[1.0, 5.0, 1.0, 7.0],
    );
    for (actual, expected) in actual.iter().zip(expected) {
        assert!((actual - expected.unwrap()).abs() < 1e-12);
    }
}

#[test]
fn vwap_bands_are_engine_owned_five_outputs_and_rebuild_equivalent() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    let volume = chart.add_series(SeriesKind::Histogram);
    let times = [1_704_067_200.0, 1_704_153_600.0, 1_706_745_600.0];
    let close = [10.0, 14.0, 20.0];
    chart
        .set_series_data(0, &times, &close, &close, &close, &close)
        .unwrap();
    chart
        .set_series_data(
            volume,
            &times,
            &[1.0, 3.0, 2.0],
            &[1.0, 3.0, 2.0],
            &[1.0, 3.0, 2.0],
            &[1.0, 3.0, 2.0],
        )
        .unwrap();
    let outputs = chart.add_vwap_bands(0, Some(volume), VwapReset::Monthly, 1.0, 10.0);
    assert_eq!(outputs.len(), 5);
    assert_eq!(chart.indicator_info(outputs[0]).unwrap().kind, "vwap_bands");
    let basis = chart.data.series_data(outputs[0]).unwrap().1[3].to_vec();
    assert_eq!(basis, vec![10.0, 13.0, 20.0]);
    let binding = chart.indicators.len() - 1;
    chart.update_series_bar(0, times[1], [15.0; 4]);
    assert_indicator_binding_matches_full(&chart, binding);
}

#[test]
fn chained_indicators_propagate_every_source_change_and_remove_together() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    let times = (0..20).map(|index| index as f64 * 60.0).collect::<Vec<_>>();
    let close = (0..20)
        .map(|index| 100.0 + index as f64)
        .collect::<Vec<_>>();
    chart
        .set_series_data(0, &times, &close, &close, &close, &close)
        .unwrap();
    let sma = chart.add_sma(0, 3).unwrap();
    let ema = chart.add_ema(sma, 2).unwrap();
    let wma = chart.add_wma(ema, 2).unwrap();
    for binding in 0..chart.indicators.len() {
        assert_indicator_binding_matches_full(&chart, binding);
    }

    let generation = chart.data.series_generation(wma).unwrap();
    chart.update_series_bar(0, 19.0 * 60.0, [125.0; 4]);
    assert!(chart.data.series_generation(wma).unwrap() > generation);
    for binding in 0..chart.indicators.len() {
        assert_indicator_binding_matches_full(&chart, binding);
    }

    chart.update_series_bar(0, 20.0 * 60.0, [126.0; 4]);
    chart.update_series_bar(0, 7.0 * 60.0, [111.0; 4]);
    for binding in 0..chart.indicators.len() {
        assert_indicator_binding_matches_full(&chart, binding);
    }

    let replacement_times = (0..12)
        .map(|index| 86_400.0 + index as f64 * 60.0)
        .collect::<Vec<_>>();
    let replacement = (0..12)
        .map(|index| 75.0 + index as f64 * 0.5)
        .collect::<Vec<_>>();
    chart
        .set_series_data(
            0,
            &replacement_times,
            &replacement,
            &replacement,
            &replacement,
            &replacement,
        )
        .unwrap();
    for binding in 0..chart.indicators.len() {
        assert_indicator_binding_matches_full(&chart, binding);
    }

    let mut removed = chart.remove_series_tracked(sma);
    removed.sort_unstable();
    let mut expected = vec![sma, ema, wma];
    expected.sort_unstable();
    assert_eq!(removed, expected);
    assert!(chart.indicators.is_empty());
}

#[test]
fn host_formatters_override_builtin_labels() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    chart.series[0].kind = SeriesKind::Line;
    chart
        .set_series_data(
            0,
            &[1.0, 2.0, 3.0],
            &[10.0, 11.0, 12.0],
            &[11.0, 12.0, 13.0],
            &[9.0, 10.0, 11.0],
            &[10.5, 11.5, 12.5],
        )
        .unwrap();
    chart.time_scale.set_width(800.0);
    chart.fit_content();
    chart.crosshair = Some((200.0, 120.0));

    // priceFormatter prefixes every non-percentage price label; tickMarkFormatter tags the tick
    // type; timeFormatter replaces the crosshair time label.
    chart.set_price_formatter(Some(Box::new(|price| Some(format!("${price:.0}")))));
    chart.set_tick_mark_formatter(Some(Box::new(|_ts, kind| Some(format!("T{kind}")))));
    chart.set_time_formatter(Some(Box::new(|_ts| Some("XHAIR".to_string()))));

    let axis = chart.build_axis_frame(
        80.0,
        |t, _bold| t.len() as f64 * 7.0,
        |t, _bold| t.len() as f64 * 6.0,
    );
    assert!(axis.labels.iter().any(|l| l.text.starts_with('$')));
    let time_ticks: Vec<_> = axis
        .labels
        .iter()
        .filter(|l| l.align == AxisTextAlign::Center && l.midpoint == AxisTextMidpoint::None)
        .collect();
    assert!(
        !time_ticks.is_empty(),
        "expected at least one time tick label"
    );
    assert!(time_ticks.iter().all(|l| l.text.starts_with('T')));
    assert!(axis
        .labels
        .iter()
        .any(|l| l.midpoint == AxisTextMidpoint::StableTime && l.text == "XHAIR"));

    // Clearing a formatter restores the built-in output.
    chart.set_price_formatter(None);
    let axis = chart.build_axis_frame(
        80.0,
        |t, _bold| t.len() as f64 * 7.0,
        |t, _bold| t.len() as f64 * 6.0,
    );
    assert!(!axis.labels.iter().any(|l| l.text.starts_with('$')));
}

#[test]
fn time_scale_option_setters_are_headless_and_clamp() {
    let mut chart = ChartEngine::new(300.0, 200.0, 1.0);
    chart
        .set_series_data(
            0,
            &[10.0, 20.0, 30.0],
            &[1.0, 2.0, 3.0],
            &[1.0, 2.0, 3.0],
            &[1.0, 2.0, 3.0],
            &[1.0, 2.0, 3.0],
        )
        .unwrap();
    chart.time_scale.set_width(300.0);
    chart.fit_content();

    // minBarSpacing floors how far the scale can zoom out.
    chart.set_min_bar_spacing(20.0);
    chart.set_bar_spacing(1.0);
    assert!(chart.bar_spacing() >= 20.0);

    // fixRightEdge pins the right offset to zero (no future whitespace).
    chart.set_fix_right_edge(true);
    chart.set_right_offset(5.0);
    assert_eq!(chart.right_offset(), 0.0);

    // timeVisible/secondsVisible are recorded on the engine and drive label formatting.
    chart.set_time_visible(true);
    chart.set_seconds_visible(true);
    assert!(chart.time_visible);
    assert!(chart.seconds_visible);
}

#[test]
fn interaction_disabled_flag_reaches_the_time_scale() {
    let mut chart = ChartEngine::new(300.0, 200.0, 1.0);
    chart
        .set_series_data(
            0,
            &[10.0, 20.0, 30.0],
            &[1.0, 2.0, 3.0],
            &[1.0, 2.0, 3.0],
            &[1.0, 2.0, 3.0],
            &[1.0, 2.0, 3.0],
        )
        .unwrap();
    chart.time_scale.set_width(300.0);
    chart.fit_content();
    chart.set_bar_spacing(10.0);

    // reference `_isAllScalingAndScrollingDisabled` (time-scale.ts:975-986): the aggregate only
    // feeds tick-label alignment — it must NOT force fix-edge semantics on the scale math, so
    // offsets and spacing stay put (a non-interactive chart never reacts to resizes).
    chart.set_interaction_disabled(true);
    assert!(chart.time_scale.interaction_disabled());
    chart.set_right_offset(5.0);
    assert_eq!(chart.right_offset(), 5.0);
    chart.set_interaction_disabled(false);
    assert!(!chart.time_scale.interaction_disabled());
    chart.set_bar_spacing(10.0);
    chart.set_right_offset(5.0);
    assert_eq!(chart.right_offset(), 5.0);
}

#[test]
fn remove_series_releases_slot_and_drops_derived_indicators() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    chart
        .set_series_data(
            0,
            &[1.0, 2.0],
            &[10.0, 11.0],
            &[10.0, 11.0],
            &[10.0, 11.0],
            &[10.0, 11.0],
        )
        .unwrap();
    // A second series with a far larger range, plus an indicator derived from it.
    let extra = chart.add_series(SeriesKind::Line);
    chart
        .set_series_data(
            extra,
            &[1.0, 2.0],
            &[1000.0, 1001.0],
            &[1000.0, 1001.0],
            &[1000.0, 1001.0],
            &[1000.0, 1001.0],
        )
        .unwrap();
    let sma = chart.add_sma(extra, 2).expect("valid indicator");
    assert_eq!(chart.series_kind(sma), Some(SeriesKind::Line));

    chart.time_scale.set_width(800.0);
    chart.fit_content();
    chart.autoscale_visible();
    // The extra series drives autoscale up to 1001 before removal.
    assert_eq!(
        chart.panes[0]
            .price_scale
            .price_range()
            .unwrap()
            .max_value(),
        1001.0
    );

    // Removing it succeeds, reports absent, and cascades to its derived indicator.
    assert!(chart.remove_series(extra));
    assert_eq!(chart.series_kind(extra), None);
    assert_eq!(chart.series_kind(sma), None);
    assert!(chart.series_data(extra).is_empty());
    // Idempotent â€” removing the same series twice reports false.
    assert!(!chart.remove_series(extra));

    // The released slot is inert: autoscale now reflects only the primary series.
    chart.autoscale_visible();
    assert_eq!(
        chart.panes[0]
            .price_scale
            .price_range()
            .unwrap()
            .max_value(),
        11.0
    );

    // Data mutations through a removed identity fail and can never silently revive it.
    assert!(!chart.update_series_bar(extra, 3.0, [5.0, 5.0, 5.0, 5.0]));
    let error = chart
        .set_series_data(extra, &[3.0], &[5.0], &[5.0], &[5.0], &[5.0])
        .unwrap_err();
    assert_eq!(error, ValidationError::StaleSeries(extra));
    assert!(chart.series_data(extra).is_empty());

    // reference `removeSeries` accepts any series: even the primary (id 0) can be released.
    assert!(chart.remove_series(0));
    assert!(!chart.remove_series(0));
}

#[test]
fn unknown_series_id_is_recoverable_and_does_not_mutate() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    let times_before = chart.data.merged_times().to_vec();
    let order_before = chart.series_order().to_vec();

    assert_eq!(
        chart.validate_series_id(u32::MAX),
        Err(SeriesIdError::Unknown(u32::MAX))
    );
    assert!(!chart.update_series_bar(u32::MAX, 1.0, [1.0; 4]));
    assert_eq!(
        chart
            .set_series_data(u32::MAX, &[1.0], &[1.0], &[1.0], &[1.0], &[1.0])
            .unwrap_err(),
        ValidationError::UnknownSeries(u32::MAX)
    );
    assert_eq!(chart.data.merged_times(), times_before);
    assert_eq!(chart.series_order(), order_before);
}

#[test]
fn stale_series_identity_never_mutates_reused_storage() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    let old = chart.add_series(SeriesKind::Line);
    chart
        .set_series_data(old, &[1.0], &[10.0], &[10.0], &[10.0], &[10.0])
        .unwrap();
    let old_slot = chart.data.series_slot(old).unwrap();
    assert!(chart.remove_series(old));

    let replacement = chart.add_series(SeriesKind::Line);
    assert_ne!(replacement, old);
    assert_eq!(chart.data.series_slot(replacement), Some(old_slot));
    chart
        .set_series_data(replacement, &[2.0], &[20.0], &[20.0], &[20.0], &[20.0])
        .unwrap();

    assert_eq!(
        chart.validate_series_id(old),
        Err(SeriesIdError::Stale(old))
    );
    assert!(!chart.update_series_bar(old, 3.0, [30.0; 4]));
    assert_eq!(
        chart
            .set_series_data(old, &[3.0], &[30.0], &[30.0], &[30.0], &[30.0])
            .unwrap_err(),
        ValidationError::StaleSeries(old)
    );
    let (_, columns) = chart.data.series_data(replacement).unwrap();
    assert_eq!(columns[3], &[20.0]);
}

#[test]
fn ten_thousand_add_remove_cycles_reuse_bounded_storage() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    let first = chart.add_series(SeriesKind::Line);
    assert!(chart.remove_series(first));

    for value in 0..10_000 {
        let id = chart.add_series(SeriesKind::Line);
        assert_ne!(id, first);
        assert!(chart.update_series_bar(id, value as f64, [value as f64; 4]));
        assert!(chart.remove_series(id));
    }

    assert_eq!(chart.data.series_count(), 1);
    assert_eq!(chart.data.slot_count(), 2);
    assert_eq!(chart.series.len(), 2);
    assert_eq!(
        chart.validate_series_id(first),
        Err(SeriesIdError::Stale(first))
    );
    assert!(!chart.update_series_bar(first, 20_000.0, [9.0; 4]));
    assert!(chart.data.series_data(first).is_none());
}

#[test]
fn indicator_outputs_drop_the_countdown_show_the_name_chip_and_default_to_2px() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    let values = [1.0, 2.0, 3.0, 4.0, 5.0];
    chart
        .set_series_data(
            0,
            &[1.0, 2.0, 3.0, 4.0, 5.0],
            &values,
            &values,
            &values,
            &values,
        )
        .unwrap();
    let rsi = chart.add_rsi(0, 2).expect("valid rsi");
    let entry = chart.series.iter().find(|s| s.id == rsi).unwrap();
    // No candle countdown on a line value, the auto name chip shows, 1px line.
    assert!(!entry.countdown_visible);
    assert!(entry.title_visible);
    assert_eq!(entry.title, "RSI 2");
    assert_eq!(entry.line_width, Some(2.0));

    // The whole native set auto-names itself for the platform's chips.
    let sma = chart.add_sma(0, 2).unwrap();
    let ema = chart.add_ema(0, 2).unwrap();
    let bands = chart.add_bollinger(0, 3, 2.0);
    let bands_frac = chart.add_bollinger(0, 3, 2.5);
    let macd = chart.add_macd(0, 2, 3, 2);
    let stoch = chart.add_stochastic(0, 2, 3);
    let atr = chart.add_atr(0, 2).unwrap();
    let vwap = chart.add_vwap(0, None).unwrap();
    let wma = chart.add_wma(0, 3).unwrap();
    let title_of = |id: SeriesId| {
        chart
            .series
            .iter()
            .find(|s| s.id == id)
            .unwrap()
            .title
            .clone()
    };
    assert_eq!(title_of(sma), "SMA 2");
    assert_eq!(title_of(ema), "EMA 2");
    assert_eq!(title_of(bands[0]), "Bollinger 3 2");
    assert_eq!(title_of(bands_frac[0]), "Bollinger 3 2.5");
    assert_eq!(title_of(macd[0]), "MACD 2 3 2");
    assert_eq!(title_of(stoch[0]), "Stochastic 2 3");
    assert_eq!(title_of(atr), "ATR 2");
    assert_eq!(title_of(vwap), "VWAP");
    assert_eq!(title_of(wma), "WMA 3");
}

#[test]
fn a_dragged_short_pane_contracts_its_scale_instead_of_flipping_it() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    let values = [1.0, 2.0, 3.0, 4.0, 5.0, 4.0, 3.0, 2.0];
    chart
        .set_series_data(
            0,
            &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0],
            &values,
            &values,
            &values,
            &values,
        )
        .unwrap();
    chart.add_rsi(0, 2).expect("valid rsi");
    chart.time_scale.set_width(800.0);
    chart.fit_content();

    let upright = |chart: &ChartEngine, pi: usize| {
        let scale = &chart.panes[pi].price_scale;
        let range = *scale.price_range().expect("a price range");
        scale.internal_height() > 0.0
            && scale.price_to_coordinate(range.max_value(), 0.0)
                < scale.price_to_coordinate(range.min_value(), 0.0)
    };

    // Default oscillator stretch: the small pane keeps an upright, positive-height mapping.
    chart.layout_panes(400.0);
    chart.autoscale_visible();
    assert!(upright(&chart, 0), "main pane upright");
    assert!(upright(&chart, 1), "indicator pane upright");

    // Squeeze the main pane to a strip and grow the indicator pane (the separator-drag
    // extremes): both scales contract but never flip.
    chart.panes[0].stretch_factor = 0.2;
    chart.panes[1].stretch_factor = 2.0;
    chart.layout_panes(400.0);
    chart.autoscale_visible();
    assert!(upright(&chart, 0), "squeezed main pane stays upright");
    assert!(upright(&chart, 1), "grown indicator pane stays upright");
    // The fractional margins (0.2 + 0.1) resolve against the pane's own slot: the internal
    // height is 70% of the pane height regardless of the total content height.
    let slot = chart.panes[1].height;
    assert!((chart.panes[1].price_scale.internal_height() - 0.7 * slot).abs() < 1e-9);
}

#[test]
fn oscillator_indicators_get_their_own_pane_and_band_levels() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    chart
        .set_series_data(
            0,
            &[1.0, 2.0, 3.0, 4.0, 5.0],
            &[1.0, 2.0, 3.0, 4.0, 5.0],
            &[1.0, 2.0, 3.0, 4.0, 5.0],
            &[1.0, 2.0, 3.0, 4.0, 5.0],
            &[1.0, 2.0, 3.0, 4.0, 5.0],
        )
        .unwrap();
    let rsi = chart.add_rsi(0, 2).expect("valid rsi");
    // A fresh oscillator pane holds the output at the reduced stretch.
    assert_eq!(chart.panes.len(), 2);
    assert!((chart.panes[1].stretch_factor - 0.3).abs() < 1e-9);
    let entry = chart.series.iter().find(|s| s.id == rsi).unwrap();
    assert_eq!(entry.pane_index, 1);
    assert_eq!(
        entry.threshold_region,
        Some(SeriesThresholdRegion {
            lower: 30.0,
            upper: 70.0,
        })
    );
    assert!(entry.price_lines.is_empty());
    assert_eq!(chart.indicator_info(rsi).unwrap().kind, "rsi");
}

#[test]
fn macd_outputs_are_line_line_histogram_with_four_state_colors() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    let values = [1.0, 2.0, 3.0, 4.0, 3.0, 2.0, 1.0, 2.0, 3.0, 4.0];
    chart
        .set_series_data(
            0,
            &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0],
            &values,
            &values,
            &values,
            &values,
        )
        .unwrap();
    let ids = chart.add_macd(0, 2, 3, 2);
    assert_eq!(ids.len(), 3);
    assert_eq!(chart.series_kind(ids[0]), Some(SeriesKind::Line));
    assert_eq!(chart.series_kind(ids[1]), Some(SeriesKind::Line));
    assert_eq!(chart.series_kind(ids[2]), Some(SeriesKind::Histogram));
    // All three live in the same new oscillator pane.
    assert!(ids
        .iter()
        .all(|&id| chart.series.iter().find(|s| s.id == id).unwrap().pane_index == 1));
    // Output slots and the packed signal period.
    assert_eq!(chart.indicator_info(ids[0]).unwrap().output_index, 0);
    assert_eq!(chart.indicator_info(ids[2]).unwrap().output_index, 2);
    let info = chart.indicator_info(ids[0]).unwrap();
    assert_eq!(
        (info.kind, info.period, info.deviation),
        ("macd", 3, Some(2.0))
    );
    // Every installed histogram row carries one of the four palette colors.
    let rows = chart.data.series_data(ids[2]).unwrap().1[3].len();
    assert!(rows > 0);
    const PALETTE: [u32; 4] = [0x089981ff, 0x08998180, 0xf7525fff, 0xf7525f80];
    for r in 0..rows {
        let color = chart
            .data
            .point_color(
                ids[2],
                aeris_charts_core::model::data_layer::PointColorChannel::Body,
                r,
            )
            .expect("every histogram row is colored");
        assert!(
            PALETTE.contains(&color),
            "color {color:#x} is in the palette"
        );
    }
}

#[test]
fn generic_threshold_region_is_series_owned_and_validated() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    let line = chart.add_series(SeriesKind::Line);
    let times = [1.0, 2.0, 3.0];
    let values = [20.0, 50.0, 80.0];
    chart
        .set_series_data(line, &times, &values, &values, &values, &values)
        .unwrap();
    let region = SeriesThresholdRegion {
        lower: 30.0,
        upper: 70.0,
    };
    assert!(chart.set_series_threshold_region(line, Some(region)));
    assert_eq!(
        chart.series_entry(line).unwrap().threshold_region,
        Some(region)
    );
    chart.time_scale.set_width(800.0);
    chart.fit_content();
    let frame = chart.build_frame();
    let boundary_lines = frame.panes[0]
        .main
        .iter()
        .filter(|prim| {
            matches!(
                prim,
                aeris_charts_render::draw_list::Prim::HLine {
                    style: LineStyle::Dotted,
                    color,
                    ..
                } if *color == Color::rgb(0x78, 0x7B, 0x86)
            )
        })
        .count();
    assert_eq!(boundary_lines, 2);
    assert!(!chart.set_series_threshold_region(line, Some(region)));
    assert!(!chart.set_series_threshold_region(
        line,
        Some(SeriesThresholdRegion {
            lower: 70.0,
            upper: 30.0,
        })
    ));
    assert_eq!(
        chart.series_entry(line).unwrap().threshold_region,
        Some(region)
    );

    let histogram = chart.add_series(SeriesKind::Histogram);
    assert!(!chart.set_series_threshold_region(histogram, Some(region)));
    assert!(chart.set_series_threshold_region(line, None));
    assert_eq!(chart.series_entry(line).unwrap().threshold_region, None);
}

#[test]
fn generic_momentum_histogram_style_uses_the_canonical_four_state_palette() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    let histogram = chart.add_series(SeriesKind::Histogram);
    let times = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
    let values = [1.0, 2.0, 1.0, -1.0, -2.0, -1.0];
    chart
        .set_series_data(histogram, &times, &values, &values, &values, &values)
        .unwrap();
    assert!(chart.apply_momentum_histogram_colors(histogram));
    const PALETTE: [u32; 4] = [0x089981ff, 0x08998180, 0xf7525fff, 0xf7525f80];
    for row in 0..values.len() {
        let color = chart
            .data
            .point_color(
                histogram,
                aeris_charts_core::model::data_layer::PointColorChannel::Body,
                row,
            )
            .expect("every histogram row is colored");
        assert!(PALETTE.contains(&color));
    }

    let values = [f64::NAN, 1.0, 2.0, f64::NAN, -1.0, -2.0];
    chart
        .set_series_data(histogram, &times, &values, &values, &values, &values)
        .unwrap();
    assert!(chart.apply_momentum_histogram_colors(histogram));
    let color = |row| {
        chart.data.point_color(
            histogram,
            aeris_charts_core::model::data_layer::PointColorChannel::Body,
            row,
        )
    };
    assert_eq!(color(0), None);
    assert_eq!(color(1), Some(0x089981ff));
    assert_eq!(color(2), Some(0x089981ff));
    assert_eq!(color(3), None);
    assert_eq!(color(4), Some(0xf7525f80));
    assert_eq!(color(5), Some(0xf7525fff));
}

#[test]
fn vwap_stays_on_the_source_pane_and_weights_by_the_volume_series() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    chart
        .set_series_data(
            0,
            &[0.0, 3_600.0, 86_400.0],
            &[10.0, 20.0, 30.0],
            &[10.0, 20.0, 30.0],
            &[10.0, 20.0, 30.0],
            &[10.0, 20.0, 30.0],
        )
        .unwrap();
    let volume = chart.add_series(SeriesKind::Histogram);
    chart
        .set_series_data(
            volume,
            &[0.0, 3_600.0, 86_400.0],
            &[1.0, 3.0, 5.0],
            &[1.0, 3.0, 5.0],
            &[1.0, 3.0, 5.0],
            &[1.0, 3.0, 5.0],
        )
        .unwrap();
    let vwap = chart.add_vwap(0, Some(volume)).expect("valid vwap");
    assert_eq!(
        chart
            .series
            .iter()
            .find(|s| s.id == vwap)
            .unwrap()
            .pane_index,
        0
    );
    assert_eq!(chart.indicator_info(vwap).unwrap().kind, "vwap");
    let values = &chart.data.series_data(vwap).unwrap().1[3];
    // (10*1 + 20*3) / 4 = 17.5 on day 0; the new UTC day restarts at 30.
    assert!((values[1] - 17.5).abs() < 1e-9);
    assert!((values[2] - 30.0).abs() < 1e-9);
}

#[test]
fn wma_atr_and_stochastic_place_and_report() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    let values = [1.0, 2.0, 3.0, 4.0, 5.0];
    chart
        .set_series_data(
            0,
            &[1.0, 2.0, 3.0, 4.0, 5.0],
            &values,
            &values,
            &values,
            &values,
        )
        .unwrap();
    let wma = chart.add_wma(0, 3).expect("valid wma");
    assert_eq!(
        chart
            .series
            .iter()
            .find(|s| s.id == wma)
            .unwrap()
            .pane_index,
        0
    );
    assert_eq!(chart.indicator_info(wma).unwrap().kind, "wma");
    // (1*1 + 2*2 + 3*3) / 6 at index 2.
    let wma_values = &chart.data.series_data(wma).unwrap().1[3];
    assert!((wma_values[0] - 14.0 / 6.0).abs() < 1e-9);

    let dema = chart.add_dema(0, 3).expect("valid dema");
    assert_eq!(chart.indicator_info(dema).unwrap().kind, "dema");
    let dema_values = &chart.data.series_data(dema).unwrap().1[3];
    assert!((dema_values[0] - 5.0).abs() < 1e-9);
    let tema = chart.add_tema(0, 2).expect("valid tema");
    assert_eq!(chart.indicator_info(tema).unwrap().kind, "tema");
    let tema_values = &chart.data.series_data(tema).unwrap().1[3];
    assert!((tema_values[0] - 4.0).abs() < 1e-9);
    let smma = chart.add_smma(0, 3).expect("valid smma");
    assert_eq!(chart.indicator_info(smma).unwrap().kind, "smma");
    let smma_values = &chart.data.series_data(smma).unwrap().1[3];
    assert!((smma_values[0] - 2.0).abs() < 1e-9);
    let rma = chart.add_rma(0, 3).expect("valid rma alias");
    assert_eq!(chart.indicator_info(rma).unwrap().kind, "smma");
    let hma = chart.add_hma(0, 3).expect("valid hma");
    assert_eq!(chart.indicator_info(hma).unwrap().kind, "hma");
    let vwma = chart.add_vwma(0, None, 3).expect("valid vwma");
    assert_eq!(chart.indicator_info(vwma).unwrap().kind, "vwma");
    let standard_deviation = chart
        .add_standard_deviation(0, 3)
        .expect("valid standard deviation");
    assert_eq!(
        chart.indicator_info(standard_deviation).unwrap().kind,
        "standard_deviation"
    );
    let donchian = chart.add_donchian(0, 3);
    assert_eq!(donchian.len(), 3);
    assert_eq!(chart.indicator_info(donchian[0]).unwrap().kind, "donchian");

    let atr = chart.add_atr(0, 2).expect("valid atr");
    assert_ne!(
        chart
            .series
            .iter()
            .find(|s| s.id == atr)
            .unwrap()
            .pane_index,
        0
    );
    assert_eq!(chart.indicator_info(atr).unwrap().kind, "atr");

    let stoch = chart.add_stochastic(0, 2, 2);
    assert_eq!(stoch.len(), 2);
    let info = chart.indicator_info(stoch[0]).unwrap();
    assert_eq!(
        (info.kind, info.period, info.deviation),
        ("stochastic", 2, Some(2.0))
    );
    assert_eq!(chart.indicator_info(stoch[1]).unwrap().output_index, 1);
    // 20/80 oscillator channel on the %K output; boundary geometry is engine-owned rather than
    // exposed as mutable price-line state.
    assert_eq!(
        chart
            .series
            .iter()
            .find(|s| s.id == stoch[0])
            .unwrap()
            .threshold_region,
        Some(SeriesThresholdRegion {
            lower: 20.0,
            upper: 80.0,
        })
    );
}

#[test]
fn bollinger_creates_three_output_series() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    chart
        .set_series_data(
            0,
            &[1.0, 2.0, 3.0],
            &[1.0, 2.0, 3.0],
            &[1.0, 2.0, 3.0],
            &[1.0, 2.0, 3.0],
            &[1.0, 2.0, 3.0],
        )
        .unwrap();
    let ids = chart.add_bollinger(0, 3, 2.0);
    assert_eq!(ids.len(), 3);
    assert!(chart.data.series_data(ids[0]).unwrap().1[3][0] > 3.0);
    assert_eq!(chart.data.series_data(ids[1]).unwrap().1[3], &[2.0]);
}

#[test]
fn indicator_info_reports_lineage_and_output_slots() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    chart
        .set_series_data(
            0,
            &[1.0, 2.0, 3.0],
            &[1.0, 2.0, 3.0],
            &[1.0, 2.0, 3.0],
            &[1.0, 2.0, 3.0],
            &[1.0, 2.0, 3.0],
        )
        .unwrap();
    let sma = chart.add_sma(0, 2).expect("valid indicator");
    let info = chart.indicator_info(sma).expect("output carries lineage");
    assert_eq!(info.kind, "sma");
    assert_eq!(info.period, 2);
    assert_eq!(info.deviation, None);
    assert_eq!(info.source, 0);
    assert_eq!(info.output_index, 0);
    assert_eq!(info.binding_id, sma);
    assert_eq!(info.parameters.period, Some(2));
    assert_eq!(info.output_name, "SMA");
    assert_eq!(info.output_count, 1);
    assert_eq!(info.volume_source, None);

    let ids = chart.add_bollinger(0, 3, 2.5);
    let upper = chart.indicator_info(ids[0]).unwrap();
    assert_eq!(upper.kind, "bollinger");
    assert_eq!(upper.deviation, Some(2.5));
    assert_eq!(upper.parameters.deviation, Some(2.5));
    assert_eq!(upper.output_name, "Upper");
    assert_eq!(chart.indicator_info(ids[1]).unwrap().output_name, "Basis");
    assert_eq!(chart.indicator_info(ids[2]).unwrap().output_name, "Lower");
    assert!(ids
        .iter()
        .all(|&id| chart.indicator_info(id).unwrap().binding_id == ids[0]));
    assert_eq!(
        (0..3)
            .map(|i| chart.indicator_info(ids[i]).unwrap().output_index)
            .collect::<Vec<_>>(),
        [0, 1, 2]
    );

    // The source series itself and unknown ids are not indicator outputs.
    assert_eq!(chart.indicator_info(0), None);
    assert_eq!(chart.indicator_info(999), None);
}

#[test]
fn typed_indicator_bindings_preserve_duplicates_output_order_and_dependencies() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    let source = chart.add_series(SeriesKind::Candlestick);
    let volume = chart.add_series(SeriesKind::Histogram);
    let values = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
    chart
        .set_series_data(source, &values, &values, &values, &values, &values)
        .unwrap();
    chart
        .set_series_data(volume, &values, &values, &values, &values, &values)
        .unwrap();

    let first = chart.add_sma(source, 2).unwrap();
    let duplicate = chart.add_sma(source, 2).unwrap();
    let bands = chart.add_bollinger(first, 3, 2.5);
    let vwap = chart.add_vwap(source, Some(volume)).unwrap();

    let bindings = chart.indicator_bindings();
    assert_eq!(bindings.len(), 4);
    assert_eq!(bindings[0].kind, IndicatorKind::Sma { period: 2 });
    assert_eq!(bindings[0].binding_id, first);
    assert_eq!(bindings[0].outputs, [first]);
    assert_eq!(bindings[1].kind, IndicatorKind::Sma { period: 2 });
    assert_eq!(bindings[1].outputs, [duplicate]);
    assert_eq!(bindings[2].source, first);
    assert_eq!(bindings[2].outputs, bands);
    assert_eq!(bindings[3].volume_source, Some(volume));
    assert_eq!(bindings[3].outputs, [vwap]);

    let mut restored = ChartEngine::new(800.0, 500.0, 1.0);
    let _unrelated = restored.add_series(SeriesKind::Line);
    let restored_source = restored.add_series(SeriesKind::Candlestick);
    let restored_volume = restored.add_series(SeriesKind::Histogram);
    restored
        .set_series_data(restored_source, &values, &values, &values, &values, &values)
        .unwrap();
    restored
        .set_series_data(restored_volume, &values, &values, &values, &values, &values)
        .unwrap();
    let mut remapped = vec![(source, restored_source), (volume, restored_volume)];
    for binding in &bindings {
        let remap = |id| {
            remapped
                .iter()
                .find_map(|&(old, new)| (old == id).then_some(new))
                .expect("dependency was restored earlier")
        };
        let outputs = restored.add_indicator_kind(
            remap(binding.source),
            binding.kind.clone(),
            binding.volume_source.map(remap),
        );
        assert_eq!(outputs.len(), binding.outputs.len());
        remapped.extend(binding.outputs.iter().copied().zip(outputs));
    }

    let restored_bindings = restored.indicator_bindings();
    assert_eq!(restored_bindings.len(), bindings.len());
    for (old, new) in bindings.iter().zip(&restored_bindings) {
        assert_eq!(new.kind, old.kind);
        assert_eq!(
            new.source,
            remapped.iter().find(|pair| pair.0 == old.source).unwrap().1
        );
        assert_eq!(
            new.volume_source,
            old.volume_source
                .map(|id| remapped.iter().find(|pair| pair.0 == id).unwrap().1)
        );
        assert_eq!(new.outputs.len(), old.outputs.len());
        for (&old_output, &new_output) in old.outputs.iter().zip(&new.outputs) {
            assert_eq!(
                chart.data.series_data(old_output),
                restored.data.series_data(new_output)
            );
        }
    }
}

#[test]
fn generic_indicator_creation_rejects_invalid_definitions_atomically() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    let stale = chart.add_series(SeriesKind::Histogram);
    let candle_volume = chart.add_series(SeriesKind::Candlestick);
    let scalar_volume = chart.add_series(SeriesKind::Histogram);
    assert!(chart.remove_series(stale));
    let order = chart.series_order().to_vec();
    let pane_count = chart.panes.len();

    assert!(chart
        .add_indicator_kind(u32::MAX, IndicatorKind::Sma { period: 2 }, None)
        .is_empty());
    assert!(chart
        .add_indicator_kind(0, IndicatorKind::Sma { period: 0 }, None)
        .is_empty());
    assert!(chart
        .add_indicator_kind(
            0,
            IndicatorKind::Keltner {
                period: 14,
                multiplier: -1.0,
            },
            None,
        )
        .is_empty());
    assert!(chart
        .add_indicator_kind(0, IndicatorKind::AdxDmi { period: 0 }, None)
        .is_empty());
    assert!(chart
        .add_indicator_kind(stale, IndicatorKind::Sma { period: 2 }, None)
        .is_empty());
    assert!(chart
        .add_indicator_kind(0, IndicatorKind::Vwap, Some(u32::MAX))
        .is_empty());
    assert!(chart
        .add_indicator_kind(0, IndicatorKind::Vwap, Some(stale))
        .is_empty());
    assert!(chart
        .add_indicator_kind(0, IndicatorKind::Vwap, Some(candle_volume))
        .is_empty());
    assert!(chart
        .add_indicator_kind(0, IndicatorKind::Vwap, Some(0))
        .is_empty());
    assert!(chart
        .add_indicator_kind(
            0,
            IndicatorKind::VwapBands {
                reset: VwapReset::Session,
                standard_deviation: f64::NAN,
                percent: 10.0,
            },
            None,
        )
        .is_empty());
    assert!(chart
        .add_indicator_kind(
            0,
            IndicatorKind::Rsi {
                period: 2,
                seed: IndicatorSeed::Sma,
            },
            Some(0)
        )
        .is_empty());

    assert_eq!(chart.series_order(), order);
    assert_eq!(chart.panes.len(), pane_count);
    assert!(chart.indicator_bindings().is_empty());
    assert_eq!(
        chart
            .add_indicator_kind(0, IndicatorKind::Vwap, Some(scalar_volume))
            .len(),
        1
    );
}

#[test]
fn typed_indicator_inputs_select_ohlc_aggregates_and_rebind_incrementally() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    let source = chart.add_series(SeriesKind::Candlestick);
    let times = [0.0, 60.0, 120.0, 180.0, 240.0, 300.0];
    let open = [10.0, 11.0, 12.0, 13.0, 14.0, 15.0];
    let high = [12.0, 14.0, 16.0, 18.0, 20.0, 22.0];
    let low = [8.0, 9.0, 10.0, 11.0, 12.0, 13.0];
    let close = [11.0, 13.0, 15.0, 17.0, 19.0, 21.0];
    chart
        .set_series_data(source, &times, &open, &high, &low, &close)
        .unwrap();

    let rsi = chart
        .add_indicator_kind_with_input(
            source,
            IndicatorInputSource::Hlc3,
            IndicatorKind::Rsi {
                period: 2,
                seed: IndicatorSeed::Sma,
            },
            None,
        )
        .into_iter()
        .next()
        .unwrap();
    let expected_input = high
        .iter()
        .zip(low.iter())
        .zip(close.iter())
        .map(|((&high, &low), &close)| (high + low + close) / 3.0)
        .collect::<Vec<_>>();
    let expected = aeris_charts_indicators::rsi(&expected_input, 2);
    let actual = &chart.data.series_data(rsi).unwrap().1[3];
    for (actual, expected) in actual.iter().zip(expected.into_iter().skip(2)) {
        match expected {
            Some(expected) => assert!((*actual - expected).abs() < 1e-12),
            None => panic!("warm-up row was not expected in the aligned output: {actual}"),
        }
    }
    assert_eq!(
        chart.indicator_info(rsi).unwrap().source_input,
        IndicatorInputSource::Hlc3
    );

    let sma = chart.add_sma(rsi, 2).unwrap();
    assert_eq!(
        chart.indicator_info(sma).unwrap().source_input,
        IndicatorInputSource::Close
    );
    assert!(chart.set_indicator_input_source(rsi, IndicatorInputSource::Open));
    assert_eq!(
        chart.indicator_info(rsi).unwrap().source_input,
        IndicatorInputSource::Open
    );
    assert!(chart
        .indicator_bindings()
        .iter()
        .any(|binding| { binding.outputs.contains(&sma) && binding.source == rsi }));
    assert!(!chart.set_indicator_input_source(u32::MAX, IndicatorInputSource::Close));
}

#[test]
fn pivot_points_align_previous_session_levels_and_expose_all_outputs() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    let times = [0.0, 3_600.0, 86_400.0, 90_000.0];
    let open = [10.0, 11.0, 12.0, 13.0];
    let high = [12.0, 14.0, 15.0, 16.0];
    let low = [8.0, 9.0, 10.0, 11.0];
    let close = [11.0, 13.0, 14.0, 15.0];
    chart
        .set_series_data(0, &times, &open, &high, &low, &close)
        .unwrap();
    let outputs = chart.add_pivot_points(0, aeris_charts_indicators::PivotKind::Standard);
    assert_eq!(outputs.len(), 5);
    let expected = aeris_charts_indicators::pivot_points(
        &[0, 3_600, 86_400, 90_000],
        &open,
        &high,
        &low,
        &close,
        aeris_charts_indicators::PivotKind::Standard,
    );
    for (output_index, output) in outputs.iter().enumerate() {
        let (output_times, values) = chart.data.series_data(*output).unwrap();
        assert_eq!(output_times, &[0, 3_600, 86_400, 90_000]);
        let expected = [
            expected[2].pivot,
            expected[2].resistance_1,
            expected[2].support_1,
            expected[2].resistance_2,
            expected[2].support_2,
        ][output_index]
            .unwrap();
        assert!(values[3][0].is_nan(), "output {output_index}");
        let actual = values[3][2];
        assert!((actual - expected).abs() < 1e-12, "output {output_index}");
        assert_eq!(values[3][3], values[3][2]);
    }
}

#[test]
fn indicator_schema_exposes_typed_parameters_and_outputs() {
    let schema = ChartEngine::indicator_schema(&IndicatorKind::Bollinger {
        period: 20,
        deviation: 2.0,
        estimator: DeviationEstimator::Population,
    });
    assert_eq!(schema.revision, INDICATOR_SCHEMA_REVISION);
    assert_eq!(schema.kind, "bollinger");
    assert_eq!(
        schema.parameters[0].parameter_type,
        IndicatorParameterType::Source
    );
    assert_eq!(schema.parameters[1].name, "period");
    assert_eq!(schema.parameters[2].name, "deviation");
    assert_eq!(schema.outputs.len(), 3);
    assert_eq!(schema.outputs[0].name, "Upper");
    assert!(schema.outputs.iter().all(|output| output.supports_style));
}

#[test]
fn indicator_output_styles_are_queryable_and_rebound_without_losing_identity() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    let values = [1.0, 2.0, 3.0, 4.0, 5.0];
    chart
        .set_series_data(
            0,
            &[1.0, 2.0, 3.0, 4.0, 5.0],
            &values,
            &values,
            &values,
            &values,
        )
        .unwrap();
    let outputs = chart.add_bollinger(0, 2, 2.0);
    let style = IndicatorOutputStyle {
        visible: false,
        line_color: Some("#123456".into()),
        line_width: Some(3.5),
        line_style: 2,
        point_markers: true,
        up_color: Some("#00ff00".into()),
        down_color: Some("#ff0000".into()),
        area_top_color: Some("rgba(1, 2, 3, 0.4)".into()),
        area_bottom_color: Some("rgba(4, 5, 6, 0.2)".into()),
    };
    assert!(chart.set_indicator_output_style(outputs[0], style.clone()));
    let binding = chart
        .indicator_bindings()
        .into_iter()
        .find(|binding| binding.outputs == outputs)
        .unwrap();
    assert_eq!(binding.styles[0], style);
    assert_eq!(
        chart.indicator_info(outputs[0]).unwrap().binding_id,
        outputs[0]
    );
    assert!(!chart.set_indicator_output_style(
        outputs[0],
        IndicatorOutputStyle {
            line_width: Some(0.0),
            ..style
        }
    ));
}

#[test]
fn indicator_snapshot_values_and_complete_multi_output_metadata_stay_ordered() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    let values = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
    chart
        .set_series_data(
            0,
            &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0],
            &values,
            &values,
            &values,
            &values,
        )
        .unwrap();
    let macd = chart.add_macd(0, 2, 3, 2);
    let names = macd
        .iter()
        .map(|&id| chart.indicator_info(id).unwrap().output_name)
        .collect::<Vec<_>>();
    assert_eq!(names, ["MACD", "Signal", "Histogram"]);
    for (index, &id) in macd.iter().enumerate() {
        let info = chart.indicator_info(id).unwrap();
        assert_eq!(info.binding_id, macd[0]);
        assert_eq!(info.output_index, index);
        assert_eq!(info.output_count, 3);
        assert_eq!(info.parameters.fast, Some(2));
        assert_eq!(info.parameters.slow, Some(3));
        assert_eq!(info.parameters.signal, Some(2));
    }
    let snapshot = chart.value_snapshot(None);
    for &id in &macd {
        let output = snapshot.iter().find(|value| value.series_id == id).unwrap();
        assert!(output.value.is_some());
        assert!(output.formatted_value.is_some());
    }
    let previous_macd = snapshot
        .iter()
        .find(|value| value.series_id == macd[0])
        .and_then(|value| value.value)
        .unwrap();
    assert!(chart.update_series_bar(0, 6.0, [12.0; 4]));
    let updated_macd = chart
        .value_snapshot(None)
        .into_iter()
        .find(|value| value.series_id == macd[0])
        .and_then(|value| value.value)
        .unwrap();
    assert_ne!(updated_macd, previous_macd);

    let volume = chart.add_series(SeriesKind::Histogram);
    chart
        .set_series_data(
            0,
            &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0],
            &values,
            &values,
            &values,
            &values,
        )
        .unwrap();
    chart
        .set_series_data(
            volume,
            &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0],
            &values,
            &values,
            &values,
            &values,
        )
        .unwrap();
    let vwap = chart.add_vwap(0, Some(volume)).unwrap();
    assert_eq!(
        chart.indicator_info(vwap).unwrap().volume_source,
        Some(volume)
    );
}

#[test]
fn remove_series_tracked_reports_the_series_and_its_derived_outputs() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    chart
        .set_series_data(
            0,
            &[1.0, 2.0, 3.0],
            &[1.0, 2.0, 3.0],
            &[1.0, 2.0, 3.0],
            &[1.0, 2.0, 3.0],
            &[1.0, 2.0, 3.0],
        )
        .unwrap();
    let sma = chart.add_sma(0, 2).expect("valid indicator");
    let bands = chart.add_bollinger(0, 3, 2.0);

    let mut dropped = chart.remove_series_tracked(0);
    dropped.sort_unstable();
    let mut expected = vec![0, sma];
    expected.extend(bands);
    expected.sort_unstable();
    assert_eq!(dropped, expected);
    // Everything tombstoned: a second attempt (or an unknown id) reports nothing.
    assert!(chart.remove_series_tracked(0).is_empty());
    assert!(chart.remove_series_tracked(42).is_empty());
}

#[test]
fn retained_frame_reuses_pane_buffers() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    chart
        .set_series_data(
            0,
            &[1.0, 2.0, 3.0],
            &[1.0, 2.0, 3.0],
            &[2.0, 3.0, 4.0],
            &[0.0, 1.0, 2.0],
            &[1.5, 2.5, 3.5],
        )
        .unwrap();
    chart.time_scale.set_width(800.0);
    chart.fit_content();
    let mut frame = ChartFrame::default();
    chart.build_frame_into(&mut frame);
    let first_capacity = frame.panes[0].main.capacity();
    chart.crosshair = Some((300.0, 100.0));
    chart.build_frame_into(&mut frame);
    assert!(frame.panes[0].main.capacity() >= first_capacity);
}

#[test]
fn frame_pane_top_layer_is_retained_per_frame() {
    // The `top` layer is host-appended (pane primitives, reference zOrder "top") after frame
    // construction; the engine owns clearing it between retained-frame rebuilds exactly like
    // `under`/`main`, so a stale top prim can never survive into the next frame.
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    chart
        .set_series_data(
            0,
            &[1.0, 2.0, 3.0],
            &[1.0, 2.0, 3.0],
            &[2.0, 3.0, 4.0],
            &[0.0, 1.0, 2.0],
            &[1.5, 2.5, 3.5],
        )
        .unwrap();
    chart.time_scale.set_width(800.0);
    chart.fit_content();
    let mut frame = ChartFrame::default();
    chart.build_frame_into(&mut frame);
    assert!(frame.panes[0].top_prims.is_empty());
    // Simulate a host appending a top-layer prim: it survives in the frame output until the
    // next rebuild, which must reset the layer.
    frame.panes[0]
        .top_prims
        .push(aeris_charts_render::draw_list::Prim::Rect {
            rect: aeris_charts_render::draw_list::IRect {
                x: 0,
                y: 0,
                w: 4,
                h: 4,
            },
            color: aeris_charts_render::color::Color::rgb(0xff, 0x00, 0x00),
        });
    assert_eq!(frame.panes[0].top_prims.len(), 1);
    chart.build_frame_into(&mut frame);
    assert!(frame.panes[0].top_prims.is_empty());
    assert!(!frame.panes[0].main.is_empty());
}

#[test]
fn axis_frame_owns_label_content_and_positions() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    chart
        .set_series_data(
            0,
            &[1.0, 2.0],
            &[10.0, 11.0],
            &[11.0, 12.0],
            &[9.0, 10.0],
            &[10.0, 11.0],
        )
        .unwrap();
    chart.time_scale.set_width(760.0);
    chart.fit_content();
    let axes = chart.build_axis_frame(
        80.0,
        |text, _bold| text.len() as f64,
        |text, _bold| text.len() as f64,
    );
    assert!(!axes.labels.is_empty());
    assert!(axes.labels.iter().any(|label| label.text.contains("11")));
}

#[test]
fn grid_line_style_and_color_flow_from_options() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    chart
        .set_series_data(
            0,
            &[1.0, 2.0, 3.0],
            &[10.0, 11.0, 12.0],
            &[9.0, 10.0, 11.0],
            &[8.0, 9.0, 10.0],
            &[9.5, 10.5, 11.5],
        )
        .unwrap();
    chart.time_scale.set_width(760.0);
    chart.fit_content();

    // Canonical default: both grid families retain dashed styling but ship disabled.
    use aeris_charts_render::draw_list::Prim;
    let mut frame = ChartFrame::default();
    chart.build_frame_into(&mut frame);
    let grid_lines: Vec<_> = frame.panes[0]
        .under
        .iter()
        .filter(|p| matches!(p, Prim::VLine { .. } | Prim::HLine { .. }))
        .collect();
    assert!(grid_lines.is_empty());

    // Explicit visibility, numeric styles (2 dashed, 3 large-dashed), and per-family colors reach
    // the frame without any renderer-specific policy.
    chart
        .options
        .apply_str(
            r##"{"grid": {
                "vertLines": { "visible": true, "style": 2, "color": "#112233" },
                "horzLines": { "visible": true, "style": 3, "color": "#445566" }
            }}"##,
        )
        .unwrap();
    chart.build_frame_into(&mut frame);
    let under = &frame.panes[0].under;
    assert!(under.iter().any(|p| matches!(
        p,
        Prim::VLine { style: LineStyle::Dashed, color, .. } if *color == Color::rgb(0x11, 0x22, 0x33)
    )));
    assert!(under.iter().any(|p| matches!(
        p,
        Prim::HLine { style: LineStyle::Dashed, color, .. } if *color == Color::rgb(0x44, 0x55, 0x66)
    )));
}

#[test]
fn crosshair_line_style_and_width_flow_from_options() {
    use aeris_charts_render::draw_list::Prim;
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    chart.series[0].kind = SeriesKind::Line;
    chart
        .set_series_data(
            0,
            &[1.0, 2.0, 3.0],
            &[10.0, 11.0, 12.0],
            &[11.0, 12.0, 13.0],
            &[9.0, 10.0, 11.0],
            &[10.5, 11.5, 12.5],
        )
        .unwrap();
    chart.time_scale.set_width(800.0);
    chart.fit_content();
    chart.crosshair = Some((200.0, 120.0));

    // Default: Aeris's dashed crosshair at the crisp 1px width.
    let mut frame = ChartFrame::default();
    chart.build_frame_into(&mut frame);
    assert!(frame.panes[0].main.iter().any(|p| matches!(
        p,
        Prim::VLine {
            style: LineStyle::Dashed,
            width: 1,
            ..
        }
    )));
    assert!(frame.panes[0].main.iter().any(|p| matches!(
        p,
        Prim::HLine {
            style: LineStyle::Dashed,
            width: 1,
            ..
        }
    )));

    // Per-line reference numeric style (1 dotted / 2 dashed) and lineWidth reach the frame.
    chart
        .options
        .apply_str(
            r##"{"crosshair": {
                "vertLine": { "style": 1, "width": 3 },
                "horzLine": { "style": 2, "width": 2 }
            }}"##,
        )
        .unwrap();
    chart.build_frame_into(&mut frame);
    assert!(frame.panes[0].main.iter().any(|p| matches!(
        p,
        Prim::VLine {
            style: LineStyle::Dotted,
            width: 3,
            ..
        }
    )));
    assert!(frame.panes[0].main.iter().any(|p| matches!(
        p,
        Prim::HLine {
            style: LineStyle::Dashed,
            width: 2,
            ..
        }
    )));
}

#[test]
fn crosshair_label_visibility_and_background_flow_from_options() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    chart.series[0].kind = SeriesKind::Line;
    chart
        .set_series_data(
            0,
            &[1.0, 2.0, 3.0],
            &[10.0, 11.0, 12.0],
            &[11.0, 12.0, 13.0],
            &[9.0, 10.0, 11.0],
            &[10.5, 11.5, 12.5],
        )
        .unwrap();
    chart.time_scale.set_width(800.0);
    chart.fit_content();
    chart.crosshair = Some((200.0, 120.0));

    let axis = chart.build_axis_frame(
        80.0,
        |text, _bold| text.len() as f64 * 7.0,
        |text, _bold| text.len() as f64 * 6.0,
    );
    let label_background = aeris_charts_core::style::DEFAULT_CROSSHAIR_LABEL_RGB;
    let label_background = Color::rgb(label_background.0, label_background.1, label_background.2);
    let foreground = label_background.contrast_text();
    let time_label = axis
        .labels
        .iter()
        .find(|label| label.midpoint == AxisTextMidpoint::StableTime)
        .expect("default crosshair time label");
    assert_eq!(time_label.color, foreground);
    assert!(matches!(time_label.background, Some((.., color)) if color == label_background));
    assert!(axis.labels.iter().any(|label| {
        label.midpoint == AxisTextMidpoint::Label
            && label.color == foreground
            && matches!(label.background, Some((.., color)) if color == label_background)
    }));

    // Distinctive per-line label backgrounds prove each honors `labelBackgroundColor`. The price
    // label follows the horizontal line; the time label is the unique `StableTime` midpoint label.
    chart
        .options
        .apply_str(
            r##"{"crosshair": {
                "horzLine": { "labelBackgroundColor": "#010203" },
                "vertLine": { "labelBackgroundColor": "#040506" }
            }}"##,
        )
        .unwrap();
    let axis = chart.build_axis_frame(
        80.0,
        |text, _bold| text.len() as f64 * 7.0,
        |text, _bold| text.len() as f64 * 6.0,
    );
    let price_bg = Color::rgb(0x01, 0x02, 0x03);
    let time_bg = Color::rgb(0x04, 0x05, 0x06);
    assert!(axis
        .labels
        .iter()
        .any(|l| matches!(l.background, Some((.., c)) if c == price_bg)));
    assert!(axis
        .labels
        .iter()
        .any(|l| l.midpoint == AxisTextMidpoint::StableTime
            && matches!(l.background, Some((.., c)) if c == time_bg)));

    // `labelVisible: false` suppresses each label independently.
    chart
        .options
        .apply_str(
            r##"{"crosshair": {
                "horzLine": { "labelVisible": false },
                "vertLine": { "labelVisible": false }
            }}"##,
        )
        .unwrap();
    let axis = chart.build_axis_frame(
        80.0,
        |text, _bold| text.len() as f64 * 7.0,
        |text, _bold| text.len() as f64 * 6.0,
    );
    assert!(!axis
        .labels
        .iter()
        .any(|l| matches!(l.background, Some((.., c)) if c == price_bg)));
    assert!(!axis
        .labels
        .iter()
        .any(|l| l.midpoint == AxisTextMidpoint::StableTime));
}

#[test]
fn layout_font_size_scales_axis_label_box_heights() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    chart
        .set_series_data(
            0,
            &[1.0, 2.0],
            &[10.0, 11.0],
            &[11.0, 12.0],
            &[9.0, 10.0],
            &[10.0, 11.0],
        )
        .unwrap();
    chart.time_scale.set_width(760.0);
    chart.fit_content();

    // The last-value badge is the only boxed label here; its box height is fontSize + padding.
    let tallest_box = |chart: &AxisFrame| {
        chart
            .labels
            .iter()
            .filter_map(|l| l.background.map(|b| b.3))
            .fold(0.0_f64, f64::max)
    };
    let axis = chart.build_axis_frame(80.0, |t, _bold| t.len() as f64, |t, _bold| t.len() as f64);
    // Price tags resolve at 11/12 of layout.fontSize plus 2px padding each side.
    assert_eq!(tallest_box(&axis), 12.0 * 11.0 / 12.0 + 2.0 * 2.0);

    chart
        .options
        .apply_str(r#"{"layout": {"fontSize": 20}}"#)
        .unwrap();
    let axis = chart.build_axis_frame(80.0, |t, _bold| t.len() as f64, |t, _bold| t.len() as f64);
    assert_eq!(tallest_box(&axis), 20.0 * 11.0 / 12.0 + 2.0 * 2.0);
}

#[test]
fn series_color_alpha_survives_into_line_and_histogram_strokes() {
    use aeris_charts_render::draw_list::Prim;
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    chart.series[0].kind = SeriesKind::Line;
    chart
        .set_series_data(
            0,
            &[1.0, 2.0, 3.0],
            &[10.0, 11.0, 12.0],
            &[10.0, 11.0, 12.0],
            &[10.0, 11.0, 12.0],
            &[10.0, 11.0, 12.0],
        )
        .unwrap();
    let histogram = chart.add_series(SeriesKind::Histogram);
    chart
        .set_series_data(
            histogram,
            &[1.0, 2.0, 3.0],
            &[1.0, 2.0, 3.0],
            &[1.0, 2.0, 3.0],
            &[1.0, 2.0, 3.0],
            &[1.0, 2.0, 3.0],
        )
        .unwrap();
    // The TS boundary passes the full CSS color; the engine keeps its alpha channel.
    let translucent = Color::parse_css("rgba(10, 20, 30, 0.5)").unwrap();
    assert_eq!(translucent.a(), 128);
    chart.series[0].line_color = Some(translucent.to_css());
    chart.series_entry_mut(histogram).unwrap().line_color = Some(translucent.to_css());
    chart.time_scale.set_width(800.0);
    chart.fit_content();

    let frame = chart.build_frame();
    assert!(frame.panes[0]
        .main
        .iter()
        .any(|p| matches!(p, Prim::Polyline { color, .. } if *color == translucent)));
    assert!(frame.panes[0]
        .main
        .iter()
        .any(|p| matches!(p, Prim::Rect { color, .. } if *color == translucent)));

    // And the options getter round-trips the alpha channel back through CSS.
    let options: serde_json::Value =
        serde_json::from_str(&chart.series_options_json(0).unwrap()).unwrap();
    assert_eq!(
        Color::parse_css(options["color"].as_str().unwrap()),
        Some(translucent)
    );
}

#[test]
fn price_line_extras_drive_line_and_axis_label_rendering() {
    use aeris_charts_render::draw_list::Prim;
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    chart
        .set_series_data(
            0,
            &[1.0, 2.0],
            &[10.0, 11.0],
            &[11.0, 12.0],
            &[9.0, 10.0],
            &[10.0, 11.0],
        )
        .unwrap();
    chart.time_scale.set_width(800.0);
    chart.fit_content();
    let line_color = Color::rgb(0x12, 0x34, 0x56);
    let id = chart.create_price_line(0, 10.5, line_color, 2, LineStyle::Solid, "target");
    let has_line = |chart: &mut ChartEngine| {
        chart.build_frame().panes[0]
            .main
            .iter()
            .any(|p| matches!(p, Prim::HLine { color, .. } if *color == line_color))
    };
    let find_label = |chart: &mut ChartEngine| {
        chart
            .build_axis_frame(
                80.0,
                |t, _bold| t.len() as f64 * 7.0,
                |t, _bold| t.len() as f64 * 6.0,
            )
            .labels
            .into_iter()
            .find(|l| l.text == "target")
    };

    // Defaults: line drawn, boxed label in the line color with contrasting text.
    assert!(has_line(&mut chart));
    let label = find_label(&mut chart).expect("price-line label");
    assert!(matches!(label.background, Some((.., c)) if c == line_color));
    assert_eq!(label.color, line_color.contrast_text());

    // `lineVisible: false` skips only the HLine; the axis label stays.
    assert!(chart.price_line_apply_options(id, r#"{"line_visible":false}"#));
    assert!(!has_line(&mut chart));
    assert!(find_label(&mut chart).is_some());

    // `axisLabelVisible: false` drops only the label; the line comes back on its own.
    assert!(
        chart.price_line_apply_options(id, r#"{"line_visible":true,"axis_label_visible":false}"#)
    );
    assert!(has_line(&mut chart));
    assert!(find_label(&mut chart).is_none());

    // A custom light background without a text override automatically selects black.
    assert!(chart.price_line_apply_options(
        id,
        r##"{"axis_label_visible":true,"axis_label_color":"#f0e68c"}"##
    ));
    let label = find_label(&mut chart).expect("price-line label");
    assert!(matches!(label.background, Some((.., c)) if c == Color::rgb(0xf0, 0xe6, 0x8c)));
    assert_eq!(label.color, Color::rgb(0, 0, 0));

    // Translucent label colors contrast against the actual theme surface.
    assert!(
        chart.price_line_apply_options(id, r##"{"axis_label_color":"rgba(255,255,255,0.25)"}"##)
    );
    chart
        .apply_options(r##"{"layout":{"background":{"type":"solid","color":"#000000"}}}"##)
        .unwrap();
    assert_eq!(
        find_label(&mut chart).expect("price-line label").color,
        Color::rgb(255, 255, 255)
    );
    chart
        .apply_options(r##"{"layout":{"background":{"type":"solid","color":"#ffffff"}}}"##)
        .unwrap();
    assert_eq!(
        find_label(&mut chart).expect("price-line label").color,
        Color::rgb(0, 0, 0)
    );

    // An explicit text color remains independent of the background and line color.
    assert!(chart.price_line_apply_options(
        id,
        r##"{"axis_label_visible":true,"axis_label_color":"#010203","axis_label_text_color":"#aabbcc"}"##
    ));
    let label = find_label(&mut chart).expect("price-line label");
    assert!(matches!(label.background, Some((.., c)) if c == Color::rgb(0x01, 0x02, 0x03)));
    assert_eq!(label.color, Color::rgb(0xaa, 0xbb, 0xcc));
}

#[test]
fn price_line_options_merge_and_serialize_round_trip() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    let id = chart.create_price_line(
        0,
        42.0,
        Color::rgb(0x21, 0x96, 0xf3),
        1,
        LineStyle::Solid,
        "",
    );

    // A partial patch merges: untouched keys keep their values (reference merge semantics).
    assert!(chart.price_line_apply_options(
        id,
        r#"{"price":43.5,"line_style":"large_dashed","line_width":3,"title":"T","line_visible":false}"#
    ));
    let options: serde_json::Value =
        serde_json::from_str(&chart.price_line_options_json(id).unwrap()).unwrap();
    assert_eq!(options["price"], 43.5);
    // The retired `large_dashed` name folds into its renamed equivalent: it serializes `dashed`.
    assert_eq!(options["line_style"], "dashed");
    assert_eq!(options["line_width"], 3);
    assert_eq!(options["title"], "T");
    assert_eq!(options["line_visible"], false);
    // Untouched defaults survive the merge.
    assert_eq!(options["axis_label_visible"], true);
    assert_eq!(options["color"], "#2196f3");
    assert_eq!(options["axis_label_color"], "");
    assert_eq!(options["axis_label_text_color"], "");

    // CamelCase aliases are accepted, and `""` clears a pinned label color back to
    // following the line color.
    assert!(chart.price_line_apply_options(id, r##"{"axisLabelColor":"#ff0000"}"##));
    let options: serde_json::Value =
        serde_json::from_str(&chart.price_line_options_json(id).unwrap()).unwrap();
    assert_eq!(options["axis_label_color"], "#ff0000");
    assert!(chart.price_line_apply_options(id, r#"{"axis_label_color":""}"#));
    let options: serde_json::Value =
        serde_json::from_str(&chart.price_line_options_json(id).unwrap()).unwrap();
    assert_eq!(options["axis_label_color"], "");

    // Unknown ids and malformed JSON are rejected without touching state.
    assert!(!chart.price_line_apply_options(999, "{}"));
    assert!(!chart.price_line_apply_options(id, "{ nope"));
    assert!(chart.price_line_options_json(999).is_none());
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&chart.price_line_options_json(id).unwrap())
            .unwrap()["price"],
        43.5
    );
}

#[test]
fn chart_json_routes_timescale_behavioral_options_patch_driven_only() {
    let mut chart = ChartEngine::new(300.0, 200.0, 1.0);
    chart
        .set_series_data(
            0,
            &[10.0, 20.0, 30.0],
            &[1.0, 2.0, 3.0],
            &[1.0, 2.0, 3.0],
            &[1.0, 2.0, 3.0],
            &[1.0, 2.0, 3.0],
        )
        .unwrap();
    chart.time_scale.set_width(300.0);
    chart.fit_content();

    chart
        .apply_options(
            r#"{"timeScale":{"barSpacing":12,"fixLeftEdge":true,"timeVisible":false,"secondsVisible":true}}"#,
        )
        .unwrap();
    assert_eq!(chart.bar_spacing(), 12.0);
    assert!(chart.time_scale.options().fix_left_edge);
    assert!(!chart.time_visible);
    assert!(chart.seconds_visible);
    // The store still deep-merges the patch for round-tripping.
    assert_eq!(
        chart.options.value()["timeScale"]["barSpacing"],
        serde_json::json!(12)
    );

    // An unrelated patch must NOT re-apply the merged store over live scale state.
    chart.set_bar_spacing(20.0);
    chart
        .apply_options(r##"{"grid":{"vertLines":{"color":"#000000"}}}"##)
        .unwrap();
    assert_eq!(chart.bar_spacing(), 20.0);

    // A patch carrying only border cosmetics leaves behavior alone too.
    chart
        .apply_options(r##"{"timeScale":{"borderColor":"#123456"}}"##)
        .unwrap();
    assert_eq!(chart.bar_spacing(), 20.0);

    // Malformed patches error out without touching state.
    assert!(chart.apply_options("{ nope").is_err());
    assert_eq!(chart.bar_spacing(), 20.0);
}

#[test]
fn max_bar_spacing_and_right_offset_pixels_setters_follow_reference() {
    let mut chart = ChartEngine::new(300.0, 200.0, 1.0);
    chart
        .set_series_data(
            0,
            &[10.0, 20.0, 30.0],
            &[1.0, 2.0, 3.0],
            &[1.0, 2.0, 3.0],
            &[1.0, 2.0, 3.0],
            &[1.0, 2.0, 3.0],
        )
        .unwrap();
    chart.time_scale.set_width(300.0);
    chart.fit_content();

    // maxBarSpacing caps zoom-in; 0 restores the default half-width cap; invalid is ignored.
    chart.set_max_bar_spacing(10.0);
    chart.set_bar_spacing(50.0);
    assert_eq!(chart.bar_spacing(), 10.0);
    chart.set_max_bar_spacing(0.0);
    chart.set_bar_spacing(10_000.0);
    assert_eq!(chart.bar_spacing(), 150.0);
    chart.set_max_bar_spacing(f64::NAN);
    chart.set_bar_spacing(10_000.0);
    assert_eq!(chart.bar_spacing(), 150.0);

    // rightOffsetPixels converts to bars through the current spacing; invalid is ignored.
    chart.set_bar_spacing(6.0);
    chart.set_right_offset_pixels(60.0);
    assert_eq!(chart.right_offset(), 10.0);
    chart.set_right_offset_pixels(f64::INFINITY);
    assert_eq!(chart.right_offset(), 10.0);
}

#[test]
fn series_options_json_covers_the_ts_field_set() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);

    // Defaults on the primary candle series.
    let options: serde_json::Value =
        serde_json::from_str(&chart.series_options_json(0).unwrap()).unwrap();
    for key in [
        "color",
        "up_color",
        "down_color",
        "wick_up_color",
        "wick_down_color",
        "border_up_color",
        "border_down_color",
        "wick_visible",
        "border_visible",
        "line_width",
        "line_type",
        "area_top_color",
        "area_bottom_color",
        "histogram_updown",
        "baseline_value",
        "point_markers",
        "last_price_animation",
        "visible",
        "price_scale_id",
        "pane",
        "title",
        "title_visible",
        "countdown_visible",
        "heikin_ashi",
    ] {
        assert!(options.get(key).is_some(), "missing key {key}");
    }
    assert_eq!(options["color"], "#2196f3");
    // Unset optional colors are the follow-body/default state: "".
    assert_eq!(options["up_color"], "");
    assert_eq!(options["down_color"], "");
    assert_eq!(options["wick_up_color"], "");
    assert_eq!(options["border_down_color"], "");
    assert_eq!(options["area_top_color"], "");
    assert_eq!(options["area_bottom_color"], "");
    assert_eq!(options["wick_visible"], true);
    assert_eq!(options["border_visible"], true);
    assert_eq!(options["line_width"], 2.0);
    assert_eq!(options["line_type"], "simple");
    assert_eq!(options["histogram_updown"], false);
    assert_eq!(options["baseline_value"], serde_json::Value::Null);
    assert_eq!(options["point_markers"], false);
    assert_eq!(options["last_price_animation"], false);
    assert_eq!(options["visible"], true);
    assert_eq!(options["price_scale_id"], "right");
    assert_eq!(options["pane"], 0);
    // industry-standard last-value cluster options: reference-informed defaults.
    assert_eq!(options["title"], "");
    assert_eq!(options["title_visible"], true);
    assert_eq!(options["countdown_visible"], true);

    // Set state round-trips with colors and flags intact.
    chart.series[0].up_color = Some("#089981".to_string());
    chart.series[0].wick_up_color = Some(Color::rgba(1, 2, 3, 0x80).to_css());
    chart.series[0].border_visible = Some(false);
    chart.series[0].line_width = Some(5.0);
    chart.series[0].line_type = LineType::WithSteps;
    chart.series[0].baseline = Some(9.5);
    chart.series[0].point_markers = true;
    chart.set_series_visible(0, false);
    let options: serde_json::Value =
        serde_json::from_str(&chart.series_options_json(0).unwrap()).unwrap();
    assert_eq!(options["up_color"], "#089981");
    assert_eq!(
        Color::parse_css(options["wick_up_color"].as_str().unwrap()),
        Some(Color::rgba(1, 2, 3, 0x80))
    );
    assert_eq!(options["border_visible"], false);
    assert_eq!(options["line_width"], 5.0);
    assert_eq!(options["line_type"], "stepped");
    assert_eq!(options["baseline_value"], 9.5);
    assert_eq!(options["point_markers"], true);
    assert_eq!(options["visible"], false);

    // Scale targeting maps to the reference priceScaleId values; removed series report nothing.
    let overlay = chart.add_series(SeriesKind::Histogram);
    chart.series_entry_mut(overlay).unwrap().price_scale_target = PriceScaleTarget::Overlay;
    let left = chart.add_series(SeriesKind::Line);
    chart.series_entry_mut(left).unwrap().price_scale_target = PriceScaleTarget::Left;
    let options: serde_json::Value =
        serde_json::from_str(&chart.series_options_json(overlay).unwrap()).unwrap();
    assert_eq!(options["price_scale_id"], "");
    let options: serde_json::Value =
        serde_json::from_str(&chart.series_options_json(left).unwrap()).unwrap();
    assert_eq!(options["price_scale_id"], "left");
    assert!(chart.remove_series(left));
    assert!(chart.series_options_json(left).is_none());
}

#[test]
fn time_scale_options_json_covers_all_fields() {
    let mut chart = ChartEngine::new(300.0, 200.0, 1.0);
    chart
        .apply_options(
            r#"{"timeScale":{"minBarSpacing":2,"fixRightEdge":true,"lockVisibleTimeRangeOnResize":true,"rightBarStaysOnScroll":true,"timeVisible":true,"secondsVisible":true,"rightOffsetPixels":30}}"#,
        )
        .unwrap();
    chart.set_max_bar_spacing(50.0);

    let options: serde_json::Value =
        serde_json::from_str(&chart.time_scale_options_json()).unwrap();
    for key in [
        "bar_spacing",
        "right_offset",
        "min_bar_spacing",
        "max_bar_spacing",
        "right_offset_pixels",
        "time_visible",
        "seconds_visible",
        "fix_left_edge",
        "fix_right_edge",
        "lock_visible_time_range_on_resize",
        "right_bar_stays_on_scroll",
    ] {
        assert!(options.get(key).is_some(), "missing key {key}");
    }
    assert_eq!(options["bar_spacing"], 6.0);
    assert_eq!(options["right_offset"], 0.0);
    assert_eq!(options["min_bar_spacing"], 2.0);
    assert_eq!(options["max_bar_spacing"], 50.0);
    assert_eq!(options["right_offset_pixels"], 30.0);
    assert_eq!(options["time_visible"], true);
    assert_eq!(options["seconds_visible"], true);
    assert_eq!(options["fix_left_edge"], false);
    assert_eq!(options["fix_right_edge"], true);
    assert_eq!(options["lock_visible_time_range_on_resize"], true);
    assert_eq!(options["right_bar_stays_on_scroll"], true);
}

#[test]
fn series_style_options_use_aeris_defaults() {
    let chart = ChartEngine::new(800.0, 500.0, 1.0);
    let options: serde_json::Value =
        serde_json::from_str(&chart.series_options_json(0).unwrap()).unwrap();
    for key in [
        "last_value_visible",
        "price_line_visible",
        "price_line_source",
        "price_line_extent",
        "price_line_width",
        "price_line_color",
        "price_line_style",
        "line_style",
        "line_visible",
        "point_markers_radius",
        "crosshair_marker_visible",
        "crosshair_marker_radius",
        "crosshair_marker_border_color",
        "crosshair_marker_background_color",
        "crosshair_marker_border_width",
        "top_fill_color1",
        "top_fill_color2",
        "top_line_color",
        "top_line_width",
        "top_line_style",
        "bottom_fill_color1",
        "bottom_fill_color2",
        "bottom_line_color",
        "bottom_line_width",
        "bottom_line_style",
        "base",
        "invert_filled_area",
        "open_visible",
        "close_visible",
        "thin_bars",
    ] {
        assert!(options.get(key).is_some(), "missing key {key}");
    }
    assert_eq!(options["last_value_visible"], true);
    assert_eq!(options["price_line_visible"], true);
    assert_eq!(options["price_line_source"], 0); // PriceLineSource.LastBar
    assert_eq!(options["price_line_extent"], "partial");
    assert_eq!(options["price_line_width"], 1.0);
    assert_eq!(options["price_line_color"], "");
    assert_eq!(options["price_line_style"], 1); // LineStyle.Dotted
    assert_eq!(options["line_style"], 0); // LineStyle.Solid
    assert_eq!(options["line_visible"], true);
    assert_eq!(options["point_markers_radius"], serde_json::Value::Null);
    assert_eq!(options["crosshair_marker_visible"], false);
    assert_eq!(options["crosshair_marker_radius"], 4.0);
    assert_eq!(options["crosshair_marker_border_color"], "");
    assert_eq!(options["crosshair_marker_background_color"], "");
    assert_eq!(options["crosshair_marker_border_width"], 2.0);
    assert_eq!(options["top_fill_color1"], "");
    assert_eq!(options["top_fill_color2"], "");
    assert_eq!(options["top_line_color"], "");
    assert_eq!(options["top_line_width"], serde_json::Value::Null);
    assert_eq!(options["top_line_style"], 0);
    assert_eq!(options["bottom_fill_color1"], "");
    assert_eq!(options["bottom_fill_color2"], "");
    assert_eq!(options["bottom_line_color"], "");
    assert_eq!(options["bottom_line_width"], serde_json::Value::Null);
    assert_eq!(options["bottom_line_style"], 0);
    assert_eq!(options["base"], 0.0);
    assert_eq!(options["invert_filled_area"], false);
    assert_eq!(options["open_visible"], true);
    assert_eq!(options["close_visible"], true);
    assert_eq!(options["thin_bars"], true);
}

#[test]
fn series_apply_options_json_round_trips_all_new_fields() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    let patch = r##"{
        "last_value_visible": false,
        "price_line_visible": false,
        "price_line_source": 1,
        "price_line_extent": "full",
        "price_line_width": 2,
        "price_line_color": "#112233",
        "price_line_style": 0,
        "line_style": 2,
        "line_visible": false,
        "point_markers_radius": 6.5,
        "crosshair_marker_visible": false,
        "crosshair_marker_radius": 7,
        "crosshair_marker_border_color": "#445566",
        "crosshair_marker_background_color": "#778899",
        "crosshair_marker_border_width": 3,
        "top_fill_color1": "rgba(1,2,3,0.5)",
        "top_fill_color2": "#040506",
        "top_line_color": "#070809",
        "top_line_width": 5,
        "top_line_style": 1,
        "bottom_fill_color1": "#0a0b0c",
        "bottom_fill_color2": "#0d0e0f",
        "bottom_line_color": "#101112",
        "bottom_line_width": 6,
        "bottom_line_style": 3,
        "base": 42.5,
        "invert_filled_area": true,
        "open_visible": false,
        "close_visible": false,
        "thin_bars": false,
        "heikin_ashi": true,
        "title": "NDQ",
        "title_visible": false,
        "countdown_visible": true
    }"##;
    assert!(chart.series_apply_options_json(0, patch));
    let options: serde_json::Value =
        serde_json::from_str(&chart.series_options_json(0).unwrap()).unwrap();
    assert_eq!(options["last_value_visible"], false);
    assert_eq!(options["price_line_visible"], false);
    assert_eq!(options["price_line_source"], 1);
    assert_eq!(options["price_line_extent"], "full");
    assert_eq!(options["price_line_width"], 2.0);
    assert_eq!(options["price_line_color"], "#112233");
    assert_eq!(options["price_line_style"], 0);
    assert_eq!(options["line_style"], 2);
    assert_eq!(options["line_visible"], false);
    assert_eq!(options["point_markers_radius"], 6.5);
    assert_eq!(options["crosshair_marker_visible"], false);
    assert_eq!(options["crosshair_marker_radius"], 7.0);
    assert_eq!(options["crosshair_marker_border_color"], "#445566");
    assert_eq!(options["crosshair_marker_background_color"], "#778899");
    assert_eq!(options["crosshair_marker_border_width"], 3.0);
    assert_eq!(options["top_fill_color1"], "rgba(1,2,3,0.5)");
    assert_eq!(options["top_fill_color2"], "#040506");
    assert_eq!(options["top_line_color"], "#070809");
    assert_eq!(options["top_line_width"], 5.0);
    assert_eq!(options["top_line_style"], 1);
    assert_eq!(options["bottom_fill_color1"], "#0a0b0c");
    assert_eq!(options["bottom_fill_color2"], "#0d0e0f");
    assert_eq!(options["bottom_line_color"], "#101112");
    assert_eq!(options["bottom_line_width"], 6.0);
    assert_eq!(options["bottom_line_style"], 3);
    assert_eq!(options["base"], 42.5);
    assert_eq!(options["invert_filled_area"], true);
    assert_eq!(options["open_visible"], false);
    assert_eq!(options["close_visible"], false);
    assert_eq!(options["thin_bars"], false);
    assert_eq!(options["heikin_ashi"], true);
    assert_eq!(options["title"], "NDQ");
    assert_eq!(options["title_visible"], false);
    assert_eq!(options["countdown_visible"], true);

    // Round-trip parity: re-applying the serialized options is a fixed point.
    let serialized = chart.series_options_json(0).unwrap();
    assert!(chart.series_apply_options_json(0, &serialized));
    assert_eq!(chart.series_options_json(0).unwrap(), serialized);

    // "" clears a pinned color, null restores an auto/follow numeric slot.
    assert!(chart.series_apply_options_json(
        0,
        r#"{"price_line_color": "", "point_markers_radius": null, "top_line_width": null}"#
    ));
    let options: serde_json::Value =
        serde_json::from_str(&chart.series_options_json(0).unwrap()).unwrap();
    assert_eq!(options["price_line_color"], "");
    assert_eq!(options["point_markers_radius"], serde_json::Value::Null);
    assert_eq!(options["top_line_width"], serde_json::Value::Null);
}

#[test]
fn series_apply_options_json_round_trips_color_strings_verbatim() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    // reference stores the user's color string verbatim: hex case, rgba() spacing, and named colors
    // all come back from options() exactly as applied (named colors the renderer cannot parse
    // fall back to the default at render time, but still round-trip).
    assert!(chart.series_apply_options_json(
        0,
        r##"{"price_line_color": "#FF0000",
             "top_line_color": "#FF0000",
             "top_fill_color1": "rgba(1, 2, 3, 0.5)",
             "top_fill_color2": "red",
             "bottom_line_color": "#FF0000",
             "bottom_fill_color1": "rgba(1, 2, 3, 0.5)",
             "bottom_fill_color2": "red"}"##
    ));
    let options: serde_json::Value =
        serde_json::from_str(&chart.series_options_json(0).unwrap()).unwrap();
    assert_eq!(options["price_line_color"], "#FF0000");
    assert_eq!(options["top_line_color"], "#FF0000");
    assert_eq!(options["top_fill_color1"], "rgba(1, 2, 3, 0.5)");
    assert_eq!(options["top_fill_color2"], "red");
    assert_eq!(options["bottom_line_color"], "#FF0000");
    assert_eq!(options["bottom_fill_color1"], "rgba(1, 2, 3, 0.5)");
    assert_eq!(options["bottom_fill_color2"], "red");

    // The serialized options remain a fixed point under re-apply.
    let serialized = chart.series_options_json(0).unwrap();
    assert!(chart.series_apply_options_json(0, &serialized));
    assert_eq!(chart.series_options_json(0).unwrap(), serialized);
}

#[test]
fn series_apply_options_json_ignores_unknown_keys_and_bad_input() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    // Unknown keys, wrong types, and out-of-range enum values leave state untouched.
    assert!(chart.series_apply_options_json(
        0,
        r#"{"unknown_key": 1, "line_style": 9, "price_line_source": 4, "price_line_extent": "wide", "line_visible": "yes",
            "price_line_width": -2, "price_line_color": 7}"#
    ));
    let options: serde_json::Value =
        serde_json::from_str(&chart.series_options_json(0).unwrap()).unwrap();
    assert_eq!(options["line_style"], 0);
    assert_eq!(options["price_line_source"], 0);
    assert_eq!(options["price_line_extent"], "partial");
    assert_eq!(options["line_visible"], true);
    assert_eq!(options["price_line_width"], 1.0);
    assert_eq!(options["price_line_color"], "");

    // A partial patch merges: untouched keys keep their values (reference merge semantics).
    assert!(chart.series_apply_options_json(0, r#"{"line_style": 3}"#));
    let options: serde_json::Value =
        serde_json::from_str(&chart.series_options_json(0).unwrap()).unwrap();
    assert_eq!(options["line_style"], 3);
    assert_eq!(options["last_value_visible"], true);

    // Malformed JSON and unknown/removed ids report failure without touching state.
    assert!(!chart.series_apply_options_json(0, "{ nope"));
    assert!(!chart.series_apply_options_json(999, "{}"));
    assert!(!chart.series_apply_options_json(0, "[]"));
    let options: serde_json::Value =
        serde_json::from_str(&chart.series_options_json(0).unwrap()).unwrap();
    assert_eq!(options["line_style"], 3);
}

// --- per-data-point colors (reference data-item colors) ---

/// Line chart with four bars and a red body override on bar 1.
fn point_colored_chart() -> ChartEngine {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    chart.series[0].kind = SeriesKind::Line;
    let times = [1.0, 2.0, 3.0, 4.0];
    let values = [10.0, 11.0, 12.0, 13.0];
    chart
        .set_series_data(0, &times, &values, &values, &values, &values)
        .unwrap();
    assert!(chart.set_series_point_colors(0, Some(vec![0, 0xFF0000FF, 0, 0]), None, None));
    chart.time_scale.set_width(800.0);
    chart.fit_content();
    chart
}

#[test]
fn point_colors_validate_lengths_and_reset_on_set_data() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    let times = [1.0, 2.0, 3.0];
    let values = [10.0, 11.0, 12.0];
    chart
        .set_series_data(0, &times, &values, &values, &values, &values)
        .unwrap();

    // A channel that does not match the row count rejects the whole call (no partial state).
    assert!(!chart.set_series_point_colors(0, Some(vec![1, 2]), None, None));
    assert!(!chart.data.has_point_colors(0));
    // Unknown series ids reject.
    assert!(!chart.set_series_point_colors(99, Some(vec![1, 2, 3]), None, None));

    assert!(chart.set_series_point_colors(0, Some(vec![1, 2, 3]), None, None));
    assert!(chart.data.has_point_colors(0));

    // set_series_data invalidates the point colors (the host re-installs them after).
    chart
        .set_series_data(0, &times, &values, &values, &values, &values)
        .unwrap();
    assert!(!chart.data.has_point_colors(0));
}

/// Body-channel override at `row` (reference data-item color), for the point-color tests.
fn body_color_at(chart: &ChartEngine, id: SeriesId, row: usize) -> Option<u32> {
    chart.data.point_color(
        id,
        aeris_charts_core::model::data_layer::PointColorChannel::Body,
        row,
    )
}

#[test]
fn point_colors_stay_aligned_through_updates() {
    let mut chart = point_colored_chart();
    assert_eq!(body_color_at(&chart, 0, 1), Some(0xFF0000FF));

    // Append with a styled update: the new bar carries its own channels.
    assert!(chart.update_series_bar_styled(
        0,
        5.0,
        [14.0, 14.0, 14.0, 14.0],
        [Some(0x00FF00FF), None, None],
    ));
    assert_eq!(body_color_at(&chart, 0, 4), Some(0x00FF00FF));
    assert_eq!(body_color_at(&chart, 0, 1), Some(0xFF0000FF));

    // Append with a plain update: no override on the new bar, channels stay aligned.
    assert!(chart.update_series_bar(0, 6.0, [15.0, 15.0, 15.0, 15.0]));
    assert_eq!(body_color_at(&chart, 0, 5), None);
    assert_eq!(body_color_at(&chart, 0, 4), Some(0x00FF00FF));

    // Replace-last with a styled update retargets the bar's channels.
    assert!(chart.update_series_bar_styled(
        0,
        6.0,
        [15.0, 15.0, 15.0, 15.0],
        [Some(0x0000FFFF), None, None],
    ));
    assert_eq!(body_color_at(&chart, 0, 5), Some(0x0000FFFF));

    // Insert ahead of the first bar with a plain update (the rebuild path): existing
    // overrides shift with their rows.
    assert!(chart.update_series_bar(0, 0.0, [8.0, 8.0, 8.0, 8.0]));
    assert_eq!(body_color_at(&chart, 0, 0), None); // the inserted bar
    assert_eq!(body_color_at(&chart, 0, 2), Some(0xFF0000FF)); // old row 1
    assert_eq!(body_color_at(&chart, 0, 6), Some(0x0000FFFF)); // old row 5
}

#[test]
fn point_colors_follow_the_winning_row_under_dedupe() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    // Duplicate time 2: the later row (value 25, color 99) wins, taking its color along.
    let report = chart
        .set_series_data_styled(
            0,
            &[1.0, 2.0, 2.0, 3.0],
            &[10.0, 20.0, 25.0, 30.0],
            &[10.0, 20.0, 25.0, 30.0],
            &[10.0, 20.0, 25.0, 30.0],
            &[10.0, 20.0, 25.0, 30.0],
            [Some(vec![10, 20, 99, 30]), None, None],
        )
        .unwrap();
    assert_eq!(report.dropped_duplicate, 1);
    assert_eq!(
        (0..3)
            .map(|row| body_color_at(&chart, 0, row))
            .collect::<Vec<_>>(),
        vec![Some(10), Some(99), Some(30)]
    );

    // A color channel length mismatch rejects the ingest like a column mismatch.
    assert!(chart
        .set_series_data_styled(
            0,
            &[1.0, 2.0],
            &[1.0, 2.0],
            &[1.0, 2.0],
            &[1.0, 2.0],
            &[1.0, 2.0],
            [Some(vec![1]), None, None],
        )
        .is_err());
}

// --- per-series price_format (reference PriceFormat) ---

#[test]
fn price_format_defaults_and_options_round_trip() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    // reference series-options-defaults.ts: {type:'price', precision:2, minMove:0.01}.
    let options: serde_json::Value =
        serde_json::from_str(&chart.series_options_json(0).unwrap()).unwrap();
    assert_eq!(
        options["price_format"],
        serde_json::json!({"type": "price", "precision": 2, "min_move": 0.01})
    );

    // Applying each built-in type round-trips through series_options_json; a nested
    // price_format key in a general options patch routes to the same applier.
    assert!(chart.series_apply_price_format_json(0, r#"{"type": "volume", "precision": 1}"#));
    let options: serde_json::Value =
        serde_json::from_str(&chart.series_options_json(0).unwrap()).unwrap();
    // the reference's PriceFormatVolume is exactly {type:"volume"}; the apply-time precision superset is
    // kept by the formatter but not serialized.
    assert_eq!(
        options["price_format"],
        serde_json::json!({"type": "volume"})
    );
    assert!(chart.series_apply_options_json(0, &chart.series_options_json(0).unwrap()));

    assert!(chart.series_apply_options_json(
        0,
        r#"{"price_format": {"type": "percent", "precision": 3}}"#
    ));
    let options: serde_json::Value =
        serde_json::from_str(&chart.series_options_json(0).unwrap()).unwrap();
    assert_eq!(
        options["price_format"],
        serde_json::json!({"type": "percent", "precision": 3})
    );

    assert!(chart.series_apply_price_format_json(
        0,
        r#"{"type": "price", "precision": 4, "min_move": 0.0001}"#
    ));
    let options: serde_json::Value =
        serde_json::from_str(&chart.series_options_json(0).unwrap()).unwrap();
    assert_eq!(
        options["price_format"],
        serde_json::json!({"type": "price", "precision": 4, "min_move": 0.0001})
    );
    // Fixed-point round-trip.
    let serialized = chart.series_options_json(0).unwrap();
    assert!(chart.series_apply_options_json(0, &serialized));
    assert_eq!(chart.series_options_json(0).unwrap(), serialized);

    // Malformed patches and unknown types/ids report failure.
    assert!(!chart.series_apply_price_format_json(0, "{ nope"));
    assert!(!chart.series_apply_price_format_json(0, r#"{"type": "nope"}"#));
    assert!(!chart.series_apply_price_format_json(0, r#"{"precision": 2}"#));
    assert!(!chart.series_apply_price_format_json(999, r#"{"type": "price"}"#));
}

#[test]
fn price_format_drives_last_value_label_and_ticks() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    chart.series[0].kind = SeriesKind::Line;
    let times = [1.0, 2.0, 3.0];
    let values = [1500.0, 2500.0, 2_500_000.0];
    chart
        .set_series_data(0, &times, &values, &values, &values, &values)
        .unwrap();
    chart.time_scale.set_width(800.0);
    chart.fit_content();

    let label_texts = |chart: &mut ChartEngine| {
        chart
            .build_axis_frame(
                80.0,
                |t, _bold| t.len() as f64 * 7.0,
                |t, _bold| t.len() as f64 * 6.0,
            )
            .labels
            .into_iter()
            .map(|l| l.text)
            .collect::<Vec<_>>()
    };

    // Default: two-decimal price labels.
    let texts = label_texts(&mut chart);
    assert!(texts.iter().any(|t| t == "2,500,000.00"), "{texts:?}");

    // Volume format: K/M/B suffixes on the series' last-value label and the axis ticks
    // (the series is the scale's primary source).
    assert!(chart.series_apply_price_format_json(0, r#"{"type": "volume", "precision": 1}"#));
    let texts = label_texts(&mut chart);
    assert!(texts.iter().any(|t| t == "2.5M"), "{texts:?}");
    assert!(!texts.iter().any(|t| t == "2,500,000.00"), "{texts:?}");

    // Percent format: a % sign (precision as decimal digits; see the reference-quirk note at
    // `format_with_price_format`).
    assert!(chart.series_apply_price_format_json(0, r#"{"type": "percent", "precision": 0}"#));
    let texts = label_texts(&mut chart);
    assert!(texts.iter().any(|t| t.ends_with('%')), "{texts:?}");

    // Price format with four decimals.
    assert!(chart.series_apply_price_format_json(
        0,
        r#"{"type": "price", "precision": 4, "min_move": 0.0001}"#
    ));
    let texts = label_texts(&mut chart);
    assert!(texts.iter().any(|t| t == "2,500,000.0000"), "{texts:?}");
}

#[test]
fn price_format_custom_fn_invocation_and_clearing() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    chart.series[0].kind = SeriesKind::Line;
    let times = [1.0, 2.0, 3.0];
    let values = [10.0, 11.0, 12.0];
    chart
        .set_series_data(0, &times, &values, &values, &values, &values)
        .unwrap();
    chart.time_scale.set_width(800.0);
    chart.fit_content();

    let label_texts = |chart: &mut ChartEngine| {
        chart
            .build_axis_frame(
                80.0,
                |t, _bold| t.len() as f64 * 7.0,
                |t, _bold| t.len() as f64 * 6.0,
            )
            .labels
            .into_iter()
            .map(|l| l.text)
            .collect::<Vec<_>>()
    };

    // The custom fn drives the series' labels (reference priceFormat.formatter).
    assert!(chart.set_series_price_formatter(0, Box::new(|price| Some(format!("P{price:.1}"))),));
    let texts = label_texts(&mut chart);
    assert!(texts.iter().any(|t| t == "P12.0"), "{texts:?}");
    // Custom serializes without the fn.
    let options: serde_json::Value =
        serde_json::from_str(&chart.series_options_json(0).unwrap()).unwrap();
    assert_eq!(
        options["price_format"],
        serde_json::json!({"type": "custom", "min_move": 0.01})
    );

    // `{type:"custom"}` keeps the installed fn (reference merge of a partial priceFormat patch).
    assert!(chart.series_apply_price_format_json(0, r#"{"type": "custom", "min_move": 0.5}"#));
    let texts = label_texts(&mut chart);
    assert!(texts.iter().any(|t| t == "P12.0"), "{texts:?}");

    // A declining fn (None return, e.g. a throw at the JS boundary) falls back to built-in.
    assert!(chart.set_series_price_formatter(0, Box::new(|_| None)));
    let texts = label_texts(&mut chart);
    assert!(texts.iter().any(|t| t == "12.00"), "{texts:?}");

    // Switching to a non-custom type clears the fn: back to custom, no fn remains.
    assert!(chart.series_apply_price_format_json(0, r#"{"type": "price"}"#));
    assert!(chart.series_apply_price_format_json(0, r#"{"type": "custom"}"#));
    let texts = label_texts(&mut chart);
    assert!(texts.iter().any(|t| t == "12.00"), "{texts:?}");
    assert!(!texts.iter().any(|t| t == "P12.0"), "{texts:?}");
}

#[test]
fn price_format_labels_follow_their_owning_series() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    chart.series[0].kind = SeriesKind::Line;
    let times = [1.0, 2.0, 3.0];
    let values = [10.0, 11.0, 12.0];
    chart
        .set_series_data(0, &times, &values, &values, &values, &values)
        .unwrap();
    let second = chart.add_series(SeriesKind::Line);
    chart
        .set_series_data(second, &times, &values, &values, &values, &values)
        .unwrap();
    chart.time_scale.set_width(800.0);
    chart.fit_content();

    // The second series formats with four decimals; the primary stays at the default.
    assert!(chart.series_apply_price_format_json(
        second,
        r#"{"type": "price", "precision": 4, "min_move": 0.0001}"#
    ));
    // Its price-line label uses its OWN format.
    let line_id = chart.create_price_line(
        second,
        11.0,
        Color::rgb(0, 0, 0),
        1,
        aeris_charts_render::draw_list::LineStyle::Solid,
        "",
    );
    assert!(line_id > 0);
    let axis = chart.build_axis_frame(
        80.0,
        |t, _bold| t.len() as f64 * 7.0,
        |t, _bold| t.len() as f64 * 6.0,
    );
    let boxed: Vec<_> = axis
        .labels
        .iter()
        .filter(|l| l.background.is_some())
        .map(|l| l.text.clone())
        .collect();
    assert!(
        boxed.iter().any(|t| t == "12.0000"),
        "second series' own last-value label: {boxed:?}"
    );
    assert!(
        boxed.iter().any(|t| t == "11.0000"),
        "second series' price-line label: {boxed:?}"
    );
    assert!(
        boxed.iter().any(|t| t == "12.00"),
        "primary series' default-format label: {boxed:?}"
    );

    // The crosshair price label uses the label source's format: restore the second series to
    // the default, give the PRIMARY series the four-decimal format, and magnet-snap the
    // crosshair to the 11.0 close â€” "11.0000" is a text no other label produces.
    assert!(chart.series_apply_price_format_json(
        second,
        r#"{"type": "price", "precision": 2, "min_move": 0.01}"#
    ));
    assert!(chart.series_apply_price_format_json(
        0,
        r#"{"type": "price", "precision": 4, "min_move": 0.0001}"#
    ));
    chart.crosshair_mode = CrosshairMode::Magnet;
    let y11 = chart.series_price_to_coordinate(0, 11.0).unwrap();
    let x1 = chart.time_to_coordinate(2.0).unwrap();
    chart.crosshair = Some((x1, y11));
    let axis = chart.build_axis_frame(
        80.0,
        |t, _bold| t.len() as f64 * 7.0,
        |t, _bold| t.len() as f64 * 6.0,
    );
    assert!(
        axis.labels
            .iter()
            .any(|l| l.background.is_some() && l.text == "11.0000"),
        "crosshair label in the primary (label source) series' format"
    );
    assert!(
        axis.labels
            .iter()
            .any(|l| l.background.is_some() && l.text == "12.00"),
        "second series back to the default format"
    );
}

// ---- wave: shiftVisibleRangeOnNewBar / whitespace / pop / lastValueData / programmatic ----
// ---- crosshair / locale / series order / verbatim colors (reference ports; see item refs)   ----

/// Install `n` ascending real bars (close = 100 + i) on series 0 and lay out the scale.
fn install_bars(chart: &mut ChartEngine, n: usize) {
    chart.time_scale.set_width(800.0);
    let times: Vec<f64> = (1..=n).map(|i| i as f64).collect();
    let values: Vec<f64> = (0..n).map(|i| 100.0 + i as f64).collect();
    chart
        .set_series_data(0, &times, &values, &values, &values, &values)
        .unwrap();
}

#[test]
fn new_bar_shift_follows_at_the_right_edge() {
    // reference chart-model.ts:968-983: last bar visible + shiftVisibleRangeOnNewBar (default
    // true) -> no right-offset compensation, the view follows the new bar.
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    install_bars(&mut chart, 10);
    assert_eq!(chart.right_offset(), 0.0);
    chart.update_series_bar(0, 11.0, [109.0, 110.0, 108.0, 109.0]);
    assert_eq!(chart.right_offset(), 0.0, "right edge follows new bars");
    assert_eq!(chart.time_scale.base_index(), 10);
}

#[test]
fn new_bar_compensation_keeps_bars_when_scrolled_back() {
    // Scrolled into the past (last bar not visible): the right offset compensates by the
    // number of new bars so the same bars stay in view (no drift).
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    install_bars(&mut chart, 10);
    chart.set_right_offset(-5.0);
    let before = chart.time_scale.visible_strict_range().unwrap();
    chart.update_series_bar(0, 11.0, [109.0, 110.0, 108.0, 109.0]);
    assert_eq!(chart.right_offset(), -6.0);
    let after = chart.time_scale.visible_strict_range().unwrap();
    assert_eq!(
        (before.left(), before.right()),
        (after.left(), after.right()),
        "same bars stay in view after compensation"
    );
}

#[test]
fn new_bar_compensation_when_shift_disabled_at_the_edge() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    install_bars(&mut chart, 10);
    chart.set_shift_visible_range_on_new_bar(false);
    chart.update_series_bar(0, 11.0, [109.0, 110.0, 108.0, 109.0]);
    assert_eq!(chart.right_offset(), -1.0);
    // After the compensation the last bar is outside the visible range, so the next append
    // keeps compensating even with the option back on (reference parity: the view stays put).
    chart.set_shift_visible_range_on_new_bar(true);
    chart.update_series_bar(0, 12.0, [110.0, 111.0, 109.0, 110.0]);
    assert_eq!(chart.right_offset(), -2.0);
}

#[test]
fn whitespace_replacement_shift_is_gated_by_the_option() {
    let nan = f64::NAN;
    // 10 real bars plus an explicit whitespace time point at 11.
    let build = |chart: &mut ChartEngine| {
        chart.time_scale.set_width(800.0);
        let times: Vec<f64> = (1..=11).map(|i| i as f64).collect();
        let values: Vec<f64> = (0..10)
            .map(|i| 100.0 + i as f64)
            .chain(std::iter::once(nan))
            .collect();
        chart
            .set_series_data(0, &times, &values, &values, &values, &values)
            .unwrap();
        assert_eq!(chart.time_scale.base_index(), 9, "base skips trailing ws");
    };

    // Default (allow=false): replacing the whitespace at the right edge compensates.
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    build(&mut chart);
    chart.update_series_bar(0, 11.0, [110.0, 111.0, 109.0, 110.0]);
    assert_eq!(chart.right_offset(), -1.0);

    // allow=true: the view follows the replacement like a real new bar.
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    build(&mut chart);
    chart.set_allow_shift_visible_range_on_whitespace_replacement(true);
    chart.update_series_bar(0, 11.0, [110.0, 111.0, 109.0, 110.0]);
    assert_eq!(chart.right_offset(), 0.0);
}

#[test]
fn time_scale_shift_options_route_via_json_and_round_trip() {
    let mut chart = ChartEngine::new(300.0, 200.0, 1.0);
    let options: serde_json::Value =
        serde_json::from_str(&chart.time_scale_options_json()).unwrap();
    // reference defaults (time-scale-options-defaults.ts:17-18)
    assert_eq!(options["shift_visible_range_on_new_bar"], true);
    assert_eq!(
        options["allow_shift_visible_range_on_whitespace_replacement"],
        false
    );
    chart
        .apply_options(
            r#"{"timeScale":{"shiftVisibleRangeOnNewBar":false,"allowShiftVisibleRangeOnWhitespaceReplacement":true}}"#,
        )
        .unwrap();
    let options: serde_json::Value =
        serde_json::from_str(&chart.time_scale_options_json()).unwrap();
    assert_eq!(options["shift_visible_range_on_new_bar"], false);
    assert_eq!(
        options["allow_shift_visible_range_on_whitespace_replacement"],
        true
    );
}

// ---- whitespace data items (reference {time}-only rows) ----

#[test]
fn whitespace_rows_draw_nothing_but_keep_their_slots() {
    use aeris_charts_render::draw_list::Prim;
    let nan = f64::NAN;
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    chart.series[0].kind = SeriesKind::Line;
    chart
        .set_series_data(
            0,
            &[1.0, 2.0, 3.0, 4.0, 5.0],
            &[10.0, 11.0, nan, 12.0, 13.0],
            &[10.0, 11.0, nan, 12.0, 13.0],
            &[10.0, 11.0, nan, 12.0, 13.0],
            &[10.0, 11.0, nan, 12.0, 13.0],
        )
        .unwrap();
    // the whitespace time keeps its merged slot (reference keeps the time-scale point)
    assert_eq!(chart.data.merged_times(), &[1, 2, 3, 4, 5]);
    chart.time_scale.set_width(800.0);
    chart.fit_content();
    let frame = chart.build_frame();
    let points: u32 = frame.panes[0]
        .main
        .iter()
        .map(|p| match p {
            Prim::Polyline { point_count, .. } => *point_count,
            _ => 0,
        })
        .sum();
    // the line skips the whitespace row and connects the four real bars across the gap
    assert_eq!(points, 4);
    // and the autoscale ignores it
    let range = chart.panes[0].price_scale.price_range().unwrap();
    assert_eq!(range.min_value(), 10.0);
    assert_eq!(range.max_value(), 13.0);
}

#[test]
fn whitespace_update_replaces_the_bar_and_skips_last_value() {
    let nan = f64::NAN;
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    install_bars(&mut chart, 3);
    chart.fit_content();
    // reference `series.update` with a {time}-only item replaces the last bar with whitespace.
    assert!(chart.update_series_bar(0, 3.0, [nan, nan, nan, nan]));
    let data = chart.series_data(0);
    assert_eq!(data.len(), 3);
    assert!(data[2].close.is_nan());
    // last-value label tracks the last real bar (close of bar 2 = 101)
    let axis = chart.build_axis_frame(
        80.0,
        |t, _bold| t.len() as f64 * 7.0,
        |t, _bold| t.len() as f64 * 6.0,
    );
    let texts: Vec<String> = axis.labels.iter().map(|l| l.text.clone()).collect();
    assert!(
        texts.iter().any(|t| t == "101.00"),
        "last-value label skips whitespace: {texts:?}"
    );
    assert!(!texts.iter().any(|t| t == "102.00"));
}

#[test]
fn magnet_treats_whitespace_as_no_bar() {
    use aeris_charts_render::draw_list::Prim;
    let nan = f64::NAN;
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    chart
        .set_series_data(
            0,
            &[1.0, 2.0, 3.0],
            &[10.0, nan, 12.0],
            &[10.0, nan, 12.0],
            &[10.0, nan, 12.0],
            &[10.0, nan, 12.0],
        )
        .unwrap();
    chart.time_scale.set_width(800.0);
    chart.fit_content();
    chart.crosshair_mode = CrosshairMode::Magnet;
    // Cursor over the whitespace bar (time 2): no candidate, the horizontal line stays at
    // the raw cursor y instead of snapping to a bar price. (The crosshair's HLine is told
    // apart from the built-in last-price line by the crosshair grey — both are Dashed now.)
    let crosshair_hline_y = |frame: &ChartFrame| {
        frame.panes[0].main.iter().find_map(|p| match p {
            Prim::HLine { y, color, .. }
                if *color
                    == Color::rgb(
                        aeris_charts_core::style::DEFAULT_CROSSHAIR_RGB.0,
                        aeris_charts_core::style::DEFAULT_CROSSHAIR_RGB.1,
                        aeris_charts_core::style::DEFAULT_CROSSHAIR_RGB.2,
                    ) =>
            {
                Some(*y)
            }
            _ => None,
        })
    };
    let x_ws = chart.time_to_coordinate(2.0).unwrap();
    chart.crosshair = Some((x_ws, 10.0));
    let frame = chart.build_frame();
    assert_eq!(
        crosshair_hline_y(&frame),
        Some(10),
        "whitespace: no magnet snap"
    );
    // Over the real bar at time 3 the magnet snaps to its close coordinate.
    let x_bar = chart.time_to_coordinate(3.0).unwrap();
    chart.crosshair = Some((x_bar, 10.0));
    let snapped = chart.series_price_to_coordinate(0, 12.0).unwrap();
    let frame = chart.build_frame();
    assert_eq!(crosshair_hline_y(&frame), Some(snapped.round() as i32));
}

// ---- series pop / lastValueData / priceFormatter ----

#[test]
fn series_pop_removes_tail_and_shifts_point_colors() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    install_bars(&mut chart, 5);
    assert!(chart.set_series_point_colors(0, Some(vec![11, 22, 33, 44, 55]), None, None));
    assert_eq!(chart.series_pop(0, 0), Some(5), "count 0 is a no-op");
    assert_eq!(chart.series_pop(0, 2), Some(3));
    assert_eq!(
        chart.data.point_color(
            0,
            aeris_charts_core::model::data_layer::PointColorChannel::Body,
            0
        ),
        Some(11)
    );
    assert_eq!(
        chart.data.point_color(
            0,
            aeris_charts_core::model::data_layer::PointColorChannel::Body,
            2
        ),
        Some(33)
    );
    assert_eq!(
        chart.data.point_color(
            0,
            aeris_charts_core::model::data_layer::PointColorChannel::Body,
            3
        ),
        None
    );
    assert_eq!(chart.data.merged_times(), &[1, 2, 3]);
    // clamp to the data length; unknown/removed ids report None
    assert_eq!(chart.series_pop(0, 99), Some(0));
    assert_eq!(chart.series_pop(42, 1), None);
}

#[test]
fn series_last_value_data_global_visible_and_whitespace() {
    let nan = f64::NAN;
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    chart
        .set_series_data(
            0,
            &[1.0, 2.0, 3.0, 4.0],
            &[10.0, 20.0, 30.0, nan],
            &[10.0, 20.0, 30.0, nan],
            &[10.0, 20.0, 30.0, nan],
            &[10.0, 20.0, 30.0, nan],
        )
        .unwrap();
    chart.time_scale.set_width(800.0);
    chart.fit_content();

    // global last: skips the trailing whitespace bar
    let global: serde_json::Value =
        serde_json::from_str(&chart.series_last_value_data(0, true).unwrap()).unwrap();
    assert_eq!(global["value"], 30.0);
    assert_eq!(global["formatted"], "30.00");
    assert_eq!(global["time"], 3);
    // visible last with the right edge at index 1: the bar at time 2
    chart.set_right_offset(-1.0);
    let visible: serde_json::Value =
        serde_json::from_str(&chart.series_last_value_data(0, false).unwrap()).unwrap();
    assert_eq!(visible["value"], 20.0);
    assert_eq!(visible["time"], 2);
    // unknown id -> None ("" at the wasm boundary)
    assert!(chart.series_last_value_data(42, true).is_none());
}

#[test]
fn series_format_price_uses_the_resolved_price_format() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    install_bars(&mut chart, 2);
    // built-in default (price, 2 decimals)
    assert_eq!(
        chart.series_format_price(0, 12.345).as_deref(),
        Some("12.35")
    );
    // per-series precision
    assert!(chart
        .series_apply_price_format_json(0, r#"{"type":"price","precision":4,"min_move":0.0001}"#));
    assert_eq!(
        chart.series_format_price(0, 12.345).as_deref(),
        Some("12.3450")
    );
    // volume / percent built-ins
    assert!(chart.series_apply_price_format_json(0, r#"{"type":"volume"}"#));
    assert_eq!(
        chart.series_format_price(0, 1500.0).as_deref(),
        Some("1.5K")
    );
    assert!(chart.series_apply_price_format_json(0, r#"{"type":"percent","precision":1}"#));
    assert_eq!(
        chart.series_format_price(0, 12.345).as_deref(),
        Some("12.3%")
    );
    // custom fn first, declining fn -> chart formatter fallback -> built-in
    chart.set_series_price_formatter(0, Box::new(|v| Some(format!("px:{v}"))));
    assert_eq!(chart.series_format_price(0, 1.5).as_deref(), Some("px:1.5"));
    chart.set_series_price_formatter(0, Box::new(|_| None));
    assert_eq!(chart.series_format_price(0, 1.5).as_deref(), Some("1.50"));
    chart.set_price_formatter(Some(Box::new(|v| Some(format!("chart:{v}")))));
    assert_eq!(
        chart.series_format_price(0, 1.5).as_deref(),
        Some("chart:1.5")
    );
    assert!(chart.series_format_price(42, 1.5).is_none());
}

// ---- programmatic crosshair ----

#[test]
fn crosshair_position_set_reject_and_clear() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    install_bars(&mut chart, 5);
    chart.fit_content();
    // lay the panes/scales out (a frame build does layout + autoscale)
    chart.build_frame();
    // time resolves to a bar: x is the bar coordinate, y the price on the series' scale
    let x = chart.time_to_coordinate(3.0).unwrap();
    let y = chart.series_price_to_coordinate(0, 102.0).unwrap();
    assert!(chart.set_crosshair_position(102.0, 3.0, 0));
    let (cx, cy) = chart.crosshair.unwrap();
    assert_eq!(cx, x);
    assert_eq!(cy, y);
    // a non-bar time is rejected and leaves the previous position untouched
    assert!(!chart.set_crosshair_position(102.0, 3.5, 0));
    assert_eq!(chart.crosshair, Some((x, y)));
    // unknown series / non-finite price rejected
    assert!(!chart.set_crosshair_position(102.0, 3.0, 42));
    assert!(!chart.set_crosshair_position(f64::NAN, 3.0, 0));
    // clear drops the stored position
    chart.clear_crosshair_position();
    assert_eq!(chart.crosshair, None);
    // a following frame draws it: set again and check the crosshair prims exist
    assert!(chart.set_crosshair_position(102.0, 3.0, 0));
    let frame = chart.build_frame();
    assert!(frame.panes[0]
        .main
        .iter()
        .any(|p| matches!(p, aeris_charts_render::draw_list::Prim::VLine { .. })));
}

// ---- locale / dateFormat ----

/// Crosshair time-label text of the bar at `time`, with the crosshair parked on it.
fn crosshair_time_label(chart: &mut ChartEngine, time: f64) -> Option<String> {
    let x = chart.time_to_coordinate(time)?;
    chart.crosshair = Some((x, 10.0));
    chart
        .build_axis_frame(
            80.0,
            |t, _bold| t.len() as f64 * 7.0,
            |t, _bold| t.len() as f64 * 6.0,
        )
        .labels
        .into_iter()
        .find(|l| l.background.is_some() && l.midpoint == AxisTextMidpoint::StableTime)
        .map(|l| l.text)
}

#[test]
fn date_format_drives_the_crosshair_time_label() {
    // 2018-06-25T14:30:45Z
    let ts = 1_529_937_045.0;
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    chart.set_time_visible(false); // date-only labels for these assertions
    chart
        .set_series_data(0, &[ts], &[10.0], &[10.0], &[10.0], &[10.0])
        .unwrap();
    chart.time_scale.set_width(800.0);
    chart.fit_content();
    // reference default `dd MMM 'yy`
    assert_eq!(
        crosshair_time_label(&mut chart, ts).as_deref(),
        Some("25 Jun '18")
    );
    chart.set_date_format("yyyy-MM-dd");
    assert_eq!(
        crosshair_time_label(&mut chart, ts).as_deref(),
        Some("2018-06-25")
    );
    chart.set_date_format("MMMM d, yyyy");
    assert_eq!(
        crosshair_time_label(&mut chart, ts).as_deref(),
        Some("June 25, 2018")
    );
    // options JSON routing (reference `applyOptions({ localization })`)
    chart
        .apply_options(r#"{"localization":{"dateFormat":"d/M/yy"}}"#)
        .unwrap();
    assert_eq!(
        crosshair_time_label(&mut chart, ts).as_deref(),
        Some("25/6/18")
    );
}

#[test]
fn injected_locale_month_names_drive_labels() {
    let ts = 1_529_937_045.0; // 2018-06-25T14:30:45Z
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    chart.set_time_visible(false);
    chart
        .set_series_data(0, &[ts], &[10.0], &[10.0], &[10.0], &[10.0])
        .unwrap();
    chart.time_scale.set_width(800.0);
    chart.fit_content();
    let mut short: [String; 12] = Default::default();
    let mut long: [String; 12] = Default::default();
    for (i, name) in [
        "Jan", "Feb", "Mär", "Apr", "Mai", "Jun", "Jul", "Aug", "Sep", "Okt", "Nov", "Dez",
    ]
    .iter()
    .enumerate()
    {
        short[i] = name.to_string();
        long[i] = format!("{name}ius");
    }
    chart.set_month_names(short, long);
    chart.set_date_format("dd MMM yyyy");
    assert_eq!(
        crosshair_time_label(&mut chart, ts).as_deref(),
        Some("25 Jun 2018")
    );
    chart.set_date_format("MMMM yyyy");
    assert_eq!(
        crosshair_time_label(&mut chart, ts).as_deref(),
        Some("Junius 2018")
    );
}

// ---- last-price pulse defaults ----

#[test]
fn line_and_area_pulse_by_default_and_every_other_kind_stays_static() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    for kind in [
        SeriesKind::Line,
        SeriesKind::Area,
        SeriesKind::Candlestick,
        SeriesKind::Bar,
        SeriesKind::Histogram,
        SeriesKind::Baseline,
    ] {
        let id = chart.add_series(kind);
        assert_eq!(
            chart.series_entry_mut(id).unwrap().last_price_animation,
            matches!(kind, SeriesKind::Line | SeriesKind::Area),
            "{kind:?} pulse default"
        );
    }
}

#[test]
fn line_defaults_are_two_pixels_and_indicator_lines_never_pulse() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    install_bars(&mut chart, 30);
    let line = chart.add_series(SeriesKind::Line);
    let options: serde_json::Value =
        serde_json::from_str(&chart.series_options_json(line).unwrap()).unwrap();
    assert_eq!(options["line_width"], 2.0);
    let sma = chart.add_sma(0, 5).expect("sma output");
    let output = chart.series_entry_mut(sma).unwrap();
    assert_eq!(output.line_width, Some(2.0));
    assert!(!output.last_price_animation, "study lines do not pulse");
}

#[test]
fn pulse_follows_kind_defaults_but_an_explicit_choice_survives_type_changes() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    let id = chart.add_series(SeriesKind::Candlestick);
    let pulse = |chart: &mut ChartEngine| chart.series_entry_mut(id).unwrap().last_price_animation;
    chart.convert_series_kind(id, SeriesKind::Line);
    assert!(
        pulse(&mut chart),
        "untouched series adopts the line default"
    );
    chart.convert_series_kind(id, SeriesKind::Candlestick);
    assert!(!pulse(&mut chart), "and drops it again for candles");

    // An explicit opt-in on a static kind survives too.
    assert!(chart.set_series_last_price_animation(id, true));
    chart.convert_series_kind(id, SeriesKind::Bar);
    assert!(pulse(&mut chart), "opt-in survives conversion");

    chart.convert_series_kind(id, SeriesKind::Area);
    assert!(chart.set_series_last_price_animation(id, false));
    for kind in [SeriesKind::Line, SeriesKind::Candlestick, SeriesKind::Area] {
        chart.convert_series_kind(id, kind);
        assert!(
            !pulse(&mut chart),
            "opt-out survives conversion to {kind:?}"
        );
    }
}

#[test]
fn pulse_clock_runs_only_while_the_primary_series_draws_a_pulse() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    // The engine starts with one candlestick main series; the primary series owns the pulse.
    let id = 0;
    chart.convert_series_kind(id, SeriesKind::Line);
    assert!(
        !chart.last_price_pulse_active(),
        "no data, no animation loop"
    );
    chart
        .set_series_data(
            id,
            &[1.0, 2.0],
            &[5.0, 6.0],
            &[5.0, 6.0],
            &[5.0, 6.0],
            &[5.0, 6.0],
        )
        .unwrap();
    assert!(chart.last_price_pulse_active());
    assert!(chart.set_series_last_price_animation(id, false));
    assert!(
        !chart.last_price_pulse_active(),
        "host opt-out stops the loop"
    );
}

#[test]
fn pulse_advances_on_every_clock_tick_without_rebuilding_series_or_chrome() {
    use aeris_charts_render::draw_list::Prim;
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    chart.convert_series_kind(0, SeriesKind::Line);
    install_bars(&mut chart, 20);
    chart.fit_content();
    let ring = |chart: &mut ChartEngine| {
        chart.build_frame().panes[0]
            .main
            .iter()
            .filter_map(|prim| match prim {
                Prim::Circle {
                    radius,
                    stroke_width,
                    ..
                } if *stroke_width > 0.0 => Some(*radius),
                _ => None,
            })
            .next()
    };
    chart.animation_time = 100.0;
    let first = ring(&mut chart).expect("ring visible early in the cycle");
    chart.animation_time = 500.0;
    let second = ring(&mut chart).expect("ring still visible");
    assert!(
        second > first,
        "the ring must grow between ticks ({first} -> {second})"
    );
    // A clock tick rebuilds only the overlay layer, never series geometry.
    let stats = chart.frame_build_stats();
    assert_eq!(stats.series_rebuilds, 0);
    assert_eq!(stats.overlay_rebuilds, 1);
    // Rest phase: only the center point remains.
    chart.animation_time = 2000.0;
    assert_eq!(ring(&mut chart), None);
}

// ---- primary-series removal + series ordering ----

#[test]
fn removing_series_zero_falls_back_to_the_first_live_series() {
    use aeris_charts_render::draw_list::Prim;
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    install_bars(&mut chart, 3);
    chart.series[0].last_price_animation = true;
    let second = chart.add_series(SeriesKind::Line);
    chart
        .set_series_data(
            second,
            &[1.0, 2.0, 3.0],
            &[5.0, 6.0, 7.0],
            &[5.0, 6.0, 7.0],
            &[5.0, 6.0, 7.0],
            &[5.0, 6.0, 7.0],
        )
        .unwrap();
    chart.series_entry_mut(second).unwrap().last_price_animation = true;
    chart.time_scale.set_width(800.0);
    chart.fit_content();

    assert!(chart.remove_series(0));
    assert_eq!(chart.primary_series().map(|s| s.id), Some(second));

    // Crosshair labels still resolve against the remaining series...
    let ts = 2.0;
    assert!(crosshair_time_label(&mut chart, ts).is_some());
    // ...the last-price pulse follows the new primary...
    let frame = chart.build_frame();
    assert!(
        frame.panes[0]
            .main
            .iter()
            .any(|p| matches!(p, Prim::Circle { .. })),
        "pulse falls back to the first visible non-removed series"
    );
    // ...and last-value labels come from the remaining series (close 7).
    let axis = chart.build_axis_frame(
        80.0,
        |t, _bold| t.len() as f64 * 7.0,
        |t, _bold| t.len() as f64 * 6.0,
    );
    assert!(axis
        .labels
        .iter()
        .any(|l| l.background.is_some() && l.text == "7.00"));
}

#[test]
fn series_order_round_trips_and_rejects_bad_permutations() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    let s1 = chart.add_series(SeriesKind::Line);
    let s2 = chart.add_series(SeriesKind::Line);
    assert_eq!(chart.series_order_json(), "[0,1,2]");
    assert!(chart.set_series_order(vec![s2, 0, s1]));
    assert_eq!(chart.series_order_json(), "[2,0,1]");
    // wrong length, duplicate, unknown id: all rejected without state change
    assert!(!chart.set_series_order(vec![0, 1]));
    assert!(!chart.set_series_order(vec![0, 1, 2, 3]));
    assert!(!chart.set_series_order(vec![0, 1, 1]));
    assert!(!chart.set_series_order(vec![0, 1, 42]));
    assert_eq!(chart.series_order_json(), "[2,0,1]");
    // removed series leave the order
    assert!(chart.remove_series(s1));
    assert_eq!(chart.series_order_json(), "[2,0]");
    assert!(chart.set_series_order(vec![0, 2]));
    assert_eq!(chart.series_order_json(), "[0,2]");
}

#[test]
fn series_order_controls_paint_order() {
    use aeris_charts_render::draw_list::Prim;
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    install_bars(&mut chart, 3);
    chart.series[0].kind = SeriesKind::Line;
    chart.series[0].line_color = Some("#ff0000".to_string());
    let second = chart.add_series(SeriesKind::Line);
    chart
        .set_series_data(
            second,
            &[1.0, 2.0, 3.0],
            &[5.0, 6.0, 7.0],
            &[5.0, 6.0, 7.0],
            &[5.0, 6.0, 7.0],
            &[5.0, 6.0, 7.0],
        )
        .unwrap();
    chart.series_entry_mut(second).unwrap().line_color = Some("#0000ff".to_string());
    chart.time_scale.set_width(800.0);
    chart.fit_content();
    let poly_colors = |chart: &mut ChartEngine| {
        chart.build_frame().panes[0]
            .main
            .iter()
            .filter_map(|p| match p {
                Prim::Polyline { color, .. } => Some(color.to_hex()),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    // added order: the second series paints last (on top)
    assert_eq!(poly_colors(&mut chart), ["#ff0000", "#0000ff"]);
    assert!(chart.set_series_order(vec![second, 0]));
    assert_eq!(poly_colors(&mut chart), ["#0000ff", "#ff0000"]);
}

// ---- verbatim CSS color storage (item 2.13b) ----

#[test]
fn color_options_round_trip_verbatim() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    let fields: Vec<(&str, &str)> = vec![
        ("color", "#AaBbCc"),
        ("up_color", "#FF0000"),
        ("down_color", "rgb(1, 2, 3)"),
        ("wick_up_color", "rgba(4,5,6,0.5)"),
        ("wick_down_color", "#111"),
        ("border_up_color", "#222233"),
        ("border_down_color", "rebeccapurple"),
        ("area_top_color", "#12345678"),
        ("area_bottom_color", "rgba(9, 8, 7, 0.25)"),
    ];
    {
        let s = &mut chart.series[0];
        s.line_color = Some("#AaBbCc".to_string());
        s.up_color = Some("#FF0000".to_string());
        s.down_color = Some("rgb(1, 2, 3)".to_string());
        s.wick_up_color = Some("rgba(4,5,6,0.5)".to_string());
        s.wick_down_color = Some("#111".to_string());
        s.border_up_color = Some("#222233".to_string());
        s.border_down_color = Some("rebeccapurple".to_string());
        s.area_top_color = Some("#12345678".to_string());
        s.area_bottom_color = Some("rgba(9, 8, 7, 0.25)".to_string());
    }
    let options: serde_json::Value =
        serde_json::from_str(&chart.series_options_json(0).unwrap()).unwrap();
    for (key, applied) in &fields {
        assert_eq!(&options[key], applied, "verbatim round-trip for {key}");
    }
    // "" clears back to the follow/default state
    chart.series[0].up_color = None;
    let options: serde_json::Value =
        serde_json::from_str(&chart.series_options_json(0).unwrap()).unwrap();
    assert_eq!(options["up_color"], "");
}

#[test]
fn series_apply_options_accepts_the_style_fields_returned_by_options() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    assert!(chart.series_apply_options_json(
        0,
        r##"{
            "color":"#3B82F6",
            "up_color":"#10B981",
            "down_color":"#EF4444",
            "wick_up_color":"#34D399",
            "wick_down_color":"#F87171",
            "border_up_color":"#059669",
            "border_down_color":"#DC2626",
            "wick_visible":false,
            "border_visible":false,
            "line_width":4,
            "area_top_color":"#2563EB",
            "area_bottom_color":"#172554"
        }"##,
    ));

    let options: serde_json::Value =
        serde_json::from_str(&chart.series_options_json(0).unwrap()).unwrap();
    assert_eq!(options["color"], "#3B82F6");
    assert_eq!(options["up_color"], "#10B981");
    assert_eq!(options["down_color"], "#EF4444");
    assert_eq!(options["wick_up_color"], "#34D399");
    assert_eq!(options["wick_down_color"], "#F87171");
    assert_eq!(options["border_up_color"], "#059669");
    assert_eq!(options["border_down_color"], "#DC2626");
    assert_eq!(options["wick_visible"], false);
    assert_eq!(options["border_visible"], false);
    assert_eq!(options["line_width"], 4.0);
    assert_eq!(options["area_top_color"], "#2563EB");
    assert_eq!(options["area_bottom_color"], "#172554");
}

#[test]
fn unparseable_verbatim_colors_fall_back_at_render_time() {
    use aeris_charts_render::draw_list::Prim;
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    install_bars(&mut chart, 3);
    chart.series[0].up_color = Some("rebeccapurple".to_string()); // stored, unparseable
    chart.time_scale.set_width(800.0);
    chart.fit_content();
    // The up bars render with Aeris's canonical default, not the stored string.
    let up = Color::rgb(
        aeris_charts_core::style::DEFAULT_MARKET_UP_RGB.0,
        aeris_charts_core::style::DEFAULT_MARKET_UP_RGB.1,
        aeris_charts_core::style::DEFAULT_MARKET_UP_RGB.2,
    );
    let frame = chart.build_frame();
    assert!(frame.panes[0]
        .main
        .iter()
        .any(|p| matches!(p, Prim::Rect { color, .. } if *color == up)));
    // but options() still returns the applied string verbatim
    let options: serde_json::Value =
        serde_json::from_str(&chart.series_options_json(0).unwrap()).unwrap();
    assert_eq!(options["up_color"], "rebeccapurple");
}

// ---- wave: reference v5 panes API + scale/time cosmetics + background gradient + separator ----
// ---- hover (chart-api.ts/pane-api.ts, price/time-scale options, pane-separator.ts)    ----

#[test]
fn panes_add_remove_swap_move_and_series_movement() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    install_bars(&mut chart, 5);
    let second = chart.add_series(SeriesKind::Line);
    let third = chart.add_series(SeriesKind::Histogram);

    // addPane appends and reports the new index (reference chart-api.ts addPane).
    let pane1 = chart.add_pane(false).unwrap();
    assert_eq!(pane1, 1);
    let pane2 = chart.add_pane(true).unwrap();
    assert_eq!(pane2, 2);
    assert!(chart.pane_preserve_empty(pane2));
    assert!(!chart.pane_preserve_empty(pane1));

    chart.set_series_pane(second, 1, 1.0);
    chart.set_series_pane(third, 2, 1.0);
    assert_eq!(chart.pane_series_ids(0), vec![0]);
    assert_eq!(chart.pane_series_ids(1), vec![second]);
    assert_eq!(chart.pane_series_ids(2), vec![third]);
    // Render order within a pane: bottom first (reference pane.ts orderedSources).
    let fourth = chart.add_series(SeriesKind::Area);
    chart.set_series_pane(fourth, 1, 1.0);
    assert_eq!(chart.pane_series_ids(1), vec![second, fourth]);

    // swapPanes: the panes trade places with their series assignments and stretch factors.
    chart.panes[1].stretch_factor = 2.0;
    assert!(chart.swap_panes(1, 2));
    assert_eq!(chart.pane_series_ids(1), vec![third]);
    assert_eq!(chart.pane_series_ids(2), vec![second, fourth]);
    assert_eq!(chart.panes[2].stretch_factor, 2.0);
    assert!(chart.pane_preserve_empty(1), "preserve flag rides along");
    assert!(!chart.swap_panes(1, 7), "stale index rejected");

    // movePane (pane-api.ts moveTo): the pane rides to its new index with its series.
    assert!(chart.move_pane(2, 0));
    assert_eq!(chart.pane_series_ids(0), vec![second, fourth]);
    assert_eq!(chart.pane_series_ids(1), vec![0]);
    assert_eq!(chart.pane_series_ids(2), vec![third]);
    assert!(chart.move_pane(0, 0), "same-index move is a no-op success");
    assert!(!chart.move_pane(0, 9), "stale target rejected");

    // removePane orphans the pane's series (reference paneForSource -> null): they keep their
    // data but render/scale nowhere; panes below shift one index up.
    assert!(chart.remove_pane(0));
    assert_eq!(chart.panes.len(), 2);
    assert_eq!(chart.series_entry(second).unwrap().pane_index, PANELESS);
    assert_eq!(chart.series_entry(fourth).unwrap().pane_index, PANELESS);
    assert_eq!(chart.pane_series_ids(0), vec![0]);
    assert_eq!(chart.pane_series_ids(1), vec![third]);
    // A pane-less series re-assigned to a live pane renders again (ids in z-order, not
    // assignment order — reference pane.ts orderedSources).
    chart.set_series_pane(second, 1, 1.0);
    assert_eq!(chart.pane_series_ids(1), vec![second, third]);
    assert!(!chart.remove_pane(9), "stale index rejected");
    chart.remove_pane(0);
    assert!(
        !chart.remove_pane(0),
        "the last remaining pane cannot be removed"
    );
    assert_eq!(chart.panes.len(), 1);
}

#[test]
fn pane_identity_survives_moves_and_never_retargets_after_removal() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    let original = chart.pane_stable_id(0).unwrap();
    let added_index = chart.add_pane(true).unwrap();
    let added = chart.pane_stable_id(added_index).unwrap();

    assert!(chart.move_pane(added_index, 0));
    assert_eq!(chart.pane_index_for_id(added), Some(0));
    assert_eq!(chart.pane_index_for_id(original), Some(1));

    assert!(chart.remove_pane(0));
    assert_eq!(chart.pane_index_for_id(added), None);
    let reused_index = chart.add_pane(true).unwrap();
    assert_eq!(reused_index, 1);
    assert_ne!(chart.pane_stable_id(reused_index), Some(added));
    assert_eq!(chart.pane_index_for_id(added), None);
}

#[test]
fn empty_preserved_last_pane_retires_into_a_fresh_default_pane() {
    let mut chart = ChartEngine::new_with_initial_domain(
        800.0,
        500.0,
        1.0,
        HorizontalDomain::Category {
            scale: CategoryScaleType::Band,
        },
    )
    .unwrap();
    let retired = chart.pane_stable_id(0).unwrap();
    chart
        .add_general_axis(GeneralAxisOptions::new(
            "retired-x",
            0,
            AxisDimension::X,
            GeneralScaleType::Band,
        ))
        .unwrap();

    assert!(chart.remove_pane(0));
    assert_eq!(chart.panes.len(), 1);
    assert_eq!(chart.pane_index_for_id(retired), None);
    assert_ne!(chart.pane_stable_id(0), Some(retired));
    assert_eq!(
        chart.pane_horizontal_domain(0),
        Some(HorizontalDomain::FinancialTime)
    );
    assert!(chart.general_axis("retired-x").is_none());
    assert_eq!(chart.general_horizontal_domains.len(), 0);
    assert!(!chart.pane_preserve_empty(0));
    assert!(!chart.remove_pane(0), "the replacement is not caller-owned");
}

#[test]
fn pane_domains_default_to_financial_time_and_follow_pane_identity() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    assert_eq!(
        chart.pane_horizontal_domain(0),
        Some(HorizontalDomain::FinancialTime)
    );
    assert_eq!(chart.memory_usage().general_domain_capacity_bytes, 0);

    let legacy = chart.add_pane(true).unwrap();
    assert_eq!(
        chart.pane_horizontal_domain(legacy),
        Some(HorizontalDomain::FinancialTime)
    );
    assert_eq!(chart.memory_usage().general_domain_capacity_bytes, 0);

    let category = chart
        .add_pane_with_domain(
            true,
            HorizontalDomain::Category {
                scale: CategoryScaleType::Band,
            },
        )
        .unwrap();
    let category_id = chart.pane_stable_id(category).unwrap();
    assert_eq!(
        chart.pane_horizontal_domain(category),
        Some(HorizontalDomain::Category {
            scale: CategoryScaleType::Band
        })
    );
    assert_eq!(chart.general_horizontal_domains.len(), 1);
    assert!(chart.memory_usage().general_domain_capacity_bytes > 0);

    assert!(chart.move_pane(category, 0));
    assert_eq!(chart.pane_index_for_id(category_id), Some(0));
    assert_eq!(
        chart.pane_horizontal_domain(0),
        Some(HorizontalDomain::Category {
            scale: CategoryScaleType::Band
        })
    );
    assert!(chart.swap_panes(0, 1));
    assert_eq!(chart.pane_index_for_id(category_id), Some(1));
    assert_eq!(
        chart.pane_horizontal_domain(1),
        Some(HorizontalDomain::Category {
            scale: CategoryScaleType::Band
        })
    );

    assert!(chart.remove_pane(1));
    assert_eq!(chart.pane_index_for_id(category_id), None);
    assert_eq!(chart.general_horizontal_domains.len(), 0);
    assert_eq!(chart.pane_horizontal_domain(99), None);
}

#[test]
fn initial_general_domain_has_one_preserved_pane_and_no_financial_series() {
    let chart = ChartEngine::new_with_initial_domain(
        800.0,
        500.0,
        1.0,
        HorizontalDomain::Category {
            scale: CategoryScaleType::Point,
        },
    )
    .unwrap();

    assert_eq!(chart.panes.len(), 1);
    assert!(chart.panes[0].preserve_empty);
    assert_eq!(
        chart.pane_horizontal_domain(0),
        Some(HorizontalDomain::Category {
            scale: CategoryScaleType::Point
        })
    );
    assert!(chart.series_entries().is_empty());
    assert!(chart.series_order.is_empty());
    assert_eq!(chart.data_layer().series_count(), 0);
    assert_eq!(chart.general_horizontal_domains.len(), 1);
}

#[test]
fn general_data_is_lazy_atomic_and_releases_capacity_when_empty() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    assert_eq!(chart.general_dataset_count(), 0);
    assert_eq!(chart.memory_usage().general_data_capacity_bytes, 0);

    let err = chart
        .create_general_xy_dataset(GeneralXyInput::Numeric {
            ids: None,
            x: vec![f64::NAN],
            y: vec![1.0],
            y_valid: None,
        })
        .unwrap_err();
    assert_eq!(err.code(), ErrorCode::InvalidData);
    assert_eq!(chart.general_dataset_count(), 0);
    assert_eq!(chart.memory_usage().general_data_capacity_bytes, 0);

    let id = chart
        .create_general_xy_dataset(GeneralXyInput::Category {
            ids: None,
            categories: vec!["Jan".into(), "Feb".into()],
            category_indices: vec![0, 1],
            y: vec![42.0, 57.0],
            y_valid: None,
        })
        .unwrap();
    assert_eq!(chart.general_dataset_count(), 1);
    assert!(chart.memory_usage().general_data_capacity_bytes > 0);
    assert_eq!(
        chart.general_dataset(id).unwrap().categories().unwrap(),
        &["Jan", "Feb"]
    );

    assert!(chart.remove_general_dataset(id));
    assert_eq!(chart.general_dataset_count(), 0);
    assert_eq!(chart.memory_usage().general_data_capacity_bytes, 0);
    assert!(chart.general_dataset(id).is_none());
}

#[test]
fn every_general_domain_variant_is_explicit_and_financial_series_cannot_enter() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    let main_series = chart.series_entries()[0].id;
    let domains = [
        HorizontalDomain::Continuous {
            scale: ContinuousScaleType::Linear,
        },
        HorizontalDomain::Continuous {
            scale: ContinuousScaleType::Logarithmic,
        },
        HorizontalDomain::Continuous {
            scale: ContinuousScaleType::SymmetricLog,
        },
        HorizontalDomain::Temporal,
        HorizontalDomain::Category {
            scale: CategoryScaleType::Band,
        },
        HorizontalDomain::Category {
            scale: CategoryScaleType::Point,
        },
        HorizontalDomain::Polar,
    ];

    for domain in domains {
        let pane = chart.add_pane_with_domain(true, domain).unwrap();
        assert_eq!(chart.pane_horizontal_domain(pane), Some(domain));
        assert!(!chart.try_set_series_pane(main_series, pane, 1.0));
        assert_eq!(chart.series_entry(main_series).unwrap().pane_index, 0);
        assert!(chart.pane_series_ids(pane).is_empty());
        assert_eq!(
            chart.add_drawing(
                DrawingKind::Text,
                pane,
                vec![DrawingPoint {
                    logical: 1.0,
                    price: 2.0,
                }],
                None,
            ),
            None
        );
    }
}

#[test]
fn general_domain_capacity_failure_is_atomic_and_removal_releases_a_slot() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    for _ in 0..MAX_GENERAL_HORIZONTAL_DOMAINS {
        chart
            .add_pane_with_domain(true, HorizontalDomain::Temporal)
            .unwrap();
    }
    let pane_count = chart.panes.len();
    let next_pane_id = chart.next_pane_id;
    let next_persistent_pane_id = chart.next_persistent_pane_id;
    let error = chart
        .add_pane_with_domain(true, HorizontalDomain::Polar)
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::ResourceLimit);
    assert_eq!(chart.panes.len(), pane_count);
    assert_eq!(chart.next_pane_id, next_pane_id);
    assert_eq!(chart.next_persistent_pane_id, next_persistent_pane_id);

    assert!(chart.remove_pane(1));
    let replacement = chart
        .add_pane_with_domain(true, HorizontalDomain::Polar)
        .unwrap();
    assert_eq!(
        chart.pane_horizontal_domain(replacement),
        Some(HorizontalDomain::Polar)
    );
}

#[test]
fn persistence_upgrades_general_panes_to_v2_without_reinterpreting_them() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    chart
        .add_pane_with_domain(true, HorizontalDomain::Temporal)
        .unwrap();
    let document = chart.export_state_json().unwrap();
    let value: serde_json::Value = serde_json::from_str(&document).unwrap();
    assert_eq!(value["schema_version"], 2);
    assert_eq!(value["panes"][0]["horizontal_domain"], "FinancialTime");
    assert_eq!(value["panes"][1]["horizontal_domain"], "Temporal");
}

#[test]
fn general_axes_are_validated_owned_and_follow_their_pane() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    assert_eq!(chart.memory_usage().general_axis_bytes, 0);
    let financial_error = chart
        .add_general_axis(GeneralAxisOptions::new(
            "financial-y",
            0,
            AxisDimension::Y,
            GeneralScaleType::Linear,
        ))
        .unwrap_err();
    assert_eq!(financial_error.code(), ErrorCode::InvalidOptions);
    assert_eq!(chart.memory_usage().general_axis_bytes, 0);

    let pane = chart
        .add_pane_with_domain(
            true,
            HorizontalDomain::Category {
                scale: CategoryScaleType::Band,
            },
        )
        .unwrap();
    let mut x =
        GeneralAxisOptions::new("category-x", pane, AxisDimension::X, GeneralScaleType::Band);
    x.domain = GeneralAxisDomain::Category(vec!["Jan".into(), "Feb".into()]);
    x.title = Some("Month".into());
    chart.add_general_axis(x).unwrap();
    chart
        .add_general_axis(GeneralAxisOptions::new(
            "value-y",
            pane,
            AxisDimension::Y,
            GeneralScaleType::Linear,
        ))
        .unwrap();

    let x_handle = chart.general_axis("category-x").unwrap().handle();
    let x_axis = chart.general_axis("category-x").unwrap();
    assert_eq!(x_axis.position(), Some(AxisPosition::Bottom));
    assert_eq!(x_axis.title(), Some("Month"));
    assert_eq!(chart.general_axis_pane_index("category-x"), Some(pane));
    assert_eq!(chart.general_axes(Some(pane)).len(), 2);
    assert_eq!(chart.general_axes(None).len(), 2);
    assert!(chart.memory_usage().general_axis_bytes > 0);

    assert!(chart.move_pane(pane, 0));
    assert_eq!(chart.general_axis_pane_index("category-x"), Some(0));
    assert_eq!(chart.general_axes(Some(0)).len(), 2);

    assert!(chart.remove_general_axis("category-x"));
    assert!(chart.general_axis("category-x").is_none());
    let mut replacement =
        GeneralAxisOptions::new("category-x", 0, AxisDimension::X, GeneralScaleType::Band);
    replacement.domain = GeneralAxisDomain::Category(vec!["Mar".into()]);
    chart.add_general_axis(replacement).unwrap();
    assert_ne!(chart.general_axis("category-x").unwrap().handle(), x_handle);
}

#[test]
fn general_axis_failures_are_atomic_and_pane_removal_releases_axes() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    let pane = chart
        .add_pane_with_domain(true, HorizontalDomain::Temporal)
        .unwrap();
    chart
        .add_general_axis(GeneralAxisOptions::new(
            "time-x",
            pane,
            AxisDimension::X,
            GeneralScaleType::Temporal,
        ))
        .unwrap();

    let mut duplicate =
        GeneralAxisOptions::new("time-x", pane, AxisDimension::Y, GeneralScaleType::Linear);
    duplicate.domain = GeneralAxisDomain::Numeric([10.0, 0.0]);
    assert_eq!(
        chart.add_general_axis(duplicate).unwrap_err().code(),
        ErrorCode::InvalidOptions
    );
    assert_eq!(chart.general_axes(None).len(), 1);

    let mismatch =
        GeneralAxisOptions::new("bad-x", pane, AxisDimension::X, GeneralScaleType::Linear);
    assert_eq!(
        chart.add_general_axis(mismatch).unwrap_err().code(),
        ErrorCode::InvalidOptions
    );
    assert_eq!(chart.general_axes(None).len(), 1);

    let handle = chart.general_axis("time-x").unwrap().handle();
    let mut update =
        GeneralAxisOptions::new("time-x", pane, AxisDimension::X, GeneralScaleType::Temporal);
    update.title = Some("Updated time".into());
    update.reverse = true;
    chart.update_general_axis_options(update).unwrap();
    let updated = chart.general_axis("time-x").unwrap();
    assert_eq!(updated.handle(), handle);
    assert_eq!(updated.title(), Some("Updated time"));
    assert!(updated.reverse());

    let mut rejected =
        GeneralAxisOptions::new("time-x", pane, AxisDimension::X, GeneralScaleType::Temporal);
    rejected.tick_count = Some(0);
    assert_eq!(
        chart
            .update_general_axis_options(rejected)
            .unwrap_err()
            .code(),
        ErrorCode::InvalidOptions
    );
    let unchanged = chart.general_axis("time-x").unwrap();
    assert_eq!(unchanged.handle(), handle);
    assert_eq!(unchanged.title(), Some("Updated time"));
    assert!(unchanged.reverse());

    assert!(chart.remove_pane(pane));
    assert_eq!(chart.general_axes.len(), 0);
    assert!(chart.general_axis("time-x").is_none());
}

#[test]
fn general_axis_count_is_bounded_without_mutating_on_overflow() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    let pane = chart
        .add_pane_with_domain(
            true,
            HorizontalDomain::Continuous {
                scale: ContinuousScaleType::Linear,
            },
        )
        .unwrap();
    for index in 0..MAX_GENERAL_AXES {
        chart
            .add_general_axis(GeneralAxisOptions::new(
                format!("y-{index}"),
                pane,
                AxisDimension::Y,
                GeneralScaleType::Linear,
            ))
            .unwrap();
    }
    let error = chart
        .add_general_axis(GeneralAxisOptions::new(
            "overflow",
            pane,
            AxisDimension::Y,
            GeneralScaleType::Linear,
        ))
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::ResourceLimit);
    assert_eq!(chart.general_axes(None).len(), MAX_GENERAL_AXES);
}

#[test]
fn pane_identity_exhaustion_is_recoverable_and_does_not_mutate_topology() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    chart.next_pane_id = u32::MAX;
    let before = chart.panes.len();
    assert_eq!(chart.add_pane(true), None);
    assert_eq!(chart.panes.len(), before);
}

#[test]
fn named_price_scales_enforce_identity_order_limits_and_removal_contracts() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    install_bars(&mut chart, 5);

    let inner = chart
        .add_price_scale(0, "inner", PriceScaleSide::Right, Some(0), true)
        .unwrap();
    let outer = chart
        .add_price_scale(0, "Outer", PriceScaleSide::Right, None, true)
        .unwrap();
    let left = chart
        .add_price_scale(0, "left-comparison", PriceScaleSide::Left, Some(0), true)
        .unwrap();
    let scales = chart.price_scales(0).unwrap();
    let info = |id: &str| scales.iter().find(|info| info.id == id).unwrap();
    assert_eq!(
        (info("inner").side, info("inner").order),
        (Some(PriceScaleSide::Right), Some(0))
    );
    assert_eq!(
        (info("right").side, info("right").order),
        (Some(PriceScaleSide::Right), Some(1))
    );
    assert_eq!(
        (info("Outer").side, info("Outer").order),
        (Some(PriceScaleSide::Right), Some(2))
    );
    assert_eq!(
        (info("left-comparison").side, info("left-comparison").order),
        (Some(PriceScaleSide::Left), Some(0))
    );
    assert_eq!(
        (info("left").side, info("left").order),
        (Some(PriceScaleSide::Left), Some(1))
    );
    assert_ne!(inner, outer);
    assert_ne!(outer, left);

    assert!(chart.move_price_scale(0, outer, PriceScaleSide::Left, 0));
    assert!(chart.move_price_scale(0, outer, PriceScaleSide::Left, 0));
    let scales = chart.price_scales(0).unwrap();
    let left_ids: Vec<_> = scales
        .iter()
        .filter(|info| info.side == Some(PriceScaleSide::Left))
        .map(|info| (info.id.as_str(), info.order.unwrap()))
        .collect();
    assert_eq!(
        left_ids,
        vec![("Outer", 0), ("left-comparison", 1), ("left", 2)]
    );

    for invalid in ["", "left", "right", "inner"] {
        let before = chart.price_scales(0).unwrap().len();
        let error = chart
            .add_price_scale(0, invalid, PriceScaleSide::Right, None, true)
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::InvalidOptions);
        assert_eq!(chart.price_scales(0).unwrap().len(), before);
    }
    let overlong = "é".repeat(65);
    assert_eq!(
        chart
            .add_price_scale(0, &overlong, PriceScaleSide::Right, None, true)
            .unwrap_err()
            .code(),
        ErrorCode::InvalidOptions
    );
    assert_eq!(
        chart
            .add_price_scale(99, "missing-pane", PriceScaleSide::Right, None, true)
            .unwrap_err()
            .code(),
        ErrorCode::InvalidHandle
    );

    chart.set_series_price_scale(0, inner);
    assert_eq!(
        chart.remove_price_scale(0, inner).unwrap_err().code(),
        ErrorCode::UnsupportedOperation
    );
    assert_eq!(
        chart
            .remove_price_scale(0, PriceScaleTarget::Right)
            .unwrap_err()
            .code(),
        ErrorCode::UnsupportedOperation
    );
    chart.set_series_price_scale(0, PriceScaleTarget::Right);
    chart.remove_price_scale(0, inner).unwrap();
    assert!(chart.price_scale_target_for_id(0, "inner").is_none());
    assert!(chart.price_scale_for(0, inner).is_none());
    let replacement = chart
        .add_price_scale(0, "inner", PriceScaleSide::Right, None, false)
        .unwrap();
    assert_ne!(
        replacement, inner,
        "removed scale identities are never reused"
    );
    assert!(!chart.price_scale_visible_for(0, replacement));

    let mut capped = ChartEngine::new(800.0, 500.0, 1.0);
    for index in 0..MAX_NAMED_PRICE_SCALES_PER_PANE {
        capped
            .add_price_scale(
                0,
                &format!("scale-{index}"),
                PriceScaleSide::Right,
                None,
                true,
            )
            .unwrap();
    }
    assert_eq!(
        capped
            .add_price_scale(0, "overflow", PriceScaleSide::Right, None, true)
            .unwrap_err()
            .code(),
        ErrorCode::ResourceLimit
    );
}

#[test]
fn empty_named_scales_survive_automatic_pane_cleanup() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    install_bars(&mut chart, 5);
    let named = chart
        .add_price_scale(0, "host-owned", PriceScaleSide::Right, None, true)
        .unwrap();
    let destination = chart.add_pane(true).unwrap();

    assert!(chart.try_set_series_pane(0, destination, 1.0));
    assert_eq!(chart.panes.len(), 2);
    assert_eq!(
        chart.price_scale_target_for_id(0, "host-owned"),
        Some(named)
    );
    assert!(chart
        .price_scales(0)
        .unwrap()
        .iter()
        .any(|info| { info.id == "host-owned" && info.series_ids.is_empty() }));
}

#[test]
fn named_scale_series_rebinding_and_pane_moves_are_atomic() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    install_bars(&mut chart, 5);
    let comparison = chart.add_series(SeriesKind::Area);
    chart
        .set_series_data(
            comparison,
            &[1.0, 2.0, 3.0],
            &[1_000.0, 1_010.0, 1_020.0],
            &[1_000.0, 1_010.0, 1_020.0],
            &[1_000.0, 1_010.0, 1_020.0],
            &[1_000.0, 1_010.0, 1_020.0],
        )
        .unwrap();
    let source = chart
        .add_price_scale(0, "comparison", PriceScaleSide::Right, Some(0), true)
        .unwrap();
    chart.set_series_price_scale(comparison, source);
    let before = chart.series_data(comparison);

    let destination_pane = chart.add_pane(true).unwrap();
    assert!(!chart.try_set_series_pane(comparison, destination_pane, 1.0));
    assert_eq!(chart.series_price_scale(comparison), Some((0, source)));
    assert!(!chart.try_set_series_pane_and_scale(comparison, 0, 1.0, "missing"));
    assert_eq!(chart.series_price_scale(comparison), Some((0, source)));
    assert_eq!(chart.series_data(comparison), before);
    assert_eq!(chart.series_kind(comparison), Some(SeriesKind::Area));

    let destination = chart
        .add_price_scale(
            destination_pane,
            "comparison",
            PriceScaleSide::Left,
            Some(0),
            true,
        )
        .unwrap();
    assert!(chart.try_set_series_pane(comparison, destination_pane, 2.0));
    assert_eq!(
        chart.series_price_scale(comparison),
        Some((destination_pane, destination))
    );
    assert_eq!(chart.series_data(comparison), before);
    assert_eq!(chart.series_kind(comparison), Some(SeriesKind::Area));

    assert!(chart.try_set_series_pane_and_scale(comparison, 0, 1.0, "right"));
    assert_eq!(
        chart.series_price_scale(comparison),
        Some((0, PriceScaleTarget::Right))
    );
}

#[test]
fn named_scales_autoscale_independently_and_shared_percentage_series_use_own_bases() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    chart
        .set_series_data(
            0,
            &[1.0, 2.0],
            &[100.0, 110.0],
            &[100.0, 110.0],
            &[100.0, 110.0],
            &[100.0, 110.0],
        )
        .unwrap();
    let first = chart.add_series(SeriesKind::Line);
    let second = chart.add_series(SeriesKind::Line);
    chart
        .set_series_data(
            first,
            &[1.0, 2.0],
            &[1_000.0, 1_100.0],
            &[1_000.0, 1_100.0],
            &[1_000.0, 1_100.0],
            &[1_000.0, 1_100.0],
        )
        .unwrap();
    chart
        .set_series_data(
            second,
            &[1.0, 2.0],
            &[2_000.0, 2_200.0],
            &[2_000.0, 2_200.0],
            &[2_000.0, 2_200.0],
            &[2_000.0, 2_200.0],
        )
        .unwrap();
    let comparison = chart
        .add_price_scale(0, "percentage", PriceScaleSide::Right, Some(0), true)
        .unwrap();
    chart.set_series_price_scale(first, comparison);
    chart.set_series_price_scale(second, comparison);
    chart.set_price_scale_mode_for(0, comparison, PriceScaleMode::Percentage);
    chart.time_scale.set_width(800.0);
    chart.fit_content();
    chart.build_frame();

    let right_range = chart
        .price_scale_visible_range_for(0, PriceScaleTarget::Right)
        .unwrap();
    let comparison_range = chart.price_scale_visible_range_for(0, comparison).unwrap();
    assert!(right_range.1 < 200.0);
    assert!(comparison_range.0 <= 0.0 && comparison_range.1 >= 10.0);
    let first_y = chart.series_price_to_coordinate(first, 1_100.0).unwrap();
    let second_y = chart.series_price_to_coordinate(second, 2_200.0).unwrap();
    assert!((first_y - second_y).abs() < 1e-9);

    chart.set_price_scale_visible_range_for(0, comparison, -5.0, 25.0);
    let right_before = chart
        .price_scale_visible_range_for(0, PriceScaleTarget::Right)
        .unwrap();
    chart.price_axis_start_scroll(0, comparison, 100.0);
    chart.price_axis_scroll_to(0, comparison, 120.0);
    chart.price_axis_end_scroll(0, comparison);
    assert_ne!(
        chart.price_scale_visible_range_for(0, comparison),
        Some((-5.0, 25.0))
    );
    assert_eq!(
        chart.price_scale_visible_range_for(0, PriceScaleTarget::Right),
        Some(right_before)
    );
}

#[test]
fn ordinary_public_handle_misuse_is_recoverable_not_a_panic() {
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        assert!(PaneId::try_from(0).is_err());
        assert!(!chart.remove_series(u32::MAX));
        assert!(!chart.remove_drawing(u32::MAX));
        assert!(!chart.remove_pane(usize::MAX));
        assert!(!chart.move_pane(usize::MAX, 0));
        assert!(!chart.swap_panes(0, usize::MAX));
        assert!(chart.series_data(u32::MAX).is_empty());
        assert!(chart.series_options_json(u32::MAX).is_none());
        assert!(chart.drawing_options_json(u32::MAX).is_none());
    }));
    assert!(outcome.is_ok());
}

#[test]
fn invalid_series_cannot_create_panes() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    chart.set_series_pane(u32::MAX, 10_000, 1.0);
    assert_eq!(chart.panes.len(), 1);
}

#[test]
fn preserve_empty_pruning_on_series_removal_and_move_out() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    install_bars(&mut chart, 5);
    let second = chart.add_series(SeriesKind::Line);
    chart.set_series_pane(second, 1, 1.0);
    assert_eq!(chart.panes.len(), 2);

    // Moving the series back collapses the emptied, non-preserved pane (reference
    // chart-model.ts `_cleanupIfPaneIsEmpty` on moveSeriesToPane).
    chart.set_series_pane(second, 0, 1.0);
    assert_eq!(chart.panes.len(), 1, "empty non-preserved pane collapses");

    // A preserved pane survives both the move-out and a series removal.
    chart.set_series_pane(second, 1, 1.0);
    chart.pane_set_preserve_empty(1, true);
    chart.set_series_pane(second, 0, 1.0);
    assert_eq!(chart.panes.len(), 2, "preserved empty pane stays");
    chart.set_series_pane(second, 1, 1.0);
    chart.pane_set_preserve_empty(1, false);
    assert!(chart.remove_series(second));
    assert_eq!(
        chart.panes.len(),
        1,
        "removal prunes the unpreserved empty pane"
    );
}

#[test]
fn price_scale_apply_options_json_round_trip_and_chart_group_routing() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    install_bars(&mut chart, 5);

    // Per-scale JSON patch (snake_case): the five cosmetics plus the scale-math keys.
    assert!(chart.price_scale_apply_options_json(
        0,
        PriceScaleTarget::Right,
        r##"{"mode":1,"auto_scale":false,"invert_scale":true,"scale_margins":{"top":0.3,"bottom":0.2},"align_labels":false,"ticks_visible":true,"entire_text_only":true,"minimum_width":80,"text_color":"#ff0000"}"##,
    ));
    let options: serde_json::Value = serde_json::from_str(
        &chart
            .price_scale_options_json(0, PriceScaleTarget::Right)
            .unwrap(),
    )
    .unwrap();
    assert_eq!(options["mode"], 1);
    assert_eq!(options["auto_scale"], false);
    assert_eq!(options["invert_scale"], true);
    assert_eq!(options["scale_margins"]["top"], 0.3);
    assert_eq!(options["scale_margins"]["bottom"], 0.2);
    assert_eq!(options["align_labels"], false);
    assert_eq!(options["ticks_visible"], true);
    assert_eq!(options["entire_text_only"], true);
    assert_eq!(options["minimum_width"], 80.0);
    assert_eq!(options["text_color"], "#ff0000");

    // reference defaults on an untouched scale; a stale pane answers None/false.
    let defaults: serde_json::Value = serde_json::from_str(
        &chart
            .price_scale_options_json(0, PriceScaleTarget::Left)
            .unwrap(),
    )
    .unwrap();
    assert_eq!(defaults["align_labels"], true);
    assert_eq!(defaults["ticks_visible"], false);
    assert_eq!(defaults["entire_text_only"], false);
    assert_eq!(defaults["minimum_width"], 0.0);
    assert_eq!(defaults["text_color"], serde_json::Value::Null);
    assert!(chart
        .price_scale_options_json(9, PriceScaleTarget::Right)
        .is_none());
    assert!(!chart.price_scale_apply_options_json(
        9,
        PriceScaleTarget::Right,
        r#"{"ticks_visible":true}"#
    ));
    assert!(!chart.price_scale_apply_options_json(0, PriceScaleTarget::Right, "{ nope"));

    // `text_color: null` / `""` clears back to following `layout.textColor`.
    assert!(chart.price_scale_apply_options_json(
        0,
        PriceScaleTarget::Right,
        r#"{"text_color":null}"#
    ));
    let cleared: serde_json::Value = serde_json::from_str(
        &chart
            .price_scale_options_json(0, PriceScaleTarget::Right)
            .unwrap(),
    )
    .unwrap();
    assert_eq!(cleared["text_color"], serde_json::Value::Null);

    // Chart-group routing (reference pane.ts applyScaleOptions): a `rightPriceScale` patch
    // applies the five camelCase keys to every pane's right scale, present keys only.
    let extra = chart.add_series(SeriesKind::Line);
    chart.set_series_pane(extra, 1, 1.0);
    chart
        .apply_options(
            r##"{"rightPriceScale":{"ticksVisible":true,"minimumWidth":64,"textColor":"#00ff00"},"leftPriceScale":{"alignLabels":false}}"##,
        )
        .unwrap();
    for pane in &chart.panes {
        assert!(pane.price_scale.options().ticks_visible);
        assert_eq!(pane.price_scale.options().minimum_width, 64.0);
        assert_eq!(
            pane.price_scale.options().text_color.as_deref(),
            Some("#00ff00")
        );
        assert!(!pane.left_scale.options().align_labels);
    }
    // A pane added afterwards inherits the merged chart-level cosmetics (reference Pane
    // constructor `_createPriceScale` from the chart options).
    let pane_index = chart.add_pane(false).unwrap();
    assert!(chart.panes[pane_index].price_scale.options().ticks_visible);
    assert!(!chart.panes[pane_index].left_scale.options().align_labels);
}

#[test]
fn time_axis_options_height_floor_visibility_collapse_and_char_length() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    install_bars(&mut chart, 200);

    // reference chart-widget.ts `Math.max(optimalHeight(), minimumHeight)`: the auto 22px
    // strip (axis 11 + 1 border + 3 tick + 3 + 3 padding, even-snapped) is floored at
    // `minimumHeight`; `visible:false` collapses it to zero.
    assert_eq!(chart.time_axis_height(), 22.0);
    chart.set_time_axis_minimum_height(40.0);
    assert_eq!(chart.time_axis_height(), 40.0);
    chart.set_time_axis_minimum_height(10.0);
    assert_eq!(
        chart.time_axis_height(),
        22.0,
        "floor never shrinks the auto height"
    );
    chart.set_time_axis_visible(false);
    assert_eq!(
        chart.time_axis_height(),
        0.0,
        "hidden strip reserves nothing"
    );
    chart.set_time_axis_visible(true);
    assert_eq!(chart.time_axis_height(), 22.0);
    // Invalid heights are ignored (NaN / negative keep the current value).
    chart.set_time_axis_minimum_height(f64::NAN);
    chart.set_time_axis_minimum_height(-5.0);
    assert_eq!(chart.time_axis_height(), 22.0);

    // `timeVisible` stays label semantics only: it never reserves the strip.
    chart.set_time_visible(false);
    assert_eq!(chart.time_axis_height(), 22.0);
    chart.set_time_visible(true);

    // tickMarkMaxCharacterLength widens/narrows the mark spacing; 0 restores the
    // default 8 (reference time-scale.ts `|| defaultTickMarkMaxCharacterLength`).
    let marks = |chart: &mut ChartEngine| {
        let width = (chart.axis_font_size() + 4.0) * 5.0 / 8.0
            * f64::from(chart.tick_mark_max_character_length);
        chart.time_marks(width).len()
    };
    let default_count = marks(&mut chart);
    chart.set_tick_mark_max_character_length(2);
    let narrow_count = marks(&mut chart);
    assert!(
        narrow_count > default_count,
        "shorter cap packs marks denser ({narrow_count} vs {default_count})"
    );
    chart.set_tick_mark_max_character_length(16);
    assert!(
        marks(&mut chart) < default_count,
        "wider cap thins marks out"
    );
    chart.set_tick_mark_max_character_length(0);
    assert_eq!(chart.tick_mark_max_character_length, 8);
    assert_eq!(marks(&mut chart), default_count);

    // All four route through the chart-options `timeScale` group and round-trip.
    chart
        .apply_options(
            r#"{"timeScale":{"visible":false,"ticksVisible":true,"minimumHeight":32,"tickMarkMaxCharacterLength":5}}"#,
        )
        .unwrap();
    assert!(!chart.time_axis_visible);
    assert!(chart.time_ticks_visible);
    assert_eq!(chart.time_axis_minimum_height, 32.0);
    assert_eq!(chart.tick_mark_max_character_length, 5);
    let options: serde_json::Value =
        serde_json::from_str(&chart.time_scale_options_json()).unwrap();
    assert_eq!(options["visible"], false);
    assert_eq!(options["ticks_visible"], true);
    assert_eq!(options["minimum_height"], 32.0);
    assert_eq!(options["tick_mark_max_character_length"], 5);
    assert_eq!(chart.time_axis_height(), 0.0, "hidden wins over the floor");

    // Tick stubs reach the axis frame only while the strip is visible and ticks on.
    chart.layout_panes(chart.css_height - chart.time_axis_height());
    chart.time_scale.set_width(800.0);
    let frame = chart.build_axis_frame(
        80.0,
        |text, _bold| text.len() as f64 * 6.0,
        |text, _bold| text.len() as f64 * 5.0,
    );
    assert!(
        frame.time_ticks.is_empty(),
        "hidden strip paints no tick stubs"
    );
    chart.set_time_axis_visible(true);
    let frame = chart.build_axis_frame(
        80.0,
        |text, _bold| text.len() as f64 * 6.0,
        |text, _bold| text.len() as f64 * 5.0,
    );
    assert!(!frame.time_ticks.is_empty(), "ticksVisible paints stubs");
}

#[test]
fn background_vertical_gradient_emits_a_per_pane_prim_solid_emits_none() {
    use aeris_charts_render::draw_list::Prim;
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    install_bars(&mut chart, 5);
    chart.time_scale.set_width(800.0);
    chart.fit_content();

    // Solid background (the default): no Background prim — the backends' clear color
    // covers it.
    let frame = chart.build_frame();
    assert!(!frame.panes[0]
        .under
        .iter()
        .any(|p| matches!(p, Prim::Background { .. })));

    // reference VerticalGradient: one prim per pane spanning that pane's full bitmap rect,
    // first in the under layer (behind the grid).
    chart
        .apply_options(
            r##"{"layout":{"background":{"type":"vertical_gradient","topColor":"#ff0000","bottomColor":"#0000ff"}}}"##,
        )
        .unwrap();
    let extra = chart.add_series(SeriesKind::Line);
    chart.set_series_pane(extra, 1, 1.0);
    let frame = chart.build_frame();
    assert_eq!(frame.panes.len(), 2);
    for pane in &frame.panes {
        let Some(Prim::Background { rect, gradient }) = pane.under.first() else {
            panic!("gradient background must lead the under layer");
        };
        assert_eq!(gradient.top, Color::rgb(0xff, 0x00, 0x00));
        assert_eq!(gradient.bottom, Color::rgb(0x00, 0x00, 0xff));
        assert_eq!(
            *rect,
            [
                pane.scissor[0] as f32,
                pane.scissor[1] as f32,
                pane.scissor[2] as f32,
                pane.scissor[3] as f32,
            ]
        );
    }

    // Back to solid: the prim disappears again.
    chart
        .apply_options(r##"{"layout":{"background":{"type":"solid","color":"#ffffff"}}}"##)
        .unwrap();
    let frame = chart.build_frame();
    assert!(!frame.panes.iter().any(|pane| pane
        .under
        .iter()
        .any(|p| matches!(p, Prim::Background { .. }))));
}

#[test]
fn separator_hover_mirrors_into_the_axis_frame() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    install_bars(&mut chart, 5);
    let extra = chart.add_series(SeriesKind::Line);
    chart.set_series_pane(extra, 1, 1.0);
    chart.layout_panes(472.0);
    chart.time_scale.set_width(800.0);

    // No hover by default; the hovered separator indexes into the frame's separators.
    let frame = chart.build_axis_frame(
        80.0,
        |text, _bold| text.len() as f64 * 6.0,
        |text, _bold| text.len() as f64 * 5.0,
    );
    assert_eq!(frame.separator_hover, None);
    assert_eq!(frame.separators.len(), 1);
    chart.set_separator_hover(Some(0));
    let frame = chart.build_axis_frame(
        80.0,
        |text, _bold| text.len() as f64 * 6.0,
        |text, _bold| text.len() as f64 * 5.0,
    );
    assert_eq!(frame.separator_hover, Some(0));
    chart.set_separator_hover(None);
    let frame = chart.build_axis_frame(
        80.0,
        |text, _bold| text.len() as f64 * 6.0,
        |text, _bold| text.len() as f64 * 5.0,
    );
    assert_eq!(frame.separator_hover, None);
}

// --- retention (`max_points`) ------------------------------------------------------------------

/// A series' current row count, straight from the data layer.
fn row_count(chart: &ChartEngine, id: SeriesId) -> usize {
    chart
        .data
        .series_data(id)
        .map_or(0, |(times, _)| times.len())
}

#[test]
fn series_are_unbounded_by_default() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    install_bars(&mut chart, 10);
    assert_eq!(chart.series_max_points(0), None);
    for i in 11..=2_000 {
        chart.update_series_bar(0, i as f64, [1.0, 1.0, 1.0, 1.0]);
    }
    assert_eq!(row_count(&chart, 0), 2_000, "no cap = no eviction");
}

#[test]
fn max_points_is_a_hard_ceiling_and_evicts_oldest_first() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    install_bars(&mut chart, 500);
    // Applying a cap trims the existing rows immediately.
    assert!(chart.set_series_max_points(0, Some(128)));
    assert_eq!(chart.series_max_points(0), Some(128));
    let after_apply = row_count(&chart, 0);
    assert!(
        after_apply <= 128,
        "over the ceiling after apply: {after_apply}"
    );

    // The newest rows are the survivors: the last installed time was 500.
    let (times, _) = chart.data.series_data(0).unwrap();
    assert_eq!(*times.last().unwrap(), 500);
    assert_eq!(times.len(), after_apply);
    assert_eq!(*times.first().unwrap(), 500 - after_apply as i64 + 1);

    // Streaming past the ceiling never exceeds it, and the floor stays within the documented
    // hysteresis margin.
    let floor = 128 - 128 / CAP_TRIM_MARGIN_DIVISOR;
    for i in 501..=3_000 {
        chart.update_series_bar(0, i as f64, [1.0, 1.0, 1.0, 1.0]);
        let rows = row_count(&chart, 0);
        assert!(rows <= 128, "exceeded the ceiling at t={i}: {rows}");
        assert!(rows >= floor, "trimmed below the margin at t={i}: {rows}");
    }
    // The window tracks the tip.
    let (times, _) = chart.data.series_data(0).unwrap();
    assert_eq!(*times.last().unwrap(), 3_000);
}

#[test]
fn max_points_trims_a_full_install_before_the_scale_sees_it() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    assert!(chart.set_series_max_points(0, Some(50)));
    install_bars(&mut chart, 1_000);
    let rows = row_count(&chart, 0);
    assert!(rows <= 50, "install ignored the ceiling: {rows}");
    // The time scale's point count matches the retained rows, not the installed ones — an evicted
    // row must never remain addressable through the shared axis.
    assert_eq!(chart.data.merged_times().len(), rows);
    assert_eq!(chart.time_scale.base_index(), rows as i64 - 1);
}

#[test]
fn max_points_during_replay_counts_and_evicts_only_the_revealed_rows() {
    let minute = |row: usize| row as f64 * 60.0;
    let clock = |row: usize| Some(row as i64 * 60 * 1_000_000);
    let visible_rows = |chart: &ChartEngine| {
        let (times, _) = chart.data.series_data(0).unwrap();
        (
            times.len(),
            times.first().map(|&time| time / 60),
            times.last().map(|&time| time / 60),
        )
    };
    let load = |rows: usize| {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        let times = (0..rows).map(minute).collect::<Vec<_>>();
        let bars = (0..rows)
            .map(|row| swinging_bar(row, 0))
            .collect::<Vec<_>>();
        let column = |index: usize| bars.iter().map(|bar| bar[index]).collect::<Vec<_>>();
        chart
            .set_series_data(0, &times, &column(0), &column(1), &column(2), &column(3))
            .unwrap();
        chart
    };
    let keep = 500 - 500 / CAP_TRIM_MARGIN_DIVISOR;

    // Live bars arriving past the clock stay hidden and uncounted: the replay view keeps its
    // rows, and the pending future rows wait for the clock.
    let mut chart = load(500);
    let sma = chart.add_sma(0, 5).unwrap();
    assert!(chart.set_series_max_points(0, Some(500)));
    chart.set_replay_clock_micros(clock(300)).unwrap();
    for row in 500..900 {
        chart.update_series_bar(0, minute(row), swinging_bar(row, 0));
    }
    assert_eq!(visible_rows(&chart), (301, Some(0), Some(300)));
    assert_eq!(chart.data.series_rows(0), Some(900));
    // A seek that reveals past the ceiling trims only the oldest revealed rows.
    chart.set_replay_clock_micros(clock(520)).unwrap();
    assert_eq!(
        visible_rows(&chart),
        (keep, Some(521 - keep as i64), Some(520))
    );
    // Another hidden bar never pushes a revealed one out.
    chart.update_series_bar(0, minute(900), swinging_bar(900, 0));
    assert_eq!(
        visible_rows(&chart),
        (keep, Some(521 - keep as i64), Some(520))
    );
    let binding = chart
        .indicators
        .iter()
        .position(|binding| binding.outputs[0] == sma)
        .unwrap();
    assert_binding_matches_fresh_install(&chart, binding, "after the revealing seek");
    // Clearing the clock reveals the rest, trimmed like a load of them.
    chart.set_replay_clock_micros(None).unwrap();
    assert_eq!(
        visible_rows(&chart),
        (keep, Some(901 - keep as i64), Some(900))
    );
    assert_binding_matches_fresh_install(&chart, binding, "after clearing the clock");

    // Applying a cap during replay keeps the newest revealed rows, however many are hidden.
    for rows in [1_000, 1_200] {
        let mut chart = load(rows);
        chart.set_replay_clock_micros(clock(600)).unwrap();
        assert!(chart.set_series_max_points(0, Some(500)));
        assert_eq!(
            visible_rows(&chart),
            (keep, Some(601 - keep as i64), Some(600)),
            "{rows} rows"
        );
        assert_eq!(chart.data.series_rows(0), Some(keep + rows - 601));
    }
}

#[test]
fn clearing_max_points_restores_unbounded_growth() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    install_bars(&mut chart, 100);
    assert!(chart.set_series_max_points(0, Some(32)));
    assert!(row_count(&chart, 0) <= 32);
    assert!(chart.set_series_max_points(0, None));
    for i in 101..=400 {
        chart.update_series_bar(0, i as f64, [1.0, 1.0, 1.0, 1.0]);
    }
    // The already-evicted rows do not come back; growth simply resumes.
    assert_eq!(chart.series_max_points(0), None);
    assert!(row_count(&chart, 0) > 32);
}

#[test]
fn a_cap_on_one_series_leaves_the_others_alone() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    install_bars(&mut chart, 200);
    let other = chart.add_series(SeriesKind::Line);
    let times: Vec<f64> = (1..=200).map(|i| i as f64).collect();
    let values: Vec<f64> = (0..200).map(|i| 50.0 + i as f64).collect();
    chart
        .set_series_data(other, &times, &values, &values, &values, &values)
        .unwrap();

    assert!(chart.set_series_max_points(0, Some(20)));
    assert!(row_count(&chart, 0) <= 20);
    assert_eq!(row_count(&chart, other), 200, "uncapped series untouched");
    // The shared axis keeps every time the uncapped series still occupies.
    assert_eq!(chart.data.merged_times().len(), 200);
}

#[test]
fn theme_switch_uses_aeris_tokens_without_replacing_market_data() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    install_bars(&mut chart, 20);

    chart.set_theme(ChartTheme::Light);
    let light = chart.options.get();
    assert_eq!(
        light.layout.background.color,
        aeris_charts_core::style::LIGHT_SURFACE_CSS
    );
    assert_eq!(
        light.layout.text_color,
        aeris_charts_core::style::LIGHT_FOREGROUND_CSS
    );
    assert_eq!(
        light.layout.muted_text_color,
        aeris_charts_core::style::LIGHT_MUTED_FOREGROUND_CSS
    );
    assert_eq!(
        light.right_price_scale.border_color,
        aeris_charts_core::style::LIGHT_BORDER_CSS
    );
    assert_eq!(
        light.crosshair.vert_line.color,
        aeris_charts_core::style::DARK_BORDER_CSS
    );
    assert_eq!(
        light.crosshair.horz_line.label_background_color,
        aeris_charts_core::style::DARK_MUTED_CSS
    );
    assert_eq!(row_count(&chart, 0), 20);

    chart.set_theme(ChartTheme::Dark);
    let dark = chart.options.get();
    assert_eq!(
        dark.layout.background.color,
        aeris_charts_core::style::DARK_SURFACE_CSS
    );
    assert_eq!(
        dark.layout.text_color,
        aeris_charts_core::style::DARK_FOREGROUND_CSS
    );
    assert_eq!(
        dark.layout.muted_text_color,
        aeris_charts_core::style::DARK_MUTED_FOREGROUND_CSS
    );
    assert_eq!(
        dark.right_price_scale.border_color,
        aeris_charts_core::style::DARK_BORDER_CSS
    );
    assert_eq!(
        dark.crosshair.vert_line.color,
        aeris_charts_core::style::DARK_BORDER_CSS
    );
    assert_eq!(
        dark.crosshair.horz_line.label_background_color,
        aeris_charts_core::style::DARK_MUTED_CSS
    );
    assert_eq!(row_count(&chart, 0), 20);
}

#[test]
fn unpinned_candles_follow_engine_theme_while_explicit_colors_stay_pinned() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    chart
        .set_series_data(
            0,
            &[1.0, 2.0],
            &[10.0, 11.0],
            &[12.0, 12.0],
            &[9.0, 8.0],
            &[11.0, 9.0],
        )
        .unwrap();

    chart.set_theme(ChartTheme::Light);
    assert_eq!(
        chart.series_bar_color(&chart.series[0], 0, None),
        Color::parse_css(aeris_charts_core::style::LIGHT_MARKET_UP_CSS).unwrap()
    );
    assert_eq!(
        chart.series_bar_color(&chart.series[0], 1, None),
        Color::parse_css(aeris_charts_core::style::LIGHT_MARKET_DOWN_CSS).unwrap()
    );

    chart.set_theme(ChartTheme::Dark);
    assert_eq!(
        chart.series_bar_color(&chart.series[0], 0, None),
        Color::parse_css(aeris_charts_core::style::DARK_MARKET_UP_CSS).unwrap()
    );
    assert_eq!(
        chart.series_bar_color(&chart.series[0], 1, None),
        Color::parse_css(aeris_charts_core::style::DARK_MARKET_DOWN_CSS).unwrap()
    );

    chart.series[0].up_color = Some("#010203".into());
    chart.series[0].down_color = Some("#040506".into());
    chart.set_theme(ChartTheme::Light);
    assert_eq!(
        chart.series_bar_color(&chart.series[0], 0, None),
        Color::rgb(1, 2, 3)
    );
    assert_eq!(
        chart.series_bar_color(&chart.series[0], 1, None),
        Color::rgb(4, 5, 6)
    );
}

// --- pane-local price scale geometry (issue #25) ------------------------------------------------

/// Main price pane (100..200) plus a bounded RSI pane, laid out through the real host path.
fn chart_with_indicator_pane() -> ChartEngine {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    let n = 40usize;
    let times: Vec<f64> = (1..=n).map(|i| i as f64).collect();
    let close: Vec<f64> = (0..n)
        .map(|i| 100.0 + (i as f64 * 0.7).sin() * 45.0 + 50.0)
        .collect();
    let high: Vec<f64> = close.iter().map(|v| v + 2.0).collect();
    let low: Vec<f64> = close.iter().map(|v| v - 2.0).collect();
    chart
        .set_series_data(0, &times, &close, &high, &low, &close)
        .unwrap();
    chart.add_rsi(0, 14).expect("valid rsi");
    chart.time_scale.set_width(800.0);
    chart.fit_content();
    chart.recompute_layout_with_measure(
        true,
        |text, _bold| text.len() as f64 * 6.0,
        |text, _bold| text.len() as f64 * 5.0,
    );
    chart.build_frame();
    chart
}

/// Every scale a pane owns is laid out against that pane's own slot and carries the pane offset
/// as its only chart-space transform — no full-content-height internal-margin simulation.
fn assert_scales_are_pane_local(chart: &ChartEngine) {
    for (pi, pane) in chart.panes.iter().enumerate() {
        for target in [
            PriceScaleTarget::Right,
            PriceScaleTarget::Left,
            PriceScaleTarget::Overlay,
        ] {
            let scale = pane.scale(target).unwrap();
            assert_eq!(scale.height(), pane.height, "pane {pi} {target:?} height");
            assert_eq!(scale.pane_offset(), pane.top, "pane {pi} {target:?} offset");
            // Fractional margins resolve against the pane slot alone.
            let margins = scale.options().scale_margins;
            let expected = pane.height * (1.0 - margins.top - margins.bottom);
            assert!(
                (scale.internal_height() - expected).abs() < 1e-9,
                "pane {pi} {target:?} internal height {} is not the pane-local {expected}",
                scale.internal_height()
            );
        }
    }
}

#[test]
fn pane_scales_own_local_geometry_and_round_trip_inside_their_pane() {
    let mut chart = chart_with_indicator_pane();
    assert_eq!(chart.panes.len(), 2);
    assert_scales_are_pane_local(&chart);

    let round_trip = |chart: &ChartEngine, pi: usize| {
        let pane = &chart.panes[pi];
        let scale = &pane.price_scale;
        let range = *scale.price_range().expect("an autoscaled range");
        for price in [
            range.min_value(),
            (range.min_value() + range.max_value()) / 2.0,
            range.max_value(),
        ] {
            let y = scale.price_to_coordinate(price, 0.0);
            assert!(
                y >= pane.top - 1.0 && y <= pane.top + pane.height + 1.0,
                "pane {pi}: price {price} maps to {y}, outside its slot \
                 [{}, {}]",
                pane.top,
                pane.top + pane.height
            );
            let back = scale.coordinate_to_price(y, 0.0);
            assert!(
                (back - price).abs() < 1e-6,
                "pane {pi}: {price} -> {y} -> {back} is not a round trip"
            );
        }
    };

    // The two panes hold materially different ranges.
    let main = *chart.panes[0].price_scale.price_range().unwrap();
    let rsi = *chart.panes[1].price_scale.price_range().unwrap();
    assert!(
        main.min_value() > rsi.max_value() || rsi.min_value() > main.max_value(),
        "the fixture panes must hold disjoint ranges (main {main:?}, rsi {rsi:?})"
    );
    round_trip(&chart, 0);
    round_trip(&chart, 1);

    // ...and still round-trip inside the owning pane after a divider resize.
    chart.drag_pane_separator(0, 90.0);
    chart.build_frame();
    assert_scales_are_pane_local(&chart);
    round_trip(&chart, 0);
    round_trip(&chart, 1);
}

#[test]
fn repeated_divider_resizes_keep_every_pane_scale_local() {
    let mut chart = chart_with_indicator_pane();
    let content_h = chart.pane_h;

    for delta in [60.0, -35.0, 120.0, -200.0, 15.0] {
        chart.drag_pane_separator(0, delta);
        chart.build_frame();
        let axis = chart.build_axis_frame(
            80.0,
            |text, _bold| text.len() as f64 * 6.0,
            |text, _bold| text.len() as f64 * 5.0,
        );
        assert_scales_are_pane_local(&chart);

        let total: f64 = chart.panes.iter().map(|p| p.height).sum();
        assert!(
            (total + PANE_SEPARATOR - content_h).abs() < 1e-6,
            "panes still tile the content area"
        );
        for pane in &chart.panes {
            assert!(pane.height >= 24.0, "the reference 24px minimum holds");
            let range = pane.price_scale.price_range().expect("a live range");
            assert!(
                range.length() > 0.0 && range.min_value().is_finite(),
                "pane keeps a valid independent range across resizes"
            );
            assert!(
                pane.price_scale.internal_height() > 0.0,
                "pane keeps a positive internal height"
            );
        }
        // Ticks and labels resolve inside the pane that owns their scale.
        for tick in &axis.price_ticks {
            let pane = &chart.panes[chart.pane_index_at_y(tick.y)];
            assert!(
                tick.y >= pane.top - 0.5 && tick.y <= pane.top + pane.height + 0.5,
                "tick {} escapes its pane [{}, {}]",
                tick.y,
                pane.top,
                pane.top + pane.height
            );
        }
    }
}

#[test]
fn panning_or_zooming_one_pane_scale_leaves_the_other_panes_untouched() {
    let mut chart = chart_with_indicator_pane();
    let target = PriceScaleTarget::Right;
    chart.set_price_scale_auto_scale_for(0, target, false);
    chart.set_price_scale_auto_scale_for(1, target, false);
    chart.build_frame();

    let main_before = *chart.panes[0].price_scale.price_range().unwrap();
    let main_revision = chart.panes[0].price_scale.revision();
    let main_probe = chart.panes[0].price_scale.price_to_coordinate(150.0, 0.0);
    let lower_before = *chart.panes[1].price_scale.price_range().unwrap();

    // Pan the lower pane's axis with chart-space coordinates inside that pane.
    let inside = chart.panes[1].top + chart.panes[1].height / 2.0;
    chart.price_axis_start_scroll(1, target, inside);
    chart.price_axis_scroll_to(1, target, inside + 25.0);
    chart.price_axis_end_scroll(1, target);
    // ...then zoom it.
    chart.price_axis_wheel_zoom(1, target, inside, 1.0);

    assert_ne!(
        *chart.panes[1].price_scale.price_range().unwrap(),
        lower_before,
        "the dragged pane's own range moved"
    );
    assert_eq!(
        *chart.panes[0].price_scale.price_range().unwrap(),
        main_before,
        "the main pane's range is untouched"
    );
    assert_eq!(
        chart.panes[0].price_scale.revision(),
        main_revision,
        "the main pane's scale state did not change"
    );
    assert_eq!(
        chart.panes[0].price_scale.price_to_coordinate(150.0, 0.0),
        main_probe,
        "the main pane's coordinates do not depend on the other pane's scale"
    );
}

#[test]
fn two_indicator_panes_and_a_named_pane_scale_stay_independent() {
    let mut chart = chart_with_indicator_pane();
    let extra = chart.add_series(SeriesKind::Line);
    chart.set_series_pane(extra, 2, 1.0);
    let times: Vec<f64> = (1..=40).map(|i| i as f64).collect();
    let values: Vec<f64> = (0..40).map(|i| -50.0 - i as f64).collect();
    chart
        .set_series_data(extra, &times, &values, &values, &values, &values)
        .unwrap();
    let named = chart
        .add_price_scale(1, "indicator-left", PriceScaleSide::Left, None, true)
        .expect("a named pane scale");
    let named_series = chart.add_series(SeriesKind::Line);
    chart.set_series_pane(named_series, 1, 1.0);
    let named_values: Vec<f64> = (0..40).map(|i| 1000.0 + i as f64 * 3.0).collect();
    chart
        .set_series_data(
            named_series,
            &times,
            &named_values,
            &named_values,
            &named_values,
            &named_values,
        )
        .unwrap();
    chart.set_series_price_scale(named_series, named);
    chart.recompute_layout_with_measure(
        true,
        |text, _bold| text.len() as f64 * 6.0,
        |text, _bold| text.len() as f64 * 5.0,
    );
    chart.build_frame();

    assert_eq!(chart.panes.len(), 3);
    assert_scales_are_pane_local(&chart);
    for (pi, pane) in chart.panes.iter().enumerate() {
        for entry in &pane.named_scales {
            assert_eq!(entry.scale.height(), pane.height, "pane {pi} named height");
            assert_eq!(
                entry.scale.pane_offset(),
                pane.top,
                "pane {pi} named offset"
            );
        }
    }

    // The named scale inside the indicator pane round-trips against its own pane, not the chart.
    let pane = &chart.panes[1];
    let scale = pane.scale(named).expect("the named scale");
    let range = *scale.price_range().expect("an autoscaled range");
    let y = scale.price_to_coordinate(range.max_value(), 0.0);
    assert!(
        y >= pane.top - 1.0 && y <= pane.top + pane.height + 1.0,
        "named scale coordinate {y} escapes its pane"
    );
    assert!((scale.coordinate_to_price(y, 0.0) - range.max_value()).abs() < 1e-6);

    // Each pane's main scale keeps its own disjoint range.
    let ranges: Vec<_> = chart
        .panes
        .iter()
        .map(|p| *p.price_scale.price_range().unwrap())
        .collect();
    assert!(ranges[0].min_value() > ranges[1].max_value());
    assert!(ranges[1].min_value() > ranges[2].max_value());
}

// --- sub-pane coordinate contract and crosshair sync -----------------------------------------------

/// `chart_with_indicator_pane` plus a volume-like series on the pane-0 overlay scale.
fn chart_with_overlay_series() -> (ChartEngine, SeriesId) {
    let mut chart = chart_with_indicator_pane();
    let overlay = chart.add_series(SeriesKind::Histogram);
    let times: Vec<f64> = (1..=40).map(|i| i as f64).collect();
    let volume: Vec<f64> = (0..40)
        .map(|i| 1_000.0 + (i as f64 * 37.0) % 900.0)
        .collect();
    chart
        .set_series_data(overlay, &times, &volume, &volume, &volume, &volume)
        .unwrap();
    chart.set_series_price_scale(overlay, PriceScaleTarget::Overlay);
    chart.build_frame();
    (chart, overlay)
}

/// A second pane-0 line on `target` whose values (300..378) sit far outside the main series'
/// (about 55..195), so its scale can never be mistaken for the main series'.
fn add_pane_zero_comparison(chart: &mut ChartEngine, target: PriceScaleTarget) -> SeriesId {
    let comparison = chart.add_series(SeriesKind::Line);
    let times: Vec<f64> = (1..=40).map(|i| i as f64).collect();
    let values: Vec<f64> = (0..40).map(|i| 300.0 + i as f64 * 2.0).collect();
    chart
        .set_series_data(comparison, &times, &values, &values, &values, &values)
        .unwrap();
    chart.set_series_price_scale(comparison, target);
    comparison
}

/// `chart_with_indicator_pane` plus a second pane-0 line on the same Right scale whose first
/// visible value differs from the main series', in percentage mode (per-series bases differ).
fn chart_with_percentage_comparison() -> (ChartEngine, SeriesId) {
    let mut chart = chart_with_indicator_pane();
    let comparison = add_pane_zero_comparison(&mut chart, PriceScaleTarget::Right);
    chart.set_price_scale_mode(0, false, PriceScaleMode::Percentage);
    chart.build_frame();
    (chart, comparison)
}

#[test]
fn crosshair_sync_round_trips_on_a_sub_pane() {
    let mut chart = chart_with_indicator_pane();
    let rsi = chart.pane_series_ids(1)[0];
    assert!(chart.set_crosshair_position(50.0, 30.0, rsi));
    let (_, y) = chart.crosshair.expect("the synthetic crosshair");
    assert_eq!(
        chart.pane_at_y(y),
        Some(1),
        "the crosshair sits in the RSI pane"
    );

    let sync = chart.crosshair_sync_position().expect("a sync position");
    assert_eq!(sync.pane_index, 1);
    assert!(
        (sync.price - 50.0).abs() < 1e-6,
        "the sync price {} is not the RSI price the crosshair was placed at",
        sync.price
    );

    // A linked, identically laid out chart lands on the same chart-content y.
    let mut linked = chart_with_indicator_pane();
    assert!(linked.apply_external_crosshair(Some(sync)));
    let (_, linked_y) = linked.crosshair.expect("the applied crosshair");
    assert!(
        (linked_y - y).abs() < 1e-9,
        "the applied crosshair y {linked_y} is not the source y {y}"
    );
}

#[test]
fn pane_and_chart_level_conversions_select_pane_by_y_and_default_scale() {
    let chart = chart_with_indicator_pane();
    let rsi = chart.pane_series_ids(1)[0];
    let y_rsi = chart.series_price_to_coordinate(rsi, 50.0).unwrap();
    let y_main = chart.series_price_to_coordinate(0, 120.0).unwrap();
    // The public coordinate space is the shared chart content, never pane-local.
    assert!(y_rsi >= chart.panes[1].top);
    assert!(y_rsi > chart.panes[0].top + chart.panes[0].height);

    let series_price = |id: SeriesId, y: f64| chart.series_coordinate_to_price(id, y).unwrap();
    assert!((chart.coordinate_to_price(y_rsi).unwrap() - series_price(rsi, y_rsi)).abs() < 1e-9);
    assert!((chart.coordinate_to_price(y_main).unwrap() - series_price(0, y_main)).abs() < 1e-9);
    assert!((chart.pane_price_to_coordinate(1, 50.0).unwrap() - y_rsi).abs() < 1e-9);
    assert!((chart.pane_price_to_coordinate(0, 120.0).unwrap() - y_main).abs() < 1e-9);

    // A separator resolves to the pane above and a y below the content to the last pane.
    let separator_y = chart.panes[0].top + chart.panes[0].height + 0.5;
    assert_eq!(chart.pane_index_at_y(separator_y), 0);
    assert!(
        (chart.coordinate_to_price(separator_y).unwrap() - series_price(0, separator_y)).abs()
            < 1e-9
    );
    let below_y = chart.pane_h + 40.0;
    assert_eq!(chart.pane_index_at_y(below_y), 1);
    assert!(
        (chart.coordinate_to_price(below_y).unwrap() - series_price(rsi, below_y)).abs() < 1e-9
    );

    // Unusable inputs are `None`, never a panic or a non-finite number.
    assert_eq!(chart.pane_price_to_coordinate(9, 50.0), None);
    assert_eq!(chart.pane_coordinate_to_price(9, y_rsi), None);
    assert_eq!(chart.pane_price_to_coordinate(1, f64::NAN), None);
    assert_eq!(chart.pane_coordinate_to_price(1, f64::INFINITY), None);
    assert_eq!(chart.coordinate_to_price(f64::NAN), None);
    let empty = ChartEngine::new(800.0, 500.0, 1.0);
    assert_eq!(empty.coordinate_to_price(10.0), None);
    assert_eq!(empty.pane_price_to_coordinate(0, 100.0), None);
}

/// The chart-level pair must agree with `series` (the pane-0 series whose scale is the pane's
/// default) at `price` and round-trip through pane 0. When `other` is given it names a pane-0
/// series on a different scale that the pair must not follow.
fn assert_chart_level_follows(
    chart: &ChartEngine,
    series: SeriesId,
    other: Option<SeriesId>,
    price: f64,
    context: &str,
) {
    let y = chart
        .pane_price_to_coordinate(0, price)
        .unwrap_or_else(|| panic!("{context}: no chart-level coordinate for {price}"));
    let y_series = chart.series_price_to_coordinate(series, price).unwrap();
    assert!(
        (y - y_series).abs() < 1e-9,
        "{context}: chart-level y {y} is not series {series}'s y {y_series}"
    );
    if let Some(other) = other {
        let y_other = chart.series_price_to_coordinate(other, price).unwrap();
        assert!(
            (y - y_other).abs() > 1.0,
            "{context}: chart-level y {y} follows series {other}'s scale ({y_other})"
        );
    }
    assert_eq!(chart.pane_index_at_y(y), 0, "{context}: y {y} left pane 0");
    let back = chart
        .coordinate_to_price(y)
        .unwrap_or_else(|| panic!("{context}: no chart-level price for y {y}"));
    assert!(
        (back - price).abs() < 1e-6,
        "{context}: {price} -> {y} -> {back}"
    );
    let back_series = chart.series_coordinate_to_price(series, y).unwrap();
    assert!(
        (back - back_series).abs() < 1e-9,
        "{context}: chart-level price {back} is not series {series}'s {back_series}"
    );
}

#[test]
fn chart_level_conversions_use_the_pane_default_scale_not_the_first_series() {
    // The main series (created first) moves to the overlay scale, which never decides a pane's
    // default; the comparison on the right scale does. A converter that follows series creation
    // order would read the main series' overlay scale here.
    let mut chart = chart_with_indicator_pane();
    let comparison = add_pane_zero_comparison(&mut chart, PriceScaleTarget::Right);
    chart.set_series_price_scale(0, PriceScaleTarget::Overlay);
    chart.build_frame();
    assert_eq!(chart.pane_default_scale_target(0), PriceScaleTarget::Right);
    assert_chart_level_follows(&chart, comparison, Some(0), 340.0, "main on overlay");

    // Back on the right scale the main series is the pane's first visible source again.
    chart.set_series_price_scale(0, PriceScaleTarget::Right);
    chart.build_frame();
    assert_chart_level_follows(&chart, 0, None, 120.0, "main back on the right scale");
}

#[test]
fn chart_level_conversions_follow_a_hidden_main_series_to_the_next_source() {
    for target in [PriceScaleTarget::Right, PriceScaleTarget::Left] {
        let mut chart = chart_with_indicator_pane();
        let comparison = add_pane_zero_comparison(&mut chart, target);
        chart.build_frame();

        // A visible main series is the default source: a comparison on the left scale never
        // takes the chart-level pair away from it.
        assert_eq!(chart.pane_default_scale_target(0), PriceScaleTarget::Right);
        let other = (target == PriceScaleTarget::Left).then_some(comparison);
        assert_chart_level_follows(&chart, 0, other, 120.0, "visible main");
        let with_main = chart.pane_price_to_coordinate(0, 340.0).unwrap();

        // Hiding it hands the pane's default scale to the comparison.
        chart.set_series_visible(0, false);
        chart.build_frame();
        assert_eq!(chart.pane_default_scale_target(0), target);
        assert_chart_level_follows(&chart, comparison, None, 340.0, "hidden main");
        let without_main = chart.pane_price_to_coordinate(0, 340.0).unwrap();
        assert!(
            (with_main - without_main).abs() > 1.0,
            "{target:?}: hiding the main series left the chart-level y at {with_main}"
        );

        // Showing it again restores the main series as the default source.
        chart.set_series_visible(0, true);
        chart.build_frame();
        assert_eq!(chart.pane_default_scale_target(0), PriceScaleTarget::Right);
        assert_chart_level_follows(&chart, 0, other, 120.0, "main shown again");
    }
}

#[test]
fn sub_pane_series_conversions_stay_in_the_shared_content_space() {
    let mut chart = chart_with_indicator_pane();
    let rsi = chart.pane_series_ids(1)[0];
    let check = |chart: &ChartEngine| {
        let pane = &chart.panes[1];
        for price in [45.0, 50.0, 55.0] {
            let y = chart.series_price_to_coordinate(rsi, price).unwrap();
            assert!(
                y >= pane.top && y <= pane.top + pane.height,
                "price {price} maps to {y}, outside pane 1 [{}, {}]",
                pane.top,
                pane.top + pane.height
            );
            let back = chart.series_coordinate_to_price(rsi, y).unwrap();
            assert!((back - price).abs() < 1e-9, "{price} -> {y} -> {back}");
            assert_eq!(y, pane.price_scale.price_to_coordinate(price, 0.0));
        }
    };
    check(&chart);
    // Conversions reflect the last layout pass: after a divider drag and a rebuild the contract
    // still holds against the moved pane.
    chart.drag_pane_separator(0, 90.0);
    chart.build_frame();
    check(&chart);
}

#[test]
fn sync_price_uses_the_pane_default_scale_for_overlay_series() {
    let (mut chart, overlay) = chart_with_overlay_series();
    let volume = chart.series_data(overlay)[29].close;
    assert!(chart.set_crosshair_position(volume, 30.0, overlay));
    let (_, y) = chart.crosshair.unwrap();
    let events = chart.take_sync_events();
    let ChartSyncEventKind::Crosshair { position } = events[0].kind.clone() else {
        panic!("a crosshair event");
    };
    assert_eq!(position.pane_index, 0);
    // The chart-level pair ignores the overlay series' own scale too.
    let y_main = chart.series_price_to_coordinate(0, 120.0).unwrap();
    assert!((chart.pane_price_to_coordinate(0, 120.0).unwrap() - y_main).abs() < 1e-9);
    let expected = chart.pane_coordinate_to_price(0, y).unwrap();
    assert_eq!(
        position.price, expected,
        "the price is on the pane default scale"
    );
    assert!(
        (position.price - volume).abs() > 1.0,
        "the raw overlay value must not leak into the sync price"
    );

    let (mut linked, _) = chart_with_overlay_series();
    assert!(linked.apply_external_crosshair(Some(position)));
    let (_, linked_y) = linked.crosshair.unwrap();
    assert!(
        (linked_y - y).abs() < 1e-9,
        "linked y {linked_y} vs source y {y}"
    );
}

#[test]
fn sync_price_is_rebased_for_non_default_series_in_percentage_mode() {
    let (mut chart, comparison) = chart_with_percentage_comparison();
    let main_close = chart.series_data(0)[29].close;
    let comparison_close = chart.series_data(comparison)[29].close;

    // The default series keeps its raw host price bit-exact.
    assert!(chart.set_crosshair_position(main_close, 30.0, 0));
    let ChartSyncEventKind::Crosshair { position: main } = chart.take_sync_events()[0].kind.clone()
    else {
        panic!("a crosshair event");
    };
    assert_eq!(main.price, main_close);

    // The comparison shares the Right scale but has its own first-value base, so its raw price is
    // not a price on the default series' base and is re-expressed on it.
    assert!(chart.set_crosshair_position(comparison_close, 30.0, comparison));
    let (_, y) = chart.crosshair.unwrap();
    let ChartSyncEventKind::Crosshair { position } = chart.take_sync_events()[0].kind.clone()
    else {
        panic!("a crosshair event");
    };
    assert!(
        (position.price - comparison_close).abs() > 1.0,
        "the comparison's raw price {comparison_close} must be rebased, got {}",
        position.price
    );

    let (mut linked, _) = chart_with_percentage_comparison();
    assert!(linked.apply_external_crosshair(Some(position)));
    let (_, linked_y) = linked.crosshair.unwrap();
    assert!(
        (linked_y - y).abs() < 1e-9,
        "linked y {linked_y} vs source y {y}"
    );
}

#[test]
fn external_crosshair_price_outside_the_pane_range_stays_in_that_pane() {
    for (pane_index, price) in [(1, 1.0e6), (1, -1.0e6), (0, -1.0e6), (0, 1.0e6)] {
        let mut chart = chart_with_indicator_pane();
        assert!(chart.apply_external_crosshair(Some(CrosshairSyncPosition {
            time: 30.0,
            price,
            pane_index,
        })));
        let (_, y) = chart.crosshair.unwrap();
        assert_eq!(
            chart.pane_at_y(y),
            Some(pane_index),
            "price {price} on pane {pane_index} drew the crosshair at y {y}"
        );
    }
}

#[test]
fn crosshair_sync_position_survives_the_indicator_warmup_window() {
    let mut chart = chart_with_indicator_pane();
    // RSI(14) has no value before row 14: park the window inside the warmup rows.
    chart.set_visible_logical_range(0.0, 8.0);
    chart.build_frame();
    let rsi_y = chart.panes[1].top + chart.panes[1].height / 2.0;
    let x = chart.time_scale.index_to_coordinate(4);
    chart.crosshair = Some((x, rsi_y));
    let sync = chart.crosshair_sync_position().expect("a sync position");
    assert_eq!(sync.pane_index, 1);
    assert!(sync.price.is_finite());
}

#[test]
fn pane_separators_span_the_full_chart_width_at_rest_and_on_hover() {
    use aeris_charts_render::draw_list::Prim;

    let mut chart = chart_with_indicator_pane();
    chart
        .apply_options(r##"{"leftPriceScale":{"visible":true}}"##)
        .unwrap();
    chart.recompute_layout_with_measure(
        true,
        |text, _bold| text.len() as f64 * 6.0,
        |text, _bold| text.len() as f64 * 5.0,
    );
    chart.build_frame();
    assert!(chart.left_axis_w > 0.0, "the left axis strip is visible");
    assert!(chart.axis_w > 0.0, "the right axis strip is visible");

    let bitmap_w = (chart.css_width * chart.dpr).round().max(1.0) as i32;
    let axis = chart.build_axis_frame(
        80.0,
        |text, _bold| text.len() as f64 * 6.0,
        |text, _bold| text.len() as f64 * 5.0,
    );
    assert_eq!(axis.separators.len(), 1);
    let separator_y = (axis.separators[0] * chart.dpr).round() as i32;

    let mut prims = Vec::new();
    chart.build_axis_primitives_into(&axis, &mut prims, |_| 0.0);
    let resting = prims
        .iter()
        .filter_map(|p| match p {
            Prim::Rect { rect, .. } if rect.y == separator_y && rect.h == 1 => Some(*rect),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(resting.len(), 1, "one resting separator line");
    assert_eq!(
        resting[0].x, 0,
        "the divider starts at the chart's left edge"
    );
    assert_eq!(
        resting[0].w, bitmap_w,
        "the divider covers the complete bitmap width, price-scale strips included"
    );

    chart.set_separator_hover(Some(0));
    let axis = chart.build_axis_frame(
        80.0,
        |text, _bold| text.len() as f64 * 6.0,
        |text, _bold| text.len() as f64 * 5.0,
    );
    chart.build_axis_primitives_into(&axis, &mut prims, |_| 0.0);
    let hover = prims
        .iter()
        .find_map(|p| match p {
            Prim::Rect { rect, .. } if rect.h == 9 => Some(*rect),
            _ => None,
        })
        .expect("the hover band");
    assert_eq!(hover.x, 0);
    assert_eq!(hover.w, bitmap_w, "the hover band matches the resting line");
    assert_eq!(hover.y, separator_y - 4);
}

#[test]
fn axis_and_pane_borders_project_the_canonical_half_pixel_width() {
    use aeris_charts_render::draw_list::Prim;

    for dpr in [1.0_f64, 1.5, 2.0, 3.0] {
        let mut chart = chart_with_indicator_pane();
        chart.dpr = dpr;
        chart.recompute_layout_with_measure(
            true,
            |text, _bold| text.len() as f64 * 6.0,
            |text, _bold| text.len() as f64 * 5.0,
        );
        chart.build_frame();

        let axis = chart.build_axis_frame(
            80.0,
            |text, _bold| text.len() as f64 * 6.0,
            |text, _bold| text.len() as f64 * 5.0,
        );
        let mut prims = Vec::new();
        chart.build_axis_primitives_into(&axis, &mut prims, |_| 0.0);

        let expected = aeris_charts_core::style::border_width_device_px(dpr) as i32;
        let right_x = ((chart.pane_left + chart.pane_w) * dpr).round() as i32;
        let pane_bottom = (chart.pane_h * dpr).round() as i32;
        let bitmap_w = (chart.css_width * dpr).round().max(1.0) as i32;
        let separator_y = (axis.separators[0] * dpr).round() as i32;

        assert!(
            prims.iter().any(|p| matches!(
                p,
                Prim::Rect { rect, .. }
                    if rect.x == right_x
                        && rect.y == 0
                        && rect.w == expected
                        && rect.h == pane_bottom
            )),
            "right price-axis border must use {expected} device px at dpr {dpr}"
        );
        assert!(
            prims.iter().any(|p| matches!(
                p,
                Prim::Rect { rect, .. }
                    if rect.x == 0
                        && rect.y == pane_bottom
                        && rect.w == bitmap_w
                        && rect.h == expected
            )),
            "time-axis border must use {expected} device px at dpr {dpr}"
        );
        assert!(
            prims.iter().any(|p| matches!(
                p,
                Prim::Rect { rect, .. }
                    if rect.x == 0
                        && rect.y == separator_y
                        && rect.w == bitmap_w
                        && rect.h == expected
            )),
            "pane separator must use {expected} device px at dpr {dpr}"
        );
    }
}

/// A hollow candle's chrome follows what is painted, not the invisible body.
#[test]
fn hollow_candles_keep_their_direction_color_on_the_live_price_chip() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    // Last bar closes UP, so the cluster follows the up direction.
    chart
        .set_series_data(
            0,
            &[1.0, 2.0, 3.0],
            &[10.0, 11.0, 12.0],
            &[11.0, 12.0, 14.0],
            &[9.0, 10.0, 11.0],
            &[10.5, 11.5, 13.5],
        )
        .unwrap();
    chart.time_scale.set_width(800.0);
    chart.fit_content();

    let up = Color::rgb(0x08, 0x99, 0x81);
    let border_up = Color::rgb(0x26, 0xa6, 0x9a);
    let wick_up = Color::rgb(0xff, 0x00, 0xff);
    let chip_color = |chart: &mut ChartEngine| {
        chart
            .build_axis_frame(
                80.0,
                |t, _bold| t.len() as f64 * 7.0,
                |t, _bold| t.len() as f64 * 6.0,
            )
            .labels
            .into_iter()
            .find_map(|label| match label.background {
                Some((.., color)) if label.midpoint == AxisTextMidpoint::Label => Some(color),
                _ => None,
            })
            .expect("a last-value chip")
    };

    // Solid body: the chip is the body color, as before.
    chart.series[0].up_color = Some("#089981".into());
    assert_eq!(chip_color(&mut chart), up);

    // Hollow body with a pinned border: the border frame is what the eye sees, so the chip
    // follows it instead of turning into a fully transparent (surface-colored) chip.
    chart.series[0].up_color = Some("transparent".into());
    chart.series[0].border_up_color = Some("#26a69a".into());
    let hollow = chip_color(&mut chart);
    assert_eq!(hollow, border_up);
    assert_ne!(hollow.a(), 0, "the chip must never be painted transparent");

    // Borders switched off: the wick is the only painted part left.
    chart.series[0].border_visible = Some(false);
    chart.series[0].wick_up_color = Some("#ff00ff".into());
    assert_eq!(chip_color(&mut chart), wick_up);

    // Nothing painted at all (an entirely invisible candle): there is no bar color to follow, so
    // the body still stands. The chip stays opaque either way — `Color::solid` pins the chip
    // alpha, which is what turned a transparent body into an opaque BLACK chip before this
    // resolution existed.
    chart.series[0].wick_visible = Some(false);
    assert_eq!(chip_color(&mut chart).a(), 0xFF);
}

#[test]
fn price_tick_labels_keep_their_full_height_clear_of_pane_dividers() {
    for dpr in [1.0, 1.25, 1.5, 2.0] {
        let mut chart = chart_with_indicator_pane();
        chart.dpr = dpr;
        for offset in 0..50 {
            chart.set_price_scale_visible_range(
                0,
                false,
                80.0 + offset as f64,
                220.0 + offset as f64,
            );
            chart.set_price_scale_visible_range(
                1,
                false,
                -50.0 + offset as f64,
                200.0 + offset as f64,
            );
            let axis = chart.build_axis_frame(
                100.0,
                |text, _| text.len() as f64 * 6.0,
                |text, _| text.len() as f64 * 5.0,
            );
            let half = chart.axis_metrics().axis / 2.0;
            for label in axis.labels.iter().filter(|label| {
                label.background.is_none() && label.midpoint == AxisTextMidpoint::Label
            }) {
                let pane = &chart.panes[chart.pane_index_at_y(label.y)];
                if pane.top > 0.0 {
                    assert!(
                        label.y - half >= pane.top,
                        "{} crosses pane top at {}",
                        label.text,
                        label.y
                    );
                }
                if pane.top + pane.height < chart.pane_h {
                    assert!(
                        label.y + half <= pane.top + pane.height,
                        "{} crosses pane bottom at {}",
                        label.text,
                        label.y
                    );
                }
            }
        }
    }
}

#[test]
fn comparison_anchor_drives_shared_bases_and_bounded_legend_values() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    chart
        .set_series_data(
            0,
            &[1.0, 2.0, 3.0],
            &[100.0, 110.0, 120.0],
            &[100.0, 110.0, 120.0],
            &[100.0, 110.0, 120.0],
            &[100.0, 110.0, 120.0],
        )
        .unwrap();
    let second = chart.add_series(SeriesKind::Line);
    chart
        .set_series_data(
            second,
            &[1.0, 2.0, 3.0],
            &[200.0, 180.0, 220.0],
            &[200.0, 180.0, 220.0],
            &[200.0, 180.0, 220.0],
            &[200.0, 180.0, 220.0],
        )
        .unwrap();
    assert!(chart.set_comparison_anchor(Some(2.0)));
    assert_eq!(chart.series_base_value(0, 0), Some(110.0));
    assert_eq!(chart.series_base_value(second, 0), Some(180.0));
    let legend = chart.comparison_legend_snapshot();
    assert_eq!(legend.len(), 2);
    assert_eq!(legend[0].anchor_value, Some(110.0));
    assert_eq!(legend[0].latest_value, Some(120.0));
    assert_eq!(legend[0].percent_change, Some(100.0 / 11.0));
    assert_eq!(legend[1].anchor_value, Some(180.0));
    assert_eq!(legend[1].latest_value, Some(220.0));
    assert_eq!(legend[1].percent_change, Some(200.0 / 9.0));
    assert!(chart.set_comparison_anchor(None));
    assert_eq!(chart.comparison_anchor(), None);
}

/// Every built-in indicator kind, including the convention variants, for whitespace checks.
fn every_indicator_kind_with_conventions() -> Vec<IndicatorKind> {
    vec![
        IndicatorKind::Sma { period: 5 },
        IndicatorKind::Ema {
            period: 5,
            seed: IndicatorSeed::Sma,
        },
        IndicatorKind::Ema {
            period: 5,
            seed: IndicatorSeed::FirstValue,
        },
        IndicatorKind::Dema {
            period: 4,
            seed: IndicatorSeed::FirstValue,
        },
        IndicatorKind::Tema {
            period: 3,
            seed: IndicatorSeed::Sma,
        },
        IndicatorKind::Smma { period: 5 },
        IndicatorKind::Hma { period: 5 },
        IndicatorKind::Vwma { period: 5 },
        IndicatorKind::StandardDeviation { period: 5 },
        IndicatorKind::Cci { period: 5 },
        IndicatorKind::WilliamsR { period: 5 },
        IndicatorKind::StochasticRsi {
            rsi_period: 5,
            stochastic_period: 5,
        },
        IndicatorKind::Momentum { period: 5 },
        IndicatorKind::RateOfChange { period: 5 },
        IndicatorKind::Donchian { period: 5 },
        IndicatorKind::PivotPoints {
            variant: aeris_charts_indicators::PivotKind::Standard,
        },
        IndicatorKind::ZigZag {
            deviation_percent: 2.0,
        },
        IndicatorKind::Keltner {
            period: 5,
            multiplier: 2.0,
        },
        IndicatorKind::AdxDmi { period: 5 },
        IndicatorKind::ParabolicSar,
        IndicatorKind::SuperTrend {
            period: 5,
            multiplier: 3.0,
        },
        IndicatorKind::Ichimoku,
        IndicatorKind::EmaRibbon {
            periods: [3, 5, 8, 13, 21],
        },
        IndicatorKind::Bollinger {
            period: 5,
            deviation: 2.0,
            estimator: DeviationEstimator::Sample,
        },
        IndicatorKind::Rsi {
            period: 5,
            seed: IndicatorSeed::Sma,
        },
        IndicatorKind::Rsi {
            period: 5,
            seed: IndicatorSeed::FirstValue,
        },
        IndicatorKind::Macd {
            fast: 3,
            slow: 6,
            signal: 4,
            seed: IndicatorSeed::Sma,
            histogram_multiplier: 1.0,
        },
        IndicatorKind::Macd {
            fast: 3,
            slow: 6,
            signal: 4,
            seed: IndicatorSeed::FirstValue,
            histogram_multiplier: 2.0,
        },
        IndicatorKind::Stochastic {
            k_period: 5,
            d_period: 3,
        },
        IndicatorKind::Atr { period: 5 },
        IndicatorKind::Vwap,
        IndicatorKind::Obv,
        IndicatorKind::Cmf { period: 5 },
        IndicatorKind::Mfi { period: 5 },
        IndicatorKind::Volume { period: 5 },
        IndicatorKind::VwapBands {
            reset: aeris_charts_indicators::VwapReset::Session,
            standard_deviation: 1.0,
            percent: 5.0,
        },
        IndicatorKind::Wma { period: 5 },
        IndicatorKind::Kdj {
            period: 5,
            k_smoothing: 3,
            d_smoothing: 3,
            seed: aeris_charts_indicators::KdjSeed::Fifty,
        },
        IndicatorKind::Kdj {
            period: 5,
            k_smoothing: 3,
            d_smoothing: 3,
            seed: aeris_charts_indicators::KdjSeed::FirstValue,
        },
    ]
}

struct WhitespaceSource {
    times: Vec<f64>,
    open: Vec<f64>,
    high: Vec<f64>,
    low: Vec<f64>,
    close: Vec<f64>,
    blank: Vec<bool>,
}

impl WhitespaceSource {
    fn new(rows: usize, blank: &[usize]) -> Self {
        let mut source = Self {
            times: Vec::new(),
            open: Vec::new(),
            high: Vec::new(),
            low: Vec::new(),
            close: Vec::new(),
            blank: Vec::new(),
        };
        for row in 0..rows {
            let is_blank = blank.contains(&row);
            let pick = |value: f64| if is_blank { f64::NAN } else { value };
            let close = 100.0 + (row as f64 * 0.43).sin() * 5.0 + row as f64 * 0.06;
            source.times.push(1_700_000_000.0 + row as f64 * 3_600.0);
            source
                .open
                .push(pick(close + (row as f64 * 0.9).cos() * 0.7));
            source.high.push(pick(close + 1.2 + (row % 3) as f64 * 0.3));
            source.low.push(pick(close - 1.0 - (row % 4) as f64 * 0.2));
            source.close.push(pick(close));
            source.blank.push(is_blank);
        }
        source
    }

    /// The same rows with every whitespace row removed.
    fn compacted(&self) -> Self {
        let keep = |column: &[f64]| {
            column
                .iter()
                .zip(&self.blank)
                .filter(|(_, blank)| !**blank)
                .map(|(value, _)| *value)
                .collect::<Vec<_>>()
        };
        Self {
            times: keep(&self.times),
            open: keep(&self.open),
            high: keep(&self.high),
            low: keep(&self.low),
            close: keep(&self.close),
            blank: vec![false; self.blank.iter().filter(|blank| !**blank).count()],
        }
    }
}

/// A chart with a candle source, a volume/amount pair covering every source timestamp, and one
/// binding of `kind`.
fn whitespace_chart(
    kind: &IndicatorKind,
    source: &WhitespaceSource,
    rows: usize,
) -> (ChartEngine, Vec<SeriesId>) {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    let volume = chart.add_series(SeriesKind::Histogram);
    let all = WhitespaceSource::new(80, &[]);
    let volumes = (0..80)
        .map(|row| (row % 6 + 1) as f64 * 10.0)
        .collect::<Vec<_>>();
    chart
        .set_series_data(volume, &all.times, &volumes, &volumes, &volumes, &volumes)
        .unwrap();
    chart
        .set_series_data(
            0,
            &source.times[..rows],
            &source.open[..rows],
            &source.high[..rows],
            &source.low[..rows],
            &source.close[..rows],
        )
        .unwrap();
    let volume_source = matches!(
        kind,
        IndicatorKind::Vwap
            | IndicatorKind::Obv
            | IndicatorKind::Cmf { .. }
            | IndicatorKind::Mfi { .. }
            | IndicatorKind::Volume { .. }
            | IndicatorKind::VwapBands { .. }
            | IndicatorKind::Vwma { .. }
    )
    .then_some(volume);
    let outputs = chart.add_indicator_kind(0, kind.clone(), volume_source);
    assert!(!outputs.is_empty(), "{kind:?} accepted");
    (chart, outputs)
}

/// Outputs over a whitespace source equal the outputs over the source without those rows, and
/// every whitespace row (or warm-up row the whitespace delayed) is a NaN output row.
fn assert_whitespace_outputs_match(
    kind: &IndicatorKind,
    spaced: (&ChartEngine, &[SeriesId]),
    compact: (&ChartEngine, &[SeriesId]),
) {
    for (&spaced_output, &compact_output) in spaced.1.iter().zip(compact.1) {
        let (spaced_times, spaced_values) = spaced.0.data.series_data(spaced_output).unwrap();
        let (compact_times, compact_values) = compact.0.data.series_data(compact_output).unwrap();
        let expected = compact_times
            .iter()
            .copied()
            .zip(compact_values[3].iter().copied())
            .collect::<HashMap<_, _>>();
        for (time, value) in spaced_times.iter().zip(spaced_values[3]) {
            match expected.get(time) {
                Some(expected) if expected.is_nan() => {
                    assert!(value.is_nan(), "{kind:?} at {time}: {value} != NaN")
                }
                Some(expected) => assert!(
                    (value - expected).abs() < 1e-9,
                    "{kind:?} at {time}: {value} != {expected}"
                ),
                None => assert!(
                    value.is_nan(),
                    "{kind:?} at {time}: whitespace emitted {value}"
                ),
            }
        }
        for time in compact_times {
            assert!(
                spaced_times.contains(time),
                "{kind:?} lost output row {time}"
            );
        }
    }
}

#[test]
fn whitespace_source_rows_never_poison_any_indicator_binding() {
    let rows = 60;
    let spaced = WhitespaceSource::new(rows, &[7, 18, 19, 31, 45, 58, 59]);
    let compact = spaced.compacted();
    let filled = WhitespaceSource::new(rows, &[7, 18, 19, 45, 58, 59]);
    let filled_compact = filled.compacted();
    for kind in every_indicator_kind_with_conventions() {
        let (reference, reference_outputs) = whitespace_chart(&kind, &compact, compact.times.len());

        // Full rebuild over mid-history and trailing whitespace.
        let (full, full_outputs) = whitespace_chart(&kind, &spaced, rows);
        assert_whitespace_outputs_match(
            &kind,
            (&full, &full_outputs),
            (&reference, &reference_outputs),
        );

        // Streaming: whitespace rows arrive through the ordinary update path.
        let (mut streamed, streamed_outputs) = whitespace_chart(&kind, &spaced, 10);
        for row in 10..rows {
            assert!(streamed.update_series_bar(
                0,
                spaced.times[row],
                [
                    spaced.open[row],
                    spaced.high[row],
                    spaced.low[row],
                    spaced.close[row],
                ],
            ));
        }
        assert_whitespace_outputs_match(
            &kind,
            (&streamed, &streamed_outputs),
            (&reference, &reference_outputs),
        );

        // A whitespace slot later filled with a real bar resumes from that row.
        assert!(streamed.update_series_bar(
            0,
            filled.times[31],
            [
                filled.open[31],
                filled.high[31],
                filled.low[31],
                filled.close[31],
            ],
        ));
        let (filled_reference, filled_outputs) =
            whitespace_chart(&kind, &filled_compact, filled_compact.times.len());
        assert_whitespace_outputs_match(
            &kind,
            (&streamed, &streamed_outputs),
            (&filled_reference, &filled_outputs),
        );
    }
}

#[test]
fn macd_histogram_colors_match_a_full_rebuild_for_every_convention_and_whitespace() {
    let spaced = WhitespaceSource::new(40, &[9, 22, 23]);
    for seed in [IndicatorSeed::Sma, IndicatorSeed::FirstValue] {
        let kind = IndicatorKind::Macd {
            fast: 3,
            slow: 6,
            signal: 4,
            seed,
            histogram_multiplier: 2.0,
        };
        let (mut streamed, outputs) = whitespace_chart(&kind, &spaced, 5);
        for row in 5..40 {
            streamed.update_series_bar(
                0,
                spaced.times[row],
                [
                    spaced.open[row],
                    spaced.high[row],
                    spaced.low[row],
                    spaced.close[row],
                ],
            );
        }
        let (full, full_outputs) = whitespace_chart(&kind, &spaced, 40);
        let rows = full.data.series_data(full_outputs[2]).unwrap().1[3].len();
        assert_eq!(
            streamed.data.series_data(outputs[2]).unwrap().1[3].len(),
            rows
        );
        for row in 0..rows {
            let color = |chart: &ChartEngine, id| {
                chart
                    .data
                    .point_color(
                        id,
                        aeris_charts_core::model::data_layer::PointColorChannel::Body,
                        row,
                    )
                    .unwrap_or(aeris_charts_core::model::data_layer::POINT_COLOR_ABSENT)
            };
            assert_eq!(
                color(&streamed, outputs[2]),
                color(&full, full_outputs[2]),
                "{seed:?} histogram row {row}"
            );
        }
    }
}

#[test]
fn convention_preset_expands_to_explicit_parameters_and_matches_the_formulas() {
    let china = aeris_charts_indicators::IndicatorConvention::China;
    let macd = IndicatorKind::Macd {
        fast: 12,
        slow: 26,
        signal: 9,
        seed: IndicatorSeed::Sma,
        histogram_multiplier: 1.0,
    }
    .with_convention(china);
    assert_eq!(
        macd,
        IndicatorKind::Macd {
            fast: 12,
            slow: 26,
            signal: 9,
            seed: IndicatorSeed::FirstValue,
            histogram_multiplier: 2.0,
        }
    );
    assert_eq!(
        IndicatorKind::Bollinger {
            period: 20,
            deviation: 2.0,
            estimator: DeviationEstimator::Population,
        }
        .with_convention(china),
        IndicatorKind::Bollinger {
            period: 20,
            deviation: 2.0,
            estimator: DeviationEstimator::Sample,
        }
    );
    assert_eq!(
        IndicatorKind::Sma { period: 5 }.with_convention(china),
        IndicatorKind::Sma { period: 5 }
    );
    let tradingview = macd
        .clone()
        .with_convention(aeris_charts_indicators::IndicatorConvention::TradingView);
    assert_eq!(
        tradingview,
        IndicatorKind::Macd {
            fast: 12,
            slow: 26,
            signal: 9,
            seed: IndicatorSeed::Sma,
            histogram_multiplier: 1.0,
        }
    );

    let source = WhitespaceSource::new(60, &[]);
    let (chart, outputs) = whitespace_chart(&macd, &source, 60);
    let expected = aeris_charts_indicators::macd_with(
        &source.close,
        12,
        26,
        9,
        IndicatorSeed::FirstValue,
        2.0,
    );
    // First-value seeds start every MACD output at the first bar.
    let histogram = chart.data.series_data(outputs[2]).unwrap().1[3];
    assert_eq!(histogram.len(), 60);
    for (actual, expected) in histogram.iter().zip(&expected) {
        assert!((actual - expected.histogram.unwrap()).abs() < 1e-12);
    }
    let info = chart.indicator_info(outputs[2]).unwrap();
    assert_eq!(info.parameters.seed, Some(IndicatorSeed::FirstValue));
    assert_eq!(info.parameters.histogram_multiplier, Some(2.0));
    let schema = ChartEngine::indicator_schema(&macd);
    let seed = schema
        .parameters
        .iter()
        .find(|parameter| parameter.name == "seed")
        .unwrap();
    assert_eq!(seed.parameter_type, IndicatorParameterType::Choice);
    assert_eq!(seed.choices, ["sma", "first_value"]);
    assert_eq!(seed.default, serde_json::json!("first_value"));
}

#[test]
fn kdj_is_one_three_output_oscillator_binding() {
    let source = WhitespaceSource::new(30, &[]);
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    chart
        .set_series_data(
            0,
            &source.times,
            &source.open,
            &source.high,
            &source.low,
            &source.close,
        )
        .unwrap();
    assert!(chart.add_kdj(0, 0, 3, 3).is_empty());
    assert!(chart.add_kdj(0, 9, 3, 0).is_empty());
    let outputs = chart.add_kdj(0, 9, 3, 3);
    assert_eq!(outputs.len(), 3);
    let expected = aeris_charts_indicators::kdj(&source.high, &source.low, &source.close, 9, 3, 3);
    for (index, &output) in outputs.iter().enumerate() {
        let info = chart.indicator_info(output).unwrap();
        assert_eq!(info.kind, "kdj");
        assert_eq!(info.output_name, ["K", "D", "J"][index]);
        assert_eq!(
            (info.parameters.k_smoothing, info.parameters.d_smoothing),
            (Some(3), Some(3))
        );
        assert_eq!(info.warmup_bars, 8);
        let series = chart
            .series
            .iter()
            .find(|series| series.id == output)
            .unwrap();
        assert_eq!(series.pane_index, 1);
        assert_eq!(series.title, "KDJ 9 3 3");
        let values = chart.data.series_data(output).unwrap().1[3];
        assert_eq!(values.len(), 22);
        for (offset, value) in values.iter().enumerate() {
            let point = expected[8 + offset];
            let expected = [point.k, point.d, point.j][index].unwrap();
            assert!((value - expected).abs() < 1e-12);
        }
    }
    assert_eq!(
        ChartEngine::indicator_schema(&IndicatorKind::Kdj {
            period: 9,
            k_smoothing: 3,
            d_smoothing: 3,
            seed: aeris_charts_indicators::KdjSeed::Fifty,
        })
        .outputs
        .iter()
        .map(|output| output.name.as_str())
        .collect::<Vec<_>>(),
        ["K", "D", "J"]
    );
}

#[test]
fn kdj_seed_is_an_explicit_parameter_with_the_textbook_default() {
    let source = WhitespaceSource::new(30, &[]);
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    chart
        .set_series_data(
            0,
            &source.times,
            &source.open,
            &source.high,
            &source.low,
            &source.close,
        )
        .unwrap();
    let textbook = chart.add_kdj(0, 9, 3, 3);
    assert_eq!(
        chart
            .indicator_info(textbook[0])
            .unwrap()
            .parameters
            .kdj_seed,
        Some(aeris_charts_indicators::KdjSeed::Fifty)
    );
    let china = IndicatorKind::Kdj {
        period: 9,
        k_smoothing: 3,
        d_smoothing: 3,
        seed: aeris_charts_indicators::KdjSeed::Fifty,
    }
    .with_convention(aeris_charts_indicators::IndicatorConvention::China);
    let first_value = chart.add_indicator_kind(0, china.clone(), None);
    let info = chart.indicator_info(first_value[1]).unwrap();
    assert_eq!(
        info.parameters.kdj_seed,
        Some(aeris_charts_indicators::KdjSeed::FirstValue)
    );
    assert_eq!(
        (info.warmup_bars, info.convergence_bars),
        (0, Some(44)),
        "the formula-language RSV uses the bars available, so values start at the first bar"
    );
    let expected = aeris_charts_indicators::kdj_with_seed(
        &source.high,
        &source.low,
        &source.close,
        9,
        3,
        3,
        aeris_charts_indicators::KdjSeed::FirstValue,
    );
    let d = chart.data.series_data(first_value[1]).unwrap().1[3];
    assert_eq!(d.len(), source.close.len());
    let first_rsv = 100.0 * (source.close[0] - source.low[0]) / (source.high[0] - source.low[0]);
    assert!(
        (d[0] - first_rsv).abs() < 1e-12,
        "D starts at the first RSV"
    );
    for (row, value) in d.iter().enumerate() {
        assert!((value - expected[row].d.unwrap()).abs() < 1e-12);
    }
    let textbook_d = chart.data.series_data(textbook[1]).unwrap().1[3];
    assert_eq!(textbook_d.len(), source.close.len() - 8);
    assert_ne!(textbook_d[0], d[8]);

    let schema = ChartEngine::indicator_schema(&china);
    let seed = schema
        .parameters
        .iter()
        .find(|parameter| parameter.name == "seed")
        .unwrap();
    assert_eq!(seed.default, serde_json::json!("first_value"));
    assert_eq!(seed.choices, ["fifty", "first_value"]);
}

#[test]
fn china_kdj_starts_at_the_first_loaded_bar_through_streaming_trims_and_corrections() {
    use aeris_charts_indicators::{kdj_with_seed, IndicatorConvention, KdjSeed};
    let source = WhitespaceSource::new(64, &[]);
    let bar = |row: usize| {
        [
            source.open[row],
            source.high[row],
            source.low[row],
            source.close[row],
        ]
    };
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    // History begins at row 16; rows 0..16 are prepended later.
    chart
        .set_series_data(
            0,
            &source.times[16..17],
            &source.open[16..17],
            &source.high[16..17],
            &source.low[16..17],
            &source.close[16..17],
        )
        .unwrap();
    let kdj = chart.add_indicator_kind(
        0,
        IndicatorKind::Kdj {
            period: 9,
            k_smoothing: 3,
            d_smoothing: 3,
            seed: KdjSeed::Fifty,
        }
        .with_convention(IndicatorConvention::China),
        None,
    );
    // Every loaded bar has K, D and J, equal to the formula over exactly the loaded bars: the
    // RSV of the first N-1 bars uses the bars available, so the start moves with the history.
    let assert_formula = |chart: &ChartEngine, label: &str| {
        let (times, values) = chart.data.series_data(0).unwrap();
        let expected = kdj_with_seed(
            values[1],
            values[2],
            values[3],
            9,
            3,
            3,
            KdjSeed::FirstValue,
        );
        for (output, &id) in kdj.iter().enumerate() {
            let (output_times, output_values) = chart.data.series_data(id).unwrap();
            assert_eq!(
                output_times, times,
                "{label}: output {output} covers every bar"
            );
            for (row, point) in expected.iter().enumerate() {
                let expected = [point.k, point.d, point.j][output].unwrap();
                assert!(
                    (output_values[3][row] - expected).abs() < 1e-9,
                    "{label}: output {output} row {row}: {} != {expected}",
                    output_values[3][row]
                );
            }
        }
    };
    assert_formula(&chart, "one bar");
    for row in 17..48 {
        assert!(chart.update_series_bar(0, source.times[row], bar(row)));
        assert_formula(&chart, "append");
    }
    let mut current = bar(47);
    current[1] += 2.0;
    current[3] += 1.5;
    assert!(chart.update_series_bar(0, source.times[47], current));
    assert_formula(&chart, "current-bar replacement");

    // Retention re-seeds at the new first bar with a partial window, like loading less history.
    assert!(chart.set_series_max_points(0, Some(24)));
    assert!(chart.data.series_data(0).unwrap().0.len() <= 24);
    assert_formula(&chart, "trim");
    for row in 48..56 {
        assert!(chart.update_series_bar(0, source.times[row], bar(row)));
        assert_formula(&chart, "append with retention");
    }

    // A historical correction inside the first window, then prepended history.
    let first_time = chart.data.series_data(0).unwrap().0[2] as f64;
    let outcome = chart.merge_series_bar(
        0,
        first_time,
        SeriesBarPatch {
            low: Some(80.0),
            ..SeriesBarPatch::default()
        },
        None,
    );
    assert!(
        matches!(outcome, SeriesUpdateOutcome::Applied),
        "{outcome:?}"
    );
    assert_formula(&chart, "historical correction");
    assert!(chart.set_series_max_points(0, None));
    chart
        .set_series_data(
            0,
            &source.times,
            &source.open,
            &source.high,
            &source.low,
            &source.close,
        )
        .unwrap();
    assert_formula(&chart, "prepended history");
}

#[test]
fn warmup_query_reports_rows_before_values_and_seed_convergence_through_chains() {
    let source = WhitespaceSource::new(40, &[]);
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    chart
        .set_series_data(
            0,
            &source.times,
            &source.open,
            &source.high,
            &source.low,
            &source.close,
        )
        .unwrap();
    let macd = chart.add_macd(0, 12, 26, 9);
    let info = chart.indicator_info(macd[1]).unwrap();
    assert_eq!((info.warmup_bars, info.convergence_bars), (33, Some(154)));
    let rsi = chart.add_rsi(0, 14).unwrap();
    let smoothed = chart.add_sma(rsi, 5).unwrap();
    let info = chart.indicator_info(smoothed).unwrap();
    // SMA(5) of RSI(14): 14 + 4 rows before the first value; RSI converges after 108 rows.
    assert_eq!((info.warmup_bars, info.convergence_bars), (18, Some(112)));
    let vwap = chart.add_vwap(0, None).unwrap();
    let info = chart.indicator_info(vwap).unwrap();
    assert_eq!((info.warmup_bars, info.convergence_bars), (0, None));
    // The actual first output row agrees with the reported warm-up.
    let first_time = chart.data.series_data(smoothed).unwrap().0[0];
    assert_eq!(first_time as f64, source.times[18]);
}

#[test]
fn amount_weighted_vwap_divides_turnover_by_volume_and_validates_its_inputs() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    let volume = chart.add_series(SeriesKind::Histogram);
    let amount = chart.add_series(SeriesKind::Line);
    let times = [0.0, 60.0, 120.0, 180.0, 240.0];
    let price = [10.0, 11.0, 12.0, 13.0, 14.0];
    chart
        .set_series_data(0, &times, &price, &price, &price, &price)
        .unwrap();
    // Volume misses time 180 and is zero at 60; amount misses time 240.
    let volume_times = [0.0, 60.0, 120.0, 240.0];
    let volumes = [10.0, 0.0, 30.0, 20.0];
    chart
        .set_series_data(
            volume,
            &volume_times,
            &volumes,
            &volumes,
            &volumes,
            &volumes,
        )
        .unwrap();
    let amount_times = [0.0, 60.0, 120.0, 180.0];
    let amounts = [100.0, 0.0, 330.0, 50.0];
    chart
        .set_series_data(
            amount,
            &amount_times,
            &amounts,
            &amounts,
            &amounts,
            &amounts,
        )
        .unwrap();

    assert!(chart
        .add_indicator_kind_with_sources(
            0,
            IndicatorInputSource::Close,
            IndicatorKind::Vwap,
            None,
            Some(amount),
        )
        .is_empty());
    assert!(chart
        .add_indicator_kind_with_sources(
            0,
            IndicatorInputSource::Close,
            IndicatorKind::Vwap,
            Some(volume),
            Some(volume),
        )
        .is_empty());
    assert!(chart
        .add_indicator_kind_with_sources(
            0,
            IndicatorInputSource::Close,
            IndicatorKind::Obv,
            Some(volume),
            Some(amount),
        )
        .is_empty());

    let vwap = chart.add_vwap_with_amount(0, volume, amount).unwrap();
    let values = chart.data.series_data(vwap).unwrap().1[3].to_vec();
    assert_eq!(values[..4], [10.0, 10.0, 10.75, 10.75]);
    // Row 240 has volume but no turnover, so it contributes nothing.
    assert_eq!(values[4], 10.75);
    assert_eq!(
        chart.indicator_info(vwap).unwrap().amount_source,
        Some(amount)
    );
    assert_eq!(chart.indicator_bindings()[0].amount_source, Some(amount));

    // A live amount correction flows into the binding like a volume correction does.
    chart.update_series_bar(amount, 240.0, [300.0; 4]);
    let values = chart.data.series_data(vwap).unwrap().1[3].to_vec();
    assert_eq!(values[4], (430.0 + 300.0) / 60.0);

    // Removing the amount series removes the binding it feeds.
    assert!(chart.remove_series(amount));
    assert!(chart.indicator_bindings().is_empty());
    assert!(chart.series_kind(vwap).is_none());
}

#[test]
fn weight_series_updates_resume_at_the_source_row_of_their_timestamp() {
    // The turnover and volume columns carry history before the price series' first row (for
    // example the previous session's last minutes). A live update to their last row must
    // recompute the price row with that timestamp, not the row at the same index.
    let build = |amount_tail: f64, volume_tail: f64| {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        let volume = chart.add_series(SeriesKind::Histogram);
        let amount = chart.add_series(SeriesKind::Line);
        let price_times = [180.0, 240.0, 300.0];
        let price = [10.0, 11.0, 12.0];
        chart
            .set_series_data(0, &price_times, &price, &price, &price, &price)
            .unwrap();
        let weight_times = [0.0, 60.0, 120.0, 180.0, 240.0, 300.0];
        let volumes = [1.0, 1.0, 1.0, 10.0, 20.0, volume_tail];
        let amounts = [9.0, 9.0, 9.0, 100.0, 220.0, amount_tail];
        chart
            .set_series_data(
                volume,
                &weight_times,
                &volumes,
                &volumes,
                &volumes,
                &volumes,
            )
            .unwrap();
        chart
            .set_series_data(
                amount,
                &weight_times,
                &amounts,
                &amounts,
                &amounts,
                &amounts,
            )
            .unwrap();
        let average = chart
            .add_vwap_with_amount(0, volume, amount)
            .expect("amount-weighted VWAP");
        let typical = chart.add_vwap(0, Some(volume)).expect("typical VWAP");
        (chart, volume, amount, average, typical)
    };
    let values =
        |chart: &ChartEngine, id: SeriesId| chart.data.series_data(id).unwrap().1[3].to_vec();

    let (mut live, volume, amount, average, typical) = build(360.0, 30.0);
    assert!(live.update_series_bar(amount, 300.0, [390.0; 4]));
    assert!(live.update_series_bar(volume, 300.0, [40.0; 4]));
    let (fresh, _, _, fresh_average, fresh_typical) = build(390.0, 40.0);
    assert_eq!(values(&live, average), values(&fresh, fresh_average));
    assert_eq!(values(&live, typical), values(&fresh, fresh_typical));
    assert_eq!(*values(&live, average).last().unwrap(), 710.0 / 70.0);
}

/// Deterministic xorshift for the randomized engine-path equivalence test.
struct MutationRng(u64);

impl MutationRng {
    fn below(&mut self, bound: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % bound as u64) as usize
    }
}

#[test]
fn random_engine_mutations_keep_every_indicator_equal_to_a_fresh_install() {
    // Candle ticks with the weights lagging, leading, or missing; whitespace ticks; historical
    // corrections and late bars; weight gap fills and weight pops; source pops; retention trims;
    // scalar-input switches; and a batch that closes the current bar and opens the next. After
    // every mutation each output must equal the same study installed fresh from the current data.
    let inputs = [
        IndicatorInputSource::Close,
        IndicatorInputSource::Hl2,
        IndicatorInputSource::Hlc3,
        IndicatorInputSource::Ohlc4,
        IndicatorInputSource::Hlcc4,
    ];
    for (kind_index, kind) in every_indicator_kind().into_iter().enumerate() {
        for seed in 1..=2_u64 {
            let mut rng = MutationRng(seed * 0x51_7cc1 + kind_index as u64 * 977 + 1);
            let with_amount = matches!(kind, IndicatorKind::Vwap) && seed == 2;
            let input = inputs[rng.below(inputs.len())];
            let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
            let volume = chart.add_series(SeriesKind::Histogram);
            let amount = chart.add_series(SeriesKind::Histogram);
            let rows = 30 + rng.below(10);
            let times = (0..rows)
                .map(|row| row as f64 * 3_600.0)
                .collect::<Vec<_>>();
            let bars = (0..rows)
                .map(|row| swinging_bar(row, 0))
                .collect::<Vec<_>>();
            let column = |index: usize| bars.iter().map(|bar| bar[index]).collect::<Vec<_>>();
            chart
                .set_series_data(0, &times, &column(0), &column(1), &column(2), &column(3))
                .unwrap();
            // The second seed starts from weight timelines that miss every fifth bar.
            let weight_times = times
                .iter()
                .enumerate()
                .filter(|(row, _)| seed == 1 || row % 5 != 2)
                .map(|(_, &time)| time)
                .collect::<Vec<_>>();
            let volumes = weight_times
                .iter()
                .map(|time| (time / 3_600.0) % 9.0 + 1.0)
                .collect::<Vec<_>>();
            let amounts = volumes
                .iter()
                .map(|volume| volume * 97.0)
                .collect::<Vec<_>>();
            for (id, values) in [(volume, &volumes), (amount, &amounts)] {
                chart
                    .set_series_data(id, &weight_times, values, values, values, values)
                    .unwrap();
            }
            let outputs = chart.add_indicator_kind_with_sources(
                0,
                input,
                kind.clone(),
                indicator_reads_volume(&kind).then_some(volume),
                with_amount.then_some(amount),
            );
            assert!(!outputs.is_empty());
            let binding = chart.indicators.len() - 1;
            let mut last_time = (rows - 1) as f64 * 3_600.0;
            for step in 0..160 {
                let label = format!("{kind:?} {input:?} seed {seed} step {step}");
                let weight = (rng.below(20) + 1) as f64;
                match rng.below(15) {
                    0..=2 => {
                        last_time += 3_600.0 * (1 + rng.below(2)) as f64;
                        let row = (last_time / 3_600.0) as usize;
                        chart.update_series_bar(0, last_time, swinging_bar(row, step));
                        if rng.below(3) != 0 {
                            chart.update_series_bar(volume, last_time, [weight; 4]);
                            chart.update_series_bar(amount, last_time, [weight * 97.0; 4]);
                        }
                    }
                    3 => {
                        last_time += 3_600.0;
                        let row = (last_time / 3_600.0) as usize;
                        chart.update_series_bar(volume, last_time, [weight; 4]);
                        assert_binding_matches_fresh_install(&chart, binding, &label);
                        chart.update_series_bar(amount, last_time, [weight * 97.0; 4]);
                        assert_binding_matches_fresh_install(&chart, binding, &label);
                        chart.update_series_bar(0, last_time, swinging_bar(row, step));
                    }
                    4..=6 => {
                        let row = (last_time / 3_600.0) as usize;
                        chart.update_series_bar(0, last_time, swinging_bar(row, step + 7));
                        if rng.below(2) == 0 {
                            chart.update_series_bar(volume, last_time, [weight; 4]);
                        }
                    }
                    7 => {
                        chart.update_series_bar(0, last_time, [f64::NAN; 4]);
                    }
                    8 => {
                        let (times, _) = chart.data.series_data(0).unwrap();
                        let row = rng.below(times.len());
                        let late = if rng.below(2) == 0 { 0.0 } else { 1_800.0 };
                        let time = times[row] as f64 + late;
                        if time < last_time {
                            chart.update_series_bar(0, time, swinging_bar(row, step + 3));
                        }
                    }
                    9 => {
                        let (times, _) = chart.data.series_data(0).unwrap();
                        let time = times[rng.below(times.len())] as f64;
                        chart.update_series_bar(volume, time, [weight; 4]);
                        if rng.below(2) == 0 {
                            chart.update_series_bar(amount, time, [weight * 97.0; 4]);
                        }
                    }
                    10 => {
                        let (times, _) = chart.data.series_data(0).unwrap();
                        if times.len() > 20 {
                            chart.series_pop(0, 1 + rng.below(2)).unwrap();
                            let (times, _) = chart.data.series_data(0).unwrap();
                            last_time = *times.last().unwrap() as f64;
                        }
                    }
                    11 => {
                        if rng.below(4) == 0 {
                            let (times, _) = chart.data.series_data(0).unwrap();
                            let cap = times.len().saturating_sub(3).max(20);
                            chart.set_series_max_points(0, Some(cap));
                        } else {
                            let next = inputs[rng.below(inputs.len())];
                            let output = chart.indicators[binding].outputs[0];
                            chart.set_indicator_input_source(output, next);
                        }
                    }
                    12 => {
                        let (weight_times, _) = chart.data.series_data(volume).unwrap();
                        if weight_times.len() > 20 {
                            chart.series_pop(volume, 1).unwrap();
                        }
                    }
                    13 => {
                        let row = (last_time / 3_600.0) as usize;
                        let bars = [swinging_bar(row, step + 11), swinging_bar(row + 1, step)];
                        let column =
                            |index: usize| bars.iter().map(|bar| bar[index]).collect::<Vec<_>>();
                        chart.update_series_bars_sanitized(
                            0,
                            vec![last_time as i64, last_time as i64 + 3_600],
                            column(0),
                            column(1),
                            column(2),
                            column(3),
                        );
                        last_time += 3_600.0;
                    }
                    _ => {}
                }
                assert_binding_matches_fresh_install(&chart, binding, &label);
            }
        }
    }
}
