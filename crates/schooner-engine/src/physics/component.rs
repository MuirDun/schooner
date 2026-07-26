//! Authoring components for physical entities.
//!
//! These components are the ECS-facing contract for physics. They describe
//! what the game wants — body authority, collision proxy, surface, and mass —
//! without exposing Rapier's arena handles. The bridge materializes them into
//! Rapier objects and keeps the runtime handles inside the private physics
//! resource.

use glam::Vec3;

/// A distance used by character-controller collision queries.
///
/// Relative lengths scale with the character shape's height; absolute
/// lengths are expressed in world metres. Keeping this vocabulary engine-owned
/// lets the physics bridge translate it to the hosted backend without exposing
/// Rapier types to gameplay.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CharacterLength {
    Relative(f32),
    Absolute(f32),
}

/// Declares that a kinematic body is moved through character-controller
/// collision resolution rather than by directly authoring its next pose.
///
/// Movement intent and solved state land in the following 2.F Steps. This
/// component establishes the stable authoring surface and keeps backend
/// configuration out of game code.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CharacterController {
    /// Clearance maintained between the character shape and nearby geometry.
    pub offset: CharacterLength,
    /// Whether blocked movement slides along contact surfaces.
    pub slide: bool,
    /// Steepest walkable slope, in radians from world up.
    pub max_slope_climb_angle: f32,
    /// Shallowest slope that causes an automatic slide, in radians.
    pub min_slope_slide_angle: f32,
    /// Downward distance used to remain attached to walkable ground.
    pub snap_to_ground: Option<CharacterLength>,
}

impl Default for CharacterController {
    fn default() -> Self {
        Self {
            offset: CharacterLength::Relative(0.01),
            slide: true,
            max_slope_climb_angle: std::f32::consts::FRAC_PI_4,
            min_slope_slide_angle: std::f32::consts::FRAC_PI_4,
            snap_to_ground: Some(CharacterLength::Relative(0.2)),
        }
    }
}

/// Runtime result of the most recent character-controller move.
///
/// The bridge owns these values because they are outcomes of collision
/// resolution. Gameplay may read them to decide whether a jump is allowed,
/// but it does not derive grounded state independently from contacts.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CharacterControllerState {
    pub grounded: bool,
    pub vertical_velocity: f32,
}

impl Default for CharacterControllerState {
    fn default() -> Self {
        Self {
            grounded: false,
            vertical_velocity: 0.0,
        }
    }
}

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

/// Opts an entity into post-solver contact reports.
///
/// Rapier evaluates this threshold as a contact **force** in newtons; the
/// engine event still carries the resolved impulse. Keeping reporting separate
/// from [`Collider`] means ordinary world geometry does not pay callback cost.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ContactEvents {
    pub force_threshold: f32,
}

impl ContactEvents {
    pub const fn new(force_threshold: f32) -> Self {
        Self { force_threshold }
    }
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
    fn character_controller_defaults_to_grounded_fps_conventions() {
        let controller = CharacterController::default();

        assert_eq!(controller.offset, CharacterLength::Relative(0.01));
        assert!(controller.slide);
        assert_eq!(
            controller.max_slope_climb_angle,
            std::f32::consts::FRAC_PI_4
        );
        assert_eq!(
            controller.min_slope_slide_angle,
            std::f32::consts::FRAC_PI_4
        );
        assert_eq!(
            controller.snap_to_ground,
            Some(CharacterLength::Relative(0.2))
        );
    }

    #[test]
    fn character_controller_state_starts_airborne_and_still() {
        assert_eq!(
            CharacterControllerState::default(),
            CharacterControllerState {
                grounded: false,
                vertical_velocity: 0.0,
            }
        );
    }

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

    #[test]
    fn contact_events_carries_a_force_threshold() {
        assert_eq!(ContactEvents::new(12.0).force_threshold, 12.0);
    }
}
