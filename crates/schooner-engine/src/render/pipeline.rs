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

use std::num::NonZeroU64;

use wgpu::util::{BufferInitDescriptor, DeviceExt};
use wgpu::{
    BindGroup, BindGroupDescriptor, BindGroupEntry, BindGroupLayout, BindGroupLayoutDescriptor,
    BindGroupLayoutEntry, BindingResource, BindingType, BlendState, Buffer, BufferBindingType,
    BufferUsages, ColorTargetState, ColorWrites, CompareFunction, DepthBiasState,
    DepthStencilState, Device, Face, FragmentState, FrontFace, MultisampleState,
    PipelineLayoutDescriptor, PolygonMode, PrimitiveState, PrimitiveTopology, RenderPipeline,
    RenderPipelineDescriptor, ShaderModuleDescriptor, ShaderSource, ShaderStages, StencilState,
    TextureFormat, VertexState,
};

use crate::render::context::DEPTH_FORMAT;
use crate::render::mesh::Vertex;
use crate::render::uniforms::{CameraUniformData, LightUniformData, ModelUniformData};

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

    pub light_buffer: Buffer,
    pub light_bind_group: BindGroup,

    pub model_buffer: Buffer,
    pub model_bind_group: BindGroup,
}

impl ForwardPipeline {
    /// Build the pipeline and all uniform buffers. `surface_format`
    /// is the swap-chain format the fragment shader writes into;
    /// must match `RenderContext::surface_format()`.
    pub fn new(device: &Device, surface_format: TextureFormat) -> Self {
        let camera_layout = create_camera_layout(device);
        let light_layout = create_light_layout(device);
        let model_layout = create_model_layout(device);

        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("forward-pipeline-layout"),
            bind_group_layouts: &[
                Some(&camera_layout),
                Some(&light_layout),
                Some(&model_layout),
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
                    format: surface_format,
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

        let light_buffer = create_uniform_buffer(
            device,
            "light-uniform",
            std::mem::size_of::<LightUniformData>() as u64,
            bytemuck::bytes_of(&LightUniformData::placeholder()),
        );
        let light_bind_group = device.create_bind_group(&BindGroupDescriptor {
            label: Some("light-bind-group"),
            layout: &light_layout,
            entries: &[BindGroupEntry {
                binding: 0,
                resource: light_buffer.as_entire_binding(),
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

        Self {
            pipeline,
            camera_buffer,
            camera_bind_group,
            light_buffer,
            light_bind_group,
            model_buffer,
            model_bind_group,
        }
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

fn create_light_layout(device: &Device) -> BindGroupLayout {
    device.create_bind_group_layout(&BindGroupLayoutDescriptor {
        label: Some("light-bgl"),
        entries: &[BindGroupLayoutEntry {
            binding: 0,
            // Fragment-only — the light is consumed in shading.
            visibility: ShaderStages::FRAGMENT,
            ty: BindingType::Buffer {
                ty: BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: NonZeroU64::new(std::mem::size_of::<LightUniformData>() as u64),
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

