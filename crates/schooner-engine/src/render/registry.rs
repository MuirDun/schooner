//! `MeshRegistry` — handle → `MeshGpu` lookup table.
//!
//! Resource form: lives in the `World`, declared by systems via
//! `Res<MeshRegistry>` (read-only path used by the renderer) or
//! `ResMut<MeshRegistry>` (write path used by asset loaders later).
//!
//! Built-ins (cube, plane) are uploaded eagerly at construction
//! through `with_builtins` — see `architecture/render.md` "What
//! the renderer owns" for why these live in the engine and not
//! in `game-void`.

use std::collections::HashMap;

use wgpu::Device;

use crate::render::mesh::{cube_mesh, plane_mesh, MeshGpu, MeshHandle};

/// Handle → GPU mesh.
///
/// Insertions go through [`MeshRegistry::insert`]; the renderer's
/// frame loop only ever calls [`MeshRegistry::get`], so the read
/// path stays cheap (one hash lookup per draw call). Game 0 has
/// no eviction — meshes live as long as the registry does.
#[derive(Debug)]
pub struct MeshRegistry {
    meshes: HashMap<MeshHandle, MeshGpu>,
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
        registry.meshes.insert(MeshHandle::CUBE, cube);
        registry.meshes.insert(MeshHandle::PLANE, plane);
        registry
    }

    /// Insert a user-supplied mesh under the given handle.
    /// Returns the prior `MeshGpu` if one existed at that handle —
    /// callers can use the return value to detect accidental
    /// overwrites of built-ins (which they should treat as a bug,
    /// not a feature).
    pub fn insert(&mut self, handle: MeshHandle, mesh: MeshGpu) -> Option<MeshGpu> {
        self.meshes.insert(handle, mesh)
    }

    /// Allocate a fresh handle past the built-in reserved range
    /// and insert `mesh` under it. The handle returned is
    /// guaranteed unique within this registry's lifetime.
    pub fn insert_new(&mut self, mesh: MeshGpu) -> MeshHandle {
        let handle = MeshHandle(self.next_user_handle);
        self.next_user_handle = self
            .next_user_handle
            .checked_add(1)
            .expect("MeshHandle u32 space exhausted");
        self.meshes.insert(handle, mesh);
        handle
    }

    pub fn get(&self, handle: MeshHandle) -> Option<&MeshGpu> {
        self.meshes.get(&handle)
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_registry_starts_blank() {
        let r = MeshRegistry::empty();
        assert!(r.is_empty());
        assert!(r.get(MeshHandle::CUBE).is_none());
        assert!(r.get(MeshHandle::PLANE).is_none());
    }

    #[test]
    fn insert_new_skips_builtin_range() {
        // The first user-allocated handle must be FIRST_USER, not
        // CUBE or PLANE — otherwise `with_builtins` followed by
        // `insert_new` would overwrite a built-in slot.
        let mut r = MeshRegistry::empty();
        // Without uploading real meshes (no Device in unit tests),
        // we can still exercise the handle allocator by skipping
        // the actual upload step and checking the allocation logic.
        assert_eq!(r.next_user_handle, MeshHandle::FIRST_USER.0);
        // Simulate two allocations.
        let h0 = MeshHandle(r.next_user_handle);
        r.next_user_handle += 1;
        let h1 = MeshHandle(r.next_user_handle);
        r.next_user_handle += 1;
        assert_eq!(h0, MeshHandle::FIRST_USER);
        assert_eq!(h1.0, MeshHandle::FIRST_USER.0 + 1);
    }
}
