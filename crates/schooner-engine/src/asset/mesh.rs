//! glTF mesh import.
//!
//! Loads the **first primitive of the first mesh** from a `.gltf` or
//! `.glb` file. Multi-primitive, multi-mesh, and scene-graph loading
//! are explicit non-goals for Game 1 — those land in Game 2A as part
//! of the level/scene loader. The single-primitive shape matches
//! Blender's default "export selected object as glTF" workflow, which
//! is the authoring path Step 1.F.5 walks.

use std::path::Path;

use crate::asset::{AssetError, AssetResult};
use crate::render::mesh::{MeshData, Vertex};

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

    let mesh = document.meshes().next().ok_or_else(|| AssetError::NoMesh {
        path: path.to_path_buf(),
    })?;

    let primitive = mesh
        .primitives()
        .next()
        .ok_or_else(|| AssetError::NoPrimitive {
            path: path.to_path_buf(),
        })?;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_surfaces_as_gltf_error() {
        let err = load_gltf_mesh(Path::new("__schooner_does_not_exist__.gltf")).unwrap_err();
        assert!(matches!(err, AssetError::Gltf { .. }));
    }
}
