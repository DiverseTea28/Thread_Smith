//! GLB (binary glTF 2.0) export of skinned meshes with their skeleton.
//!
//! Joint names and hierarchy are written exactly as loaded, so a garment fitted
//! to a MetaHuman comes out with the MetaHuman's bone names and can be imported
//! in Unreal onto the existing body skeleton.

use std::path::Path;

use anyhow::{bail, Context, Result};
use glam::{Mat4, Vec3};
use serde_json::{json, Value};
use ts_core::{Material, Mesh, Skeleton, Texture, MAX_INFLUENCES};

// glTF constants.
const ARRAY_BUFFER: u32 = 34962;
const ELEMENT_ARRAY_BUFFER: u32 = 34963;
const FLOAT: u32 = 5126;
const UNSIGNED_SHORT: u32 = 5123;
const UNSIGNED_INT: u32 = 5125;

/// One mesh to export.
pub struct ExportMesh<'a> {
    pub name: &'a str,
    pub mesh: &'a Mesh,
    /// Base colour (linear RGB), used when there is no material.
    pub color: [f32; 3],
    /// The part's material (colour and textures), written into the file.
    pub material: Option<&'a Material>,
}

/// Collects binary data and the JSON entries that point into it.
#[derive(Default)]
struct Builder {
    bin: Vec<u8>,
    views: Vec<Value>,
    accessors: Vec<Value>,
}

impl Builder {
    fn push_view(&mut self, bytes: &[u8], target: Option<u32>) -> usize {
        while !self.bin.len().is_multiple_of(4) {
            self.bin.push(0);
        }
        let offset = self.bin.len();
        self.bin.extend_from_slice(bytes);
        let mut view = json!({ "buffer": 0, "byteOffset": offset, "byteLength": bytes.len() });
        if let Some(t) = target {
            view["target"] = json!(t);
        }
        self.views.push(view);
        self.views.len() - 1
    }

    fn accessor(
        &mut self,
        view: usize,
        component: u32,
        count: usize,
        kind: &str,
        extra: Option<(Value, Value)>,
    ) -> usize {
        let mut a =
            json!({ "bufferView": view, "componentType": component, "count": count, "type": kind });
        if let Some((min, max)) = extra {
            a["min"] = min;
            a["max"] = max;
        }
        self.accessors.push(a);
        self.accessors.len() - 1
    }

    fn vec3s(&mut self, data: &[Vec3], with_bounds: bool) -> usize {
        let flat: Vec<f32> = data.iter().flat_map(|v| v.to_array()).collect();
        let view = self.push_view(bytemuck_f32(&flat), Some(ARRAY_BUFFER));
        let bounds = with_bounds.then(|| {
            let min = data.iter().fold(Vec3::splat(f32::MAX), |a, b| a.min(*b));
            let max = data.iter().fold(Vec3::splat(f32::MIN), |a, b| a.max(*b));
            (json!(min.to_array()), json!(max.to_array()))
        });
        self.accessor(view, FLOAT, data.len(), "VEC3", bounds)
    }
}

fn bytemuck_f32(v: &[f32]) -> &[u8] {
    // SAFETY: f32 has no padding and any byte pattern is a valid u8.
    unsafe { std::slice::from_raw_parts(v.as_ptr() as *const u8, std::mem::size_of_val(v)) }
}
fn bytes_u16(v: &[u16]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}
fn bytes_u32(v: &[u32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

/// Write skinned meshes and the skeleton they use to a `.glb` file.
pub fn write_glb(path: &Path, skeleton: &Skeleton, meshes: &[ExportMesh]) -> Result<()> {
    if skeleton.is_empty() {
        bail!("Nothing to bind to: the skeleton is empty.");
    }
    if meshes.is_empty() {
        bail!("No meshes to export.");
    }
    for m in meshes {
        if m.mesh.skin.is_none() {
            bail!("'{}' is not skinned yet. Run Auto skin first.", m.name);
        }
    }
    let mut b = Builder::default();

    // ---- Skeleton nodes (one node per joint, same order) ----------------------
    let mut nodes: Vec<Value> = skeleton
        .joints
        .iter()
        .map(|j| {
            json!({
                "name": j.name,
                "translation": j.bind_local.translation.to_array(),
                "rotation": j.bind_local.rotation.normalize().to_array(),
                "scale": j.bind_local.scale.to_array(),
            })
        })
        .collect();
    for (i, j) in skeleton.joints.iter().enumerate() {
        if let Some(p) = j.parent {
            let children = nodes[p]
                .as_object_mut()
                .unwrap()
                .entry("children")
                .or_insert(json!([]));
            children.as_array_mut().unwrap().push(json!(i));
        }
    }
    let roots: Vec<usize> = skeleton.roots().collect();

    // Inverse bind matrices from the skeleton itself, so they always agree with the nodes.
    let ibm: Vec<f32> = skeleton
        .bind_world_matrices()
        .iter()
        .flat_map(|m: &Mat4| {
            // The bottom row must be exactly 0 0 0 1 (the glTF rules check this).
            let mut c = m.inverse().to_cols_array();
            c[3] = 0.0;
            c[7] = 0.0;
            c[11] = 0.0;
            c[15] = 1.0;
            c
        })
        .collect();
    let ibm_view = b.push_view(bytemuck_f32(&ibm), None);
    let ibm_accessor = b.accessor(ibm_view, FLOAT, skeleton.len(), "MAT4", None);

    // ---- Meshes ---------------------------------------------------------------------
    let mut gl_meshes = Vec::new();
    let mut materials = Vec::new();
    let mut images: Vec<Value> = Vec::new();
    let mut textures: Vec<Value> = Vec::new();
    let mut add_texture = |b: &mut Builder, t: &Texture| -> usize {
        let view = b.push_view(&t.bytes, None);
        images.push(json!({ "name": t.name, "bufferView": view, "mimeType": t.mime }));
        textures.push(json!({ "sampler": 0, "source": images.len() - 1 }));
        textures.len() - 1
    };
    let mut scene_nodes: Vec<usize> = roots.clone();
    for (mi, em) in meshes.iter().enumerate() {
        let baked = em.material.map(crate::materials::baked);
        let mesh = em.mesh;
        let skin = mesh.skin.as_ref().unwrap();
        // The skin's inverse binds must match the skeleton's (true for meshes
        // skinned or loaded in ThreadSmith). Positions are in bind space.
        let mut attributes = json!({ "POSITION": b.vec3s(&mesh.positions, true) });
        if mesh.normals.len() == mesh.positions.len() {
            attributes["NORMAL"] = json!(b.vec3s(&mesh.normals, false));
        }
        if mesh.uvs.len() == mesh.positions.len() {
            let flat: Vec<f32> = mesh.uvs.iter().flat_map(|v| v.to_array()).collect();
            let view = b.push_view(bytemuck_f32(&flat), Some(ARRAY_BUFFER));
            attributes["TEXCOORD_0"] = json!(b.accessor(view, FLOAT, mesh.uvs.len(), "VEC2", None));
        }
        // 8 influences = two sets of 4.
        for set in 0..MAX_INFLUENCES / 4 {
            let joints: Vec<u16> = skin
                .joints
                .iter()
                .flat_map(|j| j[set * 4..set * 4 + 4].to_vec())
                .collect();
            let weights: Vec<f32> = skin
                .weights
                .iter()
                .flat_map(|w| w[set * 4..set * 4 + 4].to_vec())
                .collect();
            if set > 0 && weights.iter().all(|w| *w == 0.0) {
                continue; // no vertex uses influences 5-8
            }
            let jv = b.push_view(&bytes_u16(&joints), Some(ARRAY_BUFFER));
            attributes[format!("JOINTS_{set}")] =
                json!(b.accessor(jv, UNSIGNED_SHORT, skin.joints.len(), "VEC4", None));
            let wv = b.push_view(bytemuck_f32(&weights), Some(ARRAY_BUFFER));
            attributes[format!("WEIGHTS_{set}")] =
                json!(b.accessor(wv, FLOAT, skin.weights.len(), "VEC4", None));
        }
        let iv = b.push_view(&bytes_u32(&mesh.indices), Some(ELEMENT_ARRAY_BUFFER));
        let indices = b.accessor(iv, UNSIGNED_INT, mesh.indices.len(), "SCALAR", None);

        let mut material = match &baked {
            Some(m) => json!({
                "name": format!("M_{}", em.name),
                "pbrMetallicRoughness": {
                    "baseColorFactor": m.base_color,
                    "metallicFactor": m.metallic,
                    "roughnessFactor": m.roughness
                },
                "doubleSided": true
            }),
            None => json!({
                "name": format!("M_{}", em.name),
                "pbrMetallicRoughness": {
                    "baseColorFactor": [em.color[0], em.color[1], em.color[2], 1.0],
                    "metallicFactor": 0.0,
                    "roughnessFactor": 0.85
                },
                "doubleSided": true
            }),
        };
        if let Some(m) = &baked {
            if let Some(t) = &m.base_color_texture {
                let ti = add_texture(&mut b, t);
                material["pbrMetallicRoughness"]["baseColorTexture"] = json!({ "index": ti });
            }
            if let Some(t) = &m.normal_texture {
                let ti = add_texture(&mut b, t);
                material["normalTexture"] = json!({ "index": ti });
            }
        }
        materials.push(material);
        gl_meshes.push(json!({
            "name": em.name,
            "primitives": [{ "attributes": attributes, "indices": indices, "material": mi }]
        }));
        nodes.push(json!({ "name": em.name, "mesh": mi, "skin": 0 }));
        scene_nodes.push(nodes.len() - 1);
    }

    let mut skin = json!({ "inverseBindMatrices": ibm_accessor, "joints": (0..skeleton.len()).collect::<Vec<_>>() });
    if roots.len() == 1 {
        skin["skeleton"] = json!(roots[0]);
    }
    while b.bin.len() % 4 != 0 {
        b.bin.push(0);
    }
    let mut doc = json!({
        "asset": { "version": "2.0", "generator": concat!("ThreadSmith ", env!("CARGO_PKG_VERSION")) },
        "scene": 0,
        "scenes": [{ "name": "ThreadSmith", "nodes": scene_nodes }],
        "nodes": nodes,
        "meshes": gl_meshes,
        "materials": materials,
        "skins": [skin],
        "accessors": b.accessors,
        "bufferViews": b.views,
        "buffers": [{ "byteLength": b.bin.len() }],
    });
    if !images.is_empty() {
        doc["images"] = json!(images);
        doc["textures"] = json!(textures);
        // Linear filtering with mipmaps, repeat wrapping.
        doc["samplers"] = json!([{ "magFilter": 9729, "minFilter": 9987, "wrapS": 10497, "wrapT": 10497 }]);
    }

    // ---- GLB container: header, JSON chunk, BIN chunk -------------------------
    let mut json_bytes = serde_json::to_vec(&doc)?;
    while json_bytes.len() % 4 != 0 {
        json_bytes.push(b' ');
    }
    let total = 12 + 8 + json_bytes.len() + 8 + b.bin.len();
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&(json_bytes.len() as u32).to_le_bytes());
    out.extend_from_slice(b"JSON");
    out.extend_from_slice(&json_bytes);
    out.extend_from_slice(&(b.bin.len() as u32).to_le_bytes());
    out.extend_from_slice(b"BIN\0");
    out.extend_from_slice(&b.bin);
    std::fs::write(path, out).with_context(|| format!("Could not write {}", path.display()))?;
    Ok(())
}
