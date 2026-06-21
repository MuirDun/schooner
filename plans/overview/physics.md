# Physics — Rapier integration (new in Part 2)

No idea doc yet; the vision lives in `plans/architecture/overview.md`
(§Supporting Cast: "Physics (Rapier). Lives inside Layer 4. Bridges to ECS
transforms each fixed-update."). This doc is the Part-2 build plan plus the
readiness the audit established.

---

## What exists now

Nothing — Kinesis Part 1 has no physics. But the *substrate the bridge needs* is
already correct, which is why the audit rated physics **ready-with-caveats**:

- **Deterministic capped fixed-step.** `Time::advance` accumulator +
  `MAX_FIXED_STEPS_PER_FRAME` clamp (`time.rs`); `App::tick` runs `run_fixed`
  0..N before Update/Render (`app.rs`). Set Rapier `integration_parameters.dt =
  Time::fixed_delta`, step once per `run_fixed`.
- **`Transform` is flat decomposed TRS at the engine root** (`transform.rs`) — the
  cleanest possible write-back target: `t.translation = body.translation();
  t.rotation = body.rotation()`, no decompose, no rival pose-of-truth. The
  renderer re-polls `Transform` every frame, so physics poses render for free.
- **`Time::interpolation_alpha` already computed** (`time.rs`) for render-time
  body interpolation when needed.
- **`fps_move` is the sole translation writer** and explicitly the kinematic
  stand-in (`camera/controller.rs`) — swapping in a character controller is local.

## Part-2 build plan (`crates/game/implementation/part2-verbs.md`)

Ordered by dependency:

1. **Entity ↔ handle convention.** A `RigidBodyHandle`/`ColliderHandle` component
   plus a `HashMap<EntityId, Handle>` in a physics resource. Decide this first;
   everything else hangs off it.
2. **Removed-detection** (see [ecs.md](ecs.md)) so despawning a bodied entity
   frees its Rapier handle. *Note:* despawn is already observable via
   `world.is_alive` + generation bump, so a reconcile-style bridge is leak-free
   today; removed-detection is what makes cleanup *event-driven* instead of
   O(handles)/step. Land it with the rest of Tier-1.
3. **Stand up Rapier resources** (`PhysicsWorld`, pipeline, integration params,
   collision-event channel) in `App::resumed` — isolated block for now.
4. **The bridge — one exclusive `fn(&mut World)` FixedUpdate system**, registered
   *after* gameplay Transform writers: sync changed Transforms → bodies → `step()`
   → write poses back → drain Rapier collision/sensor events into `Events<Contact>`
   / `Events<TriggerEnter>` ([events.md](events.md)).
5. **Force application** — directional / radial / impulse from gameplay onto
   bodies (the grab/throw/repulse verbs).
6. **Character controller** — replace `fps_move`'s direct translation write with a
   `KinematicCharacterController`. Keep the camera **unparented** (its own
   `Transform`) and copy `body.translation + eye_offset` into it each fixed step —
   Rapier's own KCC pattern, and it matches the sibling-Transform convention the
   lights already use. (No hierarchy needed; see "Decisions".)
7. **Triggers & pressure plates** — Rapier sensor colliders → `Events<TriggerEnter>`;
   pressure plate is a held-state sensor.
8. **Destructibles** — swap a mesh for authored physics fragments on an impact
   threshold read from `Events<Contact>`.
9. **Physics debug-draw** — a `LineList` pipeline fed by Rapier's
   `DebugRenderBackend` (see [graphics.md](graphics.md)) — rendered **into the live
   game world in dev mode**, not an editor viewport.

## Sharp risks / decisions

- **FixedUpdate input hazard.** Input edges are frame-scoped and cleared once per
  frame, so reading `just_pressed` from a FixedUpdate physics system double-fires
  or misses on 0-step / multi-step frames. Resolve when wiring player physics:
  keep edge reads on Update and have FixedUpdate read *state*, or add a
  FixedUpdate input snapshot. See [input.md](input.md).
- **Hierarchy is not required for the FPS rig** (audit-verified). A copy-system
  suffices. A real parent/child `GlobalTransform` is only needed for nested /
  articulated rigs (turret-on-vehicle, joint-anchored props) — defer until that
  content appears.
- **Determinism.** The capped accumulator is deterministic per fixed step; the
  `MAX_FIXED_STEPS` clamp trades determinism under frame spikes for stability
  (correct choice — documented in `plans/architecture/reactivity.md` §Determinism).
- **Render interpolation.** Consume `interpolation_alpha` only once frame rate
  visibly decouples from the fixed rate; not needed for first bring-up.

Cross-refs: [events.md](events.md), [ecs.md](ecs.md), [graphics.md](graphics.md),
`plans/plan.md` (Physics is **new in Game 1**, reused after).
