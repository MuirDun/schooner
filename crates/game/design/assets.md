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

## Palette

Five colors, each with a job. Nothing is decorative; the tools deliberately get none of them.

| Color | Job |
|---|---|
| **Cool steel gray** | The field — walls, the indifferent world. |
| **Warm rust** | Environmental accent — corrosion, "in use, hostile air." |
| **Sulfur gold (glowing)** | The beacon — food, the thing you want. |
| **Deep red** | The Mahli's native light — comfort you can't reach. |
| **Topaz / jade** | The apparatus and the eyes — your body, and the watchers. |

---

## Chamber surfaces (iron & floor)

The chamber shell is **hot-rolled mill-scale steel** — the cheap, fast, unfinished structural metal of this world. Iron here is what plastic is to us: force-shaped by the Mahli, hastily welded, cut apart and re-welded into the next chamber. Not cold-rolled (too finished, too clean) and not painted (a finishing step a disposable test-box never gets).

- **Base color.** Desaturated, faintly cool, *medium*-value blue-gray. Brightness comes from the lighting, not the albedo — a near-white surface blows out and reads plastic; medium blue-gray under the harsh lamp reads bright *and* raw, and leaves the color grade in control of mood.
- **Rust.** Warm orange oxide, concentrated at **seams, welds, low corners, and under fixtures** — never blanketed. Rust is *active corrosion from the acidic air and use*, not abandonment: following the joints it reads operational, smeared everywhere it reads derelict. It is also the only warm note in a cool room — required, not optional (all-cool steel reads as sterile digital teal).
- **Not an ideal box.** No smooth scaled cubes. Panel seams, raised weld beads, a few bolt rows, beveled (never razor) edges, slight warp, and **mismatched panel ages** — one fresh bright panel welded beside a corroded one is the strongest "reconfigured, still in use" signal. Give this to the hero walls the camera lingers on; far/blockout walls can stay simple.
- **Floor.** Same steel, slightly more worn and polished from traffic — a tighter highlight than the walls under the raking lamp.
- **Finish.** Bare scale has a low broad sheen; rust is dead matte. A hand-painted **specular/gloss mask** (not a PBR roughness input) carries the difference — the moving glint on bare metal is what sells "metal" as the player walks.

The **three attitude states** (material override, see [`systems.md`](systems.md) §Chamber comfort) are amounts of corrosion on this same steel: **high** = less oxide, more sheen (newer, cared-for); **neutral** = mill scale with rust at the seams; **low** = heavy corrosion and pitting spreading off the joints across the panels.

**Aesthetic target: 2005–2009 (Half-Life 2 / Gothic 2 / Witcher 1).** The craft lives in **hand-painted and baked diffuse** — rust, grime, and ambient occlusion are painted/baked into the albedo by hand, not computed by the shader. Subtle normal maps, no PBR data stack, no SSAO. The hand of the artist should be visible: a *painting of a place*, not a photo and not a cartoon.

---

## Interactive objects (tools)

The cubes, spheres, and rods the player manipulates to pass chamber tests. **Forged / machined from the same iron as the walls — but dense, smooth, dark, near-matte, with minimal rust** (handled and wiped; smooth dense metal corrodes slower). The walls are rough, corroded, hastily welded; the tools are smooth, deliberate, made on purpose. *The apparatus's instruments get precision; the cage gets none.*

**Affordance without teaching.** The player must read "I can grab this" with no Mahli marking — paint, outlines, and markers are all Mahli-telling-the-player and forbidden (see [`themes.md`](themes.md)). The signal is **form and material contrast**: a smooth dark deliberate mass in a rough corroded room reads as grippable on its own (the Half-Life 2 / Portal language — props are known by form and placement, never by paint).

- **Resting:** form/material contrast only. No glow.
- **Targeted / held:** the faint topaz field (see Particle effects). Topaz = the apparatus = the player's own body, so it is *self*-perception, not a Mahli cue.
- A resting topaz sheen is **off by default** — props that glow at rest is the arcade tell that fights the painterly look.

**Type family** (final set follows from the Part 2 verb design):

- **Cube** — 2–3 sizes. The workhorse: push / pull / stack / weight / throw.
- **Sphere** — rolling mass; settles into channels and sockets; good throw.
- **Rod / bar** — spanning, levering, jamming; orientation matters.
- **Pressure plates** — *fixed receivers*, chamber-side (a recessed socket in the wall/floor material), not carried.

One material across cube / sphere / rod so they read as one family: *if it is this dark smooth dense metal, I can grab it.* Resist a zoo of one-off objects.

**Revises the earlier "sulfur blocks":** the throwables are forged metal, **not** sulfur. Sulfur belongs to food only — fusing the two makes "throw it or eat it?" unreadable under the hunger system.

---

## Food (the gel-brick)

Humans here are aliens subsisting on processed Mahli waste — there is no food made *for* them. The design target is **uncanny appetite**: the player's body must *crave* it while the human at the keyboard recoils. Both at once.

- **Form.** A wet, semi-translucent, glistening **sulfur gel, glowing faintly from within**, lumpy and secreted, pressed into a crude brick. Not a dry briquette (dry reads dead, un-craved).
- **The dissonance.** Wet + translucent + inner glow → the body reads *fresh, nourishing* (honey, roe, rendered fat). Sulfur yellow-green + secreted texture + vinegar reek → the mind reads *bile, rot, pus*. Wet-and-glowing makes you hungry; sickly-and-secreted makes you sick.
- **It is their garbage.** Processed Mahli waste — the best meal is their *nicer* refuse, the worst is dried scraps. The indifference theme, in the food itself.

**Engine fit** (every piece already exists):

- Inner glow = **emissive**, and hunger *brightens emissive as the player starves* — the gel glows brighter the hungrier you are. It calls to you.
- Translucency = the **alpha-blend + Fresnel** glass material (a wet glassy gel surface).
- Scent = the **gas-wisp smell-cloud** particle.

**Presentation by Researcher attitude:**

- **High:** cleaner metal brackets, richer / warmer / glossier — more appetizing.
- **Neutral:** extruded into crude metal brackets / troughs.
- **Low:** loose on the floor, dull, congealed, crusted.

**Guardrails:**

- Food is the **licensed vivid color** — [`world.md`](world.md) blesses bright color for "food gels, energy effects." The sulfur-gold glow is *meant* to be the one beacon in the cold room. Don't tone it down to fit the palette; it is meant to break it.
- **Never signal in-world disgust.** No recoil, no "ugh." The character wants it; the disgust is ours, reaching them through hunger. That gap is the horror.

---

## Lighting setups

**Two light languages.** Red is the Mahli's *native* light — dim, warm, soft, made by their own emissive-panel / electroluminescent / gel tech (not bulbs), the light of spaces built for them. White is a *deployed instrument* — alien to the Mahli's own red-adapted eyes, installed only so the specimen can be recorded. The chamber's horror is being lit in the wrong one: harsh foreign white on you, the comfortable red always on the *other* side of the glass.

**Chamber lighting.** A single hard, **steady**, cold high-intensity discharge lamp (metal-halide / xenon-arc family, ~5500–6500 K, faintly blue) — an **interrogation lamp / vivarium husbandry lamp**, not a room light. Mounted unreachably high in an oversized, crude hasty-iron housing (the Mahli's 50× scale). It lights the specimen for the *recording sensors*, not for any eye. **Steady, never flickering** (flicker reads derelict/haunted; an indifferent surveillance instrument never wavers). **Hard, not soft** (tight cone, hard shadows — instrument, not comfort). **Modulated by Researcher attitude:** warmer tint and slight softness at high; cold default at neutral; harder, colder, more contrast at low. Lighting changes mood only — never visibility for puzzle-solving.

**Chamber atmosphere (haze).** A thin, *homogeneous* chemical haze — the acidic air, tinted faintly toward the grade — just dense enough to make the lamp's god-ray cone visible and the air feel hostile; it may pool low (heavier-than-air gas at the specimen's level). **Not** drifting dust motes: floating particulate reads neglect. The air is the hazard, not the decay.

**Cage lighting.** Bright white from above, dim falloff toward edges. **Modulated by Child attitude:** warm orange-tint at high, cool blue-tint at low.

**Chamber surfaces.** See §Chamber surfaces — hot-rolled mill-scale steel, rust at the seams, three corrosion states overriding the same geometry by Researcher attitude.

**Service spaces / escape route.** Dim red ambient — the Mahli's native emissive-panel / gel light — high contrast, deep shadows.

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
