//! `Commands` — deferred structural world mutations.
//!
//! A non-exclusive system (one taking `Query` / `Res` params, not
//! `&mut World`) can't spawn, despawn, insert, or remove while it holds
//! its borrows. `Commands` lets it *queue* those ops; they are applied
//! at a defined sync point — the end of each stage run — when the
//! schedule holds the exclusive `&mut World`.
//!
//! ## Local queue and the scheduler application seam
//!
//! All local `Commands` handles push into one [`CommandQueue`] resource,
//! and boxed closures apply in queue order. External, AI, or threaded
//! producers cannot generally use this representation: a captured Rust
//! closure is neither a wire format nor a boundary protocol. Such producers
//! will use structured messages suited to their ordering, backpressure, and
//! failure requirements, then converge with this queue at the scheduler's
//! authoritative world-mutation seam.
//!
//! ## Boxed closures, not a command enum
//!
//! Each command is a `Box<dyn FnOnce(&mut World)>`. A closure captures
//! the concrete component type for `insert` / `remove` without a
//! variant per `T`, and `spawn_with` can run arbitrary setup against
//! the freshly created entity.

use std::any::{TypeId, type_name};

use crate::ecs::system::{ParamAccess, ResourceAccess, SystemParam};
use crate::ecs::{Component, EntityId, World};
use crate::error::EngineError;

/// One deferred world mutation. `Send + Sync` because [`CommandQueue`]
/// is a resource and the resource bag requires both (the World is kept
/// `Send + Sync` for a future parallel scheduler); `'static` because it
/// outlives the system that queued it. The `Sync` bound costs nothing
/// in practice: every command payload is an `EntityId` or a `Component`
/// value, and `Component` already implies `Send + Sync`.
type Command = Box<dyn FnOnce(&mut World) + Send + Sync + 'static>;

/// Shared buffer of deferred world ops. A resource; systems push into
/// it through the [`Commands`] param, and the schedule drains it at the
/// end of each stage via [`CommandQueue::apply`].
#[derive(Default)]
pub struct CommandQueue {
    commands: Vec<Command>,
}

impl CommandQueue {
    pub fn len(&self) -> usize {
        self.commands.len()
    }

    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }

    fn push(&mut self, command: Command) {
        self.commands.push(command);
    }

    /// Apply and clear every queued command, in order. No-op if the
    /// resource is absent.
    ///
    /// We `mem::take` the command vec out of the in-place resource
    /// before running anything, so each command gets the full
    /// `&mut World` (it can spawn, despawn, even queue more commands)
    /// with no borrow of the queue held. Commands queued *during* apply
    /// land in the now-fresh vec and flush at the next sync point —
    /// they are not applied in this pass.
    pub fn apply(world: &mut World) {
        let Some(queue) = world.resource_mut::<CommandQueue>() else {
            return;
        };
        let drained = std::mem::take(&mut queue.commands);
        // `queue` borrow ends here; `drained` is owned, `world` is free.
        for command in drained {
            command(world);
        }
    }
}

/// Deferred-mutation handle handed to a system as a parameter.
///
/// Queue ops with [`spawn_with`](Self::spawn_with), [`despawn`](Self::despawn),
/// [`insert`](Self::insert), [`remove`](Self::remove), or the general
/// [`queue`](Self::queue); they apply at the end of the current stage.
pub struct Commands<'w> {
    queue: &'w mut CommandQueue,
}

impl<'w> Commands<'w> {
    /// Queue spawning a new entity, running `build` against it once it
    /// exists at the sync point. Deferred, so the new id is *not*
    /// available to the calling system this frame — `build` is where
    /// you populate it.
    pub fn spawn_with(&mut self, build: impl FnOnce(&mut World, EntityId) + Send + Sync + 'static) {
        self.queue.push(Box::new(move |world| {
            let entity = world.spawn();
            build(world, entity);
        }));
    }

    /// Queue despawning `entity`. Goes through `World::despawn`, so the
    /// removal ledger captures it for `removed::<T>()` readers.
    pub fn despawn(&mut self, entity: EntityId) {
        self.queue.push(Box::new(move |world| {
            log::info!("DESPAWNED");
            world.despawn(entity);
        }));
    }

    /// Queue inserting `component` onto `entity` (silently dropped at
    /// apply time if the entity is no longer alive — same contract as
    /// `World::insert`).
    pub fn insert<T: Component>(&mut self, entity: EntityId, component: T) {
        self.queue.push(Box::new(move |world| {
            world.insert(entity, component);
        }));
    }

    /// Queue removing `T` from `entity`.
    pub fn remove<T: Component>(&mut self, entity: EntityId) {
        self.queue.push(Box::new(move |world| {
            world.remove::<T>(entity);
        }));
    }

    /// Queue an arbitrary local deferred world mutation. This is an
    /// in-process escape hatch for operations the typed helpers do not cover;
    /// external or concurrent producers use structured boundary messages and
    /// join this path at the scheduler's application seam.
    pub fn queue(&mut self, command: impl FnOnce(&mut World) + Send + Sync + 'static) {
        self.queue.push(Box::new(command));
    }
}

impl SystemParam for Commands<'_> {
    type Item<'w> = Commands<'w>;

    fn access(_world: &mut World) -> ParamAccess {
        let mut access = ParamAccess::new();
        // Mutable access to the one CommandQueue resource — feeds the
        // existing conflict check (two `Commands`, or `Commands` +
        // `ResMut<CommandQueue>`, in one system is rejected).
        access.resources.push(ResourceAccess {
            type_id: TypeId::of::<CommandQueue>(),
            type_name: type_name::<CommandQueue>(),
            mutable: true,
        });
        access
    }

    unsafe fn fetch<'w>(world: &'w mut World) -> Self::Item<'w> {
        let queue = world.resource_mut::<CommandQueue>().unwrap_or_else(|| {
            panic!(
                "{}",
                EngineError::MissingResource {
                    name: type_name::<CommandQueue>()
                }
            )
        });
        // SAFETY: mirrors `ResMut::fetch` — the caller validated no
        // conflicting param is live, so extending the borrow to `'w`
        // (the world borrow's lifetime) is sound.
        let queue: &'w mut CommandQueue =
            unsafe { std::mem::transmute::<&mut CommandQueue, &'w mut CommandQueue>(queue) };
        Commands { queue }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ecs::{Schedule, Stage};

    #[derive(Debug, PartialEq)]
    struct Health(i32);

    fn world_with_queue() -> World {
        let mut world = World::new();
        world.insert_resource(CommandQueue::default());
        world
    }

    #[test]
    fn apply_is_noop_without_queue_resource() {
        let mut world = World::new();
        CommandQueue::apply(&mut world); // must not panic
    }

    #[test]
    fn scheduled_insert_applies_at_stage_end() {
        let mut world = world_with_queue();
        let e = world.spawn();

        let mut sched = Schedule::new();
        sched.add_system(&mut world, Stage::Update, move |mut commands: Commands| {
            commands.insert(e, Health(50));
        });
        // Not applied until the stage runs.
        assert!(world.get::<Health>(e).is_none());
        sched.run(&mut world);
        assert_eq!(world.get::<Health>(e), Some(&Health(50)));
    }

    #[test]
    fn scheduled_spawn_with_creates_populated_entity() {
        let mut world = world_with_queue();
        let mut sched = Schedule::new();
        sched.add_system(&mut world, Stage::Update, |mut commands: Commands| {
            commands.spawn_with(|w, new| {
                w.insert(new, Health(7));
            });
        });
        sched.run(&mut world);
        let healths: Vec<i32> = world.iter::<Health>().map(|(_, h)| h.0).collect();
        assert_eq!(healths, vec![7]);
    }

    #[test]
    fn scheduled_despawn_applies_and_feeds_removed_ledger() {
        let mut world = world_with_queue();
        let e = world.spawn();
        world.insert(e, Health(1));

        let mut sched = Schedule::new();
        sched.add_system(&mut world, Stage::Update, move |mut commands: Commands| {
            commands.despawn(e);
        });
        sched.run(&mut world);

        assert!(!world.is_alive(e));
        // Despawn ran through World::despawn, so the ledger saw it.
        let removed: Vec<_> = world.removed::<Health>().collect();
        assert_eq!(removed, vec![e]);
    }

    #[test]
    fn commands_in_fixed_stage_apply_each_step() {
        // Each FixedUpdate step flushes its own commands.
        let mut world = world_with_queue();
        let mut sched = Schedule::new();
        sched.add_system(&mut world, Stage::FixedUpdate, |mut commands: Commands| {
            commands.spawn_with(|w, new| {
                w.insert(new, Health(1));
            });
        });
        sched.run_fixed(&mut world);
        sched.run_fixed(&mut world);
        // Two steps → two flushes → two entities.
        assert_eq!(world.iter::<Health>().count(), 2);
    }
}
