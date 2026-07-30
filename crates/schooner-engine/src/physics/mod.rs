//! Physics — Rapier integration and the bridge to the ECS.
//!
//! Physics is hosted, not hand-rolled: the engine embeds Rapier and the
//! work here is the **bridge** that keeps Rapier's world and the ECS's
//! world coherent each fixed step, without either becoming the other's
//! puppet. Games opt in with [`App::with_physics`](crate::App::with_physics);
//! they author bodies through plain components and react to collisions by
//! polling the Tier-2 event queues. The Rapier handles, the solver arena,
//! and the acceleration structures stay the bridge's private business.
//!
//! The idea-level design is `plans/architecture/physics.md`; the as-built
//! state and Part-2 roadmap are `plans/overview/physics.md`.

mod bridge;
mod command;
mod component;
mod diagnostics;
mod event;
mod world;

pub(crate) use bridge::{physics_bridge, physics_reconcile_lifecycle};
pub(crate) use command::PhysicsCommand;
pub use command::{PhysicsCommands, TeleportVelocity};
pub use component::{
    BodyKind, CharacterController, CharacterControllerState, CharacterLength, Collider,
    ColliderShape, ContactEvents, PhysicsMaterial, RigidBody,
};
pub(crate) use diagnostics::reset_physics_diagnostics;
pub use diagnostics::{
    PhysicsCharacterWorkload, PhysicsCommandWorkload, PhysicsDiagnostics, PhysicsEventWorkload,
    PhysicsLifecycleWorkload, PhysicsSolveWorkload, PhysicsTransformSyncWorkload,
    PhysicsWritebackWorkload,
};
pub use event::{Contact, TriggerEnter, TriggerExit};
pub(crate) use world::{CharacterMovement, PhysicsStepOutput, PhysicsWorld};
