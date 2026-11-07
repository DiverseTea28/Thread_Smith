//! glTF / GLB import.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use glam::{Mat3, Mat4, Vec2, Vec3};
use ts_core::{Joint, Mesh, MeshPart, Scene, Skeleton, Skin, Trs, MAX_INFLUENCES};

pub fn load(path: &Path) -> Result<Scene> {
    let (doc, buffers, _images) = gltf::import(path)
        .with_context(|| format!("Could not read glTF file {}", path.display()))?;

    // Parent of every node, and world matrix of every node (default pose).
    let node_count = doc.nodes().len();
    let mut node_parent = vec![None; node_count];
    for node in doc.nodes() {
        for child in node.children() {
            node_parent[child.index()] = Some(node.index());
        }
    }
    let node_local: Vec<Mat4> = doc
        .nodes()
        .map(|n| Mat4::from_cols_array_2d(&n.transform().matrix()))
        .collect();
    let mut node_world = vec![None; node_count];
    fn world_of(
        i: usize,
        parent: &[Option<usize>],
        local: &[Mat4],
        cache: &mut Vec<Option<Mat4>>,
    ) -> Mat4 {
        if let Some(m) = cache[i] {
            return m;
        }
        let m = match parent[i] {
            Some(p) => world_of(p, parent, local, cache) * local[i],
            None => local[i],
        };
        cache[i] = Some(m);
        m
    }
    let node_world: Vec<Mat4> = (0..node_count)
        .map(|i| world_of(i, &node_parent, &node_local, &mut node_world))
        .collect();

    // Every node used as a joint by any skin becomes a skeleton joint.
    let mut is_joint = vec![false; node_count];
    for skin in doc.skins() {
        for j in skin.joints() {
            is_joint[j.index()] = true;
        }
    }
    let joint_nodes: Vec<usize> = (0..node_count).filter(|&i| is_joint[i]).collect();
    // Parent = closest ancestor that is also a joint.
    let joint_parent_node: Vec<Option<usize>> = joint_nodes
        .iter()
        .map(|&n| {
            let mut p = node_parent[n];
            while let Some(pi) = p {
                if is_joint[pi] {
                    return Some(pi);
                }
                p = node_parent[pi];
            }
            None
        })
        .collect();
    let node_to_slot: HashMap<usize, usize> = joint_nodes
        .iter()
        .enumerate()
        .map(|(s, &n)| (n, s))
        .collect();
    let slot_parents: Vec<Option<usize>> = joint_parent_node
        .iter()
        .map(|p| p.map(|n| node_to_slot[&n]))
        .collect();
    let order = crate::parents_first_order(&slot_parents);
    let mut node_to_joint = HashMap::new();
    for (new_index, &slot) in order.iter().enumerate() {
        node_to_joint.insert(joint_nodes[slot], new_index);
    }

    let mut skeleton = Skeleton::default();
    for &slot in &order {
        let n = joint_nodes[slot];
        let node = doc.nodes().nth(n).unwrap();
        let parent_node = joint_parent_node[slot];
        // Local relative to the parent JOINT (folds in any non-joint nodes between).
        let local = match parent_node {
            Some(p) => node_world[p].inverse() * node_world[n],
            None => node_world[n],
        };
        skeleton.joints.push(Joint {
            name: node
                .name()
                .map(str::to_owned)
                .unwrap_or_else(|| format!("joint_{n}")),
            parent: parent_node.map(|p| node_to_joint[&p]),
            bind_local: Trs::from_mat4(local),
        });
    }

    let mut parts = Vec::new();
    for node in doc.nodes() {
        let Some(gmesh) = node.mesh() else { continue };
        let skin_info = node.skin().map(|skin| {
            let reader = skin.reader(|b| Some(&buffers[b.index()]));
            let ibms: Vec<Mat4> = reader
                .read_inverse_bind_matrices()
                .map(|it| it.map(|m| Mat4::from_cols_array_2d(&m)).collect())
                .unwrap_or_default();
            let joints: Vec<usize> = skin.joints().map(|j| node_to_joint[&j.index()]).collect();
            (joints, ibms)
        });

        let prim_count = gmesh.primitives().len();
        for (pi, prim) in gmesh.primitives().enumerate() {
            if prim.mode() != gltf::mesh::Mode::Triangles {
                log::warn!("Skipping non-triangle primitive in {:?}", gmesh.name());
                continue;
            }
            let reader = prim.reader(|b| Some(&buffers[b.index()]));
            let Some(positions) = reader.read_positions() else {
                continue;
            };
            let mut mesh = Mesh {
                positions: positions.map(Vec3::from).collect(),
                ..Default::default()
            };
            if let Some(n) = reader.read_normals() {
                mesh.normals = n.map(Vec3::from).collect();
            }
            if let Some(uv) = reader.read_tex_coords(0) {
                mesh.uvs = uv.into_f32().map(Vec2::from).collect();
            }
            mesh.indices = match reader.read_indices() {
                Some(i) => i.into_u32().collect(),
                None => (0..mesh.positions.len() as u32).collect(),
            };

            match (&skin_info, reader.read_joints(0), reader.read_weights(0)) {
                (Some((skin_joints, ibms)), Some(j0), Some(w0)) => {
                    let vcount = mesh.positions.len();
                    let mut skin = Skin {
                        joints: vec![[0; MAX_INFLUENCES]; vcount],
                        weights: vec![[0.0; MAX_INFLUENCES]; vcount],
                        inverse_bind: vec![Mat4::IDENTITY; skeleton.len()],
                    };
                    for (k, &j) in skin_joints.iter().enumerate() {
                        if let Some(ib) = ibms.get(k) {
                            skin.inverse_bind[j] = *ib;
                        }
                    }
                    let mut add_set = |js: Vec<[u16; 4]>, ws: Vec<[f32; 4]>| {
                        for v in 0..vcount.min(js.len()) {
                            for k in 0..4 {
                                let joint =
                                    skin_joints.get(js[v][k] as usize).copied().unwrap_or(0) as u16;
                                Skin::push_influence(
                                    &mut skin.joints[v],
                                    &mut skin.weights[v],
                                    joint,
                                    ws[v][k],
                                );
                            }
                        }
                    };
                    add_set(j0.into_u16().collect(), w0.into_f32().collect());
                    // A second set holds influences 5 to 8 (MetaHuman bodies use these).
                    if let (Some(j1), Some(w1)) = (reader.read_joints(1), reader.read_weights(1)) {
                        add_set(j1.into_u16().collect(), w1.into_f32().collect());
                    }
                    mesh.skin = Some(skin);
                }
                _ => {
                    // Not skinned: bake the node's placement into the vertices.
                    let m = node_world[node.index()];
                    let nm = Mat3::from_mat4(m).inverse().transpose();
                    mesh.positions
                        .iter_mut()
                        .for_each(|p| *p = m.transform_point3(*p));
                    mesh.normals
                        .iter_mut()
                        .for_each(|n| *n = (nm * *n).normalize_or_zero());
                }
            }

            let base = gmesh.name().or(node.name()).unwrap_or("mesh").to_owned();
            let name = if prim_count > 1 {
                format!("{base} #{pi}")
            } else {
                base
            };
            parts.push(MeshPart::new(name, mesh));
        }
    }

    Ok(Scene {
        name: String::new(),
        parts,
        skeleton,
    })
}
