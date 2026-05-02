# AI — Agents That Decide and Act

The agent layer (Layer 3 in `overview.md`) is what makes NPCs feel alive. A horror enemy that hunts you, a wolf pack that flanks, a blacksmith who wakes, eats, works, drinks, sleeps — these are not separate AI systems. They are the same architecture under different parameterisations. This document describes what that architecture is, why it has the shape it has, and how it scales from one hunter in Game 2B to thousands of NPCs in Game 4.

---

## How the AI Layer Serves the Pillars

**The world is alive.** Pillar 1 is impossible without agents that genuinely decide. An NPC running a scripted timetable is a puppet; an NPC scoring goals against its own needs and circumstances is an inhabitant. The agent layer is where the world's aliveness becomes legible to the player one NPC at a time.

**Built for one kind of game.** The architecture is shaped for a first-person open-world with a few hundred autonomous NPCs in earshot of the player. It is not built for an RTS with tens of thousands of units, nor for a single tightly-scripted boss. It is built for *this* game and tuned accordingly.

**Developer ergonomics is a feature.** Goal scoring functions, HTN tasks, perception responses, and personality definitions are all authored in Glyph and hot-reload. A designer changes how appealing "go to sleep" is, and the next agent tick reflects it.

**Organism, not castle.** Agents have stable structure (blackboard schema, goal registry, task library) and fluid behaviour (which goals score how, which plans decompose into which tasks). The skeleton is typed; the shape on top changes under hot reload.

---

## The Three Pillars of the Agent

Every agent — the Game 2B hunter, the Game 3 wolf pack member, the Game 4 blacksmith — is built from the same three pieces.

### Blackboard — What the Agent Knows

The blackboard is the agent's perception of itself and its situation. It is the *only* input to goal scoring and plan execution. Anything an agent decides on, it decides on through the blackboard.

What lives on it:

- **Self-state.** Health, stamina, hunger, fatigue, mood, current emotional state. The body's needs.
- **Perceived world.** What the agent has seen, heard, or remembers — last-known player position, visible threats, audible noises with timestamps, nearby allies and enemies.
- **Knowledge.** Facts pulled from the world database when the agent hydrated — faction standing, opinions of others, known places, current job.
- **Plan state.** What the agent is currently doing — the active goal, the current task in the HTN plan, scratch values the plan needs.
- **Pack/group references** (Game 3+). Pointer to a shared blackboard that members of the same group read and write.

The blackboard is the **bridge between Rust and Glyph**. Rust populates the perception fields from the ECS each tick. Rust populates the knowledge fields from the world database on hydration and on demand. Glyph reads from it during scoring and planning. Glyph writes back plan state. The blackboard schema is shared — declared once, visible to both languages.

### Utility AI — What the Agent Wants

A utility AI evaluates many candidate goals and picks the highest-scoring one. Each goal is a Glyph function over the blackboard that returns a score. The agent's behaviour emerges from the relative shapes of those scoring functions.

The shape of a goal:

- A **score** computed from blackboard inputs. Often: a base value, multiplied or added to by need urgency, modulated by personality, capped or zeroed by hard preconditions.
- A **plan** to execute when chosen — the HTN task this goal decomposes to.
- A **commitment policy** — does the agent stick with this goal once chosen until it completes, or re-evaluate every tick? Most goals commit until done or until a higher-scoring goal exceeds them by a hysteresis margin (avoids thrashing between near-equal options).

Goals are **declared, not branched**. There is no "if hungry then eat else if tired then sleep" tree anywhere. There is a list of goals, each scored, and the highest wins. Adding a new goal is adding a function to the registry; it does not require modifying any existing branch.

This matters because:

- **Game 2B has four goals** (patrol / investigate / search / chase). The architecture holds.
- **Game 3 has fifteen** (hunt / flank / retreat / regroup / eat-corpse / attack-structure / mark-territory / …). The architecture holds.
- **Game 4 has twenty or thirty per NPC** (work-job / eat / drink / sleep / socialise / pray / flee / fight / mourn / celebrate / …) plus context-specific ones from the world simulation. The architecture holds.

The same evaluator runs all three. The difference is the size of the goal registry and the richness of the scoring functions, both of which are content.

### HTN — How the Agent Acts

A Hierarchical Task Network decomposes a chosen goal into a sequence of concrete actions. A goal selects a top-level task; the planner expands it into subtasks; subtasks expand further; the leaves are *primitives* — actions the agent can execute one tick at a time.

The shape:

- **Compound tasks** decompose into ordered or branching subtasks. "Investigate noise" → "move to noise source" → "look around" → "respond to evidence."
- **Primitive tasks** are leaf actions. "Move toward this point this tick." "Play this animation." "Attack this target." Primitives produce *commands* the engine applies; they do not directly mutate the ECS.
- **Preconditions** on tasks decide whether a decomposition is valid in the current blackboard state. A task with failing preconditions is pruned; the planner backtracks.
- **Effects** on tasks describe what the blackboard will look like after the task succeeds. The planner uses effects to look ahead — "if I do this, will the next task's preconditions hold?"

Plans are authored in Glyph. A task definition is a Glyph function with a declared signature: blackboard slice in, decomposition or primitive out, effects on success. The planner is Rust; it consumes task definitions and produces a stepped plan.

**Why HTN over behaviour trees:** HTN's decomposition is the natural shape of "how to achieve a goal." Behaviour trees express "what to do next" without an explicit goal model. When goals come from utility scoring, HTN is the matching plan model — they share the goal-and-decomposition vocabulary. Behaviour trees and utility AI are not natural partners; HTN and utility AI are.

---

## Perception — How Knowledge Reaches the Blackboard

The blackboard is updated by Rust-side perception systems each agent tick.

- **Sight.** A vision cone with a frustum and a max range, modulated by lighting and occlusion. Subjects in the cone are checked against the world's spatial index and any line-of-sight occluders.
- **Hearing.** Noise events from Tier 2 (a footstep, a thrown object, a spell) write to a noise field with position, intensity, timestamp. Agents within the noise's effective radius receive the event and write to their blackboard's "heard" slot.
- **Memory.** Perception writes timestamps; the agent's blackboard retains "last seen at X, T seconds ago." Memories decay over game time. Decay rate is per-agent (a soldier remembers threats longer than a deer).
- **World knowledge.** Refreshed on hydration and on relevant Tier 2 world events ("your faction declared war"). Not polled per tick.

Perception runs at the agent layer's tick rate (10–30 Hz for active agents), not at 60 Hz. The reason: perception is expensive (cone tests, line-of-sight, noise-field lookups), and the player cannot tell the difference between an agent reacting in 33 ms and one reacting in 100 ms.

---

## The Tick — What an Agent Does Each Update

When an agent ticks:

1. **Perception update.** Rust populates the blackboard's perception fields from the ECS and the world's spatial structures.
2. **Goal scoring.** All goals in the agent's registry are scored against the current blackboard. The highest-scoring goal that passes the commitment-policy check becomes the active goal.
3. **Plan stepping.** If the active goal changed, the planner runs to produce a fresh HTN decomposition. Otherwise the existing plan continues from where it left off.
4. **Primitive execution.** The current primitive is asked for its command for this tick. The command is written to a buffer; nothing is applied to the ECS yet.
5. **Plan advancement.** If the primitive completed (succeeded or failed), the plan steps forward (or backtracks).

Commands accumulate in a buffer over the agent tick. They are applied to the ECS at a defined sync point — at the end of the agent tick when single-threaded, at the next main-thread tick when threaded. **Agents never mutate the ECS directly.** This is a non-negotiable architectural constraint, the lever that lets the agent layer move to its own thread without rewriting anything.

---

## Optimisation and Scaling

Game 2B has one hunter. Game 3 has dozens of creatures. Game 4 has hundreds to thousands of NPCs. The architecture is the same; what scales are the budgets.

### LOD by Distance and Importance

NPCs near the player (in earshot, in view, in interactive range) are **active** — full perception, full goal scoring, full plan execution at 10–30 Hz.

NPCs in the loaded world but outside the active radius are **reduced** — coarse perception (faction-level threat awareness, no individual sight cones), goal scoring at 1 Hz, plans operating at the level of "go to next scheduled location" rather than per-step movement. They still occupy the ECS as entities; they just think less.

NPCs outside the loaded chunks are **dehydrated** — they exist only in the world database. Their high-level state (location, current activity) is updated by Layer 2 on game-day ticks. When the player approaches, hydration runs and they become active.

The transitions between active / reduced / dehydrated are handled by the AI budget scheduler, not by individual agents. Agents do not know what tier they are in; they just receive ticks at different rates.

### Budgeted Scheduling

Every agent tick has a frame budget. The scheduler runs as many agents as fit; the rest wait until next frame. This is the structural answer to "what happens when there are too many NPCs to tick in 33 ms": some of them tick a frame later, and the player cannot tell.

The budget is set per-tier — active NPCs get most of the budget, reduced NPCs get a small slice. Within a tier, the scheduler round-robins so no single agent is starved.

This is the scaffold that lets threading land cleanly later. The scheduler already processes agents in batches; moving a batch to a worker thread is a wiring change.

### Caching and Reuse

- **Plans are not recomputed every tick.** Once an HTN plan is generated, it is stepped through. Re-planning happens only when the active goal changes, when a primitive fails, or when the blackboard changes in a way that invalidates a precondition.
- **Perception results are reused** within a tick. If three goals all check "can I see the player", the sight test runs once.
- **Spatial queries are batched** at the perception layer. All "what is within radius R of position P" queries from agents on the same tick are batched into a single spatial-index pass.

### Group Behaviour (Game 3+)

A pack blackboard is a regular blackboard owned by the group rather than an individual. Group members reference it from their personal blackboards. Some goals score against the pack blackboard ("flank the target the alpha is engaging"); some plans coordinate through it ("wait for pack signal to attack").

The pack itself is not an agent — it has no perception, no plan. It is a shared scratchpad. The alpha (or whatever role drives group decisions) writes to it; followers read from it. This avoids the trap of building a "group AI" as a separate layer; it stays the agent architecture, parameterised differently.

---

## Threading Posture

Through Game 2B and most of Game 3, the agent layer runs on the main thread. The command-buffer pattern is in place from day one — the agent tick produces commands; a sync step applies them to the ECS.

When profiling demands it (Game 3 horde scenes or Game 4 NPC counts), the agent tick moves to its own thread. Because nothing in the agent layer touches the ECS directly, the move is a wiring change. The blackboards are owned by the agent layer; perception inputs are read-snapshots; commands flow back through the buffer.

Determinism across the thread split is preserved because:

- Agent tick order is set by the scheduler, not by thread arrival.
- Perception inputs are snapshotted from the ECS before the agent tick; updates during the tick do not affect this tick's decisions.
- Commands are applied in scheduler order at the sync point.

---

## Three Time Scales, One NPC

The agent layer is one of three mechanisms acting on a single NPC, each at its own time scale and abstraction level. They run **concurrently**, not in turn. Understanding which mechanism owns which question is the load-bearing distinction in the engine's NPC architecture.

| Time scale | Mechanism | What it decides |
|---|---|---|
| Per-frame to per-second (active), per-second to per-hour (reduced) | **Glyph** via the agent layer described in this document | Goal scoring, plan execution, perception, animation cues, conversation, immediate reactions to threats and stimuli |
| Per game-hour to per game-day (dehydrated only) | **Background simulation** in Layer 2 (`world-state.md`) | Routine activity advancement: location-by-schedule, work output, resource consumption — for NPCs the player is not currently near |
| Per game-day to per game-month (always) | **Chronicle rules** in Layer 2 (`chronicle.md`) | Life events: marriage, illness, promotion, theft, conscription, faction changes, deaths, revolts, succession |

Three points worth being explicit about, because earlier framings of this document understated them:

**Chronicle does not stop running because the NPC is hydrated.** Chronicle ticks on its own world-clock independent of where the player is. If the player stands in front of Aldric for thirty real-world hours, every game-day Chronicle still evaluates its rules; every game-month it evaluates its monthly rules. Some of those rules may match Aldric. When they do, the rule's effect lands in Layer 1 (Aldric's record updates), and a Tier 2 event is published to the agent layer informing it of the change.

**Routine and life events are different questions, not different schedulers.** Chronicle does not have a rule for "go to work at 6am." Routine is not a Chronicle concern at any tier — it is Glyph's when the NPC is hydrated, and the background-simulation tick's when the NPC is dehydrated. Chronicle is for events with consequence: things that change *what kind of person Aldric is* or *what is true about him*, not what he is doing right now.

**Life events arrive at the blackboard via Tier 2.** When Chronicle fires a marriage rule on Aldric while he is hydrated, the rule's effect is twofold: a Layer 1 mutation (the marriage relation is recorded), and a Tier 2 event delivered to Aldric's agent. The bridge re-syncs the affected blackboard slots from his updated Layer 1 record (his "spouse" knowledge slot now points to Mira); Glyph receives the event and decides how it manifests right now — perhaps nothing visible until tomorrow's wedding scene, perhaps a celebratory moment, perhaps he drops his hammer and walks to find Mira. Chronicle decides what is true; Glyph decides how it looks.

This pattern holds for every life event. War is declared and Aldric's faction is involved → Layer 1 records the war, Tier 2 informs hydrated agents in that faction, Glyph reacts (anxiety in the blackboard, conversations turn grim, militia training added to the goal registry as available). His brother dies in a far village → Layer 1 records the death, a Tier 2 event fires to Aldric, Glyph adjusts his mood and may surface a "grieve" goal in the registry. The world keeps simulating around the player, and the player is unusually well-positioned to *witness* the consequences when they hit a hydrated character.

### When the Agent Layer Goes Silent

When an NPC dehydrates, the agent layer stops running on them. There is no blackboard, no goal registry, no HTN plan. The character continues to exist as a Layer 1 record. While dehydrated:

- The **background-simulation tick** advances their location and output according to their authored schedule and current job. This is engine code reading Layer 1 and writing Layer 1, not Glyph and not Chronicle. Per `world-state.md`.
- **Chronicle rules** continue to evaluate them on world-tick. Effects mutate Layer 1.
- No Tier 2 events flow to them, because there is no agent to receive them. Effects are absorbed into Layer 1 only; the propagation to lived experience happens at the next hydration.

When the player approaches and the NPC rehydrates, the bridge reads the current Layer 1 record (which reflects everything the background tick and Chronicle did during the absence) and reconstructs the agent. The NPC the player meets is the cumulative consequence of their absence; the agent layer picks up from this state and resumes ticking.

---

## What Each Game Demands of the AI Layer

- **Game 2B.** One hunter with four goals. Validate the architecture end-to-end. Author goals and tasks in Glyph; tune them with hot reload. The "feels right" target is harder than it sounds — a hunter that perceives plausibly and decides plausibly is the proof the architecture works.
- **Game 3.** Pack blackboards. Group goals (flank, regroup, retreat). Larger goal registries per agent. Budget scheduler under real load. Perception field consumers (noise, scent). The horde defence sections are the stress test.
- **Game 4.** Need-driven scoring with personality weights. Knowledge from the world database flowing into the blackboard. NPC LOD with active / reduced / dehydrated tiers. Hundreds of agents within the player's loaded region, thousands tracked at the dehydrated tier. Threading.
- **Game 5.** Polish. Spells in the immersive-sim substrate (Game 3) interact with agents through perception (a fire near them is a noise + threat) and through status effects (a charmed NPC has different goal weights). The agent layer absorbs this without architectural change.

---

## What the Agent Layer Is Not

- **Not the world simulation.** Layer 2 (Chronicle) handles faction politics, economy, succession. The agent layer handles individual behaviour. They communicate through Tier 2 events and through the world database.
- **Not the player.** The player is a body in the ECS; the agent layer does not run on them. The player's actions are perceived by agents and written into the world database, but the player's decision-making is the player.
- **Not pathfinding or navigation.** Those are Rust subsystems the agent layer consumes. A "move to" primitive calls the navigation system; the navigation system returns a path; the primitive follows it. The agent layer cares that movement happens, not how.
- **Not animation selection.** Animation state is mostly downstream of agent commands — when the agent commands "attack", the animation system picks the right clip. The agent layer's vocabulary is intent, not motion detail.

The agent layer is precisely "what does this NPC want, and how is it pursuing it." Everything else is downstream.
