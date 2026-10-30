//! The blur: three box passes each way (the CPU blur's radii), a full-screen pass at a time, then
//! the pixels read back.
//!
//! A box pass is O(1) a pixel: a compute pass gives every pixel the running sum along its line, and
//! a pass takes the window's average from two of those sums. (A device that can't run the scan
//! falls back to reading the window directly, which costs what the radius costs.)

use std::num::NonZeroU64;

use figcraft_render::Pixmap;
use wgpu::{
    BindGroupDescriptor, BindGroupEntry, BindGroupLayout, BindGroupLayoutDescriptor, BindGroupLayoutEntry, BindingType, BlendState, BufferDescriptor,
    BufferUsages, ColorTargetState, ColorWrites, CommandEncoder, CommandEncoderDescriptor, ComputePassDescriptor, ComputePipeline,
    ComputePipelineDescriptor, Device, Extent3d, FragmentState, LoadOp, Operations, PipelineCompilationOptions, PipelineLayoutDescriptor, Queue,
    RenderPassColorAttachment, RenderPassDescriptor, RenderPipeline, RenderPipelineDescriptor, ShaderModuleDescriptor, ShaderSource, ShaderStages,
    StoreOp, TexelCopyBufferInfo, TexelCopyBufferLayout, TexelCopyTextureInfo, Texture, TextureDescriptor, TextureDimension, TextureFormat,
    TextureUsages, TextureView, TextureViewDescriptor, VertexState,
};

/// Blurs smaller than this many bytes stay on the CPU (a read-back costs more than they save).
pub(crate) const GPU_MIN_BYTES: usize = 200_000;

/// The scan's workgroup size (one pixel a thread, a tile at a time).
const SCAN_WORKGROUP: u32 = 256;
/// The largest radius the one-dispatch halo pass covers (the shader's `MAX_RADIUS`).
const HALO_RADIUS: u32 = 256;
/// The bytes that pass's halo holds in shared memory (the shader's `HALO` × 16).
const HALO_BYTES: u32 = 768 * 16;

pub(crate) struct Blur {
    /// The sampled texture and the pass's parameters.
    layout: BindGroupLayout,
    /// The scan's layout: the same, plus the prefix sums it writes.
    scan_layout: Option<BindGroupLayout>,
    /// The running sums, a line a workgroup.
    scan: Option<ComputePipeline>,
    /// The whole box pass in one dispatch (a workgroup a tile, its halo in shared memory).
    fused: Option<(ComputePipeline, BindGroupLayout)>,
    /// The window's average from those sums.
    average: Option<RenderPipeline>,
    /// The window read directly, for a device without the scan.
    direct: Option<RenderPipeline>,
    /// The textures a run reads and writes, kept between runs: a progressive blur would otherwise
    /// make (and drop) a dozen full-size textures a frame, and that churn shows as judder.
    pool: std::sync::Mutex<Vec<Texture>>,
}

/// Most bytes the pool keeps: a blur's textures are full-size, and a progressive blur needs about
/// ten of them at once (its levels, the two a box pass ping-pongs between).
const POOL_BYTES: u64 = 128 << 20;

impl Blur {
    pub(crate) fn new(device: &Device) -> Option<Self> {
        let shader = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("figcraft blur"),
            source: ShaderSource::Wgsl(include_str!("blur.wgsl").into()),
        });
        let texture = |binding| BindGroupLayoutEntry {
            binding,
            // The compute pass reads the source too (its running sums).
            visibility: ShaderStages::FRAGMENT | ShaderStages::COMPUTE,
            ty: BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let uniform = BindGroupLayoutEntry {
            binding: 1,
            visibility: ShaderStages::FRAGMENT | ShaderStages::COMPUTE,
            // A run's passes share one buffer: a pass's parameters are a slot in it.
            ty: BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: true, min_binding_size: None },
            count: None,
        };
        let layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor { label: Some("figcraft blur"), entries: &[texture(0), uniform] });
        let pass = |label: &str, entry: &'static str, layout: &BindGroupLayout| {
            let layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
                label: Some(label),
                bind_group_layouts: &[Some(layout)],
                immediate_size: 0,
            });
            device.create_render_pipeline(&RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&layout),
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
                    targets: &[Some(ColorTargetState {
                        format: TextureFormat::Rgba8Unorm,
                        blend: Some(BlendState::REPLACE),
                        write_mask: ColorWrites::ALL,
                    })],
                }),
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        // The scan needs a compute pass (`rgba32float` storage is core WebGPU, so every desktop
        // backend the canvas runs on can write the sums).
        let scan_supported =
            device.limits().max_compute_workgroup_size_x >= SCAN_WORKGROUP && device.limits().max_compute_invocations_per_workgroup >= SCAN_WORKGROUP;
        let (scan_layout, scan, average) = if scan_supported {
            let sums = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
                label: Some("figcraft blur scan"),
                entries: &[
                    texture(0),
                    uniform,
                    BindGroupLayoutEntry {
                        binding: 2,
                        visibility: ShaderStages::COMPUTE,
                        ty: BindingType::StorageTexture {
                            access: wgpu::StorageTextureAccess::WriteOnly,
                            format: TextureFormat::Rgba32Float,
                            view_dimension: wgpu::TextureViewDimension::D2,
                        },
                        count: None,
                    },
                ],
            });
            let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
                label: Some("figcraft blur scan"),
                bind_group_layouts: &[Some(&sums)],
                immediate_size: 0,
            });
            let scan = device.create_compute_pipeline(&ComputePipelineDescriptor {
                label: Some("figcraft blur scan"),
                layout: Some(&pipeline_layout),
                module: &shader,
                entry_point: Some("scan"),
                compilation_options: PipelineCompilationOptions::default(),
                cache: None,
            });
            let average = pass("figcraft blur average", "fs_box", &layout);
            (Some(sums), Some(scan), Some(average))
        } else {
            (None, None, None)
        };
        // The one-dispatch pass: a workgroup scans 768 pixels of a line in shared memory, so it
        // needs that much of it (12 KiB, within the 16 KiB every device has).
        let fused = if scan.is_some() && device.limits().max_compute_workgroup_storage_size >= HALO_BYTES {
            let box_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
                label: Some("figcraft blur box"),
                entries: &[
                    texture(0),
                    uniform,
                    BindGroupLayoutEntry {
                        binding: 3,
                        visibility: ShaderStages::COMPUTE,
                        ty: BindingType::StorageTexture {
                            access: wgpu::StorageTextureAccess::WriteOnly,
                            format: TextureFormat::Rgba8Unorm,
                            view_dimension: wgpu::TextureViewDimension::D2,
                        },
                        count: None,
                    },
                ],
            });
            let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
                label: Some("figcraft blur box"),
                bind_group_layouts: &[Some(&box_layout)],
                immediate_size: 0,
            });
            let pipeline = device.create_compute_pipeline(&ComputePipelineDescriptor {
                label: Some("figcraft blur box"),
                layout: Some(&pipeline_layout),
                module: &shader,
                entry_point: Some("box"),
                compilation_options: PipelineCompilationOptions::default(),
                cache: None,
            });
            Some((pipeline, box_layout))
        } else {
            None
        };
        let direct = Some(pass("figcraft blur", "fs", &layout));
        Some(Self { direct, layout, scan_layout, scan, fused, average, pool: std::sync::Mutex::new(Vec::new()) })
    }

    /// A texture of `size`, `format` and `usage` for a run: one kept from an earlier run when its
    /// shape matches, else a new one.
    fn take(&self, device: &Device, label: &str, (w, h): (u32, u32), format: TextureFormat, usage: TextureUsages) -> Texture {
        let mut pool = self.pool.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(i) = pool.iter().position(|t| t.width() == w && t.height() == h && t.format() == format && t.usage() == usage) {
            return pool.swap_remove(i);
        }
        texture(device, label, (w, h), format, usage)
    }

    /// Keep `texture` for the next run (its passes are done: a queue runs in order). The oldest
    /// textures go first when the pool is full: clearing it would make the next run allocate them
    /// all again, which is exactly the churn the pool is there to avoid.
    pub(crate) fn give_back(&self, texture: Texture) {
        let mut pool = self.pool.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let bytes = |t: &Texture| u64::from(t.width()) * u64::from(t.height()) * 4;
        let mut used: u64 = pool.iter().map(bytes).sum();
        let want = bytes(&texture);
        while used.saturating_add(want) > POOL_BYTES && !pool.is_empty() {
            let old = pool.remove(0);
            used = used.saturating_sub(bytes(&old));
        }
        if used.saturating_add(want) <= POOL_BYTES {
            pool.push(texture);
        }
    }
}

/// The textures a blur reads and writes: one after one, a box pass a time.
fn texture(device: &Device, label: &str, (w, h): (u32, u32), format: TextureFormat, usage: TextureUsages) -> Texture {
    device.create_texture(&TextureDescriptor {
        label: Some(label),
        size: Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    })
}

/// The blur's box passes over the `w` × `h` crop of `src` at `origin` into `encoder` (`None` when
/// the device can't): the result texture, and the scratch to hand back to the pool once the caller
/// has submitted it. A progressive blur's levels and its blend then reach the device in one
/// submission — a submit a box pass made the canvas queue a dozen of them a frame, each waiting
/// behind the window's. `sums` is the running sums' texture (a whole layer, reused by every pass
/// and every level); without one the run takes its own and hands it back with the scratch.
#[allow(clippy::too_many_arguments)]
fn boxes(
    device: &Device,
    queue: &Queue,
    blur: &Blur,
    encoder: &mut CommandEncoder,
    src: &TextureView,
    origin: (u32, u32),
    (w, h): (u32, u32),
    sigma: f64,
    sums: Option<&Texture>,
) -> Option<(Texture, Vec<Texture>)> {
    if w == 0 || h == 0 {
        return None;
    }
    if !crate::canvas::fits(device, w, h) {
        return None;
    }
    if blur.scan.is_none() && blur.direct.is_none() {
        return None;
    }
    let io = TextureUsages::TEXTURE_BINDING
        | TextureUsages::RENDER_ATTACHMENT
        | TextureUsages::COPY_DST
        | TextureUsages::COPY_SRC
        | TextureUsages::STORAGE_BINDING;
    let mut current = blur.take(device, "figcraft blur a", (w, h), TextureFormat::Rgba8Unorm, io);
    let mut other = blur.take(device, "figcraft blur b", (w, h), TextureFormat::Rgba8Unorm, io);
    let radii = figcraft_render::fx::box_radii(sigma);
    // Every pass's parameters are a slot of one buffer: a buffer a pass (and a strip a pass
    // before that) was two hundred small allocations a frame, and the driver's churn showed.
    const SLOT: usize = 32;
    let align = usize::try_from(device.limits().min_uniform_buffer_offset_alignment).unwrap_or(SLOT).max(SLOT);
    let mut data = vec![0u8; align * radii.len() * 2];
    for (k, r) in radii.iter().enumerate() {
        for d in 0..2usize {
            if let Some(slot) = data.get_mut((k * 2 + d) * align..(k * 2 + d) * align + SLOT) {
                slot[0..4].copy_from_slice(&u32::try_from(*r).unwrap_or(u32::MAX).to_le_bytes());
                slot[4..8].copy_from_slice(&(d as u32).to_le_bytes());
                // Only the first pass reads the layer; the rest read the crop the pass before made.
                let (ox, oy) = if k == 0 && d == 0 { origin } else { (0, 0) };
                slot[8..12].copy_from_slice(&ox.to_le_bytes());
                slot[12..16].copy_from_slice(&oy.to_le_bytes());
                slot[16..20].copy_from_slice(&w.to_le_bytes());
                slot[20..24].copy_from_slice(&h.to_le_bytes());
            }
        }
    }
    let params = device.create_buffer(&BufferDescriptor {
        label: Some("figcraft blur params"),
        size: u64::try_from(data.len()).unwrap_or(64),
        usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(&params, 0, &data);
    let uniform = |offset: u64| BindGroupEntry {
        binding: 1,
        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: &params, offset, size: NonZeroU64::new(SLOT as u64) }),
    };
    // The sums: only a pass too wide for the halo keeps them (the caller's, else one from the
    // pool, kept for the next run).
    let needs_sums = blur.scan.is_some() && (blur.fused.is_none() || radii.iter().any(|r| *r > HALO_RADIUS as usize));
    let mut own_sums = None;
    let sums = if needs_sums {
        match sums {
            Some(t) => Some(t),
            None => {
                own_sums = Some(blur.take(
                    device,
                    "figcraft blur sums",
                    (w, h),
                    TextureFormat::Rgba32Float,
                    TextureUsages::STORAGE_BINDING | TextureUsages::TEXTURE_BINDING,
                ));
                own_sums.as_ref()
            }
        }
    } else {
        None
    };
    let mut read = src.clone();
    // When every pass fits the one-dispatch form, the whole chain goes into one compute pass: a
    // pass a box pass was the recording that made a window-sized blur cost 5–28 ms in
    // `encoder.finish` (a dozen bind groups and command lists), where the dispatches are cheap.
    // Each dispatch is its own usage scope, so wgpu keeps their reads and writes in order.
    let all_fused = blur.fused.is_some() && radii.iter().all(|r| *r <= HALO_RADIUS as usize);
    if all_fused && let Some((fused, fused_layout)) = &blur.fused {
        let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor { label: Some("figcraft blur box"), timestamp_writes: None });
        pass.set_pipeline(fused);
        for (k, _) in radii.iter().enumerate() {
            for (d, vertical) in [false, true].into_iter().enumerate() {
                let target = other.create_view(&TextureViewDescriptor::default());
                let slot = u32::try_from((k * 2 + d) * align).unwrap_or(0);
                let bind = device.create_bind_group(&BindGroupDescriptor {
                    label: Some("figcraft blur box"),
                    layout: fused_layout,
                    entries: &[
                        BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&read) },
                        uniform(0),
                        BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&target) },
                    ],
                });
                let (len, lines) = if vertical { (h, w) } else { (w, h) };
                pass.set_bind_group(0, &bind, &[slot]);
                pass.dispatch_workgroups(len.div_ceil(256), lines.max(1), 1);
                std::mem::swap(&mut current, &mut other);
                read = current.create_view(&TextureViewDescriptor::default());
            }
        }
    } else {
        for (k, radius) in radii.iter().copied().enumerate() {
            for (d, vertical) in [false, true].into_iter().enumerate() {
                let target = other.create_view(&TextureViewDescriptor::default());
                let slot = u32::try_from((k * 2 + d) * align).unwrap_or(0);
                let mut started = false;
                // The whole pass in one dispatch, where the halo fits in shared memory.
                if radius <= HALO_RADIUS as usize
                    && let Some((fused, fused_layout)) = &blur.fused
                {
                    let bind = device.create_bind_group(&BindGroupDescriptor {
                        label: Some("figcraft blur box"),
                        layout: fused_layout,
                        entries: &[
                            BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&read) },
                            uniform(0),
                            BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&target) },
                        ],
                    });
                    let (len, lines) = if vertical { (h, w) } else { (w, h) };
                    let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor { label: Some("figcraft blur box"), timestamp_writes: None });
                    pass.set_pipeline(fused);
                    pass.set_bind_group(0, &bind, &[slot]);
                    pass.dispatch_workgroups(len.div_ceil(256), lines.max(1), 1);
                    started = true;
                }
                if !started && let (Some(scan), Some(average), Some(scan_layout), Some(sums)) = (&blur.scan, &blur.average, &blur.scan_layout, sums) {
                    let written = sums.create_view(&TextureViewDescriptor::default());
                    // The running sums along every line, then the window's average from them.
                    let bind = device.create_bind_group(&BindGroupDescriptor {
                        label: Some("figcraft blur scan"),
                        layout: scan_layout,
                        entries: &[
                            BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&read) },
                            uniform(0),
                            BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&written) },
                        ],
                    });
                    {
                        let mut pass =
                            encoder.begin_compute_pass(&ComputePassDescriptor { label: Some("figcraft blur scan"), timestamp_writes: None });
                        pass.set_pipeline(scan);
                        pass.set_bind_group(0, &bind, &[slot]);
                        pass.dispatch_workgroups(if vertical { w } else { h }, 1, 1);
                    }
                    let bind = device.create_bind_group(&BindGroupDescriptor {
                        label: Some("figcraft blur average"),
                        layout: &blur.layout,
                        entries: &[BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&written) }, uniform(0)],
                    });
                    let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                        label: Some("figcraft blur average"),
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
                    pass.set_pipeline(average);
                    pass.set_bind_group(0, &bind, &[slot]);
                    pass.draw(0..3, 0..1);
                    started = true;
                }
                if !started {
                    let Some(direct) = &blur.direct else { return None };
                    let bind = device.create_bind_group(&BindGroupDescriptor {
                        label: Some("figcraft blur"),
                        layout: &blur.layout,
                        entries: &[BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&read) }, uniform(0)],
                    });
                    let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                        label: Some("figcraft blur"),
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
                    pass.set_pipeline(direct);
                    pass.set_bind_group(0, &bind, &[slot]);
                    pass.draw(0..3, 0..1);
                }
                std::mem::swap(&mut current, &mut other);
                read = current.create_view(&TextureViewDescriptor::default());
            }
        }
    }
    // After the last pass the freshly written texture is `current`; the rest goes back after the
    // caller submits.
    let mut scratch = vec![other];
    scratch.extend(own_sums);
    Some((current, scratch))
}

/// Blur the pixels of `src` into a new texture, the same three box passes as [`blur`] (`None`
/// when the device can't): the result never leaves the GPU.
pub(crate) fn blur_texture(device: &Device, queue: &Queue, blur: &Blur, src: &TextureView, (w, h): (u32, u32), sigma: f64) -> Option<Texture> {
    let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor { label: Some("figcraft blur") });
    let (out, scratch) = boxes(device, queue, blur, &mut encoder, src, (0, 0), (w, h), sigma, None)?;
    queue.submit([encoder.finish()]);
    for t in scratch {
        blur.give_back(t);
    }
    Some(out)
}

/// [`blur_texture`] into `encoder`: the result, and the scratch to hand back once the caller has
/// submitted it. A progressive blur's levels and their mix then go in one submission; `sums` lets
/// several passes share one running-sums texture (a run takes its own when given none), and
/// `origin` picks the crop of `src` a level is blurred in (the pixels it is used in, plus its
/// reach).
#[allow(clippy::too_many_arguments)]
pub(crate) fn blur_texture_in(
    device: &Device,
    queue: &Queue,
    blur: &Blur,
    encoder: &mut CommandEncoder,
    src: &TextureView,
    origin: (u32, u32),
    (w, h): (u32, u32),
    sigma: f64,
    sums: Option<&Texture>,
) -> Option<(Texture, Vec<Texture>)> {
    boxes(device, queue, blur, encoder, src, origin, (w, h), sigma, sums)
}

/// Blur `p` in place on the GPU; false when the device can't (the CPU blurs it instead).
pub(crate) fn blur(device: &Device, queue: &Queue, blur: &Blur, p: &mut Pixmap, sigma: f64) -> bool {
    let (w, h) = (p.width(), p.height());
    if w == 0 || h == 0 || p.data().len() < (w as usize).saturating_mul(h as usize) * 4 {
        return false;
    }
    let size = Extent3d { width: w, height: h, depth_or_array_layers: 1 };
    let upload = blur.take(device, "figcraft blur in", (w, h), TextureFormat::Rgba8Unorm, TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST);
    queue.write_texture(
        TexelCopyTextureInfo { texture: &upload, mip_level: 0, origin: Default::default(), aspect: Default::default() },
        p.data(),
        TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * 4), rows_per_image: Some(h) },
        size,
    );
    let source = upload.create_view(&TextureViewDescriptor::default());
    let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor { label: Some("figcraft blur") });
    let Some((done, scratch)) = boxes(device, queue, blur, &mut encoder, &source, (0, 0), (w, h), sigma, None) else { return false };
    // Rows of a read-back buffer must be a multiple of 256 bytes.
    let row = ((w as usize * 4).div_ceil(256)) * 256;
    let readback = device.create_buffer(&BufferDescriptor {
        label: Some("figcraft blur read-back"),
        size: (row as u64).saturating_mul(u64::from(h)),
        usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        TexelCopyTextureInfo { texture: &done, mip_level: 0, origin: Default::default(), aspect: Default::default() },
        TexelCopyBufferInfo {
            buffer: &readback,
            layout: TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row as u32), rows_per_image: Some(h) },
        },
        size,
    );
    queue.submit(Some(encoder.finish()));
    blur.give_back(upload);
    for t in scratch {
        blur.give_back(t);
    }
    blur.give_back(done);
    let slice = readback.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = tx.send(result);
    });
    if device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None }).is_err() || rx.recv().ok().and_then(Result::ok).is_none() {
        return false;
    }
    {
        let Ok(mapped) = slice.get_mapped_range() else { return false };
        let data = p.data_mut();
        for y in 0..h as usize {
            let from = y * row;
            let to = y * w as usize * 4;
            if let (Some(row_bytes), Some(out)) = (mapped.get(from..from + w as usize * 4), data.get_mut(to..to + w as usize * 4)) {
                out.copy_from_slice(row_bytes);
            }
        }
    }
    readback.unmap();
    true
}
