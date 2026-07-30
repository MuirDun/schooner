# ECS — state, queries, scheduling, change detection

Idea doc: `plans/architecture/ecs.md`. Storage rationale (sparse-set over
archetype) is settled there; this is the build state and the Kinesis roadmap.

---

## What exists now (solid)

- **Sparse-set storage**, one `SparseSet<T>` per component type behind a
  type-erased `ComponentStorage` trait; O(1) add/remove; swap-remove dense arrays
  (`ecs/sparse_set.rs`, `ecs/storage.rs`). Textbook and well-tested.
- **Generational entities** (`EntityId { index, generation }`), allocator recycles
  slots and bumps generation so stale ids are detectable via `world.is_alive`
  (`ecs/entity.rs`).
- **Query engine** — typed tuples + `Without<T>` filter, GAT-based `Fetch`/`Item`,
  a runtime `QueryAccess` (`Vec<(ComponentId, mutable)>`) feeding a real alias
  check and storage split-borrow (`ecs/query/`). The lower half (join driver
  selection, alias, split) is already `ComponentId`-driven.
- **Resources** — type-keyed singleton bag, `Res<T>`/`ResMut<T>` (`ecs/resource.rs`).
- **Schedule** — `Startup` / `FixedUpdate` / `Update` / `Render` stages, systems
  run in registration order, exclusive `fn(&mut World)` systems supported
  (`ecs/schedule.rs`, `ecs/system.rs`). Clean single-threaded runner.
- **Change-detection ledger (mutation half only)** — `ChangeTicks { last_mutation_tick }`
  per entry; `Mut<T>` bumps on `DerefMut`; `World::changed_since::<T>(tick)`. The
  struct is deliberately shaped to grow `added_tick` without an API break.

## What's missing (the Part-2 consumer layer)

The audit's headline: the change-tick *ledger* exists but its *consumer API*
doesn't, and there is no discrete-event or deferred-op primitive. These are the
shared spine of physics events and gameplay reactivity ([events.md](events.md)).

| Gap | Why it's needed | Lands |
|-----|-----------------|-------|
| `added_tick` + `Added<T>` | distinguish "newly added" from "mutated" (lazy handle creation, on-add reactions) | Part 2 |
| `Removed<T>` / despawn signal | event-driven Rapier handle cleanup; on-remove reactions (despawn drops the record today) | Part 2 |
| `Changed<T>` **filter** | composable change-detection in a `Query` (only the standalone `changed_since` scan exists) | Part 2 |
| `Events<T>` | double-buffered discrete-event queue — collisions, triggers | Part 2 |
| `Commands` | deferred spawn/despawn/insert/remove so non-exclusive systems can mutate structure | Part 2 |
| run-conditions | `run_if(..)` so mode/verb systems run only when active | Part 2 (nicety) |
| resource change ticks | react when a *resource* (hunger, attitude) crosses a threshold | Part 4 (if needed) |

## Kinesis roadmap

- **Part 2 (Verbs):** complete Tier-1 (`added_tick`, removed ledger, `Changed`/
  `Added`/`Removed` filters); add `Events<T>` and `Commands`; add run-conditions.
  This is the substrate everything in Part 2 consumes — do it first.
- **Part 4 (Mind):** the declarative state layer leans on `Changed<T>` heavily
  (eye-state / comfort / food-appearance derive from attitude). Add resource
  change-detection if the derivations want it. No new storage work expected.
- **Later games (not Kinesis):** type-erased / `ComponentId`-keyed component
  access and `query_dyn` for the Glyph VM — explicitly Game 2A
  (`plans/architecture/ecs.md`, `glyph.md`). The lower query half already
  generalizes; the erased *row materializer* and erased *insert* are the unbuilt
  part. **Out of Kinesis scope** — do not build for it now, but don't foreclose
  it (keep `ComponentId` the currency, keep the join engine id-driven).

## Sharp risks / decisions

- **Mutation ticks fire through `Mut<T>`.** `get_mut`, `iter_mut`, and writable
  queries return this guard; reaching `DerefMut` stamps the current epoch, while
  read-only access through the same guard does not mark data dirty.
- **Per-system change epochs.** `current_tick` is a monotonic change epoch, not a
  frame or simulation counter. Every scheduled system and every non-empty
  deferred-command batch gets a distinct epoch; empty barriers get none.
  Scheduled `Added<T>` / `Changed<T>` queries own a per-system last-successful-run
  cursor, while exclusive/manual consumers keep explicit caller-owned cursors.
  Epoch exhaustion fails instead of wrapping.
- **No runtime system registration / no public `schedule_mut`.** Fine for
  Kinesis (all systems are known at build). Only relevant if a future VM wants
  dynamic Rust systems — it won't; it dispatches inside one resident system.

Cross-refs: [events.md](events.md) (how these are consumed), [physics.md](physics.md)
(the first `Events<T>` producer), `plans/plan.md` (cross-game staging).
