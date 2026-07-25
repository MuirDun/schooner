//! Diagnostics-owned overlay contribution and puffin view.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use glam::Vec3;
use puffin::{FrameSinkId, FrameView, GlobalProfiler, MergeScope, ScopeCollection};

use crate::camera::ActiveCamera;
use crate::debug::DebugPanels;
use crate::debug::egui::{self, Context};
use crate::ecs::{Res, ResMut, World};
use crate::plugin::Plugin;
use crate::time::Time;
use crate::transform::Transform;
use crate::{App, Stage};

pub const FRAME_STAT_WINDOW: usize = 60;

#[derive(Debug)]
pub struct FrameStats {
    samples: [f32; FRAME_STAT_WINDOW],
    head: usize,
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
        self.samples[self.head] = delta_secs.max(1e-6);
        self.head = (self.head + 1) % FRAME_STAT_WINDOW;
        self.filled = (self.filled + 1).min(FRAME_STAT_WINDOW);
    }

    pub fn averaged(&self) -> (f32, f32) {
        if self.filled == 0 {
            return (0.0, 0.0);
        }
        let mean = self.samples[..self.filled].iter().sum::<f32>() / self.filled as f32;
        (1.0 / mean, mean * 1000.0)
    }
}

impl Default for FrameStats {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Default)]
pub struct DiagnosticsDebugState {
    pub show_profiler: bool,
    pub frame_stats: FrameStats,
}

fn update_diagnostics(time: Res<Time>, mut state: ResMut<DiagnosticsDebugState>) {
    state.frame_stats.push(time.delta_secs);
    puffin::set_scopes_on(state.show_profiler);
}

#[derive(Debug, Clone)]
pub struct ProfilerRow {
    pub name: String,
    pub depth: u8,
    pub calls: usize,
    pub ms_per_frame: f64,
    pub max_ms: f64,
}

#[derive(Debug, Default, Clone)]
pub struct ProfilerSnapshot {
    pub frame_total_ms: f64,
    pub frames_in_window: usize,
    pub rows: Vec<ProfilerRow>,
}

pub struct ProfilerView {
    inner: Arc<Mutex<FrameView>>,
    sink_id: FrameSinkId,
    snapshot: Arc<ProfilerSnapshot>,
    last_refresh: Option<Instant>,
}

impl ProfilerView {
    pub const REFRESH_INTERVAL: Duration = Duration::from_millis(500);
    pub const WINDOW_FRAMES: usize = 50;

    pub fn new() -> Self {
        let inner: Arc<Mutex<FrameView>> = Arc::new(Mutex::new(FrameView::default()));
        let sink_inner = Arc::clone(&inner);
        let sink_id = GlobalProfiler::lock().add_sink(Box::new(move |frame| {
            if let Ok(mut view) = sink_inner.lock() {
                view.add_frame(frame);
            }
        }));
        Self {
            inner,
            sink_id,
            snapshot: Arc::new(ProfilerSnapshot::default()),
            last_refresh: None,
        }
    }

    pub fn snapshot(&self) -> Arc<ProfilerSnapshot> {
        Arc::clone(&self.snapshot)
    }

    pub fn refresh(&mut self) {
        let due = self
            .last_refresh
            .is_none_or(|last| last.elapsed() >= Self::REFRESH_INTERVAL);
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

fn build_snapshot(view: &FrameView, window: usize) -> ProfilerSnapshot {
    use puffin::UnpackedFrameData;

    let frames: Vec<Arc<UnpackedFrameData>> = view
        .latest_frames(window)
        .filter_map(|frame| frame.unpacked().ok())
        .collect();
    if frames.is_empty() {
        return ProfilerSnapshot::default();
    }

    let total_ns: f64 = frames
        .iter()
        .map(|frame| (frame.meta.range_ns.1 - frame.meta.range_ns.0) as f64)
        .sum();
    let scope_collection = view.scope_collection();
    let mut rows = Vec::new();
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
        frame_total_ms: (total_ns / frames.len() as f64) / 1e6,
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
        .map(|data| data.name().to_string())
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

fn sort_by_duration_desc(scopes: &mut [MergeScope]) {
    scopes.sort_by(|a, b| b.duration_per_frame_ns.cmp(&a.duration_per_frame_ns));
}

fn build_profiler_panel(ui: &mut egui::Ui, snapshot: &ProfilerSnapshot) {
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
                ui.monospace(format!("{}{}", "  ".repeat(row.depth as usize), row.name));
                ui.monospace(format!("{:>4}", row.calls));
                ui.monospace(format!("{:>6.3}", row.ms_per_frame));
                ui.monospace(format!("{:>6.3}", row.max_ms));
                ui.end_row();
            }
        });
}

fn diagnostics_panel(world: &mut World, ctx: &Context) {
    let (fps, frame_ms, mut show_profiler) = world
        .resource::<DiagnosticsDebugState>()
        .map(|state| {
            let (fps, frame_ms) = state.frame_stats.averaged();
            (fps, frame_ms, state.show_profiler)
        })
        .unwrap_or((0.0, 0.0, false));
    let camera_pos = world
        .query::<(&Transform, &ActiveCamera)>()
        .into_iter()
        .next()
        .map(|(transform, _)| transform.translation)
        .unwrap_or(Vec3::ZERO);
    let entity_count = world.entity_count();
    let snapshot = if show_profiler {
        world.resource_mut::<ProfilerView>().map(|profiler| {
            profiler.refresh();
            profiler.snapshot()
        })
    } else {
        None
    };

    egui::Window::new("Diagnostics")
        .default_open(true)
        .resizable(false)
        .show(ctx, |ui| {
            ui.monospace(format!("FPS:      {:>6.1}", fps));
            ui.monospace(format!("Frame:    {:>6.2} ms", frame_ms));
            ui.monospace(format!("Entities: {:>6}", entity_count));
            ui.monospace(format!(
                "Camera:   ({:>6.2}, {:>6.2}, {:>6.2})",
                camera_pos.x, camera_pos.y, camera_pos.z
            ));
            ui.separator();
            ui.checkbox(&mut show_profiler, "Profiler");
            if show_profiler {
                match snapshot.as_deref() {
                    Some(snapshot) => build_profiler_panel(ui, snapshot),
                    None => {
                        ui.label("(profiler view not initialised)");
                    }
                }
            }
            ui.label("F12 hides the debug overlay");
        });

    if let Some(state) = world.resource_mut::<DiagnosticsDebugState>() {
        state.show_profiler = show_profiler;
        puffin::set_scopes_on(show_profiler);
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct DiagnosticsDebugPlugin;

impl Plugin for DiagnosticsDebugPlugin {
    fn build(&self, app: App) -> App {
        let mut app = app
            .insert_resource(DiagnosticsDebugState::default())
            .insert_resource(ProfilerView::new())
            .add_system(Stage::Update, update_diagnostics);
        if let Some(panels) = app.world_mut().resource_mut::<DebugPanels>() {
            panels.register("diagnostics", diagnostics_panel);
        }
        app
    }
}
