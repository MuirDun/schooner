# Assets and Implementation Notes

## Mahli visible elements

### Eyes (two variants)

- **Researcher:** large mesh, deep black sclera, dim topaz iris, slow-pan pupil pattern texture.
- **Child:** smaller mesh, jade iris, more intricate pupil pattern texture, faster-pan, more frequent blinks.

Both eyes need:

- Idle pan animation (pupil moves on a slow loop)
- Blink animation
- **Attitude-state animation channels** for the Researcher eye specifically:
  - Curious: pupil dilation, iris glow intensification, lean-toward-glass position offset
  - Annoyed: iris dimming, pupil contraction to fixed point, position-lock (no pan)
- Position adjustment (eye drifts toward or away from glass)

### Tentacles (two variants)

- **Researcher tentacle:** larger, darker, slower keyframed paths.
- **Child tentacle:** lighter, faster, more curious paths.

Both need keyframed animations for:

- Reaching into cage
- Placing object
- Touching player (Child only)
- Withdrawing
- Recoiling (if attacked)
- Probing through storage (Act 5 escape stealth sequence)
- Occasional non-interactive visit (Researcher only, for A1 epilogue at good Researcher attitude)

### Distant silhouettes

Used in cages and in Act 5 escape. Simple Mahli body shapes visible through frosted glass or in red-lit distance. Minimal animation — silhouette movement on long loops.

---

## Wall art (small library)

A library of approximately **10–12 wall drawings**, applied as decals. Concentrated in three locations only — see [`systems.md`](systems.md) §Human wall art and the relevant act files for placement.

Should look:

- Scratched, smeared, or carbon-drawn — not painted.
- Crude. Stick figures, simple shapes.
- Specific to narrative beats.

These are not Mahli art. They are human art, made by previous test subjects (who died for real) with whatever tools and materials they could find.

**No wall art anywhere in chambers or labyrinth proper.** Only the canopy support (cages), the hidden room (Act 3 Room 8), and the escape hideout (Act 5).

---

## Ability glyph overlays (HUD)

A small library of pictographic icons, one per ability verb, plus variants for active/inactive states. Should look:

- Etched or carved metal aesthetic, but rendered as 2D HUD overlay (not in-world geometry).
- Geometric, not organic.
- Faint topaz glow tying to the apparatus and telekinesis effects.

**Pictographic grammar:** mouse-silhouette + button highlight + result arrow. No text, no language, no alien font. The icon shows the input and the result, nothing else.

The glyphs are perceptual UI, not diegetic objects. They are the player's awareness of their own apparatus's affordances, not Mahli teaching tools. See [`themes.md`](themes.md) for the principle.

**Total unique pictograms: ~5–6** (push/pull, telekinesis range, throw, repulsion, mode-switch indicator).

---

## Lighting setups

**Chamber lighting.** Bright white directional spots, harsh shadows. **Modulated by Researcher attitude:** warmer tint and slight softness at high; default at neutral; harder, colder, slightly more contrast at low. Lighting changes must not alter visibility for puzzle-solving — only mood.

**Cage lighting.** Bright white from above, dim falloff toward edges. **Modulated by Child attitude:** warm orange-tint at high, cool blue-tint at low.

**Chamber material treatment.** Iron surface material has three states — polished/new (high Researcher attitude), default (neutral), pitted/corroded (low). This is a material override on the same geometry, not a different chamber.

**Service spaces / escape route.** Dim red ambient, high contrast, deep shadows.

**Epilogue lighting:**

- A1: Warm dim red.
- A2: Red dim, permanent and unchanging — the most rigorous lighting in the game.
- A3: Harsh white spotlights.
- B: Red oppressive throughout.
- C: Dim red, fading to black.

---

## Particle effects (minimal)

Required:

- Smell-cloud effect for food (subtle gas-like wisp)
- Hunger glow on food (additive emissive)
- Telekinesis effect on held objects (faint topaz field)
- Repulsion effect against surfaces (faint red impact ring)
- Death effect (red noise / desaturation overlay)
- Wall destruction particles (debris cloud)

All other effects can be cut. The game does not need ambient dust, weather, or environmental particles.

---

## Declarative state authoring note (Glyph / Chronicle fit)

The attitude state machine, eye state, chamber material override, cage state, hunger curve, and ending routing should all read from a single declarative world-state representation.

Imperative example to **avoid:**

```
on_chamber_complete():
    researcher_attitude += 1
    update_eye_animation()
    update_chamber_lighting()
    update_food_appearance()
    ...
```

Preferred shape:

```
world_state.researcher_attitude after chamber complete: +1
material override on iron walls in chambers: { high: polished, neutral: default, low: pitted }
eye animation state: derived from researcher_attitude curiosity/anger events
food appearance: derived from researcher_attitude
```

The first reads as a procedural cascade; the second reads as rules over state. **Game 4 (Chronicle) will require the second form anyway. Write it that way from the start.**
