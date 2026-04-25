//! Frame timing and the fixed-timestep accumulator.
//!
//! [`Time`] is the resource systems read to know how much real time
//! the last frame took (`delta_secs`), how long the app has been
//! running (`elapsed_secs`), and the interpolation alpha for
//! rendering between fixed steps. The accumulator that drives
//! `FixedUpdate` step counts is a private field — systems do not see
//! it, the app loop does.
//!
//! ## Two stages, one [`Time`]
//!
//! - [`Stage::Update`](crate::ecs::Stage::Update) runs once per frame
//!   at real frame rate. Its `delta_secs` is the wall-clock time
//!   since the last frame.
//! - [`Stage::FixedUpdate`](crate::ecs::Stage::FixedUpdate) runs
//!   0..N times per frame at `fixed_delta` (1/60 by default).
//!   The accumulator banks unspent real time and pays it out in
//!   discrete `fixed_delta` chunks.
//!
//! ## Spiral-of-death cap
//!
//! When a frame takes longer than `MAX_FIXED_STEPS_PER_FRAME *
//! fixed_delta`, [`Time::advance`] caps the number of returned steps
//! and discards the remainder. Without this, a slow frame produces
//! more catch-up steps, which slow the next frame further, which
//! produces still more catch-up — physics-engine literature calls
//! this the spiral of death. We let physics fall behind under
//! sustained slowdown rather than freeze the simulation.
//!
//! No physics consumer in Game 0 — the cap is wired now so Game 1's
//! Rapier integration plugs into a stable contract.

use log::warn;

/// Maximum number of `FixedUpdate` steps allowed per frame.
///
/// Standard "Glenn Fiedler" choice; Bevy uses the same shape. With
/// `fixed_delta = 1/60`, this gives ~83 ms of fixed-step catch-up
/// per frame before time gets dropped on the floor.
pub const MAX_FIXED_STEPS_PER_FRAME: u32 = 5;

/// Default fixed-step rate in Hz. Matches the Game 0 plan.
pub const DEFAULT_FIXED_HZ: f32 = 60.0;

/// Frame-timing state, exposed to systems via [`Res<Time>`] /
/// [`ResMut<Time>`](crate::ecs::ResMut).
///
/// Public fields are read freely by systems; the accumulator is
/// crate-private so only the app loop can drive it.
#[derive(Debug)]
pub struct Time {
    /// Real seconds elapsed since the last `Update` call. The
    /// `Update` stage uses this as its dt.
    pub delta_secs: f32,

    /// Monotonic seconds since the first call to [`Time::advance`].
    /// `f64` because seconds-since-startup loses precision in `f32`
    /// after a few hours.
    pub elapsed_secs: f64,

    /// Length of one `FixedUpdate` step, in seconds.
    pub fixed_delta: f32,

    /// `[0.0, 1.0]` — fraction of a `fixed_delta` left in the
    /// accumulator after the last `advance`. The renderer uses this
    /// to interpolate between the previous and current fixed-step
    /// physics state. No consumer in Game 0; wired for Game 1+.
    pub interpolation_alpha: f32,

    accumulator: f32,
}

impl Time {
    /// Build a [`Time`] running `fixed_hz` fixed updates per second.
    ///
    /// `fixed_hz` must be finite and `> 0.0`. Out-of-range values
    /// produce a degenerate `fixed_delta` that would either freeze
    /// the sim or divide by zero in [`Self::advance`].
    pub fn new(fixed_hz: f32) -> Self {
        assert!(
            fixed_hz.is_finite() && fixed_hz > 0.0,
            "fixed_hz must be finite and positive (got {fixed_hz})"
        );
        Self {
            delta_secs: 0.0,
            elapsed_secs: 0.0,
            fixed_delta: 1.0 / fixed_hz,
            interpolation_alpha: 0.0,
            accumulator: 0.0,
        }
    }

    /// Advance the clock by `real_delta` real seconds and return the
    /// number of `FixedUpdate` steps the app should run this frame.
    ///
    /// Updates `delta_secs`, `elapsed_secs`, and
    /// `interpolation_alpha`. The `Update` stage runs exactly once
    /// per frame regardless of the returned count — only the
    /// fixed-step loop is accumulator-driven.
    ///
    /// Negative or non-finite `real_delta` is clamped to zero;
    /// monotonic clocks should never go backwards but the OS
    /// occasionally hands us a tiny negative on suspend/resume.
    pub fn advance(&mut self, real_delta: f32) -> u32 {
        let real_delta = if real_delta.is_finite() && real_delta > 0.0 {
            real_delta
        } else {
            0.0
        };

        self.delta_secs = real_delta;
        self.elapsed_secs += real_delta as f64;
        self.accumulator += real_delta;

        let mut steps: u32 = 0;
        while self.accumulator >= self.fixed_delta && steps < MAX_FIXED_STEPS_PER_FRAME {
            self.accumulator -= self.fixed_delta;
            steps += 1;
        }

        if self.accumulator >= self.fixed_delta {
            // Cap hit: drop the remaining banked time instead of
            // letting it pile up across frames.
            let dropped = self.accumulator;
            warn!(
                "fixed-step cap hit ({} steps); dropping {:.3}s of accumulated time \
                 to avoid spiral of death",
                MAX_FIXED_STEPS_PER_FRAME, dropped
            );
            self.accumulator = 0.0;
        }

        // Alpha is the fraction of a fixed step still banked, used
        // by the renderer to interpolate. Always in [0, 1) after
        // the loop above.
        self.interpolation_alpha = self.accumulator / self.fixed_delta;

        steps
    }
}

impl Default for Time {
    fn default() -> Self {
        Self::new(DEFAULT_FIXED_HZ)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }

    #[test]
    fn new_initialises_to_zero_clock_with_correct_fixed_delta() {
        let t = Time::new(60.0);
        assert_eq!(t.delta_secs, 0.0);
        assert_eq!(t.elapsed_secs, 0.0);
        assert!(approx(t.fixed_delta, 1.0 / 60.0));
        assert_eq!(t.interpolation_alpha, 0.0);
        assert_eq!(t.accumulator, 0.0);
    }

    #[test]
    fn default_is_60_hz() {
        let t = Time::default();
        assert!(approx(t.fixed_delta, 1.0 / 60.0));
    }

    #[test]
    #[should_panic(expected = "fixed_hz must be finite and positive")]
    fn zero_hz_panics() {
        let _ = Time::new(0.0);
    }

    #[test]
    #[should_panic(expected = "fixed_hz must be finite and positive")]
    fn negative_hz_panics() {
        let _ = Time::new(-60.0);
    }

    #[test]
    fn advance_zero_runs_no_fixed_steps() {
        let mut t = Time::new(60.0);
        assert_eq!(t.advance(0.0), 0);
        assert_eq!(t.delta_secs, 0.0);
        assert_eq!(t.elapsed_secs, 0.0);
        assert_eq!(t.interpolation_alpha, 0.0);
    }

    #[test]
    fn advance_below_fixed_delta_banks_into_accumulator_and_runs_zero_steps() {
        let mut t = Time::new(60.0);
        // 10ms < 16.67ms/step
        let steps = t.advance(0.010);
        assert_eq!(steps, 0);
        assert!(approx(t.delta_secs, 0.010));
        assert!(approx(t.accumulator, 0.010));
        // alpha = 0.010 / (1/60) ≈ 0.6
        assert!(t.interpolation_alpha > 0.5 && t.interpolation_alpha < 0.7);
    }

    #[test]
    fn advance_one_fixed_delta_runs_exactly_one_step() {
        let mut t = Time::new(60.0);
        let steps = t.advance(1.0 / 60.0);
        assert_eq!(steps, 1);
        // Accumulator should be ~0 after consuming one step.
        assert!(t.accumulator.abs() < 1e-4);
        assert!(t.interpolation_alpha < 1e-3);
    }

    #[test]
    fn advance_two_fixed_deltas_runs_two_steps() {
        let mut t = Time::new(60.0);
        let steps = t.advance(2.0 / 60.0);
        assert_eq!(steps, 2);
    }

    #[test]
    fn alpha_reflects_partial_step_remaining() {
        let mut t = Time::new(60.0);
        // 1.5 steps worth — runs 1 step, leaves 0.5 of a step banked.
        let steps = t.advance(1.5 / 60.0);
        assert_eq!(steps, 1);
        assert!((t.interpolation_alpha - 0.5).abs() < 1e-3);
    }

    #[test]
    fn elapsed_accumulates_across_calls() {
        let mut t = Time::new(60.0);
        t.advance(0.016);
        t.advance(0.016);
        t.advance(0.016);
        assert!((t.elapsed_secs - 0.048).abs() < 1e-5);
    }

    #[test]
    fn delta_reflects_only_the_last_call() {
        let mut t = Time::new(60.0);
        t.advance(0.100);
        t.advance(0.020);
        assert!(approx(t.delta_secs, 0.020));
    }

    #[test]
    fn advance_caps_at_max_fixed_steps_per_frame() {
        let mut t = Time::new(60.0);
        // 1 second at 60Hz would be 60 steps without a cap.
        let steps = t.advance(1.0);
        assert_eq!(steps, MAX_FIXED_STEPS_PER_FRAME);
        // Excess time is dropped, not banked into the next frame.
        assert_eq!(t.accumulator, 0.0);
        assert_eq!(t.interpolation_alpha, 0.0);
    }

    #[test]
    fn after_cap_next_frame_starts_clean() {
        let mut t = Time::new(60.0);
        // Hit the cap.
        t.advance(1.0);
        // Next frame is normal-paced.
        let steps = t.advance(1.0 / 60.0);
        assert_eq!(steps, 1);
    }

    #[test]
    fn negative_delta_is_clamped_to_zero() {
        let mut t = Time::new(60.0);
        let steps = t.advance(-0.5);
        assert_eq!(steps, 0);
        assert_eq!(t.delta_secs, 0.0);
        assert_eq!(t.elapsed_secs, 0.0);
        assert_eq!(t.accumulator, 0.0);
    }

    #[test]
    fn nan_delta_is_clamped_to_zero() {
        let mut t = Time::new(60.0);
        let steps = t.advance(f32::NAN);
        assert_eq!(steps, 0);
        assert_eq!(t.delta_secs, 0.0);
        // Accumulator must not become NaN.
        assert!(t.accumulator.is_finite());
    }

    #[test]
    fn infinite_delta_is_clamped_to_zero() {
        let mut t = Time::new(60.0);
        let steps = t.advance(f32::INFINITY);
        assert_eq!(steps, 0);
        assert!(t.elapsed_secs.is_finite());
    }

    #[test]
    fn small_repeated_deltas_eventually_run_a_step() {
        // 5ms × 4 = 20ms, which is one 16.67ms step plus a remainder.
        let mut t = Time::new(60.0);
        assert_eq!(t.advance(0.005), 0);
        assert_eq!(t.advance(0.005), 0);
        assert_eq!(t.advance(0.005), 0);
        assert_eq!(t.advance(0.005), 1);
    }

    #[test]
    fn alpha_stays_in_unit_interval_under_normal_load() {
        let mut t = Time::new(60.0);
        for _ in 0..1000 {
            t.advance(0.016);
            assert!((0.0..1.0).contains(&t.interpolation_alpha));
        }
    }
}
