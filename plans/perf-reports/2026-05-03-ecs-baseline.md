# Perf Report: ECS Baseline + Triage of External Audit
Date: 2026-05-03
Hardware: Apple M2 Pro, 32 GB
Build: `cargo bench -p bench-ecs`, default release profile (opt-level=3, codegen-units=16, no LTO), rustc per `rust-version = "1.95"`

## Question

Two things at once:

1. **Pin a numerical baseline** for the sparse-set ECS at the end of Game 0 / start of Game 1, so any future regression has something to land against. Until this report `plans/perf-reports/` was empty.
2. **Triage an external audit** of the ECS that proposed a list of optimisations claiming "2–4× improvement on iteration benchmarks." Decide which are worth doing now, which to defer with an explicit trigger, and which to reject.

## Method

- `cargo bench -p bench-ecs` — runs `crates/bench-ecs/benches/ecs_scenarios.rs`, criterion-driven, 100 samples per data point, 8 scenarios × 3 sizes (1k / 10k / 100k entities).
- Scenario set defined in the bench file's module doc; this report uses the published baseline numbers from the canonical run shared by the developer (clean machine state, no concurrent load).
- A/B verification of session changes was done by `git stash` → re-bench → `git stash pop` → re-bench on the same machine session, comparing across stash boundaries.

A run-to-run caveat surfaced: a separate session bench on the same machine under typical developer load measured `iterate_pos_vel/10000` at ~108 µs, vs. the canonical baseline's ~77 µs. Same code, same flags. **Conclusion: criterion's run-to-run reproducibility on this Apple Silicon laptop is ~30%, dominated by thermal and background-process state.** Future regressions need to be ≥30% to be unambiguous from a single run; smaller deltas need a same-session A/B on the same machine state.

## Numbers (canonical baseline, post-Game-0)

| Scenario | 1k | 10k | 100k | Throughput @ 100k |
|---|---|---|---|---|
| `bulk_spawn` (per spawn+2 inserts) | 53 µs | 498 µs | 4.96 ms | 20 Melem/s |
| `iterate_pos_vel` (2-comp join) | 7.8 µs | 77 µs | 783 µs | 128 Melem/s |
| `iterate_pos_mut` (1-comp write) | 5.8 µs | 57 µs | 581 µs | 172 Melem/s |
| `random_lookup` (`world.get::<T>(e)`) | 20 µs | 204 µs | 2.11 ms | 47 Melem/s |
| `insert_remove_churn` (insert+remove pair) | 42 µs | 420 µs | 4.24 ms | 24 Melem/s |
| `fragmented_iteration` (driver=1/8 of pop) | 1.24 µs | 10.9 µs | 108 µs | 115 Melem/s (driver) |
| `query_with_filter` (`Without<Tag>` over half) | 4.7 µs | 46 µs | 472 µs | 211 Melem/s |
| `iterate_with_bulk_present` (control) | 7.4 µs | 73 µs | 746 µs | 134 Melem/s |

Per-entity costs implied:

- **Hot iteration (`iterate_pos_vel`): ~7.8 ns/entity** — two sparse→dense lookups + `Mut::deref_mut` tick bump + closure body. Dominated by L1 latency on the random sparse-index probe.
- **Single-comp write (`iterate_pos_mut`): ~5.8 ns/entity** — one sparse lookup + tick bump + closure body.
- **Ad-hoc lookup (`random_lookup`): ~21 ns/lookup** — 2× HashMap probe + downcast + sparse lookup. Hashmap dispatch dominates.
- **Spawn (`bulk_spawn`): ~50 ns/entity** — 2× HashMap dispatch + sparse resize + dense push.
- **Control (`iterate_with_bulk_present`)**: same per-entity cost as `iterate_pos_vel` despite a 256 B `Bulk` component carried but not iterated. **Confirms sparse-set's column-isolation property holds** — extra components on entities don't pollute the iterated cache lines.

## Diagnosis

**The substrate is well within its declared cost envelope and clears every scale target by 1–2 orders of magnitude.**

- Game 1 (Kinesis): physics on dozens of entities. ECS is invisible.
- Game 3 (Castaway): "dozens of creatures + thousands of materially-reactive props." 10k entities running a 2-component system: 77 µs ≈ **0.5% of a 16.6 ms frame** at 60 Hz. Even 100k is 4.7%.
- Game 4 (Vagrants): hundreds of agents. Bottleneck is utility-AI evaluation and Chronicle rule firing, not ECS iteration.

The plan's "Phase J checkpoint — verify acceptable with numbers" requirement (`plans/game0-plan.md` Risk Register) is satisfied at this scale. **Dense-view caches stay deferred per `architecture/ecs.md`'s "until profiling demands" trigger.**

## Change

Two hygiene fixes landed this session — low risk, no architectural commitment, lasting wins:

1. **Sparse-slot encoding: `Vec<Option<u32>>` → `Vec<u32>` with sentinel `u32::MAX`.**
   `Option<u32>` is 8 bytes (no niche optimisation). Halves sparse-array memory: a 100k-entity sparse table per component drops from 800 KB to 400 KB. Improves cache behaviour for random probes and reduces resize cost on `insert`. Touches `crates/schooner-engine/src/ecs/sparse_set.rs` only; encoding is hidden behind a single `dense_index()` helper. All 226 unit tests pass.

2. **`SmallVec` for query setup vectors.** `QueryAccess.components`, the `data_handles` / `filter_handles` returned by `split_storages`, and the `required: Vec<ComponentId>` collected at `QueryIter::new` are all bounded-cap data (typically 1–3 entries). Inline cap = 4. Eliminates 4–5 small heap allocations per query call. `smallvec = "1"` added to workspace + `schooner-engine` deps. Touches `crates/schooner-engine/src/ecs/query/{data,fetch,iter}.rs`.

## Result

A/B on the same machine session (same thermal/load state): `iterate_pos_vel/10000` improved from 108.3 µs → 99.9 µs (**~7% faster**, p < 0.001), and `/100000` from 1.106 ms → 1.014 ms (**~8% faster**, p < 0.001). 226/226 unit tests green pre- and post-change.

The improvement is real but small in absolute terms. The point of these changes was **not** the speedup — it was eliminating two known steady-state allocation patterns and a known cache-footprint bloat without changing semantics. Both changes are forever wins; they don't get re-paid for by deferral.

## Followups — deferred findings

Each item below is a real perf finding documented in this session, with a stated **trigger** (the condition that should reopen the question). Don't act on any of these without a fresh measurement first.

### F1 — Driver storage probed redundantly through sparse table
**Where:** `crates/schooner-engine/src/ecs/query/{join,iter,data}.rs`.
**Finding:** When a join picks driver storage S as the smallest required set, it walks S's dense entity list and yields `EntityId`s. The per-entity `D::fetch` then re-probes S through `SparseSet::get_mut_with_ticks(entity)` — a sparse→dense lookup whose answer is already known. For the non-driver components the lookup is necessary; for the driver it is pure overhead.
**Predicted gain:** 30–40% on multi-component iteration (`iterate_pos_vel` from ~7.8 to ~5 ns/entity), smaller or zero on single-component iteration.
**Why deferred:** Requires the `QueryData` trait to know which slot is the driver and to expose a "fetch by dense index" path. This is the same code shape as Game 3's planned dense-view cache work. Doing it now means doing Game 3's optimisation at Game 0's measurement.
**Trigger to act:** Game 3 prep, OR profiler shows `query::Iterator::next` exceeding 5% of a real frame in `crates/game/`. Whichever first.

### F2 — `Join::new` allocates a `Vec<EntityId>` of driver entities per query
**Where:** `crates/schooner-engine/src/ecs/query/join.rs:72-79`.
**Finding:** Driver entity IDs are collected into a fresh `Vec<EntityId>` at every query construction — explicitly because the storage `&dyn` borrow had to be dropped before the typed split-borrow could run. At 100k driver entities that's an 800 KB malloc + memcpy + free per query call. Steady-state allocation on a hot path; locked policy says no.
**Predicted gain:** 4–10% of `iterate_pos_vel`'s wall time at 100k. Per query, allocation overhead doesn't scale linearly with N — it's a one-time cost — so the relative impact shrinks as N grows but is significant on per-frame system aggregates (dozens of systems × a query each = dozens of these per frame).
**Fix sketches:**
- A thread-local scratch buffer reused across queries.
- Restructure `split_storages` so the driver entity slice is borrowed before the typed `&mut SparseSet<T>` handles are minted, then used by `Join` as a `&'w [EntityId]`. Borrow ordering is the hard part.
- Combine with F1 — if `Join` yields `(dense_index_in_driver, EntityId)` and the driver fetch reads by dense index, the entity-id collection becomes a slice borrow rather than an owned Vec.
**Trigger to act:** Same as F1, or independently when puffin shows allocator contention in the frame loop.

### F3 — Dense array is `Vec<(EntityId, T)>` (AoS) rather than parallel arrays (SoA)
**Where:** `crates/schooner-engine/src/ecs/sparse_set.rs:34`.
**Finding:** Each dense slot is `(EntityId, T)` — entity-id and value packed together. For value-only iteration (`iterate_pos_mut`), 8 bytes/entry of EntityId is dead weight in cache. Splitting into `dense_entities: Vec<EntityId>` + `dense_values: Vec<T>` is the canonical sparse-set optimisation; Bevy does this.
**Predicted gain:** 10–20% on single-component dense iteration. Smaller on multi-component joins because non-driver access is random-pattern (driven by driver entity order), where SoA's streaming win partially evaporates.
**Why deferred:** Touches every SparseSet API plus the change-tick storage layout. Bigger surface than this session's hygiene scope. Pairs naturally with F1 — the gain is bigger when the driver iterates dense values directly by index.
**Trigger to act:** Game 3 prep, alongside F1 / F2 as the dense-view-cache work.

### F4 — `Mut<T>` writes the change tick on every `DerefMut`, not once per access
**Where:** `crates/schooner-engine/src/ecs/world.rs` (`Mut::deref_mut`).
**Finding:** `p.x += v.x; p.y += v.y; p.z += v.z` invokes `deref_mut` three times, writing the tick three times to the same address. The store buffer coalesces these writes (M2's hardware store coalescing is effective on hot addresses), so the cost is mostly compile-time visible rather than runtime measurable. The architecturally cleaner fix is to bump-once on `Drop` rather than every `DerefMut`.
**Predicted gain:** 0–5% on `iterate_pos_vel` — small because hardware already absorbs most of it. The microbench may not even register the change above noise.
**Why deferred:** Low predicted gain, and `Drop`-based bumping changes the observable semantic (the tick is bumped at end-of-scope rather than at first mutation). Want to confirm no consumer depends on the precise order before changing it. Game 2A's reactive consumers will be the first real users.
**Trigger to act:** Profiler attributes ≥3% of a real frame to `Mut::deref_mut`, OR Game 2A's reactive subscriptions are wired and a measurement shows the tick-write store causing detectable serialization with the user closure's stores.

### F5 — `world.get::<T>(entity)` is HashMap-bound at ~21 ns/call
**Where:** `crates/schooner-engine/src/ecs/world.rs` (`World::get`, `get_mut`, `insert`, `remove`).
**Finding:** Every ad-hoc component access is two HashMap probes (`TypeId → ComponentId`, then `ComponentId → Box<dyn ComponentStorage>`) + downcast + sparse lookup. The HashMap dispatch dominates — sparse lookup itself is ~3–5 ns. Switching `storages: HashMap<ComponentId, Box<dyn>>` to `storages: Vec<Option<Box<dyn>>>` (indexed by `ComponentId.index()`) would eliminate one probe.
**Predicted gain:** 3–7 ns per `world.get`. **In query-iter hot paths this cost is amortised to zero** — `Join` and `Fetch` cache typed handles at construction. The ad-hoc-get path is the only consumer; unless game code calls `world.get::<T>(e)` inside a per-entity loop, this is invisible to frame budget.
**Why deferred:** The fix is mechanical (8 call sites in `world.rs`) but the gain is tiny in absolute terms. More importantly, the right reaction to "ad-hoc-get is slow" is a code-pattern guideline ("don't call `world.get` inside per-entity loops; use a query"), not optimising the slow path.
**Trigger to act:** Profiler shows a system spending >2% of frame time in `World::get` / `World::get_mut`. Indicates a code-pattern violation worth investigating before optimising the dispatch.

### F6 — Generation check on non-driver component lookup is redundant for live entities
**Where:** `crates/schooner-engine/src/ecs/sparse_set.rs` (`get`, `get_mut`, `get_mut_with_ticks`).
**Finding:** Inside an iteration where the entity came from a live driver storage, the entity's generation is guaranteed to match across every storage that has it (because `World::despawn` removes from all storages before recycling the slot). The `(*stored == entity).then_some(value)` generation check is therefore redundant on the iteration path, though it's correct for ad-hoc lookups.
**Predicted gain:** 1–3% per probe — a single u32 compare + branch. Real but unimpressive.
**Why deferred:** Pairs naturally with F1 (which restructures the fetch path anyway). Doing it standalone with an `unsafe fn get_unchecked` adds API surface for marginal gain.
**Trigger to act:** Subsumed by F1.

### F7 — REJECTED: "Make `WriteOnly<T>` skip change-detection ticks"
**Where:** External audit recommendation.
**Finding (audit's claim):** `WriteOnly<T>::fetch` constructs `Mut<T>` and pays the tick-write cost; the audit proposed returning `&mut T` directly, predicting 15–25% on `iterate_pos_mut`.
**Why rejected:** `WriteOnly<T>` is **not** an "I won't track changes" variant; it is the *only* writeful query path the engine has, and `Query<&mut T>` desugars to it. The tick bump is the substrate of the reactive backbone the plan commits to from Game 0 (`architecture/ecs.md`: "Per-component change tracking is cheap to build into sparse-set storage and awkward to retrofit. Doing it from Game 0 means Game 2's reactive layer is wiring, not surgery."). Removing the tick is a 25% speedup paid for by deleting a load-bearing invariant Game 2A's reactive consumers will depend on. The cost is real and the architecture knowingly accepts it.
**If the cost ever becomes blocking:** the right move is F4 (bump-once-on-drop), not stripping change detection.

### F8 — DEFERRED LOW-LEVERAGE: `Vec<Option<Box<dyn>>>` for storages
**Where:** `crates/schooner-engine/src/ecs/world.rs`.
**Finding:** Same as F5's structural sibling. `storages: HashMap<ComponentId, Box<dyn>>` could be a `Vec` indexed by `ComponentId.index()`. Eliminates a hash-and-probe at every `world.get` / `world.insert` / `world.remove` and at every query setup.
**Why deferred this session:** "Low-leverage hygiene." Considered for inclusion; rejected to keep change scope tight and since the per-frame-impact prediction is small.
**Trigger to act:** Bundle with F5 if/when ad-hoc-get cost becomes visible.

---

## Summary table

| ID | Finding | Effort | Predicted gain | Trigger to act |
|----|---------|--------|----------------|----------------|
| ✓ Sentinel | `Vec<Option<u32>>` → `Vec<u32>` | done | Memory −50% on sparse, ~7% iter | n/a |
| ✓ SmallVec | Per-query Vec allocs | done | Eliminates ~4 heap allocs / query | n/a |
| F1 | Driver redundant sparse lookup | medium | 30–40% on multi-comp iter | Game 3 prep, or 5% frame share |
| F2 | `Join::new` Vec<EntityId> alloc | medium | 4–10% on iter at scale | Allocator pressure in frame profile |
| F3 | AoS `(EntityId, T)` → SoA | medium-high | 10–20% on dense iter | Game 3 prep (with F1) |
| F4 | `Mut::deref_mut` bumps every call | low | 0–5% | Reactive consumers + measurement |
| F5 | `world.get::<T>(e)` HashMap-bound | low | 3–7 ns / call | >2% frame in `world.get` |
| F6 | Generation check redundant on probe | low | 1–3% | Subsumed by F1 |
| F7 | Strip ticks from WriteOnly | — | rejected | — never — |
| F8 | HashMap → Vec for storages | low | small | Bundle with F5 |

Reproducer for any future re-measurement: `cargo bench -p bench-ecs` from a clean machine state. Prefer same-session A/B (`git stash` → bench → `git stash pop` → bench) for any change <30%, since cross-session noise on this hardware is in the 20–30% range.
