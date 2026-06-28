use bitflags::bitflags;
use glam::{Quat, Vec2, Vec3};
use schooner_engine::{
    AutoExposure, BlendMode, Bloom, ColorGrade, DirectionalLight, EntityId, Fog, Material,
    MeshHandle, PointLight, Shadowcaster, SpotLight, Transform, Vignette, World,
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
            roughness: 0.2,
            albedo_texture: Some(albedo),
            normal_texture: Some(normal),
            normal_strength: 0.6,
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
            roughness: 0.3,
            triplanar: true,
            uv_scale: triplanar_scale(),
            ..Material::DEFAULT
        },
    );

    wall
}

pub fn spawn_cube(world: &mut World, center: Vec3, size: Vec3) -> EntityId {
    let cube = spawn_box(world, center, size);

    let texture = world
        .resource::<Assets>()
        .map(|a| a.texture(TextureAsset::MetalCube))
        .unwrap();

    world.insert(
        cube,
        Material {
            albedo_texture: Some(texture),
            roughness: 0.1,
            ..Material::DEFAULT
        },
    );
    cube
}

/// The food gel-brick (`design/assets.md` §Food). A wet, semi-translucent
/// sulfur gel glowing faintly from within, pressed into a crude brick — the
/// one beacon of warm vivid colour in the cold room, the "thing you want."
///
/// Engine fit, straight from the design doc:
/// - **Inner glow** = `emissive` in sulfur-gold, pushed past 1.0 via
///   `emissive_intensity` so it reads as a light source and trips the bloom
///   bright-pass (Part 2's hunger system will drive this intensity up as the
///   player starves — the gel glows brighter the hungrier you are).
/// - **Wet glassy translucency** = the same `AlphaBlend` + low-`roughness` +
///   `fresnel` glass material the window uses, so the surface glistens and
///   the rim catches light at grazing angles.
fn spawn_food(world: &mut World, center: Vec3, size: Vec3) -> EntityId {
    let gel = spawn_box(world, center, size);

    world.insert(
        gel,
        Material {
            // Murky sulfur body — dim on its own; the inner glow carries it.
            albedo: Vec3::new(0.5, 0.45, 0.12),
            // Sulfur-gold inner glow. Intensity past 1.0 makes it a beacon
            // that blooms against the dim red tunnel (the scene runs
            // `Bloom::ERA_GLOW`, threshold 0.9).
            emissive: Vec3::new(0.95, 0.85, 0.25),
            emissive_intensity: 2.5,
            // Wet glassy gel: glossy, translucent, fresnel rim — same
            // language as `spawn_window`, tuned more substantial (higher
            // opacity) since it's a solid brick, not a thin pane.
            roughness: 0.15,
            opacity: 0.8,
            fresnel: 1.0,
            blend: BlendMode::AlphaBlend,
            ..Material::DEFAULT
        },
    );

    gel
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

fn spawn_lamp(world: &mut World, pos: Vec3, target: Vec3, lamp_range: f32) {
    let shell = spawn_box(
        world,
        pos + Vec3::new(0.0, 0.05, 0.0),
        Vec3::new(0.35, 0.6, 0.35),
    );

    let lamp_light = Vec3::new(0.85, 0.9, 1.0);

    let shell_albedo = world
        .resource::<Assets>()
        .map(|a| a.texture(TextureAsset::MetalFloor))
        .unwrap();
    world.insert(
        shell,
        Material {
            albedo_texture: Some(shell_albedo),
            roughness: 0.15,
            ..Material::DEFAULT
        },
    );

    let lamp = spawn_box(world, pos, Vec3::new(0.3, 0.6, 0.3));
    world.insert(
        lamp,
        Material {
            albedo: lamp_light,
            // Searing HDR emissive — far over 1.0 so it survives the
            // auto-exposure crush and slams the bloom bright-pass: the lamp
            // stays blinding white and glares hard when you face it, while
            // the room around it goes black. This is the "hostile light"
            // cue. Dial up for more pain, down for a calmer fixture.
            emissive: lamp_light,
            emissive_intensity: 14.0,
            roughness: 0.15,
            opacity: 0.8,
            fresnel: 1.0,
            blend: BlendMode::AlphaBlend,
            ..Material::DEFAULT
        },
    );

    spawn_spot(
        world,
        pos + Vec3::new(0.0, -0.2, 0.0),
        target,
        SpotLight::new(lamp_light, 80.0, lamp_range, 42.0, 60.0)
            .with_god_ray_intensity(1.4),
        true,
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
    world.insert_resource(Fog::SCARSE_CHAMBER);
    world.insert_resource(Bloom::ERA_GLOW);
    // Eye adaptation tuned for the chamber: low key + tight exposure ceiling
    // keep the room gloomy, fast darken speed so facing the lamp (or its
    // god-ray) stops the image down and deepens the surrounding dark.
    world.insert_resource(AutoExposure {
        enabled: true,
        // Roughly the centre-weighted luminance that maps to exposure 1.0.
        // Lower => the room reveals/brightens more in the dark and crushes
        // sooner when light enters; raise if the adapted dark reads too
        // bright.
        key: 0.15,
        min_luma: 0.005,
        max_luma: 8.0,
        // Crush floor — facing the lamp drops the room toward black (you're
        // blinded) while the un-exposed bloom halo stays searing.
        min_exposure: 0.2,
        // Ceiling well ABOVE 1.0 so the eye opens up in the dark: stand away
        // from the light and over a couple of seconds the walls reveal. This
        // is the "see more once your eyes adjust" half of eye adaptation.
        // Lower toward 1.5 to keep the dark gloomier; raise for a stronger
        // reveal.
        max_exposure: 1.0,
        // Asymmetric, like a real eye: opening up in the dark is SLOW (the
        // reveal takes a couple seconds — the immersive beat), stopping down
        // when light hits is FAST (the blind is near-instant).
        speed_brighten: 0.5,
        speed_darken: 6.0,
    });

    let chamber_size_x = 11.0;
    let chamber_size_z = 8.0;
    let chamber_size_y = 7.0;

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
    let hole_size_y = 1.9;
    let hole_size_x = 2.0;

    let h_hs_x = hole_size_x / 2.0;
    let h_hs_y = hole_size_y / 2.0;

    // Hole centre on the north-wall plane. (0, h_cs_y) is dead-centre; nudge
    // `hole_cx` negative to slide it left (toward -x — your left when facing
    // the north wall) and raise `hole_cy` to lift it up. The tunnel, gel, and
    // red service light all hang off this point, so the whole passage moves
    // with the hole.
    let hole_cx = -1.5;
    let hole_cy = h_cs_y + 1.0;

    // Four panels framing the hole: full-height left/right flanks plus short
    // top/bottom fillers spanning just the hole width. Each size is the gap
    // between the hole edge and the chamber edge, so they stay flush as the
    // hole centre moves.
    let left_w = (hole_cx - h_hs_x) + h_cs_x;
    let right_w = h_cs_x - (hole_cx + h_hs_x);
    let top_h = chamber_size_y - (hole_cy + h_hs_y);
    let bottom_h = hole_cy - h_hs_y;

    // North wall is part of the chamber shell — iron material too.
    spawn_chamber_wall(
        world,
        Vec3::new(-h_cs_x + left_w / 2.0, h_cs_y, h_p_z),
        Vec3::new(left_w, chamber_size_y, t),
    ); // left panel
    spawn_chamber_wall(
        world,
        Vec3::new(h_cs_x - right_w / 2.0, h_cs_y, h_p_z),
        Vec3::new(right_w, chamber_size_y, t),
    ); // right panel
    spawn_chamber_wall(
        world,
        Vec3::new(hole_cx, chamber_size_y - top_h / 2.0, h_p_z),
        Vec3::new(hole_size_x, top_h, t),
    ); // top panel
    spawn_chamber_wall(
        world,
        Vec3::new(hole_cx, bottom_h / 2.0, h_p_z),
        Vec3::new(hole_size_x, bottom_h, t),
    ); // bottom panel

    let platform_length = 2.0;
    spawn_chamber_wall(
        world,
        Vec3::new(hole_cx, hole_cy - h_hs_y - h_t, h_p_z + platform_length / 2.0 - h_t),
        Vec3::new(hole_size_x, t, platform_length),
    ); // top tunnel wall


    // Tunnel
    let tunnel_length = 4.0;
    let t_p_z = h_p_z - tunnel_length / 2.0 - h_t;

    spawn_chamber_wall(
        world,
        Vec3::new(hole_cx - (h_hs_x + h_t), hole_cy, t_p_z),
        Vec3::new(t, hole_size_y, tunnel_length),
    ); // left tunnel wall
    spawn_chamber_wall(
        world,
        Vec3::new(hole_cx + h_hs_x + h_t, hole_cy, t_p_z),
        Vec3::new(t, hole_size_y, tunnel_length),
    ); // right tunnel wall
    spawn_chamber_wall(
        world,
        Vec3::new(hole_cx, hole_cy - h_hs_y - h_t, t_p_z),
        Vec3::new(hole_size_x, t, tunnel_length),
    ); // bottom tunnel wall
    spawn_chamber_wall(
        world,
        Vec3::new(hole_cx, hole_cy + h_hs_y + h_t, t_p_z),
        Vec3::new(hole_size_x, t, tunnel_length),
    ); // top tunnel wall
    spawn_chamber_wall(
        world,
        Vec3::new(hole_cx, hole_cy, h_p_z - tunnel_length - h_t),
        Vec3::new(hole_size_x, hole_size_y, t),
    ); // end of tunnel

    // East window
    spawn_window(
        world,
        Vec3::new(h_cs_x, h_cs_y, 0.0),
        Vec3::new(chamber_size_y, chamber_size_y, chamber_size_z),
    );

    // Light
    let sun = world.spawn();
    world.insert(
        sun,
        DirectionalLight {
            intensity: 0.0,
            ambient: Vec3::new(0.03, 0.04, 0.05),
            ..DirectionalLight::default()
        },
    );
    world.insert(sun, SceneEntity);

    // Overhead lamps — all room-relative so they track your resizes.

    // Hero: surgical white, offset to a corner so it RAKES across the
    // floor → long hard shadows. Intensity is high on purpose (1/d²).
    let lamp_y = chamber_size_y * 0.96;

    // ONE functional lamp: mounted slightly off-center over the subject,
    // aimed near-straight-down. Reads as equipment, not cinematography.
    let lamp_pos = Vec3::new(-h_cs_x * 0.15, lamp_y, -h_cs_z * 0.10);
    spawn_lamp(
        world,
        lamp_pos,
        lamp_pos * Vec3::new(1.0, 0.0, 1.0), // nearly below itself
        chamber_size_y * 2.0,
    );

    // Red service light, deep in the tunnel.
    spawn_point(
        world,
        Vec3::new(hole_cx, hole_cy, h_p_z - tunnel_length * 0.8),
        PointLight::new(Vec3::new(1.0, 0.06, 0.03), 6.0, tunnel_length),
    );

    let tunnel_floor_y = hole_cy - h_hs_y;
    let gel_size = Vec3::new(0.6, 0.4, 0.5);
    spawn_food(
        world,
        Vec3::new(
            hole_cx,
            tunnel_floor_y + gel_size.y / 2.0,
            h_p_z - tunnel_length * 0.7,
        ),
        gel_size,
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
        Vec3::new(80.0, 50.0, 80.0),
        0.1,
        Walls::all() & !Walls::WEST,
        spawn_lab_wall,
    );

    spawn_point(
        world,
        Vec3::new(59.0, 19.0, -29.0),
        PointLight::new(Vec3::new(1.0, 0.06, 0.03), 10.0, 30.0),
    );
}
