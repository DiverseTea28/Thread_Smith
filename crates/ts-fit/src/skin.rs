//! Auto skinning: copy skin weights from a body onto a garment.
//!
//! Based on "Robust Skin Weights Transfer via Weight Inpainting"
//! (Abdrashitov et al., Epic Games, SIGGRAPH Asia 2023):
//!
//! 1. **Match**: for each garment vertex, find the closest point on the body.
//!    If it is close enough and the surfaces face the same way, copy the
//!    body's weights there (blended over the body triangle).
//! 2. **Fill in (inpainting)**: the rest (loose sleeves, collars, hems) get
//!    smooth weights worked out from their matched neighbours. This is a
//!    harmonic fill: each unknown vertex ends up as the average of its
//!    neighbours, solved exactly with conjugate gradients.
//! 3. **Clean up**: keep the 8 biggest weights per vertex, make them add up to 1.

use std::collections::HashMap;

use glam::Vec3;
use ts_core::{Mesh, Skin, MAX_INFLUENCES};

use crate::bvh::TriBvh;
use crate::graph::WeldGraph;

#[derive(Clone, Debug)]
pub struct SkinSettings {
    /// Garment vertices further than this from the body (meters) are filled in
    /// instead of copied.
    pub max_distance: f32,
    /// Garment vertices whose normal differs from the body's by more than
    /// this (degrees) are filled in instead of copied.
    pub max_angle_deg: f32,
    /// Bind the whole garment to one bone (helmets, buckles, armour plates).
    pub rigid: bool,
}

impl Default for SkinSettings {
    fn default() -> Self {
        Self {
            max_distance: 0.05,
            max_angle_deg: 35.0,
            rigid: false,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct SkinReport {
    /// Vertices that copied weights straight from the body.
    pub matched: usize,
    /// Vertices whose weights were filled in from neighbours.
    pub inpainted: usize,
    /// Vertices on pieces with no matched vertex at all (took the nearest
    /// body weights instead).
    pub fallback: usize,
}

/// Sparse weights: (joint, weight) pairs.
type Weights = Vec<(u16, f32)>;

fn add_weights(into: &mut Weights, from: &[(u16, f32)], scale: f32) {
    for &(j, w) in from {
        match into.iter_mut().find(|(k, _)| *k == j) {
            Some(e) => e.1 += w * scale,
            None => into.push((j, w * scale)),
        }
    }
}

/// Skin `garment` (positions in world space, bind pose) from `body`.
/// `body_bvh` must be built from the body's positions, normals and indices.
pub fn transfer_skin(
    garment: &Mesh,
    body: &Mesh,
    body_bvh: &TriBvh,
    settings: &SkinSettings,
) -> (Skin, SkinReport) {
    let body_skin = body.skin.as_ref().expect("body mesh must be skinned");
    let body_weights: Vec<Weights> = body_skin
        .joints
        .iter()
        .zip(&body_skin.weights)
        .map(|(j, w)| {
            (0..MAX_INFLUENCES)
                .filter(|&k| w[k] > 0.0)
                .map(|k| (j[k], w[k]))
                .collect()
        })
        .collect();

    let graph = WeldGraph::new(&garment.positions, &garment.indices, 1e-4);
    let n = graph.len();
    let positions = graph.gather(&garment.positions);
    // Average the normals of split copies.
    let mut normals = vec![Vec3::ZERO; n];
    for (v, &w) in graph.weld_of.iter().enumerate() {
        normals[w as usize] += garment.normals.get(v).copied().unwrap_or(Vec3::ZERO);
    }
    normals.iter_mut().for_each(|x| *x = x.normalize_or_zero());

    // ---- 1. Match -----------------------------------------------------------
    let cos_limit = settings.max_angle_deg.to_radians().cos();
    let mut nearest: Vec<Weights> = Vec::with_capacity(n);
    let mut matched = vec![false; n];
    for i in 0..n {
        let Some(hit) = body_bvh.closest(positions[i]) else {
            nearest.push(Vec::new());
            continue;
        };
        let tri = body_bvh.triangles[hit.triangle];
        let mut w = Weights::new();
        for k in 0..3 {
            add_weights(&mut w, &body_weights[tri[k] as usize], hit.bary[k]);
        }
        // abs(): the inside of a garment faces the body, which is fine.
        let facing = normals[i] == Vec3::ZERO || normals[i].dot(hit.normal).abs() >= cos_limit;
        matched[i] = hit.distance <= settings.max_distance && facing;
        nearest.push(w);
    }

    let mut report = SkinReport::default();
    let mut result: Vec<Weights> = vec![Vec::new(); n];

    if settings.rigid {
        // Whole piece follows the single most important bone.
        let mut total = Weights::new();
        for w in &nearest {
            add_weights(&mut total, w, 1.0);
        }
        let best = total
            .iter()
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .map(|e| e.0)
            .unwrap_or(0);
        result.iter_mut().for_each(|r| *r = vec![(best, 1.0)]);
        report.matched = n;
    } else {
        // Pieces with no matched vertex can't be filled in: use nearest weights.
        let reachable = reachable_from(&graph, &matched);
        let mut known = matched.clone();
        for i in 0..n {
            if matched[i] {
                report.matched += 1;
            } else if !reachable[i] {
                known[i] = true;
                report.fallback += 1;
            } else {
                report.inpainted += 1;
            }
        }
        for i in 0..n {
            if known[i] {
                result[i] = nearest[i].clone();
            }
        }
        if report.inpainted > 0 {
            inpaint(&graph, &known, &nearest, &mut result);
        }
    }

    // ---- 3. Clean up and spread to every mesh vertex --------------------------
    let rows: Vec<([u16; MAX_INFLUENCES], [f32; MAX_INFLUENCES])> = result
        .into_iter()
        .map(|mut w| {
            w.retain(|e| e.1 > 1e-4);
            w.sort_by(|a, b| b.1.total_cmp(&a.1));
            w.truncate(MAX_INFLUENCES);
            let mut j = [0u16; MAX_INFLUENCES];
            let mut ww = [0f32; MAX_INFLUENCES];
            for (k, (bone, weight)) in w.iter().enumerate() {
                j[k] = *bone;
                ww[k] = *weight;
            }
            (j, ww)
        })
        .collect();
    let per_vertex = graph.expand(&rows);
    let mut skin = Skin {
        joints: per_vertex.iter().map(|r| r.0).collect(),
        weights: per_vertex.iter().map(|r| r.1).collect(),
        inverse_bind: body_skin.inverse_bind.clone(),
    };
    skin.normalize();
    log::info!("Auto skin: {report:?}");
    (skin, report)
}

/// Which vertices are connected (through the mesh) to at least one known vertex.
fn reachable_from(graph: &WeldGraph, known: &[bool]) -> Vec<bool> {
    let mut seen = known.to_vec();
    let mut queue: Vec<u32> = (0..known.len() as u32)
        .filter(|&i| known[i as usize])
        .collect();
    while let Some(i) = queue.pop() {
        for &j in &graph.neighbours[i as usize] {
            if !seen[j as usize] {
                seen[j as usize] = true;
                queue.push(j);
            }
        }
    }
    seen
}

/// Harmonic fill: for every unknown vertex u, solve
///     degree(u) * x_u - sum(unknown neighbours x) = sum(known neighbour weights)
/// one bone at a time, with conjugate gradients.
fn inpaint(graph: &WeldGraph, known: &[bool], initial: &[Weights], result: &mut [Weights]) {
    let unknown: Vec<usize> = (0..known.len()).filter(|&i| !known[i]).collect();
    let mut slot = vec![usize::MAX; known.len()];
    for (s, &u) in unknown.iter().enumerate() {
        slot[u] = s;
    }
    // Every bone used by a known vertex.
    let mut bones: Vec<u16> = Vec::new();
    for (i, w) in result.iter().enumerate() {
        if known[i] {
            for &(b, _) in w {
                if !bones.contains(&b) {
                    bones.push(b);
                }
            }
        }
    }
    let bone_col: HashMap<u16, usize> = bones.iter().enumerate().map(|(c, &b)| (b, c)).collect();

    // Matrix rows (only off-diagonal unknown neighbours; diagonal = degree).
    let degree: Vec<f32> = unknown
        .iter()
        .map(|&u| graph.neighbours[u].len() as f32)
        .collect();
    let off: Vec<Vec<usize>> = unknown
        .iter()
        .map(|&u| {
            graph.neighbours[u]
                .iter()
                .filter(|&&j| !known[j as usize])
                .map(|&j| slot[j as usize])
                .collect()
        })
        .collect();

    // Right-hand sides, per bone.
    let mut rhs = vec![vec![0f32; unknown.len()]; bones.len()];
    for (s, &u) in unknown.iter().enumerate() {
        for &j in &graph.neighbours[u] {
            if known[j as usize] {
                for &(b, w) in &result[j as usize] {
                    rhs[bone_col[&b]][s] += w;
                }
            }
        }
    }

    let apply = |x: &[f32], out: &mut [f32]| {
        for s in 0..x.len() {
            let mut v = degree[s] * x[s];
            for &t in &off[s] {
                v -= x[t];
            }
            out[s] = v;
        }
    };

    for (c, &bone) in bones.iter().enumerate() {
        // Start from the nearest-body weights: usually close already.
        let mut x: Vec<f32> = unknown
            .iter()
            .map(|&u| {
                initial[u]
                    .iter()
                    .find(|e| e.0 == bone)
                    .map(|e| e.1)
                    .unwrap_or(0.0)
            })
            .collect();
        conjugate_gradient(&apply, &rhs[c], &mut x, 400, 1e-6);
        for (s, &u) in unknown.iter().enumerate() {
            if x[s] > 1e-4 {
                result[u].push((bone, x[s]));
            }
        }
    }
}

fn conjugate_gradient(
    apply: &dyn Fn(&[f32], &mut [f32]),
    b: &[f32],
    x: &mut [f32],
    max_iter: usize,
    tol: f32,
) {
    let n = b.len();
    let mut ax = vec![0f32; n];
    apply(x, &mut ax);
    let mut r: Vec<f32> = b.iter().zip(&ax).map(|(b, a)| b - a).collect();
    let mut p = r.clone();
    let mut rr: f32 = r.iter().map(|v| v * v).sum();
    let b_norm = b.iter().map(|v| v * v).sum::<f32>().sqrt().max(1e-12);
    let mut ap = vec![0f32; n];
    for _ in 0..max_iter {
        if rr.sqrt() / b_norm < tol {
            break;
        }
        apply(&p, &mut ap);
        let pap: f32 = p.iter().zip(&ap).map(|(a, b)| a * b).sum();
        if pap.abs() < 1e-20 {
            break;
        }
        let alpha = rr / pap;
        for i in 0..n {
            x[i] += alpha * p[i];
            r[i] -= alpha * ap[i];
        }
        let rr_new: f32 = r.iter().map(|v| v * v).sum();
        let beta = rr_new / rr;
        for i in 0..n {
            p[i] = r[i] + beta * p[i];
        }
        rr = rr_new;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Mat4;

    /// A strip of quads along X, as a "body" whose left half follows bone 0
    /// and right half bone 1.
    fn strip(z: f32, n: usize, skinned: bool) -> Mesh {
        let mut m = Mesh::default();
        for i in 0..=n {
            let x = i as f32 / n as f32 * 2.0 - 1.0;
            m.positions.push(Vec3::new(x, 0.0, z));
            m.positions.push(Vec3::new(x, 1.0, z));
        }
        for i in 0..n as u32 {
            let a = i * 2;
            m.indices.extend([a, a + 2, a + 1, a + 1, a + 2, a + 3]);
        }
        m.compute_normals();
        if skinned {
            let mut s = Skin {
                inverse_bind: vec![Mat4::IDENTITY; 2],
                ..Default::default()
            };
            for p in &m.positions {
                let mut j = [0u16; MAX_INFLUENCES];
                let mut w = [0f32; MAX_INFLUENCES];
                let t = (p.x * 0.5 + 0.5).clamp(0.0, 1.0);
                j[0] = 0;
                w[0] = 1.0 - t;
                j[1] = 1;
                w[1] = t;
                s.joints.push(j);
                s.weights.push(w);
            }
            m.skin = Some(s);
        }
        m
    }

    #[test]
    fn copies_close_and_fills_far() {
        let body = strip(0.0, 20, true);
        let bvh = TriBvh::new(&body.positions, &body.normals, &body.indices);
        // Garment: a strip just in front of the body, plus a far flap that must be filled in.
        let mut garment = strip(0.01, 20, false);
        let flap_start = garment.positions.len() as u32;
        garment.positions.push(Vec3::new(1.0, 0.0, 0.5));
        garment.positions.push(Vec3::new(1.0, 1.0, 0.5));
        let last = flap_start - 2;
        garment.indices.extend([
            last,
            flap_start,
            last + 1,
            last + 1,
            flap_start,
            flap_start + 1,
        ]);
        garment.compute_normals();

        let (skin, report) = transfer_skin(&garment, &body, &bvh, &SkinSettings::default());
        assert!(
            report.matched > 30 && (2..=4).contains(&report.inpainted),
            "{report:?}"
        );
        // Middle vertex: half and half.
        let mid = 20; // x = 0
        let w1: f32 = (0..MAX_INFLUENCES)
            .filter(|&k| skin.joints[mid][k] == 1)
            .map(|k| skin.weights[mid][k])
            .sum();
        assert!((w1 - 0.5).abs() < 0.02, "{w1}");
        // Flap follows the right end (bone 1).
        let f = flap_start as usize;
        let wf: f32 = (0..MAX_INFLUENCES)
            .filter(|&k| skin.joints[f][k] == 1)
            .map(|k| skin.weights[f][k])
            .sum();
        assert!(wf > 0.9, "{wf}");
    }
}
