//! The "Generate clothing" window: pick a garment type, describe it (text,
//! picture or both), pick one of the pictures, make it 3D.
//!
//! The AI work happens in a local ComfyUI driven by `ts_gen`; this file is only
//! the window and its state.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use egui::{vec2, Align2, Color32, FontId, RichText, Sense, StrokeKind, TextureHandle};
use ts_gen::jobs::EngineReport;
use ts_gen::models::ModelList;
use ts_gen::{EngineSettings, Garment, GenEvent, GenJob, GenRequest, Launcher, ModelSet, Presets, Quality};

/// Key for saving the engine settings between runs.
pub const SETTINGS_KEY: &str = "threadsmith_generator";

/// What the window hands back to the app.
pub enum GenOutput {
    /// A finished 3D garment to add to the scene.
    Mesh { path: PathBuf, garment: Garment, prompt: String },
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum JobKind {
    Check,
    Images,
    Mesh,
    Download,
}

struct Picture {
    path: PathBuf,
    texture: Option<TextureHandle>,
}

pub struct GeneratePanel {
    pub settings: EngineSettings,
    ai_dir: Option<PathBuf>,
    presets: Result<Presets, String>,
    models: Vec<ModelSet>,
    icons: HashMap<String, TextureHandle>,
    launcher: Arc<Mutex<Launcher>>,

    selected: usize,
    prompt: String,
    reference: Option<Picture>,
    versions: u32,
    quality: Quality,
    pictures: Vec<Picture>,
    picked: Option<usize>,

    job: Option<(JobKind, GenJob)>,
    progress: Option<(String, f32)>,
    /// (is_error, text)
    message: Option<(bool, String)>,
    report: Option<EngineReport>,
    show_settings: bool,
    show_prompt: bool,
    /// When the running job started (for the timer).
    job_started: Option<Instant>,
}

impl GeneratePanel {
    pub fn new(ctx: &egui::Context, settings: EngineSettings) -> Self {
        let ai_dir = ts_gen::ai_dir();
        let presets = match &ai_dir {
            Some(d) => Presets::load(&d.join("presets").join("garment_presets.json")).map_err(|e| format!("{e:#}")),
            None => Err("The ai folder (garment presets and workflows) wasn't found next to ThreadSmith.".into()),
        };
        let models = ai_dir
            .as_ref()
            .and_then(|d| ModelList::load(&d.join("models.json")).ok())
            .map(|l| l.sets)
            .unwrap_or_default();
        let mut icons = HashMap::new();
        if let (Some(dir), Ok(p)) = (&ai_dir, &presets) {
            for g in &p.garments {
                let path = dir.join("presets").join("icons").join(format!("{}.png", g.icon));
                if let Some(t) = load_texture(ctx, &path, 64) {
                    icons.insert(g.icon.clone(), t);
                }
            }
        }
        let shirts = presets
            .as_ref()
            .ok()
            .and_then(|p| p.garments.iter().position(|g| g.id == "shirts"))
            .unwrap_or(0);
        Self {
            settings,
            ai_dir,
            presets,
            models,
            icons,
            launcher: Arc::new(Mutex::new(Launcher::default())),
            selected: shirts,
            prompt: String::new(),
            reference: None,
            versions: 4,
            quality: Quality::Balanced,
            pictures: Vec::new(),
            picked: None,
            job: None,
            progress: None,
            message: None,
            report: None,
            show_settings: false,
            show_prompt: false,
            job_started: None,
        }
    }

    pub fn busy(&self) -> bool {
        self.job.is_some()
    }

    /// Use a picture dropped onto the window as the reference.
    pub fn set_reference(&mut self, ctx: &egui::Context, path: PathBuf) {
        let texture = load_texture(ctx, &path, 256);
        self.reference = Some(Picture { path, texture });
        self.message = None;
    }

    pub fn is_picture(path: &Path) -> bool {
        path.extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .is_some_and(|e| matches!(e.as_str(), "png" | "jpg" | "jpeg" | "webp"))
    }

    fn garment(&self) -> Option<&Garment> {
        self.presets.as_ref().ok()?.garments.get(self.selected)
    }

    fn start(&mut self, ctx: &egui::Context, kind: JobKind, request: GenRequest) {
        let Some(ai_dir) = self.ai_dir.clone() else {
            self.message = Some((true, "The ai folder wasn't found.".into()));
            return;
        };
        let wake = ctx.clone();
        let job = GenJob::start(
            self.settings.clone(),
            self.launcher.clone(),
            ai_dir,
            self.models.clone(),
            request,
            move || wake.request_repaint(),
        );
        self.job = Some((kind, job));
        self.job_started = Some(Instant::now());
        self.progress = Some(("Starting…".into(), 0.0));
        self.message = None;
    }

    /// Read job events. Returns a finished garment, if one arrived.
    pub fn poll(&mut self, ctx: &egui::Context) -> Option<GenOutput> {
        let mut out = None;
        let mut finished = None;
        if let Some((kind, job)) = &self.job {
            let kind = *kind;
            while let Ok(ev) = job.rx.try_recv() {
                match ev {
                    GenEvent::Status(s) => self.message = Some((false, s)),
                    GenEvent::Progress { label, fraction } => self.progress = Some((label, fraction)),
                    GenEvent::Image(path) => {
                        let texture = load_texture(ctx, &path, 256);
                        self.pictures.push(Picture { path, texture });
                        if self.picked.is_none() {
                            self.picked = Some(self.pictures.len() - 1);
                        }
                    }
                    GenEvent::Mesh(path) => {
                        if let Some(g) = self.garment().cloned() {
                            out = Some(GenOutput::Mesh { path, garment: g, prompt: self.prompt.clone() });
                        }
                    }
                    GenEvent::Report(r) => self.report = Some(r),
                    GenEvent::Finished(result) => finished = Some((kind, result)),
                }
            }
        }
        if let Some((kind, result)) = finished {
            self.job = None;
            self.progress = None;
            let took = self.job_started.take().map(|t| t.elapsed()).unwrap_or_default();
            let took_text = format!(" (took {})", clock(took));
            match result {
                Ok(()) => {
                    match kind {
                        JobKind::Mesh => {
                            self.message = Some((false, format!("Done{took_text}. The garment was added to the scene.")));
                        }
                        JobKind::Images => {
                            if let Some((false, m)) = &mut self.message {
                                m.push_str(&took_text);
                            }
                        }
                        _ => {}
                    }
                    if kind == JobKind::Download {
                        self.start(ctx, JobKind::Check, GenRequest::Check);
                    }
                }
                Err(e) => self.message = Some((true, e)),
            }
        }
        out
    }

    /// Stop ComfyUI if ThreadSmith started it (when the app closes).
    pub fn shutdown(&mut self) {
        if let Some((_, job)) = &self.job {
            job.cancel();
        }
        if let Ok(mut l) = self.launcher.lock() {
            l.stop();
        }
    }

    /// The whole Generate tab (shown in the side panel).
    pub fn panel(&mut self, ui: &mut egui::Ui, has_body: bool) {
        ui.heading("Generate");
        ui.add_space(4.0);
        let ctx = ui.ctx().clone();
        let presets = match &self.presets {
            Ok(p) => p.clone(),
            Err(e) => {
                ui.colored_label(Color32::from_rgb(240, 120, 110), e);
                return;
            }
        };
        let busy = self.busy();

        // ---- Engine line ----
        ui.horizontal(|ui| {
            let (dot, text) = match (&self.report, busy) {
                (_, true) => (Color32::from_rgb(230, 190, 90), "AI engine: working"),
                (Some(r), _) if r.running && r.trellis_nodes && r.missing_models.is_empty() => {
                    (Color32::from_rgb(120, 210, 120), "AI engine: ready")
                }
                (Some(_), _) => (Color32::from_rgb(230, 150, 80), "AI engine: needs setup"),
                (None, _) => (Color32::GRAY, "AI engine: not checked yet"),
            };
            status_dot(ui, dot);
            ui.label(text);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.toggle_value(&mut self.show_settings, "Settings");
            });
        });
        if self.show_settings {
            egui::Frame::group(ui.style()).show(ui, |ui| self.settings_ui(ui, busy));
        }
        ui.add_space(6.0);

        // ---- 1. Type ----
        ui.label(RichText::new("1. What do you want to make?").strong());
        let cell = vec2(78.0, 70.0);
        let per_row = ((ui.available_width() / (cell.x + 4.0)).floor() as usize).max(1);
        for row in presets.garments.chunks(per_row).enumerate() {
            ui.horizontal(|ui| {
                for (j, g) in row.1.iter().enumerate() {
                    let index = row.0 * per_row + j;
                    let (rect, resp) = ui.allocate_exact_size(cell, Sense::click());
                    let selected = index == self.selected;
                    let painter = ui.painter_at(rect);
                    if selected {
                        painter.rect_filled(rect, 6.0, Color32::from_rgb(34, 52, 72));
                        painter.rect_stroke(rect, 6.0, (1.0, Color32::from_rgb(70, 140, 220)), StrokeKind::Inside);
                    } else if resp.hovered() {
                        painter.rect_filled(rect, 6.0, Color32::from_gray(45));
                    }
                    let tint = if selected { Color32::from_rgb(110, 175, 255) } else { Color32::from_gray(150) };
                    if let Some(tex) = self.icons.get(&g.icon) {
                        let icon = egui::Rect::from_center_size(rect.center() - vec2(0.0, 9.0), vec2(30.0, 30.0));
                        painter.image(tex.id(), icon, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), tint);
                    }
                    painter.text(
                        rect.center_bottom() - vec2(0.0, 6.0),
                        Align2::CENTER_BOTTOM,
                        &g.label,
                        FontId::proportional(12.5),
                        if selected { Color32::WHITE } else { Color32::from_gray(190) },
                    );
                    if resp.clicked() && !busy {
                        self.selected = index;
                    }
                }
            });
        }
        ui.add_space(6.0);

        // ---- 2. Describe ----
        ui.label(RichText::new("2. Describe it (text, picture or both)").strong());
        ui.add(
            egui::TextEdit::multiline(&mut self.prompt)
                .desired_rows(2)
                .desired_width(f32::INFINITY)
                .hint_text("worn brown leather, fleece collar"),
        );
        ui.horizontal(|ui| {
            if ui.add_enabled(!busy, egui::Button::new("Choose picture…")).clicked() {
                if let Some(p) = rfd::FileDialog::new()
                    .set_title("Pick a reference picture")
                    .add_filter("Pictures", &["png", "jpg", "jpeg", "webp"])
                    .pick_file()
                {
                    self.set_reference(&ctx, p);
                }
            }
            if self.reference.is_some() && ui.add_enabled(!busy, egui::Button::new("Remove picture")).clicked() {
                self.reference = None;
            }
        });
        match &self.reference {
            Some(r) => {
                if let Some(t) = &r.texture {
                    ui.add(egui::Image::from_texture(t).fit_to_exact_size(vec2(96.0, 96.0)));
                }
            }
            None => {
                ui.label(RichText::new("Or drop a picture onto the window.").weak().small());
            }
        }
        let garment = presets.garments[self.selected].clone();
        let full_prompt = presets.prompt(&garment, &self.prompt, self.reference.is_some());
        if self.show_prompt {
            ui.label(RichText::new(&full_prompt).small().weak());
        }
        ui.horizontal(|ui| {
            ui.label("Versions");
            egui::ComboBox::from_id_salt("versions")
                .selected_text(self.versions.to_string())
                .width(40.0)
                .show_ui(ui, |ui| {
                    for n in 1..=4 {
                        ui.selectable_value(&mut self.versions, n, n.to_string());
                    }
                });
            let can = !busy && (!self.prompt.trim().is_empty() || self.reference.is_some());
            if ui
                .add_enabled(can, egui::Button::new(RichText::new("Make pictures").strong()))
                .on_disabled_hover_text("Type a description or choose a picture first.")
                .clicked()
            {
                self.pictures.clear();
                self.picked = None;
                let request = GenRequest::Images {
                    prompt: full_prompt.clone(),
                    reference: self.reference.as_ref().map(|r| r.path.clone()),
                    count: self.versions,
                    seed: ts_gen::random_seed(),
                };
                self.start(&ctx, JobKind::Images, request);
            }
        });
        ui.add_space(6.0);

        // ---- 3. Pick ----
        ui.label(RichText::new("3. Pick one and make it 3D").strong());
        if self.pictures.is_empty() {
            ui.label(RichText::new("Your pictures show up here.").weak());
        } else {
            let size = 84.0;
            ui.horizontal_wrapped(|ui| {
                for (i, p) in self.pictures.iter().enumerate() {
                    let (rect, resp) = ui.allocate_exact_size(vec2(size, size), Sense::click());
                    let painter = ui.painter_at(rect);
                    if let Some(t) = &p.texture {
                        painter.image(t.id(), rect.shrink(3.0), egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
                    }
                    if self.picked == Some(i) {
                        painter.rect_stroke(rect, 4.0, (2.5, Color32::from_rgb(80, 160, 255)), StrokeKind::Inside);
                    }
                    if resp.clicked() && !busy {
                        self.picked = Some(i);
                    }
                    resp.on_hover_text("Click to pick. Double-click to open it.").double_clicked().then(|| open_file(&p.path));
                }
            });
        }
        ui.horizontal(|ui| {
            ui.label("Quality");
            for q in Quality::ALL {
                ui.selectable_value(&mut self.quality, q, q.label()).on_hover_text(q.hint());
            }
        });
        let source = self.picked.and_then(|i| self.pictures.get(i)).map(|p| p.path.clone());
        ui.horizontal(|ui| {
            let can = !busy && has_body && source.is_some();
            if ui
                .add_enabled(can, egui::Button::new(RichText::new("Make 3D").strong()))
                .on_disabled_hover_text(if has_body { "Make and pick a picture first." } else { "Open a body first." })
                .clicked()
            {
                if let Some(image) = source.clone() {
                    self.start(&ctx, JobKind::Mesh, GenRequest::Mesh { image, quality: self.quality, faces: garment.faces, seed: ts_gen::random_seed() });
                }
            }
            if let Some(r) = &self.reference {
                let path = r.path.clone();
                if ui
                    .add_enabled(!busy && has_body, egui::Button::new("Use my picture as is"))
                    .on_hover_text("Skip the picture step and make 3D straight from your picture. Works best on a plain background.")
                    .clicked()
                {
                    self.start(&ctx, JobKind::Mesh, GenRequest::Mesh { image: path, quality: self.quality, faces: garment.faces, seed: ts_gen::random_seed() });
                }
            }
        });

        // ---- Progress ----
        if let Some((label, fraction)) = self.progress.clone() {
            ui.add_space(4.0);
            if let Some(started) = self.job_started {
                let elapsed = started.elapsed();
                let mut timer = format!("Time: {}", clock(elapsed));
                // A guess at what's left, once there's enough to go on. The
                // 3D steps take very different times, so no guess there.
                let guessable = matches!(self.job.as_ref().map(|j| j.0), Some(JobKind::Images | JobKind::Download));
                if guessable && fraction > 0.05 && fraction < 0.999 {
                    let total = elapsed.as_secs_f32() / fraction;
                    timer.push_str(&format!("   ·   about {} left", clock(Duration::from_secs_f32(total - elapsed.as_secs_f32()))));
                }
                ui.label(RichText::new(timer).monospace());
                ui.ctx().request_repaint_after(Duration::from_millis(250));
            }
            ui.add(egui::ProgressBar::new(fraction.clamp(0.0, 1.0)).text(label));
            if ui.button("Stop").clicked() {
                if let Some((_, job)) = &self.job {
                    job.cancel();
                }
            }
        }
        if let Some((is_error, text)) = &self.message {
            let color = if *is_error { Color32::from_rgb(240, 120, 110) } else { Color32::from_rgb(150, 200, 150) };
            ui.colored_label(color, text);
        }
    }

    fn settings_ui(&mut self, ui: &mut egui::Ui, busy: bool) {
        let ctx = ui.ctx().clone();
        ui.label(RichText::new("AI engine (ComfyUI)").strong());
        ui.horizontal(|ui| {
            ui.label("Folder");
            let shown = self
                .settings
                .comfy_root
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "not found".into());
            ui.label(RichText::new(shown).monospace().small());
        });
        ui.horizontal(|ui| {
            if ui.button("Browse…").clicked() {
                if let Some(p) = rfd::FileDialog::new().set_title("ComfyUI install folder").pick_folder() {
                    self.settings.comfy_root = Some(p);
                    self.report = None;
                }
            }
            if ui.button("Find it").on_hover_text("Look in the usual install places.").clicked() {
                self.settings.comfy_root = ts_gen::comfy::detect_comfy_root();
                if self.settings.comfy_root.is_none() {
                    self.message = Some((true, "ComfyUI wasn't found in the usual places. Use Browse… to pick its folder (the one with ComfyUI and python_embeded inside).".into()));
                }
            }
            if ui.button("Open results folder").clicked() {
                let _ = std::fs::create_dir_all(ts_gen::generated_dir());
                open_file(&ts_gen::generated_dir());
            }
        });
        ui.horizontal(|ui| {
            ui.label("Port");
            ui.add(egui::DragValue::new(&mut self.settings.port).range(1024..=65535));
            ui.checkbox(&mut self.settings.auto_start, "Start ComfyUI for me");
        });
        ui.horizontal(|ui| {
            ui.label("3D attention");
            egui::ComboBox::from_id_salt("attention")
                .selected_text(self.settings.attention.clone())
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.settings.attention, "flash_attn".to_string(), "flash_attn (fast)");
                    ui.selectable_value(&mut self.settings.attention, "sdpa".to_string(), "sdpa (works everywhere)");
                });
        });
        ui.checkbox(&mut self.show_prompt, "Show the hidden prompt");
        if ui.add_enabled(!busy, egui::Button::new("Check engine")).on_hover_text("Starts ComfyUI if needed and checks the TRELLIS.2 nodes and models.").clicked() {
            self.start(&ctx, JobKind::Check, GenRequest::Check);
        }

        ui.add_space(4.0);
        ui.label(RichText::new("Models").strong());
        let models_dir = self.settings.models_dir();
        let mut download = None;
        for set in &self.models {
            let ready = models_dir.as_ref().is_some_and(|d| set.is_ready(d));
            ui.horizontal(|ui| {
                status_dot(ui, if ready { Color32::from_rgb(120, 210, 120) } else { Color32::from_rgb(230, 150, 80) });
                ui.label(&set.name).on_hover_text(format!("License: {}", set.license));
                if !ready {
                    let size = set.size_gb.map(|g| format!(" (~{g:.0} GB)")).unwrap_or_default();
                    if ui.add_enabled(!busy && models_dir.is_some(), egui::Button::new(format!("Download{size}"))).clicked() {
                        download = Some(set.clone());
                    }
                }
            });
        }
        if let Some(set) = download {
            self.start(&ctx, JobKind::Download, GenRequest::Download { sets: vec![set] });
        }
        if let Some(r) = &self.report {
            if !r.trellis_nodes {
                ui.colored_label(
                    Color32::from_rgb(230, 150, 80),
                    "TRELLIS.2 nodes are missing. In ComfyUI-Easy-Install run Add-Ons\\Trellis2 (requires Torch 2.8.0+cu128).bat.",
                );
            }
        }
    }
}

/// Load a picture as a GPU texture, shrunk to at most `max` pixels across.
fn load_texture(ctx: &egui::Context, path: &Path, max: u32) -> Option<TextureHandle> {
    let img = image::open(path).ok()?;
    let img = if img.width() > max || img.height() > max {
        img.thumbnail(max, max)
    } else {
        img
    };
    let rgba = img.to_rgba8();
    let size = [rgba.width() as usize, rgba.height() as usize];
    let color = egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
    Some(ctx.load_texture(path.display().to_string(), color, egui::TextureOptions::LINEAR))
}

fn open_file(path: &Path) {
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("explorer").arg(path).spawn();
    }
    #[cfg(not(windows))]
    {
        let _ = std::process::Command::new("xdg-open").arg(path).spawn();
    }
}

/// Minutes:seconds (or h:mm:ss).
pub fn clock(d: Duration) -> String {
    let s = d.as_secs();
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

/// A small coloured dot (the default font has no symbol for it).
pub fn status_dot(ui: &mut egui::Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
    ui.painter().circle_filled(rect.center(), 4.0, color);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_text() {
        assert_eq!(clock(Duration::from_secs(5)), "0:05");
        assert_eq!(clock(Duration::from_secs(161)), "2:41");
        assert_eq!(clock(Duration::from_secs(3725)), "1:02:05");
    }
}
