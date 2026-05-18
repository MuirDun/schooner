// Post-process — the fixed pipeline that turns the forward pass's
// linear HDR output into a final LDR image ready for the swap chain.
//
// Stages, in order (1.D delivers them one at a time):
//   1. Tonemap        — HDR → LDR. Narkowicz ACES fit. 1.D.2.
//   2. Color grade    — ASC CDL (lift/gamma/gain), post-tonemap. 1.D.3.
//   3. Vignette       — radial darken + tint. 1.D.4.
//   4. Overlay        — texture × intensity, composited last. 1.D.5.
//
// Phase 1.E adds fog *in the forward shader*, not here — fog with
// per-fragment world-position awareness is cheaper in forward than
// after re-projecting depth in post.
//
// Bind groups:
//   @group(0) — HDR texture + linear-clamp sampler (the forward
//               pass's output, sampled at the fragment's screen UV).
//
// Drawn as a single fullscreen triangle: three vertices outside the
// screen so the bounding rect covers the viewport. UVs are derived
// from `vertex_index` — no vertex buffer needed. Avoids the diagonal
// seam a two-triangle quad would introduce.

@group(0) @binding(0) var hdr: texture_2d<f32>;
@group(0) @binding(1) var hdr_sampler: sampler;

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vid: u32) -> VertexOutput {
    // Standard fullscreen-triangle trick: vertex 0 at (-1,-1),
    // vertex 1 at ( 3,-1), vertex 2 at (-1, 3). The triangle's
    // bounding rect covers [-1, 1] × [-1, 1] (the whole NDC quad);
    // the rasterizer clips everything outside. UVs in [0, 1] are
    // derived by mapping NDC.x → u and (1 - NDC.y) → v (texture
    // +V is down, NDC +Y is up).
    let x = f32((vid << 1u) & 2u);       // vid 0,1,2 → 0, 2, 0
    let y = f32(vid & 2u);               // vid 0,1,2 → 0, 0, 2
    let pos = vec2<f32>(x * 2.0 - 1.0, y * 2.0 - 1.0);
    var out: VertexOutput;
    out.clip_position = vec4<f32>(pos, 0.0, 1.0);
    out.uv = vec2<f32>(x, 1.0 - y);
    return out;
}

// Narkowicz 2015 fitted ACES — a 5-MAD approximation of the full
// ACES RRT+ODT pipeline. The original (Reference Rendering Transform
// + Output Display Transform) is a multi-stage colour-space chain
// from the Academy Color Encoding System; the Narkowicz fit is the
// curve everyone (UE, Frostbite, indie engines) actually ships
// because it captures the *look* — soft shoulder on highlights,
// gentle toe in the shadows, slight saturation push — at a tiny
// fraction of the cost. Domain: HDR linear ≥ 0; range: ~[0, 1].
// Output is still linear; the swap chain's sRGB format does the
// gamma encode on present.
//
// Reference: https://knarkowicz.wordpress.com/2016/01/06/aces-filmic-tone-mapping-curve/
//
// Alternatives we're not using:
//   - Hill 2017 ACES fit. Slightly different shoulder, similar cost;
//     two pre/post matrix multiplies hug the official RRT+ODT closer
//     in chroma. Pick this if Narkowicz desaturates the look too much.
//   - Hable / Uncharted 2 filmic. Warmer feel; A/B/C/D/E/F/W constants
//     authored by Naughty Dog. Worth trying if "memory" wants more
//     midtone warmth than ACES naturally produces.
//   - Reinhard / Reinhard-Jodie. `x/(1+x)` — simpler, no real shoulder,
//     bright lights get muddy. Cheap but boring; not the look we want.
fn tonemap_aces(x: vec3<f32>) -> vec3<f32> {
    // Polynomial coefficients chosen by Narkowicz to fit the ACES
    // curve. Numerator and denominator are both quadratic in x; the
    // ratio shapes the toe (small x) and shoulder (large x) simultaneously.
    let a = 2.51;
    let b = 0.03;
    let c = 2.43;
    let d = 0.59;
    let e = 0.14;
    return clamp((x * (a * x + b)) / (x * (c * x + d) + e),
                 vec3<f32>(0.0), vec3<f32>(1.0));
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let hdr_color = textureSample(hdr, hdr_sampler, in.uv).rgb;

    // Tonemap is per-channel. Per-channel ACES is the industry-
    // default trade: chroma can shift slightly in saturated regions
    // (the brightest channel saturates first), but the alternative
    // — luminance-only tonemapping — desaturates highlights so hard
    // the world starts looking washed out. Game 2A+ may revisit if
    // the chroma shift hurts.
    let ldr = tonemap_aces(hdr_color);

    return vec4<f32>(ldr, 1.0);
}
