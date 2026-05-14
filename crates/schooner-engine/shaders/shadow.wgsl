// Depth-only shadow pass.
//
// Vertex-only pipeline that writes per-fragment depth into a
// shadow map, sampled by the forward pass to occlude spot-light
// contribution. No fragment shader — wgpu accepts a pipeline
// with `fragment: None` for depth-only output.
//
// Bind groups:
//   @group(0) — Shadow view-projection (single mat4 per caster).
//   @group(1) — Model (per-draw model matrix, dynamic-offset,
//               same buffer as the forward pipeline's group 2).
//
// The model bind group's full uniform is the same 96 B
// `ModelUniformData` the forward shader reads. We only need the
// `.model` matrix here, but the struct layout must match the
// underlying buffer so the dynamic-offset bind validates.

struct ShadowViewProj {
    view_proj: mat4x4<f32>,
};

struct ModelUniform {
    model: mat4x4<f32>,
    // Padding to match the forward pipeline's ModelUniformData
    // layout — the bind group's min_binding_size covers the full
    // 96 B even though we ignore material params here.
    albedo_roughness: vec4<f32>,
    emissive: vec4<f32>,
};

@group(0) @binding(0) var<uniform> shadow: ShadowViewProj;
@group(1) @binding(0) var<uniform> model: ModelUniform;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
};

@vertex
fn vs_main(in: VertexInput) -> @builtin(position) vec4<f32> {
    return shadow.view_proj * model.model * vec4<f32>(in.position, 1.0);
}
