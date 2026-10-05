//! Chart option structs, reference-matching defaults, and `apply_options` deep-merge (roadmap Phase A2).
//!
//! Financial-chart APIs commonly expose deeply nested option objects and an `applyOptions(partial)`
//! operation that **deep-merges** a partial patch into the current options: nested objects
//! are merged key-by-key so an update to `grid.vertLines.color` leaves every sibling untouched,
//! while scalars and arrays replace wholesale. We reproduce that contract exactly.
//!
//! Rather than hand-roll a merge per struct, options are held as a canonical `serde_json::Value`
//! seeded from the typed defaults; a patch (also JSON, straight from the JS boundary) is
//! deep-merged into it, and typed views are produced by deserializing on demand. This independently
//! implements the public deep-merge behavior and makes partial updates and round-tripping free.
//!
//! Colors are kept as CSS strings (as in reference); the render layer parses them. `LineStyle` is the
//! numeric wire form (0 Solid, 1 Dotted, 2 Dashed). The reference's 3 LargeDashed/4 SparseDotted
//! do not exist in this engine: `Dotted` IS the sparse pattern and `Dashed` the large one, and
//! the retired values fold into them (3 → Dashed, 4 → Dotted).
//!
//! Scope note: this covers the chart-level visual groups (layout, grid, crosshair), the axis
//! strips' border cosmetics (`leftPriceScale`/`rightPriceScale`/`timeScale`), and the top-level
//! flags. Time-scale and price-scale *behavioral* options currently live in their own core
//! structs (`TimeScaleOptions`, `PriceScaleCoreOptions`) and will be folded into this store in a
//! later Phase B pass; per-series options land with the series work.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::style::{
    DARK_ACCENT_CSS, DARK_BORDER_CSS, DARK_CROSSHAIR_LABEL_CSS, DARK_CROSSHAIR_LINE_CSS,
    DARK_FOREGROUND_CSS, DARK_MARKET_DOWN_CSS, DARK_MARKET_UP_CSS, DARK_MUTED_FOREGROUND_CSS,
    DARK_SURFACE_CSS, DEFAULT_ACCENT_CSS, DEFAULT_BORDER_CSS, DEFAULT_CROSSHAIR_LABEL_CSS,
    DEFAULT_CROSSHAIR_LINE_CSS, DEFAULT_CROSSHAIR_LINE_RGB, DEFAULT_FOREGROUND_CSS,
    DEFAULT_MARKET_DOWN_CSS, DEFAULT_MARKET_UP_CSS, DEFAULT_MUTED_FOREGROUND_CSS,
    DEFAULT_SURFACE_CSS, LIGHT_ACCENT_CSS, LIGHT_BORDER_CSS, LIGHT_CROSSHAIR_LABEL_CSS,
    LIGHT_CROSSHAIR_LINE_CSS, LIGHT_FOREGROUND_CSS, LIGHT_MARKET_DOWN_CSS, LIGHT_MARKET_UP_CSS,
    LIGHT_MUTED_FOREGROUND_CSS, LIGHT_SURFACE_CSS,
};

/// Aeris-owned application color mode. Hosts select a mode; Aeris resolves every chart color
/// from its canonical style token source.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChartTheme {
    Light,
    Dark,
}

impl Default for ChartTheme {
    fn default() -> Self {
        match crate::style::DEFAULT_THEME_NAME {
            "light" => Self::Light,
            _ => Self::Dark,
        }
    }
}

/// Complete cosmetic patch for one canonical Aeris theme.
#[must_use]
pub fn chart_theme_patch(theme: ChartTheme) -> Value {
    let (
        surface,
        foreground,
        muted_foreground,
        border,
        accent,
        crosshair_line,
        crosshair_label,
        bullish,
        bearish,
    ) = match theme {
        ChartTheme::Light => (
            LIGHT_SURFACE_CSS,
            LIGHT_FOREGROUND_CSS,
            LIGHT_MUTED_FOREGROUND_CSS,
            LIGHT_BORDER_CSS,
            LIGHT_ACCENT_CSS,
            LIGHT_CROSSHAIR_LINE_CSS,
            LIGHT_CROSSHAIR_LABEL_CSS,
            LIGHT_MARKET_UP_CSS,
            LIGHT_MARKET_DOWN_CSS,
        ),
        ChartTheme::Dark => (
            DARK_SURFACE_CSS,
            DARK_FOREGROUND_CSS,
            DARK_MUTED_FOREGROUND_CSS,
            DARK_BORDER_CSS,
            DARK_ACCENT_CSS,
            DARK_CROSSHAIR_LINE_CSS,
            DARK_CROSSHAIR_LABEL_CSS,
            DARK_MARKET_UP_CSS,
            DARK_MARKET_DOWN_CSS,
        ),
    };
    serde_json::json!({
        "layout": {
            "background": {
                "type": "solid",
                "color": surface,
                "topColor": surface,
                "bottomColor": surface
            },
            "textColor": foreground,
            "mutedTextColor": muted_foreground,
            "bullishColor": bullish,
            "bearishColor": bearish,
            "panes": {
                "separatorColor": border,
                "separatorHoverColor": accent
            }
        },
        "grid": {
            "vertLines": { "color": border },
            "horzLines": { "color": border }
        },
        "crosshair": {
            "vertLine": { "color": crosshair_line, "labelBackgroundColor": crosshair_label },
            "horzLine": { "color": crosshair_line, "labelBackgroundColor": crosshair_label }
        },
        "leftPriceScale": { "borderColor": border, "textColor": foreground },
        "rightPriceScale": { "borderColor": border, "textColor": foreground },
        "timeScale": { "borderColor": border }
    })
}

/// `LineStyle`, numeric wire form (0 Solid, 1 Dotted = sparse, 2 Dashed = large). Values 3/4
/// (the reference's LargeDashed/SparseDotted) are retired and fold into 2/1 at the engine edge.
pub mod line_style {
    pub const SOLID: u8 = 0;
    pub const DOTTED: u8 = 1;
    pub const DASHED: u8 = 2;
}

/// reference `CrosshairMode` (`model/crosshair.ts`), numeric wire form.
pub mod crosshair_mode {
    pub const NORMAL: u8 = 0;
    pub const MAGNET: u8 = 1;
    pub const HIDDEN: u8 = 2;
    pub const MAGNET_OHLC: u8 = 3;
}

fn surface_color() -> String {
    DEFAULT_SURFACE_CSS.into()
}
fn text_color() -> String {
    DEFAULT_FOREGROUND_CSS.into()
}
fn muted_text_color() -> String {
    DEFAULT_MUTED_FOREGROUND_CSS.into()
}
fn grid_color() -> String {
    DEFAULT_BORDER_CSS.into()
}
fn crosshair_color() -> String {
    DEFAULT_CROSSHAIR_LINE_CSS.into()
}
/// `crosshair.shadeRight` default tint: the theme-independent crosshair line token at 12% alpha.
fn crosshair_shade_color() -> String {
    let (r, g, b) = DEFAULT_CROSSHAIR_LINE_RGB;
    format!("rgba({r}, {g}, {b}, 0.12)")
}
fn crosshair_label_bg() -> String {
    DEFAULT_CROSSHAIR_LABEL_CSS.into()
}
fn axis_border_color() -> String {
    DEFAULT_BORDER_CSS.into()
}
fn default_font_family() -> String {
    // `helpers/make-font.ts` defaultFontFamily.
    "-apple-system, BlinkMacSystemFont, 'Trebuchet MS', Roboto, Ubuntu, sans-serif".into()
}

/// `layout.background` — solid only for now (reference also has a vertical gradient variant).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BackgroundOptions {
    /// `"solid"` | `"gradient"` (only solid is honored by the renderer today).
    #[serde(rename = "type")]
    pub kind: String,
    pub color: String,
    #[serde(rename = "topColor")]
    pub top_color: String,
    #[serde(rename = "bottomColor")]
    pub bottom_color: String,
}

impl Default for BackgroundOptions {
    fn default() -> Self {
        Self {
            kind: "solid".into(),
            color: surface_color(),
            top_color: surface_color(),
            bottom_color: surface_color(),
        }
    }
}

/// `layout.panes` — stacked-pane chrome (`api/options/layout-options-defaults.ts` v5:
/// `separatorColor`/`separatorHoverColor`; `enableResize` is not modeled).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PanesOptions {
    /// The pane divider color. Empty (the default) follows the price-axis border color, so the
    /// divider always matches the axis chrome and tracks its theme; an explicit value pins it.
    #[serde(rename = "separatorColor")]
    pub separator_color: String,
    /// Hover band painted over a separator by the gesture layer.
    #[serde(rename = "separatorHoverColor")]
    pub separator_hover_color: String,
}

impl Default for PanesOptions {
    fn default() -> Self {
        Self {
            separator_color: String::new(),
            separator_hover_color: DEFAULT_ACCENT_CSS.into(),
        }
    }
}

/// `layout` — background, text, font (`api/options/layout-options-defaults.ts`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LayoutOptions {
    pub background: BackgroundOptions,
    #[serde(rename = "textColor")]
    pub text_color: String,
    /// Secondary unboxed chart text. Live labels use the dark foreground for contrast.
    #[serde(rename = "mutedTextColor")]
    pub muted_text_color: String,
    /// Theme-owned fallback for unpinned bullish candlestick and bar geometry.
    #[serde(rename = "bullishColor")]
    pub bullish_color: String,
    /// Theme-owned fallback for unpinned bearish candlestick and bar geometry.
    #[serde(rename = "bearishColor")]
    pub bearish_color: String,
    #[serde(rename = "fontSize")]
    pub font_size: f64,
    #[serde(rename = "fontFamily")]
    pub font_family: String,
    pub panes: PanesOptions,
}

impl Default for LayoutOptions {
    fn default() -> Self {
        Self {
            background: BackgroundOptions::default(),
            text_color: text_color(),
            muted_text_color: muted_text_color(),
            bullish_color: DEFAULT_MARKET_UP_CSS.into(),
            bearish_color: DEFAULT_MARKET_DOWN_CSS.into(),
            font_size: 12.0,
            font_family: default_font_family(),
            panes: PanesOptions::default(),
        }
    }
}

/// A single family of grid lines (`api/options/grid-options-defaults.ts`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GridLineOptions {
    pub color: String,
    /// [`line_style`] value.
    pub style: u8,
    pub visible: bool,
}

impl Default for GridLineOptions {
    fn default() -> Self {
        Self {
            color: grid_color(),
            // Grid style and color remain configured while the canonical chart ships clean.
            // Hosts can opt either family in without reconstructing its visual state.
            style: line_style::DASHED,
            visible: false,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GridOptions {
    #[serde(rename = "vertLines")]
    pub vert_lines: GridLineOptions,
    #[serde(rename = "horzLines")]
    pub horz_lines: GridLineOptions,
}

/// One crosshair line (`api/options/crosshair-options-defaults.ts`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CrosshairLineOptions {
    pub color: String,
    pub width: f64,
    /// [`line_style`] value (Aeris default Dashed).
    pub style: u8,
    pub visible: bool,
    #[serde(rename = "labelVisible")]
    pub label_visible: bool,
    #[serde(rename = "labelBackgroundColor")]
    pub label_background_color: String,
}

impl Default for CrosshairLineOptions {
    fn default() -> Self {
        Self {
            color: crosshair_color(),
            width: 1.0,
            // Dashed out of the box — the reference's LargeDashed default renders as this
            // engine's Dashed pattern.
            style: line_style::DASHED,
            visible: true,
            label_visible: true,
            label_background_color: crosshair_label_bg(),
        }
    }
}

/// `crosshair.shadeRight` — a translucent veil over the pane region to the right of the hovered
/// bar (Aeris extension). It starts at the snapped bar's right edge, reaches the pane's right edge,
/// and is painted in every stacked pane like the vertical line. Opacity travels in the CSS alpha of
/// `color`; an unparsable color falls back to the default tint.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CrosshairShadeOptions {
    /// Off by default.
    pub visible: bool,
    /// CSS color; the default is the crosshair line token at 12% alpha.
    pub color: String,
}

impl Default for CrosshairShadeOptions {
    fn default() -> Self {
        Self {
            visible: false,
            color: crosshair_shade_color(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CrosshairOptions {
    #[serde(rename = "vertLine")]
    pub vert_line: CrosshairLineOptions,
    #[serde(rename = "horzLine")]
    pub horz_line: CrosshairLineOptions,
    /// Veil right of the hovered bar; see [`CrosshairShadeOptions`].
    #[serde(rename = "shadeRight")]
    pub shade_right: CrosshairShadeOptions,
    /// [`crosshair_mode`] value (default Magnet, matching reference).
    pub mode: u8,
    /// reference `doNotSnapToHiddenSeriesIndices` (default false): when true, the crosshair's snapped
    /// bar index moves to the nearest index that has a bar in any visible series.
    #[serde(rename = "doNotSnapToHiddenSeriesIndices")]
    pub do_not_snap_to_hidden_series_indices: bool,
}

/// Chart-level options of a pane price-axis strip: visibility plus the reference border cosmetics
/// (`price-scale.options.ts`: `borderVisible`/`borderColor`) and the label cosmetics the
/// engine keeps per scale (`alignLabels`/`ticksVisible`/`entireTextOnly`/`minimumWidth`/
/// `textColor` — reference applies these chart-level groups to every pane's scale, pane.ts
/// `applyScaleOptions`). Scale math and per-series scale options are owned by
/// `PriceScaleCore`; `visible` determines whether layout reserves and paints the strip.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PriceAxisOptions {
    pub visible: bool,
    #[serde(rename = "borderVisible")]
    pub border_visible: bool,
    #[serde(rename = "borderColor")]
    pub border_color: String,
    /// reference `alignLabels` (default true).
    #[serde(rename = "alignLabels")]
    pub align_labels: bool,
    /// reference `ticksVisible` (default false).
    #[serde(rename = "ticksVisible")]
    pub ticks_visible: bool,
    /// reference `entireTextOnly` (default false).
    #[serde(rename = "entireTextOnly")]
    pub entire_text_only: bool,
    /// reference `minimumWidth` (default 0).
    #[serde(rename = "minimumWidth")]
    pub minimum_width: f64,
    /// reference `textColor` (default `None` = follow `layout.textColor`).
    #[serde(rename = "textColor")]
    pub text_color: Option<String>,
}

impl PriceAxisOptions {
    fn visible(visible: bool) -> Self {
        Self {
            visible,
            border_visible: true,
            border_color: axis_border_color(),
            // reference defaults (price-scale-options-defaults.ts).
            align_labels: true,
            ticks_visible: false,
            entire_text_only: false,
            minimum_width: 0.0,
            text_color: None,
        }
    }
}

/// Time-axis border cosmetics (`time-scale.options.ts`: `borderVisible`/`borderColor`). The rest
/// of the time-scale surface (bar spacing, offsets) lives in `TimeScaleCore` behind dedicated
/// setters and folds into this store in a later pass.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TimeAxisOptions {
    #[serde(rename = "borderVisible")]
    pub border_visible: bool,
    #[serde(rename = "borderColor")]
    pub border_color: String,
}

impl Default for TimeAxisOptions {
    fn default() -> Self {
        Self {
            border_visible: true,
            border_color: axis_border_color(),
        }
    }
}

impl Default for PriceAxisOptions {
    fn default() -> Self {
        Self::visible(true)
    }
}

impl Default for CrosshairOptions {
    fn default() -> Self {
        Self {
            vert_line: CrosshairLineOptions::default(),
            horz_line: CrosshairLineOptions::default(),
            shade_right: CrosshairShadeOptions::default(),
            mode: crosshair_mode::NORMAL, // deliberate divergence from the reference's Magnet default
            do_not_snap_to_hidden_series_indices: false,
        }
    }
}

/// `watermark` — a large text label painted inside the pane (`api/options/watermark`, reference v4
/// shape). Aeris draws it on the shared Canvas2D overlay above the series (a deliberate divergence
/// from the reference's behind-series placement: it is the only text path that stays pixel-identical across
/// the WebGPU and Canvas2D pane backends). Colors are CSS strings so alpha is preserved verbatim.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WatermarkOptions {
    pub visible: bool,
    pub text: String,
    /// CSS color (default fully transparent, matching reference — a watermark shows only once colored).
    pub color: String,
    #[serde(rename = "fontSize")]
    pub font_size: f64,
    #[serde(rename = "fontFamily")]
    pub font_family: String,
    #[serde(rename = "fontStyle")]
    pub font_style: String,
    /// `"left" | "center" | "right"`.
    #[serde(rename = "horzAlign")]
    pub horz_align: String,
    /// `"top" | "center" | "bottom"`.
    #[serde(rename = "vertAlign")]
    pub vert_align: String,
}

impl Default for WatermarkOptions {
    fn default() -> Self {
        Self {
            visible: false,
            text: String::new(),
            color: "rgba(0, 0, 0, 0)".into(),
            font_size: 48.0,
            font_family: default_font_family(),
            font_style: String::new(),
            horz_align: "center".into(),
            vert_align: "center".into(),
        }
    }
}

/// Chart-level options (`api/options/chart-options-defaults.ts`, visual subset).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ChartOptions {
    pub layout: LayoutOptions,
    pub grid: GridOptions,
    pub crosshair: CrosshairOptions,
    pub watermark: WatermarkOptions,
    #[serde(rename = "leftPriceScale")]
    pub left_price_scale: PriceAxisOptions,
    #[serde(rename = "rightPriceScale")]
    pub right_price_scale: PriceAxisOptions,
    #[serde(rename = "timeScale")]
    pub time_scale: TimeAxisOptions,
    #[serde(rename = "autoSize")]
    pub auto_size: bool,
    #[serde(rename = "hoveredSeriesOnTop")]
    pub hovered_series_on_top: bool,
}

impl Default for ChartOptions {
    fn default() -> Self {
        Self {
            layout: LayoutOptions::default(),
            grid: GridOptions::default(),
            crosshair: CrosshairOptions::default(),
            watermark: WatermarkOptions::default(),
            left_price_scale: PriceAxisOptions::visible(false),
            right_price_scale: PriceAxisOptions::visible(true),
            time_scale: TimeAxisOptions::default(),
            auto_size: false,
            hovered_series_on_top: true,
        }
    }
}

/// Recursively deep-merge `patch` into `dst`, matching reference `helpers/merge.ts`: when both sides of
/// a key are JSON objects, merge them key-by-key; otherwise `patch` replaces `dst` wholesale
/// (scalars, arrays, and null all overwrite). A `null` in `patch` explicitly sets the key to null.
pub fn deep_merge(dst: &mut Value, patch: &Value) {
    match (dst, patch) {
        (Value::Object(d), Value::Object(p)) => {
            for (k, pv) in p {
                match d.get_mut(k) {
                    Some(dv) if dv.is_object() && pv.is_object() => deep_merge(dv, pv),
                    _ => {
                        d.insert(k.clone(), pv.clone());
                    }
                }
            }
        }
        (d, p) => *d = p.clone(),
    }
}

/// Accumulates chart options across successive `apply_options` calls. The typed view is canonical
/// for engine/frame reads; raw JSON is retained only for deep-merge and browser round-tripping.
#[derive(Clone, Debug)]
pub struct ChartOptionsStore {
    value: Value,
    typed: ChartOptions,
    generation: u64,
}

impl Default for ChartOptionsStore {
    fn default() -> Self {
        Self::new()
    }
}

impl ChartOptionsStore {
    /// Start from the canonical Aeris engine defaults.
    pub fn new() -> Self {
        let typed = ChartOptions::default();
        Self {
            value: serde_json::to_value(&typed).expect("options serialize"),
            typed,
            generation: 0,
        }
    }

    /// Deep-merge a JSON patch object into the current options. A non-object patch is ignored
    /// (options are always an object at the root).
    pub fn apply(&mut self, patch: &Value) {
        if patch.is_object() {
            let mut patch = patch.clone();
            if let Some(layout) = patch.get_mut("layout").and_then(Value::as_object_mut) {
                layout.remove("attributionLogo");
            }
            deep_merge(&mut self.value, &patch);
            self.typed = serde_json::from_value(self.value.clone()).unwrap_or_default();
            self.generation = self.generation.wrapping_add(1);
        }
    }

    /// Deep-merge a JSON patch string (as it arrives from the JS boundary). Returns the parse
    /// error for a malformed patch; on error the current options are left unchanged.
    pub fn apply_str(&mut self, patch: &str) -> Result<(), serde_json::Error> {
        let patch: Value = serde_json::from_str(patch)?;
        self.apply(&patch);
        Ok(())
    }

    /// Typed view of the current options.
    pub fn get(&self) -> &ChartOptions {
        &self.typed
    }

    /// The raw merged JSON (for round-tripping back to JS via `options()`).
    pub fn value(&self) -> &Value {
        &self.value
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Restore engine-owned chart styling to the canonical defaults for `theme` while preserving
    /// behavioral/runtime options stored beside those visual groups. In particular, time-scale
    /// spacing/offset options, auto-size, crosshair snapping mode, and other unknown host keys are
    /// retained. Semantic follow states are restored rather than pinning their effective colors:
    /// pane separators follow the axis border and price-scale text follows layout text.
    pub fn reset_style_to_defaults(&mut self, theme: ChartTheme) {
        let mut defaults =
            serde_json::to_value(ChartOptions::default()).expect("options serialize");
        deep_merge(&mut defaults, &chart_theme_patch(theme));

        // A theme patch supplies effective colors for creation/theme switching. A style reset must
        // restore the canonical follow semantics instead of pinning those effective values.
        if let Some(layout) = defaults.get_mut("layout").and_then(Value::as_object_mut) {
            if let Some(panes) = layout.get_mut("panes").and_then(Value::as_object_mut) {
                panes.insert("separatorColor".into(), Value::String(String::new()));
            }
        }
        for key in ["leftPriceScale", "rightPriceScale"] {
            if let Some(scale) = defaults.get_mut(key).and_then(Value::as_object_mut) {
                scale.insert("textColor".into(), Value::Null);
            }
        }

        let merge_group = |dst: &mut Value, source: &Value, key: &str| {
            let Some(source_group) = source.get(key) else {
                return;
            };
            let Some(dst_object) = dst.as_object_mut() else {
                return;
            };
            match dst_object.get_mut(key) {
                Some(existing) => deep_merge(existing, source_group),
                None => {
                    dst_object.insert(key.to_string(), source_group.clone());
                }
            }
        };

        for key in ["layout", "grid", "timeScale"] {
            merge_group(&mut self.value, &defaults, key);
        }

        // Watermark content/visibility are host state; reset only its appearance.
        if let (Some(current), Some(default)) = (
            self.value
                .get_mut("watermark")
                .and_then(Value::as_object_mut),
            defaults.get("watermark").and_then(Value::as_object),
        ) {
            for key in [
                "color",
                "fontSize",
                "fontFamily",
                "fontStyle",
                "horzAlign",
                "vertAlign",
            ] {
                if let Some(value) = default.get(key) {
                    current.insert(key.to_string(), value.clone());
                }
            }
        }

        // Built-in scale strip visibility and layout/label behavior are state, not style. Keep
        // those exact settings while restoring border/text cosmetics and semantic text following.
        for group_key in ["leftPriceScale", "rightPriceScale"] {
            if let (Some(current), Some(default)) = (
                self.value.get_mut(group_key).and_then(Value::as_object_mut),
                defaults.get(group_key).and_then(Value::as_object),
            ) {
                for key in ["borderVisible", "borderColor", "textColor", "ticksVisible"] {
                    if let Some(value) = default.get(key) {
                        current.insert(key.to_string(), value.clone());
                    }
                }
                // This scale-core visual default is not represented by ChartOptions' typed shape,
                // but chart-level patches may still carry it in the raw options object.
                current.insert("boldRoundLabels".into(), Value::Bool(true));
            }
        }

        // Crosshair mode/snapping are interaction behavior; only the two visual line groups reset.
        // The right-of-cursor veil follows the watermark rule: its tint is style, whether it is
        // shown is host state.
        if let (Some(current), Some(default)) = (
            self.value
                .get_mut("crosshair")
                .and_then(Value::as_object_mut),
            defaults.get("crosshair").and_then(Value::as_object),
        ) {
            for key in ["vertLine", "horzLine"] {
                if let Some(default_line) = default.get(key) {
                    match current.get_mut(key) {
                        Some(existing) => deep_merge(existing, default_line),
                        None => {
                            current.insert(key.to_string(), default_line.clone());
                        }
                    }
                }
            }
            if let Some(default_tint) = default
                .get("shadeRight")
                .and_then(|shade| shade.get("color"))
            {
                let shade = current
                    .entry("shadeRight")
                    .or_insert_with(|| Value::Object(Map::new()));
                if !shade.is_object() {
                    *shade = Value::Object(Map::new());
                }
                if let Some(shade) = shade.as_object_mut() {
                    shade.insert("color".into(), default_tint.clone());
                }
            }
        }

        self.typed = serde_json::from_value(self.value.clone()).unwrap_or_default();
        self.generation = self.generation.wrapping_add(1);
    }
}

/// Convenience: build a one-key patch object `{ key: value }` for tests/host glue.
pub fn patch(key: &str, value: Value) -> Value {
    let mut m = Map::new();
    m.insert(key.into(), value);
    Value::Object(m)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn defaults_match_aeris_style() {
        let o = ChartOptions::default();
        assert_eq!(o.layout.background.color, DEFAULT_SURFACE_CSS);
        assert_eq!(o.layout.text_color, DEFAULT_FOREGROUND_CSS);
        assert_eq!(o.layout.muted_text_color, DEFAULT_MUTED_FOREGROUND_CSS);
        assert_eq!(o.layout.font_size, 12.0);
        assert_eq!(o.grid.vert_lines.color, DEFAULT_BORDER_CSS);
        assert_eq!(o.grid.horz_lines.style, line_style::DASHED);
        assert!(!o.grid.vert_lines.visible);
        assert!(!o.grid.horz_lines.visible);
        assert_eq!(o.crosshair.mode, crosshair_mode::NORMAL);
        assert!(!o.crosshair.do_not_snap_to_hidden_series_indices);
        assert_eq!(o.crosshair.vert_line.style, line_style::DASHED);
        assert_eq!(o.crosshair.vert_line.color, DEFAULT_CROSSHAIR_LINE_CSS);
        assert_eq!(
            o.crosshair.horz_line.label_background_color,
            DEFAULT_CROSSHAIR_LABEL_CSS
        );
        // The right-of-cursor veil ships off, tinted with the crosshair line token at 12% alpha.
        assert!(!o.crosshair.shade_right.visible);
        assert_eq!(o.crosshair.shade_right.color, "rgba(74, 74, 74, 0.12)");
        assert!(o.hovered_series_on_top);
        assert!(!o.auto_size);
        // Axis border cosmetics use the canonical border everywhere.
        assert!(o.right_price_scale.visible);
        assert!(!o.left_price_scale.visible);
        assert!(o.right_price_scale.border_visible);
        assert!(o.left_price_scale.border_visible);
        assert_eq!(o.right_price_scale.border_color, DEFAULT_BORDER_CSS);
        assert_eq!(o.left_price_scale.border_color, DEFAULT_BORDER_CSS);
        assert!(o.time_scale.border_visible);
        assert_eq!(o.time_scale.border_color, DEFAULT_BORDER_CSS);
        // Watermark defaults: hidden, transparent, 48px centered (reference v4).
        assert!(!o.watermark.visible);
        assert_eq!(o.watermark.color, "rgba(0, 0, 0, 0)");
        assert_eq!(o.watermark.font_size, 48.0);
        assert_eq!(o.watermark.horz_align, "center");
        assert_eq!(o.watermark.vert_align, "center");
    }

    #[test]
    fn canonical_theme_patch_switches_every_chart_chrome_role() {
        let mut store = ChartOptionsStore::new();
        store.apply(&chart_theme_patch(ChartTheme::Light));
        let light = store.get();
        assert_eq!(light.layout.background.color, LIGHT_SURFACE_CSS);
        assert_eq!(light.layout.text_color, LIGHT_FOREGROUND_CSS);
        assert_eq!(light.layout.muted_text_color, LIGHT_MUTED_FOREGROUND_CSS);
        assert_eq!(light.layout.bullish_color, LIGHT_MARKET_UP_CSS);
        assert_eq!(light.layout.bearish_color, LIGHT_MARKET_DOWN_CSS);
        assert_eq!(light.grid.vert_lines.color, LIGHT_BORDER_CSS);
        assert_eq!(light.crosshair.vert_line.color, LIGHT_CROSSHAIR_LINE_CSS);
        assert_eq!(
            light.crosshair.vert_line.label_background_color,
            LIGHT_CROSSHAIR_LABEL_CSS
        );
        assert_eq!(light.right_price_scale.border_color, LIGHT_BORDER_CSS);
        assert_eq!(
            light.right_price_scale.text_color.as_deref(),
            Some(LIGHT_FOREGROUND_CSS)
        );

        store.apply(&chart_theme_patch(ChartTheme::Dark));
        let dark = store.get();
        assert_eq!(dark.layout.background.color, DARK_SURFACE_CSS);
        assert_eq!(dark.layout.text_color, DARK_FOREGROUND_CSS);
        assert_eq!(dark.layout.muted_text_color, DARK_MUTED_FOREGROUND_CSS);
        assert_eq!(dark.layout.bullish_color, DARK_MARKET_UP_CSS);
        assert_eq!(dark.layout.bearish_color, DARK_MARKET_DOWN_CSS);
        assert_eq!(dark.grid.vert_lines.color, DARK_BORDER_CSS);
        assert_eq!(dark.crosshair.vert_line.color, DARK_CROSSHAIR_LINE_CSS);
        assert_eq!(
            dark.crosshair.vert_line.label_background_color,
            DARK_CROSSHAIR_LABEL_CSS
        );
        assert_eq!(dark.right_price_scale.border_color, DARK_BORDER_CSS);
        assert_eq!(
            dark.right_price_scale.text_color.as_deref(),
            Some(DARK_FOREGROUND_CSS)
        );
    }

    #[test]
    fn style_reset_is_theme_aware_and_preserves_behavioral_options() {
        let mut store = ChartOptionsStore::new();
        store.apply(&json!({
            "layout": {
                "background": { "color": "#123456" },
                "fontSize": 27,
                "panes": { "separatorColor": "#654321" }
            },
            "grid": { "vertLines": { "visible": false, "color": "#abcdef" } },
            "crosshair": {
                "mode": crosshair_mode::HIDDEN,
                "doNotSnapToHiddenSeriesIndices": true,
                "vertLine": { "color": "#abcdef", "width": 4 },
                "shadeRight": { "visible": true, "color": "#abcdef" }
            },
            "rightPriceScale": {
                "visible": false,
                "textColor": "#abcdef",
                "borderColor": "#abcdef",
                "alignLabels": false,
                "ticksVisible": true,
                "entireTextOnly": true,
                "minimumWidth": 88,
                "boldRoundLabels": false
            },
            "timeScale": { "borderColor": "#abcdef", "barSpacing": 17, "rightOffset": 9 },
            "watermark": {
                "visible": true,
                "text": "KEEP ME",
                "color": "#abcdef",
                "fontSize": 77,
                "horzAlign": "left"
            },
            "autoSize": true,
            "hoveredSeriesOnTop": false,
            "hostExtension": { "keep": 42 }
        }));

        store.reset_style_to_defaults(ChartTheme::Light);

        let options = store.get();
        assert_eq!(options.layout.background.color, LIGHT_SURFACE_CSS);
        assert_eq!(options.layout.text_color, LIGHT_FOREGROUND_CSS);
        assert_eq!(options.layout.font_size, 12.0);
        assert_eq!(options.layout.panes.separator_color, "");
        assert_eq!(options.grid.vert_lines.color, LIGHT_BORDER_CSS);
        assert!(!options.grid.vert_lines.visible);
        assert_eq!(options.crosshair.vert_line.color, LIGHT_CROSSHAIR_LINE_CSS);
        assert_eq!(options.crosshair.vert_line.width, 1.0);
        assert_eq!(options.crosshair.mode, crosshair_mode::HIDDEN);
        assert!(options.crosshair.do_not_snap_to_hidden_series_indices);
        // The veil's tint is style; whether it is shown is state (watermark precedent).
        assert!(options.crosshair.shade_right.visible);
        assert_eq!(
            options.crosshair.shade_right.color,
            "rgba(74, 74, 74, 0.12)"
        );
        assert_eq!(options.right_price_scale.border_color, LIGHT_BORDER_CSS);
        assert_eq!(options.right_price_scale.text_color, None);
        assert!(!options.right_price_scale.visible);
        assert!(!options.right_price_scale.align_labels);
        assert!(options.right_price_scale.entire_text_only);
        assert_eq!(options.right_price_scale.minimum_width, 88.0);
        assert_eq!(options.time_scale.border_color, LIGHT_BORDER_CSS);
        assert!(options.watermark.visible);
        assert_eq!(options.watermark.text, "KEEP ME");
        assert_eq!(options.watermark.color, "rgba(0, 0, 0, 0)");
        assert_eq!(options.watermark.font_size, 48.0);
        assert_eq!(options.watermark.horz_align, "center");
        assert!(options.auto_size);
        assert!(!options.hovered_series_on_top);

        let raw = store.value();
        assert_eq!(raw["timeScale"]["barSpacing"], 17);
        assert_eq!(raw["timeScale"]["rightOffset"], 9);
        assert_eq!(raw["rightPriceScale"]["ticksVisible"], false);
        assert_eq!(raw["rightPriceScale"]["boldRoundLabels"], true);
        assert_eq!(raw["hostExtension"]["keep"], 42);
    }

    #[test]
    fn crosshair_shade_patch_merges_camelcase_keys() {
        let mut store = ChartOptionsStore::new();
        store.apply(&json!({ "crosshair": { "shadeRight": { "visible": true } } }));
        let o = store.get();
        assert!(o.crosshair.shade_right.visible);
        assert_eq!(o.crosshair.shade_right.color, "rgba(74, 74, 74, 0.12)");
        // untouched siblings keep their defaults
        assert!(o.crosshair.vert_line.visible);
        assert_eq!(o.crosshair.mode, crosshair_mode::NORMAL);
        // The wire key is the reference-style camelCase group name.
        let serialized = serde_json::to_value(o).unwrap();
        assert_eq!(serialized["crosshair"]["shadeRight"]["visible"], true);
        assert!(serialized["crosshair"].get("shade_right").is_none());

        // A style reset on a store that never named the group still restores the tint.
        let mut store = ChartOptionsStore::new();
        store.apply(&json!({ "crosshair": { "vertLine": { "width": 3 } } }));
        store.reset_style_to_defaults(ChartTheme::Dark);
        let o = store.get();
        assert!(!o.crosshair.shade_right.visible);
        assert_eq!(o.crosshair.shade_right.color, "rgba(74, 74, 74, 0.12)");
    }

    #[test]
    fn watermark_patch_merges_camelcase_keys() {
        let mut store = ChartOptionsStore::new();
        store.apply(&json!({
            "watermark": { "visible": true, "text": "Aeris", "fontSize": 64, "horzAlign": "left" },
        }));
        let o = store.get();
        assert!(o.watermark.visible);
        assert_eq!(o.watermark.text, "Aeris");
        assert_eq!(o.watermark.font_size, 64.0);
        assert_eq!(o.watermark.horz_align, "left");
        // untouched siblings keep their defaults
        assert_eq!(o.watermark.vert_align, "center");
        assert_eq!(o.watermark.color, "rgba(0, 0, 0, 0)");
    }

    #[test]
    fn axis_border_patch_merges_without_touching_siblings() {
        let mut store = ChartOptionsStore::new();
        store.apply(&json!({
            "rightPriceScale": { "borderColor": "#ff0000" },
            "timeScale": { "borderVisible": false },
        }));
        let o = store.get();
        assert_eq!(o.right_price_scale.border_color, "#ff0000");
        // untouched siblings survive: strip visibility and the other border options
        assert!(o.right_price_scale.visible);
        assert!(o.right_price_scale.border_visible);
        assert_eq!(o.left_price_scale.border_color, DEFAULT_BORDER_CSS);
        assert!(!o.time_scale.border_visible);
        assert_eq!(o.time_scale.border_color, DEFAULT_BORDER_CSS);
    }

    #[test]
    fn deep_merge_overrides_nested_leaf_only() {
        let mut store = ChartOptionsStore::new();
        store.apply(&json!({ "grid": { "vertLines": { "color": "#000000" } } }));
        let o = store.get();
        // the targeted leaf changed...
        assert_eq!(o.grid.vert_lines.color, "#000000");
        // ...siblings within the same object survived...
        assert_eq!(o.grid.vert_lines.style, line_style::DASHED);
        assert!(!o.grid.vert_lines.visible);
        // ...and the neighbouring family is untouched.
        assert_eq!(o.grid.horz_lines.color, DEFAULT_BORDER_CSS);
    }

    #[test]
    fn successive_applies_accumulate() {
        let mut store = ChartOptionsStore::new();
        store.apply(&json!({ "grid": { "vertLines": { "color": "#111111" } } }));
        store.apply(&json!({ "crosshair": { "mode": crosshair_mode::NORMAL } }));
        let o = store.get();
        // first patch persists through the second
        assert_eq!(o.grid.vert_lines.color, "#111111");
        assert_eq!(o.crosshair.mode, crosshair_mode::NORMAL);
        // and unrelated defaults remain
        assert_eq!(o.layout.background.color, DEFAULT_SURFACE_CSS);
    }

    #[test]
    fn typed_options_are_retained_between_frame_reads() {
        let mut store = ChartOptionsStore::new();
        let before = std::ptr::from_ref(store.get());
        assert_eq!(before, std::ptr::from_ref(store.get()));
        store.apply(&json!({ "layout": { "fontSize": 18 } }));
        assert_eq!(store.get().layout.font_size, 18.0);
        assert_eq!(store.value()["layout"]["fontSize"], 18);
    }

    #[test]
    fn crosshair_do_not_snap_patch_merges_camelcase_key() {
        let mut store = ChartOptionsStore::new();
        store.apply(&json!({ "crosshair": { "doNotSnapToHiddenSeriesIndices": true } }));
        let o = store.get();
        assert!(o.crosshair.do_not_snap_to_hidden_series_indices);
        // untouched siblings keep their defaults
        assert_eq!(o.crosshair.mode, crosshair_mode::NORMAL);
    }

    #[test]
    fn scalar_and_bool_replace() {
        let mut store = ChartOptionsStore::new();
        store.apply(&json!({ "layout": { "fontSize": 16 } }));
        let o = store.get();
        assert_eq!(o.layout.font_size, 16.0);
    }

    #[test]
    fn retired_attribution_option_is_not_retained() {
        let mut store = ChartOptionsStore::new();
        store.apply(&json!({
            "layout": { "attributionLogo": true, "fontSize": 15 }
        }));
        assert_eq!(store.get().layout.font_size, 15.0);
        assert!(store.value()["layout"].get("attributionLogo").is_none());
    }

    #[test]
    fn apply_str_parses_and_merges() {
        let mut store = ChartOptionsStore::new();
        store
            .apply_str(r##"{ "layout": { "background": { "color": "#0d0d0d" } } }"##)
            .unwrap();
        assert_eq!(store.get().layout.background.color, "#0d0d0d");
    }

    #[test]
    fn apply_str_rejects_malformed_without_mutating() {
        let mut store = ChartOptionsStore::new();
        let before = store.get().clone();
        assert!(store.apply_str("{ not valid json ").is_err());
        assert_eq!(store.get(), &before);
    }

    #[test]
    fn unknown_keys_are_ignored_on_read() {
        // A patch with keys we don't model must not break deserialization of the ones we do.
        let mut store = ChartOptionsStore::new();
        store
            .apply(&json!({ "grid": { "vertLines": { "color": "#abcabc" } }, "somethingNew": 42 }));
        assert_eq!(store.get().grid.vert_lines.color, "#abcabc");
    }

    #[test]
    fn deep_merge_null_overwrites() {
        let mut v = json!({ "a": { "b": 1 } });
        deep_merge(&mut v, &json!({ "a": { "b": null } }));
        assert_eq!(v, json!({ "a": { "b": null } }));
    }
}
