//! Laying one canvas-sized layer over another: the page's segments are drawn into their own
//! textures and composited (premultiplied, source over) as the accumulated page moves forward.

use wgpu::{
    BindGroupDescriptor, BindGroupEntry, BindGroupLayout, BindGroupLayoutDescriptor, BindGroupLayoutEntry, BindingType, BlendComponent, BlendFactor,
    BlendOperation, BlendState, ColorTargetState, ColorWrites, CommandEncoderDescriptor, Device, FragmentState, LoadOp, Operations,
    PipelineCompilationOptions, PipelineLayoutDescriptor, Queue, RenderPassColorAttachment, RenderPassDescriptor, RenderPipeline,
    RenderPipelineDescriptor, Sampler, SamplerDescriptor, ShaderModuleDescriptor, ShaderSource, ShaderStages, StoreOp, TextureFormat, TextureView,
    VertexState,
};

pub(crate) struct Compose {
    /// Straight-alpha `src` premultiplied over the accumulation.
    pipeline: RenderPipeline,
    /// Straight-alpha `src` premultiplied into a fresh target (an effect layer).
    replace: RenderPipeline,
    /// `src` times a coverage texture's luminance × alpha (a luminance mask).
    masked: RenderPipeline,
    layout: BindGroupLayout,
    mask_layout: BindGroupLayout,
    sampler: Sampler,
    /// Bilinear, for a downscale (a blur's levels of a large layer are computed at half resolution).
    linear: Sampler,
    /// `src` into a smaller `dst` (premultiplied; its uv comes from the vertices).
    down: RenderPipeline,
}

impl Compose {
    pub(crate) fn new(device: &Device) -> Option<Self> {
        let shader = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("figcraft compose"),
            source: ShaderSource::Wgsl(include_str!("compose.wgsl").into()),
        });
        let layout = device
            .create_bind_group_layout(&BindGroupLayoutDescriptor { label: Some("figcraft compose"), entries: &[layout_entry(0), sampler_entry(1)] });
        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("figcraft compose"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        // Premultiplied "source over".
        let over = BlendComponent { src_factor: BlendFactor::One, dst_factor: BlendFactor::OneMinusSrcAlpha, operation: BlendOperation::Add };
        let build = |label: &str, blend: Option<BlendState>| {
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
                    entry_point: Some("fs"),
                    compilation_options: PipelineCompilationOptions::default(),
                    targets: &[Some(ColorTargetState { format: TextureFormat::Rgba8Unorm, blend, write_mask: ColorWrites::ALL })],
                }),
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let pipeline = build("figcraft compose", Some(BlendState { color: over, alpha: over }));
        let replace = build("figcraft premultiply", None);
        let down = device.create_render_pipeline(&RenderPipelineDescriptor {
            label: Some("figcraft downscale"),
            layout: Some(&pipeline_layout),
            vertex: VertexState {
                module: &shader,
                entry_point: Some("vs_down"),
                compilation_options: PipelineCompilationOptions::default(),
                buffers: &[],
            },
            fragment: Some(FragmentState {
                module: &shader,
                entry_point: Some("fs_down"),
                compilation_options: PipelineCompilationOptions::default(),
                targets: &[Some(ColorTargetState { format: TextureFormat::Rgba8Unorm, blend: None, write_mask: ColorWrites::ALL })],
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        // The luminance mask reads its coverage from a second texture.
        let mask_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("figcraft luminance mask"),
            entries: &[layout_entry(0), sampler_entry(1), layout_entry(2)],
        });
        let mask_pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("figcraft luminance mask"),
            bind_group_layouts: &[Some(&mask_layout)],
            immediate_size: 0,
        });
        let masked = device.create_render_pipeline(&RenderPipelineDescriptor {
            label: Some("figcraft luminance mask"),
            layout: Some(&mask_pipeline_layout),
            vertex: VertexState {
                module: &shader,
                entry_point: Some("vs"),
                compilation_options: PipelineCompilationOptions::default(),
                buffers: &[],
            },
            fragment: Some(FragmentState {
                module: &shader,
                entry_point: Some("fs_luma"),
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
            label: Some("figcraft compose"),
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        let linear = device.create_sampler(&SamplerDescriptor {
            label: Some("figcraft compose linear"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        Some(Self { pipeline, replace, down, masked, layout, mask_layout, sampler, linear })
    }

    /// Lay `src` over `dst` (both the canvas' size), keeping what `dst` already holds.
    pub(crate) fn over(&self, device: &Device, queue: &Queue, dst: &TextureView, src: &TextureView) {
        self.run(device, queue, dst, src, LoadOp::Load);
    }

    /// Premultiply `src` (straight alpha, as vello leaves it) into all of `dst`.
    pub(crate) fn premultiply(&self, device: &Device, queue: &Queue, src: &TextureView, dst: &TextureView) {
        self.run(device, queue, dst, src, LoadOp::Clear(wgpu::Color::TRANSPARENT));
    }

    /// `src` (a premultiplied layer) into a smaller `dst`, sampled bilinearly: the levels of a
    /// blur on a large layer are computed at half resolution and read upscaled.
    pub(crate) fn downscale(&self, device: &Device, queue: &Queue, dst: &TextureView, src: &TextureView) {
        self.run_with(device, queue, dst, src, LoadOp::Clear(wgpu::Color::TRANSPARENT), &self.linear, &self.down);
    }

    /// `dst` = `content` (a premultiplied layer) times `mask`'s luminance × alpha —
    /// tiny-skia's luminance mask, which the CPU renderer uses. All of `dst` is replaced.
    pub(crate) fn luminance(&self, device: &Device, queue: &Queue, dst: &TextureView, content: &TextureView, mask: &TextureView) {
        let bind = device.create_bind_group(&BindGroupDescriptor {
            label: Some("figcraft luminance mask"),
            layout: &self.mask_layout,
            entries: &[
                BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(content) },
                BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(mask) },
            ],
        });
        let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor { label: Some("figcraft luminance mask") });
        {
            let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                label: Some("figcraft luminance mask"),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: dst,
                    resolve_target: None,
                    ops: Operations { load: LoadOp::Clear(wgpu::Color::TRANSPARENT), store: StoreOp::Store },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.masked);
            pass.set_bind_group(0, &bind, &[]);
            pass.draw(0..3, 0..1);
        }
        queue.submit([encoder.finish()]);
    }

    /// Draw a straight-alpha `src` through `fs` into `dst`: premultiplied, over what `load` keeps
    /// or replacing it.
    fn run(&self, device: &Device, queue: &Queue, dst: &TextureView, src: &TextureView, load: LoadOp<wgpu::Color>) {
        let pipeline = if matches!(load, LoadOp::Clear(_)) { &self.replace } else { &self.pipeline };
        self.run_with(device, queue, dst, src, load, &self.sampler, pipeline);
    }

    /// [`Self::run`] with `sampler` and `pipeline` (linear and `down` for a downscale).
    fn run_with(
        &self,
        device: &Device,
        queue: &Queue,
        dst: &TextureView,
        src: &TextureView,
        load: LoadOp<wgpu::Color>,
        sampler: &Sampler,
        pipeline: &RenderPipeline,
    ) {
        let bind = device.create_bind_group(&BindGroupDescriptor {
            label: Some("figcraft compose"),
            layout: &self.layout,
            entries: &[
                BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(src) },
                BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(sampler) },
            ],
        });
        let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor { label: Some("figcraft compose") });
        {
            let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                label: Some("figcraft compose"),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: dst,
                    resolve_target: None,
                    ops: Operations { load, store: StoreOp::Store },
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
    }
}

/// A canvas-sized texture read by a compose pass at `binding`.
fn layout_entry(binding: u32) -> BindGroupLayoutEntry {
    BindGroupLayoutEntry {
        binding,
        visibility: ShaderStages::FRAGMENT,
        ty: BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

/// The compose passes' filterable sampler at `binding`.
fn sampler_entry(binding: u32) -> BindGroupLayoutEntry {
    BindGroupLayoutEntry { binding, visibility: ShaderStages::FRAGMENT, ty: BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None }
}

/// Clear `view` to `color` (the accumulation's first fill and the segments' transparent ground).
pub(crate) fn clear(device: &Device, queue: &Queue, view: &TextureView, color: wgpu::Color) {
    let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor { label: Some("figcraft clear") });
    {
        let _pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("figcraft clear"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view,
                resolve_target: None,
                ops: Operations { load: LoadOp::Clear(color), store: StoreOp::Store },
                depth_slice: None,
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
    }
    queue.submit([encoder.finish()]);
}
