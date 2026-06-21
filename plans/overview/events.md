# Events & Reactivity — How game logic reacts

This is the doc the three motivating questions live in:

- *Press `3` → enter repulsion mode.*
- *A cube falls on my head from 10 m → my health drops.*
- *I step on food → my hunger is filled.*

All three are "when X, do Y." The mistake is to reach for one mechanism for all
of them. There are **two**, and choosing correctly is the whole skill.

---

## Status (2026-06-19)

- **Built:** the *mutation* half of Tier-1 change detection — `SparseSet` carries
  `ChangeTicks { last_mutation_tick }`, `Mut<T>` bumps it on `DerefMut`, and
  `World::changed_since::<T>(tick)` returns the entities whose `T` was mutated.
  No consumers yet. (`ecs/sparse_set.rs`, `ecs/world.rs`.)
- **Not built (Part 2 work):** add-detection (`Added<T>`), remove-detection
  (`Removed<T>` / despawn signal), a composable `Changed<T>` query *filter*, a
  discrete-event primitive (`Events<T>`), and a deferred-op buffer (`Commands`).
  See [ecs.md](ecs.md) and [physics.md](physics.md).

The audit's one-line version: *the storage ledger exists; the consumer API is the
Part-2 build.*

---

## The one rule: poll, never subscribe

Game logic **never registers a callback**. No `on_collision(|a, b| …)`, no
`health.on_change(…)`. A reaction is always a **system** that declares what it
reads and writes, and the schedule runs it. The engine already committed to this
for input and camera (`architecture/input.md` §"Why polling, not listeners") and
the reasons carry verbatim: a callback is invisible to the scheduler and the
alias checker, and it fires from the wrong place to mutate the world safely.

This rule is also the answer to the Rust-now / Glyph-later dilemma. Because every
reaction is a polling system over a typed buffer or a change-tick, a future Glyph
VM is **one more polling system** that drains the same buffers and dispatches to
script handlers. The substrate is language-agnostic; only the *consumer surface*
differs. So Rust game logic written this way is not throwaway — Glyph becomes a
parallel consumer, never a replacement. The thing that *would* be a Glyph-hostile
trap is the convenient-looking callback API, which is exactly why we don't build it.

> It is fine for the Rust to be a little verbose (explicit systems, explicit
> queries). The *shape* — poll a buffer or a change-set, then mutate state — is
> the Glyph shape too. Verbosity ports; cleverness doesn't.

---

## The two mechanisms

### 1. State + change-detection (the declarative spine)

Most "when X, do Y" is really "**Y is a function of state X.**" Model it that way:

- World facts are **state** — components and resources (`Health`, `Hunger`,
  `ControlMode`, `ResearcherAttitude`).
- **Derived signals** are systems that *read* state and compute an output
  (reticle colour from mode, food glow from hunger, eye-state from attitude).
  They do not get "notified"; they recompute from the source of truth.
- **Reactions to change** use the change-tick ledger: `changed_since` /
  `Changed<T>` / `Added<T>` / `Removed<T>`. A reaction runs only for the entities
  whose state actually moved this frame.

This is `crates/game/development.md`'s "one place where state moves; everything
else derives" rule, and it is the shape that ports to Chronicle in Game 4 as a
translation. Use it for anything that is a *fact that persists*.

### 2. Discrete events (`Events<T>` — the Tier-2 channel)

Some things are not state — they are **instants with a payload**: a collision
happened, a trigger was entered, a key edge fired. You can't model "a contact
occurred at 3.2 kN" as "a value changed." These go in a **double-buffered
`Events<T>` queue**: producers `send`, readers drain by polling, the buffer swaps
once per frame so a one-frame-late reader still sees the event. This is the
"first Tier-2 cross-layer channel" Part 2 builds (`crates/game/implementation/part2-verbs.md`).

### The decision rule

> **Is it a fact that persists, or an instant that occurs?**
> Persists → state + change-detection. Occurs → `Events<T>`.

`Health` persists; "took 12 damage from this contact" occurs. `Hunger` persists;
"entered the food trigger" occurs. `ControlMode` persists; "pressed 3" is an
input edge (already represented in the `Input` snapshot).

---

## The three examples, worked

### 1 — Press `3` → repulsion mode  *(input edge → state transition; no queue)*

This is **not** an `Events<T>` case. The `Input` resource already represents edges
(`just_pressed`), and the mode is *state*. One system polls the edge and writes
the mode; everything downstream *derives* from the mode.

```rust
#[derive(Clone, Copy, PartialEq)]
enum ControlMode { Hands, Telekinesis, Repulsion }   // state

// Update stage. Reads input edges, writes mode. The ONE place mode changes.
fn mode_select(input: Res<Input>, mut mode: ResMut<ControlMode>) {
    if input.just_pressed(KeyCode::Digit1) { *mode = ControlMode::Hands; }
    if input.just_pressed(KeyCode::Digit2) { *mode = ControlMode::Telekinesis; }
    if input.just_pressed(KeyCode::Digit3) { *mode = ControlMode::Repulsion; }
}

// Everything else DERIVES — reads state, never subscribes.
fn reticle_tint(mode: Res<ControlMode>, mut reticle: ResMut<Reticle>) {
    reticle.color = match *mode {
        ControlMode::Hands       => WHITE,
        ControlMode::Telekinesis => TOPAZ,
        ControlMode::Repulsion   => RED,
    };
}
```

Lessons:
- **Not everything is an event.** Modes are state; input edges drive transitions;
  the reticle/HUD/active-verb all derive.
- The verbs run only in their mode. Today that's an early-return
  (`if *mode != Telekinesis { return; }`). The nicer form is a **run-condition**
  (`fn run_if(in_mode(Telekinesis))`), a small Part-2 schedule add — see
  [ecs.md](ecs.md). Glyph-future: `(when (mode? 'telekinesis) …)`.
- **Keep `mode_select` on Update, not FixedUpdate.** It reads input *edges*, which
  are frame-scoped and cleared once per frame — reading them from FixedUpdate
  double-fires or misses (see [input.md](input.md)). Physics verbs read the
  `ControlMode` *state* from FixedUpdate, which is always safe.

### 2 — Cube on head from 10 m → damage  *(physics contact → state, via a discrete event)*

Rapier emits contact events when it steps. The physics bridge (one exclusive
FixedUpdate system) drains them into `Events<Contact>`. A gameplay system reads
the queue and mutates `Health`. Death is then a *derived* reaction on the state
change — not coded inside the damage system.

```rust
struct Contact { a: EntityId, b: EntityId, impulse: f32, normal: Vec3 } // engine event

// Physics bridge (FixedUpdate), AFTER stepping Rapier:
//   for c in rapier.drain_collision_events() { events.send(Contact { .. }); }

// Gameplay reader (FixedUpdate, ordered AFTER the bridge).
fn fall_damage(contacts: Res<Events<Contact>>, mut q: Query<&mut Health>) {
    for c in contacts.iter() {
        if c.impulse < DAMAGE_THRESHOLD { continue; }   // a pebble shouldn't hurt
        let dmg = damage_from_impulse(c.impulse);
        if let Some(mut h) = q.get_mut(c.b) { h.current -= dmg; } // bumps Health's tick
        if let Some(mut h) = q.get_mut(c.a) { h.current -= dmg; }
    }
}

// Death DERIVES from the Health change — one owner, runs only on changed entities.
fn death_check(mut commands: Commands, q: Query<(EntityId, &Health), Changed<Health>>) {
    for (e, h) in &q { if h.current <= 0.0 { commands.insert(e, Dying::default()); } }
}
```

Lessons:
- The *contact* is an event (payload = `impulse`); `Health` is state; damage is
  "the one place health moves down"; death is a derived reaction. This is where
  the Part-2 `Changed<T>` filter earns its keep — `death_check` scans only
  entities whose health actually moved, not every entity with `Health`.
- **Use impulse, not velocity.** Rapier's contact impulse already integrates
  mass × Δvelocity — a heavy cube from 10 m lands a far bigger impulse than a
  pebble, with no special-casing. (Standard "contact-force event" pattern.)
- **Order, don't rely on the buffer here.** Register the bridge before the damage
  reader so damage lands in the *same* fixed step. The double-buffer guarantees no
  reader *misses* an event across a frame; it does not give you same-step ordering
  — that's the schedule's job.

### 3 — Step on food → fill hunger  *(trigger overlap → state, via a discrete event + despawn)*

A Rapier **sensor** collider reports overlaps without a physical response. The
bridge drains sensor intersections into `Events<TriggerEnter>`. A reader checks
the other entity is `Food`, raises `Hunger`, and despawns the food via `Commands`
(so the reader isn't forced to be an exclusive `fn(&mut World)`).

```rust
struct TriggerEnter { sensor: EntityId, other: EntityId } // engine event

fn eat_food(
    enters: Res<Events<TriggerEnter>>,
    food: Query<&Food>,
    mut hunger: ResMut<Hunger>,
    mut commands: Commands,
) {
    for ev in enters.iter() {
        if let Some(f) = food.get(ev.other) {   // did we step on food?
            hunger.satiate(f.nutrition);        // the one place hunger moves up
            commands.despawn(ev.other);         // deferred; applied after the system
        }
    }
}

// Hunger DECAY and food APPEARANCE are separate state systems (Part 4):
fn hunger_decay(time: Res<Time>, mut h: ResMut<Hunger>, p: Res<HungerPressure>) {
    h.current = (h.current - p.rate * time.fixed_delta).max(0.0);
}
fn food_appearance(h: Res<Hunger>, mut q: Query<(&Food, &mut Material)>) {
    for (_f, mut mat) in &mut q { mat.emissive = scent_glow(h.current); } // derived
}
```

Lessons:
- Trigger overlap is the event; `Hunger` is state; eating is the one place hunger
  rises; decay is a timed state system; appearance is *derived from hunger* —
  literally `development.md`'s `food_appearance(attitude, hunger)` derived signal.
- **`Commands`** (deferred spawn/despawn) is the Part-2 piece that lets a normal
  (non-exclusive) reader despawn safely mid-iteration. See [ecs.md](ecs.md).

---

## What the engine must build for this (Part 2), and where

| Piece | What it is | Doc |
|-------|-----------|-----|
| `Events<T>` | Double-buffered discrete-event queue; `send` + drain; swapped once/frame | [ecs.md](ecs.md) |
| `Commands` | Deferred spawn/despawn/insert/remove applied at a sync point | [ecs.md](ecs.md) |
| `Added<T>` / `Removed<T>` | The missing halves of Tier-1 change detection | [ecs.md](ecs.md) |
| `Changed<T>` filter | Composable change-detection in a `Query` (today only `changed_since` scan exists) | [ecs.md](ecs.md) |
| Physics bridge → events | Drains Rapier contacts/sensors into `Events<Contact>` / `Events<TriggerEnter>` | [physics.md](physics.md) |
| run-conditions | `system.run_if(in_mode(..))` so verbs run only in their mode | [ecs.md](ecs.md) |

Hunger/attitude/death **rules** are Part 4 (declarative state); the **mechanisms**
above (events, triggers, change-detection, commands) are Part 2.

---

## Glyph-future alignment (why this is a translation, not a rewrite)

Each Rust reaction has a one-to-one Glyph shape over the *same* substrate:

| Rust (now) | Glyph (Game 2A+, same buffers) |
|------------|-------------------------------|
| `for c in contacts.iter() { … }` | `(on-event Contact (>= impulse threshold) (damage b …))` |
| `Query<.., Changed<Health>>` | `(on-change Health (when (<= current 0) (add Dying)))` |
| `mode_select` polling `Input` | `(on-action 'mode-repulsion (set-mode 'repulsion))` |

The Glyph VM is a single engine system that drains `Events<T>` and walks the
change-tick ledger, dispatching to registered script reactions. Nothing about the
Rust substrate changes when it arrives. That is the payoff of "poll, never
subscribe": **one substrate, two languages.**

See also: `architecture/reactivity.md` (the three-tier vision),
`plans/architecture/language-binding.md` (schema ownership: Rust owns
engine-intrinsic components; Glyph will own game-defined ones).
