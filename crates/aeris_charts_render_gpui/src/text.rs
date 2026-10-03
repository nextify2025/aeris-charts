//! Text measurement, placement, and cache integration for the GPUI executor.
//!
//! # Current behavior this must reproduce
//!
//! Recorded from `aeris_charts_render::draw_list`, `aeris_charts_render::canvas2d`, `aeris_charts_wasm::text_cache`
//! and `aeris_charts_native`, which are the shipping text paths:
//!
//! | Input | Current behavior |
//! |---|---|
//! | Font shorthand | `aeris_charts_render::draw_list::text_font_spec` → `"[italic ]{weight} {size}px {family}"` |
//! | Family | Fully resolved by the engine/decoder (layout `fontFamily` default already folded in) |
//! | Weight / style | Numeric CSS weight 100–900; italic as a separate flag |
//! | Size | **Device (bitmap) px**, DPR already applied |
//! | Horizontal anchor | `Prim::Text::x` is the *aligned edge*, per `TextAlign` |
//! | Vertical anchor | `Prim::Text::y` is the run's vertical **center** (Canvas `textBaseline: "middle"`) |
//! | Baseline derivation | `aeris_charts_native`: `baseline = y + (ascent + descent) / 2` (ab_glyph signs) |
//! | Label bounds / clipping | The owning layer's clip rect; text is never clipped per-glyph |
//! | Numeric formatting, locale, timezone | Resolved upstream in `aeris_charts_core::format`; the IR carries final strings |
//! | Glyph AA | Color-dependent in Chrome (sRGB mask gamma), so the WebGPU host bakes color into the raster |
//! | Subpixel phase | Glyph AA changes with the fractional part of the draw offset, so the host keys on it |
//!
//! The last two rows are why [`TextKey`] keys on color *and* on the subpixel phase of both
//! coordinates: it is the same discipline `aeris_charts_wasm::text_cache::TextRunKey` already uses, and
//! dropping either would make a scrolling run silently reuse a wrongly-phased raster.
//!
//! # Stage 2 / Stage 3
//!
//! This module is the GPUI-free half: raster keys, measured metrics, and placement math. The
//! feature-gated [`crate::backend`] owns a separate bounded cache of GPUI `ShapedLine`s, so a
//! steady-state hit avoids shaping entirely without leaking GPUI types into the default build.
//! If GPUI-native text fails the identity gate, [`crate::image_cache`] provides the stable-id and
//! eviction policy needed by a Aeris-owned raster path (Stage 3).

use std::collections::HashMap;

use aeris_charts_render::color::Color;
use aeris_charts_render::draw_list::{text_font_spec, TextAlign};

use crate::scene::TextRun;

/// Default capacity, in runs. A chart frame has label-scale text (axis ticks, crosshair labels,
/// chips, watermark) — a few dozen distinct runs — so 512 never thrashes a realistic frame while
/// while still bounding memory.
pub const TEXT_CACHE_CAPACITY: usize = 512;

/// f32 bits of the subpixel fraction of `v`.
///
/// Bit-exact on purpose: the engine recomputes a static label's position with the same math every
/// frame, so it keys identically frame over frame and hits the cache, while a run that actually
/// moves sub-pixel correctly misses and re-shapes.
pub fn frac_bits(v: f32) -> u32 {
    (v - v.floor()).to_bits()
}

/// Everything a shaped-and-rasterized run's pixels depend on.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TextKey {
    pub text: String,
    /// Fully-resolved CSS font shorthand — pins size, family, weight and style in one string.
    pub font: String,
    pub color: Color,
    pub align: TextAlign,
    /// Subpixel phase of the anchor x and y.
    pub frac_x: u32,
    pub frac_y: u32,
}

impl TextKey {
    /// The key for one scene-plan text run.
    pub fn for_run(run: &TextRun) -> Self {
        Self {
            text: run.text.clone(),
            font: text_font_spec(run.size, &run.family, run.weight, run.italic),
            color: run.color,
            align: run.align,
            frac_x: frac_bits(run.x),
            frac_y: frac_bits(run.y),
        }
    }

    /// Rough retained size in bytes, for the cache's memory accounting.
    pub fn heap_bytes(&self) -> usize {
        self.text.len() + self.font.len() + std::mem::size_of::<Self>()
    }
}

/// What a shaper measured for a run. Sign convention matches `ab_glyph` and Aeris's native
/// rasterizer: `ascent` is positive above the baseline, `descent` negative below it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TextMetrics {
    /// Total advance width in device px.
    pub width: f32,
    pub ascent: f32,
    pub descent: f32,
}

/// Where a run actually draws, after alignment and baseline resolution.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextPlacement {
    /// Left edge of the run in device px.
    pub left: f32,
    /// Baseline y in device px.
    pub baseline: f32,
    /// Advance width in device px.
    pub width: f32,
}

/// The left edge of a run of `width` anchored at `anchor_x` under `align`.
///
/// Mirrors Canvas `textAlign`: `Left` draws from the anchor, `Center` centers on it, `Right` ends
/// at it — the same math `aeris_charts_native::fill_text` uses.
pub fn aligned_left(anchor_x: f32, width: f32, align: TextAlign) -> f32 {
    match align {
        TextAlign::Left => anchor_x,
        TextAlign::Center => anchor_x - width / 2.0,
        TextAlign::Right => anchor_x - width,
    }
}

/// The baseline for Canvas `textBaseline: "middle"` about a vertical center.
///
/// `center_y + (ascent + descent) / 2` — identical to `aeris_charts_native`'s formula, so the GPUI and
/// native rasterizers place the same run on the same scanline given the same metrics.
pub fn middle_baseline(center_y: f32, ascent: f32, descent: f32) -> f32 {
    center_y + (ascent + descent) / 2.0
}

/// How far to move a `middle`-anchored run so its cap-height ink centers on the anchor, with
/// [`middle_baseline`]'s sign convention (`descent` negative below the baseline).
pub fn cap_center_offset(ascent: f32, descent: f32, cap_height: f32) -> f32 {
    cap_height / 2.0 - middle_baseline(0.0, ascent, descent)
}

/// Resolve a run plus its measured metrics into a concrete placement.
pub fn place(run: &TextRun, metrics: TextMetrics) -> TextPlacement {
    TextPlacement {
        left: aligned_left(run.x, metrics.width, run.align),
        baseline: middle_baseline(run.y, metrics.ascent, metrics.descent),
        width: metrics.width,
    }
}

/// A bounded LRU of measured runs.
///
/// Only *metrics* are cached here, not GPUI objects or pixels, so this public layer remains
/// backend-neutral. With `gpui-backend` enabled, the backend has its own bounded `ShapedLine` cache
/// that avoids shaping on a hit; pixel caching, if Stage 3 needs it, lives in
/// [`crate::image_cache`].
///
/// Invalidation is explicit and complete: the key covers text, font shorthand (size + family +
/// weight + style), color, alignment and subpixel phase, and [`TextCache::invalidate`] drops
/// everything when the host changes something the key cannot see — a font registration, a theme
/// swap that re-resolves families, or a DPR change.
pub struct TextCache {
    entries: HashMap<TextKey, (TextMetrics, u64)>,
    capacity: usize,
    tick: u64,
    hits: u32,
    misses: u32,
    /// Bumped by [`TextCache::invalidate`]; exposed so a host can assert that a resource change
    /// really did drop the cache.
    generation: u64,
    heap_bytes: usize,
}

impl Default for TextCache {
    fn default() -> Self {
        Self::with_capacity(TEXT_CACHE_CAPACITY)
    }
}

impl TextCache {
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            entries: HashMap::new(),
            capacity: capacity.max(1),
            tick: 0,
            hits: 0,
            misses: 0,
            generation: 0,
            heap_bytes: 0,
        }
    }

    /// Measured metrics for `key`, measuring through `measure` on a miss.
    pub fn measure_with(
        &mut self,
        key: TextKey,
        measure: impl FnOnce() -> TextMetrics,
    ) -> TextMetrics {
        self.tick += 1;
        if let Some((metrics, stamp)) = self.entries.get_mut(&key) {
            *stamp = self.tick;
            self.hits += 1;
            return *metrics;
        }
        self.misses += 1;
        let metrics = measure();
        self.evict_if_full(&key);
        self.heap_bytes += key.heap_bytes();
        self.entries.insert(key, (metrics, self.tick));
        metrics
    }

    fn evict_if_full(&mut self, incoming: &TextKey) {
        if self.entries.len() < self.capacity || self.entries.contains_key(incoming) {
            return;
        }
        // Least-recently-used. A linear scan is fine: it runs only at capacity, and the capacity
        // is small and fixed.
        let oldest = self
            .entries
            .iter()
            .min_by_key(|(_, (_, stamp))| *stamp)
            .map(|(k, _)| k.clone());
        if let Some(oldest) = oldest {
            self.heap_bytes = self.heap_bytes.saturating_sub(oldest.heap_bytes());
            self.entries.remove(&oldest);
        }
    }

    /// Drop every entry. Call when a font set, theme-resolved family, or DPR changes — anything
    /// the key cannot observe.
    pub fn invalidate(&mut self) {
        self.entries.clear();
        self.heap_bytes = 0;
        self.generation += 1;
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Approximate retained heap bytes.
    pub fn heap_bytes(&self) -> usize {
        self.heap_bytes
    }

    /// Hits and misses since the last [`TextCache::reset_counters`].
    pub fn counters(&self) -> (u32, u32) {
        (self.hits, self.misses)
    }

    pub fn reset_counters(&mut self) {
        self.hits = 0;
        self.misses = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(text: &str, x: f32, y: f32, align: TextAlign) -> TextRun {
        TextRun {
            x,
            y,
            text: text.into(),
            color: Color::rgb(0x10, 0x20, 0x30),
            size: 12.0,
            family: "sans-serif".into(),
            align,
            weight: 400,
            italic: false,
            angle: 0.0,
        }
    }

    #[test]
    fn alignment_matches_canvas_text_align() {
        assert_eq!(aligned_left(100.0, 40.0, TextAlign::Left), 100.0);
        assert_eq!(aligned_left(100.0, 40.0, TextAlign::Center), 80.0);
        assert_eq!(aligned_left(100.0, 40.0, TextAlign::Right), 60.0);
    }

    #[test]
    fn middle_baseline_matches_the_native_rasterizer_formula() {
        // ab_glyph convention: ascent positive, descent negative.
        assert_eq!(middle_baseline(30.0, 12.0, -4.0), 34.0);
        assert_eq!(middle_baseline(0.0, 10.0, -10.0), 0.0);
    }

    #[test]
    fn cap_center_offset_centers_cap_ink_on_the_anchor() {
        // GPUI's 12 px test font metrics (ascent 1025, descent -275, cap 698 per 1000 em).
        let (ascent, descent, cap) = (12.3, -3.3, 8.376);
        let offset = cap_center_offset(ascent, descent, cap);
        let baseline = middle_baseline(20.0 + offset, ascent, descent);
        // The cap ink spans `baseline - cap ..= baseline`, so its center lands on the anchor.
        assert!((baseline - cap / 2.0 - 20.0).abs() < 1e-4);
        // A negative descent must not be treated as a positive one (which raised text by it).
        assert!((offset - (-0.312)).abs() < 1e-4);
    }

    #[test]
    fn place_resolves_both_axes() {
        let p = place(
            &run("hello", 100.0, 30.0, TextAlign::Right),
            TextMetrics {
                width: 40.0,
                ascent: 12.0,
                descent: -4.0,
            },
        );
        assert_eq!(
            p,
            TextPlacement {
                left: 60.0,
                baseline: 34.0,
                width: 40.0
            }
        );
    }

    #[test]
    fn key_folds_size_family_weight_and_style_into_the_font_shorthand() {
        let mut r = run("Ag", 0.0, 0.0, TextAlign::Left);
        r.weight = 700;
        r.italic = true;
        r.size = 11.5;
        let key = TextKey::for_run(&r);
        assert_eq!(key.font, "italic 700 11.5px sans-serif");
    }

    #[test]
    fn key_separates_runs_that_differ_only_in_subpixel_phase() {
        let a = TextKey::for_run(&run("123", 10.0, 20.0, TextAlign::Left));
        let b = TextKey::for_run(&run("123", 10.5, 20.0, TextAlign::Left));
        let c = TextKey::for_run(&run("123", 11.0, 20.0, TextAlign::Left));
        assert_ne!(a, b, "a half-pixel shift changes glyph AA");
        assert_eq!(a.frac_x, c.frac_x, "a whole-pixel shift does not");
    }

    #[test]
    fn key_separates_runs_that_differ_only_in_color() {
        let mut a = run("123", 10.0, 20.0, TextAlign::Left);
        let b = a.clone();
        a.color = Color::rgb(0xff, 0, 0);
        assert_ne!(TextKey::for_run(&a), TextKey::for_run(&b));
    }

    #[test]
    fn cache_measures_once_per_distinct_run() {
        let mut cache = TextCache::default();
        let key = TextKey::for_run(&run("42.50", 0.0, 0.0, TextAlign::Left));
        let mut calls = 0;
        for _ in 0..5 {
            cache.measure_with(key.clone(), || {
                calls += 1;
                TextMetrics {
                    width: 30.0,
                    ascent: 10.0,
                    descent: -3.0,
                }
            });
        }
        assert_eq!(calls, 1);
        assert_eq!(cache.counters(), (4, 1));
    }

    #[test]
    fn cache_evicts_least_recently_used_at_capacity() {
        let mut cache = TextCache::with_capacity(2);
        let m = TextMetrics::default();
        let k = |s: &str| TextKey::for_run(&run(s, 0.0, 0.0, TextAlign::Left));
        cache.measure_with(k("a"), || m);
        cache.measure_with(k("b"), || m);
        // touch "a" so "b" becomes the least recently used
        cache.measure_with(k("a"), || m);
        cache.measure_with(k("c"), || m);
        assert_eq!(cache.len(), 2);

        // Check the survivor first: re-measuring the evicted key would itself evict something.
        let mut a_remeasured = false;
        cache.measure_with(k("a"), || {
            a_remeasured = true;
            m
        });
        assert!(!a_remeasured, "a was touched and must have survived");

        let mut b_remeasured = false;
        cache.measure_with(k("b"), || {
            b_remeasured = true;
            m
        });
        assert!(
            b_remeasured,
            "b was least recently used and must have been evicted"
        );
    }

    #[test]
    fn invalidate_clears_entries_and_bumps_the_generation() {
        let mut cache = TextCache::default();
        let key = TextKey::for_run(&run("x", 0.0, 0.0, TextAlign::Left));
        cache.measure_with(key.clone(), TextMetrics::default);
        assert_eq!(cache.len(), 1);
        assert!(cache.heap_bytes() > 0);

        let gen = cache.generation();
        cache.invalidate();
        assert!(cache.is_empty());
        assert_eq!(cache.heap_bytes(), 0);
        assert_eq!(cache.generation(), gen + 1);

        let mut remeasured = false;
        cache.measure_with(key, || {
            remeasured = true;
            TextMetrics::default()
        });
        assert!(remeasured);
    }

    #[test]
    fn heap_accounting_shrinks_on_eviction() {
        let mut cache = TextCache::with_capacity(1);
        let m = TextMetrics::default();
        cache.measure_with(
            TextKey::for_run(&run("a-very-long-label-string", 0.0, 0.0, TextAlign::Left)),
            || m,
        );
        let big = cache.heap_bytes();
        cache.measure_with(TextKey::for_run(&run("b", 0.0, 0.0, TextAlign::Left)), || m);
        assert_eq!(cache.len(), 1);
        assert!(
            cache.heap_bytes() < big,
            "evicting the long run must release its bytes ({} vs {big})",
            cache.heap_bytes()
        );
    }

    #[test]
    fn zero_capacity_is_clamped_to_one_not_a_divide_by_zero() {
        let mut cache = TextCache::with_capacity(0);
        cache.measure_with(
            TextKey::for_run(&run("x", 0.0, 0.0, TextAlign::Left)),
            TextMetrics::default,
        );
        assert_eq!(cache.len(), 1);
    }
}
