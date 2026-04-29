# Render — Architecture Overview

> Status: design fixed for Game 0. Forward-only, single directional light, no asset pipeline.

## The idea

The renderer is **a system, not a subsystem with its own loop**. Frame work runs as one ECS system on the `Update` stage that consumes a wgpu device handle (resource), a mesh table (resource), and three queries (transforms-with-meshes, the active camera, the directional light). The window/event loop is winit's; the GPU is wgpu's; the orchestration is the schedule. The renderer adds nothing on top of those except a forward pipeline and a per-draw uniform.

This is deliberate. Game 0's job is to prove that the engine's foundational shape — "everything is a system reading resources and querying components" — is rich enough to render a scene. If rendering had to live outside the schedule, that contract would already be broken on day one.

## Why a forward pipeline

Forward shading writes lit pixels in one pass: vertex → fragment → swap-chain. Deferred shading writes per-pixel material attributes into a G-buffer first, then lights them in a second pass. Deferred wins when the scene has many small lights interacting with many surfaces — its cost scales with screen pixels, not light count × overdraw. Forward wins when the light count is small and the pipeline must stay simple.

Game 0 has one directional light, ten or so opaque meshes, and no transparency, no shadows, no post-processing. Every reason to pick deferred is absent. Forward is also the easier shader to debug as the user learns wgpu — the path from vertex inputs to a colored pixel is one shader, not three.

Deferred lands future games, alongside global illumination and volumetrics, when the cost of forward's per-fragment light loop becomes real.

## Why polling-flavored, not retained

A retained scene graph (parent/child nodes, dirty propagation, render queue baked once at scene-load) is a tempting shape but it forces decisions Game 0 doesn't have the information for: what's a node, what's a sub-tree, how does instancing key off identity, how does the graph survive ECS spawn/despawn.

A polling renderer asks the world a fresh question each frame — "what entities currently have `(Transform, MeshHandle)`?" — and draws the answer. The state of truth is the ECS; the renderer holds GPU caches keyed by handle, not by entity. ECS deletion is automatically reflected because the entity stops appearing in the query result.

This is worse for scenes with thousands of static objects, because the world is re-queried every frame. At Game 0 scale the cost is invisible. When it stops being invisible — Game 3's outdoor terrain, Game 4's NPC counts — the answer is a dense view cache mirrored from the ECS via the change-detection ticks already present in the storage. The renderer's polling shape doesn't change; only the cost of "what changed" goes from O(N) scan to O(changed).

## Why per-draw uniform with dynamic offset, not push constants

Each draw call needs a different model matrix bound. Three reasonable shapes:

- **Push constants** — write the matrix into the command stream. Fastest, smallest code. Requires `wgpu::Features::PUSH_CONSTANTS`, which is non-default. Available on every desktop backend, but enabling a non-baseline feature couples the engine to it.
- **One uniform buffer per draw, one bind group per draw** — simple to reason about, wasteful at scale, no feature gates.
- **One uniform buffer for all draws, one bind group with dynamic offset** — write the matrix at offset `i * stride` for draw `i`, bind once with the offset varying per draw. Single allocation, no feature flag, baseline-portable.

The third is what Game 0 commits to. Perf parity with push constants is irrelevant at this scale, and "works on a clean wgpu install everywhere" is worth more than micro-optimization. If the model uniform ever shows up in a profile, the path forward is push constants behind a feature flag — but until that profile exists, baseline wins.

## Frame lifecycle

```
winit RedrawRequested  →  App::tick
                            │
                            ▼
                       Schedule::run (Update)
                            │
                       user systems
                            │
                            ▼
                       render_frame system
                            │
                            ├─ update camera uniform
                            ├─ update light uniform
                            ├─ acquire swap-chain texture
                            │     └─ Lost / Outdated → reconfigure, skip frame
                            ├─ encode pass:
                            │     for each (Transform, MeshHandle):
                            │       write model matrix at this draw's offset
                            │       bind model group with that dynamic offset
                            │       draw indexed
                            ├─ submit
                            └─ present
```

`render_frame` lives in the dedicated `Render` stage, engine-appended from inside `App::resumed` after the user's builder chain has finished. The schedule runs `FixedUpdate` × N → `Update` → `Render` per frame, each stage bumping `current_tick` once.

## What the renderer owns (scope)

- The wgpu device, queue, surface, surface configuration, and depth texture.
- Surface (re)configuration on window resize and on `SurfaceError::Lost` / `Outdated`.
- The forward pipeline state object (vertex layout, fragment shader, depth/cull/blend state).
- The mesh registry and the engine-owned built-in meshes: `MeshHandle::CUBE`, `MeshHandle::PLANE`. Built-ins are not Game 0 content — they're test rigs every later game will want, so they live in the engine.
- The camera, light, and per-draw model uniform buffers and their bind groups.
- The `render_frame` system itself.

`Transform` is **not** owned by the renderer. It lives at the engine crate root because it is a scene-graph primitive shared with camera, physics, audio, and AI in later games. The renderer only reads it.

## What the renderer deliberately does NOT own

- **Asset loading.** No glTF, no texture loaders, no scene format. Meshes are procedural. Lands in Game 1 (minimal) and Game 2 (full).
- **Material system.** The shader is hard-coded to Blinn–Phong with global uniforms. Per-object material parameters arrive when there's more than one material to express.
- **Multiple light types.** One directional light, period. Point and spot lights land in Game 2 alongside shadow maps.
- **Shadows.** No shadow maps, no shadow volumes, no SSS. Game 2.
- **PBR / deferred / global illumination.** Game 5.
- **Instancing.** Each `(Transform, MeshHandle)` is one draw call. Instanced rendering lands in Game 4 when NPC counts justify it.
- **Culling beyond the GPU's own.** No frustum culling, no occlusion culling, no LOD selection. Game 3 introduces frustum culling for outdoor scenes.
- **Post-processing.** No tone-map, no bloom, no FXAA, no fog. The shader writes directly to an sRGB swap chain. Lands in Game 2 (gamma + fog) and Game 5 (full).
- **Render graph.** A single forward pass is hard-coded. A graph abstraction lands when there's more than one pass.

## How later phases sit on this

**Phase G (camera controls)** writes `Transform.rotation` and `Transform.translation` on the active camera entity. The renderer doesn't change.

**Phase H (debug overlay)** adds the egui pass inside `render_frame`, between the forward pass and present, via a `DebugOverlay` resource. The forward pipeline doesn't change; `render_frame` gains a second pass.

**Game 1 (physics)** writes `Transform` from Rapier each step. The renderer reads `Transform` the same way it always did.

**Game 2 (assets, shadows, multiple lights)** introduces glTF loading, shadow-map render targets, and additional light component types. The forward pipeline grows passes; the renderer's shape stays "system reading resources and components."

**Game 4 (instancing, LOD)** introduces dense view caches mirrored from the ECS via change-detection ticks. The polling shape becomes "scan the cache" instead of "scan the world," but `render_frame` still runs once per frame and produces draws.

## Open questions to resolve before later phases

- **Asset format** — glTF or custom binary. Decide during Game 1 planning.
- **Shadow map technique** — single-cascade shadow map vs. cascaded shadow maps for outdoor. Decide during Game 2 design.
- **Material model** — when more than one material exists, is "material" a component on the entity, a key inside `MeshGpu`, or a separate registry? Decide when the second material appears.
- **Instancing key** — what makes two draws instance-able? Same mesh handle is the obvious answer, but groups of NPCs sharing a mesh + similar uniforms is the real Game 4 case. Decide during Game 4.

## Inspiration and prior art

- **wgpu's official examples** — primary reference for device init, surface configuration, depth attachment, and bind group layout. The forward pipeline is structurally a `wgpu` example with a polling driver bolted to an ECS.
- **Bevy's `bevy_render`** — confirms "render is a stage of systems reading resources and querying components" is the right shape for an ECS-native engine. Bevy's render graph and pipeline cache are deferred — Game 0 doesn't have the consumers that justify them.
- **The Forge / Sokol** — minimal, single-pass forward pipelines as a proof point that you don't need a graph abstraction until you have multiple passes.
- **Blinn–Phong** — chosen over Lambert / Phong / PBR because it's the cheapest model that produces specular highlights on a directional light, which is what makes 3D geometry read as 3D in the absence of shadows.
