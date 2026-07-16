//! Layer 2 input — the named-action map.
//!
//! Architecture: `architecture/input.md` §"Layer 2 — Named Actions" and
//! §"The Fixed-Step Discipline".
//!
//! Gameplay speaks in verbs ("jump", "grab", "mode-telekinesis"), not in
//! keys. Each verb is a [`Symbol`] bound to one or more physical
//! [`Trigger`]s in the [`Bindings`] table; an action is active when *any*
//! of its triggers is (a logical OR). Once per frame, on `Update`,
//! [`resolve_actions`] recomputes [`Actions`] — each verb's
//! down / just-pressed / just-released state — from the raw [`Input`]
//! snapshot, so every gameplay system that frame reads fresh state.
//!
//! Two resources, deliberately split:
//! - [`Bindings`] is **config** — written at setup (and later by
//!   rebinding or a script), rarely changes.
//! - [`Actions`] is **derived state** — overwritten every frame by the
//!   resolve. Gameplay reads it via `Res<Actions>`.
//!
//! ## Edges belong to the action, not its triggers
//!
//! An action's `just_pressed` is computed by comparing the action's own
//! aggregate down-state to last frame's — not by OR-ing its triggers'
//! `just_pressed`. If "fire" is bound to both Left-Click and `E`, tapping
//! `E` while the click is already held must NOT re-announce the verb. The
//! "just happened" signal is a property of the verb's transition.
//!
//! ## Axes and chords are read, not declared
//!
//! The table stays a flat name → OR'd-triggers map. An *axis* is two
//! actions differenced ([`Actions::axis`]); a *chord* (two-handed grip)
//! is an `&&` in the one consumer that needs it. Expressiveness lives in
//! the reader, which keeps the binding model small.
//!
//! ## The fixed-step discipline
//!
//! Edges ([`just_pressed`](Actions::just_pressed) /
//! [`just_released`](Actions::just_released)) and the
//! [`wheel`](Actions::wheel) are **frame-scoped** — resolved once per
//! frame on `Update`, cleared once per frame by `Input::end_frame`.
//! **Never read them from a `FixedUpdate` system.** `FixedUpdate` reads
//! *levels* only: [`pressed`](Actions::pressed), [`axis`](Actions::axis),
//! or a mode/state resource.
//!
//! Why: `FixedUpdate` runs 0..N times per frame against the time
//! accumulator, while an edge lives for exactly one frame. A `FixedUpdate`
//! edge read therefore
//! - **double-fires** on a slow frame that runs several fixed steps — the
//!   edge is still true on each step (one press → N jumps);
//! - **misses** on a fast frame that runs zero fixed steps — the edge is
//!   born and cleared inside a frame the fixed clock never entered;
//! - reads a **stale** edge anyway, since the resolve runs on `Update`,
//!   *after* the frame's fixed steps have already run.
//!
//! Levels are safe because they derive from `Input` state that does not
//! change across a frame's steps — integrating "move forward" on each of
//! three steps is just correct integration.
//!
//! Two handoff shapes carry an edge down to the fixed clock:
//! - **edge → latch (consume once):** an `Update` system turns the edge
//!   into a durable intent (`jump_intent.requested = true`); the first
//!   fixed step that sees it acts and clears it. A press on a zero-step
//!   frame waits in the latch (no miss); a press on an N-step frame is
//!   consumed once (no double-fire). The cost is a deterministic
//!   one-frame latency — the same shape as an event readable the frame
//!   after it is sent.
//! - **continuous → mirror latest:** an `Update` system writes the current
//!   axis / mode / hold-distance into a state resource; each fixed step
//!   reads the latest value. No latch — only edges latch.
//!
//! Mode select is the canonical edge→state case: an `Update` system flips
//! a persistent `ControlMode` on the 1/2/3 edge, and per-mode
//! `FixedUpdate` systems gate on that mode as a level (via `run_if`). This
//! module ships only the rule and the `Update` resolve; the per-verb
//! intent resources are authored game-side when the verbs land (Part 2.F
//! / 2.G).

use std::collections::HashMap;

use crate::ecs::{Res, ResMut};
use crate::input::{Input, KeyCode, MouseButton};
use crate::symbol::Symbol;

/// The direction a wheel turned, for a wheel-bound action. A wheel action
/// is a one-frame pulse — active only on the frame the wheel ticks that
/// way, since the wheel is a per-frame accumulator, not a held level.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WheelDir {
    Up,
    Down,
}

/// A physical source an action binds to. An action is the OR of its
/// triggers — active when any one is. (Continuous wheel *magnitude*, for
/// telekinesis distance, is read via [`Actions::wheel`], not as a
/// trigger; `Wheel` here is the discrete "bind a verb to a scroll notch"
/// case.)
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Trigger {
    Key(KeyCode),
    Mouse(MouseButton),
    Wheel(WheelDir),
}

/// The binding table: each action [`Symbol`] to the triggers that fire
/// it. Config, not state. Engine-intrinsic resource, started empty; the
/// game declares its bindings at setup.
#[derive(Default, Debug)]
pub struct Bindings {
    map: HashMap<Symbol, Vec<Trigger>>,
}

impl Bindings {
    /// Append a trigger to an action. Multiple triggers on one action
    /// OR together — any one fires it. Call repeatedly to bind several.
    pub fn bind(&mut self, action: Symbol, trigger: Trigger) {
        self.map.entry(action).or_default().push(trigger);
    }

    /// Replace an action's triggers with a single one (clear-then-bind).
    /// For rebinding; the per-frame resolve reflects it next frame.
    pub fn rebind(&mut self, action: Symbol, trigger: Trigger) {
        self.map.insert(action, vec![trigger]);
    }

    fn iter(&self) -> impl Iterator<Item = (Symbol, &[Trigger])> {
        self.map
            .iter()
            .map(|(action, triggers)| (*action, triggers.as_slice()))
    }
}

/// Per-action resolved state for the current frame.
#[derive(Default, Clone, Copy, Debug)]
struct ActionState {
    down: bool,
    just_pressed: bool,
    just_released: bool,
}

/// Resolved action state, recomputed once per frame by
/// [`resolve_actions`]. Gameplay reads it via `Res<Actions>`. The read
/// surface mirrors [`Input`]'s but is keyed by action [`Symbol`] and
/// never panics on an unbound symbol (returns `false` / `0.0`).
///
/// Frame-scoped reads (`just_pressed` / `just_released` / [`wheel`](Self::wheel))
/// belong to `Update` only — see the fixed-step discipline. Levels
/// ([`pressed`](Self::pressed) / [`axis`](Self::axis)) are safe to read
/// in `FixedUpdate`.
#[derive(Default, Debug)]
pub struct Actions {
    states: HashMap<Symbol, ActionState>,
    wheel: f32,
}

impl Actions {
    /// Is the action currently held? (Level — `FixedUpdate`-safe.)
    pub fn pressed(&self, action: Symbol) -> bool {
        self.states.get(&action).is_some_and(|s| s.down)
    }

    /// Did the action become active this frame? (Edge — `Update` only.)
    pub fn just_pressed(&self, action: Symbol) -> bool {
        self.states.get(&action).is_some_and(|s| s.just_pressed)
    }

    /// Did the action release this frame? (Edge — `Update` only.)
    pub fn just_released(&self, action: Symbol) -> bool {
        self.states.get(&action).is_some_and(|s| s.just_released)
    }

    /// A digital axis from two actions: `+1.0` if only `pos` is held,
    /// `-1.0` if only `neg`, `0.0` if both or neither. The generalized
    /// form of the camera controller's hard-coded `key_axis`. (Level —
    /// `FixedUpdate`-safe.)
    pub fn axis(&self, neg: Symbol, pos: Symbol) -> f32 {
        self.pressed(pos) as i32 as f32 - self.pressed(neg) as i32 as f32
    }

    /// This frame's wheel movement in line units, mirrored from [`Input`].
    /// Frame-scoped like the edges — read on `Update`, never `FixedUpdate`.
    pub fn wheel(&self) -> f32 {
        self.wheel
    }
}

/// `Update`-stage system: recompute [`Actions`] from [`Input`] +
/// [`Bindings`]. Registered engine-intrinsically, first in `Update`, so
/// every gameplay consumer this frame reads fresh state.
///
/// It must stay on `Update`: edges and the wheel are frame-scoped, and a
/// `FixedUpdate` resolve (0..N times/frame) would double-fire or miss
/// them (see the fixed-step discipline in `architecture/input.md`).
pub fn resolve_actions(input: Res<Input>, bindings: Res<Bindings>, mut actions: ResMut<Actions>) {
    resolve(&input, &bindings, &mut actions);
}

/// The resolve logic, factored out of the system wrapper so unit tests
/// can drive it with a hand-built [`Input`] / [`Bindings`] / [`Actions`]
/// without standing up a schedule.
fn resolve(input: &Input, bindings: &Bindings, actions: &mut Actions) {
    actions.wheel = input.mouse_wheel();
    for (action, triggers) in bindings.iter() {
        let down = triggers.iter().any(|t| trigger_active(t, input));
        let state = actions.states.entry(action).or_default();
        // Edge from the action's OWN transition, not OR of trigger edges.
        state.just_pressed = down && !state.down;
        state.just_released = !down && state.down;
        state.down = down;
    }
}

fn trigger_active(trigger: &Trigger, input: &Input) -> bool {
    match *trigger {
        Trigger::Key(key) => input.is_key_down(key),
        Trigger::Mouse(button) => input.is_mouse_button_down(button),
        Trigger::Wheel(WheelDir::Up) => input.mouse_wheel() > 0.0,
        Trigger::Wheel(WheelDir::Down) => input.mouse_wheel() < 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::symbol::sym;

    /// Build an `Input` with the given keys held and a wheel value, the
    /// way the App would have recorded them this frame.
    fn input_with(keys: &[KeyCode], wheel: f32) -> Input {
        let mut input = Input::new();
        for &key in keys {
            input.record_key(key, true);
        }
        input.record_mouse_wheel(wheel);
        input
    }

    #[test]
    fn action_is_or_of_its_triggers() {
        let fire = sym("action.test.fire_or");
        let mut bindings = Bindings::default();
        bindings.bind(fire, Trigger::Key(KeyCode::KeyW));
        bindings.bind(fire, Trigger::Key(KeyCode::KeyA));
        let mut actions = Actions::default();

        resolve(&input_with(&[KeyCode::KeyW], 0.0), &bindings, &mut actions);
        assert!(actions.pressed(fire), "W alone should fire the action");

        resolve(&input_with(&[KeyCode::KeyA], 0.0), &bindings, &mut actions);
        assert!(actions.pressed(fire), "A alone should fire the action");

        resolve(&input_with(&[], 0.0), &bindings, &mut actions);
        assert!(!actions.pressed(fire), "neither held → not pressed");
    }

    #[test]
    fn edges_fire_once_on_transition() {
        let jump = sym("action.test.jump_edges");
        let mut bindings = Bindings::default();
        bindings.bind(jump, Trigger::Key(KeyCode::Space));
        let mut actions = Actions::default();

        // Press.
        resolve(&input_with(&[KeyCode::Space], 0.0), &bindings, &mut actions);
        assert!(actions.pressed(jump));
        assert!(actions.just_pressed(jump));
        assert!(!actions.just_released(jump));

        // Held — no re-fire.
        resolve(&input_with(&[KeyCode::Space], 0.0), &bindings, &mut actions);
        assert!(actions.pressed(jump));
        assert!(!actions.just_pressed(jump));
        assert!(!actions.just_released(jump));

        // Release.
        resolve(&input_with(&[], 0.0), &bindings, &mut actions);
        assert!(!actions.pressed(jump));
        assert!(!actions.just_pressed(jump));
        assert!(actions.just_released(jump));
    }

    #[test]
    fn second_trigger_press_does_not_refire_just_pressed() {
        // The central correctness property: edges derive from the
        // action's aggregate transition, not from OR-ing trigger edges.
        let fire = sym("action.test.multi_trigger");
        let mut bindings = Bindings::default();
        bindings.bind(fire, Trigger::Key(KeyCode::KeyW));
        bindings.bind(fire, Trigger::Key(KeyCode::KeyA));
        let mut actions = Actions::default();

        // W down → action just_pressed once.
        resolve(&input_with(&[KeyCode::KeyW], 0.0), &bindings, &mut actions);
        assert!(actions.just_pressed(fire));

        // Press A while W still held — the action was already down, so it
        // must NOT re-announce just_pressed even though A's own edge fired.
        resolve(
            &input_with(&[KeyCode::KeyW, KeyCode::KeyA], 0.0),
            &bindings,
            &mut actions,
        );
        assert!(actions.pressed(fire));
        assert!(!actions.just_pressed(fire));
    }

    #[test]
    fn axis_differences_two_actions() {
        let fwd = sym("action.test.fwd");
        let back = sym("action.test.back");
        let mut bindings = Bindings::default();
        bindings.bind(fwd, Trigger::Key(KeyCode::KeyW));
        bindings.bind(back, Trigger::Key(KeyCode::KeyS));
        let mut actions = Actions::default();

        resolve(&input_with(&[KeyCode::KeyW], 0.0), &bindings, &mut actions);
        assert_eq!(actions.axis(back, fwd), 1.0);

        resolve(&input_with(&[KeyCode::KeyS], 0.0), &bindings, &mut actions);
        assert_eq!(actions.axis(back, fwd), -1.0);

        resolve(
            &input_with(&[KeyCode::KeyW, KeyCode::KeyS], 0.0),
            &bindings,
            &mut actions,
        );
        assert_eq!(actions.axis(back, fwd), 0.0, "opposing keys cancel");
    }

    #[test]
    fn wheel_passes_through_and_drives_wheel_triggers() {
        let zoom_in = sym("action.test.zoom_in");
        let mut bindings = Bindings::default();
        bindings.bind(zoom_in, Trigger::Wheel(WheelDir::Up));
        let mut actions = Actions::default();

        // Scroll up this frame: wheel value mirrored, wheel action pulses.
        resolve(&input_with(&[], 2.0), &bindings, &mut actions);
        assert_eq!(actions.wheel(), 2.0);
        assert!(actions.pressed(zoom_in));
        assert!(actions.just_pressed(zoom_in));

        // No scroll next frame: the one-frame pulse releases.
        resolve(&input_with(&[], 0.0), &bindings, &mut actions);
        assert_eq!(actions.wheel(), 0.0);
        assert!(!actions.pressed(zoom_in));
        assert!(actions.just_released(zoom_in));
    }

    #[test]
    fn unknown_symbol_reads_false_not_panic() {
        let actions = Actions::default();
        let never = sym("action.test.never_bound");
        assert!(!actions.pressed(never));
        assert!(!actions.just_pressed(never));
        assert!(!actions.just_released(never));
        assert_eq!(actions.axis(never, never), 0.0);
    }
}
