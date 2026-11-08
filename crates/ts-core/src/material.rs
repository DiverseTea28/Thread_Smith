//! Materials: colour, texture and surface settings for a mesh part.

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// An image kept in its file form (PNG / JPEG / WebP bytes), so it stays
/// small in projects and can be written into exported files unchanged.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Texture {
    pub name: String,
    /// "image/png", "image/jpeg" or "image/webp".
    pub mime: String,
    #[cfg_attr(feature = "serde", serde(with = "serde_bytes"))]
    pub bytes: Vec<u8>,
}

impl Texture {
    /// Guess the type from the first bytes of the file.
    pub fn from_bytes(name: impl Into<String>, bytes: Vec<u8>) -> Option<Self> {
        let mime = if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
            "image/png"
        } else if bytes.starts_with(&[0xFF, 0xD8]) {
            "image/jpeg"
        } else if bytes.len() > 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
            "image/webp"
        } else {
            return None;
        };
        Some(Self { name: name.into(), mime: mime.into(), bytes })
    }

    pub fn extension(&self) -> &'static str {
        match self.mime.as_str() {
            "image/jpeg" => "jpg",
            "image/webp" => "webp",
            _ => "png",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Material {
    pub name: String,
    /// Linear RGBA, multiplied with the texture.
    pub base_color: [f32; 4],
    pub base_color_texture: Option<Texture>,
    pub normal_texture: Option<Texture>,
    pub roughness: f32,
    pub metallic: f32,
    /// Colour change set in ThreadSmith (baked into the texture on export).
    /// Hue turn in degrees (-180..180).
    #[cfg_attr(feature = "serde", serde(default))]
    pub hue: f32,
    /// 0 = grey, 1 = unchanged, 2 = twice as colourful.
    #[cfg_attr(feature = "serde", serde(default = "one"))]
    pub saturation: f32,
    /// 0 = black, 1 = unchanged, 2 = twice as bright.
    #[cfg_attr(feature = "serde", serde(default = "one"))]
    pub brightness: f32,
}

#[cfg(feature = "serde")]
fn one() -> f32 {
    1.0
}

impl Default for Material {
    fn default() -> Self {
        Self {
            name: "Material".into(),
            base_color: [1.0; 4],
            base_color_texture: None,
            normal_texture: None,
            roughness: 0.8,
            metallic: 0.0,
            hue: 0.0,
            saturation: 1.0,
            brightness: 1.0,
        }
    }
}

impl Material {
    /// True when hue / saturation / brightness change the colours.
    pub fn has_color_change(&self) -> bool {
        self.hue.abs() > 0.01 || (self.saturation - 1.0).abs() > 0.001 || (self.brightness - 1.0).abs() > 0.001
    }

    /// Apply this material's hue / saturation / brightness to an sRGB colour
    /// (0..1). The viewport shader does the same maths.
    pub fn adjust_srgb(&self, rgb: [f32; 3]) -> [f32; 3] {
        adjust_srgb(rgb, self.hue, self.saturation, self.brightness)
    }

    pub fn with_color(name: impl Into<String>, rgb: [f32; 3]) -> Self {
        Self {
            name: name.into(),
            base_color: [rgb[0], rgb[1], rgb[2], 1.0],
            ..Default::default()
        }
    }
}

/// Hue turn (degrees), saturation and brightness multipliers on an sRGB colour.
pub fn adjust_srgb(rgb: [f32; 3], hue: f32, saturation: f32, brightness: f32) -> [f32; 3] {
    let [r, g, b] = rgb;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;
    let mut h = if delta <= 1e-6 {
        0.0
    } else if max == r {
        ((g - b) / delta).rem_euclid(6.0)
    } else if max == g {
        (b - r) / delta + 2.0
    } else {
        (r - g) / delta + 4.0
    } / 6.0;
    let s = if max <= 1e-6 { 0.0 } else { delta / max };
    h = (h + hue / 360.0).rem_euclid(1.0);
    let s = (s * saturation).clamp(0.0, 1.0);
    let v = (max * brightness).clamp(0.0, 1.0);
    hsv_to_rgb(h, s, v)
}

fn hsv_to_rgb(h: f32, s: f32, v: f32) -> [f32; 3] {
    let k = |n: f32| {
        let k = (n + h * 6.0).rem_euclid(6.0);
        v - v * s * k.min(4.0 - k).clamp(0.0, 1.0)
    };
    [k(5.0), k(3.0), k(1.0)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colour_changes() {
        let red = [1.0, 0.0, 0.0];
        let same = adjust_srgb(red, 0.0, 1.0, 1.0);
        assert!(same.iter().zip(red).all(|(a, b)| (a - b).abs() < 1e-5));
        let green = adjust_srgb(red, 120.0, 1.0, 1.0);
        assert!((green[1] - 1.0).abs() < 1e-5 && green[0] < 1e-5 && green[2] < 1e-5);
        let grey = adjust_srgb([0.2, 0.6, 0.4], 0.0, 0.0, 1.0);
        assert!((grey[0] - grey[1]).abs() < 1e-5 && (grey[1] - grey[2]).abs() < 1e-5);
        let dark = adjust_srgb([0.5, 0.5, 0.5], 0.0, 1.0, 0.5);
        assert!((dark[0] - 0.25).abs() < 1e-5);
    }
}
