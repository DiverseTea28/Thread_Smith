//! Print what the importer sees in a file.
//! Run: cargo run -p ts-io --example inspect -- path/to/model.fbx

use ts_core::Pose;

fn main() -> anyhow::Result<()> {
    let path = std::env::args().nth(1).expect("give a file path");
    let scene = ts_io::load(std::path::Path::new(&path))?;
    println!(
        "{}: {} parts, {} verts, {} tris, {} joints",
        scene.name,
        scene.parts.len(),
        scene.vertex_count(),
        scene.triangle_count(),
        scene.skeleton.len()
    );
    println!(
        "UE5 / MetaHuman body skeleton: {}",
        scene.skeleton.looks_like_ue5_body()
    );
    let pose = Pose::bind(&scene.skeleton);
    if let Some(b) = scene.bounds(&pose) {
        println!(
            "bounds (posed): min {:?} max {:?} size {:?}",
            b.min,
            b.max,
            b.max - b.min
        );
    }
    let joints = pose.world_matrices(&scene.skeleton);
    for part in &scene.parts {
        let (skinned, _) = part.mesh.skinned(&joints);
        let drift = part
            .mesh
            .positions
            .iter()
            .zip(&skinned)
            .map(|(a, b)| (*a - *b).length())
            .fold(0.0f32, f32::max);
        println!(
            "  part '{}': {} verts, {} tris, skinned: {}, max move from file pose: {:.4} m",
            part.name,
            part.mesh.vertex_count(),
            part.mesh.triangle_count(),
            part.mesh.skin.is_some(),
            drift
        );
    }
    for (i, j) in scene.skeleton.joints.iter().enumerate().take(12) {
        println!("  joint {i}: {} (parent {:?})", j.name, j.parent);
    }
    Ok(())
}
