//! Auto-exposure — eye adaptation via average-luminance metering.
//!
//! Two halves, same split as [`crate::render::bloom`]:
//!
//! - [`AutoExposure`] is the per-scene art instrument — a `World` resource
//!   the game swaps to tune the adaptation. It carries no GPU state.
//! - [`ExposurePipeline`] is the GPU machinery — a log-luminance mip
//!   reduction down to a single 1x1 mean, plus a temporally-adapted exposure
//!   scalar held in a ping-pong pair of 1x1 textures. `post.wgsl` samples the
//!   current exposure at `@group(4)` and multiplies it into the HDR frame
//!   *before* the ACES tonemap.
//!
//! ## Why this exists
//!
//! Lambertian surfaces are view-independent: a wall's lit radiance does not
//! change when the camera moves. What *does* change is how the eye adapts to
//! the average brightness in view — look at a bright lamp and the iris stops
//! down, crushing the dim corners to black. That adaptation is an *exposure*
//! stage, not a material or fog effect, and it is the missing piece that
//! makes "look at the light → the darks get darker" read correctly. With the
//! god-ray in-scatter feeding the luminance metric, facing the cone raises
//! the average and the exposure drops on its own.
//!
//! ## Pipeline
//!
//! 1. **Prefilter** HDR -> log-luminance into luma mip 0 (half-res).
//! 2. **Downsample** the luma chain to 1x1 — the geometric-mean log-luma of
//!    the frame (Reinhard's key-value method; the log average is the mean of
//!    a multiplicative quantity).
//! 3. **Adapt** — read the 1x1 mean and the previous exposure, ease toward
//!    `key / luma` with a direction-dependent time constant, write the new
//!    exposure into the other ping-pong texture.
//!
//! ## Resize
//!
//! The luma chain depends on the surface size, so [`ExposurePipeline::ensure_targets`]
//! rebuilds it (and the HDR-source + adapt bind groups) when the size or the
//! context's HDR generation changes — the same cadence as
//! [`crate::render::bloom::BloomPipeline`]. The 1x1 exposure textures are
//! size-independent and live for the pipeline's lifetime.

use bytemuck::{Pod, Zeroable};
use wgpu::util::{BufferInitDescriptor, DeviceExt};
use wgpu::{
    AddressMode, BindGroup, BindGroupDescriptor, BindGroupEntry, BindGroupLayout,
    BindGroupLayoutDescriptor, BindGroupLayoutEntry, BindingResource, BindingType, BlendState,
    Buffer, BufferBindingType, BufferUsages, Color, ColorTargetState, ColorWrites, CommandEncoder,
    Device, Extent3d, FilterMode, FragmentState, FrontFace, LoadOp, MipmapFilterMode,
    MultisampleState, Operations, PipelineLayoutDescriptor, PolygonMode, PrimitiveState,
    PrimitiveTopology, RenderPassColorAttachment, RenderPassDescriptor, RenderPipeline,
    RenderPipelineDescriptor, Sampler, SamplerBindingType, SamplerDescriptor, ShaderModuleDescriptor,
    ShaderSource, ShaderStages, StoreOp, TextureDescriptor, TextureDimension, TextureFormat,
    TextureSampleType, TextureUsages, TextureView, TextureViewDescriptor, TextureViewDimension,
    VertexState,
};

use std::num::NonZeroU64;

/// Two-channel float format for the luma reduction chain: r = luma, g =
/// luma². Both are filterable (needed for the bilinear downsample taps).
const LUMA_FORMAT: TextureFormat = TextureFormat::Rg16Float;

/// Single-channel float format for the 1x1 exposure ping-pong textures.
const EXPOSURE_FORMAT: TextureFormat = TextureFormat::R16Float;

/// Safety cap on the luma reduction chain length. A 16-level chain reaches
/// 1x1 from a 32k-pixel edge, far beyond any real surface; the loop stops at
/// 1x1 well before this.
const MAX_LUMA_MIPS: usize = 16;

/// Per-scene auto-exposure controls — a `World` resource, no GPU state.
///
/// The exposure the pipeline converges to is `key / clamp(scene_luma,
/// min_luma, max_luma)`, itself clamped to `[min_exposure, max_exposure]`,
/// reached over time at `speed_brighten` / `speed_darken` (1/seconds).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AutoExposure {
    /// Master on/off. When false the adapt shader emits exposure = 1.0, so
    /// the post multiply is an exact no-op (the cheap reduction still runs;
    /// the value is just forced to identity).
    pub enabled: bool,
    /// Middle-grey target the metered luminance is mapped to. Lower = darker
    /// overall image (the eye is "less open"); ~0.18 is the photographic
    /// 18% grey, but interiors often want less so the dark stays dark.
    pub key: f32,
    /// Lower clamp on metered scene luminance. Guards the `key / luma`
    /// divide and caps how far the exposure opens up in near-black views —
    /// raise it to keep the void from brightening when you stare into it.
    pub min_luma: f32,
    /// Upper clamp on metered scene luminance. Caps how far the exposure
    /// stops down when a very bright source fills the view.
    pub max_luma: f32,
    /// Floor on the exposure scalar — how dark the image is allowed to get
    /// when stopping down. Above 0 so the darks crush but never vanish,
    /// echoing the tone curve's "no true black" intent.
    pub min_exposure: f32,
    /// Ceiling on the exposure scalar — how far the image brightens in dark
    /// views. Keep modest for a gloomy room so shadows don't wash out.
    pub max_exposure: f32,
    /// Adaptation rate (1/seconds) when the eye opens up (scene got darker,
    /// exposure rising). Slow — real dark adaptation takes seconds.
    pub speed_brighten: f32,
    /// Adaptation rate (1/seconds) when the eye stops down (scene got
    /// brighter, exposure falling). Faster — squinting at a light is quick.
    pub speed_darken: f32,
}

impl AutoExposure {
    /// Disabled — exposure forced to 1.0, post multiply is a no-op. The
    /// missing-resource fallback and the params-buffer seed.
    pub const OFF: Self = Self {
        enabled: false,
        key: 0.12,
        min_luma: 0.03,
        max_luma: 6.0,
        min_exposure: 0.35,
        max_exposure: 1.6,
        speed_brighten: 1.2,
        speed_darken: 5.0,
    };

    /// General interior default — gentle adaptation that lets dark areas
    /// breathe a little while clamping the bright end so a lamp in view
    /// stops the exposure down.
    pub const DEFAULT: Self = Self {
        enabled: true,
        ..Self::OFF
    };

    /// Tuned for the Kinesis chamber: a low key and a tight exposure ceiling
    /// keep the room gloomy, while a fast `speed_darken` means facing the
    /// lamp (or its god-ray) crushes the surrounding darks quickly — the
    /// "look at the light and the shadows deepen" read.
    pub const CHAMBER: Self = Self {
        enabled: true,
        // Roughly the centre-weighted luminance that maps to exposure 1.0.
        // Lower => the room reveals/brightens more in the dark and crushes
        // sooner when light enters; raise if the adapted dark reads too
        // bright.
        key: 0.15,
        min_luma: 0.005,
        max_luma: 8.0,
        // Crush floor — facing the lamp drops the room toward black (you're
        // blinded) while the un-exposed bloom halo stays searing.
        min_exposure: 0.2,
        // Ceiling well ABOVE 1.0 so the eye opens up in the dark: stand away
        // from the light and over a couple of seconds the walls reveal. This
        // is the "see more once your eyes adjust" half of eye adaptation.
        // Lower toward 1.5 to keep the dark gloomier; raise for a stronger
        // reveal.
        max_exposure: 3.0,
        // Asymmetric, like a real eye: opening up in the dark is SLOW (the
        // reveal takes a couple seconds — the immersive beat), stopping down
        // when light hits is FAST (the blind is near-instant).
        speed_brighten: 0.5,
        speed_darken: 6.0,
    };
}

impl Default for AutoExposure {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// GPU-shaped adapt params for `exposure.wgsl`'s `@group(1)`. Three vec4s,
/// std140-aligned; mirrors `AdaptParams` in the shader.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct AdaptParamsUniform {
    /// x = key, y = min_luma, z = max_luma, w = dt (seconds).
    pub a: [f32; 4],
    /// x = min_exposure, y = max_exposure, z = speed_brighten, w = speed_darken.
    pub b: [f32; 4],
    /// x = enabled (0/1), yzw padding.
    pub c: [f32; 4],
}

impl AdaptParamsUniform {
    pub fn new(exposure: &AutoExposure, dt: f32) -> Self {
        Self {
            a: [exposure.key, exposure.min_luma, exposure.max_luma, dt],
            b: [
                exposure.min_exposure,
                exposure.max_exposure,
                exposure.speed_brighten,
                exposure.speed_darken,
            ],
            c: [if exposure.enabled { 1.0 } else { 0.0 }, 0.0, 0.0, 0.0],
        }
    }
}

/// All persistent GPU state for auto-exposure.
///
/// Reads the HDR target from `RenderContext`, reduces it to a 1x1 mean
/// luminance, and maintains a ping-pong pair of 1x1 exposure textures.
/// [`ExposurePipeline::current_exposure_view`] is what the post pass binds.
pub struct ExposurePipeline {
    prefilter: RenderPipeline,
    downsample: RenderPipeline,
    adapt: RenderPipeline,
    /// BGL for the reduction passes' `@group(0)` — source texture + sampler
    /// (2 entries; the prefilter/downsample shaders never touch binding 2).
    reduction_bgl: BindGroupLayout,
    /// BGL for the adapt pass' `@group(0)` — 1x1 mean-luma + sampler + the
    /// previous exposure (3 entries).
    adapt_bgl: BindGroupLayout,
    /// Adapt params uniform buffer; written from [`AutoExposure`] each frame.
    pub params_buffer: Buffer,
    /// `@group(1)` bind group for the adapt params — stable, built once.
    params_bind_group: BindGroup,
    /// Linear-clamp sampler. Linear is mandatory for the downsample's 2x2
    /// bilinear averaging; clamp keeps edge taps in-bounds on the tiny mips.
    sampler: Sampler,

    /// Ping-pong exposure textures (1x1). `exposure_views[parity]` holds the
    /// latest exposure (what the post pass reads); the adapt pass writes into
    /// `exposure_views[1 - parity]` reading `[parity]` as the previous value.
    exposure_views: [TextureView; 2],
    parity: usize,

    // --- size-dependent chain, rebuilt by `ensure_targets` -------------
    /// One render-target / source view per luma mip level (single-mip views).
    luma_views: Vec<TextureView>,
    /// `@group(0)` reduction bind group per mip — `luma_bind_groups[k]` binds
    /// mip `k` as the sampled source for the downsample writing mip `k+1`.
    luma_bind_groups: Vec<BindGroup>,
    /// `@group(0)` reduction bind group binding the full-res HDR target — the
    /// prefilter's source. `None` until the first `ensure_targets`.
    hdr_bind_group: Option<BindGroup>,
    /// `@group(0)` adapt bind groups, indexed by which exposure texture is
    /// the *previous* one: `adapt_bind_groups[p]` binds the 1x1 mean-luma
    /// mip + `exposure_views[p]`. Rebuilt whenever the luma chain (and thus
    /// its 1x1 view) is rebuilt. `None` until the first `ensure_targets`.
    adapt_bind_groups: [Option<BindGroup>; 2],
    cached_size: (u32, u32),
    cached_hdr_generation: u64,
}

impl ExposurePipeline {
    /// Build the three pipelines, the stable bind groups, and the 1x1
    /// exposure ping-pong. The prefilter samples the HDR target through a
    /// format-agnostic bind group, so no HDR format is needed here.
    pub fn new(device: &Device) -> Self {
        let reduction_bgl = create_reduction_bgl(device);
        let adapt_bgl = create_adapt_bgl(device);
        let params_bgl = create_params_bgl(device);

        let params_buffer = device.create_buffer_init(&BufferInitDescriptor {
            label: Some("exposure-params-buffer"),
            // Seed disabled so the very first frame (before the per-frame
            // write) forces exposure = 1.0 rather than adapting from noise.
            contents: bytemuck::bytes_of(&AdaptParamsUniform::new(&AutoExposure::OFF, 0.0)),
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
        });
        let params_bind_group = device.create_bind_group(&BindGroupDescriptor {
            label: Some("exposure-params-bind-group"),
            layout: &params_bgl,
            entries: &[BindGroupEntry {
                binding: 0,
                resource: params_buffer.as_entire_binding(),
            }],
        });

        let reduction_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("exposure-reduction-layout"),
            bind_group_layouts: &[Some(&reduction_bgl)],
            immediate_size: 0,
        });
        let adapt_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("exposure-adapt-layout"),
            bind_group_layouts: &[Some(&adapt_bgl), Some(&params_bgl)],
            immediate_size: 0,
        });

        let shader = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("exposure-shader"),
            source: ShaderSource::Wgsl(include_str!("../../shaders/exposure.wgsl").into()),
        });

        let prefilter = make_pipeline(
            device,
            &reduction_layout,
            &shader,
            "fs_prefilter",
            LUMA_FORMAT,
            "exposure-prefilter",
        );
        let downsample = make_pipeline(
            device,
            &reduction_layout,
            &shader,
            "fs_downsample",
            LUMA_FORMAT,
            "exposure-downsample",
        );
        let adapt = make_pipeline(
            device,
            &adapt_layout,
            &shader,
            "fs_adapt",
            EXPOSURE_FORMAT,
            "exposure-adapt",
        );

        let sampler = device.create_sampler(&SamplerDescriptor {
            label: Some("exposure-sampler"),
            address_mode_u: AddressMode::ClampToEdge,
            address_mode_v: AddressMode::ClampToEdge,
            address_mode_w: AddressMode::ClampToEdge,
            mag_filter: FilterMode::Linear,
            min_filter: FilterMode::Linear,
            mipmap_filter: MipmapFilterMode::Nearest,
            lod_min_clamp: 0.0,
            lod_max_clamp: 0.0,
            compare: None,
            anisotropy_clamp: 1,
            border_color: None,
        });

        // Two 1x1 exposure textures. Zero-initialised by wgpu; the adapt
        // shader clamps the previous value up to `min_exposure`, so frame 0
        // adapts from a sane floor rather than from 0.
        let make_exposure = |label: &str| {
            let tex = device.create_texture(&TextureDescriptor {
                label: Some(label),
                size: Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format: EXPOSURE_FORMAT,
                usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            tex.create_view(&TextureViewDescriptor::default())
        };
        let exposure_views = [make_exposure("exposure-a"), make_exposure("exposure-b")];

        Self {
            prefilter,
            downsample,
            adapt,
            reduction_bgl,
            adapt_bgl,
            params_buffer,
            params_bind_group,
            sampler,
            exposure_views,
            parity: 0,
            luma_views: Vec::new(),
            luma_bind_groups: Vec::new(),
            hdr_bind_group: None,
            adapt_bind_groups: [None, None],
            cached_size: (0, 0),
            cached_hdr_generation: u64::MAX,
        }
    }

    /// Rebuild the luma chain (and the HDR-source + adapt bind groups) when
    /// the surface size changes or the context's HDR view is recreated.
    pub fn ensure_targets(
        &mut self,
        device: &Device,
        width: u32,
        height: u32,
        hdr_view: &TextureView,
        hdr_generation: u64,
    ) {
        if (width, height) != self.cached_size || self.luma_views.is_empty() {
            self.rebuild_chain(device, width, height);
            self.cached_size = (width, height);
            // Both the prefilter source (HDR view) and the adapt source (the
            // chain's new 1x1 view) must rebind after a chain rebuild.
            self.rebuild_hdr_group(device, hdr_view);
            self.rebuild_adapt_groups(device);
            self.cached_hdr_generation = hdr_generation;
        } else if self.cached_hdr_generation != hdr_generation || self.hdr_bind_group.is_none() {
            self.rebuild_hdr_group(device, hdr_view);
            self.cached_hdr_generation = hdr_generation;
        }
    }

    fn rebuild_chain(&mut self, device: &Device, width: u32, height: u32) {
        let dims = luma_chain_dims(width, height);
        let texture = device.create_texture(&TextureDescriptor {
            label: Some("exposure-luma-pyramid"),
            size: Extent3d {
                width: dims[0].0,
                height: dims[0].1,
                depth_or_array_layers: 1,
            },
            mip_level_count: dims.len() as u32,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: LUMA_FORMAT,
            usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });

        let mut luma_views = Vec::with_capacity(dims.len());
        let mut luma_bind_groups = Vec::with_capacity(dims.len());
        for level in 0..dims.len() as u32 {
            let view = texture.create_view(&TextureViewDescriptor {
                label: Some("exposure-luma-mip"),
                base_mip_level: level,
                mip_level_count: Some(1),
                ..Default::default()
            });
            let bind_group = device.create_bind_group(&BindGroupDescriptor {
                label: Some("exposure-luma-bind-group"),
                layout: &self.reduction_bgl,
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
            luma_views.push(view);
            luma_bind_groups.push(bind_group);
        }
        self.luma_views = luma_views;
        self.luma_bind_groups = luma_bind_groups;
    }

    fn rebuild_hdr_group(&mut self, device: &Device, hdr_view: &TextureView) {
        self.hdr_bind_group = Some(device.create_bind_group(&BindGroupDescriptor {
            label: Some("exposure-hdr-bind-group"),
            layout: &self.reduction_bgl,
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

    /// Build both adapt source bind groups. `adapt_bind_groups[p]` binds the
    /// 1x1 mean-luma mip (last in the chain) plus `exposure_views[p]` as the
    /// previous exposure. Depends on the chain's 1x1 view, so it is rebuilt
    /// alongside `rebuild_chain`.
    fn rebuild_adapt_groups(&mut self, device: &Device) {
        let luma_1x1 = self.luma_views.last().expect("luma chain built");
        for p in 0..2 {
            self.adapt_bind_groups[p] = Some(device.create_bind_group(&BindGroupDescriptor {
                label: Some("exposure-adapt-bind-group"),
                layout: &self.adapt_bgl,
                entries: &[
                    BindGroupEntry {
                        binding: 0,
                        resource: BindingResource::TextureView(luma_1x1),
                    },
                    BindGroupEntry {
                        binding: 1,
                        resource: BindingResource::Sampler(&self.sampler),
                    },
                    BindGroupEntry {
                        binding: 2,
                        resource: BindingResource::TextureView(&self.exposure_views[p]),
                    },
                ],
            }));
        }
    }

    /// The exposure texture the post pass should sample this frame — the one
    /// the most recent [`record`](Self::record) wrote. Valid after
    /// [`ensure_targets`](Self::ensure_targets) + [`record`](Self::record).
    pub fn current_exposure_view(&self) -> &TextureView {
        &self.exposure_views[self.parity]
    }

    /// Record the reduction + adapt passes into `encoder`. Assumes
    /// [`ensure_targets`](Self::ensure_targets) ran this frame and the params
    /// buffer is up to date. Advances the ping-pong parity, so
    /// [`current_exposure_view`](Self::current_exposure_view) afterwards
    /// returns the freshly-written exposure.
    pub fn record(&mut self, encoder: &mut CommandEncoder) {
        let n = self.luma_views.len();
        if n == 0 {
            return;
        }

        // Prefilter: HDR full-res -> luma mip 0 (log-luminance).
        let hdr_group = self.hdr_bind_group.as_ref().expect("hdr group built");
        self.reduce_pass(encoder, "exposure-prefilter", &self.prefilter, &self.luma_views[0], hdr_group);

        // Downsample: mip i-1 -> mip i, walking down to the 1x1 mean.
        for i in 1..n {
            self.reduce_pass(
                encoder,
                "exposure-downsample",
                &self.downsample,
                &self.luma_views[i],
                &self.luma_bind_groups[i - 1],
            );
        }

        // Adapt: read the 1x1 mean + the previous exposure (parity), write
        // the new exposure into the other texture (1 - parity).
        let prev = self.parity;
        let next = 1 - self.parity;
        let adapt_group = self.adapt_bind_groups[prev].as_ref().expect("adapt group built");
        {
            let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                label: Some("exposure-adapt"),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: &self.exposure_views[next],
                    depth_slice: None,
                    resolve_target: None,
                    ops: Operations {
                        // 1x1 fully overwritten; Clear is the cheap path.
                        load: LoadOp::Clear(Color::BLACK),
                        store: StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                multiview_mask: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.adapt);
            pass.set_bind_group(0, adapt_group, &[]);
            pass.set_bind_group(1, &self.params_bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        self.parity = next;
    }

    /// One fullscreen-triangle reduction pass writing `target` from `source`.
    fn reduce_pass(
        &self,
        encoder: &mut CommandEncoder,
        label: &str,
        pipeline: &RenderPipeline,
        target: &TextureView,
        source: &BindGroup,
    ) {
        let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some(label),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: Operations {
                    load: LoadOp::Clear(Color::BLACK),
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
        pass.draw(0..3, 0..1);
    }
}

/// Half-res-and-down mip dimensions, mip 0 first, continuing all the way to
/// 1x1 (unlike the bloom chain, which stops early — the exposure metric needs
/// the single-texel mean). Each level halves (floor), matching the GPU's mip
/// sizing so a single-mip view at level `i` is exactly `dims[i]`.
fn luma_chain_dims(width: u32, height: u32) -> Vec<(u32, u32)> {
    let mut dims = Vec::with_capacity(MAX_LUMA_MIPS);
    let mut w = (width / 2).max(1);
    let mut h = (height / 2).max(1);
    for _ in 0..MAX_LUMA_MIPS {
        dims.push((w, h));
        if w == 1 && h == 1 {
            break;
        }
        w = (w / 2).max(1);
        h = (h / 2).max(1);
    }
    dims
}

fn create_reduction_bgl(device: &Device) -> BindGroupLayout {
    device.create_bind_group_layout(&BindGroupLayoutDescriptor {
        label: Some("exposure-reduction-bgl"),
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

fn create_adapt_bgl(device: &Device) -> BindGroupLayout {
    device.create_bind_group_layout(&BindGroupLayoutDescriptor {
        label: Some("exposure-adapt-bgl"),
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
            BindGroupLayoutEntry {
                binding: 2,
                visibility: ShaderStages::FRAGMENT,
                ty: BindingType::Texture {
                    sample_type: TextureSampleType::Float { filterable: true },
                    view_dimension: TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
        ],
    })
}

fn create_params_bgl(device: &Device) -> BindGroupLayout {
    device.create_bind_group_layout(&BindGroupLayoutDescriptor {
        label: Some("exposure-params-bgl"),
        entries: &[BindGroupLayoutEntry {
            binding: 0,
            visibility: ShaderStages::FRAGMENT,
            ty: BindingType::Buffer {
                ty: BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: NonZeroU64::new(std::mem::size_of::<AdaptParamsUniform>() as u64),
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
                blend: Some(BlendState::REPLACE),
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
