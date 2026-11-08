//! Fit + skin a garment, export it as GLB, load it back and check it matches.
//! Run: cargo run --release -p ts-io --example export_demo -- body.fbx garment.fbx out.glb

use glam::{Quat, Vec3};
use ts_core::{Mesh, Pose};
use ts_fit::{align_to_body, fit_to_body, transfer_skin, FitSettings, SkinSettings, TriBvh};

fn main() -> anyhow::Result<()> {
    let a: Vec<String> = std::env::args().collect();
    let body_scene = ts_io::load(a[1].as_ref())?;
    let file = ts_io::load(a[2].as_ref())?;
    let body = &body_scene
        .parts
        .iter()
        .filter(|p| p.mesh.skin.is_some())
        .max_by_key(|p| p.mesh.triangle_count())
        .unwrap()
        .mesh;
    let reference = file
        .parts
        .iter()
        .max_by_key(|p| p.mesh.triangle_count())
        .unwrap();
    let mut g: Mesh = file
        .parts
        .iter()
        .find(|p| p.name != reference.name)
        .unwrap()
        .mesh
        .clone();
    let m = align_to_body(&reference.mesh.positions, &body.positions);
    let rp: Vec<Vec3> = reference
        .mesh
        .positions
        .iter()
        .map(|p| m.transform_point3(*p))
        .collect();
    g.positions
        .iter_mut()
        .for_each(|p| *p = m.transform_point3(*p));
    let body_bvh = TriBvh::new(&body.positions, &body.normals, &body.indices);
    fit_to_body(
        &mut g.positions,
        &g.indices,
        Some(&TriBvh::new(&rp, &[], &reference.mesh.indices)),
        &body_bvh,
        &FitSettings::default(),
    );
    g.compute_normals();
    g.skin = Some(transfer_skin(&g, body, &body_bvh, &SkinSettings::default()).0);

    let out = std::path::Path::new(&a[3]);
    ts_io::write_glb(
        out,
        &body_scene.skeleton,
        &[ts_io::ExportMesh {
            name: "TShirt",
            mesh: &g,
            color: [0.8, 0.45, 0.32],
            material: None,
        }],
    )?;
    println!(
        "wrote {} ({} KB)",
        out.display(),
        std::fs::metadata(out)?.len() / 1024
    );

    // Load it back and compare in a pose.
    let back = ts_io::load(out)?;
    println!(
        "reloaded: {} parts, {} joints (body had {})",
        back.parts.len(),
        back.skeleton.len(),
        body_scene.skeleton.len()
    );
    let pose_for = |sk: &ts_core::Skeleton| {
        let mut p = Pose::bind(sk);
        for (n, r) in [
            ("upperarm_l", Quat::from_rotation_z(1.2)),
            ("spine_03", Quat::from_rotation_y(0.4)),
        ] {
            if let Some(j) = sk.find(n) {
                p.offsets[j] = r;
            }
        }
        p.world_matrices(sk)
    };
    let (a_pos, _) = g.skinned(&pose_for(&body_scene.skeleton));
    let (b_pos, _) = back.parts[0].mesh.skinned(&pose_for(&back.skeleton));
    let err = a_pos
        .iter()
        .zip(&b_pos)
        .map(|(x, y)| (*x - *y).length())
        .fold(0.0f32, f32::max);
    println!(
        "largest difference after export + reload, posed: {:.6} m",
        err
    );
    Ok(())
}
