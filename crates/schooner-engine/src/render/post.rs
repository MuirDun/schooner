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
}

impl PostPipeline {
    /// Build the pipeline. `surface_format` is the swap-chain format
    /// the fragment shader writes into — must match
    /// `RenderContext::surface_format()`.
    pub fn new(device: &Device, surface_format: TextureFormat) -> Self {
        let bgl = create_bgl(device);
        let params_bgl = create_params_bgl(device);

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
            bind_group_layouts: &[Some(&bgl), Some(&params_bgl)],
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
