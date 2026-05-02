# Rendering — The Look and the Pipeline

The renderer's job is to make the world feel like a place worth being in. The aesthetic is specific and constrained; the pipeline is shaped to deliver that aesthetic and nothing more.

---

## How the Renderer Serves the Pillars

**The world is alive.** Pillar 1 reaches into how the world *looks*, not just how it *runs*. A forest that feels hostile to the eye is a world that the player does not want to belong to. A flat, paper-thin canopy is a world that the player does not believe in. The renderer's job is to make the simulated world *feel* simulated — embodied, volumetric, responsive to light and weather.

**Built for one kind of game.** The renderer is not general-purpose. It is built for first-person open-world environments with dense vegetation, day-night cycles, weather, and characters slightly more detailed than the world around them. Every technical commitment below is a decision shaped by that target. We are not building a renderer that could be retargeted to a stylised platformer or a top-down strategy game; we are building this renderer for this game, and that is the source of its strength.

**Developer ergonomics is a feature.** Shaders hot-reload from disk. Material parameters are tunable in the debug overlay. The post-process pipeline is fixed and understandable. When something looks wrong, finding why should take minutes, not days.

**Organism, not castle.** The pipeline grows across games — it does not arrive complete. Game 0 is one directional light and Blinn–Phong; Game 3 is the full outdoor pipeline. The path between is incremental, each game adding what it needs. We do not architect for hypothetical future features; we architect for what comes next.

---

## The Aesthetic

The world should look like a memory of a real place — soft, warm, inhabitable, slightly faded. Not a photograph. Not cartoonish. Something between.

The reference points: **Witcher 1** characters (denser meshes, normal maps, expressive faces, slightly higher fidelity than the world around them), **Gothic 2** mood (dreamy, harsh-but-warm medieval, vegetation that reads as alive), the painterly quality of older RPGs. The forest is the test case: it must feel like somewhere you want to belong, not somewhere hostile to the eye. The player should feel safe enough to *settle*, to want to build a base.

The look has three pillars of its own:

- **Volumetric reads from simple geometry.** Foliage and vegetation must feel three-dimensional even when they are not high-poly. The fix is shading, not geometry: foliage translucency that lets sunlight glow through canopies, hero-quality nearby trees with real branch geometry, shells at mid distance, imposters far away. Skyrim's flat-card foliage is the failure mode we are explicitly avoiding.
- **A muted, warm, dreamy palette.** Tone curve with lifted shadows (no true black on screen), warm height fog, slightly desaturated colour grade with shifted highlights. Specific palette is open — it may not end up gold-toned — but the *feeling* is locked: warm, soft, like a memory.
- **Two material tiers.** Characters get slightly better materials (diffuse + normal + spec mask + cheap skin wrap) than the world around them (diffuse + lighting + foliage translucency). This is the Witcher 1 trick — characters carry the visual fidelity, the world carries the mood, and together they read as a coherent place.

The test of every rendering decision: does this make the world feel more like a memory, or more like a photograph? If photograph, reconsider.

---

## Technical Commitments

These are locked and intended to remain locked across all games. They are the spine of the renderer.

### Forward Rendering, Permanently

One sun, hemisphere ambient (sky tint above, ground tint below, interpolated by surface normal), cascaded shadow maps for the sun. Point and spot lights enter when needed, in modest counts. Forward, never deferred. Even in Game 5.

The reason: deferred buys us nothing for our scene complexity. We do not run dozens of dynamic lights. We do not need per-pixel material variety. Deferred's G-buffer bandwidth is expensive on the hardware tier we target, and its strengths solve problems we do not have.

### MSAA, Never TAA

Multisample anti-aliasing on the main framebuffer. Temporal anti-aliasing is incompatible with the aesthetic: it ghosts on foliage, smears under motion, and softens the image in ways that read as "broken digital" rather than "soft analogue."

### Foliage Translucency Shader

Sunlight that passes through leaves, glowing on the back-lit side. One extra dot product per fragment, transformative effect. This is the cheapest thing in the entire pipeline and the largest single contributor to "the forest does not look flat."

### Hero / Shell / Imposter LOD for Vegetation

- **Hero trees (near):** real branch geometry with leaf clusters as small instanced meshes. A few hundred to ~1000 tris each. Genuinely volumetric.
- **Shell trees (mid):** 3–4 concentric shells with alpha-tested leaf textures and translucency shading. Cheap, hundreds at a time.
- **Imposter trees (far):** billboards rebuilt periodically from snapshots of the hero mesh. Free at distance.
- **LOD crossfade** between tiers so transitions do not pop.

The discipline: the trees the player is *near* must have body. Distance can cheat. Skyrim's failure was using the same flat trick everywhere.

### Two Material Tiers

- **World tier.** Diffuse texture, simple lighting (sun + hemisphere ambient + shadow), foliage translucency for vegetation, height fog. No PBR, no per-pixel specular, no normal maps for most props.
- **Character tier.** Diffuse + normal map + specular mask. Cheap skin wrap shading (warm wrap-light under the diffuse term — same idea as foliage translucency, different parameters). At least one fill light or hemisphere lift on faces so they do not go too contrasty under sharp sun.

The forward pipeline supports both from Game 2A onward.

### Vertex-Shader Wind on Vegetation

A global wind field resource modulates vertex positions in the vegetation shader. Trees sway, grass ripples, foliage breathes. Almost free on the GPU, transformative for "the world is alive." Not animation. Not physics. Vertex math driven by a single wind direction and a few harmonics.

### Fixed Post-Process Pipeline

A single post-pass, in this order, every frame:

1. **Tone mapping.** Custom filmic curve with a shadow lift — pure black never appears on screen. This is the single most "dreamy" parameter.
2. **Colour grading.** Desaturate slightly, warm shift, shadow tint. Implemented as an LUT or a parametric grade.
3. **Warm height fog.** Exponential-by-height, warm-tinted (not white, not grey). The fog is the atmosphere.
4. **Vignette.** Subtle, warm, darkens edges to focus the eye.

That is the post pipeline. Not a stack the artist can rearrange. Not a flexible graph. The composition is the aesthetic; making it flexible would invite drift away from the look we have committed to.

### Screen-Space Contact Shadows (Game 3+)

Cheap (~0.3 ms), and "things sit in the world" is huge for the immersive-sim feel. Lands when the outdoor pipeline does.

### Light Shafts (Game 3+)

Either screen-space god-rays or a coarse volumetric pass — TBD when we get there. *The* Gothic-2 / golden-hour image. Skipping it would be a real loss for the forest.

---

## What We Do Not Build

Locked exclusions. Each is a feature whose cost the engine cannot justify and whose absence does not hurt the game we are making.

- **No PBR.** Material complexity does not serve the aesthetic; the two-tier model carries the look.
- **No deferred rendering.** Forward fits the scene; deferred fits a different game.
- **No global illumination.** Hemisphere ambient + a few hand-placed fill lights in dense forest, plus baked indirect where appropriate, is enough.
- **No screen-space reflections.** Water and other reflective surfaces use cheaper tricks (planar reflections at most).
- **No TAA, no temporal upsampling.** Aesthetic incompatibility.
- **No film grain.** A screenshot trick that becomes noise in motion. The shadow lift and colour grade do the dreamy work.
- **No bloom by default.** Bloom on a warm-graded image quickly becomes "everything glows." Optional faint highlight bloom for specific scenes; off otherwise.

These exclusions are pillar 2 — every tool tailored, no general-purpose features for a general-purpose engine. Refusing to build them is how we ship.

---

## Roadmap by Game

The renderer grows incrementally. Each game adds what it needs and nothing more.

- **Game 0 (done).** Forward pipeline, single directional light, Blinn–Phong, depth, MSAA-ready framebuffer. The minimum that draws cubes on a floor.
- **Game 1 (Kinesis).** No renderer growth strictly required. MSAA enabled. Physics-driven dynamic transforms stress-test the renderer's batching.
- **Game 2A (scripted horror).** Shadow maps (cascaded sun + 1 spot for the flashlight). Point and spot lights. Post-process pass lands (tone curve + colour grade + warm height fog + vignette). Foliage translucency shader written even though horror is interior — keeps the shader path real for Game 3.
- **Game 2B (the hunter).** Skin shading variant. Material tier split (character vs world).
- **Game 3 (Castaway). The big graphics game.** Outdoor pipeline arrives in full: terrain shader with splat blending, vertex-shader wind, hero/shell/imposter LOD chain, light shafts, water, day/night cycle modulating tone curve and fog colour, screen-space contact shadows, optional cheap SSAO if profiling earns it. The aesthetic reaches full strength.
- **Game 4 (Vagrants).** Optimisation pass — instanced rendering for crowds, GPU-driven culling if needed. Probably no new visual features; the look is settled by Game 3.
- **Game 5.** Polish only. No deferred. No GI. The renderer's final form is the one Game 3 establishes.

The renderer is finished in Game 3, in the sense that every commitment above is delivered by then. Games 4 and 5 use it; they do not extend it.

---

## Authoring and Iteration

Pillar 3 reaches the renderer through the day-to-day authoring loop:

- **Shaders hot-reload from disk.** The shader file changes; the next frame uses the new version. A compile error displays in the debug overlay rather than crashing.
- **Material parameters are tunable live.** Tone curve shape, fog density, grade LUT — adjustable in the debug overlay, with the result visible immediately.
- **Asset hot-reload.** Meshes, textures, scenes — a changed file is picked up without restart. This lands with Game 2A's asset pipeline.
- **A render-debug overlay** that shows what the renderer is doing — draw calls, shadow cascades, material counts, hot paths. The overlay is not a luxury; it is how the renderer is understood.

The renderer is a tool the developer uses every day. It earns its complexity by being pleasant to work in.
