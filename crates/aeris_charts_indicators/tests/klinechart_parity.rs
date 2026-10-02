//! Bar-by-bar parity between `aeris_charts_indicators::klinechart` and KLineChart's own `calc`
//! functions.
//!
//! `fixtures/klinechart_parity.json` is produced by `tools/klinechart_parity/generate.ts`, which runs
//! the unmodified KLineChart v10.0.3 indicator templates over three datasets (a 360-row random walk
//! with rising, falling, flat, and zero-volume runs; a 12-row series shorter than most warm-ups; and
//! a constant series that exercises every zero-divisor branch), once with each template's default
//! parameters and once with a custom parameter set.
//!
//! The ports repeat KLineChart's arithmetic in the same order, so values must match exactly, and so
//! must the rows where a figure is unset.

use std::collections::{BTreeMap, HashMap};

use aeris_charts_indicators::klinechart::{self as klc, Column};
use serde::Deserialize;

#[derive(Deserialize)]
struct Fixture {
    klinechart: Source,
    datasets: HashMap<String, Dataset>,
    cases: Vec<Case>,
}

#[derive(Deserialize)]
struct Source {
    version: String,
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

fn periods(params: &[f64]) -> Vec<usize> {
    params.iter().map(|&p| p as usize).collect()
}

/// Names each column the way KLineChart names its figures (`ma1`, `ma2`, ...).
fn numbered(prefix: &str, columns: Vec<Column>) -> Vec<(String, Column)> {
    columns
        .into_iter()
        .enumerate()
        .map(|(i, column)| (format!("{prefix}{}", i + 1), column))
        .collect()
}

fn named<const N: usize>(pairs: [(&str, Column); N]) -> Vec<(String, Column)> {
    pairs
        .into_iter()
        .map(|(key, column)| (key.to_owned(), column))
        .collect()
}

/// Runs the Rust port for one fixture case and labels its outputs with KLineChart's figure keys.
fn compute(case: &Case, d: &Dataset) -> Vec<(String, Column)> {
    let p = &case.params;
    let n = |i: usize| p[i] as usize;
    match case.indicator.as_str() {
        "MA" => numbered("ma", klc::ma(&d.close, &periods(p))),
        "EMA" => numbered("ema", klc::ema(&d.close, &periods(p))),
        "SMA" => named([("sma", klc::sma(&d.close, n(0), p[1]))]),
        "BBI" => named([("bbi", klc::bbi(&d.close, &periods(p)))]),
        "VOL" => {
            let mut out = numbered("ma", klc::vol(&d.volume, &periods(p)));
            out.push(("volume".into(), d.volume.iter().map(|&v| Some(v)).collect()));
            out
        }
        "MACD" => {
            let r = klc::macd(&d.close, n(0), n(1), n(2));
            named([("dif", r.dif), ("dea", r.dea), ("macd", r.macd)])
        }
        "BOLL" => {
            let r = klc::boll(&d.close, n(0), p[1]);
            named([("up", r.up), ("mid", r.mid), ("dn", r.dn)])
        }
        "KDJ" => {
            let r = klc::kdj(&d.high, &d.low, &d.close, n(0), n(1), n(2));
            named([("k", r.k), ("d", r.d), ("j", r.j)])
        }
        "RSI" => numbered("rsi", klc::rsi(&d.close, &periods(p))),
        "BIAS" => numbered("bias", klc::bias(&d.close, &periods(p))),
        "BRAR" => {
            let r = klc::brar(&d.open, &d.high, &d.low, &d.close, n(0));
            named([("br", r.br), ("ar", r.ar)])
        }
        "CCI" => named([("cci", klc::cci(&d.high, &d.low, &d.close, n(0)))]),
        "CR" => {
            let r = klc::cr(&d.high, &d.low, n(0), [n(1), n(2), n(3), n(4)]);
            let [ma1, ma2, ma3, ma4] = r.ma;
            named([
                ("cr", r.cr),
                ("ma1", ma1),
                ("ma2", ma2),
                ("ma3", ma3),
                ("ma4", ma4),
            ])
        }
        "DMA" => {
            let r = klc::dma(&d.close, n(0), n(1), n(2));
            named([("dma", r.dma), ("ama", r.ama)])
        }
        "DMI" => {
            let r = klc::dmi(&d.high, &d.low, &d.close, n(0), n(1));
            named([
                ("pdi", r.pdi),
                ("mdi", r.mdi),
                ("adx", r.adx),
                ("adxr", r.adxr),
            ])
        }
        "EMV" => {
            let r = klc::emv(&d.high, &d.low, &d.volume, n(0));
            named([("emv", r.emv), ("maEmv", r.ma_emv)])
        }
        "MTM" => {
            let r = klc::mtm(&d.close, n(0), n(1));
            named([("mtm", r.mtm), ("maMtm", r.ma_mtm)])
        }
        "OBV" => {
            let r = klc::obv(&d.close, &d.volume, n(0));
            named([("obv", r.obv), ("maObv", r.ma_obv)])
        }
        "PVT" => named([("pvt", klc::pvt(&d.close, &d.volume))]),
        "PSY" => {
            let r = klc::psy(&d.close, n(0), n(1));
            named([("psy", r.psy), ("maPsy", r.ma_psy)])
        }
        "ROC" => {
            let r = klc::roc(&d.close, n(0), n(1));
            named([("roc", r.roc), ("maRoc", r.ma_roc)])
        }
        "SAR" => named([("sar", klc::sar(&d.high, &d.low, p[0], p[1], p[2]))]),
        "TRIX" => {
            let r = klc::trix(&d.close, n(0), n(1));
            named([("trix", r.trix), ("maTrix", r.ma_trix)])
        }
        "VR" => {
            let r = klc::vr(&d.close, &d.volume, n(0), n(1));
            named([("vr", r.vr), ("maVr", r.ma_vr)])
        }
        "WR" => numbered("wr", klc::wr(&d.high, &d.low, &d.close, &periods(p))),
        "AO" => named([("ao", klc::ao(&d.high, &d.low, n(0), n(1)))]),
        "AVP" => named([("avp", klc::avp(&d.volume, &d.turnover))]),
        other => panic!("fixture contains an indicator with no Rust port: {other}"),
    }
}

#[test]
fn every_klinechart_indicator_matches_the_reference_exactly() {
    let fixture = load();
    assert_eq!(fixture.klinechart.version, "10.0.3");

    let mut failures = Vec::new();
    let mut compared_values = 0usize;
    let mut indicators = std::collections::BTreeSet::new();
    for case in &fixture.cases {
        let dataset = &fixture.datasets[&case.dataset];
        let actual: BTreeMap<String, Column> = compute(case, dataset).into_iter().collect();
        indicators.insert(case.indicator.clone());
        let label = format!("{} {:?} on {}", case.indicator, case.params, case.dataset);
        let expected_keys: Vec<&String> = case.outputs.keys().collect();
        let actual_keys: Vec<&String> = actual.keys().collect();
        if expected_keys != actual_keys {
            failures.push(format!(
                "{label}: figures {actual_keys:?}, expected {expected_keys:?}"
            ));
            continue;
        }
        for (key, expected) in &case.outputs {
            let got = &actual[key];
            assert_eq!(got.len(), expected.len(), "{label}: {key} length");
            for (row, (g, e)) in got.iter().zip(expected).enumerate() {
                compared_values += 1;
                if g.map(f64::to_bits) != e.map(f64::to_bits) {
                    failures.push(format!("{label}: {key}[{row}] = {g:?}, expected {e:?}"));
                }
            }
        }
    }

    assert!(
        failures.is_empty(),
        "{} mismatches, first 20:\n{}",
        failures.len(),
        failures
            .iter()
            .take(20)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
    assert_eq!(
        indicators.len(),
        27,
        "every KLineChart indicator is covered"
    );
    assert!(
        compared_values > 40_000,
        "compared {compared_values} values"
    );
}

#[test]
fn zero_periods_return_unset_columns_instead_of_dividing_by_zero() {
    let close = [1.0, 2.0, 3.0];
    assert!(klc::ma(&close, &[0])[0].iter().all(Option::is_none));
    assert!(klc::macd(&close, 0, 2, 2).dif.iter().all(Option::is_none));
    assert!(klc::bbi(&close, &[1, 0, 2, 3]).iter().all(Option::is_none));
    assert!(klc::bbi(&close, &[]).iter().all(Option::is_none));
}
