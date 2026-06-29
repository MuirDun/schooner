# Input — Raw Polling and Named Actions

How the player's hardware reaches gameplay. This is the vision, not the
implementation; concrete shapes live next to the code. Input belongs to Layer 4
(the local, here-and-now simulation on the main thread at 60 Hz) and is part of
the supporting cast that the layer consumes each frame.

Input is two layers stacked: a **raw device snapshot** that records what the
hardware did this frame, and a **named-action map** that translates physical
triggers into the verbs gameplay actually speaks. Both are **polled, never
subscribed** — the same nervous-system principle the event backbone follows. A
system that wants to know whether the player jumped reads a value; it does not
register a callback. This is what lets a future script VM be one more polling
reader over the same state rather than a parallel input path.

---

## Layer 1 — The Raw Device Snapshot

The ground truth of the hardware: which keys and mouse buttons are held, where
the cursor is, how far the mouse moved, how far the wheel turned, whether the
cursor is captured. The engine records into this snapshot from the windowing
system; gameplay only ever reads it. It is a one-way sink — systems cannot
synthesize fake input by writing to it, which keeps the snapshot an honest
record of the physical world.

Two kinds of value live here, and the distinction is load-bearing:

- **Persistent state** — what is *currently* true. A held key, the last cursor
  position, whether the cursor is grabbed. This survives across frames until the
  hardware changes it.
- **Frame-scoped signals** — what *happened* this frame. The press and release
  edges, the accumulated mouse motion, the accumulated wheel movement. These are
  cleared once per frame, at a single defined point, so each frame starts fresh.

The clearing happens exactly once per frame, after all gameplay has run. That
single clear point is the seed of the sharpest discipline in the whole system
(see *The Fixed-Step Discipline* below).

This layer is deliberately device-shaped, not gameplay-shaped. It knows about
keys and buttons; it knows nothing about "jump" or "grab." Keeping it ignorant
of intent is what lets the action layer above it — and rebinding, and scripting —
exist at all.

---

## Layer 2 — Named Actions

Gameplay should not speak in hardware. A verb like "grab," "throw," or
"mode-telekinesis" is a stable concept; the key or button that triggers it is a
detail that changes with rebinding, with the device, and with who is authoring
the binding. The action layer is the translation: each named action is bound to
one or more **physical triggers** (a key, a mouse button, a wheel direction),
and the action is active when *any* of its triggers is — a logical OR over the
ways to invoke the same verb.

Three commitments shape this layer.

### Action identity is a name, not a closed enum

The obvious Rust move — an `enum Action { Jump, Grab, … }` — is the wrong one,
and it is wrong for a single decisive reason: **the script layer registers
actions too.** Glyph is Lisp-flavoured; symbols are its native currency. When a
Glyph script declares an action by name, that name must land in the *same*
identity space the engine's Rust-authored actions live in, or the boundary
becomes a translation table that has to be reconciled on every call. A closed
enum cannot be extended by content; a name can.

So an action is identified by an **interned symbol** — a name turned into a cheap,
comparable handle through a single shared namespace for the whole process.
Interning the name `"jump"` yields the same handle no matter who asks: engine
setup code, a rebinding routine, or a script loaded at runtime. The namespace is
shared rather than per-world precisely because a name is a global fact, not a
per-world index — it points at meaning, not at storage. This is the seam the
future language binding hangs off: the Rust-now path (`name → symbol → bind`) is
*identical* to the Glyph-later path. Scripting integration becomes a translation
of an existing pipeline, not a rewrite of it. (Pillar 4: strict skeleton —
symbols are stable identities — with fluid structure on top — what binds to what,
and who authors it, is a runtime question.)

### The action state is derived once per frame, not subscribed

Each frame, before any gameplay reads it, the action layer is recomputed from the
raw snapshot: for every action, is any of its triggers active right now? That
single resolve point is what every consumer reads. Two consequences fall out of
doing it in one place:

- **Edges belong to the action, not its triggers.** Whether an action *just*
  became active is decided by comparing the action's own state to last frame's —
  not by OR-ing the press-edges of its triggers. An action bound to two keys must
  not re-announce itself when you press the second key while the first is already
  held. The "just happened" signal is a property of the verb, computed from the
  verb's transition.
- **It stays polling.** The resolve is a derivation, not a dispatch. Nothing is
  notified; the next reader simply sees the fresh value. This keeps the action
  layer on the same poll-never-subscribe substrate as Layer 1 and the event
  backbone, which is what keeps a script VM "one more reader."

### Axes and chords are read, not declared

The binding table stays a flat map of name to OR'd triggers. Richer shapes are
**derived by the consumer**, not added to the table:

- An **axis** (move forward/back, strafe left/right) is two actions, differenced.
  "How far forward" is "forward pressed" minus "back pressed."
- A **chord** (an action that needs two inputs held at once, like a two-handed
  grip) is an AND in the one system that needs it — "push held *and* pull held."

Keeping expressiveness in the reader rather than the binding model is the
discipline that keeps the table small. The moment a chord or sequence primitive
earns its place — many chords, or content that must rebind them — the model can
grow; until then, it does not.

---

## The Fixed-Step Discipline

This is the part that is easy to get wrong and expensive to debug, so it is a
written rule, not a convention discovered later.

The local simulation runs two clocks. **Variable-rate** work (input, camera,
most gameplay) runs once per frame. **Fixed-rate** work (physics, deterministic
gameplay) runs a *variable number of times* per frame — zero times on a fast
frame, several times on a slow one — so the simulation steps at a constant rate
regardless of frame rate. Input's frame-scoped signals are cleared once per
frame. Those two facts collide:

- A fixed-step system that reads an edge **double-fires** on a slow frame that
  runs several fixed steps — the edge is still true on every step.
- It **misses** the edge entirely on a fast frame that runs zero fixed steps —
  the edge is born and cleared inside a frame the fixed clock never entered.
- And it reads a **stale** edge anyway, because the resolve runs *after* the
  frame's fixed steps have already happened.

This is the same tick-stride mismatch the change-detection cursor faces, and the
resolution is the same in spirit: keep the frame-scoped reads on the variable
clock, and hand the fixed clock durable state.

- **Edges and the wheel are read only on the variable clock.** The moment a press
  becomes a press, or the wheel turns, is observed once per frame.
- **Levels are safe on the fixed clock.** "Is forward held," "which mode is
  active" — these derive from persistent state that does not change across a
  frame's steps, so reading them in fixed work is idempotent. Continuous forces
  (walk, hold) read levels directly.
- **One-shots are latched.** An edge that must cause exactly one fixed-step action
  (a jump, a throw) is captured on the variable clock into a small intent, and the
  fixed clock consumes that intent once. A press on a zero-step frame waits in the
  latch until a step runs (no miss); a press on a many-step frame is consumed by
  the first step (no double-fire). The cost is a deterministic one-frame latency,
  the same shape as an event being readable the frame after it is sent.

Mode selection is the canonical example: the variable clock turns a key edge into
a persistent mode; the fixed clock reads the mode as a level and gates its
per-mode systems on it.

---

## What Input Is Not (Yet)

The target game (pillar 2) decides scope. Named so the boundary is explicit:

- **No rebinding persistence.** Bindings are data and can be changed at runtime,
  but there is no on-disk profile or config format until a game needs one.
- **No gamepad, no text input / IME.** Keyboard and mouse only; text entry is a
  UI concern a later game owns.
- **No chord, sequence, or hold-timing primitives**, no input contexts or layered
  action sets, no analog response curves. Each of these waits for a consumer that
  forces it, rather than being built on speculation.

Refusing this generality is not a gap; it is how the system stays small enough to
hold in one head.

---

## Cross-references

- `overview/input.md` — the as-built state and the current game's roadmap (the
  detail that tracks the code, where this doc tracks the idea).
- `ecs.md`, `reactivity.md` — the change-detection ledger whose tick-stride
  problem the fixed-step discipline mirrors.
- `events.md` (overview) — the poll-never-subscribe event backbone the action
  layer shares its philosophy with.
- `language-binding.md` — why action identity is an interned symbol: the shared
  namespace that makes script-side action registration a translation, not a
  rewrite.
- `physics.md` (overview) — the fixed-clock consumer that the fixed-step
  discipline exists to protect.
