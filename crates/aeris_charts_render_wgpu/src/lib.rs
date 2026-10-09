//! aeris_charts_render_wgpu — WebGPU backend.
//!
//! Pipelines: solid quads (integer rects — candles, wicks, grid, crosshair), textured quads
//! (label atlas), and triangles (line strokes, area fills). Frames are composed as scissored
//! draw groups in one 4x MSAA pass, replicating the reference's pane/axis canvas separation. MSAA
//! smooths diagonal lines while leaving pixel-aligned rects and text bit-identical.

mod atlas;
mod blend;
mod frame;
mod gpu_timer;
mod quad_executor;
mod quad_pipeline;
mod tex_quad_pipeline;
mod tri_executor;
mod tri_pipeline;

pub use atlas::{ATLAS_SIZE, AtlasSlot, LabelAtlas};
pub use frame::{
    BufferStats, DrawGroup, DrawRun, FrameResources, MsaaTarget, RunPipeline, SAMPLE_COUNT,
    prims_to_group, render_frame,
};
pub use gpu_timer::GpuTimer;
pub use quad_executor::{prim_to_instances, prims_to_instances};
pub use quad_pipeline::{QuadInstance, QuadRenderer};
pub use tex_quad_pipeline::{TexQuadInstance, TexQuadRenderer};
pub use tri_executor::{geom_prim_to_tris, geom_prims_to_tris};
pub use tri_pipeline::{TriRenderer, TriVertex};

pub use aeris_charts_render::draw_list::DrawList;
