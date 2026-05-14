//! ComponentId-based driver selection.
//!
//! Given a list of required [`ComponentId`]s, picks the smallest
//! sparse-set as the driver and yields its entity ids in dense
//! order. The downstream [`QueryData`](crate::ecs::query::data::QueryData)
//! `fetch` does the per-entity presence-and-value probe through its
//! typed storage handles.
//!
//! ## Why driver state owns no live storage references
//!
//! Earlier C9.2 cached `&dyn ComponentStorage` probe references in
//! the join. That was sound for read-only queries, but for `Query<&mut T>`
//! the typed `Fetch` holds `&'w mut SparseSet<T>` over the same box
//! the join's `&'w dyn` would point at — `&` and `&mut` aliasing UB.
//! Collecting the driver's entity ids into a `Vec<EntityId>` at
//! construction drops the storage borrow before any typed `Fetch`
//! is built, so the typed handles are the only live references to
//! the storage during iteration.
//!
//! ## Empty-result rules
//!
//! - `required` is empty → empty driver. We don't invent a "match
//!   every entity" semantics for that case.
//! - Any required id has no registered storage, or its storage is
//!   empty → empty driver.
//! - Otherwise: pick the smallest required storage; its dense
//!   entity list becomes the driver.
//!
//! The downstream `D::fetch` returns `None` for entities the driver
//! includes but a probe-side component happens to be missing —
//! that case shouldn't arise (driver picks the storage with the
//! fewest entities, but those entities still have to be present in
//! every other required storage). The `?` chain in `D::fetch` is
//! the load-bearing presence check.

use crate::ecs::{ComponentId, EntityId, World};

/// Pre-resolved driver entity stream for a query.
///
/// Owns no references — just a `Vec<EntityId>`. Built once at
/// query construction; consumed by the iterator.
pub struct Join {
    entities: std::vec::IntoIter<EntityId>,
}

impl Join {
    /// Build a join over `required` ids. Empty join if any required
    /// component is missing or empty, or if `required` is empty.
    pub fn new(world: &World, required: &[ComponentId]) -> Self {
        if required.is_empty() {
            return Self::empty();
        }

        // Pick the smallest required storage as the driver. Bail if
        // any required storage is missing or empty — no entity can
        // satisfy the join.
        let mut driver_id: Option<ComponentId> = None;
        let mut driver_len = usize::MAX;
        for &id in required {
            match world.storage(id) {
                Some(s) if s.len() > 0 => {
                    let len = s.len();
                    if len < driver_len {
                        driver_id = Some(id);
                        driver_len = len;
                    }
                }
                _ => return Self::empty(),
            }
        }

        let entities: Vec<EntityId> = match driver_id {
            Some(id) => world
                .storage(id)
                .expect("non-empty above")
                .entities()
                .collect(),
            None => Vec::new(),
        };
        Self {
            entities: entities.into_iter(),
        }
    }

    fn empty() -> Self {
        Self {
            entities: Vec::new().into_iter(),
        }
    }
}

impl Iterator for Join {
    type Item = EntityId;

    fn next(&mut self) -> Option<Self::Item> {
        self.entities.next()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq)]
    struct A(i32);

    #[derive(Debug, PartialEq)]
    struct B(i32);

    fn collect_sorted(join: Join) -> Vec<u32> {
        let mut out: Vec<u32> = join.map(|e| e.index).collect();
        out.sort();
        out
    }

    #[test]
    fn empty_required_list_yields_nothing() {
        let world = World::new();
        let join = Join::new(&world, &[]);
        assert_eq!(collect_sorted(join), Vec::<u32>::new());
    }

    #[test]
    fn unknown_required_id_yields_nothing() {
        let mut world = World::new();
        let a_id = world.register_component::<A>();
        let join = Join::new(&world, &[a_id]);
        assert_eq!(collect_sorted(join), Vec::<u32>::new());
    }

    #[test]
    fn single_required_walks_that_storage() {
        let mut world = World::new();
        let e1 = world.spawn();
        let e2 = world.spawn();
        let e3 = world.spawn();
        world.insert(e1, A(1));
        world.insert(e2, A(2));
        world.insert(e3, A(3));
        let a_id = world.component_id::<A>().unwrap();
        let mut out = collect_sorted(Join::new(&world, &[a_id]));
        let mut expected = vec![e1.index, e2.index, e3.index];
        expected.sort();
        out.sort();
        assert_eq!(out, expected);
    }

    #[test]
    fn smallest_set_wins_as_driver() {
        // 32 B-bearing entities, one A-bearing → A's storage wins
        // the driver coin and the join yields just one entity.
        let mut world = World::new();
        let mut commons = Vec::new();
        for _ in 0..32 {
            let e = world.spawn();
            world.insert(e, B(0));
            commons.push(e);
        }
        let rare = commons[7];
        world.insert(rare, A(777));
        let a_id = world.component_id::<A>().unwrap();
        let b_id = world.component_id::<B>().unwrap();
        // Driver is A's set (1 entity). Output is A's entities — the
        // typed fetch in D::fetch does the per-entity probe of B.
        assert_eq!(
            collect_sorted(Join::new(&world, &[a_id, b_id])),
            vec![rare.index]
        );
    }

    #[test]
    fn empty_storage_short_circuits() {
        let mut world = World::new();
        let a_id = world.register_component::<A>();
        let e = world.spawn();
        world.insert(e, B(1));
        let b_id = world.component_id::<B>().unwrap();
        // A's storage was never created.
        assert_eq!(
            collect_sorted(Join::new(&world, &[a_id, b_id])),
            Vec::<u32>::new()
        );
    }
}
