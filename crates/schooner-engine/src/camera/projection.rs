//! Camera projection and active-camera tag.
//!
//! Two pieces:
//! - [`Camera`] — projection params on the entity. The view matrix
//!   is derived from the entity's `Transform`, not stored here:
//!   one source of truth for pose, one transform to update from
//!   `fps_look` / `fps_move` (Phase G) or physics (Game 1).
//! - [`ActiveCamera`] — zero-sized tag marking which camera entity
//!   the renderer should drive. Multiple cameras can live in the
//!   world (split-screen, security cameras, debug fly-cam); only
//!   the one tagged `ActiveCamera` produces the frame.
//!
//! Why a zero-sized tag rather than a `Resource<EntityId>` pointing
//! at the active camera: the ECS query path is the renderer's
//! existing read mechanism. `Query<(&Transform, &Camera, &ActiveCamera)>`
//! drops out of the same join machinery used for renderable meshes.
//! A resource pointer would couple the renderer to the resources
//! API and require a stale-handle check the query cannot produce.

use glam::Mat4;

/// Projection model for a camera. `Perspective` is the only Game 0
/// shape; `Orthographic` is named here so adding it later (debug
/// top-down view, UI cameras) doesn't require changing the
/// `Camera` API.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Projection {
    Perspective {
        /// Vertical FOV in radians. Stored as radians, not degrees,
        /// so the projection-matrix path doesn't repeat the
        /// degrees-to-radians conversion every frame.
        fov_y_radians: f32,
        near: f32,
        far: f32,
    },
}

impl Projection {
    /// Build the projection matrix for the given viewport aspect
    /// ratio. Uses wgpu's NDC depth convention (`0..1`, not
    /// OpenGL's `-1..1`); this is `glam::Mat4::perspective_rh`.
    pub fn matrix(&self, aspect: f32) -> Mat4 {
        match *self {
            Self::Perspective {
                fov_y_radians,
                near,
                far,
            } => Mat4::perspective_rh(fov_y_radians, aspect, near, far),
        }
    }

    /// Sensible Game 0 default: 60° vertical FOV, 0.1m near, 1km
    /// far. Reverse-Z and a tighter far plane for outdoor scenes
    /// land in Game 3.
    pub fn perspective_default() -> Self {
        Self::Perspective {
            fov_y_radians: 60_f32.to_radians(),
            near: 0.1,
            far: 1000.0,
        }
    }
}

/// Camera projection parameters.
///
/// Pose lives on the same entity's `Transform`. The active camera's
/// view matrix is `Transform.matrix().inverse()` — the view matrix
/// is the inverse of the camera's world-space pose, which is what
/// puts world-space geometry into camera space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera {
    pub projection: Projection,
}

impl Camera {
    pub fn new(projection: Projection) -> Self {
        Self { projection }
    }

    pub fn perspective_default() -> Self {
        Self::new(Projection::perspective_default())
    }
}

impl Default for Camera {
    fn default() -> Self {
        Self::perspective_default()
    }
}

/// Marker tag: this entity's camera is the one the renderer drives.
///
/// Zero-sized so attaching it costs no storage beyond the sparse-set
/// slot. The renderer's `Query<(&Transform, &Camera, &ActiveCamera)>`
/// returns at most one entity per frame in well-formed scenes;
/// having two tagged cameras is a user error the renderer treats as
/// "first one wins" rather than panicking.
#[derive(Debug, Clone, Copy, Default)]
pub struct ActiveCamera;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn perspective_default_uses_60_degrees() {
        let Projection::Perspective {
            fov_y_radians,
            near,
            far,
        } = Projection::perspective_default();
        assert!((fov_y_radians - 60_f32.to_radians()).abs() < 1e-6);
        assert_eq!(near, 0.1);
        assert_eq!(far, 1000.0);
    }

    #[test]
    fn perspective_matrix_matches_glam() {
        let proj = Projection::Perspective {
            fov_y_radians: 1.0,
            near: 0.1,
            far: 100.0,
        };
        let aspect = 16.0 / 9.0;
        assert_eq!(proj.matrix(aspect), Mat4::perspective_rh(1.0, aspect, 0.1, 100.0));
    }
}
