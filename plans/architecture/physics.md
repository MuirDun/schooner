# Physics — Embodiment and the Bridge to the ECS

How the world becomes physical. This is the vision, not the implementation;
concrete shapes — components, resources, signatures — live next to the code and in
`overview/physics.md`, because those rot. Physics belongs to **Layer 4** (the
local, here-and-now simulation on the main thread) and runs on its **fixed clock**,
the deterministic sub-frame tick the rest of the simulation is paced against.

Physics is the most direct expression of **pillar 1 — the world is alive**. An
embodied world is one where objects respond, weight settles, a thrown thing flies
and lands, a struck wall gives way. The player is one mass among many in a system
that obeys the same rules whether or not they are watching. Telekinesis — Kinesis's
mechanical voice — is meaningless without a substrate where force, mass, and
contact are real. Physics is that substrate.

The engine does not write its own rigid-body solver. It hosts **Rapier**, a mature
Rust physics engine, and the architectural work is not the simulation — it is the
**bridge**: keeping Rapier's world and the ECS's world coherent without either
becoming the other's puppet. That bridge is the whole of this document.

---

## Two Worlds, One Pose

Physics runs in **two stores that must be kept reconciled**. Rapier owns its own
arena — rigid bodies, colliders, the broad-phase and narrow-phase acceleration
structures, the contact solver — addressed by opaque handles, in its own math
types. The ECS owns entities and components, addressed by entity id, in the
engine's own math. Neither can see into the other. The bridge is the membrane
between them, and it has exactly one job: make the two agree on **where things
are** each fixed step.

The agreement point is the **`Transform`** — the engine's flat, decomposed
position/orientation/scale that already serves as the single pose-of-truth the
renderer reads every frame. This is the decisive piece of luck the design rests
on: because the renderer re-derives every visual pose from `Transform` afresh,
physics gets rendered **for free**. The bridge writes a body's solved pose into its
entity's `Transform`, and the picture updates with no rendering code involved. There
is no rival pose-of-truth to keep in sync, no decompose step, no cached matrix to
invalidate.

Scale stays out of physics entirely. Rapier has no notion of a scaled body; size
lives in the **collider's own dimensions**. So the bridge touches translation and
orientation only, and leaves an entity's visual scale alone.

### The fixed-step membrane

The bridge is **one system, running once per fixed step**, structured as a strict
sequence of one-directional flows so the two stores never tug at each other
mid-step:

1. **Birth and death.** Entities that just gained a physics body get one created in
   Rapier; entities that lost theirs (including whole despawns) get their Rapier
   handles freed. This is driven by the ECS's add/remove detection, not by re-scanning
   every body every step — reconciliation is event-shaped, not poll-shaped.
2. **Intent in.** Poses the game authored or moved this step (a teleport, a
   kinematic platform) are pushed into the corresponding bodies.
3. **Solve.** Rapier advances exactly one fixed step.
4. **Result out.** Each simulated body's freshly solved pose is written back to its
   `Transform`.
5. **Report.** The contacts and sensor overlaps the solve produced are published as
   **events** for game logic to read.

Running this as a single ordered pass — rather than scattering the five concerns
across independent systems — is what makes the data flow legible: everything that
crosses the membrane crosses it in one place, in one direction, at one time.

### Ordering is the load-bearing invariant

The bridge must run **after** every system that this step wanted to influence the
bodies — the force the player applied, the velocity a verb set — and **before**
anything that wants to read the settled result. Get this backwards and forces land
a step late, or the picture shows the pre-solve pose. The discipline is simple to
state and easy to violate: *gameplay writes intent, then the bridge solves, then
gameplay reads outcome.* The schedule, not the buffer, is what guarantees it.

---

## Authoring Versus Simulation

The game never speaks Rapier. It **declares intent** by attaching plain components
that say *this entity is a physical body of such-and-such kind* and *this entity
collides with the following shape and surface properties*. These authoring
components are the stable, engine-intrinsic vocabulary — peers of `Transform` and
the material — and they are all the game ever touches. The Rapier handles, the
solver arena, the acceleration structures: those are the bridge's private business,
held in a single physics resource, never handled by gameplay code.

This split is the same **strict-skeleton / fluid-structure** shape pillar 4 names
elsewhere. The component *contract* is static and typed; *which* entity carries a
body, and what kind, is a runtime question — bodies are born and freed as the world
changes, exactly like any other component.

Three body kinds cover the target game, and the distinction is about *who decides
the pose*:

- **Dynamic** — the solver owns it. Forces, gravity, and contacts move it; the
  bridge reads its pose out. Thrown crates, the rubble of a broken wall.
- **Static** — never moves. The world's immovable shell: the chamber floor and
  walls. Cheap, because the solver never integrates it.
- **Kinematic** — the *game* owns its pose; the solver only lets other bodies
  collide against it. A moving platform, and — crucially — the player.

### The collision proxy is not the visual mesh

A collider carries its **own** shape and dimensions, authored independently of the
rendered geometry. A detailed mesh gets a simple box or capsule **proxy**; a capsule
collider can live on an entity with no mesh at all. Coupling collision shape to
visual scale is a false economy that breaks the first time the two need to differ —
which is immediately, since good collision shapes are deliberately coarser than the
art. Keeping them independent is the standard practice and the one the engine
follows.

---

## Collisions Are Events, Not Callbacks

When Rapier's solve produces a contact or a sensor overlap, that fact enters game
logic as a **discrete event on the Tier-2 backbone** — the same poll-never-subscribe
queue the rest of the engine reacts through. The bridge *publishes*; a game system
*polls*. No system ever registers an `on_collision` callback, for the same reasons
the input and reactivity layers refuse callbacks: a callback is invisible to the
scheduler and the alias checker, and it fires from inside the solver, the wrong
place to mutate the world from.

This is the moment the engine's **first Tier-2 channel** comes alive. Until now the
reactive backbone has been a Layer-4-internal affair; collisions are the first
signal that crosses from one subsystem (physics) into another (game rules) through
a typed queue. The shape it establishes is the shape every later cross-layer signal
reuses.

The decision of *which* mechanism a reaction uses is the standing rule from the
reactivity design: **a fact that persists is state; an instant that occurs is an
event.** A contact *occurs* — it is an instant with a payload — so it is an event.
The health it damages *persists* — so health is state, mutated in one place, and
death is a *derived* reaction to that state changing, never coded inside the
collision handler. The two physics events the target game needs are the **contact**
(two bodies touched) and the **trigger entry** (something entered a sensor volume,
which reports overlaps without a physical response — the substrate for pressure
plates and pickups).

### Why the payload is impulse

A contact event carries the **impulse** the solver applied to resolve it, not a
velocity and not a force. Impulse already integrates mass against change in
velocity, so it is the honest measure of *how hard* a collision was: a heavy mass
arriving fast lands a large impulse, a light one barely registers, and a
destruction threshold or a damage curve can read that single number with no
special-casing for mass. This is the standard "contact-force event" reading, and
choosing it here means the breakable wall and the fall-damage rule are both just
"impulse past a threshold," authored identically.

The bridge must collect that payload from Rapier's **post-solver contact-force
event path**, not from `CollisionEvent::Started`. A started/stopped collision event
is topology: it says two colliders began or ceased touching. It is useful for
trigger entry and contact lifetime, but it is not a reliable impact-strength
signal. The force callback is poorly named for the engine's payload — Rapier uses
it to report thresholded contact forces — but the `ContactPair` available there is
post-solve and still contains the per-point solver impulses. The bridge samples
those impulses, maps the colliders back to entities, and publishes the gameplay
`Contact`.

---

## The Reconciliation Problem

Two stores keyed two different ways must answer two questions cheaply: *given an
entity, which Rapier body?* and *given a Rapier collider in a contact, which
entity?* The bridge answers both without a search, through two cooperating pieces:

- An **entity-to-handle map** in the physics resource — the authority for runtime
  lookup and cleanup. When an entity despawns, its ordinary components may already
  be gone, so the side table is what still remembers which Rapier handles must be
  freed.
- The **entity id stamped into each Rapier object's user-data slot** — Rapier
  carries an opaque integer per body/collider that round-trips untouched, so a
  contact naming two colliders resolves straight back to two entities without an
  O(n) search.

There is deliberately **no ECS-side Rapier handle component** in the baseline
bridge. A handle is a backend arena token, not gameplay identity; exposing it as a
component would create a second source of truth and a stale-token hazard for both
Rust gameplay and future Glyph scripts. If a future engine subsystem has a concrete
need for query-local runtime handles — character controllers, joints, ragdolls, or
debug tooling — the engine can add a private generated component then. Until that
consumer exists, the resource-owned map is the narrower and safer authority.

---

## The Player Is Kinematic

The first-person body is **not** a dynamic rigid body. A dynamic capsule fights the
camera — it tips, it bounces, it accumulates spin — and a first-person player wants
none of that. The player is a **kinematic character controller**: the game proposes
a desired motion each step, and the controller slides it along walls and over steps,
resolving against the static world without ever handing pose authority to the
solver. This is Rapier's own recommended pattern and the one every first-person
engine converges on.

The camera stays its **own** entity with its **own** `Transform`, and the body's
position is copied into it (plus an eye offset) each fixed step. No parent-child
hierarchy is introduced for the rig — a copy is enough, and it matches the
sibling-transform convention the lights already follow. A true transform hierarchy
is a real need only when articulated rigs appear (a turret on a vehicle, a prop on a
joint), and the engine deliberately defers that machinery until such content exists
rather than paying for it speculatively (pillar 2).

---

## Determinism and the Fixed Clock

Physics is paced by the **fixed timestep**, never by the wall clock. The solver
advances by a constant slice of time on every step, which is what makes a tossed
stack of crates settle the same way regardless of frame rate — the bedrock property
a physics simulation needs. The variable-rate frame loop runs the fixed step a
*variable number of times* to keep up: zero on a fast frame, several on a slow one.

That accumulator is **capped**. Under a severe frame spike the engine drops the
excess time rather than running an unbounded burst of catch-up steps that would
freeze the frame further — the classic "spiral of death" avoided by trading a sliver
of determinism under duress for stability. This is the right default, and it is the
same cap the reactivity and input docs lean on.

Determinism here is **single-machine**: the same inputs reproduce the same result on
the same build. Bit-exact reproducibility *across* platforms is a stronger and more
expensive property (it constrains the entire floating-point path) that the target
game does not need, so the engine does not pay for it.

The fixed clock is also why the input discipline matters so much next door: the
verbs that drive physics run on this clock, and they must read durable **state and
latched intent**, never frame-scoped input edges — a fixed-step system that reads an
edge double-fires on a slow frame and misses it on a fast one. Physics is the
consumer the fixed-step discipline in `architecture/input.md` exists to protect.

---

## What Physics Is Not (Yet)

The target game decides scope (pillar 2). Named so the boundary is explicit:

- **No parallel solve.** The engine is single-threaded by deliberate staging;
  physics runs on the main thread with the rest of Layer 4. Threading splits along
  layer boundaries later, not by parallelizing the solver now.
- **No continuous collision detection by default.** Fast, thin objects can tunnel
  through thin walls in a discrete solver; CCD is enabled per-body only where a verb
  (a hard throw) actually produces the speed that needs it, not blanket-on.
- **No joints, no articulation, no ragdolls, no soft bodies, no vehicles.** Each
  waits for a game that forces it. Kinesis's tentacles are keyframed animation, not
  simulated rigs.
- **No transform hierarchy for the physics rig.** A copy-system suffices until
  nested rigs appear.
- **No cross-platform bit-determinism.** Single-machine determinism is the
  contract.

Refusing this generality is not a gap; it is how the bridge stays small enough to
hold in one head and correct enough to trust.

---

## Glyph-Future Alignment

Because collisions and triggers arrive as polled events on a typed queue, and
because reactions to them are ordinary systems that read state and mutate state, a
future Glyph script reacting to a collision is **one more polling reader** over the
exact same queue — not a parallel event path bolted on. The Rust-now reaction (drain
the contact queue, apply damage, let death derive) and the Glyph-later reaction
(`on-event` over the same contacts) sit on one substrate. The verbose-but-portable
Rust shape is the investment that makes the language a translation rather than a
rewrite. That is the same payoff every poll-never-subscribe decision in the engine
is buying.

---

## Cross-references

- `overview/physics.md` — the as-built state and Kinesis Part-2 build plan (the
  detail that tracks the code, where this doc tracks the idea).
- `overview/events.md`, `architecture/reactivity.md` — the Tier-2 event backbone
  and the state-versus-event decision rule the contact/trigger channels follow.
- `architecture/input.md` — the fixed-step discipline the physics-driving verbs must
  obey; physics is the fixed-clock consumer it protects.
- `architecture/ecs.md`, `overview/ecs.md` — the add/remove detection the bridge's
  birth-and-death phase is driven by, and the deferred-command path despawns flow
  through.
- `architecture/overview.md` §Supporting Cast — physics as a Layer-4 citizen that
  bridges to ECS transforms each fixed update.
