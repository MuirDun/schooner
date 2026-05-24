//! PNG decode into the engine's `TextureData` shape.
//!
//! KTX2 / Basis transcoding is deferred to Game 2A's asset-pipeline
//! maturation. PNG is sufficient for the manual-reload smoke test
//! (Step 1.F.5) and for Kinesis's full art kit: every Kinesis surface
//! texture is small enough that PNG's slower decode and lack of
//! GPU-friendly compression aren't bottlenecks at indoor scale.

use std::path::Path;

use crate::asset::{AssetError, AssetResult};
use crate::render::texture::TextureData;

/// Decode `path` (an 8-bit PNG) into RGBA8 pixels. Caller uploads with
/// `TextureGpu::upload_rgba8` — typically via
/// `TextureRegistry::load_png`, which threads the path through to the
/// registry's source tracking for the F5 reload story.
///
/// Lower-bit-depth PNGs (greyscale, 1-bit, palette) are auto-expanded
/// by the `image` crate to 8-bit RGBA, which is what the upload path
/// expects. Higher-bit-depth PNGs (16-bit) are downsampled to 8-bit
/// at decode time — fine for albedo, would be wrong for HDR data
/// (which Kinesis does not author).
pub fn load_png_pixels(path: &Path) -> AssetResult<TextureData> {
    let img = image::open(path).map_err(|source| AssetError::Png {
        path: path.to_path_buf(),
        source,
    })?;
    let rgba = img.to_rgba8();
    let (width, height) = rgba.dimensions();
    Ok(TextureData {
        width,
        height,
        pixels: rgba.into_raw(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_surfaces_as_png_error() {
        let err = load_png_pixels(Path::new("__schooner_does_not_exist__.png")).unwrap_err();
        assert!(matches!(err, AssetError::Png { .. }));
    }
}
