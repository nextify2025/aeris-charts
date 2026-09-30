//! Browser [`Canvas2d`] target: drives a real `CanvasRenderingContext2d` from the Prim-IR
//! executor (roadmap Phase D2). This is the in-browser fallback backend for machines without
//! WebGPU — the same draw list the wgpu path renders, issued as 2D canvas calls.

use crate::stroke_state::StrokeState;
use aeris_charts_render::canvas2d::Canvas2d;
use aeris_charts_render::color::Color;
use aeris_charts_render::draw_list::{RasterImage, TextAlign};
use wasm_bindgen::{Clamped, JsCast, JsValue};
use web_sys::{CanvasRenderingContext2d, ImageData, OffscreenCanvas};

const IMAGE_CACHE_CAPACITY: usize = 16;

/// Per-chart decoded Canvas resources. Pixel payloads remain in the shared frame; this cache only
/// avoids rebuilding an `ImageData`/canvas on every Canvas2D fallback frame.
#[derive(Default)]
pub(crate) struct CanvasImageStore {
    entries: Vec<(u64, OffscreenCanvas)>,
}

impl CanvasImageStore {
    fn resolve(&mut self, image: &RasterImage) -> Option<&OffscreenCanvas> {
        if let Some(index) = self.entries.iter().position(|(key, _)| *key == image.key) {
            return Some(&self.entries[index].1);
        }
        let canvas = OffscreenCanvas::new(image.width, image.height).ok()?;
        let ctx = canvas
            .get_context("2d")
            .ok()??
            .unchecked_into::<CanvasRenderingContext2d>();
        let data = ImageData::new_with_u8_clamped_array_and_sh(
            Clamped(image.pixels.as_ref()),
            image.width,
            image.height,
        )
        .ok()?;
        ctx.put_image_data(&data, 0.0, 0.0).ok()?;
        if self.entries.len() == IMAGE_CACHE_CAPACITY {
            self.entries.remove(0);
        }
        self.entries.push((image.key, canvas));
        self.entries.last().map(|(_, canvas)| canvas)
    }
}

/// CSS color string that preserves alpha (unlike `Color::to_hex`, which drops it).
fn css(c: Color) -> String {
    c.to_css()
}

/// Wraps a 2D canvas context as a [`Canvas2d`] target. Errors from fallible context calls
/// (`arc`, `set_line_dash`, gradient construction) are swallowed — a bad draw call drops the
/// primitive rather than aborting the frame.
pub struct WasmCanvas2d<'a> {
    ctx: &'a CanvasRenderingContext2d,
    image_store: Option<&'a mut CanvasImageStore>,
    stroke_state: StrokeState,
    /// Paint ops issued through this target — the ones that put pixels on the canvas
    /// (`fillRect`/`stroke`/`fill`/`fillText`), not state setters or path building. Reported as
    /// part of `frame_stats().canvas2d_ops`.
    ops: u32,
}

impl<'a> WasmCanvas2d<'a> {
    pub fn new(ctx: &'a CanvasRenderingContext2d) -> Self {
        Self {
            ctx,
            image_store: None,
            stroke_state: StrokeState::default(),
            ops: 0,
        }
    }

    pub(crate) fn with_images(
        ctx: &'a CanvasRenderingContext2d,
        image_store: &'a mut CanvasImageStore,
    ) -> Self {
        Self {
            ctx,
            image_store: Some(image_store),
            stroke_state: StrokeState::default(),
            ops: 0,
        }
    }

    /// Paint ops issued since construction.
    pub fn ops(&self) -> u32 {
        self.ops
    }
}

impl Canvas2d for WasmCanvas2d<'_> {
    fn save(&mut self) {
        self.ctx.save();
    }
    fn restore(&mut self) {
        self.ctx.restore();
        // The popped state is whatever was current at the matching `save`, not what we applied.
        self.stroke_state.invalidate();
    }
    fn clip_rect(&mut self, x: f32, y: f32, w: f32, h: f32) {
        self.ctx.begin_path();
        self.ctx.rect(x as f64, y as f64, w as f64, h as f64);
        self.ctx.clip();
    }

    fn set_fill_solid(&mut self, color: Color) {
        self.ctx.set_fill_style_str(&css(color));
    }
    fn set_fill_vgradient(&mut self, y_top: f32, y_bottom: f32, top: Color, bottom: Color) {
        // A zero-length gradient is invalid; nudge the endpoint so it degenerates to a near-solid.
        let y1 = if (y_bottom - y_top).abs() < 1e-3 {
            y_top + 1.0
        } else {
            y_bottom
        };
        let grad = self
            .ctx
            .create_linear_gradient(0.0, y_top as f64, 0.0, y1 as f64);
        let _ = grad.add_color_stop(0.0, &css(top));
        let _ = grad.add_color_stop(1.0, &css(bottom));
        self.ctx.set_fill_style_canvas_gradient(&grad);
    }
    fn set_stroke(&mut self, color: Color) {
        if self.stroke_state.needs_color(color) {
            self.ctx.set_stroke_style_str(&css(color));
        }
    }
    fn set_line_width(&mut self, width: f32) {
        if self.stroke_state.needs_width(width) {
            self.ctx.set_line_width(width as f64);
        }
    }
    fn set_line_dash(&mut self, pattern: &[f32]) {
        if !self.stroke_state.needs_dash(pattern) {
            return;
        }
        let arr = js_sys::Array::new();
        for &seg in pattern {
            arr.push(&JsValue::from_f64(seg as f64));
        }
        let _ = self.ctx.set_line_dash(&arr);
    }

    fn fill_rect(&mut self, x: f32, y: f32, w: f32, h: f32) {
        self.ops += 1;
        self.ctx.fill_rect(x as f64, y as f64, w as f64, h as f64);
    }

    fn begin_path(&mut self) {
        self.ctx.begin_path();
    }
    fn move_to(&mut self, x: f32, y: f32) {
        self.ctx.move_to(x as f64, y as f64);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.ctx.line_to(x as f64, y as f64);
    }
    fn close_path(&mut self) {
        self.ctx.close_path();
    }
    fn arc(&mut self, cx: f32, cy: f32, r: f32, start: f32, end: f32) {
        let _ = self
            .ctx
            .arc(cx as f64, cy as f64, r as f64, start as f64, end as f64);
    }
    fn stroke(&mut self) {
        self.ops += 1;
        if self.stroke_state.needs_join_and_cap() {
            self.ctx.set_line_join("round");
            self.ctx.set_line_cap("butt");
        }
        self.ctx.stroke();
    }
    fn fill(&mut self) {
        self.ops += 1;
        self.ctx.fill();
    }

    fn fill_text(
        &mut self,
        text: &str,
        x: f32,
        y: f32,
        font: &str,
        color: Color,
        align: TextAlign,
    ) {
        self.ctx.set_font(font);
        self.ctx.set_fill_style_str(&css(color));
        self.ctx.set_text_align(align.canvas_keyword());
        // House convention (axis labels): y is the vertical center of the run.
        self.ctx.set_text_baseline("middle");
        self.ops += 1;
        let _ = self.ctx.fill_text(text, x as f64, y as f64);
    }

    fn fill_rotated_text(
        &mut self,
        text: &str,
        x: f32,
        y: f32,
        font: &str,
        color: Color,
        align: TextAlign,
        angle: f32,
    ) {
        self.ctx.save();
        let transformed =
            self.ctx.translate(x as f64, y as f64).is_ok() && self.ctx.rotate(angle as f64).is_ok();
        if transformed {
            self.fill_text(text, 0.0, 0.0, font, color, align);
        }
        self.ctx.restore();
    }

    fn draw_raster_image(&mut self, image: &RasterImage, rect: [f32; 4], opacity: f32) {
        let Some(store) = self.image_store.as_deref_mut() else {
            return;
        };
        let Some(canvas) = store.resolve(image) else {
            return;
        };
        let [x, y, w, h] = rect;
        self.ctx.save();
        self.ctx.set_global_alpha(f64::from(opacity));
        let result = self.ctx.draw_image_with_offscreen_canvas_and_dw_and_dh(
            canvas,
            f64::from(x),
            f64::from(y),
            f64::from(w),
            f64::from(h),
        );
        self.ctx.restore();
        if result.is_ok() {
            self.ops += 1;
        }
    }
}
