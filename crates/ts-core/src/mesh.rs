//! Triangle meshes with optional skin weights.

use glam::{Mat4, Vec2, Vec3};

/// Most bones that can move one vertex. MetaHuman bodies use up to 8.
pub const MAX_INFLUENCES: usize = 8;

/// Skin weights for a mesh.
#[derive(Clone, Debug, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Skin {
    /// Per vertex: joint indices into the scene skeleton.
    pub joints: Vec<[u16; MAX_INFLUENCES]>,
    /// Per vertex: weights matching `joints`. Unused slots are 0. Each row adds up to 1.
    pub weights: Vec<[f32; MAX_INFLUENCES]>,
    /// Per joint in the scene skeleton: moves a point from mesh space into
    /// that joint's space at bind time ("inverse bind matrix").
    pub inverse_bind: Vec<Mat4>,
}

impl Skin {
    /// Make every vertex's weights add up to 1 (and drop tiny ones).
    pub fn normalize(&mut self) {
        for (w, j) in self.weights.iter_mut().zip(self.joints.iter_mut()) {
            for k in 0..MAX_INFLUENCES {
                if w[k] < 1e-5 {
                    w[k] = 0.0;
                    j[k] = 0;
                }
            }
            let sum: f32 = w.iter().sum();
            if sum > 0.0 {
                w.iter_mut().for_each(|x| *x /= sum);
            }
        }
    }

    /// Add one (joint, weight) to a vertex row, keeping the biggest weights
    /// if the row is already full.
    pub fn push_influence(
        joints: &mut [u16; MAX_INFLUENCES],
        weights: &mut [f32; MAX_INFLUENCES],
        joint: u16,
        weight: f32,
    ) {
        if weight <= 0.0 {
            return;
        }
        // Smallest slot (empty slots are 0, so they get used first).
        let (slot, smallest) = weights
            .iter()
            .enumerate()
            .min_by(|a, b| a.1.total_cmp(b.1))
            .map(|(i, w)| (i, *w))
            .unwrap();
        if weight > smallest {
            joints[slot] = joint;
            weights[slot] = weight;
        }
    }
}

/// A triangle mesh. All arrays are per vertex except `indices` (3 per triangle).
#[derive(Clone, Debug, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Mesh {
    pub positions: Vec<Vec3>,
    pub normals: Vec<Vec3>,
    pub uvs: Vec<Vec2>,
    pub indices: Vec<u32>,
    pub skin: Option<Skin>,
}

impl Mesh {
    pub fn vertex_count(&self) -> usize {
        self.positions.len()
    }

    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    /// Smooth normals from the triangles (area weighted).
    pub fn compute_normals(&mut self) {
        self.normals = smooth_normals(&self.positions, &self.indices);
    }

    /// Unique edges, two indices each. Used for the wireframe view.
    pub fn edges(&self) -> Vec<u32> {
        let mut edges: Vec<(u32, u32)> = Vec::with_capacity(self.indices.len());
        for t in self.indices.chunks_exact(3) {
            for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                edges.push((a.min(b), a.max(b)));
            }
        }
        edges.sort_unstable();
        edges.dedup();
        edges.into_iter().flat_map(|(a, b)| [a, b]).collect()
    }

    /// Positions and normals deformed by the given joint world matrices
    /// (linear blend skinning). Meshes without a skin are returned unchanged.
    pub fn skinned(&self, joint_world: &[Mat4]) -> (Vec<Vec3>, Vec<Vec3>) {
        let Some(skin) = &self.skin else {
            return (self.positions.clone(), self.normals.clone());
        };
        // One matrix per joint: bind space -> posed world space.
        let skin_mats: Vec<Mat4> = joint_world
            .iter()
            .zip(&skin.inverse_bind)
            .map(|(w, ib)| *w * *ib)
            .collect();

        let mut pos = Vec::with_capacity(self.positions.len());
        let mut nrm = Vec::with_capacity(self.positions.len());
        for v in 0..self.positions.len() {
            let mut m = Mat4::ZERO;
            let mut total = 0.0;
            for k in 0..MAX_INFLUENCES {
                let w = skin.weights[v][k];
                if w > 0.0 {
                    m += skin_mats[skin.joints[v][k] as usize] * w;
                    total += w;
                }
            }
            if total <= 0.0 {
                m = Mat4::IDENTITY;
            }
            pos.push(m.transform_point3(self.positions[v]));
            let n = self.normals.get(v).copied().unwrap_or(Vec3::Y);
            nrm.push(m.transform_vector3(n).normalize_or_zero());
        }
        (pos, nrm)
    }
}

/// Area-weighted smooth normals.
pub fn smooth_normals(positions: &[Vec3], indices: &[u32]) -> Vec<Vec3> {
    let mut normals = vec![Vec3::ZERO; positions.len()];
    for t in indices.chunks_exact(3) {
        let (a, b, c) = (t[0] as usize, t[1] as usize, t[2] as usize);
        let n = (positions[b] - positions[a]).cross(positions[c] - positions[a]);
        normals[a] += n;
        normals[b] += n;
        normals[c] += n;
    }
    normals
        .iter_mut()
        .for_each(|n| *n = n.normalize_or(Vec3::Y));
    normals
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edges_are_unique() {
        let m = Mesh {
            indices: vec![0, 1, 2, 2, 1, 3],
            positions: vec![Vec3::ZERO; 4],
            ..Default::default()
        };
        assert_eq!(m.edges().len() / 2, 5);
    }

    #[test]
    fn push_influence_keeps_biggest() {
        let mut j = [0u16; MAX_INFLUENCES];
        let mut w = [0f32; MAX_INFLUENCES];
        for i in 0..10u16 {
            Skin::push_influence(&mut j, &mut w, i, i as f32 + 1.0);
        }
        assert!(!j.contains(&0) && !j.contains(&1)); // two smallest dropped
    }
}
