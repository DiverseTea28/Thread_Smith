//! Generation jobs run on a background thread and report back through a channel,
//! so the window never freezes.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, bail, Context};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::comfy::{fill_workflow, Comfy, EngineSettings, Launcher, MeshEngine, Progress};
use crate::engine::{EngineHost, LivePreview};
use crate::models::ModelSet;

/// How much time to spend on the 3D model.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Quality {
    Fast,
    #[default]
    Balanced,
    Best,
}

impl Quality {
    pub const ALL: [Quality; 3] = [Quality::Fast, Quality::Balanced, Quality::Best];

    pub fn label(&self) -> &'static str {
        match self {
            Quality::Fast => "Fast",
            Quality::Balanced => "Balanced",
            Quality::Best => "Best",
        }
    }

    pub fn hint(&self) -> &'static str {
        match self {
            Quality::Fast => "Rough shape, 1K texture. Good for trying ideas.",
            Quality::Balanced => "Detailed shape, 2K texture.",
            Quality::Best => "Most detail, 2K texture. Slowest, uses the most memory.",
        }
    }

    fn workflow(&self) -> &'static str {
        match self {
            Quality::Fast => "garment_3d_fast.json",
            _ => "garment_3d.json",
        }
    }

    /// Name used by the ThreadSmith Engine.
    fn engine_name(&self) -> &'static str {
        match self {
            Quality::Fast => "fast",
            Quality::Balanced => "balanced",
            Quality::Best => "best",
        }
    }

    /// (steps, final resolution, texture size)
    fn numbers(&self) -> (u32, u32, u32) {
        match self {
            Quality::Fast => (8, 512, 1024),
            Quality::Balanced => (12, 1024, 2048),
            Quality::Best => (16, 1536, 2048),
        }
    }
}

pub enum GenRequest {
    /// Start ComfyUI if needed and report what's installed.
    Check,
    /// Make `count` pictures from a prompt (and a reference picture, if given).
    Images {
        prompt: String,
        reference: Option<PathBuf>,
        count: u32,
        seed: u64,
    },
    /// Turn a picture into a textured 3D model (GLB).
    Mesh {
        image: PathBuf,
        quality: Quality,
        faces: u32,
        seed: u64,
    },
    /// Download missing model files into ComfyUI's models folder.
    Download { sets: Vec<ModelSet> },
}

/// What the engine has, from a [`GenRequest::Check`].
#[derive(Clone, Debug, Default)]
pub struct EngineReport {
    pub running: bool,
    /// The TRELLIS.2 nodes are installed in ComfyUI.
    pub trellis_nodes: bool,
    /// Model sets (by id) that are missing files.
    pub missing_models: Vec<String>,
    /// The ThreadSmith Engine was checked (Pixal3D is the 3D model).
    pub engine_checked: bool,
    /// Pixal3D is installed and ready.
    pub pixal3d_ready: bool,
    /// What's missing for Pixal3D, in plain words.
    pub engine_problems: Vec<String>,
    /// Graphics card name and memory (GB), from the engine.
    pub gpu: Option<(String, f32)>,
}

impl EngineReport {
    /// Everything the chosen 3D model needs is there.
    pub fn ready_for(&self, engine: MeshEngine) -> bool {
        let mesh = match engine {
            MeshEngine::Pixal3D => self.engine_checked && self.pixal3d_ready,
            MeshEngine::Trellis2Comfy => self.trellis_nodes,
        };
        self.running && mesh
    }
}

pub enum GenEvent {
    /// One line for the status text.
    Status(String),
    /// Overall progress 0..1 and what's happening.
    Progress { label: String, fraction: f32 },
    Image(PathBuf),
    Mesh(PathBuf),
    /// The model forming (live view).
    Preview(LivePreview),
    Report(EngineReport),
    /// The job ended (Ok) or failed (Err with a message for the user).
    Finished(Result<(), String>),
}

pub struct GenJob {
    pub rx: Receiver<GenEvent>,
    cancel: Arc<AtomicBool>,
}

impl GenJob {
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    /// Start a job. `wake` is called after every event (to redraw the window).
    #[allow(clippy::too_many_arguments)]
    pub fn start(
        settings: EngineSettings,
        launcher: Arc<Mutex<Launcher>>,
        engine: Arc<Mutex<EngineHost>>,
        ai_dir: PathBuf,
        models: Vec<ModelSet>,
        request: GenRequest,
        wake: impl Fn() + Send + 'static,
    ) -> Self {
        let (tx, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let c = cancel.clone();
        std::thread::spawn(move || {
            let send = |e: GenEvent| {
                let _ = tx.send(e);
                wake();
            };
            let result = run(&settings, &launcher, &engine, &ai_dir, &models, request, &c, &send)
                .map_err(|e| format!("{e:#}"));
            send(GenEvent::Finished(result));
        });
        Self { rx, cancel }
    }
}

#[allow(clippy::too_many_arguments)]
fn run(
    settings: &EngineSettings,
    launcher: &Mutex<Launcher>,
    engine: &Mutex<EngineHost>,
    ai_dir: &Path,
    models: &[ModelSet],
    request: GenRequest,
    cancel: &AtomicBool,
    send: &dyn Fn(GenEvent),
) -> anyhow::Result<()> {
    if let GenRequest::Download { sets } = &request {
        let models_dir = settings
            .models_dir()
            .ok_or_else(|| anyhow!("Set the ComfyUI folder first, so ThreadSmith knows where models go."))?;
        for set in sets {
            send(GenEvent::Status(format!("Downloading {}…", set.name)));
            set.download(&models_dir, cancel, &mut |file, done, total| {
                let gb = |b: u64| b as f64 / 1e9;
                let (label, fraction) = match total {
                    Some(t) if t > 0 => (
                        format!("{}: {file} {:.1} / {:.1} GB", set.name, gb(done), gb(t)),
                        done as f32 / t as f32,
                    ),
                    _ => (format!("{}: {file} {:.1} GB", set.name, gb(done)), 0.0),
                };
                send(GenEvent::Progress { label, fraction });
            })?;
        }
        send(GenEvent::Status("Models downloaded.".into()));
        return Ok(());
    }

    if let GenRequest::Mesh { image, quality, faces, seed } = &request {
        if settings.mesh_engine == MeshEngine::Pixal3D {
            let mut host = engine.lock().unwrap();
            return engine_mesh(settings, &mut host, ai_dir, image, *quality, *faces, *seed, cancel, send);
        }
    }
    let mut engine_part = None;
    if matches!(request, GenRequest::Check) && settings.mesh_engine == MeshEngine::Pixal3D {
        send(GenEvent::Progress { label: "Starting the ThreadSmith Engine…".into(), fraction: 0.0 });
        let mut host = engine.lock().unwrap();
        engine_part = Some(engine_status(settings, &mut host, ai_dir, cancel));
    }

    let comfy = Comfy::new(settings);
    if !comfy.is_up() {
        if !settings.auto_start {
            bail!(
                "ComfyUI isn't running at {}. Start it, or turn on \"Start ComfyUI for me\".",
                settings.base_url()
            );
        }
        send(GenEvent::Progress {
            label: "Starting the AI engine (ComfyUI)… this takes a minute the first time".into(),
            fraction: 0.0,
        });
        launcher.lock().unwrap().start(settings, cancel)?;
    }

    match request {
        GenRequest::Check => {
            let models_dir = settings.models_dir();
            let mut report = EngineReport {
                running: true,
                trellis_nodes: comfy.has_node("Trellis2LoadModel"),
                missing_models: models
                    .iter()
                    .filter(|s| s.needed_for(settings.mesh_engine) && models_dir.as_ref().is_some_and(|d| !s.is_ready(d)))
                    .map(|s| s.id.clone())
                    .collect(),
                ..Default::default()
            };
            match engine_part {
                Some(Ok(status)) => apply_engine_status(&mut report, &status),
                Some(Err(e)) => {
                    report.engine_checked = true;
                    report.engine_problems = vec![format!("{e:#}")];
                }
                None => {}
            }
            send(GenEvent::Report(report));
            Ok(())
        }
        GenRequest::Images { prompt, reference, count, seed } => {
            let have = comfy.models_in("diffusion_models");
            if !have.iter().any(|m| m == &settings.image_model) {
                bail!(
                    "ComfyUI doesn't have the image model {} yet. Use \"Download models\" in the generator settings.",
                    settings.image_model
                );
            }
            let (file, ref_name) = match &reference {
                Some(path) => {
                    send(GenEvent::Progress { label: "Sending your picture".into(), fraction: 0.0 });
                    ("image_from_image.json", Some(comfy.upload_image(path)?))
                }
                None => ("image_from_text.json", None),
            };
            let template = load_workflow(ai_dir, file)?;
            let out_dir = crate::new_run_dir()?;
            for i in 0..count {
                let mut vars = base_vars(settings);
                vars.insert("PROMPT".into(), Value::from(prompt.clone()));
                vars.insert("SEED".into(), Value::from(seed + i as u64));
                vars.insert("STEPS".into(), Value::from(4));
                vars.insert("WIDTH".into(), Value::from(1024));
                vars.insert("HEIGHT".into(), Value::from(1024));
                if let Some(r) = &ref_name {
                    vars.insert("REF_IMAGE".into(), Value::from(r.clone()));
                }
                let wf = fill_workflow(&template, &vars)?;
                let outputs = comfy.run(&wf, cancel, &mut |p: Progress| {
                    let within = p.fraction.unwrap_or(0.0);
                    send(GenEvent::Progress {
                        label: format!("Picture {} of {count}: {}", i + 1, p.step),
                        fraction: (i as f32 + within) / count as f32,
                    });
                })?;
                for f in output_files(&template, &outputs) {
                    let path = comfy.download_output(&f, &out_dir)?;
                    send(GenEvent::Image(path));
                }
            }
            send(GenEvent::Status(format!("Made {count} picture(s). Pick one, then Make 3D.")));
            Ok(())
        }
        GenRequest::Mesh { image, quality, faces, seed } => {
            if !comfy.has_node("Trellis2LoadModel") {
                bail!(
                    "The TRELLIS.2 nodes aren't installed in ComfyUI. In ComfyUI-Easy-Install, run Add-Ons\\Trellis2 (requires Torch 2.8.0+cu128).bat, then try again."
                );
            }
            send(GenEvent::Progress { label: "Sending the picture".into(), fraction: 0.0 });
            let name = comfy.upload_image(&image)?;
            let template = load_workflow(ai_dir, quality.workflow())?;
            let (steps, resolution, texture) = quality.numbers();
            let mut vars = base_vars(settings);
            vars.insert("IMAGE".into(), Value::from(name));
            vars.insert("SEED".into(), Value::from(seed & 0x7fff_ffff));
            vars.insert("STEPS".into(), Value::from(steps));
            vars.insert("RESOLUTION".into(), Value::from(resolution));
            vars.insert("FACES".into(), Value::from(faces));
            vars.insert("TEXTURE_SIZE".into(), Value::from(texture));
            let wf = fill_workflow(&template, &vars)?;
            let outputs = comfy.run(&wf, cancel, &mut |p: Progress| {
                let step_share = 1.0 / p.total.max(1) as f32;
                send(GenEvent::Progress {
                    label: format!("{} ({} of {})", p.step, (p.done + 1).min(p.total), p.total),
                    fraction: (p.done as f32 + p.fraction.unwrap_or(0.0)) * step_share,
                });
            })?;
            let out_dir = image
                .parent()
                .map(Path::to_path_buf)
                .filter(|d| d.starts_with(crate::generated_dir()))
                .map(Ok)
                .unwrap_or_else(crate::new_run_dir)?;
            let file = output_files(&template, &outputs)
                .into_iter()
                .find(|f| f["filename"].as_str().is_some_and(|n| n.ends_with(".glb")))
                .ok_or_else(|| anyhow!("TRELLIS.2 finished but didn't save a model"))?;
            let path = comfy.download_output(&file, &out_dir)?;
            send(GenEvent::Mesh(path));
            Ok(())
        }
        GenRequest::Download { .. } => unreachable!(),
    }
}

/// Start the engine (if needed) and ask what it has.
fn engine_status(
    settings: &EngineSettings,
    host: &mut EngineHost,
    ai_dir: &Path,
    cancel: &AtomicBool,
) -> anyhow::Result<Value> {
    host.start(settings, ai_dir, cancel)?;
    host.send(&serde_json::json!({"type": "status"}))?;
    loop {
        let m = host.recv(cancel)?;
        match m.kind() {
            "status" => return Ok(m.header),
            "error" => bail!("{}", m.header["message"].as_str().unwrap_or("The engine had a problem")),
            _ => {}
        }
    }
}

fn apply_engine_status(report: &mut EngineReport, status: &Value) {
    report.engine_checked = true;
    report.pixal3d_ready = status["models"]["pixal3d"]["installed"].as_bool().unwrap_or(false);
    report.engine_problems = status["problems"]
        .as_array()
        .map(|a| a.iter().filter_map(|p| p.as_str().map(String::from)).collect())
        .unwrap_or_default();
    if report.engine_problems.is_empty() && !report.pixal3d_ready {
        if let Some(p) = status["problem"].as_str() {
            report.engine_problems.push(p.to_string());
        }
    }
    if let Some(gpu) = status["gpu"].as_str() {
        report.gpu = Some((gpu.to_string(), status["vram_gb"].as_f64().unwrap_or(0.0) as f32));
    }
}

/// Make a 3D model with Pixal3D in the ThreadSmith Engine, forwarding live
/// previews.
#[allow(clippy::too_many_arguments)]
fn engine_mesh(
    settings: &EngineSettings,
    host: &mut EngineHost,
    ai_dir: &Path,
    image: &Path,
    quality: Quality,
    faces: u32,
    seed: u64,
    cancel: &AtomicBool,
    send: &dyn Fn(GenEvent),
) -> anyhow::Result<()> {
    // FLUX (the picture model) may still be sitting in the graphics card's
    // memory inside ComfyUI. Pixal3D needs that room.
    let comfy = Comfy::new(settings);
    if comfy.is_up() {
        comfy.free_memory();
    }
    if !host.is_running() {
        send(GenEvent::Progress { label: "Starting the ThreadSmith Engine…".into(), fraction: 0.0 });
    }
    host.start(settings, ai_dir, cancel)?;

    let out_dir = image
        .parent()
        .map(Path::to_path_buf)
        .filter(|d| d.starts_with(crate::generated_dir()))
        .map(Ok)
        .unwrap_or_else(crate::new_run_dir)?;
    let (_, _, texture) = quality.numbers();
    host.send(&serde_json::json!({
        "type": "generate",
        "image": image,
        "out_dir": out_dir,
        "quality": quality.engine_name(),
        "faces": faces,
        "texture_size": texture,
        "seed": seed & 0x7fff_ffff,
        "live": settings.live_view,
        "fov_deg": settings.lens_degrees,
    }))?;
    loop {
        let m = host.recv(cancel)?;
        match m.kind() {
            "progress" => {
                let label = m.header["label"].as_str().unwrap_or("Working").to_string();
                let step = m.header["step"].as_u64().unwrap_or(0);
                let total = m.header["total"].as_u64().unwrap_or(0);
                let label = if total > 0 { format!("{label} ({step} of {total})") } else { label };
                let fraction = m.header["fraction"].as_f64().unwrap_or(0.0) as f32;
                send(GenEvent::Progress { label, fraction });
            }
            "preview_voxels" | "preview_mesh" => {
                if let Some(p) = LivePreview::from_message(&m) {
                    send(GenEvent::Preview(p));
                }
            }
            "log" => log::info!("Engine: {}", m.header["text"].as_str().unwrap_or("")),
            "done" => {
                let path = m.header["glb_path"].as_str().ok_or_else(|| anyhow!("The engine finished but didn't say where the model is"))?;
                send(GenEvent::Mesh(PathBuf::from(path)));
                return Ok(());
            }
            "error" => {
                let message = m.header["message"].as_str().unwrap_or("The engine had a problem").to_string();
                if let Some(d) = m.header["detail"].as_str() {
                    log::warn!("Engine error detail:\n{d}");
                }
                if m.header["cancelled"].as_bool() == Some(true) {
                    bail!("Stopped.");
                }
                bail!("{message}\n(More detail in {})", EngineHost::log_path().display());
            }
            _ => {}
        }
    }
}

fn base_vars(settings: &EngineSettings) -> HashMap<String, Value> {
    let mut vars = HashMap::new();
    vars.insert("IMAGE_MODEL".into(), Value::from(settings.image_model.clone()));
    vars.insert("TEXT_ENCODER".into(), Value::from(settings.text_encoder.clone()));
    vars.insert("IMAGE_VAE".into(), Value::from(settings.image_vae.clone()));
    vars.insert("ATTENTION".into(), Value::from(settings.attention.clone()));
    vars
}

fn load_workflow(ai_dir: &Path, file: &str) -> anyhow::Result<Value> {
    let path = ai_dir.join("workflows").join(file);
    let text = std::fs::read_to_string(&path).with_context(|| format!("Couldn't read {}", path.display()))?;
    Ok(serde_json::from_str(&text).with_context(|| format!("{} has a mistake in it", path.display()))?)
}

/// Files saved by the workflow's output node (any list of {filename, …}).
fn output_files(template: &Value, outputs: &Value) -> Vec<Value> {
    let node = template["_threadsmith"]["output_node"].as_str().unwrap_or("");
    let mut files = Vec::new();
    let mut take = |v: &Value| {
        if let Some(o) = v.as_object() {
            for list in o.values() {
                for f in list.as_array().into_iter().flatten() {
                    if f.get("filename").is_some() {
                        files.push(f.clone());
                    }
                }
            }
        }
    };
    match outputs.get(node) {
        Some(v) => take(v),
        None => outputs.as_object().into_iter().flatten().for_each(|(_, v)| take(v)),
    }
    files
}

/// Values for every blank, for tests.
#[doc(hidden)]
pub fn test_vars() -> HashMap<String, Value> {
    let mut vars = base_vars(&EngineSettings::default());
    for (k, v) in [
        ("PROMPT", Value::from("a test")),
        ("SEED", Value::from(1)),
        ("STEPS", Value::from(4)),
        ("WIDTH", Value::from(1024)),
        ("HEIGHT", Value::from(1024)),
        ("REF_IMAGE", Value::from("x.png")),
        ("IMAGE", Value::from("x.png")),
        ("RESOLUTION", Value::from(1024)),
        ("FACES", Value::from(150000)),
        ("TEXTURE_SIZE", Value::from(2048)),
    ] {
        vars.insert(k.into(), v);
    }
    vars
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_output_files() {
        let template: Value = serde_json::json!({"_threadsmith": {"output_node": "12"}});
        let outputs: Value = serde_json::json!({
            "12": {"3d": [{"filename": "garment_00001_.glb", "subfolder": "ThreadSmith", "type": "output"}]},
            "13": {"images": [{"filename": "other.png"}]}
        });
        let files = output_files(&template, &outputs);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0]["filename"], "garment_00001_.glb");
    }
}
