//! KLineChart indicator bindings: values against full recomputation through every update path,
//! per-row colors, layout, validation, metadata, and persistence.

use super::*;
use crate::tests::assert_indicator_binding_matches_full;

const ROWS: usize = 40;
const HOUR: f64 = 3_600.0;

/// A deterministic random walk whose candles open away from their close, so bars have direction.
fn candles(rows: usize, seed: u64) -> [Vec<f64>; 4] {
    let mut state = seed;
    let mut next = || {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (state >> 11) as f64 / (1u64 << 53) as f64
    };
    let mut close = 100.0;
    let (mut open, mut high, mut low, mut closes) = (vec![], vec![], vec![], vec![]);
    for _ in 0..rows {
        let o = close + (next() - 0.5) * 2.0;
        close = (o + (next() - 0.5) * 4.0).max(1.0);
        open.push(o);
        high.push(o.max(close) + next() * 1.5);
        low.push(o.min(close) - next() * 1.5);
        closes.push(close);
    }
    [open, high, low, closes]
}

/// Short-period definitions, so every output warms up well inside the test data.
fn short_indicators() -> Vec<Indicator> {
    let def = |name: &str, params: &[f64]| {
        Indicator::from_calc_params(name, params).unwrap_or_else(|| panic!("{name} {params:?}"))
    };
    vec![
        def("MA", &[3.0, 7.0]),
        def("EMA", &[3.0, 7.0]),
        def("SMA", &[5.0, 2.0]),
        def("BBI", &[2.0, 3.0, 4.0, 6.0]),
        def("VOL", &[3.0, 7.0]),
        def("MACD", &[3.0, 6.0, 4.0]),
        def("BOLL", &[5.0, 2.0]),
        def("KDJ", &[5.0, 3.0, 3.0]),
        def("RSI", &[3.0, 6.0]),
        def("BIAS", &[3.0, 6.0]),
        def("BRAR", &[5.0]),
        def("CCI", &[5.0]),
        def("CR", &[5.0, 3.0, 4.0, 6.0, 8.0]),
        def("DMA", &[3.0, 6.0, 4.0]),
        def("DMI", &[4.0, 3.0]),
        def("EMV", &[4.0]),
        def("MTM", &[4.0, 3.0]),
        def("OBV", &[4.0]),
        def("PVT", &[]),
        def("PSY", &[4.0, 3.0]),
        def("ROC", &[4.0, 3.0]),
        def("SAR", &[2.0, 2.0, 20.0]),
        def("TRIX", &[3.0, 4.0]),
        def("VR", &[5.0, 3.0]),
        def("WR", &[3.0, 6.0]),
        def("AO", &[3.0, 6.0]),
        def("AVP", &[]),
    ]
}

struct Fixture {
    chart: ChartEngine,
    volume: SeriesId,
    turnover: SeriesId,
}

fn fixture() -> Fixture {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    let volume = chart.add_series(SeriesKind::Histogram);
    let turnover = chart.add_series(SeriesKind::Line);
    chart.set_series_visible(turnover, false);
    let times = (0..ROWS).map(|row| row as f64 * HOUR).collect::<Vec<_>>();
    let [open, high, low, close] = candles(ROWS, 7);
    // Two zero-volume bars exercise KLineChart's zero-divisor branches.
    let volumes = (0..ROWS)
        .map(|row| {
            if row == 3 || row == 11 {
                0.0
            } else {
                1_000.0 + ((row * 37) % 11) as f64 * 150.0
            }
        })
        .collect::<Vec<_>>();
    let turnovers = volumes
        .iter()
        .zip(&close)
        .map(|(volume, close)| volume * close)
        .collect::<Vec<_>>();
    chart
        .set_series_data(0, &times, &open, &high, &low, &close)
        .unwrap();
    chart
        .set_series_data(volume, &times, &volumes, &volumes, &volumes, &volumes)
        .unwrap();
    chart
        .set_series_data(
            turnover, &times, &turnovers, &turnovers, &turnovers, &turnovers,
        )
        .unwrap();
    Fixture {
        chart,
        volume,
        turnover,
    }
}

fn bind(fixture: &mut Fixture, indicator: &Indicator) -> (SeriesId, Vec<SeriesId>) {
    let source = if matches!(indicator, Indicator::Avp) {
        fixture.turnover
    } else {
        0
    };
    let volume = indicator.needs_volume().then_some(fixture.volume);
    let outputs = fixture
        .chart
        .add_klinechart_indicator(source, indicator.clone(), volume);
    assert_eq!(
        outputs.len(),
        indicator.output_count(),
        "{}",
        indicator.title()
    );
    (source, outputs)
}

/// Every bar or dot output's per-row colors equal a fresh coloring of the current values.
fn assert_colors_match_full(chart: &ChartEngine, binding_index: usize) {
    let binding = &chart.indicators[binding_index];
    let IndicatorKind::KLineChart(indicator) = &binding.kind else {
        unreachable!("KLineChart binding")
    };
    for (output_index, &output) in binding.outputs.iter().enumerate() {
        let Some(rule) = klinechart_color_rule(indicator, output_index) else {
            continue;
        };
        let (_, values) = chart.data.series_data(output).unwrap();
        let values = values[3];
        let (_, bars) = chart.data.series_data(binding.source).unwrap();
        let offset = bars[3].len() - values.len();
        for row in 0..values.len() {
            let source_row = offset + row;
            let expected = rule.color(
                values[row],
                row.checked_sub(1).map(|previous| values[previous]),
                [
                    bars[0][source_row],
                    bars[1][source_row],
                    bars[2][source_row],
                    bars[3][source_row],
                ],
                chart.klinechart_palette(binding.source),
            );
            let actual = chart
                .data
                .point_color(output, PointColorChannel::Body, row)
                .unwrap_or(POINT_COLOR_ABSENT);
            assert_eq!(
                actual,
                expected,
                "{} output {output_index} row {row}",
                indicator.title()
            );
        }
    }
}

fn check(fixture: &Fixture, binding: usize) {
    assert_indicator_binding_matches_full(&fixture.chart, binding);
    assert_colors_match_full(&fixture.chart, binding);
}

fn bar(source: SeriesId, fixture: &Fixture, values: [f64; 4]) -> [f64; 4] {
    // A scalar (turnover) source takes one value in every slot.
    if source == fixture.turnover {
        [values[3] * 1_000.0; 4]
    } else {
        values
    }
}

#[test]
fn every_klinechart_binding_matches_full_recomputation_through_every_update_path() {
    for indicator in short_indicators() {
        let mut fixture = fixture();
        let (source, outputs) = bind(&mut fixture, &indicator);
        let binding = fixture.chart.indicators.len() - 1;
        check(&fixture, binding);

        // Append one bar.
        let appended = bar(source, &fixture, [104.0, 107.0, 103.0, 106.0]);
        assert!(fixture
            .chart
            .update_series_bar(source, ROWS as f64 * HOUR, appended));
        check(&fixture, binding);

        // Append a batch.
        let batch = candles(5, 99);
        let batch_times = (ROWS as i64 + 1..ROWS as i64 + 6)
            .map(|row| row * HOUR as i64)
            .collect::<Vec<_>>();
        let scale = |column: &Vec<f64>| {
            if source == fixture.turnover {
                batch[3].iter().map(|close| close * 1_000.0).collect()
            } else {
                column.clone()
            }
        };
        fixture.chart.update_series_bars_sanitized(
            source,
            batch_times,
            scale(&batch[0]),
            scale(&batch[1]),
            scale(&batch[2]),
            scale(&batch[3]),
        );
        check(&fixture, binding);

        // Replace the live bar repeatedly.
        let last = (ROWS + 5) as f64 * HOUR;
        for step in 0..50 {
            let close = 101.0 + step as f64 * 0.05;
            let values = bar(source, &fixture, [100.5, close + 1.0, close - 2.0, close]);
            fixture.chart.update_series_bar(source, last, values);
        }
        check(&fixture, binding);

        // Correct a historical bar, then insert one between two rows.
        let corrected = bar(source, &fixture, [99.0, 102.0, 97.0, 100.0]);
        fixture
            .chart
            .update_series_bar(source, 17.0 * HOUR, corrected);
        check(&fixture, binding);
        let inserted = bar(source, &fixture, [98.0, 101.0, 96.0, 99.0]);
        fixture
            .chart
            .update_series_bar(source, 17.5 * HOUR, inserted);
        check(&fixture, binding);

        // Drop the newest rows.
        fixture.chart.series_pop(source, 7).unwrap();
        check(&fixture, binding);

        // Replace the whole series with a different timeline.
        let [open, high, low, close] = candles(31, 3);
        let times = (0..31)
            .map(|row| 86_400.0 + row as f64 * 1_800.0)
            .collect::<Vec<_>>();
        if source == fixture.turnover {
            fixture
                .chart
                .set_series_data(source, &times, &close, &close, &close, &close)
                .unwrap();
        } else {
            fixture
                .chart
                .set_series_data(source, &times, &open, &high, &low, &close)
                .unwrap();
        }
        check(&fixture, binding);

        assert!(fixture.chart.remove_series(source));
        assert!(outputs
            .iter()
            .all(|&output| fixture.chart.series_kind(output).is_none()));
    }
}

#[test]
fn volume_changes_recompute_volume_indicators() {
    let mut fixture = fixture();
    let vol = Indicator::from_calc_params("VOL", &[3.0]).unwrap();
    let (_, outputs) = bind(&mut fixture, &vol);
    let binding = fixture.chart.indicators.len() - 1;
    fixture
        .chart
        .update_series_bar(fixture.volume, 20.0 * HOUR, [5_000.0; 4]);
    check(&fixture, binding);
    let (_, values) = fixture.chart.data.series_data(outputs[0]).unwrap();
    assert_eq!(values[3][20], 5_000.0, "VOL bars mirror the volume series");
}

#[test]
fn klinechart_bindings_use_klinechart_layout_and_style() {
    let mut fixture = fixture();
    let panes = fixture.chart.panes.len();

    let ma = bind(&mut fixture, &Indicator::from_name("MA").unwrap()).1;
    let macd = bind(&mut fixture, &Indicator::from_name("MACD").unwrap()).1;
    let vol = bind(&mut fixture, &Indicator::from_name("VOL").unwrap()).1;
    let sar = bind(&mut fixture, &Indicator::from_name("SAR").unwrap()).1;
    let chart = &fixture.chart;
    let entry = |id| chart.series_entry(id).unwrap();

    // Price overlays stay with the candles; MACD and VOL each get a pane of their own.
    let price_pane = entry(0).pane_index;
    assert!(ma.iter().all(|&id| entry(id).pane_index == price_pane));
    assert_eq!(entry(sar[0]).pane_index, price_pane);
    assert_eq!(chart.panes.len(), panes + 2);
    assert!(macd
        .iter()
        .all(|&id| entry(id).pane_index == entry(macd[0]).pane_index));
    assert_ne!(entry(macd[0]).pane_index, price_pane);
    assert_ne!(entry(vol[0]).pane_index, entry(macd[0]).pane_index);

    // Lines take KLineChart's palette in order at 1px, with no price-axis labels.
    let colors = ma
        .iter()
        .map(|&id| entry(id).line_color.clone().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(colors, KLINECHART_LINE_COLORS[..4]);
    let titles = ma
        .iter()
        .map(|&id| entry(id).title.clone())
        .collect::<Vec<_>>();
    assert_eq!(titles, ["MA5", "MA10", "MA30", "MA60"]);
    for &id in ma.iter().chain(&macd) {
        assert_eq!(entry(id).line_width, Some(1.0));
        assert!(!entry(id).last_value_visible);
        assert!(!entry(id).title_visible);
        assert!(!entry(id).price_line_visible);
    }

    // Bars are histograms; the DEA line takes the second palette color after DIF.
    assert_eq!(chart.series_kind(macd[2]), Some(SeriesKind::Histogram));
    assert_eq!(chart.series_kind(vol[0]), Some(SeriesKind::Histogram));
    assert_eq!(
        entry(macd[1]).line_color.as_deref(),
        Some(KLINECHART_LINE_COLORS[1])
    );
    assert_eq!(
        entry(vol[1]).line_color.as_deref(),
        Some(KLINECHART_LINE_COLORS[0])
    );
    assert_eq!(entry(vol[0]).price_format.kind, PriceFormatKind::Volume);
    assert_eq!(entry(macd[0]).price_format.precision, 4);

    // SAR is a row of dots.
    assert_eq!(chart.series_kind(sar[0]), Some(SeriesKind::Line));
    assert!(!entry(sar[0]).line_visible);
    assert!(entry(sar[0]).point_markers);
}

#[test]
fn style_reset_restores_klinechart_presentation() {
    let mut fixture = fixture();
    let sar = bind(&mut fixture, &Indicator::from_name("SAR").unwrap()).1;
    let ma = bind(&mut fixture, &Indicator::from_name("MA").unwrap()).1;
    fixture.chart.reset_style_to_defaults();
    let chart = &fixture.chart;
    let sar = chart.series_entry(sar[0]).unwrap();
    assert!(!sar.line_visible && sar.point_markers);
    let ma = chart.series_entry(ma[1]).unwrap();
    assert_eq!(ma.line_width, Some(1.0));
    assert!(!ma.title_visible && !ma.last_value_visible);
    assert_eq!(ma.line_color.as_deref(), Some(KLINECHART_LINE_COLORS[1]));
}

#[test]
fn invalid_klinechart_bindings_leave_the_chart_unchanged() {
    let mut fixture = fixture();
    let series_before = fixture.chart.series.len();
    let volume = fixture.volume;
    let turnover = fixture.turnover;
    let chart = &mut fixture.chart;
    let vol = Indicator::from_name("VOL").unwrap();
    let macd = Indicator::from_name("MACD").unwrap();
    assert!(chart
        .add_klinechart_indicator(0, vol.clone(), None)
        .is_empty());
    assert!(chart
        .add_klinechart_indicator(0, vol.clone(), Some(0))
        .is_empty());
    assert!(chart
        .add_klinechart_indicator(0, macd, Some(volume))
        .is_empty());
    assert!(chart
        .add_klinechart_indicator(0, Indicator::Avp, Some(volume))
        .is_empty());
    assert!(chart
        .add_klinechart_indicator(0, Indicator::Ma { periods: vec![] }, None)
        .is_empty());
    assert!(chart
        .add_klinechart_indicator(
            0,
            Indicator::Ma {
                periods: vec![1, 2, 3, 4, 5, 6],
            },
            None,
        )
        .is_empty());
    assert!(chart
        .add_klinechart_indicator(
            0,
            Indicator::Vol {
                periods: vec![1, 2, 3, 4, 5]
            },
            Some(volume)
        )
        .is_empty());
    assert_eq!(chart.series.len(), series_before);
    assert!(chart.indicators.is_empty());

    assert_eq!(
        chart
            .add_klinechart_indicator(turnover, Indicator::Avp, Some(volume))
            .len(),
        1
    );
}

#[test]
fn indicator_info_schema_and_serialization_describe_klinechart_bindings() {
    let mut fixture = fixture();
    let macd = Indicator::from_name("MACD").unwrap();
    let outputs = bind(&mut fixture, &macd).1;
    let info = fixture.chart.indicator_info(outputs[1]).unwrap();
    assert_eq!(info.kind, "klinechart_macd");
    assert_eq!(info.output_name, "dea");
    assert_eq!(info.output_count, 3);
    assert_eq!(info.period, 12);
    assert_eq!(info.parameters.klinechart.as_ref(), Some(&macd));

    let kind = IndicatorKind::KLineChart(macd.clone());
    let schema = ChartEngine::indicator_schema(&kind);
    assert_eq!(schema.kind, "klinechart_macd");
    let names = schema
        .parameters
        .iter()
        .map(|parameter| parameter.name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(names, ["source", "short", "long", "signal"]);
    let outputs = schema
        .outputs
        .iter()
        .map(|output| output.name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(outputs, ["dif", "dea", "macd"]);
    let vol = ChartEngine::indicator_schema(&IndicatorKind::KLineChart(
        Indicator::from_name("VOL").unwrap(),
    ));
    assert_eq!(vol.parameters.last().unwrap().name, "volume_source");

    let json = serde_json::to_value(&kind).unwrap();
    assert_eq!(
        json,
        serde_json::json!({"kind": "klinechart", "indicator": "macd", "short": 12, "long": 26, "signal": 9})
    );
    assert_eq!(serde_json::from_value::<IndicatorKind>(json).unwrap(), kind);
    let pvt = IndicatorKind::KLineChart(Indicator::Pvt);
    let json = serde_json::to_string(&pvt).unwrap();
    assert_eq!(json, r#"{"kind":"klinechart","indicator":"pvt"}"#);
    assert_eq!(serde_json::from_str::<IndicatorKind>(&json).unwrap(), pvt);

    for name in aeris_charts_indicators::klinechart::NAMES {
        let indicator = Indicator::from_name(name).unwrap();
        let kind_name = klinechart_kind_name(&indicator);
        assert_eq!(
            klinechart_indicator_for_kind_name(kind_name),
            Some(indicator)
        );
    }
}

#[test]
fn klinechart_bindings_round_trip_through_persistence() {
    let mut fixture = fixture();
    let definitions = [
        Indicator::from_name("MA").unwrap(),
        Indicator::from_calc_params("MACD", &[3.0, 6.0, 4.0]).unwrap(),
        Indicator::from_calc_params("VOL", &[3.0, 7.0]).unwrap(),
        Indicator::from_name("SAR").unwrap(),
        Indicator::Avp,
    ];
    for indicator in &definitions {
        bind(&mut fixture, indicator);
    }
    let document = fixture.chart.export_state_json().unwrap();
    assert!(document.contains(r#""kind":"klinechart""#));

    let mut restored = fixture_without_indicators();
    restored.chart.import_state_json(&document).unwrap();
    let bindings = restored.chart.indicator_bindings();
    assert_eq!(
        bindings
            .iter()
            .map(|binding| binding.kind.clone())
            .collect::<Vec<_>>(),
        definitions
            .iter()
            .cloned()
            .map(IndicatorKind::KLineChart)
            .collect::<Vec<_>>()
    );
    for (index, binding) in bindings.iter().enumerate() {
        assert_eq!(binding.outputs.len(), definitions[index].output_count());
        check(&restored, index);
    }
    assert_eq!(bindings[2].volume_source, Some(restored.volume));
    assert_eq!(bindings[4].source, restored.turnover);
}

fn fixture_without_indicators() -> Fixture {
    let fixture = fixture();
    assert!(fixture.chart.indicators.is_empty());
    fixture
}

#[test]
fn bar_colors_follow_the_chart_market_colors() {
    let mut fixture = fixture();
    let (_, outputs) = bind(&mut fixture, &Indicator::from_name("VOL").unwrap());
    let binding = fixture.chart.indicators.len() - 1;
    let body = |chart: &ChartEngine, row| {
        chart
            .data
            .point_color(outputs[0], PointColorChannel::Body, row)
            .unwrap()
    };
    // Row 0 of the fixture closes above its open or below it; find one of each.
    let (_, bars) = fixture.chart.data.series_data(0).unwrap();
    let rising = (0..ROWS).find(|&row| bars[3][row] > bars[0][row]).unwrap();
    let default_up = body(&fixture.chart, rising);

    // A red-up convention: swap the chart's bullish and bearish colors, then refresh.
    fixture
        .chart
        .options
        .apply_str(r##"{"layout":{"bullishColor":"#F92855","bearishColor":"#2DC08E"}}"##)
        .unwrap();
    fixture.chart.refresh_klinechart_colors();
    let red_up = body(&fixture.chart, rising);
    assert_ne!(red_up, default_up);
    assert_eq!(red_up >> 8, 0x00F9_2855);
    check(&fixture, binding);

    // An explicit series color wins over the layout default.
    fixture.chart.series[0].up_color = Some("#0000ff".into());
    fixture.chart.refresh_klinechart_colors();
    assert_eq!(body(&fixture.chart, rising) >> 8, 0x0000_00ff);
}
