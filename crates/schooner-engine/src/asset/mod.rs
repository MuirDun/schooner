//! On-disk asset loaders for meshes and textures.
//!
//! Lives next to `render/` rather than inside it because assets are a
//! layer *above* the renderer in the architecture: the asset module
//! depends on render types (`MeshData`, the eventual `Texture` types)
//! but render code never depends on asset code. Render owns the GPU
//! handles; asset owns the parse + upload pipeline that fills them.

use std::path::PathBuf;

pub mod mesh;
pub mod texture;

#[cfg(feature = "dev-tools")]
pub mod debug;

pub use mesh::{GltfModel, load_gltf_mesh, load_gltf_model};
pub use texture::load_png_pixels;

#[cfg(feature = "dev-tools")]
pub use debug::{AssetDebugPlugin, AssetDebugState, ReloadSummary};

/// All named failure modes the asset module surfaces.
///
/// Failures here are non-fatal at the engine level — the reload story
/// is "previous version keeps running, error logged" — but each
/// variant carries enough context that a `log::warn!` of the
/// displayed error tells the developer exactly which file to fix.
#[derive(Debug, thiserror::Error)]
pub enum AssetError {
    /// `gltf::import` failed: file missing, malformed JSON, broken
    /// binary chunk, or referenced external buffer/image unreadable.
    #[error("failed to import glTF from {path}: {source}")]
    Gltf {
        path: PathBuf,
        #[source]
        source: gltf::Error,
    },

    /// The glTF parsed but contained no meshes. A scene with only
    /// lights or cameras lands here.
    #[error("glTF file {path} contains no meshes")]
    NoMesh { path: PathBuf },

    /// The first mesh had no primitives.
    #[error("glTF mesh in {path} has no primitives")]
    NoPrimitive { path: PathBuf },

    /// glTF allows non-indexed meshes; we don't. Modern Blender export
    /// always emits indices, and the cube/plane built-ins are indexed
    /// — keeping the draw path uniform avoids a per-mesh branch.
    #[error("glTF primitive in {path} is missing indices")]
    MissingIndices { path: PathBuf },

    /// Position attribute missing. Unambiguously a broken asset.
    #[error("glTF primitive in {path} is missing position attribute")]
    MissingPositions { path: PathBuf },

    /// Normal attribute missing. Enforced because the forward shader's
    /// Blinn–Phong path reads world-space normals; silently generating
    /// flat normals here would mask a wrong Blender export setting.
    #[error("glTF primitive in {path} is missing normal attribute")]
    MissingNormals { path: PathBuf },

    /// Position / normal / UV counts disagree — malformed accessors
    /// in the source file.
    #[error("glTF primitive in {path} has mismatched attribute counts")]
    AttributeMismatch { path: PathBuf },

    /// `image::open` failed — file missing, malformed PNG, unsupported
    /// bit depth, or (with the PNG-only feature set) a non-PNG file.
    #[error("failed to load PNG from {path}: {source}")]
    Png {
        path: PathBuf,
        #[source]
        source: image::ImageError,
    },

    /// A glTF-embedded image decoded to a pixel format the upload path
    /// can't take (the 16-bit-per-channel variants). Not a parse failure
    /// — a wrong export. RGBA8 / RGB8 / grayscale are the supported set.
    #[error("glTF image in {path} has unsupported pixel format {format:?}")]
    UnsupportedImageFormat {
        path: PathBuf,
        format: gltf::image::Format,
    },
}

pub type AssetResult<T> = Result<T, AssetError>;
