//! Ordered system execution for the engine stages.
//!
//! The engine runs systems in a closed set of stages named by [`Stage`]:
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
//! `world.current_tick` is a change-detection epoch, not a stage, frame,
//! or simulation counter. Every scheduled system dispatch receives a
//! distinct epoch. Every non-empty deferred-command batch receives one
//! more; empty stages and command barriers consume none. This lets a
//! reactive consumer distinguish producers on either side of it within
//! one stage.
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
            world.increment_tick();
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

    /// Run every [`Stage::Update`] system in registration order, assigning
    /// each execution its own change epoch.
    pub fn run(&mut self, world: &mut World) {
        puffin::profile_scope!("update_stage");
        self.update.run(world);
        CommandQueue::apply(world);
    }

    /// Run every [`Stage::FixedUpdate`] intent system, then the engine physics
    /// bridge, then [`Stage::PostPhysics`] outcome systems. Each execution has
    /// its own change epoch. Commands flush at every boundary so fixed-step
    /// spawns and despawns are visible to physics; each non-empty batch gets
    /// another epoch. Called 0..N times per frame by the App's accumulator.
    pub fn run_fixed(&mut self, world: &mut World) {
        puffin::profile_scope!("fixed_stage");
        self.fixed_update.run(world);
        CommandQueue::apply(world);
        self.physics.run(world);
        CommandQueue::apply(world);
        self.post_physics.run(world);
        CommandQueue::apply(world);
    }

    /// Run every [`Stage::Render`] system in registration order, assigning
    /// each execution its own change epoch. Called once per frame after
    /// [`Schedule::run`]; this is where the forward pass and egui overlay live.
    pub fn run_render(&mut self, world: &mut World) {
        puffin::profile_scope!("render_stage");
        self.render.run(world);
        CommandQueue::apply(world);
    }

    /// Run startup systems with one change epoch per execution.
    /// Called exactly once, from `App::resumed`.
    pub fn run_startup(&mut self, world: &mut World) {
        puffin::profile_scope!("startup_stage");
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
    use crate::ecs::{
        Added, Changed, CommandQueue, Commands, Query, Res, ResMut, RunIfExt, WriteOnly, exclusive,
    };

    #[derive(Debug, PartialEq)]
    struct Counter(u32);

    #[derive(Debug, PartialEq)]
    struct Log(Vec<&'static str>);

    #[derive(Debug, PartialEq)]
    struct ReactiveValue(i32);

    #[derive(Debug, Default, PartialEq)]
    struct SeenValues(Vec<i32>);

    #[test]
    fn empty_schedule_runs_without_panic() {
        let mut world = World::new();
        let mut sched = Schedule::new();
        sched.run_fixed(&mut world);
        sched.run(&mut world);
        sched.run_render(&mut world);
    }

    #[test]
    fn empty_stages_and_command_barriers_consume_no_epochs() {
        let mut world = World::new();
        world.insert_resource(CommandQueue::default());
        let before = world.current_tick();
        let mut sched = Schedule::new();
        sched.run_fixed(&mut world);
        sched.run(&mut world);
        sched.run_render(&mut world);
        sched.run_startup(&mut world);
        assert_eq!(world.current_tick(), before);
    }

    #[test]
    fn each_scheduled_system_execution_gets_a_distinct_epoch() {
        let mut world = World::new();
        let before = world.current_tick();
        let mut sched = Schedule::new();
        sched
            .add_system(&mut world, Stage::Update, || {})
            .add_system(&mut world, Stage::Update, || {});
        sched.run(&mut world);
        assert_eq!(world.current_tick(), before + 2);
    }

    #[test]
    fn one_system_in_each_frame_stage_advances_by_each_execution() {
        let mut world = World::new();
        let before = world.current_tick();
        let mut sched = Schedule::new();
        sched
            .add_system(&mut world, Stage::FixedUpdate, || {})
            .add_system(&mut world, Stage::Update, || {})
            .add_system(&mut world, Stage::Render, || {});
        sched.run_fixed(&mut world);
        sched.run_fixed(&mut world);
        sched.run_fixed(&mut world);
        sched.run(&mut world);
        sched.run_render(&mut world);
        assert_eq!(world.current_tick(), before + 5);
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
        #[derive(Debug, PartialEq)]
        struct Health(i32);

        let mut world = World::new();
        let e = world.spawn();
        world.insert(e, Health(100));
        let baseline = world.current_tick();

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
    fn scheduled_added_and_changed_include_tick_zero_once() {
        #[derive(Debug, Default, PartialEq)]
        struct ReactiveRuns {
            added: Vec<usize>,
            changed: Vec<usize>,
        }

        let mut world = World::new();
        let entity = world.spawn();
        world.insert(entity, ReactiveValue(7));
        world.insert_resource(ReactiveRuns::default());

        let mut sched = Schedule::new();
        sched.add_system(
            &mut world,
            Stage::Update,
            |mut runs: ResMut<ReactiveRuns>,
             added: Query<&ReactiveValue, Added<ReactiveValue>>,
             changed: Query<&ReactiveValue, Changed<ReactiveValue>>| {
                runs.added.push(added.into_iter().count());
                runs.changed.push(changed.into_iter().count());
            },
        );

        sched.run(&mut world);
        sched.run(&mut world);

        assert_eq!(
            world.resource::<ReactiveRuns>(),
            Some(&ReactiveRuns {
                added: vec![1, 0],
                changed: vec![1, 0],
            })
        );
    }

    #[test]
    fn producer_before_consumer_is_seen_on_every_run() {
        let mut world = World::new();
        let entity = world.spawn();
        world.insert(entity, ReactiveValue(0));
        world.insert_resource(SeenValues::default());

        let mut sched = Schedule::new();
        sched
            .add_system(
                &mut world,
                Stage::Update,
                |values: Query<WriteOnly<ReactiveValue>>| {
                    for mut value in values {
                        value.0 += 1;
                    }
                },
            )
            .add_system(
                &mut world,
                Stage::Update,
                |mut seen: ResMut<SeenValues>,
                 values: Query<&ReactiveValue, Changed<ReactiveValue>>| {
                    seen.0.extend(values.into_iter().map(|value| value.0));
                },
            );

        sched.run(&mut world);
        sched.run(&mut world);

        assert_eq!(
            world.resource::<SeenValues>(),
            Some(&SeenValues(vec![1, 2]))
        );
    }

    #[test]
    fn producer_after_consumer_is_seen_on_the_next_run() {
        let mut world = World::new();
        let entity = world.spawn();
        world.insert(entity, ReactiveValue(0));
        world.insert_resource(SeenValues::default());

        let mut sched = Schedule::new();
        sched
            .add_system(
                &mut world,
                Stage::Update,
                |mut seen: ResMut<SeenValues>,
                 values: Query<&ReactiveValue, Changed<ReactiveValue>>| {
                    seen.0.extend(values.into_iter().map(|value| value.0));
                },
            )
            .add_system(
                &mut world,
                Stage::Update,
                |values: Query<WriteOnly<ReactiveValue>>| {
                    for mut value in values {
                        value.0 += 1;
                    }
                },
            );

        sched.run(&mut world);
        sched.run(&mut world);
        sched.run(&mut world);

        // This is the original same-stage loss shape: the producer runs
        // after the consumer. Each later mutation must survive until the
        // consumer's next execution.
        assert_eq!(
            world.resource::<SeenValues>(),
            Some(&SeenValues(vec![0, 1, 2]))
        );
    }

    #[test]
    fn non_empty_command_batch_before_consumer_has_its_own_epoch() {
        let mut world = World::new();
        world.insert_resource(CommandQueue::default());
        world.insert_resource(SeenValues::default());
        let entity = world.spawn();
        world.insert(entity, ReactiveValue(0));

        let mut sched = Schedule::new();
        sched
            .add_system(
                &mut world,
                Stage::FixedUpdate,
                move |mut commands: Commands| {
                    commands.queue(move |world| {
                        world.get_mut::<ReactiveValue>(entity).unwrap().0 += 1;
                    });
                },
            )
            .add_system(
                &mut world,
                Stage::PostPhysics,
                |mut seen: ResMut<SeenValues>,
                 values: Query<&ReactiveValue, Changed<ReactiveValue>>| {
                    seen.0.extend(values.into_iter().map(|value| value.0));
                },
            );

        sched.run_fixed(&mut world);
        sched.run_fixed(&mut world);

        assert_eq!(
            world.resource::<SeenValues>(),
            Some(&SeenValues(vec![1, 2]))
        );
        assert_eq!(world.current_tick(), 6);
    }

    #[test]
    fn non_empty_command_batch_after_consumer_is_seen_next_run() {
        let mut world = World::new();
        world.insert_resource(CommandQueue::default());
        world.insert_resource(SeenValues::default());
        let entity = world.spawn();
        world.insert(entity, ReactiveValue(0));

        let mut sched = Schedule::new();
        sched
            .add_system(
                &mut world,
                Stage::Update,
                |mut seen: ResMut<SeenValues>,
                 values: Query<&ReactiveValue, Changed<ReactiveValue>>| {
                    seen.0.extend(values.into_iter().map(|value| value.0));
                },
            )
            .add_system(&mut world, Stage::Update, move |mut commands: Commands| {
                commands.queue(move |world| {
                    world.get_mut::<ReactiveValue>(entity).unwrap().0 += 1;
                });
            });

        sched.run(&mut world);
        sched.run(&mut world);
        sched.run(&mut world);

        assert_eq!(
            world.resource::<SeenValues>(),
            Some(&SeenValues(vec![0, 1, 2]))
        );
        assert_eq!(world.current_tick(), 9);
    }

    #[test]
    fn independent_consumers_each_observe_the_same_changes() {
        #[derive(Debug, Default, PartialEq)]
        struct SeenA(Vec<i32>);
        #[derive(Debug, Default, PartialEq)]
        struct SeenB(Vec<i32>);

        let mut world = World::new();
        let entity = world.spawn();
        world.insert(entity, ReactiveValue(0));
        world.insert_resource(SeenA::default());
        world.insert_resource(SeenB::default());

        let mut sched = Schedule::new();
        sched
            .add_system(
                &mut world,
                Stage::Update,
                |values: Query<WriteOnly<ReactiveValue>>| {
                    for mut value in values {
                        value.0 += 1;
                    }
                },
            )
            .add_system(
                &mut world,
                Stage::Update,
                |mut seen: ResMut<SeenA>, values: Query<&ReactiveValue, Changed<ReactiveValue>>| {
                    seen.0.extend(values.into_iter().map(|value| value.0));
                },
            )
            .add_system(
                &mut world,
                Stage::Update,
                |mut seen: ResMut<SeenB>, values: Query<&ReactiveValue, Changed<ReactiveValue>>| {
                    seen.0.extend(values.into_iter().map(|value| value.0));
                },
            );

        sched.run(&mut world);
        sched.run(&mut world);

        assert_eq!(world.resource::<SeenA>(), Some(&SeenA(vec![1, 2])));
        assert_eq!(world.resource::<SeenB>(), Some(&SeenB(vec![1, 2])));
    }

    #[test]
    fn false_run_condition_does_not_advance_reactive_cursor() {
        #[derive(Debug, PartialEq)]
        struct Enabled(bool);

        let mut world = World::new();
        let entity = world.spawn();
        world.insert(entity, ReactiveValue(0));
        world.insert_resource(Enabled(false));
        world.insert_resource(SeenValues::default());

        let mut sched = Schedule::new();
        sched
            .add_system(
                &mut world,
                Stage::Update,
                (|mut seen: ResMut<SeenValues>,
                  values: Query<&ReactiveValue, Changed<ReactiveValue>>| {
                    seen.0.extend(values.into_iter().map(|value| value.0));
                })
                .run_if(|world| world.resource::<Enabled>().is_some_and(|enabled| enabled.0)),
            )
            .add_system(
                &mut world,
                Stage::Update,
                |values: Query<WriteOnly<ReactiveValue>>| {
                    for mut value in values {
                        value.0 += 1;
                    }
                },
            );

        sched.run(&mut world);
        world.resource_mut::<Enabled>().unwrap().0 = true;
        sched.run(&mut world);
        world.resource_mut::<Enabled>().unwrap().0 = false;
        sched.run(&mut world);
        world.resource_mut::<Enabled>().unwrap().0 = true;
        sched.run(&mut world);

        assert_eq!(
            world.resource::<SeenValues>(),
            Some(&SeenValues(vec![1, 3]))
        );
    }

    #[test]
    fn zero_then_multiple_fixed_steps_do_not_replay_additions() {
        #[derive(Debug, Default, PartialEq)]
        struct Counts(Vec<usize>);

        let mut world = World::new();
        let entity = world.spawn();
        world.insert(entity, ReactiveValue(1));
        world.insert_resource(Counts::default());

        let mut sched = Schedule::new();
        sched.add_system(
            &mut world,
            Stage::FixedUpdate,
            |mut counts: ResMut<Counts>, values: Query<&ReactiveValue, Added<ReactiveValue>>| {
                counts.0.push(values.into_iter().count());
            },
        );

        // A render frame with zero fixed steps does not execute or advance
        // the fixed consumer.
        assert_eq!(world.resource::<Counts>(), Some(&Counts(vec![])));
        assert_eq!(world.current_tick(), 0);

        sched.run_fixed(&mut world);
        sched.run_fixed(&mut world);
        sched.run_fixed(&mut world);

        assert_eq!(world.resource::<Counts>(), Some(&Counts(vec![1, 0, 0])));
    }

    #[test]
    #[should_panic(expected = "change epoch overflow")]
    fn schedule_refuses_to_wrap_change_epoch() {
        let mut world = World::new();
        world.set_current_tick_for_test(u64::MAX);
        let mut sched = Schedule::new();
        sched.add_system(&mut world, Stage::Update, || {});
        sched.run(&mut world);
    }

    #[test]
    #[should_panic(expected = "alias conflict")]
    fn registration_rejects_aliasing_queries_in_one_system() {
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
