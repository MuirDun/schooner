# Input — raw polling (Layer 1) + the action layer (Layer 2)

Idea doc: `architecture/input.md` (the two-layer design; Layer 2 deferred). This
is the build state and Kinesis roadmap.

---

## What exists now (Layer 1, solid)

- Polling-only raw state in the `Input` resource: keyboard down/just-pressed/
  just-released, mouse buttons (same), cursor position, motion delta, cursor
  grab/visible (`input.rs`). Well-tested.
- `App::window_event` translates winit events into `Input`; egui consumption is
  gated so overlay typing doesn't leak into gameplay; `Input::end_frame` rolls
  edges and clears per-frame deltas once per frame (`app.rs`).
- `record_*` methods are crate-private — systems can't synthesize input.
- **Mouse wheel** (Part 2.B.1): a per-frame vertical accumulator (`mouse_wheel`),
  the twin of the motion delta — normalized from winit `LineDelta` / `PixelDelta`
  at the App boundary, cleared by `end_frame`.

## What exists now (Layer 2, Part 2.B)

- **Interned-symbol action IDs** (`symbol.rs`): `sym(name) -> Symbol`, a
  process-global interner — the shared namespace a future Glyph script registers
  into, so script-side action registration is a translation, not a rewrite (see
  `architecture/input.md`).
- **The action map** (`action.rs`): a `Bindings` table (action `Symbol` → OR'd
  `Trigger`s — key / mouse button / wheel sign) and an `Actions` state resource,
  resolved once per frame on `Update` by `resolve_actions` (registered
  engine-intrinsic, ahead of game systems). Reader surface mirrors `Input`:
  `pressed` / `just_pressed` / `just_released` / `axis(neg, pos)` / `wheel`.
- Action edges derive from each action's own aggregate down-transition, **not**
  from OR-ing its triggers' edges — so a multi-bound action never double-announces.

## What's missing

- **Migrating existing consumers onto the action map.** `fps_move` / `fps_look`
  still read `Input` directly with hard-coded keys; they move onto `Actions` when
  the Rapier KCC replaces `fps_move` (Part 2.F).
- **Out-of-Kinesis Layer-2 generality** — rebind persistence / file format,
  gamepad, IME, chords-as-primitive, input contexts. Deferred until a game forces
  them.

## Kinesis roadmap

- **Part 2 (Verbs):**
  - Add **mouse-wheel** delta to Layer 1.
  - Add a **minimal action layer**: mode select (1/2/3), the grab/push/pull/throw/
    repulse verbs, bound through named actions. Keep IDs as interned symbols (not
    a Rust enum) so the future Glyph action registration is a translation, not a
    rewrite — this is the one forward-looking constraint worth honoring now
    (`architecture/input.md` §"Open questions" names action-ID type as the
    decision driven hardest by scripting integration).
  - Resolve the **FixedUpdate input semantics** (see risk).
- **Part 4+:** no input work expected.
- **Out of Kinesis scope:** rebinding persistence / file format, gamepad,
  text-input/IME (Game 2A's UI), full Layer-2 generality.

## The fixed-step discipline (current implementation; correction staged)

`Input::end_frame` clears the edges and the wheel exactly once per frame, *after*
the whole `run_fixed × N → Update → Render` sequence — so a **FixedUpdate** system
that reads an edge **double-fires** on a multi-step frame, **misses** on a
zero-step frame, and reads a **stale** edge anyway (the resolve runs on Update,
after the frame's fixed steps).

The convention, decided in Part 2.B and now load-bearing for every verb:

- **Edges (`just_pressed` / `just_released`) and the wheel are `Update`-only.**
  Both `Input`'s and `Actions`' frame-scoped reads happen on the variable clock.
- **Levels are `FixedUpdate`-safe.** `pressed`, `axis`, and mode/state resources
  derive from `Input` state that is unchanged across a frame's steps, so reading
  them per fixed step is idempotent (correct continuous integration).
- **One-shots are latched.** An edge that must cause exactly one fixed-step action
  is captured on `Update` into a durable intent and consumed once by the fixed
  clock — no miss on a zero-step frame, no double-fire on an N-step frame. The
  cost is a deterministic one-frame latency, the same shape as a one-frame-late
  event.

This remains the as-built behavior through 2.F.4, but controller review exposed
its mandatory stale-control frame: fixed simulation runs before action
resolution, look, movement capture, mode changes, and spectator handoff. Phase
2.FH.5 moves the single action/control sampling point before fixed simulation.
The durable-state discipline does not change: edges and wheel are still sampled
once per render frame, one-shots still latch across zero-step frames, and fixed
steps still consume state rather than raw frame edges. The first eligible fixed
step will then use the current snapshot without mandatory one-frame latency.

The intended contract lives in `architecture/input.md` §"The Fixed-Step
Discipline". The current `action.rs` module docs continue to describe the
as-built Update placement until 2.FH.5 changes the code.

Cross-refs: [events.md](events.md) (input edges drive state transitions),
[physics.md](physics.md) (the FixedUpdate consumer).
