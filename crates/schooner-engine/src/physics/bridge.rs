//! The physics ↔ ECS bridge — one exclusive FixedUpdate system.
//!
//! The membrane between Rapier's world and the ECS's world. It runs once
//! per fixed step and, when complete, performs the five-phase sequence
//! from `plans/architecture/physics.md`: reconcile body lifecycle, push
//! authored poses in, step the solver, write solved poses back to
//! `Transform`, and drain contacts/sensors into the Tier-2 event queues.
//!
//! The scheduler runs it between `FixedUpdate` intent writers and
//! `PostPhysics` outcome readers: intent in, solve, then settled results.

use crate::ecs::{EntityId, Events, World};
use crate::physics::{
    CharacterController, CharacterControllerState, CharacterIntent, CharacterMovement, Collider,
    Contact, ContactEvents, PhysicsCharacterWorkload, PhysicsCommand, PhysicsCommandWorkload,
    PhysicsCommands, PhysicsDiagnostics, PhysicsEventWorkload, PhysicsLifecycleWorkload,
    PhysicsSolveWorkload, PhysicsStepOutput, PhysicsTransformSyncWorkload, PhysicsWorld,
    PhysicsWritebackWorkload, RigidBody, TriggerEnter, TriggerExit,
};
use crate::time::Time;
use crate::transform::Transform;

#[derive(Debug, Clone, Copy)]
struct BodySpawn {
    entity: EntityId,
    transform: Transform,
    body: RigidBody,
    collider: Collider,
    contact_events: Option<ContactEvents>,
}

/// Advance the physics simulation one fixed step and reconcile it with
/// the ECS: lifecycle, authored-pose sync, solve, dynamic pose
/// write-back, and event publication.
pub(crate) fn physics_bridge(world: &mut World) {
    // Read the fixed timestep first (Copy out, releasing the borrow) so
    // the solver always integrates by the slice the schedule paces
    // FixedUpdate at — robust to `with_fixed_hz` regardless of order.
    let Some(dt) = world.resource::<Time>().map(|t| t.fixed_delta) else {
        return;
    };
    let transform_since = match world.resource::<PhysicsWorld>() {
        Some(physics) => physics.last_transform_sync_tick(),
        None => return,
    };
    let sync_tick = world.current_tick();
    physics_reconcile_lifecycle(world);
    let transform_sync = sync_changed_transforms(world, transform_since, sync_tick);
    let commands = apply_physics_commands(world);
    let characters = integrate_characters(world, dt);

    let Some(physics) = world.resource_mut::<PhysicsWorld>() else {
        return;
    };
    let body_count = physics.body_count();
    let collider_count = physics.collider_count();
    physics.integration_parameters.dt = dt;
    {
        puffin::profile_scope!("physics.rapier_solve");
        physics.step();
    }
    let output = physics.take_step_output();
    let active_dynamic_bodies = output.poses.len();

    let writeback = write_dynamic_poses(world, &output);
    let events = send_physics_events(world, &output);
    record_diagnostics(
        world,
        PhysicsDiagnostics {
            transform_sync,
            commands,
            characters,
            solve: PhysicsSolveWorkload {
                steps: 1,
                body_step_samples: count(body_count),
                collider_step_samples: count(collider_count),
                active_dynamic_body_steps: count(active_dynamic_bodies),
            },
            writeback,
            events,
            ..PhysicsDiagnostics::default()
        },
    );
    if let Some(physics) = world.resource_mut::<PhysicsWorld>() {
        physics.recycle_step_output(output);
    }
}

/// Reconcile authoring-component lifecycle without advancing the solver.
/// `App::tick` calls this at frame top so component removals are never lost
/// during zero-fixed-step frames; the bridge calls it again after fixed-step
/// commands have been applied.
pub(crate) fn physics_reconcile_lifecycle(world: &mut World) {
    puffin::profile_scope!("physics.lifecycle_reconciliation");

    let Some(since) = world
        .resource::<PhysicsWorld>()
        .map(PhysicsWorld::last_body_lifecycle_tick)
    else {
        return;
    };
    let workload = reconcile_body_lifecycle(world, since, world.current_tick());
    record_diagnostics(
        world,
        PhysicsDiagnostics {
            lifecycle: workload,
            ..PhysicsDiagnostics::default()
        },
    );
}

fn reconcile_body_lifecycle(
    world: &mut World,
    since: Option<u64>,
    lifecycle_tick: u64,
) -> PhysicsLifecycleWorkload {
    let (mut spawns, mut candidate_records) = body_spawns_since(world, since);
    let contact_event_removals: Vec<EntityId> = world.removed::<ContactEvents>().collect();
    candidate_records = candidate_records.saturating_add(contact_event_removals.len());
    for entity in contact_event_removals {
        push_spawn_if_complete(world, &mut spawns, entity);
    }
    let mut removals: Vec<EntityId> = world
        .removed::<RigidBody>()
        .chain(world.removed::<Collider>())
        .chain(world.removed::<Transform>())
        .collect();
    candidate_records = candidate_records.saturating_add(removals.len());
    removals.sort_unstable_by_key(|entity| (entity.index, entity.generation));
    removals.dedup();
    let incomplete_removals: Vec<EntityId> = removals
        .into_iter()
        .filter(|&entity| !body_is_complete(world, entity))
        .collect();
    let mut workload = PhysicsLifecycleWorkload {
        passes: 1,
        candidate_records: count(candidate_records),
        entities: count(spawns.len().saturating_add(incomplete_removals.len())),
        ..PhysicsLifecycleWorkload::default()
    };

    let Some(physics) = world.resource_mut::<PhysicsWorld>() else {
        return workload;
    };

    for entity in incomplete_removals {
        if physics.remove_body(entity).is_some() {
            workload.bodies_removed = workload.bodies_removed.saturating_add(1);
        }
    }

    for spawn in spawns {
        if physics.update_body_authoring(
            spawn.entity,
            spawn.body,
            spawn.collider,
            spawn.contact_events,
        ) {
            workload.bodies_updated = workload.bodies_updated.saturating_add(1);
        } else {
            physics.materialize_body_with_contact_events(
                spawn.entity,
                spawn.transform,
                spawn.body,
                spawn.collider,
                spawn.contact_events,
            );
            workload.bodies_materialized = workload.bodies_materialized.saturating_add(1);
        }
    }

    physics.set_last_body_lifecycle_tick(lifecycle_tick);
    workload
}

fn body_spawns_since(world: &mut World, since: Option<u64>) -> (Vec<BodySpawn>, usize) {
    let mut out = Vec::new();
    let rigid_body_changes: Vec<EntityId> = match since {
        Some(since) => world
            .added_since::<RigidBody>(since)
            .map(|(entity, _)| entity)
            .chain(
                world
                    .changed_since::<RigidBody>(since)
                    .map(|(entity, _)| entity),
            )
            .collect(),
        None => world
            .iter::<RigidBody>()
            .map(|(entity, _)| entity)
            .collect(),
    };
    let collider_changes: Vec<EntityId> = match since {
        Some(since) => world
            .added_since::<Collider>(since)
            .map(|(entity, _)| entity)
            .chain(
                world
                    .changed_since::<Collider>(since)
                    .map(|(entity, _)| entity),
            )
            .collect(),
        None => world.iter::<Collider>().map(|(entity, _)| entity).collect(),
    };
    let transform_adds: Vec<EntityId> = match since {
        Some(since) => world
            .added_since::<Transform>(since)
            .map(|(entity, _)| entity)
            .collect(),
        None => world
            .iter::<Transform>()
            .map(|(entity, _)| entity)
            .collect(),
    };
    let contact_event_changes: Vec<EntityId> = match since {
        Some(since) => world
            .added_since::<ContactEvents>(since)
            .map(|(entity, _)| entity)
            .chain(
                world
                    .changed_since::<ContactEvents>(since)
                    .map(|(entity, _)| entity),
            )
            .collect(),
        None => world
            .iter::<ContactEvents>()
            .map(|(entity, _)| entity)
            .collect(),
    };
    let candidate_records = rigid_body_changes
        .len()
        .saturating_add(collider_changes.len())
        .saturating_add(transform_adds.len())
        .saturating_add(contact_event_changes.len());

    for entity in rigid_body_changes
        .into_iter()
        .chain(collider_changes)
        .chain(transform_adds)
        .chain(contact_event_changes)
    {
        push_spawn_if_complete(world, &mut out, entity);
    }

    (out, candidate_records)
}

fn push_spawn_if_complete(world: &World, spawns: &mut Vec<BodySpawn>, entity: EntityId) {
    let (Some(transform), Some(body), Some(collider)) = (
        world.get::<Transform>(entity),
        world.get::<RigidBody>(entity),
        world.get::<Collider>(entity),
    ) else {
        return;
    };

    // TODO(performance): fix quadratic duplication
    push_unique_spawn(
        spawns,
        BodySpawn {
            entity,
            transform: *transform,
            body: *body,
            collider: *collider,
            contact_events: world.get::<ContactEvents>(entity).copied(),
        },
    );
}

fn body_is_complete(world: &World, entity: EntityId) -> bool {
    world.get::<Transform>(entity).is_some()
        && world.get::<RigidBody>(entity).is_some()
        && world.get::<Collider>(entity).is_some()
}

fn push_unique_spawn(spawns: &mut Vec<BodySpawn>, spawn: BodySpawn) {
    if !spawns
        .iter()
        .any(|existing| existing.entity == spawn.entity)
    {
        spawns.push(spawn);
    }
}

fn sync_changed_transforms(
    world: &mut World,
    since: Option<u64>,
    sync_tick: u64,
) -> PhysicsTransformSyncWorkload {
    puffin::profile_scope!("physics.authored_transform_sync");

    let changed: Vec<(EntityId, Transform)> = match since {
        Some(since) => world
            .changed_since::<Transform>(since)
            .map(|(entity, transform)| (entity, *transform))
            .collect(),
        None => world
            .iter::<Transform>()
            .map(|(entity, transform)| (entity, *transform))
            .collect(),
    };
    let mut workload = PhysicsTransformSyncWorkload {
        candidates: count(changed.len()),
        ..PhysicsTransformSyncWorkload::default()
    };

    let Some(physics) = world.resource_mut::<PhysicsWorld>() else {
        return workload;
    };

    for (entity, transform) in changed {
        if physics.set_authored_pose(entity, transform) {
            workload.applied = workload.applied.saturating_add(1);
        }
    }

    physics.set_last_transform_sync_tick(sync_tick);
    workload
}

fn apply_physics_commands(world: &mut World) -> PhysicsCommandWorkload {
    puffin::profile_scope!("physics.command_processing");

    let mut command_workload = PhysicsCommandWorkload::default();
    if !world.contains_resource::<PhysicsWorld>() {
        return command_workload;
    }
    let commands: Vec<PhysicsCommand> = match world.resource_mut::<PhysicsCommands>() {
        Some(commands) => commands.drain().collect(),
        None => return command_workload,
    };
    command_workload.total = count(commands.len());

    for command in commands {
        match command {
            PhysicsCommand::TeleportBody {
                entity,
                transform,
                velocity,
            } => {
                command_workload.teleports = command_workload.teleports.saturating_add(1);
                let applied = world
                    .resource_mut::<PhysicsWorld>()
                    .is_some_and(|physics| physics.teleport_body(entity, transform, velocity));
                if applied {
                    apply_teleport_pose(world, entity, transform);
                }
            }
        }
    }

    command_workload
}

fn integrate_characters(world: &mut World, dt: f32) -> PhysicsCharacterWorkload {
    puffin::profile_scope!("physics.character_integration");

    let mut workload = PhysicsCharacterWorkload::default();
    if !world.contains_resource::<PhysicsWorld>() {
        return workload;
    }

    let controllers: Vec<(EntityId, CharacterController)> = world
        .iter::<CharacterController>()
        .map(|(entity, controller)| (entity, *controller))
        .collect();
    workload.controllers = count(controllers.len());

    for (entity, controller) in controllers {
        let Some(mut state) = world.get::<CharacterControllerState>(entity).copied() else {
            continue;
        };
        let intent = world
            .get::<CharacterIntent>(entity)
            .copied()
            .unwrap_or_default();
        let jump_speed = intent.pending_jump_speed();
        if jump_speed.is_some() {
            workload.jump_requests = workload.jump_requests.saturating_add(1);
        }

        let mut jump_applied = false;
        if let Some(launch_speed) = jump_speed
            && state.grounded
        {
            state.grounded = false;
            state.vertical_velocity = launch_speed;
            jump_applied = true;
        }

        let movement = world.resource_mut::<PhysicsWorld>().and_then(|physics| {
            physics.move_character(entity, controller, state, intent.horizontal_velocity(), dt)
        });
        let Some(movement) = movement else {
            continue;
        };

        workload.integrations = workload.integrations.saturating_add(1);
        workload.kcc_queries = workload.kcc_queries.saturating_add(1);
        if jump_applied {
            workload.jumps_applied = workload.jumps_applied.saturating_add(1);
        }
        if jump_speed.is_some()
            && let Some(mut intent) = world.get_mut::<CharacterIntent>(entity)
        {
            intent.consume_jump();
        }
        apply_character_movement(world, movement);
    }

    workload
}

fn apply_teleport_pose(world: &mut World, entity: EntityId, transform: Transform) {
    let Some(mut current) = world.get_mut::<Transform>(entity) else {
        return;
    };
    current.translation = transform.translation;
    current.rotation = transform.rotation;
}

fn apply_character_movement(world: &mut World, movement: CharacterMovement) {
    if let Some(mut transform) = world.get_mut::<Transform>(movement.entity) {
        transform.translation = movement.translation;
    }
    if let Some(mut state) = world.get_mut::<CharacterControllerState>(movement.entity) {
        state.grounded = movement.grounded;
        state.vertical_velocity = movement.vertical_velocity;
    }
}

fn write_dynamic_poses(world: &mut World, output: &PhysicsStepOutput) -> PhysicsWritebackWorkload {
    puffin::profile_scope!("physics.dynamic_pose_writeback");

    let mut workload = PhysicsWritebackWorkload {
        pose_candidates: count(output.poses.len()),
        ..PhysicsWritebackWorkload::default()
    };
    for pose in &output.poses {
        let changed = world
            .get::<Transform>(pose.entity)
            .is_some_and(|transform| {
                transform.translation != pose.translation || transform.rotation != pose.rotation
            });
        if !changed {
            continue;
        }
        let Some(mut transform) = world.get_mut::<Transform>(pose.entity) else {
            continue;
        };
        transform.translation = pose.translation;
        transform.rotation = pose.rotation;
        workload.poses_written = workload.poses_written.saturating_add(1);
    }
    workload
}

fn send_physics_events(world: &mut World, output: &PhysicsStepOutput) -> PhysicsEventWorkload {
    puffin::profile_scope!("physics.event_publication");

    let mut workload = PhysicsEventWorkload::default();
    if let Some(events) = world.resource_mut::<Events<Contact>>() {
        for &contact in &output.contacts {
            events.send(contact);
        }
        workload.contacts_published = count(output.contacts.len());
    }

    if let Some(events) = world.resource_mut::<Events<TriggerEnter>>() {
        for &trigger_enter in &output.trigger_enters {
            events.send(trigger_enter);
        }
        workload.trigger_enters_published = count(output.trigger_enters.len());
    }

    if let Some(events) = world.resource_mut::<Events<TriggerExit>>() {
        for &trigger_exit in &output.trigger_exits {
            events.send(trigger_exit);
        }
        workload.trigger_exits_published = count(output.trigger_exits.len());
    }
    workload
}

fn record_diagnostics(world: &mut World, delta: PhysicsDiagnostics) {
    if let Some(diagnostics) = world.resource_mut::<PhysicsDiagnostics>() {
        diagnostics.merge(delta);
    }
}

fn count(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ecs::{Schedule, Stage, exclusive};
    use glam::Vec3;
    use rapier3d::na::vector;

    #[test]
    fn bridge_materializes_entity_once_body_and_collider_exist() {
        let mut world = physics_world();
        let entity = world.spawn();
        world.increment_tick();
        world.insert(
            entity,
            Transform::from_translation(Vec3::new(0.0, 2.0, 0.0)),
        );
        world.insert(entity, RigidBody::dynamic());
        world.insert(entity, Collider::ball(0.5));

        physics_bridge(&mut world);

        let physics = world.resource::<PhysicsWorld>().unwrap();
        assert!(physics.handles(entity).is_some());
        assert_eq!(physics.entity_count(), 1);
        assert_eq!(
            physics.last_body_lifecycle_tick(),
            Some(world.current_tick())
        );
    }

    #[test]
    fn bridge_first_reconciliation_materializes_tick_zero_authoring_once() {
        let mut world = physics_world();
        let entity = world.spawn();
        world.insert(
            entity,
            Transform::from_translation(Vec3::new(0.0, 2.0, 0.0)),
        );
        world.insert(entity, RigidBody::dynamic());
        world.insert(entity, Collider::ball(0.5));
        assert_eq!(world.current_tick(), 0);

        physics_bridge(&mut world);
        physics_bridge(&mut world);

        let physics = world.resource::<PhysicsWorld>().unwrap();
        assert!(physics.handles(entity).is_some());
        assert_eq!(physics.entity_count(), 1);
        assert_eq!(physics.last_body_lifecycle_tick(), Some(0));
    }

    #[test]
    fn bridge_records_workload_for_each_profiled_phase() {
        let mut world = physics_world();
        let entity = world.spawn();
        world.increment_tick();
        world.insert(
            entity,
            Transform::from_translation(Vec3::new(0.0, 2.0, 0.0)),
        );
        world.insert(entity, RigidBody::dynamic());
        world.insert(entity, Collider::ball(0.5));

        physics_bridge(&mut world);

        let diagnostics = world.resource::<PhysicsDiagnostics>().unwrap();
        assert_eq!(diagnostics.lifecycle.passes, 1);
        assert!(diagnostics.lifecycle.candidate_records >= 3);
        assert_eq!(diagnostics.lifecycle.entities, 1);
        assert_eq!(diagnostics.lifecycle.bodies_materialized, 1);
        assert_eq!(diagnostics.transform_sync.candidates, 1);
        assert_eq!(diagnostics.commands.total, 0);
        assert_eq!(diagnostics.solve.steps, 1);
        assert_eq!(diagnostics.solve.body_step_samples, 1);
        assert_eq!(diagnostics.solve.collider_step_samples, 1);
        assert_eq!(diagnostics.solve.active_dynamic_body_steps, 1);
        assert_eq!(diagnostics.writeback.pose_candidates, 1);
        assert_eq!(diagnostics.writeback.poses_written, 1);
        assert_eq!(diagnostics.events, PhysicsEventWorkload::default());
    }

    #[test]
    fn bridge_writes_dynamic_body_pose_back_to_transform() {
        let mut world = physics_world();
        let entity = world.spawn();
        world.increment_tick();
        world.insert(
            entity,
            Transform::from_translation(Vec3::new(0.0, 2.0, 0.0)),
        );
        world.insert(entity, RigidBody::dynamic());
        world.insert(entity, Collider::ball(0.5));

        physics_bridge(&mut world);

        let transform = world.get::<Transform>(entity).unwrap();
        assert!(transform.translation.y < 2.0);
    }

    #[test]
    fn bridge_applies_queued_dynamic_teleport_before_step() {
        let mut world = physics_world();
        let entity = world.spawn();
        world.increment_tick();
        world.insert(
            entity,
            Transform {
                translation: Vec3::new(0.0, 2.0, 0.0),
                scale: Vec3::splat(3.0),
                ..Transform::IDENTITY
            },
        );
        world.insert(entity, RigidBody::dynamic());
        world.insert(entity, Collider::ball(0.5));
        physics_bridge(&mut world);

        world.resource_mut::<PhysicsWorld>().unwrap().gravity = vector![0.0, 0.0, 0.0].into();
        world.increment_tick();
        world
            .resource_mut::<PhysicsCommands>()
            .unwrap()
            .teleport_body(
                entity,
                Transform::from_translation(Vec3::new(7.0, 8.0, 9.0)),
            );

        physics_bridge(&mut world);

        assert!(world.resource::<PhysicsCommands>().unwrap().is_empty());
        let transform = world.get::<Transform>(entity).unwrap();
        assert_eq!(transform.translation, Vec3::new(7.0, 8.0, 9.0));
        assert_eq!(transform.scale, Vec3::splat(3.0));
    }

    #[test]
    fn zero_input_character_integrates_gravity_once() {
        let mut world = physics_world();
        let character = spawn_character(&mut world, Vec3::new(0.0, 2.0, 0.0));

        physics_bridge(&mut world);

        let transform = world.get::<Transform>(character).unwrap();
        let state = world.get::<CharacterControllerState>(character).unwrap();
        assert!(transform.translation.y < 2.0);
        assert!(!state.grounded);
        let expected_velocity = -9.81 * world.resource::<Time>().unwrap().fixed_delta;
        assert!((state.vertical_velocity - expected_velocity).abs() < 1.0e-6);
        let diagnostics = world.resource::<PhysicsDiagnostics>().unwrap();
        assert_eq!(diagnostics.commands.total, 0);
        assert_eq!(diagnostics.characters.controllers, 1);
        assert_eq!(diagnostics.characters.integrations, 1);
        assert_eq!(diagnostics.characters.kcc_queries, 1);
    }

    #[test]
    fn repeated_intent_writes_do_not_multiply_character_integrations() {
        let mut world = physics_world();
        let character = spawn_character(&mut world, Vec3::new(0.0, 0.9, 0.0));
        world.resource_mut::<PhysicsWorld>().unwrap().gravity = vector![0.0, 0.0, 0.0].into();

        set_character_velocity(&mut world, character, Vec3::X);
        set_character_velocity(&mut world, character, Vec3::X * 5.0);
        set_character_velocity(&mut world, character, Vec3::X * 3.0);
        physics_bridge(&mut world);

        let dt = world.resource::<Time>().unwrap().fixed_delta;
        let x = world.get::<Transform>(character).unwrap().translation.x;
        assert!((x - 3.0 * dt).abs() < 1.0e-6);
        let diagnostics = world.resource::<PhysicsDiagnostics>().unwrap();
        assert_eq!(diagnostics.commands.total, 0);
        assert_eq!(diagnostics.characters.controllers, 1);
        assert_eq!(diagnostics.characters.integrations, 1);
        assert_eq!(diagnostics.characters.kcc_queries, 1);
    }

    #[test]
    fn character_workload_scales_with_controller_count() {
        let mut world = physics_world();
        let first = spawn_character(&mut world, Vec3::new(0.0, 0.9, -1.0));
        let second = spawn_character(&mut world, Vec3::new(0.0, 0.9, 1.0));
        world.resource_mut::<PhysicsWorld>().unwrap().gravity = vector![0.0, 0.0, 0.0].into();
        set_character_velocity(&mut world, first, Vec3::X);
        set_character_velocity(&mut world, second, Vec3::X * 2.0);

        physics_bridge(&mut world);

        let diagnostics = world.resource::<PhysicsDiagnostics>().unwrap();
        assert_eq!(diagnostics.characters.controllers, 2);
        assert_eq!(diagnostics.characters.integrations, 2);
        assert_eq!(diagnostics.characters.kcc_queries, 2);
    }

    #[test]
    fn event_publication_counts_each_typed_queue() {
        let mut world = physics_world();
        let a = EntityId {
            index: 1,
            generation: 0,
        };
        let b = EntityId {
            index: 2,
            generation: 0,
        };
        let output = PhysicsStepOutput {
            contacts: vec![Contact {
                a,
                b,
                impulse: 3.0,
                normal: Vec3::Y,
            }],
            trigger_enters: vec![TriggerEnter {
                sensor: a,
                other: b,
            }],
            trigger_exits: vec![TriggerExit {
                sensor: a,
                other: b,
            }],
            ..PhysicsStepOutput::default()
        };

        let workload = send_physics_events(&mut world, &output);

        assert_eq!(
            workload,
            PhysicsEventWorkload {
                contacts_published: 1,
                trigger_enters_published: 1,
                trigger_exits_published: 1,
            }
        );
        assert_eq!(world.resource::<Events<Contact>>().unwrap().len(), 1);
        assert_eq!(world.resource::<Events<TriggerEnter>>().unwrap().len(), 1);
        assert_eq!(world.resource::<Events<TriggerExit>>().unwrap().len(), 1);
    }

    #[test]
    fn character_move_excludes_self_and_stops_at_a_wall() {
        let mut world = physics_world();
        let character = spawn_character(&mut world, Vec3::new(0.0, 0.9, 0.0));
        spawn_static_box(
            &mut world,
            Vec3::new(1.1, 0.9, 0.0),
            Vec3::new(0.1, 2.0, 2.0),
        );
        physics_bridge(&mut world);
        world.resource_mut::<PhysicsWorld>().unwrap().gravity = vector![0.0, 0.0, 0.0].into();

        world.increment_tick();
        set_character_velocity(&mut world, character, Vec3::new(120.0, 0.0, 0.0));
        physics_bridge(&mut world);

        let x = world.get::<Transform>(character).unwrap().translation.x;
        assert!(x > 0.1, "self-collision prevented movement: x={x}");
        assert!(x < 0.8, "character crossed the wall: x={x}");
    }

    #[test]
    fn character_move_stops_at_a_dynamic_solid() {
        let mut world = physics_world();
        let character = spawn_character(&mut world, Vec3::new(0.0, 0.9, 0.0));
        spawn_dynamic_box(
            &mut world,
            Vec3::new(1.1, 0.9, 0.0),
            Vec3::new(0.1, 2.0, 2.0),
        );
        world.resource_mut::<PhysicsWorld>().unwrap().gravity = vector![0.0, 0.0, 0.0].into();
        physics_bridge(&mut world);

        world.increment_tick();
        set_character_velocity(&mut world, character, Vec3::new(120.0, 0.0, 0.0));
        physics_bridge(&mut world);

        let x = world.get::<Transform>(character).unwrap().translation.x;
        assert!(x < 0.8, "character crossed the dynamic solid: x={x}");
    }

    #[test]
    fn nested_sensors_do_not_change_resolved_character_movement() {
        let mut world = physics_world();
        let character = spawn_character(&mut world, Vec3::new(0.0, 0.9, 0.0));
        spawn_static_sensor(
            &mut world,
            Vec3::new(1.0, 0.9, 0.0),
            Vec3::new(0.4, 1.0, 1.0),
        );
        spawn_static_sensor(
            &mut world,
            Vec3::new(1.0, 0.9, 0.0),
            Vec3::new(0.2, 0.8, 0.8),
        );
        world.resource_mut::<PhysicsWorld>().unwrap().gravity = vector![0.0, 0.0, 0.0].into();
        physics_bridge(&mut world);

        world.increment_tick();
        set_character_velocity(&mut world, character, Vec3::new(120.0, 0.0, 0.0));
        physics_bridge(&mut world);

        let x = world.get::<Transform>(character).unwrap().translation.x;
        assert!(
            (x - 2.0).abs() < 1.0e-4,
            "nested sensors altered the requested movement: x={x}"
        );
    }

    #[test]
    fn character_traverses_corridor_sensor_and_reports_enter_and_exit() {
        let mut world = physics_world();
        let character = spawn_character(&mut world, Vec3::new(0.0, 0.9, 0.0));
        let sensor = spawn_static_sensor(
            &mut world,
            Vec3::new(1.25, 0.9, 0.0),
            Vec3::new(0.15, 1.0, 0.9),
        );
        spawn_static_box(
            &mut world,
            Vec3::new(1.75, 0.9, -1.0),
            Vec3::new(1.75, 1.0, 0.1),
        );
        spawn_static_box(
            &mut world,
            Vec3::new(1.75, 0.9, 1.0),
            Vec3::new(1.75, 1.0, 0.1),
        );
        world.resource_mut::<PhysicsWorld>().unwrap().gravity = vector![0.0, 0.0, 0.0].into();
        physics_bridge(&mut world);

        set_character_velocity(&mut world, character, Vec3::new(3.0, 0.0, 0.0));
        for _ in 0..70 {
            world.increment_tick();
            physics_bridge(&mut world);
        }

        let x = world.get::<Transform>(character).unwrap().translation.x;
        assert!(x > 3.4, "sensor blocked normal walking movement: x={x}");

        let enters: Vec<TriggerEnter> = world
            .resource::<Events<TriggerEnter>>()
            .unwrap()
            .iter()
            .copied()
            .collect();
        let exits: Vec<TriggerExit> = world
            .resource::<Events<TriggerExit>>()
            .unwrap()
            .iter()
            .copied()
            .collect();
        assert_eq!(enters.len(), 1);
        assert_eq!(enters[0].sensor, sensor);
        assert_eq!(enters[0].other, character);
        assert_eq!(exits.len(), 1);
        assert_eq!(exits[0].sensor, sensor);
        assert_eq!(exits[0].other, character);
    }

    #[test]
    fn character_move_reports_grounded_and_clears_falling_velocity() {
        let mut world = physics_world();
        let character = spawn_character(&mut world, Vec3::new(0.0, 0.9, 0.0));
        spawn_static_box(
            &mut world,
            Vec3::new(0.0, -0.1, 0.0),
            Vec3::new(5.0, 0.1, 5.0),
        );
        physics_bridge(&mut world);
        physics_bridge(&mut world);

        let state = world.get::<CharacterControllerState>(character).unwrap();
        assert!(state.grounded);
        assert_eq!(state.vertical_velocity, 0.0);
    }

    #[test]
    fn grounded_character_jump_launches_once_and_continues_under_gravity() {
        let mut world = physics_world();
        let character = spawn_character(&mut world, Vec3::new(0.0, 0.9, 0.0));
        spawn_static_box(
            &mut world,
            Vec3::new(0.0, -0.1, 0.0),
            Vec3::new(5.0, 0.1, 5.0),
        );
        physics_bridge(&mut world);
        physics_bridge(&mut world);
        assert!(
            world
                .get::<CharacterControllerState>(character)
                .unwrap()
                .grounded
        );
        let grounded_y = world.get::<Transform>(character).unwrap().translation.y;

        world.increment_tick();
        request_character_jump(&mut world, character, 5.0);
        physics_bridge(&mut world);

        let launched = *world.get::<CharacterControllerState>(character).unwrap();
        assert!(!launched.grounded);
        assert!(launched.vertical_velocity > 0.0);
        assert!(world.get::<Transform>(character).unwrap().translation.y > grounded_y);
        assert!(
            !world
                .get::<CharacterIntent>(character)
                .unwrap()
                .jump_requested()
        );

        world.increment_tick();
        physics_bridge(&mut world);

        let continued = world.get::<CharacterControllerState>(character).unwrap();
        assert!(continued.vertical_velocity < launched.vertical_velocity);
        assert!(continued.vertical_velocity > 0.0);
        let first_continuation_velocity = continued.vertical_velocity;

        world.increment_tick();
        physics_bridge(&mut world);

        let continued_again = world.get::<CharacterControllerState>(character).unwrap();
        assert!(continued_again.vertical_velocity < first_continuation_velocity);
        assert!(continued_again.vertical_velocity > 0.0);
    }

    #[test]
    fn jump_and_movement_writes_are_order_independent() {
        let mut jump_first = physics_world();
        let jump_first_character = spawn_character(&mut jump_first, Vec3::new(0.0, 0.9, 0.0));
        spawn_static_box(
            &mut jump_first,
            Vec3::new(0.0, -0.1, 0.0),
            Vec3::new(5.0, 0.1, 5.0),
        );
        physics_bridge(&mut jump_first);
        physics_bridge(&mut jump_first);

        let mut movement_first = physics_world();
        let movement_first_character =
            spawn_character(&mut movement_first, Vec3::new(0.0, 0.9, 0.0));
        spawn_static_box(
            &mut movement_first,
            Vec3::new(0.0, -0.1, 0.0),
            Vec3::new(5.0, 0.1, 5.0),
        );
        physics_bridge(&mut movement_first);
        physics_bridge(&mut movement_first);

        request_character_jump(&mut jump_first, jump_first_character, 5.0);
        set_character_velocity(
            &mut jump_first,
            jump_first_character,
            Vec3::new(3.0, 0.0, 0.0),
        );
        set_character_velocity(
            &mut movement_first,
            movement_first_character,
            Vec3::new(3.0, 0.0, 0.0),
        );
        request_character_jump(&mut movement_first, movement_first_character, 5.0);

        physics_bridge(&mut jump_first);
        physics_bridge(&mut movement_first);

        let jump_first_transform = jump_first.get::<Transform>(jump_first_character).unwrap();
        let movement_first_transform = movement_first
            .get::<Transform>(movement_first_character)
            .unwrap();
        assert!(
            jump_first_transform
                .translation
                .abs_diff_eq(movement_first_transform.translation, 1.0e-6)
        );
        assert_eq!(
            jump_first.get::<CharacterControllerState>(jump_first_character),
            movement_first.get::<CharacterControllerState>(movement_first_character)
        );
    }

    #[test]
    fn jump_latch_survives_a_frame_without_fixed_simulation() {
        let mut world = physics_world();
        let character = spawn_character(&mut world, Vec3::new(0.0, 0.9, 0.0));
        spawn_static_box(
            &mut world,
            Vec3::new(0.0, -0.1, 0.0),
            Vec3::new(5.0, 0.1, 5.0),
        );
        physics_bridge(&mut world);
        physics_bridge(&mut world);
        request_character_jump(&mut world, character, 5.0);

        let fixed_steps = world.resource_mut::<Time>().unwrap().advance(0.0);
        assert_eq!(fixed_steps, 0);
        physics_reconcile_lifecycle(&mut world);

        assert!(
            world
                .get::<CharacterIntent>(character)
                .unwrap()
                .jump_requested()
        );
        physics_bridge(&mut world);
        assert!(
            world
                .get::<CharacterControllerState>(character)
                .unwrap()
                .vertical_velocity
                > 0.0
        );
        assert!(
            !world
                .get::<CharacterIntent>(character)
                .unwrap()
                .jump_requested()
        );
    }

    #[test]
    fn controller_without_control_intent_continues_falling_and_lands() {
        let mut world = physics_world();
        let character = spawn_character(&mut world, Vec3::new(0.0, 2.0, 0.0));
        spawn_static_box(
            &mut world,
            Vec3::new(0.0, -0.1, 0.0),
            Vec3::new(5.0, 0.1, 5.0),
        );
        world.remove::<CharacterIntent>(character);

        for _ in 0..90 {
            world.increment_tick();
            physics_bridge(&mut world);
        }

        let transform = world.get::<Transform>(character).unwrap();
        let state = world.get::<CharacterControllerState>(character).unwrap();
        assert!(state.grounded);
        assert_eq!(state.vertical_velocity, 0.0);
        assert!(transform.translation.x.abs() < 1.0e-6);
        assert!((transform.translation.y - 0.9).abs() < 0.02);
        assert_eq!(
            world
                .resource::<PhysicsDiagnostics>()
                .unwrap()
                .characters
                .integrations,
            90
        );
    }

    #[test]
    fn airborne_character_rejects_jump_request() {
        let mut world = physics_world();
        let character = spawn_character(&mut world, Vec3::new(0.0, 2.0, 0.0));
        physics_bridge(&mut world);

        world.increment_tick();
        request_character_jump(&mut world, character, 5.0);
        physics_bridge(&mut world);

        let state = world.get::<CharacterControllerState>(character).unwrap();
        assert!(!state.grounded);
        assert!(state.vertical_velocity < 0.0);
        assert!(
            !world
                .get::<CharacterIntent>(character)
                .unwrap()
                .jump_requested()
        );
    }

    #[test]
    fn bridge_waits_until_collider_exists() {
        let mut world = physics_world();
        let entity = world.spawn();
        world.increment_tick();
        world.insert(
            entity,
            Transform::from_translation(Vec3::new(0.0, 2.0, 0.0)),
        );
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
        world.insert(
            entity,
            Transform::from_translation(Vec3::new(0.0, 2.0, 0.0)),
        );
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
    fn lifecycle_removes_body_when_only_collider_is_removed() {
        let mut world = populated_physics_world();
        let entity = only_physics_entity(&world);

        world.increment_tick();
        world.remove::<Collider>(entity);
        physics_reconcile_lifecycle(&mut world);

        assert!(
            world
                .resource::<PhysicsWorld>()
                .unwrap()
                .handles(entity)
                .is_none()
        );
    }

    #[test]
    fn frame_top_reconciliation_catches_a_removal_after_a_zero_step_frame() {
        let mut world = populated_physics_world();
        let entity = only_physics_entity(&world);

        world.increment_tick();
        world.remove::<Collider>(entity);
        world.swap_removed();
        physics_reconcile_lifecycle(&mut world);

        assert!(
            world
                .resource::<PhysicsWorld>()
                .unwrap()
                .handles(entity)
                .is_none()
        );
    }

    #[test]
    fn lifecycle_rebuilds_when_collider_changes() {
        let mut world = populated_physics_world();
        let entity = only_physics_entity(&world);
        let before = world
            .resource::<PhysicsWorld>()
            .unwrap()
            .handles(entity)
            .unwrap();

        world.increment_tick();
        world.insert(entity, Collider::ball(1.0));
        physics_reconcile_lifecycle(&mut world);

        let after = world
            .resource::<PhysicsWorld>()
            .unwrap()
            .handles(entity)
            .unwrap();
        assert_ne!(before, after);
    }

    #[test]
    fn lifecycle_updates_compatible_collider_changes_in_place() {
        let mut world = populated_physics_world();
        let entity = only_physics_entity(&world);
        let before = world
            .resource::<PhysicsWorld>()
            .unwrap()
            .handles(entity)
            .unwrap();

        world.increment_tick();
        world.insert(
            entity,
            Collider::ball(0.5).material(crate::physics::PhysicsMaterial {
                friction: 0.8,
                restitution: 0.2,
            }),
        );
        physics_reconcile_lifecycle(&mut world);

        let after = world
            .resource::<PhysicsWorld>()
            .unwrap()
            .handles(entity)
            .unwrap();
        assert_eq!(before, after);
    }

    #[test]
    fn lifecycle_materializes_when_transform_is_added_last() {
        let mut world = physics_world();
        let entity = world.spawn();
        world.increment_tick();
        world.insert(entity, RigidBody::dynamic());
        world.insert(entity, Collider::ball(0.5));
        physics_reconcile_lifecycle(&mut world);
        assert!(
            world
                .resource::<PhysicsWorld>()
                .unwrap()
                .handles(entity)
                .is_none()
        );

        world.increment_tick();
        world.insert(entity, Transform::IDENTITY);
        physics_reconcile_lifecycle(&mut world);
        assert!(
            world
                .resource::<PhysicsWorld>()
                .unwrap()
                .handles(entity)
                .is_some()
        );
    }

    #[test]
    fn lifecycle_removal_then_reinsertion_keeps_the_rebuilt_body() {
        let mut world = populated_physics_world();
        let entity = only_physics_entity(&world);

        world.increment_tick();
        world.remove::<RigidBody>(entity);
        world.insert(entity, RigidBody::kinematic_position_based());
        physics_reconcile_lifecycle(&mut world);

        assert!(
            world
                .resource::<PhysicsWorld>()
                .unwrap()
                .handles(entity)
                .is_some()
        );
        assert_eq!(world.resource::<PhysicsWorld>().unwrap().entity_count(), 1);

        world.swap_removed();
        physics_reconcile_lifecycle(&mut world);

        assert!(
            world
                .resource::<PhysicsWorld>()
                .unwrap()
                .handles(entity)
                .is_some()
        );
        assert_eq!(world.resource::<PhysicsWorld>().unwrap().entity_count(), 1);
    }

    #[test]
    fn startup_stage_physics_components_materialize_after_startup_tick() {
        let mut world = physics_world();
        let mut schedule = Schedule::new();
        schedule.add_system(
            &mut world,
            Stage::Startup,
            exclusive(|world: &mut World| {
                let entity = world.spawn();
                world.insert(entity, Transform::from_translation(Vec3::Y));
                world.insert(entity, RigidBody::dynamic());
                world.insert(entity, Collider::ball(0.5));
            }),
        );

        schedule.run_startup(&mut world);
        physics_reconcile_lifecycle(&mut world);

        assert_eq!(world.resource::<PhysicsWorld>().unwrap().entity_count(), 1);
    }

    #[test]
    fn bridge_removes_body_when_entity_is_despawned() {
        let mut world = physics_world();
        let entity = world.spawn();
        world.increment_tick();
        world.insert(
            entity,
            Transform::from_translation(Vec3::new(0.0, 2.0, 0.0)),
        );
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
        world.insert_resource(PhysicsCommands::new());
        world.insert_resource(PhysicsDiagnostics::default());
        world.insert_resource(Events::<Contact>::default());
        world.insert_resource(Events::<TriggerEnter>::default());
        world.insert_resource(Events::<TriggerExit>::default());
        world
    }

    fn populated_physics_world() -> World {
        let mut world = physics_world();
        let entity = world.spawn();
        world.increment_tick();
        world.insert(entity, Transform::from_translation(Vec3::Y));
        world.insert(entity, RigidBody::dynamic());
        world.insert(entity, Collider::ball(0.5));
        physics_reconcile_lifecycle(&mut world);
        world
    }

    fn spawn_character(world: &mut World, translation: Vec3) -> EntityId {
        let entity = world.spawn();
        world.increment_tick();
        world.insert(entity, Transform::from_translation(translation));
        world.insert(entity, RigidBody::kinematic_position_based());
        world.insert(entity, Collider::capsule_y(0.55, 0.35));
        world.insert(entity, CharacterController::default());
        world.insert(entity, CharacterControllerState::default());
        world.insert(entity, CharacterIntent::default());
        entity
    }

    fn set_character_velocity(world: &mut World, entity: EntityId, velocity: Vec3) {
        world
            .get_mut::<CharacterIntent>(entity)
            .unwrap()
            .set_horizontal_velocity(velocity);
    }

    fn request_character_jump(world: &mut World, entity: EntityId, launch_speed: f32) {
        world
            .get_mut::<CharacterIntent>(entity)
            .unwrap()
            .request_jump(launch_speed);
    }

    fn spawn_static_box(world: &mut World, translation: Vec3, half_extents: Vec3) -> EntityId {
        let entity = world.spawn();
        world.insert(entity, Transform::from_translation(translation));
        world.insert(entity, RigidBody::static_body());
        world.insert(entity, Collider::cuboid(half_extents));
        entity
    }

    fn spawn_dynamic_box(world: &mut World, translation: Vec3, half_extents: Vec3) -> EntityId {
        let entity = world.spawn();
        world.insert(entity, Transform::from_translation(translation));
        world.insert(entity, RigidBody::dynamic());
        world.insert(entity, Collider::cuboid(half_extents));
        entity
    }

    fn spawn_static_sensor(world: &mut World, translation: Vec3, half_extents: Vec3) -> EntityId {
        let entity = world.spawn();
        world.insert(entity, Transform::from_translation(translation));
        world.insert(entity, RigidBody::static_body());
        world.insert(entity, Collider::cuboid(half_extents).sensor(true));
        entity
    }

    fn only_physics_entity(world: &World) -> EntityId {
        world
            .iter::<RigidBody>()
            .next()
            .map(|(entity, _)| entity)
            .unwrap()
    }
}
