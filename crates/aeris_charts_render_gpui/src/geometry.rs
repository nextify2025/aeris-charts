//! Quad and triangle/path conversion for the GPUI executor.
//!
//! Every helper here reuses Aeris's own pixel math rather than recomputing it:
//! - the integer-rect subset copies the exact expansion the wgpu quad executor and the Canvas2D
//!   executor already agree on (`fillRectInnerBorder`, half-width line centering, dash phase);
//! - the anti-aliased subset reuses `aeris_charts_render::line`'s shared curve expansion and area
//!   tessellation, and extrudes strokes/discs/rings itself with a per-vertex Loop-Blinn coverage
//!   encoding ([`edge_st`]): GPUI's path pass cannot rely on MSAA (its sample count can fall back
//!   to 1x on Linux), so the same geometry the WebGPU backend's 4x MSAA target smooths carries its
//!   own coverage fade here. Polyline transitions straddle their nominal edges and keep integrated
//!   width exact; rings share that centered encoding, while filled discs retain their exterior encoding (see [`edge_st`]).
//!
//! Uses Aeris's coordinate, bar-width, and snapping calculations.

use aeris_charts_render::color::Color;
use aeris_charts_render::draw_list::{segment_points, IRect, LineStyle, LineType};
pub(crate) use aeris_charts_render::line::round_rect_polygon;
use aeris_charts_render::line::{
    build_area_fill, expand_line_into, stroke_aa, AreaMesh, LineParams, LinePoint, STROKE_AA_SOLID,
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

/// Coverage fade width in device px: the band outside every nominal edge whose alpha ramps
/// linearly to zero, matching the 1 px ramp GPUI's path shader applies (`alpha = saturate(0.5 -
/// distance)`).
const FADE_PX: f32 = 1.0;

/// `st` for a vertex at signed device-px distance `d` (positive outside) from the nearest
/// exterior edge. GPUI's path shader computes coverage from `f = s² - t` and its screen-space
/// gradient; choosing `s = d`, `t = d² - d` makes `f == d` at the vertex.
///
/// The encoding is only *exact* across a triangle when `t` interpolates linearly to the same
/// value `s²` would have — i.e. when the triangle spans `d ∈ [0, 1]`, where `t ≡ 0` at both ends
/// so `f = s²` is the exact quadratic and the shader's distance estimate `f / |∇f|` is `d / 2`,
/// a perfect linear ramp over the band. A triangle spanning a wider `d` range interpolates `t`
/// along the secant of the quadratic, collapsing the ramp toward a hard edge displaced outward —
/// so wide geometry must be solid ([`crate::scene::SOLID_ST`]) out to the nominal edge and only
/// the exterior 1 px band may carry this encoding.
const fn edge_st(d: f32) -> [f32; 2] {
    [d, d * d - d]
}

/// `st` at a nominal exterior edge: exact zero of the shader's coverage field.
const FADE_IN_ST: [f32; 2] = edge_st(0.0);
/// `st` one device px outside a nominal exterior edge: coverage reaches zero exactly here.
const FADE_OUT_ST: [f32; 2] = edge_st(FADE_PX);

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
            + self.verts.capacity() * std::mem::size_of::<[f32; 2]>()
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
        ..
    } = scratch;
    slice_into(window, points, first, count);
    if window.len() < 2 {
        return ((pool.len() as u32, 0), Paint::VGradient { top, bottom });
    }
    area.vertices.clear();
    build_area_fill(
        window,
        base_y as f64,
        top,
        bottom,
        &identity_params(0.0, line_type),
        area,
    );
    let range = push_vertices(pool, area.vertices.iter().map(|v| [v.x, v.y]));
    let paint = area_gradient(pool, range, top, bottom);
    (range, paint)
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

/// Tessellate a filled disc into `pool` (24 segments, matching `build_disc`): solid out to the
/// nominal radius, then the exact 1 px coverage fade outside it.
pub(crate) fn disc_mesh(pool: &mut Vec<MeshVertex>, cx: f32, cy: f32, radius: f32) -> (u32, u32) {
    const SEGMENTS: usize = 24;
    let first = pool.len() as u32;
    let rim = radius + FADE_PX;
    for i in 0..SEGMENTS {
        let a0 = i as f32 / SEGMENTS as f32 * std::f32::consts::TAU;
        let a1 = (i + 1) as f32 / SEGMENTS as f32 * std::f32::consts::TAU;
        let (c0, s0) = (a0.cos(), a0.sin());
        let (c1, s1) = (a1.cos(), a1.sin());
        let p0 = [cx + radius * c0, cy + radius * s0];
        let p1 = [cx + radius * c1, cy + radius * s1];
        push_tri_st(pool, [cx, cy], SOLID_ST, p0, SOLID_ST, p1, SOLID_ST);
        let o0 = [cx + rim * c0, cy + rim * s0];
        let o1 = [cx + rim * c1, cy + rim * s1];
        push_tri_st(pool, p0, FADE_IN_ST, o0, FADE_OUT_ST, o1, FADE_OUT_ST);
        push_tri_st(pool, p0, FADE_IN_ST, o1, FADE_OUT_ST, p1, FADE_IN_ST);
    }
    (first, pool.len() as u32 - first)
}

/// One annulus between radii `r0` (inner row, `st0`) and `r1` (outer row, `st1`). Equal `st` on
/// both rows gives a constant `s` gradient of zero, so the shader's solid branch fills the band
/// at full coverage.
fn annulus_st(
    pool: &mut Vec<MeshVertex>,
    cx: f32,
    cy: f32,
    r0: f32,
    st0: [f32; 2],
    r1: f32,
    st1: [f32; 2],
) {
    const SEGMENTS: usize = 24;
    if r1 - r0 <= 0.0 {
        return;
    }
    for i in 0..SEGMENTS {
        let a0 = i as f32 / SEGMENTS as f32 * std::f32::consts::TAU;
        let a1 = (i + 1) as f32 / SEGMENTS as f32 * std::f32::consts::TAU;
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
    let middle = (inner + outer) / 2.0;
    let core_outer = (outer - STROKE_AA_HALF_PX).max(middle);
    let core_inner = (inner + STROKE_AA_HALF_PX).min(middle);
    annulus_st(
        pool,
        cx,
        cy,
        core_outer,
        stroke_distance_st(core_outer - outer),
        outer + STROKE_AA_HALF_PX,
        stroke_distance_st(STROKE_AA_HALF_PX),
    );
    if inner == 0.0 {
        annulus_st(pool, cx, cy, 0.0, SOLID_ST, core_outer, SOLID_ST);
    } else {
        annulus_st(pool, cx, cy, core_inner, SOLID_ST, core_outer, SOLID_ST);
        let hole = (inner - STROKE_AA_HALF_PX).max(0.0);
        annulus_st(
            pool,
            cx,
            cy,
            hole,
            stroke_distance_st(inner - hole),
            core_inner,
            stroke_distance_st(inner - core_inner),
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
    push_vertices(pool, verts)
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
        scratch.verts.extend([u0, l0, l1, u0, l1, u1]);
    }
    push_vertices(pool, scratch.verts.iter().copied())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::SOLID_ST;

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
        for pair in expanded.windows(2) {
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
                        "gap inside the nominal band at ({ix}, {iy})"
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
        let y0 = verts.iter().map(|v| v.y).fold(f32::INFINITY, f32::min);
        let y1 = verts.iter().map(|v| v.y).fold(f32::NEG_INFINITY, f32::max);
        assert_eq!((y0, y1), (4.0, 40.0));
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
        assert_eq!(count, 3 * 6, "3 segments x 2 triangles x 3 vertices");
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
        assert!(verts
            .iter()
            .any(|v| (v.y - 11.5).abs() < 1e-4 && v.st == stroke_distance_st(-STROKE_AA_HALF_PX)));
        assert!(verts
            .iter()
            .any(|v| (v.y - 8.5).abs() < 1e-4 && v.st == SOLID_ST));
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
