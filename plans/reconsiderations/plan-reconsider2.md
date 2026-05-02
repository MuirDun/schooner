# Game 0 done, what next?

We just finished implementation of the game 0. The backbone of the engine is done. During the development, I thought about the whole plan and architecture, talked with different specialists.

You are the chief software engineer and architector, with 20-years experience in the game dev world. Together with you we should choose the future of the engine. Here is the insights from the another chief game developer I talked with.

You need to evaluate our current plan in @plans/plan.md. Look at what have been done during the game 0: @plans/game0-plan.md. Then, take a look at the current state of the engine and game-void and how it's done.

Then, you need to analyze the insigts I will give you. What do you think about it, how solid it is. Should we just take it, or adjust for our needs, or just reject(but for me new architecture sounds pretty solid)?

Also, I speak with the staff graphics programmer, and disscussed with him the style of the game. What graphics, what techniques would be used. After we done with architecture - we would talk about that, since it is very important for plans about the renderer.

Here is the insigts on plan and architecture:

# Schooner Engine Plan — Architecture Review (Post-Discussion)

This review is written in light of what we worked out together: the five-layer world architecture, the two-language strategy (Chronicle + Glyph), the threading model, and the nuanced ECS/non-ECS split. The plan was written before that conversation, so it reflects an earlier understanding. My job here is to challenge what no longer fits, inject what's missing, and provide concrete architectural proposals for the gaps.

---

## Part 1: The ECS Decision Needs a Complete Rewrite

### The old rationale is wrong (and the new one is subtler)

The plan says sparse-set was chosen primarily because of LOD hydration/dehydration cost — avoiding archetype migration when entities change detail levels. Our layered architecture **eliminates this concern entirely**, because LOD transitions aren't component mutations on living entities. They're spawn/despawn across an architectural boundary:

```
HYDRATE:   World Database record → spawn fresh ECS entity (all components at once)
DEHYDRATE: Read ECS entity → write back to World Database → despawn entity
```

Both archetype and sparse-set handle spawn/despawn well. Archetype is actually *faster* at spawning a complete entity because you know the full component set upfront and place it directly into the right table. No concern about migration.

So the original justification collapses. But the conclusion (sparse-set) might still be correct — for **completely different reasons**.

### The real argument for sparse-set: Glyph scripting ergonomics

Your scripting language will treat entities as dynamic property bags. Glyph scripts will constantly do this:

```lisp
(add-component! entity (Burning {:intensity 5.0 :duration 10.0}))
(remove-component! entity Frozen)
(when (has-component? entity Poisoned)
  (modify-component! entity Health (fn [h] (- h (* poison.dps dt)))))
```

This is the "immersive sim" pattern — entities gaining and losing status effects, temporary markers, interaction states. A witch's spell hits an NPC, adding `OnFire`. Rain starts, adding `Wet` to everything outdoors. A ward expires, removing `MagicShield`. This happens **many times per frame** across **many entities**.

In archetype ECS, every `add-component!` and `remove-component!` triggers an archetype migration — memcpy the entity's entire component set to a new table. With 15+ components per NPC and frequent status changes, this becomes a real cost.

In sparse-set ECS, add/remove is O(1) — insert into or remove from a single dense array. No other components are touched.

### The real argument for archetype: iteration throughput in Layer 4

Your local simulation layer will run tight systems over 500-5000 entities at 60fps:

```
Physics:    [Position, Velocity, Collider, Mass]          — every frame
Rendering:  [Position, Rotation, MeshHandle, Material]    — every frame  
Particles:  [Position, Velocity, Lifetime, Visual]        — every frame, thousands
Combat:     [Position, Hitbox, Health, CombatState]       — every frame during fights
```

Archetype ECS iterates these ~10x faster because the data is contiguous. Sparse-set requires joining multiple sparse arrays, which means indirection per component per entity.

### My recommendation: sparse-set with hot-path dense views

```rust
// The storage is sparse-set (cheap add/remove, script-friendly)
// But for performance-critical queries, you maintain dense "views"
// that cache the join result

struct DenseView<A, B, C> {
    // Contiguous arrays, rebuilt when composition changes
    // Amortized cost: cheap if entities aren't churning every frame
    entities: Vec<EntityId>,
    a: Vec<A>,
    b: Vec<B>, 
    c: Vec<C>,
    dirty: bool,
}

// The renderer doesn't iterate sparse sets — it iterates a dense view
// that was built once and incrementally maintained
let render_view: DenseView<Position, Rotation, MeshHandle> = 
    ecs.dense_view::<(Position, Rotation, MeshHandle)>();

// Scripts still see the sparse-set interface
// (add-component! entity (Burning ...))  ← O(1), marks render_view dirty
```

This gives you sparse-set's flexibility for scripting and dynamic composition, with archetype-like iteration performance for the hot paths you identify through profiling. The dense views are an optimization you add when needed (Game 3+), not a foundational commitment.

**Action item: Rewrite the ECS decision entry.** Remove the LOD hydration rationale. Replace with: "Sparse-set primary for scripting ergonomics and dynamic component composition. Dense view caches added as a profiling-driven optimization for iteration-heavy systems in Game 3+."

---

## Part 2: The Layer Architecture Must Enter the Plan Explicitly

The plan currently treats the engine as a flat collection of subsystems. Our discussion established that the world simulation is a **deeply layered architecture** where each layer has different computational patterns, different update rates, and different data representations. This needs to be a first-class architectural commitment, not something that emerges organically.

### What's missing: the layer boundaries as explicit engine infrastructure

```
Layer 4: Local Simulation (ECS)
  Update rate: 60fps
  Data model: sparse-set ECS components
  Scope: entities near the player (~500-5000)
  
Layer 3: Agent Behavior (Blackboard + Utility AI + HTN)
  Update rate: 10-30Hz for active NPCs, less for distant
  Data model: per-agent blackboard structs
  Scope: NPCs in loaded regions (~30 full, ~200 reduced)

Layer 2: World Simulation (Chronicle rules + relational queries)  
  Update rate: per game-day/week/month depending on system
  Data model: relational tables + relationship graph
  Scope: entire world (~10k-50k characters, all factions, all settlements)

Layer 1: World State (in-memory relational database)
  Update rate: written to by Layers 2-4, read by all
  Data model: entity tables + relationship tables + spatial index
  Scope: everything that persists

Infrastructure: Event backbone connecting all layers
```

### Where each layer enters the game progression

```
Game 0-1: Layer 4 only (ECS + physics, no world simulation)
Game 2:   Layer 4 + primitive Layer 3 (scripted AI state machines)
Game 3:   Layer 4 + Layer 3 extended (group AI, budgeted updates)
Game 4:   ALL LAYERS — this is where the architecture fully materializes
```

**Action item: Add a section to the plan titled "World Architecture Layers" that describes each layer, its update rate, data model, threading, and which game introduces it.** This prevents Game 4 from being a "figure it out when we get there" situation.

### The crate structure should reflect this

Current plan has `schooner-world` and `schooner-ai` as flat crates. For Game 4, you need:

```
schooner-world-state     (Layer 1: relational DB, relationship tables)
schooner-world-sim       (Layer 2: Chronicle VM, rule engine, economy)
schooner-ai-agent        (Layer 3: blackboard, utility AI, HTN, LOD scheduler)
schooner-sim-bridge      (hydration/dehydration, event propagation between layers)
```

These don't need to exist until Game 4 pre-production, but they should be on the architectural roadmap now so that decisions made in Games 2-3 don't accidentally make them harder to build.

---

## Part 3: Two Languages, Not One

The plan mentions "shik" as a single scripting language. Our discussion established that the world simulation and gameplay scripting have fundamentally different computational patterns that benefit from different language paradigms. This is a significant architectural change that needs to be reflected.

### Chronicle: the world simulation language

**Paradigm:** Declarative, rule-based, query-driven
**Purpose:** Author the thousands of rules that make the world feel alive — political events, economic shifts, relationship dynamics, faction behavior
**Introduced:** Game 4 (designed during Game 3)

This isn't a "programming language" in the traditional sense. It's a **rule authoring language** that queries your relational world database:

```rust
event peasant_revolt {
    trigger {
        scope: Character where { is_ruler, rank <= Baron }
        territory := scope.primary_territory
        peasant_unrest(territory) >= 0.7
        NOT has_trait(scope, Charitable)
        liege := scope.liege
        opinion(scope, liege) < -10
    }
    
    weight {
        base: 100
        +50 if culture(scope) != culture(scope.liege)
    }
    
    effect {
        let revolt_leader = spawn_character { ... }
        start_war(attacker: revolt_leader, defender: scope, ...)
        modify_opinion(scope, liege, -20, reason: FailedToControlPeasants)
    }
}
```

The key insight is that Chronicle's evaluation model is **fundamentally different** from Glyph's. Chronicle evaluates rules against a relational database on a slow tick. Glyph executes procedural logic at gameplay speed. Trying to do both in one language means the language is mediocre at both.

### Glyph: the gameplay scripting language

**Paradigm:** Functional core, imperative shell, Lisp-like syntax
**Purpose:** Spell definitions, NPC behavior trees, combat abilities, crafting recipes, UI, quest logic
**Introduced:** Game 2A

This is the language that scripts interact with the ECS, define spell compositions via macros, and drive moment-to-moment gameplay.

### Why two languages is worth the cost

```
Chronicle                          Glyph
─────────────────────────────────  ──────────────────────────────────
Queries relational data            Mutates ECS components
Evaluates triggers/conditions      Executes procedural sequences
Runs on world-tick (monthly)       Runs at gameplay speed (per-frame)
Authored by designers/modders      Authored by gameplay programmers
Feels like CK3 event scripting    Feels like a typed Lisp
Output: world state mutations      Output: entity spawns, effects, UI
```

If you force both into one language, you either get a general-purpose language that's awkward for rule authoring (designers won't use it), or a rule language that's awkward for procedural gameplay (programmers fight it).

### Mitigation for the cost

The two languages can share infrastructure:

```
Shared:
- Bytecode format (same VM instruction set, different compilers)
- Hot reload infrastructure
- FFI bridge to Rust
- Value representation (same in-memory format for numbers, strings, entities)
- Editor tooling framework (LSP skeleton, syntax highlighting framework)

Not shared:
- Parser (different syntax)
- Type system (different type rules)
- Evaluation model (rule matching vs procedural execution)
```

Sharing the VM means you're building one runtime, two frontends. This cuts the cost roughly in half compared to building two completely independent language stacks.

**Action item: Replace "shik" in the plan with the two-language strategy. Add Chronicle to the Game 4 subsystem list and note that its design begins during Game 3. Note the shared VM infrastructure to keep the cost bounded.**

---

## Part 4: The Reactive Event Backbone (Not Just an "Event Bus")

You asked whether the event bus is "kinda like a reactivity system." The answer is: it should be, and it's more fundamental than a simple pub/sub bus. This is the nervous system of the entire engine, and it needs to be designed as such.

### Why a simple event bus isn't enough

A basic pub/sub event bus handles one-shot notifications:

```rust
// Simple event bus: fire and forget
event_bus.publish(CollisionEvent { a, b, force });
// Subscribers get it this frame, process it, done.
```

But your game needs **reactive cascades** — when one thing changes, it triggers a chain of responses across layers:

```
Player casts fire spell on NPC
  → ECS: spawn fire projectile, detect collision
    → ECS: add Burning component to NPC
      → Agent AI: NPC perceives it's on fire, re-evaluates goals (flee? stop-drop-roll?)
        → Agent AI: nearby NPCs perceive fire, re-evaluate (help? flee? attack player?)
          → World Sim: if NPC was important, political consequences
            → World Sim: faction opinion of player changes
              → World Sim: triggers potential retaliation event next month
```

This crosses all four layers, happens at different time scales, and involves both immediate reactions and deferred consequences. A simple event bus can't express this cleanly.

### What you actually need: a tiered reactive system

```rust
/// Tier 1: Synchronous, within-frame reactions
/// Used by: ECS systems reacting to component changes
/// Example: Burning component added → fire particle system activates
struct ComponentChangeReactor {
    // Sparse-set change detection: each component store tracks 
    // which entities were modified this frame
    // Systems can query "give me all entities where Health changed"
    
    // This is built INTO the ECS, not alongside it
    fn on_change<C: Component>(&self) -> ChangeIterator<C>;
    fn on_added<C: Component>(&self) -> AddedIterator<C>;
    fn on_removed<C: Component>(&self) -> RemovedIterator<C>;
}

// Usage in a system:
fn fire_visual_system(added: OnAdded<Burning>, query: Query<(&Position, &Burning)>) {
    for (entity, pos, burning) in added.iter() {
        spawn_fire_particles(pos, burning.intensity);
    }
}

/// Tier 2: Deferred cross-layer events  
/// Used by: Layer 4 (ECS) → Layer 3 (Agent AI), Layer 3 → Layer 2 (World Sim)
/// Example: NPC died in combat → world simulation needs to know
struct LayerEventQueue {
    // Events are published by one layer and consumed by another
    // on the CONSUMER'S tick schedule
    
    // ECS publishes at 60fps, Agent AI consumes at 10-30Hz,
    // World Sim consumes at game-day boundaries
    
    ecs_to_agent: TypedQueue,     // "NPC took damage", "player seen"
    ecs_to_world: TypedQueue,     // "NPC died", "building destroyed"
    agent_to_ecs: TypedQueue,     // "NPC decided to attack", "NPC fleeing"
    agent_to_world: TypedQueue,   // "NPC changed job", "NPC left faction"
    world_to_agent: TypedQueue,   // "war declared", "your spouse died"
    world_to_ecs: TypedQueue,     // "spawn army entities", "weather change"
}

/// Tier 3: Chronicle rule triggers
/// Used by: World Simulation evaluating rules against accumulated events
/// Example: "3 NPCs have died in this territory this month" → trigger unrest event
struct WorldEventAccumulator {
    // Events don't trigger immediate reactions — they accumulate
    // and become queryable conditions for Chronicle rules
    
    // Chronicle can query: "count(CharacterDied, territory=X, month=current) > 3"
    fn record(&mut self, event: WorldFact);
    fn query(&self, predicate: ChronicleQuery) -> QueryResult;
}
```

### How this maps to the game progression

```
Game 0: Tier 1 only — component change detection in ECS
        (This is just good ECS design, not extra infrastructure)
        
Game 1: Tier 1 + basic typed event queue for physics → game logic
        CollisionEvent, TriggerEvent, DestructionEvent
        
Game 2: Tier 2 — cross-system events for AI perception
        "Player made noise at position X" → AI hearing system
        "Script triggered door open" → AI awareness
        This is where you formalize the tiered model.
        
Game 3: Tier 2 extended — time-of-day events, wave spawning, 
        weather affecting gameplay
        
Game 4: All three tiers — full reactive backbone connecting 
        ECS ↔ Agent AI ↔ World Simulation
        Tier 3 (accumulator) built for Chronicle integration
```

**Action item: Add "Reactive Event Infrastructure" as an engine subsystem introduced incrementally across Games 0-4. The Tier 1 component change detection should be designed into the ECS from Game 0 — it's much harder to retrofit. Replace the current vague "reactive cascade semantics" open decision with a concrete tiered design that gets refined game by game.**

### How Tier 1 integrates with the ECS from day one

This is important enough to detail now because it affects the ECS design in Game 0:

```rust
// Each SparseSet tracks changes per frame
struct SparseSet<T> {
    dense: Vec<T>,
    sparse: Vec<Option<usize>>,
    entities: Vec<EntityId>,
    
    // Change tracking — just bitsets, very cheap
    added_this_frame: BitSet,
    modified_this_frame: BitSet,
    removed_this_frame: BitSet,
    
    // Cleared at frame boundary
    fn clear_change_flags(&mut self) { ... }
}

// Systems declare interest in changes
fn poison_damage_system(
    changed: Changed<Poisoned>,  // only entities where Poisoned was added or modified
    query: Query<(&Poisoned, &mut Health)>
) {
    for entity in changed.iter() {
        let (poison, health) = query.get(entity);
        health.current -= poison.dps * dt;
    }
}
```

This costs almost nothing (three bitsets per component type, cleared each frame) but enables the entire reactive cascade model. If you don't build it into the ECS from Game 0, you'll be retrofitting it in Game 2 when scripts need to react to component changes, and that's a painful refactor.

---

## Part 5: Agent AI Architecture — Blackboard + Utility AI + HTN

The plan mentions AI state machines and behavior trees in Game 2, and utility AI in Game 4. Our discussion established a more specific architecture: Blackboard as the shared data model, Utility AI for goal selection, and Hierarchical Task Networks for plan execution. This needs to be in the plan with enough detail to guide implementation.

### What's wrong with the current plan's AI progression

The plan introduces AI as:
- Game 2: state machines / behavior trees (patrol, investigate, chase)
- Game 3: group behavior (pack tactics, flanking)
- Game 4: utility AI, needs, relationships

The problem is that **state machines and behavior trees are a different paradigm from utility AI + HTN.** If you build sophisticated behavior trees in Game 2, you'll likely rewrite the AI architecture for Game 4 when you switch to utility-based decision making. This is wasted work.

### Better progression: design for utility AI from Game 2, but simplify the evaluation

```
Game 2A: Introduce the Blackboard + simple Utility evaluation
  
  The horror game's AI enemy uses a blackboard, not a state machine:
  
  Blackboard contents:
    last_heard_noise: Option<(Position, Time)>
    last_seen_player: Option<(Position, Time)>  
    current_suspicion: f32  (0.0 = calm, 1.0 = certain player is here)
    patrol_route: Vec<Position>
    current_patrol_index: usize
  
  Utility evaluation (simple — only 4 possible goals):
    patrol:      score = 1.0 - current_suspicion
    investigate: score = if last_heard_noise.is_recent() { suspicion * 0.8 } else { 0.0 }
    search:      score = if suspicion > 0.5 && !can_see_player { suspicion * 0.9 } else { 0.0 }  
    chase:       score = if can_see_player { 1.0 } else { 0.0 }
  
  This looks EXACTLY like a state machine from the outside, but the 
  architecture is already utility-based. When Game 4 needs NPCs evaluating
  20 possible goals, you extend the same system — you don't rewrite.

Game 2B: Add HTN planning for goal execution
  
  Once the AI selects "investigate" as the best goal, HOW does it 
  investigate? HTN decomposes it:
  
  investigate(noise_source):
    → move_to(noise_source)
      → plan_path(current_pos, noise_source)
      → follow_path(path)
    → look_around(noise_source, radius: 5.0)
      → turn_to(direction_1), wait(1.0)
      → turn_to(direction_2), wait(1.0)
    → if found_evidence:
        → increase_suspicion(0.3)
        → search(area)
      else:
        → decrease_suspicion(0.2)
        → return_to_patrol()
  
  The HTN planner is authored in Glyph scripts.
  This is the same planner that Game 4's NPCs will use for 
  "go to market → buy food → go home → cook → eat → sleep."

Game 3: Add perception budget and group coordination
  
  Multiple creatures share a blackboard (pack blackboard):
    known_threats: Vec<(EntityId, Position, Time)>
    pack_state: Hunting | Flanking | Retreating
    alpha: EntityId
  
  Individual creature blackboards reference the pack blackboard.
  Utility evaluation now includes pack-level goals:
    flank_target: score based on pack_state and own position relative to target
    support_alpha: score based on alpha's health and distance
  
  This directly extends the Game 2 architecture without rewriting.

Game 4: Full utility AI with needs, personality, and world knowledge
  
  NPC blackboard is now rich:
    needs: { hunger: 0.7, rest: 0.3, safety: 0.9, wealth: 0.4, social: 0.5 }
    personality: { brave: 0.8, greedy: 0.3, loyal: 0.6 }
    world_knowledge: (imported from World Database via Layer bridge)
    relationships: (imported from Layer 2 relationship graph)
    current_job: Blacksmith
    faction_standing: { ... }
    recent_events: [ ... ]
  
  Utility evaluation over 20+ possible goals, weighted by needs 
  and personality. HTN plans the chosen goal. All authored in Glyph.
```

### Why this matters: the blackboard is also the script↔engine bridge

The blackboard is the natural interface between your Glyph scripts and the Rust engine:

```rust
// Rust side: populate blackboard from engine state
impl Blackboard {
    fn populate_from_perception(&mut self, perception: &PerceptionResult) {
        self.set("can_see_player", perception.player_visible);
        self.set("nearest_threat_distance", perception.nearest_threat_dist);
        self.set("heard_noise", perception.recent_noise);
    }
    
    fn populate_from_world(&mut self, world: &WorldDatabase, char_id: CharacterId) {
        self.set("faction", world.faction_of(char_id));
        self.set("opinion_of_player", world.opinion(char_id, player_id));
        self.set("current_territory_safety", world.territory_safety(char_id));
    }
}

// Glyph side: read blackboard, make decisions
(defbehavior blacksmith-daily
  :utility (fn [board]
    (let [hunger    (bb/get board :hunger)
          shop-stock (bb/get board :iron-stock)
          safety    (bb/get board :territory-safety)]
      (score-goals
        (work-at-forge  (* 0.7 (need-urgency shop-stock) safety))
        (eat            (* 0.9 hunger))
        (flee           (if (< safety 0.3) 1.0 0.0))
        (socialize      (* 0.3 (bb/get board :social-need)))))))
```

**Action item: Replace the Game 2 "AI state machines / behavior trees" with "Blackboard + Utility AI (simple, 4 goals) + scripted goal evaluation." Add HTN planning to Game 2B. Note that this architecture extends directly into Game 4 without a paradigm shift. Add the blackboard as a named engine concept that bridges Glyph scripts and Rust engine state.**

---

## Part 6: World Simulation Engine — Rule-Based Event System

The plan mentions "economy simulation" and "faction system" in Game 4 but doesn't describe the evaluation architecture. Our discussion established that this should be a **rule-based event system with tick-driven evaluation on relational data**, powered by the Chronicle language. This needs specifics.

### The evaluation loop

```rust
// This runs on the World Thread, NOT the main thread
// It does NOT run every frame — it runs on a game-clock

struct WorldSimulation {
    world: WorldDatabase,        // Layer 1
    chronicle_vm: ChronicleVM,   // Chronicle rule evaluator
    event_queue: PriorityQueue<WorldEvent>,
    history: EventLog,           // accumulated facts for rule queries
    
    fn tick_game_day(&mut self) {
        // Phase 1: Advance basic simulations
        self.economy.daily_tick(&mut self.world);   // production, consumption
        self.migration.daily_tick(&mut self.world);  // NPC movement between settlements
        
        // Phase 2: Evaluate character-driven events (Chronicle rules)
        // NOT every character every day — stagger evaluation
        let batch = self.world.characters
            .alive()
            .staggered_batch(self.current_day);  // evaluate ~1/7 of characters per day
        
        for character in batch {
            let triggered = self.chronicle_vm.evaluate_triggers(
                character, 
                &self.world, 
                &self.history
            );
            for event in triggered {
                self.event_queue.push(event);
            }
        }
        
        // Phase 3: Resolve events (with conflict resolution)
        while let Some(event) = self.event_queue.pop() {
            let effects = self.chronicle_vm.execute_effects(event, &mut self.world);
            self.history.record(event, effects);
            
            // Publish to other layers via event backbone
            self.layer_events.publish_to_agents(effects.agent_relevant());
            self.layer_events.publish_to_ecs(effects.ecs_relevant());
        }
    }
    
    fn tick_game_month(&mut self) {
        // Expensive evaluations that only make sense monthly
        self.economy.monthly_rebalance(&mut self.world);
        self.factions.evaluate_power_balance(&mut self.world);
        self.succession.check_inheritance_crises(&mut self.world);
        
        // Monthly Chronicle rules (different from daily ones)
        for character in self.world.characters.rulers() {
            let triggered = self.chronicle_vm.evaluate_monthly_triggers(
                character, &self.world, &self.history
            );
            // ...
        }
    }
}
```

### The World Database (Layer 1) is NOT the ECS

This is a critical distinction that the current plan doesn't make. The world database is a **relational store**, not an entity-component store:

```rust
struct WorldDatabase {
    // Entity tables (like database tables, not ECS entities)
    characters: Table<CharacterId, CharacterRecord>,
    settlements: Table<SettlementId, SettlementRecord>,
    factions: Table<FactionId, FactionRecord>,
    titles: Table<TitleId, TitleRecord>,
    
    // Relationship tables — the core differentiator from ECS
    marriages: RelationTable<CharacterId, CharacterId, MarriageData>,
    vassalage: RelationTable<CharacterId, CharacterId, VassalageData>,
    opinions: RelationTable<CharacterId, CharacterId, OpinionData>,
    title_claims: RelationTable<CharacterId, TitleId, ClaimData>,
    faction_membership: RelationTable<CharacterId, FactionId, MembershipData>,
    trade_routes: RelationTable<SettlementId, SettlementId, TradeData>,
    
    // Bidirectional indexing
    // "all vassals of X" AND "liege of X" both O(1)
    
    // Spatial index for region queries
    spatial: SpatialIndex<EntityId>,
}
```

Chronicle queries compile down to operations on this relational store:

```
// Chronicle rule trigger:
//   scope: Character where { is_ruler, rank <= Baron }
//   liege := scope.liege
//   opinion(scope, liege) < -10
//
// Compiles to:
//   1. Iterate characters table where is_ruler=true AND rank <= Baron
//   2. For each, look up vassalage relation to find liege
//   3. Look up opinion relation between scope and liege
//   4. Filter where opinion.value < -10
```

This is SQL-like query execution, not ECS iteration. Trying to model this in the ECS would be fighting the architecture.

**Action item: Add "World Database" as a named engine subsystem (schooner-world-state) that is explicitly NOT the ECS. Define its table/relation schema as an early Game 4 design task. Note that Chronicle compiles to queries against this database.**

---

## Part 7: Threading Model

The plan doesn't discuss threading at all. Our discussion established a specific threading model that's critical for performance:

```
┌─────────────────────────────────────────────────┐
│ Main Thread                                      │
│ 60fps                                            │
│ - ECS system execution (Layer 4)                 │
│ - Rendering submission                           │
│ - Input processing                               │
│ - UI                                             │
│ - Physics step (Rapier)                          │
├─────────────────────────────────────────────────┤
│ AI Thread                                        │
│ 10-30Hz (budgeted)                               │
│ - Blackboard population                          │
│ - Utility evaluation for active NPCs             │
│ - HTN plan stepping                              │
│ - Perception processing                          │
│ - Results written to command buffer, applied on   │
│   main thread next frame                         │
├─────────────────────────────────────────────────┤
│ World Thread                                     │
│ Per game-day (variable rate)                     │
│ - Chronicle rule evaluation                      │
│ - Economy simulation                             │
│ - Faction power balance                          │
│ - World event resolution                         │
│ - Can run ahead during fast-travel / time-skip   │
│ - Results published via layer event queues       │
├─────────────────────────────────────────────────┤
│ Render Thread (optional, Game 3+)                │
│ - wgpu command buffer building                   │
│ - Decoupled from simulation at high entity counts│
├─────────────────────────────────────────────────┤
│ IO Thread(s)                                     │
│ - Asset streaming                                │
│ - Save/load                                      │
│ - Chunk loading/unloading                        │
├─────────────────────────────────────────────────┤
│ Audio Thread                                     │
│ - Spatial audio mixing                           │
│ - Standard                                       │
└─────────────────────────────────────────────────┘
```

### The critical synchronization points

```rust
// AI Thread → Main Thread: command buffer pattern
// AI thread never directly mutates ECS. It writes commands.
struct AICommandBuffer {
    commands: Vec<AICommand>,
}

enum AICommand {
    MoveTo { entity: EntityId, destination: Vec3 },
    Attack { attacker: EntityId, target: EntityId },
    PlayAnimation { entity: EntityId, anim: AnimationId },
    Flee { entity: EntityId, from: Vec3 },
    SetBlackboardValue { entity: EntityId, key: BlackboardKey, value: Value },
}

// Main thread applies these at a defined sync point each frame
fn apply_ai_commands(commands: &AICommandBuffer, ecs: &mut World) {
    for cmd in &commands.commands {
        match cmd {
            AICommand::MoveTo { entity, destination } => {
                if let Some(mut nav) = ecs.get_mut::<NavTarget>(*entity) {
                    nav.target = *destination;
                }
            }
            // ...
        }
    }
}

// World Thread → Main Thread: similar pattern but less frequent
// World events are queued and processed once per frame
struct WorldEventInbox {
    events: Vec<WorldLayerEvent>,
}
```

### When each thread is introduced

```
Game 0-1: Single-threaded. Don't even think about threads yet.
Game 2:   AI logic runs on main thread but in a budgeted scheduler.
          This is the "prepare for threading" step — the budget 
          scheduler already processes NPCs in batches, so moving 
          batches to another thread later is mechanical.
Game 3:   IO thread for terrain streaming. AI might move to its 
          own thread if horde counts demand it.
Game 4:   World Thread introduced for Chronicle evaluation.
          AI Thread formalized.
Game 5:   Render thread decoupling if needed.
```

**The key design principle:** write code that's thread-ready in Game 2 (command buffers, no shared mutable state between AI and ECS), but don't actually spawn threads until profiling says you need them (Game 3 or 4). This avoids premature complexity while ensuring the architecture doesn't prevent threading when it's needed.

**Action item: Add threading model to the plan as an open decision to be refined game by game. Note the command buffer pattern as a design constraint starting in Game 2. The AI scheduler should be structured as batch processing from day one, even if it runs on the main thread initially.**

---

## Part 8: World Streaming and LOD — The Hydration Bridge

The plan mentions "chunk-based world streaming" in Game 3 but doesn't describe how it interacts with the layered world architecture. This interaction is the hardest engineering problem in the entire plan.

### The core challenge: two representations of the same entity

An NPC named "Aldric the Blacksmith" exists simultaneously as:

```
Layer 1 (always): WorldDatabase CharacterRecord
  { id: 4721, name: "Aldric", profession: Blacksmith, 
    location: Millhaven, health: 0.9, faction: MerchantGuild, ... }

Layer 4 (only when near player): ECS Entity
  Position(234.5, 0.0, -891.2)
  Velocity(0.0, 0.0, 0.0)
  MeshHandle(blacksmith_male_01)
  AnimationState(Idle)
  Health(90.0 / 100.0)
  Collider(capsule)
  ...
```

When the player approaches Millhaven, Aldric must be **hydrated**: his world database record becomes a living ECS entity with physics, rendering, and AI. When the player leaves, he must be **dehydrated**: his current state writes back to the database, and the ECS entity is despawned.

### The hydration bridge

```rust
struct HydrationBridge {
    // Maps between world IDs and ECS entity IDs
    world_to_ecs: HashMap<CharacterId, EntityId>,
    ecs_to_world: HashMap<EntityId, CharacterId>,
    
    fn hydrate_region(&mut self, region: RegionId, world: &WorldDatabase, ecs: &mut EcsWorld) {
        for char_id in world.characters_in_region(region) {
            let record = world.characters.get(char_id);
            
            // Spawn ECS entity with full component set
            let entity = ecs.spawn((
                Position::from_world_pos(record.location),
                MeshHandle::for_character(record),
                AnimationState::idle(),
                Health::new(record.health * record.max_health, record.max_health),
                Collider::humanoid(),
                NavAgent::new(),
                BlackboardHolder::new(),
                WorldIdentity(char_id),  // links back to world DB
            ));
            
            self.world_to_ecs.insert(char_id, entity);
            self.ecs_to_world.insert(entity, char_id);
            
            // Initialize agent AI blackboard from world knowledge
            let blackboard = ecs.get_mut::<BlackboardHolder>(entity);
            blackboard.populate_from_world(world, char_id);
            blackboard.apply_daily_schedule(world.current_time());
            blackboard.apply_emotional_state(&record.recent_events);
        }
    }
    
    fn dehydrate_region(&mut self, region: RegionId, world: &mut WorldDatabase, ecs: &mut EcsWorld) {
        let entities: Vec<EntityId> = self.world_to_ecs
            .iter()
            .filter(|(char_id, _)| world.character_in_region(**char_id, region))
            .map(|(_, entity)| *entity)
            .collect();
        
        for entity in entities {
            let char_id = self.ecs_to_world[&entity];
            
            // Write back current state to world database
            if let Some(health) = ecs.get::<Health>(entity) {
                world.characters.get_mut(char_id).health = health.fraction();
            }
            if let Some(pos) = ecs.get::<Position>(entity) {
                world.characters.get_mut(char_id).location = pos.to_world_pos();
            }
            // ... write back other relevant state
            
            // Despawn ECS entity
            ecs.despawn(entity);
            self.world_to_ecs.remove(&char_id);
            self.ecs_to_world.remove(&entity);
        }
    }
}
```

### The catch-up problem

When the player fast-travels to a city they haven't visited in 30 game-days, NPCs there need plausible state. The world simulation (Layer 2) already computed high-level events: wars, deaths, faction changes. But the agent-level detail (where exactly is Aldric standing? what's he doing right now?) needs reconstruction.

```rust
fn reconstruct_agent_state(
    char_id: CharacterId, 
    world: &WorldDatabase, 
    current_time: GameTime
) -> AgentState {
    let record = world.characters.get(char_id);
    
    // What SHOULD this person be doing right now?
    let schedule = generate_daily_schedule(record);
    let current_activity = schedule.activity_at(current_time.time_of_day());
    
    // Apply world events that happened while we weren't looking
    let recent_events = world.events_affecting(char_id, last_30_days);
    let emotional_state = compute_emotional_state(record, &recent_events);
    
    AgentState {
        position: current_activity.expected_position(),
        animation: current_activity.expected_animation(),
        emotional_state,
        current_goal: current_activity.as_utility_goal(),
    }
}
```

This is the "plausible illusion" approach. The world simulation provides ground truth (macro events), and the hydration bridge generates plausible micro-state. Players can't tell the difference because the macro state is real — if a war happened, buildings are damaged, certain NPCs are dead, faction control has shifted. Only the moment-to-moment positioning is reconstructed.

**Action item: Add "Hydration Bridge" as a named subsystem introduced in Game 4. Note the catch-up problem and the "plausible illusion" approach as the design direction. Link it to the LOD continuity open decision and resolve that decision in favor of the narrative-important hybrid approach: flagged characters (rulers, quest-givers, player-interacted NPCs) get full state persistence; background population gets plausible reconstruction.**

---

## Part 9: Profiling and Debug Infrastructure

The plan has zero mention of profiling. For an engine that will eventually run thousands of AI agents, a world simulation, terrain streaming, and a custom scripting language, this is a critical gap.

### What you need from Game 0

```
Game 0 checklist additions:
- [ ] Tracy or puffin integration for frame profiling
- [ ] Per-ECS-system timing (automatic — every system reports its duration)
- [ ] Entity count dashboard (total, per-component-type)
- [ ] Memory allocation tracking (per-subsystem)
- [ ] FPS counter and frame time graph (in-engine overlay)
```

### What you need by Game 2

```
Game 2 additions:
- [ ] Script execution profiling (which Glyph functions are expensive)
- [ ] Script memory tracking (how much memory is the VM using)
- [ ] AI budget visualizer (how many NPCs evaluated this frame, time spent)
- [ ] Event bus throughput (events published/consumed per frame per channel)
```

### What you need by Game 4

```
Game 4 additions:
- [ ] World simulation tick profiler (Chronicle rule evaluation time)
- [ ] Layer event latency (time from ECS event to world sim processing)
- [ ] NPC LOD distribution visualizer (how many at each detail level)
- [ ] World database query profiler (which Chronicle queries are slow)
- [ ] Thread utilization dashboard (main, AI, world, IO)
```

This isn't gold-plating. When Game 3's horde mode drops to 40fps, you need to know in seconds whether it's physics, rendering, AI, or terrain streaming. When Game 4's world simulation tick takes 100ms instead of 10ms, you need to know which Chronicle rules are the bottleneck.

**Action item: Add profiling infrastructure to Game 0's checklist. Add Tracy/puffin integration as a resolved decision. Note that per-system timing in the ECS should be automatic (every system wrapped in a timing scope) so you never have to remember to add it.**

---

## Part 10: Testing Strategy

No mention of testing in the plan. For an engine with two custom languages, a relational world database, a multi-threaded architecture, and a reactive event backbone, this is a serious omission.

### Testing layers

```
Unit tests (from Game 0):
  - ECS: spawn, despawn, add/remove component, query correctness
  - ECS: change detection fires correctly
  - Event system: publish/subscribe ordering, type safety
  - Math: transforms, spatial queries

Integration tests (from Game 1):
  - Physics + ECS sync: transforms match after physics step
  - Trigger volumes: entity enters/exits detected correctly
  - Audio: positional audio follows entity position

Script tests (from Game 2):
  - Glyph VM: arithmetic, closures, pattern matching, recursion
  - Glyph FFI: Rust → Glyph calls, Glyph → Rust calls, data marshaling
  - Hot reload: swap script, verify new behavior, verify no stale references
  - Script sandboxing: infinite loop terminated, memory budget enforced

AI tests (from Game 2B):
  - Perception: "NPC at position X can/cannot see player at position Y"
  - Utility: "given this blackboard state, the correct goal is X"
  - HTN: "given goal X, the plan decomposes to [A, B, C]"
  - Pathfinding: "path from A to B avoids obstacles and is valid"

World simulation tests (from Game 4):
  - Chronicle rule triggers: "given this world state, rule X fires"
  - Effect application: "after event X, world state is Y"
  - Relationship queries: "vassals of duke include [A, B, C]"
  - Economy: "after 10 ticks, prices converge to equilibrium"
  - Hydration round-trip: hydrate → modify → dehydrate → rehydrate = consistent

Performance regression tests (from Game 3):
  - "10,000 entities with Position+Velocity iterate in < 1ms"
  - "100 NPC utility evaluations complete in < 2ms"
  - "Terrain chunk load completes in < 50ms"
  - Run on every commit, fail the build if regressed
```

**Action item: Add a testing strategy section to the plan. The ECS tests should be written alongside the ECS in Game 0 — they're how you validate the sparse-set implementation is correct before building anything on top of it.**

---

## Part 11: Game 2 Split and Language Staging

### The revised Game 2

As discussed, Game 2 is overloaded. But the split is now informed by the two-language strategy:

```
Game 2A: "Whisper — Scripted Horror"
  New engine work:
  - Glyph language v1 (MINIMAL — see staging below)
  - Glyph VM + Rust FFI
  - Hot reload
  - Asset pipeline v1 (glTF)
  - Shadow maps
  - Scene serialization
  
  Game logic authored in Glyph:
  - Scripted scare sequences
  - Item pickups, door triggers
  - Environmental puzzles
  - UI (inventory, notes, HUD)
  
  NO AI enemy yet. Horror from atmosphere and scripted events.

Game 2B: "Whisper — The Hunter"
  New engine work:
  - Blackboard system (Rust-side, populated for scripts)
  - Utility AI framework (simple: 4 goals)
  - HTN planner (Glyph-authored plans)
  - Perception system (sight, hearing, memory)
  - NavMesh + pathfinding
  - AI budget scheduler (runs on main thread but in bounded batches)
  - 3D spatial audio
  
  AI behavior authored in Glyph:
  - Goal scoring functions
  - HTN task definitions
  - Perception response handlers
  
  Proves: AI can be authored in Glyph and runs at acceptable performance.
```

### Glyph language staging

This is the most important risk mitigation in the plan:

```
Game 2A — Glyph v0.1: "It works"
  - S-expression parser
  - Bytecode compiler + VM
  - Dynamically typed (yes, really)
  - Basic types: int, float, string, bool, list, map, entity-id
  - Functions, closures, let bindings
  - Rust FFI: call Rust from Glyph and Glyph from Rust
  - Hot reload: file watcher → recompile → swap module
  - Component access: (get-component entity 'Position) returns dynamic value
  
  You WILL feel the pain of dynamic typing. That's the point.
  You'll know exactly which type errors hurt most, and that tells 
  you what the static type system needs to catch first.

Game 2B → Game 3 — Glyph v0.5: "It's typed"
  - Hindley-Milner type inference
  - Component types known to the compiler (from Rust schema export)
  - (get-component entity Position) returns a typed Position struct
  - Pattern matching (needed for spell element interactions in Game 3)
  - Better error messages with source locations
  
  Add types where you felt pain in Game 2A. Don't type everything —
  type the things that broke.

Game 3 → Game 4 — Glyph v1.0: "It's expressive"
  - Hygienic macros (driven by spell composition needs in Game 3)
  - Refinement types on domain values (opinion: Int[-100..100])
  - ECS query expressions in the language
  - Performance optimization for hot paths (identified by profiling)

Game 4 — Chronicle v1.0: Separate language
  - Designed during Game 3, built during Game 4 pre-production
  - Shares VM backend with Glyph
  - Declarative rule syntax
  - Compiles to queries against world database
  - Static types with refinement (this language is typed from day one
    because content designers need the guardrails more than programmers)
```

**Action item: Replace the single "shik" language with this two-language staged approach. The staging is critical — it prevents the language from becoming a multi-year project that blocks game development. Each language version is motivated by concrete pain felt in the previous game.**

---

## Part 12: Revised Subsystem Reuse Table

The original table needs updating to reflect the layered architecture, two languages, and reactive event system:

```
| Subsystem                    | G0     | G1     | G2A    | G2B    | G3     | G4     |
|------------------------------|--------|--------|--------|--------|--------|--------|
| ECS (sparse-set)             | NEW    | reuse  | reuse  | reuse  | extend | extend |
| Change detection (Tier 1)    | NEW    | reuse  | extend | reuse  | reuse  | reuse  |
| Event queues (Tier 2)        |        | NEW    | extend | extend | extend | extend |
| Renderer (forward)           | NEW    | extend | extend | reuse  | REWORK | extend |
| Renderer (deferred/hybrid)   |        |        |        |        | NEW    | extend |
| Input                        | NEW    | reuse  | reuse  | reuse  | reuse  | reuse  |
| Physics (Rapier)             |        | NEW    | reuse  | reuse  | extend | reuse  |
| Audio                        |        | NEW    | reuse  | extend | reuse  | reuse  |
| Asset pipeline               |        | basic  | NEW    | reuse  | extend | reuse  |
| Glyph v0.1 (dynamic)        |        |        | NEW    | extend | —      | —      |
| Glyph v0.5 (typed)          |        |        |        |        | NEW    | extend |
| Glyph v1.0 (macros+refine)  |        |        |        |        |        | NEW    |
| Blackboard system            |        |        |        | NEW    | extend | extend |
| Utility AI framework         |        |        |        | NEW    | extend | extend |
| HTN planner                  |        |        |        | NEW    | extend | extend |
| Perception system            |        |        |        | NEW    | extend | extend |
| NavMesh + pathfinding        |        |        |        | NEW    | extend | reuse  |
| AI budget scheduler          |        |        |        | NEW    | extend | extend |
| Terrain + streaming          |        |        |        |        | NEW    | extend |
| Day/night + weather          |        |        |        |        | NEW    | reuse  |
| Dense view caches            |        |        |        |        | NEW*   | extend |
| World Database (Layer 1)     |        |        |        |        |        | NEW    |
| Chronicle language           |        |        |        |        | design | NEW    |
| World Simulation (Layer 2)   |        |        |        |        |        | NEW    |
| Hydration Bridge             |        |        |        |        |        | NEW    |
| Event accumulator (Tier 3)   |        |        |        |        |        | NEW    |
| Profiling (Tracy/puffin)     | NEW    | reuse  | reuse  | reuse  | reuse  | reuse  |
| Test infrastructure          | NEW    | extend | extend | extend | extend | extend |

* Dense view caches: added if profiling shows sparse-set iteration is the bottleneck
```

---

## Summary of All Recommended Changes

| # | Change | Rationale | Priority |
|---|--------|-----------|----------|
| 1 | Rewrite ECS rationale: scripting ergonomics, not LOD hydration | Old rationale invalidated by layered architecture | **Critical** |
| 2 | Add 5-layer world architecture as a first-class plan section | Foundation of Game 4, must be designed early | **Critical** |
| 3 | Replace single "shik" with staged Glyph + Chronicle | Two fundamentally different evaluation models | **Critical** |
| 4 | Split Game 2 into 2A (scripting) and 2B (AI) | Too much new infrastructure in one game | **Critical** |
| 5 | Add tiered reactive event system starting Game 0 | Nervous system of the entire engine | **Critical** |
| 6 | Add profiling infrastructure to Game 0 | Can't optimize what you can't measure | **High** |
| 7 | Add testing strategy to the plan | Two custom languages demand rigorous testing | **High** |
| 8 | Replace Game 2 state machines with Blackboard + Utility AI | Avoid paradigm-breaking rewrite for Game 4 | **High** |
| 9 | Add threading model as a progressive plan | Write thread-ready code in Game 2, thread in Game 3-4 | **High** |
| 10 | Add hydration bridge as a named Game 4 subsystem | Core challenge of open-world + living simulation | **High** |
| 11 | Add world database as explicitly not-ECS Layer 1 | Relational queries ≠ component iteration | **High** |
| 12 | Plan renderer transition for Game 3 | Forward rendering insufficient for outdoor scenes | **Medium** |
| 13 | Stage Glyph language: dynamic → typed → macros | Prevents language development from blocking games | **Medium** |
| 14 | Add crate restructure plan for Game 4 | Flat crates don't reflect layered architecture | **Medium** |
| 15 | Decide art strategy before Game 2 | No engine survives contact with no art | **Medium** |
| 16 | Resolve networking scope | Affects physics and combat architecture | **Low** |
| 17 | Plan modding architecture | Natural fit with custom languages | **Low** |

The plan's bones are strong. The game progression is smart and the discipline of "one new dimension per game" is exactly right. What's needed is integrating the architectural insights from our discussion — the layered world, the two languages, the reactive backbone, the threading model — as first-class commitments rather than things that emerge organically in Game 4. The decisions made in Games 0-2 either enable or prevent the architecture that Games 4-5 need.

