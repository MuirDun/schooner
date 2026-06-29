use schooner_engine::{
    MeshHandle, MeshRegistry, RenderContext, TextureHandle, TextureRegistry, World, load_gltf_model,
};
use std::collections::HashMap;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum TextureAsset {
    Glass,
    IronWall,
    IronWallNormal,
    MetalFloor,
    MetalFloorNormal,
    MetalCube,
}

impl TextureAsset {
    /// `true` for *data* textures (normal maps) — they must upload
    /// through the linear path. `false` for *color* textures (albedo,
    /// glass), which are sRGB-encoded. Drives the `load_png` vs
    /// `load_png_linear` branch in [`ensure`].
    fn is_data(self) -> bool {
        matches!(self, Self::IronWallNormal | Self::MetalFloorNormal)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum MeshAsset {
    Eye,
}

/// A glb loaded as one drawable: its mesh plus the base-color texture
/// that shipped inside the file (`None` if the glb embeds none, in which
/// case the spawn site leaves `Material::albedo_texture` at WHITE). The
/// texture rides along with the mesh so scenes don't list it as a
/// separate `TextureAsset` or hand-stitch it onto the material.
///
/// `Clone`, not `Copy`: it owns ref-counted [`MeshHandle`] /
/// [`TextureHandle`]s, so cloning bumps refcounts and the underlying GPU
/// resources free when the last clone drops.
#[derive(Clone)]
pub struct ModelHandle {
    pub mesh: MeshHandle,
    pub albedo_texture: Option<TextureHandle>,
    /// Normal map embedded in the glb, uploaded through the *linear*
    /// path (it's data, not color). `None` when the glb binds none.
    pub normal_texture: Option<TextureHandle>,
}

#[derive(Clone, Copy)]
pub struct SceneAssets {
    pub texture: &'static [TextureAsset],
    pub mesh: &'static [MeshAsset],
}

#[derive(Default)]
pub struct Assets {
    textures: HashMap<TextureAsset, TextureHandle>,
    models: HashMap<MeshAsset, ModelHandle>,
}

impl Assets {
    /// An owning clone of the resident handle for `k` (cheap refcount
    /// bump). The clone keeps the texture alive for as long as the
    /// caller — typically a spawned entity's `Material` — holds it.
    pub fn texture(&self, k: TextureAsset) -> TextureHandle {
        self.textures
            .get(&k)
            .cloned()
            .unwrap_or_else(|| panic!("texture {k:?} not resident - missing from manifest"))
    }
    pub fn model(&self, k: MeshAsset) -> ModelHandle {
        self.models
            .get(&k)
            .cloned()
            .unwrap_or_else(|| panic!("model {k:?} not resident - missing from manifest"))
    }
}

fn texture_path(k: TextureAsset) -> &'static str {
    match k {
        TextureAsset::Glass => concat!(env!("CARGO_MANIFEST_DIR"), "/assets/steel-window2.png"),
        TextureAsset::MetalCube => concat!(env!("CARGO_MANIFEST_DIR"), "/assets/rusty.png"),
        TextureAsset::IronWall => {
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/metal-wall/wall1_albedo.png"
            )
        }
        TextureAsset::IronWallNormal => {
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/metal-wall/wall1_normal.png"
            )
        }
        TextureAsset::MetalFloor => concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/metal-floor/Metal021_1K-PNG_Color.png"
        ),
        // OpenGL-convention normal (the +Y-up variant our shader expects);
        // the DX sibling in the same folder is deliberately not used.
        TextureAsset::MetalFloorNormal => concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/metal-floor/Metal021_1K-PNG_NormalGL.png"
        ),
    }
}
fn mesh_path(k: MeshAsset) -> &'static str {
    match k {
        MeshAsset::Eye => concat!(env!("CARGO_MANIFEST_DIR"), "/assets/EyeBall.glb"),
    }
}

pub fn ensure(world: &mut World, need: SceneAssets) {
    #[cfg(feature = "hot")]
    return subsecond::call(|| ensure_inner(world, need));
    #[cfg(not(feature = "hot"))]
    return ensure_inner(world, need);
}
fn ensure_inner(world: &mut World, need: SceneAssets) {
    let (device, queue) = match world.resource::<RenderContext>() {
        Some(ctx) => (ctx.device().clone(), ctx.queue().clone()),
        None => return,
    };

    // --- Textures ---
    // Load whatever the next scene needs that isn't already resident. A
    // texture shared with the current scene stays put under the same
    // handle, so a transition never reloads it.
    let missing: Vec<TextureAsset> = {
        let assets = world.resource::<Assets>().unwrap();
        need.texture
            .iter()
            .copied()
            .filter(|k| !assets.textures.contains_key(k))
            .collect()
    };

    let loaded: Vec<(TextureAsset, TextureHandle)> = {
        let reg = world.resource_mut::<TextureRegistry>().unwrap();
        missing
            .iter()
            .filter_map(|&k| {
                let path = texture_path(k);
                // Normal maps are data → linear upload; albedo/glass are
                // color → sRGB upload.
                let result = if k.is_data() {
                    reg.load_png_linear(&device, &queue, path)
                } else {
                    reg.load_png(&device, &queue, path)
                };
                match result {
                    Ok(h) => Some((k, h)),
                    Err(e) => {
                        log::warn!("texture {k:?} failed: {e}");
                        None
                    }
                }
            })
            .collect()
    };

    // Commit residency: add the newly loaded, then drop every texture the
    // next scene doesn't need. Dropping the handle *is* the eviction —
    // once its last owner is gone (this map entry, plus any entity still
    // holding a clone), the registry frees the GPU texture on the next
    // frame's reclaim pass. No manual `unload`, no leak.
    {
        let assets = world.resource_mut::<Assets>().unwrap();
        for (k, h) in loaded {
            assets.textures.insert(k, h);
        }
        assets.textures.retain(|k, _| need.texture.contains(k));
    }

    // --- Models (glb: mesh + embedded textures) ---
    let missing: Vec<MeshAsset> = {
        let assets = world.resource::<Assets>().unwrap();
        need.mesh
            .iter()
            .copied()
            .filter(|k| !assets.models.contains_key(k))
            .collect()
    };

    // Parse first (pure CPU, holds no registry borrow), then upload mesh
    // and texture in separate `&mut` scopes — `World` lends only one
    // resource mutably at a time, and the two registries are distinct
    // types on purpose.
    let parsed: Vec<(MeshAsset, schooner_engine::GltfModel)> = missing
        .iter()
        .filter_map(
            |&k| match load_gltf_model(std::path::Path::new(mesh_path(k))) {
                Ok(model) => Some((k, model)),
                Err(e) => {
                    log::warn!("model {k:?} failed: {e}");
                    None
                }
            },
        )
        .collect();

    let loaded: Vec<(MeshAsset, ModelHandle)> = parsed
        .into_iter()
        .map(|(k, model)| {
            let label = format!("model:{:?}", k);
            let mesh = world
                .resource_mut::<MeshRegistry>()
                .unwrap()
                .insert_mesh_data(&device, &label, &model.mesh);
            let albedo_texture = model.albedo.map(|tex| {
                world
                    .resource_mut::<TextureRegistry>()
                    .unwrap()
                    .insert_texture_data(&device, &queue, &label, &tex)
            });
            // Normal maps go through the *linear* upload — data, not color.
            let normal_texture = model.normal.map(|tex| {
                world
                    .resource_mut::<TextureRegistry>()
                    .unwrap()
                    .insert_texture_data_linear(&device, &queue, &label, &tex)
            });
            (
                k,
                ModelHandle {
                    mesh,
                    albedo_texture,
                    normal_texture,
                },
            )
        })
        .collect();

    // Same commit-then-prune as textures: dropping a redundant
    // `ModelHandle` releases its mesh and embedded textures together.
    {
        let assets = world.resource_mut::<Assets>().unwrap();
        for (k, h) in loaded {
            assets.models.insert(k, h);
        }
        assets.models.retain(|k, _| need.mesh.contains(k));
    }
}
