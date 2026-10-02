//! Simple RGBA8 color for draw lists.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Color(pub u32); // 0xRRGGBBAA

impl Color {
    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Color(((r as u32) << 24) | ((g as u32) << 16) | ((b as u32) << 8) | a as u32)
    }

    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self::rgba(r, g, b, 0xFF)
    }

    /// Parses "#RGB", "#RGBA", "#RRGGBB", or "#RRGGBBAA".
    pub fn from_hex(s: &str) -> Option<Self> {
        let s = s.strip_prefix('#')?;
        match s.len() {
            // shorthand: each nibble is doubled (#abc -> #aabbcc)
            3 | 4 => {
                let mut out: u32 = 0;
                for (i, c) in s.chars().enumerate() {
                    let n = c.to_digit(16)?;
                    let byte = n * 17; // 0xN -> 0xNN
                    out |= byte << (8 * (3 - i));
                }
                if s.len() == 3 {
                    out |= 0xFF; // opaque
                }
                Some(Color(out))
            }
            6 => {
                let v = u32::from_str_radix(s, 16).ok()?;
                Some(Color((v << 8) | 0xFF))
            }
            8 => u32::from_str_radix(s, 16).ok().map(Color),
            _ => None,
        }
    }

    /// Parses a CSS color string: the `transparent` keyword, hex
    /// (`#rgb`/`#rgba`/`#rrggbb`/`#rrggbbaa`), or the functional `rgb(r, g, b)` /
    /// `rgba(r, g, b, a)` forms (r/g/b are 0–255 integers, a is 0–1 float). Whitespace-tolerant;
    /// returns `None` for anything unrecognized (named colors, hsl, etc.).
    ///
    /// `transparent` is the canonical spelling for "no fill" — a candlestick body set to it is
    /// hollow, showing only its border and wick. Without it the keyword would fail to parse and
    /// silently fall back to the series' solid default.
    pub fn parse_css(s: &str) -> Option<Self> {
        let s = s.trim();
        if s.starts_with('#') {
            return Self::from_hex(s);
        }
        let lower = s.to_ascii_lowercase();
        if lower == "transparent" {
            return Some(Color::rgba(0, 0, 0, 0));
        }
        let inner = lower
            .strip_prefix("rgba(")
            .or_else(|| lower.strip_prefix("rgb("))?;
        let inner = inner.strip_suffix(')')?;
        let mut parts = inner.split(',').map(str::trim);
        let r: f64 = parts.next()?.parse().ok()?;
        let g: f64 = parts.next()?.parse().ok()?;
        let b: f64 = parts.next()?.parse().ok()?;
        let a: f64 = match parts.next() {
            Some(a) => a.parse().ok()?,
            None => 1.0,
        };
        if parts.next().is_some() {
            return None; // too many components
        }
        let clamp8 = |v: f64| v.round().clamp(0.0, 255.0) as u8;
        Some(Color::rgba(
            clamp8(r),
            clamp8(g),
            clamp8(b),
            clamp8(a * 255.0),
        ))
    }

    pub const fn r(&self) -> u8 {
        (self.0 >> 24) as u8
    }
    pub const fn g(&self) -> u8 {
        (self.0 >> 16) as u8
    }
    pub const fn b(&self) -> u8 {
        (self.0 >> 8) as u8
    }
    pub const fn a(&self) -> u8 {
        self.0 as u8
    }

    /// Perceptual luminance (Rec. 601), 0..255.
    pub fn luminance(&self) -> f64 {
        0.299 * self.r() as f64 + 0.587 * self.g() as f64 + 0.114 * self.b() as f64
    }

    /// Contrast text color for a label on this background — black on light, white on dark.
    /// Approximates the reference's `generateContrastColors`.
    pub fn contrast_text(&self) -> Color {
        if self.luminance() > 160.0 {
            Color::rgb(0, 0, 0)
        } else {
            Color::rgb(0xFF, 0xFF, 0xFF)
        }
    }

    /// Choose opaque black or white by the larger WCAG contrast ratio against this RGB.
    /// sRGB channels are linearized before relative luminance is computed. Callers resolve
    /// alpha against the painted surface first; this method treats RGB as opaque.
    pub fn contrast_srgb(&self) -> Color {
        let linear = |channel: u8| {
            let srgb = channel as f64 / 255.0;
            if srgb <= 0.04045 {
                srgb / 12.92
            } else {
                ((srgb + 0.055) / 1.055).powf(2.4)
            }
        };
        let luminance =
            0.2126 * linear(self.r()) + 0.7152 * linear(self.g()) + 0.0722 * linear(self.b());
        // (L+.05)/.05 >= 1.05/(L+.05) exactly when L >= sqrt(.0525)-.05.
        if luminance >= 0.179128784747792 {
            Color::rgb(0, 0, 0)
        } else {
            Color::rgb(255, 255, 255)
        }
    }

    /// Contrast text after compositing this color over an opaque surface. Opaque label colors
    /// take the existing fast path; translucent labels follow the color users actually see.
    pub fn contrast_text_over(&self, surface: Color) -> Color {
        if self.a() == u8::MAX {
            return self.contrast_text();
        }
        let alpha = self.a() as u32;
        let inverse = u8::MAX as u32 - alpha;
        let blend = |foreground: u8, background: u8| {
            ((foreground as u32 * alpha + background as u32 * inverse + 127) / 255) as u8
        };
        Color::rgb(
            blend(self.r(), surface.r()),
            blend(self.g(), surface.g()),
            blend(self.b(), surface.b()),
        )
        .contrast_text()
    }

    /// Same hue at full opacity: RGB preserved, alpha forced to 0xFF. Used for the last-value
    /// cluster's chip backgrounds (title/price/countdown), which follow the series color but
    /// must never turn translucent when that color carries alpha (industry-standard).
    pub fn solid(&self) -> Color {
        Color::rgba(self.r(), self.g(), self.b(), 0xFF)
    }

    /// Darker shade of this color: every sRGB channel scaled by `factor` (clamped to 0..=1),
    /// alpha preserved. Used for the title chip of the last-value label cluster, which renders
    /// in a darker shade of the label color (industry-standard).
    pub fn darken(&self, factor: f64) -> Color {
        let f = factor.clamp(0.0, 1.0);
        Color::rgba(
            (self.r() as f64 * f).round() as u8,
            (self.g() as f64 * f).round() as u8,
            (self.b() as f64 * f).round() as u8,
            self.a(),
        )
    }

    /// Lighter shade of this color: every sRGB channel blended toward white by `factor`
    /// (clamped to 0..=1), alpha preserved. Used for the selected series' axis-chip accent,
    /// which must read as the same hue one step brighter.
    pub fn lighten(&self, factor: f64) -> Color {
        let f = factor.clamp(0.0, 1.0);
        let mix = |c: u8| (c as f64 + (255.0 - c as f64) * f).round() as u8;
        Color::rgba(mix(self.r()), mix(self.g()), mix(self.b()), self.a())
    }

    /// CSS `#rrggbb` string (ignores alpha).
    pub fn to_hex(&self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.r(), self.g(), self.b())
    }

    /// CSS color string that preserves alpha: `#rrggbb` when opaque, the functional
    /// `rgba(r, g, b, a)` form otherwise (unlike `to_hex`, which always drops alpha).
    /// Round-trips through [`Color::parse_css`].
    pub fn to_css(&self) -> String {
        if self.a() == 0xFF {
            self.to_hex()
        } else {
            format!(
                "rgba({},{},{},{})",
                self.r(),
                self.g(),
                self.b(),
                self.a() as f64 / 255.0
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_parsing() {
        assert_eq!(
            Color::from_hex("#089981"),
            Some(Color::rgb(0x08, 0x99, 0x81))
        );
        assert_eq!(
            Color::from_hex("#08998180"),
            Some(Color::rgba(0x08, 0x99, 0x81, 0x80))
        );
        assert_eq!(Color::from_hex("oops"), None);
    }

    #[test]
    fn transparent_keyword_is_a_zero_alpha_color() {
        // The canonical "no fill" spelling: a candlestick body set to it paints nothing, leaving
        // the border and wick — a hollow candle. Case- and whitespace-tolerant like the rest.
        assert_eq!(
            Color::parse_css("transparent"),
            Some(Color::rgba(0, 0, 0, 0))
        );
        assert_eq!(
            Color::parse_css("  Transparent "),
            Some(Color::rgba(0, 0, 0, 0))
        );
        assert_eq!(Color::parse_css("transparent").unwrap().a(), 0);
        // Equivalent to the two spellings that already worked.
        assert_eq!(
            Color::parse_css("transparent"),
            Color::parse_css("rgba(0,0,0,0)")
        );
        assert_eq!(
            Color::parse_css("transparent"),
            Color::parse_css("#00000000")
        );
        // Still not a general named-color table.
        assert_eq!(Color::parse_css("rebeccapurple"), None);
    }

    #[test]
    fn hex_shorthand() {
        assert_eq!(Color::from_hex("#abc"), Some(Color::rgb(0xaa, 0xbb, 0xcc)));
        assert_eq!(Color::from_hex("#f00"), Some(Color::rgb(0xff, 0x00, 0x00)));
        // #RGBA -> alpha nibble doubled
        assert_eq!(
            Color::from_hex("#0f08"),
            Some(Color::rgba(0x00, 0xff, 0x00, 0x88))
        );
    }

    #[test]
    fn css_functional_parsing() {
        assert_eq!(
            Color::parse_css("rgb(38, 166, 154)"),
            Some(Color::rgb(0x26, 0xa6, 0x9a))
        );
        assert_eq!(
            Color::parse_css("rgba(38,166,154,1)"),
            Some(Color::rgb(0x26, 0xa6, 0x9a))
        );
        // half alpha rounds to 128
        assert_eq!(
            Color::parse_css("rgba(0, 0, 0, 0.5)"),
            Some(Color::rgba(0, 0, 0, 128))
        );
        // hex still works through parse_css
        assert_eq!(
            Color::parse_css("  #FFFFFF "),
            Some(Color::rgb(0xff, 0xff, 0xff))
        );
        // unsupported forms
        assert_eq!(Color::parse_css("red"), None);
        assert_eq!(Color::parse_css("rgb(1,2)"), None);
        assert_eq!(Color::parse_css("rgb(1,2,3,4,5)"), None);
    }

    #[test]
    fn contrast_and_hex() {
        // dark teal -> white text; light gray -> black text
        assert_eq!(
            Color::rgb(0x26, 0xa6, 0x9a).contrast_text(),
            Color::rgb(0xFF, 0xFF, 0xFF)
        );
        assert_eq!(
            Color::rgb(0xe0, 0xe3, 0xeb).contrast_text(),
            Color::rgb(0, 0, 0)
        );
        assert_eq!(Color::rgb(0x08, 0x99, 0x81).to_hex(), "#089981");
    }

    #[test]
    fn translucent_contrast_uses_the_composited_surface() {
        let translucent_white = Color::rgba(255, 255, 255, 64);
        assert_eq!(
            translucent_white.contrast_text_over(Color::rgb(0, 0, 0)),
            Color::rgb(255, 255, 255)
        );
        assert_eq!(
            translucent_white.contrast_text_over(Color::rgb(255, 255, 255)),
            Color::rgb(0, 0, 0)
        );
    }

    #[test]
    fn srgb_contrast_linearizes_saturated_colors_and_switches_at_equal_ratios() {
        let black = Color::rgb(0, 0, 0);
        let white = Color::rgb(255, 255, 255);
        for (color, expected) in [
            (black, white),
            (white, black),
            (Color::rgb(255, 0, 0), black),
            (Color::rgb(0, 255, 0), black),
            (Color::rgb(0, 0, 255), white),
            (Color::rgb(117, 117, 117), white),
            (Color::rgb(118, 118, 118), black),
        ] {
            assert_eq!(color.contrast_srgb(), expected);
        }
    }

    #[test]
    fn solid_forces_full_opacity_keeping_hue() {
        assert_eq!(
            Color::rgba(0x26, 0xa6, 0x9a, 0x80).solid(),
            Color::rgb(0x26, 0xa6, 0x9a)
        );
        // Already-opaque colors pass through unchanged.
        assert_eq!(Color::rgb(10, 20, 30).solid(), Color::rgb(10, 20, 30));
    }

    #[test]
    fn darken_scales_channels_and_preserves_alpha() {
        assert_eq!(
            Color::rgb(0xef, 0x53, 0x50).darken(0.72),
            Color::rgb(172, 60, 58)
        );
        // Alpha passes through untouched; the factor clamps to 0..=1.
        assert_eq!(
            Color::rgba(100, 200, 50, 0x80).darken(0.5),
            Color::rgba(50, 100, 25, 0x80)
        );
        assert_eq!(Color::rgb(10, 20, 30).darken(2.0), Color::rgb(10, 20, 30));
        assert_eq!(Color::rgb(10, 20, 30).darken(-1.0), Color::rgb(0, 0, 0));
    }

    #[test]
    fn to_css_preserves_alpha_and_round_trips() {
        // Opaque colors stay in the compact hex form.
        assert_eq!(Color::rgb(0x08, 0x99, 0x81).to_css(), "#089981");
        // Any alpha < 1 switches to the functional form with a 0..1 alpha.
        let translucent = Color::rgba(0x26, 0xa6, 0x9a, 0x80);
        assert_eq!(translucent.to_css(), "rgba(38,166,154,0.5019607843137255)");
        // Every possible alpha byte survives the string round trip exactly.
        for a in [0u8, 1, 0x80, 0xFE, 0xFF] {
            let c = Color::rgba(10, 20, 30, a);
            assert_eq!(Color::parse_css(&c.to_css()), Some(c));
        }
    }
}
