#[cfg(feature = "dev-tools")]
use game::debug::KinesisRenderDebugPlugin;
use game::scene::assets::Assets;
use game::scene::playground::spawn_cube;
use game::scene::{self, PendingTransition, Player, SceneId};
use glam::{Quat, Vec3};
#[cfg(feature = "dev-tools")]
use schooner_engine::EngineDebugPlugins;
use schooner_engine::ecs::{Commands, Events, Query, Res, ResMut, WriteOnly, exclusive};
use schooner_engine::{
    Actions, ActiveCamera, App, AppError, Camera, CharacterController, CharacterControllerState,
    CharacterIntent, Collider, EntityId, FpsController, Input, KeyCode, LogConfig, MouseButton,
    RigidBody, Stage, Symbol, Transform, Trigger, WindowConfig, World, fps_cursor_toggle, fps_look,
    logging, sym,
};

#[cfg(feature = "hot")]
mod hot {
    use game::scene::{self, ActiveScene, SceneId};
    use schooner_engine::ecs::World;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    static PATCHED: AtomicBool = AtomicBool::new(false);

    /// Call once at startup. The handler runs on patch-apply; keep it
    /// trivial — just raise the flag. (Verify the exact subsecond hook.)
    pub fn install() {
        dioxus_devtools::connect_subsecond();
        subsecond::register_handler(Arc::new(|| PATCHED.store(true, Ordering::SeqCst)));
    }

    /// Update system: on a landed patch, rebuild the live scene in
    /// place. The player isn't a SceneEntity, so the camera stays put.
    ///
    /// No manual asset clearing: `load_scene` despawns the old
    /// `SceneEntity`s (dropping their handle clones) and re-runs the
    /// patched `build`. Assets shared across the rebuild stay resident via
    /// the `Assets` map; nothing reloads or leaks.
    pub fn reload_system(world: &mut World) {
        if PATCHED.swap(false, Ordering::SeqCst) {
            let id = world
                .resource::<ActiveScene>()
                .map(|a| a.0)
                .unwrap_or(SceneId::Playground);
            scene::load_scene(world, id);
            log::info!("hot-reloaded {id:?}");
        }
    }
}

fn mode_select(actions: Res<Actions>, acts: Res<Acts>, mut mode: ResMut<EditModeResource>) {
    if actions.just_pressed(acts.mode_spawn) {
        log::info!("Edit mode now is: ADD");
        mode.0 = EditMode::Add
    }
    if actions.just_pressed(acts.mode_despawn) {
        log::info!("Edit mode now is: REMOVE");
        mode.0 = EditMode::Remove
    }
}

fn edit_input(
    actions: Res<Actions>,
    acts: Res<Acts>,
    mode: Res<EditModeResource>,
    mut spawns: ResMut<Events<SpawnRequest>>,
    mut despawns: ResMut<Events<DespawnRequest>>,
) {
    if actions.just_pressed(acts.spawn) {
        match mode.0 {
            EditMode::Add => {
                spawns.send(SpawnRequest);
                log::info!("Send Spawn Event");
            }
            EditMode::Remove => {
                despawns.send(DespawnRequest);
                log::info!("Send Despawn Event");
            }
        }
    }
}

fn apply_spawns(
    mut spawns: ResMut<Events<SpawnRequest>>,
    camera: Query<(&Transform, &ActiveCamera)>,
    mut commands: Commands,
) {
    if spawns.is_empty() {
        return;
    }

    let pose = camera
        .into_iter()
        .next()
        .map(|(t, _)| (t.translation, t.rotation));

    for _ in spawns.drain() {
        log::info!("SPAWN Scheduled");
        let Some((pos, rot)) = pose else { continue };

        let forward = rot * Vec3::NEG_Z;
        let center = pos + forward * 2.0;

        commands.queue(move |world| {
            log::info!("SPAWNED A CUBE");
            let id = spawn_cube(world, center, Vec3::splat(1.0));
            world.resource_mut::<CubeStack>().unwrap().0.push(id);
        })
    }
}

fn apply_despawns(
    mut despawn: ResMut<Events<DespawnRequest>>,
    mut stack: ResMut<CubeStack>,
    mut commands: Commands,
) {
    for _ in despawn.drain() {
        if let Some(id) = stack.0.pop() {
            log::info!("DESPAWN Scheduled");
            commands.despawn(id);
        }
    }
}

/// 2.B.5 smoke: prove the action pipeline end to end. Reads the resolved
/// `Actions` (not raw `Input`) via the cached `Acts` symbols. Discrete
/// edges log once per press (no per-frame spam); the wheel logs while
/// scrolling. Throwaway — retired when the verbs/controller consume the
/// map in Parts 2.F–2.G.
fn action_smoke(actions: Res<Actions>, acts: Res<Acts>) {
    for (name, action) in [
        ("mode_hands", acts.mode_hands),
        ("mode_telekinesis", acts.mode_telekinesis),
        ("mode_repulsion", acts.mode_repulsion),
        ("push", acts.push),
        ("pull", acts.pull),
        ("throw", acts.throw),
    ] {
        if actions.just_pressed(action) {
            log::info!("ACTION {name} pressed");
        }
        if actions.just_released(action) {
            log::info!("ACTION {name} released");
        }
    }
    let wheel = actions.wheel();
    if wheel != 0.0 {
        log::info!("ACTION wheel = {wheel:.2}");
    }
}

struct SpawnRequest;
struct DespawnRequest;
enum EditMode {
    Add,
    Remove,
}
struct EditModeResource(EditMode);

/// Kinesis action names. Defined once here so the `.bind` calls and the
/// cached [`Acts`] symbols can't drift apart on a typo (a mismatch would
/// silently bind an action no system ever reads).
mod act {
    pub const MOVE_FORWARD: &str = "move_forward";
    pub const MOVE_BACK: &str = "move_back";
    pub const MOVE_LEFT: &str = "move_left";
    pub const MOVE_RIGHT: &str = "move_right";
    pub const JUMP: &str = "jump";
    pub const MODE_HANDS: &str = "mode_hands";
    pub const MODE_TELEKINESIS: &str = "mode_telekinesis";
    pub const MODE_REPULSION: &str = "mode_repulsion";
    pub const PUSH: &str = "push";
    pub const PULL: &str = "pull";
    pub const THROW: &str = "throw";
    pub const REPULSE: &str = "repulse";

    pub const SPAWN: &str = "spawn";
    pub const MODE_SPAWN: &str = "mode_spawn";
    pub const MODE_DESPAWN: &str = "mode_despawn";
}

/// Action symbols interned once at setup so per-frame systems read action
/// state without taking the interner lock. Most fields are consumed by
/// the verbs / controller in Parts 2.F–2.G; 2.B.5 reads `jump` to prove
/// the binding pipeline.
#[allow(dead_code)] // fields wired up as the verbs land (2.F / 2.G)
#[derive(Debug)]
struct Acts {
    move_forward: Symbol,
    move_back: Symbol,
    move_left: Symbol,
    move_right: Symbol,
    jump: Symbol,
    mode_hands: Symbol,
    mode_telekinesis: Symbol,
    mode_repulsion: Symbol,
    push: Symbol,
    pull: Symbol,
    throw: Symbol,
    repulse: Symbol,

    spawn: Symbol,
    mode_spawn: Symbol,
    mode_despawn: Symbol,
}

impl Acts {
    fn new() -> Self {
        Self {
            move_forward: sym(act::MOVE_FORWARD),
            move_back: sym(act::MOVE_BACK),
            move_left: sym(act::MOVE_LEFT),
            move_right: sym(act::MOVE_RIGHT),
            jump: sym(act::JUMP),
            mode_hands: sym(act::MODE_HANDS),
            mode_telekinesis: sym(act::MODE_TELEKINESIS),
            mode_repulsion: sym(act::MODE_REPULSION),
            push: sym(act::PUSH),
            pull: sym(act::PULL),
            throw: sym(act::THROW),
            repulse: sym(act::REPULSE),

            spawn: sym(act::SPAWN),
            mode_spawn: sym(act::MODE_SPAWN),
            mode_despawn: sym(act::MODE_DESPAWN),
        }
    }
}

#[derive(Default)]
struct CubeStack(Vec<EntityId>);

#[derive(Debug)]
struct PlayerBody(EntityId);

#[derive(Debug)]
struct PlayerView(EntityId);

#[derive(Debug)]
struct PlayerCamera;

const PLAYER_CAPSULE_RADIUS: f32 = 0.35;
const PLAYER_CAPSULE_HALF_HEIGHT: f32 = 0.55;
const PLAYER_BODY_CENTER_HEIGHT: f32 = PLAYER_CAPSULE_RADIUS + PLAYER_CAPSULE_HALF_HEIGHT;
const PLAYER_EYE_HEIGHT: f32 = 1.7;
const PLAYER_JUMP_SPEED: f32 = 5.0;

fn capture_player_movement(
    actions: Res<Actions>,
    acts: Res<Acts>,
    input: Res<Input>,
    active_player_camera: Query<(&FpsController, &ActiveCamera, &PlayerCamera)>,
    player_intent: Query<(WriteOnly<CharacterIntent>, &Player)>,
) {
    let Some((mut intent, _)) = player_intent.into_iter().next() else {
        return;
    };
    let Some((controller, _, _)) = active_player_camera.into_iter().next() else {
        intent.clear();
        return;
    };
    if !input.cursor_grabbed() {
        intent.clear();
        return;
    }

    let velocity = walk_velocity(
        controller.yaw,
        actions.axis(acts.move_back, acts.move_forward),
        actions.axis(acts.move_left, acts.move_right),
        controller.move_speed,
    );
    intent.set_horizontal_velocity(velocity);
    if actions.just_pressed(acts.jump) {
        intent.request_jump(PLAYER_JUMP_SPEED);
    }
}

fn sync_player_camera(world: &mut World) {
    let (Some(body), Some(camera)) = (
        world.resource::<PlayerBody>().map(|body| body.0),
        world.resource::<PlayerView>().map(|camera| camera.0),
    ) else {
        return;
    };
    let Some(body_translation) = world.get::<Transform>(body).map(|body| body.translation) else {
        return;
    };
    let Some(mut camera) = world.get_mut::<Transform>(camera) else {
        return;
    };
    camera.translation = player_eye_position(body_translation);
}

fn player_eye_position(body_translation: Vec3) -> Vec3 {
    body_translation + Vec3::Y * (PLAYER_EYE_HEIGHT - PLAYER_BODY_CENTER_HEIGHT)
}

fn walk_velocity(yaw: f32, forward_input: f32, right_input: f32, speed: f32) -> Vec3 {
    let yaw_rotation = Quat::from_axis_angle(Vec3::Y, yaw);
    let forward = yaw_rotation * Vec3::NEG_Z;
    let right = yaw_rotation * Vec3::X;
    (forward * forward_input + right * right_input).normalize_or_zero() * speed
}

fn main() -> anyhow::Result<(), AppError> {
    logging::init(LogConfig::default()).unwrap();

    let app = App::new()
        .with_window_config(WindowConfig::new("Playground", 1280, 720))
        .with_physics()
        .add_event::<SpawnRequest>()
        .add_event::<DespawnRequest>()
        .insert_resource(Assets::default())
        .insert_resource(PendingTransition::default())
        .insert_resource(EditModeResource(EditMode::Add))
        .insert_resource(CubeStack::default())
        // Layer 2 action bindings (Part 2.B.4). The verbs/controller
        // consume these in 2.F/2.G; 2.B.5 smoke-tests the pipeline.
        .insert_resource(Acts::new())
        .bind_key(act::MOVE_FORWARD, KeyCode::KeyW)
        .bind_key(act::MOVE_BACK, KeyCode::KeyS)
        .bind_key(act::MOVE_LEFT, KeyCode::KeyA)
        .bind_key(act::MOVE_RIGHT, KeyCode::KeyD)
        .bind_key(act::JUMP, KeyCode::Space)
        .bind_key(act::MODE_HANDS, KeyCode::Digit1)
        .bind_key(act::MODE_TELEKINESIS, KeyCode::Digit2)
        .bind_key(act::MODE_REPULSION, KeyCode::Digit3)
        .bind_key(act::MODE_SPAWN, KeyCode::KeyE)
        .bind_key(act::MODE_DESPAWN, KeyCode::KeyQ)
        .bind(act::PUSH, Trigger::Mouse(MouseButton::Left))
        .bind(act::SPAWN, Trigger::Mouse(MouseButton::Left))
        .bind(act::PULL, Trigger::Mouse(MouseButton::Right))
        .bind(act::THROW, Trigger::Mouse(MouseButton::Middle))
        // Mode 3 reuses Left; the active mode disambiguates push vs repulse.
        .bind(act::REPULSE, Trigger::Mouse(MouseButton::Left))
        .add_system(Stage::Control, fps_cursor_toggle)
        .add_system(Stage::Control, fps_look)
        .add_system(Stage::Control, mode_select)
        .add_system(Stage::Control, capture_player_movement)
        .add_system(Stage::PostPhysics, exclusive(sync_player_camera))
        .add_system(Stage::Update, exclusive(scene::run_transition))
        .add_system(Stage::Update, edit_input)
        .add_system(Stage::Update, apply_spawns)
        .add_system(Stage::Update, apply_despawns)
        .add_system(Stage::Update, action_smoke)
        .add_system(Stage::Startup, exclusive(setup));

    #[cfg(feature = "dev-tools")]
    let app = app
        .add_plugin(EngineDebugPlugins)
        .add_plugin(KinesisRenderDebugPlugin);

    // in main(), after building the App:
    #[cfg(feature = "hot")]
    let app = {
        hot::install();
        app.add_system(Stage::Update, exclusive(hot::reload_system))
    };

    app.run()
}

fn setup(world: &mut World) {
    spawn_player(world);
    scene::load_scene(world, SceneId::Playground)
}

fn spawn_player(world: &mut World) {
    let body = world.spawn();
    world.insert(
        body,
        Transform::from_translation(Vec3::new(0.0, PLAYER_BODY_CENTER_HEIGHT, 0.0)),
    );
    world.insert(body, RigidBody::kinematic_position_based());
    world.insert(
        body,
        Collider::capsule_y(PLAYER_CAPSULE_HALF_HEIGHT, PLAYER_CAPSULE_RADIUS),
    );
    world.insert(body, CharacterController::default());
    world.insert(body, CharacterControllerState::default());
    world.insert(body, CharacterIntent::default());
    world.insert(body, Player);
    world.insert_resource(PlayerBody(body));

    let camera = world.spawn();
    world.insert(
        camera,
        Transform {
            translation: Vec3::new(0.0, PLAYER_EYE_HEIGHT, 0.0),
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
        },
    );
    world.insert(camera, Camera::perspective_default());
    world.insert(camera, ActiveCamera);
    world.insert(camera, FpsController::default());
    world.insert(camera, PlayerCamera);
    world.insert_resource(PlayerView(camera));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagonal_walk_is_not_faster_than_cardinal_walk() {
        let cardinal = walk_velocity(0.0, 1.0, 0.0, 5.0);
        let diagonal = walk_velocity(0.0, 1.0, 1.0, 5.0);

        assert!((cardinal.length() - 5.0).abs() < 1e-6);
        assert!((diagonal.length() - 5.0).abs() < 1e-6);
    }

    #[test]
    fn walk_velocity_uses_camera_yaw() {
        let velocity = walk_velocity(std::f32::consts::FRAC_PI_2, 1.0, 0.0, 5.0);

        assert!(velocity.abs_diff_eq(Vec3::NEG_X * 5.0, 1e-5));
    }

    #[test]
    fn player_eye_position_preserves_authored_eye_height() {
        let body = Vec3::new(3.0, PLAYER_BODY_CENTER_HEIGHT, -2.0);

        assert_eq!(
            player_eye_position(body),
            Vec3::new(3.0, PLAYER_EYE_HEIGHT, -2.0)
        );
    }

    #[test]
    fn post_physics_copy_updates_translation_without_touching_look() {
        let mut world = World::new();
        let body = world.spawn();
        world.insert(
            body,
            Transform::from_translation(Vec3::new(2.0, PLAYER_BODY_CENTER_HEIGHT, -3.0)),
        );
        world.insert(body, Player);

        let camera = world.spawn();
        let rotation = Quat::from_rotation_y(0.75);
        world.insert(
            camera,
            Transform {
                rotation,
                ..Transform::IDENTITY
            },
        );
        world.insert(camera, PlayerCamera);
        world.insert_resource(PlayerBody(body));
        world.insert_resource(PlayerView(camera));

        let mut schedule = schooner_engine::Schedule::new();
        schedule.add_system(
            &mut world,
            Stage::PostPhysics,
            exclusive(sync_player_camera),
        );
        schedule.run_fixed(&mut world);

        let camera = world.get::<Transform>(camera).unwrap();
        assert_eq!(camera.translation, Vec3::new(2.0, PLAYER_EYE_HEIGHT, -3.0));
        assert_eq!(camera.rotation, rotation);
    }

    #[test]
    fn movement_capture_clears_intent_without_active_player_camera() {
        let mut world = World::new();
        let body = world.spawn();
        let mut intent = CharacterIntent::default();
        intent.set_horizontal_velocity(Vec3::new(3.0, 0.0, 4.0));
        intent.request_jump(PLAYER_JUMP_SPEED);
        world.insert(body, intent);
        world.insert(body, Player);
        world.insert_resource(Actions::default());
        world.insert_resource(Acts::new());
        world.insert_resource(Input::default());

        let spectator = world.spawn();
        world.insert(spectator, FpsController::default());
        world.insert(spectator, ActiveCamera);

        let mut schedule = schooner_engine::Schedule::new();
        schedule.add_system(&mut world, Stage::Control, capture_player_movement);
        schedule.run_control(&mut world);

        assert_eq!(
            world.get::<CharacterIntent>(body),
            Some(&CharacterIntent::default())
        );
    }
}
