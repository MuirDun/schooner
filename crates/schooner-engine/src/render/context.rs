//! `RenderContext` — the wgpu device handle resource.
//!
//! Owns the four pieces of GPU state that survive across frames:
//! the device + queue, the configured surface, and the depth
//! texture. Built once during `App::resumed` (when the window
//! exists) and inserted into the `World` as a resource so the
//! `render_frame` system can declare `ResMut<RenderContext>` and
//! reach the device through the normal ECS contract.
//!
//! Initialization is async — `request_adapter` and `request_device`
//! both return futures. Game 0 blocks on them at the call site
//! with `pollster::block_on`. The block happens once per app run,
//! during window setup, so no frame work waits on it.
//!
//! `Surface<'static>` is achievable because `App` already holds the
//! window in `Arc<Window>`. Cloning the `Arc` into the surface
//! creator (`Instance::create_surface`) makes the surface outlast
//! any temporary borrow of the window.

use std::sync::Arc;

use log::{info, warn};
use thiserror::Error;
use wgpu::{
    Adapter, Backends, CompositeAlphaMode, Device, DeviceDescriptor, Extent3d, Features, Instance,
    InstanceDescriptor, Limits, MemoryHints, PowerPreference, PresentMode, Queue,
    RequestAdapterOptions, RequestDeviceError, Surface, SurfaceConfiguration, SurfaceError,
    SurfaceTexture, TextureDescriptor, TextureDimension, TextureFormat, TextureUsages, TextureView,
    TextureViewDescriptor,
};
use winit::window::Window;

/// Format of the depth attachment used by the forward pipeline.
///
/// `Depth32Float` is universally supported on every wgpu backend
/// without feature flags. Reverse-Z and stencil land in Game 3 (or
/// later) when outdoor depth precision starts to matter; for Game 0
/// the cheapest universal format wins.
pub const DEPTH_FORMAT: TextureFormat = TextureFormat::Depth32Float;

/// Errors produced while standing up a `RenderContext`.
///
/// Anything that fails here is fatal — we cannot draw without a
/// device — so the App propagates this back to the caller of
/// `run()`. It is not a soft error.
#[derive(Debug, Error)]
pub enum RenderContextError {
    #[error("wgpu surface creation failed: {0}")]
    CreateSurface(#[from] wgpu::CreateSurfaceError),
    #[error("no GPU adapter compatible with the surface")]
    NoCompatibleAdapter,
    #[error("device request failed: {0}")]
    RequestDevice(#[from] RequestDeviceError),
}

/// Per-frame GPU handles. Lives in the `World` as a resource;
/// `render_frame` borrows it mutably to acquire the next swap-chain
/// texture and submit commands.
///
/// The fields are crate-private; construction goes through `new`
/// and `acquire_frame` / `resize` are the supported mutators. Direct
/// mutation would risk leaving the depth texture out of sync with
/// the surface configuration.
pub struct RenderContext {
    // Note: `Instance` and `Adapter` are intentionally not retained.
    // `Device` / `Queue` are refcounted handles that keep the
    // backend alive on their own, and `Surface<'static>` carries
    // its own ref to the window — the instance and adapter are
    // only needed at construction time.
    device: Device,
    queue: Queue,
    surface: Surface<'static>,
    config: SurfaceConfiguration,
    depth_view: TextureView,
}

impl RenderContext {
    /// Stand up the GPU device and configure the surface against
    /// the given window. Async because `request_adapter` and
    /// `request_device` both return futures; the App blocks on this
    /// once during `resumed`.
    pub async fn new(window: Arc<Window>) -> Result<Self, RenderContextError> {
        // PRIMARY = native backends (Metal/Vulkan/DX12) only. WebGL
        // is excluded by design — Game 0 is desktop-only. If WASM
        // ever lands, this is the line that opens it.
        let instance = Instance::new(&InstanceDescriptor {
            backends: Backends::PRIMARY,
            ..Default::default()
        });

        // Cloning the Arc into create_surface is what gives us
        // Surface<'static>. Without the static surface the
        // RenderContext would carry the window's lifetime, which
        // doesn't compose with storing it as a resource.
        let surface = instance.create_surface(window.clone())?;

        // HighPerformance asks for the discrete GPU on laptops with
        // hybrid graphics. compatible_surface excludes adapters
        // that can't present to our surface — relevant on Linux
        // when both Vulkan and llvmpipe show up.
        let adapter = instance
            .request_adapter(&RequestAdapterOptions {
                power_preference: PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .ok_or(RenderContextError::NoCompatibleAdapter)?;

        let info = adapter.get_info();
        info!(
            "selected adapter: {} ({:?}, backend {:?})",
            info.name, info.device_type, info.backend
        );

        // No extra Features requested — Game 0 commits to baseline
        // wgpu (see architecture/render.md "Why per-draw uniform
        // with dynamic offset"). MemoryHints::Performance lets the
        // driver pick allocation strategy without the engine
        // pretending to know better.
        let (device, queue) = adapter
            .request_device(
                &DeviceDescriptor {
                    label: Some("schooner-device"),
                    required_features: Features::empty(),
                    required_limits: Limits::default(),
                    memory_hints: MemoryHints::Performance,
                },
                None,
            )
            .await?;

        let size = window.inner_size();
        let config = build_surface_config(&adapter, &surface, size.width, size.height);
        surface.configure(&device, &config);

        let depth_view = create_depth_attachment(&device, config.width, config.height);

        info!(
            "render context ready: {}x{} {:?}",
            config.width, config.height, config.format
        );

        // `instance` and `adapter` drop here — see struct comment.
        drop(instance);
        drop(adapter);

        Ok(Self {
            device,
            queue,
            surface,
            config,
            depth_view,
        })
    }

    pub fn device(&self) -> &Device {
        &self.device
    }

    pub fn queue(&self) -> &Queue {
        &self.queue
    }

    pub fn surface_format(&self) -> TextureFormat {
        self.config.format
    }

    pub fn surface_size(&self) -> (u32, u32) {
        (self.config.width, self.config.height)
    }

    pub fn depth_view(&self) -> &TextureView {
        &self.depth_view
    }

    pub fn aspect_ratio(&self) -> f32 {
        // height==0 happens when the window is minimized. Returning
        // 1.0 keeps the projection matrix non-degenerate; the frame
        // wouldn't render anyway under the zero-size guard in
        // acquire_frame.
        if self.config.height == 0 {
            1.0
        } else {
            self.config.width as f32 / self.config.height as f32
        }
    }

    /// React to a window resize: reconfigure the surface and
    /// recreate the depth texture at the new dimensions.
    ///
    /// Width or height of 0 (minimization) is a no-op rather than
    /// an error — wgpu rejects zero-sized surface configs, and
    /// reconfiguring on every minimize/restore would thrash GPU
    /// allocations. The next non-zero resize picks up the change.
    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        if (width, height) == (self.config.width, self.config.height) {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
        self.depth_view = create_depth_attachment(&self.device, width, height);
        info!("surface reconfigured: {width}x{height}");
    }

    /// Acquire the next swap-chain texture. Returns `None` when the
    /// caller should skip this frame (zero-sized window, or a
    /// recoverable surface error that we just reconfigured around).
    ///
    /// `Lost` / `Outdated` are routine on macOS during normal
    /// resizes — the right reaction is to reconfigure and try again
    /// next frame, not to crash. `OutOfMemory` is unrecoverable and
    /// panics; `Timeout` is benign and skips a frame.
    pub fn acquire_frame(&mut self) -> Option<SurfaceTexture> {
        if self.config.width == 0 || self.config.height == 0 {
            return None;
        }
        match self.surface.get_current_texture() {
            Ok(frame) => Some(frame),
            Err(SurfaceError::Lost | SurfaceError::Outdated) => {
                warn!("surface lost/outdated; reconfiguring");
                self.surface.configure(&self.device, &self.config);
                None
            }
            Err(SurfaceError::Timeout) => {
                warn!("surface acquire timed out; skipping frame");
                None
            }
            Err(SurfaceError::OutOfMemory) => {
                panic!("surface out of memory — unrecoverable");
            }
            Err(SurfaceError::Other) => {
                warn!("surface acquire failed with unspecified error; skipping frame");
                None
            }
        }
    }
}

/// Build the initial surface configuration. Picks an sRGB format
/// from the adapter's reported list when available — sRGB is what
/// the forward shader writes for, doing tone-map / gamma in the
/// hardware on present rather than in the fragment shader.
fn build_surface_config(
    adapter: &Adapter,
    surface: &Surface<'_>,
    width: u32,
    height: u32,
) -> SurfaceConfiguration {
    let caps = surface.get_capabilities(adapter);
    let format = caps
        .formats
        .iter()
        .copied()
        .find(TextureFormat::is_srgb)
        .unwrap_or(caps.formats[0]);
    SurfaceConfiguration {
        usage: TextureUsages::RENDER_ATTACHMENT,
        format,
        width,
        height,
        // Fifo is universally supported and matches vsync.
        // Mailbox/Immediate land later if vsync-off is wanted.
        present_mode: PresentMode::Fifo,
        alpha_mode: CompositeAlphaMode::Auto,
        view_formats: vec![],
        desired_maximum_frame_latency: 2,
    }
}

fn create_depth_attachment(device: &Device, width: u32, height: u32) -> TextureView {
    // The `Texture` is dropped at the end of this function — the
    // `TextureView` keeps the underlying GPU allocation alive as
    // long as it lives, and nothing else in the engine ever needs
    // a handle back to the texture itself.
    let texture = device.create_texture(&TextureDescriptor {
        label: Some("schooner-depth"),
        size: Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format: DEPTH_FORMAT,
        usage: TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    texture.create_view(&TextureViewDescriptor::default())
}
