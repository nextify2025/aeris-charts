//! Browser-boundary parsing for first-class engine-owned financial primitives.

use super::*;
use aeris_charts_core::model::data_validation::validate_timestamp;
use aeris_charts_engine::{
    AccessibilityFocusOptions, AnchoredTextHorizontalAlign, AnchoredTextOptions,
    AnchoredTextVerticalAlign, AreaBrushOverrides, BandsIndicatorOptions, BrushStyleOverride,
    DeltaTooltipOptions, ImageWatermarkOptions, OverlayPriceScaleOptions, OverlayPriceScaleSide,
    SessionHighlightingData, SessionHighlightingOptions, TextWatermarkLine, TextWatermarkOptions,
    TooltipOptions, TrendLineOptions, VerticalLineOptions, VolumeProfileData, VolumeProfileOptions,
    VolumeProfilePoint,
};
use std::sync::Arc;

fn json_color(value: &serde_json::Value, key: &str, fallback: Color) -> Color {
    value
        .get(key)
        .and_then(serde_json::Value::as_str)
        .and_then(Color::parse_css)
        .unwrap_or(fallback)
}

fn json_optional_color(value: &serde_json::Value, key: &str) -> Option<Color> {
    value
        .get(key)
        .and_then(serde_json::Value::as_str)
        .and_then(Color::parse_css)
}

fn parse_brush_override(value: &serde_json::Value) -> Option<BrushStyleOverride> {
    let color = |key: &str| match value.get(key) {
        None => Some(None),
        Some(field) => Some(Some(Color::parse_css(field.as_str()?)?)),
    };
    let line_width = match value.get("line_width") {
        None => None,
        Some(field) => Some(
            field
                .as_f64()
                .filter(|width| width.is_finite() && *width > 0.0)?,
        ),
    };
    Some(BrushStyleOverride {
        line_color: color("line_color")?,
        top_color: color("top_color")?,
        bottom_color: color("bottom_color")?,
        line_width,
    })
}

fn parse_session_highlights(json: &str) -> Option<Vec<SessionHighlightingData>> {
    serde_json::from_str::<serde_json::Value>(json)
        .ok()?
        .as_array()?
        .iter()
        .map(|item| {
            let time = validate_timestamp(item.get("time")?.as_f64()?).ok()?;
            let color = item
                .get("color")
                .and_then(serde_json::Value::as_str)
                .and_then(Color::parse_css)
                .unwrap_or(Color::rgba(0, 0, 0, 0));
            Some(SessionHighlightingData { time, color })
        })
        .collect()
}

fn parse_accessibility_focus_options(json: &str) -> Option<AccessibilityFocusOptions> {
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    let defaults = AccessibilityFocusOptions::default();
    Some(AccessibilityFocusOptions {
        color: json_color(&value, "color", defaults.color),
        size: value
            .get("size")
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(defaults.size),
        high_contrast: value
            .get("high_contrast")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(defaults.high_contrast),
    })
}

fn parse_bands_indicator_options(json: &str) -> Option<BandsIndicatorOptions> {
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    let defaults = BandsIndicatorOptions::default();
    Some(BandsIndicatorOptions {
        line_color: json_color(&value, "line_color", defaults.line_color),
        fill_color: json_color(&value, "fill_color", defaults.fill_color),
        line_width: value
            .get("line_width")
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(defaults.line_width),
    })
}

fn parse_overlay_price_scale_options(json: &str) -> Option<OverlayPriceScaleOptions> {
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    let side = match value.get("side").and_then(serde_json::Value::as_str) {
        None | Some("left") => OverlayPriceScaleSide::Left,
        Some("right") => OverlayPriceScaleSide::Right,
        _ => return None,
    };
    Some(OverlayPriceScaleOptions {
        text_color: json_optional_color(&value, "text_color"),
        side,
    })
}

fn parse_volume_profile(json: &str) -> Option<VolumeProfileData> {
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    let time = validate_timestamp(value.get("time")?.as_f64()?).ok()?;
    let width = value.get("width")?.as_f64()?;
    let profile = value
        .get("profile")?
        .as_array()?
        .iter()
        .map(|point| {
            Some(VolumeProfilePoint {
                price: point.get("price")?.as_f64()?,
                volume: point.get("vol").or_else(|| point.get("volume"))?.as_f64()?,
            })
        })
        .collect::<Option<Vec<_>>>()?;
    Some(VolumeProfileData {
        time,
        profile,
        width,
    })
}

impl ChartInner {
    pub(super) fn add_native_bands_indicator(&mut self, series_id: u32, options_json: &str) -> u32 {
        parse_bands_indicator_options(options_json)
            .and_then(|options| self.engine.add_bands_indicator(series_id, options))
            .unwrap_or(0)
    }

    pub(super) fn set_native_bands_indicator_options(
        &mut self,
        id: u32,
        options_json: &str,
    ) -> bool {
        parse_bands_indicator_options(options_json)
            .is_some_and(|options| self.engine.set_bands_indicator_options(id, options))
    }

    pub(super) fn add_native_overlay_price_scale(
        &mut self,
        series_id: u32,
        options_json: &str,
    ) -> u32 {
        parse_overlay_price_scale_options(options_json)
            .and_then(|options| self.engine.add_overlay_price_scale(series_id, options))
            .unwrap_or(0)
    }

    pub(super) fn set_native_overlay_price_scale_options(
        &mut self,
        id: u32,
        options_json: &str,
    ) -> bool {
        parse_overlay_price_scale_options(options_json)
            .is_some_and(|options| self.engine.set_overlay_price_scale_options(id, options))
    }

    pub(super) fn add_native_text_watermark(
        &mut self,
        pane_index: usize,
        options_json: &str,
    ) -> u32 {
        parse_text_watermark_options(options_json)
            .and_then(|options| self.engine.add_text_watermark(pane_index, options))
            .unwrap_or(0)
    }

    pub(super) fn set_native_text_watermark_options(
        &mut self,
        id: u32,
        options_json: &str,
    ) -> bool {
        parse_text_watermark_options(options_json)
            .is_some_and(|options| self.engine.set_text_watermark_options(id, options))
    }

    pub(super) fn add_native_anchored_text(&mut self, series_id: u32, options_json: &str) -> u32 {
        parse_anchored_text_options(options_json)
            .and_then(|options| self.engine.add_anchored_text(series_id, options))
            .unwrap_or(0)
    }

    pub(super) fn add_native_vertical_line(
        &mut self,
        series_id: u32,
        time: f64,
        options_json: &str,
    ) -> u32 {
        let Ok(time) = validate_timestamp(time) else {
            return 0;
        };
        let value: serde_json::Value = match serde_json::from_str(options_json) {
            Ok(value) => value,
            Err(_) => return 0,
        };
        let defaults = VerticalLineOptions::default();
        let options = VerticalLineOptions {
            color: json_color(&value, "color", defaults.color),
            label_text: value
                .get("label_text")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string(),
            width: value
                .get("width")
                .and_then(serde_json::Value::as_f64)
                .unwrap_or(defaults.width),
            label_background_color: json_color(
                &value,
                "label_background_color",
                defaults.label_background_color,
            ),
            label_text_color: json_optional_color(&value, "label_text_color"),
            show_label: value
                .get("show_label")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(defaults.show_label),
        };
        self.engine
            .add_vertical_line(series_id, time, options)
            .unwrap_or(0)
    }

    pub(super) fn add_native_delta_tooltip(&mut self, series_id: u32, options_json: &str) -> u32 {
        let value: serde_json::Value =
            serde_json::from_str(options_json).unwrap_or(serde_json::Value::Null);
        let defaults = DeltaTooltipOptions::default();
        self.engine
            .add_delta_tooltip(
                series_id,
                DeltaTooltipOptions {
                    line_color: json_optional_color(&value, "line_color"),
                    show_time: value
                        .get("show_time")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(defaults.show_time),
                    top_offset: value
                        .get("top_offset")
                        .and_then(serde_json::Value::as_f64)
                        .unwrap_or(defaults.top_offset),
                    requires_shift_drag: value
                        .get("requires_shift_drag")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(defaults.requires_shift_drag),
                },
            )
            .unwrap_or(0)
    }

    pub(super) fn set_native_brushable_area(&mut self, series_id: u32, options_json: &str) -> bool {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(options_json) else {
            return false;
        };
        if value.is_null() {
            return self.engine.set_brushable_area(series_id, None);
        }
        let defaults = DeltaTooltipOptions::default();
        let tooltip = DeltaTooltipOptions {
            line_color: None,
            show_time: defaults.show_time,
            top_offset: defaults.top_offset,
            requires_shift_drag: defaults.requires_shift_drag,
        };
        let (Some(outside), Some(positive), Some(negative)) = (
            parse_brush_override(&value["outside"]),
            parse_brush_override(&value["positive"]),
            parse_brush_override(&value["negative"]),
        ) else {
            return false;
        };
        let overrides = AreaBrushOverrides {
            outside,
            positive,
            negative,
        };
        self.engine
            .set_brushable_area_with_styles(series_id, Some(tooltip), overrides)
    }

    pub(super) fn add_native_tooltip(&mut self, series_id: u32, options_json: &str) -> u32 {
        parse_tooltip_options(options_json)
            .and_then(|options| self.engine.add_tooltip(series_id, options))
            .unwrap_or(0)
    }

    pub(super) fn set_native_tooltip_options(
        &mut self,
        primitive_id: u32,
        options_json: &str,
    ) -> bool {
        parse_tooltip_options(options_json)
            .is_some_and(|options| self.engine.set_tooltip_options(primitive_id, options))
    }

    pub(super) fn native_tooltip_snapshot_json(&self, primitive_id: u32) -> String {
        self.engine
            .tooltip_snapshot(primitive_id)
            .map(|snapshot| {
                serde_json::json!({
                    "x": snapshot.x,
                    "index": snapshot.index,
                    "price": snapshot.price,
                    "open": snapshot.open,
                    "high": snapshot.high,
                    "low": snapshot.low,
                    "close": snapshot.close,
                    "time": snapshot.time,
                })
                .to_string()
            })
            .unwrap_or_else(|| "null".into())
    }

    pub(super) fn native_delta_tooltip_active_range_json(&self, primitive_id: u32) -> String {
        self.engine
            .delta_tooltip_active_range(primitive_id)
            .map(|range| {
                serde_json::json!({
                    "from": range.from,
                    "to": range.to,
                    "positive": range.positive,
                })
                .to_string()
            })
            .unwrap_or_else(|| "null".into())
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn add_native_trend_line(
        &mut self,
        series_id: u32,
        first_time: f64,
        first_price: f64,
        second_time: f64,
        second_price: f64,
        options_json: &str,
    ) -> u32 {
        let (Ok(first_time), Ok(second_time)) = (
            validate_timestamp(first_time),
            validate_timestamp(second_time),
        ) else {
            return 0;
        };
        let value: serde_json::Value = match serde_json::from_str(options_json) {
            Ok(value) => value,
            Err(_) => return 0,
        };
        let defaults = TrendLineOptions::default();
        self.engine
            .add_trend_line(
                series_id,
                first_time,
                first_price,
                second_time,
                second_price,
                TrendLineOptions {
                    line_color: json_color(&value, "line_color", defaults.line_color),
                    width: value
                        .get("width")
                        .and_then(serde_json::Value::as_f64)
                        .unwrap_or(defaults.width),
                    show_labels: value
                        .get("show_labels")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(defaults.show_labels),
                    label_background_color: json_color(
                        &value,
                        "label_background_color",
                        defaults.label_background_color,
                    ),
                    label_text_color: json_color(
                        &value,
                        "label_text_color",
                        defaults.label_text_color,
                    ),
                },
            )
            .unwrap_or(0)
    }

    pub(super) fn set_native_anchored_text_options(&mut self, id: u32, options_json: &str) -> bool {
        parse_anchored_text_options(options_json)
            .is_some_and(|options| self.engine.set_anchored_text_options(id, options))
    }

    pub(super) fn add_native_image_watermark(
        &mut self,
        series_id: u32,
        width: u32,
        height: u32,
        pixels: &[u8],
        options_json: &str,
    ) -> u32 {
        let value: serde_json::Value =
            serde_json::from_str(options_json).unwrap_or(serde_json::Value::Null);
        let options = ImageWatermarkOptions {
            max_width: value.get("max_width").and_then(serde_json::Value::as_f64),
            max_height: value.get("max_height").and_then(serde_json::Value::as_f64),
            padding: value
                .get("padding")
                .and_then(serde_json::Value::as_f64)
                .unwrap_or(0.0),
            alpha: value
                .get("alpha")
                .and_then(serde_json::Value::as_f64)
                .unwrap_or(1.0),
        };
        self.engine
            .add_image_watermark(series_id, width, height, Arc::<[u8]>::from(pixels), options)
            .unwrap_or(0)
    }

    pub(super) fn add_native_accessibility_focus(
        &mut self,
        series_id: u32,
        options_json: &str,
    ) -> u32 {
        parse_accessibility_focus_options(options_json)
            .and_then(|options| self.engine.add_accessibility_focus(series_id, options))
            .unwrap_or(0)
    }

    pub(super) fn set_native_accessibility_focus(
        &mut self,
        primitive_id: u32,
        time: Option<i64>,
        options_json: &str,
    ) -> bool {
        parse_accessibility_focus_options(options_json).is_some_and(|options| {
            self.engine
                .set_accessibility_focus(primitive_id, time, options)
        })
    }

    pub(super) fn add_native_session_highlighting(
        &mut self,
        series_id: u32,
        options_json: &str,
    ) -> u32 {
        let value: serde_json::Value =
            serde_json::from_str(options_json).unwrap_or(serde_json::Value::Null);
        let defaults = SessionHighlightingOptions::default();
        // Fractional exchange-local hours; `*_hour_utc` remain accepted aliases (they equal the
        // exchange hours on the default UTC chart). A malformed hour rejects the attachment
        // rather than silently disabling the gate.
        let hour = |key: &str, legacy: &str| -> Result<Option<f64>, ()> {
            match value.get(key).or_else(|| value.get(legacy)) {
                None | Some(serde_json::Value::Null) => Ok(None),
                Some(hour) => hour
                    .as_f64()
                    .filter(|hour| hour.is_finite())
                    .map(Some)
                    .ok_or(()),
            }
        };
        let (Ok(start), Ok(end)) = (
            hour("start_hour", "start_hour_utc"),
            hour("end_hour", "end_hour_utc"),
        ) else {
            return 0;
        };
        self.engine
            .add_session_highlighting(
                series_id,
                SessionHighlightingOptions {
                    start_hour: start,
                    end_hour: end,
                    weekday_color: json_color(&value, "weekday_color", defaults.weekday_color),
                    weekend_color: json_color(&value, "weekend_color", defaults.weekend_color),
                },
            )
            .unwrap_or(0)
    }

    pub(super) fn set_native_session_highlighting_data(
        &mut self,
        primitive_id: u32,
        highlights_json: &str,
    ) -> bool {
        parse_session_highlights(highlights_json).is_some_and(|highlights| {
            self.engine
                .set_session_highlighting_data(primitive_id, highlights)
        })
    }

    pub(super) fn upsert_native_session_highlighting_data(
        &mut self,
        primitive_id: u32,
        highlights_json: &str,
    ) -> bool {
        parse_session_highlights(highlights_json).is_some_and(|highlights| {
            self.engine
                .upsert_session_highlighting_data(primitive_id, highlights)
        })
    }

    pub(super) fn add_native_crosshair_highlight(
        &mut self,
        series_id: u32,
        color: Option<String>,
    ) -> u32 {
        let color = color.map(|color| Color::parse_css(&color).unwrap_or(Color::rgba(0, 0, 0, 51)));
        self.engine
            .add_highlight_bar_crosshair(series_id, color)
            .unwrap_or(0)
    }

    pub(super) fn add_native_volume_profile(
        &mut self,
        series_id: u32,
        data_json: &str,
        options_json: &str,
    ) -> u32 {
        let Some(data) = parse_volume_profile(data_json) else {
            return 0;
        };
        let value: serde_json::Value =
            serde_json::from_str(options_json).unwrap_or(serde_json::Value::Null);
        let defaults = VolumeProfileOptions::default();
        self.engine
            .add_volume_profile(
                series_id,
                data,
                VolumeProfileOptions {
                    background_color: json_color(
                        &value,
                        "background_color",
                        defaults.background_color,
                    ),
                    row_color: json_color(&value, "color", defaults.row_color),
                },
            )
            .unwrap_or(0)
    }

    pub(super) fn set_native_volume_profile_data(&mut self, id: u32, data_json: &str) -> bool {
        parse_volume_profile(data_json)
            .is_some_and(|data| self.engine.set_volume_profile_data(id, data))
    }

    pub(super) fn remove_native_primitive(&mut self, id: u32) -> bool {
        self.engine.remove_native_primitive(id)
    }
}

fn parse_tooltip_options(options_json: &str) -> Option<TooltipOptions> {
    let value: serde_json::Value = serde_json::from_str(options_json).ok()?;
    let defaults = TooltipOptions::default();
    Some(TooltipOptions {
        line_color: json_optional_color(&value, "line_color"),
        top_margin: value
            .get("top_margin")
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(defaults.top_margin),
    })
}

fn parse_anchored_text_options(json: &str) -> Option<AnchoredTextOptions> {
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    let horizontal_align = match value.get("horizontal_align")?.as_str()? {
        "left" => AnchoredTextHorizontalAlign::Left,
        "middle" => AnchoredTextHorizontalAlign::Middle,
        "right" => AnchoredTextHorizontalAlign::Right,
        _ => return None,
    };
    let vertical_align = match value.get("vertical_align")?.as_str()? {
        "top" => AnchoredTextVerticalAlign::Top,
        "middle" => AnchoredTextVerticalAlign::Middle,
        "bottom" => AnchoredTextVerticalAlign::Bottom,
        _ => return None,
    };
    Some(AnchoredTextOptions {
        horizontal_align,
        vertical_align,
        text: value.get("text")?.as_str()?.to_string(),
        line_height: value.get("line_height")?.as_f64()?,
        font_size: value.get("font_size")?.as_f64()?,
        font_family: value.get("font_family")?.as_str()?.to_string(),
        font_weight: u16::try_from(value.get("font_weight")?.as_u64()?).ok()?,
        italic: value.get("italic")?.as_bool()?,
        color: value.get("color")?.as_str().and_then(Color::parse_css)?,
    })
}

fn parse_text_watermark_options(json: &str) -> Option<TextWatermarkOptions> {
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    let horizontal_align = match value.get("horizontal_align")?.as_str()? {
        "left" => AnchoredTextHorizontalAlign::Left,
        "center" => AnchoredTextHorizontalAlign::Middle,
        "right" => AnchoredTextHorizontalAlign::Right,
        _ => return None,
    };
    let vertical_align = match value.get("vertical_align")?.as_str()? {
        "top" => AnchoredTextVerticalAlign::Top,
        "center" => AnchoredTextVerticalAlign::Middle,
        "bottom" => AnchoredTextVerticalAlign::Bottom,
        _ => return None,
    };
    let lines = value
        .get("lines")?
        .as_array()?
        .iter()
        .map(|line| {
            Some(TextWatermarkLine {
                text: line.get("text")?.as_str()?.to_string(),
                color: line.get("color")?.as_str().and_then(Color::parse_css)?,
                font_size: line.get("font_size")?.as_f64()?,
                font_family: line.get("font_family")?.as_str()?.to_string(),
                font_weight: u16::try_from(line.get("font_weight")?.as_u64()?).ok()?,
                italic: line.get("italic")?.as_bool()?,
                line_height: line.get("line_height")?.as_f64()?,
            })
        })
        .collect::<Option<Vec<_>>>()?;
    Some(TextWatermarkOptions {
        visible: value.get("visible")?.as_bool()?,
        horizontal_align,
        vertical_align,
        lines,
    })
}
