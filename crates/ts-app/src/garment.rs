//! Garments: adding them to the scene, fitting and skinning them.

use glam::{Mat4, Quat, Vec3};
use ts_core::{Bounds, Mesh, MeshPart, PartRole, Scene};
use ts_fit::{
    align_to_body, fit_to_body, grow_to_clear, push_out, transfer_skin, FitSettings, SkinReport,
    SkinSettings, TriBvh,
};

/// Where the user has placed a garment by hand (before fitting), on top of
/// how it was added. Applied around the garment's own centre.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Placement {
    pub translation: Vec3,
    pub rotation: Quat,
    /// Stretch along the garment's own X / Y / Z.
    pub scale: Vec3,
    /// Overall size (multiplies `scale`).
    pub size: f32,
}

impl Default for Placement {
    fn default() -> Self {
        Self {
            translation: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
            size: 1.0,
        }
    }
}

impl Placement {
    pub fn is_identity(&self) -> bool {
        *self == Self::default()
    }

    /// Matrix moving the garment as added to where it is placed.
    pub fn matrix(&self, pivot: Vec3) -> Mat4 {
        Mat4::from_translation(pivot + self.translation)
            * Mat4::from_quat(self.rotation)
            * Mat4::from_scale(self.scale * self.size)
            * Mat4::from_translation(-pivot)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum GarmentStatus {
    /// Just added (lined up with the body if a reference body was found).
    Added,
    Fitted,
    Skinned,
}

impl GarmentStatus {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Added => "Not fitted yet",
            Self::Fitted => "Fitted (not skinned)",
            Self::Skinned => "Fitted and skinned",
        }
    }
}

pub struct GarmentWork {
    /// Index of the garment in `scene.parts`.
    pub part: usize,
    /// The garment as it was added (after lining up), so "Fit" can start over.
    pub original: Mesh,
    /// Centre of `original`: the point it is moved, turned and scaled around.
    pub pivot: Vec3,
    /// Hand placement on top of `original` (the gizmo and the Place sliders).
    pub placement: Placement,
    /// Index of the reference body in `scene.parts`, if the file had one.
    pub reference: Option<usize>,
    pub use_reference: bool,
    pub status: GarmentStatus,
    pub report: Option<SkinReport>,
    /// Include in the next saved / sent outfit.
    pub include: bool,
    /// Layer, 1 (against the skin) to 4 (outermost). A garment is fitted over
    /// the garments on lower layers, so a jacket goes over the shirt.
    pub layer: u8,
}

/// Layer names for the UI.
pub const LAYER_NAMES: [&str; 4] = ["1 · Next to skin", "2 · Over that", "3 · Outer", "4 · Outermost"];

/// The usual layer for a library category / preset type.
pub fn default_layer(category: &str) -> u8 {
    match category.to_ascii_lowercase().as_str() {
        "jackets" | "vests" | "backpacks" => 2,
        "belts" => 3,
        _ => 1,
    }
}

/// The body to fit to: the most detailed skinned body part (LOD0).
pub fn body_part(scene: &Scene) -> Option<usize> {
    scene
        .parts
        .iter()
        .enumerate()
        .filter(|(_, p)| p.role == PartRole::Body && p.mesh.skin.is_some())
        .max_by_key(|(_, p)| p.mesh.triangle_count())
        .map(|(i, _)| i)
}

/// Guess which part of a garment file is the body it was made on.
pub fn guess_reference(parts: &[MeshPart]) -> Option<usize> {
    if parts.len() < 2 {
        return None;
    }
    let by_name = parts.iter().position(|p| {
        let n = p.name.to_lowercase();
        ["avatar", "body", "mannequin", "reference", "base_mesh"]
            .iter()
            .any(|k| n.contains(k))
    });
    if by_name.is_some() {
        return by_name;
    }
    // Otherwise: a part much bigger than everything else is probably a body.
    let mut sizes: Vec<(usize, usize)> = parts
        .iter()
        .enumerate()
        .map(|(i, p)| (p.mesh.triangle_count(), i))
        .collect();
    sizes.sort_unstable_by(|a, b| b.cmp(a));
    (sizes[0].0 >= sizes[1].0 * 3).then_some(sizes[0].1)
}

/// Add every mesh in `file` to `scene` as garments (plus its reference body,
/// hidden). Returns the new garments. Fails if there is no body to fit to.
pub fn add_garments(scene: &mut Scene, file: Scene) -> Result<Vec<GarmentWork>, String> {
    let body =
        body_part(scene).ok_or("Open a skinned body (like a MetaHuman) before adding clothes.")?;
    let reference = guess_reference(&file.parts);

    // Line the file up with our body using its reference body.
    let align = reference.map(|r| {
        align_to_body(
            &file.parts[r].mesh.positions,
            &scene.parts[body].mesh.positions,
        )
    });

    let mut reference_index = None;
    let mut new_garments = Vec::new();
    for (i, mut part) in file.parts.into_iter().enumerate() {
        if let Some(m) = align {
            part.mesh
                .positions
                .iter_mut()
                .for_each(|p| *p = m.transform_point3(*p));
            part.mesh.compute_normals();
        }
        // Old skin weights belong to some other skeleton: drop them.
        part.mesh.skin = None;
        if Some(i) == reference {
            part.role = PartRole::Reference;
            part.visible = false;
            part.name = format!("{} (reference body)", part.name);
            reference_index = Some(scene.parts.len());
            scene.parts.push(part);
        } else {
            part.role = PartRole::Garment;
            part.visible = true;
            let pivot = Bounds::from_points(part.mesh.positions.iter().copied())
                .map(|b| b.center())
                .unwrap_or_default();
            new_garments.push(GarmentWork {
                part: scene.parts.len(),
                original: part.mesh.clone(),
                pivot,
                placement: Placement::default(),
                reference: None,
                use_reference: true,
                status: GarmentStatus::Added,
                report: None,
                include: true,
                layer: 1,
            });
            scene.parts.push(part);
        }
    }
    for g in &mut new_garments {
        g.reference = reference_index;
    }
    if new_garments.is_empty() {
        return Err("That file has no clothing mesh in it (only a body).".into());
    }
    Ok(new_garments)
}

impl GarmentWork {
    /// Centre of the garment where it is placed now (where the gizmo sits).
    pub fn gizmo_center(&self) -> Vec3 {
        self.pivot + self.placement.translation
    }

    /// The garment as added, moved to its hand placement.
    pub fn placed_mesh(&self) -> Mesh {
        let mut mesh = self.original.clone();
        if !self.placement.is_identity() {
            let m = self.placement.matrix(self.pivot);
            mesh.positions
                .iter_mut()
                .for_each(|p| *p = m.transform_point3(*p));
            mesh.compute_normals();
        }
        mesh.skin = None;
        mesh
    }
}

/// Show the garment at its hand placement. This undoes any fitting, sculpting
/// and skinning, since those were done for the old placement.
pub fn apply_placement(scene: &mut Scene, g: &mut GarmentWork) {
    scene.parts[g.part].mesh = g.placed_mesh();
    g.status = GarmentStatus::Added;
    g.report = None;
}

/// Make the garment just big enough that it no longer sinks into the body
/// (keeps its placement, only grows its overall size). Returns the new size.
pub fn grow_to_clear_body(scene: &mut Scene, g: &mut GarmentWork, inner: &[usize]) -> Result<f32, String> {
    let body = collision_bvh(scene, inner).ok_or("No skinned body to measure against.")?;
    let placed = g.placed_mesh();
    let factor = grow_to_clear(&placed.positions, g.gizmo_center(), &body, 0.03, 3.0)
        .ok_or("Even three times bigger it still sinks into the body. Check that it is lined up with the body first.")?;
    g.placement.size *= factor;
    apply_placement(scene, g);
    Ok(g.placement.size)
}

/// Push any part of the garment that has been sculpted into the skin back out.
pub fn keep_outside(scene: &mut Scene, g: &GarmentWork, body: &TriBvh, gap: f32) {
    let mesh = &mut scene.parts[g.part].mesh;
    push_out(&mut mesh.positions, &mesh.indices, body, gap, 3);
    mesh.compute_normals();
}

/// The body plus the garments in `inner` (lower layers), as one surface to
/// fit against.
pub fn collision_bvh(scene: &Scene, inner: &[usize]) -> Option<TriBvh> {
    let b = body_part(scene)?;
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut indices = Vec::new();
    for &i in std::iter::once(&b).chain(inner) {
        let Some(part) = scene.parts.get(i) else { continue };
        let m = &part.mesh;
        let base = positions.len() as u32;
        positions.extend_from_slice(&m.positions);
        if m.normals.len() == m.positions.len() {
            normals.extend_from_slice(&m.normals);
        } else {
            normals.extend(std::iter::repeat_n(Vec3::Y, m.positions.len()));
        }
        indices.extend(m.indices.iter().map(|x| x + base));
    }
    Some(TriBvh::new(&positions, &normals, &indices))
}

/// Which body triangles are covered by the given garment parts (rest pose).
/// None when there's no body or nothing to cover it with.
pub fn covered_body(scene: &Scene, parts: &[usize]) -> Option<(usize, Vec<bool>)> {
    let b = body_part(scene)?;
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut indices = Vec::new();
    for &i in parts {
        let Some(part) = scene.parts.get(i) else { continue };
        let m = &part.mesh;
        let base = positions.len() as u32;
        positions.extend_from_slice(&m.positions);
        if m.normals.len() == m.positions.len() {
            normals.extend_from_slice(&m.normals);
        } else {
            normals.extend(std::iter::repeat_n(Vec3::Y, m.positions.len()));
        }
        indices.extend(m.indices.iter().map(|x| x + base));
    }
    if indices.is_empty() {
        return None;
    }
    let clothes = TriBvh::new(&positions, &normals, &indices);
    let hidden = ts_fit::covered_triangles(&scene.parts[b].mesh, &clothes, &ts_fit::CoverSettings::default());
    Some((b, hidden))
}

/// Scene parts of the visible garments on lower layers than garment `gi`.
pub fn inner_parts(scene: &Scene, garments: &[GarmentWork], gi: usize) -> Vec<usize> {
    let Some(g) = garments.get(gi) else { return Vec::new() };
    garments
        .iter()
        .filter(|o| o.layer < g.layer && o.part != g.part)
        .filter(|o| scene.parts.get(o.part).is_some_and(|p| p.visible))
        .map(|o| o.part)
        .collect()
}

pub fn body_bvh(scene: &Scene) -> Option<(usize, TriBvh)> {
    let b = body_part(scene)?;
    let m = &scene.parts[b].mesh;
    Some((b, TriBvh::new(&m.positions, &m.normals, &m.indices)))
}

/// Fit the garment onto the body, starting again from how it was added.
pub fn fit(scene: &mut Scene, g: &mut GarmentWork, inner: &[usize], settings: &FitSettings) -> Result<(), String> {
    let body = collision_bvh(scene, inner).ok_or("No skinned body to fit to.")?;
    let reference = match (g.reference, g.use_reference) {
        (Some(r), true) => {
            let m = &scene.parts[r].mesh;
            Some(TriBvh::new(&m.positions, &m.normals, &m.indices))
        }
        _ => None,
    };
    let mut mesh = g.placed_mesh();
    fit_to_body(
        &mut mesh.positions,
        &mesh.indices,
        reference.as_ref(),
        &body,
        settings,
    );
    mesh.compute_normals();
    mesh.skin = None;
    scene.parts[g.part].mesh = mesh;
    g.status = GarmentStatus::Fitted;
    g.report = None;
    Ok(())
}

/// Copy skin weights from the body onto the garment (as it is now).
pub fn skin(scene: &mut Scene, g: &mut GarmentWork, settings: &SkinSettings) -> Result<(), String> {
    let (b, body) = body_bvh(scene).ok_or("No skinned body to copy weights from.")?;
    let (skin, report) = transfer_skin(
        &scene.parts[g.part].mesh,
        &scene.parts[b].mesh,
        &body,
        settings,
    );
    scene.parts[g.part].mesh.skin = Some(skin);
    g.status = GarmentStatus::Skinned;
    g.report = Some(report);
    Ok(())
}

/// Put the garment back to how it was when added (placement too).
pub fn reset(scene: &mut Scene, g: &mut GarmentWork) {
    g.placement = Placement::default();
    scene.parts[g.part].mesh = g.original.clone();
    g.status = GarmentStatus::Added;
    g.report = None;
}

// ---- AI garments --------------------------------------------------------------

/// Bind-pose position of a bone, by name.
fn bone_pos(scene: &Scene, name: &str) -> Option<Vec3> {
    let i = scene.skeleton.find(name)?;
    Some(scene.skeleton.bind_world_matrices()[i].w_axis.truncate())
}

/// Which way the body faces (flat, unit length): heel → toe of both feet.
fn body_forward(scene: &Scene) -> Vec3 {
    let mut f = Vec3::ZERO;
    for side in ["l", "r"] {
        if let (Some(a), Some(b)) = (
            bone_pos(scene, &format!("foot_{side}")),
            bone_pos(scene, &format!("ball_{side}")),
        ) {
            f += b - a;
        }
    }
    f.y = 0.0;
    if f.length() < 1e-4 {
        Vec3::Z
    } else {
        f.normalize()
    }
}

/// Where a named anchor is: a bone, "floor" (between the feet, on the ground)
/// or "hands" (between the hands).
fn anchor_pos(scene: &Scene, name: &str, body_min_y: f32) -> Option<Vec3> {
    match name {
        "floor" => {
            let mid = match (bone_pos(scene, "foot_l"), bone_pos(scene, "foot_r")) {
                (Some(a), Some(b)) => (a + b) * 0.5,
                _ => Vec3::ZERO,
            };
            Some(Vec3::new(mid.x, body_min_y, mid.z))
        }
        "hands" => Some((bone_pos(scene, "hand_l")? + bone_pos(scene, "hand_r")?) * 0.5),
        _ => bone_pos(scene, name),
    }
}

/// Put a freshly generated garment roughly where it belongs on the body:
/// turn it to face the same way, size it from the preset's width rule and
/// line its anchor edge up with the anchor bone. Pairs (shoes, gloves) are
/// split and each half goes to its own side. Returns a note for the user.
pub fn auto_place(scene: &mut Scene, g: &mut GarmentWork, place: &ts_gen::Place) -> String {
    let Some(body) = body_part(scene) else {
        return "No body to place it on.".into();
    };
    let body_min_y = scene.parts[body]
        .mesh
        .positions
        .iter()
        .map(|p| p.y)
        .fold(f32::INFINITY, f32::min);
    let forward = body_forward(scene);
    let side = forward.cross(Vec3::Y).normalize_or_zero();
    let mut mesh = g.original.clone();
    let Some(b) = Bounds::from_points(mesh.positions.iter().copied()) else {
        return "The new garment is empty.".into();
    };

    // 1. Face the body's way (generated models face +Z).
    let yaw = forward.x.atan2(forward.z) + place.turn_deg.to_radians();
    let turn = Mat4::from_translation(b.center())
        * Mat4::from_quat(Quat::from_rotation_y(yaw))
        * Mat4::from_translation(-b.center());
    mesh.positions.iter_mut().for_each(|p| *p = turn.transform_point3(*p));

    // 2. Size: measure the garment's width in a band near the anchor edge.
    let b = Bounds::from_points(mesh.positions.iter().copied()).unwrap();
    let h = (b.max.y - b.min.y).max(1e-4);
    let band_y = match place.anchor.as_str() {
        "top" => b.max.y - place.width_at * h,
        "bottom" => b.min.y + place.width_at * h,
        _ => b.min.y + place.width_at.clamp(0.0, 1.0) * h,
    };
    let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
    for p in mesh.positions.iter().filter(|p| (p.y - band_y).abs() < h * 0.05) {
        let s = p.dot(side);
        lo = lo.min(s);
        hi = hi.max(s);
    }
    if !lo.is_finite() || hi - lo < 1e-4 {
        lo = mesh.positions.iter().map(|p| p.dot(side)).fold(f32::INFINITY, f32::min);
        hi = mesh.positions.iter().map(|p| p.dot(side)).fold(f32::NEG_INFINITY, f32::max);
    }
    let measured = (hi - lo).max(1e-4);
    let target = place.width_cm.map(|cm| cm / 100.0).or_else(|| {
        match place.width_bones.as_slice() {
            [a, c] => {
                let (a, c) = (bone_pos(scene, a)?, bone_pos(scene, c)?);
                let d = (a - c).dot(side).abs().max(Vec3::new(a.x - c.x, 0.0, a.z - c.z).length());
                Some(d * place.width_scale)
            }
            _ => None,
        }
    });
    let scale = target.map(|t| (t / measured).clamp(0.02, 50.0)).unwrap_or(1.0);
    let c = b.center();
    mesh.positions.iter_mut().for_each(|p| *p = c + (*p - c) * scale);

    // 3. Move: anchor edge to the anchor point (+ offsets).
    let Some(anchor) = anchor_pos(scene, &place.anchor_bone, body_min_y) else {
        return format!(
            "This skeleton has no \"{}\" bone, so the garment was only resized. Place it by hand.",
            place.anchor_bone
        );
    };
    let anchor = anchor + Vec3::Y * (place.offset_cm / 100.0) - forward * (place.behind_cm / 100.0);
    let move_group = |positions: &mut [Vec3], which: &[usize], target: Vec3| {
        let bb = Bounds::from_points(which.iter().map(|&i| positions[i])).unwrap();
        let y = match place.anchor.as_str() {
            "top" => bb.max.y,
            "bottom" => bb.min.y,
            _ => bb.center().y,
        };
        let shift = Vec3::new(target.x - bb.center().x, target.y - y, target.z - bb.center().z);
        for &i in which {
            positions[i] += shift;
        }
    };
    let all: Vec<usize> = (0..mesh.positions.len()).collect();
    let mut note = String::new();
    match place.pair.as_slice() {
        [left, right] => {
            // Split into the two halves of the pair and send each to its side.
            let centre = Bounds::from_points(mesh.positions.iter().copied()).unwrap().center();
            let (mut a, mut bb): (Vec<usize>, Vec<usize>) = (Vec::new(), Vec::new());
            for (i, p) in mesh.positions.iter().enumerate() {
                if (*p - centre).dot(side) >= 0.0 {
                    a.push(i)
                } else {
                    bb.push(i)
                }
            }
            match (bone_pos(scene, left), bone_pos(scene, right)) {
                (Some(l), Some(r)) if !a.is_empty() && !bb.is_empty() => {
                    let (pos_side, neg_side) = if (l - r).dot(side) >= 0.0 { (l, r) } else { (r, l) };
                    let floor = place.anchor_bone == "floor";
                    let target_for = |bone: Vec3| {
                        if floor {
                            Vec3::new(bone.x, anchor.y, bone.z)
                        } else {
                            bone + Vec3::Y * (place.offset_cm / 100.0)
                        }
                    };
                    let (ta, tb) = (target_for(pos_side), target_for(neg_side));
                    move_group(&mut mesh.positions, &a, ta);
                    move_group(&mut mesh.positions, &bb, tb);
                    note = " Each half went to its own side; check the angle of each with Rotate.".into();
                }
                _ => move_group(&mut mesh.positions, &all, anchor),
            }
        }
        _ => move_group(&mut mesh.positions, &all, anchor),
    }

    mesh.compute_normals();
    mesh.skin = None;
    g.pivot = Bounds::from_points(mesh.positions.iter().copied())
        .map(|b| b.center())
        .unwrap_or_default();
    g.original = mesh.clone();
    g.placement = Placement::default();
    g.status = GarmentStatus::Added;
    g.report = None;
    scene.parts[g.part].mesh = mesh;
    format!("Placed it on the body (about {:.0}% of its made size).{note}", scale * 100.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec2;
    use ts_core::{Joint, Skeleton, Skin, Trs};

    /// A box "body" 0.4 m wide, 1.8 m tall, facing +X (like a turned character),
    /// with flat bones at the usual MetaHuman names.
    fn body_scene() -> Scene {
        let bones = [
            ("root", Vec3::ZERO),
            ("pelvis", Vec3::new(0.0, 1.0, 0.0)),
            ("neck_01", Vec3::new(0.0, 1.5, 0.0)),
            ("clavicle_l", Vec3::new(0.0, 1.45, 0.1)),
            ("clavicle_r", Vec3::new(0.0, 1.45, -0.1)),
            ("foot_l", Vec3::new(0.0, 0.08, 0.1)),
            ("foot_r", Vec3::new(0.0, 0.08, -0.1)),
            ("ball_l", Vec3::new(0.15, 0.02, 0.1)),
            ("ball_r", Vec3::new(0.15, 0.02, -0.1)),
        ];
        let skeleton = Skeleton {
            joints: bones
                .iter()
                .enumerate()
                .map(|(i, (n, p))| Joint {
                    name: n.to_string(),
                    parent: (i > 0).then_some(0),
                    bind_local: Trs { translation: *p, ..Default::default() },
                })
                .collect(),
        };
        let mut body = Mesh::default();
        for y in [0.0, 1.8] {
            for (x, z) in [(-0.1, -0.2), (0.1, -0.2), (0.1, 0.2), (-0.1, 0.2)] {
                body.positions.push(Vec3::new(x, y, z));
            }
        }
        body.indices = vec![0, 1, 2, 0, 2, 3, 4, 6, 5, 4, 7, 6, 0, 4, 5, 0, 5, 1, 1, 5, 6, 1, 6, 2, 2, 6, 7, 2, 7, 3, 3, 7, 4, 3, 4, 0];
        body.compute_normals();
        body.skin = Some(Skin::default());
        Scene { name: "test".into(), parts: vec![MeshPart::new("body", body)], skeleton }
    }

    /// A flat "shirt" 2 units wide (along X), 2 tall, made facing +Z.
    fn shirt_file() -> Scene {
        let mut m = Mesh::default();
        m.positions = vec![Vec3::new(-1.0, -1.0, 0.0), Vec3::new(1.0, -1.0, 0.0), Vec3::new(1.0, 1.0, 0.0), Vec3::new(-1.0, 1.0, 0.0)];
        m.indices = vec![0, 1, 2, 0, 2, 3];
        m.compute_normals();
        Scene { name: "shirt".into(), parts: vec![MeshPart::new("shirt", m)], skeleton: Skeleton::default() }
    }

    #[test]
    fn auto_place_turns_sizes_and_anchors() {
        let mut scene = body_scene();
        let mut g = add_garments(&mut scene, shirt_file()).unwrap().remove(0);
        let place = ts_gen::Place {
            anchor: "top".into(),
            anchor_bone: "neck_01".into(),
            offset_cm: -3.0,
            behind_cm: 0.0,
            width_bones: vec!["clavicle_l".into(), "clavicle_r".into()],
            width_at: 0.25,
            width_scale: 2.0,
            width_cm: None,
            turn_deg: 0.0,
            pair: vec![],
        };
        auto_place(&mut scene, &mut g, &place);
        let b = Bounds::from_points(scene.parts[g.part].mesh.positions.iter().copied()).unwrap();
        // Body faces +X, so the shirt's width (made along X) now runs along Z.
        assert!((b.max.x - b.min.x).abs() < 1e-3, "not turned: {b:?}");
        // Width = clavicle spacing (0.2) × 2 = 0.4.
        assert!(((b.max.z - b.min.z) - 0.4).abs() < 1e-3, "wrong width: {b:?}");
        // Top edge 3 cm under the neck bone.
        assert!((b.max.y - 1.47).abs() < 1e-3, "wrong height: {b:?}");
        assert!(b.center().z.abs() < 1e-3);
        assert_eq!(g.original.positions, scene.parts[g.part].mesh.positions);
    }

    #[test]
    fn auto_place_splits_pairs_onto_feet() {
        let mut scene = body_scene();
        // Two "shoes" side by side along X (made facing +Z), floating in the air.
        let mut m = Mesh::default();
        for cx in [-0.6f32, 0.6] {
            let base = m.positions.len() as u32;
            m.positions.extend([Vec3::new(cx - 0.4, 2.0, -0.5), Vec3::new(cx + 0.4, 2.0, -0.5), Vec3::new(cx + 0.4, 2.5, 0.5), Vec3::new(cx - 0.4, 2.5, 0.5)]);
            m.indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
        }
        m.compute_normals();
        let file = Scene { name: "shoes".into(), parts: vec![MeshPart::new("shoes", m)], skeleton: Skeleton::default() };
        let mut g = add_garments(&mut scene, file).unwrap().remove(0);
        let place = ts_gen::Place {
            anchor: "bottom".into(),
            anchor_bone: "floor".into(),
            offset_cm: 0.0,
            behind_cm: 0.0,
            width_bones: vec!["foot_l".into(), "foot_r".into()],
            width_at: 0.2,
            width_scale: 1.6,
            width_cm: None,
            turn_deg: 0.0,
            pair: vec!["foot_l".into(), "foot_r".into()],
        };
        auto_place(&mut scene, &mut g, &place);
        let p = &scene.parts[g.part].mesh.positions;
        let left = Bounds::from_points(p[..4].iter().copied()).unwrap();
        let right = Bounds::from_points(p[4..].iter().copied()).unwrap();
        // On the floor, one under each foot (feet at z = ±0.1).
        assert!(left.min.y.abs() < 1e-3 && right.min.y.abs() < 1e-3, "{left:?} {right:?}");
        let mut zs = [left.center().z, right.center().z];
        zs.sort_by(f32::total_cmp);
        assert!((zs[0] + 0.1).abs() < 1e-3 && (zs[1] - 0.1).abs() < 1e-3, "{zs:?}");
    }

    /// A closed cylinder around the Y axis (as a garment file).
    fn cylinder(radius: f32, name: &str) -> Scene {
        let mut m = Mesh::default();
        let (rings, segs) = (12, 32);
        for r in 0..=rings {
            let y = 0.6 + 0.8 * r as f32 / rings as f32;
            for k in 0..segs {
                let a = k as f32 / segs as f32 * std::f32::consts::TAU;
                m.positions.push(Vec3::new(a.cos() * radius, y, a.sin() * radius));
            }
        }
        for r in 0..rings {
            for k in 0..segs {
                let a = (r * segs + k) as u32;
                let b = (r * segs + (k + 1) % segs) as u32;
                let (c, d) = (a + segs as u32, b + segs as u32);
                m.indices.extend([a, c, b, b, c, d]);
            }
        }
        m.compute_normals();
        Scene { name: name.into(), parts: vec![MeshPart::new(name, m)], skeleton: Skeleton::default() }
    }

    #[test]
    fn outer_layer_fits_over_inner_layer() {
        let mut scene = body_scene();
        let mut inner = add_garments(&mut scene, cylinder(0.24, "shirt")).unwrap().remove(0);
        inner.layer = 1;
        let mut outer = add_garments(&mut scene, cylinder(0.33, "jacket")).unwrap().remove(0);
        outer.layer = 2;
        let garments = vec![inner, outer];
        let settings = FitSettings { tightness: 1.0, max_offset: 0.5, ..Default::default() };
        let radius_min = |scene: &Scene, part: usize| {
            scene.parts[part].mesh.positions.iter().map(|p| Vec2::new(p.x, p.z).length()).fold(f32::INFINITY, f32::min)
        };
        // Without layers the jacket shrinks right onto the body.
        let mut alone = scene.clone();
        let mut g = garments[1].clone_for_test();
        fit(&mut alone, &mut g, &[], &settings).unwrap();
        assert!(radius_min(&alone, g.part) < 0.2, "{}", radius_min(&alone, g.part));
        // With the shirt below, it stays outside the shirt.
        let inner_parts = inner_parts(&scene, &garments, 1);
        assert_eq!(inner_parts, vec![garments[0].part]);
        let mut g = garments[1].clone_for_test();
        fit(&mut scene, &mut g, &inner_parts, &settings).unwrap();
        assert!(radius_min(&scene, g.part) > 0.235, "{}", radius_min(&scene, g.part));
    }

    impl GarmentWork {
        fn clone_for_test(&self) -> GarmentWork {
            GarmentWork {
                part: self.part,
                original: self.original.clone(),
                pivot: self.pivot,
                placement: self.placement,
                reference: self.reference,
                use_reference: self.use_reference,
                status: self.status,
                report: None,
                include: self.include,
                layer: self.layer,
            }
        }
    }
}
