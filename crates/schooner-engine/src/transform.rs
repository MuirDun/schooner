//! Spatial pose component shared across subsystems.
//!
//! Architecture: see `architecture/render.md` — `Transform` is a
//! scene-graph primitive, not a render concept. The renderer reads
//! it to build model matrices; the camera (Phase G) reads it for
//! the view matrix and writes it from `fps_look` / `fps_move`;
//! physics (Game 1) writes it from Rapier each step; audio sources
//! and AI perception (later) read it for spatial math. It lives at
//! the engine crate root rather than under any one subsystem so no
//! later module has to depend on the renderer just to get a pose.
//!
//! Compose order is **translation × rotation × scale** — the
//! conventional TRS that matches glTF, Unity, and Unreal authoring
//! tools. A non-uniform `scale` followed by a `rotation` would shear
//! the result, which is why scale is innermost.

use glam::{Mat4, Quat, Vec3};

/// Position, orientation, and scale of an entity in world space.
///
/// `Default` is the identity pose at the world origin: zero
/// translation, identity rotation, unit scale. Spawning an entity
/// without an explicit `Transform` then `insert`-ing one with
/// `Transform::default()` is the intended idiom.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transform {
    pub translation: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
}

impl Default for Transform {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Transform {
    pub const IDENTITY: Self = Self {
        translation: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        scale: Vec3::ONE,
    };

    pub fn from_translation(translation: Vec3) -> Self {
        Self {
            translation,
            ..Self::IDENTITY
        }
    }

    pub fn from_rotation(rotation: Quat) -> Self {
        Self {
            rotation,
            ..Self::IDENTITY
        }
    }

    pub fn from_scale(scale: Vec3) -> Self {
        Self {
            scale,
            ..Self::IDENTITY
        }
    }

    /// Build the model matrix for this pose: `T * R * S`.
    ///
    /// `glam::Mat4::from_scale_rotation_translation` composes in
    /// exactly that order, which is the convention the renderer's
    /// vertex shader expects.
    pub fn matrix(&self) -> Mat4 {
        Mat4::from_scale_rotation_translation(self.scale, self.rotation, self.translation)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_identity() {
        let t = Transform::default();
        assert_eq!(t.translation, Vec3::ZERO);
        assert_eq!(t.rotation, Quat::IDENTITY);
        assert_eq!(t.scale, Vec3::ONE);
    }

    #[test]
    fn identity_matrix_is_mat4_identity() {
        assert_eq!(Transform::IDENTITY.matrix(), Mat4::IDENTITY);
    }

    #[test]
    fn translation_only_matrix_translates_origin() {
        let t = Transform::from_translation(Vec3::new(1.0, 2.0, 3.0));
        let p = t.matrix().transform_point3(Vec3::ZERO);
        assert_eq!(p, Vec3::new(1.0, 2.0, 3.0));
    }

    #[test]
    fn scale_only_matrix_scales_unit_x() {
        let t = Transform::from_scale(Vec3::new(2.0, 3.0, 4.0));
        let p = t.matrix().transform_point3(Vec3::new(1.0, 1.0, 1.0));
        assert_eq!(p, Vec3::new(2.0, 3.0, 4.0));
    }

    #[test]
    fn compose_order_is_translation_rotation_scale() {
        // A 90° rotation around Y maps +X → -Z. Applied to a point
        // that has been scaled by 2 along X, the result lands at
        // -2 along Z; then translation adds (10, 0, 0).
        let t = Transform {
            translation: Vec3::new(10.0, 0.0, 0.0),
            rotation: Quat::from_rotation_y(std::f32::consts::FRAC_PI_2),
            scale: Vec3::new(2.0, 1.0, 1.0),
        };
        let p = t.matrix().transform_point3(Vec3::new(1.0, 0.0, 0.0));
        // Floating-point: rotation introduces tiny epsilons.
        assert!((p - Vec3::new(10.0, 0.0, -2.0)).length() < 1e-5);
    }
}
