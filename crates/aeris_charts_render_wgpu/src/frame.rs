//! Frame composition: one MSAA render pass drawing scissored groups of triangle meshes and
//! solid/textured quads in the frame's prim order.
//!
//! Groups replicate the reference's pane/axis canvas separation: the pane group is scissored to the
//! pane rect. 4x MSAA smooths diagonal line edges while leaving pixel-aligned rects and
//! texture-alpha text bit-identical (their edges never straddle a pixel boundary), so
//! candles and labels stay crisp.
//!
//! Execution order matches the Canvas2D executor exactly: within a layer, prims paint in
//! list order (a marker emitted after the candles covers the wicks on both backends). The
//! group therefore keeps one vertex/instance buffer per pipeline plus a run-length schedule
//! ([`DrawRun`]) — one draw call per maximal run of the same pipeline, so a candle block of
//! thousands of quads still costs a single instanced draw.

use aeris_charts_render::draw_list::Prim;

use crate::gpu_timer::{FrameTimestamps, GpuTimer};
use crate::quad_executor::prim_to_instances;
use crate::quad_pipeline::{QuadInstance, QuadRenderer};
use crate::tex_quad_pipeline::{TexQuadInstance, TexQuadRenderer};
use crate::tri_executor::geom_prim_to_tris;
use crate::tri_pipeline::{TriRenderer, TriVertex};

pub const SAMPLE_COUNT: u32 = 4;

/// The pipeline a [`DrawRun`] draws with; selects which group buffer the run indexes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunPipeline {
    /// Triangle mesh (fills, strokes, markers, background gradient); `first`/`count` are vertices.
    Tri,
    /// Solid instanced quads (rects, grid, candles, crosshair); `first`/`count` are instances.
    Quad,
    /// Textured instanced quads (label atlas); `first`/`count` are instances.
    TexQuad,
    /// Rotated label-atlas quads using location 2 as the canonical pivot/cosine/sine transform.
    RotatedTexQuad,
    /// Raster-image atlas.
    ImageQuad,
}

/// One draw call: `count` elements starting at `first` in the pipeline's group buffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DrawRun {
    pub pipeline: RunPipeline,
    pub first: u32,
    pub count: u32,
}

#[derive(Default)]
pub struct DrawGroup {
    /// x, y, w, h in bitmap px; None = full target.
    pub scissor: Option<[u32; 4]>,
    /// Triangle-mesh vertices in prim order across all layers (under, then main, then top).
    pub tris: Vec<TriVertex>,
    /// Solid-quad instances in prim order across all layers.
    pub quads: Vec<QuadInstance>,
    /// Textured-quad instances (browser-rasterized text runs). Scheduled in prim order by
    /// [`prims_to_group`]; a buffer populated without any scheduled text run keeps the previous
    /// whole-buffer, drawn-last behavior.
    pub tex_quads: Vec<TexQuadInstance>,
    /// Raster-image textured quads, kept separate from text so a large watermark cannot evict
    /// labels or force the whole frame onto Canvas2D.
    pub image_quads: Vec<TexQuadInstance>,
    /// Run-length draw schedule over `tris`/`quads`/`tex_quads`, in Canvas2D paint order.
    pub runs: Vec<DrawRun>,
    /// Semantic content revision. The host increments this only when rebuilding this group;
    /// retained GPU buffers skip uploads while it is unchanged.
    pub revision: u64,
    /// Engine/source revision last converted into this group's CPU streams.
    pub source_revision: u64,
    /// Stable semantic slot key. A changed key invalidates uploaded revisions while retaining
    /// compatible capacity at the same vector position.
    pub key: u64,
}

impl DrawGroup {
    /// Reset the geometry and schedule for reuse next frame (keeps the allocations).
    pub fn clear(&mut self) {
        self.tris.clear();
        self.quads.clear();
        self.tex_quads.clear();
        self.image_quads.clear();
        self.runs.clear();
        self.revision = self.revision.wrapping_add(1);
    }

    pub fn rebuild(&mut self, key: u64, revision: u64) {
        self.tris.clear();
        self.quads.clear();
        self.tex_quads.clear();
        self.image_quads.clear();
        self.runs.clear();
        self.key = key;
        self.source_revision = revision;
        self.revision = self.revision.wrapping_add(1).max(1);
    }
}

/// Record a run covering `[first, first + count)` of `pipeline`'s buffer, merging with the
/// previous run when it is the immediately preceding range of the same pipeline (run-length
/// batching: consecutive same-family prims stay one draw call).
fn push_run(runs: &mut Vec<DrawRun>, pipeline: RunPipeline, first: u32, count: u32) {
    if count == 0 {
        return;
    }
    if let Some(last) = runs.last_mut()
        && last.pipeline == pipeline
        && last.first + last.count == first
    {
        last.count += count;
        return;
    }
    runs.push(DrawRun {
        pipeline,
        first,
        count,
    });
}

fn needs_legacy_tex_fallback(group: &DrawGroup) -> bool {
    !group.tex_quads.is_empty()
        && !group.runs.iter().any(|run| {
            matches!(
                run.pipeline,
                RunPipeline::TexQuad | RunPipeline::RotatedTexQuad
            )
        })
}

/// Append one layer's prims to the group in list order, exactly as the Canvas2D executor
/// would paint them: each rect-family prim (`Rect`/`RectFrame`/`HLine`/`VLine`) extends the
/// quad buffer, every other geometry prim extends the tri buffer, and each maximal
/// same-pipeline run records one [`DrawRun`]. `Text` prims resolve through `resolve_text` —
/// the host seam that maps a run to its browser-rasterized atlas quad (the IR/wgpu crates
/// stay DOM-free). A resolved text quad schedules in prim order like everything else; a
/// `None` (empty run, raster failure, oversized run) collapses the slot without splitting
/// the surrounding runs, exactly like a degenerate rect.
pub fn prims_to_group(
    prims: &[Prim],
    points: &[[f32; 2]],
    group: &mut DrawGroup,
    resolve_text: &mut dyn FnMut(&Prim) -> Option<TexQuadInstance>,
    resolve_image: &mut dyn FnMut(&Prim) -> Option<TexQuadInstance>,
) {
    for prim in prims {
        match prim {
            Prim::Rect { .. }
            | Prim::RectFrame { .. }
            | Prim::HLine { .. }
            | Prim::VLine { .. } => {
                let first = group.quads.len() as u32;
                prim_to_instances(prim, &mut group.quads);
                push_run(
                    &mut group.runs,
                    RunPipeline::Quad,
                    first,
                    group.quads.len() as u32 - first,
                );
            }
            Prim::Text { .. } => {
                if let Some(instance) = resolve_text(prim) {
                    let first = group.tex_quads.len() as u32;
                    group.tex_quads.push(instance);
                    push_run(&mut group.runs, RunPipeline::TexQuad, first, 1);
                }
            }
            Prim::RotatedText { .. } => {
                if let Some(instance) = resolve_text(prim) {
                    let first = group.tex_quads.len() as u32;
                    group.tex_quads.push(instance);
                    push_run(&mut group.runs, RunPipeline::RotatedTexQuad, first, 1);
                }
            }
            Prim::Image { .. } => {
                if let Some(instance) = resolve_image(prim) {
                    let first = group.image_quads.len() as u32;
                    group.image_quads.push(instance);
                    push_run(&mut group.runs, RunPipeline::ImageQuad, first, 1);
                }
            }
            _ => {
                let first = group.tris.len() as u32;
                geom_prim_to_tris(prim, points, &mut group.tris);
                push_run(
                    &mut group.runs,
                    RunPipeline::Tri,
                    first,
                    group.tris.len() as u32 - first,
                );
            }
        }
    }
}

/// The MSAA color target; recreated on resize.
pub struct MsaaTarget {
    view: wgpu::TextureView,
    width: u32,
    height: u32,
}

impl MsaaTarget {
    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        width: u32,
        height: u32,
    ) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("msaa_target"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: SAMPLE_COUNT,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Self {
            view,
            width,
            height,
        }
    }

    /// Recreates the target if the size changed.
    pub fn ensure(
        &mut self,
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        width: u32,
        height: u32,
    ) {
        if self.width != width || self.height != height {
            *self = Self::new(device, format, width, height);
        }
    }

    pub fn view(&self) -> &wgpu::TextureView {
        &self.view
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BufferStats {
    pub allocations: u64,
    pub write_calls: u64,
    pub uploaded_bytes: u64,
}

#[derive(Default)]
struct ReusableBuffer {
    buffer: Option<wgpu::Buffer>,
    capacity: u64,
    uploaded_revision: Option<u64>,
}

impl ReusableBuffer {
    fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        contents: &[u8],
        revision: u64,
        label: &'static str,
        stats: &mut BufferStats,
    ) {
        if contents.is_empty() {
            return;
        }
        let required = contents.len() as u64;
        if required > self.capacity {
            let capacity = buffer_capacity(required);
            self.buffer = Some(device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: capacity,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
            self.capacity = capacity;
            self.uploaded_revision = None;
            stats.allocations += 1;
        }
        if self.uploaded_revision != Some(revision)
            && let Some(buffer) = &self.buffer
        {
            queue.write_buffer(buffer, 0, contents);
            self.uploaded_revision = Some(revision);
            stats.write_calls += 1;
            stats.uploaded_bytes += required;
        }
    }
}

fn buffer_capacity(required: u64) -> u64 {
    required.next_power_of_two().max(256)
}

#[derive(Default)]
struct GroupBuffers {
    key: u64,
    tris: ReusableBuffer,
    quads: ReusableBuffer,
    tex: ReusableBuffer,
    image: ReusableBuffer,
}

/// Per-chart retained vertex resources. Capacities grow geometrically and remain at their
/// high-water mark until the chart (and therefore this owner) is dropped.
#[derive(Default)]
pub struct FrameResources {
    groups: Vec<GroupBuffers>,
    stats: BufferStats,
}

impl FrameResources {
    pub fn stats(&self) -> BufferStats {
        self.stats
    }

    pub fn reset_stats(&mut self) {
        self.stats = BufferStats::default();
    }
}

/// Encode and submit one frame. Returns the number of draw calls issued, which the host
/// surfaces as `frame_stats().draw_calls`. `timer` opts into GPU-side pass timing when the
/// device supports `timestamp-query`; `None` skips the timestamp plumbing entirely.
#[allow(clippy::too_many_arguments)]
pub fn render_frame(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    msaa_view: &wgpu::TextureView,
    resolve_view: &wgpu::TextureView,
    width_px: u32,
    height_px: u32,
    clear_color: wgpu::Color,
    quad: &QuadRenderer,
    tex: &TexQuadRenderer,
    rotated_tex: &TexQuadRenderer,
    image: &TexQuadRenderer,
    tri: &TriRenderer,
    groups: &[DrawGroup],
    resources: &mut FrameResources,
    timer: Option<&GpuTimer>,
) -> u32 {
    let timestamps = FrameTimestamps::new(timer);
    let mut draw_calls = 0u32;
    quad.write_globals(queue, width_px, height_px);
    tex.write_globals(queue, width_px, height_px);
    rotated_tex.write_globals(queue, width_px, height_px);
    image.write_globals(queue, width_px, height_px);
    tri.write_globals(queue, width_px, height_px);

    resources
        .groups
        .resize_with(groups.len(), GroupBuffers::default);
    resources.groups.truncate(groups.len());
    for (group, buffers) in groups.iter().zip(&mut resources.groups) {
        if buffers.key != group.key {
            buffers.key = group.key;
            buffers.tris.uploaded_revision = None;
            buffers.quads.uploaded_revision = None;
            buffers.tex.uploaded_revision = None;
            buffers.image.uploaded_revision = None;
        }
        buffers.tris.prepare(
            device,
            queue,
            bytemuck::cast_slice(&group.tris),
            group.revision,
            "tris",
            &mut resources.stats,
        );
        buffers.quads.prepare(
            device,
            queue,
            bytemuck::cast_slice(&group.quads),
            group.revision,
            "quads",
            &mut resources.stats,
        );
        buffers.tex.prepare(
            device,
            queue,
            bytemuck::cast_slice(&group.tex_quads),
            group.revision,
            "tex",
            &mut resources.stats,
        );
        buffers.image.prepare(
            device,
            queue,
            bytemuck::cast_slice(&group.image_quads),
            group.revision,
            "images",
            &mut resources.stats,
        );
    }

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("frame"),
    });
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("frame_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: msaa_view,
                depth_slice: None,
                resolve_target: Some(resolve_view),
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(clear_color),
                    // resolve target holds the result; MSAA buffer itself can be discarded
                    store: wgpu::StoreOp::Discard,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: timestamps.pass_writes(),
            occlusion_query_set: None,
            multiview_mask: None,
        });

        for (group, bufs) in groups.iter().zip(&resources.groups) {
            let [sx, sy, sw, sh] = match group.scissor {
                Some([x, y, w, h]) => {
                    let x = x.min(width_px);
                    let y = y.min(height_px);
                    [x, y, w.min(width_px - x), h.min(height_px - y)]
                }
                None => [0, 0, width_px, height_px],
            };
            if sw == 0 || sh == 0 {
                continue;
            }
            pass.set_scissor_rect(sx, sy, sw, sh);

            // One draw call per scheduled run, in Canvas2D paint order.
            for run in &group.runs {
                match run.pipeline {
                    RunPipeline::Tri => {
                        if let Some(b) = &bufs.tris.buffer {
                            tri.draw(&mut pass, b, run.first, run.count);
                            draw_calls += 1;
                        }
                    }
                    RunPipeline::Quad => {
                        if let Some(b) = &bufs.quads.buffer {
                            quad.draw(&mut pass, b, run.first, run.count);
                            draw_calls += 1;
                        }
                    }
                    RunPipeline::TexQuad => {
                        if let Some(b) = &bufs.tex.buffer {
                            tex.draw(&mut pass, b, run.first, run.count);
                            draw_calls += 1;
                        }
                    }
                    RunPipeline::RotatedTexQuad => {
                        if let Some(b) = &bufs.tex.buffer {
                            rotated_tex.draw(&mut pass, b, run.first, run.count);
                            draw_calls += 1;
                        }
                    }
                    RunPipeline::ImageQuad => {
                        if let Some(b) = &bufs.image.buffer {
                            image.draw(&mut pass, b, run.first, run.count);
                            draw_calls += 1;
                        }
                    }
                }
            }
            // A directly populated tex buffer with no scheduled runs keeps the previous
            // whole-buffer, drawn-last behavior (textured quads paint above everything).
            if needs_legacy_tex_fallback(group)
                && let Some(b) = &bufs.tex.buffer
            {
                tex.draw(&mut pass, b, 0, group.tex_quads.len() as u32);
                draw_calls += 1;
            }
        }
    }

    timestamps.resolve_into_staging(&mut encoder);
    queue.submit(Some(encoder.finish()));
    timestamps.begin_readback();
    draw_calls
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_quad() -> TexQuadInstance {
        TexQuadInstance {
            rect: [0.0; 4],
            uv: [0.0; 4],
            color: [1.0; 4],
        }
    }

    #[test]
    fn scheduled_rotated_text_does_not_trigger_the_legacy_unrotated_draw() {
        let mut group = DrawGroup::default();
        group.tex_quads.push(text_quad());
        assert!(needs_legacy_tex_fallback(&group));

        group.runs.push(DrawRun {
            pipeline: RunPipeline::RotatedTexQuad,
            first: 0,
            count: 1,
        });
        assert!(!needs_legacy_tex_fallback(&group));

        group.runs[0].pipeline = RunPipeline::TexQuad;
        assert!(!needs_legacy_tex_fallback(&group));
    }

    #[test]
    fn vertex_capacity_grows_geometrically_and_never_exact_sizes() {
        assert_eq!(buffer_capacity(1), 256);
        assert_eq!(buffer_capacity(256), 256);
        assert_eq!(buffer_capacity(257), 512);
        assert_eq!(buffer_capacity(700), 1024);
    }

    #[test]
    fn retained_group_revision_changes_only_on_rebuild() {
        let mut group = DrawGroup::default();
        group.rebuild(7, 11);
        let revision = group.revision;
        assert_eq!(group.key, 7);
        assert_eq!(group.source_revision, 11);
        assert_eq!(group.revision, revision);
        group.rebuild(7, 12);
        assert_ne!(group.revision, revision);
    }
}
