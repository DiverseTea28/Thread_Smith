//! A loaded file: some mesh parts sharing one skeleton.

use glam::{Mat4, Vec3};

use crate::{Mesh, Pose, Skeleton};

/// What a mesh part is for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PartRole {
    /// The character's body (what clothes are fitted to).
    #[default]
    Body,
    /// A piece of clothing.
    Garment,
    /// The body a garment was originally made on (only used for fitting).
    Reference,
    /// The character's head / face. Shown and posed, but clothes are not
    /// fitted to it.
    Head,
}

#[derive(Clone, Debug)]
pub struct MeshPart {
    pub name: String,
    pub mesh: Mesh,
    pub visible: bool,
    pub role: PartRole,
}

impl MeshPart {
    pub fn new(name: impl Into<String>, mesh: Mesh) -> Self {
        Self {
            name: name.into(),
            mesh,
            visible: true,
            role: PartRole::Body,
        }
    }
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

impl Scene {
    /// Add the meshes from `other` (for example a MetaHuman head exported on
    /// its own) to this scene, moving their skin weights onto this scene's
    /// skeleton by bone name. Bones this skeleton doesn't have (face bones)
    /// are moved to their closest ancestor that it does have.
    pub fn add_skinned_parts(&mut self, other: Scene, role: PartRole) -> usize {
        let bind_world = self.skeleton.bind_world_matrices();
        // other joint -> our joint
        let map: Vec<usize> = (0..other.skeleton.len())
            .map(|j| {
                let mut cur = Some(j);
                while let Some(c) = cur {
                    if let Some(k) = self.skeleton.find(&other.skeleton.joints[c].name) {
                        return k;
                    }
                    cur = other.skeleton.joints[c].parent;
                }
                0
            })
            .collect();
        let mut added = 0;
        for mut part in other.parts {
            if let Some(skin) = part.mesh.skin.take() {
                let mut inverse_bind: Vec<Mat4> = bind_world.iter().map(|m| m.inverse()).collect();
                // Use the file's own bind matrix where the bone matches by name.
                for (j, &k) in map.iter().enumerate() {
                    if other.skeleton.joints[j].name == self.skeleton.joints[k].name {
                        if let Some(ib) = skin.inverse_bind.get(j) {
                            inverse_bind[k] = *ib;
                        }
                    }
                }
                let mut joints = skin.joints.clone();
                for row in joints.iter_mut() {
                    for v in row.iter_mut() {
                        *v = map.get(*v as usize).copied().unwrap_or(0) as u16;
                    }
                }
                let mut new_skin = crate::Skin {
                    joints,
                    weights: skin.weights,
                    inverse_bind,
                };
                new_skin.normalize();
                part.mesh.skin = Some(new_skin);
            }
            part.role = role;
            self.parts.push(part);
            added += 1;
        }
        added
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Joint, Skin, Trs, MAX_INFLUENCES};
    use glam::Vec3;

    fn skel(names: &[(&str, Option<usize>)]) -> Skeleton {
        Skeleton {
            joints: names
                .iter()
                .map(|(n, p)| Joint {
                    name: n.to_string(),
                    parent: *p,
                    bind_local: Trs::default(),
                })
                .collect(),
        }
    }

    #[test]
    fn head_bones_map_by_name_and_ancestor() {
        let mut body = Scene {
            skeleton: skel(&[("root", None), ("neck_01", Some(0)), ("head", Some(1))]),
            ..Default::default()
        };
        // The head file has an extra face bone under "head", and a different order.
        let head_skel = skel(&[("root", None), ("head", Some(0)), ("FACIAL_C_Jaw", Some(1))]);
        let mut joints = vec![[0u16; MAX_INFLUENCES]; 2];
        let mut weights = vec![[0f32; MAX_INFLUENCES]; 2];
        joints[0][0] = 1; // head
        joints[1][0] = 2; // jaw
        weights[0][0] = 1.0;
        weights[1][0] = 1.0;
        let mesh = Mesh {
            positions: vec![Vec3::ZERO, Vec3::Y],
            skin: Some(Skin {
                joints,
                weights,
                inverse_bind: vec![Mat4::IDENTITY; 3],
            }),
            ..Default::default()
        };
        let head = Scene {
            parts: vec![MeshPart::new("Face", mesh)],
            skeleton: head_skel,
            ..Default::default()
        };
        assert_eq!(body.add_skinned_parts(head, PartRole::Head), 1);
        let skin = body.parts[0].mesh.skin.as_ref().unwrap();
        assert_eq!(skin.joints[0][0], 2); // "head" in the body skeleton
        assert_eq!(skin.joints[1][0], 2); // jaw -> its parent "head"
        assert_eq!(skin.inverse_bind.len(), 3);
        assert_eq!(body.parts[0].role, PartRole::Head);
    }
}
