//! Built-in solid icons for the icon stamp and arrow marker tools.
//!
//! The icon outlines are a curated subset of the Phosphor Icons 2.1.1 fill weight
//! (`@phosphor-icons/core`):
//!
//! MIT License. Copyright (c) 2023 Phosphor Icons.
//!
//! Permission is hereby granted, free of charge, to any person obtaining a copy of this software
//! and associated documentation files (the "Software"), to deal in the Software without
//! restriction, including without limitation the rights to use, copy, modify, merge, publish,
//! distribute, sublicense, and/or sell copies of the Software, and to permit persons to whom the
//! Software is furnished to do so, subject to the following conditions:
//!
//! The above copyright notice and this permission notice shall be included in all copies or
//! substantial portions of the Software.
//!
//! THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING
//! BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND
//! NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM,
//! DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
//! OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.
//!
//! Every icon is one nonzero-filled SVG path on a 256-unit grid. The engine flattens it at the
//! shared curve tolerance and rasterizes exact-area coverage at the icon's device-pixel size in
//! the drawing color, so every backend paints the identical image (like the crosshair icon).

use aeris_charts_render::color::Color;
use aeris_charts_render::draw_list::RasterImage;
use aeris_charts_render::shape::Rect;

use super::DrawingKind;
use super::geometry::{CurveGeometry, arc_segments};

/// The icons' design grid, in icon units.
pub(crate) const ICON_VIEWBOX: f64 = 256.0;
/// Largest rasterized icon edge in device px (the 256 CSS px display cap at DPR 4).
pub(crate) const MAX_ICON_RASTER_PX: u32 = 1024;

pub(crate) struct BuiltinIcon {
    pub(crate) name: &'static str,
    d: &'static str,
}

pub(crate) fn builtin_icon(name: &str) -> Option<&'static BuiltinIcon> {
    BUILTIN_ICONS.iter().find(|icon| icon.name == name)
}

pub(crate) fn builtin_icon_index(icon: &BuiltinIcon) -> usize {
    BUILTIN_ICONS
        .iter()
        .position(|candidate| std::ptr::eq(candidate, icon))
        .unwrap_or(0)
}

/// The icon an arrow marker paints and its tip in icon units: the marker's anchor is the tip,
/// so the arrow points at the bar or price it marks.
pub(crate) fn arrow_marker_icon(kind: DrawingKind) -> Option<(&'static str, (f64, f64))> {
    match kind {
        DrawingKind::ArrowMarkerUp => Some(("arrow-fat-up", (128.0, 16.0))),
        DrawingKind::ArrowMarkerDown => Some(("arrow-fat-down", (128.0, 240.0))),
        DrawingKind::ArrowMarkerLeft => Some(("arrow-fat-left", (16.0, 128.0))),
        DrawingKind::ArrowMarkerRight => Some(("arrow-fat-right", (240.0, 128.0))),
        _ => None,
    }
}

impl BuiltinIcon {
    /// Standalone inline SVG (`currentColor` fill) for host icon pickers, from the same path
    /// the engine paints.
    pub(crate) fn svg(&self) -> String {
        format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 256 256\" \
             fill=\"currentColor\"><path d=\"{}\"/></svg>",
            self.d
        )
    }

    /// The outline as closed polygons, mapping icon units through `scale` px per unit.
    pub(crate) fn contours(&self, scale: f64) -> Vec<Vec<(f64, f64)>> {
        flatten_path(self.d, scale)
    }

    /// Straight-alpha RGBA8 pixels of the icon filled in `color` on a `size`-px square
    /// (clamped to `1..=MAX_ICON_RASTER_PX`), with exact-area anti-aliased coverage.
    pub(crate) fn rasterize(&self, size: u32, color: Color) -> Vec<u8> {
        let size = size.clamp(1, MAX_ICON_RASTER_PX);
        let coverage = fill_coverage(&self.contours(f64::from(size) / ICON_VIEWBOX), size);
        // Coverage streams straight into the pixels: no second per-pixel buffer.
        let mut pixels = Vec::with_capacity(size as usize * size as usize * 4);
        for value in coverage {
            let alpha = (value * f64::from(color.a())).round() as u8;
            pixels.extend_from_slice(&[color.r(), color.g(), color.b(), alpha]);
        }
        pixels
    }
}

/// A cached raster's identity: built-in icon index, device size, and RGBA color.
type IconRasterKey = (usize, u32, [u8; 4]);

/// Rasters kept per chart; enough for every built-in icon at a couple of sizes and colors.
const MAX_ICON_RASTERS: usize = 64;
/// Pixel bytes kept per chart: 64 rasters of 256 device px, or four at the 1024 px maximum
/// (WASM linear memory never shrinks, so the bound is on bytes, not only entries).
pub(crate) const MAX_ICON_RASTER_BYTES: usize = 16 << 20;
// Every raster fits the budget, so the eviction loop always makes room.
const _: () =
    assert!(MAX_ICON_RASTER_PX as usize * MAX_ICON_RASTER_PX as usize * 4 <= MAX_ICON_RASTER_BYTES);

/// Bounded least-recently-used cache of rasterized built-in icons, keyed by icon, device size,
/// and color, and bounded by both entries ([`MAX_ICON_RASTERS`]) and pixel bytes
/// ([`MAX_ICON_RASTER_BYTES`]). Each new raster gets a fresh image key from a range disjoint
/// from registered images (`1 << 62` up) and the crosshair asset (bit 63), so executor image
/// caches never confuse two rasters.
pub(crate) struct IconRasterCache {
    entries: std::collections::VecDeque<(IconRasterKey, RasterImage)>,
    bytes: usize,
    next_key: u64,
}

impl Default for IconRasterCache {
    fn default() -> Self {
        Self {
            entries: std::collections::VecDeque::new(),
            bytes: 0,
            next_key: 1 << 61,
        }
    }
}

impl IconRasterCache {
    /// The raster of `icon` at `size` device px in `color`: the cached one, or a new one that
    /// is cached when `cache` holds (evicting least-recently-used rasters until it fits the byte
    /// budget). One built with `cache` off (each sample of an icon resize drag paints its exact
    /// size once) is returned uncached.
    pub(crate) fn raster(
        &mut self,
        icon: &BuiltinIcon,
        size: u32,
        color: Color,
        cache: bool,
    ) -> RasterImage {
        let size = size.clamp(1, MAX_ICON_RASTER_PX);
        let id = (
            builtin_icon_index(icon),
            size,
            [color.r(), color.g(), color.b(), color.a()],
        );
        if let Some(position) = self.entries.iter().position(|(key, _)| *key == id) {
            let entry = self.entries.remove(position).expect("cached icon raster");
            let image = entry.1.clone();
            self.entries.push_back(entry);
            return image;
        }
        let image = RasterImage {
            key: self.next_key,
            width: size,
            height: size,
            pixels: icon.rasterize(size, color).into(),
        };
        self.next_key += 1;
        let bytes = image.pixels.len();
        if !cache {
            return image;
        }
        while self.entries.len() >= MAX_ICON_RASTERS || self.bytes + bytes > MAX_ICON_RASTER_BYTES {
            let Some((_, evicted)) = self.entries.pop_front() else {
                break;
            };
            self.bytes -= evicted.pixels.len();
        }
        self.bytes += bytes;
        self.entries.push_back((id, image.clone()));
        image
    }

    /// Cached rasters and their pixel bytes.
    #[cfg(test)]
    pub(crate) fn usage(&self) -> (usize, usize) {
        (self.entries.len(), self.bytes)
    }
}

/// Nonzero coverage of closed `contours` on a `size` x `size` grid, row-major: signed-area
/// accumulation per pixel, then a running sum whose magnitude (clamped to one) is the covered
/// fraction, streamed so callers write it straight into their pixels.
fn fill_coverage(contours: &[Vec<(f64, f64)>], size: u32) -> impl Iterator<Item = f64> + use<> {
    let width = size as usize;
    let limit = f64::from(size);
    // One spare cell per row end lets a span's right remainder land past the last column.
    let mut accumulation = vec![0.0_f64; width * width + 2];
    for contour in contours {
        for pair in contour.windows(2) {
            let clamp = |(x, y): (f64, f64)| (x.clamp(0.0, limit), y.clamp(0.0, limit));
            accumulate_edge(&mut accumulation, width, clamp(pair[0]), clamp(pair[1]));
        }
    }
    accumulation.truncate(width * width);
    let mut sum = 0.0;
    accumulation.into_iter().map(move |delta| {
        sum += delta;
        sum.abs().min(1.0)
    })
}

/// Adds one edge's signed area to the accumulation buffer (the font-rs exact-area scheme).
fn accumulate_edge(acc: &mut [f64], width: usize, p0: (f64, f64), p1: (f64, f64)) {
    if (p0.1 - p1.1).abs() <= f64::EPSILON {
        return;
    }
    let (direction, top, bottom) = if p0.1 < p1.1 {
        (1.0, p0, p1)
    } else {
        (-1.0, p1, p0)
    };
    let dxdy = (bottom.0 - top.0) / (bottom.1 - top.1);
    let mut x = top.0;
    let last_row = (bottom.1.ceil() as usize).min(width);
    for row in (top.1.floor() as usize)..last_row {
        let row_start = row * width;
        let y = row as f64;
        let dy = (y + 1.0).min(bottom.1) - y.max(top.1);
        if dy <= 0.0 {
            continue;
        }
        let x_next = x + dxdy * dy;
        let d = dy * direction;
        let (left, right) = if x < x_next { (x, x_next) } else { (x_next, x) };
        let left_floor = left.floor();
        let left_cell = left_floor as usize;
        let right_ceil = right.ceil();
        let right_cell = right_ceil as usize;
        if right_cell <= left_cell + 1 {
            let mid = 0.5 * (x + x_next) - left_floor;
            acc[row_start + left_cell] += d - d * mid;
            acc[row_start + left_cell + 1] += d * mid;
        } else {
            let inverse = 1.0 / (right - left);
            let left_frac = left - left_floor;
            let first = 0.5 * inverse * (1.0 - left_frac).powi(2);
            let right_frac = right - right_ceil + 1.0;
            let last = 0.5 * inverse * right_frac.powi(2);
            acc[row_start + left_cell] += d * first;
            if right_cell == left_cell + 2 {
                acc[row_start + left_cell + 1] += d * (1.0 - first - last);
            } else {
                let second = inverse * (1.5 - left_frac);
                acc[row_start + left_cell + 1] += d * (second - first);
                for cell in (left_cell + 2)..(right_cell - 1) {
                    acc[row_start + cell] += d * inverse;
                }
                let before_last = second + (right_cell - left_cell - 3) as f64 * inverse;
                acc[row_start + right_cell - 1] += d * (1.0 - before_last - last);
            }
            acc[row_start + right_cell] += d * last;
        }
        x = x_next;
    }
}

/// Flattens SVG path data (`M L H V C S Q T A Z`, absolute and relative, implicit repeats)
/// into closed polygons in px (`scale` px per unit), tessellating curves and arcs at the
/// shared tolerance.
fn flatten_path(d: &str, scale: f64) -> Vec<Vec<(f64, f64)>> {
    let mut numbers = PathNumbers { rest: d };
    let mut contours = Vec::new();
    let mut points: Vec<(f64, f64)> = Vec::new();
    let mut command = b'M';
    let (mut current, mut start) = ((0.0, 0.0), (0.0, 0.0));
    // The previous curve's second control point, for S/T reflection.
    let mut last_control: Option<(u8, (f64, f64))> = None;
    let px = |(x, y): (f64, f64)| (x * scale, y * scale);
    let close = |points: &mut Vec<(f64, f64)>, contours: &mut Vec<Vec<(f64, f64)>>| {
        if points.len() >= 3 {
            let first = points[0];
            points.push(first);
            contours.push(std::mem::take(points));
        }
        points.clear();
    };
    loop {
        if let Some(next) = numbers.command() {
            command = next;
            if command.eq_ignore_ascii_case(&b'Z') {
                close(&mut points, &mut contours);
                current = start;
                last_control = None;
                continue;
            }
        } else if numbers.at_end() {
            break;
        }
        let relative = command.is_ascii_lowercase();
        let base = if relative { current } else { (0.0, 0.0) };
        let offset = |(x, y): (f64, f64)| (base.0 + x, base.1 + y);
        let mut control = None;
        let end = match command.to_ascii_uppercase() {
            b'M' | b'L' => numbers.pair().map(offset),
            b'H' => numbers
                .next()
                .map(|x| (if relative { current.0 + x } else { x }, current.1)),
            b'V' => numbers
                .next()
                .map(|y| (current.0, if relative { current.1 + y } else { y })),
            b'C' | b'S' => {
                let c1 = if command.eq_ignore_ascii_case(&b'C') {
                    numbers.pair().map(offset)
                } else {
                    Some(reflect(last_control, b'C', current))
                };
                let (Some(c1), Some(c2), Some(end)) =
                    (c1, numbers.pair().map(offset), numbers.pair().map(offset))
                else {
                    break;
                };
                push_curve([current, c1, c2, end], true, px, &mut points);
                control = Some((b'C', c2));
                Some(end)
            }
            b'Q' | b'T' => {
                let c = if command.eq_ignore_ascii_case(&b'Q') {
                    numbers.pair().map(offset)
                } else {
                    Some(reflect(last_control, b'Q', current))
                };
                let (Some(c), Some(end)) = (c, numbers.pair().map(offset)) else {
                    break;
                };
                push_curve([current, c, end, end], false, px, &mut points);
                control = Some((b'Q', c));
                Some(end)
            }
            b'A' => {
                let (Some(rx), Some(ry), Some(rotation), Some(large), Some(sweep), Some(end)) = (
                    numbers.next(),
                    numbers.next(),
                    numbers.next(),
                    numbers.flag(),
                    numbers.flag(),
                    numbers.pair().map(offset),
                ) else {
                    break;
                };
                push_arc(
                    current,
                    end,
                    (rx, ry),
                    rotation,
                    large,
                    sweep,
                    scale,
                    &mut points,
                );
                Some(end)
            }
            _ => None,
        };
        let Some(end) = end else {
            break;
        };
        if command.eq_ignore_ascii_case(&b'M') {
            close(&mut points, &mut contours);
            start = end;
            // Coordinates after a moveto are implicit linetos.
            command = if relative { b'l' } else { b'L' };
        }
        points.push(px(end));
        current = end;
        last_control = control;
    }
    close(&mut points, &mut contours);
    contours
}

/// The reflection of the previous same-family control point about `current`, or `current`.
fn reflect(last: Option<(u8, (f64, f64))>, family: u8, current: (f64, f64)) -> (f64, f64) {
    match last {
        Some((kind, (x, y))) if kind == family => (2.0 * current.0 - x, 2.0 * current.1 - y),
        _ => current,
    }
}

/// Interior samples of a Bezier from `points[0]`, in px within the shared curve tolerance; the
/// caller pushes its end point.
fn push_curve(
    points: [(f64, f64); 4],
    cubic: bool,
    px: impl Fn((f64, f64)) -> (f64, f64),
    out: &mut Vec<(f64, f64)>,
) {
    let points = points.map(px);
    // The curve lies in its control points' hull, so their box never clips it.
    let Some(clip) = Rect::bounding(&points) else {
        return;
    };
    let start = out.len();
    CurveGeometry {
        points,
        cubic,
        extend: [None; 2],
    }
    .flatten(clip.inflate(1.0), out);
    // The flattener includes both ends: keep the interior samples only.
    if out.len() > start {
        out.pop();
        out.remove(start);
    }
}

/// Interior samples of an SVG endpoint-parameterized elliptical arc (SVG 1.1 F.6.5), in px.
#[allow(clippy::too_many_arguments)] // the arc command's own parameters plus the output
fn push_arc(
    from: (f64, f64),
    to: (f64, f64),
    (rx, ry): (f64, f64),
    rotation_degrees: f64,
    large: bool,
    sweep: bool,
    scale: f64,
    out: &mut Vec<(f64, f64)>,
) {
    let (mut rx, mut ry) = (rx.abs(), ry.abs());
    if rx <= f64::EPSILON || ry <= f64::EPSILON || from == to {
        return;
    }
    let (sin, cos) = rotation_degrees.to_radians().sin_cos();
    let (hx, hy) = ((from.0 - to.0) / 2.0, (from.1 - to.1) / 2.0);
    let (x1, y1) = (cos * hx + sin * hy, -sin * hx + cos * hy);
    let lambda = (x1 / rx).powi(2) + (y1 / ry).powi(2);
    if lambda > 1.0 {
        rx *= lambda.sqrt();
        ry *= lambda.sqrt();
    }
    let numerator = (rx * ry).powi(2) - (rx * y1).powi(2) - (ry * x1).powi(2);
    let denominator = (rx * y1).powi(2) + (ry * x1).powi(2);
    let mut factor = (numerator / denominator).max(0.0).sqrt();
    if large == sweep {
        factor = -factor;
    }
    let (cx1, cy1) = (factor * rx * y1 / ry, -factor * ry * x1 / rx);
    let center = (
        cos * cx1 - sin * cy1 + (from.0 + to.0) / 2.0,
        sin * cx1 + cos * cy1 + (from.1 + to.1) / 2.0,
    );
    let angle = |ux: f64, uy: f64| uy.atan2(ux);
    let start = angle((x1 - cx1) / rx, (y1 - cy1) / ry);
    let mut delta = angle((-x1 - cx1) / rx, (-y1 - cy1) / ry) - start;
    if sweep && delta < 0.0 {
        delta += std::f64::consts::TAU;
    } else if !sweep && delta > 0.0 {
        delta -= std::f64::consts::TAU;
    }
    let segments = arc_segments(rx.max(ry) * scale, delta);
    for step in 1..segments {
        let theta = start + delta * f64::from(step) / f64::from(segments);
        let (x, y) = (rx * theta.cos(), ry * theta.sin());
        out.push((
            (center.0 + cos * x - sin * y) * scale,
            (center.1 + sin * x + cos * y) * scale,
        ));
    }
}

/// A cursor over SVG path data: single-letter commands and numbers separated by whitespace,
/// commas, or a sign/second decimal point.
struct PathNumbers<'a> {
    rest: &'a str,
}

impl PathNumbers<'_> {
    fn skip_separators(&mut self) {
        self.rest = self
            .rest
            .trim_start_matches(|c: char| c.is_ascii_whitespace() || c == ',');
    }

    fn at_end(&mut self) -> bool {
        self.skip_separators();
        self.rest.is_empty()
    }

    fn command(&mut self) -> Option<u8> {
        self.skip_separators();
        let &byte = self.rest.as_bytes().first()?;
        (byte.is_ascii_alphabetic() && byte != b'e' && byte != b'E').then(|| {
            self.rest = &self.rest[1..];
            byte
        })
    }

    /// An arc flag: a single `0` or `1`, which SVG allows to run into the next number.
    fn flag(&mut self) -> Option<bool> {
        self.skip_separators();
        let flag = match self.rest.as_bytes().first()? {
            b'0' => false,
            b'1' => true,
            _ => return None,
        };
        self.rest = &self.rest[1..];
        Some(flag)
    }

    fn next(&mut self) -> Option<f64> {
        self.skip_separators();
        let bytes = self.rest.as_bytes();
        let mut end = usize::from(matches!(bytes.first(), Some(b'-' | b'+')));
        let mut seen_dot = false;
        while let Some(&byte) = bytes.get(end) {
            match byte {
                b'0'..=b'9' => {}
                b'.' if !seen_dot => seen_dot = true,
                b'e' | b'E' if matches!(bytes.get(end + 1), Some(b'-' | b'+' | b'0'..=b'9')) => {
                    end += 1;
                }
                _ => break,
            }
            end += 1;
        }
        let value = self.rest[..end].parse().ok()?;
        self.rest = &self.rest[end..];
        Some(value)
    }

    fn pair(&mut self) -> Option<(f64, f64)> {
        Some((self.next()?, self.next()?))
    }
}

pub(crate) const BUILTIN_ICONS: &[BuiltinIcon] = &[
    BuiltinIcon {
        name: "arrow-fat-up",
        d: "M231.39,123.06A8,8,0,0,1,224,128H184v80a16,16,0,0,1-16,16H88a16,16,0,0,1-16-16V128H32a8,8,0,0,1-5.66-13.66l96-96a8,8,0,0,1,11.32,0l96,96A8,8,0,0,1,231.39,123.06Z",
    },
    BuiltinIcon {
        name: "arrow-fat-down",
        d: "M229.66,141.66l-96,96a8,8,0,0,1-11.32,0l-96-96A8,8,0,0,1,32,128H72V48A16,16,0,0,1,88,32h80a16,16,0,0,1,16,16v80h40a8,8,0,0,1,5.66,13.66Z",
    },
    BuiltinIcon {
        name: "arrow-fat-left",
        d: "M224,88v80a16,16,0,0,1-16,16H128v40a8,8,0,0,1-13.66,5.66l-96-96a8,8,0,0,1,0-11.32l96-96A8,8,0,0,1,128,32V72h80A16,16,0,0,1,224,88Z",
    },
    BuiltinIcon {
        name: "arrow-fat-right",
        d: "M237.66,133.66l-96,96A8,8,0,0,1,128,224V184H48a16,16,0,0,1-16-16V88A16,16,0,0,1,48,72h80V32a8,8,0,0,1,13.66-5.66l96,96A8,8,0,0,1,237.66,133.66Z",
    },
    BuiltinIcon {
        name: "arrow-up-right",
        d: "M200,64V168a8,8,0,0,1-13.66,5.66L140,127.31,69.66,197.66a8,8,0,0,1-11.32-11.32L128.69,116,82.34,69.66A8,8,0,0,1,88,56H192A8,8,0,0,1,200,64Z",
    },
    BuiltinIcon {
        name: "arrow-up-left",
        d: "M197.66,197.66a8,8,0,0,1-11.32,0L116,127.31,69.66,173.66A8,8,0,0,1,56,168V64a8,8,0,0,1,8-8H168a8,8,0,0,1,5.66,13.66L127.31,116l70.35,70.34A8,8,0,0,1,197.66,197.66Z",
    },
    BuiltinIcon {
        name: "arrow-down-right",
        d: "M200,88V192a8,8,0,0,1-8,8H88a8,8,0,0,1-5.66-13.66L128.69,140,58.34,69.66A8,8,0,0,1,69.66,58.34L140,128.69l46.34-46.35A8,8,0,0,1,200,88Z",
    },
    BuiltinIcon {
        name: "arrow-down-left",
        d: "M197.66,69.66,127.31,140l46.35,46.34A8,8,0,0,1,168,200H64a8,8,0,0,1-8-8V88a8,8,0,0,1,13.66-5.66L116,128.69l70.34-70.35a8,8,0,0,1,11.32,11.32Z",
    },
    BuiltinIcon {
        name: "trend-up",
        d: "M240,56v64a8,8,0,0,1-13.66,5.66L200,99.31l-58.34,58.35a8,8,0,0,1-11.32,0L96,123.31,29.66,189.66a8,8,0,0,1-11.32-11.32l72-72a8,8,0,0,1,11.32,0L136,140.69,188.69,88,162.34,61.66A8,8,0,0,1,168,48h64A8,8,0,0,1,240,56Z",
    },
    BuiltinIcon {
        name: "trend-down",
        d: "M240,128v64a8,8,0,0,1-8,8H168a8,8,0,0,1-5.66-13.66L188.69,160,136,107.31l-34.34,34.35a8,8,0,0,1-11.32,0l-72-72A8,8,0,0,1,29.66,58.34L96,124.69l34.34-34.35a8,8,0,0,1,11.32,0L200,148.69l26.34-26.35A8,8,0,0,1,240,128Z",
    },
    BuiltinIcon {
        name: "chart-line-up",
        d: "M216,40H40A16,16,0,0,0,24,56V200a16,16,0,0,0,16,16H216a16,16,0,0,0,16-16V56A16,16,0,0,0,216,40ZM200,192H56a8,8,0,0,1-8-8V72a8,8,0,0,1,16,0v76.69l34.34-34.35a8,8,0,0,1,11.32,0L128,132.69,172.69,88H144a8,8,0,0,1,0-16h48a8,8,0,0,1,8,8v48a8,8,0,0,1-16,0V99.31l-50.34,50.35a8,8,0,0,1-11.32,0L104,131.31l-40,40V176H200a8,8,0,0,1,0,16Z",
    },
    BuiltinIcon {
        name: "check-circle",
        d: "M128,24A104,104,0,1,0,232,128,104.11,104.11,0,0,0,128,24Zm45.66,85.66-56,56a8,8,0,0,1-11.32,0l-24-24a8,8,0,0,1,11.32-11.32L112,148.69l50.34-50.35a8,8,0,0,1,11.32,11.32Z",
    },
    BuiltinIcon {
        name: "x-circle",
        d: "M128,24A104,104,0,1,0,232,128,104.11,104.11,0,0,0,128,24Zm37.66,130.34a8,8,0,0,1-11.32,11.32L128,139.31l-26.34,26.35a8,8,0,0,1-11.32-11.32L116.69,128,90.34,101.66a8,8,0,0,1,11.32-11.32L128,116.69l26.34-26.35a8,8,0,0,1,11.32,11.32L139.31,128Z",
    },
    BuiltinIcon {
        name: "warning",
        d: "M236.8,188.09,149.35,36.22h0a24.76,24.76,0,0,0-42.7,0L19.2,188.09a23.51,23.51,0,0,0,0,23.72A24.35,24.35,0,0,0,40.55,224h174.9a24.35,24.35,0,0,0,21.33-12.19A23.51,23.51,0,0,0,236.8,188.09ZM120,104a8,8,0,0,1,16,0v40a8,8,0,0,1-16,0Zm8,88a12,12,0,1,1,12-12A12,12,0,0,1,128,192Z",
    },
    BuiltinIcon {
        name: "warning-circle",
        d: "M128,24A104,104,0,1,0,232,128,104.11,104.11,0,0,0,128,24Zm-8,56a8,8,0,0,1,16,0v56a8,8,0,0,1-16,0Zm8,104a12,12,0,1,1,12-12A12,12,0,0,1,128,184Z",
    },
    BuiltinIcon {
        name: "info",
        d: "M128,24A104,104,0,1,0,232,128,104.11,104.11,0,0,0,128,24Zm-4,48a12,12,0,1,1-12,12A12,12,0,0,1,124,72Zm12,112a16,16,0,0,1-16-16V128a8,8,0,0,1,0-16,16,16,0,0,1,16,16v40a8,8,0,0,1,0,16Z",
    },
    BuiltinIcon {
        name: "question",
        d: "M128,24A104,104,0,1,0,232,128,104.11,104.11,0,0,0,128,24Zm0,168a12,12,0,1,1,12-12A12,12,0,0,1,128,192Zm8-48.72V144a8,8,0,0,1-16,0v-8a8,8,0,0,1,8-8c13.23,0,24-9,24-20s-10.77-20-24-20-24,9-24,20v4a8,8,0,0,1-16,0v-4c0-19.85,17.94-36,40-36s40,16.15,40,36C168,125.38,154.24,139.93,136,143.28Z",
    },
    BuiltinIcon {
        name: "flag",
        d: "M232,56V176a8,8,0,0,1-2.76,6c-15.28,13.23-29.89,18-43.82,18-18.91,0-36.57-8.74-53-16.85C105.87,170,82.79,158.61,56,179.77V224a8,8,0,0,1-16,0V56a8,8,0,0,1,2.77-6h0c36-31.18,68.31-15.21,96.79-1.12C167,62.46,190.79,74.2,218.76,50A8,8,0,0,1,232,56Z",
    },
    BuiltinIcon {
        name: "flag-pennant",
        d: "M248,104a8,8,0,0,1-5.37,7.56L64,173.69V216a8,8,0,0,1-16,0V40a8,8,0,0,1,10.63-7.56l184,64A8,8,0,0,1,248,104Z",
    },
    BuiltinIcon {
        name: "flag-checkered",
        d: "M227.32,48.75A8,8,0,0,0,218.76,50c-28,24.22-51.72,12.48-79.21-1.13C111.07,34.76,78.78,18.79,42.76,50h0A8,8,0,0,0,40,56V224a8,8,0,0,0,16,0V179.77c26.79-21.16,49.87-9.75,76.45,3.41,16.4,8.11,34.06,16.85,53,16.85,13.93,0,28.54-4.75,43.82-18a8,8,0,0,0,2.76-6V56A8,8,0,0,0,227.32,48.75ZM56,160.44V109.88c16.85-11.28,32.64-11.59,48-7.34v51.74C88.87,150.47,72.87,150.71,56,160.44ZM104,50.87c9.25,2.83,18.61,7.45,28.45,12.32,11.26,5.57,23.11,11.43,35.55,14.56v51.74c15.35,4.25,31.14,3.94,48-7.35v50.11c-16.87,13.32-32.27,13.72-48,8.91V129.49c-21.62-6-42.38-21-64-26.95Z",
    },
    BuiltinIcon {
        name: "currency-dollar",
        d: "M160,152a16,16,0,0,1-16,16h-8V136h8A16,16,0,0,1,160,152Zm72-24A104,104,0,1,1,128,24,104.11,104.11,0,0,1,232,128Zm-56,24a32,32,0,0,0-32-32h-8V88h4a16,16,0,0,1,16,16,8,8,0,0,0,16,0,32,32,0,0,0-32-32h-4V64a8,8,0,0,0-16,0v8h-4a32,32,0,0,0,0,64h4v32h-8a16,16,0,0,1-16-16,8,8,0,0,0-16,0,32,32,0,0,0,32,32h8v8a8,8,0,0,0,16,0v-8h8A32,32,0,0,0,176,152Zm-76-48a16,16,0,0,0,16,16h4V88h-4A16,16,0,0,0,100,104Z",
    },
    BuiltinIcon {
        name: "currency-circle-dollar",
        d: "M128,24A104,104,0,1,0,232,128,104.11,104.11,0,0,0,128,24Zm12,152h-4v8a8,8,0,0,1-16,0v-8H104a8,8,0,0,1,0-16h36a12,12,0,0,0,0-24H116a28,28,0,0,1,0-56h4V72a8,8,0,0,1,16,0v8h16a8,8,0,0,1,0,16H116a12,12,0,0,0,0,24h24a28,28,0,0,1,0,56Z",
    },
    BuiltinIcon {
        name: "currency-eur",
        d: "M128,24A104,104,0,1,0,232,128,104.11,104.11,0,0,0,128,24Zm0,80a8,8,0,0,1,0,16H88v16h24a8,8,0,0,1,0,16H88.81a40,40,0,0,0,65.86,21.82,8,8,0,1,1,10.66,11.92A56,56,0,0,1,72.58,152H64a8,8,0,0,1,0-16h8V120H64a8,8,0,0,1,0-16h8.58a56,56,0,0,1,92.75-33.74,8,8,0,1,1-10.66,11.92A40,40,0,0,0,88.81,104Z",
    },
    BuiltinIcon {
        name: "currency-gbp",
        d: "M128,24A104,104,0,1,0,232,128,104.11,104.11,0,0,0,128,24Zm40,160H88a8,8,0,0,1,0-16,16,16,0,0,0,16-16V136H88a8,8,0,0,1,0-16h16V96a40,40,0,0,1,60-34.64,8,8,0,0,1-8,13.85A24,24,0,0,0,120,96v24h16a8,8,0,0,1,0,16H120v16a31.71,31.71,0,0,1-4.31,16H168a8,8,0,0,1,0,16Z",
    },
    BuiltinIcon {
        name: "currency-jpy",
        d: "M128,24A104,104,0,1,0,232,128,104.11,104.11,0,0,0,128,24Zm54.4,52.8L144,128h16a8,8,0,0,1,0,16H136v16h24a8,8,0,0,1,0,16H136v16a8,8,0,0,1-16,0V176H96a8,8,0,0,1,0-16h24V144H96a8,8,0,0,1,0-16h16L73.6,76.8a8,8,0,1,1,12.8-9.6L128,122.67,169.6,67.2a8,8,0,0,1,12.8,9.6Z",
    },
    BuiltinIcon {
        name: "currency-btc",
        d: "M176,152a16,16,0,0,1-16,16H112V136h48A16,16,0,0,1,176,152Zm64-24A104,104,0,1,1,136,24,104.11,104.11,0,0,1,240,128Zm-48,24a32,32,0,0,0-15.51-27.42A32,32,0,0,0,160,73V64a8,8,0,0,0-16,0v8H128V64a8,8,0,0,0-16,0v8H96a8,8,0,0,0,0,16v80a8,8,0,0,0,0,16h16v8a8,8,0,0,0,16,0v-8h16v8a8,8,0,0,0,16,0v-8A32,32,0,0,0,192,152Zm-24-48a16,16,0,0,0-16-16H112v32h40A16,16,0,0,0,168,104Z",
    },
    BuiltinIcon {
        name: "currency-eth",
        d: "M222.29,123.06l-88-112a8,8,0,0,0-12.58,0l-88,112a8,8,0,0,0,0,9.88l88,112a8,8,0,0,0,12.58,0l88-112A8,8,0,0,0,222.29,123.06ZM136,155.58V39.13l67.42,85.8Zm-16,0L52.58,124.93,120,39.13Zm0,17.57v43.72l-53.43-68Z",
    },
    BuiltinIcon {
        name: "coins",
        d: "M184,89.57V84c0-25.08-37.83-44-88-44S8,58.92,8,84v40c0,20.89,26.25,37.49,64,42.46V172c0,25.08,37.83,44,88,44s88-18.92,88-44V132C248,111.3,222.58,94.68,184,89.57ZM56,146.87C36.41,141.4,24,132.39,24,124V109.93c8.16,5.78,19.09,10.44,32,13.57Zm80-23.37c12.91-3.13,23.84-7.79,32-13.57V124c0,8.39-12.41,17.4-32,22.87Zm-16,71.37C100.41,189.4,88,180.39,88,172v-4.17c2.63.1,5.29.17,8,.17,3.88,0,7.67-.13,11.39-.35A121.92,121.92,0,0,0,120,171.41Zm0-44.62A163,163,0,0,1,96,152a163,163,0,0,1-24-1.75V126.46A183.74,183.74,0,0,0,96,128a183.74,183.74,0,0,0,24-1.54Zm64,48a165.45,165.45,0,0,1-48,0V174.4a179.48,179.48,0,0,0,24,1.6,183.74,183.74,0,0,0,24-1.54ZM232,172c0,8.39-12.41,17.4-32,22.87V171.5c12.91-3.13,23.84-7.79,32-13.57Z",
    },
    BuiltinIcon {
        name: "money",
        d: "M168,128a40,40,0,1,1-40-40A40,40,0,0,1,168,128Zm80-64V192a8,8,0,0,1-8,8H16a8,8,0,0,1-8-8V64a8,8,0,0,1,8-8H240A8,8,0,0,1,248,64Zm-16,46.35A56.78,56.78,0,0,1,193.65,72H62.35A56.78,56.78,0,0,1,24,110.35v35.3A56.78,56.78,0,0,1,62.35,184h131.3A56.78,56.78,0,0,1,232,145.65Z",
    },
    BuiltinIcon {
        name: "target",
        d: "M221.87,83.16A104.1,104.1,0,1,1,195.67,49l22.67-22.68a8,8,0,0,1,11.32,11.32L167.6,99.71h0l-37.71,37.71-23.95,23.95a40,40,0,0,0,62-35.67,8,8,0,1,1,16-.9,56,56,0,0,1-95.5,42.79h0a56,56,0,0,1,73.13-84.43L184.3,60.39a87.88,87.88,0,1,0,23.13,29.67,8,8,0,0,1,14.44-6.9Z",
    },
    BuiltinIcon {
        name: "rocket-launch",
        d: "M101.85,191.14C97.34,201,82.29,224,40,224a8,8,0,0,1-8-8c0-42.29,23-57.34,32.86-61.85a8,8,0,0,1,6.64,14.56c-6.43,2.93-20.62,12.36-23.12,38.91,26.55-2.5,36-16.69,38.91-23.12a8,8,0,1,1,14.56,6.64Zm122-144a16,16,0,0,0-15-15c-12.58-.75-44.73.4-71.4,27.07h0L88,108.7A8,8,0,0,1,76.67,97.39l26.56-26.57A4,4,0,0,0,100.41,64H74.35A15.9,15.9,0,0,0,63,68.68L28.7,103a16,16,0,0,0,9.07,27.16l38.47,5.37,44.21,44.21,5.37,38.49a15.94,15.94,0,0,0,10.78,12.92,16.11,16.11,0,0,0,5.1.83A15.91,15.91,0,0,0,153,227.3L187.32,193A16,16,0,0,0,192,181.65V155.59a4,4,0,0,0-6.83-2.82l-26.57,26.56a8,8,0,0,1-11.71-.42,8.2,8.2,0,0,1,.6-11.1l49.27-49.27h0C223.45,91.86,224.6,59.71,223.85,47.12Z",
    },
    BuiltinIcon {
        name: "fire",
        d: "M143.38,17.85a8,8,0,0,0-12.63,3.41l-22,60.41L84.59,58.26a8,8,0,0,0-11.93.89C51,87.53,40,116.08,40,144a88,88,0,0,0,176,0C216,84.55,165.21,36,143.38,17.85Zm40.51,135.49a57.6,57.6,0,0,1-46.56,46.55A7.65,7.65,0,0,1,136,200a8,8,0,0,1-1.32-15.89c16.57-2.79,30.63-16.85,33.44-33.45a8,8,0,0,1,15.78,2.68Z",
    },
    BuiltinIcon {
        name: "lightning",
        d: "M213.85,125.46l-112,120a8,8,0,0,1-13.69-7l14.66-73.33L45.19,143.49a8,8,0,0,1-3-13l112-120a8,8,0,0,1,13.69,7L153.18,90.9l57.63,21.61a8,8,0,0,1,3,12.95Z",
    },
    BuiltinIcon {
        name: "lightbulb",
        d: "M176,232a8,8,0,0,1-8,8H88a8,8,0,0,1,0-16h80A8,8,0,0,1,176,232Zm40-128a87.55,87.55,0,0,1-33.64,69.21A16.24,16.24,0,0,0,176,186v6a16,16,0,0,1-16,16H96a16,16,0,0,1-16-16v-6a16,16,0,0,0-6.23-12.66A87.59,87.59,0,0,1,40,104.49C39.74,56.83,78.26,17.14,125.88,16A88,88,0,0,1,216,104Zm-32.11-9.34a57.6,57.6,0,0,0-46.56-46.55,8,8,0,0,0-2.66,15.78c16.57,2.79,30.63,16.85,33.44,33.45A8,8,0,0,0,176,104a9,9,0,0,0,1.35-.11A8,8,0,0,0,183.89,94.66Z",
    },
    BuiltinIcon {
        name: "bell",
        d: "M221.8,175.94C216.25,166.38,208,139.33,208,104a80,80,0,1,0-160,0c0,35.34-8.26,62.38-13.81,71.94A16,16,0,0,0,48,200H88.81a40,40,0,0,0,78.38,0H208a16,16,0,0,0,13.8-24.06ZM128,216a24,24,0,0,1-22.62-16h45.24A24,24,0,0,1,128,216Z",
    },
    BuiltinIcon {
        name: "lock",
        d: "M208,80H176V56a48,48,0,0,0-96,0V80H48A16,16,0,0,0,32,96V208a16,16,0,0,0,16,16H208a16,16,0,0,0,16-16V96A16,16,0,0,0,208,80Zm-80,84a12,12,0,1,1,12-12A12,12,0,0,1,128,164Zm32-84H96V56a32,32,0,0,1,64,0Z",
    },
    BuiltinIcon {
        name: "clock",
        d: "M128,24A104,104,0,1,0,232,128,104.11,104.11,0,0,0,128,24Zm56,112H128a8,8,0,0,1-8-8V72a8,8,0,0,1,16,0v48h48a8,8,0,0,1,0,16Z",
    },
    BuiltinIcon {
        name: "calendar",
        d: "M208,32H184V24a8,8,0,0,0-16,0v8H88V24a8,8,0,0,0-16,0v8H48A16,16,0,0,0,32,48V208a16,16,0,0,0,16,16H208a16,16,0,0,0,16-16V48A16,16,0,0,0,208,32ZM112,184a8,8,0,0,1-16,0V132.94l-4.42,2.22a8,8,0,0,1-7.16-14.32l16-8A8,8,0,0,1,112,120Zm56-8a8,8,0,0,1,0,16H136a8,8,0,0,1-6.4-12.8l28.78-38.37A8,8,0,1,0,145.07,132a8,8,0,1,1-13.85-8A24,24,0,0,1,176,136a23.76,23.76,0,0,1-4.84,14.45L152,176ZM48,80V48H72v8a8,8,0,0,0,16,0V48h80v8a8,8,0,0,0,16,0V48h24V80Z",
    },
    BuiltinIcon {
        name: "push-pin",
        d: "M235.33,104l-53.47,53.65c4.56,12.67,6.45,33.89-13.19,60A15.93,15.93,0,0,1,157,224c-.38,0-.75,0-1.13,0a16,16,0,0,1-11.32-4.69L96.29,171,53.66,213.66a8,8,0,0,1-11.32-11.32L85,159.71l-48.3-48.3A16,16,0,0,1,38,87.63c25.42-20.51,49.75-16.48,60.4-13.14L152,20.7a16,16,0,0,1,22.63,0l60.69,60.68A16,16,0,0,1,235.33,104Z",
    },
    BuiltinIcon {
        name: "map-pin",
        d: "M128,16a88.1,88.1,0,0,0-88,88c0,75.3,80,132.17,83.41,134.55a8,8,0,0,0,9.18,0C136,236.17,216,179.3,216,104A88.1,88.1,0,0,0,128,16Zm0,56a32,32,0,1,1-32,32A32,32,0,0,1,128,72Z",
    },
    BuiltinIcon {
        name: "newspaper",
        d: "M216,48H56A16,16,0,0,0,40,64V184a8,8,0,0,1-16,0V88A8,8,0,0,0,8,88v96.11A24,24,0,0,0,32,208H208a24,24,0,0,0,24-24V64A16,16,0,0,0,216,48ZM176,152H96a8,8,0,0,1,0-16h80a8,8,0,0,1,0,16Zm0-32H96a8,8,0,0,1,0-16h80a8,8,0,0,1,0,16Z",
    },
    BuiltinIcon {
        name: "megaphone",
        d: "M200,72H160.2c-2.91-.17-53.62-3.74-101.91-44.24A16,16,0,0,0,32,40V200a16,16,0,0,0,26.29,12.25c37.77-31.68,77-40.76,93.71-43.3v31.72A16,16,0,0,0,159.12,214l11,7.33A16,16,0,0,0,194.5,212l11.77-44.36A48,48,0,0,0,200,72ZM179,207.89l0,.11-11-7.33V168h21.6ZM200,152H168V88h32a32,32,0,1,1,0,64Z",
    },
    BuiltinIcon {
        name: "crown",
        d: "M248,80a28,28,0,1,0-51.12,15.77l-26.79,33L146,73.4a28,28,0,1,0-36.06,0L85.91,128.74l-26.79-33a28,28,0,1,0-26.6,12L47,194.63A16,16,0,0,0,62.78,208H193.22A16,16,0,0,0,209,194.63l14.47-86.85A28,28,0,0,0,248,80ZM128,40a12,12,0,1,1-12,12A12,12,0,0,1,128,40ZM24,80A12,12,0,1,1,36,92,12,12,0,0,1,24,80ZM220,92a12,12,0,1,1,12-12A12,12,0,0,1,220,92Z",
    },
    BuiltinIcon {
        name: "trophy",
        d: "M232,64H208V48a8,8,0,0,0-8-8H56a8,8,0,0,0-8,8V64H24A16,16,0,0,0,8,80V96a40,40,0,0,0,40,40h3.65A80.13,80.13,0,0,0,120,191.61V216H96a8,8,0,0,0,0,16h64a8,8,0,0,0,0-16H136V191.58c31.94-3.23,58.44-25.64,68.08-55.58H208a40,40,0,0,0,40-40V80A16,16,0,0,0,232,64ZM48,120A24,24,0,0,1,24,96V80H48v32q0,4,.39,8ZM232,96a24,24,0,0,1-24,24h-.5a81.81,81.81,0,0,0,.5-8.9V80h24Z",
    },
    BuiltinIcon {
        name: "thumbs-up",
        d: "M234,80.12A24,24,0,0,0,216,72H160V56a40,40,0,0,0-40-40,8,8,0,0,0-7.16,4.42L75.06,96H32a16,16,0,0,0-16,16v88a16,16,0,0,0,16,16H204a24,24,0,0,0,23.82-21l12-96A24,24,0,0,0,234,80.12ZM32,112H72v88H32Z",
    },
    BuiltinIcon {
        name: "thumbs-down",
        d: "M239.82,157l-12-96A24,24,0,0,0,204,40H32A16,16,0,0,0,16,56v88a16,16,0,0,0,16,16H75.06l37.78,75.58A8,8,0,0,0,120,240a40,40,0,0,0,40-40V184h56a24,24,0,0,0,23.82-27ZM72,144H32V56H72Z",
    },
    BuiltinIcon {
        name: "smiley",
        d: "M128,24A104,104,0,1,0,232,128,104.11,104.11,0,0,0,128,24ZM92,96a12,12,0,1,1-12,12A12,12,0,0,1,92,96Zm82.92,60c-10.29,17.79-27.39,28-46.92,28s-36.63-10.2-46.92-28a8,8,0,1,1,13.84-8c7.47,12.91,19.21,20,33.08,20s25.61-7.1,33.08-20a8,8,0,1,1,13.84,8ZM164,120a12,12,0,1,1,12-12A12,12,0,0,1,164,120Z",
    },
    BuiltinIcon {
        name: "smiley-sad",
        d: "M128,24A104,104,0,1,0,232,128,104.11,104.11,0,0,0,128,24ZM92,96a12,12,0,1,1-12,12A12,12,0,0,1,92,96Zm80,86.92A8,8,0,0,1,161.08,180c-7.47-12.91-19.21-20-33.08-20s-25.61,7.1-33.08,20a8,8,0,1,1-13.84-8c10.29-17.79,27.39-28,46.92-28s36.63,10.2,46.92,28A8,8,0,0,1,172,182.92ZM164,120a12,12,0,1,1,12-12A12,12,0,0,1,164,120Z",
    },
    BuiltinIcon {
        name: "smiley-angry",
        d: "M128,24A104,104,0,1,0,232,128,104.11,104.11,0,0,0,128,24ZM80,140a12,12,0,1,1,12,12A12,12,0,0,1,80,140Zm78.66,48.43a8,8,0,0,1-11.09,2.23C141.07,186.34,136,184,128,184s-13.07,2.34-19.57,6.66a8,8,0,0,1-8.86-13.32C108,171.73,116.06,168,128,168s20,3.73,28.43,9.34A8,8,0,0,1,158.66,188.43ZM164,152a12,12,0,1,1,12-12A12,12,0,0,1,164,152Zm16.44-57.34-48,32a8,8,0,0,1-8.88,0l-48-32a8,8,0,1,1,8.88-13.32L128,110.39l43.56-29a8,8,0,0,1,8.88,13.32Z",
    },
    BuiltinIcon {
        name: "sun",
        d: "M120,40V16a8,8,0,0,1,16,0V40a8,8,0,0,1-16,0Zm8,24a64,64,0,1,0,64,64A64.07,64.07,0,0,0,128,64ZM58.34,69.66A8,8,0,0,0,69.66,58.34l-16-16A8,8,0,0,0,42.34,53.66Zm0,116.68-16,16a8,8,0,0,0,11.32,11.32l16-16a8,8,0,0,0-11.32-11.32ZM192,72a8,8,0,0,0,5.66-2.34l16-16a8,8,0,0,0-11.32-11.32l-16,16A8,8,0,0,0,192,72Zm5.66,114.34a8,8,0,0,0-11.32,11.32l16,16a8,8,0,0,0,11.32-11.32ZM48,128a8,8,0,0,0-8-8H16a8,8,0,0,0,0,16H40A8,8,0,0,0,48,128Zm80,80a8,8,0,0,0-8,8v24a8,8,0,0,0,16,0V216A8,8,0,0,0,128,208Zm112-88H216a8,8,0,0,0,0,16h24a8,8,0,0,0,0-16Z",
    },
    BuiltinIcon {
        name: "moon",
        d: "M235.54,150.21a104.84,104.84,0,0,1-37,52.91A104,104,0,0,1,32,120,103.09,103.09,0,0,1,52.88,57.48a104.84,104.84,0,0,1,52.91-37,8,8,0,0,1,10,10,88.08,88.08,0,0,0,109.8,109.8,8,8,0,0,1,10,10Z",
    },
    BuiltinIcon {
        name: "cloud",
        d: "M160.06,40A88.1,88.1,0,0,0,81.29,88.67h0A87.48,87.48,0,0,0,72,127.73,8.18,8.18,0,0,1,64.57,136,8,8,0,0,1,56,128a103.66,103.66,0,0,1,5.34-32.92,4,4,0,0,0-4.75-5.18A64.09,64.09,0,0,0,8,152c0,35.19,29.75,64,65,64H160a88.09,88.09,0,0,0,87.93-91.48C246.11,77.54,207.07,40,160.06,40Z",
    },
    BuiltinIcon {
        name: "umbrella",
        d: "M240,126.63A112.21,112.21,0,0,0,128,24h0A112.21,112.21,0,0,0,16.05,126.63,16,16,0,0,0,32,144h88v56a32,32,0,0,0,64,0,8,8,0,0,0-16,0,16,16,0,0,1-32,0V144h88a16,16,0,0,0,16-17.37ZM32,128a96.15,96.15,0,0,1,76.2-85.89C96.48,58,81.85,86.11,80.17,128H32Zm143.83,0c-1.68-41.89-16.31-70-28-85.94A96.07,96.07,0,0,1,224,128Z",
    },
    BuiltinIcon {
        name: "star",
        d: "M234.29,114.85l-45,38.83L203,211.75a16.4,16.4,0,0,1-24.5,17.82L128,198.49,77.47,229.57A16.4,16.4,0,0,1,53,211.75l13.76-58.07-45-38.83A16.46,16.46,0,0,1,31.08,86l59-4.76,22.76-55.08a16.36,16.36,0,0,1,30.27,0l22.75,55.08,59,4.76a16.46,16.46,0,0,1,9.37,28.86Z",
    },
    BuiltinIcon {
        name: "circle",
        d: "M232,128A104,104,0,1,1,128,24,104.13,104.13,0,0,1,232,128Z",
    },
    BuiltinIcon {
        name: "square",
        d: "M224,48V208a16,16,0,0,1-16,16H48a16,16,0,0,1-16-16V48A16,16,0,0,1,48,32H208A16,16,0,0,1,224,48Z",
    },
    BuiltinIcon {
        name: "triangle",
        d: "M236.78,211.81A24.34,24.34,0,0,1,215.45,224H40.55a24.34,24.34,0,0,1-21.33-12.19,23.51,23.51,0,0,1,0-23.72L106.65,36.22a24.76,24.76,0,0,1,42.7,0L236.8,188.09A23.51,23.51,0,0,1,236.78,211.81Z",
    },
    BuiltinIcon {
        name: "diamond",
        d: "M240,128a15.85,15.85,0,0,1-4.67,11.28l-96.05,96.06a16,16,0,0,1-22.56,0h0l-96-96.06a16,16,0,0,1,0-22.56l96.05-96.06a16,16,0,0,1,22.56,0l96.05,96.06A15.85,15.85,0,0,1,240,128Z",
    },
    BuiltinIcon {
        name: "heart",
        d: "M240,102c0,70-103.79,126.66-108.21,129a8,8,0,0,1-7.58,0C119.79,228.66,16,172,16,102A62.07,62.07,0,0,1,78,40c20.65,0,38.73,8.88,50,23.89C139.27,48.88,157.35,40,178,40A62.07,62.07,0,0,1,240,102Z",
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    fn covered_area(coverage: &[f64]) -> f64 {
        coverage.iter().sum()
    }

    #[test]
    fn coverage_is_exact_area_and_respects_holes() {
        // An axis-aligned 10x6 rectangle at a fractional offset covers exactly 60 px.
        let rect = vec![vec![
            (2.25, 3.5),
            (12.25, 3.5),
            (12.25, 9.5),
            (2.25, 9.5),
            (2.25, 3.5),
        ]];
        let coverage: Vec<f64> = fill_coverage(&rect, 16).collect();
        assert!((covered_area(&coverage) - 60.0).abs() < 1e-9);
        assert!(coverage.iter().all(|&value| (0.0..=1.0).contains(&value)));
        // A sloped triangle: area 0.5 * 12 * 10 = 60, at any orientation.
        let triangle = vec![vec![(1.0, 1.0), (13.0, 1.0), (4.0, 11.0), (1.0, 1.0)]];
        assert!((fill_coverage(&triangle, 16).sum::<f64>() - 60.0).abs() < 1e-9);
        // An opposite-wound inner square cuts a hole (nonzero), a same-wound one does not.
        let outer = vec![
            (0.0, 0.0),
            (16.0, 0.0),
            (16.0, 16.0),
            (0.0, 16.0),
            (0.0, 0.0),
        ];
        let inner = vec![
            (4.0, 4.0),
            (4.0, 12.0),
            (12.0, 12.0),
            (12.0, 4.0),
            (4.0, 4.0),
        ];
        let holed: Vec<f64> = fill_coverage(&[outer.clone(), inner.clone()], 16).collect();
        assert!((covered_area(&holed) - 192.0).abs() < 1e-9);
        assert_eq!(holed[8 * 16 + 8], 0.0);
        let mut same = inner;
        same.reverse();
        assert!((fill_coverage(&[outer, same], 16).sum::<f64>() - 256.0).abs() < 1e-9);
    }

    #[test]
    fn path_parser_flattens_relative_commands_arcs_and_reflections() {
        // Relative lines and H/V close into one square; scale maps units to px.
        let square = flatten_path("M10,10h20v20h-20Z", 0.5);
        assert_eq!(
            square,
            vec![vec![
                (5.0, 5.0),
                (15.0, 5.0),
                (15.0, 15.0),
                (5.0, 15.0),
                (5.0, 5.0)
            ]]
        );
        // Two arcs make a full circle of radius 50 whose samples all lie on it.
        let circle = flatten_path("M50,100a50,50,0,1,0,100,0A50,50,0,1,0,50,100Z", 1.0);
        assert_eq!(circle.len(), 1);
        for &(x, y) in &circle[0] {
            assert!(
                ((x - 100.0).hypot(y - 100.0) - 50.0).abs() < 1e-9,
                "{:?}",
                (x, y)
            );
        }
        assert!(circle[0].len() > 40);
        // Coverage equals the flattened polygon's exact (shoelace) area, which itself is within
        // the chord tolerance of pi r^2.
        let polygon = flatten_path("M50,100a50,50,0,1,0,100,0a50,50,0,1,0-100,0Z", 1.0);
        let shoelace = polygon[0]
            .windows(2)
            .map(|pair| pair[0].0 * pair[1].1 - pair[1].0 * pair[0].1)
            .sum::<f64>()
            .abs()
            / 2.0;
        let coverage: Vec<f64> = fill_coverage(&polygon, 200).collect();
        assert!((covered_area(&coverage) - shoelace).abs() < 1e-6);
        let circle_area = std::f64::consts::PI * 2500.0;
        assert!((shoelace - circle_area).abs() / circle_area < 5e-3);
        // A smooth cubic reflects the previous control point; flags may run into numbers.
        let smooth = flatten_path("M0,0C0,10,10,10,10,0S20,-10,20,0Z", 1.0);
        assert_eq!(smooth.len(), 1);
        let compact = flatten_path("M10,0a5,5,0,0110,0Z", 1.0);
        assert!(compact[0].len() > 3);
    }

    #[test]
    fn the_raster_cache_stays_within_its_byte_budget_and_skips_uncached_rasters() {
        let icon = builtin_icon("square").unwrap();
        let mut cache = IconRasterCache::default();
        let color = |index: u8| Color::rgb(index, 0, 0);
        let largest = (MAX_ICON_RASTER_PX * MAX_ICON_RASTER_PX * 4) as usize;
        let fit = MAX_ICON_RASTER_BYTES / largest;
        // Fill the budget with the largest rasters, then keep the first one recently used.
        let first = cache.raster(icon, MAX_ICON_RASTER_PX, color(0), true);
        for index in 1..fit as u8 {
            cache.raster(icon, MAX_ICON_RASTER_PX, color(index), true);
        }
        assert_eq!(cache.usage(), (fit, fit * largest));
        assert_eq!(
            cache.raster(icon, MAX_ICON_RASTER_PX, color(0), true).key,
            first.key
        );
        // One more evicts the least recently used (the second), never past the budget.
        let second_key = first.key + 1;
        cache.raster(icon, MAX_ICON_RASTER_PX, color(200), true);
        assert_eq!(cache.usage(), (fit, fit * largest));
        assert!(cache.usage().1 <= MAX_ICON_RASTER_BYTES);
        assert_eq!(
            cache.raster(icon, MAX_ICON_RASTER_PX, color(0), true).key,
            first.key
        );
        assert_ne!(
            cache.raster(icon, MAX_ICON_RASTER_PX, color(1), false).key,
            second_key
        );
        // An uncached raster paints once and leaves the cache as it was.
        let usage = cache.usage();
        let once = cache.raster(icon, 64, color(9), false);
        assert_eq!(cache.usage(), usage);
        assert_ne!(cache.raster(icon, 64, color(9), false).key, once.key);
        // Small rasters stop at the entry cap.
        let mut cache = IconRasterCache::default();
        for index in 0..=MAX_ICON_RASTERS {
            cache.raster(icon, 8, Color::rgb(index as u8, 1, 1), true);
        }
        assert_eq!(
            cache.usage(),
            (MAX_ICON_RASTERS, MAX_ICON_RASTERS * 8 * 8 * 4)
        );
    }

    #[test]
    fn every_builtin_icon_fills_inside_its_square() {
        let mut names = std::collections::HashSet::new();
        for icon in BUILTIN_ICONS {
            assert!(names.insert(icon.name), "duplicate {}", icon.name);
            assert!(
                icon.name
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'),
                "{}",
                icon.name
            );
            let contours = icon.contours(48.0 / ICON_VIEWBOX);
            assert!(!contours.is_empty(), "{}", icon.name);
            for &(x, y) in contours.iter().flatten() {
                assert!(
                    (0.0..=48.0).contains(&x) && (0.0..=48.0).contains(&y),
                    "{} point {:?} leaves its 48 px square",
                    icon.name,
                    (x, y)
                );
            }
            let pixels = icon.rasterize(48, Color::rgb(10, 20, 30));
            assert_eq!(pixels.len(), 48 * 48 * 4);
            let inked = pixels.chunks(4).filter(|px| px[3] > 0).count();
            assert!(
                inked > 48 * 48 / 20 && inked < 48 * 48,
                "{} inks {inked} of {} px",
                icon.name,
                48 * 48
            );
        }
        assert!((50..=60).contains(&BUILTIN_ICONS.len()));
        for kind in [
            DrawingKind::ArrowMarkerUp,
            DrawingKind::ArrowMarkerDown,
            DrawingKind::ArrowMarkerLeft,
            DrawingKind::ArrowMarkerRight,
        ] {
            let (name, tip) = arrow_marker_icon(kind).unwrap();
            let icon = builtin_icon(name).unwrap();
            // The declared tip is the outline's extreme point in the arrow's direction.
            let direction = (
                (tip.0 - ICON_VIEWBOX / 2.0).signum() * f64::from(u8::from(tip.0 != 128.0)),
                (tip.1 - ICON_VIEWBOX / 2.0).signum() * f64::from(u8::from(tip.1 != 128.0)),
            );
            let extreme = icon
                .contours(1.0)
                .into_iter()
                .flatten()
                .map(|(x, y)| x * direction.0 + y * direction.1)
                .fold(f64::MIN, f64::max);
            let declared = tip.0 * direction.0 + tip.1 * direction.1;
            assert!(
                (extreme - declared).abs() < 0.5,
                "{name}: {extreme} vs {declared}"
            );
        }
    }
}
