//! Per-scene atmospheric fog.
//!
//! Inserted as a `World` resource by `App::resumed` with
//! [`Fog::DEFAULT`] (off — `density = 0`). The forward pass reads it
//! once per frame and folds the parameters into the lighting uniform
//! at `@group(1)` — fog and lights live in the same bind group
//! because 1.E.2's god-ray loop reads both together inside the spot
//! iteration, and a single uniform avoids a redundant binding.
//!
//! ## The formula
//!
//! Density falls off exponentially with height above `base_height`:
//!
//! ```text
//! ρ(y) = density · exp(-falloff · (y − base_height))
//! ```
//!
//! For a view ray from camera C to fragment P with vertical delta
//! `Δy = Py − Cy`, the optical depth along the segment is the
//! analytic integral (Wenzel 2007, *Real-time Atmospheric Effects in
//! Games Revisited*, GPU Gems 2 §16):
//!
//! ```text
//! ρ_C = density · exp(-falloff · (Cy − base_height))
//! τ   = ρ_C · (1 − exp(-falloff · Δy)) / (falloff · Δy) · length(P − C)
//! transmittance = exp(-τ)
//! final = mix(fog_color, lit, transmittance)
//! ```
//!
//! The `(falloff · Δy)` divisor degenerates for horizontal rays; the
//! shader guards with the Taylor limit `(1 − e^x)/x → 1` as `x → 0`.
//!
//! ## Why exponential height fog, not constant density
//!
//! A constant-density fog has no vertical structure — looking up
//! reads the same as looking forward, which reads as "lens dirt,"
//! not "atmosphere." Height fog gives the floor a denser layer that
//! thins overhead — the Gothic / Witcher mood. Same math drives
//! outdoor sunset fog in Game 3 with a different `color` and
//! `base_height`.
//!
//! ## Why post-lighting, pre-tonemap
//!
//! Fog blends in HDR linear space against the *lit* color, so it
//! affects both shaded and emissive contributions (a glowing red
//! lamp behind dense fog dims to a red haze). The blend happens at
//! the end of the forward shader's `fs_main`; the post chain then
//! tonemaps the already-foggy image, so a heavily-fogged scene
//! tonemaps gracefully rather than getting tonemapped first and
//! then mixed against a fog color that doesn't share the curve.
//!
//! ## Scattering and god-rays (1.E.2)
//!
//! The `scattering` field drives single-scatter in-scattering through
//! the same medium. The forward shader computes, for each spot light
//! and each fragment, the segment of the view ray that lies inside
//! the spot's cone (analytic ray-cone intersection, no raymarch),
//! evaluates the in-scatter at the segment midpoint, and adds it
//! additively to the foggy scene color. The per-spot dial is
//! `SpotLight::god_ray_intensity` — a multiplier on `scattering`
//! that lets one beam read brighter than another in the same
//! medium without breaking the global atmosphere. See
//! `architecture/rendering.md` and `light.rs` for the wider story.
//!
//! Decoupled from density on purpose: physically, doubling density
//! doubles both extinction and scattering, but games (UE, Frostbite)
//! split these knobs because authors want "thick atmosphere with
//! subtle glow" and "thin atmosphere with bright god-rays" as
//! independent moods. The shader's god-ray formula still multiplies
//! by `density_at_midpoint` so `density = 0` (no medium) correctly
//! produces no in-scatter regardless of `scattering`.

use glam::Vec3;

/// Per-scene atmospheric fog. See module docs for the formula.
///
/// Disabled state is `density = 0`, which makes every other field a
/// no-op (the shader short-circuits to transmittance = 1). App seeds
/// the resource with [`Fog::DEFAULT`] so unmodified scenes render
/// without fog.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fog {
    /// RGB the fog converges to at infinity — "what the room
    /// dissolves into."
    pub color: Vec3,
    /// World-space y at which density equals `density`. Above this
    /// height density falls off exponentially.
    pub base_height: f32,
    /// Density coefficient at `base_height`. Higher = thicker fog.
    /// `0` disables fog entirely (shader short-circuits).
    pub density: f32,
    /// Exponential height falloff (1/units). `0` = uniform density
    /// across all heights; larger values concentrate fog near
    /// `base_height` and thin it overhead.
    pub falloff: f32,
    /// Single-scatter coefficient for spot god-rays — see module
    /// docs. Decoupled from `density` so authors can tune extinction
    /// and beam visibility independently. The shader still gates
    /// on `density > 0` (no medium ⇒ no in-scatter), so this is a
    /// scale on a contribution that's already conditional on the
    /// fog being present.
    pub scattering: f32,
}

impl Fog {
    /// Fog disabled — `density = 0` makes every other field a no-op.
    pub const DEFAULT: Self = Self {
        color: Vec3::ZERO,
        base_height: 0.0,
        density: 0.0,
        falloff: 0.0,
        scattering: 0.0,
    };
}

impl Default for Fog {
    fn default() -> Self {
        Self::DEFAULT
    }
}
