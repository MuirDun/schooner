# Kinesis — Development Posture

Implementation conventions specific to **Kinesis** that don't generalize to other games. Read this alongside [`plan.md`](plan.md) and the design spine in [`design/`](design/). The general dev-prompt at [`../../plans/prompts/dev-prompt.md`](../../plans/prompts/dev-prompt.md) is authoritative for the work rhythm and tool constraints; this doc only adds Kinesis-specific overrides and reminders.

---

## Build targets

Two binaries live in `crates/game/src/bin/`:

- `main.rs` — the actual Kinesis game. Launched via `cargo run -p game`. Not meaningfully populated until Part 5; Parts 1–4 leave this as a stub or a thin entry.
- `playground.rs` — the dev-only test chamber. Launched via `cargo run -p game --bin playground`. **Primary verification target for Parts 1–4.** Built up alongside the tech across those Parts; survives as the regression bench through Parts 5–6. Dropped from the shipped build in Part 7.

When a Step's verification line specifies "run the game," default to `cargo run -p game --bin playground` for tech Parts (1–4) and `cargo run -p game` for content Parts (5–6) unless the Step says otherwise.

---

## Tech-Part vs Content-Part discipline

Parts 1–4 are **tech buildouts**. They produce engine surfaces (renderer extensions, physics integration, audio system, save mechanism, state representation). They do **not** author Kinesis content. The temptation when telekinesis works in Part 2 is to start sketching Act 1 Room 1; resist. Act 1 Room 1 is Part 5's job and depends on Parts 3 and 4 having landed.

Parts 5–6 are **content builds**. They author the real acts using already-finished tech. If a tech gap surfaces during content work, that is a **bug in an earlier Part**, not new scope. Open the prior Part doc, identify the missing capability, and address it under that Part's heading — do not absorb engine work silently into a content Part.

If the line is unclear, ask the architect.

---

## Declarative state authoring rule

From [`design/assets.md`](design/assets.md) §Declarative state authoring note, repeated here because it is a load-bearing implementation rule for Part 4 onward.

The attitude state machine, eye state, chamber material override, cage state, hunger curve, and ending routing must read from a **single declarative world-state representation**. The eye-state animation, chamber comfort, food appearance, and lighting tint are **derived** signals, not procedural callbacks.

Imperative shape to **avoid**:

```rust
fn on_chamber_complete(world: &mut World) {
    world.researcher_attitude += 1;
    update_eye_animation(world);
    update_chamber_lighting(world);
    update_food_appearance(world);
    // …
}
```

Preferred shape:

```rust
// One place where attitude moves.
fn on_chamber_complete(world: &mut World) {
    world.researcher_attitude += 1;
}

// Derived signals read attitude, not the event.
fn eye_state(attitude: ResearcherAttitude, recent_events: &EventWindow) -> EyeState { … }
fn chamber_comfort(attitude: ResearcherAttitude) -> ChamberComfort { … }
fn food_appearance(attitude: ResearcherAttitude, hunger: Hunger) -> FoodAppearance { … }
```

**Why this matters:** Game 4 introduces Chronicle, a declarative language for exactly this shape of rule. If Kinesis is authored as a procedural cascade, the Chronicle port becomes a rewrite. If Kinesis is authored as rules over state, the port becomes a translation. The cost of writing it correctly the first time is small; the cost of rewriting later is large.

This applies to every gameplay system from Part 4 onward. If a tempting procedural shortcut presents itself, surface it before taking it.
