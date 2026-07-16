//! Ordered system execution for the engine stages.
//!
//! The engine runs systems in exactly three stages, named by [`Stage`]:
//! - [`Stage::FixedUpdate`] — fixed timestep, 0..N times per frame,
//!   driven by an accumulator in the App loop. This is the pre-physics
//!   intent stage: deterministic gameplay writes forces and desired poses here.
//! - [`Stage::PostPhysics`] — fixed timestep, immediately after the engine's
//!   physics bridge. Deterministic gameplay reads settled poses and contacts here.
//! - [`Stage::Update`] — variable timestep, once per frame. Input,
//!   camera, gameplay logic. `delta` scales with real frame time.
//! - [`Stage::Render`] — once per frame, after `Update`. Owns the
//!   forward pass, the egui overlay, and any future post-process
//!   work. Systems here are expected to read the post-`Update`
//!   snapshot of the world; the engine appends `render_frame` to
//!   this stage from `App::resumed` after the user's builder chain.
//!
//! The stage set is a closed enum. Game-side custom stages would
//! likely live in the scripting layer, not the Rust core — so the
//! enum cannot be extended by downstream crates.
//!
//! ## Tick semantics
//!
//! [`Schedule::run_fixed`], [`Schedule::run`], and
//! [`Schedule::run_render`] each bump `world.current_tick` exactly
//! once before dispatching their stage. A frame with three
//! `FixedUpdate` steps therefore advances `current_tick` by 5
//! (3 fixed + Update + Render). The uniform rule "every stage run
//! = one tick" keeps change-detection comparisons across stages
//! straightforward; if the reactive cascade engine in Game 2 wants
//! a different convention, that's the place to revisit it.
//!
//! ## Scope (Game 0)
//!
//! - Registration is add-only; no removal, no reordering, no
//!   dependency graph — systems run in the order they were added.
//! - Single-threaded; parallel scheduling is deferred (likely
//!   Game 4 per `plans/game0-plan.md` §1.1).
//! - Systems return `()`; a panicking system aborts the stage.
//!   Fallible systems are out of scope for Game 0.

use crate::ecs::command::CommandQueue;
use crate::ecs::system::{IntoSystem, System, check_param_conflicts};
use crate::ecs::world::World;

/// Closed set of stages the engine runs systems in.
///
/// Closed by design: gameplay-side stages belong in the scripting
/// layer, not in the Rust scheduler. Exhaustive matching on this
/// enum keeps stage-handling code honest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Stage {
    Startup,
    /// Fixed-step intent writers. Kept under its original name for source
    /// compatibility; it is the pre-physics half of the fixed schedule.
    FixedUpdate,
    /// Fixed-step outcome readers, after the engine physics bridge.
    PostPhysics,
    Update,
    Render,
}

/// Ordered system list for a single [`Stage`].
struct SystemStage {
    systems: Vec<Box<dyn System>>,
}

impl SystemStage {
    fn new() -> Self {
        Self {
            systems: Vec::new(),
        }
    }

    fn push(&mut self, system: Box<dyn System>) {
        self.systems.push(system);
    }

    fn run(&mut self, world: &mut World) {
        for sys in &mut self.systems {
            sys.run(world);
        }
    }

    fn len(&self) -> usize {
        self.systems.len()
    }
}

/// Holds the two fixed stages and runs them against a [`World`].
///
/// Add systems once during App setup via
/// [`Schedule::add_system`]; run them each frame via [`Schedule::run`]
/// (variable tick) and [`Schedule::run_fixed`] (driven by the
/// fixed-timestep accumulator).
pub struct Schedule {
    fixed_update: SystemStage,
    physics: SystemStage,
    post_physics: SystemStage,
    update: SystemStage,
    render: SystemStage,
    startup: SystemStage,
}

impl Default for Schedule {
    fn default() -> Self {
        Self::new()
    }
}

impl Schedule {
    pub fn new() -> Self {
        Self {
            fixed_update: SystemStage::new(),
            physics: SystemStage::new(),
            post_physics: SystemStage::new(),
            update: SystemStage::new(),
            render: SystemStage::new(),
            startup: SystemStage::new(),
        }
    }

    /// Register a system in the given stage. Accepts plain fns and
    /// closures via [`IntoSystem`] — no `.into_system()` at the call
    /// site.
    ///
    /// Takes `&mut World` so the system's params can be initialised
    /// (auto-registering any component types) and validated for
    /// conflicts at registration. A conflicting system panics here,
    /// not on the first frame.
    pub fn add_system<M, S: IntoSystem<M>>(
        &mut self,
        world: &mut World,
        stage: Stage,
        system: S,
    ) -> &mut Self {
        let system = system.into_system();
        let accesses = system.param_access(world);
        if let Err(err) = check_param_conflicts(&accesses, |c| {
            world
                .component_name(c.component_id)
                .unwrap_or("<unregistered>")
        }) {
            panic!("{err}");
        }
        let boxed: Box<dyn System> = Box::new(system);
        match stage {
            Stage::FixedUpdate => self.fixed_update.push(boxed),
            Stage::PostPhysics => self.post_physics.push(boxed),
            Stage::Update => self.update.push(boxed),
            Stage::Render => self.render.push(boxed),
            Stage::Startup => self.startup.push(boxed),
        }
        self
    }

    /// Register an engine-owned bridge between fixed-step intent and outcome.
    /// Kept crate-private so games cannot accidentally interleave systems with
    /// the membrane; use [`Stage::FixedUpdate`] or [`Stage::PostPhysics`] instead.
    pub(crate) fn add_physics_system<M, S: IntoSystem<M>>(
        &mut self,
        world: &mut World,
        system: S,
    ) -> &mut Self {
        let system = system.into_system();
        let accesses = system.param_access(world);
        if let Err(err) = check_param_conflicts(&accesses, |c| {
            world
                .component_name(c.component_id)
                .unwrap_or("<unregistered>")
        }) {
            panic!("{err}");
        }
        self.physics.push(Box::new(system));
        self
    }

    /// Advance `world.current_tick` by one, then run every
    /// [`Stage::Update`] system in registration order.
    pub fn run(&mut self, world: &mut World) {
        puffin::profile_scope!("update_stage");
        world.increment_tick();
        self.update.run(world);
        CommandQueue::apply(world);
    }

    /// Advance `world.current_tick` by one, then run every
    /// [`Stage::FixedUpdate`] intent systems, then the engine physics bridge,
    /// then [`Stage::PostPhysics`] outcome systems. Commands flush at every
    /// boundary so fixed-step spawns and despawns are visible to physics.
    /// Called 0..N times per frame by the App's accumulator.
    pub fn run_fixed(&mut self, world: &mut World) {
        puffin::profile_scope!("fixed_stage");
        world.increment_tick();
        self.fixed_update.run(world);
        CommandQueue::apply(world);
        self.physics.run(world);
        CommandQueue::apply(world);
        self.post_physics.run(world);
        CommandQueue::apply(world);
    }

    /// Advance `world.current_tick` by one, then run every
    /// [`Stage::Render`] system in registration order. Called once
    /// per frame after [`Schedule::run`]; this is where the forward
    /// pass and egui overlay live.
    pub fn run_render(&mut self, world: &mut World) {
        puffin::profile_scope!("render_stage");
        world.increment_tick();
        self.render.run(world);
        CommandQueue::apply(world);
    }

    /// Advance `world.current_tick` once.
    /// Called exactly once, from App:resumed.
    pub fn run_startup(&mut self, world: &mut World) {
        puffin::profile_scope!("startup_stage");
        world.increment_tick();
        self.startup.run(world);
        CommandQueue::apply(world);
    }

    pub fn system_count(&self, stage: Stage) -> usize {
        match stage {
            Stage::FixedUpdate => self.fixed_update.len(),
            Stage::PostPhysics => self.post_physics.len(),
            Stage::Update => self.update.len(),
            Stage::Render => self.render.len(),
            Stage::Startup => self.startup.len(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ecs::{Res, ResMut};

    #[derive(Debug, PartialEq)]
    struct Counter(u32);

    #[derive(Debug, PartialEq)]
    struct Log(Vec<&'static str>);

    #[test]
    fn empty_schedule_runs_without_panic() {
        let mut world = World::new();
        let mut sched = Schedule::new();
        sched.run_fixed(&mut world);
        sched.run(&mut world);
        sched.run_render(&mut world);
    }

    #[test]
    fn run_bumps_current_tick_exactly_once() {
        let mut world = World::new();
        let before = world.current_tick;
        let mut sched = Schedule::new();
        sched.run(&mut world);
        assert_eq!(world.current_tick, before + 1);
    }

    #[test]
    fn run_fixed_bumps_current_tick_exactly_once() {
        let mut world = World::new();
        let before = world.current_tick;
        let mut sched = Schedule::new();
        sched.run_fixed(&mut world);
        assert_eq!(world.current_tick, before + 1);
    }

    #[test]
    fn run_render_bumps_current_tick_exactly_once() {
        let mut world = World::new();
        let before = world.current_tick;
        let mut sched = Schedule::new();
        sched.run_render(&mut world);
        assert_eq!(world.current_tick, before + 1);
    }

    #[test]
    fn frame_with_three_fixed_steps_advances_tick_by_five() {
        let mut world = World::new();
        let before = world.current_tick;
        let mut sched = Schedule::new();
        sched.run_fixed(&mut world);
        sched.run_fixed(&mut world);
        sched.run_fixed(&mut world);
        sched.run(&mut world);
        sched.run_render(&mut world);
        assert_eq!(world.current_tick, before + 5);
    }

    #[test]
    fn update_systems_run_in_registration_order() {
        let mut world = World::new();
        world.insert_resource(Log(Vec::new()));
        let mut sched = Schedule::new();
        sched
            .add_system(&mut world, Stage::Update, |mut l: ResMut<Log>| {
                l.0.push("a")
            })
            .add_system(&mut world, Stage::Update, |mut l: ResMut<Log>| {
                l.0.push("b")
            })
            .add_system(&mut world, Stage::Update, |mut l: ResMut<Log>| {
                l.0.push("c")
            });
        sched.run(&mut world);
        assert_eq!(world.resource::<Log>().unwrap().0, vec!["a", "b", "c"]);
    }

    #[test]
    fn stages_are_isolated() {
        let mut world = World::new();
        world.insert_resource(Log(Vec::new()));
        let mut sched = Schedule::new();
        sched
            .add_system(&mut world, Stage::Update, |mut l: ResMut<Log>| {
                l.0.push("upd")
            })
            .add_system(&mut world, Stage::FixedUpdate, |mut l: ResMut<Log>| {
                l.0.push("fix")
            });
        sched.run(&mut world);
        assert_eq!(world.resource::<Log>().unwrap().0, vec!["upd"]);
        sched.run_fixed(&mut world);
        assert_eq!(world.resource::<Log>().unwrap().0, vec!["upd", "fix"]);
    }

    #[test]
    fn system_a_mutation_is_visible_to_system_b() {
        let mut world = World::new();
        world.insert_resource(Counter(0));
        let mut sched = Schedule::new();
        sched
            .add_system(&mut world, Stage::Update, |mut c: ResMut<Counter>| c.0 += 1)
            .add_system(&mut world, Stage::Update, |c: Res<Counter>| {
                assert_eq!(c.0, 1)
            });
        sched.run(&mut world);
    }

    #[test]
    fn system_count_reflects_registrations() {
        let mut world = World::new();
        let mut sched = Schedule::new();
        sched
            .add_system(&mut world, Stage::Update, || {})
            .add_system(&mut world, Stage::Update, || {})
            .add_system(&mut world, Stage::FixedUpdate, || {});
        assert_eq!(sched.system_count(Stage::Update), 2);
        assert_eq!(sched.system_count(Stage::FixedUpdate), 1);
    }

    #[test]
    fn multiple_runs_keep_running_each_system() {
        let mut world = World::new();
        world.insert_resource(Counter(0));
        let mut sched = Schedule::new();
        sched.add_system(&mut world, Stage::Update, |mut c: ResMut<Counter>| c.0 += 1);
        sched.run(&mut world);
        sched.run(&mut world);
        sched.run(&mut world);
        assert_eq!(world.resource::<Counter>(), Some(&Counter(3)));
    }

    #[test]
    fn fixed_update_steps_accumulate_mutations() {
        let mut world = World::new();
        world.insert_resource(Counter(0));
        let mut sched = Schedule::new();
        sched.add_system(&mut world, Stage::FixedUpdate, |mut c: ResMut<Counter>| {
            c.0 += 10
        });
        sched.run_fixed(&mut world);
        sched.run_fixed(&mut world);
        assert_eq!(world.resource::<Counter>(), Some(&Counter(20)));
    }

    #[test]
    fn fixed_step_runs_intent_physics_then_outcome() {
        let mut world = World::new();
        world.insert_resource(Log(Vec::new()));
        let mut sched = Schedule::new();
        sched
            .add_system(&mut world, Stage::FixedUpdate, |mut log: ResMut<Log>| {
                log.0.push("intent")
            })
            .add_physics_system(&mut world, |mut log: ResMut<Log>| log.0.push("physics"))
            .add_system(&mut world, Stage::PostPhysics, |mut log: ResMut<Log>| {
                log.0.push("outcome")
            });

        sched.run_fixed(&mut world);

        assert_eq!(
            world.resource::<Log>().unwrap().0,
            vec!["intent", "physics", "outcome"]
        );
    }

    #[test]
    fn changed_since_sees_entity_mutated_by_scheduled_system() {
        use crate::ecs::exclusive;

        #[derive(Debug, PartialEq)]
        struct Health(i32);

        let mut world = World::new();
        let e = world.spawn();
        world.insert(e, Health(100));
        let baseline = world.current_tick;

        let mut sched = Schedule::new();
        sched.add_system(
            &mut world,
            Stage::Update,
            exclusive(move |w: &mut World| {
                let mut h = w.get_mut::<Health>(e).unwrap();
                h.0 -= 10;
            }),
        );
        sched.run(&mut world);

        let changed: Vec<_> = world
            .changed_since::<Health>(baseline)
            .map(|(id, _)| id)
            .collect();
        assert_eq!(changed, vec![e]);
    }

    #[test]
    fn add_system_returns_builder_for_chaining() {
        let mut world = World::new();
        let mut sched = Schedule::new();
        let _: &mut Schedule = sched
            .add_system(&mut world, Stage::Update, || {})
            .add_system(&mut world, Stage::FixedUpdate, || {});
    }

    #[test]
    fn query_as_system_param_iterates_components() {
        use crate::ecs::Query;

        #[derive(Debug, PartialEq)]
        struct Pos(i32);

        let mut world = World::new();
        let a = world.spawn();
        let b = world.spawn();
        world.insert(a, Pos(1));
        world.insert(b, Pos(2));
        world.insert_resource(Counter(0));

        let mut sched = Schedule::new();
        sched.add_system(
            &mut world,
            Stage::Update,
            |mut total: ResMut<Counter>, q: Query<&Pos>| {
                for p in q {
                    total.0 += p.0 as u32;
                }
            },
        );
        sched.run(&mut world);
        assert_eq!(world.resource::<Counter>(), Some(&Counter(3)));
    }

    #[test]
    fn query_mut_as_system_param_writes_through() {
        use crate::ecs::{Query, WriteOnly};

        #[derive(Debug, PartialEq)]
        struct Pos(i32);

        let mut world = World::new();
        let a = world.spawn();
        let b = world.spawn();
        world.insert(a, Pos(1));
        world.insert(b, Pos(2));

        let mut sched = Schedule::new();
        sched.add_system(&mut world, Stage::Update, |q: Query<WriteOnly<Pos>>| {
            for mut p in q {
                p.0 *= 10;
            }
        });
        sched.run(&mut world);
        assert_eq!(world.get::<Pos>(a), Some(&Pos(10)));
        assert_eq!(world.get::<Pos>(b), Some(&Pos(20)));
    }

    #[test]
    #[should_panic(expected = "alias conflict")]
    fn registration_rejects_aliasing_queries_in_one_system() {
        use crate::ecs::{Query, WriteOnly};

        #[derive(Debug, PartialEq)]
        struct Pos(i32);

        let mut world = World::new();
        let e = world.spawn();
        world.insert(e, Pos(0));

        let mut sched = Schedule::new();
        // Two queries on the same component, one mutable — alias
        // conflict caught at registration, not first run.
        sched.add_system(
            &mut world,
            Stage::Update,
            |_a: Query<&Pos>, _b: Query<WriteOnly<Pos>>| {},
        );
    }

    #[test]
    fn closed_stage_set_is_exhaustive() {
        // Compile-time guard: if a new Stage variant is added, this
        // match forces an update here (and everywhere that dispatches
        // on Stage). Keeps the closed-set invariant honest.
        fn _exhaustive(s: Stage) -> &'static str {
            match s {
                Stage::FixedUpdate => "fixed",
                Stage::PostPhysics => "post_physics",
                Stage::Update => "update",
                Stage::Render => "render",
                Stage::Startup => "startup",
            }
        }
        assert_eq!(_exhaustive(Stage::FixedUpdate), "fixed");
        assert_eq!(_exhaustive(Stage::PostPhysics), "post_physics");
        assert_eq!(_exhaustive(Stage::Update), "update");
        assert_eq!(_exhaustive(Stage::Render), "render");
    }
}
