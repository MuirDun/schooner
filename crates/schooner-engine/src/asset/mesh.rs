//! glTF mesh import.
//!
//! Loads the **first primitive of the first mesh** from a `.gltf` or
//! `.glb` file. Multi-primitive, multi-mesh, and scene-graph loading
//! are explicit non-goals for Game 1 — those land in Game 2A as part
//! of the level/scene loader. The single-primitive shape matches
//! Blender's default "export selected object as glTF" workflow, which
//! is the authoring path Step 1.F.5 walks.

use std::path::Path;

use gltf::image::Format;

use crate::asset::{AssetError, AssetResult};
use crate::render::mesh::{MeshData, Vertex};
use crate::render::texture::TextureData;

/// A glTF imported as a single drawable: the first primitive's geometry
/// plus its material's decoded base-color image, if it has one.
///
/// Both fields are CPU-side and registry-agnostic on purpose — uploading
/// the mesh and texture touches two *different* registries, and `World`
/// only lends one `&mut` resource at a time, so the parse stays a pure
/// value and the caller does the two uploads sequentially. The base-color
/// image rides along here instead of forcing the caller to also name a
/// loose `TextureAsset` for a texture that already lives inside the glb.
#[derive(Debug, Clone)]
pub struct GltfModel {
    pub mesh: MeshData,
    /// Decoded RGBA8 base-color (albedo) texture, `None` when the
    /// primitive's material has no base-color texture bound.
    pub albedo: Option<TextureData>,
}

/// Parse `path` and return the CPU-side mesh. Caller uploads with
/// `MeshGpu::upload` — typically via `MeshRegistry::load_gltf`, which
/// threads the path through to the registry's source tracking for
/// the F5 reload story.
///
/// Missing UVs are tolerated (filled with `[0, 0]` per vertex) so
/// assets authored before textures land in Step 1.F.2 still load;
/// missing positions, normals, or indices are hard errors so a wrong
/// Blender export setting doesn't masquerade as a working asset.
pub fn load_gltf_mesh(path: &Path) -> AssetResult<MeshData> {
    let (document, buffers, _images) =
        gltf::import(path).map_err(|source| AssetError::Gltf {
            path: path.to_path_buf(),
            source,
        })?;

    let primitive = first_primitive(&document, path)?;
    read_primitive_mesh(&primitive, &buffers, path)
}

/// Parse `path` into a [`GltfModel`]: the first primitive's geometry and
/// the decoded base-color texture its material references, if any.
///
/// One `gltf::import` covers both — geometry and the embedded image come
/// out of the same parse. The base-color texture is read as sRGB color
/// data (the caller uploads it `Rgba8UnormSrgb`); a glb that embeds no
/// base-color texture yields `albedo: None` and the caller falls back to
/// the engine WHITE built-in. Material *factors* (base-color tint,
/// roughness, emissive) are intentionally not read here — the call site
/// authors those on the `Material` component; this loader only carries
/// what can't be hand-authored, the bitmap itself.
pub fn load_gltf_model(path: &Path) -> AssetResult<GltfModel> {
    let (document, buffers, images) = gltf::import(path).map_err(|source| AssetError::Gltf {
        path: path.to_path_buf(),
        source,
    })?;

    let primitive = first_primitive(&document, path)?;
    let mesh = read_primitive_mesh(&primitive, &buffers, path)?;

    // material → base-color texture → source image index → decoded pixels.
    let albedo = primitive
        .material()
        .pbr_metallic_roughness()
        .base_color_texture()
        .map(|info| image_to_rgba8(&images[info.texture().source().index()], path))
        .transpose()?;

    Ok(GltfModel { mesh, albedo })
}

fn first_primitive<'a>(
    document: &'a gltf::Document,
    path: &Path,
) -> AssetResult<gltf::Primitive<'a>> {
    document
        .meshes()
        .next()
        .ok_or_else(|| AssetError::NoMesh {
            path: path.to_path_buf(),
        })?
        .primitives()
        .next()
        .ok_or_else(|| AssetError::NoPrimitive {
            path: path.to_path_buf(),
        })
}

fn read_primitive_mesh(
    primitive: &gltf::Primitive,
    buffers: &[gltf::buffer::Data],
    path: &Path,
) -> AssetResult<MeshData> {
    let reader = primitive.reader(|buffer| Some(&buffers[buffer.index()]));

    let positions: Vec<[f32; 3]> = reader
        .read_positions()
        .ok_or_else(|| AssetError::MissingPositions {
            path: path.to_path_buf(),
        })?
        .collect();

    let normals: Vec<[f32; 3]> = reader
        .read_normals()
        .ok_or_else(|| AssetError::MissingNormals {
            path: path.to_path_buf(),
        })?
        .collect();

    let uvs: Vec<[f32; 2]> = reader
        .read_tex_coords(0)
        .map(|coords| coords.into_f32().collect())
        .unwrap_or_else(|| vec![[0.0, 0.0]; positions.len()]);

    if positions.len() != normals.len() || positions.len() != uvs.len() {
        return Err(AssetError::AttributeMismatch {
            path: path.to_path_buf(),
        });
    }

    let vertices: Vec<Vertex> = positions
        .iter()
        .zip(normals.iter())
        .zip(uvs.iter())
        .map(|((pos, normal), uv)| Vertex {
            position: *pos,
            normal: *normal,
            uv: *uv,
        })
        .collect();

    let indices: Vec<u32> = reader
        .read_indices()
        .ok_or_else(|| AssetError::MissingIndices {
            path: path.to_path_buf(),
        })?
        .into_u32()
        .collect();

    Ok(MeshData { vertices, indices })
}

/// Normalize a glTF-decoded image to tightly-packed RGBA8, the shape
/// `TextureGpu::upload_rgba8` expects. glTF hands back whatever channel
/// count the source PNG/JPEG had (Blender commonly exports `R8G8B8` with
/// no alpha), so 1- and 3-channel images are expanded and a missing
/// alpha is filled opaque. 16-bit formats are rejected rather than
/// silently truncated — Kinesis authors no HDR albedo, so a 16-bit image
/// here means a wrong export, not a valid asset (see `texture.rs`).
fn image_to_rgba8(img: &gltf::image::Data, path: &Path) -> AssetResult<TextureData> {
    let unsupported = || AssetError::UnsupportedImageFormat {
        path: path.to_path_buf(),
        format: img.format,
    };

    let pixels: Vec<u8> = match img.format {
        Format::R8G8B8A8 => img.pixels.clone(),
        Format::R8G8B8 => img
            .pixels
            .chunks_exact(3)
            .flat_map(|c| [c[0], c[1], c[2], 255])
            .collect(),
        // Single-channel: replicate across RGB (grayscale), opaque alpha.
        Format::R8 => img.pixels.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        // Two-channel: gray + alpha.
        Format::R8G8 => img
            .pixels
            .chunks_exact(2)
            .flat_map(|c| [c[0], c[0], c[0], c[1]])
            .collect(),
        Format::R16
        | Format::R16G16
        | Format::R16G16B16
        | Format::R16G16B16A16
        | Format::R32G32B32FLOAT
        | Format::R32G32B32A32FLOAT => return Err(unsupported()),
    };

    Ok(TextureData {
        width: img.width,
        height: img.height,
        pixels,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_surfaces_as_gltf_error() {
        let err = load_gltf_mesh(Path::new("__schooner_does_not_exist__.gltf")).unwrap_err();
        assert!(matches!(err, AssetError::Gltf { .. }));
    }
}
