use std::any::Any;
use std::sync::Arc;
use std::time::Instant;

use log::{info, warn};
use thiserror::Error;
use winit::application::ApplicationHandler;
use winit::event::{DeviceEvent, DeviceId, ElementState, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::PhysicalKey;
use winit::window::{CursorGrabMode, Window, WindowId};

use crate::action::{Actions, Bindings, Trigger, resolve_actions};
use crate::debug::{
    DebugState, ProfilerView, debug_input_system, f5_reload_system,
};
use crate::ecs::{CommandQueue, Events, IntoSystem, Schedule, Stage, World, exclusive};
use crate::input::{Input, KeyCode};
use crate::physics::{Contact, PhysicsWorld, TriggerEnter, physics_bridge};
use crate::symbol::sym;
use crate::render::{
    AutoExposure, Bloom, BloomPipeline, ColorGrade, DebugOverlay, ExposurePipeline, Fog,
    ForwardPipeline, HDR_FORMAT, MeshRegistry, PostOverlay, PostPipeline, RenderContext,
    ShadowMaps, ShadowPipeline, TextureRegistry, Vignette, render_frame,
};
use crate::time::Time;
use crate::window::WindowConfig;

/// Trackpad pixel-scroll normalization: how many physical pixels of
/// `MouseScrollDelta::PixelDelta` scroll equal one notch of
/// `LineDelta`. A notched wheel reports ~±1 line per detent; a trackpad
/// reports raw pixels (tens per gesture). Dividing by this brings the
/// trackpad onto the wheel's scale so wheel-driven gameplay (e.g.
/// telekinesis distance) feels the same on both devices. Empirical, not
/// load-bearing — tuned for feel, not correctness.
const PIXELS_PER_LINE: f32 = 16.0;

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
///    accumulator, then `Update` once, then `Render` once.
///
/// `Render` is the dedicated stage where the forward pass and the
/// debug overlay live. It bumps `current_tick` like every other
/// stage — see `ecs/schedule.rs` for the tick-semantics rationale.
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
    /// One `swap_events::<T>` per registered event type, run once per
    /// frame at tick-top. `fn` pointers, not boxed closures — each is
    /// a monomorphised swap with no captured state.
    event_updaters: Vec<fn(&mut World)>,
    /// Whether physics was requested through `with_physics`. The bridge
    /// is appended during `resumed`, after the complete builder chain,
    /// so it is guaranteed to be the last FixedUpdate system.
    physics_enabled: bool,
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
        // CommandQueue is engine-intrinsic: any system taking a
        // `Commands` param drains into it, and the schedule applies it
        // at every stage boundary. Insert it up front so the first
        // Commands-using system has a queue to write to.
        world.insert_resource(CommandQueue::default());
        // DebugState rides along — visible by default, F1 toggles.
        // The overlay's wgpu plumbing (DebugOverlay) lands later in
        // `resumed` once the device exists.
        world.insert_resource(DebugState::default());
        // ProfilerView registers a sink with puffin's GlobalProfiler
        // immediately on construction so the first frame's data has
        // somewhere to land. The sink survives until ProfilerView
        // is dropped (which happens when the World drops).
        world.insert_resource(ProfilerView::new());
        // Layer 2 input: the binding table (config) and the resolved
        // action state. Both engine-intrinsic like Input, so a game can
        // declare bindings and read actions without a setup step. Started
        // empty; the game binds its actions via the builder.
        world.insert_resource(Bindings::default());
        world.insert_resource(Actions::default());
        let mut schedule = Schedule::new();
        // Resolve the action map first each Update so every gameplay
        // system this frame reads fresh action state. It must stay on
        // Update — edges and the wheel are frame-scoped, and a
        // FixedUpdate resolve would double-fire or miss them (see the
        // fixed-step discipline in architecture/input.md).
        schedule.add_system(&mut world, Stage::Update, resolve_actions);
        // F1 toggle runs in Update so the visibility flip is
        // observable to render_frame the same frame.
        schedule.add_system(&mut world, Stage::Update, debug_input_system);
        Self {
            window_config: WindowConfig::default(),
            window: None,
            world,
            schedule,
            last_frame: None,
            // Match the OS default: a freshly created winit window
            // is not grabbed and shows the cursor.
            cursor_grab_pushed: false,
            cursor_visible_pushed: true,
            event_updaters: Vec::new(),
            physics_enabled: false,
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

    /// Enable the Rapier world and its ECS bridge.
    ///
    /// Physics is opt-in because not every engine application needs a
    /// solver. The bridge itself is appended after the builder chain is
    /// complete, preserving the fixed-step order: gameplay writes intent,
    /// then physics solves and writes the settled poses back.
    pub fn with_physics(mut self) -> Self {
        if self.physics_enabled {
            return self;
        }

        self.world.insert_resource(PhysicsWorld::new());
        self.world.insert_resource(Events::<Contact>::default());
        self.world.insert_resource(Events::<TriggerEnter>::default());
        self.event_updaters
            .push(crate::ecs::event::swap_events::<Contact>);
        self.event_updaters
            .push(crate::ecs::event::swap_events::<TriggerEnter>);
        self.physics_enabled = true;
        self
    }

    /// Insert a resource before the loop starts. For app-level
    /// state that systems read via `Res<R>` / `ResMut<R>`.
    pub fn insert_resource<R: Any + Send + Sync>(mut self, value: R) -> Self {
        self.world.insert_resource(value);
        self
    }

    /// Register a discrete-event type `T`: insert its [`Events<T>`]
    /// resource and arrange for the queue to be swapped once per frame
    /// at the top of [`App::tick`]. Producers then write
    /// `ResMut<Events<T>>`; readers poll `Res<Events<T>>`. Idempotent
    /// only if you don't register the same `T` twice — a second call
    /// resets the queue and adds a redundant swap.
    pub fn add_event<T: Send + Sync + 'static>(mut self) -> Self {
        self.world.insert_resource(Events::<T>::default());
        self.event_updaters.push(crate::ecs::event::swap_events::<T>);
        self
    }

    /// Bind an action name to a physical [`Trigger`]. Repeated calls on
    /// the same name OR the triggers together — any one fires the action.
    /// The name is interned through the global symbol table, so a later
    /// script (or another call site) binding the same name lands on the
    /// same action.
    pub fn bind(mut self, action: &str, trigger: Trigger) -> Self {
        if let Some(bindings) = self.world.resource_mut::<Bindings>() {
            bindings.bind(sym(action), trigger);
        }
        self
    }

    /// Ergonomic alias for `bind(action, Trigger::Key(key))`.
    pub fn bind_key(self, action: &str, key: KeyCode) -> Self {
        self.bind(action, Trigger::Key(key))
    }

    /// Replace an action's triggers with a single one (clear-then-bind).
    pub fn rebind(mut self, action: &str, trigger: Trigger) -> Self {
        if let Some(bindings) = self.world.resource_mut::<Bindings>() {
            bindings.rebind(sym(action), trigger);
        }
        self
    }

    /// Enable the once-per-second FPS log on the `Update` stage.
    /// Cheap, log-driven; useful in headless smoke tests or when
    /// the egui overlay is hidden.
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
        // Per-frame puffin housekeeping. `set_scopes_on` is a relaxed
        // atomic store — calling it every frame is essentially free,
        // and it lets the profiler checkbox in the overlay flip
        // collection on/off without the App needing extra signal
        // plumbing. `new_frame` flushes the previous frame's data
        // into all registered sinks (including ProfilerView).
        let scopes_on = self
            .world
            .resource::<DebugState>()
            .map(|d| d.show_profiler)
            .unwrap_or(false);
        puffin::set_scopes_on(scopes_on);
        puffin::GlobalProfiler::lock().new_frame();
        puffin::profile_scope!("frame");

        // Reactive substrate: rotate the per-frame removed-component
        // ledger at frame top so this frame's removals start fresh
        // while last frame's stay readable — a one-frame-late reactor
        // (e.g. physics handle cleanup) never misses one.
        self.world.swap_removed();

        // Swap every registered Events<T> queue at the same point: last
        // frame's events stay readable, this frame's start fresh. `&f`
        // copies the fn pointer out (Copy), so the loop's immutable
        // borrow of `event_updaters` and the `&mut self.world` call are
        // disjoint-field borrows.
        for &swap in &self.event_updaters {
            swap(&mut self.world);
        }

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
        self.schedule.run_render(&mut self.world);

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
                // WHITE 1×1 default-fallback texture lands now alongside
                // the cube/plane built-ins so materials with no albedo
                // texture can bind a real GPU view (uniform shader path)
                // before any later loads happen.
                let texture_registry = TextureRegistry::with_builtins(ctx.device(), ctx.queue());
                // Forward writes into the HDR offscreen target since
                // 1.D.1, so the pipeline's color target format must
                // be HDR_FORMAT — not the swap-chain format the post
                // pipeline targets.
                let pipeline = ForwardPipeline::new(ctx.device(), HDR_FORMAT);
                // The shadow pipeline reuses the forward pipeline's
                // per-draw model buffer through a refcounted bind
                // group, so one queue write of model uniforms feeds
                // both the shadow and forward passes.
                let shadow_pipeline = ShadowPipeline::new(ctx.device(), &pipeline.model_buffer);
                // ShadowMaps allocates the 2D-array depth texture
                // upfront (16 MB), builds its per-layer views, and
                // constructs the forward-side @group(3) bind group
                // against the pipeline's shadow BGL + comparison
                // sampler. Per-frame work is just `set_active_count`.
                let shadow_maps = ShadowMaps::new(
                    ctx.device(),
                    &pipeline.shadow_bgl,
                    &pipeline.comparison_sampler,
                );
                let overlay = DebugOverlay::new(window.clone(), ctx.device(), ctx.surface_format());
                // PostPipeline reads the HDR target from RenderContext
                // and writes the swap chain. Its HDR-sampling bind
                // group is built lazily in render_frame the first
                // time it's needed (and rebuilt after every resize).
                let post_pipeline = PostPipeline::new(ctx.device(), ctx.surface_format());
                // BloomPipeline builds the HDR bright-pass pyramid the
                // post pass composites in before tonemap. It renders in
                // HDR_FORMAT (same space as the forward output); its mip
                // chain is allocated lazily in render_frame on the first
                // frame and rebuilt on resize.
                let bloom_pipeline = BloomPipeline::new(ctx.device(), HDR_FORMAT);
                // ExposurePipeline meters the HDR target's average luminance
                // and adapts the exposure scalar the post pass applies before
                // the tone curve (eye adaptation). Its luma chain is
                // allocated lazily in render_frame and rebuilt on resize.
                let exposure_pipeline = ExposurePipeline::new(ctx.device());
                self.world.insert_resource(ctx);
                self.world.insert_resource(registry);
                self.world.insert_resource(texture_registry);
                self.world.insert_resource(pipeline);
                self.world.insert_resource(shadow_pipeline);
                self.world.insert_resource(shadow_maps);
                self.world.insert_resource(post_pipeline);
                self.world.insert_resource(bloom_pipeline);
                self.world.insert_resource(exposure_pipeline);
                // Auto-exposure seeded ON with the general interior default.
                // Games swap the resource (e.g. AutoExposure::CHAMBER) to
                // tune the adaptation, or AutoExposure::OFF to disable.
                self.world.insert_resource(AutoExposure::DEFAULT);
                // Bloom seeded with the restrained FAINT default — only
                // genuinely over-bright sources glow, gently. F7 cycles
                // it up through ERA_GLOW to EVERYTHING_GLOWS; the
                // bloom_preset in DebugState starts at Faint to match.
                self.world.insert_resource(Bloom::DEFAULT);
                // Default ColorGrade = identity (no-op) and default
                // Vignette = off. Games swap the resource values to
                // drive per-scene mood.
                self.world.insert_resource(ColorGrade::CAGE_WARM);
                self.world.insert_resource(Vignette::CINEMATIC);
                // Fog seeded with a moderate cool-grey medium so the
                // 1.E.1 smoke test is visible without authoring effort.
                // 1.E.3 will replace this with named presets driven by
                // an F4 debug cycle, matching the F2/F3 grade/vignette
                // pattern.
                self.world.insert_resource(Fog {
                    color: glam::Vec3::new(0.55, 0.58, 0.62),
                    base_height: 0.0,
                    density: 0.08,
                    falloff: 0.5,
                    scattering: 0.5,
                });
                // PostOverlay seeded off (no texture, intensity 0).
                // F6 cycles its intensity + blend; the game points
                // `.texture` at a test asset so the cycle composites a
                // real image.
                self.world.insert_resource(PostOverlay::DEFAULT);
                self.world.insert_resource(overlay);
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

        // Append render_frame to the dedicated Render stage. It
        // lands here from resumed() rather than from App::new() so
        // any user systems registered for Stage::Render via the
        // builder chain run before the engine's frame system —
        // useful for debug viewers that want to read pre-render
        // state. `render_frame` is exclusive — see its module doc
        // for why.
        self.schedule
            .add_system(&mut self.world, Stage::Render, exclusive(render_frame));

        // Physics does not depend on the GPU, but this is the first point
        // after the user's builder chain has finished. Appending here makes
        // the bridge last in FixedUpdate regardless of whether
        // `with_physics()` appeared before or after gameplay systems.
        if self.physics_enabled {
            self.schedule
                .add_system(&mut self.world, Stage::FixedUpdate, exclusive(physics_bridge));
        }

        // F5 manual asset reload. Registered here, not in App::new(),
        // because it reads RenderContext + the registries + the
        // forward pipeline — resources that only exist once the device
        // is up. See `debug::f5_reload_system`.
        self.schedule
            .add_system(&mut self.world, Stage::Update, f5_reload_system);

        // Drain the Startup stage exactly once, now that the device,
        // registries and pipelines exist. Scene builders and asset
        // loads registered for Stage::Startup run here — at the one
        // correct moment, before the first frame, with no latch.
        self.schedule.run_startup(&mut self.world);

        self.window = Some(window);
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        // Funnel the event through the egui overlay first. If egui
        // consumed it (mouse over an overlay window, keyboard focus
        // in a text field), short-circuit so the gameplay-side
        // Input resource doesn't *also* see the event. Close /
        // Resize / Focus are still processed below regardless —
        // those are App-level signals, not user-typed input.
        let egui_consumed = if let Some(overlay) = self.world.resource_mut::<DebugOverlay>() {
            overlay.on_window_event(&event).consumed
        } else {
            false
        };

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
            WindowEvent::Focused(focused) => {
                // Auto-grab on focus gain so the player drops into
                // the FPS controller without needing a click. Release
                // on focus loss so alt-tabbing doesn't strand the
                // cursor locked over the desktop with no way out.
                // The intent is written to Input; sync_cursor mirrors
                // it onto the live Window after the next schedule.
                if let Some(input) = self.world.resource_mut::<Input>() {
                    input.set_cursor_grabbed(focused);
                    input.set_cursor_visible(!focused);
                }
            }
            WindowEvent::RedrawRequested => {
                self.tick();
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if egui_consumed {
                    // egui owns the keystroke (e.g. text-field focus).
                    // Skip recording it on the gameplay-side Input so
                    // typing in the overlay can't double-fire WASD.
                    return;
                }
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
                if egui_consumed {
                    return;
                }
                let pressed = matches!(state, ElementState::Pressed);
                if let Some(input) = self.world.resource_mut::<Input>() {
                    input.record_mouse_button(button, pressed);
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                if egui_consumed {
                    return;
                }
                if let Some(input) = self.world.resource_mut::<Input>() {
                    input.record_mouse_position(position.x as f32, position.y as f32);
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                if egui_consumed {
                    return;
                }
                // Normalize winit's two report shapes onto one unit
                // (lines/notches) so the Input resource — and every
                // wheel consumer above it — sees a single scale. A
                // notched mouse sends LineDelta; a trackpad / high-res
                // device sends PixelDelta. We only track the vertical
                // axis; no Kinesis verb reads horizontal scroll.
                let lines = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(pos) => pos.y as f32 / PIXELS_PER_LINE,
                };
                if let Some(input) = self.world.resource_mut::<Input>() {
                    input.record_mouse_wheel(lines);
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
