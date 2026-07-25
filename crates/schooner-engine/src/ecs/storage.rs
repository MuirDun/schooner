use std::any::Any;

use crate::ecs::{Component, EntityId, SparseSet};

/// Type-erased storage trait. Lets `World` hold `SparseSet<T>` for many
/// different `T` in a single `HashMap<ComponentId, Box<dyn ComponentStorage>>`.
///
/// Typed access is recovered by downcasting through [`Self::as_any`] / [`Self::as_any_mut`].
/// Only type-erased operations live on the trait —
/// insertion is excluded because it needs the concrete value type
/// and therefore always goes through the typed `SparseSet<T>` path.
pub trait ComponentStorage: Any + Send + Sync {
    /// Remove whatever value (if any) is stored for `entity`.
    /// Returns `true` if a value was actually removed.
    fn remove_entity(&mut self, entity: EntityId) -> bool;

    fn len(&self) -> usize;
    fn is_empty(&self) -> bool;
    fn contains(&self, entity: EntityId) -> bool;

    /// Iterate the entity ids this storage holds, in dense order.
    /// Type-erased entry point for the join engine: callers walk the
    /// driver's entities and probe other storages via `contains`.
    fn entities(&self) -> Box<dyn Iterator<Item = EntityId> + '_>;

    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

impl<T: Component> ComponentStorage for SparseSet<T> {
    fn remove_entity(&mut self, entity: EntityId) -> bool {
        self.remove(entity).is_some()
    }

    fn len(&self) -> usize {
        SparseSet::len(self)
    }
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn contains(&self, entity: EntityId) -> bool {
        SparseSet::contains(self, entity)
    }

    fn entities(&self) -> Box<dyn Iterator<Item = EntityId> + '_> {
        Box::new(self.iter().map(|(e, _)| e))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::ecs::{ComponentId, ComponentRegistry};

    fn e(index: u32, generation: u32) -> EntityId {
        EntityId { index, generation }
    }

    #[test]
    fn box_dyn_storage_forwards_len_and_contains() {
        let mut set: SparseSet<i32> = SparseSet::new();
        set.insert(e(0, 0), 42, 1);
        let boxed: Box<dyn ComponentStorage> = Box::new(set);
        assert_eq!(boxed.len(), 1);
        assert!(boxed.contains(e(0, 0)));
        assert!(!boxed.contains(e(1, 0)));
    }

    #[test]
    fn downcast_recovers_typed_sparse_set() {
        let mut set: SparseSet<i32> = SparseSet::new();
        set.insert(e(0, 0), 42, 1);
        let boxed: Box<dyn ComponentStorage> = Box::new(set);
        let recovered = boxed
            .as_any()
            .downcast_ref::<SparseSet<i32>>()
            .expect("same type downcast");
        assert_eq!(recovered.get(e(0, 0)), Some(&42));
    }

    #[test]
    fn downcast_to_wrong_type_returns_none() {
        let set: SparseSet<i32> = SparseSet::new();
        let boxed: Box<dyn ComponentStorage> = Box::new(set);
        assert!(boxed.as_any().downcast_ref::<SparseSet<String>>().is_none());
    }

    #[test]
    fn remove_entity_via_dyn_succeeds_then_returns_false() {
        let mut set: SparseSet<i32> = SparseSet::new();
        set.insert(e(0, 0), 42, 1);
        let mut boxed: Box<dyn ComponentStorage> = Box::new(set);
        assert!(boxed.remove_entity(e(0, 0)));
        assert_eq!(boxed.len(), 0);
        assert!(!boxed.remove_entity(e(0, 0)));
    }

    #[test]
    fn heterogeneous_storages_hold_in_hashmap() {
        let mut reg = ComponentRegistry::new();
        let int_id = reg.register::<i32>();
        let string_id = reg.register::<String>();

        let mut ints: SparseSet<i32> = SparseSet::new();
        ints.insert(e(0, 0), 42, 1);

        let mut strings: SparseSet<String> = SparseSet::new();
        strings.insert(e(0, 0), "hello".into(), 1);

        let mut storages: HashMap<ComponentId, Box<dyn ComponentStorage>> = HashMap::new();
        storages.insert(int_id, Box::new(ints));
        storages.insert(string_id, Box::new(strings));

        assert_eq!(storages[&int_id].len(), 1);
        assert_eq!(storages[&string_id].len(), 1);

        let int_set = storages[&int_id]
            .as_any()
            .downcast_ref::<SparseSet<i32>>()
            .expect("i32 storage downcasts");
        assert_eq!(int_set.get(e(0, 0)), Some(&42));

        let string_set = storages[&string_id]
            .as_any()
            .downcast_ref::<SparseSet<String>>()
            .expect("String storage downcasts");
        assert_eq!(string_set.get(e(0, 0)), Some(&"hello".to_string()));
    }
}
