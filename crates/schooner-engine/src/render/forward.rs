//! `render_frame` — the system that paints the frame.
//!
//! Frame flow (also documented in `architecture/render.md`):
//!
//! 1. Resolve the active camera. If none exists, skip the frame.
//! 2. Snapshot the draw list, lights, and shadowcaster view-proj
//!    matrices through ECS queries.
//! 3. Acquire a swap-chain texture; on `Lost`/`Outdated` reconfigure
//!    and skip.
//! 4. Reallocate the shadow-map set if the caster count changed.
//! 5. Write camera, lights, model, and shadow-VP uniforms before any
//!    pass begins — the shadow and forward passes share the model
//!    buffer through separate bind groups, so a single write feeds
//!    both.
//! 6. For each shadowcaster: depth-only render pass into its shadow
//!    map, rendering the entire draw list from that light's POV.
//! 7. Forward pass — same draw list, lit + (in 1.C.4) shadow-sampled.
//! 8. Build the egui frame and encode the overlay pass on top.
//! 9. Submit + present.
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
    build_overlay_ui, DebugState, OverlayInteract, OverlayMetrics, PcfKernel, ProfilerView,
};
use crate::ecs::World;
use crate::material::Material;
use crate::render::context::RenderContext;
use crate::render::light::{DirectionalLight, PointLight, Shadowcaster, SpotLight};
use crate::render::mesh::MeshHandle;
use crate::render::overlay::DebugOverlay;
use crate::render::pipeline::{ForwardPipeline, MAX_DRAWS_PER_FRAME, MODEL_UNIFORM_STRIDE};
use crate::render::registry::MeshRegistry;
use crate::render::shadow::{
    compute_shadow_vp, ShadowMaps, ShadowPipeline, MAX_SHADOW_CASTERS, SHADOW_VP_UNIFORM_STRIDE,
};
use crate::render::uniforms::{
    CameraUniformData, DirectionalLightUniformData, LightsUniformData, MAX_POINT_LIGHTS,
    MAX_SPOT_LIGHTS, ModelUniformData, PointLightUniformData, SpotLightUniformData,
};
use crate::time::Time;
use crate::transform::Transform;

/// Background color when the swap chain clears each frame. Mid-grey
/// rather than pure black so a scene with no draws is visibly
/// "rendering nothing" instead of "renderer crashed."
const CLEAR_COLOR: wgpu::Color = wgpu::Color {
    r: 0.05,
    g: 0.06,
    b: 0.08,
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

    (data, shadow_vps)
}

pub fn render_frame(world: &mut World) {
    puffin::profile_scope!("render_frame");

    // 1. Snapshot scene data through queries. Block-scoped puffin
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

    // 2. Acquire the swap-chain frame and clone device/queue handles.
    //    Device and Queue are refcounted in wgpu 29 — clone is cheap
    //    and lets the rest of the function operate without holding a
    //    `Res`-style borrow on the World.
    let (frame, device, queue, surface_size, depth_view, aspect) = {
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
            ctx.aspect_ratio(),
        )
    };
    let view_target = frame.texture.create_view(&TextureViewDescriptor::default());

    // 3. Camera uniform — rebuilt every frame from the snapshot.
    let view = cam_matrix.inverse();
    let proj = camera.projection.matrix(aspect);
    let camera_uniform = CameraUniformData::new(view, proj, cam_pos);

    // 4. Record how many shadow-map layers are in active use this
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

    // 5. Pre-pass writes. All per-frame uniform buffers are written
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

    // 6. Shadow passes — one depth-only pass per shadowcaster,
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

            for (di, (_, handle, _)) in draws.iter().enumerate() {
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

    // 7. Forward pass.
    {
        puffin::profile_scope!("forward_pass");
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
            color_attachments: &[Some(RenderPassColorAttachment {
                view: &view_target,
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

        for (i, (_, handle, _)) in draws.iter().enumerate() {
            let Some(mesh) = meshes.get(*handle) else {
                warn!("render_frame: missing mesh for handle {handle:?}; skipping draw");
                continue;
            };
            let dyn_offset = (i as u32) * (MODEL_UNIFORM_STRIDE as u32);
            pass.set_bind_group(2, &pipeline.model_bind_group, &[dyn_offset]);
            pass.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
            pass.set_index_buffer(mesh.index_buffer.slice(..), IndexFormat::Uint32);
            pass.draw_indexed(0..mesh.index_count, 0, 0..1);
        }
    }

    // 8. Egui overlay pass — load (don't clear) the forward result.
    //
    //    We always run the egui frame (even when hidden) so its
    //    input queue stays drained; we only skip the encoded pass
    //    when the overlay is hidden, so the GPU work disappears
    //    without orphaning input state.
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

    // 9. Submit + present.
    {
        puffin::profile_scope!("submit_present");
        queue.submit(Some(encoder.finish()));
        frame.present();
    }
}
