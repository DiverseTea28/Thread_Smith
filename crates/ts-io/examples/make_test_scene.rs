//! Writes a simple skinned "body" and a textured "shirt" (GLB) for trying the
//! viewport without a MetaHuman: `cargo run -p ts-io --example make_test_scene -- out_dir`

use glam::{Mat4, Vec2, Vec3};
use ts_core::{Joint, Material, Mesh, Skeleton, Skin, Texture, Trs, MAX_INFLUENCES};

/// A tube (open ends) around the Y axis, or a capsule-ish closed shape.
fn tube(radius: impl Fn(f32) -> f32, y0: f32, y1: f32, rings: usize, segs: usize) -> Mesh {
    let mut m = Mesh::default();
    for r in 0..=rings {
        let t = r as f32 / rings as f32;
        let y = y0 + (y1 - y0) * t;
        let rad = radius(t);
        for s in 0..=segs {
            let a = s as f32 / segs as f32 * std::f32::consts::TAU;
            m.positions.push(Vec3::new(a.sin() * rad, y, a.cos() * rad));
            m.uvs.push(Vec2::new(s as f32 / segs as f32 * 2.0, t * 2.0));
        }
    }
    for r in 0..rings {
        for s in 0..segs {
            let a = (r * (segs + 1) + s) as u32;
            let b = a + segs as u32 + 1;
            m.indices.extend([a, a + 1, b, a + 1, b + 1, b]);
        }
    }
    m.compute_normals();
    m
}

fn skin(m: &mut Mesh) {
    let mut w = [0.0; MAX_INFLUENCES];
    w[0] = 1.0;
    m.skin = Some(Skin { joints: vec![[0; MAX_INFLUENCES]; m.positions.len()], weights: vec![w; m.positions.len()], inverse_bind: vec![Mat4::IDENTITY] });
}

fn main() {
    let out = std::path::PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| ".".into()));
    std::fs::create_dir_all(&out).unwrap();
    let skeleton = Skeleton { joints: vec![Joint { name: "root".into(), parent: None, bind_local: Trs::default() }] };
    // Body: a rounded column 1.8 m tall.
    let mut body = tube(|t| 0.06 + 0.12 * (t * std::f32::consts::PI).sin().max(0.0).powf(0.6), 0.0, 1.8, 48, 48);
    skin(&mut body);
    ts_io::write_glb(&out.join("test_body.glb"), &skeleton, &[ts_io::ExportMesh { name: "Body", mesh: &body, color: [0.6, 0.6, 0.65], material: None }]).unwrap();
    // Shirt: a looser tube around the chest with a camo texture.
    let mut shirt = tube(|t| 0.2 + 0.02 * (t * 6.0).sin(), 0.95, 1.5, 20, 48);
    skin(&mut shirt);
    let mut img = image::RgbImage::new(256, 256);
    for (x, y, p) in img.enumerate_pixels_mut() {
        let (fx, fy) = (x as f32 / 256.0, y as f32 / 256.0);
        let n = ((fx * 9.0).sin() * (fy * 7.0).cos() + (fx * 23.0 + fy * 11.0).sin() * 0.5 + (fy * 31.0 - fx * 5.0).cos() * 0.3).clamp(-1.0, 1.0);
        *p = if n > 0.45 { image::Rgb([40, 52, 30]) } else if n > -0.1 { image::Rgb([86, 104, 58]) } else if n > -0.6 { image::Rgb([128, 116, 78]) } else { image::Rgb([60, 70, 44]) };
    }
    let mut png = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    let material = Material { base_color_texture: Texture::from_bytes("camo.png", png), ..Material::with_color("Camo", [1.0; 3]) };
    ts_io::write_glb(&out.join("test_shirt.glb"), &skeleton, &[ts_io::ExportMesh { name: "Camo shirt", mesh: &shirt, color: [1.0; 3], material: Some(&material) }]).unwrap();
    println!("Wrote {}", out.display());
}
