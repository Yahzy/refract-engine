//! Render-agnostic triangle meshes in map space.

/// Triangles that share one material.
#[derive(Debug, Default, Clone)]
pub struct MeshBatch {
    /// Material path without `materials/` and `.vmt`.
    pub material: String,
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    /// The UVs are in texels and must be divided by the material's mapping
    /// size ([`crate::material::mapping_size`]).
    pub texel_uvs: bool,
    /// Per-vertex blend between the material's first and second texture
    /// (displacement alpha, 0..1). Empty for meshes without blending.
    pub blend: Vec<f32>,
    pub indices: Vec<u32>,
}

impl MeshBatch {
    /// Adds `other`'s triangles to this batch (same material).
    pub fn append(&mut self, other: &MeshBatch) {
        let base = self.positions.len() as u32;
        if self.blend.len() != self.positions.len() || other.blend.len() != other.positions.len() {
            // Only keep blend factors if both sides have them.
            self.blend.clear();
        } else {
            self.blend.extend_from_slice(&other.blend);
        }
        self.positions.extend_from_slice(&other.positions);
        self.normals.extend_from_slice(&other.normals);
        self.uvs.extend_from_slice(&other.uvs);
        self.indices.extend(other.indices.iter().map(|i| i + base));
    }
}
