# Schooner Agent Guidance

This repository contains Schooner, a custom Rust game engine built through a sequence of small games. Agents may be used for implementation, architecture, systems design, narrative design, audits, reviews, and planning. These instructions define the durable project contract for all of those roles.

## Project Shape

- The engine lives in `crates/schooner-engine/`.
- The active game lives in `crates/game/`. Its package name remains `game` while the contents change from game to game.
- Finished games live in `games/<n>-<name>/` and are frozen snapshots. Do not modify them unless the user explicitly asks to revive or inspect one.
- The long-term engine roadmap is `plans/plan.md`.
- Architecture intent lives in `plans/architecture/`.
- Current-state overviews, when present, live in `plans/overview/`.
- The active game's production plan is `crates/game/plan.md`; detailed Part docs live under `crates/game/implementation/`.
- The active game's design material lives under `crates/game/design/`.
- Game-specific implementation conventions may live in `crates/game/development.md`.

Plans describe intent. Code describes reality. If they disagree, surface the discrepancy before making changes that depend on it.

## Context Loading

Load the smallest context that can support the current task.

For implementation work, usually read:

- `crates/game/plan.md`
- the current Part doc under `crates/game/implementation/`
- `crates/game/development.md` if present
- relevant engine modules in `crates/schooner-engine/`
- only the architecture and design docs directly related to the requested phase

For architecture work, usually read the relevant `plans/architecture/` docs, current code, and any current-state overview.

For narrative or systems design work, usually read the relevant files under `crates/game/design/`, the active game plan, and only the engine constraints that affect the design.

Do not read every plan, architecture document, design document, and source tree at session start by default. If the task is broad enough to require that, say why before doing it.

## Editing Boundaries

Agents may update implementation progress markers after completed work:

- Step and Phase checkboxes in `crates/game/implementation/*.md`
- Part status or checklist state in `crates/game/plan.md`
- brief implementation notes that record what was actually completed

Do not change architecture intent, design canon, roadmap scope, milestone questions, phase definitions, or game narrative facts as an implementation side effect. If those surfaces appear wrong, explain the conflict and recommend a change. Edit them only after a conversation whose result is an explicit plan/design/architecture change.

Do not edit files in `games/` unless explicitly asked.

## Collaboration Style

Relative only for development mode.

The developer is an experienced Rust programmer and is using this project to learn game engine development.

- Do not over-explain Rust basics.
- Explain game-engine, rendering, ECS, simulation, scripting, AI, and world-design tradeoffs in real depth when they matter.
- State non-obvious recommendations with reasoning before asking for a decision.
- Ask when there are multiple meaningful options, the choice is hard to reverse, the plan conflicts with code, or a new dependency is needed.
- Do not ask about trivial naming, formatting, or hidden implementation details behind a settled public API.

For game development, prefer small playable or inspectable increments. When a step creates a new visible or interactive capability, suggest a short experiment before stacking more systems on top.

## Implementation Rules

Relative only for development mode.

- Keep scope tied to the active game and active phase. Do not build future-game features early.
- But remember about future-game architecture. Do not choose solutions which would block future features, like separate languages, parallelism, etc.
- Prefer existing project patterns over new abstractions.
- Add dependencies only after explaining what they do, why they are needed, and why the proposed version is appropriate.
- Never edit `Cargo.lock` by hand.
- Avoid panics in library code. Use `Result`; reserve `unwrap` and `expect` for tests, binaries, or provably infallible cases with a short reason.
- Avoid `unsafe` unless there is no reasonable safe path; document the invariant if used.
- Use `glam` through the engine's math surface instead of hand-rolled vector or matrix types.
- Comment why, not what.

## Verification

Prefer targeted verification over broad runs. Good examples:

- `cargo check -p schooner-engine`
- `cargo test -p schooner-engine <module_filter>`
- `cargo check -p game`
- `cargo run -p game`

If the session prompt says the developer will run commands, provide exact commands and wait for reported output. Otherwise, run the smallest useful checks yourself when permissions allow. Do not guess at compiler or runtime errors; use the actual output.

## Safety

Ask before destructive operations such as deleting files, resetting git state, force-pushing, or overwriting large generated outputs. Do not revert user changes unless explicitly asked.
