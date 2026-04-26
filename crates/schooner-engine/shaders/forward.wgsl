// Forward pipeline — Blinn–Phong with one directional light.
//
// Bind groups:
//   @group(0) — Camera (view, proj, view_proj, position).
//   @group(1) — Light  (direction, color, ambient).
//   @group(2) — Model  (per-draw model matrix, dynamic-offset).
//
// Vertex layout matches `render::mesh::Vertex`: pos (3f) at
// location 0, normal (3f) at location 1, uv (2f) at location 2.
// uv is unused in Game 0 — first textured material lands later.

struct CameraUniform {
    view: mat4x4<f32>,
    proj: mat4x4<f32>,
    view_proj: mat4x4<f32>,
    // vec3 pads to 16 in std140; use a vec4 to keep the WGSL and
    // Rust layouts in lockstep without manual padding here.
    position: vec4<f32>,
};

struct LightUniform {
    // .xyz = direction the light travels; .w = unused padding.
    direction: vec4<f32>,
    color: vec4<f32>,
    ambient: vec4<f32>,
};

struct ModelUniform {
    model: mat4x4<f32>,
};

@group(0) @binding(0) var<uniform> camera: CameraUniform;
@group(1) @binding(0) var<uniform> light: LightUniform;
@group(2) @binding(0) var<uniform> model: ModelUniform;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) world_position: vec3<f32>,
    @location(1) world_normal: vec3<f32>,
};

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    let world_pos4 = model.model * vec4<f32>(in.position, 1.0);
    var out: VertexOutput;
    out.clip_position = camera.view_proj * world_pos4;
    out.world_position = world_pos4.xyz;
    // Game 0 only uses uniform scale on built-ins, so the upper
    // 3x3 of the model matrix is a valid normal transform. When
    // non-uniform scale enters, switch to the inverse-transpose.
    let normal_mat = mat3x3<f32>(
        model.model[0].xyz,
        model.model[1].xyz,
        model.model[2].xyz,
    );
    out.world_normal = normalize(normal_mat * in.normal);
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // Light direction is "where the light travels"; the L vector
    // in the lighting math is "from surface to light", so negate.
    let l = normalize(-light.direction.xyz);
    let n = normalize(in.world_normal);
    let v = normalize(camera.position.xyz - in.world_position);
    let h = normalize(l + v);

    let n_dot_l = max(dot(n, l), 0.0);
    let n_dot_h = max(dot(n, h), 0.0);

    // Hard-coded base color and shininess — Game 0 has no
    // material system. White diffuse, modest specular, exponent
    // tuned to give visible highlights on the cube without
    // over-tightening on the plane.
    let base = vec3<f32>(0.8, 0.8, 0.8);
    let specular_strength = 0.3;
    let shininess = 32.0;

    let diffuse = base * n_dot_l * light.color.rgb;
    let specular = vec3<f32>(specular_strength) * pow(n_dot_h, shininess) * light.color.rgb;
    let ambient = base * light.ambient.rgb;

    return vec4<f32>(ambient + diffuse + specular, 1.0);
}
