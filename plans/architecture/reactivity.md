# Reactivity — The Engine's Nervous System

The engine is built on reactive cascades. A spell hits an NPC; a `Burning` component is added; particles spawn, the agent layer perceives the threat, the world database records a fact, a faction may eventually retaliate. None of these are scripted as a chain by hand. Each is the consequence of one layer publishing a change and another reacting to it. This document is about what that reactivity actually *is*: how it propagates, what guarantees it makes, what it deliberately does not promise.

---

## How Reactivity Serves the Pillars

**The world is alive.** A world that responds to itself is the definition of pillar 1. Reactivity is the substrate that makes the response composable — fire ignites flammables not because every fire-source knows about every flammable, but because both subscribe to the same change events.

**Built for one kind of game.** The reactivity model is shaped for an immersive-sim with three time scales (frame, agent tick, game day). It is not a general-purpose dataflow engine. It does not support arbitrary topology, arbitrary cycle resolution, or arbitrary update orderings. It supports the patterns this game needs.

**Developer ergonomics is a feature.** Subscriptions are first-class language idioms in Glyph. Authors write "when X changes, do Y" and the engine handles the rest. The substrate must be clear enough that an author can predict what their subscription does.

**Organism, not castle.** Reactive subscriptions are exactly the "fluid shape on top of strict skeleton" pattern from pillar 4. Component types are typed (skeleton); which subscriptions are active and what they do is runtime-determined and changes under hot reload (shape).

---

## The Three Tiers, Restated

Reactivity is not one mechanism. It is three, each tuned for a different time scale and a different layer.

### Tier 1 — In-Frame Component Reactivity (Layer 4)

The fastest tier. Within a single frame, in the local simulation, when a component is added, removed, or mutated, systems can react in the same frame.

**Granularity:** per-entity, per-component-type changes.
**Latency:** same frame as the change, with bounded propagation depth.
**Consumer:** ECS systems and Glyph subscriptions.
**Substrate:** per-component change ticks already built into the sparse-set storage in Game 0.

This tier carries: status-effect propagation, particle spawning on component change, sound triggers, animation transitions, immediate gameplay responses.

### Tier 2 — Cross-Layer Typed Queues

The middle tier. When something happens in one layer that another layer should hear about, it is published as a typed event. Producers publish at their own rate; consumers read on their own tick.

**Granularity:** typed event records, named per channel.
**Latency:** producer publishes; consumer reads on its next tick (frame, agent tick, or game-day tick).
**Consumer:** the layer the event is addressed to.
**Substrate:** typed channels; one queue per producer-consumer-event-type triple.

This tier carries: collision events from physics into Glyph, AI commands from the agent layer to the ECS, dialogue triggers, "your faction declared war" notifications from the world simulation to active agents, "the player burnt down a village" from the ECS into the world database.

### Tier 3 — World Event Accumulation (Layer 1)

The slowest tier. Significant facts about the world accumulate in the world database. Future Chronicle rules query these accumulated facts as conditions.

**Granularity:** structured world facts with timestamps and scope (which character, which territory, which faction).
**Latency:** measured in game-days or game-months. Tier 3 events are *queried*, not *consumed*.
**Consumer:** Chronicle rules during their world-tick evaluation.
**Substrate:** an indexed event log within Layer 1, queryable by Chronicle's relational operators.

This tier carries: historical context for rule evaluation. "Three NPCs have died in this territory this month" is a Tier 3 query. "The player has betrayed two factions in the last year" is a Tier 3 query.

The three tiers are different kinds of system. Conflating them produces architecture mistakes: making Tier 3 push-based produces a stampede on every world fact; making Tier 1 pull-based makes status-effect propagation unworkably awkward.

---

## Push or Pull?

The honest answer: **mixed, by tier, and on purpose.**

### Tier 1: Push, with Pull-Style Iteration as an Escape Hatch

The natural shape of "when component X is added, do Y" is push: the storage knows the component changed; the storage informs subscribers. This is what Tier 1 is.

But the substrate is a **change-tick ledger**, not a notification list. A system can subscribe ("notify me on every Health change this frame") and the subscription drives push semantics. Or a system can poll ("which entities have Health that changed since the tick I last processed?") and the same ledger answers the pull-style query. Both shapes use the same underlying mechanism — the component's change-tick metadata — and both are first-class.

This matters because some systems are naturally push (status-effect propagation, particle triggers) and some are naturally pull (a renderer that wants the list of moved transforms once per frame, regardless of how many times each transform was touched). The substrate supports both without favouring one.

### Tier 2: Push (Producer Publishes; Consumer Reads on Its Schedule)

A typed queue is a push from the producer's perspective and a pull from the consumer's. The producer publishes when the event happens. The consumer reads when its own tick comes around. Between them is a buffered queue.

This is the right shape because the producer and consumer run at different rates. The ECS publishes at 60 Hz; the agent layer consumes at 10–30 Hz; the world simulation consumes on game-day ticks. A pure-push model would force the consumer to handle every event in real time. A pure-pull model would force the producer to keep history. Buffered queues let each side run on its own schedule and exchange events at the boundary.

### Tier 3: Pull, Always

Chronicle rules query the world database. They are not notified of new facts; they read whatever facts are in the database when their rule evaluates. Push semantics make no sense at this tier — a rule firing "as soon as a fact is recorded" is not what a world simulation is. Rules fire on **their own tick** (game-day, game-month) and *then* see what the database says.

---

## Dataflow or Imperative?

The engine is **imperative with reactive dispatch**, not dataflow.

A pure dataflow engine (Reflex, RxJS, MobX) describes the world as a graph of derived values, where any change automatically propagates to everything downstream. That model has real strengths — explicit topology, deterministic recomputation, automatic memoisation — and real costs: every value must be expressible as a function of inputs, side effects must be quarantined, cycles require special machinery.

The engine is not that, for several reasons:

- **Most "reactions" are side effects, not derivations.** Spawning a particle when `Burning` is added is a side effect on the ECS. Modelling it as a derived value of the entity's component set would require treating particles as a function of components, which they are not.
- **The world is mutable by design.** Components are mutated in place. Treating mutation as a forbidden side effect would invert the engine's basic model.
- **Performance.** Dataflow's automatic propagation tracks dependencies dynamically; the overhead is non-trivial at frame rate. The engine cannot afford it for every component access.

What the engine *does* take from dataflow:

- **Subscriptions are declarative.** "When this component changes, run this handler." Authors do not write the dispatch loop; the engine does.
- **Topology is observable.** At a given moment, the engine knows which subscriptions exist and which entities they cover. Debugging tools can render this graph.
- **Reads do not have side effects.** A system reading a component does not affect the change-tick ledger; only mutations do. This is a small but important property.

So: **reactive dispatch on top of imperative state.** Subscribers express interest declaratively; the engine notifies them imperatively; what they do in response is their business.

---

## Determinism

Determinism is a non-negotiable property at certain seams and an explicit non-goal at others. The engine is honest about which is which.

### Where Determinism Is Guaranteed

- **Single-frame ordering within a tier.** Inside a frame, Tier 1 propagations occur in a defined order — system order is fixed by the scheduler, subscriber order within a system is defined by registration order. Two runs with the same input produce the same intermediate state.
- **Save / load fidelity.** Loading a save and playing forward produces the same world that was running when the save was made, assuming the same player input.
- **Fixed-timestep simulation.** Physics and other fixed-timestep systems run at a defined rate, decoupled from frame rate, so simulation outcomes do not vary with framerate.
- **World simulation tick order.** Chronicle rules evaluate in a defined order each game-day tick. Two rules that fire on the same trigger resolve by deterministic priority and weighted random selection seeded from the world's RNG state.

### Where Determinism Is Not Guaranteed

- **Cross-thread ordering** between the main thread and the agent / world threads is **not** strictly deterministic. The scheduler enforces sync points, but the order in which the agent thread completes its batch versus when the main thread reads the command buffer can vary by a frame. This is acceptable because gameplay does not depend on sub-frame inter-thread ordering.
- **Parallel system execution** (when it eventually arrives) does not promise determinism across runs unless explicitly seeded. Systems that need determinism declare it; everything else may run in parallel and accept the consequences.
- **Random number sources.** RNG is per-system and per-context. The world simulation has its own RNG seeded by the save state; gameplay events may use a different RNG that does not preserve across saves. Authors choose the RNG appropriate to their domain.

### Why This Mix

Strict determinism everywhere would make threading harder than it has to be. Strict non-determinism would make multiplayer and replays impossible (both are explicit future possibilities). The line drawn is: **determinism at coarse boundaries (saves, world ticks, fixed-timestep simulation), best-effort within frames, explicit non-determinism for parallel work.**

---

## Cycle Management

Reactive cascades can cycle. Component A changes, fires a subscription that mutates component B, which fires a subscription that mutates component A, and so on. The engine must not freeze.

### Tier 1 Cycles: Bounded Recursion Depth

Within a frame, Tier 1 propagation has a **bounded recursion depth**. When a subscription's handler mutates a component and that mutation would fire further subscriptions, the cascade is allowed to a fixed depth (initially small — three or four). Beyond that depth, additional changes are deferred to the next frame rather than processed inside the current cascade.

This is a deliberate compromise:

- **Within the depth budget**, propagation is synchronous, in-frame, and visible to the player on the same frame as the originating change. Status-effect chains feel immediate.
- **Beyond the budget**, propagation is deferred — the changes still happen, but on the following frame. Cycles that would otherwise loop forever cap out at one frame's worth of work and are continued (or detected as runaway) next frame.

The depth budget is observable in the debug overlay. A cascade that hits the budget logs its chain so authors can see why.

### Tier 2 Cycles: Naturally Decoupled

Tier 2 cycles are not really cycles in the same sense, because each layer reads on its own tick. A Tier 2 event from the ECS to the agent layer that produces an agent command back to the ECS does not loop within a frame; the agent layer reads the event next agent tick, produces commands, the ECS applies them next frame. Each step is bounded, and frame rate naturally throttles the cycle.

### Tier 3 Cycles: Resolved by Tick Boundaries

Tier 3 facts are read by Chronicle rules during world ticks. A rule that records a fact that another rule's trigger would match does **not** fire that other rule within the same tick — the second rule's trigger queries the database at the *start* of its evaluation tick, and the new fact is not visible until the next tick. This is a feature, not a bug: it prevents same-tick cascades that would make the world simulation impossible to reason about.

### Cycle Detection

The engine does not statically detect cycles. It detects them at runtime through the depth budget (Tier 1) and through tick boundaries (Tiers 2 and 3). When a cycle hits the depth budget, the debug overlay reports it, and the author can decide whether to break the cycle or accept the deferral.

This is correct for the engine's domain. Static cycle detection requires knowing the full topology ahead of time, which contradicts the organism principle — subscriptions are added and removed at runtime through Glyph hot reload. Dynamic detection at the cost of bounded work per cycle is the right trade.

---

## Subscription Lifecycle

A subscription has a defined lifecycle that hot reload must respect.

- **Registered** when the script defining it is loaded. The engine records the subscription against the relevant component type or event channel.
- **Active** while the script module is loaded. The handler runs on every matching change.
- **Replaced** when the script is hot-reloaded. The new version of the handler takes over; in-flight invocations of the old version complete on the old code; new invocations use the new code.
- **Removed** when the script is unloaded or the subscription is explicitly cancelled. No more invocations.

State held by a subscription handler is the handler's problem. The engine does not migrate handler-local state across reloads. Long-lived state belongs in components or in the world database, both of which survive reload.

---

## What Reactivity Costs

Honest accounting:

- **Tier 1.** Each component mutation bumps a tick — a memory write, effectively free. Subscription dispatch is a hash lookup per change-tick scan per subscribing system. At Game 0–3 scales this is negligible. At Game 4 NPC counts the system-side dirty-iteration cost is visible in profiling, and that is what dense-view caches address (`architecture/ecs.md`).
- **Tier 2.** Per-event allocation in the queue, per-event dispatch on the consumer's tick. Channels are typed and pre-sized; allocation happens in batches. Cost is proportional to event volume, which is bounded by gameplay event rates (collisions, AI commands), not by frame rate.
- **Tier 3.** Per-fact insertion into the indexed event log. Reads are Chronicle queries, which are compiled to indexed plans. Cost is amortised across game-day ticks and is dominated by query plan execution, not by reactivity.

Reactivity is not free. It is bounded, and the bounds match the budgets each tier has.

---

## What Reactivity Is Not

- **Not a free lunch.** Authors must understand that subscriptions cost dispatch overhead and cycle budget. The substrate makes reactive code easy to write; it does not make it easy to write *poorly* without consequences.
- **Not a replacement for explicit control flow.** When a system needs to do A then B then C, it does A then B then C. It does not split into three subscriptions chained through component mutations. Reactivity is for *cross-cutting* relationships, not for sequencing.
- **Not transparent.** A subscription that fires another subscription is a chain the author should be able to see. The debug overlay surfaces active subscriptions and their recent firings. Reactivity is opt-in, named, and inspectable.
- **Not for everything.** Many engine subsystems run as plain systems on the schedule, with no subscriptions. The renderer iterates transforms; it does not subscribe to transform changes. The physics step integrates rigid bodies; it does not subscribe to anything. Reactivity is the right tool for cross-domain reaction; it is not the only tool.

The engine is reactive *where reactivity earns its keep* and imperative everywhere else. The tiers exist to make that distinction concrete: Tier 1 for in-frame cross-system reaction, Tier 2 for cross-layer communication, Tier 3 for accumulated history. Use the right tier; do not try to make one cover everything.
