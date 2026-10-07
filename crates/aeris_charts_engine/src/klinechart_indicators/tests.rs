//! KLineChart indicator bindings: values against full recomputation through every update path,
//! whitespace rows, live ticks and bounded tick work, warm-up reporting, per-row colors, layout,
//! validation, metadata, and persistence.

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

/// The rows of a chart's candle, volume and turnover series. A whitespace row has no sample on any
/// of them (NaN), the way a missing bar or a pre-installed session slot is stored.
#[derive(Clone)]
struct Data {
    candles: Vec<[f64; 4]>,
    volumes: Vec<f64>,
    turnovers: Vec<f64>,
}

impl Data {
    /// `rows` generated bars, with every row of `blank` whitespace.
    fn generate(rows: usize, blank: &[usize]) -> Self {
        let [open, high, low, close] = candles(rows, 7);
        let mut data = Self {
            candles: Vec::new(),
            volumes: Vec::new(),
            turnovers: Vec::new(),
        };
        for row in 0..rows {
            // Two zero-volume bars exercise KLineChart's zero-divisor branches.
            let volume = if row == 3 || row == 11 {
                0.0
            } else {
                1_000.0 + ((row * 37) % 11) as f64 * 150.0
            };
            data.candles
                .push([open[row], high[row], low[row], close[row]]);
            data.volumes.push(volume);
            data.turnovers.push(volume * close[row]);
        }
        for &row in blank {
            data.blank(row);
        }
        data
    }

    fn blank(&mut self, row: usize) {
        self.candles[row] = [f64::NAN; 4];
        self.volumes[row] = f64::NAN;
        self.turnovers[row] = f64::NAN;
    }

    fn rows(&self) -> usize {
        self.candles.len()
    }

    fn is_blank(&self, row: usize) -> bool {
        self.candles[row][3].is_nan()
    }

    /// Appends the generated bar of `row`, which must be the next row.
    fn push_generated(&mut self, row: usize) {
        assert_eq!(row, self.rows(), "rows append in order");
        let [open, high, low, close] = candles(row + 1, 7).map(|column| column[row]);
        let volume = 1_000.0 + ((row * 37) % 11) as f64 * 150.0;
        self.candles.push([open, high, low, close]);
        self.volumes.push(volume);
        self.turnovers.push(volume * close);
    }
}

/// A chart holding the listed rows of `data` (row `r` at time `r` hours) on the candle, volume and
/// turnover series.
fn install(data: &Data, rows: &[usize]) -> Fixture {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    let volume = chart.add_series(SeriesKind::Histogram);
    let turnover = chart.add_series(SeriesKind::Line);
    chart.set_series_visible(turnover, false);
    let times = rows
        .iter()
        .map(|&row| row as f64 * HOUR)
        .collect::<Vec<_>>();
    let column = |index: usize| {
        rows.iter()
            .map(|&row| data.candles[row][index])
            .collect::<Vec<_>>()
    };
    let volumes = rows
        .iter()
        .map(|&row| data.volumes[row])
        .collect::<Vec<_>>();
    let turnovers = rows
        .iter()
        .map(|&row| data.turnovers[row])
        .collect::<Vec<_>>();
    chart
        .set_series_data(0, &times, &column(0), &column(1), &column(2), &column(3))
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

fn fixture() -> Fixture {
    install(&Data::generate(ROWS, &[]), &(0..ROWS).collect::<Vec<_>>())
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

/// Every bar or dot output's per-row colors equal a fresh coloring of the current values. Each
/// output row finds its source bar by time, independently of how the output was trimmed.
fn assert_colors_match_full(chart: &ChartEngine, binding_index: usize) {
    let binding = &chart.indicators[binding_index];
    let IndicatorKind::KLineChart(indicator) = &binding.kind else {
        unreachable!("KLineChart binding")
    };
    for (output_index, &output) in binding.outputs.iter().enumerate() {
        let Some(rule) = klinechart_color_rule(indicator, output_index) else {
            continue;
        };
        let (times, values) = chart.data.series_data(output).unwrap();
        let values = values[3];
        let (source_times, bars) = chart.data.series_data(binding.source).unwrap();
        for row in 0..values.len() {
            let source_row = source_times
                .binary_search(&times[row])
                .unwrap_or_else(|_| panic!("output row {row} has a source bar"));
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
fn colored_outputs_start_after_leading_whitespace_and_follow_repairs_before_their_start() {
    // Q-H trims every output to its first value, so a histogram or dot output over a source with
    // leading whitespace starts later than the source; its colours start with it.
    const COLOR_ROWS: usize = 60;
    for indicator in short_indicators()
        .into_iter()
        .filter(|indicator| matches!(indicator.name(), "MACD" | "VOL" | "SAR" | "AO"))
    {
        let label = label_of(&indicator);
        let mut data = Data::generate(COLOR_ROWS, &[0, 1, 2, 20]);
        let mut fixture = install(&data, &(0..COLOR_ROWS).collect::<Vec<_>>());
        let (_, outputs) = bind(&mut fixture, &indicator);
        let binding = fixture.chart.indicators.len() - 1;
        assert_matches_fresh_bind(&fixture, binding, &format!("{label}, leading whitespace"));
        let first_time = |fixture: &Fixture| {
            outputs
                .iter()
                .map(|&output| fixture.chart.data.series_data(output).unwrap().0[0])
                .min()
                .unwrap()
        };
        assert!(
            first_time(&fixture) >= 3 * HOUR as i64,
            "{label}: no output row before the first bar"
        );

        // Repairs before the first value move every output's start earlier, then later.
        let real = Data::generate(COLOR_ROWS, &[]);
        for (row, step) in [(2, "row 2 filled"), (0, "row 0 filled")] {
            data.candles[row] = real.candles[row];
            data.volumes[row] = real.volumes[row];
            data.turnovers[row] = real.turnovers[row];
            feed(&mut fixture, &data, row);
            assert_matches_fresh_bind(&fixture, binding, &format!("{label}, {step}"));
        }
        data.blank(0);
        feed(&mut fixture, &data, 0);
        assert_matches_fresh_bind(&fixture, binding, &format!("{label}, row 0 blanked"));

        // Live ticks after the repairs.
        data.push_generated(COLOR_ROWS);
        feed(&mut fixture, &data, COLOR_ROWS);
        assert_matches_fresh_bind(&fixture, binding, &format!("{label}, append"));
        data.candles[COLOR_ROWS] = [101.0, 103.0, 99.5, 102.5];
        data.turnovers[COLOR_ROWS] = data.volumes[COLOR_ROWS] * 102.5;
        feed(&mut fixture, &data, COLOR_ROWS);
        assert_matches_fresh_bind(&fixture, binding, &format!("{label}, tick"));
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
        assert!(
            fixture
                .chart
                .update_series_bar(source, ROWS as f64 * HOUR, appended)
        );
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
        assert!(
            outputs
                .iter()
                .all(|&output| fixture.chart.series_kind(output).is_none())
        );
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
    assert!(
        macd.iter()
            .all(|&id| entry(id).pane_index == entry(macd[0]).pane_index)
    );
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
    assert!(
        chart
            .add_klinechart_indicator(0, vol.clone(), None)
            .is_empty()
    );
    assert!(
        chart
            .add_klinechart_indicator(0, vol.clone(), Some(0))
            .is_empty()
    );
    assert!(
        chart
            .add_klinechart_indicator(0, macd, Some(volume))
            .is_empty()
    );
    assert!(
        chart
            .add_klinechart_indicator(0, Indicator::Avp, Some(volume))
            .is_empty()
    );
    assert!(
        chart
            .add_klinechart_indicator(0, Indicator::Ma { periods: vec![] }, None)
            .is_empty()
    );
    assert!(
        chart
            .add_klinechart_indicator(
                0,
                Indicator::Ma {
                    periods: vec![1, 2, 3, 4, 5, 6],
                },
                None,
            )
            .is_empty()
    );
    assert!(
        chart
            .add_klinechart_indicator(
                0,
                Indicator::Vol {
                    periods: vec![1, 2, 3, 4, 5]
                },
                Some(volume)
            )
            .is_empty()
    );
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

// ---- The stepped runtime: whitespace contract, live ticks, bounded work, warm-up ----------------

/// Bindings travel with their chart between threads.
const _: fn() = || {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<aeris_charts_indicators::IncrementalState>();
};

/// All 27 templates with KLineChart's default parameters.
fn default_indicators() -> Vec<Indicator> {
    aeris_charts_indicators::klinechart::NAMES
        .iter()
        .map(|name| Indicator::from_name(name).expect("every listed name is a template"))
        .collect()
}

/// Every template twice: with short periods, and with KLineChart's defaults.
fn short_and_default_indicators() -> Vec<Indicator> {
    short_indicators()
        .into_iter()
        .chain(default_indicators())
        .collect()
}

/// Whitespace rows of a `rows`-bar chart: two leading rows (the warm-up of a chained source), a
/// run of three, a single row, and one near the end.
fn gaps(rows: usize) -> Vec<usize> {
    vec![
        1,
        2,
        rows / 3,
        rows / 3 + 1,
        rows / 3 + 2,
        rows / 2,
        rows - 9,
    ]
}

/// A single gap, and a run at the very end of the chart.
fn gaps_with_trailing_run(rows: usize) -> Vec<usize> {
    vec![rows / 4, rows - 3, rows - 2, rows - 1]
}

/// The binding's template.
fn template(chart: &ChartEngine, binding: usize) -> Indicator {
    match &chart.indicators[binding].kind {
        IndicatorKind::KLineChart(indicator) => indicator.clone(),
        other => unreachable!("not a KLineChart binding: {other:?}"),
    }
}

/// `(times, values)` of every listed output series.
type Outputs = Vec<(Vec<i64>, Vec<f64>)>;

fn outputs_of(chart: &ChartEngine, ids: &[SeriesId]) -> Outputs {
    ids.iter()
        .map(|&id| {
            let (times, values) = chart.data.series_data(id).expect("output series");
            (times.to_vec(), values[3].to_vec())
        })
        .collect()
}

/// Equal numbers, or both NaN. Anything else, including a different zero or rounding, differs.
fn same_bits(a: f64, b: f64) -> bool {
    a.to_bits() == b.to_bits() || (a.is_nan() && b.is_nan())
}

fn assert_outputs_equal(label: &str, actual: &Outputs, expected: &Outputs) {
    assert_eq!(actual.len(), expected.len(), "{label}: output count");
    for (output, ((actual_times, actual), (expected_times, expected))) in
        actual.iter().zip(expected).enumerate()
    {
        assert_eq!(
            actual_times, expected_times,
            "{label} output {output} times"
        );
        for (row, (&actual, &expected)) in actual.iter().zip(expected).enumerate() {
            assert!(
                same_bits(actual, expected),
                "{label} output {output} row {row}: {actual} != {expected}"
            );
        }
    }
}

/// The values over a chart with whitespace rows equal the values over the same chart without
/// them: a whitespace row (or a row still warming up once the whitespace is removed) is NaN, and
/// every other row has bit for bit the value the chart without whitespace holds at that time.
/// Returns how many finite values were compared.
fn assert_equal_without_whitespace(label: &str, spaced: &Outputs, compact: &Outputs) -> usize {
    assert_eq!(spaced.len(), compact.len(), "{label}: output count");
    let mut finite = 0;
    for (output, ((spaced_times, spaced), (compact_times, compact))) in
        spaced.iter().zip(compact).enumerate()
    {
        let expected = compact_times
            .iter()
            .copied()
            .zip(compact.iter().copied())
            .collect::<std::collections::HashMap<_, _>>();
        for (time, &value) in spaced_times.iter().zip(spaced) {
            match expected.get(time) {
                Some(&expected) => {
                    assert!(
                        same_bits(value, expected),
                        "{label} output {output} at {time}: {value} != {expected} without the whitespace rows"
                    );
                    finite += usize::from(value.is_finite());
                }
                None => assert!(
                    value.is_nan(),
                    "{label} output {output} at {time}: a whitespace or warming-up row holds {value}"
                ),
            }
        }
        for time in compact_times {
            assert!(
                spaced_times.binary_search(time).is_ok(),
                "{label} output {output} lost row {time}"
            );
        }
    }
    finite
}

/// A new chart holding the series data `fixture` currently holds, the same three series in the
/// same order.
fn fresh_copy(fixture: &Fixture) -> Fixture {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    let volume = chart.add_series(SeriesKind::Histogram);
    let turnover = chart.add_series(SeriesKind::Line);
    chart.set_series_visible(turnover, false);
    for (from, into) in [
        (0, 0),
        (fixture.volume, volume),
        (fixture.turnover, turnover),
    ] {
        let (times, values) = fixture.chart.data.series_data(from).unwrap();
        let seconds = times.iter().map(|&time| time as f64).collect::<Vec<_>>();
        chart
            .set_series_data(into, &seconds, values[0], values[1], values[2], values[3])
            .unwrap();
    }
    Fixture {
        chart,
        volume,
        turnover,
    }
}

/// The live binding equals a fresh bind of the same template over the data the chart holds now,
/// bit for bit, and agrees with the formula run over the rows that carry a sample.
fn assert_matches_fresh_bind(fixture: &Fixture, binding: usize, label: &str) {
    let indicator = template(&fixture.chart, binding);
    let mut fresh = fresh_copy(fixture);
    let (_, fresh_outputs) = bind(&mut fresh, &indicator);
    assert_outputs_equal(
        label,
        &outputs_of(&fixture.chart, &fixture.chart.indicators[binding].outputs),
        &outputs_of(&fresh.chart, &fresh_outputs),
    );
    check(fixture, binding);
}

/// Writes `row` of `data` to the chart the way a live feed does: the candle, then the volume,
/// then the turnover.
fn feed(fixture: &mut Fixture, data: &Data, row: usize) {
    let time = row as f64 * HOUR;
    let volume = fixture.volume;
    let turnover = fixture.turnover;
    assert!(fixture.chart.update_series_bar(0, time, data.candles[row]));
    assert!(
        fixture
            .chart
            .update_series_bar(volume, time, [data.volumes[row]; 4])
    );
    assert!(
        fixture
            .chart
            .update_series_bar(turnover, time, [data.turnovers[row]; 4])
    );
}

fn label_of(indicator: &Indicator) -> String {
    format!("{} {:?}", indicator.title(), indicator.calc_params())
}

#[test]
fn whitespace_rows_never_enter_a_klinechart_window_through_any_update_path() {
    for (indicators, rows) in [(short_indicators(), 120), (default_indicators(), 400)] {
        let every_row = (0..rows).collect::<Vec<_>>();
        for indicator in indicators {
            for blank in [gaps(rows), gaps_with_trailing_run(rows)] {
                let label = format!("{} with whitespace at {blank:?}", label_of(&indicator));
                let data = Data::generate(rows, &blank);
                let kept = every_row
                    .iter()
                    .copied()
                    .filter(|row| !blank.contains(row))
                    .collect::<Vec<_>>();

                // The same chart without the whitespace rows is the reference.
                let mut without = install(&data, &kept);
                let (_, without_outputs) = bind(&mut without, &indicator);
                let reference = outputs_of(&without.chart, &without_outputs);

                // A bulk load of the whitespace chart.
                let mut loaded = install(&data, &every_row);
                let (_, outputs) = bind(&mut loaded, &indicator);
                let binding = loaded.chart.indicators.len() - 1;
                let compared = assert_equal_without_whitespace(
                    &format!("{label}, bulk load"),
                    &outputs_of(&loaded.chart, &outputs),
                    &reference,
                );
                assert!(compared > 0, "{label}: nothing finite was compared");
                check(&loaded, binding);

                // Streamed: whitespace rows arrive through the ordinary bar update path.
                let mut streamed = install(&data, &every_row[..10]);
                let (_, streamed_outputs) = bind(&mut streamed, &indicator);
                for row in 10..rows {
                    feed(&mut streamed, &data, row);
                }
                assert_equal_without_whitespace(
                    &format!("{label}, streamed"),
                    &outputs_of(&streamed.chart, &streamed_outputs),
                    &reference,
                );
                check(&streamed, binding);

                // A whitespace slot filled later with a real bar equals the chart that always
                // had that bar.
                let slot = rows / 3 + 1;
                let mut filled = data.clone();
                let real = Data::generate(rows, &[]);
                filled.candles[slot] = real.candles[slot];
                filled.volumes[slot] = real.volumes[slot];
                filled.turnovers[slot] = real.turnovers[slot];
                let filled_kept = every_row
                    .iter()
                    .copied()
                    .filter(|&row| !filled.is_blank(row))
                    .collect::<Vec<_>>();
                let mut filled_without = install(&filled, &filled_kept);
                let (_, filled_without_outputs) = bind(&mut filled_without, &indicator);
                feed(&mut loaded, &filled, slot);
                assert_equal_without_whitespace(
                    &format!("{label}, slot {slot} filled"),
                    &outputs_of(&loaded.chart, &outputs),
                    &outputs_of(&filled_without.chart, &filled_without_outputs),
                );
                check(&loaded, binding);
            }
        }
    }
}

/// Everything before the last row of every output is exactly what it was.
fn assert_only_last_row_differs(label: &str, before: &Outputs, after: &Outputs) {
    for (output, ((before_times, before), (after_times, after))) in
        before.iter().zip(after).enumerate()
    {
        assert_eq!(before_times, after_times, "{label} output {output} times");
        let last = before.len() - 1;
        for row in 0..last {
            assert!(
                same_bits(before[row], after[row]),
                "{label} output {output} row {row} changed: {} -> {}",
                before[row],
                after[row]
            );
        }
    }
}

#[test]
fn live_ticks_change_only_the_newest_row_and_equal_a_fresh_bind_for_every_template() {
    const TICK_ROWS: usize = 200;
    for indicator in short_and_default_indicators() {
        for blank in [Vec::new(), gaps(TICK_ROWS)] {
            let label = format!("{} with whitespace at {blank:?}", label_of(&indicator));
            let mut data = Data::generate(TICK_ROWS, &blank);
            let mut fixture = install(&data, &(0..TICK_ROWS).collect::<Vec<_>>());
            let (_, outputs) = bind(&mut fixture, &indicator);
            let binding = fixture.chart.indicators.len() - 1;
            assert_matches_fresh_bind(&fixture, binding, &format!("{label}, bulk load"));
            let loaded = outputs_of(&fixture.chart, &outputs);

            // Replace the live bar with a strong rise, then with a strong fall: the history
            // before it stays bit for bit, and the newest row follows the bar.
            let last = TICK_ROWS - 1;
            let previous_close = data.candles[last - 1][3];
            let mut newest = Vec::new();
            for (revision, (close, volume)) in
                [(10.0, 9_000.0), (-10.0, 150.0)].into_iter().enumerate()
            {
                let close = previous_close + close;
                data.candles[last] = [
                    previous_close,
                    close.max(previous_close) + 1.0,
                    close.min(previous_close) - 1.0,
                    close,
                ];
                data.volumes[last] = volume;
                data.turnovers[last] = volume * close;
                feed(&mut fixture, &data, last);
                let context = format!("{label}, replacement {revision}");
                assert_matches_fresh_bind(&fixture, binding, &context);
                let after = outputs_of(&fixture.chart, &outputs);
                assert_only_last_row_differs(&context, &loaded, &after);
                newest.push(
                    after
                        .iter()
                        .map(|(_, values)| values[values.len() - 1].to_bits())
                        .collect::<Vec<_>>(),
                );
            }
            assert_ne!(
                newest[0], newest[1],
                "{label}: opposite bars must move the newest row of some output"
            );

            // An append adds one row to every output and leaves every earlier row alone.
            let before_append = outputs_of(&fixture.chart, &outputs);
            data.push_generated(TICK_ROWS);
            feed(&mut fixture, &data, TICK_ROWS);
            assert_matches_fresh_bind(&fixture, binding, &format!("{label}, append"));
            let after_append = outputs_of(&fixture.chart, &outputs);
            for ((before_times, before), (after_times, after)) in
                before_append.iter().zip(&after_append)
            {
                assert_eq!(
                    after.len(),
                    before.len() + 1,
                    "{label}: an append adds one row"
                );
                assert_eq!(after_times[..before_times.len()], before_times[..]);
                for (row, (&before, &after)) in before.iter().zip(after).enumerate() {
                    assert!(
                        same_bits(before, after),
                        "{label}: the append changed row {row}: {before} -> {after}"
                    );
                }
            }

            // The new live bar, revised; a historical correction; a bar turned into whitespace
            // and a whitespace slot filled in.
            data.candles[TICK_ROWS] = [101.0, 103.0, 99.5, 102.5];
            data.turnovers[TICK_ROWS] = data.volumes[TICK_ROWS] * 102.5;
            feed(&mut fixture, &data, TICK_ROWS);
            assert_matches_fresh_bind(&fixture, binding, &format!("{label}, revised append"));
            data.candles[120] = [99.0, 104.0, 96.0, 101.0];
            data.turnovers[120] = data.volumes[120] * 101.0;
            feed(&mut fixture, &data, 120);
            assert_matches_fresh_bind(&fixture, binding, &format!("{label}, correction"));
            data.blank(70);
            feed(&mut fixture, &data, 70);
            assert_matches_fresh_bind(&fixture, binding, &format!("{label}, bar blanked"));
            let real = Data::generate(TICK_ROWS, &[]);
            data.candles[2] = real.candles[2];
            data.volumes[2] = real.volumes[2];
            data.turnovers[2] = real.turnovers[2];
            feed(&mut fixture, &data, 2);
            assert_matches_fresh_bind(&fixture, binding, &format!("{label}, slot filled"));
        }
    }
}

/// Rows a template's steps can read back, bounded from its public parameters: every lookback is
/// at most twice the sum of the template's whole-number periods (CR adds its shift, at most the
/// period itself).
fn reach(indicator: &Indicator) -> usize {
    2 * indicator
        .params()
        .iter()
        .filter(|param| param.integer)
        .map(|param| param.value as usize)
        .sum::<usize>()
        + 8
}

/// How far back the historical repair of the work-bound test reaches.
const REPAIR_DEPTH: usize = 50;

/// Rows of work the bindings an update reached report after each live update, in binding order:
/// a binding the updated series is not an input of keeps its previous rebuild and has no entry.
fn work_of_live_updates(rows: usize, blank: &[usize]) -> Vec<Vec<Option<usize>>> {
    let indicators = short_and_default_indicators();
    let mut data = Data::generate(rows, blank);
    let mut fixture = install(&data, &(0..rows).collect::<Vec<_>>());
    for indicator in &indicators {
        bind(&mut fixture, indicator);
    }
    let volume = fixture.volume;
    let turnover = fixture.turnover;
    let last = rows - 1;
    data.push_generated(rows);
    let mut revised = data.candles[last];
    revised[3] += 0.75;
    let live = last as f64 * HOUR;
    let next = rows as f64 * HOUR;
    let mut repaired = data.candles[rows - REPAIR_DEPTH];
    repaired[3] -= 0.5;
    let updates = [
        // Replace the live bar's candle, volume and turnover.
        (0, live, revised),
        (volume, live, [data.volumes[last] + 5.0; 4]),
        (turnover, live, [data.turnovers[last] + 5.0; 4]),
        // Open the next bar: candle, volume and turnover in the usual order, then revise it.
        (0, next, data.candles[rows]),
        (volume, next, [data.volumes[rows]; 4]),
        (turnover, next, [data.turnovers[rows]; 4]),
        (0, next, revised),
        // Correct a bar `REPAIR_DEPTH` rows back.
        (0, (rows - REPAIR_DEPTH) as f64 * HOUR, repaired),
    ];
    updates
        .into_iter()
        .map(|(series, time, values)| {
            assert!(fixture.chart.update_series_bar(series, time, values));
            fixture
                .chart
                .indicators
                .iter()
                .map(|binding| {
                    (binding.source == series || binding.volume_source == Some(series))
                        .then(|| binding.last_work_rows())
                })
                .collect()
        })
        .collect()
}

/// Index, in [`work_of_live_updates`], of the historical repair.
const REPAIR_UPDATE: usize = 7;
/// Rows between two checkpoints of the incremental history.
const CHECKPOINT_INTERVAL: usize = 1_024;

#[test]
fn a_klinechart_tick_costs_the_templates_window_not_the_history() {
    let indicators = short_and_default_indicators();
    let reaches = indicators.iter().map(reach).collect::<Vec<_>>();
    let mut worst_clean = 0;
    for gapped in [false, true] {
        let mut by_history = Vec::new();
        for rows in [2_000, 8_000] {
            // With whitespace in history the window is compacted: it also re-reads the rows its
            // lookback spans, including a whitespace row inside it.
            let blank = if gapped {
                vec![50, 51, rows / 2, rows - 4]
            } else {
                Vec::new()
            };
            let work = work_of_live_updates(rows, &blank);
            for (update, per_binding) in work.iter().enumerate() {
                for (index, rows_of_work) in per_binding.iter().enumerate() {
                    let Some(&rows_of_work) = rows_of_work.as_ref() else {
                        continue;
                    };
                    // A historical repair replays the rows from the changed one to the end, and
                    // re-derives the same suffix of any aligned weight column, plus at most one
                    // checkpoint interval before it.
                    let limit = if update == REPAIR_UPDATE {
                        2 * REPAIR_DEPTH + CHECKPOINT_INTERVAL + reaches[index] + 4
                    } else if gapped {
                        reaches[index] + 4
                    } else {
                        2
                    };
                    assert!(
                        rows_of_work <= limit,
                        "{} over {rows} rows (whitespace in history: {gapped}), update {update}: {rows_of_work} work rows (bound {limit})",
                        label_of(&indicators[index]),
                    );
                    if !gapped {
                        worst_clean = worst_clean.max(rows_of_work);
                    }
                }
            }
            by_history.push(work);
        }
        // The window is a property of the template: four times the history costs the same for
        // every tick. (A repair's replay starts at the checkpoint before the changed row, so its
        // cost depends on where that row falls between two checkpoints; the bound above covers it.)
        assert_eq!(
            by_history[0][..REPAIR_UPDATE],
            by_history[1][..REPAIR_UPDATE],
            "whitespace in history: {gapped}: work must not depend on the history length"
        );
    }
    assert!(worst_clean > 0, "the ticks did evaluate rows");
}

#[test]
fn klinechart_outputs_start_where_the_template_declares_and_the_warm_up_query_agrees() {
    const START_ROWS: usize = 400;
    let data = Data::generate(START_ROWS, &[]);
    let rows = (0..START_ROWS).collect::<Vec<_>>();
    for indicator in short_and_default_indicators() {
        let label = label_of(&indicator);
        let mut fixture = install(&data, &rows);
        let (source, outputs) = bind(&mut fixture, &indicator);
        let source_times = fixture.chart.data.series_data(source).unwrap().0.to_vec();
        let starts = indicator.output_starts();
        let extra = indicator.extra_convergence_rows();
        for (output, &id) in outputs.iter().enumerate() {
            let context = format!("{label} output {output}");
            // The warm-up query reports the declared start, and the convergence horizon is that
            // start plus the template's extra rows (or none for recursive and path formulas).
            let info = fixture.chart.indicator_info(id).unwrap();
            assert_eq!(info.warmup_bars, starts[output], "{context}: warm-up");
            assert_eq!(
                info.convergence_bars,
                extra.map(|extra| starts[output] + extra),
                "{context}: convergence"
            );
            // The series begins at the declared row, and its first row is a value: the runtime
            // produces its first finite value exactly where the template says it starts.
            let (times, values) = fixture.chart.data.series_data(id).unwrap();
            assert_eq!(
                times[0], source_times[starts[output]],
                "{context}: first row"
            );
            assert_eq!(times.len(), START_ROWS - starts[output], "{context}: rows");
            assert_eq!(
                values[3].iter().position(|value| value.is_finite()),
                Some(0),
                "{context}: first finite row"
            );
        }
    }
}

#[test]
fn a_tick_over_pre_installed_session_slots_recolors_only_the_rows_it_rewrote() {
    // The color-ruled templates (VOL bars, MACD and AO columns, SAR dots) on a chart whose session
    // is pre-installed as whitespace slots: filling the forming slot must not walk the slots after
    // it. A sentinel color planted on the last slot survives the ticks, while the filled rows are
    // colored exactly as a full recoloring would.
    const SLOTS: usize = 30;
    let rows = ROWS + SLOTS;
    let data = {
        let mut data = Data::generate(rows, &[]);
        for slot in ROWS..rows {
            data.blank(slot);
        }
        data
    };
    let mut fixture = install(&data, &(0..rows).collect::<Vec<_>>());
    let mut colored = Vec::new();
    for name in ["VOL", "MACD", "AO", "SAR"] {
        let indicator = Indicator::from_name(name).unwrap();
        let (_, outputs) = bind(&mut fixture, &indicator);
        let binding = fixture.chart.indicators.len() - 1;
        for (index, &output) in outputs.iter().enumerate() {
            if klinechart_color_rule(&indicator, index).is_some() {
                colored.push((binding, output));
            }
        }
    }
    assert_eq!(
        colored.len(),
        4,
        "VOL, MACD, AO and SAR each color one output"
    );
    let sentinel = 0x0102_0304;
    let body = PointColorChannel::Body;
    let last_slot_row =
        |chart: &ChartEngine, output| chart.data.series_data(output).unwrap().1[3].len() - 1;
    for &(_, output) in &colored {
        let last = last_slot_row(&fixture.chart, output);
        assert!(
            fixture
                .chart
                .data
                .set_point_color(output, body, last, sentinel)
        );
    }

    // Fill the forming slot, revise it, and fill the next one.
    let mut filled = Data::generate(rows, &[]);
    for slot in ROWS + 2..rows {
        filled.blank(slot);
    }
    for row in [ROWS, ROWS, ROWS + 1] {
        feed(&mut fixture, &filled, row);
    }
    for &(_, output) in &colored {
        let last = last_slot_row(&fixture.chart, output);
        assert_eq!(
            fixture.chart.data.point_color(output, body, last),
            Some(sentinel),
            "a tick recolored a slot it did not rewrite"
        );
        assert!(
            fixture
                .chart
                .data
                .set_point_color(output, body, last, POINT_COLOR_ABSENT)
        );
    }
    for &(binding, _) in &colored {
        check(&fixture, binding);
    }
}
