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
    // .x = outer cone cosine, .y = shadow_index (negative ⇒ no
    // shadow), .zw padding.
    outer_cos_shadow: vec4<f32>,
    // Light-space view-projection matrix. Read only when
    // shadow_index >= 0; zero matrix for non-shadowcasters.
    view_proj: mat4x4<f32>,
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
    // count, .w = shadow PCF half-kernel (0 → 1×1, 1 → 3×3,
    // 2 → 5×5). Driven by the P debug key via `DebugState`.
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
@group(3) @binding(0) var shadow_maps: texture_depth_2d_array;
@group(3) @binding(1) var shadow_sampler: sampler_comparison;

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

// Variable-radius PCF tap. Half-kernel size comes from
// `lights.counts.w` (0/1/2 → 1×1/3×3/5×5); the loop is uniform
// across the workgroup because the source is a uniform buffer.
// The comparison sampler's bilinear filter already gives 2×2 PCF
// per tap, so a 3×3 kernel here is effectively a 4×4 weighted
// average — generous soft edges at low texture cost.
//
// Returns 1.0 (lit) when the fragment falls outside the shadow
// frustum or behind its far plane. ClampToEdge on the sampler
// hides the visible boundary; the depth-range guard ensures
// fragments past the spot's `range` aren't comparison-tested
// against potentially-stale depth at the texture edge.
fn sample_shadow_pcf(layer: i32, uv: vec2<f32>, depth: f32) -> f32 {
    if (uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0
        || depth < 0.0 || depth > 1.0) {
        return 1.0;
    }
    let dims = textureDimensions(shadow_maps);
    let texel = 1.0 / vec2<f32>(f32(dims.x), f32(dims.y));
    let half_k = i32(lights.counts.w);
    let kernel_side = 2 * half_k + 1;
    let tap_count = f32(kernel_side * kernel_side);
    var sum = 0.0;
    for (var y = -half_k; y <= half_k; y = y + 1) {
        for (var x = -half_k; x <= half_k; x = x + 1) {
            let offset = vec2<f32>(f32(x), f32(y)) * texel;
            sum = sum + textureSampleCompare(
                shadow_maps, shadow_sampler, uv + offset, layer, depth);
        }
    }
    return sum / tap_count;
}

// Project a world-space position into a spot light's shadow-map
// UV + depth. Returns the layer to sample (caller checks it
// against -1 before sampling).
fn spot_shadow_factor(spot: SpotLight, world_position: vec3<f32>) -> f32 {
    let shadow_idx = i32(spot.outer_cos_shadow.y);
    if (shadow_idx < 0) {
        return 1.0;
    }
    let light_space = spot.view_proj * vec4<f32>(world_position, 1.0);
    if (light_space.w <= 0.0) {
        // Behind the light's near plane — outside the frustum,
        // treat as fully lit (the surface contributes via the
        // cone falloff anyway).
        return 1.0;
    }
    let proj = light_space.xyz / light_space.w;
    // NDC.xy ∈ [-1, 1] → UV ∈ [0, 1]. Y is flipped because NDC's
    // +Y is up while texture +V is down.
    let uv = vec2<f32>(proj.x * 0.5 + 0.5, -proj.y * 0.5 + 0.5);
    // NDC.z is already in [0, 1] for wgpu/RH projection — same
    // convention `Mat4::perspective_rh` produces.
    return sample_shadow_pcf(shadow_idx, uv, proj.z);
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
            spot.outer_cos_shadow.x,
            spot.color_inner_cos.w,
            cone_dot,
        );

        let range_factor = range_attenuation(dist, spot_range);
        // Shadow factor multiplies the radiance so the cone shape
        // and range falloff still apply at the silhouette of the
        // shadow — a fragment in shadow but inside the cone is
        // dimmer-but-still-tinted, not pitch black.
        let shadow_factor = spot_shadow_factor(spot, in.world_position);
        let radiance = spot.color_inner_cos.xyz
                     * spot.position_intensity.w
                     * cone_factor
                     * range_factor
                     * shadow_factor;

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
