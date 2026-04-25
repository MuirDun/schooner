use std::any::{TypeId, type_name};
use std::collections::HashMap;

/// Marker trait for types that can be used as ECS components.
///
/// Enbaled for parallelism
pub trait Component: 'static + Send + Sync {}
impl<T: 'static + Send + Sync> Component for T {}

/// Dense numeric identifier for a component type.
///
/// Only meaningful within the [`ComponentRegistry`] that issued it.
/// Backed by `u32` so it indexes densely into per-World storage tables.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ComponentId(u32);

impl ComponentId {
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// Per-World table that interns Rust `TypeId`s into dense [`ComponentId`]
/// handles, and remembers each type's name for diagnostics.
#[derive(Debug, Default)]
pub struct ComponentRegistry {
    ids: HashMap<TypeId, ComponentId>,
    names: Vec<&'static str>,
}

impl ComponentRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Return the id for `T`, assigning one on first use.
    pub fn register<T: Component>(&mut self) -> ComponentId {
        let type_id = TypeId::of::<T>();
        if let Some(&existing) = self.ids.get(&type_id) {
            return existing;
        }
        let id = ComponentId(self.names.len() as u32);
        self.names.push(type_name::<T>());
        self.ids.insert(type_id, id);
        id
    }

    /// Return the id for `T` only if it has already been registered.
    pub fn id_of<T: Component>(&self) -> Option<ComponentId> {
        self.ids.get(&TypeId::of::<T>()).copied()
    }

    pub fn name(&self, id: ComponentId) -> Option<&'static str> {
        self.names.get(id.index()).copied()
    }

    pub fn len(&self) -> usize {
        self.names.len()
    }

    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Transform;
    struct Velocity;
    struct Health(#[allow(dead_code)] i32);

    #[test]
    fn component_trait_is_blanket_for_static_send_sync() {
        fn accepts_component<T: Component>() {}
        accepts_component::<i32>();
        accepts_component::<String>();
        accepts_component::<Transform>();
    }

    #[test]
    fn register_is_idempotent_for_same_type() {
        let mut reg = ComponentRegistry::new();
        let a = reg.register::<Transform>();
        let b = reg.register::<Transform>();
        assert_eq!(a, b);
    }

    #[test]
    fn different_types_get_different_ids() {
        let mut reg = ComponentRegistry::new();
        let t = reg.register::<Transform>();
        let v = reg.register::<Velocity>();
        let h = reg.register::<Health>();
        assert_ne!(t, v);
        assert_ne!(v, h);
        assert_ne!(t, h);
    }

    #[test]
    fn ids_are_dense_from_zero() {
        let mut reg = ComponentRegistry::new();
        assert_eq!(reg.register::<Transform>().index(), 0);
        assert_eq!(reg.register::<Velocity>().index(), 1);
        assert_eq!(reg.register::<Health>().index(), 2);
    }

    #[test]
    fn id_of_returns_none_before_register() {
        let reg = ComponentRegistry::new();
        assert_eq!(reg.id_of::<Transform>(), None);
    }

    #[test]
    fn id_of_returns_some_after_register() {
        let mut reg = ComponentRegistry::new();
        let id = reg.register::<Transform>();
        assert_eq!(reg.id_of::<Transform>(), Some(id));
    }

    #[test]
    fn name_returns_type_name_for_registered_id() {
        let mut reg = ComponentRegistry::new();
        let id = reg.register::<Transform>();
        let name = reg.name(id).expect("registered id has a name");
        assert!(name.ends_with("::Transform"), "got {name}");
    }

    #[test]
    fn len_tracks_registered_count_and_is_idempotent() {
        let mut reg = ComponentRegistry::new();
        assert_eq!(reg.len(), 0);
        assert!(reg.is_empty());

        reg.register::<Transform>();
        assert_eq!(reg.len(), 1);

        reg.register::<Transform>();
        assert_eq!(reg.len(), 1);

        reg.register::<Velocity>();
        assert_eq!(reg.len(), 2);
    }
}
