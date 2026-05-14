//! `QueryIter` — drives [`QueryData`] iteration over a `World`,
//! gated by a [`QueryFilter`] predicate.
//!
//! ## How the layers compose
//!
//! - [`QueryData::access`] declares the required `ComponentId` set.
//! - [`QueryFilter::access`] declares the *read* component access
//!   the filter performs. Both feed the alias check together — a
//!   filter that reads `T` blocks a data write of `T` in the same
//!   query.
//! - [`Join`] picks the smallest required storage and yields its
//!   entity ids in dense order, owning no live storage references.
//! - [`build_fetch`] runs the alias check (data ∪ filter access)
//!   and resolves typed storage handles via the audited unsafe
//!   split-borrow.
//! - For each yielded entity:
//!   1. `F::matches(entity)` — skip if false.
//!   2. `D::fetch(entity)` — `?`-chain skip if some required
//!      component happens to be missing for this entity.
//!
//! ## Why we hold `&'w mut World`
//!
//! `Query<&mut T>` needs an exclusive storage handle, which the
//! audited split-borrow only produces from `&mut World`. The borrow
//! is held for `'w` — the iteration's lifetime. The filter's
//! `matches` reborrows `&world` per call; that's safe because the
//! mutable handle inside `Fetch` is over a *different* component
//! storage (the alias check guarantees it).

use smallvec::SmallVec;

use crate::ecs::ComponentId;
use crate::ecs::World;
use crate::ecs::query::data::{ACCESS_INLINE, QueryData, build_fetch_with_filter};
use crate::ecs::query::filter::QueryFilter;
use crate::ecs::query::join::Join;

/// Iterator returned by
/// [`World::query_filtered`](crate::ecs::World::query_filtered).
///
/// `F` defaults to `()` (no-op filter), so the unfiltered call
/// `world.query::<D>()` returns `QueryIter<'_, D, ()>`. Both fetches
/// hold typed handles tied to `'w`; no `World` reborrow during
/// iteration.
pub struct QueryIter<'w, D: QueryData, F: QueryFilter = ()> {
    fetch: Option<(D::Fetch<'w>, F::Fetch<'w>)>,
    driver: Join,
}

impl<'w, D: QueryData, F: QueryFilter> QueryIter<'w, D, F> {
    pub(crate) fn new(world: &'w mut World, state: D::State, filter_state: F::State) -> Self {
        let data_access = D::access(&state);
        let required: SmallVec<[ComponentId; ACCESS_INLINE]> = data_access
            .components
            .iter()
            .map(|c| c.component_id)
            .collect();
        let driver = Join::new(world, &required);
        let fetch = build_fetch_with_filter::<D, F>(&state, &filter_state, world);
        Self { fetch, driver }
    }
}

impl<'w, D: QueryData, F: QueryFilter> Iterator for QueryIter<'w, D, F> {
    type Item = D::Item<'w>;

    fn next(&mut self) -> Option<Self::Item> {
        let (data_fetch, filter_fetch) = self.fetch.as_mut()?;
        for entity in self.driver.by_ref() {
            if !F::matches(filter_fetch, entity) {
                continue;
            }
            if let Some(item) = D::fetch(data_fetch, entity) {
                return Some(item);
            }
        }
        None
    }
}
