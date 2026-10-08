//! Engine-owned indicator producers.
//!
//! Indicators are bound to a source series and recomputed on source updates; their outputs are
//! ordinary engine series (`aeris_charts_indicators` holds the pure math). Extracted from `lib.rs`.

use super::*;
use crate::custom_studies::{CustomBinding, CustomBindingState};
use aeris_charts_indicators::structure_studies::{
    BreakOn, Mitigation, MitigationPrice, OrderBlockZone as CalculationOrderBlockZone,
    StructureStudy, StructureStudyKind,
};
use aeris_charts_indicators::study_annotations::StudyAnnotations;
use aeris_charts_indicators::{SessionSource, SessionStudy, SessionStudyState};
use std::borrow::Cow;

/// Scalar source selected by a study.  The aggregate sources are calculated from the source
/// bar's OHLC columns without changing the canonical source series; each binding keeps the derived
/// column as private runtime state (see `IndicatorInputs`), not as a series.
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
    Boolean,
    Source,
    Series,
    /// One of the string values listed in [`IndicatorParameterDescriptor::options`].
    Choice,
}

/// Session policy selected by calendar-aware studies. No timezone is inferred from the host.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StudyCalendarPolicy {
    /// The chart's exchange trading calendar (the default): the exchange time zone and trading
    /// session start of [`crate::ChartEngine::exchange_time`], the calendar VWAP resets and pivots
    /// use. Days, Monday weeks and civil months are counted on the trading date, so a night
    /// session that crosses midnight belongs to the next trading day, week and month; an opening
    /// range starts at the latest session start at or before the trading day's first bar. Equal
    /// to [`Self::Utc`] while the exchange time is UTC with a midnight session start.
    #[default]
    Exchange,
    /// UTC calendar days, Monday UTC weeks and UTC civil months.
    Utc,
    /// Host session spans from [`crate::ChartEngine::set_study_calendar`].
    Host,
}

impl StudyCalendarPolicy {
    /// Wire names in display order (`exchange` first: the default).
    pub const NAMES: [&'static str; 3] = ["exchange", "utc", "host"];

    pub fn name(self) -> &'static str {
        match self {
            Self::Exchange => "exchange",
            Self::Utc => "utc",
            Self::Host => "host",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "exchange" => Some(Self::Exchange),
            "utc" => Some(Self::Utc),
            "host" => Some(Self::Host),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreviousPeriod {
    Day,
    Week,
    Month,
}

/// Price used to detect a structural break.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StructureBreakOn {
    #[default]
    Close,
    Wick,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StructureMitigation {
    #[default]
    Touch,
    Half,
    Full,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StructureMitigationPrice {
    #[default]
    Wick,
    Close,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderBlockZone {
    #[default]
    Wick,
    Body,
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
    /// Allowed values of a [`IndicatorParameterType::Choice`] parameter, in display order.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<Vec<String>>,
}

impl IndicatorParameterDescriptor {
    /// A choice has an ordered, nonempty option list and a default from that list.
    pub fn choice(name: &str, default: &str, options: &[&str]) -> Option<Self> {
        if options.is_empty() || !options.contains(&default) {
            return None;
        }
        Some(Self {
            name: name.into(),
            parameter_type: IndicatorParameterType::Choice,
            default: serde_json::json!(default),
            min: None,
            max: None,
            options: Some(options.iter().map(|option| (*option).into()).collect()),
        })
    }
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

pub const INDICATOR_SCHEMA_REVISION: u32 = 4;

// `remote = "Self"` makes serde emit the derived bodies as inherent functions, so the trait impls below can
// keep the large internally tagged `Deserialize` body out of line. Without that, every call path
// (`from_value` on one side, a struct field through `PhantomData` on the other) carried its own inlined
// copy of the roughly 110 KB body in the shipped WASM. The price is two public inherent functions on the
// published type, `IndicatorKind::serialize` and `IndicatorKind::deserialize`, which serde generates with
// the type's visibility; the `Serialize`/`Deserialize` trait impls remain the supported entry points.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", remote = "Self")]
pub enum IndicatorKind {
    Aroon {
        period: usize,
    },
    AwesomeOscillator,
    Dpo {
        period: usize,
    },
    ChandeMomentum {
        period: usize,
    },
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
    BollingerMetrics {
        period: usize,
        deviation: f64,
    },
    Envelopes {
        period: usize,
        percent: f64,
        exponential: bool,
    },
    Alma {
        period: usize,
        offset: f64,
        sigma: f64,
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
    AccumulationDistribution,
    PriceVolumeTrend,
    ChaikinOscillator {
        fast: usize,
        slow: usize,
    },
    Klinger {
        fast: usize,
        slow: usize,
        signal: usize,
    },
    Kama {
        period: usize,
        fast: usize,
        slow: usize,
    },
    #[serde(rename = "mcginley")]
    McGinley {
        period: usize,
    },
    LinearRegression {
        period: usize,
        deviation: f64,
    },
    Choppiness {
        period: usize,
    },
    AtrBands {
        period: usize,
        multiplier: f64,
    },
    RelativeVolume {
        period: usize,
    },
    VolumeOscillator {
        fast: usize,
        slow: usize,
        signal: usize,
    },
    ElderForce {
        period: usize,
    },
    EaseOfMovement {
        period: usize,
        divisor: f64,
    },
    HistoricalVolatility {
        period: usize,
        annualization: f64,
    },
    Trix {
        period: usize,
        signal: usize,
    },
    Kst {
        roc: [usize; 4],
        smoothing: [usize; 4],
        signal: usize,
    },
    Tsi {
        long: usize,
        short: usize,
        signal: usize,
    },
    MassIndex {
        ema_period: usize,
        sum_period: usize,
    },
    Vortex {
        period: usize,
    },
    CoppockCurve {
        long: usize,
        short: usize,
        smoothing: usize,
    },
    FisherTransform {
        period: usize,
    },
    UltimateOscillator {
        short: usize,
        medium: usize,
        long: usize,
    },
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
    SwingPoints {
        left: usize,
        right: usize,
    },
    MarketStructure {
        left: usize,
        right: usize,
        break_on: StructureBreakOn,
    },
    FairValueGaps {
        min_size: f64,
        mitigation: StructureMitigation,
        mitigation_price: StructureMitigationPrice,
        max_active: usize,
        show_mitigated: bool,
    },
    OrderBlocks {
        left: usize,
        right: usize,
        break_on: StructureBreakOn,
        zone: OrderBlockZone,
        mitigation: StructureMitigation,
        mitigation_price: StructureMitigationPrice,
        max_active: usize,
        show_mitigated: bool,
    },
    /// A persisted definition without `calendar` reads as [`StudyCalendarPolicy::Exchange`].
    SessionLevels {
        #[serde(default)]
        calendar: StudyCalendarPolicy,
    },
    PreviousPeriodLevels {
        period: PreviousPeriod,
        #[serde(default)]
        calendar: StudyCalendarPolicy,
    },
    OpeningRange {
        duration_seconds: u32,
        #[serde(default)]
        calendar: StudyCalendarPolicy,
    },
    Custom {
        type_id: String,
        version: u32,
        parameters: BTreeMap<String, serde_json::Value>,
        output_count: usize,
    },
    /// KDJ: RSV over `period` rows, `K = SMA(RSV, k_smoothing, 1)`,
    /// `D = SMA(K, d_smoothing, 1)`, `J = 3K - 2D` (defaults 9/3/3). `seed` also decides whether
    /// the first rows use a partial RSV window (see `KdjSeed`).
    Kdj {
        period: usize,
        k_smoothing: usize,
        d_smoothing: usize,
        /// K/D start; documents written before it existed restore the textbook 50.
        #[serde(default)]
        seed: aeris_charts_indicators::KdjSeed,
    },
    /// One of the 27 KLineChart indicators with KLineChart's formulas and presentation (see
    /// [`aeris_charts_indicators::klinechart`]). Serialized with its template tag beside the kind:
    /// `{"kind": "klinechart", "indicator": "macd", "short": 12, "long": 26, "signal": 9}`.
    ///
    /// `VOL`, `OBV`, `PVT`, `EMV`, and `VR` require a scalar `volume_source`. `AVP` reads turnover
    /// from its source, which must then be a scalar series (typically hidden), and volume from its
    /// `volume_source`.
    #[serde(rename = "klinechart")]
    KLineChart(aeris_charts_indicators::klinechart::Indicator),
}

impl serde::Serialize for IndicatorKind {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        Self::serialize(self, serializer)
    }
}

impl<'de> serde::Deserialize<'de> for IndicatorKind {
    #[inline(never)]
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::deserialize(deserializer)
    }
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
            // ponytail: the breadth-tier EMA kinds (exponential Envelopes, Chaikin Oscillator,
            // Volume Oscillator, Elder Force, TRIX, TSI, Klinger, Mass Index) and KAMA always seed
            // with the SMA of their first N samples, and McGinley with its first close; none
            // carries a `seed` parameter, so the China preset leaves them unchanged;
            // likewise Bollinger metrics always use the population deviation. Add the typed
            // parameters here (and to the schema, persistence and TS) once a platform reference
            // for their formula-language forms is verified. The structure and session studies have
            // no recursive average to seed, so no preset applies to them either; a custom study
            // owns its formula, so its parameters are the host's and the preset leaves them as
            // they are.
            other => other,
        }
    }

    /// Resolve the browser's legacy name/period/deviation schema query to an engine definition.
    /// The implicit (14, 2) query uses each study's canonical defaults (KDJ 9/3/3); a different
    /// period retains the legacy query's period substitution for multi-period studies. Seeded
    /// kinds describe the textbook SMA seed, Bollinger the population deviation, and a
    /// `klinechart_*` name the KLineChart template with its default parameters.
    pub fn schema_definition(kind: &str, period: usize, deviation: f64) -> Option<Self> {
        use aeris_charts_indicators::{PivotKind, VwapReset};
        Some(match kind {
            "swing_points" => Self::SwingPoints { left: 5, right: 5 },
            "market_structure" => Self::MarketStructure {
                left: 5,
                right: 5,
                break_on: StructureBreakOn::Close,
            },
            "fair_value_gaps" => Self::FairValueGaps {
                min_size: 0.0,
                mitigation: StructureMitigation::Touch,
                mitigation_price: StructureMitigationPrice::Wick,
                max_active: 20,
                show_mitigated: false,
            },
            "order_blocks" => Self::OrderBlocks {
                left: 5,
                right: 5,
                break_on: StructureBreakOn::Close,
                zone: OrderBlockZone::Wick,
                mitigation: StructureMitigation::Touch,
                mitigation_price: StructureMitigationPrice::Wick,
                max_active: 20,
                show_mitigated: false,
            },
            "session_levels" => Self::SessionLevels {
                calendar: StudyCalendarPolicy::Exchange,
            },
            "previous_period_levels" => Self::PreviousPeriodLevels {
                period: PreviousPeriod::Day,
                calendar: StudyCalendarPolicy::Exchange,
            },
            "opening_range" => Self::OpeningRange {
                duration_seconds: 1800,
                calendar: StudyCalendarPolicy::Exchange,
            },
            "aroon" => Self::Aroon { period },
            "awesome_oscillator" => Self::AwesomeOscillator,
            "dpo" => Self::Dpo { period },
            "chande_momentum" => Self::ChandeMomentum { period },
            "bollinger_metrics" => Self::BollingerMetrics { period, deviation },
            "envelopes" => Self::Envelopes {
                period,
                percent: deviation,
                exponential: false,
            },
            "alma" => Self::Alma {
                period,
                offset: 0.85,
                sigma: 6.0,
            },
            "sma" => Self::Sma { period },
            "ema" => Self::Ema {
                period,
                seed: IndicatorSeed::Sma,
            },
            "dema" => Self::Dema {
                period,
                seed: IndicatorSeed::Sma,
            },
            "tema" => Self::Tema {
                period,
                seed: IndicatorSeed::Sma,
            },
            "smma" | "rma" => Self::Smma { period },
            "hma" => Self::Hma { period },
            "vwma" => Self::Vwma { period },
            "standard_deviation" => Self::StandardDeviation { period },
            "cci" => Self::Cci { period },
            "williams_r" => Self::WilliamsR { period },
            "stochastic_rsi" => Self::StochasticRsi {
                rsi_period: period,
                stochastic_period: period,
            },
            "momentum" => Self::Momentum { period },
            "roc" => Self::RateOfChange { period },
            "donchian" => Self::Donchian { period },
            "pivot_points" => Self::PivotPoints {
                variant: match period {
                    1 => PivotKind::Standard,
                    2 => PivotKind::Fibonacci,
                    3 => PivotKind::Camarilla,
                    4 => PivotKind::Woodie,
                    5 => PivotKind::DeMark,
                    _ => return None,
                },
            },
            "zigzag" => Self::ZigZag {
                deviation_percent: deviation,
            },
            "keltner" => Self::Keltner {
                period,
                multiplier: deviation,
            },
            "adx_dmi" => Self::AdxDmi { period },
            "parabolic_sar" => Self::ParabolicSar,
            "supertrend" => Self::SuperTrend {
                period,
                // The legacy query's implicit 2.0 is not SuperTrend's own 3.0 default.
                multiplier: if deviation == 2.0 { 3.0 } else { deviation },
            },
            "ichimoku" => Self::Ichimoku,
            "ema_ribbon" => Self::EmaRibbon {
                periods: if period == 14 {
                    [5, 10, 20, 50, 200]
                } else {
                    [period; 5]
                },
            },
            "bollinger" => Self::Bollinger {
                period,
                deviation,
                estimator: DeviationEstimator::Population,
            },
            "rsi" => Self::Rsi {
                period,
                seed: IndicatorSeed::Sma,
            },
            "macd" => Self::Macd {
                fast: if period == 14 { 12 } else { period },
                slow: if period == 14 {
                    26
                } else {
                    period.saturating_mul(2)
                },
                signal: if period == 14 { 9 } else { period },
                seed: IndicatorSeed::Sma,
                histogram_multiplier: 1.0,
            },
            "kdj" => Self::Kdj {
                period: if period == 14 { 9 } else { period },
                k_smoothing: 3,
                d_smoothing: 3,
                seed: KdjSeed::Fifty,
            },
            "stochastic" => Self::Stochastic {
                k_period: period,
                d_period: if period == 14 { 3 } else { period },
            },
            "atr" => Self::Atr { period },
            "vwap" => Self::Vwap,
            "obv" => Self::Obv,
            "accumulation_distribution" => Self::AccumulationDistribution,
            "price_volume_trend" => Self::PriceVolumeTrend,
            "chaikin_oscillator" => Self::ChaikinOscillator { fast: 3, slow: 10 },
            "klinger" => Self::Klinger {
                fast: 34,
                slow: 55,
                signal: 13,
            },
            "kama" => Self::Kama {
                period: 10,
                fast: 2,
                slow: 30,
            },
            "mcginley" => Self::McGinley { period },
            "linear_regression" => Self::LinearRegression {
                period: 20,
                deviation: 2.0,
            },
            "choppiness" => Self::Choppiness { period },
            "atr_bands" => Self::AtrBands {
                period,
                multiplier: deviation,
            },
            "relative_volume" => Self::RelativeVolume { period },
            "elder_force" => Self::ElderForce { period },
            "ease_of_movement" => Self::EaseOfMovement {
                period,
                divisor: 100_000_000.0,
            },
            "historical_volatility" => Self::HistoricalVolatility {
                period,
                annualization: 252.0,
            },
            "trix" => Self::Trix { period, signal: 9 },
            "kst" => Self::Kst {
                roc: [10, 15, 20, 30],
                smoothing: [10, 10, 10, 15],
                signal: 9,
            },
            "tsi" => Self::Tsi {
                long: 25,
                short: 13,
                signal: 13,
            },
            "mass_index" => Self::MassIndex {
                ema_period: 9,
                sum_period: 25,
            },
            "vortex" => Self::Vortex { period },
            "coppock_curve" => Self::CoppockCurve {
                long: 14,
                short: 11,
                smoothing: 10,
            },
            "fisher_transform" => Self::FisherTransform { period },
            "ultimate_oscillator" => Self::UltimateOscillator {
                short: 7,
                medium: 14,
                long: 28,
            },
            "volume_oscillator" => Self::VolumeOscillator {
                fast: 12,
                slow: 26,
                signal: 9,
            },
            "cmf" => Self::Cmf { period },
            "mfi" => Self::Mfi { period },
            "volume" => Self::Volume { period },
            "vwap_bands" => Self::VwapBands {
                reset: VwapReset::Session,
                // The legacy TS query supplies 2.0 even with no arguments, whereas the
                // study's own default is 1.0. Other values remain explicit overrides.
                standard_deviation: if deviation == 2.0 { 1.0 } else { deviation },
                percent: 10.0,
            },
            "wma" => Self::Wma { period },
            other => Self::KLineChart(crate::klinechart_indicator_for_kind_name(other)?),
        })
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

pub(crate) struct IndicatorBinding {
    pub(crate) source: SeriesId,
    pub(crate) source_input: IndicatorInputSource,
    pub(crate) kind: IndicatorKind,
    pub(crate) outputs: Vec<SeriesId>,
    /// Parallel volume column source (VWAP); `None` = unit weights.
    pub(crate) volume_source: Option<SeriesId>,
    /// Turnover column of an amount-weighted VWAP, aligned by timestamp like volume.
    pub(crate) amount_source: Option<SeriesId>,
    /// Structural-study geometry is binding-owned, never a synthetic output series.
    pub(crate) annotations: Option<StudyAnnotations>,
    /// Incremental OHLC scanner for structure studies; its history is not persisted.
    pub(crate) structure: Option<StructureStudy>,
    pub(crate) session: Option<SessionStudyState>,
    pub(crate) calendar: Option<StudyCalendarPolicy>,
    pub(crate) runtime: BindingRuntime,
    pub(crate) inputs: IndicatorInputs,
    pub(crate) source_generation: u64,
    pub(crate) volume_generation: Option<u64>,
    pub(crate) amount_generation: Option<u64>,
    /// Source rows the runtime covers: through the source's last real row as of the latest
    /// rebuild. Output rows at or past it are whitespace (the trailing rows of pre-installed
    /// session slots), so a tail rebuild never recomputes or rewrites them.
    pub(crate) data_end: usize,
}

/// A binding's formula runtime: a built-in incremental state, or a host-registered custom study
/// (see `custom_studies`), which is not `Clone` and is scheduled by the same engine paths.
pub(crate) enum BindingRuntime {
    BuiltIn(Box<aeris_charts_indicators::IncrementalState>),
    Custom(CustomBinding),
}

impl BindingRuntime {
    pub(crate) fn custom(&self) -> Option<&CustomBinding> {
        match self {
            Self::Custom(custom) => Some(custom),
            Self::BuiltIn(_) => None,
        }
    }

    pub(crate) fn custom_mut(&mut self) -> Option<&mut CustomBinding> {
        match self {
            Self::Custom(custom) => Some(custom),
            Self::BuiltIn(_) => None,
        }
    }

    fn built_in(&self) -> &aeris_charts_indicators::IncrementalState {
        match self {
            Self::BuiltIn(runtime) => runtime,
            Self::Custom(_) => unreachable!("custom bindings are dispatched before built-in work"),
        }
    }

    fn built_in_mut(&mut self) -> &mut aeris_charts_indicators::IncrementalState {
        match self {
            Self::BuiltIn(runtime) => runtime.as_mut(),
            Self::Custom(_) => unreachable!("custom bindings are dispatched before built-in work"),
        }
    }
}

impl IndicatorBinding {
    /// Warm-up and convergence rows of output `output_index`. Structure and session studies run
    /// their own scanners, not the scalar placeholder runtime: a swing level needs `left + right`
    /// rows to confirm its first pivot, the other study outputs have no fixed warm-up, and none
    /// converges after a fixed number of rows (each value follows the latest confirmed pivot or
    /// the session and period its row falls in).
    fn output_warmup(&self, output_index: usize) -> (usize, Option<usize>) {
        match self.kind {
            IndicatorKind::SwingPoints { left, right } => (left.saturating_add(right), None),
            _ if self.structure.is_some() || self.session.is_some() => (0, None),
            // A custom study's warm-up is its own: rows before its first value are whitespace.
            _ if self.runtime.custom().is_some() => (0, None),
            _ => (
                self.runtime.built_in().warmup_rows(output_index),
                self.runtime.built_in().convergence_rows(output_index),
            ),
        }
    }

    /// Rows of work this binding's most recent rebuild performed: formula rows its runtime
    /// evaluated plus aggregate-input and weight-alignment rows it derived, the rows a structure
    /// or session study scanned (checkpoint replays included), or the rows handed to a custom
    /// study's runtime (counted with its input rows).
    pub(crate) fn last_work_rows(&self) -> usize {
        let runtime_rows = match &self.runtime {
            BindingRuntime::BuiltIn(runtime) => runtime.last_work_rows(),
            BindingRuntime::Custom(_) => 0,
        };
        runtime_rows
            + self.inputs.work_rows
            + self
                .structure
                .as_ref()
                .map_or(0, StructureStudy::last_work_rows)
            + self
                .session
                .as_ref()
                .map_or(0, SessionStudyState::last_work_rows)
    }
}

/// Binding-owned input columns derived from canonical series: an aggregate price column (`hl2`,
/// `hlc3`, `ohlc4`, `hlcc4`) and timestamp-aligned weight columns. They share the runtime's
/// invariant that source rows before a rebuild's first changed row are unchanged, so a tail
/// update derives only the changed suffix. They are private runtime state, never canonical
/// series, and are counted in the indicator runtime memory.
#[derive(Clone, Debug, Default)]
pub(crate) struct IndicatorInputs {
    pub(crate) price: Vec<f64>,
    pub(crate) volume: AlignedWeights,
    amount: AlignedWeights,
    /// Input rows derived or compared by the last rebuild.
    pub(crate) work_rows: usize,
}

impl IndicatorInputs {
    fn bytes(&self) -> usize {
        (self.price.capacity() + self.volume.aligned.capacity() + self.amount.aligned.capacity())
            * std::mem::size_of::<f64>()
    }
}

/// A weight column (volume or turnover) paired with the source rows by timestamp.
#[derive(Clone, Debug, Default)]
pub(crate) struct AlignedWeights {
    /// Leading rows at which the weight series and the source carry the same timestamps, as of
    /// the last rebuild.
    matched: usize,
    /// Aligned weights while the two timelines diverge; empty while one is a prefix of the other.
    aligned: Vec<f64>,
}

impl AlignedWeights {
    /// Weights for `source_times`, re-deriving only rows `from..`, plus the rows touched.
    ///
    /// While one timeline is a prefix of the other (identical timelines, or a candle that
    /// streamed its new bar before its volume did, or after), the weight column is borrowed as
    /// is and the runtime applies each formula's missing-weight fallback past its end. Otherwise
    /// the retained aligned column keeps its unchanged prefix and uses `fallback` for source
    /// timestamps the weight series lacks.
    pub(crate) fn column<'a>(
        &'a mut self,
        source_times: &[i64],
        weight: Option<(&'a [i64], &'a [f64])>,
        from: usize,
        fallback: f64,
    ) -> (&'a [f64], usize) {
        let Some((times, values)) = weight else {
            *self = Self::default();
            return (&[], 0);
        };
        let rows = source_times.len();
        let shared = rows.min(times.len());
        let resume = self.matched.min(from).min(shared);
        let mut matched = resume;
        while matched < shared && times[matched] == source_times[matched] {
            matched += 1;
        }
        self.matched = matched;
        let compared = matched - resume;
        // An empty weight series keeps the explicit fallback column (an amount-weighted VWAP
        // treats an empty turnover column as "no turnover source").
        if matched == shared && (shared > 0 || rows == 0) {
            self.aligned = Vec::new();
            return (&values[..shared], compared);
        }
        let keep = self.realign(source_times, times, values, from, fallback);
        (&self.aligned, compared + rows - keep)
    }

    /// Weights for every row of `source_times`, `fallback` where the weight series has no row
    /// (including rows past its end), or empty without a weight series, plus the rows derived.
    /// The column is always retained and re-derives only rows `from..`. Custom runtimes index it
    /// by source row, so it is never a borrowed prefix shorter than the source.
    pub(crate) fn full_column(
        &mut self,
        source_times: &[i64],
        weight: Option<(&[i64], &[f64])>,
        from: usize,
        fallback: f64,
    ) -> (&[f64], usize) {
        let Some((times, values)) = weight else {
            *self = Self::default();
            return (&[], 0);
        };
        let keep = self.realign(source_times, times, values, from, fallback);
        (&self.aligned, source_times.len() - keep)
    }

    /// Re-derive the retained aligned column from row `from` and return the rows kept.
    fn realign(
        &mut self,
        source_times: &[i64],
        times: &[i64],
        values: &[f64],
        from: usize,
        fallback: f64,
    ) -> usize {
        let rows = source_times.len();
        let keep = self.aligned.len().min(from).min(rows);
        self.aligned.truncate(keep);
        // Same explicit capacity policy as the retained aggregate price column.
        let target = rows + price_headroom(rows);
        if keep == 0 && self.aligned.capacity() > 2 * target {
            self.aligned.shrink_to(target);
        }
        if self.aligned.capacity() < rows {
            self.aligned.reserve_exact(target - keep);
        }
        let mut weight_row = source_times.get(keep).map_or(times.len(), |&first| {
            times.partition_point(|&time| time < first)
        });
        for &time in &source_times[keep..] {
            while weight_row < times.len() && times[weight_row] < time {
                weight_row += 1;
            }
            self.aligned.push(if times.get(weight_row) == Some(&time) {
                values[weight_row]
            } else {
                fallback
            });
        }
        keep
    }
}

/// Spare rows a retained aggregate column keeps past its source: one eighth of the rows plus a
/// fixed floor, so the capacity stays within `rows + price_headroom(rows)`. The floor only spares
/// small charts their first regrowth.
fn price_headroom(rows: usize) -> usize {
    rows / 8 + 4096
}

/// The scalar input column a study reads as its close: a canonical column, or the aggregate
/// price retained in `cache` with rows `from..` re-derived. Returns the column and derived rows.
pub(crate) fn price_input<'a>(
    cache: &'a mut Vec<f64>,
    source: IndicatorInputSource,
    values: [&'a [f64]; 4],
    from: usize,
) -> (&'a [f64], usize) {
    let aggregate: fn(f64, f64, f64, f64) -> f64 = match source {
        IndicatorInputSource::Open
        | IndicatorInputSource::High
        | IndicatorInputSource::Low
        | IndicatorInputSource::Close => {
            *cache = Vec::new();
            let column = match source {
                IndicatorInputSource::Open => 0,
                IndicatorInputSource::High => 1,
                IndicatorInputSource::Low => 2,
                _ => 3,
            };
            return (values[column], 0);
        }
        IndicatorInputSource::Hl2 => |_, high, low, _| (high + low) * 0.5,
        IndicatorInputSource::Hlc3 => |_, high, low, close| (high + low + close) / 3.0,
        IndicatorInputSource::Ohlc4 => |open, high, low, close| (open + high + low + close) * 0.25,
        IndicatorInputSource::Hlcc4 => |_, high, low, close| (high + low + 2.0 * close) * 0.25,
    };
    let rows = values.iter().map(|column| column.len()).min().unwrap_or(0);
    let keep = cache.len().min(from).min(rows);
    cache.truncate(keep);
    // Capacity is explicit: a column built exactly to size would double on the first append after
    // a bulk install, and plain `extend` growth would keep the largest size it ever reached.
    let target = rows + price_headroom(rows);
    if keep == 0 && cache.capacity() > 2 * target {
        cache.shrink_to(target);
    }
    if cache.capacity() < rows {
        cache.reserve_exact(target - keep);
    }
    cache.extend((keep..rows).map(|row| {
        aggregate(
            values[0][row],
            values[1][row],
            values[2][row],
            values[3][row],
        )
    }));
    (cache, rows - keep)
}

#[derive(Clone, Copy)]
pub(crate) struct IndicatorChange {
    pub(crate) from: usize,
    pub(crate) previous_generation: u64,
    pub(crate) full_replace: bool,
}

pub const EMA_RIBBON_DEFAULT_PERIODS: [usize; aeris_charts_indicators::MAX_OUTPUTS] =
    [5, 10, 20, 50, 200];
pub const EMA_RIBBON_DEFAULT_COLORS: [&str; aeris_charts_indicators::MAX_OUTPUTS] =
    ["#335cff", "#FF9800", "#7d52f4", "#fb4ba3", "#fb3748"];

/// Chart-wide chrome policy for engine-owned indicator bindings.
///
/// The engine retains this policy so newly-created and restored bindings cannot silently diverge
/// from existing outputs. Hosts choose the preference; they do not walk output series to enforce
/// it themselves.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct IndicatorChromeOptions {
    pub name_labels_visible: bool,
    pub value_labels_visible: bool,
    pub price_lines_visible: bool,
}

impl Default for IndicatorChromeOptions {
    fn default() -> Self {
        Self {
            name_labels_visible: true,
            value_labels_visible: true,
            price_lines_visible: true,
        }
    }
}

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
    pub kind: Cow<'static, str>,
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
    pub output_name: Cow<'static, str>,
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
    pub calendar: Option<StudyCalendarPolicy>,
    pub previous_period: Option<PreviousPeriod>,
    pub duration_seconds: Option<u32>,
    pub left: Option<usize>,
    pub right: Option<usize>,
    pub break_on: Option<StructureBreakOn>,
    pub min_size: Option<f64>,
    pub mitigation: Option<StructureMitigation>,
    pub mitigation_price: Option<StructureMitigationPrice>,
    pub max_active: Option<usize>,
    pub show_mitigated: Option<bool>,
    pub zone: Option<OrderBlockZone>,
    pub period: Option<usize>,
    pub periods: Option<[usize; aeris_charts_indicators::MAX_OUTPUTS]>,
    pub pivot_kind: Option<aeris_charts_indicators::PivotKind>,
    pub deviation_percent: Option<f64>,
    pub deviation: Option<f64>,
    pub multiplier: Option<f64>,
    pub fast: Option<usize>,
    pub slow: Option<usize>,
    pub signal: Option<usize>,
    pub k_period: Option<usize>,
    pub d_period: Option<usize>,
    pub reset: Option<aeris_charts_indicators::VwapReset>,
    pub standard_deviation: Option<f64>,
    pub percent: Option<f64>,
    pub exponential: Option<bool>,
    pub offset: Option<f64>,
    pub sigma: Option<f64>,
    pub divisor: Option<f64>,
    pub annualization: Option<f64>,
    pub long_period: Option<usize>,
    pub short_period: Option<usize>,
    pub smoothing: Option<usize>,
    pub roc: Option<[usize; 4]>,
    pub smoothing_periods: Option<[usize; 4]>,
    pub ema_period: Option<usize>,
    pub sum_period: Option<usize>,
    pub seed: Option<aeris_charts_indicators::IndicatorSeed>,
    pub histogram_multiplier: Option<f64>,
    pub estimator: Option<aeris_charts_indicators::DeviationEstimator>,
    pub k_smoothing: Option<usize>,
    pub d_smoothing: Option<usize>,
    /// KDJ K/D start (the `seed` field of a KDJ definition).
    pub kdj_seed: Option<aeris_charts_indicators::KdjSeed>,
    /// The full definition of a KLineChart indicator binding.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub klinechart: Option<aeris_charts_indicators::klinechart::Indicator>,
    pub custom: Option<BTreeMap<String, serde_json::Value>>,
}

fn indicator_default_line_width(kind: &IndicatorKind) -> f64 {
    if matches!(
        kind,
        IndicatorKind::Ema { .. }
            | IndicatorKind::Dema { .. }
            | IndicatorKind::Tema { .. }
            | IndicatorKind::EmaRibbon { .. }
    ) {
        1.0
    } else {
        2.0
    }
}

impl ChartEngine {
    pub fn add_session_levels(
        &mut self,
        source: SeriesId,
        calendar: StudyCalendarPolicy,
    ) -> Vec<SeriesId> {
        self.add_indicator_kind(source, IndicatorKind::SessionLevels { calendar }, None)
    }

    pub fn add_previous_period_levels(
        &mut self,
        source: SeriesId,
        period: PreviousPeriod,
        calendar: StudyCalendarPolicy,
    ) -> Vec<SeriesId> {
        self.add_indicator_kind(
            source,
            IndicatorKind::PreviousPeriodLevels { period, calendar },
            None,
        )
    }

    pub fn add_opening_range(
        &mut self,
        source: SeriesId,
        duration_seconds: u32,
        calendar: StudyCalendarPolicy,
    ) -> Vec<SeriesId> {
        self.add_indicator_kind(
            source,
            IndicatorKind::OpeningRange {
                duration_seconds,
                calendar,
            },
            None,
        )
    }

    /// Add confirmed swing levels on the source price pane.
    pub fn add_swing_points(
        &mut self,
        source: SeriesId,
        left: usize,
        right: usize,
    ) -> Vec<SeriesId> {
        self.add_indicator_kind(source, IndicatorKind::SwingPoints { left, right }, None)
    }

    pub fn add_market_structure(
        &mut self,
        source: SeriesId,
        left: usize,
        right: usize,
        break_on: StructureBreakOn,
    ) -> Vec<SeriesId> {
        self.add_indicator_kind(
            source,
            IndicatorKind::MarketStructure {
                left,
                right,
                break_on,
            },
            None,
        )
    }

    pub fn add_fair_value_gaps(
        &mut self,
        source: SeriesId,
        min_size: f64,
        mitigation: StructureMitigation,
        mitigation_price: StructureMitigationPrice,
        max_active: usize,
        show_mitigated: bool,
    ) -> Vec<SeriesId> {
        self.add_indicator_kind(
            source,
            IndicatorKind::FairValueGaps {
                min_size,
                mitigation,
                mitigation_price,
                max_active,
                show_mitigated,
            },
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn add_order_blocks(
        &mut self,
        source: SeriesId,
        left: usize,
        right: usize,
        break_on: StructureBreakOn,
        zone: OrderBlockZone,
        mitigation: StructureMitigation,
        mitigation_price: StructureMitigationPrice,
        max_active: usize,
        show_mitigated: bool,
    ) -> Vec<SeriesId> {
        self.add_indicator_kind(
            source,
            IndicatorKind::OrderBlocks {
                left,
                right,
                break_on,
                zone,
                mitigation,
                mitigation_price,
                max_active,
                show_mitigated,
            },
            None,
        )
    }

    pub(crate) fn reset_indicator_output_styles_to_defaults(&mut self) {
        let outputs = self
            .indicators
            .iter()
            .map(|binding| (binding.kind.clone(), binding.outputs.clone()))
            .collect::<Vec<_>>();
        for (kind, ids) in outputs {
            for (output_index, id) in ids.into_iter().enumerate() {
                let custom_style = match &kind {
                    IndicatorKind::Custom {
                        type_id, version, ..
                    } => self
                        .custom_studies
                        .get(type_id)
                        .filter(|registration| registration.definition.version == *version)
                        .and_then(|registration| registration.definition.outputs.get(output_index))
                        .map(|output| output.default_style.clone()),
                    _ => None,
                };
                let Some(series) = self.series_entry_mut(id) else {
                    continue;
                };
                series.countdown_visible = false;
                series.title_visible = true;
                series.line_width = Some(indicator_default_line_width(&kind));
                series.line_color = indicator_output_color(&kind, output_index).map(str::to_string);
                if let IndicatorKind::KLineChart(indicator) = &kind {
                    apply_klinechart_output_style(series, indicator, output_index);
                }
                if let Some(style) = custom_style {
                    series.visible = style.visible;
                    series.line_width = style.line_width;
                    series.line_color = style.line_color;
                    series.line_style = style.line_style;
                    series.point_markers = style.point_markers;
                    series.up_color = style.up_color;
                    series.down_color = style.down_color;
                    series.area_top_color = style.area_top_color;
                    series.area_bottom_color = style.area_bottom_color;
                }
                if matches!(
                    kind,
                    IndicatorKind::MarketStructure { .. } | IndicatorKind::OrderBlocks { .. }
                ) {
                    series.line_style = 2;
                }
            }
        }
    }

    /// Applies Aeris's canonical four-state momentum palette to an existing histogram series.
    ///
    /// Colors are derived from each value's sign and whether it moved toward or away from zero.
    /// The caller owns only the semantic request; Aeris retains palette and row-style ownership.
    ///
    /// Returns `false` for an unknown or non-histogram series and for a source-owned one (see
    /// [`ChartEngine::series_is_source_owned`]), such as the trade delta and volume studies, which
    /// keep the palette their stream installs. Indicator outputs are not source-owned and accept it.
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
                usage.0
                    + match &binding.runtime {
                        BindingRuntime::BuiltIn(runtime) => runtime.runtime_bytes(),
                        BindingRuntime::Custom(_) => 0,
                    }
                    + binding.inputs.bytes()
                    + binding
                        .structure
                        .as_ref()
                        .map_or(0, StructureStudy::capacity_bytes)
                    + binding
                        .session
                        .as_ref()
                        .map_or(0, SessionStudyState::capacity_bytes)
                    + binding
                        .annotations
                        .as_ref()
                        .map_or(0, StudyAnnotations::capacity_bytes),
                usage.1
                    + match &binding.runtime {
                        BindingRuntime::BuiltIn(runtime) => runtime.transfer_capacity_bytes(),
                        BindingRuntime::Custom(_) => 0,
                    },
            )
        })
    }

    /// Rows of work each binding's most recent rebuild performed, summed over bindings: formula
    /// rows its runtime evaluated plus aggregate-input and weight-alignment rows it derived.
    pub fn last_indicator_work_rows(&self) -> usize {
        self.indicators
            .iter()
            .map(IndicatorBinding::last_work_rows)
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

    // ponytail: annotations are display-only (no hit target, selection or hover). Interaction,
    // if a product needs it, belongs in the engine input controller (`chart_input.rs`) querying
    // the same interval index, never in a host.
    /// Snapshot the bounded annotation history of a structural study by its binding identity.
    ///
    /// An ordinary scalar binding has no annotation output; passing an output other than the
    /// binding's first output is not a binding identity.
    pub fn study_annotations(&self, binding: SeriesId) -> Result<StudyAnnotations, ChartError> {
        let producer = self
            .indicators
            .iter()
            .find(|producer| producer.outputs.first() == Some(&binding))
            .ok_or_else(|| ChartError::new(ErrorCode::InvalidHandle, "unknown study binding"))?;
        producer
            .annotations
            .as_ref()
            .or_else(|| producer.structure.as_ref().map(StructureStudy::annotations))
            .cloned()
            .ok_or_else(|| {
                ChartError::new(
                    ErrorCode::UnsupportedOperation,
                    "study binding has no structural annotations",
                )
            })
    }

    /// Install bounded test geometry without exposing an incomplete structural-study kind.
    #[cfg(test)]
    pub(crate) fn inject_study_annotations_for_test(
        &mut self,
        binding: SeriesId,
        annotations: StudyAnnotations,
    ) -> bool {
        let Some(producer) = self
            .indicators
            .iter_mut()
            .find(|producer| producer.outputs.first() == Some(&binding))
        else {
            return false;
        };
        producer.annotations = Some(annotations);
        let source = producer.source;
        let outputs = producer.outputs.clone();
        self.invalidate_frame_series(source);
        for output in outputs {
            self.invalidate_frame_series(output);
        }
        true
    }

    /// Whether the chart currently owns at least one live native indicator binding.
    #[must_use]
    pub fn has_indicator_bindings(&self) -> bool {
        !self.indicators.is_empty()
    }

    /// Remove every native indicator binding as one engine-owned operation.
    pub fn clear_indicator_bindings(&mut self) -> bool {
        let binding_ids = self
            .indicators
            .iter()
            .filter_map(|binding| binding.outputs.first().copied())
            .collect::<Vec<_>>();
        if binding_ids.is_empty() {
            return false;
        }
        for binding_id in binding_ids {
            let _ = self.remove_indicator_binding(binding_id);
        }
        true
    }

    /// Current chart-wide chrome policy inherited by every engine-owned indicator output.
    #[must_use]
    pub const fn indicator_chrome_options(&self) -> IndicatorChromeOptions {
        self.indicator_chrome
    }

    /// Apply one chart-wide indicator chrome policy to current and future bindings.
    pub fn set_indicator_chrome_options(&mut self, options: IndicatorChromeOptions) -> bool {
        let mut changed = self.indicator_chrome != options;
        self.indicator_chrome = options;
        let outputs = self
            .indicators
            .iter()
            .flat_map(|binding| binding.outputs.iter().copied())
            .collect::<Vec<_>>();
        for output in outputs {
            if let Some(series) = self.series_entry_mut(output) {
                changed |= series.title_visible != options.name_labels_visible
                    || series.last_value_visible != options.value_labels_visible
                    || series.price_line_visible != options.price_lines_visible;
                series.title_visible = options.name_labels_visible;
                series.last_value_visible = options.value_labels_visible;
                series.price_line_visible = options.price_lines_visible;
            }
        }
        changed |= self.apply_indicator_chrome_to_external_studies(options);
        changed |= self.apply_indicator_chrome_to_trade_studies(options);
        if changed {
            self.invalidate_frame_layout_and_axis();
        }
        changed
    }

    /// Set every output in one binding visible or hidden as one engine-owned operation.
    pub fn set_indicator_binding_visible(&mut self, binding_id: SeriesId, visible: bool) -> bool {
        let Some(outputs) = self
            .indicators
            .iter()
            .find(|binding| binding.outputs.first() == Some(&binding_id))
            .map(|binding| binding.outputs.clone())
        else {
            return false;
        };
        let changed = outputs.iter().any(|&output| {
            self.series_entry(output)
                .is_some_and(|series| series.visible != visible)
        });
        for output in outputs {
            self.set_series_visible(output, visible);
        }
        changed
    }

    /// Remove one complete indicator binding by its stable binding identity.
    pub fn remove_indicator_binding(&mut self, binding_id: SeriesId) -> bool {
        if !self
            .indicators
            .iter()
            .any(|binding| binding.outputs.first() == Some(&binding_id))
        {
            return false;
        }
        self.remove_series(binding_id)
    }

    /// Remove the complete native indicator binding that owns one output series.
    pub fn remove_indicator_for_series(&mut self, series_id: SeriesId) -> bool {
        let Some(binding_id) = self.indicator_binding_id(series_id) else {
            return false;
        };
        self.remove_indicator_binding(binding_id)
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
        self.invalidate_frame_series(output);
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
                        IndicatorKind::Custom { ref parameters, .. } => (
                            "custom",
                            0,
                            None,
                            IndicatorParameters {
                                custom: Some(parameters.clone()),
                                ..Default::default()
                            },
                        ),
                        IndicatorKind::SwingPoints { left, right } => (
                            "swing_points",
                            0,
                            None,
                            IndicatorParameters {
                                left: Some(left),
                                right: Some(right),
                                ..Default::default()
                            },
                        ),
                        IndicatorKind::MarketStructure {
                            left,
                            right,
                            break_on,
                        } => (
                            "market_structure",
                            0,
                            None,
                            IndicatorParameters {
                                left: Some(left),
                                right: Some(right),
                                break_on: Some(break_on),
                                ..Default::default()
                            },
                        ),
                        IndicatorKind::FairValueGaps {
                            min_size,
                            mitigation,
                            mitigation_price,
                            max_active,
                            show_mitigated,
                        } => (
                            "fair_value_gaps",
                            0,
                            None,
                            IndicatorParameters {
                                min_size: Some(min_size),
                                mitigation: Some(mitigation),
                                mitigation_price: Some(mitigation_price),
                                max_active: Some(max_active),
                                show_mitigated: Some(show_mitigated),
                                ..Default::default()
                            },
                        ),
                        IndicatorKind::OrderBlocks {
                            left,
                            right,
                            break_on,
                            zone,
                            mitigation,
                            mitigation_price,
                            max_active,
                            show_mitigated,
                        } => (
                            "order_blocks",
                            0,
                            None,
                            IndicatorParameters {
                                left: Some(left),
                                right: Some(right),
                                break_on: Some(break_on),
                                zone: Some(zone),
                                mitigation: Some(mitigation),
                                mitigation_price: Some(mitigation_price),
                                max_active: Some(max_active),
                                show_mitigated: Some(show_mitigated),
                                ..Default::default()
                            },
                        ),
                        IndicatorKind::SessionLevels { calendar } => (
                            "session_levels",
                            0,
                            None,
                            IndicatorParameters {
                                calendar: Some(calendar),
                                ..Default::default()
                            },
                        ),
                        IndicatorKind::PreviousPeriodLevels { period, calendar } => (
                            "previous_period_levels",
                            0,
                            None,
                            IndicatorParameters {
                                calendar: Some(calendar),
                                previous_period: Some(period),
                                ..Default::default()
                            },
                        ),
                        IndicatorKind::OpeningRange {
                            duration_seconds,
                            calendar,
                        } => (
                            "opening_range",
                            0,
                            None,
                            IndicatorParameters {
                                calendar: Some(calendar),
                                duration_seconds: Some(duration_seconds),
                                ..Default::default()
                            },
                        ),
                        IndicatorKind::Aroon { period } => (
                            "aroon",
                            period,
                            None,
                            IndicatorParameters {
                                period: Some(period),
                                ..IndicatorParameters::default()
                            },
                        ),
                        IndicatorKind::AwesomeOscillator => (
                            "awesome_oscillator",
                            0,
                            None,
                            IndicatorParameters::default(),
                        ),
                        IndicatorKind::Dpo { period } => (
                            "dpo",
                            period,
                            None,
                            IndicatorParameters {
                                period: Some(period),
                                ..IndicatorParameters::default()
                            },
                        ),
                        IndicatorKind::ChandeMomentum { period } => (
                            "chande_momentum",
                            period,
                            None,
                            IndicatorParameters {
                                period: Some(period),
                                ..IndicatorParameters::default()
                            },
                        ),
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
                        IndicatorKind::BollingerMetrics { period, deviation } => (
                            "bollinger_metrics",
                            period,
                            Some(deviation),
                            IndicatorParameters {
                                period: Some(period),
                                deviation: Some(deviation),
                                ..IndicatorParameters::default()
                            },
                        ),
                        IndicatorKind::Envelopes {
                            period,
                            percent,
                            exponential,
                        } => (
                            "envelopes",
                            period,
                            Some(percent),
                            IndicatorParameters {
                                period: Some(period),
                                percent: Some(percent),
                                exponential: Some(exponential),
                                ..IndicatorParameters::default()
                            },
                        ),
                        IndicatorKind::Alma {
                            period,
                            offset,
                            sigma,
                        } => (
                            "alma",
                            period,
                            None,
                            IndicatorParameters {
                                period: Some(period),
                                offset: Some(offset),
                                sigma: Some(sigma),
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
                        IndicatorKind::AccumulationDistribution => (
                            "accumulation_distribution",
                            0,
                            None,
                            IndicatorParameters::default(),
                        ),
                        IndicatorKind::PriceVolumeTrend => (
                            "price_volume_trend",
                            0,
                            None,
                            IndicatorParameters::default(),
                        ),
                        IndicatorKind::ChaikinOscillator { fast, slow } => (
                            "chaikin_oscillator",
                            0,
                            None,
                            IndicatorParameters {
                                fast: Some(fast),
                                slow: Some(slow),
                                ..IndicatorParameters::default()
                            },
                        ),
                        IndicatorKind::Klinger { fast, slow, signal } => (
                            "klinger",
                            slow,
                            Some(signal as f64),
                            IndicatorParameters {
                                fast: Some(fast),
                                slow: Some(slow),
                                signal: Some(signal),
                                ..IndicatorParameters::default()
                            },
                        ),
                        IndicatorKind::Kama { period, fast, slow } => (
                            "kama",
                            period,
                            None,
                            IndicatorParameters {
                                period: Some(period),
                                fast: Some(fast),
                                slow: Some(slow),
                                ..IndicatorParameters::default()
                            },
                        ),
                        IndicatorKind::McGinley { period } => (
                            "mcginley",
                            period,
                            None,
                            IndicatorParameters {
                                period: Some(period),
                                ..IndicatorParameters::default()
                            },
                        ),
                        IndicatorKind::LinearRegression { period, deviation } => (
                            "linear_regression",
                            period,
                            Some(deviation),
                            IndicatorParameters {
                                period: Some(period),
                                deviation: Some(deviation),
                                ..IndicatorParameters::default()
                            },
                        ),
                        IndicatorKind::Choppiness { period } => (
                            "choppiness",
                            period,
                            None,
                            IndicatorParameters {
                                period: Some(period),
                                ..IndicatorParameters::default()
                            },
                        ),
                        IndicatorKind::AtrBands { period, multiplier } => (
                            "atr_bands",
                            period,
                            Some(multiplier),
                            IndicatorParameters {
                                period: Some(period),
                                multiplier: Some(multiplier),
                                ..IndicatorParameters::default()
                            },
                        ),
                        IndicatorKind::RelativeVolume { period } => (
                            "relative_volume",
                            period,
                            None,
                            IndicatorParameters {
                                period: Some(period),
                                ..IndicatorParameters::default()
                            },
                        ),
                        IndicatorKind::VolumeOscillator { fast, slow, signal } => (
                            "volume_oscillator",
                            slow,
                            Some(signal as f64),
                            IndicatorParameters {
                                fast: Some(fast),
                                slow: Some(slow),
                                signal: Some(signal),
                                ..IndicatorParameters::default()
                            },
                        ),
                        IndicatorKind::ElderForce { period } => (
                            "elder_force",
                            period,
                            None,
                            IndicatorParameters {
                                period: Some(period),
                                ..IndicatorParameters::default()
                            },
                        ),
                        IndicatorKind::EaseOfMovement { period, divisor } => (
                            "ease_of_movement",
                            period,
                            None,
                            IndicatorParameters {
                                period: Some(period),
                                divisor: Some(divisor),
                                ..IndicatorParameters::default()
                            },
                        ),
                        IndicatorKind::HistoricalVolatility {
                            period,
                            annualization,
                        } => (
                            "historical_volatility",
                            period,
                            None,
                            IndicatorParameters {
                                period: Some(period),
                                annualization: Some(annualization),
                                ..IndicatorParameters::default()
                            },
                        ),
                        IndicatorKind::Trix { period, signal } => (
                            "trix",
                            period,
                            Some(signal as f64),
                            IndicatorParameters {
                                period: Some(period),
                                signal: Some(signal),
                                ..IndicatorParameters::default()
                            },
                        ),
                        IndicatorKind::Kst {
                            roc,
                            smoothing,
                            signal,
                        } => (
                            "kst",
                            0,
                            None,
                            IndicatorParameters {
                                roc: Some(roc),
                                smoothing_periods: Some(smoothing),
                                signal: Some(signal),
                                ..IndicatorParameters::default()
                            },
                        ),
                        IndicatorKind::Tsi {
                            long,
                            short,
                            signal,
                        } => (
                            "tsi",
                            long,
                            None,
                            IndicatorParameters {
                                long_period: Some(long),
                                short_period: Some(short),
                                signal: Some(signal),
                                ..IndicatorParameters::default()
                            },
                        ),
                        IndicatorKind::MassIndex {
                            ema_period,
                            sum_period,
                        } => (
                            "mass_index",
                            ema_period,
                            None,
                            IndicatorParameters {
                                ema_period: Some(ema_period),
                                sum_period: Some(sum_period),
                                ..IndicatorParameters::default()
                            },
                        ),
                        IndicatorKind::Vortex { period } => (
                            "vortex",
                            period,
                            None,
                            IndicatorParameters {
                                period: Some(period),
                                ..IndicatorParameters::default()
                            },
                        ),
                        IndicatorKind::CoppockCurve {
                            long,
                            short,
                            smoothing,
                        } => (
                            "coppock_curve",
                            smoothing,
                            None,
                            IndicatorParameters {
                                long_period: Some(long),
                                short_period: Some(short),
                                smoothing: Some(smoothing),
                                ..IndicatorParameters::default()
                            },
                        ),
                        IndicatorKind::FisherTransform { period } => (
                            "fisher_transform",
                            period,
                            None,
                            IndicatorParameters {
                                period: Some(period),
                                ..IndicatorParameters::default()
                            },
                        ),
                        IndicatorKind::UltimateOscillator {
                            short,
                            medium,
                            long,
                        } => (
                            "ultimate_oscillator",
                            medium,
                            None,
                            IndicatorParameters {
                                short_period: Some(short),
                                period: Some(medium),
                                long_period: Some(long),
                                ..IndicatorParameters::default()
                            },
                        ),
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
                        IndicatorKind::KLineChart(ref indicator) => (
                            klinechart_kind_name(indicator),
                            klinechart_primary_period(indicator),
                            None,
                            IndicatorParameters {
                                klinechart: Some(indicator.clone()),
                                ..IndicatorParameters::default()
                            },
                        ),
                    };
                    let (warmup_bars, convergence_bars) = self.indicator_output_warmup(id);
                    IndicatorInfo {
                        binding_id: binding.outputs[0],
                        kind: if let IndicatorKind::Custom { ref type_id, .. } = binding.kind {
                            Cow::Owned(type_id.clone())
                        } else {
                            Cow::Borrowed(kind)
                        },
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
                        output_name: if let IndicatorKind::Custom {
                            ref type_id,
                            ref version,
                            ..
                        } = binding.kind
                        {
                            Cow::Owned(
                                self.custom_studies
                                    .get(type_id)
                                    .filter(|d| d.definition.version == *version)
                                    .and_then(|d| d.definition.outputs.get(output_index))
                                    .map_or_else(
                                        || format!("Output {}", output_index + 1),
                                        |o| o.name.clone(),
                                    ),
                            )
                        } else {
                            Cow::Borrowed(indicator_output_name(&binding.kind, output_index))
                        },
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
            let (own_warmup, own_convergence) = binding.output_warmup(output_index);
            warmup = warmup.saturating_add(own_warmup);
            convergence = convergence
                .zip(own_convergence)
                .map(|(total, own)| total.saturating_add(own));
            current = binding.source;
        }
        (warmup, convergence)
    }

    /// Add a Rust-native simple moving-average producer. The returned line series is owned by the
    /// engine and is recomputed whenever its source series changes.
    pub fn add_aroon(&mut self, source: SeriesId, period: usize) -> Vec<SeriesId> {
        self.add_indicator_kind(source, IndicatorKind::Aroon { period }, None)
    }

    pub fn add_awesome_oscillator(&mut self, source: SeriesId) -> Option<SeriesId> {
        self.add_indicator_kind(source, IndicatorKind::AwesomeOscillator, None)
            .into_iter()
            .next()
    }

    pub fn add_dpo(&mut self, source: SeriesId, period: usize) -> Option<SeriesId> {
        self.add_indicator_kind(source, IndicatorKind::Dpo { period }, None)
            .into_iter()
            .next()
    }

    pub fn add_chande_momentum(&mut self, source: SeriesId, period: usize) -> Option<SeriesId> {
        self.add_indicator_kind(source, IndicatorKind::ChandeMomentum { period }, None)
            .into_iter()
            .next()
    }

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
            if let Some(series) = self.series.iter_mut().find(|series| series.id == output)
                && series.title == previous_title
            {
                series.title = format!("EMA {}", periods[output_index]);
            }
        }
        let kind = IndicatorKind::EmaRibbon { periods };
        self.indicators[index].kind = kind.clone();
        self.indicators[index].runtime =
            BindingRuntime::BuiltIn(Box::new(incremental_state(&kind)));
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

    pub fn add_bollinger_metrics(
        &mut self,
        source: SeriesId,
        period: usize,
        deviation: f64,
    ) -> Vec<SeriesId> {
        self.add_indicator_kind(
            source,
            IndicatorKind::BollingerMetrics { period, deviation },
            None,
        )
    }

    pub fn add_envelopes(
        &mut self,
        source: SeriesId,
        period: usize,
        percent: f64,
        exponential: bool,
    ) -> Vec<SeriesId> {
        self.add_indicator_kind(
            source,
            IndicatorKind::Envelopes {
                period,
                percent,
                exponential,
            },
            None,
        )
    }

    pub fn add_alma(
        &mut self,
        source: SeriesId,
        period: usize,
        offset: f64,
        sigma: f64,
    ) -> Option<SeriesId> {
        self.add_indicator_kind(
            source,
            IndicatorKind::Alma {
                period,
                offset,
                sigma,
            },
            None,
        )
        .into_iter()
        .next()
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

    pub fn add_accumulation_distribution(
        &mut self,
        source: SeriesId,
        volume_source: SeriesId,
    ) -> Option<SeriesId> {
        self.add_indicator_kind(
            source,
            IndicatorKind::AccumulationDistribution,
            Some(volume_source),
        )
        .into_iter()
        .next()
    }

    pub fn add_price_volume_trend(
        &mut self,
        source: SeriesId,
        volume_source: SeriesId,
    ) -> Option<SeriesId> {
        self.add_indicator_kind(source, IndicatorKind::PriceVolumeTrend, Some(volume_source))
            .into_iter()
            .next()
    }

    pub fn add_chaikin_oscillator(
        &mut self,
        source: SeriesId,
        volume_source: SeriesId,
        fast: usize,
        slow: usize,
    ) -> Option<SeriesId> {
        self.add_indicator_kind(
            source,
            IndicatorKind::ChaikinOscillator { fast, slow },
            Some(volume_source),
        )
        .into_iter()
        .next()
    }

    pub fn add_klinger(
        &mut self,
        source: SeriesId,
        volume_source: SeriesId,
        fast: usize,
        slow: usize,
        signal: usize,
    ) -> Vec<SeriesId> {
        self.add_indicator_kind(
            source,
            IndicatorKind::Klinger { fast, slow, signal },
            Some(volume_source),
        )
    }

    pub fn add_kama(
        &mut self,
        source: SeriesId,
        period: usize,
        fast: usize,
        slow: usize,
    ) -> Option<SeriesId> {
        self.add_indicator_kind(source, IndicatorKind::Kama { period, fast, slow }, None)
            .into_iter()
            .next()
    }

    pub fn add_mcginley(&mut self, source: SeriesId, period: usize) -> Option<SeriesId> {
        self.add_indicator_kind(source, IndicatorKind::McGinley { period }, None)
            .into_iter()
            .next()
    }

    pub fn add_linear_regression(
        &mut self,
        source: SeriesId,
        period: usize,
        deviation: f64,
    ) -> Vec<SeriesId> {
        self.add_indicator_kind(
            source,
            IndicatorKind::LinearRegression { period, deviation },
            None,
        )
    }

    /// Add a Choppiness Index (0–100) line in its own oscillator pane (with the 38.2/61.8 Chop
    /// Zone channel and dotted band lines).
    pub fn add_choppiness(&mut self, source: SeriesId, period: usize) -> Option<SeriesId> {
        self.add_indicator_kind(source, IndicatorKind::Choppiness { period }, None)
            .into_iter()
            .next()
    }

    pub fn add_atr_bands(
        &mut self,
        source: SeriesId,
        period: usize,
        multiplier: f64,
    ) -> Vec<SeriesId> {
        self.add_indicator_kind(source, IndicatorKind::AtrBands { period, multiplier }, None)
    }

    pub fn add_relative_volume(
        &mut self,
        source: SeriesId,
        volume_source: SeriesId,
        period: usize,
    ) -> Option<SeriesId> {
        self.add_indicator_kind(
            source,
            IndicatorKind::RelativeVolume { period },
            Some(volume_source),
        )
        .into_iter()
        .next()
    }

    pub fn add_volume_oscillator(
        &mut self,
        source: SeriesId,
        volume_source: SeriesId,
        fast: usize,
        slow: usize,
        signal: usize,
    ) -> Vec<SeriesId> {
        self.add_indicator_kind(
            source,
            IndicatorKind::VolumeOscillator { fast, slow, signal },
            Some(volume_source),
        )
    }

    pub fn add_elder_force(
        &mut self,
        source: SeriesId,
        volume_source: SeriesId,
        period: usize,
    ) -> Option<SeriesId> {
        self.add_indicator_kind(
            source,
            IndicatorKind::ElderForce { period },
            Some(volume_source),
        )
        .into_iter()
        .next()
    }

    pub fn add_ease_of_movement(
        &mut self,
        source: SeriesId,
        volume_source: SeriesId,
        period: usize,
        divisor: f64,
    ) -> Option<SeriesId> {
        self.add_indicator_kind(
            source,
            IndicatorKind::EaseOfMovement { period, divisor },
            Some(volume_source),
        )
        .into_iter()
        .next()
    }

    pub fn add_historical_volatility(
        &mut self,
        source: SeriesId,
        period: usize,
        annualization: f64,
    ) -> Option<SeriesId> {
        self.add_indicator_kind(
            source,
            IndicatorKind::HistoricalVolatility {
                period,
                annualization,
            },
            None,
        )
        .into_iter()
        .next()
    }

    pub fn add_trix(&mut self, source: SeriesId, period: usize, signal: usize) -> Vec<SeriesId> {
        self.add_indicator_kind(source, IndicatorKind::Trix { period, signal }, None)
    }

    pub fn add_kst(
        &mut self,
        source: SeriesId,
        roc: [usize; 4],
        smoothing: [usize; 4],
        signal: usize,
    ) -> Vec<SeriesId> {
        self.add_indicator_kind(
            source,
            IndicatorKind::Kst {
                roc,
                smoothing,
                signal,
            },
            None,
        )
    }

    pub fn add_tsi(
        &mut self,
        source: SeriesId,
        long: usize,
        short: usize,
        signal: usize,
    ) -> Vec<SeriesId> {
        self.add_indicator_kind(
            source,
            IndicatorKind::Tsi {
                long,
                short,
                signal,
            },
            None,
        )
    }

    pub fn add_mass_index(
        &mut self,
        source: SeriesId,
        ema_period: usize,
        sum_period: usize,
    ) -> Option<SeriesId> {
        self.add_indicator_kind(
            source,
            IndicatorKind::MassIndex {
                ema_period,
                sum_period,
            },
            None,
        )
        .into_iter()
        .next()
    }

    pub fn add_vortex(&mut self, source: SeriesId, period: usize) -> Vec<SeriesId> {
        self.add_indicator_kind(source, IndicatorKind::Vortex { period }, None)
    }

    pub fn add_coppock_curve(
        &mut self,
        source: SeriesId,
        long: usize,
        short: usize,
        smoothing: usize,
    ) -> Option<SeriesId> {
        self.add_indicator_kind(
            source,
            IndicatorKind::CoppockCurve {
                long,
                short,
                smoothing,
            },
            None,
        )
        .into_iter()
        .next()
    }

    pub fn add_fisher_transform(&mut self, source: SeriesId, period: usize) -> Vec<SeriesId> {
        self.add_indicator_kind(source, IndicatorKind::FisherTransform { period }, None)
    }

    pub fn add_ultimate_oscillator(
        &mut self,
        source: SeriesId,
        short: usize,
        medium: usize,
        long: usize,
    ) -> Option<SeriesId> {
        self.add_indicator_kind(
            source,
            IndicatorKind::UltimateOscillator {
                short,
                medium,
                long,
            },
            None,
        )
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
    /// lines). K and D start from the textbook 50 after a full RSV window;
    /// `IndicatorKind::Kdj { seed, .. }` selects the formula-language start instead (RSV over the
    /// rows available, first value at row 0).
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
        // OHLC structural rules consume the complete candle. A scalar source override
        // must not be accepted and then silently ignored by their scanner.
        if structure_output_count(&kind).is_some() && source_input != IndicatorInputSource::Close {
            return Vec::new();
        }
        let ids = self.add_indicator(
            source,
            source_input,
            kind.clone(),
            volume_source,
            amount_source,
        );
        match kind {
            IndicatorKind::Aroon { .. } => {
                if !ids.is_empty() {
                    self.place_outputs_in_oscillator_pane(&ids);
                }
            }
            IndicatorKind::Dpo { .. } | IndicatorKind::ChandeMomentum { .. } => {
                if !ids.is_empty() {
                    self.place_outputs_in_oscillator_pane(&ids);
                }
            }
            IndicatorKind::BollingerMetrics { .. } => {
                for &id in &ids {
                    self.place_outputs_in_oscillator_pane(&[id]);
                }
            }
            IndicatorKind::AwesomeOscillator => {
                if let Some(&histogram) = ids.first() {
                    self.convert_series_kind(histogram, SeriesKind::Histogram);
                    self.place_outputs_in_oscillator_pane(&ids);
                }
            }
            IndicatorKind::Rsi { .. } => {
                if !ids.is_empty() {
                    self.place_outputs_in_oscillator_pane(&ids);
                }
            }
            IndicatorKind::Macd { .. } | IndicatorKind::VolumeOscillator { .. } => {
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
            IndicatorKind::Obv
            | IndicatorKind::AccumulationDistribution
            | IndicatorKind::PriceVolumeTrend => {
                if !ids.is_empty() {
                    self.place_outputs_in_oscillator_pane(&ids);
                }
            }
            IndicatorKind::ChaikinOscillator { .. }
            | IndicatorKind::Klinger { .. }
            | IndicatorKind::RelativeVolume { .. }
            | IndicatorKind::ElderForce { .. } => {
                if !ids.is_empty() {
                    self.place_outputs_in_oscillator_pane(&ids);
                }
            }
            IndicatorKind::EaseOfMovement { .. } => {
                if !ids.is_empty() {
                    self.place_outputs_in_oscillator_pane(&ids);
                }
            }
            IndicatorKind::HistoricalVolatility { .. } => {
                if !ids.is_empty() {
                    self.place_outputs_in_oscillator_pane(&ids);
                }
            }
            IndicatorKind::Trix { .. }
            | IndicatorKind::Kst { .. }
            | IndicatorKind::Tsi { .. }
            | IndicatorKind::MassIndex { .. }
            | IndicatorKind::Vortex { .. }
            | IndicatorKind::Choppiness { .. } => {
                if !ids.is_empty() {
                    self.place_outputs_in_oscillator_pane(&ids);
                }
            }
            IndicatorKind::CoppockCurve { .. } => {
                if !ids.is_empty() {
                    self.place_outputs_in_oscillator_pane(&ids);
                }
            }
            IndicatorKind::FisherTransform { .. } => {
                if !ids.is_empty() {
                    self.place_outputs_in_oscillator_pane(&ids);
                }
            }
            IndicatorKind::UltimateOscillator { .. } => {
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
            IndicatorKind::KLineChart(indicator) => {
                if !ids.is_empty() {
                    self.lay_out_klinechart_outputs(&indicator, &ids);
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
            | IndicatorKind::Envelopes { .. }
            | IndicatorKind::Alma { .. }
            | IndicatorKind::ParabolicSar
            | IndicatorKind::SuperTrend { .. }
            | IndicatorKind::Ichimoku
            | IndicatorKind::Vwap
            | IndicatorKind::VwapBands { .. }
            | IndicatorKind::Kama { .. }
            | IndicatorKind::McGinley { .. }
            | IndicatorKind::LinearRegression { .. }
            | IndicatorKind::AtrBands { .. }
            | IndicatorKind::SwingPoints { .. }
            | IndicatorKind::MarketStructure { .. }
            | IndicatorKind::FairValueGaps { .. }
            | IndicatorKind::OrderBlocks { .. }
            | IndicatorKind::SessionLevels { .. }
            | IndicatorKind::PreviousPeriodLevels { .. }
            | IndicatorKind::OpeningRange { .. }
            | IndicatorKind::Wma { .. }
            | IndicatorKind::Custom { .. } => {}
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
        if self.indicators[index].structure.is_some() {
            return false;
        }
        let kind = self.indicators[index].kind.clone();
        self.indicators[index].source_input = source_input;
        match &mut self.indicators[index].runtime {
            BindingRuntime::Custom(custom) => custom.state = CustomBindingState::Pending,
            runtime @ BindingRuntime::BuiltIn(_) => {
                *runtime = BindingRuntime::BuiltIn(Box::new(incremental_state(&kind)));
            }
        }
        self.indicator_changes.clear();
        let changes = self.rebuild_indicator(index, 0, true);
        self.indicator_changes.extend(changes.into_iter().flatten());
        self.propagate_indicator_changes();
        self.sync_time_points();
        true
    }

    /// Return the bounded typed editor schema for an indicator definition.
    pub fn indicator_schema(kind: &IndicatorKind) -> IndicatorSchema {
        if let IndicatorKind::Custom {
            type_id,
            parameters,
            output_count,
            ..
        } = kind
        {
            return IndicatorSchema {
                revision: INDICATOR_SCHEMA_REVISION,
                kind: type_id.clone(),
                parameters: parameters
                    .iter()
                    .map(|(name, default)| IndicatorParameterDescriptor {
                        name: name.clone(),
                        parameter_type: IndicatorParameterType::Number,
                        default: default.clone(),
                        min: None,
                        max: None,
                        options: None,
                    })
                    .collect(),
                outputs: (0..*output_count)
                    .map(|index| IndicatorOutputDescriptor {
                        name: format!("Output {}", index + 1),
                        index,
                        supports_style: true,
                    })
                    .collect(),
            };
        }
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
                options: None,
            };
        let mut parameters = vec![descriptor(
            "source",
            IndicatorParameterType::Source,
            serde_json::json!(IndicatorInputSource::Close),
            None,
        )];
        if structure_output_count(kind).is_some() {
            parameters.clear();
        }
        if matches!(
            kind,
            IndicatorKind::SessionLevels { .. }
                | IndicatorKind::PreviousPeriodLevels { .. }
                | IndicatorKind::OpeningRange { .. }
        ) {
            parameters.clear();
        }
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
        let boolean = |name: &str, default: bool| {
            descriptor(
                name,
                IndicatorParameterType::Boolean,
                serde_json::json!(default),
                None,
            )
        };
        let choice = |name: &str, default: &str, options: &[&str]| {
            IndicatorParameterDescriptor::choice(name, default, options)
                .expect("built-in choice defaults are listed options")
        };
        let seed = |value: aeris_charts_indicators::IndicatorSeed| {
            choice(
                "seed",
                match value {
                    aeris_charts_indicators::IndicatorSeed::Sma => "sma",
                    aeris_charts_indicators::IndicatorSeed::FirstValue => "first_value",
                },
                &["sma", "first_value"],
            )
        };
        let swing = |parameters: &mut Vec<IndicatorParameterDescriptor>, left, right| {
            for (name, value) in [("left", left), ("right", right)] {
                let mut descriptor = integer(name, value);
                descriptor.max = Some(50.0);
                parameters.push(descriptor);
            }
        };
        let zones = |parameters: &mut Vec<IndicatorParameterDescriptor>,
                     mitigation: StructureMitigation,
                     mitigation_price: StructureMitigationPrice,
                     max_active: usize,
                     show_mitigated: bool| {
            parameters.push(
                IndicatorParameterDescriptor::choice(
                    "mitigation",
                    match mitigation {
                        StructureMitigation::Touch => "touch",
                        StructureMitigation::Half => "half",
                        StructureMitigation::Full => "full",
                    },
                    &["touch", "half", "full"],
                )
                .unwrap(),
            );
            parameters.push(
                IndicatorParameterDescriptor::choice(
                    "mitigation_price",
                    match mitigation_price {
                        StructureMitigationPrice::Wick => "wick",
                        StructureMitigationPrice::Close => "close",
                    },
                    &["wick", "close"],
                )
                .unwrap(),
            );
            let mut active = integer("max_active", max_active);
            active.max = Some(64.0);
            parameters.push(active);
            parameters.push(IndicatorParameterDescriptor {
                name: "show_mitigated".into(),
                parameter_type: IndicatorParameterType::Boolean,
                default: serde_json::json!(show_mitigated),
                min: None,
                max: None,
                options: None,
            });
        };
        match *kind {
            IndicatorKind::SwingPoints { left, right } => swing(&mut parameters, left, right),
            IndicatorKind::MarketStructure {
                left,
                right,
                break_on,
            } => {
                swing(&mut parameters, left, right);
                parameters.push(
                    IndicatorParameterDescriptor::choice(
                        "break_on",
                        if break_on == StructureBreakOn::Close {
                            "close"
                        } else {
                            "wick"
                        },
                        &["close", "wick"],
                    )
                    .unwrap(),
                );
            }
            IndicatorKind::FairValueGaps {
                min_size,
                mitigation,
                mitigation_price,
                max_active,
                show_mitigated,
            } => {
                parameters.push(number("min_size", min_size));
                zones(
                    &mut parameters,
                    mitigation,
                    mitigation_price,
                    max_active,
                    show_mitigated,
                );
            }
            IndicatorKind::OrderBlocks {
                left,
                right,
                break_on,
                zone,
                mitigation,
                mitigation_price,
                max_active,
                show_mitigated,
            } => {
                swing(&mut parameters, left, right);
                parameters.push(
                    IndicatorParameterDescriptor::choice(
                        "break_on",
                        if break_on == StructureBreakOn::Close {
                            "close"
                        } else {
                            "wick"
                        },
                        &["close", "wick"],
                    )
                    .unwrap(),
                );
                parameters.push(
                    IndicatorParameterDescriptor::choice(
                        "zone",
                        if zone == OrderBlockZone::Wick {
                            "wick"
                        } else {
                            "body"
                        },
                        &["wick", "body"],
                    )
                    .unwrap(),
                );
                zones(
                    &mut parameters,
                    mitigation,
                    mitigation_price,
                    max_active,
                    show_mitigated,
                );
            }
            IndicatorKind::SessionLevels { calendar }
            | IndicatorKind::PreviousPeriodLevels { calendar, .. }
            | IndicatorKind::OpeningRange { calendar, .. } => {
                if let IndicatorKind::PreviousPeriodLevels { period, .. } = kind {
                    parameters.push(
                        IndicatorParameterDescriptor::choice(
                            "period",
                            match period {
                                PreviousPeriod::Day => "day",
                                PreviousPeriod::Week => "week",
                                PreviousPeriod::Month => "month",
                            },
                            &["day", "week", "month"],
                        )
                        .unwrap(),
                    );
                }
                if let IndicatorKind::OpeningRange {
                    duration_seconds, ..
                } = kind
                {
                    parameters.push(integer("duration_seconds", *duration_seconds as usize));
                }
                parameters.push(
                    IndicatorParameterDescriptor::choice(
                        "calendar",
                        calendar.name(),
                        &StudyCalendarPolicy::NAMES,
                    )
                    .unwrap(),
                );
            }
            IndicatorKind::Aroon { period } => parameters.push(integer("period", period)),
            IndicatorKind::AwesomeOscillator => {}
            IndicatorKind::Dpo { period } => parameters.push(integer("period", period)),
            IndicatorKind::ChandeMomentum { period } => parameters.push(integer("period", period)),
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
                    match estimator {
                        aeris_charts_indicators::DeviationEstimator::Population => "population",
                        aeris_charts_indicators::DeviationEstimator::Sample => "sample",
                    },
                    &["population", "sample"],
                ));
            }
            IndicatorKind::BollingerMetrics { period, deviation } => {
                parameters.push(integer("period", period));
                parameters.push(number("deviation", deviation));
            }
            IndicatorKind::Envelopes {
                period,
                percent,
                exponential,
            } => {
                parameters.push(integer("period", period));
                parameters.push(number("percent", percent));
                parameters.push(boolean("exponential", exponential));
            }
            IndicatorKind::ChaikinOscillator { fast, slow } => {
                parameters.push(integer("fast", fast));
                parameters.push(integer("slow", slow));
                parameters.push(series("volume_source"));
            }
            IndicatorKind::Klinger { fast, slow, signal } => {
                parameters.push(integer("fast", fast));
                parameters.push(integer("slow", slow));
                parameters.push(integer("signal", signal));
                parameters.push(series("volume_source"));
            }
            IndicatorKind::Kama { period, fast, slow } => {
                parameters.push(integer("period", period));
                parameters.push(integer("fast", fast));
                parameters.push(integer("slow", slow));
            }
            IndicatorKind::McGinley { period } => parameters.push(integer("period", period)),
            IndicatorKind::LinearRegression { period, deviation } => {
                parameters.push(integer("period", period));
                parameters.push(number("deviation", deviation));
            }
            IndicatorKind::Choppiness { period } => {
                let mut descriptor = integer("period", period);
                descriptor.min = Some(2.0);
                parameters.push(descriptor);
            }
            IndicatorKind::AtrBands { period, multiplier } => {
                parameters.push(integer("period", period));
                parameters.push(number("multiplier", multiplier));
            }
            IndicatorKind::RelativeVolume { period } => {
                parameters.push(integer("period", period));
                parameters.push(series("volume_source"));
            }
            IndicatorKind::ElderForce { period } => {
                parameters.push(integer("period", period));
                parameters.push(series("volume_source"));
            }
            IndicatorKind::EaseOfMovement { period, divisor } => {
                parameters.push(integer("period", period));
                parameters.push(number("divisor", divisor));
                parameters.push(series("volume_source"));
            }
            IndicatorKind::HistoricalVolatility {
                period,
                annualization,
            } => {
                parameters.push(integer("period", period));
                parameters.push(number("annualization", annualization));
            }
            IndicatorKind::Trix { period, signal } => {
                parameters.push(integer("period", period));
                parameters.push(integer("signal", signal));
            }
            IndicatorKind::Kst {
                roc,
                smoothing,
                signal,
            } => {
                for (index, period) in roc.into_iter().enumerate() {
                    parameters.push(integer(&format!("roc_{}", index + 1), period));
                }
                for (index, period) in smoothing.into_iter().enumerate() {
                    parameters.push(integer(&format!("smoothing_{}", index + 1), period));
                }
                parameters.push(integer("signal", signal));
            }
            IndicatorKind::Tsi {
                long,
                short,
                signal,
            } => {
                parameters.push(integer("long", long));
                parameters.push(integer("short", short));
                parameters.push(integer("signal", signal));
            }
            IndicatorKind::MassIndex {
                ema_period,
                sum_period,
            } => {
                parameters.push(integer("ema_period", ema_period));
                parameters.push(integer("sum_period", sum_period));
            }
            IndicatorKind::Vortex { period } => parameters.push(integer("period", period)),
            IndicatorKind::CoppockCurve {
                long,
                short,
                smoothing,
            } => {
                parameters.push(integer("long_period", long));
                parameters.push(integer("short_period", short));
                parameters.push(integer("smoothing", smoothing));
            }
            IndicatorKind::FisherTransform { period } => parameters.push(integer("period", period)),
            IndicatorKind::UltimateOscillator {
                short,
                medium,
                long,
            } => {
                parameters.push(integer("short_period", short));
                parameters.push(integer("medium_period", medium));
                parameters.push(integer("long_period", long));
            }
            IndicatorKind::VolumeOscillator { fast, slow, signal } => {
                parameters.push(integer("fast", fast));
                parameters.push(integer("slow", slow));
                parameters.push(integer("signal", signal));
                parameters.push(series("volume_source"));
            }
            IndicatorKind::Alma {
                period,
                offset,
                sigma,
            } => {
                parameters.push(integer("period", period));
                parameters.push(descriptor(
                    "offset",
                    IndicatorParameterType::Number,
                    serde_json::json!(offset),
                    Some((0.0, 1.0)),
                ));
                parameters.push(descriptor(
                    "sigma",
                    IndicatorParameterType::Number,
                    serde_json::json!(sigma),
                    Some((0.01, 1_000_000.0)),
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
                    match seed {
                        aeris_charts_indicators::KdjSeed::Fifty => "fifty",
                        aeris_charts_indicators::KdjSeed::FirstValue => "first_value",
                    },
                    &["fifty", "first_value"],
                ));
            }
            IndicatorKind::Vwap => {
                parameters.push(series("volume_source"));
                parameters.push(series("amount_source"));
            }
            IndicatorKind::Obv
            | IndicatorKind::AccumulationDistribution
            | IndicatorKind::PriceVolumeTrend => parameters.push(series("volume_source")),
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
            IndicatorKind::KLineChart(ref indicator) => {
                for param in indicator.params() {
                    parameters.push(if param.integer {
                        integer(&param.name, param.value as usize)
                    } else {
                        number(&param.name, param.value)
                    });
                }
                if indicator.needs_volume() {
                    parameters.push(series("volume_source"));
                }
            }
            IndicatorKind::Custom { .. } => unreachable!("custom schema handled above"),
        }
        let output_count = session_output_count(kind)
            .or_else(|| structure_output_count(kind))
            .unwrap_or_else(|| incremental_state(kind).output_count());
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
    pub(crate) fn place_outputs_in_oscillator_pane(&mut self, ids: &[SeriesId]) {
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
            p.stretch_factor = crate::SEPARATE_INDICATOR_PANE_STRETCH;
        }
        for &id in ids {
            self.set_series_pane(id, pane, crate::SEPARATE_INDICATOR_PANE_STRETCH);
        }
    }

    /// The band-fill companion for an output series: when `id` is a band UPPER
    /// (output slot 0), the LOWER series (slot 2) the fill closes toward, else `None`. The
    /// frame builder paints the fill between them under the band strokes (the public reference's
    /// background fill).
    pub(crate) fn band_fill_companion(&self, id: SeriesId) -> Option<SeriesId> {
        self.indicators.iter().find_map(|binding| {
            if matches!(
                binding.kind,
                IndicatorKind::Bollinger { .. } | IndicatorKind::Envelopes { .. }
            ) && binding.outputs.first() == Some(&id)
            {
                binding.outputs.get(2).copied()
            } else {
                None
            }
        })
    }

    /// The reset period of the binding that owns output `id`, when its values restart at period
    /// boundaries: session VWAP (typical-price or amount-weighted) and pivot sessions per
    /// exchange trading day, VWAP bands per their configured reset. The frame ends a line run
    /// wherever consecutive drawn rows' [`VwapReset::period_key`]s differ, using the same
    /// trading-day mapping the runtime resets on, so the break and the reset always coincide.
    pub(crate) fn indicator_reset_period(
        &self,
        id: SeriesId,
    ) -> Option<aeris_charts_indicators::VwapReset> {
        let binding = self
            .indicators
            .iter()
            .find(|binding| binding.outputs.contains(&id))?;
        match binding.kind {
            IndicatorKind::Vwap | IndicatorKind::PivotPoints { .. } => {
                Some(aeris_charts_indicators::VwapReset::Session)
            }
            IndicatorKind::VwapBands { reset, .. } => Some(reset),
            _ => None,
        }
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
        if !structure_kind_is_valid(&kind)
            || !self.structure_source_is_supported(source, &kind)
            || self.series_entry(source).is_none()
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
                // AVP reads turnover as the value of a scalar source series.
                IndicatorKind::KLineChart(aeris_charts_indicators::klinechart::Indicator::Avp)
                    if self
                        .series_entry(source)
                        .is_some_and(|series| !series.kind.stores_scalar_values()) =>
                {
                    true
                }
                IndicatorKind::KLineChart(indicator) if indicator.needs_volume() => volume_source
                    .is_none_or(|id| {
                        id == source
                            || self
                                .series_entry(id)
                                .is_none_or(|series| !series.kind.stores_scalar_values())
                    }),
                _ => volume_source.is_some(),
            }
            || match &kind {
                IndicatorKind::Aroon { period } => *period == 0,
                IndicatorKind::AwesomeOscillator => false,
                IndicatorKind::Dpo { period } => *period == 0,
                IndicatorKind::ChandeMomentum { period } => *period == 0,
                IndicatorKind::Sma { period }
                | IndicatorKind::Ema { period, .. }
                | IndicatorKind::Dema { period, .. }
                | IndicatorKind::Tema { period, .. }
                | IndicatorKind::Smma { period }
                | IndicatorKind::Hma { period }
                | IndicatorKind::Vwma { period }
                | IndicatorKind::StandardDeviation { period }
                | IndicatorKind::Donchian { period }
                | IndicatorKind::Rsi { period, .. }
                | IndicatorKind::Atr { period }
                | IndicatorKind::Wma { period } => *period == 0,
                IndicatorKind::Kdj {
                    period,
                    k_smoothing,
                    d_smoothing,
                    ..
                } => *period == 0 || *k_smoothing == 0 || *d_smoothing == 0,
                IndicatorKind::Bollinger {
                    period, deviation, ..
                }
                | IndicatorKind::BollingerMetrics { period, deviation } => {
                    *period == 0 || !deviation.is_finite() || *deviation < 0.0
                }
                IndicatorKind::Envelopes {
                    period, percent, ..
                } => *period == 0 || !percent.is_finite() || *percent < 0.0,
                IndicatorKind::Alma {
                    period,
                    offset,
                    sigma,
                } => {
                    *period == 0
                        || !offset.is_finite()
                        || !(0.0..=1.0).contains(offset)
                        || !sigma.is_finite()
                        || !(0.01..=1_000_000.0).contains(sigma)
                }
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
                IndicatorKind::AccumulationDistribution | IndicatorKind::PriceVolumeTrend => false,
                IndicatorKind::ChaikinOscillator { fast, slow } => {
                    *fast == 0 || *slow == 0 || *fast >= *slow
                }
                IndicatorKind::Klinger { fast, slow, signal } => {
                    *fast == 0 || *fast >= *slow || *signal == 0
                }
                IndicatorKind::Kama { period, fast, slow } => {
                    *period == 0 || *fast == 0 || *fast >= *slow
                }
                IndicatorKind::McGinley { period } => *period == 0,
                IndicatorKind::LinearRegression { period, deviation } => {
                    *period == 0 || !deviation.is_finite() || *deviation < 0.0
                }
                IndicatorKind::Choppiness { period } => *period < 2,
                IndicatorKind::AtrBands { period, multiplier } => {
                    *period == 0 || !multiplier.is_finite() || *multiplier < 0.0
                }
                IndicatorKind::RelativeVolume { period } => *period == 0,
                IndicatorKind::VolumeOscillator { fast, slow, signal } => {
                    *fast == 0 || *fast >= *slow || *signal == 0
                }
                IndicatorKind::ElderForce { period } => *period == 0,
                IndicatorKind::EaseOfMovement { period, divisor } => {
                    *period == 0 || !divisor.is_finite() || *divisor <= 0.0
                }
                IndicatorKind::HistoricalVolatility {
                    period,
                    annualization,
                } => *period < 2 || !annualization.is_finite() || *annualization <= 0.0,
                IndicatorKind::Trix { period, signal } => *period == 0 || *signal == 0,
                IndicatorKind::Kst {
                    roc,
                    smoothing,
                    signal,
                } => roc.contains(&0) || smoothing.contains(&0) || *signal == 0,
                IndicatorKind::Tsi {
                    long,
                    short,
                    signal,
                } => *long == 0 || *short == 0 || *signal == 0,
                IndicatorKind::MassIndex {
                    ema_period,
                    sum_period,
                } => *ema_period == 0 || *sum_period == 0,
                IndicatorKind::Vortex { period } => *period == 0,
                IndicatorKind::CoppockCurve {
                    long,
                    short,
                    smoothing,
                } => *long == 0 || *short == 0 || *smoothing == 0,
                IndicatorKind::FisherTransform { period } => *period == 0,
                IndicatorKind::UltimateOscillator {
                    short,
                    medium,
                    long,
                } => *short == 0 || *medium == 0 || *long == 0,
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
                IndicatorKind::KLineChart(indicator) => !indicator.is_valid(),
                IndicatorKind::SwingPoints { .. }
                | IndicatorKind::MarketStructure { .. }
                | IndicatorKind::FairValueGaps { .. }
                | IndicatorKind::OrderBlocks { .. } => !structure_kind_is_valid(&kind),
                IndicatorKind::SessionLevels { .. }
                | IndicatorKind::PreviousPeriodLevels { .. } => false,
                IndicatorKind::OpeningRange {
                    duration_seconds, ..
                } => *duration_seconds == 0,
                IndicatorKind::Custom { .. } => true,
            }
        {
            return Vec::new();
        }
        let runtime = incremental_state(&kind);
        let output_count = session_output_count(&kind)
            .or_else(|| structure_output_count(&kind))
            .unwrap_or_else(|| runtime.output_count());
        let structure = structure_study_kind(&kind).map(StructureStudy::new);
        let source_price_format = self.series_entry(source).map(|series| {
            (
                series.price_format.kind,
                series.price_format.precision,
                series.price_format.min_move,
            )
        });
        let source_placement = self
            .series_entry(source)
            .map(|series| (series.pane_index, series.price_scale_target));
        let ids = (0..output_count)
            .map(|_| self.add_series(SeriesKind::Line))
            .collect::<Vec<_>>();
        // Indicator chrome defaults: no candle-close countdown (theirs is a line value, not a
        // bar close), the auto-generated name chip shows (platforms override the name through
        // the series `title` option — custom-script indicators will set their own), and the
        // line draws at its kind's default width without a last-price pulse — every default is
        // overridable through the ordinary series options.
        for (output_index, &id) in ids.iter().enumerate() {
            if let Some(s) = self.series.iter_mut().find(|s| s.id == id) {
                s.countdown_visible = false;
                s.title_visible = self.indicator_chrome.name_labels_visible;
                s.last_value_visible = self.indicator_chrome.value_labels_visible;
                s.price_line_visible = self.indicator_chrome.price_lines_visible;
                s.title = indicator_output_title(&kind, output_index);
                s.line_width = Some(indicator_default_line_width(&kind));
                if matches!(
                    kind,
                    IndicatorKind::SwingPoints { .. }
                        | IndicatorKind::MarketStructure { .. }
                        | IndicatorKind::FairValueGaps { .. }
                        | IndicatorKind::OrderBlocks { .. }
                        | IndicatorKind::SessionLevels { .. }
                        | IndicatorKind::PreviousPeriodLevels { .. }
                        | IndicatorKind::OpeningRange { .. }
                ) && let Some((pane_index, price_scale_target)) = source_placement
                {
                    s.pane_index = pane_index;
                    s.price_scale_target = price_scale_target;
                }
                if matches!(
                    kind,
                    IndicatorKind::MarketStructure { .. } | IndicatorKind::OrderBlocks { .. }
                ) {
                    s.line_style = 2;
                }
                if matches!(kind, IndicatorKind::SwingPoints { .. }) {
                    s.line_type = LineType::WithSteps;
                }
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
                        // Chop Zone: below 38.2 trending, above 61.8 choppy.
                        IndicatorKind::Choppiness { .. } => Some(SeriesThresholdRegion {
                            lower: 38.2,
                            upper: 61.8,
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
                if let IndicatorKind::KLineChart(indicator) = &kind {
                    apply_klinechart_output_style(s, indicator, output_index);
                    apply_klinechart_value_format(s, indicator);
                }
            }
        }
        let calendar = match &kind {
            IndicatorKind::SessionLevels { calendar }
            | IndicatorKind::PreviousPeriodLevels { calendar, .. }
            | IndicatorKind::OpeningRange { calendar, .. } => Some(*calendar),
            _ => None,
        };
        let session = session_study_kind(&kind).map(SessionStudyState::new);
        self.indicators.push(IndicatorBinding {
            source,
            source_input,
            runtime: BindingRuntime::BuiltIn(Box::new(runtime)),
            inputs: IndicatorInputs::default(),
            kind,
            outputs: ids.clone(),
            volume_source,
            amount_source,
            annotations: None,
            structure,
            session,
            calendar,
            source_generation: 0,
            volume_generation: None,
            amount_generation: None,
            data_end: 0,
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

    /// Whether `source` can carry a structure study. Its annotations name the source's own rows
    /// and paint through the source's plot rows, which an as-of overlay (or an indicator output
    /// of one) repeats or skips, so an as-of source is refused; other kinds accept any source.
    fn structure_source_is_supported(&self, source: SeriesId, kind: &IndicatorKind) -> bool {
        structure_output_count(kind).is_none()
            || !self
                .data
                .time_alignment(source)
                .is_some_and(TimeAlignment::is_as_of)
    }

    /// One past `source`'s last real row among its `rows` visible rows: the rows after it are
    /// whitespace (such as pre-installed session slots), which studies and resampled bars leave
    /// untouched on a tail change. An as-of source's plot rows are not its canonical rows, so
    /// every row counts there. Logarithmic through the source's LOD pyramid.
    pub(crate) fn source_data_end(&self, source: SeriesId, rows: usize) -> usize {
        let plot = self.data.plot(source);
        if plot.is_as_of() || plot.size() != rows {
            return rows;
        }
        plot.last_non_whitespace_row_before(rows)
            .map_or(0, |row| row + 1)
    }

    /// Report a data change of `series` from generation `previous`, touching rows from `from` on,
    /// to the derived drawing state that follows series data incrementally (regression fits), so
    /// it can extend over a tail change instead of repeating a full pass. A change reported
    /// nowhere (a complete replacement) leaves that state behind, which then rebuilds in full.
    fn note_series_change(&mut self, series: SeriesId, previous: u64, from: usize) {
        let generation = self.data.series_generation(series).unwrap_or(0);
        self.drawing_settings
            .regression_memo
            .borrow_mut()
            .note_change(series, previous, generation, from);
    }

    /// Visible length and generation of every host-owned indicator dependency (a binding's
    /// price, volume, or turnover source that is not itself an indicator output), captured before
    /// a replay clock move changes the visible prefixes.
    pub(crate) fn indicator_dependency_extents(&self) -> Vec<(SeriesId, usize, u64)> {
        let mut extents: Vec<(SeriesId, usize, u64)> = Vec::new();
        for binding in &self.indicators {
            for id in [
                Some(binding.source),
                binding.volume_source,
                binding.amount_source,
            ]
            .into_iter()
            .flatten()
            {
                if extents.iter().any(|&(seen, ..)| seen == id)
                    || self.indicator_binding_id(id).is_some()
                {
                    continue;
                }
                let visible = self
                    .data
                    .series_data(id)
                    .map_or(0, |(times, _)| times.len());
                let generation = self.data.series_generation(id).unwrap_or(0);
                extents.push((id, visible, generation));
            }
        }
        extents
    }

    /// Refresh indicators after a replay clock move, from their dependencies' `extents` before
    /// it. A dependency whose rows were only revealed (a forward move) refreshes from its previous
    /// visible length, like a tail append; one whose visible prefix shrank (a backward move)
    /// rebuilds. A dependency its owner rewrote during the move (a trade-derived or resampled
    /// series) already refreshed its bindings through that owner's update path, unless one of
    /// them is still behind, which rebuilds.
    pub(crate) fn refresh_indicators_after_cutoff(&mut self, extents: &[(SeriesId, usize, u64)]) {
        self.indicator_changes.clear();
        for &(id, before, generation) in extents {
            let current = self.data.series_generation(id).unwrap_or(0);
            let change = if current != generation {
                let behind = self.indicators.iter().any(|binding| {
                    let tracked = if binding.source == id {
                        Some(binding.source_generation)
                    } else if binding.volume_source == Some(id) {
                        binding.volume_generation
                    } else if binding.amount_source == Some(id) {
                        binding.amount_generation
                    } else {
                        return false;
                    };
                    tracked != Some(current)
                });
                if !behind {
                    continue;
                }
                IndicatorChange {
                    from: 0,
                    previous_generation: current,
                    full_replace: true,
                }
            } else {
                let now = self
                    .data
                    .series_data(id)
                    .map_or(0, |(times, _)| times.len());
                if now == before {
                    continue;
                }
                IndicatorChange {
                    from: before.min(now),
                    previous_generation: generation,
                    full_replace: now < before,
                }
            };
            self.indicator_changes.push((id, change));
        }
        if !self.indicator_changes.is_empty() {
            self.propagate_indicator_changes();
        }
        self.sync_time_points();
    }

    pub(crate) fn update_indicators_after_change(
        &mut self,
        dependency: SeriesId,
        change: IndicatorChange,
    ) {
        self.note_series_change(
            dependency,
            change.previous_generation,
            if change.full_replace { 0 } else { change.from },
        );
        self.indicator_changes.clear();
        self.indicator_changes.push((dependency, change));
        self.propagate_indicator_changes();
        self.sync_time_points();
        self.refresh_resampled_after_change(dependency, change);
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
    /// rebuild every period-keyed binding (VWAP, VWAP bands, pivots, exchange-calendar session
    /// studies) and its dependents once.
    pub(crate) fn rebuild_trading_day_indicators(&mut self) {
        self.rebuild_calendar_indicators(|binding| {
            binding.calendar == Some(StudyCalendarPolicy::Exchange)
                || matches!(
                    binding.kind,
                    IndicatorKind::Vwap
                        | IndicatorKind::VwapBands { .. }
                        | IndicatorKind::PivotPoints { .. }
                )
        });
    }

    /// The one rebuild for a calendar change (trading days or the host study calendar): rebuild
    /// every binding `affected` selects from its first row, propagate to its dependents once, and
    /// resync the time points so the time scale follows the new outputs.
    pub(crate) fn rebuild_calendar_indicators(&mut self, affected: fn(&IndicatorBinding) -> bool) {
        self.indicator_changes.clear();
        for index in 0..self.indicators.len() {
            if affected(&self.indicators[index]) {
                let changes = self.rebuild_indicator(index, 0, true);
                self.indicator_changes.extend(changes.into_iter().flatten());
            }
        }
        if !self.indicator_changes.is_empty() {
            self.propagate_indicator_changes();
            self.sync_time_points();
        }
    }

    pub(crate) fn propagate_indicator_changes(&mut self) {
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

    /// Source rows a rebuild of binding `index` covers: through the source's last real row
    /// (see [`Self::source_data_end`]), and never fewer than the previous rebuild covered unless
    /// the source shrank, so rows that just turned whitespace are rewritten too.
    pub(crate) fn indicator_data_end(
        &self,
        index: usize,
        rows: usize,
        full_replace: bool,
    ) -> usize {
        let data_end = self.source_data_end(self.indicators[index].source, rows);
        if full_replace {
            data_end
        } else {
            data_end.max(self.indicators[index].data_end).min(rows)
        }
    }

    fn rebuild_structure_indicator(
        &mut self,
        index: usize,
        from: usize,
        full_replace: bool,
        outputs: [Option<SeriesId>; aeris_charts_indicators::MAX_OUTPUTS],
    ) -> [Option<(SeriesId, IndicatorChange)>; aeris_charts_indicators::MAX_OUTPUTS] {
        let mut changes = [None; aeris_charts_indicators::MAX_OUTPUTS];
        let source = self.indicators[index].source;
        let Some((times, values)) = self.data.series_data(source) else {
            return changes;
        };
        let rows = times.len();
        // Like the built-in runtimes, the scanner stops at the source's last real row, so a tick
        // filling a pre-installed session slot stays a tail update.
        let end = self.indicator_data_end(index, rows, full_replace);
        let source_generation = self.data.series_generation(source).unwrap_or(0);
        let input = aeris_charts_indicators::IndicatorInput {
            times: &times[..end],
            open: &values[0][..end],
            high: &values[1][..end],
            low: &values[2][..end],
            close: &values[3][..end],
            volume: &[],
            amount: &[],
        };
        let binding = &mut self.indicators[index];
        let structure = binding.structure.as_mut().expect("structure runtime");
        // Only a full replacement requires a fresh runtime. Appends and tip replacements preserve
        // its bounded checkpoint state. A tick that skips whitespace slots reports a `from` past
        // the scanner's end, but the rows between are untouched whitespace, so the scan resumes
        // at the scanner's end instead of replaying the history.
        let output_start = if full_replace {
            0
        } else {
            from.min(end).min(structure.len())
        };
        if full_replace {
            *structure = StructureStudy::new(structure_study_kind(&binding.kind).unwrap());
        }
        structure.update(input, output_start);
        binding.source_generation = source_generation;
        binding.data_end = end;
        // Market-structure, fair-value-gap and order-block anchors keep every source time; the
        // stepped levels start at their first value like every scalar output.
        let anchor = matches!(
            binding.kind,
            IndicatorKind::MarketStructure { .. }
                | IndicatorKind::FairValueGaps { .. }
                | IndicatorKind::OrderBlocks { .. }
        );
        for (output_index, output) in outputs.iter().flatten().copied().enumerate() {
            let values = structure.outputs()[output_index][output_start..end]
                .iter()
                .map(|value| value.unwrap_or(f64::NAN))
                .collect::<Vec<_>>();
            let rewrite = if full_replace {
                OutputRows::Replace(values)
            } else {
                OutputRows::Update(&values)
            };
            let stored = store_indicator_output(
                &mut self.data,
                AlignedOutput {
                    source,
                    output,
                    anchor,
                },
                output_start,
                rewrite,
                end,
                rows,
            );
            changes[output_index] = stored.change.map(|change| (output, change));
        }
        changes
    }

    fn rebuild_session_indicator(
        &mut self,
        index: usize,
        from: usize,
        full_replace: bool,
        outputs: [Option<SeriesId>; aeris_charts_indicators::MAX_OUTPUTS],
    ) -> [Option<(SeriesId, IndicatorChange)>; aeris_charts_indicators::MAX_OUTPUTS] {
        let mut changes = [None; aeris_charts_indicators::MAX_OUTPUTS];
        let source = self.indicators[index].source;
        let Some((times, values)) = self.data.series_data(source) else {
            return changes;
        };
        let rows = times.len();
        let end = self.indicator_data_end(index, rows, full_replace);
        let exchange_time = &self.exchange_time;
        let trading_day_seconds = |time| exchange_time.trading_day_seconds(time);
        let session_open = |time| exchange_time.session_open_utc(time);
        let session_source = match self.indicators[index].calendar {
            Some(StudyCalendarPolicy::Host) => SessionSource::Host(&self.study_calendar_spans),
            Some(StudyCalendarPolicy::Utc) => SessionSource::Utc,
            _ => SessionSource::Exchange {
                trading_day_seconds: &trading_day_seconds,
                session_open: &session_open,
            },
        };
        let kind = session_study_kind(&self.indicators[index].kind).expect("session kind");
        let state = self.indicators[index]
            .session
            .as_mut()
            .expect("session runtime");
        if full_replace {
            *state = SessionStudyState::new(kind);
        }
        state.update(
            aeris_charts_indicators::IndicatorInput {
                times: &times[..end],
                open: &values[0][..end],
                high: &values[1][..end],
                low: &values[2][..end],
                close: &values[3][..end],
                volume: &[],
                amount: &[],
            },
            session_source,
            if full_replace { 0 } else { from },
        );
        let points = state.outputs();
        let source_generation = self.data.series_generation(source).unwrap_or(0);
        let start = if full_replace {
            0
        } else {
            from.min(points.len())
        };
        for (output_index, output) in outputs.iter().flatten().copied().enumerate() {
            let values = points[start..]
                .iter()
                .map(|p| {
                    match output_index {
                        0 => p.high,
                        1 => p.low,
                        2 if matches!(kind, SessionStudy::OpeningRange { .. }) => {
                            p.high.zip(p.low).map(|(high, low)| (high + low) * 0.5)
                        }
                        2 => p.close,
                        _ => None,
                    }
                    .unwrap_or(f64::NAN)
                })
                .collect::<Vec<_>>();
            let rewrite = if full_replace {
                OutputRows::Replace(values)
            } else {
                OutputRows::Update(&values)
            };
            let stored = store_indicator_output(
                &mut self.data,
                AlignedOutput {
                    source,
                    output,
                    anchor: false,
                },
                start,
                rewrite,
                end,
                rows,
            );
            changes[output_index] = stored.change.map(|change| (output, change));
        }
        self.indicators[index].source_generation = source_generation;
        self.indicators[index].data_end = end;
        changes
    }

    pub(crate) fn rebuild_indicator(
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
        let source_generation = self.data.series_generation(source).unwrap_or(0);
        if self.indicators[index].structure.is_some() {
            return self.rebuild_structure_indicator(index, from, full_replace, outputs);
        }
        if session_output_count(&self.indicators[index].kind).is_some() {
            return self.rebuild_session_indicator(index, from, full_replace, outputs);
        }
        if matches!(self.indicators[index].runtime, BindingRuntime::Custom(_)) {
            return self.rebuild_custom_indicator(index, from, full_replace, outputs);
        }
        if (full_replace || self.indicators[index].source_generation != source_generation)
            && let Some(annotations) = self.indicators[index].annotations.as_mut()
        {
            annotations.rebuild_from(if full_replace { 0 } else { from });
        }
        let rows;
        let end;
        {
            let Some((times, values)) = self.data.series_data(source) else {
                return changes;
            };
            // Rows past the source's last real row, before and after this change, are whitespace
            // (pre-installed session slots), so their outputs are empty and stay so: the runtime
            // stops at that data end and the outputs keep the rows past it untouched, which keeps
            // a tick filling a slot as cheap as an append.
            rows = times.len();
            end = self.indicator_data_end(index, rows, full_replace);
            let times = &times[..end];
            let values = values.map(|column| &column[..end.min(column.len())]);
            let from = if full_replace { 0 } else { from.min(end) };
            // Missing volume/amount timestamps use the documented fallback: zero for volume
            // studies, unit weight for VWAP/VWMA, and "no trade" (NaN, skipped) for the
            // amount-weighted average price.
            let fallback = if amount_source.is_some() {
                f64::NAN
            } else {
                missing_volume(&self.indicators[index].kind)
            };
            let weight = |column: Option<SeriesId>| {
                column
                    .and_then(|id| self.data.series_data(id))
                    .map(|(column_times, values)| (column_times, values[3]))
            };
            let (volume_column, amount_column) = (weight(volume_source), weight(amount_source));
            let exchange_time = &self.exchange_time;
            let binding = &mut self.indicators[index];
            let (close, price_rows) =
                price_input(&mut binding.inputs.price, source_input, values, from);
            let (volume, volume_rows) =
                binding
                    .inputs
                    .volume
                    .column(times, volume_column, from, fallback);
            let (amount, amount_rows) =
                binding
                    .inputs
                    .amount
                    .column(times, amount_column, from, fallback);
            binding
                .runtime
                .built_in_mut()
                .rebuild_from_with_trading_days(
                    aeris_charts_indicators::IndicatorInput {
                        times,
                        open: values[0],
                        high: values[1],
                        low: values[2],
                        close,
                        volume,
                        amount,
                    },
                    from,
                    &|seconds| exchange_time.trading_day_seconds(seconds),
                );
            binding.inputs.work_rows = price_rows + volume_rows + amount_rows;
        }
        self.indicators[index].data_end = end;
        self.indicators[index].source_generation = source_generation;
        self.indicators[index].volume_generation =
            volume_source.and_then(|id| self.data.series_generation(id));
        self.indicators[index].amount_generation =
            amount_source.and_then(|id| self.data.series_generation(id));

        // First output row each output rewrote, and whether it was rewritten whole, for per-row
        // colors.
        let mut changed_rows = [0usize; aeris_charts_indicators::MAX_OUTPUTS];
        let mut replaced = [false; aeris_charts_indicators::MAX_OUTPUTS];
        for (output_index, output) in outputs.iter().flatten().copied().enumerate() {
            // The runtime stops at the data end, so an output whose first row lies past it (its
            // warm-up, or the rebuild's first row, falls among trailing whitespace rows) starts
            // where it would over every row, not at the data end.
            let runtime_from = self.indicators[index]
                .runtime
                .built_in()
                .output_from(output_index);
            let source_from = if runtime_from < end {
                runtime_from
            } else {
                let requested = if full_replace { 0 } else { from };
                requested
                    .max(
                        self.indicators[index]
                            .runtime
                            .built_in()
                            .warmup_rows(output_index),
                    )
                    .clamp(end, rows)
            };

            let rewrite = if full_replace {
                OutputRows::Replace(
                    self.indicators[index]
                        .runtime
                        .built_in_mut()
                        .take_output(output_index),
                )
            } else {
                OutputRows::Update(
                    self.indicators[index]
                        .runtime
                        .built_in()
                        .output(output_index),
                )
            };
            let stored = store_indicator_output(
                &mut self.data,
                AlignedOutput {
                    source,
                    output,
                    anchor: false,
                },
                source_from,
                rewrite,
                end,
                rows,
            );
            changed_rows[output_index] = stored.from;
            replaced[output_index] = stored.replaced;
            changes[output_index] = stored.change.map(|change| (output, change));
        }

        if let Some(output_index) = histogram_output(&self.indicators[index].kind) {
            let histogram_id = outputs[output_index].unwrap();
            if replaced[output_index] {
                let colors = self
                    .data
                    .series_data(histogram_id)
                    .map(|(_, values)| momentum_histogram_colors(values[3]))
                    .unwrap_or_default();
                self.data
                    .set_point_colors(histogram_id, [Some(colors), None, None]);
            } else {
                let histogram = self.indicators[index]
                    .runtime
                    .built_in()
                    .output(output_index);
                // Rows of the histogram series itself, which starts at its first value.
                let output_start = changed_rows[output_index];
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
        if let IndicatorKind::KLineChart(indicator) = &self.indicators[index].kind {
            let rules: [Option<KLineChartColorRule>; aeris_charts_indicators::MAX_OUTPUTS] =
                std::array::from_fn(|output_index| klinechart_color_rule(indicator, output_index));
            for (output_index, rule) in rules.into_iter().enumerate() {
                if let (Some(rule), Some(output)) = (rule, outputs[output_index]) {
                    // The runtime rewrote this many rows from the first changed one; a tick over
                    // pre-installed session slots must not recolor the slots after them.
                    let changed = (!replaced[output_index]).then(|| {
                        let from = changed_rows[output_index];
                        from..from
                            + self.indicators[index]
                                .runtime
                                .built_in()
                                .output(output_index)
                                .len()
                    });
                    self.color_klinechart_output(rule, source, output, changed);
                }
            }
        }
        self.indicators[index]
            .runtime
            .built_in_mut()
            .release_transfer_capacity();
        changes
    }
}

/// Rows a rebuild writes into one indicator output, starting at its first rewritten source row.
pub(crate) enum OutputRows<'a> {
    /// The whole output from that row on (a full rebuild).
    Replace(Vec<f64>),
    /// Only the rewritten rows; the output keeps every other row.
    Update(&'a [f64]),
}

/// One indicator output series and the source it aliases rows of.
#[derive(Clone, Copy)]
pub(crate) struct AlignedOutput {
    pub(crate) source: SeriesId,
    pub(crate) output: SeriesId,
    /// Keeps every source time (a structure anchor) instead of starting at the first value.
    pub(crate) anchor: bool,
}

/// What [`store_indicator_output`] did to one output.
pub(crate) struct StoredOutput {
    /// First changed output row: an index into the output's own rows, which are the rows a
    /// dependent study reads.
    pub(crate) from: usize,
    /// The whole output was rewritten (and its per-point colors cleared).
    pub(crate) replaced: bool,
    pub(crate) change: Option<IndicatorChange>,
}

/// Store the rows one rebuild produced for `output`, aligned to `source`. Built-in, structure,
/// session and custom studies share it.
///
/// An output starts at its first value (owner decision Q-H, upstream's output shape): its leading
/// NaN rows are not stored, except for an `anchor` output (the structure anchors), which keeps
/// every source time. A rewrite that reaches the output's current start (its first row at or
/// before that start, or an empty output) may move the start, so it replaces the whole output,
/// built from the rewritten rows alone because no row precedes the start; it reports
/// `full_replace` to dependents only for a full rebuild or when the start actually moved (their
/// rows are renumbered), and an empty output that stays empty is not rewritten at all, so a
/// warming or all-whitespace source stays bounded per tick. Any other update rewrites only its
/// rows; one that stops at the data end `end` leaves the rows from `end` to `rows` (whitespace
/// session slots) untouched. Replaced outputs are padded with whitespace to `rows`.
pub(crate) fn store_indicator_output(
    data: &mut DataLayer,
    target: AlignedOutput,
    source_from: usize,
    rewrite: OutputRows<'_>,
    end: usize,
    rows: usize,
) -> StoredOutput {
    let AlignedOutput {
        source,
        output,
        anchor,
    } = target;
    let previous_generation = data.series_generation(output).unwrap_or(0);
    // The output aliases a contiguous range of source rows, so its first time locates its start.
    let existing_start = data
        .series_data(output)
        .and_then(|(times, _)| times.first().copied())
        .and_then(|first| {
            data.series_data(source)
                .and_then(|(times, _)| times.binary_search(&first).ok())
        });
    let full_rebuild = matches!(rewrite, OutputRows::Replace(_));
    let output_from = match rewrite {
        OutputRows::Update(values) if existing_start.is_some_and(|start| source_from > start) => {
            if end == rows {
                data.update_single_aligned(output, source, source_from, values)
            } else {
                data.update_single_aligned_within(output, source, source_from, values)
            }
            .expect("an update after the output's first row stays aligned to its source")
        }
        rewrite => {
            let mut values = match rewrite {
                OutputRows::Replace(values) => values,
                OutputRows::Update(values) => values.to_vec(),
            };
            let first = if anchor {
                0
            } else {
                values
                    .iter()
                    .position(|value| !value.is_nan())
                    .unwrap_or(values.len())
            };
            if first == values.len() {
                values.clear();
            } else {
                if first > 0 {
                    // A trimmed output is stored exactly sized, like an untrimmed one, so the
                    // first live append grows the column once instead of a later tick, once the
                    // trimmed slack runs out.
                    values = values[first..].to_vec();
                }
                values.resize(rows.saturating_sub(source_from + first), f64::NAN);
            }
            let start = (!values.is_empty()).then_some(source_from + first);
            if !full_rebuild && start.is_none() && existing_start.is_none() {
                // Still empty: nothing to store, nothing for dependents to redo.
                return StoredOutput {
                    from: 0,
                    replaced: false,
                    change: None,
                };
            }
            let start_moved = start != existing_start;
            data.set_single_data_aligned(output, source, start.unwrap_or(rows), values);
            return StoredOutput {
                from: 0,
                replaced: true,
                change: Some(IndicatorChange {
                    from: 0,
                    previous_generation,
                    full_replace: full_rebuild || start_moved,
                }),
            };
        }
    };
    let change = (data.series_generation(output).unwrap_or(0) != previous_generation).then_some(
        IndicatorChange {
            from: output_from,
            previous_generation,
            full_replace: false,
        },
    );
    StoredOutput {
        from: output_from,
        replaced: false,
        change,
    }
}

/// The output an indicator draws as a momentum-coloured histogram, if any.
fn histogram_output(kind: &IndicatorKind) -> Option<usize> {
    match kind {
        IndicatorKind::Macd { .. } | IndicatorKind::VolumeOscillator { .. } => Some(2),
        IndicatorKind::AwesomeOscillator => Some(0),
        _ => None,
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

/// The volume assumed for a source bar the volume series has no row for: zero for volume-flow
/// studies, the formula's own default for KLineChart indicators, and a unit weight otherwise.
fn missing_volume(kind: &IndicatorKind) -> f64 {
    match kind {
        IndicatorKind::Obv
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
        | IndicatorKind::Volume { .. } => 0.0,
        IndicatorKind::KLineChart(indicator) => indicator.missing_volume(),
        _ => 1.0,
    }
}

fn indicator_kind_name(kind: &IndicatorKind) -> &'static str {
    match kind {
        IndicatorKind::SwingPoints { .. } => "swing_points",
        IndicatorKind::MarketStructure { .. } => "market_structure",
        IndicatorKind::FairValueGaps { .. } => "fair_value_gaps",
        IndicatorKind::OrderBlocks { .. } => "order_blocks",
        IndicatorKind::SessionLevels { .. } => "session_levels",
        IndicatorKind::PreviousPeriodLevels { .. } => "previous_period_levels",
        IndicatorKind::OpeningRange { .. } => "opening_range",
        IndicatorKind::Aroon { .. } => "aroon",
        IndicatorKind::AwesomeOscillator => "awesome_oscillator",
        IndicatorKind::Dpo { .. } => "dpo",
        IndicatorKind::ChandeMomentum { .. } => "chande_momentum",
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
        IndicatorKind::BollingerMetrics { .. } => "bollinger_metrics",
        IndicatorKind::Envelopes { .. } => "envelopes",
        IndicatorKind::Alma { .. } => "alma",
        IndicatorKind::Rsi { .. } => "rsi",
        IndicatorKind::Macd { .. } => "macd",
        IndicatorKind::Stochastic { .. } => "stochastic",
        IndicatorKind::Atr { .. } => "atr",
        IndicatorKind::Vwap => "vwap",
        IndicatorKind::Obv => "obv",
        IndicatorKind::AccumulationDistribution => "accumulation_distribution",
        IndicatorKind::PriceVolumeTrend => "price_volume_trend",
        IndicatorKind::ChaikinOscillator { .. } => "chaikin_oscillator",
        IndicatorKind::Klinger { .. } => "klinger",
        IndicatorKind::Kama { .. } => "kama",
        IndicatorKind::McGinley { .. } => "mcginley",
        IndicatorKind::LinearRegression { .. } => "linear_regression",
        IndicatorKind::Choppiness { .. } => "choppiness",
        IndicatorKind::AtrBands { .. } => "atr_bands",
        IndicatorKind::RelativeVolume { .. } => "relative_volume",
        IndicatorKind::VolumeOscillator { .. } => "volume_oscillator",
        IndicatorKind::ElderForce { .. } => "elder_force",
        IndicatorKind::EaseOfMovement { .. } => "ease_of_movement",
        IndicatorKind::HistoricalVolatility { .. } => "historical_volatility",
        IndicatorKind::Trix { .. } => "trix",
        IndicatorKind::Kst { .. } => "kst",
        IndicatorKind::Tsi { .. } => "tsi",
        IndicatorKind::MassIndex { .. } => "mass_index",
        IndicatorKind::Vortex { .. } => "vortex",
        IndicatorKind::CoppockCurve { .. } => "coppock_curve",
        IndicatorKind::FisherTransform { .. } => "fisher_transform",
        IndicatorKind::UltimateOscillator { .. } => "ultimate_oscillator",
        IndicatorKind::Cmf { .. } => "cmf",
        IndicatorKind::Mfi { .. } => "mfi",
        IndicatorKind::Volume { .. } => "volume",
        IndicatorKind::VwapBands { .. } => "vwap_bands",
        IndicatorKind::Wma { .. } => "wma",
        IndicatorKind::Custom { .. } => "custom",
        IndicatorKind::Kdj { .. } => "kdj",
        IndicatorKind::KLineChart(indicator) => klinechart_kind_name(indicator),
    }
}

fn structure_output_count(kind: &IndicatorKind) -> Option<usize> {
    match kind {
        IndicatorKind::SwingPoints { .. } => Some(2),
        IndicatorKind::MarketStructure { .. }
        | IndicatorKind::FairValueGaps { .. }
        | IndicatorKind::OrderBlocks { .. } => Some(1),
        _ => None,
    }
}

fn session_output_count(kind: &IndicatorKind) -> Option<usize> {
    match kind {
        IndicatorKind::SessionLevels { .. } => Some(2),
        IndicatorKind::PreviousPeriodLevels { .. } | IndicatorKind::OpeningRange { .. } => Some(3),
        _ => None,
    }
}

fn session_study_kind(kind: &IndicatorKind) -> Option<SessionStudy> {
    match kind {
        IndicatorKind::SessionLevels { .. } => Some(SessionStudy::SessionLevels),
        IndicatorKind::PreviousPeriodLevels { period, .. } => {
            Some(SessionStudy::PreviousPeriodLevels(match period {
                PreviousPeriod::Day => aeris_charts_indicators::PreviousPeriod::Day,
                PreviousPeriod::Week => aeris_charts_indicators::PreviousPeriod::Week,
                PreviousPeriod::Month => aeris_charts_indicators::PreviousPeriod::Month,
            }))
        }
        IndicatorKind::OpeningRange {
            duration_seconds, ..
        } => Some(SessionStudy::OpeningRange {
            duration_seconds: i64::from(*duration_seconds),
        }),
        _ => None,
    }
}

pub(crate) fn structure_kind_is_valid(kind: &IndicatorKind) -> bool {
    structure_study_kind(kind).is_none_or(StructureStudyKind::is_valid)
}

fn structure_study_kind(kind: &IndicatorKind) -> Option<StructureStudyKind> {
    let break_on = |value| match value {
        StructureBreakOn::Close => BreakOn::Close,
        StructureBreakOn::Wick => BreakOn::Wick,
    };
    let mitigation = |value| match value {
        StructureMitigation::Touch => Mitigation::Touch,
        StructureMitigation::Half => Mitigation::Half,
        StructureMitigation::Full => Mitigation::Full,
    };
    let mitigation_price = |value| match value {
        StructureMitigationPrice::Wick => MitigationPrice::Wick,
        StructureMitigationPrice::Close => MitigationPrice::Close,
    };
    match kind {
        IndicatorKind::SwingPoints { left, right } => {
            Some(StructureStudyKind::swing_points(*left, *right))
        }
        IndicatorKind::MarketStructure {
            left,
            right,
            break_on: mode,
        } => Some(StructureStudyKind::market_structure(
            *left,
            *right,
            break_on(*mode),
        )),
        IndicatorKind::FairValueGaps {
            min_size,
            mitigation: rule,
            mitigation_price: price,
            max_active,
            show_mitigated,
        } => Some(StructureStudyKind::fair_value_gaps(
            *min_size,
            mitigation(*rule),
            mitigation_price(*price),
            *max_active,
            *show_mitigated,
        )),
        IndicatorKind::OrderBlocks {
            left,
            right,
            break_on: mode,
            zone,
            mitigation: rule,
            mitigation_price: price,
            max_active,
            show_mitigated,
        } => Some(StructureStudyKind::OrderBlocks {
            left: *left,
            right: *right,
            break_on: break_on(*mode),
            zone: match zone {
                OrderBlockZone::Wick => CalculationOrderBlockZone::Wick,
                OrderBlockZone::Body => CalculationOrderBlockZone::Body,
            },
            mitigation: mitigation(*rule),
            mitigation_price: mitigation_price(*price),
            max_active: *max_active,
            show_mitigated: *show_mitigated,
        }),
        _ => None,
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

pub(crate) fn incremental_state(kind: &IndicatorKind) -> aeris_charts_indicators::IncrementalState {
    match *kind {
        // Structural bindings execute their own OHLC scanner, not this scalar placeholder.
        IndicatorKind::SwingPoints { .. } => aeris_charts_indicators::IncrementalState::aroon(1),
        IndicatorKind::MarketStructure { .. }
        | IndicatorKind::FairValueGaps { .. }
        | IndicatorKind::OrderBlocks { .. } => aeris_charts_indicators::IncrementalState::sma(1),
        IndicatorKind::SessionLevels { .. }
        | IndicatorKind::PreviousPeriodLevels { .. }
        | IndicatorKind::OpeningRange { .. } => aeris_charts_indicators::IncrementalState::sma(1),
        IndicatorKind::Aroon { period } => aeris_charts_indicators::IncrementalState::aroon(period),
        IndicatorKind::AwesomeOscillator => {
            aeris_charts_indicators::IncrementalState::awesome_oscillator()
        }
        IndicatorKind::Dpo { period } => aeris_charts_indicators::IncrementalState::dpo(period),
        IndicatorKind::ChandeMomentum { period } => {
            aeris_charts_indicators::IncrementalState::chande_momentum(period)
        }
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
        IndicatorKind::BollingerMetrics { period, deviation } => {
            aeris_charts_indicators::IncrementalState::bollinger_metrics(period, deviation)
        }
        IndicatorKind::Envelopes {
            period,
            percent,
            exponential,
        } => aeris_charts_indicators::IncrementalState::envelopes(period, percent, exponential),
        IndicatorKind::Alma {
            period,
            offset,
            sigma,
        } => aeris_charts_indicators::IncrementalState::alma(period, offset, sigma),
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
        IndicatorKind::AccumulationDistribution => {
            aeris_charts_indicators::IncrementalState::accumulation_distribution()
        }
        IndicatorKind::PriceVolumeTrend => {
            aeris_charts_indicators::IncrementalState::price_volume_trend()
        }
        IndicatorKind::ChaikinOscillator { fast, slow } => {
            aeris_charts_indicators::IncrementalState::chaikin_oscillator(fast, slow)
        }
        IndicatorKind::Klinger { fast, slow, signal } => {
            aeris_charts_indicators::IncrementalState::klinger(fast, slow, signal)
        }
        IndicatorKind::Kama { period, fast, slow } => {
            aeris_charts_indicators::IncrementalState::kama(period, fast, slow)
        }
        IndicatorKind::McGinley { period } => {
            aeris_charts_indicators::IncrementalState::mcginley(period)
        }
        IndicatorKind::LinearRegression { period, deviation } => {
            aeris_charts_indicators::IncrementalState::linear_regression(period, deviation)
        }
        IndicatorKind::Choppiness { period } => {
            aeris_charts_indicators::IncrementalState::choppiness(period)
        }
        IndicatorKind::AtrBands { period, multiplier } => {
            aeris_charts_indicators::IncrementalState::atr_bands(period, multiplier)
        }
        IndicatorKind::RelativeVolume { period } => {
            aeris_charts_indicators::IncrementalState::relative_volume(period)
        }
        IndicatorKind::VolumeOscillator { fast, slow, signal } => {
            aeris_charts_indicators::IncrementalState::volume_oscillator(fast, slow, signal)
        }
        IndicatorKind::ElderForce { period } => {
            aeris_charts_indicators::IncrementalState::elder_force(period)
        }
        IndicatorKind::EaseOfMovement { period, divisor } => {
            aeris_charts_indicators::IncrementalState::ease_of_movement(period, divisor)
        }
        IndicatorKind::HistoricalVolatility {
            period,
            annualization,
        } => {
            aeris_charts_indicators::IncrementalState::historical_volatility(period, annualization)
        }
        IndicatorKind::Trix { period, signal } => {
            aeris_charts_indicators::IncrementalState::trix(period, signal)
        }
        IndicatorKind::Kst {
            roc,
            smoothing,
            signal,
        } => aeris_charts_indicators::IncrementalState::kst(roc, smoothing, signal),
        IndicatorKind::Tsi {
            long,
            short,
            signal,
        } => aeris_charts_indicators::IncrementalState::tsi(long, short, signal),
        IndicatorKind::MassIndex {
            ema_period,
            sum_period,
        } => aeris_charts_indicators::IncrementalState::mass_index(ema_period, sum_period),
        IndicatorKind::Vortex { period } => {
            aeris_charts_indicators::IncrementalState::vortex(period)
        }
        IndicatorKind::CoppockCurve {
            long,
            short,
            smoothing,
        } => aeris_charts_indicators::IncrementalState::coppock_curve(long, short, smoothing),
        IndicatorKind::FisherTransform { period } => {
            aeris_charts_indicators::IncrementalState::fisher_transform(period)
        }
        IndicatorKind::UltimateOscillator {
            short,
            medium,
            long,
        } => aeris_charts_indicators::IncrementalState::ultimate_oscillator(short, medium, long),
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
        IndicatorKind::Custom { .. } => unreachable!("custom runtime is dispatched separately"),
        IndicatorKind::KLineChart(ref indicator) => {
            aeris_charts_indicators::IncrementalState::klinechart(indicator.clone())
        }
    }
}

fn momentum_histogram_color(value: f64, previous: Option<f64>) -> u32 {
    let rising = previous.is_none_or(|previous| value >= previous);
    if value >= 0.0 {
        if rising { MACD_UP } else { MACD_UP_WEAK }
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
        IndicatorKind::SwingPoints { left, right } => format!("Swing Points {left} {right}"),
        IndicatorKind::MarketStructure { left, right, .. } => {
            format!("Market Structure {left} {right}")
        }
        IndicatorKind::FairValueGaps { .. } => "Fair Value Gaps".into(),
        IndicatorKind::OrderBlocks { .. } => "Order Blocks".into(),
        IndicatorKind::SessionLevels { .. } => "Session Levels".into(),
        IndicatorKind::PreviousPeriodLevels { period, .. } => format!("Previous {period:?} Levels"),
        IndicatorKind::OpeningRange {
            duration_seconds, ..
        } => format!("Opening Range {duration_seconds}s"),
        IndicatorKind::Aroon { period } => format!("Aroon {period}"),
        IndicatorKind::AwesomeOscillator => "Awesome Oscillator".to_string(),
        IndicatorKind::Dpo { period } => format!("DPO {period}"),
        IndicatorKind::ChandeMomentum { period } => format!("CMO {period}"),
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
        IndicatorKind::BollingerMetrics { period, deviation } => {
            format!("Bollinger Metrics {period} {}", params(*deviation))
        }
        IndicatorKind::Envelopes {
            period,
            percent,
            exponential,
        } => format!(
            "Envelopes {} {period} {}%",
            if *exponential { "EMA" } else { "SMA" },
            params(*percent)
        ),
        IndicatorKind::Alma {
            period,
            offset,
            sigma,
        } => format!("ALMA {period} {} {}", params(*offset), params(*sigma)),
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
        IndicatorKind::AccumulationDistribution => "Accumulation/Distribution".to_string(),
        IndicatorKind::PriceVolumeTrend => "PVT".to_string(),
        IndicatorKind::ChaikinOscillator { fast, slow } => {
            format!("Chaikin Oscillator {fast} {slow}")
        }
        IndicatorKind::Klinger { fast, slow, signal } => {
            format!("Klinger {fast} {slow} {signal}")
        }
        IndicatorKind::Kama { period, fast, slow } => format!("KAMA {period} {fast} {slow}"),
        IndicatorKind::McGinley { period } => format!("McGinley {period}"),
        IndicatorKind::LinearRegression { period, deviation } => {
            format!("Linear Regression {period} {}", params(*deviation))
        }
        IndicatorKind::Choppiness { period } => format!("Choppiness {period}"),
        IndicatorKind::AtrBands { period, multiplier } => {
            format!("ATR Bands {period} {}", params(*multiplier))
        }
        IndicatorKind::RelativeVolume { period } => format!("Relative Volume {period}"),
        IndicatorKind::VolumeOscillator { fast, slow, signal } => {
            format!("Volume Oscillator {fast} {slow} {signal}")
        }
        IndicatorKind::ElderForce { period } => format!("Elder Force {period}"),
        IndicatorKind::EaseOfMovement { period, divisor } => {
            format!("Ease of Movement {period} {}", params(*divisor))
        }
        IndicatorKind::HistoricalVolatility {
            period,
            annualization,
        } => format!("Historical Volatility {period} {}", params(*annualization)),
        IndicatorKind::Trix { period, signal } => format!("TRIX {period} {signal}"),
        IndicatorKind::Kst {
            roc,
            smoothing,
            signal,
        } => format!(
            "KST {} {} {} {} / {} {} {} {} / {signal}",
            roc[0], roc[1], roc[2], roc[3], smoothing[0], smoothing[1], smoothing[2], smoothing[3]
        ),
        IndicatorKind::Tsi {
            long,
            short,
            signal,
        } => format!("TSI {long} {short} {signal}"),
        IndicatorKind::MassIndex {
            ema_period,
            sum_period,
        } => format!("Mass Index {ema_period} {sum_period}"),
        IndicatorKind::Vortex { period } => format!("Vortex {period}"),
        IndicatorKind::CoppockCurve {
            long,
            short,
            smoothing,
        } => format!("Coppock Curve {long} {short} {smoothing}"),
        IndicatorKind::FisherTransform { period } => format!("Fisher Transform {period}"),
        IndicatorKind::UltimateOscillator {
            short,
            medium,
            long,
        } => format!("Ultimate Oscillator {short} {medium} {long}"),
        IndicatorKind::Cmf { period } => format!("CMF {period}"),
        IndicatorKind::Mfi { period } => format!("MFI {period}"),
        IndicatorKind::Volume { .. } => "Volume".to_string(),
        IndicatorKind::VwapBands { .. } => "VWAP Bands".to_string(),
        IndicatorKind::Wma { period } => format!("WMA {period}"),
        IndicatorKind::Custom { type_id, .. } => type_id.clone(),
        IndicatorKind::Kdj {
            period,
            k_smoothing,
            d_smoothing,
            ..
        } => format!("KDJ {period} {k_smoothing} {d_smoothing}"),
        IndicatorKind::KLineChart(indicator) => indicator.title(),
    }
}

fn indicator_output_title(kind: &IndicatorKind, output_index: usize) -> String {
    match kind {
        IndicatorKind::Aroon { period } => {
            format!("Aroon {} {period}", ["Up", "Down"][output_index])
        }
        IndicatorKind::EmaRibbon { periods } => format!("EMA {}", periods[output_index]),
        IndicatorKind::KLineChart(indicator) => indicator
            .output_titles()
            .into_iter()
            .nth(output_index)
            .unwrap_or_default(),
        _ => indicator_title(kind),
    }
}

fn indicator_output_color(kind: &IndicatorKind, output_index: usize) -> Option<&'static str> {
    match kind {
        IndicatorKind::EmaRibbon { .. } => Some(EMA_RIBBON_DEFAULT_COLORS[output_index]),
        IndicatorKind::KLineChart(indicator) => klinechart_output_color(indicator, output_index),
        _ => None,
    }
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
        IndicatorKind::SwingPoints { .. } => ["swing_high", "swing_low"][output_index],
        IndicatorKind::MarketStructure { .. }
        | IndicatorKind::FairValueGaps { .. }
        | IndicatorKind::OrderBlocks { .. } => "anchor",
        IndicatorKind::SessionLevels { .. } => ["high", "low"][output_index],
        IndicatorKind::PreviousPeriodLevels { .. } => ["high", "low", "close"][output_index],
        IndicatorKind::OpeningRange { .. } => ["high", "low", "mid"][output_index],
        IndicatorKind::Aroon { .. } => ["Aroon Up", "Aroon Down"][output_index],
        IndicatorKind::AwesomeOscillator => "AO",
        IndicatorKind::Dpo { .. } => "DPO",
        IndicatorKind::ChandeMomentum { .. } => "CMO",
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
        IndicatorKind::BollingerMetrics { .. } => ["%B", "BandWidth"][output_index],
        IndicatorKind::Envelopes { .. } => ["Upper", "Basis", "Lower"][output_index],
        IndicatorKind::Alma { .. } => "ALMA",
        IndicatorKind::Rsi { .. } => "RSI",
        IndicatorKind::Macd { .. } => ["MACD", "Signal", "Histogram"][output_index],
        IndicatorKind::Stochastic { .. } => ["%K", "%D"][output_index],
        IndicatorKind::Atr { .. } => "ATR",
        IndicatorKind::Vwap => "VWAP",
        IndicatorKind::Obv => "OBV",
        IndicatorKind::AccumulationDistribution => "A/D",
        IndicatorKind::PriceVolumeTrend => "PVT",
        IndicatorKind::ChaikinOscillator { .. } => "Chaikin Oscillator",
        IndicatorKind::Klinger { .. } => ["Klinger", "Signal"][output_index],
        IndicatorKind::Kama { .. } => "KAMA",
        IndicatorKind::McGinley { .. } => "McGinley",
        IndicatorKind::LinearRegression { .. } => ["Curve", "Upper", "Lower"][output_index],
        IndicatorKind::Choppiness { .. } => "Choppiness",
        IndicatorKind::AtrBands { .. } => ["Upper", "Basis", "Lower"][output_index],
        IndicatorKind::RelativeVolume { .. } => "Relative Volume",
        IndicatorKind::VolumeOscillator { .. } => ["PVO", "Signal", "Histogram"][output_index],
        IndicatorKind::ElderForce { .. } => "Elder Force",
        IndicatorKind::EaseOfMovement { .. } => "EOM",
        IndicatorKind::HistoricalVolatility { .. } => "HV",
        IndicatorKind::Trix { .. } => ["TRIX", "Signal"][output_index],
        IndicatorKind::Kst { .. } => ["KST", "Signal"][output_index],
        IndicatorKind::Tsi { .. } => ["TSI", "Signal"][output_index],
        IndicatorKind::MassIndex { .. } => "Mass Index",
        IndicatorKind::Vortex { .. } => ["VI+", "VI-"][output_index],
        IndicatorKind::CoppockCurve { .. } => "Coppock Curve",
        IndicatorKind::FisherTransform { .. } => ["Fisher", "Trigger"][output_index],
        IndicatorKind::UltimateOscillator { .. } => "Ultimate Oscillator",
        IndicatorKind::Cmf { .. } => "CMF",
        IndicatorKind::Mfi { .. } => "MFI",
        IndicatorKind::Volume { .. } => ["Volume", "MA"][output_index],
        IndicatorKind::VwapBands { .. } => {
            ["Basis", "Std Upper", "Std Lower", "% Upper", "% Lower"][output_index]
        }
        IndicatorKind::Wma { .. } => "WMA",
        IndicatorKind::Custom { .. } => "Custom",
        IndicatorKind::Kdj { .. } => ["K", "D", "J"][output_index],
        IndicatorKind::KLineChart(indicator) => indicator
            .output_keys()
            .get(output_index)
            .copied()
            .unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AGGREGATES: [IndicatorInputSource; 4] = [
        IndicatorInputSource::Hl2,
        IndicatorInputSource::Hlc3,
        IndicatorInputSource::Ohlc4,
        IndicatorInputSource::Hlcc4,
    ];

    /// Deterministic OHLC columns with `high >= open, close >= low`.
    fn ohlc(rows: usize) -> [Vec<f64>; 4] {
        let mut columns: [Vec<f64>; 4] = std::array::from_fn(|_| Vec::with_capacity(rows));
        for row in 0..rows {
            let base = 100.0 + (row as f64 * 0.013).sin() * 7.0;
            let close = base + (row as f64 * 0.7).cos() * 0.2;
            let open = base - 0.3;
            columns[0].push(open);
            columns[1].push(open.max(close) + 0.9);
            columns[2].push(open.min(close) - 0.8);
            columns[3].push(close);
        }
        columns
    }

    /// The per-row definition of each aggregate, written independently of `price_input`.
    fn reference(source: IndicatorInputSource, [open, high, low, close]: [f64; 4]) -> f64 {
        match source {
            IndicatorInputSource::Hl2 => (high + low) * 0.5,
            IndicatorInputSource::Hlc3 => (high + low + close) / 3.0,
            IndicatorInputSource::Ohlc4 => (open + high + low + close) * 0.25,
            IndicatorInputSource::Hlcc4 => (high + low + 2.0 * close) * 0.25,
            _ => unreachable!("not an aggregate source"),
        }
    }

    /// Derive rows `from..` and require exactly `derived` of them, bit-identical to the reference
    /// over the whole column.
    fn derive(
        cache: &mut Vec<f64>,
        source: IndicatorInputSource,
        columns: &[Vec<f64>; 4],
        from: usize,
        derived: usize,
    ) {
        let (column, rows) = price_input(
            cache,
            source,
            std::array::from_fn(|i| &columns[i][..]),
            from,
        );
        assert_eq!(rows, derived, "{source:?} rows derived from {from}");
        assert_eq!(column.len(), columns[0].len(), "{source:?} column length");
        for (row, &value) in column.iter().enumerate() {
            let expected = reference(source, std::array::from_fn(|i| columns[i][row]));
            assert_eq!(
                value.to_bits(),
                expected.to_bits(),
                "{source:?} row {row}: {value} != {expected}"
            );
        }
    }

    fn extend(columns: &mut [Vec<f64>; 4], rows: usize) {
        let more = ohlc(rows);
        for (column, more) in columns.iter_mut().zip(&more) {
            let start = column.len();
            column.extend_from_slice(&more[start..]);
        }
    }

    #[test]
    fn aggregate_column_keeps_bounded_tail_headroom() {
        const ROWS: usize = 50_000;
        for source in AGGREGATES {
            let mut columns = ohlc(ROWS);
            let mut cache = Vec::new();
            derive(&mut cache, source, &columns, 0, ROWS);
            assert!(
                (ROWS..=ROWS + price_headroom(ROWS)).contains(&cache.capacity()),
                "{source:?} install capacity {} for {ROWS} rows",
                cache.capacity()
            );

            // A live append derives one row in place: the first append after a bulk install must
            // not reallocate the column.
            let (capacity, pointer) = (cache.capacity(), cache.as_ptr());
            columns = ohlc(ROWS + 1);
            derive(&mut cache, source, &columns, ROWS, 1);
            assert_eq!(cache.capacity(), capacity, "{source:?} first append grew");
            assert_eq!(cache.as_ptr(), pointer, "{source:?} first append moved");

            // Appending past the headroom grows to a bounded size, not by doubling.
            let rows = ROWS + price_headroom(ROWS) + 2;
            columns = ohlc(rows);
            derive(&mut cache, source, &columns, ROWS + 1, rows - ROWS - 1);
            assert!(
                (rows..=rows + price_headroom(rows)).contains(&cache.capacity()),
                "{source:?} grown capacity {} for {rows} rows",
                cache.capacity()
            );

            // A rebuild from row 0 over far fewer rows releases the oversized column.
            columns = ohlc(1_000);
            derive(&mut cache, source, &columns, 0, 1_000);
            assert_eq!(
                cache.capacity(),
                1_000 + price_headroom(1_000),
                "{source:?} rebuilt capacity"
            );

            // A source with no rows allocates nothing.
            let mut empty = Vec::new();
            derive(&mut empty, source, &ohlc(0), 0, 0);
            assert_eq!(empty.capacity(), 0, "{source:?} empty capacity");
        }
    }

    #[test]
    fn canonical_inputs_borrow_the_source_and_drop_the_column() {
        let columns = ohlc(1_000);
        let values: [&[f64]; 4] = std::array::from_fn(|i| &columns[i][..]);
        for (column, source) in [
            IndicatorInputSource::Open,
            IndicatorInputSource::High,
            IndicatorInputSource::Low,
            IndicatorInputSource::Close,
        ]
        .into_iter()
        .enumerate()
        {
            let mut cache = vec![0.0; 1_000];
            let (input, derived) = price_input(&mut cache, source, values, 0);
            assert_eq!(derived, 0, "{source:?}");
            assert_eq!(input.as_ptr(), columns[column].as_ptr(), "{source:?}");
            assert_eq!(cache.capacity(), 0, "{source:?} kept a column");
        }
    }

    #[test]
    fn tail_rebuilds_keep_the_retained_prefix() {
        let mut columns = ohlc(2_000);
        let mut cache = Vec::new();
        derive(&mut cache, IndicatorInputSource::Ohlc4, &columns, 0, 2_000);
        // Revising the last bar re-derives it alone; growing by three rows derives three.
        derive(&mut cache, IndicatorInputSource::Ohlc4, &columns, 1_999, 1);
        extend(&mut columns, 2_003);
        derive(&mut cache, IndicatorInputSource::Ohlc4, &columns, 2_000, 3);
        // A retention trim shortens the source: the column follows it.
        for column in &mut columns {
            column.truncate(1_500);
        }
        derive(&mut cache, IndicatorInputSource::Ohlc4, &columns, 1_500, 0);
        assert_eq!(cache.len(), 1_500);
    }
}

#[cfg(test)]
mod annotation_binding_tests {
    use super::*;
    use aeris_charts_indicators::study_annotations::{StudyMarker, StudyMarkerKind, StudyZone};

    #[test]
    fn schema_revision_two_exposes_only_choice_options() {
        // The fork's revision: 2 with its `choices` seed field, 3 for the breadth tier, 4 for
        // upstream's `options` field replacing `choices` (upstream itself is at 2).
        assert_eq!(INDICATOR_SCHEMA_REVISION, 4);
        let choice =
            IndicatorParameterDescriptor::choice("calendar", "host", &["utc", "host"]).unwrap();
        assert_eq!(
            choice.options.as_deref(),
            Some(&["utc".into(), "host".into()][..])
        );
        assert_eq!(choice.default, serde_json::json!("host"));
        assert!(
            IndicatorParameterDescriptor::choice("calendar", "local", &["utc", "host"]).is_none()
        );
        let schema = ChartEngine::indicator_schema(&IndicatorKind::Sma { period: 14 });
        assert_eq!(schema.revision, 4);
        assert!(
            schema
                .parameters
                .iter()
                .all(|parameter| parameter.options.is_none())
        );
        // The fork's seed choices travel under the same wire key; `choices` is gone.
        let ema = serde_json::to_value(ChartEngine::indicator_schema(&IndicatorKind::Ema {
            period: 14,
            seed: aeris_charts_indicators::IndicatorSeed::Sma,
        }))
        .unwrap();
        let seed = ema["parameters"]
            .as_array()
            .unwrap()
            .iter()
            .find(|parameter| parameter["name"] == "seed")
            .unwrap();
        assert_eq!(seed["options"], serde_json::json!(["sma", "first_value"]));
        assert!(seed.get("choices").is_none());
    }

    #[test]
    fn binding_snapshot_repairs_on_source_changes_and_drops_on_removal() {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        let values = [10.0, 11.0, 12.0, 13.0];
        chart
            .set_series_data(0, &[1.0, 2.0, 3.0, 4.0], &values, &values, &values, &values)
            .unwrap();
        let binding = chart.add_sma(0, 2).unwrap();
        assert_eq!(
            chart.study_annotations(binding).unwrap_err().code(),
            ErrorCode::UnsupportedOperation
        );
        assert_eq!(
            chart.study_annotations(0).unwrap_err().code(),
            ErrorCode::InvalidHandle
        );
        let mut annotations = StudyAnnotations::default();
        for confirm_row in [1, 3] {
            annotations.push_marker(StudyMarker {
                row: confirm_row,
                confirm_row,
                price: values[confirm_row],
                kind: StudyMarkerKind::SwingHigh,
                from_row: None,
            });
        }
        annotations.push_zone(StudyZone {
            start_row: 0,
            confirm_row: 1,
            top: 12.0,
            bottom: 10.0,
            bullish: true,
            end_row: None,
            retired: false,
        });
        assert!(annotations.end_zone(0, 3));
        assert!(chart.inject_study_annotations_for_test(binding, annotations.clone()));
        let snapshot = chart.study_annotations(binding).unwrap();
        assert_eq!(snapshot, annotations);
        let json = serde_json::to_value(&snapshot).unwrap();
        assert_eq!(json["markers"].as_array().unwrap().len(), 2);
        assert_eq!(json["zones"].as_array().unwrap().len(), 1);
        assert!(json.get("active_zones").is_none());
        assert!(chart.update_series_bar(0, 4.0, [14.0; 4]));
        let repaired = chart.study_annotations(binding).unwrap();
        assert_eq!(repaired.markers().len(), 1);
        assert_eq!(repaired.markers()[0].confirm_row, 1);
        assert_eq!(repaired.zones()[0].end_row, None);
        assert_eq!(snapshot.markers().len(), 2); // The returned snapshot does not alias the binding.
        assert!(chart.remove_indicator_binding(binding));
        assert_eq!(
            chart.study_annotations(binding).unwrap_err().code(),
            ErrorCode::InvalidHandle
        );
    }

    #[test]
    fn full_source_replacement_clears_test_annotations() {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        let values = [10.0, 11.0, 12.0];
        chart
            .set_series_data(0, &[1.0, 2.0, 3.0], &values, &values, &values, &values)
            .unwrap();
        let binding = chart.add_sma(0, 2).unwrap();
        let mut annotations = StudyAnnotations::default();
        annotations.push_marker(StudyMarker {
            row: 1,
            confirm_row: 2,
            price: 11.0,
            kind: StudyMarkerKind::SwingLow,
            from_row: None,
        });
        assert!(chart.inject_study_annotations_for_test(binding, annotations));
        chart
            .set_series_data(
                0,
                &[2.0, 3.0],
                &values[..2],
                &values[..2],
                &values[..2],
                &values[..2],
            )
            .unwrap();
        assert!(
            chart
                .study_annotations(binding)
                .unwrap()
                .markers()
                .is_empty()
        );
    }
}

#[cfg(test)]
mod schema_mapping_tests {
    use super::*;
    use serde_json::{Value, json};

    #[test]
    fn named_schema_uses_canonical_multi_parameter_defaults() {
        // These are the public TS query's implicit (period=14, deviation=2) arguments.
        // Every independent parameter must match the Rust study definition, not a
        // repetition or arithmetic derivation of the query's one period argument.
        for (name, expected) in [
            (
                "stochastic_rsi",
                json!({"rsi_period":14,"stochastic_period":14}),
            ),
            ("bollinger_metrics", json!({"period":14,"deviation":2.0})),
            (
                "envelopes",
                json!({"period":14,"percent":2.0,"exponential":false}),
            ),
            ("alma", json!({"period":14,"offset":0.85,"sigma":6.0})),
            ("keltner", json!({"period":14,"multiplier":2.0})),
            ("supertrend", json!({"period":14,"multiplier":3.0})),
            (
                "ema_ribbon",
                json!({"period_1":5,"period_2":10,"period_3":20,"period_4":50,"period_5":200}),
            ),
            // The fork's seed, histogram multiplier and estimator descriptors describe the
            // textbook defaults.
            (
                "bollinger",
                json!({"period":14,"deviation":2.0,"estimator":"population"}),
            ),
            (
                "macd",
                json!({"fast":12,"slow":26,"signal":9,"seed":"sma","histogram_multiplier":1.0}),
            ),
            (
                "kdj",
                json!({"period":9,"k_smoothing":3,"d_smoothing":3,"seed":"fifty"}),
            ),
            ("stochastic", json!({"k_period":14,"d_period":3})),
            (
                "chaikin_oscillator",
                json!({"fast":3,"slow":10,"volume_source":null}),
            ),
            (
                "klinger",
                json!({"fast":34,"slow":55,"signal":13,"volume_source":null}),
            ),
            ("kama", json!({"period":10,"fast":2,"slow":30})),
            ("linear_regression", json!({"period":20,"deviation":2.0})),
            ("atr_bands", json!({"period":14,"multiplier":2.0})),
            (
                "ease_of_movement",
                json!({"period":14,"divisor":100_000_000.0,"volume_source":null}),
            ),
            (
                "historical_volatility",
                json!({"period":14,"annualization":252.0}),
            ),
            ("trix", json!({"period":14,"signal":9})),
            (
                "kst",
                json!({"roc_1":10,"roc_2":15,"roc_3":20,"roc_4":30,"smoothing_1":10,"smoothing_2":10,"smoothing_3":10,"smoothing_4":15,"signal":9}),
            ),
            ("tsi", json!({"long":25,"short":13,"signal":13})),
            ("mass_index", json!({"ema_period":9,"sum_period":25})),
            (
                "coppock_curve",
                json!({"long_period":14,"short_period":11,"smoothing":10}),
            ),
            (
                "ultimate_oscillator",
                json!({"short_period":7,"medium_period":14,"long_period":28}),
            ),
            (
                "volume_oscillator",
                json!({"fast":12,"slow":26,"signal":9,"volume_source":null}),
            ),
            (
                "vwap_bands",
                json!({"standard_deviation":1.0,"percent":10.0,"volume_source":null}),
            ),
        ] {
            let definition = IndicatorKind::schema_definition(name, 14, 2.0).unwrap();
            let schema = ChartEngine::indicator_schema(&definition);
            assert_eq!(schema.kind, name);
            let actual: serde_json::Map<String, Value> = schema
                .parameters
                .into_iter()
                .filter(|parameter| parameter.name != "source")
                .map(|parameter| (parameter.name, parameter.default))
                .collect();
            let expected = expected.as_object().unwrap();
            for (parameter, default) in expected {
                assert_eq!(actual.get(parameter), Some(default), "{name}.{parameter}");
            }
            // No unexpected parameters other than the reset enum descriptor, if present.
            assert_eq!(
                actual.len(),
                expected.len() + usize::from(name == "vwap_bands"),
                "{name}"
            );
        }
    }

    #[test]
    fn named_schema_retains_legacy_overrides_and_unknowns() {
        fn schema(name: &str, period: usize, deviation: f64) -> IndicatorSchema {
            ChartEngine::indicator_schema(
                &IndicatorKind::schema_definition(name, period, deviation).unwrap(),
            )
        }
        fn default(name: &str, period: usize, deviation: f64, parameter: &str) -> Value {
            schema(name, period, deviation)
                .parameters
                .into_iter()
                .find(|item| item.name == parameter)
                .unwrap()
                .default
        }
        assert_eq!(default("trix", 21, 2.0, "period"), json!(21));
        assert_eq!(default("trix", 21, 2.0, "signal"), json!(9));
        assert_eq!(default("stochastic", 21, 2.0, "k_period"), json!(21));
        assert_eq!(default("stochastic", 21, 2.0, "d_period"), json!(21));
        assert_eq!(default("macd", 21, 2.0, "fast"), json!(21));
        assert_eq!(default("macd", 21, 2.0, "slow"), json!(42));
        assert_eq!(default("macd", 21, 2.0, "signal"), json!(21));
        assert_eq!(default("ema_ribbon", 21, 2.0, "period_5"), json!(21));
        assert_eq!(default("bollinger", 21, 2.5, "deviation"), json!(2.5));
        assert_eq!(
            default("vwap_bands", 14, 2.5, "standard_deviation"),
            json!(2.5)
        );
        assert_eq!(schema("rma", 21, 2.0).kind, "smma");
        assert_eq!(default("kdj", 21, 2.0, "period"), json!(21));
        assert_eq!(default("ema", 21, 2.0, "seed"), json!("sma"));
        // KLineChart templates describe their own default parameters.
        assert_eq!(
            IndicatorKind::schema_definition("klinechart_macd", 14, 2.0),
            Some(IndicatorKind::KLineChart(
                crate::klinechart_indicator_for_kind_name("klinechart_macd").unwrap()
            ))
        );
        assert!(IndicatorKind::schema_definition("klinechart_unknown", 14, 2.0).is_none());
        assert!(IndicatorKind::schema_definition("unknown", 14, 2.0).is_none());
        assert!(IndicatorKind::schema_definition("pivot_points", 14, 2.0).is_none());
    }
}

#[cfg(test)]
mod structure_engine_tests {
    use super::*;
    use aeris_charts_indicators::structure_studies::StructureStudy;

    #[test]
    fn retained_source_bounds_annotations_and_attributes_their_bytes() {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        let n = 5_000;
        let times: Vec<_> = (0..n).map(|i| i as f64 + 1.).collect();
        let close: Vec<_> = (0..n).map(|i| (i * 3) as f64 + 10.).collect();
        let high: Vec<_> = close.iter().map(|v| v + 1.).collect();
        let low: Vec<_> = close.iter().map(|v| v - 1.).collect();
        chart
            .set_series_data(0, &times, &close, &high, &low, &close)
            .unwrap();
        let anchor = chart.add_fair_value_gaps(
            0,
            0.,
            StructureMitigation::Full,
            StructureMitigationPrice::Close,
            1,
            true,
        )[0];
        let before = chart.study_annotations(anchor).unwrap();
        assert!(before.zones().len() > 4_096);
        assert!(before.zones()[0].retired);
        let large = chart.memory_usage().indicator_runtime_bytes;
        assert!(
            large
                >= before.zones().len() * std::mem::size_of::<aeris_charts_indicators::StudyZone>()
        );
        assert!(chart.set_series_max_points(0, Some(128)));
        let after = chart.study_annotations(anchor).unwrap();
        assert!(after.zones().len() < 128);
        assert!(after.zones().iter().all(|zone| zone.start_row < 128));
        assert!(chart.memory_usage().indicator_runtime_bytes < large);
    }

    #[test]
    fn structure_bindings_align_anchor_and_confirmed_swing_levels() {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        let high = [12.0, 14.0, 13.0, 11.0, 13.0];
        let low = [9.0, 10.0, 9.0, 8.0, 10.0];
        let open = [10.0, 11.0, 10.0, 9.0, 11.0];
        let close = [11.0, 12.0, 10.0, 10.0, 12.0];
        chart
            .set_series_data(0, &[1.0, 2.0, 3.0, 4.0, 5.0], &open, &high, &low, &close)
            .unwrap();
        let swings = chart.add_swing_points(0, 1, 1);
        assert_eq!(swings.len(), 2);
        assert!(
            swings
                .iter()
                .all(|id| chart.series_entry(*id).unwrap().line_type == LineType::WithSteps)
        );
        let (high_times, high_values) = chart.data.series_data(swings[0]).unwrap();
        assert_eq!(high_times[0], 3);
        assert_eq!(high_values[3][0], 14.0);
        assert_eq!(
            chart.study_annotations(swings[0]).unwrap().markers()[0].row,
            1
        );
        let base_index = chart.data.base_index();
        assert_eq!(base_index, Some(4));
        let gaps = chart.add_fair_value_gaps(
            0,
            0.0,
            StructureMitigation::Touch,
            StructureMitigationPrice::Wick,
            20,
            false,
        );
        assert_eq!(gaps.len(), 1);
        let (times, values) = chart.data.series_data(gaps[0]).unwrap();
        assert_eq!(times.len(), 5);
        assert!(values[3].iter().all(|value| value.is_nan()));
        assert!(chart.study_annotations(gaps[0]).is_ok());
        // The anchor is all-whitespace by contract. The fork's base index finds each series'
        // last data row through its LOD pyramid, so the anchor needs no flag (upstream's
        // `whitespace_only`) and never moves the base index, even once the anchor (following a
        // whitespace session slot on its source) extends past the last real row.
        assert_eq!(chart.data.base_index(), base_index);
        assert!(chart.update_series_bar(0, 6.0, [f64::NAN; 4]));
        assert_eq!(chart.data.series_data(gaps[0]).unwrap().0.len(), 6);
        assert_eq!(chart.data.merged_times().len(), 6);
        assert_eq!(chart.data.base_index(), base_index);
    }

    #[test]
    fn invalid_structure_parameters_do_not_create_outputs() {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        for (left, right) in [(0, 5), (5, 0), (51, 5), (5, 51)] {
            assert!(chart.add_swing_points(0, left, right).is_empty());
        }
        assert!(
            chart
                .add_fair_value_gaps(
                    0,
                    f64::NAN,
                    StructureMitigation::Touch,
                    StructureMitigationPrice::Wick,
                    20,
                    false
                )
                .is_empty()
        );
        assert!(
            chart
                .add_fair_value_gaps(
                    0,
                    0.0,
                    StructureMitigation::Touch,
                    StructureMitigationPrice::Wick,
                    65,
                    false
                )
                .is_empty()
        );
        assert!(
            chart
                .add_indicator_kind_with_input(
                    0,
                    IndicatorInputSource::Hlc3,
                    IndicatorKind::SwingPoints { left: 2, right: 2 },
                    None,
                )
                .is_empty()
        );
        assert!(chart.indicator_bindings().is_empty());
    }

    #[test]
    fn structure_schema_reports_exact_choices_and_bounds() {
        let kind = IndicatorKind::OrderBlocks {
            left: 3,
            right: 7,
            break_on: StructureBreakOn::Wick,
            zone: OrderBlockZone::Body,
            mitigation: StructureMitigation::Half,
            mitigation_price: StructureMitigationPrice::Close,
            max_active: 12,
            show_mitigated: true,
        };
        let schema = ChartEngine::indicator_schema(&kind);
        assert_eq!(schema.revision, INDICATOR_SCHEMA_REVISION);
        assert_eq!(schema.outputs[0].name, "anchor");
        for (name, default, options) in [
            ("break_on", "wick", &["close", "wick"][..]),
            ("zone", "body", &["wick", "body"][..]),
            ("mitigation", "half", &["touch", "half", "full"][..]),
            ("mitigation_price", "close", &["wick", "close"][..]),
        ] {
            let descriptor = schema.parameters.iter().find(|p| p.name == name).unwrap();
            assert_eq!(descriptor.parameter_type, IndicatorParameterType::Choice);
            assert_eq!(descriptor.default, serde_json::json!(default));
            assert_eq!(
                descriptor.options.as_ref().unwrap(),
                &options.iter().map(|s| s.to_string()).collect::<Vec<_>>()
            );
        }
        for (name, value) in [("left", 3), ("right", 7), ("max_active", 12)] {
            let descriptor = schema.parameters.iter().find(|p| p.name == name).unwrap();
            assert_eq!(descriptor.default, serde_json::json!(value));
            assert_eq!(descriptor.min, Some(1.0));
            assert_eq!(
                descriptor.max,
                Some(if name == "max_active" { 64.0 } else { 50.0 })
            );
        }
    }

    #[test]
    fn structure_append_tip_and_history_repair_match_pure_study() {
        let kinds = [
            IndicatorKind::SwingPoints { left: 2, right: 1 },
            IndicatorKind::MarketStructure {
                left: 2,
                right: 1,
                break_on: StructureBreakOn::Wick,
            },
            IndicatorKind::FairValueGaps {
                min_size: 0.1,
                mitigation: StructureMitigation::Half,
                mitigation_price: StructureMitigationPrice::Close,
                max_active: 3,
                show_mitigated: true,
            },
            IndicatorKind::OrderBlocks {
                left: 2,
                right: 1,
                break_on: StructureBreakOn::Wick,
                zone: OrderBlockZone::Body,
                mitigation: StructureMitigation::Full,
                mitigation_price: StructureMitigationPrice::Wick,
                max_active: 3,
                show_mitigated: false,
            },
        ];
        let times = (1..=24).map(f64::from).collect::<Vec<_>>();
        let mut close = (0..24)
            .map(|row| 100.0 + ((row * 7) % 13) as f64)
            .collect::<Vec<_>>();
        let open = close.iter().map(|c| c - 0.5).collect::<Vec<_>>();
        let high = close.iter().map(|c| c + 2.0).collect::<Vec<_>>();
        let mut low = open.iter().map(|o| o - 2.0).collect::<Vec<_>>();
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        chart
            .set_series_data(
                0,
                &times[..8],
                &open[..8],
                &high[..8],
                &low[..8],
                &close[..8],
            )
            .unwrap();
        let bindings = kinds
            .iter()
            .map(|kind| chart.add_indicator_kind(0, kind.clone(), None))
            .collect::<Vec<_>>();
        let assert_equal = |chart: &ChartEngine, len: usize, close: &[f64], low: &[f64]| {
            let integer_times = (1..=len as i64).collect::<Vec<_>>();
            for (kind, ids) in kinds.iter().zip(&bindings) {
                let mut expected = StructureStudy::new(structure_study_kind(kind).unwrap());
                expected.update(
                    aeris_charts_indicators::IndicatorInput {
                        times: &integer_times,
                        open: &open[..len],
                        high: &high[..len],
                        low: &low[..len],
                        close: &close[..len],
                        volume: &[],
                        amount: &[],
                    },
                    0,
                );
                assert_eq!(
                    chart.study_annotations(ids[0]).unwrap(),
                    *expected.annotations(),
                    "{kind:?}"
                );
                for (index, &id) in ids.iter().enumerate() {
                    let (actual_times, actual) = chart.data.series_data(id).unwrap();
                    let anchor = matches!(
                        kind,
                        IndicatorKind::MarketStructure { .. }
                            | IndicatorKind::FairValueGaps { .. }
                            | IndicatorKind::OrderBlocks { .. }
                    );
                    let from = if anchor {
                        0
                    } else {
                        expected.outputs()[index]
                            .iter()
                            .position(Option::is_some)
                            .unwrap_or(len)
                    };
                    assert_eq!(
                        actual_times,
                        &integer_times[from..],
                        "{kind:?} output {index}"
                    );
                    for (row, (actual, expected)) in actual[3]
                        .iter()
                        .zip(&expected.outputs()[index][from..])
                        .enumerate()
                    {
                        assert!(
                            expected.is_some_and(|value| *actual == value)
                                || expected.is_none() && actual.is_nan(),
                            "{kind:?} output {index}, row {row}"
                        );
                    }
                }
            }
        };
        assert_equal(&chart, 8, &close, &low);
        for len in 9..=24 {
            assert!(chart.update_series_bar(
                0,
                times[len - 1],
                [open[len - 1], high[len - 1], low[len - 1], close[len - 1]]
            ));
            assert_equal(&chart, len, &close, &low);
        }
        close[23] -= 1.0;
        assert!(chart.update_series_bar(0, times[23], [open[23], high[23], low[23], close[23]]));
        assert_equal(&chart, 24, &close, &low);
        low[10] -= 1.0;
        assert!(chart.update_series_bar(0, times[10], [open[10], high[10], low[10], close[10]]));
        assert_equal(&chart, 24, &close, &low);
    }
}
