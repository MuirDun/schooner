//! Fullscreen post-process overlay slot — the compositor layer that
//! sits on top of the graded, vignetted frame.
//!
//! Inserted as a `World` resource by `App::resumed` with
//! [`PostOverlay::DEFAULT`] (off — `intensity = 0`, no texture). The
//! post pass composites it **last**, after tonemap → grade → vignette,
//! then the final LDR clamp.
//!
//! This is the slot Parts 3–4 drive for three named effects — the
//! triad is fixed up front so a later consumer can't hit a blend the
//! slot can't express:
//!
//! - **Death red-noise** (Part 3) — [`OverlayBlend::AlphaBlend`]; the
//!   texture's alpha channel masks the red-noise pattern to specific
//!   screen regions.
//! - **Hunger tint** (Part 4) — [`OverlayBlend::Multiply`]; a warm
//!   tint modulates the entire frame.
//! - **Flashes** (instrument-scene glow, lightning) —
//!   [`OverlayBlend::Additive`]; light added on top, allowed to blow
//!   past white.
//!
//! ## Why a `TextureHandle`, not an owned texture
//!
//! The overlay texture lives in the [`TextureRegistry`] like every
//! other texture, so it loads through the same PNG path and is
//! F5-reloadable. The post pass resolves `handle → view` each frame,
//! exactly as the forward pass resolves a material's albedo. The
//! alternative — `PostOverlay` owning its own `TextureGpu` — would
//! duplicate the texture-ownership story for no gain.
//!
//! ## Why "off" is `intensity = 0`, not `texture = None` alone
//!
//! A wgpu pipeline layout binds all its groups or none: every group
//! the layout declares must be set before each `draw`, every frame —
//! there is no optional bind group. So the post pass binds *some*
//! texture at the overlay's `@group(2)` on every frame, falling back
//! to the engine's WHITE 1×1 built-in when `texture` is `None`. "Off"
//! is therefore expressed as `intensity = 0` in the params uniform,
//! which makes the shader's overlay term a no-op (`mix(c, …, 0) = c`)
//! regardless of what texture happens to be bound — the same
//! WHITE-when-absent trick the forward pass uses for albedo.
//!
//! [`TextureRegistry`]: crate::render::registry::TextureRegistry

use crate::render::texture::TextureHandle;

/// How the overlay texture composites onto the graded frame.
///
/// A new enum rather than a reuse of [`crate::material::BlendMode`]
/// (Opaque / AlphaBlend): that one drives the transparent-pass sort
/// for world geometry; this one is a fullscreen compositor with three
/// distinct mathematical operations. Different domain, different
/// variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayBlend {
    /// `mix(scene, tex.rgb, tex.a * intensity)` — the texture's alpha
    /// masks where it lands. Death red-noise.
    AlphaBlend,
    /// `mix(scene, scene * tex.rgb, intensity)` — overlay modulates
    /// the frame. Hunger warm-tint.
    Multiply,
    /// `scene + tex.rgb * intensity` — light added on top, may exceed
    /// white before the final clamp. Flashes.
    Additive,
}

impl OverlayBlend {
    /// Selector value the WGSL `apply_overlay` branches on. Written
    /// explicitly (not `self as u32`) so reordering the enum can never
    /// silently remap the shader's branch. `0` is unused on the CPU
    /// side and treated as pass-through by the shader, which keeps a
    /// future "none" sentinel free.
    pub fn shader_index(self) -> u32 {
        match self {
            OverlayBlend::AlphaBlend => 1,
            OverlayBlend::Multiply => 2,
            OverlayBlend::Additive => 3,
        }
    }
}

/// Per-scene post overlay parameters. See module docs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PostOverlay {
    /// Texture to composite. `None` binds WHITE and (with the default
    /// `intensity = 0`) renders nothing — see module docs on the
    /// all-groups-bound constraint.
    pub texture: Option<TextureHandle>,
    /// `0` = off (pass-through), `1` = full effect. The single knob
    /// that gates the whole slot: at `0` the overlay term is a no-op
    /// whatever the blend mode or bound texture.
    pub intensity: f32,
    /// Compositing math. Ignored while `intensity = 0`.
    pub blend: OverlayBlend,
}

impl PostOverlay {
    /// Overlay disabled — no texture, `intensity = 0`. App seeds the
    /// resource with this so unmodified scenes show no overlay.
    pub const DEFAULT: Self = Self {
        texture: None,
        intensity: 0.0,
        blend: OverlayBlend::AlphaBlend,
    };
}

impl Default for PostOverlay {
    fn default() -> Self {
        Self::DEFAULT
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_off() {
        let o = PostOverlay::default();
        assert_eq!(o.intensity, 0.0);
        assert_eq!(o.texture, None);
    }

    #[test]
    fn blend_indices_are_distinct_and_nonzero() {
        // 0 is reserved as the shader's pass-through sentinel, so no
        // real mode may map to it; and the three must be distinct or
        // the shader can't tell them apart.
        let a = OverlayBlend::AlphaBlend.shader_index();
        let m = OverlayBlend::Multiply.shader_index();
        let d = OverlayBlend::Additive.shader_index();
        assert!(a != 0 && m != 0 && d != 0);
        assert!(a != m && m != d && a != d);
    }
}
