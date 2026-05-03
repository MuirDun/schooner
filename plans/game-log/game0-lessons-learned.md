# Game 0 — Lessons Learned

> Closing artifact for Game 0 ("The Void"). One page on what worked, what hurt, what to change before Game 1, and the load-bearing-but-invisible decisions downstream work inherits. Written 2026-04-29 after Phase H landed and Phase I's polish + Done Bar walkthrough cleared 7 of 8 items (the 8th is a developer-access verification task, not engineering).

---

## What we built

A walkable 3D scene driven by a from-scratch sparse-set ECS, a wgpu forward renderer, a fixed-timestep accumulator loop, a layered keyboard/mouse input layer, an FPS controller, and an egui debug overlay with an in-app puffin profiler. Two crates: `schooner-engine` (library) and `game-void` (binary). Cross-platform from day one (macOS + Linux + Windows), with a `cargo check` matrix on all three OSes via GitHub Actions.

The deliverable that mattered isn't the cubes — it's that every choice was made with Games 1–5 in mind. The shape works.

---

## What worked

- **The plan held.** Game 0 shipped with effectively zero scope creep. Every phase landed close to its written scope; the Done Bar (`plans/game0-plan.md` §1.6) acted as a real anchor whenever a "just one more thing" temptation appeared.
- **Idea-level architecture docs.** `architecture/{render,camera,input}.md` captured the *why* without struct shapes or signatures that rot. They survived multiple refactors with no edits because they were never wrong about the underlying idea.
- **Chunked work + check-ins.** "One chunk, then run, then proceed" caught regressions in the same conversation rather than two phases later. Phase H mid-chunk-3 surfaced a "wait, the scene looks black" moment that turned out to be a screenshot-cropping artifact — but the discipline mattered.
- **Change-detection substrate built early.** Per-component mutation ticks landed in Phase C with no consumer. Phase H's egui scope toggle and Phase 2's reactive cascade engine will both bind onto a working substrate instead of forcing a storage rewrite.
- **Sparse-set ECS over archetype.** Confirmed correct for the long arc: `O(1)` per-component add/remove matches shik's "organism not castle" composability and Game 4's LOD hydration/dehydration cost story. The iteration-cost tradeoff is acceptable through Game 3.
- **Typed `SystemParam` with alias-conflict checking at registration.** Conflicts panic at registration, not on the first frame. Caught zero real bugs in Game 0 because we wrote nothing aliasing — but the safety net is exactly the right shape.
- **`thiserror` for library errors / `anyhow` only in the game binary.** Stayed out of trouble; no ad-hoc `Box<dyn Error>` anywhere.
- **The render system is `fn(&mut World)`.** Going exclusive when typed parameters started fighting us was the right call; the wgpu encoder + frame texture stay on a single stack frame.
- **macOS as primary dev target plus CI for Linux + Windows.** Linux + Windows compile-level breakage will surface on PR, before a contributor ever clones on those OSes.

---

## What hurt

- **wgpu 24 → 29 in the middle of Phase H.** Five major releases at once because we'd been pinned to the version Phase F shipped against. The migration burned half a session: `Surface::get_current_texture` returns an enum now, `InstanceDescriptor` lost `Default`, `request_device` shed an arg, `bind_group_layouts` became `&[Option<&BGL>]`, push constants became "immediate", `multiview` became `multiview_mask: Option<NonZero<u32>>`, render-pass attachments gained `depth_slice`. **Lesson:** pin to current latest at project start; plan a controlled bump roughly every 6 months instead of riding ten majors at once.
- **`puffin_egui` chase.** Spent meaningful time hunting for a working egui+puffin pair before realizing puffin_egui permanently lags egui by one minor. Building our own snapshot panel from `puffin::GlobalProfiler` ended up cleaner anyway. **Lesson:** when an ecosystem shows a release-cadence mismatch like this, don't try to chase it — read it as a signal to own the integration.
- **The 6-arg `SystemParam` ceiling.** `render_frame` walked into it the moment egui added another resource. Going exclusive was the right escape, but the ceiling will reappear in Game 1 (physics-step + collision-events + force-application all need similar fan-in). The current arity expansions (0–6) are 60-line copy-pastes — repeating that to 8 is mechanical, but a `SystemParam` bundle or multi-resource fetch helper would scale better.
- **Two-resource `&mut` borrow conflicts in `render_frame`.** When the egui closure needed `&mut DebugState` while `&mut DebugOverlay` was already held, the cleanest path took two iterations to find. The "snapshot in / write back out" pattern is now the house style (recorded as feedback memory). The underlying gap is that the resource API exposes only single-resource fetch.
- **Rendered-data jitter on the profiler panel.** First version of the per-scope readout re-tessellated every frame at 100 fps and the digits were unreadable. Throttling to 500 ms with averaging fell out cleanly once we noticed, but the UX miss should have been predictable from the start. **Lesson:** any live numeric readout needs an explicit refresh policy, not "as fast as possible."
- **Cube-face winding bugs in Phase F.** The first cube generator had two faces wound the wrong way and a third with flipped normals. The geometric-normal unit test caught all three within minutes; without it, we'd have spent days chasing "why is one face dark." **Lesson:** any hand-authored mesh data needs a generated-normal-vs-authored-normal cross-check.

---

## What to change before Game 1

These are the items worth resolving before Phase 1 (Kinesis) starts laying physics on top:

1. **Bump `SystemParam` arity to 8 OR design a `SystemParam` bundle.** Game 1's physics step + collision-event reader + force-applicator all want fan-in. Decide: mechanical extension to 8/10, or a derive macro that bundles into one logical param? Bias: bundle, since the same need recurs every game.
2. **Add a multi-resource fetch helper to `World`.** `World::resources_mut2::<A, B>()` (or `n`-arity variant) with the unsafe living once in the world impl. Removes the snapshot-in/write-back-out tax for every future overlay-style cross-resource interaction. Bevy's `SystemState` is the prior art.
3. **Decide `schooner-ecs` / `schooner-render` extraction.** After 8 phases the module boundaries inside `schooner-engine` are clean — there's no API pain pushing extraction. **Recommendation: keep as modules.** Extract only when a third game wants to depend on one without the other. The maintenance overhead of multiple crate manifests outweighs any architectural benefit at current scale.
4. **Pin a system-ordering convention.** Registration order works for Game 0 but Game 2's many interacting systems will outgrow it. Bevy-style `before(...)` / `after(...)` annotations? Stage sub-stages? Decide before Game 2; for Game 1, registration order is still fine — physics + render are clearly separated stages.
5. **Plan the multi-pass renderer shape.** Game 2 wants shadow-map pre-pass + main forward pass + post-process. Today's `render_frame` is a single exclusive system; splitting into multiple systems sharing in-flight frame state via a `FrameInFlight` resource is the obvious next move, but defer the actual split until Game 2 has a concrete second pass to motivate it.
6. **Decide the asset format.** glTF for meshes (Game 1 minimum) — confirmed in `plans/plan.md` Open Decisions. Hot-reload strategy is the open question; defer until Game 2 if Game 1 only needs one-shot loads at level start.
7. **Bump dep majors deliberately, not all at once.** A scheduled review of workspace deps every ~6 months beats the "five-majors-in-one-PR" trap we hit with wgpu. Pin and watch.

---

## Quiet but load-bearing for later

These decisions are invisible from the running binary today but are the reason later games won't have to throw shape away:

- **`ComponentId`-based join internals under a typed `Query` surface.** Means shik's eventual `world.query_dyn(&[component_ids])` is a new public surface, not a storage rewrite. Already implemented in Phase C.
- **Three-stage closed enum (`FixedUpdate` / `Update` / `Render`), each runner bumps `current_tick` once.** Uniform tick semantics keep change-detection comparisons simple. The reactive cascade engine in Game 2 may revisit if there's a reason; until then, uniform rule wins.
- **Resources are `TypeId`-keyed in a HashMap.** Trivial to add a typed bridge to shik-side resources later — TypeId is the natural identity for a typed-contract boundary.
- **Mesh handles are dense `u32` indices into a registry, not `EntityId` references.** Means GPU resources don't tangle with entity lifecycle; instancing in Game 4 keys off mesh handle naturally.
- **Built-in cube + plane meshes are engine-owned, not game-owned.** Every later game (physics demo, scripting REPL, debug spawns) will want them; carrying them in the engine avoids a copy-paste cycle.
- **`Transform` lives at the engine crate root, not under `render/`.** Camera / physics / audio / AI all read transforms in later games; putting it inside `render/` would have forced every later subsystem to import the renderer for its pose type.
- **Input has a two-layer architecture** (raw polling state shipped, action map deferred to Game 1's rebindable controls or Game 2's shik bindings). The Layer 1 surface is small enough that adding Layer 2 is additive, not a rewrite.
- **Cursor grab uses `Locked` first, falls back to `Confined`.** Standard cross-platform recipe; works on macOS / Win / Linux without per-OS branching.
- **`pollster::block_on` for one-shot device init.** No async runtime needed; wgpu's async surface is a one-time cost during `App::resumed`.
- **The change-detection substrate exists in storage from day one.** Game 2's reactive cascade engine wires onto a working substrate; nobody needs to retrofit ticks into a storage that was already shipped.

---

## The honest summary

Game 0 took the time it needed and produced an engine skeleton that doesn't have to be thrown away. The two areas that will demand attention before Game 1 starts are the system-parameter ergonomics (arity / multi-resource fetch) and the system-ordering convention. Everything else is either fine or can wait for the game that motivates it.

The plan worked. The check-in cadence worked. Idea-level docs worked. Sparse-set + change-detection + typed-but-id-internal queries worked. Build the scripting language onto this in Game 2 and let it dictate the next round of shape decisions.
