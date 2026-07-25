//! Depth-only shadow rendering — pipeline shell.
//!
//! Phase 1.C.1 lands the pipeline + bind-group layouts. The
//! per-caster textures (1.C.2), the view-projection buffer +
//! matrix computation (1.C.3), and the actual shadow-pass draws
//! plus forward-shader sampling (1.C.4) arrive in subsequent
//! steps. Until then `ShadowPipeline` is constructed and stored as
//! a resource but never drawn into — proving it links and
//! validates against wgpu.
//!
//! ## Bind-group layout
//!
//! The shadow pipeline declares two groups:
//!
//! - **@group(0)** — single `mat4` view-projection per caster.
//!   Bound and rewritten between the N shadow passes per frame.
//! - **@group(1)** — the same per-draw model uniform layout the
//!   forward pipeline uses at group 2 (dynamic-offset, 96 B
//!   `ModelUniformData`). The shadow pass reuses the forward
//!   pipeline's model buffer; only the `.model` matrix is read.
//!
//! Building a separate model BGL here (rather than threading the
//! forward pipeline's BGL through) keeps `ShadowPipeline::new`'s
//! signature small. wgpu treats two BGLs with identical entries as
//! compatible, so the underlying buffer can be bound through
//! either bind group.
//!
//! ## What this pipeline does *not* do
//!
//! - No fragment shader. wgpu accepts `fragment: None` for
//!   depth-only output; the rasterizer writes only `gl_Position`-
//!   derived depth into the bound depth attachment.
//! - No color targets.
//! - Culling is disabled. Front-face culling cleanly kills self-
//!   shadow acne on solid occluders but light-leaks under them
//!   (back-face depth is too far; receivers between front and back
//!   of the occluder silhouette test as lit). The modern recipe is
//!   no-cull (depth test keeps the closer fragment) plus a
//!   normal-offset bias applied fragment-side in
//!   `spot_shadow_factor`; see Real-Time Rendering 4th ed. §7.5,
//!   Holbert 2010, Sylvan 2008. A small slope-scaled bias is kept
//!   on the rasterizer as defense at the very-far-plane.

use std::num::NonZeroU64;

use glam::{Mat4, Vec3};
use log::warn;
use wgpu::{
    BindGroup, BindGroupDescriptor, BindGroupEntry, BindGroupLayout, BindGroupLayoutDescriptor,
    BindGroupLayoutEntry, BindingResource, BindingType, Buffer, BufferBindingType, BufferUsages,
    DepthBiasState, DepthStencilState, Device, Extent3d, FrontFace, MultisampleState,
    PipelineLayoutDescriptor, PolygonMode, PrimitiveState, PrimitiveTopology, RenderPipeline,
    RenderPipelineDescriptor, Sampler, ShaderModuleDescriptor, ShaderSource, ShaderStages,
    StencilState, Texture, TextureAspect, TextureDescriptor, TextureDimension, TextureUsages,
    TextureView, TextureViewDescriptor, TextureViewDimension, VertexState,
};

use crate::render::context::DEPTH_FORMAT;
use crate::render::light::SpotLight;
use crate::render::mesh::Vertex;
use crate::render::uniforms::ModelUniformData;
use crate::transform::Transform;

/// Maximum spot lights that may carry a [`Shadowcaster`] marker
/// simultaneously.
///
/// [`Shadowcaster`]: crate::render::light::Shadowcaster
///
/// Each caster pays one depth render pass per frame plus one
/// 1024² depth texture (4 MB at `Depth32Float`). Indoor Kinesis
/// chambers comfortably fit inside this cap — the playground has
/// one, the densest authored room is unlikely to want more than
/// three. Overflow is warn-and-dropped on the render side (same
/// pattern as `MAX_SPOT_LIGHTS` overflow).
pub const MAX_SHADOW_CASTERS: usize = 4;

/// Square edge length of every shadow map, in texels.
///
/// 1024² is the indoor-Kinesis budget — large enough that PCF
/// taps blur softly without granular stair-stepping at the kind
/// of distances chamber spots cast across (6–10 m), small enough
/// that four maps cost 16 MB total at `Depth32Float`. If god-ray
/// scattering in 1.E wants higher resolution we revisit, but for
/// the carve-the-room contribution PCF dominates the perceptual
/// quality and 1024² is plenty.
pub const SHADOW_MAP_RESOLUTION: u32 = 1024;

/// Near plane for the shadow projection.
///
/// Small enough to capture geometry close to the spot housing
/// (the lamp's own mesh, if any), large enough to keep the depth
/// distribution well-conditioned. Indoor spots cast across a
/// 6–10 m range; near = 0.1 gives ~100× depth resolution at the
/// near end of that span, well within `Depth32Float` precision.
const SHADOW_NEAR: f32 = 0.1;

/// Stride of one entry in the per-caster view-projection uniform
/// buffer. wgpu's baseline `min_uniform_buffer_offset_alignment`
/// is 256 B; the mat4 itself is 64 B — the rest is padding the
/// GPU never reads. Same pattern as `MODEL_UNIFORM_STRIDE` in
/// the forward pipeline.
pub const SHADOW_VP_UNIFORM_STRIDE: u64 = 256;

/// Per-caster view-projection uniform size (one `mat4`, 64 B).
const SHADOW_VIEW_PROJ_SIZE: u64 = std::mem::size_of::<[[f32; 4]; 4]>() as u64;

/// Per-draw model uniform size, mirrored from the forward
/// pipeline. The shadow pass reuses the forward pipeline's model
/// buffer through a separately-built bind group; the
/// `min_binding_size` must match what the forward side declares
/// or wgpu validation rejects the dynamic-offset bind.
const MODEL_UNIFORM_SIZE: u64 = std::mem::size_of::<ModelUniformData>() as u64;

/// All persistent GPU state for the depth-only shadow pipeline.
/// Lives in the `World` as a resource alongside [`ForwardPipeline`].
///
/// The view-projection uniform buffer is sized for
/// [`MAX_SHADOW_CASTERS`] entries at a 256 B stride so each shadow
/// pass picks its caster's matrix via dynamic offset — same trick
/// the forward pipeline uses for the per-draw model uniform. The
/// model bind group reuses the forward pipeline's underlying
/// buffer through a separately-built BGL with matching entry shape,
/// so per-draw matrices written once before the shadow passes are
/// visible to both the shadow pass and the forward pass.
///
/// [`ForwardPipeline`]: crate::render::ForwardPipeline
pub struct ShadowPipeline {
    /// The depth-only render pipeline. Bound at the start of each
    /// shadow pass; the same pipeline serves every caster.
    pub pipeline: RenderPipeline,

    /// Per-caster view-projection uniform buffer. Capacity =
    /// `MAX_SHADOW_CASTERS` × `SHADOW_VP_UNIFORM_STRIDE`. Written
    /// once per frame before the shadow passes; each pass rebinds
    /// the same buffer with a different dynamic offset.
    pub vp_buffer: Buffer,

    /// Bind group for @group(0). Created once with single-entry
    /// range; the dynamic offset chooses which caster's matrix is
    /// "live" for the current shadow pass.
    pub vp_bind_group: BindGroup,

    /// Bind group for @group(1) — points at the forward pipeline's
    /// model buffer. Dynamic offset matches whatever the forward
    /// path uses for that draw, so a draw renders the same geometry
    /// in both passes without duplicate uniform writes.
    pub model_bind_group: BindGroup,
}

impl ShadowPipeline {
    /// Build the depth-only pipeline.
    ///
    /// `forward_model_buffer` is the dynamic-offset per-draw model
    /// uniform buffer owned by [`ForwardPipeline`]. The shadow side
    /// constructs its own bind group from that buffer through a
    /// locally-built BGL whose entries match the forward BGL slot
    /// for slot; this is what lets one buffer back two pipelines.
    ///
    /// The pipeline's depth target format is fixed to
    /// [`DEPTH_FORMAT`]; the shadow textures allocated in 1.C.2
    /// must use the same format or the depth-stencil attachment
    /// will fail validation.
    ///
    /// [`ForwardPipeline`]: crate::render::ForwardPipeline
    pub fn new(device: &Device, forward_model_buffer: &Buffer) -> Self {
        let view_proj_layout = create_view_proj_layout(device);
        let model_layout = create_model_layout(device);

        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("shadow-pipeline-layout"),
            bind_group_layouts: &[Some(&view_proj_layout), Some(&model_layout)],
            immediate_size: 0,
        });

        let shader = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("shadow-shader"),
            source: ShaderSource::Wgsl(include_str!("../../shaders/shadow.wgsl").into()),
        });

        let pipeline = device.create_render_pipeline(&RenderPipelineDescriptor {
            label: Some("shadow-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Vertex::LAYOUT],
            },
            // Depth-only. The rasterizer writes clip-space depth
            // straight into the bound depth attachment; no color
            // target means no fragment work at all.
            fragment: None,
            primitive: PrimitiveState {
                topology: PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: FrontFace::Ccw,
                // No culling. Front-face culling cleanly suppresses
                // self-shadow acne on solid occluders, but records
                // back-face depth in the shadow map — so receivers
                // beyond the occluder (e.g. the floor under a cube)
                // light-leak because the recorded depth is too far.
                // The modern recipe is back-face cull (or no cull —
                // equivalent here since the depth test keeps the
                // closer fragment) plus normal-offset bias on the
                // fragment-side lookup; see `spot_shadow_factor` in
                // `forward.wgsl`. Reference: Holbert 2010 /
                // Sylvan 2008 / Real-Time Rendering 4th ed. §7.5.
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: Some(DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: StencilState::default(),
                // Small slope-scaled bias as a defense at the very-
                // far-plane case. The primary acne suppressant is
                // the normal-offset bias in `spot_shadow_factor`;
                // this constant covers the case where stored depth
                // is the clear value (1.0) and a lit fragment lands
                // very close to it due to perspective compression.
                bias: DepthBiasState {
                    constant: 2,
                    slope_scale: 2.0,
                    clamp: 0.0,
                },
            }),
            multisample: MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let vp_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("shadow-vp-uniform"),
            size: SHADOW_VP_UNIFORM_STRIDE * MAX_SHADOW_CASTERS as u64,
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let vp_bind_group = device.create_bind_group(&BindGroupDescriptor {
            label: Some("shadow-vp-bind-group"),
            layout: &view_proj_layout,
            entries: &[BindGroupEntry {
                binding: 0,
                resource: BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &vp_buffer,
                    offset: 0,
                    size: NonZeroU64::new(SHADOW_VIEW_PROJ_SIZE),
                }),
            }],
        });

        // Model bind group points at the forward pipeline's buffer.
        // wgpu refcounts the buffer through the bind group, so the
        // shadow side stays valid even though the forward side owns
        // the underlying allocation.
        let model_bind_group = device.create_bind_group(&BindGroupDescriptor {
            label: Some("shadow-model-bind-group"),
            layout: &model_layout,
            entries: &[BindGroupEntry {
                binding: 0,
                resource: BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: forward_model_buffer,
                    offset: 0,
                    size: NonZeroU64::new(MODEL_UNIFORM_SIZE),
                }),
            }],
        });

        Self {
            pipeline,
            vp_buffer,
            vp_bind_group,
            model_bind_group,
        }
    }
}

fn create_view_proj_layout(device: &Device) -> BindGroupLayout {
    device.create_bind_group_layout(&BindGroupLayoutDescriptor {
        label: Some("shadow-view-proj-bgl"),
        entries: &[BindGroupLayoutEntry {
            binding: 0,
            // Vertex-only — the shadow shader has no fragment.
            visibility: ShaderStages::VERTEX,
            ty: BindingType::Buffer {
                ty: BufferBindingType::Uniform,
                // Dynamic offset selects which caster's mat4 is
                // live during the current shadow pass; the buffer
                // packs N mat4s at 256 B stride for alignment.
                has_dynamic_offset: true,
                min_binding_size: NonZeroU64::new(SHADOW_VIEW_PROJ_SIZE),
            },
            count: None,
        }],
    })
}

fn create_model_layout(device: &Device) -> BindGroupLayout {
    device.create_bind_group_layout(&BindGroupLayoutDescriptor {
        label: Some("shadow-model-bgl"),
        entries: &[BindGroupLayoutEntry {
            binding: 0,
            // Vertex-only — only the model matrix is read in this
            // pipeline. Material params live in the buffer's
            // trailing bytes but are never sampled here.
            visibility: ShaderStages::VERTEX,
            ty: BindingType::Buffer {
                ty: BufferBindingType::Uniform,
                has_dynamic_offset: true,
                min_binding_size: NonZeroU64::new(MODEL_UNIFORM_SIZE),
            },
            count: None,
        }],
    })
}

/// Per-caster depth maps backed by a single 2D-array depth
/// texture.
///
/// Storage strategy is one `Texture` with `MAX_SHADOW_CASTERS`
/// layers (planning revisit during 1.C.4: `binding_array<texture_
/// depth_2d>` in WGSL would have required `Features::TEXTURE_
/// BINDING_ARRAY`, which narrows adapter compatibility for no
/// real benefit at indoor scale — the 2D-array path is core
/// wgpu). The forward shader samples the whole stack through
/// `texture_depth_2d_array` and chooses the layer per fragment;
/// each shadow pass attaches a per-layer view as its depth target.
///
/// Memory is always allocated for the full cap (4 × 1024² × 4 B =
/// 16 MB at `Depth32Float`) — the trade-off vs. lazy allocation is
/// favourable because the per-frame bind group becomes static, no
/// rebuild on caster-count change, no fallback-texture bookkeeping
/// for empty slots. 16 MB on a desktop GPU is rounding error.
///
/// `active_count` tracks how many layers are in use for the
/// current frame. The shadow pass iterates `0..active_count`;
/// the forward shader iterates spot lights whose `shadow_index`
/// is in that same range. Layers past `active_count` are not
/// read (spots with no shadowcaster bear `shadow_index = -1`).
#[derive(Debug)]
pub struct ShadowMaps {
    #[allow(dead_code)] // kept alive — wgpu views refcount the texture
    texture: Texture,
    /// Per-layer views. Built once at construction; each entry is
    /// a depth view limited to a single array slice, suitable for
    /// use as a shadow-pass depth attachment.
    layer_views: [TextureView; MAX_SHADOW_CASTERS],
    /// Whole-stack view covering all layers; what the forward
    /// shader's `texture_depth_2d_array` binding reads from.
    array_view: TextureView,
    /// The bind group at `@group(3)` of the forward pipeline.
    /// Combines `array_view` with the comparison sampler. Built
    /// once at construction; never rebuilt.
    bind_group: BindGroup,
    /// Layers in use for the current frame. Updated by
    /// [`set_active_count`]; never larger than [`MAX_SHADOW_CASTERS`].
    active_count: usize,
}

impl ShadowMaps {
    /// Allocate the 2D-array depth texture, build per-layer views +
    /// the array view, and create the forward-side bind group
    /// against the provided BGL and sampler.
    ///
    /// `forward_shadow_bgl` and `comparison_sampler` are owned by
    /// [`ForwardPipeline`]; the bind group constructed here
    /// refcounts the sampler through wgpu so the forward pipeline
    /// retains its own handle independently.
    ///
    /// [`ForwardPipeline`]: crate::render::ForwardPipeline
    pub fn new(
        device: &Device,
        forward_shadow_bgl: &BindGroupLayout,
        comparison_sampler: &Sampler,
    ) -> Self {
        let texture = device.create_texture(&TextureDescriptor {
            label: Some("shadow-map-array"),
            size: Extent3d {
                width: SHADOW_MAP_RESOLUTION,
                height: SHADOW_MAP_RESOLUTION,
                depth_or_array_layers: MAX_SHADOW_CASTERS as u32,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: DEPTH_FORMAT,
            // RENDER_ATTACHMENT: each per-layer view used as
            //   depth target by the shadow pass.
            // TEXTURE_BINDING: array_view sampled by the forward
            //   shader via the comparison sampler.
            usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });

        // Per-layer views — one for each MAX_SHADOW_CASTERS slot.
        // Built up-front because TextureView creation needs the
        // texture and we want the views to outlive any per-frame
        // path. `base_array_layer: i, array_layer_count: Some(1)`
        // narrows the view to a single layer.
        let layer_views: [TextureView; MAX_SHADOW_CASTERS] = std::array::from_fn(|i| {
            texture.create_view(&TextureViewDescriptor {
                label: Some("shadow-map-layer-view"),
                format: Some(DEPTH_FORMAT),
                dimension: Some(TextureViewDimension::D2),
                aspect: TextureAspect::DepthOnly,
                base_mip_level: 0,
                mip_level_count: Some(1),
                base_array_layer: i as u32,
                array_layer_count: Some(1),
                usage: None,
            })
        });

        // Array view — the forward shader binds this once and
        // selects the layer per fragment.
        let array_view = texture.create_view(&TextureViewDescriptor {
            label: Some("shadow-map-array-view"),
            format: Some(DEPTH_FORMAT),
            dimension: Some(TextureViewDimension::D2Array),
            aspect: TextureAspect::DepthOnly,
            base_mip_level: 0,
            mip_level_count: Some(1),
            base_array_layer: 0,
            array_layer_count: Some(MAX_SHADOW_CASTERS as u32),
            usage: None,
        });

        let bind_group = device.create_bind_group(&BindGroupDescriptor {
            label: Some("shadow-bind-group"),
            layout: forward_shadow_bgl,
            entries: &[
                BindGroupEntry {
                    binding: 0,
                    resource: BindingResource::TextureView(&array_view),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: BindingResource::Sampler(comparison_sampler),
                },
            ],
        });

        Self {
            texture,
            layer_views,
            array_view,
            bind_group,
            active_count: 0,
        }
    }

    /// Number of layers in use this frame. Always
    /// `<= MAX_SHADOW_CASTERS`.
    pub fn active_count(&self) -> usize {
        self.active_count
    }

    pub fn is_empty(&self) -> bool {
        self.active_count == 0
    }

    /// Depth view scoped to a single array layer. Used as the
    /// shadow pass's depth-stencil attachment.
    pub fn layer_view(&self, index: usize) -> Option<&TextureView> {
        self.layer_views.get(index)
    }

    /// Whole-stack array view sampled by the forward shader.
    pub fn array_view(&self) -> &TextureView {
        &self.array_view
    }

    /// Bind group for `@group(3)` of the forward pipeline.
    pub fn bind_group(&self) -> &BindGroup {
        &self.bind_group
    }

    /// Record how many shadow-map layers are in active use for
    /// this frame. Clamps to [`MAX_SHADOW_CASTERS`] and warns
    /// once on overflow, matching the warn-and-drop pattern light
    /// overflow uses. No GPU allocation — the texture is permanent.
    pub fn set_active_count(&mut self, count: usize) {
        let (target, overflowed) = clamp_caster_count(count);
        if overflowed {
            warn!(
                "ShadowMaps: {} shadowcasters requested, capped at {}",
                count, MAX_SHADOW_CASTERS
            );
        }
        self.active_count = target;
    }
}

/// Build the world-space view-projection matrix that the shadow
/// pass uses to render from a spot light's point of view.
///
/// The view matrix looks from `transform.translation` along
/// `transform.rotation * Vec3::NEG_Z` (the spot's own "forward,"
/// matching the camera-forward convention used elsewhere in the
/// engine). The up-vector defaults to `Vec3::Y`, falling back to
/// `Vec3::Z` when the direction is nearly parallel to Y — without
/// the fallback, `look_at_rh` collapses to a singular matrix and
/// the shadow map fills with NaN.
///
/// The projection is a square `glam::Mat4::perspective_rh` (NDC
/// depth [0, 1], matching wgpu/Metal/Vulkan/DX12 — same
/// convention the camera uses). FOVy is twice the cone's outer
/// half-angle, recovered from `outer_cone_cos`. The far plane is
/// the spot's range — beyond range the light contributes nothing
/// anyway, so depth precision past it is wasted.
pub fn compute_shadow_vp(transform: &Transform, spot: &SpotLight) -> Mat4 {
    let position = transform.translation;
    let direction = (transform.rotation * Vec3::NEG_Z).normalize_or_zero();

    // 0.999 ≈ cos(2.6°). Anything closer to vertical than that
    // gives `look_at_rh(eye, eye + Vec3::NEG_Y, Vec3::Y)` a
    // near-zero cross product. The standard fix is to pick a
    // different up — Z is the canonical orthogonal choice for the
    // spotlight-straight-down case (overhead chamber lamps).
    let up = if direction.y.abs() > 0.999 {
        Vec3::Z
    } else {
        Vec3::Y
    };

    let view = Mat4::look_at_rh(position, position + direction, up);

    // Full vertical FOV = 2 × outer half-angle.
    // outer_cone_cos = cos(half_angle) ⇒ half_angle = acos(...).
    let fovy = spot.outer_cone_cos.acos() * 2.0;

    let proj = Mat4::perspective_rh(fovy, 1.0, SHADOW_NEAR, spot.range);

    proj * view
}

/// Decide the actual shadow-map count for a requested caster
/// count, and report whether the request overflowed the cap.
///
/// Extracted so the clamp + overflow-flag decision is testable
/// without needing a wgpu device. Inlining the `min`/`>` pair into
/// `ensure_count` was the original shape — that lost the invariant
/// that "requests beyond the cap must not trigger reallocations
/// once we're already at the cap," which is the kind of thing a
/// well-intentioned future change can quietly regress.
fn clamp_caster_count(requested: usize) -> (usize, bool) {
    let target = requested.min(MAX_SHADOW_CASTERS);
    let overflowed = requested > MAX_SHADOW_CASTERS;
    (target, overflowed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamp_below_or_at_cap_passes_through() {
        assert_eq!(clamp_caster_count(0), (0, false));
        assert_eq!(
            clamp_caster_count(MAX_SHADOW_CASTERS - 1),
            (MAX_SHADOW_CASTERS - 1, false)
        );
        // At-cap is the boundary: the count is fine, the flag must
        // stay off. A flag fired here would spam the log every
        // frame in any scene that hits the cap exactly.
        assert_eq!(
            clamp_caster_count(MAX_SHADOW_CASTERS),
            (MAX_SHADOW_CASTERS, false)
        );
    }

    #[test]
    fn clamp_above_cap_truncates_and_flags() {
        // The flag must fire on overflow — losing it means a
        // scene that wires too many shadowcasters silently drops
        // some, which is the worst possible failure mode (looks
        // fine until you wonder why a light isn't casting).
        let (target, overflowed) = clamp_caster_count(MAX_SHADOW_CASTERS + 1);
        assert_eq!(target, MAX_SHADOW_CASTERS);
        assert!(overflowed);
    }

    #[test]
    fn shadow_vp_overhead_spot_projects_floor_into_clip_space() {
        // Real-scene shape: an overhead spot 5 m above the origin,
        // aimed straight down via the same `from_rotation_arc` the
        // game scene uses. The floor at y=0 inside the cone must
        // project to clip-space coordinates that lie inside the
        // canonical [-w, w]³ frustum. If `look_at_rh` collapsed
        // because of an up-vector singularity, the result would be
        // NaN or land outside the frustum.
        use glam::Quat;
        let transform = Transform {
            translation: Vec3::new(0.0, 5.0, 0.0),
            rotation: Quat::from_rotation_arc(Vec3::NEG_Z, Vec3::NEG_Y),
            scale: Vec3::ONE,
        };
        let spot = SpotLight::new(Vec3::ONE, 15.0, 8.0, 20.0, 30.0);
        let vp = compute_shadow_vp(&transform, &spot);

        // Floor point directly below the spot — must be inside the
        // unit cube after the perspective divide.
        let clip = vp * glam::Vec4::new(0.0, 0.0, 0.0, 1.0);
        assert!(
            clip.w > 0.0,
            "floor below spot should be in front of the light"
        );
        let ndc = clip.truncate() / clip.w;
        assert!(
            ndc.x.abs() <= 1.0 && ndc.y.abs() <= 1.0 && (0.0..=1.0).contains(&ndc.z),
            "floor-below-spot NDC out of frustum: {ndc:?}"
        );
    }

    #[test]
    fn shadow_vp_far_plane_matches_range() {
        // A point at `range` along the spot's forward axis sits on
        // the far plane. NDC z must be ~1 (the wgpu/glam-rh depth
        // convention). Tolerance is generous because perspective
        // divide loses precision near the far plane.
        use glam::Quat;
        let range = 8.0;
        let transform = Transform {
            translation: Vec3::new(0.0, 5.0, 0.0),
            rotation: Quat::from_rotation_arc(Vec3::NEG_Z, Vec3::NEG_Y),
            scale: Vec3::ONE,
        };
        let spot = SpotLight::new(Vec3::ONE, 1.0, range, 20.0, 30.0);
        let vp = compute_shadow_vp(&transform, &spot);

        // Spot is at y=5 pointing down; range=8, so the far plane
        // hits y = 5 - 8 = -3.
        let on_far = vp * glam::Vec4::new(0.0, -3.0, 0.0, 1.0);
        let ndc_z = on_far.z / on_far.w;
        assert!(
            (ndc_z - 1.0).abs() < 0.01,
            "far-plane point should map to NDC z≈1, got {ndc_z}"
        );
    }

    #[test]
    fn shadow_vp_handles_straight_down_without_nan() {
        // Up-vector singularity probe: a spot pointing exactly
        // along -Y is the most common indoor case (overhead lamps).
        // The fallback to Vec3::Z must keep every matrix entry
        // finite — without it, `look_at_rh` collapses and every
        // subsequent multiply produces NaN that the GPU
        // silently turns into corrupt shadow maps.
        use glam::Quat;
        let transform = Transform {
            translation: Vec3::Y,
            rotation: Quat::from_rotation_arc(Vec3::NEG_Z, Vec3::NEG_Y),
            scale: Vec3::ONE,
        };
        let spot = SpotLight::default();
        let vp = compute_shadow_vp(&transform, &spot);
        for col in vp.to_cols_array().iter() {
            assert!(
                col.is_finite(),
                "shadow VP contains non-finite entry: {col}"
            );
        }
    }

    #[test]
    fn clamp_is_idempotent_above_cap() {
        // Load-bearing invariant for `set_active_count`: once at
        // the cap, every further request >= cap must produce the
        // same active count. A future "optimization" that compared
        // unclamped counts would let an overflow scene re-clamp
        // back-and-forth between cap and (cap+k) on alternating
        // frames — silent visual flicker, no panic to flag it.
        let (target_at_cap, _) = clamp_caster_count(MAX_SHADOW_CASTERS);
        let (target_over, _) = clamp_caster_count(MAX_SHADOW_CASTERS + 5);
        let (target_way_over, _) = clamp_caster_count(MAX_SHADOW_CASTERS * 10);
        assert_eq!(target_at_cap, target_over);
        assert_eq!(target_over, target_way_over);
    }
}
