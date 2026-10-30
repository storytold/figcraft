//! Shadow passes: the renderer's tint and morph on the GPU, so a shadow of any shape and an inner
//! shadow cost a blur and a full-screen pass each, with no pixels crossing the bus.

use figcraft_color::Color;
use wgpu::{
    BindGroupDescriptor, BindGroupEntry, BindGroupLayout, BindGroupLayoutDescriptor, BindGroupLayoutEntry, BindingType, BufferDescriptor,
    BufferUsages, ColorTargetState, ColorWrites, CommandEncoderDescriptor, Device, Extent3d, FragmentState, LoadOp, Operations,
    PipelineCompilationOptions, PipelineLayoutDescriptor, Queue, RenderPassColorAttachment, RenderPassDescriptor, RenderPipeline,
    RenderPipelineDescriptor, ShaderModuleDescriptor, ShaderSource, ShaderStages, StoreOp, Texture, TextureDescriptor, TextureDimension,
    TextureFormat, TextureUsages, TextureViewDescriptor, VertexState,
};

pub(crate) struct Shadow {
    /// `fx::tint`: a coverage in the shadow's colour.
    tint: RenderPipeline,
    /// `fx::morph`: alpha grown or shrunk by whole pixels, white.
    morph: RenderPipeline,
    layout: BindGroupLayout,
}

impl Shadow {
    pub(crate) fn new(device: &Device) -> Option<Self> {
        let shader = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("figcraft shadow"),
            source: ShaderSource::Wgsl(include_str!("shadow.wgsl").into()),
        });
        let layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("figcraft shadow"),
            entries: &[
                BindGroupLayoutEntry {
                    binding: 0,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 1,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("figcraft shadow"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let build = |label: &str, entry: &str| {
            device.create_render_pipeline(&RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                vertex: VertexState {
                    module: &shader,
                    entry_point: Some("vs"),
                    compilation_options: PipelineCompilationOptions::default(),
                    buffers: &[],
                },
                fragment: Some(FragmentState {
                    module: &shader,
                    entry_point: Some(entry),
                    compilation_options: PipelineCompilationOptions::default(),
                    targets: &[Some(ColorTargetState { format: TextureFormat::Rgba8Unorm, blend: None, write_mask: ColorWrites::ALL })],
                }),
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let tint = build("figcraft shadow tint", "tint");
        let morph = build("figcraft shadow morph", "morph");
        Some(Self { tint, morph, layout })
    }

    /// `src` in `color` at its own alpha (alpha only: a silhouette's coverage, as `fx::tint`).
    pub(crate) fn tint(&self, device: &Device, queue: &Queue, src: &Texture, (w, h): (u32, u32), color: Color) -> Option<Texture> {
        let c = color.sanitized();
        let mut params = [0u8; 32];
        for (i, v) in [c.r, c.g, c.b, c.a].into_iter().enumerate() {
            params[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
        }
        self.pass(device, queue, &self.tint, src, (w, h), &params, "figcraft shadow tint")
    }

    /// `src`'s alpha grown (`grow > 0`) or shrunk by `grow` whole pixels with a square kernel, as
    /// white; `invert` gives 1 - alpha (`fx::morph` and `fx::invert_alpha`).
    pub(crate) fn morph(&self, device: &Device, queue: &Queue, src: &Texture, (w, h): (u32, u32), grow: i32, invert: bool) -> Option<Texture> {
        let mut params = [0u8; 32];
        params[16..20].copy_from_slice(&(grow as f32).to_le_bytes());
        params[20..24].copy_from_slice(&f32::from(u8::from(invert)).to_le_bytes());
        let horizontal = |params: &mut [u8; 32]| {
            params[24..28].copy_from_slice(&1.0f32.to_le_bytes());
        };
        let mut first = params;
        horizontal(&mut first);
        let a = self.pass(device, queue, &self.morph, src, (w, h), &first, "figcraft shadow morph")?;
        self.pass(device, queue, &self.morph, &a, (w, h), &params, "figcraft shadow morph")
    }

    fn pass(
        &self,
        device: &Device,
        queue: &Queue,
        pipeline: &RenderPipeline,
        src: &Texture,
        (w, h): (u32, u32),
        params: &[u8],
        label: &str,
    ) -> Option<Texture> {
        if w == 0 || h == 0 || !crate::canvas::fits(device, w, h) {
            return None;
        }
        let uniform = device.create_buffer(&BufferDescriptor {
            label: Some(label),
            size: params.len() as u64,
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&uniform, 0, params);
        let bind = device.create_bind_group(&BindGroupDescriptor {
            label: Some(label),
            layout: &self.layout,
            entries: &[
                BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&src.create_view(&TextureViewDescriptor::default())) },
                BindGroupEntry { binding: 1, resource: uniform.as_entire_binding() },
            ],
        });
        let out = texture(device, label, w, h);
        let target = out.create_view(&TextureViewDescriptor::default());
        let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor { label: Some(label) });
        {
            let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                label: Some(label),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: &target,
                    resolve_target: None,
                    ops: Operations { load: LoadOp::Clear(wgpu::Color::TRANSPARENT), store: StoreOp::Store },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.draw(0..3, 0..1);
        }
        queue.submit([encoder.finish()]);
        Some(out)
    }
}

fn texture(device: &Device, label: &str, w: u32, h: u32) -> Texture {
    device.create_texture(&TextureDescriptor {
        label: Some(label),
        size: Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format: TextureFormat::Rgba8Unorm,
        usage: TextureUsages::TEXTURE_BINDING
            | TextureUsages::RENDER_ATTACHMENT
            | TextureUsages::COPY_DST
            | TextureUsages::COPY_SRC
            | TextureUsages::STORAGE_BINDING,
        view_formats: &[],
    })
}
