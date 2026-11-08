//! ThreadSmith project files (`.tsproj`): the body, head, skeleton, every
//! garment with its fitting / sculpting / skinning, the settings, and where
//! the body came from in Unreal. Open one and carry on where you left off.
//!
//! File layout: `TSPROJ` + version (u32, little endian) + gzip(data).
//! Version 2+ data is CBOR, so new fields can be added later without breaking
//! old files. Version 1 (postcard) files still open.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use glam::Vec3;
use serde::{Deserialize, Serialize};
use ts_core::{Mesh, Scene};
use ts_fit::{FitSettings, SkinSettings};

use crate::bridge::BodyRequest;
use crate::garment::{GarmentStatus, GarmentWork, Placement};

pub const EXTENSION: &str = "tsproj";
const MAGIC: &[u8; 6] = b"TSPROJ";
const VERSION: u32 = 2;

/// One garment's working state.
#[derive(Serialize, Deserialize)]
pub struct SavedGarment {
    pub part: usize,
    pub original: Mesh,
    pub pivot: Vec3,
    pub placement: Placement,
    pub reference: Option<usize>,
    pub use_reference: bool,
    pub status: GarmentStatus,
    pub include: bool,
    #[serde(default = "one")]
    pub layer: u8,
}

fn one() -> u8 {
    1
}

impl SavedGarment {
    pub fn from_work(g: &GarmentWork) -> Self {
        Self {
            part: g.part,
            original: g.original.clone(),
            pivot: g.pivot,
            placement: g.placement,
            reference: g.reference,
            use_reference: g.use_reference,
            status: g.status,
            include: g.include,
            layer: g.layer,
        }
    }

    pub fn into_work(self) -> GarmentWork {
        GarmentWork {
            part: self.part,
            original: self.original,
            pivot: self.pivot,
            placement: self.placement,
            reference: self.reference,
            use_reference: self.use_reference,
            status: self.status,
            report: None,
            include: self.include,
            layer: self.layer.clamp(1, 4),
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(default)]
pub struct Project {
    /// Body, head, garments (as they are now) and the skeleton.
    pub scene: Scene,
    pub garments: Vec<SavedGarment>,
    pub selected_garment: usize,
    pub fit_settings: FitSettings,
    pub skin_settings: SkinSettings,
    /// Where the body came from in Unreal (so Send to Unreal still knows the Blueprint).
    pub unreal_body: Option<BodyRequest>,
    pub outfit_name: String,
    pub outfit_category: String,
    pub apply_to_character: bool,
    /// Remove the body's skin under the clothes when exporting.
    pub hide_body: bool,
}

impl Project {
    pub fn save(&self, path: &Path) -> Result<()> {
        let mut data = Vec::new();
        ciborium::into_writer(self, &mut data).context("Couldn't pack the project")?;
        // Write to a temporary file first, so a crash never leaves half a project.
        let tmp = path.with_extension("tsproj.saving");
        {
            let file = std::fs::File::create(&tmp)
                .with_context(|| format!("Couldn't write {}", tmp.display()))?;
            let mut out = std::io::BufWriter::new(file);
            out.write_all(MAGIC)?;
            out.write_all(&VERSION.to_le_bytes())?;
            let mut gz = flate2::write::GzEncoder::new(out, flate2::Compression::fast());
            gz.write_all(&data)?;
            gz.finish()?.flush()?;
        }
        std::fs::rename(&tmp, path).with_context(|| format!("Couldn't write {}", path.display()))?;
        Ok(())
    }

    pub fn load(path: &Path) -> Result<Self> {
        let file = std::fs::File::open(path).with_context(|| format!("Couldn't open {}", path.display()))?;
        let mut input = std::io::BufReader::new(file);
        let mut header = [0u8; 10];
        input.read_exact(&mut header).context("This isn't a ThreadSmith project")?;
        if &header[..6] != MAGIC {
            bail!("{} isn't a ThreadSmith project.", path.display());
        }
        let version = u32::from_le_bytes(header[6..10].try_into().unwrap());
        if version > VERSION {
            bail!("This project was saved by a newer ThreadSmith (format {version}). Update ThreadSmith to open it.");
        }
        let mut data = Vec::new();
        flate2::read::GzDecoder::new(input)
            .read_to_end(&mut data)
            .context("The project file is damaged")?;
        let project: Project = if version == 1 {
            postcard::from_bytes::<legacy::ProjectV1>(&data)
                .context("The project file is damaged")?
                .into()
        } else {
            ciborium::from_reader(&data[..]).context("The project file is damaged")?
        };
        // Sanity: garment and reference indices must point at real parts.
        let n = project.scene.parts.len();
        if project.garments.iter().any(|g| g.part >= n || g.reference.is_some_and(|r| r >= n)) {
            bail!("The project file is damaged (garment list doesn't match the meshes).");
        }
        Ok(project)
    }
}

impl Default for Project {
    fn default() -> Self {
        Self {
            scene: Scene::default(),
            garments: Vec::new(),
            selected_garment: 0,
            fit_settings: FitSettings::default(),
            skin_settings: SkinSettings::default(),
            unreal_body: None,
            outfit_name: String::new(),
            outfit_category: "Top".into(),
            apply_to_character: true,
            hide_body: true,
        }
    }
}

/// Version 1 files (postcard), from before layers existed.
mod legacy {
    use super::*;

    #[derive(Serialize, Deserialize)]
    pub struct GarmentV1 {
        pub part: usize,
        pub original: Mesh,
        pub pivot: Vec3,
        pub placement: Placement,
        pub reference: Option<usize>,
        pub use_reference: bool,
        pub status: GarmentStatus,
        pub include: bool,
    }

    #[derive(Serialize, Deserialize)]
    pub struct ProjectV1 {
        pub scene: Scene,
        pub garments: Vec<GarmentV1>,
        pub selected_garment: usize,
        pub fit_settings: FitSettings,
        pub skin_settings: SkinSettings,
        pub unreal_body: Option<BodyRequest>,
        pub outfit_name: String,
        pub outfit_category: String,
        pub apply_to_character: bool,
    }

    impl From<ProjectV1> for Project {
        fn from(p: ProjectV1) -> Self {
            Project {
                scene: p.scene,
                garments: p
                    .garments
                    .into_iter()
                    .map(|g| SavedGarment {
                        part: g.part,
                        original: g.original,
                        pivot: g.pivot,
                        placement: g.placement,
                        reference: g.reference,
                        use_reference: g.use_reference,
                        status: g.status,
                        include: g.include,
                        layer: 1,
                    })
                    .collect(),
                selected_garment: p.selected_garment,
                fit_settings: p.fit_settings,
                skin_settings: p.skin_settings,
                unreal_body: p.unreal_body,
                outfit_name: p.outfit_name,
                outfit_category: p.outfit_category,
                apply_to_character: p.apply_to_character,
                hide_body: true,
            }
        }
    }
}

/// `Documents\ThreadSmith\Projects`.
pub fn default_dir() -> PathBuf {
    dirs_documents().join("ThreadSmith").join("Projects")
}

fn dirs_documents() -> PathBuf {
    ts_gen::generated_dir()
        .parent()
        .map(Path::to_path_buf)
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .unwrap_or_else(std::env::temp_dir)
}

/// A file name from a character name ("Ryan" → "Ryan.tsproj").
pub fn suggested_name(character: &str) -> String {
    let clean: String = character
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' || c == ' ' { c } else { '_' })
        .collect();
    let clean = clean.trim();
    format!("{}.{EXTENSION}", if clean.is_empty() { "Untitled" } else { clean })
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::{Mat4, Quat, Vec2};
    use ts_core::{Joint, Material, MeshPart, PartRole, Skeleton, Skin, Texture, Trs, MAX_INFLUENCES};

    #[test]
    fn save_and_open_round_trip() {
        let mut w = [0.0; MAX_INFLUENCES];
        w[0] = 1.0;
        let body = Mesh {
            positions: vec![Vec3::ZERO, Vec3::X, Vec3::Y],
            normals: vec![Vec3::Z; 3],
            uvs: vec![Vec2::ZERO; 3],
            indices: vec![0, 1, 2],
            skin: Some(Skin { joints: vec![[0; MAX_INFLUENCES]; 3], weights: vec![w; 3], inverse_bind: vec![Mat4::IDENTITY] }),
        };
        let mut shirt = MeshPart::new("Shirt", body.clone());
        shirt.role = PartRole::Garment;
        shirt.material = Some(Material {
            base_color_texture: Texture::from_bytes("t.png", vec![0x89, b'P', b'N', b'G', 1, 2, 3]),
            ..Material::with_color("Camo", [0.2, 0.5, 0.1])
        });
        let scene = Scene {
            name: "Ryan".into(),
            parts: vec![MeshPart::new("Body", body.clone()), shirt],
            skeleton: Skeleton { joints: vec![Joint { name: "root".into(), parent: None, bind_local: Trs::default() }] },
        };
        let mut placement = Placement::default();
        placement.rotation = Quat::from_rotation_y(0.5);
        placement.size = 1.15;
        let project = Project {
            scene,
            garments: vec![SavedGarment {
                part: 1,
                original: body,
                pivot: Vec3::new(0.0, 1.2, 0.0),
                placement,
                reference: None,
                use_reference: true,
                status: GarmentStatus::Skinned,
                include: true,
                layer: 2,
            }],
            selected_garment: 0,
            fit_settings: FitSettings::default(),
            skin_settings: SkinSettings { rigid: true, ..Default::default() },
            unreal_body: Some(BodyRequest {
                file: "C:/x.fbx".into(),
                character: "Ryan".into(),
                skeleton_asset: Some("/Game/Ryan/Skel".into()),
                body_asset: None,
                head_file: None,
            }),
            outfit_name: "Camo shirt".into(),
            outfit_category: "Top".into(),
            apply_to_character: true,
            hide_body: false,
        };
        let path = std::env::temp_dir().join("threadsmith_project_test.tsproj");
        project.save(&path).unwrap();
        let back = Project::load(&path).unwrap();
        assert_eq!(back.scene.parts.len(), 2);
        assert_eq!(back.scene.parts[1].material, project.scene.parts[1].material);
        assert_eq!(back.scene.parts[0].mesh.positions, project.scene.parts[0].mesh.positions);
        assert_eq!(back.garments[0].placement, placement);
        assert_eq!(back.garments[0].status, GarmentStatus::Skinned);
        assert!(back.skin_settings.rigid);
        assert_eq!(back.unreal_body.unwrap().skeleton_asset.as_deref(), Some("/Game/Ryan/Skel"));
        assert_eq!(back.outfit_name, "Camo shirt");
        std::fs::write(&path, b"not a project").unwrap();
        assert!(Project::load(&path).is_err());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn opens_version_1_files() {
        let v1 = legacy::ProjectV1 {
            scene: Scene { name: "Old".into(), ..Default::default() },
            garments: Vec::new(),
            selected_garment: 0,
            fit_settings: FitSettings::default(),
            skin_settings: SkinSettings::default(),
            unreal_body: None,
            outfit_name: "Old outfit".into(),
            outfit_category: "Top".into(),
            apply_to_character: false,
        };
        let data = postcard::to_stdvec(&v1).unwrap();
        let path = std::env::temp_dir().join("threadsmith_v1_test.tsproj");
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(MAGIC).unwrap();
        f.write_all(&1u32.to_le_bytes()).unwrap();
        let mut gz = flate2::write::GzEncoder::new(f, flate2::Compression::fast());
        gz.write_all(&data).unwrap();
        gz.finish().unwrap();
        let p = Project::load(&path).unwrap();
        assert_eq!(p.scene.name, "Old");
        assert_eq!(p.outfit_name, "Old outfit");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn names() {
        assert_eq!(suggested_name("Ryan"), "Ryan.tsproj");
        assert_eq!(suggested_name("a/b:c"), "a_b_c.tsproj");
        assert_eq!(suggested_name(""), "Untitled.tsproj");
    }
}
