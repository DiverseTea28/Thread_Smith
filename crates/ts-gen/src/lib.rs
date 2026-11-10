//! AI garment generation for ThreadSmith.
//!
//! 1. **Images:** FLUX.2 [klein] 4B turns the garment preset + the user's text
//!    (and optional reference image) into a few "ghost mannequin" pictures.
//!    This runs in a local ComfyUI (hidden in the background) through its HTTP API.
//! 2. **3D:** Pixal3D turns the picked picture into a textured GLB, run by the
//!    ThreadSmith Engine (`ai/engine/threadsmith_engine.py`, see [`engine`]),
//!    which sends live previews while the model forms. TRELLIS.2 through a
//!    ComfyUI workflow is still available as the other choice.
//!
//! Data files live in the `ai/` folder next to the app:
//! - `ai/presets/garment_presets.json`: hidden prompts and placement per garment type.
//! - `ai/workflows/*.json`: ComfyUI workflows (API format) with `$NAME` blanks.
//! - `ai/models.json`: model files and where to download them.

pub mod comfy;
pub mod engine;
pub mod jobs;
pub mod models;
pub mod presets;

use std::path::{Path, PathBuf};

pub use comfy::{Comfy, EngineSettings, Launcher, MeshEngine};
pub use engine::{EngineHost, LivePreview};
pub use jobs::{GenEvent, GenJob, GenRequest, Quality};
pub use models::{ModelFile, ModelSet};
pub use presets::{Garment, Place, Presets};

/// The `ai/` data folder: next to the app, or (while developing) in the
/// project folder a few levels above `target/release/threadsmith.exe`.
pub fn ai_dir() -> Option<PathBuf> {
    let mut starts: Vec<PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        starts.push(exe);
    }
    if let Ok(cwd) = std::env::current_dir() {
        starts.push(cwd);
    }
    for start in starts {
        let mut dir: Option<&Path> = Some(start.as_path());
        while let Some(d) = dir {
            let candidate = d.join("ai");
            if candidate.join("presets").join("garment_presets.json").is_file() {
                return Some(candidate);
            }
            dir = d.parent();
        }
    }
    None
}

/// Where generated pictures and models are kept:
/// `Documents\ThreadSmith\Generated`.
pub fn generated_dir() -> PathBuf {
    dirs::document_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("ThreadSmith")
        .join("Generated")
}

/// A fresh folder for one generation run.
pub fn new_run_dir() -> std::io::Result<PathBuf> {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or_default();
    let dir = generated_dir().join(format!("run_{secs}"));
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// A random-enough seed without pulling in a random number crate.
pub fn random_seed() -> u64 {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u128(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default(),
    );
    h.finish() & 0x7fff_ffff_ffff
}
