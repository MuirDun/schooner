use std::sync::Arc;

use log::info;
use thiserror::Error;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

use crate::window::WindowConfig;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("event loop error: {0}")]
    EventLoop(#[from] winit::error::EventLoopError),
}

#[derive(Default)]
pub struct App {
    window_config: WindowConfig,
    window: Option<Arc<Window>>,
}

impl App {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_window_config(mut self, config: WindowConfig) -> Self {
        self.window_config = config;
        self
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
            _ => {}
        }
    }
}
