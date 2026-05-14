//! Camera subsystem.
//!
//! Architecture: see `architecture/camera.md` for the why
//! (projection / controller split, two-system polling shape, sign
//! conventions, cursor-grab policy).
//!
//! Two halves wired through the ECS, never to each other:
//! - [`projection`] — `Camera`, `Projection`, `ActiveCamera`. Read by
//!   the renderer to build view / projection matrices.
//! - [`controller`] — `FpsController` component plus the `fps_*`
//!   systems that read `Input` + `Time` and write `Transform`.

pub mod controller;
pub mod projection;

pub use controller::{FpsController, fps_cursor_toggle, fps_look, fps_move};
pub use projection::{ActiveCamera, Camera, Projection};
