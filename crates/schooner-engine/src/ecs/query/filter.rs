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
use crate::ecs::query::fetch::{StorageHandle, handle_as_read};
use crate::ecs::{Component, ComponentId, EntityId, SparseSet};

/// Skip-or-include predicate over query iteration.
pub trait QueryFilter {
    type State;
    type Fetch<'w>;

    /// One-time setup. Auto-registers any component types the
    /// filter probes.
    fn init_state(world: &mut crate::ecs::World) -> Self::State;

    /// Cursor-aware setup for the change-detection filters
    /// (`Added<T>` / `Changed<T>`), threaded in by
    /// [`World::query_filtered_since`](crate::ecs::World::query_filtered_since).
    /// Presence filters (`()`, `Without<T>`) ignore the cursor, so the
    /// default just forwards to [`init_state`](Self::init_state).
    fn init_state_since(world: &mut crate::ecs::World, _since: u64) -> Self::State {
        Self::init_state(world)
    }

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

/// Marker for filters that are meaningful without a consumer cursor.
///
/// [`World::query_filtered`](crate::ecs::World::query_filtered) accepts
/// only these filters. Reactive filters such as [`Added`] and [`Changed`]
/// deliberately do not implement this trait: scheduled [`Query`](crate::ecs::Query)
/// parameters receive their owning system's cursor, while manual callers must
/// make cursor ownership explicit through
/// [`World::query_filtered_since`](crate::ecs::World::query_filtered_since).
pub trait CursorlessQueryFilter: QueryFilter {}

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

impl CursorlessQueryFilter for () {}

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

impl<T: Component> CursorlessQueryFilter for Without<T> {}

// --- Added<T> ------------------------------------------------------------

/// Filter that keeps entities whose `T` is new to the owning consumer.
///
/// A scheduled `Query<_, Added<T>>` receives the system's last successful-run
/// epoch. Its first execution has no prior cursor and therefore observes every
/// currently matching `T`, including components inserted at epoch zero.
/// Subsequent executions use strict `added_tick > since` comparison.
///
/// Manual world queries must supply their caller-owned cursor through
/// [`World::query_filtered_since`](crate::ecs::World::query_filtered_since).
pub struct Added<T: Component>(PhantomData<fn() -> T>);

/// Resolved fetch for [`Added<T>`]: the typed read handle (or `None`
/// when the storage doesn't exist — then nothing was added, so no
/// entity passes) plus the since-cursor to compare add ticks against.
pub struct AddedFetch<'w, T: Component> {
    storage: Option<&'w SparseSet<T>>,
    since: Option<u64>,
}

impl<T: Component> QueryFilter for Added<T> {
    type State = (Option<ComponentId>, Option<u64>);
    type Fetch<'w> = AddedFetch<'w, T>;

    fn init_state(world: &mut crate::ecs::World) -> Self::State {
        (Some(world.register_component::<T>()), None)
    }

    fn init_state_since(world: &mut crate::ecs::World, since: u64) -> Self::State {
        (Some(world.register_component::<T>()), Some(since))
    }

    fn access(state: &Self::State) -> QueryAccess {
        let mut access = QueryAccess::new();
        if let Some(id) = state.0 {
            access.push(ComponentAccess {
                component_id: id,
                mutable: false,
            });
        }
        access
    }

    fn init_fetch<'w, I>(state: &Self::State, handles: &mut I) -> Self::Fetch<'w>
    where
        I: Iterator<Item = Option<StorageHandle<'w>>>,
    {
        let slot = handles
            .next()
            .expect("filter init_fetch ran out of slots (framework bug)");
        AddedFetch {
            storage: slot.map(handle_as_read::<T>),
            since: state.1,
        }
    }

    fn matches<'w>(fetch: &Self::Fetch<'w>, entity: EntityId) -> bool {
        match fetch.storage {
            // No `T` for this entity → not added; missing storage →
            // nothing was added at all. Both exclude.
            Some(s) => s
                .ticks(entity)
                .is_some_and(|t| fetch.since.is_none_or(|since| t.added_tick > since)),
            None => false,
        }
    }
}

// --- Changed<T> ----------------------------------------------------------

/// Filter that keeps only entities whose `T` was mutated (through the
/// tick-bumping path) after the query's since-cursor
/// (`last_mutation_tick > since`).
///
/// A fresh insert stamps the mutation tick too, so `Changed<T>` also
/// matches newly-added entities — "an add is a change," the standard
/// convention. Use [`Added<T>`] when you want *only* adds.
///
/// The cursor is supplied through
/// [`World::query_filtered_since`](crate::ecs::World::query_filtered_since),
/// or by the scheduler for a `Query<_, Changed<T>>` system parameter.
/// See [`Added<T>`] for first-run and subsequent-run cursor rules.
pub struct Changed<T: Component>(PhantomData<fn() -> T>);

/// Resolved fetch for [`Changed<T>`]: the typed read handle (or `None`
/// when the storage doesn't exist — then nothing changed, so no entity
/// passes) plus the since-cursor to compare mutation ticks against.
pub struct ChangedFetch<'w, T: Component> {
    storage: Option<&'w SparseSet<T>>,
    since: Option<u64>,
}

impl<T: Component> QueryFilter for Changed<T> {
    type State = (Option<ComponentId>, Option<u64>);
    type Fetch<'w> = ChangedFetch<'w, T>;

    fn init_state(world: &mut crate::ecs::World) -> Self::State {
        (Some(world.register_component::<T>()), None)
    }

    fn init_state_since(world: &mut crate::ecs::World, since: u64) -> Self::State {
        (Some(world.register_component::<T>()), Some(since))
    }

    fn access(state: &Self::State) -> QueryAccess {
        let mut access = QueryAccess::new();
        if let Some(id) = state.0 {
            access.push(ComponentAccess {
                component_id: id,
                mutable: false,
            });
        }
        access
    }

    fn init_fetch<'w, I>(state: &Self::State, handles: &mut I) -> Self::Fetch<'w>
    where
        I: Iterator<Item = Option<StorageHandle<'w>>>,
    {
        let slot = handles
            .next()
            .expect("filter init_fetch ran out of slots (framework bug)");
        ChangedFetch {
            storage: slot.map(handle_as_read::<T>),
            since: state.1,
        }
    }

    fn matches<'w>(fetch: &Self::Fetch<'w>, entity: EntityId) -> bool {
        match fetch.storage {
            Some(s) => s
                .ticks(entity)
                .is_some_and(|t| fetch.since.is_none_or(|since| t.last_mutation_tick > since)),
            None => false,
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

    fn init_state_since(world: &mut crate::ecs::World, since: u64) -> Self::State {
        let s1 = F1::init_state_since(world, since);
        let s2 = F2::init_state_since(world, since);
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

impl<F1, F2> CursorlessQueryFilter for (F1, F2)
where
    F1: CursorlessQueryFilter,
    F2: CursorlessQueryFilter,
{
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
        <() as QueryFilter>::init_state(&mut world);
        let access = <() as QueryFilter>::access(&());
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
    fn added_filter_selects_only_entities_added_after_cursor() {
        let mut world = World::new();
        let early = world.spawn();
        world.insert(early, Pos(1)); // added_tick = 0
        world.increment_tick(); // tick 1
        let cursor = 0; // "since tick 0"
        let late = world.spawn();
        world.insert(late, Pos(2)); // added_tick = 1 > 0

        // `&Pos` data + `Added<Pos>` filter both read Pos — two reads,
        // no alias conflict. Only `late` was added after the cursor.
        let got: Vec<i32> = world
            .query_filtered_since::<&Pos, Added<Pos>>(cursor)
            .map(|p| p.0)
            .collect();
        assert_eq!(got, vec![2]);
    }

    #[test]
    fn added_filter_excludes_everyone_when_storage_absent() {
        let mut world = World::new();
        let e = world.spawn();
        world.insert(e, Pos(1));
        // Tag never inserted: Added<Tag> sees no storage → nobody passes.
        let got = world.query_filtered_since::<&Pos, Added<Tag>>(0).count();
        assert_eq!(got, 0);
    }

    #[test]
    fn changed_filter_selects_only_entities_mutated_after_cursor() {
        let mut world = World::new();
        let a = world.spawn();
        let b = world.spawn();
        world.insert(a, Pos(1)); // mutation tick 0
        world.insert(b, Pos(2)); // mutation tick 0
        world.increment_tick(); // tick 1
        {
            let mut p = world.get_mut::<Pos>(b).unwrap();
            p.0 = 20; // last_mutation_tick = 1
        }
        // a was last touched at tick 0 (== cursor, strict > excludes it);
        // b was mutated at tick 1.
        let got: Vec<i32> = world
            .query_filtered_since::<&Pos, Changed<Pos>>(0)
            .map(|p| p.0)
            .collect();
        assert_eq!(got, vec![20]);
    }

    #[test]
    fn changed_filter_includes_fresh_adds() {
        let mut world = World::new();
        world.increment_tick(); // tick 1
        let e = world.spawn();
        world.insert(e, Pos(5)); // added & mutated at tick 1
        // An insert stamps the mutation tick too, so the fresh add
        // reads as changed since tick 0.
        let got: Vec<i32> = world
            .query_filtered_since::<&Pos, Changed<Pos>>(0)
            .map(|p| p.0)
            .collect();
        assert_eq!(got, vec![5]);
    }

    #[test]
    fn explicit_cursor_zero_keeps_strict_caller_owned_semantics() {
        let mut world = World::new();
        let entity = world.spawn();
        world.insert(entity, Pos(1));

        assert_eq!(world.query_filtered_since::<&Pos, Added<Pos>>(0).count(), 0);
        assert_eq!(
            world.query_filtered_since::<&Pos, Changed<Pos>>(0).count(),
            0
        );

        world.increment_tick();
        world.get_mut::<Pos>(entity).unwrap().0 = 2;

        assert_eq!(
            world
                .query_filtered_since::<&Pos, Changed<Pos>>(0)
                .map(|pos| pos.0)
                .collect::<Vec<_>>(),
            vec![2]
        );
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
