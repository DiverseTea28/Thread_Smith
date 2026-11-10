//! Live view: turns the engine's previews (a cloud of cubes, then grey
//! meshes) into a mesh the viewport can draw, placed next to the body.

use std::collections::HashSet;

use glam::Vec3;
use ts_core::{Pose, Scene};
use ts_gen::LivePreview;

/// Something to draw for the live view.
pub struct PreviewGeometry {
    pub positions: Vec<Vec3>,
    pub normals: Vec<Vec3>,
    pub color: [f32; 3],
    pub indices: Vec<u32>,
}

/// Where the live view goes: a box of size `scale` centred on `center`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub center: Vec3,
    pub scale: f32,
}

impl Placement {
    /// Next to the body (to its right, at chest height, a bit over half its
    /// height across), or at the origin when nothing is open.
    pub fn beside_body(scene: Option<&Scene>, pose: &Pose) -> Self {
        match scene.and_then(|s| s.bounds(pose)) {
            Some(b) => {
                let height = (b.max.y - b.min.y).max(0.01);
                let scale = height * 0.55;
                Placement {
                    center: Vec3::new(b.max.x + scale * 0.65, b.min.y + height * 0.6, b.center().z),
                    scale,
                }
            }
            None => Placement { center: Vec3::new(0.0, 0.5, 0.0), scale: 1.0 },
        }
    }

    /// From the engine's space (Y up, about -0.5..0.5, front facing -Z) to
    /// the scene: turned around to face the camera like the body does.
    fn place(&self, p: [f32; 3]) -> Vec3 {
        self.center + Vec3::new(-p[0], p[1], -p[2]) * self.scale
    }

    fn turn(n: Vec3) -> Vec3 {
        Vec3::new(-n.x, n.y, -n.z)
    }

    pub fn bounds(&self) -> ts_core::scene::Bounds {
        let h = Vec3::splat(self.scale * 0.5);
        ts_core::scene::Bounds { min: self.center - h, max: self.center + h }
    }
}

const CUBE_COLOR: [f32; 3] = [0.30, 0.66, 0.95];
const MESH_COLOR: [f32; 3] = [0.72, 0.72, 0.74];

pub fn build(preview: &LivePreview, place: Placement) -> PreviewGeometry {
    match preview {
        LivePreview::Voxels { centres, size } => cubes(centres, *size, place),
        LivePreview::Mesh { vertices, indices } => mesh(vertices, indices, place),
    }
}

/// One cube per voxel, leaving out faces two cubes share.
fn cubes(centres: &[[f32; 3]], size: f32, place: Placement) -> PreviewGeometry {
    let size = size.max(1e-4);
    let key = |c: &[f32; 3]| -> (i32, i32, i32) {
        (
            (c[0] / size).floor() as i32,
            (c[1] / size).floor() as i32,
            (c[2] / size).floor() as i32,
        )
    };
    let filled: HashSet<(i32, i32, i32)> = centres.iter().map(key).collect();
    // A little gap between cubes so they read as cubes.
    let half = size * 0.45;
    // neighbour offset, normal, u axis, v axis
    type Face = ([i32; 3], [f32; 3], [f32; 3], [f32; 3]);
    const FACES: [Face; 6] = [
        ([1, 0, 0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]),
        ([-1, 0, 0], [-1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]),
        ([0, 1, 0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]),
        ([0, -1, 0], [0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
        ([0, 0, 1], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
        ([0, 0, -1], [0.0, 0.0, -1.0], [0.0, 1.0, 0.0], [1.0, 0.0, 0.0]),
    ];
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut indices = Vec::new();
    for c in centres {
        let k = key(c);
        let centre = Vec3::from(*c);
        for (off, n, u, v) in FACES {
            if filled.contains(&(k.0 + off[0], k.1 + off[1], k.2 + off[2])) {
                continue;
            }
            let (n, u, v) = (Vec3::from(n), Vec3::from(u), Vec3::from(v));
            let base = positions.len() as u32;
            let mid = centre + n * half;
            for (a, b) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
                positions.push(place.place((mid + u * (a * half) + v * (b * half)).to_array()));
                normals.push(Placement::turn(n));
            }
            // Turning around (x and z flipped) keeps handedness, so the
            // winding stays the same.
            indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        }
    }
    PreviewGeometry { positions, normals, color: CUBE_COLOR, indices }
}

/// A grey mesh with smooth normals.
fn mesh(vertices: &[[f32; 3]], indices: &[u32], place: Placement) -> PreviewGeometry {
    let positions: Vec<Vec3> = vertices.iter().map(|v| place.place(*v)).collect();
    let mut normals = vec![Vec3::ZERO; positions.len()];
    for t in indices.chunks_exact(3) {
        let (a, b, c) = (t[0] as usize, t[1] as usize, t[2] as usize);
        if a >= positions.len() || b >= positions.len() || c >= positions.len() {
            continue;
        }
        let n = (positions[b] - positions[a]).cross(positions[c] - positions[a]);
        normals[a] += n;
        normals[b] += n;
        normals[c] += n;
    }
    for n in &mut normals {
        *n = n.try_normalize().unwrap_or(Vec3::Y);
    }
    let indices = indices.chunks_exact(3).flatten().copied().collect();
    PreviewGeometry { positions, normals, color: MESH_COLOR, indices }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_faces_are_left_out() {
        let place = Placement { center: Vec3::ZERO, scale: 1.0 };
        let one = cubes(&[[0.05, 0.05, 0.05]], 0.1, place);
        assert_eq!(one.indices.len(), 6 * 6);
        let two = cubes(&[[0.05, 0.05, 0.05], [0.15, 0.05, 0.05]], 0.1, place);
        assert_eq!(two.indices.len(), 10 * 6);
        assert_eq!(two.positions.len(), 10 * 4);
    }

    #[test]
    fn cube_faces_point_outwards() {
        let place = Placement { center: Vec3::ZERO, scale: 1.0 };
        let g = cubes(&[[0.0, 0.0, 0.0]], 0.1, place);
        for t in g.indices.chunks_exact(3) {
            let (a, b, c) = (g.positions[t[0] as usize], g.positions[t[1] as usize], g.positions[t[2] as usize]);
            let face_n = (b - a).cross(c - a).normalize();
            assert!(face_n.dot(g.normals[t[0] as usize]) > 0.99, "{face_n} vs {}", g.normals[t[0] as usize]);
            // and away from the centre
            assert!(face_n.dot((a + b + c) / 3.0) > 0.0);
        }
    }

    #[test]
    fn mesh_is_placed_and_turned() {
        let place = Placement { center: Vec3::new(1.0, 2.0, 3.0), scale: 2.0 };
        let g = mesh(&[[0.5, 0.0, 0.0], [0.0, 0.5, 0.0], [0.0, 0.0, 0.5]], &[0, 1, 2], place);
        assert_eq!(g.positions[0], Vec3::new(0.0, 2.0, 3.0));
        assert_eq!(g.positions[1], Vec3::new(1.0, 3.0, 3.0));
        assert_eq!(g.positions[2], Vec3::new(1.0, 2.0, 2.0));
    }
}
