use std::any::Any;
use std::sync::Arc;
use std::time::Instant;

use log::{info, warn};
use thiserror::Error;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

use crate::ecs::{IntoSystem, Schedule, Stage, World};
use crate::time::Time;
use crate::window::WindowConfig;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("event loop error: {0}")]
    EventLoop(#[from] winit::error::EventLoopError),
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
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

impl App {
    pub fn new() -> Self {
        let mut world = World::new();
        // Time is engine-intrinsic — insert it now so systems
        // registered before `run()` can declare `Res<Time>` /
        // `ResMut<Time>` without a separate setup step.
        world.insert_resource(Time::default());
        Self {
            window_config: WindowConfig::default(),
            window: None,
            world,
            schedule: Schedule::new(),
            last_frame: None,
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
        self.window = Some(Arc::new(window));
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
            WindowEvent::RedrawRequested => {
                self.tick();
            }
            _ => {}
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
