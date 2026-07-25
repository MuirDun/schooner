# Debugging — Typed Tools for the Running Engine

Schooner's Rust-side debugging system exists for deep inspection and live tuning
of engine internals. It is how the developer asks the renderer what it is drawing,
shows physics colliders and contacts, inspects audio sources, changes pipeline
modes, and profiles the running world. It is not the future in-game debugging
surface for Glyph, Chronicle, or other authored languages. Those tools serve a
different audience and may reuse engine state, but they do not define this API.

## Host, not catalogue

The debug core knows how to host tools. It does not know what tools every
subsystem might need. It owns the overlay lifecycle, visibility, plugin
composition, and a registry of UI contributions. It does not own physics flags,
render modes, audio controls, game presets, or a universal bag of debug values.

Every inspected subsystem owns its instrumentation and its debug plugin. The
renderer owns shadow-map views and rasterization controls. Physics owns collider,
contact, and force visualization. Audio owns source and mixer inspection. A game
owns game-specific tuning presets. Adding one of these must not require another
match arm or field in the debug core.

## Typed code, dynamic composition

Debug tooling is made available by a dedicated `dev-tools` build flag. An active
binary conditionally composes the requested plugins into its `App`; a shipping
binary omits them. Composition is dynamic at process startup and panel
registration remains dynamic while the app runs, but the plugins themselves are
ordinary statically linked Rust code.

This distinction is deliberate. Rust has no stable dynamic-library ABI suitable
for a long-lived engine plugin contract. Schooner therefore does not load debug
plugins from arbitrary shared libraries. Plugin types, resources, systems, and UI
callbacks stay compiler-checked. The only intentionally name-based boundary is
input action identity, which uses the engine's interned symbols so tooling can
share the normal binding pipeline.

The general app plugin surface is small: a plugin composes resources, systems,
bindings, and debug panels into an `App`. The debug core is one plugin. Diagnostics,
asset reload, renderer inspection, physics visualization, and game tuning are
separate plugins. A convenience plugin group may install the normal engine-side
set, but the core must not depend on the group's members.

## Production state remains authoritative

A target subsystem must never depend on debug tooling to operate. Shipping
configuration lives in typed production resources whether or not `dev-tools` is
enabled. A debug plugin reads or mutates those resources through the same public
surface as any other Rust system. It may own additional visualization state, but
the target subsystem must not read a generic `DebugState` field to decide its
shipping behavior.

For example, the shadow-filter kernel is a renderer resource. The renderer reads
it directly. A render-debug plugin may expose a selector for it, but removing that
plugin leaves the configured kernel and the render path intact. The same rule
applies to fog, bloom, physics parameters, and future audio controls.

## Overlay contributions

The egui overlay is one host over the live ECS world. Registered panels execute
sequentially at a defined Render-stage point and receive access to the world plus
their UI surface. Their subsystem resources and queries remain typed; the registry
stores executable panel contributions, not erased setting values.

The UI frame is built before the renderer encodes the overlay pass. This keeps
panel discovery out of the renderer and avoids forcing `render_frame` to import
every subsystem with debug controls. A missing optional target resource should
make its panel unavailable or empty, not crash the engine.

World-space visualization follows the same ownership rule but uses the live game
view rather than egui. The shared line/gizmo pipeline is infrastructure; physics,
AI, audio, and game plugins decide what lines or markers to submit.

## Input and lifetime

Debug keybindings are ordinary namespaced actions such as
`debug.overlay.toggle`, `debug.assets.reload`, or
`debug.physics.colliders`. They resolve through the same action map as gameplay
and obey the same Update/fixed-step discipline. There is no second debug input
dispatcher.

The `dev-tools` flag controls availability, while plugin installation controls
which tools are active in a particular binary. This permits optimized development
builds with full inspection and release builds with no overlay, bindings, or
debug-plugin state. Tools inspect the always-running world; there is no separate
editor mode or editor-owned copy of engine state.

## Language boundary

Future language-facing debugging is built for authors working inside the game:
rule traces, REPL evaluation, script reload errors, and world-state inspection.
It may consume typed engine diagnostics through an explicit binding, but it is not
a frontend for arbitrary Rust debug plugins. Keeping the two surfaces separate
lets the Rust tooling remain deep and engine-specific without turning the language
runtime into an unsafe reflection layer.
