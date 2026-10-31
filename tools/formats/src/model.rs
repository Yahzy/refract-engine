//! Studio models (MDL + VVD + VTX).

use anyhow::{Context, Result};
use vmdl::{Mdl, Vtx, Vvd};

use crate::fs::FileSystem;
use crate::mesh::MeshBatch;
use crate::path::normalize;

#[derive(Debug, Clone)]
pub struct ModelData {
    /// Normalized path, e.g. `models/props/barrel01.mdl`.
    pub path: String,
    /// One batch per material.
    pub meshes: Vec<MeshBatch>,
    /// Bounding box (mins, maxs) in world units.
    pub bounds: ([f32; 3], [f32; 3]),
}

/// Loads a model with the given skin. Only LOD 0 is used.
pub fn load_model(fs: &FileSystem, path: &str, skin: usize) -> Result<ModelData> {
    let path = normalize(path);
    let base = path.strip_suffix(".mdl").unwrap_or(&path);
    let read = |p: String| fs.read(&p, None).with_context(|| format!("{p} not found"));
    let mdl = Mdl::read(&read(format!("{base}.mdl"))?).with_context(|| format!("parsing {base}.mdl"))?;
    let vvd = Vvd::read(&read(format!("{base}.vvd"))?).with_context(|| format!("parsing {base}.vvd"))?;
    let vtx = Vtx::read(&read(format!("{base}.dx90.vtx"))?).with_context(|| format!("parsing {base}.dx90.vtx"))?;
    let model = vmdl::Model::from_parts(mdl, vtx, vvd);

    let skin_table = model.skin_tables().nth(skin).or_else(|| model.skin_tables().next());
    let directories = model.texture_directories();
    let mut meshes: Vec<MeshBatch> = Vec::new();

    for mesh in model.meshes() {
        let texture = skin_table.as_ref().and_then(|t| t.texture(mesh.material_index())).unwrap_or("");
        let material = resolve_material(fs, directories, texture);
        let batch = match meshes.iter().position(|b| b.material == material) {
            Some(i) => &mut meshes[i],
            None => {
                meshes.push(MeshBatch { material, ..Default::default() });
                meshes.last_mut().unwrap()
            }
        };
        // VTX strips are already expanded to counter-clockwise triangle lists.
        for vertex in mesh.vertices() {
            batch.indices.push(batch.positions.len() as u32);
            batch.positions.push([vertex.position.x, vertex.position.y, vertex.position.z]);
            batch.normals.push([vertex.normal.x, vertex.normal.y, vertex.normal.z]);
            batch.uvs.push(vertex.texture_coordinates);
        }
    }

    let (mins, maxs) = model.bounding_box();
    Ok(ModelData {
        path,
        meshes: meshes.into_iter().filter(|m| !m.indices.is_empty()).collect(),
        bounds: ([mins.x, mins.y, mins.z], [maxs.x, maxs.y, maxs.z]),
    })
}

/// Finds `<cdmaterials dir><texture>` among the model's material directories,
/// in order.
fn resolve_material(fs: &FileSystem, directories: &[String], texture: &str) -> String {
    let candidates: Vec<String> = directories.iter().map(|dir| normalize(&format!("{dir}/{texture}"))).collect();
    candidates
        .iter()
        .find(|c| fs.exists(&format!("materials/{c}.vmt"), None))
        .or(candidates.first())
        .cloned()
        .unwrap_or_else(|| normalize(texture))
}
