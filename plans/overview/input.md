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

## What's missing

- **Layer 2 — the action map.** Named verbs ("grab", "throw", "mode-repulsion")
  OR'd from one-or-many physical triggers, recomputed once per frame from Layer 1.
  `architecture/input.md` defers it until a consumer forces it. **Kinesis forces a
  light version in Part 2** (modes, verbs, mouse-wheel distance) — but it can stay
  minimal (a small enum-keyed binding table), not the full rebind/file-format
  build.
- **Mouse wheel** — not recorded yet; Part 2 needs it for telekinesis distance.
- **A FixedUpdate-safe read path** — edges are frame-scoped; see hazard below.

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

## Sharp risk: edges vs the fixed step

`Input::end_frame` clears `just_pressed`/`just_released` and mouse delta exactly
once per frame, *after* the whole `run_fixed × N → Update → Render` sequence. So a
system in **FixedUpdate** that reads an edge will:
- **double-fire** when a frame runs multiple fixed steps (edge true in each), and
- **miss** the edge entirely on frames that run zero fixed steps (edge born and
  cleared within a frame that ran no fixed step).

Latent today (every input consumer runs on Update). It bites the moment a
physics/gameplay verb reads input in FixedUpdate. Standard fixes: keep edge reads
on Update and have FixedUpdate read *state* (`ControlMode`, etc.); or maintain a
separate FixedUpdate input snapshot cleared by the fixed schedule. Decide in
Part 2 when player physics lands. (Mode select stays on Update — see
[events.md](events.md) example 1.)

Cross-refs: [events.md](events.md) (input edges drive state transitions),
[physics.md](physics.md) (the FixedUpdate consumer).
