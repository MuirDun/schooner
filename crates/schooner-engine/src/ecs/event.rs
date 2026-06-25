//! `Events<T>` — the double-buffered discrete-event queue.
//!
//! The Tier-2 channel for *instants with a payload* — a collision
//! happened, a trigger was entered — as opposed to *facts that
//! persist* (those are component/resource state, observed through the
//! change-tick ledger). The decision rule lives in
//! `plans/overview/events.md`: persists → state + change detection;
//! occurs → `Events<T>`.
//!
//! ## Poll, never subscribe
//!
//! A reader never registers a callback; it is a system that declares
//! `Res<Events<T>>` and drains the queue by polling `iter()`. That is
//! what lets a future Glyph VM be "one more polling system" over the
//! same buffer rather than a parallel event path.
//!
//! ## Double buffer
//!
//! Two buffers: `front` (this frame's sends) and `back` (last
//! frame's). Readers see both, so a reader that runs once per frame —
//! even a frame after the event was sent — never misses it.
//! [`update`](Events::update) runs once per frame at the top of
//! `App::tick` (registered via `App::add_event`): it drops the
//! two-frame-old set, slides this frame's into the readable window,
//! and opens a fresh buffer.

use crate::ecs::World;

/// Double-buffered queue of discrete events of type `T`.
///
/// Producers `send`; readers `iter` (draining by poll). The buffer is
/// swapped exactly once per frame so an event is readable for the
/// frame it is sent *and* the following frame.
#[derive(Debug)]
pub struct Events<T> {
    /// This frame's sends — the newest events.
    front: Vec<T>,
    /// Last frame's sends — still inside the readable window.
    back: Vec<T>,
}

impl<T> Default for Events<T> {
    fn default() -> Self {
        Self {
            front: Vec::new(),
            back: Vec::new(),
        }
    }
}

impl<T> Events<T> {
    pub fn new() -> Self {
        Self::default()
    }

    /// Queue an event. Visible to readers this frame (when ordered
    /// after this send) and the next frame.
    pub fn send(&mut self, event: T) {
        self.front.push(event);
    }

    /// Iterate every readable event, oldest first: last frame's events,
    /// then this frame's.
    pub fn iter(&self) -> impl Iterator<Item = &T> + '_ {
        self.back.iter().chain(self.front.iter())
    }

    /// Number of events currently readable across both buffers.
    pub fn len(&self) -> usize {
        self.front.len() + self.back.len()
    }

    pub fn is_empty(&self) -> bool {
        self.front.is_empty() && self.back.is_empty()
    }

    /// Per-frame swap: drop the two-frame-old buffer, slide this
    /// frame's sends into the readable window, and open a fresh buffer
    /// for the coming frame. Called once per frame at tick-top.
    pub fn update(&mut self) {
        std::mem::swap(&mut self.front, &mut self.back);
        // New front = old back (two frames old) → clear, keeping the
        // allocation. New back = last frame's sends, still readable.
        self.front.clear();
    }

    /// Drop all buffered events immediately. For explicit resets /
    /// tests; the per-frame lifecycle uses [`update`](Self::update).
    pub fn clear(&mut self) {
        self.front.clear();
        self.back.clear();
    }
}

/// Per-frame swap entry point for `Events<T>`, registered by
/// `App::add_event` and run once per frame at the top of `App::tick`.
/// A no-op when the resource is absent. Stored as a `fn(&mut World)`
/// pointer per event type — no boxing.
pub(crate) fn swap_events<T: Send + Sync + 'static>(world: &mut World) {
    if let Some(events) = world.resource_mut::<Events<T>>() {
        events.update();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq, Clone)]
    struct Ping(u32);

    fn drained(events: &Events<Ping>) -> Vec<u32> {
        events.iter().map(|p| p.0).collect()
    }

    #[test]
    fn send_is_readable_same_frame() {
        let mut e = Events::new();
        e.send(Ping(1));
        e.send(Ping(2));
        assert_eq!(drained(&e), vec![1, 2]);
        assert_eq!(e.len(), 2);
    }

    #[test]
    fn event_survives_exactly_one_swap() {
        let mut e = Events::new();
        e.send(Ping(1)); // sent frame 0
        e.update(); // frame 1 swap — 1 slides into the readable window
        assert_eq!(drained(&e), vec![1]); // still readable one frame late
        e.update(); // frame 2 swap — now dropped
        assert!(e.is_empty());
    }

    #[test]
    fn iter_orders_last_frame_before_this_frame() {
        let mut e = Events::new();
        e.send(Ping(1)); // frame 0
        e.update(); // 1 -> back
        e.send(Ping(2)); // frame 1 -> front
        assert_eq!(drained(&e), vec![1, 2]); // back (older) then front
    }

    #[test]
    fn swap_events_helper_is_noop_when_absent() {
        let mut world = World::new();
        // No Events<Ping> inserted — must not panic.
        swap_events::<Ping>(&mut world);
        assert!(world.resource::<Events<Ping>>().is_none());
    }

    #[test]
    fn swap_events_helper_advances_the_resource() {
        let mut world = World::new();
        world.insert_resource(Events::<Ping>::new());
        world.resource_mut::<Events<Ping>>().unwrap().send(Ping(7));
        swap_events::<Ping>(&mut world); // 7 -> back, still readable
        assert_eq!(drained(world.resource::<Events<Ping>>().unwrap()), vec![7]);
        swap_events::<Ping>(&mut world); // dropped
        assert!(world.resource::<Events<Ping>>().unwrap().is_empty());
    }
}
