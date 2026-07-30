//! `PhysicsWorld` — the Rapier arena, held as a single resource.
//!
//! Everything Rapier owns lives here: the rigid-body and collider sets,
//! the broad/narrow-phase acceleration structures, the contact solver,
//! and the integration parameters. Gameplay never touches this resource;
//! it declares intent through the authoring components and the bridge is
//! the only system that reaches in. Handles are opaque and meaningless
//! outside the set that minted them, which is exactly why the two stores
//! need the bridge to stay reconciled (`plans/architecture/physics.md`).

use std::collections::HashMap;
use std::sync::Mutex;

use rapier3d::control::{CharacterLength as RapierCharacterLength, KinematicCharacterController};
use rapier3d::prelude::{nalgebra, *};

use crate::ecs::EntityId;
use crate::physics::{
    BodyKind, CharacterController, CharacterControllerState, CharacterLength, Collider,
    ColliderShape, Contact, ContactEvents, RigidBody, TeleportVelocity, TriggerEnter, TriggerExit,
};
use crate::transform::Transform;

const ENTITY_USER_DATA_TAG: u128 = 0x5343_484f_4f4e_4552;

/// Sync-safe collector for the solve's collision events.
///
/// Rapier's own `ChannelEventCollector` is built on `std::sync::mpsc`,
/// whose `Receiver` is `!Sync` — it could not live in a `Send + Sync`
/// resource. So we collect into a `Mutex<Vec<_>>` instead. The solver
/// calls the handler through `&self` (it must be safe even under the
/// parallel solver, which we don't enable); the lock makes that sound and
/// is uncontended in our single-threaded step.
#[derive(Default)]
pub(crate) struct CollisionSink {
    // Pushed during the solve; drained by the bridge once it maps Rapier
    // handles back to entities. Read path lands with the bridge body.
    #[allow(dead_code)]
    pub(crate) collisions: Mutex<Vec<CollisionEvent>>,
    // Post-solver impact signal. Collision Started/Stopped only tells us
    // topology changed; breakage and fall damage need the solver's contact
    // force callback path, where the resolved contact impulses are valid.
    // Materialized colliders arm CONTACT_FORCE_EVENTS; the bridge
    // drains this queue into `Contact`.
    #[allow(dead_code)]
    pub(crate) contact_impacts: Mutex<Vec<ContactImpact>>,
}

/// Post-solver impact sample collected from Rapier's contact-force path.
///
/// Rapier names the callback after force thresholds, but the `ContactPair`
/// available there still contains the solver impulses at each contact point.
/// We keep impulse as the engine payload because it is mass × Δvelocity —
/// the quantity breakage and damage rules actually want.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ContactImpact {
    pub collider1: ColliderHandle,
    pub collider2: ColliderHandle,
    pub impulse: Real,
    pub normal: Vector,
}

/// Rapier arena handles materialized for one ECS entity.
///
/// Kept inside [`PhysicsWorld`] so gameplay and future Glyph scripts cannot
/// retain stale backend tokens. The entity id remains the public identity.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PhysicsHandles {
    pub body: RigidBodyHandle,
    pub collider: ColliderHandle,
}

/// Minimal collider metadata needed after Rapier has removed a collider
/// from the live set but before its queued stopped-overlap events are
/// drained by the bridge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ColliderMetadata {
    entity: EntityId,
    is_sensor: bool,
}

/// Last ECS authoring state applied to a materialized body.
#[derive(Debug, Clone, Copy, PartialEq)]
struct BodyAuthoring {
    body: RigidBody,
    collider: Collider,
    contact_events: Option<ContactEvents>,
}

/// Solved pose emitted by Rapier for a dynamic body.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct PhysicsPose {
    pub entity: EntityId,
    pub translation: glam::Vec3,
    pub rotation: glam::Quat,
}

/// Collision-resolved result of one character movement command.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct CharacterMovement {
    pub entity: EntityId,
    pub translation: glam::Vec3,
    pub grounded: bool,
    pub vertical_velocity: f32,
}

/// Reusable bridge-owned output buffers. The bridge takes this bundle after a
/// solve, writes ECS state, then returns it to [`PhysicsWorld`] for the next
/// tick, avoiding steady-state per-step allocations.
#[derive(Default)]
pub(crate) struct PhysicsStepOutput {
    pub(crate) poses: Vec<PhysicsPose>,
    pub(crate) contacts: Vec<Contact>,
    pub(crate) trigger_enters: Vec<TriggerEnter>,
    pub(crate) trigger_exits: Vec<TriggerExit>,
}

impl EventHandler for CollisionSink {
    fn handle_collision_event(
        &self,
        _bodies: &RigidBodySet,
        _colliders: &ColliderSet,
        event: CollisionEvent,
        _contact_pair: Option<&ContactPair>,
    ) {
        if let Ok(mut queue) = self.collisions.lock() {
            queue.push(event);
        }
    }

    fn handle_contact_force_event(
        &self,
        _dt: Real,
        _bodies: &RigidBodySet,
        _colliders: &ColliderSet,
        contact_pair: &ContactPair,
        _total_force_magnitude: Real,
    ) {
        let mut impulse = 0.0;
        let mut strongest_impulse = 0.0;
        let mut strongest_normal = Vector::ZERO;

        for manifold in &contact_pair.manifolds {
            for point in manifold.contacts() {
                impulse += point.data.impulse;

                if point.data.impulse > strongest_impulse {
                    strongest_impulse = point.data.impulse;
                    strongest_normal = manifold.data.normal;
                }
            }
        }

        if let Ok(mut queue) = self.contact_impacts.lock() {
            queue.push(ContactImpact {
                collider1: contact_pair.collider1,
                collider2: contact_pair.collider2,
                impulse,
                normal: strongest_normal,
            });
        }
    }
}

/// The Rapier physics arena as an ECS resource. Opt in with
/// [`App::with_physics`](crate::App::with_physics); the bridge system is
/// the only thing that mutates it.
pub(crate) struct PhysicsWorld {
    pub(crate) gravity: Vector,
    pub(crate) integration_parameters: IntegrationParameters,
    pipeline: PhysicsPipeline,
    islands: IslandManager,
    broad_phase: BroadPhaseBvh,
    narrow_phase: NarrowPhase,
    bodies: RigidBodySet,
    colliders: ColliderSet,
    impulse_joints: ImpulseJointSet,
    multibody_joints: MultibodyJointSet,
    ccd_solver: CCDSolver,
    sink: CollisionSink,
    entities: HashMap<EntityId, PhysicsHandles>,
    body_entities: HashMap<RigidBodyHandle, EntityId>,
    collider_metadata: HashMap<ColliderHandle, ColliderMetadata>,
    stale_collider_metadata: HashMap<ColliderHandle, ColliderMetadata>,
    authoring: HashMap<EntityId, BodyAuthoring>,
    last_body_lifecycle_tick: u64,
    last_transform_sync_tick: u64,
    step_output: PhysicsStepOutput,
}

impl Default for PhysicsWorld {
    fn default() -> Self {
        Self::new()
    }
}

impl PhysicsWorld {
    pub(crate) fn new() -> Self {
        Self {
            // Standard earth gravity along -Y. The integration `dt` is set
            // by the bridge each step from `Time::fixed_delta`, so it
            // tracks the configured fixed rate without a construction-time
            // coupling to it.
            gravity: vector![0.0, -9.81, 0.0].into(),
            integration_parameters: IntegrationParameters::default(),
            pipeline: PhysicsPipeline::new(),
            islands: IslandManager::new(),
            broad_phase: BroadPhaseBvh::new(),
            narrow_phase: NarrowPhase::new(),
            bodies: RigidBodySet::new(),
            colliders: ColliderSet::new(),
            impulse_joints: ImpulseJointSet::new(),
            multibody_joints: MultibodyJointSet::new(),
            ccd_solver: CCDSolver::new(),
            sink: CollisionSink::default(),
            entities: HashMap::new(),
            body_entities: HashMap::new(),
            collider_metadata: HashMap::new(),
            stale_collider_metadata: HashMap::new(),
            authoring: HashMap::new(),
            last_body_lifecycle_tick: 0,
            last_transform_sync_tick: 0,
            step_output: PhysicsStepOutput::default(),
        }
    }

    /// Advance the simulation by one fixed step. Disjoint field borrows in
    /// the one call expression let the pipeline (the `&mut` receiver), the
    /// sets, and the event sink (`&self.sink`) all be borrowed at once. No
    /// physics hooks (`&()`); events land in the sink.
    pub(crate) fn step(&mut self) {
        self.pipeline.step(
            self.gravity,
            &self.integration_parameters,
            &mut self.islands,
            &mut self.broad_phase,
            &mut self.narrow_phase,
            &mut self.bodies,
            &mut self.colliders,
            &mut self.impulse_joints,
            &mut self.multibody_joints,
            &mut self.ccd_solver,
            &(),
            &self.sink,
        );
    }
}

#[allow(dead_code)]
impl PhysicsWorld {
    pub(crate) fn last_body_lifecycle_tick(&self) -> u64 {
        self.last_body_lifecycle_tick
    }

    pub(crate) fn set_last_body_lifecycle_tick(&mut self, tick: u64) {
        self.last_body_lifecycle_tick = tick;
    }

    pub(crate) fn last_transform_sync_tick(&self) -> u64 {
        self.last_transform_sync_tick
    }

    pub(crate) fn set_last_transform_sync_tick(&mut self, tick: u64) {
        self.last_transform_sync_tick = tick;
    }

    /// Materialize an ECS-authored body into Rapier and remember the
    /// private handles. Entities must carry both body and collider
    /// authoring components in Part 2's baseline bridge; a body without
    /// a collision proxy is not useful to Kinesis yet.
    pub(crate) fn materialize_body(
        &mut self,
        entity: EntityId,
        transform: Transform,
        body: RigidBody,
        collider: Collider,
    ) -> PhysicsHandles {
        self.materialize_body_with_contact_events(entity, transform, body, collider, None)
    }

    /// Materialize a body with optional post-solver contact reporting.
    pub(crate) fn materialize_body_with_contact_events(
        &mut self,
        entity: EntityId,
        transform: Transform,
        body: RigidBody,
        collider: Collider,
        contact_events: Option<ContactEvents>,
    ) -> PhysicsHandles {
        // Idempotence matters because change/removal ledgers have a
        // readable window. If a stale handle exists, free it before
        // installing the fresh Rapier objects.
        self.remove_body(entity);

        let user_data = Self::entity_user_data(entity);
        let body_builder = body_builder(body, transform).user_data(user_data);
        let body_handle = self.bodies.insert(body_builder);
        let collider_builder = collider_builder(collider, contact_events).user_data(user_data);
        let collider_handle =
            self.colliders
                .insert_with_parent(collider_builder, body_handle, &mut self.bodies);
        let handles = PhysicsHandles {
            body: body_handle,
            collider: collider_handle,
        };
        self.entities.insert(entity, handles);
        self.body_entities.insert(body_handle, entity);
        self.collider_metadata.insert(
            collider_handle,
            ColliderMetadata {
                entity,
                is_sensor: collider.sensor,
            },
        );
        self.authoring.insert(
            entity,
            BodyAuthoring {
                body,
                collider,
                contact_events,
            },
        );
        handles
    }

    /// Apply compatible authoring changes without replacing the Rapier
    /// objects. This preserves dynamic velocity, forces, sleeping state,
    /// and solver warm-starting for routine runtime edits like material,
    /// mass, sensor, contact reporting, or body-authority changes. Shape
    /// changes deliberately fall back to rematerialization for now because
    /// they alter broad-phase geometry and mass properties.
    pub(crate) fn update_body_authoring(
        &mut self,
        entity: EntityId,
        body: RigidBody,
        collider: Collider,
        contact_events: Option<ContactEvents>,
    ) -> bool {
        let Some(handles) = self.handles(entity) else {
            return false;
        };
        let Some(previous) = self.authoring.get(&entity).copied() else {
            return false;
        };
        if previous.collider.shape != collider.shape {
            return false;
        }

        let Some(rapier_body) = self.bodies.get_mut(handles.body) else {
            return false;
        };
        rapier_body.set_body_type(rigid_body_type(body.kind), true);

        let Some(rapier_collider) = self.colliders.get_mut(handles.collider) else {
            return false;
        };
        rapier_collider.set_mass(collider.mass);
        rapier_collider.set_friction(collider.material.friction);
        rapier_collider.set_restitution(collider.material.restitution);
        rapier_collider.set_sensor(collider.sensor);
        rapier_collider.set_active_events(active_events(collider, contact_events));
        if let Some(events) = contact_events {
            rapier_collider.set_contact_force_event_threshold(events.force_threshold);
        }

        if let Some(metadata) = self.collider_metadata.get_mut(&handles.collider) {
            metadata.is_sensor = collider.sensor;
        }
        self.authoring.insert(
            entity,
            BodyAuthoring {
                body,
                collider,
                contact_events,
            },
        );
        true
    }

    /// Remove an entity's Rapier body and any attached collider. Safe to
    /// call repeatedly; the side map is the cleanup authority.
    pub(crate) fn remove_body(&mut self, entity: EntityId) -> Option<PhysicsHandles> {
        let handles = self.entities.remove(&entity)?;
        self.body_entities.remove(&handles.body);
        self.authoring.remove(&entity);
        if let Some(metadata) = self.collider_metadata.remove(&handles.collider) {
            self.stale_collider_metadata
                .insert(handles.collider, metadata);
        }
        self.bodies.remove(
            handles.body,
            &mut self.islands,
            &mut self.colliders,
            &mut self.impulse_joints,
            &mut self.multibody_joints,
            true,
        );
        Some(handles)
    }

    /// Push a regular ECS-authored pose into Rapier. Kinematic bodies use
    /// Rapier's next-kinematic-position path so their motion produces the
    /// velocity dynamic contacts need; dynamic bodies remain solver-owned.
    pub(crate) fn set_authored_pose(&mut self, entity: EntityId, transform: Transform) -> bool {
        let Some(handles) = self.handles(entity) else {
            return false;
        };
        let Some(body) = self.bodies.get_mut(handles.body) else {
            return false;
        };
        let pose = transform_pose(transform);
        if body.body_type() == RigidBodyType::KinematicPositionBased {
            body.set_next_kinematic_position(pose);
            true
        } else if body.is_fixed() {
            body.set_position(pose, true);
            true
        } else {
            false
        }
    }

    /// Resolve one fixed step of character movement against the hosted world.
    ///
    /// The query excludes the character's own collider. Horizontal velocity
    /// comes from gameplay; gravity and ground detection remain physics
    /// outcomes so downstream rules do not infer them independently.
    pub(crate) fn move_character(
        &mut self,
        entity: EntityId,
        controller: CharacterController,
        state: CharacterControllerState,
        horizontal_velocity: glam::Vec3,
        dt: f32,
    ) -> Option<CharacterMovement> {
        let handles = self.handles(entity)?;
        let body = self.bodies.get(handles.body)?;
        if body.body_type() != RigidBodyType::KinematicPositionBased {
            return None;
        }
        let character_position = *body.position();

        let mut vertical_velocity = state.vertical_velocity;
        if state.grounded && vertical_velocity < 0.0 {
            vertical_velocity = 0.0;
        }
        vertical_velocity += self.gravity.y * dt;

        let desired_translation = Vector::new(
            horizontal_velocity.x * dt,
            vertical_velocity * dt,
            horizontal_velocity.z * dt,
        );
        let effective = {
            puffin::profile_scope!("physics.character_kcc_query");
            let collider = self.colliders.get(handles.collider)?;
            let query = self.broad_phase.as_query_pipeline(
                self.narrow_phase.query_dispatcher(),
                &self.bodies,
                &self.colliders,
                QueryFilter::default().exclude_rigid_body(handles.body),
            );
            let controller = rapier_character_controller(controller);
            controller.move_shape(
                dt,
                &query,
                collider.shape(),
                &character_position,
                desired_translation,
                |_| {},
            )
        };

        if effective.grounded && vertical_velocity < 0.0 {
            vertical_velocity = 0.0;
        }

        let translation = character_position.translation + effective.translation;
        self.bodies
            .get_mut(handles.body)?
            .set_next_kinematic_translation(translation);

        Some(CharacterMovement {
            entity,
            translation: rapier_vec_to_glam(translation),
            grounded: effective.grounded,
            vertical_velocity,
        })
    }

    /// Move a body discontinuously. This intentionally bypasses the regular
    /// kinematic-motion path; gameplay should use it only for teleports.
    pub(crate) fn teleport_body(
        &mut self,
        entity: EntityId,
        transform: Transform,
        velocity: TeleportVelocity,
    ) -> bool {
        let Some(handles) = self.handles(entity) else {
            return false;
        };
        let Some(body) = self.bodies.get_mut(handles.body) else {
            return false;
        };
        let pose = transform_pose(transform);
        body.set_position(pose, true);
        if body.body_type() == RigidBodyType::KinematicPositionBased {
            body.set_next_kinematic_position(pose);
        }
        if body.is_dynamic() && velocity == TeleportVelocity::Clear {
            body.set_linvel(vector![0.0, 0.0, 0.0].into(), true);
            body.set_angvel(vector![0.0, 0.0, 0.0].into(), true);
        }
        true
    }

    /// Take one solve's reusable bridge output. Call
    /// [`Self::recycle_step_output`] after the ECS bridge has consumed it.
    pub(crate) fn take_step_output(&mut self) -> PhysicsStepOutput {
        let mut output = std::mem::take(&mut self.step_output);
        for handle in self.islands.active_bodies() {
            let Some(&entity) = self.body_entities.get(&handle) else {
                continue;
            };
            let Some(body) = self.bodies.get(handle) else {
                continue;
            };
            if body.is_dynamic() {
                output.poses.push(PhysicsPose {
                    entity,
                    translation: rapier_vec_to_glam(body.translation()),
                    rotation: rapier_rotation_to_glam(body.rotation()),
                });
            }
        }

        let colliders = &self.colliders;
        if let Ok(mut queue) = self.sink.contact_impacts.lock() {
            for impact in queue.drain(..) {
                let Some(a) = Self::entity_for_collider_in(colliders, impact.collider1) else {
                    continue;
                };
                let Some(b) = Self::entity_for_collider_in(colliders, impact.collider2) else {
                    continue;
                };
                output.contacts.push(Contact {
                    a,
                    b,
                    impulse: impact.impulse,
                    normal: rapier_vec_to_glam(impact.normal),
                });
            }
        }

        if let Ok(mut queue) = self.sink.collisions.lock() {
            for event in queue.drain(..) {
                match event {
                    CollisionEvent::Started(collider1, collider2, _) => {
                        if let Some(event) =
                            Self::trigger_enter_for(colliders, collider1, collider2)
                        {
                            output.trigger_enters.push(event);
                        }
                    }
                    CollisionEvent::Stopped(collider1, collider2, _) => {
                        if let Some(event) = self.trigger_exit_for(collider1, collider2) {
                            output.trigger_exits.push(event);
                        }
                    }
                }
            }
        }
        self.stale_collider_metadata.clear();
        output
    }

    pub(crate) fn recycle_step_output(&mut self, mut output: PhysicsStepOutput) {
        output.poses.clear();
        output.contacts.clear();
        output.trigger_enters.clear();
        output.trigger_exits.clear();
        self.step_output = output;
    }

    fn trigger_enter_for(
        colliders: &ColliderSet,
        collider1: ColliderHandle,
        collider2: ColliderHandle,
    ) -> Option<TriggerEnter> {
        let first = colliders.get(collider1)?;
        let second = colliders.get(collider2)?;
        let first_entity = Self::entity_from_user_data(first.user_data)?;
        let second_entity = Self::entity_from_user_data(second.user_data)?;

        match (first.is_sensor(), second.is_sensor()) {
            (true, false) => Some(TriggerEnter {
                sensor: first_entity,
                other: second_entity,
            }),
            (false, true) => Some(TriggerEnter {
                sensor: second_entity,
                other: first_entity,
            }),
            _ => None,
        }
    }

    fn trigger_exit_for(
        &self,
        collider1: ColliderHandle,
        collider2: ColliderHandle,
    ) -> Option<TriggerExit> {
        let first = self.metadata_for_collider(collider1)?;
        let second = self.metadata_for_collider(collider2)?;

        match (first.is_sensor, second.is_sensor) {
            (true, false) => Some(TriggerExit {
                sensor: first.entity,
                other: second.entity,
            }),
            (false, true) => Some(TriggerExit {
                sensor: second.entity,
                other: first.entity,
            }),
            _ => None,
        }
    }

    fn metadata_for_collider(&self, handle: ColliderHandle) -> Option<ColliderMetadata> {
        self.collider_metadata
            .get(&handle)
            .or_else(|| self.stale_collider_metadata.get(&handle))
            .copied()
    }

    fn entity_for_collider_in(colliders: &ColliderSet, handle: ColliderHandle) -> Option<EntityId> {
        let collider = colliders.get(handle)?;
        Self::entity_from_user_data(collider.user_data)
    }

    pub(crate) fn insert_handles(
        &mut self,
        entity: EntityId,
        handles: PhysicsHandles,
    ) -> Option<PhysicsHandles> {
        self.body_entities.insert(handles.body, entity);
        self.entities.insert(entity, handles)
    }

    pub(crate) fn remove_handles(&mut self, entity: EntityId) -> Option<PhysicsHandles> {
        let handles = self.entities.remove(&entity)?;
        self.body_entities.remove(&handles.body);
        self.collider_metadata.remove(&handles.collider);
        self.authoring.remove(&entity);
        Some(handles)
    }

    pub(crate) fn handles(&self, entity: EntityId) -> Option<PhysicsHandles> {
        self.entities.get(&entity).copied()
    }

    pub(crate) fn entity_count(&self) -> usize {
        self.entities.len()
    }

    pub(crate) fn body_count(&self) -> usize {
        self.bodies.len()
    }

    pub(crate) fn collider_count(&self) -> usize {
        self.colliders.len()
    }

    pub(crate) fn entity_user_data(entity: EntityId) -> u128 {
        (ENTITY_USER_DATA_TAG << 64)
            | ((entity.generation as u128) << u32::BITS)
            | entity.index as u128
    }

    pub(crate) fn entity_from_user_data(user_data: u128) -> Option<EntityId> {
        if user_data >> 64 != ENTITY_USER_DATA_TAG {
            return None;
        }

        Some(EntityId {
            index: user_data as u32,
            generation: (user_data >> u32::BITS) as u32,
        })
    }
}

fn body_builder(body: RigidBody, transform: Transform) -> RigidBodyBuilder {
    let builder = RigidBodyBuilder::new(rigid_body_type(body.kind));
    builder.pose(transform_pose(transform))
}

fn collider_builder(collider: Collider, contact_events: Option<ContactEvents>) -> ColliderBuilder {
    let builder = match collider.shape {
        ColliderShape::Cuboid { half_extents } => {
            ColliderBuilder::cuboid(half_extents.x, half_extents.y, half_extents.z)
        }
        ColliderShape::Ball { radius } => ColliderBuilder::ball(radius),
        ColliderShape::CapsuleY {
            half_height,
            radius,
        } => ColliderBuilder::capsule_y(half_height, radius),
    };

    let builder = builder
        .mass(collider.mass)
        .friction(collider.material.friction)
        .restitution(collider.material.restitution)
        .sensor(collider.sensor)
        .active_events(active_events(collider, contact_events));
    match contact_events {
        Some(events) => builder.contact_force_event_threshold(events.force_threshold),
        None => builder,
    }
}

fn rigid_body_type(kind: BodyKind) -> RigidBodyType {
    match kind {
        BodyKind::Dynamic => RigidBodyType::Dynamic,
        BodyKind::Static => RigidBodyType::Fixed,
        BodyKind::KinematicPositionBased => RigidBodyType::KinematicPositionBased,
    }
}

fn active_events(collider: Collider, contact_events: Option<ContactEvents>) -> ActiveEvents {
    if collider.sensor {
        ActiveEvents::COLLISION_EVENTS
    } else if contact_events.is_some() {
        ActiveEvents::CONTACT_FORCE_EVENTS
    } else {
        ActiveEvents::empty()
    }
}

fn rapier_character_controller(controller: CharacterController) -> KinematicCharacterController {
    KinematicCharacterController {
        offset: rapier_character_length(controller.offset),
        slide: controller.slide,
        max_slope_climb_angle: controller.max_slope_climb_angle,
        min_slope_slide_angle: controller.min_slope_slide_angle,
        snap_to_ground: controller.snap_to_ground.map(rapier_character_length),
        ..KinematicCharacterController::default()
    }
}

fn rapier_character_length(length: CharacterLength) -> RapierCharacterLength {
    match length {
        CharacterLength::Relative(value) => RapierCharacterLength::Relative(value),
        CharacterLength::Absolute(value) => RapierCharacterLength::Absolute(value),
    }
}

fn transform_pose(transform: Transform) -> Pose {
    let t = transform.translation;
    let r = transform.rotation;
    let rotation =
        nalgebra::UnitQuaternion::new_normalize(nalgebra::Quaternion::new(r.w, r.x, r.y, r.z));

    nalgebra::Isometry3::from_parts(nalgebra::Translation3::new(t.x, t.y, t.z), rotation).into()
}

fn rapier_vec_to_glam(v: Vector) -> glam::Vec3 {
    glam::Vec3::new(v.x, v.y, v.z)
}

fn rapier_rotation_to_glam(rotation: &Rotation) -> glam::Quat {
    glam::Quat::from_xyzw(rotation.x, rotation.y, rotation.z, rotation.w)
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::{Quat, Vec3};

    #[test]
    fn entity_user_data_round_trips_with_generation() {
        let entity = EntityId {
            index: 42,
            generation: 7,
        };

        let user_data = PhysicsWorld::entity_user_data(entity);

        assert_eq!(PhysicsWorld::entity_from_user_data(user_data), Some(entity));
    }

    #[test]
    fn entity_user_data_ignores_unstamped_rapier_objects() {
        assert_eq!(PhysicsWorld::entity_from_user_data(0), None);
    }

    #[test]
    fn handle_map_is_owned_by_physics_world() {
        let mut physics = PhysicsWorld::new();
        let entity = EntityId {
            index: 3,
            generation: 1,
        };
        let handles = PhysicsHandles {
            body: RigidBodyHandle::from_raw_parts(11, 0),
            collider: ColliderHandle::from_raw_parts(17, 0),
        };

        assert_eq!(physics.entity_count(), 0);
        assert_eq!(physics.insert_handles(entity, handles), None);
        assert_eq!(physics.handles(entity), Some(handles));
        assert_eq!(physics.entity_count(), 1);
        assert_eq!(physics.remove_handles(entity), Some(handles));
        assert_eq!(physics.handles(entity), None);
        assert_eq!(physics.entity_count(), 0);
    }

    #[test]
    fn materialize_body_creates_rapier_body_and_collider() {
        let mut physics = PhysicsWorld::new();
        let entity = EntityId {
            index: 4,
            generation: 2,
        };
        let transform = Transform {
            translation: Vec3::new(1.0, 2.0, 3.0),
            rotation: Quat::from_rotation_y(std::f32::consts::FRAC_PI_2),
            scale: Vec3::splat(2.0),
        };
        let collider = Collider::cuboid(Vec3::new(0.5, 1.0, 1.5)).mass(3.0);

        let handles = physics.materialize_body(entity, transform, RigidBody::dynamic(), collider);

        assert_eq!(physics.handles(entity), Some(handles));
        assert_eq!(physics.entity_count(), 1);
        assert_eq!(physics.bodies.len(), 1);
        assert_eq!(physics.colliders.len(), 1);

        let body = &physics.bodies[handles.body];
        assert_eq!(body.user_data, PhysicsWorld::entity_user_data(entity));
        assert_eq!(body.translation(), vector![1.0, 2.0, 3.0].into());

        let collider = &physics.colliders[handles.collider];
        assert_eq!(collider.user_data, PhysicsWorld::entity_user_data(entity));
        assert!(collider.active_events().is_empty());
    }

    #[test]
    fn remove_body_drops_map_body_and_attached_collider() {
        let mut physics = PhysicsWorld::new();
        let entity = EntityId {
            index: 5,
            generation: 1,
        };
        let handles = physics.materialize_body(
            entity,
            Transform::from_translation(Vec3::new(0.0, 5.0, 0.0)),
            RigidBody::dynamic(),
            Collider::ball(0.5),
        );

        assert_eq!(physics.remove_body(entity), Some(handles));
        assert_eq!(physics.handles(entity), None);
        assert_eq!(physics.entity_count(), 0);
        assert_eq!(physics.bodies.len(), 0);
        assert_eq!(physics.colliders.len(), 0);
        assert_eq!(physics.remove_body(entity), None);
    }

    #[test]
    fn lifecycle_cursor_is_owned_by_physics_world() {
        let mut physics = PhysicsWorld::new();
        assert_eq!(physics.last_body_lifecycle_tick(), 0);
        physics.set_last_body_lifecycle_tick(12);
        assert_eq!(physics.last_body_lifecycle_tick(), 12);
        assert_eq!(physics.last_transform_sync_tick(), 0);
        physics.set_last_transform_sync_tick(13);
        assert_eq!(physics.last_transform_sync_tick(), 13);
    }

    #[test]
    fn authored_kinematic_pose_sets_next_position_without_teleporting() {
        let mut physics = PhysicsWorld::new();
        let entity = EntityId {
            index: 6,
            generation: 0,
        };
        physics.materialize_body(
            entity,
            Transform::IDENTITY,
            RigidBody::kinematic_position_based(),
            Collider::ball(0.5),
        );
        let transform = Transform {
            translation: Vec3::new(3.0, 4.0, 5.0),
            rotation: Quat::from_rotation_x(0.5),
            scale: Vec3::splat(7.0),
        };

        assert!(physics.set_authored_pose(entity, transform));

        let handles = physics.handles(entity).unwrap();
        let body = &physics.bodies[handles.body];
        assert_eq!(body.translation(), vector![0.0, 0.0, 0.0].into());
        assert_eq!(
            body.next_position().translation,
            vector![3.0, 4.0, 5.0].into()
        );
    }

    #[test]
    fn teleport_body_clears_dynamic_velocity_when_requested() {
        let mut physics = PhysicsWorld::new();
        let entity = EntityId {
            index: 7,
            generation: 0,
        };
        let handles = physics.materialize_body(
            entity,
            Transform::IDENTITY,
            RigidBody::dynamic(),
            Collider::ball(0.5),
        );
        let body = physics.bodies.get_mut(handles.body).unwrap();
        body.set_linvel(vector![3.0, 4.0, 5.0].into(), true);
        body.set_angvel(vector![0.1, 0.2, 0.3].into(), true);

        assert!(physics.teleport_body(
            entity,
            Transform::from_translation(Vec3::new(8.0, 9.0, 10.0)),
            TeleportVelocity::Clear,
        ));

        let body = &physics.bodies[handles.body];
        assert_eq!(body.translation(), vector![8.0, 9.0, 10.0].into());
        assert_eq!(body.linvel(), vector![0.0, 0.0, 0.0].into());
        assert_eq!(body.angvel(), vector![0.0, 0.0, 0.0].into());
    }

    #[test]
    fn teleport_body_can_preserve_dynamic_velocity() {
        let mut physics = PhysicsWorld::new();
        let entity = EntityId {
            index: 8,
            generation: 0,
        };
        let handles = physics.materialize_body(
            entity,
            Transform::IDENTITY,
            RigidBody::dynamic(),
            Collider::ball(0.5),
        );
        let body = physics.bodies.get_mut(handles.body).unwrap();
        body.set_linvel(vector![3.0, 4.0, 5.0].into(), true);
        body.set_angvel(vector![0.1, 0.2, 0.3].into(), true);

        assert!(physics.teleport_body(
            entity,
            Transform::from_translation(Vec3::new(8.0, 9.0, 10.0)),
            TeleportVelocity::Preserve,
        ));

        let body = &physics.bodies[handles.body];
        assert_eq!(body.translation(), vector![8.0, 9.0, 10.0].into());
        assert_eq!(body.linvel(), vector![3.0, 4.0, 5.0].into());
        assert_eq!(body.angvel(), vector![0.1, 0.2, 0.3].into());
    }

    #[test]
    fn teleport_kinematic_pose_resets_current_and_next_position() {
        let mut physics = PhysicsWorld::new();
        let entity = EntityId {
            index: 9,
            generation: 0,
        };
        physics.materialize_body(
            entity,
            Transform::IDENTITY,
            RigidBody::kinematic_position_based(),
            Collider::ball(0.5),
        );
        assert!(physics.set_authored_pose(
            entity,
            Transform::from_translation(Vec3::new(1.0, 0.0, 0.0)),
        ));

        assert!(physics.teleport_body(
            entity,
            Transform::from_translation(Vec3::new(4.0, 5.0, 6.0)),
            TeleportVelocity::Clear,
        ));

        let handles = physics.handles(entity).unwrap();
        let body = &physics.bodies[handles.body];
        assert_eq!(body.translation(), vector![4.0, 5.0, 6.0].into());
        assert_eq!(
            body.next_position().translation,
            vector![4.0, 5.0, 6.0].into()
        );
    }

    #[test]
    fn dynamic_body_poses_only_reports_dynamic_bodies() {
        let mut physics = PhysicsWorld::new();
        let dynamic = EntityId {
            index: 7,
            generation: 0,
        };
        let static_body = EntityId {
            index: 8,
            generation: 0,
        };
        physics.materialize_body(
            dynamic,
            Transform::from_translation(Vec3::new(0.0, 2.0, 0.0)),
            RigidBody::dynamic(),
            Collider::ball(0.5),
        );
        physics.materialize_body(
            static_body,
            Transform::from_translation(Vec3::new(0.0, -1.0, 0.0)),
            RigidBody::static_body(),
            Collider::ball(0.5),
        );

        physics.step();
        let output = physics.take_step_output();

        assert_eq!(output.poses.len(), 1);
        assert_eq!(output.poses[0].entity, dynamic);
    }

    #[test]
    fn drain_trigger_events_maps_sensor_overlap_lifecycle() {
        let mut physics = PhysicsWorld::new();
        let sensor = EntityId {
            index: 9,
            generation: 0,
        };
        let other = EntityId {
            index: 10,
            generation: 0,
        };
        let sensor_handles = physics.materialize_body(
            sensor,
            Transform::IDENTITY,
            RigidBody::static_body(),
            Collider::ball(1.0).sensor(true),
        );
        let other_handles = physics.materialize_body(
            other,
            Transform::IDENTITY,
            RigidBody::dynamic(),
            Collider::ball(1.0),
        );
        physics
            .sink
            .collisions
            .lock()
            .unwrap()
            .push(CollisionEvent::Started(
                sensor_handles.collider,
                other_handles.collider,
                CollisionEventFlags::SENSOR,
            ));
        physics
            .sink
            .collisions
            .lock()
            .unwrap()
            .push(CollisionEvent::Stopped(
                sensor_handles.collider,
                other_handles.collider,
                CollisionEventFlags::SENSOR,
            ));

        let output = physics.take_step_output();

        assert_eq!(output.trigger_enters.len(), 1);
        assert_eq!(output.trigger_enters[0].sensor, sensor);
        assert_eq!(output.trigger_enters[0].other, other);
        assert_eq!(output.trigger_exits.len(), 1);
        assert_eq!(output.trigger_exits[0].sensor, sensor);
        assert_eq!(output.trigger_exits[0].other, other);
    }

    #[test]
    fn trigger_exit_maps_removed_collider_through_stale_metadata() {
        let mut physics = PhysicsWorld::new();
        let sensor = EntityId {
            index: 11,
            generation: 0,
        };
        let other = EntityId {
            index: 12,
            generation: 0,
        };
        let sensor_handles = physics.materialize_body(
            sensor,
            Transform::IDENTITY,
            RigidBody::static_body(),
            Collider::ball(1.0).sensor(true),
        );
        let other_handles = physics.materialize_body(
            other,
            Transform::IDENTITY,
            RigidBody::dynamic(),
            Collider::ball(1.0),
        );

        physics.remove_body(other);
        physics
            .sink
            .collisions
            .lock()
            .unwrap()
            .push(CollisionEvent::Stopped(
                sensor_handles.collider,
                other_handles.collider,
                CollisionEventFlags::SENSOR,
            ));

        let output = physics.take_step_output();

        assert_eq!(output.trigger_exits.len(), 1);
        assert_eq!(output.trigger_exits[0].sensor, sensor);
        assert_eq!(output.trigger_exits[0].other, other);
    }

    #[test]
    fn compatible_authoring_update_preserves_dynamic_body_state() {
        let mut physics = PhysicsWorld::new();
        let entity = EntityId {
            index: 13,
            generation: 0,
        };
        let handles = physics.materialize_body(
            entity,
            Transform::IDENTITY,
            RigidBody::dynamic(),
            Collider::ball(1.0),
        );
        physics
            .bodies
            .get_mut(handles.body)
            .unwrap()
            .set_linvel(vector![3.0, 4.0, 5.0].into(), true);

        let updated = Collider::ball(1.0)
            .mass(4.0)
            .material(crate::physics::PhysicsMaterial {
                friction: 0.9,
                restitution: 0.25,
            });

        assert!(physics.update_body_authoring(
            entity,
            RigidBody::dynamic(),
            updated,
            Some(ContactEvents::new(12.0)),
        ));

        assert_eq!(physics.handles(entity), Some(handles));
        let body = &physics.bodies[handles.body];
        assert_eq!(body.linvel(), vector![3.0, 4.0, 5.0].into());
        let collider = &physics.colliders[handles.collider];
        assert_eq!(collider.mass(), 4.0);
        assert_eq!(collider.friction(), 0.9);
        assert_eq!(collider.restitution(), 0.25);
        assert_eq!(collider.active_events(), ActiveEvents::CONTACT_FORCE_EVENTS);
    }

    #[test]
    fn drain_contacts_maps_collider_handles_to_entities() {
        let mut physics = PhysicsWorld::new();
        let first = EntityId {
            index: 14,
            generation: 0,
        };
        let second = EntityId {
            index: 15,
            generation: 0,
        };
        let first_handles = physics.materialize_body(
            first,
            Transform::IDENTITY,
            RigidBody::dynamic(),
            Collider::ball(1.0),
        );
        let second_handles = physics.materialize_body(
            second,
            Transform::IDENTITY,
            RigidBody::dynamic(),
            Collider::ball(1.0),
        );
        physics
            .sink
            .contact_impacts
            .lock()
            .unwrap()
            .push(ContactImpact {
                collider1: first_handles.collider,
                collider2: second_handles.collider,
                impulse: 42.0,
                normal: vector![0.0, 1.0, 0.0].into(),
            });

        let output = physics.take_step_output();

        assert_eq!(output.contacts.len(), 1);
        assert_eq!(output.contacts[0].a, first);
        assert_eq!(output.contacts[0].b, second);
        assert_eq!(output.contacts[0].impulse, 42.0);
        assert_eq!(output.contacts[0].normal, Vec3::Y);
    }
}
