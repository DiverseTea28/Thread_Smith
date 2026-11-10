//! The ThreadSmith Engine: a small Python helper (`ai/engine/threadsmith_engine.py`)
//! that runs Pixal3D and sends live previews while a model is being made.
//!
//! ThreadSmith opens a port on 127.0.0.1, starts the engine hidden with the
//! Python from the ComfyUI install, and the engine connects back. The engine
//! stays running between jobs so the model only loads once, and it quits by
//! itself when ThreadSmith closes the connection.
//!
//! Each message (both ways) is `u32 header length | JSON header | u32 data
//! length | data`, lengths little-endian. See the Python file for the list.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context};
use serde_json::Value;

use crate::comfy::{log_tail, python_exe, EngineSettings};

/// The engine protocol version this app speaks.
pub const PROTOCOL: u64 = 1;

/// One message from the engine.
#[derive(Debug, Clone)]
pub struct Message {
    pub header: Value,
    pub data: Vec<u8>,
}

impl Message {
    pub fn kind(&self) -> &str {
        self.header["type"].as_str().unwrap_or("")
    }
}

/// A live preview of the model being made, in the same space as the final
/// GLB (Y up, about -0.5..0.5).
#[derive(Debug, Clone)]
pub enum LivePreview {
    /// The rough shape: a cloud of cubes (centres, and the cube size).
    Voxels { centres: Vec<[f32; 3]>, size: f32 },
    /// A grey mesh (no texture yet).
    Mesh { vertices: Vec<[f32; 3]>, indices: Vec<u32> },
}

impl LivePreview {
    /// Read a `preview_voxels` or `preview_mesh` message.
    pub fn from_message(m: &Message) -> Option<Self> {
        match m.kind() {
            "preview_voxels" => {
                let count = m.header["count"].as_u64()? as usize;
                let size = m.header["size"].as_f64().unwrap_or(1.0 / 32.0) as f32;
                let centres = read_vec3(&m.data, count)?;
                Some(LivePreview::Voxels { centres, size })
            }
            "preview_mesh" => {
                let nv = m.header["vertices"].as_u64()? as usize;
                let nf = m.header["faces"].as_u64()? as usize;
                let vertices = read_vec3(&m.data, nv)?;
                let rest = m.data.get(nv * 12..)?;
                if rest.len() < nf * 12 {
                    return None;
                }
                let indices: Vec<u32> = rest[..nf * 12]
                    .chunks_exact(4)
                    .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                    .collect();
                if indices.iter().any(|&i| i as usize >= nv) {
                    return None;
                }
                Some(LivePreview::Mesh { vertices, indices })
            }
            _ => None,
        }
    }
}

fn read_vec3(data: &[u8], count: usize) -> Option<Vec<[f32; 3]>> {
    let bytes = data.get(..count * 12)?;
    Some(
        bytes
            .chunks_exact(12)
            .map(|c| {
                let f = |i: usize| f32::from_le_bytes([c[i], c[i + 1], c[i + 2], c[i + 3]]);
                [f(0), f(4), f(8)]
            })
            .collect(),
    )
}

/// Write one message.
pub fn write_message(w: &mut impl Write, header: &Value, data: &[u8]) -> std::io::Result<()> {
    let head = serde_json::to_vec(header)?;
    let mut packet = Vec::with_capacity(8 + head.len() + data.len());
    packet.extend_from_slice(&(head.len() as u32).to_le_bytes());
    packet.extend_from_slice(&head);
    packet.extend_from_slice(&(data.len() as u32).to_le_bytes());
    packet.extend_from_slice(data);
    w.write_all(&packet)?;
    w.flush()
}

/// Read one message (blocking).
pub fn read_message(r: &mut impl Read) -> std::io::Result<Message> {
    let mut len = [0u8; 4];
    r.read_exact(&mut len)?;
    let head_len = u32::from_le_bytes(len) as usize;
    if head_len > 16 << 20 {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "message header too big"));
    }
    let mut head = vec![0u8; head_len];
    r.read_exact(&mut head)?;
    let header: Value = serde_json::from_slice(&head)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    r.read_exact(&mut len)?;
    let data_len = u32::from_le_bytes(len) as usize;
    if data_len > 1 << 30 {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "message data too big"));
    }
    let mut data = vec![0u8; data_len];
    r.read_exact(&mut data)?;
    Ok(Message { header, data })
}

/// The running engine (if any). Shared between jobs so the model stays loaded.
#[derive(Default)]
pub struct EngineHost {
    child: Option<Child>,
    writer: Option<TcpStream>,
    rx: Option<Receiver<Message>>,
    /// The engine's `hello` message.
    pub hello: Option<Value>,
}

impl EngineHost {
    /// Engine log written by the Python side.
    pub fn log_path() -> PathBuf {
        crate::generated_dir().join("engine_log.txt")
    }

    /// Anything Python printed itself (crashes before logging starts).
    pub fn console_path() -> PathBuf {
        crate::generated_dir().join("engine_console.txt")
    }

    pub fn is_running(&mut self) -> bool {
        let alive = match &mut self.child {
            Some(c) => matches!(c.try_wait(), Ok(None)),
            None => false,
        };
        if !alive {
            self.child = None;
            self.writer = None;
            self.rx = None;
            self.hello = None;
        }
        alive && self.writer.is_some()
    }

    /// Start the engine if it isn't running, and wait for its hello.
    pub fn start(&mut self, settings: &EngineSettings, ai_dir: &Path, cancel: &AtomicBool) -> anyhow::Result<()> {
        if self.is_running() {
            return Ok(());
        }
        let script = ai_dir.join("engine").join("threadsmith_engine.py");
        if !script.is_file() {
            bail!("The engine script is missing: {}", script.display());
        }
        let python = match std::env::var_os("THREADSMITH_ENGINE_PYTHON") {
            Some(p) => PathBuf::from(p),
            None => {
                let root = settings
                    .comfy_root
                    .as_ref()
                    .ok_or_else(|| anyhow!("ComfyUI wasn't found. Set its folder in the generator settings (the engine uses its Python)."))?;
                python_exe(root).ok_or_else(|| anyhow!("No python_embeded\\python.exe in {}", root.display()))?
            }
        };

        let listener = TcpListener::bind("127.0.0.1:0").context("Couldn't open a local port for the engine")?;
        listener.set_nonblocking(true)?;
        let port = listener.local_addr()?.port();

        std::fs::create_dir_all(crate::generated_dir())?;
        let console = std::fs::File::create(Self::console_path())?;
        let mut cmd = Command::new(&python);
        cmd.arg("-I")
            .arg("-u")
            .arg(&script)
            .arg("--port")
            .arg(port.to_string())
            .arg("--log")
            .arg(Self::log_path());
        if let Some(root) = &settings.comfy_root {
            cmd.arg("--comfy-root").arg(root);
        }
        cmd.stdin(Stdio::null())
            .stdout(Stdio::from(console.try_clone()?))
            .stderr(Stdio::from(console));
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        let mut child = cmd.spawn().with_context(|| format!("Couldn't start {}", python.display()))?;
        log::info!("Started the ThreadSmith Engine with {} on port {port}", python.display());

        // Wait for it to connect.
        let started = Instant::now();
        let stream = loop {
            if cancel.load(Ordering::Relaxed) {
                let _ = child.kill();
                bail!("Stopped.");
            }
            match listener.accept() {
                Ok((s, _)) => break s,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(e) => {
                    let _ = child.kill();
                    return Err(e).context("The engine couldn't connect");
                }
            }
            if let Ok(Some(status)) = child.try_wait() {
                bail!(
                    "The engine closed while starting ({status}). The end of {} says:\n{}",
                    Self::console_path().display(),
                    log_tail(&Self::console_path())
                );
            }
            if started.elapsed() > Duration::from_secs(120) {
                let _ = child.kill();
                bail!("The engine took over 2 minutes to start. See {}", Self::console_path().display());
            }
            std::thread::sleep(Duration::from_millis(50));
        };
        stream.set_nonblocking(false)?;
        let _ = stream.set_nodelay(true);
        let mut reader = stream.try_clone()?;
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            while let Ok(m) = read_message(&mut reader) {
                if tx.send(m).is_err() {
                    break;
                }
            }
        });
        self.child = Some(child);
        self.writer = Some(stream);
        self.rx = Some(rx);

        let hello = self.recv_timeout(Duration::from_secs(60))?;
        if hello.kind() != "hello" {
            bail!("The engine said {} instead of hello", hello.header);
        }
        let protocol = hello.header["protocol"].as_u64().unwrap_or(0);
        if protocol != PROTOCOL {
            self.stop();
            bail!("The engine script speaks version {protocol}, this ThreadSmith speaks {PROTOCOL}. Update ThreadSmith's ai folder.");
        }
        self.hello = Some(hello.header);
        Ok(())
    }

    /// Send a request.
    pub fn send(&mut self, header: &Value) -> anyhow::Result<()> {
        let w = self.writer.as_mut().ok_or_else(|| anyhow!("The engine isn't running"))?;
        write_message(w, header, &[]).context("Couldn't talk to the engine")
    }

    fn recv_timeout(&mut self, timeout: Duration) -> anyhow::Result<Message> {
        let rx = self.rx.as_ref().ok_or_else(|| anyhow!("The engine isn't running"))?;
        match rx.recv_timeout(timeout) {
            Ok(m) => Ok(m),
            Err(RecvTimeoutError::Timeout) => bail!("The engine didn't answer in time"),
            Err(RecvTimeoutError::Disconnected) => Err(self.closed_error()),
        }
    }

    /// Wait for the next message. When `cancel` turns on, tells the engine to
    /// stop (once) and keeps waiting for its answer.
    pub fn recv(&mut self, cancel: &AtomicBool) -> anyhow::Result<Message> {
        let mut told = false;
        let mut since_cancel: Option<Instant> = None;
        loop {
            if cancel.load(Ordering::Relaxed) && !told {
                told = true;
                since_cancel = Some(Instant::now());
                let _ = self.send(&serde_json::json!({"type": "cancel"}));
            }
            // A stuck engine (e.g. inside a long GPU call) is stopped hard
            // if it hasn't answered a minute after Stop.
            if since_cancel.is_some_and(|t| t.elapsed() > Duration::from_secs(60)) {
                self.stop();
                bail!("Stopped (the engine was restarted).");
            }
            let rx = self.rx.as_ref().ok_or_else(|| anyhow!("The engine isn't running"))?;
            match rx.recv_timeout(Duration::from_millis(200)) {
                Ok(m) => return Ok(m),
                Err(RecvTimeoutError::Timeout) => {
                    if !self.is_running() {
                        return Err(self.closed_error());
                    }
                }
                Err(RecvTimeoutError::Disconnected) => return Err(self.closed_error()),
            }
        }
    }

    fn closed_error(&mut self) -> anyhow::Error {
        self.stop();
        anyhow!(
            "The engine closed unexpectedly. The end of {} says:\n{}",
            Self::log_path().display(),
            {
                let a = log_tail(&Self::log_path());
                if a.trim().is_empty() { log_tail(&Self::console_path()) } else { a }
            }
        )
    }

    /// Ask the engine to quit (and make sure it does).
    pub fn stop(&mut self) {
        if let Some(w) = self.writer.as_mut() {
            let _ = write_message(w, &serde_json::json!({"type": "shutdown"}), &[]);
        }
        self.writer = None;
        self.rx = None;
        self.hello = None;
        if let Some(mut c) = self.child.take() {
            let started = Instant::now();
            while started.elapsed() < Duration::from_secs(3) {
                if let Ok(Some(_)) = c.try_wait() {
                    return;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

impl Drop for EngineHost {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_round_trip() {
        let mut buf = Vec::new();
        write_message(&mut buf, &serde_json::json!({"type": "progress", "step": 3}), b"abc").unwrap();
        let m = read_message(&mut buf.as_slice()).unwrap();
        assert_eq!(m.kind(), "progress");
        assert_eq!(m.header["step"], 3);
        assert_eq!(m.data, b"abc");
    }

    #[test]
    fn reads_previews() {
        let mut data = Vec::new();
        for f in [0.0f32, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0] {
            data.extend_from_slice(&f.to_le_bytes());
        }
        for i in [0u32, 1, 2] {
            data.extend_from_slice(&i.to_le_bytes());
        }
        let m = Message { header: serde_json::json!({"type": "preview_mesh", "vertices": 3, "faces": 1}), data: data.clone() };
        match LivePreview::from_message(&m) {
            Some(LivePreview::Mesh { vertices, indices }) => {
                assert_eq!(vertices[2], [6.0, 7.0, 8.0]);
                assert_eq!(indices, vec![0, 1, 2]);
            }
            other => panic!("{other:?}"),
        }
        let v = Message { header: serde_json::json!({"type": "preview_voxels", "count": 2, "size": 0.5}), data: data[..24].to_vec() };
        match LivePreview::from_message(&v) {
            Some(LivePreview::Voxels { centres, size }) => {
                assert_eq!(centres.len(), 2);
                assert_eq!(size, 0.5);
            }
            other => panic!("{other:?}"),
        }
        // Bad index: refused.
        let mut bad = data.clone();
        bad[36..40].copy_from_slice(&9u32.to_le_bytes());
        let m = Message { header: serde_json::json!({"type": "preview_mesh", "vertices": 3, "faces": 1}), data: bad };
        assert!(LivePreview::from_message(&m).is_none());
    }
}
