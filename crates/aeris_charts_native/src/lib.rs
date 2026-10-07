//! Native rasterizer target for the [`aeris_charts_render::canvas2d`] executor (roadmap Phase D1/D2).
//!
//! Implements [`Canvas2d`] on top of [`tiny_skia`] — a pure-Rust CPU rasterizer —
//! so the same `Prim` draw-list IR the WebGPU backend renders can also be rasterized to a
//! [`tiny_skia::Pixmap`] and saved as a PNG. Text resolves the requested family stack, weight,
//! and style against installed system fonts, and the engine measures labels with the same faces,
//! so axis widths follow the fonts available on the machine. The scene golden masks its text region for the
//! exact bitmap comparison and separately verifies that the requested run painted ink.

pub mod engine_scene;
pub mod scene;

use ab_glyph::{Font, FontArc, FontVec, PxScale, ScaleFont};
use aeris_charts_engine::{ChartEngine, ExportFrameRequest};
use aeris_charts_render::canvas2d::{Canvas2d, Viewport, execute};
use aeris_charts_render::color::Color;
use aeris_charts_render::draw_list::{Prim, RasterImage, TextAlign};
use std::sync::{LazyLock, Mutex};
use tiny_skia::{
    Color as SkColor, FillRule, FilterQuality, GradientStop, IntSize, LineCap, LineJoin,
    LinearGradient, Mask, Paint, PathBuilder, Pixmap, PixmapPaint, Point, PremultipliedColorU8,
    Rect, Shader, SpreadMode, Stroke, StrokeDash, Transform,
};

/// System font lookup shared by native paint and engine measurement. Retention is bounded so a
/// chart that supplies many family names cannot make native glyph storage grow without limit.
struct FontCache {
    database: fontdb::Database,
    entries: Vec<(String, u16, bool, FontArc)>,
}

impl FontCache {
    fn new() -> Self {
        let mut database = fontdb::Database::new();
        database.load_system_fonts();
        Self {
            database,
            entries: Vec::new(),
        }
    }

    /// Adds host-bundled faces so native text matches a host that does not rely on installed
    /// fonts. Cached resolutions are dropped because a new face can win an earlier query.
    fn register(&mut self, data: Vec<u8>) -> Result<usize, String> {
        let before = self.database.len();
        self.database.load_font_data(data);
        let added = self.database.len() - before;
        if added == 0 {
            return Err("font data contains no usable TTF or OTF face".to_string());
        }
        self.entries.clear();
        Ok(added)
    }

    fn get(&mut self, family: &str, weight: u16, italic: bool) -> Option<FontArc> {
        if let Some((_, _, _, font)) = self
            .entries
            .iter()
            .find(|(name, w, i, _)| name == family && *w == weight && *i == italic)
        {
            return Some(font.clone());
        }
        let mut families = Vec::new();
        for name in family.split(',').take(8) {
            let name = name.trim().trim_matches(['\'', '"']);
            let resolved = match name.to_ascii_lowercase().as_str() {
                "serif" => fontdb::Family::Serif,
                "sans-serif" | "system-ui" => fontdb::Family::SansSerif,
                "monospace" => fontdb::Family::Monospace,
                "cursive" => fontdb::Family::Cursive,
                "fantasy" => fontdb::Family::Fantasy,
                _ => fontdb::Family::Name(name),
            };
            families.push(resolved);
        }
        families.push(fontdb::Family::SansSerif);
        families.extend(SANS_SERIF_FALLBACKS.map(fontdb::Family::Name));
        let style = if italic {
            fontdb::Style::Italic
        } else {
            fontdb::Style::Normal
        };
        let id = self
            .database
            .query(&fontdb::Query {
                families: &families,
                weight: fontdb::Weight(weight),
                style,
                ..fontdb::Query::default()
            })
            .or_else(|| stable_fallback_face(&self.database, weight, style))?;
        let font = self
            .database
            .with_face_data(id, |data, index| {
                FontVec::try_from_vec_and_index(data.to_vec(), index)
                    .ok()
                    .map(FontArc::from)
            })
            .flatten()?;
        if self.entries.len() == 32 {
            self.entries.remove(0);
        }
        self.entries
            .push((family.to_string(), weight, italic, font.clone()));
        Some(font)
    }
}

/// Common sans-serif families tried after the generic `sans-serif` alias, which fontdb maps to a
/// single name ("Arial") that many Linux and minimal hosts do not install.
const SANS_SERIF_FALLBACKS: [&str; 9] = [
    "Arial",
    "Helvetica",
    "Liberation Sans",
    "DejaVu Sans",
    "Noto Sans",
    "Segoe UI",
    "Roboto",
    "Ubuntu",
    "Cantarell",
];

/// Last-resort face when no requested or known sans-serif family is installed. The choice is
/// ordered by face properties and names, never by OS enumeration order, so the same installed
/// set renders the same pixels on every run and machine.
fn stable_fallback_face(
    database: &fontdb::Database,
    weight: u16,
    style: fontdb::Style,
) -> Option<fontdb::ID> {
    database
        .faces()
        .min_by(|a, b| {
            let rank = |face: &fontdb::FaceInfo| {
                (
                    face.monospaced,
                    face.style != style,
                    face.weight.0.abs_diff(weight),
                )
            };
            let family = |face: &fontdb::FaceInfo| {
                face.families
                    .first()
                    .map(|(name, _)| name.to_ascii_lowercase())
                    .unwrap_or_default()
            };
            rank(a)
                .cmp(&rank(b))
                .then_with(|| family(a).cmp(&family(b)))
                .then_with(|| a.post_script_name.cmp(&b.post_script_name))
                .then_with(|| a.index.cmp(&b.index))
        })
        .map(|face| face.id)
}

static FONTS: LazyLock<Mutex<FontCache>> = LazyLock::new(|| Mutex::new(FontCache::new()));

fn fonts() -> std::sync::MutexGuard<'static, FontCache> {
    // The cache holds no invariant a panicking reader could break mid-update.
    FONTS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// `None` only when the machine has no installed or registered face at all; text is then
/// skipped by paint, measures as zero, and image export reports it as an error.
fn font_for(family: &str, weight: u16, italic: bool) -> Option<FontArc> {
    fonts().get(family, weight, italic)
}

/// Registers font file bytes (TTF, OTF, or a collection) for native text paint and measurement,
/// so exported images use the host's bundled faces instead of whatever the OS installs. Returns
/// the number of faces added.
pub fn register_font_data(data: Vec<u8>) -> Result<usize, String> {
    fonts().register(data)
}

/// Advance width in bitmap pixels of `text` with the face native paint would use, for hosts
/// that lay out export decorations. `None` when no face is available.
pub fn measure_text(text: &str, size: f32, family: &str, weight: u16, italic: bool) -> Option<f32> {
    let font = font_for(family, weight, italic)?;
    Some(text_advance(&font.as_scaled(PxScale::from(size)), text))
}

fn text_advance(scaled: &ab_glyph::PxScaleFont<&FontArc>, text: &str) -> f32 {
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
}

fn parse_font_spec(spec: &str) -> Option<(f32, &str, u16, bool)> {
    let (before_family, family) = spec.split_once("px ")?;
    let size = before_family
        .split_whitespace()
        .last()?
        .parse::<f32>()
        .ok()?;
    let weight = before_family
        .split_whitespace()
        .find_map(|token| token.parse::<u16>().ok())
        .unwrap_or(400);
    Some((
        size,
        family,
        weight,
        before_family
            .split_whitespace()
            .any(|token| token == "italic"),
    ))
}

/// Returns whether a measure was installed, which invalidates the chart's current layout.
fn install_native_text_measure(chart: &mut ChartEngine) -> bool {
    if chart.has_text_measure() {
        return false;
    }
    chart.set_text_measure(Some(Box::new(|text, size, family, weight, italic| {
        measure_text(text, size as f32, family, weight, italic).map_or(0.0, f64::from)
    })));
    true
}

/// Axis-label advance at `size` in the chart's layout family, as the engine's axis negotiation
/// expects (`bold` selects weight 700).
fn axis_text_width(text: &str, bold: bool, size: f64, family: &str) -> f64 {
    measure_text(
        text,
        size as f32,
        family,
        if bold { 700 } else { 400 },
        false,
    )
    .map_or(0.0, f64::from)
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
    clip: Option<[i32; 4]>,
    clip_stack: Vec<Option<[i32; 4]>>,
    clip_mask: Option<Mask>,
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
            clip: None,
            clip_stack: Vec::new(),
            clip_mask: None,
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

    fn refresh_clip_mask(&mut self) {
        let Some([x0, y0, x1, y1]) = self.clip else {
            return;
        };
        let width = self.pixmap.width();
        let height = self.pixmap.height();
        let mask = self
            .clip_mask
            .get_or_insert_with(|| Mask::new(width, height).expect("valid clip mask size"));
        let data = mask.data_mut();
        data.fill(0);
        let x0 = x0.clamp(0, width as i32) as usize;
        let x1 = x1.clamp(0, width as i32) as usize;
        let y0 = y0.clamp(0, height as i32) as usize;
        let y1 = y1.clamp(0, height as i32) as usize;
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        for y in y0..y1 {
            data[y * width as usize + x0..y * width as usize + x1].fill(255);
        }
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
                    // Preserve at least the existing corner density while large full circles
                    // follow the shared device-pixel chord-error bound.
                    let full = aeris_charts_render::line::circle_segments(r);
                    let sweep = ((end - start).abs() / std::f32::consts::TAU).max(0.0);
                    let segments = ((full as f32 * sweep).ceil() as usize).clamp(24, 256);
                    for i in 0..=segments {
                        let t = start + (end - start) * (i as f32 / segments as f32);
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
        if self
            .clip
            .is_some_and(|[x0, y0, x1, y1]| x < x0 || y < y0 || x >= x1 || y >= y1)
        {
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
    fn save(&mut self) {
        self.clip_stack.push(self.clip);
    }

    fn restore(&mut self) {
        if let Some(clip) = self.clip_stack.pop() {
            self.clip = clip;
            self.refresh_clip_mask();
        }
    }

    fn clip_rect(&mut self, x: f32, y: f32, w: f32, h: f32) {
        let rect = if x.is_finite()
            && y.is_finite()
            && w.is_finite()
            && h.is_finite()
            && w > 0.0
            && h > 0.0
        {
            [
                x.floor() as i32,
                y.floor() as i32,
                (x + w).ceil() as i32,
                (y + h).ceil() as i32,
            ]
        } else {
            [0, 0, 0, 0]
        };
        self.clip = Some(if let Some([x0, y0, x1, y1]) = self.clip {
            [
                x0.max(rect[0]),
                y0.max(rect[1]),
                x1.min(rect[2]),
                y1.min(rect[3]),
            ]
        } else {
            rect
        });
        self.refresh_clip_mask();
    }

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
            let mask = if self.clip.is_some() {
                self.clip_mask.as_ref()
            } else {
                None
            };
            self.pixmap
                .fill_rect(rect, &paint, Transform::identity(), mask);
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
        let mask = if self.clip.is_some() {
            self.clip_mask.as_ref()
        } else {
            None
        };
        self.pixmap
            .stroke_path(&path, &paint, &stroke, Transform::identity(), mask);
    }
    fn fill(&mut self) {
        let Some(path) = self.build_path() else {
            return;
        };
        let paint = self.fill_paint();
        let mask = if self.clip.is_some() {
            self.clip_mask.as_ref()
        } else {
            None
        };
        self.pixmap.fill_path(
            &path,
            &paint,
            FillRule::Winding,
            Transform::identity(),
            mask,
        );
    }

    /// Rasterize a run using the requested installed family, weight, and italic face. The `font`
    /// spec carries size in bitmap pixels; x is the aligned edge and y the vertical center,
    /// approximated by the ascent/descent midpoint. ab_glyph includes fractional placement in
    /// coverage and returns integer pixel bounds, including for negative positions.
    fn fill_text(
        &mut self,
        text: &str,
        x: f32,
        y: f32,
        font: &str,
        color: Color,
        align: TextAlign,
    ) {
        let Some((size, family, weight, italic)) = parse_font_spec(font) else {
            return;
        };
        let scale = PxScale::from(size);
        let Some(face) = font_for(family, weight, italic) else {
            return;
        };
        let scaled = face.as_scaled(scale);
        let baseline = y + (scaled.ascent() + scaled.descent()) / 2.0;
        let mut pen_x = match align {
            TextAlign::Left => x,
            TextAlign::Center => x - text_advance(&scaled, text) / 2.0,
            TextAlign::Right => x - text_advance(&scaled, text),
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
            if let Some(outlined) = face.outline_glyph(glyph) {
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
        let Some((size, family, weight, italic)) = parse_font_spec(font) else {
            return;
        };
        let scale = PxScale::from(size);
        let Some(face) = font_for(family, weight, italic) else {
            return;
        };
        let scaled = face.as_scaled(scale);
        let width = text_advance(&scaled, text);
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
            if let Some(outlined) = face.outline_glyph(glyph) {
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
        let mask = if self.clip.is_some() {
            self.clip_mask.as_ref()
        } else {
            None
        };
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
            mask,
        );
    }

    fn draw_raster_image(&mut self, image: &RasterImage, rect: [f32; 4], opacity: f32) {
        let Some(size) = IntSize::from_wh(image.width, image.height) else {
            return;
        };
        let mut pixels = image.pixels.to_vec();
        aeris_charts_render::draw_list::premultiply_rgba8(&mut pixels);
        let Some(source) = Pixmap::from_vec(pixels, size) else {
            return;
        };
        let [x, y, w, h] = rect;
        let paint = PixmapPaint {
            opacity: opacity.clamp(0.0, 1.0),
            quality: FilterQuality::Bilinear,
            ..PixmapPaint::default()
        };
        let mask = if self.clip.is_some() {
            self.clip_mask.as_ref()
        } else {
            None
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
            mask,
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
    install_native_text_measure(chart);
    let frame = chart.build_frame();
    let options = chart.options.get();
    let default_surface = aeris_charts_core::style::DEFAULT_SURFACE_RGB;
    let background = Color::parse_css(&options.layout.background.color).unwrap_or(Color::rgb(
        default_surface.0,
        default_surface.1,
        default_surface.2,
    ));
    let width = (frame.width * frame.pixel_ratio).round().max(1.0) as u32;
    let height = (frame.height * frame.pixel_ratio).round().max(1.0) as u32;
    let mut canvas = TinySkiaCanvas::new(width, height, background);
    let viewport = Viewport {
        width: width as f32,
        height: height as f32,
    };
    for pane in &frame.panes {
        canvas.save();
        let [x, y, w, h] = pane.scissor;
        canvas.clip_rect(x as f32, y as f32, w as f32, h as f32);
        execute(&pane.under, &pane.points, &mut canvas, viewport);
        execute(&pane.main, &pane.points, &mut canvas, viewport);
        execute(&pane.top_prims, &pane.points, &mut canvas, viewport);
        canvas.restore();
    }
    canvas
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

/// One pane of a prepared export: its device-pixel clip and the selected frame layers in paint
/// order, all indexing the pane's point pool.
#[derive(Clone, Debug)]
struct PreparedPane {
    scissor: [u32; 4],
    prims: Vec<Prim>,
    points: Vec<[f32; 2]>,
}

/// A chart frame captured for image export. Capture needs the live engine and is cheap; the
/// rasterization in [`PreparedChartImage::render`] owns its data, so a host can run it off the
/// thread that owns the chart.
#[derive(Clone, Debug)]
pub struct PreparedChartImage {
    width: u32,
    height: u32,
    pixel_ratio: f32,
    background: Color,
    panes: Vec<PreparedPane>,
    /// The engine's unscissored axis/top layer: scales, tags, separators, and watermark.
    axis: Vec<Prim>,
}

impl PreparedChartImage {
    /// Output size in device pixels.
    #[must_use]
    pub const fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Device pixels per requested CSS pixel, for host decorations placed in CSS units.
    #[must_use]
    pub const fn pixel_ratio(&self) -> f32 {
        self.pixel_ratio
    }

    /// Device-pixel rects `[x, y, width, height]` of each pane's plot area, top to bottom, so a
    /// host can place decorations such as a legend inside the plot rather than over an axis.
    #[must_use]
    pub fn pane_rects(&self) -> Vec<[u32; 4]> {
        self.panes.iter().map(|pane| pane.scissor).collect()
    }

    /// Rasterize the captured panes and axes, then `overlay` in image device pixels on top (an
    /// overlay's point-indexed prims, if any, read an empty pool and draw nothing).
    pub fn render(&self, overlay: &[Prim]) -> Result<RgbaImage, String> {
        let canvas = self.rasterize(overlay)?;
        let mut pixels = vec![0; self.width as usize * self.height as usize * 4];
        for y in 0..self.height {
            for x in 0..self.width {
                let pixel = canvas.pixel_rgba(x, y);
                let offset = ((y * self.width + x) * 4) as usize;
                pixels[offset..offset + 4].copy_from_slice(&pixel);
            }
        }
        Ok(RgbaImage {
            width: self.width,
            height: self.height,
            pixels,
        })
    }

    /// [`Self::render`] encoded as PNG file bytes, for clipboard and file export.
    pub fn render_png(&self, overlay: &[Prim]) -> Result<Vec<u8>, String> {
        self.rasterize(overlay)?
            .pixmap()
            .encode_png()
            .map_err(|error| error.to_string())
    }

    fn rasterize(&self, overlay: &[Prim]) -> Result<TinySkiaCanvas, String> {
        if font_for("sans-serif", 400, false).is_none() {
            return Err("image export needs an installed or registered font".to_string());
        }
        let mut canvas = TinySkiaCanvas::new(self.width, self.height, self.background);
        let viewport = Viewport {
            width: self.width as f32,
            height: self.height as f32,
        };
        for pane in &self.panes {
            canvas.save();
            let [x, y, w, h] = pane.scissor;
            canvas.clip_rect(x as f32, y as f32, w as f32, h as f32);
            execute(&pane.prims, &pane.points, &mut canvas, viewport);
            canvas.restore();
        }
        execute(&self.axis, &[], &mut canvas, viewport);
        execute(overlay, &[], &mut canvas, viewport);
        Ok(canvas)
    }
}

/// Export the ordered engine frame without mutating the live chart. Dimensions are requested in
/// CSS pixels and multiplied by `scale`; a zero dimension uses the chart's current viewport.
pub fn render_engine_rgba(
    chart: &mut ChartEngine,
    options: ImageExportOptions,
) -> Result<RgbaImage, String> {
    prepare_engine_image(chart, options)?.render(&[])
}

/// Capture the ordered engine frame for export without mutating the live chart; see
/// [`render_engine_rgba`] for sizing and [`PreparedChartImage::render`] for rasterization.
pub fn prepare_engine_image(
    chart: &mut ChartEngine,
    options: ImageExportOptions,
) -> Result<PreparedChartImage, String> {
    if !options.scale.is_finite() || options.scale <= 0.0 || options.scale > 8.0 {
        return Err("image export scale must be finite and in 0..=8".to_string());
    }
    install_native_text_measure(chart);
    let css_width = if options.width == 0 {
        chart.css_width
    } else {
        f64::from(options.width)
    };
    let css_height = if options.height == 0 {
        chart.css_height
    } else {
        f64::from(options.height)
    };
    let output_width = (css_width * f64::from(options.scale)).round().max(1.0);
    let output_height = (css_height * f64::from(options.scale)).round().max(1.0);
    if !output_width.is_finite()
        || !output_height.is_finite()
        || output_width * output_height > 32_000_000.0
    {
        return Err("image export exceeds 32 million pixels".to_string());
    }
    let family = chart.options.get().layout.font_family.clone();
    let axis_size = chart.axis_font_size();
    let countdown_size = chart.countdown_font_size();
    let capture = chart.capture_export_frame(
        ExportFrameRequest {
            width: css_width,
            height: css_height,
            dpr: f64::from(options.scale),
            include_crosshair: options.include_crosshair,
        },
        |text, bold| axis_text_width(text, bold, axis_size, &family),
        |text, bold| axis_text_width(text, bold, countdown_size, &family),
    );
    let default_surface = aeris_charts_core::style::DEFAULT_SURFACE_RGB;
    let background = Color::parse_css(&chart.options.get().layout.background.color).unwrap_or(
        Color::rgb(default_surface.0, default_surface.1, default_surface.2),
    );
    let mut panes = Vec::with_capacity(capture.frame.panes.len());
    for (pane, segments) in capture.frame.panes.into_iter().zip(capture.segments) {
        let main_len = pane.main.len();
        let base_end = if options.include_trading {
            segments.trading_end
        } else {
            segments.series_end
        };
        let mut prims = pane.under;
        prims.extend_from_slice(&pane.main[..base_end.min(main_len)]);
        if options.include_crosshair {
            prims.extend_from_slice(
                &pane.main[segments.trading_end.min(main_len)..segments.overlay_end.min(main_len)],
            );
            prims.extend(pane.top_prims);
        }
        panes.push(PreparedPane {
            scissor: pane.scissor,
            prims,
            points: pane.points,
        });
    }
    Ok(PreparedChartImage {
        width: output_width as u32,
        height: output_height as u32,
        pixel_ratio: options.scale,
        background,
        panes,
        axis: capture.axis_primitives,
    })
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
        AxisDimension, CategoryScaleType, ContinuousScaleType, DrawingKind, DrawingPoint,
        GeneralAxisOptions, GeneralScaleType, GeneralSeriesOptions, GeneralXyInput,
        HorizontalDomain, SeriesKind,
    };
    use aeris_charts_render::draw_list::{Gradient, IRect};
    use std::sync::Arc;

    /// The face text assertions run against. A host with no installed or registered font
    /// cannot paint native text at all, so those tests report the skip instead of panicking.
    fn test_face(family: &str) -> Option<FontArc> {
        let face = font_for(family, 400, false);
        if face.is_none() {
            eprintln!("skipping native text assertions: no installed or registered font face");
        }
        face
    }

    /// Paints the chart the way a live native host does: one prepared financial frame with
    /// native axis measurement at `dpr`, then the panes beneath the axis/top layer.
    fn render_live_host(chart: &mut ChartEngine, dpr: f64) -> TinySkiaCanvas {
        install_native_text_measure(chart);
        let family = chart.options.get().layout.font_family.clone();
        let axis_size = chart.axis_font_size();
        let countdown_size = chart.countdown_font_size();
        let (width, height) = (chart.css_width, chart.css_height);
        let mut frame = aeris_charts_engine::ChartFrame::default();
        let mut axis = Vec::new();
        chart.prepare_financial_frame_with_measure(
            aeris_charts_engine::FinancialFrameRequest {
                width,
                height,
                dpr,
                force_layout: false,
                allow_axis_shrink: false,
                force_frame: false,
                force_axis: false,
                layout_only: false,
                fit_content: false,
                frame: &mut frame,
                axis_frame: None,
                axis_primitives: Some(&mut axis),
            },
            |text, bold| axis_text_width(text, bold, axis_size, &family),
            |text, bold| axis_text_width(text, bold, countdown_size, &family),
        );
        let background = Color::parse_css(&chart.options.get().layout.background.color).unwrap();
        // `frame.width` is the plot width; the surface spans the whole chart including axes.
        let width = (width * dpr).round().max(1.0) as u32;
        let height = (height * dpr).round().max(1.0) as u32;
        let mut canvas = TinySkiaCanvas::new(width, height, background);
        let viewport = Viewport {
            width: width as f32,
            height: height as f32,
        };
        for pane in &frame.panes {
            canvas.save();
            let [x, y, w, h] = pane.scissor;
            canvas.clip_rect(x as f32, y as f32, w as f32, h as f32);
            execute(&pane.under, &pane.points, &mut canvas, viewport);
            execute(&pane.main, &pane.points, &mut canvas, viewport);
            execute(&pane.top_prims, &pane.points, &mut canvas, viewport);
            canvas.restore();
        }
        execute(&axis, &[], &mut canvas, viewport);
        canvas
    }

    #[test]
    fn font_fallback_ignores_os_enumeration_order() {
        let mut forward = fontdb::Database::new();
        forward.load_system_fonts();
        let Some(chosen) = stable_fallback_face(&forward, 400, fontdb::Style::Normal) else {
            eprintln!("skipping font fallback order test: no installed font face");
            return;
        };
        let mut paths: Vec<std::path::PathBuf> = forward
            .faces()
            .filter_map(|face| match &face.source {
                fontdb::Source::File(path) | fontdb::Source::SharedFile(path, _) => {
                    Some(path.clone())
                }
                fontdb::Source::Binary(_) => None,
            })
            .collect();
        paths.dedup();
        let mut reversed = fontdb::Database::new();
        for path in paths.iter().rev() {
            reversed
                .load_font_file(path)
                .expect("an installed font file reloads");
        }
        let name = |database: &fontdb::Database, id| {
            database
                .face(id)
                .map(|face| (face.post_script_name.clone(), face.index))
        };
        let reversed_choice = stable_fallback_face(&reversed, 400, fontdb::Style::Normal).unwrap();
        assert_eq!(name(&reversed, reversed_choice), name(&forward, chosen));
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
    fn degenerate_circle_and_polyline_leave_native_pixels_untouched() {
        let background = Color::rgb(255, 255, 255);
        let points = [[2.0, 2.0], [18.0, 18.0]];
        let prims = [
            Prim::Circle {
                cx: 10.0,
                cy: 10.0,
                radius: -4.0,
                fill: Color::rgb(255, 0, 0),
                stroke_width: 2.0,
                stroke: Color::rgb(255, 0, 0),
            },
            Prim::Polyline {
                first_point: 0,
                point_count: 2,
                width: 0.0,
                style: aeris_charts_render::draw_list::LineStyle::Solid,
                line_type: aeris_charts_render::draw_list::LineType::Simple,
                color: Color::rgb(0, 0, 255),
            },
        ];
        let canvas = render_prims(20, 20, background, &prims, &points);
        for y in 0..20 {
            for x in 0..20 {
                assert_eq!(canvas.pixel_rgba(x, y), [255, 255, 255, 255], "at {x},{y}");
            }
        }
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
    fn pane_zero_scatter_cannot_paint_into_the_next_pane() {
        let mut chart = ChartEngine::new(220.0, 240.0, 1.0);
        let pane = chart
            .add_pane_with_domain(
                true,
                HorizontalDomain::Continuous {
                    scale: ContinuousScaleType::Linear,
                },
            )
            .unwrap();
        let following = chart.add_pane(true).unwrap();
        for (id, dimension) in [("x", AxisDimension::X), ("y", AxisDimension::Y)] {
            chart
                .add_general_axis(GeneralAxisOptions::new(
                    id,
                    pane,
                    dimension,
                    GeneralScaleType::Linear,
                ))
                .unwrap();
        }
        let dataset = chart
            .create_general_xy_dataset(GeneralXyInput::Numeric {
                ids: None,
                x: vec![0.0, 1.0],
                y: vec![0.0, 1.0],
                y_valid: None,
            })
            .unwrap();
        let mut options = GeneralSeriesOptions::scatter(pane, dataset, "x", "y");
        options.color = Some("#ff0000".into());
        options.point_radius = 45.0;
        chart.add_general_series(options).unwrap();
        chart.recompute_layout_with_measure(true, |text, _| text.len() as f64 * 7.0, |_, _| 0.0);
        let next = chart.build_frame().panes[following].scissor;
        let canvas = render_engine(&mut chart);
        let mut red = 0;
        for y in next[1]..next[1] + next[3] {
            for x in next[0]..next[0] + next[2] {
                let pixel = canvas.pixel_rgba(x, y);
                red += usize::from(pixel[0] == 255 && pixel[1] == 0 && pixel[2] == 0);
            }
        }
        assert_eq!(
            red, 0,
            "scatter from pane {pane} leaked {red} pixels into pane {following}"
        );
    }

    #[test]
    fn later_pane_band_uses_its_own_point_pool() {
        let mut chart = ChartEngine::new(280.0, 220.0, 1.0);
        chart
            .set_series_data(
                0,
                &[1.0, 2.0, 3.0],
                &[10.0, 11.0, 12.0],
                &[11.0, 12.0, 13.0],
                &[9.0, 10.0, 11.0],
                &[10.5, 11.5, 12.5],
            )
            .unwrap();
        chart.series[0].kind = SeriesKind::Line;
        let pane = chart
            .add_pane_with_domain(
                true,
                HorizontalDomain::Continuous {
                    scale: ContinuousScaleType::Linear,
                },
            )
            .unwrap();
        for (id, dimension) in [("x", AxisDimension::X), ("y", AxisDimension::Y)] {
            chart
                .add_general_axis(GeneralAxisOptions::new(
                    id,
                    pane,
                    dimension,
                    GeneralScaleType::Linear,
                ))
                .unwrap();
        }
        let dataset = chart
            .create_general_xy_dataset(GeneralXyInput::RangeNumeric {
                ids: None,
                x: vec![0.0, 1.0, 2.0],
                low: vec![20.0, 25.0, 22.0],
                low_valid: None,
                high: vec![80.0, 75.0, 78.0],
                high_valid: None,
            })
            .unwrap();
        let mut options = GeneralSeriesOptions::range_area(pane, dataset, "x", "y");
        options.color = Some("#2277bb".into());
        chart.add_general_series(options).unwrap();
        chart.recompute_layout_with_measure(true, |text, _| text.len() as f64 * 7.0, |_, _| 0.0);
        let frame = chart.build_frame();
        assert!(
            !frame.panes[0].points.is_empty(),
            "earlier pane must offset the point pool"
        );
        let owning = &frame.panes[pane];
        let band = owning
            .main
            .iter()
            .find(|prim| matches!(prim, Prim::BandFill { .. }))
            .expect("range area emits a band");
        let background = Color::parse_css(&chart.options.get().layout.background.color).unwrap();
        let reference = render_prims(
            280,
            220,
            background,
            std::slice::from_ref(band),
            &owning.points,
        );
        let actual = render_engine(&mut chart);
        let [sx, sy, sw, sh] = owning.scissor;
        let mut expected_fill = 0;
        for y in sy + 4..sy + sh - 4 {
            for x in sx + 4..sx + sw - 4 {
                let pixel = reference.pixel_rgba(x, y);
                let interior = (-3..=3).all(|dy| {
                    (-3..=3).all(|dx| {
                        reference.pixel_rgba((x as i32 + dx) as u32, (y as i32 + dy) as u32)
                            == pixel
                    })
                });
                if interior
                    && pixel
                        != [
                            background.r(),
                            background.g(),
                            background.b(),
                            background.a(),
                        ]
                {
                    expected_fill += 1;
                    assert_eq!(actual.pixel_rgba(x, y), pixel, "band interior at ({x},{y})");
                }
            }
        }
        assert!(expected_fill > 100, "band fixture must visibly paint");
    }

    #[test]
    fn native_label_hit_width_reaches_the_painted_glyph_advance() {
        let mut chart = ChartEngine::new(320.0, 160.0, 1.0);
        chart
            .set_series_data(
                0,
                &[1.0, 2.0, 3.0],
                &[10.0, 11.0, 12.0],
                &[11.0, 12.0, 13.0],
                &[9.0, 10.0, 11.0],
                &[10.5, 11.5, 12.5],
            )
            .unwrap();
        chart.time_scale.set_width(320.0);
        chart.fit_content();
        let id = chart
            .add_drawing(
                DrawingKind::TrendLine,
                0,
                vec![
                    DrawingPoint { logical: 0.0, price: 11.0 },
                    DrawingPoint { logical: 2.0, price: 11.0 },
                ],
                Some(r#"{"text":"WWWW","text_size":22,"text_h_align":"left","text_v_align":"middle"}"#),
            )
            .unwrap();
        let Some(face) = test_face(&chart.options.get().layout.font_family) else {
            return;
        };
        render_engine(&mut chart);
        let (x, y, angle) = chart.drawing_text_transform(id).unwrap();
        let scaled = face.as_scaled(PxScale::from(22.0));
        let painted_advance = text_advance(&scaled, "WWWW") as f64;
        assert!(
            painted_advance > 60.0,
            "fixture must exceed the fallback width"
        );
        let hit = |along: f64| {
            chart.drawing_text_hit_at(x + angle.cos() * along, y + angle.sin() * along) == Some(id)
        };
        let along = painted_advance - 2.0;
        assert!(
            hit(along),
            "label hit box must reach its painted glyph advance {painted_advance}"
        );
        // The engine pads the label box by 4 px on each side of the measured run.
        const PAD: f64 = 4.0;
        let samples: Vec<f64> = (-160..=(painted_advance as i32 + 40) * 8)
            .map(|step| f64::from(step) / 8.0)
            .filter(|&along| hit(along))
            .collect();
        let (first, last) = (samples[0], *samples.last().unwrap());
        let box_width = last - first - 2.0 * PAD;
        assert!(
            (box_width - painted_advance).abs() <= 1.0,
            "label box spans {box_width} px but the run paints {painted_advance} px"
        );
        assert!(
            (first + PAD).abs() <= 1.0,
            "left-aligned label box must start at the anchor, not {}",
            first + PAD
        );
    }

    #[test]
    fn image_export_renders_custom_background_at_requested_scale() {
        if test_face("sans-serif").is_none() {
            return;
        }
        let new_chart = || {
            let mut chart = ChartEngine::new(120.0, 80.0, 1.0);
            chart
                .options
                .apply_str(r##"{"layout":{"background":{"type":"solid","color":"#2468ac"}}}"##)
                .unwrap();
            chart
                .set_series_data(
                    0,
                    &[1.0, 2.0, 3.0],
                    &[10.0, 11.0, 12.0],
                    &[11.0, 12.0, 13.0],
                    &[9.0, 10.0, 11.0],
                    &[10.5, 11.5, 12.5],
                )
                .unwrap();
            chart.time_scale.set_width(120.0);
            chart.fit_content();
            chart
        };
        let mut chart = new_chart();
        let original = render_live_host(&mut chart, 1.0).pixmap().data().to_vec();
        let image = render_engine_rgba(
            &mut chart,
            ImageExportOptions {
                scale: 2.0,
                ..ImageExportOptions::default()
            },
        )
        .unwrap();
        assert_eq!((image.width, image.height), (240, 160));
        assert_eq!(&image.pixels[..4], &[0x24, 0x68, 0xac, 0xff]);
        let after = render_live_host(&mut chart, 1.0).pixmap().data().to_vec();
        assert_eq!(after, original, "export must restore the live chart view");
        let resized = render_engine_rgba(
            &mut chart,
            ImageExportOptions {
                width: 160,
                height: 100,
                scale: 2.0,
                ..ImageExportOptions::default()
            },
        )
        .unwrap();
        assert_eq!((resized.width, resized.height), (320, 200));
        let restored = render_live_host(&mut chart, 1.0).pixmap().data().to_vec();
        assert_eq!(
            restored
                .iter()
                .zip(&original)
                .filter(|(a, b)| a != b)
                .count(),
            0,
            "resized export must restore the live chart view; first mismatch at {:?}",
            restored.iter().zip(&original).position(|(a, b)| a != b)
        );

        // The export must equal what a live host shows after moving the same chart to a 2x
        // display, axis text included.
        let mut direct = new_chart();
        render_live_host(&mut direct, 1.0);
        let live_2x = render_live_host(&mut direct, 2.0);
        let axis_left = ((direct.pane_left + direct.pane_w) * 2.0).round() as u32;
        assert!(axis_left < 240, "the layout reserves a right price axis");
        let background = [0x24, 0x68, 0xac, 0xff];
        let axis_ink = (axis_left..240)
            .flat_map(|x| (0..160).map(move |y| (x, y)))
            .filter(|&(x, y)| live_2x.pixel_rgba(x, y) != background)
            .count();
        assert!(
            axis_ink > 40,
            "2x price-axis text must paint ink, got {axis_ink}"
        );
        let live_2x = live_2x.pixmap().data();
        assert_eq!(
            image
                .pixels
                .iter()
                .zip(live_2x)
                .filter(|(a, b)| a != b)
                .count(),
            0,
            "2x export must match the live 2x frame; first mismatch at byte {:?}",
            image.pixels.iter().zip(live_2x).position(|(a, b)| a != b)
        );
    }

    #[test]
    fn prepared_export_renders_off_thread_with_host_overlay_on_top() {
        fn assert_send<T: Send + 'static>(_: &T) {}
        let mut chart = ChartEngine::new(120.0, 80.0, 1.0);
        chart
            .set_series_data(
                0,
                &[1.0, 2.0, 3.0],
                &[10.0, 11.0, 12.0],
                &[11.0, 12.0, 13.0],
                &[9.0, 10.0, 11.0],
                &[10.5, 11.5, 12.5],
            )
            .unwrap();
        chart.time_scale.set_width(120.0);
        chart.fit_content();
        let options = ImageExportOptions {
            scale: 2.0,
            ..ImageExportOptions::default()
        };
        let direct = render_engine_rgba(&mut chart, options).unwrap();
        let again = render_engine_rgba(&mut chart, options).unwrap();
        let mismatch = |a: &RgbaImage, b: &RgbaImage| {
            let count = a
                .pixels
                .iter()
                .zip(&b.pixels)
                .filter(|(x, y)| x != y)
                .count();
            let first = a.pixels.iter().zip(&b.pixels).position(|(x, y)| x != y);
            (
                count,
                first.map(|i| (i / 4 % a.width as usize, i / 4 / a.width as usize)),
            )
        };
        assert_eq!(mismatch(&direct, &again), (0, None), "repeat export");
        let prepared = prepare_engine_image(&mut chart, options).unwrap();
        assert_send(&prepared);
        assert_eq!(prepared.size(), (240, 160));
        assert!((prepared.pixel_ratio() - 2.0).abs() < f32::EPSILON);
        let rects = prepared.pane_rects();
        assert!(!rects.is_empty() && rects[0][2] > 0 && rects[0][2] <= 240);
        let plain = std::thread::spawn({
            let prepared = prepared.clone();
            move || prepared.render(&[])
        })
        .join()
        .unwrap()
        .unwrap();
        assert_eq!(
            mismatch(&plain, &direct),
            (0, None),
            "off-thread render matches direct export"
        );

        let marker = Color::rgb(0xfe, 0x01, 0x02);
        let overlaid = prepared
            .render(&[Prim::Rect {
                rect: aeris_charts_render::draw_list::IRect {
                    x: 230,
                    y: 150,
                    w: 10,
                    h: 10,
                },
                color: marker,
            }])
            .unwrap();
        let pixel = |image: &RgbaImage, x: usize, y: usize| {
            image.pixels[(y * image.width as usize + x) * 4..][..4].to_vec()
        };
        assert_eq!(pixel(&overlaid, 235, 155), vec![0xfe, 0x01, 0x02, 0xff]);
        assert_eq!(pixel(&overlaid, 5, 5), pixel(&direct, 5, 5));
    }

    #[test]
    fn image_export_paints_the_price_axis_beside_the_plot() {
        let mut chart = ChartEngine::new(240.0, 160.0, 1.0);
        chart
            .set_series_data(
                0,
                &[1.0, 2.0, 3.0],
                &[10.0, 11.0, 12.0],
                &[11.0, 12.0, 13.0],
                &[9.0, 10.0, 11.0],
                &[10.5, 11.5, 12.5],
            )
            .unwrap();
        chart.fit_content();
        let options = ImageExportOptions {
            width: 320,
            height: 200,
            include_crosshair: false,
            ..ImageExportOptions::default()
        };
        let prepared = prepare_engine_image(&mut chart, options).unwrap();
        let [plot_x, _, plot_w, plot_h] = prepared.pane_rects()[0];
        assert!(
            plot_x + plot_w < 320,
            "the layout reserves a right price axis"
        );
        let image = prepared.render(&[]).unwrap();
        let background = image.pixels[..4].to_vec();
        let axis_ink = (plot_x + plot_w..320)
            .flat_map(|x| (0..plot_h).map(move |y| (x, y)))
            .filter(|&(x, y)| {
                let offset = ((y * image.width + x) * 4) as usize;
                image.pixels[offset..offset + 4] != background[..]
            })
            .count();
        assert!(axis_ink > 0, "price-axis labels paint in the export");
        let png = prepared.render_png(&[]).unwrap();
        let decoded = Pixmap::decode_png(&png).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (320, 200));
        assert_eq!(
            (chart.css_width, chart.css_height),
            (240.0, 160.0),
            "export restores the live viewport"
        );
    }

    #[test]
    fn registered_font_data_rejects_non_font_bytes() {
        assert!(register_font_data(b"not a font".to_vec()).is_err());
        assert!(measure_text("Aeris", 12.0, "sans-serif", 400, false).is_some_and(|w| w > 0.0));
    }

    #[test]
    fn image_export_keeps_the_engine_gradient_background() {
        let mut chart = ChartEngine::new(40.0, 60.0, 1.0);
        chart
            .options
            .apply_str(
                r##"{"layout":{"background":{"type":"gradient","topColor":"#112233","bottomColor":"#99aabb"}}}"##,
            )
            .unwrap();
        let image = render_engine_rgba(&mut chart, ImageExportOptions::default()).unwrap();
        let pixel = |x: usize, y: usize| &image.pixels[(y * image.width as usize + x) * 4..][..4];
        assert!(
            pixel(5, 0)
                .iter()
                .zip([0x11_u8, 0x22, 0x33, 0xff])
                .all(|(actual, expected)| actual.abs_diff(expected) <= 1)
        );
        assert!(pixel(5, 20)[0] > pixel(5, 0)[0]);
    }

    #[test]
    fn native_glyphs_keep_negative_and_fractional_x_coverage_columns() {
        if test_face("sans-serif").is_none() {
            return;
        }
        // Per-column ink of "W" painted at `x` on an 80 px canvas.
        let column_ink = |x: f32| -> Vec<u32> {
            let mut canvas = TinySkiaCanvas::new(80, 48, Color::rgba(0, 0, 0, 0));
            canvas.fill_text(
                "W",
                x,
                24.0,
                "400 20px sans-serif",
                Color::rgb(255, 255, 255),
                TextAlign::Left,
            );
            (0..80)
                .map(|column| {
                    (0..48)
                        .map(|row| u32::from(canvas.pixel_rgba(column, row)[3]))
                        .sum()
                })
                .collect()
        };
        // A whole-pixel shift keeps the subpixel phase, so the run at 30.5 is an independent
        // reference for the run at -0.5: every column moves left by exactly 31 and the columns
        // that fall off the left edge are the only ones lost.
        let reference = column_ink(30.5);
        let negative = column_ink(-0.5);
        assert!(reference[..30].iter().all(|&ink| ink == 0));
        let expected: Vec<u32> = (0..80)
            .map(|column| reference.get(column + 31).copied().unwrap_or(0))
            .collect();
        assert!(expected[0] > 0, "the fixture glyph must straddle x = 0");
        assert_eq!(negative, expected, "negative-x glyph coverage columns");

        // Fractional placement moves the coverage centroid by the fraction, not a whole pixel.
        let centroid = |ink: &[u32]| {
            let total: f64 = ink.iter().map(|&v| f64::from(v)).sum();
            ink.iter()
                .enumerate()
                .map(|(column, &v)| (column as f64 + 0.5) * f64::from(v))
                .sum::<f64>()
                / total
        };
        let shift = centroid(&column_ink(10.75)) - centroid(&column_ink(10.25));
        assert!(
            (shift - 0.5).abs() <= 0.1,
            "a 0.5 px fractional move shifted ink by {shift} px"
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
