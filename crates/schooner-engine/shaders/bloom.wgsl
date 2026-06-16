// Bloom — HDR bright-pass + Jimenez (COD:AW 2014) dual-filter pyramid.
//
// Runs between the forward pass and the post pass, entirely in HDR-
// linear space (Rgba16Float), so the downstream ACES tonemap still sees
// a single combined HDR image — bloom rolls into the highlights through
// the tonemap shoulder exactly the way the HL2 / Witcher 1 era glow did.
//
// The technique is modern (dual-filter pyramid, 2014); the *look* is
// tuned for 2005–2009 — a wide, soft, warm halation rather than the
// tight modern lens-bloom. Width comes from FILTER_RADIUS + mip count,
// warmth + intensity come from the composite in post.wgsl. Threshold,
// knee and radius are uniform-driven (@group(1)) so the whole effect
// stays a live art instrument.
//
// Three fragment entry points share one fullscreen-triangle vertex
// stage, one source bind group (mip + linear sampler), and one params
// uniform:
//   fs_prefilter   HDR full-res -> mip 0    threshold + Karis average
//   fs_downsample  mip i        -> mip i+1  13-tap box
//   fs_upsample    mip i        -> mip i-1  3x3 tent, additive-blended
//
// Pass schedule (driven by `BloomPipeline::record`):
//   prefilter HDR -> mip0
//   downsample mip0 -> mip1 -> ... -> mip[N-1]   (REPLACE)
//   upsample  mip[N-1] -> mip[N-2] -> ... -> mip0 (additive, accumulates)
// After upsample, mip0 holds the full accumulated bloom; post samples it.

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var src_sampler: sampler;

// x = threshold, y = soft-knee width, z = upsample filter radius, w = pad.
// Mirrors `BloomParamsUniform` in src/render/bloom.rs.
struct BloomParams {
  params: vec4<f32>,
};
@group(1) @binding(0) var<uniform> bloom: BloomParams;

struct VertexOutput {
  @builtin(position) clip_position: vec4<f32>,
  @location(0) uv: vec2<f32>,
};

// Same fullscreen triangle as in post.wgsl: vertices outside the screen
// whose bounding rect covers the viewport; UVs derived from vertex_index.
@vertex
fn vs_main(@builtin(vertex_index) vid: u32) -> VertexOutput {
  let x = f32((vid << 1u) & 2u);
  let y = f32(vid & 2u);
  let pos = vec2<f32>(x * 2.0 - 1.0, y * 2.0 - 1.0);
  var out: VertexOutput;
  out.clip_position = vec4<f32>(pos, 0.0, 1.0);
  out.uv = vec2<f32>(x, 1.0 - y);
  return out;
}

// ---- threshold / firefly suppression ----------------------------------

// Soft-knee bright pass. Only HDR values above `threshold` bloom; the
// knee gives a smooth ramp into it rather than a hard clip, so a surface
// hovering near the threshold doesn't pop in and out as it brightens.
// Keep threshold >= 1.0 for the restrained default: "only things brighter
// than a fully-lit white surface glow" — the emissive food gel, the lamp
// filament, hot speculars. Drop it below 1.0 and the whole room starts to
// haze (the "everything glows" end of the dial, which is deliberately
// reachable via the Bloom resource — just not the default).
fn prefilter_curve(c: vec3<f32>, threshold: f32, knee: f32) -> vec3<f32> {
    let brightness = max(c.r, max(c.g, c.b));
    let k = threshold * knee;
    var soft = brightness - threshold + k;
    soft = clamp(soft, 0.0, 2.0 * k);
    soft = soft * soft / (4.0 * k + 1e-4);
    let contribution = max(soft, brightness - threshold) / max(brightness, 1e-4);
    return c * contribution;
}

fn luma(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

// Karis average weight: a box that is much brighter than its neighbours
// (a sub-pixel firefly that survived rasterization) gets downweighted by
// 1/(1+luma), so it can't dominate the downsample and smear into a
// flickering bloom dot. Applied on the *first* downsample only — once the
// pyramid is built, energy is already spread and fireflies are gone.
fn karis_weight(c: vec3<f32>) -> f32 {
    return 1.0 / (1.0 + luma(c));
}

// ---- shared 13-tap sampler --------------------------------------------
// Returns the five 2x2 box averages so the prefilter can Karis-weight
// them and the plain downsample can sum them. `t` is the SOURCE texel
// size (1 / textureDimensions); linear sampling means each tap is itself
// a 2x2 average — 13 taps cover ~36 source texels. Layout:
//
//   a   b   c
//     j   k
//   d   e   f
//     l   m
//   g   h   i
struct Groups {
    g0: vec3<f32>, g1: vec3<f32>, g2: vec3<f32>, g3: vec3<f32>, g4: vec3<f32>,
};

fn sample13(uv: vec2<f32>, t: vec2<f32>) -> Groups {
    let a = textureSample(src, src_sampler, uv + vec2<f32>(-2.0 * t.x,  2.0 * t.y)).rgb;
    let b = textureSample(src, src_sampler, uv + vec2<f32>( 0.0,        2.0 * t.y)).rgb;
    let c = textureSample(src, src_sampler, uv + vec2<f32>( 2.0 * t.x,  2.0 * t.y)).rgb;
    let d = textureSample(src, src_sampler, uv + vec2<f32>(-2.0 * t.x,  0.0)).rgb;
    let e = textureSample(src, src_sampler, uv).rgb;
    let f = textureSample(src, src_sampler, uv + vec2<f32>( 2.0 * t.x,  0.0)).rgb;
    let g = textureSample(src, src_sampler, uv + vec2<f32>(-2.0 * t.x, -2.0 * t.y)).rgb;
    let h = textureSample(src, src_sampler, uv + vec2<f32>( 0.0,       -2.0 * t.y)).rgb;
    let i = textureSample(src, src_sampler, uv + vec2<f32>( 2.0 * t.x, -2.0 * t.y)).rgb;
    let j = textureSample(src, src_sampler, uv + vec2<f32>(-t.x,  t.y)).rgb;
    let k = textureSample(src, src_sampler, uv + vec2<f32>( t.x,  t.y)).rgb;
    let l = textureSample(src, src_sampler, uv + vec2<f32>(-t.x, -t.y)).rgb;
    let m = textureSample(src, src_sampler, uv + vec2<f32>( t.x, -t.y)).rgb;
    var out: Groups;
    out.g0 = (a + b + d + e) * 0.25; // top-left   2x2
    out.g1 = (b + c + e + f) * 0.25; // top-right
    out.g2 = (d + e + g + h) * 0.25; // bottom-left
    out.g3 = (e + f + h + i) * 0.25; // bottom-right
    out.g4 = (j + k + l + m) * 0.25; // centre
    return out;
}

@fragment
fn fs_prefilter(in: VertexOutput) -> @location(0) vec4<f32> {
    let t = 1.0 / vec2<f32>(textureDimensions(src));
    let gr = sample13(in.uv, t);
    // Karis-weighted, centre-heavy combine. The base weights (0.125*4 +
    // 0.5) sum to 1.0; the Karis factor deliberately breaks that per-box
    // to kill fireflies.
    let c = gr.g0 * (karis_weight(gr.g0) * 0.125)
          + gr.g1 * (karis_weight(gr.g1) * 0.125)
          + gr.g2 * (karis_weight(gr.g2) * 0.125)
          + gr.g3 * (karis_weight(gr.g3) * 0.125)
          + gr.g4 * (karis_weight(gr.g4) * 0.5);
    // Bright pass last, on the firefly-tamed downsample.
    let bright = prefilter_curve(c, bloom.params.x, bloom.params.y);
    return vec4<f32>(bright, 1.0);
}

@fragment
fn fs_downsample(in: VertexOutput) -> @location(0) vec4<f32> {
    let t = 1.0 / vec2<f32>(textureDimensions(src));
    let gr = sample13(in.uv, t);
    let c = (gr.g0 + gr.g1 + gr.g2 + gr.g3) * 0.125 + gr.g4 * 0.5;
    return vec4<f32>(c, 1.0);
}

@fragment
fn fs_upsample(in: VertexOutput) -> @location(0) vec4<f32> {
    // Widen for a softer, larger halo (in source texels). 1.0 is a tight,
    // crisp bloom; the era look wants 2–3. Driven by the uniform so it
    // stays tunable.
    let t = (1.0 / vec2<f32>(textureDimensions(src))) * bloom.params.z;
    let a = textureSample(src, src_sampler, in.uv + vec2<f32>(-t.x,  t.y)).rgb;
    let b = textureSample(src, src_sampler, in.uv + vec2<f32>( 0.0,  t.y)).rgb;
    let c = textureSample(src, src_sampler, in.uv + vec2<f32>( t.x,  t.y)).rgb;
    let d = textureSample(src, src_sampler, in.uv + vec2<f32>(-t.x,  0.0)).rgb;
    let e = textureSample(src, src_sampler, in.uv).rgb;
    let f = textureSample(src, src_sampler, in.uv + vec2<f32>( t.x,  0.0)).rgb;
    let g = textureSample(src, src_sampler, in.uv + vec2<f32>(-t.x, -t.y)).rgb;
    let h = textureSample(src, src_sampler, in.uv + vec2<f32>( 0.0, -t.y)).rgb;
    let i = textureSample(src, src_sampler, in.uv + vec2<f32>( t.x, -t.y)).rgb;
    // 3x3 tent (1 2 1 / 2 4 2 / 1 2 1) / 16. The additive accumulate onto
    // the destination mip (which holds its own downsampled content) is
    // done by the pipeline's BlendState, not here.
    let c2 = (e * 4.0 + (b + d + f + h) * 2.0 + (a + c + g + i)) * (1.0 / 16.0);
    return vec4<f32>(c2, 1.0);
}
