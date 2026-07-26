//! Engine-owned spectator camera for `dev-tools` builds.
//!
//! The spectator is a separate camera entity. Toggling it copies the active
//! gameplay camera pose, transfers [`ActiveCamera`], and leaves the physical
//! player untouched. Toggling back restores the prior camera.

use glam::{Quat, Vec3};

use crate::action::Actions;
use crate::camera::{ActiveCamera, Camera, FpsController};
use crate::debug::DebugPanels;
use crate::debug::egui::{self, Context};
use crate::ecs::{EntityId, Query, Res, World, WriteOnly, exclusive};
use crate::input::{Input, KeyCode};
use crate::plugin::Plugin;
use crate::symbol::{Symbol, sym};
use crate::time::Time;
use crate::transform::Transform;
use crate::{App, Stage};

const TOGGLE_SPECTATOR_ACTION: &str = "debug.camera.spectator";

/// Marker for the engine's debug-only free camera.
#[derive(Debug, Clone, Copy, Default)]
pub struct SpectatorCamera;

/// Runtime state for the spectator toggle.
#[derive(Debug)]
pub struct SpectatorDebugState {
    toggle: Symbol,
    pub active: bool,
    spectator: Option<EntityId>,
    previous: Option<EntityId>,
}

impl Default for SpectatorDebugState {
    fn default() -> Self {
        Self {
            toggle: sym(TOGGLE_SPECTATOR_ACTION),
            active: false,
            spectator: None,
            previous: None,
        }
    }
}

fn toggle_spectator(world: &mut World) {
    let should_toggle = world
        .resource::<SpectatorDebugState>()
        .zip(world.resource::<Actions>())
        .is_some_and(|(state, actions)| actions.just_pressed(state.toggle));
    if !should_toggle {
        return;
    }

    let Some(mut state) = world.remove_resource::<SpectatorDebugState>() else {
        return;
    };
    if state.active {
        deactivate_spectator(world, &mut state);
    } else {
        activate_spectator(world, &mut state);
    }
    world.insert_resource(state);
}

fn activate_spectator(world: &mut World, state: &mut SpectatorDebugState) {
    let source = world
        .iter::<ActiveCamera>()
        .map(|(entity, _)| entity)
        .find(|&entity| !world.contains::<SpectatorCamera>(entity));
    let Some(source) = source else {
        log::warn!("spectator camera: no gameplay ActiveCamera to take over from");
        return;
    };

    let (Some(transform), Some(camera), Some(controller)) = (
        world.get::<Transform>(source).copied(),
        world.get::<Camera>(source).copied(),
        world.get::<FpsController>(source).copied(),
    ) else {
        log::warn!("spectator camera: active camera lacks its controller components");
        return;
    };

    let spectator = state
        .spectator
        .filter(|&entity| world.is_alive(entity))
        .unwrap_or_else(|| {
            let entity = world.spawn();
            world.insert(entity, SpectatorCamera);
            entity
        });
    world.insert(spectator, transform);
    world.insert(spectator, camera);
    world.insert(spectator, controller);

    world.remove::<ActiveCamera>(source);
    world.insert(spectator, ActiveCamera);
    state.active = true;
    state.spectator = Some(spectator);
    state.previous = Some(source);
}

fn deactivate_spectator(world: &mut World, state: &mut SpectatorDebugState) {
    if let Some(spectator) = state.spectator.filter(|&entity| world.is_alive(entity)) {
        world.remove::<ActiveCamera>(spectator);
    }

    let previous = state
        .previous
        .filter(|&entity| world.is_alive(entity))
        .or_else(|| {
            world
                .iter::<Camera>()
                .map(|(entity, _)| entity)
                .find(|&entity| !world.contains::<SpectatorCamera>(entity))
        });
    if let Some(previous) = previous {
        world.insert(previous, ActiveCamera);
    } else {
        log::warn!("spectator camera: no gameplay camera available to restore");
    }

    state.active = false;
    state.previous = None;
}

/// `Update`-stage free-flight movement for the active spectator.
///
/// WASD moves on a yaw-only horizontal basis; Space and LeftCtrl move along
/// world Y. This system exists only in `dev-tools` builds.
pub fn spectator_move(
    input: Res<Input>,
    time: Res<Time>,
    cameras: Query<(
        WriteOnly<Transform>,
        (&FpsController, &ActiveCamera, &SpectatorCamera),
    )>,
) {
    if !input.cursor_grabbed() {
        return;
    }

    let forward = key_axis(&input, KeyCode::KeyW, KeyCode::KeyS);
    let right = key_axis(&input, KeyCode::KeyD, KeyCode::KeyA);
    let up = key_axis(&input, KeyCode::Space, KeyCode::ControlLeft);
    if forward == 0.0 && right == 0.0 && up == 0.0 {
        return;
    }

    for (mut transform, (controller, _, _)) in cameras {
        let velocity = spectator_velocity(controller.yaw, forward, right, up);
        transform.translation += velocity * controller.move_speed * time.delta_secs;
    }
}

fn spectator_velocity(yaw: f32, forward: f32, right: f32, up: f32) -> Vec3 {
    let yaw_rotation = Quat::from_axis_angle(Vec3::Y, yaw);
    let forward_axis = yaw_rotation * Vec3::NEG_Z;
    let right_axis = yaw_rotation * Vec3::X;
    let horizontal = (forward_axis * forward + right_axis * right).normalize_or_zero();
    horizontal + Vec3::Y * up
}

fn key_axis(input: &Input, positive: KeyCode, negative: KeyCode) -> f32 {
    input.is_key_down(positive) as i32 as f32 - input.is_key_down(negative) as i32 as f32
}

fn spectator_panel(world: &mut World, ctx: &Context) {
    let active = world
        .resource::<SpectatorDebugState>()
        .is_some_and(|state| state.active);
    egui::Window::new("Camera")
        .default_open(false)
        .show(ctx, |ui| {
            ui.label("F8 toggles the collision-free spectator camera");
            ui.monospace(if active {
                "Spectator: active"
            } else {
                "Spectator: inactive"
            });
            ui.small("WASD move, Space rises, Left Ctrl descends");
        });
}

/// Installs the engine's debug-only spectator camera.
#[derive(Debug, Default, Clone, Copy)]
pub struct CameraDebugPlugin;

impl Plugin for CameraDebugPlugin {
    fn build(&self, app: App) -> App {
        let mut app = app
            .insert_resource(SpectatorDebugState::default())
            .bind_key(TOGGLE_SPECTATOR_ACTION, KeyCode::F8)
            .add_system(Stage::Update, exclusive(toggle_spectator))
            .add_system(Stage::Update, spectator_move);
        if let Some(panels) = app.world_mut().resource_mut::<DebugPanels>() {
            panels.register("camera", spectator_panel);
        }
        app
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activation_transfers_active_camera_and_copies_pose() {
        let mut world = World::new();
        let gameplay = spawn_gameplay_camera(&mut world);
        let original = *world.get::<Transform>(gameplay).unwrap();
        let mut state = SpectatorDebugState::default();

        activate_spectator(&mut world, &mut state);

        let spectator = state.spectator.unwrap();
        assert!(state.active);
        assert!(!world.contains::<ActiveCamera>(gameplay));
        assert!(world.contains::<ActiveCamera>(spectator));
        assert_eq!(world.get::<Transform>(spectator), Some(&original));
    }

    #[test]
    fn deactivation_restores_gameplay_camera_without_moving_it() {
        let mut world = World::new();
        let gameplay = spawn_gameplay_camera(&mut world);
        let original = *world.get::<Transform>(gameplay).unwrap();
        let mut state = SpectatorDebugState::default();
        activate_spectator(&mut world, &mut state);

        let spectator = state.spectator.unwrap();
        world.get_mut::<Transform>(spectator).unwrap().translation = Vec3::splat(99.0);
        deactivate_spectator(&mut world, &mut state);

        assert!(!state.active);
        assert!(world.contains::<ActiveCamera>(gameplay));
        assert!(!world.contains::<ActiveCamera>(spectator));
        assert_eq!(world.get::<Transform>(gameplay), Some(&original));
    }

    #[test]
    fn f8_action_toggles_the_spectator_through_update() {
        use crate::action::{Bindings, Trigger, resolve_actions};
        use crate::ecs::Schedule;

        let mut world = World::new();
        spawn_gameplay_camera(&mut world);

        let mut input = Input::new();
        input.record_key(KeyCode::F8, true);
        let mut bindings = Bindings::default();
        bindings.bind(sym(TOGGLE_SPECTATOR_ACTION), Trigger::Key(KeyCode::F8));
        world.insert_resource(input);
        world.insert_resource(bindings);
        world.insert_resource(Actions::default());
        world.insert_resource(SpectatorDebugState::default());

        let mut schedule = Schedule::new();
        schedule.add_system(&mut world, Stage::Update, resolve_actions);
        schedule.add_system(&mut world, Stage::Update, exclusive(toggle_spectator));
        schedule.run(&mut world);

        assert!(world.resource::<SpectatorDebugState>().unwrap().active);
    }

    #[test]
    fn spectator_diagonal_preserves_horizontal_speed() {
        let velocity = spectator_velocity(0.0, 1.0, 1.0, 0.0);
        assert!((velocity.length() - 1.0).abs() < 1e-5);
    }

    #[test]
    fn spectator_vertical_input_is_world_aligned() {
        assert!(spectator_velocity(1.7, 0.0, 0.0, 1.0).abs_diff_eq(Vec3::Y, 1e-5));
        assert!(spectator_velocity(0.0, 0.0, 0.0, -1.0).abs_diff_eq(Vec3::NEG_Y, 1e-5));
    }

    fn spawn_gameplay_camera(world: &mut World) -> EntityId {
        let entity = world.spawn();
        world.insert(
            entity,
            Transform {
                translation: Vec3::new(1.0, 2.0, 3.0),
                rotation: Quat::from_rotation_y(0.5),
                scale: Vec3::ONE,
            },
        );
        world.insert(entity, Camera::perspective_default());
        world.insert(entity, FpsController::from_yaw_pitch(0.5, 0.0));
        world.insert(entity, ActiveCamera);
        entity
    }
}
