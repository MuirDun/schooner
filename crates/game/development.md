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

---

## Design-doc constraints that are easy to violate

The themes spine in [`design/themes.md`](design/themes.md) names anti-patterns that the implementation can introduce by accident. Worth re-reading at the start of any content-Part Phase. The most common drift risks for an implementer:

- **No Mahli writing anywhere in the game world.** Pictograms, symbols, alien-font signage, or arranged-world-messages intended for the player to interpret are thesis violations. The HUD glyph overlays are *the player's perception of their own apparatus*, not Mahli teaching — they live in HUD space, never in-world.
- **No music outside the Act 4 instrument scene and Epilogue A1.** Don't add ambient music tracks "for atmosphere." Audio is noise and vocalization, not music.
- **No text in the author's voice.** No "Press 2 to use telekinesis." No internal monologue. The player has no language; respect it.
- **No comfort signal grants a mechanical advantage.** High Researcher attitude warms the lighting and polishes the iron; it does not make the puzzle easier, the food fuller, or the death threshold higher. If you find yourself reaching for a mechanical effect from attitude, stop — that's a design violation, not a feature.
- **Ambient particles are cut.** Per [`design/assets.md`](design/assets.md), the game does not need ambient dust, weather, or environmental particles. Fog and god-rays carry the atmosphere; particles only fire where physics produces them (destruction debris, telekinesis hold field, repulsion impact, food scent-cloud).

If a Step's instruction seems to push against any of these, raise it before implementing.

---

## Localization scope

The only player-facing localized text in Kinesis is in the endings and the menu replacement, per [`design/acts/endings.md`](design/acts/endings.md). Russian + English only. Don't build a general localization framework; a small string table is enough. Game 2A onward may extend; Kinesis does not.
