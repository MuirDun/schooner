use schooner_engine::World;

pub mod assets;
pub mod playground;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SceneId {
    Playground,
}

pub struct SceneEntity;

pub struct Player;

pub struct ActiveScene(pub SceneId);

#[derive(Default)]
pub struct PendingTransition(pub Option<SceneId>);

/// Despawn the current scene, build a new one, record its identity.
/// Deliberately does NOT touch the player — positioning is the caller's
/// job, because a TRANSITION repositions but a dev-RELOAD must not.
pub fn load_scene(world: &mut World, id: SceneId) {
    assets::ensure(world, manifest(id));
    // Find all entities related to the active scene and unload them
    let owned: Vec<_> = world.iter::<SceneEntity>().map(|(e, _)| e).collect();
    for e in owned {
        world.despawn(e);
    }

    // Spawn new content
    build(world, id);

    // Record identity
    match world.resource_mut::<ActiveScene>() {
        Some(active) => active.0 = id,
        None => {
            world.insert_resource(ActiveScene(id));
        }
    };
}

/// Startup system: spawn the persistent player once, then load the
/// first scene. Registered as exclusive(setup) in Stage::Startup.
// pub fn setup(world: &mut World, id: SceneId) {
//     world.insert_resource(PendingTransitions::default());
//     spawn_player(world);
//     load_scene(world, id);
// }

pub fn run_transition(world: &mut World) {
    let Some(next) = world
        .resource_mut::<PendingTransition>()
        .and_then(|p| p.0.take())
    else {
        return;
    };

    load_scene(world, next);
    // reposition_player(world, entry_point());
}

fn build(world: &mut World, id: SceneId) {
    #[cfg(feature = "hot")]
    subsecond::call(|| build_inner(world, id));
    #[cfg(not(feature = "hot"))]
    build_inner(world, id);
}
fn build_inner(world: &mut World, id: SceneId) {
    match id {
        SceneId::Playground => playground::build(world),
    }
}

pub fn manifest(id: SceneId) -> assets::SceneAssets {
    #[cfg(feature = "hot")]
    return subsecond::call(|| manifest_inner(id));
    #[cfg(not(feature = "hot"))]
    return manifest_inner(id);
}

fn manifest_inner(id: SceneId) -> assets::SceneAssets {
    match id {
        SceneId::Playground => playground::MANIFEST,
    }
}
