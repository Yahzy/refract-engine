//! Models: MDL (with its VVD and VTX) to `.rmdl`, converting their materials.

use refract_asset::geometry::GeometryData;
use refract_formats::model;

use crate::{Converter, geometry, model_path};

impl Converter {
    /// Converts a model with one of its skins; returns its native path, or
    /// `None` when the model cannot be read.
    pub(crate) fn model(&self, path: &str, skin: usize) -> Option<String> {
        let out = model_path(path, skin);
        if !self.claim(&out) {
            return Some(out);
        }
        let model = match model::load_model(&self.fs, path, skin) {
            Ok(model) => model,
            Err(err) => {
                log::warn!("model {path}: {err:#}");
                return None;
            }
        };
        let mut data = GeometryData::default();
        for batch in &model.meshes {
            let material = self.material(&batch.material);
            geometry::append(&mut data, batch, &material, [1.0, 1.0], 0);
        }
        match self.write(&out, &refract_asset::model::write(&data)) {
            Ok(()) => Some(out),
            Err(err) => {
                log::warn!("{err:#}");
                None
            }
        }
    }
}
