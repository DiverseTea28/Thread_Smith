//! The model files the generator needs (`ai/models.json`): checking them and
//! downloading the missing ones into ComfyUI's models folder, with progress.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{bail, Context};
use serde::Deserialize;
use serde_json::Value;

#[derive(Clone, Debug, Deserialize)]
pub struct ModelList {
    pub sets: Vec<ModelSet>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ModelSet {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub license: String,
    #[serde(default)]
    pub size_gb: Option<f32>,
    #[serde(default)]
    pub files: Vec<ModelFile>,
    /// A whole Hugging Face repo to copy into one folder.
    #[serde(default)]
    pub hf_repo: Option<HfRepo>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ModelFile {
    /// Path inside ComfyUI's `models` folder.
    pub path: String,
    pub url: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct HfRepo {
    pub repo: String,
    /// Folder inside ComfyUI's `models` folder.
    pub path: String,
    /// A file that exists once the repo is fully there.
    pub check: String,
}

impl ModelList {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("Couldn't read {}", path.display()))?;
        Ok(serde_json::from_str(&text)
            .with_context(|| format!("{} has a mistake in it", path.display()))?)
    }
}

impl ModelSet {
    /// Files of this set that aren't in `models_dir` yet.
    pub fn missing(&self, models_dir: &Path) -> Vec<String> {
        let mut missing: Vec<String> = self
            .files
            .iter()
            .filter(|f| !models_dir.join(&f.path).is_file())
            .map(|f| f.path.clone())
            .collect();
        if let Some(repo) = &self.hf_repo {
            if !models_dir.join(&repo.path).join(&repo.check).is_file() {
                missing.push(format!("{} (whole model)", repo.path));
            }
        }
        missing
    }

    pub fn is_ready(&self, models_dir: &Path) -> bool {
        self.missing(models_dir).is_empty()
    }

    /// Download what's missing. `progress(file name, bytes done, bytes total)`.
    pub fn download(
        &self,
        models_dir: &Path,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(&str, u64, Option<u64>),
    ) -> anyhow::Result<()> {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_connect(Some(Duration::from_secs(30)))
            .http_status_as_error(false)
            .build()
            .into();
        let mut jobs: Vec<(String, PathBuf)> = self
            .files
            .iter()
            .filter(|f| !models_dir.join(&f.path).is_file())
            .map(|f| (f.url.clone(), models_dir.join(&f.path)))
            .collect();
        if let Some(repo) = &self.hf_repo {
            if !models_dir.join(&repo.path).join(&repo.check).is_file() {
                for file in hf_repo_files(&agent, &repo.repo)? {
                    let dest = models_dir.join(&repo.path).join(&file);
                    if !dest.is_file() {
                        jobs.push((
                            format!("https://huggingface.co/{}/resolve/main/{}", repo.repo, file),
                            dest,
                        ));
                    }
                }
            }
        }
        for (url, dest) in jobs {
            download_file(&agent, &url, &dest, cancel, progress)?;
        }
        Ok(())
    }
}

/// Every file in a public Hugging Face repo.
fn hf_repo_files(agent: &ureq::Agent, repo: &str) -> anyhow::Result<Vec<String>> {
    let mut resp = agent
        .get(&format!("https://huggingface.co/api/models/{repo}"))
        .call()
        .context("Couldn't reach Hugging Face")?;
    let text = resp.body_mut().read_to_string()?;
    let v: Value = serde_json::from_str(&text)?;
    let files: Vec<String> = v["siblings"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|s| s["rfilename"].as_str())
        .filter(|f| !f.starts_with('.') && !f.ends_with(".md") && !f.starts_with("assets/"))
        .map(str::to_string)
        .collect();
    if files.is_empty() {
        bail!("Hugging Face listed no files for {repo}");
    }
    Ok(files)
}

/// Download to `dest.part`, then rename, so a stopped download never looks finished.
fn download_file(
    agent: &ureq::Agent,
    url: &str,
    dest: &Path,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(&str, u64, Option<u64>),
) -> anyhow::Result<()> {
    let name = dest
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut resp = agent
        .get(url)
        .header("User-Agent", "ThreadSmith")
        .call()
        .with_context(|| format!("Couldn't start downloading {name}"))?;
    let status = resp.status().as_u16();
    if status != 200 {
        bail!("Downloading {name} failed: the server answered {status} ({url})");
    }
    let total = resp
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok());
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let part = dest.with_extension(format!(
        "{}.part",
        dest.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default()
    ));
    let mut out = std::io::BufWriter::new(std::fs::File::create(&part)?);
    let mut reader = resp.body_mut().with_config().limit(u64::MAX).reader();
    let mut buf = vec![0u8; 1 << 20];
    let mut done = 0u64;
    let mut last_report = 0u64;
    loop {
        if cancel.load(Ordering::Relaxed) {
            drop(out);
            let _ = std::fs::remove_file(&part);
            bail!("Stopped.");
        }
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        out.write_all(&buf[..n])?;
        done += n as u64;
        if done - last_report > 8 << 20 {
            last_report = done;
            progress(&name, done, total);
        }
    }
    out.flush()?;
    drop(out);
    if let Some(t) = total {
        if done != t {
            let _ = std::fs::remove_file(&part);
            bail!("{name} only partly downloaded ({done} of {t} bytes). Try again.");
        }
    }
    std::fs::rename(&part, dest)?;
    progress(&name, done, total);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_list_loads_and_checks() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ai/models.json");
        let list = ModelList::load(&path).unwrap();
        assert_eq!(list.sets.len(), 2);
        let empty = std::env::temp_dir().join("threadsmith_no_models_here");
        assert!(!list.sets[0].is_ready(&empty));
        assert_eq!(list.sets[1].missing(&empty).len(), 6);
    }
}
