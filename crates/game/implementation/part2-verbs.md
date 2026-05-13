# Part 2 — Verbs

**Kind:** Tech buildout (physics & player abilities)
**Status:** Not started
**Depends on:** Part 1 (Mood) complete

---

## Goal

The player's mechanical body. By end of Part 2, every Kinesis verb works in the playground: hands push/pull, telekinesis at range with mouse-wheel distance control, throwing, repulsion against surfaces (flight in short bursts), destructibles, pressure plates, trigger volumes. The HUD glyph overlays teach each verb in pictogram form. The mode indicator and reticle reflect active state.

The Part-1 playground acquires physics: the sulfur blocks placed for dressing become grabbable and throwable, a cracked wall is added, a pressure plate is added.

## The question this Part answers

**Do the verbs feel right?**

Telekinesis grab/throw/repulse is the game's mechanical voice — every chamber's puzzle expression depends on these feeling tactile, precise, and a little dangerous. Iterate here until they do.

## In scope

- Rapier integration; physics ↔ ECS bridge
- Force application (directional + impulse)
- Hands mode (Mode 1) — close-range push/pull/hold
- Telekinesis mode (Mode 2) — same verbs at range, mouse-wheel distance
- Throwing — mouse-wheel-click launch while gripping with both buttons
- Repulsion mode (Mode 3) — self-impulse against surfaces, enables flight bursts
- Destructible walls with mesh fragmentation on impact threshold
- Pressure plates (held-state trigger)
- Generic trigger volumes
- Collision events as the first Tier 2 cross-layer channel
- Gameplay particles: telekinesis hold field, repulsion impact ring, destruction debris
- First-person hand mesh with keyframed per-verb poses (no skeletal rigging)
- UI glyph overlay system (5–6 pictograms)
- Mode indicator and mode-tinted reticle

## Out of scope

- Tentacle entry / any Mahli interaction with held objects (Part 3)
- Audio for impacts, throws, repulsion (Part 3)
- Hunger, attitude, save, persistence (Part 4)
- Chamber content beyond the playground (Part 5+)
- Final tuning of telekinesis "weight" and "feel" — bar here is correctness and tactile satisfaction; final tuning happens against real chambers in Part 5
