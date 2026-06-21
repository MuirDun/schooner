# Schooner Engine — System Overviews

Per-system **as-built state + near-term roadmap**. These docs sit between two
existing layers of documentation:

- `architecture/*.md` + `plans/architecture/*.md` — the timeless **idea**: what a
  system *is* and *why*. No build status, no schedule, nothing that rots.
- `plans/plan.md` — the cross-game **staging**: which Game introduces what, and why.

This folder is the **middle term**: for each engine system, *what exists in the
code today* and *what Kinesis (Game 1) still needs, by Part*. Unlike the
architecture docs, these may name concrete current state — but they avoid
struct/signature detail that rots; for that, read the code. They are **living**:
when a Kinesis Part lands work in a system, update that system's doc.

## Why this folder exists

The Part-2 readiness audit (2026-06-19) found the architecture docs were
trustworthy on *idea* but silent or stale on *current build state* — e.g. they
imply Tier-1 change detection is fully built when only the mutation half is. The
fix is not to pollute the idea docs with status that rots; it is to keep status
**here**, next to the roadmap, where being current is the whole point.

## Kinesis Parts (the schedule these docs map onto)

From `crates/game/plan.md`. Parts 1–4 are tech buildouts converging on the
playground; 5–6 are content; 7 is ship.

| # | Name | Owns (engine-relevant) |
|---|------|------------------------|
| 1 | Mood ✅ | Per-instance materials, spot/point lights + shadows, post-FX v0 (bloom/exposure/grade/vignette/fog/god-rays), glTF+PNG assets |
| 2 | **Verbs** (next) | **Rapier physics + ECS bridge; force/grab/throw/repulse; triggers & pressure plates; destructibles; the first Tier-2 event channel; mode switching; particles; HUD glyphs** |
| 3 | Watcher | Positional audio v0; eye-render shader + state channels; keyframed transform-track animation; death-sequence overlay; frosted-glass refinement |
| 4 | Mind | Single declarative world-state; attitude/hunger as state; derived signals; death+respawn; per-scene closed-schema save v0; chamber-state persistence |
| 5–6 | Content | Scene loader + transitions; act authoring on matured kit |
| 7 | Ship | Strip playground; release |

## Index

- **[events.md](events.md)** — the reactive backbone: state + change-detection
  vs discrete `Events<T>`. **Read first; physics, input, and gameplay all build on it.**
- [ecs.md](ecs.md) — storage, query, schedule, change-detection.
- [physics.md](physics.md) — Rapier integration (new in Part 2).
- [input.md](input.md) — raw polling (Layer 1) + the action layer (Layer 2).
- [graphics.md](graphics.md) — forward renderer, post-FX, and the in-game dev-draw posture.
- [bridge.md](bridge.md) — App / frame loop / time / resources, the "always running"
  posture, and the future scp seam.

## The two cross-cutting commitments these docs share

1. **Polling, never callbacks.** Every reaction is a *system that polls* declared
   inputs and writes declared outputs — never a registered listener. This is the
   engine's standing rule (`architecture/input.md`, `camera.md`) and the event
   system (events.md) extends it rather than breaking it. It is also what keeps
   game logic portable: a future Glyph VM is just one more polling system over the
   same substrate, so Rust-now logic ports as a *translation*, not a *rewrite*.
2. **State is declarative; events are discrete.** World facts live in
   components/resources; derived signals *read* state; reactions to change use
   change-detection. Only genuine instantaneous occurrences (a collision, a
   trigger entry, an input edge) become `Events<T>`. This is the
   `crates/game/development.md` "rules over state, not procedural cascades" shape,
   which ports to Chronicle in Game 4.
