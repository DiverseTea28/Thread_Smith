//! A loaded file: some mesh parts sharing one skeleton.

use glam::{Mat4, Vec3};

use crate::{Mesh, Pose, Skeleton};

#[derive(Clone, Debug)]
pub struct MeshPart {
    pub name: String,
    pub mesh: Mesh,
    pub visible: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Scene {
    pub name: String,
    pub parts: Vec<MeshPart>,
    pub skeleton: Skeleton,
}

#[derive(Clone, Copy, Debug)]
pub struct Bounds {
    pub min: Vec3,
    pub max: Vec3,
}

impl Bounds {
    pub fn center(&self) -> Vec3 {
        (self.min + self.max) * 0.5
    }

    pub fn radius(&self) -> f32 {
        ((self.max - self.min).length() * 0.5).max(0.01)
    }

    pub fn from_points(points: impl IntoIterator<Item = Vec3>) -> Option<Self> {
        let mut it = points.into_iter();
        let first = it.next()?;
        let mut b = Bounds {
            min: first,
            max: first,
        };
        for p in it {
            b.min = b.min.min(p);
            b.max = b.max.max(p);
        }
        Some(b)
    }
}

impl Scene {
    pub fn vertex_count(&self) -> usize {
        self.parts.iter().map(|p| p.mesh.vertex_count()).sum()
    }

    pub fn triangle_count(&self) -> usize {
        self.parts.iter().map(|p| p.mesh.triangle_count()).sum()
    }

    /// Bounds of the visible parts in the given pose.
    pub fn bounds(&self, pose: &Pose) -> Option<Bounds> {
        let joints: Vec<Mat4> = pose.world_matrices(&self.skeleton);
        let mut pts = Vec::new();
        for part in self.parts.iter().filter(|p| p.visible) {
            pts.extend(part.mesh.skinned(&joints).0);
        }
        if pts.is_empty() {
            pts.extend(joints.iter().map(|m| m.w_axis.truncate()));
        }
        Bounds::from_points(pts)
    }
}
