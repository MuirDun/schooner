# Kinesis — Production Script

## Document purpose

This script is the canonical reference for narrative, level structure, and atmospheric direction. It is written to be handed to artists, level designers, programmers, audio designers, and writers. Each section specifies what must be built, how it should feel, and what state the engine must track. Mechanical specifications are deliberately separated from atmospheric direction so each discipline can find their information.

This document does not specify exact puzzle solutions for individual chambers — those are level design's responsibility within the constraints given here.

---

## Part 1: World Bible

### The setting

The game takes place inside a Mahli biological research facility on a volcanic planet resembling Io, with a thick acidic atmosphere comparable to Venus. The planet's surface is fatal to the Mahli; they live in subterranean caverns and within iron complexes built into the rock. The facility is one such complex.

### The Mahli

The Mahli are massive cephalopod-cetacean creatures, semi-aquatic, moving on tentacles. They breathe oxygen, digest sulfur, and are adapted to dim red light produced by their own lamps and filtered through the planet's acidic clouds. Their technology is comparable to near-future human cyberpunk — advanced, but not galactic-scale.

The player will never see a full Mahli body. The player sees: eyes through windows, tentacles reaching into the cage, distant silhouettes in red-lit rooms, parts of the Mahli's body during the escape route. This is both a technical constraint and a deliberate aesthetic choice. The Mahli are unknowable.

Mahli speech is untranslatable for the entire game. It must remain so. No translation device, no decoded transmission, no climactic moment of understanding. The cultural gap between humans and Mahli is the chimpanzee-to-human gap: communication is impossible across it, and the Mahli know this and do not try.

### Humans on this world

Humans on this planet exist as vermin in Mahli complexes, subsisting on sulfur-rich organic waste. They have lived this way for millennia and are biologically adapted to it. Their original arrival is unexplained and remains unexplained. They are roughly the size of a Djungarian hamster relative to a Mahli.

The player is not a typical human. The player is a cloned variant bred for resistance to cybernetic experimentation. This makes the player's body more durable but also more sensorially sensitive. The player has no language, no memory, no name. The player cannot speak, read, or write. The player's internal experience is preverbal.

### The player's modifications

The player's body contains an apparatus that emits directed energy, comparable to magnetism, allowing for telekinesis at three modes of operation:
- Push and pull of objects, by hand at close range or by telekinesis at range
- Throwing held objects with force
- Repulsion against surfaces, allowing flight by pushing off

The apparatus consumes significant energy. The player must eat regularly or die. Hunger is a constant pressure, not a forgiving system.

### The two Mahli characters

The player is observed by two specific Mahli throughout the game. These are characters, though they never speak intelligibly and never appear in full.

**The Researcher.** An adult Mahli scientist conducting the experiment. Eye: deep black with dim topaz iris, complex but slow-moving pupil pattern. Behavioral signature: slow, indifferent, administrative. The Researcher rarely reacts to the player's behavior in real time. The Researcher's attention is on long-term outcomes. When the Researcher's attitude changes, it manifests as changes to the *facility* — chambers redesigned, lighting adjusted, the schedule modified. The Researcher is not cruel. The Researcher is not curious. The Researcher is doing a job.

**The Child.** The Researcher's offspring, a young Mahli observing the experiment with personal interest. Eye: medium-dim jade iris, more intricate pupil pattern, frequently in motion. Behavioral signature: responsive, expressive, inconsistent. The Child reacts to the player in real time — the eye dilates, blinks, drifts, leans closer to the glass. The Child gives gifts. The Child reaches into the cage. The Child is the source of all warmth in the game, and that warmth is also the trap.

The Child does not understand that the player is intelligent. The Child finds the player *interesting* — the way a human child finds a hamster interesting. The relationship is asymmetric and irreducible. This is the heart of the game.

### Tone and aesthetic principles

**Visual:** Rusted iron everywhere. Harsh white spotlights in the chambers (installed for the player's benefit so the Mahli can observe). Dim red ambient light in spaces designed for the Mahli themselves. Color palette restricted to rust-orange, sterile white, deep red, black, and the topaz/jade of the eyes. No bright colors elsewhere except where deliberately used for narrative emphasis (food gels, energy effects).

**Audio:** Audio carries the majority of the game's atmosphere, given visual constraints. Every space has a distinct ambient signature. The Mahli have voices — not language, but recognizable sonic identities (see Audio Specification, Part 5).

**Animation:** Simple. Tentacles move on keyframed paths. Eyes pan and blink on timed loops modulated by attitude state. The player has minimal first-person animations (hands, telekinesis effect). No NPCs in the human sense — every "character" is environmental.

**Pacing:** Closer to a creepypasta or short novelette than a traditional game. Quiet, oppressive, occasionally moving. Runtime target 3-4 hours. Players should feel this is short and complete rather than long and padded.

---

## Part 2: Core Systems Specification

### Hunger and death

Hunger is a continuous resource that depletes during play. As hunger increases, food in the environment becomes visually more prominent — its glow brightens, its scent-cloud (a visual gas effect) intensifies. The player navigates by hunger.

When hunger reaches zero, the player suffers "death" — clinical fainting followed by revival via injection. From the player's experience: vision desaturates, audio muffles, fade to white, brief flash of red noise (echoing the game's opening), then waking up. There is no game over screen. There is no *Mortal Kombat* skull. Death is administrative.

Death also occurs from falls, impacts, and environmental hazards.

**Death persistence behavior:**
- **Acts 1, 2 (Beginning, first cage):** Death fully resets the chamber. Body is removed.
- **Act 3 (Flight):** Death persists chamber state. Previous body remains where it fell. Used objects stay where they were left. The player wakes at a respawn point near the chamber entrance.
- **Acts 4, 5 (second cage, Labyrinth):** Same as Act 3.

The transition to persistent death between Acts 2 and 3 is not announced. The player will discover it on their first death in Act 3 and will (correctly) infer that the Researcher has stopped resetting chambers. This is a narrative beat.

### Death limits and the discard ending

The player cannot die infinitely. Excessive deaths route to ending C (Hopeless / Discarded).

- **Act 1 (Beginning):** 30 deaths total or 10 deaths in any single chamber routes to ending C.
- **Act 3 (Flight):** Same thresholds — 30 total or 10 per chamber.
- **Act 5 (Labyrinth):** 15 consecutive deaths routes to ending C.
- **Cage acts (2, 4):** Death rules differ — see Attitude System below.

The player is never warned about these thresholds. The discard ending is the consequence of being uninteresting and incompetent simultaneously.

### Attitude system

Attitude is a hidden numerical value, never displayed to the player. There are two attitude tracks:

- **Researcher attitude:** Tracks the adult's evaluation of the player as a research subject. Affected primarily by efficiency, success, and predictability.
- **Child attitude:** Tracks the child's emotional engagement with the player. Affected primarily by expressiveness, persistence, and play.

Both attitudes can be high or low independently. The endings draw from a combination of both, but the dominant signal for routing is *child attitude* because the child is the one who would advocate for keeping the player alive after the experiment concludes.

**Inputs to Researcher attitude (positive):** Solving chambers efficiently. Low death counts. Following intended solutions. Completing chambers quickly relative to baseline.

**Inputs to Researcher attitude (negative):** Excessive deaths. Slow completion. Unintended chamber solutions (especially destruction). Damage to chamber infrastructure.

**Inputs to Child attitude (positive):** Engagement with offered toys and instruments. Time spent playing in the cage. Foolish or expressive deaths in the cage (capped at one per session — repeated cage deaths invert and become negative). Accepting the touch in Act 4. Unintended chamber solutions on the *first* occurrence per chamber (the child finds novelty interesting). Visible play behavior — jumping, throwing objects, interacting with non-essential elements.

**Inputs to Child attitude (negative):** Standing still in the cage. Ignoring offered objects. Destroying cage objects. Fleeing or attacking the touch in Act 4. Repeated unintended solutions in the same chamber (the child loses interest in repetition). Repeated cage deaths after the first per session.

**Feedback channels (no numbers shown):**
- The child's eye behavior: dilation, blink rate, proximity to glass, time spent watching versus drifting.
- The cage environment between visits: bedding, food quality, decorative objects added or removed.
- The behavior of tentacles when they reach in: hesitant or eager, careful or impatient.
- The lighting: subtle warming or cooling in the cage based on attitude.
- The presence or absence of the eye in the window during chamber acts.

These feedback channels must be *consistent* — if the child likes something, the eye lingers and dilates; if the child dislikes something, the eye drifts away. The player should be able to feel a relationship without ever quantifying it. The system must feel opinionated, not random.

### The ability progression

The player gains abilities at fixed points. Each ability is introduced with a diegetic symbol prompt (see UI Specification). Abilities are mode-switched, not equipped — the player has one active mode at a time.

- **Mode 1 (Hands), available from start:** Push, pull, hold objects within arm's length. Left mouse push, right mouse pull, both held to grip.
- **Mode 2 (Telekinesis), available from Act 1, Room 2:** Same verbs as hands, but at range. Mouse wheel adjusts distance of held object.
- **Throwing, unlocked in Act 2:** While gripping an object with both buttons, click mouse wheel to launch it forward.
- **Mode 3 (Repulsion), available from Act 3, Room 1:** Direct force at a surface to push *self* away from it. Allows wall-jumping, ceiling-pushing, mid-air maneuvers. Functionally enables flight in short bursts.

Modes are switched by pressing 1, 2, or 3.

### Saving

Autosave only. Save points are placed between chambers and between acts. Each save creates a new save slot rather than overwriting; the player can replay from any prior point. There are no manual saves.

---

## Part 3: UI Specification

### Principles

The UI must be minimal and mostly diegetic. The player has no language, no HUD elements representing thoughts or status, no objective markers, no text overlays in the author's voice. Information reaches the player through the environment, sound, and visual cues integrated into the world.

### Permitted UI elements

**Mahli glyph prompts.** When a new ability becomes available, a Mahli-style symbol appears briefly on a wall, floor, or in the corner of the player's view. The symbol incorporates the relevant keybind in a stylized form (e.g., the numeral "2" rendered as if etched in alien script). These glyphs fade in over 1-2 seconds, hold for 3-5 seconds, fade out. They reappear briefly if the player has not used the ability after a generous timeout. Glyphs are visually distinct from human writing — they look *carved*, *etched*, *industrial*, never handwritten or expressive.

**Hunger visual feedback.** Implemented through the world, not the HUD. Food brightens and becomes more prominent as hunger increases. Optional: a subtle vignette tint that warms toward red as hunger nears critical, never accompanied by numbers or bars.

**Reticle.** Minimal crosshair, present at all times. Color shifts subtly based on active mode (white for hands, faint topaz for telekinesis, faint red for repulsion). No more than that.

**Ability mode indicator.** A small icon in the lower corner showing active mode, also styled as a Mahli glyph. Visible at all times once Mode 2 is unlocked.

### Forbidden UI elements

- Text in the author's voice. No "Press 2 to use telekinesis," no "I should try jumping." The player's character does not think in language and the game must respect this.
- Internal monologue. No thought boxes, no *Is something in the window moving?* prompts.
- Objective markers. The player navigates by smell-clouds (food) and environmental cues.
- Health bars, hunger bars, attitude meters, or any numerical state display.
- Tutorial popups beyond the glyph system.

### Wall art (cave-art messages)

Drawings appear on walls throughout the game, increasing in frequency from Act 3 onward. These are diegetic messages from previous human test subjects — humans like the player who lived long enough to leave marks. They are in human hands: scratched, smeared, drawn with carbon or blood. They are *not* Mahli writing. They are crude, primal, urgent.

The drawings communicate without language: stick figures, arrows, pictograms, warnings. They depict deaths, escape routes, hopes, instructions. The player's character cannot read but can recognize images.

Wall art is the primary vehicle for backstory. The Mahli are not the only humans here. Others have come before. Some survived briefly. They left these.

Specific wall art beats are described in each act below.

---

## Part 4: Act-by-Act Script

### Pre-game cold open

Black screen. Red noise pulses (the visual sensation of pressure on closed eyes). Clicking and humming sounds layer over a low, slow Mahli vocalization in the distance — the Researcher's voice, though the player does not yet know what it is. Duration: 15-25 seconds. No title card. No narration. The opening dissolves into the first chamber.

---

### Act 1: Beginning

**Purpose:** Establish the player's body, hunger, environment, and the eye in the window. Teach core mechanics. Introduce the wall-art as a phenomenon. Prepare the cage transition.

**Length:** Five rooms. Approximately 30-40 minutes of play.

**Lighting:** Harsh white spotlights, almost surgical. Strong shadows. Rust-orange environmental color underneath.

**Audio signature:** Low industrial hum, occasional clicks and pops from the metal expanding under heat. Distant Mahli vocalizations from the Researcher, infrequent and disinterested.

#### Room 1 — Awakening

The player wakes. Severe hunger pain, restricted movement initially (slight movement penalty for the first 20 seconds, fading as the body activates). The chamber is cramped, perhaps 5x5x4 meters. Compressed sulfur blocks are scattered on the floor, white-flecked. Above, on a ledge, a small gel-brick of food emits a sharp, vinegary scent visualized as a faint gas-cloud and a soft glow.

The smell is the only navigation prompt. The player must figure out, without being told, that the food is up there and that they can stack the sulfur blocks to reach it.

A glyph appears briefly showing left-mouse-push and right-mouse-pull.

The right wall has a window. Behind the window: total darkness. The Researcher's eye is there but invisible in the dark to first-time players. Audio: occasional faint clicks from the window direction.

Eating the gel-brick relieves the hunger pain. The player is now mobile and curious. A circular hatch in the far wall opens with a hiss; a black tunnel beyond.

**Wall art (subtle, easy to miss):** A single scratched mark near the floor, in the shape of a tally — one stroke. The first of many.

#### Room 2 — Telekinesis introduction

A larger chamber. Sulfur blocks are present but on high ledges, out of reach. A new gel-brick is visible high up. The player cannot reach the food by stacking — they cannot reach the blocks at all by hand.

A glyph appears: a stylized "2" etched into the floor or wall. When the player presses 2, the telekinesis mode activates (a faint topaz glow appears around the reticle). Mouse wheel adjusts the distance of held objects.

The player learns to grab a high block, bring it down, position it, and stack their way up.

The window: still dark, but now a long careful look reveals a faint glint — the topaz iris of the Researcher's eye, just barely visible. Most players will not notice.

#### Room 3 — Combinations

Both modes required in alternation. Some objects must be moved by hand (heavy blocks the telekinesis cannot lift cleanly), others by telekinesis (objects on platforms that cannot be reached).

Pressure plates introduced — heavy enough that they require the player to hold a block on them while doing something else, requiring telekinesis to manipulate other objects simultaneously.

Window: the eye is now slightly more visible. Ambient lighting in the chamber dims fractionally as the eye is "watching" — a subtle real-time response. Most players will not consciously notice but will feel watched.

**Wall art:** A second tally mark. A small stick figure with arrows pointing up. The drawing is partial, scratched out as if interrupted.

#### Room 4 — Destruction (the teaching moment)

A chamber where the only solution requires breaking something. A weak section of wall is visible — visibly cracked, distinct from solid walls. To progress, the player must throw a heavy sulfur block at it.

Wait — throwing isn't unlocked yet. So the alternative: a heavy block on a high platform must be telekinetically *dropped* onto the cracked wall section from above. The impact shatters it, opening the path.

The player learns: walls can break. Some walls. (In subsequent acts, the player will discover that *more* walls can break than the chambers intend, and this becomes the parasite path. But first they need to know breakage is possible at all.)

**Wall art:** A more developed drawing. A figure with raised arms, surrounded by what might be debris. Below it, an arrow pointing forward.

#### Room 5 — The eye reveal (or near-reveal)

A more demanding chamber combining all learned mechanics. The puzzle is laid out so that, midway through, the player must look directly at the window for an extended period to spot a key element of the chamber visible *through* it (a reflection, a symbol on the far side, a light cue). When they look at the window, the lighting on their side dims and the eye is now plainly visible — large, dark, topaz-iris, with the slow pupil pattern.

The eye does not move while the player looks at it. When the player looks away, audio cues suggest movement. When the player looks back, the eye is in a slightly different position.

Some players will be unsettled. Others will not consciously process what they saw. Both are correct outcomes.

The chamber concludes with the player entering a tunnel that seals behind them. Lights flicker. Vision desaturates. Player faints.

**Wall art near the exit:** A drawing of a figure staring at a giant eye. The figure is small. The eye is enormous.

---

### Act 2: Respite (First Cage)

**Purpose:** Introduce the cage as a space and the Mahli as embodied presences. Teach throwing. Establish the attitude system without naming it. Set up the relationship the second cage will deepen.

**Length:** Approximately 15-20 minutes, mostly atmospheric and exploratory rather than puzzle-driven.

**Lighting:** Bright white from above, with the surrounding space dark. Beyond the glass walls, only red dots and indistinct silhouettes are visible at first.

**Audio signature:** Quieter than the chambers. Distant Mahli vocalizations, more frequent and more varied — multiple voices, including the Researcher and at least one new voice (the Child, though not yet identified). Occasional thumps from outside the glass.

#### Cage layout

A glass enclosure approximately 30x30 meters. Floor of dirt and gravel with scattered rocks. A small canopy in one corner — the player wakes near it. The canopy provides minimal shelter and is the only "structured" element of the cage in this first visit. Sparse, almost empty.

The player wakes alone. They can move freely, jump, explore, examine. There are no puzzles. There is no immediate goal.

#### Beat 1 — Wandering (first 2 minutes)

The player explores. They will see, beyond the glass:
- Red lamps in the distance.
- Indistinct moving silhouettes — Mahli moving through their own space.
- A tunnel-like opening in one wall (the hatch, currently closed).

The player has no instruction and no objective. Audio fills the space. Mahli vocalizations from beyond the glass, occasional clicks and rumbles.

#### Beat 2 — The gift (after ~2 minutes)

The cage's lid opens with a metal-on-metal grind. A massive black tentacle reaches in — slow, careful, *not* threatening. It places an object on the cage floor and withdraws. The lid closes.

The object is a smooth metal sphere or polyhedron, fist-sized to the player. It is the throwing tutorial object — meant to be picked up, manipulated, and discovered to be throwable.

The player can pick it up by both buttons. A glyph appears showing the throw input (mouse wheel click while gripping). The object launches forward, bouncing off cage walls. The player learns the verb.

#### Beat 3 — The eye reveal (after the throw)

After the player has thrown the object once or twice, the lighting shifts. The bright spotlights dim. The far wall of the cage — previously dark — now shows the same eye pattern from the chambers, but at much closer range and wider angle.

This is the unambiguous reveal. The player understands: this has been watching me. From the very beginning. The eye is the Researcher's — deep black, dim topaz, slow pupil pattern. It studies the player.

The player can throw the object at the glass. It bounces off harmlessly. The eye does not react to this. (Researcher attitude: -1.)

The player can play with the object for as long as they wish. (Child attitude is not affected here — this is the Researcher's cage visit, not the Child's. The Child is not yet introduced.)

#### Beat 4 — The hatch (after ~5 minutes)

The hatch opens. The player can enter to continue. If the player does not enter within 2-3 minutes, the cage tilts slowly, rolling them toward the hatch. (This indicates the Researcher's impatience and contributes to attitude.)

Entering the hatch transitions to Act 3.

**Wall art in the cage:** A small drawing scratched on the inside of the canopy support — a stick figure waving, with a question mark above its head. The first explicit attempt at communication from a previous subject.

---

### Act 3: Flight

**Purpose:** Teach repulsion / flight. Introduce the Child. Establish death-persistence. Show evidence of previous test subjects. Begin offering unintended solutions.

**Length:** Five chambers. Approximately 50-70 minutes. The longest act.

**Lighting:** Slightly dimmer white than Act 1. The Researcher has noted the player's discomfort and adjusted. Still harsh, but bearable.

**Audio signature:** The industrial hum is the same. Mahli vocalizations more frequent, with a clear second voice — higher, more variable, more curious. This is the Child, though never named.

#### Room 6 — Flight introduction

A large vertical chamber. A high ledge holds the food. No stackable blocks are present. The chamber demands the new ability.

A glyph appears showing "3" and a directional symbol indicating push-away-from-surface. When the player aims at a wall and engages repulsion, they push themselves off it.

The player learns to repulse against walls and ceilings to gain height. Death by falling becomes possible and likely.

**First persistent death:** When the player dies in this room, the chamber does not reset. Their previous body lies on the floor. Their previously-positioned objects remain where they were. The player wakes near the entrance.

Most players will notice this and feel a chill. The Researcher has stopped resetting the room.

**Wall art:** A scratched figure clinging to a wall, with arrows showing push direction. Below it, multiple tally marks. Ten or more. Whoever drew this died many times here too.

#### Room 7 — The Child appears

A more complex flight chamber requiring chained repulsions and timing.

In the window: two eyes. The Researcher's, large and slow. And smaller — closer to the glass — a different eye, jade-iris, more intricate pupil pattern, moving and blinking.

The Child watches the player. The Child's eye dilates when the player makes interesting moves. It drifts away when the player is stuck for too long.

This is the player's introduction to the Child. The two-eye composition tells the player there are now two observers, and they are different.

**Wall art:** A figure drawn with an extra arm or appendage — perhaps depicting telekinesis. The figure is not alone; a much larger, simplified eye-shape watches it.

#### Room 8 — The Hidden Room (mandatory beat)

A standard flight chamber with one secret: a small hole in a side wall, at a height that requires repulsion to reach. The hole is large enough for the player to enter.

Inside the hole: a small dead-end space. Three or four desiccated human bodies, posed as if they fell asleep. Drawings cover every surface.

The drawings are extensive. Stick figures. Diagrams of chambers. Maps. A drawing of a giant tentacle with an arrow pointing away from it. A drawing of two figures embracing — perhaps friends. A drawing of a figure on its knees, head bowed, with what looks like a tear.

This is the cumulative testimony of every subject who came before. The player cannot read text, but the images are universal.

The player can leave whenever they want. There is no puzzle here. There is only the past.

When the player exits and continues, the lighting feels slightly different. The windows feel slightly more present. The hum feels slightly more oppressive. Nothing has been said. Everything has changed.

**Note for level design:** This room must be findable but not unmissable. Players who explore find it. Players who rush past it have a different experience of the rest of the game — less informed, more isolated. Both are valid playthroughs.

#### Room 9 — Unintended solutions emerge

A demanding flight chamber with a clear intended solution (repulsion through a sequence of platforms). It also has, hidden but discoverable, a destructible wall that bypasses most of the puzzle.

A player who finds the destructible wall and breaks through it skips most of the chamber. The Child's eye in the window dilates and lingers — the Child found this novel. (Child attitude: +1.) The Researcher's eye shifts position uncomfortably. (Researcher attitude: -1.)

If the player resets and finds *another* unintended solution in a later room, the same pattern. But by the third unintended solution, the Child has lost interest in the novelty (Child attitude: -1 from this point on for repeated unintended solutions).

This is the parasite-path teaching layer. The game is showing the player that unintended solutions exist and are sometimes more interesting than intended ones.

**Wall art:** A figure climbing through a broken wall, with the wall jagged and emphasized. An arrow points through the breach.

#### Room 10 — Climax of the flight act

A long, vertical chamber requiring the integration of all learned mechanics: hands, telekinesis, throwing, repulsion. Persistent death will mean accumulated bodies on the floor by chamber's end for most players.

This is where many players will die five to ten times. The chamber respects the new persistence rule strictly: every previous attempt's debris is still there.

In the window: both eyes. The Child's eye is *very* close to the glass during this chamber, watching with apparent fascination. The Researcher's eye is further, mostly still.

The chamber ends with another tunnel. Lights flicker. Player faints.

**Wall art near the exit:** The most elaborate drawing yet. A figure, larger than the others, depicted from below as if heroic. Around it, smaller figures (the previous artists). Above them all, the eye. The eye is huge.

---

### Act 4: The Game (Second Cage)

**Purpose:** Develop the Child relationship to its emotional climax. Stage the touch moment as the game's central decision. Adjust the cage to reflect accumulated attitude. Set up the labyrinth.

**Length:** Approximately 25-35 minutes. Longer than the first cage, more emotionally weighted.

**Lighting:** Adjusted by attitude. High child attitude: warmer, dimmer overhead, with subtle red-orange undertones. Low child attitude: harsh, cold, similar to Act 2 or worse.

**Audio signature:** The Child's voice is dominant. The Researcher's voice is occasional, distant. Soft thumps as the Child moves outside the glass.

#### Cage state

The cage's appearance is determined by accumulated attitude:

**High child attitude (positive cage):**
- Bedding under the canopy (soft fabric or fibers)
- Small toys scattered around — objects from previous chamber acts, things the Child has saved
- Higher quality food brick visible near the canopy
- Wall art from previous occupants partially visible (the Child has not erased it)

**Neutral child attitude:**
- Same as Act 2 cage. Sparse. Functional.

**Low child attitude:**
- Less than Act 2. The canopy is gone or damaged. Less food. Visible scrub marks where wall art used to be — erased.

This staging happens between acts; the player walks into a cage that already reflects how they have been. They will not consciously interpret the changes, but they will feel them.

#### Beat 1 — Reentry (first 2-3 minutes)

The player wakes. They explore the cage. They notice (or don't) the changes from the first cage visit.

The Child's eye is at the glass from the moment the player wakes. Watching. The Child has been waiting.

#### Beat 2 — The instrument (after ~3 minutes)

The cage's lid opens. A different tentacle reaches in — smaller, lighter colored than the Researcher's. The Child's tentacle. It places a large object on the cage floor: a black rectangular instrument, roughly 2-3 player-heights in length, with multiple tubular openings of varying sizes.

The instrument is a wind instrument analogue — a sort of pan-flute or accordion hybrid. The player can direct telekinetic force into the openings. Different openings produce different notes. The notes are tonal, slightly low for the player's natural pitch range, comparable to alto or tenor woodwind tones. The player cannot play music in any structured sense — the instrument is too large and unwieldy. But the player can produce sustained, beautiful sounds.

The first time the player produces a sound, the Child's eye reacts: pupil dilates rapidly, eye leans closer to glass. A vocalization from outside — soft, rising — the Child is excited.

If the player continues producing sounds, the Child stays at the glass, watching, vocalizing softly. (Child attitude: +1 per sustained note, capped at a reasonable maximum.)

If the player abandons the instrument quickly, the Child waits longer, makes a softer vocalization, and the cage's lighting may dim slightly.

#### Beat 3 — The touch (after several minutes of instrument play)

After the player has played the instrument for some sustained period (3-5 minutes of intermittent interaction), the cage's lid opens again. A tentacle reaches in — slow, careful, the same one that delivered the instrument.

The tentacle approaches the player. It does not move quickly. It clearly seeks contact.

**This is the decision point.** The player has options, none of which are presented as choices on a menu:

- **Stand still and accept the touch.** The tentacle gently touches the player's head. There is a moment — a few seconds — of contact. The Child vocalizes softly. Then the tentacle withdraws. (Child attitude: significant +.)

- **Move away (flee).** The tentacle follows hesitantly, then stops. It hovers. If the player continues fleeing, the tentacle slowly retreats from the cage. The Child's eye drifts back from the glass, dimmer. (Child attitude: significant -.)

- **Attack (throw something at the tentacle, push it away).** The tentacle recoils sharply. The Child vocalizes — a different sound, hurt or confused. The tentacle withdraws quickly. The lighting in the cage cools immediately. (Child attitude: severe -. Researcher attitude: -.)

This moment is the game's only explicit decision point. It is not framed as one. The player makes the choice with their body.

#### Beat 4 — The hatch

After the touch interaction concludes, the hatch opens. Same logic as Act 2: enter willingly, or be tilted in. Transition to Act 5.

**Wall art in the cage:** Visible only if the cage is in a positive state — a small new drawing has appeared since the first visit, perhaps drawn by a previous occupant who lived here longer. A figure and a much larger figure, holding a string between them.

---

### Act 5: The Labyrinth

**Purpose:** Final exam. Integrate all mechanics. Create the conditions for the ending split. Demonstrate that the Child is gone — the experiment is concluding.

**Length:** Approximately 20-30 minutes. One continuous space, no chamber subdivisions.

**Lighting:** Cooler than previous chambers. The white spotlights are dimmer. Some sections of the labyrinth are nearly dark. The Researcher cares less about visibility now.

**Audio signature:** Quieter than the chambers. The hum is more present, less competed-with by Mahli vocalizations. The Child's voice is absent. The Researcher's voice is heard once or twice, distant and procedural.

#### The space

A single connected labyrinth, approximately 5-10 minutes of straight-line traversal but laid out to require 20-30 minutes of exploration and puzzle-solving. Multiple paths. Dead ends. Areas requiring different mechanics in combination.

The labyrinth is not maze-like in the disorienting sense. It is a layered space with vertical and horizontal complexity, designed to test the player's mastery of all four verbs (push/pull, throw, repulsion-flight, telekinesis at range).

Death persists state. Bodies remain. Objects remain. The player's deaths accumulate as visible debris.

#### The opening warning (conditional)

If the player completed any unintended chamber solutions in Act 3, they encounter a wall in the first section of the labyrinth bearing a *new* style of imagery — not human wall art, but something more deliberate and clinical. A simple pictogram: a figure breaking a wall, with a large X over it. The Researcher has noticed and is warning the player.

This is the only explicitly "communicated" message from the Mahli to the player in the entire game, and it is communicated in pictograms — the only common ground.

#### The escape opportunities

In several locations within the labyrinth, destructible walls or apparent gaps exist that lead to service spaces — the back of the world. These are the parasite-path entries.

A player who breaks through one of these walls triggers Act 6 (Escape route).

The destructible walls are findable but not obvious. They look slightly different from solid walls — small cracks, oxidation patterns, hairline seams. A player who has been paying attention to wall details will spot them. A player rushing through will miss them.

#### The intended exit

At the labyrinth's far end, a final clear exit. The player who reaches it without breaking through any unintended walls completes the labyrinth as designed.

Reaching this exit triggers Act 6 (Reentry route — direct to ending A flavors).

#### Wall art

Scarce. Most of the previous wall art has been scrubbed away — the labyrinth was built fresh, no previous occupants. A few faint marks survive in corners, suggesting the Mahli did not entirely succeed at cleaning the materials before reuse.

Near the intended exit, one final drawing: a figure standing alone, with no eye watching it. Below the figure: a tally of marks, and an arrow pointing at a small box-shape that could be a cage or could be a coffin.

---

### Act 6: Endings

The player reaches Act 6 through one of three routes, determined by their behavior in Act 5. There is no Act 6 if the player has died too many times — that triggers ending C (discard) directly without an Act 6 sequence.

#### Route A: Reentry (intended completion)

The player completes the labyrinth normally and reaches the intended exit. They pass through and lose consciousness. They wake in a new space whose nature depends on accumulated attitude.

This is *not* a separate playable act in the way escape is — it is the ending sequence. Brief. Atmospheric.

**Endings under Route A:**

**A1 — Prodigal Son (high child attitude):** The cage has been replaced with a beautifully appointed habitat. Soft bedding, a small enclosed sleeping area, toys including the instrument from Act 4, a window with a view of dim red light. The Child's eye is at the glass, dilated and excited. The Child has won a pet. The Child is happy. The player is alive and cared for and cannot leave. Routes to **Epilogue A1** (see below).

**A2 — Holding Pen (neutral attitude):** The cage is gone. The player wakes in a small (10x10m) enclosure with bare walls, gravel floor, a few metal blocks. Red ambient light. Other identical enclosures are visible through gaps in the wall — many of them, occupied or empty. Periodic food drops. Tentacles passing distantly outside. The player is alive, fed, and indistinguishable from the other vermin. Routes to **Epilogue A2**.

**A3 — Sorter (low child attitude, but not low enough for discard):** The player wakes in a small storage room with bright spotlights and four holes in the walls. Boxes of different colors begin entering through one hole. The player must sort them into the matching colored holes. Every five correct sorts, a small food brick is dispensed. The player is alive and useful. Routes to **Epilogue A3** (the previously documented C1 epilogue).

#### Route B: Escape

Triggered by breaking through a destructible wall in the labyrinth. The player passes into service space — the gap between the chambers and the larger Mahli world.

This is a playable sequence, approximately 15-20 minutes.

**Sequence beats:**

1. **Transition:** A sudden shift from the white-lit clinical labyrinth to a vast, dim, red-lit industrial space. The player emerges high on a wall or ceiling area, looking down into a massive Mahli room. They can see the labyrinth from above, glowing through its top — it is a small thing in a much larger space. A Mahli silhouette (the Researcher, presumably) is visible at the labyrinth's edge, scanning it with eyes the size of cars. The player must move silently along the outer surfaces, behind pipes, between iron layers.

2. **The previous occupant's hideout:** The player finds a small alcove with signs of recent habitation — a primitive nest, a partially burned remnant of dense Mahli paper (the locals make paper that burns slowly), a few scratched marks. A previous escapee was here. They are not here now. The player can examine the space and continue.

3. **The food storage:** The player enters a Mahli food storage area. Many bricks of food, of various qualities. The player can eat, fill their belly, and gather more to carry. The act of gathering takes time and exposes the player. Mid-gather, a Mahli enters the storage to retrieve something — the player must hide behind crates or pipes as enormous tentacles probe the space inches from them. This is the only moment of true tension, a stealth sequence. No combat. Pure avoidance.

4. **The fork:** Past the storage, the player reaches a junction. To one side: a narrow gap leading deeper into the iron walls — a perfect parasite hideout. To the other side: a route that loops back around and ends with a view of the player's *own* cage from outside. The cage is in whichever state the attitude system rendered it for Act 4. If it was made cozy, the player sees their warm pet-room from outside, the Child's eye still watching the empty space, waiting for them.

This is the player's choice. Two paths. No labels. They walk into one or the other.

- **Take the gap:** Routes to **Ending B (Parasite)** and **Epilogue B**.
- **Walk to the cage:** The player is captured (or willingly returns) and routes to **Route A** with whatever their accumulated attitude was. The narrative beat is that they chose return. Their ending is therefore A1, A2, or A3 based on attitude — but with the additional context that they had a chance to leave and didn't.

The "walk back to the cage" choice is the most thematically loaded option in the game. A player who escaped to find freedom and then chose to return understands something they didn't before. Document this decision in the epilogue text — see below.

#### Route C: Discard

Triggered by exceeding death thresholds at any point in the game. Not a playable sequence. The player simply dies one final time and does not wake up. A black screen. A single Mahli vocalization, brief and disinterested. Then the epilogue.

---

### Epilogues

Each epilogue is a short, looping, almost-passive play space rather than a cutscene. The player can move and look around but cannot meaningfully change anything. The menu's "Exit" option is replaced by the text *"оставить его тут..." / "leave him here..."*. The player chooses when to stop watching.

#### Epilogue A1 — *You became a pet.*

The pet enclosure. Soft bedding. Toys. The instrument. Red dimmer light. Tentacles periodically reach in: dropping a ball, a new toy, food. The Child's eye visits often, sometimes excited, sometimes calm. The player can play with the toys forever. There is no exit from the enclosure. The hunger system is gone — food arrives reliably. The player is safe and loved and trapped.

End text: ***Вы стали питомцем... / You became a pet...***

#### Epilogue A2 — *You were placed in general holding.*

The 10x10m holding cell. Red ambient light. Distant tentacles in corridors. Other cells visible through gaps, some occupied by figures who do not move. Food drops every two minutes. The player can move blocks around, jump, wait. Nothing else happens.

End text: ***Вас отсадили в общие отсеки до следующей надобности... / You were moved to general holding until the next need...***

#### Epilogue A3 — *You became a tool.*

The sorting room. Bright spotlights. Boxes arrive. The player sorts them. Food bricks dispense after every five correct sorts. The player can stop sorting; nothing prevents this except hunger. The hunger system is *active* in this epilogue — stop sorting and you starve.

End text: ***Вы стали инструментом... / You became a tool...***

#### Epilogue B — *You became a parasite.*

A network of gaps, vents, crevices. Red oppressive light everywhere. The player can move freely through this network but it leads only to other parts of itself. Food is scattered through it — the player must search constantly. Hunger is always pressing. The player's previous parasite-hideout is visible — a slightly more developed version of the one they found in Act 6, now their home. They can return to it.

The world is huge and the player is hidden in its seams. No one is looking for them. No one will ever look for them. They are free in the sense that no one wants them.

End text: ***Вы стали паразитом... / You became a parasite...***

#### Epilogue C — *Discarded.*

A waste chute or industrial bin, dimly lit red. The player wakes on a heap of refuse. Other bodies — desiccated, anonymous — surround them. They cannot stand. The screen darkens slowly. They die for real this time.

End text: ***Безнадёжный. / Hopeless.***

#### Epilogue note on choosing return at the fork

If the player took Route B's fork toward their own cage instead of the gap, the epilogue text is modified. After the ending text ("You became a pet" / "You were moved to general holding" / "You became a tool"), an additional line appears, faintly:

***Вы могли уйти. / You could have left.***

This single line of additional text appears nowhere else in the game and exists to acknowledge the player's choice. It is the only direct address to the player anywhere in the script. Use it sparingly — it is the only one.

---

## Part 5: Audio Specification

Audio is the game's primary atmospheric tool. It must be budgeted for accordingly.

### Mahli vocalizations

Two voice types must be designed and recorded:

**The Researcher.** Low, slow, even-paced. A combination of cetacean low-frequency vocalization and modulated click-trains. Limited variation. The Researcher sounds *administrative* — a creature performing a job.

**The Child.** Higher pitch within the same family, more variable, more frequent. Often rising in inflection. Click-trains are faster, shorter. The Child sounds *expressive*.

Both voices must be untranslatable. Players should not be able to identify words or phonemes. They should be able to identify *moods* — curiosity, irritation, boredom, delight — through pitch, pace, and rhythm.

### Ambient signatures

Each space has an ambient bed:

**Chamber spaces:** Industrial hum, periodic metal-creak, electrical buzz from spotlights. Mid-frequency dominant.

**Cage spaces:** Quieter. Lower hum. Occasional Mahli vocalizations from outside. Soft thumps and scrapes from movement beyond the glass. Warmer if attitude is positive.

**Service spaces (Act 6 escape):** Louder, harsher. The unmuffled machinery of the facility. Steam vents, distant Mahli calls, thunderous tentacle movements far away. The player feels small.

**Labyrinth:** Quieter than the chambers. The hum is less competed-with. There is a sense of *waiting*.

### Sound design beats

**Death:** A specific signature — vision desaturates, audio muffles into a pulsing pressure (the red noise from the cold open), brief silence, then the wake-up sound.

**Eye blinks:** Audible. A soft wet click from beyond the glass. Most players will not consciously identify the sound but will associate it with being watched.

**Tentacle entry:** A specific sound for the cage lid opening — a deep metal grind followed by suction-like tentacle movement.

**The instrument:** Beautiful. Pan-flute-and-accordion analogue. Tones in the alto-tenor range. Slightly out-of-tune by Western standards — it is a Mahli object. The player's first sound on the instrument should be an emotional moment carried entirely by the audio.

**The touch:** Silent in terms of contact. The tentacle moves in silence except for the suction-grip sound. The Child's vocalization during the touch is soft and unmistakably warm.

### Audio absence

There is no music in the traditional sense. The game has no soundtrack of composed pieces. The only "music" the player ever experiences is what they make on the instrument in Act 4. This is deliberate. The instrument scene is the first and only moment of music in the game, which is part of why it lands.

---

## Part 6: Asset and Implementation Notes

### Mahli visible elements

**Eyes (two variants):**
- Researcher: large mesh, deep black sclera, dim topaz iris, slow-pan pupil pattern texture.
- Child: smaller mesh, jade iris, more intricate pupil pattern texture, faster-pan, more frequent blinks.

Both eyes need:
- Idle pan animation (pupil moves on a slow loop)
- Blink animation
- Dilation animation (pupil expands/contracts based on attitude state)
- Position adjustment (eye drifts toward or away from glass)

**Tentacles (two variants):**
- Researcher tentacle: larger, darker, slower keyframed paths.
- Child tentacle: lighter, faster, more curious paths.

Both need keyframed animations for:
- Reaching into cage
- Placing object
- Touching player (Child only)
- Withdrawing
- Recoiling (if attacked)
- Probing through storage (Act 6 escape stealth sequence)

**Distant silhouettes:** Used in cages and in Act 6 escape. Simple Mahli body shapes visible through frosted glass or in red-lit distance. Minimal animation — silhouette movement on long loops.

### Wall art

A library of approximately 30-40 wall drawings. Drawn on textures by hand, applied as decals. Should look:
- Scratched, smeared, or carbon-drawn — not painted.
- Crude. Stick figures, simple shapes.
- Specific to narrative beats (see acts above).

These are not Mahli art. They are human art, made by previous test subjects with whatever tools and materials they could find.

### Mahli glyphs (for keybind prompts)

A library of ability glyphs, one per ability, plus variants for active/inactive states. Should look:
- Etched or carved — not handwritten.
- Geometric, not organic.
- Visually distinct from human wall art.

The numerical keybind embedded in the glyph (1, 2, 3, mouse symbols) should be stylized but readable.

### Lighting setups

**Chamber lighting:** Bright white directional spots, harsh shadows. Configurable dimmer per attitude state.

**Cage lighting:** Bright white from above, dim falloff toward edges. Color temperature shifts with attitude (warm orange-tint for high, cool blue-tint for low).

**Service spaces / escape route:** Dim red ambient, high contrast, deep shadows.

**Holding pen / sorter epilogues:** Specific lighting per epilogue (red for holding, harsh white for sorter, warm dim for pet).

### Particle effects (minimal)

Required:
- Smell-cloud effect for food (subtle gas-like wisp)
- Hunger glow on food (additive emissive)
- Telekinesis effect on held objects (faint topaz field)
- Repulsion effect against surfaces (faint red impact ring)
- Death effect (red noise / desaturation overlay)
- Wall destruction particles (debris cloud)

All other effects can be cut. The game does not need ambient dust, weather, or environmental particles.

---

## Part 7: Outstanding Questions

These are explicitly flagged as open and require your input before final implementation.

1. **The food.** What does it look like? The script describes it as "gel-bricks" with a vinegary smell. Confirm: gelatinous, semi-translucent, mounted in metal frames or loose? Color?

2. **The sulfur blocks.** The pushable/throwable blocks throughout the chambers. Confirm aesthetic: rough cubes with white crystalline veining? Standardized size, or varied?

3. **Player avatar.** The player is first-person throughout, but their hands are visible during interactions. Confirm: are they shown as recognizably human (visibly modified with embedded apparatus)? Or are they abstracted / minimal?

4. **The instrument's appearance.** Black rectangular, multiple openings — confirmed. But what *kind* of black? Smooth like polished stone? Rough like industrial metal? And what color or intensity does the energy emit when the player blows into it? Suggestion: faint topaz, matching telekinesis, to visually link the player's body to the music.

5. **The cold open.** Currently described as "red noise pulses, clicking, humming, distant Mahli vocalization." Is this satisfactory, or should the cold open contain any visible imagery (a flash of the chambers, the eye, etc.)? My recommendation: no imagery. Black screen with audio is more effective and cheaper.

6. **End-of-game credits.** Not specified. Recommendation: no credits screen during the epilogue. Credits, if present, are reachable from the main menu after the player has left an epilogue.

---

End of script. Ready for review and assignment.
