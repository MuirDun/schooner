//! `render_frame` — the system that paints the frame.
//!
//! Frame flow (also documented in `architecture/render.md`):
//!
//! - Resolve the active camera. If none exists, skip the frame.
//! - Snapshot the draw list, lights, and shadowcaster view-proj
//!   matrices through ECS queries.
//! - Acquire a swap-chain texture; on `Lost`/`Outdated` reconfigure
//!   and skip.
//! - Reallocate the shadow-map set if the caster count changed.
//! - Write camera, lights, model, and shadow-VP uniforms before any
//!   pass begins — the shadow and forward passes share the model
//!   buffer through separate bind groups, so a single write feeds
//!   both.
//! - For each shadowcaster: depth-only render pass into its shadow
//!   map, rendering the entire draw list from that light's POV.
//! - Forward pass — same draw list, lit + shadow-sampled. Output is
//!   linear HDR written into the `RenderContext` HDR target.
//! - Post pass — fullscreen triangle that samples the HDR target
//!   and writes the swap-chain texture. 1.D.1 is a passthrough
//!   clamp; later 1.D Steps stack tonemap, grade, vignette, overlay
//!   inside the same shader.
//! - Egui overlay pass on top of the swap-chain texture. Drawing
//!   egui *after* post keeps the debug UI uncoloured by the
//!   gameplay grade.
//! - Submit + present.
//!
//! ## Why exclusive
//!
//! `render_frame` is the last system in the per-frame schedule and
//! touches a wide set of globals — `RenderContext`, `ForwardPipeline`,
//! `MeshRegistry`, `DebugOverlay`, plus three component queries.
//! Wrapping it as an exclusive system (`fn(&mut World)`) avoids the
//! 6-tuple `SystemParam` arity ceiling and keeps the wgpu encoder
//! and frame texture on a single stack frame instead of split across
//! `Res`/`ResMut` borrows. Renderer parallelism is not a Game 0
//! concern; revisit when the parallel scheduler arrives.

use glam::{Mat4, Vec3};
use log::warn;
use wgpu::{
    CommandEncoderDescriptor, IndexFormat, LoadOp, Operations, RenderPassColorAttachment,
    RenderPassDepthStencilAttachment, RenderPassDescriptor, StoreOp, TextureViewDescriptor,
};

use crate::camera::{ActiveCamera, Camera};
use crate::debug::{
    DebugState, OverlayInteract, OverlayMetrics, PcfKernel, ProfilerView, build_overlay_ui,
};
use crate::ecs::World;
use crate::material::{BlendMode, Material};
use crate::render::context::RenderContext;
use crate::render::fog::Fog;
use crate::render::light::{DirectionalLight, PointLight, Shadowcaster, SpotLight};
use crate::render::mesh::MeshHandle;
use crate::render::overlay::DebugOverlay;
use crate::render::grade::ColorGrade;
use crate::render::pipeline::{ForwardPipeline, MAX_DRAWS_PER_FRAME, MODEL_UNIFORM_STRIDE};
use crate::render::post::PostPipeline;
use crate::render::post_overlay::PostOverlay;
use crate::render::registry::{MeshRegistry, TextureRegistry};
use crate::render::texture::TextureHandle;
use crate::render::shadow::{
    MAX_SHADOW_CASTERS, SHADOW_VP_UNIFORM_STRIDE, ShadowMaps, ShadowPipeline, compute_shadow_vp,
};
use crate::render::uniforms::{
    CameraUniformData, DirectionalLightUniformData, LightsUniformData, MAX_POINT_LIGHTS,
    MAX_SPOT_LIGHTS, ModelUniformData, PointLightUniformData, PostParamsUniform,
    SpotLightUniformData,
};
use crate::render::vignette::Vignette;
use crate::time::Time;
use crate::transform::Transform;

/// Background color when the swap chain clears each frame. Mid-grey
/// rather than pure black so a scene with no draws is visibly
/// "rendering nothing" instead of "renderer crashed."
const CLEAR_COLOR: wgpu::Color = wgpu::Color {
    r: 0.02,
    g: 0.02,
    b: 0.02,
    a: 1.0,
};

/// Collect all light components in the world and pack them into a
/// single `LightsUniformData`. Spot and point lights pair with a
/// sibling `Transform` (translation = position, rotation = aim);
/// directional is positionless. Overflow past the fixed caps is
/// warned and dropped.
///
/// Returns the lights uniform plus the per-shadowcaster view-proj
/// matrices, in `shadow_index` order. The returned VP vec aligns
/// 1:1 with the shadow-pass loop and with each spot's
/// `shadow_index` field in the uniform — index `i` in the vec is
/// the matrix the shadow pass writes into layer `i` and the
/// matrix referenced by any spot bearing `shadow_index = i`.
fn build_lights_uniform(world: &mut World) -> (LightsUniformData, Vec<Mat4>) {
    let mut data = LightsUniformData::zeroed();
    let mut shadow_vps: Vec<Mat4> = Vec::new();

    // Directional: first one wins. Fall back to the placeholder's
    // ambient-grey when no DirectionalLight exists.
    match world.query::<&DirectionalLight>().into_iter().next() {
        Some(dir) => {
            data.directional = DirectionalLightUniformData::new(
                dir.direction,
                dir.color,
                dir.intensity,
                dir.ambient,
            );
            data.counts[0] = 1;
        }
        None => {
            data.directional = DirectionalLightUniformData::new(
                Vec3::new(0.0, -1.0, 0.0),
                Vec3::ZERO,
                0.0,
                Vec3::splat(0.3),
            );
            // counts[0] stays 0 — the shader skips directional
            // contribution but still reads ambient from this slot.
        }
    }

    // Spots: iter the components, resolve sibling Transform per
    // entity (same iter-then-get pattern as the mesh draw path).
    // A second component lookup checks for `Shadowcaster` — when
    // present and under the cap, the spot gets the next
    // `shadow_index` and contributes its VP to `shadow_vps`.
    let spot_entities: Vec<crate::ecs::EntityId> =
        world.iter::<SpotLight>().map(|(e, _)| e).collect();
    let mut spot_count = 0usize;
    let total_spots = spot_entities.len();
    let mut total_casters = 0usize;
    for entity in spot_entities {
        if spot_count == MAX_SPOT_LIGHTS {
            break;
        }
        let Some(transform) = world.get::<Transform>(entity).copied() else {
            continue;
        };
        let Some(spot) = world.get::<SpotLight>(entity).copied() else {
            continue;
        };
        // Default spot-forward is local -Z (camera-forward convention).
        let direction = (transform.rotation * Vec3::NEG_Z).normalize_or_zero();

        // Shadow assignment. The total caster count includes
        // overflow so the warn fires accurately; the assigned
        // index is `-1` once the cap is reached.
        let is_caster = world.get::<Shadowcaster>(entity).is_some();
        let (shadow_index, view_proj_cols) = if is_caster {
            total_casters += 1;
            if shadow_vps.len() < MAX_SHADOW_CASTERS {
                let vp = compute_shadow_vp(&transform, &spot);
                let idx = shadow_vps.len() as i32;
                shadow_vps.push(vp);
                (idx, vp.to_cols_array_2d())
            } else {
                (-1, Mat4::ZERO.to_cols_array_2d())
            }
        } else {
            (-1, Mat4::ZERO.to_cols_array_2d())
        };

        data.spots[spot_count] = SpotLightUniformData::new(
            transform.translation,
            direction,
            spot.color,
            spot.intensity,
            spot.range,
            spot.inner_cone_cos,
            spot.outer_cone_cos,
            shadow_index,
            spot.god_ray_intensity,
            view_proj_cols,
        );
        spot_count += 1;
    }
    if total_spots > MAX_SPOT_LIGHTS {
        warn!(
            "render_frame: {} SpotLights exceeds MAX_SPOT_LIGHTS ({}); dropping overflow",
            total_spots, MAX_SPOT_LIGHTS
        );
    }
    if total_casters > MAX_SHADOW_CASTERS {
        warn!(
            "render_frame: {} Shadowcasters exceeds MAX_SHADOW_CASTERS ({}); dropping overflow",
            total_casters, MAX_SHADOW_CASTERS
        );
    }
    data.counts[1] = spot_count as u32;

    // Points: same pattern.
    let point_entities: Vec<crate::ecs::EntityId> =
        world.iter::<PointLight>().map(|(e, _)| e).collect();
    let mut point_count = 0usize;
    let total_points = point_entities.len();
    for entity in point_entities {
        if point_count == MAX_POINT_LIGHTS {
            break;
        }
        let Some(transform) = world.get::<Transform>(entity).copied() else {
            continue;
        };
        let Some(point) = world.get::<PointLight>(entity).copied() else {
            continue;
        };
        data.points[point_count] = PointLightUniformData::new(
            transform.translation,
            point.color,
            point.intensity,
            point.range,
        );
        point_count += 1;
    }
    if total_points > MAX_POINT_LIGHTS {
        warn!(
            "render_frame: {} PointLights exceeds MAX_POINT_LIGHTS ({}); dropping overflow",
            total_points, MAX_POINT_LIGHTS
        );
    }
    data.counts[2] = point_count as u32;

    // counts[3] is overwritten in render_frame from `DebugState`
    // (PCF kernel) — left zero here, but treated as authoritative
    // only after that pass. Keeping the assignment out of this
    // function keeps `build_lights_uniform` independent of debug
    // state.

    // Fog: fold the per-scene atmosphere into the same uniform.
    // Missing resource falls back to `Fog::DEFAULT` (density = 0,
    // shader short-circuits). Co-located with the lights because
    // 1.E.2's god-ray loop reads both inside the spot iteration.
    let fog = world.resource::<Fog>().copied().unwrap_or(Fog::DEFAULT);
    data.set_fog(&fog);

    (data, shadow_vps)
}

pub fn render_frame(world: &mut World) {
    puffin::profile_scope!("render_frame");

    // Snapshot scene data through queries. Block-scoped puffin
    //    spans nest correctly under `render_frame` and let the
    //    profiler attribute time to the right phase.
    let (cam_matrix, camera, cam_pos, lights_uniform, draws, shadowcaster_vps) = {
        puffin::profile_scope!("snapshot");

        let camera_data = world
            .query::<(&Transform, &Camera, &ActiveCamera)>()
            .into_iter()
            .next()
            .map(|(t, c, _)| (t.matrix(), *c, t.translation));
        let Some((cam_matrix, camera, cam_pos)) = camera_data else {
            warn!("render_frame: no ActiveCamera in world; skipping frame");
            return;
        };

        let (mut lights_uniform, shadowcaster_vps) = build_lights_uniform(world);
        // PCF kernel is debug state, threaded through the lights
        // uniform's spare `counts.w` slot rather than its own
        // bind group — the shader reads it inside the spot loop,
        // so co-locating with the rest of the per-frame lighting
        // payload is the cheapest path. Default is `Soft3x3`.
        let pcf_half_kernel = world
            .resource::<DebugState>()
            .map(|d| d.pcf_kernel.half_kernel())
            .unwrap_or_else(|| PcfKernel::Soft3x3.half_kernel());
        lights_uniform.counts[3] = pcf_half_kernel;

        // Two-pass collection: gather entity ids that have a mesh,
        // then resolve `Transform` and the *optional* `Material` per
        // entity. Single-pass chaining of `iter` and `get` fights
        // the borrow checker — the iterator holds a shared borrow
        // of the world while the closure wants its own. Collecting
        // entity ids first ends the iter borrow before the lookups.
        let mesh_entities: Vec<(crate::ecs::EntityId, MeshHandle)> = world
            .iter::<MeshHandle>()
            .map(|(entity, handle)| (entity, *handle))
            .collect();
        let mut draws: Vec<(glam::Mat4, MeshHandle, Material)> = mesh_entities
            .into_iter()
            .filter_map(|(entity, handle)| {
                let transform = world.get::<Transform>(entity)?;
                let material = world
                    .get::<Material>(entity)
                    .copied()
                    .unwrap_or(Material::DEFAULT);
                Some((transform.matrix(), handle, material))
            })
            .collect();
        if draws.len() as u64 > MAX_DRAWS_PER_FRAME {
            warn!(
                "render_frame: {} draws exceeds MAX_DRAWS_PER_FRAME ({}); dropping overflow",
                draws.len(),
                MAX_DRAWS_PER_FRAME
            );
            draws.truncate(MAX_DRAWS_PER_FRAME as usize);
        }

        (
            cam_matrix,
            camera,
            cam_pos,
            lights_uniform,
            draws,
            shadowcaster_vps,
        )
    };

    // Partition the draw list by blend mode in one pass. Opaque draws
    // render first (the depth buffer resolves their order for free);
    // AlphaBlend draws render after, in a separate transparent range,
    // because the over-operator is order-dependent. Both lists index
    // into the same `draws` vec, so each draw's model-buffer slot
    // (= original index) stays stable for the uniform writes and the
    // shadow pass below. The exhaustive `match` makes a future
    // `BlendMode` variant a compile error here rather than a silently
    // dropped draw. 1.G.1 leaves the transparent range in natural
    // order; 1.G.2 sorts it back-to-front by camera distance.
    let mut opaque_draws: Vec<usize> = Vec::with_capacity(draws.len());
    let mut transparent_draws: Vec<usize> = Vec::new();
    for (i, (_, _, material)) in draws.iter().enumerate() {
        match material.blend {
            BlendMode::Opaque => opaque_draws.push(i),
            BlendMode::AlphaBlend => transparent_draws.push(i),
        }
    }

    // Back-to-front sort of the transparent range — the painter's
    // algorithm. The over-operator is order-dependent and depth-write
    // is off, so the depth buffer can't order these for us; we must
    // blend farthest-first. Key is the squared distance from the camera
    // to each draw's world-space origin (the model matrix's translation
    // column) — squaring drops the sqrt and gives the same ordering.
    // `partial_cmp` because f32 isn't `Ord`; an unexpected NaN position
    // sorts as Equal rather than panicking.
    //
    // Sorting by object origin is the standard cheap approximation: it
    // orders *separated* transparent props correctly but can't resolve
    // two interpenetrating transparent meshes, which would need per-
    // triangle sorting or order-independent transparency (out of scope —
    // Game 2A+).
    transparent_draws.sort_by(|&a, &b| {
        let da = (draws[a].0.w_axis.truncate() - cam_pos).length_squared();
        let db = (draws[b].0.w_axis.truncate() - cam_pos).length_squared();
        db.partial_cmp(&da).unwrap_or(std::cmp::Ordering::Equal)
    });

    // Acquire the swap-chain frame and clone device/queue handles.
    //    Device and Queue are refcounted in wgpu 29 — clone is cheap
    //    and lets the rest of the function operate without holding a
    //    `Res`-style borrow on the World. The HDR view is cloned the
    //    same way; the post pipeline's bind group is rebuilt against
    //    it when `hdr_generation` differs from the cached value.
    let (frame, device, queue, surface_size, depth_view, hdr_view, hdr_generation, aspect) = {
        puffin::profile_scope!("acquire");
        let Some(ctx) = world.resource_mut::<RenderContext>() else {
            warn!("render_frame: RenderContext missing");
            return;
        };
        let Some(frame) = ctx.acquire_frame() else {
            return;
        };
        (
            frame,
            ctx.device().clone(),
            ctx.queue().clone(),
            ctx.surface_size(),
            ctx.depth_view().clone(),
            ctx.hdr_view().clone(),
            ctx.hdr_generation(),
            ctx.aspect_ratio(),
        )
    };
    let view_target = frame.texture.create_view(&TextureViewDescriptor::default());

    // Camera uniform — rebuilt every frame from the snapshot.
    let view = cam_matrix.inverse();
    let proj = camera.projection.matrix(aspect);
    let camera_uniform = CameraUniformData::new(view, proj, cam_pos);

    // Record how many shadow-map layers are in active use this
    //    frame. No GPU allocation — the texture is permanent;
    //    `set_active_count` only updates the bookkeeping the
    //    shadow-pass loop reads.
    {
        let Some(maps) = world.resource_mut::<ShadowMaps>() else {
            warn!("render_frame: ShadowMaps missing");
            return;
        };
        maps.set_active_count(shadowcaster_vps.len());
    }

    let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor {
        label: Some("frame-encoder"),
    });

    // Pre-pass writes. All per-frame uniform buffers are written
    //    here, before any render pass begins, so the shadow pass
    //    and the forward pass both observe the same up-to-date
    //    contents. `queue.write_buffer` is queue-scheduled — order
    //    within submit() is preserved.
    {
        puffin::profile_scope!("uniform_writes");
        let Some(pipeline) = world.resource::<ForwardPipeline>() else {
            warn!("render_frame: ForwardPipeline missing");
            return;
        };

        queue.write_buffer(
            &pipeline.camera_buffer,
            0,
            bytemuck::bytes_of(&camera_uniform),
        );
        queue.write_buffer(
            &pipeline.lights_buffer,
            0,
            bytemuck::bytes_of(&lights_uniform),
        );
        for (i, (model, _, material)) in draws.iter().enumerate() {
            let offset = (i as u64) * MODEL_UNIFORM_STRIDE;
            queue.write_buffer(
                &pipeline.model_buffer,
                offset,
                bytemuck::bytes_of(&ModelUniformData::new(*model, material)),
            );
        }

        let Some(shadow) = world.resource::<ShadowPipeline>() else {
            warn!("render_frame: ShadowPipeline missing");
            return;
        };
        for (i, vp) in shadowcaster_vps.iter().enumerate() {
            let offset = (i as u64) * SHADOW_VP_UNIFORM_STRIDE;
            // Pack as the same `[[f32; 4]; 4]` shape the forward
            // pipeline's CameraUniformData uses — bytemuck reads
            // the same 64 B regardless.
            let cols: [[f32; 4]; 4] = vp.to_cols_array_2d();
            queue.write_buffer(&shadow.vp_buffer, offset, bytemuck::bytes_of(&cols));
        }
    }

    // Shadow passes — one depth-only pass per shadowcaster,
    //    each rendering the entire draw list into the caster's own
    //    shadow map. Re-traversing the draw list here is cheap at
    //    indoor scale; instancing or bulk submission lands when
    //    profiling demands it.
    if !shadowcaster_vps.is_empty() {
        puffin::profile_scope!("shadow_pass");
        let Some(shadow) = world.resource::<ShadowPipeline>() else {
            warn!("render_frame: ShadowPipeline missing");
            return;
        };
        let Some(maps) = world.resource::<ShadowMaps>() else {
            warn!("render_frame: ShadowMaps missing");
            return;
        };
        let Some(meshes) = world.resource::<MeshRegistry>() else {
            warn!("render_frame: MeshRegistry missing");
            return;
        };

        let caster_count = shadowcaster_vps.len().min(maps.active_count());
        for i in 0..caster_count {
            let Some(map_view) = maps.layer_view(i) else {
                continue;
            };
            let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                label: Some("shadow-pass"),
                // Depth-only: no color attachments, the shadow
                // shader has no fragment.
                color_attachments: &[],
                depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                    view: map_view,
                    depth_ops: Some(Operations {
                        load: LoadOp::Clear(1.0),
                        store: StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                multiview_mask: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            pass.set_pipeline(&shadow.pipeline);
            let vp_offset = (i as u32) * (SHADOW_VP_UNIFORM_STRIDE as u32);
            pass.set_bind_group(0, &shadow.vp_bind_group, &[vp_offset]);

            // Only opaque draws occlude. Decals are flush stickers and
            // the frosted pane can't throw a partial shadow into a
            // binary depth map, so AlphaBlend draws are skipped here.
            for &di in &opaque_draws {
                let (_, handle, _) = &draws[di];
                let Some(mesh) = meshes.get(*handle) else {
                    continue;
                };
                let model_offset = (di as u32) * (MODEL_UNIFORM_STRIDE as u32);
                pass.set_bind_group(1, &shadow.model_bind_group, &[model_offset]);
                pass.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
                pass.set_index_buffer(mesh.index_buffer.slice(..), IndexFormat::Uint32);
                pass.draw_indexed(0..mesh.index_count, 0, 0..1);
            }
        }
    }

    // Forward pass.
    {
        puffin::profile_scope!("forward_pass");

        // Pre-pass — populate the per-material bind-group cache for
        // every unique texture handle this frame's draws need. Done
        // outside the render pass because cache writes take a
        // mutable borrow on `ForwardPipeline` while the pass below
        // holds a shared borrow. The two-scope shape keeps the
        // `TextureRegistry` (shared) and `ForwardPipeline` (mutable)
        // borrows strictly disjoint. `TextureView` is internally an
        // `Arc`, so cloning it across the borrow boundary is cheap.
        {
            let mut unique_handles = std::collections::HashSet::new();
            // WHITE is the universal fallback — always cached so a
            // draw that references a missing texture still binds
            // something valid.
            unique_handles.insert(TextureHandle::WHITE);
            for (_, _, material) in &draws {
                unique_handles
                    .insert(material.albedo_texture.unwrap_or(TextureHandle::WHITE));
            }

            let views: Vec<(TextureHandle, wgpu::TextureView)> = {
                let Some(textures) = world.resource::<TextureRegistry>() else {
                    warn!("render_frame: TextureRegistry missing");
                    return;
                };
                unique_handles
                    .iter()
                    .filter_map(|&h| {
                        // Handles absent from the registry resolve to
                        // WHITE; the cache still ends up with an entry
                        // under the requested key, pointing at the
                        // WHITE view.
                        let actual = if textures.contains(h) {
                            h
                        } else {
                            TextureHandle::WHITE
                        };
                        textures.get(actual).map(|tex| (h, tex.view.clone()))
                    })
                    .collect()
            };

            let Some(pipeline) = world.resource_mut::<ForwardPipeline>() else {
                warn!("render_frame: ForwardPipeline missing");
                return;
            };
            for (handle, view) in &views {
                pipeline.ensure_material_bind_group_with_view(&device, *handle, view);
            }
        }

        let Some(pipeline) = world.resource::<ForwardPipeline>() else {
            warn!("render_frame: ForwardPipeline missing");
            return;
        };
        let Some(meshes) = world.resource::<MeshRegistry>() else {
            warn!("render_frame: MeshRegistry missing");
            return;
        };
        let Some(shadow_maps) = world.resource::<ShadowMaps>() else {
            warn!("render_frame: ShadowMaps missing");
            return;
        };

        let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("forward-pass"),
            // Forward writes into the HDR offscreen target; post will
            // sample it and write the swap chain.
            color_attachments: &[Some(RenderPassColorAttachment {
                view: &hdr_view,
                depth_slice: None,
                resolve_target: None,
                ops: Operations {
                    load: LoadOp::Clear(CLEAR_COLOR),
                    store: StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                view: &depth_view,
                depth_ops: Some(Operations {
                    load: LoadOp::Clear(1.0),
                    store: StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            multiview_mask: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });

        pass.set_pipeline(&pipeline.pipeline);
        pass.set_bind_group(0, &pipeline.camera_bind_group, &[]);
        pass.set_bind_group(1, &pipeline.lights_bind_group, &[]);
        pass.set_bind_group(3, shadow_maps.bind_group(), &[]);

        // Per-draw record shared by both ranges. Bind groups 0/1/3 stay
        // set across the opaque→transparent pipeline switch because the
        // two pipelines share one layout; only group 2 (per-draw
        // dynamic offset) and group 4 (per-material texture) rebind.
        let draw_one = |pass: &mut wgpu::RenderPass, i: usize| {
            let (_, handle, material) = &draws[i];
            let Some(mesh) = meshes.get(*handle) else {
                warn!("render_frame: missing mesh for handle {handle:?}; skipping draw");
                return;
            };
            let dyn_offset = (i as u32) * (MODEL_UNIFORM_STRIDE as u32);
            pass.set_bind_group(2, &pipeline.model_bind_group, &[dyn_offset]);

            // The pre-pass guarantees an entry exists for every
            // handle the draw list needs, including the WHITE
            // fallback for `None` and unknown handles. A `None`
            // here would indicate a logic error above; the warn
            // surfaces it without crashing the frame.
            let texture_handle = material.albedo_texture.unwrap_or(TextureHandle::WHITE);
            let Some(material_bg) = pipeline.material_bind_group(texture_handle) else {
                warn!(
                    "render_frame: material bind group missing for {:?}; skipping draw",
                    texture_handle
                );
                return;
            };
            pass.set_bind_group(4, material_bg, &[]);

            pass.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
            pass.set_index_buffer(mesh.index_buffer.slice(..), IndexFormat::Uint32);
            pass.draw_indexed(0..mesh.index_count, 0, 0..1);
        };

        // Opaque range first — written into a fresh depth buffer.
        for &i in &opaque_draws {
            draw_one(&mut pass, i);
        }

        // Transparent range: switch to the alpha-blend pipeline (depth
        // test on, depth write off, no cull) and draw. The over-operator
        // is order-dependent, so 1.G.2 will sort this range back-to-
        // front; 1.G.1 draws it in natural order (correct for a single
        // transparent surface).
        if !transparent_draws.is_empty() {
            pass.set_pipeline(&pipeline.transparent_pipeline);
            for &i in &transparent_draws {
                draw_one(&mut pass, i);
            }
        }
    }

    // Post-process pass — single fullscreen triangle samples the
    // HDR target and writes the swap chain. ACES tonemap + color
    // grade live in the fragment shader; vignette and overlay stack
    // in later 1.D Steps without changing the bind-group story.
    {
        puffin::profile_scope!("post_pass");

        // Pack per-scene grade + vignette + overlay. Missing any
        // resource falls back to its identity (no-op) — the renderer
        // should never wedge on an unconfigured world.
        let grade = world
            .resource::<ColorGrade>()
            .copied()
            .unwrap_or(ColorGrade::DEFAULT);
        let vignette = world
            .resource::<Vignette>()
            .copied()
            .unwrap_or(Vignette::DEFAULT);
        let overlay = world
            .resource::<PostOverlay>()
            .copied()
            .unwrap_or(PostOverlay::DEFAULT);
        let params = PostParamsUniform::pack(&grade, &vignette, &overlay);

        // Resolve the overlay texture to a view under a shared borrow,
        // then drop it before taking the mutable PostPipeline borrow —
        // same disjoint-borrow shape as the material pre-pass. An
        // absent or missing handle falls back to WHITE; the shader's
        // overlay term is gated by `intensity = 0` anyway, so the bound
        // texture is irrelevant when the overlay is off. `TextureView`
        // is `Arc`-backed, so the clone is cheap.
        let overlay_handle = overlay.texture.unwrap_or(TextureHandle::WHITE);
        let (overlay_handle, overlay_view) = {
            let Some(textures) = world.resource::<TextureRegistry>() else {
                warn!("render_frame: TextureRegistry missing");
                return;
            };
            let actual = if textures.contains(overlay_handle) {
                overlay_handle
            } else {
                TextureHandle::WHITE
            };
            match textures.get(actual) {
                Some(tex) => (actual, tex.view.clone()),
                None => {
                    warn!("render_frame: overlay texture + WHITE both missing");
                    return;
                }
            }
        };

        let Some(post) = world.resource_mut::<PostPipeline>() else {
            warn!("render_frame: PostPipeline missing");
            return;
        };
        // Rebuilds only on first frame and after each resize; pointer
        // compare otherwise. See PostPipeline::ensure_bind_group.
        let bind_group = post
            .ensure_bind_group(&device, &hdr_view, hdr_generation)
            .clone();
        // Overlay group rebuilds only when the active handle changes.
        let overlay_bind_group = post
            .ensure_overlay_bind_group(&device, overlay_handle, &overlay_view)
            .clone();
        queue.write_buffer(&post.params_buffer, 0, bytemuck::bytes_of(&params));

        let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("post-pass"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: &view_target,
                depth_slice: None,
                resolve_target: None,
                ops: Operations {
                    // Post writes every pixel — Clear is cheaper than
                    // Load on tiled GPUs (the load wouldn't read
                    // anything useful) and behaviorally identical.
                    load: LoadOp::Clear(wgpu::Color::BLACK),
                    store: StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            multiview_mask: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });

        pass.set_pipeline(&post.pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.set_bind_group(1, &post.params_bind_group, &[]);
        pass.set_bind_group(2, &overlay_bind_group, &[]);
        // Three verts, one instance — fullscreen triangle covers the
        // whole viewport; positions and UVs come from `vertex_index`.
        pass.draw(0..3, 0..1);
    }

    // Egui overlay pass — load (don't clear) the post result.
    // Drawing egui *after* post keeps the debug UI uncoloured by
    // the gameplay grade: the overlay is a tool, not part of the
    // look.
    //
    // We always run the egui frame (even when hidden) so its
    // input queue stays drained; we only skip the encoded pass
    // when the overlay is hidden, so the GPU work disappears
    // without orphaning input state.
    {
        puffin::profile_scope!("overlay");

        // Update FPS / frame-ms ring buffer from the latest delta.
        // The Update-stage system already ran this frame, so
        // delta_secs reflects the variable-tick delta.
        let delta_secs = world
            .resource::<Time>()
            .map(|t| t.delta_secs)
            .unwrap_or(0.0);
        if let Some(debug) = world.resource_mut::<DebugState>() {
            debug.frame_stats.push(delta_secs);
        }

        // Snapshot the data the UI reads, *and* the interactive bits
        // the UI may flip, before the overlay's mutable borrow opens.
        // After the overlay borrow ends we write any changed bits
        // back into DebugState. Two resource lookups per frame on
        // DebugState — one read, one write — beats moving the
        // overlay in and out of the resource map every frame.
        let (visible, fps, frame_ms, mut interact) = world
            .resource::<DebugState>()
            .map(|d| {
                let (fps, ms) = d.frame_stats.averaged();
                (
                    d.overlay_visible,
                    fps,
                    ms,
                    OverlayInteract {
                        show_profiler: d.show_profiler,
                    },
                )
            })
            .unwrap_or((
                false,
                0.0,
                0.0,
                OverlayInteract {
                    show_profiler: false,
                },
            ));
        let entity_count = world.entity_count();
        let metrics = OverlayMetrics {
            fps,
            frame_ms,
            entity_count,
            camera_pos: cam_pos,
        };

        // Refresh the profiler snapshot (no-op unless the refresh
        // interval has elapsed) and clone the Arc out so the
        // ProfilerView borrow ends before we touch the overlay.
        // Skip the refresh entirely when the panel is hidden — no
        // point spending the merge work on data the user can't see.
        let profiler_snapshot = if interact.show_profiler {
            world.resource_mut::<ProfilerView>().map(|p| {
                p.refresh();
                p.snapshot()
            })
        } else {
            None
        };

        if let Some(overlay) = world.resource_mut::<DebugOverlay>() {
            {
                puffin::profile_scope!("overlay_build");
                overlay.run(|ctx| {
                    build_overlay_ui(ctx, &mut interact, metrics, profiler_snapshot.as_deref());
                });
            }

            if visible {
                puffin::profile_scope!("overlay_render");
                let pixels_per_point = overlay.context().pixels_per_point();
                overlay.render(
                    &device,
                    &queue,
                    &mut encoder,
                    &view_target,
                    [surface_size.0, surface_size.1],
                    pixels_per_point,
                );
            }
        }

        // Write back any UI-flipped bits.
        if let Some(debug) = world.resource_mut::<DebugState>() {
            debug.show_profiler = interact.show_profiler;
        }
    }

    // Submit + present.
    {
        puffin::profile_scope!("submit_present");
        queue.submit(Some(encoder.finish()));
        frame.present();
    }
}
