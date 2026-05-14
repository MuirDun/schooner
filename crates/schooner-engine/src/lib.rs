pub mod app;
pub mod camera;
pub mod debug;
pub mod diagnostics;
pub mod ecs;
pub mod error;
pub mod input;
pub mod logging;
pub mod material;
pub mod render;
pub mod time;
pub mod transform;
pub mod window;

pub use app::{App, AppError};
pub use camera::{
    ActiveCamera, Camera, FpsController, Projection, fps_cursor_toggle, fps_look, fps_move,
};
pub use debug::{
    DebugState, FRAME_STAT_WINDOW, FrameStats, OverlayInteract, OverlayMetrics, PcfKernel,
    ProfilerRow, ProfilerSnapshot, ProfilerView, build_overlay_ui, build_profiler_panel,
    debug_input_system,
};
pub use diagnostics::{FpsLogger, log_fps_system, log_input_system};
pub use ecs::{EntityId, Schedule, Stage, World};
pub use error::{EngineError, EngineResult};
pub use input::{Input, KeyCode, MouseButton};
pub use logging::LogConfig;
pub use material::{BlendMode, Material};
pub use render::{
    DebugOverlay, DirectionalLight, ForwardPipeline, MeshHandle, MeshRegistry, PointLight,
    ShadowMaps, ShadowPipeline, Shadowcaster, SpotLight, render_frame,
};
pub use time::{DEFAULT_FIXED_HZ, MAX_FIXED_STEPS_PER_FRAME, Time};
pub use transform::Transform;
pub use window::WindowConfig;
