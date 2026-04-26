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

/// Directional light uniform.
///
/// Direction is stored as the direction the light **travels** (the
/// shader negates it on the way in). All three vectors are stored
/// as `vec4` so the std140 layout is identical to the WGSL struct
/// without manual padding.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct LightUniformData {
    pub direction: [f32; 4],
    pub color: [f32; 4],
    pub ambient: [f32; 4],
}

impl LightUniformData {
    pub fn new(direction: Vec3, color: Vec3, ambient: Vec3) -> Self {
        Self {
            direction: [direction.x, direction.y, direction.z, 0.0],
            color: [color.x, color.y, color.z, 0.0],
            ambient: [ambient.x, ambient.y, ambient.z, 0.0],
        }
    }

    /// Fallback when no `DirectionalLight` exists in the world.
    /// Pure ambient grey so geometry stays visible without a sun.
    pub fn placeholder() -> Self {
        Self::new(Vec3::new(0.0, -1.0, 0.0), Vec3::ZERO, Vec3::splat(0.3))
    }
}

/// Per-draw model matrix uniform.
///
/// Stored alone in its own struct because `ForwardPipeline` packs
/// many of these into a single buffer indexed by dynamic offset —
/// see `pipeline.rs` for the alignment story.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct ModelUniformData {
    pub model: [[f32; 4]; 4],
}

impl ModelUniformData {
    pub fn from_matrix(model: Mat4) -> Self {
        Self {
            model: model.to_cols_array_2d(),
        }
    }
}
