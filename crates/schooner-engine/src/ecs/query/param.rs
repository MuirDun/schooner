//! `Query<D, F>` — the user-facing system parameter.
//!
//! `Query` is a thin wrapper around [`QueryIter`]: it lets us add
//! ergonomic methods (`single`, `get`, `len`, ...) over time without
//! breaking existing call sites. For Game 0 it just `IntoIterator`s
//! into the underlying iterator.
//!
//! ## How it plugs into the system machinery
//!
//! `Query<D, F>` implements [`SystemParam`]. `access(world)`
//! resolves the (data ∪ filter) component access via
//! `D::init_state` + `F::init_state`. `fetch(world, last_run_epoch)`
//! builds the query with the owning function system's cursor and wraps it.
//!
//! ## Why this lives in `query/` (not `system.rs`)
//!
//! It depends on [`QueryData`], [`QueryFilter`], and the join/fetch
//! engines — keeping it next to them avoids a circular module
//! relationship.

use crate::ecs::World;
use crate::ecs::query::data::QueryData;
use crate::ecs::query::filter::QueryFilter;
use crate::ecs::query::iter::QueryIter;
use crate::ecs::system::{ParamAccess, SystemParam};

/// User-facing wrapper around [`QueryIter`].
///
/// Iterate via `for item in query { ... }` — the `IntoIterator` impl
/// hands back the underlying [`QueryIter`].
pub struct Query<'w, D: QueryData, F: QueryFilter = ()> {
    iter: QueryIter<'w, D, F>,
}

impl<'w, D: QueryData, F: QueryFilter> Query<'w, D, F> {
    pub(crate) fn new(iter: QueryIter<'w, D, F>) -> Self {
        Self { iter }
    }
}

impl<'w, D: QueryData, F: QueryFilter> IntoIterator for Query<'w, D, F> {
    type Item = D::Item<'w>;
    type IntoIter = QueryIter<'w, D, F>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter
    }
}

// --- SystemParam impl ----------------------------------------------------

impl<D, F> SystemParam for Query<'_, D, F>
where
    D: QueryData + 'static,
    F: QueryFilter + 'static,
{
    type Item<'w> = Query<'w, D, F>;

    fn access(world: &mut World) -> ParamAccess {
        // Materialise the resolved state so we can ask each layer
        // for its access set. The state is rebuilt per call (cheap
        // — registry lookups), which is fine at Game 0 scale; if it
        // ever matters, the FunctionSystem could cache it.
        let data_state = D::init_state(world);
        let filter_state = F::init_state(world);
        let mut access = ParamAccess::new();
        for c in D::access(&data_state).components {
            access.components.push(c);
        }
        for c in F::access(&filter_state).components {
            access.components.push(c);
        }
        access
    }

    unsafe fn fetch<'w>(world: &'w mut World, last_run_epoch: Option<u64>) -> Self::Item<'w> {
        // SAFETY: the SystemParam contract guarantees the caller has
        // verified non-conflict. `query_filtered_for_system` consumes
        // `&mut world` and returns a `QueryIter<'_, D, F>` whose
        // typed handles are tied to that borrow. Wrapping in
        // `Query<'w, D, F>` carries the lifetime through.
        let iter = world.query_filtered_for_system::<D, F>(last_run_epoch);
        Query::new(iter)
    }
}
