# Part 2 — Verbs

**Kind:** Tech buildout (physics & player abilities)
**Status:** In progress — 2.A
**Depends on:** Part 1 (Mood) complete

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
- **Debug-system rework** — split engine-intrinsic dev tooling from game-owned
  episodic tweaks; the game binds its tweaks through the input-binding API.
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
  that queue them. Shape it so an out-of-process actor (scp, [bridge.md]) could
  enqueue into the same path later — it is one more producer, not a parallel path.
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
  down): edge reads stay on Update and write *state/intent* resources; FixedUpdate
  systems read state, never edges. The once-per-frame action resolve makes this the
  natural path. (Edges are frame-scoped and cleared once/frame — reading them from
  a 0..N-step fixed stage double-fires or misses.)
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
- [ ] **2.D.3** Body / collider lifecycle: on `Added<RigidBody>` (2.A.1) create the
  Rapier body + collider and record the mapping; on `Removed<RigidBody>` / despawn
  (2.A.2) free the handle. Event-driven, not O(handles)/step.
- [ ] **2.D.4** The bridge — one exclusive `fn(&mut World)` FixedUpdate system,
  registered *after* gameplay Transform writers: sync changed Transforms → bodies
  → `step()` → write dynamic poses back (`t.translation = body.translation();
  t.rotation = body.rotation()`) → drain Rapier collision / sensor events into
  `Events<Contact>` / `Events<TriggerEnter>`. Use **contact impulse**, not velocity,
  as the `Contact` payload (it already integrates mass × Δv).
- [ ] **2.D.5** Smoke test: spawn a dynamic cube above a static floor collider — it
  falls, collides, and rests, rendering for free via Transform write-back. Drop two;
  they stack.
- [ ] **2.D.6** Experiment (pillar 4): toss a handful of dynamic cubes into the
  chamber and watch them tumble and settle — the first taste of the world becoming
  physical. If the settling looks wrong, the bridge ordering is the suspect.

### Phase 2.C — Debug-system rework

The engine's `debug.rs` is a monolith that mixes engine-intrinsic dev tooling
(frame stats, profiler, overlay shell, asset reload) with game-specific episodic
render-mood cycling (fog / grade / vignette / bloom / overlay / PCF presets). Split
it: the engine keeps the intrinsic half; the game owns its tweaks, bound through
the 2.B input API. Confirmed coupling to remove: `render::forward` reads
`DebugState.pcf_kernel` (`forward.rs:268`) and `DebugState.frame_stats`
(`forward.rs:899`).

**Steps:**
- [ ] **2.C.1** Decide the debug surface now that the binding API exists: debug
  keybindings are ordinary action bindings (optionally dev-only-flagged), not a
  bespoke mechanism. (This is the API-shape decision deliberately deferred until
  2.B existed.)
- [ ] **2.C.2** Promote the shipping render knob out of `DebugState`: `pcf_kernel`
  becomes a small engine render resource (a peer of `ColorGrade` / `Fog` /
  `Bloom`), read directly by `render_frame`. `DebugState` carries no render state.
- [ ] **2.C.3** Strip engine `debug.rs` to engine-intrinsic only: `FrameStats`,
  `ProfilerView` / snapshot / panel, the egui overlay shell + `build_overlay_ui`,
  `overlay_visible` / `show_profiler`. Engine keeps `f5_reload_system` and the
  profiler toggle. Remove the `*Preset` enums, `debug_input_system`, and
  `bloom_input_system` from the engine.
- [x] **2.C.4** Move the game's episodic render-mood cycles game-side: the
  grade / vignette / fog / bloom / overlay become game-owned, registered
  as action bindings (2.C.1) in the game setup. They poke the same engine render
  resources, just from the game crate. PCF cycles no more needed
- [ ] **2.C.5** Overlay extensibility (keep minimal): give the game a small hook to
  contribute to the egui overlay if it wants a dev readout, or let it render its own
  — whichever is lighter. Don't over-build; the binding API is the main surface.
- [ ] **2.C.6** Smoke test: the same keys still cycle fog / grade / etc., but the
  cycling code lives in the game crate; engine `debug.rs` no longer imports `Fog` /
  `ColorGrade` / `Bloom` / `Vignette` / `PostOverlay`; profiler + F5 reload remain
  engine-side and still work.


### Phase 2.E — Physics debug-draw (the line / gizmo pipeline / profiling)

Source: [graphics.md] — "the one genuinely new GPU primitive Part 2 needs." All
existing pipelines are TriangleList + Fill. Build the line pipeline once and reuse
it for colliders, force vectors, trigger volumes, and the future in-game gizmos.
Land it right after the bridge so it is a debugging multiplier for everything that
follows. Drawn **into the live game world in dev mode** — there is no editor
viewport.

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
- [ ] **2.F.1** Player body: a capsule (kinematic-position-based) + a KCC. Camera
  stays its own entity / `Transform`.
- [ ] **2.F.2** Walking: feed desired horizontal motion (from the 2.B action axes,
  captured on Update into an intent/state resource per 2.B.3) into the KCC; resolve
  against geometry; gravity + ground detection.
- [ ] **2.F.3** Jumping: vertical impulse on the jump action when grounded; apply
  gravity to vertical velocity; land detection.
- [ ] **2.F.4** Camera copy: each fixed step, `camera.translation = body.translation
  + eye_offset`. `fps_look` stays as-is on the camera Transform; retire the noclip
  `fps_move`.
- [ ] **2.F.5** Smoke test: walk the chamber — can't clip walls, floor, or stacked
  cubes; jump and land; the tunnel / doorway gates the body correctly.

### Phase 2.G — Verbs & modes (force application)

The heart of Part 2. Modes are **state**; the reticle / indicator / active verb all
**derive** from the mode; physics applies forces in FixedUpdate reading the Update-
captured intent (2.B.3). Verb mappings from `design/systems.md`: Mode 1 hands
(left = push, right = pull, both = grip); Mode 2 telekinesis (same at range, wheel =
distance); throw (both-buttons grip + wheel-click launch); Mode 3 repulsion
(self-impulse against a surface).

**Steps:**
- [ ] **2.G.1** `ControlMode` state resource (`Hands` / `Telekinesis` / `Repulsion`);
  a `mode_select` system on Update reads the 1/2/3 actions and writes the mode —
  the one place mode changes. Run-conditions (2.A.6) gate the per-mode systems.
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
- [ ] The engine debug system is split: engine keeps profiler + asset-reload +
  overlay shell; the game owns its render-mood tweaks via the input-binding API.
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
  decouples from the fixed rate; not needed for first bring-up.
- **`GlobalTransform` / hierarchy** → only when nested / articulated rigs appear
  (turret-on-vehicle, joint-anchored props); not Part 2.
- **scp inter-frame mutation seam** → designed when scp resumes, reusing the 2.A
  `Commands` / `Events` path ([bridge.md]). Keep `Commands` enqueue-able by an
  out-of-process actor.
- **Doc debt**: update each `plans/overview/*` system doc as its phase lands; the
  reactive-substrate work also lets `architecture/render.md`'s stale status section
  and the `architecture/reactivity.md` cursor convention be re-baselined.

[ecs.md]: ../../../plans/overview/ecs.md
[events.md]: ../../../plans/overview/events.md
[physics.md]: ../../../plans/overview/physics.md
[input.md]: ../../../plans/overview/input.md
[graphics.md]: ../../../plans/overview/graphics.md
[bridge.md]: ../../../plans/overview/bridge.md
