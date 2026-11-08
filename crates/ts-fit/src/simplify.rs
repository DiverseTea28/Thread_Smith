//! Fewer triangles: shrink a garment's triangle count while keeping its
//! shape, UVs and skin weights. Openings (sleeves, neck) are kept exactly.

use ts_core::{Mesh, Skin};

/// The result of simplifying: new triangles, and which old vertices are kept.
#[derive(Clone, Debug)]
pub struct Simplified {
    /// Triangles, indexing into `keep`.
    pub indices: Vec<u32>,
    /// For each new vertex, the old vertex it comes from.
    pub keep: Vec<u32>,
}

/// Work out a simpler version of `mesh` with about `ratio` (0..1) of its
/// triangles. Use [`apply`] to build the new mesh (and the same `Simplified`
/// can be applied to other meshes with the same vertices, e.g. the garment
/// before it was fitted).
pub fn simplify(mesh: &Mesh, ratio: f32, keep_openings: bool) -> Simplified {
    let ratio = ratio.clamp(0.01, 1.0);
    let flat: Vec<f32> = mesh.positions.iter().flat_map(|p| [p.x, p.y, p.z]).collect();
    let bytes: &[u8] = bytemuck_f32(&flat);
    let target = ((mesh.indices.len() as f32 * ratio) as usize / 3 * 3).max(3);
    let options = if keep_openings {
        meshopt::SimplifyOptions::LockBorder
    } else {
        meshopt::SimplifyOptions::None
    };
    let indices = match meshopt::VertexDataAdapter::new(bytes, 12, 0) {
        // Error target is relative to the mesh size; 5% lets it reach low counts.
        Ok(adapter) if ratio < 0.999 => meshopt::simplify(&mesh.indices, &adapter, target, 0.05, options, None),
        _ => mesh.indices.clone(),
    };
    compact(&indices, mesh.positions.len())
}

/// Drop vertices no triangle uses any more.
fn compact(indices: &[u32], vertex_count: usize) -> Simplified {
    let mut new_of = vec![u32::MAX; vertex_count];
    let mut keep = Vec::new();
    let indices = indices
        .iter()
        .map(|&i| {
            let slot = &mut new_of[i as usize];
            if *slot == u32::MAX {
                *slot = keep.len() as u32;
                keep.push(i);
            }
            *slot
        })
        .collect();
    Simplified { indices, keep }
}

/// Build the simpler mesh. `mesh` must have the same vertices as the one
/// that was simplified.
pub fn apply(mesh: &Mesh, s: &Simplified) -> Mesh {
    let pick = |len: usize| len == mesh.positions.len();
    let gather = |v: &[glam::Vec3]| s.keep.iter().map(|&i| v[i as usize]).collect::<Vec<_>>();
    let mut out = Mesh {
        positions: gather(&mesh.positions),
        normals: if pick(mesh.normals.len()) { gather(&mesh.normals) } else { Vec::new() },
        uvs: if pick(mesh.uvs.len()) { s.keep.iter().map(|&i| mesh.uvs[i as usize]).collect() } else { Vec::new() },
        indices: s.indices.clone(),
        skin: mesh.skin.as_ref().map(|skin| Skin {
            joints: s.keep.iter().map(|&i| skin.joints[i as usize]).collect(),
            weights: s.keep.iter().map(|&i| skin.weights[i as usize]).collect(),
            inverse_bind: skin.inverse_bind.clone(),
        }),
    };
    if out.normals.is_empty() {
        out.compute_normals();
    }
    out
}

fn bytemuck_f32(v: &[f32]) -> &[u8] {
    // SAFETY: f32 has no padding and any byte pattern is a valid u8.
    unsafe { std::slice::from_raw_parts(v.as_ptr().cast::<u8>(), std::mem::size_of_val(v)) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::{Vec2, Vec3};

    /// An open tube (like a sleeve): n around, m along.
    fn tube(n: usize, m: usize) -> Mesh {
        let mut positions = Vec::new();
        let mut uvs = Vec::new();
        for j in 0..m {
            for i in 0..n {
                let a = i as f32 / n as f32 * std::f32::consts::TAU;
                positions.push(Vec3::new(a.cos() * 0.1, j as f32 / (m - 1) as f32 * 0.5, a.sin() * 0.1));
                uvs.push(Vec2::new(i as f32 / n as f32, j as f32 / (m - 1) as f32));
            }
        }
        let mut indices = Vec::new();
        for j in 0..m - 1 {
            for i in 0..n {
                let a = (j * n + i) as u32;
                let b = (j * n + (i + 1) % n) as u32;
                let c = a + n as u32;
                let d = b + n as u32;
                indices.extend([a, c, b, b, c, d]);
            }
        }
        let mut mesh = Mesh { positions, uvs, indices, ..Default::default() };
        mesh.compute_normals();
        mesh
    }

    #[test]
    fn halves_the_triangles_and_keeps_the_openings() {
        let mesh = tube(48, 40);
        let s = simplify(&mesh, 0.25, true);
        let out = apply(&mesh, &s);
        let before = mesh.triangle_count();
        let after = out.triangle_count();
        assert!(after < before / 2, "{before} -> {after}");
        assert_eq!(out.uvs.len(), out.positions.len());
        // Every point of both rims is still there.
        for p in mesh.positions.iter().filter(|p| p.y < 1e-6 || p.y > 0.5 - 1e-6) {
            assert!(out.positions.iter().any(|q| q.distance(*p) < 1e-6));
        }
        // Still the same tube: all points stay on the 10 cm radius.
        for q in &out.positions {
            assert!(((q.x * q.x + q.z * q.z).sqrt() - 0.1).abs() < 0.01);
        }
    }

    #[test]
    fn ratio_one_changes_nothing() {
        let mesh = tube(12, 5);
        let out = apply(&mesh, &simplify(&mesh, 1.0, true));
        assert_eq!(out.triangle_count(), mesh.triangle_count());
        assert_eq!(out.positions.len(), mesh.positions.len());
    }
}
