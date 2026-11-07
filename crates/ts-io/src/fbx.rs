//! FBX (and OBJ) import through ufbx.
//!
//! ufbx converts the file into Y up, right handed, meters for us, so an
//! Unreal export (Z up, centimeters) lands the right way up and size.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{anyhow, Result};
use glam::{Mat3, Mat4, Vec2, Vec3, Vec4};
use ts_core::{Joint, Mesh, MeshPart, Scene, Skeleton, Skin, Trs, MAX_INFLUENCES};

fn mat(m: &ufbx::Matrix) -> Mat4 {
    Mat4::from_cols(
        Vec4::new(m.m00 as f32, m.m10 as f32, m.m20 as f32, 0.0),
        Vec4::new(m.m01 as f32, m.m11 as f32, m.m21 as f32, 0.0),
        Vec4::new(m.m02 as f32, m.m12 as f32, m.m22 as f32, 0.0),
        Vec4::new(m.m03 as f32, m.m13 as f32, m.m23 as f32, 1.0),
    )
}

fn v3(v: &ufbx::Vec3) -> Vec3 {
    Vec3::new(v.x as f32, v.y as f32, v.z as f32)
}

pub fn load(path: &Path) -> Result<Scene> {
    let path_str = path
        .to_str()
        .ok_or_else(|| anyhow!("File path is not valid text: {}", path.display()))?;
    let opts = ufbx::LoadOpts {
        target_axes: ufbx::CoordinateAxes::right_handed_y_up(),
        target_unit_meters: 1.0,
        space_conversion: ufbx::SpaceConversion::ModifyGeometry,
        generate_missing_normals: true,
        ..Default::default()
    };
    let fbx = ufbx::load_file(path_str, opts)
        .map_err(|e| anyhow!("Could not read {}: {:?}", path.display(), e))?;

    // ---- Skeleton -------------------------------------------------------
    // Joints = nodes marked as bones + every node a skin cluster points at.
    let mut bind_world: HashMap<u32, Mat4> = HashMap::new(); // node element id -> bind world
    let mut is_joint: HashMap<u32, bool> = HashMap::new();
    for node in &fbx.nodes {
        if node.bone.is_some() {
            is_joint.insert(node.element.element_id, true);
        }
    }
    for cluster in &fbx.skin_clusters {
        if let Some(bone) = &cluster.bone_node {
            is_joint.insert(bone.element.element_id, true);
            bind_world.insert(bone.element.element_id, mat(&cluster.bind_to_world));
        }
    }

    let joint_nodes: Vec<&ufbx::Node> = fbx
        .nodes
        .iter()
        .filter(|n| is_joint.contains_key(&n.element.element_id))
        .map(|n| &**n)
        .collect();
    let id_to_slot: HashMap<u32, usize> = joint_nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.element.element_id, i))
        .collect();
    // Parent = closest ancestor that is also a joint.
    let slot_parent: Vec<Option<usize>> = joint_nodes
        .iter()
        .map(|n| {
            let mut p = n.parent.as_ref();
            while let Some(pn) = p {
                if let Some(&s) = id_to_slot.get(&pn.element.element_id) {
                    return Some(s);
                }
                p = pn.parent.as_ref();
            }
            None
        })
        .collect();
    let order = crate::parents_first_order(&slot_parent);
    let mut slot_to_joint = vec![0usize; joint_nodes.len()];
    for (j, &s) in order.iter().enumerate() {
        slot_to_joint[s] = j;
    }

    // Bind world for joints no cluster told us about: follow the file's
    // current offset from the parent joint.
    let mut world_by_slot: Vec<Mat4> = vec![Mat4::IDENTITY; joint_nodes.len()];
    for &s in &order {
        let node = joint_nodes[s];
        let id = node.element.element_id;
        let w = match bind_world.get(&id) {
            Some(w) => *w,
            None => match slot_parent[s] {
                Some(p) => {
                    let parent_now = mat(&joint_nodes[p].node_to_world);
                    world_by_slot[p] * parent_now.inverse() * mat(&node.node_to_world)
                }
                None => mat(&node.node_to_world),
            },
        };
        world_by_slot[s] = w;
    }

    let mut skeleton = Skeleton::default();
    for &s in &order {
        let local = match slot_parent[s] {
            Some(p) => world_by_slot[p].inverse() * world_by_slot[s],
            None => world_by_slot[s],
        };
        skeleton.joints.push(Joint {
            name: joint_nodes[s].element.name.to_string(),
            parent: slot_parent[s].map(|p| slot_to_joint[p]),
            bind_local: Trs::from_mat4(local),
        });
    }
    let joint_of_node = |n: &ufbx::Node| {
        id_to_slot
            .get(&n.element.element_id)
            .map(|&s| slot_to_joint[s])
    };

    // ---- Meshes ---------------------------------------------------------
    let mut parts = Vec::new();
    for node in &fbx.nodes {
        let Some(fmesh) = &node.mesh else { continue };
        let fmesh: &ufbx::Mesh = fmesh;
        let skin_def = fmesh.skin_deformers.iter().next();

        let mut mesh = Mesh::default();
        // ufbx stores attributes per face corner. Merge corners that share
        // the same vertex, normal and UV into one output vertex.
        let mut corner_to_vertex: HashMap<(u32, u32, u32), u32> = HashMap::new();
        let mut source_vertex: Vec<u32> = Vec::new(); // output vertex -> fbx logical vertex
        let mut tri = vec![0u32; fmesh.max_face_triangles * 3];
        let has_uv = fmesh.vertex_uv.exists;
        let has_n = fmesh.vertex_normal.exists;

        for face in fmesh.faces.iter() {
            let n = fmesh.triangulate_face(&mut tri, *face) as usize;
            for t in 0..n {
                let mut corners = [tri[t * 3], tri[t * 3 + 1], tri[t * 3 + 2]];
                if fmesh.reversed_winding {
                    corners.swap(1, 2);
                }
                for c in corners {
                    let ci = c as usize;
                    let vi = fmesh.vertex_indices[ci];
                    let ni = if has_n {
                        fmesh.vertex_normal.indices[ci]
                    } else {
                        0
                    };
                    let ui = if has_uv {
                        fmesh.vertex_uv.indices[ci]
                    } else {
                        0
                    };
                    let out = *corner_to_vertex.entry((vi, ni, ui)).or_insert_with(|| {
                        mesh.positions.push(v3(&fmesh.vertex_position[ci]));
                        if has_n {
                            mesh.normals.push(v3(&fmesh.vertex_normal[ci]));
                        }
                        if has_uv {
                            let uv = &fmesh.vertex_uv[ci];
                            // FBX has V going up; flip to the usual top-left origin.
                            mesh.uvs.push(Vec2::new(uv.x as f32, 1.0 - uv.y as f32));
                        }
                        source_vertex.push(vi);
                        (mesh.positions.len() - 1) as u32
                    });
                    mesh.indices.push(out);
                }
            }
        }
        if mesh.indices.is_empty() {
            continue;
        }

        match skin_def {
            Some(skin) if !skeleton.is_empty() => {
                let vcount = mesh.positions.len();
                let mut out = Skin {
                    joints: vec![[0; MAX_INFLUENCES]; vcount],
                    weights: vec![[0.0; MAX_INFLUENCES]; vcount],
                    inverse_bind: vec![Mat4::IDENTITY; skeleton.len()],
                };
                let cluster_joint: Vec<Option<usize>> = skin
                    .clusters
                    .iter()
                    .map(|c| c.bone_node.as_ref().and_then(|b| joint_of_node(b)))
                    .collect();
                for (c, cluster) in skin.clusters.iter().enumerate() {
                    if let Some(j) = cluster_joint[c] {
                        out.inverse_bind[j] = mat(&cluster.geometry_to_bone);
                    }
                }
                for (v, &src) in source_vertex.iter().enumerate() {
                    let Some(sv) = skin.vertices.get(src as usize) else {
                        continue;
                    };
                    for k in 0..sv.num_weights {
                        let w = &skin.weights[(sv.weight_begin + k) as usize];
                        if let Some(Some(j)) = cluster_joint.get(w.cluster_index as usize) {
                            Skin::push_influence(
                                &mut out.joints[v],
                                &mut out.weights[v],
                                *j as u16,
                                w.weight as f32,
                            );
                        }
                    }
                }
                mesh.skin = Some(out);
            }
            _ => {
                let m = mat(&node.geometry_to_world);
                let nm = Mat3::from_mat4(m).inverse().transpose();
                mesh.positions
                    .iter_mut()
                    .for_each(|p| *p = m.transform_point3(*p));
                mesh.normals
                    .iter_mut()
                    .for_each(|n| *n = (nm * *n).normalize_or_zero());
            }
        }

        let name = if node.element.name.is_empty() {
            fmesh.element.name.to_string()
        } else {
            node.element.name.to_string()
        };
        parts.push(MeshPart::new(name, mesh));
    }

    Ok(Scene {
        name: String::new(),
        parts,
        skeleton,
    })
}
