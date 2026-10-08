//! Headless-engine unit tests (extracted from `lib.rs`; `super` is the crate root).

use super::*;
use aeris_charts_render::canvas2d::{Canvas2d, Viewport, execute};
use aeris_charts_render::color::Color;

#[derive(Default)]
struct CountingCanvas {
    calls: usize,
}

/// VAL-CROSS-005: one host replay clock must project ordinary studies and trade-derived
/// annotations from the same eligible history, including after seeking backwards.
#[test]
fn replay_seek_structure_auction_and_custom_study_match_fresh_prefix() {
    struct TwoBarAverage;
    impl CustomStudyRuntime for TwoBarAverage {
        fn compute(
            &mut self,
            input: CustomStudyInput<'_>,
            out: &mut [Vec<f64>],
        ) -> Result<(), CustomStudyFault> {
            for row in input.from..input.times.len() {
                out[0].push(if row == 0 {
                    f64::NAN
                } else {
                    (input.close[row - 1] + input.close[row]) / 2.0
                });
            }
            Ok(())
        }
    }

    fn setup(
        times: &[f64],
        highs: &[f64],
        lows: &[f64],
        trades: Vec<FootprintTrade>,
    ) -> (ChartEngine, Vec<SeriesId>, SeriesId, NativePrimitiveId) {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        let closes = highs
            .iter()
            .zip(lows)
            .map(|(high, low)| (high + low) / 2.0)
            .collect::<Vec<_>>();
        chart
            .set_series_data(0, times, &closes, highs, lows, &closes)
            .unwrap();
        let structure =
            chart.add_indicator_kind(0, IndicatorKind::SwingPoints { left: 1, right: 1 }, None);
        chart
            .register_custom_study(
                CustomStudyDefinition {
                    type_id: "replay_two_bar_average".into(),
                    version: 1,
                    title: "Replay average".into(),
                    parameters: vec![],
                    outputs: vec![CustomStudyOutput {
                        name: "Average".into(),
                        plot: CustomStudyPlot::Line,
                        pane: CustomStudyPane::Price,
                        default_style: IndicatorOutputStyle::default(),
                    }],
                    uses_volume: false,
                },
                Box::new(|_| Ok(Box::new(TwoBarAverage))),
            )
            .unwrap();
        let custom = chart
            .add_custom_study(
                "replay_two_bar_average",
                0,
                IndicatorInputSource::Close,
                None,
                CustomStudyParams::new(),
            )
            .unwrap()[0];
        let stream = chart
            .add_trade_stream(
                "replay_auction",
                FootprintAggregationOptions {
                    tick_size: 1.0,
                    ..FootprintAggregationOptions::default()
                },
            )
            .unwrap();
        chart.set_trade_stream_trades(stream, trades).unwrap();
        let auction = chart
            .add_auction_markers(
                stream,
                0,
                AuctionMarkerOptions {
                    include_forming_bar: true,
                    ..AuctionMarkerOptions::default()
                },
            )
            .unwrap();
        (chart, structure, custom, auction)
    }

    fn output(chart: &ChartEngine, id: SeriesId) -> (Vec<i64>, Vec<Option<f64>>) {
        let (times, columns) = chart.data.series_data(id).unwrap();
        (
            times.to_vec(),
            columns[3]
                .iter()
                .map(|value| value.is_finite().then_some(*value))
                .collect(),
        )
    }

    let times = (1..=10).map(|row| row as f64 * 60.0).collect::<Vec<_>>();
    let highs = [10., 14., 12., 18., 16., 20., 19., 17., 25., 21.];
    let lows = [8., 9., 10., 15., 11., 8., 16., 13., 22., 17.];
    let trades = times
        .iter()
        .enumerate()
        .flat_map(|(index, time)| {
            [AggressorSide::Buy, AggressorSide::Sell]
                .into_iter()
                .enumerate()
                .map(move |(offset, side)| FootprintTrade {
                    timestamp_micros: *time as i64 * 1_000_000 + offset as i64,
                    price: 100.0 + index as f64,
                    volume: 2.0,
                    aggressor: side,
                    bid: None,
                    ask: None,
                    sequence: None,
                    trade_id: None,
                    conditions: 0,
                    session_id: Some(1),
                })
        })
        .collect::<Vec<_>>();
    let (mut replay, structure, custom, auction) = setup(&times, &highs, &lows, trades.clone());
    assert!(
        !replay
            .study_annotations(structure[0])
            .unwrap()
            .markers()
            .is_empty()
    );
    assert!(!replay.auction_markers_snapshot(auction).unwrap().is_empty());
    assert_eq!(output(&replay, custom).1[8], Some(21.25));

    for len in [10usize, 6, 9, 3, 8, 1, 7, 10] {
        let clock = times[len - 1] as i64 * 1_000_000 + 1;
        replay.set_replay_clock_micros(Some(clock)).unwrap();
        let eligible = trades
            .iter()
            .filter(|trade| trade.timestamp_micros <= clock)
            .cloned()
            .collect();
        let (fresh, expected_structure, expected_custom, expected_auction) =
            setup(&times[..len], &highs[..len], &lows[..len], eligible);
        for (&actual, &expected) in structure.iter().zip(&expected_structure) {
            assert_eq!(
                output(&replay, actual),
                output(&fresh, expected),
                "structure prefix {len}"
            );
        }
        assert_eq!(
            replay.study_annotations(structure[0]).unwrap(),
            fresh.study_annotations(expected_structure[0]).unwrap(),
            "annotations prefix {len}"
        );
        assert_eq!(
            output(&replay, custom),
            output(&fresh, expected_custom),
            "custom prefix {len}"
        );
        assert_eq!(
            output(&replay, custom).0.len(),
            len.saturating_sub(1),
            "custom prefix {len}"
        );
        assert_eq!(
            replay.auction_markers_snapshot(auction).unwrap().len(),
            len * 2,
            "each eligible bar has both unfinished-auction sides"
        );
        assert_eq!(
            replay.auction_markers_snapshot(auction).unwrap(),
            fresh.auction_markers_snapshot(expected_auction).unwrap(),
            "auction prefix {len}"
        );
    }
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
    assert!(chart.add_accumulation_distribution(0, histogram).is_some());
    assert!(chart.add_price_volume_trend(0, histogram).is_some());
    assert!(chart.add_chaikin_oscillator(0, histogram, 3, 10).is_some());
    assert_eq!(chart.add_klinger(0, histogram, 3, 10, 4).len(), 2);
    assert!(chart.add_kama(0, 10, 2, 30).is_some());
    assert!(chart.add_mcginley(0, 10).is_some());
    assert_eq!(chart.add_linear_regression(0, 10, 2.0).len(), 3);
    assert_eq!(
        chart
            .add_kst(0, [10, 15, 20, 30], [10, 10, 10, 15], 9)
            .len(),
        2
    );
    assert_eq!(chart.add_tsi(0, 25, 13, 13).len(), 2);
    assert!(chart.add_mass_index(0, 9, 25).is_some());
    assert_eq!(chart.add_vortex(0, 14).len(), 2);
    assert!(chart.add_choppiness(0, 14).is_some());
    assert_eq!(chart.add_atr_bands(0, 14, 2.0).len(), 3);
    assert!(chart.add_relative_volume(0, histogram, 5).is_some());
    assert!(chart.add_elder_force(0, histogram, 5).is_some());
    assert!(chart.add_ease_of_movement(0, histogram, 5, 100.0).is_some());
    assert!(chart.add_historical_volatility(0, 5, 252.0).is_some());
    assert_eq!(chart.add_trix(0, 5, 3).len(), 2);
    assert!(chart.add_coppock_curve(0, 14, 11, 10).is_some());
    assert_eq!(chart.add_fisher_transform(0, 10).len(), 2);
    assert!(chart.add_ultimate_oscillator(0, 7, 14, 28).is_some());
    assert_eq!(chart.add_volume_oscillator(0, histogram, 3, 10, 4).len(), 3);
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
            IndicatorKind::AccumulationDistribution,
            IndicatorKind::PriceVolumeTrend,
            IndicatorKind::ChaikinOscillator { fast: 3, slow: 10 },
            IndicatorKind::Klinger {
                fast: 3,
                slow: 10,
                signal: 4,
            },
            IndicatorKind::Kama {
                period: 10,
                fast: 2,
                slow: 30,
            },
            IndicatorKind::McGinley { period: 10 },
            IndicatorKind::LinearRegression {
                period: 10,
                deviation: 2.0,
            },
            IndicatorKind::Kst {
                roc: [10, 15, 20, 30],
                smoothing: [10, 10, 10, 15],
                signal: 9,
            },
            IndicatorKind::Tsi {
                long: 25,
                short: 13,
                signal: 13,
            },
            IndicatorKind::MassIndex {
                ema_period: 9,
                sum_period: 25,
            },
            IndicatorKind::Vortex { period: 14 },
            IndicatorKind::Choppiness { period: 14 },
            IndicatorKind::AtrBands {
                period: 14,
                multiplier: 2.0,
            },
            IndicatorKind::RelativeVolume { period: 5 },
            IndicatorKind::ElderForce { period: 5 },
            IndicatorKind::EaseOfMovement {
                period: 5,
                divisor: 100.0
            },
            IndicatorKind::HistoricalVolatility {
                period: 5,
                annualization: 252.0,
            },
            IndicatorKind::Trix {
                period: 5,
                signal: 3
            },
            IndicatorKind::CoppockCurve {
                long: 14,
                short: 11,
                smoothing: 10
            },
            IndicatorKind::FisherTransform { period: 10 },
            IndicatorKind::UltimateOscillator {
                short: 7,
                medium: 14,
                long: 28
            },
            IndicatorKind::VolumeOscillator {
                fast: 3,
                slow: 10,
                signal: 4
            },
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
fn all_i2_studies_round_trip_together_in_v3_with_non_default_parameters() {
    let times = (0..96).map(|row| row as f64 * 60.0).collect::<Vec<_>>();
    let close = (0..96)
        .map(|row| 100.0 + row as f64 * 0.3 + (row as f64 * 0.21).sin())
        .collect::<Vec<_>>();
    let high = close.iter().map(|value| value + 1.0).collect::<Vec<_>>();
    let low = close.iter().map(|value| value - 1.0).collect::<Vec<_>>();
    let volume_values = (0..96)
        .map(|row| 20.0 + (row % 7) as f64)
        .collect::<Vec<_>>();
    let install_sources = || {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        chart
            .set_series_data(0, &times, &close, &high, &low, &close)
            .unwrap();
        let volume = chart.add_series(SeriesKind::Histogram);
        chart
            .set_series_data(
                volume,
                &times,
                &volume_values,
                &volume_values,
                &volume_values,
                &volume_values,
            )
            .unwrap();
        (chart, volume)
    };
    let (mut chart, volume) = install_sources();
    let kinds = [
        IndicatorKind::Klinger {
            fast: 4,
            slow: 9,
            signal: 3,
        },
        IndicatorKind::Kama {
            period: 7,
            fast: 3,
            slow: 12,
        },
        IndicatorKind::McGinley { period: 9 },
        IndicatorKind::LinearRegression {
            period: 8,
            deviation: 1.5,
        },
        IndicatorKind::Kst {
            roc: [3, 5, 7, 9],
            smoothing: [2, 3, 4, 5],
            signal: 4,
        },
        IndicatorKind::Tsi {
            long: 7,
            short: 4,
            signal: 5,
        },
        IndicatorKind::MassIndex {
            ema_period: 4,
            sum_period: 7,
        },
        IndicatorKind::Vortex { period: 8 },
        IndicatorKind::Choppiness { period: 8 },
        IndicatorKind::AtrBands {
            period: 8,
            multiplier: 1.75,
        },
    ];
    let mut original_outputs = Vec::new();
    for (index, (kind, expected_outputs)) in
        kinds.iter().zip([2, 1, 1, 3, 2, 2, 1, 2, 1, 3]).enumerate()
    {
        let outputs = chart.add_indicator_kind(0, kind.clone(), (index == 0).then_some(volume));
        assert_eq!(outputs.len(), expected_outputs, "{kind:?}");
        original_outputs.push(outputs);
    }
    assert!(chart.set_indicator_output_style(
        original_outputs[3][1],
        IndicatorOutputStyle {
            visible: false,
            line_color: Some("#123456".into()),
            ..Default::default()
        }
    ));
    let original_bindings = chart.indicator_bindings();
    let document = chart.export_state_json().unwrap();
    let json: serde_json::Value = serde_json::from_str(&document).unwrap();
    assert_eq!(json["schema_version"], 3);
    assert_eq!(json["indicators"].as_array().unwrap().len(), kinds.len());

    let (mut restored, restored_volume) = install_sources();
    assert_eq!(restored_volume, volume);
    restored.import_state_json(&document).unwrap();
    let bindings = restored.indicator_bindings();
    assert_eq!(bindings.len(), kinds.len());
    for (index, ((kind, original), binding)) in kinds
        .iter()
        .zip(&original_bindings)
        .zip(&bindings)
        .enumerate()
    {
        assert_eq!(&binding.kind, kind, "study {index}");
        assert_eq!(binding.outputs, original.outputs, "study {index}");
        assert_eq!(binding.styles, original.styles, "study {index}");
        assert_eq!(
            binding.volume_source,
            (index == 0).then_some(restored_volume),
            "study {index}"
        );
        for &output in &original_outputs[index] {
            let before = chart.data.series_data(output).unwrap();
            let after = restored.data.series_data(output).unwrap();
            assert_eq!(before.0, after.0, "output {output} timestamps");
            for (left, right) in before.1.into_iter().zip(after.1) {
                assert_eq!(left.len(), right.len(), "output {output} rows");
                assert!(
                    left.iter()
                        .zip(right)
                        .all(|(a, b)| a == b || a.is_nan() && b.is_nan()),
                    "output {output} values"
                );
            }
        }
    }
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
fn histogram_autoscale_includes_the_column_base() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    let volume = chart.add_series(SeriesKind::Histogram);
    chart.set_series_visible(0, false);
    chart
        .set_series_data(
            volume,
            &[1.0, 2.0, 3.0],
            &[120.0, 150.0, 180.0],
            &[120.0, 150.0, 180.0],
            &[120.0, 150.0, 180.0],
            &[120.0, 150.0, 180.0],
        )
        .unwrap();
    chart.time_scale.set_width(800.0);
    chart.fit_content();
    chart.autoscale_visible();
    let range = chart.panes[0].price_scale.price_range().unwrap();
    assert_eq!((range.min_value(), range.max_value()), (0.0, 180.0));

    // A base above the data extends the range upward instead.
    chart
        .series
        .iter_mut()
        .find(|series| series.id == volume)
        .unwrap()
        .base = 200.0;
    chart.autoscale_visible();
    let range = chart.panes[0].price_scale.price_range().unwrap();
    assert_eq!((range.min_value(), range.max_value()), (120.0, 200.0));
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
fn chart_time_zone_rebuilds_tick_weights_and_formats_live_clock() {
    let mut chart = ChartEngine::new(300.0, 200.0, 1.0);
    chart
        .set_series_data(
            0,
            &[1_767_329_940.0, 1_767_330_000.0],
            &[100.0, 101.0],
            &[101.0, 102.0],
            &[99.0, 100.0],
            &[100.5, 101.5],
        )
        .unwrap();
    assert_eq!(chart.time_zone_id(), DEFAULT_TIME_ZONE);
    assert!(chart.set_time_zone("America/New_York").unwrap());
    assert_eq!(chart.time_zone_id(), "America/New_York");
    chart.set_date_format("yyyy-MM-dd");
    assert_eq!(
        chart.format_crosshair_ts(1_767_229_200),
        "2025-12-31   20:00"
    );
    let marks = chart.time_marks(1.0);
    assert!(marks.iter().any(|&(index, weight)| {
        index == 1 && weight == aeris_charts_core::scale::time_tick_marks::TickMarkWeight::Day as u8
    }));
    assert_eq!(
        chart.time_zone_clock_text(1_784_116_800, true),
        "08:00:00 EDT"
    );
    assert!(!chart.set_time_zone("America/New_York").unwrap());
    assert!(chart.set_time_zone("Mars/Olympus_Mons").is_err());
}

#[test]
fn future_time_projection_labels_empty_space_without_creating_data() {
    let mut chart = ChartEngine::new(600.0, 300.0, 1.0);
    chart
        .set_series_data(
            0,
            &[1_000.0, 1_060.0, 1_120.0],
            &[100.0, 101.0, 102.0],
            &[101.0, 102.0, 103.0],
            &[99.0, 100.0, 101.0],
            &[100.5, 101.5, 102.5],
        )
        .unwrap();
    let canonical_len = chart.data_layer().merged_times().len();
    let base_index = chart.time_scale.base_index();

    assert!(chart.set_future_time_projection(Some(60), 32));
    assert_eq!(chart.axis_time_key_at(3), Some(1_180));
    assert_eq!(chart.axis_time_key_at(34), Some(3_040));
    assert_eq!(chart.axis_time_key_at(35), None);
    assert_eq!(chart.data_layer().merged_times().len(), canonical_len);
    assert_eq!(chart.time_scale.base_index(), base_index);
    assert_eq!(chart.time_scale.points_len(), canonical_len);
    assert!(
        chart
            .time_marks(1.0)
            .iter()
            .any(|(index, _)| *index > base_index)
    );
}

#[test]
fn past_time_projection_labels_left_whitespace_without_creating_data() {
    let mut chart = ChartEngine::new(600.0, 300.0, 1.0);
    chart
        .set_series_data(
            0,
            &[1_000.0, 1_060.0, 1_120.0],
            &[100.0, 101.0, 102.0],
            &[101.0, 102.0, 103.0],
            &[99.0, 100.0, 101.0],
            &[100.5, 101.5, 102.5],
        )
        .unwrap();
    let canonical_len = chart.data_layer().merged_times().len();
    let base_index = chart.time_scale.base_index();

    assert!(chart.set_past_time_projection(Some(60), 32));
    assert_eq!(chart.axis_time_key_at_logical(-1), Some(940));
    assert_eq!(chart.axis_time_key_at_logical(-32), Some(-920));
    assert_eq!(chart.axis_time_key_at_logical(-33), None);
    assert_eq!(chart.data_layer().merged_times().len(), canonical_len);
    assert_eq!(chart.time_scale.base_index(), base_index);
    assert_eq!(chart.time_scale.points_len(), canonical_len);
    assert!(chart.time_marks(1.0).iter().any(|(index, _)| *index < 0));
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
    assert!(
        chart.series_apply_price_format_json(
            0,
            r#"{"type":"price","precision":4,"min_move":0.0001}"#,
        )
    );

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
    assert!(
        chart
            .price_scale_visible_range_for(0, PriceScaleTarget::Left)
            .is_some()
    );
    assert!(
        chart
            .price_scale_visible_range_for(0, PriceScaleTarget::Right)
            .is_none()
    );
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
    assert!(
        axis.labels
            .iter()
            .any(|label| label.align == AxisTextAlign::Right)
    );
    assert!(
        !axis
            .labels
            .iter()
            .any(|label| label.align == AxisTextAlign::Left)
    );
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
    assert!(
        frame.panes[0]
            .main
            .iter()
            .any(|p| matches!(p, aeris_charts_render::draw_list::Prim::VLine { .. }))
    );
    assert!(
        frame.panes[0]
            .main
            .iter()
            .any(|p| matches!(p, aeris_charts_render::draw_list::Prim::HLine { .. }))
    );
    assert!(
        frame.panes[0]
            .main
            .iter()
            .any(|p| matches!(p, aeris_charts_render::draw_list::Prim::Circle { .. }))
    );

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
fn ema_family_defaults_to_one_pixel_and_respects_explicit_widths() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    let times = (0..30).map(|i| i as f64).collect::<Vec<_>>();
    let values = times.iter().map(|v| v + 100.0).collect::<Vec<_>>();
    chart
        .set_series_data(0, &times, &values, &values, &values, &values)
        .unwrap();

    let sma = chart.add_sma(0, 5).unwrap();
    let mut ema_outputs = vec![
        chart.add_ema(0, 5).unwrap(),
        chart.add_dema(0, 5).unwrap(),
        chart.add_tema(0, 5).unwrap(),
    ];
    ema_outputs.extend(chart.add_ema_ribbon(0, [2, 3, 5, 8, 13]));
    assert_eq!(chart.series_entry(sma).unwrap().line_width, Some(2.0));
    for &id in &ema_outputs {
        assert_eq!(chart.series_entry(id).unwrap().line_width, Some(1.0));
    }

    let customized = ema_outputs[0];
    assert!(chart.series_apply_options_json(customized, r#"{"line_width":3}"#));
    assert_eq!(
        chart.series_entry(customized).unwrap().line_width,
        Some(3.0)
    );
    chart.reset_style_to_defaults();
    for &id in &ema_outputs {
        assert_eq!(chart.series_entry(id).unwrap().line_width, Some(1.0));
    }
    assert_eq!(chart.series_entry(sma).unwrap().line_width, Some(2.0));
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
        (matches!(
            kind,
            IndicatorKind::Vwap
                | IndicatorKind::Obv
                | IndicatorKind::AccumulationDistribution
                | IndicatorKind::PriceVolumeTrend
                | IndicatorKind::ChaikinOscillator { .. }
                | IndicatorKind::Klinger { .. }
                | IndicatorKind::RelativeVolume { .. }
                | IndicatorKind::ElderForce { .. }
                | IndicatorKind::EaseOfMovement { .. }
                | IndicatorKind::VolumeOscillator { .. }
                | IndicatorKind::Cmf { .. }
                | IndicatorKind::Mfi { .. }
                | IndicatorKind::Volume { .. }
        ) || matches!(kind, IndicatorKind::KLineChart(indicator) if indicator.needs_volume()))
        .then_some(volume)
        .flatten(),
    )
}

pub(crate) fn assert_indicator_binding_matches_full(chart: &ChartEngine, binding_index: usize) {
    let binding = &chart.indicators[binding_index];
    let (all_times, all_source) = chart.data.series_data(binding.source).unwrap();
    // The fork whitespace rule, independent of how the runtime implements it: a whitespace row
    // (non-finite close, high or low) is absent from every window and every recursion, so the
    // expected values are each formula over the remaining rows (their volumes with them),
    // scattered back with no value on the whitespace rows. Every output then starts at its first
    // value (owner decision Q-H) and spans the rest of the source rows.
    let kept = (0..all_times.len())
        .filter(|&row| (1..4).all(|column| all_source[column][row].is_finite()))
        .collect::<Vec<_>>();
    let compact_times = kept.iter().map(|&row| all_times[row]).collect::<Vec<_>>();
    let compact_source: [Vec<f64>; 4] =
        std::array::from_fn(|column| kept.iter().map(|&row| all_source[column][row]).collect());
    let times = &compact_times[..];
    let source: [&[f64]; 4] = std::array::from_fn(|column| &compact_source[column][..]);
    let expected = match binding.kind {
        IndicatorKind::Custom { .. } => return,
        IndicatorKind::Aroon { period } => {
            let values = aeris_charts_indicators::aroon(source[1], source[2], period);
            vec![
                values.iter().map(|value| value.0).collect(),
                values.iter().map(|value| value.1).collect(),
            ]
        }
        IndicatorKind::AwesomeOscillator => vec![aeris_charts_indicators::awesome_oscillator(
            source[1], source[2],
        )],
        IndicatorKind::Dpo { period } => vec![aeris_charts_indicators::dpo(source[3], period)],
        IndicatorKind::ChandeMomentum { period } => {
            vec![aeris_charts_indicators::chande_momentum(source[3], period)]
        }
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
        IndicatorKind::HistoricalVolatility {
            period,
            annualization,
        } => vec![aeris_charts_indicators::historical_volatility(
            source[3],
            period,
            annualization,
        )],
        IndicatorKind::Trix { period, signal } => {
            let points = aeris_charts_indicators::trix(source[3], period, signal);
            vec![
                points.iter().map(|point| point.line).collect(),
                points.iter().map(|point| point.signal).collect(),
            ]
        }
        IndicatorKind::Kst {
            roc,
            smoothing,
            signal,
        } => {
            let points = aeris_charts_indicators::kst(source[3], roc, smoothing, signal);
            vec![
                points.iter().map(|point| point.line).collect(),
                points.iter().map(|point| point.signal).collect(),
            ]
        }
        IndicatorKind::Kama { period, fast, slow } => {
            vec![aeris_charts_indicators::kama(source[3], period, fast, slow)]
        }
        IndicatorKind::McGinley { period } => {
            vec![aeris_charts_indicators::mcginley(source[3], period)]
        }
        IndicatorKind::LinearRegression { period, deviation } => {
            let points = aeris_charts_indicators::linear_regression(source[3], period, deviation);
            vec![
                points.iter().map(|point| point.curve).collect(),
                points.iter().map(|point| point.upper).collect(),
                points.iter().map(|point| point.lower).collect(),
            ]
        }
        IndicatorKind::Choppiness { period } => {
            vec![aeris_charts_indicators::choppiness(
                source[1], source[2], source[3], period,
            )]
        }
        IndicatorKind::AtrBands { period, multiplier } => {
            let points = aeris_charts_indicators::atr_bands(
                source[1], source[2], source[3], period, multiplier,
            );
            vec![
                points.iter().map(|point| point.upper).collect(),
                points.iter().map(|point| point.basis).collect(),
                points.iter().map(|point| point.lower).collect(),
            ]
        }
        IndicatorKind::Tsi {
            long,
            short,
            signal,
        } => {
            let points = aeris_charts_indicators::tsi(source[3], long, short, signal);
            vec![
                points.iter().map(|point| point.line).collect(),
                points.iter().map(|point| point.signal).collect(),
            ]
        }
        IndicatorKind::MassIndex {
            ema_period,
            sum_period,
        } => vec![aeris_charts_indicators::mass_index(
            source[1], source[2], ema_period, sum_period,
        )],
        IndicatorKind::Vortex { period } => {
            let points = aeris_charts_indicators::vortex(source[1], source[2], source[3], period);
            vec![
                points.iter().map(|point| point.plus).collect(),
                points.iter().map(|point| point.minus).collect(),
            ]
        }
        IndicatorKind::CoppockCurve {
            long,
            short,
            smoothing,
        } => {
            vec![aeris_charts_indicators::coppock_curve(
                source[3], long, short, smoothing,
            )]
        }
        IndicatorKind::FisherTransform { period } => {
            let points = aeris_charts_indicators::fisher_transform(source[1], source[2], period);
            vec![
                points.iter().map(|point| point.line).collect(),
                points.iter().map(|point| point.trigger).collect(),
            ]
        }
        IndicatorKind::UltimateOscillator {
            short,
            medium,
            long,
        } => {
            vec![aeris_charts_indicators::ultimate_oscillator(
                source[1], source[2], source[3], short, medium, long,
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
        IndicatorKind::BollingerMetrics { period, deviation } => {
            let points = aeris_charts_indicators::bollinger_metrics(source[3], period, deviation);
            vec![
                points.iter().map(|point| point.0).collect(),
                points.iter().map(|point| point.1).collect(),
            ]
        }
        IndicatorKind::Envelopes {
            period,
            percent,
            exponential,
        } => {
            let points =
                aeris_charts_indicators::envelopes(source[3], period, percent, exponential);
            vec![
                points.iter().map(|point| point.0).collect(),
                points.iter().map(|point| point.1).collect(),
                points.iter().map(|point| point.2).collect(),
            ]
        }
        IndicatorKind::Alma {
            period,
            offset,
            sigma,
        } => vec![aeris_charts_indicators::alma(
            source[3], period, offset, sigma,
        )],
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
        IndicatorKind::Obv
        | IndicatorKind::AccumulationDistribution
        | IndicatorKind::PriceVolumeTrend
        | IndicatorKind::ChaikinOscillator { .. }
        | IndicatorKind::Klinger { .. }
        | IndicatorKind::RelativeVolume { .. }
        | IndicatorKind::ElderForce { .. }
        | IndicatorKind::EaseOfMovement { .. }
        | IndicatorKind::VolumeOscillator { .. } => {
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
            if let IndicatorKind::VolumeOscillator { fast, slow, signal } = binding.kind {
                let points =
                    aeris_charts_indicators::volume_oscillator(&volume, fast, slow, signal);
                vec![
                    points.iter().map(|point| point.line).collect(),
                    points.iter().map(|point| point.signal).collect(),
                    points.iter().map(|point| point.histogram).collect(),
                ]
            } else if let IndicatorKind::Klinger { fast, slow, signal } = binding.kind {
                let points = aeris_charts_indicators::klinger(
                    source[1], source[2], source[3], &volume, fast, slow, signal,
                );
                vec![
                    points.iter().map(|point| point.line).collect(),
                    points.iter().map(|point| point.signal).collect(),
                ]
            } else if let IndicatorKind::ElderForce { period } = binding.kind {
                vec![aeris_charts_indicators::elder_force(
                    source[3], &volume, period,
                )]
            } else if let IndicatorKind::EaseOfMovement { period, divisor } = binding.kind {
                vec![aeris_charts_indicators::ease_of_movement(
                    source[1], source[2], &volume, period, divisor,
                )]
            } else {
                vec![match binding.kind {
                    IndicatorKind::Obv => aeris_charts_indicators::obv(source[3], &volume),
                    IndicatorKind::AccumulationDistribution => {
                        aeris_charts_indicators::accumulation_distribution(
                            source[1], source[2], source[3], &volume,
                        )
                    }
                    IndicatorKind::PriceVolumeTrend => {
                        aeris_charts_indicators::price_volume_trend(source[3], &volume)
                    }
                    IndicatorKind::ChaikinOscillator { fast, slow } => {
                        aeris_charts_indicators::chaikin_oscillator(
                            source[1], source[2], source[3], &volume, fast, slow,
                        )
                    }
                    IndicatorKind::RelativeVolume { period } => {
                        aeris_charts_indicators::relative_volume(&volume, period)
                    }
                    _ => unreachable!(),
                }]
            }
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
        IndicatorKind::KLineChart(ref indicator) => {
            let missing = indicator.missing_volume();
            let volume = binding
                .volume_source
                .and_then(|id| chart.data.series_data(id))
                .map(|(volume_times, values)| {
                    let mut aligned = vec![missing; times.len()];
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
                .unwrap_or_else(|| vec![missing; times.len()]);
            indicator.compute(&aeris_charts_indicators::klinechart::Bars {
                open: source[0],
                high: source[1],
                low: source[2],
                close: source[3],
                volume: &volume,
                turnover: source[3],
            })
        }
        IndicatorKind::SwingPoints { .. }
        | IndicatorKind::MarketStructure { .. }
        | IndicatorKind::FairValueGaps { .. }
        | IndicatorKind::OrderBlocks { .. }
        | IndicatorKind::SessionLevels { .. }
        | IndicatorKind::PreviousPeriodLevels { .. }
        | IndicatorKind::OpeningRange { .. } => {
            panic!(
                "session and structural studies require calendar/annotation-aware reference fixtures"
            )
        }
    };

    for (&output, expected) in binding.outputs.iter().zip(expected) {
        let mut scattered = vec![None; all_times.len()];
        for (&row, value) in kept.iter().zip(expected) {
            scattered[row] = value;
        }
        let from = scattered
            .iter()
            .position(|value| value.is_some_and(|value| !value.is_nan()))
            .unwrap_or(all_times.len());
        let expected = all_times
            .iter()
            .copied()
            .zip(scattered)
            .skip(from)
            .map(|(time, value)| (time, value.unwrap_or(f64::NAN)))
            .collect::<Vec<_>>();
        let (actual_times, actual) = chart.data.series_data(output).unwrap();
        assert_eq!(
            actual_times.len(),
            expected.len(),
            "{:?} output {output:?} times count, source {}, from {from}",
            binding.kind,
            all_times.len()
        );
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
    let kinds = scalar_indicator_kinds();
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
        assert!(
            outputs
                .iter()
                .all(|&output| chart.series_kind(output).is_none())
        );
        assert!(chart.indicators.is_empty());
    }
}

/// Every built-in indicator kind with short periods, for engine-path equivalence checks.
fn every_indicator_kind() -> Vec<IndicatorKind> {
    use aeris_charts_indicators::{PivotKind, VwapReset};
    vec![
        IndicatorKind::Aroon { period: 5 },
        IndicatorKind::AwesomeOscillator,
        IndicatorKind::Dpo { period: 5 },
        IndicatorKind::ChandeMomentum { period: 5 },
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
        IndicatorKind::BollingerMetrics {
            period: 5,
            deviation: 2.0,
        },
        IndicatorKind::Envelopes {
            period: 5,
            percent: 10.0,
            exponential: false,
        },
        IndicatorKind::Envelopes {
            period: 5,
            percent: 10.0,
            exponential: true,
        },
        IndicatorKind::Alma {
            period: 5,
            offset: 0.85,
            sigma: 6.0,
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
        IndicatorKind::AccumulationDistribution,
        IndicatorKind::PriceVolumeTrend,
        IndicatorKind::ChaikinOscillator { fast: 3, slow: 7 },
        IndicatorKind::RelativeVolume { period: 5 },
        IndicatorKind::ElderForce { period: 5 },
        IndicatorKind::EaseOfMovement {
            period: 5,
            divisor: 100.0,
        },
        IndicatorKind::HistoricalVolatility {
            period: 5,
            annualization: 252.0,
        },
        IndicatorKind::Trix {
            period: 3,
            signal: 4,
        },
        IndicatorKind::CoppockCurve {
            long: 7,
            short: 5,
            smoothing: 3,
        },
        IndicatorKind::FisherTransform { period: 5 },
        IndicatorKind::UltimateOscillator {
            short: 3,
            medium: 5,
            long: 7,
        },
        IndicatorKind::Kst {
            roc: [2, 3, 4, 5],
            smoothing: [2, 2, 2, 3],
            signal: 3,
        },
        IndicatorKind::Tsi {
            long: 5,
            short: 3,
            signal: 3,
        },
        IndicatorKind::MassIndex {
            ema_period: 3,
            sum_period: 5,
        },
        IndicatorKind::Klinger {
            fast: 3,
            slow: 7,
            signal: 4,
        },
        IndicatorKind::Kama {
            period: 5,
            fast: 2,
            slow: 10,
        },
        IndicatorKind::McGinley { period: 5 },
        IndicatorKind::LinearRegression {
            period: 5,
            deviation: 2.0,
        },
        IndicatorKind::Choppiness { period: 5 },
        IndicatorKind::AtrBands {
            period: 5,
            multiplier: 2.0,
        },
        IndicatorKind::Vortex { period: 5 },
        IndicatorKind::VolumeOscillator {
            fast: 3,
            slow: 7,
            signal: 4,
        },
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
    .into_iter()
    .chain(study_indicator_kinds())
    .collect()
}

fn structure_kind(kind: &IndicatorKind) -> bool {
    matches!(
        kind,
        IndicatorKind::SwingPoints { .. }
            | IndicatorKind::MarketStructure { .. }
            | IndicatorKind::FairValueGaps { .. }
            | IndicatorKind::OrderBlocks { .. }
    )
}

/// Source bars, a volume series on the same timestamps, and a volume series that skips every
/// tenth history row, kept alongside the engine so a fresh engine can be loaded with the same data.
struct LiveIndicatorFixture {
    chart: ChartEngine,
    aligned_volume: SeriesId,
    sparse_volume: SeriesId,
    times: Vec<f64>,
    bars: Vec<[f64; 4]>,
    volumes: Vec<f64>,
    sparse_rows: Vec<usize>,
    outputs: Vec<Vec<SeriesId>>,
}

impl LiveIndicatorFixture {
    fn bar(row: usize) -> [f64; 4] {
        let x = row as f64;
        if row % 97 == 41 {
            return [f64::NAN; 4];
        }
        let close = 100.0 + (x * 0.0031).sin() * 25.0 + (x * 0.17).sin() * 1.5;
        let open = close - (x * 0.53).sin() * 0.6;
        [open, close.max(open) + 0.4, close.min(open) - 0.4, close]
    }

    fn new(rows: usize) -> Self {
        let times = (0..rows)
            .map(|row| row as f64 * 3_600.0)
            .collect::<Vec<_>>();
        let bars = (0..rows).map(Self::bar).collect::<Vec<_>>();
        let volumes = (0..rows)
            .map(|row| 100.0 + (row % 37) as f64 * 7.0)
            .collect::<Vec<_>>();
        let sparse_rows = (0..rows).filter(|row| row % 10 != 9).collect();
        let mut fixture = Self {
            chart: ChartEngine::new(800.0, 500.0, 1.0),
            aligned_volume: 0,
            sparse_volume: 0,
            times,
            bars,
            volumes,
            sparse_rows,
            outputs: Vec::new(),
        };
        fixture.load();
        fixture.attach();
        fixture
    }

    fn load(&mut self) {
        let column = |index: usize| self.bars.iter().map(|bar| bar[index]).collect::<Vec<_>>();
        let chart = &mut self.chart;
        self.aligned_volume = chart.add_series(SeriesKind::Histogram);
        self.sparse_volume = chart.add_series(SeriesKind::Histogram);
        chart
            .set_series_data(
                0,
                &self.times,
                &column(0),
                &column(1),
                &column(2),
                &column(3),
            )
            .unwrap();
        let volumes = &self.volumes;
        chart
            .set_series_data(
                self.aligned_volume,
                &self.times,
                volumes,
                volumes,
                volumes,
                volumes,
            )
            .unwrap();
        let sparse_times = self
            .sparse_rows
            .iter()
            .map(|&row| self.times[row])
            .collect::<Vec<_>>();
        let sparse = self
            .sparse_rows
            .iter()
            .map(|&row| self.volumes[row])
            .collect::<Vec<_>>();
        chart
            .set_series_data(
                self.sparse_volume,
                &sparse_times,
                &sparse,
                &sparse,
                &sparse,
                &sparse,
            )
            .unwrap();
    }

    /// Every kind at once, plus each derived price input and volume on its own timeline. The
    /// fork list carries the seed and convention variants, KDJ, the structure and session studies
    /// and the KLineChart templates (upstream: its 71 built-in kinds plus the study schemas).
    fn attach(&mut self) {
        let kinds = every_indicator_kind_with_klinechart_templates();
        let chart = &mut self.chart;
        let mut outputs = kinds
            .iter()
            .map(|kind| add_test_indicator(chart, kind, Some(self.aligned_volume)))
            .collect::<Vec<_>>();
        for input in [
            IndicatorInputSource::Hl2,
            IndicatorInputSource::Hlc3,
            IndicatorInputSource::Ohlc4,
            IndicatorInputSource::Hlcc4,
        ] {
            outputs.push(chart.add_indicator_kind_with_input(
                0,
                input,
                IndicatorKind::Sma { period: 5 },
                None,
            ));
        }
        outputs.push(chart.add_indicator_kind(0, IndicatorKind::Obv, Some(self.sparse_volume)));
        outputs.push(chart.add_indicator_kind_with_input(
            0,
            IndicatorInputSource::Hlc3,
            IndicatorKind::Vwap,
            Some(self.sparse_volume),
        ));
        assert!(outputs.iter().all(|outputs| !outputs.is_empty()));
        self.outputs = outputs;
    }

    /// Write one row to the source and then to both volume series, as a host feeding one
    /// stream does.
    fn write(&mut self, row: usize, bar: [f64; 4], volume: f64) {
        let time = row as f64 * 3_600.0;
        if row == self.times.len() {
            self.times.push(time);
            self.bars.push(bar);
            self.volumes.push(volume);
            self.sparse_rows.push(row);
        } else {
            self.bars[row] = bar;
            self.volumes[row] = volume;
            if let Err(position) = self.sparse_rows.binary_search(&row) {
                self.sparse_rows.insert(position, row);
            }
        }
        assert!(self.chart.update_series_bar(0, time, bar));
        for series in [self.aligned_volume, self.sparse_volume] {
            assert!(self.chart.update_series_bar(series, time, [volume; 4]));
        }
    }

    fn assert_matches_fresh_engine(&self, stage: &str) {
        let mut fresh = Self {
            chart: ChartEngine::new(800.0, 500.0, 1.0),
            aligned_volume: 0,
            sparse_volume: 0,
            times: self.times.clone(),
            bars: self.bars.clone(),
            volumes: self.volumes.clone(),
            sparse_rows: self.sparse_rows.clone(),
            outputs: Vec::new(),
        };
        fresh.load();
        fresh.attach();
        for (live, expected) in self.outputs.iter().zip(&fresh.outputs) {
            for (&id, &reference) in live.iter().zip(expected) {
                let kind = &self
                    .chart
                    .indicators
                    .iter()
                    .find(|binding| binding.outputs.contains(&id))
                    .unwrap()
                    .kind;
                let (actual_times, actual) = self.chart.data.series_data(id).unwrap();
                let (expected_times, values) = fresh.chart.data.series_data(reference).unwrap();
                assert_eq!(actual_times, expected_times, "{kind:?} {stage} output {id}");
                for (row, (&a, &b)) in actual[3].iter().zip(values[3]).enumerate() {
                    assert!(
                        (a.is_nan() && b.is_nan()) || (a - b).abs() <= 1e-9 * b.abs().max(1.0),
                        "{kind:?} {stage} output {id} row {row}: {a:?} != {b:?}"
                    );
                }
            }
        }
    }

    /// Indicator source rows read and the largest output LOD rewrite of the last update.
    ///
    /// Fork: ZigZag's count adds the rows it re-emits back to its unconfirmed leg's open row, which
    /// follows the price path at each size rather than the history length, so it is summed apart
    /// (the third value) and bounded instead of compared.
    fn work(&self) -> (usize, usize, usize) {
        let lod_nodes = self
            .outputs
            .iter()
            .flatten()
            .map(|&id| self.chart.data.last_lod_update_nodes(id).unwrap())
            .max()
            .unwrap();
        let zigzag = self
            .chart
            .indicators
            .iter()
            .filter(|binding| matches!(binding.kind, IndicatorKind::ZigZag { .. }))
            .map(IndicatorBinding::last_work_rows)
            .sum::<usize>();
        (
            self.chart.last_indicator_work_rows() - zigzag,
            lod_nodes,
            zigzag,
        )
    }
}

#[test]
fn engine_live_updates_with_every_kind_attached_do_bounded_work_and_match_a_fresh_engine() {
    let mut work = Vec::new();
    // Fork: a window that spans a whitespace row reads it too (KLineChart templates step in
    // source rows; the compacted windows count the valid rows they read), so both sizes put the
    // fixture's periodic gap rows (`row % 97 == 41`) at the same distance from the tip, and, like
    // upstream's 65,536, at the same offset from the 1,024-row checkpoints the structure studies
    // resume a tip replacement from: 103,424 = 4,096 + lcm(97, 1,024).
    for rows in [4_096, 103_424] {
        let mut fixture = LiveIndicatorFixture::new(rows);
        let row = rows;
        fixture.write(row, LiveIndicatorFixture::bar(row), 333.0);
        let append = fixture.work();
        let [open, high, low, close] = LiveIndicatorFixture::bar(row);
        fixture.write(row, [open, high + 0.75, low - 0.5, close + 0.25], 444.0);
        let replace = fixture.work();
        work.push((rows, append, replace));
        if rows == 4_096 {
            fixture.assert_matches_fresh_engine("live tip");
            // A historical volume sample at a timestamp the sparse series did not have shifts
            // every later sparse volume row; the aligned rows before it must stay unchanged.
            fixture.write(rows - 7, LiveIndicatorFixture::bar(rows - 7), 555.0);
            fixture.assert_matches_fresh_engine("historical sparse volume insert");
        }
    }
    let (_, small_append, small_replace) = work[0];
    let (_, large_append, large_replace) = work[1];
    assert_eq!(
        (small_append.0, small_replace.0),
        (large_append.0, large_replace.0),
        "indicator source rows read per live update grow with history: {work:?}"
    );
    assert!(
        large_append.1 <= 16 && large_replace.1 <= 16,
        "an output series was rewritten beyond its tail: {work:?}"
    );
    assert!(
        work.iter()
            .all(|(_, append, replace)| append.2 <= 64 && replace.2 <= 64),
        "ZigZag re-emitted more than its unconfirmed leg: {work:?}"
    );
}

#[test]
fn every_indicator_binding_matches_fresh_engine_on_flat_runs_after_large_moves() {
    // Large $1M and $100 moves followed by exact flat runs (one straddling the 1,024-row
    // checkpoint), $1M prices with 1e-7 moves, near-flat and alternating runs, single- and
    // multi-row gaps. Half-ranges are dyadic so flat closes keep exactly flat midpoints while
    // the range and volume change on every row; a running window sum leaves residue here.
    const N: usize = 1_100;
    let mut seed = 0x9e37_79b9_u64;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let mut close = 1e6;
    let mut bars = Vec::with_capacity(N + 3);
    let mut volumes = Vec::with_capacity(N + 3);
    for row in 0..N + 3 {
        let unit = (next() >> 11) as f64 / (1_u64 << 53) as f64 * 2.0 - 1.0;
        let tick = (next() % 11) as f64 - 5.0;
        close = match row {
            0..150 => close + unit * 5_000.0,
            300..450 => 1e6 + tick * 1e-7,
            450..600 => (if row == 450 { 100.0 } else { close } + unit * 5.0).max(25.0),
            600..750 => 100.0 + tick * 1e-8,
            750..900 => 100.0 + if row % 2 == 0 { 1.0 } else { -1.0 },
            900..960 => (if row == 900 { 1e6 } else { close }) + unit * 5_000.0,
            _ => close,
        };
        let half = 0.25
            * (1 + next()
                % match row {
                    0..150 | 900..960 => 3_600,
                    450..600 => 40,
                    _ => 8,
                }) as f64;
        bars.push([close, close + half, close - half, close]);
        volumes.push(if row % 29 == 0 {
            0.0
        } else if next() % 50 == 0 {
            1e6
        } else {
            (1 + next() % 1_000) as f64
        });
    }
    for row in [200, 640, 641, 1_019, 1_020, 1_021, 1_022, 1_023, 1_060] {
        bars[row] = [f64::NAN; 4];
    }
    let times = (0..N + 3)
        .map(|row| row as f64 * 3_600.0)
        .collect::<Vec<_>>();
    let column = |rows: usize, index: usize| bars[..rows].iter().map(|bar| bar[index]).collect();
    let install = |rows: usize, kind: &IndicatorKind| {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        let volume = chart.add_series(SeriesKind::Histogram);
        let [open, high, low, close]: [Vec<f64>; 4] =
            std::array::from_fn(|index| column(rows, index));
        chart
            .set_series_data(0, &times[..rows], &open, &high, &low, &close)
            .unwrap();
        let v = &volumes[..rows];
        chart
            .set_series_data(volume, &times[..rows], v, v, v, v)
            .unwrap();
        let outputs = add_test_indicator(&mut chart, kind, Some(volume));
        (chart, volume, outputs)
    };
    for kind in every_indicator_kind_with_klinechart_templates() {
        let (mut chart, volume, outputs) = install(N, &kind);
        for rows in N + 1..=N + 3 {
            let row = rows - 1;
            assert!(
                chart.update_series_bar(0, times[row], bars[row]),
                "{kind:?}"
            );
            assert!(
                chart.update_series_bar(volume, times[row], [volumes[row]; 4]),
                "{kind:?}"
            );
            let (fresh, _, expected) = install(rows, &kind);
            for (&id, &reference) in outputs.iter().zip(&expected) {
                let (_, actual) = chart.data.series_data(id).unwrap();
                let (_, wanted) = fresh.data.series_data(reference).unwrap();
                assert_eq!(actual[3].len(), wanted[3].len(), "{kind:?} rows {rows}");
                for (index, (&a, &b)) in actual[3].iter().zip(wanted[3]).enumerate() {
                    assert!(
                        (a.is_nan() && b.is_nan())
                            || (a - b).abs() <= 1e-12_f64.max(1e-9 * b.abs()),
                        "{kind:?} rows {rows} output {id} point {index}: binding {a:e} != fresh {b:e}"
                    );
                }
            }
        }
    }
}

/// The structure and session studies, with small windows so short fixtures confirm structure.
fn study_indicator_kinds() -> Vec<IndicatorKind> {
    use crate::indicators::{
        OrderBlockZone, PreviousPeriod, StructureBreakOn, StructureMitigation,
        StructureMitigationPrice, StudyCalendarPolicy,
    };
    vec![
        IndicatorKind::SwingPoints { left: 2, right: 2 },
        IndicatorKind::MarketStructure {
            left: 2,
            right: 2,
            break_on: StructureBreakOn::Close,
        },
        IndicatorKind::FairValueGaps {
            min_size: 0.0,
            mitigation: StructureMitigation::Touch,
            mitigation_price: StructureMitigationPrice::Wick,
            max_active: 8,
            show_mitigated: true,
        },
        IndicatorKind::OrderBlocks {
            left: 2,
            right: 2,
            break_on: StructureBreakOn::Wick,
            zone: OrderBlockZone::Body,
            mitigation: StructureMitigation::Half,
            mitigation_price: StructureMitigationPrice::Close,
            max_active: 8,
            show_mitigated: false,
        },
        IndicatorKind::SessionLevels {
            calendar: StudyCalendarPolicy::Utc,
        },
        IndicatorKind::PreviousPeriodLevels {
            period: PreviousPeriod::Day,
            calendar: StudyCalendarPolicy::Utc,
        },
        IndicatorKind::OpeningRange {
            duration_seconds: 3_600,
            calendar: StudyCalendarPolicy::Utc,
        },
    ]
}

#[test]
fn volume_studies_weight_source_bars_missing_from_the_volume_series_as_zero() {
    // The volume series lacks two source timestamps. Every volume-flow study must weight them as
    // zero volume, exactly like a volume series that carries explicit zeros there.
    let times = (0..30).map(|row| row as f64 * 60.0).collect::<Vec<_>>();
    let close = (0..30)
        .map(|row| 100.0 + (row as f64 * 0.7).sin() * 3.0)
        .collect::<Vec<_>>();
    let high = close.iter().map(|value| value + 1.0).collect::<Vec<_>>();
    let low = close.iter().map(|value| value - 1.5).collect::<Vec<_>>();
    let volume = |row: usize| 100.0 + (row % 5) as f64 * 40.0;
    let missing = [7, 18];
    let install = |kind: &IndicatorKind, sparse: bool| {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        chart
            .set_series_data(0, &times, &close, &high, &low, &close)
            .unwrap();
        let volume_series = chart.add_series(SeriesKind::Histogram);
        let rows = (0..30)
            .filter(|row| !sparse || !missing.contains(row))
            .collect::<Vec<_>>();
        let volume_times = rows.iter().map(|&row| times[row]).collect::<Vec<_>>();
        let values = rows
            .iter()
            .map(|&row| {
                if missing.contains(&row) {
                    0.0
                } else {
                    volume(row)
                }
            })
            .collect::<Vec<_>>();
        chart
            .set_series_data(
                volume_series,
                &volume_times,
                &values,
                &values,
                &values,
                &values,
            )
            .unwrap();
        let outputs = chart.add_indicator_kind(0, kind.clone(), Some(volume_series));
        assert!(!outputs.is_empty(), "{kind:?} binds");
        outputs
            .iter()
            .map(|&output| chart.data.series_data(output).unwrap().1[3].to_vec())
            .collect::<Vec<_>>()
    };
    for kind in [
        IndicatorKind::Obv,
        IndicatorKind::AccumulationDistribution,
        IndicatorKind::PriceVolumeTrend,
        IndicatorKind::ChaikinOscillator { fast: 3, slow: 7 },
        IndicatorKind::RelativeVolume { period: 5 },
        IndicatorKind::VolumeOscillator {
            fast: 3,
            slow: 7,
            signal: 4,
        },
        IndicatorKind::ElderForce { period: 5 },
        IndicatorKind::EaseOfMovement {
            period: 5,
            divisor: 100.0,
        },
        IndicatorKind::Klinger {
            fast: 3,
            slow: 7,
            signal: 4,
        },
    ] {
        let sparse = install(&kind, true);
        let zeros = install(&kind, false);
        for (output, (sparse, zeros)) in sparse.iter().zip(&zeros).enumerate() {
            for (row, (sparse, zeros)) in sparse.iter().zip(zeros).enumerate() {
                assert!(
                    (sparse.is_nan() && zeros.is_nan()) || (sparse - zeros).abs() < 1e-9,
                    "{kind:?} output {output} row {row}: {sparse} != {zeros}"
                );
            }
        }
    }
}

/// `IndicatorKind` keeps its large internally tagged serde bodies out of line (see the enum), so every
/// entry point that used to carry its own copy must still agree on one JSON contract.
#[test]
fn indicator_kind_json_contract_is_identical_across_every_deserialization_path() {
    #[derive(serde::Deserialize)]
    struct Holder {
        kind: IndicatorKind,
    }
    for kind in every_indicator_kind() {
        let value = serde_json::to_value(&kind).expect("kind serializes");
        assert!(value["kind"].is_string(), "{kind:?} is internally tagged");
        assert_eq!(
            serde_json::from_value::<IndicatorKind>(value.clone()).expect("from_value"),
            kind
        );
        assert_eq!(
            serde_json::from_str::<IndicatorKind>(&value.to_string()).expect("from_str"),
            kind
        );
        let held = serde_json::from_value::<Holder>(serde_json::json!({ "kind": value }))
            .expect("as a struct field");
        assert_eq!(held.kind, kind);
    }
    assert_eq!(
        serde_json::from_value::<IndicatorKind>(serde_json::json!({ "kind": "ema", "period": 9 }))
            .expect("defaults fill omitted fields"),
        IndicatorKind::Ema {
            period: 9,
            seed: IndicatorSeed::default()
        }
    );
    assert!(
        serde_json::from_value::<IndicatorKind>(serde_json::json!({ "kind": "unknown_study" }))
            .is_err()
    );
}

fn indicator_reads_volume(kind: &IndicatorKind) -> bool {
    matches!(
        kind,
        IndicatorKind::Vwma { .. }
            | IndicatorKind::Vwap
            | IndicatorKind::VwapBands { .. }
            | IndicatorKind::Obv
            | IndicatorKind::AccumulationDistribution
            | IndicatorKind::PriceVolumeTrend
            | IndicatorKind::ChaikinOscillator { .. }
            | IndicatorKind::Klinger { .. }
            | IndicatorKind::RelativeVolume { .. }
            | IndicatorKind::VolumeOscillator { .. }
            | IndicatorKind::ElderForce { .. }
            | IndicatorKind::EaseOfMovement { .. }
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
        // Structure studies read the whole candle and accept only the close input.
        let inputs = if structure_kind(&kind) { 1 } else { 5 };
        for input in [
            IndicatorInputSource::Close,
            IndicatorInputSource::Hl2,
            IndicatorInputSource::Hlc3,
            IndicatorInputSource::Ohlc4,
            IndicatorInputSource::Hlcc4,
        ]
        .into_iter()
        .take(inputs)
        {
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
        assert!(
            !canonical
                .add_indicator_kind_with_input(0, IndicatorInputSource::Close, kind.clone(), None)
                .is_empty()
        );
        assert!(
            !composite
                .add_indicator_kind_with_input(0, input, kind, None)
                .is_empty()
        );
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

/// Rows one tick may cost a binding: its window for the built-in runtimes; a structure study
/// replays a revised row from its preceding 1,024-row checkpoint (appends scan one row).
fn indicator_tick_work_bound(binding: &IndicatorBinding, window: usize) -> usize {
    if binding.structure.is_some() {
        STRUCTURE_CHECKPOINT_ROWS + window
    } else {
        window
    }
}

const STRUCTURE_CHECKPOINT_ROWS: usize = 1_024;

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
        assert!(
            !chart
                .add_indicator_kind(0, kind, weighted.then_some(volume))
                .is_empty()
        );
    }
    for input in [
        IndicatorInputSource::Hl2,
        IndicatorInputSource::Hlc3,
        IndicatorInputSource::Ohlc4,
        IndicatorInputSource::Hlcc4,
    ] {
        assert!(
            !chart
                .add_indicator_kind_with_input(
                    0,
                    input,
                    IndicatorKind::StochasticRsi {
                        rsi_period: 5,
                        stochastic_period: 5,
                    },
                    None,
                )
                .is_empty()
        );
    }
    assert!(
        !chart
            .add_indicator_kind(0, IndicatorKind::Vwap, Some(gappy_volume))
            .is_empty()
    );
    // Every binding's latest rebuild (formula rows plus derived input rows) stays within its
    // window, so a regression in one binding cannot hide behind the others' small ticks.
    let assert_bounded = |chart: &ChartEngine, mutation: &str| {
        for binding in &chart.indicators {
            let bound = indicator_tick_work_bound(binding, WORK_PER_BINDING);
            assert!(
                binding.last_work_rows() <= bound,
                "{mutation}: {:?} {:?} did {} work rows (bound {bound})",
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
fn structure_and_session_slot_fill_ticks_take_the_tail_path() {
    // A time-sharing chart with the rest of the session pre-installed as whitespace slots. The
    // structure and session runtimes stop at the last real row like the built-in ones, so filling
    // the next slot scans that one row (an append), not a checkpoint replay through every slot,
    // and revising it replays at most from its preceding checkpoint; the work is reported.
    const ROWS: usize = 3_000;
    const SLOTS: usize = 2_000;
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
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
    for kind in study_indicator_kinds() {
        assert!(!chart.add_indicator_kind(0, kind, None).is_empty());
    }
    let assert_work =
        |chart: &ChartEngine, mutation: &str, structure_bound: usize, bound: usize| {
            for binding in &chart.indicators {
                let bound = if binding.structure.is_some() {
                    structure_bound
                } else {
                    bound
                };
                let work = binding.last_work_rows();
                assert!(
                    (1..=bound).contains(&work),
                    "{mutation}: {:?} did {work} work rows (expected 1..={bound})",
                    binding.kind
                );
            }
        };
    for row in ROWS..ROWS + 3 {
        let time = row as f64 * 60.0;
        chart.update_series_bar(0, time, swinging_bar(row, 0));
        assert_work(&chart, "fill the next slot", 1, 1);
        chart.update_series_bar(0, time, swinging_bar(row, 5));
        assert_work(
            &chart,
            "revise the forming slot",
            STRUCTURE_CHECKPOINT_ROWS,
            1,
        );
    }
    // A tick that skips a slot (a minute without trades) leaves a whitespace slot behind it: the
    // scan resumes at the previous last real row and covers only that slot and the new row.
    let row = ROWS + 4;
    chart.update_series_bar(0, row as f64 * 60.0, swinging_bar(row, 0));
    assert_work(&chart, "fill a slot past a whitespace one", 2, 2);
    for binding in 0..chart.indicators.len() {
        let label = format!("{:?}", chart.indicators[binding].kind);
        assert_binding_matches_fresh_install(&chart, binding, &label);
        let anchor = chart.indicators[binding].outputs[0];
        if chart.indicators[binding].structure.is_some() {
            let mut fresh = ChartEngine::new(800.0, 500.0, 1.0);
            let (times, values) = chart.data.series_data(0).unwrap();
            let seconds = times.iter().map(|&time| time as f64).collect::<Vec<_>>();
            fresh
                .set_series_data(0, &seconds, values[0], values[1], values[2], values[3])
                .unwrap();
            let outputs = fresh.add_indicator_kind(0, chart.indicators[binding].kind.clone(), None);
            assert_eq!(
                chart.study_annotations(anchor).unwrap(),
                fresh.study_annotations(outputs[0]).unwrap(),
                "{label} annotations"
            );
        }
    }
}

#[test]
fn study_outputs_report_warmup_and_no_fixed_convergence() {
    // Structure and session studies run their own scanners; their info must not read the
    // single-output placeholder runtime (a second session output once panicked there).
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    let times = (0..40).map(|row| row as f64 * 3_600.0).collect::<Vec<_>>();
    let bars = (0..40).map(|row| swinging_bar(row, 0)).collect::<Vec<_>>();
    let column = |index: usize| bars.iter().map(|bar| bar[index]).collect::<Vec<_>>();
    chart
        .set_series_data(0, &times, &column(0), &column(1), &column(2), &column(3))
        .unwrap();
    for kind in study_indicator_kinds() {
        let outputs = chart.add_indicator_kind(0, kind.clone(), None);
        assert!(!outputs.is_empty());
        let warmup = match kind {
            IndicatorKind::SwingPoints { left, right } => left + right,
            _ => 0,
        };
        for output in outputs {
            let info = chart.indicator_info(output).expect("study output info");
            assert_eq!(info.warmup_bars, warmup, "{kind:?}");
            assert_eq!(info.convergence_bars, None, "{kind:?}");
        }
    }
    // A study chained on a study output inherits the unbounded convergence.
    let session_low = chart.indicators[4].outputs[1];
    let sma = chart.add_sma(session_low, 3).unwrap();
    let info = chart.indicator_info(sma).unwrap();
    assert_eq!(info.warmup_bars, 2);
    assert_eq!(info.convergence_bars, None);
}

#[test]
fn structure_anchors_show_no_legend_value_and_are_never_hit() {
    // The market-structure, fair-value-gap and order-block anchors are all whitespace: fork chrome
    // (legend values, series hit testing) must treat them as carrying no value, and their zones
    // and segments have no hit target.
    use crate::financial_legend::{FinancialLegendIdentity, FinancialLegendRequest};
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    let times = (0..120).map(|row| row as f64 * 3_600.0).collect::<Vec<_>>();
    // Swings on a rising trend, with gapping bars, so highs break and gaps open.
    let bars = (0..120)
        .map(|row| {
            let [open, high, low, close] = swinging_bar(row, 0);
            let lift = row as f64 * 0.6;
            [open + lift, high + lift, low + lift, close + lift]
        })
        .collect::<Vec<_>>();
    let column = |index: usize| bars.iter().map(|bar| bar[index]).collect::<Vec<_>>();
    chart
        .set_series_data(0, &times, &column(0), &column(1), &column(2), &column(3))
        .unwrap();
    let anchors = study_indicator_kinds()
        .into_iter()
        .filter(|kind| structure_kind(kind) && !matches!(kind, IndicatorKind::SwingPoints { .. }))
        .map(|kind| {
            let outputs = chart.add_indicator_kind(0, kind, None);
            assert_eq!(outputs.len(), 1);
            outputs[0]
        })
        .collect::<Vec<_>>();
    chart.time_scale.set_width(800.0);
    chart.fit_content();
    chart.build_frame();
    let annotations = anchors
        .iter()
        .map(|&anchor| chart.study_annotations(anchor).unwrap())
        .collect::<Vec<_>>();
    assert!(annotations.iter().any(|a| !a.markers().is_empty()));
    assert!(annotations.iter().any(|a| !a.zones().is_empty()));
    let rows = chart.financial_legend(FinancialLegendRequest {
        logical_index: None,
        primary_title: "",
        show_primary_ohlc: false,
        primary_values_series: None,
        leading_series: &[],
        trailing_series: &[],
    });
    for &anchor in &anchors {
        let row = rows
            .iter()
            .find(|row| row.identity == FinancialLegendIdentity::Indicator(anchor))
            .expect("anchor legend row");
        assert!(row.values.is_empty(), "{:?}", row.values);
    }
    // Probe every zone centre and every marker, plus a grid over the pane.
    let mut probes = Vec::new();
    for annotations in &annotations {
        for zone in annotations.zones() {
            probes.push((zone.start_row, (zone.top + zone.bottom) * 0.5));
        }
        for marker in annotations.markers() {
            probes.push((marker.row, marker.price));
        }
    }
    let mut points = probes
        .into_iter()
        .filter_map(|(row, price)| {
            Some((
                chart.logical_to_coordinate(row as f64)?,
                chart.series_price_to_coordinate(0, price)?,
            ))
        })
        .collect::<Vec<_>>();
    assert!(!points.is_empty());
    points.extend((0..80).flat_map(|x| (0..50).map(move |y| (x as f64 * 10.0, y as f64 * 10.0))));
    let mut candle_hits = 0;
    for (x, y) in points {
        if let Some(hit) = chart.hit_test_series(x, y) {
            assert!(!anchors.contains(&hit), "anchor {hit} hit at ({x}, {y})");
            candle_hits += usize::from(hit == 0);
        }
    }
    // The probes reach the plotted bars, so the anchors' absence is not vacuous.
    assert!(candle_hits > 0);
}

#[test]
fn structure_studies_refuse_as_of_sources_atomically() {
    use crate::indicators::StructureBreakOn;
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    let days = |list: &[f64]| list.iter().map(|day| day * 86_400.0).collect::<Vec<_>>();
    let close = [100.0, 102.0, 101.0, 105.0, 104.0, 103.0, 107.0];
    chart
        .set_series_data(
            0,
            &days(&[1.0, 2.0, 3.0, 5.0, 6.0, 8.0, 9.0]),
            &close,
            &close,
            &close,
            &close,
        )
        .unwrap();
    let overlay = chart.add_series(SeriesKind::Candlestick);
    let other = [10.0, 12.0, 11.0, 15.0, 14.0, 13.0, 17.0];
    chart
        .set_series_data(
            overlay,
            &days(&[1.0, 2.0, 4.0, 5.0, 7.0, 8.0, 10.0]),
            &other,
            &other,
            &other,
            &other,
        )
        .unwrap();
    let as_of = TimeAlignment::AsOf {
        max_staleness: None,
    };
    chart.set_series_time_alignment(overlay, as_of).unwrap();
    let swings = IndicatorKind::SwingPoints { left: 1, right: 1 };
    let structure = IndicatorKind::MarketStructure {
        left: 1,
        right: 1,
        break_on: StructureBreakOn::Close,
    };
    let before = chart.export_state_json().unwrap();
    // The as-of overlay and an indicator output that follows its alignment are both refused,
    // leaving the chart unchanged; a scalar study on the overlay is still accepted.
    assert!(
        chart
            .add_indicator_kind(overlay, swings.clone(), None)
            .is_empty()
    );
    assert!(
        chart
            .add_indicator_kind(overlay, structure, None)
            .is_empty()
    );
    assert_eq!(chart.export_state_json().unwrap(), before);
    let sma = chart.add_indicator_kind(overlay, IndicatorKind::Sma { period: 2 }, None);
    assert_eq!(sma.len(), 1);
    assert!(
        chart
            .add_indicator_kind(sma[0], swings.clone(), None)
            .is_empty()
    );
    // A structure study on a union series keeps that series off the as-of alignment.
    assert_eq!(chart.add_indicator_kind(0, swings.clone(), None).len(), 2);
    let before = chart.export_state_json().unwrap();
    assert_eq!(
        chart
            .set_series_time_alignment(0, as_of)
            .unwrap_err()
            .code(),
        ErrorCode::UnsupportedOperation
    );
    assert_eq!(chart.export_state_json().unwrap(), before);
    assert_eq!(chart.series_time_alignment(0), Some(TimeAlignment::Union));
    // So does one chained on an indicator output of the series, which follows its alignment.
    let chained = chart.add_series(SeriesKind::Candlestick);
    chart
        .set_series_data(
            chained,
            &days(&[1.0, 2.0, 3.0, 5.0, 6.0, 8.0, 9.0]),
            &other,
            &other,
            &other,
            &other,
        )
        .unwrap();
    let sma = chart.add_indicator_kind(chained, IndicatorKind::Sma { period: 2 }, None);
    assert_eq!(sma.len(), 1);
    assert_eq!(chart.add_indicator_kind(sma[0], swings, None).len(), 2);
    let before = chart.export_state_json().unwrap();
    assert_eq!(
        chart
            .set_series_time_alignment(chained, as_of)
            .unwrap_err()
            .code(),
        ErrorCode::UnsupportedOperation
    );
    assert_eq!(chart.export_state_json().unwrap(), before);
    assert_eq!(
        chart.series_time_alignment(chained),
        Some(TimeAlignment::Union)
    );
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
        assert!(
            !chart
                .add_indicator_kind(0, kind, weighted.then_some(volume))
                .is_empty()
        );
    }
    let rsi = chart.add_indicator_kind(
        0,
        IndicatorKind::Rsi {
            period: 5,
            seed: IndicatorSeed::Sma,
        },
        None,
    )[0];
    assert!(
        !chart
            .add_indicator_kind(
                rsi,
                IndicatorKind::Ema {
                    period: 4,
                    seed: IndicatorSeed::Sma
                },
                None
            )
            .is_empty()
    );
    let assert_bounded = |chart: &ChartEngine, mutation: &str| {
        for binding in &chart.indicators {
            let bound = indicator_tick_work_bound(binding, WORK_PER_BINDING);
            assert!(
                binding.last_work_rows() <= bound,
                "{mutation}: {:?} did {} work rows (bound {bound})",
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
        assert!(
            !chart
                .add_indicator_kind(0, kind, weighted.then_some(volume))
                .is_empty()
        );
    }
    assert!(
        !chart
            .add_indicator_kind_with_sources(
                0,
                IndicatorInputSource::Hlc3,
                IndicatorKind::Vwap,
                Some(volume),
                Some(amount),
            )
            .is_empty()
    );
    let rsi = chart.add_indicator_kind(
        0,
        IndicatorKind::Rsi {
            period: 5,
            seed: IndicatorSeed::Sma,
        },
        None,
    )[0];
    assert!(
        !chart
            .add_indicator_kind(rsi, IndicatorKind::Sma { period: 3 }, None)
            .is_empty()
    );
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
fn live_appends_after_the_first_grow_no_series_storage() {
    // The first append past a bulk install grows each exactly sized column once (Target M
    // reports it apart). Later appends must grow nothing: each study output reaches its next
    // summary-pyramid node on its own tick (its warm-up offsets its rows from the source's), so
    // levels sized exactly by the install would each be copied on one of those live ticks.
    const ROWS: usize = 5_000;
    let (mut chart, volume) = replay_study_chart(ROWS);
    let amount = chart
        .indicators
        .iter()
        .find_map(|binding| binding.amount_source)
        .unwrap();
    let append = |chart: &mut ChartEngine, row: usize| {
        let time = row as f64 * 60.0;
        let weight = (row % 13 + 1) as f64;
        chart.update_series_bar(0, time, swinging_bar(row, 0));
        chart.update_series_bar(volume, time, [weight; 4]);
        chart.update_series_bar(amount, time, [weight * 101.0; 4]);
    };
    append(&mut chart, ROWS);
    let capacity = chart.memory_usage().data.allocated_capacity_bytes;
    for row in ROWS + 1..=ROWS + 2 * aeris_charts_core::model::lod::LOD_FANOUT {
        append(&mut chart, row);
        assert_eq!(
            chart.memory_usage().data.allocated_capacity_bytes,
            capacity,
            "appending row {row}"
        );
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
    let kinds = scalar_indicator_kinds();
    for kind in kinds {
        let setup = || {
            let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
            let volume = chart.add_series(SeriesKind::Histogram);
            let times = (0..40)
                .map(|index| index as f64 * 3_600.0)
                .collect::<Vec<_>>();
            let mut close = (0..40)
                .map(|index| 90.0 + index as f64 * 0.4)
                .collect::<Vec<_>>();
            let mut high = close.iter().map(|value| value + 2.0).collect::<Vec<_>>();
            let mut low = close.iter().map(|value| value - 1.0).collect::<Vec<_>>();
            close[20] = f64::NAN;
            high[20] = f64::NAN;
            low[20] = f64::NAN;
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
            (42.0 * 3_600.0, [f64::NAN; 4]),
            (43.0 * 3_600.0, [107.0, 109.0, 106.0, 108.0]),
        ];
        for (time, values) in rows {
            singles.update_series_bar(0, time, values);
        }
        assert_eq!(batch.update_series_bars(0, rows), 5);

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
            for (single_column, batched_column) in single.1.iter().zip(batched.1.iter()) {
                assert_eq!(single_column.len(), batched_column.len());
                for (&single_value, &batched_value) in
                    single_column.iter().zip(batched_column.iter())
                {
                    assert!(
                        single_value == batched_value
                            || (single_value.is_nan() && batched_value.is_nan()),
                        "{kind:?} output {series}: {single_value} != {batched_value}"
                    );
                }
            }
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
    assert!(
        axis.labels
            .iter()
            .any(|l| l.midpoint == AxisTextMidpoint::StableTime && l.text == "XHAIR")
    );

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
fn indicator_binding_owns_group_chrome_visibility_and_removal() {
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
    let chrome = IndicatorChromeOptions {
        name_labels_visible: false,
        value_labels_visible: false,
        price_lines_visible: false,
    };
    assert!(chart.set_indicator_chrome_options(chrome));
    let outputs = chart.add_macd(0, 2, 3, 2);
    assert_eq!(outputs.len(), 3);
    assert_eq!(chart.indicator_chrome_options(), chrome);
    assert!(outputs.iter().all(|output| {
        chart.series_entry(*output).is_some_and(|series| {
            !series.title_visible && !series.last_value_visible && !series.price_line_visible
        })
    }));

    chart
        .series_entry_mut(outputs[0])
        .expect("MACD output exists")
        .title_visible = true;
    assert!(
        chart.set_indicator_chrome_options(chrome),
        "reapplying the retained policy must repair a drifted output"
    );
    assert!(
        !chart
            .series_entry(outputs[0])
            .expect("MACD output exists")
            .title_visible
    );

    assert!(chart.set_indicator_binding_visible(outputs[0], false));
    assert!(outputs.iter().all(|output| {
        chart
            .series_entry(*output)
            .is_some_and(|series| !series.visible)
    }));
    assert!(chart.remove_indicator_for_series(outputs[1]));
    assert!(
        outputs
            .iter()
            .all(|output| chart.series_entry(*output).is_none())
    );
    assert!(chart.indicator_bindings().is_empty());
    assert!(!chart.has_indicator_bindings());

    let sma = chart.add_sma(0, 2).expect("valid SMA");
    let rsi = chart.add_rsi(0, 2).expect("valid RSI");
    assert!(chart.has_indicator_bindings());
    assert!(chart.clear_indicator_bindings());
    assert!(!chart.has_indicator_bindings());
    assert!(chart.series_entry(sma).is_none());
    assert!(chart.series_entry(rsi).is_none());
    assert!(!chart.clear_indicator_bindings());
}

#[test]
fn a_selected_scale_price_format_carries_to_series_that_join_the_scale() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    let values = [10.0, 11.0, 12.0];
    chart
        .set_series_data(0, &[1.0, 2.0, 3.0], &values, &values, &values, &values)
        .unwrap();
    let left = chart.add_series(SeriesKind::Line);
    chart.set_series_price_scale(left, PriceScaleTarget::Left);
    let volume = chart.add_series(SeriesKind::Histogram);
    assert!(chart.series_apply_price_format_json(volume, r#"{"type":"volume"}"#));
    chart.set_series_price_scale(volume, PriceScaleTarget::Left);
    assert!(chart.set_price_format_for_scale(0, PriceScaleTarget::Right, 0, 1.0));

    let joined = chart.add_series(SeriesKind::Line);
    assert_eq!(
        chart.series_entry(joined).unwrap().price_format.precision,
        0
    );
    chart.set_series_price_scale(left, PriceScaleTarget::Right);
    assert_eq!(chart.series_entry(left).unwrap().price_format.precision, 0);
    // A volume series keeps its own format on any scale.
    chart.set_series_price_scale(volume, PriceScaleTarget::Right);
    assert_eq!(
        chart.series_entry(volume).unwrap().price_format.kind,
        PriceFormatKind::Volume
    );
    let sma = chart.add_sma(0, 2).expect("valid SMA");
    assert_eq!(chart.series_entry(sma).unwrap().price_format.precision, 0);
    assert!(chart.try_set_series_pane(sma, 1, 0.3));
    assert_eq!(chart.series_entry(sma).unwrap().price_format.precision, 0);

    // Moving the scale's series to another built-in scale carries the selection with them.
    assert!(chart.rebind_price_scale_series(0, PriceScaleTarget::Right, PriceScaleTarget::Left));
    let late = chart.add_series(SeriesKind::Line);
    chart.set_series_price_scale(late, PriceScaleTarget::Left);
    assert_eq!(chart.series_entry(late).unwrap().price_format.precision, 0);
}

#[test]
fn price_scale_series_operations_are_typed_and_atomic() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    let values = [10.0, 11.0, 12.0];
    chart
        .set_series_data(0, &[1.0, 2.0, 3.0], &values, &values, &values, &values)
        .unwrap();
    let second = chart.add_series(SeriesKind::Line);
    chart
        .set_series_data(second, &[1.0, 2.0, 3.0], &values, &values, &values, &values)
        .unwrap();
    chart.set_series_price_scale(second, PriceScaleTarget::Left);

    assert_eq!(
        chart
            .primary_series_on_price_scale(0, PriceScaleTarget::Right)
            .map(|series| series.series_id),
        Some(0)
    );
    assert_eq!(
        chart
            .primary_series_on_price_scale(0, PriceScaleTarget::Left)
            .map(|series| series.series_id),
        Some(second)
    );
    assert_eq!(chart.series_visible(second), Some(true));
    let align = chart
        .price_scale_for(0, PriceScaleTarget::Right)
        .unwrap()
        .options()
        .align_labels;
    assert!(chart.toggle_price_scale_align_labels(0, PriceScaleTarget::Right));
    assert_eq!(
        chart
            .price_scale_for(0, PriceScaleTarget::Right)
            .unwrap()
            .options()
            .align_labels,
        !align
    );
    assert!(chart.toggle_series_chrome(0, SeriesChromeFlag::PriceLine));
    assert!(!chart.series_entry(0).unwrap().price_line_visible);

    assert!(chart.set_price_format_for_scale(0, PriceScaleTarget::Right, 4, 0.0001));
    assert_eq!(chart.series_entry(0).unwrap().price_format.precision, 4);
    assert_eq!(
        chart.series_entry(second).unwrap().price_format.precision,
        2
    );
    assert!(!chart.set_price_format_for_scale(0, PriceScaleTarget::Right, 4, f64::NAN));

    assert!(chart.rebind_price_scale_series(0, PriceScaleTarget::Right, PriceScaleTarget::Left));
    assert_eq!(
        chart.series_entry(0).unwrap().price_scale_target,
        PriceScaleTarget::Left
    );
    assert!(!chart.price_scale_visible_for(0, PriceScaleTarget::Right));
    assert!(chart.price_scale_visible_for(0, PriceScaleTarget::Left));
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
    assert!(
        ids.iter()
            .all(|&id| chart.series.iter().find(|s| s.id == id).unwrap().pane_index == 1)
    );
    // Output slots and the packed signal period.
    assert_eq!(chart.indicator_info(ids[0]).unwrap().output_index, 0);
    assert_eq!(chart.indicator_info(ids[2]).unwrap().output_index, 2);
    let info = chart.indicator_info(ids[0]).unwrap();
    assert_eq!(
        (info.kind.as_ref(), info.period, info.deviation),
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
fn volume_oscillator_uses_one_pane_and_histogram_output() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    let values = [10.0, 20.0, 30.0, 20.0, 10.0, 30.0];
    let times = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
    chart
        .set_series_data(0, &times, &values, &values, &values, &values)
        .unwrap();
    let volume = chart.add_series(SeriesKind::Histogram);
    chart
        .set_series_data(volume, &times, &values, &values, &values, &values)
        .unwrap();
    let ids = chart.add_volume_oscillator(0, volume, 2, 3, 2);
    assert_eq!(ids.len(), 3);
    assert_eq!(chart.series_kind(ids[0]), Some(SeriesKind::Line));
    assert_eq!(chart.series_kind(ids[1]), Some(SeriesKind::Line));
    assert_eq!(chart.series_kind(ids[2]), Some(SeriesKind::Histogram));
    assert!(ids.iter().all(|&id| {
        chart
            .series
            .iter()
            .find(|series| series.id == id)
            .unwrap()
            .pane_index
            == 1
    }));
    assert_eq!(
        chart.indicator_info(ids[2]).unwrap().output_name,
        "Histogram"
    );
    assert_indicator_binding_matches_full(&chart, 0);
}

#[test]
fn ease_of_movement_requires_positive_divisor_and_uses_oscillator_pane() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    let values = [10.0, 20.0, 30.0];
    let times = [1.0, 2.0, 3.0];
    chart
        .set_series_data(0, &times, &values, &values, &values, &values)
        .unwrap();
    let volume = chart.add_series(SeriesKind::Histogram);
    chart
        .set_series_data(volume, &times, &values, &values, &values, &values)
        .unwrap();
    assert!(chart.add_ease_of_movement(0, volume, 2, 0.0).is_none());
    let output = chart.add_ease_of_movement(0, volume, 2, 100.0).unwrap();
    assert_eq!(chart.series_kind(output), Some(SeriesKind::Line));
    assert_eq!(
        chart
            .series
            .iter()
            .find(|series| series.id == output)
            .unwrap()
            .pane_index,
        1
    );
    assert_eq!(
        chart.indicator_info(output).unwrap().parameters.divisor,
        Some(100.0)
    );
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
        (info.kind.as_ref(), info.period, info.deviation),
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
    assert!(
        ids.iter()
            .all(|&id| chart.indicator_info(id).unwrap().binding_id == ids[0])
    );
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

    assert!(
        chart
            .add_indicator_kind(u32::MAX, IndicatorKind::Sma { period: 2 }, None)
            .is_empty()
    );
    assert!(
        chart
            .add_indicator_kind(0, IndicatorKind::Sma { period: 0 }, None)
            .is_empty()
    );
    assert!(
        chart
            .add_indicator_kind(
                0,
                IndicatorKind::Keltner {
                    period: 14,
                    multiplier: -1.0,
                },
                None,
            )
            .is_empty()
    );
    assert!(
        chart
            .add_indicator_kind(0, IndicatorKind::AdxDmi { period: 0 }, None)
            .is_empty()
    );
    assert!(
        chart
            .add_indicator_kind(stale, IndicatorKind::Sma { period: 2 }, None)
            .is_empty()
    );
    assert!(
        chart
            .add_indicator_kind(0, IndicatorKind::Vwap, Some(u32::MAX))
            .is_empty()
    );
    assert!(
        chart
            .add_indicator_kind(0, IndicatorKind::Vwap, Some(stale))
            .is_empty()
    );
    assert!(
        chart
            .add_indicator_kind(0, IndicatorKind::Vwap, Some(candle_volume))
            .is_empty()
    );
    assert!(
        chart
            .add_indicator_kind(0, IndicatorKind::Vwap, Some(0))
            .is_empty()
    );
    assert!(
        chart
            .add_indicator_kind(
                0,
                IndicatorKind::VwapBands {
                    reset: VwapReset::Session,
                    standard_deviation: f64::NAN,
                    percent: 10.0,
                },
                None,
            )
            .is_empty()
    );
    assert!(
        chart
            .add_indicator_kind(
                0,
                IndicatorKind::Rsi {
                    period: 2,
                    seed: IndicatorSeed::Sma,
                },
                Some(0)
            )
            .is_empty()
    );
    for kind in [
        IndicatorKind::Kst {
            roc: [2, 0, 4, 5],
            smoothing: [2, 3, 4, 5],
            signal: 2,
        },
        IndicatorKind::Kst {
            roc: [2, 3, 4, 5],
            smoothing: [2, 3, 0, 5],
            signal: 2,
        },
        IndicatorKind::Kst {
            roc: [2, 3, 4, 5],
            smoothing: [2, 3, 4, 5],
            signal: 0,
        },
        IndicatorKind::Tsi {
            long: 0,
            short: 3,
            signal: 2,
        },
        IndicatorKind::Tsi {
            long: 5,
            short: 0,
            signal: 2,
        },
        IndicatorKind::Tsi {
            long: 5,
            short: 3,
            signal: 0,
        },
        IndicatorKind::MassIndex {
            ema_period: 0,
            sum_period: 5,
        },
        IndicatorKind::MassIndex {
            ema_period: 3,
            sum_period: 0,
        },
        IndicatorKind::Vortex { period: 0 },
    ] {
        assert!(chart.add_indicator_kind(0, kind, None).is_empty());
    }

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
    assert!(
        chart
            .indicator_bindings()
            .iter()
            .any(|binding| { binding.outputs.contains(&sma) && binding.source == rsi })
    );
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
        assert_eq!(output_times, &[86_400, 90_000]);
        let expected = [
            expected[2].pivot,
            expected[2].resistance_1,
            expected[2].support_1,
            expected[2].resistance_2,
            expected[2].support_2,
        ][output_index]
            .unwrap();
        let actual = values[3][0];
        assert!((actual - expected).abs() < 1e-12, "output {output_index}");
        assert_eq!(values[3][1], values[3][0]);
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
    // Revision 3 introduced the boolean parameter type and revision 4 renamed the choice list to
    // `options`; hosts key editors on this number.
    assert_eq!(INDICATOR_SCHEMA_REVISION, 4);
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

    let envelopes = ChartEngine::indicator_schema(&IndicatorKind::Envelopes {
        period: 20,
        percent: 2.5,
        exponential: true,
    });
    let exponential = envelopes
        .parameters
        .iter()
        .find(|parameter| parameter.name == "exponential")
        .unwrap();
    assert_eq!(exponential.parameter_type, IndicatorParameterType::Boolean);
    assert_eq!(exponential.default, serde_json::json!(true));
    assert_eq!((exponential.min, exponential.max), (None, None));

    for (kind, names, defaults, outputs) in [
        (
            IndicatorKind::Kst {
                roc: [2, 3, 4, 5],
                smoothing: [6, 7, 8, 9],
                signal: 10,
            },
            vec![
                "roc_1",
                "roc_2",
                "roc_3",
                "roc_4",
                "smoothing_1",
                "smoothing_2",
                "smoothing_3",
                "smoothing_4",
                "signal",
            ],
            vec![2, 3, 4, 5, 6, 7, 8, 9, 10],
            vec!["KST", "Signal"],
        ),
        (
            IndicatorKind::Tsi {
                long: 11,
                short: 12,
                signal: 13,
            },
            vec!["long", "short", "signal"],
            vec![11, 12, 13],
            vec!["TSI", "Signal"],
        ),
        (
            IndicatorKind::MassIndex {
                ema_period: 14,
                sum_period: 15,
            },
            vec!["ema_period", "sum_period"],
            vec![14, 15],
            vec!["Mass Index"],
        ),
        (
            IndicatorKind::Vortex { period: 16 },
            vec!["period"],
            vec![16],
            vec!["VI+", "VI-"],
        ),
    ] {
        let schema = ChartEngine::indicator_schema(&kind);
        assert_eq!(
            schema
                .parameters
                .iter()
                .skip(1)
                .map(|parameter| parameter.name.as_str())
                .collect::<Vec<_>>(),
            names
        );
        assert_eq!(
            schema
                .parameters
                .iter()
                .skip(1)
                .map(|parameter| parameter.default.as_u64().unwrap())
                .collect::<Vec<_>>(),
            defaults
        );
        assert_eq!(
            schema
                .outputs
                .iter()
                .map(|output| output.name.as_str())
                .collect::<Vec<_>>(),
            outputs
        );
    }
}

#[test]
fn adaptive_regression_and_klinger_bindings_validate_and_place_outputs() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    let volume = chart.add_series(SeriesKind::Histogram);
    let times = (0..24).map(|row| row as f64).collect::<Vec<_>>();
    let values = (0..24).map(|row| 10.0 + row as f64).collect::<Vec<_>>();
    chart
        .set_series_data(0, &times, &values, &values, &values, &values)
        .unwrap();
    chart
        .set_series_data(volume, &times, &values, &values, &values, &values)
        .unwrap();

    for (kind, volume_source) in [
        (
            IndicatorKind::Klinger {
                fast: 3,
                slow: 7,
                signal: 4,
            },
            Some(volume),
        ),
        (
            IndicatorKind::Kama {
                period: 5,
                fast: 2,
                slow: 10,
            },
            None,
        ),
        (IndicatorKind::McGinley { period: 5 }, None),
        (
            IndicatorKind::LinearRegression {
                period: 5,
                deviation: 2.0,
            },
            None,
        ),
    ] {
        let schema = ChartEngine::indicator_schema(&kind);
        assert_eq!(schema.kind, serde_json::to_value(&kind).unwrap()["kind"]);
        let outputs = chart.add_indicator_kind(0, kind.clone(), volume_source);
        assert_eq!(outputs.len(), schema.outputs.len());
        assert!(
            outputs
                .iter()
                .all(|&output| { chart.indicator_info(output).unwrap().kind == schema.kind })
        );
        let pane = chart.series_entry(outputs[0]).unwrap().pane_index;
        assert!(
            outputs
                .iter()
                .all(|&output| chart.series_entry(output).unwrap().pane_index == pane)
        );
        assert_eq!(pane != 0, matches!(kind, IndicatorKind::Klinger { .. }));
    }

    for (kind, volume_source) in [
        (
            IndicatorKind::Klinger {
                fast: 3,
                slow: 7,
                signal: 4,
            },
            None,
        ),
        (
            IndicatorKind::Klinger {
                fast: 7,
                slow: 7,
                signal: 4,
            },
            Some(volume),
        ),
        (
            IndicatorKind::Klinger {
                fast: 3,
                slow: 7,
                signal: 0,
            },
            Some(volume),
        ),
        (
            IndicatorKind::Klinger {
                fast: 3,
                slow: 7,
                signal: 4,
            },
            Some(0),
        ),
        (
            IndicatorKind::Kama {
                period: 5,
                fast: 10,
                slow: 2,
            },
            None,
        ),
        (IndicatorKind::McGinley { period: 0 }, None),
        (
            IndicatorKind::LinearRegression {
                period: 5,
                deviation: -1.0,
            },
            None,
        ),
    ] {
        assert!(chart.add_indicator_kind(0, kind, volume_source).is_empty());
    }
}

#[test]
fn negative_bollinger_deviation_is_rejected_without_changing_the_chart() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    let values = [10.0, 11.0, 12.0, 11.0, 10.0];
    chart
        .set_series_data(
            0,
            &[0.0, 1.0, 2.0, 3.0, 4.0],
            &values,
            &values,
            &values,
            &values,
        )
        .unwrap();
    let before = chart.export_state_json().unwrap();
    assert!(chart.add_bollinger(0, 3, -1.0).is_empty());
    assert!(chart.add_bollinger_metrics(0, 3, -1.0).is_empty());
    assert!(chart.indicator_bindings().is_empty());
    assert_eq!(chart.export_state_json().unwrap(), before);
    assert_eq!(chart.add_bollinger(0, 3, 0.0).len(), 3);
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
fn crosshair_shade_right_round_trips_and_resets_only_its_color() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    let defaults = &chart.options.get().crosshair.shade_right;
    assert!(!defaults.visible);
    assert_eq!(defaults.color, "rgba(74, 74, 74, 0.12)");

    chart
        .apply_options(r##"{"crosshair":{"shadeRight":{"visible":true,"color":"#ff000040"}}}"##)
        .unwrap();
    let shade = &chart.options.get().crosshair.shade_right;
    assert!(shade.visible);
    assert_eq!(shade.color, "#ff000040");
    let raw = chart.options.value();
    assert_eq!(raw["crosshair"]["shadeRight"]["visible"], true);
    assert_eq!(raw["crosshair"]["shadeRight"]["color"], "#ff000040");

    // The tint is style; whether the veil is on is host state (the watermark precedent).
    chart.reset_style_to_defaults();
    let shade = &chart.options.get().crosshair.shade_right;
    assert!(shade.visible);
    assert_eq!(shade.color, "rgba(74, 74, 74, 0.12)");
}

#[test]
fn v2_persistence_carries_the_crosshair_shade() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    chart
        .add_pane_with_domain(true, HorizontalDomain::Temporal)
        .unwrap();
    chart
        .apply_options(
            r##"{"crosshair":{"shadeRight":{"visible":true,"color":"rgba(1, 2, 3, 0.5)"}}}"##,
        )
        .unwrap();
    let document = chart.export_state_json().unwrap();
    let value: serde_json::Value = serde_json::from_str(&document).unwrap();
    assert_eq!(value["schema_version"], 2);
    assert_eq!(
        value["chart_options"]["crosshair"]["shadeRight"]["visible"],
        true
    );

    let mut restored = ChartEngine::new(800.0, 500.0, 1.0);
    restored.import_state_json(&document).unwrap();
    let shade = &restored.options.get().crosshair.shade_right;
    assert!(shade.visible);
    assert_eq!(shade.color, "rgba(1, 2, 3, 0.5)");
    assert_eq!(restored.export_state_json().unwrap(), document);
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
    assert!(
        axis.labels
            .iter()
            .any(|l| matches!(l.background, Some((.., c)) if c == price_bg))
    );
    assert!(
        axis.labels
            .iter()
            .any(|l| l.midpoint == AxisTextMidpoint::StableTime
                && matches!(l.background, Some((.., c)) if c == time_bg))
    );

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
    assert!(
        !axis
            .labels
            .iter()
            .any(|l| matches!(l.background, Some((.., c)) if c == price_bg))
    );
    assert!(
        !axis
            .labels
            .iter()
            .any(|l| l.midpoint == AxisTextMidpoint::StableTime)
    );
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
    assert!(
        frame.panes[0]
            .main
            .iter()
            .any(|p| matches!(p, Prim::Polyline { color, .. } if *color == translucent))
    );
    assert!(
        frame.panes[0]
            .main
            .iter()
            .any(|p| matches!(p, Prim::Rect { color, .. } if *color == translucent))
    );

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
        "baseline_mode",
        "baseline_line_visible",
        "baseline_line_color",
        "baseline_line_width",
        "baseline_line_style",
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
    assert_eq!(options["baseline_mode"], "visible_midpoint");
    assert_eq!(options["baseline_line_visible"], false);
    assert_eq!(options["baseline_line_color"], "");
    assert_eq!(options["baseline_line_width"], 1.0);
    assert_eq!(options["baseline_line_style"], 2);
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
        "baseline_mode": "close_before_visible_range",
        "baseline_line_visible": true,
        "baseline_line_color": "#131415",
        "baseline_line_width": 2.5,
        "baseline_line_style": 0,
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
    assert_eq!(options["baseline_mode"], "close_before_visible_range");
    assert_eq!(options["baseline_line_visible"], true);
    assert_eq!(options["baseline_line_color"], "#131415");
    assert_eq!(options["baseline_line_width"], 2.5);
    assert_eq!(options["baseline_line_style"], 0);
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
        r#"{"price_line_color": "", "point_markers_radius": null, "top_line_width": null,
            "baseline_line_color": null}"#
    ));
    let options: serde_json::Value =
        serde_json::from_str(&chart.series_options_json(0).unwrap()).unwrap();
    assert_eq!(options["price_line_color"], "");
    assert_eq!(options["point_markers_radius"], serde_json::Value::Null);
    assert_eq!(options["top_line_width"], serde_json::Value::Null);
    assert_eq!(options["baseline_line_color"], "");

    // Wrong types and out-of-range values leave the baseline line keys untouched.
    assert!(chart.series_apply_options_json(
        0,
        r#"{"baseline_mode": "previous_close", "baseline_line_visible": 1,
            "baseline_line_width": 0, "baseline_line_style": 9, "baseline_line_color": 7}"#
    ));
    let options: serde_json::Value =
        serde_json::from_str(&chart.series_options_json(0).unwrap()).unwrap();
    assert_eq!(options["baseline_mode"], "close_before_visible_range");
    assert_eq!(options["baseline_line_visible"], true);
    assert_eq!(options["baseline_line_width"], 2.5);
    assert_eq!(options["baseline_line_style"], 0);
    assert_eq!(options["baseline_line_color"], "");
}

#[test]
fn style_reset_restores_the_baseline_line_and_keeps_the_baseline_mode() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    chart.series[0].kind = SeriesKind::Baseline;
    assert!(chart.series_apply_options_json(
        0,
        r##"{"baseline_mode": "close_before_visible_range", "baseline_line_visible": true,
            "baseline_line_color": "#131415", "baseline_line_width": 3, "baseline_line_style": 1}"##
    ));
    chart.reset_style_to_defaults();
    let series = &chart.series[0];
    assert_eq!(series.baseline_mode, BaselineMode::CloseBeforeVisibleRange);
    assert!(!series.baseline_line_visible);
    assert_eq!(series.baseline_line_color, None);
    assert_eq!(series.baseline_line_width, 1.0);
    assert_eq!(series.baseline_line_style, 2);
}

#[test]
fn series_baseline_price_reports_the_pinned_or_resolved_baseline_of_baseline_series_only() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    let times = [1.0, 2.0, 3.0, 4.0];
    let values = [10.0, 20.0, 25.0, 30.0];
    chart
        .set_series_data(0, &times, &values, &values, &values, &values)
        .unwrap();
    chart.time_scale.set_width(800.0);
    chart.fit_content();

    // Not a Baseline series, unknown id, removed id: None.
    assert_eq!(chart.series_baseline_price(0), None);
    assert_eq!(chart.series_baseline_price(99), None);
    let extra = chart.add_series(SeriesKind::Baseline);
    chart
        .set_series_data(extra, &times, &values, &values, &values, &values)
        .unwrap();
    assert_eq!(chart.series_baseline_price(extra), Some(20.0));
    chart.remove_series(extra);
    assert_eq!(chart.series_baseline_price(extra), None);

    // Auto midpoint, then the mode, then the pin; the pin wins over every mode.
    chart.series[0].kind = SeriesKind::Baseline;
    assert_eq!(chart.series_baseline_price(0), Some(20.0));
    assert!(
        chart.series_apply_options_json(0, r#"{"baseline_mode": "close_before_visible_range"}"#)
    );
    assert_eq!(chart.series_baseline_price(0), Some(10.0));
    chart.set_lock_visible_logical_range(true);
    chart.set_visible_logical_range(2.0, 3.0);
    assert_eq!(chart.series_baseline_price(0), Some(20.0));
    chart.series[0].baseline = Some(9.5);
    assert_eq!(chart.series_baseline_price(0), Some(9.5));

    // A shorter Baseline series with no row in a window another series keeps visible: the
    // midpoint has nothing to average, the close before the window is its last close, and a pin
    // reads back as set.
    let short = chart.add_series(SeriesKind::Baseline);
    chart
        .set_series_data(
            short,
            &times[..2],
            &values[..2],
            &values[..2],
            &values[..2],
            &values[..2],
        )
        .unwrap();
    chart.set_visible_logical_range(2.0, 3.0);
    assert_eq!(chart.visible_range(), Some((2, 3)));
    assert_eq!(chart.series_baseline_price(short), None);
    assert!(
        chart
            .series_apply_options_json(short, r#"{"baseline_mode": "close_before_visible_range"}"#)
    );
    assert_eq!(chart.series_baseline_price(short), Some(20.0));
    chart.series[1].baseline = Some(7.0);
    assert_eq!(chart.series_baseline_price(short), Some(7.0));

    // No visible range yet: None.
    let chart = ChartEngine::new(800.0, 500.0, 1.0);
    assert_eq!(chart.series_baseline_price(0), None);
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
    assert!(
        chart
            .set_series_data_styled(
                0,
                &[1.0, 2.0],
                &[1.0, 2.0],
                &[1.0, 2.0],
                &[1.0, 2.0],
                &[1.0, 2.0],
                [Some(vec![1]), None, None],
            )
            .is_err()
    );
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
    assert!(
        chart.series_apply_price_format_json(
            0,
            r#"{"type":"price","precision":4,"min_move":0.0001}"#
        )
    );
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
    assert!(
        frame.panes[0]
            .main
            .iter()
            .any(|p| matches!(p, aeris_charts_render::draw_list::Prim::VLine { .. }))
    );
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
fn shared_time_formatter_matches_the_zoned_axis_label() {
    let ts = 1_767_229_200.0; // 2026-01-01 01:00 UTC, previous day in New York.
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    chart
        .set_series_data(0, &[ts], &[10.0], &[10.0], &[10.0], &[10.0])
        .unwrap();
    chart.time_scale.set_width(800.0);
    chart.fit_content();
    chart.set_date_format("yyyy-MM-dd");
    chart.set_time_zone("America/New_York").unwrap();
    let formatted = chart.format_crosshair_ts(ts as i64);
    assert_eq!(formatted, "2025-12-31   20:00");
    assert_eq!(
        crosshair_time_label(&mut chart, ts).as_deref(),
        Some(formatted.as_str())
    );
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
fn restating_the_line_pulse_default_does_not_carry_the_pulse_onto_candles() {
    use aeris_charts_render::draw_list::Prim;
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    install_bars(&mut chart, 20);
    chart.fit_content();
    chart.convert_series_kind(0, SeriesKind::Line);
    // Hosts re-send their whole style on every apply, including the default-on line pulse.
    assert!(chart.set_series_last_price_animation(0, true));
    assert!(chart.last_price_pulse_active());
    chart.convert_series_kind(0, SeriesKind::Candlestick);
    assert!(
        !chart.last_price_pulse_active(),
        "candles stop the animation clock"
    );
    let frame = chart.build_frame();
    assert!(
        !frame.panes[0]
            .main
            .iter()
            .any(|prim| matches!(prim, Prim::Circle { .. })),
        "candles paint no live-price pulse"
    );
    chart.convert_series_kind(0, SeriesKind::Area);
    assert!(chart.last_price_pulse_active(), "area restores its default");
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

// ---- live-bar easing ----

/// Ten candles (open 100 + i, high +2, low -2, close +1 at times 1..=10) with live-bar easing on.
fn eased_candles(tau_ms: f64) -> ChartEngine {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    chart.time_scale.set_width(800.0);
    let times: Vec<f64> = (1..=10).map(|i| i as f64).collect();
    let open: Vec<f64> = (0..10).map(|i| 100.0 + i as f64).collect();
    let high: Vec<f64> = open.iter().map(|v| v + 2.0).collect();
    let low: Vec<f64> = open.iter().map(|v| v - 2.0).collect();
    let close: Vec<f64> = open.iter().map(|v| v + 1.0).collect();
    chart
        .set_series_data(0, &times, &open, &high, &low, &close)
        .unwrap();
    assert!(chart.series_apply_options_json(0, &format!(r#"{{"live_bar_easing_ms":{tau_ms}}}"#)));
    chart.fit_content();
    chart
}

/// The last bar as the frame draws it (`[open, high, low, close]`).
fn displayed(chart: &ChartEngine, id: SeriesId) -> [f64; 4] {
    let plot = chart.display_plot(id);
    let row = plot.size() - 1;
    [
        PlotValueIndex::Open,
        PlotValueIndex::High,
        PlotValueIndex::Low,
        PlotValueIndex::Close,
    ]
    .map(|index| plot.value_at(row, index))
}

const OLD_LAST: [f64; 4] = [109.0, 111.0, 107.0, 110.0];
const NEW_LAST: [f64; 4] = [109.0, 115.0, 105.0, 114.0];

fn between(value: f64, a: f64, b: f64) -> bool {
    (a.min(b) < value) && (value < a.max(b))
}

#[test]
fn live_bar_easing_option_round_trips_clamps_and_resets() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    let options = |chart: &ChartEngine| -> serde_json::Value {
        serde_json::from_str(&chart.series_options_json(0).unwrap()).unwrap()
    };
    assert_eq!(options(&chart)["live_bar_easing_ms"], 0.0);
    assert!(chart.series_apply_options_json(0, r#"{"live_bar_easing_ms":250}"#));
    assert_eq!(options(&chart)["live_bar_easing_ms"], 250.0);
    // Clamped to the cap; negative, non-finite and non-numeric values are ignored.
    assert!(chart.series_apply_options_json(0, r#"{"live_bar_easing_ms":5000}"#));
    assert_eq!(
        options(&chart)["live_bar_easing_ms"],
        MAX_LIVE_BAR_EASING_MS
    );
    assert!(chart.series_apply_options_json(0, r#"{"live_bar_easing_ms":-1}"#));
    assert!(chart.series_apply_options_json(0, r#"{"live_bar_easing_ms":"fast"}"#));
    assert_eq!(
        options(&chart)["live_bar_easing_ms"],
        MAX_LIVE_BAR_EASING_MS
    );
    assert!(chart.series_apply_options_json(0, r#"{"live_bar_easing_ms":0}"#));
    assert_eq!(options(&chart)["live_bar_easing_ms"], 0.0);
    // Style class: a style reset turns it off.
    assert!(chart.series_apply_options_json(0, r#"{"live_bar_easing_ms":80}"#));
    chart.reset_style_to_defaults();
    assert_eq!(options(&chart)["live_bar_easing_ms"], 0.0);
}

#[test]
fn a_same_time_replace_glides_toward_the_new_values_on_pinned_clocks() {
    let mut chart = eased_candles(100.0);
    assert_eq!(displayed(&chart, 0), OLD_LAST);
    assert!(!chart.live_bar_easing_active());
    assert!(chart.update_series_bar(0, 10.0, NEW_LAST));
    // Unsettled, but no clock has advanced: the frame still shows the old values.
    assert!(chart.live_bar_easing_active());
    assert!(chart.animation_active());
    assert_eq!(displayed(&chart, 0), OLD_LAST);
    // The first clock after the transition only stamps the base.
    chart.set_animation_time(0.0);
    assert_eq!(displayed(&chart, 0), OLD_LAST);
    chart.set_animation_time(50.0);
    let first = displayed(&chart, 0);
    assert_eq!(first[0], NEW_LAST[0], "open never eases");
    for channel in 1..4 {
        assert!(
            between(first[channel], OLD_LAST[channel], NEW_LAST[channel]),
            "channel {channel}: {first:?}"
        );
    }
    chart.set_animation_time(100.0);
    let second = displayed(&chart, 0);
    for channel in 1..4 {
        assert!(
            between(second[channel], first[channel], NEW_LAST[channel]),
            "monotone toward the target: {first:?} -> {second:?}"
        );
    }
    // Exact exponential approach: x += (target - x) * (1 - exp(-dt / tau)).
    let expected = OLD_LAST[3] + (NEW_LAST[3] - OLD_LAST[3]) * (1.0 - (-50.0f64 / 100.0).exp());
    assert!(
        (first[3] - expected).abs() < 1e-12,
        "{first:?} vs {expected}"
    );
    // Queries, snapshots and the data API read the real values throughout.
    assert_eq!(chart.series_data(0).last().unwrap().close, NEW_LAST[3]);
    let snapshot = chart.value_snapshot(None);
    assert_eq!(snapshot[0].close, Some(NEW_LAST[3]));
    assert_eq!(snapshot[0].high, Some(NEW_LAST[1]));
    assert_eq!(
        chart.data.plot(0).value_at(9, PlotValueIndex::Close),
        NEW_LAST[3]
    );
}

#[test]
fn a_new_bar_snaps_the_live_bar() {
    let mut chart = eased_candles(100.0);
    assert!(chart.update_series_bar(0, 10.0, NEW_LAST));
    chart.set_animation_time(0.0);
    chart.set_animation_time(30.0);
    assert!(chart.live_bar_easing_active());
    let next = [114.0, 116.0, 113.0, 115.0];
    assert!(chart.update_series_bar(0, 11.0, next));
    assert_eq!(displayed(&chart, 0), next);
    assert!(!chart.live_bar_easing_active());
    // The previous bar reads its final real values (no override lingers on row 9).
    assert_eq!(
        chart.display_plot(0).value_at(9, PlotValueIndex::Close),
        NEW_LAST[3]
    );
}

#[test]
fn reduced_motion_snaps_instead_of_gliding() {
    let mut chart = eased_candles(100.0);
    let mut options = chart.interaction_options();
    options.reduced_motion = true;
    chart.set_interaction_options(options);
    assert!(chart.update_series_bar(0, 10.0, NEW_LAST));
    assert_eq!(displayed(&chart, 0), NEW_LAST);
    assert!(!chart.live_bar_easing_active());
    assert!(!advance_changes_display(&mut chart));
}

/// Whether an advance at a fresh clock moved any series' display.
fn advance_changes_display(chart: &mut ChartEngine) -> bool {
    let shown = |chart: &ChartEngine| -> Vec<[f64; 4]> {
        chart
            .series
            .iter()
            .map(|s| displayed(chart, s.id))
            .collect()
    };
    let before = shown(chart);
    let advanced = chart.advance_live_bar_easing(chart.animation_time + 16.0);
    advanced || before != shown(chart)
}

#[test]
fn an_idle_gap_resumes_with_a_glide_instead_of_a_jump() {
    let mut chart = eased_candles(100.0);
    chart.set_animation_time(0.0);
    assert!(chart.update_series_bar(0, 10.0, NEW_LAST));
    // No frames for five seconds, then two frames at 60 Hz.
    chart.set_animation_time(5016.0);
    assert_eq!(displayed(&chart, 0), OLD_LAST);
    chart.set_animation_time(5032.0);
    let shown = displayed(&chart, 0);
    for channel in 1..4 {
        assert!(between(
            shown[channel],
            OLD_LAST[channel],
            NEW_LAST[channel]
        ));
    }
    let expected = OLD_LAST[3] + (NEW_LAST[3] - OLD_LAST[3]) * (1.0 - (-16.0f64 / 100.0).exp());
    assert!((shown[3] - expected).abs() < 1e-12);
    // One frame integrates at most 100 ms: a 200 ms stall inside the settle window takes one
    // capped step instead of jumping.
    chart.set_animation_time(5232.0);
    let capped = displayed(&chart, 0);
    let expected = shown[3] + (NEW_LAST[3] - shown[3]) * (1.0 - (-100.0f64 / 100.0).exp());
    assert!((capped[3] - expected).abs() < 1e-12, "dt capped at 100 ms");
    // Past six time constants since the last change the glide snaps, so a long stall after the
    // last tick never leaves a stale bar on screen.
    chart.set_animation_time(9000.0);
    assert_eq!(displayed(&chart, 0), NEW_LAST);
    assert!(!chart.live_bar_easing_active());
}

#[test]
fn a_sixty_hertz_feed_never_freezes_the_glide() {
    let mut chart = eased_candles(100.0);
    chart.set_animation_time(0.0);
    let mut clock = 0.0;
    let mut previous = OLD_LAST[3];
    for tick in 1..=60 {
        let close = 110.0 + tick as f64 * 0.1;
        assert!(chart.update_series_bar(0, 10.0, [109.0, close + 1.0, 107.0, close]));
        clock += 1000.0 / 60.0;
        chart.set_animation_time(clock);
        let shown = displayed(&chart, 0)[3];
        if tick >= 2 {
            assert!(
                shown > previous && shown < close,
                "frame {tick}: {previous} -> {shown} (target {close})"
            );
        }
        previous = shown;
    }
    assert!(chart.live_bar_easing_active());
}

#[test]
fn the_glide_settles_by_six_tau_and_by_epsilon() {
    // By time: six time constants after the first stamped clock.
    let mut chart = eased_candles(100.0);
    assert!(chart.update_series_bar(0, 10.0, NEW_LAST));
    chart.set_animation_time(1000.0);
    let mut clock = 1000.0;
    while clock < 1500.0 {
        clock += 100.0;
        chart.set_animation_time(clock);
        assert!(chart.live_bar_easing_active(), "clock {clock}");
        assert_ne!(displayed(&chart, 0), NEW_LAST);
    }
    chart.set_animation_time(1600.0);
    assert_eq!(displayed(&chart, 0), NEW_LAST, "snaps exactly at 6 tau");
    assert!(!chart.live_bar_easing_active());
    assert!(!chart.animation_active());
    // By epsilon: a tiny change settles on the first advance.
    let mut chart = eased_candles(100.0);
    let tiny = [109.0, 111.0, 107.0, 110.0 + 1e-9];
    assert!(chart.update_series_bar(0, 10.0, tiny));
    chart.set_animation_time(0.0);
    assert!(chart.live_bar_easing_active());
    chart.set_animation_time(16.0);
    assert_eq!(displayed(&chart, 0), tiny);
    assert!(!chart.live_bar_easing_active());
}

#[test]
fn a_rust_host_advancing_only_the_easing_clock_leaves_the_pulse_at_phase_zero() {
    let mut chart = eased_candles(100.0);
    chart.convert_series_kind(0, SeriesKind::Line);
    assert!(chart.last_price_pulse_active());
    assert!(chart.update_series_bar(0, 10.0, NEW_LAST));
    assert!(
        !chart.advance_live_bar_easing(10.0),
        "the first clock only stamps"
    );
    assert!(chart.advance_live_bar_easing(26.0));
    assert_eq!(
        chart.animation_time, 0.0,
        "advancing only the easing clock leaves the pulse clock alone"
    );
    assert!(chart.animation_frame_requested());
    // A settled chart requests no frame; an advance with nothing unsettled changes nothing.
    chart.advance_live_bar_easing(5000.0);
    assert!(!chart.live_bar_easing_active());
    assert!(!chart.animation_frame_requested());
    assert!(!chart.advance_live_bar_easing(5016.0));
}

#[test]
fn a_stamp_only_tick_builds_nothing_and_an_advance_skips_autoscale() {
    let mut chart = eased_candles(100.0);
    chart.build_frame();
    // Nothing unsettled and no pulse: the clock is not even written.
    chart.set_animation_time(10.0);
    assert_eq!(chart.animation_time, 0.0);
    chart.build_frame();
    assert_eq!(
        chart.frame_build_stats(),
        crate::frame::FrameBuildStats::default()
    );

    assert!(chart.update_series_bar(0, 10.0, NEW_LAST));
    chart.build_frame();
    let tick = chart.frame_build_stats();
    assert_eq!(tick.autoscale_runs, 1, "the tick re-runs autoscale once");
    assert_eq!(tick.series_rebuilds, 1);
    // The stamp frame: no display change, nothing rebuilt.
    chart.set_animation_time(100.0);
    chart.build_frame();
    assert_eq!(
        chart.frame_build_stats(),
        crate::frame::FrameBuildStats::default()
    );
    // Advance frames rebuild the eased series layer and chrome, never autoscale or the grid.
    for clock in [116.0, 132.0, 148.0, 164.0] {
        chart.set_animation_time(clock);
        chart.build_frame();
        let stats = chart.frame_build_stats();
        assert_eq!(stats.autoscale_runs, 0, "clock {clock}");
        assert_eq!(stats.grid_rebuilds, 0);
        assert_eq!(stats.layout_rebuilds, 0);
        assert_eq!(stats.drawing_rebuilds, 0);
        assert_eq!(stats.series_rebuilds, 1);
        assert_eq!(stats.overlay_rebuilds, 1);
    }
}

#[test]
fn a_replay_cutoff_keys_the_glide_on_the_drawn_bar() {
    let mut chart = eased_candles(100.0);
    let clock = |seconds: i64| Some(seconds * 1_000_000);
    chart.set_replay_clock_micros(clock(5)).unwrap();
    assert_eq!(chart.display_plot(0).size(), 5);
    // A tick on the canonical tail past the clock never unsettles the drawn bar (row 4, time 5).
    assert!(chart.update_series_bar(0, 10.0, NEW_LAST));
    assert!(!chart.live_bar_easing_active());
    // A same-time replace of the drawn bar glides.
    let drawn = [104.0, 108.0, 100.0, 107.0];
    assert!(chart.update_series_bar(0, 5.0, drawn));
    assert!(chart.live_bar_easing_active());
    chart.set_animation_time(0.0);
    chart.set_animation_time(40.0);
    let shown = displayed(&chart, 0);
    assert!(between(shown[3], 105.0, drawn[3]));
    assert_eq!(
        chart.display_plot(0).value_at(4, PlotValueIndex::Close),
        shown[3]
    );
    // Advancing the clock changes the drawn row: snap, nothing left gliding.
    chart.set_replay_clock_micros(clock(6)).unwrap();
    assert_eq!(chart.display_plot(0).size(), 6);
    assert!(!chart.live_bar_easing_active());
    assert_eq!(
        chart.display_plot(0).value_at(4, PlotValueIndex::Close),
        drawn[3]
    );
    assert_eq!(displayed(&chart, 0), [105.0, 107.0, 103.0, 106.0]);
}

#[test]
fn an_as_of_overlay_glides_on_a_same_row_replace_and_snaps_on_a_new_row() {
    const DAY: f64 = 86_400.0;
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    chart.time_scale.set_width(800.0);
    let hk_times: Vec<f64> = [1.0, 2.0, 3.0, 5.0, 6.0].map(|d| d * DAY).to_vec();
    let hk = [100.0, 101.0, 102.0, 103.0, 104.0];
    chart
        .set_series_data(0, &hk_times, &hk, &hk, &hk, &hk)
        .unwrap();
    let us = chart.add_series(SeriesKind::Line);
    let us_times: Vec<f64> = [1.0, 2.0, 4.0].map(|d| d * DAY).to_vec();
    let us_close = [4000.0, 4040.0, 4100.0];
    chart
        .set_series_data(us, &us_times, &us_close, &us_close, &us_close, &us_close)
        .unwrap();
    chart
        .set_series_time_alignment(
            us,
            TimeAlignment::AsOf {
                max_staleness: None,
            },
        )
        .unwrap();
    assert!(chart.series_apply_options_json(us, r#"{"live_bar_easing_ms":100}"#));
    chart.fit_content();
    // US d4 backs HK d5 and d6: both plot rows show canonical row 2.
    let plot = chart.display_plot(us);
    assert_eq!(plot.size(), 5);
    assert_eq!(plot.source_row(3), 2);
    assert_eq!(plot.source_row(4), 2);
    // A same-row replace glides on every plot row showing that canonical row.
    assert!(chart.update_series_bar(us, 4.0 * DAY, [4200.0; 4]));
    chart.set_animation_time(0.0);
    chart.set_animation_time(50.0);
    let plot = chart.display_plot(us);
    let shown = plot.value_at(4, PlotValueIndex::Close);
    assert!(between(shown, 4100.0, 4200.0));
    assert_eq!(plot.value_at(3, PlotValueIndex::Close), shown);
    assert_eq!(plot.value_at(2, PlotValueIndex::Close), 4040.0);
    assert_eq!(
        chart.data.plot(us).value_at(4, PlotValueIndex::Close),
        4200.0,
        "the canonical view is untouched"
    );
    // A new US row (d6) adds no time point but changes the drawn canonical row: snap.
    assert!(chart.update_series_bar(us, 6.0 * DAY, [4300.0; 4]));
    assert!(!chart.live_bar_easing_active());
    let plot = chart.display_plot(us);
    assert_eq!(plot.value_at(4, PlotValueIndex::Close), 4300.0);
    assert_eq!(plot.value_at(3, PlotValueIndex::Close), 4200.0);
}

#[test]
fn a_same_time_reinstall_and_a_pop_snap_the_live_bar() {
    let mut chart = eased_candles(100.0);
    assert!(chart.update_series_bar(0, 10.0, NEW_LAST));
    chart.set_animation_time(0.0);
    chart.set_animation_time(30.0);
    assert!(chart.live_bar_easing_active());
    // The same ten times again with the final values: no glide from stale values.
    let times: Vec<f64> = (1..=10).map(|i| i as f64).collect();
    let open: Vec<f64> = (0..10).map(|i| 100.0 + i as f64).collect();
    let mut high: Vec<f64> = open.iter().map(|v| v + 2.0).collect();
    let mut low: Vec<f64> = open.iter().map(|v| v - 2.0).collect();
    let mut close: Vec<f64> = open.iter().map(|v| v + 1.0).collect();
    high[9] = NEW_LAST[1];
    low[9] = NEW_LAST[2];
    close[9] = NEW_LAST[3];
    chart
        .set_series_data(0, &times, &open, &high, &low, &close)
        .unwrap();
    assert!(!chart.live_bar_easing_active());
    assert_eq!(displayed(&chart, 0), NEW_LAST);
    // A tick after the reinstall glides from the reinstalled values, not from older ones.
    assert!(chart.update_series_bar(0, 10.0, [109.0, 120.0, 105.0, 118.0]));
    chart.set_animation_time(100.0);
    chart.set_animation_time(150.0);
    let shown = displayed(&chart, 0);
    assert!(between(shown[3], NEW_LAST[3], 118.0));
    // Popping the drawn bar snaps onto the new last row.
    assert_eq!(chart.series_pop(0, 1), Some(9));
    assert!(!chart.live_bar_easing_active());
    assert_eq!(displayed(&chart, 0), [108.0, 110.0, 106.0, 109.0]);
}

#[test]
fn heikin_ashi_easing_keeps_autoscale_and_the_base_value_canonical() {
    let mut chart = eased_candles(100.0);
    chart.series[0].heikin_ashi = true;
    chart.build_frame();
    assert!(chart.update_series_bar(0, 10.0, NEW_LAST));
    chart.build_frame();
    let canonical_last = chart.heikin_ashi_row(0, 9).unwrap();
    let base = chart.series_base_value(0, 0).unwrap();
    chart.set_animation_time(0.0);
    chart.build_frame();
    for clock in [16.0, 32.0, 48.0] {
        chart.set_animation_time(clock);
        chart.build_frame();
        let stats = chart.frame_build_stats();
        assert_eq!(stats.autoscale_runs, 0, "clock {clock}");
        assert_eq!(stats.series_rebuilds, 1);
        // The drawn Heikin Ashi row follows the eased raw values; the cache stays canonical.
        let plot = chart.display_plot(0);
        let drawn = chart.display_heikin_ashi_row(0, plot, 9).unwrap();
        assert_ne!(drawn, canonical_last);
        assert_eq!(chart.heikin_ashi_row(0, 9), Some(canonical_last));
        let raw = displayed(&chart, 0);
        assert_eq!(drawn[3], (raw[0] + raw[1] + raw[2] + raw[3]) / 4.0);
        let previous = chart.heikin_ashi_row(0, 8).unwrap();
        assert_eq!(drawn[0], (previous[0] + previous[3]) / 2.0);
        assert_eq!(chart.series_base_value(0, 0), Some(base));
    }
    chart.set_animation_time(5000.0);
    assert_eq!(
        chart.display_heikin_ashi_row(0, chart.display_plot(0), 9),
        Some(canonical_last),
        "settled: the display row equals the cached row"
    );
}

#[test]
fn heikin_ashi_easing_carries_the_previous_row_across_whitespace() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    chart.time_scale.set_width(800.0);
    let nan = f64::NAN;
    chart
        .set_series_data(
            0,
            &[1.0, 2.0, 3.0],
            &[10.0, nan, 14.0],
            &[14.0, nan, 18.0],
            &[8.0, nan, 12.0],
            &[12.0, nan, 16.0],
        )
        .unwrap();
    chart.series[0].heikin_ashi = true;
    assert!(chart.series_apply_options_json(0, r#"{"live_bar_easing_ms":100}"#));
    assert!(chart.update_series_bar(0, 3.0, [14.0, 20.0, 12.0, 18.0]));
    chart.set_animation_time(0.0);
    chart.set_animation_time(50.0);
    let plot = chart.display_plot(0);
    let drawn = chart.display_heikin_ashi_row(0, plot, 2).unwrap();
    let first = chart.heikin_ashi_row(0, 0).unwrap();
    assert_eq!(drawn[0], (first[0] + first[3]) / 2.0, "row 1 is whitespace");
    let raw = displayed(&chart, 0);
    assert_eq!(drawn[3], (raw[0] + raw[1] + raw[2] + raw[3]) / 4.0);
    assert!(
        chart
            .heikin_ashi_row(0, 1)
            .unwrap()
            .iter()
            .all(|v| v.is_nan())
    );
}

#[test]
fn heikin_ashi_easing_skips_a_partially_nan_row_like_the_cached_rebuild() {
    // A row that is not whitespace but not finite either projects to NaN and is skipped by the
    // cached rebuild; the display path must step over it the same way, or the drawn open jumps
    // when the glide settles. Only the pre-validated installer can land such a row.
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    chart.time_scale.set_width(800.0);
    let nan = f64::NAN;
    assert!(chart.install_series_data(
        0,
        vec![1, 2, 3],
        vec![10.0, 10.0, 14.0],
        vec![14.0, nan, 18.0],
        vec![8.0, 8.0, 12.0],
        vec![12.0, 12.0, 16.0],
    ));
    chart.series[0].heikin_ashi = true;
    assert!(chart.series_apply_options_json(0, r#"{"live_bar_easing_ms":100}"#));
    assert!(chart.update_series_bar(0, 3.0, [14.0, 20.0, 12.0, 18.0]));
    chart.set_animation_time(0.0);
    chart.set_animation_time(50.0);
    assert!(chart.live_bar_easing_active());
    let first = chart.heikin_ashi_row(0, 0).unwrap();
    let drawn = chart
        .display_heikin_ashi_row(0, chart.display_plot(0), 2)
        .unwrap();
    assert_eq!(
        drawn[0],
        (first[0] + first[3]) / 2.0,
        "the partially NaN row 1 is skipped, as the rebuild skips it"
    );
    chart.set_animation_time(5000.0);
    assert!(!chart.live_bar_easing_active());
    let settled = chart.heikin_ashi_row(0, 2).unwrap();
    assert_eq!(
        chart.display_heikin_ashi_row(0, chart.display_plot(0), 2),
        Some(settled)
    );
    assert_eq!(settled[0], drawn[0], "the open does not jump on settle");
}

#[test]
fn a_backwards_host_clock_restarts_the_glide_epoch_instead_of_pinning_it() {
    // An adapter whose clock restarts below the stored base (a new adapter instance, a switched
    // clock source) must not leave the glide unsettled forever with frames requested.
    let mut chart = eased_candles(100.0);
    assert!(chart.update_series_bar(0, 10.0, NEW_LAST));
    assert!(!chart.advance_live_bar_easing(1000.0), "stamp only");
    assert!(chart.advance_live_bar_easing(1016.0));
    let before = displayed(&chart, 0);
    assert!(
        !chart.advance_live_bar_easing(5.0),
        "a backwards clock never advances"
    );
    assert_eq!(displayed(&chart, 0), before);
    assert!(chart.live_bar_easing_active());
    assert!(
        chart.advance_live_bar_easing(21.0),
        "the new epoch glides again"
    );
    let after = displayed(&chart, 0);
    for channel in 1..4 {
        assert!(
            between(after[channel], before[channel], NEW_LAST[channel]),
            "channel {channel}: {before:?} -> {after:?}"
        );
    }
    assert!(chart.advance_live_bar_easing(7000.0));
    assert_eq!(
        displayed(&chart, 0),
        NEW_LAST,
        "settles six tau into the new epoch"
    );
    assert!(!chart.live_bar_easing_active());
    assert!(!chart.animation_frame_requested());
}

#[test]
fn a_render_cutoff_hiding_the_last_bar_never_eases_it() {
    let mut chart = eased_candles(100.0);
    chart.set_series_render_before_time(0, Some(10));
    assert!(chart.update_series_bar(0, 10.0, NEW_LAST));
    assert!(
        !chart.live_bar_easing_active(),
        "an undrawn bar never glides"
    );
    assert!(!chart.animation_frame_requested());
    assert!(!chart.animation_active());
    assert_eq!(displayed(&chart, 0), NEW_LAST);
    assert!(!chart.advance_live_bar_easing(16.0));
    // Revealing the bar again shows its real values; the next same-time tick glides from them.
    chart.set_series_render_before_time(0, None);
    assert!(!chart.live_bar_easing_active());
    assert_eq!(displayed(&chart, 0), NEW_LAST);
    let next = [109.0, 120.0, 105.0, 118.0];
    assert!(chart.update_series_bar(0, 10.0, next));
    chart.set_animation_time(100.0);
    chart.set_animation_time(150.0);
    assert!(between(displayed(&chart, 0)[3], NEW_LAST[3], next[3]));
}

#[test]
fn a_reinstall_trimmed_to_the_retention_cap_keeps_the_next_tick_gliding() {
    let mut chart = eased_candles(100.0);
    assert!(chart.set_series_max_points(0, Some(8)));
    // Ten rows land, the cap trims the front: the easing state must key to the trimmed row set.
    let times: Vec<f64> = (1..=10).map(|i| i as f64).collect();
    let open: Vec<f64> = (0..10).map(|i| 100.0 + i as f64).collect();
    let high: Vec<f64> = open.iter().map(|v| v + 2.0).collect();
    let low: Vec<f64> = open.iter().map(|v| v - 2.0).collect();
    let close: Vec<f64> = open.iter().map(|v| v + 1.0).collect();
    chart
        .set_series_data(0, &times, &open, &high, &low, &close)
        .unwrap();
    assert!(
        chart.series_data(0).len() < 10,
        "the cap trimmed the install"
    );
    // The tick lands before any frame re-reads the drawn row (a `setData` + `update` in one task).
    assert!(chart.update_series_bar(0, 10.0, NEW_LAST));
    assert!(
        chart.live_bar_easing_active(),
        "a same-time tick after the trimmed install glides"
    );
    assert_eq!(displayed(&chart, 0), OLD_LAST);
    chart.set_animation_time(0.0);
    chart.set_animation_time(50.0);
    let shown = displayed(&chart, 0);
    for channel in 1..4 {
        assert!(
            between(shown[channel], OLD_LAST[channel], NEW_LAST[channel]),
            "{shown:?}"
        );
    }
}

#[test]
fn a_historical_correction_during_a_glide_leaves_the_drawn_bar_gliding() {
    let mut chart = eased_candles(100.0);
    assert!(chart.update_series_bar(0, 10.0, NEW_LAST));
    chart.set_animation_time(0.0);
    chart.set_animation_time(30.0);
    let mid = displayed(&chart, 0);
    assert!(between(mid[3], OLD_LAST[3], NEW_LAST[3]));
    // Correcting a bar far back changes nothing about the drawn row: no jump.
    assert!(chart.update_series_bar(0, 2.0, [101.0, 104.0, 98.0, 102.5]));
    assert_eq!(displayed(&chart, 0), mid);
    assert!(chart.live_bar_easing_active());
    chart.set_animation_time(60.0);
    let later = displayed(&chart, 0);
    assert!(
        between(later[3], mid[3], NEW_LAST[3]),
        "{mid:?} -> {later:?}"
    );
    assert_eq!(chart.series_data(0)[1].close, 102.5);
    // Settled, a correction keeps the display on the real drawn row.
    chart.set_animation_time(5000.0);
    assert!(!chart.live_bar_easing_active());
    assert!(chart.update_series_bar(0, 3.0, [102.0, 105.0, 99.0, 103.5]));
    assert_eq!(displayed(&chart, 0), NEW_LAST);
    assert!(!chart.live_bar_easing_active());
}

#[test]
fn a_non_finite_time_constant_written_by_a_rust_host_is_off() {
    let mut chart = eased_candles(100.0);
    chart.series[0].live_bar_easing_ms = f64::NAN;
    assert!(chart.update_series_bar(0, 10.0, NEW_LAST));
    assert!(!chart.live_bar_easing_active());
    assert!(!chart.animation_frame_requested());
    assert!(!chart.advance_live_bar_easing(16.0));
    assert_eq!(displayed(&chart, 0), NEW_LAST);
    chart.series[0].live_bar_easing_ms = f64::INFINITY;
    assert!(chart.update_series_bar(0, 10.0, OLD_LAST));
    assert!(!chart.live_bar_easing_active());
    assert_eq!(displayed(&chart, 0), OLD_LAST);
    // Above the bound a Rust host glides with the bound's time constant, like the JSON path. The
    // first tick after enabling keys the state to the drawn row (writes while off never tracked
    // it), so the glide starts with the second same-time tick.
    chart.series[0].live_bar_easing_ms = 5000.0;
    assert!(chart.update_series_bar(0, 10.0, NEW_LAST));
    assert!(!chart.live_bar_easing_active());
    assert_eq!(displayed(&chart, 0), NEW_LAST);
    let next = [109.0, 120.0, 105.0, 118.0];
    assert!(chart.update_series_bar(0, 10.0, next));
    assert!(chart.live_bar_easing_active());
    chart.set_animation_time(0.0);
    chart.set_animation_time(100.0);
    let expected = NEW_LAST[3] + (next[3] - NEW_LAST[3]) * (1.0 - (-100.0f64 / 1000.0).exp());
    assert!((displayed(&chart, 0)[3] - expected).abs() < 1e-12);
}

/// The pulse is decorative motion: under reduced motion the host's animation loop stops and the
/// next frame drops the pulse, whichever host fed the preference.
#[test]
fn reduced_motion_removes_the_pulse_and_stops_its_clock() {
    use aeris_charts_render::draw_list::Prim;
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    chart.convert_series_kind(0, SeriesKind::Line);
    install_bars(&mut chart, 20);
    chart.fit_content();
    let circles = |chart: &mut ChartEngine| {
        chart.build_frame().panes[0]
            .main
            .iter()
            .filter(|prim| matches!(prim, Prim::Circle { .. }))
            .count()
    };
    chart.animation_time = 100.0;
    let pulse = circles(&mut chart);
    assert!(pulse > 0, "a line pulses its last price");
    assert!(chart.last_price_pulse_active());

    let mut options = chart.interaction_options();
    options.reduced_motion = true;
    chart.set_interaction_options(options);
    assert!(
        !chart.last_price_pulse_active(),
        "the host's animation loop stops"
    );
    assert_eq!(circles(&mut chart), 0, "the next frame drops the pulse");
    assert_eq!(chart.frame_build_stats().series_rebuilds, 0);

    options.reduced_motion = false;
    chart.set_interaction_options(options);
    assert!(chart.last_price_pulse_active());
    assert_eq!(circles(&mut chart), pulse);
}

/// `countdown_shown` holds exactly while a countdown row shows at the pinned clock, so a host
/// re-pins the clock each second only while the row can change.
#[test]
fn countdown_shown_follows_the_pinned_clock_and_the_forming_bar() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    chart.time_scale.set_width(800.0);
    let times: Vec<f64> = (0..20).map(|i| 1_000.0 + i as f64 * 60.0).collect();
    let values = vec![100.0; times.len()];
    chart
        .set_series_data(0, &times, &values, &values, &values, &values)
        .unwrap();
    let forming = times[times.len() - 1];
    assert!(chart.series[0].countdown_visible);
    assert!(!chart.countdown_shown(), "no clock pinned, no countdown");

    chart.set_now_seconds(forming + 15.0);
    assert!(chart.countdown_shown());
    chart.set_now_seconds(forming + 60.0);
    assert!(!chart.countdown_shown(), "the forming bar closed");
    chart.set_now_seconds(forming - 5.0);
    assert!(
        !chart.countdown_shown(),
        "the clock is before the bar opens"
    );

    chart.set_now_seconds(forming + 15.0);
    chart.set_bar_countdown_active(false);
    assert!(!chart.countdown_shown(), "the session is closed");
    chart.set_bar_countdown_active(true);
    chart.series[0].countdown_visible = false;
    assert!(!chart.countdown_shown(), "the series hides its countdown");
    chart.series[0].countdown_visible = true;
    assert!(chart.countdown_shown());
    chart.set_series_visible(0, false);
    assert!(
        !chart.countdown_shown(),
        "a hidden series shows no countdown"
    );
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
    assert!(
        axis.labels
            .iter()
            .any(|l| l.background.is_some() && l.text == "7.00")
    );
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
    assert!(
        frame.panes[0]
            .main
            .iter()
            .any(|p| matches!(p, Prim::Rect { color, .. } if *color == up))
    );
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
    assert!(
        chart
            .price_scales(0)
            .unwrap()
            .iter()
            .any(|info| { info.id == "host-owned" && info.series_ids.is_empty() })
    );
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
    assert!(
        chart
            .price_scale_options_json(9, PriceScaleTarget::Right)
            .is_none()
    );
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
    assert!(
        !frame.panes[0]
            .under
            .iter()
            .any(|p| matches!(p, Prim::Background { .. }))
    );

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
    assert!(!frame.panes.iter().any(|pane| {
        pane.under
            .iter()
            .any(|p| matches!(p, Prim::Background { .. }))
    }));
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
    assert_eq!(light.crosshair.vert_line.color, "#4a4a4a");
    assert_eq!(light.crosshair.horz_line.color, "#4a4a4a");
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
    assert_eq!(dark.crosshair.vert_line.color, "#4a4a4a");
    assert_eq!(dark.crosshair.horz_line.color, "#4a4a4a");
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
        chart.series_bar_color(&chart.series[0], chart.data.plot(0), 0, None),
        Color::parse_css(aeris_charts_core::style::LIGHT_MARKET_UP_CSS).unwrap()
    );
    assert_eq!(
        chart.series_bar_color(&chart.series[0], chart.data.plot(0), 1, None),
        Color::parse_css(aeris_charts_core::style::LIGHT_MARKET_DOWN_CSS).unwrap()
    );

    chart.set_theme(ChartTheme::Dark);
    assert_eq!(
        chart.series_bar_color(&chart.series[0], chart.data.plot(0), 0, None),
        Color::parse_css(aeris_charts_core::style::DARK_MARKET_UP_CSS).unwrap()
    );
    assert_eq!(
        chart.series_bar_color(&chart.series[0], chart.data.plot(0), 1, None),
        Color::parse_css(aeris_charts_core::style::DARK_MARKET_DOWN_CSS).unwrap()
    );

    chart.series[0].up_color = Some("#010203".into());
    chart.series[0].down_color = Some("#040506".into());
    chart.set_theme(ChartTheme::Light);
    assert_eq!(
        chart.series_bar_color(&chart.series[0], chart.data.plot(0), 0, None),
        Color::rgb(1, 2, 3)
    );
    assert_eq!(
        chart.series_bar_color(&chart.series[0], chart.data.plot(0), 1, None),
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
    chart.build_axis_primitives_into(&axis, &mut prims);
    let resting = prims
        .iter()
        .filter_map(|p| match p {
            Prim::Rect { rect, .. } if rect.y == separator_y && rect.h == 2 => Some(*rect),
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
    chart.build_axis_primitives_into(&axis, &mut prims);
    let hover = prims
        .iter()
        .find_map(|p| match p {
            Prim::Rect { rect, .. } if rect.h == 10 => Some(*rect),
            _ => None,
        })
        .expect("the hover band");
    assert_eq!(hover.x, 0);
    assert_eq!(hover.w, bitmap_w, "the hover band matches the resting line");
    assert_eq!(hover.y, separator_y - 4);
}

#[test]
fn axis_borders_are_one_css_px_and_pane_separators_two() {
    use aeris_charts_render::draw_list::Prim;

    for dpr in [1.0_f64, 1.25, 1.5, 2.0, 3.0] {
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
        chart.build_axis_primitives_into(&axis, &mut prims);

        let expected = aeris_charts_core::style::border_width_device_px(dpr) as i32;
        let right_x = ((chart.pane_left + chart.pane_w) * dpr).round() as i32;
        let pane_bottom = (chart.pane_h * dpr).round() as i32;
        let bitmap_w = (chart.css_width * dpr).round().max(1.0) as i32;
        let separator_y = (axis.separators[0] * dpr).round() as i32;
        let separator_h = (crate::PANE_SEPARATOR * dpr).round().max(1.0) as i32;
        assert_eq!(expected, dpr.floor() as i32, "1 CSS px border at dpr {dpr}");
        assert!(
            (separator_h - (2.0 * dpr) as i32).abs() <= 1,
            "2 CSS px separator at dpr {dpr}"
        );

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
                        && rect.h == separator_h
            )),
            "pane separator must use {separator_h} device px at dpr {dpr}"
        );
    }
}

/// The crosshair time label's vertical placement belongs to the shared axis builder and is keyed
/// to the host's cap-center metric for the painted size, family and weight, never to the label's
/// own glyphs. The text therefore sits at the same offset in the time strip for every month name
/// (a label without descenders is not re-centred by its own ink), font, DPR and backend. Pixel
/// probes of the painted label must still fix the label text rather than depend on the calendar.
#[test]
fn crosshair_time_text_is_placed_by_the_cap_center_metric_not_its_own_ink() {
    use aeris_charts_render::draw_list::Prim;

    const CAP_CENTER: f64 = 3.5;
    let measure = |text: &str, _bold: bool| text.len() as f64 * 6.0;

    for dpr in [1.0_f64, 1.25, 2.0] {
        let mut chart = ChartEngine::new(800.0, 500.0, dpr);
        let times: Vec<f64> = (0..10)
            .map(|i| 1_700_000_000.0 + i as f64 * 3_600.0)
            .collect();
        let closes: Vec<f64> = (0..10).map(|i| 100.0 + i as f64).collect();
        chart
            .set_series_data(0, &times, &closes, &closes, &closes, &closes)
            .unwrap();
        chart.time_scale.set_width(800.0);
        chart.fit_content();
        chart.set_time_visible(true);
        chart.recompute_layout_with_measure(true, measure, measure);
        chart.build_frame();
        let x = chart.time_scale.index_to_coordinate(5);
        chart.set_crosshair_at(x, 100.0);

        let axis = chart.build_axis_frame(80.0, measure, measure);
        let label = axis
            .labels
            .iter()
            .find(|label| label.midpoint == AxisTextMidpoint::StableTime)
            .expect("the crosshair time label");
        // Default 12 px layout font: 11 CSS px axis text centered below the 1 px border slot,
        // 3 px tick allowance and 3 px padding.
        assert_eq!(label.y, chart.pane_h + 1.0 + 3.0 + 3.0 + 11.0 / 2.0);

        // The metric answers per (size, family, weight); a different run of text can only get
        // the same answer because the text is not an input.
        let asked = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let seen = std::rc::Rc::clone(&asked);
        chart.set_text_cap_center(Some(Box::new(move |size, family, weight, italic| {
            seen.borrow_mut()
                .push((size, family.to_owned(), weight, italic));
            CAP_CENTER
        })));
        let mut prims = Vec::new();
        chart.build_axis_primitives_into(&axis, &mut prims);
        let y = prims
            .iter()
            .find_map(|prim| match prim {
                Prim::Text { text, y, .. } if *text == label.text => Some(*y),
                _ => None,
            })
            .expect("the crosshair time text primitive");
        let expected = ((label.y + CAP_CENTER) * dpr) as f32;
        assert_eq!(
            y, expected,
            "text y follows the cap-center metric at dpr {dpr}"
        );
        let size = chart.options.get().layout.font_size * label.font_scale;
        assert!(
            asked
                .borrow()
                .iter()
                .any(|(asked_size, _, weight, italic)| {
                    *asked_size == size && *weight == if label.bold { 700 } else { 400 } && !*italic
                }),
            "the metric is sampled at the painted size and weight at dpr {dpr}"
        );
    }
}

#[test]
fn pane_separators_have_identical_device_thickness_at_fractional_dpr() {
    use aeris_charts_render::draw_list::Prim;

    let mut chart = chart_with_indicator_pane();
    let extra = chart.add_series(SeriesKind::Line);
    chart.set_series_pane(extra, 2, 1.0);
    chart.dpr = 1.25;
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
    assert_eq!(axis.separators.len(), 2);
    let mut prims = Vec::new();
    chart.build_axis_primitives_into(&axis, &mut prims);
    let expected_height = (crate::PANE_SEPARATOR * chart.dpr).round() as i32;
    let heights = axis
        .separators
        .iter()
        .map(|separator| {
            let y = (separator * chart.dpr).round() as i32;
            prims
                .iter()
                .find_map(|primitive| match primitive {
                    Prim::Rect { rect, .. } if rect.x == 0 && rect.y == y => Some(rect.h),
                    _ => None,
                })
                .expect("full-width separator primitive")
        })
        .collect::<Vec<_>>();
    assert_eq!(heights, vec![expected_height; 2]);
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
        IndicatorKind::Aroon { period: 5 },
        IndicatorKind::AwesomeOscillator,
        IndicatorKind::Dpo { period: 5 },
        IndicatorKind::ChandeMomentum { period: 5 },
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
        IndicatorKind::BollingerMetrics {
            period: 5,
            deviation: 2.0,
        },
        IndicatorKind::Envelopes {
            period: 5,
            percent: 10.0,
            exponential: false,
        },
        IndicatorKind::Envelopes {
            period: 5,
            percent: 10.0,
            exponential: true,
        },
        IndicatorKind::Alma {
            period: 5,
            offset: 0.85,
            sigma: 6.0,
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
        IndicatorKind::AccumulationDistribution,
        IndicatorKind::PriceVolumeTrend,
        IndicatorKind::ChaikinOscillator { fast: 3, slow: 7 },
        IndicatorKind::RelativeVolume { period: 5 },
        IndicatorKind::ElderForce { period: 5 },
        IndicatorKind::EaseOfMovement {
            period: 5,
            divisor: 100.0,
        },
        IndicatorKind::HistoricalVolatility {
            period: 5,
            annualization: 252.0,
        },
        IndicatorKind::Trix {
            period: 3,
            signal: 4,
        },
        IndicatorKind::CoppockCurve {
            long: 7,
            short: 5,
            smoothing: 3,
        },
        IndicatorKind::FisherTransform { period: 5 },
        IndicatorKind::UltimateOscillator {
            short: 3,
            medium: 5,
            long: 7,
        },
        IndicatorKind::Kst {
            roc: [2, 3, 4, 5],
            smoothing: [2, 2, 2, 3],
            signal: 3,
        },
        IndicatorKind::Tsi {
            long: 5,
            short: 3,
            signal: 3,
        },
        IndicatorKind::MassIndex {
            ema_period: 3,
            sum_period: 5,
        },
        IndicatorKind::Klinger {
            fast: 3,
            slow: 7,
            signal: 4,
        },
        IndicatorKind::Kama {
            period: 5,
            fast: 2,
            slow: 10,
        },
        IndicatorKind::McGinley { period: 5 },
        IndicatorKind::LinearRegression {
            period: 5,
            deviation: 2.0,
        },
        IndicatorKind::Choppiness { period: 5 },
        IndicatorKind::AtrBands {
            period: 5,
            multiplier: 2.0,
        },
        IndicatorKind::Vortex { period: 5 },
        IndicatorKind::VolumeOscillator {
            fast: 3,
            slow: 7,
            signal: 4,
        },
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
    .into_iter()
    .chain(study_indicator_kinds())
    .collect()
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
    let volume_source = indicator_reads_volume(kind).then_some(volume);
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
    // Mid-history and trailing whitespace, then the same with leading whitespace too (every
    // output then starts at its first value, later than over a gap-free source).
    for leading in [&[][..], &[0, 1, 2][..]] {
        let blank = [leading, &[7, 18, 19, 31, 45, 58, 59]].concat();
        let filled_blank = [leading, &[7, 18, 19, 45, 58, 59]].concat();
        whitespace_rows_never_poison_bindings(rows, &blank, &filled_blank);
    }
}

fn whitespace_rows_never_poison_bindings(rows: usize, blank: &[usize], filled_blank: &[usize]) {
    let spaced = WhitespaceSource::new(rows, blank);
    let compact = spaced.compacted();
    let filled = WhitespaceSource::new(rows, filled_blank);
    let filled_compact = filled.compacted();
    for kind in every_indicator_kind_with_conventions() {
        // Structure studies count bars: a whitespace row breaks a pivot window rather than
        // vanishing (upstream's rule; see docs/features/studies.md), so their reference is the
        // same rows rebuilt fresh, and their whitespace rows must still emit nothing.
        let structure = structure_kind(&kind);
        let (reference, reference_outputs) = if structure {
            whitespace_chart(&kind, &spaced, rows)
        } else {
            whitespace_chart(&kind, &compact, compact.times.len())
        };
        if structure {
            for &output in &reference_outputs {
                let (times, values) = reference.data.series_data(output).unwrap();
                for (time, value) in times.iter().zip(values[3]) {
                    let row = spaced
                        .times
                        .iter()
                        .position(|t| *t as i64 == *time)
                        .unwrap();
                    assert!(
                        !spaced.blank[row] || value.is_nan(),
                        "{kind:?} at {time}: whitespace emitted {value}"
                    );
                }
            }
        }

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
        let (filled_reference, filled_outputs) = if structure {
            whitespace_chart(&kind, &filled, rows)
        } else {
            whitespace_chart(&kind, &filled_compact, filled_compact.times.len())
        };
        assert_whitespace_outputs_match(
            &kind,
            (&streamed, &streamed_outputs),
            (&filled_reference, &filled_outputs),
        );
    }
}

#[test]
fn macd_histogram_colors_match_a_full_rebuild_for_every_convention_and_whitespace() {
    for seed in [IndicatorSeed::Sma, IndicatorSeed::FirstValue] {
        let kind = IndicatorKind::Macd {
            fast: 3,
            slow: 6,
            signal: 4,
            seed,
            histogram_multiplier: 2.0,
        };
        assert_streamed_histogram_colors_match_a_full_rebuild(&kind, 2);
    }
}

#[test]
fn awesome_and_volume_oscillator_histogram_colors_match_a_full_rebuild_over_whitespace() {
    assert_streamed_histogram_colors_match_a_full_rebuild(&IndicatorKind::AwesomeOscillator, 0);
    assert_streamed_histogram_colors_match_a_full_rebuild(
        &IndicatorKind::VolumeOscillator {
            fast: 3,
            slow: 7,
            signal: 4,
        },
        2,
    );
}

/// Stream `kind` bar by bar over a source with mid-history whitespace and compare the per-bar
/// momentum colours of its histogram output with a full rebuild; every finite bar is coloured.
fn assert_streamed_histogram_colors_match_a_full_rebuild(kind: &IndicatorKind, histogram: usize) {
    // Leading whitespace moves the histogram's first row, which its colours must follow.
    for blank in [&[9, 22, 23, 40][..], &[0, 1, 2, 9, 22, 23, 40][..]] {
        assert_streamed_histogram_colors_match(kind, histogram, blank);
    }
}

fn assert_streamed_histogram_colors_match(kind: &IndicatorKind, histogram: usize, blank: &[usize]) {
    let spaced = WhitespaceSource::new(48, blank);
    let (mut streamed, outputs) = whitespace_chart(kind, &spaced, 5);
    for row in 5..48 {
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
    let (full, full_outputs) = whitespace_chart(kind, &spaced, 48);
    let values = full.data.series_data(full_outputs[histogram]).unwrap().1[3];
    assert_eq!(
        streamed.data.series_data(outputs[histogram]).unwrap().1[3].len(),
        values.len()
    );
    let mut coloured = 0;
    for (row, value) in values.iter().enumerate() {
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
        let full_color = color(&full, full_outputs[histogram]);
        assert_eq!(
            color(&streamed, outputs[histogram]),
            full_color,
            "{kind:?} histogram row {row}"
        );
        if value.is_finite() {
            assert_ne!(
                full_color,
                aeris_charts_core::model::data_layer::POINT_COLOR_ABSENT,
                "{kind:?} histogram row {row} is coloured"
            );
            coloured += 1;
        }
    }
    assert!(coloured > 0, "{kind:?} draws histogram bars");
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
    assert_eq!(
        seed.options,
        Some(vec!["sma".to_string(), "first_value".to_string()])
    );
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
    assert_eq!(
        seed.options,
        Some(vec!["fifty".to_string(), "first_value".to_string()])
    );
}

#[test]
fn china_kdj_starts_at_the_first_loaded_bar_through_streaming_trims_and_corrections() {
    use aeris_charts_indicators::{IndicatorConvention, KdjSeed, kdj_with_seed};
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

    assert!(
        chart
            .add_indicator_kind_with_sources(
                0,
                IndicatorInputSource::Close,
                IndicatorKind::Vwap,
                None,
                Some(amount),
            )
            .is_empty()
    );
    assert!(
        chart
            .add_indicator_kind_with_sources(
                0,
                IndicatorInputSource::Close,
                IndicatorKind::Vwap,
                Some(volume),
                Some(volume),
            )
            .is_empty()
    );
    assert!(
        chart
            .add_indicator_kind_with_sources(
                0,
                IndicatorInputSource::Close,
                IndicatorKind::Obv,
                Some(volume),
                Some(amount),
            )
            .is_empty()
    );

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
            let drawn = inputs[rng.below(inputs.len())];
            // Structure studies accept only the close input (and refuse input switches).
            let input = if structure_kind(&kind) {
                IndicatorInputSource::Close
            } else {
                drawn
            };
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

mod session_study_regressions {
    use super::*;

    fn boundary(start_time: i64, end_time: i64, session_id: u64) -> ResampleBoundary {
        ResampleBoundary {
            start_time,
            end_time,
            session_id,
        }
    }

    fn install(chart: &mut ChartEngine, times: &[f64], highs: &[f64], lows: &[f64]) {
        let open = highs
            .iter()
            .zip(lows)
            .map(|(h, l)| (h + l) / 2.0)
            .collect::<Vec<_>>();
        chart
            .set_series_data(0, times, &open, highs, lows, &open)
            .unwrap();
    }

    fn values(chart: &ChartEngine, id: SeriesId) -> Vec<Option<f64>> {
        let (times, columns) = chart.data.series_data(id).unwrap();
        let mut row = 0;
        chart
            .data
            .series_data(0)
            .unwrap()
            .0
            .iter()
            .map(|source_time| {
                if times.get(row) == Some(source_time) {
                    let value = columns[3][row];
                    row += 1;
                    value.is_finite().then_some(value)
                } else {
                    None
                }
            })
            .collect()
    }

    fn all_studies(chart: &mut ChartEngine) -> Vec<Vec<SeriesId>> {
        let mut bindings = Vec::new();
        for calendar in [
            StudyCalendarPolicy::Utc,
            StudyCalendarPolicy::Host,
            StudyCalendarPolicy::Exchange,
        ] {
            bindings.push(chart.add_session_levels(0, calendar));
            for period in [
                PreviousPeriod::Day,
                PreviousPeriod::Week,
                PreviousPeriod::Month,
            ] {
                bindings.push(chart.add_previous_period_levels(0, period, calendar));
            }
            bindings.push(chart.add_opening_range(0, 60, calendar));
        }
        assert_eq!(
            bindings.iter().map(Vec::len).collect::<Vec<_>>(),
            [2, 3, 3, 3, 3, 2, 3, 3, 3, 3, 2, 3, 3, 3, 3]
        );
        bindings
    }

    #[test]
    fn host_calendar_merges_touching_identity_but_keeps_gaps_and_new_sessions_empty() {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        let times = [0., 30., 60., 90., 120., 150., 160., 86400., 86430.];
        let high = [12., 15., 14., 20., 17., 16., 50., 30., 34.];
        let low = [8., 7., 9., 6., 10., 11., 40., 25., 24.];
        install(&mut chart, &times, &high, &low);
        chart
            .set_study_calendar(vec![
                boundary(0, 100, 7),
                boundary(100, 160, 7),
                boundary(86400, 86500, 8),
            ])
            .unwrap();
        assert_eq!(
            chart.study_session_spans(),
            vec![
                aeris_charts_indicators::SessionSpan {
                    start: 0,
                    end: 160,
                    session_id: 7
                },
                aeris_charts_indicators::SessionSpan {
                    start: 86400,
                    end: 86500,
                    session_id: 8
                }
            ]
        );
        let session = chart.add_session_levels(0, StudyCalendarPolicy::Host);
        let previous =
            chart.add_previous_period_levels(0, PreviousPeriod::Day, StudyCalendarPolicy::Host);
        let opening = chart.add_opening_range(0, 70, StudyCalendarPolicy::Host);
        assert_eq!(
            values(&chart, session[0]),
            [
                Some(12.),
                Some(15.),
                Some(15.),
                Some(20.),
                Some(20.),
                Some(20.),
                None,
                Some(30.),
                Some(34.)
            ]
        );
        assert_eq!(
            values(&chart, session[1]),
            [
                Some(8.),
                Some(7.),
                Some(7.),
                Some(6.),
                Some(6.),
                Some(6.),
                None,
                Some(25.),
                Some(24.)
            ]
        );
        assert_eq!(
            values(&chart, opening[0]),
            [
                Some(12.),
                Some(15.),
                Some(15.),
                Some(15.),
                Some(15.),
                Some(15.),
                None,
                Some(30.),
                Some(34.)
            ]
        );
        assert_eq!(
            values(&chart, opening[2]),
            [
                Some(10.),
                Some(11.),
                Some(11.),
                Some(11.),
                Some(11.),
                Some(11.),
                None,
                Some(27.5),
                Some(29.)
            ]
        );
        assert_eq!(values(&chart, previous[0])[..7], [None; 7]);
        assert_eq!(values(&chart, previous[0])[7..], [Some(20.), Some(20.)]);
        assert_eq!(values(&chart, previous[1])[7..], [Some(6.), Some(6.)]);
        assert_eq!(values(&chart, previous[2])[7..], [Some(13.5), Some(13.5)]);

        // A rejected replacement cannot disturb the live calendar or any outputs.
        let before = values(&chart, session[0]);
        assert!(
            chart
                .set_study_calendar(vec![boundary(0, 100, 1), boundary(90, 200, 2)])
                .is_err()
        );
        assert_eq!(values(&chart, session[0]), before);
        chart
            .set_study_calendar(vec![boundary(0, 80, 1), boundary(86400, 86500, 8)])
            .unwrap();
        assert_eq!(values(&chart, session[0])[3..7], [None; 4]);
        chart.clear_study_calendar();
        assert!(values(&chart, session[0]).iter().all(Option::is_none));
    }

    #[test]
    fn utc_previous_periods_use_day_monday_week_and_civil_month_not_host_boundaries() {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        // Monday Jan 29, Wednesday Jan 31, Thursday Feb 1, Monday Feb 5, 2024.
        let times = [1706486400., 1706659200., 1706745600., 1707091200.];
        install(
            &mut chart,
            &times,
            &[12., 22., 32., 42.],
            &[8., 18., 28., 38.],
        );
        chart
            .set_study_calendar(vec![boundary(1706486400, 1707177600, 1)])
            .unwrap();
        let day =
            chart.add_previous_period_levels(0, PreviousPeriod::Day, StudyCalendarPolicy::Utc);
        let week =
            chart.add_previous_period_levels(0, PreviousPeriod::Week, StudyCalendarPolicy::Utc);
        let month =
            chart.add_previous_period_levels(0, PreviousPeriod::Month, StudyCalendarPolicy::Utc);
        let session = chart.add_session_levels(0, StudyCalendarPolicy::Utc);
        let opening = chart.add_opening_range(0, 60, StudyCalendarPolicy::Utc);
        assert_eq!(
            values(&chart, session[0]),
            [Some(12.), Some(22.), Some(32.), Some(42.)]
        );
        assert_eq!(
            values(&chart, opening[2]),
            [Some(10.), Some(20.), Some(30.), Some(40.)]
        );
        assert_eq!(
            values(&chart, day[0]),
            [None, Some(12.), Some(22.), Some(32.)]
        );
        assert_eq!(values(&chart, week[0]), [None, None, None, Some(32.)]);
        assert_eq!(values(&chart, week[1]), [None, None, None, Some(8.)]);
        assert_eq!(values(&chart, month[0]), [None, None, Some(22.), Some(22.)]);
        assert_eq!(values(&chart, month[2]), [None, None, Some(20.), Some(20.)]);
        chart.clear_study_calendar();
        assert_eq!(values(&chart, week[0])[3], Some(32.));
    }

    #[test]
    fn calendar_replacement_tip_updates_and_chained_sma_match_fresh_engine() {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        let times = [0., 30., 60., 90., 120., 180.];
        let high = [12., 15., 14., 20., 17., 30.];
        let low = [8., 7., 9., 6., 10., 25.];
        install(&mut chart, &times, &high, &low);
        let calendar = vec![boundary(0, 160, 1), boundary(180, 260, 2)];
        chart.set_study_calendar(calendar.clone()).unwrap();
        let levels = chart.add_session_levels(0, StudyCalendarPolicy::Host);
        let opening = chart.add_opening_range(0, 90, StudyCalendarPolicy::Host);
        let sma = chart.add_indicator_kind(levels[0], IndicatorKind::Sma { period: 2 }, None)[0];
        let check =
            |chart: &ChartEngine, calendar: Vec<ResampleBoundary>, high: &[f64], low: &[f64]| {
                let mut fresh = ChartEngine::new(800.0, 500.0, 1.0);
                install(&mut fresh, &times, high, low);
                fresh.set_study_calendar(calendar).unwrap();
                let expected = fresh.add_session_levels(0, StudyCalendarPolicy::Host);
                let expected_open = fresh.add_opening_range(0, 90, StudyCalendarPolicy::Host);
                let expected_sma =
                    fresh.add_indicator_kind(expected[0], IndicatorKind::Sma { period: 2 }, None)
                        [0];
                for (left, right) in levels
                    .iter()
                    .zip(expected.iter())
                    .chain(opening.iter().zip(expected_open.iter()))
                {
                    assert_eq!(values(chart, *left), values(&fresh, *right));
                }
                assert_eq!(values(chart, sma), values(&fresh, expected_sma));
            };
        check(&chart, calendar.clone(), &high, &low);
        assert!(chart.update_series_bar(0, 180., [31., 36., 22., 29.]));
        let mut high = high;
        let mut low = low;
        high[5] = 36.;
        low[5] = 22.;
        check(&chart, calendar.clone(), &high, &low);
        let changed = vec![boundary(0, 100, 1), boundary(100, 200, 2)];
        chart.set_study_calendar(changed.clone()).unwrap();
        check(&chart, changed, &high, &low);
        assert!(chart.update_series_bar(0, 190., [29., 38., 20., 31.]));
        assert_eq!(values(&chart, levels[0]).last(), Some(&Some(38.)));
        assert_eq!(values(&chart, opening[0]).last(), Some(&Some(36.)));
        assert_eq!(values(&chart, sma).last(), Some(&Some(37.)));
    }

    #[test]
    fn all_session_variants_are_prefix_stable_through_replay_seek_and_tip_replacement() {
        let times = [
            1706486400.,
            1706486460.,
            1706659200.,
            1706745600.,
            1706745660.,
            1707091200.,
        ];
        let high = [12., 15., 22., 32., 34., 42.];
        let low = [8., 7., 18., 28., 24., 38.];
        let calendar = vec![
            boundary(1706486400, 1706659200, 1),
            boundary(1706659200, 1706745600, 1),
            boundary(1706745600, 1706918400, 2),
            boundary(1707091200, 1707177600, 3),
        ];
        // The exchange variants run on a New York exchange calendar with an evening session start.
        let exchange = |chart: &mut ChartEngine| {
            chart.set_time_zone("America/New_York").unwrap();
            chart.set_session_start_seconds(-7 * 3_600).unwrap();
        };
        let mut replay = ChartEngine::new(800.0, 500.0, 1.0);
        replay.set_study_calendar(calendar.clone()).unwrap();
        exchange(&mut replay);
        let bindings = all_studies(&mut replay);
        for len in (0..=times.len()).chain((0..times.len()).rev()) {
            install(&mut replay, &times[..len], &high[..len], &low[..len]);
            let mut fresh = ChartEngine::new(800.0, 500.0, 1.0);
            fresh.set_study_calendar(calendar.clone()).unwrap();
            exchange(&mut fresh);
            install(&mut fresh, &times[..len], &high[..len], &low[..len]);
            let reference = all_studies(&mut fresh);
            for (actual, expected) in bindings.iter().zip(reference.iter()) {
                for (&actual, &expected) in actual.iter().zip(expected) {
                    assert_eq!(
                        values(&replay, actual),
                        values(&fresh, expected),
                        "prefix {len}"
                    );
                }
            }
        }
        install(&mut replay, &times, &high, &low);
        assert!(replay.update_series_bar(0, times[5], [40., 60., 35., 45.]));
        let mut high = high;
        let mut low = low;
        high[5] = 60.;
        low[5] = 35.;
        let mut fresh = ChartEngine::new(800.0, 500.0, 1.0);
        fresh.set_study_calendar(calendar).unwrap();
        exchange(&mut fresh);
        install(&mut fresh, &times, &high, &low);
        let reference = all_studies(&mut fresh);
        for (actual, expected) in bindings.iter().zip(reference.iter()) {
            for (&actual, &expected) in actual.iter().zip(expected) {
                assert_eq!(values(&replay, actual), values(&fresh, expected));
            }
        }
    }

    #[test]
    fn pd2_clock_seek_matches_fresh_prefix_for_all_seven_structure_studies() {
        let times = (1..=12).map(f64::from).collect::<Vec<_>>();
        let highs = [10., 14., 12., 18., 16., 20., 19., 17., 25., 21., 24., 15.];
        let lows = [8., 9., 10., 15., 11., 8., 16., 13., 22., 17., 20., 9.];
        let kinds = [
            IndicatorKind::SwingPoints { left: 1, right: 1 },
            IndicatorKind::MarketStructure {
                left: 1,
                right: 1,
                break_on: StructureBreakOn::Wick,
            },
            IndicatorKind::FairValueGaps {
                min_size: 0.0,
                mitigation: StructureMitigation::Touch,
                mitigation_price: StructureMitigationPrice::Wick,
                max_active: 3,
                show_mitigated: true,
            },
            IndicatorKind::OrderBlocks {
                left: 1,
                right: 1,
                break_on: StructureBreakOn::Wick,
                zone: OrderBlockZone::Wick,
                mitigation: StructureMitigation::Touch,
                mitigation_price: StructureMitigationPrice::Wick,
                max_active: 3,
                show_mitigated: true,
            },
            IndicatorKind::SessionLevels {
                calendar: StudyCalendarPolicy::Utc,
            },
            IndicatorKind::PreviousPeriodLevels {
                period: PreviousPeriod::Day,
                calendar: StudyCalendarPolicy::Utc,
            },
            IndicatorKind::OpeningRange {
                duration_seconds: 3,
                calendar: StudyCalendarPolicy::Utc,
            },
        ];
        let mut replay = ChartEngine::new(800.0, 500.0, 1.0);
        install(&mut replay, &times, &highs, &lows);
        let outputs = kinds
            .iter()
            .map(|kind| replay.add_indicator_kind(0, kind.clone(), None))
            .collect::<Vec<_>>();
        for len in [8usize, 4, 11, 1, 12, 6] {
            replay
                .set_replay_clock_micros(Some(times[len - 1] as i64 * 1_000_000))
                .unwrap();
            let mut fresh = ChartEngine::new(800.0, 500.0, 1.0);
            install(&mut fresh, &times[..len], &highs[..len], &lows[..len]);
            for (kind, actual) in kinds.iter().zip(&outputs) {
                let expected = fresh.add_indicator_kind(0, kind.clone(), None);
                for (&actual, &expected) in actual.iter().zip(&expected) {
                    assert_eq!(
                        values(&replay, actual),
                        values(&fresh, expected),
                        "{kind:?} prefix {len}"
                    );
                }
                if matches!(
                    kind,
                    IndicatorKind::SwingPoints { .. }
                        | IndicatorKind::MarketStructure { .. }
                        | IndicatorKind::FairValueGaps { .. }
                        | IndicatorKind::OrderBlocks { .. }
                ) {
                    assert_eq!(
                        replay.study_annotations(actual[0]).unwrap(),
                        fresh.study_annotations(expected[0]).unwrap(),
                        "{kind:?} prefix {len}"
                    );
                }
            }
        }
    }

    #[test]
    fn retention_trim_rebuilds_structure_from_only_the_retained_rows() {
        let times = (0..96).map(|row| row as f64 + 1.).collect::<Vec<_>>();
        let highs = (0..96)
            .map(|row| 30. + (row / 7) as f64 * 16. + [0., 4., 2., 8., 6., 10., 11.][row % 7])
            .collect::<Vec<_>>();
        let lows = highs.iter().map(|high| high - 6.).collect::<Vec<_>>();
        let opens = highs
            .iter()
            .enumerate()
            .map(|(row, high)| high - if row % 7 == 4 { 1. } else { 3. })
            .collect::<Vec<_>>();
        let closes = highs
            .iter()
            .enumerate()
            .map(|(row, high)| high - if row % 7 == 4 { 5. } else { 3. })
            .collect::<Vec<_>>();
        let kinds = [
            IndicatorKind::SwingPoints { left: 1, right: 1 },
            IndicatorKind::MarketStructure {
                left: 1,
                right: 1,
                break_on: StructureBreakOn::Wick,
            },
            IndicatorKind::FairValueGaps {
                min_size: 0.,
                mitigation: StructureMitigation::Touch,
                mitigation_price: StructureMitigationPrice::Wick,
                max_active: 3,
                show_mitigated: true,
            },
            IndicatorKind::OrderBlocks {
                left: 1,
                right: 1,
                break_on: StructureBreakOn::Wick,
                zone: OrderBlockZone::Wick,
                mitigation: StructureMitigation::Touch,
                mitigation_price: StructureMitigationPrice::Wick,
                max_active: 3,
                show_mitigated: true,
            },
        ];
        let mut chart = ChartEngine::new(800., 500., 1.);
        chart
            .set_series_data(0, &times, &opens, &highs, &lows, &closes)
            .unwrap();
        let bindings = kinds
            .iter()
            .map(|kind| chart.add_indicator_kind(0, kind.clone(), None))
            .collect::<Vec<_>>();
        assert!(chart.set_series_max_points(0, Some(40)));
        for row in 96..110 {
            let high = 30. + (row / 7) as f64 * 16. + [0., 4., 2., 8., 6., 10., 11.][row % 7];
            let (open, close) = if row % 7 == 4 {
                (high - 1., high - 5.)
            } else {
                (high - 3., high - 3.)
            };
            assert!(chart.update_series_bar(0, row as f64 + 1., [open, high, high - 6., close]));
        }
        let (retained, columns) = chart.data.series_data(0).unwrap();
        let (retained, open, high, low, close) = (
            retained.to_vec(),
            columns[0].to_vec(),
            columns[1].to_vec(),
            columns[2].to_vec(),
            columns[3].to_vec(),
        );
        assert!(
            retained[0] > times[56] as i64,
            "streaming must evict additional leading rows"
        );
        let mut fresh = ChartEngine::new(800., 500., 1.);
        fresh
            .set_series_data(
                0,
                &retained.iter().map(|&t| t as f64).collect::<Vec<_>>(),
                &open,
                &high,
                &low,
                &close,
            )
            .unwrap();
        for (kind, actual) in kinds.iter().zip(&bindings) {
            let expected = fresh.add_indicator_kind(0, kind.clone(), None);
            for (&actual, &expected) in actual.iter().zip(&expected) {
                assert_eq!(values(&chart, actual), values(&fresh, expected), "{kind:?}");
            }
            let snapshot = chart.study_annotations(actual[0]).unwrap();
            assert!(
                !snapshot.markers().is_empty() || !snapshot.zones().is_empty(),
                "{kind:?} must generate annotations after trimming"
            );
            assert_eq!(
                snapshot,
                fresh.study_annotations(expected[0]).unwrap(),
                "{kind:?}"
            );
            assert!(
                snapshot
                    .markers()
                    .iter()
                    .all(|marker| marker.row < retained.len()
                        && marker.from_row.is_none_or(|row| row < retained.len()))
            );
            assert!(
                snapshot
                    .zones()
                    .iter()
                    .all(|zone| zone.start_row < retained.len())
            );
        }
    }

    #[test]
    fn kama_and_swings_on_chained_indicator_outputs_follow_source_repairs() {
        let times = (1..=16).map(f64::from).collect::<Vec<_>>();
        let high = [
            12., 14., 18., 14., 11., 16., 22., 19., 15., 12., 17., 24., 18., 13., 20., 15.,
        ];
        let low = high.map(|high| high - 4.0);
        let setup = |chart: &mut ChartEngine| {
            let base = chart.add_indicator_kind(0, IndicatorKind::Sma { period: 2 }, None)[0];
            let kama = chart.add_indicator_kind(
                base,
                IndicatorKind::Kama {
                    period: 2,
                    fast: 2,
                    slow: 5,
                },
                None,
            )[0];
            let swings = chart.add_swing_points(kama, 1, 1);
            assert_eq!(swings.len(), 2);
            (base, kama, swings)
        };
        let mut incremental = ChartEngine::new(800.0, 500.0, 1.0);
        install(&mut incremental, &times[..8], &high[..8], &low[..8]);
        let (base, kama, swings) = setup(&mut incremental);
        let mut final_high = high;
        let mut final_low = low;
        for len in 9..=times.len() {
            assert!(incremental.update_series_bar(
                0,
                times[len - 1],
                [
                    (high[len - 1] + low[len - 1]) / 2.0,
                    high[len - 1],
                    low[len - 1],
                    (high[len - 1] + low[len - 1]) / 2.0
                ],
            ));
        }
        for (row, new_high) in [(15, 28.), (4, 25.)] {
            final_high[row] = new_high;
            final_low[row] = new_high - 4.0;
            assert!(incremental.update_series_bar(
                0,
                times[row],
                [new_high - 2.0, new_high, new_high - 4.0, new_high - 2.0],
            ));
            let mut fresh = ChartEngine::new(800.0, 500.0, 1.0);
            install(&mut fresh, &times, &final_high, &final_low);
            let (fresh_base, fresh_kama, fresh_swings) = setup(&mut fresh);
            for (actual, expected) in [(base, fresh_base), (kama, fresh_kama)]
                .into_iter()
                .chain(swings.iter().copied().zip(fresh_swings.iter().copied()))
            {
                assert_eq!(values(&incremental, actual), values(&fresh, expected));
            }
            assert_eq!(
                incremental.study_annotations(swings[0]).unwrap(),
                fresh.study_annotations(fresh_swings[0]).unwrap(),
            );
        }
    }

    #[test]
    fn opening_range_zero_and_invalid_sources_are_atomic() {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        let before = chart.indicator_bindings().len();
        assert!(
            chart
                .add_opening_range(0, 0, StudyCalendarPolicy::Utc)
                .is_empty()
        );
        assert!(
            chart
                .add_session_levels(u32::MAX, StudyCalendarPolicy::Host)
                .is_empty()
        );
        assert!(
            chart
                .add_previous_period_levels(
                    u32::MAX,
                    PreviousPeriod::Month,
                    StudyCalendarPolicy::Utc
                )
                .is_empty()
        );
        assert!(
            chart
                .add_opening_range(u32::MAX, 30, StudyCalendarPolicy::Host)
                .is_empty()
        );
        assert_eq!(chart.indicator_bindings().len(), before);
        let valid = chart.add_opening_range(0, 1, StudyCalendarPolicy::Utc);
        assert_eq!(valid.len(), 3);
    }

    #[test]
    fn v3_persists_study_policies_not_runtime_calendar_or_annotations() {
        let times = [0., 30., 60., 86400.];
        let high = [12., 15., 14., 20.];
        let low = [8., 7., 9., 16.];
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        install(&mut chart, &times, &high, &low);
        chart
            .set_study_calendar(vec![boundary(0, 100, 1), boundary(86400, 86500, 2)])
            .unwrap();
        let session = chart.add_session_levels(0, StudyCalendarPolicy::Host);
        let previous =
            chart.add_previous_period_levels(0, PreviousPeriod::Day, StudyCalendarPolicy::Utc);
        let opening = chart.add_opening_range(0, 50, StudyCalendarPolicy::Host);
        let sma = chart.add_indicator_kind(session[0], IndicatorKind::Sma { period: 2 }, None)[0];
        let mut annotations = aeris_charts_indicators::StudyAnnotations::default();
        annotations.push_marker(aeris_charts_indicators::StudyMarker {
            row: 1,
            confirm_row: 2,
            price: 15.,
            kind: aeris_charts_indicators::StudyMarkerKind::SwingHigh,
            from_row: None,
        });
        assert!(chart.inject_study_annotations_for_test(session[0], annotations));
        let document = chart.export_state_json().unwrap();
        let json: serde_json::Value = serde_json::from_str(&document).unwrap();
        assert_eq!(json["schema_version"], 3);
        assert!(json.get("study_calendar").is_none());
        assert!(!document.contains("\"annotations\""));
        assert_eq!(json["indicators"][0]["kind"]["calendar"], "host");
        assert_eq!(json["indicators"][1]["kind"]["period"], "day");
        assert_eq!(json["indicators"][2]["kind"]["duration_seconds"], 50);

        let mut restored = ChartEngine::new(800.0, 500.0, 1.0);
        install(&mut restored, &times, &high, &low);
        restored.import_state_json(&document).unwrap();
        assert!(restored.study_session_spans().is_empty());
        assert_eq!(restored.indicator_bindings().len(), 4);
        assert_eq!(restored.indicator_bindings()[0].outputs, session);
        assert_eq!(restored.indicator_bindings()[1].outputs, previous);
        assert_eq!(restored.indicator_bindings()[2].outputs, opening);
        assert_eq!(restored.indicator_bindings()[3].outputs, [sma]);
        assert!(values(&restored, session[0]).iter().all(Option::is_none));
        assert_eq!(values(&restored, previous[0]), values(&chart, previous[0]));
        assert!(restored.study_annotations(session[0]).is_err());
        restored
            .set_study_calendar(vec![boundary(0, 100, 1), boundary(86400, 86500, 2)])
            .unwrap();
        for id in session.into_iter().chain(opening).chain([sma]) {
            assert_eq!(values(&restored, id), values(&chart, id));
        }

        // Invalid persisted parameters fail without installing any binding.
        for (index, field, invalid) in [
            (0, "calendar", serde_json::json!("local")),
            (1, "period", serde_json::json!("quarter")),
            (2, "duration_seconds", serde_json::json!(0)),
        ] {
            let mut invalid_document = json.clone();
            invalid_document["indicators"][index]["kind"][field] = invalid;
            let mut target = ChartEngine::new(800.0, 500.0, 1.0);
            install(&mut target, &times, &high, &low);
            let before = target.export_state_json().unwrap();
            assert!(
                target
                    .import_state_json(&invalid_document.to_string())
                    .is_err()
            );
            assert_eq!(target.export_state_json().unwrap(), before);
        }
    }

    /// UTC instant of an exchange-local CST (UTC+8) wall time.
    fn cst(month: u32, day: u32, hour: i64, minute: i64) -> f64 {
        let day =
            aeris_charts_core::scale::time_tick_marks::days_from_civil(2024, month, day).unwrap();
        (day * 86_400 + (hour - 8) * 3_600 + minute * 60) as f64
    }

    /// A SHFE-style chart: day sessions and a 21:00-02:30 night session that opens the next
    /// trading day, including a Friday night that belongs to Monday.
    fn shfe_bars() -> (Vec<f64>, Vec<f64>, Vec<f64>) {
        let rows = [
            (cst(1, 31, 9, 0), 10., 5.), // 0: Wed Jan 31 day session, trading day Jan 31
            (cst(1, 31, 21, 0), 20., 15.), // 1: Wed night: trading day Thu Feb 1 (February)
            (cst(2, 1, 0, 30), 22., 14.), // 2: past midnight, still Feb 1's night session
            (cst(2, 1, 9, 0), 21., 16.), // 3: Thu Feb 1 day session
            (cst(2, 2, 21, 0), 30., 25.), // 4: Fri night: trading day Mon Feb 5 (next week)
            (cst(2, 3, 1, 0), 31., 24.), // 5: Sat 01:00, still Monday's night session
            (cst(2, 5, 9, 0), 32., 26.), // 6: Mon Feb 5 day session
        ];
        (
            rows.iter().map(|row| row.0).collect(),
            rows.iter().map(|row| row.1).collect(),
            rows.iter().map(|row| row.2).collect(),
        )
    }

    fn shfe_chart() -> ChartEngine {
        let (times, high, low) = shfe_bars();
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        install(&mut chart, &times, &high, &low);
        assert!(chart.set_time_zone("Asia/Shanghai").unwrap());
        chart.set_session_start_seconds(-3 * 3_600).unwrap();
        chart
    }

    #[test]
    fn exchange_calendar_counts_night_sessions_in_the_next_trading_day_week_and_month() {
        let mut chart = shfe_chart();
        let exchange = |chart: &mut ChartEngine, period| {
            chart.add_previous_period_levels(0, period, StudyCalendarPolicy::Exchange)
        };
        let day = exchange(&mut chart, PreviousPeriod::Day);
        let week = exchange(&mut chart, PreviousPeriod::Week);
        let month = exchange(&mut chart, PreviousPeriod::Month);
        let utc_week =
            chart.add_previous_period_levels(0, PreviousPeriod::Week, StudyCalendarPolicy::Utc);
        let utc_month =
            chart.add_previous_period_levels(0, PreviousPeriod::Month, StudyCalendarPolicy::Utc);
        let session = chart.add_session_levels(0, StudyCalendarPolicy::Exchange);
        // The night session (rows 1-2) and the following day session (row 3) are one trading day.
        assert_eq!(
            values(&chart, session[0]),
            [
                Some(10.),
                Some(20.),
                Some(22.),
                Some(22.),
                Some(30.),
                Some(31.),
                Some(32.)
            ]
        );
        assert_eq!(
            values(&chart, day[0]),
            [
                None,
                Some(10.),
                Some(10.),
                Some(10.),
                Some(22.),
                Some(22.),
                Some(22.)
            ]
        );
        // The Friday night session opens Monday's week: last week's levels appear at 21:00 Friday.
        assert_eq!(
            values(&chart, week[0]),
            [None, None, None, None, Some(22.), Some(22.), Some(22.)]
        );
        assert_eq!(
            values(&chart, week[1]),
            [None, None, None, None, Some(5.), Some(5.), Some(5.)]
        );
        assert_eq!(values(&chart, week[2])[4], Some(18.5));
        // Wednesday night already trades February 1: January's levels appear on that row.
        assert_eq!(
            values(&chart, month[0]),
            [
                None,
                Some(10.),
                Some(10.),
                Some(10.),
                Some(10.),
                Some(10.),
                Some(10.)
            ]
        );
        assert_eq!(values(&chart, month[1])[1], Some(5.));
        // UTC counts the wall-clock UTC calendar instead: the week turns on Monday morning
        // (row 6) and the month on Thursday 09:00 CST (row 3).
        assert_eq!(values(&chart, utc_week[0])[..6], [None; 6]);
        assert_eq!(values(&chart, utc_week[0])[6], Some(31.));
        assert_eq!(values(&chart, utc_month[0])[..3], [None; 3]);
        assert_eq!(values(&chart, utc_month[0])[3], Some(22.));
    }

    #[test]
    fn exchange_opening_range_starts_at_the_night_session_open() {
        let mut chart = shfe_chart();
        let opening = chart.add_opening_range(0, 3_600, StudyCalendarPolicy::Exchange);
        // Thursday's range is the first hour after Wednesday 21:00 (row 1 only); Monday's is the
        // first hour after Friday 21:00 (row 4 only), not after the Monday day start.
        assert_eq!(
            values(&chart, opening[0]),
            [
                None,
                Some(20.),
                Some(20.),
                Some(20.),
                Some(30.),
                Some(30.),
                Some(30.)
            ]
        );
        assert_eq!(
            values(&chart, opening[1]),
            [
                None,
                Some(15.),
                Some(15.),
                Some(15.),
                Some(25.),
                Some(25.),
                Some(25.)
            ]
        );
    }

    #[test]
    fn exchange_calendar_equals_utc_while_the_exchange_time_is_utc() {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        // Irregular bars across Monday weeks and a civil month boundary, starting before 1970.
        let times = (0..400_i64)
            .map(|row| row * row * 97 % 3_000_000 - 900_000)
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .map(|time| time as f64)
            .collect::<Vec<_>>();
        let high = (0..times.len())
            .map(|row| 100. + (row % 23) as f64)
            .collect::<Vec<_>>();
        let low = (0..times.len())
            .map(|row| 80. - (row % 19) as f64)
            .collect::<Vec<_>>();
        install(&mut chart, &times, &high, &low);
        for calendar_dates in [false, true] {
            chart.set_calendar_date_axis(calendar_dates);
            let mut pairs = Vec::new();
            pairs.push((
                chart.add_session_levels(0, StudyCalendarPolicy::Exchange),
                chart.add_session_levels(0, StudyCalendarPolicy::Utc),
            ));
            for period in [
                PreviousPeriod::Day,
                PreviousPeriod::Week,
                PreviousPeriod::Month,
            ] {
                pairs.push((
                    chart.add_previous_period_levels(0, period, StudyCalendarPolicy::Exchange),
                    chart.add_previous_period_levels(0, period, StudyCalendarPolicy::Utc),
                ));
            }
            pairs.push((
                chart.add_opening_range(0, 5_000, StudyCalendarPolicy::Exchange),
                chart.add_opening_range(0, 5_000, StudyCalendarPolicy::Utc),
            ));
            for (exchange, utc) in pairs {
                for (left, right) in exchange.into_iter().zip(utc) {
                    let expected = values(&chart, right);
                    assert!(expected.iter().any(Option::is_some));
                    assert_eq!(
                        values(&chart, left),
                        expected,
                        "calendar dates {calendar_dates}"
                    );
                }
            }
        }
    }

    #[test]
    fn exchange_time_changes_rebuild_every_exchange_study_in_one_operation() {
        let (times, high, low) = shfe_bars();
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        install(&mut chart, &times, &high, &low);
        let add = |chart: &mut ChartEngine, calendar| {
            let mut outputs = chart.add_session_levels(0, calendar);
            outputs.extend(chart.add_previous_period_levels(0, PreviousPeriod::Week, calendar));
            outputs.extend(chart.add_previous_period_levels(0, PreviousPeriod::Month, calendar));
            outputs.extend(chart.add_opening_range(0, 3_600, calendar));
            outputs
        };
        let exchange = add(&mut chart, StudyCalendarPolicy::Exchange);
        let utc = add(&mut chart, StudyCalendarPolicy::Utc);
        let chained = chart.add_indicator_kind(exchange[2], IndicatorKind::Sma { period: 2 }, None);
        let utc_before = utc.iter().map(|&id| values(&chart, id)).collect::<Vec<_>>();
        let utc_generations = utc
            .iter()
            .map(|&id| chart.data.series_generation(id))
            .collect::<Vec<_>>();
        // Each step is one engine call; the expected outputs come from a fresh engine that had
        // the same exchange time before any study was added.
        type Step = fn(&mut ChartEngine);
        let steps: [(Step, Step); 3] = [
            (
                |chart| assert!(chart.set_time_zone("Asia/Shanghai").unwrap()),
                |chart| assert!(chart.set_time_zone("Asia/Shanghai").unwrap()),
            ),
            (
                |chart| chart.set_session_start_seconds(-3 * 3_600).unwrap(),
                |chart| {
                    chart.set_time_zone("Asia/Shanghai").unwrap();
                    chart.set_session_start_seconds(-3 * 3_600).unwrap();
                },
            ),
            (
                |chart| assert!(chart.set_time_zone("Etc/UTC").unwrap()),
                |chart| chart.set_session_start_seconds(-3 * 3_600).unwrap(),
            ),
        ];
        for (step, (change, configure)) in steps.into_iter().enumerate() {
            let before = exchange
                .iter()
                .map(|&id| values(&chart, id))
                .collect::<Vec<_>>();
            change(&mut chart);
            let mut fresh = ChartEngine::new(800.0, 500.0, 1.0);
            install(&mut fresh, &times, &high, &low);
            configure(&mut fresh);
            let fresh_exchange = add(&mut fresh, StudyCalendarPolicy::Exchange);
            let fresh_chained =
                fresh.add_indicator_kind(fresh_exchange[2], IndicatorKind::Sma { period: 2 }, None);
            let after = exchange
                .iter()
                .map(|&id| values(&chart, id))
                .collect::<Vec<_>>();
            assert_ne!(after, before, "step {step} moved no trading-day boundary");
            for (&id, &expected) in exchange.iter().zip(&fresh_exchange) {
                assert_eq!(values(&chart, id), values(&fresh, expected), "step {step}");
            }
            assert_eq!(
                values(&chart, chained[0]),
                values(&fresh, fresh_chained[0]),
                "step {step}: dependents follow in the same operation"
            );
            // Invariant check only, not proof of the resync: session-study outputs reuse source
            // rows, so this rebuild never changes the merged time points and the assertion would
            // hold without `sync_time_points`. The resync is guaranteed by routing through the
            // shared `rebuild_calendar_indicators` helper (the VWAP and pivot rebuild path); the
            // routing itself is what the fresh-engine and chained comparisons above prove.
            assert_eq!(
                chart.data.time_points_generation(),
                chart.synced_time_points_generation
            );
            // UTC studies are left untouched.
            assert_eq!(
                utc.iter().map(|&id| values(&chart, id)).collect::<Vec<_>>(),
                utc_before
            );
            assert_eq!(
                utc.iter()
                    .map(|&id| chart.data.series_generation(id))
                    .collect::<Vec<_>>(),
                utc_generations
            );
        }
    }

    #[test]
    fn exchange_session_studies_do_bounded_work_per_tick() {
        let rows = 5_000;
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        let times = (0..rows).map(|row| row as f64 * 900.0).collect::<Vec<_>>();
        let high = (0..rows)
            .map(|row| 100. + (row % 7) as f64)
            .collect::<Vec<_>>();
        let low = (0..rows)
            .map(|row| 90. - (row % 5) as f64)
            .collect::<Vec<_>>();
        install(&mut chart, &times, &high, &low);
        assert!(chart.set_time_zone("Asia/Shanghai").unwrap());
        chart.set_session_start_seconds(-3 * 3_600).unwrap();
        chart.add_session_levels(0, StudyCalendarPolicy::Exchange);
        chart.add_previous_period_levels(0, PreviousPeriod::Month, StudyCalendarPolicy::Exchange);
        chart.add_opening_range(0, 1_800, StudyCalendarPolicy::Exchange);
        for row in rows..rows + 200 {
            let time = row as f64 * 900.0;
            // A new bar, then a replacement of the forming bar.
            assert!(chart.update_series_bar(0, time, [95.0, 101.0, 89.0, 96.0]));
            assert!(chart.update_series_bar(0, time, [95.0, 102.0, 88.0, 97.0]));
            for binding in &chart.indicators {
                assert!(
                    binding.last_work_rows() <= 2,
                    "{:?} did {} rows of work on a tick",
                    binding.kind,
                    binding.last_work_rows()
                );
            }
        }
    }

    #[test]
    fn v3_exchange_policy_round_trips_and_a_missing_calendar_reads_as_exchange() {
        let (times, high, low) = shfe_bars();
        let mut chart = shfe_chart();
        let session = chart.add_session_levels(0, StudyCalendarPolicy::Exchange);
        let week = chart.add_previous_period_levels(
            0,
            PreviousPeriod::Week,
            StudyCalendarPolicy::Exchange,
        );
        let opening = chart.add_opening_range(0, 3_600, StudyCalendarPolicy::Exchange);
        let document = chart.export_state_json().unwrap();
        let json: serde_json::Value = serde_json::from_str(&document).unwrap();
        for index in 0..3 {
            assert_eq!(json["indicators"][index]["kind"]["calendar"], "exchange");
        }
        let mut omitted = json.clone();
        for index in 0..3 {
            omitted["indicators"][index]["kind"]
                .as_object_mut()
                .unwrap()
                .remove("calendar");
        }
        for document in [document, omitted.to_string()] {
            let mut restored = ChartEngine::new(800.0, 500.0, 1.0);
            install(&mut restored, &times, &high, &low);
            restored.set_time_zone("Asia/Shanghai").unwrap();
            restored.set_session_start_seconds(-3 * 3_600).unwrap();
            restored.import_state_json(&document).unwrap();
            assert!(
                restored.indicators[..3]
                    .iter()
                    .all(|binding| binding.calendar == Some(StudyCalendarPolicy::Exchange))
            );
            for id in session.iter().chain(&week).chain(&opening).copied() {
                assert_eq!(values(&restored, id), values(&chart, id));
            }
        }
        // On a chart whose exchange time is UTC the new default reads exactly like `utc`.
        let utc_times = [0., 30., 60., 86_400., 7. * 86_400.];
        let utc_high = [12., 15., 14., 20., 30.];
        let utc_low = [8., 7., 9., 16., 25.];
        let mut utc_chart = ChartEngine::new(800.0, 500.0, 1.0);
        install(&mut utc_chart, &utc_times, &utc_high, &utc_low);
        let explicit =
            utc_chart.add_previous_period_levels(0, PreviousPeriod::Week, StudyCalendarPolicy::Utc);
        let mut legacy: serde_json::Value =
            serde_json::from_str(&utc_chart.export_state_json().unwrap()).unwrap();
        assert_eq!(legacy["indicators"][0]["kind"]["calendar"], "utc");
        legacy["indicators"][0]["kind"]
            .as_object_mut()
            .unwrap()
            .remove("calendar");
        let mut restored = ChartEngine::new(800.0, 500.0, 1.0);
        install(&mut restored, &utc_times, &utc_high, &utc_low);
        restored.import_state_json(&legacy.to_string()).unwrap();
        assert_eq!(
            restored.indicators[0].calendar,
            Some(StudyCalendarPolicy::Exchange)
        );
        for id in explicit {
            assert_eq!(values(&restored, id), values(&utc_chart, id));
        }
    }

    #[test]
    fn session_study_schemas_default_to_the_exchange_calendar() {
        for kind in ["session_levels", "previous_period_levels", "opening_range"] {
            let schema = ChartEngine::indicator_schema(
                &IndicatorKind::schema_definition(kind, 14, 2.0).unwrap(),
            );
            let calendar = schema
                .parameters
                .iter()
                .find(|parameter| parameter.name == "calendar")
                .unwrap();
            assert_eq!(calendar.default, serde_json::json!("exchange"), "{kind}");
            assert_eq!(
                calendar.options.as_deref(),
                Some(&["exchange".into(), "utc".into(), "host".into()][..])
            );
        }
    }
}

/// The scalar built-in kinds of [`every_indicator_kind_with_conventions`]: the structure and
/// session studies need calendar- and annotation-aware references.
fn scalar_indicator_kinds() -> Vec<IndicatorKind> {
    let studies = study_indicator_kinds();
    every_indicator_kind_with_conventions()
        .into_iter()
        .filter(|kind| !studies.contains(kind))
        .collect()
}

// Compare installed tick weights, not only the visible subset selected by the time-axis layout.
#[derive(Debug, PartialEq)]
struct AxisSyncSnapshot {
    marks: Vec<(i64, u8)>,
    points_len: usize,
    base_index: i64,
    first: Option<i64>,
    last: Option<i64>,
    labels: Vec<AxisLabel>,
}

fn axis_sync_snapshot(chart: &mut ChartEngine) -> AxisSyncSnapshot {
    let marks = chart
        .tick_marks
        .build(1.0, 0.0)
        .iter()
        .map(|mark| (mark.index, mark.weight))
        .collect();
    let points_len = chart.time_scale.points_len();
    let base_index = chart.time_scale.base_index();
    let first = chart.synced_first_time;
    let last = chart.synced_last_time;
    let labels = chart
        .build_axis_frame(
            80.0,
            |text, _| text.len() as f64 * 7.0,
            |text, _| text.len() as f64 * 6.0,
        )
        .labels
        .into_iter()
        .filter(|label| label.y >= chart.css_height - chart.time_axis_height())
        .collect();
    AxisSyncSnapshot {
        marks,
        points_len,
        base_index,
        first,
        last,
        labels,
    }
}

#[test]
fn irregular_append_reclassifies_first_tick_like_fresh_install() {
    let mut chart = ChartEngine::new(900.0, 400.0, 1.0);
    chart.time_scale.set_width(800.0);
    let initial = [86_280.0, 86_340.0, 86_400.0, 86_460.0];
    chart
        .set_series_data(0, &initial, &initial, &initial, &initial, &initial)
        .unwrap();
    assert!(chart.update_series_bar(0, 172_800.0, [172_800.0; 4]));

    let final_times = [86_280.0, 86_340.0, 86_400.0, 86_460.0, 172_800.0];
    let mut fresh = ChartEngine::new(900.0, 400.0, 1.0);
    fresh.time_scale.set_width(800.0);
    fresh
        .set_series_data(
            0,
            &final_times,
            &final_times,
            &final_times,
            &final_times,
            &final_times,
        )
        .unwrap();
    fresh.set_right_offset(chart.right_offset());
    assert_eq!(installed_tick_weights(&mut fresh), [32, 20, 50, 20, 50]);
    assert_eq!(
        axis_sync_snapshot(&mut chart),
        axis_sync_snapshot(&mut fresh)
    );
}

#[test]
fn capped_batch_append_rebuilds_shifted_tick_weights() {
    let mut chart = ChartEngine::new(900.0, 400.0, 1.0);
    chart.time_scale.set_width(800.0);
    let initial = (0..10)
        .map(|i| 86_280.0 + i as f64 * 61.0)
        .collect::<Vec<_>>();
    chart
        .set_series_data(0, &initial, &initial, &initial, &initial, &initial)
        .unwrap();
    assert!(chart.set_series_max_points(0, Some(64)));
    let rows = (0..60)
        .map(|i| {
            let time = 172_800.0 + i as f64 * 3_600.0;
            (time, [time; 4])
        })
        .collect::<Vec<_>>();
    assert_eq!(chart.update_series_bars(0, rows), 60);
    let retained = chart
        .data
        .merged_times()
        .iter()
        .map(|&t| t as f64)
        .collect::<Vec<_>>();
    assert_eq!(retained.len(), 62);
    let mut fresh = ChartEngine::new(900.0, 400.0, 1.0);
    fresh.time_scale.set_width(800.0);
    fresh
        .set_series_data(0, &retained, &retained, &retained, &retained, &retained)
        .unwrap();
    fresh.set_right_offset(chart.right_offset());
    assert_eq!(
        axis_sync_snapshot(&mut chart),
        axis_sync_snapshot(&mut fresh)
    );
}

#[test]
fn seeded_irregular_axis_mutations_match_fresh_install() {
    // Fixed xorshift seed; spacings span near-duplicates, minute/day boundaries and weeks.
    let mut seed = 0x7ac3_19d2_4e86_502bu64;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let cadences = [1, 2, 59, 60, 61, 299, 300, 3_600, 43_200, 86_400, 604_800];
    for projection in [0, 1, -1] {
        let mut times = vec![86_280.0, 86_340.0, 86_400.0, 86_460.0];
        let mut values = times.clone();
        let mut chart = ChartEngine::new(900.0, 400.0, 1.0);
        chart.time_scale.set_width(800.0);
        if projection > 0 {
            chart.set_future_time_projection(Some(60), 3);
        } else if projection < 0 {
            chart.set_past_time_projection(Some(60), 3);
        }
        chart
            .set_series_data(0, &times, &values, &values, &values, &values)
            .unwrap();
        let compare = |chart: &mut ChartEngine, times: &[f64], values: &[f64], step: &str| {
            let mut fresh = ChartEngine::new(900.0, 400.0, 1.0);
            fresh.time_scale.set_width(800.0);
            if projection > 0 {
                fresh.set_future_time_projection(Some(60), 3);
            } else if projection < 0 {
                fresh.set_past_time_projection(Some(60), 3);
            }
            fresh
                .set_series_data(0, times, values, values, values, values)
                .unwrap();
            fresh.set_right_offset(chart.right_offset());
            assert_eq!(
                axis_sync_snapshot(chart),
                axis_sync_snapshot(&mut fresh),
                "{step}, projection {projection}, times {times:?}"
            );
        };
        compare(&mut chart, &times, &values, "initial");
        for round in 0..40 {
            let cadence = cadences[next() as usize % cadences.len()] as f64;
            let last = times.last().copied().unwrap();
            times.push(last + cadence);
            values.push(last + cadence);
            assert!(chart.update_series_bar(0, last + cadence, [last + cadence; 4]));
            compare(&mut chart, &times, &values, &format!("{round} append"));

            let tip = times.len() - 1;
            values[tip] += (next() % 3 + 1) as f64;
            assert!(chart.update_series_bar(0, times[tip], [values[tip]; 4]));
            compare(&mut chart, &times, &values, &format!("{round} tip"));

            let middle = 1 + next() as usize % (times.len() - 2);
            values[middle] += 1.0;
            assert!(chart.update_series_bar(0, times[middle], [values[middle]; 4]));
            compare(&mut chart, &times, &values, &format!("{round} correction"));

            if let Some((index, window)) = times
                .windows(2)
                .enumerate()
                .find(|(_, pair)| pair[1] - pair[0] >= 2.0)
            {
                let inserted = window[0] + ((window[1] - window[0]) / 2.0).floor();
                times.insert(index + 1, inserted);
                values.insert(index + 1, inserted);
                assert!(chart.update_series_bar(0, inserted, [inserted; 4]));
                compare(&mut chart, &times, &values, &format!("{round} mid insert"));
            }

            assert_eq!(chart.series_pop(0, 1), Some(times.len() - 1));
            times.pop();
            values.pop();
            compare(&mut chart, &times, &values, &format!("{round} pop"));

            if times.len() > 8 {
                let limit = 4 + next() as usize % 5;
                assert!(chart.set_series_max_points(0, Some(limit)));
                times.drain(..times.len() - limit);
                values.drain(..values.len() - limit);
                compare(&mut chart, &times, &values, &format!("{round} trim"));
                assert!(chart.set_series_max_points(0, None));
            }
            // An append batch can grow the retained series while trimming its front
            // in the same transaction, unlike an explicit set_series_max_points trim.
            if round % 8 == 0 && times.len() < 60 {
                assert!(chart.set_series_max_points(0, Some(64)));
                let count = 70 - times.len();
                let mut rows = Vec::with_capacity(count);
                for _ in 0..count {
                    let cadence = cadences[next() as usize % cadences.len()] as f64;
                    let time = times.last().unwrap() + cadence;
                    times.push(time);
                    values.push(time);
                    rows.push((time, [time; 4]));
                }
                assert_eq!(chart.update_series_bars(0, rows), count);
                times.drain(..times.len() - 62);
                values.drain(..values.len() - 62);
                compare(
                    &mut chart,
                    &times,
                    &values,
                    &format!("{round} capped batch"),
                );
                assert!(chart.set_series_max_points(0, None));
            }
        }
    }
}

#[test]
fn irregular_second_series_union_reclassifies_first_tick_on_add_and_remove() {
    let base = [86_280.0, 86_340.0, 86_400.0, 86_460.0];
    let other = [86_280.0, 172_800.0];
    for projection in [0, 1, -1] {
        let mut chart = ChartEngine::new(900.0, 400.0, 1.0);
        chart.time_scale.set_width(800.0);
        if projection > 0 {
            chart.set_future_time_projection(Some(60), 3);
        } else if projection < 0 {
            chart.set_past_time_projection(Some(60), 3);
        }
        chart
            .set_series_data(0, &base, &base, &base, &base, &base)
            .unwrap();
        let second = chart.add_series(SeriesKind::Line);
        chart
            .set_series_data(second, &other, &other, &other, &other, &other)
            .unwrap();
        let mut fresh = ChartEngine::new(900.0, 400.0, 1.0);
        fresh.time_scale.set_width(800.0);
        if projection > 0 {
            fresh.set_future_time_projection(Some(60), 3);
        } else if projection < 0 {
            fresh.set_past_time_projection(Some(60), 3);
        }
        fresh
            .set_series_data(0, &base, &base, &base, &base, &base)
            .unwrap();
        let fresh_second = fresh.add_series(SeriesKind::Line);
        fresh
            .set_series_data(fresh_second, &other, &other, &other, &other, &other)
            .unwrap();
        fresh.set_right_offset(chart.right_offset());
        assert_eq!(
            axis_sync_snapshot(&mut chart),
            axis_sync_snapshot(&mut fresh),
            "add {projection}"
        );
        assert!(chart.remove_series(second));
        assert!(fresh.remove_series(fresh_second));
        assert_eq!(
            axis_sync_snapshot(&mut chart),
            axis_sync_snapshot(&mut fresh),
            "remove {projection}"
        );
    }
}

#[test]
fn incremental_axis_matches_fresh_install_across_mutations_and_projections() {
    let base = [86_160.0, 86_220.0, 86_280.0, 86_340.0];
    let mut times = base.to_vec();
    let mut values = vec![10.0, 11.0, 12.0, 13.0];
    for projection in [0, 1, -1] {
        let mut chart = ChartEngine::new(900.0, 400.0, 1.0);
        chart.time_scale.set_width(800.0);
        if projection > 0 {
            chart.set_future_time_projection(Some(60), 4);
        } else if projection < 0 {
            chart.set_past_time_projection(Some(60), 4);
        }
        chart
            .set_series_data(0, &times, &values, &values, &values, &values)
            .unwrap();
        let compare = |chart: &mut ChartEngine, times: &[f64], values: &[f64], case: &str| {
            let mut fresh = ChartEngine::new(900.0, 400.0, 1.0);
            fresh.time_scale.set_width(800.0);
            if projection > 0 {
                fresh.set_future_time_projection(Some(60), 4);
            } else if projection < 0 {
                fresh.set_past_time_projection(Some(60), 4);
            }
            fresh
                .set_series_data(0, times, values, values, values, values)
                .unwrap();
            fresh.set_right_offset(chart.right_offset());
            assert_eq!(
                axis_sync_snapshot(chart),
                axis_sync_snapshot(&mut fresh),
                "{case}, projection {projection}"
            );
            assert_eq!(chart.data.merged_times().len(), times.len());
            for &time in times {
                let x = chart.time_to_coordinate(time).unwrap();
                assert_eq!(chart.coordinate_to_time(x), Some(time));
                let logical = chart.time_to_index(time, false).unwrap() as f64;
                let x = chart.logical_to_coordinate(logical).unwrap();
                assert_eq!(chart.coordinate_to_time(x), Some(time));
            }
            for logical in -4..(times.len() as i64 + 4) {
                let label_time = chart.axis_time_key_at_logical(logical);
                assert_eq!(
                    label_time,
                    fresh.axis_time_key_at_logical(logical),
                    "{case}: projected label at {logical}"
                );
                if let Some(time) = label_time {
                    let x = chart.logical_to_coordinate(logical as f64).unwrap();
                    assert_eq!(chart.coordinate_to_logical(x), Some(logical as f64));
                    assert_eq!(chart.coordinate_to_time(x), Some(time as f64));
                    if logical < 0 || logical >= times.len() as i64 {
                        assert_eq!(chart.time_to_index(time as f64, false), None);
                    }
                }
            }
        };
        compare(&mut chart, &times, &values, "initial");
        values[3] = 14.0;
        assert!(chart.update_series_bar(0, times[3], [14.0; 4]));
        compare(&mut chart, &times, &values, "tip replacement");
        times.push(86_400.0);
        values.push(15.0);
        assert!(chart.update_series_bar(0, 86_400.0, [15.0; 4]));
        compare(&mut chart, &times, &values, "day-boundary append");
        values[1] = 16.0;
        assert!(chart.update_series_bar(0, times[1], [16.0; 4]));
        compare(&mut chart, &times, &values, "mid correction");
        times.insert(3, 86_310.0);
        values.insert(3, 17.0);
        assert!(chart.update_series_bar(0, 86_310.0, [17.0; 4]));
        compare(&mut chart, &times, &values, "mid insert");
        times.push(86_460.0);
        values.push(f64::NAN);
        assert!(chart.update_series_bar(0, 86_460.0, [f64::NAN; 4]));
        // NaN is whitespace in the source; compare with a separately installed blank row.
        // The equality helper compares the stored source state and the derived axis.
        compare(&mut chart, &times, &values, "trailing whitespace");
        *values.last_mut().unwrap() = 18.0;
        assert!(chart.update_series_bar(0, 86_460.0, [18.0; 4]));
        compare(&mut chart, &times, &values, "whitespace replaced");
        assert_eq!(chart.series_pop(0, 1), Some(times.len() - 1));
        times.pop();
        values.pop();
        compare(&mut chart, &times, &values, "pop");
        assert!(chart.set_series_max_points(0, Some(4)));
        times.drain(..times.len() - 4);
        values.drain(..values.len() - 4);
        compare(&mut chart, &times, &values, "retention trim");
        times = base.to_vec();
        values = vec![10.0, 11.0, 12.0, 13.0];
    }
}

#[test]
fn axis_sync_tracks_second_series_indicator_and_projection_revision() {
    let base = [86_160.0, 86_220.0, 86_280.0, 86_340.0];
    let other = [86_220.0, 86_250.0, 86_400.0];
    for projection in [0, 1, -1] {
        let mut chart = ChartEngine::new(900.0, 400.0, 1.0);
        chart.time_scale.set_width(800.0);
        if projection == 1 {
            chart.set_future_time_projection(Some(60), 4);
        } else if projection == -1 {
            chart.set_past_time_projection(Some(60), 4);
        }
        chart
            .set_series_data(0, &base, &base, &base, &base, &base)
            .unwrap();
        let second = chart.add_series(SeriesKind::Line);
        chart
            .set_series_data(second, &other, &other, &other, &other, &other)
            .unwrap();
        let mut fresh = ChartEngine::new(900.0, 400.0, 1.0);
        fresh.time_scale.set_width(800.0);
        if projection == 1 {
            fresh.set_future_time_projection(Some(60), 4);
        } else if projection == -1 {
            fresh.set_past_time_projection(Some(60), 4);
        }
        fresh
            .set_series_data(0, &base, &base, &base, &base, &base)
            .unwrap();
        let fresh_second = fresh.add_series(SeriesKind::Line);
        fresh
            .set_series_data(fresh_second, &other, &other, &other, &other, &other)
            .unwrap();
        fresh.set_right_offset(chart.right_offset());
        assert_eq!(
            axis_sync_snapshot(&mut chart),
            axis_sync_snapshot(&mut fresh),
            "second series added with projection {projection}"
        );
        let reinstall = |second_values: Option<Vec<f64>>| {
            let mut installed = ChartEngine::new(900.0, 400.0, 1.0);
            installed.time_scale.set_width(800.0);
            if projection == 1 {
                installed.set_future_time_projection(Some(60), 4);
            } else if projection == -1 {
                installed.set_past_time_projection(Some(60), 4);
            }
            installed
                .set_series_data(0, &base, &base, &base, &base, &base)
                .unwrap();
            if let Some(values) = second_values {
                let series = installed.add_series(SeriesKind::Line);
                installed
                    .set_series_data(series, &other, &values, &values, &values, &values)
                    .unwrap();
            }
            installed
        };
        let mut installed = reinstall(Some(other.to_vec()));
        installed.set_right_offset(chart.right_offset());
        assert_eq!(
            axis_sync_snapshot(&mut chart),
            axis_sync_snapshot(&mut installed)
        );
        let generation = chart.synced_time_points_generation;
        assert!(chart.update_series_bar(second, 86_400.0, [86_401.0; 4]));
        assert_eq!(chart.synced_time_points_generation, generation);
        assert!(fresh.update_series_bar(fresh_second, 86_400.0, [86_401.0; 4]));
        assert_eq!(
            axis_sync_snapshot(&mut chart),
            axis_sync_snapshot(&mut fresh),
            "data-only tip with unchanged projection {projection}"
        );
        let mut changed_other = other.to_vec();
        changed_other[2] = 86_401.0;
        let mut installed = reinstall(Some(changed_other));
        installed.set_right_offset(chart.right_offset());
        assert_eq!(
            axis_sync_snapshot(&mut chart),
            axis_sync_snapshot(&mut installed)
        );
        assert!(chart.remove_series(second));
        assert!(fresh.remove_series(fresh_second));
        assert_eq!(
            axis_sync_snapshot(&mut chart),
            axis_sync_snapshot(&mut fresh),
            "second series removed with projection {projection}"
        );
        let mut installed = reinstall(None);
        installed.set_right_offset(chart.right_offset());
        assert_eq!(
            axis_sync_snapshot(&mut chart),
            axis_sync_snapshot(&mut installed)
        );
        // A dependent output has a separate synchronization after source mutation. Its
        // trimming and base-index moves must leave the same axis as a fresh install.
        let output = chart.add_sma(0, 2).unwrap();
        assert!(chart.update_series_bar(0, 86_400.0, [86_400.0; 4]));
        let fresh_output = fresh.add_sma(0, 2).unwrap();
        assert!(fresh.update_series_bar(0, 86_400.0, [86_400.0; 4]));
        assert_eq!(
            axis_sync_snapshot(&mut chart),
            axis_sync_snapshot(&mut fresh),
            "indicator propagation {projection}"
        );
        assert!(chart.remove_series(output));
        assert!(fresh.remove_series(fresh_output));
        if projection == 1 {
            assert!(chart.set_future_time_projection(Some(120), 3));
            assert!(fresh.set_future_time_projection(Some(120), 3));
            assert!(chart.update_series_bar(0, 86_400.0, [86_401.0; 4]));
            assert!(fresh.update_series_bar(0, 86_400.0, [86_401.0; 4]));
            assert_eq!(
                axis_sync_snapshot(&mut chart),
                axis_sync_snapshot(&mut fresh)
            );
            assert!(chart.set_future_time_projection(None, 0));
            assert!(fresh.set_future_time_projection(None, 0));
        } else if projection == -1 {
            assert!(chart.set_past_time_projection(Some(120), 3));
            assert!(fresh.set_past_time_projection(Some(120), 3));
            assert!(chart.update_series_bar(0, 86_400.0, [86_401.0; 4]));
            assert!(fresh.update_series_bar(0, 86_400.0, [86_401.0; 4]));
            assert_eq!(
                axis_sync_snapshot(&mut chart),
                axis_sync_snapshot(&mut fresh)
            );
            assert!(chart.set_past_time_projection(None, 0));
            assert!(fresh.set_past_time_projection(None, 0));
        }
        assert!(chart.update_series_bar(0, 86_400.0, [86_402.0; 4]));
        assert!(fresh.update_series_bar(0, 86_400.0, [86_402.0; 4]));
        assert_eq!(
            axis_sync_snapshot(&mut chart),
            axis_sync_snapshot(&mut fresh),
            "projection cleared {projection}"
        );
    }
}

#[test]
fn sequence_axis_round_trip_and_clear_match_fresh_install() {
    let points = |middle| {
        [86_160, middle, 86_400]
            .into_iter()
            .enumerate()
            .map(|(index, time)| BarSequencePoint {
                logical_index: index as u64,
                open_timestamp_micros: time * 1_000_000,
                close_timestamp_micros: (time + 30) * 1_000_000,
            })
            .collect::<Vec<_>>()
    };
    let install = |chart: &mut ChartEngine, middle| {
        let values = vec![10.0, 11.0, 12.0];
        // The fork keys the projection from an explicit row-key base (here the first key).
        assert!(chart.install_trade_bar_sequence_projection(
            0,
            0,
            (
                points(middle),
                values.clone(),
                values.clone(),
                values.clone(),
                values
            )
        ));
    };
    let mut chart = ChartEngine::new(900.0, 400.0, 1.0);
    chart.time_scale.set_width(800.0);
    install(&mut chart, 86_220);
    install(&mut chart, 86_280);
    let mut fresh = ChartEngine::new(900.0, 400.0, 1.0);
    fresh.time_scale.set_width(800.0);
    install(&mut fresh, 86_280);
    assert_eq!(
        axis_sync_snapshot(&mut chart),
        axis_sync_snapshot(&mut fresh)
    );
    for time in [86_160.0, 86_280.0, 86_400.0] {
        let x = chart.time_to_coordinate(time).unwrap();
        assert_eq!(chart.coordinate_to_time(x), Some(time));
        let index = chart.time_to_index(time, false).unwrap();
        assert_eq!(
            chart.coordinate_to_time(chart.logical_to_coordinate(index as f64).unwrap()),
            Some(time)
        );
    }
    chart.clear_sequence_axis_if_unused();
    fresh.clear_sequence_axis_if_unused();
    assert!(chart.sequence_points().is_none());
    assert_eq!(
        axis_sync_snapshot(&mut chart),
        axis_sync_snapshot(&mut fresh)
    );
    assert_eq!(chart.synced_first_time, Some(0));
    assert_eq!(chart.synced_last_time, Some(2));
}

#[test]
fn seeded_sequence_axis_appends_reweigh_the_first_tick_like_fresh_install() {
    // Trade bars (one trade each) on a non-time sequence axis: several bars may open in the same
    // second, and day or week jumps move the average cadence, so the first point's weight changes
    // on append and must be re-inferred exactly as a fresh install infers it.
    let trade = |timestamp_micros: i64| FootprintTrade {
        timestamp_micros,
        price: 100.0 + (timestamp_micros / 1_000_000 % 5) as f64,
        volume: 1.0,
        aggressor: AggressorSide::Buy,
        bid: None,
        ask: None,
        sequence: None,
        trade_id: None,
        conditions: 0,
        session_id: None,
    };
    let chart_with = |trades: Vec<FootprintTrade>| {
        let mut chart = ChartEngine::new(900.0, 400.0, 1.0);
        chart.time_scale.set_width(800.0);
        let id = chart
            .add_footprint_series(FootprintSeriesOptions {
                aggregation: FootprintAggregationOptions {
                    tick_size: 1.0,
                    bars: FootprintBarAggregation::Trades { trades_per_bar: 1 },
                    ..FootprintAggregationOptions::default()
                },
                ..FootprintSeriesOptions::default()
            })
            .unwrap();
        chart.set_footprint_trades(id, trades).unwrap();
        chart.set_bar_spacing(16.0);
        (chart, id)
    };
    // Fixed xorshift seed; steps in microseconds: same second, one second, minute and hour
    // boundaries, a day and a week.
    let mut seed = 0x51e9_c0de_2026_a7b3u64;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let steps = [
        1,
        250_000,
        1_000_000,
        59_000_000,
        3_600_000_000,
        86_400_000_000,
    ];
    let mut times = (0..4)
        .map(|i| 86_340_000_000 + i * 1_000_000)
        .collect::<Vec<i64>>();
    let (mut chart, id) = chart_with(times.iter().copied().map(trade).collect());
    let mut first_weights = std::collections::BTreeSet::new();
    for round in 0..40 {
        let step = if round == 20 {
            604_800_000_000
        } else {
            steps[next() as usize % steps.len()]
        };
        let time = times.last().unwrap() + step;
        times.push(time);
        chart.update_footprint_trade(id, trade(time)).unwrap();
        assert_eq!(chart.sequence_points().unwrap().len(), times.len());
        let (mut fresh, _) = chart_with(times.iter().copied().map(trade).collect());
        fresh.set_right_offset(chart.right_offset());
        let appended = axis_sync_snapshot(&mut chart);
        first_weights.insert(appended.marks.iter().find(|mark| mark.0 == 0).map(|m| m.1));
        assert_eq!(
            appended,
            axis_sync_snapshot(&mut fresh),
            "round {round}, step {step} us"
        );
    }
    assert!(
        first_weights.len() > 1,
        "the appends moved the first weight: {first_weights:?}"
    );
}

/// [`every_indicator_kind_with_conventions`] plus the KLineChart templates: every fork binding
/// kind a chart can attach to one OHLC source. AVP reads turnover as its scalar source series;
/// its gap rules are covered in `klinechart_indicators`.
fn every_indicator_kind_with_klinechart_templates() -> Vec<IndicatorKind> {
    let mut kinds = every_indicator_kind_with_conventions();
    kinds.extend(
        aeris_charts_indicators::klinechart::NAMES
            .into_iter()
            .filter(|&name| name != "AVP")
            .map(|name| {
                IndicatorKind::KLineChart(
                    aeris_charts_indicators::klinechart::Indicator::from_name(name).unwrap(),
                )
            }),
    );
    kinds
}

#[test]
fn every_indicator_engine_path_matches_fresh_engine_on_gap_mutations() {
    let kinds = every_indicator_kind_with_klinechart_templates();
    // Every built-in kind with its seed and convention variants (both Envelopes modes, both KDJ
    // seeds), the structure and session studies, and 26 of the 27 KLineChart templates.
    assert_eq!(kinds.len(), 102, "update the fork kind list count");
    // Rebuild each kind in a *different* engine, not merely via the dense
    // formula used by assert_indicator_binding_matches_full.
    for kind in kinds {
        let mut times = (0..1050)
            .map(|row| row as f64 * 3_600.0)
            .collect::<Vec<_>>();
        let mut close = (0..1050)
            .map(|row| 90.0 + row as f64 * 0.05 + (row as f64 * 0.37).sin() * 2.0)
            .collect::<Vec<_>>();
        for row in [30, 75, 76, 1023, 1024, 1025] {
            close[row] = f64::NAN;
        }
        let ohlc = |value: f64| {
            if value.is_finite() {
                [value - 0.35, value + 1.4, value - 1.2, value]
            } else {
                [f64::NAN; 4]
            }
        };
        let open = close.iter().map(|&v| ohlc(v)[0]).collect::<Vec<_>>();
        let high = close.iter().map(|&v| ohlc(v)[1]).collect::<Vec<_>>();
        let low = close.iter().map(|&v| ohlc(v)[2]).collect::<Vec<_>>();
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        let volume = chart.add_series(SeriesKind::Histogram);
        let volumes = (0..1053)
            .map(|row| (row % 7 + 1) as f64)
            .collect::<Vec<_>>();
        chart
            .set_series_data(0, &times, &open, &high, &low, &close)
            .unwrap();
        chart
            .set_series_data(
                volume,
                &times,
                &volumes[..times.len()],
                &volumes[..times.len()],
                &volumes[..times.len()],
                &volumes[..times.len()],
            )
            .unwrap();
        let outputs = add_test_indicator(&mut chart, &kind, Some(volume));
        assert!(!outputs.is_empty(), "{kind:?}");
        let verify = |chart: &ChartEngine, times: &[f64], close: &[f64], stage: &str| {
            let open = close.iter().map(|&v| ohlc(v)[0]).collect::<Vec<_>>();
            let high = close.iter().map(|&v| ohlc(v)[1]).collect::<Vec<_>>();
            let low = close.iter().map(|&v| ohlc(v)[2]).collect::<Vec<_>>();
            let mut fresh = ChartEngine::new(800.0, 500.0, 1.0);
            let fresh_volume = fresh.add_series(SeriesKind::Histogram);
            fresh
                .set_series_data(0, times, &open, &high, &low, close)
                .unwrap();
            fresh
                .set_series_data(
                    fresh_volume,
                    times,
                    &volumes[..times.len()],
                    &volumes[..times.len()],
                    &volumes[..times.len()],
                    &volumes[..times.len()],
                )
                .unwrap();
            let expected = add_test_indicator(&mut fresh, &kind, Some(fresh_volume));
            assert_eq!(outputs.len(), expected.len(), "{kind:?} {stage}");
            for (&id, &reference) in outputs.iter().zip(&expected) {
                let (actual_times, actual) = chart.data.series_data(id).unwrap();
                let (expected_times, expected_values) = fresh.data.series_data(reference).unwrap();
                if matches!(kind, IndicatorKind::MassIndex { .. }) {
                    assert!(
                        actual[3].iter().any(|v| v.is_finite()),
                        "MassIndex must have nonempty range output at {stage}"
                    );
                }
                assert_eq!(actual_times, expected_times, "{kind:?} {stage}");
                assert_eq!(
                    actual[3].len(),
                    expected_values[3].len(),
                    "{kind:?} {stage}"
                );
                for (row, (&a, &b)) in actual[3].iter().zip(expected_values[3]).enumerate() {
                    assert!(
                        (a.is_nan() && b.is_nan()) || (a - b).abs() <= 1e-9 * b.abs().max(1.0),
                        "{kind:?} {stage} output {id} row {row}: {a:?} != {b:?}"
                    );
                }
            }
        };
        verify(&chart, &times, &close, "initial");
        for (row, value) in [
            (1050, 139.0),    // one-row append
            (1051, f64::NAN), // append a gap
            (1052, 140.0),    // cross the gap one row at a time
        ] {
            times.push(row as f64 * 3_600.0);
            close.push(value);
            assert!(
                chart.update_series_bar(0, times[row], ohlc(value)),
                "{kind:?} append row {row}"
            );
            assert!(
                chart.update_series_bar(volume, times[row], [volumes[row]; 4]),
                "{kind:?} volume append row {row}"
            );
            verify(&chart, &times, &close, "single append");
        }
        for (row, value, stage) in [
            (1052, f64::NAN, "tip becomes whitespace"),
            (1052, 140.0, "tip filled"),
            (1030, f64::NAN, "historical gap created"),
            (1030, 145.0, "historical gap filled"),
            (1024, 143.0, "checkpoint gap filled"),
            (1024, f64::NAN, "checkpoint gap restored"),
        ] {
            close[row] = value;
            assert!(
                chart.update_series_bar(0, times[row], ohlc(value)),
                "{kind:?} {stage}"
            );
            verify(&chart, &times, &close, stage);
        }
    }
}

#[test]
fn every_scalar_indicator_trims_leading_whitespace_and_repairs_before_its_start() {
    for kind in scalar_indicator_kinds() {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        let volume = chart.add_series(SeriesKind::Histogram);
        let times = (0..80).map(|i| i as f64 * 60.0).collect::<Vec<_>>();
        let mut values = (0..80).map(|i| 100.0 + i as f64).collect::<Vec<_>>();
        let volumes = (0..80).map(|i| 10.0 + i as f64).collect::<Vec<_>>();
        values[..3].fill(f64::NAN);
        values[35] = f64::NAN;
        chart
            .set_series_data(0, &times, &values, &values, &values, &values)
            .unwrap();
        chart
            .set_series_data(volume, &times, &volumes, &volumes, &volumes, &volumes)
            .unwrap();
        let outputs = add_test_indicator(&mut chart, &kind, Some(volume));
        let binding = chart.indicators.len() - 1;
        let mut without_leading = ChartEngine::new(800.0, 500.0, 1.0);
        let volume_without_leading = without_leading.add_series(SeriesKind::Histogram);
        without_leading
            .set_series_data(
                0,
                &times[3..],
                &values[3..],
                &values[3..],
                &values[3..],
                &values[3..],
            )
            .unwrap();
        without_leading
            .set_series_data(
                volume_without_leading,
                &times[3..],
                &volumes[3..],
                &volumes[3..],
                &volumes[3..],
                &volumes[3..],
            )
            .unwrap();
        let oracle = add_test_indicator(&mut without_leading, &kind, Some(volume_without_leading));
        for (&actual, &expected) in outputs.iter().zip(&oracle) {
            let (actual_times, actual_values) = chart.data.series_data(actual).unwrap();
            let (expected_times, expected_values) =
                without_leading.data.series_data(expected).unwrap();
            assert_eq!(actual_times, expected_times, "{kind:?}");
            for (&a, &b) in actual_values[3].iter().zip(expected_values[3]) {
                assert!(
                    (a.is_nan() && b.is_nan()) || (a - b).abs() < 1e-9,
                    "{kind:?}: {a} != {b}"
                );
            }
        }
        let check = |chart: &ChartEngine| {
            assert_indicator_binding_matches_full(chart, binding);
            for &id in &outputs {
                let (_, columns) = chart.data.series_data(id).unwrap();
                assert!(columns[3].first().is_none_or(|v| !v.is_nan()), "{kind:?}");
            }
        };
        check(&chart);
        for (row, value) in [(0, 101.0), (1, 102.0), (3, f64::NAN), (4, 105.0)] {
            assert!(
                chart.update_series_bar(0, times[row], [value; 4]),
                "{kind:?}"
            );
            check(&chart);
        }
    }
}

#[test]
fn structure_anchors_keep_every_source_time_when_scalar_outputs_trim() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    let times = (0..40).map(|i| i as f64 * 60.0).collect::<Vec<_>>();
    let mut values = (0..40).map(|i| 100.0 + i as f64).collect::<Vec<_>>();
    values[..3].fill(f64::NAN);
    values[20] = f64::NAN;
    chart
        .set_series_data(0, &times, &values, &values, &values, &values)
        .unwrap();
    for name in ["market_structure", "fair_value_gaps", "order_blocks"] {
        let kind = IndicatorKind::schema_definition(name, 14, 2.0).unwrap();
        let output = chart.add_indicator_kind(0, kind.clone(), None)[0];
        let (anchor_times, columns) = chart.data.series_data(output).unwrap();
        assert_eq!(
            anchor_times,
            times.iter().map(|t| *t as i64).collect::<Vec<_>>(),
            "{kind:?}"
        );
        assert!(columns[3].iter().all(|value| value.is_nan()));
        assert!(chart.update_series_bar(0, times[0], [99.0; 4]));
        assert_eq!(chart.data.series_data(output).unwrap().0.len(), times.len());
    }
}

#[test]
fn swing_and_session_outputs_trim_like_a_source_without_leading_gaps() {
    let times = (0..100).map(|i| i as f64 * 1_800.0).collect::<Vec<_>>();
    let mut values = (0..100)
        .map(|i| 100.0 + i as f64 * 0.02 + (i as f64 * 0.9).sin() * 3.0)
        .collect::<Vec<_>>();
    values[..4].fill(f64::NAN);
    values[27] = f64::NAN;
    let kinds = [
        IndicatorKind::SwingPoints { left: 1, right: 1 },
        IndicatorKind::SessionLevels {
            calendar: StudyCalendarPolicy::Utc,
        },
        IndicatorKind::PreviousPeriodLevels {
            period: PreviousPeriod::Day,
            calendar: StudyCalendarPolicy::Utc,
        },
        IndicatorKind::OpeningRange {
            duration_seconds: 3_600,
            calendar: StudyCalendarPolicy::Utc,
        },
    ];
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    let mut fresh = ChartEngine::new(800.0, 500.0, 1.0);
    chart
        .set_series_data(0, &times, &values, &values, &values, &values)
        .unwrap();
    fresh
        .set_series_data(
            0,
            &times[4..],
            &values[4..],
            &values[4..],
            &values[4..],
            &values[4..],
        )
        .unwrap();
    for kind in kinds {
        let outputs = chart.add_indicator_kind(0, kind.clone(), None);
        let oracle = fresh.add_indicator_kind(0, kind.clone(), None);
        for (&output, &expected) in outputs.iter().zip(oracle.iter()) {
            let (actual_times, actual_values) = chart.data.series_data(output).unwrap();
            let (expected_times, expected_values) = fresh.data.series_data(expected).unwrap();
            assert_eq!(actual_times, expected_times, "{kind:?}");
            assert!(actual_values[3].first().is_none_or(|value| !value.is_nan()));
            for (&a, &b) in actual_values[3].iter().zip(expected_values[3]) {
                assert!(
                    (a.is_nan() && b.is_nan()) || (a - b).abs() < 1e-9,
                    "{kind:?}"
                );
            }
        }
    }
}

#[test]
fn built_in_and_custom_studies_share_trim_gap_and_chained_repair_semantics() {
    struct Formula(&'static str);
    impl CustomStudyRuntime for Formula {
        fn compute(
            &mut self,
            input: CustomStudyInput<'_>,
            out: &mut [Vec<f64>],
        ) -> Result<(), CustomStudyFault> {
            const PERIOD: usize = 3;
            for row in input.from..input.times.len() {
                let valid = input.close[..=row]
                    .iter()
                    .copied()
                    .filter(|value| value.is_finite())
                    .collect::<Vec<_>>();
                let value = match self.0 {
                    // The fork's window rule: the SMA averages the last PERIOD valid samples.
                    "sma" if input.close[row].is_finite() && valid.len() >= PERIOD => {
                        valid[valid.len() - PERIOD..].iter().sum::<f64>() / PERIOD as f64
                    }
                    "ema" if input.close[row].is_finite() && valid.len() >= PERIOD => {
                        let mut average = valid[..PERIOD].iter().sum::<f64>() / PERIOD as f64;
                        for &price in &valid[PERIOD..] {
                            average += (price - average) * (2.0 / (PERIOD as f64 + 1.0));
                        }
                        average
                    }
                    "rsi" if input.close[row].is_finite() && valid.len() > PERIOD => {
                        let changes = valid
                            .windows(2)
                            .map(|pair| pair[1] - pair[0])
                            .collect::<Vec<_>>();
                        let mut gain = changes[..PERIOD]
                            .iter()
                            .map(|change| change.max(0.0))
                            .sum::<f64>()
                            / PERIOD as f64;
                        let mut loss = changes[..PERIOD]
                            .iter()
                            .map(|change| (-change).max(0.0))
                            .sum::<f64>()
                            / PERIOD as f64;
                        for &change in &changes[PERIOD..] {
                            gain = (gain * (PERIOD - 1) as f64 + change.max(0.0)) / PERIOD as f64;
                            loss =
                                (loss * (PERIOD - 1) as f64 + (-change).max(0.0)) / PERIOD as f64;
                        }
                        if loss == 0.0 {
                            if gain == 0.0 { 50.0 } else { 100.0 }
                        } else {
                            100.0 - 100.0 / (1.0 + gain / loss)
                        }
                    }
                    _ => f64::NAN,
                };
                out[0].push(value);
            }
            Ok(())
        }
    }

    fn build(times: &[f64], values: &[f64]) -> (ChartEngine, Vec<SeriesId>) {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        for formula in ["sma", "ema", "rsi"] {
            chart
                .register_custom_study(
                    CustomStudyDefinition {
                        type_id: format!("trim.{formula}"),
                        version: 1,
                        title: formula.into(),
                        parameters: vec![],
                        outputs: vec![CustomStudyOutput {
                            name: formula.into(),
                            plot: CustomStudyPlot::Line,
                            pane: CustomStudyPane::Price,
                            default_style: IndicatorOutputStyle::default(),
                        }],
                        uses_volume: false,
                    },
                    Box::new(move |_| Ok(Box::new(Formula(formula)))),
                )
                .unwrap();
        }
        chart
            .set_series_data(0, times, values, values, values, values)
            .unwrap();
        let built = [
            chart.add_sma(0, 3).unwrap(),
            chart.add_ema(0, 3).unwrap(),
            chart.add_rsi(0, 3).unwrap(),
        ];
        let custom = ["sma", "ema", "rsi"].map(|name| {
            chart
                .add_custom_study(
                    &format!("trim.{name}"),
                    0,
                    IndicatorInputSource::Close,
                    None,
                    CustomStudyParams::new(),
                )
                .unwrap()[0]
        });
        let mut outputs = built.into_iter().chain(custom).collect::<Vec<_>>();
        for (upstream, downstream) in [(1, 2), (2, 0), (0, 1)] {
            let mut chains = Vec::new();
            for source in [built[upstream], custom[upstream]] {
                chains.push(match downstream {
                    0 => chart.add_sma(source, 3).unwrap(),
                    1 => chart.add_ema(source, 3).unwrap(),
                    _ => chart.add_rsi(source, 3).unwrap(),
                });
                chains.push(
                    chart
                        .add_custom_study(
                            &format!("trim.{}", ["sma", "ema", "rsi"][downstream]),
                            source,
                            IndicatorInputSource::Close,
                            None,
                            CustomStudyParams::new(),
                        )
                        .unwrap()[0],
                );
            }
            outputs.extend(chains);
        }
        (chart, outputs)
    }

    fn verify(chart: &ChartEngine, outputs: &[SeriesId]) {
        let (times, columns) = chart.data.series_data(0).unwrap();
        let input_times = times.iter().map(|&time| time as f64).collect::<Vec<_>>();
        let (fresh, fresh_outputs) = build(&input_times, columns[3]);
        for (&output, &reference) in outputs.iter().zip(&fresh_outputs) {
            let (actual_times, actual_values) = chart.data.series_data(output).unwrap();
            let (fresh_times, fresh_values) = fresh.data.series_data(reference).unwrap();
            assert_eq!(actual_times, fresh_times, "stale times on {output}");
            assert_eq!(actual_values[3].len(), fresh_values[3].len());
            assert!(actual_values[3].first().is_none_or(|value| !value.is_nan()));
            for (&actual, &expected) in actual_values[3].iter().zip(fresh_values[3]) {
                assert!(
                    (actual.is_nan() && expected.is_nan()) || (actual - expected).abs() < 1e-9,
                    "{output}: {actual} != {expected}"
                );
            }
        }
        for i in 0..3 {
            let (built_times, built_values) = chart.data.series_data(outputs[i]).unwrap();
            let (custom_times, custom_values) = chart.data.series_data(outputs[i + 3]).unwrap();
            assert_eq!(built_times, custom_times);
            for (&a, &b) in built_values[3].iter().zip(custom_values[3]) {
                assert!((a.is_nan() && b.is_nan()) || (a - b).abs() < 1e-9);
            }
        }
        for chains in outputs[6..].as_chunks::<4>().0 {
            let (reference_times, reference_values) = chart.data.series_data(chains[0]).unwrap();
            assert!(!reference_times.is_empty(), "chain unexpectedly blank");
            for &id in &chains[1..] {
                let (times, values) = chart.data.series_data(id).unwrap();
                assert_eq!(times, reference_times, "chain {id} times");
                for (&a, &b) in values[3].iter().zip(reference_values[3]) {
                    assert!((a.is_nan() && b.is_nan()) || (a - b).abs() < 1e-9);
                }
            }
        }
    }

    let times = (0..60).map(|i| i as f64 * 60.0).collect::<Vec<_>>();
    let mut values = (0..60)
        .map(|i| 100.0 + i as f64 * 0.2 + (i as f64 * 0.53).sin())
        .collect::<Vec<_>>();
    values[..3].fill(f64::NAN);
    values[23] = f64::NAN;
    let (mut chart, outputs) = build(&times, &values);
    verify(&chart, &outputs);
    assert!(chart.update_series_bar(0, 3600.0, [125.0; 4]));
    verify(&chart, &outputs);
    assert!(chart.update_series_bar(0, 3600.0, [126.0; 4]));
    verify(&chart, &outputs);
    for (row, value) in [(0, 100.0), (2, 102.0), (3, f64::NAN), (3, 103.0)] {
        assert!(chart.update_series_bar(0, times[row], [value; 4]));
        verify(&chart, &outputs);
    }
    chart
        .set_series_data(0, &times, &values, &values, &values, &values)
        .unwrap();
    verify(&chart, &outputs);
    assert!(chart.set_series_max_points(0, Some(30)));
    verify(&chart, &outputs);
}

#[test]
fn every_scalar_engine_binding_keeps_gap_rows_blank_after_repair() {
    for kind in scalar_indicator_kinds() {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        let volume_id = chart.add_series(SeriesKind::Histogram);
        let n = 1100;
        let times = (0..n).map(|row| row as f64 * 3_600.0).collect::<Vec<_>>();
        let mut close = (0..n)
            .map(|row| 100.0 + row as f64 * 0.04 + (row as f64 * 0.43).sin() * 2.0)
            .collect::<Vec<_>>();
        let mut high = close.iter().map(|v| v + 1.0).collect::<Vec<_>>();
        let mut low = close.iter().map(|v| v - 1.0).collect::<Vec<_>>();
        let volumes = (0..n).map(|row| (row % 11 + 1) as f64).collect::<Vec<_>>();
        for row in (70..72).chain(1023..1026) {
            close[row] = f64::NAN;
            high[row] = f64::NAN;
            low[row] = f64::NAN;
        }
        chart
            .set_series_data(0, &times, &close, &high, &low, &close)
            .unwrap();
        chart
            .set_series_data(volume_id, &times, &volumes, &volumes, &volumes, &volumes)
            .unwrap();
        let outputs = add_test_indicator(&mut chart, &kind, Some(volume_id));
        assert!(!outputs.is_empty(), "{kind:?}");
        let binding = chart.indicators.len() - 1;
        let check = |chart: &ChartEngine, gaps: &[usize]| {
            assert_indicator_binding_matches_full(chart, binding);
            for &output in &outputs {
                let (output_times, values) = chart.data.series_data(output).unwrap();
                for &row in gaps {
                    let pos = output_times
                        .iter()
                        .position(|&time| time == times[row] as i64)
                        .expect("interior gap time remains aligned");
                    assert!(values[3][pos].is_nan(), "{kind:?} gap row {row}");
                }
            }
        };
        check(&chart, &[70, 71, 1023, 1024, 1025]);
        chart.update_series_bar(0, times[1024], [149.0, 150.0, 148.0, 149.0]);
        check(&chart, &[70, 71, 1023, 1025]);
        chart.update_series_bar(0, times[1010], [f64::NAN; 4]);
        check(&chart, &[70, 71, 1010, 1023, 1025]);
        chart.update_series_bar(0, times[1010], [148.0, 149.0, 147.0, 148.0]);
        check(&chart, &[70, 71, 1023, 1025]);
    }
}

#[test]
fn every_structure_and_session_binding_preserves_gap_whitespace() {
    // The I3 scanners keep their own session and annotation rules; this
    // checks the shared source-row whitespace contract for all seven kinds.
    for name in [
        "swing_points",
        "market_structure",
        "fair_value_gaps",
        "order_blocks",
        "session_levels",
        "previous_period_levels",
        "opening_range",
    ] {
        let kind = IndicatorKind::schema_definition(name, 14, 2.0).unwrap();
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        let times = (0..1100)
            .map(|row| row as f64 * 3_600.0)
            .collect::<Vec<_>>();
        let mut close = (0..1100)
            .map(|row| 100.0 + (row as f64 * 0.13).sin())
            .collect::<Vec<_>>();
        let mut high = close.iter().map(|v| v + 2.0).collect::<Vec<_>>();
        let mut low = close.iter().map(|v| v - 2.0).collect::<Vec<_>>();
        for row in [70, 1023, 1024] {
            close[row] = f64::NAN;
            high[row] = f64::NAN;
            low[row] = f64::NAN;
        }
        chart
            .set_series_data(0, &times, &close, &high, &low, &close)
            .unwrap();
        let outputs = chart.add_indicator_kind(0, kind, None);
        assert!(!outputs.is_empty(), "{name}");
        let check = |chart: &ChartEngine, rows: &[usize]| {
            for &output in &outputs {
                let (aligned, values) = chart.data.series_data(output).unwrap();
                for &row in rows {
                    let offset = aligned
                        .iter()
                        .position(|&time| time == times[row] as i64)
                        .expect("gap row aligned");
                    assert!(
                        values[3][offset].is_nan(),
                        "{name} output {output} row {row}"
                    );
                }
            }
        };
        check(&chart, &[70, 1023, 1024]);
        chart.update_series_bar(0, times[1024], [101.0, 103.0, 99.0, 101.0]);
        check(&chart, &[70, 1023]);
    }
}

#[test]
fn an_all_whitespace_source_keeps_chained_outputs_empty_with_bounded_work_per_tick() {
    // A whitespace primary (an order-flow chart's candle grid) streams whitespace slots. Every
    // output starts at its first value, so they stay empty: no tick rewrites them, reports a
    // replace to a dependent, or does work proportional to the history.
    let rows = 5_000;
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    let times = (0..rows).map(|row| row as f64 * 60.0).collect::<Vec<_>>();
    let blank = vec![f64::NAN; rows];
    chart
        .set_series_data(0, &times, &blank, &blank, &blank, &blank)
        .unwrap();
    let rsi = chart.add_rsi(0, 14).unwrap();
    let sma = chart.add_sma(rsi, 5).unwrap();
    let generations = |chart: &ChartEngine| [rsi, sma].map(|id| chart.data.series_generation(id));
    let before = generations(&chart);
    for row in rows..rows + 50 {
        assert!(chart.update_series_bar(0, row as f64 * 60.0, [f64::NAN; 4]));
        for binding in &chart.indicators {
            assert!(
                binding.last_work_rows() <= 2,
                "{:?} did {} rows of work on a whitespace tick",
                binding.kind,
                binding.last_work_rows()
            );
        }
        assert_eq!(
            generations(&chart),
            before,
            "row {row}: an empty output was rewritten"
        );
        for id in [rsi, sma] {
            assert!(chart.data.series_data(id).unwrap().0.is_empty());
        }
    }
    // Real bars start the chain at its first values.
    for row in rows + 50..rows + 80 {
        let value = 100.0 + (row % 7) as f64;
        assert!(chart.update_series_bar(0, row as f64 * 60.0, [value; 4]));
    }
    let (rsi_times, _) = chart.data.series_data(rsi).unwrap();
    assert_eq!(rsi_times.first(), Some(&((rows as i64 + 50 + 14) * 60)));
    let (sma_times, sma_values) = chart.data.series_data(sma).unwrap();
    assert_eq!(sma_times.first(), Some(&((rows as i64 + 50 + 14 + 4) * 60)));
    assert!(sma_values[3].iter().all(|value| value.is_finite()));
}
