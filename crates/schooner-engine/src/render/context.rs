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
    Adapter, Backends, CompositeAlphaMode, CurrentSurfaceTexture, Device, DeviceDescriptor,
    ExperimentalFeatures, Extent3d, Features, Instance, InstanceDescriptor, Limits, MemoryHints,
    PowerPreference, PresentMode, Queue, RequestAdapterError, RequestAdapterOptions,
    RequestDeviceError, Surface, SurfaceConfiguration, SurfaceTexture, TextureDescriptor,
    TextureDimension, TextureFormat, TextureUsages, TextureView, TextureViewDescriptor, Trace,
};
use winit::window::Window;

/// Format of the depth attachment used by the forward pipeline.
///
/// `Depth32Float` is universally supported on every wgpu backend
/// without feature flags. Reverse-Z and stencil land in Game 3 (or
/// later) when outdoor depth precision starts to matter; for Game 0
/// the cheapest universal format wins.
pub const DEPTH_FORMAT: TextureFormat = TextureFormat::Depth32Float;

/// Format of the HDR offscreen color target the forward pass writes
/// into.
///
/// `Rgba16Float` gives a wide enough linear range that the forward
/// shader can hand out values > 1.0 (sun + emissive overlap, future
/// bloom seed) without clipping, while staying half the bandwidth of
/// `Rgba32Float`. The post-process pass samples this target, applies
/// the fixed pipeline (tonemap → grade → vignette → overlay), and
/// writes the result into the sRGB swap-chain texture — hardware
/// encodes linear → sRGB on present.
pub const HDR_FORMAT: TextureFormat = TextureFormat::Rgba16Float;

/// Multisample count for the forward pass — coverage anti-aliasing on
/// geometry silhouettes.
///
/// MSAA shades once per *pixel* but evaluates triangle *coverage* per
/// sample, so it cleans up the jagged silhouettes this game is full of
/// (hard-edged steel panels, weld seams, the tool meshes against the
/// dark) for a small bandwidth cost. It does **not** touch shading
/// aliasing (specular sparkle) — that is a separate problem.
///
/// `4` is the desktop sweet spot: the resolve is a 4-sample box filter,
/// and 8× rarely earns its extra bandwidth for a scene like this. The
/// resolve runs in **linear HDR, before the tone curve** — the forward
/// target is `Rgba16Float` and tonemapping lives in the post pass, so
/// the multisampled samples are averaged in linear light and only then
/// rolled through the curve. Resolving after tonemap would average
/// nonlinear values and reintroduce the aliasing MSAA is meant to fix.
pub const MSAA_SAMPLE_COUNT: u32 = 4;

/// Errors produced while standing up a `RenderContext`.
///
/// Anything that fails here is fatal — we cannot draw without a
/// device — so the App propagates this back to the caller of
/// `run()`. It is not a soft error.
#[derive(Debug, Error)]
pub enum RenderContextError {
    #[error("wgpu surface creation failed: {0}")]
    CreateSurface(#[from] wgpu::CreateSurfaceError),
    #[error("no GPU adapter compatible with the surface: {0}")]
    RequestAdapter(#[from] RequestAdapterError),
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
    /// Multisampled HDR target the forward pass *renders* into (one
    /// color sample per coverage sample). Never sampled — it is
    /// resolved into `hdr_view` at the end of the forward pass, so it
    /// carries `RENDER_ATTACHMENT` only. Recreated on resize.
    hdr_ms_view: TextureView,
    /// Single-sample HDR target the forward pass *resolves* into and
    /// the post chain (bloom, exposure, tonemap) samples. This is the
    /// view post bind groups reference; the multisampled `hdr_ms_view`
    /// stays private to the forward pass. Recreated alongside the depth
    /// attachment on resize.
    hdr_view: TextureView,
    /// Monotonically bumped each time `hdr_view` is recreated. The
    /// post pipeline reads this to decide whether its cached
    /// HDR-sampling bind group is still valid; a mismatch triggers
    /// a rebuild on the next frame.
    hdr_generation: u64,
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
        let instance = Instance::new(InstanceDescriptor {
            backends: Backends::PRIMARY,
            ..InstanceDescriptor::new_without_display_handle()
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
            .await?;

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
        //
        // `max_bind_groups` is raised from wgpu's spec-minimum default
        // of 4 to 5, accommodating the forward pipeline's per-material
        // texture group at @group(4) (camera + lights + model + shadow
        // + material). 5 is universally supported on every desktop GPU
        // shipped this decade; mobile Tegra/Mali sometimes report 4
        // as the floor, but Kinesis is desktop-only by design. Game 4
        // skeletal animation likely raises this to 6 for per-skinned-
        // instance bone matrices — we'll request what we actually
        // need each time rather than over-budget speculatively.
        let mut required_limits = Limits::default();
        required_limits.max_bind_groups = 5;
        let (device, queue) = adapter
            .request_device(&DeviceDescriptor {
                label: Some("schooner-device"),
                required_features: Features::empty(),
                required_limits,
                experimental_features: ExperimentalFeatures::default(),
                memory_hints: MemoryHints::Performance,
                trace: Trace::Off,
            })
            .await?;

        let size = window.inner_size();
        let config = build_surface_config(&adapter, &surface, size.width, size.height);
        surface.configure(&device, &config);

        let depth_view = create_depth_attachment(&device, config.width, config.height);
        let hdr_ms_view = create_hdr_ms_attachment(&device, config.width, config.height);
        let hdr_view = create_hdr_attachment(&device, config.width, config.height);

        info!(
            "render context ready: {}x{} {:?} ({}x MSAA)",
            config.width, config.height, config.format, MSAA_SAMPLE_COUNT
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
            hdr_ms_view,
            hdr_view,
            hdr_generation: 0,
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

    /// The multisampled HDR view the forward pass renders into. Bound
    /// as the forward color attachment with [`Self::hdr_view`] as its
    /// resolve target; never sampled directly.
    pub fn hdr_ms_view(&self) -> &TextureView {
        &self.hdr_ms_view
    }

    /// The single-sample HDR view the forward pass resolves into and
    /// the post pass samples from. See [`HDR_FORMAT`] for format choice.
    pub fn hdr_view(&self) -> &TextureView {
        &self.hdr_view
    }

    /// Generation counter on the HDR view; bumped each time the view
    /// is recreated (currently: surface resize). Bind groups that
    /// reference the HDR view should compare against this and rebuild
    /// when they observe a mismatch.
    pub fn hdr_generation(&self) -> u64 {
        self.hdr_generation
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
        self.hdr_ms_view = create_hdr_ms_attachment(&self.device, width, height);
        self.hdr_view = create_hdr_attachment(&self.device, width, height);
        self.hdr_generation = self.hdr_generation.wrapping_add(1);
        info!("surface reconfigured: {width}x{height}");
    }

    /// Acquire the next swap-chain texture. Returns `None` when the
    /// caller should skip this frame (zero-sized window, or a
    /// recoverable surface state we just reconfigured around).
    ///
    /// `Lost` / `Outdated` are routine on macOS during normal
    /// resizes — the right reaction is to reconfigure and try again
    /// next frame, not to crash. `Suboptimal` still hands back a
    /// usable texture; we present it and reconfigure for the next
    /// frame so we don't drop a visible frame on a transient mismatch.
    /// `Validation` errors arrive as a sentinel on this enum starting
    /// in wgpu 29 — out-of-memory now flows through the device-lost
    /// callback, not through this match.
    pub fn acquire_frame(&mut self) -> Option<SurfaceTexture> {
        if self.config.width == 0 || self.config.height == 0 {
            return None;
        }
        match self.surface.get_current_texture() {
            CurrentSurfaceTexture::Success(frame) => Some(frame),
            CurrentSurfaceTexture::Suboptimal(frame) => {
                warn!("surface suboptimal; reconfiguring for next frame");
                self.surface.configure(&self.device, &self.config);
                Some(frame)
            }
            CurrentSurfaceTexture::Outdated | CurrentSurfaceTexture::Lost => {
                warn!("surface lost/outdated; reconfiguring");
                self.surface.configure(&self.device, &self.config);
                None
            }
            CurrentSurfaceTexture::Timeout => {
                warn!("surface acquire timed out; skipping frame");
                None
            }
            CurrentSurfaceTexture::Occluded => None,
            CurrentSurfaceTexture::Validation => {
                warn!("surface validation error; skipping frame");
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
        // Multisampled to match the forward color attachment — a render
        // pass requires every attachment to share a sample count. The
        // depth buffer is consumed entirely within the forward pass
        // (nothing samples it afterward), so it is never resolved.
        sample_count: MSAA_SAMPLE_COUNT,
        dimension: TextureDimension::D2,
        format: DEPTH_FORMAT,
        usage: TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    texture.create_view(&TextureViewDescriptor::default())
}

/// The multisampled HDR color target the forward pass renders into.
///
/// `RENDER_ATTACHMENT` only — a multisampled texture cannot be bound as
/// a normal sampled texture, and it never needs to be: the pass resolves
/// it down into the single-sample [`create_hdr_attachment`] target, and
/// that resolved view is what the post chain samples.
fn create_hdr_ms_attachment(device: &Device, width: u32, height: u32) -> TextureView {
    let texture = device.create_texture(&TextureDescriptor {
        label: Some("schooner-hdr-ms"),
        size: Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: MSAA_SAMPLE_COUNT,
        dimension: TextureDimension::D2,
        format: HDR_FORMAT,
        usage: TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    texture.create_view(&TextureViewDescriptor::default())
}

fn create_hdr_attachment(device: &Device, width: u32, height: u32) -> TextureView {
    // `RENDER_ATTACHMENT` — forward pass writes; `TEXTURE_BINDING` —
    // post pass samples. The underlying Texture is owned by the
    // returned View (same pattern as the depth attachment), so the
    // GPU allocation lives exactly as long as we keep `hdr_view`.
    let texture = device.create_texture(&TextureDescriptor {
        label: Some("schooner-hdr"),
        size: Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format: HDR_FORMAT,
        usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    texture.create_view(&TextureViewDescriptor::default())
}
