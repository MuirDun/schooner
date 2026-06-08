use schooner_engine::{RenderContext, TextureHandle, TextureRegistry, World};
use std::collections::HashMap;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum TextureAsset {
    Glass,
}

#[derive(Clone, Copy)]
pub struct SceneAssets {
    pub texture: &'static [TextureAsset],
}

#[derive(Default)]
pub struct Assets {
    textures: HashMap<TextureAsset, TextureHandle>,
}

impl Assets {
    pub fn texture(&self, k: TextureAsset) -> TextureHandle {
        *self
            .textures
            .get(&k)
            .unwrap_or_else(|| panic!("texture {k:?} not resident - missing from manifest"))
    }
}

fn texture_path(k: TextureAsset) -> &'static str {
    match k {
        TextureAsset::Glass => concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/steel-window2.png"
        ),
    }
}

pub fn ensure(world: &mut World, need: SceneAssets) {
    let (device, queue) = match world.resource::<RenderContext>() {
        Some(ctx) => (ctx.device().clone(), ctx.queue().clone()),
        None => return,
    };

    let missing: Vec<TextureAsset> = {
        let a = world.resource::<Assets>().unwrap();
        need.texture
            .iter()
            .copied()
            .filter(|k| !a.textures.contains_key(k))
            .collect()
    };
    let loaded: Vec<(TextureAsset, TextureHandle)> = {
        let reg = world.resource_mut::<TextureRegistry>().unwrap();
        missing
            .iter()
            .filter_map(|&k| match reg.load_png(&device, &queue, texture_path(k)) {
                Ok(h) => Some((k, h)),
                Err(e) => {
                    log::warn!("texture {k:?} failed: {e}");
                    None
                }
            })
            .collect()
    };

    {
        let a = world.resource_mut::<Assets>().unwrap();
        for (k, h) in loaded {
            a.textures.insert(k, h);
        }
    }
}
