use game::scene::assets::Assets;
use game::scene::{self, PendingTransition, SceneId};
use glam::{Quat, Vec3};
use schooner_engine::ecs::exclusive;
use schooner_engine::{
    ActiveCamera, App, AppError, Camera, FpsController, LogConfig, Stage, Transform, WindowConfig,
    World, fps_cursor_toggle, fps_look, fps_move, logging,
};

fn main() -> anyhow::Result<(), AppError> {
    logging::init(LogConfig::default()).unwrap();

    App::new()
        .with_window_config(WindowConfig::new("Playground", 1280, 720))
        .insert_resource(Assets::default())
        .insert_resource(PendingTransition::default())
        .add_system(Stage::Update, fps_cursor_toggle)
        .add_system(Stage::Update, fps_look)
        .add_system(Stage::Update, fps_move)
        .add_system(Stage::Update, exclusive(scene::run_transition))
        .add_system(Stage::Startup, exclusive(setup))
        .run()
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
}
