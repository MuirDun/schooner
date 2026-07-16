use std::any::Any;
use std::collections::HashMap;
use std::ops::{Deref, DerefMut};

use crate::ecs::query::data::QueryData;
use crate::ecs::query::iter::QueryIter;
use crate::ecs::{
    ChangeTicks, Component, ComponentId, ComponentRegistry, ComponentStorage, EntityAllocator,
    EntityId, Resources, SparseSet,
};

/// Smart pointer returned by [`World::get_mut`].
///
/// Holds a mutable borrow of the component value alongside its
/// [`ChangeTicks`] record and the world's current tick. The tick is bumped
/// only when the caller reaches through [`DerefMut`] — plain [`Deref`]
/// reads do not touch the tick. This is the load-bearing invariant for
/// change-detection: a system that only reads must not mark data dirty.
pub struct Mut<'w, T> {
    value: &'w mut T,
    ticks: &'w mut ChangeTicks,
    current_tick: u64,
}

impl<'w, T> Mut<'w, T> {
    /// Crate-internal constructor for the query fetch path.
    ///
    /// `value` and `ticks` must be a paired (value, change-ticks)
    /// pair from a `SparseSet<T>` dense slot — same pairing
    /// `World::get_mut` produces. `current_tick` is the world tick at
    /// the start of the iteration, which is what gets stamped on
    /// `DerefMut`. C9.4 takes the tick at query construction so all
    /// writes inside one query share the same tick — consistent with
    /// `Schedule::run`'s "one tick bump per stage" invariant.
    pub(crate) fn from_raw_parts(
        value: &'w mut T,
        ticks: &'w mut ChangeTicks,
        current_tick: u64,
    ) -> Self {
        Self {
            value,
            ticks,
            current_tick,
        }
    }
}

impl<'w, T> Deref for Mut<'w, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &*self.value
    }
}

impl<'w, T> DerefMut for Mut<'w, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.ticks.last_mutation_tick = self.current_tick;
        &mut *self.value
    }
}

/// Double-buffered record of component removals, keyed by
/// [`ComponentId`].
///
/// Removal is the one signal that can't be recovered by querying the
/// world after the fact — once `T` is gone, nothing in the world says
/// it was ever there. So we capture it at the moment of removal
/// (`World::remove` / `World::despawn`) into this ledger, which
/// outlives the data. It is the *poll*-shaped answer to "react to a
/// removal" — the sibling of `Changed<T>` and `Events<T>`, and the
/// deliberate alternative to a subscribe-style on-remove callback.
///
/// Double-buffered so a one-frame-late reader still sees the removal:
/// producers write `front`; readers see `front` + `back`; [`swap`] at
/// frame top rotates `front` into `back` and clears the new `front`.
/// Readers must be idempotent — an entity can appear in both buffers
/// across the two-frame window.
///
/// [`swap`]: RemovedLedger::swap
#[derive(Default)]
struct RemovedLedger {
    front: HashMap<ComponentId, Vec<EntityId>>,
    back: HashMap<ComponentId, Vec<EntityId>>,
}

impl RemovedLedger {
    fn record(&mut self, id: ComponentId, entity: EntityId) {
        self.front.entry(id).or_default().push(entity);
    }

    fn swap(&mut self) {
        std::mem::swap(&mut self.front, &mut self.back);
        // New front = old back; clear it so this frame starts empty,
        // keeping the Vec allocations for reuse.
        for v in self.front.values_mut() {
            v.clear();
        }
    }

    fn iter_for(&self, id: Option<ComponentId>) -> impl Iterator<Item = EntityId> + '_ {
        id.into_iter().flat_map(move |id| {
            let front = self.front.get(&id).into_iter().flatten();
            let back = self.back.get(&id).into_iter().flatten();
            front.chain(back).copied()
        })
    }
}

/// The ECS world: the single authority over entities, components, and
/// (later) resources.
///
/// Owns its own [`ComponentRegistry`] so tests and sub-worlds don't share
/// a global mutex. Storages are type-erased behind [`ComponentStorage`]
/// and keyed by [`ComponentId`].
///
/// `current_tick` is threaded into every [`Mut<T>`]; the bump strategy
/// itself lands in C7 alongside `changed_since`.
#[derive(Default)]
pub struct World {
    entities: EntityAllocator,
    registry: ComponentRegistry,
    storages: HashMap<ComponentId, Box<dyn ComponentStorage>>,
    resources: Resources,
    removed: RemovedLedger,
    pub current_tick: u64,
}

impl World {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn spawn(&mut self) -> EntityId {
        self.entities.allocate()
    }

    /// Tear down an entity: drop every component it holds, then return
    /// the slot to the allocator. Returns `true` if the entity was alive.
    ///
    /// Each component the entity actually carried is recorded in the
    /// removed-ledger under its [`ComponentId`], so `removed::<T>()`
    /// fires on whole-entity despawn, not just on an explicit
    /// `remove::<T>`.
    pub fn despawn(&mut self, entity: EntityId) -> bool {
        if !self.entities.is_alive(entity) {
            return false;
        }
        // `storages` and `removed` are disjoint fields, so the loop's
        // `&mut storages` borrow and `removed.record`'s `&mut removed`
        // borrow don't conflict. `iter_mut` (not `values_mut`) so we
        // have the ComponentId to key the ledger.
        for (id, storage) in self.storages.iter_mut() {
            if storage.remove_entity(entity) {
                self.removed.record(*id, entity);
            }
        }
        self.entities.free(entity)
    }

    pub fn is_alive(&self, entity: EntityId) -> bool {
        self.entities.is_alive(entity)
    }

    pub fn entity_count(&self) -> usize {
        self.entities.len()
    }

    pub fn current_tick(&self) -> u64 {
        self.current_tick
    }

    /// Register a component type and return its id, idempotent.
    ///
    /// Exposed for the [`QueryData`](crate::ecs::QueryData) machinery,
    /// which resolves type-level access to runtime [`ComponentId`]s
    /// once at query construction. Calling this from app code is
    /// harmless — `insert` does the same thing internally.
    pub fn register_component<T: Component>(&mut self) -> ComponentId {
        self.registry.register::<T>()
    }

    /// Return the id assigned to `T`, or `None` if `T` was never
    /// inserted. Used by `QueryData::init_state` only when it wants to
    /// avoid registering an unused type.
    pub fn component_id<T: Component>(&self) -> Option<ComponentId> {
        self.registry.id_of::<T>()
    }

    /// Borrow the type-erased storage for `id` if one exists. The
    /// join engine caches these references at construction time so
    /// the per-entity probe is one virtual call, no `HashMap` hash.
    pub(crate) fn storage(&self, id: ComponentId) -> Option<&dyn ComponentStorage> {
        self.storages.get(&id).map(|s| s.as_ref())
    }

    /// Raw pointer to the boxed storage cell for `id`, used by the
    /// query fetch's `unsafe` split-borrow. The pointer is valid as
    /// long as `&mut World` is held by the caller — `HashMap`
    /// reallocation can only happen on insert/remove, neither of
    /// which can run while the caller holds the world borrow.
    pub(crate) fn storage_box_ptr(
        &mut self,
        id: ComponentId,
    ) -> Option<*mut Box<dyn ComponentStorage>> {
        self.storages.get_mut(&id).map(|b| b as *mut _)
    }

    /// Resolve a `ComponentId` to its registered Rust `type_name`,
    /// for diagnostics. Returns `None` if the id was never
    /// registered in this world.
    pub(crate) fn component_name(&self, id: ComponentId) -> Option<&'static str> {
        self.registry.name(id)
    }

    /// Advance the world clock by one and return the new tick. The
    /// schedule calls this between system runs so each system observes a
    /// monotonically increasing tick against which `changed_since`
    /// queries are anchored.
    pub fn increment_tick(&mut self) -> u64 {
        self.current_tick += 1;
        self.current_tick
    }

    /// Iterate components of type `T` whose last mutation tick is
    /// strictly greater than `since`. Empty iter if `T` was never
    /// registered. Intended idiom: a system remembers
    /// `last_seen = world.current_tick()` at end-of-run and queries
    /// `changed_since(last_seen)` on its next run.
    pub fn changed_since<T: Component>(
        &self,
        since: u64,
    ) -> impl Iterator<Item = (EntityId, &T)> + '_ {
        self.sparse_set::<T>().into_iter().flat_map(move |s| {
            s.iter()
                .zip(s.iter_ticks())
                .filter_map(move |((entity, value), (_, ticks))| {
                    (ticks.last_mutation_tick > since).then_some((entity, value))
                })
        })
    }

    /// Iterate components of type `T` whose *add* tick is strictly
    /// greater than `since` — newly inserted (not merely mutated)
    /// since the cursor. Same cursor idiom as [`Self::changed_since`]:
    /// a reaction remembers its own last-run tick and passes it in. A
    /// remove-then-reinsert re-stamps the add tick, so it resurfaces.
    pub fn added_since<T: Component>(
        &self,
        since: u64,
    ) -> impl Iterator<Item = (EntityId, &T)> + '_ {
        self.sparse_set::<T>().into_iter().flat_map(move |s| {
            s.iter()
                .zip(s.iter_ticks())
                .filter_map(move |((entity, value), (_, ticks))| {
                    (ticks.added_tick > since).then_some((entity, value))
                })
        })
    }

    /// Iterate entities whose `T` was removed — by an explicit
    /// `remove::<T>` or by a whole-entity `despawn` — within the
    /// readable window (this frame and the previous one). Empty if `T`
    /// was never registered.
    ///
    /// Readers must be **idempotent**: an entity can surface more than
    /// once across the two-frame window (and the entity is already gone,
    /// so only its [`EntityId`] is available, not its data). The
    /// canonical consumer is resource cleanup keyed off a side map —
    /// e.g. freeing a physics handle via `map.remove(entity)`, which is
    /// a no-op the second time.
    pub fn removed<T: Component>(&self) -> impl Iterator<Item = EntityId> + '_ {
        self.removed.iter_for(self.registry.id_of::<T>())
    }

    /// Rotate the removed-ledger's double buffer: last frame's removals
    /// drop out of the readable window, this frame's begin
    /// accumulating. Call exactly once per frame, at the top of
    /// `App::tick` — calling it mid-frame would drop removals before
    /// their readers run.
    pub fn swap_removed(&mut self) {
        self.removed.swap();
    }

    // --- queries ---------------------------------------------------------

    fn sparse_set<T: Component>(&self) -> Option<&SparseSet<T>> {
        let id = self.registry.id_of::<T>()?;
        let storage = self.storages.get(&id)?;
        storage.as_any().downcast_ref::<SparseSet<T>>()
    }

    fn sparse_set_mut<T: Component>(&mut self) -> Option<&mut SparseSet<T>> {
        let id = self.registry.id_of::<T>()?;
        let storage = self.storages.get_mut(&id)?;
        storage.as_any_mut().downcast_mut::<SparseSet<T>>()
    }

    /// Iterate every `(EntityId, &T)` live in the world. Empty iter if
    /// `T` was never registered.
    pub fn iter<T: Component>(&self) -> impl Iterator<Item = (EntityId, &T)> + '_ {
        self.sparse_set::<T>().into_iter().flat_map(|s| s.iter())
    }

    /// Iterate `(EntityId, Mut<T>)` over every live `T`. Each yielded
    /// `Mut` bumps the tick only on `DerefMut` — pure reads during
    /// iteration do not mark data dirty.
    pub fn iter_mut<'w, T: Component>(
        &'w mut self,
    ) -> impl Iterator<Item = (EntityId, Mut<'w, T>)> + 'w {
        let current_tick = self.current_tick;
        self.sparse_set_mut::<T>().into_iter().flat_map(move |s| {
            s.iter_mut_with_ticks().map(move |(entity, value, ticks)| {
                (
                    entity,
                    Mut {
                        value,
                        ticks,
                        current_tick,
                    },
                )
            })
        })
    }

    /// Run a typed [`QueryData`] over the world, with no filter.
    ///
    /// Equivalent to `query_filtered::<D, ()>()`. See
    /// [`Self::query_filtered`] for the trait-level details.
    pub fn query<D: QueryData>(&mut self) -> QueryIter<'_, D, ()> {
        self.query_filtered::<D, ()>()
    }

    /// Run a typed [`QueryData`] gated by a [`QueryFilter`].
    ///
    /// `D::init_state` and `F::init_state` both auto-register every
    /// component type they touch. The combined access set feeds the
    /// alias check; the join engine picks the smallest required
    /// data storage as the driver; the typed `Fetch` pair (built via
    /// the audited unsafe split-borrow in
    /// [`fetch::split_storages`](crate::ecs::query::fetch::split_storages))
    /// materialises items per entity, with `F::matches` skipping
    /// entries the filter rejects.
    pub fn query_filtered<D: QueryData, F: crate::ecs::query::filter::QueryFilter>(
        &mut self,
    ) -> QueryIter<'_, D, F> {
        let state = D::init_state(self);
        let filter_state = F::init_state(self);
        QueryIter::new(self, state, filter_state)
    }

    /// Run a typed [`QueryData`] gated by a [`QueryFilter`], supplying a
    /// `since` cursor to the change-detection filters (`Added<T>` /
    /// `Changed<T>`). Presence filters ignore it.
    ///
    /// This is the explicit-cursor entry point for reaction systems: a
    /// system remembers its own last-run tick (`world.current_tick()`
    /// captured at the start of its run) and passes it as `since`, so
    /// the filter selects only the entities whose `T` was added /
    /// changed since that system last looked. Strict `>`: an entity
    /// touched exactly at `since` is excluded.
    pub fn query_filtered_since<D: QueryData, F: crate::ecs::query::filter::QueryFilter>(
        &mut self,
        since: u64,
    ) -> QueryIter<'_, D, F> {
        let state = D::init_state(self);
        let filter_state = F::init_state_since(self, since);
        QueryIter::new(self, state, filter_state)
    }

    /// Insert (or replace) `T` on `entity`. Auto-registers `T` on first
    /// use. Returns the previous value if one existed; returns `None`
    /// silently when the entity is not alive.
    pub fn insert<T: Component>(&mut self, entity: EntityId, value: T) -> Option<T> {
        if !self.entities.is_alive(entity) {
            return None;
        }
        let id = self.registry.register::<T>();
        let storage = self
            .storages
            .entry(id)
            .or_insert_with(|| Box::new(SparseSet::<T>::new()));
        let set = storage
            .as_any_mut()
            .downcast_mut::<SparseSet<T>>()
            .expect("storage for ComponentId always holds SparseSet<T>");
        set.insert(entity, value, self.current_tick)
    }

    /// Remove `T` from `entity`, returning the prior value if present.
    /// Records the removal in the ledger so `removed::<T>()` reports it.
    pub fn remove<T: Component>(&mut self, entity: EntityId) -> Option<T> {
        if !self.entities.is_alive(entity) {
            return None;
        }
        let id = self.registry.id_of::<T>()?;
        let storage = self.storages.get_mut(&id)?;
        let set = storage
            .as_any_mut()
            .downcast_mut::<SparseSet<T>>()
            .expect("storage for ComponentId always holds SparseSet<T>");
        let removed = set.remove(entity);
        if removed.is_some() {
            self.removed.record(id, entity);
        }
        removed
    }

    pub fn contains<T: Component>(&self, entity: EntityId) -> bool {
        self.get::<T>(entity).is_some()
    }

    pub fn get<T: Component>(&self, entity: EntityId) -> Option<&T> {
        if !self.entities.is_alive(entity) {
            return None;
        }
        let id = self.registry.id_of::<T>()?;
        let storage = self.storages.get(&id)?;
        let set = storage
            .as_any()
            .downcast_ref::<SparseSet<T>>()
            .expect("storage for ComponentId always holds SparseSet<T>");
        set.get(entity)
    }

    // --- resources -------------------------------------------------------

    /// Insert or replace the single instance of `R`. Returns the prior
    /// value, if any. Resources are world-scoped singletons (clocks,
    /// input, renderer handles) — they do not live on entities.
    pub fn insert_resource<R: Any + Send + Sync>(&mut self, value: R) -> Option<R> {
        self.resources.insert(value)
    }

    pub fn remove_resource<R: Any + Send + Sync>(&mut self) -> Option<R> {
        self.resources.remove::<R>()
    }

    pub fn resource<R: Any + Send + Sync>(&self) -> Option<&R> {
        self.resources.get::<R>()
    }

    pub fn resource_mut<R: Any + Send + Sync>(&mut self) -> Option<&mut R> {
        self.resources.get_mut::<R>()
    }

    pub fn contains_resource<R: Any + Send + Sync>(&self) -> bool {
        self.resources.contains::<R>()
    }

    // --- component mutable access ----------------------------------------

    /// Mutable component access. The returned [`Mut<T>`] bumps the
    /// mutation tick only when the caller goes through [`DerefMut`],
    /// not on construction and not on plain [`Deref`].
    pub fn get_mut<T: Component>(&mut self, entity: EntityId) -> Option<Mut<'_, T>> {
        if !self.entities.is_alive(entity) {
            return None;
        }
        let id = self.registry.id_of::<T>()?;
        let current_tick = self.current_tick;
        let storage = self.storages.get_mut(&id)?;
        let set = storage
            .as_any_mut()
            .downcast_mut::<SparseSet<T>>()
            .expect("storage for ComponentId always holds SparseSet<T>");
        let (value, ticks) = set.get_mut_with_ticks(entity)?;
        Some(Mut {
            value,
            ticks,
            current_tick,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- entity lifecycle -------------------------------------------------

    #[test]
    fn spawn_yields_fresh_live_entities() {
        let mut world = World::new();
        let a = world.spawn();
        let b = world.spawn();
        assert_ne!(a, b);
        assert!(world.is_alive(a));
        assert!(world.is_alive(b));
        assert_eq!(world.entity_count(), 2);
    }

    #[test]
    fn despawn_live_entity_returns_true_and_kills_it() {
        let mut world = World::new();
        let e = world.spawn();
        assert!(world.despawn(e));
        assert!(!world.is_alive(e));
        assert_eq!(world.entity_count(), 0);
    }

    #[test]
    fn despawn_stale_entity_returns_false() {
        let mut world = World::new();
        let e = world.spawn();
        world.despawn(e);
        assert!(!world.despawn(e));
    }

    #[test]
    fn despawn_drops_all_components_on_entity() {
        let mut world = World::new();
        let e = world.spawn();
        world.insert::<i32>(e, 7);
        world.insert::<String>(e, "hi".into());
        assert!(world.despawn(e));
        // Slot recycles; the new entity must not inherit old components.
        let reused = world.spawn();
        assert_eq!(reused.index, e.index);
        assert_eq!(world.get::<i32>(reused), None);
        assert_eq!(world.get::<String>(reused), None);
    }

    // --- component lifecycle ---------------------------------------------

    #[test]
    fn insert_then_get_roundtrips_value() {
        let mut world = World::new();
        let e = world.spawn();
        assert_eq!(world.insert::<i32>(e, 42), None);
        assert_eq!(world.get::<i32>(e), Some(&42));
        assert!(world.contains::<i32>(e));
    }

    #[test]
    fn insert_replace_returns_prior_value() {
        let mut world = World::new();
        let e = world.spawn();
        world.insert::<i32>(e, 1);
        assert_eq!(world.insert::<i32>(e, 2), Some(1));
        assert_eq!(world.get::<i32>(e), Some(&2));
    }

    #[test]
    fn insert_on_dead_entity_is_silent_none() {
        let mut world = World::new();
        let e = world.spawn();
        world.despawn(e);
        assert_eq!(world.insert::<i32>(e, 42), None);
        assert!(!world.contains::<i32>(e));
    }

    #[test]
    fn remove_returns_value_then_none() {
        let mut world = World::new();
        let e = world.spawn();
        world.insert::<i32>(e, 42);
        assert_eq!(world.remove::<i32>(e), Some(42));
        assert_eq!(world.remove::<i32>(e), None);
        assert!(!world.contains::<i32>(e));
    }

    #[test]
    fn remove_unknown_component_type_returns_none() {
        let mut world = World::new();
        let e = world.spawn();
        // Type was never registered — still must be a silent None.
        assert_eq!(world.remove::<i32>(e), None);
    }

    #[test]
    fn heterogeneous_components_coexist_on_one_entity() {
        let mut world = World::new();
        let e = world.spawn();
        world.insert::<i32>(e, 42);
        world.insert::<String>(e, "hello".into());
        assert_eq!(world.get::<i32>(e), Some(&42));
        assert_eq!(world.get::<String>(e), Some(&"hello".to_string()));
    }

    // --- read / write + Mut<T> semantics ---------------------------------

    #[test]
    fn get_on_dead_entity_returns_none() {
        let mut world = World::new();
        let e = world.spawn();
        world.insert::<i32>(e, 42);
        world.despawn(e);
        assert_eq!(world.get::<i32>(e), None);
    }

    #[test]
    fn get_on_unregistered_type_returns_none() {
        let mut world = World::new();
        let e = world.spawn();
        assert_eq!(world.get::<i32>(e), None);
    }

    #[test]
    fn insert_bumps_tick_to_current() {
        let mut world = World::new();
        let e = world.spawn();
        world.insert::<i32>(e, 42);
        let expected = world.current_tick;
        let m = world.get_mut::<i32>(e).unwrap();
        assert_eq!(m.ticks.last_mutation_tick, expected);
    }

    #[test]
    fn mut_construction_does_not_bump_tick() {
        let mut world = World::new();
        let e = world.spawn();
        world.insert::<i32>(e, 42);
        // Simulate a system advancing the clock without C7's API.
        world.current_tick = 10;
        {
            let _m = world.get_mut::<i32>(e).unwrap();
            // Drop without deref_mut.
        }
        // Tick must still be the insert-time tick (0), not 10.
        let m = world.get_mut::<i32>(e).unwrap();
        assert_eq!(m.ticks.last_mutation_tick, 0);
    }

    #[test]
    fn mut_deref_does_not_bump_tick() {
        let mut world = World::new();
        let e = world.spawn();
        world.insert::<i32>(e, 42);
        world.current_tick = 10;
        {
            let m = world.get_mut::<i32>(e).unwrap();
            assert_eq!(*m, 42); // Deref — read only.
        }
        let m = world.get_mut::<i32>(e).unwrap();
        assert_eq!(m.ticks.last_mutation_tick, 0);
    }

    #[test]
    fn mut_deref_mut_bumps_tick_to_current() {
        let mut world = World::new();
        let e = world.spawn();
        world.insert::<i32>(e, 42);
        world.current_tick = 10;
        {
            let mut m = world.get_mut::<i32>(e).unwrap();
            *m = 99; // DerefMut — bumps.
        }
        let m = world.get_mut::<i32>(e).unwrap();
        assert_eq!(*m, 99);
        assert_eq!(m.ticks.last_mutation_tick, 10);
    }

    #[test]
    fn mut_multiple_deref_muts_keep_latest_tick_only() {
        let mut world = World::new();
        let e = world.spawn();
        world.insert::<i32>(e, 0);
        world.current_tick = 5;
        {
            let mut m = world.get_mut::<i32>(e).unwrap();
            *m = 1;
            *m = 2;
            *m = 3;
        }
        let m = world.get_mut::<i32>(e).unwrap();
        assert_eq!(*m, 3);
        assert_eq!(m.ticks.last_mutation_tick, 5);
    }

    #[test]
    fn get_mut_on_dead_entity_returns_none() {
        let mut world = World::new();
        let e = world.spawn();
        world.insert::<i32>(e, 42);
        world.despawn(e);
        assert!(world.get_mut::<i32>(e).is_none());
    }

    // --- misc -------------------------------------------------------------

    #[test]
    fn current_tick_defaults_to_zero() {
        let world = World::new();
        assert_eq!(world.current_tick(), 0);
    }

    #[test]
    fn insert_auto_registers_new_component_types() {
        let mut world = World::new();
        let e = world.spawn();
        // No explicit registration — this must Just Work.
        world.insert::<i32>(e, 1);
        world.insert::<String>(e, "x".into());
        world.insert::<f64>(e, 2.5);
        assert_eq!(world.get::<i32>(e), Some(&1));
        assert_eq!(world.get::<f64>(e), Some(&2.5));
    }

    // --- resources -------------------------------------------------------

    #[derive(Debug, PartialEq)]
    struct Gravity(f32);

    #[test]
    fn resource_insert_get_mut_roundtrip() {
        let mut world = World::new();
        assert_eq!(world.insert_resource(Gravity(9.8)), None);
        assert_eq!(world.resource::<Gravity>(), Some(&Gravity(9.8)));
        assert!(world.contains_resource::<Gravity>());
        if let Some(g) = world.resource_mut::<Gravity>() {
            g.0 = 1.62;
        }
        assert_eq!(world.resource::<Gravity>(), Some(&Gravity(1.62)));
    }

    #[test]
    fn resource_remove_returns_value_then_none() {
        let mut world = World::new();
        world.insert_resource(Gravity(9.8));
        assert_eq!(world.remove_resource::<Gravity>(), Some(Gravity(9.8)));
        assert_eq!(world.remove_resource::<Gravity>(), None);
        assert!(!world.contains_resource::<Gravity>());
    }

    #[test]
    fn resources_are_independent_of_entities() {
        let mut world = World::new();
        world.insert_resource(Gravity(9.8));
        let e = world.spawn();
        world.insert::<i32>(e, 42);
        world.despawn(e);
        // Despawning an entity must not touch resources.
        assert_eq!(world.resource::<Gravity>(), Some(&Gravity(9.8)));
    }

    // --- change detection ------------------------------------------------

    fn collect_changed<T: Component + Clone>(world: &World, since: u64) -> Vec<(EntityId, T)> {
        let mut out: Vec<_> = world
            .changed_since::<T>(since)
            .map(|(e, v)| (e, v.clone()))
            .collect();
        out.sort_by_key(|(e, _)| e.index);
        out
    }

    #[test]
    fn increment_tick_advances_and_returns_new_value() {
        let mut world = World::new();
        assert_eq!(world.current_tick(), 0);
        assert_eq!(world.increment_tick(), 1);
        assert_eq!(world.increment_tick(), 2);
        assert_eq!(world.current_tick(), 2);
    }

    #[test]
    fn changed_since_on_unregistered_type_is_empty() {
        let world = World::new();
        assert_eq!(collect_changed::<i32>(&world, 0), vec![]);
    }

    #[test]
    fn changed_since_returns_empty_when_nothing_mutated() {
        let mut world = World::new();
        let e = world.spawn();
        world.insert::<i32>(e, 42);
        // Insert happened at tick 0, so asking "what changed after tick 0"
        // should return nothing (strict >).
        assert_eq!(collect_changed::<i32>(&world, 0), vec![]);
    }

    #[test]
    fn insert_at_tick_shows_up_as_changed_below_that_tick() {
        let mut world = World::new();
        world.increment_tick(); // tick = 1
        let e = world.spawn();
        world.insert::<i32>(e, 42); // last_mutation_tick = 1
        assert_eq!(collect_changed::<i32>(&world, 0), vec![(e, 42)]);
        assert_eq!(collect_changed::<i32>(&world, 1), vec![]);
    }

    #[test]
    fn mutation_via_deref_mut_shows_up_in_changed_since() {
        let mut world = World::new();
        let e = world.spawn();
        world.insert::<i32>(e, 1); // last_mutation_tick = 0
        world.increment_tick(); // tick = 1
        {
            let mut m = world.get_mut::<i32>(e).unwrap();
            *m = 99;
        }
        // last_mutation_tick now = 1; since=0 catches it, since=1 doesn't.
        assert_eq!(collect_changed::<i32>(&world, 0), vec![(e, 99)]);
        assert_eq!(collect_changed::<i32>(&world, 1), vec![]);
    }

    #[test]
    fn read_only_access_does_not_show_up_in_changed_since() {
        let mut world = World::new();
        let e = world.spawn();
        world.insert::<i32>(e, 1); // tick 0
        world.increment_tick(); // tick 1
        {
            let m = world.get_mut::<i32>(e).unwrap();
            assert_eq!(*m, 1); // Deref only.
        }
        let _ = world.get::<i32>(e);
        // No DerefMut happened; last_mutation_tick stays at 0.
        assert_eq!(collect_changed::<i32>(&world, 0), vec![]);
    }

    #[test]
    fn changed_since_strict_greater_than_boundary() {
        let mut world = World::new();
        let e = world.spawn();
        world.insert::<i32>(e, 42); // tick 0, last_mutation_tick = 0
        // since == last_mutation_tick must be excluded.
        assert_eq!(collect_changed::<i32>(&world, 0), vec![]);
        // since < last_mutation_tick requires advancing and mutating.
        world.increment_tick();
        {
            let mut m = world.get_mut::<i32>(e).unwrap();
            *m = 7;
        }
        // last_mutation_tick = 1
        assert_eq!(collect_changed::<i32>(&world, 0), vec![(e, 7)]);
        assert_eq!(collect_changed::<i32>(&world, 1), vec![]);
    }

    #[test]
    fn changed_since_isolates_touched_entities_only() {
        let mut world = World::new();
        let a = world.spawn();
        let b = world.spawn();
        let c = world.spawn();
        world.insert::<i32>(a, 1);
        world.insert::<i32>(b, 2);
        world.insert::<i32>(c, 3);
        world.increment_tick(); // tick 1
        // Only mutate b.
        {
            let mut m = world.get_mut::<i32>(b).unwrap();
            *m = 20;
        }
        let changed = collect_changed::<i32>(&world, 0);
        assert_eq!(changed, vec![(b, 20)]);
    }

    // --- add detection ---------------------------------------------------

    fn collect_added<T: Component + Clone>(world: &World, since: u64) -> Vec<(EntityId, T)> {
        let mut out: Vec<_> = world
            .added_since::<T>(since)
            .map(|(e, v)| (e, v.clone()))
            .collect();
        out.sort_by_key(|(e, _)| e.index);
        out
    }

    #[test]
    fn added_since_reports_new_inserts_strictly_after_cursor() {
        let mut world = World::new();
        world.increment_tick(); // tick 1
        let e = world.spawn();
        world.insert::<i32>(e, 1); // added_tick = 1
        assert_eq!(collect_added::<i32>(&world, 0), vec![(e, 1)]);
        // Strict >: an add exactly at the cursor is excluded.
        assert_eq!(collect_added::<i32>(&world, 1), vec![]);
    }

    #[test]
    fn mutation_does_not_resurface_as_added() {
        let mut world = World::new();
        let e = world.spawn();
        world.insert::<i32>(e, 1); // added_tick = 0, mutation = 0
        world.increment_tick(); // tick 1
        {
            let mut m = world.get_mut::<i32>(e).unwrap();
            *m = 99; // last_mutation_tick = 1, added_tick untouched
        }
        // It changed since tick 0, but it was not *added* since tick 0.
        assert_eq!(collect_changed::<i32>(&world, 0), vec![(e, 99)]);
        assert_eq!(collect_added::<i32>(&world, 0), vec![]);
    }

    #[test]
    fn reinsert_after_remove_counts_as_a_fresh_add() {
        let mut world = World::new();
        let e = world.spawn();
        world.insert::<i32>(e, 1); // added_tick = 0
        world.increment_tick(); // tick 1
        world.remove::<i32>(e);
        world.insert::<i32>(e, 2); // new dense entry → added_tick = 1
        assert_eq!(collect_added::<i32>(&world, 0), vec![(e, 2)]);
    }

    // --- change-cursor convention ----------------------------------------

    #[test]
    fn change_cursor_is_robust_across_fixed_step_stride() {
        // The pinned convention: a reaction anchors on *its own last-run
        // tick* (not a frame counter) and compares strict `>`. Because
        // `current_tick` is monotonic across every stage, the
        // FixedUpdate 0..N stride never over- or under-counts — the
        // reaction catches exactly the changes since it last ran,
        // however many stage bumps elapsed in between.
        let mut world = World::new();
        let x = world.spawn();
        let y = world.spawn();
        world.insert::<i32>(x, 0);
        world.insert::<i32>(y, 0);

        // Reaction's cursor starts at the current tick — nothing prior.
        let mut last_run = world.current_tick();

        // Frame 1: three fixed steps then update, mutating x mid-stride.
        world.increment_tick(); // fixed step 1
        world.increment_tick(); // fixed step 2
        {
            let mut p = world.get_mut::<i32>(x).unwrap();
            *p = 1;
        }
        world.increment_tick(); // fixed step 3
        world.increment_tick(); // update — the reaction "runs" here

        let changed: Vec<_> = world
            .changed_since::<i32>(last_run)
            .map(|(e, _)| e)
            .collect();
        assert_eq!(changed, vec![x]); // only x, despite four tick bumps
        last_run = world.current_tick();

        // Frame 2: nothing mutates. The advanced cursor must not
        // re-report x (no double-count across frames).
        world.increment_tick();
        world.increment_tick();
        let changed: Vec<_> = world
            .changed_since::<i32>(last_run)
            .map(|(e, _)| e)
            .collect();
        assert!(changed.is_empty());
    }

    // --- removal ledger --------------------------------------------------

    // Collect as `Vec<EntityId>`, not `Vec<u32>`: a transitive `gltf`
    // dep adds a second `PartialEq` impl for `u32`, which makes an
    // empty `vec![]` comparison against `Vec<u32>` ambiguous. `EntityId`
    // has a single `PartialEq`, so the empty case infers cleanly.
    fn collect_removed<T: Component>(world: &World) -> Vec<EntityId> {
        let mut out: Vec<EntityId> = world.removed::<T>().collect();
        out.sort_by_key(|e| e.index);
        out
    }

    #[test]
    fn removed_of_unregistered_type_is_empty() {
        let world = World::new();
        assert_eq!(collect_removed::<i32>(&world), vec![]);
    }

    #[test]
    fn remove_records_entity_in_removed_reader() {
        let mut world = World::new();
        let e = world.spawn();
        world.insert::<i32>(e, 1);
        assert_eq!(collect_removed::<i32>(&world), vec![]);
        world.remove::<i32>(e);
        assert_eq!(collect_removed::<i32>(&world), vec![e]);
    }

    #[test]
    fn remove_of_absent_component_records_nothing() {
        let mut world = World::new();
        let e = world.spawn();
        // i32 never inserted on e.
        assert_eq!(world.remove::<i32>(e), None);
        assert_eq!(collect_removed::<i32>(&world), vec![]);
    }

    #[test]
    fn despawn_records_under_every_component_the_entity_had() {
        let mut world = World::new();
        let e = world.spawn();
        world.insert::<i32>(e, 1);
        world.insert::<String>(e, "x".into());
        world.despawn(e);
        assert_eq!(collect_removed::<i32>(&world), vec![e]);
        assert_eq!(collect_removed::<String>(&world), vec![e]);
    }

    #[test]
    fn despawn_does_not_record_components_the_entity_lacked() {
        let mut world = World::new();
        let e = world.spawn();
        world.insert::<i32>(e, 1);
        // f64 storage exists (another entity has it) but e never did.
        let other = world.spawn();
        world.insert::<f64>(other, 2.0);
        world.despawn(e);
        assert_eq!(collect_removed::<i32>(&world), vec![e]);
        assert_eq!(collect_removed::<f64>(&world), vec![]);
    }

    #[test]
    fn swap_removed_keeps_prior_frame_then_drops_it() {
        let mut world = World::new();
        let e = world.spawn();
        world.insert::<i32>(e, 1);
        world.remove::<i32>(e); // lands in front
        assert_eq!(collect_removed::<i32>(&world), vec![e]);
        world.swap_removed(); // front -> back, still readable
        assert_eq!(collect_removed::<i32>(&world), vec![e]);
        world.swap_removed(); // back dropped, window closes
        assert_eq!(collect_removed::<i32>(&world), vec![]);
    }

    // --- queries ---------------------------------------------------------

    fn sorted_iter<T: Component + Clone>(world: &World) -> Vec<(EntityId, T)> {
        let mut out: Vec<_> = world.iter::<T>().map(|(e, v)| (e, v.clone())).collect();
        out.sort_by_key(|(e, _)| e.index);
        out
    }

    #[test]
    fn iter_on_empty_world_is_empty() {
        let world = World::new();
        assert_eq!(sorted_iter::<i32>(&world), vec![]);
    }

    #[test]
    fn iter_on_unregistered_type_is_empty() {
        let mut world = World::new();
        let e = world.spawn();
        world.insert::<String>(e, "x".into());
        assert_eq!(sorted_iter::<i32>(&world), vec![]);
    }

    #[test]
    fn iter_returns_all_entries() {
        let mut world = World::new();
        let a = world.spawn();
        let b = world.spawn();
        let c = world.spawn();
        world.insert::<i32>(a, 1);
        world.insert::<i32>(b, 2);
        world.insert::<i32>(c, 3);
        assert_eq!(sorted_iter::<i32>(&world), vec![(a, 1), (b, 2), (c, 3)]);
    }

    #[test]
    fn iter_mut_deref_mut_is_visible_in_changed_since() {
        let mut world = World::new();
        let a = world.spawn();
        let b = world.spawn();
        world.insert::<i32>(a, 1);
        world.insert::<i32>(b, 2);
        world.increment_tick(); // tick 1
        for (_, mut m) in world.iter_mut::<i32>() {
            *m *= 10;
        }
        let changed = collect_changed::<i32>(&world, 0);
        let mut idx: Vec<u32> = changed.iter().map(|(e, _)| e.index).collect();
        idx.sort();
        assert_eq!(idx, vec![a.index, b.index]);
        assert_eq!(world.get::<i32>(a), Some(&10));
        assert_eq!(world.get::<i32>(b), Some(&20));
    }

    #[test]
    fn iter_mut_deref_only_does_not_bump_ticks() {
        let mut world = World::new();
        let a = world.spawn();
        world.insert::<i32>(a, 42);
        world.increment_tick(); // tick 1
        let mut sum = 0;
        for (_, m) in world.iter_mut::<i32>() {
            sum += *m; // Deref only.
        }
        assert_eq!(sum, 42);
        assert_eq!(collect_changed::<i32>(&world, 0), vec![]);
    }
}
