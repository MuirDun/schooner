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

use crate::ecs::World;
use crate::physics::PhysicsWorld;
use crate::time::Time;

/// Advance the physics simulation one fixed step and reconcile it with
/// the ECS. For now it only syncs the timestep and steps an empty world;
/// body lifecycle, transform sync, pose write-back, and event draining
/// land in the following steps.
pub(crate) fn physics_bridge(world: &mut World) {
    // Read the fixed timestep first (Copy out, releasing the borrow) so
    // the solver always integrates by the slice the schedule paces
    // FixedUpdate at — robust to `with_fixed_hz` regardless of order.
    let Some(dt) = world.resource::<Time>().map(|t| t.fixed_delta) else {
        return;
    };
    let Some(physics) = world.resource_mut::<PhysicsWorld>() else {
        return;
    };
    physics.integration_parameters.dt = dt;
    physics.step();
}
