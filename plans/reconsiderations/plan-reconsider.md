# Memo to Lead Architect: Rethinking ECS Foundation Before Game 0

## Context

I've been exploring the ECS architecture question in depth, and I've arrived at conclusions that meaningfully diverge from our current Game 0 plan. Before we commit to archetype storage as the foundation, I want to put the alternative analysis on the table so we can discuss it together. My concern is not that the current plan is wrong — it's that several foundational decisions locked in during Game 0 may be actively misaligned with what Games 2–4 will actually need, and I'd rather surface this friction now than refactor under pressure later.

## The Core Tension

Our current Game 0 plan picks **archetype-based ECS** with the reasoning that bulk iteration (rendering, physics sync, utility-AI evaluation) is the dominant workload, and that AI state transitions can be managed by mutating enum values inside a stable component rather than adding/removing marker components.

That reasoning is internally consistent, but it rests on assumptions that look fragile when I examine the full trajectory toward Game 4:

**Assumption 1: "Component shape stays stable at runtime."** This holds only if we commit to the enum-inside-component discipline *everywhere*. The moment Game 4's LOD system needs to hydrate/dehydrate NPCs — adding ~10–15 components (Velocity, Mesh, Animator, Collider, FullAIBrain, Buffs, SoundEmitter…) when the player approaches, removing them when the player leaves — we are paying archetype migration cost on a large fraction of entities every time the player crosses a zone boundary. This isn't a marginal buff/debuff case; it's a structural LOD mechanism, and it fights archetype storage directly.

**Assumption 2: "Utility AI iteration speed dominates."** True for the inner scoring loop, but the systems surrounding it — event reactions, proximity triggers, relationship graph traversals, state transitions — are not bulk iterations. They're sparse, reactive, cascade-driven. The total frame budget is shared between both, and optimizing only for the bulk-iteration half at the cost of making the reactive half painful is a questionable tradeoff given our stated goals.

**Assumption 3: "Hybrid is a non-breaking migration path."** Bevy's sparse-set annotation suggests this, but the deeper issue is that the *developer mental model* calcifies around archetype patterns during Games 0–2. Switching storage strategy for a subset of components is mechanical; switching the surrounding design idioms (how systems are written, how behaviors are expressed, how the scripting language binds to entities) is not.

## What I Think We're Actually Building

The bigger picture that crystallized for me is that the ECS isn't just a data structure — it's **the runtime our custom scripting language targets**. Given that:

- Most game logic, UI, and AI will live in our language, not Rust
- The language is interpreted during development, bytecode-compiled for production
- REPL during gameplay is a hard requirement
- In order for development to be nice and smooth, what about making the design philosophy a Lisp-like: joy of malleability, seamless debugging, rapid prototyping
- So behaviors would be naturally expressed as reactive declarations (`when (hp < 30) …`, `on :pack-member-died …`)

…the ECS layer the scripting language sees should look like an **open, inspectable bag of properties per entity**, not a rigid table row. That's a sparse-set shape, not an archetype shape. Archetype storage can still exist underneath as a hot-path cache for physics and rendering — but it shouldn't be the primary model the language or developers interact with.

## Proposed Architectural Reframe

The architecture I'd like to discuss looks like this, bottom to top:

1. **Sparse-set component storage** as the primary model. Entity = ID. Components attached independently. O(1) add/remove with no migration. LOD hydration/dehydration becomes trivial.
2. **Reactive observation layer** on top. Per-component change detection feeds subscriptions and event listeners. This is the substrate the scripting language binds to — and it matches the mental model I already work well in (I built reroi on these principles).
3. **Relationship graph** as a first-class structure alongside components, not hacked into them — for group membership, social knowledge, inventory, spatial containment.
4. **Hot-path dense caches** as an invisible optimization. Physics and rendering systems read from packed `[Position, Velocity]` or `[Position, Mesh, Anim]` arrays that sync from sparse-sets via dirty flags. The scripting language never sees these; they're a rendering/physics implementation detail.

The important inversion here: in our current plan, archetype is the foundation and sparse-set is a mitigation we add if profiling demands it. In the proposed plan, sparse-set is the foundation and dense packing is an optimization we add where profiling demands it. Both are valid; the question is which direction we want the default pressure to push.

## Open Questions We Need to Resolve Together

These are the design questions that matter most, and I don't think any of them should be decided unilaterally:

- **Reactive cascade semantics.** When a component change fires a reaction that changes another component, do we propagate synchronously (simple, debuggable, can frame-spike), deferred across ticks (smooth, but feels laggy), or budget-based (sophisticated, harder to implement)? This deeply shapes how the game *feels*.
- **Scripting language ↔ ECS binding.** How much of the entity model does the language see? My inclination is that entities appear to the language as open maps, and behaviors are themselves data the interpreter evaluates (so the REPL can redefine them live). This requires the ECS to expose a very different surface than a typical Rust ECS crate would.
- **LOD continuity fidelity.** When a dehydrated wolf "kills a deer" abstractly and the player later walks back into the zone, do we reconcile state with high fidelity (world feels genuinely persistent), plausible illusion (cheap, most games do this), or a hybrid tied to narrative importance? This affects how much infrastructure the abstract→concrete reconciliation layer needs.

## Specific Challenges to the Game 0 Plan

With that framing, here's where I think the Game 0 plan needs pressure-testing:

**The ECS storage choice is presented as resolved, but it isn't really.** Section 1.1 picks archetype and lists the hybrid as a future escape hatch. I think we should pick sparse-set-first with dense packing as the future optimization.

**The ECS API surface in §3.3 is too archetype-shaped.** `Query<(&A, &mut B)>` as the primary query pattern is a Bevy-ism that assumes compile-time-known component sets. Our scripting language will need to query entities by runtime-dynamic component presence — "give me everything that currently has a State component equal to :hunting." Designing the query API around static tuples now will create friction when we integrate scripting in Game 2. I'd rather the Game 0 query API be a deliberate minimal subset that leaves room for both static-typed Rust queries *and* dynamic runtime queries, than one that assumes the static shape is primary.

**Change detection and events are deferred, but they're the substrate of the reactive layer.** The plan says "no change detection, no events… add in Game 1 when physics forces the issue." I'd argue the opposite: change detection is the reactive layer's foundation, and adding it in Game 1 as an afterthought will make it awkward. Either we build a minimal per-component dirty-flag mechanism in Game 0 (cheap, doesn't need to be wired to anything yet), or we acknowledge that the reactive layer is a Game 2 addition and design the ECS API to accommodate it without breakage.

**The Game 0 done-bar is scoped correctly, but the architectural decisions embedded in it have Game 4 consequences.** The window, the renderer, the camera, the input layer — all fine as scoped. The ECS surface, the entity/component model, and the query API are where decisions ripple forward. I'd like us to treat those three as the genuinely load-bearing decisions and give them more design scrutiny than the rest.

## What I'm Proposing

Not a rewrite of the plan. A conversation and possibly a short detour before Phase C (ECS v1) begins:

1. **Challenge the approach**: First we need to decide, is Lisp-like design philosophy, sparse-set with reactivity architecture would survive our ambitions.
3. **Extend the Game 0 "Critical Design Decisions" list** with the questions above — cascade semantics, scripting binding model, LOD continuity. These don't all need to be resolved before Phase C, but they need to be *named* so we're not blindsided later.
4. **Reshape §3.3's query API** to be a minimal subset that doesn't pre-commit to static tuple queries as the only pattern. Even if we build only static queries in Game 0, the internal design should leave room for dynamic ones.

The guiding principle — and I think we're aligned on this already — is that Game 0's job is to be a solid foundation that *doesn't hurt to change*. I'm proposing we examine which parts of the current foundation are quietly load-bearing for Game 4 and make sure those specific parts are the most revisitable, not the most committed-to.

Happy to talk through any of this whenever you have time. The spike alone would resolve most of my uncertainty and is cheap.
