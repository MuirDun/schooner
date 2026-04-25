//! Engine-wide error type.
//!
//! Central home for recoverable and programmer-error conditions the
//! engine can encounter at runtime. The engine stays debuggable and
//! deterministic by funnelling *all* named failure modes through one
//! type — every panic message in the engine is constructed via
//! [`EngineError`]'s `Display`, and every fallible API returns
//! [`EngineResult`].
//!
//! New error cases expand this enum rather than inventing ad-hoc
//! [`std::fmt`] strings in call sites; that keeps error-path behavior
//! auditable as the engine grows.
//!
//! ## When to use a variant vs `Option`
//!
//! - `Option` — "does this exist?" where absence is normal control
//!   flow (e.g. `World::get::<T>(entity)` on an entity that simply
//!   has no `T`).
//! - `EngineError` — something the engine cannot silently paper over.
//!   Split into two shapes: **programmer bug** (aliasing violations,
//!   unregistered systems) and **runtime failure** (surface lost,
//!   shader compile error).

use crate::ecs::EntityId;

/// All named failure modes the engine surfaces.
///
/// Variants are added — never renamed, never silently repurposed —
/// so log output and panic messages remain grep-able across versions.
#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    /// A system requested a resource via [`crate::ecs::Res`] /
    /// [`crate::ecs::ResMut`] before anything inserted it into the
    /// world. Programmer error: insert the resource during startup.
    #[error(
        "resource `{name}` not found in world. Insert it with \
         `world.insert_resource(..)` before running the system."
    )]
    MissingResource { name: &'static str },

    /// A system declared the same resource type twice in its
    /// parameter list (even as `Res` + `ResMut`). The disjoint-fetch
    /// machinery requires distinct `TypeId` keys. Programmer error:
    /// read the resource once and reuse the binding.
    #[error(
        "system parameter conflict: resource `{name}` requested more \
         than once (access modes: {first_mode}, {second_mode}). Each \
         resource type may appear at most once per system."
    )]
    DuplicateSystemParam {
        name: &'static str,
        first_mode: &'static str,
        second_mode: &'static str,
    },

    /// An `EntityId` was used after the slot it pointed at had been
    /// recycled (generation bumped). Programmer error or logic bug:
    /// hold onto `EntityId`s only as long as the entity is alive.
    #[error("stale entity handle: {entity:?} (generation mismatch)")]
    StaleEntity { entity: EntityId },

    /// A `Query` declared the same component type more than once with
    /// at least one `&mut` access. Aliasing `&mut T` (or mixing `&T`
    /// and `&mut T`) over the same storage is unsound; we reject the
    /// query at construction rather than producing aliased borrows.
    /// Programmer error: use a single access mode per component type
    /// inside one query.
    #[error(
        "query alias conflict: component `{name}` requested more than \
         once with at least one mutable access. Each component type \
         may appear at most once per query, and never as both `&T` \
         and `&mut T`."
    )]
    QueryAliasConflict { name: &'static str },
}

/// Convenience alias used by fallible engine APIs.
pub type EngineResult<T> = Result<T, EngineError>;
