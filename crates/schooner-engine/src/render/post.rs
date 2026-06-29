//! Post-process pipeline — the single fixed pass that turns the
//! forward shader's HDR linear output into the final swap-chain image.
//!
//! Game 1.D builds this up in stages. 1.D.1 + 1.D.2 (shipped
//! together) landed the plumbing — HDR target sampling, fullscreen
//! triangle, swap-chain write — plus the Narkowicz ACES tonemap in
//! the fragment shader. 1.D.3 adds color grade; 1.D.4 vignette;
//! 1.D.5 overlay. The pipeline, bind groups, and resource shape
//! stay constant across those Steps.
//!
//! ## Why a cached, generation-tracked bind group
//!
//! The bind group references the HDR texture view that lives in
//! `RenderContext`. Surface resize recreates the HDR view, which
//! invalidates the bind group. Rather than rebuild it every frame
//! unconditionally (wasteful, even though bind group creation is
//! cheap) or move it onto `RenderContext` (mixes pipeline state into
//! the device resource), we cache the bind group here and bump a
//! generation counter on the context. `ensure_bind_group` lazily
//! rebuilds when the cached generation no longer matches.

use std::num::NonZeroU64;

use wgpu::util::{BufferInitDescriptor, DeviceExt};
use wgpu::{
    AddressMode, BindGroup, BindGroupDescriptor, BindGroupEntry, BindGroupLayout,
    BindGroupLayoutDescriptor, BindGroupLayoutEntry, BindingResource, BindingType, BlendState,
    Buffer, BufferBindingType, BufferUsages, ColorTargetState, ColorWrites, Device, FilterMode,
    FragmentState, FrontFace, MipmapFilterMode, MultisampleState, PipelineLayoutDescriptor,
    PolygonMode, PrimitiveState, PrimitiveTopology, RenderPipeline, RenderPipelineDescriptor,
    Sampler, SamplerBindingType, SamplerDescriptor, ShaderModuleDescriptor, ShaderSource,
    ShaderStages, TextureFormat, TextureSampleType, TextureView, TextureViewDimension,
    VertexState,
};

use crate::render::texture::RawTextureId;
use crate::render::uniforms::PostParamsUniform;

/// All persistent GPU state for the post-process pass.
///
/// Inserted as a resource alongside `ForwardPipeline`. The forward
/// pass writes the HDR target on `RenderContext`; this pipeline
/// reads that target and writes the swap chain.
pub struct PostPipeline {
    pub pipeline: RenderPipeline,
    /// BGL for `@group(0)` — HDR sampled texture + linear sampler.
    pub bgl: BindGroupLayout,
    /// BGL for `@group(1)` — post-params uniform (grade now, vignette
    /// and overlay later). Stable across 1.D Steps; the struct
    /// behind the binding is what grows.
    pub params_bgl: BindGroupLayout,
    /// GPU-side params buffer. Written from the [`ColorGrade`]
    /// resource (and later vignette / overlay state) each frame.
    ///
    /// [`ColorGrade`]: crate::render::grade::ColorGrade
    pub params_buffer: Buffer,
    /// Bind group for the params buffer. Built once at construction
    /// — the buffer view is stable, so no rebuild on resize.
    pub params_bind_group: BindGroup,
    /// Linear-clamp sampler used to fetch the HDR target. Linear
    /// filtering is harmless at 1:1 viewport size (every fragment
    /// hits a texel centre); kept linear so a later supersampling
    /// or scale-up path doesn't have to swap samplers.
    pub sampler: Sampler,
    /// Cached HDR bind group; rebuilt when `cached_generation` no
    /// longer matches `RenderContext::hdr_generation`. `None` until
    /// the first frame after a resize (or initial construction).
    cached_bind_group: Option<BindGroup>,
    cached_generation: u64,
    /// BGL for `@group(2)` — the overlay texture + sampler. Same shape
    /// as the HDR group (filterable float 2D + filtering sampler); a
    /// distinct layout object so the label and any future divergence
    /// stay clean.
    pub overlay_bgl: BindGroupLayout,
    /// Cached overlay bind group + the handle it was built for.
    /// Rebuilt when the active overlay handle changes — a
    /// consumer-driven cadence (gameplay flips the overlay), distinct
    /// from the HDR group's resize cadence and the params group's
    /// per-frame writes. `None` until the first frame.
    cached_overlay_bind_group: Option<BindGroup>,
    cached_overlay_handle: Option<RawTextureId>,
    /// BGL for `@group(3)` — the bloom pyramid's mip 0 + sampler. Same
    /// shape as the HDR group (filterable float 2D + filtering sampler).
    pub bloom_bgl: BindGroupLayout,
    /// Cached bloom bind group; rebuilt when `cached_bloom_generation` no
    /// longer matches the context's HDR generation. The bloom mip-0 view
    /// is recreated on resize alongside the HDR view, so they share a
    /// rebuild cadence. `None` until the first frame.
    cached_bloom_bind_group: Option<BindGroup>,
    cached_bloom_generation: u64,
    /// BGL for `@group(4)` — the 1x1 auto-exposure texture + sampler. Same
    /// shape as the HDR group (filterable float 2D + filtering sampler).
    pub exposure_bgl: BindGroupLayout,
    /// Cached exposure bind group. Unlike the others this is rebuilt every
    /// frame: the exposure pipeline ping-pongs between two 1x1 textures, so
    /// the "current" view alternates each frame. Rebuilding one tiny bind
    /// group per frame is cheaper than tracking the alternation. `None` until
    /// the first frame.
    cached_exposure_bind_group: Option<BindGroup>,
}

impl PostPipeline {
    /// Build the pipeline. `surface_format` is the swap-chain format
    /// the fragment shader writes into — must match
    /// `RenderContext::surface_format()`.
    pub fn new(device: &Device, surface_format: TextureFormat) -> Self {
        let bgl = create_bgl(device);
        let params_bgl = create_params_bgl(device);
        let overlay_bgl = create_overlay_bgl(device);
        let bloom_bgl = create_bloom_bgl(device);
        let exposure_bgl = create_exposure_bgl(device);

        // Seed the params buffer with the identity grade so the first
        // frame — which runs *before* the per-frame write in
        // `render_frame` — doesn't render through a zero-gamma `pow`
        // (which would produce NaN/black). Subsequent frames overwrite
        // this from the `ColorGrade` resource.
        let params_buffer = device.create_buffer_init(&BufferInitDescriptor {
            label: Some("post-params-buffer"),
            contents: bytemuck::bytes_of(&PostParamsUniform::identity()),
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
        });

        let params_bind_group = device.create_bind_group(&BindGroupDescriptor {
            label: Some("post-params-bind-group"),
            layout: &params_bgl,
            entries: &[BindGroupEntry {
                binding: 0,
                resource: params_buffer.as_entire_binding(),
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("post-pipeline-layout"),
            bind_group_layouts: &[
                Some(&bgl),
                Some(&params_bgl),
                Some(&overlay_bgl),
                Some(&bloom_bgl),
                Some(&exposure_bgl),
            ],
            immediate_size: 0,
        });

        let shader = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("post-shader"),
            source: ShaderSource::Wgsl(include_str!("../../shaders/post.wgsl").into()),
        });

        let pipeline = device.create_render_pipeline(&RenderPipelineDescriptor {
            label: Some("post-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                // Fullscreen triangle derives positions from
                // `vertex_index` — no vertex buffer is bound.
                buffers: &[],
            },
            fragment: Some(FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(ColorTargetState {
                    format: surface_format,
                    // Replace: post writes every pixel; the swap
                    // chain is cleared by this pass's LoadOp.
                    blend: Some(BlendState::REPLACE),
                    write_mask: ColorWrites::ALL,
                })],
            }),
            primitive: PrimitiveState {
                topology: PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: FrontFace::Ccw,
                // No culling — the fullscreen triangle's winding
                // depends on UV convention and viewport flip; one
                // pipeline state across both works only with culling
                // off. Cost is negligible (3 verts, 1 triangle).
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: PolygonMode::Fill,
                conservative: false,
            },
            // No depth: post is a fullscreen overwrite. Disabling
            // here keeps the pipeline independent of the forward
            // pass's depth attachment.
            depth_stencil: None,
            multisample: MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let sampler = device.create_sampler(&SamplerDescriptor {
            label: Some("post-hdr-sampler"),
            address_mode_u: AddressMode::ClampToEdge,
            address_mode_v: AddressMode::ClampToEdge,
            address_mode_w: AddressMode::ClampToEdge,
            mag_filter: FilterMode::Linear,
            min_filter: FilterMode::Linear,
            // No mipmaps on the HDR target (mip_level_count = 1 +
            // lod clamps are 0..0), so this is unreachable. Nearest
            // matches the comparison sampler's choice for the same
            // reason.
            mipmap_filter: MipmapFilterMode::Nearest,
            lod_min_clamp: 0.0,
            lod_max_clamp: 0.0,
            compare: None,
            anisotropy_clamp: 1,
            border_color: None,
        });

        Self {
            pipeline,
            bgl,
            params_bgl,
            params_buffer,
            params_bind_group,
            sampler,
            cached_bind_group: None,
            // u64::MAX so the first `ensure_bind_group` call always
            // rebuilds against whatever the context's generation is
            // — no need for a separate "first frame" branch.
            cached_generation: u64::MAX,
            overlay_bgl,
            cached_overlay_bind_group: None,
            // None so the first `ensure_overlay_bind_group` call always
            // builds — no handle has been bound yet.
            cached_overlay_handle: None,
            bloom_bgl,
            cached_bloom_bind_group: None,
            // MAX so the first `ensure_bloom_bind_group` call always
            // rebuilds against whatever generation the context reports.
            cached_bloom_generation: u64::MAX,
            exposure_bgl,
            cached_exposure_bind_group: None,
        }
    }

    /// Lazily rebuild the HDR bind group when the context's HDR view
    /// has been recreated since we last cached it.
    ///
    /// Called once per frame from `render_frame`. The first call after
    /// construction or after a resize rebuilds; subsequent calls are a
    /// pointer compare + early return.
    pub fn ensure_bind_group(
        &mut self,
        device: &Device,
        hdr_view: &TextureView,
        hdr_generation: u64,
    ) -> &BindGroup {
        if self.cached_generation != hdr_generation || self.cached_bind_group.is_none() {
            self.cached_bind_group = Some(device.create_bind_group(&BindGroupDescriptor {
                label: Some("post-hdr-bind-group"),
                layout: &self.bgl,
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
            self.cached_generation = hdr_generation;
        }
        // Just-assigned-above ⇒ infallible.
        self.cached_bind_group.as_ref().expect("bind group present")
    }

    /// Lazily rebuild the overlay bind group when the active overlay
    /// texture handle changes. `view` is the registry view for
    /// `handle` (or WHITE's view when the overlay is off — the caller
    /// resolves the fallback). Called once per frame from
    /// `render_frame`; the steady state (overlay unchanged) is a
    /// handle compare + early return.
    ///
    /// Keyed on handle, not on a generation: an F5 reload of the
    /// overlay texture *specifically* (same handle, new view) won't
    /// refresh until the handle changes. That's acceptable while the
    /// overlay has no live consumer (1.D.5 ships a debug-key test
    /// texture only); when Part 3's death sequence lands, revisit by
    /// folding overlay invalidation into `f5_reload_system`.
    pub fn ensure_overlay_bind_group(
        &mut self,
        device: &Device,
        handle: RawTextureId,
        view: &TextureView,
    ) -> &BindGroup {
        if self.cached_overlay_handle != Some(handle) || self.cached_overlay_bind_group.is_none() {
            self.cached_overlay_bind_group = Some(device.create_bind_group(&BindGroupDescriptor {
                label: Some("post-overlay-bind-group"),
                layout: &self.overlay_bgl,
                entries: &[
                    BindGroupEntry {
                        binding: 0,
                        resource: BindingResource::TextureView(view),
                    },
                    BindGroupEntry {
                        binding: 1,
                        // Reuse the HDR linear-clamp sampler — the
                        // overlay is sampled at the same 1:1 screen UV.
                        resource: BindingResource::Sampler(&self.sampler),
                    },
                ],
            }));
            self.cached_overlay_handle = Some(handle);
        }
        self.cached_overlay_bind_group
            .as_ref()
            .expect("overlay bind group present")
    }

    /// Drop the cached overlay bind group if it was built for `id` — the
    /// texture-side twin of [`ForwardPipeline::invalidate_material_bind_group`].
    /// Called from `render_frame`'s reclaim pass for every freed texture
    /// id: a freed texture whose view is still held by this cached bind
    /// group would stay pinned (and the `TextureRegistry` entry already
    /// gone), so the cache must release it too. The next
    /// [`Self::ensure_overlay_bind_group`] rebuilds against whatever the
    /// overlay points at now.
    ///
    /// In practice the overlay cache is single-entry and re-keyed every
    /// frame, so it would self-correct on the next overlay change anyway —
    /// but wiring it into the freed-id path makes the "no cache outlives
    /// the texture it samples" invariant hold by construction rather than
    /// by that timing accident.
    pub fn invalidate_overlay_bind_group(&mut self, id: RawTextureId) {
        if self.cached_overlay_handle == Some(id) {
            self.cached_overlay_handle = None;
            self.cached_overlay_bind_group = None;
        }
    }

    /// Lazily rebuild the bloom bind group when the bloom pyramid's mip-0
    /// view has been recreated (resize bumps the context's HDR generation,
    /// which is also when the bloom chain is rebuilt). `bloom_view` is
    /// `BloomPipeline::mip0_view`. Called once per frame from
    /// `render_frame`; steady state is a generation compare + early return.
    pub fn ensure_bloom_bind_group(
        &mut self,
        device: &Device,
        bloom_view: &TextureView,
        bloom_generation: u64,
    ) -> &BindGroup {
        if self.cached_bloom_generation != bloom_generation
            || self.cached_bloom_bind_group.is_none()
        {
            self.cached_bloom_bind_group = Some(device.create_bind_group(&BindGroupDescriptor {
                label: Some("post-bloom-bind-group"),
                layout: &self.bloom_bgl,
                entries: &[
                    BindGroupEntry {
                        binding: 0,
                        resource: BindingResource::TextureView(bloom_view),
                    },
                    BindGroupEntry {
                        binding: 1,
                        // Reuse the HDR linear-clamp sampler — bloom is
                        // sampled at the same 1:1 screen UV.
                        resource: BindingResource::Sampler(&self.sampler),
                    },
                ],
            }));
            self.cached_bloom_generation = bloom_generation;
        }
        self.cached_bloom_bind_group
            .as_ref()
            .expect("bloom bind group present")
    }

    /// Rebuild the exposure bind group for the current frame's exposure
    /// texture. `exposure_view` is
    /// [`ExposurePipeline::current_exposure_view`]. Rebuilt unconditionally
    /// each frame because the exposure pipeline ping-pongs its 1x1 textures,
    /// so the bound view changes every frame — there is no stable generation
    /// to compare against. The cost is one tiny bind group per frame.
    ///
    /// [`ExposurePipeline::current_exposure_view`]: crate::render::exposure::ExposurePipeline::current_exposure_view
    pub fn ensure_exposure_bind_group(
        &mut self,
        device: &Device,
        exposure_view: &TextureView,
    ) -> &BindGroup {
        self.cached_exposure_bind_group = Some(device.create_bind_group(&BindGroupDescriptor {
            label: Some("post-exposure-bind-group"),
            layout: &self.exposure_bgl,
            entries: &[
                BindGroupEntry {
                    binding: 0,
                    resource: BindingResource::TextureView(exposure_view),
                },
                BindGroupEntry {
                    binding: 1,
                    // Reuse the HDR linear-clamp sampler — the 1x1 exposure
                    // is point-sampled at the texel centre, so the filter
                    // mode is irrelevant.
                    resource: BindingResource::Sampler(&self.sampler),
                },
            ],
        }));
        self.cached_exposure_bind_group
            .as_ref()
            .expect("exposure bind group present")
    }
}

fn create_bgl(device: &Device) -> BindGroupLayout {
    device.create_bind_group_layout(&BindGroupLayoutDescriptor {
        label: Some("post-bgl"),
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

fn create_overlay_bgl(device: &Device) -> BindGroupLayout {
    // Same shape as the HDR group: a filterable float 2D texture plus
    // a filtering sampler. Built as its own layout so the binding
    // labels read "overlay" and a later divergence (e.g. a repeat
    // sampler for tiling noise) doesn't force a shared-layout split.
    device.create_bind_group_layout(&BindGroupLayoutDescriptor {
        label: Some("post-overlay-bgl"),
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

fn create_bloom_bgl(device: &Device) -> BindGroupLayout {
    // Same shape as the HDR group: a filterable float 2D texture plus a
    // filtering sampler. Its own layout so the binding labels read "bloom"
    // and a later divergence stays clean.
    device.create_bind_group_layout(&BindGroupLayoutDescriptor {
        label: Some("post-bloom-bgl"),
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

fn create_exposure_bgl(device: &Device) -> BindGroupLayout {
    // Same shape as the HDR group: a filterable float 2D texture plus a
    // filtering sampler. The bound texture is the exposure pipeline's 1x1
    // R16Float ping-pong target.
    device.create_bind_group_layout(&BindGroupLayoutDescriptor {
        label: Some("post-exposure-bgl"),
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
        label: Some("post-params-bgl"),
        entries: &[BindGroupLayoutEntry {
            binding: 0,
            // Fragment-only — the post chain reads grade/vignette/
            // overlay parameters at shading time; the vertex stage is
            // a parameterless fullscreen triangle.
            visibility: ShaderStages::FRAGMENT,
            ty: BindingType::Buffer {
                ty: BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: NonZeroU64::new(
                    std::mem::size_of::<PostParamsUniform>() as u64
                ),
            },
            count: None,
        }],
    })
}
