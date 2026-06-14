use game::scene::assets::Assets;
use game::scene::{self, PendingTransition, Player, SceneId};
use glam::{Quat, Vec3};
use schooner_engine::ecs::exclusive;
use schooner_engine::{
    ActiveCamera, App, AppError, Camera, FpsController, LogConfig, Stage, Transform, WindowConfig,
    World, fps_cursor_toggle, fps_look, fps_move, logging,
};

#[cfg(feature = "hot")]
mod hot {
    use game::scene::assets::Assets;
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
    pub fn reload_system(world: &mut World) {
        if PATCHED.swap(false, Ordering::SeqCst) {
            let id = world
                .resource::<ActiveScene>()
                .map(|a| a.0)
                .unwrap_or(SceneId::Playground);
            let assets = world.resource_mut::<Assets>().unwrap();
            assets.clean();
            scene::load_scene(world, id);
            log::info!("hot-reloaded {id:?}");
        }
    }
}

fn main() -> anyhow::Result<(), AppError> {
    logging::init(LogConfig::default()).unwrap();

    let mut app = App::new()
        .with_window_config(WindowConfig::new("Playground", 1280, 720))
        .insert_resource(Assets::default())
        .insert_resource(PendingTransition::default())
        .add_system(Stage::Update, fps_cursor_toggle)
        .add_system(Stage::Update, fps_look)
        .add_system(Stage::Update, fps_move)
        .add_system(Stage::Update, exclusive(scene::run_transition))
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
