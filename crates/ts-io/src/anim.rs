//! Reading animations from FBX files (Mixamo downloads, Unreal exports).

use std::collections::HashMap;
use std::path::Path;

use anyhow::{anyhow, bail, Result};
use glam::{Mat4, Vec4};
use ts_core::anim::strip_prefix;
use ts_core::AnimClip;

fn mat(m: &ufbx::Matrix) -> Mat4 {
    Mat4::from_cols(
        Vec4::new(m.m00 as f32, m.m10 as f32, m.m20 as f32, 0.0),
        Vec4::new(m.m01 as f32, m.m11 as f32, m.m21 as f32, 0.0),
        Vec4::new(m.m02 as f32, m.m12 as f32, m.m22 as f32, 0.0),
        Vec4::new(m.m03 as f32, m.m13 as f32, m.m23 as f32, 1.0),
    )
}

/// Longest an animation may be (frames), so a broken file can't eat all memory.
const MAX_FRAMES: usize = 30 * 60 * 5;

/// Load the (longest) animation in an FBX file, sampled at the file's frame
/// rate (30 fps if it doesn't say).
pub fn load_animation(path: &Path) -> Result<AnimClip> {
    let path_str = path
        .to_str()
        .ok_or_else(|| anyhow!("File path is not valid text: {}", path.display()))?;
    let opts = ufbx::LoadOpts {
        target_axes: ufbx::CoordinateAxes::right_handed_y_up(),
        target_unit_meters: 1.0,
        space_conversion: ufbx::SpaceConversion::ModifyGeometry,
        ..Default::default()
    };
    let fbx = ufbx::load_file(path_str, opts)
        .map_err(|e| anyhow!("Could not read {}: {:?}", path.display(), e))?;

    let Some(stack) = fbx
        .anim_stacks
        .iter()
        .max_by(|a, b| (a.time_end - a.time_begin).total_cmp(&(b.time_end - b.time_begin)))
    else {
        bail!("{} has no animation in it.", path.display());
    };

    // Bones: nodes marked as bones, plus anything a skin points at.
    let mut bind_world: HashMap<u32, Mat4> = HashMap::new();
    let mut is_joint: HashMap<u32, bool> = HashMap::new();
    for node in &fbx.nodes {
        if node.bone.is_some() {
            is_joint.insert(node.element.element_id, true);
        }
    }
    for cluster in &fbx.skin_clusters {
        if let Some(bone) = &cluster.bone_node {
            is_joint.insert(bone.element.element_id, true);
            bind_world.insert(bone.element.element_id, mat(&cluster.bind_to_world));
        }
    }
    let joints: Vec<&ufbx::Node> = fbx
        .nodes
        .iter()
        .filter(|n| is_joint.contains_key(&n.element.element_id))
        .map(|n| &**n)
        .collect();
    if joints.is_empty() {
        bail!("{} has no skeleton in it.", path.display());
    }
    let slot: HashMap<u32, usize> = joints
        .iter()
        .enumerate()
        .map(|(i, n)| (n.element.element_id, i))
        .collect();
    let parents: Vec<Option<usize>> = joints
        .iter()
        .map(|n| {
            let mut p = n.parent.as_ref();
            while let Some(pn) = p {
                if let Some(&s) = slot.get(&pn.element.element_id) {
                    return Some(s);
                }
                p = pn.parent.as_ref();
            }
            None
        })
        .collect();
    // Rest pose: the skin's bind pose if there is one (Mixamo "with skin"),
    // else the bones as they are in the file.
    let rest_world: Vec<Mat4> = joints
        .iter()
        .map(|n| {
            bind_world
                .get(&n.element.element_id)
                .copied()
                .unwrap_or_else(|| mat(&n.node_to_world))
        })
        .collect();

    let fps = if fbx.settings.frames_per_second > 1.0 {
        fbx.settings.frames_per_second
    } else {
        30.0
    };
    let length = (stack.time_end - stack.time_begin).max(0.0);
    let count = ((length * fps).round() as usize + 1).min(MAX_FRAMES);
    let mut frames = Vec::with_capacity(count);
    for f in 0..count {
        let time = stack.time_begin + f as f64 / fps;
        let eval = ufbx::evaluate_scene(&fbx, &stack.anim, time, ufbx::EvaluateOpts::default())
            .map_err(|e| anyhow!("Could not play the animation at {time:.2}s: {e:?}"))?;
        frames.push(
            joints
                .iter()
                .map(|n| mat(&eval.nodes[n.element.typed_id as usize].node_to_world))
                .collect(),
        );
    }

    let name = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Animation".into());
    Ok(AnimClip {
        name,
        fps: fps as f32,
        joint_names: joints
            .iter()
            .map(|n| strip_prefix(&n.element.name).to_string())
            .collect(),
        parents,
        rest_world,
        frames,
    })
}
