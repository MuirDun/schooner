// Forward pipeline — Blinn–Phong with one directional + N spot +
// N point lights.
//
// Bind groups:
//   @group(0) — Camera  (view, proj, view_proj, position).
//   @group(1) — Lights  (directional + spot/point arrays + counts).
//   @group(2) — Model   (per-draw model matrix + material params,
//                        dynamic-offset).
//
// Vertex layout matches `render::mesh::Vertex`: pos (3f) at
// location 0, normal (3f) at location 1, uv (2f) at location 2.
// uv is unused for now — first textured material lands in Phase 1.F.

struct CameraUniform {
    view: mat4x4<f32>,
    proj: mat4x4<f32>,
    view_proj: mat4x4<f32>,
    // vec3 pads to 16 in std140; use a vec4 to keep the WGSL and
    // Rust layouts in lockstep without manual padding here.
    position: vec4<f32>,
};

struct DirectionalLight {
    // .xyz = direction the light travels; .w = unused padding.
    direction: vec4<f32>,
    // .xyz = color (unit tint), .w = intensity.
    color_intensity: vec4<f32>,
    // .xyz = ambient scene tint, .w = unused padding.
    ambient: vec4<f32>,
};

struct SpotLight {
    // .xyz = world position, .w = intensity.
    position_intensity: vec4<f32>,
    // .xyz = world-space direction (unit), .w = range.
    direction_range: vec4<f32>,
    // .xyz = color (unit tint), .w = inner cone cosine.
    color_inner_cos: vec4<f32>,
    // .x = outer cone cosine, .yzw padding.
    outer_cos_pad: vec4<f32>,
};

struct PointLight {
    // .xyz = world position, .w = range.
    position_range: vec4<f32>,
    // .xyz = color (unit tint), .w = intensity.
    color_intensity: vec4<f32>,
};

// Array sizes must match MAX_SPOT_LIGHTS / MAX_POINT_LIGHTS in
// `render/uniforms.rs`. If you change one, change both.
struct LightsUniform {
    directional: DirectionalLight,
    spots: array<SpotLight, 8>,
    points: array<PointLight, 16>,
    // .x = directional count (0 or 1), .y = spot count, .z = point
    // count, .w = padding.
    counts: vec4<u32>,
};

struct ModelUniform {
    model: mat4x4<f32>,
    // .xyz = albedo (linear), .w = roughness ∈ [0, 1].
    albedo_roughness: vec4<f32>,
    // .xyz = emissive color (linear), .w = emissive intensity.
    emissive: vec4<f32>,
};

@group(0) @binding(0) var<uniform> camera: CameraUniform;
@group(1) @binding(0) var<uniform> lights: LightsUniform;
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

// One light's Blinn–Phong contribution. `l` is the surface-to-light
// vector (unit), `v` is the surface-to-camera vector (unit),
// `radiance` is the light's color × intensity × attenuation already
// folded in. Specular stays white per the dielectric convention.
fn blinn_phong_contribution(
    n: vec3<f32>,
    l: vec3<f32>,
    v: vec3<f32>,
    albedo: vec3<f32>,
    specular_strength: f32,
    shininess: f32,
    radiance: vec3<f32>,
) -> vec3<f32> {
    let h = normalize(l + v);
    let n_dot_l = max(dot(n, l), 0.0);
    let n_dot_h = max(dot(n, h), 0.0);
    let diffuse = albedo * n_dot_l;
    let specular = vec3<f32>(specular_strength) * pow(n_dot_h, shininess);
    return (diffuse + specular) * radiance;
}

// Windowed inverse-square range attenuation (UE4/Frostbite shape).
// `1 / (1 + d²)` is the physically motivated falloff with a guard
// against the d→0 singularity at the lamp itself; the windowed
// `(1 - (d/range)⁴)²` term tapers smoothly to zero at the cutoff.
fn range_attenuation(dist: f32, range: f32) -> f32 {
    let d_over_r = dist / range;
    let d4 = d_over_r * d_over_r * d_over_r * d_over_r;
    let window = max(1.0 - d4, 0.0);
    return window * window / (1.0 + dist * dist);
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let n = normalize(in.world_normal);
    let v = normalize(camera.position.xyz - in.world_position);

    let albedo = model.albedo_roughness.xyz;
    let roughness = model.albedo_roughness.w;

    // Map roughness ∈ [0, 1] to Blinn–Phong shininess. Calibrated
    // so roughness = 0.5 yields shininess ≈ 32 — matching the
    // Game 0 hardcoded value, so a Material::DEFAULT surface reads
    // the same as the pre-Material baseline.
    let shininess = pow(2.0, mix(0.0, 10.0, 1.0 - roughness));

    let specular_strength = 0.3;

    var lit = vec3<f32>(0.0);

    // Directional contribution. Skipped when counts.x == 0 so the
    // shader doesn't add a phantom sun from stale placeholder data.
    if (lights.counts.x > 0u) {
        let l = normalize(-lights.directional.direction.xyz);
        let radiance = lights.directional.color_intensity.xyz
                     * lights.directional.color_intensity.w;
        lit += blinn_phong_contribution(n, l, v, albedo, specular_strength, shininess, radiance);
    }

    // Spot lights.
    for (var i = 0u; i < lights.counts.y; i = i + 1u) {
        let spot = lights.spots[i];
        let spot_pos = spot.position_intensity.xyz;
        let spot_range = spot.direction_range.w;

        let to_surface = in.world_position - spot_pos;
        let dist = length(to_surface);
        // Outside range: contribution is zero by the windowed
        // attenuation anyway; the early-skip just avoids the
        // diffuse/specular work for fragments far from the lamp.
        if (dist >= spot_range) {
            continue;
        }
        let to_surface_n = to_surface / dist;

        // Cone factor: smoothstep between outer and inner cosines.
        // dot(to_surface_n, spot_dir) ≈ 1 when the fragment sits
        // directly in the beam; falls off as the angle widens.
        let cone_dot = dot(to_surface_n, spot.direction_range.xyz);
        let cone_factor = smoothstep(
            spot.outer_cos_pad.x,
            spot.color_inner_cos.w,
            cone_dot,
        );

        let range_factor = range_attenuation(dist, spot_range);
        let radiance = spot.color_inner_cos.xyz
                     * spot.position_intensity.w
                     * cone_factor
                     * range_factor;

        // Surface-to-light is the negation of light-to-surface.
        let l = -to_surface_n;
        lit += blinn_phong_contribution(n, l, v, albedo, specular_strength, shininess, radiance);
    }

    // Point lights.
    for (var i = 0u; i < lights.counts.z; i = i + 1u) {
        let pt = lights.points[i];
        let pt_pos = pt.position_range.xyz;
        let pt_range = pt.position_range.w;

        let to_surface = in.world_position - pt_pos;
        let dist = length(to_surface);
        if (dist >= pt_range) {
            continue;
        }
        let to_surface_n = to_surface / dist;

        let range_factor = range_attenuation(dist, pt_range);
        let radiance = pt.color_intensity.xyz
                     * pt.color_intensity.w
                     * range_factor;

        let l = -to_surface_n;
        lit += blinn_phong_contribution(n, l, v, albedo, specular_strength, shininess, radiance);
    }

    // Ambient is read unconditionally from the directional slot —
    // see `LightsUniformData::placeholder` for why ambient lives
    // there even when no directional exists.
    let ambient = albedo * lights.directional.ambient.xyz;

    // Emissive is the surface's own light — outside the lit term
    // so it survives shadow and ambient is irrelevant.
    let emissive = model.emissive.xyz * model.emissive.w;

    return vec4<f32>(ambient + lit + emissive, 1.0);
}
