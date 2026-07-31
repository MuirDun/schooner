# Camera — Architecture Overview

> **Historical Game 0 snapshot.** The projection/controller split and
> coordinate conventions remain useful decision history, but the direct
> `fps_move` free-flight player and deferred-physics timeline are no longer
> current. See [`player-control.md`](player-control.md) for the physical player,
> spectator ownership, and pre-fixed look/control pipeline.

## The idea

The camera subsystem is **two halves wired through the ECS, never to each other**.

- **Half one — projection data.** A `Camera` component carrying the projection params (FOV, near, far) and a zero-sized `ActiveCamera` tag picking which entity the renderer drives. Pose lives on the same entity's `Transform`. The renderer reads all three.
- **Half two — first-person controller.** An `FpsController` component carrying yaw/pitch/speed/sensitivity, plus three small systems (`fps_cursor_toggle`, `fps_look`, `fps_move`) that read `Input` + `Time` and write the entity's `Transform`.

The renderer never sees `FpsController`; the controller never sees `Camera`. They cooperate by sharing the same `Transform` on the same entity. That's the whole protocol.

## Why projection and controller are split

A camera in this engine is not one object. It's a *pose* (Transform), a *lens* (Camera), and an *input policy* (FpsController). Splitting them makes each replaceable:

- A scripted cutscene camera in Game 2 keeps `Camera` + `Transform`, drops `FpsController`, and drives `Transform` from a path-following system.
- A debug top-down camera in Game 3 keeps `FpsController`-style control but swaps `Camera.projection` to `Orthographic`.
- Game 4's possession-by-NPC keeps `Transform` + `Camera`, but the controller becomes an AI brain instead of player input.

A monolithic `Camera` struct holding all three concerns would force every later use case to re-implement what it didn't want.

## Why the projection lives in `camera/`, not `render/`

The renderer is the *consumer* of `Camera`, not the owner — same relationship physics will have to `Transform` in Game 1, audio to `Transform` in Game 2, AI to `Transform` in Game 4. Putting the projection type under `render/` would force every later subsystem that wants to know "what's the camera looking at?" (occlusion culling, audio listener, AI line-of-sight) to import the renderer for the answer.

`camera/` sits next to `transform.rs` at the engine crate root for the same reason `Transform` does: it's a scene-graph primitive shared across subsystems.

## Why polling, not eventful

Same reason as `input.md`. The controller systems declare their access (`Res<Input>`, `Res<Time>`, `Query<(WriteOnly<Transform>, &FpsController, &ActiveCamera)>`), the schedule sees them, and the only way they affect the world is through their declared writes. A camera-control callback registered at startup would re-enter the schedule from the event-loop thread and mutate `Transform` invisibly to the alias checker.

## Frame lifecycle

```
Update stage:
    fps_cursor_toggle  →  reads Input.just_pressed(Esc), flips
                          Input.cursor_grabbed / cursor_visible.
                          App::sync_cursor mirrors to Window
                          after the schedule.

    fps_look           →  if not cursor_grabbed: skip.
                          dx,dy = Input.mouse_delta()
                          yaw   -= dx * sensitivity
                          pitch -= dy * sensitivity (clamped)
                          Transform.rotation = q_yaw * q_pitch

    fps_move           →  if not cursor_grabbed: skip.
                          horizontal forward = Quat(Y, yaw) * -Z
                          horizontal right   = Quat(Y, yaw) * +X
                          accumulate WASD on (forward, right)
                          accumulate Space/Ctrl on world ±Y
                          Transform.translation += v * speed * dt

    ...other systems...

    render_frame       →  reads (Transform, Camera, ActiveCamera),
                          builds view = Transform.matrix().inverse()
                          and proj = Camera.projection.matrix(aspect),
                          uploads camera uniform.
```

Cursor toggle runs before look/move so a single Esc-press in frame N grabs/releases without a one-frame lag.

## Sign and basis conventions

Y-up, right-handed, camera looks down `-Z` at identity rotation. From the right-hand rule:

- **Yaw** = rotation around world `+Y`. Positive yaw rotates `-Z` (forward) toward `-X` — i.e. turns the player to their **left**. So mouse-right (`dx > 0`) decreases yaw.
- **Pitch** = rotation around the camera's local `+X` (right) axis. Positive pitch rotates `-Z` (forward) toward `+Y` — i.e. tilts to look **up**. Winit's `MouseMotion.dy` is positive when the mouse moves down, so mouse-down should look down, which means `pitch -= dy * sensitivity`.
- **Compose order:** `Transform.rotation = Quat::from_axis_angle(Y, yaw) * Quat::from_axis_angle(X, pitch)`. Pitch is the inner rotation (applied first to a vector), so it rotates around the *original* X axis; yaw then sweeps the result around world Y. This is the standard FPS "yaw around world up, pitch around local right" behavior — no roll, no gimbal swap at the poles.
- **Pitch clamp:** `(-π/2 + ε, π/2 - ε)`. At exactly ±π/2 the local right axis aligns with world Y and yaw stops being meaningful around the camera's view direction; clamping with a small epsilon keeps yaw responsive at extreme pitches.

## Movement model

WASD moves on the **horizontal** plane; vertical motion is *only* Space/Ctrl. Look-direction does not couple to translation.

```
forward_h = Quat(Y, yaw) * -Z       // unit, in XZ plane
right_h   = Quat(Y, yaw) * +X       // unit, in XZ plane
move      = forward_h * (W − S)
          + right_h   * (D − A)
move      = move.normalize_or_zero()
          + Y * (Space − Ctrl)
Transform.translation += move * move_speed * delta_secs
```

Why horizontal-only WASD: looking up at a ceiling and pressing W should not lift the player off the floor. This is the FPS-on-foot convention every player has muscle memory for, and it's the right precursor to Game 3's grounded character controller — Game 3 swaps in a velocity integrator with gravity but keeps the same horizontal forward/right basis. Space/Ctrl are the noclip escape hatch for Game 0, where there is no floor collision yet to keep the camera honest.

`normalize_or_zero` on the horizontal component prevents the well-known WASD diagonal-faster bug (`forward + right` has length √2). Vertical input stays additive on top because Space + W should feel like "fly up while moving forward," not be diluted by normalization.

## Cursor-grab policy

The grab state is owned by `Input` (set by systems, mirrored by `App::sync_cursor` once per frame). Two callers write to it:

- **`fps_cursor_toggle`** — `Esc just_pressed` flips both `cursor_grabbed` and `cursor_visible` (inverted: grabbed ⇒ invisible, ungrabbed ⇒ visible).
- **`App::window_event` on `Focused(focused)`** — focus gain grabs + hides; focus loss releases + shows. Without the focus-loss release, alt-tabbing strands the cursor locked over the desktop with no way to ungrab.

`fps_look` and `fps_move` early-return when the cursor is not grabbed. This prevents the camera from spinning whenever the user moves the mouse over the window during dev (e.g. clicking the IDE while the game runs in the background). It also makes the toggle feel correct: Esc → cursor reappears → mouse motion is ignored by the game.

Why the toggle is its own system instead of folded into `fps_look`: Phase H's debug overlay will want to consume Esc for its own purposes (toggling the egui panel), and a separate one-line system makes the binding conflict visible at the schedule level instead of buried inside a controller body.

## What the controller owns (scope)

- `FpsController` component (yaw, pitch, move_speed, mouse_sensitivity).
- `fps_cursor_toggle`, `fps_look`, `fps_move` systems.
- The sign-convention math relating mouse delta to yaw/pitch and yaw to forward/right basis.
- The pitch clamp.

## What the controller deliberately does NOT own

- **Action-map bindings.** Keys are hard-coded to `KeyCode::KeyW/A/S/D/Space/ControlLeft/Escape` here. When Layer 2 of input lands (see `architecture/input.md`), this system rebinds onto named verbs ("move_forward", "look", "toggle_capture") and the hard-codes go away. Until then there's no consumer to justify the indirection.
- **Smoothing / acceleration / inertia.** Mouse delta is consumed raw; WASD is binary on/off. Game-feel polish (mouse smoothing curves, accel ramps, deadzones) lands when there's a target feel to tune toward — Game 1's puzzle interactions or Game 4's traversal.
- **Head-bob, view kick, FOV pulse.** Game 1+ cosmetic.
- **Gamepad / analog stick look.** Layer 2 + gamepad story; Game 1 at earliest.
- **Grounded physics.** No gravity, no ground check, no jump arc — Space/Ctrl is the placeholder vertical axis. Game 3's character controller integrates with Rapier's KinematicCharacterController and replaces `fps_move`'s vertical handling.
- **Multiple camera entities / split-screen / picture-in-picture.** The renderer takes the first `ActiveCamera` it finds. Multi-camera lands when a use case forces it.
- **Camera shake / scripted cinematic control.** Game 2's scripting layer drives `Transform` directly via its own systems; the FPS controller doesn't need a "scripted override" mode because removing the `FpsController` component is the override.

## How later phases sit on this

**Phase H (debug overlay)** registers a system that consumes F1 for the overlay toggle. The cursor-toggle system stays on Esc; if Phase H wants Esc to also dismiss the overlay, the binding policy is decided at the schedule level (which one runs first, do they short-circuit each other) instead of inside either controller.

**Game 1 (physics)** introduces a `KinematicCharacterController` component on the same entity, and `fps_move` rewrites translation through it instead of writing `Transform.translation` directly. `fps_look` is unchanged — looking around is still pure rotation.

**Game 2 (scripting + AI)** moves the binding layer behind named actions. `fps_cursor_toggle` becomes one of the registered actions; rebinding becomes a script concern; the underlying systems still poll an `Input`-shaped surface.

**Game 4 (NPC possession, scripted cameras)** swaps `FpsController` for AI-driven or scripted `Transform` writers on a per-entity basis. Because the renderer reads only `Transform` + `Camera` + `ActiveCamera`, no render code changes.

## Open questions to resolve before later phases

- **Esc binding conflict policy.** When Phase H adds an overlay-toggle that also wants Esc, who wins, and is it ordered in the schedule or via a "consume" flag on `Input` events? Decide during Phase H.
- **Smoothing curve for mouse look.** Raw delta is fine for Game 0 testing; whether Game 1 wants a per-frame low-pass filter or just a higher poll rate is a feel question we don't have evidence to answer yet.
- **Yaw representation under Game 4 NPC possession.** When an AI brain drives the camera, does it write `Transform.rotation` directly (skipping yaw/pitch state), or does it write `FpsController.yaw/pitch` and let `fps_look` derive rotation? Decide when the first AI-driven camera lands.

## Inspiration and prior art

- **Quake / Half-Life / Counter-Strike FPS controllers** — yaw around world up, pitch around local right, horizontal-only WASD, vertical free in noclip. The convention this controller follows verbatim.
- **Bevy `bevy_flycam`** — confirms "two systems (look + move) reading `Input` and writing `Transform`" is the ECS-native shape. Bevy's version conflates cursor toggle into the look system; we split it for the Phase H reason above.
- **Godot `Camera3D` + `CharacterBody3D`** — confirms the projection/controller split. Godot's `Camera3D` is the lens, `CharacterBody3D` (or a script) is the policy; the same node can swap controllers without changing the lens.
- **Unity `CharacterController` + `MouseLook`** — same split. The `MouseLook` script is the historical reference for the sign-convention math used here.
