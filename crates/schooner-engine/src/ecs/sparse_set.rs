use crate::ecs::EntityId;

/// Per-instance change-detection bookkeeping, stored parallel to dense
/// values. Struct-shape (not raw `u64`) leaves room for future fields
/// like `added_tick` without an API break.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ChangeTicks {
    pub last_mutation_tick: u64,
}

impl ChangeTicks {
    pub fn new(tick: u64) -> Self {
        Self {
            last_mutation_tick: tick,
        }
    }
}

type DenseIndex = u32;

/// Sparse-set storage for a single component type.
///
/// Layout:
/// - `sparse[entity.index]` → `Some(dense_idx)` if this entity stores a
///   value, else `None`. Grown on demand to the largest seen index.
/// - `dense[dense_idx]` = `(EntityId, T)`. The `EntityId` carries the
///   generation so stale lookups can be detected.
/// - `ticks[dense_idx]` — parallel to `dense`, change-detection
///   bookkeeping.
///
/// Removal uses swap-remove: move `dense.last()` into the removed slot
/// and truncate. Iteration order is NOT insertion-stable.
#[derive(Debug)]
pub struct SparseSet<T> {
    sparse: Vec<Option<DenseIndex>>,
    dense: Vec<(EntityId, T)>,
    ticks: Vec<ChangeTicks>,
}

impl<T> Default for SparseSet<T> {
    fn default() -> Self {
        Self {
            sparse: Vec::new(),
            dense: Vec::new(),
            ticks: Vec::new(),
        }
    }
}

impl<T> SparseSet<T> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.dense.len()
    }

    pub fn is_empty(&self) -> bool {
        self.dense.is_empty()
    }

    /// Insert or replace the value for `entity`, recording `tick` as the
    /// mutation tick. Returns the previous value on replace.
    pub fn insert(&mut self, entity: EntityId, value: T, tick: u64) -> Option<T> {
        let slot = entity.index as usize;
        if slot >= self.sparse.len() {
            self.sparse.resize(slot + 1, None);
        }

        if let Some(dense_idx) = self.sparse[slot] {
            let dense_idx = dense_idx as usize;
            // Contract: `World::despawn` must `remove` before a slot is
            // recycled to a new generation. If this fires, a storage was
            // missed during despawn.
            debug_assert_eq!(
                self.dense[dense_idx].0, entity,
                "SparseSet::insert into recycled slot without prior remove"
            );
            let old = std::mem::replace(&mut self.dense[dense_idx].1, value);
            self.ticks[dense_idx].last_mutation_tick = tick;
            Some(old)
        } else {
            let dense_idx = self.dense.len() as DenseIndex;
            self.sparse[slot] = Some(dense_idx);
            self.dense.push((entity, value));
            self.ticks.push(ChangeTicks::new(tick));
            None
        }
    }

    /// Remove and return the value, or `None` if absent or stale.
    pub fn remove(&mut self, entity: EntityId) -> Option<T> {
        let slot = entity.index as usize;
        let dense_idx = self.sparse.get(slot).copied().flatten()? as usize;
        if self.dense[dense_idx].0 != entity {
            return None;
        }

        let (_, value) = self.dense.swap_remove(dense_idx);
        self.ticks.swap_remove(dense_idx);
        self.sparse[slot] = None;

        // If swap-remove moved the last entry into `dense_idx`, update
        // its sparse pointer to the new position.
        if dense_idx < self.dense.len() {
            let moved_entity = self.dense[dense_idx].0;
            self.sparse[moved_entity.index as usize] = Some(dense_idx as DenseIndex);
        }

        Some(value)
    }

    pub fn contains(&self, entity: EntityId) -> bool {
        self.get(entity).is_some()
    }

    pub fn get(&self, entity: EntityId) -> Option<&T> {
        let slot = entity.index as usize;
        let dense_idx = self.sparse.get(slot).copied().flatten()? as usize;
        let (stored, value) = &self.dense[dense_idx];
        (*stored == entity).then_some(value)
    }

    pub fn get_mut(&mut self, entity: EntityId) -> Option<&mut T> {
        let slot = entity.index as usize;
        let dense_idx = self.sparse.get(slot).copied().flatten()? as usize;
        let (stored, value) = &mut self.dense[dense_idx];
        (*stored == entity).then_some(value)
    }

    /// Paired mutable access to the value and its change-detection
    /// record. Primary primitive for `Mut<T>` in `World`: the caller
    /// chooses when (or whether) to bump the tick.
    pub fn get_mut_with_ticks(
        &mut self,
        entity: EntityId,
    ) -> Option<(&mut T, &mut ChangeTicks)> {
        let slot = entity.index as usize;
        let dense_idx = self.sparse.get(slot).copied().flatten()? as usize;
        if self.dense[dense_idx].0 != entity {
            return None;
        }
        // Split borrows across the disjoint `dense` and `ticks` fields.
        let value = &mut self.dense[dense_idx].1;
        let ticks = &mut self.ticks[dense_idx];
        Some((value, ticks))
    }

    pub fn ticks(&self, entity: EntityId) -> Option<ChangeTicks> {
        let slot = entity.index as usize;
        let dense_idx = self.sparse.get(slot).copied().flatten()? as usize;
        if self.dense[dense_idx].0 != entity {
            return None;
        }
        Some(self.ticks[dense_idx])
    }

    pub fn ticks_mut(&mut self, entity: EntityId) -> Option<&mut ChangeTicks> {
        let slot = entity.index as usize;
        let dense_idx = self.sparse.get(slot).copied().flatten()? as usize;
        if self.dense[dense_idx].0 != entity {
            return None;
        }
        Some(&mut self.ticks[dense_idx])
    }

    pub fn iter(&self) -> impl Iterator<Item = (EntityId, &T)> + '_ {
        self.dense.iter().map(|(e, v)| (*e, v))
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = (EntityId, &mut T)> + '_ {
        self.dense.iter_mut().map(|(e, v)| (*e, v))
    }

    /// Mutable triplet iterator: value and change-ticks side-by-side.
    /// Primary primitive for `World::iter_mut`, which wraps each triplet
    /// in a `Mut<T>` so DerefMut semantics carry into bulk iteration.
    pub fn iter_mut_with_ticks(
        &mut self,
    ) -> impl Iterator<Item = (EntityId, &mut T, &mut ChangeTicks)> + '_ {
        // Split borrow across disjoint `dense` and `ticks` fields.
        self.dense
            .iter_mut()
            .zip(self.ticks.iter_mut())
            .map(|((e, v), t)| (*e, v, t))
    }

    pub fn iter_ticks(&self) -> impl Iterator<Item = (EntityId, ChangeTicks)> + '_ {
        self.dense
            .iter()
            .zip(self.ticks.iter())
            .map(|((e, _), t)| (*e, *t))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(index: u32, generation: u32) -> EntityId {
        EntityId { index, generation }
    }

    #[test]
    fn insert_new_records_value_and_initial_tick() {
        let mut set = SparseSet::<i32>::new();
        let prev = set.insert(e(0, 0), 42, 7);
        assert_eq!(prev, None);
        assert_eq!(set.get(e(0, 0)), Some(&42));
        assert_eq!(set.ticks(e(0, 0)).unwrap().last_mutation_tick, 7);
        assert_eq!(set.len(), 1);
    }

    #[test]
    fn insert_replace_returns_old_value_and_bumps_tick() {
        let mut set = SparseSet::<i32>::new();
        set.insert(e(0, 0), 42, 1);
        let prev = set.insert(e(0, 0), 99, 5);
        assert_eq!(prev, Some(42));
        assert_eq!(set.get(e(0, 0)), Some(&99));
        assert_eq!(set.ticks(e(0, 0)).unwrap().last_mutation_tick, 5);
        assert_eq!(set.len(), 1);
    }

    #[test]
    fn insert_at_sparse_gap_grows_sparse() {
        let mut set = SparseSet::<i32>::new();
        set.insert(e(100, 0), 42, 1);
        assert_eq!(set.get(e(100, 0)), Some(&42));
        assert_eq!(set.len(), 1);
    }

    #[test]
    fn remove_returns_value_and_clears_slot() {
        let mut set = SparseSet::<i32>::new();
        set.insert(e(0, 0), 42, 1);
        assert_eq!(set.remove(e(0, 0)), Some(42));
        assert!(!set.contains(e(0, 0)));
        assert_eq!(set.get(e(0, 0)), None);
        assert_eq!(set.len(), 0);
    }

    #[test]
    fn remove_of_stale_entity_returns_none() {
        let mut set = SparseSet::<i32>::new();
        set.insert(e(0, 1), 42, 1);
        assert_eq!(set.remove(e(0, 0)), None);
        // Original entry still intact.
        assert_eq!(set.get(e(0, 1)), Some(&42));
    }

    #[test]
    fn remove_of_absent_entity_returns_none() {
        let mut set = SparseSet::<i32>::new();
        assert_eq!(set.remove(e(5, 0)), None);
        set.insert(e(0, 0), 1, 1);
        assert_eq!(set.remove(e(99, 0)), None);
    }

    #[test]
    fn contains_flips_across_insert_and_remove() {
        let mut set = SparseSet::<i32>::new();
        assert!(!set.contains(e(0, 0)));
        set.insert(e(0, 0), 1, 1);
        assert!(set.contains(e(0, 0)));
        set.remove(e(0, 0));
        assert!(!set.contains(e(0, 0)));
    }

    #[test]
    fn get_does_not_bump_tick() {
        let mut set = SparseSet::<i32>::new();
        set.insert(e(0, 0), 42, 3);
        let _ = set.get(e(0, 0));
        assert_eq!(set.ticks(e(0, 0)).unwrap().last_mutation_tick, 3);
    }

    #[test]
    fn get_mut_does_not_bump_tick() {
        let mut set = SparseSet::<i32>::new();
        set.insert(e(0, 0), 42, 3);
        if let Some(v) = set.get_mut(e(0, 0)) {
            *v = 100;
        }
        assert_eq!(set.get(e(0, 0)), Some(&100));
        assert_eq!(set.ticks(e(0, 0)).unwrap().last_mutation_tick, 3);
    }

    #[test]
    fn get_mut_with_ticks_yields_paired_refs_for_explicit_bump() {
        let mut set = SparseSet::<i32>::new();
        set.insert(e(0, 0), 42, 3);
        let (v, t) = set.get_mut_with_ticks(e(0, 0)).unwrap();
        *v = 100;
        t.last_mutation_tick = 10;
        assert_eq!(set.get(e(0, 0)), Some(&100));
        assert_eq!(set.ticks(e(0, 0)).unwrap().last_mutation_tick, 10);
    }

    #[test]
    fn swap_remove_preserves_sparse_dense_mapping() {
        let mut set = SparseSet::<i32>::new();
        set.insert(e(0, 0), 10, 1);
        set.insert(e(1, 0), 20, 1);
        set.insert(e(2, 0), 30, 1);
        // Remove middle — last entry swaps into the removed dense slot.
        assert_eq!(set.remove(e(1, 0)), Some(20));
        assert_eq!(set.get(e(0, 0)), Some(&10));
        assert_eq!(set.get(e(2, 0)), Some(&30));
        assert_eq!(set.len(), 2);
    }

    #[test]
    fn iter_visits_all_live_entries() {
        let mut set = SparseSet::<i32>::new();
        set.insert(e(0, 0), 10, 1);
        set.insert(e(1, 0), 20, 1);
        set.insert(e(2, 0), 30, 1);
        set.remove(e(1, 0));
        let mut collected: Vec<_> = set.iter().map(|(e, v)| (e.index, *v)).collect();
        collected.sort();
        assert_eq!(collected, vec![(0, 10), (2, 30)]);
    }

    #[test]
    fn iter_mut_allows_in_place_mutation() {
        let mut set = SparseSet::<i32>::new();
        set.insert(e(0, 0), 10, 1);
        set.insert(e(1, 0), 20, 1);
        for (_, v) in set.iter_mut() {
            *v *= 2;
        }
        assert_eq!(set.get(e(0, 0)), Some(&20));
        assert_eq!(set.get(e(1, 0)), Some(&40));
    }

    #[test]
    fn iter_ticks_yields_all_records_with_ticks() {
        let mut set = SparseSet::<i32>::new();
        set.insert(e(0, 0), 10, 5);
        set.insert(e(1, 0), 20, 9);
        let mut collected: Vec<_> = set
            .iter_ticks()
            .map(|(e, t)| (e.index, t.last_mutation_tick))
            .collect();
        collected.sort();
        assert_eq!(collected, vec![(0, 5), (1, 9)]);
    }
}
