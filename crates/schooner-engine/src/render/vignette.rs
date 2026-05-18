//! Per-scene vignette — radial darkening from screen centre out to
//! the corners, with optional color tint.
//!
//! Inserted as a `World` resource by `App::resumed` with
//! [`Vignette::DEFAULT`] (off — `intensity = 0`). The post pass
//! packs it into the post-params uniform alongside the color grade;
//! the WGSL applies it after grade and before overlay.
//!
//! ## Why a separate resource from `ColorGrade`
//!
//! Color grade is *colorist territory* — ASC CDL, film-pipeline-
//! portable, the look of the scene. Vignette is a *compositor
//! effect* — lens artifact, mood tool, often modulated separately
//! from the look (heavier in tense moments, off in calm ones).
//! Splitting them lets a single scene-look (e.g. service-red) carry
//! variable vignette without disturbing the grade.
//!
//! ## The math
//!
//! In screen UV space (UV in `[0, 1]`):
//!
//! ```text
//! dist  = length(uv - 0.5) * √2          // 0 at centre, 1 at corner
//! v     = smoothstep(inner, outer, dist) // 0 inside `inner`, 1 outside `outer`
//! out   = mix(color, tint, v * intensity)
//! ```
//!
//! UV-space distance gives an **elliptical** vignette that tracks
//! the viewport's aspect ratio — what UE/Unity/Godot do. A pure
//! circle (aspect-corrected) reads as "wrong" on widescreen because
//! the corners darken differently than the sides; nobody wants
//! that. `smoothstep` is a cubic ramp — reads as a soft optical
//! falloff rather than a hard edge.
//!
//! Historical note: real lenses have a `cos⁴` falloff (off-axis
//! light passes through the lens at an oblique angle and loses
//! energy). `cos⁴` is shader-expensive without a perceptual win —
//! the *shape* of `smoothstep` reads the same to a viewer.

use glam::Vec3;

/// Per-scene vignette parameters. See module docs for the formula.
///
/// `inner` and `outer` are radii in normalized corner-units: `0` at
/// screen centre, `1` at any corner. Fully bright inside `inner`,
/// fully tinted outside `outer`, smooth cubic ramp between.
/// `inner < outer` must hold or the smoothstep degenerates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vignette {
    /// `0` = off (pass-through), `1` = corners pure tint. Linear
    /// blend with the underlying color: `mix(color, tint, v * intensity)`.
    pub intensity: f32,
    /// Inner radius (corner-units). Pixels inside this radius are
    /// untouched.
    pub inner: f32,
    /// Outer radius (corner-units). Pixels outside this radius are
    /// at full `intensity * tint` blend.
    pub outer: f32,
    /// RGB the corners converge to. Usually black; warm/cold tints
    /// are stylization choices (the Kinesis death overlay's red is a
    /// separate `PostOverlay` effect, not this).
    pub tint: Vec3,
}

impl Vignette {
    /// Vignette disabled — `intensity = 0` makes every other field a
    /// no-op. App seeds the resource with this so unmodified scenes
    /// don't show a vignette they didn't ask for.
    pub const DEFAULT: Self = Self {
        intensity: 0.0,
        inner: 1.0,
        outer: 1.0,
        tint: Vec3::ZERO,
    };

    /// Subtle cinematic vignette — gentle corner darkening that
    /// reads as "this is a photographed image" without becoming
    /// a mood signal. The default look for calm scenes.
    pub const CINEMATIC: Self = Self {
        intensity: 0.4,
        inner: 0.5,
        outer: 1.0,
        tint: Vec3::ZERO,
    };

    /// Heavy, tight vignette for tension — corners crush to black,
    /// effective field of view narrows. Reads as threat or
    /// tunnel-vision.
    pub const OPPRESSIVE: Self = Self {
        intensity: 0.9,
        inner: 0.2,
        outer: 0.9,
        tint: Vec3::ZERO,
    };
}

impl Default for Vignette {
    fn default() -> Self {
        Self::DEFAULT
    }
}
