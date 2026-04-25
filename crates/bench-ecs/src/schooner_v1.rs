//! `BenchEcs` implementation for the current schooner sparse-set ECS.
//!
//! Maps the [`BenchEcs`](crate::BenchEcs) operations directly onto
//! `World` / `Query` from `schooner-engine`. Component types are the
//! ones defined in [`crate`] — they're plain `T: 'static + Send +
//! Sync`, so the engine's blanket `Component` impl picks them up
//! automatically.
//!
//! The `iterate_*` methods go through the typed `Query` API, not
//! through any internal sparse-set shortcut — so the benchmarks
//! measure the *real* path system code uses.

use schooner_engine::ecs::{EntityId, World, Without, WriteOnly};

use crate::{BenchEcs, Bulk, Pos, Tag, Vel};

pub struct SchoonerV1;

impl BenchEcs for SchoonerV1 {
    type World = World;
    type Entity = EntityId;

    fn name() -> &'static str {
        "schooner-sparse-set-v1"
    }

    fn new_world() -> Self::World {
        World::new()
    }

    fn spawn(world: &mut Self::World) -> Self::Entity {
        world.spawn()
    }

    fn insert_pos(world: &mut Self::World, e: Self::Entity, p: Pos) {
        world.insert(e, p);
    }

    fn insert_vel(world: &mut Self::World, e: Self::Entity, v: Vel) {
        world.insert(e, v);
    }

    fn insert_tag(world: &mut Self::World, e: Self::Entity) {
        world.insert(e, Tag);
    }

    fn insert_bulk(world: &mut Self::World, e: Self::Entity, b: Bulk) {
        world.insert(e, b);
    }

    fn remove_pos(world: &mut Self::World, e: Self::Entity) {
        world.remove::<Pos>(e);
    }

    fn get_pos(world: &Self::World, e: Self::Entity) -> Option<Pos> {
        world.get::<Pos>(e).copied()
    }

    fn iterate_pos_vel(world: &mut Self::World, f: &mut dyn FnMut(&mut Pos, &Vel)) {
        // `world.query::<D>()` returns `QueryIter<'_, D, ()>` — the
        // raw iterator. The `Query<...>` wrapper is the SystemParam
        // surface only; for direct world iteration we consume the
        // iterator directly.
        for (mut p, v) in world.query::<(WriteOnly<Pos>, &Vel)>() {
            f(&mut *p, v);
        }
    }

    fn iterate_pos_without_tag(world: &mut Self::World, f: &mut dyn FnMut(&Pos)) {
        for p in world.query_filtered::<&Pos, Without<Tag>>() {
            f(p);
        }
    }

    fn iterate_pos_mut(world: &mut Self::World, f: &mut dyn FnMut(&mut Pos)) {
        for mut p in world.query::<WriteOnly<Pos>>() {
            f(&mut *p);
        }
    }
}
