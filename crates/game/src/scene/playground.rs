use glam::{Quat, Vec3};
use schooner_engine::{
    BlendMode, ColorGrade, DirectionalLight, EntityId, Fog, Material, MeshHandle, Transform,
    Vignette, World,
};

use crate::scene::{
    SceneEntity,
    assets::{Assets, SceneAssets, TextureAsset},
};

pub const MANIFEST: SceneAssets = SceneAssets {
    texture: &[TextureAsset::Glass],
};

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

pub fn build(world: &mut World) {
    // Zone mood — the per-scene resources. The "chamber" look.
    world.insert_resource(ColorGrade::CHAMBER_WHITE);
    world.insert_resource(Vignette::OPPRESSIVE);
    world.insert_resource(Fog::DEFAULT);

    // Geometry — your greybox chamber, every slab tagged SceneEntity.
    let t = 0.2;
    // Floor + ceiling
    spawn_box(world, Vec3::new(0.0, -t / 2.0, 0.0), Vec3::new(6.0, t, 6.0));
    spawn_box(
        world,
        Vec3::new(0.0, 4.0 + t / 2.0, 0.0),
        Vec3::new(6.0, t, 6.0),
    );

    // Solid walls: west (x=-3) and south (z=+3)
    spawn_box(
        world,
        Vec3::new(-3.0 + -t / 2.0, 2.0, 0.0),
        Vec3::new(t, 4.0, 6.0),
    );
    spawn_box(
        world,
        Vec3::new(0.0, 2.0, 3.0 + t / 2.0),
        Vec3::new(6.0, 4.0, t),
    );

    // North Wall with a hole
    let zn = -3.0 - t / 2.0;
    spawn_box(world, Vec3::new(-1.9, 2.0, zn), Vec3::new(2.2, 4.0, t)); // left jamb
    spawn_box(world, Vec3::new(1.9, 2.0, zn), Vec3::new(2.2, 4.0, t)); // right jamb
    spawn_box(world, Vec3::new(0.0, 0.5, zn), Vec3::new(1.6, 1.0, t)); // sill (below)
    spawn_box(world, Vec3::new(0.0, 3.1, zn), Vec3::new(1.6, 1.8, t)); // sill (below)

    // East window
    spawn_window(world, Vec3::new(3.0, 2.0, 0.0), Vec3::new(4.0, 4.0, 6.0));

    let sun = world.spawn();
    world.insert(
        sun,
        DirectionalLight {
            intensity: 0.2,
            ..DirectionalLight::default()
        },
    );
    world.insert(sun, SceneEntity);
}
