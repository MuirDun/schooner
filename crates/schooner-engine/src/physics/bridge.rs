//! The physics ↔ ECS bridge — one exclusive FixedUpdate system.
//!
//! The membrane between Rapier's world and the ECS's world. It runs once
//! per fixed step and, when complete, performs the five-phase sequence
//! from `plans/architecture/physics.md`: reconcile body lifecycle, push
//! authored poses in, step the solver, write solved poses back to
//! `Transform`, and drain contacts/sensors into the Tier-2 event queues.
//!
//! It is registered **last** in FixedUpdate (appended in `App::resumed`,
//! mirroring `render_frame` in Render) so it always steps after the
//! gameplay systems that pushed forces into bodies this step — intent in,
//! then solve, then everyone reads the settled result.

use crate::ecs::{EntityId, World};
use crate::physics::{Collider, PhysicsWorld, RigidBody};
use crate::time::Time;
use crate::transform::Transform;

#[derive(Debug, Clone, Copy)]
struct BodySpawn {
    entity: EntityId,
    transform: Transform,
    body: RigidBody,
    collider: Collider,
}

/// Advance the physics simulation one fixed step and reconcile body
/// lifecycle with the ECS. Transform sync, pose write-back, and event
/// draining land in the following steps.
pub(crate) fn physics_bridge(world: &mut World) {
    // Read the fixed timestep first (Copy out, releasing the borrow) so
    // the solver always integrates by the slice the schedule paces
    // FixedUpdate at — robust to `with_fixed_hz` regardless of order.
    let Some(dt) = world.resource::<Time>().map(|t| t.fixed_delta) else {
        return;
    };
    let since = match world.resource::<PhysicsWorld>() {
        Some(physics) => physics.last_body_lifecycle_tick(),
        None => return,
    };
    let lifecycle_tick = world.current_tick();

    reconcile_body_lifecycle(world, since, lifecycle_tick);

    let Some(physics) = world.resource_mut::<PhysicsWorld>() else {
        return;
    };
    physics.integration_parameters.dt = dt;
    physics.step();
}

fn reconcile_body_lifecycle(world: &mut World, since: u64, lifecycle_tick: u64) {
    let spawns = body_spawns_since(world, since);
    let removals: Vec<EntityId> = world.removed::<RigidBody>().collect();

    let Some(physics) = world.resource_mut::<PhysicsWorld>() else {
        return;
    };

    for spawn in spawns {
        physics.materialize_body(spawn.entity, spawn.transform, spawn.body, spawn.collider);
    }

    for entity in removals {
        physics.remove_body(entity);
    }

    physics.set_last_body_lifecycle_tick(lifecycle_tick);
}

fn body_spawns_since(world: &mut World, since: u64) -> Vec<BodySpawn> {
    let mut out = Vec::new();
    let rigid_body_adds: Vec<EntityId> = world
        .added_since::<RigidBody>(since)
        .map(|(entity, _)| entity)
        .collect();
    let collider_adds: Vec<EntityId> = world
        .added_since::<Collider>(since)
        .map(|(entity, _)| entity)
        .collect();

    for entity in rigid_body_adds.into_iter().chain(collider_adds) {
        push_spawn_if_complete(world, &mut out, entity);
    }

    out
}

fn push_spawn_if_complete(world: &World, spawns: &mut Vec<BodySpawn>, entity: EntityId) {
    let (Some(transform), Some(body), Some(collider)) = (
        world.get::<Transform>(entity),
        world.get::<RigidBody>(entity),
        world.get::<Collider>(entity),
    ) else {
        return;
    };

    push_unique_spawn(
        spawns,
        BodySpawn {
            entity,
            transform: *transform,
            body: *body,
            collider: *collider,
        },
    );
}

fn push_unique_spawn(spawns: &mut Vec<BodySpawn>, spawn: BodySpawn) {
    if !spawns.iter().any(|existing| existing.entity == spawn.entity) {
        spawns.push(spawn);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

    #[test]
    fn bridge_materializes_entity_once_body_and_collider_exist() {
        let mut world = physics_world();
        let entity = world.spawn();
        world.increment_tick();
        world.insert(entity, Transform::from_translation(Vec3::new(0.0, 2.0, 0.0)));
        world.insert(entity, RigidBody::dynamic());
        world.insert(entity, Collider::ball(0.5));

        physics_bridge(&mut world);

        let physics = world.resource::<PhysicsWorld>().unwrap();
        assert!(physics.handles(entity).is_some());
        assert_eq!(physics.entity_count(), 1);
        assert_eq!(physics.last_body_lifecycle_tick(), world.current_tick());
    }

    #[test]
    fn bridge_waits_until_collider_exists() {
        let mut world = physics_world();
        let entity = world.spawn();
        world.increment_tick();
        world.insert(entity, Transform::from_translation(Vec3::new(0.0, 2.0, 0.0)));
        world.insert(entity, RigidBody::dynamic());

        physics_bridge(&mut world);
        assert_eq!(world.resource::<PhysicsWorld>().unwrap().entity_count(), 0);

        world.increment_tick();
        world.insert(entity, Collider::ball(0.5));
        physics_bridge(&mut world);

        let physics = world.resource::<PhysicsWorld>().unwrap();
        assert!(physics.handles(entity).is_some());
        assert_eq!(physics.entity_count(), 1);
    }

    #[test]
    fn bridge_removes_body_when_rigid_body_component_is_removed() {
        let mut world = physics_world();
        let entity = world.spawn();
        world.increment_tick();
        world.insert(entity, Transform::from_translation(Vec3::new(0.0, 2.0, 0.0)));
        world.insert(entity, RigidBody::dynamic());
        world.insert(entity, Collider::ball(0.5));
        physics_bridge(&mut world);
        assert_eq!(world.resource::<PhysicsWorld>().unwrap().entity_count(), 1);

        world.increment_tick();
        world.remove::<RigidBody>(entity);
        physics_bridge(&mut world);

        let physics = world.resource::<PhysicsWorld>().unwrap();
        assert!(physics.handles(entity).is_none());
        assert_eq!(physics.entity_count(), 0);
    }

    #[test]
    fn bridge_removes_body_when_entity_is_despawned() {
        let mut world = physics_world();
        let entity = world.spawn();
        world.increment_tick();
        world.insert(entity, Transform::from_translation(Vec3::new(0.0, 2.0, 0.0)));
        world.insert(entity, RigidBody::dynamic());
        world.insert(entity, Collider::ball(0.5));
        physics_bridge(&mut world);
        assert_eq!(world.resource::<PhysicsWorld>().unwrap().entity_count(), 1);

        world.increment_tick();
        world.despawn(entity);
        physics_bridge(&mut world);

        let physics = world.resource::<PhysicsWorld>().unwrap();
        assert!(physics.handles(entity).is_none());
        assert_eq!(physics.entity_count(), 0);
    }

    fn physics_world() -> World {
        let mut world = World::new();
        world.insert_resource(Time::default());
        world.insert_resource(PhysicsWorld::new());
        world
    }
}
