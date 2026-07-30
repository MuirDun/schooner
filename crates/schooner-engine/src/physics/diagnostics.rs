//! Physics profiling workload counters.
//!
//! Puffin answers "where did the frame time go?" These counters answer the
//! adjacent question: "how much work did that scope receive?" They are reset at
//! the start of each render frame and accumulated across every fixed step in
//! that frame, so a diagnostics panel can compare one puffin frame with the
//! matching physics volume without constructing dynamic profiler labels.

use crate::ecs::World;

/// Physics workload accumulated during the current render frame.
///
/// Inserted automatically by [`App::with_physics`](crate::App::with_physics).
/// A frame with several fixed steps accumulates all of them; a frame with no
/// fixed step can still report frame-top lifecycle reconciliation.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct PhysicsDiagnostics {
    pub lifecycle: PhysicsLifecycleWorkload,
    pub transform_sync: PhysicsTransformSyncWorkload,
    pub commands: PhysicsCommandWorkload,
    pub characters: PhysicsCharacterWorkload,
    pub solve: PhysicsSolveWorkload,
    pub writeback: PhysicsWritebackWorkload,
    pub events: PhysicsEventWorkload,
}

/// ECS authoring records considered and Rapier bodies reconciled.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct PhysicsLifecycleWorkload {
    /// Number of lifecycle passes. There can be one at frame top and one per
    /// fixed bridge step.
    pub passes: u64,
    /// Raw authoring/removal records inspected before entity deduplication.
    pub candidate_records: u64,
    /// Distinct complete or incomplete entities handed to reconciliation.
    pub entities: u64,
    pub bodies_removed: u64,
    pub bodies_updated: u64,
    pub bodies_materialized: u64,
}

/// Authored `Transform` changes considered and accepted by Rapier.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct PhysicsTransformSyncWorkload {
    pub candidates: u64,
    pub applied: u64,
}

/// Discrete commands drained by the bridge.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct PhysicsCommandWorkload {
    pub total: u64,
    pub teleports: u64,
    pub character_moves: u64,
    pub character_jumps: u64,
}

/// Character-controller integrations and their hosted KCC queries.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct PhysicsCharacterWorkload {
    pub integrations: u64,
    pub kcc_queries: u64,
}

/// Rapier solver volume accumulated across fixed steps.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct PhysicsSolveWorkload {
    pub steps: u64,
    /// Sum of live body counts sampled before each Rapier step.
    pub body_step_samples: u64,
    /// Sum of live collider counts sampled before each Rapier step.
    pub collider_step_samples: u64,
    /// Sum of awake dynamic bodies emitted for write-back after each step.
    pub active_dynamic_body_steps: u64,
}

/// Dynamic poses emitted by Rapier and ECS `Transform`s actually changed.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct PhysicsWritebackWorkload {
    pub pose_candidates: u64,
    pub poses_written: u64,
}

/// Physics events copied into typed ECS event queues.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct PhysicsEventWorkload {
    pub contacts_published: u64,
    pub trigger_enters_published: u64,
    pub trigger_exits_published: u64,
}

impl PhysicsDiagnostics {
    pub(crate) fn reset(&mut self) {
        *self = Self::default();
    }

    pub(crate) fn merge(&mut self, delta: Self) {
        self.lifecycle.merge(delta.lifecycle);
        self.transform_sync.merge(delta.transform_sync);
        self.commands.merge(delta.commands);
        self.characters.merge(delta.characters);
        self.solve.merge(delta.solve);
        self.writeback.merge(delta.writeback);
        self.events.merge(delta.events);
    }
}

impl PhysicsLifecycleWorkload {
    fn merge(&mut self, delta: Self) {
        add(&mut self.passes, delta.passes);
        add(&mut self.candidate_records, delta.candidate_records);
        add(&mut self.entities, delta.entities);
        add(&mut self.bodies_removed, delta.bodies_removed);
        add(&mut self.bodies_updated, delta.bodies_updated);
        add(&mut self.bodies_materialized, delta.bodies_materialized);
    }
}

impl PhysicsTransformSyncWorkload {
    fn merge(&mut self, delta: Self) {
        add(&mut self.candidates, delta.candidates);
        add(&mut self.applied, delta.applied);
    }
}

impl PhysicsCommandWorkload {
    fn merge(&mut self, delta: Self) {
        add(&mut self.total, delta.total);
        add(&mut self.teleports, delta.teleports);
        add(&mut self.character_moves, delta.character_moves);
        add(&mut self.character_jumps, delta.character_jumps);
    }
}

impl PhysicsCharacterWorkload {
    fn merge(&mut self, delta: Self) {
        add(&mut self.integrations, delta.integrations);
        add(&mut self.kcc_queries, delta.kcc_queries);
    }
}

impl PhysicsSolveWorkload {
    fn merge(&mut self, delta: Self) {
        add(&mut self.steps, delta.steps);
        add(&mut self.body_step_samples, delta.body_step_samples);
        add(&mut self.collider_step_samples, delta.collider_step_samples);
        add(
            &mut self.active_dynamic_body_steps,
            delta.active_dynamic_body_steps,
        );
    }
}

impl PhysicsWritebackWorkload {
    fn merge(&mut self, delta: Self) {
        add(&mut self.pose_candidates, delta.pose_candidates);
        add(&mut self.poses_written, delta.poses_written);
    }
}

impl PhysicsEventWorkload {
    fn merge(&mut self, delta: Self) {
        add(&mut self.contacts_published, delta.contacts_published);
        add(
            &mut self.trigger_enters_published,
            delta.trigger_enters_published,
        );
        add(
            &mut self.trigger_exits_published,
            delta.trigger_exits_published,
        );
    }
}

fn add(total: &mut u64, delta: u64) {
    *total = total.saturating_add(delta);
}

pub(crate) fn reset_physics_diagnostics(world: &mut World) {
    if let Some(diagnostics) = world.resource_mut::<PhysicsDiagnostics>() {
        diagnostics.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_accumulates_and_saturates_counters() {
        let mut diagnostics = PhysicsDiagnostics::default();
        diagnostics.solve.steps = u64::MAX;

        let mut delta = PhysicsDiagnostics::default();
        delta.solve.steps = 1;
        delta.events.contacts_published = 3;
        diagnostics.merge(delta);

        assert_eq!(diagnostics.solve.steps, u64::MAX);
        assert_eq!(diagnostics.events.contacts_published, 3);
    }

    #[test]
    fn frame_reset_clears_every_workload_group() {
        let mut world = World::new();
        let mut diagnostics = PhysicsDiagnostics::default();
        diagnostics.lifecycle.passes = 2;
        diagnostics.characters.kcc_queries = 4;
        world.insert_resource(diagnostics);

        reset_physics_diagnostics(&mut world);

        assert_eq!(
            world.resource::<PhysicsDiagnostics>(),
            Some(&PhysicsDiagnostics::default())
        );
    }
}
