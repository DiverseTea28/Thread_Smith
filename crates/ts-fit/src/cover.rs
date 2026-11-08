//! Which parts of the body are hidden under clothing (to remove them, so the
//! skin can't poke through in the game, and to save triangles).

use glam::Vec3;
use ts_core::Mesh;

use crate::TriBvh;

/// Settings for [`covered_triangles`].
#[derive(Clone, Copy, Debug)]
pub struct CoverSettings {
    /// A garment surface within this distance outside the skin covers it (meters).
    pub max_distance: f32,
    /// Rings of triangles kept along the edge of a covered area, so openings
    /// (neck, cuffs) never show a gap.
    pub keep_rings: usize,
}

impl Default for CoverSettings {
    fn default() -> Self {
        Self { max_distance: 0.06, keep_rings: 2 }
    }
}

/// Per body triangle: true if clothing fully covers it.
///
/// A body vertex is covered when the closest clothing surface is close, lies
/// outside the skin (along the skin's normal) and faces the same way. The
/// facing test stops, say, the inside of the arm counting as covered by the
/// side of a shirt.
pub fn covered_triangles(body: &Mesh, clothes: &TriBvh, settings: &CoverSettings) -> Vec<bool> {
    let n = body.positions.len();
    let mut covered = vec![false; n];
    for (i, p) in body.positions.iter().enumerate() {
        let normal = body.normals.get(i).copied().unwrap_or(Vec3::Y).normalize_or_zero();
        let Some(hit) = clothes.closest(*p) else { continue };
        let to = hit.point - *p;
        let d = to.length();
        if d > settings.max_distance {
            continue;
        }
        let outside = to.dot(normal) > -0.004; // not under the skin (small slack)
        let facing = hit.normal.normalize_or_zero().dot(normal) > 0.2;
        covered[i] = outside && facing;
    }
    // Weld vertices at the same spot (UV seams), so both sides agree.
    let graph = crate::WeldGraph::new(&body.positions, &body.indices, 1e-5);
    let mut welded = vec![true; graph.len()];
    for (v, &w) in graph.weld_of.iter().enumerate() {
        welded[w as usize] &= covered[v];
    }
    // Shrink each covered area: a vertex stays covered only if all its
    // neighbours are covered too.
    for _ in 0..settings.keep_rings + 1 {
        let mut next = welded.clone();
        for (w, n) in graph.neighbours.iter().enumerate() {
            if welded[w] && n.iter().any(|&u| !welded[u as usize]) {
                next[w] = false;
            }
        }
        welded = next;
    }
    let covered = graph.expand(&welded);
    body.indices
        .chunks_exact(3)
        .map(|t| t.iter().all(|&v| covered[v as usize]))
        .collect()
}

/// The body with covered triangles removed (unused vertices are dropped too).
pub fn without_triangles(body: &Mesh, hidden: &[bool]) -> Mesh {
    let mut used = vec![u32::MAX; body.positions.len()];
    let mut out = Mesh::default();
    let mut map = |v: u32, out: &mut Mesh| -> u32 {
        let v = v as usize;
        if used[v] == u32::MAX {
            used[v] = out.positions.len() as u32;
            out.positions.push(body.positions[v]);
            if let Some(n) = body.normals.get(v) {
                out.normals.push(*n);
            }
            if let Some(uv) = body.uvs.get(v) {
                out.uvs.push(*uv);
            }
        }
        used[v]
    };
    for (t, tri) in body.indices.chunks_exact(3).enumerate() {
        if hidden.get(t).copied().unwrap_or(false) {
            continue;
        }
        for &v in tri {
            let i = map(v, &mut out);
            out.indices.push(i);
        }
    }
    if let Some(skin) = &body.skin {
        let mut s = ts_core::Skin {
            joints: vec![[0; ts_core::MAX_INFLUENCES]; out.positions.len()],
            weights: vec![[0.0; ts_core::MAX_INFLUENCES]; out.positions.len()],
            inverse_bind: skin.inverse_bind.clone(),
        };
        for (old, &new) in used.iter().enumerate() {
            if new != u32::MAX {
                s.joints[new as usize] = skin.joints[old];
                s.weights[new as usize] = skin.weights[old];
            }
        }
        out.skin = Some(s);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tube around Y from y0 to y1.
    fn tube(r: f32, y0: f32, y1: f32) -> Mesh {
        let mut m = Mesh::default();
        let (rings, segs) = (20, 24);
        for i in 0..=rings {
            let y = y0 + (y1 - y0) * i as f32 / rings as f32;
            for k in 0..segs {
                let a = k as f32 / segs as f32 * std::f32::consts::TAU;
                m.positions.push(Vec3::new(a.cos() * r, y, a.sin() * r));
            }
        }
        for i in 0..rings {
            for k in 0..segs {
                let a = (i * segs + k) as u32;
                let b = (i * segs + (k + 1) % segs) as u32;
                m.indices.extend([a, a + segs as u32, b, b, a + segs as u32, b + segs as u32]);
            }
        }
        m.compute_normals();
        m
    }

    #[test]
    fn sleeve_covers_the_middle_only() {
        let body = tube(0.1, 0.0, 1.0);
        let sleeve = tube(0.12, 0.3, 0.7);
        let bvh = TriBvh::new(&sleeve.positions, &sleeve.normals, &sleeve.indices);
        let hidden = covered_triangles(&body, &bvh, &CoverSettings::default());
        let tri_y = |t: usize| {
            let i = &body.indices[t * 3..t * 3 + 3];
            i.iter().map(|&v| body.positions[v as usize].y).sum::<f32>() / 3.0
        };
        let n_hidden = hidden.iter().filter(|h| **h).count();
        assert!(n_hidden > 0);
        for (t, h) in hidden.iter().enumerate() {
            let y = tri_y(t);
            if *h {
                assert!((0.3..=0.7).contains(&y), "hidden outside the sleeve at y={y}");
            }
            if (0.42..=0.58).contains(&y) {
                assert!(*h, "the middle of the sleeve should hide the skin (y={y})");
            }
        }
        let trimmed = without_triangles(&body, &hidden);
        assert_eq!(trimmed.indices.len(), body.indices.len() - n_hidden * 3);
        assert!(trimmed.positions.len() < body.positions.len());
    }

    #[test]
    fn far_or_inside_clothes_dont_cover() {
        let body = tube(0.1, 0.0, 1.0);
        let far = tube(0.3, 0.3, 0.7); // 20 cm away
        let bvh = TriBvh::new(&far.positions, &far.normals, &far.indices);
        assert!(covered_triangles(&body, &bvh, &CoverSettings::default()).iter().all(|h| !h));
        let inside = tube(0.08, 0.3, 0.7); // under the skin
        let bvh = TriBvh::new(&inside.positions, &inside.normals, &inside.indices);
        assert!(covered_triangles(&body, &bvh, &CoverSettings::default()).iter().all(|h| !h));
    }
}
