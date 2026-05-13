# Kinesis — Production Script

The canonical reference for **Game 1 — Kinesis**, the first narrative game on the Schooner engine. This document is the entry point; the script proper is split into focused documents under `design/`.

---

## Thematic spine

Kinesis is a retrospective on the use of animals as instruments for our interests. *How does it feel from the other side of the cage.*

- **The player is not the point of the experiment.** The cybernetic implant is. The player is a substrate.
- **The Researchers are not cruel.** They are doing a job. Indifference, not malice.
- **The chimp-human gap is irreducible.** The Mahli never communicate with the player at any layer.
- **Loneliness is a core line.** In this facility, humans are only test subjects or wild intruders.

See [`design/themes.md`](design/themes.md) for the full thematic spine and its load-bearing constraints. Every decision in the rest of the document answers to it.

---

## Authoring constraint (Glyph / Chronicle fit)

Game 1 ships before the Glyph scripting language (Game 2A) and Chronicle world-rules language (Game 4). All gameplay logic in Kinesis will therefore be authored in Rust. **However:** the attitude state machine, environmental responses (lighting tint, eye behavior, cage decor, room comfort), and ending routing should be written as **rules over a world state**, not as procedural callbacks scattered across systems. This is a load-bearing constraint, not a stylistic preference — it makes the Game 4 Chronicle port a translation and continuation of existing style, rather than a rewrite and new pattern.

When a designer says "if Researcher attitude is high, the chamber's iron looks polished," that should resolve to a single declarative rule reading attitude state and emitting a material override, not a chain of imperative updates fired from multiple input handlers.

See [`design/assets.md`](design/assets.md) §Declarative state authoring note.

---

## Document map

### Design spine
- [`design/themes.md`](design/themes.md) — the thematic spine and what it forbids
- [`design/world.md`](design/world.md) — setting, Mahli, humans, the two characters, aesthetic principles

### Production
- [`design/systems.md`](design/systems.md) — hunger, death, attitude, abilities, UI
- [`design/audio.md`](design/audio.md) — vocalizations, ambient beds, sound design beats
- [`design/assets.md`](design/assets.md) — visible elements, wall art, glyphs, lighting, particles, declarative-state note
- [`design/open-questions.md`](design/open-questions.md) — items flagged for input before final implementation

### Act-by-act
- [`design/acts/act1-beginning.md`](design/acts/act1-beginning.md) — five rooms, mechanic introduction
- [`design/acts/act2-respite.md`](design/acts/act2-respite.md) — first cage, throwing, eye reveal
- [`design/acts/act3-flight.md`](design/acts/act3-flight.md) — repulsion, the Child, hidden room
- [`design/acts/act4-game.md`](design/acts/act4-game.md) — second cage, instrument, the touch
- [`design/acts/act5-labyrinth.md`](design/acts/act5-labyrinth.md) — final exam, escape branch
- [`design/acts/endings.md`](design/acts/endings.md) — endings A1/A2/A3/B/C and their epilogues

---

## Pacing target

Closer to a creepypasta or short novelette than a traditional game. Quiet, oppressive, occasionally moving. Runtime target **3–4 hours**. Players should feel this is short and complete rather than long and padded.

---

## Status

Ready for review and assignment.
