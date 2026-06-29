use game::scene::assets::Assets;
use game::scene::playground::spawn_cube;
use game::scene::{self, PendingTransition, Player, SceneId};
use glam::{Quat, Vec3};
use log::logger;
use schooner_engine::ecs::{Commands, Events, Query, Res, ResMut, exclusive};
use schooner_engine::{
    ActiveCamera, App, AppError, Camera, EntityId, FpsController, Input, KeyCode, LogConfig,
    MouseButton, Stage, Transform, WindowConfig, World, fps_cursor_toggle, fps_look, fps_move,
    logging,
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

fn mode_select(input: Res<Input>, mut mode: ResMut<EditModeResource>) {
    if input.just_pressed(KeyCode::KeyE) {
        log::info!("Edit mode now is: ADD");
        mode.0 = EditMode::Add
    }
    if input.just_pressed(KeyCode::KeyQ) {
        log::info!("Edit mode now is: REMOVE");
        mode.0 = EditMode::Remove
    }
}

fn edit_input(
    input: Res<Input>,
    mode: Res<EditModeResource>,
    mut spawns: ResMut<Events<SpawnRequest>>,
    mut despawns: ResMut<Events<DespawnRequest>>,
) {
    if input.mouse_button_just_pressed(MouseButton::Left) {
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
    player: Query<(&Transform, &Player)>,
    mut commands: Commands,
) {
    if spawns.is_empty() {
        return;
    }

    let pose = player
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

struct SpawnRequest;
struct DespawnRequest;
enum EditMode {
    Add,
    Remove,
}
struct EditModeResource(EditMode);

#[derive(Default)]
struct CubeStack(Vec<EntityId>);

fn main() -> anyhow::Result<(), AppError> {
    logging::init(LogConfig::default()).unwrap();

    let mut app = App::new()
        .with_window_config(WindowConfig::new("Playground", 1280, 720))
        .add_event::<SpawnRequest>()
        .add_event::<DespawnRequest>()
        .insert_resource(Assets::default())
        .insert_resource(PendingTransition::default())
        .insert_resource(EditModeResource(EditMode::Add))
        .insert_resource(CubeStack::default())
        .add_system(Stage::Update, fps_cursor_toggle)
        .add_system(Stage::Update, fps_look)
        .add_system(Stage::Update, fps_move)
        .add_system(Stage::Update, exclusive(scene::run_transition))
        .add_system(Stage::Update, mode_select)
        .add_system(Stage::Update, edit_input)
        .add_system(Stage::Update, apply_spawns)
        .add_system(Stage::Update, apply_despawns)
        .add_system(Stage::Startup, exclusive(setup));

    // in main(), after building the App:
    #[cfg(feature = "hot")]
    {
        hot::install();
        app = app.add_system(Stage::Update, exclusive(hot::reload_system));
    }

    app.run()
}

fn setup(world: &mut World) {
    spawn_player(world);
    scene::load_scene(world, SceneId::Playground)
}

fn spawn_player(world: &mut World) {
    let camera = world.spawn();
    world.insert(
        camera,
        Transform {
            translation: Vec3::new(0.0, 1.7, 0.0),
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
        },
    );
    world.insert(camera, Camera::perspective_default());
    world.insert(camera, ActiveCamera);
    world.insert(camera, FpsController::default());
    world.insert(camera, Player);
}
