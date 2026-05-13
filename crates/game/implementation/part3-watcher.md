# Part 3 — Watcher

**Kind:** Tech buildout (Mahli presence & audio v0)
**Status:** Not started
**Depends on:** Part 2 (Verbs) complete

---

## Goal

The Mahli become real presences in the playground. By end of Part 3, the eye behind the frosted window watches the player — panning slowly, blinking, dilating on interesting moves, dimming when the player misbehaves. A tentacle reaches into a cage corner of the playground on cue, places an object, withdraws. Mahli vocalizations come from the right direction in space. The death sequence loops correctly: fade-to-white with red-noise pulse and audio muffling, then waking. The cold-open red-noise can be played on demand.

The playground gains its watcher. The conceit of "being observed across the gap" becomes testable.

## The question this Part answers

**Does the watched-from-the-glass conceit work?**

This is the most fragile load-bearing piece of the game's atmosphere. The eye and its responses must feel alien and authoritative, not cartoonish. The tentacle must feel large and indifferent, not menacing or comic. The positional audio must place the Mahli outside the glass without expensive occlusion modeling.

## In scope

- Eye render: pupil pattern texture pan, blink animation, dilation/contraction, iris glow intensity, position drift (lean toward / away from glass)
- Eye attitude-state animation channels: indifferent (baseline), curious (dilation + glow + lean-in), annoyed (dim + pin-pupil + position-lock)
- Two eye variants: Researcher (topaz, slow) and Child (jade, faster, more variable)
- Tentacle keyframed transform-track animation system; one Researcher tentacle and one Child tentacle with the choreography catalog from `design/assets.md` (reach-in, place, touch, withdraw, recoil, probe)
- Frosted-glass material refinement (eye-reveal trick: when player-side lights dim, the eye becomes visible)
- Distant-silhouette rendering for cage and service spaces (simple Mahli body shapes behind frosted glass at distance)
- Positional audio v0 via `kira` or `rodio`: position + distance attenuation, no occlusion
- Mahli vocalization triggering driven by ECS state (will be wired to attitude in Part 4 — Part 3 exposes the trigger surface)
- Two Mahli voice asset families (Researcher and Child) with mood-readable variants
- Per-zone ambient audio beds (chamber hum, cage quieter, service-space machinery, labyrinth waiting-hum)
- Death sequence: vision desaturation + red-noise overlay (driven through the Part-1 overlay slot) + audio muffling + fade-to-white + wake-up sound
- Cold-open: same red-noise + Researcher vocalization, playable on demand from the playground
- Audible eye blinks (soft wet click from the window direction)
- Tentacle entry sound (deep metal grind + suction)

## Out of scope

- Hunger, attitude tracking, save (Part 4) — Part 3 builds the Mahli's *presentation surface*; the *rules* that decide which state to show land in Part 4
- The instrument (Part 6 — sample-based system specific to Act 4)
- Audio occlusion / muffling-through-walls (Game 2B)
- Skeletal animation (tentacles are explicitly keyframed transforms — never deviate)
- Mahli interaction with telekinesis-held objects (Part 5/6 — choreography in real cages)
- Stealth detection for the Act 5 escape (Part 6 — scripted choreography there, not generalized perception)
