//! Animations and retargeting (Mixamo -> MetaHuman / UE5 skeleton).
//!
//! An [`AnimClip`] stores every source bone's world transform for every frame.
//! A [`Retargeter`] turns a moment of that clip into local bone transforms for
//! our skeleton:
//!
//! 1. **Facing**: the source character is turned to face the same way as ours
//!    (measured from the hips: left thigh minus right thigh).
//! 2. **Rest difference**: Mixamo is in a T-pose, MetaHumans in an A-pose. For
//!    each limb bone we work out the turn that lines our bone up with the
//!    source bone at rest, so the arms aren't 45 degrees off.
//! 3. **Motion**: each mapped bone gets the same turn (in world space) that
//!    the source bone made since its rest pose. Bones without a match keep
//!    their rest offset from their parent (twist bones, extra spine bones).
//! 4. **Hips movement** is scaled by the difference in hip height.

use std::collections::HashMap;

use glam::{Mat4, Quat, Vec3};

use crate::{Skeleton, Trs};

/// An animation: world transforms of the source skeleton, frame by frame.
#[derive(Clone, Debug)]
pub struct AnimClip {
    pub name: String,
    pub fps: f32,
    /// Source bone names (any "mixamorig:" style prefix removed).
    pub joint_names: Vec<String>,
    pub parents: Vec<Option<usize>>,
    /// World transforms in the rest pose (T-pose for Mixamo).
    pub rest_world: Vec<Mat4>,
    /// frames[f][j] = world transform of bone j at frame f.
    pub frames: Vec<Vec<Mat4>>,
}

impl AnimClip {
    pub fn duration(&self) -> f32 {
        (self.frames.len().saturating_sub(1)) as f32 / self.fps.max(1.0)
    }

    pub fn find(&self, name: &str) -> Option<usize> {
        let n = name.to_lowercase();
        self.joint_names.iter().position(|j| j.to_lowercase() == n)
    }
}

/// Remove a "mixamorig:" (or "mixamorig1:", "Armature|") style prefix.
pub fn strip_prefix(name: &str) -> &str {
    let name = name.rsplit(':').next().unwrap_or(name);
    name.rsplit('|').next().unwrap_or(name)
}

/// Mixamo bone -> UE5 / MetaHuman bone. Left side only; the right side is the
/// same with Left/Right and _l/_r swapped.
const MIXAMO_TO_UE: &[(&str, &str)] = &[
    ("Hips", "pelvis"),
    ("Spine", "spine_01"),
    ("Spine1", "spine_03"),
    ("Spine2", "spine_05"),
    ("Neck", "neck_01"),
    ("Head", "head"),
    ("LeftShoulder", "clavicle_l"),
    ("LeftArm", "upperarm_l"),
    ("LeftForeArm", "lowerarm_l"),
    ("LeftHand", "hand_l"),
    ("LeftHandThumb1", "thumb_01_l"),
    ("LeftHandThumb2", "thumb_02_l"),
    ("LeftHandThumb3", "thumb_03_l"),
    ("LeftHandIndex1", "index_01_l"),
    ("LeftHandIndex2", "index_02_l"),
    ("LeftHandIndex3", "index_03_l"),
    ("LeftHandMiddle1", "middle_01_l"),
    ("LeftHandMiddle2", "middle_02_l"),
    ("LeftHandMiddle3", "middle_03_l"),
    ("LeftHandRing1", "ring_01_l"),
    ("LeftHandRing2", "ring_02_l"),
    ("LeftHandRing3", "ring_03_l"),
    ("LeftHandPinky1", "pinky_01_l"),
    ("LeftHandPinky2", "pinky_02_l"),
    ("LeftHandPinky3", "pinky_03_l"),
    ("LeftUpLeg", "thigh_l"),
    ("LeftLeg", "calf_l"),
    ("LeftFoot", "foot_l"),
    ("LeftToeBase", "ball_l"),
];

/// Bone -> the bone it points at, used to line up limbs at rest (Mixamo names).
const AIM_CHILD: &[(&str, &str)] = &[
    ("LeftShoulder", "LeftArm"),
    ("LeftArm", "LeftForeArm"),
    ("LeftForeArm", "LeftHand"),
    ("LeftHand", "LeftHandMiddle1"),
    ("LeftHandThumb1", "LeftHandThumb2"),
    ("LeftHandThumb2", "LeftHandThumb3"),
    ("LeftHandIndex1", "LeftHandIndex2"),
    ("LeftHandIndex2", "LeftHandIndex3"),
    ("LeftHandMiddle1", "LeftHandMiddle2"),
    ("LeftHandMiddle2", "LeftHandMiddle3"),
    ("LeftHandRing1", "LeftHandRing2"),
    ("LeftHandRing2", "LeftHandRing3"),
    ("LeftHandPinky1", "LeftHandPinky2"),
    ("LeftHandPinky2", "LeftHandPinky3"),
    ("LeftUpLeg", "LeftLeg"),
    ("LeftLeg", "LeftFoot"),
    ("LeftFoot", "LeftToeBase"),
];

fn mirror_mixamo(n: &str) -> String {
    n.replacen("Left", "Right", 1)
}

fn mirror_ue(n: &str) -> String {
    n.strip_suffix("_l")
        .map(|b| format!("{b}_r"))
        .unwrap_or_else(|| n.to_string())
}

/// Both sides of a left-side table.
fn both_sides(table: &[(&str, &str)], mirror_b: fn(&str) -> String) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for (a, b) in table {
        out.push((a.to_string(), b.to_string()));
        if a.contains("Left") {
            out.push((mirror_mixamo(a), mirror_b(b)));
        }
    }
    out
}

/// One matched bone.
#[derive(Clone, Debug)]
struct Link {
    source: usize,
    target: usize,
    /// Turn that lines our bone's rest pose up with the source's.
    rest_fix: Quat,
}

/// Turns an [`AnimClip`] into poses for one target skeleton.
#[derive(Clone, Debug)]
pub struct Retargeter {
    /// Per target bone: index into `links`, if it is driven.
    driven: Vec<Option<usize>>,
    links: Vec<Link>,
    /// Turns the source character to face our way.
    facing: Quat,
    /// Source hips bone and our pelvis.
    hips: Option<(usize, usize)>,
    /// Our hip height / source hip height.
    hip_scale: f32,
    target_rest_world: Vec<Mat4>,
    /// How many of the 52 Mixamo bones were found (for a "looks OK" check).
    pub matched: usize,
}

fn rot(m: &Mat4) -> Quat {
    m.to_scale_rotation_translation().1.normalize()
}

fn pos(m: &Mat4) -> Vec3 {
    m.w_axis.truncate()
}

impl Retargeter {
    pub fn new(clip: &AnimClip, target: &Skeleton) -> Self {
        let target_rest_world = target.bind_world_matrices();
        let pairs = both_sides(MIXAMO_TO_UE, mirror_ue);
        let aims = both_sides(AIM_CHILD, mirror_mixamo);
        let aim_of: HashMap<String, String> = aims.into_iter().collect();

        // Facing: line up the hips' left-right direction (on the floor plane).
        let flat = |v: Vec3| Vec3::new(v.x, 0.0, v.z).normalize_or_zero();
        let side = |a: Option<usize>, b: Option<usize>, w: &[Mat4]| -> Option<Vec3> {
            Some(flat(pos(&w[a?]) - pos(&w[b?])))
        };
        let src_side = side(
            clip.find("LeftUpLeg"),
            clip.find("RightUpLeg"),
            &clip.rest_world,
        );
        let tgt_side = side(
            target.find("thigh_l"),
            target.find("thigh_r"),
            &target_rest_world,
        );
        let facing = match (src_side, tgt_side) {
            (Some(s), Some(t)) if s.length() > 0.5 && t.length() > 0.5 => {
                Quat::from_rotation_arc(s, t)
            }
            _ => Quat::IDENTITY,
        };

        let mut driven = vec![None; target.len()];
        let mut links: Vec<Link> = Vec::new();
        let mut name_of_link: Vec<String> = Vec::new();
        for (src_name, tgt_name) in &pairs {
            let (Some(s), Some(t)) = (clip.find(src_name), target.find(tgt_name)) else {
                continue;
            };
            // Rest fix: turn our bone's aim onto the source bone's aim.
            let mut rest_fix = Quat::IDENTITY;
            if let Some(child) = aim_of.get(src_name) {
                let tgt_child = pairs.iter().find(|(a, _)| a == child).map(|(_, b)| b);
                if let (Some(sc), Some(tc)) =
                    (clip.find(child), tgt_child.and_then(|n| target.find(n)))
                {
                    let ds = facing * (pos(&clip.rest_world[sc]) - pos(&clip.rest_world[s]));
                    let dt = pos(&target_rest_world[tc]) - pos(&target_rest_world[t]);
                    if ds.length() > 1e-5 && dt.length() > 1e-5 {
                        rest_fix = Quat::from_rotation_arc(dt.normalize(), ds.normalize());
                    }
                }
            }
            driven[t] = Some(links.len());
            links.push(Link {
                source: s,
                target: t,
                rest_fix,
            });
            name_of_link.push(src_name.clone());
        }
        // Tip bones read their parent's fix, which may have been set later in
        // the table; do one more pass now that every link exists.
        for i in 0..links.len() {
            let n = &name_of_link[i];
            if (n.contains("Hand") && n.ends_with('3')) || n.contains("ToeBase") {
                if let Some(p) = target.joints[links[i].target]
                    .parent
                    .and_then(|p| driven[p])
                {
                    links[i].rest_fix = links[p].rest_fix;
                }
            }
        }

        let hips = clip.find("Hips").zip(target.find("pelvis"));
        let hip_scale = match hips {
            Some((s, t)) => {
                let floor_s = clip
                    .rest_world
                    .iter()
                    .map(|m| pos(m).y)
                    .fold(f32::MAX, f32::min);
                let hs = pos(&clip.rest_world[s]).y - floor_s.min(0.0);
                let ht = pos(&target_rest_world[t]).y;
                if hs > 1e-3 {
                    ht / hs
                } else {
                    1.0
                }
            }
            None => 1.0,
        };
        Self {
            matched: links.len(),
            driven,
            links,
            facing,
            hips,
            hip_scale,
            target_rest_world,
        }
    }

    /// Source bone world transforms at `time` seconds (blended between frames).
    fn sample(clip: &AnimClip, time: f32) -> Vec<(Quat, Vec3)> {
        let n = clip.frames.len();
        if n == 0 {
            return clip.rest_world.iter().map(|m| (rot(m), pos(m))).collect();
        }
        let f = (time * clip.fps).clamp(0.0, (n - 1) as f32);
        let (a, b) = (f.floor() as usize, (f.ceil() as usize).min(n - 1));
        let t = f - a as f32;
        clip.frames[a]
            .iter()
            .zip(&clip.frames[b])
            .map(|(ma, mb)| (rot(ma).slerp(rot(mb), t), pos(ma).lerp(pos(mb), t)))
            .collect()
    }

    /// Our bones' local transforms at `time` seconds. With `in_place`, the
    /// hips don't travel across the floor (they still move up and down).
    pub fn pose(&self, clip: &AnimClip, target: &Skeleton, time: f32, in_place: bool) -> Vec<Trs> {
        let src = Self::sample(clip, time);
        let mut world: Vec<Mat4> = Vec::with_capacity(target.len());
        let mut locals = Vec::with_capacity(target.len());
        for (j, joint) in target.joints.iter().enumerate() {
            let parent_world = joint.parent.map(|p| world[p]).unwrap_or(Mat4::IDENTITY);
            let mut local = joint.bind_local;
            if let Some(li) = self.driven[j] {
                let link = &self.links[li];
                let src_rest = self.facing * rot(&clip.rest_world[link.source]);
                let src_now = self.facing * src[link.source].0;
                let delta = src_now * src_rest.inverse();
                let world_rot =
                    (delta * link.rest_fix * rot(&self.target_rest_world[j])).normalize();
                let parent_rot = rot(&parent_world);
                local.rotation = (parent_rot.inverse() * world_rot).normalize();

                if let Some((sh, th)) = self.hips {
                    if th == j {
                        let mut moved =
                            self.facing * (src[sh].1 - pos(&clip.rest_world[sh])) * self.hip_scale;
                        if in_place {
                            moved.x = 0.0;
                            moved.z = 0.0;
                        }
                        let world_pos = pos(&self.target_rest_world[j]) + moved;
                        local.translation = parent_world.inverse().transform_point3(world_pos);
                    }
                }
            }
            world.push(parent_world * local.to_mat4());
            locals.push(local);
        }
        locals
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Joint;

    /// A tiny body: hips, two thighs and a left arm, in the given arm pose.
    fn body(names: [&str; 5], arm_down: f32) -> (Vec<String>, Vec<Option<usize>>, Vec<Mat4>) {
        let hips = Mat4::from_translation(Vec3::new(0.0, 1.0, 0.0));
        let thigh_l = Mat4::from_translation(Vec3::new(0.1, 0.95, 0.0));
        let thigh_r = Mat4::from_translation(Vec3::new(-0.1, 0.95, 0.0));
        let arm = Mat4::from_translation(Vec3::new(0.2, 1.4, 0.0));
        // Forearm out along +X, tilted down by `arm_down` radians.
        let fore = Mat4::from_translation(
            Vec3::new(0.2, 1.4, 0.0) + Vec3::new(arm_down.cos(), -arm_down.sin(), 0.0) * 0.3,
        );
        (
            names.iter().map(|s| s.to_string()).collect(),
            vec![None, Some(0), Some(0), Some(0), Some(3)],
            vec![hips, thigh_l, thigh_r, arm, fore],
        )
    }

    fn skeleton_from(names: Vec<String>, parents: Vec<Option<usize>>, world: &[Mat4]) -> Skeleton {
        Skeleton {
            joints: names
                .into_iter()
                .enumerate()
                .map(|(i, name)| Joint {
                    parent: parents[i],
                    bind_local: Trs::from_mat4(match parents[i] {
                        Some(p) => world[p].inverse() * world[i],
                        None => world[i],
                    }),
                    name,
                })
                .collect(),
        }
    }

    #[test]
    fn strips_prefixes() {
        assert_eq!(strip_prefix("mixamorig:Hips"), "Hips");
        assert_eq!(strip_prefix("mixamorig1:LeftArm"), "LeftArm");
        assert_eq!(strip_prefix("pelvis"), "pelvis");
    }

    #[test]
    fn t_pose_rest_puts_a_pose_arm_level() {
        // Source in T-pose (arm level), target in A-pose (arm 45° down).
        let (sn, sp, sw) = body(
            ["Hips", "LeftUpLeg", "RightUpLeg", "LeftArm", "LeftForeArm"],
            0.0,
        );
        let clip = AnimClip {
            name: "t".into(),
            fps: 30.0,
            joint_names: sn,
            parents: sp,
            rest_world: sw.clone(),
            frames: vec![sw.clone(), sw],
        };
        let (tn, tp, tw) = body(
            ["pelvis", "thigh_l", "thigh_r", "upperarm_l", "lowerarm_l"],
            0.785,
        );
        let target = skeleton_from(tn, tp, &tw);
        let r = Retargeter::new(&clip, &target);
        assert_eq!(r.matched, 5);
        let locals = r.pose(&clip, &target, 0.0, false);
        let world = target.world_matrices(&locals);
        let dir = (pos(&world[4]) - pos(&world[3])).normalize();
        // Playing the T-pose frame should lift our arm level (along +X).
        assert!(dir.x > 0.99, "{dir}");
    }

    #[test]
    fn hips_move_and_scale() {
        let (sn, sp, sw) = body(
            ["Hips", "LeftUpLeg", "RightUpLeg", "LeftArm", "LeftForeArm"],
            0.0,
        );
        let mut moved = sw.clone();
        for m in moved.iter_mut() {
            *m = Mat4::from_translation(Vec3::new(0.0, 0.0, 0.5)) * *m;
        }
        let clip = AnimClip {
            name: "walk".into(),
            fps: 1.0,
            joint_names: sn.clone(),
            parents: sp.clone(),
            rest_world: sw.clone(),
            frames: vec![sw.clone(), moved],
        };
        // Target twice as tall.
        let tw: Vec<Mat4> = sw
            .iter()
            .map(|m| Mat4::from_scale(Vec3::splat(2.0)) * *m)
            .collect();
        let tn = vec!["pelvis", "thigh_l", "thigh_r", "upperarm_l", "lowerarm_l"]
            .into_iter()
            .map(String::from)
            .collect();
        let target = skeleton_from(tn, sp, &tw);
        let r = Retargeter::new(&clip, &target);
        let w = target.world_matrices(&r.pose(&clip, &target, 1.0, false));
        assert!((pos(&w[0]).z - 1.0).abs() < 1e-3, "{}", pos(&w[0]));
        let w = target.world_matrices(&r.pose(&clip, &target, 1.0, true));
        assert!(pos(&w[0]).z.abs() < 1e-3);
    }
}
