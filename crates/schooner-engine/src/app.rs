use std::any::Any;
use std::sync::Arc;
use std::time::Instant;

use log::{info, warn};
use thiserror::Error;
use winit::application::ApplicationHandler;
use winit::event::{DeviceEvent, DeviceId, ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::PhysicalKey;
use winit::window::{CursorGrabMode, Window, WindowId};

use crate::ecs::{IntoSystem, Schedule, Stage, World};
use crate::input::Input;
use crate::render::{render_frame, ForwardPipeline, MeshRegistry, RenderContext};
use crate::time::Time;
use crate::window::WindowConfig;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("event loop error: {0}")]
    EventLoop(#[from] winit::error::EventLoopError),
    #[error("render context init failed: {0}")]
    RenderContext(#[from] crate::render::RenderContextError),
}

/// Top-level application: owns the window, the ECS world, the
/// schedule, and the frame clock.
///
/// The loop shape is:
///
/// 1. winit fires `about_to_wait` after handling pending events →
///    we ask the window for a redraw.
/// 2. winit fires `RedrawRequested` → we call [`App::tick`], which
///    advances [`Time`], runs `FixedUpdate` 0..N times against the
///    accumulator, and runs `Update` once.
///
/// Render submission joins the loop in Phase F; for now the tick
/// only drives ECS systems.
pub struct App {
    window_config: WindowConfig,
    window: Option<Arc<Window>>,
    world: World,
    schedule: Schedule,
    last_frame: Option<Instant>,
    /// Last cursor-grab state we pushed to the window — used to
    /// elide redundant syscalls on frames where Input did not
    /// change.
    cursor_grab_pushed: bool,
    /// Last cursor-visibility we pushed; same elision purpose.
    cursor_visible_pushed: bool,
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

impl App {
    pub fn new() -> Self {
        let mut world = World::new();
        // Time and Input are engine-intrinsic — insert them now so
        // systems registered before `run()` can declare `Res<Time>`
        // / `Res<Input>` without a separate setup step.
        world.insert_resource(Time::default());
        world.insert_resource(Input::new());
        Self {
            window_config: WindowConfig::default(),
            window: None,
            world,
            schedule: Schedule::new(),
            last_frame: None,
            // Match the OS default: a freshly created winit window
            // is not grabbed and shows the cursor.
            cursor_grab_pushed: false,
            cursor_visible_pushed: true,
        }
    }

    pub fn with_window_config(mut self, config: WindowConfig) -> Self {
        self.window_config = config;
        self
    }

    /// Override the fixed-update rate. Defaults to 60 Hz.
    pub fn with_fixed_hz(mut self, fixed_hz: f32) -> Self {
        self.world.insert_resource(Time::new(fixed_hz));
        self
    }

    /// Insert a resource before the loop starts. For app-level
    /// state that systems read via `Res<R>` / `ResMut<R>`.
    pub fn insert_resource<R: Any + Send + Sync>(mut self, value: R) -> Self {
        self.world.insert_resource(value);
        self
    }

    /// Enable the once-per-second FPS log on the `Update` stage.
    /// Phase H replaces this with an egui overlay; until then it
    /// is the cheapest way to confirm the loop is ticking.
    pub fn with_fps_logging(self) -> Self {
        self.insert_resource(crate::diagnostics::FpsLogger::default())
            .add_system(Stage::Update, crate::diagnostics::log_fps_system)
    }

    /// Log every keyboard / mouse-button edge through `log::info`.
    /// Throwaway smoke test for Phase E — turn it on, press keys,
    /// confirm the pipeline records them.
    pub fn with_input_logging(self) -> Self {
        self.add_system(Stage::Update, crate::diagnostics::log_input_system)
    }

    /// Register a system in the given stage. Systems are run in
    /// registration order within a stage.
    pub fn add_system<M, S: IntoSystem<M>>(mut self, stage: Stage, system: S) -> Self {
        self.schedule.add_system(&mut self.world, stage, system);
        self
    }

    /// Direct mutable access to the world for one-shot setup
    /// (spawning startup entities, inserting components). For
    /// long-lived state that systems need to coordinate over,
    /// prefer a resource.
    pub fn world_mut(&mut self) -> &mut World {
        &mut self.world
    }

    pub fn run(mut self) -> Result<(), AppError> {
        info!("starting event loop");
        let event_loop = EventLoop::new()?;
        // Poll never blocks waiting for OS events — a game wants the loop
        // to tick continuously. Wait (the winit default) is for GUI apps
        // that only redraw on user input.
        event_loop.set_control_flow(ControlFlow::Poll);
        event_loop.run_app(&mut self)?;
        info!("event loop exited");
        Ok(())
    }

    /// One frame: advance [`Time`], run the fixed-step accumulator,
    /// run `Update` once. The first call always reports a delta of
    /// zero — there is no prior frame to measure against.
    fn tick(&mut self) {
        let now = Instant::now();
        let real_delta = match self.last_frame.replace(now) {
            Some(prev) => now.duration_since(prev).as_secs_f32(),
            None => 0.0,
        };

        let steps = match self.world.resource_mut::<Time>() {
            Some(time) => time.advance(real_delta),
            None => {
                // Defensive: nothing in the engine removes the Time
                // resource, but if a system did, we'd lose the
                // accumulator and silently freeze the sim. Surface it.
                warn!("Time resource missing during tick; skipping frame");
                return;
            }
        };

        for _ in 0..steps {
            self.schedule.run_fixed(&mut self.world);
        }
        self.schedule.run(&mut self.world);

        // After systems have run, mirror the requested cursor state
        // onto the actual Window. Doing it here (not before the
        // schedule) means a system that flips grab/visibility this
        // frame takes effect before the next event-loop iteration.
        self.sync_cursor();

        // End-of-frame rollover: clear one-shot edges and per-frame
        // mouse delta so the next frame starts fresh. Held state
        // (down keys, cursor position, grab/visibility) persists.
        if let Some(input) = self.world.resource_mut::<Input>() {
            input.end_frame();
        }
    }

    /// Push the desired cursor state from `Input` onto the live
    /// `Window`, eliding the syscall when nothing changed.
    ///
    /// Cursor grab uses `Locked` first (macOS) and falls back to
    /// `Confined` (Windows / Linux X11+Wayland). Trying both in
    /// order is the standard winit recipe — neither mode is
    /// universally supported, but every desktop OS supports at
    /// least one.
    fn sync_cursor(&mut self) {
        let Some(window) = self.window.as_ref() else {
            return;
        };
        let Some(input) = self.world.resource::<Input>() else {
            return;
        };

        let want_grab = input.cursor_grabbed();
        let want_visible = input.cursor_visible();

        if want_grab != self.cursor_grab_pushed {
            let result = if want_grab {
                window
                    .set_cursor_grab(CursorGrabMode::Locked)
                    .or_else(|_| window.set_cursor_grab(CursorGrabMode::Confined))
            } else {
                window.set_cursor_grab(CursorGrabMode::None)
            };
            match result {
                Ok(()) => self.cursor_grab_pushed = want_grab,
                Err(err) => warn!("set_cursor_grab({want_grab}) failed: {err}"),
            }
        }

        if want_visible != self.cursor_visible_pushed {
            window.set_cursor_visible(want_visible);
            self.cursor_visible_pushed = want_visible;
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        // resumed() fires once at startup on desktop, and again on every
        // mobile foreground return. Guard against recreating the window
        // on subsequent resumes.
        if self.window.is_some() {
            return;
        }

        let attrs = Window::default_attributes()
            .with_title(&self.window_config.title)
            .with_inner_size(self.window_config.size);
        let window = event_loop
            .create_window(attrs)
            .expect("failed to create window");
        let size = window.inner_size();
        info!("window created: {}x{}", size.width, size.height);
        let window = Arc::new(window);

        // Stand up the wgpu device against the just-created window
        // and insert it as a resource. Async init is blocked on
        // here because RenderContext is a hard prerequisite for any
        // frame work; the block happens once per app run.
        match pollster::block_on(RenderContext::new(window.clone())) {
            Ok(ctx) => {
                // Built-in cube + plane upload here, while the
                // device is still in scope, so the registry is
                // ready before any frame work runs. Registry
                // creation is sync — the device handle is a
                // ref-counted, share-safe view.
                let registry = MeshRegistry::with_builtins(ctx.device());
                let pipeline = ForwardPipeline::new(ctx.device(), ctx.surface_format());
                self.world.insert_resource(ctx);
                self.world.insert_resource(registry);
                self.world.insert_resource(pipeline);
            }
            Err(err) => {
                // No way to surface AppError back through the
                // ApplicationHandler trait — winit owns the loop.
                // Log and exit; main() sees the loop terminate
                // without an error, which is the best winit allows.
                log::error!("render context init failed: {err}");
                event_loop.exit();
                return;
            }
        }

        // Append render_frame to Update *after* the user's builder
        // chain has finished registering systems. resumed() runs
        // on the first event-loop tick, by which point App::run
        // has already consumed the builder. This is what enforces
        // "render runs last" without a dedicated Render stage —
        // see plans/game0-plan.md §3.6.1.
        self.schedule
            .add_system(&mut self.world, Stage::Update, render_frame);

        self.window = Some(window);
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => {
                info!("close requested");
                event_loop.exit();
            }
            WindowEvent::Resized(size) => {
                // Forward to the render context. The actual surface
                // reconfigure + depth-texture rebuild happens there;
                // the App just routes the event into the resource.
                if let Some(ctx) = self.world.resource_mut::<RenderContext>() {
                    ctx.resize(size.width, size.height);
                }
            }
            WindowEvent::RedrawRequested => {
                self.tick();
            }
            WindowEvent::KeyboardInput { event, .. } => {
                // Drop OS auto-repeat: `record_key` is idempotent on
                // repeats anyway, but skipping early avoids touching
                // the world borrow per autorepeat tick.
                if event.repeat {
                    return;
                }
                if let PhysicalKey::Code(code) = event.physical_key {
                    let pressed = matches!(event.state, ElementState::Pressed);
                    if let Some(input) = self.world.resource_mut::<Input>() {
                        input.record_key(code, pressed);
                    }
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let pressed = matches!(state, ElementState::Pressed);
                if let Some(input) = self.world.resource_mut::<Input>() {
                    input.record_mouse_button(button, pressed);
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                if let Some(input) = self.world.resource_mut::<Input>() {
                    input.record_mouse_position(position.x as f32, position.y as f32);
                }
            }
            _ => {}
        }
    }

    fn device_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        _device_id: DeviceId,
        event: DeviceEvent,
    ) {
        // Raw mouse motion: unaffected by cursor clamping at screen
        // edges. This is what FPS look-controllers want, not the
        // window-relative `CursorMoved` deltas.
        if let DeviceEvent::MouseMotion { delta: (dx, dy) } = event {
            if let Some(input) = self.world.resource_mut::<Input>() {
                input.record_mouse_motion(dx as f32, dy as f32);
            }
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        // Drives the next RedrawRequested. Without this the loop
        // would only redraw on OS-pushed paints, which never happen
        // under ControlFlow::Poll on a stable window.
        if let Some(window) = self.window.as_ref() {
            window.request_redraw();
        }
    }
}
