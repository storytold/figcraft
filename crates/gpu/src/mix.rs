//! The progressive blur's blend: every pixel between the two blur levels around its own radius,
//! as the renderer works it out (`figcraft_render::blur::progressive`).

use wgpu::{
    BindGroupDescriptor, BindGroupEntry, BindGroupLayout, BindGroupLayoutDescriptor, BindGroupLayoutEntry, BindingType, BufferDescriptor,
    BufferUsages, ColorTargetState, ColorWrites, CommandEncoder, Device, FragmentState, LoadOp, Operations, PipelineCompilationOptions,
    PipelineLayoutDescriptor, Queue, RenderPassColorAttachment, RenderPassDescriptor, RenderPipeline, RenderPipelineDescriptor,
    ShaderModuleDescriptor, ShaderSource, ShaderStages, StoreOp, Texture, TextureFormat, TextureViewDescriptor, VertexState,
};

/// The most blur levels a progressive blur draws (the renderer's own cap).
pub(crate) const MAX_LEVELS: usize = 8;

pub(crate) struct Mix {
    pipeline: RenderPipeline,
    layout: BindGroupLayout,
}

impl Mix {
    pub(crate) fn new(device: &Device) -> Option<Self> {
        let shader = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("figcraft progressive blur"),
            source: ShaderSource::Wgsl(include_str!("mix.wgsl").into()),
        });
        let texture = |binding: u32| BindGroupLayoutEntry {
            binding,
            visibility: ShaderStages::FRAGMENT,
            ty: BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let mut entries = vec![BindGroupLayoutEntry {
            binding: 0,
            visibility: ShaderStages::FRAGMENT,
            ty: BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
            count: None,
        }];
        entries.extend((1..=MAX_LEVELS as u32).map(texture));
        let layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor { label: Some("figcraft progressive blur"), entries: &entries });
        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("figcraft progressive blur"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&RenderPipelineDescriptor {
            label: Some("figcraft progressive blur"),
            layout: Some(&pipeline_layout),
            vertex: VertexState {
                module: &shader,
                entry_point: Some("vs"),
                compilation_options: PipelineCompilationOptions::default(),
                buffers: &[],
            },
            fragment: Some(FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                compilation_options: PipelineCompilationOptions::default(),
                targets: &[Some(ColorTargetState { format: TextureFormat::Rgba8Unorm, blend: None, write_mask: ColorWrites::ALL })],
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        Some(Self { pipeline, layout })
    }

    /// The blend of `levels` (their standard deviations, nearest first, each one's top-left in the
    /// layer and the resolution it was blurred at — the levels of a large layer are computed at
    /// half resolution and read upscaled, the layer itself never is) at a ramp from `s0` at the
    /// pixel point `a` to `s1` at `b` into `encoder`: the caller submits, with the levels' own
    /// passes, so a progressive blur reaches the device in one submission.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn encode(
        &self,
        device: &Device,
        queue: &Queue,
        encoder: &mut CommandEncoder,
        levels: &[(Texture, (f32, f32), f32)],
        deviations: &[f64],
        (s0, s1, a, b): (f64, f64, (f64, f64), (f64, f64)),
        size: (u32, u32),
        out: Texture,
    ) -> Option<Texture> {
        let (w, h) = size;
        if w == 0 || h == 0 || levels.is_empty() {
            return None;
        }
        let d = (b.0 - a.0, b.1 - a.1);
        let len2 = d.0 * d.0 + d.1 * d.1;
        let (tx, ty) = if len2.is_finite() && len2 > 1e-12 { (d.0 / len2, d.1 / len2) } else { (0.0, 0.0) };
        // The levels (32 bytes), the ramp, two levels' top-lefts to a 16-byte row, and the eight
        // scales.
        let mut params = [0u8; 160];
        for (i, v) in deviations.iter().take(MAX_LEVELS).enumerate() {
            params[i * 4..i * 4 + 4].copy_from_slice(&(*v as f32).to_le_bytes());
        }
        let values = [levels.len() as f32, s0 as f32, s1 as f32, ((0.5 - a.0) * tx + (0.5 - a.1) * ty) as f32, tx as f32, ty as f32];
        for (i, v) in values.iter().enumerate() {
            params[32 + i * 4..36 + i * 4].copy_from_slice(&v.to_le_bytes());
        }
        for (i, (_, (x, y), _)) in levels.iter().take(MAX_LEVELS).enumerate() {
            let at = 64 + (i / 2) * 16 + (i % 2) * 8;
            params[at..at + 4].copy_from_slice(&x.to_le_bytes());
            params[at + 4..at + 8].copy_from_slice(&y.to_le_bytes());
        }
        for (i, (_, _, scale)) in levels.iter().take(MAX_LEVELS).enumerate() {
            params[128 + i * 4..132 + i * 4].copy_from_slice(&scale.to_le_bytes());
        }
        let uniform = device.create_buffer(&BufferDescriptor {
            label: Some("figcraft progressive blur"),
            size: 160,
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&uniform, 0, &params);
        // Every slot is bound: unused ones read the first level (their index is never picked).
        let views: Vec<wgpu::TextureView> =
            (0..MAX_LEVELS).map(|i| levels.get(i).unwrap_or(&levels[0]).0.create_view(&TextureViewDescriptor::default())).collect();
        let mut entries = vec![BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() }];
        entries
            .extend(views.iter().enumerate().map(|(i, v)| BindGroupEntry { binding: 1 + i as u32, resource: wgpu::BindingResource::TextureView(v) }));
        let bind =
            device.create_bind_group(&BindGroupDescriptor { label: Some("figcraft progressive blur"), layout: &self.layout, entries: &entries });
        let target = out.create_view(&TextureViewDescriptor::default());
        {
            let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                label: Some("figcraft progressive blur"),
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
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.draw(0..3, 0..1);
        }
        Some(out)
    }
}
