//! Camera subsystem.
//!
//! Architecture: see `architecture/camera.md` for the why
//! (projection / controller split, two-system polling shape, sign
//! conventions, cursor-grab policy).
//!
//! Two halves wired through the ECS, never to each other:
//! - [`projection`] — `Camera`, `Projection`, `ActiveCamera`. Read by
//!   the renderer to build view / projection matrices.
//! - [`controller`] — `FpsController` plus production look/cursor systems.
//! - `debug` — the `dev-tools` spectator camera and free-flight movement.

pub mod controller;
#[cfg(feature = "dev-tools")]
mod debug;
pub mod projection;

pub use controller::{FpsController, fps_cursor_toggle, fps_look};
#[cfg(feature = "dev-tools")]
pub use debug::{CameraDebugPlugin, SpectatorCamera, SpectatorDebugState, spectator_move};
pub use projection::{ActiveCamera, Camera, Projection};
