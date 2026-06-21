# Graphics — forward renderer, post-FX, and in-game dev draw

Idea docs: `architecture/render.md` (forward / polling / per-draw uniform
rationale — still valid) and `plans/architecture/rendering.md` (the aesthetic
vision). **Note:** `architecture/render.md`'s *status/scope* section is stale — it
still says "single directional light, no shadows, no post-processing," which Part
1 has long since superseded. Treat *this* doc as the current-state source until
that re-baseline lands (see "Doc debt").

---

## Rendering posture (load-bearing — pillar 1 & 4)

The engine is a **living organism that is always running**. There is no
editor/viewport split, no "place objects → compile → run." The game runs; dev
tooling and (future) external tooling talk *to the running game* (see
[bridge.md](bridge.md) §scp seam). Consequences for graphics:

- **No gizmos in an engine overview panel** — there is no such panel. Debug /
  collider / gizmo rendering draws **into the live game world, in dev mode**.
- The future **in-game editing** (drag/select objects on the fly) is an in-world
  dev overlay, not a separate scene editor. Build the line/gizmo pipeline with
  that in mind: world-space, toggled by a dev flag, composited into the live frame.

## What exists now (mature)

- **Forward pipeline**, polling renderer: `render_frame` is one exclusive system
  that re-queries `(Transform, MeshHandle, Material)` each frame and submits draws;
  per-draw model uniform via dynamic offset; surface-loss/resize recovery
  (`render/forward.rs`, `pipeline.rs`, `context.rs`). The "renderer is just a
  system reading resources and components" contract holds.
- **Lighting:** directional + spot + point lights; per-spot shadow maps with PCF
  (`render/light.rs`, `shadow.rs`).
- **Post-FX stack (HDR):** bloom, auto-exposure, color grade, vignette, height fog
  with analytic god-rays, fullscreen overlay slot — each effect a `Copy` World
  *resource* with an OFF/identity default (`render/{bloom,exposure,grade,vignette,
  fog,post,post_overlay}.rs`). The resource-per-effect shape is **ideal for live
  poking** from a dev console or scp.
- **Assets:** glTF mesh + PNG texture loaders, registries with built-ins, F5
  manual reload (`asset/`, `render/registry.rs`).
- **MSAA**; per-instance `Material` (albedo/roughness/emissive).

## What's missing for Kinesis

| Need | Part | Notes |
|------|------|-------|
| **`LineList` / gizmo pipeline** | 2 | None exists — all pipelines are TriangleList+Fill. Feeds Rapier `DebugRenderBackend` *and* the future in-game select/drag. A new pipeline + per-frame growable vertex buffer + a pass slot carved into `render_frame`. |
| Gameplay **particles** (CPU) | 2 | telekinesis hold field, repulsion ring, destruction debris, food scent-cloud |
| **Decals + transparency v0** | 2–3 | wall art, glass; `AlphaBlend` material flag |
| **Frosted-glass** material (Fresnel) | 3 | the eye-reveal trick |
| **Eye-render** shader + state channels | 3 | UV-pan, dilation, glow, drift; selector driven by ECS state |
| Death-sequence overlay (red-noise) | 3 | through the existing post overlay slot |
| Per-instance material **variants** (iron polished/default/pitted) | 4 | drives chamber comfort; already have per-instance `Material`, needs the variant selector |
| Scene loader + transitions | 5 | tear down + load level entities |

## Sharp risks / decisions

- **`render_frame` is a hard-coded inline pass list, not a render graph**
  (shadow→forward→post inlined in one ~740-line exclusive system). This is *fine
  now* and was the right call (no graph until there are many passes), but every
  new pass (debug-draw, decals, particles) edits that body. Watch it: if Part 2–3
  push it past ~3 more passes, factor a minimal pass-sequencing seam before it
  becomes a merge hazard. Not yet.
- **The line pipeline is the one genuinely new GPU primitive Part 2 needs.** Build
  it once, reuse for colliders, force vectors, trigger volumes, and in-game gizmos.

## Doc debt (tracked, not done here)

`architecture/render.md` status/scope and `plans/architecture/rendering.md`
roadmap (off by a game) + technique drift (cascaded-sun vs per-spot; "custom
filmic" vs stock ACES; auto-exposure undocumented) need re-baselining — Part-2
doc work. This overview carries the accurate state meanwhile.

Cross-refs: [physics.md](physics.md) (debug-draw consumer), [bridge.md](bridge.md)
(always-running posture, scp), `plans/plan.md` (renderer staging).
