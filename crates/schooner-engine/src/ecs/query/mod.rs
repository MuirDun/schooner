//! Query surface: declarative, statically typed access to components
//! inside systems.
//!
//! ## Layering
//!
//! - [`data`] — `QueryData`. Describes *what* a query reads/writes.
//! - [`filter`] — `QueryFilter`. Per-entity skip predicate (`Without<T>`,
//!   eventually `With<T>` / `Changed<T>`). `()` is the no-op filter.
//! - [`fetch`] — the audited unsafe split-borrow that resolves the
//!   world's type-erased storages into typed handles.
//! - [`join`] — driver selection: pick the smallest required storage
//!   and hand its entity ids to the iterator.
//! - [`iter`] — `QueryIter<'w, D, F>` that wires it all together.
//!
//! ## Why `ComponentId` lives in the access description
//!
//! `game0-plan.md` §3.3 mandates that the join code not bake
//! static-tuple-only assumptions. Both `QueryData::access` and
//! `QueryFilter::access` return `Vec<ComponentAccess>` so shik's
//! eventual `world.query_dyn(&[ids])` reuses the same machinery —
//! only the user-facing surface changes.

pub mod data;
pub mod fetch;
pub mod filter;
pub mod iter;
pub mod join;
pub mod param;

pub use data::{ComponentAccess, QueryAccess, QueryData, WriteOnly};
pub use filter::{QueryFilter, Without};
pub use iter::QueryIter;
pub use param::Query;
