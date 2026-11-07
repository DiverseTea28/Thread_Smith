//! Bounding volume hierarchy over triangles, for "closest point on this mesh"
//! questions. Fitting and skinning ask it hundreds of thousands of times.

use glam::Vec3;

#[derive(Clone, Copy, Debug)]
pub struct ClosestHit {
    pub point: Vec3,
    pub triangle: usize,
    /// Barycentric weights of `point` on the triangle's three corners.
    pub bary: Vec3,
    /// Smooth surface normal at `point` (blended from the corner normals).
    pub normal: Vec3,
    pub distance: f32,
}

#[derive(Clone, Copy)]
struct Node {
    min: Vec3,
    max: Vec3,
    /// Leaf: first triangle in `order`. Inner node: index of the left child
    /// (the right child is always next to it).
    first: u32,
    /// Number of triangles for a leaf, 0 for an inner node.
    count: u32,
}

pub struct TriBvh {
    pub positions: Vec<Vec3>,
    pub normals: Vec<Vec3>,
    pub triangles: Vec<[u32; 3]>,
    nodes: Vec<Node>,
    order: Vec<u32>,
}

const LEAF_SIZE: usize = 4;

impl TriBvh {
    /// `normals` may be empty; then face normals are used.
    pub fn new(positions: &[Vec3], normals: &[Vec3], indices: &[u32]) -> Self {
        let triangles: Vec<[u32; 3]> = indices
            .chunks_exact(3)
            .map(|t| [t[0], t[1], t[2]])
            .collect();
        let normals = if normals.len() == positions.len() {
            normals.to_vec()
        } else {
            ts_core::mesh::smooth_normals(positions, indices)
        };
        let mut bvh = Self {
            positions: positions.to_vec(),
            normals,
            triangles,
            nodes: Vec::new(),
            order: Vec::new(),
        };
        let centroids: Vec<Vec3> = bvh
            .triangles
            .iter()
            .map(|t| {
                (positions[t[0] as usize] + positions[t[1] as usize] + positions[t[2] as usize])
                    / 3.0
            })
            .collect();
        bvh.order = (0..bvh.triangles.len() as u32).collect();
        if !bvh.triangles.is_empty() {
            bvh.nodes.push(Node {
                min: Vec3::ZERO,
                max: Vec3::ZERO,
                first: 0,
                count: 0,
            });
            bvh.build(0, 0, bvh.triangles.len(), &centroids);
        }
        bvh
    }

    fn tri_bounds(&self, t: usize) -> (Vec3, Vec3) {
        let [a, b, c] = self.triangles[t].map(|i| self.positions[i as usize]);
        (a.min(b).min(c), a.max(b).max(c))
    }

    fn build(&mut self, node: usize, start: usize, end: usize, centroids: &[Vec3]) {
        let mut min = Vec3::splat(f32::MAX);
        let mut max = Vec3::splat(f32::MIN);
        let mut cmin = Vec3::splat(f32::MAX);
        let mut cmax = Vec3::splat(f32::MIN);
        for &t in &self.order[start..end] {
            let (a, b) = self.tri_bounds(t as usize);
            min = min.min(a);
            max = max.max(b);
            cmin = cmin.min(centroids[t as usize]);
            cmax = cmax.max(centroids[t as usize]);
        }
        let count = end - start;
        if count <= LEAF_SIZE {
            self.nodes[node] = Node {
                min,
                max,
                first: start as u32,
                count: count as u32,
            };
            return;
        }
        // Split at the middle of the longest side (by triangle count).
        let extent = cmax - cmin;
        let axis = if extent.x >= extent.y && extent.x >= extent.z {
            0
        } else if extent.y >= extent.z {
            1
        } else {
            2
        };
        let mid = start + count / 2;
        self.order[start..end].select_nth_unstable_by(count / 2, |a, b| {
            centroids[*a as usize][axis].total_cmp(&centroids[*b as usize][axis])
        });
        let left = self.nodes.len();
        self.nodes.push(Node {
            min: Vec3::ZERO,
            max: Vec3::ZERO,
            first: 0,
            count: 0,
        });
        self.nodes.push(Node {
            min: Vec3::ZERO,
            max: Vec3::ZERO,
            first: 0,
            count: 0,
        });
        self.nodes[node] = Node {
            min,
            max,
            first: left as u32,
            count: 0,
        };
        self.build(left, start, mid, centroids);
        self.build(left + 1, mid, end, centroids);
    }

    /// Closest point on the mesh to `p`. `None` only for an empty mesh.
    pub fn closest(&self, p: Vec3) -> Option<ClosestHit> {
        if self.nodes.is_empty() {
            return None;
        }
        let mut best_d2 = f32::MAX;
        let mut best: Option<(usize, Vec3, Vec3)> = None;
        let mut stack = Vec::with_capacity(64);
        stack.push(0usize);
        while let Some(n) = stack.pop() {
            let node = self.nodes[n];
            if box_dist2(p, node.min, node.max) >= best_d2 {
                continue;
            }
            if node.count > 0 {
                for &t in &self.order[node.first as usize..(node.first + node.count) as usize] {
                    let [a, b, c] = self.triangles[t as usize].map(|i| self.positions[i as usize]);
                    let (q, bary) = closest_on_triangle(p, a, b, c);
                    let d2 = (q - p).length_squared();
                    if d2 < best_d2 {
                        best_d2 = d2;
                        best = Some((t as usize, q, bary));
                    }
                }
            } else {
                // Visit the nearer child first so the far one is often skipped.
                let (l, r) = (node.first as usize, node.first as usize + 1);
                let dl = box_dist2(p, self.nodes[l].min, self.nodes[l].max);
                let dr = box_dist2(p, self.nodes[r].min, self.nodes[r].max);
                if dl < dr {
                    stack.push(r);
                    stack.push(l);
                } else {
                    stack.push(l);
                    stack.push(r);
                }
            }
        }
        let (t, point, bary) = best?;
        let [i, j, k] = self.triangles[t].map(|i| i as usize);
        let mut normal =
            (self.normals[i] * bary.x + self.normals[j] * bary.y + self.normals[k] * bary.z)
                .normalize_or_zero();
        if normal == Vec3::ZERO {
            let [a, b, c] = [self.positions[i], self.positions[j], self.positions[k]];
            normal = (b - a).cross(c - a).normalize_or(Vec3::Y);
        }
        Some(ClosestHit {
            point,
            triangle: t,
            bary,
            normal,
            distance: best_d2.sqrt(),
        })
    }
}

fn box_dist2(p: Vec3, min: Vec3, max: Vec3) -> f32 {
    let d = (min - p).max(Vec3::ZERO).max(p - max);
    d.length_squared()
}

/// Closest point on triangle abc to p, and its barycentric weights.
/// (Christer Ericson, "Real-Time Collision Detection", 5.1.5.)
pub fn closest_on_triangle(p: Vec3, a: Vec3, b: Vec3, c: Vec3) -> (Vec3, Vec3) {
    let ab = b - a;
    let ac = c - a;
    let ap = p - a;
    let d1 = ab.dot(ap);
    let d2 = ac.dot(ap);
    if d1 <= 0.0 && d2 <= 0.0 {
        return (a, Vec3::X);
    }
    let bp = p - b;
    let d3 = ab.dot(bp);
    let d4 = ac.dot(bp);
    if d3 >= 0.0 && d4 <= d3 {
        return (b, Vec3::Y);
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        let v = d1 / (d1 - d3);
        return (a + ab * v, Vec3::new(1.0 - v, v, 0.0));
    }
    let cp = p - c;
    let d5 = ab.dot(cp);
    let d6 = ac.dot(cp);
    if d6 >= 0.0 && d5 <= d6 {
        return (c, Vec3::Z);
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        let w = d2 / (d2 - d6);
        return (a + ac * w, Vec3::new(1.0 - w, 0.0, w));
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
        return (b + (c - b) * w, Vec3::new(0.0, 1.0 - w, w));
    }
    let denom = 1.0 / (va + vb + vc);
    let v = vb * denom;
    let w = vc * denom;
    (a + ab * v + ac * w, Vec3::new(1.0 - v - w, v, w))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tiny random generator so tests need no extra crates.
    fn rand(seed: &mut u32) -> f32 {
        *seed ^= *seed << 13;
        *seed ^= *seed >> 17;
        *seed ^= *seed << 5;
        (*seed as f32 / u32::MAX as f32) * 2.0 - 1.0
    }

    #[test]
    fn matches_brute_force() {
        let mut s = 12345u32;
        let positions: Vec<Vec3> = (0..600)
            .map(|_| Vec3::new(rand(&mut s), rand(&mut s), rand(&mut s)))
            .collect();
        let indices: Vec<u32> = (0..600).collect();
        let bvh = TriBvh::new(&positions, &[], &indices);
        for _ in 0..200 {
            let p = Vec3::new(rand(&mut s), rand(&mut s), rand(&mut s)) * 1.5;
            let hit = bvh.closest(p).unwrap();
            let brute = indices
                .chunks_exact(3)
                .map(|t| {
                    let (q, _) = closest_on_triangle(
                        p,
                        positions[t[0] as usize],
                        positions[t[1] as usize],
                        positions[t[2] as usize],
                    );
                    (q - p).length()
                })
                .fold(f32::MAX, f32::min);
            assert!(
                (hit.distance - brute).abs() < 1e-5,
                "{} vs {}",
                hit.distance,
                brute
            );
        }
    }
}
