//! Engine-owned indicator producers.
//!
//! Indicators are bound to a source series and recomputed on source updates; their outputs are
//! ordinary engine series (`aeris_charts_indicators` holds the pure math). Extracted from `lib.rs`.

use super::*;
use std::borrow::Cow;

/// Scalar source selected by a study.  The aggregate sources are calculated from the source
/// bar's OHLC columns without changing the canonical source series or duplicating its storage.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IndicatorInputSource {
    Open,
    High,
    Low,
    #[default]
    Close,
    Hl2,
    Hlc3,
    Ohlc4,
    Hlcc4,
}

impl IndicatorInputSource {
    pub const ALL: [Self; 8] = [
        Self::Open,
        Self::High,
        Self::Low,
        Self::Close,
        Self::Hl2,
        Self::Hlc3,
        Self::Ohlc4,
        Self::Hlcc4,
    ];
}

/// Typed parameter kinds exposed to hosts when building study editors.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IndicatorParameterType {
    Integer,
    Number,
    Source,
    Series,
    /// One of the string values listed in [`IndicatorParameterDescriptor::choices`].
    Choice,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct IndicatorParameterDescriptor {
    pub name: String,
    pub parameter_type: IndicatorParameterType,
    pub default: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    /// Allowed values of a [`IndicatorParameterType::Choice`] parameter.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub choices: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct IndicatorOutputDescriptor {
    pub name: String,
    pub index: usize,
    pub supports_style: bool,
}

/// Persistable presentation state for one output of an indicator binding.
///
/// The series store remains the owner of the live style. This compact snapshot is exposed with
/// the binding contract so a host can persist and restore a study without reconstructing styles
/// from indicator kind heuristics.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct IndicatorOutputStyle {
    pub visible: bool,
    pub line_color: Option<String>,
    pub line_width: Option<f64>,
    pub line_style: u8,
    pub point_markers: bool,
    pub up_color: Option<String>,
    pub down_color: Option<String>,
    pub area_top_color: Option<String>,
    pub area_bottom_color: Option<String>,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct IndicatorSchema {
    pub revision: u32,
    pub kind: String,
    pub parameters: Vec<IndicatorParameterDescriptor>,
    pub outputs: Vec<IndicatorOutputDescriptor>,
}

pub const INDICATOR_SCHEMA_REVISION: u32 = 2;

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum IndicatorKind {
    Sma {
        period: usize,
    },
    Ema {
        period: usize,
        /// Seed convention. Documents written before it existed restore the SMA seed.
        #[serde(default)]
        seed: aeris_charts_indicators::IndicatorSeed,
    },
    Dema {
        period: usize,
        #[serde(default)]
        seed: aeris_charts_indicators::IndicatorSeed,
    },
    Tema {
        period: usize,
        #[serde(default)]
        seed: aeris_charts_indicators::IndicatorSeed,
    },
    Smma {
        period: usize,
    },
    Hma {
        period: usize,
    },
    Vwma {
        period: usize,
    },
    StandardDeviation {
        period: usize,
    },
    Cci {
        period: usize,
    },
    WilliamsR {
        period: usize,
    },
    StochasticRsi {
        rsi_period: usize,
        stochastic_period: usize,
    },
    Momentum {
        period: usize,
    },
    RateOfChange {
        period: usize,
    },
    Donchian {
        period: usize,
    },
    PivotPoints {
        variant: aeris_charts_indicators::PivotKind,
    },
    ZigZag {
        deviation_percent: f64,
    },
    Keltner {
        period: usize,
        multiplier: f64,
    },
    AdxDmi {
        period: usize,
    },
    ParabolicSar,
    SuperTrend {
        period: usize,
        multiplier: f64,
    },
    Ichimoku,
    EmaRibbon {
        periods: [usize; aeris_charts_indicators::MAX_OUTPUTS],
    },
    Bollinger {
        period: usize,
        deviation: f64,
        /// Standard-deviation estimator; older documents restore the population estimator.
        #[serde(default)]
        estimator: aeris_charts_indicators::DeviationEstimator,
    },
    Rsi {
        period: usize,
        #[serde(default)]
        seed: aeris_charts_indicators::IndicatorSeed,
    },
    Macd {
        fast: usize,
        slow: usize,
        signal: usize,
        #[serde(default)]
        seed: aeris_charts_indicators::IndicatorSeed,
        /// Histogram scale: 1 is `MACD - signal`, 2 is 通达信 `(DIF-DEA)*2`.
        #[serde(default = "default_histogram_multiplier")]
        histogram_multiplier: f64,
    },
    Stochastic {
        k_period: usize,
        d_period: usize,
    },
    Atr {
        period: usize,
    },
    Vwap,
    Obv,
    Cmf {
        period: usize,
    },
    Mfi {
        period: usize,
    },
    Volume {
        period: usize,
    },
    VwapBands {
        reset: aeris_charts_indicators::VwapReset,
        standard_deviation: f64,
        percent: f64,
    },
    Wma {
        period: usize,
    },
    /// KDJ: RSV over `period` rows, `K = SMA(RSV, k_smoothing, 1)`,
    /// `D = SMA(K, d_smoothing, 1)`, `J = 3K - 2D` (defaults 9/3/3).
    Kdj {
        period: usize,
        k_smoothing: usize,
        d_smoothing: usize,
        /// K/D start; documents written before it existed restore the textbook 50.
        #[serde(default)]
        seed: aeris_charts_indicators::KdjSeed,
    },
}

fn default_histogram_multiplier() -> f64 {
    1.0
}

impl IndicatorKind {
    /// Expand a convention preset into this definition's explicit parameters: seeds for EMA,
    /// DEMA, TEMA, MACD and RSI, the KDJ K/D start, the MACD histogram scale, and the Bollinger
    /// estimator. Periods
    /// and kinds without a convention-dependent parameter are unchanged, and the preset itself
    /// is not retained.
    pub fn with_convention(self, convention: aeris_charts_indicators::IndicatorConvention) -> Self {
        let convention_seed = convention.seed();
        match self {
            Self::Ema { period, .. } => Self::Ema {
                period,
                seed: convention_seed,
            },
            Self::Dema { period, .. } => Self::Dema {
                period,
                seed: convention_seed,
            },
            Self::Tema { period, .. } => Self::Tema {
                period,
                seed: convention_seed,
            },
            Self::Rsi { period, .. } => Self::Rsi {
                period,
                seed: convention_seed,
            },
            Self::Macd {
                fast, slow, signal, ..
            } => Self::Macd {
                fast,
                slow,
                signal,
                seed: convention_seed,
                histogram_multiplier: convention.macd_histogram_multiplier(),
            },
            Self::Bollinger {
                period, deviation, ..
            } => Self::Bollinger {
                period,
                deviation,
                estimator: convention.deviation_estimator(),
            },
            Self::Kdj {
                period,
                k_smoothing,
                d_smoothing,
                ..
            } => Self::Kdj {
                period,
                k_smoothing,
                d_smoothing,
                seed: convention.kdj_seed(),
            },
            other => other,
        }
    }
}

/// One live indicator producer's typed, runtime-independent definition.
///
/// Bindings are enumerated in creation order, which is also dependency order: an output must
/// exist before it can become a later binding's source. Hosts can therefore recreate bindings in
/// this order while remapping each old output identity to the newly returned output identity.
#[derive(Clone, Debug, PartialEq)]
pub struct IndicatorBindingInfo {
    /// Stable chart-local binding identity, equal to the first output identity.
    pub binding_id: SeriesId,
    pub kind: IndicatorKind,
    pub source: SeriesId,
    pub source_input: IndicatorInputSource,
    /// Parallel volume column source for VWAP; `None` means unit weights.
    pub volume_source: Option<SeriesId>,
    /// Turnover column for an amount-weighted VWAP (`sum(amount) / sum(volume)`).
    pub amount_source: Option<SeriesId>,
    /// Output identities in the indicator's documented order.
    pub outputs: Vec<SeriesId>,
    /// Per-output presentation snapshots in the same order as `outputs`.
    pub styles: Vec<IndicatorOutputStyle>,
}

#[derive(Clone, Debug)]
pub(crate) struct IndicatorBinding {
    pub(crate) source: SeriesId,
    pub(crate) source_input: IndicatorInputSource,
    pub(crate) kind: IndicatorKind,
    pub(crate) outputs: Vec<SeriesId>,
    /// Parallel volume column source (VWAP); `None` = unit weights.
    pub(crate) volume_source: Option<SeriesId>,
    /// Turnover column of an amount-weighted VWAP, aligned by timestamp like volume.
    pub(crate) amount_source: Option<SeriesId>,
    runtime: aeris_charts_indicators::IncrementalState,
    source_generation: u64,
    volume_generation: Option<u64>,
    amount_generation: Option<u64>,
}

#[derive(Clone, Copy)]
pub(crate) struct IndicatorChange {
    pub(crate) from: usize,
    pub(crate) previous_generation: u64,
    pub(crate) full_replace: bool,
}

/// Stretch factor of the pane a separate-pane indicator creates for itself (the public reference
/// oscillators stack as a shorter strip under the price pane).
pub(crate) const OSCILLATOR_PANE_STRETCH: f64 = 0.3;

pub const EMA_RIBBON_DEFAULT_PERIODS: [usize; aeris_charts_indicators::MAX_OUTPUTS] =
    [5, 10, 20, 50, 200];
pub const EMA_RIBBON_DEFAULT_COLORS: [&str; aeris_charts_indicators::MAX_OUTPUTS] =
    ["#335cff", "#FF9800", "#7d52f4", "#fb4ba3", "#fb3748"];

/// MACD histogram four-state palette: strong when moving away from zero, weak when falling
/// back toward it (industry-standard). Packed `0xRRGGBBAA`.
const MACD_UP: u32 = rgb_u32(aeris_charts_core::style::MARKET_UP_RGB, 0xff);
const MACD_UP_WEAK: u32 = rgb_u32(
    aeris_charts_core::style::MARKET_UP_RGB,
    aeris_charts_core::style::MARKET_VOLUME_ALPHA,
);
const MACD_DOWN: u32 = rgb_u32(aeris_charts_core::style::MARKET_DOWN_RGB, 0xff);
const MACD_DOWN_WEAK: u32 = rgb_u32(
    aeris_charts_core::style::MARKET_DOWN_RGB,
    aeris_charts_core::style::MARKET_VOLUME_ALPHA,
);

const fn rgb_u32(rgb: (u8, u8, u8), alpha: u8) -> u32 {
    (rgb.0 as u32) << 24 | (rgb.1 as u32) << 16 | (rgb.2 as u32) << 8 | alpha as u32
}

/// An indicator output series' lineage: which binding it belongs to (kind + params), the
/// source series it derives from, and which output slot it is (Bollinger: 0 = upper,
/// 1 = middle, 2 = lower; SMA/EMA: always 0). Platforms read this to render their own
/// indicator chrome (legend chips, counts, settings) without the engine owning any UI.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct IndicatorInfo {
    /// Stable identity shared by every output of this binding. The first output's opaque series
    /// identity is safe because output identities are monotonic and the binding owns all outputs.
    pub binding_id: SeriesId,
    pub kind: &'static str,
    pub parameters: IndicatorParameters,
    pub period: usize,
    pub deviation: Option<f64>,
    pub source: SeriesId,
    pub source_input: IndicatorInputSource,
    pub volume_source: Option<SeriesId>,
    /// Turnover source of an amount-weighted VWAP.
    pub amount_source: Option<SeriesId>,
    /// Current engine-owned presentation state for this output.
    pub style: IndicatorOutputStyle,
    pub output_name: &'static str,
    pub output_index: usize,
    pub output_count: usize,
    /// Rows of the root (non-indicator) source before this output's first value, counting
    /// every chained indicator source, on a whitespace-free source.
    pub warmup_bars: usize,
    /// Recommended rows of root-source history before this output stops depending on where the
    /// loaded history begins: the warm-up for windowed formulas, plus the rows for every
    /// recursive seed's weight to fall below 0.1%, summed over chained sources. `None` when no
    /// row count suffices (time-anchored VWAP/pivots, cumulative or path-dependent formulas).
    pub convergence_bars: Option<usize>,
}

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize)]
pub struct IndicatorParameters {
    pub period: Option<usize>,
    pub periods: Option<[usize; aeris_charts_indicators::MAX_OUTPUTS]>,
    pub pivot_kind: Option<aeris_charts_indicators::PivotKind>,
    pub deviation_percent: Option<f64>,
    pub deviation: Option<f64>,
    pub fast: Option<usize>,
    pub slow: Option<usize>,
    pub signal: Option<usize>,
    pub k_period: Option<usize>,
    pub d_period: Option<usize>,
    pub reset: Option<aeris_charts_indicators::VwapReset>,
    pub standard_deviation: Option<f64>,
    pub percent: Option<f64>,
    pub seed: Option<aeris_charts_indicators::IndicatorSeed>,
    pub histogram_multiplier: Option<f64>,
    pub estimator: Option<aeris_charts_indicators::DeviationEstimator>,
    pub k_smoothing: Option<usize>,
    pub d_smoothing: Option<usize>,
    /// KDJ K/D start (the `seed` field of a KDJ definition).
    pub kdj_seed: Option<aeris_charts_indicators::KdjSeed>,
}

impl ChartEngine {
    pub(crate) fn reset_indicator_output_styles_to_defaults(&mut self) {
        let outputs = self
            .indicators
            .iter()
            .map(|binding| (binding.kind.clone(), binding.outputs.clone()))
            .collect::<Vec<_>>();
        for (kind, ids) in outputs {
            for (output_index, id) in ids.into_iter().enumerate() {
                let Some(series) = self.series_entry_mut(id) else {
                    continue;
                };
                series.countdown_visible = false;
                series.title_visible = true;
                series.line_width = Some(2.0);
                series.line_color = indicator_output_color(&kind, output_index).map(str::to_string);
            }
        }
    }

    /// Applies Aeris's canonical four-state momentum palette to an existing histogram series.
    ///
    /// Colors are derived from each value's sign and whether it moved toward or away from zero.
    /// The caller owns only the semantic request; Aeris retains palette and row-style ownership.
    pub fn apply_momentum_histogram_colors(&mut self, id: SeriesId) -> bool {
        if self
            .series_entry(id)
            .is_none_or(|series| series.kind != SeriesKind::Histogram)
        {
            return false;
        }
        let Some(colors) = self
            .data
            .series_data(id)
            .map(|(_, values)| momentum_histogram_colors(values[3]))
        else {
            return false;
        };
        self.set_series_point_colors(id, Some(colors), None, None)
    }

    pub(crate) fn indicator_memory_usage(&self) -> (usize, usize) {
        self.indicators.iter().fold((0, 0), |usage, binding| {
            (
                usage.0 + binding.runtime.runtime_bytes(),
                usage.1 + binding.runtime.transfer_capacity_bytes(),
            )
        })
    }

    pub fn last_indicator_work_rows(&self) -> usize {
        self.indicators
            .iter()
            .map(|binding| binding.runtime.last_work_rows())
            .sum()
    }

    /// Return one typed definition for each live indicator binding in creation/dependency order.
    pub fn indicator_bindings(&self) -> Vec<IndicatorBindingInfo> {
        self.indicators
            .iter()
            .map(|binding| IndicatorBindingInfo {
                binding_id: binding.outputs[0],
                kind: binding.kind.clone(),
                source: binding.source,
                source_input: binding.source_input,
                volume_source: binding.volume_source,
                amount_source: binding.amount_source,
                outputs: binding.outputs.clone(),
                styles: binding
                    .outputs
                    .iter()
                    .filter_map(|&id| self.series_entry(id).map(indicator_output_style))
                    .collect(),
            })
            .collect()
    }

    /// Replace one output's presentation atomically while retaining the binding and output id.
    /// Invalid widths are rejected before any series state is changed.
    pub fn set_indicator_output_style(
        &mut self,
        output: SeriesId,
        style: IndicatorOutputStyle,
    ) -> bool {
        if !style
            .line_width
            .is_none_or(|width| width.is_finite() && width > 0.0)
            || style.line_style > 4
            || !style_color_is_valid(style.line_color.as_deref())
            || !style_color_is_valid(style.up_color.as_deref())
            || !style_color_is_valid(style.down_color.as_deref())
            || !style_color_is_valid(style.area_top_color.as_deref())
            || !style_color_is_valid(style.area_bottom_color.as_deref())
        {
            return false;
        }
        if !self
            .indicators
            .iter()
            .any(|binding| binding.outputs.contains(&output))
        {
            return false;
        }
        let Some(series) = self.series_entry_mut(output) else {
            return false;
        };
        series.visible = style.visible;
        series.line_color = style.line_color;
        series.line_width = style.line_width;
        series.line_style = style.line_style;
        series.point_markers = style.point_markers;
        series.up_color = style.up_color;
        series.down_color = style.down_color;
        series.area_top_color = style.area_top_color;
        series.area_bottom_color = style.area_bottom_color;
        true
    }

    /// The binding an output series belongs to, or `None` when `id` is not an indicator output
    /// (a plain series, an unknown/removed id, or a source series itself).
    pub fn indicator_info(&self, id: SeriesId) -> Option<IndicatorInfo> {
        self.indicators.iter().find_map(|binding| {
            binding
                .outputs
                .iter()
                .position(|&output| output == id)
                .map(|output_index| {
                    let (kind, period, deviation, parameters) = match binding.kind {
                        IndicatorKind::Sma { period } => (
                            "sma",
                            period,
                            None,
                            IndicatorParameters {
                                period: Some(period),
                                ..IndicatorParameters::default()
                            },
                        ),
                        IndicatorKind::Ema { period, seed } => (
                            "ema",
                            period,
                            None,
                            IndicatorParameters {
                                period: Some(period),
                                seed: Some(seed),
                                ..IndicatorParameters::default()
                            },
                        ),
                        IndicatorKind::Dema { period, seed } => (
                            "dema",
                            period,
                            None,
                            IndicatorParameters {
                                period: Some(period),
                                seed: Some(seed),
                                ..IndicatorParameters::default()
                            },
                        ),
                        IndicatorKind::Tema { period, seed } => (
                            "tema",
                            period,
                            None,
                            IndicatorParameters {
                                period: Some(period),
                                seed: Some(seed),
                                ..IndicatorParameters::default()
                            },
                        ),
                        IndicatorKind::Smma { period } => (
                            "smma",
                            period,
                            None,
                            IndicatorParameters {
                                period: Some(period),
                                ..IndicatorParameters::default()
                            },
                        ),
                        IndicatorKind::Hma { period } => (
                            "hma",
                            period,
                            None,
                            IndicatorParameters {
                                period: Some(period),
                                ..IndicatorParameters::default()
                            },
                        ),
                        IndicatorKind::Vwma { period } => (
                            "vwma",
                            period,
                            None,
                            IndicatorParameters {
                                period: Some(period),
                                ..IndicatorParameters::default()
                            },
                        ),
                        IndicatorKind::StandardDeviation { period } => (
                            "standard_deviation",
                            period,
                            None,
                            IndicatorParameters {
                                period: Some(period),
                                ..IndicatorParameters::default()
                            },
                        ),
                        IndicatorKind::Cci { period } => (
                            "cci",
                            period,
                            None,
                            IndicatorParameters {
                                period: Some(period),
                                ..IndicatorParameters::default()
                            },
                        ),
                        IndicatorKind::WilliamsR { period } => (
                            "williams_r",
                            period,
                            None,
                            IndicatorParameters {
                                period: Some(period),
                                ..IndicatorParameters::default()
                            },
                        ),
                        IndicatorKind::StochasticRsi {
                            rsi_period,
                            stochastic_period,
                        } => (
                            "stochastic_rsi",
                            rsi_period,
                            Some(stochastic_period as f64),
                            IndicatorParameters {
                                period: Some(rsi_period),
                                k_period: Some(stochastic_period),
                                ..IndicatorParameters::default()
                            },
                        ),
                        IndicatorKind::Momentum { period } => (
                            "momentum",
                            period,
                            None,
                            IndicatorParameters {
                                period: Some(period),
                                ..IndicatorParameters::default()
                            },
                        ),
                        IndicatorKind::RateOfChange { period } => (
                            "roc",
                            period,
                            None,
                            IndicatorParameters {
                                period: Some(period),
                                ..IndicatorParameters::default()
                            },
                        ),
                        IndicatorKind::Donchian { period } => (
                            "donchian",
                            period,
                            None,
                            IndicatorParameters {
                                period: Some(period),
                                ..IndicatorParameters::default()
                            },
                        ),
                        IndicatorKind::PivotPoints { variant } => (
                            "pivot_points",
                            0,
                            None,
                            IndicatorParameters {
                                pivot_kind: Some(variant),
                                ..IndicatorParameters::default()
                            },
                        ),
                        IndicatorKind::ZigZag { deviation_percent } => (
                            "zigzag",
                            0,
                            None,
                            IndicatorParameters {
                                deviation_percent: Some(deviation_percent),
                                ..IndicatorParameters::default()
                            },
                        ),
                        IndicatorKind::Keltner { period, multiplier } => (
                            "keltner",
                            period,
                            Some(multiplier),
                            IndicatorParameters {
                                period: Some(period),
                                deviation: Some(multiplier),
                                ..IndicatorParameters::default()
                            },
                        ),
                        IndicatorKind::AdxDmi { period } => (
                            "adx_dmi",
                            period,
                            None,
                            IndicatorParameters {
                                period: Some(period),
                                ..IndicatorParameters::default()
                            },
                        ),
                        IndicatorKind::ParabolicSar => {
                            ("parabolic_sar", 0, None, IndicatorParameters::default())
                        }
                        IndicatorKind::SuperTrend { period, multiplier } => (
                            "supertrend",
                            period,
                            Some(multiplier),
                            IndicatorParameters {
                                period: Some(period),
                                deviation: Some(multiplier),
                                ..IndicatorParameters::default()
                            },
                        ),
                        IndicatorKind::Ichimoku => {
                            ("ichimoku", 0, None, IndicatorParameters::default())
                        }
                        IndicatorKind::EmaRibbon { periods } => (
                            "ema_ribbon",
                            periods[output_index],
                            None,
                            IndicatorParameters {
                                periods: Some(periods),
                                ..IndicatorParameters::default()
                            },
                        ),
                        IndicatorKind::Bollinger {
                            period,
                            deviation,
                            estimator,
                        } => (
                            "bollinger",
                            period,
                            Some(deviation),
                            IndicatorParameters {
                                period: Some(period),
                                deviation: Some(deviation),
                                estimator: Some(estimator),
                                ..IndicatorParameters::default()
                            },
                        ),
                        IndicatorKind::Rsi { period, seed } => (
                            "rsi",
                            period,
                            None,
                            IndicatorParameters {
                                period: Some(period),
                                seed: Some(seed),
                                ..IndicatorParameters::default()
                            },
                        ),
                        // MACD/Stochastic pack their second period into `deviation`.
                        IndicatorKind::Macd {
                            fast,
                            slow,
                            signal,
                            seed,
                            histogram_multiplier,
                        } => (
                            "macd",
                            slow,
                            Some(signal as f64),
                            IndicatorParameters {
                                fast: Some(fast),
                                slow: Some(slow),
                                signal: Some(signal),
                                seed: Some(seed),
                                histogram_multiplier: Some(histogram_multiplier),
                                ..IndicatorParameters::default()
                            },
                        ),
                        IndicatorKind::Stochastic { k_period, d_period } => (
                            "stochastic",
                            k_period,
                            Some(d_period as f64),
                            IndicatorParameters {
                                k_period: Some(k_period),
                                d_period: Some(d_period),
                                ..IndicatorParameters::default()
                            },
                        ),
                        IndicatorKind::Atr { period } => (
                            "atr",
                            period,
                            None,
                            IndicatorParameters {
                                period: Some(period),
                                ..IndicatorParameters::default()
                            },
                        ),
                        IndicatorKind::Vwap => ("vwap", 0, None, IndicatorParameters::default()),
                        IndicatorKind::Obv => ("obv", 0, None, IndicatorParameters::default()),
                        IndicatorKind::Cmf { period } => (
                            "cmf",
                            period,
                            None,
                            IndicatorParameters {
                                period: Some(period),
                                ..IndicatorParameters::default()
                            },
                        ),
                        IndicatorKind::Mfi { period } => (
                            "mfi",
                            period,
                            None,
                            IndicatorParameters {
                                period: Some(period),
                                ..IndicatorParameters::default()
                            },
                        ),
                        IndicatorKind::Volume { period } => (
                            "volume",
                            period,
                            None,
                            IndicatorParameters {
                                period: Some(period),
                                ..IndicatorParameters::default()
                            },
                        ),
                        IndicatorKind::VwapBands {
                            reset,
                            standard_deviation,
                            percent,
                        } => (
                            "vwap_bands",
                            0,
                            None,
                            IndicatorParameters {
                                reset: Some(reset),
                                standard_deviation: Some(standard_deviation),
                                percent: Some(percent),
                                ..IndicatorParameters::default()
                            },
                        ),
                        IndicatorKind::Wma { period } => (
                            "wma",
                            period,
                            None,
                            IndicatorParameters {
                                period: Some(period),
                                ..IndicatorParameters::default()
                            },
                        ),
                        IndicatorKind::Kdj {
                            period,
                            k_smoothing,
                            d_smoothing,
                            seed,
                        } => (
                            "kdj",
                            period,
                            None,
                            IndicatorParameters {
                                period: Some(period),
                                k_smoothing: Some(k_smoothing),
                                d_smoothing: Some(d_smoothing),
                                kdj_seed: Some(seed),
                                ..IndicatorParameters::default()
                            },
                        ),
                    };
                    let (warmup_bars, convergence_bars) = self.indicator_output_warmup(id);
                    IndicatorInfo {
                        binding_id: binding.outputs[0],
                        kind,
                        parameters,
                        period,
                        deviation,
                        source: binding.source,
                        source_input: binding.source_input,
                        volume_source: binding.volume_source,
                        amount_source: binding.amount_source,
                        style: self
                            .series_entry(binding.outputs[output_index])
                            .map(indicator_output_style)
                            .unwrap_or_default(),
                        output_name: indicator_output_name(&binding.kind, output_index),
                        output_index,
                        output_count: binding.outputs.len(),
                        warmup_bars,
                        convergence_bars,
                    }
                })
        })
    }

    /// `(warmup, convergence)` rows of root-source history for an indicator output, following
    /// chained indicator sources back to their plain series. A plain series needs none.
    fn indicator_output_warmup(&self, id: SeriesId) -> (usize, Option<usize>) {
        // Bindings are topological (a source output always precedes its consumer), so this walk
        // visits each binding at most once.
        let mut warmup = 0_usize;
        let mut convergence = Some(0_usize);
        let mut current = id;
        while let Some((binding, output_index)) = self.indicators.iter().find_map(|binding| {
            binding
                .outputs
                .iter()
                .position(|&output| output == current)
                .map(|index| (binding, index))
        }) {
            warmup = warmup.saturating_add(binding.runtime.warmup_rows(output_index));
            convergence = convergence
                .zip(binding.runtime.convergence_rows(output_index))
                .map(|(total, own)| total.saturating_add(own));
            current = binding.source;
        }
        (warmup, convergence)
    }

    /// Add a Rust-native simple moving-average producer. The returned line series is owned by the
    /// engine and is recomputed whenever its source series changes.
    pub fn add_sma(&mut self, source: SeriesId, period: usize) -> Option<SeriesId> {
        self.add_indicator_kind(source, IndicatorKind::Sma { period }, None)
            .into_iter()
            .next()
    }

    /// Add a Rust-native exponential moving-average producer.
    pub fn add_ema(&mut self, source: SeriesId, period: usize) -> Option<SeriesId> {
        self.add_indicator_kind(
            source,
            IndicatorKind::Ema {
                period,
                seed: IndicatorSeed::Sma,
            },
            None,
        )
        .into_iter()
        .next()
    }

    pub fn add_dema(&mut self, source: SeriesId, period: usize) -> Option<SeriesId> {
        self.add_indicator_kind(
            source,
            IndicatorKind::Dema {
                period,
                seed: IndicatorSeed::Sma,
            },
            None,
        )
        .into_iter()
        .next()
    }

    pub fn add_tema(&mut self, source: SeriesId, period: usize) -> Option<SeriesId> {
        self.add_indicator_kind(
            source,
            IndicatorKind::Tema {
                period,
                seed: IndicatorSeed::Sma,
            },
            None,
        )
        .into_iter()
        .next()
    }

    pub fn add_smma(&mut self, source: SeriesId, period: usize) -> Option<SeriesId> {
        self.add_indicator_kind(source, IndicatorKind::Smma { period }, None)
            .into_iter()
            .next()
    }

    pub fn add_rma(&mut self, source: SeriesId, period: usize) -> Option<SeriesId> {
        self.add_smma(source, period)
    }

    pub fn add_hma(&mut self, source: SeriesId, period: usize) -> Option<SeriesId> {
        self.add_indicator_kind(source, IndicatorKind::Hma { period }, None)
            .into_iter()
            .next()
    }

    pub fn add_vwma(
        &mut self,
        source: SeriesId,
        volume_source: Option<SeriesId>,
        period: usize,
    ) -> Option<SeriesId> {
        self.add_indicator_kind(source, IndicatorKind::Vwma { period }, volume_source)
            .into_iter()
            .next()
    }

    pub fn add_standard_deviation(&mut self, source: SeriesId, period: usize) -> Option<SeriesId> {
        self.add_indicator_kind(source, IndicatorKind::StandardDeviation { period }, None)
            .into_iter()
            .next()
    }

    pub fn add_cci(&mut self, source: SeriesId, period: usize) -> Option<SeriesId> {
        self.add_indicator_kind(source, IndicatorKind::Cci { period }, None)
            .into_iter()
            .next()
    }

    pub fn add_williams_r(&mut self, source: SeriesId, period: usize) -> Option<SeriesId> {
        self.add_indicator_kind(source, IndicatorKind::WilliamsR { period }, None)
            .into_iter()
            .next()
    }

    pub fn add_stochastic_rsi(
        &mut self,
        source: SeriesId,
        rsi_period: usize,
        stochastic_period: usize,
    ) -> Option<SeriesId> {
        self.add_indicator_kind(
            source,
            IndicatorKind::StochasticRsi {
                rsi_period,
                stochastic_period,
            },
            None,
        )
        .into_iter()
        .next()
    }

    pub fn add_momentum(&mut self, source: SeriesId, period: usize) -> Option<SeriesId> {
        self.add_indicator_kind(source, IndicatorKind::Momentum { period }, None)
            .into_iter()
            .next()
    }

    pub fn add_roc(&mut self, source: SeriesId, period: usize) -> Option<SeriesId> {
        self.add_indicator_kind(source, IndicatorKind::RateOfChange { period }, None)
            .into_iter()
            .next()
    }

    pub fn add_donchian(&mut self, source: SeriesId, period: usize) -> Vec<SeriesId> {
        self.add_indicator_kind(source, IndicatorKind::Donchian { period }, None)
    }

    pub fn add_pivot_points(
        &mut self,
        source: SeriesId,
        kind: aeris_charts_indicators::PivotKind,
    ) -> Vec<SeriesId> {
        self.add_indicator_kind(source, IndicatorKind::PivotPoints { variant: kind }, None)
    }

    pub fn add_zigzag(&mut self, source: SeriesId, deviation_percent: f64) -> SeriesId {
        self.add_indicator_kind(source, IndicatorKind::ZigZag { deviation_percent }, None)[0]
    }

    pub fn add_keltner(
        &mut self,
        source: SeriesId,
        period: usize,
        multiplier: f64,
    ) -> Vec<SeriesId> {
        self.add_indicator_kind(source, IndicatorKind::Keltner { period, multiplier }, None)
    }

    pub fn add_adx_dmi(&mut self, source: SeriesId, period: usize) -> Vec<SeriesId> {
        self.add_indicator_kind(source, IndicatorKind::AdxDmi { period }, None)
    }

    pub fn add_parabolic_sar(&mut self, source: SeriesId) -> Option<SeriesId> {
        self.add_indicator_kind(source, IndicatorKind::ParabolicSar, None)
            .into_iter()
            .next()
    }

    pub fn add_supertrend(
        &mut self,
        source: SeriesId,
        period: usize,
        multiplier: f64,
    ) -> Option<SeriesId> {
        self.add_indicator_kind(
            source,
            IndicatorKind::SuperTrend { period, multiplier },
            None,
        )
        .into_iter()
        .next()
    }

    pub fn add_ichimoku(&mut self, source: SeriesId) -> Vec<SeriesId> {
        self.add_indicator_kind(source, IndicatorKind::Ichimoku, None)
    }

    /// Add five exponential moving averages as one binding in fastest-to-slowest output order.
    pub fn add_ema_ribbon(
        &mut self,
        source: SeriesId,
        periods: [usize; aeris_charts_indicators::MAX_OUTPUTS],
    ) -> Vec<SeriesId> {
        self.add_indicator_kind(source, IndicatorKind::EmaRibbon { periods }, None)
    }

    /// Atomically update all periods of an EMA ribbon while retaining its output identities and
    /// presentation options. `id` may identify any output in the ribbon.
    pub fn set_ema_ribbon_periods(
        &mut self,
        id: SeriesId,
        periods: [usize; aeris_charts_indicators::MAX_OUTPUTS],
    ) -> bool {
        if periods.contains(&0) {
            return false;
        }
        let Some(index) = self.indicators.iter().position(|binding| {
            matches!(binding.kind, IndicatorKind::EmaRibbon { .. }) && binding.outputs.contains(&id)
        }) else {
            return false;
        };
        let IndicatorKind::EmaRibbon { periods: previous } = self.indicators[index].kind else {
            unreachable!("binding kind checked above")
        };
        if periods == previous {
            return true;
        }

        let outputs = self.indicators[index].outputs.clone();
        for (output_index, &output) in outputs.iter().enumerate() {
            let previous_title = format!("EMA {}", previous[output_index]);
            if let Some(series) = self.series.iter_mut().find(|series| series.id == output) {
                if series.title == previous_title {
                    series.title = format!("EMA {}", periods[output_index]);
                }
            }
        }
        let kind = IndicatorKind::EmaRibbon { periods };
        self.indicators[index].kind = kind.clone();
        self.indicators[index].runtime = incremental_state(&kind);
        let changes = self.rebuild_indicator(index, 0, true);
        self.indicator_changes.clear();
        self.indicator_changes.extend(changes.into_iter().flatten());
        self.propagate_indicator_changes();
        self.sync_time_points();
        true
    }

    /// Add upper, middle, and lower Bollinger-band line series in that order.
    pub fn add_bollinger(
        &mut self,
        source: SeriesId,
        period: usize,
        deviation: f64,
    ) -> Vec<SeriesId> {
        self.add_indicator_kind(
            source,
            IndicatorKind::Bollinger {
                period,
                deviation,
                estimator: DeviationEstimator::Population,
            },
            None,
        )
    }

    /// Add a Wilder RSI line in its own oscillator pane (with dotted 30/70 band lines).
    pub fn add_rsi(&mut self, source: SeriesId, period: usize) -> Option<SeriesId> {
        self.add_indicator_kind(
            source,
            IndicatorKind::Rsi {
                period,
                seed: IndicatorSeed::Sma,
            },
            None,
        )
        .into_iter()
        .next()
    }

    /// Add MACD line, signal line, and histogram series in that order, in their own
    /// oscillator pane. The histogram is a Histogram-kind series whose per-bar color follows
    /// four conventional states (strong/weak × above/below zero).
    pub fn add_macd(
        &mut self,
        source: SeriesId,
        fast: usize,
        slow: usize,
        signal: usize,
    ) -> Vec<SeriesId> {
        self.add_indicator_kind(
            source,
            IndicatorKind::Macd {
                fast,
                slow,
                signal,
                seed: IndicatorSeed::Sma,
                histogram_multiplier: 1.0,
            },
            None,
        )
    }

    /// Add Stochastic %K and %D lines in that order, in their own oscillator pane (with
    /// dotted 20/80 band lines).
    pub fn add_stochastic(
        &mut self,
        source: SeriesId,
        k_period: usize,
        d_period: usize,
    ) -> Vec<SeriesId> {
        self.add_indicator_kind(
            source,
            IndicatorKind::Stochastic { k_period, d_period },
            None,
        )
    }

    /// Add a Wilder ATR line in its own oscillator pane.
    pub fn add_atr(&mut self, source: SeriesId, period: usize) -> Option<SeriesId> {
        self.add_indicator_kind(source, IndicatorKind::Atr { period }, None)
            .into_iter()
            .next()
    }

    /// Add a session-anchored VWAP line on the source's pane, reset at each exchange trading day
    /// (the UTC day unless the chart sets a time zone or session start).
    /// `volume_source` supplies the per-bar volume column (its close slot); `None` = unit
    /// weights.
    pub fn add_vwap(
        &mut self,
        source: SeriesId,
        volume_source: Option<SeriesId>,
    ) -> Option<SeriesId> {
        self.add_indicator_kind(source, IndicatorKind::Vwap, volume_source)
            .into_iter()
            .next()
    }

    /// Add on-balance volume in its own oscillator pane. `volume_source` is required and supplies
    /// the per-bar volume column used by the cumulative direction signal.
    pub fn add_obv(&mut self, source: SeriesId, volume_source: SeriesId) -> Option<SeriesId> {
        self.add_indicator_kind(source, IndicatorKind::Obv, Some(volume_source))
            .into_iter()
            .next()
    }

    /// Add Chaikin money flow in its own oscillator pane.
    pub fn add_cmf(
        &mut self,
        source: SeriesId,
        volume_source: SeriesId,
        period: usize,
    ) -> Option<SeriesId> {
        self.add_indicator_kind(source, IndicatorKind::Cmf { period }, Some(volume_source))
            .into_iter()
            .next()
    }

    /// Add money flow index in its own oscillator pane.
    pub fn add_mfi(
        &mut self,
        source: SeriesId,
        volume_source: SeriesId,
        period: usize,
    ) -> Option<SeriesId> {
        self.add_indicator_kind(source, IndicatorKind::Mfi { period }, Some(volume_source))
            .into_iter()
            .next()
    }

    /// Add an engine-owned volume histogram and moving-average line in one pane.
    pub fn add_volume(
        &mut self,
        source: SeriesId,
        volume_source: SeriesId,
        period: usize,
    ) -> Vec<SeriesId> {
        self.add_indicator_kind(
            source,
            IndicatorKind::Volume { period },
            Some(volume_source),
        )
    }

    /// Add session/weekly/monthly VWAP basis, standard-deviation and percentage bands.
    pub fn add_vwap_bands(
        &mut self,
        source: SeriesId,
        volume_source: Option<SeriesId>,
        reset: aeris_charts_indicators::VwapReset,
        standard_deviation: f64,
        percent: f64,
    ) -> Vec<SeriesId> {
        self.add_indicator_kind(
            source,
            IndicatorKind::VwapBands {
                reset,
                standard_deviation,
                percent,
            },
            volume_source,
        )
    }

    /// Add a weighted moving-average line (linear weights, recent heaviest) on the source's pane.
    pub fn add_wma(&mut self, source: SeriesId, period: usize) -> Option<SeriesId> {
        self.add_indicator_kind(source, IndicatorKind::Wma { period }, None)
            .into_iter()
            .next()
    }

    /// Add KDJ K, D and J lines in that order, in their own oscillator pane (dotted 20/80 band
    /// lines). K and D start from the textbook 50; `IndicatorKind::Kdj { seed, .. }` selects the
    /// formula-language first value instead.
    pub fn add_kdj(
        &mut self,
        source: SeriesId,
        period: usize,
        k_smoothing: usize,
        d_smoothing: usize,
    ) -> Vec<SeriesId> {
        self.add_indicator_kind(
            source,
            IndicatorKind::Kdj {
                period,
                k_smoothing,
                d_smoothing,
                seed: aeris_charts_indicators::KdjSeed::Fifty,
            },
            None,
        )
    }

    /// Add the 分时 average-price line: a VWAP of `sum(amount) / sum(volume)` per reset period
    /// over the source's rows. Both columns align to the source by exact timestamp; rows whose
    /// amount or volume is missing, whitespace or non-positive volume contribute nothing.
    pub fn add_vwap_with_amount(
        &mut self,
        source: SeriesId,
        volume_source: SeriesId,
        amount_source: SeriesId,
    ) -> Option<SeriesId> {
        self.add_indicator_kind_with_sources(
            source,
            IndicatorInputSource::Close,
            IndicatorKind::Vwap,
            Some(volume_source),
            Some(amount_source),
        )
        .into_iter()
        .next()
    }

    /// Add an indicator from its typed definition, applying the same output, pane, and chrome
    /// defaults as the specialized convenience methods. Invalid definitions return no outputs and
    /// leave the chart unchanged.
    pub fn add_indicator_kind(
        &mut self,
        source: SeriesId,
        kind: IndicatorKind,
        volume_source: Option<SeriesId>,
    ) -> Vec<SeriesId> {
        self.add_indicator_kind_with_input(source, IndicatorInputSource::Close, kind, volume_source)
    }

    /// Add an indicator with an explicit scalar source. Existing convenience methods use close;
    /// this typed path is used for hlc3/hl2 and other study-input selections.
    pub fn add_indicator_kind_with_input(
        &mut self,
        source: SeriesId,
        source_input: IndicatorInputSource,
        kind: IndicatorKind,
        volume_source: Option<SeriesId>,
    ) -> Vec<SeriesId> {
        self.add_indicator_kind_with_sources(source, source_input, kind, volume_source, None)
    }

    /// Add an indicator with every input explicit. `amount_source` is accepted only by VWAP,
    /// which then also requires `volume_source`, and must be a distinct scalar series.
    pub fn add_indicator_kind_with_sources(
        &mut self,
        source: SeriesId,
        source_input: IndicatorInputSource,
        kind: IndicatorKind,
        volume_source: Option<SeriesId>,
        amount_source: Option<SeriesId>,
    ) -> Vec<SeriesId> {
        let ids = self.add_indicator(
            source,
            source_input,
            kind.clone(),
            volume_source,
            amount_source,
        );
        match kind {
            IndicatorKind::Rsi { .. } => {
                if !ids.is_empty() {
                    self.place_outputs_in_oscillator_pane(&ids);
                }
            }
            IndicatorKind::Macd { .. } => {
                if let Some(&histogram) = ids.get(2) {
                    self.convert_series_kind(histogram, SeriesKind::Histogram);
                    self.place_outputs_in_oscillator_pane(&ids);
                }
            }
            IndicatorKind::Stochastic { .. } | IndicatorKind::Kdj { .. } => {
                if !ids.is_empty() {
                    self.place_outputs_in_oscillator_pane(&ids);
                }
            }
            IndicatorKind::Atr { .. } => {
                if !ids.is_empty() {
                    self.place_outputs_in_oscillator_pane(&ids);
                }
            }
            IndicatorKind::AdxDmi { .. } => {
                if !ids.is_empty() {
                    self.place_outputs_in_oscillator_pane(&ids);
                }
            }
            IndicatorKind::Cci { .. } => {
                if !ids.is_empty() {
                    self.place_outputs_in_oscillator_pane(&ids);
                }
            }
            IndicatorKind::WilliamsR { .. } => {
                if !ids.is_empty() {
                    self.place_outputs_in_oscillator_pane(&ids);
                }
            }
            IndicatorKind::StochasticRsi { .. } => {
                if !ids.is_empty() {
                    self.place_outputs_in_oscillator_pane(&ids);
                }
            }
            IndicatorKind::Momentum { .. } => {
                if !ids.is_empty() {
                    self.place_outputs_in_oscillator_pane(&ids);
                }
            }
            IndicatorKind::RateOfChange { .. } => {
                if !ids.is_empty() {
                    self.place_outputs_in_oscillator_pane(&ids);
                }
            }
            IndicatorKind::Obv => {
                if !ids.is_empty() {
                    self.place_outputs_in_oscillator_pane(&ids);
                }
            }
            IndicatorKind::Cmf { .. } => {
                if !ids.is_empty() {
                    self.place_outputs_in_oscillator_pane(&ids);
                }
            }
            IndicatorKind::Mfi { .. } => {
                if !ids.is_empty() {
                    self.place_outputs_in_oscillator_pane(&ids);
                }
            }
            IndicatorKind::Volume { .. } => {
                if let Some(&histogram) = ids.first() {
                    self.convert_series_kind(histogram, SeriesKind::Histogram);
                    self.place_outputs_in_oscillator_pane(&ids);
                }
            }
            IndicatorKind::Sma { .. }
            | IndicatorKind::Ema { .. }
            | IndicatorKind::Dema { .. }
            | IndicatorKind::Tema { .. }
            | IndicatorKind::Smma { .. }
            | IndicatorKind::Hma { .. }
            | IndicatorKind::Vwma { .. }
            | IndicatorKind::StandardDeviation { .. }
            | IndicatorKind::Donchian { .. }
            | IndicatorKind::PivotPoints { .. }
            | IndicatorKind::ZigZag { .. }
            | IndicatorKind::Keltner { .. }
            | IndicatorKind::EmaRibbon { .. }
            | IndicatorKind::Bollinger { .. }
            | IndicatorKind::ParabolicSar
            | IndicatorKind::SuperTrend { .. }
            | IndicatorKind::Ichimoku
            | IndicatorKind::Vwap
            | IndicatorKind::VwapBands { .. }
            | IndicatorKind::Wma { .. } => {}
        }
        ids
    }

    /// Change the scalar source of an existing binding while retaining all output identities.
    /// The owner rebuilds from the source and propagates the revision to chained studies.
    pub fn set_indicator_input_source(
        &mut self,
        output: SeriesId,
        source_input: IndicatorInputSource,
    ) -> bool {
        let Some(index) = self
            .indicators
            .iter()
            .position(|binding| binding.outputs.contains(&output))
        else {
            return false;
        };
        if self.indicators[index].source_input == source_input {
            return true;
        }
        let kind = self.indicators[index].kind.clone();
        self.indicators[index].source_input = source_input;
        self.indicators[index].runtime = incremental_state(&kind);
        self.indicator_changes.clear();
        let changes = self.rebuild_indicator(index, 0, true);
        self.indicator_changes.extend(changes.into_iter().flatten());
        self.propagate_indicator_changes();
        self.sync_time_points();
        true
    }

    /// Return the bounded typed editor schema for an indicator definition.
    pub fn indicator_schema(kind: &IndicatorKind) -> IndicatorSchema {
        let descriptor =
            |name: &str,
             parameter_type: IndicatorParameterType,
             default: serde_json::Value,
             range: Option<(f64, f64)>| IndicatorParameterDescriptor {
                name: name.into(),
                parameter_type,
                default,
                min: range.map(|range| range.0),
                max: range.map(|range| range.1),
                choices: Vec::new(),
            };
        let mut parameters = vec![descriptor(
            "source",
            IndicatorParameterType::Source,
            serde_json::json!(IndicatorInputSource::Close),
            None,
        )];
        let integer = |name: &str, default: usize| {
            descriptor(
                name,
                IndicatorParameterType::Integer,
                serde_json::json!(default),
                Some((1.0, 1_000_000.0)),
            )
        };
        let number = |name: &str, default: f64| {
            descriptor(
                name,
                IndicatorParameterType::Number,
                serde_json::json!(default),
                Some((0.0, 1_000_000.0)),
            )
        };
        let series = |name: &str| {
            descriptor(
                name,
                IndicatorParameterType::Series,
                serde_json::Value::Null,
                None,
            )
        };
        let choice = |name: &str, default: serde_json::Value, choices: &[&str]| {
            IndicatorParameterDescriptor {
                choices: choices.iter().map(|choice| (*choice).to_string()).collect(),
                ..descriptor(name, IndicatorParameterType::Choice, default, None)
            }
        };
        let seed = |value: aeris_charts_indicators::IndicatorSeed| {
            choice("seed", serde_json::json!(value), &["sma", "first_value"])
        };
        match *kind {
            IndicatorKind::Sma { period }
            | IndicatorKind::Smma { period }
            | IndicatorKind::Hma { period }
            | IndicatorKind::StandardDeviation { period }
            | IndicatorKind::Cci { period }
            | IndicatorKind::WilliamsR { period }
            | IndicatorKind::Donchian { period }
            | IndicatorKind::Atr { period }
            | IndicatorKind::Wma { period } => parameters.push(integer("period", period)),
            IndicatorKind::Ema {
                period,
                seed: value,
            }
            | IndicatorKind::Dema {
                period,
                seed: value,
            }
            | IndicatorKind::Tema {
                period,
                seed: value,
            }
            | IndicatorKind::Rsi {
                period,
                seed: value,
            } => {
                parameters.push(integer("period", period));
                parameters.push(seed(value));
            }
            IndicatorKind::PivotPoints { variant } => {
                parameters.push(integer("kind", pivot_kind_index(variant)))
            }
            IndicatorKind::ZigZag { deviation_percent } => {
                parameters.push(number("deviation_percent", deviation_percent))
            }
            IndicatorKind::EmaRibbon { periods } => {
                for (index, period) in periods.into_iter().enumerate() {
                    parameters.push(integer(&format!("period_{}", index + 1), period));
                }
            }
            IndicatorKind::Bollinger {
                period,
                deviation,
                estimator,
            } => {
                parameters.push(integer("period", period));
                parameters.push(number("deviation", deviation));
                parameters.push(choice(
                    "estimator",
                    serde_json::json!(estimator),
                    &["population", "sample"],
                ));
            }
            IndicatorKind::Keltner { period, multiplier } => {
                parameters.push(integer("period", period));
                parameters.push(number("multiplier", multiplier));
            }
            IndicatorKind::StochasticRsi {
                rsi_period,
                stochastic_period,
            } => {
                parameters.push(integer("rsi_period", rsi_period));
                parameters.push(integer("stochastic_period", stochastic_period));
            }
            IndicatorKind::Momentum { period } | IndicatorKind::RateOfChange { period } => {
                parameters.push(integer("period", period));
            }
            IndicatorKind::AdxDmi { period } => parameters.push(integer("period", period)),
            IndicatorKind::ParabolicSar => {}
            IndicatorKind::SuperTrend { period, multiplier } => {
                parameters.push(integer("period", period));
                parameters.push(number("multiplier", multiplier));
            }
            IndicatorKind::Ichimoku => {}
            IndicatorKind::Macd {
                fast,
                slow,
                signal,
                seed: value,
                histogram_multiplier,
            } => {
                parameters.push(integer("fast", fast));
                parameters.push(integer("slow", slow));
                parameters.push(integer("signal", signal));
                parameters.push(seed(value));
                parameters.push(number("histogram_multiplier", histogram_multiplier));
            }
            IndicatorKind::Stochastic { k_period, d_period } => {
                parameters.push(integer("k_period", k_period));
                parameters.push(integer("d_period", d_period));
            }
            IndicatorKind::Kdj {
                period,
                k_smoothing,
                d_smoothing,
                seed,
            } => {
                parameters.push(integer("period", period));
                parameters.push(integer("k_smoothing", k_smoothing));
                parameters.push(integer("d_smoothing", d_smoothing));
                parameters.push(choice(
                    "seed",
                    serde_json::json!(seed),
                    &["fifty", "first_value"],
                ));
            }
            IndicatorKind::Vwap => {
                parameters.push(series("volume_source"));
                parameters.push(series("amount_source"));
            }
            IndicatorKind::Obv => parameters.push(series("volume_source")),
            IndicatorKind::Cmf { period }
            | IndicatorKind::Mfi { period }
            | IndicatorKind::Volume { period }
            | IndicatorKind::Vwma { period } => {
                parameters.push(integer("period", period));
                parameters.push(series("volume_source"));
            }
            IndicatorKind::VwapBands {
                reset,
                standard_deviation,
                percent,
            } => {
                parameters.push(descriptor(
                    "reset",
                    IndicatorParameterType::Source,
                    serde_json::json!(reset),
                    None,
                ));
                parameters.push(number("standard_deviation", standard_deviation));
                parameters.push(number("percent", percent));
                parameters.push(series("volume_source"));
            }
        }
        let output_count = incremental_state(kind).output_count();
        IndicatorSchema {
            revision: INDICATOR_SCHEMA_REVISION,
            kind: indicator_kind_name(kind).into(),
            parameters,
            outputs: (0..output_count)
                .map(|index| IndicatorOutputDescriptor {
                    name: indicator_output_name(kind, index).into(),
                    index,
                    supports_style: true,
                })
                .collect(),
        }
    }

    /// Move output series into a fresh oscillator pane below everything (the public reference
    /// separate-pane default, reduced stretch).
    fn place_outputs_in_oscillator_pane(&mut self, ids: &[SeriesId]) {
        let restored_pane = self.study_restore_pane_cursor.take().and_then(|index| {
            if index > 0 && index < self.panes.len() {
                self.study_restore_pane_cursor = Some(index + 1);
                Some(index)
            } else {
                None
            }
        });
        let Some(pane) = restored_pane.or_else(|| self.add_pane(false)) else {
            return;
        };
        if let Some(p) = self.panes.get_mut(pane) {
            p.stretch_factor = OSCILLATOR_PANE_STRETCH;
        }
        for &id in ids {
            self.set_series_pane(id, pane, OSCILLATOR_PANE_STRETCH);
        }
    }

    /// The bollinger band-fill companion for an output series: when `id` is a bollinger UPPER
    /// (output slot 0), the LOWER series (slot 2) the fill closes toward, else `None`. The
    /// frame builder paints the fill between them under the band strokes (the public reference's
    /// background fill).
    pub(crate) fn bollinger_fill_companion(&self, id: SeriesId) -> Option<SeriesId> {
        self.indicators.iter().find_map(|binding| {
            if matches!(binding.kind, IndicatorKind::Bollinger { .. })
                && binding.outputs.first() == Some(&id)
            {
                binding.outputs.get(2).copied()
            } else {
                None
            }
        })
    }

    /// Drop every indicator binding that reads from or writes to `id`, returning the output series
    /// ids those bindings owned so the caller can tombstone them alongside `id`. Used by
    /// `remove_series`: removing a source drops its derived indicators; removing an indicator's own
    /// output series drops the whole binding (and its sibling outputs).
    pub(crate) fn drop_indicators_touching(&mut self, id: SeriesId) -> Vec<SeriesId> {
        let mut dropped_outputs = Vec::new();
        self.indicators.retain(|binding| {
            let touches_removed = binding.source == id
                || binding.volume_source == Some(id)
                || binding.amount_source == Some(id)
                || binding.outputs.contains(&id)
                || dropped_outputs.contains(&binding.source)
                || binding
                    .volume_source
                    .is_some_and(|source| dropped_outputs.contains(&source))
                || binding
                    .amount_source
                    .is_some_and(|source| dropped_outputs.contains(&source));
            if touches_removed {
                dropped_outputs.extend(binding.outputs.iter().copied());
                false
            } else {
                true
            }
        });
        dropped_outputs
    }

    fn add_indicator(
        &mut self,
        source: SeriesId,
        source_input: IndicatorInputSource,
        kind: IndicatorKind,
        volume_source: Option<SeriesId>,
        amount_source: Option<SeriesId>,
    ) -> Vec<SeriesId> {
        if self.series_entry(source).is_none()
            || amount_source.is_some_and(|id| {
                // Turnover weighting divides by volume, so it needs a distinct volume column.
                !matches!(kind, IndicatorKind::Vwap)
                    || volume_source.is_none_or(|volume| volume == id)
                    || id == source
                    || self
                        .series_entry(id)
                        .is_none_or(|series| !series.kind.stores_scalar_values())
            })
            || match &kind {
                IndicatorKind::Obv
                | IndicatorKind::Cmf { .. }
                | IndicatorKind::Mfi { .. }
                | IndicatorKind::Volume { .. } => volume_source.is_none_or(|id| {
                    id == source
                        || self
                            .series_entry(id)
                            .is_none_or(|series| !series.kind.stores_scalar_values())
                }),
                IndicatorKind::Vwap
                | IndicatorKind::VwapBands { .. }
                | IndicatorKind::Vwma { .. } => volume_source.is_some_and(|id| {
                    id == source
                        || self
                            .series_entry(id)
                            .is_none_or(|series| !series.kind.stores_scalar_values())
                }),
                _ => volume_source.is_some(),
            }
            || match &kind {
                IndicatorKind::Sma { period }
                | IndicatorKind::Ema { period, .. }
                | IndicatorKind::Dema { period, .. }
                | IndicatorKind::Tema { period, .. }
                | IndicatorKind::Smma { period }
                | IndicatorKind::Hma { period }
                | IndicatorKind::Vwma { period }
                | IndicatorKind::StandardDeviation { period }
                | IndicatorKind::Donchian { period }
                | IndicatorKind::Bollinger { period, .. }
                | IndicatorKind::Rsi { period, .. }
                | IndicatorKind::Atr { period }
                | IndicatorKind::Wma { period } => *period == 0,
                IndicatorKind::Kdj {
                    period,
                    k_smoothing,
                    d_smoothing,
                    ..
                } => *period == 0 || *k_smoothing == 0 || *d_smoothing == 0,
                IndicatorKind::EmaRibbon { periods } => periods.contains(&0),
                IndicatorKind::Keltner { period, multiplier } => {
                    *period == 0 || !multiplier.is_finite() || *multiplier < 0.0
                }
                IndicatorKind::AdxDmi { period } => *period == 0,
                IndicatorKind::Cci { period } => *period == 0,
                IndicatorKind::WilliamsR { period } => *period == 0,
                IndicatorKind::StochasticRsi {
                    rsi_period,
                    stochastic_period,
                } => *rsi_period == 0 || *stochastic_period == 0,
                IndicatorKind::Momentum { period } | IndicatorKind::RateOfChange { period } => {
                    *period == 0
                }
                IndicatorKind::ParabolicSar | IndicatorKind::PivotPoints { .. } => false,
                IndicatorKind::ZigZag { deviation_percent } => {
                    !deviation_percent.is_finite() || *deviation_percent <= 0.0
                }
                IndicatorKind::SuperTrend { period, multiplier } => {
                    *period == 0 || !multiplier.is_finite() || *multiplier < 0.0
                }
                IndicatorKind::Ichimoku => false,
                IndicatorKind::Macd {
                    fast,
                    slow,
                    signal,
                    histogram_multiplier,
                    ..
                } => {
                    *fast == 0
                        || *slow == 0
                        || *signal == 0
                        || !histogram_multiplier.is_finite()
                        || *histogram_multiplier <= 0.0
                }
                IndicatorKind::Stochastic { k_period, d_period } => {
                    *k_period == 0 || *d_period == 0
                }
                IndicatorKind::Vwap => false,
                IndicatorKind::Obv => false,
                IndicatorKind::Cmf { period } => *period == 0,
                IndicatorKind::Mfi { period } => *period == 0,
                IndicatorKind::Volume { period } => *period == 0,
                IndicatorKind::VwapBands {
                    standard_deviation,
                    percent,
                    ..
                } => {
                    !standard_deviation.is_finite()
                        || *standard_deviation < 0.0
                        || !percent.is_finite()
                        || *percent < 0.0
                }
            }
        {
            return Vec::new();
        }
        let runtime = incremental_state(&kind);
        let output_count = runtime.output_count();
        let source_price_format = self.series_entry(source).map(|series| {
            (
                series.price_format.kind,
                series.price_format.precision,
                series.price_format.min_move,
            )
        });
        let ids = (0..output_count)
            .map(|_| self.add_series(SeriesKind::Line))
            .collect::<Vec<_>>();
        // Indicator chrome defaults: no candle-close countdown (theirs is a line value, not a
        // bar close), the auto-generated name chip shows (platforms override the name through
        // the series `title` option — custom-script indicators will set their own), and the
        // line draws at 2px without a last-price pulse — every default is overridable through
        // the ordinary series options.
        for (output_index, &id) in ids.iter().enumerate() {
            if let Some(s) = self.series.iter_mut().find(|s| s.id == id) {
                s.countdown_visible = false;
                s.title_visible = true;
                s.title = indicator_output_title(&kind, output_index);
                s.line_width = Some(2.0);
                // The last-price pulse marks the traded series, never a derived study line.
                s.last_price_animation = false;
                if output_index == 0 {
                    s.threshold_region = match kind {
                        IndicatorKind::Rsi { .. } => Some(SeriesThresholdRegion {
                            lower: 30.0,
                            upper: 70.0,
                        }),
                        IndicatorKind::Stochastic { .. } | IndicatorKind::Kdj { .. } => {
                            Some(SeriesThresholdRegion {
                                lower: 20.0,
                                upper: 80.0,
                            })
                        }
                        IndicatorKind::Cci { .. } => Some(SeriesThresholdRegion {
                            lower: -100.0,
                            upper: 100.0,
                        }),
                        IndicatorKind::WilliamsR { .. } => Some(SeriesThresholdRegion {
                            lower: -80.0,
                            upper: -20.0,
                        }),
                        IndicatorKind::StochasticRsi { .. } => Some(SeriesThresholdRegion {
                            lower: 20.0,
                            upper: 80.0,
                        }),
                        _ => None,
                    };
                }
                if let Some(color) = indicator_output_color(&kind, output_index) {
                    s.line_color = Some(color.to_string());
                }
                if let Some((kind, precision, min_move)) = source_price_format {
                    s.price_format.kind = kind;
                    s.price_format.precision = precision;
                    s.price_format.min_move = min_move;
                }
            }
        }
        self.indicators.push(IndicatorBinding {
            source,
            source_input,
            runtime,
            kind,
            outputs: ids.clone(),
            volume_source,
            amount_source,
            source_generation: 0,
            volume_generation: None,
            amount_generation: None,
        });
        self.rebuild_indicator(self.indicators.len() - 1, 0, true);
        ids
    }

    pub(crate) fn recompute_indicators_for(&mut self, dependency: SeriesId) {
        self.indicator_changes.clear();
        self.indicator_changes.push((
            dependency,
            IndicatorChange {
                from: 0,
                previous_generation: 0,
                full_replace: true,
            },
        ));
        self.propagate_indicator_changes();
        self.sync_time_points();
        self.refresh_resampled_dependents(dependency);
    }

    pub(crate) fn recompute_all_indicators(&mut self) {
        self.indicator_changes.clear();
        for binding in &self.indicators {
            for dependency in [Some(binding.source), binding.volume_source]
                .into_iter()
                .flatten()
            {
                if !self
                    .indicator_changes
                    .iter()
                    .any(|(existing, _)| *existing == dependency)
                {
                    self.indicator_changes.push((
                        dependency,
                        IndicatorChange {
                            from: 0,
                            previous_generation: 0,
                            full_replace: true,
                        },
                    ));
                }
            }
        }
        self.propagate_indicator_changes();
        self.sync_time_points();
    }

    pub(crate) fn update_indicators_after_change(
        &mut self,
        dependency: SeriesId,
        change: IndicatorChange,
    ) {
        self.indicator_changes.clear();
        self.indicator_changes.push((dependency, change));
        self.propagate_indicator_changes();
        self.sync_time_points();
        self.refresh_resampled_dependents(dependency);
    }

    /// First source row affected when a timestamp-aligned weight column (volume or turnover)
    /// changed from its own row `from`. Weight rows pair with source rows by timestamp, not by
    /// position, so every source row after the last unchanged weight timestamp is affected.
    fn weight_change_source_row(&self, source: SeriesId, weight: SeriesId, from: usize) -> usize {
        let Some(previous) = from.checked_sub(1) else {
            return 0;
        };
        let (Some((source_times, _)), Some((weight_times, _))) =
            (self.data.series_data(source), self.data.series_data(weight))
        else {
            return 0;
        };
        match weight_times.get(previous).or(weight_times.last()) {
            Some(&unchanged) => source_times.partition_point(|&time| time <= unchanged),
            None => 0,
        }
    }

    /// Exchange time zone, session start, or calendar-date changes move trading-day boundaries:
    /// rebuild every period-keyed binding (VWAP, VWAP bands, pivots) and its dependents once.
    pub(crate) fn rebuild_trading_day_indicators(&mut self) {
        self.indicator_changes.clear();
        for index in 0..self.indicators.len() {
            if matches!(
                self.indicators[index].kind,
                IndicatorKind::Vwap
                    | IndicatorKind::VwapBands { .. }
                    | IndicatorKind::PivotPoints { .. }
            ) {
                let changes = self.rebuild_indicator(index, 0, true);
                self.indicator_changes.extend(changes.into_iter().flatten());
            }
        }
        if !self.indicator_changes.is_empty() {
            self.propagate_indicator_changes();
            self.sync_time_points();
        }
    }

    fn propagate_indicator_changes(&mut self) {
        // Bindings are topological by construction: an indicator output must exist before it can
        // be selected as a later indicator's source. One forward pass therefore updates direct
        // dependencies and every downstream chain without repeatedly scanning the whole graph.
        for index in 0..self.indicators.len() {
            let update = {
                let binding = &self.indicators[index];
                self.indicator_changes
                    .iter()
                    .filter_map(|&(dependency, change)| {
                        let (tracked, weight_column) = if binding.source == dependency {
                            (binding.source_generation, false)
                        } else if binding.volume_source == Some(dependency) {
                            (binding.volume_generation.unwrap_or(0), true)
                        } else if binding.amount_source == Some(dependency) {
                            (binding.amount_generation.unwrap_or(0), true)
                        } else {
                            return None;
                        };
                        let stale = tracked != change.previous_generation;
                        let from = if stale {
                            0
                        } else if weight_column {
                            self.weight_change_source_row(binding.source, dependency, change.from)
                        } else {
                            change.from
                        };
                        Some((from, change.full_replace || stale))
                    })
                    .reduce(|left, right| (left.0.min(right.0), left.1 || right.1))
            };
            if let Some((from, full_replace)) = update {
                let changes = self.rebuild_indicator(index, from, full_replace);
                self.indicator_changes.extend(changes.into_iter().flatten());
            }
        }
    }

    fn rebuild_indicator(
        &mut self,
        index: usize,
        from: usize,
        full_replace: bool,
    ) -> [Option<(SeriesId, IndicatorChange)>; aeris_charts_indicators::MAX_OUTPUTS] {
        let mut changes = [None; aeris_charts_indicators::MAX_OUTPUTS];
        let outputs: [Option<SeriesId>; aeris_charts_indicators::MAX_OUTPUTS] =
            std::array::from_fn(|slot| self.indicators[index].outputs.get(slot).copied());
        for &output in outputs.iter().flatten() {
            self.invalidate_frame_series(output);
        }

        let source = self.indicators[index].source;
        let source_input = self.indicators[index].source_input;
        let volume_source = self.indicators[index].volume_source;
        let amount_source = self.indicators[index].amount_source;
        {
            let Some((times, values)) = self.data.series_data(source) else {
                return changes;
            };
            let selected_close = selected_input(source_input, values);
            // Missing volume/amount timestamps use the documented fallback: zero for volume
            // studies, unit weight for VWAP/VWMA, and "no trade" (NaN, skipped) for the
            // amount-weighted average price.
            let fallback = if amount_source.is_some() {
                f64::NAN
            } else if matches!(
                &self.indicators[index].kind,
                IndicatorKind::Obv
                    | IndicatorKind::Cmf { .. }
                    | IndicatorKind::Mfi { .. }
                    | IndicatorKind::Volume { .. }
            ) {
                0.0
            } else {
                1.0
            };
            let aligned = |column: Option<SeriesId>| {
                column.and_then(|id| self.data.series_data(id)).map_or(
                    Cow::Borrowed(&[][..]),
                    |(column_times, values)| {
                        if column_times == times {
                            Cow::Borrowed(values[3])
                        } else {
                            Cow::Owned(align_volume_to_source_times(
                                times,
                                column_times,
                                values[3],
                                fallback,
                            ))
                        }
                    },
                )
            };
            let volume = aligned(volume_source);
            let amount = aligned(amount_source);
            let exchange_time = &self.exchange_time;
            let binding = &mut self.indicators[index];
            binding.runtime.rebuild_from_with_trading_days(
                aeris_charts_indicators::IndicatorInput {
                    times,
                    open: values[0],
                    high: values[1],
                    low: values[2],
                    close: selected_close.as_ref(),
                    volume: volume.as_ref(),
                    amount: amount.as_ref(),
                },
                if full_replace { 0 } else { from },
                &|seconds| exchange_time.trading_day_seconds(seconds),
            );
        }
        self.indicators[index].source_generation = self.data.series_generation(source).unwrap_or(0);
        self.indicators[index].volume_generation =
            volume_source.and_then(|id| self.data.series_generation(id));
        self.indicators[index].amount_generation =
            amount_source.and_then(|id| self.data.series_generation(id));

        let mut full_histogram_colors = None;
        for (output_index, output) in outputs.iter().flatten().copied().enumerate() {
            let previous_generation = self.data.series_generation(output).unwrap_or(0);
            let source_from = self.indicators[index].runtime.output_from(output_index);

            let output_from = if full_replace {
                let values = self.indicators[index].runtime.take_output(output_index);
                if output_index == 2
                    && matches!(self.indicators[index].kind, IndicatorKind::Macd { .. })
                {
                    full_histogram_colors = Some(momentum_histogram_colors(&values));
                }
                self.data
                    .set_single_data_aligned(output, source, source_from, values);
                0
            } else {
                let values = self.indicators[index].runtime.output(output_index);
                self.data
                    .update_single_aligned(output, source, source_from, values)
                    .expect("indicator output remains aligned to its source")
            };
            if self.data.series_generation(output).unwrap_or(0) != previous_generation {
                changes[output_index] = Some((
                    output,
                    IndicatorChange {
                        from: output_from,
                        previous_generation,
                        full_replace,
                    },
                ));
            }
        }

        if matches!(self.indicators[index].kind, IndicatorKind::Macd { .. }) {
            let histogram_id = outputs[2].unwrap();
            if let Some(colors) = full_histogram_colors {
                self.data
                    .set_point_colors(histogram_id, [Some(colors), None, None]);
            } else {
                let histogram = self.indicators[index].runtime.output(2);
                // The histogram series aliases source rows from its warm-up row, which depends on
                // the seed convention.
                let first_histogram = self.indicators[index].runtime.warmup_rows(2);
                let source_from = self.indicators[index].runtime.output_from(2);
                let output_start = source_from.saturating_sub(first_histogram);
                let mut previous = output_start.checked_sub(1).and_then(|row| {
                    self.data
                        .series_data(histogram_id)
                        .and_then(|(_, values)| values[3].get(row).copied())
                        .filter(|value| value.is_finite())
                });
                for (offset, &value) in histogram.iter().enumerate() {
                    // Whitespace resets the momentum comparison exactly like a full rebuild.
                    let (color, next) = if value.is_finite() {
                        (momentum_histogram_color(value, previous), Some(value))
                    } else {
                        (
                            aeris_charts_core::model::data_layer::POINT_COLOR_ABSENT,
                            None,
                        )
                    };
                    self.data.set_point_color(
                        histogram_id,
                        PointColorChannel::Body,
                        output_start + offset,
                        color,
                    );
                    previous = next;
                }
            }
        }
        self.indicators[index].runtime.release_transfer_capacity();
        changes
    }
}

fn momentum_histogram_colors(values: &[f64]) -> Vec<u32> {
    let mut colors = Vec::with_capacity(values.len());
    let mut previous = None;
    for &value in values {
        if value.is_finite() {
            colors.push(momentum_histogram_color(value, previous));
            previous = Some(value);
        } else {
            colors.push(aeris_charts_core::model::data_layer::POINT_COLOR_ABSENT);
            previous = None;
        }
    }
    colors
}

fn selected_input<'a>(source: IndicatorInputSource, values: [&'a [f64]; 4]) -> Cow<'a, [f64]> {
    match source {
        IndicatorInputSource::Open => Cow::Borrowed(values[0]),
        IndicatorInputSource::High => Cow::Borrowed(values[1]),
        IndicatorInputSource::Low => Cow::Borrowed(values[2]),
        IndicatorInputSource::Close => Cow::Borrowed(values[3]),
        IndicatorInputSource::Hl2 => Cow::Owned(
            values[1]
                .iter()
                .zip(values[2])
                .map(|(&high, &low)| (high + low) * 0.5)
                .collect(),
        ),
        IndicatorInputSource::Hlc3 => Cow::Owned(
            values[1]
                .iter()
                .zip(values[2])
                .zip(values[3])
                .map(|((&high, &low), &close)| (high + low + close) / 3.0)
                .collect(),
        ),
        IndicatorInputSource::Ohlc4 => Cow::Owned(
            values[0]
                .iter()
                .zip(values[1])
                .zip(values[2])
                .zip(values[3])
                .map(|(((&open, &high), &low), &close)| (open + high + low + close) * 0.25)
                .collect(),
        ),
        IndicatorInputSource::Hlcc4 => Cow::Owned(
            values[1]
                .iter()
                .zip(values[2])
                .zip(values[3])
                .map(|((&high, &low), &close)| (high + low + 2.0 * close) * 0.25)
                .collect(),
        ),
    }
}

/// Align an optional volume input to the source timeline without retaining a second canonical
/// timeline. Missing timestamps intentionally use the indicator layer's unit-weight fallback.
fn align_volume_to_source_times(
    source_times: &[i64],
    volume_times: &[i64],
    values: &[f64],
    default: f64,
) -> Vec<f64> {
    let mut aligned = vec![default; source_times.len()];
    let mut volume_row = 0;
    for (source_row, &source_time) in source_times.iter().enumerate() {
        while volume_row < volume_times.len() && volume_times[volume_row] < source_time {
            volume_row += 1;
        }
        if volume_times.get(volume_row) == Some(&source_time) {
            if let Some(&volume) = values.get(volume_row) {
                aligned[source_row] = volume;
            }
        }
    }
    aligned
}

fn indicator_kind_name(kind: &IndicatorKind) -> &'static str {
    match kind {
        IndicatorKind::Sma { .. } => "sma",
        IndicatorKind::Ema { .. } => "ema",
        IndicatorKind::Dema { .. } => "dema",
        IndicatorKind::Tema { .. } => "tema",
        IndicatorKind::Smma { .. } => "smma",
        IndicatorKind::Hma { .. } => "hma",
        IndicatorKind::Vwma { .. } => "vwma",
        IndicatorKind::StandardDeviation { .. } => "standard_deviation",
        IndicatorKind::Cci { .. } => "cci",
        IndicatorKind::WilliamsR { .. } => "williams_r",
        IndicatorKind::StochasticRsi { .. } => "stochastic_rsi",
        IndicatorKind::Momentum { .. } => "momentum",
        IndicatorKind::RateOfChange { .. } => "roc",
        IndicatorKind::Donchian { .. } => "donchian",
        IndicatorKind::PivotPoints { .. } => "pivot_points",
        IndicatorKind::ZigZag { .. } => "zigzag",
        IndicatorKind::Keltner { .. } => "keltner",
        IndicatorKind::AdxDmi { .. } => "adx_dmi",
        IndicatorKind::ParabolicSar => "parabolic_sar",
        IndicatorKind::SuperTrend { .. } => "supertrend",
        IndicatorKind::Ichimoku => "ichimoku",
        IndicatorKind::EmaRibbon { .. } => "ema_ribbon",
        IndicatorKind::Bollinger { .. } => "bollinger",
        IndicatorKind::Rsi { .. } => "rsi",
        IndicatorKind::Macd { .. } => "macd",
        IndicatorKind::Stochastic { .. } => "stochastic",
        IndicatorKind::Atr { .. } => "atr",
        IndicatorKind::Vwap => "vwap",
        IndicatorKind::Obv => "obv",
        IndicatorKind::Cmf { .. } => "cmf",
        IndicatorKind::Mfi { .. } => "mfi",
        IndicatorKind::Volume { .. } => "volume",
        IndicatorKind::VwapBands { .. } => "vwap_bands",
        IndicatorKind::Wma { .. } => "wma",
        IndicatorKind::Kdj { .. } => "kdj",
    }
}

fn pivot_kind_index(kind: aeris_charts_indicators::PivotKind) -> usize {
    match kind {
        aeris_charts_indicators::PivotKind::Standard => 1,
        aeris_charts_indicators::PivotKind::Fibonacci => 2,
        aeris_charts_indicators::PivotKind::Camarilla => 3,
        aeris_charts_indicators::PivotKind::Woodie => 4,
        aeris_charts_indicators::PivotKind::DeMark => 5,
    }
}

fn incremental_state(kind: &IndicatorKind) -> aeris_charts_indicators::IncrementalState {
    match *kind {
        IndicatorKind::Sma { period } => aeris_charts_indicators::IncrementalState::sma(period),
        IndicatorKind::Ema { period, seed } => {
            aeris_charts_indicators::IncrementalState::ema_with_seed(period, seed)
        }
        IndicatorKind::Dema { period, seed } => {
            aeris_charts_indicators::IncrementalState::dema_with_seed(period, seed)
        }
        IndicatorKind::Tema { period, seed } => {
            aeris_charts_indicators::IncrementalState::tema_with_seed(period, seed)
        }
        IndicatorKind::Smma { period } => aeris_charts_indicators::IncrementalState::smma(period),
        IndicatorKind::Hma { period } => aeris_charts_indicators::IncrementalState::hma(period),
        IndicatorKind::Vwma { period } => aeris_charts_indicators::IncrementalState::vwma(period),
        IndicatorKind::StandardDeviation { period } => {
            aeris_charts_indicators::IncrementalState::standard_deviation(period)
        }
        IndicatorKind::Donchian { period } => {
            aeris_charts_indicators::IncrementalState::donchian(period)
        }
        IndicatorKind::PivotPoints { variant } => {
            aeris_charts_indicators::IncrementalState::pivot_points(variant)
        }
        IndicatorKind::ZigZag { deviation_percent } => {
            aeris_charts_indicators::IncrementalState::zigzag(deviation_percent)
        }
        IndicatorKind::Keltner { period, multiplier } => {
            aeris_charts_indicators::IncrementalState::keltner(period, multiplier)
        }
        IndicatorKind::AdxDmi { period } => {
            aeris_charts_indicators::IncrementalState::adx_dmi(period)
        }
        IndicatorKind::Cci { period } => aeris_charts_indicators::IncrementalState::cci(period),
        IndicatorKind::WilliamsR { period } => {
            aeris_charts_indicators::IncrementalState::williams_r(period)
        }
        IndicatorKind::StochasticRsi {
            rsi_period,
            stochastic_period,
        } => {
            aeris_charts_indicators::IncrementalState::stochastic_rsi(rsi_period, stochastic_period)
        }
        IndicatorKind::ParabolicSar => aeris_charts_indicators::IncrementalState::parabolic_sar(),
        IndicatorKind::SuperTrend { period, multiplier } => {
            aeris_charts_indicators::IncrementalState::supertrend(period, multiplier)
        }
        IndicatorKind::Ichimoku => aeris_charts_indicators::IncrementalState::ichimoku(),
        IndicatorKind::EmaRibbon { periods } => {
            aeris_charts_indicators::IncrementalState::ema_ribbon(periods)
        }
        IndicatorKind::Bollinger {
            period,
            deviation,
            estimator,
        } => {
            aeris_charts_indicators::IncrementalState::bollinger_with(period, deviation, estimator)
        }
        IndicatorKind::Rsi { period, seed } => {
            aeris_charts_indicators::IncrementalState::rsi_with_seed(period, seed)
        }
        IndicatorKind::Macd {
            fast,
            slow,
            signal,
            seed,
            histogram_multiplier,
        } => aeris_charts_indicators::IncrementalState::macd_with(
            fast,
            slow,
            signal,
            seed,
            histogram_multiplier,
        ),
        IndicatorKind::Kdj {
            period,
            k_smoothing,
            d_smoothing,
            seed,
        } => aeris_charts_indicators::IncrementalState::kdj_with_seed(
            period,
            k_smoothing,
            d_smoothing,
            seed,
        ),
        IndicatorKind::Stochastic { k_period, d_period } => {
            aeris_charts_indicators::IncrementalState::stochastic(k_period, d_period)
        }
        IndicatorKind::Atr { period } => aeris_charts_indicators::IncrementalState::atr(period),
        IndicatorKind::Vwap => aeris_charts_indicators::IncrementalState::vwap(),
        IndicatorKind::Obv => aeris_charts_indicators::IncrementalState::obv(),
        IndicatorKind::Cmf { period } => aeris_charts_indicators::IncrementalState::cmf(period),
        IndicatorKind::Mfi { period } => aeris_charts_indicators::IncrementalState::mfi(period),
        IndicatorKind::Volume { period } => {
            aeris_charts_indicators::IncrementalState::volume(period)
        }
        IndicatorKind::VwapBands {
            reset,
            standard_deviation,
            percent,
        } => aeris_charts_indicators::IncrementalState::vwap_bands(
            reset,
            standard_deviation,
            percent,
        ),
        IndicatorKind::Momentum { period } => {
            aeris_charts_indicators::IncrementalState::momentum(period)
        }
        IndicatorKind::RateOfChange { period } => {
            aeris_charts_indicators::IncrementalState::rate_of_change(period)
        }
        IndicatorKind::Wma { period } => aeris_charts_indicators::IncrementalState::wma(period),
    }
}

fn momentum_histogram_color(value: f64, previous: Option<f64>) -> u32 {
    let rising = previous.is_none_or(|previous| value >= previous);
    if value >= 0.0 {
        if rising {
            MACD_UP
        } else {
            MACD_UP_WEAK
        }
    } else if rising {
        MACD_DOWN_WEAK
    } else {
        MACD_DOWN
    }
}

/// The auto-generated indicator name behind the (hidden-by-default) name chip — what
/// the public reference shows in its indicator legend ("SMA 20", "MACD 12 26 9"). Platforms can read it
/// via the series options or override it with their own `title`.
fn indicator_title(kind: &IndicatorKind) -> String {
    let params = |d: f64| {
        if d.fract() == 0.0 {
            format!("{}", d as i64)
        } else {
            format!("{d}")
        }
    };
    match kind {
        IndicatorKind::Sma { period } => format!("SMA {period}"),
        IndicatorKind::Ema { period, .. } => format!("EMA {period}"),
        IndicatorKind::Dema { period, .. } => format!("DEMA {period}"),
        IndicatorKind::Tema { period, .. } => format!("TEMA {period}"),
        IndicatorKind::Smma { period } => format!("SMMA {period}"),
        IndicatorKind::Hma { period } => format!("HMA {period}"),
        IndicatorKind::Vwma { period } => format!("VWMA {period}"),
        IndicatorKind::StandardDeviation { period } => format!("Std Dev {period}"),
        IndicatorKind::Cci { period } => format!("CCI {period}"),
        IndicatorKind::WilliamsR { period } => format!("Williams %R {period}"),
        IndicatorKind::StochasticRsi {
            rsi_period,
            stochastic_period,
        } => format!("Stochastic RSI {rsi_period} {stochastic_period}"),
        IndicatorKind::Momentum { period } => format!("Momentum {period}"),
        IndicatorKind::RateOfChange { period } => format!("ROC {period}"),
        IndicatorKind::Donchian { period } => format!("Donchian {period}"),
        IndicatorKind::PivotPoints { variant } => format!("Pivot Points {variant:?}"),
        IndicatorKind::ZigZag { deviation_percent } => {
            format!("ZigZag {deviation_percent}%")
        }
        IndicatorKind::Keltner { period, multiplier } => {
            format!("Keltner {period} {}", params(*multiplier))
        }
        IndicatorKind::AdxDmi { period } => format!("ADX/DMI {period}"),
        IndicatorKind::ParabolicSar => "Parabolic SAR".to_string(),
        IndicatorKind::SuperTrend { period, multiplier } => {
            format!("SuperTrend {period} {}", params(*multiplier))
        }
        IndicatorKind::Ichimoku => "Ichimoku".to_string(),
        IndicatorKind::EmaRibbon { periods } => format!(
            "EMA Ribbon {} {} {} {} {}",
            periods[0], periods[1], periods[2], periods[3], periods[4]
        ),
        IndicatorKind::Bollinger {
            period, deviation, ..
        } => {
            format!("Bollinger {period} {}", params(*deviation))
        }
        IndicatorKind::Rsi { period, .. } => format!("RSI {period}"),
        IndicatorKind::Macd {
            fast, slow, signal, ..
        } => format!("MACD {fast} {slow} {signal}"),
        IndicatorKind::Stochastic { k_period, d_period } => {
            format!("Stochastic {k_period} {d_period}")
        }
        IndicatorKind::Atr { period } => format!("ATR {period}"),
        IndicatorKind::Vwap => "VWAP".to_string(),
        IndicatorKind::Obv => "OBV".to_string(),
        IndicatorKind::Cmf { period } => format!("CMF {period}"),
        IndicatorKind::Mfi { period } => format!("MFI {period}"),
        IndicatorKind::Volume { .. } => "Volume".to_string(),
        IndicatorKind::VwapBands { .. } => "VWAP Bands".to_string(),
        IndicatorKind::Wma { period } => format!("WMA {period}"),
        IndicatorKind::Kdj {
            period,
            k_smoothing,
            d_smoothing,
            ..
        } => format!("KDJ {period} {k_smoothing} {d_smoothing}"),
    }
}

fn indicator_output_title(kind: &IndicatorKind, output_index: usize) -> String {
    match kind {
        IndicatorKind::EmaRibbon { periods } => format!("EMA {}", periods[output_index]),
        _ => indicator_title(kind),
    }
}

fn indicator_output_color(kind: &IndicatorKind, output_index: usize) -> Option<&'static str> {
    matches!(kind, IndicatorKind::EmaRibbon { .. }).then(|| EMA_RIBBON_DEFAULT_COLORS[output_index])
}

pub(crate) fn indicator_output_style(series: &SeriesEntry) -> IndicatorOutputStyle {
    IndicatorOutputStyle {
        visible: series.visible,
        line_color: series.line_color.clone(),
        line_width: series.line_width,
        line_style: series.line_style,
        point_markers: series.point_markers,
        up_color: series.up_color.clone(),
        down_color: series.down_color.clone(),
        area_top_color: series.area_top_color.clone(),
        area_bottom_color: series.area_bottom_color.clone(),
    }
}

fn style_color_is_valid(value: Option<&str>) -> bool {
    value.is_none_or(|value| {
        value.len() <= 256 && aeris_charts_render::color::Color::parse_css(value).is_some()
    })
}

fn indicator_output_name(kind: &IndicatorKind, output_index: usize) -> &'static str {
    match kind {
        IndicatorKind::Sma { .. } => "SMA",
        IndicatorKind::Ema { .. } => "EMA",
        IndicatorKind::Dema { .. } => "DEMA",
        IndicatorKind::Tema { .. } => "TEMA",
        IndicatorKind::Smma { .. } => "SMMA",
        IndicatorKind::Hma { .. } => "HMA",
        IndicatorKind::Vwma { .. } => "VWMA",
        IndicatorKind::StandardDeviation { .. } => "Std Dev",
        IndicatorKind::Cci { .. } => "CCI",
        IndicatorKind::WilliamsR { .. } => "%R",
        IndicatorKind::StochasticRsi { .. } => "Stoch RSI",
        IndicatorKind::Momentum { .. } => "Momentum",
        IndicatorKind::RateOfChange { .. } => "ROC",
        IndicatorKind::Donchian { .. } => ["Upper", "Basis", "Lower"][output_index],
        IndicatorKind::PivotPoints { .. } => ["Pivot", "R1", "S1", "R2", "S2"][output_index],
        IndicatorKind::ZigZag { .. } => "ZigZag",
        IndicatorKind::Keltner { .. } => ["Upper", "Basis", "Lower"][output_index],
        IndicatorKind::AdxDmi { .. } => ["+DI", "-DI", "ADX"][output_index],
        IndicatorKind::ParabolicSar => "SAR",
        IndicatorKind::SuperTrend { .. } => "SuperTrend",
        IndicatorKind::Ichimoku => {
            ["Conversion", "Base", "Leading A", "Leading B", "Lagging"][output_index]
        }
        IndicatorKind::EmaRibbon { .. } => {
            ["EMA 1", "EMA 2", "EMA 3", "EMA 4", "EMA 5"][output_index]
        }
        IndicatorKind::Bollinger { .. } => ["Upper", "Basis", "Lower"][output_index],
        IndicatorKind::Rsi { .. } => "RSI",
        IndicatorKind::Macd { .. } => ["MACD", "Signal", "Histogram"][output_index],
        IndicatorKind::Stochastic { .. } => ["%K", "%D"][output_index],
        IndicatorKind::Atr { .. } => "ATR",
        IndicatorKind::Vwap => "VWAP",
        IndicatorKind::Obv => "OBV",
        IndicatorKind::Cmf { .. } => "CMF",
        IndicatorKind::Mfi { .. } => "MFI",
        IndicatorKind::Volume { .. } => ["Volume", "MA"][output_index],
        IndicatorKind::VwapBands { .. } => {
            ["Basis", "Std Upper", "Std Lower", "% Upper", "% Lower"][output_index]
        }
        IndicatorKind::Wma { .. } => "WMA",
        IndicatorKind::Kdj { .. } => ["K", "D", "J"][output_index],
    }
}
