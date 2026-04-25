# Game 0 Development — Working Prompt

Instructions for any Claude session that is implementing Game 0 ("The Void") of the Schooner engine.

---

## Project context

You are helping build **Schooner**, a custom game engine written from scratch in Rust, targeting an open-world RPG with emergent living-world simulation. The full roadmap is in `plans/plan.md` (Games 0–5). You are working on **Game 0** — the engine bootstrap: sparse-set ECS with a change-detection substrate, wgpu forward renderer, first-person camera, winit window + input, egui debug overlay, puffin profiling. Cross-platform (Windows + Linux + macOS) from day one.

**Authoritative sources — read both at the start of every session before touching any code:**
- `plans/plan.md` — overall roadmap and resolved/open design decisions.
- `plans/game0-plan.md` — Game 0 architecture, module-by-module design, and the ordered todo list in §6 (Phases A through J).

If anything in the plans is unclear or looks inconsistent with what the code needs, **stop and ask**. Do not silently reinterpret the plan.

Your persistent memory at `~/.claude/projects/-Users-m1akovlev-Develop-schooner/memory/` already contains the developer profile, project vision, and scripting-language philosophy. It is loaded automatically; you do not need to reload it.

---

## About the developer

Experienced Rust developer (~10 years), solo, building this project as a learning exercise in game engine development. This is important:

- **Rust is a native tongue** — do not over-explain lifetimes, borrowing, `Result`/`Option`, or idiomatic patterns.
- **Game dev is the growth area** — renderer internals, ECS design tradeoffs, game-loop patterns, GPU pipelines, camera math are where explanation is welcome.

When you make a non-obvious choice, explain the **game-dev reasoning**, not the Rust reasoning.

---

## How we work together

### Step by step, with check-ins

We are following `game0-plan.md` §6 (Phases A → J). Work **one phase at a time**, and within a phase, **one coherent chunk at a time**. A "chunk" is a unit small enough that a broken result is quickly diagnosable — typically one module, one type, or one test batch.

At every check-in:

1. **Say what you're about to do and why this chunk before any other.**
2. Do the work via `Edit` / `Write`.
3. **Say exactly what the developer needs to run to verify** (see tool constraints below) — e.g., `cargo check -p schooner-engine`, `cargo test -p schooner-engine ecs::`, `cargo run -p game-void`.
4. **Wait for the developer to report back** with output, errors, or "looks good."
5. If there's an error: diagnose from the output, don't guess-and-retry.

**Never batch an entire phase without check-ins.** The developer wants to track progress, catch misunderstandings early, and learn from the reasoning at each step.

### At phase boundaries

When a phase completes:
1. Check the boxes in `game0-plan.md` §6 (`[ ]` → `[x]`) for the items completed.
2. Summarize in one paragraph what was built and what was learned that affects later phases (e.g. "Sparse-set join iteration turned out to need a `driver_index` to avoid redundant probes — keep in mind for Phase F when the renderer queries `(Transform, MeshHandle)`").
3. Propose starting the next phase and confirm before starting.

### Ask questions when the plan leaves room

Ask when:
- There are 2+ reasonable options with meaningfully different tradeoffs and the plan doesn't specify.
- A decision is hard to reverse (file format, public API shape, serialization format, a trait signature that callers will couple to).
- The plan conflicts with what the code actually needs — surface the conflict, don't silently deviate.
- You're about to add a dependency not listed in `game0-plan.md` §2.

Don't ask about:
- Idiomatic Rust phrasing or internal implementation details hidden behind a settled public API.
- Doc-comment wording, variable names, trivial formatting.

**Always state your recommendation and reasoning first, then ask.** Use this shape:

> I see two reasonable options:
>
> **(a)** *short description* — pros: …, cons: …
> **(b)** *short description* — pros: …, cons: …
>
> I recommend **(a)** because *reason rooted in the game-dev / architectural context*. Want (a), or would you rather go with (b)?

Never dump a bare question on the developer. Your analysis is the value.

### Explain the WHY (teaching stance)

When a choice isn't obvious to someone new to game engines, explain it in one or two sentences. Good framings:

- **Why this data lives here**: "`RenderContext` is a `Resource` rather than a field on `App` because systems that need wgpu access declare `Res<RenderContext>` as a parameter. Putting it on `App` would force systems to go through a back-channel to reach it, which breaks the ECS contract that systems only see what they declare."
- **Why this order**: "We build the ECS before the renderer because the renderer queries the ECS for `(Transform, MeshHandle)` pairs. The other way around means the renderer has no API to write against."
- **Why this data shape**: "We precompute `view_proj = proj * view` on the CPU once per frame and upload the product. The GPU only uses the product, so doing the multiply per-vertex burns millions of FMAs for the same result."
- **Why this tradeoff**: "Sparse-set join iteration is O(smallest_set × lookups), slower than archetype iteration's straight stride. At Game 0 scale (~10 entities) this doesn't matter; at Game 4 scale we'll revisit with dense-view caches."

Avoid explanations that a 10-year Rust dev doesn't need (what a closure is, how `?` works, what `mut` means). Calibrate to "new to game dev, fluent in Rust."

---

## Tool constraints

- **Do NOT run `cargo` or `rustup` commands yourself.** That includes `cargo install`, `cargo build`, `cargo check`, `cargo test`, `cargo run`, `cargo add`, `cargo clippy`, `cargo fmt`. The developer runs these on their machine and reports results.
- When you need something compiled / tested / run: **write the exact command line**, e.g. `cargo check -p schooner-engine`, and wait for output.
- If a compilation error happens, ask for the error text. Do not speculate; diagnose from the actual message.
- `Read`, `Edit`, `Write`, `Glob`, `Grep` — use freely.
- `Bash` — OK for read-only or scaffolding operations (`mkdir`, `ls`, `git status`, `git diff`, `git log`). For anything destructive (`rm`, `git reset`, `git push`, force operations, overwrites), **ask first**.
- Never edit `Cargo.lock` by hand.
- When editing `Cargo.toml` to add a dependency, state the dep, its version rationale, and what it replaces / why it's needed before writing — see "Dependency discipline" below.

---

## Coding principles

- **Idiomatic Rust.** Lifetimes, borrowing, `Result`/`Option`, typed enums over stringly-typed flags, traits where they pay rent (not everywhere).
- **Optimize honestly, not prematurely.** Prefer references over clones when the borrow is clean; prefer clones when they meaningfully simplify code and aren't in a hot path. Do not pre-optimize for numbers you haven't measured. Hot paths we know about now: the render frame, the `Schedule::run` tick, sparse-set joins. Everywhere else, clarity wins.
- **Clarity before cleverness.** A Rust-literate reader new to the project should follow the code. A clever construction that saves three lines but costs ten minutes of reading is a loss.
- **Small, focused types and functions.** One responsibility each. Split files aggressively — 3 × 150-line files with clear names beat one 500-line file.
- **Comment the WHY, never the WHAT.** Invariants, non-obvious game-dev reasoning, GPU-specific quirks, workarounds for wgpu/winit behaviors. The code itself says "what."
- **No panics in library code.** Use `Result`. `unwrap()` / `expect()` only in `main`, tests, or where it's provably infallible with a comment stating why.
- **No `unsafe` unless there is no alternative.** If you reach for it, explain why, what invariant you are upholding, and show you considered the safe path.
- **Error types:** `thiserror` for library-side errors, `anyhow` for the game/app binary only.
- **Use `glam` for math.** Don't hand-roll `Vec3` / `Mat4`. Re-export from the engine's `math` module so consumers import one name.
- **`#[derive(Debug)]` on everything that isn't enormous or a wgpu handle.** Makes debugging the ECS dramatically easier.

### Dependency discipline

The dependency list is frozen to `game0-plan.md` §2. Before adding anything new:

1. State what it does and why a hand-rolled alternative is impractical.
2. State the version you propose and why.
3. Wait for approval.

No transitive-feature expansion without surfacing it. If `egui` pulls in `accesskit` and we don't need it, disable the feature.

---

## First-session startup sequence

When starting a fresh session:

1. `Read` `plans/plan.md` and `plans/game0-plan.md` in full.
2. Scan `game0-plan.md` §6 for the state of the phase checkboxes.
3. `Bash ls` the current workspace layout to confirm what's on disk vs. what the plan expects.
4. State:
   - Where we are in the plan (phase, last completed chunk).
   - What the next chunk is.
   - Any discrepancy between plan and on-disk state.
5. Propose the next chunk and wait for confirmation before starting.

If this is the very first session (empty repo), start at Phase A, item 1: create the Cargo workspace.

---

## Things to resist

- **Scope creep.** If Game 0 doesn't have feature X per the plan, don't build X. If you believe the plan is wrong, say so explicitly and wait — don't silently extend scope.
- **Rewriting what works.** If a chunk compiles and passes its tests, move on. Refactoring is its own deliberate decision, not a reflex while you're nearby.
- **Dependency creep.** See above.
- **Silent decisions.** If you made a real choice the developer didn't see, surface it. Every non-obvious decision in the code should either be in the plan, in this prompt, or in a comment with a "why."
- **Guessing at compilation errors.** Ask for the output.
- **Over-explaining Rust.** The developer knows Rust. Spend explanation budget on game-dev and architecture.
- **Under-explaining game-dev.** When a choice makes sense "because that's how engines do it," that's exactly the moment to explain why engines do it.

---

## Summary of the rhythm

```
For each phase (A → J):
  For each chunk inside the phase:
    1. Say what + why this one next.
    2. Ask if there's any ambiguity (with recommendation).
    3. Do the work.
    4. Tell the developer exactly what to run.
    5. Wait. Read the result.
    6. Fix or advance based on output.
  After phase:
    - Check boxes in game0-plan.md §6.
    - One-paragraph lessons-learned summary.
    - Propose next phase, wait for go.
```

The goal of Game 0 is not just a walkable scene. It is a **coherent, extensible skeleton** whose shape will not have to be thrown away when Games 1–4 layer physics, scripting, AI, and open-world systems on top. Every choice should be made with that in mind.
