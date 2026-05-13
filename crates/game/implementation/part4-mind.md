# Part 4 — Mind

**Kind:** Tech buildout (state, hunger, persistence, save)
**Status:** Not started
**Depends on:** Part 3 (Watcher) complete

---

## Goal

The world reacts. By end of Part 4, the playground holds a single declarative world-state representation that drives every observable surface from Parts 1–3 as a derived signal. Researcher and Child attitude tracks accumulate from player behavior; eye state, chamber comfort (iron polish level + light tint), food appearance, and ambient mood are derivations of attitude — never direct procedural updates. Hunger ticks dynamically. Food brightens and emits more visible scent-cloud as hunger increases. Death respawns the player without resetting the playground's room state (the persistence rule of Acts 3–5). Death thresholds are counted and route correctly. The save system writes per-scene closed-schema records and restores them on load.

The playground is now a small functioning Kinesis-mind: you can play in it, fail, succeed, and watch the world remember.

## The question this Part answers

**Does the declarative rule-shape feel maintainable, and does chamber-state persistence read as a narrative beat?**

Two questions packed together because they share the same underlying system. The rule-shape question is about *engineering* — if writing "eye state derives from attitude" in Rust is so painful that the team reaches for procedural cascades anyway, the Game-4 Chronicle port becomes a rewrite instead of a translation. The persistence question is about *experience* — when the player dies in Acts 3–5 and wakes to find their displaced blocks still where they left them, that *must* feel like a deliberate withdrawal of care by the Researcher, not a save-system quirk.

## In scope

- Single declarative world-state representation (Rust, but rule-shaped per `design/assets.md` §Declarative state authoring note)
- Researcher attitude track (scored in chamber-type scenes) with the input catalog from `design/systems.md` §Attitude system
- Child attitude track (scored in cage-type scenes) with its input catalog
- Derived signals computed from attitude: eye-state selector (drives Part 3's animation channels), chamber comfort selector (drives Part 1's iron variant + light tint + color grade), food-appearance selector, ambient-audio selector
- Hunger curve as a dynamic resource with per-act-type pressure presets (severe / off / variable per `design/systems.md`)
- Hunger-driven food prominence: emissive intensity and scent-cloud particle density read from current hunger
- Death sequence wired to a `Dying` component / state transition; death triggers respawn
- Chamber-state persistence: per-scene closed-schema serialization of stateful entities (block positions, broken-wall states, plate states, trigger-once flags) preserved across player death in Acts 3–5; reset between acts
- Player respawn: body restored to a scene's respawn point; chamber state untouched
- Death threshold counters per scene type (chamber / cage / labyrinth) and routing logic for ending C
- Unintended-solution detection: first-vs-subsequent unintended-solution tracking across the playthrough (one bit in save state plus per-act counts)
- Save system v0: per-scene closed-schema save records (not generalized ECS serialization); autosave at scene boundaries; slot-per-save (no overwriting); load from any slot
- Cage-state accumulation: between-cage-visit state (canopy condition, toys present, food quality, light tint) materialized from Child attitude on cage entry

## Out of scope

- Glyph (the rule-shape is followed in Rust, not in a scripting language — Game 2A's job)
- Generalized ECS serialization with reflection (Game 2A — Part 4 uses fixed per-scene record types)
- File-watcher-driven hot reload of any of the above (Game 2A)
- Actual chamber content authored for the game's acts (Part 5)
- Cage authored content (Part 5/6 — the cage *type* and its rendering of accumulated state lands here; specific cage layouts are content)
- The instrument's audio system (Part 6)
- Ending routing beyond ending-C threshold logic (Part 6 — A1/A2/A3/B selection and epilogues are authored content)
