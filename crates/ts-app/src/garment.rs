//! Garments: adding them to the scene, fitting and skinning them.

use glam::{Mat4, Quat, Vec3};
use ts_core::{Bounds, Mesh, MeshPart, PartRole, Scene};
use ts_fit::{
    align_to_body, fit_to_body, grow_to_clear, push_out, transfer_skin, FitSettings, SkinReport,
    SkinSettings, TriBvh,
};

/// Where the user has placed a garment by hand (before fitting), on top of
/// how it was added. Applied around the garment's own centre.
#[derive(Clone, Copy, Debug, PartialEq)]
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
pub fn grow_to_clear_body(scene: &mut Scene, g: &mut GarmentWork) -> Result<f32, String> {
    let (_, body) = body_bvh(scene).ok_or("No skinned body to measure against.")?;
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

pub fn body_bvh(scene: &Scene) -> Option<(usize, TriBvh)> {
    let b = body_part(scene)?;
    let m = &scene.parts[b].mesh;
    Some((b, TriBvh::new(&m.positions, &m.normals, &m.indices)))
}

/// Fit the garment onto the body, starting again from how it was added.
pub fn fit(scene: &mut Scene, g: &mut GarmentWork, settings: &FitSettings) -> Result<(), String> {
    let (_, body) = body_bvh(scene).ok_or("No skinned body to fit to.")?;
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
