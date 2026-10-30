//! Plus-darker: the CPU renderer's `fx::plus_darker` as one pass, since vello has no such blend
//! mode. The page is copied first (`under`), because a render pass can't read its own target.

use wgpu::{
    BindGroupDescriptor, BindGroupEntry, BindGroupLayoutDescriptor, BindGroupLayoutEntry, BindingType, ColorTargetState, ColorWrites,
    CommandEncoderDescriptor, Device, FragmentState, LoadOp, Operations, PipelineCompilationOptions, PipelineLayoutDescriptor, Queue,
    RenderPassColorAttachment, RenderPassDescriptor, RenderPipeline, RenderPipelineDescriptor, SamplerDescriptor, ShaderModuleDescriptor,
    ShaderSource, ShaderStages, StoreOp, TextureFormat, TextureView, VertexState,
};

pub(crate) struct Plus {
    pipeline: RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
}

impl Plus {
    pub(crate) fn new(device: &Device) -> Option<Self> {
        let shader = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("figcraft plus-darker"),
            source: ShaderSource::Wgsl(include_str!("plus.wgsl").into()),
        });
        let texture = |binding| BindGroupLayoutEntry {
            binding,
            visibility: ShaderStages::FRAGMENT,
            ty: BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("figcraft plus-darker"),
            entries: &[
                texture(0),
                BindGroupLayoutEntry {
                    binding: 1,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                texture(2),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("figcraft plus-darker"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&RenderPipelineDescriptor {
            label: Some("figcraft plus-darker"),
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
        let sampler = device.create_sampler(&SamplerDescriptor {
            label: Some("figcraft plus-darker"),
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        Some(Self { pipeline, layout, sampler })
    }

    /// `page` = `src` plus-darker'd over the copy of the page in `under`, both premultiplied.
    pub(crate) fn run(&self, device: &Device, queue: &Queue, page: &TextureView, under: &TextureView, src: &TextureView) {
        let bind = device.create_bind_group(&BindGroupDescriptor {
            label: Some("figcraft plus-darker"),
            layout: &self.layout,
            entries: &[
                BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(under) },
                BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(src) },
            ],
        });
        let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor { label: Some("figcraft plus-darker") });
        {
            let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                label: Some("figcraft plus-darker"),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: page,
                    resolve_target: None,
                    ops: Operations { load: LoadOp::Load, store: StoreOp::Store },
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
        queue.submit([encoder.finish()]);
    }
}
