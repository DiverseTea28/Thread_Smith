//! Dump posed vertex positions of each part as CSV (for debugging plots).
fn main() -> anyhow::Result<()> {
    let path = std::env::args().nth(1).unwrap();
    let out = std::env::args().nth(2).unwrap();
    let scene = ts_io::load(std::path::Path::new(&path))?;
    let pose = ts_core::Pose::bind(&scene.skeleton);
    let jw = pose.world_matrices(&scene.skeleton);
    let mut s = String::from("part,x,y,z\n");
    for p in &scene.parts {
        let (pos, _) = p.mesh.skinned(&jw);
        for v in pos.iter().step_by(3) {
            s += &format!("{},{},{},{}\n", p.name, v.x, v.y, v.z);
        }
    }
    let mut j = String::from("name,x,y,z\n");
    for (i, m) in jw.iter().enumerate() {
        let t = m.w_axis;
        j += &format!(
            "{},{},{},{}\n",
            scene.skeleton.joints[i].name, t.x, t.y, t.z
        );
    }
    std::fs::write(&out, s)?;
    std::fs::write(format!("{out}.joints.csv"), j)?;
    Ok(())
}
