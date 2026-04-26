# Input — Architecture Overview

> Status: design fixed for Game 0. Action layer deferred.

## The idea

Input is **two layers**, built bottom-up as games demand them.

- **Layer 1 — Raw input state.** A polling-only resource that knows what physical keys, mouse buttons, cursor position, and motion deltas are currently true. No interpretation, no naming, no rebinding. The substrate everything else reads.
- **Layer 2 — Action map.** Named verbs ("jump", "fire") bound to one-or-many physical triggers, recomputed once per frame from Layer 1. Where the player rebinds, where shik registers actions, where gamepad axes get thresholded into button-shaped events.

Layer 1 lands in Game 0. Layer 2 is deferred until something forces it — rebindable controls, scripting, or gamepad — at which point its state will be derived from Layer 1 without changing Layer 1's shape.

## Why polling, not listeners

The `onActionPressed("jump", fn)` in an ECS it fights the contract.

A system's behavior must be fully described by the params it declares. A callback registered at startup is a back-channel: invisible at the call site, invisible to the alias checker, invisible to the schedule. It also fires from the event-loop thread, where mutating world state means re-entering the schedule.

Polling sidesteps both problems. Edge transitions ("just pressed") fall out for free as the diff between this frame's state and last frame's state — no listener bookkeeping. Bevy, Unity DOTS, and Flecs all converged here for the same reason.

## Why the action layer is deferred

The action map is the right shape for player-facing input. But its **right Rust API depends on decisions we haven't made yet**: how shik registers bindings from script, what the rebinding file format is, how gamepad axes thresh into button events, whether action IDs are interned strings or typed enums.

Game 0's only consumer is a hard-coded FPS controller. Building the action layer now means baking those decisions into the surface that the rest of the engine couples to. Cheaper to ship Layer 1, let real gameplay code drive the requirements, then design Layer 2 once.

## Frame lifecycle

```
winit WindowEvent  →  App translates  →  Input records state
                                         │
                                         ▼
                       Schedule::run  →  systems poll Input
                                         │
                                         ▼
                                  Input::end_frame
                       (rolls "just-pressed" → "still down",
                        clears per-frame deltas)
```

`end_frame` is what makes "just pressed" a one-frame event. Without it a held key would re-trigger every frame.

## What Layer 1 owns (scope)

- Keyboard: down / just-pressed / just-released per key.
- Mouse: cursor position, motion delta, buttons (down / just-pressed / just-released).
- Cursor: grabbed state, visible state.

`record_*` methods are crate-private. Systems cannot synthesize input — only `App::window_event` produces state.

## What Layer 1 deliberately does NOT own

- **Gamepad.** Game 1 at the earliest. Adds a dimension (multiple devices, analog axes) that Layer 1's "what is true right now" model handles fine, but the API isn't worth scaffolding without a consumer.
- **Text input / IME composition.** Lands when UI text fields appear (Game 2's notes/inventory). Different model from key state — buffered Unicode, not key codes.
- **Mouse wheel.** Trivial to add when something asks.
- **Raw-input bypass of OS cursor** (Windows/Linux high-DPI mouse). Plain winit deltas are fine for Game 0; revisit if FPS aim feels coarse.

## How Layer 2 will sit on Layer 1

A single system runs at the top of `Update`, ahead of all gameplay systems. It walks every registered action, ORs its bindings against Layer 1's current state to get `active`, derives edges from `active && !was_active_last_frame` and the inverse.

Layer 1 doesn't change. Removing or rewriting Layer 2 leaves Layer 1 untouched.

## Open questions to resolve before Layer 2 is built

- **Action ID type.** Interned string, `&'static str`, or game-defined enum? Driven hardest by shik integration.
- **Rebinding persistence.** File format and load path. Almost certainly text (RON / JSON) to start.
- **Composite bindings.** Modifier chords (Ctrl+Click) and key sequences (Ctrl+K Ctrl+S). Action games rarely need them; settings menus and dev consoles do.
- **Analog axes vs button thresholds.** Gamepad-driven; decide alongside gamepad.

## Inspiration and prior art

- **Godot `InputMap`** — primary inspiration for the action + multi-trigger shape. Polling-friendly.
- **Bevy `Input<KeyCode>` resource** — almost exactly Layer 1. Confirms polling-from-resource is the ECS-native fit.
- **Unity New Input System** — same shape, heavier abstractions than we need.
- **mura2 `InputService`** (author's prior 2D engine) — polling getter carries forward; listener API does not, for the ECS-contract reasons above.
