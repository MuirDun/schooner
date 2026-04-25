# ECS Query systm

## 0. Why this exists

In an ECS, a system needs to say: *"give me all entities with `Pos` and `Vel`, mutating `Pos`, except those with `Frozen`."* This module is the type-safe, alias-checked surface for that ask — it turns `Query<(&mut Pos, &Vel), Without<Frozen>>` into an iterator that obeys Rust aliasing despite the world holding type-erased storages.

## 1. Essentially

A typed relational projection — `SELECT proj FROM entities WHERE filter` — whose shape is encoded at the type level by two trait families, with a join planner that picks the smallest "table" to drive iteration.

## 2. Structure

```
        Query<D, F>
         /       \
  QueryData    QueryFilter        both share shape:
 (projection)  (presence-only)      State  →  Fetch<'w>  →  per-entity op
         \       /
          \     /
        QueryAccess = Vec<(ComponentId, mutable: bool)>
              │
              ▼
        check_no_alias            ◄── invariant gate; precedes ALL unsafe
              │
              ▼
        split_storages : &'w mut World → Vec<StorageHandle<'w>>
              │
              ▼
        D::init_fetch / F::init_fetch  (consume handles in decl. order)
              │
              ▼
        Join : smallest required storage → Vec<EntityId>
              │
              ▼
        for e in driver:
          F::matches(e)  ∧  D::fetch(e)   ⟹   yield D::Item<'w>
```

Entities:

- **`QueryData`** — typeclass for projections. Impls: `&T`, `WriteOnly<T>`, tuples `(D1, D2, …)`. Each carries `State` (resolved `ComponentId`s + tick for writes), `Fetch<'w>` (typed storage handles), `Item<'w>` (the yielded shape).
- **`QueryFilter`** — typeclass for presence predicates; same `State`/`Fetch<'w>`/`matches` shape, but access is always read-only. Impls: `()`, `Without<T>`, tuples.
- **`QueryAccess`** — runtime list of `(ComponentId, mutable)`. The shared currency that the alias check, the join, and (eventually) a dynamic `world.query_dyn(&[ids])` API all consume. Typed and dyn share the same machinery — only the surface differs.
- **`StorageHandle<'w>`** — `Read(&'w dyn) | Write(&'w mut dyn)`. The bridge from type-erased boxes in the world to typed `Fetch`. Always consumed by value; reborrowing it would alias.
- **`Join`** — owns *only* `Vec<EntityId>`, no live storage refs. The decoupling matters: a cached `&dyn` to the same box `Fetch` holds `&mut` over would be Stacked-Borrows UB.
- **`QueryIter<'w, D, F>`** — the iterator user code sees.

## Invariant

(load-bearing): every `unsafe` block is dominated by `check_no_alias(data ∪ filter)`. There are exactly two unsafe sites (`split_storages`, `handle_as_read/write`). Per-entity iteration is fully safe.

**Subtle point:** the driver yields *candidate* ids from the smallest required storage; presence of every *other* required component falls out of `D::fetch`'s `?`-chain. So `(&Pos, &Vel)` with 1 Pos and 32 Vels costs O(1), not O(32).

## Rust concepts required to read this:

- **GATs** — `type Fetch<'w>`, `type Item<'w>` (associated types parameterised by a lifetime introduced at use-site).
- **Newtype-wrapped `&mut T`** — `WriteOnly<T>` exists because a blanket `impl QueryData for &T` would conflict with `impl … for &mut T` on the trait solver level.
- **`dyn Trait` + `Any` downcast** — how type-erased storage in `World` is typed back into `SparseSet<T>` at fetch time.
- **Lifetime extension via `transmute`** — `downcast_ref` reborrows under `&self` and shortens `'w`; the transmute restores it.
- **Stacked Borrows** — explains why `Join` collects entity ids and drops the storage borrow before the typed `Fetch` is built.
- **`PhantomData<fn() -> T>`** — variance / `Send`+`Sync` posture on type-only markers like `WriteOnly<T>` and `Without<T>`.
- **Audited-unsafe pattern** — pre-flight invariant proof, then localised `unsafe` whose soundness reduces to the proven invariant.
