//! Talking to a local ComfyUI: start it hidden, send workflows, follow
//! progress, download results.
//!
//! API used (all on 127.0.0.1):
//! - `GET /system_stats`: is it running?
//! - `GET /object_info/<Node>`: is a node installed?
//! - `POST /upload/image`: give it our picture.
//! - `POST /prompt`: run a workflow. `GET /history/<id>`: its results.
//! - `GET /view`: download a result. `POST /interrupt`: stop.
//! - `ws://…/ws?clientId=…`: live progress.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Settings for the generator engine (saved by the app).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct EngineSettings {
    /// ComfyUI install folder: the one holding `ComfyUI\` and `python_embeded\`
    /// (ComfyUI-Easy-Install and the portable ComfyUI both look like this).
    pub comfy_root: Option<PathBuf>,
    pub port: u16,
    /// Start ComfyUI (hidden) when it isn't running.
    pub auto_start: bool,
    /// TRELLIS.2 attention backend: "flash_attn" (fast) or "sdpa" (works everywhere).
    pub attention: String,
    pub image_model: String,
    pub text_encoder: String,
    pub image_vae: String,
    /// Which 3D model makes the garment.
    pub mesh_engine: MeshEngine,
    /// Show the model forming in the viewport while it's made.
    pub live_view: bool,
    /// Lens width (degrees) Pixal3D assumes for the picture. Product
    /// pictures are close to a narrow, straight-on lens.
    pub lens_degrees: f32,
}

/// Which 3D model turns the picture into a garment.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum MeshEngine {
    /// Pixal3D, run by ThreadSmith's own engine (live previews).
    #[default]
    Pixal3D,
    /// TRELLIS.2 through a ComfyUI workflow (no live previews).
    Trellis2Comfy,
}

impl MeshEngine {
    pub const ALL: [MeshEngine; 2] = [MeshEngine::Pixal3D, MeshEngine::Trellis2Comfy];

    pub fn label(&self) -> &'static str {
        match self {
            MeshEngine::Pixal3D => "Pixal3D (ThreadSmith Engine)",
            MeshEngine::Trellis2Comfy => "TRELLIS.2 (ComfyUI workflow)",
        }
    }

    pub fn hint(&self) -> &'static str {
        match self {
            MeshEngine::Pixal3D => "Follows the picture closely. Shows the model forming live. MIT licence.",
            MeshEngine::Trellis2Comfy => "Runs as a ComfyUI workflow. No live view. MIT licence.",
        }
    }
}

impl Default for EngineSettings {
    fn default() -> Self {
        Self {
            comfy_root: detect_comfy_root(),
            port: 8188,
            auto_start: true,
            attention: "flash_attn".into(),
            image_model: "flux-2-klein-4b.safetensors".into(),
            text_encoder: "qwen_3_4b.safetensors".into(),
            image_vae: "flux2-vae.safetensors".into(),
            mesh_engine: MeshEngine::Pixal3D,
            live_view: true,
            lens_degrees: 20.0,
        }
    }
}

impl EngineSettings {
    pub fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// `ComfyUI\models` inside the install folder.
    pub fn models_dir(&self) -> Option<PathBuf> {
        let root = self.comfy_root.as_ref()?;
        Some(comfy_dir(root)?.join("models"))
    }
}

/// The folder with ComfyUI's `main.py` inside an install folder.
pub fn comfy_dir(root: &Path) -> Option<PathBuf> {
    [root.join("ComfyUI"), root.to_path_buf()]
        .into_iter()
        .find(|d| d.join("main.py").is_file())
}

/// The Python that came with an install folder.
pub fn python_exe(root: &Path) -> Option<PathBuf> {
    [
        root.join("python_embeded").join("python.exe"),
        root.join("python_embedded").join("python.exe"),
        root.join(".venv").join("Scripts").join("python.exe"),
        root.join("venv").join("Scripts").join("python.exe"),
    ]
    .into_iter()
    .find(|p| p.is_file())
}

/// Look for ComfyUI in the usual places.
pub fn detect_comfy_root() -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = vec![
        r"C:\ComfyUI-Easy-Install\ComfyUI-Easy-Install".into(),
        r"C:\ComfyUI-Easy-Install".into(),
        r"C:\ComfyUI_windows_portable".into(),
        r"D:\ComfyUI-Easy-Install\ComfyUI-Easy-Install".into(),
        r"D:\ComfyUI_windows_portable".into(),
    ];
    if let Some(home) = dirs::home_dir() {
        candidates.push(home.join("ComfyUI_windows_portable"));
        candidates.push(home.join("Documents").join("ThreadSmith").join("AI").join("Engine"));
    }
    candidates
        .into_iter()
        .find(|c| comfy_dir(c).is_some() && python_exe(c).is_some())
}

/// Fill a workflow template: replace every `"$NAME"` string with its value,
/// and drop ThreadSmith's own notes (top-level keys starting with `_`).
pub fn fill_workflow(template: &Value, vars: &HashMap<String, Value>) -> anyhow::Result<Value> {
    fn walk(v: &Value, vars: &HashMap<String, Value>, missing: &mut Vec<String>) -> Value {
        match v {
            Value::String(s) if s.starts_with('$') => match vars.get(&s[1..]) {
                Some(x) => x.clone(),
                None => {
                    missing.push(s.clone());
                    v.clone()
                }
            },
            Value::Array(a) => Value::Array(a.iter().map(|x| walk(x, vars, missing)).collect()),
            Value::Object(o) => Value::Object(
                o.iter()
                    .map(|(k, x)| (k.clone(), walk(x, vars, missing)))
                    .collect(),
            ),
            _ => v.clone(),
        }
    }
    let Value::Object(nodes) = template else {
        bail!("A workflow must be a JSON object of nodes");
    };
    let mut missing = Vec::new();
    let mut out = serde_json::Map::new();
    for (id, node) in nodes {
        if id.starts_with('_') {
            continue;
        }
        out.insert(id.clone(), walk(node, vars, &mut missing));
    }
    if !missing.is_empty() {
        missing.sort();
        missing.dedup();
        bail!("The workflow still has blanks: {}", missing.join(", "));
    }
    Ok(Value::Object(out))
}

/// Live progress from a running workflow.
#[derive(Clone, Debug)]
pub struct Progress {
    /// Friendly name of the step running now (the node's title).
    pub step: String,
    /// 0..1 within the step, when the step reports it.
    pub fraction: Option<f32>,
    /// Steps finished / total steps in the workflow.
    pub done: usize,
    pub total: usize,
}

pub struct Comfy {
    base: String,
    client_id: String,
    agent: ureq::Agent,
}

impl Comfy {
    pub fn new(settings: &EngineSettings) -> Self {
        Self {
            base: settings.base_url(),
            client_id: format!("threadsmith-{}", crate::random_seed()),
            agent: ureq::Agent::config_builder()
                .timeout_global(Some(Duration::from_secs(600)))
                .http_status_as_error(false)
                .build()
                .into(),
        }
    }

    fn quick_agent() -> ureq::Agent {
        ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_millis(1500)))
            .http_status_as_error(false)
            .build()
            .into()
    }

    fn get_json(&self, path: &str) -> anyhow::Result<Value> {
        let mut resp = self
            .agent
            .get(&format!("{}{}", self.base, path))
            .call()
            .with_context(|| format!("ComfyUI didn't answer at {}", self.base))?;
        let status = resp.status().as_u16();
        let text = resp.body_mut().read_to_string().unwrap_or_default();
        if status != 200 {
            bail!("ComfyUI answered {status} for {path}: {text}");
        }
        Ok(serde_json::from_str(&text)?)
    }

    /// Is ComfyUI running and answering?
    pub fn is_up(&self) -> bool {
        Self::quick_agent()
            .get(&format!("{}/system_stats", self.base))
            .call()
            .is_ok_and(|r| r.status().as_u16() == 200)
    }

    /// Is a node type installed (e.g. "Trellis2LoadModel")?
    pub fn has_node(&self, class: &str) -> bool {
        self.get_json(&format!("/object_info/{class}"))
            .is_ok_and(|v| v.get(class).is_some())
    }

    /// Files ComfyUI sees in one of its model folders ("diffusion_models", "vae"…).
    pub fn models_in(&self, folder: &str) -> Vec<String> {
        self.get_json(&format!("/models/{folder}"))
            .ok()
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default()
    }

    /// Copy a picture into ComfyUI's input folder. Returns the name to use in workflows.
    pub fn upload_image(&self, path: &Path) -> anyhow::Result<String> {
        let bytes = std::fs::read(path).with_context(|| format!("Couldn't read {}", path.display()))?;
        let ext = path
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_else(|| "png".into());
        let name = format!("threadsmith_{}.{ext}", crate::random_seed());
        let mime = match ext.as_str() {
            "jpg" | "jpeg" => "image/jpeg",
            "webp" => "image/webp",
            _ => "image/png",
        };
        let boundary = format!("----threadsmith{}", crate::random_seed());
        let mut body = Vec::with_capacity(bytes.len() + 512);
        body.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"image\"; filename=\"{name}\"\r\nContent-Type: {mime}\r\n\r\n"
            )
            .as_bytes(),
        );
        body.extend_from_slice(&bytes);
        body.extend_from_slice(
            format!("\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"overwrite\"\r\n\r\ntrue\r\n--{boundary}--\r\n").as_bytes(),
        );
        let mut resp = self
            .agent
            .post(&format!("{}/upload/image", self.base))
            .header("Content-Type", &format!("multipart/form-data; boundary={boundary}"))
            .send(&body[..])
            .context("Couldn't send the picture to ComfyUI")?;
        let text = resp.body_mut().read_to_string().unwrap_or_default();
        let v: Value = serde_json::from_str(&text).unwrap_or_default();
        match (v["name"].as_str(), v["subfolder"].as_str()) {
            (Some(n), Some(sub)) if !sub.is_empty() => Ok(format!("{sub}/{n}")),
            (Some(n), _) => Ok(n.to_string()),
            _ => bail!("ComfyUI didn't accept the picture: {text}"),
        }
    }

    /// Ask ComfyUI to unload its models, so the graphics card has room for
    /// the 3D engine. Does nothing if ComfyUI isn't running.
    pub fn free_memory(&self) {
        let _ = Self::quick_agent()
            .post(&format!("{}/free", self.base))
            .header("Content-Type", "application/json")
            .send(r#"{"unload_models": true, "free_memory": true}"#);
    }

    /// Stop whatever is running.
    pub fn interrupt(&self) {
        let _ = Self::quick_agent()
            .post(&format!("{}/interrupt", self.base))
            .send_empty();
    }

    fn queue(&self, workflow: &Value) -> anyhow::Result<String> {
        let body = serde_json::json!({ "prompt": workflow, "client_id": self.client_id });
        let mut resp = self
            .agent
            .post(&format!("{}/prompt", self.base))
            .header("Content-Type", "application/json")
            .send(body.to_string().as_str())
            .with_context(|| format!("ComfyUI didn't answer at {}", self.base))?;
        let text = resp.body_mut().read_to_string().unwrap_or_default();
        let v: Value = serde_json::from_str(&text).unwrap_or_default();
        if let Some(id) = v["prompt_id"].as_str() {
            return Ok(id.to_string());
        }
        bail!("ComfyUI refused the workflow: {}", explain_prompt_error(&v, &text))
    }

    /// Download one output file (`/view`) into `dest_dir`. Returns its path.
    pub fn download_output(&self, file: &Value, dest_dir: &Path) -> anyhow::Result<PathBuf> {
        let name = file["filename"].as_str().ok_or_else(|| anyhow!("Output has no file name"))?;
        let sub = file["subfolder"].as_str().unwrap_or("");
        let kind = file["type"].as_str().unwrap_or("output");
        let url = format!(
            "{}/view?filename={}&subfolder={}&type={}",
            self.base,
            encode(name),
            encode(sub),
            encode(kind)
        );
        let mut resp = self.agent.get(&url).call().context("Couldn't download the result")?;
        if resp.status().as_u16() != 200 {
            bail!("ComfyUI couldn't find the result {name}");
        }
        std::fs::create_dir_all(dest_dir)?;
        let dest = dest_dir.join(name);
        let mut out = std::fs::File::create(&dest)?;
        std::io::copy(&mut resp.body_mut().as_reader(), &mut out)?;
        Ok(dest)
    }

    /// Run a filled workflow and wait for it. Calls `on_progress` as it goes.
    /// Returns the workflow's outputs (node id → what it saved).
    pub fn run(
        &self,
        workflow: &Value,
        cancel: &AtomicBool,
        on_progress: &mut dyn FnMut(Progress),
    ) -> anyhow::Result<Value> {
        let titles: HashMap<String, String> = workflow
            .as_object()
            .map(|o| {
                o.iter()
                    .map(|(id, n)| {
                        let t = n["_meta"]["title"]
                            .as_str()
                            .or(n["class_type"].as_str())
                            .unwrap_or("Working")
                            .to_string();
                        (id.clone(), t)
                    })
                    .collect()
            })
            .unwrap_or_default();
        let total = titles.len();

        // Listen first so no progress is missed.
        let ws_url = format!(
            "{}/ws?clientId={}",
            self.base.replacen("http://", "ws://", 1),
            self.client_id
        );
        let mut socket = match tungstenite::connect(ws_url.as_str()) {
            Ok((s, _)) => {
                if let tungstenite::stream::MaybeTlsStream::Plain(tcp) = s.get_ref() {
                    let _ = tcp.set_read_timeout(Some(Duration::from_millis(500)));
                }
                Some(s)
            }
            Err(e) => {
                log::warn!("No live progress from ComfyUI ({e}); checking now and then instead.");
                None
            }
        };

        let prompt_id = self.queue(workflow)?;
        let mut done_nodes = std::collections::HashSet::new();
        let mut current = String::from("Waiting for ComfyUI");
        let mut last_history_check = Instant::now();
        on_progress(Progress { step: current.clone(), fraction: None, done: 0, total });

        loop {
            if cancel.load(Ordering::Relaxed) {
                self.interrupt();
                bail!("Stopped.");
            }
            let mut got_message = false;
            if let Some(ws) = socket.as_mut() {
                match ws.read() {
                    Ok(tungstenite::Message::Text(text)) => {
                        got_message = true;
                        let msg: Value = serde_json::from_str(text.as_str()).unwrap_or_default();
                        let data = &msg["data"];
                        let for_us = data["prompt_id"].as_str().is_none_or(|p| p == prompt_id);
                        match msg["type"].as_str().unwrap_or("") {
                            "executing" if for_us => {
                                if let Some(node) = data["node"].as_str() {
                                    current = titles.get(node).cloned().unwrap_or_else(|| "Working".into());
                                    done_nodes.insert(node.to_string());
                                    on_progress(Progress {
                                        step: current.clone(),
                                        fraction: None,
                                        done: done_nodes.len().saturating_sub(1),
                                        total,
                                    });
                                }
                            }
                            "execution_cached" if for_us => {
                                if let Some(nodes) = data["nodes"].as_array() {
                                    for n in nodes.iter().filter_map(|n| n.as_str()) {
                                        done_nodes.insert(n.to_string());
                                    }
                                }
                            }
                            "progress" if for_us => {
                                let v = data["value"].as_f64().unwrap_or(0.0);
                                let m = data["max"].as_f64().unwrap_or(0.0);
                                if m > 0.0 {
                                    on_progress(Progress {
                                        step: current.clone(),
                                        fraction: Some((v / m) as f32),
                                        done: done_nodes.len().saturating_sub(1),
                                        total,
                                    });
                                }
                            }
                            "execution_error" if for_us => {
                                let node = data["node_type"].as_str().unwrap_or("a step");
                                let message = data["exception_message"].as_str().unwrap_or("unknown error");
                                bail!("{} failed in {node}: {}", current, message.trim());
                            }
                            "execution_interrupted" if for_us => bail!("Stopped."),
                            "execution_success" if for_us => break,
                            _ => {}
                        }
                    }
                    Ok(_) => got_message = true,
                    Err(tungstenite::Error::Io(e))
                        if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
                    Err(e) => {
                        log::warn!("Lost the live progress connection ({e}); checking now and then instead.");
                        socket = None;
                    }
                }
            }
            if socket.is_none() && !got_message {
                std::thread::sleep(Duration::from_millis(500));
            }
            // Backup: look at the history now and then (if the socket missed the end).
            if last_history_check.elapsed() > Duration::from_secs(3) {
                last_history_check = Instant::now();
                if let Some(result) = self.finished(&prompt_id)? {
                    return Ok(result);
                }
            }
        }
        // Finished: fetch the outputs.
        for _ in 0..20 {
            if let Some(result) = self.finished(&prompt_id)? {
                return Ok(result);
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        bail!("ComfyUI finished but didn't list any results")
    }

    /// `Some(outputs)` once the prompt is finished; an error if it failed.
    fn finished(&self, prompt_id: &str) -> anyhow::Result<Option<Value>> {
        let history = self.get_json(&format!("/history/{prompt_id}"))?;
        let Some(entry) = history.get(prompt_id) else {
            return Ok(None);
        };
        let status = &entry["status"];
        if status["status_str"].as_str() == Some("error") {
            let messages = status["messages"].as_array().cloned().unwrap_or_default();
            let detail = messages
                .iter()
                .find(|m| m[0].as_str() == Some("execution_error"))
                .map(|m| {
                    format!(
                        "{}: {}",
                        m[1]["node_type"].as_str().unwrap_or("a step"),
                        m[1]["exception_message"].as_str().unwrap_or("unknown error").trim()
                    )
                })
                .unwrap_or_else(|| "unknown error".into());
            bail!("ComfyUI failed in {detail}");
        }
        if status["completed"].as_bool() == Some(true) || entry["outputs"].as_object().is_some_and(|o| !o.is_empty()) {
            return Ok(Some(entry["outputs"].clone()));
        }
        Ok(None)
    }
}

/// Turn ComfyUI's "this workflow is wrong" answer into one readable line.
fn explain_prompt_error(v: &Value, raw: &str) -> String {
    let mut parts = Vec::new();
    if let Some(m) = v["error"]["message"].as_str() {
        parts.push(m.to_string());
    }
    if let Some(nodes) = v["node_errors"].as_object() {
        for (id, e) in nodes {
            let class = e["class_type"].as_str().unwrap_or(id);
            for err in e["errors"].as_array().into_iter().flatten() {
                parts.push(format!(
                    "{class}: {} {}",
                    err["message"].as_str().unwrap_or(""),
                    err["details"].as_str().unwrap_or("")
                ));
            }
        }
    }
    if parts.is_empty() {
        raw.chars().take(400).collect()
    } else {
        parts.join(" · ")
    }
}

fn encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Starts ComfyUI hidden in the background (and stops it again if we started it).
#[derive(Default)]
pub struct Launcher {
    child: Option<Child>,
}

impl Launcher {
    /// We started ComfyUI and it is still running.
    pub fn is_ours(&mut self) -> bool {
        match &mut self.child {
            Some(c) => matches!(c.try_wait(), Ok(None)),
            None => false,
        }
    }

    /// Start ComfyUI from its install folder and wait until it answers.
    pub fn start(&mut self, settings: &EngineSettings, cancel: &AtomicBool) -> anyhow::Result<()> {
        let comfy = Comfy::new(settings);
        if comfy.is_up() {
            return Ok(());
        }
        let root = settings
            .comfy_root
            .clone()
            .ok_or_else(|| anyhow!("ComfyUI wasn't found. Set its folder in the generator settings."))?;
        let main_dir = comfy_dir(&root)
            .ok_or_else(|| anyhow!("No ComfyUI\\main.py in {}", root.display()))?;
        let python = python_exe(&root)
            .ok_or_else(|| anyhow!("No python_embeded\\python.exe in {}", root.display()))?;
        let log_path = crate::generated_dir().join("comfyui_log.txt");
        std::fs::create_dir_all(crate::generated_dir())?;
        let log = std::fs::File::create(&log_path)?;
        let mut cmd = Command::new(&python);
        cmd.current_dir(&root)
            .arg("-I")
            .arg("-W")
            .arg("ignore::FutureWarning")
            .arg(main_dir.join("main.py"))
            .args(["--windows-standalone-build", "--disable-auto-launch", "--listen", "127.0.0.1", "--port"])
            .arg(settings.port.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::from(log.try_clone()?))
            .stderr(Stdio::from(log));
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        let child = cmd
            .spawn()
            .with_context(|| format!("Couldn't start {}", python.display()))?;
        self.child = Some(child);
        log::info!("Started ComfyUI from {} (log: {})", root.display(), log_path.display());

        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(300) {
            if cancel.load(Ordering::Relaxed) {
                self.stop();
                bail!("Stopped.");
            }
            if comfy.is_up() {
                return Ok(());
            }
            if let Some(c) = &mut self.child {
                if let Ok(Some(status)) = c.try_wait() {
                    self.child = None;
                    bail!(
                        "ComfyUI closed while starting ({status}). The end of {} says:\n{}",
                        log_path.display(),
                        log_tail(&log_path)
                    );
                }
            }
            std::thread::sleep(Duration::from_millis(500));
        }
        bail!("ComfyUI took over 5 minutes to start. See {}", log_path.display())
    }

    pub fn stop(&mut self) {
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

impl Drop for Launcher {
    fn drop(&mut self) {
        self.stop();
    }
}

pub(crate) fn log_tail(path: &Path) -> String {
    let mut text = String::new();
    if let Ok(mut f) = std::fs::File::open(path) {
        let _ = f.read_to_string(&mut text);
    }
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(12)..].join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fills_blanks_and_drops_notes() {
        let t: Value = serde_json::json!({
            "_threadsmith": {"about": "x"},
            "1": {"class_type": "A", "inputs": {"text": "$PROMPT", "n": "$COUNT", "keep": "$5 shirt"}}
        });
        let vars: HashMap<String, Value> = [
            ("PROMPT".to_string(), Value::from("hello")),
            ("COUNT".to_string(), Value::from(3)),
            ("5 shirt".to_string(), Value::from("ok")),
        ]
        .into();
        let w = fill_workflow(&t, &vars).unwrap();
        assert!(w.get("_threadsmith").is_none());
        assert_eq!(w["1"]["inputs"]["text"], "hello");
        assert_eq!(w["1"]["inputs"]["n"], 3);
        let err = fill_workflow(&t, &HashMap::new()).unwrap_err();
        assert!(err.to_string().contains("$PROMPT"));
    }

    #[test]
    fn real_workflows_have_known_blanks() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ai/workflows");
        for f in ["image_from_text.json", "image_from_image.json", "garment_3d.json", "garment_3d_fast.json"] {
            let t: Value = serde_json::from_str(&std::fs::read_to_string(dir.join(f)).unwrap()).unwrap();
            let vars = crate::jobs::test_vars();
            let w = fill_workflow(&t, &vars).unwrap_or_else(|e| panic!("{f}: {e}"));
            // Every link points at a node that exists.
            for (id, node) in w.as_object().unwrap() {
                for (name, v) in node["inputs"].as_object().unwrap() {
                    if let Some(arr) = v.as_array() {
                        if arr.len() == 2 && arr[0].is_string() && arr[1].is_u64() {
                            let target = arr[0].as_str().unwrap();
                            assert!(w.get(target).is_some(), "{f}: node {id}.{name} points at missing node {target}");
                        }
                    }
                }
            }
        }
    }
}
