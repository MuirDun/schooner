//! GPU-shaped uniform structs.
//!
//! Each struct mirrors a WGSL uniform in `shaders/forward.wgsl`,
//! laid out with explicit padding so the Rust `#[repr(C)]` layout
//! matches std140. Rust `Mat4` is 64B; `Vec3` is 12B but std140
//! pads to 16, so vectors are stored as `[f32; 4]` with the last
//! element doubling as padding (or holding a real scalar like
//! ambient when convenient).
//!
//! `Pod` is what lets us copy a `&CameraUniformData` into the
//! command queue with `bytemuck::cast_slice` — no field-by-field
//! serialization, no allocation.

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};

use crate::material::Material;

/// Maximum spot lights packed into [`LightsUniformData`] per frame.
///
/// Mirrored in `shaders/forward.wgsl` as the size of `lights.spots`.
/// Indoor Kinesis chambers use 1–3 spots typically; 8 is generous
/// headroom without growing the uniform meaningfully (8 × 64 B =
/// 512 B).
pub const MAX_SPOT_LIGHTS: usize = 8;

/// Maximum point lights packed into [`LightsUniformData`] per frame.
///
/// Mirrored in `shaders/forward.wgsl` as the size of `lights.points`.
pub const MAX_POINT_LIGHTS: usize = 16;

/// Camera uniform — `view`, `proj`, `view_proj`, `position`.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct CameraUniformData {
    pub view: [[f32; 4]; 4],
    pub proj: [[f32; 4]; 4],
    pub view_proj: [[f32; 4]; 4],
    /// `xyz` = world-space camera position; `w` unused.
    pub position: [f32; 4],
}

impl CameraUniformData {
    /// Build the uniform from a view matrix, a projection matrix,
    /// and the camera's world-space position.
    pub fn new(view: Mat4, proj: Mat4, position: Vec3) -> Self {
        let view_proj = proj * view;
        Self {
            view: view.to_cols_array_2d(),
            proj: proj.to_cols_array_2d(),
            view_proj: view_proj.to_cols_array_2d(),
            position: [position.x, position.y, position.z, 0.0],
        }
    }

    /// Sensible "no camera in scene" fallback so an unbound buffer
    /// never reaches the GPU. The view is identity and the proj is
    /// a 1:1 perspective — anything drawn under these will land
    /// off-screen, which is the correct behavior when no camera is
    /// active.
    pub fn placeholder() -> Self {
        Self::new(Mat4::IDENTITY, Mat4::IDENTITY, Vec3::ZERO)
    }
}

/// Directional light layout inside [`LightsUniformData`].
///
/// `direction.xyz` is the direction the light **travels** (sun →
/// surface); the shader negates it. `color_intensity.xyz` is the
/// unit-magnitude tint and `.w` is the scalar multiplier — the
/// shader multiplies them to get final radiance. `ambient.xyz` is
/// the flat ambient term carried along with the directional for
/// convenience; the shader reads it unconditionally regardless of
/// `counts.x`, so scenes without a sun can still surface ambient
/// via the placeholder fallback.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct DirectionalLightUniformData {
    pub direction: [f32; 4],
    pub color_intensity: [f32; 4],
    pub ambient: [f32; 4],
}

impl DirectionalLightUniformData {
    pub fn new(direction: Vec3, color: Vec3, intensity: f32, ambient: Vec3) -> Self {
        Self {
            direction: [direction.x, direction.y, direction.z, 0.0],
            color_intensity: [color.x, color.y, color.z, intensity],
            ambient: [ambient.x, ambient.y, ambient.z, 0.0],
        }
    }
}

/// Spot light layout inside [`LightsUniformData`].
///
/// Cone falloff is stored as cosines (matching `SpotLight` on the
/// CPU side) so the fragment shader's `smoothstep` doesn't need a
/// per-fragment `cos()`.
///
/// `shadow_index` is encoded as `f32` in the `.y` slot of
/// `outer_cos_shadow`. The shader compares against `0.0` instead
/// of using `i32` to keep the slot in a `vec4<f32>` neighbourhood
/// and avoid a `bitcast` or a separate `vec4<i32>` member. A
/// negative value means "no shadow" — non-shadowcasting spots
/// pass `-1.0` and the shadow-sampling branch skips them.
///
/// `view_proj` is the light-space matrix the forward shader uses
/// to project world position into the spot's shadow map (the same
/// matrix the shadow pass uses to render that map). Storing it
/// per-spot duplicates it against `ShadowPipeline::vp_buffer`, at
/// 64 B × 8 spots = 512 B added — trivial vs. the simpler bind
/// group story (no extra binding for the forward shader to read).
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct SpotLightUniformData {
    /// `xyz` = world position, `w` = intensity.
    pub position_intensity: [f32; 4],
    /// `xyz` = world-space direction (unit), `w` = range.
    pub direction_range: [f32; 4],
    /// `xyz` = color (unit tint), `w` = inner cone cosine.
    pub color_inner_cos: [f32; 4],
    /// `x` = outer cone cosine, `y` = shadow_index as f32
    /// (`-1.0` ⇒ no shadow), `zw` padding.
    pub outer_cos_shadow: [f32; 4],
    /// Light-space view-projection matrix. Zero matrix when
    /// `shadow_index < 0` (never read in that case).
    pub view_proj: [[f32; 4]; 4],
}

impl SpotLightUniformData {
    pub fn new(
        position: Vec3,
        direction: Vec3,
        color: Vec3,
        intensity: f32,
        range: f32,
        inner_cone_cos: f32,
        outer_cone_cos: f32,
        shadow_index: i32,
        view_proj: [[f32; 4]; 4],
    ) -> Self {
        Self {
            position_intensity: [position.x, position.y, position.z, intensity],
            direction_range: [direction.x, direction.y, direction.z, range],
            color_inner_cos: [color.x, color.y, color.z, inner_cone_cos],
            outer_cos_shadow: [outer_cone_cos, shadow_index as f32, 0.0, 0.0],
            view_proj,
        }
    }
}

/// Point light layout inside [`LightsUniformData`].
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct PointLightUniformData {
    /// `xyz` = world position, `w` = range.
    pub position_range: [f32; 4],
    /// `xyz` = color (unit tint), `w` = intensity.
    pub color_intensity: [f32; 4],
}

impl PointLightUniformData {
    pub fn new(position: Vec3, color: Vec3, intensity: f32, range: f32) -> Self {
        Self {
            position_range: [position.x, position.y, position.z, range],
            color_intensity: [color.x, color.y, color.z, intensity],
        }
    }
}

/// Combined lighting uniform — one directional, fixed-cap spot and
/// point arrays, and per-type active counts.
///
/// `counts.x` = directional count (0 or 1), `.y` = spot count,
/// `.z` = point count, `.w` = shadow PCF half-kernel (0 / 1 / 2,
/// driven by `DebugState::pcf_kernel`). The shader iterates each
/// light array `..counts.[y|z]`; trailing slots are stale data
/// and never read.
///
/// Spot lights grew to 128 B (each carries its own shadow VP);
/// totals are 48 + 8 × 128 + 16 × 32 + 16 = 1600 B. Still
/// comfortably below the 64 KB uniform-buffer limit. If indoor
/// scenes ever push past this scale we'd move to a storage
/// buffer; uniform is the right call for now.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct LightsUniformData {
    pub directional: DirectionalLightUniformData,
    pub spots: [SpotLightUniformData; MAX_SPOT_LIGHTS],
    pub points: [PointLightUniformData; MAX_POINT_LIGHTS],
    pub counts: [u32; 4],
}

impl LightsUniformData {
    /// All-zero buffer. Used both for the GPU buffer's initial
    /// contents and as the base the per-frame packing fills into.
    pub fn zeroed() -> Self {
        <Self as Zeroable>::zeroed()
    }

    /// Fallback for the "no DirectionalLight in world" case.
    /// Surfaces a flat grey ambient so geometry is visible without
    /// authoring effort; no directional contribution, no spots, no
    /// points. Mirrors Game 0's `LightUniformData::placeholder`
    /// behaviour: the *value* lived on the directional even when
    /// no directional existed, simply because the ambient slot has
    /// to live somewhere.
    pub fn placeholder() -> Self {
        let mut data = Self::zeroed();
        data.directional = DirectionalLightUniformData::new(
            Vec3::new(0.0, -1.0, 0.0),
            Vec3::ZERO,
            0.0,
            Vec3::splat(0.3),
        );
        // counts.x = 0 — the shader's directional-contribution
        // branch stays off; only ambient lights the scene.
        data
    }
}

/// Per-draw model + material uniform.
///
/// Carries everything a single draw needs that varies per entity:
/// the model matrix (for vertex transform) and the shading params
/// (albedo, roughness, emissive). `ForwardPipeline` packs many of
/// these into one buffer indexed by dynamic offset — see
/// `pipeline.rs` for the alignment story.
///
/// Layout in `[f32; 4]` slots:
/// - `model` — 64 B (4 × vec4)
/// - `albedo_roughness` — 16 B (`xyz` = albedo, `w` = roughness)
/// - `emissive` — 16 B (`xyz` = emissive color, `w` = intensity)
///
/// Total 96 B, well under the 256-byte dynamic-offset stride.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct ModelUniformData {
    pub model: [[f32; 4]; 4],
    pub albedo_roughness: [f32; 4],
    pub emissive: [f32; 4],
}

impl ModelUniformData {
    pub fn new(model: Mat4, material: &Material) -> Self {
        Self {
            model: model.to_cols_array_2d(),
            albedo_roughness: [
                material.albedo.x,
                material.albedo.y,
                material.albedo.z,
                material.roughness,
            ],
            emissive: [
                material.emissive.x,
                material.emissive.y,
                material.emissive.z,
                material.emissive_intensity,
            ],
        }
    }
}
