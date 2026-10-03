//! Bounded LRU cache of browser-rasterized text runs for the WebGPU text path, plus the pure
//! placement math that maps a text anchor to an atlas quad. Compiled for the host target too,
//! so cache/eviction/positioning logic is unit-testable outside the browser (the DOM
//! rasterizer that feeds it lives in `chart::text_runs`).
//!
//! Keying: a run's glyph pixels depend on its text, the fully-resolved font shorthand, the
//! align mode (it shifts the run's left edge), and the **subpixel phase** of the draw position
//! (glyph AA changes with the fractional part of the offset). Two draws of the same run at the
//! same phase share one rasterization exactly — which is why static labels hit the cache every
//! frame while a scrolling run (new phase each frame) correctly re-rasterizes.

use std::collections::HashMap;

use aeris_charts_render::color::Color;
use aeris_charts_render::draw_list::TextAlign;
use aeris_charts_render_wgpu::AtlasSlot;

/// Cache capacity in runs. Plugin text is label-scale (a handful of distinct runs per frame);
/// 256 bounds atlas churn while never thrashing a realistic frame. Eviction only drops the
/// cache entry (the atlas texels stay until the atlas itself resets — tracked by epoch).
pub const TEXT_RUN_CACHE_CAPACITY: usize = 256;

/// f32 bits of the subpixel fraction of `v` — the cache key's AA-phase discriminator. Bit
/// exactness is deliberate: the host recomputes a run's position with the same f64 math every
/// frame, so a static run keys identically frame over frame.
pub fn frac_bits(v: f32) -> u32 {
    (v - v.floor()).to_bits()
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TextRunKey {
    pub text: String,
    /// Fully-resolved CSS font shorthand (`text_font_spec`): pins size/family/weight.
    pub font: String,
    /// The run color. Chrome's glyph AA is color-dependent (sRGB mask gamma), so the raster
    /// bakes the color in and the cache keys on it — a shared white raster tinted per draw is
    /// measurably wrong for non-white text.
    pub color: Color,
    pub align: TextAlign,
    /// AA phase of the run's left em edge (`left_edge`) and of the anchor y.
    pub frac_x: u32,
    pub frac_y: u32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CachedRun {
    pub slot: AtlasSlot,
    /// Quad offset relative to the anchor: `offset = (x + dx, y + dy)` — integer-valued, so
    /// the quad samples the atlas 1:1 with nearest filtering.
    pub dx: f32,
    pub dy: f32,
}

#[derive(Default)]
pub struct TextRunCache {
    entries: HashMap<TextRunKey, (CachedRun, u64)>,
    tick: u64,
    /// Atlas generation the entries were inserted against; a mismatch clears the cache.
    atlas_epoch: u64,
    /// Total rasterizations performed (one per `insert`) — the test instrumentation counter.
    rasterizations: u64,
}

impl TextRunCache {
    pub fn get(&mut self, key: &TextRunKey, atlas_epoch: u64) -> Option<CachedRun> {
        self.sync_epoch(atlas_epoch);
        self.tick += 1;
        let (run, stamp) = self.entries.get_mut(key)?;
        *stamp = self.tick;
        Some(*run)
    }

    pub fn insert(&mut self, key: TextRunKey, run: CachedRun, atlas_epoch: u64) {
        self.sync_epoch(atlas_epoch);
        self.rasterizations += 1;
        self.tick += 1;
        if self.entries.len() >= TEXT_RUN_CACHE_CAPACITY && !self.entries.contains_key(&key) {
            // Evict the least-recently-used entry (linear scan: rare and capacity-bounded).
            let oldest = self
                .entries
                .iter()
                .min_by_key(|(_, (_, stamp))| *stamp)
                .map(|(key, _)| key.clone());
            if let Some(oldest) = oldest {
                self.entries.remove(&oldest);
            }
        }
        self.entries.insert(key, (run, self.tick));
    }

    /// An atlas reset reuses every texel: cached placements are stale the moment the epoch
    /// moves, so the whole map clears (entries re-rasterize on demand, like the atlas's own).
    fn sync_epoch(&mut self, atlas_epoch: u64) {
        if self.atlas_epoch != atlas_epoch {
            self.entries.clear();
            self.atlas_epoch = atlas_epoch;
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn rasterizations(&self) -> u64 {
        self.rasterizations
    }
}

/// The browser's metrics for one run (the HTML `TextMetrics` subset the placement math needs):
/// advance `width` plus the anchor-relative ink box (`abl`/`abr`/`asc`/`desc`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RunBBox {
    pub width: f32,
    pub abl: f32,
    pub abr: f32,
    pub asc: f32,
    pub desc: f32,
}

/// Where one text run lands in the label atlas: an integer-offset quad plus the in-raster draw
/// position that reproduces the anchor's exact subpixel glyph placement.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RunPlacement {
    /// Integer bitmap-px offset of the texture quad.
    pub base_x: i32,
    pub base_y: i32,
    pub w: u32,
    pub h: u32,
    /// Draw position inside the offscreen raster (`anchor - base`), preserving the phase.
    pub draw_x: f32,
    pub draw_y: f32,
}

/// Map an anchor + browser metrics to the quad placement. `bbox` carries the run's exact
/// font/align/baseline measurements — the HTML spec defines the bounding box relative to the
/// anchor point, so the ink box is anchor-relative for every align. Returns `None` when the
/// run has no ink (whitespace-only text) so both backends skip it identically.
pub fn place_run(x: f32, y: f32, bbox: RunBBox) -> Option<RunPlacement> {
    let RunBBox {
        abl,
        abr,
        asc,
        desc,
        ..
    } = bbox;
    if abl + abr <= 0.0 || asc + desc <= 0.0 {
        return None;
    }
    // Symmetric pad: keeps overhanging ink (italic) and anti-aliased edge texels inside the
    // quad; the pad texels are transparent, so the quad's pixel-aligned edges blend to nothing.
    const PAD: i32 = 2;
    let base_x = (x - abl).floor() as i32 - PAD;
    let base_y = (y - asc).floor() as i32 - PAD;
    let right = (x + abr).ceil() as i32 + PAD;
    let bottom = (y + desc).ceil() as i32 + PAD;
    Some(RunPlacement {
        base_x,
        base_y,
        w: (right - base_x) as u32,
        h: (bottom - base_y) as u32,
        draw_x: x - base_x as f32,
        draw_y: y - base_y as f32,
    })
}

/// The run's left em edge — the value whose fractional part phases the glyph AA. Center/right
/// alignment shifts the edge by the (fractional) measured width, so the phase must key on this,
/// not on the anchor x.
pub fn left_edge(x: f32, align: TextAlign, width: f32) -> f32 {
    match align {
        TextAlign::Left => x,
        TextAlign::Center => x - width / 2.0,
        TextAlign::Right => x - width,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(text: &str, frac_x: u32) -> TextRunKey {
        TextRunKey {
            text: text.into(),
            font: "400 12px Test".into(),
            color: Color::rgb(0, 0, 0),
            align: TextAlign::Left,
            frac_x,
            frac_y: 0,
        }
    }

    fn run(slot_x: u32) -> CachedRun {
        CachedRun {
            slot: AtlasSlot {
                x: slot_x,
                y: 0,
                w: 10,
                h: 10,
            },
            dx: -2.0,
            dy: -8.0,
        }
    }

    #[test]
    fn identical_runs_hit_one_rasterization() {
        let mut cache = TextRunCache::default();
        let key = key("hello", 0);
        assert_eq!(cache.get(&key, 0), None);
        cache.insert(key.clone(), run(0), 0);
        assert_eq!(cache.get(&key, 0), Some(run(0)));
        assert_eq!(cache.get(&key, 0), Some(run(0)));
        assert_eq!(cache.rasterizations(), 1);
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn phase_change_misses() {
        let mut cache = TextRunCache::default();
        cache.insert(key("hello", 0), run(0), 0);
        assert_eq!(cache.get(&key("hello", 0.5f32.to_bits()), 0), None);
        assert_eq!(cache.get(&key("other", 0), 0), None);
    }

    #[test]
    fn atlas_epoch_bump_clears_entries() {
        let mut cache = TextRunCache::default();
        cache.insert(key("hello", 0), run(0), 0);
        assert_eq!(cache.len(), 1);
        // The atlas reset: every cached slot points at reused texels.
        assert_eq!(cache.get(&key("hello", 0), 1), None);
        assert_eq!(cache.len(), 0);
        // New inserts bind to the new epoch.
        cache.insert(key("hello", 0), run(5), 1);
        assert_eq!(cache.get(&key("hello", 0), 1), Some(run(5)));
    }

    #[test]
    fn least_recently_used_entry_is_evicted_at_capacity() {
        let mut cache = TextRunCache::default();
        for i in 0..TEXT_RUN_CACHE_CAPACITY {
            cache.insert(key(&format!("run{i}"), 0), run(i as u32), 0);
        }
        assert_eq!(cache.len(), TEXT_RUN_CACHE_CAPACITY);
        // Touch run1 so run0 is the oldest.
        assert!(cache.get(&key("run1", 0), 0).is_some());
        cache.insert(key("overflow", 0), run(9999), 0);
        assert_eq!(cache.len(), TEXT_RUN_CACHE_CAPACITY);
        assert_eq!(cache.get(&key("run0", 0), 0), None, "oldest entry evicted");
        assert!(
            cache.get(&key("run1", 0), 0).is_some(),
            "touched entry survives"
        );
        assert!(cache.get(&key("overflow", 0), 0).is_some());
    }

    #[test]
    fn frac_bits_keys_the_subpixel_fraction() {
        assert_eq!(frac_bits(0.0), 0);
        assert_eq!(frac_bits(10.0), 0);
        assert_eq!(frac_bits(10.25), frac_bits(0.25));
        assert_eq!(frac_bits(10.25), 0.25f32.to_bits());
        assert_ne!(frac_bits(10.25), frac_bits(10.5));
    }

    #[test]
    fn placement_covers_the_anchor_relative_ink_box_with_pad() {
        // 30 wide, 10 above / 2 below the middle-baseline anchor at (100.4, 50.6).
        let bbox = RunBBox {
            width: 30.0,
            abl: 1.0,
            abr: 29.0,
            asc: 10.0,
            desc: 2.0,
        };
        let p = place_run(100.4, 50.6, bbox).unwrap();
        assert_eq!(p.base_x, 97); // floor(100.4 - 1) - 2
        assert_eq!(p.base_y, 38); // floor(50.6 - 10) - 2
        assert_eq!(p.w, 35); // ceil(100.4 + 29) + 2 - 97 = 132 - 97
        assert_eq!(p.h, 17); // ceil(50.6 + 2) + 2 - 38 = 55 - 38
                             // The in-raster draw position keeps the anchor's fractional phase exactly.
        assert!((p.draw_x - 3.4).abs() < 1e-4);
        assert!((p.draw_y - 12.6).abs() < 1e-4);
    }

    #[test]
    fn placement_skips_inkless_runs() {
        let no_ink = RunBBox {
            width: 8.0,
            abl: 0.0,
            abr: 0.0,
            asc: 10.0,
            desc: 2.0,
        };
        assert_eq!(place_run(0.0, 0.0, no_ink), None);
        let no_height = RunBBox {
            width: 8.0,
            abl: 1.0,
            abr: 7.0,
            asc: 0.0,
            desc: 0.0,
        };
        assert_eq!(place_run(0.0, 0.0, no_height), None);
    }

    #[test]
    fn left_edge_shifts_by_alignment() {
        assert_eq!(left_edge(100.0, TextAlign::Left, 30.0), 100.0);
        assert_eq!(left_edge(100.0, TextAlign::Center, 30.0), 85.0);
        assert_eq!(left_edge(100.0, TextAlign::Right, 30.0), 70.0);
    }
}
