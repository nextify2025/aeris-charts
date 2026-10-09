//! Price-line creation and option application/serialization (reference `createPriceLine`,
//! `IPriceLine.applyOptions`/`IPriceLine.options`). Headless so hosts delegate here; the
//! wasm boundary only converts wire types. Extracted from `lib.rs`.

use super::*;

/// JSON patch accepted by [`ChartEngine::price_line_apply_options`]. Snake_case keys are
/// canonical (matching the TS `price_line_options`); the reference camelCase forms are accepted as
/// aliases. Every field is optional — absent keys keep their current values (reference merge).
#[derive(serde::Deserialize)]
struct PriceLinePatch {
    price: Option<f64>,
    color: Option<String>,
    #[serde(alias = "lineWidth")]
    line_width: Option<f64>,
    #[serde(alias = "lineStyle")]
    line_style: Option<serde_json::Value>,
    title: Option<String>,
    #[serde(alias = "lineVisible")]
    line_visible: Option<bool>,
    #[serde(alias = "axisLabelVisible")]
    axis_label_visible: Option<bool>,
    #[serde(alias = "axisLabelColor")]
    axis_label_color: Option<String>,
    #[serde(alias = "axisLabelTextColor")]
    axis_label_text_color: Option<String>,
}

/// The TS wire name of a line style (`solid`/`dotted`/`dashed`, matching `line_style`).
fn line_style_name(style: LineStyle) -> &'static str {
    match style {
        LineStyle::Dotted => "dotted",
        LineStyle::Dashed => "dashed",
        LineStyle::Solid => "solid",
    }
}

/// A patch's `line_style`: the TS string form, or the reference numeric enum for untyped callers.
/// The retired names/values fold into their renamed equivalents (large_dashed → dashed,
/// sparse_dotted → dotted).
fn parse_line_style(value: &serde_json::Value) -> Option<LineStyle> {
    match value {
        serde_json::Value::String(s) => Some(match s.as_str() {
            "dotted" | "sparse_dotted" => LineStyle::Dotted,
            "dashed" | "large_dashed" => LineStyle::Dashed,
            _ => LineStyle::Solid,
        }),
        serde_json::Value::Number(n) => n.as_u64().map(|v| line_style_from_u8(v as u8)),
        _ => None,
    }
}

/// Apply an optional CSS color input to a stored `Option<String>` slot: `""` clears the
/// override (back to its follow state), a parseable color pins the string verbatim, and an
/// unparseable one is ignored — the same keep/clear/pin contract as the candle part colors.
fn update_css_slot(slot: &mut Option<String>, value: String) {
    if value.is_empty() {
        *slot = None;
    } else if Color::parse_css(&value).is_some() {
        *slot = Some(value);
    }
}

impl ChartEngine {
    /// Add a horizontal price line to a series; returns its chart-unique id (reference
    /// `createPriceLine`). reference defaults apply to the visibility/label extras
    /// (`price-line-options.ts`: `lineVisible`/`axisLabelVisible` true, label colors `''`).
    pub fn create_price_line(
        &mut self,
        series_id: SeriesId,
        price: f64,
        color: Color,
        width: i32,
        style: LineStyle,
        title: &str,
    ) -> u32 {
        self.invalidate_frame_scene();
        let id = self.next_price_line_id;
        self.next_price_line_id += 1;
        if let Some(s) = self.series.iter_mut().find(|s| s.id == series_id) {
            s.price_lines.push(PriceLine {
                id,
                price,
                color,
                width: width.max(1),
                style,
                title: title.to_string(),
                line_visible: true,
                axis_label_visible: true,
                axis_label_color: None,
                axis_label_text_color: None,
            });
        }
        id
    }

    /// Remove a price line by id (from whichever series holds it).
    pub fn remove_price_line(&mut self, id: u32) {
        self.invalidate_frame_scene();
        for s in &mut self.series {
            s.price_lines.retain(|pl| pl.id != id);
        }
    }

    /// Merge a JSON options patch into the price line with `id` (reference
    /// `IPriceLine.applyOptions`): absent keys keep their current values. Returns false for a
    /// malformed patch or an unknown id (both are host no-ops).
    pub fn price_line_apply_options(&mut self, id: u32, json: &str) -> bool {
        self.invalidate_frame_scene();
        let Ok(patch) = serde_json::from_str::<PriceLinePatch>(json) else {
            return false;
        };
        let Some(line) = self
            .series
            .iter_mut()
            .flat_map(|s| s.price_lines.iter_mut())
            .find(|line| line.id == id)
        else {
            return false;
        };
        if let Some(price) = patch.price
            && price.is_finite()
        {
            line.price = price;
        }
        if let Some(css) = patch.color
            && let Some(c) = Color::parse_css(&css)
        {
            line.color = c;
        }
        if let Some(width) = patch.line_width
            && width.is_finite()
        {
            line.width = (width.round() as i32).max(1);
        }
        if let Some(style) = patch.line_style.as_ref().and_then(parse_line_style) {
            line.style = style;
        }
        if let Some(title) = patch.title {
            line.title = title;
        }
        if let Some(visible) = patch.line_visible {
            line.line_visible = visible;
        }
        if let Some(visible) = patch.axis_label_visible {
            line.axis_label_visible = visible;
        }
        if let Some(css) = patch.axis_label_color {
            update_css_slot(&mut line.axis_label_color, css);
        }
        if let Some(css) = patch.axis_label_text_color {
            update_css_slot(&mut line.axis_label_text_color, css);
        }
        true
    }

    /// The price line's full options as a snake_case JSON object (reference `IPriceLine.options`).
    /// Colors serialize as CSS strings that keep alpha; the label-color overrides serialize
    /// as `""` while following their default. `None` for an unknown id.
    pub fn price_line_options_json(&self, id: u32) -> Option<String> {
        let line = self
            .series
            .iter()
            .flat_map(|s| s.price_lines.iter())
            .find(|line| line.id == id)?;
        Some(
            serde_json::json!({
                "price": line.price,
                "color": line.color.to_css(),
                "line_width": line.width,
                "line_style": line_style_name(line.style),
                "title": line.title,
                "line_visible": line.line_visible,
                "axis_label_visible": line.axis_label_visible,
                "axis_label_color": line.axis_label_color.as_deref().unwrap_or(""),
                "axis_label_text_color": line.axis_label_text_color.as_deref().unwrap_or(""),
            })
            .to_string(),
        )
    }
}
