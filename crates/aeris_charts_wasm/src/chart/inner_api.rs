//! `ChartInner` model/state API: series, panes, scales, coordinates, ranges. These are the
//! working halves of the thin `#[wasm_bindgen] impl AerisChart` delegations in `chart.rs`.

use super::inner_render::measure_text_ctx;
use super::*;
use aeris_charts_engine::{
    ChartEngine, ChartError, DeviationEstimator, DrawingAnchor, DrawingMagnetMode,
    DrawingPriceSegment, ErrorCode, IndicatorConvention, IndicatorInputSource, IndicatorKind,
    IndicatorOutputStyle, IndicatorSeed, PivotKind, SyntheticBarOptions, SyntheticSourceBar,
    VwapReset,
};

fn drawing_invalid_data(message: impl Into<String>) -> ChartError {
    ChartError::new(ErrorCode::InvalidData, message)
}

/// `{ok:false,error:{code,message}}`, the shared drawing result envelope.
fn drawing_error_json(error: &ChartError) -> String {
    serde_json::json!({
        "ok": false,
        "error": { "code": error.code().name(), "message": error.message() }
    })
    .to_string()
}

fn drawing_magnet_from_u8(mode: u8) -> Option<DrawingMagnetMode> {
    Some(match mode {
        0 => DrawingMagnetMode::Off,
        1 => DrawingMagnetMode::Weak,
        2 => DrawingMagnetMode::Strong,
        _ => return None,
    })
}

fn drawing_magnet_to_u8(mode: DrawingMagnetMode) -> u8 {
    match mode {
        DrawingMagnetMode::Off => 0,
        DrawingMagnetMode::Weak => 1,
        DrawingMagnetMode::Strong => 2,
    }
}

impl ChartInner {
    pub fn configure_synthetic_bar_series(&mut self, id: u32, options_json: &str) -> String {
        let options: SyntheticBarOptions = match serde_json::from_str(options_json) {
            Ok(options) => options,
            Err(error) => return format!("invalid synthetic bar options: {error}"),
        };
        self.engine
            .configure_synthetic_bar_series(id as SeriesId, options)
            .map_or_else(|error| error.to_string(), |()| String::new())
    }

    pub fn set_synthetic_bar_source_typed(
        &mut self,
        id: u32,
        times: &Float64Array,
        open: &Float64Array,
        high: &Float64Array,
        low: &Float64Array,
        close: &Float64Array,
    ) -> String {
        let sanitized = match aeris_charts_core::model::data_validation::sanitize_ohlc_owned(
            times.to_vec(),
            open.to_vec(),
            high.to_vec(),
            low.to_vec(),
            close.to_vec(),
        ) {
            Ok(sanitized) => sanitized,
            Err(error) => return format!("invalid synthetic source: {error}"),
        };
        if sanitized.report.accepted == 0 && sanitized.report.dropped_invalid > 0 {
            return "synthetic source contains no valid rows".to_string();
        }
        let mut source = Vec::with_capacity(sanitized.times.len());
        for index in 0..sanitized.times.len() {
            let Some(timestamp_micros) = sanitized.times[index].checked_mul(1_000_000) else {
                return format!(
                    "synthetic source timestamp at index {index} is outside microsecond range"
                );
            };
            source.push(SyntheticSourceBar {
                timestamp_micros,
                open: sanitized.open[index],
                high: sanitized.high[index],
                low: sanitized.low[index],
                close: sanitized.close[index],
            });
        }
        self.engine
            .set_synthetic_bar_source(id as SeriesId, source)
            .map_or_else(|error| error.to_string(), |()| String::new())
    }

    pub fn update_synthetic_bar_source(
        &mut self,
        id: u32,
        time: f64,
        open: f64,
        high: f64,
        low: f64,
        close: f64,
    ) -> String {
        let timestamp = match aeris_charts_core::model::data_validation::validate_timestamp(time) {
            Ok(timestamp) => timestamp,
            Err(error) => return format!("invalid synthetic source timestamp: {error}"),
        };
        let Some(timestamp_micros) = timestamp.checked_mul(1_000_000) else {
            return "synthetic source timestamp is outside microsecond range".to_string();
        };
        self.engine
            .update_synthetic_bar_source(
                id as SeriesId,
                SyntheticSourceBar {
                    timestamp_micros,
                    open,
                    high,
                    low,
                    close,
                },
            )
            .map_or_else(|error| error.to_string(), |()| String::new())
    }

    pub fn set_series_area_brush_state(&mut self, id: u32, state_json: &str) -> bool {
        if state_json.is_empty() || state_json == "null" {
            return self.engine.clear_area_brush_state(id as SeriesId);
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(state_json) else {
            return false;
        };
        // Styles are engine-owned defaults (one fill strength, brand up/down hues, the series' own
        // faded stroke outside the selection); the host sends only the fields it overrides.
        let Some(defaults) = self.engine.area_brush_defaults(id as SeriesId) else {
            return false;
        };
        let Some(outside) = parse_area_brush_style(value.get("outside"), defaults.outside) else {
            return false;
        };
        let Some(ranges) = value.get("ranges").and_then(serde_json::Value::as_array) else {
            return false;
        };
        let mut parsed_ranges = Vec::with_capacity(ranges.len());
        for entry in ranges {
            let Some(range) = entry.get("range") else {
                return false;
            };
            let Some(from) = range.get("from").and_then(serde_json::Value::as_f64) else {
                return false;
            };
            let Some(to) = range.get("to").and_then(serde_json::Value::as_f64) else {
                return false;
            };
            let base = match entry.get("tone").and_then(serde_json::Value::as_str) {
                None | Some("positive") => defaults.positive,
                Some("negative") => defaults.negative,
                Some(_) => return false,
            };
            let Some(style) = parse_area_brush_style(entry.get("style"), base) else {
                return false;
            };
            parsed_ranges.push(BrushRange { from, to, style });
        }
        self.engine
            .set_area_brush_state(id as SeriesId, outside, parsed_ranges)
    }

    fn price_scale_error_json(error: aeris_charts_engine::ChartError) -> String {
        serde_json::json!({
            "ok": false,
            "error": {"code": error.code().name(), "message": error.message()}
        })
        .to_string()
    }

    pub(super) fn dispose_extensions(&mut self) {
        let pane_ids = self
            .primitives
            .iter()
            .map(|entry| entry.id)
            .collect::<Vec<_>>();
        for id in pane_ids {
            self.detach_pane_primitive(id);
        }
        let series_ids = self
            .series_primitives
            .iter()
            .map(|entry| entry.id)
            .collect::<Vec<_>>();
        for id in series_ids {
            self.detach_series_primitive(id);
        }
        self.destroy_all_custom_series();
        self.rings.clear();
        self.primitive_texts.clear();
    }

    /// Adds a series and returns its id. `kind`: 0 candles, 1 bars, 2 line, 3 area, 4 histogram.
    pub fn add_series(&mut self, kind: u8) -> u32 {
        self.engine.add_series(SeriesKind::from_u8(kind))
    }

    /// Remove a series and any indicators derived from it. Returns true if a live, non-primary
    /// series was removed. The primary series (id 0) cannot be removed.
    pub fn remove_series(&mut self, id: u32) -> bool {
        let removed = self.engine.remove_series(id as SeriesId);
        if removed {
            // Series primitives bound to the removed series (or to indicator outputs
            // tombstoned with it) auto-detach (reference `removeSeries` drops them with the series).
            self.detach_orphaned_series_primitives();
            // Custom series drop their entry with the series, firing the pane view's
            // `destroy` hook (reference `ICustomSeriesPaneView.destroy`).
            self.drop_orphaned_custom_series();
        }
        removed
    }

    /// `remove_series` that reports every tombstoned id (the series plus its derived indicator
    /// outputs) so the host can fire one removal event per series; empty = nothing removed.
    pub fn remove_series_tracked(&mut self, id: u32) -> Vec<u32> {
        let dropped = self.engine.remove_series_tracked(id as SeriesId);
        if !dropped.is_empty() {
            self.detach_orphaned_series_primitives();
            self.drop_orphaned_custom_series();
            // A removed series must not keep the engine holding views over its ring's shared
            // buffer, or a `remove_series` would leak the whole buffer for the chart's lifetime.
            let removed: Vec<SeriesId> = dropped.iter().map(|id| *id as SeriesId).collect();
            self.rings.retain(|r| !removed.contains(&r.series_id));
        }
        dropped
    }

    /// JSON [`IndicatorInfo`] for an indicator output series, or `null` for a plain/source
    /// series — the lineage a platform needs to render its own indicator chips.
    pub fn series_indicator_info_json(&self, id: u32) -> String {
        match self.engine.indicator_info(id as SeriesId) {
            Some(info) => serde_json::to_string(&info).unwrap_or_else(|_| "null".to_string()),
            None => "null".to_string(),
        }
    }

    pub fn set_indicator_output_style(&mut self, id: u32, style_json: &str) -> bool {
        serde_json::from_str::<IndicatorOutputStyle>(style_json).is_ok_and(|style| {
            self.engine
                .set_indicator_output_style(id as SeriesId, style)
        })
    }

    pub fn indicator_schema_json(&self, kind: &str, period: u32, deviation: f64) -> String {
        let period = period as usize;
        let definition = match kind {
            "sma" => IndicatorKind::Sma { period },
            "ema" => IndicatorKind::Ema {
                period,
                seed: IndicatorSeed::Sma,
            },
            "dema" => IndicatorKind::Dema {
                period,
                seed: IndicatorSeed::Sma,
            },
            "tema" => IndicatorKind::Tema {
                period,
                seed: IndicatorSeed::Sma,
            },
            "smma" | "rma" => IndicatorKind::Smma { period },
            "hma" => IndicatorKind::Hma { period },
            "vwma" => IndicatorKind::Vwma { period },
            "standard_deviation" => IndicatorKind::StandardDeviation { period },
            "cci" => IndicatorKind::Cci { period },
            "williams_r" => IndicatorKind::WilliamsR { period },
            "stochastic_rsi" => IndicatorKind::StochasticRsi {
                rsi_period: period,
                stochastic_period: period,
            },
            "momentum" => IndicatorKind::Momentum { period },
            "roc" => IndicatorKind::RateOfChange { period },
            "donchian" => IndicatorKind::Donchian { period },
            "pivot_points" => {
                let variant = match period {
                    1 => PivotKind::Standard,
                    2 => PivotKind::Fibonacci,
                    3 => PivotKind::Camarilla,
                    4 => PivotKind::Woodie,
                    5 => PivotKind::DeMark,
                    _ => return "null".into(),
                };
                IndicatorKind::PivotPoints { variant }
            }
            "zigzag" => IndicatorKind::ZigZag {
                deviation_percent: deviation,
            },
            "keltner" => IndicatorKind::Keltner {
                period,
                multiplier: deviation,
            },
            "adx_dmi" => IndicatorKind::AdxDmi { period },
            "parabolic_sar" => IndicatorKind::ParabolicSar,
            "supertrend" => IndicatorKind::SuperTrend {
                period,
                multiplier: deviation,
            },
            "ichimoku" => IndicatorKind::Ichimoku,
            "ema_ribbon" => IndicatorKind::EmaRibbon {
                periods: [period; 5],
            },
            "bollinger" => IndicatorKind::Bollinger {
                period,
                deviation,
                estimator: DeviationEstimator::Population,
            },
            "rsi" => IndicatorKind::Rsi {
                period,
                seed: IndicatorSeed::Sma,
            },
            "macd" => IndicatorKind::Macd {
                fast: period,
                slow: period.saturating_mul(2),
                signal: period,
                seed: IndicatorSeed::Sma,
                histogram_multiplier: 1.0,
            },
            "kdj" => IndicatorKind::Kdj {
                period,
                k_smoothing: 3,
                d_smoothing: 3,
                seed: aeris_charts_engine::KdjSeed::Fifty,
            },
            "stochastic" => IndicatorKind::Stochastic {
                k_period: period,
                d_period: period,
            },
            "atr" => IndicatorKind::Atr { period },
            "vwap" => IndicatorKind::Vwap,
            "obv" => IndicatorKind::Obv,
            "cmf" => IndicatorKind::Cmf { period },
            "mfi" => IndicatorKind::Mfi { period },
            "volume" => IndicatorKind::Volume { period },
            "vwap_bands" => IndicatorKind::VwapBands {
                reset: VwapReset::Session,
                standard_deviation: deviation,
                percent: 10.0,
            },
            "wma" => IndicatorKind::Wma { period },
            _ => return "null".into(),
        };
        serde_json::to_string(&ChartEngine::indicator_schema(&definition))
            .unwrap_or_else(|_| "null".into())
    }

    fn indicator_input_source(value: &str) -> Option<IndicatorInputSource> {
        match value {
            "open" => Some(IndicatorInputSource::Open),
            "high" => Some(IndicatorInputSource::High),
            "low" => Some(IndicatorInputSource::Low),
            "close" => Some(IndicatorInputSource::Close),
            "hl2" => Some(IndicatorInputSource::Hl2),
            "hlc3" => Some(IndicatorInputSource::Hlc3),
            "ohlc4" => Some(IndicatorInputSource::Ohlc4),
            "hlcc4" => Some(IndicatorInputSource::Hlcc4),
            _ => None,
        }
    }

    fn pivot_kind(value: u32) -> Option<PivotKind> {
        Some(match value {
            1 => PivotKind::Standard,
            2 => PivotKind::Fibonacci,
            3 => PivotKind::Camarilla,
            4 => PivotKind::Woodie,
            5 => PivotKind::DeMark,
            _ => return None,
        })
    }

    pub fn add_sma(&mut self, source_id: u32, period: u32) -> u32 {
        self.engine
            .add_sma(source_id as SeriesId, period as usize)
            .unwrap_or(u32::MAX)
    }

    pub fn add_sma_with_source(&mut self, source_id: u32, source: &str, period: u32) -> u32 {
        let Some(source_input) = Self::indicator_input_source(source) else {
            return u32::MAX;
        };
        self.engine
            .add_indicator_kind_with_input(
                source_id as SeriesId,
                source_input,
                IndicatorKind::Sma {
                    period: period as usize,
                },
                None,
            )
            .into_iter()
            .next()
            .unwrap_or(u32::MAX)
    }

    pub fn add_ema(&mut self, source_id: u32, period: u32) -> u32 {
        self.engine
            .add_ema(source_id as SeriesId, period as usize)
            .unwrap_or(u32::MAX)
    }

    pub fn add_dema(&mut self, source_id: u32, period: u32) -> u32 {
        self.engine
            .add_dema(source_id as SeriesId, period as usize)
            .unwrap_or(u32::MAX)
    }

    pub fn add_tema(&mut self, source_id: u32, period: u32) -> u32 {
        self.engine
            .add_tema(source_id as SeriesId, period as usize)
            .unwrap_or(u32::MAX)
    }

    pub fn add_smma(&mut self, source_id: u32, period: u32) -> u32 {
        self.engine
            .add_smma(source_id as SeriesId, period as usize)
            .unwrap_or(u32::MAX)
    }

    pub fn add_rma(&mut self, source_id: u32, period: u32) -> u32 {
        self.add_smma(source_id, period)
    }

    pub fn add_hma(&mut self, source_id: u32, period: u32) -> u32 {
        self.engine
            .add_hma(source_id as SeriesId, period as usize)
            .unwrap_or(u32::MAX)
    }

    pub fn add_vwma(&mut self, source_id: u32, volume_source: i32, period: u32) -> u32 {
        let volume = (volume_source >= 0).then_some(volume_source as SeriesId);
        self.engine
            .add_vwma(source_id as SeriesId, volume, period as usize)
            .unwrap_or(u32::MAX)
    }

    pub fn add_standard_deviation(&mut self, source_id: u32, period: u32) -> u32 {
        self.engine
            .add_standard_deviation(source_id as SeriesId, period as usize)
            .unwrap_or(u32::MAX)
    }

    pub fn add_cci(&mut self, source_id: u32, period: u32) -> u32 {
        self.engine
            .add_cci(source_id as SeriesId, period as usize)
            .unwrap_or(u32::MAX)
    }

    pub fn add_williams_r(&mut self, source_id: u32, period: u32) -> u32 {
        self.engine
            .add_williams_r(source_id as SeriesId, period as usize)
            .unwrap_or(u32::MAX)
    }

    pub fn add_stochastic_rsi(
        &mut self,
        source_id: u32,
        rsi_period: u32,
        stochastic_period: u32,
    ) -> u32 {
        self.engine
            .add_stochastic_rsi(
                source_id as SeriesId,
                rsi_period as usize,
                stochastic_period as usize,
            )
            .unwrap_or(u32::MAX)
    }

    pub fn add_momentum(&mut self, source_id: u32, period: u32) -> u32 {
        self.engine
            .add_momentum(source_id as SeriesId, period as usize)
            .unwrap_or(u32::MAX)
    }

    pub fn add_roc(&mut self, source_id: u32, period: u32) -> u32 {
        self.engine
            .add_roc(source_id as SeriesId, period as usize)
            .unwrap_or(u32::MAX)
    }

    pub fn add_donchian(&mut self, source_id: u32, period: u32) -> Vec<u32> {
        self.engine
            .add_donchian(source_id as SeriesId, period as usize)
    }

    pub fn add_pivot_points(&mut self, source_id: u32, variant: u32) -> Vec<u32> {
        let Some(kind) = Self::pivot_kind(variant) else {
            return Vec::new();
        };
        self.engine.add_pivot_points(source_id as SeriesId, kind)
    }

    pub fn add_zigzag(&mut self, source_id: u32, deviation_percent: f64) -> u32 {
        self.engine
            .add_zigzag(source_id as SeriesId, deviation_percent)
    }

    pub fn add_keltner(&mut self, source_id: u32, period: u32, multiplier: f64) -> Vec<u32> {
        self.engine
            .add_keltner(source_id as SeriesId, period as usize, multiplier)
    }

    pub fn add_adx_dmi(&mut self, source_id: u32, period: u32) -> Vec<u32> {
        self.engine
            .add_adx_dmi(source_id as SeriesId, period as usize)
    }

    pub fn add_parabolic_sar(&mut self, source_id: u32) -> u32 {
        self.engine
            .add_parabolic_sar(source_id as SeriesId)
            .unwrap_or(u32::MAX)
    }

    pub fn add_supertrend(&mut self, source_id: u32, period: u32, multiplier: f64) -> u32 {
        self.engine
            .add_supertrend(source_id as SeriesId, period as usize, multiplier)
            .unwrap_or(u32::MAX)
    }

    pub fn add_ichimoku(&mut self, source_id: u32) -> Vec<u32> {
        self.engine.add_ichimoku(source_id as SeriesId)
    }

    pub fn add_ema_ribbon(&mut self, source_id: u32, periods: [u32; 5]) -> Vec<u32> {
        self.engine
            .add_ema_ribbon(source_id as SeriesId, periods.map(|period| period as usize))
    }

    pub fn set_ema_ribbon_periods(&mut self, id: u32, periods: [u32; 5]) -> bool {
        self.engine
            .set_ema_ribbon_periods(id as SeriesId, periods.map(|period| period as usize))
    }

    pub fn add_bollinger(&mut self, source_id: u32, period: u32, deviation: f64) -> Vec<u32> {
        self.engine
            .add_bollinger(source_id as SeriesId, period as usize, deviation)
    }

    pub fn add_bollinger_with_source(
        &mut self,
        source_id: u32,
        source: &str,
        period: u32,
        deviation: f64,
    ) -> Vec<u32> {
        let Some(source_input) = Self::indicator_input_source(source) else {
            return Vec::new();
        };
        self.engine.add_indicator_kind_with_input(
            source_id as SeriesId,
            source_input,
            IndicatorKind::Bollinger {
                period: period as usize,
                deviation,
                estimator: DeviationEstimator::Population,
            },
            None,
        )
    }

    pub fn add_rsi(&mut self, source_id: u32, period: u32) -> u32 {
        self.engine
            .add_rsi(source_id as SeriesId, period as usize)
            .unwrap_or(u32::MAX)
    }

    pub fn add_rsi_with_source(&mut self, source_id: u32, source: &str, period: u32) -> u32 {
        let Some(source_input) = Self::indicator_input_source(source) else {
            return u32::MAX;
        };
        self.engine
            .add_indicator_kind_with_input(
                source_id as SeriesId,
                source_input,
                IndicatorKind::Rsi {
                    period: period as usize,
                    seed: IndicatorSeed::Sma,
                },
                None,
            )
            .into_iter()
            .next()
            .unwrap_or(u32::MAX)
    }

    pub fn set_indicator_input_source(&mut self, id: u32, source: &str) -> bool {
        Self::indicator_input_source(source).is_some_and(|source_input| {
            self.engine
                .set_indicator_input_source(id as SeriesId, source_input)
        })
    }

    pub fn add_macd(&mut self, source_id: u32, fast: u32, slow: u32, signal: u32) -> Vec<u32> {
        self.engine.add_macd(
            source_id as SeriesId,
            fast as usize,
            slow as usize,
            signal as usize,
        )
    }

    pub fn add_stochastic(&mut self, source_id: u32, k_period: u32, d_period: u32) -> Vec<u32> {
        self.engine
            .add_stochastic(source_id as SeriesId, k_period as usize, d_period as usize)
    }

    pub fn add_atr(&mut self, source_id: u32, period: u32) -> u32 {
        self.engine
            .add_atr(source_id as SeriesId, period as usize)
            .unwrap_or(u32::MAX)
    }

    /// `volume_source`: series id supplying the per-bar volume column, or -1 for unit weights.
    pub fn add_vwap(&mut self, source_id: u32, volume_source: i32) -> u32 {
        let volume = (volume_source >= 0).then_some(volume_source as SeriesId);
        self.engine
            .add_vwap(source_id as SeriesId, volume)
            .unwrap_or(u32::MAX)
    }

    pub fn add_obv(&mut self, source_id: u32, volume_source: i32) -> u32 {
        if volume_source < 0 {
            return u32::MAX;
        }
        self.engine
            .add_obv(source_id as SeriesId, volume_source as SeriesId)
            .unwrap_or(u32::MAX)
    }

    pub fn add_cmf(&mut self, source_id: u32, volume_source: i32, period: u32) -> u32 {
        if volume_source < 0 {
            return u32::MAX;
        }
        self.engine
            .add_cmf(
                source_id as SeriesId,
                volume_source as SeriesId,
                period as usize,
            )
            .unwrap_or(u32::MAX)
    }

    pub fn add_mfi(&mut self, source_id: u32, volume_source: i32, period: u32) -> u32 {
        if volume_source < 0 {
            return u32::MAX;
        }
        self.engine
            .add_mfi(
                source_id as SeriesId,
                volume_source as SeriesId,
                period as usize,
            )
            .unwrap_or(u32::MAX)
    }

    pub fn add_volume(&mut self, source_id: u32, volume_source: i32, period: u32) -> Vec<u32> {
        if volume_source < 0 {
            return Vec::new();
        }
        self.engine.add_volume(
            source_id as SeriesId,
            volume_source as SeriesId,
            period as usize,
        )
    }

    pub fn add_vwap_bands(
        &mut self,
        source_id: u32,
        volume_source: i32,
        reset: &str,
        standard_deviation: f64,
        percent: f64,
    ) -> Vec<u32> {
        let reset = match reset {
            "session" => VwapReset::Session,
            "weekly" => VwapReset::Weekly,
            "monthly" => VwapReset::Monthly,
            _ => return Vec::new(),
        };
        let volume = (volume_source >= 0).then_some(volume_source as SeriesId);
        self.engine.add_vwap_bands(
            source_id as SeriesId,
            volume,
            reset,
            standard_deviation,
            percent,
        )
    }

    pub fn add_wma(&mut self, source_id: u32, period: u32) -> u32 {
        self.engine
            .add_wma(source_id as SeriesId, period as usize)
            .unwrap_or(u32::MAX)
    }

    pub fn add_kdj(
        &mut self,
        source_id: u32,
        period: u32,
        k_smoothing: u32,
        d_smoothing: u32,
    ) -> Vec<u32> {
        self.engine.add_kdj(
            source_id as SeriesId,
            period as usize,
            k_smoothing as usize,
            d_smoothing as usize,
        )
    }

    /// Add any built-in indicator from its typed JSON definition, e.g.
    /// `{"kind":"macd","fast":12,"slow":26,"signal":9}`. `convention` (`""`, `"tradingview"` or
    /// `"china"`) first expands the convention-dependent parameters; fields present in
    /// `kind_json` override the preset. Series ids of -1 mean "none". Invalid definitions or
    /// inputs return no outputs and leave the chart unchanged.
    pub fn add_indicator(
        &mut self,
        source_id: u32,
        source: &str,
        kind_json: &str,
        convention: &str,
        volume_source: i32,
        amount_source: i32,
    ) -> Vec<u32> {
        let Some(source_input) = Self::indicator_input_source(source) else {
            return Vec::new();
        };
        let Some(kind) = Self::indicator_kind_from_json(kind_json, convention) else {
            return Vec::new();
        };
        let optional = |id: i32| (id >= 0).then_some(id as SeriesId);
        self.engine.add_indicator_kind_with_sources(
            source_id as SeriesId,
            source_input,
            kind,
            optional(volume_source),
            optional(amount_source),
        )
    }

    fn indicator_kind_from_json(kind_json: &str, convention: &str) -> Option<IndicatorKind> {
        let convention = match convention {
            "" | "tradingview" => IndicatorConvention::TradingView,
            "china" => IndicatorConvention::China,
            _ => return None,
        };
        let explicit = serde_json::from_str::<serde_json::Value>(kind_json).ok()?;
        // Serde defaults fill omitted parameters, the preset replaces the convention-dependent
        // ones, and the host's explicit fields finally win over the preset.
        let preset = serde_json::from_value::<IndicatorKind>(explicit.clone())
            .ok()?
            .with_convention(convention);
        let mut merged = serde_json::to_value(preset).ok()?;
        let fields = merged.as_object_mut()?;
        for (key, value) in explicit.as_object()? {
            fields.insert(key.clone(), value.clone());
        }
        serde_json::from_value(merged).ok()
    }

    /// Sets the main series' data (series 0). `times` are ascending UTC seconds.
    pub fn set_data(
        &mut self,
        times: &[f64],
        open: &[f64],
        high: &[f64],
        low: &[f64],
        close: &[f64],
    ) {
        self.set_series_data(0, times, open, high, low, close);
    }

    /// Sets a series' data by id.
    pub fn set_series_data(
        &mut self,
        id: u32,
        times: &[f64],
        open: &[f64],
        high: &[f64],
        low: &[f64],
        close: &[f64],
    ) {
        if self.series_is_source_owned(id) {
            web_sys::console::warn_1(
                &format!(
                    "aeris_charts: set_series_data rejected — series {id} is derived by the engine; write to its trade stream or source instead"
                )
                .into(),
            );
            return;
        }
        // Repair messy feed data (out-of-order, duplicate times, NaN/Inf, length mismatch) at the
        // boundary so the DataLayer's ascending-unique-finite contract always holds — a malformed
        // feed yields a warning and a rendered chart, never a wasm panic (roadmap Phase A3).
        let s = match sanitize_ohlc(times, open, high, low, close) {
            Ok(s) => s,
            Err(e) => {
                web_sys::console::warn_1(
                    &format!("aeris_charts: set_series_data rejected — {e}").into(),
                );
                return;
            }
        };
        if !s.report.is_clean() {
            web_sys::console::warn_1(
                &format!(
                    "aeris_charts: set_series_data sanitized data — accepted {}, dropped {} invalid, {} duplicate{}",
                    s.report.accepted,
                    s.report.dropped_invalid,
                    s.report.dropped_duplicate,
                    if s.report.reordered { ", reordered" } else { "" },
                )
                .into(),
            );
        }
        if !self
            .engine
            .install_series_data(id as SeriesId, s.times, s.open, s.high, s.low, s.close)
        {
            web_sys::console::warn_1(
                &"aeris_charts: set_series_data rejected — the engine did not install the data (unknown or removed series id)".into(),
            );
        }
    }

    pub fn set_series_data_typed(
        &mut self,
        id: u32,
        times: &Float64Array,
        open: &Float64Array,
        high: &Float64Array,
        low: &Float64Array,
        close: &Float64Array,
    ) -> Option<String> {
        if let Err(error) = self.engine.validate_series_id(id as SeriesId) {
            return Some(rejected_diagnostics_json(format_args!("{error:?}")));
        }
        // Decide ownership before copying and sanitizing a batch that is about to be refused.
        if let Some(rejected) = self.derived_write_rejection(id) {
            return Some(rejected);
        }
        let s = match aeris_charts_core::model::data_validation::sanitize_ohlc_owned(
            times.to_vec(),
            open.to_vec(),
            high.to_vec(),
            low.to_vec(),
            close.to_vec(),
        ) {
            Ok(s) => s,
            Err(e) => {
                web_sys::console::warn_1(
                    &format!("aeris_charts: set_series_data rejected — {e}").into(),
                );
                return Some(rejected_validation_diagnostics_json(e));
            }
        };
        if !s.report.is_clean() {
            web_sys::console::warn_1(&format!("aeris_charts: set_series_data sanitized data — accepted {}, dropped {} invalid, {} duplicate{}", s.report.accepted, s.report.dropped_invalid, s.report.dropped_duplicate, if s.report.reordered { ", reordered" } else { "" }).into());
        }
        let diagnostics = validation_diagnostics_json(&s.report);
        if s.report.accepted == 0 && s.report.dropped_invalid > 0 {
            return diagnostics;
        }
        if !self
            .engine
            .install_series_data(id as SeriesId, s.times, s.open, s.high, s.low, s.close)
        {
            return Some(rejected_diagnostics_json(
                "the engine did not install the series data",
            ));
        }
        diagnostics
    }

    /// Columnar streaming append (consumer Item 2): a batch of points in `set_series_data_typed`'s
    /// column layout, sanitized once and handed to one engine batch transaction.
    ///
    /// The batch is first run through the shared repair pipeline (drop non-finite → stable sort by
    /// time → collapse duplicate times last-wins), matching `set_series_data_typed` rather than
    /// inventing a second policy. All-NaN rows survive as whitespace, as they do for `update`.
    ///
    /// Tail rows remain O(1) apiece; historical rows are merged in one O(n + k) pass and cause one
    /// canonical time/indicator synchronization rather than one global reindex per row.
    pub fn update_series_bars_typed(
        &mut self,
        id: u32,
        times: &Float64Array,
        open: &Float64Array,
        high: &Float64Array,
        low: &Float64Array,
        close: &Float64Array,
    ) -> Option<String> {
        // Ignore updates to an unknown series rather than corrupting the data layer (same guard
        // as the single-point path).
        if !self.series.iter().any(|s| s.id == id as SeriesId) {
            web_sys::console::warn_1(&"aeris_charts: update_typed for unknown series id".into());
            return Some(rejected_diagnostics_json("unknown or stale series id"));
        }
        if let Some(rejected) = self.derived_write_rejection(id) {
            return Some(rejected);
        }
        let (s, diagnostics) =
            match super::series_update::sanitize_typed_batch(times, open, high, low, close) {
                Ok(batch) => batch,
                Err(rejected) => return rejected,
            };
        let rows = s.times.len();
        let applied = self.engine.update_series_bars_sanitized(
            id as SeriesId,
            s.times,
            s.open,
            s.high,
            s.low,
            s.close,
        );
        if rows > 0 && applied == 0 {
            return Some(rejected_diagnostics_json(
                "the engine did not apply the update batch",
            ));
        }
        diagnostics
    }

    /// Bind a ring source to a series (consumer Item 3), replacing any ring already bound to it.
    /// Returns `""` on success, else the reason the layout was rejected.
    pub fn set_ring_source(
        &mut self,
        series_id: u32,
        bytes: js_sys::Uint8Array,
        cursor_view: js_sys::Int32Array,
        layout_json: &str,
    ) -> String {
        if !self.series.iter().any(|s| s.id == series_id as SeriesId) {
            return "unknown or removed series id".into();
        }
        // A drain would write host rows into rows the engine owns, and be dropped on every frame.
        if self.series_is_source_owned(series_id) {
            return format!(
                "series {series_id} is derived by the engine; feed its trade stream or source instead of binding a ring"
            );
        }
        let layout: super::RingLayoutInput = match serde_json::from_str(layout_json) {
            Ok(layout) => layout,
            Err(e) => return format!("malformed ring_source_layout: {e}"),
        };
        match super::BoundRing::new(series_id as SeriesId, bytes, cursor_view, &layout) {
            Ok(ring) => {
                self.clear_ring_source(series_id);
                self.rings.push(ring);
                String::new()
            }
            Err(reason) => reason,
        }
    }

    /// Unbind a series' ring, dropping the engine's views over the shared buffer so the buffer can
    /// be collected once the host releases it too.
    pub fn clear_ring_source(&mut self, series_id: u32) {
        self.rings.retain(|r| r.series_id != series_id as SeriesId);
    }

    /// Drain every bound ring for this frame. See the wasm-facing wrapper for the `out` layout.
    pub fn drain_ring_sources(&mut self, out: &mut [f64]) -> u32 {
        let clock = self.clock.clone();
        let started = clock.as_ref().map(|clock| clock.now());
        // Destructure once so each ring's `&mut` and the engine's `&mut` are disjoint borrows.
        let Self {
            rings,
            engine,
            telemetry,
            ..
        } = self;
        let mut total = 0u32;
        let mut had_work = false;
        let mut pairs = 0usize;
        let mut lost = 0u32;
        let mut dropped = 0u32;
        for ring in rings.iter_mut() {
            let series_id = ring.series_id;
            let outcome = ring.drain(engine);
            had_work |= outcome.had_work;
            lost += outcome.lost_rows;
            dropped += outcome.dropped_rows;
            if outcome.rows == 0 {
                continue;
            }
            total += outcome.rows;
            // `[pair_count, id, rows, ...]` — stop reporting if the caller's scratch is short, but
            // keep draining so no ring stalls behind a mis-sized buffer.
            if (pairs + 1) * 2 < out.len() {
                out[1 + pairs * 2] = series_id as f64;
                out[2 + pairs * 2] = f64::from(outcome.rows);
                pairs += 1;
            }
        }
        if !out.is_empty() {
            out[0] = pairs as f64;
        }
        if lost > 0 {
            // Surfaced on `frame_stats().ring_overruns` rather than as a console warning: an
            // overrun under load would otherwise flood the console at frame rate.
            telemetry.count_ring_overruns(lost);
        }
        if dropped > 0 {
            // Same channel as overruns: a producer writing bad rows gets a counter, not a
            // frame-rate console flood.
            telemetry.count_ring_dropped_rows(dropped);
        }
        if had_work {
            if let (Some(clock), Some(start)) = (clock.as_ref(), started) {
                telemetry.add_pending_ingest_ms(clock.now() - start);
            }
        }
        total
    }

    /// Streaming update of the main series (append new time or replace last).
    pub fn update_bar(&mut self, time: f64, open: f64, high: f64, low: f64, close: f64) {
        self.update_series_bar(0, time, open, high, low, close);
    }

    /// Streaming update of the series with `series_id` (append a new time or replace the last).
    pub fn update_series_bar(
        &mut self,
        series_id: u32,
        time: f64,
        open: f64,
        high: f64,
        low: f64,
        close: f64,
    ) {
        // Ignore updates to an unknown series rather than corrupting the data layer.
        if !self.series.iter().any(|s| s.id == series_id as SeriesId) {
            web_sys::console::warn_1(&"aeris_charts: update_bar for unknown series id".into());
            return;
        }
        if let Err(error) = aeris_charts_core::model::data_validation::validate_timestamp(time) {
            web_sys::console::warn_1(
                &format!("aeris_charts: update_bar rejected — invalid timestamp: {error}").into(),
            );
            return;
        }
        if self.series_is_source_owned(series_id) {
            web_sys::console::warn_1(
                &format!(
                    "aeris_charts: update_bar rejected — series {series_id} is derived by the engine; write to its trade stream or source instead"
                )
                .into(),
            );
            return;
        }
        if !self
            .engine
            .update_series_bar(series_id as SeriesId, time, [open, high, low, close])
        {
            web_sys::console::warn_1(
                &"aeris_charts: update_bar rejected invalid or out-of-range values".into(),
            );
        }
    }

    /// Per-data-point color overrides (reference data-item colors; packed RGBA `0xRRGGBBAA`, 0 = no
    /// override at that row). Delegates to the engine; a rejection (unknown/removed id or a
    /// channel length that does not match the row count) warns and leaves no partial state.
    pub fn set_series_point_colors(
        &mut self,
        id: u32,
        body: Option<Vec<u32>>,
        wick: Option<Vec<u32>>,
        border: Option<Vec<u32>>,
    ) {
        if self.series_is_source_owned(id) {
            web_sys::console::warn_1(
                &format!(
                    "aeris_charts: set_series_point_colors rejected — series {id} is derived by the engine"
                )
                .into(),
            );
            return;
        }
        if !self
            .engine
            .set_series_point_colors(id as SeriesId, body, wick, border)
        {
            web_sys::console::warn_1(
                &"aeris_charts: set_series_point_colors rejected (unknown id or channel length != row count)".into(),
            );
        }
    }

    /// Streaming update like [`update_series_bar`] that also sets the target bar's per-point
    /// color channels (None = no custom color for that channel). Returns whether the engine
    /// applied the bar. A `false` also covers invalid values and unknown ids, so callers ask
    /// [`Self::series_is_source_owned`] to tell an engine-derived series (a footprint, a
    /// trade-bound candle or study, synthetic or resampled bars) apart.
    #[allow(clippy::too_many_arguments)] // mirrors update_series_bar plus the three reference color slots
    pub fn update_series_bar_styled(
        &mut self,
        series_id: u32,
        time: f64,
        open: f64,
        high: f64,
        low: f64,
        close: f64,
        body: Option<u32>,
        wick: Option<u32>,
        border: Option<u32>,
    ) -> bool {
        // Ignore updates to an unknown series rather than corrupting the data layer.
        if !self.series.iter().any(|s| s.id == series_id as SeriesId) {
            web_sys::console::warn_1(
                &"aeris_charts: update_series_bar_styled for unknown series id".into(),
            );
            return false;
        }
        if let Err(error) = aeris_charts_core::model::data_validation::validate_timestamp(time) {
            web_sys::console::warn_1(
                &format!(
                    "aeris_charts: update_series_bar_styled rejected — invalid timestamp: {error}"
                )
                .into(),
            );
            return false;
        }
        self.engine.update_series_bar_styled(
            series_id as SeriesId,
            time,
            [open, high, low, close],
            [body, wick, border],
        )
    }

    /// Apply a per-series `priceFormat` JSON patch (reference PriceFormat). Returns false, leaving
    /// the format unchanged, for malformed JSON, an unknown type, an unknown/removed id, or a
    /// rejected `tick_ladder`; the package reports it as `invalid_options`.
    pub fn series_apply_price_format_json(&mut self, id: u32, json: &str) -> bool {
        self.engine
            .series_apply_price_format_json(id as SeriesId, json)
    }

    /// Install a series' custom price formatter fn (reference `priceFormat.formatter`), switching it
    /// to `type:"custom"`. Same boundary contract as the chart-level price formatter: a throw
    /// or non-string result falls back to the built-in formatter.
    pub fn set_series_price_formatter(&mut self, id: u32, formatter: js_sys::Function) {
        let installed = self.engine.set_series_price_formatter(
            id as SeriesId,
            Box::new(move |price: f64| {
                formatter
                    .call1(&JsValue::NULL, &JsValue::from_f64(price))
                    .ok()
                    .and_then(|v| v.as_string())
            }) as PriceFormatterFn,
        );
        if !installed {
            web_sys::console::warn_1(
                &"aeris_charts: set_series_price_formatter ignored (unknown series id)".into(),
            );
        }
    }

    /// Install (or clear with `None`) a series' reference `autoscaleInfoProvider`. The package
    /// passes an adapter `(base) => info`: `base` is the series' own
    /// `{price_range: {min_value, max_value} | null, margins?: {above, below}}` (or `null` without
    /// data) and the returned info replaces it (`null` removes the series from autoscale). A
    /// throw keeps the series' own info for that pass.
    pub fn set_series_autoscale_info_provider(
        &mut self,
        id: u32,
        provider: Option<js_sys::Function>,
    ) {
        let provider = provider.map(|provider| {
            Box::new(move |base: Option<aeris_charts_engine::AutoscaleInfo>| {
                match provider.call1(&JsValue::NULL, &autoscale_info_to_js(base)) {
                    Ok(result) => autoscale_info_from_js(&result),
                    Err(error) => {
                        web_sys::console::warn_1(
                            &format!(
                                "aeris_charts: series `autoscale_info_provider` threw — {error:?}"
                            )
                            .into(),
                        );
                        base
                    }
                }
            }) as aeris_charts_engine::AutoscaleInfoProviderFn
        });
        if !self
            .engine
            .set_series_autoscale_info_provider(id as SeriesId, provider)
        {
            web_sys::console::warn_1(
                &"aeris_charts: set_series_autoscale_info_provider ignored (unknown series id)"
                    .into(),
            );
        }
    }

    /// Sets a series' line/area color (overrides the kind default). The numeric r/g/b form
    /// stores the computed CSS string so `series_options_json` round-trips it exactly.
    pub fn set_series_color(&mut self, id: u32, r: u8, g: u8, b: u8) {
        if let Some(s) = self.series.iter_mut().find(|s| s.id == id as SeriesId) {
            s.line_color = Some(Color::rgb(r, g, b).to_css());
        }
    }

    /// Sets a series' line/area/histogram stroke color from a CSS string, preserving alpha
    /// (the r/g/b `set_series_color` form is opaque-only). Stored verbatim (reference `options()`
    /// returns the applied string); parsed at render time.
    pub fn set_series_color_css(&mut self, id: u32, css: &str) {
        if let Some(s) = self.series.iter_mut().find(|s| s.id == id as SeriesId) {
            s.line_color = Some(css.to_string());
        }
    }

    pub fn set_series_visible(&mut self, id: u32, visible: bool) {
        self.engine.set_series_visible(id as SeriesId, visible);
    }

    /// Retention ceiling for a series: at most `max_points` rows, oldest evicted first. A
    /// non-positive or non-finite value clears the cap (unbounded, the default).
    pub fn set_series_max_points(&mut self, id: u32, max_points: Option<f64>) {
        let cap = max_points
            .filter(|n| n.is_finite() && *n >= 1.0)
            .map(|n| n as usize);
        if !self.engine.set_series_max_points(id as SeriesId, cap) {
            // A bound candle or synthetic series keeps the retention of the stream or source
            // that feeds it; only unknown and removed ids are otherwise refused.
            let reason = if self.series_is_source_owned(id) {
                "series is engine-owned; retention is set on the trade stream or source"
            } else {
                "unknown or removed series id"
            };
            web_sys::console::warn_1(
                &format!("aeris_charts: set_series_max_points ignored ({reason})").into(),
            );
        }
    }

    /// This series' retention ceiling, or `None` when unbounded.
    pub fn series_max_points(&self, id: u32) -> Option<f64> {
        self.engine
            .series_max_points(id as SeriesId)
            .map(|n| n as f64)
    }

    /// Set candlestick/bar body colors per direction, stored verbatim (reference `options()`
    /// returns the applied string; the strings are parsed at render time). Same keep/clear/pin
    /// contract as the wick and border setters: `undefined` = keep current, `""` = clear the
    /// override back to the reference default palette, a CSS color = pin it verbatim
    /// (`"transparent"` gives a hollow body).
    pub fn set_series_updown_colors(&mut self, id: u32, up: Option<String>, down: Option<String>) {
        if let Some(s) = self.series.iter_mut().find(|s| s.id == id as SeriesId) {
            crate::color_policy::update_color_slot(&mut s.up_color, up);
            crate::color_policy::update_color_slot(&mut s.down_color, down);
        }
    }

    /// Set candlestick wick colors per direction. `undefined` = keep current, `""` = clear the
    /// override (follow the direction's body color), a CSS color = pin it verbatim.
    pub fn set_series_wick_colors(&mut self, id: u32, up: Option<String>, down: Option<String>) {
        if let Some(s) = self.series.iter_mut().find(|s| s.id == id as SeriesId) {
            crate::color_policy::update_color_slot(&mut s.wick_up_color, up);
            crate::color_policy::update_color_slot(&mut s.wick_down_color, down);
        }
    }

    /// Set candlestick border colors per direction; same keep/clear/pin contract as the wicks.
    pub fn set_series_border_colors(&mut self, id: u32, up: Option<String>, down: Option<String>) {
        if let Some(s) = self.series.iter_mut().find(|s| s.id == id as SeriesId) {
            crate::color_policy::update_color_slot(&mut s.border_up_color, up);
            crate::color_policy::update_color_slot(&mut s.border_down_color, down);
        }
    }

    /// Toggle candlestick wick visibility (default visible; bars ignore this).
    pub fn set_series_wick_visible(&mut self, id: u32, visible: bool) {
        if let Some(s) = self.series.iter_mut().find(|s| s.id == id as SeriesId) {
            s.wick_visible = Some(visible);
        }
    }

    /// Toggle candlestick body-border visibility (default visible; bars ignore this).
    pub fn set_series_border_visible(&mut self, id: u32, visible: bool) {
        if let Some(s) = self.series.iter_mut().find(|s| s.id == id as SeriesId) {
            s.border_visible = Some(visible);
        }
    }

    /// Set a line/area series' stroke width (css px; non-positive ignored).
    pub fn set_series_line_width(&mut self, id: u32, width: f64) {
        if width > 0.0 {
            if let Some(s) = self.series.iter_mut().find(|s| s.id == id as SeriesId) {
                s.line_width = Some(width);
            }
        }
    }

    /// Set an area series' fill gradient colors (top at the line, bottom at the base), stored
    /// verbatim like the other color slots (`""` clears back to the engine default; parsed at
    /// render time).
    pub fn set_series_area_colors(&mut self, id: u32, top: &str, bottom: &str) {
        if let Some(s) = self.series.iter_mut().find(|s| s.id == id as SeriesId) {
            crate::color_policy::update_color_slot(&mut s.area_top_color, Some(top.to_string()));
            crate::color_policy::update_color_slot(
                &mut s.area_bottom_color,
                Some(bottom.to_string()),
            );
        }
    }

    /// Color a histogram by the main price series' up/down direction per bar (reference-informed volume).
    pub fn set_series_histogram_updown(&mut self, id: u32, enabled: bool) {
        if let Some(s) = self.series.iter_mut().find(|s| s.id == id as SeriesId) {
            s.histogram_updown = enabled;
        }
    }

    /// Set a line/area series' join type: 0 = simple, 1 = stepped, 2 = curved (roadmap Phase B3).
    pub fn set_series_line_type(&mut self, id: u32, line_type: u8) {
        let lt = match line_type {
            1 => LineType::WithSteps,
            2 => LineType::Curved,
            _ => LineType::Simple,
        };
        if let Some(s) = self.series.iter_mut().find(|s| s.id == id as SeriesId) {
            s.line_type = lt;
        }
    }

    /// Toggle per-point disc markers on a line/area series (roadmap Phase B3).
    pub fn set_series_point_markers(&mut self, id: u32, visible: bool) {
        if let Some(s) = self.series.iter_mut().find(|s| s.id == id as SeriesId) {
            s.point_markers = visible;
        }
    }

    /// Toggle the close tick on OHLC bars. Set this and `open_visible` false for high-low bars.
    pub fn set_series_close_visible(&mut self, id: u32, visible: bool) {
        if let Some(s) = self.series.iter_mut().find(|s| s.id == id as SeriesId) {
            s.close_visible = visible;
        }
    }

    /// Set a Baseline series' baseline price. `NaN` resets to auto (visible-range midpoint).
    pub fn set_series_baseline(&mut self, id: u32, price: f64) {
        if let Some(s) = self.series.iter_mut().find(|s| s.id == id as SeriesId) {
            s.baseline = if price.is_finite() { Some(price) } else { None };
        }
    }

    /// Add a horizontal price line to a series; returns its id (roadmap Phase B4).
    #[allow(clippy::too_many_arguments)]
    pub fn create_price_line(
        &mut self,
        series_id: u32,
        price: f64,
        r: u8,
        g: u8,
        b: u8,
        width: u32,
        style: u8,
        title: &str,
    ) -> u32 {
        self.engine.create_price_line(
            series_id as SeriesId,
            price,
            Color::rgb(r, g, b),
            width as i32,
            line_style_from_u8(style),
            title,
        )
    }

    /// Remove a price line by id (from whichever series holds it).
    pub fn remove_price_line(&mut self, id: u32) {
        self.engine.remove_price_line(id);
    }

    /// Merge a JSON options patch into the price line with `id` (reference `IPriceLine.applyOptions`).
    pub fn price_line_apply_options(&mut self, id: u32, json: &str) {
        if !self.engine.price_line_apply_options(id, json) {
            web_sys::console::warn_1(
                &"aeris_charts: price_line_apply_options ignored (unknown id or malformed JSON)"
                    .into(),
            );
        }
    }

    /// The price line's full options as snake_case JSON ("" for an unknown id).
    pub fn price_line_options_json(&self, id: u32) -> String {
        self.engine.price_line_options_json(id).unwrap_or_default()
    }

    /// Transactionally replace a series' official marker state. Invalid input keeps the previous
    /// marker set instead of silently clearing it.
    pub fn set_series_markers(&mut self, series_id: u32, json: &str) -> bool {
        let Ok(inputs) = serde_json::from_str::<Vec<MarkerInput>>(json) else {
            return false;
        };
        let mut markers = Vec::with_capacity(inputs.len());
        for marker in inputs {
            let Ok(time) =
                aeris_charts_core::model::data_validation::validate_timestamp(marker.time)
            else {
                return false;
            };
            if !marker.size.is_finite() || marker.price.is_some_and(|price| !price.is_finite()) {
                return false;
            }
            let position = match marker.position.as_str() {
                "" | "above" | "aboveBar" => marker_pos::ABOVE,
                "below" | "belowBar" => marker_pos::BELOW,
                "inBar" | "in" => marker_pos::IN_BAR,
                "atPriceTop" => marker_pos::AT_PRICE_TOP,
                "atPriceBottom" => marker_pos::AT_PRICE_BOTTOM,
                "atPriceMiddle" => marker_pos::AT_PRICE_MIDDLE,
                _ => return false,
            };
            if matches!(
                position,
                marker_pos::AT_PRICE_TOP
                    | marker_pos::AT_PRICE_BOTTOM
                    | marker_pos::AT_PRICE_MIDDLE
            ) && marker.price.is_none()
            {
                return false;
            }
            let shape = match marker.shape.as_str() {
                "" | "circle" => marker_shape::CIRCLE,
                "square" => marker_shape::SQUARE,
                "arrowUp" | "arrow_up" => marker_shape::ARROW_UP,
                "arrowDown" | "arrow_down" => marker_shape::ARROW_DOWN,
                _ => return false,
            };
            let color = if marker.color.is_empty() {
                Color::rgb(0x21, 0x96, 0xf3)
            } else {
                let Some(color) = Color::parse_css(&marker.color) else {
                    return false;
                };
                color
            };
            markers.push(Marker {
                time,
                position,
                shape,
                color,
                text: marker.text,
                id: marker.id,
                size: marker.size.max(0.0),
                price: marker.price,
            });
        }
        self.engine
            .set_series_markers(series_id as SeriesId, markers);
        true
    }

    pub fn set_series_markers_auto_scale(&mut self, series_id: u32, enabled: bool) {
        self.engine
            .set_series_markers_auto_scale(series_id as SeriesId, enabled);
    }

    /// Toggle the pulsing last-price ring on a series (roadmap Phase B3).
    pub fn set_series_last_price_animation(&mut self, id: u32, enabled: bool) {
        self.engine
            .set_series_last_price_animation(id as SeriesId, enabled);
    }

    /// Whether the frame draws the last-price pulse (so the host can start/stop its rAF loop).
    pub fn wants_animation(&self) -> bool {
        self.engine.last_price_pulse_active()
    }

    /// Set the host animation clock (ms). The shell's rAF loop calls this then `render()`.
    pub fn set_animation_time(&mut self, t_ms: f64) {
        self.animation_time = t_ms;
    }

    /// Move a series onto its pane's bottom-band overlay scale (volume-style) and set that band's
    /// margins as fractions of the pane slot: `top` leaves that fraction above the band, `bottom`
    /// below it (e.g. top=0.8, bottom=0.0 ⇒ bottom 20%). Excludes the series from the pane's main
    /// autoscale (roadmap Phase B2).
    pub fn set_series_overlay(&mut self, id: u32, top: f64, bottom: f64) {
        let mut pane_index = 0;
        if let Some(s) = self.series.iter_mut().find(|s| s.id == id as SeriesId) {
            s.price_scale_target = PriceScaleTarget::Overlay;
            pane_index = s.pane_index;
        }
        if let Some(p) = self.panes.get_mut(pane_index) {
            p.overlay_top = top.clamp(0.0, 1.0);
            p.overlay_bottom = bottom.clamp(0.0, 1.0);
            p.overlay_scale
                .set_scale_margins(p.overlay_top, p.overlay_bottom);
            p.refresh_internal_margins();
        }
    }

    /// Move a series into pane `pane_index`, creating panes (with the given stretch factor for a
    /// newly-created last pane) as needed. Pane 0 is the top/price pane (roadmap Phase B1).
    pub fn set_series_pane(&mut self, id: u32, pane_index: usize, stretch_factor: f64) {
        self.engine
            .set_series_pane(id as SeriesId, pane_index, stretch_factor);
    }

    pub fn try_set_series_pane(&mut self, id: u32, pane_index: usize, stretch_factor: f64) -> bool {
        let changed = self
            .engine
            .try_set_series_pane(id as SeriesId, pane_index, stretch_factor);
        if changed {
            self.recompute_layout(true);
        }
        changed
    }

    pub fn try_set_series_pane_and_scale(
        &mut self,
        id: u32,
        pane_index: usize,
        stretch_factor: f64,
        price_scale_id: &str,
    ) -> bool {
        let changed = self.engine.try_set_series_pane_and_scale(
            id as SeriesId,
            pane_index,
            stretch_factor,
            price_scale_id,
        );
        if changed {
            self.recompute_layout(true);
        }
        changed
    }

    /// Number of stacked panes.
    pub fn pane_count(&self) -> usize {
        self.panes.len()
    }

    pub fn pane_stable_id(&self, index: u32) -> Option<u32> {
        self.engine.pane_stable_id(index as usize).map(PaneId::get)
    }

    pub fn pane_index_for_id(&self, stable_id: u32) -> Option<u32> {
        let stable_id = PaneId::try_from(stable_id).ok()?;
        self.engine
            .pane_index_for_id(stable_id)
            .map(|index| index as u32)
    }

    /// CSS Y of each pane boundary (top edge of panes 1..n), for separator hit-testing by the host.
    /// Reflects the last layout pass.
    pub fn pane_separator_ys(&self) -> Vec<f64> {
        self.panes.iter().skip(1).map(|p| p.top).collect()
    }

    /// Drag the separator below pane `i` by `delta_css` (positive grows pane `i`, shrinks `i+1`),
    /// keeping both at least a minimum height. Shared engine policy freezes current heights as
    /// stretch factors so browser and native hosts resize panes identically.
    pub fn drag_pane_separator(&mut self, i: usize, delta_css: f64) {
        self.engine.drag_pane_separator(i, delta_css);
    }

    /// CSS height of pane `i` from the last layout pass.
    pub fn pane_height(&self, i: usize) -> f64 {
        self.panes.get(i).map(|p| p.height).unwrap_or(0.0)
    }

    /// JSON `{left, top, width, height}` of pane `i`'s content area in CSS px relative to the
    /// chart container's top-left (`{}` for a stale index). This is the anchor a platform
    /// absolutely-positions per-pane chrome against (e.g. an industry-standard indicator chip
    /// at the pane's top-left). Reflects the last layout pass.
    pub fn pane_geometry_json(&self, i: usize) -> String {
        match self.panes.get(i) {
            Some(p) => format!(
                r#"{{"left":{},"top":{},"width":{},"height":{}}}"#,
                self.pane_left, p.top, self.pane_w, p.height
            ),
            None => "{}".to_string(),
        }
    }

    /// Relative stretch factor of pane `i`.
    pub fn pane_stretch(&self, i: usize) -> f64 {
        self.panes.get(i).map(|p| p.stretch_factor).unwrap_or(1.0)
    }

    /// Set pane `i`'s stretch factor (its share of the content height relative to the others).
    pub fn set_pane_stretch(&mut self, i: usize, factor: f64) {
        if let Some(p) = self.panes.get_mut(i) {
            p.stretch_factor = factor.max(0.01);
            if self.css_width > 0.0 {
                self.recompute_layout(false);
            }
        }
    }

    /// Resize pane `i` to `height_css`, absorbing the delta from its neighbour below (or above for
    /// the last pane) — the same freeze-and-redistribute behavior as dragging its separator.
    pub fn set_pane_height(&mut self, i: usize, height_css: f64) {
        if i >= self.panes.len() {
            return;
        }
        let current = self.panes[i].height;
        let delta = height_css - current;
        if i + 1 < self.panes.len() {
            self.drag_pane_separator(i, delta);
        } else if i > 0 {
            // last pane: move the separator above it the other way to grow/shrink it
            self.drag_pane_separator(i - 1, -delta);
        }
        if self.css_width > 0.0 {
            self.recompute_layout(false);
        }
    }

    /// reference v5 `chart.addPane(preserveEmptyPane)`: append a pane and return its index.
    pub fn add_pane(&mut self, preserve_empty: bool) -> Option<u32> {
        self.engine
            .add_pane(preserve_empty)
            .and_then(|index| u32::try_from(index).ok())
    }

    pub fn export_state_result_json(&self) -> String {
        match self.engine.export_state_json() {
            Ok(document) => serde_json::json!({ "ok": true, "document": document }).to_string(),
            Err(error) => serde_json::json!({
                "ok": false,
                "error": { "code": error.code().name(), "message": error.message() }
            })
            .to_string(),
        }
    }

    pub fn import_state_result_json(&mut self, document: &str) -> String {
        match self.engine.import_state_json(document) {
            Ok(result) => serde_json::json!({
                "ok": true,
                "result": {
                    "schema_version": result.schema_version,
                    "panes": result.panes,
                    "drawings": result.drawings,
                    "points": result.points,
                }
            })
            .to_string(),
            Err(error) => serde_json::json!({
                "ok": false,
                "error": { "code": error.code().name(), "message": error.message() }
            })
            .to_string(),
        }
    }

    /// reference `chart.removePane`: rejects stale indices and a non-empty final pane. An empty
    /// preserved final pane is retired and replaced by a fresh default pane so its handle stales
    /// while the engine retains one layout slot. The pane's series are NOT removed — they become
    /// pane-less (reference `paneForSource` → null) until re-assigned; panes below shift up.
    pub fn remove_pane(&mut self, index: u32) -> bool {
        self.engine.remove_pane(index as usize)
    }

    /// reference `chart.swapPanes`: the two panes trade places — series assignments, stretch
    /// factors, scales, and preserve flags ride along with them.
    pub fn swap_panes(&mut self, first: u32, second: u32) -> bool {
        self.engine.swap_panes(first as usize, second as usize)
    }

    /// reference `IPaneApi.moveTo`: relocate the pane (with its series) to a new index; the panes
    /// in between shift one slot. False for a stale index.
    pub fn pane_move_to(&mut self, index: u32, target: u32) -> bool {
        self.engine.move_pane(index as usize, target as usize)
    }

    /// reference `IPaneApi.preserveEmptyPane` (false for a stale index).
    pub fn pane_preserve_empty(&self, index: u32) -> bool {
        self.engine.pane_preserve_empty(index as usize)
    }

    /// reference `IPaneApi.setPreserveEmptyPane`: an empty pane collapses on the next series
    /// removal/move-out unless this flag holds it open (chart-model.ts
    /// `_cleanupIfPaneIsEmpty`).
    pub fn pane_set_preserve_empty(&mut self, index: u32, flag: bool) {
        self.engine.pane_set_preserve_empty(index as usize, flag);
    }

    /// reference `IPaneApi.getSeries`: the pane's live series ids in render order (bottom first).
    pub fn pane_series_ids(&self, index: u32) -> Vec<u32> {
        self.engine.pane_series_ids(index as usize)
    }

    /// Attach a pane primitive (reference `IPaneApi.attachPrimitive`, plugin platform Phase C-a) and
    /// return its registry id (0 when the pane index is stale). Fires the plugin's `attached`
    /// hook with `{pane_index}` if present (reference `PaneAttachedParameter`, reduced to what the
    /// host can provide headlessly); a throwing hook detaches nothing and is only reported.
    pub fn attach_pane_primitive(&mut self, pane: u32, primitive: js_sys::Object) -> u32 {
        if pane as usize >= self.panes.len() {
            web_sys::console::warn_1(
                &format!("aeris_charts: attach_pane_primitive ignored — stale pane index {pane}")
                    .into(),
            );
            return 0;
        }
        let id = self.next_primitive_id;
        self.next_primitive_id += 1;
        self.primitives.push(PanePrimitiveEntry {
            id,
            pane,
            obj: primitive.clone(),
        });
        self.engine.invalidate_axis_frame();
        self.axis_dirty = true;
        if let Ok(hook) = js_sys::Reflect::get(&primitive, &"attached".into()) {
            if let Ok(hook) = hook.dyn_into::<js_sys::Function>() {
                let params = js_sys::Object::new();
                let _ = js_sys::Reflect::set(&params, &"pane_index".into(), &pane.into());
                if let Err(error) = hook.call1(&primitive, &params) {
                    web_sys::console::warn_1(
                        &format!("aeris_charts: pane primitive `attached` hook threw — {error:?}")
                            .into(),
                    );
                }
            }
        }
        id
    }

    /// Detach a pane primitive by id (reference `IPaneApi.detachPrimitive`): fires its `detached`
    /// hook and drops the retained object so no JS reference leaks (mirrors the formatter
    /// clear paths). False for an unknown id.
    pub fn detach_pane_primitive(&mut self, id: u32) -> bool {
        let Some(position) = self.primitives.iter().position(|entry| entry.id == id) else {
            return false;
        };
        let entry = self.primitives.remove(position);
        self.engine.invalidate_axis_frame();
        self.axis_dirty = true;
        if let Ok(hook) = js_sys::Reflect::get(&entry.obj, &"detached".into()) {
            if let Ok(hook) = hook.dyn_into::<js_sys::Function>() {
                if let Err(error) = hook.call0(&entry.obj) {
                    web_sys::console::warn_1(
                        &format!("aeris_charts: pane primitive `detached` hook threw — {error:?}")
                            .into(),
                    );
                }
            }
        }
        true
    }

    /// Attach a series primitive (reference `ISeriesApi.attachPrimitive`, plugin platform Phase C-b)
    /// and return its registry id (0 when the series id is unknown or already removed). Fires
    /// the plugin's `attached` hook with `{series_id, pane_index}` if present (`pane_index`
    /// omitted while the series is pane-less); the TS adapter injects `request_update` before
    /// the plugin sees the params. A throwing hook detaches nothing and is only reported.
    pub fn attach_series_primitive(&mut self, series_id: u32, primitive: js_sys::Object) -> u32 {
        let Some(series) = self
            .series
            .iter()
            .find(|s| s.id == series_id as SeriesId && !s.removed)
        else {
            web_sys::console::warn_1(
                &format!(
                    "aeris_charts: attach_series_primitive ignored — unknown/removed series {series_id}"
                )
                .into(),
            );
            return 0;
        };
        let pane_index = series.pane_index;
        let id = self.next_primitive_id;
        self.next_primitive_id += 1;
        self.series_primitives.push(SeriesPrimitiveEntry {
            id,
            series: series_id,
            obj: primitive.clone(),
        });
        self.engine.invalidate_axis_frame();
        self.axis_dirty = true;
        if let Ok(hook) = js_sys::Reflect::get(&primitive, &"attached".into()) {
            if let Ok(hook) = hook.dyn_into::<js_sys::Function>() {
                let params = js_sys::Object::new();
                let _ = js_sys::Reflect::set(&params, &"series_id".into(), &series_id.into());
                if pane_index < self.panes.len() {
                    let _ = js_sys::Reflect::set(
                        &params,
                        &"pane_index".into(),
                        &(pane_index as u32).into(),
                    );
                }
                if let Err(error) = hook.call1(&primitive, &params) {
                    web_sys::console::warn_1(
                        &format!(
                            "aeris_charts: series primitive `attached` hook threw — {error:?}"
                        )
                        .into(),
                    );
                }
            }
        }
        id
    }

    /// Detach a series primitive by id (reference `ISeriesApi.detachPrimitive`): fires its `detached`
    /// hook and drops the retained object. False for an unknown id.
    pub fn detach_series_primitive(&mut self, id: u32) -> bool {
        let Some(position) = self
            .series_primitives
            .iter()
            .position(|entry| entry.id == id)
        else {
            return false;
        };
        let entry = self.series_primitives.remove(position);
        self.engine.invalidate_axis_frame();
        self.axis_dirty = true;
        fire_primitive_detached(&entry.obj);
        true
    }

    /// Auto-detach every series primitive whose owning series is gone (reference drops a removed
    /// series' primitives with it, chart-model.ts `removeSeries`). Called after any
    /// `remove_series` — indicator outputs tombstoned alongside their source are covered too,
    /// since they scan by live state rather than by the removed id.
    fn detach_orphaned_series_primitives(&mut self) {
        if self.series_primitives.is_empty() {
            return;
        }
        let entries = std::mem::take(&mut self.series_primitives);
        let mut orphans = Vec::new();
        for entry in entries {
            let live = self
                .series
                .iter()
                .any(|s| s.id == entry.series as SeriesId && !s.removed);
            if !live {
                orphans.push(entry);
            } else {
                self.series_primitives.push(entry);
            }
        }
        for entry in orphans {
            fire_primitive_detached(&entry.obj);
        }
    }

    /// Merge a snake_case JSON patch of price-scale options into one pane scale (reference
    /// `priceScale.applyOptions`; unknown keys are ignored). Mode/width-affecting keys force
    /// a full axis-width renegotiation like `set_price_scale_mode`.
    pub fn price_scale_apply_options_json(&mut self, pane: u32, target: u32, json: &str) {
        if !self.engine.price_scale_apply_options_json(
            pane as usize,
            price_scale_target_from_u32(target),
            json,
        ) {
            web_sys::console::warn_1(
                &"aeris_charts: price_scale_apply_options_json ignored (unknown pane/target or malformed JSON)".into(),
            );
            return;
        }
        self.recompute_layout(true);
    }

    /// One pane scale's full options as a snake_case JSON string ("" for an unknown
    /// pane/target) — reference `priceScale.options()`.
    pub fn price_scale_options_json(&self, pane: u32, target: u32) -> String {
        self.engine
            .price_scale_options_json(pane as usize, price_scale_target_from_u32(target))
            .unwrap_or_default()
    }

    pub fn add_price_scale_result_json(&mut self, pane: u32, json: &str) -> String {
        let value: serde_json::Value = match serde_json::from_str(json) {
            Ok(value) => value,
            Err(error) => {
                return Self::price_scale_error_json(aeris_charts_engine::ChartError::new(
                    aeris_charts_engine::ErrorCode::InvalidOptions,
                    format!("malformed price scale options: {error}"),
                ));
            }
        };
        let Some(object) = value.as_object() else {
            return Self::price_scale_error_json(aeris_charts_engine::ChartError::new(
                aeris_charts_engine::ErrorCode::InvalidOptions,
                "price scale options must be an object",
            ));
        };
        let Some(id) = object.get("id").and_then(serde_json::Value::as_str) else {
            return Self::price_scale_error_json(aeris_charts_engine::ChartError::new(
                aeris_charts_engine::ErrorCode::InvalidOptions,
                "price scale id is required",
            ));
        };
        let side = match object.get("side").and_then(serde_json::Value::as_str) {
            Some("left") => PriceScaleSide::Left,
            Some("right") => PriceScaleSide::Right,
            _ => {
                return Self::price_scale_error_json(aeris_charts_engine::ChartError::new(
                    aeris_charts_engine::ErrorCode::InvalidOptions,
                    "price scale side must be left or right",
                ));
            }
        };
        let order = object
            .get("order")
            .and_then(serde_json::Value::as_u64)
            .map(|value| value.min(usize::MAX as u64) as usize);
        let visible = object
            .get("visible")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(true);
        let target = match self
            .engine
            .add_price_scale(pane as usize, id, side, order, visible)
        {
            Ok(target) => target,
            Err(error) => return Self::price_scale_error_json(error),
        };
        self.engine
            .price_scale_apply_options_json(pane as usize, target, json);
        self.recompute_layout(true);
        serde_json::json!({"ok": true, "target": price_scale_target_to_u32(target)}).to_string()
    }

    pub fn price_scales_json(&self, pane: u32) -> String {
        let Some(scales) = self.engine.price_scales(pane as usize) else {
            return "[]".to_string();
        };
        serde_json::Value::Array(
            scales
                .into_iter()
                .map(|info| {
                    serde_json::json!({
                        "id": info.id,
                        "side": match info.side {
                            Some(PriceScaleSide::Left) => Some("left"),
                            Some(PriceScaleSide::Right) => Some("right"),
                            None => None,
                        },
                        "order": info.order,
                        "visible": info.visible,
                        "built_in": info.built_in,
                        "pane_index": info.pane_index,
                        "series_ids": info.series_ids,
                    })
                })
                .collect(),
        )
        .to_string()
    }

    pub fn price_scale_target_by_id(&self, pane: u32, id: &str) -> Option<u32> {
        self.engine
            .price_scale_target_for_id(pane as usize, id)
            .map(price_scale_target_to_u32)
    }

    pub fn move_price_scale_result_json(
        &mut self,
        pane: u32,
        target: u32,
        side: &str,
        order: usize,
    ) -> String {
        let side = match side {
            "left" => PriceScaleSide::Left,
            "right" => PriceScaleSide::Right,
            _ => {
                return Self::price_scale_error_json(aeris_charts_engine::ChartError::new(
                    aeris_charts_engine::ErrorCode::InvalidOptions,
                    "price scale side must be left or right",
                ));
            }
        };
        if !self.engine.move_price_scale(
            pane as usize,
            price_scale_target_from_u32(target),
            side,
            order,
        ) {
            return Self::price_scale_error_json(aeris_charts_engine::ChartError::new(
                aeris_charts_engine::ErrorCode::InvalidHandle,
                "price scale is not live or cannot move to that side",
            ));
        }
        self.recompute_layout(true);
        r#"{"ok":true}"#.to_string()
    }

    pub fn remove_price_scale_result_json(&mut self, pane: u32, target: u32) -> String {
        match self
            .engine
            .remove_price_scale(pane as usize, price_scale_target_from_u32(target))
        {
            Ok(()) => {
                self.recompute_layout(true);
                r#"{"ok":true}"#.to_string()
            }
            Err(error) => Self::price_scale_error_json(error),
        }
    }

    /// 0 = candlestick, 1 = OHLC bars, 2 = line, 3 = area, 4 = histogram (sets the main series).
    pub fn set_series_type(&mut self, kind: u8) {
        self.engine
            .convert_series_kind(0, SeriesKind::from_u8(kind));
    }

    pub fn set_series_kind(&mut self, id: u32, kind: u8) -> bool {
        if self.engine.series_kind(id as SeriesId).is_none() {
            return false;
        }
        self.engine
            .convert_series_kind(id as SeriesId, SeriesKind::from_u8(kind));
        true
    }

    pub fn set_time_visible(&mut self, visible: bool) {
        // reference `timeScale.timeVisible` — label semantics only (whether tick/crosshair labels
        // include the time of day). Strip reservation is `set_time_axis_visible`.
        self.engine.set_time_visible(visible);
    }

    /// reference `timeScale.visible`: reserve/collapse the whole time-axis strip.
    pub fn set_time_axis_visible(&mut self, visible: bool) {
        self.engine.set_time_axis_visible(visible);
        // The strip reservation feeds the pane content height — relayout immediately so
        // getters (`pane_height`, `time_scale_height`) agree before the next render.
        self.recompute_layout(true);
    }

    /// reference `timeScale.ticksVisible`: tick marks beside the time-axis labels.
    pub fn set_time_ticks_visible(&mut self, visible: bool) {
        self.engine.set_time_ticks_visible(visible);
    }

    /// reference `timeScale.minimumHeight` (CSS px): floor for the time-axis strip height.
    pub fn set_time_axis_minimum_height(&mut self, height: f64) {
        self.engine.set_time_axis_minimum_height(height);
        self.recompute_layout(true);
    }

    /// reference `timeScale.tickMarkMaxCharacterLength` (0 restores the default 8).
    pub fn set_tick_mark_max_character_length(&mut self, n: u32) {
        self.engine.set_tick_mark_max_character_length(n);
    }

    /// Set/clear the hovered pane separator (reference pane-separator.ts hover; -1 = none). The
    /// host repaints; the next axis frame carries the band position.
    pub fn set_separator_hover(&mut self, index: i32) {
        self.engine
            .set_separator_hover((index >= 0).then_some(index as usize));
    }

    /// reference `timeScale.secondsVisible`: include seconds in time labels when the time is shown.
    pub fn set_seconds_visible(&mut self, visible: bool) {
        self.engine.set_seconds_visible(visible);
    }

    /// reference `timeScale.minBarSpacing`.
    pub fn set_min_bar_spacing(&mut self, spacing: f64) {
        self.engine.set_min_bar_spacing(spacing);
    }

    /// reference `timeScale.maxBarSpacing` (CSS px; 0 restores the default half-width cap).
    pub fn set_max_bar_spacing(&mut self, spacing: f64) {
        self.engine.set_max_bar_spacing(spacing);
    }

    /// reference `timeScale().applyOptions({ barSpacing })`: write the option and apply it live.
    pub fn apply_bar_spacing_option(&mut self, spacing: f64) {
        self.engine.apply_bar_spacing_option(spacing);
    }

    /// reference `timeScale().applyOptions({ rightOffset })`: write the option and apply it live.
    pub fn apply_right_offset_option(&mut self, offset: f64) {
        self.engine.apply_right_offset_option(offset);
    }

    /// reference `timeScale.rightOffsetPixels`: pin the right offset in pixels.
    pub fn set_right_offset_pixels(&mut self, pixels: f64) {
        self.engine.set_right_offset_pixels(pixels);
    }

    /// reference `timeScale.fixLeftEdge`.
    pub fn set_fix_left_edge(&mut self, fix: bool) {
        self.engine.set_fix_left_edge(fix);
    }

    /// reference `timeScale.fixRightEdge`.
    pub fn set_fix_right_edge(&mut self, fix: bool) {
        self.engine.set_fix_right_edge(fix);
    }

    /// reference `timeScale.lockVisibleTimeRangeOnResize`.
    pub fn set_lock_visible_time_range_on_resize(&mut self, lock: bool) {
        self.engine.set_lock_visible_time_range_on_resize(lock);
    }

    /// reference `timeScale.rightBarStaysOnScroll`.
    pub fn set_right_bar_stays_on_scroll(&mut self, stays: bool) {
        self.engine.set_right_bar_stays_on_scroll(stays);
    }

    /// Aeris `lock_visible_logical_range`: hold the visible logical range across data and resizes.
    pub fn set_lock_visible_logical_range(&mut self, lock: bool) {
        self.engine.set_lock_visible_logical_range(lock);
    }

    /// reference `timeScale.shiftVisibleRangeOnNewBar` (default true): when the last bar is
    /// visible, the view follows newly appended bars.
    pub fn set_shift_visible_range_on_new_bar(&mut self, shift: bool) {
        self.engine.set_shift_visible_range_on_new_bar(shift);
    }

    /// reference `timeScale.allowShiftVisibleRangeOnWhitespaceReplacement` (default false).
    pub fn set_allow_shift_visible_range_on_whitespace_replacement(&mut self, allow: bool) {
        self.engine
            .set_allow_shift_visible_range_on_whitespace_replacement(allow);
    }

    /// reference `timeScale.allowBoldLabels` (default true): bold the major time tick labels.
    pub fn set_allow_bold_labels(&mut self, allow: bool) {
        self.engine.set_allow_bold_labels(allow);
    }

    /// reference `localization.dateFormat` (default `dd MMM \'yy`): the crosshair time-label
    /// pattern. Tokens `dd`/`d`, `MM`/`M`/`MMM`/`MMMM`, `yy`/`yyyy` with `'…'` quoting.
    pub fn set_date_format(&mut self, pattern: &str) {
        self.engine.set_date_format(pattern);
    }

    /// reference `localization.locale` (default the browser language): regenerate the engine's
    /// month-name tables (12 short + 12 long) from `Intl.DateTimeFormat` so the date-format
    /// `MMM`/`MMMM` tokens and the month tick labels localize. An invalid/unsupported tag
    /// warns and keeps the current tables.
    pub fn set_locale(&mut self, locale: &str) {
        let Some((short, long)) = locale_month_names(locale) else {
            web_sys::console::warn_1(
                &format!("aeris_charts: set_locale ignored unsupported locale {locale:?}").into(),
            );
            return;
        };
        self.engine.set_month_names(short, long);
    }

    /// reference v5.2 `ISeriesApi.pop(count)`: remove the last `count` data points (clamped to the
    /// data length; point colors shift along). Returns the new data length (0 for an
    /// unknown/removed id — such a series has no data anyway). A custom series' host-side
    /// items truncate in lockstep with the engine rows.
    pub fn series_pop(&mut self, id: u32, count: u32) -> u32 {
        let new_len = self
            .engine
            .series_pop(id as SeriesId, count as usize)
            .unwrap_or(0) as u32;
        if let Some(entry) = self.custom_series.iter_mut().find(|e| e.series == id) {
            crate::custom_align::pop_items(&mut entry.times, &mut entry.items, count as usize);
        }
        new_len
    }

    /// reference `ISeriesApi.lastValueData(globalLast)`: JSON `{"value","formatted","time"}` of the
    /// last (global) or last visible non-whitespace bar; "" when there is none.
    pub fn series_last_value_data(&self, id: u32, global_last: bool) -> String {
        self.engine
            .series_last_value_data(id as SeriesId, global_last)
            .unwrap_or_default()
    }

    /// Format a value with the series' resolved price format, backing the TS
    /// `series.priceFormatter()` ("" for an unknown/removed id).
    pub fn series_format_price(&self, id: u32, value: f64) -> String {
        self.engine
            .series_format_price(id as SeriesId, value)
            .unwrap_or_default()
    }

    /// reference `chart.setCrosshairPosition(price, time, series)`: position the crosshair at a
    /// data point with no DOM event; the next render draws it (the TS layer emits its
    /// crosshair-move event). False when the time is not a bar or the series/scale can't
    /// place it.
    pub fn set_crosshair_position(&mut self, price: f64, time: f64, series_id: u32) -> bool {
        self.engine
            .set_crosshair_position(price, time, series_id as SeriesId)
    }

    /// reference `chart.clearCrosshairPosition`: clear the programmatic crosshair (see the engine
    /// note on the stored position/offset).
    pub fn clear_crosshair_position(&mut self) {
        self.engine.clear_crosshair_position();
    }

    /// Series ids in current render order (topmost LAST) as a JSON array — backs the TS
    /// `chart.seriesOrder()`.
    pub fn series_order_json(&self) -> String {
        self.engine.series_order_json()
    }

    /// reference `chart.setSeriesOrder`: reorder which series paints on top. The ids must name
    /// every live series exactly once; a bad permutation is rejected (false, no change).
    pub fn set_series_order(&mut self, ids: Vec<u32>) -> bool {
        self.engine
            .set_series_order(ids.into_iter().map(|id| id as SeriesId).collect())
    }

    /// Host-pushed "all scaling and scrolling disabled" aggregate (reference
    /// `_isAllScalingAndScrollingDisabled`): forces fix-edge semantics on the time scale.
    pub fn set_interaction_disabled(&mut self, disabled: bool) {
        self.engine.set_interaction_disabled(disabled);
    }

    /// Install/clear the host price formatter (reference `localization.priceFormatter`). The JS callback
    /// receives the numeric price and returns a string; a throw or non-string result falls back to
    /// the built-in formatter.
    pub fn set_price_formatter(&mut self, f: Option<js_sys::Function>) {
        self.engine.set_price_formatter(f.map(|func| {
            Box::new(move |price: f64| {
                func.call1(&JsValue::NULL, &JsValue::from_f64(price))
                    .ok()
                    .and_then(|v| v.as_string())
            }) as PriceFormatterFn
        }));
    }

    /// Install/clear the host time-axis tick formatter (reference `timeScale.tickMarkFormatter`). The JS
    /// callback receives `(timeSeconds, tickMarkType)`.
    pub fn set_tick_mark_formatter(&mut self, f: Option<js_sys::Function>) {
        self.engine.set_tick_mark_formatter(f.map(|func| {
            Box::new(move |ts: i64, tick_type: u8| {
                func.call2(
                    &JsValue::NULL,
                    &JsValue::from_f64(ts as f64),
                    &JsValue::from_f64(tick_type as f64),
                )
                .ok()
                .and_then(|v| v.as_string())
            }) as TickMarkFormatterFn
        }));
    }

    /// Install/clear the host crosshair time formatter (reference `localization.timeFormatter`). The JS
    /// callback receives the UTC-second timestamp.
    pub fn set_time_formatter(&mut self, f: Option<js_sys::Function>) {
        self.engine.set_time_formatter(f.map(|func| {
            Box::new(move |ts: i64| {
                func.call1(&JsValue::NULL, &JsValue::from_f64(ts as f64))
                    .ok()
                    .and_then(|v| v.as_string())
            }) as TimeFormatterFn
        }));
    }

    /// Apply `timeScale.timeZone` (`"UTC"` or explicit `{from_utc_seconds, offset_seconds}`
    /// transitions), `timeScale.sessionStart` (seconds from local midnight),
    /// `timeScale.tickMarks` (explicit `[{time, label?}]` axis marks or `null`), and/or
    /// `timeScale.barTimeLabel` (`"open"`, `null`, or `{anchor: "close", interval_seconds,
    /// windows?}`). The package resolves IANA names to schedules and mark times to UTC seconds
    /// before calling. Returns an empty string on success or the validation message; a rejected
    /// patch changes nothing.
    pub fn set_exchange_time_json(&mut self, time_scale_json: &str) -> String {
        let time_scale: serde_json::Value = match serde_json::from_str(time_scale_json) {
            Ok(value) => value,
            Err(error) => return error.to_string(),
        };
        if !time_scale.is_object() {
            return "exchange time options must be an object".to_string();
        }
        let patch = serde_json::json!({ "timeScale": time_scale }).to_string();
        if let Err(error) = self.engine.apply_options(&patch) {
            return error.to_string();
        }
        self.recompute_layout(true);
        String::new()
    }

    /// Declare whether the chart's financial time points are calendar dates (business-day or
    /// `YYYY-MM-DD` input). The package derives this from its series' input forms.
    pub fn set_calendar_date_axis(&mut self, calendar_dates: bool) {
        self.engine.set_calendar_date_axis(calendar_dates);
    }

    /// Label flags for package-owned time text: bit 0 `timeVisible`, bit 1 `secondsVisible`,
    /// bit 2 calendar-date axis.
    pub fn time_label_flags(&self) -> u32 {
        u32::from(self.engine.time_visible)
            | (u32::from(self.engine.seconds_visible) << 1)
            | (u32::from(self.engine.exchange_time().calendar_dates()) << 2)
    }

    /// Exchange-local wall-clock seconds for a UTC timestamp (identity for calendar dates).
    pub fn exchange_local_seconds(&self, time: f64) -> f64 {
        if !time.is_finite() {
            return f64::NAN;
        }
        self.engine.exchange_local_seconds(time as i64) as f64
    }

    /// The UTC instant a bar identified by open time `time` prints under
    /// `timeScale.barTimeLabel` (`time` itself unless a close-time label is set).
    pub fn bar_label_time(&self, time: f64) -> f64 {
        if !time.is_finite() {
            return f64::NAN;
        }
        self.engine.bar_label_time(time as i64) as f64
    }

    /// 0 = normal, 1 = magnet (reference default), 2 = hidden, 3 = magnet OHLC.
    pub fn set_crosshair_mode(&mut self, mode: u8) {
        self.crosshair_mode = crosshair_mode_from_u8(mode);
        // keep the options store consistent so `options()` reflects it
        self.options.apply(&aeris_charts_core::options::patch(
            "crosshair",
            serde_json::json!({ "mode": mode }),
        ));
    }

    /// Deep-merge a JSON options patch and apply the runtime-affecting fields (crosshair mode,
    /// plus any behavioral `timeScale` keys routed to the core scale by the engine). Colors
    /// (grid/crosshair/background) are read from the store during `render`. Call `render()`
    /// after to repaint (roadmap Phase A2).
    pub fn apply_options(&mut self, patch_json: &str) {
        // `localization.locale` needs the host's `Intl` (the engine is headless), so it is
        // intercepted here; the engine routes `localization.dateFormat` itself and the store
        // keeps both keys for the options round-trip.
        if let Ok(patch) = serde_json::from_str::<serde_json::Value>(patch_json) {
            if let Some(locale) = patch
                .get("localization")
                .and_then(|l| l.get("locale"))
                .and_then(serde_json::Value::as_str)
            {
                self.set_locale(locale);
            }
        }
        if let Err(e) = self.engine.apply_options(patch_json) {
            web_sys::console::warn_1(
                &format!("aeris_charts: apply_options ignored malformed patch - {e}").into(),
            );
        }
        // reference `applyOptions` triggers a FULL update: the axis width renegotiates in both
        // directions (a smaller font/narrower labels shrink the strip). The grow-only rule
        // applies only to the incremental marks path (render()).
        self.recompute_layout(true);
    }

    pub fn reset_style_to_defaults(&mut self, theme: ChartTheme) {
        self.engine.reset_style_to_theme_defaults(theme);
        // Font and axis cosmetics can shrink as well as grow, exactly like a full options update.
        self.recompute_layout(true);
    }

    /// Current options as a JSON string (round-trips the deep-merged state back to JS).
    pub fn options_json(&self) -> String {
        self.options.value().to_string()
    }

    /// A series' current options as a snake_case JSON string ("" for an unknown/removed id).
    pub fn series_options_json(&self, id: u32) -> String {
        self.engine
            .series_options_json(id as SeriesId)
            .unwrap_or_default()
    }

    /// Merge a snake_case JSON patch of series style options into the series (reference
    /// `series.applyOptions`; unknown keys are ignored gracefully).
    pub fn series_apply_options_json(&mut self, id: u32, json: &str) {
        if !self.engine.series_apply_options_json(id as SeriesId, json) {
            web_sys::console::warn_1(
                &"aeris_charts: series_apply_options_json ignored (unknown id or malformed JSON)"
                    .into(),
            );
        }
    }

    /// All time-scale options as a snake_case JSON string.
    pub fn time_scale_options_json(&self) -> String {
        self.engine.time_scale_options_json()
    }

    /// Typed snapshot of the current options for the render path.
    pub(super) fn opts(&self) -> ChartOptions {
        self.options.get().clone()
    }

    pub fn resize(&mut self, css_width: f64, css_height: f64, dpr: f64) {
        self.css_width = css_width;
        self.css_height = css_height;
        self.dpr = dpr;
        let bitmap_w = (css_width * dpr).round().max(1.0) as u32;
        let bitmap_h = (css_height * dpr).round().max(1.0) as u32;
        self.bitmap_w = bitmap_w;
        self.bitmap_h = bitmap_h;
        if let Some(gfx) = self.gfx.as_mut() {
            gfx.config.width = bitmap_w;
            gfx.config.height = bitmap_h;
            gfx.surface.configure(&gfx.shared.device, &gfx.config);
        }
        // Update geometry eagerly so fit_content/zoom/scroll called before the next render
        // (and the price_axis_width getter) see the new pane size, not a stale one.
        self.recompute_layout(true);
    }

    /// Negotiates the price-axis width against its labels and sets the time-scale width / price
    /// scale height. The shared engine policy owns all geometry; this browser host supplies only
    /// Canvas text widths.
    pub(super) fn recompute_layout(&mut self, allow_axis_shrink: bool) {
        let axis_ctx = self.axis_ctx.clone();
        let dpr = self.dpr;
        let layout = self.opts().layout;
        let font_family = layout.font_family;
        let axis_size = self.engine.axis_font_size();
        let countdown_size = self.engine.countdown_font_size();
        self.engine.recompute_layout_with_measure(
            allow_axis_shrink,
            |text, bold| measure_text_ctx(&axis_ctx, dpr, &font_family, axis_size, bold, text),
            |text, bold| measure_text_ctx(&axis_ctx, dpr, &font_family, countdown_size, bold, text),
        );
    }

    // --- gestures ---

    pub fn zoom(&mut self, x_css: f64, scale: f64) {
        let x = x_css.max(1.0).min(self.time_scale.width());
        self.time_scale_zoom(x, scale);
    }
    pub fn zoom_focused(&mut self, x_css: f64, scale: f64) {
        let x = x_css.max(1.0).min(self.time_scale.width());
        self.time_scale_zoom_focused(x, scale);
    }
    pub fn scroll_start(&mut self, x_css: f64) {
        self.time_scale_start_scroll(x_css);
    }
    pub fn scroll_move(&mut self, x_css: f64) {
        self.time_scale_scroll_to(x_css);
    }
    pub fn scroll_end(&mut self) {
        self.time_scale_end_scroll();
    }

    // --- engine-owned interaction models (kinetic, axis drag-to-scale, price pan, eased
    // scroll): the TS recognizer forwards normalized samples and schedules frames; every
    // formula lives in the engine so the headless harness runs the same code. ---

    /// reference wheel zoom increment: `sign(deltaY) * min(1, |deltaY|)`.
    pub fn wheel_zoom_scale(&self, delta_y: f64) -> f64 {
        aeris_charts_engine::wheel_zoom_scale(delta_y)
    }
    /// reference pinch zoom increment: the scale-ratio delta ×5.
    pub fn pinch_zoom_scale(&self, scale_delta: f64) -> f64 {
        aeris_charts_engine::pinch_zoom_scale(scale_delta)
    }
    /// reference wheel scroll: `deltaX * -80` px ("made-up coefficient").
    pub fn wheel_scroll_delta(&self, delta_x: f64) -> f64 {
        delta_x * aeris_charts_engine::WHEEL_SCROLL_PX_PER_DELTA
    }

    /// Open a kinetic sampling session alongside the drag, seeded with logical rightOffset.
    pub fn kinetic_begin_sampling(&mut self, enabled: bool, position: f64, now_ms: f64) {
        self.engine
            .kinetic_begin_sampling(enabled, position, now_ms);
    }
    pub fn kinetic_add_sample(&mut self, position: f64, now_ms: f64) {
        self.engine.kinetic_add_sample(position, now_ms);
    }
    /// The drag was released: returns whether a momentum coast engaged (the host then drives
    /// `kinetic_position` per frame instead of ending the scroll session).
    pub fn kinetic_release(&mut self, position: f64, now_ms: f64) -> bool {
        self.engine.kinetic_release(position, now_ms)
    }
    /// The coast's logical rightOffset at `now_ms` (NaN when no coast runs).
    pub fn kinetic_position(&self, now_ms: f64) -> f64 {
        self.engine.kinetic_position(now_ms).unwrap_or(f64::NAN)
    }
    pub fn kinetic_finished(&self, now_ms: f64) -> bool {
        self.engine.kinetic_finished(now_ms)
    }
    pub fn kinetic_stop(&mut self) {
        self.engine.kinetic_stop();
    }

    pub fn start_keyboard_scroll(&mut self, delta_bars: f64, now_ms: f64) {
        self.engine.start_keyboard_scroll(delta_bars, now_ms);
    }
    pub fn keyboard_scroll_tick(&mut self, now_ms: f64) -> f64 {
        self.engine.keyboard_scroll_tick(now_ms).unwrap_or(f64::NAN)
    }
    pub fn cancel_keyboard_scroll(&mut self) {
        self.engine.cancel_keyboard_scroll();
    }

    /// Axis drag-to-scale arms/applies (reference `TimeAxisWidget`/`PriceAxisWidget`
    /// pressedMouseMove): y is chart-content CSS px for price axes, x pane-relative for time.
    pub fn time_axis_start_scale(&mut self, x_css: f64) {
        self.engine.time_axis_start_scale(x_css);
    }
    pub fn time_axis_scale_to(&mut self, x_css: f64) {
        self.engine.time_axis_scale_to(x_css);
    }
    pub fn time_axis_end_scale(&mut self) {
        self.engine.time_axis_end_scale();
    }
    /// Whether a price-axis drag can scale this scale (false in percentage/indexed-to-100
    /// modes or with no range — reference `PriceScale.scaleTo` no-ops there).
    pub fn price_axis_scalable(&self, pane: usize, target: u32) -> bool {
        self.engine
            .price_axis_scalable(pane, price_scale_target_from_u32(target))
    }
    pub fn price_axis_start_scale(&mut self, pane: usize, target: u32, y_css: f64) {
        self.engine
            .price_axis_start_scale(pane, price_scale_target_from_u32(target), y_css);
    }
    pub fn price_axis_scale_to(&mut self, pane: usize, target: u32, y_css: f64) {
        self.engine
            .price_axis_scale_to(pane, price_scale_target_from_u32(target), y_css);
    }
    pub fn price_axis_end_scale(&mut self, pane: usize, target: u32) {
        self.engine
            .price_axis_end_scale(pane, price_scale_target_from_u32(target));
    }
    /// Vertical price pan (reference `startScrollPrice`/`scrollPriceTo`); autoscale remains locked.
    pub fn price_axis_start_scroll(&mut self, pane: usize, target: u32, y_css: f64) {
        self.engine
            .price_axis_start_scroll(pane, price_scale_target_from_u32(target), y_css);
    }
    pub fn price_axis_scroll_to(&mut self, pane: usize, target: u32, y_css: f64) {
        self.engine
            .price_axis_scroll_to(pane, price_scale_target_from_u32(target), y_css);
    }
    pub fn price_axis_end_scroll(&mut self, pane: usize, target: u32) {
        self.engine
            .price_axis_end_scroll(pane, price_scale_target_from_u32(target));
    }
    pub fn begin_price_pan_at(&mut self, pane: usize, x_css: f64, y_css: f64) -> Option<u32> {
        self.engine
            .begin_price_pan_at(pane, x_css, y_css)
            .map(price_scale_target_to_u32)
    }
    pub fn price_pan_target_at(&self, pane: usize, x_css: f64, y_css: f64) -> Option<u32> {
        self.engine
            .price_pan_target_at(pane, x_css, y_css)
            .map(price_scale_target_to_u32)
    }

    /// Eased scroll-to-position (cubic ease-out): the engine owns the easing and applies each
    /// tick; the host schedules frames and repaints. A newer start or a user gesture
    /// (`cancel_scroll_animation`) supersedes.
    pub fn start_scroll_animation(&mut self, target: f64, duration_ms: f64, now_ms: f64) {
        self.engine
            .start_scroll_animation(target, duration_ms, now_ms);
    }
    /// Apply the eased position for `now_ms`; returns NaN when the animation is finished or
    /// none is running (the host then stops scheduling frames).
    pub fn scroll_animation_tick(&mut self, now_ms: f64) -> f64 {
        self.engine
            .scroll_animation_tick(now_ms)
            .unwrap_or(f64::NAN)
    }
    pub fn cancel_scroll_animation(&mut self) {
        self.engine.cancel_scroll_animation();
    }
    /// Index of the stacked pane containing content-y `y` (engine-owned pane bounds).
    pub fn pane_index_at_y(&self, y_css: f64) -> usize {
        self.engine.pane_index_at_y(y_css)
    }

    pub fn chart_context_at(&self, x_css: f64, y_css: f64) -> Vec<f64> {
        let Some(context) = self.engine.chart_context_at(x_css, y_css) else {
            return Vec::new();
        };
        vec![
            context.x,
            context.y,
            context.pane_index as f64,
            context.time.unwrap_or(f64::NAN),
            context.logical.unwrap_or(f64::NAN),
            context.price,
            context.series.map_or(f64::NAN, f64::from),
        ]
    }

    pub fn price_axis_target_at(&self, pane: usize, x_css: f64) -> Option<u32> {
        self.engine
            .price_axis_target_at(pane, x_css)
            .map(price_scale_target_to_u32)
    }

    pub fn fit_content(&mut self) {
        self.engine.fit_content();
    }
    pub fn scroll_position(&self) -> f64 {
        self.engine.scroll_position()
    }
    pub fn scroll_to_position(&mut self, position: f64) {
        self.engine.scroll_to_position(position);
    }
    pub fn scroll_to_real_time(&mut self) {
        self.engine.scroll_to_real_time();
    }
    pub fn start_real_time_scroll_animation(&mut self, duration_ms: f64, now_ms: f64) {
        self.engine
            .start_real_time_scroll_animation(duration_ms, now_ms);
    }
    pub fn reset_time_scale(&mut self) {
        self.engine.reset_time_scale();
    }
    pub fn time_scale_width(&self) -> f64 {
        self.time_scale.width()
    }
    pub fn time_scale_height(&self) -> f64 {
        // reference `timeScale().height()`: the reserved strip height (0 when `timeScale.visible`
        // is false, else the auto height floored at `minimumHeight`).
        self.engine.time_axis_height()
    }
    pub fn price_scale_width(&self, pane: usize, target: u32) -> f64 {
        self.engine
            .price_scale_axis_width(pane, price_scale_target_from_u32(target))
            .unwrap_or(0.0)
    }
    pub fn price_scale_visible_range(&self, pane: usize, target: u32) -> Vec<f64> {
        self.engine
            .price_scale_visible_range_for(pane, price_scale_target_from_u32(target))
            .map(|(from, to)| vec![from, to])
            .unwrap_or_default()
    }
    pub fn set_price_scale_visible_range(&mut self, pane: usize, target: u32, from: f64, to: f64) {
        self.engine.set_price_scale_visible_range_for(
            pane,
            price_scale_target_from_u32(target),
            from,
            to,
        );
    }
    pub fn price_scale_auto_scale(&self, pane: usize, target: u32) -> Option<bool> {
        self.engine
            .price_scale_auto_scale_for(pane, price_scale_target_from_u32(target))
    }
    pub fn set_price_scale_auto_scale(&mut self, pane: usize, target: u32, enabled: bool) {
        self.engine.set_price_scale_auto_scale_for(
            pane,
            price_scale_target_from_u32(target),
            enabled,
        );
    }
    pub fn price_scale_inverted(&self, pane: usize, target: u32) -> Option<bool> {
        self.engine
            .price_scale_inverted_for(pane, price_scale_target_from_u32(target))
    }
    pub fn set_price_scale_inverted(&mut self, pane: usize, target: u32, inverted: bool) {
        self.engine.set_price_scale_inverted_for(
            pane,
            price_scale_target_from_u32(target),
            inverted,
        );
    }
    pub fn price_scale_margins(&self, pane: usize, target: u32) -> Vec<f64> {
        self.engine
            .price_scale_margins_for(pane, price_scale_target_from_u32(target))
            .map(|(top, bottom)| vec![top, bottom])
            .unwrap_or_default()
    }
    pub fn set_price_scale_margins(&mut self, pane: usize, target: u32, top: f64, bottom: f64) {
        self.engine.set_price_scale_margins_for(
            pane,
            price_scale_target_from_u32(target),
            top,
            bottom,
        );
    }
    pub fn price_scale_mode(&self, pane: usize, target: u32) -> Option<u8> {
        self.engine
            .price_scale_mode_for(pane, price_scale_target_from_u32(target))
            .map(price_scale_mode_to_u8)
    }
    pub fn set_price_scale_mode(&mut self, pane: usize, target: u32, mode: u8) {
        self.engine.set_price_scale_mode_for(
            pane,
            price_scale_target_from_u32(target),
            price_scale_mode_from_u8(mode),
        );
        // A mode change is a full layout invalidation in reference: label formatting can become wider
        // (percentage) or narrower (indexed/normal), so the grow-fast/shrink-on-full-layout axis
        // policy must be allowed to renegotiate in both directions immediately.
        self.recompute_layout(true);
    }
    pub fn series_pane_index(&self, id: u32) -> Option<usize> {
        self.engine
            .series_price_scale(id as SeriesId)
            .map(|(pane, _)| pane)
    }
    pub fn series_is_overlay(&self, id: u32) -> Option<bool> {
        self.engine
            .series_price_scale(id as SeriesId)
            .map(|(_, target)| target == PriceScaleTarget::Overlay)
    }
    pub fn series_price_scale_id(&self, id: u32) -> Option<u32> {
        self.engine
            .series_price_scale(id as SeriesId)
            .map(|(_, target)| price_scale_target_to_u32(target))
    }
    pub fn series_price_scale_name(&self, id: u32) -> String {
        let Some((pane, target)) = self.engine.series_price_scale(id as SeriesId) else {
            return String::new();
        };
        self.engine
            .price_scale_id_for_target(pane, target)
            .unwrap_or("")
            .to_string()
    }
    pub fn set_series_price_scale_by_name(&mut self, id: u32, name: &str) -> bool {
        let Some((pane, _)) = self.engine.series_price_scale(id as SeriesId) else {
            return false;
        };
        let Some(target) = self.engine.price_scale_target_for_id(pane, name) else {
            return false;
        };
        self.engine.set_series_price_scale(id as SeriesId, target);
        self.recompute_layout(true);
        true
    }
    pub fn set_series_price_scale(&mut self, id: u32, target: u32) {
        self.engine
            .set_series_price_scale(id as SeriesId, price_scale_target_from_u32(target));
        self.recompute_layout(true);
    }
    pub fn series_price_to_coordinate(&self, id: u32, price: f64) -> Option<f64> {
        self.engine
            .series_price_to_coordinate(id as SeriesId, price)
    }
    pub fn series_coordinate_to_price(&self, id: u32, coordinate: f64) -> Option<f64> {
        self.engine
            .series_coordinate_to_price(id as SeriesId, coordinate)
    }
    pub fn series_kind(&self, id: u32) -> Option<u8> {
        self.engine
            .series_kind(id as SeriesId)
            .map(SeriesKind::to_u8)
    }

    /// Whether the engine writes this series' rows: a footprint, a trade-bound candle or bar, a
    /// CVD, delta, or volume study, resampled or synthetic bars. Every host data write to such a
    /// series is refused, so the boundary decides with this, never from a refused write.
    pub fn series_is_source_owned(&self, id: u32) -> bool {
        self.engine.series_is_source_owned(id as SeriesId)
    }

    /// The `derived_series` rejection for a host write to an engine-owned series, `None` for any
    /// other id. Checked before a batch is copied or sanitized.
    pub(super) fn derived_write_rejection(&self, id: u32) -> Option<String> {
        self.series_is_source_owned(id)
            .then(|| derived_series_rejection(id))
    }
    pub fn series_data_by_index(&self, id: u32, index: f64, mismatch: i8) -> Vec<f64> {
        if !index.is_finite() || index.fract() != 0.0 {
            return Vec::new();
        }
        self.engine
            .series_data_by_index(
                id as SeriesId,
                index as i64,
                mismatch_direction_from_i8(mismatch),
            )
            .map(|point| {
                vec![
                    point.time as f64,
                    point.open,
                    point.high,
                    point.low,
                    point.close,
                ]
            })
            .unwrap_or_default()
    }
    pub fn series_data(&self, id: u32) -> Vec<f64> {
        let points = self.engine.series_data(id as SeriesId);
        let mut output = Vec::with_capacity(points.len() * 5);
        for point in points {
            output.extend_from_slice(&[
                point.time as f64,
                point.open,
                point.high,
                point.low,
                point.close,
            ]);
        }
        output
    }

    /// Encode the engine-owned chart value snapshot. `NaN` selects latest mode; finite integer
    /// values select exact logical mode. No history or snapshot semantics are assembled here.
    pub fn value_snapshot_json(&self, logical_index: f64) -> String {
        let logical_index = if logical_index.is_nan() {
            None
        } else if logical_index.is_finite()
            && logical_index.fract() == 0.0
            && logical_index >= i64::MIN as f64
            && logical_index <= i64::MAX as f64
        {
            Some(logical_index as i64)
        } else {
            return "[]".to_string();
        };
        serde_json::to_string(&self.engine.value_snapshot(logical_index))
            .unwrap_or_else(|_| "[]".to_string())
    }

    pub fn set_comparison_anchor(&mut self, time: f64) -> bool {
        let anchor = if time.is_nan() { None } else { Some(time) };
        self.engine.set_comparison_anchor(anchor)
    }

    pub fn comparison_anchor(&self) -> f64 {
        self.engine
            .comparison_anchor()
            .map_or(f64::NAN, |time| time as f64)
    }

    pub fn comparison_legend_json(&self) -> String {
        serde_json::to_string(&self.engine.comparison_legend_snapshot())
            .unwrap_or_else(|_| "[]".to_string())
    }
    pub fn series_bars_in_logical_range(&self, id: u32, from: f64, to: f64) -> Vec<f64> {
        self.engine
            .series_bars_in_logical_range(id as SeriesId, from, to)
            .map(|info| {
                let mut output = vec![info.bars_before, info.bars_after];
                if let (Some(from), Some(to)) = (info.from, info.to) {
                    output.extend_from_slice(&[from as f64, to as f64]);
                }
                output
            })
            .unwrap_or_default()
    }
    pub fn set_crosshair(&mut self, x_css: f64, y_css: f64) {
        self.set_crosshair_at(x_css, y_css);
    }
    pub fn clear_crosshair(&mut self) {
        self.clear_crosshair_at();
    }

    // --- drawing tools (engine-owned drawing objects; aeris_charts_engine drawings.rs) ---
    //
    // Thin wire adapters: kinds cross as `u8` (`DrawingKind::from_u8`), anchors and options as
    // JSON strings. All state, hit-testing, and drag math is engine-side; the gesture layer only
    // forwards pointer samples and repaints.

    /// Add a drawing from a JSON `[{logical?, time?, price}, ...]` anchor array plus an optional
    /// options patch ("" = defaults). Returns the drawing id, or 0 when the engine rejects it.
    pub fn add_drawing(
        &mut self,
        kind: u8,
        pane: usize,
        points_json: &str,
        options_json: &str,
    ) -> u32 {
        let Some(kind) = DrawingKind::from_u8(kind) else {
            return 0;
        };
        let Ok(anchors) = serde_json::from_str::<Vec<DrawingAnchor>>(points_json) else {
            return 0;
        };
        let options = (!options_json.is_empty()).then_some(options_json);
        self.engine
            .add_drawing_anchors(kind, pane, &anchors, options)
            .unwrap_or(0)
    }
    /// Result envelope form of [`Self::add_drawing`]: `{ok:true,id}` or `{ok:false,error}` naming
    /// whether the anchors or the options patch were rejected.
    pub fn add_drawing_result_json(
        &mut self,
        kind: u8,
        pane: usize,
        points_json: &str,
        options_json: &str,
    ) -> String {
        let result = DrawingKind::from_u8(kind)
            .ok_or_else(|| drawing_invalid_data("unknown drawing kind"))
            .and_then(|kind| {
                let anchors = serde_json::from_str::<Vec<DrawingAnchor>>(points_json)
                    .map_err(|error| drawing_invalid_data(format!("malformed anchors: {error}")))?;
                let options = (!options_json.is_empty()).then_some(options_json);
                self.engine
                    .add_drawing_anchors(kind, pane, &anchors, options)
            });
        match result {
            Ok(id) => serde_json::json!({ "ok": true, "id": id }).to_string(),
            Err(error) => drawing_error_json(&error),
        }
    }
    pub fn drawing_apply_options(&mut self, id: u32, options_json: &str) -> bool {
        self.engine.drawing_apply_options(id, options_json)
    }
    pub fn drawing_set_points(&mut self, id: u32, points_json: &str) -> bool {
        self.engine.drawing_set_points(id, points_json)
    }
    /// Result envelope form of [`Self::drawing_set_points`].
    pub fn drawing_set_points_result_json(&mut self, id: u32, points_json: &str) -> String {
        let result = serde_json::from_str::<Vec<DrawingAnchor>>(points_json)
            .map_err(|error| drawing_invalid_data(format!("malformed anchors: {error}")))
            .and_then(|anchors| self.engine.set_drawing_anchors(id, &anchors));
        match result {
            Ok(()) => serde_json::json!({ "ok": true }).to_string(),
            Err(error) => drawing_error_json(&error),
        }
    }
    /// Atomic multi-drawing anchor rewrite from `[{id, points: [...]}, ...]` (one undo step):
    /// `{ok:true,changed}` or `{ok:false,error}`.
    pub fn set_drawings_points_result_json(&mut self, updates_json: &str) -> String {
        #[derive(serde::Deserialize)]
        struct Update {
            id: u32,
            points: Vec<DrawingAnchor>,
        }
        let result = serde_json::from_str::<Vec<Update>>(updates_json)
            .map_err(|error| drawing_invalid_data(format!("malformed updates: {error}")))
            .and_then(|updates| {
                let updates = updates
                    .into_iter()
                    .map(|update| (update.id, update.points))
                    .collect::<Vec<_>>();
                self.engine.set_drawings_anchors(&updates)
            });
        match result {
            Ok(changed) => serde_json::json!({ "ok": true, "changed": changed }).to_string(),
            Err(error) => drawing_error_json(&error),
        }
    }
    /// Price-basis rescale from `[{from_time?, to_time?, factor}, ...]`; `price_basis` `None`
    /// keeps the current label. `{ok:true,changed}` or `{ok:false,error}`.
    pub fn rescale_drawing_prices_result_json(
        &mut self,
        segments_json: &str,
        price_basis: Option<String>,
    ) -> String {
        let result = serde_json::from_str::<Vec<DrawingPriceSegment>>(segments_json)
            .map_err(|error| drawing_invalid_data(format!("malformed price segments: {error}")))
            .and_then(|segments| {
                self.engine
                    .rescale_drawing_prices(&segments, price_basis.as_deref())
            });
        match result {
            Ok(changed) => serde_json::json!({ "ok": true, "changed": changed }).to_string(),
            Err(error) => drawing_error_json(&error),
        }
    }
    pub fn set_drawing_price_basis(&mut self, basis: Option<String>) -> bool {
        self.engine
            .set_drawing_price_basis(basis.as_deref())
            .is_ok()
    }
    pub fn drawing_price_basis(&self) -> Option<String> {
        self.engine.drawing_price_basis().map(str::to_string)
    }
    /// Chart drawing magnet: 0 off, 1 weak, 2 strong (unknown values are rejected).
    pub fn set_drawing_magnet_mode(&mut self, mode: u8) -> bool {
        let Some(mode) = drawing_magnet_from_u8(mode) else {
            return false;
        };
        self.engine.set_drawing_magnet_mode(mode);
        true
    }
    pub fn drawing_magnet_mode(&self) -> u8 {
        drawing_magnet_to_u8(self.engine.drawing_magnet_mode())
    }
    /// Effective magnet for the armed tool's next placement with the Ctrl/Cmd toggle state.
    pub fn armed_drawing_magnet(&self, toggle: bool) -> u8 {
        drawing_magnet_to_u8(self.engine.armed_drawing_magnet(toggle))
    }
    /// Keyboard handle count of a drawing, or -1 for an unknown/unplaceable drawing.
    pub fn drawing_handle_count(&self, id: u32) -> i32 {
        self.engine
            .drawing_handle_count(id)
            .and_then(|count| i32::try_from(count).ok())
            .unwrap_or(-1)
    }
    /// The drawing's options JSON ("" for an unknown id — the wasm boundary has no Option<String>).
    pub fn drawing_options_json(&self, id: u32) -> String {
        self.engine.drawing_options_json(id).unwrap_or_default()
    }
    pub fn drawing_property_schema_json(&self, id: u32) -> String {
        self.engine
            .drawing_property_schema_json(id)
            .unwrap_or_default()
    }

    pub fn drawing_kind_options_json(&self, id: u32) -> String {
        self.engine
            .drawing_kind_options_json(id)
            .unwrap_or_default()
    }
    pub fn drawing_object_tree_json(&self) -> String {
        self.engine.drawing_object_tree_json()
    }
    pub fn drawing_points_json(&self, id: u32) -> String {
        self.engine.drawing_points_json(id).unwrap_or_default()
    }
    /// One anchor's CSS-px position `[x, y]` (x includes the pane's left offset — overlay
    /// space; empty when it cannot convert). The text editor positions itself with it.
    pub fn drawing_point_to_coordinate(&self, id: u32, index: usize) -> Vec<f64> {
        self.engine
            .drawing_point_to_coordinate(id, index)
            .map(|(x, y)| vec![x + self.pane_left, y])
            .unwrap_or_default()
    }
    pub fn drawing_text_coordinate(&self, id: u32) -> Vec<f64> {
        self.engine
            .drawing_text_coordinate(id)
            .map(|(x, y)| vec![x + self.pane_left, y])
            .unwrap_or_default()
    }
    pub fn drawing_text_transform(&self, id: u32) -> Vec<f64> {
        self.engine
            .drawing_text_transform(id)
            .map(|(x, y, angle)| vec![x + self.pane_left, y, angle])
            .unwrap_or_default()
    }
    pub fn drawing_text_hit_at(&self, x_css: f64, y_css: f64) -> u32 {
        self.engine.drawing_text_hit_at(x_css, y_css).unwrap_or(0)
    }
    pub fn drawing_at(&self, x_css: f64, y_css: f64) -> u32 {
        self.engine.drawing_at(x_css, y_css).unwrap_or(0)
    }
    pub fn drawing_text_edit_layout_json(&self, id: u32) -> String {
        self.engine
            .drawing_text_edit_layout(id)
            .map(|mut layout| {
                layout.x += self.pane_left;
                layout.rect[0] += self.pane_left;
                layout.rect[2] += self.pane_left;
                serde_json::to_string(&layout).unwrap_or_default()
            })
            .unwrap_or_default()
    }
    pub fn drawings_json(&self) -> String {
        self.engine.drawings_json()
    }
    pub fn set_drawing_interval(&mut self, interval_json: &str) -> bool {
        let interval = if interval_json.is_empty() {
            None
        } else {
            serde_json::from_str(interval_json).ok()
        };
        if !interval_json.is_empty() && interval.is_none() {
            return false;
        }
        self.engine.set_drawing_interval(interval);
        true
    }
    pub fn selected_drawings_json(&self) -> String {
        serde_json::to_string(self.engine.selected_drawings()).unwrap_or_else(|_| "[]".to_string())
    }
    pub fn set_selected_drawings(&mut self, ids_json: &str) -> bool {
        let Ok(ids) = serde_json::from_str::<Vec<u32>>(ids_json) else {
            return false;
        };
        self.engine.set_selected_drawings(&ids)
    }
    /// `{"ok":true,"payload":"..."}`, or the engine's error (`invalid_data` when nothing could be
    /// copied, `resource_limit` past the clipboard bounds).
    pub fn copy_drawings_json(&self, ids_json: &str) -> String {
        let ids = serde_json::from_str::<Vec<u32>>(ids_json).unwrap_or_default();
        let result = match self.engine.copy_drawings_json(&ids) {
            Ok(payload) => serde_json::json!({ "ok": true, "payload": payload }),
            Err(error) => serde_json::json!({
                "ok": false,
                "error": { "code": error.code().name(), "message": error.message() },
            }),
        };
        result.to_string()
    }
    pub fn paste_drawings_json(
        &mut self,
        payload: &str,
        pane: usize,
        logical_offset: f64,
        price_offset: f64,
    ) -> String {
        serde_json::to_string(
            &self
                .engine
                .paste_drawings_json(payload, pane, logical_offset, price_offset)
                .unwrap_or_default(),
        )
        .unwrap_or_else(|_| "[]".to_string())
    }
    pub fn clone_drawing(&mut self, id: u32, logical_offset: f64, price_offset: f64) -> u32 {
        self.engine
            .clone_drawing(id, logical_offset, price_offset)
            .unwrap_or(0)
    }
    pub fn move_drawing_z_order(&mut self, id: u32, delta: i32) -> bool {
        self.engine.move_drawing_z_order(id, delta)
    }
    pub fn set_drawing_visibility(&mut self, id: u32, visible: bool) -> bool {
        self.engine.set_drawing_visibility(id, visible)
    }
    pub fn set_drawing_locked(&mut self, id: u32, locked: bool) -> bool {
        self.engine.set_drawing_locked(id, locked)
    }
    pub fn set_drawing_group(&mut self, id: u32, group_json: &str) -> bool {
        let group = serde_json::from_str::<Option<String>>(group_json).unwrap_or(None);
        self.engine.set_drawing_group(id, group)
    }
    pub fn set_drawing_group_visibility(&mut self, group_id: &str, visible: bool) -> u32 {
        self.engine.set_drawing_group_visibility(group_id, visible) as u32
    }
    pub fn set_drawing_group_locked(&mut self, group_id: &str, locked: bool) -> u32 {
        self.engine.set_drawing_group_locked(group_id, locked) as u32
    }
    pub fn move_drawing_group(
        &mut self,
        group_id: &str,
        logical_delta: f64,
        price_delta: f64,
    ) -> u32 {
        self.engine
            .move_drawing_group(group_id, logical_delta, price_delta) as u32
    }
    pub fn drawing_sync_payload_json(&self, source: &str) -> String {
        self.engine
            .drawing_sync_payload_json(source)
            .unwrap_or_default()
    }
    pub fn apply_drawing_sync_payload_json(&mut self, payload: &str) -> bool {
        self.engine.apply_drawing_sync_payload_json(payload)
    }
    pub fn apply_drawing_template_json(&mut self, id: u32, template_json: &str) -> bool {
        self.engine.apply_drawing_template_json(id, template_json)
    }
    pub fn drawing_template_json(&self, id: u32, name: &str) -> String {
        self.engine
            .drawing_template_json(id, name)
            .unwrap_or_default()
    }
    pub fn remove_drawing(&mut self, id: u32) -> bool {
        self.engine.remove_drawing(id)
    }
    pub fn clear_drawings(&mut self) {
        self.engine.clear_drawings();
    }
    /// Click-to-select arbitration: selects the drawing under the point (clearing on a miss) and
    /// reports whether one was hit, so the host can skip its series-selection path.
    pub fn select_drawing_at(&mut self, x_css: f64, y_css: f64) -> bool {
        self.engine.select_drawing_at(x_css, y_css)
    }
    pub fn set_selected_drawing(&mut self, id: Option<u32>) {
        self.engine.set_selected_drawing(id);
    }
    pub fn selected_drawing(&self) -> Option<u32> {
        self.engine.selected_drawing()
    }
    /// Delete/Backspace: remove the selected drawing. False while nothing is selected.
    pub fn remove_selected_drawing(&mut self) -> bool {
        self.engine.remove_selected_drawing()
    }
    /// Press routing: opens an anchor/body drag on the drawing under the point (false = the host
    /// falls through to pan/scroll).
    pub fn drawing_drag_start_at(&mut self, x_css: f64, y_css: f64) -> bool {
        self.engine.drawing_drag_start_at(x_css, y_css)
    }
    /// Forward a drag position with the modifier state (magnet = rendered-price snap, straighten =
    /// 0°/45°/90° anchor constraint / dominant-axis body move; drawings.rs `DrawingModifiers`).
    pub fn drawing_drag_to(&mut self, x_css: f64, y_css: f64, magnet: bool, straighten: bool) {
        self.engine
            .drawing_drag_to(x_css, y_css, DrawingModifiers { magnet, straighten });
    }
    pub fn drawing_drag_end(&mut self) {
        self.engine.drawing_drag_end();
    }
    pub fn drawing_drag_active(&self) -> bool {
        self.engine.drawing_drag_active()
    }

    pub fn set_series_markers_z_order(&mut self, series_id: u32, z_order: u8) -> bool {
        self.engine
            .set_series_markers_z_order(series_id as SeriesId, z_order)
    }
    pub fn undo_drawing(&mut self) -> bool {
        self.engine.undo_drawing()
    }
    pub fn redo_drawing(&mut self) -> bool {
        self.engine.redo_drawing()
    }
    pub fn can_undo_drawing(&self) -> bool {
        self.engine.can_undo_drawing()
    }
    pub fn can_redo_drawing(&self) -> bool {
        self.engine.can_redo_drawing()
    }

    // Canonical drawing-tool controller. These are the host-facing interaction methods; the
    // lower-level drawing_create_*/brush_create_* methods below remain as compatibility/test seams.
    pub fn set_drawing_tool(&mut self, kind: i32, options_json: &str, pane: i32) -> bool {
        let kind = if kind < 0 {
            None
        } else {
            let Ok(wire_kind) = u8::try_from(kind) else {
                return false;
            };
            let Some(kind) = DrawingKind::from_u8(wire_kind) else {
                return false;
            };
            Some(kind)
        };
        let pane = usize::try_from(pane).ok();
        let options = (!options_json.is_empty()).then_some(options_json);
        self.engine.set_drawing_tool(kind, options, pane)
    }

    pub fn active_drawing_tool(&self) -> i32 {
        self.engine
            .active_drawing_tool()
            .map_or(-1, |kind| i32::from(kind.to_u8()))
    }

    pub fn active_drawing_tool_pane(&self) -> i32 {
        self.engine
            .active_drawing_tool_pane()
            .and_then(|pane| i32::try_from(pane).ok())
            .unwrap_or(-1)
    }

    pub fn drawing_tool_apply_options(&mut self, options_json: &str) -> bool {
        self.engine.drawing_tool_apply_options(options_json)
    }

    pub fn drawing_tool_pointer_down(
        &mut self,
        x_css: f64,
        y_css: f64,
        magnet: bool,
        straighten: bool,
    ) -> u32 {
        self.engine
            .drawing_tool_pointer_down(x_css, y_css, DrawingModifiers { magnet, straighten })
            .created
            .unwrap_or(0)
    }

    pub fn drawing_tool_pointer_move(
        &mut self,
        x_css: f64,
        y_css: f64,
        magnet: bool,
        straighten: bool,
        pressed: bool,
    ) -> bool {
        self.engine
            .drawing_tool_pointer_move(
                x_css,
                y_css,
                DrawingModifiers { magnet, straighten },
                pressed,
            )
            .changed
    }

    pub fn drawing_tool_pointer_up(
        &mut self,
        x_css: f64,
        y_css: f64,
        magnet: bool,
        straighten: bool,
    ) -> u32 {
        self.engine
            .drawing_tool_pointer_up(x_css, y_css, DrawingModifiers { magnet, straighten })
            .created
            .unwrap_or(0)
    }

    pub fn drawing_tool_activate(
        &mut self,
        x_css: f64,
        y_css: f64,
        magnet: bool,
        straighten: bool,
    ) -> u32 {
        self.engine
            .drawing_tool_activate(x_css, y_css, DrawingModifiers { magnet, straighten })
            .created
            .unwrap_or(0)
    }

    pub fn drawing_tool_finish(&mut self) -> u32 {
        self.engine.drawing_tool_finish().created.unwrap_or(0)
    }

    pub fn drawing_tool_pop_anchor(&mut self) -> bool {
        self.engine.drawing_tool_pop_anchor()
    }

    pub fn drawing_tool_capture_active(&self) -> bool {
        self.engine.drawing_tool_capture_active()
    }

    pub fn drawing_tool_sequence_active(&self) -> bool {
        self.engine.drawing_tool_sequence_active()
    }

    pub fn drawing_requests_text_edit(&self, id: u32) -> bool {
        self.engine.drawing_requests_text_edit(id)
    }

    pub fn cancel_drawing_creation(&mut self) {
        self.engine.cancel_drawing_creation();
    }

    pub fn cancel_drawing_tool(&mut self) {
        self.engine.cancel_drawing_tool();
    }

    /// Arm interactive creation of a tool kind ("" options = defaults).
    pub fn drawing_create_begin(&mut self, kind: u8, options_json: &str) -> bool {
        let Some(kind) = DrawingKind::from_u8(kind) else {
            return false;
        };
        let options = (!options_json.is_empty()).then_some(options_json);
        self.engine.drawing_create_begin(kind, options)
    }
    pub fn drawing_create_apply_options(&mut self, options_json: &str) -> bool {
        self.engine.drawing_create_apply_options(options_json)
    }
    /// Place the next creation anchor (modifiers snap it): 0 unarmed, -1 pending more anchors,
    /// > 0 the committed id.
    pub fn drawing_create_click(
        &mut self,
        x_css: f64,
        y_css: f64,
        magnet: bool,
        straighten: bool,
    ) -> i64 {
        self.engine
            .drawing_create_click(x_css, y_css, DrawingModifiers { magnet, straighten })
    }
    pub fn drawing_create_move(&mut self, x_css: f64, y_css: f64, magnet: bool, straighten: bool) {
        self.engine
            .drawing_create_move(x_css, y_css, DrawingModifiers { magnet, straighten });
    }
    pub fn drawing_create_finish(&mut self) -> u32 {
        self.engine.drawing_create_finish()
    }
    pub fn drawing_create_pop_anchor(&mut self) -> bool {
        self.engine.drawing_create_pop_anchor()
    }
    pub fn drawing_create_cancel(&mut self) {
        self.engine.drawing_create_cancel();
    }
    pub fn drawing_create_active(&self) -> bool {
        self.engine.drawing_create_active()
    }

    /// Begin a freehand brush stroke (pointer-down with the brush tool armed; "" options =
    /// defaults). False off the panes/data.
    pub fn brush_create_start(&mut self, options_json: &str, x_css: f64, y_css: f64) -> bool {
        let options = (!options_json.is_empty()).then_some(options_json);
        self.engine.brush_create_start(options, x_css, y_css)
    }
    /// Capture the next stroke point (engine-decimated by distance).
    pub fn brush_create_add(&mut self, x_css: f64, y_css: f64) {
        self.engine.brush_create_add(x_css, y_css);
    }
    /// Commit the stroke (pointer-up): the captured path is stored as-is as a selected
    /// drawing. 0 = degenerate stroke discarded.
    pub fn brush_create_end(&mut self) -> u32 {
        self.engine.brush_create_end()
    }
    pub fn brush_create_cancel(&mut self) {
        self.engine.brush_create_cancel();
    }
    pub fn brush_create_active(&self) -> bool {
        self.engine.brush_create_active()
    }
    pub fn price_axis_width(&self) -> f64 {
        self.axis_w
    }
    pub fn pane_left(&self) -> f64 {
        self.pane_left
    }

    // --- coordinate & logical-range API (roadmap Phase A4) ---
    //
    // Reflects the state of the last render (scale height/width, price range). X is CSS px from
    // the plot-area left edge and Y CSS px from the top of the stacked pane area (never
    // pane-local), matching the pointer coords JS passes to `set_crosshair`. `None`/empty means
    // the query falls off the chart or there is no data.

    /// Y (CSS px, chart content) for a price on pane 0's default price scale, or `None` if that
    /// scale has no range yet. Series handles convert on their own pane and scale instead.
    pub fn price_to_coordinate(&self, price: f64) -> Option<f64> {
        self.engine.pane_price_to_coordinate(0, price)
    }

    /// Price for a Y (CSS px, chart content) on the default price scale of the pane containing it
    /// (the scale the crosshair label reads there), or `None` if that scale has no range yet.
    pub fn coordinate_to_price(&self, y_css: f64) -> Option<f64> {
        self.engine.coordinate_to_price(y_css)
    }

    /// X (CSS px) for a UTC-seconds timestamp that sits exactly on a data point, else `None`
    /// (mirrors reference `timeToCoordinate`, which does not snap to the nearest bar).
    pub fn time_to_coordinate(&self, time: f64) -> Option<f64> {
        self.engine.time_to_coordinate(time)
    }

    /// UTC-seconds timestamp of the data point nearest to X (CSS px), or `None` if X maps outside
    /// the data range (mirrors reference `coordinateToTime`).
    pub fn coordinate_to_time(&self, x_css: f64) -> Option<f64> {
        self.engine.coordinate_to_time(x_css)
    }

    /// Integer logical bar owning an X coordinate, or `None` when there is no data. May be negative
    /// or beyond the last bar, matching the reference's public `coordinateToLogical`.
    pub fn coordinate_to_logical(&self, x_css: f64) -> Option<f64> {
        self.engine.coordinate_to_logical(x_css)
    }

    pub fn logical_to_coordinate(&self, logical: f64) -> Option<f64> {
        self.engine.logical_to_coordinate(logical)
    }

    pub fn time_to_index(&self, time: f64, find_nearest: bool) -> Option<i64> {
        self.engine.time_to_index(time, find_nearest)
    }

    /// Visible window in logical (bar) units as `[from, to]`, or empty when there is no data.
    pub fn visible_logical_range(&self) -> Vec<f64> {
        match self.engine.visible_logical_range() {
            Some((from, to)) => vec![from, to],
            None => Vec::new(),
        }
    }

    /// Set the visible window in logical (bar) units. No-op if `from > to`. Call `render()` after.
    pub fn set_visible_logical_range(&mut self, from: f64, to: f64) {
        self.engine.set_visible_logical_range(from, to);
    }

    /// Visible window as `[from_time, to_time]` UTC seconds (data points nearest each edge), or
    /// empty when there is no data.
    pub fn visible_time_range(&self) -> Vec<f64> {
        self.engine
            .visible_time_range()
            .map(|(from, to)| vec![from, to])
            .unwrap_or_default()
    }

    /// Set the visible window to span the data points bracketing `[from_time, to_time]` (UTC
    /// seconds). No-op if the times are reversed or there is no data. Call `render()` after.
    pub fn set_visible_time_range(&mut self, from_time: f64, to_time: f64) {
        self.engine.set_visible_time_range(from_time, to_time);
    }
}

/// Engine autoscale info -> the package's `autoscale_info` object (`null` for `None`).
fn autoscale_info_to_js(info: Option<aeris_charts_engine::AutoscaleInfo>) -> JsValue {
    let Some(info) = info else {
        return JsValue::NULL;
    };
    let object = js_sys::Object::new();
    let range = match info.price_range {
        Some((min, max)) => {
            let range = js_sys::Object::new();
            let _ = js_sys::Reflect::set(&range, &"min_value".into(), &JsValue::from_f64(min));
            let _ = js_sys::Reflect::set(&range, &"max_value".into(), &JsValue::from_f64(max));
            range.into()
        }
        None => JsValue::NULL,
    };
    let _ = js_sys::Reflect::set(&object, &"price_range".into(), &range);
    if let Some((above, below)) = info.margins {
        let margins = js_sys::Object::new();
        let _ = js_sys::Reflect::set(&margins, &"above".into(), &JsValue::from_f64(above));
        let _ = js_sys::Reflect::set(&margins, &"below".into(), &JsValue::from_f64(below));
        let _ = js_sys::Reflect::set(&object, &"margins".into(), &margins);
    }
    object.into()
}

/// The package's `autoscale_info` object -> engine info. `null`/`undefined` is `None`; a missing,
/// `null`, or non-numeric range contributes no range (the engine re-validates finiteness/order).
fn autoscale_info_from_js(value: &JsValue) -> Option<aeris_charts_engine::AutoscaleInfo> {
    if value.is_null() || value.is_undefined() {
        return None;
    }
    let get = |object: &JsValue, key: &str| {
        js_sys::Reflect::get(object, &key.into())
            .ok()
            .filter(|value| !value.is_null() && !value.is_undefined())
    };
    let number = |object: &JsValue, key: &str| get(object, key).and_then(|value| value.as_f64());
    let price_range = get(value, "price_range")
        .and_then(|range| Some((number(&range, "min_value")?, number(&range, "max_value")?)));
    let margins = get(value, "margins")
        .and_then(|margins| Some((number(&margins, "above")?, number(&margins, "below")?)));
    Some(aeris_charts_engine::AutoscaleInfo {
        price_range,
        margins,
    })
}

/// Overlay a host's partial brush style on the engine default. An absent style or field keeps the
/// default; a present but invalid field rejects the whole brush update.
fn parse_area_brush_style(
    value: Option<&serde_json::Value>,
    base: BrushStyle,
) -> Option<BrushStyle> {
    let Some(value) = value.filter(|value| !value.is_null()) else {
        return Some(base);
    };
    let color = |key: &str, fallback: Color| match value.get(key) {
        None => Some(fallback),
        Some(field) => field.as_str().and_then(Color::parse_css),
    };
    Some(BrushStyle {
        line_color: color("line_color", base.line_color)?,
        top_color: color("top_color", base.top_color)?,
        bottom_color: color("bottom_color", base.bottom_color)?,
        line_width: match value.get("line_width") {
            None => base.line_width,
            Some(field) => field.as_f64()?,
        },
    })
}

/// 2024-01-01T00:00:00Z in Unix-ms — the reference year for locale month-name generation
/// (mid-month UTC instants, so no time zone can shift the month).
const LOCALE_MONTH_YEAR0_MS: f64 = 1_704_067_200_000.0;
const LOCALE_MONTH_DAY_OFFSETS: [i64; 12] = [0, 31, 60, 91, 121, 152, 182, 213, 244, 274, 305, 335];

/// Build a month-formatting `Intl.DateTimeFormat` for `locale` (`{month: short|long}`), or
/// `None` when the tag is rejected (the constructor throws for malformed BCP 47 tags; the
/// `Reflect::construct` boundary catches that into a `None`).
fn intl_month_format(locale: &str, month: &str) -> Option<js_sys::Intl::DateTimeFormat> {
    let intl = js_sys::Reflect::get(&js_sys::global(), &JsValue::from_str("Intl")).ok()?;
    let ctor = js_sys::Reflect::get(&intl, &JsValue::from_str("DateTimeFormat"))
        .ok()?
        .dyn_into::<js_sys::Function>()
        .ok()?;
    let options = js_sys::Object::new();
    js_sys::Reflect::set(
        &options,
        &JsValue::from_str("month"),
        &JsValue::from_str(month),
    )
    .ok()?;
    let locales = js_sys::Array::of1(&JsValue::from_str(locale));
    let args = js_sys::Array::of2(&locales, &options);
    js_sys::Reflect::construct(&ctor, &args)
        .ok()?
        .dyn_into::<js_sys::Intl::DateTimeFormat>()
        .ok()
}

/// The 12 short and 12 long month names for `locale` (reference `localization.locale`), generated
/// through `Intl.DateTimeFormat` exactly like the reference's `format-date.ts` `toLocaleString` calls.
fn locale_month_names(locale: &str) -> Option<([String; 12], [String; 12])> {
    let short_fmt = intl_month_format(locale, "short")?;
    let long_fmt = intl_month_format(locale, "long")?;
    // js-sys stable models `DateTimeFormat.prototype.format` as its getter: the returned
    // bound function formats one date per call.
    let short_fn = short_fmt.format();
    let long_fn = long_fmt.format();
    let mut short: [String; 12] = Default::default();
    let mut long: [String; 12] = Default::default();
    for (m, offset) in LOCALE_MONTH_DAY_OFFSETS.iter().enumerate() {
        let ms = LOCALE_MONTH_YEAR0_MS + (*offset as f64 + 14.0) * 86_400_000.0;
        let date = js_sys::Date::new(&JsValue::from_f64(ms));
        short[m] = short_fn
            .call1(&JsValue::UNDEFINED, &date)
            .ok()
            .and_then(|v| v.as_string())?;
        long[m] = long_fn
            .call1(&JsValue::UNDEFINED, &date)
            .ok()
            .and_then(|v| v.as_string())?;
    }
    Some((short, long))
}

/// Fire a detached primitive's `detached` hook (shared by the series-primitive detach paths);
/// a throwing hook is only reported.
fn fire_primitive_detached(obj: &js_sys::Object) {
    if let Ok(hook) = js_sys::Reflect::get(obj, &"detached".into()) {
        if let Ok(hook) = hook.dyn_into::<js_sys::Function>() {
            if let Err(error) = hook.call0(obj) {
                web_sys::console::warn_1(
                    &format!("aeris_charts: series primitive `detached` hook threw — {error:?}")
                        .into(),
                );
            }
        }
    }
}
