//! The outfit library: a plain folder that both ThreadSmith and the Unreal
//! wardrobe read, so the wardrobe works even when ThreadSmith is closed.
//!
//! ```text
//! Documents/ThreadSmith/Library/
//!   Outfits/
//!     red-tshirt-1a2b3c/
//!       outfit.json      what it is (name, type, body, files) - see `OutfitInfo`
//!       outfit.glb       the skinned mesh(es), on the body's skeleton
//!       thumbnail.png    512 x 512 picture for the wardrobe grid
//! ```

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use ts_core::Skeleton;
use ts_io::ExportMesh;

/// Bump when `outfit.json` changes in a way older readers can't handle.
pub const FORMAT_VERSION: u32 = 1;
pub const INFO_FILE: &str = "outfit.json";
pub const MESH_FILE: &str = "outfit.glb";
pub const THUMBNAIL_FILE: &str = "thumbnail.png";
pub const TRIMMED_BODY_FILE: &str = "body_trimmed.glb";

/// Garment types, shown as filters in the app and the Unreal wardrobe.
pub const CATEGORIES: &[&str] = &[
    "Top",
    "Bottom",
    "Full body",
    "Footwear",
    "Headwear",
    "Gloves",
    "Accessory",
    "Outfit",
];

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct BodyInfo {
    /// Character name, e.g. "Ryan".
    pub character: String,
    /// Number of bones in the skeleton the outfit is skinned to.
    pub bones: usize,
    /// Unreal asset path of that skeleton, when the body came from Unreal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unreal_skeleton: Option<String>,
    /// Unreal asset path of the body mesh, when the body came from Unreal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unreal_body: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct GarmentInfo {
    pub name: String,
    pub triangles: usize,
}

/// Contents of `outfit.json`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct OutfitInfo {
    pub format: u32,
    pub id: String,
    pub name: String,
    pub category: String,
    /// Seconds since 1970 (UTC).
    pub created: u64,
    pub modified: u64,
    pub garments: Vec<GarmentInfo>,
    pub body: BodyInfo,
    pub mesh_file: String,
    pub thumbnail_file: String,
    #[serde(default)]
    pub tags: Vec<String>,
    /// The body with the skin under the clothes removed (optional).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trimmed_body_file: Option<String>,
}

/// An outfit found in the library.
#[derive(Clone, Debug)]
pub struct Outfit {
    pub dir: PathBuf,
    pub info: OutfitInfo,
}

impl Outfit {
    pub fn mesh_path(&self) -> PathBuf {
        self.dir.join(&self.info.mesh_file)
    }
    pub fn thumbnail_path(&self) -> PathBuf {
        self.dir.join(&self.info.thumbnail_file)
    }
}

/// A finished picture: width, height, RGBA8 pixels.
pub struct Thumbnail {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

pub struct Library {
    pub root: PathBuf,
}

impl Library {
    /// `Documents/ThreadSmith/Library` (created if missing).
    pub fn default_location() -> PathBuf {
        dirs::document_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("ThreadSmith")
            .join("Library")
    }

    pub fn open(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        std::fs::create_dir_all(root.join("Outfits"))
            .with_context(|| format!("Could not create the library folder {}", root.display()))?;
        // Always a full path: it is sent to Unreal, which runs in another folder.
        let root = std::path::absolute(&root).unwrap_or(root);
        Ok(Self { root })
    }

    pub fn outfits_dir(&self) -> PathBuf {
        self.root.join("Outfits")
    }

    /// Every outfit in the library, newest first. Broken folders are skipped.
    pub fn list(&self) -> Vec<Outfit> {
        let mut out = Vec::new();
        let Ok(entries) = std::fs::read_dir(self.outfits_dir()) else {
            return out;
        };
        for e in entries.flatten() {
            let dir = e.path();
            match read_info(&dir) {
                Ok(info) => out.push(Outfit { dir, info }),
                Err(err) => log::debug!("Skipping {}: {err:#}", dir.display()),
            }
        }
        out.sort_by_key(|o| std::cmp::Reverse(o.info.modified));
        out
    }

    /// Save a new outfit. Returns where it was saved.
    pub fn save(
        &self,
        name: &str,
        category: &str,
        body: BodyInfo,
        skeleton: &Skeleton,
        meshes: &[ExportMesh],
        thumbnail: Option<&Thumbnail>,
    ) -> Result<Outfit> {
        let now = now_secs();
        let id = new_id();
        let dir = self
            .outfits_dir()
            .join(format!("{}-{}", slug(name), &id[..6]));
        std::fs::create_dir_all(&dir)?;
        let info = OutfitInfo {
            format: FORMAT_VERSION,
            id,
            name: name.trim().to_string(),
            category: category.to_string(),
            created: now,
            modified: now,
            garments: meshes
                .iter()
                .map(|m| GarmentInfo {
                    name: m.name.to_string(),
                    triangles: m.mesh.triangle_count(),
                })
                .collect(),
            body,
            mesh_file: MESH_FILE.into(),
            thumbnail_file: THUMBNAIL_FILE.into(),
            tags: Vec::new(),
            trimmed_body_file: None,
        };
        let outfit = Outfit { dir, info };
        ts_io::write_glb(&outfit.mesh_path(), skeleton, meshes)?;
        if let Some(t) = thumbnail {
            image::save_buffer(
                outfit.thumbnail_path(),
                &t.rgba,
                t.width,
                t.height,
                image::ExtendedColorType::Rgba8,
            )
            .context("Could not save the thumbnail")?;
        }
        write_info(&outfit.dir, &outfit.info)?;
        Ok(outfit)
    }

    /// Add the body with the skin under the clothes removed to a saved outfit.
    /// Unreal swaps it in for the character's body so skin can't poke through.
    pub fn save_trimmed_body(&self, outfit: &mut Outfit, skeleton: &Skeleton, body: &ExportMesh) -> Result<()> {
        ts_io::write_glb(&outfit.dir.join(TRIMMED_BODY_FILE), skeleton, std::slice::from_ref(body))?;
        outfit.info.trimmed_body_file = Some(TRIMMED_BODY_FILE.into());
        write_info(&outfit.dir, &outfit.info)
    }

    /// Delete an outfit folder.
    pub fn delete(&self, outfit: &Outfit) -> Result<()> {
        // Only ever delete folders inside our own Outfits folder.
        anyhow::ensure!(
            outfit.dir.starts_with(self.outfits_dir()),
            "Not a library folder"
        );
        std::fs::remove_dir_all(&outfit.dir)?;
        Ok(())
    }
}

pub fn read_info(dir: &Path) -> Result<OutfitInfo> {
    let text = std::fs::read_to_string(dir.join(INFO_FILE))?;
    Ok(serde_json::from_str(&text)?)
}

fn write_info(dir: &Path, info: &OutfitInfo) -> Result<()> {
    // Write to a temp file and rename, so a crash never leaves half a file.
    let tmp = dir.join(format!("{INFO_FILE}.tmp"));
    std::fs::write(&tmp, serde_json::to_string_pretty(info)?)?;
    std::fs::rename(&tmp, dir.join(INFO_FILE))?;
    Ok(())
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 16 hex characters, unique enough for local folders.
fn new_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut x = (nanos as u64) ^ ((std::process::id() as u64) << 32);
    // splitmix64 scramble
    x = x.wrapping_add(0x9E3779B97F4A7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D049BB133111EB);
    format!("{:016x}", x ^ (x >> 31))
}

/// "Red T-Shirt!" -> "red-t-shirt"
pub fn slug(name: &str) -> String {
    let mut s = String::new();
    for c in name.trim().chars() {
        if c.is_ascii_alphanumeric() {
            s.push(c.to_ascii_lowercase());
        } else if !s.ends_with('-') && !s.is_empty() {
            s.push('-');
        }
    }
    let s = s.trim_end_matches('-').to_string();
    if s.is_empty() {
        "outfit".into()
    } else {
        s.chars().take(40).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::{Mat4, Vec3};
    use ts_core::{Joint, Mesh, Skin, Trs, MAX_INFLUENCES};

    #[test]
    fn slugs() {
        assert_eq!(slug("Red T-Shirt!"), "red-t-shirt");
        assert_eq!(slug("  "), "outfit");
    }

    #[test]
    fn save_and_list() {
        let root = std::env::temp_dir().join(format!("ts-lib-test-{}", new_id()));
        let lib = Library::open(&root).unwrap();
        let skeleton = Skeleton {
            joints: vec![Joint {
                name: "root".into(),
                parent: None,
                bind_local: Trs::default(),
            }],
        };
        let mut mesh = Mesh {
            positions: vec![Vec3::ZERO, Vec3::X, Vec3::Y],
            indices: vec![0, 1, 2],
            ..Default::default()
        };
        mesh.compute_normals();
        let mut w = [0.0; MAX_INFLUENCES];
        w[0] = 1.0;
        mesh.skin = Some(Skin {
            joints: vec![[0; MAX_INFLUENCES]; 3],
            weights: vec![w; 3],
            inverse_bind: vec![Mat4::IDENTITY],
        });
        let thumb = Thumbnail {
            width: 2,
            height: 2,
            rgba: vec![255; 16],
        };
        let saved = lib
            .save(
                "Test Shirt",
                "Top",
                BodyInfo {
                    character: "Ryan".into(),
                    bones: 1,
                    ..Default::default()
                },
                &skeleton,
                &[ExportMesh {
                    name: "Shirt",
                    mesh: &mesh,
                    color: [1.0, 0.0, 0.0],
                    material: None,
                }],
                Some(&thumb),
            )
            .unwrap();
        assert!(saved.mesh_path().exists() && saved.thumbnail_path().exists());
        let list = lib.list();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].info, saved.info);
        lib.delete(&list[0]).unwrap();
        assert!(lib.list().is_empty());
        let _ = std::fs::remove_dir_all(root);
    }
}
