//! `DebugOverlay` — egui-on-wgpu integration as a render-pass tail.
//!
//! Owns the three pieces of egui state that survive across frames:
//!
//! - `egui::Context` — the immediate-mode UI state (windows, focus,
//!   text-edit cursors, animation timing).
//! - `egui_winit::State` — translates winit events into egui events,
//!   manages cursor-icon and IME requests back to the OS.
//! - `egui_wgpu::Renderer` — owns the egui shader/pipeline and the
//!   per-frame texture/vertex/index uploads.
//!
//! The overlay also holds an `Arc<Window>` so frame-build and event-
//! forwarding APIs don't have to thread the window in from the App
//! every call. `Window` is the OS handle, not a render target — it's
//! safe to keep a clone here alongside the App's own clone.
//!
//! ## Lifecycle inside a frame
//!
//! 1. Each `WindowEvent` reaches `App::window_event`. Before any
//!    gameplay-side input handling, the App calls
//!    [`DebugOverlay::on_window_event`]. If egui consumed the event
//!    (mouse over an overlay window, keyboard focus in a text field),
//!    the App skips feeding it into the gameplay `Input` resource.
//! 2. Inside `render_frame`, after the forward pass is encoded but
//!    before submit, the renderer calls [`DebugOverlay::run`] with a
//!    closure that builds the UI, then [`DebugOverlay::render`] to
//!    encode the egui pass that loads (does not clear) the existing
//!    color attachment.
//!
//! egui needs no depth buffer for its 2D pass — `RendererOptions`'
//! `depth_stencil_format` is set to `None`.

use std::sync::Arc;

use egui::{Context, FullOutput, ViewportId};
use egui_wgpu::{Renderer, RendererOptions, ScreenDescriptor};
use egui_winit::{EventResponse, State};
use wgpu::{
    CommandEncoder, Device, LoadOp, Operations, Queue, RenderPassColorAttachment,
    RenderPassDescriptor, StoreOp, TextureFormat, TextureView,
};
use winit::event::WindowEvent;
use winit::window::Window;

pub struct DebugOverlay {
    ctx: Context,
    state: State,
    renderer: Renderer,
    window: Arc<Window>,
    /// Output of the most recent `run()`, consumed on the next
    /// `render()`. Held across the gap between "build the UI" and
    /// "encode the pass" so the renderer can do other work in
    /// between (e.g. close the forward pass) without losing the
    /// egui shape lists.
    pending: Option<FullOutput>,
}

impl DebugOverlay {
    pub fn new(window: Arc<Window>, device: &Device, surface_format: TextureFormat) -> Self {
        let ctx = Context::default();
        let state = State::new(
            ctx.clone(),
            ViewportId::ROOT,
            window.as_ref(),
            Some(window.scale_factor() as f32),
            None,
            // Default max_texture_side: let egui-winit query the
            // adapter limit via the window's display handle.
            None,
        );
        let renderer = Renderer::new(
            device,
            surface_format,
            RendererOptions {
                msaa_samples: 1,
                // No depth: egui draws in 2D on top of whatever the
                // forward pass left behind.
                depth_stencil_format: None,
                // Dithering masks the banding sRGB-blended UIs can
                // show on dark backgrounds. Cheap, on by default.
                dithering: true,
                // Snapshot-reproducibility flag — only relevant for
                // image-diff testing. We don't snapshot-test the
                // overlay, so leave it at the lazy default.
                predictable_texture_filtering: false,
            },
        );
        Self {
            ctx,
            state,
            renderer,
            window,
            pending: None,
        }
    }

    /// Read access to the egui [`Context`]. Useful for callers that
    /// need `pixels_per_point` for `ScreenDescriptor` between `run`
    /// and `render`, or that want to query egui state outside the
    /// frame closure.
    pub fn context(&self) -> &Context {
        &self.ctx
    }

    /// Forward a winit event to egui. The returned [`EventResponse`]
    /// tells the App whether egui consumed the event — when it did,
    /// gameplay-side input must skip the event so e.g. typing in an
    /// overlay text field doesn't also fire WASD.
    pub fn on_window_event(&mut self, event: &WindowEvent) -> EventResponse {
        self.state.on_window_event(self.window.as_ref(), event)
    }

    /// Run the egui frame: take accumulated input, invoke the
    /// supplied closure to build the UI, store the resulting paint
    /// jobs for the next [`DebugOverlay::render`].
    ///
    /// Uses `begin_pass` / `end_pass` rather than the deprecated
    /// `Context::run` so the closure can be `FnOnce` (no `FnMut`
    /// requirement leaking out) and so the caller receives a plain
    /// `&Context` for the usual `egui::Window::new(...).show(ctx)`
    /// patterns.
    pub fn run(&mut self, build_ui: impl FnOnce(&Context)) {
        let raw_input = self.state.take_egui_input(self.window.as_ref());
        self.ctx.begin_pass(raw_input);
        build_ui(&self.ctx);
        let output = self.ctx.end_pass();
        // PlatformOutput carries cursor-icon and IME requests; pass
        // them back to winit so e.g. text-field hover changes the
        // cursor and IME windows track focus.
        self.state
            .handle_platform_output(self.window.as_ref(), output.platform_output.clone());
        self.pending = Some(output);
    }

    /// Encode the egui pass into `encoder` against `target`. The
    /// pass *loads* the existing color attachment so the forward
    /// pass's contents stay underneath. No-op if [`Self::run`] was
    /// not called this frame.
    pub fn render(
        &mut self,
        device: &Device,
        queue: &Queue,
        encoder: &mut CommandEncoder,
        target: &TextureView,
        size_in_pixels: [u32; 2],
        pixels_per_point: f32,
    ) {
        let Some(output) = self.pending.take() else {
            return;
        };
        let paint_jobs = self
            .ctx
            .tessellate(output.shapes, output.pixels_per_point);
        let screen = ScreenDescriptor {
            size_in_pixels,
            pixels_per_point,
        };

        // Apply texture deltas (font atlas, user-allocated images)
        // *before* update_buffers so any new IDs the paint jobs
        // reference are bound when the pass dispatches.
        for (id, image_delta) in &output.textures_delta.set {
            self.renderer.update_texture(device, queue, *id, image_delta);
        }
        self.renderer
            .update_buffers(device, queue, encoder, &paint_jobs, &screen);

        {
            // egui-wgpu's `render` takes `&mut RenderPass<'static>`,
            // so the pass must outlive the encoder borrow. wgpu's
            // `forget_lifetime` is the documented escape hatch for
            // exactly this case (encoder + pass stored adjacently
            // in one data structure).
            let mut pass = encoder
                .begin_render_pass(&RenderPassDescriptor {
                    label: Some("egui-pass"),
                    color_attachments: &[Some(RenderPassColorAttachment {
                        view: target,
                        depth_slice: None,
                        resolve_target: None,
                        ops: Operations {
                            load: LoadOp::Load,
                            store: StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    multiview_mask: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                })
                .forget_lifetime();
            self.renderer.render(&mut pass, &paint_jobs, &screen);
        }

        // Free *after* the pass — egui's `set` deltas may
        // reference textures that `free` would otherwise drop.
        for id in &output.textures_delta.free {
            self.renderer.free_texture(id);
        }
    }
}
