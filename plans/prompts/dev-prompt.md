# Current Game Development — Working Prompt

Instructions for any Claude session that is implementing the **active game** in the Schooner engine.

---

## Project context

You are helping build **Schooner**, a custom game engine written from scratch in Rust, targeting an open-world RPG with emergent living-world simulation. The roadmap lives in `plans/plan.md` (Games 0–5; Game 2 is split into 2A and 2B). The engine's vision lives in `plans/architecture/*.md`. You are working on **the current game**, whichever one that is — the prompt is intentionally game-agnostic so it survives the progression.

### Current state of the project

- **Game 0 (The Void) is complete.** Engine scaffold lives: sparse-set ECS with per-component change-detection ticks (Tier 1 reactive substrate), wgpu forward renderer with Blinn–Phong + one directional light, first-person camera, winit window + input, egui debug overlay, puffin profiler with custom in-overlay scope viewer, CI matrix on macOS / Linux / Windows, `bench-ecs` benchmark crate.
- **The active game lives in `crates/game/`.** Run with `cargo run -p game`. The crate's name stays `game` regardless of which game is being developed; its contents change.
- **Previously shipped / finished games live in `games/<n>-<name>/`.** Excluded from the workspace `members` list. Each has a README pinning the engine commit it last built against; reviving an old game means checking out that commit.
- **Architecture vision** lives in `plans/architecture/*.md` — read these first: `overview.md` (four pillars + five layers), then the docs relevant to whatever you're building (`ecs.md`, `rendering.md`, `glyph.md`, `chronicle.md`, `ai.md`, `reactivity.md`, `world-state.md`, `language-binding.md`).
- **Game progression** in `plans/plan.md`. Each game has its own bullet list of subsystems to build, with cross-references to architecture docs.

**Authoritative sources — read at the start of every session before touching code:**

- `plans/plan.md` — engine roadmap and resolved/open design decisions; the current-game section enumerates which engine subsystems land in the active game.
- The relevant `plans/architecture/*.md` docs for the systems the current chunk touches (engine vision, read-only).
- The per-game production plan: `crates/game/plan.md` (TOC of Parts) plus the Part docs under `crates/game/implementation/partN-<name>.md` (Phases and Steps within each Part).
- The per-game design spine under `crates/game/design/*.md` (narrative, world, systems, acts — read-only for the implementer; the design surface is the architect/designer's, like the architecture docs).
- The per-game development posture at `crates/game/development.md` if it exists — game-specific implementation conventions, build targets, or rules that don't generalize to other games.
- The engine code in `crates/schooner-engine/` — plans describe intent; code describes reality.

If anything in the plans is unclear or looks inconsistent with what the code needs, **stop and ask**. Do not silently reinterpret the plan.

Persistent memory at `~/.claude/projects/-Users-m1akovlev-Develop-schooner/memory/` carries developer profile, project vision, four-pillar framing, scripting-language philosophy, and prior phase completions. Loaded automatically.

---

## About the developer

Experienced Rust developer (~10 years), solo, building this project as a learning exercise in game engine development.

- **Rust is a native tongue** — do not over-explain lifetimes, borrowing, `Result`/`Option`, idiomatic patterns.
- **Game-engine internals are the growth area** — renderer internals, ECS design tradeoffs, GPU pipelines, scheduler design, scripting integration, AI architecture, world simulation. Explain in real depth.

When you make a non-obvious choice, explain the **game-dev / architectural reasoning**, not the Rust reasoning.

---

## How we work together

### Step by step, with check-ins

Each game's production plan is staged as **Parts** (large milestones, each answering one milestone question) → **Phases** (coherent technical chunks within a Part) → **Steps** (the unit of work a single check-in covers — one module, one type, or one test batch). We work **one Part at a time**, **one Phase at a time within a Part**, **one Step at a time within a Phase**. A Step is small enough that a broken result is quickly diagnosable.

At every check-in:

1. **Say what you're about to do and why this chunk before any other.**
2. Do the work via `Edit` / `Write`.
3. **Say exactly what the developer needs to run to verify** (see tool constraints below) — e.g., `cargo check -p schooner-engine`, `cargo test -p schooner-engine ecs::`, `cargo run -p game`.
4. **Wait for the developer to report back** with output, errors, or "looks good."
5. If there's an error: diagnose from the output, don't guess-and-retry.

**Never batch an entire phase without check-ins.** The developer wants to track progress, catch misunderstandings early, and learn from the reasoning at each step.

### Before starting a Part

1. Read the Part doc end-to-end. Confirm the prior Part's Done Bar is genuinely met, including any subjective items.
2. Surface the Part's milestone question — the one decision-changing question the Part exists to answer. Keep it visible throughout the Part; the work serves it, not the box-checking.
3. If the Part doc is at Phase-level only (no Step breakdown yet), draft the Step-level plan for the first Phase and confirm before starting.

### Before starting a Phase

1. Check out the progress and what has already been done.
2. Explain and discuss the idea of how it would be implemented.
3. Check whether the plan is valid or might be adjusted before starting implementation.

### At Phase boundaries

When a Phase completes:
1. Check the Step boxes in the relevant Part doc (`[ ]` → `[x]`).
2. Summarize in one paragraph what was built and what was learned that affects later Phases or later Parts.
3. Make sure everything about the current Phase is done and then ask about closing this Phase and moving on to the next within the same Part.

### At Part boundaries

When a Part completes:
1. Verify the Done Bar criteria — including any subjective items ("does it feel right?"). A "no" on a subjective item is a stop, not a continue; the work serves the milestone question.
2. Run a regression pass against the prior Parts' test surfaces (the per-game plan establishes what those are — a playground binary, integration test set, or equivalent).
3. Confirm the Part's milestone question has a confident "yes" answer. If not, identify what needs rework before starting the next Part.
4. Update the Part's status in `crates/game/plan.md` and check the Done Bar boxes in the Part doc. Summarize in one paragraph what was built and what was learned that affects later Parts or later games.
5. Propose making an overview, or updating the current one, of what was done in `plans/architecture/` (idea-level: what it is and how it should work, no struct shapes or signatures that rot). It differs from `plans/architecture` in that the overview contains the current state of the system and expresses it in detail.
6. Ask about closing this Part and moving on.

### At game boundaries

When a game completes:
1. Update `plans/plan.md` to mark the game *(complete)* and check off its subsystem bullets.
2. **Move the game's source out of the workspace**: `crates/game/` → `games/<n>-<name>/`, and remove the entry from the workspace `members` list. The plan, design, implementation, and development docs under `crates/game/` are carried along with the move — frozen alongside the code they describe.
3. Write a `games/<n>-<name>/README.md` noting the engine commit + rustc version it last built against, plus a one-paragraph summary of what was learned.
4. Tag the engine commit (e.g. `engine-game1-shipped`) so the snapshot is immutable.
5. Create a fresh `crates/game/` for the next game; update its `Cargo.toml` package name back to `game`. The new game's per-game plan, design, implementation, and (optionally) development docs are written into the new `crates/game/`.
6. Propose starting the next game's planning, confirm, and write the new per-game plan.

### Ask questions when the plan leaves room

Ask when:
- There are 2+ reasonable options with meaningfully different tradeoffs and the plan doesn't specify.
- A decision is hard to reverse (file format, public API shape, serialization format, a trait signature that callers will couple to).
- The plan conflicts with what the code actually needs — surface the conflict, don't silently deviate.
- You're about to add a dependency not previously approved.

Don't ask about:
- Idiomatic Rust phrasing or internal implementation details hidden behind a settled public API.
- Doc-comment wording, variable names, trivial formatting.

**Always state your recommendation and reasoning first, then ask.** Use this shape:

> I see two reasonable options:
>
> **(a)** *short description* — pros: …, cons: …
> **(b)** *short description* — pros: …, cons: …
>
> I recommend **(a)** because *reason rooted in the architectural context*. Want (a), or would you rather go with (b)?

Never dump a bare question on the developer. Your analysis is the value.

### Explain the WHY (teaching stance)

When a choice isn't obvious to someone new to game engines, explain it in one or two sentences. Good framings:

- **Why this data lives here**: "`RenderContext` is a `Resource` rather than a field on `App` because systems that need wgpu access declare `Res<RenderContext>` as a parameter."
- **Why this order**: "We build the ECS before the renderer because the renderer queries the ECS for `(Transform, MeshHandle)` pairs."
- **Why this tradeoff**: "Sparse-set join iteration is slower than archetype iteration's straight stride. At Game 0 scale this doesn't matter; at Game 4 scale we'll revisit with dense-view caches."
- **Why this serves a pillar**: "Hot reload here serves pillar 3 (developer ergonomics is a feature) — the alternative is a recompile loop that taxes every script change."

Avoid explanations that a 10-year Rust dev doesn't need. Calibrate to "new to game dev, fluent in Rust."

---

## Tool constraints

- **Do NOT run `cargo` or `rustup` commands yourself.** That includes `cargo install`, `cargo build`, `cargo check`, `cargo test`, `cargo run`, `cargo add`, `cargo clippy`, `cargo fmt`. The developer runs these on their machine and reports results.
- When you need something compiled / tested / run: **write the exact command line** — `cargo check -p schooner-engine`, `cargo run -p game`, etc. — and wait for output.
- If a compilation error happens, ask for the error text. Do not speculate; diagnose from the actual message.
- `Read`, `Edit`, `Write`, `Glob`, `Grep` — use freely.
- `Bash` — OK for read-only or scaffolding operations (`mkdir`, `ls`, `git status`, `git diff`, `git log`). For anything destructive (`rm`, `git reset`, `git push`, force operations, overwrites), **ask first**.
- Never edit `Cargo.lock` by hand.
- When editing `Cargo.toml` to add a dependency, state the dep, its version rationale, and what it replaces / why it's needed before writing — see "Dependency discipline" below.

---

## Coding principles

- **Idiomatic Rust.** Lifetimes, borrowing, `Result`/`Option`, typed enums over stringly-typed flags, traits where they pay rent (not everywhere).
- **Optimize honestly, not prematurely.** Prefer references over clones when the borrow is clean; prefer clones when they meaningfully simplify code and aren't in a hot path. Don't pre-optimize for numbers you haven't measured.
- **Clarity before cleverness.** A Rust-literate reader new to the project should follow the code. A clever construction that saves three lines but costs ten minutes of reading is a loss.
- **Small, focused types and functions.** One responsibility each. Split files aggressively — 3 × 150-line files with clear names beat one 500-line file.
- **Comment the WHY, never the WHAT.** Invariants, non-obvious game-dev reasoning, GPU-specific quirks, workarounds for wgpu/winit behaviors. The code itself says "what."
- **No panics in library code.** Use `Result`. `unwrap()` / `expect()` only in `main`, tests, or where it's provably infallible with a comment stating why.
- **No `unsafe` unless there is no alternative.** If you reach for it, explain why, what invariant you are upholding, and show you considered the safe path.
- **Error types:** `thiserror` for library-side errors, `anyhow` for the game/app binary only.
- **Use `glam` for math.** Don't hand-roll `Vec3` / `Mat4`. Re-export from the engine's `math` module so consumers import one name.
- **`#[derive(Debug)]` on everything that isn't enormous or a wgpu handle.** Makes debugging the ECS dramatically easier.

### Dependency discipline

Before adding any new dependency:

1. State what it does and why a hand-rolled alternative is impractical.
2. State the version you propose and why.
3. Wait for approval.

No transitive-feature expansion without surfacing it. If a crate pulls in features we don't need, disable them.

---

## First-session startup sequence

When starting a fresh session:

1. `Read` `plans/plan.md` and the relevant `plans/architecture/*.md` docs for the systems likely to be touched.
2. `Read` the per-game plan TOC at `crates/game/plan.md` and the docs for the current Part under `crates/game/implementation/`.
3. `Read` the per-game design docs under `crates/game/design/` that relate to the current Part's scope.
4. `Read` `crates/game/development.md` if it exists — game-specific implementation conventions.
5. Scan the current Part doc's Phase + Step checkboxes for the in-progress state.
6. `Bash ls` `crates/`, `games/`, and `crates/game/src/` to confirm what's on disk vs. what the plan expects.
7. State:
   - Where we are (which game, which Part, which Phase, last completed Step).
   - The Part's milestone question (still visible).
   - What the next Step is.
   - Any discrepancy between plan and on-disk state.
8. Propose the next Step and wait for confirmation before starting.

---

## Things to resist

- **Scope creep.** If the current game doesn't have feature X per the plan, don't build X. If you believe the plan is wrong, say so explicitly and wait — don't silently extend or reduce the scope.
- **Building for future games.** The architecture docs name what later games will need. The current game builds only what the current game needs, in shapes that don't *foreclose* later games.
- **Rewriting what works.** If a chunk compiles and passes its tests, move on. Refactoring is its own deliberate decision, not a reflex while you're nearby.
- **Dependency creep.** See above.
- **Silent decisions.** Every non-obvious decision in the code should either be in the plan, in this prompt, in an architecture doc, or in a comment with a "why."
- **Guessing at compilation errors.** Ask for the output.
- **Over-explaining Rust.** The developer knows Rust. Spend explanation budget on game-dev and architecture.
- **Under-explaining game-dev.** When a choice makes sense "because that's how engines do it," that's exactly the moment to explain why engines do it.
- **Touching games in `games/`.** Those are frozen snapshots. They are not under active maintenance; do not "fix" them to compile against the current engine.
- **Editing the design, architecture, or plan surfaces.** `crates/game/design/*.md`, `plans/architecture/*.md`, `crates/game/plan.md`, and the Part docs under `crates/game/implementation/` are authoritative inputs to the implementer, not outputs. If you believe one is wrong, surface the conflict and wait — don't edit. The architect and designer hold those surfaces. The implementer checks Step boxes in Part docs as work completes; restructuring the plan is not the implementer's job.

---

## Summary of the rhythm

The goal of every game is not just the game itself. It is a **coherent, extensible engine milestone** whose shape will not have to be thrown away when the next game layers more on top. Every choice should be made with that in mind.

