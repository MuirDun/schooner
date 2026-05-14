//! Per-instance shading parameters.
//!
//! Architecture: see `architecture/rendering.md` — Kinesis is the
//! game that introduces per-instance material params (Game 0 hard-
//! coded a single white Blinn–Phong tint). `Material` sits next to
//! `Transform` at the engine root because, like pose, several
//! subsystems will read and write it: the renderer consumes it for
//! shading each draw, later games' status-effect systems (`Wet`,
//! `Burning` in Game 3) will mutate it, and authored content (the
//! glTF loader in Phase 1.F) will populate it from disk. Keeping it
//! out of `render/` avoids forcing later consumers to depend on the
//! renderer just to describe a surface.
//!
//! `Material` is an *optional* ECS component: meshes without one
//! render against `Material::DEFAULT` (white albedo, mid roughness,
//! no emissive, opaque). That keeps Game 0's spawn sites — and any
//! future debug/helper geometry — rendering unchanged.

use glam::Vec3;

/// Per-draw surface parameters consumed by the forward shader.
///
/// `roughness` is a Blinn–Phong analogue, not a PBR term: it
/// modulates the specular lobe width. Lower values produce a
/// tighter, brighter highlight; higher values broaden and dim it.
/// The architecture vision is permanently Blinn–Phong + warm grade
/// (no PBR even in Game 5), so this stays a single scalar rather
/// than the GGX/metallic split a PBR pipeline would need.
///
/// `emissive` is added *outside* the lighting equation — it is the
/// surface's own light, not a reflection. Red lamps and the glowing
/// food gel use this; the Mahli eye behind the frosted window will
/// in Phase 3.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Material {
    pub albedo: Vec3,
    pub roughness: f32,
    pub emissive: Vec3,
    pub emissive_intensity: f32,
    pub blend: BlendMode,
}

impl Default for Material {
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl Material {
    /// Neutral opaque surface: white albedo, mid roughness, no
    /// emissive. The render system falls back to this for entities
    /// that carry a `MeshHandle` but no `Material` component.
    pub const DEFAULT: Self = Self {
        albedo: Vec3::ONE,
        roughness: 0.5,
        emissive: Vec3::ZERO,
        emissive_intensity: 0.0,
        blend: BlendMode::Opaque,
    };
}

/// How a material's fragments combine with the framebuffer.
///
/// Only `Opaque` is consumed by the forward pass in Phase 1.A.
/// `AlphaBlend` is defined now — and pattern-matched everywhere it
/// must be — so the public component type doesn't change shape
/// when Phase 1.G wires the transparent pass; authors can already
/// label decals and glass with the eventual variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlendMode {
    Opaque,
    AlphaBlend,
}

impl Default for BlendMode {
    fn default() -> Self {
        Self::Opaque
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_neutral_opaque() {
        let m = Material::default();
        assert_eq!(m.albedo, Vec3::ONE);
        assert_eq!(m.roughness, 0.5);
        assert_eq!(m.emissive, Vec3::ZERO);
        assert_eq!(m.emissive_intensity, 0.0);
        assert_eq!(m.blend, BlendMode::Opaque);
    }

    #[test]
    fn default_matches_const() {
        assert_eq!(Material::default(), Material::DEFAULT);
    }

    #[test]
    fn blend_mode_default_is_opaque() {
        assert_eq!(BlendMode::default(), BlendMode::Opaque);
    }
}
