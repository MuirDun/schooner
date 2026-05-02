# Language Binding — How Rust, Glyph, and Chronicle Meet

The engine is written in Rust. Two languages run on top: Glyph for procedural gameplay, Chronicle for declarative world rules. The binding between Rust and the script layer is one of the most critical interfaces in the engine — it is what makes the scripting languages first-class rather than bolted-on, and it is what determines whether scripts feel like they belong or like they are fighting the host.

This document describes the principles of the binding, not the wire format.

---

## One Runtime, Two Frontends

Glyph and Chronicle are two languages but one virtual machine. They share:

- A bytecode instruction set.
- A value representation in memory.
- The FFI bridge to Rust.
- The hot-reload infrastructure.
- The editor tooling skeleton.

They differ in:

- Syntax.
- Type system rules.
- Evaluation model — Glyph executes procedurally; Chronicle compiles rules to query plans against the world database.
- Standard library.

The shared backend is the cost-saving move that makes two languages tractable. We are building one runtime; the parsers, type checkers, and stdlibs are the per-language work. When the runtime improves — better GC, better JIT, better debugger — both languages benefit.

---

## Component Schema Ownership

Components are the contract between Rust and the script world. The question of who defines them must be answered cleanly or the boundary becomes a swamp.

The split is:

- **Rust owns engine-intrinsic components.** Transform, Mesh, Camera, RigidBody, Collider, AnimationState, NavAgent, AudioSource. These exist because the renderer, physics, navigation, and audio subsystems need them with statically known shapes. They cannot be defined in script because the engine code that consumes them must reach them at native speed.
- **Glyph owns game-defined components.** Health, Faction, Inventory, Burning, Wet, SpellCaster, QuestMarker. These are gameplay concepts. The engine has no opinion on them; only scripts and other game-defined components do.
- **Chronicle does not own components.** Chronicle queries the world database, not the ECS. Its schema is in tables and relations, not components.

The bridge that makes this work is a **shared schema description** — a way of declaring a component once and having both Rust code and Glyph code see the same shape. Rust-declared components export their schema to Glyph at startup. Glyph-declared components register themselves with the engine so that systems written in Rust (or other Glyph modules) can name them. The schema is the truth; the language bindings are projections of it.

---

## How Scripts Call Rust

The engine exposes capabilities to scripts as **declared functions** with typed signatures. A Glyph script that wants to spawn a particle calls a Rust function whose signature both languages understand. A Chronicle rule that wants to read a character's faction calls a Rust function that touches the world database.

The principles:

- **Signatures are stable contracts.** A function exposed to scripts has a declared name, parameters, and return type. The implementation behind it can change freely; the signature cannot without coordinated migration.
- **Capability scope is explicit.** A function exposed to Glyph is not automatically exposed to Chronicle, and vice versa. Each language sees the surface that makes sense for its domain. Glyph cannot evaluate world rules; Chronicle cannot spawn particles. The boundary is enforced at the binding layer.
- **No implicit Rust execution from script.** Scripts call declared functions; they do not reflect into Rust types or invoke arbitrary Rust code. This keeps the security and performance contract clean.

---

## How Rust Calls Scripts

Rust invokes scripts by name through the VM, with values that match the script function's declared parameters. A common pattern is the engine running script-authored systems each frame: "for each scripted system in this stage, hand it its query results, run, take its returned commands." Another is event handling: a Tier 2 event the engine raises is dispatched to script-authored handlers registered for that event type.

The principle: **Rust never embeds script source**. Rust runs bytecode. Source-to-bytecode translation is the responsibility of the compiler that runs at script-load time and again on hot-reload.

---

## Hot Reload as a First-Class Commitment

The scripting philosophy — programs as organisms, not castles — is meaningless without hot reload. A script change must take effect without restarting the game. This is not a stretch goal; it is the feature that makes the language worth having.

Reload semantics:

- **A reloaded module replaces the previous bytecode.** Functions called after reload run new code.
- **In-flight state is preserved where possible.** Live ECS components are not destroyed by a reload; running HTN plans are not torn down. Where the new code is incompatible with the old data, the offending data is reset rather than the entire game.
- **Reload is observable.** Scripts can register hooks that run on their own reload, allowing custom migration of long-lived state.
- **Reload failures are non-fatal.** A syntax error in a hot-reloaded script reports the error and keeps the previous version running.

The substrate for hot reload is shared between Glyph and Chronicle. The semantics are tuned per language — Chronicle reload is closer to recompiling rule definitions; Glyph reload is closer to swapping in new procedural code — but the file watcher, the bytecode loader, and the error reporting are one system.

---

## The Type System at the Boundary

Glyph and Chronicle are statically typed (eventually, in Glyph's case — see `glyph.md` for staging). At the Rust boundary, types are reconciled in one direction: **Rust schemas are the source of truth for engine-intrinsic types**, and **script schemas are the source of truth for script-defined types**. Both sides agree by reading the same registry.

When the boundary disagrees — a Glyph script tries to call a Rust function with a wrongly-typed argument, or a Chronicle rule queries a non-existent relation — the failure is at compile time, not at runtime. This is non-negotiable. Type errors at runtime in scripts running on the world thread are unacceptable; they would mean a content bug discovered five game-months into a save.

Refinement types are a tool for the domain wherever they earn their cost. An opinion is an integer in a known range; a faction's standing is bounded; a spell's elemental composition is a small set. Where the domain has constraints the language can express, the language expresses them, and content authors get errors before the game runs the rule.

---

## Performance and the Hot Path

Glyph runs at 60 Hz on the main thread for moment-to-moment gameplay code. Chronicle runs at game-tick rates on the world thread. Both must be fast for their domain.

The principles:

- **Bytecode compilation, not tree-walking.** Both languages compile to the shared instruction set. Tree-walking interpretation is acceptable for shik-the-shell-scripting-language; the engine languages must be faster.
- **Native paths for the hot loops.** Component access, vector math, ECS queries, world database queries — these are reachable from script as compiled-down operations, not as general FFI calls. The compiler recognises them and emits specialised opcodes.
- **No allocation in the steady state.** A spell that fires every frame should not allocate. Glyph and Chronicle both have first-class concepts for stack-allocated and pooled values for hot-path use.
- **JIT is on the table** for Glyph if profiling demands it, but is not built until profiling demands it. The bytecode VM with native specialised ops is the floor.

---

## Sandboxing

Scripts are content, not engine code, and content can be wrong. The runtime guarantees:

- **Bounded execution.** A script function has a budget; runaway scripts are interrupted, not allowed to freeze the game.
- **Bounded memory.** A script module has a memory budget; allocations beyond it fail explicitly.
- **No host filesystem or network access from script** unless the engine declares a capability for it. Mods do not get arbitrary system access by default.

These exist primarily because mods are an explicit future direction (`overview.md`). When the engine is shipped to players who run third-party Chronicle and Glyph code, the runtime must be the boundary.
