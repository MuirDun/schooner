The central rule is now:

> Input does not move the player. Input writes durable intent; the fixed-step physics bridge converts that intent into one collision-resolved movement.

This separates the variable-rate control clock from the fixed-rate simulation clock:

```text
OS input
   ↓
Input resource
   ↓ once per render frame
Named Actions + camera yaw
   ↓
CharacterIntent component
   ↓ once per fixed step
Character controller integration
   ↓
Rapier KCC query + solver
   ↓
Transform + CharacterControllerState
   ↓ PostPhysics
Camera position
```

## 1. The player is an embodied ECS entity

The physical player and the camera are separate entities.

The player body carries:

| Component | Authority |
|---|---|
| `Transform` | ECS-visible physical pose |
| `RigidBody::kinematic_position_based()` | Declares that gameplay/KCC controls the pose |
| Capsule `Collider` | Physical proxy, independent of visuals |
| `CharacterController` | KCC configuration: sliding, slopes, offset, ground snapping |
| `CharacterIntent` | Current control request |
| `CharacterControllerState` | Latest solved grounding and vertical velocity |
| `Player` | Game-specific identity |

This is assembled in [spawn_player](/home/pungy/Develop/schooner/crates/game/src/bin/playground.rs:370).

The camera is a sibling entity rather than a child of the capsule. Its rotation belongs to the look controller; its position is copied from the body after physics. This avoids introducing a transform hierarchy merely for an FPS rig.

That gives us three distinct kinds of state:

- `CharacterController` is configuration.
- `CharacterIntent` is input.
- `CharacterControllerState` is physics output.

Gameplay may author the first two and read the third. It should not independently derive grounding or vertical velocity.

## 2. Raw input becomes named gameplay actions

The engine initially records device state in the `Input` resource:

- keys currently held;
- mouse buttons;
- mouse delta;
- cursor capture state;
- one-frame edges.

The game does not use `KeyW` or `Space` directly for movement. `App` owns a named-action layer and registers `resolve_actions` as the first `Update` system ([app.rs](/home/pungy/Develop/schooner/crates/schooner-engine/src/app.rs:98)).

Bindings map physical controls to symbols such as:

```text
W     → move_forward
S     → move_back
A     → move_left
D     → move_right
Space → jump
```

`Actions` then exposes two different temporal forms:

- Levels such as `pressed` and `axis`, which remain true while held.
- Edges such as `just_pressed`, which exist for one render frame.

The distinction matters because fixed simulation can run zero, one, or several times during one render frame. Reading `just_pressed` directly from every fixed step would either miss the edge or repeat it.

## 3. Control sampling writes `CharacterIntent`

The playground’s control sampler is [capture_player_movement](/home/pungy/Develop/schooner/crates/game/src/bin/playground.rs:249).

It performs four jobs.

First, it establishes control ownership. It only writes physical-player intent when there is an active player camera and the cursor is captured. Otherwise it calls:

```rust
intent.clear();
```

Clearing means:

- horizontal velocity becomes zero;
- any unconsumed jump request is discarded.

The character itself remains active in physics. Disabling controls does not disable gravity.

Second, it reads the two named axes:

```text
forward = move_forward - move_back
right   = move_right   - move_left
```

Third, it converts camera-relative input into world-space velocity:

```rust
forward = yaw_rotation * Vec3::NEG_Z;
right   = yaw_rotation * Vec3::X;

velocity =
    normalize(forward * forward_input + right * right_input)
    * move_speed;
```

Normalizing prevents diagonal movement from being √2 times faster than cardinal movement.

Fourth, it writes the result into the player’s [`CharacterIntent`](/home/pungy/Develop/schooner/crates/schooner-engine/src/physics/component.rs:55):

```rust
intent.set_horizontal_velocity(velocity);

if actions.just_pressed(jump) {
    intent.request_jump(PLAYER_JUMP_SPEED);
}
```

The intent fields are private. Its methods enforce two engine rules:

- Horizontal movement cannot inject vertical velocity.
- Jump speed cannot be negative.

## 4. Why intent is a component rather than a command

Previously, every `MoveCharacter` command caused a complete KCC integration.

That made command count accidentally control simulation count:

```text
0 MoveCharacter commands → 0 gravity integrations
1 MoveCharacter command  → 1 integration
2 MoveCharacter commands → 2 integrations
```

Gravity, landing, and grounding therefore depended on an input producer submitting exactly one command in exactly the right order.

Now `CharacterIntent` is persistent state:

```text
Character entity → one CharacterIntent → one effective value
```

Repeated movement writes replace the stored velocity. They do not enqueue work. If three systems write:

```text
1 m/s → 5 m/s → 3 m/s
```

the bridge sees one final intent of `3 m/s` and still performs only one integration. Writer ordering remains deterministic through schedule ordering, but write multiplicity no longer changes physics multiplicity.

Making intent a component also gives it normal ECS lifecycle:

- despawning the character removes its intent;
- multiple characters each own independent intent;
- no resource-side `EntityId → intent` map can retain stale entries;
- future systems or scripts can access intent through ordinary ECS access declarations.

Most importantly, the bridge does not use `Changed<CharacterIntent>`. That would be incorrect: unchanged input still requires gravity and grounding every fixed step.

## 5. Continuous movement and jump use different persistence

Horizontal movement is held state. Once sampled, the same velocity remains available to every fixed step until the next control sample replaces it.

Jump is a latch:

```text
None
  ↓ request_jump
Some(launch_speed)
  ↓ first successful character integration
None
```

This handles both extremes of the fixed accumulator.

If a frame has no fixed step:

```text
Update samples Space
→ jump latch remains stored
→ next eligible fixed step consumes it
```

If a frame has several fixed steps:

```text
fixed step 1 → consumes and applies jump
fixed step 2 → no jump request
fixed step 3 → no jump request
```

An airborne jump request is consumed but not applied. It is not buffered until landing; buffered jumping would be a separate gameplay feature.

## 6. The fixed-step scheduler owns the causal boundary

`Time` maintains an accumulator and decides how many fixed steps to execute. The default interval is 1/60 second.

For every fixed step, the scheduler runs this sequence ([schedule.rs](/home/pungy/Develop/schooner/crates/schooner-engine/src/ecs/schedule.rs:184)):

```text
FixedUpdate gameplay systems
→ apply deferred ECS commands
→ physics bridge
→ apply deferred ECS commands
→ PostPhysics systems
→ apply deferred ECS commands
```

This establishes the engine-wide invariant:

```text
intent in → physics solve → outcome out
```

Structural commands submitted before physics are materialized before the bridge. PostPhysics systems therefore observe settled results.

## 7. The bridge processes movement exactly once

The physics bridge’s internal sequence is visible in [physics_bridge](/home/pungy/Develop/schooner/crates/schooner-engine/src/physics/bridge.rs:35):

1. Reconcile body/collider lifecycle.
2. Synchronize authored transforms.
3. Apply discrete physics commands.
4. Integrate every character.
5. Step Rapier.
6. Write dynamic poses back.
7. Publish contact and trigger events.

Discrete `PhysicsCommands` now contain only operations such as teleports. Teleports are applied before character integration, so a teleported character performs its one integration from the new position.

Character integration is driven by the controller set, not the intent set or command queue. The bridge collects every entity with `CharacterController` and visits each once in [integrate_characters](/home/pungy/Develop/schooner/crates/schooner-engine/src/physics/bridge.rs:355).

For each controller:

1. Read its `CharacterControllerState`.
2. Read its `CharacterIntent`, or use the zero-input default if absent.
3. Apply a pending jump to the copied state if currently grounded.
4. Run one KCC integration.
5. Consume the jump latch after successful integration.
6. Write the resolved pose and state back.

Thus:

```text
integrations per fixed step
    = number of complete, materialized character controllers
```

It no longer equals the number of input submissions.

A character with no `CharacterIntent` still falls, lands, snaps to ground, and updates grounding. It simply receives `Vec3::ZERO` as horizontal velocity.

## 8. Vertical movement belongs entirely to physics

Inside [PhysicsWorld::move_character](/home/pungy/Develop/schooner/crates/schooner-engine/src/physics/world.rs:426), the bridge starts from the previous solved vertical velocity.

Conceptually:

```text
if grounded and falling:
    vertical_velocity = 0

vertical_velocity += gravity * fixed_delta
```

If a grounded jump was latched, its launch speed has already replaced vertical velocity before this calculation. Gravity therefore begins affecting the jump in the same fixed step.

The requested displacement becomes:

```text
dx = horizontal_velocity.x * dt
dy = vertical_velocity     * dt
dz = horizontal_velocity.z * dt
```

This is a proposal, not the final movement. Rapier’s character controller resolves it against the hosted collision world.

## 9. KCC resolution decides where the capsule may actually go

The KCC query uses the player’s real capsule shape and current Rapier pose.

Its scene filter excludes:

- the character’s own rigid body, preventing self-collision;
- sensors, preventing triggers from becoming invisible walls.

It still includes static and dynamic solid colliders.

Rapier then applies the configured character behavior:

- collision stopping;
- sliding;
- maximum climbable slope;
- steep-slope sliding;
- ground snapping;
- grounded detection.

The result contains an effective translation that may differ from the desired translation:

```text
desired:   move 0.08 m forward
effective: move 0.03 m forward and slide 0.02 m sideways
```

The bridge sets that result as the kinematic body’s next position. If the controller reports grounded while vertical velocity is downward, vertical velocity is reset to zero.

Sensors are absent only from this movement query. The subsequent normal Rapier step still sees the kinematic player overlapping them and produces trigger enter/exit events.

## 10. Rapier then advances the rest of the physical world

After all characters have received their one resolved kinematic target, Rapier advances one fixed timestep.

This distinction is important:

- The KCC determines legal character movement.
- The solver advances dynamic bodies and physical contacts.
- Sensors and collision events are produced through the normal physics pipeline.

Dynamic body poses are then copied back into their ECS `Transform`s. Contacts and trigger events are translated from private Rapier handles back to `EntityId`s and published into typed event queues.

Rapier handles never escape `PhysicsWorld`; gameplay continues to reason in ECS entities.

## 11. The body result drives the camera

After physics, `sync_player_camera` runs in `PostPhysics` ([playground.rs](/home/pungy/Develop/schooner/crates/game/src/bin/playground.rs:280)).

It copies:

```text
camera position = body position + eye offset
```

It deliberately does not overwrite camera rotation. Therefore:

- physical translation comes from the capsule;
- yaw and pitch come from look processing;
- the camera never tips or spins because the player body is kinematic;
- no parent/child transform machinery is needed.

The body’s `Transform` is the agreement point between physics, camera, and rendering.

## 12. What remains incorrect until 2.FH.5

The movement architecture itself is now correct, but the render-frame sampling boundary is still late.

The current `App::tick` order is:

```text
advance time
→ run 0..N fixed steps
→ run Update
→ render
```

`resolve_actions`, `fps_look`, and `capture_player_movement` currently run during that later `Update`.

Consequently, today’s timeline is:

```text
Frame N fixed steps:
    consume intent sampled during Frame N-1

Frame N Update:
    resolve current input
    update look
    write intent for the next eligible fixed step
```

The persistent intent makes this safe—nothing is lost or multiplied—but it still carries approximately one render-frame of control latency. It also means spectator ownership changes cannot clear physical-player intent until after this frame’s fixed work.

2.FH.5 will move these operations to a defined pre-fixed control-sampling boundary:

```text
resolve actions
→ cursor/UI/F8 ownership
→ look/yaw
→ movement + jump intent
→ first eligible fixed step
```

That will change when intent is sampled, not how physics consumes it. The component/bridge architecture built in 2.FH.4 is already the intended long-term handoff.

In one sentence: the variable-rate game layer says what the character wants, the fixed-rate bridge guarantees one attempt to realize it, Rapier decides what motion is physically legal, and PostPhysics exposes the settled result to the camera and gameplay.