//! Interned symbols — names turned into cheap, comparable handles.
//!
//! Architecture: `architecture/input.md` §"Action identity is a name,
//! not a closed enum", and `architecture/language-binding.md`.
//!
//! A [`Symbol`] is an interned name: [`sym`] maps a string to a small
//! `Copy` handle, and the same name always yields the same handle. The
//! interner is **process-global**, not per-[`World`](crate::ecs::World):
//! a name is a global fact (it points at meaning, not at per-world
//! storage), so two worlds — or the engine and a future script VM —
//! that intern `"jump"` get the *same* handle with nothing to reconcile
//! at the boundary. This is the seam the language binding hangs off: a
//! Glyph script registering an action by name walks the identical
//! `name → symbol` path the engine's Rust setup does.
//!
//! Interned names are leaked to `'static`. The set of distinct names a
//! program ever sees is small and bounded (a few dozen actions, plus the
//! component names Glyph will register), and they live for the whole
//! process, so leaking is the right model — it matches the Lisp symbol
//! table this seeds, and gives [`symbol_name`] a real `&'static str` with
//! no lifetime plumbing.

use std::collections::HashMap;
use std::sync::{LazyLock, RwLock};

/// An interned name. Construct via [`sym`]; compare and hash cheaply.
/// Opaque by design — the backing integer is an interner detail, never
/// a stable id to serialize or assert on (it depends on intern order).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Symbol(u32);

#[derive(Default)]
struct Interner {
    ids: HashMap<&'static str, Symbol>,
    names: Vec<&'static str>,
}

static INTERNER: LazyLock<RwLock<Interner>> = LazyLock::new(|| RwLock::new(Interner::default()));

/// Intern `name`, returning its stable [`Symbol`]. Idempotent: the same
/// name always returns the same handle. Cheap on the common path (a read
/// lock + hash lookup); only the first sighting of a name takes the write
/// lock and leaks the string.
///
/// Mint symbols at setup / script-load and cache the handle — the
/// per-frame hot path compares cached `Symbol`s and never calls this.
pub fn sym(name: &str) -> Symbol {
    // Fast path: already interned. A read lock lets concurrent readers
    // (e.g. the world thread once Glyph lands) intern in parallel.
    {
        let interner = INTERNER.read().unwrap_or_else(|e| e.into_inner());
        if let Some(&existing) = interner.ids.get(name) {
            return existing;
        }
    }
    // Slow path: take the write lock and re-check — another thread may
    // have interned the name between dropping the read lock and here.
    let mut interner = INTERNER.write().unwrap_or_else(|e| e.into_inner());
    if let Some(&existing) = interner.ids.get(name) {
        return existing;
    }
    // Leak the name to 'static so the handle's name is borrow-free.
    let leaked: &'static str = Box::leak(name.to_owned().into_boxed_str());
    let symbol = Symbol(interner.names.len() as u32);
    interner.names.push(leaked);
    interner.ids.insert(leaked, symbol);
    symbol
}

/// The name a [`Symbol`] was interned from, for logging / diagnostics.
/// `None` only for a handle never produced by [`sym`] (not constructible
/// through the public API, so in practice always `Some`).
pub fn symbol_name(symbol: Symbol) -> Option<&'static str> {
    let interner = INTERNER.read().unwrap_or_else(|e| e.into_inner());
    interner.names.get(symbol.0 as usize).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    // The interner is process-global and shared across every test in
    // this binary, so these assert RELATIVE properties (equality,
    // distinctness, round-trip) — never an absolute index like
    // `Symbol(0)`, which depends on what else interned first.

    #[test]
    fn same_name_interns_to_same_symbol() {
        assert_eq!(sym("symbol.test.same"), sym("symbol.test.same"));
    }

    #[test]
    fn distinct_names_intern_to_distinct_symbols() {
        assert_ne!(sym("symbol.test.alpha"), sym("symbol.test.beta"));
    }

    #[test]
    fn symbol_name_round_trips() {
        let s = sym("symbol.test.round_trip");
        assert_eq!(symbol_name(s), Some("symbol.test.round_trip"));
    }

    #[test]
    fn interning_is_stable_across_calls() {
        let first = sym("symbol.test.stable");
        let _noise = sym("symbol.test.noise");
        assert_eq!(sym("symbol.test.stable"), first);
    }
}
