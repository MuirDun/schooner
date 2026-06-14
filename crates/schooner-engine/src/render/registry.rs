//! Handle → GPU resource registries for meshes and textures.
//!
//! Resource form: live in the `World`, declared by systems via
//! `Res<MeshRegistry>` / `Res<TextureRegistry>` (read-only path used
//! by the renderer) or `ResMut<…>` (write path used by asset loaders).
//!
//! Built-ins (cube + plane for meshes, WHITE for textures) are
//! uploaded eagerly at construction through `with_builtins` — see
//! `architecture/render.md` "What the renderer owns" for why these
//! live in the engine and not in `game-void`.
//!
//! Each entry remembers its disk source path when one exists, so the
//! F5 manual reload (Step 1.F.4) can walk the reloadable subset and
//! re-read each file in place. Built-ins carry `None` and are
//! naturally skipped by that walk.
//!
//! The two registries are parallel-shaped on purpose — same allocator,
//! same source-tracking, same `with_builtins` pattern — but kept as
//! distinct types so that systems can declare `Res<MeshRegistry>` and
//! `Res<TextureRegistry>` independently and the change-detection /
//! resource-disjointness machinery treats them as separate concerns.
//! A shared generic registry would save ~30 lines and lose that.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use wgpu::{Device, Queue};

use crate::asset::{self, AssetResult};
use crate::render::mesh::{MeshData, MeshGpu, MeshHandle, cube_mesh, plane_mesh};
use crate::render::texture::{TextureData, TextureGpu, TextureHandle};

/// Outcome of one manual-reload pass over a registry's disk-sourced
/// entries (the F5 path, Step 1.F.4).
///
/// `reloaded` lists the handles whose GPU resource was replaced in
/// place this pass. The caller uses it to invalidate any downstream
/// cache keyed by handle — the forward pipeline holds one bind group
/// per `TextureHandle` (the `TextureView` is baked into the bind
/// group), so a reloaded texture's cached bind group is stale until
/// invalidated. Meshes have no such cache: `render_frame` reads the
/// `MeshGpu` buffers straight from the registry each draw, so a
/// swapped-in mesh is picked up with no further bookkeeping.
///
/// `failed` counts entries whose re-read errored. Those keep their
/// previous GPU resource — the scene renders the last-good version —
/// per the non-fatal reload contract. Per-failure detail is logged at
/// `warn` inside `reload_all`; this struct carries only the count so
/// the caller can print a one-line summary.
///
/// Generic over the handle type so both registries return the same
/// shape; the registries themselves stay distinct types (see the
/// module doc) — only this value object is shared.
#[derive(Debug, Clone)]
pub struct ReloadReport<H> {
    pub reloaded: Vec<H>,
    pub failed: u32,
}

// Hand-rolled rather than derived: `#[derive(Default)]` would add a
// spurious `H: Default` bound (the well-known derive over-constraint),
// but an empty report is valid for any handle type — `Vec<H>::default()`
// is empty regardless of `H`. Handles deliberately have no `Default`
// (handle 0 is a reserved built-in, not a sensible "default value").
impl<H> Default for ReloadReport<H> {
    fn default() -> Self {
        Self {
            reloaded: Vec::new(),
            failed: 0,
        }
    }
}

#[derive(Debug)]
struct MeshEntry {
    gpu: MeshGpu,
    source: Option<PathBuf>,
}

/// Handle → GPU mesh.
///
/// Insertions go through [`MeshRegistry::insert`],
/// [`MeshRegistry::insert_new`], or [`MeshRegistry::load_gltf`]; the
/// renderer's frame loop only ever calls [`MeshRegistry::get`], so the
/// read path stays cheap (one hash lookup per draw call). Game 0 has
/// no eviction — meshes live as long as the registry does.
#[derive(Debug)]
pub struct MeshRegistry {
    meshes: HashMap<MeshHandle, MeshEntry>,
    next_user_handle: u32,
}

impl MeshRegistry {
    /// Empty registry. Useful for tests; production code goes
    /// through [`MeshRegistry::with_builtins`].
    pub fn empty() -> Self {
        Self {
            meshes: HashMap::new(),
            next_user_handle: MeshHandle::FIRST_USER.0,
        }
    }

    /// Build the registry and upload the engine-owned cube + plane
    /// at the reserved built-in slots. Called once during
    /// `App::resumed` after `RenderContext` is up.
    pub fn with_builtins(device: &Device) -> Self {
        let mut registry = Self::empty();
        let cube = MeshGpu::upload(device, "builtin-cube", &cube_mesh());
        let plane = MeshGpu::upload(device, "builtin-plane", &plane_mesh());
        registry.meshes.insert(
            MeshHandle::CUBE,
            MeshEntry {
                gpu: cube,
                source: None,
            },
        );
        registry.meshes.insert(
            MeshHandle::PLANE,
            MeshEntry {
                gpu: plane,
                source: None,
            },
        );
        registry
    }

    /// Insert a user-supplied mesh under the given handle.
    /// Returns the prior `MeshGpu` if one existed at that handle —
    /// callers can use the return value to detect accidental
    /// overwrites of built-ins (which they should treat as a bug,
    /// not a feature).
    pub fn insert(&mut self, handle: MeshHandle, mesh: MeshGpu) -> Option<MeshGpu> {
        self.meshes
            .insert(
                handle,
                MeshEntry {
                    gpu: mesh,
                    source: None,
                },
            )
            .map(|entry| entry.gpu)
    }

    /// Allocate a fresh handle past the built-in reserved range
    /// and insert `mesh` under it. The handle returned is
    /// guaranteed unique within this registry's lifetime.
    pub fn insert_new(&mut self, mesh: MeshGpu) -> MeshHandle {
        let handle = self.allocate_handle();
        self.meshes.insert(
            handle,
            MeshEntry {
                gpu: mesh,
                source: None,
            },
        );
        handle
    }

    /// Upload already-parsed CPU mesh data under a fresh handle. Unlike
    /// [`MeshRegistry::load_gltf`] this records no source path, so the F5
    /// reload walk skips it — used for meshes that came bundled inside a
    /// glb via `load_gltf_model`, where the source is the glb (not a
    /// standalone mesh file) and hot-reload of bundled assets is deferred
    /// to the Game 2A asset pipeline.
    pub fn insert_mesh_data(&mut self, device: &Device, label: &str, data: &MeshData) -> MeshHandle {
        let gpu = MeshGpu::upload(device, label, data);
        self.insert_new(gpu)
    }

    /// Parse a glTF mesh from disk, upload it, and register under a
    /// fresh handle. The path is remembered so Step 1.F.4's F5
    /// manual reload can re-read this entry in place.
    pub fn load_gltf(
        &mut self,
        device: &Device,
        path: impl AsRef<Path>,
    ) -> AssetResult<MeshHandle> {
        let path = path.as_ref();
        let data = asset::load_gltf_mesh(path)?;
        let label = format!("gltf:{}", path.display());
        let gpu = MeshGpu::upload(device, &label, &data);
        let handle = self.allocate_handle();
        self.meshes.insert(
            handle,
            MeshEntry {
                gpu,
                source: Some(path.to_path_buf()),
            },
        );
        Ok(handle)
    }

    /// Re-read every disk-sourced mesh from its tracked path and
    /// replace the GPU buffers in place under the same handle. The F5
    /// manual-reload path (Step 1.F.4).
    ///
    /// A snapshot of `(handle, path)` pairs is taken up front so the
    /// re-read / upload / replace loop doesn't hold an iteration borrow
    /// on the map it mutates. Built-ins (cube / plane, `source = None`)
    /// are skipped by the filter. Each entry is independent: a malformed
    /// glTF logs a `warn` and leaves that entry's previous `MeshGpu`
    /// intact, then the loop carries on to the next — one broken file
    /// never blocks the rest of the reload.
    pub fn reload_all(&mut self, device: &Device) -> ReloadReport<MeshHandle> {
        let targets: Vec<(MeshHandle, PathBuf)> = self
            .meshes
            .iter()
            .filter_map(|(handle, entry)| entry.source.as_ref().map(|p| (*handle, p.clone())))
            .collect();

        let mut report = ReloadReport::default();
        for (handle, path) in targets {
            match asset::load_gltf_mesh(&path) {
                Ok(data) => {
                    let label = format!("gltf:{}", path.display());
                    let gpu = MeshGpu::upload(device, &label, &data);
                    if let Some(entry) = self.meshes.get_mut(&handle) {
                        // Same handle, new buffers, source path
                        // preserved so the next reload finds it again.
                        entry.gpu = gpu;
                    }
                    report.reloaded.push(handle);
                }
                Err(err) => {
                    log::warn!(
                        "F5 reload: mesh {handle:?} from {} failed: {err}",
                        path.display()
                    );
                    report.failed += 1;
                }
            }
        }
        report
    }

    pub fn get(&self, handle: MeshHandle) -> Option<&MeshGpu> {
        self.meshes.get(&handle).map(|entry| &entry.gpu)
    }

    pub fn contains(&self, handle: MeshHandle) -> bool {
        self.meshes.contains_key(&handle)
    }

    pub fn len(&self) -> usize {
        self.meshes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.meshes.is_empty()
    }

    /// Source path for `handle` if it was loaded from disk. Built-ins
    /// return `None`. The F5 manual-reload system (Step 1.F.4) uses
    /// this to discover which entries should be re-read.
    pub fn source(&self, handle: MeshHandle) -> Option<&Path> {
        self.meshes
            .get(&handle)
            .and_then(|entry| entry.source.as_deref())
    }

    fn allocate_handle(&mut self) -> MeshHandle {
        let handle = MeshHandle(self.next_user_handle);
        self.next_user_handle = self
            .next_user_handle
            .checked_add(1)
            .expect("MeshHandle u32 space exhausted");
        handle
    }
}

#[derive(Debug)]
struct TextureEntry {
    gpu: TextureGpu,
    source: Option<PathBuf>,
}

/// Handle → GPU texture.
///
/// Parallel-shaped to [`MeshRegistry`]: built-in WHITE at handle 0,
/// user-loaded textures past `FIRST_USER`, disk-loaded entries carry
/// their source path for F5 manual reload.
#[derive(Debug)]
pub struct TextureRegistry {
    textures: HashMap<TextureHandle, TextureEntry>,
    next_user_handle: u32,
}

impl TextureRegistry {
    /// Empty registry. Useful for tests; production code goes
    /// through [`TextureRegistry::with_builtins`].
    pub fn empty() -> Self {
        Self {
            textures: HashMap::new(),
            next_user_handle: TextureHandle::FIRST_USER.0,
        }
    }

    /// Build the registry and upload the engine-owned WHITE 1×1
    /// texel at the reserved built-in slot. Called once during
    /// `App::resumed` after `RenderContext` is up.
    pub fn with_builtins(device: &Device, queue: &Queue) -> Self {
        let mut registry = Self::empty();

        let white = TextureGpu::upload_rgba8(device, queue, "builtin-white", &TextureData::white_1x1());
        registry.textures.insert(
            TextureHandle::WHITE,
            TextureEntry {
                gpu: white,
                source: None,
            },
        );

        // Flat normal goes through the *linear* path — it's data, not
        // color (see `upload_rgba8_linear`).
        let flat_normal = TextureGpu::upload_rgba8_linear(
            device,
            queue,
            "builtin-flat-normal",
            &TextureData::flat_normal_1x1(),
        );
        registry.textures.insert(
            TextureHandle::FLAT_NORMAL,
            TextureEntry {
                gpu: flat_normal,
                source: None,
            },
        );

        registry
    }

    /// Decode a PNG from disk, upload it, and register under a fresh
    /// handle. The path is remembered so Step 1.F.4's F5 manual
    /// reload can re-read this entry in place.
    pub fn load_png(
        &mut self,
        device: &Device,
        queue: &Queue,
        path: impl AsRef<Path>,
    ) -> AssetResult<TextureHandle> {
        let path = path.as_ref();
        let data = asset::load_png_pixels(path)?;
        let label = format!("png:{}", path.display());
        let gpu = TextureGpu::upload_rgba8(device, queue, &label, &data);
        let handle = self.allocate_handle();
        self.textures.insert(
            handle,
            TextureEntry {
                gpu,
                source: Some(path.to_path_buf()),
            },
        );
        Ok(handle)
    }

    /// Decode a PNG from disk and upload it through the *linear*
    /// (`Rgba8Unorm`) path — for **data** textures (normal maps, mask
    /// maps) loaded loose from disk rather than out of a glb. Routing a
    /// normal map through `load_png` would apply a spurious sRGB→linear
    /// curve and bend every sampled normal.
    ///
    /// Records `source: None` — F5 reload re-reads every tracked entry
    /// via the sRGB path, which would corrupt a linear texture's
    /// colorspace, so data textures opt out of hot-reload for now (the
    /// same deferral as glb-bundled textures; revisits with the Game 2A
    /// asset pipeline, where colorspace becomes per-entry metadata).
    pub fn load_png_linear(
        &mut self,
        device: &Device,
        queue: &Queue,
        path: impl AsRef<Path>,
    ) -> AssetResult<TextureHandle> {
        let path = path.as_ref();
        let data = asset::load_png_pixels(path)?;
        let label = format!("png-linear:{}", path.display());
        let gpu = TextureGpu::upload_rgba8_linear(device, queue, &label, &data);
        Ok(self.insert_gpu(gpu))
    }

    /// Upload already-decoded RGBA8 texture data under a fresh handle.
    /// The texture-side twin of [`MeshRegistry::insert_mesh_data`]:
    /// records no source path (F5 reload skips it), used for base-color
    /// images pulled out of a glb by `load_gltf_model`. Hot-reload of
    /// glb-bundled textures is deferred to the Game 2A asset pipeline.
    pub fn insert_texture_data(
        &mut self,
        device: &Device,
        queue: &Queue,
        label: &str,
        data: &TextureData,
    ) -> TextureHandle {
        let gpu = TextureGpu::upload_rgba8(device, queue, label, data);
        self.insert_gpu(gpu)
    }

    /// As [`TextureRegistry::insert_texture_data`] but uploads through
    /// the *linear* (`Rgba8Unorm`) path — for normal maps and other data
    /// textures pulled out of a glb by `load_gltf_model`. Routing a
    /// normal map through the sRGB `insert_texture_data` would bend every
    /// sampled normal; the two entry points keep that distinction
    /// impossible to get wrong at the call site.
    pub fn insert_texture_data_linear(
        &mut self,
        device: &Device,
        queue: &Queue,
        label: &str,
        data: &TextureData,
    ) -> TextureHandle {
        let gpu = TextureGpu::upload_rgba8_linear(device, queue, label, data);
        self.insert_gpu(gpu)
    }

    fn insert_gpu(&mut self, gpu: TextureGpu) -> TextureHandle {
        let handle = self.allocate_handle();
        self.textures.insert(
            handle,
            TextureEntry {
                gpu,
                source: None,
            },
        );
        handle
    }

    /// Re-read every disk-sourced texture from its tracked path and
    /// replace the GPU texture in place under the same handle. The F5
    /// manual-reload path (Step 1.F.4).
    ///
    /// Same snapshot-then-mutate shape as [`MeshRegistry::reload_all`].
    /// The returned `reloaded` list matters more here than for meshes:
    /// the forward pipeline caches one bind group per `TextureHandle`
    /// against the old `TextureView`, so the caller must invalidate
    /// each reloaded handle's cached bind group — otherwise the next
    /// frame keeps sampling the pre-reload texture. A failed decode
    /// leaves the previous `TextureGpu` (and its still-valid cached
    /// bind group) untouched.
    pub fn reload_all(&mut self, device: &Device, queue: &Queue) -> ReloadReport<TextureHandle> {
        let targets: Vec<(TextureHandle, PathBuf)> = self
            .textures
            .iter()
            .filter_map(|(handle, entry)| entry.source.as_ref().map(|p| (*handle, p.clone())))
            .collect();

        let mut report = ReloadReport::default();
        for (handle, path) in targets {
            match asset::load_png_pixels(&path) {
                Ok(data) => {
                    let label = format!("png:{}", path.display());
                    let gpu = TextureGpu::upload_rgba8(device, queue, &label, &data);
                    if let Some(entry) = self.textures.get_mut(&handle) {
                        entry.gpu = gpu;
                    }
                    report.reloaded.push(handle);
                }
                Err(err) => {
                    log::warn!(
                        "F5 reload: texture {handle:?} from {} failed: {err}",
                        path.display()
                    );
                    report.failed += 1;
                }
            }
        }
        report
    }

    pub fn get(&self, handle: TextureHandle) -> Option<&TextureGpu> {
        self.textures.get(&handle).map(|entry| &entry.gpu)
    }

    pub fn contains(&self, handle: TextureHandle) -> bool {
        self.textures.contains_key(&handle)
    }

    pub fn len(&self) -> usize {
        self.textures.len()
    }

    pub fn is_empty(&self) -> bool {
        self.textures.is_empty()
    }

    /// Source path for `handle` if it was loaded from disk. Built-ins
    /// return `None`.
    pub fn source(&self, handle: TextureHandle) -> Option<&Path> {
        self.textures
            .get(&handle)
            .and_then(|entry| entry.source.as_deref())
    }

    fn allocate_handle(&mut self) -> TextureHandle {
        let handle = TextureHandle(self.next_user_handle);
        self.next_user_handle = self
            .next_user_handle
            .checked_add(1)
            .expect("TextureHandle u32 space exhausted");
        handle
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_mesh_registry_starts_blank() {
        let r = MeshRegistry::empty();
        assert!(r.is_empty());
        assert!(r.get(MeshHandle::CUBE).is_none());
        assert!(r.get(MeshHandle::PLANE).is_none());
    }

    #[test]
    fn mesh_insert_new_skips_builtin_range() {
        let mut r = MeshRegistry::empty();
        assert_eq!(r.next_user_handle, MeshHandle::FIRST_USER.0);
        let h0 = r.allocate_handle();
        let h1 = r.allocate_handle();
        assert_eq!(h0, MeshHandle::FIRST_USER);
        assert_eq!(h1.0, MeshHandle::FIRST_USER.0 + 1);
    }

    #[test]
    fn mesh_source_is_none_for_unknown_handle() {
        let r = MeshRegistry::empty();
        assert!(r.source(MeshHandle::FIRST_USER).is_none());
    }

    #[test]
    fn empty_texture_registry_starts_blank() {
        let r = TextureRegistry::empty();
        assert!(r.is_empty());
        assert!(r.get(TextureHandle::WHITE).is_none());
    }

    #[test]
    fn texture_allocator_skips_builtin_slot() {
        let mut r = TextureRegistry::empty();
        assert_eq!(r.next_user_handle, TextureHandle::FIRST_USER.0);
        assert_eq!(TextureHandle::FIRST_USER.0, 2); // WHITE + FLAT_NORMAL reserved
        let h0 = r.allocate_handle();
        let h1 = r.allocate_handle();
        assert_eq!(h0, TextureHandle::FIRST_USER);
        assert_eq!(h1.0, TextureHandle::FIRST_USER.0 + 1);
    }

    #[test]
    fn texture_source_is_none_for_unknown_handle() {
        let r = TextureRegistry::empty();
        assert!(r.source(TextureHandle::FIRST_USER).is_none());
    }

    // `reload_all` itself is GPU-bound (it uploads through a `Device`)
    // and is exercised by the Step 1.F.5 smoke test, matching how the
    // loaders' upload paths are validated — there is no headless test
    // device in this crate. What we can guard here is the hand-rolled
    // `Default`, since a typo there (e.g. `failed: 1`) would make every
    // reload pass start out already reporting a phantom failure.
    #[test]
    fn reload_report_default_is_empty() {
        let report = ReloadReport::<MeshHandle>::default();
        assert!(report.reloaded.is_empty());
        assert_eq!(report.failed, 0);
    }
}
