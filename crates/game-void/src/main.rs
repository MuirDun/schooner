use glam::{Mat4, Quat, Vec3};
use schooner_engine::ecs::{Query, Res, WriteOnly};
use schooner_engine::logging::{self, LogConfig};
use schooner_engine::{
    ActiveCamera, App, Camera, DirectionalLight, MeshHandle, Stage, Time, Transform, WindowConfig,
    World,
};

fn main() -> anyhow::Result<()> {
    logging::init(LogConfig::default())?;

    let mut app = App::new()
        .with_window_config(WindowConfig::new("Schooner — The Void", 1280, 720))
        .with_fps_logging()
        .with_input_logging()
        // Orbit the camera around the origin so every cube face is
        // viewed from every angle — this is the diagnostic that
        // would catch a winding bug, a normal-flip, or a depth-test
        // bug that a static camera could hide. Will be removed when
        // Phase G's FPS controller takes over.
        .add_system(Stage::Update, orbit_camera);

    spawn_scene(app.world_mut());

    app.run()?;
    Ok(())
}

/// Diagnostic system: orbit the active camera around the origin at
/// a fixed radius and height, looking inward, advancing one full
/// revolution every 8 seconds. Phase G replaces this with the FPS
/// controller; for now it lets us eyeball every face from every
/// angle during Phase F sign-off.
fn orbit_camera(
    time: Res<Time>,
    cameras: Query<(WriteOnly<Transform>, &ActiveCamera)>,
) {
    const RADIUS: f32 = 7.0;
    const HEIGHT: f32 = 3.0;
    const PERIOD_SECS: f32 = 8.0;

    let angle = (time.elapsed_secs as f32 / PERIOD_SECS) * std::f32::consts::TAU;
    let cam_pos = Vec3::new(angle.sin() * RADIUS, HEIGHT, angle.cos() * RADIUS);
    let view = Mat4::look_at_rh(cam_pos, Vec3::ZERO, Vec3::Y);
    let world_from_view = view.inverse();
    let (_, rotation, _) = world_from_view.to_scale_rotation_translation();

    for (mut transform, _) in cameras {
        transform.translation = cam_pos;
        transform.rotation = rotation;
    }
}

/// Game 0 scene: a wide floor, a small grid of cubes a few meters
/// in front of the camera, a sun-shaped directional light, and a
/// static camera looking at the cubes.
///
/// Phase F has no controller yet; the camera is fixed. Phase G
/// adds the FPS controller that turns this into a walkable scene.
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

    // Static camera at (0, 3, 6) looking at the origin. The
    // canonical view matrix from glam (look_at_rh) is the
    // world→camera transform; the camera's pose in the world is
    // its inverse, and we keep just the rotation since the
    // translation is already authored explicitly.
    let camera = world.spawn();
    let cam_pos = Vec3::new(0.0, 3.0, 6.0);
    let view = Mat4::look_at_rh(cam_pos, Vec3::ZERO, Vec3::Y);
    let world_from_view = view.inverse();
    let (_, rotation, _) = world_from_view.to_scale_rotation_translation();
    world.insert(
        camera,
        Transform {
            translation: cam_pos,
            rotation,
            scale: Vec3::ONE,
        },
    );
    world.insert(camera, Camera::perspective_default());
    world.insert(camera, ActiveCamera);
}
