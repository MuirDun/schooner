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
//! 5. Submit + present.
//!
//! The system declares its access through the normal ECS contract:
//! `ResMut<RenderContext>` (acquire/configure surface),
//! `ResMut<ForwardPipeline>` (write uniforms),
//! `Res<MeshRegistry>` (lookup vertex/index buffers),
//! and three queries (renderables, the active camera, lights).

use glam::Vec3;
use log::warn;
use wgpu::{
    CommandEncoderDescriptor, IndexFormat, LoadOp, Operations, RenderPassColorAttachment,
    RenderPassDepthStencilAttachment, RenderPassDescriptor, StoreOp, TextureViewDescriptor,
};

use crate::camera::{ActiveCamera, Camera};
use crate::ecs::{Query, Res, ResMut};
use crate::render::context::RenderContext;
use crate::render::light::DirectionalLight;
use crate::render::mesh::MeshHandle;
use crate::render::pipeline::{ForwardPipeline, MAX_DRAWS_PER_FRAME, MODEL_UNIFORM_STRIDE};
use crate::render::registry::MeshRegistry;
use crate::render::uniforms::{CameraUniformData, LightUniformData, ModelUniformData};
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

pub fn render_frame(
    mut ctx: ResMut<RenderContext>,
    pipeline: ResMut<ForwardPipeline>,
    meshes: Res<MeshRegistry>,
    renderables: Query<(&Transform, &MeshHandle)>,
    cameras: Query<(&Transform, &Camera, &ActiveCamera)>,
    lights: Query<&DirectionalLight>,
) {
    // 1. Resolve active camera. First match wins; without a camera
    //    the scene has no view, so the right behavior is "skip the
    //    frame, log once" rather than "draw garbage."
    let Some((cam_transform, camera, _)) = cameras.into_iter().next() else {
        warn!("render_frame: no ActiveCamera in world; skipping frame");
        return;
    };

    let aspect = ctx.aspect_ratio();
    let view = cam_transform.matrix().inverse();
    let proj = camera.projection.matrix(aspect);
    let camera_uniform = CameraUniformData::new(view, proj, cam_transform.translation);
    ctx.queue().write_buffer(
        &pipeline.camera_buffer,
        0,
        bytemuck::bytes_of(&camera_uniform),
    );

    // 2. Light. First DirectionalLight wins; placeholder otherwise.
    let light_uniform = match lights.into_iter().next() {
        Some(light) => LightUniformData::new(light.direction, light.color, light.ambient),
        None => LightUniformData::new(
            Vec3::new(0.0, -1.0, 0.0),
            Vec3::ZERO,
            Vec3::splat(0.3),
        ),
    };
    ctx.queue().write_buffer(
        &pipeline.light_buffer,
        0,
        bytemuck::bytes_of(&light_uniform),
    );

    // 3. Acquire frame.
    let Some(frame) = ctx.acquire_frame() else {
        return;
    };
    let view = frame.texture.create_view(&TextureViewDescriptor::default());

    // 4. Per-draw model uniforms.
    //
    // Collect into a Vec so we can bound by MAX_DRAWS_PER_FRAME
    // and write uniforms before the borrow on the encoder begins.
    let mut draws: Vec<(glam::Mat4, MeshHandle)> = renderables
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
    for (i, (model, _)) in draws.iter().enumerate() {
        let offset = (i as u64) * MODEL_UNIFORM_STRIDE;
        ctx.queue().write_buffer(
            &pipeline.model_buffer,
            offset,
            bytemuck::bytes_of(&ModelUniformData::from_matrix(*model)),
        );
    }

    let mut encoder = ctx.device().create_command_encoder(&CommandEncoderDescriptor {
        label: Some("forward-encoder"),
    });

    {
        let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("forward-pass"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: &view,
                resolve_target: None,
                ops: Operations {
                    load: LoadOp::Clear(CLEAR_COLOR),
                    store: StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                view: ctx.depth_view(),
                depth_ops: Some(Operations {
                    load: LoadOp::Clear(1.0),
                    store: StoreOp::Store,
                }),
                stencil_ops: None,
            }),
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

    let cmd = encoder.finish();
    ctx.queue().submit(Some(cmd));
    frame.present();
}
