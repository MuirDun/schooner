//! Authoring components for physical entities.
//!
//! These components are the ECS-facing contract for physics. They describe
//! what the game wants — body authority, collision proxy, surface, and mass —
//! without exposing Rapier's arena handles. The bridge materializes them into
//! Rapier objects and keeps the runtime handles inside the private physics
//! resource.

use glam::Vec3;

/// Which side owns an entity's physical pose.
///
/// Dynamic bodies are solved by physics, static bodies are immovable world
/// geometry, and kinematic bodies are moved by gameplay while still pushing
/// against dynamic bodies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyKind {
    Dynamic,
    Static,
    KinematicPositionBased,
}

/// Declares that an entity should exist as a Rapier rigid body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RigidBody {
    pub kind: BodyKind,
}

impl RigidBody {
    pub const fn dynamic() -> Self {
        Self {
            kind: BodyKind::Dynamic,
        }
    }

    pub const fn static_body() -> Self {
        Self {
            kind: BodyKind::Static,
        }
    }

    pub const fn kinematic_position_based() -> Self {
        Self {
            kind: BodyKind::KinematicPositionBased,
        }
    }
}

impl Default for RigidBody {
    fn default() -> Self {
        Self::dynamic()
    }
}

/// Coarse collision proxy authored independently from visual mesh scale.
///
/// Rapier's capsule convention uses `half_height` for the half-distance
/// between the two spherical cap centers, not half of the capsule's total
/// visual height.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ColliderShape {
    Cuboid { half_extents: Vec3 },
    Ball { radius: f32 },
    CapsuleY { half_height: f32, radius: f32 },
}

/// Surface parameters used by contact response.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PhysicsMaterial {
    pub friction: f32,
    pub restitution: f32,
}

impl Default for PhysicsMaterial {
    fn default() -> Self {
        Self {
            friction: 0.5,
            restitution: 0.0,
        }
    }
}

/// Declares the collision shape and physical surface attached to a body.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Collider {
    pub shape: ColliderShape,
    pub mass: f32,
    pub material: PhysicsMaterial,
    pub sensor: bool,
}

impl Collider {
    pub fn cuboid(half_extents: Vec3) -> Self {
        Self {
            shape: ColliderShape::Cuboid { half_extents },
            ..Self::default()
        }
    }

    pub fn ball(radius: f32) -> Self {
        Self {
            shape: ColliderShape::Ball { radius },
            ..Self::default()
        }
    }

    pub fn capsule_y(half_height: f32, radius: f32) -> Self {
        Self {
            shape: ColliderShape::CapsuleY {
                half_height,
                radius,
            },
            ..Self::default()
        }
    }

    pub const fn sensor(mut self, sensor: bool) -> Self {
        self.sensor = sensor;
        self
    }

    pub const fn mass(mut self, mass: f32) -> Self {
        self.mass = mass;
        self
    }

    pub const fn material(mut self, material: PhysicsMaterial) -> Self {
        self.material = material;
        self
    }
}

impl Default for Collider {
    fn default() -> Self {
        Self {
            shape: ColliderShape::Cuboid {
                half_extents: Vec3::splat(0.5),
            },
            mass: 1.0,
            material: PhysicsMaterial::default(),
            sensor: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rigid_body_defaults_to_dynamic() {
        assert_eq!(RigidBody::default(), RigidBody::dynamic());
    }

    #[test]
    fn collider_default_is_unit_cuboid_with_neutral_material() {
        let collider = Collider::default();
        assert_eq!(
            collider.shape,
            ColliderShape::Cuboid {
                half_extents: Vec3::splat(0.5)
            }
        );
        assert_eq!(collider.mass, 1.0);
        assert_eq!(collider.material, PhysicsMaterial::default());
        assert!(!collider.sensor);
    }

    #[test]
    fn collider_builders_preserve_independent_authored_fields() {
        let material = PhysicsMaterial {
            friction: 0.9,
            restitution: 0.2,
        };

        let collider = Collider::ball(0.75)
            .mass(3.0)
            .material(material)
            .sensor(true);

        assert_eq!(collider.shape, ColliderShape::Ball { radius: 0.75 });
        assert_eq!(collider.mass, 3.0);
        assert_eq!(collider.material, material);
        assert!(collider.sensor);
    }
}
