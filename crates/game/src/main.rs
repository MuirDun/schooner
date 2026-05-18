use glam::{Quat, Vec3};
use schooner_engine::ecs::{Query, Res, WriteOnly};
use schooner_engine::logging::{self, LogConfig};
use schooner_engine::{
    ActiveCamera, App, Camera, DirectionalLight, FpsController, Material, MeshHandle, PointLight,
    Shadowcaster, SpotLight, Stage, Time, Transform, WindowConfig, World, fps_cursor_toggle,
    fps_look, fps_move,
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
        .add_system(Stage::Update, fps_move)
        .add_system(Stage::Update, orbit_spot);

    spawn_scene(app.world_mut());

    app.run()?;
    Ok(())
}

/// Phase 1.C.5 marker — the spot light this rides on orbits a
/// fixed `target` in the XZ plane at constant `height`, always
/// aiming back at the target. Position changes; the aim follows.
/// Shadows sweep across the grid because the *light source* moves,
/// the way a flashlight circling a room would. Removed when the
/// playground (Phase 1.H) replaces this scene.
#[derive(Debug, Clone, Copy)]
struct OrbitingSpot {
    /// World-space point the spot continuously aims at.
    target: Vec3,
    /// Orbit radius in the XZ plane around `target`.
    radius: f32,
    /// Y offset above `target` during the orbit.
    height: f32,
    /// Angular speed in radians per second.
    rate: f32,
}

/// `Update` system: drive every `OrbitingSpot` around its target.
///
/// Recomputes position and rotation from elapsed time each frame
/// rather than integrating deltas — keeps the orbit drift-free
/// and immune to frame-time jitter. Re-deriving the rotation
/// from the look-at vector each frame means the spot's beam is
/// always pointed at the target regardless of where the orbit is.
fn orbit_spot(time: Res<Time>, spots: Query<(WriteOnly<Transform>, &OrbitingSpot)>) {
    let elapsed = time.elapsed_secs as f32;
    for (mut transform, spot) in spots {
        let angle = elapsed * spot.rate;
        let position = spot.target
            + Vec3::new(
                spot.radius * angle.cos(),
                spot.height,
                spot.radius * angle.sin(),
            );
        let direction = (spot.target - position).normalize_or_zero();
        transform.translation = position;
        transform.rotation = Quat::from_rotation_arc(Vec3::NEG_Z, direction);
    }
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
    //
    // The front row (z = 1, closest to the camera) carries three
    // explicit `Material`s as the Phase 1.A.4 smoke test: warm
    // polished, neutral mid, and cool dim. The rear two rows have
    // no `Material` and fall back to `Material::DEFAULT` — a
    // side-by-side visual comparison that the fallback path also
    // still works.
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
            if z == 1 {
                let material = match x {
                    -1 => Material {
                        albedo: Vec3::new(1.0, 0.7, 0.5),
                        roughness: 0.15,
                        ..Material::DEFAULT
                    },
                    0 => Material {
                        albedo: Vec3::splat(0.7),
                        roughness: 0.5,
                        ..Material::DEFAULT
                    },
                    1 => Material {
                        albedo: Vec3::new(0.3, 0.4, 0.55),
                        roughness: 0.85,
                        ..Material::DEFAULT
                    },
                    _ => unreachable!(),
                };
                world.insert(cube, material);
            }
        }
    }

    // Sun. Default direction points down-and-forward; default
    // ambient gives the shadow-side surfaces a baseline so the
    // cubes don't read as flat black.
    //
    // Phase 1.C.5 diagnostic: intensity dialled way down so the
    // spot can dominate the floor. Without this, sun + ambient
    // saturate the white floor to display-white before any spot
    // contribution lands — and the per-spot shadow attenuation
    // becomes invisible. Restore to `DirectionalLight::default()`
    // (intensity 1.0) once Phase 1.D's tonemap compresses the
    // headroom properly.
    let sun = world.spawn();
    world.insert(
        sun,
        DirectionalLight {
            intensity: 0.08,
            ..DirectionalLight::default()
        },
    );

    // Phase 1.C.5 smoke test — a white spot that orbits the cube
    // grid, always aiming at its centre. As it travels, shadows
    // sweep across the floor in long arcs and cubes occlude each
    // other from the light at oblique angles. Initial transform
    // is overwritten every frame by `orbit_spot`, so the values
    // here are only visible on the first sub-frame.
    //
    // Debug controls:
    // - P cycles PCF kernel: Soft3x3 → Wide5x5 → Single → Soft3x3.
    // - F1 toggles the debug overlay.
    let spot = world.spawn();
    world.insert(
        spot,
        Transform {
            translation: Vec3::new(0.0, 5.0, 0.0),
            rotation: Quat::from_rotation_arc(Vec3::NEG_Z, Vec3::NEG_Y),
            scale: Vec3::ONE,
        },
    );
    // Range 10 covers the cube grid from any orbit position
    // (max spot-to-far-cube distance ~7.3 m at the opposite side
    // of the orbit). Outer cone 30° at distance ~4.2 m to grid
    // centre yields a ~2.4 m cone radius — narrower than the grid,
    // so the lit pool sweeps rather than illuminating everything
    // at once. Intensity 18 keeps the pool bright after the larger
    // range stretches the inverse-square falloff.
    world.insert(spot, SpotLight::new(Vec3::ONE, 50.0, 120.0, 25.0, 40.0));
    world.insert(spot, Shadowcaster);
    world.insert(
        spot,
        OrbitingSpot {
            target: Vec3::ZERO,
            // Radius 6 m clears the cube grid (cubes extend to
            // ±2 in x and z). Height 3 m gives a ~45° down-angle
            // from spot to target — shadows trail at a readable
            // length without smearing into infinity.
            radius: 6.0,
            height: 3.0,
            // ~28°/sec, full revolution in ~12 s. Slow enough to
            // track a single shadow's sweep without motion
            // sickness, fast enough to see the loop in one run.
            rate: 0.5,
        },
    );

    let red_lamp = world.spawn();
    world.insert(
        red_lamp,
        Transform::from_translation(Vec3::new(-4.0, 1.5, 2.0)),
    );
    world.insert(
        red_lamp,
        PointLight::new(Vec3::new(1.0, 0.1, 0.05), 5.0, 4.0),
    );

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
