//! WebGPU `Prim::Text` path: browser-rasterized glyph runs. Each run is drawn into an
//! offscreen 2D canvas with the exact font string the Canvas2D backend's `fillText` uses
//! ([`text_font_spec`]) **and in the run's own color** — Chrome's glyph anti-aliasing is
//! color-dependent (sRGB mask gamma), so a shared white raster tinted at draw time visibly
//! diverges from direct `fillText` for non-white text. The tex-quad shader folds the
//! straight-alpha texels into premultiplied form with a white tint, reproducing the browser's
//! blend exactly. Runs shelf-pack into the shared label atlas and draw as texture quads at the
//! prim's position in the layer order. Same rasterizer, same font, same color, same AA ⇒ same
//! pixels as the Canvas2D backend (measured 0-diff by the prim-text parity spec).
//!
//! The cache/placement logic is pure and lives in [`crate::text_cache`]; this module is the
//! DOM seam (offscreen canvas + `getImageData` readback) and therefore wasm-only.

use aeris_charts_render::draw_list::{Prim, text_font_spec};
use aeris_charts_render_wgpu::{ATLAS_SIZE, LabelAtlas, TexQuadInstance};
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement, OffscreenCanvas};

use crate::text_cache::{CachedRun, TextRunCache, TextRunKey, frac_bits, left_edge, place_run};

enum RasterCanvas {
    Html(HtmlCanvasElement),
    Offscreen(OffscreenCanvas),
}

impl RasterCanvas {
    fn set_size(&self, width: u32, height: u32) {
        match self {
            Self::Html(canvas) => {
                canvas.set_width(width);
                canvas.set_height(height);
            }
            Self::Offscreen(canvas) => {
                canvas.set_width(width);
                canvas.set_height(height);
            }
        }
    }
}

pub(super) struct TextRunStore {
    canvas: RasterCanvas,
    ctx: CanvasRenderingContext2d,
    cache: TextRunCache,
    /// Monotonic atlas key (the atlas's own string map is unused; the cache is keyed richer).
    next_slot_key: u64,
}

impl TextRunStore {
    pub(super) fn new() -> Result<Self, JsValue> {
        let document = web_sys::window()
            .and_then(|window| window.document())
            .ok_or_else(|| JsValue::from_str("no document"))?;
        let canvas = document
            .create_element("canvas")?
            .dyn_into::<HtmlCanvasElement>()?;
        let ctx = canvas
            .get_context("2d")?
            .ok_or_else(|| JsValue::from_str("no 2d text-raster context"))?
            .dyn_into::<CanvasRenderingContext2d>()?;
        Ok(Self::from_parts(RasterCanvas::Html(canvas), ctx))
    }

    /// Worker-safe text rasterizer. `OffscreenCanvasRenderingContext2D` implements the same 2D
    /// methods used here; the unchecked cast only selects web-sys's shared method bindings and
    /// does not reinterpret memory.
    pub(super) fn new_offscreen() -> Result<Self, JsValue> {
        let canvas = OffscreenCanvas::new(1, 1)?;
        let ctx = canvas
            .get_context("2d")?
            .ok_or_else(|| JsValue::from_str("no offscreen 2d text-raster context"))?
            .unchecked_into::<CanvasRenderingContext2d>();
        Ok(Self::from_parts(RasterCanvas::Offscreen(canvas), ctx))
    }

    fn from_parts(canvas: RasterCanvas, ctx: CanvasRenderingContext2d) -> Self {
        Self {
            canvas,
            ctx,
            cache: TextRunCache::default(),
            next_slot_key: 0,
        }
    }

    /// Resolve one prim to its atlas quad, rasterizing on a cache miss. `None` for non-text
    /// prims, empty/inkless runs, measurement failures, and runs larger than the atlas. The
    /// render owner switches the whole frame to Canvas2D when a non-empty text run fails.
    pub(super) fn resolve(
        &mut self,
        atlas: &mut LabelAtlas,
        queue: &wgpu::Queue,
        prim: &Prim,
    ) -> Option<TexQuadInstance> {
        let (x, y, text, color, size, family, align, weight, italic, angle, rotated) = match prim {
            Prim::Text {
                x,
                y,
                text,
                color,
                size,
                family,
                align,
                weight,
                italic,
            } => (
                x, y, text, color, size, family, align, weight, italic, 0.0, false,
            ),
            Prim::RotatedText {
                x,
                y,
                text,
                color,
                size,
                family,
                align,
                weight,
                italic,
                angle,
            } => (
                x, y, text, color, size, family, align, weight, italic, *angle, true,
            ),
            _ => return None,
        };
        if text.is_empty() {
            return None;
        }
        let (x, y) = (*x, *y);
        let font = text_font_spec(*size, family, *weight, *italic);
        self.ctx.set_font(&font);
        self.ctx.set_text_align(align.canvas_keyword());
        self.ctx.set_text_baseline("middle");
        let metrics = self.ctx.measure_text(text).ok()?;
        let key = TextRunKey {
            text: text.clone(),
            font,
            color: *color,
            align: *align,
            frac_x: frac_bits(left_edge(x, *align, metrics.width() as f32)),
            frac_y: frac_bits(y),
        };
        let bbox = crate::text_cache::RunBBox {
            width: metrics.width() as f32,
            abl: metrics.actual_bounding_box_left() as f32,
            abr: metrics.actual_bounding_box_right() as f32,
            asc: metrics.actual_bounding_box_ascent() as f32,
            desc: metrics.actual_bounding_box_descent() as f32,
        };
        let epoch = atlas.epoch();
        let run = match self.cache.get(&key, epoch) {
            Some(run) => run,
            None => {
                let run = self.rasterize(atlas, queue, &key, x, y, bbox)?;
                // Rasterization may safely reset an atlas before the first insertion of this
                // frame. Cache the slot against the resulting epoch, never the stale pre-reset one.
                self.cache.insert(key, run, atlas.epoch());
                run
            }
        };
        Some(TexQuadInstance {
            rect: [x + run.dx, y + run.dy, run.slot.w as f32, run.slot.h as f32],
            uv: run.slot.uv(),
            // The raster bakes the run color (module docs). The dedicated rotated pipeline
            // interprets this otherwise-white tint slot as its canonical transform.
            color: if rotated {
                [x, y, angle.cos(), angle.sin()]
            } else {
                [1.0, 1.0, 1.0, 1.0]
            },
        })
    }

    /// Rasterize the run on transparent in its own color at its exact subpixel phase and pack
    /// it into the atlas. The quad's integer offset is folded into
    /// [`CachedRun::dx`]/[`CachedRun::dy`].
    fn rasterize(
        &mut self,
        atlas: &mut LabelAtlas,
        queue: &wgpu::Queue,
        key: &TextRunKey,
        x: f32,
        y: f32,
        bbox: crate::text_cache::RunBBox,
    ) -> Option<CachedRun> {
        let placement = place_run(x, y, bbox)?;
        if placement.w > ATLAS_SIZE || placement.h > ATLAS_SIZE {
            return None;
        }
        // Resizing resets all context state — re-establish the exact draw state.
        self.canvas.set_size(placement.w, placement.h);
        self.ctx.set_font(&key.font);
        self.ctx.set_text_align(key.align.canvas_keyword());
        self.ctx.set_text_baseline("middle");
        self.ctx.set_fill_style_str(&key.color.to_css());
        self.ctx
            .fill_text(
                &key.text,
                f64::from(placement.draw_x),
                f64::from(placement.draw_y),
            )
            .ok()?;
        let pixels = self
            .ctx
            .get_image_data(0.0, 0.0, f64::from(placement.w), f64::from(placement.h))
            .ok()?;
        self.next_slot_key += 1;
        let slot = atlas.insert(
            queue,
            self.next_slot_key.to_string(),
            placement.w,
            placement.h,
            &pixels.data(),
        )?;
        Some(CachedRun {
            slot,
            dx: placement.base_x as f32 - x,
            dy: placement.base_y as f32 - y,
        })
    }

    /// Test instrumentation (the prim-text cache spec): live entries and total rasterizations.
    pub(super) fn debug_stats(&self) -> String {
        format!(
            "{{\"entries\":{},\"rasterizations\":{}}}",
            self.cache.len(),
            self.cache.rasterizations()
        )
    }

    pub(super) fn rasterizations(&self) -> u64 {
        self.cache.rasterizations()
    }
}
