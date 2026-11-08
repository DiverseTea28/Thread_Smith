//! Lattice (free-form deformation): a box of control points around a
//! garment. Moving the points bends everything inside smoothly.

use glam::Vec3;
use ts_core::Bounds;

#[derive(Clone, Debug)]
pub struct Lattice {
    /// The box the lattice was made around.
    pub min: Vec3,
    pub max: Vec3,
    /// Points along each side (2..=5).
    pub size: usize,
    /// Control points, x fastest, then y, then z.
    pub points: Vec<Vec3>,
}

impl Lattice {
    /// A lattice of `size` x `size` x `size` points around `bounds`
    /// (a little bigger, so nothing sits right on the edge).
    pub fn new(bounds: Bounds, size: usize) -> Self {
        let size = size.clamp(2, 5);
        let pad = (bounds.max - bounds.min).max(Vec3::splat(0.01)) * 0.03;
        let min = bounds.min - pad;
        let max = bounds.max + pad;
        let mut lattice = Self { min, max, size, points: Vec::new() };
        lattice.points = (0..size * size * size).map(|i| lattice.rest_point(i)).collect();
        lattice
    }

    pub fn index(&self, x: usize, y: usize, z: usize) -> usize {
        x + self.size * (y + self.size * z)
    }

    pub fn coords(&self, i: usize) -> (usize, usize, usize) {
        let n = self.size;
        (i % n, (i / n) % n, i / (n * n))
    }

    /// Where control point `i` sits before anything is moved.
    pub fn rest_point(&self, i: usize) -> Vec3 {
        let (x, y, z) = self.coords(i);
        let t = Vec3::new(x as f32, y as f32, z as f32) / (self.size - 1) as f32;
        self.min + (self.max - self.min) * t
    }

    pub fn reset(&mut self) {
        self.points = (0..self.points.len()).map(|i| self.rest_point(i)).collect();
    }

    pub fn is_moved(&self) -> bool {
        self.points.iter().enumerate().any(|(i, p)| p.distance_squared(self.rest_point(i)) > 1e-12)
    }

    /// Bend `rest` (positions inside the original box) by the moved points.
    /// Points outside the box move with the nearest edge of the lattice.
    pub fn deform(&self, rest: &[Vec3]) -> Vec<Vec3> {
        let n = self.size;
        let ext = (self.max - self.min).max(Vec3::splat(1e-6));
        let mut bx = vec![0.0f32; n];
        let mut by = vec![0.0f32; n];
        let mut bz = vec![0.0f32; n];
        rest.iter()
            .map(|p| {
                let local = (*p - self.min) / ext;
                let t = local.clamp(Vec3::ZERO, Vec3::ONE);
                // Anything outside the box keeps its offset from the box edge.
                let outside = (local - t) * ext;
                bernstein(n - 1, t.x, &mut bx);
                bernstein(n - 1, t.y, &mut by);
                bernstein(n - 1, t.z, &mut bz);
                let mut q = Vec3::ZERO;
                for z in 0..n {
                    for y in 0..n {
                        let wyz = by[y] * bz[z];
                        if wyz == 0.0 {
                            continue;
                        }
                        for x in 0..n {
                            q += self.points[self.index(x, y, z)] * (bx[x] * wyz);
                        }
                    }
                }
                q + outside
            })
            .collect()
    }

    /// Each pair of neighbouring control points (for drawing the cage).
    pub fn edges(&self) -> Vec<(usize, usize)> {
        let n = self.size;
        let mut e = Vec::new();
        for z in 0..n {
            for y in 0..n {
                for x in 0..n {
                    let i = self.index(x, y, z);
                    if x + 1 < n {
                        e.push((i, self.index(x + 1, y, z)));
                    }
                    if y + 1 < n {
                        e.push((i, self.index(x, y + 1, z)));
                    }
                    if z + 1 < n {
                        e.push((i, self.index(x, y, z + 1)));
                    }
                }
            }
        }
        e
    }
}

/// Bernstein weights of degree `d` at `t` (they add up to 1).
fn bernstein(d: usize, t: f32, out: &mut [f32]) {
    let binom = |n: usize, k: usize| -> f32 {
        (1..=k).fold(1.0f32, |acc, i| acc * (n + 1 - i) as f32 / i as f32)
    };
    for (k, w) in out.iter_mut().enumerate().take(d + 1) {
        *w = binom(d, k) * t.powi(k as i32) * (1.0 - t).powi((d - k) as i32);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cloud() -> Vec<Vec3> {
        (0..50)
            .map(|i| {
                let f = i as f32;
                Vec3::new((f * 0.37).sin(), (f * 0.11).cos() * 2.0, (f * 0.23).sin() * 0.5)
            })
            .collect()
    }

    #[test]
    fn untouched_lattice_changes_nothing() {
        let pts = cloud();
        for size in 2..=5 {
            let l = Lattice::new(Bounds::from_points(pts.clone()).unwrap(), size);
            for (a, b) in pts.iter().zip(l.deform(&pts)) {
                assert!(a.distance(b) < 1e-4, "size {size}: {a} -> {b}");
            }
        }
    }

    #[test]
    fn moving_the_whole_lattice_moves_everything() {
        let pts = cloud();
        let mut l = Lattice::new(Bounds::from_points(pts.clone()).unwrap(), 3);
        for p in &mut l.points {
            *p += Vec3::Y;
        }
        for (a, b) in pts.iter().zip(l.deform(&pts)) {
            assert!((b - *a - Vec3::Y).length() < 1e-4);
        }
    }

    #[test]
    fn moving_one_corner_bends_nearby_points_most() {
        let pts = vec![Vec3::new(-1.0, -1.0, -1.0), Vec3::new(1.0, 1.0, 1.0)];
        let mut l = Lattice::new(Bounds::from_points(pts.clone()).unwrap(), 2);
        let corner = l.index(1, 1, 1);
        l.points[corner] += Vec3::X;
        let out = l.deform(&pts);
        assert!(out[1].x - pts[1].x > 0.9);
        assert!((out[0] - pts[0]).length() < 1e-3);
    }
}
