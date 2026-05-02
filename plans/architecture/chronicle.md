# Chronicle — The World Rule Language

Chronicle is the language of the world simulation. Wars, succession, peasant revolts, faction politics, economic rebalancing, character traits firing into action, history accumulating into trend — Chronicle is where these are authored. If Glyph is what the world *does* moment to moment, Chronicle is what the world *becomes* over months and years.

Chronicle and Glyph are two languages over one virtual machine. They share infrastructure (`language-binding.md`); they differ where their domains differ.

---

## How Chronicle Serves the Pillars

**The world is alive.** Pillar 1, in its purest form. The world feels alive because it changes when no one is looking. Chronicle is the language that authors *how* it changes. A peasant revolt is not scripted into a quest by a designer placing a flag; it is the consequence of a Chronicle rule firing when a ruler's cruelty meets a province's unrest. This is the difference between a world that simulates and a world that performs.

**Built for one kind of game.** Chronicle is not a general-purpose rule engine. It evaluates against this engine's relational world database, with this game's relations and this game's facts. The set of operations Chronicle supports — set-returning queries, indexed joins, weighted random selection, structured world mutation, history accumulation — is exactly the set our world simulation needs. Outside this domain it is useless. Inside, it should feel inevitable.

**Developer ergonomics is a feature.** Chronicle is for designers as much as for programmers. Rule authors should be able to read a rule and understand what fires it; should get static errors when they reference a relation that does not exist; should be able to hot-reload a rule and see it take effect on the next world tick. Errors found at content authoring time are bugs not shipped.

**Organism, not castle.** Rules are the body of the world. Adding a rule grows the simulation. Removing one shrinks it. Modders write rules. The engine's behaviour is not a fixed castle of C++ code; it is a fluid body of declarative rules over a typed schema.

---

## The Domain

Chronicle runs in Layer 2 — the world simulation — on its own thread, at game-day and game-month tick rates. It evaluates rules against the world database (Layer 1, `world-state.md`) and applies effects that mutate the database. It does not touch the ECS. It does not run on the main thread. It does not run at 60 Hz.

**Chronicle's tick is independent of player location and NPC hydration state.** This is worth being explicit about because the framing is easy to muddle. If the player stands in front of Aldric for thirty real-world hours, every game-day Chronicle still evaluates its rules; every game-month it evaluates its monthly rules. Some of those rules may match Aldric. The marriage rule may fire on him; the plague rule may fire on his village; the guild-promotion rule may fire on his career. Chronicle does not look at where the player is or whether the matched character is currently an ECS entity. It looks at Layer 1, finds matches, fires effects.

What changes between hydrated and dehydrated is the **delivery** of consequences, not whether they occur:

- **Always:** the rule's effect mutates Layer 1. The fact becomes true in the world.
- **Hydrated character only:** the rule additionally publishes a Tier 2 event to the agent layer, so Glyph can decide how the change manifests in lived experience right now (an animation, a conversation, a goal-registry shift, nothing visible until tomorrow). The bridge re-syncs the relevant blackboard slots from the updated Layer 1 record.
- **Dehydrated character:** no Tier 2 event flows, because there is no agent to receive it. The consequence is absorbed into Layer 1 only and surfaces the next time the character hydrates and the bridge reads their record.

Chronicle decides what is true. Glyph decides how it looks. The agent layer is silent for dehydrated NPCs; Chronicle is not.

What it is for, in concrete terms:

- **Character-driven life events.** Marriage, illness, betrothal, conscription, theft, promotion, religious vision, debt called in, a brother killed in a far village. Things that change *what kind of person Aldric is* or *what is true about him*, not what he is doing right now.
- **Political events.** A baron is cruel and his peasants are restless; a rule fires that may produce a revolt. A duke loses his only heir; a rule fires that may produce a succession crisis. Two kingdoms share a contested border; a rule fires that may declare war. A guild grows wealthy and powerful; a rule fires that may stage a political coup.
- **Economic rules.** Prices respond to supply and demand. Trade routes reroute around dangers. Settlements grow or decline based on safety, food, and trade access.
- **Migration and population.** NPCs move between settlements when conditions favour it. Settlements gain and lose inhabitants over time.
- **History accumulation.** Significant events become facts the database records. Past events are queryable conditions for future rules — "this character has been wronged by that one" is a fact a future rule can read.

What Chronicle is **not** for:

- **Routine activity.** "Aldric goes to work at 6am" is not a Chronicle rule. Routine is Glyph's job when the character is hydrated and the background-simulation tick's job when dehydrated (`world-state.md`). Chronicle is for events with consequence; "going to work" is not an event.
- **Frame-rate gameplay.** Spell logic, combat, real-time AI plans, UI, moment-to-moment interaction — Glyph's domain. Chronicle would fight all of them.

---

## Principles

### Declarative and Set-Returning

A Chronicle rule starts with a query. The query names a starting set ("rulers"), filters it ("rank at most Baron"), joins to related tables ("their primary territory," "their liege"), and filters again ("territory's unrest above 0.7," "liege's culture differs"). The query produces a set of matched scopes; the rule may fire on any or all of them.

This is the operation Chronicle is **for**. It is the reason Chronicle is its own language rather than Glyph with a different schedule. A general procedural language can express this only as opaque lambdas, which the runtime cannot optimise. A declarative query language exposes the query's structure to the optimiser, which compiles it into an indexed plan against the world database. The difference is two orders of magnitude on the same data.

### Trigger → Weight → Effect

Every Chronicle rule has three parts. **Trigger** is the query and condition that decides whether the rule applies. **Weight** is a scoring expression that determines how likely the rule is to fire among other applicable rules. **Effect** is the structured mutation of the world database that happens when the rule fires.

This three-phase shape is the simulation's grammar. Trigger answers "could this happen here?". Weight answers "how likely is it relative to other things?". Effect answers "what happens?". Designers think in this shape. Modders read rules in this shape. The engine evaluates rules in this shape. The shape is the language.

### Statically Typed from Day One

Glyph starts dynamic and grows static. Chronicle starts static and stays static. The reason is the audience: rule authors are designers as much as programmers, and they need the compiler catching their mistakes before any rule reaches a game-month evaluation. A typo in a relation name is a content bug, and content bugs found five game-months into a save are unacceptable.

Refinement types are first-class in Chronicle. An opinion is bounded. A faction standing has discrete categories. A character's rank is one of a fixed set. Where the domain has constraints, Chronicle expresses them, and rule authors get errors at edit time when their condition can never be true.

### Compiled to Query Plans

A Chronicle rule's trigger is not interpreted at evaluation time. It is **compiled** into a query plan against the world database — index scan, filter, join, filter, output. This compilation happens once when the rule is loaded; reload swaps the plan.

The implication for the language: the trigger syntax exposes the query's structure. There is no escape into "give me a lambda over all characters" because that would defeat the optimiser. If the language cannot express a needed query shape, the language grows a new operator; it does not fall back to procedural code.

### No Direct Mutation of the ECS

Chronicle never touches Layer 4. It cannot. A rule cannot spawn a particle, modify a transform, play a sound. When a Chronicle effect needs to influence the immediate world — say, a war begins and an army should march — the effect publishes a Tier 2 event. The agent layer and the ECS pick that event up on their own ticks and translate it into local action.

This separation is what lets the world thread evaluate freely without contention with the main thread. It is what lets Chronicle run ahead during fast travel. And it is what lets the world simulation be tested independently of the rest of the engine.

### Hot Reload Is the Way of Working

Pillar 3, made concrete in Chronicle. A designer tunes a weight; saves; the next world tick reflects it. A new rule drops in; existing facts immediately become available to it. A rule with a syntax error reports the error and the previous version of that rule keeps running. The REPL into the world simulation is not "a debugger" — it is the workshop where the simulation is built.

---

## Why Chronicle and Not Glyph

This is the core of the two-language case. Chronicle and Glyph have **different evaluation models**, not just different schedules.

- Chronicle evaluates queries against a relational store. Glyph executes procedures against ECS components.
- Chronicle's first-class operation is a set-returning indexed join. Glyph's first-class operation is a procedural step that mutates a specific entity.
- Chronicle's "find all matching X" must compile to a query plan to be tractable. Glyph's "do X to this entity" is a sequence of operations.
- Chronicle's authors are designers as much as programmers. Glyph's authors are programmers and designers, leaning programmer.
- Chronicle is statically typed from day one because content bugs are unacceptable on the world thread. Glyph is statically typed by Game 3 because the iteration costs of static typing in early gameplay scripting are higher than the bug rate.

A single language cannot serve both well. A general-purpose Lisp can express trigger conditions as opaque lambdas, but the optimiser cannot see into them; the world simulation becomes O(n) per rule per tick, and the architecture's promise of ten thousand simulated characters dies. A pure rule language can express world rules beautifully but is awkward for procedural gameplay; spell composition fights against the rule shape.

Two languages, one VM, sharing infrastructure where they can — this is the shape that fits. Pillar 2 again: every tool tailored for its task.

---

## Staging

Chronicle is not built before Game 4. The reason is that none of Games 0–3 need a world simulation; building Chronicle earlier is building a tool with no consumer.

- **Game 3.** Chronicle is **designed**. The schema of the world database is sketched; the rule syntax is drafted; the query operations are enumerated. No implementation; the design feeds into Game 4 pre-production with enough detail that the implementation work is execution, not exploration.
- **Game 4 — Chronicle v1.0.** Implementation. Statically typed from the start. Compiles rules to indexed query plans. Hot reload from day one. The world simulation runs on Chronicle and only on Chronicle.
- **Game 5.** Chronicle gains expressive power as the spell system and the deeper RPG mechanics demand new rule shapes — but the language's bones are set in Game 4.

Designing Chronicle in Game 3 also means Game 3's terrain, agent layer, and immersive-sim foundations are shaped knowing what Game 4 will need from them. Information flows backward as well as forward.
