# Glyph — The Gameplay Language

Glyph is the language of moment-to-moment gameplay. Spells, abilities, item logic, NPC plans, UI, combat, interaction — every behaviour the player sees expressed at frame rate is authored in Glyph. The engine is the world; Glyph is what the world *does*.

This document describes Glyph's domain and the principles that justify its existence. Syntax and grammar are deliberately not pinned here; those crystallise as the language meets its first real consumers.

---

## How Glyph Serves the Pillars

Glyph exists because none of the engine's four pillars (`overview.md`) can be served by an off-the-shelf scripting language.

**The world is alive.** A live world means physics objects, status effects, spell interactions, and NPC behaviours that compose in ways the engine cannot enumerate ahead of time. Glyph is where that composition is authored. A fireball that ignites flammables, evaporates water on wet enemies, cracks ice — that is not data; it is procedural code with pattern matching over component composition. Glyph is the language whose first-class noun is "the entity, and what it currently is."

**Built for one kind of game.** Glyph is not a general-purpose programming language. ECS components are first-class. Pattern matching by component shape is first-class. Reactive subscriptions to component change are first-class. Refinement types target the domain's bounded values. Within the domain it reads like the domain; outside the domain it would be an awkward choice — and that is correct, because the engine is not for outside the domain.

**Developer ergonomics is a feature.** Hot reload, REPL access into the running game, fast iteration on tuning values, clean errors. The act of authoring gameplay is meant to be enjoyable; Glyph is the surface where that enjoyment lives or dies.

**Organism, not castle.** Glyph code is written to be modified in flight. Hot reload is not a development convenience; it is the way the language is meant to be lived in. Static typing forms the skeleton — function signatures, component schemas, refinement types on bounded values — and the shape inside that skeleton is fluid. New behaviour drops in without restart. The REPL is a window into the running game, not a separate sandbox.

---

## The Domain

Glyph runs in Layer 4 — the local simulation — at 60 Hz on the main thread. It executes during gameplay, mutates ECS components, spawns and despawns entities, drives UI, and dispatches Tier 1 and Tier 2 events. It is the procedural shell wrapped around the engine's native systems.

What it is for, in concrete terms:

- **Spells and abilities.** A spell composes elemental rules over the entities it affects. The composition is structural and recursive; spell components combine in ways the engine cannot enumerate. This is the immersive sim's language.
- **HTN plans.** When an agent decides to investigate a noise, the plan that decomposes "investigate" into "move to → look around → respond to evidence" is authored in Glyph. The same planner serves Game 2's hunter, Game 3's creatures, and Game 4's NPC daily routines.
- **Utility scoring functions.** "How appealing is the goal of going to bed right now?" — a function over a blackboard. Authored in Glyph because designers tune these often, and tuning a Rust function requires a recompile.
- **Item and interaction logic.** What happens when the player drinks a potion, picks a lock, talks to an NPC, drops a torch in dry grass. The logic of objects in the world is the logic the player feels, constantly.
- **UI behaviour.** Inventory, dialogue, spell-crafting, journal. Reactive bindings between UI state and ECS state are Glyph's idiom.

What it is **not** for: the world simulation. Faction power balances, economy ticks, succession crises, monthly events — those are Chronicle's domain. Trying to write them in Glyph fights Chronicle's evaluation model, which is the reason the languages are split.

---

## Principles

### Procedural and Higher-Order

Glyph is procedural at the bottom, functional at the top. Closures, higher-order functions, and pattern matching are first-class because the domain demands them. A spell's "on hit" is naturally a function. An HTN task's "precondition" is a function over a blackboard. The pattern matcher decomposes a target by its component composition.

This is the load-bearing case for Glyph being its own language rather than a configuration format: the domain wants real abstraction, real composition, the kind of expressivity a structured data format cannot reach without growing into a language by accident.

### Statically Typed, Eventually

Glyph begins dynamic. The cost of building a static type checker is real, and Game 2A is the wrong place to pay it; the pain of dynamic typing in a fast-iterating gameplay language is the design data the type system needs.

By Game 3, Glyph is typed: type inference, component types known to the compiler, errors at compile time rather than runtime. By Game 4, types include refinement on domain values where they earn it — opinions bounded to their valid range, spell intensities to non-negative values, faction standings to discrete categories.

Refinement types are not academic decoration. They are how authors get errors before a designer playtest, instead of crashes during one. They are how the type system **secures the domain logic**, not just the data shapes.

### Reactive by Construction

The reactive backbone exists; Glyph integrates with it natively. "When this component is added, run this handler." "When this value changes, update this binding." These are language idioms, not library calls. The Tier 1 substrate in the ECS is what makes them cheap; Glyph is what makes them readable.

This is how pillar 4 manifests in the language: rules are strict, shape is fluid. Glyph code declares typed contracts (a handler's signature is fixed), but which handlers are subscribed to which events is determined at runtime, and changes as scripts reload.

### Performant in the Hot Path

Glyph runs in the inner loop. Particle systems updating thousands of entities, spell collisions firing dozens of times per second, UI bindings reacting to every component change — these cannot be slow.

The compiler emits bytecode, not a tree-walked AST. Common operations — ECS queries, vector math, component access — compile to specialised opcodes that touch native data directly. The steady state is allocation-free; hot loops that allocate are a bug, and the language gives authors the tools to avoid them. JIT is reachable from the bytecode design if profiling demands it, but it is not built until then.

A Glyph that is not fast in its domain is not worth its existence. It must be at least as fast as Lua-with-FFI and ideally closer to native — close enough that the choice between writing a system in Rust and writing it in Glyph is about iteration speed and authoring ergonomics, not performance.

### Designed for the Domain, Not for Generality

This is pillar 2 made concrete. Glyph has dedicated syntax for component access. ECS queries are first-class. Pattern matching by component composition is first-class. Hot-reloaded module boundaries are visible in the language. Refinement types target the domain's bounded values.

The cost is that Glyph is not the right language for, say, writing a level editor or a build script. The benefit is that within its domain, it reads like the domain. A spell looks like a spell. An HTN task looks like an HTN task. A utility function looks like a scoring function over a blackboard. This is the test of whether the language is earning its keep.

### Hot Reload Is the Way of Working

Pillar 3, made concrete in Glyph. State held in script is held conservatively. Long-lived state belongs in the world database or in ECS components — places that survive code reload. Script modules hold definitions, not data. When data is held, it is held in places that have explicit migration paths under reload. Reload failures are non-fatal: a syntax error reports cleanly and keeps the previous version running. The REPL is a tool, not a curiosity.

---

## Why Glyph and Not Existing Languages

Lua, Wren, Rhai, Rune, embedded JavaScript — there are existing scripting languages and they are mostly fine. The reasons we are not using one of them are specific.

- **None of them have ECS integration as a first-class concept.** Treating components as the language's core noun is a design point, not a library binding.
- **None of them have refinement types over domain values.** A spell intensity bounded to non-negative reals catches a class of authoring bugs that no general scripting language can, because they do not know the domain.
- **None of them are designed for the engine's reactive substrate.** Subscriptions, change detection, hot reload of behaviour — these can be retrofitted, but the result is awkward.
- **The engine's shape is the language's shape.** Pillar 2 says every tool is tailored. Adopting a third-party language imports decisions made for someone else's game.
- **The author of this engine has the experience and inclination to build it**, and the project does not have a release deadline that makes "buy, don't build" the obvious move.

The first four are technical justifications. The fifth is the honest one: this is a project where the language is itself part of the artefact, and where the cost of building it is acceptable because the builder enjoys the work and the schedule allows it. Pillar 3 — ergonomics is a feature — covers the developer of the engine as much as anyone else.

---

## Staging

Glyph arrives in stages, each motivated by concrete pain in the previous game.

- **v0.1 (Game 2A) — Dynamic.** Shik syntax. Bytecode and VM. Hot reload. Rust FFI. Dynamically typed. The minimum that scripts a horror game's logic and lets us feel where dynamic typing hurts.
- **v0.5 (Game 2B → Game 3) — Typed.** Type inference. Engine-intrinsic component types known to the compiler. Pattern matching by component shape, with the compiler verifying matches. Better error messages.
- **v1.0 (Game 4) — Expressive.** Hygienic macros, driven by the spell-composition needs Game 3's immersive-sim foundations expose. Refinement types on domain values. ECS queries embedded in the language with the compiler emitting specialised opcodes. Profiling-driven optimisation passes for the hot paths Game 4's NPC counts surface.

Each version pays for itself in the game it ships in. Each version is constrained to what its game needs. The discipline is the same as the engine's: build no abstraction before its consumer is alive.
