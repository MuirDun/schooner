use bitflags::bitflags;
use glam::{Quat, Vec2, Vec3};
use schooner_engine::{
    BlendMode, ColorGrade, DirectionalLight, EntityId, Fog, Material, MeshHandle, PointLight,
    Shadowcaster, SpotLight, Transform, Vignette, World,
};

use crate::scene::{
    SceneEntity,
    assets::{Assets, MeshAsset, SceneAssets, TextureAsset},
};

pub const MANIFEST: SceneAssets = SceneAssets {
    texture: &[
        TextureAsset::Glass,
        TextureAsset::IronWall,
        TextureAsset::IronWallNormal,
        TextureAsset::MetalFloor,
        TextureAsset::MetalFloorNormal,
        TextureAsset::MetalCube,
    ],
    mesh: &[MeshAsset::Eye],
};

/// World-space size of one texture tile, in metres. The chamber surfaces
/// use triplanar projection, so this is a single world-space repeat
/// distance shared by every wall and the floor — the texture is
/// continuous across the boxes, repeating every `TILE_METERS`.
const TILE_METERS: f32 = 2.0;

/// Triplanar world scale (repeats per metre) packed into `uv_scale.x`.
fn triplanar_scale() -> Vec2 {
    Vec2::splat(1.0 / TILE_METERS)
}

bitflags! {
    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    pub struct Walls: u8 {
        const FLOOR   = 1 << 0;
        const CEILING = 1 << 1;
        const EAST    = 1 << 2;
        const WEST    = 1 << 3;
        const NORTH   = 1 << 4;
        const SOUTH   = 1 << 5;
    }
}

impl Default for Walls {
    fn default() -> Self {
        Walls::all()
    }
}

fn spawn_room(
    world: &mut World,
    center: Vec3,
    size: Vec3,
    thickness: f32,
    walls: Walls,
    spawn_wall: impl Fn(&mut World, Vec3, Vec3) -> EntityId,
) {
    let h = size / 2.0; // interior half-extents
    let ht = thickness / 2.0;

    if walls.contains(Walls::FLOOR) {
        spawn_wall(
            world,
            Vec3::new(center.x, center.y - h.y - ht, center.z),
            Vec3::new(size.x, thickness, size.z),
        );
    }
    if walls.contains(Walls::CEILING) {
        spawn_wall(
            world,
            Vec3::new(center.x, center.y + h.y + ht, center.z),
            Vec3::new(size.x, thickness, size.z),
        );
    }
    if walls.contains(Walls::WEST) {
        spawn_wall(
            world,
            Vec3::new(center.x - h.x - ht, center.y, center.z),
            Vec3::new(thickness, size.y, size.z),
        );
    }
    if walls.contains(Walls::EAST) {
        spawn_wall(
            world,
            Vec3::new(center.x + h.x + ht, center.y, center.z),
            Vec3::new(thickness, size.y, size.z),
        );
    }
    if walls.contains(Walls::NORTH) {
        spawn_wall(
            world,
            Vec3::new(center.x, center.y, center.z - h.z - ht),
            Vec3::new(size.x, size.y, thickness),
        );
    }
    if walls.contains(Walls::SOUTH) {
        spawn_wall(
            world,
            Vec3::new(center.x, center.y, center.z + h.z + ht),
            Vec3::new(size.x, size.y, thickness),
        );
    }
}

fn spawn_box(world: &mut World, center: Vec3, size: Vec3) -> EntityId {
    let e = world.spawn();
    world.insert(
        e,
        Transform {
            translation: center,
            rotation: Quat::IDENTITY,
            scale: size,
        },
    );
    world.insert(e, MeshHandle::CUBE);
    world.insert(e, SceneEntity);

    e
}

fn spawn_lab_wall(world: &mut World, center: Vec3, size: Vec3) -> EntityId {
    let wall = spawn_box(world, center, size);

    world.insert(
        wall,
        Material {
            albedo: Vec3::splat(0.02),
            roughness: 1.0,
            ..Material::default()
        },
    );

    wall
}

/// A chamber wall/ceiling box: built geometry plus the iron-wall albedo +
/// normal, tiled to `TILE_METERS`. `normal_strength` is held low (0.6) so
/// the relief reads as raked panelwork, not a bump-everything demo.
fn spawn_chamber_wall(world: &mut World, center: Vec3, size: Vec3) -> EntityId {
    let wall = spawn_box(world, center, size);

    let (albedo, normal) = world
        .resource::<Assets>()
        .map(|a| {
            (
                a.texture(TextureAsset::IronWall),
                a.texture(TextureAsset::IronWallNormal),
            )
        })
        .unwrap();

    world.insert(
        wall,
        Material {
            albedo_texture: Some(albedo),
            normal_texture: Some(normal),
            normal_strength: 0.6,
            roughness: 0.7,
            triplanar: true,
            uv_scale: triplanar_scale(),
            ..Material::DEFAULT
        },
    );

    wall
}

fn spawn_cube(world: &mut World, center: Vec3, size: Vec3) -> EntityId {
    let cube = spawn_box(world, center, size);

    let texture = world
        .resource::<Assets>()
        .map(|a| a.texture(TextureAsset::MetalCube))
        .unwrap();

    world.insert(
        cube,
        Material {
            albedo_texture: Some(texture),
            roughness: 0.5,
            ..Material::DEFAULT
        },
    );
    cube
}

/// The chamber floor: the AmbientCG metal albedo + OpenGL normal, tiled to
/// `TILE_METERS`. Slightly lower roughness than the walls for a tighter
/// floor highlight under the raking lamp.
fn spawn_chamber_floor(world: &mut World, center: Vec3, size: Vec3) -> EntityId {
    let floor = spawn_box(world, center, size);

    let (albedo, normal) = world
        .resource::<Assets>()
        .map(|a| {
            (
                a.texture(TextureAsset::MetalFloor),
                a.texture(TextureAsset::MetalFloorNormal),
            )
        })
        .unwrap();

    world.insert(
        floor,
        Material {
            albedo_texture: Some(albedo),
            normal_texture: Some(normal),
            normal_strength: 0.6,
            roughness: 0.35,
            triplanar: true,
            uv_scale: triplanar_scale(),
            ..Material::DEFAULT
        },
    );

    floor
}
fn spawn_window(world: &mut World, center: Vec3, size: Vec3) {
    let e = world.spawn();
    world.insert(
        e,
        Transform {
            translation: center,
            rotation: Quat::from_rotation_z(std::f32::consts::FRAC_PI_2),
            scale: size,
        },
    );
    world.insert(e, MeshHandle::PLANE);
    world.insert(e, SceneEntity);

    let glass_texture = world
        .resource::<Assets>()
        .map(|a| a.texture(TextureAsset::Glass));

    log::info!("Paint window {:?}", glass_texture);
    world.insert(
        e,
        Material {
            albedo: Vec3::new(0.6, 0.7, 0.8),
            albedo_texture: glass_texture,
            roughness: 0.1,
            opacity: 0.2,
            blend: BlendMode::AlphaBlend,
            fresnel: 1.0,
            ..Material::DEFAULT
        },
    );
}

fn spawn_spot(
    world: &mut World,
    pos: Vec3,
    target: Vec3,
    light: SpotLight,
    casts_shadow: bool,
) -> EntityId {
    let e = world.spawn();
    let dir = (target - pos).normalize();
    world.insert(
        e,
        Transform {
            translation: pos,
            // Spot shines along local -Z; rotate -Z onto the aim vector.
            // (from_rotation_arc is only ambiguous when dir == +Z; our
            // lamps always point downward, so it's safe here.)
            rotation: Quat::from_rotation_arc(Vec3::NEG_Z, dir),
            scale: Vec3::ONE,
        },
    );
    world.insert(e, light);
    if casts_shadow {
        world.insert(e, Shadowcaster);
    }
    world.insert(e, SceneEntity);
    e
}

fn spawn_point(world: &mut World, pos: Vec3, light: PointLight) -> EntityId {
    let e = world.spawn();
    world.insert(
        e,
        Transform {
            translation: pos,
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
        },
    );
    world.insert(e, light);
    world.insert(e, SceneEntity);
    e
}

fn spawn_eye(world: &mut World, pos: Vec3, size: Vec3) {
    let model = world
        .resource::<Assets>()
        .map(|a| a.model(MeshAsset::Eye))
        .unwrap();

    let eye = world.spawn();

    world.insert(
        eye,
        Transform {
            translation: pos,
            rotation: Quat::from_rotation_z((270.0 as f32).to_radians()),
            scale: size,
        },
    );
    world.insert(eye, model.mesh);
    world.insert(
        eye,
        Material {
            albedo_texture: model.albedo_texture,
            normal_texture: model.normal_texture,
            // Subtle relief — dial up if the eye should read more rugged.
            normal_strength: 0.5,
            ..Material::DEFAULT
        },
    );
    world.insert(eye, SceneEntity);
}

pub fn build(world: &mut World) {
    // Zone mood — hostile clinical lab: cold grade + cold thin haze.
    world.insert_resource(ColorGrade::CHAMBER_WHITE);
    world.insert_resource(Vignette::CINEMATIC);
    world.insert_resource(Fog {
        color: Vec3::new(0.05, 0.06, 0.08),
        base_height: 0.0,
        density: 0.03,
        falloff: 0.3,
        scattering: 0.25,
    });

    let chamber_size_x = 11.0;
    let chamber_size_z = 8.0;
    let chamber_size_y = 5.0;

    let h_cs_x = chamber_size_x / 2.0;
    let h_cs_y = chamber_size_y / 2.0;
    let h_cs_z = chamber_size_z / 2.0;

    // thickness
    let t = 0.2;
    let h_t = t / 2.0;

    // Chamber shell: ceiling + west/south walls get the iron material.
    // FLOOR is excluded here and spawned separately so it can take the
    // distinct metal-floor texture.
    spawn_room(
        world,
        Vec3::new(0.0, h_cs_y, 0.0),
        Vec3::new(chamber_size_x, chamber_size_y, chamber_size_z),
        t,
        Walls::all() & !Walls::NORTH & !Walls::EAST & !Walls::FLOOR,
        spawn_chamber_wall,
    );

    // Chamber floor — its own texture. Position/size mirror what
    // `spawn_room` would compute for the FLOOR slab.
    spawn_chamber_floor(
        world,
        Vec3::new(0.0, -h_t, 0.0),
        Vec3::new(chamber_size_x, t, chamber_size_z),
    );

    // North Wall with a hole
    // hole position z
    let h_p_z = -h_cs_z - h_t;
    let hole_size_y = 1.2;
    let hole_size_x = 2.0;

    let h_hs_x = hole_size_x / 2.0;
    let h_hs_y = hole_size_y / 2.0;
    let panel_horizontal_size = h_cs_x - h_hs_x;
    let panel_vertical_size = h_cs_y - h_hs_y;

    // North wall is part of the chamber shell — iron material too.
    spawn_chamber_wall(
        world,
        Vec3::new(-h_cs_x + panel_horizontal_size / 2.0, h_cs_y, h_p_z),
        Vec3::new(panel_horizontal_size, chamber_size_y, t),
    ); // left panel
    spawn_chamber_wall(
        world,
        Vec3::new(h_cs_x - panel_horizontal_size / 2.0, h_cs_y, h_p_z),
        Vec3::new(panel_horizontal_size, chamber_size_y, t),
    ); // right panel
    spawn_chamber_wall(
        world,
        Vec3::new(0.0, chamber_size_y - panel_vertical_size / 2.0, h_p_z),
        Vec3::new(hole_size_x, panel_vertical_size, t),
    ); // top panel
    spawn_chamber_wall(
        world,
        Vec3::new(0.0, 0.0 + panel_vertical_size / 2.0, h_p_z),
        Vec3::new(hole_size_x, panel_vertical_size, t),
    ); // bottom panel

    // Tunnel
    let tunnel_length = 4.0;
    let t_p_z = h_p_z - tunnel_length / 2.0 - h_t;
    spawn_box(
        world,
        Vec3::new(-(h_hs_x + h_t), h_cs_y, t_p_z),
        Vec3::new(t, hole_size_y, tunnel_length),
    ); // left tunnel wall
    spawn_box(
        world,
        Vec3::new(h_hs_x + h_t, h_cs_y, t_p_z),
        Vec3::new(t, hole_size_y, tunnel_length),
    ); // right tunnel wall
    spawn_box(
        world,
        Vec3::new(0.0, h_cs_y - h_hs_y - h_t, t_p_z),
        Vec3::new(hole_size_x, t, tunnel_length),
    ); // top tunnel wall
    spawn_box(
        world,
        Vec3::new(0.0, h_cs_y + h_hs_y + h_t, t_p_z),
        Vec3::new(hole_size_x, t, tunnel_length),
    ); // bottom tunnel wall
    spawn_box(
        world,
        Vec3::new(0.0, h_cs_y, h_p_z - tunnel_length - h_t),
        Vec3::new(hole_size_x, hole_size_y, t),
    ); // end of tunnel
    // spawn_box(world, Vec3::new(1.0, 1.5, -5.0), Vec3::new(t, 1.0, 4.0));
    // spawn_box(world, Vec3::new(0.0, 1.0, -5.0), Vec3::new(1.6, t, 4.0)); // sill (below)
    // spawn_box(world, Vec3::new(0.0, 2.0, -5.0), Vec3::new(1.6, t, 4.0)); // sill (top)

    // East window
    spawn_window(
        world,
        Vec3::new(h_cs_x, h_cs_y, 0.0),
        Vec3::new(chamber_size_y, chamber_size_y, chamber_size_z),
    );

    // Light

    // Ambient — cold and lifted. A hostile lab is over-lit: brighter than
    // the moody warm default and pushed blue-grey, so shadows read as cold
    // fill rather than black. This is the main "birch → clinical" move; it
    // lifts the whole room toward aggressive white without flattening the
    // raking hero spot (which still carves the normal-map relief).
    let sun = world.spawn();
    world.insert(
        sun,
        DirectionalLight {
            intensity: 0.0,
            ambient: Vec3::new(0.18, 0.20, 0.26),
            ..DirectionalLight::default()
        },
    );
    world.insert(sun, SceneEntity);

    // Overhead lamps — all room-relative so they track your resizes.

    // Hero: surgical white, offset to a corner so it RAKES across the
    // floor → long hard shadows. Intensity is high on purpose (1/d²).
    let lamp_y = chamber_size_y * 0.92;
    let lamp_range = chamber_size_y * 2.0;

    // ONE functional lamp: mounted slightly off-center over the subject,
    // aimed near-straight-down. Reads as equipment, not cinematography.
    spawn_spot(
        world,
        Vec3::new(-h_cs_x * 0.15, lamp_y, -h_cs_z * 0.10),
        Vec3::new(h_cs_x * 0.05, 0.0, h_cs_z * 0.05), // nearly below itself
        // Cool fluorescent white — faintly blue, the interrogation-lamp
        // tone. Pairs with the cold ambient and CLINICAL_COLD grade.
        SpotLight::new(Vec3::new(0.92, 0.96, 1.0), 28.0, lamp_range, 42.0, 60.0)
            .with_god_ray_intensity(1.4),
        true,
    );

    // Fill: dimmer, faintly warm, opposite corner — keeps the room legible
    // without flattening the hero's shadows. No shadow (cost), low god-ray.
    // spawn_spot(
    //     world,
    //     Vec3::new(h_cs_x * 0.4, lamp_y, h_cs_z * 0.35),
    //     Vec3::ZERO,
    //     SpotLight::new(Vec3::new(1.0, 0.96, 0.9), 10.0, lamp_range, 40.0, 60.0)
    //         .with_god_ray_intensity(0.4),
    //     false,
    // );

    // Red service light, deep in the tunnel.
    spawn_point(
        world,
        Vec3::new(0.0, chamber_size_y * 0.5, h_p_z - tunnel_length * 0.8),
        PointLight::new(Vec3::new(1.0, 0.06, 0.03), 6.0, tunnel_length),
    );

    spawn_cube(world, Vec3::new(h_cs_x * 0.1, 0.4, 0.0), Vec3::splat(0.8));
    spawn_cube(world, Vec3::new(h_cs_x * 0.1, 1.0, 0.0), Vec3::splat(0.7));
    spawn_cube(
        world,
        Vec3::new(-h_cs_x * 0.25, 0.35, h_cs_z * 0.2),
        Vec3::splat(0.7),
    );

    spawn_eye(
        world,
        Vec3::new(h_cs_x + 5.0, h_cs_y, 0.0),
        Vec3::new(3.0, 2.8, 3.0),
    );

    // Outer room
    spawn_room(
        world,
        Vec3::new(20.0, 0.0, 0.0),
        Vec3::new(40.0, 40.0, 60.0),
        0.1,
        Walls::all() & !Walls::WEST,
        spawn_lab_wall,
    );

    spawn_point(
        world,
        Vec3::new(39.99, 19.99, -29.99),
        PointLight::new(Vec3::new(1.0, 0.06, 0.03), 2.0, 20.0),
    );
}
