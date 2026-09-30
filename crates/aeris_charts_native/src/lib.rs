//! Native rasterizer target for the [`aeris_charts_render::canvas2d`] executor (roadmap Phase D1/D2).
//!
//! Implements [`Canvas2d`] on top of [`tiny_skia`] — a pure-Rust CPU rasterizer —
//! so the same `Prim` draw-list IR the WebGPU backend renders can also be rasterized to a
//! [`tiny_skia::Pixmap`] and saved as a PNG. Geometry is independent of installed fonts.
//! Text uses the host system UI sans-serif face. Scene goldens that contain no text stay
//! machine-independent; glyph outlines follow whatever sans the OS provides.

pub mod engine_scene;
pub mod scene;

use ab_glyph::{Font, FontArc, FontVec, PxScale, ScaleFont};
use aeris_charts_engine::ChartEngine;
use aeris_charts_render::canvas2d::{execute, Canvas2d, Viewport};
use aeris_charts_render::color::Color;
use aeris_charts_render::draw_list::{Prim, RasterImage, TextAlign};
use std::sync::LazyLock;
use tiny_skia::{
    Color as SkColor, FillRule, FilterQuality, GradientStop, IntSize, LineCap, LineJoin,
    LinearGradient, Paint, PathBuilder, Pixmap, PixmapPaint, Point, PremultipliedColorU8, Rect,
    Shader, SpreadMode, Stroke, StrokeDash, Transform,
};

/// Host system UI sans-serif. Chart layout defaults already name this stack; native
/// rasterization must use the same source instead of embedding a product face.
static FONT: LazyLock<FontArc> = LazyLock::new(system_ui_sans);

fn system_ui_sans() -> FontArc {
    let mut db = fontdb::Database::new();
    db.load_system_fonts();
    let id = db
        .query(&fontdb::Query {
            families: &[fontdb::Family::SansSerif],
            ..fontdb::Query::default()
        })
        .or_else(|| db.faces().next().map(|face| face.id))
        .expect("native rasterizer needs a system UI sans-serif font");
    db.with_face_data(id, |data, index| {
        FontVec::try_from_vec_and_index(data.to_vec(), index)
            .map(FontArc::from)
            .expect("system UI sans-serif font must be a valid TTF or OTF")
    })
    .expect("system UI sans-serif font file must be readable")
}

/// Current fill style. Rebuilt into a `tiny_skia` shader on each paint so we sidestep the
/// `Shader<'a>` lifetime — solid colors and vertical gradients both own their data.
#[derive(Clone)]
enum Fill {
    Solid(SkColor),
    VGradient {
        y_top: f32,
        y_bottom: f32,
        top: SkColor,
        bottom: SkColor,
    },
}

/// One accumulated path command; arcs are tessellated to line segments when the path is built.
#[derive(Clone, Copy)]
enum PathOp {
    MoveTo(f32, f32),
    LineTo(f32, f32),
    Close,
    Arc {
        cx: f32,
        cy: f32,
        r: f32,
        start: f32,
        end: f32,
    },
}

fn sk(c: Color) -> SkColor {
    SkColor::from_rgba8(c.r(), c.g(), c.b(), c.a())
}

/// A [`Canvas2d`] that rasterizes into a `tiny_skia::Pixmap`.
pub struct TinySkiaCanvas {
    pixmap: Pixmap,
    fill: Fill,
    stroke: SkColor,
    line_width: f32,
    dash: Vec<f32>,
    ops: Vec<PathOp>,
}

impl TinySkiaCanvas {
    /// A new canvas of `width`×`height` device px, cleared to `background`.
    pub fn new(width: u32, height: u32, background: Color) -> Self {
        let mut pixmap = Pixmap::new(width.max(1), height.max(1)).expect("valid pixmap size");
        pixmap.fill(sk(background));
        Self {
            pixmap,
            fill: Fill::Solid(SkColor::BLACK),
            stroke: SkColor::BLACK,
            line_width: 1.0,
            dash: Vec::new(),
            ops: Vec::new(),
        }
    }

    /// The rasterized pixels (RGBA8, premultiplied as tiny-skia stores them).
    pub fn pixmap(&self) -> &Pixmap {
        &self.pixmap
    }

    /// Encode the current canvas to a PNG file.
    pub fn save_png(&self, path: &str) -> Result<(), String> {
        self.pixmap.save_png(path).map_err(|e| e.to_string())
    }

    /// Straight (un-premultiplied) RGBA of one pixel — convenient for pixel assertions in tests.
    pub fn pixel_rgba(&self, x: u32, y: u32) -> [u8; 4] {
        let p = self
            .pixmap
            .pixel(x, y)
            .unwrap_or(tiny_skia::PremultipliedColorU8::from_rgba(0, 0, 0, 0).unwrap());
        let c = p.demultiply();
        [c.red(), c.green(), c.blue(), c.alpha()]
    }

    /// Build the current fill paint (solid or vertical gradient).
    fn fill_paint(&self) -> Paint<'static> {
        let shader = match self.fill {
            Fill::Solid(c) => Shader::SolidColor(c),
            Fill::VGradient {
                y_top,
                y_bottom,
                top,
                bottom,
            } => LinearGradient::new(
                Point::from_xy(0.0, y_top),
                // guard against a zero-length gradient (LinearGradient::new returns None)
                Point::from_xy(
                    0.0,
                    if (y_bottom - y_top).abs() < 1e-3 {
                        y_top + 1.0
                    } else {
                        y_bottom
                    },
                ),
                vec![GradientStop::new(0.0, top), GradientStop::new(1.0, bottom)],
                SpreadMode::Pad,
                Transform::identity(),
            )
            .unwrap_or(Shader::SolidColor(top)),
        };
        Paint {
            anti_alias: true,
            shader,
            ..Paint::default()
        }
    }

    /// Materialize the accumulated path ops into a `tiny_skia::Path`.
    fn build_path(&self) -> Option<tiny_skia::Path> {
        let mut pb = PathBuilder::new();
        for op in &self.ops {
            match *op {
                PathOp::MoveTo(x, y) => pb.move_to(x, y),
                PathOp::LineTo(x, y) => pb.line_to(x, y),
                PathOp::Close => pb.close(),
                PathOp::Arc {
                    cx,
                    cy,
                    r,
                    start,
                    end,
                } => {
                    // tessellate the arc; ensure the sub-path is started
                    const SEGS: usize = 24;
                    for i in 0..=SEGS {
                        let t = start + (end - start) * (i as f32 / SEGS as f32);
                        let (x, y) = (cx + r * t.cos(), cy + r * t.sin());
                        if i == 0
                            && self
                                .ops
                                .first()
                                .map(|o| matches!(o, PathOp::Arc { .. }))
                                .unwrap_or(false)
                            && pb.is_empty()
                        {
                            pb.move_to(x, y);
                        } else {
                            pb.line_to(x, y);
                        }
                    }
                }
            }
        }
        pb.finish()
    }

    /// Source-over blend of one coverage sample into the premultiplied pixmap.
    fn blend_coverage(&mut self, x: i32, y: i32, color: SkColor, coverage: f32) {
        let (w, h) = (self.pixmap.width() as i32, self.pixmap.height() as i32);
        if x < 0 || y < 0 || x >= w || y >= h {
            return;
        }
        let idx = (y as u32 * self.pixmap.width() + x as u32) as usize;
        let sa = color.alpha() * coverage;
        let inv = 1.0 - sa;
        let dst = self.pixmap.pixels()[idx];
        let chan = |src: f32, dst: u8| {
            (src * 255.0 * sa + dst as f32 * inv)
                .round()
                .clamp(0.0, 255.0) as u8
        };
        let a = (sa * 255.0 + dst.alpha() as f32 * inv)
            .round()
            .clamp(0.0, 255.0) as u8;
        self.pixmap.pixels_mut()[idx] = PremultipliedColorU8::from_rgba(
            chan(color.red(), dst.red()),
            chan(color.green(), dst.green()),
            chan(color.blue(), dst.blue()),
            a,
        )
        .expect("blended channels are in range");
    }
}

impl Canvas2d for TinySkiaCanvas {
    fn set_fill_solid(&mut self, color: Color) {
        self.fill = Fill::Solid(sk(color));
    }
    fn set_fill_vgradient(&mut self, y_top: f32, y_bottom: f32, top: Color, bottom: Color) {
        self.fill = Fill::VGradient {
            y_top,
            y_bottom,
            top: sk(top),
            bottom: sk(bottom),
        };
    }
    fn set_stroke(&mut self, color: Color) {
        self.stroke = sk(color);
    }
    fn set_line_width(&mut self, width: f32) {
        self.line_width = width.max(0.01);
    }
    fn set_line_dash(&mut self, pattern: &[f32]) {
        self.dash = pattern.to_vec();
    }

    fn fill_rect(&mut self, x: f32, y: f32, w: f32, h: f32) {
        if let Some(rect) = Rect::from_xywh(x, y, w, h) {
            let paint = self.fill_paint();
            self.pixmap
                .fill_rect(rect, &paint, Transform::identity(), None);
        }
    }

    fn begin_path(&mut self) {
        self.ops.clear();
    }
    fn move_to(&mut self, x: f32, y: f32) {
        self.ops.push(PathOp::MoveTo(x, y));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.ops.push(PathOp::LineTo(x, y));
    }
    fn close_path(&mut self) {
        self.ops.push(PathOp::Close);
    }
    fn arc(&mut self, cx: f32, cy: f32, r: f32, start: f32, end: f32) {
        self.ops.push(PathOp::Arc {
            cx,
            cy,
            r,
            start,
            end,
        });
    }
    fn stroke(&mut self) {
        let Some(path) = self.build_path() else {
            return;
        };
        let paint = Paint {
            anti_alias: true,
            shader: Shader::SolidColor(self.stroke),
            ..Paint::default()
        };
        // The shared stroke contract: round joins, butt caps (see `Canvas2d::stroke`).
        let mut stroke = Stroke {
            width: self.line_width,
            line_join: LineJoin::Round,
            line_cap: LineCap::Butt,
            ..Default::default()
        };
        if !self.dash.is_empty() {
            stroke.dash = StrokeDash::new(self.dash.clone(), 0.0);
        }
        self.pixmap
            .stroke_path(&path, &paint, &stroke, Transform::identity(), None);
    }
    fn fill(&mut self) {
        let Some(path) = self.build_path() else {
            return;
        };
        let paint = self.fill_paint();
        self.pixmap.fill_path(
            &path,
            &paint,
            FillRule::Winding,
            Transform::identity(),
            None,
        );
    }

    /// Rasterize a text run with the host system UI sans. The `font` spec carries the size
    /// (`"{weight} {size}px {family}"`); native CPU output uses one system face for every
    /// family/weight/italic, matching Canvas2D metrics wherever the OS face allows:
    /// x is the `align`ed edge, y the vertical center (`textBaseline: "middle"`, approximated
    /// by the ascent/descent midpoint). Coordinates are used as-is (already bitmap space).
    fn fill_text(
        &mut self,
        text: &str,
        x: f32,
        y: f32,
        font: &str,
        color: Color,
        align: TextAlign,
    ) {
        let Some(size) = font
            .split_whitespace()
            .find_map(|tok| tok.strip_suffix("px")?.parse::<f32>().ok())
        else {
            return;
        };
        let scale = PxScale::from(size);
        let scaled = FONT.as_scaled(scale);
        let baseline = y + (scaled.ascent() + scaled.descent()) / 2.0;
        let run_width = |scaled: &ab_glyph::PxScaleFont<&FontArc>| -> f32 {
            let mut w = 0.0;
            let mut prev = None;
            for ch in text.chars() {
                let id = scaled.glyph_id(ch);
                if let Some(p) = prev {
                    w += scaled.kern(p, id);
                }
                w += scaled.h_advance(id);
                prev = Some(id);
            }
            w
        };
        let mut pen_x = match align {
            TextAlign::Left => x,
            TextAlign::Center => x - run_width(&scaled) / 2.0,
            TextAlign::Right => x - run_width(&scaled),
        };
        let ink = sk(color);
        let mut prev = None;
        for ch in text.chars() {
            let id = scaled.glyph_id(ch);
            if let Some(p) = prev {
                pen_x += scaled.kern(p, id);
            }
            let glyph = id.with_scale_and_position(scale, ab_glyph::point(pen_x, baseline));
            pen_x += scaled.h_advance(id);
            prev = Some(id);
            if let Some(outlined) = FONT.outline_glyph(glyph) {
                let bounds = outlined.px_bounds();
                outlined.draw(|gx, gy, cov| {
                    self.blend_coverage(
                        bounds.min.x as i32 + gx as i32,
                        bounds.min.y as i32 + gy as i32,
                        ink,
                        cov,
                    );
                });
            }
        }
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
        let Some(size) = font
            .split_whitespace()
            .find_map(|tok| tok.strip_suffix("px")?.parse::<f32>().ok())
        else {
            return;
        };
        let scale = PxScale::from(size);
        let scaled = FONT.as_scaled(scale);
        let run_width = |scaled: &ab_glyph::PxScaleFont<&FontArc>| -> f32 {
            let mut width = 0.0;
            let mut previous = None;
            for ch in text.chars() {
                let id = scaled.glyph_id(ch);
                if let Some(prev) = previous {
                    width += scaled.kern(prev, id);
                }
                width += scaled.h_advance(id);
                previous = Some(id);
            }
            width
        };
        let width = run_width(&scaled);
        let local_left = match align {
            TextAlign::Left => 0.0,
            TextAlign::Center => -width / 2.0,
            TextAlign::Right => -width,
        };
        let line_height = (scaled.ascent() - scaled.descent()).max(size);
        let origin_x = local_left.floor() - 2.0;
        let origin_y = (-line_height / 2.0).floor() - 2.0;
        let source_width = (width.ceil() + 4.0).max(1.0) as u32;
        let source_height = (line_height.ceil() + 4.0).max(1.0) as u32;
        let mut source = TinySkiaCanvas::new(source_width, source_height, Color::rgba(0, 0, 0, 0));
        let mut pen_x = local_left;
        let baseline = (scaled.ascent() + scaled.descent()) / 2.0;
        let ink = sk(color);
        let mut previous = None;
        for ch in text.chars() {
            let id = scaled.glyph_id(ch);
            if let Some(prev) = previous {
                pen_x += scaled.kern(prev, id);
            }
            let glyph = id.with_scale_and_position(scale, ab_glyph::point(pen_x, baseline));
            pen_x += scaled.h_advance(id);
            previous = Some(id);
            if let Some(outlined) = FONT.outline_glyph(glyph) {
                let bounds = outlined.px_bounds();
                outlined.draw(|gx, gy, cov| {
                    source.blend_coverage(
                        (bounds.min.x + gx as f32 - origin_x).round() as i32,
                        (bounds.min.y + gy as f32 - origin_y).round() as i32,
                        ink,
                        cov,
                    );
                });
            }
        }
        let (sin, cos) = angle.sin_cos();
        self.pixmap.draw_pixmap(
            0,
            0,
            source.pixmap.as_ref(),
            &PixmapPaint {
                quality: FilterQuality::Bicubic,
                ..PixmapPaint::default()
            },
            Transform::from_row(
                cos,
                sin,
                -sin,
                cos,
                x + origin_x * cos - origin_y * sin,
                y + origin_x * sin + origin_y * cos,
            ),
            None,
        );
    }

    fn draw_raster_image(&mut self, image: &RasterImage, rect: [f32; 4], opacity: f32) {
        let Some(size) = IntSize::from_wh(image.width, image.height) else {
            return;
        };
        let mut pixels = image.pixels.to_vec();
        let (rgba_pixels, _) = pixels.as_chunks_mut::<4>();
        for rgba in rgba_pixels {
            let alpha = u16::from(rgba[3]);
            for channel in &mut rgba[..3] {
                *channel = ((u16::from(*channel) * alpha + 127) / 255) as u8;
            }
        }
        let Some(source) = Pixmap::from_vec(pixels, size) else {
            return;
        };
        let [x, y, w, h] = rect;
        let paint = PixmapPaint {
            opacity: opacity.clamp(0.0, 1.0),
            quality: FilterQuality::Bicubic,
            ..PixmapPaint::default()
        };
        self.pixmap.draw_pixmap(
            0,
            0,
            source.as_ref(),
            &paint,
            Transform::from_row(
                w / image.width as f32,
                0.0,
                0.0,
                h / image.height as f32,
                x,
                y,
            ),
            None,
        );
    }
}

/// Convenience: rasterize one layer of prims into a fresh canvas and return it.
pub fn render_prims(
    width: u32,
    height: u32,
    background: Color,
    prims: &[Prim],
    points: &[[f32; 2]],
) -> TinySkiaCanvas {
    let mut canvas = TinySkiaCanvas::new(width, height, background);
    execute(
        prims,
        points,
        &mut canvas,
        Viewport {
            width: width as f32,
            height: height as f32,
        },
    );
    canvas
}

/// Render a real headless chart instance through the same Prim frame consumed by browser hosts.
/// This intentionally covers the chart pane layer; browser-only axis text remains a host concern.
pub fn render_engine(chart: &mut ChartEngine) -> TinySkiaCanvas {
    let frame = chart.build_frame();
    let mut prims = Vec::new();
    let mut points = Vec::new();
    for pane in frame.panes {
        let point_base = points.len() as u32;
        points.extend(pane.points);
        prims.extend(pane.under);
        for prim in pane.main.into_iter().chain(pane.top_prims) {
            prims.push(remap_prim_points(prim, point_base));
        }
    }
    let options = chart.options.get();
    let default_surface = aeris_charts_core::style::DEFAULT_SURFACE_RGB;
    let background = Color::parse_css(&options.layout.background.color).unwrap_or(Color::rgb(
        default_surface.0,
        default_surface.1,
        default_surface.2,
    ));
    render_prims(
        (frame.width * frame.pixel_ratio).round().max(1.0) as u32,
        (frame.height * frame.pixel_ratio).round().max(1.0) as u32,
        background,
        &prims,
        &points,
    )
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ImageExportOptions {
    pub width: u32,
    pub height: u32,
    pub scale: f32,
    pub include_crosshair: bool,
    pub include_trading: bool,
}

impl Default for ImageExportOptions {
    fn default() -> Self {
        Self {
            width: 0,
            height: 0,
            scale: 1.0,
            include_crosshair: true,
            include_trading: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RgbaImage {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// Export the ordered engine frame without mutating the live chart. Dimensions are requested in
/// CSS pixels and multiplied by `scale`; a zero dimension uses the chart's current viewport.
pub fn render_engine_rgba(
    chart: &mut ChartEngine,
    options: ImageExportOptions,
) -> Result<RgbaImage, String> {
    if !options.scale.is_finite() || options.scale <= 0.0 || options.scale > 8.0 {
        return Err("image export scale must be finite and in 0..=8".to_string());
    }
    let frame = chart.build_frame();
    let source_width = (frame.width * frame.pixel_ratio).round().max(1.0) as u32;
    let source_height = (frame.height * frame.pixel_ratio).round().max(1.0) as u32;
    let mut prims = Vec::new();
    let mut points = Vec::new();
    for (index, pane) in frame.panes.iter().enumerate() {
        let point_base = points.len() as u32;
        points.extend_from_slice(&pane.points);
        prims.extend(pane.under.iter().cloned());
        let segments = chart
            .frame_pane_segments(index)
            .ok_or_else(|| "frame pane segments unavailable".to_string())?;
        let base_end = if options.include_trading {
            segments.trading_end
        } else {
            segments.series_end
        };
        for prim in pane.main[..base_end.min(pane.main.len())].iter().cloned() {
            prims.push(remap_prim_points(prim, point_base));
        }
        if options.include_crosshair {
            for prim in pane.main[segments.trading_end.min(pane.main.len())
                ..segments.overlay_end.min(pane.main.len())]
                .iter()
                .cloned()
            {
                prims.push(remap_prim_points(prim, point_base));
            }
        }
        if options.include_crosshair {
            for prim in pane.top_prims.iter().cloned() {
                prims.push(remap_prim_points(prim, point_base));
            }
        }
    }
    let background = aeris_charts_core::style::DEFAULT_SURFACE_RGB;
    let source = render_prims(
        source_width,
        source_height,
        Color::rgb(background.0, background.1, background.2),
        &prims,
        &points,
    );
    let width = if options.width == 0 {
        (frame.width * f64::from(options.scale)).round().max(1.0) as u32
    } else {
        (options.width as f32 * options.scale).round().max(1.0) as u32
    };
    let height = if options.height == 0 {
        (frame.height * f64::from(options.scale)).round().max(1.0) as u32
    } else {
        (options.height as f32 * options.scale).round().max(1.0) as u32
    };
    let mut pixels = vec![0; width as usize * height as usize * 4];
    for y in 0..height {
        let source_y = (y as u64 * source_height as u64 / height as u64) as u32;
        for x in 0..width {
            let source_x = (x as u64 * source_width as u64 / width as u64) as u32;
            let pixel = source.pixel_rgba(source_x, source_y);
            let offset = ((y * width + x) * 4) as usize;
            pixels[offset..offset + 4].copy_from_slice(&pixel);
        }
    }
    Ok(RgbaImage {
        width,
        height,
        pixels,
    })
}

/// Rebase `prim`'s point-pool indices by `base`, where the image export concatenates every pane's
/// pool into one. The match is exhaustive so a pool-indexed prim cannot be forgotten and silently
/// read another pane's points.
fn remap_prim_points(mut prim: Prim, base: u32) -> Prim {
    match &mut prim {
        Prim::Polyline { first_point, .. }
        | Prim::Segments { first_point, .. }
        | Prim::AreaFill { first_point, .. } => *first_point += base,
        Prim::BandFill {
            upper_first,
            lower_first,
            ..
        } => {
            *upper_first += base;
            *lower_first += base;
        }
        Prim::Rect { .. }
        | Prim::RectFrame { .. }
        | Prim::HLine { .. }
        | Prim::VLine { .. }
        | Prim::RoundRect { .. }
        | Prim::Circle { .. }
        | Prim::Triangle { .. }
        | Prim::Background { .. }
        | Prim::Text { .. }
        | Prim::RotatedText { .. }
        | Prim::Image { .. } => {}
    }
    prim
}

/// Result of comparing two rasterized images pixel-by-pixel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DiffStats {
    /// Pixels whose per-channel delta exceeded `tolerance`.
    pub differing_pixels: u32,
    /// Largest single-channel absolute delta seen anywhere.
    pub max_channel_delta: u8,
    /// Total pixels compared.
    pub total_pixels: u32,
}

impl DiffStats {
    /// Fraction of pixels that differed beyond tolerance (0.0–1.0).
    pub fn fraction(&self) -> f64 {
        if self.total_pixels == 0 {
            0.0
        } else {
            self.differing_pixels as f64 / self.total_pixels as f64
        }
    }
}

/// Per-pixel diff of two same-size PNGs (straight RGBA). A pixel counts as differing when any
/// channel differs by more than `tolerance` (allowing small AA/text wobble, per the roadmap).
/// Returns `None` on a size mismatch.
pub fn diff_pixmaps(a: &Pixmap, b: &Pixmap, tolerance: u8) -> Option<DiffStats> {
    if a.width() != b.width() || a.height() != b.height() {
        return None;
    }
    let (pa, pb) = (a.data(), b.data());
    let total = a.width() * a.height();
    let mut differing = 0u32;
    let mut max_delta = 0u8;
    for i in 0..total as usize {
        let mut over = false;
        for c in 0..4 {
            let da = pa[i * 4 + c];
            let db = pb[i * 4 + c];
            let delta = da.abs_diff(db);
            max_delta = max_delta.max(delta);
            if delta > tolerance {
                over = true;
            }
        }
        if over {
            differing += 1;
        }
    }
    Some(DiffStats {
        differing_pixels: differing,
        max_channel_delta: max_delta,
        total_pixels: total,
    })
}

/// Load a PNG file into a `Pixmap`.
pub fn load_png(path: &str) -> Result<Pixmap, String> {
    Pixmap::load_png(path).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use aeris_charts_engine::{
        AxisDimension, CategoryScaleType, ContinuousScaleType, GeneralAxisOptions,
        GeneralScaleType, GeneralSeriesOptions, GeneralXyInput, HorizontalDomain, SeriesKind,
    };
    use aeris_charts_render::draw_list::{Gradient, IRect};
    use std::sync::Arc;

    #[test]
    fn remap_rebases_every_pool_indexed_prim() {
        let c = Color::rgb(1, 2, 3);
        let rebased = |prim| remap_prim_points(prim, 100);
        assert_eq!(
            rebased(Prim::Polyline {
                first_point: 4,
                point_count: 3,
                width: 1.0,
                style: aeris_charts_render::draw_list::LineStyle::Solid,
                line_type: aeris_charts_render::draw_list::LineType::Simple,
                color: c,
            }),
            Prim::Polyline {
                first_point: 104,
                point_count: 3,
                width: 1.0,
                style: aeris_charts_render::draw_list::LineStyle::Solid,
                line_type: aeris_charts_render::draw_list::LineType::Simple,
                color: c,
            }
        );
        assert_eq!(
            rebased(Prim::Segments {
                first_point: 4,
                segment_count: 5,
                width: 1.0,
                color: c,
            }),
            Prim::Segments {
                first_point: 104,
                segment_count: 5,
                width: 1.0,
                color: c,
            }
        );
        let gradient = Gradient { top: c, bottom: c };
        assert_eq!(
            rebased(Prim::AreaFill {
                first_point: 4,
                point_count: 3,
                base_y: 9.0,
                line_type: aeris_charts_render::draw_list::LineType::Simple,
                gradient,
            }),
            Prim::AreaFill {
                first_point: 104,
                point_count: 3,
                base_y: 9.0,
                line_type: aeris_charts_render::draw_list::LineType::Simple,
                gradient,
            }
        );
        // Both boundaries of a band fill move (they were skipped before).
        assert_eq!(
            rebased(Prim::BandFill {
                upper_first: 4,
                lower_first: 10,
                point_count: 3,
                line_type: aeris_charts_render::draw_list::LineType::Simple,
                fill: c,
            }),
            Prim::BandFill {
                upper_first: 104,
                lower_first: 110,
                point_count: 3,
                line_type: aeris_charts_render::draw_list::LineType::Simple,
                fill: c,
            }
        );
        let rect = Prim::Rect {
            rect: IRect {
                x: 1,
                y: 2,
                w: 3,
                h: 4,
            },
            color: c,
        };
        assert_eq!(rebased(rect.clone()), rect, "prims without a pool window");
    }

    #[test]
    fn fills_a_rect_at_expected_pixels() {
        let bg = Color::rgb(0xff, 0xff, 0xff);
        let red = Color::rgb(0xff, 0x00, 0x00);
        let canvas = render_prims(
            20,
            20,
            bg,
            &[Prim::Rect {
                rect: IRect {
                    x: 5,
                    y: 5,
                    w: 10,
                    h: 10,
                },
                color: red,
            }],
            &[],
        );
        // inside the rect -> red
        assert_eq!(canvas.pixel_rgba(10, 10), [0xff, 0x00, 0x00, 0xff]);
        // outside -> background white
        assert_eq!(canvas.pixel_rgba(1, 1), [0xff, 0xff, 0xff, 0xff]);
    }

    #[test]
    fn circle_paints_center_not_far_corner() {
        let bg = Color::rgb(0xff, 0xff, 0xff);
        let blue = Color::rgb(0x00, 0x00, 0xff);
        let canvas = render_prims(
            40,
            40,
            bg,
            &[Prim::Circle {
                cx: 20.0,
                cy: 20.0,
                radius: 8.0,
                fill: blue,
                stroke_width: 0.0,
                stroke: blue,
            }],
            &[],
        );
        assert_eq!(canvas.pixel_rgba(20, 20), [0x00, 0x00, 0xff, 0xff]);
        // a corner far from the disc stays background
        assert_eq!(canvas.pixel_rgba(2, 2), [0xff, 0xff, 0xff, 0xff]);
    }

    #[test]
    fn background_gradient_differs_top_to_bottom() {
        let g = Gradient {
            top: Color::rgb(0x00, 0x00, 0x00),
            bottom: Color::rgb(0xff, 0xff, 0xff),
        };
        let canvas = render_prims(
            10,
            100,
            Color::rgb(0, 0, 0),
            &[Prim::Background {
                rect: [0.0, 0.0, 10.0, 100.0],
                gradient: g,
            }],
            &[],
        );
        let top = canvas.pixel_rgba(5, 2);
        let bottom = canvas.pixel_rgba(5, 97);
        assert!(top[0] < 40, "top should be near-black, got {top:?}");
        assert!(
            bottom[0] > 215,
            "bottom should be near-white, got {bottom:?}"
        );
    }

    #[test]
    fn raster_image_scales_and_blends_through_the_shared_executor() {
        let image = RasterImage {
            key: 1,
            width: 1,
            height: 1,
            pixels: Arc::<[u8]>::from([255, 0, 0, 255]),
        };
        let canvas = render_prims(
            10,
            10,
            Color::rgb(255, 255, 255),
            &[Prim::Image {
                image,
                rect: [2.0, 2.0, 6.0, 6.0],
                opacity: 0.5,
            }],
            &[],
        );
        let center = canvas.pixel_rgba(5, 5);
        assert!(center[0] >= 250 && (120..=135).contains(&center[1]));
        assert_eq!(canvas.pixel_rgba(0, 0), [255, 255, 255, 255]);
    }

    #[test]
    fn text_prim_rasterizes_glyphs_in_the_text_color() {
        let bg = Color::rgb(0xff, 0xff, 0xff);
        let ink = Color::rgb(0x00, 0x66, 0x00);
        let text = |x: f32, align: TextAlign| Prim::Text {
            x,
            y: 30.0,
            text: "Ag 123".into(),
            color: ink,
            size: 24.0,
            family: "sans-serif".into(),
            align,
            weight: 400,
            italic: false,
        };
        let canvas = render_prims(200, 60, bg, &[text(10.0, TextAlign::Left)], &[]);
        let ink_pixels = (0..60)
            .flat_map(|py| (0..200).map(move |px| (px, py)))
            .filter(|&(px, py)| {
                let [r, g, b, a] = canvas.pixel_rgba(px, py);
                a == 0xff && r < 0x40 && (0x30..0x9b).contains(&g) && b < 0x40
            })
            .count();
        assert!(
            ink_pixels >= 50,
            "expected at least 50 ink-colored glyph pixels, got {ink_pixels}"
        );

        // Alignment semantics: a centered run has ink on both sides of its anchor. A
        // left-aligned run may overhang its anchor by a few pixels depending on the host font,
        // but it must not paint far to the left of it.
        let centered = render_prims(200, 60, bg, &[text(100.0, TextAlign::Center)], &[]);
        let has_ink = |c: &TinySkiaCanvas, x0: u32, x1: u32| {
            (x0..x1).any(|px| (0..60).any(|py| c.pixel_rgba(px, py) != [0xff, 0xff, 0xff, 0xff]))
        };
        assert!(has_ink(&centered, 0, 100));
        assert!(has_ink(&centered, 100, 200));
        assert!(!has_ink(&canvas, 0, 6));
    }

    #[test]
    fn rotated_text_raster_follows_the_canonical_angle() {
        let bg = Color::rgb(0xff, 0xff, 0xff);
        let ink = Color::rgb(0x00, 0x66, 0x00);
        let canvas = render_prims(
            120,
            120,
            bg,
            &[Prim::RotatedText {
                x: 60.0,
                y: 60.0,
                text: "TREND".into(),
                color: ink,
                size: 20.0,
                family: "sans-serif".into(),
                align: TextAlign::Center,
                weight: 400,
                italic: false,
                angle: std::f32::consts::FRAC_PI_2,
            }],
            &[],
        );
        let ink_at = |x: u32, y: u32| canvas.pixel_rgba(x, y) != [255, 255, 255, 255];
        assert!(
            (5..115).any(|y| ink_at(60, y)),
            "vertical run must cross its anchor x"
        );
        assert!(
            !(5..45).any(|x| (50..70).any(|y| ink_at(x, y))),
            "a quarter-turn must not remain in the old horizontal location"
        );
    }

    #[test]
    fn renders_a_real_headless_chart_frame() {
        let mut chart = ChartEngine::new(160.0, 100.0, 1.0);
        chart
            .set_series_data(
                0,
                &[1.0, 2.0, 3.0],
                &[10.0, 11.0, 10.5],
                &[12.0, 13.0, 12.0],
                &[9.0, 10.0, 9.5],
                &[11.0, 12.0, 10.0],
            )
            .unwrap();
        chart.time_scale.set_width(160.0);
        chart.fit_content();
        chart.series[0].kind = SeriesKind::Candlestick;
        let canvas = render_engine(&mut chart);
        let non_background = canvas
            .pixmap()
            .data()
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|px| px[0..3] != [0xff, 0xff, 0xff])
            .count();
        assert!(non_background > 0);
    }

    #[test]
    fn renders_category_columns_from_the_shared_engine_frame() {
        let mut chart = ChartEngine::new(240.0, 160.0, 1.0);
        let pane = chart
            .add_pane_with_domain(
                true,
                HorizontalDomain::Category {
                    scale: CategoryScaleType::Band,
                },
            )
            .unwrap();
        chart
            .add_general_axis(GeneralAxisOptions::new(
                "x",
                pane,
                AxisDimension::X,
                GeneralScaleType::Band,
            ))
            .unwrap();
        chart
            .add_general_axis(GeneralAxisOptions::new(
                "y",
                pane,
                AxisDimension::Y,
                GeneralScaleType::Linear,
            ))
            .unwrap();
        let dataset = chart
            .create_general_xy_dataset(GeneralXyInput::Category {
                ids: None,
                categories: vec!["A".into(), "B".into()],
                category_indices: vec![0, 1],
                y: vec![-3.0, 7.0],
                y_valid: None,
            })
            .unwrap();
        let mut options = GeneralSeriesOptions::column(pane, dataset, "x", "y");
        options.color = Some("#6a4c93".into());
        chart.add_general_series(options).unwrap();
        chart.recompute_layout_with_measure(true, |text, _| text.len() as f64 * 7.0, |_, _| 0.0);

        let canvas = render_engine(&mut chart);
        let target = [0x6a, 0x4c, 0x93];
        let painted = canvas
            .pixmap()
            .data()
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|pixel| pixel[0..3] == target)
            .count();
        assert!(
            painted > 20,
            "expected native category-column pixels, got {painted}"
        );
    }

    #[test]
    fn renders_xy_scatter_from_the_shared_engine_frame() {
        let mut chart = ChartEngine::new(260.0, 180.0, 1.0);
        let pane = chart
            .add_pane_with_domain(
                true,
                HorizontalDomain::Continuous {
                    scale: ContinuousScaleType::Linear,
                },
            )
            .unwrap();
        chart
            .add_general_axis(GeneralAxisOptions::new(
                "x",
                pane,
                AxisDimension::X,
                GeneralScaleType::Linear,
            ))
            .unwrap();
        chart
            .add_general_axis(GeneralAxisOptions::new(
                "y",
                pane,
                AxisDimension::Y,
                GeneralScaleType::Linear,
            ))
            .unwrap();
        let dataset = chart
            .create_general_xy_dataset(GeneralXyInput::Numeric {
                ids: None,
                x: vec![1.0, 2.0, 3.0],
                y: vec![-3.0, 2.0, 7.0],
                y_valid: None,
            })
            .unwrap();
        let mut options = GeneralSeriesOptions::scatter(pane, dataset, "x", "y");
        options.color = Some("#2f7d8c".into());
        options.point_radius = 5.0;
        chart.add_general_series(options).unwrap();
        chart.recompute_layout_with_measure(true, |text, _| text.len() as f64 * 7.0, |_, _| 0.0);

        let canvas = render_engine(&mut chart);
        let target = [0x2f, 0x7d, 0x8c];
        let painted = canvas
            .pixmap()
            .data()
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|pixel| pixel[0..3] == target)
            .count();
        assert!(
            painted > 30,
            "expected native scatter pixels, got {painted}"
        );
    }

    #[test]
    fn malformed_surface_css_falls_back_to_the_canonical_aeris_surface() {
        let mut chart = ChartEngine::new(1.0, 1.0, 1.0);
        chart
            .options
            .apply_str(r#"{"layout":{"background":{"color":"not-a-color"}}}"#)
            .unwrap();

        let canvas = render_engine(&mut chart);
        let expected = aeris_charts_core::style::DEFAULT_SURFACE_RGB;
        assert_eq!(
            canvas.pixel_rgba(0, 0),
            [expected.0, expected.1, expected.2, 0xff]
        );
    }
}
