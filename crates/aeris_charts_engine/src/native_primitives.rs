//! Engine-owned implementations of the official financial primitive examples.
//!
//! Browser hosts translate declarative values at the WASM boundary. Primitive state, autoscale
//! participation, pixel geometry, and lifecycle remain in the shared engine so every retained-
//! frame executor observes the same result.

use std::sync::Arc;

use crate::{AreaBrushOverrides, ChartEngine, PaneId, SeriesId, SeriesKind};
use aeris_charts_render::color::Color;
use aeris_charts_render::draw_list::RasterImage;

pub type NativePrimitiveId = u32;

pub const MAX_RASTER_IMAGE_DIMENSION: u32 = 1024;
const MAX_NATIVE_PANE_PRIMITIVES: usize = 16;
const MAX_NATIVE_TEXT_BYTES: usize = 4 * 1024;
const MAX_NATIVE_FONT_FAMILY_BYTES: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AccessibilityFocusOptions {
    pub color: Color,
    pub size: f64,
    pub high_contrast: bool,
}

impl Default for AccessibilityFocusOptions {
    fn default() -> Self {
        Self {
            color: Color::rgb(41, 98, 255),
            size: 14.0,
            high_contrast: false,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct AccessibilityFocusState {
    pub(crate) options: AccessibilityFocusOptions,
    pub(crate) time: Option<i64>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TooltipOptions {
    pub line_color: Option<Color>,
    pub top_margin: f64,
}

impl Default for TooltipOptions {
    fn default() -> Self {
        Self {
            line_color: None,
            top_margin: 0.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TooltipSnapshot {
    pub x: f64,
    pub index: i64,
    /// Retained for compatibility with the original single-value tooltip contract; always the
    /// canonical close/value at this row.
    pub price: f64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub time: i64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DeltaTooltipOptions {
    pub line_color: Option<Color>,
    pub show_time: bool,
    pub top_offset: f64,
    /// Mouse activation is Shift+primary-drag instead of primary-drag. This is used by composed
    /// interactions that must preserve the chart's ordinary grab-to-pan gesture.
    pub requires_shift_drag: bool,
}

impl Default for DeltaTooltipOptions {
    fn default() -> Self {
        Self {
            line_color: None,
            show_time: false,
            top_offset: 20.0,
            requires_shift_drag: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DeltaTooltipPoint {
    pub index: i64,
}

/// One Area series composed with its Delta Tooltip (see [`ChartEngine::set_brushable_area`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct BrushableArea {
    pub(crate) series: SeriesId,
    pub(crate) tooltip: NativePrimitiveId,
    /// The active range the area brush currently reflects.
    pub(crate) styled: Option<DeltaTooltipActiveRange>,
    pub(crate) overrides: AreaBrushOverrides,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeltaTooltipActiveRange {
    pub from: i64,
    pub to: i64,
    pub positive: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct DeltaTooltipState {
    pub options: DeltaTooltipOptions,
    pub committed_points: Vec<DeltaTooltipPoint>,
    pub preview_points: Vec<DeltaTooltipPoint>,
    pub mouse_start: Option<DeltaTooltipPoint>,
    pub mouse_drawing: bool,
}

impl DeltaTooltipState {
    pub(crate) fn visible_points(&self) -> &[DeltaTooltipPoint] {
        if self.mouse_drawing && self.preview_points.len() == 2 {
            &self.preview_points
        } else if self.committed_points.len() == 2 {
            &self.committed_points
        } else {
            &self.preview_points
        }
    }

    fn range_points(&self) -> &[DeltaTooltipPoint] {
        if self.mouse_drawing && self.preview_points.len() == 2 {
            &self.preview_points
        } else {
            &self.committed_points
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnchoredTextHorizontalAlign {
    Left,
    Middle,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnchoredTextVerticalAlign {
    Top,
    Middle,
    Bottom,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AnchoredTextOptions {
    pub horizontal_align: AnchoredTextHorizontalAlign,
    pub vertical_align: AnchoredTextVerticalAlign,
    pub text: String,
    pub line_height: f64,
    pub font_size: f64,
    pub font_family: String,
    pub font_weight: u16,
    pub italic: bool,
    pub color: Color,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BandsIndicatorOptions {
    pub line_color: Color,
    pub fill_color: Color,
    pub line_width: f64,
}

impl Default for BandsIndicatorOptions {
    fn default() -> Self {
        Self {
            line_color: Color::rgb(25, 200, 100),
            fill_color: Color::rgba(25, 200, 100, 64),
            line_width: 1.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OverlayPriceScaleSide {
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OverlayPriceScaleOptions {
    pub text_color: Option<Color>,
    pub side: OverlayPriceScaleSide,
}

impl Default for OverlayPriceScaleOptions {
    fn default() -> Self {
        Self {
            text_color: None,
            side: OverlayPriceScaleSide::Left,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TextWatermarkLine {
    pub text: String,
    pub color: Color,
    pub font_size: f64,
    pub font_family: String,
    pub font_weight: u16,
    pub italic: bool,
    pub line_height: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TextWatermarkOptions {
    pub visible: bool,
    pub horizontal_align: AnchoredTextHorizontalAlign,
    pub vertical_align: AnchoredTextVerticalAlign,
    pub lines: Vec<TextWatermarkLine>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ImageWatermarkOptions {
    pub max_width: Option<f64>,
    pub max_height: Option<f64>,
    pub padding: f64,
    pub alpha: f64,
}

impl Default for ImageWatermarkOptions {
    fn default() -> Self {
        Self {
            max_width: None,
            max_height: None,
            padding: 0.0,
            alpha: 1.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SessionHighlightingOptions {
    /// Optional exchange-local hour gate in fractional hours (`9.5` is 09:30). Without it, every
    /// source bar receives the weekday/weekend color, matching the official example. With it,
    /// bars outside the half-open session `[start_hour, end_hour)` are clear; `start > end` wraps
    /// midnight. Hours and the weekend test use the chart's exchange time (UTC by default).
    pub start_hour: Option<f64>,
    pub end_hour: Option<f64>,
    pub weekday_color: Color,
    pub weekend_color: Color,
}

/// Host-evaluated result of the official session-highlighter callback. The callback itself is a
/// browser-host concern; the shared engine retains these aligned records and owns all geometry.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SessionHighlightingData {
    pub time: i64,
    pub color: Color,
}

impl Default for SessionHighlightingOptions {
    fn default() -> Self {
        Self {
            start_hour: None,
            end_hour: None,
            weekday_color: Color::rgba(41, 98, 255, 20),
            weekend_color: Color::rgba(255, 152, 1, 20),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct SessionHighlightingState {
    pub(crate) options: SessionHighlightingOptions,
    pub(crate) highlights: Option<Vec<SessionHighlightingData>>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VolumeProfilePoint {
    pub price: f64,
    pub volume: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct VolumeProfileData {
    pub time: i64,
    pub profile: Vec<VolumeProfilePoint>,
    /// Width in time-scale bar slots, matching the official example.
    pub width: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VolumeProfileOptions {
    pub background_color: Color,
    pub row_color: Color,
}

#[derive(Clone, Debug, PartialEq)]
pub struct VerticalLineOptions {
    pub color: Color,
    pub label_text: String,
    pub width: f64,
    pub label_background_color: Color,
    pub label_text_color: Option<Color>,
    pub show_label: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TrendLineOptions {
    pub line_color: Color,
    pub width: f64,
    pub show_labels: bool,
    pub label_background_color: Color,
    pub label_text_color: Color,
}

impl Default for TrendLineOptions {
    fn default() -> Self {
        Self {
            line_color: Color::rgb(0, 0, 0),
            width: 6.0,
            show_labels: true,
            label_background_color: Color::rgba(255, 255, 255, 217),
            label_text_color: Color::rgb(0, 0, 0),
        }
    }
}

impl Default for VerticalLineOptions {
    fn default() -> Self {
        Self {
            color: Color::rgb(0, 128, 0),
            label_text: String::new(),
            width: 3.0,
            label_background_color: Color::rgb(0, 128, 0),
            label_text_color: None,
            show_label: false,
        }
    }
}

impl Default for VolumeProfileOptions {
    fn default() -> Self {
        Self {
            background_color: Color::rgba(0, 0, 255, 51),
            row_color: Color::rgba(80, 80, 255, 204),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) enum NativeSeriesPrimitiveKind {
    AnchoredText(AnchoredTextOptions),
    ImageWatermark {
        image: RasterImage,
        options: ImageWatermarkOptions,
    },
    BandsIndicator(BandsIndicatorOptions),
    OverlayPriceScale(OverlayPriceScaleOptions),
    AccessibilityFocus(AccessibilityFocusState),
    SessionHighlighting(SessionHighlightingState),
    HighlightBarCrosshair {
        color: Option<Color>,
    },
    VerticalLine {
        time: i64,
        options: VerticalLineOptions,
    },
    Tooltip(TooltipOptions),
    DeltaTooltip(DeltaTooltipState),
    TrendLine {
        first_time: i64,
        first_price: f64,
        second_time: i64,
        second_price: f64,
        options: TrendLineOptions,
    },
    VolumeProfileIndicator(Box<crate::volume_profile::VolumeProfileIndicatorState>),
    VolumeProfile {
        data: VolumeProfileData,
        options: VolumeProfileOptions,
    },
}

#[derive(Clone, Debug)]
pub(crate) struct NativeSeriesPrimitive {
    pub id: NativePrimitiveId,
    pub kind: NativeSeriesPrimitiveKind,
}

#[derive(Clone, Debug)]
pub(crate) enum NativePanePrimitiveKind {
    TextWatermark(TextWatermarkOptions),
}

#[derive(Clone, Debug)]
pub(crate) struct NativePanePrimitive {
    pub id: NativePrimitiveId,
    pub pane_id: PaneId,
    pub kind: NativePanePrimitiveKind,
}

impl NativeSeriesPrimitive {
    fn capacity_bytes(&self) -> usize {
        match &self.kind {
            NativeSeriesPrimitiveKind::AnchoredText(options) => {
                options.text.capacity() + options.font_family.capacity()
            }
            NativeSeriesPrimitiveKind::ImageWatermark { image, .. } => image.pixels.len(),
            NativeSeriesPrimitiveKind::SessionHighlighting(state) => {
                state.highlights.as_ref().map_or(0, |items| {
                    items.capacity() * core::mem::size_of::<SessionHighlightingData>()
                })
            }
            NativeSeriesPrimitiveKind::VolumeProfileIndicator(state) => state.capacity_bytes(),
            NativeSeriesPrimitiveKind::VolumeProfile { data, .. } => {
                data.profile.capacity() * core::mem::size_of::<VolumeProfilePoint>()
            }
            NativeSeriesPrimitiveKind::VerticalLine { options, .. } => {
                options.label_text.capacity()
            }
            NativeSeriesPrimitiveKind::DeltaTooltip(state) => {
                (state.committed_points.capacity() + state.preview_points.capacity())
                    * core::mem::size_of::<DeltaTooltipPoint>()
            }
            _ => 0,
        }
    }
}

fn valid_volume_profile(data: &VolumeProfileData) -> bool {
    data.width.is_finite()
        && data.width > 0.0
        && data.width <= 10_000.0
        && (2..=512).contains(&data.profile.len())
        && data
            .profile
            .iter()
            .all(|point| point.price.is_finite() && point.volume.is_finite() && point.volume >= 0.0)
        && data.profile.iter().any(|point| point.volume > 0.0)
}

fn valid_bands_indicator_options(options: BandsIndicatorOptions) -> bool {
    options.line_width.is_finite() && options.line_width > 0.0 && options.line_width <= 64.0
}

fn valid_accessibility_focus_options(options: AccessibilityFocusOptions) -> bool {
    options.size.is_finite() && (4.0..=128.0).contains(&options.size)
}

fn valid_vertical_line(time: i64, options: &VerticalLineOptions) -> bool {
    time != i64::MIN
        && options.width.is_finite()
        && options.width > 0.0
        && options.width <= 32.0
        && options.label_text.len() <= MAX_NATIVE_TEXT_BYTES
}

fn valid_trend_line(
    first_time: i64,
    first_price: f64,
    second_time: i64,
    second_price: f64,
    options: &TrendLineOptions,
) -> bool {
    first_time != i64::MIN
        && second_time != i64::MIN
        && first_price.is_finite()
        && second_price.is_finite()
        && options.width.is_finite()
        && options.width > 0.0
        && options.width <= 64.0
}

fn valid_image_options(options: ImageWatermarkOptions) -> bool {
    options.padding.is_finite()
        && options.padding >= 0.0
        && options.alpha.is_finite()
        && (0.0..=1.0).contains(&options.alpha)
        && options
            .max_width
            .is_none_or(|value| value.is_finite() && value > 0.0)
        && options
            .max_height
            .is_none_or(|value| value.is_finite() && value > 0.0)
}

fn valid_anchored_text_options(options: &AnchoredTextOptions) -> bool {
    !options.text.is_empty()
        && options.text.len() <= MAX_NATIVE_TEXT_BYTES
        && !options.font_family.is_empty()
        && options.font_family.len() <= MAX_NATIVE_FONT_FAMILY_BYTES
        && options.line_height.is_finite()
        && options.line_height > 0.0
        && options.font_size.is_finite()
        && options.font_size > 0.0
        && options.font_size <= 512.0
        && (100..=900).contains(&options.font_weight)
}

fn valid_text_watermark_options(options: &TextWatermarkOptions) -> bool {
    options.lines.len() <= 32
        && options.lines.iter().all(|line| {
            line.text.len() <= MAX_NATIVE_TEXT_BYTES
                && !line.font_family.is_empty()
                && line.font_family.len() <= MAX_NATIVE_FONT_FAMILY_BYTES
                && line.font_size.is_finite()
                && line.font_size > 0.0
                && line.font_size <= 512.0
                && line.line_height.is_finite()
                && line.line_height > 0.0
                && (100..=900).contains(&line.font_weight)
        })
}

impl ChartEngine {
    fn insert_native_pane_primitive(
        &mut self,
        pane_index: usize,
        kind: NativePanePrimitiveKind,
    ) -> Option<NativePrimitiveId> {
        let pane_id = self.panes.get(pane_index)?.stable_id()?;
        if self.native_pane_primitives.len() >= MAX_NATIVE_PANE_PRIMITIVES {
            return None;
        }
        let id = self.next_native_primitive_id;
        self.next_native_primitive_id = id.checked_add(1)?;
        self.native_pane_primitives
            .push(NativePanePrimitive { id, pane_id, kind });
        self.invalidate_frame_scene();
        Some(id)
    }

    pub(crate) fn insert_native_primitive(
        &mut self,
        series_id: SeriesId,
        kind: NativeSeriesPrimitiveKind,
    ) -> Option<NativePrimitiveId> {
        self.validate_series_id(series_id).ok()?;
        let id = self.next_native_primitive_id;
        self.next_native_primitive_id = id.checked_add(1)?;
        self.series_entry_mut(series_id)?
            .native_primitives
            .push(NativeSeriesPrimitive { id, kind });
        self.invalidate_frame_series(series_id);
        Some(id)
    }

    pub fn add_bands_indicator(
        &mut self,
        series_id: SeriesId,
        options: BandsIndicatorOptions,
    ) -> Option<NativePrimitiveId> {
        valid_bands_indicator_options(options).then_some(())?;
        self.insert_native_primitive(
            series_id,
            NativeSeriesPrimitiveKind::BandsIndicator(options),
        )
    }

    pub fn set_bands_indicator_options(
        &mut self,
        primitive_id: NativePrimitiveId,
        options: BandsIndicatorOptions,
    ) -> bool {
        if !valid_bands_indicator_options(options) {
            return false;
        }
        let Some((series_id, current)) = self.series.iter_mut().find_map(|series| {
            series.native_primitives.iter_mut().find_map(|primitive| {
                (primitive.id == primitive_id).then_some(()).and_then(|()| {
                    let NativeSeriesPrimitiveKind::BandsIndicator(current) = &mut primitive.kind
                    else {
                        return None;
                    };
                    Some((series.id, current))
                })
            })
        }) else {
            return false;
        };
        *current = options;
        self.invalidate_frame_series(series_id);
        true
    }

    pub fn add_overlay_price_scale(
        &mut self,
        series_id: SeriesId,
        options: OverlayPriceScaleOptions,
    ) -> Option<NativePrimitiveId> {
        self.insert_native_primitive(
            series_id,
            NativeSeriesPrimitiveKind::OverlayPriceScale(options),
        )
    }

    pub fn set_overlay_price_scale_options(
        &mut self,
        primitive_id: NativePrimitiveId,
        options: OverlayPriceScaleOptions,
    ) -> bool {
        let Some((series_id, current)) = self.series.iter_mut().find_map(|series| {
            series.native_primitives.iter_mut().find_map(|primitive| {
                (primitive.id == primitive_id).then_some(()).and_then(|()| {
                    let NativeSeriesPrimitiveKind::OverlayPriceScale(current) = &mut primitive.kind
                    else {
                        return None;
                    };
                    Some((series.id, current))
                })
            })
        }) else {
            return false;
        };
        *current = options;
        self.invalidate_frame_series(series_id);
        true
    }

    pub fn add_accessibility_focus(
        &mut self,
        series_id: SeriesId,
        options: AccessibilityFocusOptions,
    ) -> Option<NativePrimitiveId> {
        valid_accessibility_focus_options(options).then_some(())?;
        let id = self.insert_native_primitive(
            series_id,
            NativeSeriesPrimitiveKind::AccessibilityFocus(AccessibilityFocusState {
                options,
                time: None,
            }),
        )?;
        self.invalidate_frame_overlay();
        Some(id)
    }

    pub fn set_accessibility_focus(
        &mut self,
        primitive_id: NativePrimitiveId,
        time: Option<i64>,
        options: AccessibilityFocusOptions,
    ) -> bool {
        if !valid_accessibility_focus_options(options) {
            return false;
        }
        let Some(state) = self.series.iter_mut().find_map(|series| {
            series.native_primitives.iter_mut().find_map(|primitive| {
                (primitive.id == primitive_id)
                    .then_some(&mut primitive.kind)
                    .and_then(|kind| match kind {
                        NativeSeriesPrimitiveKind::AccessibilityFocus(state) => Some(state),
                        _ => None,
                    })
            })
        }) else {
            return false;
        };
        state.time = time;
        state.options = options;
        self.invalidate_frame_overlay();
        true
    }

    pub fn add_session_highlighting(
        &mut self,
        series_id: SeriesId,
        options: SessionHighlightingOptions,
    ) -> Option<NativePrimitiveId> {
        if options
            .start_hour
            .is_some_and(|hour| !(0.0..24.0).contains(&hour))
            || options
                .end_hour
                .is_some_and(|hour| !(0.0..=24.0).contains(&hour))
            || options.start_hour.is_some() != options.end_hour.is_some()
        {
            return None;
        }
        self.insert_native_primitive(
            series_id,
            NativeSeriesPrimitiveKind::SessionHighlighting(SessionHighlightingState {
                options,
                highlights: None,
            }),
        )
    }

    /// Replace the callback-derived colors for an official session-highlighting primitive.
    /// Records must exactly align with the primitive's source series so stale host state can never
    /// color a different bar after an update or retention trim.
    pub fn set_session_highlighting_data(
        &mut self,
        primitive_id: NativePrimitiveId,
        highlights: Vec<SessionHighlightingData>,
    ) -> bool {
        let Some(series_id) = self.series.iter().find_map(|series| {
            series
                .native_primitives
                .iter()
                .any(|primitive| primitive.id == primitive_id)
                .then_some(series.id)
        }) else {
            return false;
        };
        let Some((times, _)) = self.data.series_data(series_id) else {
            return false;
        };
        if highlights.len() != times.len()
            || highlights
                .iter()
                .zip(times)
                .any(|(highlight, time)| highlight.time != *time)
        {
            return false;
        }
        let Some(state) = self.series_entry_mut(series_id).and_then(|series| {
            series.native_primitives.iter_mut().find_map(|primitive| {
                (primitive.id == primitive_id)
                    .then_some(&mut primitive.kind)
                    .and_then(|kind| match kind {
                        NativeSeriesPrimitiveKind::SessionHighlighting(state) => Some(state),
                        _ => None,
                    })
            })
        }) else {
            return false;
        };
        state.highlights = Some(highlights);
        self.invalidate_frame_series(series_id);
        true
    }

    /// Merge callback colors for source rows that changed at the tail (appended rows and a
    /// replaced current bar) without resending the aligned history, so a live update costs work
    /// proportional to the changed rows. Records must be strictly ascending; each replaces the
    /// record with the same time or appends after the last one. Records older than the source's
    /// first row are dropped first, so `max_points` retention trims stay incremental. Nothing
    /// changes unless the result stays aligned with the source (count and both endpoints); the
    /// host then falls back to [`Self::set_session_highlighting_data`].
    pub fn upsert_session_highlighting_data(
        &mut self,
        primitive_id: NativePrimitiveId,
        records: Vec<SessionHighlightingData>,
    ) -> bool {
        if records.windows(2).any(|pair| pair[0].time >= pair[1].time) {
            return false;
        }
        let Some(series_id) = self.series.iter().find_map(|series| {
            series
                .native_primitives
                .iter()
                .any(|primitive| primitive.id == primitive_id)
                .then_some(series.id)
        }) else {
            return false;
        };
        let Some((times, _)) = self.data.series_data(series_id) else {
            return false;
        };
        let source_len = times.len();
        let source_first = times.first().copied();
        let source_last = times.last().copied();
        let Some(state) = self.series_entry_mut(series_id).and_then(|series| {
            series.native_primitives.iter_mut().find_map(|primitive| {
                (primitive.id == primitive_id)
                    .then_some(&mut primitive.kind)
                    .and_then(|kind| match kind {
                        NativeSeriesPrimitiveKind::SessionHighlighting(state) => Some(state),
                        _ => None,
                    })
            })
        }) else {
            return false;
        };
        let Some(highlights) = state.highlights.as_mut() else {
            return false;
        };
        if records
            .first()
            .is_some_and(|record| source_first.is_none_or(|first| record.time < first))
        {
            return false;
        }
        // Rows retention evicted from the source's head.
        let evicted = source_first.map_or(0, |first| {
            highlights.partition_point(|highlight| highlight.time < first)
        });
        let last_time = highlights.last().map(|highlight| highlight.time);
        let appended = records
            .iter()
            .filter(|record| last_time.is_none_or(|last| record.time > last))
            .count();
        let replaced_exist = records
            .iter()
            .filter(|record| last_time.is_some_and(|last| record.time <= last))
            .all(|record| {
                highlights
                    .binary_search_by_key(&record.time, |highlight| highlight.time)
                    .is_ok()
            });
        let final_first = highlights
            .get(evicted)
            .map(|highlight| highlight.time)
            .or_else(|| records.first().map(|record| record.time));
        let final_last = records.last().map(|record| record.time).max(last_time);
        if !replaced_exist
            || highlights.len() - evicted + appended != source_len
            || final_first != source_first
            || final_last != source_last
        {
            return false;
        }
        if records.is_empty() && evicted == 0 {
            return true;
        }
        highlights.drain(..evicted);
        for record in records {
            match highlights.binary_search_by_key(&record.time, |highlight| highlight.time) {
                Ok(index) => highlights[index] = record,
                Err(_) => highlights.push(record),
            }
        }
        self.invalidate_frame_series(series_id);
        true
    }

    /// Canonical source times at or after `since` (ascending). Lets a host evaluate per-row
    /// callbacks only for the rows a live update touched.
    pub fn series_times_since(&self, series_id: SeriesId, since: i64) -> &[i64] {
        self.data
            .series_data(series_id)
            .map_or(&[][..], |(times, _)| {
                &times[times.partition_point(|&time| time < since)..]
            })
    }

    pub fn add_highlight_bar_crosshair(
        &mut self,
        series_id: SeriesId,
        color: Option<Color>,
    ) -> Option<NativePrimitiveId> {
        self.insert_native_primitive(
            series_id,
            NativeSeriesPrimitiveKind::HighlightBarCrosshair { color },
        )
    }

    pub fn add_vertical_line(
        &mut self,
        series_id: SeriesId,
        time: i64,
        options: VerticalLineOptions,
    ) -> Option<NativePrimitiveId> {
        valid_vertical_line(time, &options).then_some(())?;
        self.insert_native_primitive(
            series_id,
            NativeSeriesPrimitiveKind::VerticalLine { time, options },
        )
    }

    pub fn add_delta_tooltip(
        &mut self,
        series_id: SeriesId,
        options: DeltaTooltipOptions,
    ) -> Option<NativePrimitiveId> {
        // Product contract: candlesticks use the ordinary hover tooltip. The drag-comparison
        // delta tooltip is intentionally unavailable on candlestick series because its two-point
        // interaction conflicts with candle inspection and produces misleading/broken chrome.
        // Keep the restriction in the engine so every host (WASM, native/GPUI, future adapters)
        // observes the same capability boundary rather than relying on demo/UI gating.
        if self.series_entry(series_id)?.kind == SeriesKind::Candlestick {
            return None;
        }
        if !options.top_offset.is_finite() || !(0.0..=1_000.0).contains(&options.top_offset) {
            return None;
        }
        let id = self.insert_native_primitive(
            series_id,
            NativeSeriesPrimitiveKind::DeltaTooltip(DeltaTooltipState {
                options,
                committed_points: Vec::with_capacity(2),
                preview_points: Vec::with_capacity(2),
                mouse_start: None,
                mouse_drawing: false,
            }),
        )?;
        self.invalidate_frame_overlay();
        Some(id)
    }

    pub fn add_tooltip(
        &mut self,
        series_id: SeriesId,
        options: TooltipOptions,
    ) -> Option<NativePrimitiveId> {
        if !options.top_margin.is_finite() || !(0.0..=1_000.0).contains(&options.top_margin) {
            return None;
        }
        let id =
            self.insert_native_primitive(series_id, NativeSeriesPrimitiveKind::Tooltip(options))?;
        self.invalidate_frame_overlay();
        Some(id)
    }

    pub fn set_tooltip_options(
        &mut self,
        primitive_id: NativePrimitiveId,
        options: TooltipOptions,
    ) -> bool {
        if !options.top_margin.is_finite() || !(0.0..=1_000.0).contains(&options.top_margin) {
            return false;
        }
        let Some(current) = self.series.iter_mut().find_map(|series| {
            series.native_primitives.iter_mut().find_map(|primitive| {
                (primitive.id == primitive_id)
                    .then_some(&mut primitive.kind)
                    .and_then(|kind| {
                        let NativeSeriesPrimitiveKind::Tooltip(current) = kind else {
                            return None;
                        };
                        Some(current)
                    })
            })
        }) else {
            return false;
        };
        if *current != options {
            *current = options;
            self.invalidate_frame_overlay();
        }
        true
    }

    pub fn tooltip_snapshot(&self, primitive_id: NativePrimitiveId) -> Option<TooltipSnapshot> {
        let (series_id, _) = self.series.iter().find_map(|series| {
            series.native_primitives.iter().find_map(|primitive| {
                (primitive.id == primitive_id
                    && matches!(primitive.kind, NativeSeriesPrimitiveKind::Tooltip(_)))
                .then_some((series.id, primitive))
            })
        })?;
        let (pointer_x, pointer_y) = self.clamped_crosshair()?;
        let pane_index = self.series_entry(series_id)?.pane_index;
        if self.pane_at_y(pointer_y) != Some(pane_index) {
            return None;
        }
        let index = self.snapped_crosshair_index(pointer_x);
        let plot = self.data.plot(series_id);
        let row = plot.search(
            index,
            aeris_charts_core::model::plot_list::MismatchDirection::None,
        )?;
        if plot.is_whitespace_row(row) {
            return None;
        }
        let (times, _) = self.data.series_data(series_id)?;
        let open = plot.value_at(
            row,
            aeris_charts_core::model::plot_list::PlotValueIndex::Open,
        );
        let high = plot.value_at(
            row,
            aeris_charts_core::model::plot_list::PlotValueIndex::High,
        );
        let low = plot.value_at(
            row,
            aeris_charts_core::model::plot_list::PlotValueIndex::Low,
        );
        let close = plot.value_at(
            row,
            aeris_charts_core::model::plot_list::PlotValueIndex::Close,
        );
        Some(TooltipSnapshot {
            x: self.time_scale.index_to_coordinate(index),
            index,
            price: close,
            open,
            high,
            low,
            close,
            // The bar's own time (an as-of point repeats an earlier row of its series).
            time: *times.get(plot.source_row(row))?,
        })
    }

    pub fn set_delta_tooltip_points(
        &mut self,
        primitive_id: NativePrimitiveId,
        xs: &[f64],
    ) -> bool {
        if xs.len() > 2 || xs.iter().any(|x| !x.is_finite()) {
            return false;
        }
        let Some(series_id) = self.series.iter().find_map(|series| {
            series
                .native_primitives
                .iter()
                .any(|primitive| {
                    primitive.id == primitive_id
                        && matches!(primitive.kind, NativeSeriesPrimitiveKind::DeltaTooltip(_))
                })
                .then_some(series.id)
        }) else {
            return false;
        };
        let plot = self.data.plot(series_id);
        let points: Vec<_> = xs
            .iter()
            .filter(|x| **x >= 0.0 && **x <= self.pane_w)
            .filter_map(|x| {
                let index = self.time_scale.coordinate_to_index(*x);
                plot.search(
                    index,
                    aeris_charts_core::model::plot_list::MismatchDirection::None,
                )
                .filter(|row| !plot.is_whitespace_row(*row))
                .map(|_| DeltaTooltipPoint { index })
            })
            .collect();
        let Some(state) = self.series.iter_mut().find_map(|series| {
            series.native_primitives.iter_mut().find_map(|primitive| {
                if primitive.id != primitive_id {
                    return None;
                }
                let NativeSeriesPrimitiveKind::DeltaTooltip(state) = &mut primitive.kind else {
                    return None;
                };
                Some(state)
            })
        }) else {
            return false;
        };
        let (committed, preview) = if points.len() == 2 {
            (points, Vec::new())
        } else {
            (Vec::new(), points)
        };
        if state.committed_points == committed && state.preview_points == preview {
            return true;
        }
        state.committed_points = committed;
        state.preview_points = preview;
        state.mouse_start = None;
        state.mouse_drawing = false;
        self.invalidate_frame_overlay();
        true
    }

    pub fn delta_tooltip_active_range(
        &self,
        primitive_id: NativePrimitiveId,
    ) -> Option<DeltaTooltipActiveRange> {
        let (series_id, state) = self.series.iter().find_map(|series| {
            series.native_primitives.iter().find_map(|primitive| {
                if primitive.id != primitive_id {
                    return None;
                }
                let NativeSeriesPrimitiveKind::DeltaTooltip(state) = &primitive.kind else {
                    return None;
                };
                Some((series.id, state))
            })
        })?;
        let range_points = state.range_points();
        if range_points.len() != 2 {
            return None;
        }
        let mut points = range_points.to_vec();
        points.sort_by_key(|point| point.index);
        let plot = self.data.plot(series_id);
        let price = |point: DeltaTooltipPoint| {
            plot.search(
                point.index,
                aeris_charts_core::model::plot_list::MismatchDirection::None,
            )
            .map(|row| {
                plot.value_at(
                    row,
                    aeris_charts_core::model::plot_list::PlotValueIndex::Close,
                )
            })
        };
        let first = price(points[0])?;
        let second = price(points[1])?;
        Some(DeltaTooltipActiveRange {
            from: points[0].index.saturating_add(1),
            to: points[1].index.saturating_add(1),
            positive: second - first >= 0.0,
        })
    }

    /// Capture the first comparison point for the official mouse-drag interaction. The host
    /// forwards only the pane x-coordinate; logical lookup and all plugin state stay here.
    pub fn delta_tooltip_mouse_down(&mut self, x: f64) -> bool {
        self.delta_tooltip_mouse_down_with_shift(x, false)
    }

    pub fn delta_tooltip_mouse_down_with_shift(&mut self, x: f64, shift: bool) -> bool {
        if !x.is_finite() {
            return false;
        }
        let targets: Vec<_> = self
            .series
            .iter()
            .flat_map(|series| {
                series
                    .native_primitives
                    .iter()
                    .filter_map(move |primitive| {
                        let NativeSeriesPrimitiveKind::DeltaTooltip(state) = &primitive.kind else {
                            return None;
                        };
                        (!state.options.requires_shift_drag || shift)
                            .then_some((series.id, primitive.id))
                    })
            })
            .collect();
        let starts: Vec<_> = targets
            .iter()
            .map(|(series_id, primitive_id)| {
                (*primitive_id, self.delta_tooltip_point(*series_id, x))
            })
            .collect();
        let mut handled = false;
        let mut changed = false;
        for series in self.series.presentation_iter_mut() {
            for primitive in &mut series.native_primitives {
                let NativeSeriesPrimitiveKind::DeltaTooltip(state) = &mut primitive.kind else {
                    continue;
                };
                if state.options.requires_shift_drag && !shift {
                    continue;
                }
                handled = true;
                if !state.preview_points.is_empty() {
                    state.preview_points.clear();
                    changed = true;
                }
                state.mouse_start = starts
                    .iter()
                    .find_map(|(id, point)| (*id == primitive.id).then_some(*point))
                    .flatten();
                state.mouse_drawing = state.mouse_start.is_some();
            }
        }
        if changed {
            self.invalidate_frame_overlay();
        }
        handled
    }

    /// Update every attached delta tooltip from the live mouse position. A held primary button
    /// yields the captured point plus the live point; ordinary hover yields one point.
    pub fn delta_tooltip_mouse_move(&mut self, x: f64) -> bool {
        if !x.is_finite() {
            return self.clear_delta_tooltip_previews(true);
        }
        let targets: Vec<_> = self
            .series
            .iter()
            .flat_map(|series| {
                series
                    .native_primitives
                    .iter()
                    .filter_map(move |primitive| {
                        matches!(primitive.kind, NativeSeriesPrimitiveKind::DeltaTooltip(_))
                            .then_some((series.id, primitive.id))
                    })
            })
            .collect();
        let live: Vec<_> = targets
            .iter()
            .map(|(series_id, primitive_id)| {
                (*primitive_id, self.delta_tooltip_point(*series_id, x))
            })
            .collect();
        let mut changed = false;
        for series in self.series.presentation_iter_mut() {
            for primitive in &mut series.native_primitives {
                let NativeSeriesPrimitiveKind::DeltaTooltip(state) = &mut primitive.kind else {
                    continue;
                };
                let current = live
                    .iter()
                    .find_map(|(id, point)| (*id == primitive.id).then_some(*point))
                    .flatten();
                let mut points = Vec::with_capacity(2);
                if let Some(current) = current {
                    if state.mouse_drawing {
                        if let Some(start) = state.mouse_start {
                            points.push(start);
                        }
                    }
                    points.push(current);
                }
                if state.preview_points != points {
                    state.preview_points = points;
                    changed = true;
                }
            }
        }
        if changed {
            self.invalidate_frame_overlay();
        }
        changed
    }

    /// Commit a completed mouse comparison. An incomplete replacement gesture leaves the prior
    /// committed range intact.
    pub fn delta_tooltip_mouse_up(&mut self) -> bool {
        let mut changed = false;
        for series in self.series.presentation_iter_mut() {
            for primitive in &mut series.native_primitives {
                let NativeSeriesPrimitiveKind::DeltaTooltip(state) = &mut primitive.kind else {
                    continue;
                };
                if state.mouse_drawing
                    && state.preview_points.len() == 2
                    && state.committed_points != state.preview_points
                {
                    state.committed_points.clone_from(&state.preview_points);
                    changed = true;
                }
                if !state.preview_points.is_empty() {
                    state.preview_points.clear();
                    changed = true;
                }
                state.mouse_drawing = false;
                state.mouse_start = None;
            }
        }
        if changed {
            self.invalidate_frame_overlay();
        }
        changed
    }

    /// Forward a touch-move sample. The first two touches become comparison points; the chart
    /// host remains responsible only for normalizing browser coordinates.
    pub fn delta_tooltip_touch_move(&mut self, xs: &[f64]) -> bool {
        if xs.len() > 2 || xs.iter().any(|x| !x.is_finite()) {
            return false;
        }
        let targets: Vec<_> = self
            .series
            .iter()
            .flat_map(|series| {
                series
                    .native_primitives
                    .iter()
                    .filter_map(move |primitive| {
                        let NativeSeriesPrimitiveKind::DeltaTooltip(state) = &primitive.kind else {
                            return None;
                        };
                        (!state.options.requires_shift_drag).then_some((series.id, primitive.id))
                    })
            })
            .collect();
        let resolved: Vec<_> = targets
            .iter()
            .map(|(series_id, primitive_id)| {
                let points = xs
                    .iter()
                    .filter_map(|x| self.delta_tooltip_point(*series_id, *x))
                    .collect::<Vec<_>>();
                (*primitive_id, points)
            })
            .collect();
        let mut changed = false;
        for series in self.series.presentation_iter_mut() {
            for primitive in &mut series.native_primitives {
                let NativeSeriesPrimitiveKind::DeltaTooltip(state) = &mut primitive.kind else {
                    continue;
                };
                let points = resolved
                    .iter()
                    .find_map(|(id, points)| (*id == primitive.id).then_some(points))
                    .cloned()
                    .unwrap_or_default();
                let state_changed = if points.len() == 2 {
                    let changed =
                        state.committed_points != points || !state.preview_points.is_empty();
                    state.committed_points = points;
                    state.preview_points.clear();
                    changed
                } else if state.preview_points != points {
                    state.preview_points = points;
                    true
                } else {
                    false
                };
                if state_changed {
                    changed = true;
                }
            }
        }
        if changed {
            self.invalidate_frame_overlay();
        }
        changed
    }

    /// Clear transient hover/gesture state on leave while preserving a committed comparison.
    pub fn delta_tooltip_leave(&mut self) -> bool {
        self.clear_delta_tooltip_previews(true)
    }

    /// Explicitly clear one committed delta-tooltip selection and any transient gesture state.
    pub fn clear_delta_tooltip(&mut self, primitive_id: NativePrimitiveId) -> bool {
        let Some(state) = self.series.iter_mut().find_map(|series| {
            series.native_primitives.iter_mut().find_map(|primitive| {
                if primitive.id != primitive_id {
                    return None;
                }
                let NativeSeriesPrimitiveKind::DeltaTooltip(state) = &mut primitive.kind else {
                    return None;
                };
                Some(state)
            })
        }) else {
            return false;
        };
        let changed = !state.committed_points.is_empty() || !state.preview_points.is_empty();
        state.committed_points.clear();
        state.preview_points.clear();
        state.mouse_start = None;
        state.mouse_drawing = false;
        if changed {
            self.invalidate_frame_overlay();
        }
        true
    }

    /// Compose a Delta Tooltip with an ordinary Area series into a brushable comparison, or remove
    /// that composition with `None`. The tooltip owns the range gesture; the engine restyles the
    /// area from its active range with the engine-owned brush defaults, so no host repeats the
    /// styling. Returns false when the series is not a live Area series or rejects the tooltip.
    pub fn set_brushable_area(
        &mut self,
        series_id: SeriesId,
        options: Option<DeltaTooltipOptions>,
    ) -> bool {
        self.set_brushable_area_with_styles(series_id, options, AreaBrushOverrides::default())
    }

    /// Compose an Area and Delta Tooltip with explicit style overrides owned by the engine.
    pub fn set_brushable_area_with_styles(
        &mut self,
        series_id: SeriesId,
        options: Option<DeltaTooltipOptions>,
        overrides: AreaBrushOverrides,
    ) -> bool {
        if let Some(index) = self
            .brushable_areas
            .iter()
            .position(|area| area.series == series_id)
        {
            let area = self.brushable_areas.swap_remove(index);
            self.remove_native_primitive(area.tooltip);
            self.clear_area_brush_state(series_id);
        }
        let Some(options) = options else {
            return true;
        };
        if self.series_entry(series_id).map(|series| series.kind) != Some(SeriesKind::Area) {
            return false;
        }
        let Some(tooltip) = self.add_delta_tooltip(series_id, options) else {
            return false;
        };
        self.brushable_areas.push(BrushableArea {
            series: series_id,
            tooltip,
            styled: None,
            overrides,
        });
        true
    }

    /// Whether `series_id` is currently composed as a brushable area.
    pub fn is_brushable_area(&self, series_id: SeriesId) -> bool {
        self.brushable_areas
            .iter()
            .any(|area| area.series == series_id)
    }

    /// The brushed comparison range of a brushable area, if one is selected.
    pub fn brushable_area_range(&self, series_id: SeriesId) -> Option<DeltaTooltipActiveRange> {
        let area = self
            .brushable_areas
            .iter()
            .find(|area| area.series == series_id)?;
        self.delta_tooltip_active_range(area.tooltip)
    }

    /// Clear one brushable Area's comparison while keeping its interaction attached.
    pub fn clear_brushable_area_range(&mut self, series_id: SeriesId) -> bool {
        let Some(tooltip) = self
            .brushable_areas
            .iter()
            .find(|area| area.series == series_id)
            .map(|area| area.tooltip)
        else {
            return false;
        };
        self.clear_delta_tooltip(tooltip);
        self.sync_brushable_areas();
        true
    }

    /// Clear every brushed comparison range (double-click and Escape).
    pub(crate) fn clear_brushable_ranges(&mut self) {
        let tooltips: Vec<_> = self
            .brushable_areas
            .iter()
            .map(|area| area.tooltip)
            .collect();
        for tooltip in tooltips {
            self.clear_delta_tooltip(tooltip);
        }
        self.sync_brushable_areas();
    }

    /// Restyle each brushable area from its tooltip's active range. Compositions whose series or
    /// tooltip no longer exists are dropped.
    pub(crate) fn sync_brushable_areas(&mut self) {
        let mut index = 0;
        while index < self.brushable_areas.len() {
            let area = self.brushable_areas[index];
            let live = self.series_entry(area.series).map(|series| series.kind)
                == Some(SeriesKind::Area)
                && self.series.iter().any(|series| {
                    series
                        .native_primitives
                        .iter()
                        .any(|primitive| primitive.id == area.tooltip)
                });
            if !live {
                self.brushable_areas.swap_remove(index);
                self.clear_area_brush_state(area.series);
                continue;
            }
            let range = self.delta_tooltip_active_range(area.tooltip);
            if range != area.styled {
                match (
                    range,
                    self.area_brush_defaults(area.series)
                        .map(|defaults| area.overrides.apply(defaults)),
                ) {
                    (Some(range), Some(defaults)) => {
                        let style = if range.positive {
                            defaults.positive
                        } else {
                            defaults.negative
                        };
                        let brush = crate::BrushRange {
                            from: range.from as f64,
                            to: range.to as f64,
                            style,
                        };
                        self.set_area_brush_state(area.series, defaults.outside, vec![brush]);
                    }
                    _ => {
                        self.clear_area_brush_state(area.series);
                    }
                }
                self.brushable_areas[index].styled = range;
            }
            index += 1;
        }
    }

    /// Whether any live series currently owns a delta-tooltip interaction. Hosts use this rather
    /// than retained callback/listener counts so converting an attached series to candlesticks
    /// immediately stops delta-tooltip touch ownership after the engine removes the primitive.
    pub fn has_delta_tooltip(&self) -> bool {
        self.series.iter().any(|series| {
            !series.removed
                && series.native_primitives.iter().any(|primitive| {
                    matches!(&primitive.kind, NativeSeriesPrimitiveKind::DeltaTooltip(_))
                })
        })
    }

    fn delta_tooltip_point(&self, series_id: SeriesId, x: f64) -> Option<DeltaTooltipPoint> {
        if !(0.0..=self.pane_w).contains(&x) {
            return None;
        }
        let index = self.time_scale.coordinate_to_index(x);
        let plot = self.data.plot(series_id);
        plot.search(
            index,
            aeris_charts_core::model::plot_list::MismatchDirection::None,
        )
        .filter(|row| !plot.is_whitespace_row(*row))
        .map(|_| DeltaTooltipPoint { index })
    }

    fn clear_delta_tooltip_previews(&mut self, end_mouse: bool) -> bool {
        let mut changed = false;
        for series in self.series.presentation_iter_mut() {
            for primitive in &mut series.native_primitives {
                let NativeSeriesPrimitiveKind::DeltaTooltip(state) = &mut primitive.kind else {
                    continue;
                };
                if !state.preview_points.is_empty() {
                    state.preview_points.clear();
                    changed = true;
                }
                if end_mouse {
                    state.mouse_drawing = false;
                    state.mouse_start = None;
                }
            }
        }
        if changed {
            self.invalidate_frame_overlay();
        }
        changed
    }

    #[allow(clippy::too_many_arguments)]
    pub fn add_trend_line(
        &mut self,
        series_id: SeriesId,
        first_time: i64,
        first_price: f64,
        second_time: i64,
        second_price: f64,
        options: TrendLineOptions,
    ) -> Option<NativePrimitiveId> {
        valid_trend_line(first_time, first_price, second_time, second_price, &options)
            .then_some(())?;
        self.insert_native_primitive(
            series_id,
            NativeSeriesPrimitiveKind::TrendLine {
                first_time,
                first_price,
                second_time,
                second_price,
                options,
            },
        )
    }

    pub fn add_volume_profile(
        &mut self,
        series_id: SeriesId,
        data: VolumeProfileData,
        options: VolumeProfileOptions,
    ) -> Option<NativePrimitiveId> {
        valid_volume_profile(&data).then_some(())?;
        self.insert_native_primitive(
            series_id,
            NativeSeriesPrimitiveKind::VolumeProfile { data, options },
        )
    }

    pub fn add_image_watermark(
        &mut self,
        series_id: SeriesId,
        width: u32,
        height: u32,
        pixels: Arc<[u8]>,
        options: ImageWatermarkOptions,
    ) -> Option<NativePrimitiveId> {
        let expected = usize::try_from(width)
            .ok()?
            .checked_mul(usize::try_from(height).ok()?)?
            .checked_mul(4)?;
        if width == 0
            || height == 0
            || width > MAX_RASTER_IMAGE_DIMENSION
            || height > MAX_RASTER_IMAGE_DIMENSION
            || pixels.len() != expected
            || !valid_image_options(options)
        {
            return None;
        }
        self.insert_native_primitive(
            series_id,
            NativeSeriesPrimitiveKind::ImageWatermark {
                image: RasterImage {
                    key: u64::from(self.next_native_primitive_id),
                    width,
                    height,
                    pixels,
                },
                options,
            },
        )
    }

    pub fn add_anchored_text(
        &mut self,
        series_id: SeriesId,
        options: AnchoredTextOptions,
    ) -> Option<NativePrimitiveId> {
        valid_anchored_text_options(&options).then_some(())?;
        self.insert_native_primitive(series_id, NativeSeriesPrimitiveKind::AnchoredText(options))
    }

    pub fn add_text_watermark(
        &mut self,
        pane_index: usize,
        options: TextWatermarkOptions,
    ) -> Option<NativePrimitiveId> {
        valid_text_watermark_options(&options).then_some(())?;
        self.insert_native_pane_primitive(
            pane_index,
            NativePanePrimitiveKind::TextWatermark(options),
        )
    }

    pub fn set_text_watermark_options(
        &mut self,
        id: NativePrimitiveId,
        options: TextWatermarkOptions,
    ) -> bool {
        if !valid_text_watermark_options(&options) {
            return false;
        }
        let Some(primitive) = self
            .native_pane_primitives
            .iter_mut()
            .find(|primitive| primitive.id == id)
        else {
            return false;
        };
        let NativePanePrimitiveKind::TextWatermark(current) = &mut primitive.kind;
        *current = options;
        self.invalidate_frame_scene();
        true
    }

    pub fn set_anchored_text_options(
        &mut self,
        id: NativePrimitiveId,
        options: AnchoredTextOptions,
    ) -> bool {
        if !valid_anchored_text_options(&options) {
            return false;
        }
        let Some((series_id, current)) = self.series.iter_mut().find_map(|series| {
            series.native_primitives.iter_mut().find_map(|primitive| {
                (primitive.id == id).then_some(()).and_then(|()| {
                    let NativeSeriesPrimitiveKind::AnchoredText(current) = &mut primitive.kind
                    else {
                        return None;
                    };
                    Some((series.id, current))
                })
            })
        }) else {
            return false;
        };
        *current = options;
        self.invalidate_frame_series(series_id);
        true
    }

    pub fn set_volume_profile_data(
        &mut self,
        id: NativePrimitiveId,
        data: VolumeProfileData,
    ) -> bool {
        if !valid_volume_profile(&data) {
            return false;
        }
        let Some((series_id, primitive)) = self.series.iter_mut().find_map(|series| {
            series
                .native_primitives
                .iter_mut()
                .find(|primitive| primitive.id == id)
                .map(|primitive| (series.id, primitive))
        }) else {
            return false;
        };
        let NativeSeriesPrimitiveKind::VolumeProfile {
            data: current_data, ..
        } = &mut primitive.kind
        else {
            return false;
        };
        *current_data = data;
        self.invalidate_frame_series(series_id);
        true
    }

    pub fn remove_native_primitive(&mut self, id: NativePrimitiveId) -> bool {
        if let Some((series_id, index)) = self.series.iter().find_map(|series| {
            series
                .native_primitives
                .iter()
                .position(|primitive| primitive.id == id)
                .map(|index| (series.id, index))
        }) {
            let Some(series) = self.series_entry_mut(series_id) else {
                return false;
            };
            let removed_delta_tooltip = matches!(
                series.native_primitives[index].kind,
                NativeSeriesPrimitiveKind::DeltaTooltip(_)
            );
            let removed_tooltip = matches!(
                series.native_primitives[index].kind,
                NativeSeriesPrimitiveKind::Tooltip(_)
            );
            series.native_primitives.remove(index);
            self.invalidate_frame_series(series_id);
            if removed_delta_tooltip || removed_tooltip {
                self.invalidate_frame_overlay();
                if removed_delta_tooltip {
                    self.invalidate_axis_frame();
                }
            }
            return true;
        }
        let Some(index) = self
            .native_pane_primitives
            .iter()
            .position(|primitive| primitive.id == id)
        else {
            return false;
        };
        self.native_pane_primitives.remove(index);
        self.invalidate_frame_scene();
        true
    }

    pub(crate) fn native_primitive_capacity_bytes(&self) -> usize {
        self.series
            .iter()
            .map(|series| {
                series.native_primitives.capacity() * core::mem::size_of::<NativeSeriesPrimitive>()
                    + series
                        .native_primitives
                        .iter()
                        .map(NativeSeriesPrimitive::capacity_bytes)
                        .sum::<usize>()
            })
            .sum::<usize>()
            + self.native_pane_primitives.capacity() * core::mem::size_of::<NativePanePrimitive>()
            + self
                .native_pane_primitives
                .iter()
                .map(|primitive| match &primitive.kind {
                    NativePanePrimitiveKind::TextWatermark(options) => {
                        options.lines.capacity() * core::mem::size_of::<TextWatermarkLine>()
                            + options
                                .lines
                                .iter()
                                .map(|line| line.text.capacity() + line.font_family.capacity())
                                .sum::<usize>()
                    }
                })
                .sum::<usize>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aeris_charts_render::draw_list::Prim;
    use std::sync::Arc;

    fn chart() -> ChartEngine {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        let times: Vec<f64> = (0..10).map(|day| day as f64 * 86_400.0).collect();
        let close: Vec<f64> = (0..10).map(|day| 100.0 + day as f64).collect();
        let open: Vec<f64> = close.iter().map(|value| value - 1.0).collect();
        let high: Vec<f64> = close.iter().map(|value| value + 2.0).collect();
        let low: Vec<f64> = close.iter().map(|value| value - 2.0).collect();
        chart
            .set_series_data(0, &times, &open, &high, &low, &close)
            .unwrap();
        chart.time_scale.set_width(800.0);
        chart.fit_content();
        chart.series[0].price_line_visible = false;
        chart
    }

    fn delta_chart() -> ChartEngine {
        let mut chart = chart();
        chart.series[0].kind = SeriesKind::Area;
        chart
    }

    #[test]
    fn delta_tooltip_rejects_candlesticks_and_accepts_non_candlestick_series() {
        let mut candles = chart();
        assert!(candles
            .add_delta_tooltip(0, DeltaTooltipOptions::default())
            .is_none());
        assert!(candles.add_tooltip(0, TooltipOptions::default()).is_some());

        for kind in [
            SeriesKind::Bar,
            SeriesKind::Line,
            SeriesKind::Area,
            SeriesKind::Histogram,
            SeriesKind::Baseline,
            SeriesKind::Custom,
            SeriesKind::Feature,
            SeriesKind::Footprint,
        ] {
            let mut chart = chart();
            chart.series[0].kind = kind;
            assert!(
                chart
                    .add_delta_tooltip(0, DeltaTooltipOptions::default())
                    .is_some(),
                "delta tooltip should support {kind:?}"
            );
        }
    }

    /// The comparison tooltip's bordered box lands on whole device pixels, and its colored delta
    /// band spans exactly the box's rows, so all four borders stay one crisp device pixel.
    #[test]
    fn delta_tooltip_box_and_band_land_on_whole_device_pixels() {
        use aeris_charts_render::draw_list::Prim;
        for dpr in [1.0_f64, 1.25, 1.5, 1.75, 2.0, 2.5] {
            let mut chart = delta_chart();
            chart.dpr = dpr;
            chart
                .add_delta_tooltip(0, DeltaTooltipOptions::default())
                .unwrap();
            let x2 = chart.time_scale.index_to_coordinate(2) + 0.37;
            let x7 = chart.time_scale.index_to_coordinate(7) + 0.61;
            assert!(chart.delta_tooltip_mouse_down(x2));
            assert!(chart.delta_tooltip_mouse_move(x7));
            let frame = chart.build_frame();
            let prims = frame.panes.iter().flat_map(|pane| pane.main.iter());
            let mut boxes = Vec::new();
            let mut bands = Vec::new();
            for primitive in prims {
                match primitive {
                    Prim::RoundRect {
                        x,
                        y,
                        w,
                        h,
                        border_width,
                        ..
                    } if *border_width > 0.0 => boxes.push((*x, *y, *w, *h)),
                    Prim::Rect { rect, color } if color.a() == 51 => bands.push(*rect),
                    _ => {}
                }
            }
            let &(x, y, w, h) = boxes.first().expect("tooltip box");
            for edge in [x, y, x + w, y + h] {
                assert_eq!(edge.fract(), 0.0, "box {x},{y} {w}x{h} at dpr {dpr}");
            }
            let band = bands.first().expect("delta band");
            assert_eq!((band.y as f32, band.h as f32), (y, h), "dpr {dpr}");
        }
    }

    #[test]
    fn converting_a_delta_series_to_candlestick_removes_the_interaction() {
        let mut chart = delta_chart();
        let primitive = chart
            .add_delta_tooltip(0, DeltaTooltipOptions::default())
            .unwrap();
        assert!(chart.has_delta_tooltip());

        let x2 = chart.time_scale.index_to_coordinate(2);
        let x7 = chart.time_scale.index_to_coordinate(7);
        assert!(chart.delta_tooltip_mouse_down(x2));
        assert!(chart.delta_tooltip_mouse_move(x7));
        assert!(chart.delta_tooltip_active_range(primitive).is_some());

        chart.convert_series_kind(0, SeriesKind::Candlestick);
        assert!(!chart.has_delta_tooltip());
        assert_eq!(chart.delta_tooltip_active_range(primitive), None);
        assert!(!chart.delta_tooltip_mouse_down(x2));
        assert!(!chart.delta_tooltip_mouse_move(x7));
    }

    fn profile() -> VolumeProfileData {
        VolumeProfileData {
            time: 2 * 86_400,
            profile: vec![
                VolumeProfilePoint {
                    price: 95.0,
                    volume: 4.0,
                },
                VolumeProfilePoint {
                    price: 100.0,
                    volume: 10.0,
                },
                VolumeProfilePoint {
                    price: 105.0,
                    volume: 6.0,
                },
            ],
            width: 3.0,
        }
    }

    #[test]
    fn official_primitives_emit_shared_underlay_and_series_geometry() {
        let mut chart = chart();
        chart
            .add_session_highlighting(0, SessionHighlightingOptions::default())
            .unwrap();
        let highlight = Color::rgba(10, 20, 30, 51);
        chart
            .add_highlight_bar_crosshair(0, Some(highlight))
            .unwrap();
        let profile_options = VolumeProfileOptions::default();
        chart
            .add_volume_profile(0, profile(), profile_options)
            .unwrap();
        let x = chart.time_scale.index_to_coordinate(4);
        chart.set_crosshair_at(x, 250.0);

        let frame = chart.build_frame();
        let pane = &frame.panes[0];
        assert!(pane.under.iter().any(|primitive| matches!(
            primitive,
            Prim::Rect { color, .. } if *color == SessionHighlightingOptions::default().weekday_color
        )));
        assert!(pane.under.iter().any(|primitive| matches!(
            primitive,
            Prim::Rect { color, .. } if *color == SessionHighlightingOptions::default().weekend_color
        )));
        assert!(pane
            .under
            .iter()
            .any(|primitive| matches!(primitive, Prim::Rect { color, .. } if *color == highlight)));
        assert!(pane.main.iter().any(|primitive| matches!(
            primitive,
            Prim::Rect { color, .. } if *color == profile_options.background_color
        )));
        assert!(pane.main.iter().any(|primitive| matches!(
            primitive,
            Prim::Rect { color, .. } if *color == profile_options.row_color
        )));
    }

    #[test]
    fn implicit_crosshair_highlight_tracks_the_engine_surface_and_explicit_color_wins() {
        fn has_color(chart: &mut ChartEngine, expected: Color) -> bool {
            chart.build_frame().panes[0].under.iter().any(
                |primitive| matches!(primitive, Prim::Rect { color, .. } if *color == expected),
            )
        }

        let mut implicit = chart();
        implicit.add_highlight_bar_crosshair(0, None).unwrap();
        let x = implicit.time_scale.index_to_coordinate(4);
        implicit.set_crosshair_at(x, 250.0);
        assert!(has_color(&mut implicit, Color::rgba(255, 255, 255, 31)));
        implicit
            .apply_options(r##"{"layout":{"background":{"color":"#ffffff"}}}"##)
            .unwrap();
        assert!(has_color(&mut implicit, Color::rgba(0, 0, 0, 51)));

        let explicit_color = Color::rgba(12, 34, 56, 78);
        let mut explicit = chart();
        explicit
            .add_highlight_bar_crosshair(0, Some(explicit_color))
            .unwrap();
        let x = explicit.time_scale.index_to_coordinate(4);
        explicit.set_crosshair_at(x, 250.0);
        explicit
            .apply_options(r##"{"layout":{"background":{"color":"#ffffff"}}}"##)
            .unwrap();
        assert!(has_color(&mut explicit, explicit_color));
    }

    #[test]
    fn implicit_tooltip_guides_track_the_engine_surface() {
        fn has_tooltip_guide(chart: &mut ChartEngine, expected: Color) -> bool {
            chart.build_frame().panes[0].under.iter().any(
                |primitive| matches!(primitive, Prim::Rect { color, .. } if *color == expected),
            )
        }

        fn delta_guide_count(chart: &mut ChartEngine, expected: Color) -> usize {
            chart.build_frame().panes[0]
                .main
                .iter()
                .filter(|primitive| {
                    matches!(primitive, Prim::VLine { color, .. } if *color == expected)
                })
                .count()
        }

        let mut chart = delta_chart();
        chart.add_tooltip(0, TooltipOptions::default()).unwrap();
        chart
            .add_delta_tooltip(0, DeltaTooltipOptions::default())
            .unwrap();
        let x2 = chart.time_scale.index_to_coordinate(2);
        let x7 = chart.time_scale.index_to_coordinate(7);
        chart.set_crosshair_at(x2, 250.0);
        assert!(chart.delta_tooltip_mouse_down(x2));
        assert!(chart.delta_tooltip_mouse_move(x7));

        let dark_guide = Color::rgba(255, 255, 255, 31);
        assert!(has_tooltip_guide(&mut chart, dark_guide));
        assert_eq!(delta_guide_count(&mut chart, dark_guide), 2);

        chart
            .apply_options(r##"{"layout":{"background":{"color":"#ffffff"}}}"##)
            .unwrap();
        let light_guide = Color::rgba(0, 0, 0, 51);
        assert!(has_tooltip_guide(&mut chart, light_guide));
        assert_eq!(delta_guide_count(&mut chart, light_guide), 2);
    }

    #[test]
    fn shift_drag_delta_tooltip_preserves_normal_drag_and_owns_its_guides() {
        let mut chart = chart();
        chart.convert_series_kind(0, crate::SeriesKind::Area);
        chart.time_scale.set_width(800.0);
        chart.fit_content();

        let guide_color = Color::rgb(12, 34, 56);
        let primitive = chart
            .add_delta_tooltip(
                0,
                DeltaTooltipOptions {
                    line_color: Some(guide_color),
                    requires_shift_drag: true,
                    ..DeltaTooltipOptions::default()
                },
            )
            .unwrap();
        let x2 = chart.time_scale.index_to_coordinate(2);
        let x7 = chart.time_scale.index_to_coordinate(7);
        chart.set_crosshair_at(x7, 250.0);

        assert!(!chart.delta_tooltip_mouse_down(x2));
        assert!(chart.delta_tooltip_mouse_move(x7));
        assert!(chart.delta_tooltip_mouse_up());
        assert!(chart.delta_tooltip_active_range(primitive).is_none());
        assert!(!chart.delta_tooltip_touch_move(&[x2, x7]));
        assert!(chart.delta_tooltip_active_range(primitive).is_none());

        assert!(chart.delta_tooltip_mouse_down_with_shift(x2, true));
        assert!(chart.delta_tooltip_mouse_move(x7));
        assert!(chart.delta_tooltip_mouse_up());
        assert!(chart.delta_tooltip_active_range(primitive).is_some());

        let frame = chart.build_frame();
        assert_eq!(
            frame.panes[0]
                .main
                .iter()
                .filter(|primitive| matches!(
                    primitive,
                    Prim::VLine { color, .. } if *color == guide_color
                ))
                .count(),
            2,
            "the delta tooltip owns both comparison guides without mutating chart crosshair options",
        );
    }

    #[test]
    fn bands_indicator_uses_official_ten_percent_data_background_and_visible_autoscale() {
        let mut chart = chart();
        let options = BandsIndicatorOptions::default();
        let id = chart.add_bands_indicator(0, options).unwrap();
        let frame = chart.build_frame();
        let pane = &frame.panes[0];
        assert!(matches!(
            pane.main.as_slice(),
            [
                Prim::Polyline { color: upper, width: upper_width, .. },
                Prim::Polyline { color: lower, width: lower_width, .. },
                Prim::BandFill { fill, point_count: 10, .. },
                ..
            ] if *upper == options.line_color
                && *lower == options.line_color
                && *fill == options.fill_color
                && *upper_width == 1.0
                && *lower_width == 1.0
        ));
        let Prim::BandFill {
            upper_first,
            lower_first,
            point_count,
            ..
        } = &pane.main[2]
        else {
            unreachable!();
        };
        assert_eq!(*point_count, 10);
        let upper = pane.points[*upper_first as usize];
        let lower = pane.points[*lower_first as usize];
        assert!((upper[0] - chart.time_scale.index_to_coordinate(0) as f32).abs() < 1e-4);
        assert!(
            (upper[1] - chart.series_price_to_coordinate(0, 110.0).unwrap() as f32).abs() < 1e-4
        );
        assert!(
            (lower[1] - chart.series_price_to_coordinate(0, 90.0).unwrap() as f32).abs() < 1e-4
        );
        let range = chart.panes[0].price_scale.price_range().unwrap();
        assert!(range.min_value() <= 90.0);
        assert!(range.max_value() >= 109.0 * 1.1);

        let updated = BandsIndicatorOptions {
            line_color: Color::rgb(1, 2, 3),
            fill_color: Color::rgba(4, 5, 6, 70),
            line_width: 3.0,
        };
        assert!(chart.set_bands_indicator_options(id, updated));
        assert!(!chart.set_bands_indicator_options(
            id,
            BandsIndicatorOptions {
                line_width: f64::NAN,
                ..updated
            }
        ));
        let frame = chart.build_frame();
        assert!(matches!(
            frame.panes[0].main.as_slice(),
            [Prim::Polyline { color, width, .. }, ..]
                if *color == updated.line_color && *width == 3.0
        ));
    }

    #[test]
    fn overlay_price_scale_emits_unboxed_in_pane_text_and_updates_side() {
        let mut chart = chart();
        chart.set_series_price_scale(0, crate::PriceScaleTarget::Overlay);
        let defaults = OverlayPriceScaleOptions::default();
        assert_eq!(defaults.text_color, None);
        let id = chart.add_overlay_price_scale(0, defaults).unwrap();
        let frame = chart.build_frame();
        assert!(!frame.panes[0]
            .main
            .iter()
            .any(|primitive| matches!(primitive, Prim::RoundRect { .. })));
        let dark_text = Color::parse_css(aeris_charts_core::style::DARK_FOREGROUND_CSS).unwrap();
        let labels = frame.panes[0]
            .main
            .iter()
            .filter(|primitive| {
                matches!(primitive, Prim::Text { color, size, .. }
                    if *color == dark_text && *size == 12.0)
            })
            .count();
        assert_eq!(labels, 13);

        chart
            .apply_options(&format!(
                r##"{{"layout":{{"textColor":"{}"}}}}"##,
                aeris_charts_core::style::LIGHT_FOREGROUND_CSS
            ))
            .unwrap();
        let light_text = Color::parse_css(aeris_charts_core::style::LIGHT_FOREGROUND_CSS).unwrap();
        assert_eq!(
            chart.build_frame().panes[0]
                .main
                .iter()
                .filter(|primitive| matches!(
                    primitive,
                    Prim::Text { color, size, .. } if *color == light_text && *size == 12.0
                ))
                .count(),
            13
        );

        let right = OverlayPriceScaleOptions {
            text_color: Some(Color::rgb(1, 2, 3)),
            side: OverlayPriceScaleSide::Right,
        };
        assert!(chart.set_overlay_price_scale_options(id, right));
        let frame = chart.build_frame();
        assert!(frame.panes[0].main.iter().any(|primitive| {
            matches!(primitive, Prim::Text { x, color, .. }
                if Some(*color) == right.text_color && *x > 700.0)
        }));

        chart
            .apply_options(r##"{"layout":{"textColor":"#f0f0f0"}}"##)
            .unwrap();
        assert!(chart.build_frame().panes[0].main.iter().any(|primitive| {
            matches!(primitive, Prim::Text { color, .. } if Some(*color) == right.text_color)
        }));
    }

    #[test]
    fn session_highlighting_retains_exact_callback_colors_aligned_to_source_times() {
        let mut chart = chart();
        let primitive = chart
            .add_session_highlighting(0, SessionHighlightingOptions::default())
            .unwrap();
        let first = Color::rgba(1, 2, 3, 40);
        let second = Color::rgba(4, 5, 6, 50);
        let highlights = (0..10)
            .map(|day| SessionHighlightingData {
                time: day * 86_400,
                color: if day % 2 == 0 { first } else { second },
            })
            .collect();
        assert!(chart.set_session_highlighting_data(primitive, highlights));
        let frame = chart.build_frame();
        assert!(frame.panes[0]
            .under
            .iter()
            .any(|primitive| matches!(primitive, Prim::Rect { color, .. } if *color == first)));
        assert!(frame.panes[0]
            .under
            .iter()
            .any(|primitive| matches!(primitive, Prim::Rect { color, .. } if *color == second)));
        assert!(!chart.set_session_highlighting_data(
            primitive,
            vec![SessionHighlightingData {
                time: 86_400,
                color: first,
            }],
        ));
    }

    #[test]
    fn session_highlighting_gate_uses_fractional_exchange_hours_and_local_weekends() {
        // 2024-01-05 (Friday) 30-minute bars from 13:00 to 21:30 UTC.
        let base = 1_704_459_600;
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        let times: Vec<f64> = (0..18).map(|i| (base + i * 1_800) as f64).collect();
        let values = vec![100.0; times.len()];
        chart
            .set_series_data(0, &times, &values, &values, &values, &values)
            .unwrap();
        chart.time_scale.set_width(800.0);
        chart.fit_content();
        let weekday = Color::rgba(1, 2, 3, 40);
        let weekend = Color::rgba(4, 5, 6, 50);
        // Fractional hours are no longer dropped: 09:30..16:00 is a valid gate.
        assert!(chart
            .add_session_highlighting(
                0,
                SessionHighlightingOptions {
                    start_hour: Some(9.5),
                    end_hour: Some(16.0),
                    weekday_color: weekday,
                    weekend_color: weekend,
                },
            )
            .is_some());
        assert!(chart
            .add_session_highlighting(
                0,
                SessionHighlightingOptions {
                    start_hour: Some(9.5),
                    end_hour: None,
                    ..SessionHighlightingOptions::default()
                },
            )
            .is_none());
        let shaded = |chart: &mut ChartEngine, color: Color| -> i32 {
            chart.build_frame().panes[0]
                .under
                .iter()
                .filter_map(|primitive| match primitive {
                    Prim::Rect { rect, color: c } if *c == color => Some(rect.w),
                    _ => None,
                })
                .sum()
        };
        // In UTC the 09:30..16:00 gate covers the 13:00..15:30 UTC bars; in New York (-5h) the
        // same exchange-local gate covers 14:30..20:30 UTC.
        let utc_width = shaded(&mut chart, weekday);
        chart.set_exchange_offsets(crate::UtcOffsetSchedule::fixed(-5 * 3_600).unwrap());
        let eastern_width = shaded(&mut chart, weekday);
        assert!(utc_width > 0 && eastern_width > utc_width);
        assert_eq!(shaded(&mut chart, weekend), 0);

        // In Tokyo (+9h) the Friday 15:00+ UTC bars are already Saturday: weekend color.
        let mut all_day = ChartEngine::new(800.0, 500.0, 1.0);
        all_day
            .set_series_data(0, &times, &values, &values, &values, &values)
            .unwrap();
        all_day.time_scale.set_width(800.0);
        all_day.fit_content();
        all_day.set_exchange_offsets(crate::UtcOffsetSchedule::fixed(9 * 3_600).unwrap());
        all_day
            .add_session_highlighting(
                0,
                SessionHighlightingOptions {
                    weekday_color: weekday,
                    weekend_color: weekend,
                    ..SessionHighlightingOptions::default()
                },
            )
            .unwrap();
        assert!(shaded(&mut all_day, weekend) > 0);
        assert!(shaded(&mut all_day, weekday) > 0);
    }

    #[test]
    fn session_highlighting_tail_upsert_touches_only_changed_rows() {
        let mut chart = chart();
        let primitive = chart
            .add_session_highlighting(0, SessionHighlightingOptions::default())
            .unwrap();
        let color = Color::rgba(1, 2, 3, 40);
        let replaced = Color::rgba(7, 8, 9, 40);
        let records = |days: std::ops::Range<i64>, color: Color| {
            days.map(|day| SessionHighlightingData {
                time: day * 86_400,
                color,
            })
            .collect::<Vec<_>>()
        };
        // Tail merge needs a full baseline first.
        assert!(!chart.upsert_session_highlighting_data(primitive, records(9..10, color)));
        assert!(chart.set_session_highlighting_data(primitive, records(0..10, color)));
        assert_eq!(chart.series_times_since(0, 9 * 86_400), &[9 * 86_400]);

        // A live append: the source grows by one row and only that row is sent.
        assert!(chart.update_series_bar(0, 10.0 * 86_400.0, [1.0, 2.0, 0.5, 1.5]));
        assert_eq!(
            chart.series_times_since(0, 9 * 86_400),
            &[9 * 86_400, 10 * 86_400]
        );
        assert!(chart.upsert_session_highlighting_data(primitive, records(10..11, replaced)));
        // Replacing the current bar keeps the alignment.
        assert!(chart.upsert_session_highlighting_data(primitive, records(10..11, color)));
        // Unaligned tails (a row the source does not have) are rejected without mutation.
        assert!(!chart.upsert_session_highlighting_data(primitive, records(11..12, color)));
        assert!(!chart.upsert_session_highlighting_data(
            primitive,
            vec![
                SessionHighlightingData {
                    time: 10 * 86_400,
                    color,
                },
                SessionHighlightingData {
                    time: 9 * 86_400,
                    color,
                },
            ],
        ));
        let frame = chart.build_frame();
        assert!(frame.panes[0]
            .under
            .iter()
            .any(|primitive| matches!(primitive, Prim::Rect { color: c, .. } if *c == color)));

        // `max_points` retention evicts the oldest source row on the next append. The tail merge
        // drops that row's record instead of forcing a full callback re-evaluation.
        let retained = |chart: &ChartEngine| -> (usize, Option<i64>) {
            chart.series[0]
                .native_primitives
                .iter()
                .find_map(|entry| match &entry.kind {
                    NativeSeriesPrimitiveKind::SessionHighlighting(state)
                        if entry.id == primitive =>
                    {
                        state.highlights.as_ref().map(|highlights| {
                            (
                                highlights.len(),
                                highlights.first().map(|highlight| highlight.time),
                            )
                        })
                    }
                    _ => None,
                })
                .unwrap()
        };
        assert!(chart.set_series_max_points(0, Some(11)));
        assert!(chart.update_series_bar(0, 11.0 * 86_400.0, [1.0, 2.0, 0.5, 1.5]));
        assert_eq!(chart.series_times_since(0, 0)[0], 86_400);
        // A record older than the retained source is never merged.
        assert!(!chart.upsert_session_highlighting_data(primitive, records(0..1, color)));
        assert_eq!(retained(&chart), (11, Some(0)));
        assert!(chart.upsert_session_highlighting_data(primitive, records(11..12, color)));
        assert_eq!(retained(&chart), (11, Some(86_400)));
    }

    #[test]
    fn volume_profile_validation_is_transactional_and_memory_is_attributed() {
        let mut chart = chart();
        let id = chart
            .add_volume_profile(0, profile(), VolumeProfileOptions::default())
            .unwrap();
        let before = chart.build_frame();
        let malformed = VolumeProfileData {
            time: 0,
            profile: vec![VolumeProfilePoint {
                price: f64::NAN,
                volume: 1.0,
            }],
            width: 1.0,
        };
        assert!(!chart.set_volume_profile_data(id, malformed));
        assert_eq!(before, chart.build_frame());
        assert!(chart.memory_usage().native_primitive_capacity_bytes > 0);
        assert!(chart.remove_native_primitive(id));
        assert!(!chart.remove_native_primitive(id));
    }

    #[test]
    fn delta_tooltip_owns_pointer_state_sorted_range_and_official_frame_content() {
        let mut chart = delta_chart();
        let guide_color = Color::rgb(12, 34, 56);
        let options = DeltaTooltipOptions {
            line_color: Some(guide_color),
            ..DeltaTooltipOptions::default()
        };
        let primitive = chart.add_delta_tooltip(0, options).unwrap();
        let x2 = chart.time_scale.index_to_coordinate(2);
        let x7 = chart.time_scale.index_to_coordinate(7);

        assert!(chart.delta_tooltip_mouse_down(x7));
        assert!(chart.delta_tooltip_mouse_move(x2));
        assert_eq!(
            chart.delta_tooltip_active_range(primitive),
            Some(DeltaTooltipActiveRange {
                from: 3,
                to: 8,
                positive: true,
            })
        );
        let frame = chart.build_frame();
        assert_eq!(
            frame.panes[0]
                .main
                .iter()
                .filter(|primitive| matches!(
                    primitive,
                    Prim::VLine { color, .. } if *color == guide_color
                ))
                .count(),
            2
        );
        assert!(frame.panes[0].main.iter().any(|primitive| matches!(
            primitive,
            Prim::Circle { radius, .. } if (*radius - 6.0).abs() < f32::EPSILON
        )));
        let tooltip_boxes: Vec<_> = frame.panes[0]
            .main
            .iter()
            .filter_map(|primitive| {
                let Prim::RoundRect {
                    fill,
                    border_width,
                    border_color,
                    radii,
                    ..
                } = primitive
                else {
                    return None;
                };
                Some((*fill, *border_width, *border_color, *radii))
            })
            .collect();
        assert_eq!(tooltip_boxes.len(), 1, "tooltip must not emit a shadow box");
        assert_eq!(
            tooltip_boxes[0].0,
            Color::rgb(
                aeris_charts_core::style::DEFAULT_SURFACE_RGB.0,
                aeris_charts_core::style::DEFAULT_SURFACE_RGB.1,
                aeris_charts_core::style::DEFAULT_SURFACE_RGB.2,
            )
        );
        assert!(tooltip_boxes[0].1 > 0.0);
        assert_eq!(
            tooltip_boxes[0].2,
            Color::rgb(
                aeris_charts_core::style::DEFAULT_BORDER_RGB.0,
                aeris_charts_core::style::DEFAULT_BORDER_RGB.1,
                aeris_charts_core::style::DEFAULT_BORDER_RGB.2,
            )
        );
        assert!(tooltip_boxes[0].3.iter().all(|radius| *radius == 6.0));
        assert!(frame.panes[0].main.iter().any(|primitive| matches!(
            primitive,
            Prim::Rect { color, .. } if *color == Color::rgba(
                aeris_charts_core::style::MARKET_UP_RGB.0,
                aeris_charts_core::style::MARKET_UP_RGB.1,
                aeris_charts_core::style::MARKET_UP_RGB.2,
                51,
            )
        )));
        for expected in ["102.00", "107.00", "+5.00", "+4.90%"] {
            assert!(frame.panes[0].main.iter().any(|primitive| matches!(
                primitive,
                Prim::Text { text, .. } if text == expected
            )));
        }
        chart
            .apply_options(r##"{"layout":{"background":{"color":"#ffffff"}}}"##)
            .unwrap();
        assert!(chart.build_frame().panes[0].main.iter().any(|primitive| {
            matches!(primitive, Prim::VLine { color, .. } if *color == guide_color)
        }));

        // Mouse-up commits the comparison; later hover and leave must not clear it.
        assert!(chart.delta_tooltip_mouse_up());
        assert!(chart.delta_tooltip_active_range(primitive).is_some());
        chart.delta_tooltip_mouse_move(x2);
        assert!(chart.delta_tooltip_active_range(primitive).is_some());
        chart.delta_tooltip_leave();
        assert!(chart.delta_tooltip_active_range(primitive).is_some());

        // Touch order is normalized by logical index, matching the official active-range contract.
        chart.delta_tooltip_touch_move(&[x7, x2]);
        assert_eq!(
            chart.delta_tooltip_active_range(primitive),
            Some(DeltaTooltipActiveRange {
                from: 3,
                to: 8,
                positive: true,
            })
        );
        assert!(!chart.delta_tooltip_touch_move(&[x2, x7, x2]));
        assert!(chart.delta_tooltip_active_range(primitive).is_some());
        chart.delta_tooltip_leave();
        assert!(chart.delta_tooltip_active_range(primitive).is_some());
        assert!(chart.build_frame().panes[0].main.iter().any(|primitive| {
            matches!(primitive, Prim::VLine { color, .. } if *color == guide_color)
        }));

        assert!(chart.clear_delta_tooltip(primitive));
        assert!(chart.delta_tooltip_active_range(primitive).is_none());
        assert!(chart.build_frame().panes[0].main.iter().all(|primitive| {
            !matches!(primitive, Prim::VLine { color, .. } if *color == guide_color)
        }));
    }

    #[test]
    fn delta_tooltip_uses_the_series_price_format_exchange_time_and_host_formatter() {
        let mut chart = delta_chart();
        chart.series[0].price_format = crate::SeriesPriceFormat {
            precision: 3,
            min_move: 0.001,
            ..crate::SeriesPriceFormat::default()
        };
        // America/New_York winter offset: day 2 (1970-01-03 00:00 UTC) is 2 Jan 19:00 locally.
        chart.set_exchange_offsets(crate::UtcOffsetSchedule::fixed(-5 * 3_600).unwrap());
        chart
            .add_delta_tooltip(
                0,
                DeltaTooltipOptions {
                    show_time: true,
                    ..DeltaTooltipOptions::default()
                },
            )
            .unwrap();
        let x2 = chart.time_scale.index_to_coordinate(2);
        let x7 = chart.time_scale.index_to_coordinate(7);
        assert!(chart.delta_tooltip_mouse_down(x2));
        assert!(chart.delta_tooltip_mouse_move(x7));
        let texts = |chart: &mut ChartEngine| -> Vec<String> {
            chart.build_frame().panes[0]
                .main
                .iter()
                .filter_map(|primitive| match primitive {
                    Prim::Text { text, .. } => Some(text.clone()),
                    _ => None,
                })
                .collect()
        };
        let built_in = texts(&mut chart);
        for expected in ["102.000", "107.000", "+5.000", "02 Jan 1970", "19:00"] {
            assert!(
                built_in.iter().any(|text| text == expected),
                "missing {expected:?} in {built_in:?}"
            );
        }
        chart.set_time_formatter(Some(Box::new(|time| Some(format!("T{time}")))));
        let hosted = texts(&mut chart);
        assert!(hosted.iter().any(|text| text == "T172800"), "{hosted:?}");
        assert!(!hosted.iter().any(|text| text == "02 Jan 1970"));
    }

    #[test]
    fn delta_tooltip_guides_reproject_with_the_time_scale() {
        let mut chart = delta_chart();
        let guide_color = Color::rgb(12, 34, 56);
        let options = DeltaTooltipOptions {
            line_color: Some(guide_color),
            ..DeltaTooltipOptions::default()
        };
        let primitive = chart.add_delta_tooltip(0, options).unwrap();
        let first_index = 2;
        let second_index = 7;
        let spacing = chart.time_scale.bar_spacing();
        let first_pointer = chart.time_scale.index_to_coordinate(first_index) + spacing * 0.3;
        let second_pointer = chart.time_scale.index_to_coordinate(second_index) - spacing * 0.3;

        assert!(chart.delta_tooltip_mouse_down(first_pointer));
        assert!(chart.delta_tooltip_mouse_move(second_pointer));
        assert!(chart.delta_tooltip_mouse_up());

        let guide_xs = |chart: &mut ChartEngine| {
            let mut xs = chart.build_frame().panes[0]
                .main
                .iter()
                .filter_map(|primitive| match primitive {
                    Prim::VLine { x, color, .. } if *color == guide_color => Some(*x),
                    _ => None,
                })
                .collect::<Vec<_>>();
            xs.sort_unstable();
            xs
        };
        let expected_xs = |chart: &ChartEngine| {
            let mut xs = [
                chart.time_scale.index_to_coordinate(first_index).round() as i32,
                chart.time_scale.index_to_coordinate(second_index).round() as i32,
            ];
            xs.sort_unstable();
            xs
        };

        assert_eq!(guide_xs(&mut chart), expected_xs(&chart));

        chart.scroll_to_position(chart.scroll_position() - 2.0);
        assert_eq!(guide_xs(&mut chart), expected_xs(&chart));

        chart.css_width = 1_200.0;
        chart.recompute_layout_with_measure(true, |_, _| 0.0, |_, _| 0.0);
        assert_eq!(guide_xs(&mut chart), expected_xs(&chart));
        assert!(chart.delta_tooltip_active_range(primitive).is_some());
    }

    #[test]
    fn delta_tooltip_reads_runtime_chart_theme_and_font_options() {
        let mut chart = delta_chart();
        let primitive = chart
            .add_delta_tooltip(0, DeltaTooltipOptions::default())
            .unwrap();
        let x2 = chart.time_scale.index_to_coordinate(2);
        let x7 = chart.time_scale.index_to_coordinate(7);
        assert!(chart.delta_tooltip_mouse_down(x2));
        assert!(chart.delta_tooltip_mouse_move(x7));
        assert!(chart.delta_tooltip_mouse_up());
        chart
            .apply_options(
                r##"{
                    "layout": {
                        "background": { "color": "#112233" },
                        "textColor": "#ddeeff",
                        "mutedTextColor": "#778899",
                        "fontSize": 15,
                        "fontFamily": "Theme Test"
                    },
                    "rightPriceScale": { "borderColor": "#445566" }
                }"##,
            )
            .unwrap();

        let frame = chart.build_frame();
        assert!(frame.panes[0].main.iter().any(|primitive| matches!(
            primitive,
            Prim::RoundRect { fill, border_color, radii, .. }
                if *fill == Color::rgb(0x11, 0x22, 0x33)
                    && *border_color == Color::rgb(0x44, 0x55, 0x66)
                    && radii.iter().all(|radius| *radius == 6.0)
        )));
        assert!(frame.panes[0].main.iter().any(|primitive| matches!(
            primitive,
            Prim::Text { color, family, size, .. }
                if *color == Color::rgb(0xdd, 0xee, 0xff)
                    && family == "Theme Test"
                    && (*size - 17.0).abs() < f32::EPSILON
        )));
        assert!(frame.panes[0].main.iter().any(|primitive| matches!(
            primitive,
            Prim::Text { color, family, size, .. }
                if *color == Color::rgb(0x77, 0x88, 0x99)
                    && family == "Theme Test"
                    && (*size - 15.0).abs() < f32::EPSILON
        )));
        assert!(chart.clear_delta_tooltip(primitive));
    }

    #[test]
    fn delta_tooltip_replaces_only_with_complete_mouse_or_touch_ranges() {
        let mut chart = delta_chart();
        let primitive = chart
            .add_delta_tooltip(0, DeltaTooltipOptions::default())
            .unwrap();
        let x1 = chart.time_scale.index_to_coordinate(1);
        let x2 = chart.time_scale.index_to_coordinate(2);
        let x4 = chart.time_scale.index_to_coordinate(4);
        let x5 = chart.time_scale.index_to_coordinate(5);
        let x7 = chart.time_scale.index_to_coordinate(7);
        let x8 = chart.time_scale.index_to_coordinate(8);

        assert!(chart.delta_tooltip_mouse_down(x2));
        assert!(chart.delta_tooltip_mouse_move(x7));
        assert!(chart.delta_tooltip_mouse_up());
        let first = chart.delta_tooltip_active_range(primitive).unwrap();

        // A click or cancelled replacement does not erase the committed range.
        assert!(chart.delta_tooltip_mouse_down(x4));
        chart.delta_tooltip_mouse_up();
        assert_eq!(chart.delta_tooltip_active_range(primitive), Some(first));

        assert!(chart.delta_tooltip_mouse_down(x1));
        assert!(chart.delta_tooltip_mouse_move(x5));
        assert!(chart.delta_tooltip_mouse_up());
        let replacement = chart.delta_tooltip_active_range(primitive).unwrap();
        assert_ne!(replacement, first);

        // Two touches commit immediately. Movement by the sole survivor is only a preview.
        assert!(chart.delta_tooltip_touch_move(&[x7, x8]));
        let touch_range = chart.delta_tooltip_active_range(primitive).unwrap();
        assert_ne!(touch_range, replacement);
        assert!(chart.delta_tooltip_touch_move(&[x4]));
        assert_eq!(
            chart.delta_tooltip_active_range(primitive),
            Some(touch_range)
        );
        chart.delta_tooltip_leave();
        assert_eq!(
            chart.delta_tooltip_active_range(primitive),
            Some(touch_range)
        );

        assert!(chart.remove_native_primitive(primitive));
        assert_eq!(chart.delta_tooltip_active_range(primitive), None);
    }

    #[test]
    fn delta_tooltip_direction_uses_chronological_prices_not_drag_order() {
        let mut chart = delta_chart();
        let times: Vec<f64> = (0..10).map(|day| day as f64 * 86_400.0).collect();
        let close: Vec<f64> = (0..10).map(|day| 110.0 - day as f64).collect();
        let open: Vec<f64> = close.iter().map(|value| value + 1.0).collect();
        let high: Vec<f64> = close.iter().map(|value| value + 2.0).collect();
        let low: Vec<f64> = close.iter().map(|value| value - 2.0).collect();
        chart
            .set_series_data(0, &times, &open, &high, &low, &close)
            .unwrap();
        chart.fit_content();
        let primitive = chart
            .add_delta_tooltip(0, DeltaTooltipOptions::default())
            .unwrap();
        let x2 = chart.time_scale.index_to_coordinate(2);
        let x7 = chart.time_scale.index_to_coordinate(7);

        for (start, end) in [(x2, x7), (x7, x2)] {
            assert!(chart.delta_tooltip_mouse_down(start));
            assert!(chart.delta_tooltip_mouse_move(end));
            assert_eq!(
                chart.delta_tooltip_active_range(primitive),
                Some(DeltaTooltipActiveRange {
                    from: 3,
                    to: 8,
                    positive: false,
                })
            );
            assert!(chart.delta_tooltip_mouse_up());
        }

        chart.delta_tooltip_touch_move(&[x7, x2]);
        assert_eq!(
            chart.delta_tooltip_active_range(primitive),
            Some(DeltaTooltipActiveRange {
                from: 3,
                to: 8,
                positive: false,
            })
        );
    }

    #[test]
    fn tooltip_snapshot_and_bottom_guide_are_owned_by_the_engine() {
        let mut chart = chart();
        let color = Color::rgb(12, 34, 56);
        let primitive = chart
            .add_tooltip(
                0,
                TooltipOptions {
                    line_color: Some(color),
                    top_margin: 30.0,
                },
            )
            .unwrap();
        let _ = chart.build_frame();
        let x = chart.time_scale.index_to_coordinate(4);
        chart.set_crosshair_at(x, 200.0);
        assert_eq!(
            chart.tooltip_snapshot(primitive),
            Some(TooltipSnapshot {
                x,
                index: 4,
                price: 104.0,
                open: 103.0,
                high: 106.0,
                low: 102.0,
                close: 104.0,
                time: 4 * 86_400,
            })
        );
        assert!(chart.build_frame().panes[0]
            .under
            .iter()
            .any(|primitive| matches!(
                primitive,
                Prim::Rect { rect, color: actual }
                    if *actual == color && rect.y == 30 && rect.h == 470
            )));
        chart
            .apply_options(r##"{"layout":{"background":{"color":"#ffffff"}}}"##)
            .unwrap();
        assert!(chart.build_frame().panes[0]
            .under
            .iter()
            .any(|primitive| matches!(
                primitive,
                Prim::Rect { color: actual, .. } if *actual == color
            )));
        assert!(chart.set_tooltip_options(primitive, TooltipOptions::default()));
        chart.clear_crosshair_at();
        assert!(chart.tooltip_snapshot(primitive).is_none());
        assert!(chart.build_frame().panes[0].under.iter().all(
            |primitive| !matches!(primitive, Prim::Rect { color: actual, .. } if *actual == color)
        ));
    }

    #[test]
    fn tooltip_snapshot_keeps_ohlc_for_scalar_presentations() {
        let mut chart = chart();
        chart.convert_series_kind(0, SeriesKind::Area);
        let primitive = chart.add_tooltip(0, TooltipOptions::default()).unwrap();
        let _ = chart.build_frame();
        let x = chart.time_scale.index_to_coordinate(4);
        chart.set_crosshair_at(x, 200.0);
        let snapshot = chart.tooltip_snapshot(primitive).unwrap();
        assert_eq!(snapshot.open, 103.0);
        assert_eq!(snapshot.high, 106.0);
        assert_eq!(snapshot.low, 102.0);
        assert_eq!(snapshot.close, 104.0);

        chart.convert_series_kind(0, SeriesKind::Line);
        let _ = chart.build_frame();
        chart.set_crosshair_at(x, 200.0);
        let snapshot = chart.tooltip_snapshot(primitive).unwrap();
        assert_eq!(snapshot.open, 103.0);
        assert_eq!(snapshot.high, 106.0);
        assert_eq!(snapshot.low, 102.0);
        assert_eq!(snapshot.close, 104.0);

        let scalar: Vec<f64> = (0..10).map(|day| 200.0 + day as f64).collect();
        let times: Vec<f64> = (0..10).map(|day| day as f64 * 86_400.0).collect();
        chart
            .set_series_data(0, &times, &scalar, &scalar, &scalar, &scalar)
            .unwrap();
        let _ = chart.build_frame();
        chart.set_crosshair_at(x, 200.0);
        let snapshot = chart.tooltip_snapshot(primitive).unwrap();
        assert_eq!(snapshot.open, 204.0);
        assert_eq!(snapshot.high, 204.0);
        assert_eq!(snapshot.low, 204.0);
        assert_eq!(snapshot.close, 204.0);
    }

    #[test]
    fn accessibility_focus_ring_uses_exact_engine_data_and_shared_overlay_geometry() {
        let mut chart = chart();
        let options = AccessibilityFocusOptions {
            color: Color::rgb(12, 34, 56),
            size: 14.0,
            high_contrast: false,
        };
        let primitive = chart.add_accessibility_focus(0, options).unwrap();
        let _ = chart.build_frame();
        assert!(chart.set_accessibility_focus(primitive, Some(4 * 86_400), options));
        let frame = chart.build_frame();
        assert!(frame.panes[0].main.iter().any(|primitive| matches!(
            primitive,
            Prim::Circle { radius, stroke, stroke_width, .. }
                if *stroke == options.color
                    && (*radius - 7.0).abs() < f32::EPSILON
                    && (*stroke_width - 2.0).abs() < f32::EPSILON
        )));
        assert!(chart.set_accessibility_focus(primitive, None, options));
        assert!(chart.build_frame().panes[0]
            .main
            .iter()
            .all(|primitive| !matches!(primitive, Prim::Circle { stroke, .. } if *stroke == options.color)));
    }

    #[test]
    fn image_watermark_is_centered_aspect_preserving_bounded_and_series_owned() {
        let mut chart = chart();
        let pixels = Arc::<[u8]>::from(vec![255; 4 * 2 * 4]);
        let id = chart
            .add_image_watermark(
                0,
                4,
                2,
                Arc::clone(&pixels),
                ImageWatermarkOptions {
                    max_width: Some(200.0),
                    max_height: Some(100.0),
                    padding: 8.0,
                    alpha: 0.4,
                },
            )
            .unwrap();
        let frame = chart.build_frame();
        let image = frame.panes[0]
            .under
            .iter()
            .find_map(|primitive| match primitive {
                Prim::Image {
                    image,
                    rect,
                    opacity,
                } => Some((image, rect, opacity)),
                _ => None,
            })
            .expect("watermark must be emitted through the shared underlay");
        assert_eq!(image.0.width, 4);
        assert_eq!(image.0.height, 2);
        assert_eq!(image.1[2], 200.0);
        assert_eq!(image.1[3], 100.0);
        assert_eq!(*image.2, 0.4);
        assert!((image.1[0] - 300.0).abs() < f32::EPSILON);
        assert!(chart.memory_usage().native_primitive_capacity_bytes >= pixels.len());

        chart.pane_set_preserve_empty(0, true);
        let second_pane = chart.add_pane(true).unwrap();
        chart.set_series_pane(0, second_pane, 1.0);
        let moved = chart.build_frame();
        assert!(moved.panes[0]
            .under
            .iter()
            .all(|primitive| !matches!(primitive, Prim::Image { .. })));
        assert!(moved.panes[second_pane]
            .under
            .iter()
            .any(|primitive| matches!(primitive, Prim::Image { .. })));
        assert!(chart.remove_native_primitive(id));

        assert!(chart
            .add_image_watermark(
                0,
                MAX_RASTER_IMAGE_DIMENSION + 1,
                1,
                Arc::<[u8]>::from(vec![0; 4]),
                ImageWatermarkOptions::default(),
            )
            .is_none());
        assert!(chart.build_frame().panes[0]
            .under
            .iter()
            .all(|primitive| !matches!(primitive, Prim::Image { .. })));
    }

    #[test]
    fn anchored_text_matches_viewport_alignment_updates_and_follows_its_series() {
        let mut chart = chart();
        let options = AnchoredTextOptions {
            horizontal_align: AnchoredTextHorizontalAlign::Middle,
            vertical_align: AnchoredTextVerticalAlign::Middle,
            text: "Anchored Text".into(),
            line_height: 54.0,
            font_size: 54.0,
            font_family: "Arial".into(),
            font_weight: 700,
            italic: true,
            color: Color::rgb(255, 0, 0),
        };
        let id = chart.add_anchored_text(0, options.clone()).unwrap();
        let frame = chart.build_frame();
        let text = frame.panes[0]
            .main
            .iter()
            .find_map(|primitive| match primitive {
                Prim::Text {
                    x,
                    y,
                    text,
                    align,
                    weight,
                    italic,
                    ..
                } if text == "Anchored Text" => Some((*x, *y, *align, *weight, *italic)),
                _ => None,
            })
            .expect("anchored text must be emitted into the shared pane frame");
        assert_eq!(text.0, 400.0);
        assert_eq!(text.1, 250.0);
        assert_eq!(text.2, aeris_charts_render::draw_list::TextAlign::Center);
        assert_eq!(text.3, 700);
        assert!(text.4);

        chart.pane_set_preserve_empty(0, true);
        let second_pane = chart.add_pane(true).unwrap();
        chart.set_series_pane(0, second_pane, 1.0);
        let moved = chart.build_frame();
        assert!(moved.panes[0].main.iter().all(
            |primitive| !matches!(primitive, Prim::Text { text, .. } if text == "Anchored Text")
        ));
        assert!(moved.panes[second_pane].main.iter().any(
            |primitive| matches!(primitive, Prim::Text { text, .. } if text == "Anchored Text")
        ));
        chart.set_series_pane(0, 0, 1.0);
        assert!(chart.remove_pane(second_pane));

        let before = chart.build_frame();
        let mut malformed = options;
        malformed.font_size = f64::NAN;
        assert!(!chart.set_anchored_text_options(id, malformed));
        assert_eq!(before, chart.build_frame());

        assert!(chart.set_anchored_text_options(
            id,
            AnchoredTextOptions {
                horizontal_align: AnchoredTextHorizontalAlign::Right,
                vertical_align: AnchoredTextVerticalAlign::Bottom,
                text: "Updated".into(),
                line_height: 20.0,
                font_size: 18.0,
                font_family: "Arial".into(),
                font_weight: 400,
                italic: false,
                color: Color::rgb(0, 0, 0),
            },
        ));
        assert!(chart.build_frame().panes[0].main.iter().any(|primitive| {
            matches!(primitive, Prim::Text { x, y, text, align, .. }
                if text == "Updated" && *x == 780.0 && *y == 480.0
                    && *align == aeris_charts_render::draw_list::TextAlign::Right)
        }));
    }

    #[test]
    fn vertical_line_emits_exact_pane_slot_and_time_axis_label() {
        let mut chart = chart();
        let color = Color::rgb(10, 20, 30);
        let label_background = Color::rgb(40, 50, 60);
        let label_text = Color::rgb(250, 251, 252);
        chart
            .add_vertical_line(
                0,
                4 * 86_400,
                VerticalLineOptions {
                    color,
                    label_text: "Event".into(),
                    width: 3.0,
                    label_background_color: label_background,
                    label_text_color: Some(label_text),
                    show_label: true,
                },
            )
            .unwrap();
        let frame = chart.build_frame();
        assert!(frame.panes[0].main.iter().any(|primitive| {
            matches!(primitive, Prim::Rect { rect, color: actual }
                if *actual == color && rect.h == 500 && rect.w == 3)
        }));
        let axis = chart.build_axis_frame(
            80.0,
            |text, _bold| text.len() as f64 * 7.0,
            |text, _bold| text.len() as f64 * 6.0,
        );
        let label = axis
            .labels
            .iter()
            .find(|label| label.text == "Event")
            .expect("vertical line must contribute its time-axis view");
        assert_eq!(label.color, label_text);
        assert_eq!(label.background.unwrap().4, label_background);
        assert_eq!(label.background_corners, crate::AxisLabelCorners::BOTTOM);
    }

    #[test]
    fn trend_line_emits_endpoint_labels_and_contributes_visible_autoscale() {
        let mut chart = chart();
        chart
            .add_trend_line(
                0,
                2 * 86_400,
                50.0,
                6 * 86_400,
                200.0,
                TrendLineOptions::default(),
            )
            .unwrap();
        let frame = chart.build_frame();
        assert!(frame.panes[0].main.iter().any(|primitive| {
            matches!(primitive, Prim::Polyline { width, point_count: 2, .. } if *width == 6.0)
        }));
        assert!(frame.panes[0]
            .main
            .iter()
            .any(|primitive| { matches!(primitive, Prim::Text { text, .. } if text == "50.0") }));
        assert!(frame.panes[0]
            .main
            .iter()
            .any(|primitive| { matches!(primitive, Prim::Text { text, .. } if text == "200.0") }));
        let range = chart.panes[0].price_scale.price_range().unwrap();
        assert!(range.min_value() <= 50.0);
        assert!(range.max_value() >= 200.0);
    }

    #[test]
    fn text_watermark_zoom_alignment_and_visibility_are_engine_owned() {
        let mut chart = chart();
        chart.set_text_measure(Some(Box::new(
            |text, _, _, _, _| {
                if text == "Wide" {
                    1_600.0
                } else {
                    0.0
                }
            },
        )));
        let options = TextWatermarkOptions {
            visible: true,
            horizontal_align: AnchoredTextHorizontalAlign::Middle,
            vertical_align: AnchoredTextVerticalAlign::Middle,
            lines: vec![TextWatermarkLine {
                text: "Wide".into(),
                color: Color::rgba(10, 20, 30, 128),
                font_size: 48.0,
                font_family: "Arial".into(),
                font_weight: 700,
                italic: false,
                line_height: 57.6,
            }],
        };
        let id = chart.add_text_watermark(0, options.clone()).unwrap();
        let frame = chart.build_frame();
        let text = frame.panes[0]
            .main
            .iter()
            .find_map(|primitive| match primitive {
                Prim::Text {
                    x,
                    y,
                    text,
                    size,
                    align,
                    ..
                } if text == "Wide" => Some((*x, *y, *size, *align)),
                _ => None,
            })
            .unwrap();
        assert_eq!(text.0, 400.0);
        assert!((text.1 - 247.6).abs() < 0.001);
        assert_eq!(text.2, 24.0);
        assert_eq!(text.3, aeris_charts_render::draw_list::TextAlign::Center);

        assert!(chart.set_text_watermark_options(
            id,
            TextWatermarkOptions {
                visible: false,
                ..options
            },
        ));
        assert!(chart.build_frame().panes[0]
            .main
            .iter()
            .all(|primitive| !matches!(primitive, Prim::Text { text, .. } if text == "Wide")));
    }
}
