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
    CharacterController, CharacterControllerState, CharacterMovement, Collider, Contact,
    ContactEvents, PhysicsCharacterWorkload, PhysicsCommand, PhysicsCommandWorkload,
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
    let (commands, characters) = apply_physics_commands(world, dt);

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
    since: u64,
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

fn body_spawns_since(world: &mut World, since: u64) -> (Vec<BodySpawn>, usize) {
    let mut out = Vec::new();
    let rigid_body_changes: Vec<EntityId> = world
        .added_since::<RigidBody>(since)
        .map(|(entity, _)| entity)
        .chain(
            world
                .changed_since::<RigidBody>(since)
                .map(|(entity, _)| entity),
        )
        .collect();
    let collider_changes: Vec<EntityId> = world
        .added_since::<Collider>(since)
        .map(|(entity, _)| entity)
        .chain(
            world
                .changed_since::<Collider>(since)
                .map(|(entity, _)| entity),
        )
        .collect();
    let transform_adds: Vec<EntityId> = world
        .added_since::<Transform>(since)
        .map(|(entity, _)| entity)
        .collect();
    let contact_event_changes: Vec<EntityId> = world
        .added_since::<ContactEvents>(since)
        .map(|(entity, _)| entity)
        .chain(
            world
                .changed_since::<ContactEvents>(since)
                .map(|(entity, _)| entity),
        )
        .collect();
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
    since: u64,
    sync_tick: u64,
) -> PhysicsTransformSyncWorkload {
    puffin::profile_scope!("physics.authored_transform_sync");

    let changed: Vec<(EntityId, Transform)> = world
        .changed_since::<Transform>(since)
        .map(|(entity, transform)| (entity, *transform))
        .collect();
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

fn apply_physics_commands(
    world: &mut World,
    dt: f32,
) -> (PhysicsCommandWorkload, PhysicsCharacterWorkload) {
    puffin::profile_scope!("physics.command_processing");

    let mut command_workload = PhysicsCommandWorkload::default();
    let mut character_workload = PhysicsCharacterWorkload::default();
    if !world.contains_resource::<PhysicsWorld>() {
        return (command_workload, character_workload);
    }
    let commands: Vec<PhysicsCommand> = match world.resource_mut::<PhysicsCommands>() {
        Some(commands) => commands.drain().collect(),
        None => return (command_workload, character_workload),
    };
    command_workload.total = count(commands.len());
    if commands.is_empty() {
        return (command_workload, character_workload);
    }

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
            PhysicsCommand::MoveCharacter {
                entity,
                horizontal_velocity,
            } => {
                puffin::profile_scope!("physics.character_integration");
                command_workload.character_moves =
                    command_workload.character_moves.saturating_add(1);
                let Some(controller) = world.get::<CharacterController>(entity).copied() else {
                    continue;
                };
                let Some(state) = world.get::<CharacterControllerState>(entity).copied() else {
                    continue;
                };
                let movement = world.resource_mut::<PhysicsWorld>().and_then(|physics| {
                    physics.move_character(entity, controller, state, horizontal_velocity, dt)
                });
                if let Some(movement) = movement {
                    character_workload.integrations =
                        character_workload.integrations.saturating_add(1);
                    character_workload.kcc_queries =
                        character_workload.kcc_queries.saturating_add(1);
                    apply_character_movement(world, movement);
                }
            }
            PhysicsCommand::JumpCharacter {
                entity,
                launch_speed,
            } => {
                command_workload.character_jumps =
                    command_workload.character_jumps.saturating_add(1);
                let Some(mut state) = world.get_mut::<CharacterControllerState>(entity) else {
                    continue;
                };
                if state.grounded {
                    state.grounded = false;
                    state.vertical_velocity = launch_speed;
                }
            }
        }
    }

    (command_workload, character_workload)
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
        assert_eq!(physics.last_body_lifecycle_tick(), world.current_tick());
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
    fn character_move_applies_gravity_while_airborne() {
        let mut world = physics_world();
        let character = spawn_character(&mut world, Vec3::new(0.0, 2.0, 0.0));
        physics_bridge(&mut world);

        world.increment_tick();
        world
            .resource_mut::<PhysicsCommands>()
            .unwrap()
            .move_character(character, Vec3::ZERO);
        physics_bridge(&mut world);

        let transform = world.get::<Transform>(character).unwrap();
        let state = world.get::<CharacterControllerState>(character).unwrap();
        assert!(transform.translation.y < 2.0);
        assert!(!state.grounded);
        assert!(state.vertical_velocity < 0.0);
        let diagnostics = world.resource::<PhysicsDiagnostics>().unwrap();
        assert_eq!(diagnostics.commands.total, 1);
        assert_eq!(diagnostics.commands.character_moves, 1);
        assert_eq!(diagnostics.characters.integrations, 1);
        assert_eq!(diagnostics.characters.kcc_queries, 1);
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
        world
            .resource_mut::<PhysicsCommands>()
            .unwrap()
            .move_character(character, Vec3::new(120.0, 0.0, 0.0));
        physics_bridge(&mut world);

        let x = world.get::<Transform>(character).unwrap().translation.x;
        assert!(x > 0.1, "self-collision prevented movement: x={x}");
        assert!(x < 0.8, "character crossed the wall: x={x}");
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

        world.increment_tick();
        world
            .resource_mut::<PhysicsCommands>()
            .unwrap()
            .move_character(character, Vec3::ZERO);
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

        world.increment_tick();
        world
            .resource_mut::<PhysicsCommands>()
            .unwrap()
            .move_character(character, Vec3::ZERO);
        physics_bridge(&mut world);
        assert!(
            world
                .get::<CharacterControllerState>(character)
                .unwrap()
                .grounded
        );
        let grounded_y = world.get::<Transform>(character).unwrap().translation.y;

        world.increment_tick();
        {
            let commands = world.resource_mut::<PhysicsCommands>().unwrap();
            commands.jump_character(character, 5.0);
            commands.move_character(character, Vec3::ZERO);
        }
        physics_bridge(&mut world);

        let launched = *world.get::<CharacterControllerState>(character).unwrap();
        assert!(!launched.grounded);
        assert!(launched.vertical_velocity > 0.0);
        assert!(world.get::<Transform>(character).unwrap().translation.y > grounded_y);

        world.increment_tick();
        world
            .resource_mut::<PhysicsCommands>()
            .unwrap()
            .move_character(character, Vec3::ZERO);
        physics_bridge(&mut world);

        let continued = world.get::<CharacterControllerState>(character).unwrap();
        assert!(continued.vertical_velocity < launched.vertical_velocity);
        assert!(continued.vertical_velocity > 0.0);
    }

    #[test]
    fn airborne_character_rejects_jump_request() {
        let mut world = physics_world();
        let character = spawn_character(&mut world, Vec3::new(0.0, 2.0, 0.0));
        physics_bridge(&mut world);

        world.increment_tick();
        {
            let commands = world.resource_mut::<PhysicsCommands>().unwrap();
            commands.jump_character(character, 5.0);
            commands.move_character(character, Vec3::ZERO);
        }
        physics_bridge(&mut world);

        let state = world.get::<CharacterControllerState>(character).unwrap();
        assert!(!state.grounded);
        assert!(state.vertical_velocity < 0.0);
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
        entity
    }

    fn spawn_static_box(world: &mut World, translation: Vec3, half_extents: Vec3) -> EntityId {
        let entity = world.spawn();
        world.insert(entity, Transform::from_translation(translation));
        world.insert(entity, RigidBody::static_body());
        world.insert(entity, Collider::cuboid(half_extents));
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
