//! A texture written into a GLB comes back when the GLB is opened again.

use glam::{Mat4, Vec2, Vec3};
use ts_core::{Joint, Material, Mesh, Skeleton, Skin, Texture, Trs, MAX_INFLUENCES};

fn tiny_png() -> Vec<u8> {
    let img = image::RgbaImage::from_pixel(4, 4, image::Rgba([10, 200, 30, 255]));
    let mut bytes = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png).unwrap();
    bytes
}

#[test]
fn texture_round_trip() {
    let skeleton = Skeleton {
        joints: vec![Joint { name: "root".into(), parent: None, bind_local: Trs::default() }],
    };
    let mut w = [0.0; MAX_INFLUENCES];
    w[0] = 1.0;
    let mut mesh = Mesh {
        positions: vec![Vec3::ZERO, Vec3::X, Vec3::Y],
        uvs: vec![Vec2::ZERO, Vec2::X, Vec2::Y],
        indices: vec![0, 1, 2],
        skin: Some(Skin { joints: vec![[0; MAX_INFLUENCES]; 3], weights: vec![w; 3], inverse_bind: vec![Mat4::IDENTITY] }),
        ..Default::default()
    };
    mesh.compute_normals();
    let material = Material {
        base_color_texture: Texture::from_bytes("camo.png", tiny_png()),
        ..Material::with_color("Camo", [1.0, 1.0, 1.0])
    };
    let path = std::env::temp_dir().join("threadsmith_texture_test.glb");
    ts_io::write_glb(&path, &skeleton, &[ts_io::ExportMesh { name: "Shirt", mesh: &mesh, color: [1.0; 3], material: Some(&material) }]).unwrap();
    let scene = ts_io::load(&path).unwrap();
    let m = scene.parts[0].material.as_ref().expect("material");
    let t = m.base_color_texture.as_ref().expect("texture");
    assert_eq!(t.mime, "image/png");
    assert_eq!(t.bytes, material.base_color_texture.as_ref().unwrap().bytes);
    let _ = std::fs::remove_file(path);
}

#[test]
fn colour_change_is_baked_on_export() {
    let material = Material {
        base_color_texture: Texture::from_bytes("green.png", tiny_png()),
        hue: 120.0, // green -> blue
        ..Material::with_color("Green", [1.0, 1.0, 1.0])
    };
    let baked = ts_io::baked_material(&material);
    assert_eq!(baked.hue, 0.0);
    let img = image::load_from_memory(&baked.base_color_texture.unwrap().bytes).unwrap().to_rgba8();
    let p = img.get_pixel(0, 0);
    assert!(p[2] > 150 && p[1] < 60, "expected blue, got {p:?}");

    // No texture: the plain colour changes instead.
    let plain = Material { brightness: 0.5, ..Material::with_color("Grey", [1.0, 1.0, 1.0]) };
    let baked = ts_io::baked_material(&plain);
    assert!(baked.base_color[0] < 0.3 && baked.base_color[0] > 0.15, "{:?}", baked.base_color);
}
