//! Camera subsystem.
//!
//! Architecture: see `architecture/camera.md` for the why
//! (projection / controller split, two-system polling shape, sign
//! conventions, cursor-grab policy).
//!
//! Two halves wired through the ECS, never to each other:
//! - [`projection`] — `Camera`, `Projection`, `ActiveCamera`. Read by
//!   the renderer to build view / projection matrices.
//! - `controller` — `FpsController` + `fps_*` systems land in Phase G.

pub mod projection;

pub use projection::{ActiveCamera, Camera, Projection};
