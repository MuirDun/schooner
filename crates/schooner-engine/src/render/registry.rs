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
use std::sync::{Arc, Mutex};

use wgpu::{Device, Queue};

use crate::asset::{self, AssetResult};
use crate::render::mesh::{
    MeshData, MeshFreeList, MeshGpu, MeshHandle, RawMeshId, cube_mesh, plane_mesh,
};
use crate::render::texture::{
    RawTextureId, TextureData, TextureFreeList, TextureGpu, TextureHandle,
};

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

/// [`RawMeshId`] → GPU mesh.
///
/// Insertions go through [`MeshRegistry::insert_new`] or
/// [`MeshRegistry::load_gltf`] and hand back an owning [`MeshHandle`];
/// the renderer's frame loop only ever calls [`MeshRegistry::get`] with
/// a [`RawMeshId`], so the read path stays cheap (one hash lookup per
/// draw call). Eviction is by ownership: when the last [`MeshHandle`] for
/// an id drops, the id lands on `free`, and [`MeshRegistry::drain_dead`]
/// removes the entry on the next frame. Built-in ids never drop.
#[derive(Debug)]
pub struct MeshRegistry {
    meshes: HashMap<RawMeshId, MeshEntry>,
    next_user_id: u32,
    /// Ids whose last owning handle has dropped, drained each frame.
    /// Cloned into every handle so `Drop` can enqueue without the
    /// registry. See [`MeshFreeList`].
    free: MeshFreeList,
}

impl MeshRegistry {
    /// Empty registry. Useful for tests; production code goes
    /// through [`MeshRegistry::with_builtins`].
    pub fn empty() -> Self {
        Self {
            meshes: HashMap::new(),
            next_user_id: RawMeshId::FIRST_USER.0,
            free: Arc::new(Mutex::new(Vec::new())),
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
            RawMeshId::CUBE,
            MeshEntry {
                gpu: cube,
                source: None,
            },
        );
        registry.meshes.insert(
            RawMeshId::PLANE,
            MeshEntry {
                gpu: plane,
                source: None,
            },
        );
        registry
    }

    /// An owning handle to the built-in unit cube. Cheap (one `Arc`
    /// alloc); built-in ids are never evicted, so handing out fresh
    /// handles for them is always safe.
    pub fn cube(&self) -> MeshHandle {
        self.make_handle(RawMeshId::CUBE)
    }

    /// An owning handle to the built-in unit plane. See [`Self::cube`].
    pub fn plane(&self) -> MeshHandle {
        self.make_handle(RawMeshId::PLANE)
    }

    /// Mint an owning handle for `id`, wiring it to this registry's free
    /// list so the handle's `Drop` enqueues the id for teardown.
    fn make_handle(&self, id: RawMeshId) -> MeshHandle {
        MeshHandle::new(id, Arc::clone(&self.free))
    }

    /// Allocate a fresh id past the built-in reserved range, insert
    /// `mesh` under it, and return an owning [`MeshHandle`]. The id is
    /// unique within this registry's lifetime (ids are never reused).
    pub fn insert_new(&mut self, mesh: MeshGpu) -> MeshHandle {
        let id = self.allocate_id();
        self.meshes.insert(
            id,
            MeshEntry {
                gpu: mesh,
                source: None,
            },
        );
        self.make_handle(id)
    }

    /// Upload already-parsed CPU mesh data under a fresh handle. Unlike
    /// [`MeshRegistry::load_gltf`] this records no source path, so the F5
    /// reload walk skips it — used for meshes that came bundled inside a
    /// glb via `load_gltf_model`, where the source is the glb (not a
    /// standalone mesh file) and hot-reload of bundled assets is deferred
    /// to the Game 2A asset pipeline.
    pub fn insert_mesh_data(
        &mut self,
        device: &Device,
        label: &str,
        data: &MeshData,
    ) -> MeshHandle {
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
        let id = self.allocate_id();
        self.meshes.insert(
            id,
            MeshEntry {
                gpu,
                source: Some(path.to_path_buf()),
            },
        );
        Ok(self.make_handle(id))
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
    pub fn reload_all(&mut self, device: &Device) -> ReloadReport<RawMeshId> {
        let targets: Vec<(RawMeshId, PathBuf)> = self
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

    /// Tear down every mesh whose last owning [`MeshHandle`] has dropped
    /// since the previous call: remove the entry, which drops its
    /// [`MeshGpu`] (wgpu frees the buffers once in-flight frames release
    /// them). Ids are monotonic and never reused, so a queued id is
    /// always a genuine last-drop. Called once per frame from
    /// `render_frame`.
    ///
    /// Built-in ids are never queued (their handle `Drop` skips them),
    /// but the `>= FIRST_USER` guard is kept as a backstop. Meshes carry
    /// no bind-group cache, so removal is the whole teardown — unlike
    /// [`TextureRegistry::drain_dead`], which also invalidates the
    /// material cache.
    pub fn drain_dead(&mut self) {
        let dead = match self.free.lock() {
            Ok(mut queue) => std::mem::take(&mut *queue),
            Err(_) => return,
        };
        for id in dead {
            if id.0 >= RawMeshId::FIRST_USER.0 {
                self.meshes.remove(&id);
            }
        }
    }

    pub fn get(&self, id: RawMeshId) -> Option<&MeshGpu> {
        self.meshes.get(&id).map(|entry| &entry.gpu)
    }

    pub fn contains(&self, id: RawMeshId) -> bool {
        self.meshes.contains_key(&id)
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
    pub fn source(&self, id: RawMeshId) -> Option<&Path> {
        self.meshes
            .get(&id)
            .and_then(|entry| entry.source.as_deref())
    }

    fn allocate_id(&mut self) -> RawMeshId {
        let id = RawMeshId(self.next_user_id);
        self.next_user_id = self
            .next_user_id
            .checked_add(1)
            .expect("RawMeshId u32 space exhausted");
        id
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
    textures: HashMap<RawTextureId, TextureEntry>,
    next_user_id: u32,
    /// Ids whose last owning handle has dropped, drained each frame. See
    /// [`TextureFreeList`] and [`TextureRegistry::drain_dead`].
    free: TextureFreeList,
}

impl TextureRegistry {
    /// Empty registry. Useful for tests; production code goes
    /// through [`TextureRegistry::with_builtins`].
    pub fn empty() -> Self {
        Self {
            textures: HashMap::new(),
            next_user_id: RawTextureId::FIRST_USER.0,
            free: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Build the registry and upload the engine-owned WHITE 1×1
    /// texel at the reserved built-in slot. Called once during
    /// `App::resumed` after `RenderContext` is up.
    pub fn with_builtins(device: &Device, queue: &Queue) -> Self {
        let mut registry = Self::empty();

        let white =
            TextureGpu::upload_rgba8(device, queue, "builtin-white", &TextureData::white_1x1());
        registry.textures.insert(
            RawTextureId::WHITE,
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
            RawTextureId::FLAT_NORMAL,
            TextureEntry {
                gpu: flat_normal,
                source: None,
            },
        );

        registry
    }

    /// Mint an owning handle for `id`, wiring it to this registry's free
    /// list so the handle's `Drop` enqueues the id for teardown.
    fn make_handle(&self, id: RawTextureId) -> TextureHandle {
        TextureHandle::new(id, Arc::clone(&self.free))
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
        let id = self.allocate_id();
        self.textures.insert(
            id,
            TextureEntry {
                gpu,
                source: Some(path.to_path_buf()),
            },
        );
        Ok(self.make_handle(id))
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
        let id = self.allocate_id();
        self.textures.insert(id, TextureEntry { gpu, source: None });
        self.make_handle(id)
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
    pub fn reload_all(&mut self, device: &Device, queue: &Queue) -> ReloadReport<RawTextureId> {
        let targets: Vec<(RawTextureId, PathBuf)> = self
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

    /// Tear down every texture whose last owning [`TextureHandle`] has
    /// dropped since the previous call, and **return the freed ids** so
    /// the caller can invalidate any forward-pipeline material bind group
    /// that cached them. That second step is essential: a cached bind
    /// group holds its own clone of the texture's view, so dropping the
    /// registry entry alone would leave the GPU texture pinned and free
    /// nothing. Removal here drops the [`TextureGpu`]; the matching
    /// `invalidate_material_bind_group` call drops the last view clone,
    /// and wgpu frees the allocation once in-flight frames release it.
    ///
    /// Built-in ids are never queued; the `>= FIRST_USER` guard is a
    /// backstop. Called once per frame from `render_frame`.
    pub fn drain_dead(&mut self) -> Vec<RawTextureId> {
        let dead = match self.free.lock() {
            Ok(mut queue) => std::mem::take(&mut *queue),
            Err(_) => return Vec::new(),
        };
        let mut freed = Vec::new();
        for id in dead {
            if id.0 >= RawTextureId::FIRST_USER.0 {
                self.textures.remove(&id);
                freed.push(id);
            }
        }
        freed
    }

    pub fn get(&self, id: RawTextureId) -> Option<&TextureGpu> {
        self.textures.get(&id).map(|entry| &entry.gpu)
    }

    pub fn contains(&self, id: RawTextureId) -> bool {
        self.textures.contains_key(&id)
    }

    pub fn len(&self) -> usize {
        self.textures.len()
    }

    pub fn is_empty(&self) -> bool {
        self.textures.is_empty()
    }

    /// Source path for `handle` if it was loaded from disk. Built-ins
    /// return `None`.
    pub fn source(&self, id: RawTextureId) -> Option<&Path> {
        self.textures
            .get(&id)
            .and_then(|entry| entry.source.as_deref())
    }

    fn allocate_id(&mut self) -> RawTextureId {
        let id = RawTextureId(self.next_user_id);
        self.next_user_id = self
            .next_user_id
            .checked_add(1)
            .expect("RawTextureId u32 space exhausted");
        id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_mesh_registry_starts_blank() {
        let r = MeshRegistry::empty();
        assert!(r.is_empty());
        assert!(r.get(RawMeshId::CUBE).is_none());
        assert!(r.get(RawMeshId::PLANE).is_none());
    }

    #[test]
    fn mesh_allocator_skips_builtin_range() {
        let mut r = MeshRegistry::empty();
        assert_eq!(r.next_user_id, RawMeshId::FIRST_USER.0);
        let h0 = r.allocate_id();
        let h1 = r.allocate_id();
        assert_eq!(h0, RawMeshId::FIRST_USER);
        assert_eq!(h1.0, RawMeshId::FIRST_USER.0 + 1);
    }

    #[test]
    fn mesh_source_is_none_for_unknown_id() {
        let r = MeshRegistry::empty();
        assert!(r.source(RawMeshId::FIRST_USER).is_none());
    }

    #[test]
    fn empty_texture_registry_starts_blank() {
        let r = TextureRegistry::empty();
        assert!(r.is_empty());
        assert!(r.get(RawTextureId::WHITE).is_none());
    }

    #[test]
    fn texture_allocator_skips_builtin_slot() {
        let mut r = TextureRegistry::empty();
        assert_eq!(r.next_user_id, RawTextureId::FIRST_USER.0);
        assert_eq!(RawTextureId::FIRST_USER.0, 2); // WHITE + FLAT_NORMAL reserved
        let h0 = r.allocate_id();
        let h1 = r.allocate_id();
        assert_eq!(h0, RawTextureId::FIRST_USER);
        assert_eq!(h1.0, RawTextureId::FIRST_USER.0 + 1);
    }

    #[test]
    fn texture_source_is_none_for_unknown_id() {
        let r = TextureRegistry::empty();
        assert!(r.source(RawTextureId::FIRST_USER).is_none());
    }

    // `reload_all` itself is GPU-bound (it uploads through a `Device`)
    // and is exercised by the Step 1.F.5 smoke test, matching how the
    // loaders' upload paths are validated — there is no headless test
    // device in this crate. What we can guard here is the hand-rolled
    // `Default`, since a typo there (e.g. `failed: 1`) would make every
    // reload pass start out already reporting a phantom failure.
    #[test]
    fn reload_report_default_is_empty() {
        let report = ReloadReport::<RawMeshId>::default();
        assert!(report.reloaded.is_empty());
        assert_eq!(report.failed, 0);
    }
}
