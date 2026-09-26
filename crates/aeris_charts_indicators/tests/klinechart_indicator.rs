//! Checks [`klinechart::Indicator`], the bindable form of the KLineChart ports, against the same
//! KLineChart reference fixture as `klinechart_parity.rs`: every output under its KLineChart figure
//! key, the declared warm-up rows, the parameter round trips, and the incremental runtime.

use std::collections::{BTreeMap, HashMap};

use aeris_charts_indicators::klinechart::{Bars, Column, Figure, Indicator, Placement, NAMES};
use aeris_charts_indicators::{IncrementalState, IndicatorInput};
use serde::Deserialize;

#[derive(Deserialize)]
struct Fixture {
    datasets: HashMap<String, Dataset>,
    cases: Vec<Case>,
}

#[derive(Deserialize)]
struct Dataset {
    open: Vec<f64>,
    high: Vec<f64>,
    low: Vec<f64>,
    close: Vec<f64>,
    volume: Vec<f64>,
    turnover: Vec<f64>,
}

impl Dataset {
    fn bars(&self) -> Bars<'_> {
        Bars {
            open: &self.open,
            high: &self.high,
            low: &self.low,
            close: &self.close,
            volume: &self.volume,
            turnover: &self.turnover,
        }
    }
}

#[derive(Deserialize)]
struct Case {
    indicator: String,
    dataset: String,
    params: Vec<f64>,
    outputs: BTreeMap<String, Column>,
}

fn load() -> Fixture {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/klinechart_parity.json"
    );
    let text = std::fs::read_to_string(path).expect("read KLineChart parity fixture");
    serde_json::from_str(&text).expect("parse KLineChart parity fixture")
}

/// Fixture indicator names are KLineChart template names.
fn indicator(case: &Case) -> Indicator {
    Indicator::from_calc_params(&case.indicator, &case.params).unwrap_or_else(|| {
        panic!(
            "{} {:?} is a valid KLineChart parameter set",
            case.indicator, case.params
        )
    })
}

#[test]
fn every_output_matches_the_klinechart_figure_with_the_same_key() {
    let fixture = load();
    let mut compared = 0usize;
    for case in &fixture.cases {
        let indicator = indicator(case);
        let dataset = &fixture.datasets[&case.dataset];
        let columns = indicator.compute(&dataset.bars());
        let keys = indicator.output_keys();
        assert_eq!(
            columns.len(),
            indicator.output_count(),
            "{}",
            indicator.title()
        );
        assert_eq!(keys.len(), columns.len(), "{}", indicator.title());
        let actual: BTreeMap<String, &Column> = keys
            .iter()
            .map(|key| (*key).to_owned())
            .zip(&columns)
            .collect();
        assert_eq!(
            actual.keys().collect::<Vec<_>>(),
            case.outputs.keys().collect::<Vec<_>>(),
            "{} on {}",
            indicator.title(),
            case.dataset
        );
        for (key, expected) in &case.outputs {
            let got = actual[key];
            assert_eq!(got.len(), expected.len());
            for (row, (g, e)) in got.iter().zip(expected).enumerate() {
                assert_eq!(
                    g.map(f64::to_bits),
                    e.map(f64::to_bits),
                    "{} on {}: {key}[{row}]",
                    indicator.title(),
                    case.dataset
                );
                compared += 1;
            }
        }
    }
    assert!(compared > 40_000, "compared {compared} values");
}

#[test]
fn declared_warm_up_rows_are_where_each_output_starts() {
    let fixture = load();
    for case in &fixture.cases {
        let indicator = indicator(case);
        let dataset = &fixture.datasets[&case.dataset];
        let columns = indicator.compute(&dataset.bars());
        let starts = indicator.output_starts();
        for (index, column) in columns.iter().enumerate() {
            let first = column.iter().position(Option::is_some);
            let label = format!(
                "{} on {}: {}",
                indicator.title(),
                case.dataset,
                indicator.output_keys()[index]
            );
            match first {
                // AVP waits for the running volume to become non-zero, so its start is a bound.
                Some(first) if matches!(indicator, Indicator::Avp) => {
                    assert!(first >= starts[index], "{label}")
                }
                Some(first) => assert_eq!(first, starts[index], "{label}"),
                None => assert!(starts[index] >= column.len(), "{label} never starts"),
            }
        }
    }
}

#[test]
fn names_parameters_and_serialization_round_trip() {
    for name in NAMES {
        let indicator = Indicator::from_name(name).expect("every listed name is a template");
        assert_eq!(indicator.name(), name);
        assert!(indicator.is_valid(), "{name} defaults are valid");
        assert_eq!(
            Indicator::from_name(&name.to_ascii_lowercase()),
            Some(indicator.clone())
        );
        assert_eq!(
            Indicator::from_calc_params(name, &indicator.calc_params()),
            Some(indicator.clone()),
            "{name} calcParams round trip"
        );
        let count = indicator.output_count();
        assert!((1..=aeris_charts_indicators::MAX_OUTPUTS).contains(&count));
        assert_eq!(indicator.output_keys().len(), count, "{name}");
        assert_eq!(indicator.output_titles().len(), count, "{name}");
        assert_eq!(indicator.figures().len(), count, "{name}");
        assert_eq!(indicator.params().len(), indicator.calc_params().len());

        let json = serde_json::to_string(&indicator).expect("serialize");
        let back: Indicator = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, indicator, "{json}");
    }
    assert_eq!(NAMES.len(), 27);
    assert_eq!(Indicator::from_name("NOPE"), None);
}

#[test]
fn presentation_metadata_follows_klinechart_templates() {
    let macd = Indicator::from_name("MACD").unwrap();
    assert_eq!(macd.title(), "MACD(12,26,9)");
    assert_eq!(macd.output_keys(), ["dif", "dea", "macd"]);
    assert_eq!(macd.output_titles(), ["DIF", "DEA", "MACD"]);
    assert_eq!(macd.figures(), [Figure::Line, Figure::Line, Figure::Bar]);
    assert_eq!(macd.placement(), Placement::Pane);

    let ma = Indicator::from_name("MA").unwrap();
    assert_eq!(ma.title(), "MA(5,10,30,60)");
    assert_eq!(ma.output_titles(), ["MA5", "MA10", "MA30", "MA60"]);
    assert_eq!(ma.placement(), Placement::Price);

    let vol = Indicator::from_name("VOL").unwrap();
    assert_eq!(vol.output_keys(), ["volume", "ma1", "ma2", "ma3"]);
    assert_eq!(vol.output_titles(), ["VOLUME", "MA5", "MA10", "MA20"]);
    assert_eq!(vol.figures()[0], Figure::Bar);
    assert!(vol.needs_volume());

    let rsi = Indicator::from_name("RSI").unwrap();
    assert_eq!(rsi.output_titles(), ["RSI1", "RSI2", "RSI3"]);
    assert_eq!(
        Indicator::from_name("SAR").unwrap().figures(),
        [Figure::Circle]
    );
    assert_eq!(Indicator::from_name("SAR").unwrap().title(), "SAR(2,2,20)");
    assert_eq!(Indicator::from_name("PVT").unwrap().title(), "PVT");
    assert_eq!(Indicator::from_name("PVT").unwrap().missing_volume(), 1.0);
    assert_eq!(Indicator::from_name("EMV").unwrap().calc_params(), [14.0]);
    assert_eq!(
        Indicator::from_calc_params("EMV", &[14.0, 9.0]),
        Indicator::from_name("EMV")
    );
}

#[test]
fn invalid_parameters_are_rejected() {
    for (name, params) in [
        ("MA", &[][..]),
        ("MA", &[0.0][..]),
        ("MA", &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0][..]),
        ("VOL", &[1.0, 2.0, 3.0, 4.0, 5.0][..]),
        ("MACD", &[12.0, 26.0][..]),
        ("MACD", &[12.5, 26.0, 9.0][..]),
        ("BOLL", &[20.0, -1.0][..]),
        ("BOLL", &[20.0, f64::NAN][..]),
        ("SMA", &[12.0, 0.0][..]),
        ("SAR", &[2.0, 0.0, 20.0][..]),
        ("PVT", &[1.0][..]),
        ("EMV", &[14.0, 9.0, 1.0][..]),
        ("CR", &[26.0, 10.0, 20.0, 40.0][..]),
    ] {
        assert_eq!(
            Indicator::from_calc_params(name, params),
            None,
            "{name} {params:?}"
        );
    }
    assert!(Indicator::from_calc_params("MA", &[1.0, 2.0, 3.0, 4.0, 5.0]).is_some());
    assert!(Indicator::from_calc_params("VOL", &[1.0, 2.0, 3.0, 4.0]).is_some());
}

/// Appending bars one at a time and rebuilding only the new suffix yields the same series as one
/// full rebuild, the way the chart engine stitches incremental output.
#[test]
fn incremental_suffix_rebuilds_match_a_full_rebuild() {
    let fixture = load();
    let data = &fixture.datasets["random_walk"];
    let rows = 120;
    let times: Vec<i64> = (0..rows as i64).collect();
    let input = |n: usize| IndicatorInput {
        times: &times[..n],
        open: &data.open[..n],
        high: &data.high[..n],
        low: &data.low[..n],
        close: &data.close[..n],
        volume: &data.volume[..n],
    };
    for name in NAMES {
        let indicator = Indicator::from_name(name).unwrap();
        let mut full = IncrementalState::klinechart(indicator.clone());
        full.rebuild_from(input(rows), 0);
        let expected: Vec<(usize, Vec<f64>)> = (0..full.output_count())
            .map(|index| (full.output_from(index), full.output(index).to_vec()))
            .collect();

        let mut live = IncrementalState::klinechart(indicator.clone());
        // Each output is stored from its first emitted row, like an aligned engine series.
        let mut stitched: Vec<Option<(usize, Vec<f64>)>> = vec![None; live.output_count()];
        for n in 1..=rows {
            live.rebuild_from(input(n), n - 1);
            for (index, slot) in stitched.iter_mut().enumerate() {
                let from = live.output_from(index);
                let values = live.output(index);
                match slot {
                    None if values.is_empty() => {}
                    None => *slot = Some((from, values.to_vec())),
                    Some((start, stored)) => {
                        stored.truncate(from - *start);
                        stored.extend_from_slice(values);
                    }
                }
            }
        }
        for (index, (from, values)) in expected.iter().enumerate() {
            let (start, stored) = stitched[index]
                .clone()
                .expect("output starts within 120 rows");
            assert_eq!(start, *from, "{name} output {index} start");
            assert_eq!(
                stored.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
                values.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
                "{name} output {index}"
            );
        }
    }
}
