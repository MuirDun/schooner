//! Texture handles, GPU-side texture wrapper, and the built-in WHITE
//! 1×1 texel that materials with no `albedo_texture` bind by default.
//!
//! `TextureHandle` mirrors `MeshHandle`: an opaque `u32` newtype, key
//! into `TextureRegistry`. The WHITE built-in is the engine's "no
//! albedo texture" fallback — the shader is uniform across textured
//! and untextured surfaces; a Material without an albedo texture binds
//! WHITE and the per-fragment multiply is a no-op. The alternative
//! shape (a `has_texture` flag in the model uniform + a shader branch)
//! adds a uniform branch on every fragment with no real win.
//!
//! ## Why `Rgba8UnormSrgb`
//!
//! Authored 8-bit PNGs are sRGB-encoded — every standard art tool
//! (Krita, Photoshop, GIMP, Blender's image editor) writes sRGB by
//! default. Binding the texture as `Rgba8UnormSrgb` makes the GPU's
//! texture-unit perform the sRGB→linear conversion at sample time, so
//! the value reaching the forward shader is already in the linear
//! light space the Blinn–Phong path expects.
//!
//! Game 2B's character material tier introduces normal maps and spec
//! masks, which are *data* (not color) and must be `Rgba8Unorm` —
//! sampling sRGB-decoded values from a normal map would tilt every
//! normal toward the dark end. That distinction lands when the
//! character tier does; until then every loaded texture is albedo.

use wgpu::{
    Device, Extent3d, Origin3d, Queue, TexelCopyBufferLayout, TexelCopyTextureInfo, Texture,
    TextureAspect, TextureDescriptor, TextureDimension, TextureFormat, TextureUsages, TextureView,
    TextureViewDescriptor,
};

/// Opaque handle into the `TextureRegistry`.
///
/// One slot is reserved for the engine-owned WHITE built-in:
/// [`TextureHandle::WHITE`]. User-loaded textures start at higher
/// indices via the registry's allocator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TextureHandle(pub u32);

impl TextureHandle {
    /// 1×1 white texel. Materials with no `albedo_texture` bind this;
    /// the multiply into `Material.albedo` is then a no-op so the
    /// shader's textured and untextured paths stay uniform.
    pub const WHITE: Self = Self(0);

    /// 1×1 flat tangent-space normal `(0, 0, 1)`, encoded `(128, 128,
    /// 255)`. Materials with no `normal_texture` bind this; decoding
    /// `xyz * 2 - 1` yields `(0, 0, 1)` so the TBN rotate is the identity
    /// and the mapped/unmapped fragment paths stay uniform — the normal-
    /// map twin of [`TextureHandle::WHITE`]. Uploaded **linear**
    /// (`Rgba8Unorm`); a normal map must never go through the sRGB path.
    pub const FLAT_NORMAL: Self = Self(1);

    /// First handle a user-supplied texture may take. Registry's
    /// allocator starts here so built-in slots are never overwritten
    /// by a later load.
    pub const FIRST_USER: Self = Self(2);
}

/// CPU-side decoded texture: tightly packed RGBA8 pixels, dimensions,
/// ready for upload. Built by `asset::load_png_pixels` and by the
/// WHITE built-in's hardcoded constructor.
#[derive(Debug, Clone)]
pub struct TextureData {
    pub width: u32,
    pub height: u32,
    /// Tightly packed RGBA8 — length must equal `width * height * 4`.
    pub pixels: Vec<u8>,
}

impl TextureData {
    /// 1×1 white texel — the default-fallback the WHITE built-in
    /// uploads. Constructed dynamically because `Vec<u8>` is not
    /// const-constructable.
    pub fn white_1x1() -> Self {
        Self {
            width: 1,
            height: 1,
            pixels: vec![255, 255, 255, 255],
        }
    }

    /// 1×1 flat tangent-space normal — `(0, 0, 1)` encoded as
    /// `(128, 128, 255)`. The FLAT_NORMAL built-in uploads this through
    /// the *linear* path so the decode `xyz * 2 - 1` recovers `(0, 0, 1)`
    /// exactly. (128 decodes to ≈0.0039, not 0 — close enough that the
    /// perturbation is imperceptible; 127.5 isn't representable in u8.)
    pub fn flat_normal_1x1() -> Self {
        Self {
            width: 1,
            height: 1,
            pixels: vec![128, 128, 255, 255],
        }
    }
}

/// GPU-resident texture: the wgpu texture plus a default view ready
/// to bind in a sampled-texture slot.
///
/// The sampler is *not* held here — samplers are pipeline state and
/// live with the bind group that consumes them, so one sampler
/// shared across all material textures avoids per-texture allocations
/// and keeps state-switch cost on material changes at one bind-group
/// rebind, not two.
#[derive(Debug)]
pub struct TextureGpu {
    pub texture: Texture,
    pub view: TextureView,
}

impl TextureGpu {
    /// Upload `data` as a single-mip `Rgba8UnormSrgb` 2D texture — the
    /// **color** path. See the module docs for the sRGB-format rationale:
    /// authored albedo is sRGB-encoded, so this format makes the texture
    /// unit linearize at sample time.
    ///
    /// Mip generation is deferred to Game 2A's asset-pipeline
    /// maturation — runtime mip downsampling is its own subsystem,
    /// and authored KTX2 with precomputed mips is the longer-term
    /// answer. Single-mip is correct for indoor Kinesis scenes where
    /// every textured surface is viewed at near-1:1 sampling.
    pub fn upload_rgba8(device: &Device, queue: &Queue, label: &str, data: &TextureData) -> Self {
        Self::upload_with_format(device, queue, label, data, TextureFormat::Rgba8UnormSrgb)
    }

    /// Upload `data` as a single-mip `Rgba8Unorm` 2D texture — the
    /// **data** path. Normal maps, spec/roughness masks, and any other
    /// non-color texture must use this: their bytes are not sRGB-encoded,
    /// so routing them through `upload_rgba8` would have the texture unit
    /// apply a spurious sRGB→linear curve, tilting every sampled value
    /// (a flat normal `(128,128,255)` would read low and bend lighting).
    pub fn upload_rgba8_linear(
        device: &Device,
        queue: &Queue,
        label: &str,
        data: &TextureData,
    ) -> Self {
        Self::upload_with_format(device, queue, label, data, TextureFormat::Rgba8Unorm)
    }

    fn upload_with_format(
        device: &Device,
        queue: &Queue,
        label: &str,
        data: &TextureData,
        format: TextureFormat,
    ) -> Self {
        let size = Extent3d {
            width: data.width,
            height: data.height,
            depth_or_array_layers: 1,
        };
        let texture = device.create_texture(&TextureDescriptor {
            label: Some(label),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format,
            usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
            view_formats: &[],
        });
        // `queue.write_texture` (unlike `copy_buffer_to_texture`) has
        // no 256-byte row-alignment requirement — wgpu stages the
        // upload through an internal aligned buffer. That's why a 1×1
        // WHITE upload with `bytes_per_row = 4` is fine.
        queue.write_texture(
            TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: Origin3d::ZERO,
                aspect: TextureAspect::All,
            },
            &data.pixels,
            TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(data.width * 4),
                rows_per_image: Some(data.height),
            },
            size,
        );
        let view = texture.create_view(&TextureViewDescriptor::default());
        Self { texture, view }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn white_builtin_is_handle_zero() {
        assert_eq!(TextureHandle::WHITE.0, 0);
        assert!(TextureHandle::WHITE.0 < TextureHandle::FIRST_USER.0);
    }

    #[test]
    fn first_user_skips_builtin_slots() {
        // WHITE (0) and FLAT_NORMAL (1) are reserved; users start at 2.
        assert_eq!(TextureHandle::WHITE.0, 0);
        assert_eq!(TextureHandle::FLAT_NORMAL.0, 1);
        assert_eq!(TextureHandle::FIRST_USER.0, 2);
    }

    #[test]
    fn flat_normal_1x1_decodes_to_up() {
        let data = TextureData::flat_normal_1x1();
        assert_eq!(data.width, 1);
        assert_eq!(data.height, 1);
        // (128, 128, 255) ≈ (0, 0, 1) after `xyz * 2 - 1` in the shader.
        assert_eq!(data.pixels, vec![128, 128, 255, 255]);
    }

    #[test]
    fn white_1x1_is_opaque_white_rgba() {
        let data = TextureData::white_1x1();
        assert_eq!(data.width, 1);
        assert_eq!(data.height, 1);
        assert_eq!(data.pixels, vec![255, 255, 255, 255]);
    }
}
