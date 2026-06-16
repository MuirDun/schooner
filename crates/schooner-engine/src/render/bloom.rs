//! Bloom — HDR bright-pass + dual-filter pyramid (COD:AW 2014).
//!
//! Two halves:
//!
//! - [`Bloom`] is the per-scene art instrument — a `World` resource the
//!   game (or the F7 debug cycle) swaps to dial the glow from "faint
//!   highlight" up to "everything glows." It carries no GPU state.
//! - [`BloomPipeline`] is the GPU machinery — three pipelines (prefilter
//!   / downsample / upsample) over a half-resolution mip pyramid. It
//!   builds the blurred bright-pass image; `post.wgsl` composites the
//!   result back into the HDR frame *before* the ACES tonemap, so the
//!   glow rolls into the highlight shoulder the way the 2005–2009 era did.
//!
//! ## Why a half-res pyramid (and not a separable Gaussian)
//!
//! The dual-filter pyramid is a fast, banding-free approximation of a
//! very wide Gaussian: each downsample halves resolution with a 13-tap
//! box, each upsample widens with a 3x3 tent and accumulates additively.
//! N levels of half-res passes cost a fraction of a full-res separable
//! blur of equivalent radius, with no visible banding. It's the modern
//! production standard (Bevy, Unreal, Frostbite); tuned wide + warm +
//! additive (see [`Bloom`] presets and the composite in `post.wgsl`) it
//! reproduces the HL2 / Witcher 1 halation.
//!
//! ## Resize
//!
//! The mip chain depends on the surface size, so [`BloomPipeline::ensure_targets`]
//! rebuilds the textures + per-mip bind groups when the size changes, and
//! rebuilds just the HDR-source bind group when the context's HDR view is
//! recreated (same cadence as [`crate::render::post::PostPipeline`]'s
//! generation tracking). Called once per frame from `render_frame`; the
//! steady state is a size/generation compare and early return.

use bytemuck::{Pod, Zeroable};
use glam::Vec3;
use wgpu::util::{BufferInitDescriptor, DeviceExt};
use wgpu::{
    AddressMode, BindGroup, BindGroupDescriptor, BindGroupEntry, BindGroupLayout,
    BindGroupLayoutDescriptor, BindGroupLayoutEntry, BindingResource, BindingType, BlendComponent,
    BlendFactor, BlendOperation, BlendState, Buffer, BufferBindingType, BufferUsages,
    Color, ColorTargetState, ColorWrites, CommandEncoder, Device, Extent3d, FilterMode,
    FragmentState, FrontFace, LoadOp, MipmapFilterMode, MultisampleState, Operations,
    PipelineLayoutDescriptor, PolygonMode, PrimitiveState, PrimitiveTopology,
    RenderPassColorAttachment, RenderPassDescriptor, RenderPipeline, RenderPipelineDescriptor,
    Sampler, SamplerBindingType, SamplerDescriptor, ShaderModuleDescriptor, ShaderSource,
    ShaderStages, StoreOp, TextureDescriptor, TextureDimension, TextureFormat, TextureSampleType,
    TextureUsages, TextureView, TextureViewDescriptor, TextureViewDimension, VertexState,
};

use std::num::NonZeroU64;

/// Deepest the mip pyramid is allowed to go. 7 half-res levels is a very
/// wide spread (the deepest mip covers most of the screen) — generous for
/// the indoor Kinesis scale and the era's atmospheric glow. The chain
/// stops early when a mip would drop below [`MIN_MIP_DIM`].
const MAX_MIPS: usize = 7;

/// Smallest mip edge before the chain stops growing. Below ~4px the 13-tap
/// kernel starts sampling mostly clamped edge texels and contributes noise,
/// not blur.
const MIN_MIP_DIM: u32 = 4;

/// Per-scene bloom controls — a `World` resource, no GPU state.
///
/// `strength` and `tint` drive the composite in `post.wgsl`; `threshold`,
/// `knee` and `filter_radius` drive the pyramid build in `bloom.wgsl`.
/// `enabled` gates the whole effect: when false the build passes are
/// skipped and the composite contributes nothing (see
/// [`Bloom::effective_strength`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bloom {
    /// Master on/off. Off skips the GPU pyramid build entirely.
    pub enabled: bool,
    /// Composite multiplier — the master "how much glow." Additive onto
    /// the HDR frame, so values are small: ~0.04 is a faint highlight
    /// lift, ~0.4+ is full "everything glows."
    pub strength: f32,
    /// HDR bright-pass cutoff (scene-linear). >= 1.0 blooms only genuinely
    /// over-bright pixels (emissive past 1, hot speculars, the sun term on
    /// white). < 1.0 starts hazing midtones — the aggressive era end.
    pub threshold: f32,
    /// Soft-knee width as a fraction of `threshold`. Smooths the ramp into
    /// the threshold so brightening surfaces fade into bloom instead of
    /// popping.
    pub knee: f32,
    /// Upsample tent spread, in source texels. 1.0 = tight/crisp, 2–3 =
    /// the wide soft era halo.
    pub filter_radius: f32,
    /// Warm tint multiplied into the bloom at composite time. Slightly
    /// amber by default — the Witcher-1 / golden-hour cast. Keep channels
    /// near 1.0 so the tint colours the glow without darkening it.
    pub tint: Vec3,
}

impl Bloom {
    /// Disabled — no glow, build passes skipped. Used as the post-uniform
    /// identity seed and the missing-resource fallback.
    pub const OFF: Self = Self {
        enabled: false,
        strength: 0.0,
        threshold: 1.1,
        knee: 0.5,
        filter_radius: 2.0,
        tint: Vec3::new(1.0, 0.95, 0.85),
    };

    /// Restrained highlight bloom — the shipping default. Honours
    /// `rendering.md`'s "faint highlight bloom, off otherwise": only
    /// over-bright sources glow, gently, with a wide warm falloff.
    pub const FAINT: Self = Self {
        enabled: true,
        strength: 0.04,
        threshold: 1.1,
        knee: 0.5,
        filter_radius: 2.0,
        tint: Vec3::new(1.0, 0.95, 0.85),
    };

    /// Pronounced HL2 / Witcher-1 halation — wider, warmer, a lower
    /// threshold so mid-bright surfaces start to bleed. The "look at the
    /// HDR" Lost Coast feel without going all the way to a white-out.
    pub const ERA_GLOW: Self = Self {
        enabled: true,
        strength: 0.15,
        threshold: 0.9,
        knee: 0.6,
        filter_radius: 2.5,
        tint: Vec3::new(1.0, 0.92, 0.78),
    };

    /// Everything glows — the far end of the dial. Low threshold + high
    /// strength haze the whole frame. Deliberately reachable so the effect
    /// stays a real art instrument; not a default the aesthetic endorses.
    pub const EVERYTHING_GLOWS: Self = Self {
        enabled: true,
        strength: 0.45,
        threshold: 0.5,
        knee: 0.8,
        filter_radius: 3.0,
        tint: Vec3::new(1.0, 0.9, 0.72),
    };

    /// Shipping default — [`Bloom::FAINT`].
    pub const DEFAULT: Self = Self::FAINT;

    /// Composite multiplier folded with the enable gate: 0 when disabled,
    /// so the post shader's `if strength > 0` branch skips the bloom
    /// sample without needing a separate "enabled" uniform.
    pub fn effective_strength(&self) -> f32 {
        if self.enabled { self.strength } else { 0.0 }
    }
}

impl Default for Bloom {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// GPU-shaped build params for `bloom.wgsl`'s `@group(1)`. Packs the
/// three build-time scalars into one vec4 (`w` is std140 padding).
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct BloomParamsUniform {
    /// x = threshold, y = knee, z = filter_radius, w = pad.
    pub params: [f32; 4],
}

impl BloomParamsUniform {
    pub fn from_bloom(bloom: &Bloom) -> Self {
        Self {
            params: [bloom.threshold, bloom.knee, bloom.filter_radius, 0.0],
        }
    }
}

/// All persistent GPU state for the bloom pyramid.
///
/// Inserted as a resource alongside the other pipelines. Reads the HDR
/// target from `RenderContext`, builds the pyramid into its own mip
/// texture, and exposes mip 0 ([`BloomPipeline::mip0_view`]) for the post
/// pass to composite.
pub struct BloomPipeline {
    prefilter: RenderPipeline,
    downsample: RenderPipeline,
    upsample: RenderPipeline,
    /// BGL for `@group(0)` — source mip (or the HDR target) + linear sampler.
    source_bgl: BindGroupLayout,
    /// Build-params uniform buffer; written from [`Bloom`] each frame.
    pub params_buffer: Buffer,
    /// Bind group for `@group(1)` — stable, built once.
    params_bind_group: BindGroup,
    /// Linear-clamp sampler. Linear is mandatory: the 13-tap downsample
    /// relies on each tap being a bilinear 2x2 average. Clamp avoids the
    /// glow wrapping around screen edges.
    sampler: Sampler,
    /// Pyramid texture format — the same HDR format the pipelines target,
    /// kept so `rebuild_chain` can't drift from the pipeline color state.
    format: TextureFormat,

    // --- size-dependent chain, rebuilt by `ensure_targets` -------------
    /// One render-target view per mip level (single-mip views).
    mip_views: Vec<TextureView>,
    /// One `@group(0)` source bind group per mip level — `mip_bind_groups[k]`
    /// binds mip `k` as the sampled source. Downsample writing mip `i`
    /// reads `[i-1]`; upsample writing mip `i-1` reads `[i]`.
    mip_bind_groups: Vec<BindGroup>,
    /// `@group(0)` bind group binding the full-res HDR target — the
    /// prefilter's source. `None` until the first `ensure_targets`.
    hdr_bind_group: Option<BindGroup>,
    cached_size: (u32, u32),
    cached_hdr_generation: u64,
}

impl BloomPipeline {
    /// Build the three pipelines and the stable (size-independent) state.
    /// `hdr_format` is the offscreen target format (must match
    /// [`crate::render::context::HDR_FORMAT`]) — the pyramid renders in
    /// the same HDR-linear space.
    pub fn new(device: &Device, hdr_format: TextureFormat) -> Self {
        let source_bgl = create_source_bgl(device);
        let params_bgl = create_params_bgl(device);

        let params_buffer = device.create_buffer_init(&BufferInitDescriptor {
            label: Some("bloom-params-buffer"),
            contents: bytemuck::bytes_of(&BloomParamsUniform::from_bloom(&Bloom::DEFAULT)),
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
        });
        let params_bind_group = device.create_bind_group(&BindGroupDescriptor {
            label: Some("bloom-params-bind-group"),
            layout: &params_bgl,
            entries: &[BindGroupEntry {
                binding: 0,
                resource: params_buffer.as_entire_binding(),
            }],
        });

        let layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("bloom-pipeline-layout"),
            bind_group_layouts: &[Some(&source_bgl), Some(&params_bgl)],
            immediate_size: 0,
        });

        let shader = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("bloom-shader"),
            source: ShaderSource::Wgsl(include_str!("../../shaders/bloom.wgsl").into()),
        });

        // Prefilter and downsample overwrite their target; upsample
        // accumulates onto the destination mip's existing downsampled
        // content, so it blends additively (src*1 + dst*1).
        let prefilter = make_pipeline(
            device, &layout, &shader, "fs_prefilter", hdr_format, BlendState::REPLACE,
            "bloom-prefilter",
        );
        let downsample = make_pipeline(
            device, &layout, &shader, "fs_downsample", hdr_format, BlendState::REPLACE,
            "bloom-downsample",
        );
        let additive = BlendState {
            color: BlendComponent {
                src_factor: BlendFactor::One,
                dst_factor: BlendFactor::One,
                operation: BlendOperation::Add,
            },
            alpha: BlendComponent {
                src_factor: BlendFactor::One,
                dst_factor: BlendFactor::One,
                operation: BlendOperation::Add,
            },
        };
        let upsample = make_pipeline(
            device, &layout, &shader, "fs_upsample", hdr_format, additive, "bloom-upsample",
        );

        let sampler = device.create_sampler(&SamplerDescriptor {
            label: Some("bloom-sampler"),
            address_mode_u: AddressMode::ClampToEdge,
            address_mode_v: AddressMode::ClampToEdge,
            address_mode_w: AddressMode::ClampToEdge,
            mag_filter: FilterMode::Linear,
            min_filter: FilterMode::Linear,
            // Each pass samples a single-mip view, so LOD never moves —
            // mipmap filter is unreachable.
            mipmap_filter: MipmapFilterMode::Nearest,
            lod_min_clamp: 0.0,
            lod_max_clamp: 0.0,
            compare: None,
            anisotropy_clamp: 1,
            border_color: None,
        });

        Self {
            prefilter,
            downsample,
            upsample,
            source_bgl,
            params_buffer,
            params_bind_group,
            sampler,
            format: hdr_format,
            mip_views: Vec::new(),
            mip_bind_groups: Vec::new(),
            hdr_bind_group: None,
            // (0,0) so the first call always rebuilds the chain; MAX so
            // the HDR group also rebuilds on that first call.
            cached_size: (0, 0),
            cached_hdr_generation: u64::MAX,
        }
    }

    /// Rebuild the mip chain when the surface size changes, and the
    /// HDR-source bind group when the context's HDR view is recreated.
    /// Both triggers fire together on resize; tracked separately so a
    /// future HDR-only recreation still refreshes the prefilter source.
    pub fn ensure_targets(
        &mut self,
        device: &Device,
        width: u32,
        height: u32,
        hdr_view: &TextureView,
        hdr_generation: u64,
    ) {
        if (width, height) != self.cached_size || self.mip_views.is_empty() {
            self.rebuild_chain(device, width, height);
            self.cached_size = (width, height);
            // The mip-0 view changed, so the prefilter source must rebind
            // too — fold the HDR-group rebuild in unconditionally here.
            self.rebuild_hdr_group(device, hdr_view);
            self.cached_hdr_generation = hdr_generation;
        } else if self.cached_hdr_generation != hdr_generation || self.hdr_bind_group.is_none() {
            self.rebuild_hdr_group(device, hdr_view);
            self.cached_hdr_generation = hdr_generation;
        }
    }

    fn rebuild_chain(&mut self, device: &Device, width: u32, height: u32) {
        let dims = mip_chain_dims(width, height);
        let texture = device.create_texture(&TextureDescriptor {
            label: Some("bloom-pyramid"),
            size: Extent3d {
                width: dims[0].0,
                height: dims[0].1,
                depth_or_array_layers: 1,
            },
            mip_level_count: dims.len() as u32,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: self.format,
            usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });

        // Single-mip views: each is both a render target (for its own
        // level) and a sampled source (for the adjacent level). The views
        // keep the texture alive — same ownership pattern as the depth /
        // HDR attachments, no separate Texture handle retained.
        let mut mip_views = Vec::with_capacity(dims.len());
        let mut mip_bind_groups = Vec::with_capacity(dims.len());
        for level in 0..dims.len() as u32 {
            let view = texture.create_view(&TextureViewDescriptor {
                label: Some("bloom-mip"),
                base_mip_level: level,
                mip_level_count: Some(1),
                ..Default::default()
            });
            let bind_group = device.create_bind_group(&BindGroupDescriptor {
                label: Some("bloom-mip-bind-group"),
                layout: &self.source_bgl,
                entries: &[
                    BindGroupEntry {
                        binding: 0,
                        resource: BindingResource::TextureView(&view),
                    },
                    BindGroupEntry {
                        binding: 1,
                        resource: BindingResource::Sampler(&self.sampler),
                    },
                ],
            });
            mip_views.push(view);
            mip_bind_groups.push(bind_group);
        }
        self.mip_views = mip_views;
        self.mip_bind_groups = mip_bind_groups;
    }

    fn rebuild_hdr_group(&mut self, device: &Device, hdr_view: &TextureView) {
        self.hdr_bind_group = Some(device.create_bind_group(&BindGroupDescriptor {
            label: Some("bloom-hdr-bind-group"),
            layout: &self.source_bgl,
            entries: &[
                BindGroupEntry {
                    binding: 0,
                    resource: BindingResource::TextureView(hdr_view),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: BindingResource::Sampler(&self.sampler),
                },
            ],
        }));
    }

    /// Mip 0 of the pyramid — the accumulated bloom the post pass samples.
    /// Valid only after [`ensure_targets`](Self::ensure_targets) has run
    /// at a non-zero size.
    pub fn mip0_view(&self) -> &TextureView {
        self.mip_views.first().expect("bloom chain built")
    }

    /// Record the full pyramid into `encoder`: prefilter → downsample
    /// chain → additive upsample chain. Assumes [`ensure_targets`] has run
    /// this frame and the params buffer is up to date. Caller gates this
    /// on `Bloom::enabled`.
    ///
    /// [`ensure_targets`]: Self::ensure_targets
    pub fn record(&self, encoder: &mut CommandEncoder) {
        let n = self.mip_views.len();
        if n == 0 {
            return;
        }

        // Prefilter: HDR full-res → mip 0 (threshold + Karis), overwrite.
        let hdr_group = self.hdr_bind_group.as_ref().expect("hdr group built");
        self.pass(encoder, "bloom-prefilter", &self.prefilter, &self.mip_views[0], hdr_group, false);

        // Downsample: mip i-1 → mip i, overwrite, walking down the chain.
        for i in 1..n {
            self.pass(
                encoder, "bloom-downsample", &self.downsample, &self.mip_views[i],
                &self.mip_bind_groups[i - 1], false,
            );
        }

        // Upsample: mip i → mip i-1, additive, walking back up. The
        // destination already holds its downsampled content (Load), and
        // the pipeline's additive blend accumulates the widened lower mip
        // onto it. After this, mip 0 holds the full bloom.
        for i in (1..n).rev() {
            self.pass(
                encoder, "bloom-upsample", &self.upsample, &self.mip_views[i - 1],
                &self.mip_bind_groups[i], true,
            );
        }
    }

    /// One fullscreen-triangle pass. `load` preserves the target's
    /// existing content (for the additive upsample); otherwise it clears.
    fn pass(
        &self,
        encoder: &mut CommandEncoder,
        label: &str,
        pipeline: &RenderPipeline,
        target: &TextureView,
        source: &BindGroup,
        load: bool,
    ) {
        let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some(label),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: Operations {
                    load: if load {
                        LoadOp::Load
                    } else {
                        LoadOp::Clear(Color::BLACK)
                    },
                    store: StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            multiview_mask: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, source, &[]);
        pass.set_bind_group(1, &self.params_bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}

/// Half-res-and-down mip dimensions, mip 0 first. Each level halves (floor)
/// — matching the GPU's automatic mip sizing, so a single-mip view at
/// level `i` has exactly `dims[i]`. Stops at [`MAX_MIPS`] or once an edge
/// reaches [`MIN_MIP_DIM`].
fn mip_chain_dims(width: u32, height: u32) -> Vec<(u32, u32)> {
    let mut dims = Vec::with_capacity(MAX_MIPS);
    let mut w = (width / 2).max(1);
    let mut h = (height / 2).max(1);
    for _ in 0..MAX_MIPS {
        dims.push((w, h));
        if w <= MIN_MIP_DIM || h <= MIN_MIP_DIM {
            break;
        }
        w = (w / 2).max(1);
        h = (h / 2).max(1);
    }
    dims
}

fn create_source_bgl(device: &Device) -> BindGroupLayout {
    device.create_bind_group_layout(&BindGroupLayoutDescriptor {
        label: Some("bloom-source-bgl"),
        entries: &[
            BindGroupLayoutEntry {
                binding: 0,
                visibility: ShaderStages::FRAGMENT,
                ty: BindingType::Texture {
                    sample_type: TextureSampleType::Float { filterable: true },
                    view_dimension: TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            BindGroupLayoutEntry {
                binding: 1,
                visibility: ShaderStages::FRAGMENT,
                ty: BindingType::Sampler(SamplerBindingType::Filtering),
                count: None,
            },
        ],
    })
}

fn create_params_bgl(device: &Device) -> BindGroupLayout {
    device.create_bind_group_layout(&BindGroupLayoutDescriptor {
        label: Some("bloom-params-bgl"),
        entries: &[BindGroupLayoutEntry {
            binding: 0,
            visibility: ShaderStages::FRAGMENT,
            ty: BindingType::Buffer {
                ty: BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: NonZeroU64::new(std::mem::size_of::<BloomParamsUniform>() as u64),
            },
            count: None,
        }],
    })
}

fn make_pipeline(
    device: &Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    fs_entry: &str,
    format: TextureFormat,
    blend: BlendState,
    label: &str,
) -> RenderPipeline {
    device.create_render_pipeline(&RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
        vertex: VertexState {
            module: shader,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        fragment: Some(FragmentState {
            module: shader,
            entry_point: Some(fs_entry),
            compilation_options: Default::default(),
            targets: &[Some(ColorTargetState {
                format,
                blend: Some(blend),
                write_mask: ColorWrites::ALL,
            })],
        }),
        primitive: PrimitiveState {
            topology: PrimitiveTopology::TriangleList,
            strip_index_format: None,
            front_face: FrontFace::Ccw,
            cull_mode: None,
            unclipped_depth: false,
            polygon_mode: PolygonMode::Fill,
            conservative: false,
        },
        depth_stencil: None,
        multisample: MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}
