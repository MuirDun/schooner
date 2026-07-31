//! First-person camera controller.
//!
//! Architecture: see `architecture/camera.md` for the why
//! (projection / controller split, two-system polling shape, sign
//! conventions, cursor-grab policy).
//!
//! Two production systems compose the controller:
//! - [`fps_cursor_toggle`] — Esc flips cursor grab + visibility.
//! - `fps_look` (Phase G chunk 4) — mouse delta → yaw/pitch → rotation.
//!
//! Both are registered on the pre-fixed `Control` stage. Physical player
//! movement belongs to the hosted character controller. The old free-fly
//! translation behavior survives only as the `dev-tools` spectator camera.

use std::f32::consts::FRAC_PI_2;

use glam::{Quat, Vec3};

use crate::camera::projection::ActiveCamera;
use crate::ecs::{Query, Res, ResMut, WriteOnly};
use crate::input::{Input, KeyCode};
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
/// sensitivity). Camera systems read it together with `Transform` on the same
/// entity.
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
    /// Movement speed in m/s. Physical controllers and the debug spectator
    /// both use this as their default traversal speed.
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

/// `Control`-stage system: mouse delta → yaw/pitch → rotation.
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

/// `Control`-stage system: toggle cursor capture on `Esc just_pressed`.
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
    // Raw device motion continues while the cursor is free. Do not apply that
    // accumulated delta to the camera when this transition reclaims it.
    input.discard_mouse_delta();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Stage;
    use crate::camera::Camera;
    use crate::ecs::{Schedule, World};
    use crate::transform::Transform;

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

    #[test]
    fn cursor_reclaim_does_not_replay_free_cursor_mouse_motion() {
        let mut world = World::new();
        let mut input = Input::new();
        input.record_key(KeyCode::Escape, true);
        input.record_mouse_motion(80.0, -30.0);
        world.insert_resource(input);

        let camera = world.spawn();
        world.insert(camera, Transform::IDENTITY);
        world.insert(camera, Camera::perspective_default());
        world.insert(camera, FpsController::default());
        world.insert(camera, ActiveCamera);

        let mut schedule = Schedule::new();
        schedule.add_system(&mut world, Stage::Control, fps_cursor_toggle);
        schedule.add_system(&mut world, Stage::Control, fps_look);
        schedule.run_control(&mut world);

        assert!(world.resource::<Input>().unwrap().cursor_grabbed());
        assert_eq!(
            world.resource::<Input>().unwrap().mouse_delta(),
            glam::Vec2::ZERO
        );
        let controller = world.get::<FpsController>(camera).unwrap();
        assert_eq!(controller.yaw, 0.0);
        assert_eq!(controller.pitch, 0.0);
    }

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
