//! Sculpt brushes for touching up a garment by hand.
//!
//! - **Grab**: drag a soft patch of cloth around.
//! - **Relax**: smooth out wrinkles and bumps (each point moves towards the
//!   average of its neighbours).
//! - **Inflate**: push cloth out along its surface (or in, with a negative amount).
//!
//! All brushes use a soft falloff: full strength at the centre, fading to
//! nothing at the edge of the brush.
//!
//! Split vertices (UV seams) are handled with a [`WeldGraph`], so seams never
//! open up while sculpting.

use glam::Vec3;

use crate::graph::WeldGraph;

/// Brush strength at distance `d` from the centre of a brush of radius `r`:
/// 1 at the centre, smoothly down to 0 at the edge.
pub fn falloff(d: f32, r: f32) -> f32 {
    if r <= 0.0 || d >= r {
        return 0.0;
    }
    let x = 1.0 - (d / r) * (d / r);
    x * x
}

/// Where a ray first hits a triangle mesh.
#[derive(Clone, Copy, Debug)]
pub struct RayHit {
    /// Distance along the ray.
    pub t: f32,
    pub point: Vec3,
    pub triangle: usize,
    /// Flat normal of the hit triangle (facing the ray's origin).
    pub normal: Vec3,
}

/// First hit of the ray `origin + t * dir` on the mesh (both triangle sides count).
pub fn raycast(positions: &[Vec3], indices: &[u32], origin: Vec3, dir: Vec3) -> Option<RayHit> {
    let mut best: Option<RayHit> = None;
    for (ti, t) in indices.chunks_exact(3).enumerate() {
        let (a, b, c) = (
            positions[t[0] as usize],
            positions[t[1] as usize],
            positions[t[2] as usize],
        );
        // Möller–Trumbore.
        let e1 = b - a;
        let e2 = c - a;
        let p = dir.cross(e2);
        let det = e1.dot(p);
        if det.abs() < 1e-12 {
            continue;
        }
        let inv = 1.0 / det;
        let s = origin - a;
        let u = s.dot(p) * inv;
        if !(0.0..=1.0).contains(&u) {
            continue;
        }
        let q = s.cross(e1);
        let v = dir.dot(q) * inv;
        if v < 0.0 || u + v > 1.0 {
            continue;
        }
        let dist = e2.dot(q) * inv;
        if dist <= 0.0 || best.is_some_and(|h| h.t <= dist) {
            continue;
        }
        let mut n = e1.cross(e2).normalize_or_zero();
        if n.dot(dir) > 0.0 {
            n = -n;
        }
        best = Some(RayHit {
            t: dist,
            point: origin + dir * dist,
            triangle: ti,
            normal: n,
        });
    }
    best
}

/// Vertices under a brush at `center` with their falloff weights.
pub fn brush_weights(positions: &[Vec3], center: Vec3, radius: f32) -> Vec<(u32, f32)> {
    positions
        .iter()
        .enumerate()
        .filter_map(|(i, p)| {
            let w = falloff(p.distance(center), radius);
            (w > 0.0).then_some((i as u32, w))
        })
        .collect()
}

/// A grab in progress: which vertices move, how much, and where they started.
pub struct GrabStroke {
    pub weights: Vec<(u32, f32)>,
    pub start: Vec<Vec3>,
}

impl GrabStroke {
    pub fn begin(positions: &[Vec3], center: Vec3, radius: f32) -> Self {
        let weights = brush_weights(positions, center, radius);
        let start = weights
            .iter()
            .map(|(i, _)| positions[*i as usize])
            .collect();
        Self { weights, start }
    }

    /// Move the grabbed patch by `offset` (total since the stroke began).
    pub fn apply(&self, positions: &mut [Vec3], offset: Vec3) {
        for ((i, w), s) in self.weights.iter().zip(&self.start) {
            positions[*i as usize] = *s + offset * *w;
        }
    }
}

/// One dab of the relax brush. `strength` 0..1 is how far each point moves
/// towards its neighbours' average at the centre of the brush.
pub fn relax(positions: &mut [Vec3], graph: &WeldGraph, center: Vec3, radius: f32, strength: f32) {
    let welded = graph.gather(positions);
    let mut next = welded.clone();
    for (i, p) in welded.iter().enumerate() {
        let w = falloff(p.distance(center), radius) * strength.clamp(0.0, 1.0);
        let n = &graph.neighbours[i];
        if w <= 0.0 || n.is_empty() {
            continue;
        }
        let avg = n.iter().map(|&j| welded[j as usize]).sum::<Vec3>() / n.len() as f32;
        next[i] = p.lerp(avg, w);
    }
    for (v, w) in positions.iter_mut().zip(&graph.weld_of) {
        *v = next[*w as usize];
    }
}

/// One dab of the inflate brush: moves points along the surface normal by up
/// to `amount` meters (negative = deflate).
pub fn inflate(
    positions: &mut [Vec3],
    indices: &[u32],
    graph: &WeldGraph,
    center: Vec3,
    radius: f32,
    amount: f32,
) {
    let welded = graph.gather(positions);
    let welded_indices: Vec<u32> = indices.iter().map(|&i| graph.weld_of[i as usize]).collect();
    let normals = ts_core::mesh::smooth_normals(&welded, &welded_indices);
    let mut next = welded.clone();
    for (i, p) in welded.iter().enumerate() {
        let w = falloff(p.distance(center), radius);
        if w > 0.0 {
            next[i] = *p + normals[i] * amount * w;
        }
    }
    for (v, w) in positions.iter_mut().zip(&graph.weld_of) {
        *v = next[*w as usize];
    }
}

/// One dab of the shrink brush: pulls loose cloth in towards the surface
/// under it (body and lower layers), stopping `gap` meters away from it.
/// Cloth that's already close or inside is left alone.
pub fn shrink(
    positions: &mut [Vec3],
    graph: &WeldGraph,
    surface: &crate::TriBvh,
    center: Vec3,
    radius: f32,
    strength: f32,
    gap: f32,
) {
    let welded = graph.gather(positions);
    let mut next = welded.clone();
    for (i, p) in welded.iter().enumerate() {
        let w = falloff(p.distance(center), radius) * strength.clamp(0.0, 1.0);
        if w <= 0.0 {
            continue;
        }
        let Some(hit) = surface.closest(*p) else { continue };
        let out = (*p - hit.point).dot(hit.normal);
        if out <= gap {
            continue;
        }
        let target = hit.point + hit.normal * gap;
        next[i] = p.lerp(target, w);
    }
    for (v, w) in positions.iter_mut().zip(&graph.weld_of) {
        *v = next[*w as usize];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A flat grid of n x n points in the XZ plane, 1 m across.
    fn grid(n: usize) -> (Vec<Vec3>, Vec<u32>) {
        let mut p = Vec::new();
        for z in 0..n {
            for x in 0..n {
                p.push(Vec3::new(
                    x as f32 / (n - 1) as f32 - 0.5,
                    0.0,
                    z as f32 / (n - 1) as f32 - 0.5,
                ));
            }
        }
        let mut idx = Vec::new();
        for z in 0..n - 1 {
            for x in 0..n - 1 {
                let i = (z * n + x) as u32;
                let n = n as u32;
                idx.extend([i, i + n, i + 1, i + 1, i + n, i + n + 1]);
            }
        }
        (p, idx)
    }

    #[test]
    fn falloff_shape() {
        assert_eq!(falloff(0.0, 1.0), 1.0);
        assert_eq!(falloff(1.0, 1.0), 0.0);
        assert!(falloff(0.5, 1.0) > 0.4 && falloff(0.5, 1.0) < 0.7);
    }

    #[test]
    fn ray_hits_grid() {
        let (p, i) = grid(5);
        let h = raycast(&p, &i, Vec3::new(0.1, 1.0, 0.1), -Vec3::Y).unwrap();
        assert!((h.t - 1.0).abs() < 1e-5 && h.normal.y > 0.99);
        assert!(raycast(&p, &i, Vec3::new(2.0, 1.0, 0.0), -Vec3::Y).is_none());
    }

    #[test]
    fn grab_moves_centre_most() {
        let (mut p, _) = grid(11);
        let stroke = GrabStroke::begin(&p, Vec3::ZERO, 0.3);
        stroke.apply(&mut p, Vec3::Y * 0.1);
        let centre = p[5 * 11 + 5];
        let edge = p[5 * 11 + 7];
        assert!((centre.y - 0.1).abs() < 1e-6);
        assert!(edge.y > 0.0 && edge.y < 0.1);
        assert_eq!(p[0].y, 0.0);
    }

    #[test]
    fn relax_flattens_a_bump() {
        let (mut p, i) = grid(11);
        p[5 * 11 + 5].y = 0.2;
        let g = WeldGraph::new(&p, &i, 1e-4);
        relax(&mut p, &g, Vec3::ZERO, 0.6, 1.0);
        assert!(p[5 * 11 + 5].y < 0.1);
    }

    #[test]
    fn shrink_pulls_loose_cloth_to_the_surface() {
        // Floor at y = 0 (facing up), cloth 10 cm above it.
        let (floor, fi) = grid(5);
        let floor_n = vec![Vec3::Y; floor.len()];
        let surface = crate::TriBvh::new(&floor, &floor_n, &fi);
        let (mut p, i) = grid(11);
        for v in &mut p {
            v.y = 0.1;
        }
        let g = WeldGraph::new(&p, &i, 1e-4);
        for _ in 0..20 {
            shrink(&mut p, &g, &surface, Vec3::new(0.0, 0.1, 0.0), 0.3, 0.5, 0.01);
        }
        let centre = p[5 * 11 + 5].y;
        assert!((centre - 0.01).abs() < 0.002, "centre at {centre}");
        assert_eq!(p[0].y, 0.1, "outside the brush stays put");
    }

    #[test]
    fn inflate_moves_along_normal() {
        let (mut p, i) = grid(11);
        let g = WeldGraph::new(&p, &i, 1e-4);
        inflate(&mut p, &i, &g, Vec3::ZERO, 0.3, 0.05);
        // Grid winding faces -Y or +Y; either way the centre moved 5 cm.
        assert!((p[5 * 11 + 5].y.abs() - 0.05).abs() < 1e-4);
    }
}
