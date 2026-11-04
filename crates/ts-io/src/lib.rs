//! ThreadSmith file import.
//!
//! - glTF / GLB through the `gltf` crate
//! - FBX and OBJ through `ufbx`
//!
//! Everything comes out in ThreadSmith space: Y up, right handed, meters.

use std::path::Path;

use anyhow::{bail, Result};
use ts_core::Scene;

mod fbx;
mod gltf_import;

/// File types the importer understands (lower case, no dot).
pub const SUPPORTED_EXTENSIONS: &[&str] = &["fbx", "glb", "gltf", "obj"];

/// Load a model file into a [`Scene`].
pub fn load(path: &Path) -> Result<Scene> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    let mut scene = match ext.as_str() {
        "glb" | "gltf" => gltf_import::load(path)?,
        "fbx" | "obj" => fbx::load(path)?,
        other => bail!(
            "Can't open .{other} files yet. Supported: {}",
            SUPPORTED_EXTENSIONS.join(", ")
        ),
    };
    if scene.name.is_empty() {
        scene.name = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
    }
    for part in &mut scene.parts {
        if part.mesh.normals.len() != part.mesh.positions.len() {
            part.mesh.compute_normals();
        }
        if let Some(skin) = &mut part.mesh.skin {
            skin.normalize();
        }
    }
    log::info!(
        "Loaded {}: {} parts, {} triangles, {} joints",
        path.display(),
        scene.parts.len(),
        scene.triangle_count(),
        scene.skeleton.len()
    );
    Ok(scene)
}

/// Put joints in parents-first order. `parents[i]` is the parent of joint `i`
/// in the incoming order. Returns the new order (list of old indices).
pub(crate) fn parents_first_order(parents: &[Option<usize>]) -> Vec<usize> {
    let depth = |mut i: usize| {
        let mut d = 0;
        while let Some(p) = parents[i] {
            i = p;
            d += 1;
            if d > parents.len() {
                break; // broken hierarchy (a loop); stop instead of hanging
            }
        }
        d
    };
    let mut order: Vec<usize> = (0..parents.len()).collect();
    order.sort_by_key(|&i| depth(i)); // stable sort keeps file order inside a depth
    order
}
