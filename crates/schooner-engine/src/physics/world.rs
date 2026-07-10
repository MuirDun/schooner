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

use rapier3d::prelude::{nalgebra, *};

use crate::ecs::EntityId;
use crate::physics::{BodyKind, Collider, ColliderShape, RigidBody};
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
    // 2.D.3 must arm CONTACT_FORCE_EVENTS on materialized colliders and
    // 2.D.4 drains this queue into `Contact`.
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
    last_body_lifecycle_tick: u64,
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
            last_body_lifecycle_tick: 0,
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
        // Idempotence matters because change/removal ledgers have a
        // readable window. If a stale handle exists, free it before
        // installing the fresh Rapier objects.
        self.remove_body(entity);

        let user_data = Self::entity_user_data(entity);
        let body_builder = body_builder(body, transform).user_data(user_data);
        let body_handle = self.bodies.insert(body_builder);
        let collider_builder = collider_builder(collider).user_data(user_data);
        let collider_handle =
            self.colliders
                .insert_with_parent(collider_builder, body_handle, &mut self.bodies);
        let handles = PhysicsHandles {
            body: body_handle,
            collider: collider_handle,
        };
        self.entities.insert(entity, handles);
        handles
    }

    /// Remove an entity's Rapier body and any attached collider. Safe to
    /// call repeatedly; the side map is the cleanup authority.
    pub(crate) fn remove_body(&mut self, entity: EntityId) -> Option<PhysicsHandles> {
        let handles = self.entities.remove(&entity)?;
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

    pub(crate) fn insert_handles(
        &mut self,
        entity: EntityId,
        handles: PhysicsHandles,
    ) -> Option<PhysicsHandles> {
        self.entities.insert(entity, handles)
    }

    pub(crate) fn remove_handles(&mut self, entity: EntityId) -> Option<PhysicsHandles> {
        self.entities.remove(&entity)
    }

    pub(crate) fn handles(&self, entity: EntityId) -> Option<PhysicsHandles> {
        self.entities.get(&entity).copied()
    }

    pub(crate) fn entity_count(&self) -> usize {
        self.entities.len()
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
    let builder = match body.kind {
        BodyKind::Dynamic => RigidBodyBuilder::dynamic(),
        BodyKind::Static => RigidBodyBuilder::fixed(),
        BodyKind::KinematicPositionBased => RigidBodyBuilder::kinematic_position_based(),
    };
    builder.pose(transform_pose(transform))
}

fn collider_builder(collider: Collider) -> ColliderBuilder {
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

    builder
        .mass(collider.mass)
        .friction(collider.material.friction)
        .restitution(collider.material.restitution)
        .sensor(collider.sensor)
        .active_events(ActiveEvents::COLLISION_EVENTS | ActiveEvents::CONTACT_FORCE_EVENTS)
}

fn transform_pose(transform: Transform) -> Pose {
    let t = transform.translation;
    let r = transform.rotation;
    let rotation =
        nalgebra::UnitQuaternion::new_normalize(nalgebra::Quaternion::new(r.w, r.x, r.y, r.z));

    nalgebra::Isometry3::from_parts(nalgebra::Translation3::new(t.x, t.y, t.z), rotation).into()
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

        let handles =
            physics.materialize_body(entity, transform, RigidBody::dynamic(), collider);

        assert_eq!(physics.handles(entity), Some(handles));
        assert_eq!(physics.entity_count(), 1);
        assert_eq!(physics.bodies.len(), 1);
        assert_eq!(physics.colliders.len(), 1);

        let body = &physics.bodies[handles.body];
        assert_eq!(body.user_data, PhysicsWorld::entity_user_data(entity));
        assert_eq!(body.translation(), vector![1.0, 2.0, 3.0].into());

        let collider = &physics.colliders[handles.collider];
        assert_eq!(collider.user_data, PhysicsWorld::entity_user_data(entity));
        assert!(collider
            .active_events()
            .contains(ActiveEvents::COLLISION_EVENTS));
        assert!(collider
            .active_events()
            .contains(ActiveEvents::CONTACT_FORCE_EVENTS));
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
    }
}
