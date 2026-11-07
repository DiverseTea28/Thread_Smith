//! Vertex neighbours for smoothing and weight filling.
//!
//! Meshes often split one corner into several vertices (at UV seams and hard
//! edges). Those copies sit at the same spot. Here they are joined into one
//! "welded" vertex so smoothing and skinning treat the surface as one piece.

use std::collections::HashMap;

use glam::Vec3;

pub struct WeldGraph {
    /// Mesh vertex -> welded vertex.
    pub weld_of: Vec<u32>,
    /// Welded vertex -> one mesh vertex it stands for.
    pub representative: Vec<u32>,
    /// Welded vertex -> its neighbours (welded indices).
    pub neighbours: Vec<Vec<u32>>,
}

impl WeldGraph {
    /// `tolerance`: vertices closer than this (meters) count as the same point.
    pub fn new(positions: &[Vec3], indices: &[u32], tolerance: f32) -> Self {
        let inv = 1.0 / tolerance.max(1e-7);
        let key = |p: Vec3| {
            (
                (p.x * inv).round() as i64,
                (p.y * inv).round() as i64,
                (p.z * inv).round() as i64,
            )
        };
        let mut map: HashMap<(i64, i64, i64), u32> = HashMap::new();
        let mut weld_of = Vec::with_capacity(positions.len());
        let mut representative = Vec::new();
        for (i, p) in positions.iter().enumerate() {
            let id = *map.entry(key(*p)).or_insert_with(|| {
                representative.push(i as u32);
                (representative.len() - 1) as u32
            });
            weld_of.push(id);
        }
        let mut neighbours: Vec<Vec<u32>> = vec![Vec::new(); representative.len()];
        for t in indices.chunks_exact(3) {
            let w = [
                weld_of[t[0] as usize],
                weld_of[t[1] as usize],
                weld_of[t[2] as usize],
            ];
            for (a, b) in [(w[0], w[1]), (w[1], w[2]), (w[2], w[0])] {
                if a != b {
                    neighbours[a as usize].push(b);
                    neighbours[b as usize].push(a);
                }
            }
        }
        for n in &mut neighbours {
            n.sort_unstable();
            n.dedup();
        }
        Self {
            weld_of,
            representative,
            neighbours,
        }
    }

    pub fn len(&self) -> usize {
        self.representative.len()
    }

    pub fn is_empty(&self) -> bool {
        self.representative.is_empty()
    }

    /// Smooth a per-welded-vertex field: each pass moves values halfway to
    /// their neighbours' average.
    pub fn smooth(&self, values: &mut [Vec3], passes: usize) {
        let mut next = values.to_vec();
        for _ in 0..passes {
            for (i, n) in self.neighbours.iter().enumerate() {
                if n.is_empty() {
                    continue;
                }
                let avg = n.iter().map(|&j| values[j as usize]).sum::<Vec3>() / n.len() as f32;
                next[i] = values[i] * 0.5 + avg * 0.5;
            }
            values.copy_from_slice(&next);
        }
    }

    /// Spread welded values back out to every mesh vertex.
    pub fn expand<T: Copy>(&self, welded: &[T]) -> Vec<T> {
        self.weld_of.iter().map(|&w| welded[w as usize]).collect()
    }

    /// Welded positions (taken from each group's representative).
    pub fn gather(&self, per_vertex: &[Vec3]) -> Vec<Vec3> {
        self.representative
            .iter()
            .map(|&r| per_vertex[r as usize])
            .collect()
    }
}
