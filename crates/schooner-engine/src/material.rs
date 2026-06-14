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

use glam::{Vec2, Vec3};

use crate::render::texture::TextureHandle;

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
/// later.
///
/// `albedo_texture` is multiplied into the per-fragment albedo at
/// sample time. `None` binds the engine's WHITE 1×1 built-in so the
/// shader's textured and untextured paths stay uniform — a `Material`
/// with no texture reads `albedo = albedo_tint × white = albedo_tint`,
/// identical to the pre-texture pipeline's behaviour.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Material {
    pub albedo: Vec3,
    pub roughness: f32,
    pub emissive: Vec3,
    pub emissive_intensity: f32,
    pub blend: BlendMode,
    /// Surface coverage in `[0, 1]`, multiplied into the texture's
    /// alpha channel to give the final fragment alpha. `1.0` is fully
    /// opaque. Only meaningful under `BlendMode::AlphaBlend` — the
    /// opaque pipeline uses REPLACE blend and ignores alpha — so an
    /// `Opaque` material with `opacity < 1` still renders solid. Lets
    /// a flat-tinted surface (no authored alpha texture) fade uniformly:
    /// glass panes and decal fades drive this.
    pub opacity: f32,
    /// World-space depth bias in metres: the vertex shader nudges this
    /// surface toward the camera *along the view ray* by this amount
    /// before the depth test, so screen position is unchanged but depth
    /// shrinks. `0.0` (default) is no bias. Used by decal quads that sit
    /// coplanar with a host surface — a few millimetres lets the decal
    /// win `depth_compare: Less` against the wall it's painted on,
    /// killing the coplanar z-fighting without the decal visibly
    /// floating off the surface.
    pub depth_bias: f32,
    /// Schlick-Fresnel rim strength in `[0, 1]`. `0.0` (default) is a
    /// matte dielectric — no rim reflection, the look of every opaque
    /// surface and flat decal. Above zero, the shader lifts both the
    /// specular highlight and the fragment alpha toward 1 at grazing
    /// view angles via Schlick's `(1 − n·v)^5`, so the surface reads as
    /// glass: clear face-on, reflective and opaque along its silhouette.
    /// This is the only knob that turns a translucent pane into frosted
    /// glass; pair it with a low `opacity` and low `roughness`.
    pub fresnel: f32,
    pub albedo_texture: Option<TextureHandle>,
    /// Per-axis UV tiling: the vertex shader multiplies mesh UVs by this
    /// before interpolation, so `(4, 2)` repeats the texture 4× across U
    /// and 2× across V. `(1, 1)` (default) is the authored mapping. The
    /// material sampler is `Repeat`, so any value past 1 wraps. Per-axis
    /// (not a scalar) keeps texels square on non-square surfaces — a wall
    /// that is wider than it is tall tiles correctly. Textures are
    /// single-mip until Game 2A, so very high tiling shimmers at distance.
    pub uv_scale: Vec2,
    /// UV translation applied after `uv_scale`: `uv * scale + offset`.
    /// `(0, 0)` (default) is no shift. Lets coplanar decals or atlas
    /// sub-rects pick a region without re-authoring the mesh UVs.
    pub uv_offset: Vec2,
    /// Tangent-space normal map perturbing the per-fragment normal. The
    /// texture is *data, not color* — uploaded `Rgba8Unorm` (linear), not
    /// sRGB — and decoded `xyz * 2 - 1`. `None` binds the engine's
    /// FLAT_NORMAL 1×1 built-in `(0, 0, 1)`, so the perturbation is the
    /// identity and the shader's mapped/unmapped paths stay uniform.
    pub normal_texture: Option<TextureHandle>,
    /// Scales the normal map's tangent-space `xy` before the TBN rotate,
    /// so `0` is flat (geometric normal) and `1` is full authored relief.
    /// Keep this *low* (≈0.3–0.6) for the HL2/Gothic look — subtle relief
    /// raked by hard light, not high-frequency bump. `1.0` is the default
    /// so an authored map reads at full strength unless dialed down.
    pub normal_strength: f32,
    /// When `true`, the surface is textured by **world-space triplanar
    /// projection** instead of mesh UVs: albedo and normal are sampled on
    /// the three world planes and blended by the geometric normal. This
    /// makes the texture continuous across separately-spawned boxes (no
    /// per-box UV seam, no thin-reveal stretching) — the right mode for
    /// procedural architecture (walls, floors). `uv_scale.x` is then read
    /// as world *repeats per metre* (the y/offset are ignored). `false`
    /// (default) keeps ordinary UV mapping for authored meshes (the eye).
    pub triplanar: bool,
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
        opacity: 1.0,
        depth_bias: 0.0,
        fresnel: 0.0,
        albedo_texture: None,
        uv_scale: Vec2::ONE,
        uv_offset: Vec2::ZERO,
        normal_texture: None,
        normal_strength: 1.0,
        triplanar: false,
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
        assert_eq!(m.opacity, 1.0);
        assert_eq!(m.depth_bias, 0.0);
        assert_eq!(m.fresnel, 0.0);
        assert_eq!(m.albedo_texture, None);
        assert_eq!(m.uv_scale, Vec2::ONE);
        assert_eq!(m.uv_offset, Vec2::ZERO);
        assert_eq!(m.normal_texture, None);
        assert_eq!(m.normal_strength, 1.0);
        assert!(!m.triplanar);
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
