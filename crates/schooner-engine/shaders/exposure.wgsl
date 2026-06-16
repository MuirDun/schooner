// Auto-exposure (eye adaptation) — measure the average scene luminance off
// the HDR target and drive a temporally-adapted exposure scalar that the
// post pass multiplies in *before* the tone curve.
//
// Three fragment entry points share this module, each used by its own
// pipeline:
//   fs_prefilter  — HDR colour -> (luma·w, w) for a centre weight w, into
//                   luma mip 0. The pixel luma is soft-capped first.
//   fs_downsample — box-average one mip down; the chain's final 1x1 mip
//                   holds (Σ luma·w, Σ w) over the whole frame.
//   fs_adapt      — divide to get the centre-weighted mean luminance, ease
//                   toward the target exposure (key / luma), write it out.
//
// ## Metering design — centre-weighted + soft-capped
//
// Two earlier metrics failed in this high-contrast dark room:
//   - Geometric mean (log average): deliberately *insensitive* to small
//     highlights, so it stayed pinned to the dark level and never reacted.
//   - Highlight-weighted E[luma²]/E[luma]: the opposite failure — dominated
//     by the single brightest pixel, so the exposure flipped between
//     extremes depending on whether the tiny lamp was on screen (twitchy).
//
// The fix is a *centre-weighted average of soft-capped luminance*:
//   - CENTRE WEIGHT: a Gaussian falloff from screen centre means the
//     exposure tracks what the player is *looking at*. Glance the lamp to
//     the edge and it barely counts; look straight at it and it dominates.
//     Smooth and directable instead of binary on/off.
//   - SOFT CAP: clamping each pixel's metered luma means one searing lamp
//     pixel can't dominate the average. Exposure is driven by *how much of
//     the central view is bright*, not by the peak — so a brighter lamp
//     doesn't make the metering twitchier.
// This is the classic centre-weighted meter every camera ships, adapted to
// HDR with a highlight cap.
//
// Bind groups:
//   @group(0) binding 0/1 — source texture + linear-clamp sampler. For the
//     reduction passes this is the HDR target / a luma mip; for adapt it is
//     the 1x1 mean-luma mip. binding 2 (adapt only) is the previous
//     exposure (the reduction pipelines bind a 2-entry layout that omits it).
//   @group(1) binding 0 — adapt params uniform (adapt pipeline only).

@group(0) @binding(0) var src_tex: texture_2d<f32>;
@group(0) @binding(1) var src_sampler: sampler;
// Adapt-only: the previous frame's exposure (1x1). The reduction entry
// points never reference it, so their 2-entry bind-group layout is a valid
// subset of what they use.
@group(0) @binding(2) var prev_exposure_tex: texture_2d<f32>;

// Mirrors `AdaptParamsUniform` in `src/render/exposure.rs`.
struct AdaptParams {
    // x = key (middle-grey target), y = min_luma, z = max_luma,
    // w = dt (seconds since last frame).
    a: vec4<f32>,
    // x = min_exposure, y = max_exposure, z = speed_brighten (eye opening
    // up, slow), w = speed_darken (eye stopping down, fast).
    b: vec4<f32>,
    // x = enabled (0/1). yzw padding.
    c: vec4<f32>,
};
@group(1) @binding(0) var<uniform> adapt: AdaptParams;

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

// Standard fullscreen-triangle (see `post.wgsl` for the derivation).
@vertex
fn vs_main(@builtin(vertex_index) vid: u32) -> VertexOutput {
    let x = f32((vid << 1u) & 2u);
    let y = f32(vid & 2u);
    var out: VertexOutput;
    out.clip_position = vec4<f32>(x * 2.0 - 1.0, y * 2.0 - 1.0, 0.0, 1.0);
    out.uv = vec2<f32>(x, 1.0 - y);
    return out;
}

// Rec. 709 luma coefficients — photometric weighting so exposure tracks
// perceived brightness rather than the raw RGB sum.
const LUMA = vec3<f32>(0.2126, 0.7152, 0.0722);
// Safety bound only — clamps pathological values (a future very-bright
// source) out of the average so they can't slam the meter to an extreme.
// Set high enough that the hostile lamp (emissive ~14) passes through
// *uncapped*: the lamp MUST count at full strength, or a small bright source
// can't pull the metered average down and looking at it won't crush. The
// directability ("only when I look at it") comes from the centre weight
// below, not from capping the lamp.
const METER_CAP = 50.0;
// Gaussian centre-weight falloff: w = exp(-CENTRE_FALLOFF · d²) for d the
// uv-space distance from screen centre. 7.0 puts the corners (d ≈ 0.71) at
// ~3% weight, concentrating the meter on the central third of the frame so
// the exposure follows where you *aim* — centre a small bright lamp and it
// dominates; let it drift to the edge and it barely counts. Raise for a
// tighter "spot meter", lower for a broader average.
const CENTRE_FALLOFF = 7.0;

// HDR -> (luma·w, w) where w is the centre weight. The targets are
// two-channel Rg16Float; `fs_adapt` divides Σluma·w by Σw for the
// centre-weighted mean luminance.
@fragment
fn fs_prefilter(in: VertexOutput) -> @location(0) vec4<f32> {
    let c = textureSample(src_tex, src_sampler, in.uv).rgb;
    let l = min(max(dot(c, LUMA), 0.0), METER_CAP);
    let d = in.uv - vec2<f32>(0.5, 0.5);
    let w = exp(-CENTRE_FALLOFF * dot(d, d));
    return vec4<f32>(l * w, w, 0.0, 1.0);
}

// Box-average down one mip level. A single bilinear tap at the destination
// texel centre averages the 2x2 source block (the destination is half the
// source size), so successive halving carries every source texel into the
// final 1x1 sums — both channels at once.
@fragment
fn fs_downsample(in: VertexOutput) -> @location(0) vec4<f32> {
    let s = textureSample(src_tex, src_sampler, in.uv);
    return vec4<f32>(s.r, s.g, 0.0, 1.0);
}

// Temporal adaptation. Reads the 1x1 mean log-luma and the previous
// exposure, eases toward the target exposure at a direction-dependent rate
// (the eye stops down faster than it opens up), and writes the new exposure.
//
// The ease is the standard exponential smoothing toward a moving target:
//   exposure += (target - exposure) * (1 - exp(-dt * speed))
// which is frame-rate independent (the `dt` inside the exponent makes the
// time constant 1/speed seconds regardless of how the frame time is sliced).
@fragment
fn fs_adapt(in: VertexOutput) -> @location(0) vec4<f32> {
    let center = vec2<f32>(0.5, 0.5);
    // Centre-weighted mean luminance: Σ(luma·w) / Σ(w). Tracks the brightness
    // of what's near the middle of the view, soft-capped so the lamp's peak
    // can't dominate.
    let means = textureSample(src_tex, src_sampler, center);
    let avg_luma = means.r / max(means.g, 1e-4);
    // Clamp the previous exposure up to the floor so the zero-initialised
    // texture on frame 0 adapts up from a sane value instead of from 0 (a
    // one-frame black flash on startup / resize).
    let prev = max(textureSample(prev_exposure_tex, src_sampler, center).r, adapt.b.x);

    // Disabled -> identity exposure, so the post multiply is a no-op.
    if (adapt.c.x < 0.5) {
        return vec4<f32>(1.0, 0.0, 0.0, 1.0);
    }

    let clamped = clamp(avg_luma, adapt.a.y, adapt.a.z);
    // `target` is a WGSL reserved keyword — use `goal`.
    let goal = clamp(adapt.a.x / clamped, adapt.b.x, adapt.b.y);
    // goal < prev means the scene got brighter, so we want to stop the
    // exposure down — use the faster `speed_darken`. Opening up (goal >
    // prev, a darker scene) uses the slower `speed_brighten`.
    let speed = select(adapt.b.z, adapt.b.w, goal < prev);
    let alpha = 1.0 - exp(-adapt.a.w * speed);
    return vec4<f32>(prev + (goal - prev) * alpha, 0.0, 0.0, 1.0);
}
