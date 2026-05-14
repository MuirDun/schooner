use std::any::{Any, TypeId};
use std::collections::HashMap;

/// Type-keyed singleton bag for things the `World` owns outside the
/// entity/component graph — clocks, input state, render contexts,
/// configuration.
///
/// Each `R` is stored at most once; re-inserting replaces and returns the
/// prior value. Requires `Send + Sync` so worlds remain shareable across
/// threads; non-`Send` globals will get a separate bucket if/when we need
/// them.
#[derive(Default)]
pub struct Resources {
    map: HashMap<TypeId, Box<dyn Any + Send + Sync>>,
}

impl Resources {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Insert or replace the single instance of `R`. Returns the prior
    /// value, if any.
    pub fn insert<R: Any + Send + Sync>(&mut self, value: R) -> Option<R> {
        self.map
            .insert(TypeId::of::<R>(), Box::new(value))
            .map(|prev| {
                *prev
                    .downcast::<R>()
                    .expect("TypeId keyed the resource map by R")
            })
    }

    pub fn remove<R: Any + Send + Sync>(&mut self) -> Option<R> {
        self.map.remove(&TypeId::of::<R>()).map(|value| {
            *value
                .downcast::<R>()
                .expect("TypeId keyed the resource map by R")
        })
    }

    pub fn get<R: Any + Send + Sync>(&self) -> Option<&R> {
        self.map
            .get(&TypeId::of::<R>())
            .and_then(|v| v.downcast_ref::<R>())
    }

    pub fn get_mut<R: Any + Send + Sync>(&mut self) -> Option<&mut R> {
        self.map
            .get_mut(&TypeId::of::<R>())
            .and_then(|v| v.downcast_mut::<R>())
    }

    pub fn contains<R: Any + Send + Sync>(&self) -> bool {
        self.map.contains_key(&TypeId::of::<R>())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq)]
    struct Clock {
        tick: u64,
    }

    #[derive(Debug, PartialEq)]
    struct Config {
        name: String,
    }

    #[test]
    fn insert_then_get_roundtrips_value() {
        let mut res = Resources::new();
        assert_eq!(res.insert(Clock { tick: 7 }), None);
        assert_eq!(res.get::<Clock>(), Some(&Clock { tick: 7 }));
        assert!(res.contains::<Clock>());
        assert_eq!(res.len(), 1);
    }

    #[test]
    fn insert_replace_returns_prior_value() {
        let mut res = Resources::new();
        res.insert(Clock { tick: 1 });
        assert_eq!(res.insert(Clock { tick: 2 }), Some(Clock { tick: 1 }));
        assert_eq!(res.get::<Clock>(), Some(&Clock { tick: 2 }));
        assert_eq!(res.len(), 1);
    }

    #[test]
    fn remove_returns_value_then_none() {
        let mut res = Resources::new();
        res.insert(Clock { tick: 5 });
        assert_eq!(res.remove::<Clock>(), Some(Clock { tick: 5 }));
        assert_eq!(res.remove::<Clock>(), None);
        assert!(!res.contains::<Clock>());
    }

    #[test]
    fn get_absent_resource_returns_none() {
        let res = Resources::new();
        assert_eq!(res.get::<Clock>(), None);
    }

    #[test]
    fn get_mut_allows_in_place_mutation() {
        let mut res = Resources::new();
        res.insert(Clock { tick: 0 });
        if let Some(clock) = res.get_mut::<Clock>() {
            clock.tick = 99;
        }
        assert_eq!(res.get::<Clock>(), Some(&Clock { tick: 99 }));
    }

    #[test]
    fn distinct_resource_types_coexist() {
        let mut res = Resources::new();
        res.insert(Clock { tick: 1 });
        res.insert(Config {
            name: "void".into(),
        });
        assert_eq!(res.get::<Clock>(), Some(&Clock { tick: 1 }));
        assert_eq!(
            res.get::<Config>(),
            Some(&Config {
                name: "void".into()
            })
        );
        assert_eq!(res.len(), 2);
    }

    #[test]
    fn contains_reflects_insert_and_remove() {
        let mut res = Resources::new();
        assert!(!res.contains::<Clock>());
        res.insert(Clock { tick: 0 });
        assert!(res.contains::<Clock>());
        res.remove::<Clock>();
        assert!(!res.contains::<Clock>());
    }

    #[test]
    fn remove_unknown_resource_is_none() {
        let mut res = Resources::new();
        assert_eq!(res.remove::<Clock>(), None);
    }

    #[test]
    fn insert_after_remove_restores_resource() {
        let mut res = Resources::new();
        res.insert(Clock { tick: 1 });
        res.remove::<Clock>();
        assert_eq!(res.insert(Clock { tick: 2 }), None);
        assert_eq!(res.get::<Clock>(), Some(&Clock { tick: 2 }));
    }

    #[test]
    fn default_resources_is_empty() {
        let res = Resources::default();
        assert!(res.is_empty());
        assert_eq!(res.len(), 0);
    }
}
