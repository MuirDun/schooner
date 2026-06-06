# Part 1 — Mood

**Kind:** Tech buildout (renderer & atmosphere)
**Status:** Phases 1.A–1.G completed
**Depends on:** Game 0 (complete)

---

## Goal

Indoor renderer extensions and atmosphere foundations that make a static scene **read as Kinesis**. By end of Part 1, the playground is a single sealed room dressed with real Kinesis assets — rusted iron walls, harsh spotlight from above with a visible god-ray cone cutting through fog, a dim service corridor visible through a doorway lit by a red lamp, a dark window in one wall with placeholder eye geometry behind frosted glass — walkable in the Game-0 FPS controller. The aesthetic is unmistakable.

## The question this Part answers

**Does the aesthetic land?**

If the answer is no — if rusted iron looks plastic, if spotlights don't carve the room, if fog feels weather-y instead of oppressive, if mood doesn't shift between zones — we discover it before any verb or system depends on the visual language. The fix is bounded to Part 1.

## In scope

- Per-instance material parameters and three-state iron variants
- Multi-light shading (spot + point lights)
- Per-light shadow maps for spot lights (single map per light, indoor-scoped)
- Post-process pipeline v0: tonemap, color grade, vignette, fullscreen overlay slot
- Atmospheric fog with light-shaft / god-ray scattering
- Asset pipeline v0: glTF mesh loader, texture loader, manual reload
- Decal & transparency support (textured-quad decals, alpha-blend material flag)
- Playground binary target dressed with real asset kit

## Out of scope

- Physics, telekinesis, any player verb (Part 2)
- Eye and tentacle animation states (Part 3)
- Audio, even ambient beds (Part 3)
- Attitude system, hunger, save, persistence (Part 4)
- Cascaded shadow maps (Game 2A — outdoor)
- File-watcher hot reload (Game 2A — Part 1 ships manual reload only)
- Projected decals proper (Game 2A — textured-quad approximation here)
- PBR (architecture vision is permanently Blinn-Phong + warm grade)

## Phases

### Phase 1.A — Per-instance material params

Extend Game 0's single-albedo Blinn-Phong to per-instance material parameters so the same iron geometry can present three visual states (polished / default / pitted), and so emissive surfaces (red lamps, food gel) are first-class.

**Steps:**
- [x] **1.A.1** Define `Material` component at engine root (alongside `Transform`). Fields: `albedo: Vec3`, `roughness: f32`, `emissive: Vec3`, `emissive_intensity: f32`, `blend: BlendMode` (`Opaque` for now; `AlphaBlend` lands in 1.G).
- [x] **1.A.2** Extend the per-draw uniform (currently model matrix only) to carry material params alongside the transform. Reuse the existing dynamic-offset uniform pattern.
- [x] **1.A.3** Update the WGSL shader to read material params per-instance; apply albedo + roughness modulation to Blinn–Phong; add emissive contribution outside the lighting equation.
- [x] **1.A.4** Smoke test: three cubes in the existing G0 scene, each with a different `Material` (one warm-tinted polished, one neutral, one cool-tinted dim) — visibly distinct under the existing directional light.

### Phase 1.B — Multi-light shading

Replace G0's single-directional-light shader with a small dynamic light array that supports spot and point lights. No shadows yet — that's the next phase, kept separate because shadows are the fiddly part.

**Steps:**
- [x] **1.B.1** Define `DirectionalLight`, `SpotLight`, `PointLight` components. Fields per architecture/rendering vision: spot has position + direction + color + intensity + inner/outer cone; point has position + color + intensity + range; directional has direction + color + intensity.
- [x] **1.B.2** Per-frame light-collection system: gather lights into a fixed-size uniform buffer (e.g. up to 1 directional + 8 spot + 16 point) with an active count.
- [x] **1.B.3** WGSL: iterate active lights, accumulate Blinn–Phong contribution per light. Range attenuation for point; cone + range attenuation for spot.
- [x] **1.B.4** Smoke test in playground precursor: a white spot light from above + a dim red point light in the corner. Both visibly contribute. No shadows yet — surfaces are still bright on the back side of objects, expected.

### Phase 1.C — Shadow maps for spot lights

Add per-spot-light shadow maps. Single shadow map per casting light, indoor-scoped (small frustum, modest resolution). This is the phase that makes spotlights actually *carve* the room.

**Steps:**
- [x] **1.C.1** Depth-only shadow render pass infrastructure. A `Shadowcaster` marker component on lights that should cast shadows (so not every light pays the cost).
- [x] **1.C.2** Per-light shadow map allocation: one 1024×1024 depth texture per shadowcasting spot light, recreated when the set of casters changes. *(Landed as a single 2D-array depth texture with `MAX_SHADOW_CASTERS = 4` layers — `binding_array<texture_depth_2d>` needed `Features::TEXTURE_BINDING_ARRAY`, which narrows adapter compatibility; `texture_depth_2d_array` is core wgpu.)*
- [x] **1.C.3** Per-light view-projection matrix from the spot's position + direction + cone.
- [x] **1.C.4** Main pass reads shadow maps with PCF (3×3 or 5×5) for soft edges.
- [x] **1.C.5** Smoke test: a single spot light with shadowcaster on, a few cubes in the cone. Shadows fall correctly. Toggle PCF kernel size with a debug key to compare. *(Spot orbits the cube grid via game-side `OrbitingSpot` so shadows sweep at all angles in one run; P cycles Single / Soft3x3 / Wide5x5.)*

### Phase 1.D — Post-process v0

Introduce an offscreen HDR render target and a post-process chain. This is the phase that gives us tonemap, color grade, vignette, and — most importantly — a **fullscreen overlay slot** that Parts 3 and 4 will drive (death red-noise, cold-open, hunger tint).

**Steps:**
- [x] **1.D.1** Render the main scene into an HDR offscreen color target (Rgba16Float) instead of directly into the swap chain. *(Forward target switched to `RenderContext::hdr_view()`; new `PostPipeline` resource owns the fullscreen-triangle shader, BGL, sampler, and a generation-tracked cached bind group rebuilt on surface resize. Egui draws after post so debug UI is never graded. `post_pass` profiler scope ~0.004 ms.)*
- [x] **1.D.2** Fullscreen post-process pass: ACES-ish tonemap to LDR. *(Shipped alongside 1.D.1 — fragment shader runs Narkowicz 2015 fitted ACES per-channel; `shaders/post.wgsl` documents the curve choice and alternatives (Hill, Hable, Reinhard). The G0 sun was still at the 1.C.5 diagnostic value of 0.08 at landing time; restore to default once verified under tonemap.)*
- [x] **1.D.3** Color grade: a per-scene `ColorGrade` resource with lift / gamma / gain (or a simple LUT pointer for later). Drives the chamber/cage/service-space distinct looks. *(ASC CDL primary grade, post-tonemap. `ColorGrade` resource at engine root (re-exported from `render::grade`) carries `lift/gamma/gain: Vec3` plus `DEFAULT`, `CHAMBER_WHITE`, `CAGE_WARM`, `SERVICE_RED` preset constants. New 48 B `PostParamsUniform` packed each frame into a stable `@group(1)` on `PostPipeline`; struct grows for 1.D.4 (vignette) and 1.D.5 (overlay), bind group stays. LUT path explicitly deferred to Game 2A so engine tooling lands with universal scope rather than as an intermediate. App seeds the resource with `ColorGrade::default()` — scene is visually unchanged until a debug cycle in 1.D.6 swaps it.)*
- [x] **1.D.4** Vignette: radial darkening with color tint (intensity + tint as `ColorGrade` fields). *(Plan deviation: vignette lives as its own `Vignette` resource in `render/vignette.rs`, not as fields on `ColorGrade`. Reasoning: color grade is colorist/CDL territory; vignette is a compositor effect modulated independently per scene mood. `PostParamsUniform` grew to 80 B (two new vec4 slots — `vignette_tint` packing intensity into `.w`, `vignette_radii` packing inner+outer into `.xy`). WGSL `apply_vignette` uses `smoothstep(inner, outer, length(uv − 0.5) · √2)` then `mix(scene, tint, v · intensity)` — elliptical (aspect-tracking), applied after grade. Presets: `DEFAULT` (off), `CINEMATIC` (subtle), `OPPRESSIVE` (tight/heavy). Default state is off so unmodified scenes are unaffected.)*
- [x] **1.D.6** Smoke test: playground precursor with three `ColorGrade` presets (chamber-white, cage-warm, service-red), switchable by debug key. Visibly different moods on the same geometry.

### Phase 1.E — Atmospheric fog & god-rays

Make spotlight cones visible by scattering in the lighting model + a light-shaft contribution. This is the single biggest aesthetic win of Part 1.

**Steps:**
- [x] **1.E.1** `Fog` resource: per-scene height-fog params (color, base height, density, falloff). Applied in the main shader as in-scattering on the lit color (exponential height fog). *(Landed 2026-05-19. `Fog` lives at engine root in `render/fog.rs`. Folded into the existing `LightsUniformData` at `@group(1)` rather than its own bind group — the same uniform feeds 1.E.2's god-ray loop, so co-location keeps the pipeline at four bind groups (wgpu's default `max_bind_groups`). WGSL applies Wenzel 2007's closed-form integral of `density · exp(-falloff · Δy)` with a Taylor guard at horizontal rays. `density = 0` short-circuits to transmittance = 1. F4 cycle landed in `debug.rs` as a `FogPreset` enum (`Off / Medium / Heavy`) — placeholder shape, 1.E.3 reshapes into the per-zone presets matching `ColorGrade`. `scattering` deliberately deferred to 1.E.2.)*
- [x] **1.E.2** Spot-light god-ray contribution: analytic in-scattering through the spot cone, computed in the main shader against the fog medium. Cheaper than screen-space radial blur and reads correctly even with shadow occlusion. *(Landed 2026-05-19. `Fog::scattering` decoupled from `density` (UE/Frostbite split — "thick atmosphere with subtle glow" and "thin atmosphere with bright god-rays" are independently authorable). Per-spot `SpotLight::god_ray_intensity` multiplier on the medium's scattering, default 1.0, packed into the previously-reserved `outer_cos_shadow.z` slot (zero growth in `SpotLightUniformData`). WGSL `ray_cone_segment` solves the `(D·V)² ≥ cos²θ·|U|²` quadratic with both `a < 0` (narrow cone) and `a > 0` (wide / near-axial) branches, plus a front-half-plane clip for the back-mirror cone and a range-sphere clip. `t_max = view_len` so walls correctly occlude the god-ray. In-scatter evaluated as `spot_radiance · scattering · god_ray_intensity · density_at_mid · seg_len · transmittance(camera→mid)` — Toth 2009 midpoint analytic. Additive composition: `mix(fog_color, scene, T) + god_ray_sum`, the second term of the radiative-transfer equation joining the first. `do_god_rays` flag is a uniform branch so disabling fog is divergence-free.)*
- [x] **1.E.3** Per-scene fog presets matching the `ColorGrade` zones (chamber, cage, service-red, labyrinth). *(Descoped 2026-05-19. Engine-level zone presets deferred — the developer will author scene-specific fog values directly against the playground binary (Phase 1.H), the same way per-scene lighting setups will be authored there. The 1.E.1/1.E.2 work landed `Fog::DEFAULT` plus the F4 debug cycle (`Off / Medium / Heavy`); zone-named constants on `Fog` would have been speculative without the playground to tune against.)*
- [x] **1.E.4** Smoke test: playground precursor with a single spot light, fog enabled. Cone is visible as a god-ray. Walk through it — fog density and scattering read correctly from inside vs outside the beam. *(Verified 2026-05-19 against the orbiting-spot Game-0 scene. The F4 cycle from 1.E.1 carried this — `Off` confirms the unfogged path is a true no-op; `Medium` shows the cone glowing in the air, walls correctly clipping the god-ray via `view_len`; `Heavy` darkens the transmittance term and reads as a denser-medium environment. Playground-proper revisit lands in 1.H when the dedicated scene exists.)*

### Phase 1.F — Asset pipeline v0

Replace G0's hardcoded cube/plane workflow with on-disk glTF meshes and textures, plus a manual-reload key. No file watcher — Game 2A's job.

**Steps:**
- [x] **1.F.1** glTF mesh loader: parse a `.gltf` / `.glb` file into vertex + index buffers, register into a `MeshRegistry` keyed by `MeshHandle`. Built-in cube/plane stay registered eagerly for debug.
- [x] **1.F.2** Texture loader: load PNG/KTX → wgpu texture; register into a `TextureRegistry` keyed by `TextureHandle`.
- [x] **1.F.3** `Material` extended: `albedo_texture: Option<TextureHandle>` slot (multiplied with `Material.albedo` if present).
- [x] **1.F.4** Manual reload: a debug key (F5) re-reads tracked asset files from disk and updates registry entries in place. Failures are non-fatal — previous version keeps running, error logged. *(Landed 2026-05-24. `reload_all` on both registries walks a snapshot of `(handle, source_path)` pairs, re-runs the loader, and swaps the `MeshGpu`/`TextureGpu` in place under the same handle — source path preserved so the next F5 finds it again. Returns a generic `ReloadReport<H> { reloaded, failed }`; per-failure detail logs at `warn` inside `reload_all`, the count rides the report for the caller's one-line summary. `f5_reload_system` lives in `debug.rs` next to the other F-row affordances but is registered from `App::resumed` (not `App::new`) because it needs `RenderContext` + the registries + `ForwardPipeline`, which only exist post-device; folding it into `debug_input_system` would have forced that system to declare resources absent on frame 1. Texture reloads invalidate the forward pipeline's per-handle material bind group — the cache keys on handle but bakes in the old `TextureView`. Meshes need no such step: `render_frame` reads `MeshGpu` buffers straight from the registry each draw. Verified live by editing `rusty.png` mid-run + F5 (albedo updates) and by renaming it away + F5 (warn logged, last-good texture kept, no crash).)*
- [x] **1.D.5** (Moved from phase D, required texture) Fullscreen overlay slot: a `PostOverlay` resource exposing an overlay texture + intensity + blend mode. Wired through the post-process pass; default-off. Consumers (death, hunger) come in later Parts. **Blend modes**: three are required because the announced consumers each need different math — *AlphaBlend* (death-red-noise, where the texture's alpha channel masks the noise pattern to specific screen regions), *Multiply* (hunger, where a warm tint modulates the entire frame), and *Additive* (flashes — instrument-scene glow, lightning). Naming the triad up-front prevents drift when later Parts hit a consumer the slot can't express. **Bind group**: the overlay texture rides on its own bind group separate from the HDR-sampling group and the post-params uniform group, because its change frequency is consumer-driven (gameplay flips the overlay) rather than surface-resize-driven (HDR) or per-frame-driven (params). Three groups = three independent rebuild lifecycles.
- [x] **1.F.5** Smoke test: author a single rusted-iron wall panel (mesh + albedo texture) in Blender, export glTF + PNG, load it, swap a playground wall to use it. Tweak the PNG, hit F5, see the change.

### Phase 1.G — Decals & transparency

Alpha-blended materials for wall art decals and frosted glass. Textured-quad decals (not projected) — depth-biased to avoid z-fighting against the wall they sit on.

**Steps:**
- [x] **1.G.1** `Material.blend` honored: `Opaque` (default) renders in the opaque pass; `AlphaBlend` is collected into a transparent pass.
- [x] **1.G.2** Transparent pass: rendered after opaque, sorted back-to-front by camera distance.
- [x] **1.G.3** Depth-bias control on `Material` (`f32` polygon offset) so decal quads sit cleanly on host surfaces.
- [x] **1.G.4** First decal asset: one scratched-stick-figure wall drawing (texture with alpha) on a quad mesh. Authored in Blender / Krita.
- [x] **1.G.5** Glass material: an alpha-blended iron-frame-around-frosted-pane composite. Frost is a simple tint + high-Fresnel specular — not real refraction.
- [x] **1.G.6** Smoke test: place a wall-art decal on the playground's iron wall (visibly flush); install the eye-window with its frosted-glass material; place a placeholder eye geometry behind it that's dimly visible.

### Phase 1.H — The playground

A single sealed indoor space using everything Parts 1.A–1.G produced. This is the artifact every subsequent Part returns to.

**Steps:**
- [ ] **1.H.1** New binary target: `crates/game/src/bin/playground.rs`. Default `main.rs` becomes the game stub (will be Kinesis proper later). Playground launched via `cargo run -p game --bin playground`.
- [ ] **1.H.2** Playground room: a ~6×6×4m sealed iron chamber, one wall with a frosted-glass window, one doorway opening into a short service corridor lit by a red point light.
- [ ] **1.H.3** Lighting setup: one directional fill (very dim), one shadow-casting white spot from above the chamber, one red point in the service corridor, one dim spot inside the cavity behind the window.
- [ ] **1.H.4** Materials: chamber walls use the rusted-iron material; the wall flagged for state-cycling carries all three variants accessible via debug keys (1 = polished, 2 = default, 3 = pitted).
- [ ] **1.H.5** Dressing: a handful of static sulfur-block meshes (no physics yet — purely visual), a placeholder gel-brick with emissive material, the eye placeholder behind the glass.
- [ ] **1.H.6** Debug controls (dev keys, not gameplay): cycle iron state; cycle scene `ColorGrade` between chamber / cage / service / labyrinth; dim player-side lights to test the eye-reveal trick; toggle fog density.
- [ ] **1.H.7** FPS controller from G0 carries over unchanged. Mouse capture, WASD, look. Existing F1 debug overlay still works.
- [ ] **1.H.8** One wall-art decal scratched on the iron, to validate decals in the live scene.

---

## Done Bar

The Part is complete when all of the following are true in the playground binary:

- [ ] Three iron states are visibly distinct on the same geometry.
- [x] At least one shadow-casting spot light carves the room with PCF-soft shadows. *(Confirmed 2026-05-14 with an orbiting spot in the Game-0 smoke scene; sun dialled down to 0.15 to keep the floor unsaturated until 1.D's tonemap lands.)*
- [x] At least one point light contributes localized warm/red illumination.
- [ ] Fog is enabled; the spot light cone reads as a visible god-ray; fog density adjustable at runtime.
- [ ] At least three `ColorGrade` zones (chamber-white, cage-warm, service-red) are switchable, with visibly different moods on the same geometry.
- [ ] Vignette is in the pipeline; the overlay slot exists and can be driven by a debug key with a test texture.
- [ ] At least one mesh and one albedo texture are loaded from disk via the glTF/texture loader.
- [ ] Manual reload (F5) reloads a texture or mesh in place without restart.
- [ ] At least one decal sits flush on an iron wall without z-fighting; at least one frosted-glass window is present with placeholder eye geometry visible behind it.
- [ ] Walking through the playground, the developer believes the aesthetic is right. (Subjective, but the gate.)

If the last bullet fails, fix the underlying renderer or material work before starting Part 2. **Don't proceed on mood that isn't carrying.**

---

## Followups

- Step-level plans for Parts 2–7 are deliberately deferred until the preceding Part is complete and has informed them.
- The light-shaft technique choice (analytic in-scattering vs screen-space god-rays) is committed here to analytic; the engine roadmap leaves "light-shaft technique for Game 3" open — this Part's choice may inform that decision.
