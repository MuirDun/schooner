# Kinesis — Production Plan

Production staging for **Game 1 — Kinesis**, the first ship target on the Schooner engine. This document is the TOC; each Part is staged in its own document under [`implementation/`](implementation/).

**Status:** Not started. Game 0 (The Void) is complete and the engine baseline is the starting point.

**Target:** Ship-quality 3–4 hour first-person psychological-horror physics-puzzle game. See [`script.md`](script.md) for the design TOC and [`design/`](design/) for the spine.

---

## Production approach

### Vertical slabs converging on a persistent playground

The work is sliced into seven Parts. The first four are **technical buildouts** that each converge on the same artifact: a **playground room** that all later content reuses for regression testing. Parts five and six are **content builds** that author the real acts using the playground's matured asset kit. Part seven is **ship**.

Each Part ends at a checkpoint where one specific question is answerable. If the answer is "no," the rework cost is bounded to that Part. Subsequent Parts depend on every preceding Part being load-bearing-correct.

### The playground

A dedicated test chamber, lit and dressed using the *real* Kinesis asset kit — rusted iron, gel-bricks, sulfur blocks, eye-and-window setup, decals — not gray-box primitives. It is built up alongside the tech in Parts 1–4 and survives as the regression bench through Parts 5–6.

**Build-flag-gated.** The playground is launched via a separate binary target, not surfaced in the main menu. It never enters the shipped build. Part 7 removes it (or simply doesn't ship the binary).

This gives us two things:

1. Every tech part's milestone is validated against real-art assets, not stand-ins. If lighting + materials + fog don't carry the rusted-iron mood in the playground, they won't carry it in Act 1 Room 1 either — we find out in Part 1, not Part 5.
2. By Part 5, building an act room becomes mostly composition: place the matured components from the playground into a new scene.

### Authoring posture

Gameplay logic is authored in **Rust** (pre-Glyph), but written in the **declarative-rule shape** specified in [`design/assets.md`](design/assets.md) §Declarative state authoring note. Attitude tracks, eye state, chamber comfort, food appearance, lighting tint, and ending routing read from a single world-state representation; derived signals are computed, not procedurally cascaded. This shape ports to Chronicle in Game 4 as a translation, not a rewrite.

---

## Parts

| # | Name | Kind | Milestone question |
|---|------|------|--------------------|
| 1 | [Mood](implementation/part1-mood.md) | Tech | Does the aesthetic land? |
| 2 | [Verbs](implementation/part2-verbs.md) | Tech | Do the verbs feel right? |
| 3 | [Watcher](implementation/part3-watcher.md) | Tech | Does the watched-from-the-glass conceit work? |
| 4 | [Mind](implementation/part4-mind.md) | Tech | Does the rule-shape feel maintainable, and does persistence read as a narrative beat? |
| 5 | [First Half](implementation/part5-first-half.md) | Content | Does pacing work? Are players reading the eye? |
| 6 | [Second Half](implementation/part6-second-half.md) | Content | Do the endings land? |
| 7 | [Ship](implementation/part7-ship.md) | Release | Is it shippable? |

---

## Engine roadmap impact

Several systems originally staged for Game 2A are pulled into Game 1 because Kinesis cannot ship without them. The engine roadmap (`plans/plan.md`) reflects this re-staging. In summary:

**Pulled into Game 1 (was Game 2A):**
- Spot + point lights with per-light shadow maps
- Post-process pipeline v0 (tonemap, color grade, vignette, height fog, overlay slot)
- Save system v0 (per-scene closed schema — not generalized ECS serialization)
- Positional audio v0 (position + distance attenuation — no occlusion)
- Decals + transparency v0
- Keyframed transform-track animation v0

**Stays in Game 2A:**
- Glyph language + FFI + hot reload
- Generalized scene serialization & file-watcher-driven asset hot reload
- Full post-process pipeline (warm height fog refinements, etc. — Game 1 lands lean version)
- Cascaded shadow maps for outdoor sun (Game 1 uses single-shadow-map-per-light, indoor-scoped)

**Stays out of Game 1 entirely:**
- Glyph (gameplay in Rust)
- Autonomous AI (Act 5 escape stealth is scripted choreography)
- Skeletal animation (tentacles are keyframed transform tracks)
- Outdoor rendering, foliage, day-night, weather
- Audio occlusion (Game 2B)

---

## Cross-references

- Design spine: [`script.md`](script.md), [`design/themes.md`](design/themes.md), [`design/world.md`](design/world.md)
- Production design: [`design/systems.md`](design/systems.md), [`design/audio.md`](design/audio.md), [`design/assets.md`](design/assets.md), [`design/open-questions.md`](design/open-questions.md)
- Engine roadmap: [`../../plans/plan.md`](../../plans/plan.md)
- Architecture vision: [`../../plans/architecture/`](../../plans/architecture/)
