# Part 2 — Verbs

**Kind:** Tech buildout (physics & player abilities)

---

## Goal

The player's mechanical body. By end of Part 2, every Kinesis verb works in
the playground: hands push/pull/hold at close range (Mode 1, press `1`),
telekinesis at range with mouse-wheel distance control and throwing (Mode 2,
press `2`), repulsion against surfaces for short flight bursts (Mode 3, press
`3`). The world becomes physical — the sulfur dressing blocks become grabbable
and throwable, a cracked wall shatters on impact, a pressure plate and a
trigger volume drive reactions. A mode-tinted reticle and a mode indicator
render through a new screen-space UI pass; gameplay particles give the verbs a
tactile voice.

Underneath the verbs, Part 2 lays the substrate the rest of the engine has
been waiting for: the discrete-event + deferred-op + change-detection layer
([events.md], [ecs.md]), a general input-binding API ([input.md]), and the
Rapier bridge ([physics.md]). The first gameplay rules — the breakable wall,
the pressure plate — are authored in the poll-never-subscribe "rules over
state" shape so that Part 4's logic *extends* them rather than rewriting them.

This plan is built on the `plans/overview/` system docs, which describe the
as-built engine and each system's Part-2 roadmap. Each phase cross-references
the overview doc it derives from.

## The question this Part answers

**Do the verbs feel right?**

Telekinesis grab/throw/repulse is the game's mechanical voice — every chamber's
puzzle expression depends on these feeling tactile, precise, and a little
dangerous. The bar here is **correctness and tactile satisfaction**; final
weight/feel tuning happens against real chambers in Part 5. If the verbs don't
carry, fix the physics/force work before Part 3.

---

## In scope

- **ECS reactive substrate** — `Added<T>` / `Removed<T>` / `Changed<T>` filters,
  `Events<T>`, `Commands`, run-conditions. The spine of the first Tier-2 channel.
- **General input-binding API** (Layer 2 action map) + mouse-wheel input.
- **Typed debug-plugin framework** — a dev-flag-gated host with subsystem-owned
  plugins; Kinesis binds its episodic tweaks through the input-binding API.
- **Rapier integration**; the physics ↔ ECS bridge.
- **Character controller** — walking and jumping (Rapier KCC replaces the
  kinematic `fps_move` stand-in).
- **Force application** — directional, impulse, radial.
- **Hands (Mode 1)** — close-range push / pull / hold.
- **Telekinesis (Mode 2)** — the same verbs at range, mouse-wheel distance.
- **Throwing** — wheel-click launch while gripping with both buttons.
- **Repulsion (Mode 3)** — self-impulse against surfaces; short flight bursts.
- **Destructible wall** — mesh fragmentation on an impact threshold.
- **Pressure plates** (held-state) and **generic trigger volumes**.
- **Collision events** as the first Tier-2 cross-layer channel.
- **Line / gizmo pipeline** — physics debug-draw, in-world in dev mode.
- **Screen-space quad UI pass** — mode indicator + mode-tinted reticle.
- **Gameplay particles** — telekinesis hold field, repulsion impact ring,
  destruction debris.
- **First gameplay rules** authored in the rules-over-state shape — the Rust
  logic-scripting basics, extended (not rewritten) in Part 4.

## Out of scope

- First-person **hand mesh** with keyframed per-verb poses — **Part 3** (needs
  the keyframed transform-track animation Part 3 builds; pulling it forward
  would build animation early).
- **Glyph pictogram tutorial HUD** (5–6 pictograms) — **Part 3 / 5** (reuses the
  2.I screen-space UI pass).
- Tentacle entry / any Mahli interaction with held objects — **Part 3**.
- Audio for impacts, throws, repulsion — **Part 3**.
- Attitude, hunger, save, persistence — **Part 4**.
- Final telekinesis "weight" / "feel" tuning — **Part 5**, against real chambers.
- Render interpolation (`interpolation_alpha`), `GlobalTransform` / hierarchy,
  the scp inter-frame seam — deferred (see Followups).

---

## Phases

The dependency spine: the reactive substrate (2.A) and input API (2.B) come
first because everything consumes them; the debug rework (2.C) folds onto the
input API; physics (2.D) consumes the substrate; the line pipeline (2.E) lands
right after the bridge as a debugging multiplier; the controller (2.F) and
verbs (2.G) are the heart; interactables (2.H) are the first rules; UI +
particles (2.I) are the feel layer; the playground (2.Z) is the artifact.

### Phase 2.A — Reactive substrate (ECS)

Source: [ecs.md], [events.md]. Game 0 shipped only the *mutation* half of
Tier-1 change detection (`last_mutation_tick` + a pull-style `changed_since`
scan). Part 2 completes Tier-1 and adds the discrete-event and deferred-op
primitives. **Do this first** — it is the shared spine of physics events,
verbs, and the first gameplay rules. Non-visual but the riskiest to get wrong.

**Steps:**
- [x] **2.A.1** `added_tick` on the change ledger + a composable `Added<T>` query
  filter. Distinguishes "newly added" from "mutated" (lazy handle creation,
  on-add reactions). The tick struct was shaped to grow this without an API break.
- [x] **2.A.2** Removal / despawn signal. Today despawn drops the record; capture
  it instead: a per-frame removed-ledger (component-keyed list of entities whose
  `T` was removed, plus whole-entity despawn), drained once per frame, exposed as
  a `Removed<T>` reader. Needed for event-driven Rapier handle cleanup.
- [x] **2.A.3** `Changed<T>` composable query *filter* (today only the standalone
  `changed_since` scan exists). **Pin the change-tick cursor convention** against
  a concrete reaction before committing — the per-stage tick stride (FixedUpdate
  runs 0..N times/frame) makes a naive "since last frame" cursor over/under-count.
- [x] **2.A.4** `Events<T>` — a generic double-buffered discrete-event queue
  resource: `send`, drain-by-poll, buffer swapped exactly once per frame at a
  defined point in `App::tick` so a one-frame-late reader still sees the event.
- [x] **2.A.5** `Commands` — a deferred spawn / despawn / insert / remove buffer a
  non-exclusive system can hold, applied at a defined sync point after the systems
  that queue them. Local producers use boxed Rust closures for heterogeneous
  in-process mutations. Future external, AI, or threaded producers use structured
  messages suited to their boundary and converge on the same authoritative
  scheduler application seam, not necessarily this queue or representation.
- [x] **2.A.6** Run-conditions: `system.run_if(cond)` (a predicate over `World`,
  e.g. `in_mode(Telekinesis)`) so per-mode verb systems run only when active.
  Minimal — replaces the early-return idiom.
- [x] **2.A.7** Smoke test (throwaway unit reactions): mutate via `Mut` and assert
  `Changed`/`Added` fire; `send` an `Events<Ping>` and drain it next frame; queue
  a `Commands::despawn` and assert the entity is gone after apply and that
  `Removed` reports it.

### Phase 2.B — Input-binding API

Source: [input.md]. Part 2 forces a light Layer 2 (modes, verbs, wheel distance).
Build the **general** binding API here — named actions over physical triggers —
so the verbs, the character controller, and the debug rework all consume one
substrate. Keep action IDs as interned symbols (not a Rust enum) so the future
Glyph action registration is a translation, not a rewrite.

**Steps:**
- [x] **2.B.1** Mouse-wheel delta in Layer 1 `Input` (record from winit
  `MouseWheel`; accumulate per frame; cleared by `end_frame` like the motion
  delta). Telekinesis distance and the throw launch need it.
- [x] **2.B.2** Action map (Layer 2): a binding-table resource mapping
  interned-symbol action IDs → one-or-many physical triggers (key / mouse button /
  wheel sign), recomputed once per frame from Layer 1 on Update. Read helpers:
  `pressed`, `just_pressed`, `just_released`, `axis(neg, pos)`, `wheel`.
- [x] **2.B.3** Resolve the **FixedUpdate input hazard** (the convention, written
  down): edge reads happen once per render frame and write *state/intent*
  resources; FixedUpdate systems read state, never edges. The initial Update
  placement shipped here is superseded by 2.FH.5's pre-fixed control boundary,
  while the durable-state and latch semantics remain. (Edges are frame-scoped
  and cleared once/frame — reading them from a 0..N-step fixed stage
  double-fires or misses.)
- [x] **2.B.4** Convenience surface: register the gameplay actions (move axes,
  jump, mode 1/2/3, push/pull/grip/throw, repulse) through the table at game setup,
  with an ergonomic binding API (builder or simple insert).
- [x] **2.B.5** Smoke test: rebind "jump" to a different key at setup and confirm;
  log an action's `just_pressed`; read wheel delta in a system. (Migrating the FPS
  controller onto this is deferred to 2.F, where the KCC replaces `fps_move`.)

### Phase 2.D — Rapier core + the physics ↔ ECS bridge

Source: [physics.md]. New dependency (`rapier3d`). The substrate is ready: a
deterministic capped fixed-step, a flat decomposed `Transform` at the engine root
(the cleanest write-back target), and `fps_move` as the sole — explicitly
kinematic — translation writer.

**Steps:**
- [x] **2.D.1** Add `rapier3d`; stand up `PhysicsWorld` + pipeline + integration
  params + collision-event channel as resources in `App::resumed` (isolated block).
  Set `integration_parameters.dt = Time::fixed_delta`; step once per `run_fixed`.
- [x] **2.D.2** **Entity ↔ handle convention** (decide first — everything hangs off
  it): `RigidBody` / `Collider` authoring components (body type, shape, mass,
  material) + a bidirectional `EntityId ↔ Handle` map in the physics resource.
- [x] **2.D.3** Body / collider lifecycle: on `Added<RigidBody>` (2.A.1) create the
  Rapier body + collider and record the mapping; on `Removed<RigidBody>` / despawn
  (2.A.2) free the handle. Event-driven, not O(handles)/step.
- [x] **2.D.4** The bridge — one exclusive `fn(&mut World)` physics system,
  scheduled between fixed-step intent writers and outcome readers: reconcile → sync
  authored static / kinematic poses → `step()` → write dynamic poses back (`t.translation = body.translation();
  t.rotation = body.rotation()`) → drain Rapier collision / sensor events into
  `Events<Contact>` / `Events<TriggerEnter>` / `Events<TriggerExit>`. Use **contact impulse**, not velocity,
  as the `Contact` payload (it already integrates mass × Δv).
- [x] **2.D.5** Smoke test: spawn a dynamic cube above a static floor collider — it
  falls, collides, and rests, rendering for free via Transform write-back. Drop two;
  they stack.
- [x] **2.D.6** Experiment (pillar 4): toss a handful of dynamic cubes into the
  chamber and watch them tumble and settle — the first taste of the world becoming
  physical. If the settling looks wrong, the bridge ordering is the suspect.

### Phase 2.C — Debug-system rework

Source: [debugging.md]. This phase establishes the Rust-side deep-inspection
framework used to understand the running engine. It is distinct from the future
in-game debugging facilities exposed through Glyph, Chronicle, or other authored
languages.

The debug core is a host, not a catalogue of every subsystem's controls. A
dedicated `dev-tools` build flag makes debug tooling available, and the active
binary conditionally installs strongly typed Rust plugins at app composition.
The registry is dynamic at runtime, but plugins are not dynamically linked
libraries: plugin types, resources, systems, and panel callbacks remain checked by
Rust. The renderer owns render-debug state, physics owns physics visualization,
audio will own audio-source inspection, and Kinesis owns its episodic mood
presets. The core owns only plugin composition, the egui shell, visibility, and
panel registration.

Production resources remain authoritative and usable without debug tooling. A
debug plugin may inspect or mutate its subsystem's typed resources, but target
subsystems never read a generic `DebugState` value bag. Debug keybindings use the
2.B named-action path with namespaced symbols.

**Steps:**
- [x] **2.C.1** Pin the architecture: statically typed Rust plugins, dynamically
  composed into the running app under the `dev-tools` flag; a runtime panel
  registry; subsystem ownership; ordinary namespaced action bindings; no Rust
  dynamic-library ABI and no untyped debug-value bag.
- [x] **2.C.2** Add the small general app-composition seam: a typed `Plugin`
  contract, `App::add_plugin`, and conditional plugin installation under the
  `dev-tools` flag. Provide a convenience engine-debug group without making the
  debug core depend on its member plugins.
- [x] **2.C.3** Build the minimal debug core: overlay visibility, a panel registry,
  and an exclusive Render-stage UI-build system. Move egui frame construction out
  of `render_frame`; the renderer only encodes the prepared overlay pass. Panel
  callbacks receive typed access through `World` and run sequentially.
- [x] **2.C.4** Split intrinsic tooling into owner-side plugins: diagnostics owns
  frame statistics + puffin UI; assets own F5 reload; rendering owns its internal
  controls. Each registers its resources, systems, actions, and panel with the
  core. The core imports none of their target resources.
- [x] **2.C.5** Correct render-resource ownership. Keep one render-owned
  `PcfKernel` resource read by `render_frame`; remove the stale debug copy and the
  renderer's private hardcoded copy. Make GPU startup preserve preconfigured
  production render resources, and move Kinesis mood defaults out of engine
  startup.
- [x] **2.C.6** Add a game-owned `KinesisRenderDebugPlugin`: restore grade /
  vignette / fog / bloom / overlay cycles as namespaced action bindings and typed
  systems over the existing render resources. No Kinesis preset type lives in the
  engine.
- [x] **2.C.7** Smoke test both compositions. With `dev-tools`, F12, profiler, F5,
  renderer controls, and Kinesis mood cycles work in the playground. Without the
  flag, the app exposes no debug overlay or debug bindings and production render
  state is unchanged. `debug.rs` imports no game or target-subsystem resources.


### Phase 2.E — Physics debug-draw (the line / gizmo pipeline)

Source: [graphics.md] — "the one genuinely new GPU primitive Part 2 needs." All
existing pipelines are TriangleList + Fill. Build the line pipeline once and reuse
it for colliders, force vectors, trigger volumes, and the future in-game gizmos.
Land it right after the bridge so it is a debugging multiplier for everything that
follows. Drawn **into the live game world in dev mode** — there is no editor
viewport.

**Status:** Intentionally deferred while the blocking character-control runtime
is hardened. It remains required by the Part 2 done bar and returns before Part 2
closes, but it does not block 2.FH or ordinary 2.G force work.

**Steps:**
- [ ] **2.E.1** `LineList` pipeline: a new wgpu pipeline (LineList topology) + a
  per-frame growable vertex buffer (position + color) + a pass slot carved into
  `render_frame`, composited into the live frame and toggled by a dev flag.
- [ ] **2.E.2** Feed Rapier's `DebugRenderBackend` into the line buffer (collider
  wireframes, contact points). Toggle on/off via a 2.C debug binding.
- [ ] **2.E.3** A small immediate dev-draw API (`debug_line(a, b, color)`,
  `debug_ray`) usable from game systems for force vectors and pick rays.
- [ ] **2.E.4** Smoke test: collider wireframes overlay the falling / stacking cubes
  and the chamber geometry; toggle them with a debug key.

> Watch the `render_frame` body: it is a hard-coded inline pass list, not a render
> graph ([graphics.md]). The line pass is fine to inline. If 2.E + 2.I push it past
> ~3 more passes, factor a minimal pass-sequencing seam before it becomes a merge
> hazard — but not preemptively.

### Phase 2.F — Character controller (player physics)

Source: [physics.md] step 6. Replace `fps_move`'s direct translation write with a
Rapier `KinematicCharacterController`. Keep the camera **unparented** (its own
`Transform`) and copy `body.translation + eye_offset` into it each fixed step —
Rapier's own KCC pattern, and it matches the sibling-Transform convention the lights
already use. No hierarchy needed.

**Steps:**
- [x] **2.F.1** Player body: a capsule (kinematic-position-based) + a KCC. Camera
  stays its own entity / `Transform`.
- [x] **2.F.2** Walking: feed desired horizontal motion (from the 2.B action axes,
  captured on Update into an intent/state resource per 2.B.3) into the KCC; resolve
  against geometry; gravity + ground detection.
- [x] **2.F.3** Jumping: vertical impulse on the jump action when grounded; apply
  gravity to vertical velocity; land detection.
- [x] **2.F.4** Camera copy: each fixed step, `camera.translation = body.translation
  + eye_offset`. `fps_look` stays as-is on the camera Transform; retire the noclip
  `fps_move`.
- [x] **2.F.5** Smoke test: walk the chamber — can't clip walls, floor, or stacked
  cubes; jump and land; the tunnel / doorway gates the body correctly. This
  validation is held until the runtime corrections below; 2.FH.6 executes and
  satisfies this step rather than duplicating the same playtest.

**Implementation note:** The retired free-flight behavior survives only under
`dev-tools` as the engine-owned F8 spectator camera. It transfers `ActiveCamera`
to a separate entity and suspends physical-player intent while active.

### Phase 2.FH — Character-control runtime hardening

**Kind:** Correctness, scheduling, and diagnostics

**Placement:** After 2.F.4 and before the 2.F.5 validation

**Blocks:** 2.F.5 and 2.G

**Does not include:** Event fan-out, lifecycle burst optimization,
interpolation, external command ingress, or production walking-push

#### Goal

Make the physical player execute exactly once per fixed step, observe current
control state, traverse sensors correctly, and expose enough physics profiling
information to diagnose subsequent work.

**Steps:**

- [x] **2.FH.1 — Physics profiling scopes and reusable workload counters.**

  Add the missing puffin breakdown before changing performance-sensitive paths.
  The bridge currently exposes no way to distinguish lifecycle reconciliation,
  transform synchronization, character queries, Rapier solving, write-back, and
  event publication.

  The diagnostics must expose:

  - Lifecycle reconciliation.
  - Authored-transform synchronization.
  - Discrete physics-command processing.
  - Character integration and KCC queries.
  - Rapier solve.
  - Dynamic-pose write-back.
  - Contact and trigger publication.

  Reusable diagnostics state records relevant workload counts without
  constructing formatted profiler labels in the fixed-step path. Counts cover
  enough volume information to distinguish an expensive operation from an
  unusually large workload.

  This step adds no new dependency; puffin and its diagnostics UI already exist.

  **Implementation note (2026-07-30):** The bridge now emits static puffin scopes
  for all seven required phases, with character integration and the hosted KCC
  query separately attributable. The public `PhysicsDiagnostics` resource resets
  on puffin's render-frame boundary and accumulates lifecycle candidates,
  transform/command/controller volume, Rapier body-step samples, pose write-back,
  and published event counts across every fixed step in that frame.

- [x] **2.FH.2 — Correct change epochs and reactive-query cursor ownership.**

  The current one-epoch-per-stage convention cannot distinguish mutations
  occurring on opposite sides of a consumer inside that stage. A later mutation
  can receive the consumer's recorded cursor value and fail the strict
  `changed_tick > since` comparison forever.

  The runtime contract becomes:

  - Every scheduled system execution receives a distinct change epoch.
  - Every non-empty deferred-command application batch receives a distinct
    epoch.
  - Empty command barriers do not consume epochs.
  - The epoch remains a `u64` monotonic change-detection value, not a simulation
    or frame counter.
  - Overflow must not silently wrap into apparently valid old epochs.

  Cursor ownership becomes explicit:

  - Each scheduled system owns its last successful-run epoch.
  - `Query<_, Added<T>>` and `Query<_, Changed<T>>` receive that system-owned
    cursor.
  - A run condition that prevents the system body from executing does not
    advance its consumer cursor.
  - Explicit-cursor world APIs remain caller-owned.
  - Specialized exclusive consumers such as the physics bridge continue owning
    their explicit cursors and define their own first-run initialization.
  - The physics lifecycle consumer must observe complete physics authoring
    inserted at epoch zero.
  - No public cursorless reactive API may look like a valid per-run reactor while
    silently using zero forever.

  First-run semantics:

  - A scheduled reactive query's first execution observes all currently matching
    components, including components inserted at epoch zero.
  - Since an addition is also a change, first-run `Changed<T>` includes existing
    components.
  - Subsequent executions observe only changes newer than that system's last
    successful run.

  Regression coverage must include:

  - Epoch-zero insertion, including the physics bridge's explicit lifecycle
    cursor.
  - First execution and second execution of bare scheduled `Added`/`Changed`
    queries.
  - Producer before consumer.
  - Producer after consumer.
  - A non-empty command batch before and after a consumer.
  - Multiple independent consumers.
  - Run-condition false/true transitions.
  - Zero and multiple fixed steps per frame.
  - Explicit-cursor queries retaining their caller-owned semantics.
  - Defined overflow behavior.
  - The original same-epoch lifecycle-loss reproduction at the generic ECS
    level.

  **Implementation note (2026-07-30):** The scheduler now assigns a checked
  `u64` change epoch to every system dispatch and non-empty command batch.
  Parameter-injected systems retain an `Option<u64>` last-successful-run cursor,
  so first-run `Added` / `Changed` queries include epoch-zero state and later
  runs compare against that system alone; a false run condition leaves the
  cursor untouched. Cursorless world queries accept presence filters only,
  explicit-cursor APIs remain caller-owned, and the physics bridge uses explicit
  first-run sentinels for lifecycle and transform reconciliation. The regression
  suite covers both producer orderings, both command-batch orderings,
  independent consumers, run-condition transitions, fixed-step stride,
  tick-zero physics authoring, strict explicit cursors, and overflow refusal.

- [ ] **2.FH.3 — Sensor-transparent character movement.**

  The KCC movement query currently includes sensors, causing trigger volumes to
  behave as invisible walls.

  Required behavior:

  - Sensors are excluded only from the character movement query.
  - Sensors remain present in Rapier's normal collision/overlap pipeline.
  - Enter and exit events still carry the correct sensor and other entity.
  - Static and dynamic solid colliders continue obstructing the capsule.
  - Nested or overlapping sensors do not alter resolved movement.
  - A sensor placed directly across a corridor is traversable at normal walking
    speed.

- [ ] **2.FH.4 — One character integration per entity per fixed step.**

  `MoveCharacter` is currently an order-sensitive discrete command. Each
  occurrence performs a full KCC integration, including gravity. Two commands
  for one entity integrate twice; no command means gravity and grounding do not
  advance.

  Continuous character movement becomes persistent per-entity intent consumed
  exactly once by the bridge on every fixed step.

  Required semantics:

  - Every active character controller integrates exactly once per fixed step.
  - Absence of movement input means zero horizontal intent, not absence of
    simulation.
  - Gravity, grounding, snapping, and landing continue without a movement
    submission.
  - Each entity has at most one effective motion intent for a step.
  - Jump remains a latched one-shot request and is consumed at most once.
  - Continuous movement no longer depends on command ordering.
  - Teleports remain discrete physics commands.
  - Multiple characters scale by controller count, not by arbitrary command
    multiplicity.
  - A character whose controls are disabled still receives zero-input physics
    integration.

  Regression coverage must prove:

  - Zero horizontal input still advances gravity once.
  - Repeated intent writes cannot cause repeated integration.
  - Jump and movement in the same step do not depend on submission order.
  - A latched jump survives a render frame with no fixed step.
  - A jump is not repeated across several fixed steps.
  - Grounding continues while controls are inactive.

- [ ] **2.FH.5 — Pre-fixed control sampling and ownership.**

  The current frame order runs fixed simulation before action resolution, look
  processing, movement capture, mode changes, and spectator handoff. The
  physical player consequently acts on previous-frame control and aim state.

  A defined control-sampling boundary establishes the authoritative control
  snapshot before fixed simulation. This supersedes 2.B.3's one-frame-late
  placement while preserving its durable-state rule: frame-scoped edges are
  still sampled once per render frame, latched, and never read independently by
  each fixed step.

  The boundary includes:

  - Named-action resolution.
  - Cursor/input-capture gating.
  - F8 spectator ownership transition.
  - Aim/yaw update used by movement and future targeting.
  - Mode changes needed by fixed-step verbs.
  - Held movement intent.
  - Latched jump and future one-shot verb intent.

  Required semantics:

  - The first eligible fixed step uses the current control snapshot.
  - Aim direction, movement direction, and future verb targeting agree within
    that step.
  - Zero-fixed-step frames retain one-shot requests.
  - Multi-fixed-step frames consume one-shot requests once while continuing held
    intent.
  - Activating the spectator clears or suspends physical-player intent before
    physics consumes it.
  - Returning from spectator mode does not replay stale movement, aim delta, or
    jump.
  - UI-captured input cannot leak into the player or spectator.
  - Action resolution remains proportional to bindings once per render frame and
    independent of actor count.
  - The later Update stage remains available for non-control variable-rate
    gameplay.

- [ ] **2.FH.6 — Character smoke test and recorded experiments.**

  Run and close the original 2.F.5 validation against the corrected scheduling
  and integration model:

  - Walls, floor, doorway, tunnel, and stacked cubes obstruct the capsule.
  - Walking, jumping, falling, landing, slopes, and ground snapping behave
    consistently.
  - Sensors are traversable while producing overlaps.
  - F8 transitions cleanly between physical player and spectator.
  - Puffin attributes physics time to the new subscopes.

  Record, but do not silently canonize, two experiments:

  **Walking-push A/B**

  Compare ordinary solid obstruction with Rapier's approximate character impulse
  response.

  Evaluate:

  - Light and heavy block response.
  - Stack stability.
  - Sustained contact.
  - Pushing a block near an edge.
  - Whether walking-push weakens Mode 1's identity.
  - Whether the behavior feels intentional rather than like solver leakage.

  Production behavior changes only after an explicit design decision.

  **High-refresh presentation**

  Observe player and dynamic-body motion above the 60 Hz fixed rate. Record
  whether judder is visible and materially harms play. Do not implement
  interpolation in this phase.

#### 2.FH done bar

- Reactive scheduled queries have real per-system cursors and do not replay from
  epoch zero.
- Same-stage and deferred mutations cannot be permanently hidden.
- Sensors do not block the KCC.
- Every character integrates exactly once per fixed step.
- Current aim and control ownership are established before physics.
- The original 2.F.5 smoke test passes.
- Walking-push and high-refresh behavior have recorded results, not assumed
  conclusions.
- Physics costs are attributable through puffin.

### Phase 2.G — Verbs & modes (force application)

The heart of Part 2. Modes are **state**; the reticle / indicator / active verb all
**derive** from the mode; physics applies forces in FixedUpdate reading the
pre-fixed control snapshot established by 2.FH.5. Verb mappings from
`design/systems.md`: Mode 1 hands
(left = push, right = pull, both = grip); Mode 2 telekinesis (same at range, wheel =
distance); throw (both-buttons grip + wheel-click launch); Mode 3 repulsion
(self-impulse against a surface).

**Steps:**
- [ ] **2.G.0** Resolve playground mouse-action ownership. The playground
  currently binds left mouse simultaneously to the editor spawn action, Mode 1
  push, and Mode 3 repulsion. Push and repulsion are disambiguated by
  `ControlMode`; the unrelated editor action must be removed, remapped, or gated
  behind an explicit dev-only editing state. A gameplay click produces only the
  active mode's verb.
- [ ] **2.G.1** `ControlMode` state resource (`Hands` / `Telekinesis` / `Repulsion`);
  `mode_select` runs at the 2.FH.5 control-sampling boundary, reads the 1/2/3
  actions, and writes the mode — the one place mode changes. Run-conditions
  (2.A.6) gate the per-mode systems.
- [ ] **2.G.2** Force-application primitives on the physics side: apply impulse /
  continuous force / radial impulse to a body by `EntityId`. FixedUpdate.
- [ ] **2.G.3** Pick / target: a camera-forward raycast (Rapier query) to find the
  targeted body + hit point; expose the current target as state (drives the reticle
  and the hold field).
- [ ] **2.G.4** Hands (Mode 1): close-range push (left), pull (right), hold/grip
  (both) — spring the held body toward a hold point in front of the camera at a fixed
  close range.
- [ ] **2.G.5** Telekinesis (Mode 2): the same grip at range; mouse-wheel adjusts the
  hold distance (clamped); push / pull as impulses along the view ray.
- [ ] **2.G.6** Throw: while gripping (both buttons), wheel-click launches the held
  body forward with an impulse; releasing grip drops it.
- [ ] **2.G.7** Repulsion (Mode 3): self-impulse — cast toward a surface, apply an
  impulse to the *player* body away from it; enables wall-jumps and short flight
  bursts. Tune burst magnitude + a brief cooldown for control.
- [ ] **2.G.8** Smoke test: enter each mode (a debug readout reflects it until 2.I's
  reticle lands). Grab a cube at close range and at distance; wheel changes distance;
  throw it across the room; repulse off a wall for a flight burst.
- [ ] **2.G.9** Experiment (pillar 4): the "juggle" loop — hold a cube, wheel it out,
  throw it at a stack, then repulse-dodge backward. If that loop feels good, the
  verbs are landing. This is the milestone question made concrete.

### Phase 2.HP — Interactable substrate hardening

**Placement:** After 2.G and before 2.H

**Blocks:** Trigger-driven rules, destructibles, and independent contact
consumers

**Does not block:** 2.F.5 or ordinary 2.G force work

**Steps:**

- [ ] **2.HP.1 — Cursor-based event fan-out.**

  Adopt a bounded per-consumer guarantee:

  - Every event receives a monotonic sequence ID with defined overflow behavior.
  - Each consumer owns its own typed cursor.
  - Reading advances only the caller's cursor.
  - New cursors explicitly start "from now" or "from oldest retained."
  - Falling behind retained history produces a detectable missed-event result.
  - The event queue does not centrally register consumers.
  - Reader destruction therefore requires no queue cleanup.
  - Independent breakage, audio, particle, trigger, and debug consumers cannot
    steal or replay one another's events; queue-wide draining is not a normal
    reader API.
  - Event order remains stable across multiple fixed steps in one render frame.

  The contract is exactly once while the cursor remains within retained history,
  not "exactly once forever."

- [ ] **2.HP.2 — Lifecycle candidate burst scaling.**

  Remove the demonstrated quadratic behavior from physics authoring
  reconciliation.

  Preserve:

  - One reconciliation per affected entity even when several authoring
    components change.
  - Correct incomplete-body handling.
  - Remove/reinsert behavior.
  - Body and collider authoring updates.
  - Deterministic results.

  Retain a persistent benchmark covering at least 100, 500, 1,000, and 2,000
  candidates, with the reported measurements recorded as the before baseline.

- [ ] **2.HP.3 — Fragment-style deferred materialization regression.**

  Reproduce the actual future pipeline:

  ```text
  Contact → PostPhysics rule → Commands spawn fragments
  ```

  Verify that a batch of fragment entities authored with `Transform`,
  `RigidBody`, and `Collider` is materialized on the next physics step, exactly
  once, without missing bodies or duplicate Rapier handles.

  This complements the generic epoch suite with the real 2.H integration path.

- [ ] **2.HP.4 — Integrated trigger and destruction substrate check.**

  Confirm that:

  - Sensors remain traversable.
  - Trigger readers receive enter/exit once per cursor.
  - Multiple readers see the same event independently.
  - Fragment batches reconcile within the measured budget.
  - PostPhysics structural commands remain visible to physics.
  - Puffin exposes lifecycle and event-publication burst costs.

Queue-capacity reuse is not a standalone task. If the epoch work naturally
changes command-batch ownership, retained capacity may be preserved as local
hygiene. Otherwise it remains deferred until measurement justifies it.

### Phase 2.H — Interactables & the first gameplay rules

The Rust logic-scripting basics. These are the first Tier-2 consumers, authored in
the poll-never-subscribe / rules-over-state shape ([events.md], `development.md`) so
Part 4 *extends* them, not rewrites. Deliberately minimal — one of each rule shape.

**Steps:**
- [ ] **2.H.1** Trigger volumes: Rapier sensor colliders, drained by the bridge into
  `Events<TriggerEnter>` / `Events<TriggerExit>`. A generic `Trigger` component.
- [ ] **2.H.2** Pressure plate: a held-state sensor — derive a `Pressed` *state* from
  current overlaps (state, not a one-shot event), so "while held" reactions read
  state. Drives a visible demo reaction (a light or material change).
- [ ] **2.H.3** Destructible wall: a `Breakable { impulse_threshold }` component; a
  system reads `Events<Contact>` and, when impulse ≥ threshold, swaps the wall mesh
  for authored physics fragments via `Commands` (despawn intact + spawn fragments as
  dynamic bodies). One cracked-wall asset.
- [ ] **2.H.4** Establish the reaction-authoring pattern explicitly: one event→state
  rule (contact → break) and one state→derived-signal rule (plate pressed → effect),
  written the portable way. Part 4's attitude / hunger / eye-state rules slot into the
  same shape; this is the seed they extend.
- [ ] **2.H.5** Smoke test: throw a cube at the cracked wall hard enough → it shatters
  into fragments that fall and settle; stand on the plate → the reaction holds while
  you're on it, releases when you step off.

### Phase 2.I — Screen-space UI pass + gameplay particles

The feel-critical feedback layer. A **new screen-space textured-quad UI pass** (the
HUD render decision) draws camera-independent quads for the reticle and mode
indicator, reusable later for the glyph pictograms. CPU particles give the verbs a
tactile voice. No ambient / environmental particles (design constraint).

**Steps:**
- [ ] **2.I.1** Screen-space quad UI pass: a new pass in `render_frame` drawing
  quads in screen/NDC space, with a small `Hud` submission API. Reusable for reticle,
  mode indicator, and (Part 3/5) glyph pictograms.
- [ ] **2.I.2** Reticle: a center reticle quad whose tint derives from `ControlMode`
  (white / topaz / red). A derived signal — reads mode state, never subscribes.
- [ ] **2.I.3** Mode indicator: a small HUD element (icon or label quad) showing the
  active mode.
- [ ] **2.I.4** Gameplay particles (CPU): a minimal particle system + emitters —
  telekinesis hold field (topaz, around the held/targeted body), repulsion impact
  ring (red, at the surface point), destruction debris (on wall break).
- [ ] **2.I.5** Smoke test: each mode shows its reticle tint + indicator; holding
  shows the topaz field; repulsion shows the ring; breaking the wall kicks debris.

### Phase 2.Z — The playground becomes physical

The artifact every subsequent Part returns to. Dress the Part-1 playground with the
verbs' content and tune first-pass feel.

**Steps:**
- [ ] **2.Z.1** Make the sulfur dressing blocks dynamic rigid bodies
  (grabbable / throwable); give the chamber walls and floor static colliders.
- [ ] **2.Z.2** Add a cracked / breakable wall panel (2.H.3) somewhere reachable;
  author its fragments.
- [ ] **2.Z.3** Add a pressure plate (2.H.2) and a generic trigger volume (2.H.1),
  each wired to a visible demo reaction.
- [ ] **2.Z.4** First-pass feel tuning: masses, grip spring stiffness, telekinesis
  distance range, throw impulse, repulsion burst + cooldown, contact / destruction
  thresholds. Bar = correctness + tactile satisfaction; final tuning is Part 5.
- [ ] **2.Z.5** Done-bar walk: switch modes 1/2/3, grab a block close and at range,
  throw it, smash the cracked wall, repulse off a surface, trip the plate and the
  trigger volume. Everything reads and feels deliberate.

---

## Done Bar

The Part is complete when all of the following are true in the playground binary:

- [ ] Walking and jumping run through the Rapier character controller; the player
  can't clip the chamber, floor, or stacked objects.
- [ ] Mode 1 (Hands) grabs / pushes / pulls / holds at close range (press `1`).
- [ ] Mode 2 (Telekinesis) grabs and throws at range with mouse-wheel distance
  (press `2`).
- [ ] Mode 3 (Repulsion) self-impulses against surfaces for short flight bursts
  (press `3`).
- [ ] Dynamic objects are grabbable, throwable, and collide; at least one breakable
  wall shatters on an impact threshold.
- [ ] A held-state pressure plate and a generic trigger volume each drive a visible
  reaction.
- [ ] Collision events flow as the first Tier-2 channel; the breakable wall and
  pressure plate are authored as rules over state (the Rust-logic basics that Part 4
  extends, not rewrites).
- [ ] A mode-tinted reticle and a mode indicator render through the screen-space UI
  pass; hold-field / impact-ring / debris particles fire.
- [ ] The `dev-tools`-gated debug host dynamically composes typed, subsystem-owned
  plugins; Kinesis owns its render-mood tweaks and the core owns no target state.
- [ ] Physics debug-draw (collider wireframes) toggles on in dev mode.
- [ ] Walking the playground, the developer believes the verbs feel right.

If the last bullet fails, tune forces / masses / feel before starting Part 3.
**Don't proceed on verbs that don't carry.**

---

## Followups

- Final telekinesis "weight" / "feel" tuning → **Part 5**, against real chambers.
- First-person **hand mesh** + keyframed per-verb poses → **Part 3** (needs the
  keyframed transform-track animation Part 3 builds).
- **Glyph pictogram tutorial HUD** (5–6 pictograms) → **Part 3 / 5** (reuses the 2.I
  quad UI pass).
- **Render interpolation** via `interpolation_alpha` → when frame rate visibly
  decouples from the fixed rate and the 2.FH.6 high-refresh experiment shows a
  material defect. Any accepted design must account for its approximately
  one-fixed-step presentation delay.
- **`GlobalTransform` / hierarchy** → only when nested / articulated rigs appear
  (turret-on-vehicle, joint-anchored props); not Part 2.
- **External/concurrent ingress** → designed when a real producer supplies
  payload, ordering, backpressure, and failure requirements. It uses structured
  boundary messages and converges with local `Commands` at the authoritative
  scheduler application seam; it does not enqueue boxed Rust closures.
- **Generic command-queue allocation optimization** → only with supporting
  measurements. Queue-capacity reuse may land as local hygiene if 2.FH.2
  naturally changes command-batch ownership.
- **Production walking-push** → only after the explicit 2.FH.6 A/B decision.
- **Doc debt**: update each `plans/overview/*` system doc as its phase lands; the
  reactive-substrate work also lets `architecture/render.md`'s stale status section
  and the `architecture/reactivity.md` cursor convention be re-baselined.

[ecs.md]: ../../../plans/overview/ecs.md
[events.md]: ../../../plans/overview/events.md
[physics.md]: ../../../plans/overview/physics.md
[input.md]: ../../../plans/overview/input.md
[graphics.md]: ../../../plans/overview/graphics.md
[bridge.md]: ../../../plans/overview/bridge.md
[debugging.md]: ../../../plans/architecture/debugging.md
