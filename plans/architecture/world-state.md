# World State — The Relational Ground Truth

The world database is Layer 1. It is the persistent, world-spanning store that holds every fact the simulation depends on: who is alive, who serves whom, who owns what, who hates whom, where the trade routes run, what each settlement produces, what happened last month. It is the answer to "what is true about the world right now," and it is true even when the player is asleep.

This layer is **not the ECS** and is not built like one. The two stores have different shapes because they answer different questions.

---

## What Lives Here

The world database is the home of everything that persists across hydration boundaries and across save/load.

- **Characters.** Every named individual in the world. Their identity, traits, vital statistics, location, status, history.
- **Settlements.** Villages, towns, cities, holds. Their population, economy, infrastructure, controlling faction.
- **Factions.** Houses, guilds, cults, kingdoms. Their members, holdings, goals, internal politics.
- **Titles and claims.** Who holds what title, who has a claim on it, what the claim's basis is.
- **Relationships.** Marriages, vassalages, friendships, rivalries, parent-child, employer-employee, opinions. These are first-class — not fields on a character, but entries in relation tables that can be queried in either direction.
- **Geography.** Territories, regions, biomes. Spatial indexing so "everyone in this region" is cheap.
- **History.** A log of significant events. Wars declared, deaths, inheritances, revolts. Past events are queryable conditions for Chronicle rules.
- **Economy.** Production, consumption, prices per settlement, trade flows.

Anything the player can ask "what was that NPC's name again, and where do they live?" must have its answer here.

---

## Why It Is Not the ECS

The two stores serve different evaluation patterns, and trying to unify them produces something that is bad at both.

**The ECS is iterated; the world database is queried.** ECS systems sweep over components: "for every entity with Position and Velocity, integrate." World simulation rules ask: "find every character who is a ruler whose primary territory has unrest above 0.7 whose liege's culture differs from theirs." That second pattern is a relational join across multiple tables with indexed filters. Expressed against an ECS, it is a slow scan; expressed against a relational store with appropriate indexes, it is what databases were designed to do.

**The ECS is local; the world database is global.** The ECS holds what is hydrated near the player — a few thousand entities. The world database holds the entire world — tens of thousands of characters, every settlement, every faction. Loading all of that into the ECS would defeat the purpose of streaming.

**The ECS is fast; the world database is slow.** ECS systems run at 60 Hz on the main thread. The world database is read at any rate but written largely by the world thread on game-day or game-month ticks. The two have different concurrency rules and different performance budgets.

**Relationships are first-class here, not in the ECS.** "Who is married to Aldric" is a question Layer 1 must answer instantly in either direction. In an ECS, that requires either embedding the relation as a component on every party (with consistency burden) or scanning every entity. In a relational store, it is an indexed lookup.

---

## How Other Layers See It

The world database does not belong to the ECS; it belongs to the engine. Multiple layers read and write it, each in a different way.

- **Layer 2 (World Simulation)** is the primary author. Chronicle rules query the database, fire on matched conditions, and apply structured effects that mutate it. The background-simulation tick (described below) also writes here. The world thread is where most writes happen.
- **Layer 3 (Agent Behavior)** reads it heavily and writes occasionally. An agent's blackboard is populated from world knowledge — faction standing, opinions, recent events involving them. When an agent makes a persistent decision (changes job, leaves a faction), it writes back to the database.
- **Layer 4 (ECS)** reads through the hydration bridge when entities spawn and writes back through the bridge when entities despawn. ECS code does not touch the world database directly during a frame — that responsibility is the bridge's.
- **The Player** is, fundamentally, just another agent whose actions become events the database records and Chronicle rules query.

---

## The Background-Simulation Tick

Layer 2 has two distinct mechanisms acting on the world database at different time scales. **Chronicle** evaluates rules on game-day and game-month ticks; that is `chronicle.md`'s subject. The other mechanism is the **background-simulation tick**, and it is what advances routine NPC life while no agent is running.

### What it is for

When an NPC is dehydrated, the agent layer is silent on them — no blackboard, no goal scoring, no HTN plan. But the world is not paused around them. Aldric, while the player is two regions away, still works at his forge and produces iron tools; the village's economic stocks change; the inn down the road serves drinks; travellers move along trade routes. None of this is Chronicle's concern (these are not events with consequence — they are continuous low-level processes), and none of it is Glyph's concern (there is no ECS entity to run Glyph against). It is the background simulation's concern.

The background tick advances dehydrated characters according to their **authored schedule and current job**, both of which are data fields on their Layer 1 record. Aldric's record says: blacksmith; works mornings at the village forge; eats midday; drinks at the inn evenings; sleeps at home. The background tick reads this on each game-hour or game-day step and updates his location and his output accordingly.

### What it is not

- **Not Chronicle.** Chronicle's evaluation model is "find characters where conditions hold, fire weighted events." Routine work is not an event. It is a continuous process that should accumulate output, not produce a discrete fired effect.
- **Not Glyph.** Glyph requires an agent to run on. Dehydrated characters have no agent.
- **Not a fine-grained simulation.** It does not know what Aldric is doing this minute; it knows what he is doing this hour. It does not know whether his hammer struck cleanly; it knows he produced N tools this day. The resolution is deliberately coarse — the player will never see this state directly, only its consequences when they next hydrate the character.

### What it produces

Background-tick output is always Layer 1 mutations:

- **Location.** Updated to the location dictated by the schedule for the current game-time-of-day.
- **Inventory and stock deltas.** Tools produced, food consumed, coin earned or spent, raw materials drawn down.
- **Settlement-level aggregates.** Village production, consumption, trade-route flow.
- **Status drifts.** Slow changes to needs (rest, food) consistent with the activity, tracked at low resolution. The point is not to perfectly simulate hunger; it is to ensure that when the player returns, the character's needs are in a plausible state for their schedule.

The tick is **coarse and best-effort**. It does not simulate failures, distractions, or surprises. If a fire breaks out in Aldric's village while he is dehydrated, the background tick does not have him flee — there is no fire at this resolution. If Chronicle's "village burns" rule fires, the rule's effect is what handles consequences (Aldric's record may be flagged dead, displaced, injured). The background tick handles routine; Chronicle handles events.

### How it interacts with Chronicle

The two mechanisms run on the same world thread, on independent ticks, both writing to Layer 1.

- The background tick is **frequent and shallow**: every game-hour or game-day, advance every dehydrated character a small step. Cost is bounded by the dehydrated population and by the simplicity of the per-character update.
- Chronicle is **less frequent and richer**: every game-day and every game-month, evaluate the rule registry against the database. Cost is bounded by the rule registry size and by the indexed query plans rules compile to.
- They do not race because they run on the same thread sequentially. Within a game-day step the background tick runs first, then Chronicle's daily rules evaluate against the resulting state.

The two mechanisms together are what makes the world feel alive *between* the player's visits. The background tick makes Aldric's day pass; Chronicle decides what life event finds him while it does.

### Granularity is a tuning knob

The background tick's resolution (game-hour vs game-day) and the per-character update cost determine how many dehydrated characters the simulation can sustain. Game 4's target is "thousands tracked at the dehydrated tier"; reaching that target may require coarsening the tick to game-day for non-flagged characters and game-hour only for narrative-flagged ones. The hybrid LOD approach (`overview.md`) extends here: flagged characters get richer background simulation, background population gets cheaper.

---

## The Shape of Queries

Chronicle is the language designed against this store, and the queries it compiles to are of one kind: **set-returning, indexed, joinable**. A trigger names a starting set ("rulers"), filters it ("rank at most Baron"), joins to related tables ("their primary territory," "their liege's faction"), and filters again ("territory's unrest above 0.7"). The database must make these cheap, which means it must maintain the right indexes from the start.

The discipline this imposes on the schema: every relationship that Chronicle rules will query must have a relation table with bidirectional indexes, not be a field embedded in a character record. Vassalage is its own table, not `character.liege_id`. Marriage is its own table. Faction membership is its own table. The database is structured for the query patterns Chronicle will run, not for an ad-hoc collection of fields.

---

## Persistence and Saves

The world database is what a save file is. Everything else — the ECS, the agent layer's transient state — is reconstructed from it on load. Save and load is dump-and-restore of the database plus a flag for the player's location, which determines the initial hydration.

This makes save files small and reload deterministic. It also makes the world database the only place we have to be careful about backward compatibility as the game evolves.

---

## When This Layer Appears

The world database does not exist in Games 0–3. The local simulation, the AI, the survival systems all run without it. Layer 1 lands in Game 4, alongside Chronicle and the hydration bridge, because those three are co-dependent and there is no value in shipping any one of them alone.

This is a deliberate scope decision. The engine subsystems built before Game 4 are designed to **not foreclose** the world database — components that will need to write back to it carry stable identities; the asset pipeline supports the formats it will need; the agent layer's blackboard is designed to take both ECS-perception and world-knowledge inputs. But none of the persistence machinery exists until Game 4 forces it into being.
