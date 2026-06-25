//! Run-conditions — gate a system on a predicate over the world.
//!
//! `sys.run_if(cond)` wraps a system so it runs only when
//! `cond(&World)` is true. This replaces the early-return idiom
//! (`if *mode != Telekinesis { return; }`) with a declared gate: the
//! per-mode verb systems (Part 2.G) register once and the schedule
//! skips them when their mode is inactive.
//!
//! ## Why the conflict check is unaffected
//!
//! [`RunIf::param_access`] delegates to the inner system, so the
//! registration-time alias/conflict check sees the *same* access set
//! whether or not the system is gated. Gating changes when a system
//! runs, never what it's allowed to touch.
//!
//! ## Conditions are pure reads
//!
//! A condition is `Fn(&World) -> bool` — it only ever reads. The
//! generic [`resource_equals`] / [`resource_exists`] cover the common
//! cases; game-specific conditions (e.g. a mode check) compose from
//! `resource_equals` or are written game-side, since the engine
//! doesn't know game resource types.

use crate::ecs::World;
use crate::ecs::system::{IntoSystem, ParamAccess, System};

/// A system wrapped with a run-condition. Runs `inner` only on the
/// frames where `condition(&World)` returns true.
pub struct RunIf<S, C> {
    inner: S,
    condition: C,
}

impl<S, C> System for RunIf<S, C>
where
    S: System,
    C: Fn(&World) -> bool + 'static,
{
    fn run(&mut self, world: &mut World) {
        // Reborrow `&mut World` as `&World` for the pure-read
        // condition; the borrow ends before `inner.run`.
        if (self.condition)(&*world) {
            self.inner.run(world);
        }
    }

    fn param_access(&self, world: &mut World) -> Vec<ParamAccess> {
        // Delegate: the gate doesn't change what the system accesses,
        // so the conflict check must see the inner system's access.
        self.inner.param_access(world)
    }
}

/// `IntoSystem` marker for [`RunIf`] — a `RunIf` is already a system,
/// so `into_system` is the identity.
pub struct IsRunIf;

impl<S, C> IntoSystem<IsRunIf> for RunIf<S, C>
where
    S: System,
    C: Fn(&World) -> bool + 'static,
{
    type System = Self;
    fn into_system(self) -> Self::System {
        self
    }
}

/// Blanket extension adding `.run_if(..)` to anything that is an
/// [`IntoSystem`] — closures, fns, exclusive systems, even another
/// `RunIf` (conditions then compose with AND).
pub trait RunIfExt<Marker>: IntoSystem<Marker> + Sized {
    /// Gate this system on `condition`. The result is itself a system,
    /// so it goes straight into `add_system`.
    fn run_if<C>(self, condition: C) -> RunIf<Self::System, C>
    where
        C: Fn(&World) -> bool + 'static,
    {
        RunIf {
            inner: self.into_system(),
            condition,
        }
    }
}

impl<Marker, T: IntoSystem<Marker>> RunIfExt<Marker> for T {}

// --- generic conditions --------------------------------------------------

/// Condition: true when resource `R` is present and equals `value`.
///
/// The building block for mode gating —
/// `sys.run_if(resource_equals(ControlMode::Telekinesis))`.
pub fn resource_equals<R>(value: R) -> impl Fn(&World) -> bool
where
    R: PartialEq + Send + Sync + 'static,
{
    move |world| world.resource::<R>() == Some(&value)
}

/// Condition: true when resource `R` is present.
pub fn resource_exists<R: Send + Sync + 'static>() -> impl Fn(&World) -> bool {
    |world| world.contains_resource::<R>()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ecs::{Res, ResMut, Schedule, Stage};

    #[derive(Debug, PartialEq)]
    struct Counter(u32);

    #[derive(Debug, PartialEq, Clone, Copy)]
    enum Mode {
        On,
        Off,
    }

    #[test]
    fn system_runs_only_when_condition_true() {
        let mut world = World::new();
        world.insert_resource(Counter(0));
        world.insert_resource(Mode::Off);

        let mut sched = Schedule::new();
        sched.add_system(
            &mut world,
            Stage::Update,
            (|mut c: ResMut<Counter>| c.0 += 1).run_if(resource_equals(Mode::On)),
        );

        sched.run(&mut world); // Mode::Off → skipped
        assert_eq!(world.resource::<Counter>(), Some(&Counter(0)));

        *world.resource_mut::<Mode>().unwrap() = Mode::On;
        sched.run(&mut world); // Mode::On → runs
        assert_eq!(world.resource::<Counter>(), Some(&Counter(1)));
    }

    #[test]
    fn resource_exists_gates_on_presence() {
        let mut world = World::new();
        world.insert_resource(Counter(0));

        let mut sched = Schedule::new();
        sched.add_system(
            &mut world,
            Stage::Update,
            (|mut c: ResMut<Counter>| c.0 += 1).run_if(resource_exists::<Mode>()),
        );

        sched.run(&mut world); // no Mode → skipped
        assert_eq!(world.resource::<Counter>(), Some(&Counter(0)));

        world.insert_resource(Mode::On);
        sched.run(&mut world); // Mode present → runs
        assert_eq!(world.resource::<Counter>(), Some(&Counter(1)));
    }

    #[test]
    #[should_panic(expected = "more than once")]
    fn run_if_preserves_param_conflict_check() {
        // The gate delegates param_access, so a conflicting wrapped
        // system is still rejected at registration.
        let mut world = World::new();
        world.insert_resource(Counter(0));
        let mut sched = Schedule::new();
        sched.add_system(
            &mut world,
            Stage::Update,
            (|_a: Res<Counter>, _b: ResMut<Counter>| {}).run_if(resource_exists::<Mode>()),
        );
    }
}
