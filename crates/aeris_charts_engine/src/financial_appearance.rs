//! Typed financial-chart appearance and theme provenance.

use crate::{ChartEngine, ChartTheme, SeriesId};
use aeris_charts_core::style::{
    DARK_BORDER_CSS, DARK_CROSSHAIR_LINE_CSS, DARK_MARKET_DOWN_CSS, DARK_MARKET_UP_CSS,
    LIGHT_BORDER_CSS, LIGHT_CROSSHAIR_LINE_CSS, LIGHT_MARKET_DOWN_CSS, LIGHT_MARKET_UP_CSS,
};

/// A color either follows its field's canonical semantic theme role or is explicitly pinned.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AppearanceColor {
    Theme,
    Custom(String),
}

impl AppearanceColor {
    fn custom_or(&self, themed: &str) -> String {
        match self {
            Self::Theme => themed.to_string(),
            Self::Custom(color) => color.clone(),
        }
    }

    fn override_value(&self) -> Option<String> {
        match self {
            Self::Theme => None,
            Self::Custom(color) => Some(color.clone()),
        }
    }
}

/// Canonical effective colors for the semantic roles exposed by financial appearance controls.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FinancialThemeColors {
    pub grid: &'static str,
    pub crosshair: &'static str,
    pub bullish: &'static str,
    pub bearish: &'static str,
}

impl FinancialThemeColors {
    #[must_use]
    pub const fn for_theme(theme: ChartTheme) -> Self {
        match theme {
            ChartTheme::Light => Self {
                grid: LIGHT_BORDER_CSS,
                crosshair: LIGHT_CROSSHAIR_LINE_CSS,
                bullish: LIGHT_MARKET_UP_CSS,
                bearish: LIGHT_MARKET_DOWN_CSS,
            },
            ChartTheme::Dark => Self {
                grid: DARK_BORDER_CSS,
                crosshair: DARK_CROSSHAIR_LINE_CSS,
                bullish: DARK_MARKET_UP_CSS,
                bearish: DARK_MARKET_DOWN_CSS,
            },
        }
    }
}

/// Durable financial canvas and primary-series appearance with explicit theme provenance.
#[derive(Clone, Debug, PartialEq)]
#[allow(clippy::struct_excessive_bools)]
pub struct FinancialAppearance {
    pub grid_visible: bool,
    pub grid_color: AppearanceColor,
    pub grid_style: u8,
    pub crosshair_color: AppearanceColor,
    pub crosshair_width: u8,
    pub crosshair_style: u8,
    pub up_color: AppearanceColor,
    pub down_color: AppearanceColor,
    pub wick_up_color: AppearanceColor,
    pub wick_down_color: AppearanceColor,
    pub border_up_color: AppearanceColor,
    pub border_down_color: AppearanceColor,
    pub wick_visible: bool,
    pub border_visible: bool,
    pub open_visible: bool,
    pub thin_bars: bool,
    pub line_color: String,
    pub line_width: u8,
    pub line_style: u8,
    pub area_top_color: String,
    pub baseline_top_color: String,
    pub baseline_bottom_color: String,
}

impl Default for FinancialAppearance {
    fn default() -> Self {
        Self {
            grid_visible: true,
            grid_color: AppearanceColor::Theme,
            grid_style: 2,
            crosshair_color: AppearanceColor::Theme,
            crosshair_width: 1,
            crosshair_style: 2,
            up_color: AppearanceColor::Theme,
            down_color: AppearanceColor::Theme,
            wick_up_color: AppearanceColor::Theme,
            wick_down_color: AppearanceColor::Theme,
            border_up_color: AppearanceColor::Theme,
            border_down_color: AppearanceColor::Theme,
            wick_visible: true,
            border_visible: true,
            open_visible: true,
            thin_bars: true,
            line_color: "#2196f3".to_string(),
            line_width: 2,
            line_style: 0,
            area_top_color: "#089981".to_string(),
            baseline_top_color: "#089981".to_string(),
            baseline_bottom_color: "#f7525f".to_string(),
        }
    }
}

impl FinancialAppearance {
    #[must_use]
    pub fn effective_grid_color(&self, theme: ChartTheme) -> String {
        self.grid_color
            .custom_or(FinancialThemeColors::for_theme(theme).grid)
    }

    #[must_use]
    pub fn effective_crosshair_color(&self, theme: ChartTheme) -> String {
        self.crosshair_color
            .custom_or(FinancialThemeColors::for_theme(theme).crosshair)
    }

    #[must_use]
    pub fn effective_up_color(&self, theme: ChartTheme) -> String {
        self.up_color
            .custom_or(FinancialThemeColors::for_theme(theme).bullish)
    }

    #[must_use]
    pub fn effective_down_color(&self, theme: ChartTheme) -> String {
        self.down_color
            .custom_or(FinancialThemeColors::for_theme(theme).bearish)
    }
}

impl ChartEngine {
    /// Typed appearance snapshot. Theme-following colors remain semantic rather than being
    /// flattened to whichever theme happened to be active when the host persisted the value.
    #[must_use]
    pub fn financial_appearance(&self, series_id: SeriesId) -> Option<FinancialAppearance> {
        let series = self
            .series
            .iter()
            .find(|series| series.id == series_id && !series.removed)?;
        let options = self.options.get();
        let mut appearance = FinancialAppearance::default();
        appearance.grid_visible =
            options.grid.vert_lines.visible && options.grid.horz_lines.visible;
        appearance.grid_color = if self.grid_color_follows_theme {
            AppearanceColor::Theme
        } else {
            AppearanceColor::Custom(options.grid.vert_lines.color.clone())
        };
        appearance.grid_style = options.grid.vert_lines.style.min(4);
        appearance.crosshair_color = if self.crosshair_color_follows_theme {
            AppearanceColor::Theme
        } else {
            AppearanceColor::Custom(options.crosshair.vert_line.color.clone())
        };
        appearance.crosshair_width = bounded_width(options.crosshair.vert_line.width);
        appearance.crosshair_style = options.crosshair.vert_line.style.min(4);
        appearance.up_color = color_provenance(&series.up_color);
        appearance.down_color = color_provenance(&series.down_color);
        appearance.wick_up_color = color_provenance(&series.wick_up_color);
        appearance.wick_down_color = color_provenance(&series.wick_down_color);
        appearance.border_up_color = color_provenance(&series.border_up_color);
        appearance.border_down_color = color_provenance(&series.border_down_color);
        appearance.wick_visible = series.wick_visible.unwrap_or(true);
        appearance.border_visible = series.border_visible.unwrap_or(true);
        appearance.open_visible = series.open_visible;
        appearance.thin_bars = series.thin_bars;
        appearance.line_color = series
            .line_color
            .clone()
            .unwrap_or_else(|| crate::frame::series_stroke_color(series).to_css());
        appearance.line_width =
            bounded_width(series.line_width.unwrap_or(crate::frame::LINE_WIDTH));
        appearance.line_style = series.line_style.min(4);
        appearance.area_top_color = series
            .area_top_color
            .clone()
            .unwrap_or_else(|| appearance.area_top_color.clone());
        appearance.baseline_top_color = series
            .top_line_color
            .clone()
            .unwrap_or_else(|| appearance.baseline_top_color.clone());
        appearance.baseline_bottom_color = series
            .bottom_line_color
            .clone()
            .unwrap_or_else(|| appearance.baseline_bottom_color.clone());
        Some(appearance)
    }

    /// Apply only canvas appearance, retaining typed follow-theme provenance.
    pub fn apply_financial_canvas_appearance(&mut self, appearance: &FinancialAppearance) -> bool {
        let before = self.financial_appearance(0);
        let colors = FinancialThemeColors::for_theme(self.theme);
        self.grid_color_follows_theme = matches!(appearance.grid_color, AppearanceColor::Theme);
        self.crosshair_color_follows_theme =
            matches!(appearance.crosshair_color, AppearanceColor::Theme);
        let patch = serde_json::json!({
            "grid": {
                "vertLines": {
                    "visible": appearance.grid_visible,
                    "color": appearance.grid_color.custom_or(colors.grid),
                    "style": appearance.grid_style.min(4),
                },
                "horzLines": {
                    "visible": appearance.grid_visible,
                    "color": appearance.grid_color.custom_or(colors.grid),
                    "style": appearance.grid_style.min(4),
                }
            },
            "crosshair": {
                "vertLine": {
                    "color": appearance.crosshair_color.custom_or(colors.crosshair),
                    "width": appearance.crosshair_width.clamp(1, 4),
                    "style": appearance.crosshair_style.min(4),
                },
                "horzLine": {
                    "color": appearance.crosshair_color.custom_or(colors.crosshair),
                    "width": appearance.crosshair_width.clamp(1, 4),
                    "style": appearance.crosshair_style.min(4),
                }
            }
        });
        self.options.apply(&patch);
        self.invalidate_frame_all();
        before != self.financial_appearance(0)
    }

    /// Apply only primary-series appearance without JSON sentinels or host-side field knowledge.
    pub fn apply_financial_series_appearance(
        &mut self,
        series_id: SeriesId,
        appearance: &FinancialAppearance,
    ) -> bool {
        let before = self.financial_appearance(series_id);
        let Some(series) = self
            .series
            .iter_mut()
            .find(|series| series.id == series_id && !series.removed)
        else {
            return false;
        };
        series.up_color = appearance.up_color.override_value();
        series.down_color = appearance.down_color.override_value();
        series.wick_up_color = appearance.wick_up_color.override_value();
        series.wick_down_color = appearance.wick_down_color.override_value();
        series.border_up_color = appearance.border_up_color.override_value();
        series.border_down_color = appearance.border_down_color.override_value();
        series.wick_visible = Some(appearance.wick_visible);
        series.border_visible = Some(appearance.border_visible);
        series.open_visible = appearance.open_visible;
        series.thin_bars = appearance.thin_bars;
        series.line_color = Some(appearance.line_color.clone());
        series.line_width = Some(f64::from(appearance.line_width.clamp(1, 4)));
        series.line_style = appearance.line_style.min(4);
        series.area_top_color = Some(appearance.area_top_color.clone());
        series.top_line_color = Some(appearance.baseline_top_color.clone());
        series.bottom_line_color = Some(appearance.baseline_bottom_color.clone());
        self.invalidate_frame_all();
        before != self.financial_appearance(series_id)
    }
}

fn color_provenance(color: &Option<String>) -> AppearanceColor {
    color.as_ref().map_or(AppearanceColor::Theme, |color| {
        AppearanceColor::Custom(color.clone())
    })
}

fn bounded_width(width: f64) -> u8 {
    if !width.is_finite() {
        return 1;
    }
    width.round().clamp(1.0, 4.0) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_theme_provenance_survives_theme_changes() {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        let mut appearance = chart.financial_appearance(0).unwrap();
        appearance.grid_color = AppearanceColor::Custom("#123456".into());
        appearance.crosshair_color = AppearanceColor::Theme;
        assert!(chart.apply_financial_canvas_appearance(&appearance));

        chart.set_theme(ChartTheme::Light);
        let light = chart.financial_appearance(0).unwrap();
        assert_eq!(light.grid_color, AppearanceColor::Custom("#123456".into()));
        assert_eq!(light.crosshair_color, AppearanceColor::Theme);
        assert_eq!(
            chart.options.get().crosshair.vert_line.color,
            FinancialThemeColors::for_theme(ChartTheme::Light).crosshair
        );

        chart.set_theme(ChartTheme::Dark);
        assert_eq!(chart.options.get().grid.vert_lines.color, "#123456");
        assert_eq!(
            chart.options.get().crosshair.vert_line.color,
            FinancialThemeColors::for_theme(ChartTheme::Dark).crosshair
        );
    }

    #[test]
    fn typed_series_theme_colors_clear_overrides_without_json_sentinels() {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        let mut appearance = chart.financial_appearance(0).unwrap();
        appearance.up_color = AppearanceColor::Custom("#abcdef".into());
        appearance.wick_up_color = AppearanceColor::Custom("#fedcba".into());
        assert!(chart.apply_financial_series_appearance(0, &appearance));
        assert_eq!(
            chart.financial_appearance(0).unwrap().up_color,
            AppearanceColor::Custom("#abcdef".into())
        );

        appearance.up_color = AppearanceColor::Theme;
        appearance.wick_up_color = AppearanceColor::Theme;
        assert!(chart.apply_financial_series_appearance(0, &appearance));
        let restored = chart.financial_appearance(0).unwrap();
        assert_eq!(restored.up_color, AppearanceColor::Theme);
        assert_eq!(restored.wick_up_color, AppearanceColor::Theme);
        assert!(chart
            .series
            .iter()
            .find(|series| series.id == 0)
            .unwrap()
            .up_color
            .is_none());
    }

    #[test]
    fn raw_canvas_color_patch_is_explicit_and_is_not_retokenized() {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        chart
            .apply_options(r##"{"grid":{"vertLines":{"color":"#112233"}}}"##)
            .unwrap();
        chart.set_theme(ChartTheme::Light);
        assert_eq!(chart.options.get().grid.vert_lines.color, "#112233");
        assert!(matches!(
            chart.financial_appearance(0).unwrap().grid_color,
            AppearanceColor::Custom(_)
        ));
    }
}
