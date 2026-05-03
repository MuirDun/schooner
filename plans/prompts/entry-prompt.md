@plans/prompts/game0-dev-prompt.md

We are starting implementing Phase _ of the current game. Review what was done before and get acquainted with the project.

First, let's discuss the idea of how it would be implemented. Check whether the plan is valid or might be adjusted before starting implementation. After we have a solid idea of how it would work — make an overview of it in `plans/architecture/` (idea-level: what it is and how it should work, no struct shapes or signatures that rot).

---

## Current state of the project (read this first)

- **Game 0 (The Void) is complete.** The engine has: a sparse-set ECS with per-component change-detection ticks (Tier 1 reactive substrate); a wgpu forward renderer with Blinn–Phong + one directional light; first-person camera; winit window + input; egui debug overlay; puffin profiler with a custom in-overlay scope viewer; CI matrix on macOS / Linux / Windows; a `bench-ecs` benchmark crate.
- **The active game lives in `crates/game/`.** The crate's name stays `game` regardless of which game is being developed; its contents change with each new game.
- **Previously shipped / finished games live in `games/<n>-<name>/`.** Excluded from the workspace `members` list. Each has a README pinning the engine commit it last built against; reviving an old game means checking out that commit. The workspace builds only the active game by default.
- **Architecture vision lives in `plans/architecture/*.md`** — `overview.md` (four pillars + five layers), `ecs.md`, `world-state.md`, `language-binding.md`, `glyph.md`, `chronicle.md`, `ai.md`, `reactivity.md`, `rendering.md`. These describe the *idea*; concrete shapes (types, signatures) live next to the code.
- **Game progression lives in `plans/plan.md`** (Games 0–5, Game 2 split into 2A and 2B). Game 0 done; Game 1 (Kinesis — physics puzzle) is next.
