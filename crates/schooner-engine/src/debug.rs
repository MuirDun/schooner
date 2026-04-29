//! Debug overlay state + UI builder.
//!
//! `DebugState` is the world-side resource that drives what the
//! egui overlay shows. The overlay's *plumbing* (egui-winit /
//! egui-wgpu lifecycle, event forwarding, render pass encoding)
//! lives in `render::overlay`. This module owns:
//!
//! - The visibility/section toggles the player flips at runtime.
//! - The frame-stat ring buffer that smooths instantaneous deltas
//!   into a readable FPS / frame-ms readout.
//! - The system that reads the gameplay-side `Input` and updates
//!   visibility state (F1 toggles the overlay).
//! - The pure builder function `build_overlay_ui` that the renderer
//!   calls from inside `DebugOverlay::run`'s closure.
//! - The puffin sink + `FrameView` cache (`ProfilerView`) and the
//!   profiler-panel renderer.
//!
//! Keeping the UI builder pure (no resource lookups, no World
//! borrows) means `render_frame` can collect metrics with the world
//! borrow open, drop the borrow, then build the UI from the
//! collected snapshot — sidestepping the borrow tangle that a
//! UI-building system reading the world inline would create.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use egui::Context;
use glam::Vec3;
use puffin::{FrameSinkId, FrameView, GlobalProfiler, MergeScope, ScopeCollection};

use crate::ecs::{Res, ResMut};
use crate::input::{Input, KeyCode};

/// Number of frames the FPS / ms readout averages over.
///
/// 60 ≈ one second at 60 Hz — long enough that a single stutter
/// doesn't dominate, short enough that a real perf shift is
/// visible within a second.
pub const FRAME_STAT_WINDOW: usize = 60;

/// Rolling window of the most recent frame deltas in seconds.
///
/// Push-and-overwrite ring buffer; the head walks forward, wraps
/// at `FRAME_STAT_WINDOW`. Mean over the window gives the readout.
#[derive(Debug)]
pub struct FrameStats {
    samples: [f32; FRAME_STAT_WINDOW],
    head: usize,
    /// How many slots are populated. Saturates at `FRAME_STAT_WINDOW`
    /// after the first second; until then we average over fewer
    /// samples so the readout isn't dragged down by zero-filled
    /// slots in the first frames.
    filled: usize,
}

impl FrameStats {
    pub fn new() -> Self {
        Self {
            samples: [0.0; FRAME_STAT_WINDOW],
            head: 0,
            filled: 0,
        }
    }

    pub fn push(&mut self, delta_secs: f32) {
        // Defensive: a paused frame can produce delta=0; the FPS
        // readout would explode to infinity. Clamp to a small
        // positive so the mean stays finite.
        let dt = delta_secs.max(1e-6);
        self.samples[self.head] = dt;
        self.head = (self.head + 1) % FRAME_STAT_WINDOW;
        if self.filled < FRAME_STAT_WINDOW {
            self.filled += 1;
        }
    }

    /// Returns `(fps, frame_ms)`. Both `0.0` until the first push.
    pub fn averaged(&self) -> (f32, f32) {
        if self.filled == 0 {
            return (0.0, 0.0);
        }
        let sum: f32 = self.samples[..self.filled].iter().sum();
        let mean = sum / self.filled as f32;
        (1.0 / mean, mean * 1000.0)
    }
}

impl Default for FrameStats {
    fn default() -> Self {
        Self::new()
    }
}

/// Resource: what the debug overlay shows and how.
#[derive(Debug)]
pub struct DebugState {
    /// Master visibility — F1 toggles. When false, the overlay
    /// processes events to keep its egui state coherent but the
    /// renderer skips the encoded pass.
    pub overlay_visible: bool,
    /// Profiler panel visibility. Toggled by a button inside the
    /// overlay. The panel itself populates in chunk 6 with puffin.
    pub show_profiler: bool,
    pub frame_stats: FrameStats,
}

impl Default for DebugState {
    fn default() -> Self {
        Self {
            overlay_visible: true,
            show_profiler: false,
            frame_stats: FrameStats::new(),
        }
    }
}

/// System: read `Input::just_pressed(F1)` and toggle the overlay.
///
/// Lives on `Stage::Update` so the flip is visible to `render_frame`
/// in the same frame the key was pressed. Runs even when the
/// overlay is hidden so the player can re-summon it.
pub fn debug_input_system(input: Res<Input>, mut debug: ResMut<DebugState>) {
    if input.just_pressed(KeyCode::F1) {
        debug.overlay_visible = !debug.overlay_visible;
    }
}

/// Snapshot the renderer hands to [`build_overlay_ui`]. Owned
/// values only — no world borrows leak into the UI builder.
#[derive(Debug, Clone, Copy)]
pub struct OverlayMetrics {
    pub fps: f32,
    pub frame_ms: f32,
    pub entity_count: usize,
    pub camera_pos: Vec3,
}

/// Build the debug overlay window. Pure with respect to the world —
/// the renderer collects metrics into [`OverlayMetrics`] and the
/// interactive bits into [`OverlayInteract`], drives the closure,
/// then writes any user-flipped state back to [`DebugState`] after
/// the overlay's borrow ends. Keeping the renderer's `DebugState`
/// borrow short avoids overlapping with the `DebugOverlay`
/// `ResMut` it needs at the same time.
pub fn build_overlay_ui(
    ctx: &Context,
    interact: &mut OverlayInteract,
    metrics: OverlayMetrics,
    profiler_snapshot: Option<&ProfilerSnapshot>,
) {
    egui::Window::new("Debug")
        .default_open(true)
        .resizable(false)
        .show(ctx, |ui| {
            // Monospace numbers so the values don't visually jitter
            // frame-to-frame as digits change width.
            ui.monospace(format!("FPS:      {:>6.1}", metrics.fps));
            ui.monospace(format!("Frame:    {:>6.2} ms", metrics.frame_ms));
            ui.monospace(format!("Entities: {:>6}", metrics.entity_count));
            ui.monospace(format!(
                "Camera:   ({:>6.2}, {:>6.2}, {:>6.2})",
                metrics.camera_pos.x, metrics.camera_pos.y, metrics.camera_pos.z
            ));

            ui.separator();
            ui.checkbox(&mut interact.show_profiler, "Profiler");
            if interact.show_profiler {
                match profiler_snapshot {
                    Some(snapshot) => build_profiler_panel(ui, snapshot),
                    None => {
                        ui.label("(profiler view not initialised)");
                    }
                }
            }
            ui.label("F1 hides this overlay");
        });
}

/// User-toggleable bits the UI mutates this frame. The renderer
/// snapshots from [`DebugState`] before the overlay's mutable
/// borrow opens, hands `&mut` of this to the UI closure, and writes
/// the changed values back into [`DebugState`] after the borrow
/// closes. Add fields here as more interactive controls land.
#[derive(Debug, Clone, Copy)]
pub struct OverlayInteract {
    pub show_profiler: bool,
}

/// One row in the cached profiler readout. Owned strings so the
/// snapshot has no lifetime ties to the `FrameView` it was built
/// from — the renderer drops the `FrameView` lock the moment a
/// snapshot is built.
#[derive(Debug, Clone)]
pub struct ProfilerRow {
    pub name: String,
    pub depth: u8,
    pub calls: usize,
    pub ms_per_frame: f64,
    pub max_ms: f64,
}

/// Cached profiler readout. Built once per refresh interval from a
/// window of puffin frames; the UI re-renders this without touching
/// puffin again until the next refresh.
#[derive(Debug, Default, Clone)]
pub struct ProfilerSnapshot {
    pub frame_total_ms: f64,
    pub frames_in_window: usize,
    pub rows: Vec<ProfilerRow>,
}

/// Resource: shared `FrameView` of recent puffin frames + a cached
/// `ProfilerSnapshot` rebuilt at most twice per second.
///
/// Construction registers a sink with the global puffin profiler
/// that pushes each finalized frame into the inner
/// `Arc<Mutex<FrameView>>`. Drop deregisters the sink.
///
/// The overlay calls [`ProfilerView::refresh`] each frame; it
/// rebuilds [`ProfilerView::snapshot`] only when the refresh
/// interval has elapsed. Holding the readout still for ~500 ms
/// makes the digits readable and lets the snapshot average over a
/// window of frames (`merge_scopes_for_thread` aggregates natively
/// when given multiple frames).
pub struct ProfilerView {
    inner: Arc<Mutex<FrameView>>,
    sink_id: FrameSinkId,
    snapshot: Arc<ProfilerSnapshot>,
    last_refresh: Option<Instant>,
}

impl ProfilerView {
    /// How often to rebuild the cached snapshot. 500 ms ⇒ two
    /// updates per second — slow enough that digits are readable,
    /// fast enough that a sudden hot scope shows up within a beat.
    pub const REFRESH_INTERVAL: Duration = Duration::from_millis(500);

    /// How many recent frames to merge into one snapshot. ~50 covers
    /// 0.5 s at 100 fps; on a slower machine the same count covers
    /// proportionally more wall-clock — fine, the average is what
    /// we want either way.
    pub const WINDOW_FRAMES: usize = 50;

    pub fn new() -> Self {
        let inner: Arc<Mutex<FrameView>> = Arc::new(Mutex::new(FrameView::default()));
        let sink_inner = Arc::clone(&inner);
        let sink_id = GlobalProfiler::lock().add_sink(Box::new(move |frame| {
            // Best-effort: drop the frame on a poisoned mutex.
            // Profiler data is non-critical so panicking the sink
            // (which runs inside puffin's lock) would do more harm
            // than skipping a frame ever could.
            if let Ok(mut v) = sink_inner.lock() {
                v.add_frame(frame);
            }
        }));
        Self {
            inner,
            sink_id,
            snapshot: Arc::new(ProfilerSnapshot::default()),
            last_refresh: None,
        }
    }

    /// Cheap clone of the current snapshot Arc — hand it off to
    /// the UI builder so the overlay's borrow on `ProfilerView`
    /// can end before the egui closure runs.
    pub fn snapshot(&self) -> Arc<ProfilerSnapshot> {
        Arc::clone(&self.snapshot)
    }

    /// If at least [`Self::REFRESH_INTERVAL`] has passed since the
    /// last rebuild, lock the underlying `FrameView` and rebuild
    /// the cached snapshot from the last [`Self::WINDOW_FRAMES`]
    /// frames. No-op otherwise; idempotent in the same instant.
    pub fn refresh(&mut self) {
        let due = match self.last_refresh {
            Some(t) => t.elapsed() >= Self::REFRESH_INTERVAL,
            None => true,
        };
        if !due {
            return;
        }

        let snapshot = match self.inner.lock() {
            Ok(view) => build_snapshot(&view, Self::WINDOW_FRAMES),
            Err(_) => return,
        };
        self.snapshot = Arc::new(snapshot);
        self.last_refresh = Some(Instant::now());
    }
}

fn build_snapshot(view: &FrameView, window: usize) -> ProfilerSnapshot {
    use puffin::UnpackedFrameData;

    let frames: Vec<Arc<UnpackedFrameData>> = view
        .latest_frames(window)
        .filter_map(|f| f.unpacked().ok())
        .collect();
    if frames.is_empty() {
        return ProfilerSnapshot::default();
    }

    // Average the per-frame total across the window — matches the
    // merged per-scope `duration_per_frame_ns` semantics.
    let total_ns: f64 = frames
        .iter()
        .map(|f| (f.meta.range_ns.1 - f.meta.range_ns.0) as f64)
        .sum();
    let frame_total_ms = (total_ns / frames.len() as f64) / 1e6;

    let scope_collection = view.scope_collection();
    let mut rows = Vec::new();
    // Iterate the most recent frame's threads — every frame in the
    // window comes from the same process so the thread set is stable.
    for thread_info in frames[frames.len() - 1].thread_streams.keys() {
        if let Ok(mut merged) =
            puffin::merge_scopes_for_thread(scope_collection, &frames, thread_info)
        {
            sort_by_duration_desc(&mut merged);
            for scope in &merged {
                push_scope_recursive(&mut rows, scope_collection, scope, 0);
            }
        }
    }

    ProfilerSnapshot {
        frame_total_ms,
        frames_in_window: frames.len(),
        rows,
    }
}

fn push_scope_recursive(
    rows: &mut Vec<ProfilerRow>,
    scopes: &ScopeCollection,
    scope: &MergeScope,
    depth: u8,
) {
    let name = scopes
        .fetch_by_id(&scope.id)
        .map(|d| d.name().to_string())
        .unwrap_or_else(|| format!("scope#{:?}", scope.id));
    rows.push(ProfilerRow {
        name,
        depth,
        calls: scope.num_pieces,
        ms_per_frame: scope.duration_per_frame_ns as f64 / 1e6,
        max_ms: scope.max_duration_ns as f64 / 1e6,
    });
    let mut children: Vec<MergeScope> = scope.children.iter().cloned().collect();
    sort_by_duration_desc(&mut children);
    for child in &children {
        push_scope_recursive(rows, scopes, child, depth + 1);
    }
}

impl Default for ProfilerView {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for ProfilerView {
    fn drop(&mut self) {
        GlobalProfiler::lock().remove_sink(self.sink_id);
    }
}

/// Render the profiler panel into an existing `Ui`. Reads the
/// pre-built [`ProfilerSnapshot`] (refreshed at most every
/// `ProfilerView::REFRESH_INTERVAL`) into an indented table.
///
/// All averaging and tree flattening happen in
/// [`ProfilerView::refresh`]; this function is pure layout — it
/// can re-render the same snapshot many times per second without
/// allocating beyond what egui itself needs for text layout.
/// Indentation is leading spaces so columns stay aligned across
/// depths.
pub fn build_profiler_panel(ui: &mut egui::Ui, snapshot: &ProfilerSnapshot) {
    if snapshot.frames_in_window == 0 {
        ui.label("(no frames captured yet)");
        return;
    }

    ui.monospace(format!(
        "Frame total: {:>6.2} ms  (avg over {} frames, refresh {} ms)",
        snapshot.frame_total_ms,
        snapshot.frames_in_window,
        ProfilerView::REFRESH_INTERVAL.as_millis(),
    ));

    egui::Grid::new("schooner-profiler-scopes")
        .num_columns(4)
        .striped(true)
        .show(ui, |ui| {
            ui.monospace("scope");
            ui.monospace("calls");
            ui.monospace("ms/frame");
            ui.monospace("max ms");
            ui.end_row();

            for row in &snapshot.rows {
                let indent = "  ".repeat(row.depth as usize);
                ui.monospace(format!("{indent}{}", row.name));
                ui.monospace(format!("{:>4}", row.calls));
                ui.monospace(format!("{:>6.3}", row.ms_per_frame));
                ui.monospace(format!("{:>6.3}", row.max_ms));
                ui.end_row();
            }
        });
}

fn sort_by_duration_desc(scopes: &mut [MergeScope]) {
    scopes.sort_by(|a, b| b.duration_per_frame_ns.cmp(&a.duration_per_frame_ns));
}
