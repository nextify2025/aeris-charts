//! Boundary-driven volume, delta, TPO, and anchored-VWAP analytics.
//!
//! These snapshots are executor-neutral. Tape requests read the canonical classified trade stream;
//! candle requests are explicitly marked as approximations. Calendar/session policy is never
//! inferred: every periodic query receives host-supplied UTC boundaries.

use std::collections::{BTreeMap, VecDeque};

use aeris_charts_indicators::volume_profile::{
    volume_profile, volume_profile_developing, ProfileBar,
};
use aeris_charts_render::color::Color;
use serde::{Deserialize, Serialize};

use crate::{
    AggressorSide, ChartEngine, DrawingId, DrawingKind, NativePrimitiveId, ResampleBoundary,
    SeriesId, SeriesKind,
};

pub const MAX_PROFILE_PERIODS: usize = 5_000;
pub const MAX_PROFILE_ROWS: usize = 2_048;
pub const MAX_PROFILE_DEVELOPING_POINTS: usize = 100_000;
pub const MAX_PROFILE_TOTAL_ROWS: usize = 100_000;
pub const MAX_PROFILE_TOTAL_DEVELOPING_POINTS: usize = 100_000;
pub const MAX_PERIODIC_PROFILE_PRESENTATIONS: usize = 16;
pub const MAX_PERIODIC_PRESENTATION_ROWS: usize = 32_768;
pub const MAX_PERIODIC_DEVELOPING_POINTS: usize = 2_048;

#[derive(Clone, Copy)]
enum DevelopingMode {
    Omit,
    Full,
    Sampled(usize),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfileDisplayMode {
    #[default]
    BidAsk,
    Delta,
    Total,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PeriodicProfilePresentationRequest {
    pub source: ProfileSource,
    pub boundaries: Vec<ResampleBoundary>,
    pub tick_size: f64,
    pub row_count: usize,
    pub value_area_percent: f64,
}

impl PeriodicProfilePresentationRequest {
    fn reserved_rows(&self) -> usize {
        let per_period = match self.source {
            ProfileSource::Tape { .. } => MAX_PROFILE_ROWS,
            ProfileSource::Candles { .. } => self.row_count,
        };
        let groups = usize::from(!self.boundaries.is_empty())
            + self
                .boundaries
                .windows(2)
                .filter(|pair| pair[0].session_id != pair[1].session_id)
                .count();
        groups.saturating_mul(per_period)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PeriodicProfilePresentationOptions {
    pub mode: ProfileDisplayMode,
    pub width_percent: f64,
    pub bid_color: String,
    pub ask_color: String,
    pub unknown_color: String,
    pub poc_color: String,
    pub value_area_color: String,
    pub extend_naked_levels: bool,
    pub show_developing: bool,
}

impl Default for PeriodicProfilePresentationOptions {
    fn default() -> Self {
        Self {
            mode: ProfileDisplayMode::BidAsk,
            width_percent: 35.0,
            bid_color: "#f7525f".into(),
            ask_color: "#089981".into(),
            unknown_color: "#788291".into(),
            poc_color: "#f5a623".into(),
            value_area_color: "#335cff".into(),
            extend_naked_levels: false,
            show_developing: false,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct PeriodicProfilePresentationState {
    pub request: PeriodicProfilePresentationRequest,
    pub mode: ProfileDisplayMode,
    pub width_percent: f64,
    pub colors: [Color; 5],
    pub extend_naked_levels: bool,
    pub show_developing: bool,
}

impl PeriodicProfilePresentationOptions {
    fn colors(&self) -> Option<[Color; 5]> {
        if !self.width_percent.is_finite()
            || !(0.0 < self.width_percent && self.width_percent <= 100.0)
        {
            return None;
        }
        let colors = [
            &self.bid_color,
            &self.ask_color,
            &self.unknown_color,
            &self.poc_color,
            &self.value_area_color,
        ];
        if colors.iter().any(|color| color.len() > 32) {
            return None;
        }
        Some([
            Color::parse_css(colors[0])?,
            Color::parse_css(colors[1])?,
            Color::parse_css(colors[2])?,
            Color::parse_css(colors[3])?,
            Color::parse_css(colors[4])?,
        ])
    }
}
pub const MAX_TPO_PERIODS: usize = 1_024;
pub const MAX_TPO_TOTAL_ROWS: usize = 100_000;
pub const MAX_TPO_TOTAL_CELLS: usize = 100_000;
pub const MAX_TPO_PRESENTATIONS: usize = 16;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TpoCellMode {
    #[default]
    Letters,
    Blocks,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TpoPresentationOptions {
    pub mode: TpoCellMode,
    pub color: String,
    pub value_area_color: String,
    pub single_print_color: String,
    pub poc_color: String,
    pub initial_balance_color: String,
}

impl Default for TpoPresentationOptions {
    fn default() -> Self {
        Self {
            mode: TpoCellMode::Letters,
            color: "#8493a8".into(),
            value_area_color: "#335cff".into(),
            single_print_color: "#fb4b5f".into(),
            poc_color: "#f5a623".into(),
            initial_balance_color: "#7d52f4".into(),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct TpoPresentationState {
    pub request: TpoRequest,
    pub mode: TpoCellMode,
    pub colors: [Color; 5],
}

impl TpoPresentationOptions {
    fn colors(&self) -> Option<[Color; 5]> {
        let colors = [
            &self.color,
            &self.value_area_color,
            &self.single_print_color,
            &self.poc_color,
            &self.initial_balance_color,
        ];
        if colors.iter().any(|color| color.len() > 32) {
            return None;
        }
        Some([
            Color::parse_css(colors[0])?,
            Color::parse_css(colors[1])?,
            Color::parse_css(colors[2])?,
            Color::parse_css(colors[3])?,
            Color::parse_css(colors[4])?,
        ])
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum ProfileSource {
    Tape {
        stream_id: u64,
    },
    Candles {
        price_series: SeriesId,
        volume_series: SeriesId,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileRequest {
    pub source: ProfileSource,
    pub start_timestamp_micros: i64,
    pub end_timestamp_micros: i64,
    pub tick_size: f64,
    pub row_count: usize,
    pub value_area_percent: f64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileRowSnapshot {
    pub low: f64,
    pub high: f64,
    pub bid_volume: f64,
    pub ask_volume: f64,
    pub unknown_volume: f64,
    pub total_volume: f64,
    pub delta: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DevelopingValueArea {
    pub timestamp_micros: i64,
    pub poc: f64,
    pub value_area_low: f64,
    pub value_area_high: f64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileSnapshot {
    pub start_timestamp_micros: i64,
    pub end_timestamp_micros: i64,
    pub session_id: u64,
    pub tick_size: f64,
    pub rows: Vec<ProfileRowSnapshot>,
    pub total_volume: f64,
    pub poc: Option<f64>,
    pub value_area_low: Option<f64>,
    pub value_area_high: Option<f64>,
    pub developing: Vec<DevelopingValueArea>,
    pub candle_approximation: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NakedProfileLevel {
    pub session_id: u64,
    pub price: f64,
    pub kind: NakedProfileLevelKind,
    pub start_timestamp_micros: i64,
    pub touched_timestamp_micros: Option<i64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NakedProfileLevelKind {
    Poc,
    ValueAreaLow,
    ValueAreaHigh,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TpoRequest {
    pub price_series: SeriesId,
    pub boundaries: Vec<ResampleBoundary>,
    pub period_seconds: u32,
    pub tick_size: f64,
    pub value_area_percent: f64,
    pub initial_balance_periods: u16,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TpoRowSnapshot {
    pub price: f64,
    /// Zero-based period indices. Hosts map these to letters or block glyphs.
    pub periods: Vec<u16>,
    pub single_print: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TpoSnapshot {
    pub session_id: u64,
    pub start_timestamp_micros: i64,
    pub end_timestamp_micros: i64,
    pub rows: Vec<TpoRowSnapshot>,
    pub poc: Option<f64>,
    pub value_area_low: Option<f64>,
    pub value_area_high: Option<f64>,
    pub initial_balance_low: Option<f64>,
    pub initial_balance_high: Option<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnchoredVwapPoint {
    pub timestamp_micros: i64,
    pub vwap: f64,
    pub upper_band: f64,
    pub lower_band: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileDrawingOptions {
    pub source: ProfileSource,
    pub tick_size: f64,
    pub row_count: usize,
    pub value_area_percent: f64,
    pub band_multiplier: f64,
    pub width_percent: f64,
}

impl Default for ProfileDrawingOptions {
    fn default() -> Self {
        Self {
            source: ProfileSource::Candles {
                price_series: 0,
                volume_series: 0,
            },
            tick_size: 0.01,
            row_count: 48,
            value_area_percent: 70.0,
            band_multiplier: 1.0,
            width_percent: 30.0,
        }
    }
}

impl ProfileDrawingOptions {
    pub(crate) fn valid(&self) -> bool {
        valid_tick(self.tick_size)
            && (1..=MAX_PROFILE_ROWS).contains(&self.row_count)
            && (0.0 < self.value_area_percent && self.value_area_percent <= 100.0)
            && self.band_multiplier.is_finite()
            && self.band_multiplier >= 0.0
            && self.width_percent.is_finite()
            && (0.0 < self.width_percent && self.width_percent <= 100.0)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "data")]
pub enum ProfileDrawingSnapshot {
    Volume(ProfileSnapshot),
    Vwap(Vec<AnchoredVwapPoint>),
}

#[derive(Clone, Debug, PartialEq)]
pub enum ProfileError {
    InvalidRequest,
    UnknownSource,
    LimitExceeded,
}

impl std::fmt::Display for ProfileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::InvalidRequest => "invalid profile request",
            Self::UnknownSource => "profile source is unavailable",
            Self::LimitExceeded => "profile result limit exceeded",
        })
    }
}
impl std::error::Error for ProfileError {}

impl ChartEngine {
    pub fn add_periodic_profile_presentation(
        &mut self,
        anchor_series: SeriesId,
        request: PeriodicProfilePresentationRequest,
        options: PeriodicProfilePresentationOptions,
    ) -> Result<NativePrimitiveId, ProfileError> {
        match request.source {
            ProfileSource::Candles { price_series, .. } if price_series != anchor_series => {
                return Err(ProfileError::InvalidRequest);
            }
            _ => {}
        }
        if self.series_entry(anchor_series).is_none() {
            return Err(ProfileError::UnknownSource);
        }
        let colors = options.colors().ok_or(ProfileError::InvalidRequest)?;
        let (live, reserved_rows) = self
            .series
            .iter()
            .flat_map(|series| &series.native_primitives)
            .filter_map(|primitive| match &primitive.kind {
                crate::native_primitives::NativeSeriesPrimitiveKind::PeriodicProfile(state) => {
                    Some(state.request.reserved_rows())
                }
                _ => None,
            })
            .fold((0_usize, 0_usize), |(count, rows), capacity| {
                (count + 1, rows.saturating_add(capacity))
            });
        let requested_rows = request.reserved_rows();
        if live >= MAX_PERIODIC_PROFILE_PRESENTATIONS
            || requested_rows > MAX_PERIODIC_PRESENTATION_ROWS.saturating_sub(reserved_rows)
        {
            return Err(ProfileError::LimitExceeded);
        }
        if options.show_developing {
            self.periodic_volume_profiles_for_frame_developing(
                request.source,
                &request.boundaries,
                request.tick_size,
                request.row_count,
                request.value_area_percent,
            )?;
        } else {
            self.periodic_volume_profiles_for_frame(
                request.source,
                &request.boundaries,
                request.tick_size,
                request.row_count,
                request.value_area_percent,
            )?;
        }
        self.insert_native_primitive(
            anchor_series,
            crate::native_primitives::NativeSeriesPrimitiveKind::PeriodicProfile(Box::new(
                PeriodicProfilePresentationState {
                    request,
                    width_percent: options.width_percent,
                    colors,
                    mode: options.mode,
                    extend_naked_levels: options.extend_naked_levels,
                    show_developing: options.show_developing,
                },
            )),
        )
        .ok_or(ProfileError::LimitExceeded)
    }
    pub fn add_tpo_presentation(
        &mut self,
        request: TpoRequest,
        options: TpoPresentationOptions,
    ) -> Result<NativePrimitiveId, ProfileError> {
        if !matches!(
            self.series_entry(request.price_series)
                .map(|entry| entry.kind),
            Some(SeriesKind::Candlestick | SeriesKind::Bar)
        ) {
            return Err(ProfileError::UnknownSource);
        }
        let colors = options.colors().ok_or(ProfileError::InvalidRequest)?;
        let live = self
            .series
            .iter()
            .flat_map(|series| &series.native_primitives)
            .filter(|primitive| {
                matches!(
                    primitive.kind,
                    crate::native_primitives::NativeSeriesPrimitiveKind::TpoProfile(_)
                )
            })
            .count();
        if live >= MAX_TPO_PRESENTATIONS {
            return Err(ProfileError::LimitExceeded);
        }
        self.tpo_profiles(&request)?;
        let source = request.price_series;
        self.insert_native_primitive(
            source,
            crate::native_primitives::NativeSeriesPrimitiveKind::TpoProfile(Box::new(
                TpoPresentationState {
                    request,
                    mode: options.mode,
                    colors,
                },
            )),
        )
        .ok_or(ProfileError::LimitExceeded)
    }
    pub(crate) fn invalidate_profile_drawings_using_series(&mut self, series_id: SeriesId) {
        if self.drawings.iter().any(|drawing| {
            drawing.profile.as_ref().is_some_and(|profile| {
                matches!(profile.source, ProfileSource::Candles { price_series, volume_series }
                    if price_series == series_id || volume_series == series_id)
            })
        }) {
            self.invalidate_frame_drawings();
        }
        let anchors = self.series.iter().filter(|series| series.id != series_id)
            .filter(|series| series.native_primitives.iter().any(|primitive| {
                matches!(&primitive.kind,
                    crate::native_primitives::NativeSeriesPrimitiveKind::PeriodicProfile(state)
                    if matches!(state.request.source, ProfileSource::Candles { price_series, volume_series }
                        if price_series == series_id || volume_series == series_id))
            }))
            .map(|series| series.id)
            .collect::<Vec<_>>();
        for anchor in anchors {
            self.invalidate_frame_series(anchor);
        }
    }

    pub(crate) fn drop_periodic_profiles_using(&mut self, source_id: SeriesId) {
        let mut anchors = Vec::new();
        for series in &mut self.series {
            let before = series.native_primitives.len();
            series.native_primitives.retain(|primitive| {
                !matches!(&primitive.kind,
                    crate::native_primitives::NativeSeriesPrimitiveKind::PeriodicProfile(state)
                    if matches!(state.request.source, ProfileSource::Candles { price_series, volume_series }
                        if price_series == source_id || volume_series == source_id))
            });
            if series.native_primitives.len() != before {
                anchors.push(series.id);
            }
        }
        for anchor in anchors {
            self.invalidate_frame_series_geometry(anchor);
        }
    }

    pub(crate) fn profile_drawings_use_stream(&self, stream_id: u64) -> bool {
        self.drawings.iter().any(|drawing| {
            drawing.profile.as_ref().is_some_and(|profile| {
                matches!(profile.source, ProfileSource::Tape { stream_id: bound } if bound == stream_id)
            })
        }) || self.series.iter().any(|series| series.native_primitives.iter().any(|primitive| {
            matches!(&primitive.kind,
                crate::native_primitives::NativeSeriesPrimitiveKind::PeriodicProfile(state)
                if matches!(state.request.source, ProfileSource::Tape { stream_id: bound } if bound == stream_id))
        }))
    }

    pub(crate) fn invalidate_profile_drawings_using_stream(&mut self, stream_id: u64) {
        if self.drawings.iter().any(|drawing| {
            drawing.profile.as_ref().is_some_and(|profile| {
                matches!(profile.source, ProfileSource::Tape { stream_id: bound } if bound == stream_id)
            })
        }) {
            self.invalidate_frame_drawings();
        }
        let anchors = self.series.iter().filter(|series| series.native_primitives.iter().any(|primitive| {
            matches!(&primitive.kind,
                crate::native_primitives::NativeSeriesPrimitiveKind::PeriodicProfile(state)
                if matches!(state.request.source, ProfileSource::Tape { stream_id: bound } if bound == stream_id))
        }))
        .map(|series| series.id)
        .collect::<Vec<_>>();
        for anchor in anchors {
            self.invalidate_frame_series(anchor);
        }
    }

    pub fn configure_profile_drawing(
        &mut self,
        drawing_id: DrawingId,
        options: ProfileDrawingOptions,
    ) -> Result<(), ProfileError> {
        let drawing = self
            .drawing(drawing_id)
            .ok_or(ProfileError::UnknownSource)?;
        if !matches!(
            drawing.kind,
            DrawingKind::FixedRangeVolumeProfile
                | DrawingKind::AnchoredVolumeProfile
                | DrawingKind::AnchoredVwap
        ) || !options.valid()
        {
            return Err(ProfileError::InvalidRequest);
        }
        self.validate_profile_source(options.source)?;
        self.set_drawing_profile_options(drawing_id, options)
            .then_some(())
            .ok_or(ProfileError::UnknownSource)
    }

    pub fn profile_drawing_options(&self, drawing_id: DrawingId) -> Option<&ProfileDrawingOptions> {
        self.drawing(drawing_id)?.profile.as_ref()
    }

    pub fn profile_drawing_snapshot(
        &self,
        drawing_id: DrawingId,
    ) -> Result<ProfileDrawingSnapshot, ProfileError> {
        self.profile_drawing_snapshot_impl(drawing_id, true)
    }

    pub(crate) fn profile_drawing_snapshot_for_frame(
        &self,
        drawing_id: DrawingId,
    ) -> Result<ProfileDrawingSnapshot, ProfileError> {
        self.profile_drawing_snapshot_impl(drawing_id, false)
    }

    fn profile_drawing_snapshot_impl(
        &self,
        drawing_id: DrawingId,
        include_developing: bool,
    ) -> Result<ProfileDrawingSnapshot, ProfileError> {
        let drawing = self
            .drawing(drawing_id)
            .ok_or(ProfileError::UnknownSource)?;
        let options = drawing
            .profile
            .as_ref()
            .ok_or(ProfileError::UnknownSource)?;
        let first = drawing
            .points
            .first()
            .ok_or(ProfileError::InvalidRequest)?
            .logical;
        let last = match drawing.kind {
            DrawingKind::FixedRangeVolumeProfile => {
                drawing
                    .points
                    .get(1)
                    .ok_or(ProfileError::InvalidRequest)?
                    .logical
            }
            DrawingKind::AnchoredVolumeProfile | DrawingKind::AnchoredVwap => {
                self.data.merged_times().len().saturating_sub(1) as f64
            }
            _ => return Err(ProfileError::InvalidRequest),
        };
        let start = self
            .logical_timestamp_micros(first.min(last))
            .ok_or(ProfileError::InvalidRequest)?;
        let end = self
            .logical_timestamp_micros(first.max(last))
            .ok_or(ProfileError::InvalidRequest)?
            .saturating_add(1);
        if drawing.kind == DrawingKind::AnchoredVwap {
            return self
                .anchored_vwap(options.source, start, end, options.band_multiplier)
                .map(ProfileDrawingSnapshot::Vwap);
        }
        self.volume_profile_snapshot_impl(
            &ProfileRequest {
                source: options.source,
                start_timestamp_micros: start,
                end_timestamp_micros: end,
                tick_size: options.tick_size,
                row_count: options.row_count,
                value_area_percent: options.value_area_percent,
            },
            include_developing,
            None,
        )
        .map(ProfileDrawingSnapshot::Volume)
    }

    fn logical_timestamp_micros(&self, logical: f64) -> Option<i64> {
        if !logical.is_finite() || logical < 0.0 {
            return None;
        }
        let index = logical.round() as usize;
        if let Some(points) = self.sequence_points() {
            return points.get(index).map(|point| point.open_timestamp_micros);
        }
        self.data
            .merged_times()
            .get(index)
            .map(|time| time.saturating_mul(1_000_000))
    }

    fn validate_profile_source(&self, source: ProfileSource) -> Result<(), ProfileError> {
        match source {
            ProfileSource::Tape { stream_id } => self
                .trade_stream(stream_id)
                .map(|_| ())
                .ok_or(ProfileError::UnknownSource),
            ProfileSource::Candles {
                price_series,
                volume_series,
            } => {
                if matches!(
                    self.series_entry(price_series).map(|entry| entry.kind),
                    Some(SeriesKind::Candlestick | SeriesKind::Bar)
                ) && matches!(
                    self.series_entry(volume_series).map(|entry| entry.kind),
                    Some(SeriesKind::Histogram)
                ) && self.data.series_data(price_series).is_some()
                    && self.data.series_data(volume_series).is_some()
                {
                    Ok(())
                } else {
                    Err(ProfileError::UnknownSource)
                }
            }
        }
    }

    pub fn volume_profile_snapshot(
        &self,
        request: &ProfileRequest,
    ) -> Result<ProfileSnapshot, ProfileError> {
        self.volume_profile_snapshot_impl(request, true, None)
    }

    fn volume_profile_snapshot_impl(
        &self,
        request: &ProfileRequest,
        include_developing: bool,
        developing_sample_limit: Option<usize>,
    ) -> Result<ProfileSnapshot, ProfileError> {
        self.volume_profile_snapshot_segments_impl(
            request,
            None,
            include_developing,
            developing_sample_limit,
        )
    }

    fn volume_profile_snapshot_segments_impl(
        &self,
        request: &ProfileRequest,
        segments: Option<&[ResampleBoundary]>,
        include_developing: bool,
        developing_sample_limit: Option<usize>,
    ) -> Result<ProfileSnapshot, ProfileError> {
        validate_profile_request(request)?;
        self.validate_profile_source(request.source)?;
        match request.source {
            ProfileSource::Tape { stream_id } => self.tape_profile(
                stream_id,
                request,
                segments,
                include_developing,
                developing_sample_limit,
            ),
            ProfileSource::Candles {
                price_series,
                volume_series,
            } => self.candle_profile(
                price_series,
                volume_series,
                request,
                segments,
                include_developing,
                developing_sample_limit,
            ),
        }
    }

    pub fn periodic_volume_profiles(
        &self,
        source: ProfileSource,
        boundaries: &[ResampleBoundary],
        tick_size: f64,
        row_count: usize,
        value_area_percent: f64,
    ) -> Result<Vec<ProfileSnapshot>, ProfileError> {
        self.periodic_volume_profiles_impl(
            source,
            boundaries,
            tick_size,
            row_count,
            value_area_percent,
            DevelopingMode::Full,
        )
    }

    pub(crate) fn periodic_volume_profiles_for_frame(
        &self,
        source: ProfileSource,
        boundaries: &[ResampleBoundary],
        tick_size: f64,
        row_count: usize,
        value_area_percent: f64,
    ) -> Result<Vec<ProfileSnapshot>, ProfileError> {
        self.periodic_volume_profiles_impl(
            source,
            boundaries,
            tick_size,
            row_count,
            value_area_percent,
            DevelopingMode::Omit,
        )
    }

    pub(crate) fn periodic_volume_profiles_for_frame_developing(
        &self,
        source: ProfileSource,
        boundaries: &[ResampleBoundary],
        tick_size: f64,
        row_count: usize,
        value_area_percent: f64,
    ) -> Result<Vec<ProfileSnapshot>, ProfileError> {
        self.periodic_volume_profiles_impl(
            source,
            boundaries,
            tick_size,
            row_count,
            value_area_percent,
            DevelopingMode::Sampled(MAX_PERIODIC_DEVELOPING_POINTS),
        )
    }

    fn periodic_volume_profiles_impl(
        &self,
        source: ProfileSource,
        boundaries: &[ResampleBoundary],
        tick_size: f64,
        row_count: usize,
        value_area_percent: f64,
        developing_mode: DevelopingMode,
    ) -> Result<Vec<ProfileSnapshot>, ProfileError> {
        if boundaries.is_empty()
            || boundaries.len() > MAX_PROFILE_PERIODS
            || boundaries.iter().any(|b| {
                b.start_time >= b.end_time
                    || b.start_time.checked_mul(1_000_000).is_none()
                    || b.end_time.checked_mul(1_000_000).is_none()
            })
            || boundaries
                .windows(2)
                .any(|pair| pair[0].end_time > pair[1].start_time)
        {
            return Err(ProfileError::InvalidRequest);
        }
        let developing_total_limit = match developing_mode {
            DevelopingMode::Sampled(limit) => Some(limit),
            _ => None,
        };
        let include_developing = !matches!(developing_mode, DevelopingMode::Omit);
        let group_count = 1 + boundaries
            .windows(2)
            .filter(|pair| pair[0].session_id != pair[1].session_id)
            .count();
        let developing_sample_limit = developing_total_limit.map(|limit| limit / group_count);
        if developing_sample_limit == Some(0) {
            return Err(ProfileError::LimitExceeded);
        }
        let mut profiles = Vec::with_capacity(boundaries.len());
        let mut total_rows = 0usize;
        let mut total_developing = 0usize;
        let mut group_start = 0;
        while group_start < boundaries.len() {
            let mut group_end = group_start + 1;
            while group_end < boundaries.len()
                && boundaries[group_end].session_id == boundaries[group_start].session_id
            {
                group_end += 1;
            }
            let segments = &boundaries[group_start..group_end];
            let mut profile = self.volume_profile_snapshot_segments_impl(
                &ProfileRequest {
                    source,
                    start_timestamp_micros: segments[0].start_time * 1_000_000,
                    end_timestamp_micros: segments[segments.len() - 1].end_time * 1_000_000,
                    tick_size,
                    row_count,
                    value_area_percent,
                },
                Some(segments),
                include_developing,
                developing_sample_limit,
            )?;
            total_rows += profile.rows.len();
            total_developing += profile.developing.len();
            if total_rows > MAX_PROFILE_TOTAL_ROWS
                || total_developing > MAX_PROFILE_TOTAL_DEVELOPING_POINTS
                || developing_total_limit.is_some_and(|limit| total_developing > limit)
            {
                return Err(ProfileError::LimitExceeded);
            }
            profile.session_id = segments[0].session_id;
            profiles.push(profile);
            group_start = group_end;
        }
        Ok(profiles)
    }

    pub fn naked_profile_levels(
        &self,
        source: ProfileSource,
        profiles: &[ProfileSnapshot],
    ) -> Result<Vec<NakedProfileLevel>, ProfileError> {
        if profiles.len() > MAX_PROFILE_PERIODS
            || profiles.iter().any(|profile| {
                !valid_tick(profile.tick_size)
                    || [profile.poc, profile.value_area_low, profile.value_area_high]
                        .into_iter()
                        .flatten()
                        .any(|price| !price.is_finite())
            })
        {
            return Err(ProfileError::InvalidRequest);
        }
        let mut levels = Vec::with_capacity(profiles.len().saturating_mul(3));
        let mut tape_levels: BTreeMap<u64, BTreeMap<i64, Vec<usize>>> = BTreeMap::new();
        let mut candle_levels = Vec::new();
        for profile in profiles {
            for (kind, price) in [
                (NakedProfileLevelKind::Poc, profile.poc),
                (NakedProfileLevelKind::ValueAreaLow, profile.value_area_low),
                (
                    NakedProfileLevelKind::ValueAreaHigh,
                    profile.value_area_high,
                ),
            ] {
                if let Some(price) = price {
                    let index = levels.len();
                    levels.push(NakedProfileLevel {
                        session_id: profile.session_id,
                        price,
                        kind,
                        start_timestamp_micros: profile.end_timestamp_micros,
                        touched_timestamp_micros: None,
                    });
                    if matches!(source, ProfileSource::Tape { .. }) {
                        tape_levels
                            .entry(profile.tick_size.to_bits())
                            .or_default()
                            .entry(price_level(price, profile.tick_size))
                            .or_default()
                            .push(index);
                    } else {
                        candle_levels.push(index);
                    }
                }
            }
        }
        if let ProfileSource::Tape { stream_id } = source {
            let stream = self
                .trade_stream(stream_id)
                .ok_or(ProfileError::UnknownSource)?;
            for (tick_bits, by_price) in tape_levels {
                let tick_size = f64::from_bits(tick_bits);
                let mut waiting = by_price
                    .into_iter()
                    .map(|(level, mut indices)| {
                        indices.sort_unstable_by_key(|&index| levels[index].start_timestamp_micros);
                        (level, VecDeque::from(indices))
                    })
                    .collect::<BTreeMap<_, _>>();
                for (trade, _) in stream.classified_trades() {
                    let level = price_level(trade.price, tick_size);
                    if let Some(indices) = waiting.get_mut(&level) {
                        while indices.front().is_some_and(|&index| {
                            levels[index].start_timestamp_micros <= trade.timestamp_micros
                        }) {
                            let index = indices.pop_front().expect("front was present");
                            levels[index].touched_timestamp_micros = Some(trade.timestamp_micros);
                        }
                    }
                }
            }
        } else if let ProfileSource::Candles { price_series, .. } = source {
            let (times, values) = self
                .data
                .series_data(price_series)
                .ok_or(ProfileError::UnknownSource)?;
            candle_levels.sort_unstable_by_key(|&index| levels[index].start_timestamp_micros);
            let mut next = 0;
            let mut waiting: BTreeMap<u64, Vec<usize>> = BTreeMap::new();
            for (row, &time) in times.iter().enumerate() {
                let timestamp_micros = time.saturating_mul(1_000_000);
                while next < candle_levels.len()
                    && levels[candle_levels[next]].start_timestamp_micros <= timestamp_micros
                {
                    let index = candle_levels[next];
                    waiting
                        .entry(ordered_price_key(levels[index].price))
                        .or_default()
                        .push(index);
                    next += 1;
                }
                let low = values[2][row];
                let high = values[1][row];
                if !low.is_finite() || !high.is_finite() || low > high {
                    continue;
                }
                let touched = waiting
                    .range(ordered_price_key(low)..=ordered_price_key(high))
                    .map(|(&key, _)| key)
                    .collect::<Vec<_>>();
                for key in touched {
                    if let Some(indices) = waiting.remove(&key) {
                        for index in indices {
                            levels[index].touched_timestamp_micros = Some(timestamp_micros);
                        }
                    }
                }
            }
        }
        Ok(levels)
    }

    /// Resolve host periods and their untouched levels as one engine operation.
    pub fn periodic_naked_profile_levels(
        &self,
        source: ProfileSource,
        boundaries: &[ResampleBoundary],
        tick_size: f64,
        row_count: usize,
        value_area_percent: f64,
    ) -> Result<Vec<NakedProfileLevel>, ProfileError> {
        let profiles = self.periodic_volume_profiles_for_frame(
            source,
            boundaries,
            tick_size,
            row_count,
            value_area_percent,
        )?;
        self.naked_profile_levels(source, &profiles)
    }

    pub fn tpo_profiles(&self, request: &TpoRequest) -> Result<Vec<TpoSnapshot>, ProfileError> {
        if request.boundaries.is_empty()
            || request.boundaries.len() > MAX_PROFILE_PERIODS
            || request.boundaries.iter().any(|boundary| {
                boundary.start_time >= boundary.end_time
                    || boundary.start_time.checked_mul(1_000_000).is_none()
                    || boundary.end_time.checked_mul(1_000_000).is_none()
            })
            || request
                .boundaries
                .windows(2)
                .any(|pair| pair[0].end_time > pair[1].start_time)
            || request.period_seconds == 0
            || !valid_tick(request.tick_size)
            || !(0.0 < request.value_area_percent && request.value_area_percent <= 100.0)
            || request.initial_balance_periods == 0
        {
            return Err(ProfileError::InvalidRequest);
        }
        let (times, values) = self
            .data
            .series_data(request.price_series)
            .ok_or(ProfileError::UnknownSource)?;
        let period_micros = i128::from(request.period_seconds) * 1_000_000;
        let mut output = Vec::with_capacity(request.boundaries.len());
        let mut group_start = 0;
        let mut total_rows = 0usize;
        let mut total_cells = 0usize;
        while group_start < request.boundaries.len() {
            let session_id = request.boundaries[group_start].session_id;
            let group_end = group_start
                + request.boundaries[group_start..]
                    .iter()
                    .take_while(|boundary| boundary.session_id == session_id)
                    .count();
            let mut rows: BTreeMap<i64, Vec<u16>> = BTreeMap::new();
            let mut ib_low = f64::INFINITY;
            let mut ib_high = f64::NEG_INFINITY;
            let mut period_offset = 0usize;
            for boundary in &request.boundaries[group_start..group_end] {
                let start = boundary.start_time * 1_000_000;
                let end = boundary.end_time * 1_000_000;
                let period_count = usize::try_from(
                    (i128::from(end) - i128::from(start) + period_micros - 1) / period_micros,
                )
                .map_err(|_| ProfileError::LimitExceeded)?;
                if period_count > MAX_TPO_PERIODS.saturating_sub(period_offset) {
                    return Err(ProfileError::LimitExceeded);
                }
                let first = times.partition_point(|&time| time < boundary.start_time);
                let finish = times.partition_point(|&time| time < boundary.end_time);
                for row in first..finish {
                    let time = times[row] * 1_000_000;
                    let period = period_offset
                        + ((i128::from(time) - i128::from(start)) / period_micros) as usize;
                    if period < usize::from(request.initial_balance_periods) {
                        ib_low = ib_low.min(values[2][row]);
                        ib_high = ib_high.max(values[1][row]);
                    }
                    let low = price_level(values[2][row], request.tick_size);
                    let high = price_level(values[1][row], request.tick_size);
                    if high.saturating_sub(low) as usize > MAX_PROFILE_ROWS {
                        return Err(ProfileError::LimitExceeded);
                    }
                    for level in low..=high {
                        let periods = rows.entry(level).or_default();
                        let period = period as u16;
                        if periods.last() != Some(&period) {
                            if total_cells == MAX_TPO_TOTAL_CELLS {
                                return Err(ProfileError::LimitExceeded);
                            }
                            periods.push(period);
                            total_cells += 1;
                        }
                    }
                    if rows.len() > MAX_PROFILE_ROWS {
                        return Err(ProfileError::LimitExceeded);
                    }
                }
                period_offset += period_count;
            }
            if rows.len() > MAX_PROFILE_ROWS {
                return Err(ProfileError::LimitExceeded);
            }
            total_rows += rows.len();
            if total_rows > MAX_TPO_TOTAL_ROWS {
                return Err(ProfileError::LimitExceeded);
            }
            let counts = rows
                .iter()
                .map(|(&level, periods)| (level, periods.len() as f64))
                .collect::<Vec<_>>();
            let (poc, val, vah) =
                value_area_levels(&counts, request.value_area_percent, request.tick_size);
            output.push(TpoSnapshot {
                session_id,
                start_timestamp_micros: request.boundaries[group_start].start_time * 1_000_000,
                end_timestamp_micros: request.boundaries[group_end - 1].end_time * 1_000_000,
                rows: rows
                    .into_iter()
                    .map(|(level, periods)| TpoRowSnapshot {
                        price: level as f64 * request.tick_size,
                        single_print: periods.len() == 1,
                        periods,
                    })
                    .collect(),
                poc,
                value_area_low: val,
                value_area_high: vah,
                initial_balance_low: ib_low.is_finite().then_some(ib_low),
                initial_balance_high: ib_high.is_finite().then_some(ib_high),
            });
            group_start = group_end;
        }
        Ok(output)
    }

    pub fn anchored_vwap(
        &self,
        source: ProfileSource,
        start_timestamp_micros: i64,
        end_timestamp_micros: i64,
        band_multiplier: f64,
    ) -> Result<Vec<AnchoredVwapPoint>, ProfileError> {
        if start_timestamp_micros >= end_timestamp_micros
            || !band_multiplier.is_finite()
            || band_multiplier < 0.0
        {
            return Err(ProfileError::InvalidRequest);
        }
        self.validate_profile_source(source)?;
        let mut weighted = 0.0;
        let mut weight = 0.0;
        let mut weighted_square = 0.0;
        let mut output = Vec::new();
        let mut push = |timestamp_micros: i64, price: f64, volume: f64| {
            if price.is_finite() && volume.is_finite() && volume > 0.0 {
                weighted += price * volume;
                weighted_square += price * price * volume;
                weight += volume;
                let vwap = weighted / weight;
                let deviation = (weighted_square / weight - vwap * vwap).max(0.0).sqrt();
                output.push(AnchoredVwapPoint {
                    timestamp_micros,
                    vwap,
                    upper_band: vwap + deviation * band_multiplier,
                    lower_band: vwap - deviation * band_multiplier,
                });
            }
        };
        match source {
            ProfileSource::Tape { stream_id } => {
                for (trade, _) in self
                    .trade_stream(stream_id)
                    .ok_or(ProfileError::UnknownSource)?
                    .classified_trades()
                    .filter(|(trade, _)| {
                        trade.timestamp_micros >= start_timestamp_micros
                            && trade.timestamp_micros < end_timestamp_micros
                    })
                {
                    push(trade.timestamp_micros, trade.price, trade.volume);
                }
            }
            ProfileSource::Candles {
                price_series,
                volume_series,
            } => {
                let (times, prices) = self
                    .data
                    .series_data(price_series)
                    .ok_or(ProfileError::UnknownSource)?;
                let (volume_times, volumes) = self
                    .data
                    .series_data(volume_series)
                    .ok_or(ProfileError::UnknownSource)?;
                for row in times.partition_point(|&time| {
                    time.saturating_mul(1_000_000) < start_timestamp_micros
                })
                    ..times.partition_point(|&time| {
                        time.saturating_mul(1_000_000) < end_timestamp_micros
                    })
                {
                    if let Ok(volume_row) = volume_times.binary_search(&times[row]) {
                        push(
                            times[row].saturating_mul(1_000_000),
                            (prices[1][row] + prices[2][row] + prices[3][row]) / 3.0,
                            volumes[3][volume_row],
                        );
                    }
                }
            }
        }
        Ok(output)
    }

    fn tape_profile(
        &self,
        stream_id: u64,
        request: &ProfileRequest,
        segments: Option<&[ResampleBoundary]>,
        include_developing: bool,
        developing_sample_limit: Option<usize>,
    ) -> Result<ProfileSnapshot, ProfileError> {
        let stream = self
            .trade_stream(stream_id)
            .ok_or(ProfileError::UnknownSource)?;
        let mut rows: BTreeMap<i64, ProfileRowSnapshot> = BTreeMap::new();
        let mut developing = Vec::new();
        let in_period = |timestamp: i64| {
            timestamp >= request.start_timestamp_micros
                && timestamp < request.end_timestamp_micros
                && timestamp_in_profile_segments(timestamp, segments)
        };
        let trade_count = if developing_sample_limit.is_some() {
            stream
                .classified_trades()
                .filter(|(trade, _)| in_period(trade.timestamp_micros))
                .count()
        } else {
            0
        };
        let stride = developing_sample_limit.map(|limit| {
            if limit < 2 || trade_count < 2 {
                usize::MAX
            } else {
                (trade_count - 1).div_ceil(limit - 1)
            }
        });
        for (trade_index, (trade, side)) in stream
            .classified_trades()
            .filter(|(trade, _)| in_period(trade.timestamp_micros))
            .enumerate()
        {
            let level = price_level(trade.price, request.tick_size);
            let row = rows.entry(level).or_insert_with(|| ProfileRowSnapshot {
                low: level as f64 * request.tick_size,
                high: (level + 1) as f64 * request.tick_size,
                ..Default::default()
            });
            match side {
                AggressorSide::Buy => row.ask_volume += trade.volume,
                AggressorSide::Sell => row.bid_volume += trade.volume,
                AggressorSide::Unknown => row.unknown_volume += trade.volume,
            }
            row.total_volume += trade.volume;
            row.delta = row.ask_volume - row.bid_volume;
            if rows.len() > MAX_PROFILE_ROWS {
                return Err(ProfileError::LimitExceeded);
            }
            let capture_developing = include_developing
                && stride.is_none_or(|stride| {
                    trade_index + 1 == trade_count
                        || (stride != usize::MAX && trade_index % stride == 0)
                });
            if capture_developing {
                if developing.len() >= MAX_PROFILE_DEVELOPING_POINTS {
                    return Err(ProfileError::LimitExceeded);
                }
                let counts = rows
                    .iter()
                    .map(|(&level, row)| (level, row.total_volume))
                    .collect::<Vec<_>>();
                let (poc, val, vah) =
                    value_area_levels(&counts, request.value_area_percent, request.tick_size);
                if let (Some(poc), Some(value_area_low), Some(value_area_high)) = (poc, val, vah) {
                    developing.push(DevelopingValueArea {
                        timestamp_micros: trade.timestamp_micros,
                        poc,
                        value_area_low,
                        value_area_high,
                    });
                }
            }
        }
        let counts = rows
            .iter()
            .map(|(&level, row)| (level, row.total_volume))
            .collect::<Vec<_>>();
        let (poc, value_area_low, value_area_high) =
            value_area_levels(&counts, request.value_area_percent, request.tick_size);
        Ok(ProfileSnapshot {
            start_timestamp_micros: request.start_timestamp_micros,
            end_timestamp_micros: request.end_timestamp_micros,
            tick_size: request.tick_size,
            rows: rows.into_values().collect(),
            total_volume: counts.iter().map(|(_, volume)| volume).sum(),
            poc,
            value_area_low,
            value_area_high,
            developing,
            candle_approximation: false,
            ..Default::default()
        })
    }

    fn candle_profile(
        &self,
        price_series: SeriesId,
        volume_series: SeriesId,
        request: &ProfileRequest,
        segments: Option<&[ResampleBoundary]>,
        include_developing: bool,
        developing_sample_limit: Option<usize>,
    ) -> Result<ProfileSnapshot, ProfileError> {
        let (times, prices) = self
            .data
            .series_data(price_series)
            .ok_or(ProfileError::UnknownSource)?;
        let (volume_times, volumes) = self
            .data
            .series_data(volume_series)
            .ok_or(ProfileError::UnknownSource)?;
        let first = times.partition_point(|&time| {
            time.saturating_mul(1_000_000) < request.start_timestamp_micros
        });
        let finish = times
            .partition_point(|&time| time.saturating_mul(1_000_000) < request.end_timestamp_micros);
        let bars = (first..finish)
            .filter_map(|row| {
                if !timestamp_in_profile_segments(times[row].saturating_mul(1_000_000), segments) {
                    return None;
                }
                volume_times
                    .binary_search(&times[row])
                    .ok()
                    .map(|volume_row| {
                        (
                            times[row] * 1_000_000,
                            ProfileBar {
                                open: prices[0][row],
                                high: prices[1][row],
                                low: prices[2][row],
                                close: prices[3][row],
                                volume: volumes[3][volume_row],
                            },
                        )
                    })
            })
            .collect::<Vec<_>>();
        let profile = volume_profile(
            bars.iter().map(|(_, bar)| *bar),
            request.row_count.min(MAX_PROFILE_ROWS),
            request.value_area_percent,
            request.tick_size,
        )
        .map_err(|_| ProfileError::InvalidRequest)?;
        let rows = profile
            .rows
            .iter()
            .map(|row| ProfileRowSnapshot {
                low: row.low,
                high: row.high,
                bid_volume: row.down_volume,
                ask_volume: row.up_volume,
                unknown_volume: 0.0,
                total_volume: row.volume,
                delta: row.up_volume - row.down_volume,
            })
            .collect::<Vec<_>>();
        let center = |index: Option<usize>| {
            index
                .and_then(|index| rows.get(index))
                .map(|row| (row.low + row.high) * 0.5)
        };
        let poc = center(profile.poc_index);
        let value_area_low = center(profile.value_area_low_index);
        let value_area_high = center(profile.value_area_high_index);
        let developing = if include_developing {
            let sample_limit = developing_sample_limit.unwrap_or(MAX_PROFILE_DEVELOPING_POINTS);
            if developing_sample_limit.is_none() && bars.len() > MAX_PROFILE_DEVELOPING_POINTS {
                return Err(ProfileError::LimitExceeded);
            }
            volume_profile_developing(
                &bars,
                &profile.rows,
                request.value_area_percent,
                sample_limit,
            )
            .map_err(|_| ProfileError::InvalidRequest)?
            .into_iter()
            .map(|point| DevelopingValueArea {
                timestamp_micros: point.timestamp_micros,
                poc: center(Some(point.poc_index)).unwrap_or_default(),
                value_area_low: center(Some(point.value_area_low_index)).unwrap_or_default(),
                value_area_high: center(Some(point.value_area_high_index)).unwrap_or_default(),
            })
            .collect()
        } else {
            Vec::new()
        };
        Ok(ProfileSnapshot {
            start_timestamp_micros: request.start_timestamp_micros,
            end_timestamp_micros: request.end_timestamp_micros,
            tick_size: request.tick_size,
            rows,
            total_volume: profile.total_volume,
            poc,
            value_area_low,
            value_area_high,
            developing,
            candle_approximation: true,
            ..Default::default()
        })
    }
}

fn ordered_price_key(price: f64) -> u64 {
    let bits = if price == 0.0 {
        0.0_f64.to_bits()
    } else {
        price.to_bits()
    };
    if bits >> 63 == 0 {
        bits ^ (1_u64 << 63)
    } else {
        !bits
    }
}

fn timestamp_in_profile_segments(
    timestamp_micros: i64,
    segments: Option<&[ResampleBoundary]>,
) -> bool {
    segments.is_none_or(|segments| {
        let index = segments.partition_point(|boundary| {
            boundary.end_time.saturating_mul(1_000_000) <= timestamp_micros
        });
        segments.get(index).is_some_and(|boundary| {
            boundary.start_time.saturating_mul(1_000_000) <= timestamp_micros
        })
    })
}

fn validate_profile_request(request: &ProfileRequest) -> Result<(), ProfileError> {
    if request.start_timestamp_micros >= request.end_timestamp_micros
        || !valid_tick(request.tick_size)
        || !(1..=MAX_PROFILE_ROWS).contains(&request.row_count)
        || !request.value_area_percent.is_finite()
        || !(0.0 < request.value_area_percent && request.value_area_percent <= 100.0)
    {
        Err(ProfileError::InvalidRequest)
    } else {
        Ok(())
    }
}

fn valid_tick(tick: f64) -> bool {
    tick.is_finite() && tick > 0.0
}
fn price_level(price: f64, tick: f64) -> i64 {
    (price / tick).round() as i64
}

fn value_area_levels(
    rows: &[(i64, f64)],
    percent: f64,
    tick: f64,
) -> (Option<f64>, Option<f64>, Option<f64>) {
    if rows.is_empty() {
        return (None, None, None);
    }
    let mut poc = 0;
    for index in 1..rows.len() {
        if rows[index].1 > rows[poc].1 {
            poc = index;
        }
    }
    let target = rows.iter().map(|row| row.1).sum::<f64>() * percent / 100.0;
    let (mut low, mut high, mut included) = (poc, poc, rows[poc].1);
    while included < target && (low > 0 || high + 1 < rows.len()) {
        if high + 1 == rows.len() || (low > 0 && rows[low - 1].1 >= rows[high + 1].1) {
            low -= 1;
            included += rows[low].1;
        } else {
            high += 1;
            included += rows[high].1;
        }
    }
    (
        Some(rows[poc].0 as f64 * tick),
        Some(rows[low].0 as f64 * tick),
        Some(rows[high].0 as f64 * tick),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FootprintAggregationOptions, FootprintTrade, SeriesKind};

    #[test]
    fn candle_profile_requires_ohlc_and_histogram_sources() {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        let line = chart.add_series(SeriesKind::Line);
        let candles = chart.add_series(SeriesKind::Candlestick);
        let request = |price_series, volume_series| ProfileRequest {
            source: ProfileSource::Candles {
                price_series,
                volume_series,
            },
            start_timestamp_micros: 0,
            end_timestamp_micros: 1_000_000,
            tick_size: 1.0,
            row_count: 8,
            value_area_percent: 70.0,
        };
        assert_eq!(
            chart.volume_profile_snapshot(&request(line, candles)),
            Err(ProfileError::UnknownSource)
        );
        assert_eq!(
            chart.volume_profile_snapshot(&request(candles, line)),
            Err(ProfileError::UnknownSource)
        );
    }

    #[test]
    fn periodic_tape_composite_excludes_gap_trades_and_splits_distinct_sessions() {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        let stream_id = chart
            .add_trade_stream(
                "composite-tape",
                FootprintAggregationOptions {
                    tick_size: 1.0,
                    ..Default::default()
                },
            )
            .unwrap();
        let trade = |timestamp_micros, price, volume| FootprintTrade {
            timestamp_micros,
            price,
            volume,
            aggressor: AggressorSide::Buy,
            bid: None,
            ask: None,
            sequence: None,
            trade_id: None,
            conditions: 0,
            session_id: Some(1),
        };
        chart
            .set_trade_stream_trades(
                stream_id,
                vec![
                    trade(0, 100.0, 1.0),
                    trade(60_000_000, 900.0, 50.0),
                    trade(120_000_000, 101.0, 2.0),
                ],
            )
            .unwrap();
        let mut boundaries = vec![
            ResampleBoundary {
                start_time: 0,
                end_time: 30,
                session_id: 7,
            },
            ResampleBoundary {
                start_time: 120,
                end_time: 150,
                session_id: 7,
            },
        ];
        let merged = chart
            .periodic_volume_profiles(ProfileSource::Tape { stream_id }, &boundaries, 1.0, 8, 70.0)
            .unwrap();
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].total_volume, 3.0);
        assert_eq!(merged[0].rows.len(), 2);
        assert_eq!(merged[0].developing.len(), 2);
        boundaries[1].session_id = 8;
        assert_eq!(
            chart
                .periodic_volume_profiles(
                    ProfileSource::Tape { stream_id },
                    &boundaries,
                    1.0,
                    8,
                    70.0
                )
                .unwrap()
                .len(),
            2
        );
    }

    #[test]
    fn periodic_candle_composite_excludes_gap_bars() {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        let price_series = chart.add_series(SeriesKind::Candlestick);
        let volume_series = chart.add_series(SeriesKind::Histogram);
        let times = [0.0, 60.0, 120.0];
        chart
            .set_series_data(
                price_series,
                &times,
                &[100.0, 900.0, 101.0],
                &[100.0, 900.0, 101.0],
                &[100.0, 900.0, 101.0],
                &[100.0, 900.0, 101.0],
            )
            .unwrap();
        chart
            .set_series_data(
                volume_series,
                &times,
                &[1.0, 50.0, 2.0],
                &[1.0, 50.0, 2.0],
                &[1.0, 50.0, 2.0],
                &[1.0, 50.0, 2.0],
            )
            .unwrap();
        let result = chart
            .periodic_volume_profiles(
                ProfileSource::Candles {
                    price_series,
                    volume_series,
                },
                &[
                    ResampleBoundary {
                        start_time: 0,
                        end_time: 30,
                        session_id: 1,
                    },
                    ResampleBoundary {
                        start_time: 120,
                        end_time: 150,
                        session_id: 1,
                    },
                ],
                1.0,
                8,
                70.0,
            )
            .unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].total_volume, 3.0);
        assert_eq!(result[0].developing.len(), 2);
        assert!(result[0].rows.iter().all(|row| row.high < 200.0));
    }

    #[test]
    fn naked_tape_level_ends_on_a_trade_at_the_same_tick() {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        let stream_id = chart
            .add_trade_stream(
                "fixture",
                FootprintAggregationOptions {
                    tick_size: 0.1,
                    ..Default::default()
                },
            )
            .unwrap();
        let trade = |timestamp_micros, price, volume| FootprintTrade {
            timestamp_micros,
            price,
            volume,
            aggressor: AggressorSide::Buy,
            bid: None,
            ask: None,
            sequence: None,
            trade_id: None,
            conditions: 0,
            session_id: Some(1),
        };
        chart
            .set_trade_stream_trades(
                stream_id,
                vec![
                    trade(0, 100.3, 5.0),
                    trade(1_000_000, 100.4, 2.0),
                    trade(2_000_000, 100.3, 1.0),
                ],
            )
            .unwrap();
        let source = ProfileSource::Tape { stream_id };
        let profile = chart
            .volume_profile_snapshot(&ProfileRequest {
                source,
                start_timestamp_micros: 0,
                end_timestamp_micros: 2_000_000,
                tick_size: 0.1,
                row_count: 16,
                value_area_percent: 70.0,
            })
            .unwrap();
        let levels = chart.naked_profile_levels(source, &[profile]).unwrap();
        let poc = levels
            .iter()
            .find(|level| level.kind == NakedProfileLevelKind::Poc)
            .unwrap();
        assert_eq!(poc.touched_timestamp_micros, Some(2_000_000));

        let request = ProfileRequest {
            source,
            start_timestamp_micros: 0,
            end_timestamp_micros: 2_000_000,
            tick_size: 0.1,
            row_count: 16,
            value_area_percent: 70.0,
        };
        let full = chart.volume_profile_snapshot(&request).unwrap();
        let frame = chart
            .volume_profile_snapshot_impl(&request, false, None)
            .unwrap();
        assert_eq!(frame.rows, full.rows);
        assert_eq!(
            (frame.poc, frame.value_area_low, frame.value_area_high),
            (full.poc, full.value_area_low, full.value_area_high)
        );
        assert!(frame.developing.is_empty());
        assert_eq!(full.developing.len(), 2);
    }

    #[test]
    fn naked_tape_sweep_resolves_each_period_after_its_own_end() {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        let stream_id = chart
            .add_trade_stream(
                "naked-periods",
                FootprintAggregationOptions {
                    tick_size: 1.0,
                    ..Default::default()
                },
            )
            .unwrap();
        let trade = |timestamp_micros, price| FootprintTrade {
            timestamp_micros,
            price,
            volume: 1.0,
            aggressor: AggressorSide::Buy,
            bid: None,
            ask: None,
            sequence: None,
            trade_id: None,
            conditions: 0,
            session_id: Some(1),
        };
        chart
            .set_trade_stream_trades(
                stream_id,
                vec![trade(10, 100.0), trade(20, 100.0), trade(30, 101.0)],
            )
            .unwrap();
        let profiles = [
            ProfileSnapshot {
                session_id: 1,
                end_timestamp_micros: 5,
                tick_size: 1.0,
                poc: Some(100.0),
                ..Default::default()
            },
            ProfileSnapshot {
                session_id: 2,
                end_timestamp_micros: 15,
                tick_size: 1.0,
                poc: Some(100.0),
                ..Default::default()
            },
            ProfileSnapshot {
                session_id: 3,
                end_timestamp_micros: 25,
                tick_size: 1.0,
                poc: Some(100.0),
                ..Default::default()
            },
        ];
        let levels = chart
            .naked_profile_levels(ProfileSource::Tape { stream_id }, &profiles)
            .unwrap();
        assert_eq!(
            levels
                .iter()
                .map(|level| level.touched_timestamp_micros)
                .collect::<Vec<_>>(),
            vec![Some(10), Some(20), None]
        );
    }

    #[test]
    fn naked_candle_sweep_uses_exact_price_and_each_period_end() {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        let price_series = chart.add_series(SeriesKind::Candlestick);
        let volume_series = chart.add_series(SeriesKind::Histogram);
        chart
            .set_series_data(
                price_series,
                &[0.0, 1.0, 2.0],
                &[100.0; 3],
                &[100.1, 100.1, 100.3],
                &[99.9; 3],
                &[100.0; 3],
            )
            .unwrap();
        let profiles = [
            ProfileSnapshot {
                session_id: 1,
                end_timestamp_micros: 500_000,
                tick_size: 0.1,
                poc: Some(100.05),
                value_area_high: Some(100.25),
                ..Default::default()
            },
            ProfileSnapshot {
                session_id: 2,
                end_timestamp_micros: 1_500_000,
                tick_size: 0.1,
                poc: Some(100.05),
                ..Default::default()
            },
        ];
        let levels = chart
            .naked_profile_levels(
                ProfileSource::Candles {
                    price_series,
                    volume_series,
                },
                &profiles,
            )
            .unwrap();
        assert_eq!(
            levels
                .iter()
                .map(|level| level.touched_timestamp_micros)
                .collect::<Vec<_>>(),
            vec![Some(1_000_000), Some(2_000_000), Some(2_000_000)]
        );
    }

    #[test]
    fn frame_developing_profile_samples_bounded_history_and_keeps_last_trade() {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        let stream_id = chart
            .add_trade_stream(
                "developing-sample",
                FootprintAggregationOptions {
                    tick_size: 1.0,
                    ..Default::default()
                },
            )
            .unwrap();
        let trades = (0..4_096)
            .map(|timestamp_micros| FootprintTrade {
                timestamp_micros,
                price: if timestamp_micros < 2_048 {
                    100.0
                } else {
                    101.0
                },
                volume: 1.0,
                aggressor: AggressorSide::Buy,
                bid: None,
                ask: None,
                sequence: None,
                trade_id: None,
                conditions: 0,
                session_id: Some(1),
            })
            .collect();
        chart.set_trade_stream_trades(stream_id, trades).unwrap();
        let profiles = chart
            .periodic_volume_profiles_for_frame_developing(
                ProfileSource::Tape { stream_id },
                &[ResampleBoundary {
                    start_time: 0,
                    end_time: 1,
                    session_id: 1,
                }],
                1.0,
                8,
                70.0,
            )
            .unwrap();
        let profile = &profiles[0];
        assert!(profile.developing.len() <= MAX_PERIODIC_DEVELOPING_POINTS);
        assert_eq!(profile.developing.first().unwrap().timestamp_micros, 0);
        assert_eq!(profile.developing.last().unwrap().timestamp_micros, 4_095);
        assert_eq!(profile.developing.last().unwrap().poc, profile.poc.unwrap());
    }

    #[test]
    fn tpo_rejects_overlapping_or_reversed_host_periods() {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        let source = chart.add_series(SeriesKind::Candlestick);
        let mut request = TpoRequest {
            price_series: source,
            boundaries: vec![
                ResampleBoundary {
                    start_time: 0,
                    end_time: 120,
                    session_id: 1,
                },
                ResampleBoundary {
                    start_time: 60,
                    end_time: 180,
                    session_id: 2,
                },
            ],
            period_seconds: 30,
            tick_size: 1.0,
            value_area_percent: 70.0,
            initial_balance_periods: 2,
        };
        assert_eq!(
            chart.tpo_profiles(&request),
            Err(ProfileError::InvalidRequest)
        );
        request.boundaries[1] = ResampleBoundary {
            start_time: 180,
            end_time: 120,
            session_id: 2,
        };
        assert_eq!(
            chart.tpo_profiles(&request),
            Err(ProfileError::InvalidRequest)
        );
    }

    #[test]
    fn tpo_merges_adjacent_host_segments_with_the_same_session_id() {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        let source = chart.add_series(SeriesKind::Candlestick);
        chart
            .set_series_data(
                source,
                &[0.0, 60.0, 1_000.0, 1_060.0],
                &[100.0, 101.0, 102.0, 103.0],
                &[100.0, 101.0, 102.0, 103.0],
                &[100.0, 101.0, 102.0, 103.0],
                &[100.0, 101.0, 102.0, 103.0],
            )
            .unwrap();
        let mut request = TpoRequest {
            price_series: source,
            boundaries: vec![
                ResampleBoundary {
                    start_time: 0,
                    end_time: 120,
                    session_id: 7,
                },
                ResampleBoundary {
                    start_time: 1_000,
                    end_time: 1_120,
                    session_id: 7,
                },
            ],
            period_seconds: 60,
            tick_size: 1.0,
            value_area_percent: 70.0,
            initial_balance_periods: 2,
        };
        let merged = chart.tpo_profiles(&request).unwrap();
        assert_eq!(merged.len(), 1);
        assert_eq!(
            merged[0]
                .rows
                .iter()
                .map(|row| &row.periods)
                .collect::<Vec<_>>(),
            vec![&vec![0], &vec![1], &vec![2], &vec![3]]
        );
        assert_eq!(
            (
                merged[0].initial_balance_low,
                merged[0].initial_balance_high
            ),
            (Some(100.0), Some(101.0))
        );
        request.boundaries[1].session_id = 8;
        assert_eq!(chart.tpo_profiles(&request).unwrap().len(), 2);
    }

    #[test]
    fn tpo_rejects_excessive_total_cells_before_retaining_them() {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        let source = chart.add_series(SeriesKind::Candlestick);
        let times = (0..1_024).map(f64::from).collect::<Vec<_>>();
        chart
            .set_series_data(
                source,
                &times,
                &vec![50.0; times.len()],
                &vec![100.0; times.len()],
                &vec![1.0; times.len()],
                &vec![50.0; times.len()],
            )
            .unwrap();
        let result = chart.tpo_profiles(&TpoRequest {
            price_series: source,
            boundaries: vec![ResampleBoundary {
                start_time: 0,
                end_time: 1_024,
                session_id: 1,
            }],
            period_seconds: 1,
            tick_size: 1.0,
            value_area_percent: 70.0,
            initial_balance_periods: 2,
        });
        assert_eq!(result, Err(ProfileError::LimitExceeded));
    }

    #[test]
    fn periodic_profiles_reject_unrepresentable_boundaries_and_total_rows() {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        let source = chart.add_series(SeriesKind::Candlestick);
        let volume = chart.add_series(SeriesKind::Histogram);
        let input = ProfileSource::Candles {
            price_series: source,
            volume_series: volume,
        };
        assert_eq!(
            chart.periodic_volume_profiles(
                input,
                &[ResampleBoundary {
                    start_time: i64::MAX - 1,
                    end_time: i64::MAX,
                    session_id: 1
                }],
                1.0,
                2_048,
                70.0,
            ),
            Err(ProfileError::InvalidRequest),
        );

        let times = (0..196)
            .map(|index| f64::from(index * 60))
            .collect::<Vec<_>>();
        chart
            .set_series_data(
                source,
                &times,
                &vec![256.0; 196],
                &vec![512.0; 196],
                &vec![1.0; 196],
                &vec![256.0; 196],
            )
            .unwrap();
        chart
            .set_series_data(
                volume,
                &times,
                &vec![1.0; 196],
                &vec![1.0; 196],
                &vec![1.0; 196],
                &vec![1.0; 196],
            )
            .unwrap();
        let boundaries = (0..196)
            .map(|index| ResampleBoundary {
                start_time: i64::from(index * 60),
                end_time: i64::from(index * 60 + 1),
                session_id: index as u64,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            chart.periodic_volume_profiles(input, &boundaries, 1.0, 512, 70.0),
            Err(ProfileError::LimitExceeded)
        );
    }
}
