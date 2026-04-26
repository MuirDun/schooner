//! Mesh handles, vertex layout, GPU-side mesh wrapper, and the
//! procedural cube + plane the engine ships with.
//!
//! `MeshHandle` is the opaque key into the `MeshRegistry`. Stays
//! `Copy` and 4-byte so component storage is cheap and queries
//! over `(Transform, MeshHandle)` carry no indirection.
//!
//! The engine reserves the two lowest values for built-in meshes
//! every later game will want as test rigs (procedural debug spawns,
//! physics demos, scripting REPL) — see `architecture/render.md`
//! "What the renderer owns". User-defined meshes start at higher
//! indices via the registry's allocator.
//!
//! `Vertex` is the canonical vertex layout for Game 0: position +
//! normal + uv, all `f32`. The forward pipeline binds a buffer of
//! these and the WGSL vertex shader reads matching `@location`s.
//! Procedural builders return CPU-side `(Vec<Vertex>, Vec<u32>)`;
//! `MeshGpu::upload` turns that into vertex/index buffers on the
//! device.

use bytemuck::{Pod, Zeroable};
use wgpu::util::{BufferInitDescriptor, DeviceExt};
use wgpu::{Buffer, BufferUsages, Device, VertexAttribute, VertexBufferLayout, VertexStepMode};

/// Canonical vertex layout for the forward pipeline.
///
/// `position` is in mesh-local space; the model matrix transforms
/// it into world space in the vertex shader. `normal` is the
/// surface normal in mesh-local space; the shader transforms it
/// using the model matrix's normal sub-matrix (or its inverse
/// transpose for non-uniform scale, but Game 0 only uses uniform
/// scale on the cube/plane built-ins).
///
/// `uv` is unused by the Blinn–Phong shader for Game 0 — there is
/// no texture sampling yet — but the slot is reserved in the
/// vertex layout so adding the first texture in Game 1 is a
/// shader change, not a layout change.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct Vertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
}

impl Vertex {
    /// `wgpu::VertexBufferLayout` matching the field order.
    /// Locations are `@location(0)` position, `@location(1)`
    /// normal, `@location(2)` uv in WGSL.
    pub const LAYOUT: VertexBufferLayout<'static> = VertexBufferLayout {
        array_stride: std::mem::size_of::<Self>() as u64,
        step_mode: VertexStepMode::Vertex,
        attributes: &[
            VertexAttribute {
                offset: 0,
                shader_location: 0,
                format: wgpu::VertexFormat::Float32x3,
            },
            VertexAttribute {
                offset: 12, // 3 × f32
                shader_location: 1,
                format: wgpu::VertexFormat::Float32x3,
            },
            VertexAttribute {
                offset: 24, // 6 × f32
                shader_location: 2,
                format: wgpu::VertexFormat::Float32x2,
            },
        ],
    };
}

/// CPU-side mesh: a vertex/index pair ready to upload. Built by
/// the procedural generators below and consumed by `MeshGpu::upload`.
#[derive(Debug, Clone)]
pub struct MeshData {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
}

/// GPU-resident mesh: a vertex buffer, an index buffer, and the
/// number of indices to draw. Construction goes through
/// `MeshGpu::upload` so the buffers are always created with the
/// right usage flags.
#[derive(Debug)]
pub struct MeshGpu {
    pub vertex_buffer: Buffer,
    pub index_buffer: Buffer,
    pub index_count: u32,
}

impl MeshGpu {
    /// Upload a `MeshData` to the device. Buffers are created with
    /// `VERTEX | COPY_DST` and `INDEX | COPY_DST` so a future asset
    /// pipeline can stream updates into them via `Queue::write_buffer`
    /// without recreating the buffer.
    pub fn upload(device: &Device, label: &str, data: &MeshData) -> Self {
        let vertex_buffer = device.create_buffer_init(&BufferInitDescriptor {
            label: Some(&format!("{label}-vertices")),
            contents: bytemuck::cast_slice(&data.vertices),
            usage: BufferUsages::VERTEX | BufferUsages::COPY_DST,
        });
        let index_buffer = device.create_buffer_init(&BufferInitDescriptor {
            label: Some(&format!("{label}-indices")),
            contents: bytemuck::cast_slice(&data.indices),
            usage: BufferUsages::INDEX | BufferUsages::COPY_DST,
        });
        Self {
            vertex_buffer,
            index_buffer,
            index_count: data.indices.len() as u32,
        }
    }
}

/// Opaque handle into the `MeshRegistry`.
///
/// Two slots are reserved for engine-owned built-ins:
/// [`MeshHandle::CUBE`] and [`MeshHandle::PLANE`]. They are the
/// canonical "is the renderer alive?" primitives — every later
/// game gets them for free without re-deriving the vertex arrays.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MeshHandle(pub u32);

impl MeshHandle {
    /// Built-in unit cube centered at the origin, edges aligned to
    /// world axes. Registered eagerly during render init.
    pub const CUBE: Self = Self(0);

    /// Built-in unit plane on the Y=0 plane, normal +Y. Registered
    /// eagerly during render init.
    pub const PLANE: Self = Self(1);

    /// First handle a user-supplied mesh may take. Registry's
    /// allocator starts here so built-in slots are never overwritten
    /// by a later `insert`.
    pub const FIRST_USER: Self = Self(2);
}

/// Built-in unit cube: ±0.5 on each axis, six faces, one normal
/// per face.
///
/// Vertices are **not** shared across faces — each of the six
/// quads has four dedicated vertices with the face normal. Sharing
/// across faces would interpolate normals across the cube edges
/// and the shading would round the cube into a sphere. 24
/// vertices for a cube is the cost of correct flat-shaded faces.
pub fn cube_mesh() -> MeshData {
    // Per-face quad: four corners listed CCW when viewed from
    // outside, with the face normal applied to all four. Indices
    // are emitted as two triangles per face.
    fn face(
        verts: &mut Vec<Vertex>,
        indices: &mut Vec<u32>,
        corners: [[f32; 3]; 4],
        normal: [f32; 3],
    ) {
        let base = verts.len() as u32;
        let uvs = [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]];
        for (corner, uv) in corners.iter().zip(uvs.iter()) {
            verts.push(Vertex {
                position: *corner,
                normal,
                uv: *uv,
            });
        }
        // CCW front-face: 0-1-2 and 0-2-3.
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    let mut verts = Vec::with_capacity(24);
    let mut indices = Vec::with_capacity(36);

    // Each face is listed CCW when viewed from outside the cube
    // (from the side the normal points to). The triangulation
    // (0-1-2, 0-2-3) keeps that winding.
    //
    // +Z (front, normal +Z, viewed from +Z toward origin):
    // bottom-left → bottom-right → top-right → top-left.
    face(
        &mut verts,
        &mut indices,
        [
            [-0.5, -0.5, 0.5],
            [0.5, -0.5, 0.5],
            [0.5, 0.5, 0.5],
            [-0.5, 0.5, 0.5],
        ],
        [0.0, 0.0, 1.0],
    );
    // -Z (back, normal -Z, viewed from -Z toward origin):
    // local "right" along -Z view is +X-on-screen → world -X.
    face(
        &mut verts,
        &mut indices,
        [
            [0.5, -0.5, -0.5],
            [-0.5, -0.5, -0.5],
            [-0.5, 0.5, -0.5],
            [0.5, 0.5, -0.5],
        ],
        [0.0, 0.0, -1.0],
    );
    // +X (right, normal +X, viewed from +X toward origin):
    // local "right" is +Z-axis-toward-viewer's-left → world -Z.
    face(
        &mut verts,
        &mut indices,
        [
            [0.5, -0.5, 0.5],
            [0.5, -0.5, -0.5],
            [0.5, 0.5, -0.5],
            [0.5, 0.5, 0.5],
        ],
        [1.0, 0.0, 0.0],
    );
    // -X (left, normal -X, viewed from -X toward origin):
    // local "right" → world +Z.
    face(
        &mut verts,
        &mut indices,
        [
            [-0.5, -0.5, -0.5],
            [-0.5, -0.5, 0.5],
            [-0.5, 0.5, 0.5],
            [-0.5, 0.5, -0.5],
        ],
        [-1.0, 0.0, 0.0],
    );
    // +Y (top, normal +Y, viewed from above looking down):
    // bottom-left along +Z (toward viewer's bottom of screen).
    face(
        &mut verts,
        &mut indices,
        [
            [-0.5, 0.5, 0.5],
            [0.5, 0.5, 0.5],
            [0.5, 0.5, -0.5],
            [-0.5, 0.5, -0.5],
        ],
        [0.0, 1.0, 0.0],
    );
    // -Y (bottom, normal -Y, viewed from below looking up):
    // bottom-left along -Z.
    face(
        &mut verts,
        &mut indices,
        [
            [-0.5, -0.5, -0.5],
            [0.5, -0.5, -0.5],
            [0.5, -0.5, 0.5],
            [-0.5, -0.5, 0.5],
        ],
        [0.0, -1.0, 0.0],
    );

    MeshData {
        vertices: verts,
        indices,
    }
}

/// Built-in 1×1 plane on Y=0, normal +Y, centered on the origin.
///
/// CCW winding viewed from above so the plane survives back-face
/// culling when looked at from +Y. To make a "floor" big enough to
/// stand on, the consumer scales the entity's `Transform` rather
/// than the mesh — keeps the built-in mesh canonical and lets
/// users tile it later.
pub fn plane_mesh() -> MeshData {
    let normal = [0.0, 1.0, 0.0];
    let vertices = vec![
        Vertex { position: [-0.5, 0.0, 0.5], normal, uv: [0.0, 1.0] },
        Vertex { position: [0.5, 0.0, 0.5], normal, uv: [1.0, 1.0] },
        Vertex { position: [0.5, 0.0, -0.5], normal, uv: [1.0, 0.0] },
        Vertex { position: [-0.5, 0.0, -0.5], normal, uv: [0.0, 0.0] },
    ];
    let indices = vec![0, 1, 2, 0, 2, 3];
    MeshData { vertices, indices }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_handles_are_distinct_and_lowest() {
        assert_ne!(MeshHandle::CUBE, MeshHandle::PLANE);
        assert!(MeshHandle::CUBE.0 < MeshHandle::FIRST_USER.0);
        assert!(MeshHandle::PLANE.0 < MeshHandle::FIRST_USER.0);
    }

    #[test]
    fn first_user_skips_builtin_slots() {
        assert_eq!(MeshHandle::FIRST_USER.0, 2);
    }

    #[test]
    fn vertex_layout_stride_matches_struct_size() {
        // If the layout's stride doesn't match the struct, the
        // shader reads the wrong bytes. This is a load-bearing
        // assertion the rest of Game 0 silently depends on.
        assert_eq!(
            Vertex::LAYOUT.array_stride as usize,
            std::mem::size_of::<Vertex>()
        );
    }

    #[test]
    fn cube_has_24_vertices_and_36_indices() {
        // Six faces × four unique vertices = 24. Six faces × two
        // triangles × three indices = 36. Sharing vertices across
        // faces would drop the vertex count to 8 but break per-face
        // shading.
        let m = cube_mesh();
        assert_eq!(m.vertices.len(), 24);
        assert_eq!(m.indices.len(), 36);
    }

    #[test]
    fn cube_normals_are_unit_axis_aligned() {
        // Each of the six faces should use one of the six unit
        // axis-aligned normals, with four vertices per normal.
        let m = cube_mesh();
        let mut counts = std::collections::HashMap::<[i32; 3], u32>::new();
        for v in &m.vertices {
            // Round to integers — flat normals are exactly ±1.
            let key = [
                v.normal[0] as i32,
                v.normal[1] as i32,
                v.normal[2] as i32,
            ];
            *counts.entry(key).or_default() += 1;
        }
        assert_eq!(counts.len(), 6, "expected six distinct face normals");
        for (_, c) in counts {
            assert_eq!(c, 4, "expected four vertices per face");
        }
    }

    #[test]
    fn plane_normal_points_up_and_is_a_quad() {
        let m = plane_mesh();
        assert_eq!(m.vertices.len(), 4);
        assert_eq!(m.indices.len(), 6);
        for v in &m.vertices {
            assert_eq!(v.normal, [0.0, 1.0, 0.0]);
        }
    }

    /// The geometric normal of a CCW triangle (a,b,c) is
    /// `(b-a) × (c-a)` normalized. If the per-face authored normal
    /// agrees with this for every triangle, then "CCW from outside"
    /// holds and back-face culling drops the inside.
    fn triangle_geometric_normal(a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> [f32; 3] {
        let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let ac = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
        let n = [
            ab[1] * ac[2] - ab[2] * ac[1],
            ab[2] * ac[0] - ab[0] * ac[2],
            ab[0] * ac[1] - ab[1] * ac[0],
        ];
        let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        [n[0] / len, n[1] / len, n[2] / len]
    }

    #[test]
    fn cube_winding_matches_authored_normals() {
        // Catches the class of bug where a face's CCW corner order
        // produces a geometric normal pointing opposite to the
        // authored normal — i.e. the face is wound inside-out and
        // back-face culling would drop the visible side.
        let m = cube_mesh();
        for tri in m.indices.chunks_exact(3) {
            let a = m.vertices[tri[0] as usize];
            let b = m.vertices[tri[1] as usize];
            let c = m.vertices[tri[2] as usize];
            let geom = triangle_geometric_normal(a.position, b.position, c.position);
            let authored = a.normal;
            // Same direction: dot product close to +1.
            let dot = geom[0] * authored[0] + geom[1] * authored[1] + geom[2] * authored[2];
            assert!(
                dot > 0.99,
                "triangle {tri:?} winds inside-out: geometric {geom:?} vs authored {authored:?}",
            );
        }
    }

    #[test]
    fn plane_winding_matches_normal() {
        let m = plane_mesh();
        for tri in m.indices.chunks_exact(3) {
            let a = m.vertices[tri[0] as usize];
            let b = m.vertices[tri[1] as usize];
            let c = m.vertices[tri[2] as usize];
            let geom = triangle_geometric_normal(a.position, b.position, c.position);
            let authored = a.normal;
            let dot = geom[0] * authored[0] + geom[1] * authored[1] + geom[2] * authored[2];
            assert!(dot > 0.99, "plane wound inside-out: {geom:?} vs {authored:?}");
        }
    }
}
