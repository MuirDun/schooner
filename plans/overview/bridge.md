# Bridge — App, frame loop, time, resources, the "always running" seam

This is the wiring that holds the subsystems together: the winit↔ECS translation,
the frame loop, the resource standup, and (future) the external-tooling seam. No
single idea doc; it threads through `architecture/render.md`, `input.md`, and
`plans/architecture/overview.md` §Threading.

---

## What exists now

- **`App`** owns the window, the `World`, the `Schedule`, and the frame clock.
  Builder API (`add_system`, `insert_resource`, `with_*`, `world_mut`); winit
  `ApplicationHandler` drives it (`app.rs`).
- **The frame loop** (`App::tick`): puffin frame housekeeping → advance `Time` →
  `run_fixed × N` (accumulator) → `run` (Update) → `run_render` (Render) →
  `sync_cursor` → `Input::end_frame`. Clean and physics-ready.
- **`Time`** — fixed-step accumulator, `MAX_FIXED_STEPS_PER_FRAME` clamp,
  `fixed_delta`, `interpolation_alpha` (`time.rs`).
- **Resource standup in `App::resumed`** — once the device exists, builds and
  inserts render context, registries, pipelines, post-FX resources, overlay, then
  appends `render_frame` + `f5_reload` and drains `Startup`.
- **Window events** → `Input`; resize → render context; focus → cursor grab.

## The "always running" posture (pillar 1 & 4)

The game is **always live**; there is no edit-mode / play-mode split. Dev tooling
and (later) external tooling interact with the running world, not a paused scene.
This is why the engine has no editor viewport and why dev visualization renders
in-world ([graphics.md](graphics.md)). The bridge's job is to keep the loop
running and expose **safe seams** for things to reach into the live `World`.

## Kinesis roadmap

- **Part 2 (Verbs):**
  - Stand up **Rapier resources** in `resumed` (isolated block) and register the
    **physics bridge system** in FixedUpdate, after gameplay Transform writers
    ([physics.md](physics.md)).
  - Register the **`Events<T>` swap** and **`Commands` apply** at defined points in
    `tick` (events swapped once/frame; commands applied at a sync point after the
    systems that queue them). See [ecs.md](ecs.md), [events.md](events.md).
- **Housekeeping (any Part):** `resumed` is a ~120-line standup monolith with
  log-and-exit-only device-init failure. As physics/audio/save resources land, it
  wants a small "plugin"/setup seam (a list of setup closures) rather than more
  inline blocks. Low priority; do it when the monolith next hurts.

## The scp seam (future — not Kinesis)

External tooling (the on-hold `scp` sub-project, Morse now / Glyph-driven later)
will talk to the running game over the scp protocol. The engine side needs **one
thing it does not have yet**: a safe inter-frame mutation point. `tick` currently
has no injection slot for an out-of-process actor to mutate `World` between
frames.

The intended shape (design when scp resumes, **after** Part 2 ships `Events<T>` +
`Commands`): a command-queue resource that scp fills from its transport thread,
drained by a dedicated exclusive system at a defined point in `tick` (e.g. between
`end_frame` and the next `run`), reusing the same `Commands`/`Events` plumbing
gameplay uses. Because it reuses that substrate, scp is **not** a parallel
mutation path — it's another producer into the one the engine already has. Keep
that in mind when building `Commands` in Part 2: make it the kind of thing an
out-of-process actor could also enqueue into.

## Sharp risks / decisions

- **Device-init failure is log-and-exit** (winit owns the loop, can't surface
  `AppError`). Acceptable; revisit only if a recoverable failure mode appears.
- **Resource standup ordering is implicit** (manual sequence in `resumed`). Fine
  at current scale; the plugin seam above is the relief valve when it grows.

Cross-refs: [physics.md](physics.md), [events.md](events.md), [ecs.md](ecs.md),
`plans/architecture/overview.md` §Threading (layer-boundary thread splits, later).
