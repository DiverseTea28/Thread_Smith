//! The main window: menus, side panels and the 3D viewport.

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};

use egui::{Color32, PointerButton, RichText, Sense};
use glam::{EulerRot, Mat4, Quat, Vec2, Vec3, Vec4};
use ts_core::{Pose, Scene};

use crate::camera::OrbitCamera;
use crate::viewport::{DrawSettings, LineVertex, Viewport};

/// Result of a background file load.
type LoadResult = (PathBuf, anyhow::Result<Scene>);

pub struct ThreadSmithApp {
    scene: Option<Scene>,
    pose: Pose,
    /// Per joint: the pose rotation as X/Y/Z degrees (what the sliders edit).
    pose_euler: Vec<Vec3>,
    selected_joint: Option<usize>,
    joint_filter: String,

    camera: OrbitCamera,
    show_shaded: bool,
    show_wireframe: bool,
    show_skeleton: bool,
    show_grid: bool,

    viewport: Option<Viewport>,
    /// The scene changed shape and must be re-uploaded / re-skinned.
    needs_upload: bool,
    needs_pose_update: bool,

    loading: Option<(String, Receiver<LoadResult>)>,
    status: String,
    error: Option<String>,
}

impl ThreadSmithApp {
    pub fn new(cc: &eframe::CreationContext<'_>, open_path: Option<PathBuf>) -> Self {
        cc.egui_ctx.set_visuals(egui::Visuals::dark());
        let viewport = cc
            .wgpu_render_state
            .as_ref()
            .map(|rs| Viewport::new(&rs.device));
        let mut app = Self {
            scene: None,
            pose: Pose::default(),
            pose_euler: Vec::new(),
            selected_joint: None,
            joint_filter: String::new(),
            camera: OrbitCamera::default(),
            show_shaded: true,
            show_wireframe: false,
            show_skeleton: true,
            show_grid: true,
            viewport,
            needs_upload: false,
            needs_pose_update: false,
            loading: None,
            status: "Open a MetaHuman body (FBX) or any glTF / OBJ model to start.".into(),
            error: None,
        };
        if app.viewport.is_none() {
            app.error = Some("Could not start the GPU renderer (wgpu).".into());
        }
        if let Some(p) = open_path {
            app.start_load(p);
        }
        app
    }

    // ---- Loading -----------------------------------------------------------

    fn open_dialog(&mut self) {
        let picked = rfd::FileDialog::new()
            .set_title("Open a model")
            .add_filter("3D models", ts_io::SUPPORTED_EXTENSIONS)
            .pick_file();
        if let Some(path) = picked {
            self.start_load(path);
        }
    }

    fn start_load(&mut self, path: PathBuf) {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let result = ts_io::load(&path);
            let _ = tx.send((path, result));
        });
        self.status = format!("Loading {name}…");
        self.error = None;
        self.loading = Some((name, rx));
    }

    fn poll_loading(&mut self, ctx: &egui::Context) {
        let Some((_, rx)) = &self.loading else { return };
        match rx.try_recv() {
            Ok((path, Ok(scene))) => {
                self.loading = None;
                self.status = format!(
                    "Opened {} · {} parts · {} triangles · {} bones{}",
                    path.file_name()
                        .map(|n| n.to_string_lossy())
                        .unwrap_or_default(),
                    scene.parts.len(),
                    fmt_count(scene.triangle_count()),
                    scene.skeleton.len(),
                    if scene.skeleton.looks_like_ue5_body() {
                        " · UE5 / MetaHuman skeleton"
                    } else {
                        ""
                    }
                );
                self.set_scene(scene);
            }
            Ok((path, Err(e))) => {
                self.loading = None;
                self.error = Some(format!("Could not open {}:\n{e:#}", path.display()));
                self.status = "Open failed.".into();
            }
            Err(mpsc::TryRecvError::Empty) => ctx.request_repaint(),
            Err(mpsc::TryRecvError::Disconnected) => {
                self.loading = None;
                self.error = Some("The loader stopped unexpectedly.".into());
            }
        }
    }

    fn set_scene(&mut self, scene: Scene) {
        self.pose = Pose::bind(&scene.skeleton);
        self.pose_euler = vec![Vec3::ZERO; scene.skeleton.len()];
        self.selected_joint = None;
        if let Some(b) = scene.bounds(&self.pose) {
            self.camera.frame(b);
        }
        self.scene = Some(scene);
        self.needs_upload = true;
    }

    fn handle_dropped_files(&mut self, ctx: &egui::Context) {
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .collect()
        });
        if let Some(path) = dropped.into_iter().next() {
            self.start_load(path);
        }
    }

    // ---- Posing --------------------------------------------------------------

    fn reset_pose(&mut self) {
        if let Some(scene) = &self.scene {
            self.pose = Pose::bind(&scene.skeleton);
            self.pose_euler = vec![Vec3::ZERO; scene.skeleton.len()];
            self.needs_pose_update = true;
        }
    }

    fn apply_euler(&mut self, joint: usize) {
        let e = self.pose_euler[joint];
        self.pose.offsets[joint] = Quat::from_euler(
            EulerRot::XYZ,
            e.x.to_radians(),
            e.y.to_radians(),
            e.z.to_radians(),
        );
        self.needs_pose_update = true;
    }

    fn frame_scene(&mut self) {
        if let Some(b) = self.scene.as_ref().and_then(|s| s.bounds(&self.pose)) {
            self.camera.frame(b);
        }
    }

    // ---- Panels --------------------------------------------------------------

    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                if ui.button("Open model…").clicked() {
                    ui.close();
                    self.open_dialog();
                }
                if ui
                    .add_enabled(self.scene.is_some(), egui::Button::new("Close model"))
                    .clicked()
                {
                    ui.close();
                    self.scene = None;
                    self.selected_joint = None;
                    if let Some(v) = &mut self.viewport {
                        v.clear_scene();
                    }
                    self.status = "Model closed.".into();
                }
                ui.separator();
                if ui.button("Quit").clicked() {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
            ui.menu_button("View", |ui| {
                ui.checkbox(&mut self.show_shaded, "Shaded");
                ui.checkbox(&mut self.show_wireframe, "Wireframe");
                ui.checkbox(&mut self.show_skeleton, "Skeleton");
                ui.checkbox(&mut self.show_grid, "Floor grid");
                ui.separator();
                if ui.button("Frame model   (F)").clicked() {
                    ui.close();
                    self.frame_scene();
                }
            });
            ui.menu_button("Pose", |ui| {
                if ui.button("Reset to bind pose").clicked() {
                    ui.close();
                    self.reset_pose();
                }
            });
        });
    }

    fn scene_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Scene");
        let Some(scene) = &mut self.scene else {
            ui.add_space(8.0);
            if ui.button("Open model…").clicked() {
                self.open_dialog();
            }
            ui.label(RichText::new("Or drag a file onto the window.").weak());
            return;
        };
        ui.label(RichText::new(&scene.name).strong());
        ui.add_space(4.0);

        egui::CollapsingHeader::new(format!("Meshes ({})", scene.parts.len()))
            .default_open(true)
            .show(ui, |ui| {
                let mut changed = false;
                for part in &mut scene.parts {
                    changed |= ui.checkbox(&mut part.visible, &part.name).changed();
                    let skin = if part.mesh.skin.is_some() {
                        " · skinned"
                    } else {
                        ""
                    };
                    ui.indent(("part info", &part.name), |ui| {
                        ui.label(
                            RichText::new(format!(
                                "{} triangles{skin}",
                                fmt_count(part.mesh.triangle_count())
                            ))
                            .weak()
                            .small(),
                        );
                    });
                }
                if changed {
                    if let Some(v) = &mut self.viewport {
                        v.set_visibility(scene);
                    }
                }
            });

        let skeleton = &scene.skeleton;
        egui::CollapsingHeader::new(format!("Skeleton ({} bones)", skeleton.len()))
            .default_open(true)
            .show(ui, |ui| {
                if skeleton.is_empty() {
                    ui.label(RichText::new("No bones in this file.").weak());
                    return;
                }
                ui.add(
                    egui::TextEdit::singleline(&mut self.joint_filter).hint_text("Search bones…"),
                );
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let filter = self.joint_filter.trim().to_lowercase();
                        if filter.is_empty() {
                            let roots: Vec<usize> = skeleton.roots().collect();
                            for r in roots {
                                joint_tree(ui, skeleton, r, &mut self.selected_joint);
                            }
                        } else {
                            for (i, j) in skeleton.joints.iter().enumerate() {
                                if j.name.to_lowercase().contains(&filter) {
                                    let selected = self.selected_joint == Some(i);
                                    if ui.selectable_label(selected, &j.name).clicked() {
                                        self.selected_joint = Some(i);
                                    }
                                }
                            }
                        }
                    });
            });
    }

    fn pose_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Pose");
        match &self.scene {
            None => {
                ui.label(RichText::new("Open a model to pose it.").weak());
                return;
            }
            Some(s) if s.skeleton.is_empty() => {
                ui.label(RichText::new("This model has no skeleton.").weak());
                return;
            }
            _ => {}
        }
        if ui.button("Reset to bind pose").clicked() {
            self.reset_pose();
        }
        let Some(scene) = &self.scene else { return };
        ui.add_space(8.0);
        let Some(j) = self.selected_joint else {
            ui.label(
                RichText::new("Pick a bone in the list, or click a bone dot in the viewport.")
                    .weak(),
            );
            return;
        };
        let name = scene.skeleton.joints[j].name.clone();
        let parent = scene.skeleton.joints[j]
            .parent
            .map(|p| scene.skeleton.joints[p].name.clone());
        ui.label(
            RichText::new(&name)
                .strong()
                .color(Color32::from_rgb(255, 210, 90)),
        );
        if let Some(p) = parent {
            ui.label(RichText::new(format!("parent: {p}")).weak());
        }
        ui.add_space(6.0);
        ui.label("Rotation added on top of the bind pose (degrees):");
        let mut changed = false;
        let e = &mut self.pose_euler[j];
        for (axis, value) in [("X", &mut e.x), ("Y", &mut e.y), ("Z", &mut e.z)] {
            changed |= ui
                .add(
                    egui::Slider::new(value, -180.0..=180.0)
                        .text(axis)
                        .fixed_decimals(1),
                )
                .changed();
        }
        if changed {
            self.apply_euler(j);
        }
        if ui.button("Reset this bone").clicked() {
            self.pose_euler[j] = Vec3::ZERO;
            self.apply_euler(j);
        }
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if self.loading.is_some() {
                ui.spinner();
            }
            ui.label(&self.status);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    RichText::new(
                        "Left drag: orbit · Right / middle drag: pan · Scroll: zoom · F: frame",
                    )
                    .weak(),
                );
            });
        });
    }

    // ---- Viewport ------------------------------------------------------------

    fn viewport_ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let (rect, response) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
        let rect_size = Vec2::new(rect.width(), rect.height());

        // Camera controls.
        let delta = response.drag_delta();
        if response.dragged_by(PointerButton::Primary) {
            self.camera.orbit(Vec2::new(delta.x, delta.y));
        } else if response.dragged_by(PointerButton::Secondary)
            || response.dragged_by(PointerButton::Middle)
        {
            self.camera.pan(Vec2::new(delta.x, delta.y), rect.height());
        }
        if response.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 {
                self.camera.zoom(scroll);
            }
        }
        if ui.input(|i| i.key_pressed(egui::Key::F)) && !ui.ctx().egui_wants_keyboard_input() {
            self.frame_scene();
        }

        // Joint world positions for this pose.
        let joint_world: Vec<Mat4> = self
            .scene
            .as_ref()
            .map(|s| self.pose.world_matrices(&s.skeleton))
            .unwrap_or_default();

        // Click a bone dot to select it.
        if response.clicked() && self.show_skeleton {
            if let Some(click) = response.interact_pointer_pos() {
                let local = Vec2::new(click.x - rect.min.x, click.y - rect.min.y);
                let best = joint_world
                    .iter()
                    .enumerate()
                    .filter_map(|(i, m)| {
                        self.camera
                            .project(m.w_axis.truncate(), rect_size)
                            .map(|p| (i, p.distance(local)))
                    })
                    .min_by(|a, b| a.1.total_cmp(&b.1));
                self.selected_joint = match best {
                    Some((i, d)) if d < 10.0 => Some(i),
                    _ => self.selected_joint,
                };
            }
        }

        let Some(rs) = frame.wgpu_render_state() else {
            return;
        };
        let Some(viewport) = &mut self.viewport else {
            return;
        };

        if let Some(scene) = &self.scene {
            if self.needs_upload {
                viewport.set_scene(&rs.device, scene, &joint_world);
                self.needs_upload = false;
                self.needs_pose_update = false;
            } else if self.needs_pose_update {
                viewport.update_pose(&rs.queue, scene, &joint_world);
                self.needs_pose_update = false;
            }
        }

        let overlay_lines = match (&self.scene, self.show_skeleton) {
            (Some(scene), true) => {
                skeleton_lines(scene, &joint_world, self.selected_joint, &self.camera)
            }
            _ => Vec::new(),
        };
        let settings = DrawSettings {
            wireframe: self.show_wireframe,
            shaded: self.show_shaded,
            grid: self.show_grid,
            overlay_lines,
        };
        let ppp = ui.ctx().pixels_per_point();
        let px = [
            (rect.width() * ppp).round() as u32,
            (rect.height() * ppp).round() as u32,
        ];
        let texture = {
            let mut renderer = rs.renderer.write();
            viewport.render(
                &rs.device,
                &rs.queue,
                &mut renderer,
                px,
                &self.camera,
                &settings,
            )
        };
        ui.painter().image(
            texture,
            rect,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            Color32::WHITE,
        );

        if self.scene.is_none() && self.loading.is_none() {
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "File → Open model…   or drop a file here",
                egui::FontId::proportional(18.0),
                Color32::from_gray(140),
            );
        }
    }
}

impl eframe::App for ThreadSmithApp {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll_loading(&ctx);
        self.handle_dropped_files(&ctx);

        egui::Panel::top("menu").show(ui, |ui| self.menu_bar(ui));
        egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui));
        egui::Panel::left("scene")
            .resizable(true)
            .default_size(280.0)
            .show(ui, |ui| self.scene_panel(ui));
        egui::Panel::right("pose")
            .resizable(true)
            .default_size(260.0)
            .show(ui, |ui| self.pose_panel(ui));
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| self.viewport_ui(ui, frame));

        if let Some(err) = self.error.clone() {
            egui::Window::new("Problem")
                .collapsible(false)
                .resizable(false)
                .show(&ctx, |ui| {
                    ui.label(err);
                    if ui.button("OK").clicked() {
                        self.error = None;
                    }
                });
        }
    }
}

/// One row of the bone tree, and its children.
fn joint_tree(
    ui: &mut egui::Ui,
    skeleton: &ts_core::Skeleton,
    joint: usize,
    selected: &mut Option<usize>,
) {
    let children: Vec<usize> = skeleton.children(joint).collect();
    let name = &skeleton.joints[joint].name;
    let is_selected = *selected == Some(joint);
    if children.is_empty() {
        if ui.selectable_label(is_selected, name).clicked() {
            *selected = Some(joint);
        }
        return;
    }
    let id = ui.make_persistent_id(("joint", joint));
    egui::collapsing_header::CollapsingState::load_with_default_open(
        ui.ctx(),
        id,
        skeleton.joints[joint].parent.is_none(),
    )
    .show_header(ui, |ui| {
        if ui.selectable_label(is_selected, name).clicked() {
            *selected = Some(joint);
        }
    })
    .body(|ui| {
        for c in children {
            joint_tree(ui, skeleton, c, selected);
        }
    });
}

/// Bone lines plus a small cross at each joint. The selected bone is yellow.
fn skeleton_lines(
    scene: &Scene,
    joint_world: &[Mat4],
    selected: Option<usize>,
    camera: &OrbitCamera,
) -> Vec<LineVertex> {
    let normal = Vec4::new(1.0, 0.55, 0.15, 0.9);
    let hot = Vec4::new(1.0, 0.85, 0.2, 1.0);
    let cross = camera.distance * 0.006;
    let mut lines = Vec::with_capacity(joint_world.len() * 8);
    for (i, joint) in scene.skeleton.joints.iter().enumerate() {
        let p = joint_world[i].w_axis.truncate();
        let color = if selected == Some(i) { hot } else { normal };
        if let Some(parent) = joint.parent {
            let pp = joint_world[parent].w_axis.truncate();
            let c = if selected == Some(parent) || selected == Some(i) {
                hot
            } else {
                normal
            };
            lines.push(LineVertex::new(pp, c));
            lines.push(LineVertex::new(p, c));
        }
        let s = if selected == Some(i) {
            cross * 2.0
        } else {
            cross
        };
        for axis in [Vec3::X, Vec3::Y, Vec3::Z] {
            lines.push(LineVertex::new(p - axis * s, color));
            lines.push(LineVertex::new(p + axis * s, color));
        }
    }
    lines
}

fn fmt_count(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}
