//! Textured-quad pipeline for atlas labels. Glyph runs are rasterized by the host (Canvas2D)
//! **in the run's own color** — Chrome's glyph anti-aliasing is color-dependent (sRGB mask
//! gamma), so a white-on-transparent raster tinted at draw time visibly diverges from direct
//! `fillText` for non-white text. The shader folds the straight-alpha texel into premultiplied
//! form and multiplies by the instance tint (white for text), reproducing the browser's blend
//! exactly. Nearest sampling: labels are drawn 1:1 at integer bitmap positions, matching
//! Canvas2D `fillText` crispness. Raster images use the same instance layout with a dedicated
//! linear-sampling fragment entry point; their atlas bytes are premultiplied on insertion.
//! Rotated labels reconstruct bilinearly in their own shader from premultiplied texel loads.

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct TexQuadInstance {
    /// x, y, w, h in bitmap pixels.
    pub rect: [f32; 4],
    /// u0, v0, u1, v1 normalized atlas coords.
    pub uv: [f32; 4],
    /// Straight RGBA tint in 0..1.
    pub color: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Globals {
    viewport: [f32; 2],
    _pad: [f32; 2],
}

const SHADER: &str = r#"
struct Globals {
    viewport: vec2<f32>,
    _pad: vec2<f32>,
};

@group(0) @binding(0) var<uniform> globals: Globals;
@group(1) @binding(0) var atlas_tex: texture_2d<f32>;
@group(1) @binding(1) var atlas_samp: sampler;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) uv_bounds: vec4<f32>,
};

@vertex
fn vs_main(
    @builtin(vertex_index) vi: u32,
    @location(0) rect: vec4<f32>,
    @location(1) uv: vec4<f32>,
    @location(2) color: vec4<f32>,
) -> VsOut {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0),
        vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0),
    );
    let c = corners[vi];
    let px = rect.xy + c * rect.zw;
    let ndc = vec2<f32>(
        px.x / globals.viewport.x * 2.0 - 1.0,
        1.0 - px.y / globals.viewport.y * 2.0,
    );
    var out: VsOut;
    out.pos = vec4<f32>(ndc, 0.0, 1.0);
    out.uv = mix(uv.xy, uv.zw, c);
    out.color = color;
    out.uv_bounds = uv;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    // Texels are straight-alpha browser rasters in the run's color; fold to premultiplied and
    // tint. With the text path's white tint this is exactly the browser's source-over source.
    let t = textureSample(atlas_tex, atlas_samp, in.uv);
    let a = t.a * in.color.a;
    return vec4<f32>(t.rgb * in.color.rgb * a, a);
}

@fragment
fn fs_image(in: VsOut) -> @location(0) vec4<f32> {
    // Clamp to this image's edge texel, not the shared atlas edge. Linear reconstruction must
    // never read an adjacent image or the empty shelf area.
    let half_texel = vec2<f32>(0.5) / vec2<f32>(textureDimensions(atlas_tex));
    let uv = clamp(in.uv, in.uv_bounds.xy + half_texel, in.uv_bounds.zw - half_texel);
    let t = textureSample(atlas_tex, atlas_samp, uv);
    let a = t.a * in.color.a;
    return vec4<f32>(t.rgb * in.color.rgb * in.color.a, a);
}
"#;

// Rotated glyphs use the same legacy 48-byte instance layout and atlas raster. Text rasters
// always carry a white tint, so this dedicated pipeline interprets location 2 as
// [pivot_x, pivot_y, cos(angle), sin(angle)] and supplies white to the fragment stage. Keeping
// ordinary text on SHADER preserves its approved byte-for-byte raster and buffer contract.
// Filtering happens in the shader from `textureLoad`, so this pipeline binds no sampler.
const ROTATED_SHADER: &str = r#"
struct Globals {
    viewport: vec2<f32>,
    _pad: vec2<f32>,
};

@group(0) @binding(0) var<uniform> globals: Globals;
@group(1) @binding(0) var atlas_tex: texture_2d<f32>;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) uv_bounds: vec4<f32>,
};

@vertex
fn vs_main(
    @builtin(vertex_index) vi: u32,
    @location(0) rect: vec4<f32>,
    @location(1) uv: vec4<f32>,
    @location(2) pivot_rotation: vec4<f32>,
) -> VsOut {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0),
        vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0),
    );
    let c = corners[vi];
    let unrotated = rect.xy + c * rect.zw;
    let delta = unrotated - pivot_rotation.xy;
    let px = pivot_rotation.xy + vec2<f32>(
        delta.x * pivot_rotation.z - delta.y * pivot_rotation.w,
        delta.x * pivot_rotation.w + delta.y * pivot_rotation.z,
    );
    let ndc = vec2<f32>(
        px.x / globals.viewport.x * 2.0 - 1.0,
        1.0 - px.y / globals.viewport.y * 2.0,
    );
    var out: VsOut;
    out.pos = vec4<f32>(ndc, 0.0, 1.0);
    out.uv = mix(uv.xy, uv.zw, c);
    out.uv_bounds = uv;
    return out;
}

fn premultiply(t: vec4<f32>) -> vec4<f32> {
    return vec4<f32>(t.rgb * t.a, t.a);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    // Browser rasters are straight-alpha. Premultiply each source texel *before* bilinear
    // reconstruction; multiplying after filtering darkens every partially covered edge.
    let size = vec2<f32>(textureDimensions(atlas_tex));
    let pixel = in.uv * size - vec2<f32>(0.5);
    let base = vec2<i32>(floor(pixel));
    let fraction = fract(pixel);
    let lo = vec2<i32>(in.uv_bounds.xy * size);
    let hi = vec2<i32>(in.uv_bounds.zw * size) - vec2<i32>(1);
    let p00 = premultiply(textureLoad(atlas_tex, clamp(base, lo, hi), 0));
    let p10 = premultiply(textureLoad(atlas_tex, clamp(base + vec2<i32>(1, 0), lo, hi), 0));
    let p01 = premultiply(textureLoad(atlas_tex, clamp(base + vec2<i32>(0, 1), lo, hi), 0));
    let p11 = premultiply(textureLoad(atlas_tex, clamp(base + vec2<i32>(1, 1), lo, hi), 0));
    return mix(mix(p00, p10, fraction.x), mix(p01, p11, fraction.x), fraction.y);
}
"#;

pub struct TexQuadRenderer {
    pipeline: wgpu::RenderPipeline,
    globals_buf: wgpu::Buffer,
    globals_bg: wgpu::BindGroup,
    atlas_bg: wgpu::BindGroup,
}

impl TexQuadRenderer {
    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        atlas_view: &wgpu::TextureView,
        sample_count: u32,
    ) -> Self {
        Self::new_with_shader(
            device,
            format,
            atlas_view,
            sample_count,
            SHADER,
            Some(wgpu::FilterMode::Nearest),
            "fs_main",
        )
    }

    pub fn new_rotated(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        atlas_view: &wgpu::TextureView,
        sample_count: u32,
    ) -> Self {
        Self::new_with_shader(
            device,
            format,
            atlas_view,
            sample_count,
            ROTATED_SHADER,
            None,
            "fs_main",
        )
    }

    /// Raster images use the frame contract's bilinear filter. Ordinary text stays nearest.
    pub fn new_image(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        atlas_view: &wgpu::TextureView,
        sample_count: u32,
    ) -> Self {
        Self::new_with_shader(
            device,
            format,
            atlas_view,
            sample_count,
            SHADER,
            Some(wgpu::FilterMode::Linear),
            "fs_image",
        )
    }

    fn new_with_shader(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        atlas_view: &wgpu::TextureView,
        sample_count: u32,
        shader_source: &'static str,
        filter: Option<wgpu::FilterMode>,
        fragment_entry: &'static str,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("tex_quad_shader"),
            source: wgpu::ShaderSource::Wgsl(shader_source.into()),
        });

        let globals_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("tex_quad_globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let globals_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("tex_quad_globals_bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let globals_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("tex_quad_globals_bg"),
            layout: &globals_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: globals_buf.as_entire_binding(),
            }],
        });

        let texture_entry = wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let sampler_entry = wgpu::BindGroupLayoutEntry {
            binding: 1,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        };
        let layout_entries = [texture_entry, sampler_entry];
        let atlas_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("tex_quad_atlas_bgl"),
            entries: if filter.is_some() {
                &layout_entries
            } else {
                &layout_entries[..1]
            },
        });

        // Axis-aligned text samples nearest so it stays pixel-exact at 1:1; raster images
        // sample linearly. The rotated-text shader reconstructs bilinearly from `textureLoad`
        // after premultiplying each texel, so it binds no sampler at all.
        let sampler = filter.map(|filter| {
            device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("atlas_sampler"),
                mag_filter: filter,
                min_filter: filter,
                ..Default::default()
            })
        });
        let texture_binding = wgpu::BindGroupEntry {
            binding: 0,
            resource: wgpu::BindingResource::TextureView(atlas_view),
        };
        let atlas_bg = match &sampler {
            Some(sampler) => device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("tex_quad_atlas_bg"),
                layout: &atlas_bgl,
                entries: &[
                    texture_binding,
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(sampler),
                    },
                ],
            }),
            None => device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("tex_quad_atlas_bg"),
                layout: &atlas_bgl,
                entries: &[texture_binding],
            }),
        };

        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("tex_quad_layout"),
            bind_group_layouts: &[Some(&globals_bgl), Some(&atlas_bgl)],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("tex_quad_pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<TexQuadInstance>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &[
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x4,
                            offset: 0,
                            shader_location: 0,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x4,
                            offset: 16,
                            shader_location: 1,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x4,
                            offset: 32,
                            shader_location: 2,
                        },
                    ],
                })],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState {
                count: sample_count,
                ..Default::default()
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some(fragment_entry),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        Self {
            pipeline,
            globals_buf,
            globals_bg,
            atlas_bg,
        }
    }

    pub(crate) fn write_globals(&self, queue: &wgpu::Queue, width_px: u32, height_px: u32) {
        let globals = Globals {
            viewport: [width_px as f32, height_px as f32],
            _pad: [0.0, 0.0],
        };
        queue.write_buffer(&self.globals_buf, 0, bytemuck::bytes_of(&globals));
    }

    pub(crate) fn draw<'p>(
        &'p self,
        pass: &mut wgpu::RenderPass<'p>,
        instances: &'p wgpu::Buffer,
        first: u32,
        count: u32,
    ) {
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.globals_bg, &[]);
        pass.set_bind_group(1, &self.atlas_bg, &[]);
        pass.set_vertex_buffer(0, instances.slice(..));
        pass.draw(0..6, first..first + count);
    }
}
