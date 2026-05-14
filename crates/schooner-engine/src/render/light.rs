//! Lighting components.
//!
//! Game 0 supported exactly one directional source. Game 1
//! (Kinesis) adds spot and point lights — overhead chamber lamps,
//! service-corridor red points, the dim spot behind the eye-window
//! cavity. Shadow maps for spot lights arrive in Phase 1.C; this
//! module is shapes-only.
//!
//! ## Light-type conventions
//!
//! - **Directional**: positionless (at infinity). Carries its own
//!   `direction` field — does not pair with `Transform`. The
//!   renderer uses the first `DirectionalLight` it finds.
//! - **Spot**: positioned. Pairs with a sibling `Transform`.
//!   Position is `Transform.translation`; direction is
//!   `Transform.rotation * Vec3::NEG_Z` — i.e. the spot shines
//!   along its local -Z axis by default, matching the camera-
//!   forward convention. Aim by rotating the transform.
//! - **Point**: positioned. Pairs with a sibling `Transform`;
//!   only `Transform.translation` is consumed.
//!
//! ## `color` vs `intensity`
//!
//! All lights split unit-magnitude tint (`color`) from a scalar
//! multiplier (`intensity`). Adjusting brightness without
//! retouching tint is a frequent enough authoring move that the
//! split earns its keep. Final contribution per draw is roughly
//! `albedo * color * intensity * attenuation * dot(N, L)`.

use glam::Vec3;

/// Infinitely-distant directional light.
///
/// `direction` is the direction the light **travels** (sun →
/// surface), not the surface-to-sun vector. Stored pre-normalization;
/// the shader normalizes.
///
/// `ambient` is a flat term added to every fragment regardless of
/// normal — a stand-in for indirect lighting. Lives on
/// `DirectionalLight` rather than a separate resource because the
/// sun is the natural anchor for hemisphere-style ambient; per-zone
/// ambient becomes the `ColorGrade` resource's job in Phase 1.D.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DirectionalLight {
    pub direction: Vec3,
    pub color: Vec3,
    pub intensity: f32,
    pub ambient: Vec3,
}

impl DirectionalLight {
    /// Sensible default: light traveling down-and-forward, neutral
    /// white at unit intensity, low-grey ambient. Picked so a scene
    /// with the default camera and the built-in cube shows visible
    /// shading without authoring effort.
    pub fn sun() -> Self {
        Self {
            direction: Vec3::new(-0.4, -1.0, -0.3),
            color: Vec3::splat(1.0),
            intensity: 1.0,
            ambient: Vec3::splat(0.1),
        }
    }
}

impl Default for DirectionalLight {
    fn default() -> Self {
        Self::sun()
    }
}

/// Cone-shaped light with position + direction taken from a sibling
/// `Transform`.
///
/// **Authoring contract**: spawn the entity with both a `Transform`
/// and a `SpotLight`. Position is `Transform.translation`; direction
/// is `Transform.rotation * Vec3::NEG_Z`.
///
/// Cone falloff is stored as cosines of the inner and outer half-
/// angles so the fragment shader can compute the cone factor as
/// `smoothstep(outer_cone_cos, inner_cone_cos, dot(L, spot_dir))`
/// without a per-fragment `cos()`. Use [`SpotLight::new`] to author
/// in degrees; the conversion happens once at construction.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpotLight {
    pub color: Vec3,
    pub intensity: f32,
    pub range: f32,
    pub inner_cone_cos: f32,
    pub outer_cone_cos: f32,
}

impl SpotLight {
    /// Build a spot light from degree-valued cone half-angles.
    /// `inner_deg` is the angle within which the light is at full
    /// strength; between `inner_deg` and `outer_deg` it falls off
    /// smoothly; beyond `outer_deg` it contributes nothing.
    pub fn new(color: Vec3, intensity: f32, range: f32, inner_deg: f32, outer_deg: f32) -> Self {
        Self {
            color,
            intensity,
            range,
            inner_cone_cos: inner_deg.to_radians().cos(),
            outer_cone_cos: outer_deg.to_radians().cos(),
        }
    }
}

impl Default for SpotLight {
    fn default() -> Self {
        // Indoor lamp default: neutral white, modest indoor reach,
        // 20°/30° cone (tight beam with a visible soft edge).
        Self::new(Vec3::splat(1.0), 1.0, 10.0, 20.0, 30.0)
    }
}

/// Omnidirectional light at a position taken from a sibling
/// `Transform`.
///
/// **Authoring contract**: spawn the entity with both a `Transform`
/// and a `PointLight`. Only `Transform.translation` is consumed.
///
/// `range` controls a soft cutoff at the edge of the lit volume;
/// the fragment shader uses inverse-square attenuation with a
/// windowing function so brightness falls smoothly to zero at
/// `range`, avoiding the `1/d²` blow-up near the source.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PointLight {
    pub color: Vec3,
    pub intensity: f32,
    pub range: f32,
}

impl PointLight {
    pub fn new(color: Vec3, intensity: f32, range: f32) -> Self {
        Self {
            color,
            intensity,
            range,
        }
    }
}

impl Default for PointLight {
    fn default() -> Self {
        Self::new(Vec3::splat(1.0), 1.0, 10.0)
    }
}

/// Marker component requesting that a light cast shadows.
///
/// Attach to a `SpotLight` entity to opt that light into the
/// depth-only shadow pass. Lights without this marker pay no
/// shadow cost — both bandwidth (no depth render) and shading
/// (the forward shader skips the comparison sample).
///
/// Indoor Kinesis scenes typically want one shadow caster (the
/// overhead chamber spot); rarely two. The runtime cap lives at
/// [`crate::render::shadow::MAX_SHADOW_CASTERS`].
///
/// Point-light shadows are not supported in Phase 1.C — they need
/// cube maps (six render passes per light), and the only point
/// lights in Kinesis are dim service-corridor accents that don't
/// carry visible shadowing anyway.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Shadowcaster;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sun_default_points_downward() {
        // A sun pointing sideways or up would leave the floor
        // unlit, which defeats the point of a default.
        let sun = DirectionalLight::sun();
        assert!(sun.direction.y < 0.0);
    }

    #[test]
    fn sun_default_has_visible_ambient() {
        // Zero ambient would leave shadow-side surfaces pure black,
        // which reads as broken rather than unlit.
        let sun = DirectionalLight::sun();
        assert!(sun.ambient.element_sum() > 0.0);
    }

    #[test]
    fn sun_default_has_unit_intensity() {
        // The split between color and intensity means the default
        // intensity is 1.0 — color carries the unit tint.
        assert_eq!(DirectionalLight::sun().intensity, 1.0);
    }

    #[test]
    fn spot_cone_cosines_are_monotone() {
        // inner_deg < outer_deg ⇒ cos(inner) > cos(outer), because
        // cos is monotonically decreasing on [0, π]. If we ever get
        // this backwards the shader's smoothstep produces an
        // inverted falloff.
        let spot = SpotLight::new(Vec3::ONE, 1.0, 10.0, 20.0, 30.0);
        assert!(spot.inner_cone_cos > spot.outer_cone_cos);
    }

    #[test]
    fn spot_new_converts_degrees_to_cosines() {
        let spot = SpotLight::new(Vec3::ONE, 1.0, 10.0, 30.0, 30.0);
        let expected = (30.0_f32).to_radians().cos();
        assert!((spot.inner_cone_cos - expected).abs() < 1e-6);
        assert!((spot.outer_cone_cos - expected).abs() < 1e-6);
    }

    #[test]
    fn point_default_has_unit_intensity() {
        assert_eq!(PointLight::default().intensity, 1.0);
    }
}
