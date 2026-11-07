//! Fit and skin a garment onto a body from the command line, and report how it went.
//! Run: cargo run --release -p ts-fit --example fit_demo -- body.fbx garment.fbx [out.csv]

use std::time::Instant;

use glam::{Quat, Vec3};
use ts_core::{Mesh, Pose};
use ts_fit::{align_to_body, fit_to_body, transfer_skin, FitSettings, SkinSettings, TriBvh};

fn inside_count(points: &[Vec3], body: &TriBvh) -> usize {
    points
        .iter()
        .filter(|p| {
            body.closest(**p)
                .map(|h| (**p - h.point).dot(h.normal) < 0.0)
                .unwrap_or(false)
        })
        .count()
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let body_scene = ts_io::load(args[1].as_ref())?;
    let garment_scene = ts_io::load(args[2].as_ref())?;

    // Body = biggest skinned part (LOD0).
    let body = &body_scene
        .parts
        .iter()
        .filter(|p| p.mesh.skin.is_some())
        .max_by_key(|p| p.mesh.triangle_count())
        .unwrap()
        .mesh;
    // In the garment file: the biggest part is its reference body, the others are clothes.
    let reference = garment_scene
        .parts
        .iter()
        .max_by_key(|p| p.mesh.triangle_count())
        .unwrap();
    let mut garment: Mesh = garment_scene
        .parts
        .iter()
        .find(|p| p.name != reference.name)
        .unwrap()
        .mesh
        .clone();
    println!(
        "body {} tris, reference '{}' {} tris, garment {} verts",
        body.triangle_count(),
        reference.name,
        reference.mesh.triangle_count(),
        garment.vertex_count()
    );

    let t = Instant::now();
    let body_bvh = TriBvh::new(&body.positions, &body.normals, &body.indices);
    println!("body search tree: {:?}", t.elapsed());

    let before = inside_count(&garment.positions, &body_bvh);
    println!("garment vertices inside the body before anything: {before}");

    // Align the reference body (and garment) to our body.
    let m = align_to_body(&reference.mesh.positions, &body.positions);
    let ref_pos: Vec<Vec3> = reference
        .mesh
        .positions
        .iter()
        .map(|p| m.transform_point3(*p))
        .collect();
    garment
        .positions
        .iter_mut()
        .for_each(|p| *p = m.transform_point3(*p));
    let ref_bvh = TriBvh::new(&ref_pos, &[], &reference.mesh.indices);
    println!(
        "after align: inside = {}",
        inside_count(&garment.positions, &body_bvh)
    );

    let t = Instant::now();
    fit_to_body(
        &mut garment.positions,
        &garment.indices,
        Some(&ref_bvh),
        &body_bvh,
        &FitSettings::default(),
    );
    garment.compute_normals();
    println!(
        "fit: {:?}, inside after fit = {}",
        t.elapsed(),
        inside_count(&garment.positions, &body_bvh)
    );

    let t = Instant::now();
    let (skin, report) = transfer_skin(&garment, body, &body_bvh, &SkinSettings::default());
    println!("skin: {:?}, {report:?}", t.elapsed());
    garment.skin = Some(skin);

    // Pose test: raise the left arm and bend the right elbow; check the shirt follows.
    let sk = &body_scene.skeleton;
    let mut pose = Pose::bind(sk);
    for (name, rot) in [
        ("upperarm_l", Quat::from_rotation_x(-0.9)),
        ("lowerarm_r", Quat::from_rotation_z(0.8)),
        ("spine_03", Quat::from_rotation_y(0.4)),
    ] {
        if let Some(j) = sk.find(name) {
            pose.offsets[j] = rot;
        }
    }
    let jw = pose.world_matrices(sk);
    let (gp, _) = garment.skinned(&jw);
    let (bp, bn) = body.skinned(&jw);
    let posed_bvh = TriBvh::new(&bp, &bn, &body.indices);
    let inside_posed = inside_count(&gp, &posed_bvh);
    println!(
        "posed: garment vertices inside the body = {inside_posed} of {}",
        gp.len()
    );

    if let Some(out) = args.get(3) {
        let mut s = String::from("part,x,y,z\n");
        for v in garment.positions.iter() {
            s += &format!("shirt,{},{},{}\n", v.x, v.y, v.z);
        }
        for v in body.positions.iter().step_by(4) {
            s += &format!("body,{},{},{}\n", v.x, v.y, v.z);
        }
        for v in gp.iter() {
            s += &format!("shirt_posed,{},{},{}\n", v.x, v.y, v.z);
        }
        for v in bp.iter().step_by(4) {
            s += &format!("body_posed,{},{},{}\n", v.x, v.y, v.z);
        }
        std::fs::write(out, s)?;
    }
    Ok(())
}
