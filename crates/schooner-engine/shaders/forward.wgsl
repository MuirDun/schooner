// Forward pipeline — Blinn–Phong with one directional + N spot +
// N point lights.
//
// Bind groups:
//   @group(0) — Camera   (view, proj, view_proj, position).
//   @group(1) — Lights   (directional + spot/point arrays + counts).
//   @group(2) — Model    (per-draw model matrix + material params,
//                         dynamic-offset).
//   @group(3) — Shadow   (depth-array + comparison sampler).
//   @group(4) — Material (albedo texture + linear-repeat sampler).
//
// Vertex layout matches `render::mesh::Vertex`: pos (3f) at
// location 0, normal (3f) at location 1, uv (2f) at location 2.

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
    // shadow), .z = god_ray_intensity (per-spot multiplier on the
    // medium's scattering coefficient, 1.E.2), .w padding.
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
//
// Fog is folded into this uniform (rather than a separate bind group)
// because 1.E.2's god-ray loop reads both fog and spot fields inside
// the spot iteration — co-location keeps the bind-group count at four
// (wgpu's default `max_bind_groups`).
struct LightsUniform {
    directional: DirectionalLight,
    spots: array<SpotLight, 8>,
    points: array<PointLight, 16>,
    // .x = directional count (0 or 1), .y = spot count, .z = point
    // count, .w = shadow PCF half-kernel (0 → 1×1, 1 → 3×3,
    // 2 → 5×5). Driven by the F1 debug key via `DebugState`.
    counts: vec4<u32>,
    // .xyz = fog color (linear), .w = density. density=0 disables fog.
    fog_color_density: vec4<f32>,
    // .x = base_height (world y), .y = falloff (1/units),
    // .z = scattering coefficient (god-ray strength), .w reserved.
    fog_base_falloff: vec4<f32>,
};

struct ModelUniform {
    model: mat4x4<f32>,
    // .xyz = albedo (linear), .w = roughness ∈ [0, 1].
    albedo_roughness: vec4<f32>,
    // .xyz = emissive color (linear), .w = emissive intensity.
    emissive: vec4<f32>,
    // .x = opacity ∈ [0, 1] (multiplied into texture alpha), .y =
    // depth bias in world metres (view-ray nudge toward camera), .z =
    // Fresnel rim strength (0 = matte dielectric), .w = normal-map
    // strength (scales the tangent-space xy; 0 = flat, 1 = full relief).
    params: vec4<f32>,
    // .xy = UV tiling scale, .zw = UV offset. Applied as
    // `uv * scale + offset` in the vertex shader. In triplanar mode
    // .x is reused as world repeats/metre.
    uv_scale_offset: vec4<f32>,
    // .x = triplanar on/off (0.0 / 1.0), .yzw reserved.
    flags: vec4<f32>,
};

@group(0) @binding(0) var<uniform> camera: CameraUniform;
@group(1) @binding(0) var<uniform> lights: LightsUniform;
@group(2) @binding(0) var<uniform> model: ModelUniform;
@group(3) @binding(0) var shadow_maps: texture_depth_2d_array;
@group(3) @binding(1) var shadow_sampler: sampler_comparison;
@group(4) @binding(0) var albedo_texture: texture_2d<f32>;
@group(4) @binding(1) var albedo_sampler: sampler;
// Normal map — sampled with the same sampler as albedo. The texture is
// linear-format (`Rgba8Unorm`); FLAT_NORMAL (0,0,1) binds here for
// materials with no authored map, making the perturbation the identity.
@group(4) @binding(2) var normal_texture: texture_2d<f32>;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    // Mesh-local tangent: xyz = direction of increasing U, w = bitangent
    // handedness (±1). Forms the TBN basis with the normal.
    @location(3) tangent: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) world_position: vec3<f32>,
    @location(1) world_normal: vec3<f32>,
    // Perspective-correct interpolated per-fragment UV. The
    // rasterizer divides by W to keep texture-space distances
    // correct across foreshortened triangles — without that, a
    // wall viewed at an angle would stretch the texture toward
    // the far end.
    @location(2) uv: vec2<f32>,
    // World-space tangent (xyz) + handedness (w), interpolated. The
    // fragment shader re-orthonormalizes against the interpolated
    // normal before building the TBN matrix.
    @location(3) world_tangent: vec4<f32>,
};

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    let world_pos4 = model.model * vec4<f32>(in.position, 1.0);
    var out: VertexOutput;

    // Depth bias: shift the vertex toward the camera *along the view
    // ray* by `params.y` world metres before projection. Because the
    // shift is along the camera→vertex ray, the vertex stays on the
    // same ray and therefore projects to the same screen pixel — only
    // its depth shrinks, so a coplanar decal wins `depth_compare: Less`
    // against its host surface without visibly lifting off it. Bias = 0
    // (every opaque surface) leaves the position untouched. The guard
    // avoids a normalize-of-zero when a vertex sits exactly at the eye.
    let to_cam = camera.position.xyz - world_pos4.xyz;
    let to_cam_dir = to_cam / max(length(to_cam), 1e-6);
    let biased = world_pos4.xyz + to_cam_dir * model.params.y;
    out.clip_position = camera.view_proj * vec4<f32>(biased, 1.0);

    // Shading reads the *unbiased* world position — the bias is a
    // depth-test trick, not a real geometric move, so lighting, shadows,
    // and fog must see where the surface actually is.
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
    // Tangent transforms by the same model 3×3 as the normal (under
    // uniform scale they share the basis); handedness passes through
    // untouched. Re-orthonormalization is deferred to the fragment
    // shader, after interpolation.
    out.world_tangent = vec4<f32>(normal_mat * in.tangent.xyz, in.tangent.w);
    // UV tiling + offset. Linear, so perspective-correct interpolation
    // still holds across the triangle.
    out.uv = in.uv * model.uv_scale_offset.xy + model.uv_scale_offset.zw;
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
// UV + depth, with normal-offset bias applied to suppress self-
// shadow acne. Returns the comparison factor in [0, 1].
//
// Normal-offset bias (Sylvan 2008, Holbert 2010): rather than
// shifting the comparison *depth*, shift the *sample position*
// along the surface normal before projection. The shift is
// scaled by (1 - n·l) so:
//   - perpendicular surfaces (n·l → 1) get zero offset — they
//     don't need bias and we don't want to peter-pan their
//     contact points.
//   - grazing surfaces (n·l → 0) get the full offset, which
//     pushes the sampling position off the surface toward the
//     light just enough that its `proj.z` lands cleanly inside
//     the lit half of the comparison.
// The offset is in world units; tune via `NORMAL_OFFSET_SCALE`
// if a future scene's typical occluder-to-receiver distance
// changes. Combined with the rasterizer-level slope-scaled bias
// (defense at the far plane), this is the modern shadow-mapping
// recipe.
fn spot_shadow_factor(spot: SpotLight, world_position: vec3<f32>, n: vec3<f32>) -> f32 {
    let shadow_idx = i32(spot.outer_cos_shadow.y);
    if (shadow_idx < 0) {
        return 1.0;
    }

    let to_light = normalize(spot.position_intensity.xyz - world_position);
    let n_dot_l = clamp(dot(n, to_light), 0.0, 1.0);
    let NORMAL_OFFSET_SCALE = 0.02;
    let sample_pos = world_position + n * (NORMAL_OFFSET_SCALE * (1.0 - n_dot_l));

    let light_space = spot.view_proj * vec4<f32>(sample_pos, 1.0);
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

// Optical depth of an exponential-height fog medium along the view
// ray from camera to fragment. Closed-form integral derived in
// `src/render/fog.rs` (Wenzel 2007, GPU Gems 2 §16):
//
//     ρ(y) = density · exp(-falloff · (y − base_height))
//     ρ_C  = density · exp(-falloff · (Cy − base_height))
//     τ    = ρ_C · (1 − exp(-falloff · Δy)) / (falloff · Δy) · length
//
// `density = 0` is the common path (no scene has authored fog) and
// short-circuits to zero. The `(falloff · Δy)` divisor degenerates
// at horizontal rays — the guard substitutes the Taylor limit
// `(1 − e^x)/x → 1` as x → 0. An `if` rather than `select()` because
// WGSL `select` evaluates both branches, and the divide would
// produce NaN for the unused branch at exactly horizontal rays.
fn fog_optical_depth(world_position: vec3<f32>, camera_position: vec3<f32>) -> f32 {
    let density = lights.fog_color_density.w;
    if (density <= 0.0) {
        return 0.0;
    }
    let base = lights.fog_base_falloff.x;
    let falloff = lights.fog_base_falloff.y;

    let segment = world_position - camera_position;
    let dist = length(segment);
    if (dist <= 0.0) {
        return 0.0;
    }

    let density_at_camera = density * exp(-falloff * (camera_position.y - base));
    let kdy = falloff * segment.y;
    var attenuation: f32;
    if (abs(kdy) < 1e-4) {
        attenuation = 1.0;
    } else {
        attenuation = (1.0 - exp(-kdy)) / kdy;
    }
    return density_at_camera * attenuation * dist;
}

// Analytic ray-cone segment intersection. Returns (t0, t1) such that
// `origin + t·dir` is inside the spot cone for `t ∈ [t0, t1]`,
// clipped to `[0, t_max]` and the apex-centered `range` sphere.
// Caller treats `t0 > t1` as "no hit." `dir` and `axis` are assumed
// unit-length; both invariants hold at the call site below.
//
// ## The quadratic
//
// A point P is inside an infinite cone with apex A, axis V (unit),
// half-angle h iff `dot(P − A, V) ≥ cos(h) · |P − A|`. Squaring and
// substituting `P = origin + t·dir` (so `U = (origin − A) + t·dir`)
// yields a quadratic in t with coefficients:
//
//     a = (D·V)² − cos²h
//     b = 2·(D·V)·(Q·V) − 2·cos²h·(D·Q)        where Q = origin − A
//     c = (Q·V)² − cos²h·|Q|²
//
// The `a < 0` branch is the common case (cone half-angle < 45° and
// the view ray not aligned with the axis): inside-cone is `[lo, hi]`.
// The `a > 0` branch is the wide-cone / near-axial case: inside-cone
// is `t ≤ lo` or `t ≥ hi`, and we pick the segment in the front cone
// (the one where `dot(P − A, V) ≥ 0`). The shared squaring step
// makes the quadratic agnostic to the front/back cone split, so we
// always reapply the front-half-plane clip downstream.
//
// Reference: Eberly, *3D Game Engine Design* 2e §6.2 — the same
// derivation, written for raytracers.
fn ray_cone_segment(
    origin: vec3<f32>,
    dir: vec3<f32>,
    t_max: f32,
    apex: vec3<f32>,
    axis: vec3<f32>,
    cos_half_angle: f32,
    range: f32,
) -> vec2<f32> {
    let no_hit = vec2<f32>(1.0, 0.0);

    let q = origin - apex;
    let dv = dot(dir, axis);
    let qv = dot(q, axis);
    let dq = dot(dir, q);
    let qq = dot(q, q);
    let cos2 = cos_half_angle * cos_half_angle;

    let a = dv * dv - cos2;
    let b = 2.0 * (dv * qv - cos2 * dq);
    let c = qv * qv - cos2 * qq;

    // Degenerate (ray skims cone surface) — rare; skip rather than
    // wedge on the divide.
    if (abs(a) < 1e-6) {
        return no_hit;
    }

    let disc = b * b - 4.0 * a * c;
    if (disc < 0.0) {
        return no_hit;
    }
    let sd = sqrt(disc);
    let inv2a = 0.5 / a;
    let lo = min((-b - sd) * inv2a, (-b + sd) * inv2a);
    let hi = max((-b - sd) * inv2a, (-b + sd) * inv2a);

    var t0: f32;
    var t1: f32;
    if (a < 0.0) {
        // Narrow cone — inside region is [lo, hi]. May still
        // straddle the apex plane; the front-half-plane clip below
        // trims the back half.
        t0 = lo;
        t1 = hi;
        if (abs(dv) > 1e-6) {
            let t_split = -qv / dv;
            if (dv > 0.0) {
                t0 = max(t0, t_split);
            } else {
                t1 = min(t1, t_split);
            }
        } else if (qv < 0.0) {
            return no_hit;
        }
    } else {
        // Wide cone / near-axial view — inside region is the union
        // of `t ≤ lo` and `t ≥ hi`. Pick whichever falls in the
        // front cone.
        let front_hi = qv + hi * dv;
        let front_lo = qv + lo * dv;
        if (front_hi >= 0.0) {
            t0 = hi;
            t1 = 1e30;
        } else if (front_lo >= 0.0) {
            t0 = -1e30;
            t1 = lo;
        } else {
            return no_hit;
        }
    }

    // Clip to the visible view-ray segment.
    t0 = max(t0, 0.0);
    t1 = min(t1, t_max);

    // Intersect with the apex-centered range sphere
    // (|q + t·d|² ≤ range²; |d| = 1 makes it a clean quadratic in t).
    let disc_r = dq * dq - qq + range * range;
    if (disc_r <= 0.0) {
        return no_hit;
    }
    let sr = sqrt(disc_r);
    t0 = max(t0, -dq - sr);
    t1 = min(t1, -dq + sr);

    return vec2<f32>(t0, t1);
}

// Albedo + world-space normal produced by a sampling path (UV or
// triplanar), handed back to the shared lighting code.
struct Surface {
    albedo: vec4<f32>,
    normal: vec3<f32>,
};

// World-space triplanar projection. Samples albedo + normal on the three
// world planes and blends by the geometric normal, so a texture stays
// continuous across separately-spawned boxes — no UVs, no per-box seam,
// no thin-reveal stretching. The normal uses Golus's whiteout blend
// ("Normal Mapping for a Triplanar Shader", 2017): naive triplanar
// flattens normal maps, so each plane's tangent normal is reoriented by
// the geometric normal before the blend. `scale` is world repeats/metre,
// `strength` scales the tangent-space xy (the normal_strength knob).
fn sample_triplanar(world_pos: vec3<f32>, geo_n: vec3<f32>, scale: f32, strength: f32) -> Surface {
    let p = world_pos * scale;

    // Blend weights, sharpened so the three-way overlap band stays tight.
    var w = abs(geo_n);
    w = w * w * w;
    w = w / (w.x + w.y + w.z);

    // Canonical plane UVs: X-facing reads zy, Y-facing xz, Z-facing xy.
    let a_x = textureSample(albedo_texture, albedo_sampler, p.zy);
    let a_y = textureSample(albedo_texture, albedo_sampler, p.xz);
    let a_z = textureSample(albedo_texture, albedo_sampler, p.xy);

    var n_x = textureSample(normal_texture, albedo_sampler, p.zy).xyz * 2.0 - 1.0;
    var n_y = textureSample(normal_texture, albedo_sampler, p.xz).xyz * 2.0 - 1.0;
    var n_z = textureSample(normal_texture, albedo_sampler, p.xy).xyz * 2.0 - 1.0;
    n_x = vec3<f32>(n_x.xy * strength, n_x.z);
    n_y = vec3<f32>(n_y.xy * strength, n_y.z);
    n_z = vec3<f32>(n_z.xy * strength, n_z.z);

    // Whiteout blend: reorient each plane's tangent normal into world
    // space via the geometric normal, then blend with the same weights.
    let t_x = vec3<f32>(n_x.xy + geo_n.zy, abs(n_x.z) * geo_n.x);
    let t_y = vec3<f32>(n_y.xy + geo_n.xz, abs(n_y.z) * geo_n.y);
    let t_z = vec3<f32>(n_z.xy + geo_n.xy, abs(n_z.z) * geo_n.z);
    let world_n = normalize(t_x.zyx * w.x + t_y.xzy * w.y + t_z.xyz * w.z);

    var out: Surface;
    out.albedo = a_x * w.x + a_y * w.y + a_z * w.z;
    out.normal = world_n;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let geo_n = normalize(in.world_normal);

    // Surface sampling: triplanar (world-space, seamless across boxes)
    // when `flags.x` is set, otherwise ordinary UV mapping. The branch is
    // uniform across the draw (the flag is per-draw uniform data), so no
    // warp divergence — the unused path is skipped for free.
    //
    // UV-path normal mapping: decode the tangent-space normal [0,1] →
    // [-1,1], scale xy by `normal_strength` (params.w), and rotate into
    // world space through the TBN. The interpolated tangent is
    // re-orthonormalized (Gram-Schmidt) since interpolation breaks
    // orthogonality; handedness comes from the tangent's w. FLAT_NORMAL
    // decodes to (0,0,1), the identity for unmapped materials.
    var tex_sample: vec4<f32>;
    var n: vec3<f32>;
    if (model.flags.x > 0.5) {
        let surf = sample_triplanar(
            in.world_position, geo_n, model.uv_scale_offset.x, model.params.w);
        tex_sample = surf.albedo;
        n = surf.normal;
    } else {
        tex_sample = textureSample(albedo_texture, albedo_sampler, in.uv);
        let tn_raw = textureSample(normal_texture, albedo_sampler, in.uv).xyz * 2.0 - 1.0;
        let tn = vec3<f32>(tn_raw.xy * model.params.w, tn_raw.z);
        let t = normalize(in.world_tangent.xyz - geo_n * dot(geo_n, in.world_tangent.xyz));
        let b = cross(geo_n, t) * in.world_tangent.w;
        let tbn = mat3x3<f32>(t, b, geo_n);
        n = normalize(tbn * tn);
    }

    let v = normalize(camera.position.xyz - in.world_position);

    // Albedo: per-instance tint × sampled albedo (UV or triplanar, from
    // the branch above). The texture is `Rgba8UnormSrgb`, so the sample
    // is *linear* — no manual `pow(rgb, 2.2)`. Materials without an
    // authored texture bind the WHITE 1×1 built-in, making the multiply a
    // no-op so textured and untextured paths stay uniform.
    let albedo = model.albedo_roughness.xyz * tex_sample.rgb;
    let roughness = model.albedo_roughness.w;
    // Output alpha comes straight from the texture's alpha channel
    // (linear even on an sRGB texture — sRGB encodes only RGB). The
    // opaque pipeline uses REPLACE blend, which writes this alpha but
    // never blends on it, so the opaque path is unchanged; the
    // transparent pipeline's over-operator reads it as coverage. A
    // material with no authored texture binds the WHITE 1×1 built-in
    // (alpha = 1), so untextured surfaces stay fully opaque.

    // Map roughness ∈ [0, 1] to Blinn–Phong shininess. Calibrated
    // so roughness = 0.5 yields shininess ≈ 32 — matching the
    // Game 0 hardcoded value, so a Material::DEFAULT surface reads
    // the same as the pre-Material baseline.
    let shininess = pow(2.0, mix(0.0, 10.0, 1.0 - roughness));

    // Fresnel (Schlick 1994, "An Inexpensive BRDF Model"): a smooth
    // dielectric reflects little head-on and approaches a mirror at
    // grazing angles. `params.z` gates it — 0 leaves every opaque
    // surface and flat decal matte (no behaviour change). `abs(dot)`
    // rather than a clamped dot so a double-sided pane (the transparent
    // pipeline disables back-face cull) reads the same Fresnel from
    // either side instead of forcing its back face fully reflective.
    let n_dot_v = abs(dot(n, v));
    let fresnel = pow(1.0 - n_dot_v, 5.0) * model.params.z;

    // Glass is smooth and mirror-bright at the silhouette — lift the
    // specular toward 1 by the Fresnel factor so lights glint hardest
    // at the pane's edges. fresnel = 0 keeps the matte 0.3 baseline.
    let specular_strength = mix(0.3, 1.0, fresnel);

    var lit = vec3<f32>(0.0);
    // God-ray (analytic in-scattering through spot cones, 1.E.2)
    // accumulates separately from the surface `lit` term. The two
    // terms are different parts of the radiative-transfer equation —
    // `lit` is the surface radiance reaching the camera attenuated
    // by transmittance; `god_ray_sum` is the integral of in-scattered
    // light from the medium along the view ray. They sum at the end
    // alongside the height-fog blend.
    var god_ray_sum = vec3<f32>(0.0);

    // View ray from camera through this fragment. `view_len` is the
    // distance to the visible surface — the t_max for the cone-segment
    // clip so god-rays don't shine through walls. `do_god_rays` is a
    // uniform branch (fog params are uniform across the workgroup), so
    // the per-spot block is skipped for free when fog is disabled.
    let cam_to_surface = in.world_position - camera.position.xyz;
    let view_len = length(cam_to_surface);
    let view_dir = select(
        vec3<f32>(0.0, 0.0, 1.0),
        cam_to_surface / max(view_len, 1e-6),
        view_len > 1e-6,
    );
    let fog_density = lights.fog_color_density.w;
    let fog_scattering = lights.fog_base_falloff.z;
    let do_god_rays = (fog_density > 0.0) && (fog_scattering > 0.0);

    // Directional contribution. Skipped when counts.x == 0 so the
    // shader doesn't add a phantom sun from stale placeholder data.
    if (lights.counts.x > 0u) {
        let l = normalize(-lights.directional.direction.xyz);
        let radiance = lights.directional.color_intensity.xyz
                     * lights.directional.color_intensity.w;
        lit += blinn_phong_contribution(n, l, v, albedo, specular_strength, shininess, radiance);
    }

    // Spot lights — surface lighting AND god-ray in-scattering share
    // the same loop iteration. Spot data is loaded once and feeds
    // both contributions; the surface block is gated on the fragment
    // being inside the spot's range, while the god-ray block runs
    // whenever any view ray segment intersects the cone (a wall in
    // front of the cone clips correctly via `view_len` in t_max).
    for (var i = 0u; i < lights.counts.y; i = i + 1u) {
        let spot = lights.spots[i];
        let spot_pos = spot.position_intensity.xyz;
        let spot_range = spot.direction_range.w;
        let spot_dir = spot.direction_range.xyz;

        let to_surface = in.world_position - spot_pos;
        let dist = length(to_surface);
        // Surface contribution: skipped when the surface is outside
        // the spot's range (range attenuation would zero it anyway,
        // but the early skip avoids the diffuse/specular work).
        if (dist < spot_range) {
            let to_surface_n = to_surface / dist;

            // Cone factor: smoothstep between outer and inner cosines.
            // dot(to_surface_n, spot_dir) ≈ 1 when the fragment sits
            // directly in the beam; falls off as the angle widens.
            let cone_dot = dot(to_surface_n, spot_dir);
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
            let shadow_factor = spot_shadow_factor(spot, in.world_position, n);
            let radiance = spot.color_inner_cos.xyz
                         * spot.position_intensity.w
                         * cone_factor
                         * range_factor
                         * shadow_factor;

            // Surface-to-light is the negation of light-to-surface.
            let l = -to_surface_n;
            lit += blinn_phong_contribution(n, l, v, albedo, specular_strength, shininess, radiance);
        }

        // God-ray in-scattering — analytic single-scatter through
        // the cone segment of the view ray. See `ray_cone_segment`
        // above for the intersection math. The contribution at this
        // fragment is the segment-midpoint approximation of:
        //
        //     ∫ L_spot(P(t)) · σ_s · ρ(P(t).y) · T(camera→P(t)) dt
        //
        // where L_spot is the spot's radiance (color × intensity ×
        // cone × range attenuation), σ_s is `fog.scattering ·
        // spot.god_ray_intensity`, ρ is the exponential-height fog
        // density, and T is transmittance camera→midpoint via the
        // same Beer's-law optical depth the surface fog uses.
        //
        // Midpoint sampling is the practical-analytic shortcut: the
        // true integral with windowed `1/d²` and exponential ρ has
        // no clean closed form, so we evaluate radiance and density
        // once at the segment midpoint and scale by the segment
        // length. Toth 2009 popularised this for real-time light
        // shafts; the cost is ≈10 instructions per spot per fragment
        // regardless of segment length, and the read at indoor scale
        // is right. Volumetric shadow occlusion (the wall carving
        // the god-ray) wants raymarched shadow taps — deferred to
        // Game 2A+ if it earns its keep.
        if (do_god_rays) {
            let seg = ray_cone_segment(
                camera.position.xyz, view_dir, view_len,
                spot_pos, spot_dir,
                spot.outer_cos_shadow.x, spot_range,
            );
            if (seg.y > seg.x) {
                let t_mid = 0.5 * (seg.x + seg.y);
                let seg_len = seg.y - seg.x;
                let p_mid = camera.position.xyz + view_dir * t_mid;

                let to_mid = p_mid - spot_pos;
                let dist_to_mid = length(to_mid);
                let to_mid_n = to_mid / max(dist_to_mid, 1e-6);
                let cone_dot_mid = dot(to_mid_n, spot_dir);
                let cone_factor_mid = smoothstep(
                    spot.outer_cos_shadow.x,
                    spot.color_inner_cos.w,
                    cone_dot_mid,
                );
                let range_factor_mid = range_attenuation(dist_to_mid, spot_range);

                // Density at the midpoint's height (same formula as
                // the surface optical-depth helper, evaluated point-
                // wise rather than along an integral).
                let density_at_mid = fog_density
                    * exp(-lights.fog_base_falloff.y * (p_mid.y - lights.fog_base_falloff.x));
                // Transmittance camera→midpoint via the same Beer's-
                // law integral the surface fog uses.
                let tau_to_mid = fog_optical_depth(p_mid, camera.position.xyz);
                let trans_to_mid = exp(-tau_to_mid);

                let inscatter_coeff = fog_scattering * spot.outer_cos_shadow.z;
                let spot_radiance = spot.color_inner_cos.xyz
                                  * spot.position_intensity.w
                                  * cone_factor_mid
                                  * range_factor_mid;
                god_ray_sum += spot_radiance
                             * inscatter_coeff
                             * density_at_mid
                             * seg_len
                             * trans_to_mid;
            }
        }
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

    let scene = ambient + lit + emissive;

    // Height fog blends in HDR linear space against the lit color so
    // emissive surfaces dim through dense fog (a red lamp behind a
    // wall of fog reads as a red haze, not a hot spot poking through).
    // Tonemap then sees the already-foggy frame, so the curve shapes
    // the foggy highlights coherently with the rest of the scene.
    //
    // God-rays are additive on top — they represent extra radiance
    // scattered into the view ray by the medium itself, separate
    // from the surface-attenuation + ambient-in-scatter that `mix`
    // captures. This is the second term of the radiative-transfer
    // equation L = L_surface · T + ∫J · T dt; the mix is the first
    // term plus the ambient half of the integral.
    let optical_depth = fog_optical_depth(in.world_position, camera.position.xyz);
    let transmittance = exp(-optical_depth);
    let final_color = mix(lights.fog_color_density.xyz, scene, transmittance) + god_ray_sum;

    // Final alpha = texture coverage × material opacity, then lifted
    // toward 1 by Fresnel so edge-on glass turns reflective/opaque while
    // staying clear face-on. fresnel = 0 leaves it untouched, so the
    // opaque pipeline's REPLACE blend and every flat decal are
    // unaffected; the transparent pipeline's over-operator reads the
    // result as the blend weight.
    let alpha = mix(tex_sample.a * model.params.x, 1.0, fresnel);
    return vec4<f32>(final_color, alpha);
}
