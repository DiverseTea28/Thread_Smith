//! The Unreal bridge: ThreadSmith and the ThreadSmith Bridge plugin talk over
//! local HTTP on this PC only (127.0.0.1, never the network).
//!
//! - ThreadSmith listens on port [`APP_PORT`]. The plugin sends it bodies.
//! - The plugin listens on port [`PLUGIN_PORT`]. ThreadSmith sends it outfits.
//!
//! Full message list: `docs/bridge-protocol.md`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};

pub const APP_PORT: u16 = 47830;
pub const PLUGIN_PORT: u16 = 47831;
pub const PROTOCOL: u32 = 1;

/// Sent by the plugin: "open this body".
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct BodyRequest {
    /// Exported FBX on disk.
    pub file: String,
    /// Character name (e.g. "Ryan").
    #[serde(default)]
    pub character: String,
    /// Unreal path of the body skeleton the outfit must be skinned to.
    #[serde(default)]
    pub skeleton_asset: Option<String>,
    /// Unreal path of the body skeletal mesh.
    #[serde(default)]
    pub body_asset: Option<String>,
    /// Exported head / face FBX, if the character has one (shown, not fitted to).
    #[serde(default)]
    pub head_file: Option<String>,
}

/// Sent to the plugin: "import this outfit".
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct OutfitMessage {
    pub outfit_dir: String,
    pub mesh_file: String,
    pub name: String,
    pub category: String,
    pub character: String,
    pub skeleton_asset: Option<String>,
    pub body_asset: Option<String>,
    /// Add it to the character's Blueprint / selected actor after importing.
    pub apply_to_character: bool,
}

/// What the bridge reports to the app.
pub enum BridgeEvent {
    LoadBody(BodyRequest),
    /// Result of sending an outfit to Unreal: Ok(imported asset path) or Err(message).
    OutfitSent(Result<String, String>),
    /// The local server could not start (port busy?).
    ServerError(String),
}

pub struct Bridge {
    pub events: Receiver<BridgeEvent>,
    sender: Sender<BridgeEvent>,
    plugin_connected: Arc<AtomicBool>,
}

impl Bridge {
    /// Start the local server and the plugin watcher. `wake` asks the UI to redraw.
    pub fn start(wake: impl Fn() + Send + Sync + Clone + 'static) -> Self {
        let (sender, events) = mpsc::channel();
        let plugin_connected = Arc::new(AtomicBool::new(false));
        let server_running = Arc::new(AtomicBool::new(false));

        // ---- Server: receives requests from the plugin ----
        {
            let tx = sender.clone();
            let running = server_running.clone();
            let wake = wake.clone();
            std::thread::Builder::new()
                .name("bridge-server".into())
                .spawn(move || serve(tx, running, wake))
                .expect("thread");
        }
        // ---- Watcher: is the plugin there? ----
        {
            let connected = plugin_connected.clone();
            let wake = wake.clone();
            std::thread::Builder::new()
                .name("bridge-watch".into())
                .spawn(move || loop {
                    let now = plugin_status().is_ok();
                    if connected.swap(now, Ordering::Relaxed) != now {
                        wake();
                    }
                    std::thread::sleep(Duration::from_secs(3));
                })
                .expect("thread");
        }
        Self {
            events,
            sender,
            plugin_connected,
        }
    }

    pub fn plugin_connected(&self) -> bool {
        self.plugin_connected.load(Ordering::Relaxed)
    }

    /// Send an outfit to the plugin in the background; the answer arrives as
    /// [`BridgeEvent::OutfitSent`].
    pub fn send_outfit(&self, msg: OutfitMessage, wake: impl Fn() + Send + 'static) {
        let tx = self.sender.clone();
        std::thread::spawn(move || {
            let result = post_outfit(&msg);
            let _ = tx.send(BridgeEvent::OutfitSent(result));
            wake();
        });
    }
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(120)))
        .http_status_as_error(false)
        .build()
        .into()
}

fn plugin_status() -> Result<(), String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_millis(800)))
        .build()
        .into();
    agent
        .get(&format!("http://127.0.0.1:{PLUGIN_PORT}/status"))
        .call()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

fn post_outfit(msg: &OutfitMessage) -> Result<String, String> {
    let body = serde_json::to_string(msg).map_err(|e| e.to_string())?;
    let mut resp = agent()
        .post(&format!("http://127.0.0.1:{PLUGIN_PORT}/outfit"))
        .header("Content-Type", "application/json")
        .send(body.as_str())
        .map_err(|e| format!("Unreal didn't answer. Is the editor open with the ThreadSmith Bridge plugin on? ({e})"))?;
    let status = resp.status().as_u16();
    let text = resp.body_mut().read_to_string().unwrap_or_default();
    let json: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
    if status == 200 && json["ok"].as_bool() == Some(true) {
        Ok(json["asset"].as_str().unwrap_or("").to_string())
    } else {
        Err(json["error"]
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| format!("Unreal answered {status}: {text}")))
    }
}

fn serve(tx: Sender<BridgeEvent>, running: Arc<AtomicBool>, wake: impl Fn()) {
    let server = match tiny_http::Server::http(("127.0.0.1", APP_PORT)) {
        Ok(s) => s,
        Err(e) => {
            let _ = tx.send(BridgeEvent::ServerError(format!(
                "Couldn't start the Unreal bridge on port {APP_PORT} (is another ThreadSmith open?): {e}"
            )));
            wake();
            return;
        }
    };
    running.store(true, Ordering::Relaxed);
    log::info!("Unreal bridge listening on 127.0.0.1:{APP_PORT}");
    for mut req in server.incoming_requests() {
        let method = req.method().as_str().to_string();
        let url = req.url().to_string();
        let (code, reply) = match (method.as_str(), url.as_str()) {
            ("GET", "/status") => (
                200,
                serde_json::json!({ "ok": true, "app": "ThreadSmith", "version": env!("CARGO_PKG_VERSION"), "protocol": PROTOCOL }),
            ),
            ("POST", "/body") => {
                let mut text = String::new();
                let _ = req.as_reader().read_to_string(&mut text);
                match serde_json::from_str::<BodyRequest>(&text) {
                    Ok(b) if std::path::Path::new(&b.file).exists() => {
                        let _ = tx.send(BridgeEvent::LoadBody(b));
                        wake();
                        (200, serde_json::json!({ "ok": true }))
                    }
                    Ok(b) => (
                        404,
                        serde_json::json!({ "ok": false, "error": format!("File not found: {}", b.file) }),
                    ),
                    Err(e) => (
                        400,
                        serde_json::json!({ "ok": false, "error": format!("Bad request: {e}") }),
                    ),
                }
            }
            _ => (
                404,
                serde_json::json!({ "ok": false, "error": "Unknown request" }),
            ),
        };
        let header =
            tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap();
        let _ = req.respond(
            tiny_http::Response::from_string(reply.to_string())
                .with_status_code(code)
                .with_header(header),
        );
    }
}
