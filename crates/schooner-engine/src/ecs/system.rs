//! `SystemParam` machinery + `FunctionSystem` dispatcher.
//!
//! ## Why each system holds its own `ParamAccess` set
//!
//! Earlier we got away with `SystemParam` only fetching a single
//! resource slot — disjoint resource access per param was the only
//! conflict to worry about. C9.6 introduces `Query<D, F>`, which
//! takes the *whole* `&mut World` and reads/writes a (potentially
//! large) component set. The per-system pre-check now has two shapes
//! to validate:
//!
//! - **Resource conflicts**: the same resource type must not appear
//!   twice in one system's params (even as `Res` + `ResMut`). This
//!   is the existing rule.
//! - **Component conflicts**: two queries in the same system whose
//!   component access overlaps with at least one mutable side are
//!   rejected. Same rule as the inside-one-query alias check, lifted
//!   to the system scope.
//!
//! Both fire at **registration** (`Schedule::add_system`) so the
//! panic surface is at setup, not at the first frame.
//!
//! ## The unsafe sequential-fetch
//!
//! `FunctionSystem<_, (A, B)>::run` needs to give both `A::fetch` and
//! `B::fetch` `&'w mut World` access at the same time so their
//! returned items can coexist for the function call. Rust can't
//! prove the disjointness from the source — we proved it once at
//! registration via the access check. Hand out N sequential `&mut`
//! borrows through a raw-pointer round-trip, with SAFETY: see the
//! `run` method comments.

use std::any::{Any, TypeId, type_name};
use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};

use crate::ecs::World;
use crate::ecs::query::data::{ComponentAccess, QueryAccess};
use crate::error::EngineError;

// --- resource smart pointers ---------------------------------------------

/// Shared access to a resource, delivered to systems as a parameter.
pub struct Res<'w, T: 'static> {
    value: &'w T,
    _marker: PhantomData<fn() -> T>,
}

impl<'w, T: 'static> Deref for Res<'w, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.value
    }
}

/// Exclusive access to a resource, delivered to systems as a parameter.
pub struct ResMut<'w, T: 'static> {
    value: &'w mut T,
    _marker: PhantomData<fn() -> T>,
}

impl<'w, T: 'static> Deref for ResMut<'w, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &*self.value
    }
}

impl<'w, T: 'static> DerefMut for ResMut<'w, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut *self.value
    }
}

// --- access descriptors --------------------------------------------------

/// Resource access descriptor — `TypeId` plus mutability bit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResourceAccess {
    pub type_id: TypeId,
    pub type_name: &'static str,
    pub mutable: bool,
}

/// What a single `SystemParam` reads or writes from the world.
///
/// Resource and component access live in separate buckets — they
/// have different conflict semantics (resources alias by `TypeId`,
/// components by `ComponentId`). The per-system conflict check runs
/// each bucket independently.
#[derive(Clone, Debug, Default)]
pub struct ParamAccess {
    pub resources: Vec<ResourceAccess>,
    pub components: Vec<ComponentAccess>,
}

impl ParamAccess {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn extend(&mut self, other: ParamAccess) {
        self.resources.extend(other.resources);
        self.components.extend(other.components);
    }
}

// --- SystemParam ---------------------------------------------------------

/// A value that can be pulled out of a [`World`] to feed a system
/// parameter.
///
/// `access(world)` is called once at system registration; it may
/// auto-register component types so any IDs it returns are stable
/// for the system's lifetime. The returned `ParamAccess` is unioned
/// across all the system's params and validated for conflicts.
///
/// `fetch(world)` is the per-tick hot path. It is `unsafe` because
/// `FunctionSystem` hands out N sequential `&mut World` borrows
/// through a raw pointer to satisfy multi-param signatures; the
/// caller is responsible for having already validated that the
/// param's access does not conflict with concurrently-live params.
pub trait SystemParam {
    type Item<'w>;

    /// One-time access description. May register component types.
    fn access(world: &mut World) -> ParamAccess;

    /// Per-tick fetch. Returns an item bound to the world borrow.
    ///
    /// # Safety
    ///
    /// The caller must have run the per-system access check (see
    /// [`check_param_conflicts`]) and not have any other live
    /// `Item` from a conflicting param. `FunctionSystem::run`
    /// satisfies this contract.
    unsafe fn fetch<'w>(world: &'w mut World) -> Self::Item<'w>;
}

impl<T: Any + Send + Sync> SystemParam for Res<'_, T> {
    type Item<'w> = Res<'w, T>;

    fn access(_world: &mut World) -> ParamAccess {
        let mut access = ParamAccess::new();
        access.resources.push(ResourceAccess {
            type_id: TypeId::of::<T>(),
            type_name: type_name::<T>(),
            mutable: false,
        });
        access
    }

    unsafe fn fetch<'w>(world: &'w mut World) -> Self::Item<'w> {
        let value = world.resource::<T>().unwrap_or_else(|| {
            panic!(
                "{}",
                EngineError::MissingResource {
                    name: type_name::<T>()
                }
            )
        });
        // SAFETY: `value: &'_ T` reborrowed from `&world`. The
        // caller has guaranteed no conflicting param is live, so
        // extending the borrow lifetime to `'w` (the world borrow's
        // lifetime) is sound — nothing else will mutate this
        // resource for the duration of `'w`.
        let value: &'w T = unsafe { std::mem::transmute::<&T, &'w T>(value) };
        Res {
            value,
            _marker: PhantomData,
        }
    }
}

impl<T: Any + Send + Sync> SystemParam for ResMut<'_, T> {
    type Item<'w> = ResMut<'w, T>;

    fn access(_world: &mut World) -> ParamAccess {
        let mut access = ParamAccess::new();
        access.resources.push(ResourceAccess {
            type_id: TypeId::of::<T>(),
            type_name: type_name::<T>(),
            mutable: true,
        });
        access
    }

    unsafe fn fetch<'w>(world: &'w mut World) -> Self::Item<'w> {
        let value = world.resource_mut::<T>().unwrap_or_else(|| {
            panic!(
                "{}",
                EngineError::MissingResource {
                    name: type_name::<T>()
                }
            )
        });
        // SAFETY: see `Res::fetch` — caller has validated no
        // concurrent access to this resource.
        let value: &'w mut T = unsafe { std::mem::transmute::<&mut T, &'w mut T>(value) };
        ResMut {
            value,
            _marker: PhantomData,
        }
    }
}

// --- per-system conflict check -------------------------------------------

/// Reject a system whose params conflict with each other.
///
/// Two checks: resources by `TypeId`, components by `ComponentId`.
/// The component check uses the same rule as the inside-one-query
/// alias check — duplicates are fine if all reads, illegal if any
/// write.
pub(crate) fn check_param_conflicts(
    accesses: &[ParamAccess],
    component_name: impl Fn(&ComponentAccess) -> &'static str,
) -> Result<(), EngineError> {
    // Resource conflicts: any duplicate `TypeId` is rejected,
    // matching the legacy disjoint-fetch contract.
    let mut all_resources: Vec<&ResourceAccess> = Vec::new();
    for a in accesses {
        for r in &a.resources {
            if let Some(prev) = all_resources.iter().find(|p| p.type_id == r.type_id) {
                return Err(EngineError::DuplicateSystemParam {
                    name: r.type_name,
                    first_mode: access_mode(prev.mutable),
                    second_mode: access_mode(r.mutable),
                });
            }
            all_resources.push(r);
        }
    }

    // Component conflicts: only matter when at least one side is
    // mutable. Two reads are fine.
    let mut combined = QueryAccess::new();
    for a in accesses {
        for c in &a.components {
            combined.push(*c);
        }
    }
    crate::ecs::query::fetch::check_no_alias(&combined, component_name)
}

fn access_mode(mutable: bool) -> &'static str {
    if mutable { "ResMut" } else { "Res" }
}

// --- System + IntoSystem -------------------------------------------------

/// Anything that can be run against a world to advance its state.
///
/// `param_access` is called once at registration so the schedule
/// can verify the system's params don't conflict with each other.
/// The default returns an empty access set — exclusive systems and
/// arity-0 function systems have no params to validate.
pub trait System: 'static {
    fn run(&mut self, world: &mut World);

    /// Per-param access descriptors, in declaration order. Called
    /// once at registration.
    fn param_access(&self, _world: &mut World) -> Vec<ParamAccess> {
        Vec::new()
    }
}

/// Convert a value (typically a function or closure) into a [`System`].
pub trait IntoSystem<Marker> {
    type System: System;
    fn into_system(self) -> Self::System;
}

// --- Exclusive systems (fn(&mut World)) ----------------------------------

pub struct IsExclusiveSystem;

pub struct ExclusiveSystem<F> {
    f: F,
}

impl<F> ExclusiveSystem<F>
where
    F: FnMut(&mut World) + 'static,
{
    pub fn new(f: F) -> Self {
        Self { f }
    }
}

pub fn exclusive<F>(f: F) -> ExclusiveSystem<F>
where
    F: FnMut(&mut World) + 'static,
{
    ExclusiveSystem::new(f)
}

impl<F> System for ExclusiveSystem<F>
where
    F: FnMut(&mut World) + 'static,
{
    fn run(&mut self, world: &mut World) {
        (self.f)(world)
    }
}

impl<F> IntoSystem<IsExclusiveSystem> for ExclusiveSystem<F>
where
    F: FnMut(&mut World) + 'static,
{
    type System = ExclusiveSystem<F>;
    fn into_system(self) -> Self::System {
        self
    }
}

// --- Param-injected systems ----------------------------------------------

pub struct FunctionSystem<F, Params> {
    f: F,
    _marker: PhantomData<fn() -> Params>,
}

// Arity 0 ----------------------------------------------------------------

impl<F> System for FunctionSystem<F, ()>
where
    F: FnMut() + 'static,
{
    fn run(&mut self, _world: &mut World) {
        (self.f)()
    }
}

impl<F> IntoSystem<fn()> for F
where
    F: FnMut() + 'static,
{
    type System = FunctionSystem<F, ()>;
    fn into_system(self) -> Self::System {
        FunctionSystem {
            f: self,
            _marker: PhantomData,
        }
    }
}

// Arity 1 ----------------------------------------------------------------

impl<F, A> System for FunctionSystem<F, (A,)>
where
    A: SystemParam + 'static,
    F: FnMut(A) + for<'w> FnMut(A::Item<'w>) + 'static,
{
    fn run(&mut self, world: &mut World) {
        // Single param: no aliasing risk between params, so the
        // unsafe is just lifetime juggling.
        let world_ptr: *mut World = world;
        // SAFETY: only one param, so no inter-param aliasing.
        unsafe {
            let a = A::fetch(&mut *world_ptr);
            (self.f)(a);
        }
    }

    fn param_access(&self, world: &mut World) -> Vec<ParamAccess> {
        vec![A::access(world)]
    }
}

impl<F, A> IntoSystem<fn(A)> for F
where
    A: SystemParam + 'static,
    F: FnMut(A) + for<'w> FnMut(A::Item<'w>) + 'static,
{
    type System = FunctionSystem<F, (A,)>;
    fn into_system(self) -> Self::System {
        FunctionSystem {
            f: self,
            _marker: PhantomData,
        }
    }
}

// Arity 2 ----------------------------------------------------------------

impl<F, A, B> System for FunctionSystem<F, (A, B)>
where
    A: SystemParam + 'static,
    B: SystemParam + 'static,
    F: FnMut(A, B) + for<'w> FnMut(A::Item<'w>, B::Item<'w>) + 'static,
{
    fn run(&mut self, world: &mut World) {
        // SAFETY: per-system conflict check at registration
        // guarantees A's access and B's access do not overlap. We
        // can therefore hand out two sequential `&mut World` borrows
        // via raw pointer; the resulting items borrow disjoint
        // parts of the world (different resource cells, different
        // component storages) and are safe to coexist for the call.
        let world_ptr: *mut World = world;
        unsafe {
            let a = A::fetch(&mut *world_ptr);
            let b = B::fetch(&mut *world_ptr);
            (self.f)(a, b);
        }
    }

    fn param_access(&self, world: &mut World) -> Vec<ParamAccess> {
        vec![A::access(world), B::access(world)]
    }
}

impl<F, A, B> IntoSystem<fn(A, B)> for F
where
    A: SystemParam + 'static,
    B: SystemParam + 'static,
    F: FnMut(A, B) + for<'w> FnMut(A::Item<'w>, B::Item<'w>) + 'static,
{
    type System = FunctionSystem<F, (A, B)>;
    fn into_system(self) -> Self::System {
        FunctionSystem {
            f: self,
            _marker: PhantomData,
        }
    }
}

// Arity 3 ----------------------------------------------------------------

impl<F, A, B, C> System for FunctionSystem<F, (A, B, C)>
where
    A: SystemParam + 'static,
    B: SystemParam + 'static,
    C: SystemParam + 'static,
    F: FnMut(A, B, C) + for<'w> FnMut(A::Item<'w>, B::Item<'w>, C::Item<'w>) + 'static,
{
    fn run(&mut self, world: &mut World) {
        // SAFETY: see arity-2 above; access check covers all 3.
        let world_ptr: *mut World = world;
        unsafe {
            let a = A::fetch(&mut *world_ptr);
            let b = B::fetch(&mut *world_ptr);
            let c = C::fetch(&mut *world_ptr);
            (self.f)(a, b, c);
        }
    }

    fn param_access(&self, world: &mut World) -> Vec<ParamAccess> {
        vec![A::access(world), B::access(world), C::access(world)]
    }
}

impl<F, A, B, C> IntoSystem<fn(A, B, C)> for F
where
    A: SystemParam + 'static,
    B: SystemParam + 'static,
    C: SystemParam + 'static,
    F: FnMut(A, B, C) + for<'w> FnMut(A::Item<'w>, B::Item<'w>, C::Item<'w>) + 'static,
{
    type System = FunctionSystem<F, (A, B, C)>;
    fn into_system(self) -> Self::System {
        FunctionSystem {
            f: self,
            _marker: PhantomData,
        }
    }
}

// Arity 4 ----------------------------------------------------------------

impl<F, A, B, C, D> System for FunctionSystem<F, (A, B, C, D)>
where
    A: SystemParam + 'static,
    B: SystemParam + 'static,
    C: SystemParam + 'static,
    D: SystemParam + 'static,
    F: FnMut(A, B, C, D)
        + for<'w> FnMut(A::Item<'w>, B::Item<'w>, C::Item<'w>, D::Item<'w>)
        + 'static,
{
    fn run(&mut self, world: &mut World) {
        // SAFETY: see arity-2 above; access check covers all 4.
        let world_ptr: *mut World = world;
        unsafe {
            let a = A::fetch(&mut *world_ptr);
            let b = B::fetch(&mut *world_ptr);
            let c = C::fetch(&mut *world_ptr);
            let d = D::fetch(&mut *world_ptr);
            (self.f)(a, b, c, d);
        }
    }

    fn param_access(&self, world: &mut World) -> Vec<ParamAccess> {
        vec![
            A::access(world),
            B::access(world),
            C::access(world),
            D::access(world),
        ]
    }
}

impl<F, A, B, C, D> IntoSystem<fn(A, B, C, D)> for F
where
    A: SystemParam + 'static,
    B: SystemParam + 'static,
    C: SystemParam + 'static,
    D: SystemParam + 'static,
    F: FnMut(A, B, C, D)
        + for<'w> FnMut(A::Item<'w>, B::Item<'w>, C::Item<'w>, D::Item<'w>)
        + 'static,
{
    type System = FunctionSystem<F, (A, B, C, D)>;
    fn into_system(self) -> Self::System {
        FunctionSystem {
            f: self,
            _marker: PhantomData,
        }
    }
}

// Arity 5 ----------------------------------------------------------------

impl<F, A, B, C, D, E> System for FunctionSystem<F, (A, B, C, D, E)>
where
    A: SystemParam + 'static,
    B: SystemParam + 'static,
    C: SystemParam + 'static,
    D: SystemParam + 'static,
    E: SystemParam + 'static,
    F: FnMut(A, B, C, D, E)
        + for<'w> FnMut(A::Item<'w>, B::Item<'w>, C::Item<'w>, D::Item<'w>, E::Item<'w>)
        + 'static,
{
    fn run(&mut self, world: &mut World) {
        // SAFETY: see arity-2 above; access check covers all 5.
        let world_ptr: *mut World = world;
        unsafe {
            let a = A::fetch(&mut *world_ptr);
            let b = B::fetch(&mut *world_ptr);
            let c = C::fetch(&mut *world_ptr);
            let d = D::fetch(&mut *world_ptr);
            let e = E::fetch(&mut *world_ptr);
            (self.f)(a, b, c, d, e);
        }
    }

    fn param_access(&self, world: &mut World) -> Vec<ParamAccess> {
        vec![
            A::access(world),
            B::access(world),
            C::access(world),
            D::access(world),
            E::access(world),
        ]
    }
}

impl<F, A, B, C, D, E> IntoSystem<fn(A, B, C, D, E)> for F
where
    A: SystemParam + 'static,
    B: SystemParam + 'static,
    C: SystemParam + 'static,
    D: SystemParam + 'static,
    E: SystemParam + 'static,
    F: FnMut(A, B, C, D, E)
        + for<'w> FnMut(A::Item<'w>, B::Item<'w>, C::Item<'w>, D::Item<'w>, E::Item<'w>)
        + 'static,
{
    type System = FunctionSystem<F, (A, B, C, D, E)>;
    fn into_system(self) -> Self::System {
        FunctionSystem {
            f: self,
            _marker: PhantomData,
        }
    }
}

// Arity 6 ----------------------------------------------------------------

impl<F, A, B, C, D, E, G> System for FunctionSystem<F, (A, B, C, D, E, G)>
where
    A: SystemParam + 'static,
    B: SystemParam + 'static,
    C: SystemParam + 'static,
    D: SystemParam + 'static,
    E: SystemParam + 'static,
    G: SystemParam + 'static,
    F: FnMut(A, B, C, D, E, G)
        + for<'w> FnMut(A::Item<'w>, B::Item<'w>, C::Item<'w>, D::Item<'w>, E::Item<'w>, G::Item<'w>)
        + 'static,
{
    fn run(&mut self, world: &mut World) {
        // SAFETY: see arity-2 above; access check covers all 6.
        let world_ptr: *mut World = world;
        unsafe {
            let a = A::fetch(&mut *world_ptr);
            let b = B::fetch(&mut *world_ptr);
            let c = C::fetch(&mut *world_ptr);
            let d = D::fetch(&mut *world_ptr);
            let e = E::fetch(&mut *world_ptr);
            let g = G::fetch(&mut *world_ptr);
            (self.f)(a, b, c, d, e, g);
        }
    }

    fn param_access(&self, world: &mut World) -> Vec<ParamAccess> {
        vec![
            A::access(world),
            B::access(world),
            C::access(world),
            D::access(world),
            E::access(world),
            G::access(world),
        ]
    }
}

impl<F, A, B, C, D, E, G> IntoSystem<fn(A, B, C, D, E, G)> for F
where
    A: SystemParam + 'static,
    B: SystemParam + 'static,
    C: SystemParam + 'static,
    D: SystemParam + 'static,
    E: SystemParam + 'static,
    G: SystemParam + 'static,
    F: FnMut(A, B, C, D, E, G)
        + for<'w> FnMut(A::Item<'w>, B::Item<'w>, C::Item<'w>, D::Item<'w>, E::Item<'w>, G::Item<'w>)
        + 'static,
{
    type System = FunctionSystem<F, (A, B, C, D, E, G)>;
    fn into_system(self) -> Self::System {
        FunctionSystem {
            f: self,
            _marker: PhantomData,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq)]
    struct Counter(u32);

    #[derive(Debug, PartialEq)]
    struct Label(String);

    #[derive(Debug, PartialEq)]
    struct Gravity(f32);

    #[derive(Debug, PartialEq)]
    struct Paused(bool);

    fn run_once<M>(world: &mut World, sys: impl IntoSystem<M>) {
        let mut s = sys.into_system();
        // Validate the system's params before running, mirroring
        // what `Schedule::add_system` does at registration.
        let accesses = s.param_access(world);
        if let Err(err) = check_param_conflicts(&accesses, |c| {
            world
                .component_name(c.component_id)
                .unwrap_or("<unregistered>")
        }) {
            panic!("{err}");
        }
        s.run(world);
    }

    // --- exclusive systems ----------------------------------------------

    #[test]
    fn exclusive_system_mutates_world() {
        let mut world = World::new();
        world.insert_resource(Counter(0));
        run_once(
            &mut world,
            exclusive(|w: &mut World| {
                let c = w.resource_mut::<Counter>().unwrap();
                c.0 += 1;
            }),
        );
        assert_eq!(world.resource::<Counter>(), Some(&Counter(1)));
    }

    #[test]
    fn exclusive_system_holds_internal_state_across_runs() {
        let mut world = World::new();
        world.insert_resource(Counter(0));
        let mut local_calls = 0u32;
        let mut sys = exclusive(move |w: &mut World| {
            local_calls += 1;
            w.resource_mut::<Counter>().unwrap().0 = local_calls;
        })
        .into_system();
        sys.run(&mut world);
        sys.run(&mut world);
        sys.run(&mut world);
        assert_eq!(world.resource::<Counter>(), Some(&Counter(3)));
    }

    // --- arity 0 ---------------------------------------------------------

    #[test]
    fn zero_arity_system_runs() {
        let mut world = World::new();
        let mut calls = 0u32;
        let mut sys = (move || {
            calls += 1;
            assert!(calls <= 2);
        })
        .into_system();
        sys.run(&mut world);
        sys.run(&mut world);
    }

    // --- arity 1 ---------------------------------------------------------

    #[test]
    fn res_reads_resource() {
        let mut world = World::new();
        world.insert_resource(Counter(7));
        run_once(&mut world, |c: Res<Counter>| {
            assert_eq!(c.0, 7);
        });
    }

    #[test]
    fn res_mut_mutates_resource() {
        let mut world = World::new();
        world.insert_resource(Counter(0));
        run_once(&mut world, |mut c: ResMut<Counter>| {
            c.0 = 42;
        });
        assert_eq!(world.resource::<Counter>(), Some(&Counter(42)));
    }

    #[test]
    fn res_deref_gives_shared_access() {
        let mut world = World::new();
        world.insert_resource(Label("hi".into()));
        run_once(&mut world, |l: Res<Label>| {
            assert_eq!(l.0.as_str(), "hi");
        });
    }

    #[test]
    fn res_mut_deref_and_deref_mut_work() {
        let mut world = World::new();
        world.insert_resource(Label("old".into()));
        run_once(&mut world, |mut l: ResMut<Label>| {
            assert_eq!(l.0.as_str(), "old");
            l.0 = "new".into();
        });
        assert_eq!(world.resource::<Label>().unwrap().0, "new");
    }

    // --- arity 2 ---------------------------------------------------------

    #[test]
    fn arity_two_res_res_different_types() {
        let mut world = World::new();
        world.insert_resource(Counter(5));
        world.insert_resource(Gravity(9.8));
        run_once(&mut world, |c: Res<Counter>, g: Res<Gravity>| {
            assert_eq!(c.0, 5);
            assert_eq!(g.0, 9.8);
        });
    }

    #[test]
    fn arity_two_res_mut_res() {
        let mut world = World::new();
        world.insert_resource(Counter(0));
        world.insert_resource(Gravity(2.0));
        run_once(&mut world, |mut c: ResMut<Counter>, g: Res<Gravity>| {
            c.0 = (g.0 * 10.0) as u32;
        });
        assert_eq!(world.resource::<Counter>(), Some(&Counter(20)));
    }

    #[test]
    fn arity_two_res_mut_res_mut_different_types() {
        let mut world = World::new();
        world.insert_resource(Counter(1));
        world.insert_resource(Gravity(1.0));
        run_once(
            &mut world,
            |mut c: ResMut<Counter>, mut g: ResMut<Gravity>| {
                c.0 += 1;
                g.0 *= 2.0;
            },
        );
        assert_eq!(world.resource::<Counter>(), Some(&Counter(2)));
        assert_eq!(world.resource::<Gravity>(), Some(&Gravity(2.0)));
    }

    // --- arities 3 and 4 -------------------------------------------------

    #[test]
    fn arity_three_mixed_params() {
        let mut world = World::new();
        world.insert_resource(Counter(0));
        world.insert_resource(Label("x".into()));
        world.insert_resource(Gravity(3.0));
        run_once(
            &mut world,
            |mut c: ResMut<Counter>, l: Res<Label>, g: Res<Gravity>| {
                c.0 = (l.0.len() as f32 + g.0) as u32;
            },
        );
        assert_eq!(world.resource::<Counter>(), Some(&Counter(4)));
    }

    #[test]
    fn arity_four_mixed_params() {
        let mut world = World::new();
        world.insert_resource(Counter(0));
        world.insert_resource(Label("".into()));
        world.insert_resource(Gravity(0.0));
        world.insert_resource(Paused(false));
        run_once(
            &mut world,
            |mut c: ResMut<Counter>, mut l: ResMut<Label>, g: Res<Gravity>, p: Res<Paused>| {
                if !p.0 {
                    c.0 = 9;
                    l.0 = format!("g={}", g.0);
                }
            },
        );
        assert_eq!(world.resource::<Counter>(), Some(&Counter(9)));
        assert_eq!(world.resource::<Label>().unwrap().0, "g=0");
    }

    // --- ergonomics: named fn + closure ---------------------------------

    fn named_system(mut c: ResMut<Counter>) {
        c.0 += 100;
    }

    #[test]
    fn named_fn_converts_via_into_system() {
        let mut world = World::new();
        world.insert_resource(Counter(1));
        run_once(&mut world, named_system);
        assert_eq!(world.resource::<Counter>(), Some(&Counter(101)));
    }

    #[test]
    fn system_state_persists_via_fn_mut_capture() {
        let mut world = World::new();
        world.insert_resource(Counter(0));
        let mut ticks = 0u32;
        let mut sys = (move |mut c: ResMut<Counter>| {
            ticks += 1;
            c.0 = ticks;
        })
        .into_system();
        sys.run(&mut world);
        sys.run(&mut world);
        sys.run(&mut world);
        assert_eq!(world.resource::<Counter>(), Some(&Counter(3)));
    }

    // --- error cases -----------------------------------------------------

    #[test]
    #[should_panic(expected = "resource")]
    fn missing_resource_panics_with_clear_message() {
        let mut world = World::new();
        run_once(&mut world, |_c: Res<Counter>| {});
    }

    #[test]
    #[should_panic(expected = "more than once")]
    fn duplicate_params_panic_before_fetch() {
        let mut world = World::new();
        world.insert_resource(Counter(0));
        run_once(&mut world, |_a: Res<Counter>, _b: ResMut<Counter>| {});
    }

    #[test]
    #[should_panic(expected = "more than once")]
    fn two_res_for_same_type_panic() {
        let mut world = World::new();
        world.insert_resource(Counter(0));
        run_once(&mut world, |_a: Res<Counter>, _b: Res<Counter>| {});
    }
}
