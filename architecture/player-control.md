# Player Control — From Human Input to Physical Motion

> **Status:** Current through Kinesis Phase 2.FH.5 (2026-07-31).
>
> This is an as-built architectural explanation: it describes the control path
> a developer works with today and explains why it has this shape. The durable
> engine intent lives in [`plans/architecture/`](../plans/architecture/); source
> code remains the final authority when implementation details change.

This document answers four practical questions:

1. When the player presses a key or moves the mouse, when does the simulation
   see it?
2. Which state belongs to input, gameplay, the character controller, and
   Rapier?
3. Why can a jump neither disappear on a fast frame nor repeat on a slow one?
4. How do the physical body, gameplay camera, and debug spectator exchange
   control without fighting over a transform?

For a working mental model, read Sections 1–8. Sections 9–12 follow the request
through physics and back to the camera. Sections 13–16 cover extension,
debugging, invariants, and deferred work.

The central rule is:

> **Input does not move the player. A once-per-frame control boundary writes
> durable intent; the fixed-step physics bridge attempts that intent exactly
> once per character, and Rapier decides what motion is physically legal.**

---

## 1. Read This First: One Render Frame

Schooner has a render-rate clock and a fixed simulation clock. They meet at a
named `Control` stage:

```text
winit events accumulate
        │
        ▼
Input resource
        │
        │  once per render frame
        ▼
┌─────────────────────────────────────────────────────────┐
│ Control preparation (engine-owned)                      │
│   1. Resolve Input + Bindings → Actions                 │
│   2. Apply control-ownership handoffs, including F8     │
├─────────────────────────────────────────────────────────┤
│ Stage::Control (game-visible)                           │
│   3. Apply cursor/input-capture gating                  │
│   4. Update active-camera yaw and pitch                 │
│   5. Apply control-mode changes                         │
│   6. Write held movement + latched jump intent          │
└─────────────────────────────────────────────────────────┘
        │
        │  zero, one, or several times this render frame
        ▼
┌─────────────────────────────────────────────────────────┐
│ Fixed step                                              │
│   FixedUpdate gameplay                                  │
│   → deferred-command barrier                            │
│   → physics bridge                                      │
│   → deferred-command barrier                            │
│   → PostPhysics gameplay, including camera position     │
│   → deferred-command barrier                            │
└─────────────────────────────────────────────────────────┘
        │
        ▼
Update (later variable-rate, non-control gameplay)
        │
        ▼
Render
        │
        ▼
Input frame signals are cleared; held levels remain
```

`Time` decides the number of fixed steps before this scheduled sequence runs,
but `Control` always executes once before the entire fixed-step burst. The first
eligible fixed step therefore sees this frame's aim, mode, movement, and
one-shot requests. There is no mandatory one-render-frame control delay.

The load-bearing causal order is:

```text
current hardware state
→ one coherent control snapshot
→ durable per-entity intent
→ one character integration per fixed step
→ collision-resolved body result
→ PostPhysics camera position
```

The scheduler implements this order in
[`ecs/schedule.rs`](../crates/schooner-engine/src/ecs/schedule.rs), and
[`App::tick`](../crates/schooner-engine/src/app.rs) invokes the stages in that
order.

---

## 2. Ownership: Who Is Allowed to Write What?

Most player-control bugs are ownership bugs disguised as timing bugs. Schooner
keeps the important responsibilities separate:

| State | Authority | Meaning |
|---|---|---|
| `Input` | `App` records devices; focus/cursor-control policy writes capture state | Honest snapshot of physical devices and requested cursor state |
| `Bindings` | Application setup; later rebinding/script tooling | Configuration mapping physical triggers to action names |
| `Actions` | Engine control preparation | Once-per-frame named interpretation of `Input` |
| `ActiveCamera` | Control-ownership handoff | Which camera currently receives look/control |
| `FpsController` | Control sampling | Yaw, pitch, movement speed, and mouse sensitivity |
| `CharacterIntent` | Gameplay control sampler | Held horizontal motion and a latched jump request |
| `CharacterController` | Gameplay authoring | KCC configuration such as sliding, slopes, offset, and snapping |
| `CharacterControllerState` | Physics bridge | Latest solved grounded state and vertical velocity |
| Player-body `Transform` | Physics bridge after KCC resolution | ECS-visible physical pose |
| Player-camera rotation | `fps_look` during `Control` | Current view and movement basis |
| Player-camera position | `sync_player_camera` during `PostPhysics` | Physical body position plus eye offset |
| Rapier handles and solver state | `PhysicsWorld` | Private backend representation |

This split prevents circular authority:

- Gameplay requests horizontal movement; it does not author grounded state.
- Physics owns gravity and vertical velocity; it does not interpret keyboard
  bindings.
- Look owns camera rotation; physics does not tip or spin the view.
- PostPhysics copies body position to the camera without overwriting look.
- Gameplay never stores or manipulates Rapier handles.

---

## 3. The Physical Player and the Camera Are Sibling Entities

The player body is an embodied ECS entity. In the playground it carries:

| Component | Role |
|---|---|
| `Transform` | ECS-visible body pose |
| `RigidBody::kinematic_position_based()` | Declares game/KCC pose authority |
| Capsule `Collider` | Coarse physical proxy |
| `CharacterController` | Collision-resolution configuration |
| `CharacterIntent` | Current held and one-shot control request |
| `CharacterControllerState` | Latest grounding and vertical-motion result |
| `Player` | Game-specific identity |

The setup lives in
[`spawn_player`](../crates/game/src/bin/playground.rs).

The camera is a separate entity carrying `Transform`, `Camera`,
`FpsController`, `ActiveCamera`, and a game-side `PlayerCamera` marker. It is
not parented to the capsule. A small `PostPhysics` copy is sufficient:

```text
camera translation = body translation + eye-height offset
camera rotation    = yaw/pitch rotation written during Control
```

This avoids introducing a general transform hierarchy merely to build an FPS
rig. More importantly, it makes the two pose authorities visible: physics owns
where the body ended up, while look control owns where the player is facing.

The body is kinematic rather than dynamic because a first-person capsule should
not tip, bounce, or accumulate angular velocity. Gameplay proposes movement,
the KCC resolves it, and the solver still lets the kinematic body participate in
the physical world.

---

## 4. Raw Input and Named Actions Have Different Jobs

The engine's input architecture has two polling layers.

### Raw `Input`: what the hardware did

`App` records window and device events into `Input`. It contains two temporal
forms:

- **Held levels:** keys/buttons currently down, cursor position, cursor capture
  state. These survive render frames until something changes them.
- **Frame-scoped signals:** press/release edges, relative mouse motion, and
  wheel motion. These are accumulated during event handling and cleared once
  after the frame's stages finish.

Systems cannot call the crate-private recording methods, so gameplay cannot
forge physical input accidentally.

The UI boundary is asymmetric on purpose:

- If egui consumes a key/button press, cursor-position event, or wheel event,
  gameplay does not receive it.
- A consumed release still clears gameplay's held state. Otherwise a key
  pressed before UI focus could remain stuck forever.
- Losing window focus releases every held key and mouse button because the OS
  may not deliver releases that occur while another application is focused.

Raw relative mouse motion arrives as a device event rather than an egui-owned
window event, so it may still accumulate while the cursor is free. Cursor
capture is therefore the authoritative look/movement gate. When capture
changes, accumulated mouse motion is discarded so motion gathered for the old
cursor owner cannot snap the new camera.

### `Actions`: what the hardware means

`Bindings` maps physical triggers to interned action symbols:

```text
W     → move_forward
S     → move_back
A     → move_left
D     → move_right
Space → jump
F8    → debug.camera.spectator
```

The engine resolves the complete binding table exactly once per render frame,
before public `Control` systems run. Cost is proportional to bindings, not to
the number of player entities or action consumers.

`Actions` exposes:

- **Levels** such as `pressed` and `axis`.
- **Edges** such as `just_pressed` and `just_released`.
- The frame's resolved wheel value.

Action edges belong to the action's aggregate transition, not to individual
triggers. If one action is bound to two keys, pressing the second while the
first is held does not announce a second action press.

The full rationale is in
[`plans/architecture/input.md`](../plans/architecture/input.md).

---

## 5. Why `Control` Exists Before Fixed Simulation

A render frame may owe zero, one, or several 60 Hz simulation steps. That makes
frame-scoped input unsafe inside `FixedUpdate`:

- On a **zero-step frame**, an edge could be created and cleared without any
  fixed system seeing it.
- On a **multi-step frame**, every fixed step could see the same edge and repeat
  the action.
- If sampling happened after the fixed burst, current input would pay a
  mandatory frame of latency.

`Stage::Control` bridges the clocks. It executes once per render frame and turns
frame-scoped signals into durable state that fixed work can safely consume.

There are two lanes inside the boundary.

### Engine-owned preparation

The private preparation lane runs before every public control sampler:

1. `resolve_actions` derives `Actions`.
2. Engine-owned control handoffs, currently the debug spectator's F8 toggle,
   establish who owns the active camera.
3. Deferred structural commands are applied before gameplay samples control.

This lane exists so F8 correctness does not depend on whether a debug plugin was
added before or after game systems in the application builder.

### Public `Stage::Control`

The playground registers, in order:

1. `fps_cursor_toggle`
2. `fps_look`
3. `mode_select`
4. `capture_player_movement`

Registration order matters here. Look updates yaw before movement converts its
axes into world-space velocity, so aim, movement direction, and future
camera-targeted verbs share one orientation.

Look follows the engine's Y-up, right-handed convention: identity faces world
`-Z`; mouse-right decreases yaw, mouse-down decreases pitch, and pitch is
clamped just inside `±π/2`. The camera rotation is `yaw × pitch`, with no roll.

The current `mode_select` changes the playground's editor/smoke mode. Phase 2.G
will put the gameplay verb mode on the same boundary; no fixed-step consumer
should poll its selection edge independently.

The later `Update` stage remains intentionally available. Scene transitions,
editor smoke actions, asset/debug controls, and spectator free-flight can remain
variable-rate work without delaying physical-player control.

---

## 6. Control Ownership and the Spectator

`ActiveCamera` identifies the current control/view owner. The dev-tools
spectator is a separate camera entity rather than a mode flag on the physical
player.

### Activating the spectator

During control preparation, F8:

1. Copies the active gameplay camera's transform, lens, and FPS-controller
   state to the spectator.
2. Moves `ActiveCamera` from the gameplay camera to the spectator.
3. Discards this frame's mouse delta, action edges, and wheel input.
4. Leaves held key/button levels intact for the new owner.

Public control sampling then sees no active `PlayerCamera`, so
`capture_player_movement` clears the physical player's `CharacterIntent` before
physics runs. The body still receives a zero-input integration: gravity,
grounding, snapping, and landing continue.

`fps_look` operates on the now-active spectator during `Control`. The spectator's
collision-free translation remains a later `Update` system because it is
variable-rate debug movement, not physical simulation.

### Returning to the player

F8 restores `ActiveCamera` to the previous gameplay camera and again discards
frame-scoped old-owner signals:

- accumulated spectator mouse motion cannot rotate the player;
- a jump or verb edge cannot cross the handoff;
- the physical `CharacterIntent` has already been cleared while the spectator
  was active.

Held levels are deliberately not erased. If the user is still physically
holding `W` after returning, that is current input and produces a fresh movement
sample. What is prohibited is replay of stored movement from before spectator
activation.

The handoff implementation lives in
[`camera/debug.rs`](../crates/schooner-engine/src/camera/debug.rs).

---

## 7. `CharacterIntent` Is the Clock-Crossing State

The game-side sampler
[`capture_player_movement`](../crates/game/src/bin/playground.rs) writes one
`CharacterIntent` on the physical player.

It first checks ownership and gating:

```text
active PlayerCamera exists?
and cursor is captured?
    yes → sample controls
    no  → clear CharacterIntent
```

It then derives two digital axes:

```text
forward_input = move_forward − move_back
right_input   = move_right   − move_left
```

Yaw turns those camera-relative axes into a world-space horizontal velocity:

```text
forward = yaw_rotation × world -Z
right   = yaw_rotation × world +X

horizontal_velocity =
    normalize(forward × forward_input + right × right_input)
    × move_speed
```

Normalizing prevents diagonal input from being `√2` times faster. Pitch is
ignored for locomotion, so looking upward and pressing forward does not lift the
player. The intent setter also discards any authored Y value: gravity and jump
own the vertical axis.

The component contains two forms of persistence.

### Held horizontal movement

The latest horizontal velocity remains until the next control sample replaces
or clears it. Every fixed step in a burst reads the same held value.

Repeated writes replace one component value; they do not queue integrations.
If several deterministic writers produce:

```text
1 m/s → 5 m/s → 3 m/s
```

the bridge sees one final value of `3 m/s`. Registration order decides the
winner, but write count cannot multiply physics work.

### Latched jump

`request_jump(speed)` stores one optional launch speed:

```text
no request
    │ request_jump
    ▼
pending launch speed
    │ first successful character integration
    ▼
no request
```

A later request before integration replaces the launch speed rather than
creating a second jump. The bridge consumes the request after an actual KCC
integration. If the character is grounded, the speed becomes vertical velocity;
if airborne, the request is consumed without applying a jump. Jump buffering
until landing would be a separate gameplay feature.

The component API is defined in
[`physics/component.rs`](../crates/schooner-engine/src/physics/component.rs).

---

## 8. The Fixed-Step Cases

The control/intent handoff is easiest to verify by looking at edge cases:

| Render-frame situation | Held movement | Jump request |
|---|---|---|
| Zero fixed steps | Latest value remains stored | Latch waits for a future step |
| One fixed step | Applied once | Consumed by that integration |
| Several fixed steps | Applied on every step | Consumed on the first integration only |
| No movement input | Zero horizontal velocity | Gravity and grounding still advance |
| Controls disabled | Intent is cleared to zero | Old latch is discarded |
| Spectator active | Physical body integrates zero input | Spectator inputs cannot trigger it |
| Repeated intent writes | Last value wins | Still at most one pending request |

This is why neither raw edges nor `Changed<CharacterIntent>` drive character
integration. An unchanged intent still needs gravity, collision resolution,
ground snapping, and grounded-state updates on every fixed step.

---

## 9. The Physics Bridge Integrates Controllers, Not Commands

The old movement path treated `MoveCharacter` as a discrete physics command.
Each command performed a complete KCC integration, so command count accidentally
became simulation count:

```text
0 movement commands → no gravity integration
1 movement command  → one integration
2 movement commands → two gravity integrations
```

That model is gone. `PhysicsCommands` now carries discontinuous operations such
as teleports. Continuous character movement is persistent component state.

For each fixed step, the bridge executes:

1. Reconcile ECS physics authoring with Rapier bodies and colliders.
2. Synchronize changed ECS-authored transforms.
3. Apply discrete physics commands, including teleports.
4. Visit the `CharacterController` set and perform at most one KCC integration
   for each complete, materialized controller.
5. Advance Rapier's global solver once.
6. Write dynamic-body poses back to ECS transforms.
7. Publish contacts and trigger enter/exit events.

The durable physics document groups this into five conceptual flows—lifecycle,
intent in, solve, result out, and report. The seven concrete phases above are
the same membrane with command processing and character integration exposed
separately for ordering and profiling.

Lifecycle reconciliation also runs at render-frame top so a removal cannot be
lost merely because a high-refresh frame had no fixed step.

For each character, the integration phase:

1. Reads `CharacterControllerState`.
2. Reads `CharacterIntent`, or substitutes the zero-input default.
3. Applies a pending grounded jump to the copied vertical state.
4. Performs one KCC movement query.
5. Consumes the jump after successful integration.
6. Writes the effective body translation, grounded result, and vertical
   velocity back to the ECS.

Therefore:

```text
character integrations per fixed step
    = controllers with state and a materialized body
      whose KCC query succeeds
```

It does not equal the number of input submissions, movement writers, or action
consumers.

The implementation is in
[`physics/bridge.rs`](../crates/schooner-engine/src/physics/bridge.rs).

---

## 10. Vertical Motion Is a Physics Outcome

Gameplay authors horizontal velocity and an optional jump launch speed. The
physics world owns ongoing vertical velocity:

```text
if grounded and moving downward:
    vertical_velocity = 0

if a grounded jump is pending:
    vertical_velocity = jump_speed

vertical_velocity += gravity × fixed_delta
```

The KCC proposal for this step is:

```text
desired translation =
    horizontal_velocity × fixed_delta
    + world_up × vertical_velocity × fixed_delta
```

This proposal is not the final pose. Rapier resolves it against the hosted
collision world. Gameplay reads `CharacterControllerState.grounded` and
`vertical_velocity` afterward; it does not independently infer either from
contacts.

---

## 11. KCC Movement and the Normal Rapier Step Do Different Work

The character movement query uses the player's actual capsule at its current
Rapier pose. Its query filter excludes:

- the character's own rigid body;
- sensors.

It still includes static and dynamic solid colliders. Rapier's KCC then applies
the authored controller behavior:

- obstruction and sliding;
- maximum climbable slope;
- steep-slope sliding;
- controller offset;
- ground snapping;
- grounded detection.

The effective translation may be smaller or point in a different direction than
the requested translation:

```text
requested: 0.08 m forward
effective: 0.03 m forward + 0.02 m sideways
```

Sensors are excluded only from this KCC movement query. They remain in Rapier's
normal collision/overlap pipeline. After character targets are established, the
global Rapier step sees the kinematic capsule overlap sensors and can publish
the correct trigger enter/exit events. A trigger is therefore traversable
without becoming invisible to gameplay.

The hosted query and vertical integration live in
[`physics/world.rs`](../crates/schooner-engine/src/physics/world.rs).

---

## 12. The Solved Body Drives the Camera

`sync_player_camera` runs during `PostPhysics`, after the KCC and Rapier work for
that fixed step:

```text
camera translation = body translation + eye offset
```

It deliberately leaves rotation untouched. The resulting frame combines:

- current-frame yaw and pitch sampled before fixed simulation;
- the latest collision-resolved body position;
- the camera's authored eye-height offset.

The player can therefore turn and move using one coherent yaw while the camera
still follows the physical capsule rather than a speculative desired pose.

On a render frame with no fixed step, the camera rotation can still update
because look belongs to `Control`; camera translation remains at the most recent
physical result. This is expected. Presentation interpolation is a separate,
explicitly deferred concern.

---

## 13. Why This Shape Fits the ECS

`CharacterIntent` is an ordinary per-entity component rather than a global
`EntityId → intent` map or a movement queue. That gives it normal ECS lifecycle:

- despawning a character removes its intent automatically;
- multiple characters own independent intent;
- no side map can retain stale entity identifiers;
- future Rust, Glyph, AI, replay, or remote-control producers can target the
  same component contract;
- access remains visible to the scheduler and alias checker.

The engine bridge scales by controller count. The current Kinesis sampler owns
one local physical player, but the physics contract itself is not a singleton.

If several systems are allowed to write one character's intent, their schedule
order is the arbitration policy: the final write wins. A future possession or
network layer may introduce a more explicit ownership component when a real
second producer requires it; the physics bridge does not need to change.

---

## 14. How to Debug the Pipeline

When movement feels wrong, locate the broken ownership boundary rather than
starting inside Rapier.

### Input/control questions

- Is the cursor captured?
- Did the UI consume the press?
- Which entity has `ActiveCamera`?
- Did `Actions` resolve the expected named action?
- Did `fps_look` update yaw before movement capture?
- Does the player have the expected `CharacterIntent` after `Control`?

### Physics questions

- Is the body fully authored with `Transform`, `RigidBody`, and `Collider`?
- Does it also have `CharacterControllerState`?
- Was the body materialized in `PhysicsWorld`?
- Did the character workload record one integration?
- Is the obstacle a solid collider or a sensor?
- What grounded and vertical-velocity result was written back?

Puffin separates the relevant costs into scopes including:

```text
control_stage
physics.lifecycle_reconciliation
physics.authored_transform_sync
physics.command_processing
physics.character_integration
physics.character_kcc_query
physics.rapier_solve
physics.dynamic_pose_writeback
physics.event_publication
```

`PhysicsDiagnostics` complements time with workload counts such as controller
candidates, actual integrations, KCC queries, jump requests, and jumps applied.
This distinguishes an expensive operation from an unexpectedly large workload.

---

## 15. Invariants to Preserve

Future changes to control, verbs, cameras, scripting, or physics must preserve
these rules:

1. Named actions resolve once per render frame before public control sampling.
2. Control ownership is established before look and physical intent capture.
3. Aim, movement direction, mode, and targeted one-shots use one frame's
   coherent snapshot.
4. Frame-scoped edges are latched once; fixed systems do not independently poll
   them.
5. Every complete active character integrates at most once per fixed step.
6. Missing or cleared movement intent means zero horizontal input, not disabled
   physics.
7. Gravity, grounded state, snapping, and vertical velocity remain physics
   outcomes.
8. Sensors are transparent only to the KCC query and remain visible to overlap
   reporting.
9. Player-camera rotation and physical translation have distinct writers.
10. `Update` remains available for later non-control variable-rate gameplay.
11. Rapier handles remain private to `PhysicsWorld`; ECS entities remain
    gameplay identity.

---

## 16. Deliberately Deferred

The current architecture leaves several decisions open until measurement or a
real producer justifies them:

- **Production walking-push behavior.** Ordinary obstruction versus Rapier's
  approximate character impulse response remains an explicit A/B decision.
- **Render interpolation.** High-refresh judder must first be demonstrated as a
  material defect, and any design must account for approximately one fixed
  step of presentation delay.
- **Gamepad and analog response.** The input substrate is keyboard/mouse today.
- **Binding persistence and input contexts.** Bindings are runtime data, but
  profiles, layered contexts, chords, and sequences wait for consumers.
- **General possession arbitration.** The spectator handoff proves ownership,
  but multi-agent control policy waits for AI/network/script producers.
- **Jump buffering, coyote time, acceleration, and other feel policies.** These
  belong above the durable intent/physics boundary and do not require changing
  it.

---

## Related Architecture

- [`plans/architecture/input.md`](../plans/architecture/input.md) — raw input,
  named actions, and the fixed-step discipline.
- [`plans/architecture/physics.md`](../plans/architecture/physics.md) — physical
  embodiment, Rapier ownership, and the ECS bridge.
- [`plans/architecture/ecs.md`](../plans/architecture/ecs.md) — why intent and
  controller state are per-entity components.
- [`plans/architecture/reactivity.md`](../plans/architecture/reactivity.md) —
  state versus events and scheduler-visible polling.
- [`crates/game/implementation/part2-verbs.md`](../crates/game/implementation/part2-verbs.md)
  — the staged Kinesis implementation and validation history.

In one sentence: **the render-rate layer decides what the controlled character
wants, the fixed-rate bridge guarantees one physical attempt per controller,
Rapier decides what motion is legal, and `PostPhysics` exposes the settled body
to the camera and gameplay.**
