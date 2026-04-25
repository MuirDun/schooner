//! Storage-handle resolution for [`QueryData::Fetch`].
//!
//! The [`Join`](crate::ecs::query::join::Join) engine yields entity
//! ids; converting those into typed `&T` / `Mut<T>` items requires
//! per-component **typed storage handles**: a `&'w SparseSet<T>` for
//! a read access, a `&'w mut SparseSet<T>` for a write. Producing
//! many of those from a single `&mut World` borrow is the part the
//! borrow checker cannot prove safe on its own — Rust's reasoning
//! stops at "you handed out N mutable borrows of `World`."
//!
//! ## The unsafe budget
//!
//! Two `unsafe` blocks total:
//!
//! 1. [`split_storages`] — given an aliasing-checked
//!    [`QueryAccess`], produces one [`StorageHandle`] per access by
//!    casting raw pointers into references with the world's
//!    lifetime. Each handle is consumed by exactly one call site
//!    downstream.
//! 2. [`handle_as_read`] / [`handle_as_write`] — extend a `&dyn` /
//!    `&mut dyn` reborrow's lifetime back to `'w`. Sound because the
//!    wrapper is consumed by value (so the inner reference is the
//!    only live one) and the storage box stays in the world for the
//!    full `'w` borrow.
//!
//! Per-entity fetch is fully safe — the unsafe is paid once at
//! query-iteration setup, not per item.

use std::any::TypeId;

use crate::ecs::query::data::{ComponentAccess, QueryAccess};
use crate::ecs::{Component, ComponentStorage, SparseSet, World};
use crate::error::EngineError;

/// One resolved storage borrow. Read or Write — the variant matches
/// the requested access mode.
///
/// **Always consumed by value** in `init_fetch` — taking this by
/// `&mut` and reborrowing the inner reference would create aliased
/// `&mut`s. Move-by-value extracts the inner reference exactly once.
pub enum StorageHandle<'w> {
    Read(&'w dyn ComponentStorage),
    Write(&'w mut dyn ComponentStorage),
}

/// Pre-flight: reject access sets that would produce aliased borrows
/// inside the unsafe split. A duplicate read is fine; a duplicate
/// where any side is mutable is not.
pub fn check_no_alias(
    access: &QueryAccess,
    name_of: impl Fn(&ComponentAccess) -> &'static str,
) -> Result<(), EngineError> {
    let comps = &access.components;
    for i in 0..comps.len() {
        for j in (i + 1)..comps.len() {
            if comps[i].component_id == comps[j].component_id
                && (comps[i].mutable || comps[j].mutable)
            {
                return Err(EngineError::QueryAliasConflict {
                    name: name_of(&comps[i]),
                });
            }
        }
    }
    Ok(())
}

/// Split the world's storages into handles for both required (data)
/// and optional (filter) access in one pass.
///
/// - `data_access`: every entry MUST have a registered storage.
///   Missing → returns `None` (the caller iterates empty).
/// - `filter_access`: each entry produces `Some(handle)` if the
///   storage exists, `None` if it doesn't. `Without<T>` treats
///   `None` as "no entity has `T`".
///
/// # Safety contract for the call site
///
/// The caller must have run [`check_no_alias`] on the *concatenated*
/// access set (`data_access` ++ `filter_access`) so no two emitted
/// handles overlap. With that guarantee:
/// - Two reads of the same storage are allowed.
/// - Two writes, or a read + write of the same storage, are
///   rejected by the alias check.
/// - Different `ComponentId`s map to different `Box`es in the
///   storages `HashMap`, so distinct accesses never touch the same
///   memory.
pub fn split_storages<'w>(
    world: &'w mut World,
    data_access: &QueryAccess,
    filter_access: &QueryAccess,
) -> Option<(Vec<StorageHandle<'w>>, Vec<Option<StorageHandle<'w>>>)> {
    // Required: every data storage must exist.
    for c in &data_access.components {
        if world.storage(c.component_id).is_none() {
            return None;
        }
    }

    // SAFETY: the alias check guarantees no two accesses overlap.
    // The world is held by `&'w mut`, so the storages `HashMap` and
    // every `Box` it owns stay pinned for `'w`. Each iteration
    // produces a single reference to a distinct `Box`'s contents at
    // the same `'w` lifetime; the resulting handles never alias.
    let world_ptr: *mut World = world;

    let mut data_handles = Vec::with_capacity(data_access.components.len());
    for c in &data_access.components {
        unsafe {
            let storage_ptr = (*world_ptr)
                .storage_box_ptr(c.component_id)
                .expect("checked above");
            data_handles.push(if c.mutable {
                StorageHandle::Write(&mut **storage_ptr)
            } else {
                StorageHandle::Read(&**storage_ptr)
            });
        }
    }

    let mut filter_handles = Vec::with_capacity(filter_access.components.len());
    for c in &filter_access.components {
        unsafe {
            let storage_ptr_opt = (*world_ptr).storage_box_ptr(c.component_id);
            filter_handles.push(storage_ptr_opt.map(|storage_ptr| {
                if c.mutable {
                    StorageHandle::Write(&mut **storage_ptr)
                } else {
                    StorageHandle::Read(&**storage_ptr)
                }
            }));
        }
    }

    Some((data_handles, filter_handles))
}

/// Consume a `StorageHandle` and produce a typed read borrow with
/// the world-borrow lifetime `'w`.
///
/// Panics if the typed downcast fails (framework bug — alias check
/// + tuple wiring should guarantee `T` matches the registered type).
///
/// # Safety reasoning
///
/// The handle is consumed by value, so its internal reference is
/// the only live borrow when we extract it. The pointed-to
/// `SparseSet<T>` lives inside a `Box` owned by `World`; that box
/// stays in place for `'w` (the caller's `&mut World` borrow). The
/// transmute restores the `'w` lifetime that downcast's
/// `&mut self`-style reborrow shortens.
pub fn handle_as_read<'w, T: Component>(handle: StorageHandle<'w>) -> &'w SparseSet<T> {
    let dyn_ref: &'w dyn ComponentStorage = match handle {
        StorageHandle::Read(s) => s,
        StorageHandle::Write(s) => {
            // SAFETY: downgrade `&'w mut` to `&'w` over the same
            // memory. The wrapper has been consumed; the original
            // mutable reference is no longer reachable.
            unsafe {
                std::mem::transmute::<&dyn ComponentStorage, &'w dyn ComponentStorage>(&*s)
            }
        }
    };
    let typed = dyn_ref
        .as_any()
        .downcast_ref::<SparseSet<T>>()
        .expect("query fetch downcast: typed mismatch (framework bug)");
    // SAFETY: see function-level doc. `typed` points at the same
    // memory as `dyn_ref` whose lifetime is `'w`; the transmute
    // restores that lifetime through `downcast_ref`'s shortening.
    unsafe { std::mem::transmute::<&SparseSet<T>, &'w SparseSet<T>>(typed) }
}

/// Consume a `StorageHandle` and produce a typed mutable borrow
/// with the world-borrow lifetime `'w`.
///
/// Panics on `StorageHandle::Read` (framework bug — wrapper builder
/// should never produce a read handle for a write-typed slot).
pub fn handle_as_write<'w, T: Component>(handle: StorageHandle<'w>) -> &'w mut SparseSet<T> {
    let dyn_mut: &'w mut dyn ComponentStorage = match handle {
        StorageHandle::Write(s) => s,
        StorageHandle::Read(_) => {
            panic!("query fetch: requested write handle but got read (framework bug)")
        }
    };
    let typed = dyn_mut
        .as_any_mut()
        .downcast_mut::<SparseSet<T>>()
        .expect("query fetch downcast: typed mismatch (framework bug)");
    // SAFETY: same lifetime extension reasoning as `handle_as_read`.
    unsafe { std::mem::transmute::<&mut SparseSet<T>, &'w mut SparseSet<T>>(typed) }
}

/// Resolve a `ComponentAccess`'s component type name from the world
/// registry, for the alias-check error message.
pub fn name_of_component(world: &World, access: &ComponentAccess) -> &'static str {
    world
        .component_name(access.component_id)
        .unwrap_or(std::any::type_name::<TypeId>())
}
