# Debugging — Overlay Host and Subsystem Plugins

Architecture: [`../architecture/debugging.md`](../architecture/debugging.md).
This document records the current implementation state and the Kinesis work that
will establish the typed plugin framework.

## What exists now

- `DebugState`, frame statistics, the puffin view, the egui UI builder, and asset
  reload live together in `debug.rs`.
- `App::new` installs debug state and raw-input handling unconditionally;
  `App::resumed` always creates the egui GPU resources.
- `render_frame` builds the only debug window inline and therefore owns the
  concrete diagnostics/profiler UI call.
- F12 and F5 still read raw `Input`, bypassing the named action layer.
- The earlier game-mood preset enums were removed from the engine, but their
  action-driven game-side replacements were not implemented.
- PCF ownership is incomplete: a stale type remains in `debug.rs`, a second type
  lives privately in the forward renderer, and the renderer hardcodes the default
  rather than reading a production resource.
- Engine GPU startup still seeds Kinesis-specific grade, vignette, and fog values.
- There is no app plugin contract or debug panel registry.

This means the old monolith has been reduced, but the planned ownership split is
not complete. Phase 2.C starts from this state; its previous 2.C.4 completion mark
did not describe code reality and has been corrected.

## Phase 2.C target

- A dedicated `dev-tools` build flag makes Rust debug tooling available. Enabled
  binaries conditionally install statically typed plugins; this is not a shared-
  library ABI.
- A small general `Plugin` composition surface lets plugins register resources,
  systems, bindings, and panels with an `App`.
- `DebugCorePlugin` owns only overlay lifecycle, visibility, namespaced toggle
  input, and the panel registry.
- Diagnostics/profiling, asset reload, renderer inspection, and Kinesis mood
  tuning live in owner-side plugins. A convenience group composes the usual
  engine plugins without coupling them to the core.
- Debug panels are registered dynamically and run sequentially against the live
  `World`; target resource access remains typed.
- Egui frame construction becomes a Render-stage debug-host system. The renderer
  only encodes prepared overlay output.
- Debug actions use Layer 2 bindings. Plugin absence, rather than a parallel input
  mechanism, gates the bindings.
- Production render resources are authoritative without debug tooling. PCF has one
  render-owned resource, and engine startup preserves values configured by the
  game.

## Ownership map

| Capability | Owner |
|---|---|
| Overlay host, visibility, panel registry | debug core |
| Frame statistics and puffin panel | diagnostics |
| F5 mesh/texture reload | asset system |
| PCF and future render views/counters | renderer |
| Kinesis fog/grade/vignette/bloom/overlay presets | active game |
| Collider/contact/force visualization | physics, Phase 2.E |
| Source positions, ranges, voices, mixer inspection | audio, when audio lands |

## Boundaries

Phase 2.C builds composition, overlay contribution, input, and current-tool
ownership. Phase 2.E builds the shared world-space line/gizmo primitive and the
first substantial consumer, `PhysicsDebugPlugin`. Shadow-map-only output,
rasterization inspection, and audio visualization extend their owner plugins when
their target systems require them; they do not expand the debug core.

Future Glyph and Chronicle debugging remains a separate in-game authoring surface.
It can consume explicit engine diagnostics later, but it does not dynamically load
or reflect over Rust debug plugins.
