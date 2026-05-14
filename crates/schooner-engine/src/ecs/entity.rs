/// Unique identifier for an entity.
///
/// Combines a slot index with a generation counter. When an entity is
/// despawned the slot returns to the pool with an incremented generation,
/// so any older `EntityId` referring to that slot becomes detectably
/// stale via [`EntityAllocator::is_alive`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EntityId {
    pub index: u32,
    pub generation: u32,
}

/// Allocates and recycles entity slots.
///
/// Freed slots are reused LIFO and have their generation bumped, so a
/// stale `EntityId` (pointing at a slot that has since been recycled)
/// fails `is_alive`. After 2^32 reuses of a single slot the generation
/// wraps and stale detection can collide — an accepted tradeoff given
/// the 4B-reuse budget per slot.
#[derive(Debug, Default)]
pub struct EntityAllocator {
    generations: Vec<u32>,
    free: Vec<u32>,
}

impl EntityAllocator {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn allocate(&mut self) -> EntityId {
        if let Some(index) = self.free.pop() {
            EntityId {
                index,
                generation: self.generations[index as usize],
            }
        } else {
            let index = self.generations.len() as u32;
            self.generations.push(0);
            EntityId {
                index,
                generation: 0,
            }
        }
    }

    /// Returns `true` if `entity` was alive and is now freed; `false` if
    /// it was already stale (double-free, or a fabricated id).
    pub fn free(&mut self, entity: EntityId) -> bool {
        if !self.is_alive(entity) {
            return false;
        }
        let slot = &mut self.generations[entity.index as usize];
        *slot = slot.wrapping_add(1);
        self.free.push(entity.index);
        true
    }

    pub fn is_alive(&self, entity: EntityId) -> bool {
        (entity.index as usize) < self.generations.len() // was correctly allocated
            && self.generations[entity.index as usize] == entity.generation // has current generation
    }

    pub fn len(&self) -> usize {
        self.generations.len() - self.free.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allocate_gives_sequential_indices_from_zero() {
        let mut alloc = EntityAllocator::new();
        let a = alloc.allocate();
        let b = alloc.allocate();
        assert_eq!(
            a,
            EntityId {
                index: 0,
                generation: 0
            }
        );
        assert_eq!(
            b,
            EntityId {
                index: 1,
                generation: 0
            }
        );
    }

    #[test]
    fn free_and_reallocate_recycles_slot_with_bumped_generation() {
        let mut alloc = EntityAllocator::new();
        let a = alloc.allocate();
        assert!(alloc.free(a));
        let b = alloc.allocate();
        let c = alloc.allocate();
        assert_eq!(b.index, a.index);
        assert_eq!(b.generation, a.generation + 1);
        assert_eq!(
            c,
            EntityId {
                index: 1,
                generation: 0
            }
        );
    }

    #[test]
    fn stale_entity_id_is_detected() {
        let mut alloc = EntityAllocator::new();
        let a = alloc.allocate();
        assert!(alloc.is_alive(a));
        alloc.free(a);
        let b = alloc.allocate();
        assert!(alloc.is_alive(b));
        assert!(!alloc.is_alive(a));
    }

    #[test]
    fn double_free_returns_false() {
        let mut alloc = EntityAllocator::new();
        let a = alloc.allocate();
        assert!(alloc.free(a));
        assert!(!alloc.free(a));
    }

    #[test]
    fn fabricated_entity_id_is_not_alive() {
        let alloc = EntityAllocator::new();
        assert!(!alloc.is_alive(EntityId {
            index: 42,
            generation: 0
        }));
    }

    #[test]
    fn len_tracks_live_count_across_allocations_and_frees() {
        let mut alloc = EntityAllocator::new();
        assert_eq!(alloc.len(), 0);
        assert!(alloc.is_empty());

        let a = alloc.allocate();
        let _b = alloc.allocate();
        assert_eq!(alloc.len(), 2);

        alloc.free(a);
        assert_eq!(alloc.len(), 1);

        let _c = alloc.allocate();
        assert_eq!(alloc.len(), 2);
    }
}
