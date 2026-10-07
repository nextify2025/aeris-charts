//! The backend-neutral GPUI *scene plan*: an ordered list of exactly the draw calls GPUI can
//! make, carrying no GPUI types.
//!
//! Why an intermediate representation instead of calling `gpui::Window` straight from the `Prim`
//! match arm: GPUI painting needs a live window, a platform renderer, and a GPU. Lowering into
//! this plan first means the whole interesting half of the adapter — paint order, clipping,
//! snapping, gradient extents, dash expansion, tessellation, text placement — is ordinary
//! deterministic data that unit tests can assert on, on any machine, with GPUI not even compiled.
//! [`crate::backend`] is then a thin, near-branchless walk of this plan.
//!
//! Coordinates are **device (bitmap) px**, matching the `Prim` IR (`aeris_charts_render::draw_list`),
//! which already has the DPR baked in by the engine. GPUI's `paint_*` entry points take *logical*
//! px and scale by the window's scale factor themselves, so [`crate::backend`] divides by the
//! scale factor at the boundary — see [`crate::AerisViewport`].

use aeris_charts_render::color::Color;
use aeris_charts_render::draw_list::RasterImage;

/// An axis-aligned rectangle in device px.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DeviceRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl DeviceRect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    /// Whether the rect covers no pixels. GPUI drops empty-bounds primitives itself
    /// (`Scene::insert_primitive`); dropping them here too keeps the metrics honest.
    ///
    /// A NaN extent counts as empty: it cannot be painted, and letting it through would put a NaN
    /// into GPUI's bounds tree, where it would corrupt the draw-order comparisons for the whole
    /// frame. `matches!(.., Some(Greater))` says that explicitly rather than relying on negated
    /// float comparisons.
    pub fn is_empty(&self) -> bool {
        use std::cmp::Ordering::Greater;
        !(matches!(self.w.partial_cmp(&0.0), Some(Greater))
            && matches!(self.h.partial_cmp(&0.0), Some(Greater)))
    }

    pub fn right(&self) -> f32 {
        self.x + self.w
    }

    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }

    /// Intersection, or `None` when the two rects do not overlap.
    pub fn intersect(&self, other: &DeviceRect) -> Option<DeviceRect> {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let r = self.right().min(other.right());
        let b = self.bottom().min(other.bottom());
        let out = DeviceRect::new(x, y, r - x, b - y);
        (!out.is_empty()).then_some(out)
    }
}

/// How a quad or mesh is filled.
///
/// `VGradient` is a two-stop top-to-bottom ramp over the primitive's **own bounds**. That is not a
/// simplification: GPUI gradients are bounds-relative, and every vertical gradient in the `Prim`
/// IR already spans exactly its primitive's extent — `Background`'s ramp spans its rect
/// (`canvas2d.rs`: `set_fill_vgradient(y, y + h, ..)`), and an `AreaFill`'s spans
/// `[y_top, y_bottom]`, which is precisely the bounding box of the traced path because the path
/// includes both the polyline and the `base_y` edge. [`crate::geometry`] rescales the stops for
/// the one case where Aeris's ramp is *wider* than the geometry (`build_area_fill` floors its
/// span at 1 px), so the mapping stays exact rather than approximate.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Paint {
    Solid(Color),
    VGradient { top: Color, bottom: Color },
}

impl Paint {
    /// Whether the paint contributes nothing (fully transparent).
    pub fn is_invisible(&self) -> bool {
        match self {
            Paint::Solid(c) => c.a() == 0,
            Paint::VGradient { top, bottom } => top.a() == 0 && bottom.a() == 0,
        }
    }
}

/// A text run to be shaped and painted.
///
/// Mirrors `Prim::Text` but resolves the anchor into a concrete left edge and baseline during
/// painting (see [`crate::text`]) — the plan keeps the semantic anchor so the shaping backend can
/// apply its own metrics.
#[derive(Clone, Debug, PartialEq)]
pub struct TextRun {
    /// Aligned edge x in device px (meaning set by `align`).
    pub x: f32,
    /// Vertical **center** of the run in device px (Canvas `textBaseline: "middle"`).
    pub y: f32,
    pub text: String,
    pub color: Color,
    /// Font size in device px.
    pub size: f32,
    /// Fully-resolved family list (layout defaults already folded in by the engine).
    pub family: String,
    pub align: aeris_charts_render::draw_list::TextAlign,
    /// Numeric CSS font weight (100–900).
    pub weight: u16,
    pub italic: bool,
    /// Clockwise radians around the aligned `(x, y)` anchor.
    pub angle: f32,
}

/// One GPUI-bound draw command. The plan's `Vec<SceneOp>` order **is** the paint order.
#[derive(Clone, Debug, PartialEq)]
pub enum SceneOp {
    /// Restrict subsequent ops to `rect` → `gpui::Window::with_content_mask`.
    PushClip(DeviceRect),
    /// Undo the innermost [`SceneOp::PushClip`].
    PopClip,
    /// Axis-aligned quad → `gpui::Window::paint_quad`.
    ///
    /// `corner_radii` is left-top, right-top, right-bottom, left-bottom (the `Prim::RoundRect`
    /// order). A non-zero `border_width` paints an inner border in `border_color`, matching
    /// `PaintQuad`'s border semantics.
    Quad {
        rect: DeviceRect,
        fill: Paint,
        corner_radii: [f32; 4],
        border_width: f32,
        border_color: Color,
    },
    /// A triangle list over [`ScenePlan::vertices`] → one `gpui::Path` built with
    /// `Path::push_triangle`, painted via `gpui::Window::paint_path`.
    ///
    /// `vertex_count` is always a multiple of 3.
    Mesh {
        first_vertex: u32,
        vertex_count: u32,
        fill: Paint,
    },
    /// A shaped-and-painted text run → see [`crate::text`].
    Text(TextRun),
    /// Shared-frame raster image → `gpui::Window::paint_image`.
    Image {
        image: RasterImage,
        rect: DeviceRect,
        opacity: f32,
    },
}

/// A mesh vertex in device px. Colors live on the owning [`SceneOp::Mesh`]'s [`Paint`], because a
/// GPUI `Path` carries a single `Background` rather than per-vertex colors.
///
/// `st` feeds the coverage term of GPUI's path shader (`f = s² - t`, `alpha = saturate(0.5 -
/// f / |∇f|)`). [`SOLID_ST`] marks a fully covered interior vertex (constant `s` → the shader's
/// zero-gradient solid branch). Edge vertices produced by `geometry`'s anti-aliased tessellators
/// instead carry a Loop-Blinn signed-distance encoding, which the shader turns into a 1 px edge
/// transition — the coverage WebGPU gets from its 4x MSAA target, which GPUI's path pass does not
/// guarantee (its sample count can fall back to 1x on Linux).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeshVertex {
    pub x: f32,
    pub y: f32,
    pub st: [f32; 2],
}

/// The `st` of a fully covered interior vertex (GPUI's "solid interior" convention).
pub const SOLID_ST: [f32; 2] = [0.0, 1.0];

impl MeshVertex {
    /// A fully covered interior vertex.
    pub const fn solid(x: f32, y: f32) -> Self {
        Self { x, y, st: SOLID_ST }
    }
}

impl Default for MeshVertex {
    fn default() -> Self {
        Self::solid(0.0, 0.0)
    }
}

/// One lowered frame: the ordered ops plus the shared vertex pool their meshes index.
///
/// Reused across frames via [`ScenePlan::clear`] so steady-state painting does not reallocate
/// for stable steady-state memory.
#[derive(Clone, Debug, Default)]
pub struct ScenePlan {
    pub ops: Vec<SceneOp>,
    pub vertices: Vec<MeshVertex>,
}

impl ScenePlan {
    /// Drop the contents but keep the allocations.
    pub fn clear(&mut self) {
        self.ops.clear();
        self.vertices.clear();
    }

    /// The vertices of one mesh op.
    pub fn mesh_vertices(&self, first_vertex: u32, vertex_count: u32) -> &[MeshVertex] {
        let a = first_vertex as usize;
        let b = a.saturating_add(vertex_count as usize);
        self.vertices.get(a..b).unwrap_or(&[])
    }

    /// Axis-aligned bounds of one mesh, or `None` when it has no vertices. Used to derive
    /// bounds-relative gradient stops and to assert clipping in tests.
    pub fn mesh_bounds(&self, first_vertex: u32, vertex_count: u32) -> Option<DeviceRect> {
        let verts = self.mesh_vertices(first_vertex, vertex_count);
        let first = verts.first()?;
        let (mut x0, mut y0, mut x1, mut y1) = (first.x, first.y, first.x, first.y);
        for v in &verts[1..] {
            x0 = x0.min(v.x);
            y0 = y0.min(v.y);
            x1 = x1.max(v.x);
            y1 = y1.max(v.y);
        }
        Some(DeviceRect::new(x0, y0, x1 - x0, y1 - y0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_rects_are_detected_including_nan() {
        assert!(DeviceRect::new(0.0, 0.0, 0.0, 5.0).is_empty());
        assert!(DeviceRect::new(0.0, 0.0, 5.0, -1.0).is_empty());
        assert!(DeviceRect::new(0.0, 0.0, f32::NAN, 5.0).is_empty());
        assert!(!DeviceRect::new(0.0, 0.0, 1.0, 1.0).is_empty());
    }

    #[test]
    fn intersect_returns_none_when_disjoint() {
        let a = DeviceRect::new(0.0, 0.0, 10.0, 10.0);
        let b = DeviceRect::new(20.0, 0.0, 10.0, 10.0);
        assert_eq!(a.intersect(&b), None);
        let c = DeviceRect::new(5.0, 5.0, 10.0, 10.0);
        assert_eq!(a.intersect(&c), Some(DeviceRect::new(5.0, 5.0, 5.0, 5.0)));
    }

    #[test]
    fn mesh_bounds_covers_every_vertex() {
        let mut plan = ScenePlan::default();
        plan.vertices.extend([
            MeshVertex::solid(3.0, 4.0),
            MeshVertex::solid(-1.0, 9.0),
            MeshVertex::solid(5.0, 2.0),
        ]);
        assert_eq!(
            plan.mesh_bounds(0, 3),
            Some(DeviceRect::new(-1.0, 2.0, 6.0, 7.0))
        );
        assert_eq!(plan.mesh_bounds(0, 0), None);
    }

    #[test]
    fn out_of_range_mesh_slices_are_empty_not_panics() {
        let plan = ScenePlan::default();
        assert!(plan.mesh_vertices(10, 3).is_empty());
        assert_eq!(plan.mesh_bounds(u32::MAX, u32::MAX), None);
    }

    #[test]
    fn clear_keeps_capacity() {
        let mut plan = ScenePlan::default();
        plan.vertices.extend([MeshVertex::solid(1.0, 2.0); 8]);
        plan.ops.push(SceneOp::PopClip);
        let cap = plan.vertices.capacity();
        plan.clear();
        assert!(plan.ops.is_empty() && plan.vertices.is_empty());
        assert_eq!(plan.vertices.capacity(), cap);
    }

    #[test]
    fn fully_transparent_paints_are_invisible() {
        assert!(Paint::Solid(Color::rgba(1, 2, 3, 0)).is_invisible());
        assert!(!Paint::Solid(Color::rgba(1, 2, 3, 1)).is_invisible());
        assert!(
            Paint::VGradient {
                top: Color::rgba(0, 0, 0, 0),
                bottom: Color::rgba(9, 9, 9, 0),
            }
            .is_invisible()
        );
        assert!(
            !Paint::VGradient {
                top: Color::rgba(0, 0, 0, 0),
                bottom: Color::rgba(9, 9, 9, 5),
            }
            .is_invisible()
        );
    }
}
