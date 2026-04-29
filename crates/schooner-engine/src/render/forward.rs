//! `render_frame` — the system that paints the frame.
//!
//! Frame flow (also documented in `architecture/render.md`):
//!
//! 1. Resolve the active camera. If none exists, skip the frame.
//! 2. Build and upload the camera + light uniforms.
//! 3. Acquire a swap-chain texture; on `Lost`/`Outdated` reconfigure
//!    and skip.
//! 4. For each `(Transform, MeshHandle)`: write its model matrix
//!    into the per-draw uniform buffer at slot `i`'s offset, bind
//!    the model group with that dynamic offset, draw indexed.
//! 5. Build the egui frame and encode the overlay pass on top.
//! 6. Submit + present.
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

use glam::Vec3;
use log::warn;
use wgpu::{
    CommandEncoderDescriptor, IndexFormat, LoadOp, Operations, RenderPassColorAttachment,
    RenderPassDepthStencilAttachment, RenderPassDescriptor, StoreOp, TextureViewDescriptor,
};

use crate::camera::{ActiveCamera, Camera};
use crate::debug::{
    build_overlay_ui, DebugState, OverlayInteract, OverlayMetrics, ProfilerView,
};
use crate::ecs::World;
use crate::render::context::RenderContext;
use crate::render::light::DirectionalLight;
use crate::render::mesh::MeshHandle;
use crate::render::overlay::DebugOverlay;
use crate::render::pipeline::{ForwardPipeline, MAX_DRAWS_PER_FRAME, MODEL_UNIFORM_STRIDE};
use crate::render::registry::MeshRegistry;
use crate::render::uniforms::{CameraUniformData, LightUniformData, ModelUniformData};
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

pub fn render_frame(world: &mut World) {
    puffin::profile_scope!("render_frame");

    // 1. Snapshot scene data through queries. Block-scoped puffin
    //    spans nest correctly under `render_frame` and let the
    //    profiler attribute time to the right phase.
    let (cam_matrix, camera, cam_pos, light_uniform, draws) = {
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

        let light_uniform = world
            .query::<&DirectionalLight>()
            .into_iter()
            .next()
            .map(|l| LightUniformData::new(l.direction, l.color, l.ambient))
            .unwrap_or_else(|| {
                LightUniformData::new(Vec3::new(0.0, -1.0, 0.0), Vec3::ZERO, Vec3::splat(0.3))
            });

        let mut draws: Vec<(glam::Mat4, MeshHandle)> = world
            .query::<(&Transform, &MeshHandle)>()
            .into_iter()
            .map(|(t, h)| (t.matrix(), *h))
            .collect();
        if draws.len() as u64 > MAX_DRAWS_PER_FRAME {
            warn!(
                "render_frame: {} draws exceeds MAX_DRAWS_PER_FRAME ({}); dropping overflow",
                draws.len(),
                MAX_DRAWS_PER_FRAME
            );
            draws.truncate(MAX_DRAWS_PER_FRAME as usize);
        }

        (cam_matrix, camera, cam_pos, light_uniform, draws)
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

    let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor {
        label: Some("frame-encoder"),
    });

    // 4. Forward pass.
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

        queue.write_buffer(
            &pipeline.camera_buffer,
            0,
            bytemuck::bytes_of(&camera_uniform),
        );
        queue.write_buffer(
            &pipeline.light_buffer,
            0,
            bytemuck::bytes_of(&light_uniform),
        );
        for (i, (model, _)) in draws.iter().enumerate() {
            let offset = (i as u64) * MODEL_UNIFORM_STRIDE;
            queue.write_buffer(
                &pipeline.model_buffer,
                offset,
                bytemuck::bytes_of(&ModelUniformData::from_matrix(*model)),
            );
        }

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
        pass.set_bind_group(1, &pipeline.light_bind_group, &[]);

        for (i, (_, handle)) in draws.iter().enumerate() {
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

    // 5. Egui overlay pass — load (don't clear) the forward result.
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
        let delta_secs = world.resource::<Time>().map(|t| t.delta_secs).unwrap_or(0.0);
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
                    build_overlay_ui(
                        ctx,
                        &mut interact,
                        metrics,
                        profiler_snapshot.as_deref(),
                    );
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

    // 6. Submit + present.
    {
        puffin::profile_scope!("submit_present");
        queue.submit(Some(encoder.finish()));
        frame.present();
    }
}
