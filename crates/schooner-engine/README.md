# schooner-engine

The Schooner engine library: the ECS, the wgpu renderer, the input layer, the time + scheduler, the camera/transform primitives, and the egui debug overlay. All Game 0 functionality lives here; the `game-void` binary is just a `main` that builds an `App`, spawns a scene, and calls `run()`.

For the why behind specific subsystems, see `architecture/render.md`, `architecture/camera.md`, `architecture/input.md` at the repo root. For the phase-by-phase plan, see `plans/game0-plan.md`.

---

## The shape of the engine

Three things to internalize before reading any module:

1. **Everything is a system reading resources and querying components.** The renderer is a system. The FPS controller is two systems. Input update is event handlers writing into a resource. There is no parallel "engine subsystem with its own loop." The `Schedule` is the only loop.

2. **The world holds two kinds of state: components and resources.**
   - **Components** belong to entities. Entities are bags of components; sparse-set storage means add/remove is O(1) per component, with no archetype migration. Built around the long-term need for a scripting language (shik) where entities are runtime-composable property sets.
   - **Resources** are world-scoped singletons. The wgpu device, the input snapshot, the time clock, the mesh registry, the debug state — all resources. Systems request them by type via `Res<T>` / `ResMut<T>`.

3. **Three stages, in order, every frame**: `FixedUpdate` (0..N times via the accumulator) → `Update` (once) → `Render` (once). Each stage runner bumps `World::current_tick` once. Per-component change ticks bump on `&mut` access — the substrate for the reactive cascade engine that arrives in Game 2; nothing consumes them yet.

If a piece of code wants to do work each frame and isn't a system, it's probably misplaced.

---

## Module map

```
src/
├── lib.rs              — re-exports the public surface
├── app.rs              — App: window + world + schedule + tick loop
├── time.rs             — Time resource + fixed-step accumulator
├── ecs/                — sparse-set storage, queries, scheduler
├── window/             — winit wrapper
├── input.rs            — keyboard + mouse state, cursor grab
├── transform.rs        — Transform component (scene-graph primitive)
├── camera/             — Camera + Projection + ActiveCamera + FpsController
├── render/             — wgpu device, forward pipeline, mesh registry, render_frame, egui overlay
├── debug.rs            — DebugState, FrameStats, OverlayMetrics, ProfilerView
├── diagnostics.rs      — log_fps_system, log_input_system (optional, builder-opt-in)
├── logging.rs          — env_logger wrapper with a sane fallback filter
└── error.rs            — engine-level error type
```

`Transform` lives at the crate root rather than under `render/` because it's a scene-graph primitive shared across camera, physics, audio, and AI in later games — putting it inside `render/` would force every later subsystem to import the renderer for its pose type.

---

## How to add a component

Components are plain Rust types. There is no derive — any `'static + Send + Sync` type is a component the moment something stores it on an entity.

The mental model: pick a name, define a struct (or tuple struct), spawn it onto an entity. The world auto-registers the component type the first time it sees it. Systems can immediately query it.

The decisions that matter:

- **Where the type lives.** Engine-intrinsic components (`Transform`, `Camera`, `MeshHandle`, `DirectionalLight`) live in the engine crate. Game-defined components (later: `Health`, `Faction`, etc.) live in the game crate. The renderer only consumes engine-side types.
- **What it carries.** Components are pure data. No methods that touch other components, no cross-component logic. Behavior lives in systems.
- **Whether it's a tag.** A unit struct (e.g. `ActiveCamera`) is a marker — it costs almost nothing in the storage and lets queries filter by presence.

Tag components and their consumers are the cleanest way to express "which entity is special" without baking the choice into a resource.

---

## How to add a system

Systems are plain functions. Their parameter types declare what they read and write.

The mental model: write a function that takes the resources and queries it needs, register it on a stage, and the scheduler does the rest. The function signature is the contract — change the signature, and the scheduler sees the change at registration time.

The decisions that matter:

- **Which stage.** `Update` for variable-tick gameplay (input, camera). `FixedUpdate` for deterministic simulation (physics in Game 1, AI ticks in later games). `Render` for the forward pass and the overlay; the engine appends `render_frame` to this stage from `App::resumed` after the user's builder chain.
- **What to ask for.** Request the smallest set that does the job. The scheduler validates that no two systems claim conflicting `&mut` access to the same component or resource — a violation panics at registration, not on the first frame.
- **Mutation through queries.** Use the read-only-vs-write parameter shapes that the query layer exposes; the change-detection ticks bump on the right access by construction. Reading does not bump ticks.
- **When to go exclusive.** A system that genuinely needs the whole world (`fn(&mut World)`) is registered via the `exclusive` wrapper. `render_frame` does this — it touches more resources than the typed-tuple `SystemParam` arity supports, and it's the last system every frame so parallelism would buy nothing. The general guidance is: prefer typed parameters; reach for exclusive only when the typed surface starts fighting you.

A system that wants to run last in its stage just registers last. No dependency graph, no ordering DSL — registration order is the schedule. This is a deliberate Game 0 simplification; a real ordering surface arrives when the parallel scheduler does (likely Game 4).

---

## What's deliberately not here yet

These come in later games — listed here so a reader doesn't waste time looking:

- **Asset loading** (glTF, textures from disk) — Game 1/2.
- **Physics integration** (Rapier, collision events, force application) — Game 1.
- **Scripting language integration** (the shik VM, hot-reload, dynamic queries) — Game 2.
- **Reactive cascade engine** (subscribers firing on component change) — Game 2. The change-detection substrate exists; the cascade engine consumes it later.
- **Multiple light types, shadows, PBR, deferred shading** — Game 2 onward.
- **Terrain streaming, LOD, vegetation, weather** — Game 3.
- **Utility AI, NPC needs simulation, faction system** — Game 4.
- **Parallel scheduling, Changed<T> query filter, dense-view hot caches** — added when profiling demands.

The Game 0 done-bar is in `plans/game0-plan.md` §1.6. Anything beyond it is intentional Game 1+ scope.

---

## Where the design lives

- **Roadmap** — `plans/plan.md` (Games 0–5, what each game adds, releasable evaluations).
- **Game 0 architecture** — `plans/game0-plan.md` (resolved decisions, module-by-module design, ordered phase list).
- **Subsystem design notes** — `architecture/{render,camera,input}.md` (idea-level — no struct shapes or method signatures that rot; the code is the source of truth for those).

When the architecture docs and the code disagree, the code is right and the doc is stale — open an issue.
