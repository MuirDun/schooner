//! Engine-side diagnostic systems.
//!
//! Throwaway-validation tools that surface loop health via the
//! `log` facade. Phase H replaces the FPS readout with an egui
//! overlay; this module remains as the no-window fallback and as
//! a home for any future "is the loop alive?" probes.

use log::info;

use crate::ecs::{Res, ResMut};
use crate::input::Input;
use crate::time::Time;

#[cfg(feature = "dev-tools")]
#[path = "diagnostics/debug.rs"]
pub mod debug;

#[cfg(feature = "dev-tools")]
pub use debug::{
    DiagnosticsDebugPlugin, DiagnosticsDebugState, FRAME_STAT_WINDOW, FrameStats, ProfilerRow,
    ProfilerSnapshot, ProfilerView,
};

/// One-second-window FPS state.
///
/// `accumulator` banks real frame time, `frames` counts the
/// frames that contributed. Once a full second has banked, the
/// pair is published and both fields reset. Holding state in a
/// resource (rather than a system-local) keeps the system pure
/// and lets the test exercise the math without a `World`.
///
/// The accumulator is `f64` rather than `f32` so 60+ sequential
/// adds of `1.0/60.0` reliably cross the 1.0 threshold —
/// `f32` accumulation drifts low enough to skip a publish
/// every ~few minutes of real run time.
#[derive(Debug, Default)]
pub struct FpsLogger {
    accumulator: f64,
    frames: u32,
}

impl FpsLogger {
    /// Record one frame worth of `delta_secs`. Returns
    /// `Some((fps, frame_ms))` once per accumulated second and
    /// resets internal state; otherwise returns `None`.
    pub fn record_frame(&mut self, delta_secs: f32) -> Option<(f32, f32)> {
        self.frames += 1;
        self.accumulator += delta_secs as f64;
        if self.accumulator < 1.0 {
            return None;
        }
        let fps = self.frames as f64 / self.accumulator;
        let frame_ms = 1000.0 * self.accumulator / self.frames as f64;
        // Reset both fields. A trailing fraction of a second is
        // dropped, which slightly under-counts in steady state —
        // negligible for a stdout heartbeat and avoids a skewed
        // first reading on the next window.
        self.accumulator = 0.0;
        self.frames = 0;
        Some((fps as f32, frame_ms as f32))
    }
}

/// `Update`-stage system: log FPS + frame time once per second.
///
/// Reads `Time::delta_secs` and a [`FpsLogger`] resource; the
/// app inserts the logger via [`App::with_fps_logging`](crate::App::with_fps_logging).
pub fn log_fps_system(time: Res<Time>, mut state: ResMut<FpsLogger>) {
    if let Some((fps, frame_ms)) = state.record_frame(time.delta_secs) {
        info!("fps {fps:.1} ({frame_ms:.2} ms/frame)");
    }
}

/// `Update`-stage system: log every keyboard / mouse-button edge
/// the frame produced. Throwaway smoke-test for Phase E; replaced
/// by the egui overlay in Phase H.
///
/// Mouse motion is intentionally not logged — it fires every frame
/// the user moves the cursor and would drown out edge events. Phase
/// G's FPS controller is the real test that mouse delta flows
/// through the pipeline.
pub fn log_input_system(input: Res<Input>) {
    for key in input.iter_just_pressed_keys() {
        info!("key pressed: {key:?}");
    }
    for key in input.iter_just_released_keys() {
        info!("key released: {key:?}");
    }
    for btn in input.iter_just_pressed_mouse_buttons() {
        info!("mouse pressed: {btn:?}");
    }
    for btn in input.iter_just_released_mouse_buttons() {
        info!("mouse released: {btn:?}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sub_second_accumulation_yields_none() {
        let mut log = FpsLogger::default();
        for _ in 0..30 {
            assert!(log.record_frame(0.016).is_none());
        }
    }

    #[test]
    fn one_second_of_60_fps_publishes_roughly_60_fps() {
        let mut log = FpsLogger::default();
        let mut readout = None;
        for _ in 0..60 {
            if let Some(r) = log.record_frame(1.0 / 60.0) {
                readout = Some(r);
            }
        }
        let (fps, frame_ms) = readout.expect("should have published once");
        assert!((fps - 60.0).abs() < 0.5, "fps was {fps}");
        assert!(
            (frame_ms - 1000.0 / 60.0).abs() < 0.05,
            "frame_ms was {frame_ms}"
        );
    }

    #[test]
    fn publishing_resets_internal_state() {
        let mut log = FpsLogger::default();
        for _ in 0..60 {
            log.record_frame(1.0 / 60.0);
        }
        // After publish, a single short frame must not immediately
        // re-fire — the accumulator and counter started over.
        assert!(log.record_frame(0.001).is_none());
    }

    #[test]
    fn two_seconds_publishes_twice() {
        let mut log = FpsLogger::default();
        let mut publishes = 0;
        for _ in 0..120 {
            if log.record_frame(1.0 / 60.0).is_some() {
                publishes += 1;
            }
        }
        assert_eq!(publishes, 2);
    }

    #[test]
    fn one_long_frame_can_publish_immediately() {
        // A pathological 2-second stall should still emit a single
        // reading rather than swallow it.
        let mut log = FpsLogger::default();
        let r = log.record_frame(2.0);
        assert!(r.is_some());
        let (fps, _) = r.unwrap();
        // 1 frame in 2 seconds → 0.5 fps.
        assert!((fps - 0.5).abs() < 1e-3);
    }

    #[test]
    fn zero_delta_frames_never_publish() {
        // A loop that calls record_frame with delta=0 (e.g. the
        // first tick) must not divide by zero or fire spuriously.
        let mut log = FpsLogger::default();
        for _ in 0..1000 {
            assert!(log.record_frame(0.0).is_none());
        }
    }
}
