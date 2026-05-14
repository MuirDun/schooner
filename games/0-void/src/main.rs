use glam::{Quat, Vec3};
use schooner_engine::logging::{self, LogConfig};
use schooner_engine::{
    fps_cursor_toggle, fps_look, fps_move, ActiveCamera, App, Camera, DirectionalLight,
    FpsController, MeshHandle, Stage, Transform, WindowConfig, World,
};

fn main() -> anyhow::Result<()> {
    logging::init(LogConfig::default())?;

    let mut app = App::new()
        .with_window_config(WindowConfig::new("Schooner — The Void", 1280, 720))
        // Order matters: cursor toggle runs first so the same frame's
        // look/move see the new grab state. render_frame is appended
        // last by App::resumed.
        .add_system(Stage::Update, fps_cursor_toggle)
        .add_system(Stage::Update, fps_look)
        .add_system(Stage::Update, fps_move);

    spawn_scene(app.world_mut());

    app.run()?;
    Ok(())
}

/// Game 0 scene: a wide floor, a small grid of cubes around the
/// origin, a sun-shaped directional light, and a player-controlled
/// FPS camera spawned at standard eye height a few meters back.
fn spawn_scene(world: &mut World) {
    // Floor: scale the unit plane up so it covers the visible
    // area. The plane mesh is canonical; size is the entity's
    // responsibility.
    let floor = world.spawn();
    world.insert(
        floor,
        Transform {
            translation: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            scale: Vec3::new(20.0, 1.0, 20.0),
        },
    );
    world.insert(floor, MeshHandle::PLANE);

    // A 3×3 grid of cubes resting on the floor, spaced 2 units
    // apart. Each cube is the unit built-in (extent ±0.5), so the
    // center y is 0.5 + epsilon — the epsilon is what keeps the
    // cube's bottom face from being exactly coplanar with the
    // floor at y=0, which would z-fight under Depth32Float + Less.
    const CUBE_LIFT: f32 = 0.001;
    for x in -1..=1 {
        for z in -1..=1 {
            let cube = world.spawn();
            world.insert(
                cube,
                Transform::from_translation(Vec3::new(
                    x as f32 * 2.0,
                    0.5 + CUBE_LIFT,
                    z as f32 * 2.0,
                )),
            );
            world.insert(cube, MeshHandle::CUBE);
        }
    }

    // Sun. Default direction points down-and-forward; default
    // ambient gives the shadow-side surfaces a baseline so the
    // cubes don't read as flat black.
    let sun = world.spawn();
    world.insert(sun, DirectionalLight::default());

    // Player camera: standard FPS eye height (1.7 m), 8 m back from
    // the cube grid, facing -Z (yaw=0, pitch=0). FpsController
    // starts at the same yaw/pitch so the first mouse-move doesn't
    // snap the orientation.
    let camera = world.spawn();
    world.insert(
        camera,
        Transform {
            translation: Vec3::new(0.0, 1.7, 8.0),
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
        },
    );
    world.insert(camera, Camera::perspective_default());
    world.insert(camera, ActiveCamera);
    world.insert(camera, FpsController::default());
}
