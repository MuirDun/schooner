# Graphics Audit — Game 0 Forward Renderer

Date: 2026-04-26
Milestone: Game 0 (Phase F sign-off, before Phase G FPS controller)
Scope: `crates/schooner-engine/shaders/` and `crates/schooner-engine/src/render/`

---

## Verdict

The renderer is **structurally sound** and a **good foundation** for Games 1–4. Visual output is correct except for one scene-authoring artifact (z-fighting at the cube/floor seam, screenshot 1). Performance is far below any meaningful budget at the current scale. The few inefficiencies that exist are explicit in the code (commented, named) rather than hidden, so they're easy to fix when entity counts justify it.

Cleared for Phase G.

---

## What's good

### Module organization
- One responsibility per file. `context` (device/surface/depth), `pipeline` (PSO + layouts + uniform buffers), `forward` (the per-frame system), `uniforms` (bytemuck-shaped GPU data), `camera` / `light` / `mesh` / `registry` for components and assets.
- A future deferred path slots in next to `forward.rs` without rearranging anything.

### System-not-subsystem
- `render_frame` is a plain ECS system with declared `Res<…>` / `Query<…>` parameters. No back-channel between `App` and the renderer. Matches the engine's stated philosophy and falls out of the existing query machinery.

### Bind group ordering
- `@group(0)` Camera, `@group(1)` Light, `@group(2)` Model (dynamic offset). This is the long-tail layout: per-frame globals → per-pass/lighting → per-draw. Matches Filament and Bevy at miniature scale. Survives shadow maps, clustered forward, and material additions without renumbering.

### Failure-mode handling
- Surface acquire correctly distinguishes `Lost` / `Outdated` (reconfigure & skip), `Timeout` (skip), `OutOfMemory` (panic, unrecoverable). Most renderers ship with this missing.
- Resize rebuilds the depth texture, not just the swap chain. Zero/min-size guards prevent wgpu validation failures during minimize.

### Geometry tests
- `cube_winding_matches_authored_normals`, `vertex_layout_stride_matches_struct_size`, `cube_normals_are_unit_axis_aligned` are exactly the load-bearing tests — they catch the silent-correctness bugs that would otherwise show as visible artifacts. Worth keeping as the geometry surface grows.

### Forward-compatible asset shape
- `MeshGpu` buffers carry `VERTEX | COPY_DST` and `INDEX | COPY_DST` — a future asset-streaming path can `write_buffer` updates without recreate.
- Vertex layout reserves UV. First texture in Game 1 is a shader change, not a layout change.
- `MeshHandle` is `Copy + 4 bytes` with reserved built-in slots and a clean user allocator.

### Coordinate / depth conventions
- Y-up, RH, NDC depth `0..1`, `Mat4::perspective_rh`. Consistent throughout; matches the resolved decisions in `plans/plan.md`.
- Reverse-Z correctly deferred. The migration when Game 3 lands is a one-day change (compare flip + clear=0.0 + `perspective_infinite_reverse_rh`) — nothing in the current shape fights it.

### sRGB handling
- Surface format selection prefers sRGB. Hardware does the gamma encode on present; the shader writes linear. Correct for an untextured Game 0; will need a real authored-color-space conversation when Game 2 introduces post-processing and Game 3 brings outdoor textures.

---

## Findings — action taken in this pass

### 1. Z-fighting at cube/floor seam (visible in screenshot 1)
**Cause:** cubes spawned at `y=0.5` with extent ±0.5 → bottom face exactly coplanar with the floor at `y=0`. With `Depth32Float` + `Less` the rasterizer flips per-pixel between the two surfaces, producing the sliced/flickering geometry.
**Fix:** scene authoring. `game-void/src/main.rs` now spawns cubes at `y = 0.5 + 0.001`. Renderer state was correct; depth bias is the wrong tool for coplanar opaque geometry.

### 2. Dead code: `PipelineLayoutHandle`
**Cause:** preemptive scaffolding with `#[allow(dead_code)]` for a hypothetical render-graph reuse.
**Fix:** removed. Build it when a render-graph actually exists.

### 3. Unused `_instance` and `adapter` fields on `RenderContext`
**Cause:** retained "for safety" but never read after construction. wgpu's `Device` / `Queue` are refcounted handles that keep the backend alive on their own; `Surface<'static>` carries its own ref to the window via `Arc<Window>`. Holding the instance and adapter is harmless but suggests uncertainty.
**Fix:** dropped both at the end of `RenderContext::new`. Also folded `depth_texture` into the `TextureView` — `TextureView` keeps the underlying allocation alive, and nothing else in the engine ever needs the source `Texture` back.

---

## Findings — recommended later (not now)

### 4. Per-draw model uniform path will not scale past Game 2
- Each draw pays 256 bytes (192 wasted to alignment) + one `set_bind_group(2, …, &[offset])` + one `write_buffer` + one `draw_indexed`. At Game 0 entity counts this is rounding error. At Game 3 (terrain chunks + horde NPCs) it dominates the CPU side of the frame.
- **Migration path** when needed: model matrices → storage buffer indexed by `instance_index` in the VS, one instanced draw per (mesh, material), bind group 2 collapses to a single SBO. The bind group layout that needs to change is exactly one (`create_model_layout`); `render_frame` is the single consumer. Bet kept clean.
- **Trigger:** when frame profiling shows `set_bind_group` + `write_buffer` overhead exceeding ~1 ms in a Game 3 outdoor scene.

### 5. Per-frame `Vec` allocation in `render_frame`
- `renderables.into_iter().collect::<Vec<_>>()` allocates each frame.
- **Fix:** stash a reusable scratch `Vec<(Mat4, MeshHandle)>` on `ForwardPipeline`, `clear()` and `extend(...)` per frame. Trivial; do it the first time `render_frame` is touched again.

### 6. Per-draw `write_buffer` instead of one packed write
- The current loop calls `write_buffer` N times. wgpu coalesces internally so the visible cost is small, but a single `write_buffer` over the packed slice is cleaner and reduces validation noise.
- **Fix:** build a `Vec<u8>` (or write into a scratch buffer) of `MAX_DRAWS_PER_FRAME × MODEL_UNIFORM_STRIDE` bytes once, then one `write_buffer`. Also opportunity to elide writes for empty draws.

### 7. Normal transform shortcut hides a future trap
- The vertex shader uses the upper-3×3 of the model matrix as the normal matrix. The comment correctly flags it as wrong under non-uniform scale.
- The floor *does* use non-uniform scale (20, 1, 20), but its mesh normal is +Y aligned with the un-scaled axis, so the math happens to give the right answer after `normalize()`. The first non-axis-aligned normal on a non-uniformly-scaled entity will silently shade wrong.
- **Fix:** lift the comment into a `// TODO(non-uniform-scale)` referenced from a single tracked location (e.g. a renderer todo file or an inline `unimplemented!`-flavored debug-build check on `Transform::scale.is_uniform()`). Switch to inverse-transpose when the trap is about to be sprung — at the latest, Game 1 if any physics body uses non-uniform scale, otherwise Game 2.

### 8. Plane is single-sided — Phase G's FPS controller will hit this
- The built-in plane is CCW from above with back-face culling on. The instant the controller allows the camera below `y=0`, the floor disappears.
- **Fix:** the natural answer is to clamp the FPS controller's Y above the floor — that's what an FPS controller does anyway. If the floor needs to be visible from below for a debug fly-cam, add a separate two-sided pipeline state for it (or drop the cull mode on the floor draw specifically). Not a renderer change today; a Phase G design constraint.

### 9. Hot-reload of WGSL is reachable
- Currently `include_str!`'d at compile time. To unlock hot-reload:
  1. Switch to runtime read from `crates/schooner-engine/shaders/`.
  2. Watch the file with `notify` (new dep — propose with version + reason).
  3. On change, rebuild the shader module and the `RenderPipeline`.
- The pipeline-rebuild path is already isolated in `ForwardPipeline::new`. Stretch goal for late Game 0 or first chunk of Game 1.

### 10. Cosmetic
- `info.device_type` log uses `{:?}` on a non-stable wgpu enum. Fine for development; if log-parsing tooling enters the picture, switch to a manual match.

---

## What this milestone has *not* bought (intentional)

- No frustum culling. ~110 triangles total; culling overhead dominates savings. Defer to Game 3 outdoor.
- No instancing. 9 cubes; per-instance buffer overhead dominates. Same trigger as #4.
- No tone mapping / HDR. One light at `color=1.0`, base color 0.8 — peak luminance is bounded. Becomes a real conversation when Game 2 stacks point/spot lights.
- No post-processing. Plan correctly defers gamma/fog/vignette to Game 2.
- No MSAA. Sharp edges read fine on the test scene; if egui (Phase H) wants smooth edges in 3D-overlay mode this returns.
- No shadow maps. Game 2 milestone gate.

These are **deliberate non-goals**, not gaps.

---

## Milestone-fit check

Game 0's renderer brief is "forward + one directional + Blinn–Phong, full stop." The shader does exactly that. Nothing has been snuck in early. Nothing that belongs to Game 0 is missing. Cleared.

---

## Concrete followups for plan.md / game0-plan.md

These are not blocking Phase G but are worth recording:

1. Add an item to `plans/game0-plan.md` Phase H or J (whichever covers polish): "Switch shader load from `include_str!` to runtime read; add `notify`-based hot-reload."
2. Add a renderer-debt entry (perhaps in a new `plans/render-debt.md` or as a row in the renderer table): "Per-draw uniform → SBO + instancing migration when frame profiling shows set_bind_group dominance."
3. Add a constraint to Phase G's FPS controller chunk: "Clamp camera Y above floor, OR switch the floor pipeline to two-sided."
4. Add a `// TODO(non-uniform-scale)` reference somewhere visible — easiest is a single-line entry in the renderer-debt doc that points at `forward.wgsl:vs_main` and `pipeline.rs::create_model_layout`.

---

## Files changed in this audit pass

- `crates/game-void/src/main.rs` — cubes lifted by `0.001` to clear z-fighting with floor.
- `crates/schooner-engine/src/render/pipeline.rs` — removed dead `PipelineLayoutHandle`.
- `crates/schooner-engine/src/render/context.rs` — dropped unused `_instance`, `adapter`, and `depth_texture` fields; `create_depth_attachment` now returns just the `TextureView`.

No shader changes. No pipeline-state changes. No bind-group-layout changes. The audit was a cleanup pass, not a redesign.
