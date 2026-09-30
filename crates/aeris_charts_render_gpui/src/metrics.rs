//! Adapter timing and count metrics.
//!
//! Deliberately free of GPUI types so a host can log or assert on backend metrics without linking
//! GPUI or leaking GPUI types back into the engine.

/// What one `paint_frame` call emitted, and how long it took.
///
/// `plan_*` fields describe the backend-neutral lowering pass ([`crate::ScenePlan`]); `paint_*`
/// fields describe handing that plan to GPUI. Counts are exact, not sampled.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GpuiFrameMetrics {
    /// `Prim`s consumed from the frame, across every pane and axis layer.
    pub prims: u32,
    /// Ops emitted into the plan (the sum of the per-kind counts below plus clip ops).
    pub ops: u32,
    /// Axis-aligned quads (`Rect`/`RectFrame`/`HLine`/`VLine`/`Background`/`RoundRect` fast path).
    pub quads: u32,
    /// Triangle-mesh paths (`Polyline`/`Segments`/`AreaFill`/`BandFill`/`Circle`/`Triangle`/`RoundRect`).
    pub paths: u32,
    /// Triangles across every mesh.
    pub triangles: u32,
    /// Text runs lowered into the plan.
    pub text_runs: u32,
    /// Raster-image runs lowered into the plan and successfully submitted to GPUI.
    pub image_runs: u32,
    pub image_runs_painted: u32,
    /// `paint_image` errors surfaced as an exact count rather than silently discarded.
    pub image_paint_failures: u32,
    /// Text runs GPUI actually shaped and painted (`paint_frame` only).
    pub glyph_runs_painted: u32,
    /// Clip (content-mask) pushes.
    pub clips: u32,
    /// Plain opaque quads collapsed into batched paths at submission time, and how many batches
    /// they became. `batched_quads - quad_batches` is the number of `paint_quad` calls saved.
    pub batched_quads: u32,
    pub quad_batches: u32,
    /// Prims that lowered to nothing (degenerate extents, empty text, short polylines). Tracked
    /// because a silent drop is otherwise indistinguishable from a mapping gap.
    pub dropped_prims: u32,
    /// Nanoseconds spent lowering `Prim`s into the plan.
    pub plan_nanos: u64,
    /// Nanoseconds spent issuing the plan to GPUI (0 when the plan was only built).
    pub paint_nanos: u64,
    /// Shaped-text cache hits / misses over this frame.
    pub text_cache_hits: u32,
    pub text_cache_misses: u32,
    /// Mesh vertices retained in the plan's shared pool (steady-state allocation signal).
    pub mesh_vertices: u32,
}

impl GpuiFrameMetrics {
    /// Total nanoseconds attributable to the adapter.
    pub fn total_nanos(&self) -> u64 {
        self.plan_nanos.saturating_add(self.paint_nanos)
    }

    /// Total milliseconds attributable to the adapter, for the performance gate.
    pub fn total_millis(&self) -> f64 {
        self.total_nanos() as f64 / 1_000_000.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn total_sums_both_phases() {
        let m = GpuiFrameMetrics {
            plan_nanos: 1_200_000,
            paint_nanos: 800_000,
            ..Default::default()
        };
        assert_eq!(m.total_nanos(), 2_000_000);
        assert!((m.total_millis() - 2.0).abs() < 1e-9);
    }

    #[test]
    fn total_saturates_instead_of_overflowing() {
        let m = GpuiFrameMetrics {
            plan_nanos: u64::MAX,
            paint_nanos: 5,
            ..Default::default()
        };
        assert_eq!(m.total_nanos(), u64::MAX);
    }
}
