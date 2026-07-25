# Schooner — Architectural Overview

This is the vision of the engine, not its implementation. Each section describes **what a thing is** and **how it should work**. Concrete shapes — types, signatures, file layouts — live next to the code, not here, because those rot. Ideas don't.

---

## The Four Pillars

The engine exists to serve four commitments. Every architectural choice in this folder traces back to at least one; choices that serve none do not belong.

### 1. The world is alive

The world simulates itself. NPCs have lives, factions wage their conflicts, economies move, history accumulates — and they do all of this whether the player is watching or not. The environment is **embodied**: objects respond, physics is interactive and predictable, materials react to one another, fire spreads, water douses, weight settles. The player is one agent among many, and every action they take lands with weight and consequence inside a system that was already running.

This is not a feature layered on top. This is the design. Immersive simulation is the engine's reason for existence; everything else is in service of making it work.

### 2. Built for one kind of game

This is not a general-purpose engine. It is built for **first-person open-world games with immersive environments**, and every tool inside it is tailored for that target. The renderer is shaped by the kind of forests we want to walk through, not by what other games might need. The ECS is shaped by the kind of object interactions our game has, not by what general-purpose ECS frameworks support. The scripting languages are shaped by the rule systems and the moment-to-moment gameplay we are authoring, not by what scripting languages "should" do.

The discipline this imposes: every feature is justified against the target game. A capability that serves "any game" but not "this game" does not belong. Refusing generality is how the engine stays small enough for one developer to ship.

### 3. Developer ergonomics is a feature

Making the game should be a pleasure. Hot reload of scripts and shaders, REPL access into the running game, fast iteration cycles, clear error messages, languages designed for their domain rather than imported from elsewhere — these are not conveniences. They are part of the artefact.

The implication: the engine pays for ergonomics where ergonomics earns its cost. We build languages because they fit the domain better than third-party ones. We build a hot-reload substrate because the alternative is a worse working life. We invest in profiling and debug overlays from Game 0 because we will use them every day.

### 4. Organism, not castle

The architecture and the game logic are alive. **Strict rules form the skeleton; the shape on top is fluid.** Component types are statically typed contracts; which components live on which entity is a runtime question. Function signatures are stable; behaviour swaps in under hot reload. The world database has a schema; what facts populate it changes every game-day.

Static typing for safety. Dynamic structure for life. The engine is a body that grows, not a building that is finished.

The aesthetic flows from the same idea: a world that feels like a memory of a real place, soft and warm and inhabitable, rather than a polished but lifeless photograph. Pillar 1 reaches into how the world *looks*, not just how it *runs*.

---

## The Five Layers

The engine is not a flat collection of subsystems. It is a stack of layers, each at a different time scale, on a different data model, in a different threading context. Each layer is a genuinely different problem and resists being collapsed into the others.

### Layer 1 — World State

The relational ground truth. Every character, settlement, faction, title, claim, marriage, vassalage, opinion, trade route — every fact about the world that persists. A relational store, not an ECS. Queried like a database, indexed for set-returning joins, lives across the entire world regardless of where the player is.

Detail in `world-state.md`.

### Layer 2 — World Simulation

The forces that move the world. Economy, factions, succession, migration, war, weather. Authored as **rules** in Chronicle, evaluated on a slow tick (game-day, game-month) against Layer 1. Rules are declarative: a trigger is a query over the world database; an effect is a structured mutation of it. Runs on its own thread; may run ahead of the player during fast travel.

Detail in `chronicle.md`.

### Layer 3 — Agent Behavior

Individual NPCs deciding what to do. Built on three pillars: a **blackboard** (the agent's perception of its situation), **utility AI** (many candidate goals scored by need, personality, context — highest wins), and **hierarchical task networks** (chosen goals decomposed into ordered, conditional plans). Behaviour is authored in Glyph. The engine populates blackboards from perception and from the world database; the agent layer evaluates goals; the chosen plan is stepped frame by frame. Runs at 10–30 Hz for active NPCs, less for distant ones, and produces commands that Layer 4 applies.

### Layer 4 — Local Simulation (ECS)

The here-and-now. Everything in the player's immediate world: their body, the wind in the trees, the rigid bodies, the spell projectile, the wet-and-burning NPC. Sparse-set ECS, single-threaded, 60 Hz. Components are composable property sets that attach and detach freely; status effects flicker on and off; physics integrates; rendering submits. The only layer the player perceives directly.

Detail in `ecs.md`.

### Infrastructure — The Reactive Event Backbone

Layers do not call each other. They communicate through tiered events:

- **Tier 1 — component change detection.** Within Layer 4, systems react to component additions, removals, and mutations in the same frame. Built into the ECS storage. (Game 0 ships the mutation half; add/remove detection lands in Game 1.)
- **Tier 2 — cross-layer typed queues.** Layer 4 publishes to Layer 3, Layer 3 to Layer 2, and back. Producer publishes at its own rate; consumer reads on its own tick.
- **Tier 3 — world event accumulation.** Facts accumulate in Layer 1 over time and become queryable conditions for Chronicle rules ("three NPCs died here this month").

This backbone is the nervous system. It is what lets a fire spell propagate from "Burning component added" to "faction reputation shifts" through multiple time scales without any one layer knowing about the others.

---

## The Supporting Cast

Everything else exists to serve the layers above.

- **Renderer.** Forward, stylised, deliberately constrained to the aesthetic in `rendering.md`. Reads the ECS each frame and submits draws.
- **Physics (Rapier).** Lives inside Layer 4. Bridges to ECS transforms each fixed-update.
- **Audio.** Spatial. Reads ECS positions; consumes Tier 2 events for cues.
- **Asset pipeline.** Loads meshes, textures, scripts, scenes from authored files. Hot-reload is a first-class concern, not a stretch goal — pillar 3.
- **Developer tooling.** A `dev-tools`-gated host dynamically composes statically
  typed, subsystem-owned Rust debug plugins against the running world. Detail in
  `debugging.md`.
- **Hydration bridge.** The engineering centerpiece of Game 4. Translates between Layer 1 records and Layer 4 entities as the player moves through the world.
- **Glyph and Chronicle.** Two languages over one virtual machine. Each owns a domain. Detail in `glyph.md`, `chronicle.md`, and `language-binding.md`.

---

## Hydration: How Layer 1 and Layer 4 Meet

A character named Aldric exists, at all times, as a record in the world database. When the player approaches Aldric's village, Aldric is **hydrated** — a fresh ECS entity is spawned, all components attached at once, blackboard initialised from world knowledge. He becomes a body in the local simulation. When the player leaves, Aldric is **dehydrated** — relevant state is written back to his world record, the ECS entity is despawned.

This is **not** a component shuffle. It is spawn-and-despawn across an architectural boundary. The two representations have different shapes for good reasons: Layer 1 wants relational queries, Layer 4 wants composable component bags. The bridge is what makes them coherent.

The unsolved problem the bridge tackles is **catch-up**: when the player returns after thirty in-game days, Aldric needs plausible micro-state that is consistent with the macro-events Layer 2 simulated in his absence. World simulation provides ground truth (Aldric is alive, recently married); the bridge generates plausible local state (where he is standing now, what he is doing). Most NPCs reconstruct fine. Story-flagged characters get full state persisted across the boundary.

---

## Threading Direction

Single-threaded for Games 0 and 1. As load demands, threading splits along **layer boundaries**, not along subsystem boundaries.

- **Main thread** — Layer 4. ECS systems, physics, rendering submission, input, UI.
- **AI thread** — Layer 3. Blackboard population, utility evaluation, plan stepping. Produces a command buffer the main thread applies at a defined sync point.
- **World thread** — Layer 2. Chronicle evaluation, economy, faction power, history accumulation. Produces Tier 2 events the other layers consume on their own ticks.
- **IO thread(s)** — asset and chunk streaming. Always off the main thread.
- **Audio thread** — standard.

The discipline that lets us defer threading until it is needed: code is **written thread-ready** from Game 2. AI is structured as batched processing with command buffers from the day it appears, even when running on the main thread. World simulation is designed never to mutate ECS state directly. When threading lands, it is wiring, not rewriting.

---

## How the Architecture Grows Across Games

The plan introduces one layer at a time. The temptation to build for Game 4 in Game 0 is the temptation to build a castle out of mud — premature, brittle, wrong by the time you reach it.

- **Games 0–1.** Layer 4 only. ECS, physics, rendering, input, audio. Tier 1's mutation substrate from day one because it is cheap and painful to retrofit; add/remove detection and the push-subscription dispatch complete it in Game 1, alongside Tier 2. Tier 2 first appears in Game 1 with collision events crossing from physics into game logic.
- **Game 2A.** Glyph enters. Layer 4 grows script-driven game logic and UI. The script-engine boundary is established.
- **Game 2B.** A primitive Layer 3 appears: blackboard, utility, HTN. The horror enemy is the proving ground for the agent architecture Game 4 will scale.
- **Game 3.** Layer 4 grows outward — terrain, weather, vegetation, the full forward renderer. Layer 3 grows group behaviour and budgeted scheduling. **Immersive-sim foundations land here**: material reactions, environment response, the substrate Game 5's spell system will compose on.
- **Game 4.** Layer 1 (World State) and Layer 2 (World Simulation) come online. Chronicle ships. The hydration bridge ships. Tier 3 event accumulation ships. The full layered architecture is alive for the first time.
- **Game 5.** No new layers. Polish, character progression, and the immersive spell system that composes Game 3's substrate.

This staging is the entire risk-management strategy. Each game pays for the layer it adds; each layer earns its complexity by being demonstrated in a shipping game before the next is built.
