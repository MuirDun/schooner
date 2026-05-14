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
    // .xyz = albedo (linear), .w = roughness ∈ [0, 1].
    albedo_roughness: vec4<f32>,
    // .xyz = emissive color (linear), .w = emissive intensity.
    emissive: vec4<f32>,
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

    let albedo = model.albedo_roughness.xyz;
    let roughness = model.albedo_roughness.w;

    // Map roughness ∈ [0, 1] to Blinn–Phong shininess. Calibrated
    // so roughness = 0.5 yields shininess ≈ 32 — matching the
    // Game 0 hardcoded value, so a Material::DEFAULT surface reads
    // the same as the pre-Material baseline. Lower roughness
    // tightens the highlight (mirror-ish at 0); higher broadens
    // it (near-Lambertian at 1).
    let shininess = pow(2.0, mix(0.0, 10.0, 1.0 - roughness));

    // Specular stays white (dielectric approximation). Not tinted
    // by albedo — that's the metallic case, which the architecture
    // vision intentionally does not pursue.
    let specular_strength = 0.3;

    let diffuse = albedo * n_dot_l * light.color.rgb;
    let specular = vec3<f32>(specular_strength) * pow(n_dot_h, shininess) * light.color.rgb;
    let ambient = albedo * light.ambient.rgb;

    // Emissive is the surface's own light — added outside the lit
    // term so it survives shadow and ambient is irrelevant.
    let emissive = model.emissive.xyz * model.emissive.w;

    return vec4<f32>(ambient + diffuse + specular + emissive, 1.0);
}
