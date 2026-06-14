//! Per-scene primary color grade — ASC CDL (lift / gamma / gain).
//!
//! Inserted as a `World` resource by `App::resumed` with
//! [`ColorGrade::DEFAULT`] (an identity grade). The post pass reads
//! it once per frame, packs it into the post-params uniform buffer,
//! and the WGSL applies it post-tonemap. Gameplay code (or, later,
//! Chronicle rules) swaps the resource value to change the scene's
//! mood — chamber-white, cage-warm, service-red, labyrinth-cold.
//!
//! ## The formula
//!
//! For a per-channel tonemapped linear LDR color `c`:
//!
//! ```text
//! graded = (c * gain + lift) ^ (1 / gamma)
//! ```
//!
//! `gain` scales highlights, `lift` shifts shadows, `gamma` reshapes
//! the mids. The order is fixed (ASC CDL §1.1) so every grade
//! authored in the engine is portable to a film pipeline by
//! definition — useful long-term because a real colorist can take
//! the same numbers, paste them into DaVinci, and see the same look.
//!
//! ## Why post-tonemap
//!
//! ACES first, grade second. The artist's numbers map to what they
//! see on screen rather than to HDR scene-linear values that change
//! shape under tonemap. UE / Frostbite / most shipped game engines
//! do this; film pipelines often grade pre-tonemap because they want
//! absolute scene-linear control before the display transform.

use glam::Vec3;

/// Per-scene primary color grade. See module docs for the formula.
///
/// Identity grade is `lift = (0,0,0)`, `gamma = (1,1,1)`,
/// `gain = (1,1,1)` — accessible as [`ColorGrade::DEFAULT`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorGrade {
    /// Shadow shift, added after `gain`. Per-channel tint of the toe.
    pub lift: Vec3,
    /// Mid-tone reshape via `pow(x, 1 / gamma)`. Per-channel.
    /// `> 1` brightens that channel's mids; `< 1` darkens.
    /// Must stay strictly positive — zero or negative produces NaNs
    /// in the WGSL `pow`.
    pub gamma: Vec3,
    /// Highlight multiplier, applied before `lift`. Per-channel tint
    /// of the shoulder.
    pub gain: Vec3,
}

impl ColorGrade {
    /// Identity grade — no visible change. The default value the
    /// post params uniform is seeded with, so the scene looks
    /// unchanged until a Step or gameplay rule swaps the resource.
    pub const DEFAULT: Self = Self {
        lift: Vec3::ZERO,
        gamma: Vec3::ONE,
        gain: Vec3::ONE,
    };

    /// Clinical white-room feel — neutral midtones, faintly cool
    /// shadows and highlights. The chamber zone.
    ///
    /// Starting-point values; final calibration happens in 1.D.6
    /// against the playground binary with the cycle key.
    pub const CHAMBER_WHITE: Self = Self {
        lift: Vec3::new(0.0, 0.0, 0.01),
        gamma: Vec3::new(1.0, 1.0, 1.0),
        gain: Vec3::new(1.0, 1.0, 1.02),
    };

    /// Hostile clinical lab — an aggressive cold push: red pulled down,
    /// blue lifted across mids and highlights so white surfaces read as
    /// hard fluorescent blue-white rather than warm cream. Stronger than
    /// [`ColorGrade::CHAMBER_WHITE`]'s faint cool; pairs with cold,
    /// over-lit ambient. The "interrogation room" end of the chamber mood.
    pub const CLINICAL_COLD: Self = Self {
        lift: Vec3::new(0.0, 0.005, 0.02),
        gamma: Vec3::new(0.95, 1.0, 1.06),
        gain: Vec3::new(0.95, 1.0, 1.08),
    };

    /// Warm "comfortable" cage feel — slight amber push across
    /// shadows, mids, highlights. Reads as the comfort-side of the
    /// attitude axis.
    pub const CAGE_WARM: Self = Self {
        lift: Vec3::new(0.01, 0.005, 0.0),
        gamma: Vec3::new(1.05, 1.0, 0.95),
        gain: Vec3::new(1.05, 1.0, 0.95),
    };

    /// Red-corridor feel — strong red bias even on neutral
    /// surfaces. Reads as the service zone with its red point light.
    pub const SERVICE_RED: Self = Self {
        lift: Vec3::new(0.03, 0.0, 0.0),
        gamma: Vec3::new(1.1, 0.95, 0.9),
        gain: Vec3::new(1.1, 0.9, 0.85),
    };
}

impl Default for ColorGrade {
    fn default() -> Self {
        Self::DEFAULT
    }
}
