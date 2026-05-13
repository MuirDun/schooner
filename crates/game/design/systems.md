# Core Systems and UI

## Hunger curve

Hunger is a continuous resource that depletes during play. As hunger increases, food in the environment becomes visually more prominent — its glow brightens, its scent-cloud (a visual gas effect) intensifies. The player navigates by hunger.

**Hunger is dynamic across acts, not uniform.** Treat it as a dramatic tempo line, not a constant background pressure:

| Act | Hunger pressure | Rationale |
|---|---|---|
| Act 1 (Beginning) | Severe | Teaches the resource. Player must learn the verb. |
| Act 2 (First cage) | Off | The player is honored / observed. Food is provided. |
| Act 3 (Flight) | Variable, moderate-to-severe | The Researcher cares about your performance; failure costs you. |
| Act 4 (Second cage) | Off | The Child has things to offer you. You are a guest. |
| Act 5 (Labyrinth + escape) | Severe | The experiment is concluding. The Researcher's interest is rationed. |
| Epilogues | Specific per ending (see [`acts/endings.md`](acts/endings.md)) | |

Cage acts deactivate the hunger drain entirely. The player is fed by the Mahli, on the Mahli's schedule. **The asymmetry — when am I being kept alive, and when am I being made to earn it — is the design statement.**

---

## Death and reincarnation

When the player loses all hunger, suffers a fatal fall, or hits an environmental hazard, they suffer "death" — clinical fainting followed by revival via injection. From the player's experience: vision desaturates, audio muffles, fade to white, brief flash of red noise (echoing the game's opening), then waking up. There is no game over screen. There is no *Mortal Kombat* skull. **Death is administrative.**

**The player is a permanent character.** The player who wakes up after a death is the same player — same continuity, same memories. The bodies the player finds in Act 3's hidden room belong to other subjects, not previous lives.

### Chamber-state persistence in Acts 3–5

- The chamber's *object state* persists across deaths: blocks the player moved stay where they were left, walls the player broke stay broken, used pressure plates remain triggered.
- The *player's body* does not persist. The player simply wakes at a respawn point near the chamber entrance, intact.

This split — **persistent room, transient body** — is the production rule. It preserves the puzzle-mechanic value of accumulated state without contradicting the narrative.

The transition from Act-1/2 reset behavior to Act-3+ chamber-state persistence is **not** announced. The player will discover it on their first death in Act 3 and will (correctly) infer that the Researcher has stopped tidying. This is a narrative beat.

### Death thresholds — the route to C

The player cannot die infinitely. Excessive deaths route to ending C (Hopeless / Discarded), the only true death in the game.

- **Act 1 (Beginning):** 15 deaths total routes to ending C.
- **Act 3 (Flight):** 22 total deaths.
- **Act 5 (Labyrinth):** 15 deaths routes to ending C.
- **Cage acts (2, 4):** 4 self-inflicted deaths in a single cage visit routes to ending C. The cage has no danger — these are deliberate self-harm: jumping from the canopy, smashing into the glass repeatedly. The Researcher discards a self-destructive subject. See Attitude system for the 1-cage-death-as-positive rule.

The player is never warned about these thresholds. **The discard ending is the consequence of being uninteresting and incompetent simultaneously.**

---

## Attitude system

Attitude is hidden numerical state, never displayed to the player. There are two attitude tracks, and they are **strictly domain-separated**:

| Track | Scored in | Not scored in |
|---|---|---|
| **Researcher** | Chamber acts (1, 3) and Labyrinth (5) | Cage acts (2, 4) |
| **Child** | Cage acts (2, 4) | Chamber acts and Labyrinth |

The Child's *eye is visible* in chamber windows during Act 3 and Act 4-adjacent visits, and the eye *reacts* to player behavior (dilates on interesting moves, drifts on stagnation). But Child attitude *does not change* during chamber play. The eye is a window into the Child's interest, not a scoring surface. **The Child is observing, not advocating; the parent is at work and the child knows not to interfere with outcomes.**

The Child's eye is **absent** during Act 5 (the labyrinth). By the time the experiment is concluding, the Child has been removed from the observation deck. This absence is what tells the player the bond is no longer mutable.

### Inputs to Researcher attitude

**Positive:**
- Completing a chamber via the intended solution.
- Low death counts per chamber.
- Brisk completion relative to baseline.
- **The first unintended (novel) solution per act** — the Researcher is curious. Anomalies are scientifically interesting on first observation.

**Negative:**
- The **second and subsequent** unintended solutions across the playthrough. Repeated divergence from protocol is contamination, not curiosity.
- Excessive deaths within a chamber.
- Slow completion.
- Damage to chamber infrastructure beyond what the intended solution requires.

### Inputs to Child attitude (cage only)

**Positive:**
- Engagement with offered toys and instruments.
- Time spent in active play — jumping, throwing objects, interacting with non-essential elements.
- A single foolish or expressive death in the cage (capped at 1 per cage visit — repeated cage deaths after the first invert and become strongly negative; 4+ route to C).
- Accepting the touch in Act 4.

**Negative:**
- Standing still in the cage.
- Ignoring offered objects.
- Destroying cage objects.
- Fleeing or attacking the touch in Act 4.
- Repeated cage self-deaths after the first.

### Feedback channels — Researcher

The Researcher's regard is shown through two coordinated channels: the eye, and the room.

**Eye state.** The Researcher's eye in the chamber window communicates current evaluative state:

- **Indifferent (baseline):** Slow pupil pan, neutral topaz iris, standard blink rate. The Researcher is doing a job.
- **Curious:** Pupil dilates, iris glow intensifies, eye drifts toward the glass, blink rate slows. The eye is *more* present, not less — the Researcher is paying real attention. This state fires briefly after the player executes an interesting move or finds a novel solution for the first time.
- **Annoyed:** Iris dims toward black, pupil contracts to a small fixed point, eye stops panning. The eye is present but cold. Fires once the player has crossed the line on repeated unintended solutions, or after excessive deaths. Persists until attitude recovers.

**This eye-state convention is the most important feedback channel in the game.** It must be consistent. Players will not articulate the rule, but they will read the mood.

**Chamber comfort.** Researcher attitude reshapes the *appearance* of the chambers, **never their mechanics**. Same puzzle, same fullness from food, same physics — but:

- **High attitude:** Iron polished, surfaces look new and cared-for. Lighting warmer (still white, but tinged toward warm). Food gel presented in cleaner brackets, with a richer, more appetizing color and texture. The room *looks like* the Researcher is invested in the player's comfort.
- **Neutral:** Default — rusted iron, harsh white spots, basic gel bricks.
- **Low:** Iron pitted and corroded, spotlights harsher and colder, food presented loose on the floor. The room looks neglected.

**Critical constraint:** comfort signals must never give the player a *mechanical* advantage or disadvantage. No "an extra block placed near the start" at high attitude, no "smaller food brick" at low. Same puzzle, same fullness, same difficulty. **Attitude is pure regard.** Players who feel the difference at all will feel they are being treated better or worse — they will not be playing an easier or harder game.

### Feedback channels — Child

The Child's attachment is shown through the cage between visits, and through the Child's tentacle and eye in real time during visits.

**Between cage visits — cage state.** When the player re-enters the cage in Act 4, the cage's appearance reflects accumulated Child attitude:

- **High:** Soft bedding under the canopy. Small toys scattered — objects from previous chamber acts, things the Child has saved. Higher-quality food brick. Subtle warm light tint.
- **Neutral:** As in Act 2 — sparse, functional.
- **Low:** Canopy damaged or removed. Less food. Cooler light.

**During cage visits.** The Child's eye behavior: dilation, blink rate, proximity to glass, time spent watching versus drifting. The Child's tentacle behavior when it reaches in: hesitant or eager, careful or impatient. The Child's vocalizations: rising and warm when delighted, falling and quiet when disappointed.

**In chamber windows (Act 3, Act 4-adjacent).** The Child's smaller jade eye appears alongside the Researcher's. It dilates on interesting player moves and drifts on stagnation — visible interest in real time. But it does **not** update Child attitude. The Child is watching, not scoring.

---

## The ability progression

The player gains abilities at fixed points. Each ability is introduced with a HUD glyph overlay (see UI section). Abilities are mode-switched, not equipped — the player has one active mode at a time.

- **Mode 1 (Hands), available from start:** Push, pull, hold objects within arm's length. Left mouse push, right mouse pull, both held to grip.
- **Mode 2 (Telekinesis), available from Act 1, Room 2:** Same verbs as hands, but at range. Mouse wheel adjusts distance of held object.
- **Throwing, unlocked in Act 2:** While gripping an object with both buttons, click mouse wheel to launch it forward. See Throwing tutorial soft-escalation in Act 2.
- **Mode 3 (Repulsion), available from Act 3, Room 1:** Direct force at a surface to push *self* away from it. Allows wall-jumping, ceiling-pushing, mid-air maneuvers. Functionally enables flight in short bursts.

Modes are switched by pressing 1, 2, or 3.

---

## Saving

Autosave only. Save points are placed between chambers and between acts. Each save creates a new save slot rather than overwriting; the player can replay from any prior point. **There are no manual saves.**

---

## UI

### Principles

The UI must be minimal. The player has no language, no internal monologue, no objective markers, no text overlays in the author's voice. Information reaches the player through the environment, sound, and a small set of perceptual overlays representing the player's own awareness.

**The Mahli never communicate with the player through UI or world.** Glyph overlays are not Mahli teaching. They are the player's perception of their own apparatus — the way a magnetic field "knows" itself. See [`themes.md`](themes.md) for the principle.

### Permitted UI elements

**Ability glyph overlays.** When a new ability becomes available, a stylized HUD overlay fades in for 3–5 seconds at the edge of the player's view. The overlay uses **pictographic grammar**: a small mouse-silhouette icon with the relevant button highlighted, plus a small arrow indicating the result (push, pull, throw, repulse). Styled with etched, alien geometry and a faint topaz glow to link visually to the apparatus.

- Fade in over 1–2 seconds, hold for 3–5 seconds, fade out.
- Reappear briefly if the player has not used the ability after a generous timeout.
- The overlay is **HUD**, never placed in the world. The player perceives it; the Mahli have not put it there.
- No text. No author voice. Pictographic input-symbol-plus-result-arrow only.

**Hunger visual feedback.** Implemented through the world, not the HUD. Food brightens and becomes more prominent as hunger increases. Optional: a subtle vignette tint that warms toward red as hunger nears critical, never accompanied by numbers or bars.

**Reticle.** Minimal crosshair, present at all times. Color shifts subtly based on active mode (white for hands, faint topaz for telekinesis, faint red for repulsion). No more than that.

**Ability mode indicator.** A small icon in the lower corner showing active mode, styled identically to the glyph overlays. Visible at all times once Mode 2 is unlocked.

### Forbidden UI elements

- **Text in the author's voice.** No "Press 2 to use telekinesis," no "I should try jumping." The player's character does not think in language and the game must respect this.
- **Text in any stylized form representing language.** An alien font that *means* English instructions is the same forbidden thing in costume. Pictographs only.
- **Internal monologue.** No thought boxes, no *Is something in the window moving?* prompts.
- **Objective markers.** The player navigates by smell-clouds (food) and environmental cues.
- **Health bars, hunger bars, attitude meters, or any numerical state display.**
- **Tutorial popups beyond the glyph overlay system.**
- **Mahli symbols in the game world.** The Mahli do not write for the player. Anywhere. Ever. See [`themes.md`](themes.md).

---

## Human wall art (decals)

Wall art is *not* a tutorial vehicle. It is concentrated, deliberate diegetic evidence of previous human presence, present **only in spaces where humans lived** — never in chambers or the labyrinth proper.

The three permitted locations:

- **Act 2 / Act 4 cage** — a small scratched drawing on the inside of the canopy support. A previous subject lived here long enough to leave one mark. A second mark may appear in Act 4 if Child attitude was high enough that the cage was kept rich.
- **Act 3 Room 8 (the hidden room)** — extensive drawings cover every surface. This is the canonical place. Three or four desiccated bodies here, posed as if they fell asleep. These are subjects who died for real.
- **Act 5 escape sequence hideout** — a previous escapee's alcove with a few scratched marks and a primitive nest.

Wall art is in human hands: scratched, smeared, drawn with carbon or blood. It is crude, primal, urgent. It is **not** Mahli writing and never reads as instruction. The player's character cannot read but can recognize images. The drawings communicate without language: stick figures, arrows, depictions of bodies and watchers.

**The asset library for wall art is small.** Roughly 10–12 unique decals, concentrated in the hidden room. See [`assets.md`](assets.md).

The chambers and labyrinth contain **no wall art.** That sterility is the loneliness. See [`themes.md`](themes.md).
