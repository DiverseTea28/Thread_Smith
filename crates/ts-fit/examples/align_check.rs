//! Check body alignment with and without a head.
//! Run: cargo run --release -p ts-fit --example align_check -- body.fbx garment.fbx
use glam::Vec3;
use ts_fit::align_to_body;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let body = ts_io::load(args[1].as_ref())?;
    let garment = ts_io::load(args[2].as_ref())?;
    let b = &body
        .parts
        .iter()
        .filter(|p| p.mesh.skin.is_some())
        .max_by_key(|p| p.mesh.triangle_count())
        .unwrap()
        .mesh;
    let r = &garment
        .parts
        .iter()
        .max_by_key(|p| p.mesh.triangle_count())
        .unwrap()
        .mesh;
    let top = b.positions.iter().map(|p| p.y).fold(f32::MIN, f32::max);
    let floor = b.positions.iter().map(|p| p.y).fold(f32::MAX, f32::min);
    println!("body height {:.3}", top - floor);
    let cut = floor + (top - floor) * 0.86;
    let headless: Vec<Vec3> = b.positions.iter().copied().filter(|p| p.y < cut).collect();
    for (name, pts) in [("full", &b.positions), ("headless", &headless)] {
        let m = align_to_body(&r.positions, pts);
        println!("{name}: {:?}", m.to_scale_rotation_translation());
    }
    Ok(())
}
