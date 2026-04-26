//! Forward renderer.
//!
//! Architecture: see `architecture/render.md` for the why
//! (system-not-subsystem, forward-only, polling-flavored,
//! per-draw uniform with dynamic offset).
//!
//! Game 0 layout:
//! - [`mesh`] — `MeshHandle` keys into the registry. Procedural
//!   mesh data and the GPU buffer wrapper land in subsequent
//!   chunks alongside `MeshRegistry`.
//! - [`light`] — `DirectionalLight`.
//!
//! `Camera`, `Projection`, and `ActiveCamera` live at the engine
//! crate root under [`crate::camera`] — see `architecture/camera.md`
//! for why the renderer is the consumer, not the owner.
//!
//! `RenderContext`, the forward pipeline, the model uniform
//! buffer, and the `render_frame` system arrive in subsequent
//! Phase F chunks.

pub mod context;
pub mod forward;
pub mod light;
pub mod mesh;
pub mod pipeline;
pub mod registry;
pub mod uniforms;

pub use context::{RenderContext, RenderContextError, DEPTH_FORMAT};
pub use forward::render_frame;
pub use light::DirectionalLight;
pub use mesh::{cube_mesh, plane_mesh, MeshData, MeshGpu, MeshHandle, Vertex};
pub use pipeline::{ForwardPipeline, MAX_DRAWS_PER_FRAME, MODEL_UNIFORM_STRIDE};
pub use registry::MeshRegistry;
pub use uniforms::{CameraUniformData, LightUniformData, ModelUniformData};
