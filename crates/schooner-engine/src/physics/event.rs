//! The Tier-2 events the physics bridge publishes.
//!
//! Collisions and sensor overlaps are *instants with a payload* — the
//! event half of the state-versus-event rule (`plans/overview/events.md`).
//! The bridge drains Rapier's per-step output into these queues; gameplay
//! reacts by polling `Res<Events<Contact>>` / `Res<Events<TriggerEnter>>`,
//! never by registering a callback. These are the engine's first Tier-2
//! cross-layer channel.

use glam::Vec3;

use crate::ecs::EntityId;

/// Two physical bodies touched. The payload is the **impulse** the solver
/// applied to resolve the contact (newton-seconds) — mass × Δvelocity
/// already integrated, so a heavy mass arriving fast reads large and a
/// pebble reads small with no per-mass special-casing. This is what the
/// breakable wall (impulse past a threshold) and fall damage both read.
#[derive(Debug, Clone, Copy)]
pub struct Contact {
    pub a: EntityId,
    pub b: EntityId,
    pub impulse: f32,
    pub normal: Vec3,
}

/// Something entered a sensor volume. A sensor reports overlaps without a
/// physical response, so this is the substrate for pressure plates and
/// pickups (Part 2.H). `sensor` is the trigger entity; `other` is what
/// entered it.
#[derive(Debug, Clone, Copy)]
pub struct TriggerEnter {
    pub sensor: EntityId,
    pub other: EntityId,
}
