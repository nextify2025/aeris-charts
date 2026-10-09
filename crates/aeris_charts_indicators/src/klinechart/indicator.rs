//! [`Indicator`]: one KLineChart indicator template together with its `calcParams`.
//!
//! The formula functions in this module take plain columns. `Indicator` is what a chart binds: it
//! names the template, holds its parameters, computes every output from one set of bars, and
//! describes how KLineChart presents the result — output keys and titles, whether each output is a
//! line, a bar, or a dot, which pane the indicator belongs in, and from which row each output can
//! first hold a value.

use serde::{Deserialize, Serialize};

use super::{
    Column, ao, avp, bbi, bias, boll, brar, cci, cr, current_ratio::forward_shift, dma, dmi, ema,
    emv, kdj, ma, macd, mtm, obv, psy, pvt, roc, rsi, sar, sma, trix, vol, vr, wr,
};
use crate::MAX_OUTPUTS;

/// The largest period [`Indicator::is_valid`] accepts.
pub const MAX_PERIOD: usize = 1_000_000;

/// KLineChart's indicator names, price-overlay indicators first, in the order a picker lists them.
pub const NAMES: [&str; 27] = [
    "MA", "EMA", "SMA", "BOLL", "SAR", "BBI", "AVP", "VOL", "MACD", "KDJ", "RSI", "BIAS", "BRAR",
    "CCI", "DMI", "CR", "PSY", "DMA", "TRIX", "OBV", "VR", "WR", "MTM", "EMV", "ROC", "PVT", "AO",
];

/// How KLineChart draws one output (its figure `type`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Figure {
    /// A line (`type: 'line'`), colored from KLineChart's line palette in figure order.
    Line,
    /// A column rising from zero (`type: 'bar'`), colored row by row.
    Bar,
    /// One dot per row (`type: 'circle'`), colored row by row.
    Circle,
}

/// Where KLineChart places an indicator by default.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placement {
    /// Over the candles, on the price axis (KLineChart `series: 'price'`).
    Price,
    /// In a pane of its own below the candles.
    Pane,
}

/// How an indicator's values are best formatted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValueFormat {
    /// The candles' own price format.
    Price,
    /// A fixed number of decimals.
    Decimals(u32),
    /// Large counts, abbreviated with K/M/B suffixes.
    Volume,
}

/// One named parameter, for settings editors.
#[derive(Clone, Debug, PartialEq)]
pub struct Param {
    pub name: String,
    pub value: f64,
    /// Whether the parameter is a whole-number period rather than a real number.
    pub integer: bool,
}

/// The bar columns an indicator reads. Every column holds one value per bar.
#[derive(Clone, Copy, Debug)]
pub struct Bars<'a> {
    pub open: &'a [f64],
    pub high: &'a [f64],
    pub low: &'a [f64],
    pub close: &'a [f64],
    pub volume: &'a [f64],
    /// Traded value per bar; only [`Indicator::Avp`] reads it.
    pub turnover: &'a [f64],
}

/// One KLineChart indicator with its parameters.
///
/// Variants are named after KLineChart's templates, and each variant's fields are that template's
/// `calcParams` in order. Serialized, the variant is an `indicator` tag beside its fields:
/// `{"indicator": "macd", "short": 12, "long": 26, "signal": 9}`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "indicator", rename_all = "snake_case")]
pub enum Indicator {
    /// `MA`: a rolling mean of close per period. Default `[5, 10, 30, 60]`.
    Ma { periods: Vec<usize> },
    /// `EMA`: an exponential moving average of close per period. Default `[6, 12, 20]`.
    Ema { periods: Vec<usize> },
    /// `SMA`: the weighted `SMA(CLOSE, N, M)` smoothing. Default `N = 12`, `M = 2`.
    Sma { period: usize, weight: f64 },
    /// `BBI`: the average of four moving averages. Default `[3, 6, 12, 24]`.
    Bbi { periods: [usize; 4] },
    /// `VOL`: volume bars plus a moving average of volume per period. Default `[5, 10, 20]`.
    Vol { periods: Vec<usize> },
    /// `MACD`. Default `12, 26, 9`.
    Macd {
        short: usize,
        long: usize,
        signal: usize,
    },
    /// `BOLL`. Default `20, 2`.
    Boll { period: usize, multiplier: f64 },
    /// `KDJ`. Default `9, 3, 3`.
    Kdj {
        period: usize,
        k_smoothing: usize,
        d_smoothing: usize,
    },
    /// `RSI`: one line per period. Default `[6, 12, 24]`.
    Rsi { periods: Vec<usize> },
    /// `BIAS`: one line per period. Default `[6, 12, 24]`.
    Bias { periods: Vec<usize> },
    /// `BRAR`. Default `26`.
    Brar { period: usize },
    /// `CCI`. Default `20`.
    Cci { period: usize },
    /// `CR`: the ratio plus four shifted moving averages of it. Default `26, [10, 20, 40, 60]`.
    Cr {
        period: usize,
        ma_periods: [usize; 4],
    },
    /// `DMA`. Default `10, 50, 10`.
    Dma {
        short: usize,
        long: usize,
        signal: usize,
    },
    /// `DMI`. Default `14, 6`.
    Dmi { period: usize, adxr_period: usize },
    /// `EMV`. Default `14`. KLineChart lists a second parameter, `9`, that its formula never reads.
    Emv { period: usize },
    /// `MTM`. Default `12, 6`.
    Mtm { period: usize, ma_period: usize },
    /// `OBV`. Default `30`.
    Obv { ma_period: usize },
    /// `PVT`. No parameters.
    Pvt,
    /// `PSY`. Default `12, 6`.
    Psy { period: usize, ma_period: usize },
    /// `ROC`. Default `12, 6`.
    Roc { period: usize, ma_period: usize },
    /// `SAR`: parabolic stop-and-reverse, with its factors in percent. Default `2, 2, 20`.
    Sar { start: f64, step: f64, max: f64 },
    /// `TRIX`. Default `12, 9`.
    Trix { period: usize, ma_period: usize },
    /// `VR`. Default `26, 6`.
    Vr { period: usize, ma_period: usize },
    /// `WR`: one line per period. Default `[6, 10, 14]`.
    Wr { periods: Vec<usize> },
    /// `AO`. Default `5, 34`.
    Ao { short: usize, long: usize },
    /// `AVP`: the running average traded price, `SUM(TURNOVER) / SUM(VOLUME)`. No parameters.
    Avp,
}

const MA_KEYS: [&str; MAX_OUTPUTS] = ["ma1", "ma2", "ma3", "ma4", "ma5"];
const EMA_KEYS: [&str; MAX_OUTPUTS] = ["ema1", "ema2", "ema3", "ema4", "ema5"];
const RSI_KEYS: [&str; MAX_OUTPUTS] = ["rsi1", "rsi2", "rsi3", "rsi4", "rsi5"];
const BIAS_KEYS: [&str; MAX_OUTPUTS] = ["bias1", "bias2", "bias3", "bias4", "bias5"];
const WR_KEYS: [&str; MAX_OUTPUTS] = ["wr1", "wr2", "wr3", "wr4", "wr5"];
const VOL_KEYS: [&str; MAX_OUTPUTS] = ["volume", "ma1", "ma2", "ma3", "ma4"];

impl Indicator {
    /// The template called `name` (KLineChart's name, in any letter case) with KLineChart's default
    /// parameters.
    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name.to_ascii_uppercase().as_str() {
            "MA" => Self::Ma {
                periods: vec![5, 10, 30, 60],
            },
            "EMA" => Self::Ema {
                periods: vec![6, 12, 20],
            },
            "SMA" => Self::Sma {
                period: 12,
                weight: 2.0,
            },
            "BBI" => Self::Bbi {
                periods: [3, 6, 12, 24],
            },
            "VOL" => Self::Vol {
                periods: vec![5, 10, 20],
            },
            "MACD" => Self::Macd {
                short: 12,
                long: 26,
                signal: 9,
            },
            "BOLL" => Self::Boll {
                period: 20,
                multiplier: 2.0,
            },
            "KDJ" => Self::Kdj {
                period: 9,
                k_smoothing: 3,
                d_smoothing: 3,
            },
            "RSI" => Self::Rsi {
                periods: vec![6, 12, 24],
            },
            "BIAS" => Self::Bias {
                periods: vec![6, 12, 24],
            },
            "BRAR" => Self::Brar { period: 26 },
            "CCI" => Self::Cci { period: 20 },
            "CR" => Self::Cr {
                period: 26,
                ma_periods: [10, 20, 40, 60],
            },
            "DMA" => Self::Dma {
                short: 10,
                long: 50,
                signal: 10,
            },
            "DMI" => Self::Dmi {
                period: 14,
                adxr_period: 6,
            },
            "EMV" => Self::Emv { period: 14 },
            "MTM" => Self::Mtm {
                period: 12,
                ma_period: 6,
            },
            "OBV" => Self::Obv { ma_period: 30 },
            "PVT" => Self::Pvt,
            "PSY" => Self::Psy {
                period: 12,
                ma_period: 6,
            },
            "ROC" => Self::Roc {
                period: 12,
                ma_period: 6,
            },
            "SAR" => Self::Sar {
                start: 2.0,
                step: 2.0,
                max: 20.0,
            },
            "TRIX" => Self::Trix {
                period: 12,
                ma_period: 9,
            },
            "VR" => Self::Vr {
                period: 26,
                ma_period: 6,
            },
            "WR" => Self::Wr {
                periods: vec![6, 10, 14],
            },
            "AO" => Self::Ao { short: 5, long: 34 },
            "AVP" => Self::Avp,
            _ => return None,
        })
    }

    /// The template called `name` with KLineChart-style `calcParams`. Periods must be whole numbers.
    /// Returns `None` for an unknown name, a parameter list of the wrong length, or parameters
    /// [`Self::is_valid`] rejects.
    pub fn from_calc_params(name: &str, params: &[f64]) -> Option<Self> {
        let template = Self::from_name(name)?;
        let whole = |value: f64| {
            (value.is_finite()
                && value.fract() == 0.0
                && (0.0..=MAX_PERIOD as f64).contains(&value))
            .then_some(value as usize)
        };
        let periods = |values: &[f64]| values.iter().map(|&v| whole(v)).collect::<Option<Vec<_>>>();
        let fixed = |count: usize| (params.len() == count).then(|| periods(params)).flatten();
        let indicator = match template {
            Self::Ma { .. } => Self::Ma {
                periods: periods(params)?,
            },
            Self::Ema { .. } => Self::Ema {
                periods: periods(params)?,
            },
            Self::Vol { .. } => Self::Vol {
                periods: periods(params)?,
            },
            Self::Rsi { .. } => Self::Rsi {
                periods: periods(params)?,
            },
            Self::Bias { .. } => Self::Bias {
                periods: periods(params)?,
            },
            Self::Wr { .. } => Self::Wr {
                periods: periods(params)?,
            },
            Self::Sma { .. } => match params {
                &[period, weight] => Self::Sma {
                    period: whole(period)?,
                    weight,
                },
                _ => return None,
            },
            Self::Bbi { .. } => {
                let p = fixed(4)?;
                Self::Bbi {
                    periods: [p[0], p[1], p[2], p[3]],
                }
            }
            Self::Macd { .. } => {
                let p = fixed(3)?;
                Self::Macd {
                    short: p[0],
                    long: p[1],
                    signal: p[2],
                }
            }
            Self::Boll { .. } => match params {
                &[period, multiplier] => Self::Boll {
                    period: whole(period)?,
                    multiplier,
                },
                _ => return None,
            },
            Self::Kdj { .. } => {
                let p = fixed(3)?;
                Self::Kdj {
                    period: p[0],
                    k_smoothing: p[1],
                    d_smoothing: p[2],
                }
            }
            Self::Brar { .. } => Self::Brar {
                period: fixed(1)?[0],
            },
            Self::Cci { .. } => Self::Cci {
                period: fixed(1)?[0],
            },
            Self::Cr { .. } => {
                let p = fixed(5)?;
                Self::Cr {
                    period: p[0],
                    ma_periods: [p[1], p[2], p[3], p[4]],
                }
            }
            Self::Dma { .. } => {
                let p = fixed(3)?;
                Self::Dma {
                    short: p[0],
                    long: p[1],
                    signal: p[2],
                }
            }
            Self::Dmi { .. } => {
                let p = fixed(2)?;
                Self::Dmi {
                    period: p[0],
                    adxr_period: p[1],
                }
            }
            // KLineChart's EMV declares `[14, 9]` but reads only the first value.
            Self::Emv { .. } => match params {
                [period] | [period, _] => Self::Emv {
                    period: whole(*period)?,
                },
                _ => return None,
            },
            Self::Mtm { .. } => {
                let p = fixed(2)?;
                Self::Mtm {
                    period: p[0],
                    ma_period: p[1],
                }
            }
            Self::Obv { .. } => Self::Obv {
                ma_period: fixed(1)?[0],
            },
            Self::Psy { .. } => {
                let p = fixed(2)?;
                Self::Psy {
                    period: p[0],
                    ma_period: p[1],
                }
            }
            Self::Roc { .. } => {
                let p = fixed(2)?;
                Self::Roc {
                    period: p[0],
                    ma_period: p[1],
                }
            }
            Self::Sar { .. } => match params {
                &[start, step, max] => Self::Sar { start, step, max },
                _ => return None,
            },
            Self::Trix { .. } => {
                let p = fixed(2)?;
                Self::Trix {
                    period: p[0],
                    ma_period: p[1],
                }
            }
            Self::Vr { .. } => {
                let p = fixed(2)?;
                Self::Vr {
                    period: p[0],
                    ma_period: p[1],
                }
            }
            Self::Ao { .. } => {
                let p = fixed(2)?;
                Self::Ao {
                    short: p[0],
                    long: p[1],
                }
            }
            Self::Pvt | Self::Avp => {
                if !params.is_empty() {
                    return None;
                }
                template
            }
        };
        indicator.is_valid().then_some(indicator)
    }

    /// KLineChart's name for the template: `"MA"`, `"MACD"`, ...
    pub fn name(&self) -> &'static str {
        match self {
            Self::Ma { .. } => "MA",
            Self::Ema { .. } => "EMA",
            Self::Sma { .. } => "SMA",
            Self::Bbi { .. } => "BBI",
            Self::Vol { .. } => "VOL",
            Self::Macd { .. } => "MACD",
            Self::Boll { .. } => "BOLL",
            Self::Kdj { .. } => "KDJ",
            Self::Rsi { .. } => "RSI",
            Self::Bias { .. } => "BIAS",
            Self::Brar { .. } => "BRAR",
            Self::Cci { .. } => "CCI",
            Self::Cr { .. } => "CR",
            Self::Dma { .. } => "DMA",
            Self::Dmi { .. } => "DMI",
            Self::Emv { .. } => "EMV",
            Self::Mtm { .. } => "MTM",
            Self::Obv { .. } => "OBV",
            Self::Pvt => "PVT",
            Self::Psy { .. } => "PSY",
            Self::Roc { .. } => "ROC",
            Self::Sar { .. } => "SAR",
            Self::Trix { .. } => "TRIX",
            Self::Vr { .. } => "VR",
            Self::Wr { .. } => "WR",
            Self::Ao { .. } => "AO",
            Self::Avp => "AVP",
        }
    }

    /// The parameters in KLineChart's `calcParams` order.
    pub fn calc_params(&self) -> Vec<f64> {
        self.params().into_iter().map(|param| param.value).collect()
    }

    /// The parameters with editor names, in KLineChart's `calcParams` order.
    pub fn params(&self) -> Vec<Param> {
        let int = |name: &str, value: usize| Param {
            name: name.to_owned(),
            value: value as f64,
            integer: true,
        };
        let real = |name: &str, value: f64| Param {
            name: name.to_owned(),
            value,
            integer: false,
        };
        let list = |periods: &[usize]| {
            periods
                .iter()
                .enumerate()
                .map(|(i, &period)| int(&format!("period_{}", i + 1), period))
                .collect()
        };
        match self {
            Self::Ma { periods }
            | Self::Ema { periods }
            | Self::Vol { periods }
            | Self::Rsi { periods }
            | Self::Bias { periods }
            | Self::Wr { periods } => list(periods),
            Self::Bbi { periods } => list(periods),
            Self::Sma { period, weight } => vec![int("period", *period), real("weight", *weight)],
            Self::Macd {
                short,
                long,
                signal,
            }
            | Self::Dma {
                short,
                long,
                signal,
            } => vec![
                int("short", *short),
                int("long", *long),
                int("signal", *signal),
            ],
            Self::Boll { period, multiplier } => {
                vec![int("period", *period), real("multiplier", *multiplier)]
            }
            Self::Kdj {
                period,
                k_smoothing,
                d_smoothing,
            } => vec![
                int("period", *period),
                int("k_smoothing", *k_smoothing),
                int("d_smoothing", *d_smoothing),
            ],
            Self::Brar { period } | Self::Cci { period } | Self::Emv { period } => {
                vec![int("period", *period)]
            }
            Self::Cr { period, ma_periods } => {
                let mut params = vec![int("period", *period)];
                params.extend(
                    ma_periods
                        .iter()
                        .enumerate()
                        .map(|(i, &m)| int(&format!("ma_period_{}", i + 1), m)),
                );
                params
            }
            Self::Dmi {
                period,
                adxr_period,
            } => vec![int("period", *period), int("adxr_period", *adxr_period)],
            Self::Mtm { period, ma_period }
            | Self::Psy { period, ma_period }
            | Self::Roc { period, ma_period }
            | Self::Trix { period, ma_period }
            | Self::Vr { period, ma_period } => {
                vec![int("period", *period), int("ma_period", *ma_period)]
            }
            Self::Obv { ma_period } => vec![int("ma_period", *ma_period)],
            Self::Sar { start, step, max } => vec![
                real("start", *start),
                real("step", *step),
                real("max", *max),
            ],
            Self::Ao { short, long } => vec![int("short", *short), int("long", *long)],
            Self::Pvt | Self::Avp => Vec::new(),
        }
    }

    /// KLineChart's tooltip name: the template name followed by its parameters, as in
    /// `MACD(12,26,9)`; just the name when there are none.
    pub fn title(&self) -> String {
        let params = self.calc_params();
        if params.is_empty() {
            return self.name().to_owned();
        }
        let params = params
            .iter()
            .map(|value| format!("{value}"))
            .collect::<Vec<_>>()
            .join(",");
        format!("{}({params})", self.name())
    }

    /// Whether the parameters can be computed and bound: every period in `1..=MAX_PERIOD`, one to
    /// five outputs, and finite real parameters in range (`SMA` weight and `SAR` factors positive,
    /// the `BOLL` multiplier not negative).
    pub fn is_valid(&self) -> bool {
        let period = |p: usize| (1..=MAX_PERIOD).contains(&p);
        let list = |periods: &[usize], max: usize| {
            !periods.is_empty() && periods.len() <= max && periods.iter().all(|&p| period(p))
        };
        match self {
            Self::Ma { periods }
            | Self::Ema { periods }
            | Self::Rsi { periods }
            | Self::Bias { periods }
            | Self::Wr { periods } => list(periods, MAX_OUTPUTS),
            // One output is the volume itself.
            Self::Vol { periods } => list(periods, MAX_OUTPUTS - 1),
            Self::Bbi { periods } => periods.iter().all(|&p| period(p)),
            Self::Sma { period: p, weight } => period(*p) && weight.is_finite() && *weight > 0.0,
            Self::Macd {
                short,
                long,
                signal,
            }
            | Self::Dma {
                short,
                long,
                signal,
            } => period(*short) && period(*long) && period(*signal),
            Self::Boll {
                period: p,
                multiplier,
            } => period(*p) && multiplier.is_finite() && *multiplier >= 0.0,
            Self::Kdj {
                period: p,
                k_smoothing,
                d_smoothing,
            } => period(*p) && period(*k_smoothing) && period(*d_smoothing),
            Self::Brar { period: p } | Self::Cci { period: p } | Self::Emv { period: p } => {
                period(*p)
            }
            Self::Cr {
                period: p,
                ma_periods,
            } => period(*p) && ma_periods.iter().all(|&m| period(m)),
            Self::Dmi {
                period: p,
                adxr_period,
            } => period(*p) && period(*adxr_period),
            Self::Mtm {
                period: p,
                ma_period,
            }
            | Self::Psy {
                period: p,
                ma_period,
            }
            | Self::Roc {
                period: p,
                ma_period,
            }
            | Self::Trix {
                period: p,
                ma_period,
            }
            | Self::Vr {
                period: p,
                ma_period,
            } => period(*p) && period(*ma_period),
            Self::Obv { ma_period } => period(*ma_period),
            Self::Sar { start, step, max } => [start, step, max]
                .iter()
                .all(|value| value.is_finite() && **value > 0.0),
            Self::Ao { short, long } => period(*short) && period(*long),
            Self::Pvt | Self::Avp => true,
        }
    }

    /// How many output columns [`Self::compute`] returns (at most [`MAX_OUTPUTS`] for a valid
    /// indicator).
    pub fn output_count(&self) -> usize {
        match self {
            Self::Ma { periods }
            | Self::Ema { periods }
            | Self::Rsi { periods }
            | Self::Bias { periods }
            | Self::Wr { periods } => periods.len(),
            Self::Vol { periods } => periods.len() + 1,
            Self::Sma { .. }
            | Self::Bbi { .. }
            | Self::Cci { .. }
            | Self::Pvt
            | Self::Sar { .. }
            | Self::Ao { .. }
            | Self::Avp => 1,
            Self::Brar { .. }
            | Self::Dma { .. }
            | Self::Emv { .. }
            | Self::Mtm { .. }
            | Self::Obv { .. }
            | Self::Psy { .. }
            | Self::Roc { .. }
            | Self::Trix { .. }
            | Self::Vr { .. } => 2,
            Self::Macd { .. } | Self::Boll { .. } | Self::Kdj { .. } => 3,
            Self::Dmi { .. } => 4,
            Self::Cr { .. } => 5,
        }
    }

    /// Stable machine keys for each output, in output order: KLineChart's figure keys (`dif`,
    /// `ma1`, `maObv`, ...). `VOL` lists its volume bars first so they paint beneath the averages.
    pub fn output_keys(&self) -> Vec<&'static str> {
        let count = self.output_count().min(MAX_OUTPUTS);
        let keys: &[&'static str] = match self {
            Self::Ma { .. } => &MA_KEYS,
            Self::Ema { .. } => &EMA_KEYS,
            Self::Rsi { .. } => &RSI_KEYS,
            Self::Bias { .. } => &BIAS_KEYS,
            Self::Wr { .. } => &WR_KEYS,
            Self::Vol { .. } => &VOL_KEYS,
            Self::Sma { .. } => &["sma"],
            Self::Bbi { .. } => &["bbi"],
            Self::Macd { .. } => &["dif", "dea", "macd"],
            Self::Boll { .. } => &["up", "mid", "dn"],
            Self::Kdj { .. } => &["k", "d", "j"],
            Self::Brar { .. } => &["br", "ar"],
            Self::Cci { .. } => &["cci"],
            Self::Cr { .. } => &["cr", "ma1", "ma2", "ma3", "ma4"],
            Self::Dma { .. } => &["dma", "ama"],
            Self::Dmi { .. } => &["pdi", "mdi", "adx", "adxr"],
            Self::Emv { .. } => &["emv", "maEmv"],
            Self::Mtm { .. } => &["mtm", "maMtm"],
            Self::Obv { .. } => &["obv", "maObv"],
            Self::Pvt => &["pvt"],
            Self::Psy { .. } => &["psy", "maPsy"],
            Self::Roc { .. } => &["roc", "maRoc"],
            Self::Sar { .. } => &["sar"],
            Self::Trix { .. } => &["trix", "maTrix"],
            Self::Vr { .. } => &["vr", "maVr"],
            Self::Ao { .. } => &["ao"],
            Self::Avp => &["avp"],
        };
        keys[..count.min(keys.len())].to_vec()
    }

    /// KLineChart's tooltip label for each output, in output order: `MA5`, `DIF`, `MAOBV`, ...
    pub fn output_titles(&self) -> Vec<String> {
        let numbered = |prefix: &str, values: &[usize]| {
            values
                .iter()
                .map(|value| format!("{prefix}{value}"))
                .collect::<Vec<_>>()
        };
        let ordinal = |prefix: &str, count: usize| {
            (1..=count)
                .map(|i| format!("{prefix}{i}"))
                .collect::<Vec<_>>()
        };
        let fixed = |titles: &[&str]| titles.iter().map(|t| (*t).to_owned()).collect::<Vec<_>>();
        let mut titles = match self {
            Self::Ma { periods } => numbered("MA", periods),
            Self::Ema { periods } => numbered("EMA", periods),
            Self::Bias { periods } => numbered("BIAS", periods),
            Self::Rsi { periods } => ordinal("RSI", periods.len()),
            Self::Wr { periods } => ordinal("WR", periods.len()),
            Self::Vol { periods } => {
                let mut titles = vec!["VOLUME".to_owned()];
                titles.extend(numbered("MA", periods));
                titles
            }
            Self::Sma { .. } => fixed(&["SMA"]),
            Self::Bbi { .. } => fixed(&["BBI"]),
            Self::Macd { .. } => fixed(&["DIF", "DEA", "MACD"]),
            Self::Boll { .. } => fixed(&["UP", "MID", "DN"]),
            Self::Kdj { .. } => fixed(&["K", "D", "J"]),
            Self::Brar { .. } => fixed(&["BR", "AR"]),
            Self::Cci { .. } => fixed(&["CCI"]),
            Self::Cr { .. } => fixed(&["CR", "MA1", "MA2", "MA3", "MA4"]),
            Self::Dma { .. } => fixed(&["DMA", "AMA"]),
            Self::Dmi { .. } => fixed(&["PDI", "MDI", "ADX", "ADXR"]),
            Self::Emv { .. } => fixed(&["EMV", "MAEMV"]),
            Self::Mtm { .. } => fixed(&["MTM", "MAMTM"]),
            Self::Obv { .. } => fixed(&["OBV", "MAOBV"]),
            Self::Pvt => fixed(&["PVT"]),
            Self::Psy { .. } => fixed(&["PSY", "MAPSY"]),
            Self::Roc { .. } => fixed(&["ROC", "MAROC"]),
            Self::Sar { .. } => fixed(&["SAR"]),
            Self::Trix { .. } => fixed(&["TRIX", "MATRIX"]),
            Self::Vr { .. } => fixed(&["VR", "MAVR"]),
            Self::Ao { .. } => fixed(&["AO"]),
            Self::Avp => fixed(&["AVP"]),
        };
        titles.truncate(MAX_OUTPUTS);
        titles
    }

    /// How KLineChart draws each output, in output order.
    pub fn figures(&self) -> Vec<Figure> {
        let count = self.output_count().min(MAX_OUTPUTS);
        (0..count)
            .map(|index| match (self, index) {
                (Self::Vol { .. }, 0) | (Self::Macd { .. }, 2) | (Self::Ao { .. }, 0) => {
                    Figure::Bar
                }
                (Self::Sar { .. }, 0) => Figure::Circle,
                _ => Figure::Line,
            })
            .collect()
    }

    /// Where KLineChart puts the indicator by default.
    pub fn placement(&self) -> Placement {
        match self {
            Self::Ma { .. }
            | Self::Ema { .. }
            | Self::Sma { .. }
            | Self::Bbi { .. }
            | Self::Boll { .. }
            | Self::Sar { .. }
            | Self::Avp => Placement::Price,
            _ => Placement::Pane,
        }
    }

    /// How the values are best formatted. Price overlays follow the candles; `VOL`, `OBV`, and
    /// `PVT` are volume-sized; `AVP` uses two decimals and every other indicator four, as in
    /// KLineChart.
    pub fn value_format(&self) -> ValueFormat {
        match self {
            Self::Ma { .. }
            | Self::Ema { .. }
            | Self::Sma { .. }
            | Self::Bbi { .. }
            | Self::Boll { .. }
            | Self::Sar { .. } => ValueFormat::Price,
            Self::Avp => ValueFormat::Decimals(2),
            Self::Vol { .. } | Self::Obv { .. } | Self::Pvt => ValueFormat::Volume,
            _ => ValueFormat::Decimals(4),
        }
    }

    /// Rows past an output's warm-up after which it reads the same wherever the loaded history
    /// begins, or `None` when it reads the whole loaded history. Windowed formulas need none; a
    /// window whose every term also reads the bar before it (BRAR, CR, PSY, VR) differs only on
    /// its first row, so it needs one. A recursive smoothing, a running total, or a path state
    /// keeps a trace of every earlier row. Every variant is listed, so a new indicator must decide.
    pub fn extra_convergence_rows(&self) -> Option<usize> {
        match self {
            Self::Ma { .. }
            | Self::Bbi { .. }
            | Self::Vol { .. }
            | Self::Boll { .. }
            | Self::Bias { .. }
            | Self::Cci { .. }
            | Self::Dma { .. }
            | Self::Emv { .. }
            | Self::Mtm { .. }
            | Self::Roc { .. }
            | Self::Wr { .. }
            | Self::Ao { .. } => Some(0),
            Self::Brar { .. } | Self::Cr { .. } | Self::Psy { .. } | Self::Vr { .. } => Some(1),
            Self::Ema { .. }
            | Self::Sma { .. }
            | Self::Macd { .. }
            | Self::Kdj { .. }
            | Self::Rsi { .. }
            | Self::Dmi { .. }
            | Self::Trix { .. }
            | Self::Obv { .. }
            | Self::Pvt
            | Self::Avp
            | Self::Sar { .. } => None,
        }
    }

    /// Whether the indicator reads volume (and, for `AVP`, turnover).
    pub fn needs_volume(&self) -> bool {
        matches!(
            self,
            Self::Vol { .. }
                | Self::Obv { .. }
                | Self::Pvt
                | Self::Emv { .. }
                | Self::Vr { .. }
                | Self::Avp
        )
    }

    /// The volume KLineChart assumes for a bar without one: 1 for `PVT`, 0 everywhere else.
    pub fn missing_volume(&self) -> f64 {
        if matches!(self, Self::Pvt) { 1.0 } else { 0.0 }
    }

    /// The first row at which each output can hold a value, in output order. It depends only on
    /// the parameters, so it is a lower bound for every data set; it is exact except for `AVP`,
    /// which stays unset while the running volume is zero.
    pub fn output_starts(&self) -> [usize; MAX_OUTPUTS] {
        let mut starts = [0; MAX_OUTPUTS];
        let warm = |period: usize| period.saturating_sub(1);
        let mut set = |values: &[usize]| {
            for (slot, &value) in starts.iter_mut().zip(values) {
                *slot = value;
            }
        };
        match self {
            Self::Ma { periods }
            | Self::Ema { periods }
            | Self::Bias { periods }
            | Self::Wr { periods } => set(&periods.iter().map(|&p| warm(p)).collect::<Vec<_>>()),
            Self::Rsi { periods } => set(periods),
            Self::Vol { periods } => {
                let mut values = vec![0];
                values.extend(periods.iter().map(|&p| warm(p)));
                set(&values);
            }
            Self::Sma { period, .. } | Self::Cci { period } => set(&[warm(*period)]),
            Self::Bbi { periods } => set(&[warm(periods.iter().copied().max().unwrap_or(0))]),
            Self::Macd {
                short,
                long,
                signal,
            } => {
                let line = warm(*short.max(long));
                let signal = line + warm(*signal);
                set(&[line, signal, signal]);
            }
            Self::Dma {
                short,
                long,
                signal,
            } => {
                let line = warm(*short.max(long));
                set(&[line, line + warm(*signal)]);
            }
            Self::Boll { period, .. } => set(&[warm(*period); 3]),
            Self::Kdj { period, .. } => set(&[warm(*period); 3]),
            Self::Brar { period } => set(&[warm(*period); 2]),
            Self::Cr { period, ma_periods } => {
                let mut values = vec![warm(*period)];
                values.extend(
                    ma_periods
                        .iter()
                        .map(|&m| period + m + forward_shift(m) - 2),
                );
                set(&values);
            }
            Self::Dmi {
                period,
                adxr_period,
            } => {
                let adx = warm(*period) + warm(*period);
                set(&[warm(*period), warm(*period), adx, adx + warm(*adxr_period)]);
            }
            Self::Emv { period } => set(&[1, (*period).max(1)]),
            Self::Mtm { period, ma_period } | Self::Roc { period, ma_period } => {
                set(&[*period, period + warm(*ma_period)]);
            }
            Self::Obv { ma_period } => set(&[0, warm(*ma_period)]),
            Self::Psy { period, ma_period } | Self::Vr { period, ma_period } => {
                set(&[warm(*period), warm(*period) + warm(*ma_period)]);
            }
            Self::Trix { period, ma_period } => {
                let trix = 3 * warm(*period);
                set(&[trix, trix + warm(*ma_period)]);
            }
            Self::Ao { short, long } => set(&[warm(*short.max(long))]),
            Self::Pvt | Self::Sar { .. } | Self::Avp => {}
        }
        starts
    }

    /// Computes every output over `bars`, in output order. Each column has one entry per bar;
    /// `None` marks a bar where KLineChart leaves that output unset. `bars.volume` must be as long
    /// as `bars.close` when [`Self::needs_volume`], and `bars.turnover` as well for `AVP`.
    pub fn compute(&self, bars: &Bars<'_>) -> Vec<Column> {
        let Bars {
            open,
            high,
            low,
            close,
            volume,
            turnover,
        } = *bars;
        let mut columns = match self {
            Self::Ma { periods } => ma(close, periods),
            Self::Ema { periods } => ema(close, periods),
            Self::Sma { period, weight } => vec![sma(close, *period, *weight)],
            Self::Bbi { periods } => vec![bbi(close, periods)],
            Self::Vol { periods } => {
                let mut columns = vec![volume.iter().map(|&v| Some(v)).collect()];
                columns.extend(vol(volume, periods));
                columns
            }
            Self::Macd {
                short,
                long,
                signal,
            } => {
                let out = macd(close, *short, *long, *signal);
                vec![out.dif, out.dea, out.macd]
            }
            Self::Boll { period, multiplier } => {
                let out = boll(close, *period, *multiplier);
                vec![out.up, out.mid, out.dn]
            }
            Self::Kdj {
                period,
                k_smoothing,
                d_smoothing,
            } => {
                let out = kdj(high, low, close, *period, *k_smoothing, *d_smoothing);
                vec![out.k, out.d, out.j]
            }
            Self::Rsi { periods } => rsi(close, periods),
            Self::Bias { periods } => bias(close, periods),
            Self::Brar { period } => {
                let out = brar(open, high, low, close, *period);
                vec![out.br, out.ar]
            }
            Self::Cci { period } => vec![cci(high, low, close, *period)],
            Self::Cr { period, ma_periods } => {
                let out = cr(high, low, *period, *ma_periods);
                let [ma1, ma2, ma3, ma4] = out.ma;
                vec![out.cr, ma1, ma2, ma3, ma4]
            }
            Self::Dma {
                short,
                long,
                signal,
            } => {
                let out = dma(close, *short, *long, *signal);
                vec![out.dma, out.ama]
            }
            Self::Dmi {
                period,
                adxr_period,
            } => {
                let out = dmi(high, low, close, *period, *adxr_period);
                vec![out.pdi, out.mdi, out.adx, out.adxr]
            }
            Self::Emv { period } => {
                let out = emv(high, low, volume, *period);
                vec![out.emv, out.ma_emv]
            }
            Self::Mtm { period, ma_period } => {
                let out = mtm(close, *period, *ma_period);
                vec![out.mtm, out.ma_mtm]
            }
            Self::Obv { ma_period } => {
                let out = obv(close, volume, *ma_period);
                vec![out.obv, out.ma_obv]
            }
            Self::Pvt => vec![pvt(close, volume)],
            Self::Psy { period, ma_period } => {
                let out = psy(close, *period, *ma_period);
                vec![out.psy, out.ma_psy]
            }
            Self::Roc { period, ma_period } => {
                let out = roc(close, *period, *ma_period);
                vec![out.roc, out.ma_roc]
            }
            Self::Sar { start, step, max } => vec![sar(high, low, *start, *step, *max)],
            Self::Trix { period, ma_period } => {
                let out = trix(close, *period, *ma_period);
                vec![out.trix, out.ma_trix]
            }
            Self::Vr { period, ma_period } => {
                let out = vr(close, volume, *period, *ma_period);
                vec![out.vr, out.ma_vr]
            }
            Self::Wr { periods } => wr(high, low, close, periods),
            Self::Ao { short, long } => vec![ao(high, low, *short, *long)],
            Self::Avp => vec![avp(volume, turnover)],
        };
        columns.truncate(MAX_OUTPUTS);
        columns
    }
}
