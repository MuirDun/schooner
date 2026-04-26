//! Lighting components.
//!
//! Game 0 supports exactly one light type: an infinitely-distant
//! directional source modeling sunlight. The shader iterates the
//! `DirectionalLight` query and uses the first one it finds, so a
//! scene with two tagged directional lights is a user error
//! resolved as "first one wins" rather than a panic. Point and
//! spot lights — and the shadow-map pipeline that justifies them —
//! land in Game 2.

use glam::Vec3;

/// Infinitely-distant directional light.
///
/// `direction` is the direction the light **travels** (i.e. the
/// vector from sun to surface), not the surface-to-sun vector. The
/// shader negates it on the way in to compute `dot(N, L)`. Stored
/// pre-normalization is fine because the shader normalizes before
/// the dot product; making the user pass a unit vector here would
/// just push the burden to authoring code.
///
/// `color` is the linear RGB radiance of the light. `ambient` is a
/// flat term added to every fragment regardless of normal — a
/// stand-in for indirect lighting until Game 5 introduces global
/// illumination.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DirectionalLight {
    pub direction: Vec3,
    pub color: Vec3,
    pub ambient: Vec3,
}

impl DirectionalLight {
    /// Sensible Game 0 default: light traveling down-and-forward,
    /// neutral white, low-grey ambient. Picked so a scene with the
    /// default camera and the built-in cube shows visible shading
    /// without authoring effort.
    pub fn sun() -> Self {
        Self {
            direction: Vec3::new(-0.4, -1.0, -0.3),
            color: Vec3::splat(1.0),
            ambient: Vec3::splat(0.1),
        }
    }
}

impl Default for DirectionalLight {
    fn default() -> Self {
        Self::sun()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sun_default_points_downward() {
        // The default light must have a non-trivial downward
        // component — a sun pointing sideways or up would leave
        // the floor unlit, which defeats the point of a default.
        let sun = DirectionalLight::sun();
        assert!(sun.direction.y < 0.0);
    }

    #[test]
    fn sun_default_has_visible_ambient() {
        // Zero ambient on the default would leave shadow-side
        // surfaces pure black, which reads as broken rather than
        // unlit. Tiny but non-zero ambient is the right baseline.
        let sun = DirectionalLight::sun();
        assert!(sun.ambient.element_sum() > 0.0);
    }
}
