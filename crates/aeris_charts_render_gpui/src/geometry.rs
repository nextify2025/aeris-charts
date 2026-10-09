//! Quad and triangle/path conversion for the GPUI executor.
//!
//! Every helper here reuses Aeris's own pixel math rather than recomputing it:
//! - the integer-rect subset copies the exact expansion the wgpu quad executor and the Canvas2D
//!   executor already agree on (`fillRectInnerBorder`, half-width line centering, dash phase);
//! - the anti-aliased subset reuses `aeris_charts_render::line`'s shared curve expansion and area
//!   tessellation, and adds per-vertex signed-distance coverage to strokes and fill boundaries:
//!   GPUI's path pass can fall back to 1x MSAA on Linux. Polyline, ring, and disc transitions
//!   straddle their nominal edges; filled meshes fade outside their nominal boundary.
//!
//! Uses Aeris's coordinate, bar-width, and snapping calculations.

use aeris_charts_render::color::Color;
use aeris_charts_render::draw_list::{IRect, LineStyle, LineType, segment_points};
pub(crate) use aeris_charts_render::line::round_rect_polygon;
use aeris_charts_render::line::{
    AreaMesh, LineParams, LinePoint, RoundRectBorder, STROKE_AA_SOLID, build_area_fill,
    expand_line_into, stroke_aa,
};

use crate::scene::{DeviceRect, MeshVertex, Paint, SOLID_ST};

/// The identity `LineParams` the tessellators must run with: the shared point pool already carries
/// the DPR (see `aeris_charts_render_wgpu::tri_executor`), so re-scaling here would double-apply it.
pub(crate) fn identity_params(line_width: f64, line_type: LineType) -> LineParams {
    LineParams {
        horizontal_pixel_ratio: 1.0,
        vertical_pixel_ratio: 1.0,
        line_width,
        line_type,
    }
}

/// An integer rect as a device rect, or `None` when degenerate.
///
/// Matches `fill_irect` in the Canvas2D executor and `push_rect` in the wgpu quad executor: a
/// non-positive width or height paints nothing (it is *not* clamped to 1 px).
pub(crate) fn irect(rect: IRect) -> Option<DeviceRect> {
    if rect.w <= 0 || rect.h <= 0 {
        return None;
    }
    Some(DeviceRect::new(
        rect.x as f32,
        rect.y as f32,
        rect.w as f32,
        rect.h as f32,
    ))
}

/// The four edge rects of a `RectFrame`, in the same order both existing executors emit them
/// (top, bottom, left, right) — a port of the reference `fillRectInnerBorder`.
pub(crate) fn rect_frame_edges(rect: IRect, border: i32) -> [IRect; 4] {
    let IRect { x, y, w, h } = rect;
    [
        IRect {
            x: x + border,
            y,
            w: w - border * 2,
            h: border,
        },
        IRect {
            x: x + border,
            y: y + h - border,
            w: w - border * 2,
            h: border,
        },
        IRect { x, y, w: border, h },
        IRect {
            x: x + w - border,
            y,
            w: border,
            h,
        },
    ]
}

/// Emit the filled dash spans of `[from, to)` for `style` at `width`.
///
/// Byte-for-byte the same routine as the wgpu and Canvas2D executors, including the `round()` on
/// each span edge and the phase starting at the path start — so dashed grid lines, price lines and
/// crosshairs land on identical pixels in all three backends.
pub(crate) fn dash_spans(
    style: LineStyle,
    width: i32,
    from: i32,
    to: i32,
    mut emit: impl FnMut(i32, i32),
) {
    let pattern = style.dash_pattern(width as f32);
    if pattern.is_empty() {
        emit(from, to);
        return;
    }
    let mut pos = from as f32;
    let mut i = 0usize;
    let mut on = true;
    while pos < to as f32 {
        let seg = pattern[i % pattern.len()];
        if on {
            let a = pos.round() as i32;
            let b = ((pos + seg).min(to as f32)).round() as i32;
            if b > a {
                emit(a, b);
            }
        }
        pos += seg;
        i += 1;
        on = !on;
    }
}

/// The top edge of a horizontal line of `width` centered on integer `y` — and, transposed, the
/// left edge of a vertical line. `y - width / 2` with integer division, exactly as both existing
/// executors compute it (the half-pixel translate in the reference's `strokeInPixel` makes odd
/// widths symmetric around `y`).
pub(crate) fn line_span_start(center: i32, width: i32) -> i32 {
    center - width / 2
}

/// Append `verts` as a solid-interior triangle list, returning `(first_vertex, vertex_count)`.
pub(crate) fn push_vertices(
    pool: &mut Vec<MeshVertex>,
    verts: impl IntoIterator<Item = [f32; 2]>,
) -> (u32, u32) {
    let first = pool.len() as u32;
    pool.extend(verts.into_iter().map(|[x, y]| MeshVertex::solid(x, y)));
    let count = pool.len() as u32 - first;
    // A partial triangle would make GPUI's `push_triangle` loop drop a vertex silently.
    let count = count - count % 3;
    pool.truncate(first as usize + count as usize);
    (first, count)
}

/// Append one coverage-encoded triangle.
fn push_tri_st(
    pool: &mut Vec<MeshVertex>,
    a: [f32; 2],
    st_a: [f32; 2],
    b: [f32; 2],
    st_b: [f32; 2],
    c: [f32; 2],
    st_c: [f32; 2],
) {
    pool.extend([
        MeshVertex {
            x: a[0],
            y: a[1],
            st: st_a,
        },
        MeshVertex {
            x: b[0],
            y: b[1],
            st: st_b,
        },
        MeshVertex {
            x: c[0],
            y: c[1],
            st: st_c,
        },
    ]);
}

/// Add a one-device-pixel coverage fringe outside a filled polygon. The solid triangles remain
/// unchanged; only boundary edges get extra geometry, so interior seams cannot darken a fill.
fn push_polygon_fringe(pool: &mut Vec<MeshVertex>, polygon: &[[f32; 2]]) {
    let count = if polygon.len() > 1 && polygon.first() == polygon.last() {
        polygon.len() - 1
    } else {
        polygon.len()
    };
    if count < 3 {
        return;
    }
    let polygon = &polygon[..count];
    let signed_area = polygon.iter().enumerate().fold(0.0, |sum, (index, p)| {
        let next = polygon[(index + 1) % count];
        sum + p[0] * next[1] - next[0] * p[1]
    });
    if signed_area.abs() < 1e-6 {
        return;
    }
    let side = signed_area.signum();
    let normal = |a: [f32; 2], b: [f32; 2]| {
        let dx = b[0] - a[0];
        let dy = b[1] - a[1];
        let length = dx.hypot(dy);
        if length <= 1e-6 {
            [0.0, 0.0]
        } else {
            [side * dy / length, -side * dx / length]
        }
    };
    let offset = |index: usize| {
        let previous = polygon[(index + count - 1) % count];
        let current = polygon[index];
        let next = polygon[(index + 1) % count];
        let a = normal(previous, current);
        let b = normal(current, next);
        let sum = [a[0] + b[0], a[1] + b[1]];
        let denominator = sum[0] * b[0] + sum[1] * b[1];
        if denominator <= 1e-4 {
            b
        } else {
            let scale = (FADE_PX / denominator).min(2.0);
            [sum[0] * scale, sum[1] * scale]
        }
    };
    for index in 0..count {
        let p0 = polygon[index];
        let p1 = polygon[(index + 1) % count];
        if (p1[0] - p0[0]).hypot(p1[1] - p0[1]) <= 1e-6 {
            continue;
        }
        let o0 = offset(index);
        let o1 = offset((index + 1) % count);
        let q0 = [p0[0] + o0[0], p0[1] + o0[1]];
        let q1 = [p1[0] + o1[0], p1[1] + o1[1]];
        let inner_st = stroke_distance_st(0.0);
        let outer_st = stroke_distance_st(FADE_PX);
        push_tri_st(pool, p0, inner_st, q0, outer_st, q1, outer_st);
        push_tri_st(pool, p0, inner_st, q1, outer_st, p1, inner_st);
    }
}

/// Keep the shared inside-ring triangles intact and fade both exposed boundaries. The fringes
/// follow the shared outer and inner contours, which are the ring's own edge vertices.
pub(crate) fn round_rect_ring_mesh(
    pool: &mut Vec<MeshVertex>,
    geometry: &RoundRectBorder,
) -> (u32, u32) {
    let first = pool.len() as u32;
    push_vertices(pool, geometry.ring.iter().copied());
    push_polygon_fringe(pool, &geometry.outer);
    if !geometry.inner.is_empty() {
        push_polygon_fringe(pool, &geometry.inner);
    }
    (first, pool.len() as u32 - first)
}

/// Coverage fade width in device px: the band outside every nominal edge whose alpha ramps
/// linearly to zero, matching the 1 px ramp GPUI's path shader applies (`alpha = saturate(0.5 -
/// distance)`).
const FADE_PX: f32 = 1.0;

/// Polyline coverage is centered on the nominal stroke edge. Keeping `s` constant is required by
/// GPUI's Windows path shader: that backend takes any triangle with a varying `s` derivative down
/// its solid branch. With `s = 0` and `t = -distance`, `s² - t` is the signed device-pixel
/// distance directly on every platform.
const STROKE_AA_HALF_PX: f32 = 0.5;

const fn stroke_distance_st(distance: f32) -> [f32; 2] {
    [0.0, -distance]
}

/// Reusable tessellation buffers, owned by the renderer and cleared (never freed) per prim.
///
/// Tessellation writes into caller-provided `Vec`s. Allocating those fresh per prim made scene
/// construction allocation-bound: a single dense polyline grows a large expansion buffer from
/// empty every frame, and doubling reallocations copy its contents. Retaining these buffers avoids
/// that repeated allocation while keeping the produced geometry unchanged.
///
/// Buffers are separate fields rather than one arena so the tessellators can borrow the input and
/// output halves disjointly.
#[derive(Default)]
pub struct Scratch {
    /// The point window sliced out of the frame's shared pool.
    points: Vec<LinePoint>,
    /// Curve expansion of the window, reused across prims.
    expanded: Vec<LinePoint>,
    /// Coupled curve expansions for band boundaries.
    band_upper: Vec<LinePoint>,
    band_lower: Vec<LinePoint>,
    /// Area tessellation output.
    area: AreaMesh,
    /// `[f32; 2]` staging for polygons, discs, rings and band fills.
    verts: Vec<[f32; 2]>,
    /// Exterior contour of a fill, reused for its coverage fringe.
    contour: Vec<[f32; 2]>,
    /// Mesh ranges produced by a dashed polyline, read back by the executor.
    pub(crate) ranges: Vec<(u32, u32)>,
}

impl Scratch {
    /// Approximate retained bytes, so a host can assert the scratch stays bounded across a replay.
    pub fn capacity_bytes(&self) -> usize {
        self.points.capacity() * std::mem::size_of::<LinePoint>()
            + self.expanded.capacity() * std::mem::size_of::<LinePoint>()
            + self.band_upper.capacity() * std::mem::size_of::<LinePoint>()
            + self.band_lower.capacity() * std::mem::size_of::<LinePoint>()
            + self.area.vertices.capacity()
                * std::mem::size_of::<aeris_charts_render::line::LineVertex>()
            + self.area.crossings.capacity() * std::mem::size_of::<usize>()
            + self.verts.capacity() * std::mem::size_of::<[f32; 2]>()
            + self.contour.capacity() * std::mem::size_of::<[f32; 2]>()
            + self.ranges.capacity() * std::mem::size_of::<(u32, u32)>()
    }
}

/// Copy a `[first, first+count)` window of the shared pool into `out`, reusing its allocation.
fn slice_into(out: &mut Vec<LinePoint>, points: &[[f32; 2]], first: u32, count: u32) {
    out.clear();
    let a = first as usize;
    let b = a.saturating_add(count as usize);
    out.extend(points.get(a..b).unwrap_or(&[]).iter().map(|p| LinePoint {
        x: p[0] as f64,
        y: p[1] as f64,
    }));
}

/// Tessellate a solid polyline stroke into `pool` with a 1 px coverage transition centered on every
/// nominal edge. Returns a zero-length range for a run with fewer than two points, which the caller
/// reports as a dropped prim.
pub(crate) fn polyline_mesh(
    scratch: &mut Scratch,
    pool: &mut Vec<MeshVertex>,
    points: &[[f32; 2]],
    first: u32,
    count: u32,
    width: f32,
    line_type: LineType,
) -> (u32, u32) {
    let Scratch {
        points: window,
        expanded,
        ..
    } = scratch;
    slice_into(window, points, first, count);
    if window.len() < 2 {
        return (pool.len() as u32, 0);
    }
    // The pool already carries the DPR, so expansion adapts to device px directly.
    expand_line_into(window, line_type, 1.0, 1.0, expanded);
    stroke_aa_into(pool, expanded, width)
}

/// Tessellate a [`Prim::Segments`] batch into one contiguous mesh: each pair goes through the same
/// [`stroke_aa`] stroker as a solid two-point polyline, so the triangles equal the per-pair
/// `polyline_mesh` ones and the caller's single `push_mesh` applies the chunk bound to the whole
/// batch. Returns a zero-length range when the pair window leaves the pool, which the caller
/// reports as a dropped prim.
pub(crate) fn segments_mesh(
    pool: &mut Vec<MeshVertex>,
    points: &[[f32; 2]],
    first: u32,
    segment_count: u32,
    width: f32,
) -> (u32, u32) {
    let start = pool.len() as u32;
    if let Some(pairs) = segment_points(points, first, segment_count) {
        let point = |p: [f32; 2]| LinePoint {
            x: p[0] as f64,
            y: p[1] as f64,
        };
        for &[from, to] in pairs.as_chunks::<2>().0 {
            stroke_aa_into(pool, &[point(from), point(to)], width);
        }
    }
    (start, pool.len() as u32 - start)
}

/// Anti-aliased polyline stroke through the shared [`stroke_aa`] tessellator. GPUI encodes each
/// vertex's signed edge distance in the path shader's `st` channel; the solid core maps to
/// [`SOLID_ST`] (`stroke_distance_st(-1)`), so geometry and coverage match the WebGPU executor.
fn stroke_aa_into(pool: &mut Vec<MeshVertex>, pts: &[LinePoint], width: f32) -> (u32, u32) {
    let first = pool.len() as u32;
    stroke_aa(pts, width, |tri| {
        pool.extend(tri.map(|vertex| MeshVertex {
            x: vertex.position[0],
            y: vertex.position[1],
            st: if vertex.distance == STROKE_AA_SOLID {
                SOLID_ST
            } else {
                stroke_distance_st(vertex.distance)
            },
        }));
    });
    (first, pool.len() as u32 - first)
}

/// Tessellate a dashed polyline into one mesh per solid dash run, recording the ranges in
/// [`Scratch::ranges`] for the caller to emit in order.
#[allow(clippy::too_many_arguments)] // the parameters are the prim's fields plus the dash pattern
pub(crate) fn dashed_polyline_meshes(
    scratch: &mut Scratch,
    pool: &mut Vec<MeshVertex>,
    points: &[[f32; 2]],
    first: u32,
    count: u32,
    width: f32,
    line_type: LineType,
    pattern: &[f32],
) {
    scratch.ranges.clear();
    slice_into(&mut scratch.points, points, first, count);
    if scratch.points.len() < 2 {
        return;
    }
    expand_line_into(&scratch.points, line_type, 1.0, 1.0, &mut scratch.expanded);
    // `dash_split` returns owned Vecs. Dashed polylines are the rare route (the engine pre-splits
    // the series it owns into solid runs), so they keep the simple form.
    let pattern: Vec<f64> = pattern.iter().map(|&len| len as f64).collect();
    for run in aeris_charts_render::line::dash_split(&scratch.expanded, &pattern) {
        let range = stroke_aa_into(pool, &run, width);
        if range.1 >= 3 {
            scratch.ranges.push(range);
        }
    }
}

/// Tessellate an area fill into `pool`, returning the mesh range and the bounds-relative gradient
/// that reproduces `build_area_fill`'s per-vertex shading exactly.
///
/// Aeris shades each vertex with `t = clamp((y - top) / span, 0, 1)` where
/// `span = max(bottom - top, 1.0)`. When the geometry is at least 1 px tall, `span` equals the
/// mesh's own height and the shading is precisely a bounds-relative 0→1 ramp. When it is shorter,
/// Aeris's ramp is *wider* than the geometry, so the bottom stop is never reached; we compress it
/// by substituting the color Aeris would actually have produced at the mesh's bottom edge. Both
/// cases are exact, so no fidelity is traded for the single-`Background` GPUI `Path`.
#[allow(clippy::too_many_arguments)] // the parameters are the prim's own fields
pub(crate) fn area_fill_mesh(
    scratch: &mut Scratch,
    pool: &mut Vec<MeshVertex>,
    points: &[[f32; 2]],
    first: u32,
    count: u32,
    base_y: f32,
    line_type: LineType,
    top: Color,
    bottom: Color,
) -> ((u32, u32), Paint) {
    let Scratch {
        points: window,
        area,
        contour,
        ..
    } = scratch;
    slice_into(window, points, first, count);
    if window.len() < 2 {
        return ((pool.len() as u32, 0), Paint::VGradient { top, bottom });
    }
    area.vertices.clear();
    area.crossings.clear();
    build_area_fill(
        window,
        base_y as f64,
        top,
        bottom,
        &identity_params(0.0, line_type),
        area,
    );
    let core = push_vertices(pool, area.vertices.iter().map(|v| [v.x, v.y]));
    let paint = area_gradient(pool, core, top, bottom);
    // Each base crossing splits the fill into two simple lobes. Trace their exteriors
    // independently: a fringe around a self-intersecting bow-tie would paint the empty wedge.
    // The shared tessellator reports which segments cross, so lobe splits never depend on
    // comparing vertex coordinates.
    let segments = area.vertices.as_chunks::<6>().0;
    let mut crossings = area.crossings.iter().map(|&offset| offset / 6).peekable();
    if let Some(first_segment) = segments.first() {
        contour.clear();
        contour.push([first_segment[0].x, first_segment[0].y]);
        let mut start_x = first_segment[0].x;
        let finish_lobe =
            |contour: &mut Vec<[f32; 2]>, start_x: f32, pool: &mut Vec<MeshVertex>| {
                if let Some(&end) = contour.last() {
                    if end[1] != base_y {
                        contour.push([end[0], base_y]);
                    }
                    if contour.first().is_some_and(|first| first[1] != base_y) {
                        contour.push([start_x, base_y]);
                    }
                    push_polygon_fringe(pool, contour);
                }
            };
        for (index, segment) in segments.iter().enumerate() {
            let next = [segment[1].x, segment[1].y];
            contour.push(next);
            if crossings.next_if_eq(&index).is_some() {
                finish_lobe(contour, start_x, pool);
                contour.clear();
                contour.push(next);
                contour.push([segment[4].x, segment[4].y]);
                start_x = next[0];
            }
        }
        finish_lobe(contour, start_x, pool);
    }
    (core, paint)
}

/// Rescale an area fill's gradient stops onto the mesh's own bounds (see [`area_fill_mesh`]).
fn area_gradient(
    pool: &[MeshVertex],
    (first, count): (u32, u32),
    top: Color,
    bottom: Color,
) -> Paint {
    let verts = pool
        .get(first as usize..(first as usize).saturating_add(count as usize))
        .unwrap_or(&[]);
    if verts.is_empty() {
        return Paint::VGradient { top, bottom };
    }
    let mut y0 = f32::INFINITY;
    let mut y1 = f32::NEG_INFINITY;
    for v in verts {
        y0 = y0.min(v.y);
        y1 = y1.max(v.y);
    }
    let height = (y1 - y0) as f64;
    let span = height.max(1.0);
    // How far along Aeris's ramp the mesh's bottom edge actually sits.
    let end_t = (height / span).clamp(0.0, 1.0) as f32;
    Paint::VGradient {
        top,
        bottom: lerp_color(top, bottom, end_t),
    }
}

/// A fringe is painted as a separate GPUI path so its expanded bounds cannot restretch the
/// core area's exact gradient. Adjust its stops to sample the same vertical ramp in device space.
pub(crate) fn area_fringe_gradient(
    pool: &[MeshVertex],
    core: (u32, u32),
    fringe: (u32, u32),
    paint: Paint,
) -> Paint {
    let Paint::VGradient { top, bottom } = paint else {
        return paint;
    };
    let bounds = |(first, count): (u32, u32)| {
        pool.get(first as usize..(first + count) as usize)
            .map(|vertices| {
                vertices
                    .iter()
                    .fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), v| {
                        (lo.min(v.y), hi.max(v.y))
                    })
            })
    };
    let (Some((core_top, core_bottom)), Some((fringe_top, fringe_bottom))) =
        (bounds(core), bounds(fringe))
    else {
        return paint;
    };
    let span = core_bottom - core_top;
    if span <= 0.0 {
        return paint;
    }
    let sample = |y: f32| {
        let t = (y - core_top) / span;
        let channel = |a: u8, b: u8| {
            (a as f32 + (b as f32 - a as f32) * t)
                .round()
                .clamp(0.0, 255.0) as u8
        };
        Color::rgba(
            channel(top.r(), bottom.r()),
            channel(top.g(), bottom.g()),
            channel(top.b(), bottom.b()),
            channel(top.a(), bottom.a()),
        )
    };
    Paint::VGradient {
        top: sample(fringe_top),
        bottom: sample(fringe_bottom),
    }
}

/// Channel-wise linear interpolation, rounding the way `build_area_fill`'s f32 shading does once
/// the GPU quantizes it back to 8 bits.
pub(crate) fn lerp_color(a: Color, b: Color, t: f32) -> Color {
    if t >= 1.0 {
        return b;
    }
    if t <= 0.0 {
        return a;
    }
    let ch = |x: u8, y: u8| {
        (x as f32 + (y as f32 - x as f32) * t)
            .round()
            .clamp(0.0, 255.0) as u8
    };
    Color::rgba(
        ch(a.r(), b.r()),
        ch(a.g(), b.g()),
        ch(a.b(), b.b()),
        ch(a.a(), b.a()),
    )
}

/// Tessellate a filled disc into `pool` with the shared `circle_segments` density for its outer
/// rim. The one-pixel coverage transition straddles the nominal radius and keeps `s` constant for
/// GPUI on Windows.
pub(crate) fn disc_mesh(pool: &mut Vec<MeshVertex>, cx: f32, cy: f32, radius: f32) -> (u32, u32) {
    let first = pool.len() as u32;
    let core = (radius - STROKE_AA_HALF_PX).max(0.0);
    let rim = radius + STROKE_AA_HALF_PX;
    let segments = aeris_charts_render::line::circle_segments(rim);
    let core_st = stroke_distance_st(core - radius);
    let rim_st = stroke_distance_st(STROKE_AA_HALF_PX);
    for i in 0..segments {
        let a0 = i as f32 / segments as f32 * std::f32::consts::TAU;
        let a1 = (i + 1) as f32 / segments as f32 * std::f32::consts::TAU;
        let (c0, s0) = (a0.cos(), a0.sin());
        let (c1, s1) = (a1.cos(), a1.sin());
        let p0 = [cx + core * c0, cy + core * s0];
        let p1 = [cx + core * c1, cy + core * s1];
        push_tri_st(pool, [cx, cy], SOLID_ST, p0, SOLID_ST, p1, SOLID_ST);
        let o0 = [cx + rim * c0, cy + rim * s0];
        let o1 = [cx + rim * c1, cy + rim * s1];
        push_tri_st(pool, p0, core_st, o0, rim_st, o1, rim_st);
        push_tri_st(pool, p0, core_st, o1, rim_st, p1, core_st);
    }
    (first, pool.len() as u32 - first)
}

/// One annulus between radii `r0` (inner row, `st0`) and `r1` (outer row, `st1`). Equal `st` on
/// both rows gives a constant `s` gradient of zero, so the shader's solid branch fills the band
/// at full coverage.
fn annulus_st(
    pool: &mut Vec<MeshVertex>,
    center: [f32; 2],
    r0: f32,
    st0: [f32; 2],
    r1: f32,
    st1: [f32; 2],
    segments: usize,
) {
    let [cx, cy] = center;
    if r1 - r0 <= 0.0 {
        return;
    }
    for i in 0..segments {
        let a0 = i as f32 / segments as f32 * std::f32::consts::TAU;
        let a1 = (i + 1) as f32 / segments as f32 * std::f32::consts::TAU;
        let (c0, s0) = (a0.cos(), a0.sin());
        let (c1, s1) = (a1.cos(), a1.sin());
        let o0 = [cx + r1 * c0, cy + r1 * s0];
        let o1 = [cx + r1 * c1, cy + r1 * s1];
        let i0 = [cx + r0 * c0, cy + r0 * s0];
        let i1 = [cx + r0 * c1, cy + r0 * s1];
        push_tri_st(pool, o0, st1, o1, st1, i1, st0);
        push_tri_st(pool, o0, st1, i1, st0, i0, st0);
    }
}

/// A centered circle stroke with the same signed-distance coverage as polylines.
/// Keep `s` constant: varying it selects solid triangles in GPUI's Windows shader.
pub(crate) fn ring_mesh(
    pool: &mut Vec<MeshVertex>,
    cx: f32,
    cy: f32,
    radius: f32,
    stroke_width: f32,
) -> (u32, u32) {
    let first = pool.len() as u32;
    let outer = radius + stroke_width / 2.0;
    let inner = (radius - stroke_width / 2.0).max(0.0);
    if outer <= 0.0 || stroke_width <= 0.0 {
        return (first, 0);
    }
    let segments = aeris_charts_render::line::circle_segments(outer + STROKE_AA_HALF_PX);
    let middle = (inner + outer) / 2.0;
    let core_outer = (outer - STROKE_AA_HALF_PX).max(middle);
    let core_inner = (inner + STROKE_AA_HALF_PX).min(middle);
    annulus_st(
        pool,
        [cx, cy],
        core_outer,
        stroke_distance_st(core_outer - outer),
        outer + STROKE_AA_HALF_PX,
        stroke_distance_st(STROKE_AA_HALF_PX),
        segments,
    );
    if inner == 0.0 {
        annulus_st(
            pool,
            [cx, cy],
            0.0,
            SOLID_ST,
            core_outer,
            SOLID_ST,
            segments,
        );
    } else {
        annulus_st(
            pool,
            [cx, cy],
            core_inner,
            SOLID_ST,
            core_outer,
            SOLID_ST,
            segments,
        );
        let hole = (inner - STROKE_AA_HALF_PX).max(0.0);
        annulus_st(
            pool,
            [cx, cy],
            hole,
            stroke_distance_st(inner - hole),
            core_inner,
            stroke_distance_st(inner - core_inner),
            segments,
        );
    }
    (first, pool.len() as u32 - first)
}

/// Fan-triangulate a closed convex-ish polygon around its centroid, as the wgpu executor's
/// `fill_polygon` does, so rounded corners land on the same triangles.
pub(crate) fn fill_polygon(pool: &mut Vec<MeshVertex>, poly: &[[f32; 2]]) -> (u32, u32) {
    if poly.len() < 3 {
        return (pool.len() as u32, 0);
    }
    let n = poly.len() as f32;
    let center = [
        poly.iter().map(|p| p[0]).sum::<f32>() / n,
        poly.iter().map(|p| p[1]).sum::<f32>() / n,
    ];
    let mut verts = Vec::with_capacity(poly.len() * 3);
    for pair in poly.windows(2) {
        verts.extend([center, pair[0], pair[1]]);
    }
    verts.extend([center, poly[poly.len() - 1], poly[0]]);
    let (first, _) = push_vertices(pool, verts);
    push_polygon_fringe(pool, poly);
    (first, pool.len() as u32 - first)
}

/// A band fill between two polylines over the same x sequence: two triangles per segment, in the
/// same vertex order as the wgpu executor.
///
/// Reads both edges straight out of the frame's shared pool — a band over a dense visible range is
/// as large as a polyline, so it uses the reusable staging buffer rather than allocating.
pub(crate) fn band_fill_mesh(
    scratch: &mut Scratch,
    pool: &mut Vec<MeshVertex>,
    points: &[[f32; 2]],
    upper_first: u32,
    lower_first: u32,
    count: u32,
    line_type: LineType,
) -> (u32, u32) {
    let at = |first: u32, i: u32| -> Option<[f32; 2]> {
        points.get(first as usize + i as usize).copied()
    };
    if count < 2 || at(upper_first, count - 1).is_none() || at(lower_first, count - 1).is_none() {
        return (pool.len() as u32, 0);
    }
    scratch.points.clear();
    scratch.expanded.clear();
    for i in 0..count {
        let (Some(upper), Some(lower)) = (at(upper_first, i), at(lower_first, i)) else {
            break;
        };
        scratch.points.push(LinePoint {
            x: f64::from(upper[0]),
            y: f64::from(upper[1]),
        });
        scratch.expanded.push(LinePoint {
            x: f64::from(lower[0]),
            y: f64::from(lower[1]),
        });
    }
    aeris_charts_render::line::expand_band_into(
        &scratch.points,
        &scratch.expanded,
        line_type,
        1.0,
        1.0,
        &mut scratch.band_upper,
        &mut scratch.band_lower,
    );
    let expanded_count = scratch.band_upper.len().min(scratch.band_lower.len());
    if expanded_count < 2 {
        return (pool.len() as u32, 0);
    }
    scratch.verts.clear();
    for i in 0..expanded_count - 1 {
        let upper = scratch.band_upper[i];
        let next_upper = scratch.band_upper[i + 1];
        let lower = scratch.band_lower[i];
        let next_lower = scratch.band_lower[i + 1];
        let u0 = [upper.x as f32, upper.y as f32];
        let u1 = [next_upper.x as f32, next_upper.y as f32];
        let l0 = [lower.x as f32, lower.y as f32];
        let l1 = [next_lower.x as f32, next_lower.y as f32];
        scratch
            .verts
            .extend(aeris_charts_render::line::band_segment_triangles(
                u0, u1, l0, l1,
            ));
    }
    let (first, _) = push_vertices(pool, scratch.verts.iter().copied());
    scratch.verts.clear();
    scratch.contour.clear();
    let point = |p: &LinePoint| [p.x as f32, p.y as f32];
    scratch.verts.push(point(&scratch.band_upper[0]));
    scratch.contour.push(point(&scratch.band_lower[0]));
    let finish_lobe =
        |upper: &mut Vec<[f32; 2]>, lower: &[[f32; 2]], pool: &mut Vec<MeshVertex>| {
            let skip_end = usize::from(upper.last() == lower.last());
            for p in lower.iter().rev().skip(skip_end) {
                if Some(p) != upper.first() {
                    upper.push(*p);
                }
            }
            push_polygon_fringe(pool, upper);
        };
    for i in 0..expanded_count - 1 {
        let (u0, u1) = (
            point(&scratch.band_upper[i]),
            point(&scratch.band_upper[i + 1]),
        );
        let (l0, l1) = (
            point(&scratch.band_lower[i]),
            point(&scratch.band_lower[i + 1]),
        );
        if let Some(crossing) = aeris_charts_render::line::band_crossing(u0, u1, l0, l1) {
            scratch.verts.push(crossing);
            scratch.contour.push(crossing);
            finish_lobe(&mut scratch.verts, &scratch.contour, pool);
            scratch.verts.clear();
            scratch.contour.clear();
            scratch.verts.push(crossing);
            scratch.contour.push(crossing);
        }
        scratch.verts.push(u1);
        scratch.contour.push(l1);
    }
    finish_lobe(&mut scratch.verts, &scratch.contour, pool);
    (first, pool.len() as u32 - first)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::SOLID_ST;

    #[test]
    fn crossed_area_and_band_keep_their_solid_cores_in_the_exact_lobes() {
        let mut area = Vec::new();
        let (core, _) = area_fill_mesh(
            &mut Scratch::default(),
            &mut area,
            &[[0.0, 0.0], [10.0, 10.0]],
            0,
            2,
            5.0,
            LineType::Simple,
            Color::rgb(1, 2, 3),
            Color::rgb(1, 2, 3),
        );
        let mut band = Vec::new();
        let (first, band_count) = band_fill_mesh(
            &mut Scratch::default(),
            &mut band,
            &[[0.0, 0.0], [10.0, 10.0], [0.0, 5.0], [10.0, 5.0]],
            0,
            2,
            2,
            LineType::Simple,
        );
        for (name, vertices, first, count) in
            [("area", &area, core.0, core.1), ("band", &band, first, 6)]
        {
            assert!(
                covered(vertices, first, count, 3.0, 4.0),
                "{name} left lobe"
            );
            assert!(
                covered(vertices, first, count, 7.0, 6.0),
                "{name} right lobe"
            );
            assert!(
                !covered(vertices, first, count, 3.0, 6.0),
                "{name} left void"
            );
            assert!(
                !covered(vertices, first, count, 7.0, 4.0),
                "{name} right void"
            );
        }
        assert!(!covered(&area, core.0, area.len() as u32, 7.0, 3.5));
        assert!(!covered(&band, first, band_count, 7.0, 3.5));
    }

    #[test]
    fn crossed_area_fringes_each_lobe_separately() {
        let mut pool = Vec::new();
        // A base that is not exactly representable, crossed twice.
        let base = 0.1f32 + 0.2;
        let (core, _) = area_fill_mesh(
            &mut Scratch::default(),
            &mut pool,
            &[[0.0, -9.7], [10.0, 10.3], [20.0, -9.7]],
            0,
            3,
            base,
            LineType::Simple,
            Color::rgb(1, 2, 3),
            Color::rgb(1, 2, 3),
        );
        let fringe = (core.0 + core.1, pool.len() as u32 - core.0 - core.1);
        // Three triangular lobes, each traced alone: three edges of two triangles each.
        assert_eq!(fringe.1, 3 * 3 * 6, "one fringe per lobe");
        let probes = [
            // Just outside an exposed edge of each lobe.
            ([-0.3, -4.0], true),
            ([2.5, base + 0.3], true),
            ([7.5, 5.6], true),
            ([10.0, base - 0.3], true),
            ([12.5, 5.6], true),
            ([17.5, base + 0.3], true),
            ([20.3, -4.0], true),
            // The fringe fades outward, never across a lobe's own interior.
            ([10.0, 5.0], false),
            // The empty wedges between the lobes stay empty.
            ([5.0, 4.0], false),
            ([15.0, 4.0], false),
        ];
        for (point, expected) in probes {
            assert_eq!(
                covered(&pool, fringe.0, fringe.1, point[0], point[1]),
                expected,
                "fringe at {point:?}"
            );
        }
    }

    #[test]
    fn polygon_fringe_fades_one_pixel_outside_each_edge() {
        let mut pool = Vec::new();
        let (first, count) = fill_polygon(
            &mut pool,
            &[[10.0, 10.0], [30.0, 10.0], [30.0, 30.0], [10.0, 30.0]],
        );
        let vertices = &pool[first as usize..(first + count) as usize];
        assert!(
            vertices
                .iter()
                .any(|v| v.y == 10.0 && v.st == stroke_distance_st(0.0))
        );
        assert!(
            vertices
                .iter()
                .any(|v| v.y == 9.0 && v.st == stroke_distance_st(1.0))
        );
        assert!(
            vertices
                .iter()
                .any(|v| v.x == 31.0 && v.st == stroke_distance_st(1.0))
        );
        assert!(
            vertices
                .iter()
                .any(|v| v.y == 31.0 && v.st == stroke_distance_st(1.0))
        );
        assert!(
            vertices
                .iter()
                .any(|v| v.x == 9.0 && v.st == stroke_distance_st(1.0))
        );
    }

    #[test]
    fn disc_mesh_integrated_coverage_matches_nominal_area() {
        for radius in [5.0_f32, 10.0, 20.0] {
            let mut pool = Vec::new();
            let (first, count) = disc_mesh(&mut pool, 0.0, 0.0, radius);
            let mut covered_area = 0.0;
            for triangle in pool[first as usize..(first + count) as usize]
                .as_chunks::<3>()
                .0
            {
                let [a, b, c] = triangle;
                let area = ((b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)).abs() * 0.5;
                // The pinned GPUI Windows path shader takes a varying `s` down its solid branch.
                // For constant `s`, its one-pixel signed-distance ramp is linear over this strip.
                let coverage = if a.st[0] != b.st[0]
                    || b.st[0] != c.st[0]
                    || (a.st == SOLID_ST && b.st == SOLID_ST && c.st == SOLID_ST)
                {
                    1.0
                } else {
                    [a, b, c]
                        .iter()
                        .map(|v| (0.5 + v.st[1]).clamp(0.0, 1.0))
                        .sum::<f32>()
                        / 3.0
                };
                covered_area += area * coverage;
            }
            let target = std::f32::consts::PI * radius * radius;
            assert!(
                (covered_area - target).abs() / target < 0.02,
                "radius {radius}: coverage area {covered_area} vs {target}"
            );
        }
    }

    /// Point-in-mesh test over the triangle coverage (ignoring `st`), for coverage-gap checks.
    fn covered(pool: &[MeshVertex], first: u32, count: u32, qx: f32, qy: f32) -> bool {
        pool[first as usize..(first + count) as usize]
            .as_chunks::<3>()
            .0
            .iter()
            .any(|tri| {
                let (a, b, c) = (&tri[0], &tri[1], &tri[2]);
                let d1 = (qx - b.x) * (a.y - b.y) - (a.x - b.x) * (qy - b.y);
                let d2 = (qx - c.x) * (b.y - c.y) - (b.x - c.x) * (qy - c.y);
                let d3 = (qx - a.x) * (c.y - a.y) - (c.x - a.x) * (qy - a.y);
                let neg = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
                let pos = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
                !(neg && pos)
            })
    }

    /// The mesh must cover the full nominal stroke (no gaps or notches, e.g. at joins) and stay
    /// within the fade band (no overreach). Regresses the brush's staircase texture.
    #[test]
    fn curved_stroke_mesh_covers_the_nominal_band_without_gaps() {
        let knots = [
            [20.0f32, 42.0],
            [92.0, 18.0],
            [176.0, 62.0],
            [270.0, 24.0],
            [360.0, 66.0],
            [460.0, 36.0],
        ];
        let mut pool = Vec::new();
        let (first, count) = polyline_mesh(
            &mut Scratch::default(),
            &mut pool,
            &knots,
            0,
            knots.len() as u32,
            6.0,
            LineType::Curved,
        );
        // Walk the expanded centerline and probe perpendicular to it.
        let window: Vec<LinePoint> = knots
            .iter()
            .map(|p| LinePoint {
                x: p[0] as f64,
                y: p[1] as f64,
            })
            .collect();
        let mut expanded = Vec::new();
        expand_line_into(&window, LineType::Curved, 1.0, 1.0, &mut expanded);
        let half = 3.0f32;
        for (index, pair) in expanded.windows(2).enumerate() {
            let (a, b) = (pair[0], pair[1]);
            let (dx, dy) = ((b.x - a.x) as f32, (b.y - a.y) as f32);
            let len = (dx * dx + dy * dy).sqrt();
            if len < 1e-6 {
                continue;
            }
            let (nx, ny) = (-dy / len, dx / len);
            for t in [0.25f32, 0.5, 0.75] {
                let (px, py) = (a.x as f32 + dx * t, a.y as f32 + dy * t);
                for side in [1.0f32, -1.0] {
                    // Just inside the nominal edge: always covered.
                    let (ix, iy) = (
                        px + nx * (half - 0.25) * side,
                        py + ny * (half - 0.25) * side,
                    );
                    assert!(
                        covered(&pool, first, count, ix, iy),
                        "gap inside the nominal band at ({ix}, {iy}), segment {index}, t={t}, side={side}, a={a:?}, b={b:?}"
                    );
                    // Past the fade band: never covered.
                    let (ox, oy) = (
                        px + nx * (half + STROKE_AA_HALF_PX + 0.25) * side,
                        py + ny * (half + STROKE_AA_HALF_PX + 0.25) * side,
                    );
                    assert!(
                        !covered(&pool, first, count, ox, oy),
                        "overreach past the fade band at ({ox}, {oy})"
                    );
                }
            }
        }
    }

    #[test]
    fn degenerate_integer_rects_are_dropped() {
        assert_eq!(
            irect(IRect {
                x: 1,
                y: 2,
                w: 0,
                h: 5
            }),
            None
        );
        assert_eq!(
            irect(IRect {
                x: 1,
                y: 2,
                w: 5,
                h: -1
            }),
            None
        );
        assert_eq!(
            irect(IRect {
                x: 1,
                y: 2,
                w: 3,
                h: 4
            }),
            Some(DeviceRect::new(1.0, 2.0, 3.0, 4.0))
        );
    }

    #[test]
    fn rect_frame_edges_match_the_existing_executors() {
        // Same expectation as `canvas2d::tests::rect_frame_expands_to_four_edges`.
        let edges = rect_frame_edges(
            IRect {
                x: 10,
                y: 20,
                w: 8,
                h: 6,
            },
            1,
        );
        assert_eq!(
            edges[0],
            IRect {
                x: 11,
                y: 20,
                w: 6,
                h: 1
            }
        );
        assert_eq!(
            edges[1],
            IRect {
                x: 11,
                y: 25,
                w: 6,
                h: 1
            }
        );
        assert_eq!(
            edges[2],
            IRect {
                x: 10,
                y: 20,
                w: 1,
                h: 6
            }
        );
        assert_eq!(
            edges[3],
            IRect {
                x: 17,
                y: 20,
                w: 1,
                h: 6
            }
        );
    }

    #[test]
    fn dashed_spans_match_the_existing_executors() {
        // Same expectation as `canvas2d::tests::large_dashed_vline_emits_on_segments_only`.
        let mut spans = Vec::new();
        dash_spans(LineStyle::Dashed, 1, 0, 24, |a, b| spans.push((a, b)));
        assert_eq!(spans, vec![(0, 6), (12, 18)]);

        let mut solid = Vec::new();
        dash_spans(LineStyle::Solid, 1, 3, 9, |a, b| solid.push((a, b)));
        assert_eq!(solid, vec![(3, 9)]);
    }

    #[test]
    fn odd_line_widths_center_on_the_coordinate() {
        assert_eq!(line_span_start(50, 1), 50);
        assert_eq!(line_span_start(50, 2), 49);
        assert_eq!(line_span_start(50, 3), 49);
        assert_eq!(line_span_start(50, 4), 48);
    }

    #[test]
    fn push_vertices_never_leaves_a_partial_triangle() {
        let mut pool = Vec::new();
        let (first, count) = push_vertices(&mut pool, [[0.0, 0.0], [1.0, 1.0]]);
        assert_eq!((first, count), (0, 0));
        assert!(pool.is_empty(), "the partial triangle was rolled back");

        let (first, count) =
            push_vertices(&mut pool, [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [2.0, 2.0]]);
        assert_eq!((first, count), (0, 3));
        assert_eq!(pool.len(), 3);
    }

    #[test]
    fn area_gradient_is_a_plain_ramp_when_the_mesh_is_tall_enough() {
        let top = Color::rgba(0x2e, 0xdc, 0x87, 102);
        let bottom = Color::rgba(0x28, 0xdd, 0x64, 0);
        let mut pool = Vec::new();
        let pts = [[0.0f32, 10.0], [20.0, 4.0]];
        let (_, paint) = area_fill_mesh(
            &mut Scratch::default(),
            &mut pool,
            &pts,
            0,
            2,
            40.0,
            LineType::Simple,
            top,
            bottom,
        );
        assert_eq!(paint, Paint::VGradient { top, bottom });
    }

    #[test]
    fn area_gradient_compresses_when_base_floors_its_span() {
        // A fill only 0.5 px tall: Aeris's `span` floors at 1.0, so its bottom stop is only half
        // reached. The equivalent bounds-relative ramp ends at the midpoint color.
        let top = Color::rgba(0, 0, 0, 0xff);
        let bottom = Color::rgba(0xff, 0xff, 0xff, 0xff);
        let mut pool = Vec::new();
        let pts = [[0.0f32, 10.0], [20.0, 10.0]];
        let (_, paint) = area_fill_mesh(
            &mut Scratch::default(),
            &mut pool,
            &pts,
            0,
            2,
            10.5,
            LineType::Simple,
            top,
            bottom,
        );
        let Paint::VGradient { top: t, bottom: b } = paint else {
            panic!("expected a gradient, got {paint:?}");
        };
        assert_eq!(t, top);
        assert_eq!(b, Color::rgba(0x80, 0x80, 0x80, 0xff));
    }

    #[test]
    fn area_mesh_bounds_match_the_canvas2d_gradient_extent() {
        // `canvas2d::area_extent` spans [min point y, base_y]; the mesh must have the same extent
        // or a bounds-relative gradient would shift.
        let mut pool = Vec::new();
        let pts = [[0.0f32, 10.0], [20.0, 4.0]];
        let ((first, count), _) = area_fill_mesh(
            &mut Scratch::default(),
            &mut pool,
            &pts,
            0,
            2,
            40.0,
            LineType::Simple,
            Color::rgb(0, 0, 0xff),
            Color::rgba(0, 0, 0xff, 0),
        );
        let verts = &pool[first as usize..(first + count) as usize];
        let solid = verts.iter().filter(|v| v.st == SOLID_ST);
        let (y0, y1) = solid.fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), v| {
            (lo.min(v.y), hi.max(v.y))
        });
        assert_eq!((y0, y1), (4.0, 40.0));
        assert!(
            pool[(first + count) as usize..]
                .iter()
                .any(|v| v.st != SOLID_ST)
        );
    }

    #[test]
    fn lerp_color_hits_both_endpoints_exactly() {
        let a = Color::rgba(0, 10, 20, 30);
        let b = Color::rgba(200, 210, 220, 230);
        assert_eq!(lerp_color(a, b, 0.0), a);
        assert_eq!(lerp_color(a, b, 1.0), b);
        assert_eq!(lerp_color(a, b, -5.0), a);
        assert_eq!(lerp_color(a, b, 5.0), b);
    }

    #[test]
    fn band_fill_emits_two_triangles_per_segment() {
        let mut pool = Vec::new();
        // Upper edge at rows 0..4, lower edge at rows 4..8 of one shared pool.
        let mut pts: Vec<[f32; 2]> = (0..4).map(|i| [i as f32, 0.0]).collect();
        pts.extend((0..4).map(|i| [i as f32, 5.0]));
        let (_, count) = band_fill_mesh(
            &mut Scratch::default(),
            &mut pool,
            &pts,
            0,
            4,
            4,
            LineType::Simple,
        );
        assert_eq!(
            pool[..count as usize]
                .iter()
                .filter(|v| v.st == SOLID_ST)
                .count(),
            3 * 6,
            "3 segments x 2 solid triangles x 3 vertices"
        );
        assert!(count > 3 * 6, "outer edges carry coverage triangles");
    }

    #[test]
    fn band_fill_needs_two_points_per_edge() {
        let mut pool = Vec::new();
        let one = [[0.0f32, 0.0]];
        assert_eq!(
            band_fill_mesh(
                &mut Scratch::default(),
                &mut pool,
                &one,
                0,
                0,
                1,
                LineType::Simple,
            )
            .1,
            0
        );
    }

    #[test]
    fn round_rect_radii_clamp_to_half_the_shorter_side() {
        let poly = round_rect_polygon(0.0, 0.0, 10.0, 4.0, [99.0, 99.0, 99.0, 99.0]);
        for p in &poly {
            assert!(p[0] >= -0.01 && p[0] <= 10.01, "x out of bounds: {p:?}");
            assert!(p[1] >= -0.01 && p[1] <= 4.01, "y out of bounds: {p:?}");
        }
    }

    #[test]
    fn ring_coverage_uses_windows_compatible_centered_distance() {
        let mut pool = Vec::new();
        ring_mesh(&mut pool, 0.0, 0.0, 10.0, 1.5);
        for vertex in &pool {
            assert_eq!(
                vertex.st[0], 0.0,
                "varying s becomes solid coverage on Windows"
            );
            let radius = vertex.x.hypot(vertex.y);
            assert!(
                (8.749..=11.251).contains(&radius),
                "AA must extend only half a pixel: {radius}"
            );
        }
    }

    #[test]
    fn ring_mesh_covers_the_stroke_band_with_a_coverage_fringe() {
        let mut pool = Vec::new();
        let (first, count) = ring_mesh(&mut pool, 0.0, 0.0, 10.0, 2.0);
        // Outer fade + solid middle + inner fade, 24 segments of 2 triangles each.
        assert_eq!(count, 3 * 24 * 6);
        // A Canvas2D `arc` + `stroke` of width 2 at radius 10 covers [9, 11]; the fade reaches
        // half a pixel past it on both sides, and nothing approaches the disc's interior.
        for v in &pool[first as usize..(first + count) as usize] {
            let r = (v.x * v.x + v.y * v.y).sqrt();
            assert!((8.49..=11.51).contains(&r), "radius {r} outside the band");
            assert!(r > 1.0, "the ring must not cover the disc's interior");
        }
        // The extreme rows sit half a pixel past the nominal edges, where coverage reaches zero.
        assert!(
            pool[first as usize..(first + count) as usize]
                .iter()
                .any(|v| v.st == stroke_distance_st(STROKE_AA_HALF_PX)),
            "the fringe rows carry the Loop-Blinn edge encoding"
        );
    }

    #[test]
    fn stroke_mesh_centers_coverage_on_nominal_edges() {
        let mut pool = Vec::new();
        // One horizontal 4 px segment at y = 10: nominal band [8, 12], with a one-pixel
        // coverage transition centered on each nominal edge.
        let pts = [[0.0f32, 10.0], [20.0, 10.0]];
        let (first, count) = polyline_mesh(
            &mut Scratch::default(),
            &mut pool,
            &pts,
            0,
            2,
            4.0,
            LineType::Simple,
        );
        let verts = &pool[first as usize..(first + count) as usize];
        assert!(
            verts.iter().any(|v| v.st != SOLID_ST),
            "edge vertices carry the coverage encoding, not the solid convention"
        );
        let y0 = verts.iter().map(|v| v.y).fold(f32::INFINITY, f32::min);
        let y1 = verts.iter().map(|v| v.y).fold(f32::NEG_INFINITY, f32::max);
        assert_eq!(
            (y0, y1),
            (7.5, 12.5),
            "coverage extends half a pixel past each edge"
        );
        // Keeping `s` constant avoids GPUI Windows' solid-triangle branch. The transition runs
        // from -0.5 to +0.5 signed pixels around the nominal edge and the inner core stays solid.
        assert!(verts
            .iter()
            .any(|v| (v.y - 12.5).abs() < 1e-4 && v.st == stroke_distance_st(STROKE_AA_HALF_PX)));
        assert!(
            verts.iter().any(
                |v| (v.y - 11.5).abs() < 1e-4 && v.st == stroke_distance_st(-STROKE_AA_HALF_PX)
            )
        );
        assert!(
            verts
                .iter()
                .any(|v| (v.y - 8.5).abs() < 1e-4 && v.st == SOLID_ST)
        );
        assert!(
            verts
                .iter()
                .filter(|v| v.st != SOLID_ST)
                .all(|v| v.st[0] == 0.0),
            "coverage triangles keep s constant so Windows does not force them opaque"
        );
    }

    #[test]
    fn out_of_range_windows_are_clamped_not_panics() {
        let pts = [[0.0f32, 1.0], [2.0, 3.0]];
        let mut out = Vec::new();
        slice_into(&mut out, &pts, 0, 2);
        assert_eq!(out.len(), 2);
        slice_into(&mut out, &pts, 5, 2);
        assert!(out.is_empty());
        slice_into(&mut out, &pts, 0, u32::MAX);
        assert!(out.is_empty());
    }

    #[test]
    fn a_polyline_window_out_of_range_yields_an_empty_mesh() {
        let mut pool = Vec::new();
        let pts = [[0.0f32, 0.0], [1.0, 1.0]];
        let (_, count) = polyline_mesh(
            &mut Scratch::default(),
            &mut pool,
            &pts,
            9,
            5,
            2.0,
            LineType::Simple,
        );
        assert_eq!(count, 0);
    }

    #[test]
    fn scratch_buffers_are_reused_across_calls_not_regrown() {
        // The point of `Scratch`: after the first frame the capacity is already there.
        let mut scratch = Scratch::default();
        let mut pool = Vec::new();
        let pts: Vec<[f32; 2]> = (0..512).map(|i| [i as f32, (i % 7) as f32]).collect();
        polyline_mesh(&mut scratch, &mut pool, &pts, 0, 512, 2.0, LineType::Simple);
        let after_first = scratch.capacity_bytes();
        assert!(after_first > 0);
        for _ in 0..20 {
            pool.clear();
            polyline_mesh(&mut scratch, &mut pool, &pts, 0, 512, 2.0, LineType::Simple);
        }
        assert_eq!(
            scratch.capacity_bytes(),
            after_first,
            "repeated identical frames must not grow the scratch"
        );
    }
}
