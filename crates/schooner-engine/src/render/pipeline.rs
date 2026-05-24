//! Forward render pipeline + uniform plumbing.
//!
//! Owns:
//! - The three bind group layouts (camera, light, model with
//!   dynamic offset).
//! - The `RenderPipeline` itself (vertex layout, shader, depth
//!   state, rasterizer, fragment target).
//! - Camera and light uniform buffers + their bind groups —
//!   single-buffer, written once per frame.
//! - The per-draw model uniform buffer — sized for
//!   `MAX_DRAWS_PER_FRAME` slots, each `MODEL_UNIFORM_STRIDE`
//!   bytes apart so wgpu's 256-byte dynamic-offset alignment is
//!   satisfied. The bind group is created once with a single-slot
//!   range; the dynamic offset chooses which slot is "live" per
//!   draw call.
//!
//! `render_frame` (in `forward.rs`) is the only consumer.

use std::collections::HashMap;
use std::num::NonZeroU64;

use wgpu::util::{BufferInitDescriptor, DeviceExt};
use wgpu::{
    AddressMode, BindGroup, BindGroupDescriptor, BindGroupEntry, BindGroupLayout,
    BindGroupLayoutDescriptor, BindGroupLayoutEntry, BindingResource, BindingType, BlendState,
    Buffer, BufferBindingType, BufferUsages, ColorTargetState, ColorWrites, CompareFunction,
    DepthBiasState, DepthStencilState, Device, Face, FilterMode, FragmentState, FrontFace,
    MipmapFilterMode, MultisampleState, PipelineLayoutDescriptor, PolygonMode, PrimitiveState,
    PrimitiveTopology, RenderPipeline, RenderPipelineDescriptor, Sampler, SamplerBindingType,
    SamplerDescriptor, ShaderModuleDescriptor, ShaderSource, ShaderStages, StencilState,
    TextureFormat, TextureSampleType, TextureView, TextureViewDimension, VertexState,
};

use crate::render::context::DEPTH_FORMAT;
use crate::render::mesh::Vertex;
use crate::render::texture::TextureHandle;
use crate::render::uniforms::{CameraUniformData, LightsUniformData, ModelUniformData};

/// Maximum number of draws the per-frame model buffer can serve
/// without reallocation. Game 0 scenes are far below this; growing
/// the buffer when overflow appears is a future concern.
pub const MAX_DRAWS_PER_FRAME: u64 = 256;

/// Per-draw stride in the model uniform buffer.
///
/// wgpu's baseline `min_uniform_buffer_offset_alignment` is 256
/// bytes. `ModelUniformData` is 64 bytes (one mat4); the rest of
/// each 256-byte slot is padding the GPU never reads. Without this
/// alignment the dynamic-offset bind would fail validation.
pub const MODEL_UNIFORM_STRIDE: u64 = 256;

const MODEL_UNIFORM_SIZE: u64 = std::mem::size_of::<ModelUniformData>() as u64;

/// All persistent GPU state for the forward pipeline. Lives in
/// the `World` as a resource alongside `RenderContext`.
pub struct ForwardPipeline {
    pub pipeline: RenderPipeline,

    pub camera_buffer: Buffer,
    pub camera_bind_group: BindGroup,

    pub lights_buffer: Buffer,
    pub lights_bind_group: BindGroup,

    pub model_buffer: Buffer,
    pub model_bind_group: BindGroup,

    /// BGL for `@group(3)` — depth-array texture + comparison
    /// sampler. Exposed so [`ShadowMaps`] can construct its bind
    /// group against this layout.
    ///
    /// [`ShadowMaps`]: crate::render::ShadowMaps
    pub shadow_bgl: BindGroupLayout,

    /// Comparison sampler shared by every spot's shadow lookup.
    /// `CompareFunction::Less` with linear filtering — the linear
    /// filter does built-in 2×2 PCF for the comparison result, and
    /// the shader stacks a 3×3 tap loop on top for the soft edge.
    pub comparison_sampler: Sampler,

    /// BGL for `@group(4)` — per-material albedo texture + linear
    /// sampler. Material bind groups are looked up per draw call from
    /// `material_bind_groups` and built lazily on first use; rebuilt
    /// on F5 manual reload when an entry's underlying texture view is
    /// replaced.
    pub material_bgl: BindGroupLayout,

    /// Linear-repeat sampler bound into every material bind group.
    /// `Repeat` because authored albedo textures are tileable across
    /// surfaces; `Linear` mag/min for smooth zoom; `Nearest` mipmap
    /// because we don't author mips yet (Game 2A's job).
    pub material_sampler: Sampler,

    /// Cache: `TextureHandle` → material bind group. Built lazily by
    /// [`Self::ensure_material_bind_group_with_view`] and read by
    /// [`Self::material_bind_group`]; reload invalidates a single
    /// entry via [`Self::invalidate_material_bind_group`] so the
    /// next frame rebuilds against the reloaded view.
    material_bind_groups: HashMap<TextureHandle, BindGroup>,
}

impl ForwardPipeline {
    /// Build the pipeline and all uniform buffers.
    /// `color_target_format` is the format of the color attachment
    /// the fragment shader writes into — `HDR_FORMAT` since 1.D.1,
    /// because forward writes into the HDR offscreen target rather
    /// than directly into the swap chain.
    pub fn new(device: &Device, color_target_format: TextureFormat) -> Self {
        let camera_layout = create_camera_layout(device);
        let lights_layout = create_lights_layout(device);
        let model_layout = create_model_layout(device);
        let shadow_bgl = create_shadow_layout(device);
        let material_bgl = create_material_layout(device);

        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("forward-pipeline-layout"),
            bind_group_layouts: &[
                Some(&camera_layout),
                Some(&lights_layout),
                Some(&model_layout),
                Some(&shadow_bgl),
                Some(&material_bgl),
            ],
            // wgpu 29 renamed push-constant capacity to "immediate
            // size" and gated it behind Features::IMMEDIATES; we use
            // dynamic-offset uniforms instead, so leave it at zero.
            immediate_size: 0,
        });

        let shader = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("forward-shader"),
            source: ShaderSource::Wgsl(include_str!("../../shaders/forward.wgsl").into()),
        });

        let pipeline = device.create_render_pipeline(&RenderPipelineDescriptor {
            label: Some("forward-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Vertex::LAYOUT],
            },
            fragment: Some(FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(ColorTargetState {
                    format: color_target_format,
                    blend: Some(BlendState::REPLACE),
                    write_mask: ColorWrites::ALL,
                })],
            }),
            primitive: PrimitiveState {
                topology: PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: FrontFace::Ccw,
                cull_mode: Some(Face::Back),
                unclipped_depth: false,
                polygon_mode: PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: Some(DepthStencilState {
                format: DEPTH_FORMAT,
                // wgpu 29 lifted these to Option to mirror the
                // WebGPU spec's "None means depth not written /
                // not tested." For an opaque forward pass we want
                // both on.
                depth_write_enabled: Some(true),
                depth_compare: Some(CompareFunction::Less),
                stencil: StencilState::default(),
                bias: DepthBiasState::default(),
            }),
            multisample: MultisampleState::default(),
            // Replaces the old `multiview` field. `None` means
            // single-view (no array-layer broadcast); the mask is
            // only used when rendering into multiple layers at once.
            multiview_mask: None,
            cache: None,
        });

        let camera_buffer = create_uniform_buffer(
            device,
            "camera-uniform",
            std::mem::size_of::<CameraUniformData>() as u64,
            bytemuck::bytes_of(&CameraUniformData::placeholder()),
        );
        let camera_bind_group = device.create_bind_group(&BindGroupDescriptor {
            label: Some("camera-bind-group"),
            layout: &camera_layout,
            entries: &[BindGroupEntry {
                binding: 0,
                resource: camera_buffer.as_entire_binding(),
            }],
        });

        let lights_buffer = create_uniform_buffer(
            device,
            "lights-uniform",
            std::mem::size_of::<LightsUniformData>() as u64,
            bytemuck::bytes_of(&LightsUniformData::placeholder()),
        );
        let lights_bind_group = device.create_bind_group(&BindGroupDescriptor {
            label: Some("lights-bind-group"),
            layout: &lights_layout,
            entries: &[BindGroupEntry {
                binding: 0,
                resource: lights_buffer.as_entire_binding(),
            }],
        });

        let model_buffer_size = MODEL_UNIFORM_STRIDE * MAX_DRAWS_PER_FRAME;
        let model_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("model-uniform"),
            size: model_buffer_size,
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        // Bind group covers a single ModelUniformData slot; the
        // dynamic offset selects which slot the draw reads.
        let model_bind_group = device.create_bind_group(&BindGroupDescriptor {
            label: Some("model-bind-group"),
            layout: &model_layout,
            entries: &[BindGroupEntry {
                binding: 0,
                resource: BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &model_buffer,
                    offset: 0,
                    size: NonZeroU64::new(MODEL_UNIFORM_SIZE),
                }),
            }],
        });

        // Comparison sampler shared across every spot's shadow
        // tap. Linear filter + ClampToEdge: the linear filter does
        // 2×2 PCF on the comparison result; the shader adds a 3×3
        // tap kernel around the centre UV for a wider soft edge.
        // ClampToEdge handles fragments whose UV falls on the
        // shadow-map border without sampling neighbour layers;
        // out-of-frustum fragments are guarded in the shader.
        let comparison_sampler = device.create_sampler(&SamplerDescriptor {
            label: Some("shadow-comparison-sampler"),
            address_mode_u: AddressMode::ClampToEdge,
            address_mode_v: AddressMode::ClampToEdge,
            address_mode_w: AddressMode::ClampToEdge,
            mag_filter: FilterMode::Linear,
            min_filter: FilterMode::Linear,
            mipmap_filter: MipmapFilterMode::Nearest,
            lod_min_clamp: 0.0,
            lod_max_clamp: 1.0,
            // The comparison itself: `reference < sampled` ⇒ lit.
            // Surface depth (light-space z) is the reference; the
            // shadow map's stored depth is `sampled`. A surface
            // closer to the light than the occluder is unshadowed.
            compare: Some(CompareFunction::Less),
            anisotropy_clamp: 1,
            border_color: None,
        });

        // Material sampler shared by every albedo texture binding.
        // `Repeat` so an authored brick or iron texture can tile
        // naturally across a large surface; `Linear` mag/min for
        // smooth zoom; `Nearest` mipmap because the texture loader
        // ships single-mip textures (mip generation is Game 2A's
        // asset-pipeline maturation). One sampler shared across
        // every material bind group keeps state-switch cost at one
        // bind-group rebind on material change, not two.
        let material_sampler = device.create_sampler(&SamplerDescriptor {
            label: Some("material-sampler"),
            address_mode_u: AddressMode::Repeat,
            address_mode_v: AddressMode::Repeat,
            address_mode_w: AddressMode::Repeat,
            mag_filter: FilterMode::Linear,
            min_filter: FilterMode::Linear,
            mipmap_filter: MipmapFilterMode::Nearest,
            lod_min_clamp: 0.0,
            lod_max_clamp: 0.0,
            compare: None,
            anisotropy_clamp: 1,
            border_color: None,
        });

        Self {
            pipeline,
            camera_buffer,
            camera_bind_group,
            lights_buffer,
            lights_bind_group,
            model_buffer,
            model_bind_group,
            shadow_bgl,
            comparison_sampler,
            material_bgl,
            material_sampler,
            material_bind_groups: HashMap::new(),
        }
    }

    /// Ensure `handle` has a cached material bind group bound to the
    /// given `view`. No-op if the entry already exists; otherwise
    /// builds it and caches under `handle`.
    ///
    /// Two-arg shape (handle + view) instead of (handle + registry)
    /// because the renderer's pre-pass already extracts views from
    /// the registry under a shared borrow, then takes a mutable
    /// borrow on this pipeline to populate the cache — the two
    /// borrows can't overlap, so the view is threaded through as a
    /// value.
    pub fn ensure_material_bind_group_with_view(
        &mut self,
        device: &Device,
        handle: TextureHandle,
        view: &TextureView,
    ) {
        self.material_bind_groups
            .entry(handle)
            .or_insert_with(|| {
                device.create_bind_group(&BindGroupDescriptor {
                    label: Some("material-bind-group"),
                    layout: &self.material_bgl,
                    entries: &[
                        BindGroupEntry {
                            binding: 0,
                            resource: BindingResource::TextureView(view),
                        },
                        BindGroupEntry {
                            binding: 1,
                            resource: BindingResource::Sampler(&self.material_sampler),
                        },
                    ],
                })
            });
    }

    /// Read-side lookup for the per-draw bind step. Returns the
    /// cached bind group if one has been built for `handle`; `None`
    /// otherwise. The forward pass populates every handle it needs
    /// ahead of opening the render pass, so a `None` here means a
    /// logic error in the pre-pass — not a routine cache miss.
    pub fn material_bind_group(&self, handle: TextureHandle) -> Option<&BindGroup> {
        self.material_bind_groups.get(&handle)
    }

    /// Drop the cached bind group for `handle`. Called from the F5
    /// reload path when a texture's underlying GPU view has been
    /// replaced — the next pre-pass population rebuilds against the
    /// reloaded view.
    pub fn invalidate_material_bind_group(&mut self, handle: TextureHandle) {
        self.material_bind_groups.remove(&handle);
    }
}

fn create_uniform_buffer(device: &Device, label: &str, size: u64, initial: &[u8]) -> Buffer {
    // Pad initial contents with zeros to `size` so the buffer is
    // fully initialized at creation — wgpu validation is happier
    // with this than with a zero-init descriptor + queue write
    // chain on first frame.
    let mut padded = initial.to_vec();
    padded.resize(size as usize, 0);
    device.create_buffer_init(&BufferInitDescriptor {
        label: Some(label),
        contents: &padded,
        usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
    })
}

fn create_camera_layout(device: &Device) -> BindGroupLayout {
    device.create_bind_group_layout(&BindGroupLayoutDescriptor {
        label: Some("camera-bgl"),
        entries: &[BindGroupLayoutEntry {
            binding: 0,
            // VERTEX_FRAGMENT because the vertex shader needs
            // view_proj and the fragment shader needs `position`
            // for the specular term.
            visibility: ShaderStages::VERTEX_FRAGMENT,
            ty: BindingType::Buffer {
                ty: BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: NonZeroU64::new(std::mem::size_of::<CameraUniformData>() as u64),
            },
            count: None,
        }],
    })
}

fn create_lights_layout(device: &Device) -> BindGroupLayout {
    device.create_bind_group_layout(&BindGroupLayoutDescriptor {
        label: Some("lights-bgl"),
        entries: &[BindGroupLayoutEntry {
            binding: 0,
            // Fragment-only — lights are consumed in shading.
            visibility: ShaderStages::FRAGMENT,
            ty: BindingType::Buffer {
                ty: BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: NonZeroU64::new(std::mem::size_of::<LightsUniformData>() as u64),
            },
            count: None,
        }],
    })
}

fn create_model_layout(device: &Device) -> BindGroupLayout {
    device.create_bind_group_layout(&BindGroupLayoutDescriptor {
        label: Some("model-bgl"),
        entries: &[BindGroupLayoutEntry {
            binding: 0,
            // VERTEX_FRAGMENT: vertex reads `model` for position +
            // normal transform; fragment reads `albedo_roughness`
            // and `emissive` for per-instance shading from 1.A.3
            // onward.
            visibility: ShaderStages::VERTEX_FRAGMENT,
            ty: BindingType::Buffer {
                ty: BufferBindingType::Uniform,
                // The load-bearing flag: enables the per-draw
                // dynamic offset rebind without recreating the
                // bind group.
                has_dynamic_offset: true,
                min_binding_size: NonZeroU64::new(MODEL_UNIFORM_SIZE),
            },
            count: None,
        }],
    })
}

fn create_material_layout(device: &Device) -> BindGroupLayout {
    device.create_bind_group_layout(&BindGroupLayoutDescriptor {
        label: Some("material-bgl"),
        entries: &[
            BindGroupLayoutEntry {
                binding: 0,
                // Fragment-only — albedo is sampled in shading.
                visibility: ShaderStages::FRAGMENT,
                ty: BindingType::Texture {
                    // `filterable: true` so the linear `material_sampler`
                    // can interpolate texels; pairs with the BGL entry's
                    // `Filtering` sampler type below.
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

fn create_shadow_layout(device: &Device) -> BindGroupLayout {
    device.create_bind_group_layout(&BindGroupLayoutDescriptor {
        label: Some("shadow-bgl"),
        entries: &[
            BindGroupLayoutEntry {
                binding: 0,
                // Fragment-only — shadow sampling lives in shading.
                visibility: ShaderStages::FRAGMENT,
                ty: BindingType::Texture {
                    sample_type: TextureSampleType::Depth,
                    // 2D array: one allocation, N layers, the
                    // shader picks the layer per-fragment from the
                    // spot's shadow_index. See `ShadowMaps` for
                    // the planning trade-off vs binding_array.
                    view_dimension: TextureViewDimension::D2Array,
                    multisampled: false,
                },
                count: None,
            },
            BindGroupLayoutEntry {
                binding: 1,
                visibility: ShaderStages::FRAGMENT,
                // Comparison sampler: paired with the depth
                // texture, `textureSampleCompare` returns the
                // filtered comparison result in [0, 1].
                ty: BindingType::Sampler(SamplerBindingType::Comparison),
                count: None,
            },
        ],
    })
}
