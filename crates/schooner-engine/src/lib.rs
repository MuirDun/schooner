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
    fps_cursor_toggle, fps_look, fps_move, ActiveCamera, Camera, FpsController, Projection,
};
pub use debug::{
    build_overlay_ui, build_profiler_panel, debug_input_system, DebugState, FrameStats,
    OverlayInteract, OverlayMetrics, ProfilerRow, ProfilerSnapshot, ProfilerView,
    FRAME_STAT_WINDOW,
};
pub use diagnostics::{log_fps_system, log_input_system, FpsLogger};
pub use ecs::{EntityId, Schedule, Stage, World};
pub use error::{EngineError, EngineResult};
pub use input::{Input, KeyCode, MouseButton};
pub use logging::LogConfig;
pub use material::{BlendMode, Material};
pub use render::{
    render_frame, DebugOverlay, DirectionalLight, ForwardPipeline, MeshHandle, MeshRegistry,
    PointLight, SpotLight,
};
pub use time::{Time, DEFAULT_FIXED_HZ, MAX_FIXED_STEPS_PER_FRAME};
pub use transform::Transform;
pub use window::WindowConfig;
