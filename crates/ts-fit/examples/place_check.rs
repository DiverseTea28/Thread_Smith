//! Check the "too small garment" workflow on real files:
//! shrink the garment, Grow to clear body, then shrink wrap (Fit to body).
//! Run: cargo run --release -p ts-fit --example place_check -- body.fbx garment.glb

use std::time::Instant;

use glam::Vec3;
use ts_core::Bounds;
use ts_fit::{fit_to_body, grow_to_clear, inside_fraction, FitSettings, TriBvh};

fn mean_gap(points: &[Vec3], body: &TriBvh) -> f32 {
    let gaps: Vec<f32> = points
        .iter()
        .filter_map(|p| body.closest(*p).map(|h| (*p - h.point).dot(h.normal)))
        .collect();
    gaps.iter().sum::<f32>() / gaps.len().max(1) as f32
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let body_scene = ts_io::load(args[1].as_ref())?;
    let garment_scene = ts_io::load(args[2].as_ref())?;
    let body = &body_scene
        .parts
        .iter()
        .filter(|p| p.mesh.skin.is_some())
        .max_by_key(|p| p.mesh.triangle_count())
        .unwrap()
        .mesh;
    let bvh = TriBvh::new(&body.positions, &body.normals, &body.indices);
    let garment = garment_scene
        .parts
        .iter()
        .max_by_key(|p| p.mesh.triangle_count())
        .unwrap();
    let mut pts = garment.mesh.positions.clone();
    let idx = &garment.mesh.indices;
    println!("garment: {} verts", pts.len());

    let pivot = Bounds::from_points(pts.iter().copied()).unwrap().center();
    for p in pts.iter_mut() {
        *p = pivot + (*p - pivot) * 0.8;
    }
    println!(
        "shrunk to 80%: {:.0}% inside the body",
        inside_fraction(&pts, &bvh, 0.0, 100_000) * 100.0
    );

    let t = Instant::now();
    let s = grow_to_clear(&pts, pivot, &bvh, 0.03, 3.0).expect("grow");
    for p in pts.iter_mut() {
        *p = pivot + (*p - pivot) * s;
    }
    println!(
        "grow to clear: x{s:.3} ({:.0} ms) -> {:.1}% inside, mean gap {:.1} cm",
        t.elapsed().as_secs_f32() * 1000.0,
        inside_fraction(&pts, &bvh, 0.0, 100_000) * 100.0,
        mean_gap(&pts, &bvh) * 100.0
    );

    let t = Instant::now();
    let settings = FitSettings::default();
    fit_to_body(&mut pts, idx, None, &bvh, &settings);
    println!(
        "shrink wrap (tightness {}): {:.0} ms -> {:.2}% inside, mean gap {:.1} cm",
        settings.tightness,
        t.elapsed().as_secs_f32() * 1000.0,
        inside_fraction(&pts, &bvh, 0.0, 100_000) * 100.0,
        mean_gap(&pts, &bvh) * 100.0
    );
    Ok(())
}
