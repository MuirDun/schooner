//! ECS benchmark harness — implementation-agnostic.
//!
//! The point: write each scenario *once* against the [`BenchEcs`]
//! trait, then plug in different ECS implementations behind it. Today
//! the only implementation is [`schooner_v1`] (sparse-set ECS as built
//! through C9). When we add an archetype implementation in Game 3+,
//! it lands as a sibling module here and the same scenarios run
//! against it for direct comparison.
//!
//! ## Why a trait, not a macro
//!
//! Static dispatch through generics gives the optimiser the same
//! freedom it would have if each scenario were hand-written against a
//! single impl. The trait is consumed in `benches/ecs_scenarios.rs`
//! via `criterion::bench_function::<E: BenchEcs>(...)` style scoping —
//! see the bench file for the pattern.
//!
//! ## Component shapes used by the scenarios
//!
//! - [`Pos`], [`Vel`] — the canonical "moving entity" pair: 12 + 12
//!   bytes, drives the iterate-and-mutate scenarios.
//! - [`Tag`] — zero-sized marker for filter scenarios.
//! - [`Bulk`] — 256-byte struct used to stress sparse-set's
//!   per-component cache footprint.

pub mod schooner_v1;

/// Canonical components every `BenchEcs` impl must support.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Pos {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Vel {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Tag;

/// Heavyweight component — 256 bytes. Inserts a non-trivial cache
/// footprint per entity for the cache-locality scenarios.
#[derive(Debug, Clone, Copy, Default)]
pub struct Bulk(pub [u64; 32]);

/// Surface every benchable ECS impl provides.
///
/// `World` and `Entity` are associated types so each impl can pick
/// its own concrete representation (generational index, raw u64,
/// archetype + slot, etc.) without leaking into scenario code.
pub trait BenchEcs: 'static {
    type World;
    type Entity: Copy;

    /// Human-readable name for criterion's group / id system.
    fn name() -> &'static str;

    fn new_world() -> Self::World;

    fn spawn(world: &mut Self::World) -> Self::Entity;

    fn insert_pos(world: &mut Self::World, e: Self::Entity, p: Pos);
    fn insert_vel(world: &mut Self::World, e: Self::Entity, v: Vel);
    fn insert_tag(world: &mut Self::World, e: Self::Entity);
    fn insert_bulk(world: &mut Self::World, e: Self::Entity, b: Bulk);

    fn remove_pos(world: &mut Self::World, e: Self::Entity);

    /// Look up a component by entity. The bench expects this to be
    /// the impl's fastest path for a single random-access lookup.
    fn get_pos(world: &Self::World, e: Self::Entity) -> Option<Pos>;

    /// Iterate every `(Pos, Vel)` pair, applying `f`. The closure is
    /// given mutable `Pos` and shared `Vel` — that's the canonical
    /// "physics step" shape and the most-discussed ECS hot path.
    fn iterate_pos_vel(world: &mut Self::World, f: &mut dyn FnMut(&mut Pos, &Vel));

    /// Iterate `Pos` only, skipping entities that carry `Tag`. The
    /// "filter" scenario; exercises the `Without<T>` path.
    fn iterate_pos_without_tag(world: &mut Self::World, f: &mut dyn FnMut(&Pos));

    /// Iterate `Pos` mutably; the closure receives `&mut Pos`.
    /// Distinct from `iterate_pos_vel` because it has no probe — the
    /// driver IS the Pos storage. Used to measure the per-item cost
    /// of `Mut<T>` change-tick bookkeeping.
    fn iterate_pos_mut(world: &mut Self::World, f: &mut dyn FnMut(&mut Pos));
}
