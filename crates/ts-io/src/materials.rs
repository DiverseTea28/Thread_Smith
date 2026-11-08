//! Reading materials and textures from files.

use std::path::Path;

use ts_core::{Material, Texture};

/// A texture file next to (or named by) a model file.
pub(crate) fn texture_from_file(model: &Path, candidates: &[&str]) -> Option<Texture> {
    let dir = model.parent().unwrap_or(Path::new("."));
    for c in candidates.iter().filter(|c| !c.is_empty()) {
        let p = Path::new(c);
        let mut tries = vec![p.to_path_buf(), dir.join(p)];
        if let Some(name) = p.file_name() {
            tries.push(dir.join(name));
            tries.push(dir.join("textures").join(name));
        }
        for t in tries {
            if let Ok(bytes) = std::fs::read(&t) {
                if let Some(tex) = Texture::from_bytes(file_name(&t), bytes) {
                    return Some(tex);
                }
            }
        }
    }
    None
}

fn file_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "texture".into())
}

/// Encode decoded pixels as PNG (when the original file bytes aren't available).
pub(crate) fn texture_from_pixels(name: &str, width: u32, height: u32, rgba: Vec<u8>) -> Option<Texture> {
    let img = image::RgbaImage::from_raw(width, height, rgba)?;
    let mut bytes = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png).ok()?;
    Texture::from_bytes(format!("{name}.png"), bytes)
}

/// Load an image file as a texture (PNG/JPEG kept as they are, anything
/// else the image crate can read is turned into PNG).
pub fn load_texture(path: &Path) -> anyhow::Result<Texture> {
    let bytes = std::fs::read(path)?;
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("texture").to_string();
    if let Some(t) = portable(Texture::from_bytes(name.clone(), bytes.clone())) {
        return Ok(t);
    }
    let img = image::load_from_memory(&bytes)
        .map_err(|e| anyhow::anyhow!("{} isn't an image ThreadSmith can read: {e}", name))?
        .to_rgba8();
    let (w, h) = img.dimensions();
    let stem = name.rsplit_once('.').map_or(name.as_str(), |(s, _)| s);
    texture_from_pixels(stem, w, h, img.into_raw()).ok_or_else(|| anyhow::anyhow!("Couldn't convert {name}"))
}

/// The material with its hue / saturation / brightness change baked in
/// (into the texture, or the plain colour when there's no texture).
pub fn baked(m: &Material) -> Material {
    if !m.has_color_change() {
        return m.clone();
    }
    let mut out = m.clone();
    out.hue = 0.0;
    out.saturation = 1.0;
    out.brightness = 1.0;
    match &m.base_color_texture {
        Some(t) => match image::load_from_memory(&t.bytes) {
            Ok(img) => {
                let mut img = img.to_rgba8();
                for p in img.pixels_mut() {
                    let c = m.adjust_srgb([p[0] as f32 / 255.0, p[1] as f32 / 255.0, p[2] as f32 / 255.0]);
                    for k in 0..3 {
                        p[k] = (c[k] * 255.0).round().clamp(0.0, 255.0) as u8;
                    }
                }
                let (w, h) = img.dimensions();
                let stem = t.name.rsplit_once('.').map_or(t.name.as_str(), |(s, _)| s);
                out.base_color_texture = texture_from_pixels(stem, w, h, img.into_raw()).or_else(|| Some(t.clone()));
            }
            Err(e) => log::warn!("Couldn't recolour texture {}: {e}", t.name),
        },
        None => {
            let to_srgb = |c: f32| if c <= 0.0031308 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 };
            let to_linear = |c: f32| if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) };
            let [r, g, b, a] = m.base_color;
            let c = m.adjust_srgb([to_srgb(r), to_srgb(g), to_srgb(b)]);
            out.base_color = [to_linear(c[0]), to_linear(c[1]), to_linear(c[2]), a];
        }
    }
    out
}

// ---- glTF -----------------------------------------------------------------

/// WebP isn't part of plain glTF, so turn it into PNG.
pub(crate) fn portable(t: Option<Texture>) -> Option<Texture> {
    let t = t?;
    if t.mime != "image/webp" {
        return Some(t);
    }
    let img = image::load_from_memory(&t.bytes).ok()?.to_rgba8();
    let (w, h) = img.dimensions();
    texture_from_pixels(t.name.trim_end_matches(".webp"), w, h, img.into_raw())
}

pub(crate) fn gltf_material(
    m: gltf::Material,
    buffers: &[gltf::buffer::Data],
    images: &[gltf::image::Data],
    model: &Path,
) -> Material {
    let pbr = m.pbr_metallic_roughness();
    let name = m.name().unwrap_or("Material").to_string();
    let tex = |t: gltf::Texture| gltf_image(t.source(), buffers, images, model);
    Material {
        base_color: pbr.base_color_factor(),
        base_color_texture: portable(pbr.base_color_texture().and_then(|i| tex(i.texture()))),
        normal_texture: portable(m.normal_texture().and_then(|i| tex(i.texture()))),
        // With a metal/rough map the factors are just multipliers (often 1.0),
        // and we don't keep that map, so use plain cloth values instead.
        roughness: if pbr.metallic_roughness_texture().is_some() { 0.8 } else { pbr.roughness_factor() },
        metallic: if pbr.metallic_roughness_texture().is_some() { 0.0 } else { pbr.metallic_factor() },
        name,
        ..Default::default()
    }
}

fn gltf_image(
    img: gltf::Image,
    buffers: &[gltf::buffer::Data],
    images: &[gltf::image::Data],
    model: &Path,
) -> Option<Texture> {
    let name = img.name().map(str::to_owned).unwrap_or_else(|| format!("image_{}", img.index()));
    // Keep the original file bytes when we can.
    match img.source() {
        gltf::image::Source::View { view, .. } => {
            let data = buffers.get(view.buffer().index())?;
            let start = view.offset();
            if let Some(bytes) = data.get(start..start + view.length()) {
                if let Some(t) = Texture::from_bytes(name.clone(), bytes.to_vec()) {
                    return Some(t);
                }
            }
        }
        gltf::image::Source::Uri { uri, .. } if !uri.starts_with("data:") => {
            let decoded = uri.replace("%20", " ");
            if let Some(t) = texture_from_file(model, &[&decoded]) {
                return Some(t);
            }
        }
        _ => {}
    }
    // Otherwise re-encode the decoded pixels.
    let d = images.get(img.index())?;
    let rgba = match d.format {
        gltf::image::Format::R8G8B8A8 => d.pixels.clone(),
        gltf::image::Format::R8G8B8 => d.pixels.chunks_exact(3).flat_map(|c| [c[0], c[1], c[2], 255]).collect(),
        gltf::image::Format::R8 => d.pixels.iter().flat_map(|&v| [v, v, v, 255]).collect(),
        _ => return None,
    };
    texture_from_pixels(&name, d.width, d.height, rgba)
}

// ---- FBX ------------------------------------------------------------------

pub(crate) fn fbx_material(m: &ufbx::Material, model: &Path) -> Material {
    let tex = |t: &Option<ufbx::Ref<ufbx::Texture>>| -> Option<Texture> {
        let t = t.as_ref()?;
        if !t.content.is_empty() {
            let name = Path::new(t.filename.as_ref() as &str)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "texture".into());
            if let Some(tex) = Texture::from_bytes(name, t.content.to_vec()) {
                return Some(tex);
            }
        }
        texture_from_file(
            model,
            &[&t.absolute_filename, &t.relative_filename, &t.filename],
        )
    };
    let base_map = if m.pbr.base_color.texture.is_some() || m.pbr.base_color.has_value {
        &m.pbr.base_color
    } else {
        &m.fbx.diffuse_color
    };
    let c = base_map.value_vec4;
    let mut base = [c.x as f32, c.y as f32, c.z as f32, 1.0];
    if base[..3].iter().all(|v| *v <= 0.0) {
        base = [1.0; 4]; // black usually means "not set"
    }
    let base_color_texture = portable(tex(&m.pbr.base_color.texture).or_else(|| tex(&m.fbx.diffuse_color.texture)));
    Material {
        name: m.element.name.to_string(),
        base_color: if base_color_texture.is_some() { [1.0; 4] } else { base },
        base_color_texture,
        normal_texture: portable(tex(&m.pbr.normal_map.texture).or_else(|| tex(&m.fbx.normal_map.texture))),
        roughness: if m.pbr.roughness.has_value { m.pbr.roughness.value_vec4.x as f32 } else { 0.8 },
        metallic: if m.pbr.metalness.has_value { m.pbr.metalness.value_vec4.x as f32 } else { 0.0 },
        ..Default::default()
    }
}
