//! `QueryFilter` — predicate layer over [`QueryData`] iteration.
//!
//! Filters declare *read-only* component access (presence checks
//! only) and a per-entity `matches` predicate. They cannot return
//! data — that's `QueryData`'s job.
//!
//! ## Why filters live in their own trait
//!
//! Every filter ever (`With<T>`, `Without<T>`, `Changed<T>`,
//! `Added<T>`) wants the same shape: state-resolved-from-world plus
//! a per-entity boolean. Bundling them into `QueryData` would force
//! every data tuple to know about every filter combinator, blowing
//! up the impl matrix. Keeping the two traits separate means
//! `Query<D, F>` is composable: any `D` + any `F`.
//!
//! ## Why filters get a `Fetch<'w>` (not a `&World` reborrow)
//!
//! Earlier draft had `matches` take `&World` and re-resolve storages
//! per entity. That sounds simpler but conflicts with the typed
//! `&mut SparseSet<T>` handles in `QueryData::Fetch` at the
//! HashMap-borrow level — a `&self` reborrow of `World.storages`
//! aliases the raw-pointer-derived `&mut` to one of its values
//! under Stacked Borrows. The clean fix is to feed filters through
//! the same [`split_storages`](super::fetch::split_storages) that
//! data access goes through: `Fetch<'w>` holds typed `&'w` (read)
//! handles to the filter's storages, and the alias check guarantees
//! no overlap with data writes. Per-entity `matches` then runs
//! through pre-resolved typed handles, no `World` reborrow.

use std::marker::PhantomData;

use crate::ecs::query::data::{ComponentAccess, QueryAccess};
use crate::ecs::query::fetch::{handle_as_read, StorageHandle};
use crate::ecs::{Component, ComponentId, EntityId, SparseSet};

/// Skip-or-include predicate over query iteration.
pub trait QueryFilter {
    type State;
    type Fetch<'w>;

    /// One-time setup. Auto-registers any component types the
    /// filter probes.
    fn init_state(world: &mut crate::ecs::World) -> Self::State;

    /// Read-only component access. Feeds the alias check alongside
    /// `QueryData::access` — filters that read `T` block any data
    /// write of `T` in the same query.
    fn access(state: &Self::State) -> QueryAccess;

    /// Build the typed-handle bundle from the pre-split iterator.
    /// Each impl pulls exactly `access(state).components.len()`
    /// handles in declaration order. Each yielded item is
    /// `Option<StorageHandle>` — `None` means the requested storage
    /// has not been registered; filters like `Without<T>` interpret
    /// that as "no entity has `T`".
    fn init_fetch<'w, I>(state: &Self::State, handles: &mut I) -> Self::Fetch<'w>
    where
        I: Iterator<Item = Option<StorageHandle<'w>>>;

    /// Per-entity decision. `true` includes; `false` skips.
    fn matches<'w>(fetch: &Self::Fetch<'w>, entity: EntityId) -> bool;
}

// --- () : the no-op filter -----------------------------------------------

impl QueryFilter for () {
    type State = ();
    type Fetch<'w> = ();

    fn init_state(_world: &mut crate::ecs::World) -> Self::State {}

    fn access(_state: &Self::State) -> QueryAccess {
        QueryAccess::new()
    }

    fn init_fetch<'w, I>(_state: &Self::State, _handles: &mut I) -> Self::Fetch<'w>
    where
        I: Iterator<Item = Option<StorageHandle<'w>>>,
    {
    }

    fn matches<'w>(_fetch: &Self::Fetch<'w>, _entity: EntityId) -> bool {
        true
    }
}

// --- Without<T> ----------------------------------------------------------

/// Filter that excludes entities carrying `T`.
///
/// Auto-registers `T` so the id is stable across the filter's life.
/// The fetch holds an `Option<&'w SparseSet<T>>` — `None` when the
/// component was registered but never inserted (storage missing),
/// in which case every entity passes (no entity has `T`).
pub struct Without<T: Component>(PhantomData<fn() -> T>);

/// Resolved fetch for [`Without<T>`]: `None` when the storage doesn't
/// exist (no entity has `T`, so every entity passes); `Some(&set)`
/// when the storage is live and we probe `contains`.
pub struct WithoutFetch<'w, T: Component> {
    storage: Option<&'w SparseSet<T>>,
}

impl<T: Component> QueryFilter for Without<T> {
    type State = Option<ComponentId>;
    type Fetch<'w> = WithoutFetch<'w, T>;

    fn init_state(world: &mut crate::ecs::World) -> Self::State {
        Some(world.register_component::<T>())
    }

    fn access(state: &Self::State) -> QueryAccess {
        let mut access = QueryAccess::new();
        if let Some(id) = *state {
            access.push(ComponentAccess {
                component_id: id,
                mutable: false,
            });
        }
        access
    }

    fn init_fetch<'w, I>(_state: &Self::State, handles: &mut I) -> Self::Fetch<'w>
    where
        I: Iterator<Item = Option<StorageHandle<'w>>>,
    {
        // Each filter access slot yields an `Option<StorageHandle>`:
        // `Some(h)` when the storage exists, `None` when the
        // component type was registered but never inserted. For
        // `Without<T>` the latter means no entity has `T`, so the
        // filter accepts everyone — represented as `storage: None`.
        let slot = handles
            .next()
            .expect("filter init_fetch ran out of slots (framework bug)");
        WithoutFetch {
            storage: slot.map(handle_as_read::<T>),
        }
    }

    fn matches<'w>(fetch: &Self::Fetch<'w>, entity: EntityId) -> bool {
        match fetch.storage {
            Some(s) => !s.contains(entity),
            None => true, // No storage → no entity has T → all pass.
        }
    }
}

// --- (F1, F2) : two-filter AND ------------------------------------------

impl<F1, F2> QueryFilter for (F1, F2)
where
    F1: QueryFilter,
    F2: QueryFilter,
{
    type State = (F1::State, F2::State);
    type Fetch<'w> = (F1::Fetch<'w>, F2::Fetch<'w>);

    fn init_state(world: &mut crate::ecs::World) -> Self::State {
        let s1 = F1::init_state(world);
        let s2 = F2::init_state(world);
        (s1, s2)
    }

    fn access(state: &Self::State) -> QueryAccess {
        let mut access = F1::access(&state.0);
        for c in F2::access(&state.1).components {
            access.push(c);
        }
        access
    }

    fn init_fetch<'w, I>(state: &Self::State, handles: &mut I) -> Self::Fetch<'w>
    where
        I: Iterator<Item = Option<StorageHandle<'w>>>,
    {
        let f1 = F1::init_fetch(&state.0, handles);
        let f2 = F2::init_fetch(&state.1, handles);
        (f1, f2)
    }

    fn matches<'w>(fetch: &Self::Fetch<'w>, entity: EntityId) -> bool {
        F1::matches(&fetch.0, entity) && F2::matches(&fetch.1, entity)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ecs::World;

    #[derive(Debug, PartialEq)]
    struct Pos(i32);

    #[derive(Debug, PartialEq)]
    struct Tag;

    #[derive(Debug, PartialEq)]
    struct Hidden;

    #[test]
    fn unit_filter_access_is_empty() {
        let mut world = World::new();
        let state = <() as QueryFilter>::init_state(&mut world);
        let access = <() as QueryFilter>::access(&state);
        assert!(access.components.is_empty());
    }

    #[test]
    fn without_access_is_a_single_read() {
        let mut world = World::new();
        let state = <Without<Tag> as QueryFilter>::init_state(&mut world);
        let access = <Without<Tag> as QueryFilter>::access(&state);
        assert_eq!(access.components.len(), 1);
        assert!(!access.components[0].mutable);
    }

    #[test]
    fn tuple_filter_access_concatenates() {
        let mut world = World::new();
        let state = <(Without<Tag>, Without<Hidden>) as QueryFilter>::init_state(&mut world);
        let access = <(Without<Tag>, Without<Hidden>) as QueryFilter>::access(&state);
        assert_eq!(access.components.len(), 2);
        assert!(access.components.iter().all(|c| !c.mutable));
        let mut ids: Vec<_> = access.components.iter().map(|c| c.component_id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), 2);
    }

    #[test]
    fn pos_query_with_without_tag_excludes_tagged_entities() {
        let mut world = World::new();
        let a = world.spawn();
        let b = world.spawn();
        let c = world.spawn();
        world.insert(a, Pos(1));
        world.insert(b, Pos(2));
        world.insert(c, Pos(3));
        world.insert(b, Tag);
        let mut got: Vec<i32> = world
            .query_filtered::<&Pos, Without<Tag>>()
            .map(|p| p.0)
            .collect();
        got.sort();
        assert_eq!(got, vec![1, 3]);
    }

    #[test]
    fn pos_query_with_without_unregistered_tag_passes_all() {
        let mut world = World::new();
        let a = world.spawn();
        let b = world.spawn();
        world.insert(a, Pos(1));
        world.insert(b, Pos(2));
        // Tag never inserted — every entity passes Without<Tag>.
        let mut got: Vec<i32> = world
            .query_filtered::<&Pos, Without<Tag>>()
            .map(|p| p.0)
            .collect();
        got.sort();
        assert_eq!(got, vec![1, 2]);
    }

    #[test]
    fn tuple_filter_excludes_either_side() {
        let mut world = World::new();
        let bare = world.spawn();
        let only_t = world.spawn();
        let only_h = world.spawn();
        let both = world.spawn();
        for e in [bare, only_t, only_h, both] {
            world.insert(e, Pos(0));
        }
        world.insert(only_t, Tag);
        world.insert(only_h, Hidden);
        world.insert(both, Tag);
        world.insert(both, Hidden);

        let got_count = world
            .query_filtered::<&Pos, (Without<Tag>, Without<Hidden>)>()
            .count();
        // Only `bare` survives both filters.
        assert_eq!(got_count, 1);
    }

    #[test]
    #[should_panic(expected = "alias conflict")]
    fn data_write_aliases_filter_read_panics() {
        let mut world = World::new();
        let e = world.spawn();
        world.insert(e, Pos(0));
        world.insert(e, Tag);
        // `WriteOnly<Tag>` + `Without<Tag>` — same component touched
        // for write (data) and read (filter). Alias check rejects.
        let _ = world
            .query_filtered::<crate::ecs::WriteOnly<Tag>, Without<Tag>>()
            .count();
    }
}
