use std::time::{SystemTime, UNIX_EPOCH};

use glam::{Quat, Vec3};
use schooner_engine::ecs::{Query, Res, WriteOnly, exclusive};
use schooner_engine::logging::{self, LogConfig};
use schooner_engine::{
    ActiveCamera, App, Camera, DirectionalLight, FpsController, Material, MeshHandle, MeshRegistry,
    PointLight, PostOverlay, RenderContext, Shadowcaster, SpotLight, Stage, TextureRegistry, Time,
    Transform, WindowConfig, World, fps_cursor_toggle, fps_look, fps_move,
};

// Asset paths, compile-time absolute via `CARGO_MANIFEST_DIR` so the
// binary loads them regardless of which directory `cargo run` is
// invoked from.
const RUSTY_TEXTURE_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/rusty.png");
const EYE_PRESSURE_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/eye-pressure.png");
const WALL_MESH_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/wall.glb");
const CUBE_MESH_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/cube.glb");
const MONKEY_MESH_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/monkey.glb");
const TORUS_MESH_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/torus.glb");

fn main() -> anyhow::Result<()> {
    logging::init(LogConfig::default())?;

    let mut app = App::new()
        .with_window_config(WindowConfig::new("Schooner", 1280, 720))
        // Order matters: cursor toggle runs first so the same frame's
        // look/move see the new grab state. render_frame is appended
        // last by App::resumed.
        .add_system(Stage::Update, fps_cursor_toggle)
        .add_system(Stage::Update, fps_look)
        .add_system(Stage::Update, fps_move)
        .add_system(Stage::Update, orbit_spot)
        // One-shot asset loader + scene populator. Exclusive because it
        // both reads device-backed resources (RenderContext, the
        // registries) and spawns entities; it runs in Update because
        // those resources only exist after `App::resumed`.
        .add_system(Stage::Update, exclusive(load_assets))
        .insert_resource(AssetsLoaded::default());

    spawn_scene(app.world_mut());

    app.run()?;
    Ok(())
}

/// Phase 1.C.5 marker — the spot light this rides on orbits a fixed
/// `target` in the XZ plane at constant `height`, always aiming back
/// at the target. Shadows sweep across the scene because the *light
/// source* moves, the way a flashlight circling a room would. Removed
/// when the playground (Phase 1.H) replaces this scene.
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

/// Latch resource for [`load_assets`]. Set to `true` after the first
/// attempt so the loader doesn't re-read disk or re-spawn the scene
/// every frame.
///
/// The "load once on first Update" pattern stands in for an engine
/// `StartupStage` we don't have yet — engine-side stage support
/// becomes a concern when Glyph systems need it later.
#[derive(Default, Debug)]
struct AssetsLoaded {
    done: bool,
}

/// Minimal xorshift64 — enough to scatter the smoke-test cubes without
/// pulling in the `rand` crate for a throwaway scene. Seeded from the
/// wall clock so the layout varies run to run.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        // Avoid the all-zero state, which xorshift gets stuck on.
        Self(seed | 1)
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    /// Uniform `f32` in `[lo, hi)`. Top 24 bits → mantissa precision,
    /// which is plenty for scattering a handful of cubes.
    fn range(&mut self, lo: f32, hi: f32) -> f32 {
        let unit = (self.next_u64() >> 40) as f32 / (1u32 << 24) as f32;
        lo + unit * (hi - lo)
    }
}

/// `Update` system: drive every `OrbitingSpot` around its target.
///
/// Recomputes position and rotation from elapsed time each frame
/// rather than integrating deltas — keeps the orbit drift-free and
/// immune to frame-time jitter. Re-deriving the rotation from the
/// look-at vector each frame keeps the beam pointed at the target
/// regardless of where the orbit is.
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

/// One-shot loader + scene populator (Step 1.F.5 smoke test).
///
/// Loads the authored glTF meshes and PNGs from disk through the v0
/// asset pipeline, points the post-overlay slot at `eye-pressure.png`,
/// and spawns the disk-dependent content. Self-disables via
/// [`AssetsLoaded`] so it costs one branch per frame afterward.
///
/// Exclusive (`&mut World`) so it can both pull device-backed
/// resources and spawn entities in one pass — a normal system can't
/// spawn. Each asset is independent: a failed load logs a warning and
/// that slice of the scene is skipped, so a single bad path doesn't
/// blank the whole room.
fn load_assets(world: &mut World) {
    if world.resource::<AssetsLoaded>().map(|s| s.done).unwrap_or(true) {
        return;
    }

    // Device + queue are refcounted; clone so the registry borrows
    // below don't overlap the RenderContext borrow. Type is inferred,
    // so the game crate never names a wgpu handle.
    let (device, queue) = {
        let Some(ctx) = world.resource::<RenderContext>() else {
            // RenderContext not up yet — leave the latch unset and
            // retry next frame.
            return;
        };
        (ctx.device().clone(), ctx.queue().clone())
    };

    // Meshes — one shared mutable borrow on the registry, loaded via a
    // FnMut closure so we never have to name wgpu's `Device` in a
    // helper signature (the game crate doesn't depend on wgpu).
    let (wall, cube, monkey, torus) = {
        let Some(reg) = world.resource_mut::<MeshRegistry>() else {
            return;
        };
        let mut load = |path: &str| match reg.load_gltf(&device, path) {
            Ok(h) => Some(h),
            Err(e) => {
                log::warn!("mesh {path} failed to load: {e}");
                None
            }
        };
        (
            load(WALL_MESH_PATH),
            load(CUBE_MESH_PATH),
            load(MONKEY_MESH_PATH),
            load(TORUS_MESH_PATH),
        )
    };

    // Textures — rusty for the cubes' albedo, eye-pressure for the
    // post-overlay slot.
    let (rusty, eye) = {
        let Some(reg) = world.resource_mut::<TextureRegistry>() else {
            return;
        };
        let mut load = |path: &str| match reg.load_png(&device, &queue, path) {
            Ok(h) => Some(h),
            Err(e) => {
                log::warn!("texture {path} failed to load: {e}");
                None
            }
        };
        (load(RUSTY_TEXTURE_PATH), load(EYE_PRESSURE_PATH))
    };

    // Hand eye-pressure to the overlay slot. It stays off (intensity 0)
    // until F6 cycles it; this just gives the cycle a real image.
    if let Some(eye) = eye {
        if let Some(overlay) = world.resource_mut::<PostOverlay>() {
            overlay.texture = Some(eye);
        }
    }

    // --- populate the scene -------------------------------------------
    // Transforms are first-guesses, tuned by eye against the live view:
    // the authored meshes' native size / origin aren't known here, so
    // expect to nudge these once you can see them.

    // Walls in the four corners of a ~12 m square.
    if let Some(wall) = wall {
        for (x, z) in [(-6.0, -6.0), (6.0, -6.0), (6.0, 6.0), (-6.0, 6.0)] {
            let e = world.spawn();
            world.insert(e, Transform::from_translation(Vec3::new(x, 0.0, z)));
            world.insert(e, wall);
        }
    }

    // Five rusty cubes scattered across the floor.
    if let Some(cube) = cube {
        let seed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x9E3779B97F4A7C15);
        let mut rng = Rng::new(seed);
        for _ in 0..5 {
            let e = world.spawn();
            world.insert(
                e,
                Transform::from_translation(Vec3::new(
                    rng.range(-7.0, 7.0),
                    1.0,
                    rng.range(-7.0, 7.0),
                )),
            );
            world.insert(e, cube);
            world.insert(
                e,
                Material {
                    albedo_texture: rusty,
                    ..Material::DEFAULT
                },
            );
        }
    }

    // Monkey at the centre, torus resting below it.
    if let Some(monkey) = monkey {
        let e = world.spawn();
        world.insert(e, Transform::from_translation(Vec3::new(0.0, 1.5, 0.0)));
        world.insert(e, monkey);
    }
    if let Some(torus) = torus {
        let e = world.spawn();
        world.insert(e, Transform::from_translation(Vec3::new(0.0, 0.5, 0.0)));
        world.insert(e, torus);
    }

    if let Some(state) = world.resource_mut::<AssetsLoaded>() {
        state.done = true;
    }
}

/// Scaffolding spawned at startup with built-in meshes and no disk
/// dependency: the floor, the lights, and the player camera. The
/// disk-loaded content (walls, cubes, monkey, torus) is spawned by
/// [`load_assets`] once the device is up.
fn spawn_scene(world: &mut World) {
    // Floor: scale the unit plane up so it covers the visible area.
    // The plane mesh is canonical; size is the entity's responsibility.
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

    // Dim sun fill so shadow-side surfaces don't read as flat black.
    let sun = world.spawn();
    world.insert(
        sun,
        DirectionalLight {
            intensity: 0.08,
            ..DirectionalLight::default()
        },
    );

    // White spot orbiting the centre, always aiming at it. As it
    // travels, shadows sweep across the monkey, torus, and cubes from
    // every angle in one run. Initial transform is overwritten every
    // frame by `orbit_spot`.
    let spot = world.spawn();
    world.insert(
        spot,
        Transform {
            translation: Vec3::new(0.0, 5.0, 0.0),
            rotation: Quat::from_rotation_arc(Vec3::NEG_Z, Vec3::NEG_Y),
            scale: Vec3::ONE,
        },
    );
    world.insert(spot, SpotLight::new(Vec3::ONE, 50.0, 120.0, 25.0, 40.0));
    world.insert(spot, Shadowcaster);
    world.insert(
        spot,
        OrbitingSpot {
            target: Vec3::ZERO,
            // Radius 6 m clears the centre cluster; height 3 m gives a
            // ~45° down-angle so shadows trail at a readable length.
            radius: 6.0,
            height: 3.0,
            // ~28°/sec, full revolution in ~12 s — slow enough to track
            // a single shadow's sweep, fast enough to see the loop.
            rate: 0.5,
        },
    );

    // Warm red point light off to one side.
    let red_lamp = world.spawn();
    world.insert(
        red_lamp,
        Transform::from_translation(Vec3::new(-4.0, 1.5, 2.0)),
    );
    world.insert(
        red_lamp,
        PointLight::new(Vec3::new(1.0, 0.1, 0.05), 5.0, 4.0),
    );

    // Player camera: standard FPS eye height (1.7 m), 8 m back from the
    // centre, facing -Z. FpsController starts at the same yaw/pitch so
    // the first mouse-move doesn't snap the orientation.
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
