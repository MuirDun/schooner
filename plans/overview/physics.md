# Physics — Rapier integration (new in Part 2)

The vision lives in `plans/architecture/physics.md`, rooted in
`plans/architecture/overview.md` (§Supporting Cast: "Physics (Rapier). Lives inside
Layer 4. Bridges to ECS transforms each fixed-update."). This doc is the as-built
state plus the Part-2 build plan.

---

## What exists now

Kinesis Part 2 has the first hosted-physics substrate in place:

- **Rapier resource host.** `PhysicsWorld` owns the Rapier pipeline, body/collider
  sets, broad/narrow phase, joints, CCD solver, integration parameters, gravity,
  and a sync-safe collision/impact sink.
- **Fixed-step bridge.** `App::with_physics` installs `PhysicsWorld` plus the
  `Events<Contact>` / `Events<TriggerEnter>` / `Events<TriggerExit>` queues. The
  exclusive bridge runs between fixed-step intent writers and outcome readers;
  it reconciles authoring lifecycle, syncs authored static/kinematic poses, sets
  `integration_parameters.dt` from `Time::fixed_delta`, steps Rapier, writes awake
  dynamic poses back, and publishes collision events.
- **Attributable bridge diagnostics.** Static puffin scopes separate lifecycle,
  authored-transform sync, discrete commands, character integration/KCC queries,
  Rapier solve, dynamic write-back, and event publication. The public
  `PhysicsDiagnostics` resource resets at the puffin render-frame boundary and
  accumulates matching workload counts across all fixed steps in that frame.
- **Authoring components.** `RigidBody`, `BodyKind`, `Collider`, `ColliderShape`,
  and `PhysicsMaterial` are the public ECS vocabulary. They describe gameplay
  intent; they do not expose Rapier handles.
- **Private identity bridge.** `PhysicsWorld` owns the `EntityId -> PhysicsHandles`
  side table. Rapier body/collider `user_data` stores a tagged `EntityId` so future
  collision draining can map solver output back to ECS entities without scanning.
- **Impact-strength convention.** `CollisionEvent::Started` is treated as topology,
  not impact strength. The sink samples solver impulses from Rapier's post-solver
  contact-force callback; 2.D.3 arms `CONTACT_FORCE_EVENTS`, and 2.D.4 drains those
  samples into `Contact`.

The substrate the bridge relies on was already correct before Rapier landed:

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

1. **Entity ↔ handle convention.** Public ECS authoring components
   (`RigidBody`/`Collider`) plus a private `EntityId -> PhysicsHandles` map in
   `PhysicsWorld`; Rapier user-data carries a tagged `EntityId` for reverse lookup.
   No ECS-side Rapier handle component exists until a concrete internal consumer
   requires one.
2. **Removed-detection** (see [ecs.md](ecs.md)) so despawning a bodied entity
   frees its Rapier handle. *Note:* despawn is already observable via
   `world.is_alive` + generation bump, so a reconcile-style bridge is leak-free
   today; removed-detection is what makes cleanup *event-driven* instead of
   O(handles)/step. Land it with the rest of Tier-1.
3. **Stand up Rapier resources** (`PhysicsWorld`, pipeline, integration params,
   collision-event channel) in `App::resumed` — isolated block for now.
4. **The bridge — one exclusive `fn(&mut World)` FixedUpdate system**, registered
   between fixed-step intent writers and outcome readers: sync changed Transforms → bodies → `step()`
   → write poses back → drain Rapier collision/sensor events into `Events<Contact>`
   / `Events<TriggerEnter>` / `Events<TriggerExit>` ([events.md](events.md)).
5. **Force application** — directional / radial / impulse from gameplay onto
   bodies (the grab/throw/repulse verbs).
6. **Character controller** — replace `fps_move`'s direct translation write with a
   `KinematicCharacterController`. Keep the camera **unparented** (its own
   `Transform`) and copy `body.translation + eye_offset` into it each fixed step —
   Rapier's own KCC pattern, and it matches the sibling-Transform convention the
   lights already use. (No hierarchy needed; see "Decisions".)
7. **Triggers & pressure plates** — Rapier sensor colliders → `Events<TriggerEnter>` /
   `Events<TriggerExit>`;
   pressure plate is a held-state sensor.
8. **Destructibles** — swap a mesh for authored physics fragments on an impact
   threshold read from `Events<Contact>`.
9. **Physics debug-draw** — a `LineList` pipeline fed by Rapier's
   `DebugRenderBackend` (see [graphics.md](graphics.md)) — rendered **into the live
   game world in dev mode**, not an editor viewport.

## Sharp risks / decisions

- **FixedUpdate control sampling.** Durable intent prevents edge double-fire and
  zero-step loss, but the as-built frame order through 2.F.4 captures actions,
  look, movement, and spectator ownership after fixed simulation. Phase 2.FH.5
  moves their once-per-render-frame sampling boundary before fixed simulation;
  FixedUpdate continues reading *state*, never raw frame edges. See
  [input.md](input.md).
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
