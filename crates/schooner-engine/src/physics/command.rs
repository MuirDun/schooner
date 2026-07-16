//! Explicit commands consumed by the physics bridge.
//!
//! Authoring components describe persistent physical state. Commands describe
//! one-shot operations that should not be inferred from ordinary component
//! mutation, such as discontinuously relocating a solver-owned body.

use crate::ecs::EntityId;
use crate::transform::Transform;

/// What should happen to a dynamic body's solver velocity when it is
/// teleported.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TeleportVelocity {
    /// Keep linear and angular velocity. Useful for portals or scene
    /// transitions that preserve momentum.
    Preserve,
    /// Clear linear and angular velocity. This is the usual reset /
    /// respawn behavior.
    Clear,
}

/// Queues one-shot physics operations for the next physics bridge run.
///
/// Systems write commands through `ResMut<PhysicsCommands>`. The bridge drains
/// the queue immediately before stepping Rapier, so commands persist across
/// zero-fixed-step frames instead of expiring like short-lived events.
#[derive(Debug, Default)]
pub struct PhysicsCommands {
    commands: Vec<PhysicsCommand>,
}

impl PhysicsCommands {
    pub fn new() -> Self {
        Self::default()
    }

    /// Discontinuously move a body and clear its dynamic velocity.
    ///
    /// This is for reset/respawn-style movement. For ordinary kinematic
    /// motion, write the entity's `Transform`; the bridge will use Rapier's
    /// next-kinematic-position path.
    pub fn teleport_body(&mut self, entity: EntityId, transform: Transform) {
        self.teleport_body_with_velocity(entity, transform, TeleportVelocity::Clear);
    }

    /// Discontinuously move a body while preserving its dynamic velocity.
    pub fn teleport_body_preserving_velocity(&mut self, entity: EntityId, transform: Transform) {
        self.teleport_body_with_velocity(entity, transform, TeleportVelocity::Preserve);
    }

    /// Discontinuously move a body with an explicit dynamic velocity policy.
    pub fn teleport_body_with_velocity(
        &mut self,
        entity: EntityId,
        transform: Transform,
        velocity: TeleportVelocity,
    ) {
        self.commands.push(PhysicsCommand::TeleportBody {
            entity,
            transform,
            velocity,
        });
    }

    pub fn len(&self) -> usize {
        self.commands.len()
    }

    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }

    pub(crate) fn drain(&mut self) -> impl Iterator<Item = PhysicsCommand> + '_ {
        self.commands.drain(..)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum PhysicsCommand {
    TeleportBody {
        entity: EntityId,
        transform: Transform,
        velocity: TeleportVelocity,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

    #[test]
    fn teleport_body_queues_clear_velocity_command() {
        let mut commands = PhysicsCommands::new();
        let entity = EntityId {
            index: 1,
            generation: 0,
        };
        let transform = Transform::from_translation(Vec3::Y);

        commands.teleport_body(entity, transform);

        assert_eq!(
            commands.drain().collect::<Vec<_>>(),
            vec![PhysicsCommand::TeleportBody {
                entity,
                transform,
                velocity: TeleportVelocity::Clear,
            }]
        );
    }
}
