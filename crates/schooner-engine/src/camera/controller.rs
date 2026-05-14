//! First-person camera controller.
//!
//! Architecture: see `architecture/camera.md` for the why
//! (projection / controller split, two-system polling shape, sign
//! conventions, cursor-grab policy).
//!
//! Three small systems compose the controller:
//! - [`fps_cursor_toggle`] — Esc flips cursor grab + visibility.
//! - `fps_look` (Phase G chunk 4) — mouse delta → yaw/pitch → rotation.
//! - `fps_move` (Phase G chunk 5) — WASD + Space/Ctrl → translation.
//!
//! All three are registered on the `Update` stage. `fps_cursor_toggle`
//! runs first so the same frame's look/move see the new grab state.

use std::f32::consts::FRAC_PI_2;

use glam::{Quat, Vec3};

use crate::camera::projection::ActiveCamera;
use crate::ecs::{Query, Res, ResMut, WriteOnly};
use crate::input::{Input, KeyCode};
use crate::time::Time;
use crate::transform::Transform;

/// Hard pitch clamp to keep yaw meaningful at the poles. At exactly
/// ±π/2 the camera's local right axis aligns with world Y and yaw
/// no longer rotates around the view direction; the epsilon is the
/// smallest float bump that visibly preserves yaw responsiveness.
const PITCH_LIMIT: f32 = FRAC_PI_2 - 1e-3;

/// First-person camera state.
///
/// Pose lives on the entity's `Transform`; this component is the
/// integrator state (yaw/pitch) plus the tunables (speed,
/// sensitivity). `fps_look` and `fps_move` read it together with
/// `Transform` on the same entity.
#[derive(Debug, Clone, Copy)]
pub struct FpsController {
    /// Yaw in radians around world `+Y`. Positive yaw rotates the
    /// camera's forward from `-Z` toward `-X` (turns the player to
    /// their left).
    pub yaw: f32,
    /// Pitch in radians around the camera's local right axis.
    /// `fps_look` clamps this to `(-π/2 + ε, π/2 - ε)` to avoid
    /// gimbal flip at the poles.
    pub pitch: f32,
    /// Horizontal movement speed in m/s. Vertical (Space/Ctrl) uses
    /// the same magnitude.
    pub move_speed: f32,
    /// Radians of yaw/pitch per pixel of mouse motion. The default
    /// is calibrated for a typical 1000 DPI mouse at default OS
    /// pointer speed; players will tune.
    pub mouse_sensitivity: f32,
}

impl Default for FpsController {
    fn default() -> Self {
        Self {
            yaw: 0.0,
            pitch: 0.0,
            move_speed: 5.0,
            mouse_sensitivity: 0.0025,
        }
    }
}

impl FpsController {
    /// Build a controller pre-aimed at the given yaw/pitch (radians)
    /// with default speed and sensitivity. Useful when the spawn
    /// pose was authored with a `look_at` matrix and the controller
    /// needs to start in lockstep — otherwise the first mouse-move
    /// would snap the orientation back to whatever yaw/pitch the
    /// component was constructed with.
    pub fn from_yaw_pitch(yaw: f32, pitch: f32) -> Self {
        Self {
            yaw,
            pitch,
            ..Self::default()
        }
    }
}

/// Compose `Transform.rotation` from yaw/pitch.
///
/// Yaw is around world `+Y`, pitch is around the camera's local
/// right axis. Quaternion multiplication is right-to-left for
/// vector rotation, so `q_yaw * q_pitch` applies pitch first
/// (around the original X axis) and yaw second (around world Y) —
/// the standard FPS "yaw around world up, pitch around local right"
/// behavior with no roll.
fn rotation_from_yaw_pitch(yaw: f32, pitch: f32) -> Quat {
    Quat::from_axis_angle(Vec3::Y, yaw) * Quat::from_axis_angle(Vec3::X, pitch)
}

/// `Update`-stage system: mouse delta → yaw/pitch → rotation.
///
/// Sign convention (Y-up RH, camera looks down `-Z`):
/// - Mouse-right (`dx > 0`) turns the player to the right, which
///   means yaw decreases (positive yaw rotates `-Z` toward `-X`,
///   i.e. *left*). So `yaw -= dx * sensitivity`.
/// - Mouse-down (winit's `dy > 0`) tilts the view down, which means
///   pitch decreases (positive pitch tilts up). So
///   `pitch -= dy * sensitivity`.
///
/// Pitch is clamped just inside `±π/2` so yaw stays meaningful at
/// extreme angles (see [`PITCH_LIMIT`]).
///
/// Early-returns when the cursor is not grabbed: hovering the mouse
/// over an unfocused window during dev would otherwise spin the
/// camera. This mirrors the convention every shipping FPS follows.
pub fn fps_look(
    input: Res<Input>,
    cameras: Query<(
        WriteOnly<Transform>,
        WriteOnly<FpsController>,
        &ActiveCamera,
    )>,
) {
    if !input.cursor_grabbed() {
        return;
    }

    let delta = input.mouse_delta();
    if delta == glam::Vec2::ZERO {
        return;
    }

    for (mut transform, mut controller, _) in cameras {
        controller.yaw -= delta.x * controller.mouse_sensitivity;
        controller.pitch = (controller.pitch - delta.y * controller.mouse_sensitivity)
            .clamp(-PITCH_LIMIT, PITCH_LIMIT);
        transform.rotation = rotation_from_yaw_pitch(controller.yaw, controller.pitch);
    }
}

/// Build the per-frame translation step from yaw + key state.
///
/// Horizontal motion (WASD) uses a yaw-only basis so that looking
/// up at the ceiling and pressing W does not lift the player off
/// the floor — the FPS-on-foot convention. Vertical motion
/// (Space/Ctrl) is a separate world-up axis added on top, the
/// noclip escape hatch for Game 0 where there is no ground
/// collision yet.
///
/// `normalize_or_zero` on the horizontal component prevents the
/// classic WASD bug where `forward + right` has length √2 and
/// diagonal motion is faster than cardinal motion. Vertical input
/// stays additive *after* the normalize so Space + W feels like
/// "fly up while moving forward," not "Space dilutes your speed."
fn move_velocity(yaw: f32, fwd: f32, right: f32, up: f32) -> Vec3 {
    let yaw_rot = Quat::from_axis_angle(Vec3::Y, yaw);
    let forward_h = yaw_rot * Vec3::NEG_Z;
    let right_h = yaw_rot * Vec3::X;
    let horizontal = (forward_h * fwd + right_h * right).normalize_or_zero();
    horizontal + Vec3::Y * up
}

/// `Update`-stage system: WASD + Space/Ctrl → translation.
///
/// Reads key state and the active camera's yaw, projects WASD
/// onto the horizontal plane through the camera's facing, adds
/// Space/Ctrl as world ±Y, scales by `move_speed * delta_secs`,
/// and writes into `Transform.translation`.
///
/// Bindings (hard-coded until input Layer 2 lands — see
/// `architecture/input.md`): W/A/S/D for forward/left/back/right,
/// Space for up, LeftCtrl for down.
///
/// Early-returns when the cursor is not grabbed: motion shouldn't
/// happen while the user is interacting with the desktop. Mirrors
/// `fps_look`'s gate.
pub fn fps_move(
    input: Res<Input>,
    time: Res<Time>,
    cameras: Query<(WriteOnly<Transform>, &FpsController, &ActiveCamera)>,
) {
    if !input.cursor_grabbed() {
        return;
    }

    // Axis sums: each direction contributes ±1, opposing keys
    // cancel exactly so a player holding W+S stays put.
    let fwd = key_axis(&input, KeyCode::KeyW, KeyCode::KeyS);
    let right = key_axis(&input, KeyCode::KeyD, KeyCode::KeyA);
    let up = key_axis(&input, KeyCode::Space, KeyCode::ControlLeft);

    if fwd == 0.0 && right == 0.0 && up == 0.0 {
        return;
    }

    let dt = time.delta_secs;
    for (mut transform, controller, _) in cameras {
        let v = move_velocity(controller.yaw, fwd, right, up);
        transform.translation += v * controller.move_speed * dt;
    }
}

fn key_axis(input: &Input, positive: KeyCode, negative: KeyCode) -> f32 {
    let mut axis = 0.0;
    if input.is_key_down(positive) {
        axis += 1.0;
    }
    if input.is_key_down(negative) {
        axis -= 1.0;
    }
    axis
}

/// `Update`-stage system: toggle cursor capture on `Esc just_pressed`.
///
/// Grabbed implies invisible; ungrabbed implies visible. Held as a
/// separate system (rather than folded into `fps_look`) so Phase H's
/// debug overlay can introduce its own Esc behavior visibly at the
/// schedule level instead of buried inside a controller body.
///
/// The flip is mirrored onto the live `Window` after the schedule
/// by `App::sync_cursor`.
pub fn fps_cursor_toggle(mut input: ResMut<Input>) {
    if !input.just_pressed(KeyCode::Escape) {
        return;
    }
    let grabbed = input.cursor_grabbed();
    input.set_cursor_grabbed(!grabbed);
    input.set_cursor_visible(grabbed);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_controller_starts_level() {
        let c = FpsController::default();
        assert_eq!(c.yaw, 0.0);
        assert_eq!(c.pitch, 0.0);
        assert!(c.move_speed > 0.0);
        assert!(c.mouse_sensitivity > 0.0);
    }

    #[test]
    fn from_yaw_pitch_keeps_default_tunables() {
        let c = FpsController::from_yaw_pitch(1.0, -0.5);
        let d = FpsController::default();
        assert_eq!(c.yaw, 1.0);
        assert_eq!(c.pitch, -0.5);
        assert_eq!(c.move_speed, d.move_speed);
        assert_eq!(c.mouse_sensitivity, d.mouse_sensitivity);
    }

    // The cursor-toggle system itself is exercised behind a
    // `ResMut<Input>` system param. Direct unit-testing the resource
    // wiring would re-test the schedule; the toggle's logic is a
    // straight read-flip-write that we verify by exercising
    // `Input::set_cursor_*` semantics here.

    #[test]
    fn esc_press_flips_grab_and_visibility() {
        // Simulate what the system does: read just_pressed(Escape),
        // flip the pair. Starts in the OS-default state (ungrabbed,
        // visible).
        let mut input = Input::new();
        input.record_key(KeyCode::Escape, true);
        assert!(input.just_pressed(KeyCode::Escape));
        assert!(!input.cursor_grabbed());
        assert!(input.cursor_visible());

        // Toggle.
        let grabbed = input.cursor_grabbed();
        input.set_cursor_grabbed(!grabbed);
        input.set_cursor_visible(grabbed);

        assert!(input.cursor_grabbed());
        assert!(!input.cursor_visible());
    }

    // -- rotation math ---------------------------------------------

    #[test]
    fn zero_yaw_and_pitch_is_identity() {
        // Identity orientation should leave the camera looking
        // straight down -Z, the canonical "facing forward" pose.
        let q = rotation_from_yaw_pitch(0.0, 0.0);
        assert!(q.abs_diff_eq(Quat::IDENTITY, 1e-6));
    }

    #[test]
    fn positive_yaw_rotates_forward_toward_negative_x() {
        // +90° yaw: camera forward goes from -Z to -X (turns left).
        let q = rotation_from_yaw_pitch(FRAC_PI_2, 0.0);
        let forward = q * Vec3::NEG_Z;
        assert!(
            forward.abs_diff_eq(Vec3::NEG_X, 1e-5),
            "forward was {forward:?}"
        );
    }

    #[test]
    fn positive_pitch_tilts_forward_up() {
        // +90° pitch: camera forward goes from -Z to +Y (looks up).
        let q = rotation_from_yaw_pitch(0.0, FRAC_PI_2);
        let forward = q * Vec3::NEG_Z;
        assert!(
            forward.abs_diff_eq(Vec3::Y, 1e-5),
            "forward was {forward:?}"
        );
    }

    #[test]
    fn yaw_then_pitch_yaw_is_around_world_up() {
        // Yaw 90° then pitch 45° must keep yaw around world Y, not
        // around the post-pitch local axis. Forward = (-X) rotated
        // 45° toward Y, lifted into the XY plane: (-cos45, sin45, 0).
        let q = rotation_from_yaw_pitch(FRAC_PI_2, std::f32::consts::FRAC_PI_4);
        let forward = q * Vec3::NEG_Z;
        let s = std::f32::consts::FRAC_1_SQRT_2;
        assert!(
            forward.abs_diff_eq(Vec3::new(-s, s, 0.0), 1e-5),
            "forward was {forward:?}"
        );
    }

    // -- movement basis --------------------------------------------

    #[test]
    fn no_input_yields_zero_velocity() {
        assert_eq!(move_velocity(0.0, 0.0, 0.0, 0.0), Vec3::ZERO);
        assert_eq!(move_velocity(1.23, 0.0, 0.0, 0.0), Vec3::ZERO);
    }

    #[test]
    fn forward_at_zero_yaw_moves_negative_z() {
        // At yaw=0 the camera looks down -Z, so W should walk -Z.
        let v = move_velocity(0.0, 1.0, 0.0, 0.0);
        assert!(v.abs_diff_eq(Vec3::NEG_Z, 1e-5), "velocity was {v:?}");
    }

    #[test]
    fn right_at_zero_yaw_moves_positive_x() {
        // At yaw=0, camera right (D) is +X.
        let v = move_velocity(0.0, 0.0, 1.0, 0.0);
        assert!(v.abs_diff_eq(Vec3::X, 1e-5), "velocity was {v:?}");
    }

    #[test]
    fn forward_at_quarter_yaw_moves_negative_x() {
        // After +90° yaw, forward (-Z) rotates to -X.
        let v = move_velocity(FRAC_PI_2, 1.0, 0.0, 0.0);
        assert!(v.abs_diff_eq(Vec3::NEG_X, 1e-5), "velocity was {v:?}");
    }

    #[test]
    fn diagonal_horizontal_motion_is_normalized() {
        // W + D held together: forward + right both 1. Without
        // normalization the magnitude would be √2; we want 1 so
        // diagonals don't outrun cardinals.
        let v = move_velocity(0.0, 1.0, 1.0, 0.0);
        assert!((v.length() - 1.0).abs() < 1e-5, "length was {}", v.length());
    }

    #[test]
    fn opposing_keys_cancel_to_zero() {
        // W+S, A+D should each cancel exactly — no jitter from a
        // tiny residual.
        assert_eq!(move_velocity(0.0, 0.0, 0.0, 0.0), Vec3::ZERO);
        // Simulating W+S (fwd=0) and A+D (right=0):
        let v = move_velocity(0.7, 0.0, 0.0, 0.0);
        assert_eq!(v, Vec3::ZERO);
    }

    #[test]
    fn vertical_input_stays_additive_after_horizontal_normalize() {
        // W + Space: horizontal contribution is -Z (length 1),
        // vertical adds +Y on top. Total length √2 — *not*
        // re-normalized, so flying forward-and-up doesn't slow
        // your forward speed.
        let v = move_velocity(0.0, 1.0, 0.0, 1.0);
        assert!(
            v.abs_diff_eq(Vec3::new(0.0, 1.0, -1.0), 1e-5),
            "velocity was {v:?}"
        );
    }

    #[test]
    fn vertical_only_input_yields_world_up() {
        // Space alone: pure +Y, regardless of yaw.
        let v = move_velocity(1.7, 0.0, 0.0, 1.0);
        assert!(v.abs_diff_eq(Vec3::Y, 1e-5), "velocity was {v:?}");
        let v = move_velocity(0.0, 0.0, 0.0, -1.0);
        assert!(v.abs_diff_eq(Vec3::NEG_Y, 1e-5), "velocity was {v:?}");
    }

    // -- key axis sums ---------------------------------------------

    #[test]
    fn key_axis_sums_to_zero_when_both_held() {
        let mut input = Input::new();
        input.record_key(KeyCode::KeyW, true);
        input.record_key(KeyCode::KeyS, true);
        assert_eq!(key_axis(&input, KeyCode::KeyW, KeyCode::KeyS), 0.0);
    }

    #[test]
    fn key_axis_returns_signed_axis() {
        let mut input = Input::new();
        input.record_key(KeyCode::KeyD, true);
        assert_eq!(key_axis(&input, KeyCode::KeyD, KeyCode::KeyA), 1.0);
        let mut input = Input::new();
        input.record_key(KeyCode::KeyA, true);
        assert_eq!(key_axis(&input, KeyCode::KeyD, KeyCode::KeyA), -1.0);
    }

    #[test]
    fn no_esc_press_leaves_cursor_state_alone() {
        let mut input = Input::new();
        input.set_cursor_grabbed(true);
        input.set_cursor_visible(false);
        // No Esc this frame.
        assert!(!input.just_pressed(KeyCode::Escape));
        // System would early-return; state stays put.
        assert!(input.cursor_grabbed());
        assert!(!input.cursor_visible());
    }
}
