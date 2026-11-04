//! Skeletons (bone hierarchies) and poses.

use glam::{Mat4, Quat, Vec3};

/// Translation / rotation / scale. Used for a bone's local transform.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Trs {
    pub translation: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
}

impl Default for Trs {
    fn default() -> Self {
        Self {
            translation: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
        }
    }
}

impl Trs {
    pub fn from_mat4(m: Mat4) -> Self {
        let (scale, rotation, translation) = m.to_scale_rotation_translation();
        Self {
            translation,
            rotation: rotation.normalize(),
            scale,
        }
    }

    pub fn to_mat4(&self) -> Mat4 {
        Mat4::from_scale_rotation_translation(self.scale, self.rotation, self.translation)
    }
}

/// One bone.
#[derive(Clone, Debug)]
pub struct Joint {
    pub name: String,
    /// Index of the parent joint. Parents always come before children.
    pub parent: Option<usize>,
    /// Local transform in the bind pose (the pose the mesh was skinned in).
    pub bind_local: Trs,
}

/// A bone hierarchy. Joints are stored parents-first, so one pass from
/// start to end is enough to work out world transforms.
#[derive(Clone, Debug, Default)]
pub struct Skeleton {
    pub joints: Vec<Joint>,
}

impl Skeleton {
    pub fn len(&self) -> usize {
        self.joints.len()
    }

    pub fn is_empty(&self) -> bool {
        self.joints.is_empty()
    }

    pub fn find(&self, name: &str) -> Option<usize> {
        self.joints.iter().position(|j| j.name == name)
    }

    pub fn children(&self, index: usize) -> impl Iterator<Item = usize> + '_ {
        self.joints
            .iter()
            .enumerate()
            .filter(move |(_, j)| j.parent == Some(index))
            .map(|(i, _)| i)
    }

    pub fn roots(&self) -> impl Iterator<Item = usize> + '_ {
        self.joints
            .iter()
            .enumerate()
            .filter(|(_, j)| j.parent.is_none())
            .map(|(i, _)| i)
    }

    /// World matrices for the given local transforms.
    pub fn world_matrices(&self, locals: &[Trs]) -> Vec<Mat4> {
        let mut world = Vec::with_capacity(self.joints.len());
        for (i, joint) in self.joints.iter().enumerate() {
            let local = locals[i].to_mat4();
            let m = match joint.parent {
                Some(p) => world[p] * local,
                None => local,
            };
            world.push(m);
        }
        world
    }

    /// World matrices of the bind pose.
    pub fn bind_world_matrices(&self) -> Vec<Mat4> {
        let locals: Vec<Trs> = self.joints.iter().map(|j| j.bind_local).collect();
        self.world_matrices(&locals)
    }

    /// True if this looks like an Unreal Engine 5 / MetaHuman body skeleton.
    pub fn looks_like_ue5_body(&self) -> bool {
        ["pelvis", "spine_01", "upperarm_l", "thigh_l", "head"]
            .iter()
            .all(|n| self.find(n).is_some())
    }
}

/// A pose: an extra rotation on top of each bone's bind rotation.
/// Keeping it as an offset means "reset" is just clearing the offsets.
#[derive(Clone, Debug, Default)]
pub struct Pose {
    pub offsets: Vec<Quat>,
}

impl Pose {
    pub fn bind(skeleton: &Skeleton) -> Self {
        Self {
            offsets: vec![Quat::IDENTITY; skeleton.len()],
        }
    }

    pub fn is_bind(&self) -> bool {
        self.offsets
            .iter()
            .all(|q| q.abs_diff_eq(Quat::IDENTITY, 1e-6))
    }

    /// Local transforms for this pose.
    pub fn locals(&self, skeleton: &Skeleton) -> Vec<Trs> {
        skeleton
            .joints
            .iter()
            .zip(&self.offsets)
            .map(|(j, off)| Trs {
                rotation: (j.bind_local.rotation * *off).normalize(),
                ..j.bind_local
            })
            .collect()
    }

    pub fn world_matrices(&self, skeleton: &Skeleton) -> Vec<Mat4> {
        skeleton.world_matrices(&self.locals(skeleton))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn two_bone() -> Skeleton {
        Skeleton {
            joints: vec![
                Joint {
                    name: "root".into(),
                    parent: None,
                    bind_local: Trs::default(),
                },
                Joint {
                    name: "arm".into(),
                    parent: Some(0),
                    bind_local: Trs {
                        translation: Vec3::X,
                        ..Default::default()
                    },
                },
            ],
        }
    }

    #[test]
    fn world_matrices_chain() {
        let s = two_bone();
        let w = s.bind_world_matrices();
        assert!(w[1].w_axis.truncate().abs_diff_eq(Vec3::X, 1e-6));
    }

    #[test]
    fn pose_rotates_children() {
        let s = two_bone();
        let mut pose = Pose::bind(&s);
        pose.offsets[0] = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
        let w = pose.world_matrices(&s);
        assert!(w[1].w_axis.truncate().abs_diff_eq(Vec3::Y, 1e-5));
    }
}
