//! Canonical Aeris styling tokens shared by every rendering backend.

include!(concat!(env!("OUT_DIR"), "/style_tokens.rs"));

/// `--border-width` (1 CSS px) in device pixels with browser border semantics: whole device
/// pixels, rounded down, never thinner than one. At DPR 1 and 1.5 this is one device pixel; at
/// DPR 2, two.
pub fn border_width_device_px(pixel_ratio: f64) -> f64 {
    (BORDER_WIDTH * pixel_ratio).floor().max(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brand_border_snaps_like_a_browser_border() {
        for (ratio, expected) in [(1.0, 1.0), (1.25, 1.0), (1.5, 1.0), (2.0, 2.0), (3.0, 3.0)] {
            assert_eq!(border_width_device_px(ratio), expected, "dpr {ratio}");
        }
    }

    #[test]
    fn canonical_dark_and_market_tokens_are_exact() {
        assert_eq!(DEFAULT_THEME_NAME, "dark");
        assert_eq!(BORDER_WIDTH, 1.0);
        assert_eq!(RADIUS_SMALL, 4.0);
        assert_eq!(RADIUS_DEFAULT, 8.0);
        assert_eq!(RADIUS_MEDIUM, 12.0);
        assert_eq!(RADIUS_LARGE, 999.0);
        assert_eq!(LIGHT_SURFACE_CSS, "#ffffff");
        assert_eq!(LIGHT_FOREGROUND_CSS, "#222222");
        assert_eq!(LIGHT_MUTED_CSS, "#fafafa");
        assert_eq!(LIGHT_MUTED_FOREGROUND_CSS, "#646465");
        assert_eq!(LIGHT_PRIMARY_CSS, "#0091ff");
        assert_eq!(LIGHT_PRIMARY_FOREGROUND_CSS, "#ffffff");
        assert_eq!(LIGHT_PRIMARY_HOVER_CSS, "#0077fa");
        assert_eq!(LIGHT_DANGER_CSS, "#f7525f");
        assert_eq!(LIGHT_ACCENT_CSS, "#f0f0f0");
        assert_eq!(LIGHT_ACTIVE_CSS, "#e5e5e5");
        assert_eq!(DARK_ACCENT_CSS, "#333333");
        assert_eq!(DARK_ACTIVE_CSS, "#404040");
        assert_eq!(LIGHT_BORDER_CSS, "#e5e5e5");
        assert_eq!(LIGHT_MUTED_BORDER_CSS, LIGHT_BORDER_CSS);
        assert_eq!(LIGHT_RING_CSS, "#e0e0e0");
        assert_eq!(DARK_SURFACE_CSS, "#1f1f1f");
        assert_eq!(DARK_FOREGROUND_CSS, "#f5f5f5");
        assert_eq!(DARK_MUTED_CSS, "#222222");
        assert_eq!(DARK_MUTED_FOREGROUND_CSS, "#c2c2c2");
        assert_eq!(DARK_PRIMARY_CSS, "#0091ff");
        assert_eq!(DARK_PRIMARY_FOREGROUND_CSS, "#ffffff");
        assert_eq!(DARK_PRIMARY_HOVER_CSS, "#0077fa");
        assert_eq!(DARK_DANGER_CSS, LIGHT_DANGER_CSS);
        assert_eq!(DARK_BORDER_CSS, "#333333");
        assert_eq!(DARK_CROSSHAIR_CSS, DARK_BORDER_CSS);
        assert_eq!(DARK_CROSSHAIR_LABEL_CSS, DARK_MUTED_CSS);
        assert_eq!(LIGHT_CROSSHAIR_CSS, DARK_BORDER_CSS);
        assert_eq!(LIGHT_CROSSHAIR_LABEL_CSS, DARK_MUTED_CSS);
        assert_eq!(DARK_SEPARATOR_HOVER_CSS, DARK_ACCENT_CSS);
        assert_eq!(LIGHT_MARKET_UP_CSS, "#089981");
        assert_eq!(LIGHT_MARKET_DOWN_CSS, "#f7525f");
        assert_eq!(DARK_MARKET_UP_CSS, "#089981");
        assert_eq!(DARK_MARKET_DOWN_CSS, "#f7525f");
        assert_eq!(MARKET_UP_CSS, LIGHT_MARKET_UP_CSS);
        assert_eq!(MARKET_DOWN_CSS, LIGHT_MARKET_DOWN_CSS);
        assert_eq!(DEFAULT_SURFACE_CSS, DARK_SURFACE_CSS);
        assert_eq!(DEFAULT_BORDER_CSS, DARK_BORDER_CSS);
        assert_eq!(DEFAULT_FOREGROUND_CSS, DARK_FOREGROUND_CSS);
        assert_eq!(DEFAULT_MUTED_CSS, DARK_MUTED_CSS);
        assert_eq!(DEFAULT_SEPARATOR_HOVER_CSS, DARK_SEPARATOR_HOVER_CSS);
    }
}
