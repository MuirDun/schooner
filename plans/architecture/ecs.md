# ECS — The Local Simulation Layer

The ECS is Layer 4: the here-and-now of the world the player perceives. It is not the engine's storage for everything. It is specifically the layer where bodies, physics, animations, particles, and frame-rate gameplay live. The world database (Layer 1) is a different store with different rules; do not confuse the two.

---

## What the ECS Is For

The ECS solves three problems at 60 Hz:

1. **Composable property bags.** An entity is whatever properties it currently has. A burning entity that was a moment ago a calm one differs by exactly one component. A wet entity becomes a steaming one when fire touches it. The engine and gameplay code must add and remove components freely without ceremony.
2. **Hot iteration.** Each frame, dozens of systems sweep over thousands of entities — physics integrates positions, the renderer culls and submits, particles age, AI commands apply. These sweeps must be cheap.
3. **Reactive change.** When something changes — a component is added, mutated, removed — other systems must be able to respond, often in the same frame. The reactive backbone (Tier 1) is built into the ECS storage, not bolted on.

Anything that is not this — anything persistent, world-spanning, or queried relationally — does not belong in the ECS.

---

## Why Sparse-Set Storage

The choice of sparse-set over archetype is one of the more consequential decisions in the engine, and the reasoning is multi-pronged. No single argument carries it; together they are decisive.

**The scripting layer is the long-term primary consumer.** Glyph treats entities as dynamic property bags. Glyph code adds and removes components many times per frame across many entities — status effects, interaction states, temporary markers. In archetype storage, every add or remove triggers a full-entity migration to a new archetype table. In sparse-set storage, add and remove are O(1) operations on a single component's storage. The scripting language's philosophy and the storage's behaviour need to agree, and sparse-set is the agreement.

**Immersive-sim status churn is the load-bearing case at 60 Hz.** Burning, Wet, Frozen, Stunned, Poisoned, MagicShield, OnFire, Bleeding — these flicker on and off across many entities every frame in the kind of game we are building. This is not LOD-scale, edge-case churn; it is the moment-to-moment behaviour of a living combat encounter. Archetype migration on this load is a real cost. Sparse-set absorbs it.

**Reactive subscriptions are a first-class engine idiom.** "When Health drops below 30, do X." "On Burning added, spawn fire particles." These are the substrate of how Glyph code expresses behaviour. Per-component change tracking is cheap to build into sparse-set storage and awkward to retrofit. Doing it from Game 0 means Game 2's reactive layer is wiring, not surgery.

**Hydration is across the boundary, not within the ECS.** This is worth naming explicitly because it eliminates an old argument that does not apply. When a character is hydrated from Layer 1 into Layer 4, the ECS entity is **spawned fresh** with all its components at once; when dehydrated, it is **despawned**. There is no in-place LOD migration of an existing ECS entity changing detail level. Both archetype and sparse-set handle spawn and despawn cleanly, so this is a wash for storage choice. The other reasons above stand without it.

---

## The Cost We Accept

Sparse-set iteration over multiple components is slower than archetype column scans. Joining "every entity that has Position and Velocity and Collider" requires walking the smallest sparse set and probing the others. At Game 0 entity scale this is invisible. At Game 3 outdoor scale or Game 4 NPC scale it might not be.

The mitigation is **dense view caches** — incrementally maintained mirrors of specific component packs, kept in sync via the change-detection ticks the storage already keeps. The renderer iterates a dense view; the script-facing API still sees sparse sets. This is an optimisation we add when profiling demands it, not a foundational commitment. The storage contract that scripts depend on does not change.

Dense view caches are not a Game 0 concept. Naming them now is just promising they are reachable from the storage we choose today.

---

## Change Detection as Substrate

Every component mutation through the ECS bumps a per-entity tick. Every system can ask: which entities of component type T have changed since I last looked?

This is the foundation of the reactive backbone. In Game 0 it is a substrate without consumers. In Game 2 it becomes the lever Glyph subscriptions pull. In Game 4 it feeds Tier 2 events that propagate to other layers.

The implementation detail of how change is recorded — ticks, bitsets, generations — is an implementation detail. The contract is: a system can ask, cheaply, what changed.

---

## What the ECS Is Not

Several things look ECS-shaped and are not.

- **The world database is not the ECS.** Aldric the blacksmith has a world record at all times; he has an ECS entity only when the player is near. The two stores have different shapes because they answer different questions.
- **The agent's brain is not in the ECS.** A blackboard, a utility evaluation, an HTN plan — these are richer structures than components. They live with the agent layer and are referenced from the ECS by handle.
- **Quest state is not in the ECS.** Quests are facts the world database tracks. The ECS may carry markers for "this entity is the current quest target" but the quest itself is not a component bag.
- **Persistent inventory state is not in the ECS** for distant NPCs. It lives in the world database. When the NPC hydrates, the inventory becomes ECS components on the spawned entity for the duration.

The rule is: if it lives only when the player is here, it belongs in the ECS. If it persists, it belongs in the world database, with the ECS as its temporary projection.

---

## Why This Shape Survives the Future

The ECS we are building is shaped by what comes later, even though Game 0 cannot use most of it.

- The change-detection substrate **is** the seed of the reactive backbone.
- The sparse-set add/remove ergonomics **are** what Glyph's organism philosophy demands.
- The component-id internal representation **is** what the dynamic query API will compile to when scripts ask "give me everything with Health and Burning."
- The dense-view escape hatch **is** the path the renderer takes when entity counts grow.

Game 0 builds none of those consumers. It builds the storage that none of them will fight when their time comes. That is the discipline.
