//! Layer 1 input — raw polling state.
//!
//! Architecture: see `architecture/input.md` for the why (two-layer
//! model, polling-not-listeners, action-map deferred).
//!
//! [`Input`] is a resource: systems read it via `Res<Input>` /
//! `ResMut<Input>`. State is recorded by the App from winit events
//! through the crate-private `record_*` setters; a per-frame
//! [`Input::end_frame`] call clears one-shot edges and per-frame
//! deltas. The resource never calls back into winit — it is a
//! one-way sink.

use std::collections::HashSet;

use glam::Vec2;

pub use winit::event::MouseButton;
pub use winit::keyboard::KeyCode;

/// Snapshot of the physical input devices, polled by systems.
///
/// `down` survives across frames; `just_pressed` and
/// `just_released` are one-frame edges, cleared by
/// [`Input::end_frame`]. Mouse `delta` accumulates within a frame
/// and resets at end-of-frame; `position` is the last seen cursor
/// location and persists.
#[derive(Debug)]
pub struct Input {
    keyboard: KeyboardState,
    mouse: MouseState,
    cursor: CursorState,
}

#[derive(Debug, Default)]
struct KeyboardState {
    down: HashSet<KeyCode>,
    just_pressed: HashSet<KeyCode>,
    just_released: HashSet<KeyCode>,
}

#[derive(Debug, Default)]
struct MouseState {
    position: Vec2,
    delta: Vec2,
    down: HashSet<MouseButton>,
    just_pressed: HashSet<MouseButton>,
    just_released: HashSet<MouseButton>,
}

#[derive(Debug)]
struct CursorState {
    grabbed: bool,
    visible: bool,
}

impl Default for CursorState {
    fn default() -> Self {
        // Windows show a cursor at startup; the app explicitly grabs
        // and hides on focus / Esc later. Defaulting `visible: false`
        // would briefly hide the cursor between window creation and
        // the first system request.
        Self {
            grabbed: false,
            visible: true,
        }
    }
}

impl Default for Input {
    fn default() -> Self {
        Self::new()
    }
}

impl Input {
    pub fn new() -> Self {
        Self {
            keyboard: KeyboardState::default(),
            mouse: MouseState::default(),
            cursor: CursorState::default(),
        }
    }

    // -- keyboard read ---------------------------------------------

    pub fn is_key_down(&self, code: KeyCode) -> bool {
        self.keyboard.down.contains(&code)
    }

    pub fn just_pressed(&self, code: KeyCode) -> bool {
        self.keyboard.just_pressed.contains(&code)
    }

    pub fn just_released(&self, code: KeyCode) -> bool {
        self.keyboard.just_released.contains(&code)
    }

    // -- mouse buttons read ----------------------------------------

    pub fn is_mouse_button_down(&self, button: MouseButton) -> bool {
        self.mouse.down.contains(&button)
    }

    pub fn mouse_button_just_pressed(&self, button: MouseButton) -> bool {
        self.mouse.just_pressed.contains(&button)
    }

    pub fn mouse_button_just_released(&self, button: MouseButton) -> bool {
        self.mouse.just_released.contains(&button)
    }

    // -- iterators over one-shot edges (diagnostics, action layer) --

    pub fn iter_just_pressed_keys(&self) -> impl Iterator<Item = KeyCode> + '_ {
        self.keyboard.just_pressed.iter().copied()
    }

    pub fn iter_just_released_keys(&self) -> impl Iterator<Item = KeyCode> + '_ {
        self.keyboard.just_released.iter().copied()
    }

    pub fn iter_just_pressed_mouse_buttons(&self) -> impl Iterator<Item = MouseButton> + '_ {
        self.mouse.just_pressed.iter().copied()
    }

    pub fn iter_just_released_mouse_buttons(&self) -> impl Iterator<Item = MouseButton> + '_ {
        self.mouse.just_released.iter().copied()
    }

    // -- mouse motion read -----------------------------------------

    /// Last seen cursor position in physical pixels relative to the
    /// window. Survives across frames until the cursor moves again.
    pub fn mouse_position(&self) -> Vec2 {
        self.mouse.position
    }

    /// Accumulated motion since the last [`end_frame`](Self::end_frame).
    /// Reset to zero on rollover. FPS look should read this in
    /// `Update`, before the frame ends.
    pub fn mouse_delta(&self) -> Vec2 {
        self.mouse.delta
    }

    // -- cursor read -----------------------------------------------

    pub fn cursor_grabbed(&self) -> bool {
        self.cursor.grabbed
    }

    pub fn cursor_visible(&self) -> bool {
        self.cursor.visible
    }

    // -- cursor request (callable from systems) --------------------

    /// Set the desired cursor-grabbed state. The App syncs the
    /// actual `Window` to match this flag once per frame.
    pub fn set_cursor_grabbed(&mut self, grabbed: bool) {
        self.cursor.grabbed = grabbed;
    }

    /// Set the desired cursor-visible state. The App syncs the
    /// actual `Window` to match this flag once per frame.
    pub fn set_cursor_visible(&mut self, visible: bool) {
        self.cursor.visible = visible;
    }

    // -- recording (App-side, crate-private) -----------------------

    /// Record a key transition. Idempotent: pressing an
    /// already-down key does not re-stamp `just_pressed`, and
    /// releasing a key that wasn't down is a no-op. Keeps OS-level
    /// auto-repeat from spuriously firing one-shot edges.
    pub(crate) fn record_key(&mut self, code: KeyCode, pressed: bool) {
        if pressed {
            if self.keyboard.down.insert(code) {
                self.keyboard.just_pressed.insert(code);
            }
        } else if self.keyboard.down.remove(&code) {
            self.keyboard.just_released.insert(code);
        }
    }

    pub(crate) fn record_mouse_button(&mut self, button: MouseButton, pressed: bool) {
        if pressed {
            if self.mouse.down.insert(button) {
                self.mouse.just_pressed.insert(button);
            }
        } else if self.mouse.down.remove(&button) {
            self.mouse.just_released.insert(button);
        }
    }

    pub(crate) fn record_mouse_position(&mut self, x: f32, y: f32) {
        self.mouse.position = Vec2::new(x, y);
    }

    /// Add to the per-frame motion delta. Multiple motion events in
    /// one frame accumulate.
    pub(crate) fn record_mouse_motion(&mut self, dx: f32, dy: f32) {
        self.mouse.delta += Vec2::new(dx, dy);
    }

    /// End-of-frame rollover.
    ///
    /// Clears one-shot edges (`just_pressed`, `just_released` for
    /// both keys and mouse buttons) and resets `mouse_delta` to
    /// zero. Persistent state — what's currently `down`, last
    /// cursor `position`, cursor grab/visibility — is left alone.
    pub(crate) fn end_frame(&mut self) {
        self.keyboard.just_pressed.clear();
        self.keyboard.just_released.clear();
        self.mouse.just_pressed.clear();
        self.mouse.just_released.clear();
        self.mouse.delta = Vec2::ZERO;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- keyboard ---------------------------------------------------

    #[test]
    fn fresh_input_has_no_keys_down() {
        let input = Input::new();
        assert!(!input.is_key_down(KeyCode::Space));
        assert!(!input.just_pressed(KeyCode::Space));
        assert!(!input.just_released(KeyCode::Space));
    }

    #[test]
    fn press_marks_down_and_just_pressed() {
        let mut input = Input::new();
        input.record_key(KeyCode::KeyW, true);
        assert!(input.is_key_down(KeyCode::KeyW));
        assert!(input.just_pressed(KeyCode::KeyW));
        assert!(!input.just_released(KeyCode::KeyW));
    }

    #[test]
    fn release_marks_just_released_and_clears_down() {
        let mut input = Input::new();
        input.record_key(KeyCode::KeyW, true);
        input.end_frame();
        input.record_key(KeyCode::KeyW, false);
        assert!(!input.is_key_down(KeyCode::KeyW));
        assert!(input.just_released(KeyCode::KeyW));
        assert!(!input.just_pressed(KeyCode::KeyW));
    }

    #[test]
    fn redundant_press_does_not_refire_just_pressed() {
        // OS auto-repeat on a held key fires repeated KeyDown
        // events. The just_pressed edge must only fire on the real
        // transition, not on auto-repeat.
        let mut input = Input::new();
        input.record_key(KeyCode::KeyA, true);
        input.end_frame();
        input.record_key(KeyCode::KeyA, true);
        assert!(input.is_key_down(KeyCode::KeyA));
        assert!(!input.just_pressed(KeyCode::KeyA));
    }

    #[test]
    fn release_of_key_that_was_not_down_is_silent() {
        let mut input = Input::new();
        input.record_key(KeyCode::KeyA, false);
        assert!(!input.just_released(KeyCode::KeyA));
    }

    #[test]
    fn end_frame_clears_just_pressed_and_just_released_but_keeps_down() {
        let mut input = Input::new();
        input.record_key(KeyCode::KeyW, true);
        input.record_key(KeyCode::KeyA, true);
        input.record_key(KeyCode::KeyA, false);
        assert!(input.just_pressed(KeyCode::KeyW));
        assert!(input.just_released(KeyCode::KeyA));
        input.end_frame();
        assert!(input.is_key_down(KeyCode::KeyW));
        assert!(!input.just_pressed(KeyCode::KeyW));
        assert!(!input.just_released(KeyCode::KeyA));
    }

    #[test]
    fn press_release_in_one_frame_yields_both_edges() {
        // A very fast tap that lands inside one frame should
        // surface both edges on that frame.
        let mut input = Input::new();
        input.record_key(KeyCode::Space, true);
        input.record_key(KeyCode::Space, false);
        assert!(input.just_pressed(KeyCode::Space));
        assert!(input.just_released(KeyCode::Space));
        assert!(!input.is_key_down(KeyCode::Space));
    }

    // -- mouse buttons ----------------------------------------------

    #[test]
    fn mouse_press_marks_down_and_just_pressed() {
        let mut input = Input::new();
        input.record_mouse_button(MouseButton::Left, true);
        assert!(input.is_mouse_button_down(MouseButton::Left));
        assert!(input.mouse_button_just_pressed(MouseButton::Left));
    }

    #[test]
    fn mouse_redundant_press_does_not_refire_just_pressed() {
        let mut input = Input::new();
        input.record_mouse_button(MouseButton::Right, true);
        input.end_frame();
        input.record_mouse_button(MouseButton::Right, true);
        assert!(input.is_mouse_button_down(MouseButton::Right));
        assert!(!input.mouse_button_just_pressed(MouseButton::Right));
    }

    #[test]
    fn mouse_release_marks_just_released_and_clears_down() {
        let mut input = Input::new();
        input.record_mouse_button(MouseButton::Left, true);
        input.end_frame();
        input.record_mouse_button(MouseButton::Left, false);
        assert!(!input.is_mouse_button_down(MouseButton::Left));
        assert!(input.mouse_button_just_released(MouseButton::Left));
    }

    // -- mouse motion -----------------------------------------------

    #[test]
    fn mouse_position_records_last_value() {
        let mut input = Input::new();
        assert_eq!(input.mouse_position(), Vec2::ZERO);
        input.record_mouse_position(100.0, 50.0);
        assert_eq!(input.mouse_position(), Vec2::new(100.0, 50.0));
        input.record_mouse_position(101.0, 51.0);
        assert_eq!(input.mouse_position(), Vec2::new(101.0, 51.0));
    }

    #[test]
    fn mouse_delta_accumulates_within_frame() {
        let mut input = Input::new();
        input.record_mouse_motion(1.0, 0.0);
        input.record_mouse_motion(0.5, 2.0);
        input.record_mouse_motion(-0.5, 1.0);
        assert_eq!(input.mouse_delta(), Vec2::new(1.0, 3.0));
    }

    #[test]
    fn end_frame_clears_mouse_delta_but_keeps_position() {
        let mut input = Input::new();
        input.record_mouse_position(100.0, 50.0);
        input.record_mouse_motion(5.0, -3.0);
        input.end_frame();
        assert_eq!(input.mouse_delta(), Vec2::ZERO);
        assert_eq!(input.mouse_position(), Vec2::new(100.0, 50.0));
    }

    #[test]
    fn end_frame_preserves_held_mouse_button() {
        let mut input = Input::new();
        input.record_mouse_button(MouseButton::Left, true);
        input.end_frame();
        assert!(input.is_mouse_button_down(MouseButton::Left));
        assert!(!input.mouse_button_just_pressed(MouseButton::Left));
    }

    // -- cursor -----------------------------------------------------

    #[test]
    fn cursor_defaults_to_visible_and_not_grabbed() {
        let input = Input::new();
        assert!(!input.cursor_grabbed());
        assert!(input.cursor_visible());
    }

    #[test]
    fn cursor_grab_and_visibility_toggle() {
        let mut input = Input::new();
        input.set_cursor_grabbed(true);
        input.set_cursor_visible(false);
        assert!(input.cursor_grabbed());
        assert!(!input.cursor_visible());
        input.set_cursor_grabbed(false);
        input.set_cursor_visible(true);
        assert!(!input.cursor_grabbed());
        assert!(input.cursor_visible());
    }

    #[test]
    fn end_frame_does_not_touch_cursor_state() {
        let mut input = Input::new();
        input.set_cursor_grabbed(true);
        input.set_cursor_visible(false);
        input.end_frame();
        assert!(input.cursor_grabbed());
        assert!(!input.cursor_visible());
    }

    // -- isolation --------------------------------------------------

    #[test]
    fn keyboard_and_mouse_state_are_independent() {
        let mut input = Input::new();
        input.record_key(KeyCode::KeyW, true);
        input.record_mouse_button(MouseButton::Left, true);
        input.record_key(KeyCode::KeyW, false);
        // Releasing W must not touch the mouse button.
        assert!(!input.is_key_down(KeyCode::KeyW));
        assert!(input.is_mouse_button_down(MouseButton::Left));
    }
}
