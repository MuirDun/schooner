//! `QueryData` — declarative description of what a query reads or
//! writes, plus the per-entity materialiser.
//!
//! ## Why three associated types
//!
//! The trait splits "what's in flight during iteration" from "what
//! the user sees per entity":
//!
//! - [`QueryData::State`] — resolved-once metadata (component ids,
//!   plus, for mutable fetches, the world's tick at iteration start).
//!   Built by [`init_state`](QueryData::init_state) before any
//!   per-entity work.
//! - [`QueryData::Fetch`] — typed storage handles bound to the world
//!   borrow. Read-only impls hold `&'w SparseSet<T>`; mutable impls
//!   hold `&'w mut SparseSet<T>` + the current tick. Built by
//!   [`init_fetch`](QueryData::init_fetch), which performs the
//!   audited unsafe split-borrow exactly once. Per-entity `fetch`
//!   uses this handle and never re-touches `World`.
//! - [`QueryData::Item`] — what each `next()` yields (`&'w T`,
//!   `Mut<'w, T>`, or a tuple thereof).
//!
//! ## Why `Fetch` is mutable in `fetch()`
//!
//! For `&mut T` we want to call `SparseSet::iter_mut_with_ticks` —
//! taking `&mut`. So `D::fetch` takes `&mut Fetch`. Read-only impls
//! ignore the `mut`. Tuple impls hand each inner a disjoint slice of
//! the `Fetch` tuple via field projection — no aliasing.
//!
//! ## ComponentId in the access description
//!
//! Plan §3.3 mandates: the join code must not bake static-tuple-only
//! assumptions. `QueryAccess` carries a `Vec<ComponentAccess>` even
//! from typed impls so shik's `world.query_dyn(&[ids])` reuses the
//! same join machinery — only the user-facing surface changes.

use smallvec::SmallVec;

use crate::ecs::query::fetch::{
    StorageHandle, check_no_alias, handle_as_read, handle_as_write, name_of_component,
    split_storages,
};
use crate::ecs::world::Mut;
use crate::ecs::{Component, ComponentId, EntityId, SparseSet, World};

/// Inline capacity for [`QueryAccess::components`]. Covers the common
/// queries: 1/2/3-tuple data with up to one filter slot in the
/// combined alias check. Wider queries spill to the heap.
pub(crate) const ACCESS_INLINE: usize = 4;

/// Per-component access descriptor: which component, in which mode.
///
/// Carried alongside `ComponentId` rather than `TypeId` because the
/// join engine wants `O(1)` storage lookup, not a hash on every
/// probe. The `mutable` flag is what the alias check reads to reject
/// `Query<(&mut T, &T)>`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ComponentAccess {
    pub component_id: ComponentId,
    pub mutable: bool,
}

/// The access set of a whole query — every component it touches, in
/// declaration order.
///
/// `components` is a `SmallVec` so the common 1/2/3-tuple queries stay
/// stack-allocated through the per-query setup path (`access` is built
/// fresh on every `world.query::<D>()`). Wider queries fall back to the
/// heap.
#[derive(Clone, Debug, Default)]
pub struct QueryAccess {
    pub components: SmallVec<[ComponentAccess; ACCESS_INLINE]>,
}

impl QueryAccess {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, access: ComponentAccess) {
        self.components.push(access);
    }
}

/// What kind of access pattern a query declares.
///
/// `State` is resolved-once data carried for the whole iteration:
/// `ComponentId`s, plus (for mutable fetches) the tick at iteration
/// start. `Fetch<'w>` is the typed-handle bundle the iterator holds.
/// `Item<'w>` is what the user receives per entity.
pub trait QueryData {
    /// Per-entity yielded item.
    type Item<'w>;

    /// Typed storage handles, bound to the world borrow.
    type Fetch<'w>;

    /// Resolved-once iteration state. Always carries the resolved
    /// `ComponentId`s; mutable impls also stash the tick taken at
    /// query construction.
    type State;

    /// One-time setup. Auto-registers component types so later
    /// `world.insert::<T>` calls share the same id, and stores the
    /// resolved ids for the per-entity fetch path.
    fn init_state(world: &mut World) -> Self::State;

    /// Static description of what this query reads/writes. Read by
    /// the alias check before any unsafe runs.
    fn access(state: &Self::State) -> QueryAccess;

    /// Build the per-iteration fetch from an iterator over the
    /// pre-split storage handles. Each impl pulls (consumes) exactly
    /// `access(state).components.len()` handles in declaration order.
    /// Tuple impls chain inner calls.
    ///
    /// Handles are consumed by value rather than borrowed because
    /// extracting a typed `&'w mut SparseSet<T>` from a borrowed
    /// `&mut StorageHandle::Write(&'w mut dyn ...)` would create
    /// aliased mutable borrows; consuming the wrapper makes the
    /// inner reference's transfer the only live borrow.
    fn init_fetch<'w, I>(state: &Self::State, handles: &mut I) -> Self::Fetch<'w>
    where
        I: Iterator<Item = StorageHandle<'w>>;

    /// Per-entity probe. Returns `None` if `entity` does not have
    /// every component this query requires.
    fn fetch<'w>(fetch: &mut Self::Fetch<'w>, entity: EntityId) -> Option<Self::Item<'w>>;
}

// --- &T ------------------------------------------------------------------

impl<T: Component> QueryData for &T {
    type Item<'w> = &'w T;
    type Fetch<'w> = &'w SparseSet<T>;
    type State = Option<ComponentId>;

    fn init_state(world: &mut World) -> Self::State {
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
        I: Iterator<Item = StorageHandle<'w>>,
    {
        let head = handles
            .next()
            .expect("init_fetch ran out of handles (framework bug)");
        handle_as_read::<T>(head)
    }

    fn fetch<'w>(fetch: &mut Self::Fetch<'w>, entity: EntityId) -> Option<Self::Item<'w>> {
        fetch.get(entity)
    }
}

// --- &mut T --------------------------------------------------------------

/// Wrapper marking the mutable-access path for `T`.
///
/// We can't `impl QueryData for &mut T` directly: a blanket impl
/// over `&T` already exists, and the trait solver needs distinct
/// impl heads to disambiguate. The wrapper is what `Query<&mut T>`
/// will be desugared to in C9.6.
pub struct WriteOnly<T: Component>(std::marker::PhantomData<fn() -> T>);

/// Mutable fetch bundle: typed storage + the iteration's tick.
pub struct WriteFetch<'w, T: Component> {
    storage: &'w mut SparseSet<T>,
    current_tick: u64,
}

impl<T: Component> QueryData for WriteOnly<T> {
    type Item<'w> = Mut<'w, T>;
    type Fetch<'w> = WriteFetch<'w, T>;
    type State = (Option<ComponentId>, u64);

    fn init_state(world: &mut World) -> Self::State {
        let id = world.register_component::<T>();
        (Some(id), world.current_tick())
    }

    fn access(state: &Self::State) -> QueryAccess {
        let mut access = QueryAccess::new();
        if let Some(id) = state.0 {
            access.push(ComponentAccess {
                component_id: id,
                mutable: true,
            });
        }
        access
    }

    fn init_fetch<'w, I>(state: &Self::State, handles: &mut I) -> Self::Fetch<'w>
    where
        I: Iterator<Item = StorageHandle<'w>>,
    {
        let head = handles
            .next()
            .expect("init_fetch ran out of handles (framework bug)");
        WriteFetch {
            storage: handle_as_write::<T>(head),
            current_tick: state.1,
        }
    }

    fn fetch<'w>(fetch: &mut Self::Fetch<'w>, entity: EntityId) -> Option<Self::Item<'w>> {
        // SAFETY: `fetch.storage: &'w mut SparseSet<T>` was produced
        // by the audited split-borrow in `split_storages`. The join
        // engine yields each entity at most once, so successive
        // `fetch` calls for distinct entities ask for disjoint
        // dense slots — the resulting `Mut<T>`s never alias. The
        // raw-pointer detour exists because `&mut self` reborrows
        // shorten the lifetime; we re-extend to `'w` so the
        // returned `Mut` is bound to the iteration, not to the
        // local stack frame of `fetch`.
        let storage_ptr: *mut SparseSet<T> = fetch.storage as *mut SparseSet<T>;
        unsafe {
            let (value, ticks) = (*storage_ptr).get_mut_with_ticks(entity)?;
            Some(Mut::from_raw_parts(value, ticks, fetch.current_tick))
        }
    }
}

// --- (D1, D2) ------------------------------------------------------------

impl<D1, D2> QueryData for (D1, D2)
where
    D1: QueryData,
    D2: QueryData,
{
    type Item<'w> = (D1::Item<'w>, D2::Item<'w>);
    type Fetch<'w> = (D1::Fetch<'w>, D2::Fetch<'w>);
    type State = (D1::State, D2::State);

    fn init_state(world: &mut World) -> Self::State {
        let s1 = D1::init_state(world);
        let s2 = D2::init_state(world);
        (s1, s2)
    }

    fn access(state: &Self::State) -> QueryAccess {
        let mut access = D1::access(&state.0);
        for c in D2::access(&state.1).components {
            access.push(c);
        }
        access
    }

    fn init_fetch<'w, I>(state: &Self::State, handles: &mut I) -> Self::Fetch<'w>
    where
        I: Iterator<Item = StorageHandle<'w>>,
    {
        let f1 = D1::init_fetch(&state.0, handles);
        let f2 = D2::init_fetch(&state.1, handles);
        (f1, f2)
    }

    fn fetch<'w>(fetch: &mut Self::Fetch<'w>, entity: EntityId) -> Option<Self::Item<'w>> {
        let a = D1::fetch(&mut fetch.0, entity)?;
        let b = D2::fetch(&mut fetch.1, entity)?;
        Some((a, b))
    }
}

// --- (D1, D2, D3) --------------------------------------------------------

impl<D1, D2, D3> QueryData for (D1, D2, D3)
where
    D1: QueryData,
    D2: QueryData,
    D3: QueryData,
{
    type Item<'w> = (D1::Item<'w>, D2::Item<'w>, D3::Item<'w>);
    type Fetch<'w> = (D1::Fetch<'w>, D2::Fetch<'w>, D3::Fetch<'w>);
    type State = (D1::State, D2::State, D3::State);

    fn init_state(world: &mut World) -> Self::State {
        let s1 = D1::init_state(world);
        let s2 = D2::init_state(world);
        let s3 = D3::init_state(world);
        (s1, s2, s3)
    }

    fn access(state: &Self::State) -> QueryAccess {
        let mut access = D1::access(&state.0);
        for c in D2::access(&state.1).components {
            access.push(c);
        }
        for c in D3::access(&state.2).components {
            access.push(c);
        }
        access
    }

    fn init_fetch<'w, I>(state: &Self::State, handles: &mut I) -> Self::Fetch<'w>
    where
        I: Iterator<Item = StorageHandle<'w>>,
    {
        let f1 = D1::init_fetch(&state.0, handles);
        let f2 = D2::init_fetch(&state.1, handles);
        let f3 = D3::init_fetch(&state.2, handles);
        (f1, f2, f3)
    }

    fn fetch<'w>(fetch: &mut Self::Fetch<'w>, entity: EntityId) -> Option<Self::Item<'w>> {
        let a = D1::fetch(&mut fetch.0, entity)?;
        let b = D2::fetch(&mut fetch.1, entity)?;
        let c = D3::fetch(&mut fetch.2, entity)?;
        Some((a, b, c))
    }
}

// --- run-the-checks helper -----------------------------------------------

/// Build the (data, filter) `Fetch` pair from a `&'w mut World`.
///
/// Returns `None` if any required *data* storage is missing — the
/// caller iterates empty. Filter slots whose storages are missing
/// pass `None` to the filter's `init_fetch`; filters like
/// `Without<T>` interpret that as "no entity has `T`", so the query
/// still iterates and the filter accepts everyone.
///
/// Panics with a descriptive `EngineError` if the combined access
/// declares aliasing.
pub fn build_fetch_with_filter<'w, D, F>(
    state: &D::State,
    filter_state: &F::State,
    world: &'w mut World,
) -> Option<(D::Fetch<'w>, F::Fetch<'w>)>
where
    D: QueryData,
    F: crate::ecs::query::filter::QueryFilter,
{
    let data_access = D::access(state);
    let filter_access = F::access(filter_state);

    // Combined alias check: a filter that reads `T` must not
    // coexist with a data write of `T` in the same query.
    let mut combined = QueryAccess::new();
    for c in &data_access.components {
        combined.push(*c);
    }
    for c in &filter_access.components {
        combined.push(*c);
    }
    if let Err(err) = check_no_alias(&combined, |c| name_of_component(world, c)) {
        panic!("{err}");
    }

    let (data_handles, filter_handles) = split_storages(world, &data_access, &filter_access)?;

    let mut data_iter = data_handles.into_iter();
    let data_fetch = D::init_fetch(state, &mut data_iter);
    debug_assert!(data_iter.next().is_none(), "data init_fetch left handles");

    let mut filter_iter = filter_handles.into_iter();
    let filter_fetch = F::init_fetch(filter_state, &mut filter_iter);
    debug_assert!(
        filter_iter.next().is_none(),
        "filter init_fetch left handles"
    );

    Some((data_fetch, filter_fetch))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq)]
    struct Pos(i32);

    #[derive(Debug, PartialEq)]
    struct Vel(i32);

    fn collect_pos(world: &mut World) -> Vec<i32> {
        let mut out: Vec<i32> = world.query::<&Pos>().map(|p| p.0).collect();
        out.sort();
        out
    }

    // --- &T --------------------------------------------------------------

    #[test]
    fn query_on_empty_world_yields_nothing() {
        let mut world = World::new();
        let out: Vec<&Pos> = world.query::<&Pos>().collect();
        assert!(out.is_empty());
    }

    #[test]
    fn query_yields_all_entries_for_single_component() {
        let mut world = World::new();
        let a = world.spawn();
        let b = world.spawn();
        let c = world.spawn();
        world.insert(a, Pos(1));
        world.insert(b, Pos(2));
        world.insert(c, Pos(3));
        assert_eq!(collect_pos(&mut world), vec![1, 2, 3]);
    }

    #[test]
    fn query_skips_other_component_types() {
        let mut world = World::new();
        let a = world.spawn();
        let b = world.spawn();
        world.insert(a, Pos(7));
        world.insert(b, Vel(99));
        let out: Vec<i32> = world.query::<&Pos>().map(|p| p.0).collect();
        assert_eq!(out, vec![7]);
    }

    #[test]
    fn query_for_unregistered_type_is_empty_and_does_not_panic() {
        let mut world = World::new();
        let a = world.spawn();
        world.insert(a, Vel(1));
        let out: Vec<&Pos> = world.query::<&Pos>().collect();
        assert!(out.is_empty());
    }

    #[test]
    fn query_after_remove_omits_removed_entity() {
        let mut world = World::new();
        let a = world.spawn();
        let b = world.spawn();
        world.insert(a, Pos(10));
        world.insert(b, Pos(20));
        world.remove::<Pos>(a);
        let out: Vec<i32> = world.query::<&Pos>().map(|p| p.0).collect();
        assert_eq!(out, vec![20]);
    }

    #[test]
    fn query_after_despawn_omits_despawned_entity() {
        let mut world = World::new();
        let a = world.spawn();
        let b = world.spawn();
        world.insert(a, Pos(10));
        world.insert(b, Pos(20));
        world.despawn(a);
        let out: Vec<i32> = world.query::<&Pos>().map(|p| p.0).collect();
        assert_eq!(out, vec![20]);
    }

    #[test]
    fn init_state_registers_component_so_id_is_stable() {
        let mut world = World::new();
        assert_eq!(world.component_id::<Pos>(), None);
        let _ = world.query::<&Pos>().count();
        let id = world.component_id::<Pos>().expect("init_state registered");
        let e = world.spawn();
        world.insert(e, Pos(1));
        assert_eq!(world.component_id::<Pos>(), Some(id));
    }

    #[test]
    fn access_describes_single_read_for_ref_t() {
        let mut world = World::new();
        let state = <&Pos as QueryData>::init_state(&mut world);
        let access = <&Pos as QueryData>::access(&state);
        assert_eq!(access.components.len(), 1);
        assert!(!access.components[0].mutable);
        assert_eq!(Some(access.components[0].component_id), state);
    }

    // --- tuple read-only -------------------------------------------------

    #[derive(Debug, PartialEq)]
    struct C(i32);

    fn collect_pos_vel(world: &mut World) -> Vec<(i32, i32)> {
        let mut out: Vec<(i32, i32)> = world
            .query::<(&Pos, &Vel)>()
            .map(|(p, v)| (p.0, v.0))
            .collect();
        out.sort();
        out
    }

    #[test]
    fn two_tuple_yields_intersection_only() {
        let mut world = World::new();
        let only_pos = world.spawn();
        let both = world.spawn();
        let only_vel = world.spawn();
        world.insert(only_pos, Pos(1));
        world.insert(both, Pos(2));
        world.insert(both, Vel(20));
        world.insert(only_vel, Vel(99));
        assert_eq!(collect_pos_vel(&mut world), vec![(2, 20)]);
    }

    #[test]
    fn two_tuple_empty_when_no_overlap() {
        let mut world = World::new();
        let a = world.spawn();
        let b = world.spawn();
        world.insert(a, Pos(1));
        world.insert(b, Vel(2));
        assert_eq!(collect_pos_vel(&mut world), vec![]);
    }

    #[test]
    fn two_tuple_empty_when_one_side_unregistered() {
        let mut world = World::new();
        let e = world.spawn();
        world.insert(e, Pos(1));
        assert_eq!(collect_pos_vel(&mut world), vec![]);
    }

    #[test]
    fn two_tuple_smallest_drives_iteration() {
        let mut world = World::new();
        let mut commons = Vec::new();
        for i in 0..32 {
            let e = world.spawn();
            world.insert(e, Vel(i));
            commons.push(e);
        }
        let rare = commons[7];
        world.insert(rare, Pos(777));
        let out: Vec<(i32, i32)> = world
            .query::<(&Pos, &Vel)>()
            .map(|(p, v)| (p.0, v.0))
            .collect();
        assert_eq!(out, vec![(777, 7)]);
    }

    #[test]
    fn three_tuple_yields_triple_intersection() {
        let mut world = World::new();
        let pvc = world.spawn();
        let pv = world.spawn();
        let pc = world.spawn();
        let vc = world.spawn();
        for (e, p) in [(pvc, 1), (pv, 2), (pc, 3)] {
            world.insert(e, Pos(p));
        }
        for (e, v) in [(pvc, 10), (pv, 20), (vc, 30)] {
            world.insert(e, Vel(v));
        }
        for (e, c) in [(pvc, 100), (pc, 200), (vc, 300)] {
            world.insert(e, C(c));
        }
        let out: Vec<(i32, i32, i32)> = world
            .query::<(&Pos, &Vel, &C)>()
            .map(|(p, v, c)| (p.0, v.0, c.0))
            .collect();
        assert_eq!(out, vec![(1, 10, 100)]);
    }

    #[test]
    fn three_tuple_access_lists_all_three_components_in_order() {
        let mut world = World::new();
        let state = <(&Pos, &Vel, &C) as QueryData>::init_state(&mut world);
        let access = <(&Pos, &Vel, &C) as QueryData>::access(&state);
        assert_eq!(access.components.len(), 3);
        assert!(access.components.iter().all(|c| !c.mutable));
        let mut ids: Vec<_> = access.components.iter().map(|c| c.component_id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), 3);
    }

    #[test]
    fn three_tuple_empty_when_one_side_missing() {
        let mut world = World::new();
        let e = world.spawn();
        world.insert(e, Pos(1));
        world.insert(e, Vel(2));
        let out: Vec<_> = world.query::<(&Pos, &Vel, &C)>().collect();
        assert!(out.is_empty());
    }

    // --- &mut T (C9.4) ---------------------------------------------------

    #[test]
    fn write_only_query_visits_every_entry() {
        let mut world = World::new();
        let a = world.spawn();
        let b = world.spawn();
        world.insert(a, Pos(1));
        world.insert(b, Pos(2));
        for mut p in world.query::<WriteOnly<Pos>>() {
            p.0 *= 10;
        }
        assert_eq!(world.get::<Pos>(a), Some(&Pos(10)));
        assert_eq!(world.get::<Pos>(b), Some(&Pos(20)));
    }

    #[test]
    fn write_only_deref_mut_bumps_change_tick() {
        let mut world = World::new();
        let e = world.spawn();
        world.insert(e, Pos(0)); // tick 0 at insert
        world.increment_tick(); // tick = 1

        for mut p in world.query::<WriteOnly<Pos>>() {
            p.0 = 42; // bumps to tick 1
        }

        let changed: Vec<_> = world.changed_since::<Pos>(0).map(|(id, _)| id).collect();
        assert_eq!(changed, vec![e]);
        assert!(world.changed_since::<Pos>(1).next().is_none());
    }

    #[test]
    fn write_only_deref_only_does_not_bump_tick() {
        let mut world = World::new();
        let e = world.spawn();
        world.insert(e, Pos(7));
        world.increment_tick();

        let mut sum = 0;
        for p in world.query::<WriteOnly<Pos>>() {
            sum += p.0; // Deref only, no DerefMut.
        }
        assert_eq!(sum, 7);
        assert!(world.changed_since::<Pos>(0).next().is_none());
    }

    #[test]
    fn read_and_write_on_disjoint_types_compose() {
        let mut world = World::new();
        let e = world.spawn();
        world.insert(e, Pos(0));
        world.insert(e, Vel(5));
        for (mut p, v) in world.query::<(WriteOnly<Pos>, &Vel)>() {
            p.0 += v.0;
        }
        assert_eq!(world.get::<Pos>(e), Some(&Pos(5)));
    }

    #[test]
    #[should_panic(expected = "alias conflict")]
    fn aliasing_write_panics_on_query_construction() {
        let mut world = World::new();
        let e = world.spawn();
        world.insert(e, Pos(0));
        // `&Pos` + `WriteOnly<Pos>` over the same type — alias check
        // must reject this before any unsafe runs.
        let _ = world.query::<(&Pos, WriteOnly<Pos>)>().count();
    }

    #[test]
    #[should_panic(expected = "alias conflict")]
    fn double_write_panics_on_query_construction() {
        let mut world = World::new();
        let e = world.spawn();
        world.insert(e, Pos(0));
        let _ = world.query::<(WriteOnly<Pos>, WriteOnly<Pos>)>().count();
    }

    #[test]
    fn double_read_does_not_panic() {
        let mut world = World::new();
        let e = world.spawn();
        world.insert(e, Pos(7));
        // Two `&Pos` slots are a redundancy, not an alias violation.
        let out: Vec<_> = world
            .query::<(&Pos, &Pos)>()
            .map(|(a, b)| (a.0, b.0))
            .collect();
        assert_eq!(out, vec![(7, 7)]);
    }
}
