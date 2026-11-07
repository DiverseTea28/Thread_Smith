//! The main window: menus, side panels and the 3D viewport.

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};

use egui::{Color32, PointerButton, RichText, Sense};
use glam::{EulerRot, Mat4, Quat, Vec2, Vec3, Vec4};
use ts_core::{Mesh, PartRole, Pose, Scene};
use ts_fit::sculpt::{self, GrabStroke, RayHit};
use ts_fit::{FitSettings, SkinSettings, TriBvh, WeldGraph};

use ts_library::{BodyInfo, Library, Outfit, Thumbnail, CATEGORIES};

use crate::animation::{self, AnimPlayer};
use crate::bridge::{BodyRequest, Bridge, BridgeEvent, OutfitMessage};
use crate::camera::OrbitCamera;
use crate::garment::{self, GarmentStatus, GarmentWork, Placement};
use crate::tools::{self, BrushSettings, Gizmo, GizmoDrag, Handle, Tool};
use crate::viewport::{DrawSettings, LineVertex, Viewport};

/// What a file is being opened as.
#[derive(Clone, Copy, PartialEq, Eq)]
enum LoadKind {
    /// Replaces everything: the character's body.
    Body,
    /// Added on top of the body as clothing.
    Garment,
}

/// Result of a background file load.
type LoadResult = (PathBuf, LoadKind, anyhow::Result<Scene>);

/// How a garment was before a change, for Undo.
struct UndoStep {
    garment: usize,
    mesh: Mesh,
    placement: Placement,
    status: GarmentStatus,
}

/// Most undo steps kept.
const UNDO_LIMIT: usize = 40;

/// A sculpt stroke in progress.
enum Stroke {
    Grab {
        grab: GrabStroke,
        /// Where the stroke started on the garment.
        start: Vec3,
    },
    /// Relax / Inflate: dabs every frame while the mouse is held.
    Paint { graph: WeldGraph },
}

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

    garments: Vec<GarmentWork>,
    selected_garment: usize,
    fit_settings: FitSettings,
    skin_settings: SkinSettings,

    loading: Option<(String, Receiver<LoadResult>)>,
    /// Garment to add once the body has loaded (from the command line).
    pending_garment: Option<PathBuf>,

    ctx: egui::Context,
    bridge: Bridge,
    /// Where the current body came from in Unreal (if it was sent by the plugin).
    unreal_body: Option<BodyRequest>,
    /// Body request waiting for its file to finish loading.
    pending_unreal: Option<BodyRequest>,
    library: Option<Library>,
    outfits: Vec<Outfit>,
    outfit_name: String,
    outfit_category: String,
    apply_to_character: bool,
    /// Save (false) or save + send to Unreal (true), done on the next frame
    /// because it needs the GPU for the thumbnail.
    pending_save: Option<bool>,
    status: String,
    error: Option<String>,

    // ---- Placing and sculpting garments ----
    tool: Tool,
    brush: BrushSettings,
    /// Gizmo handle under the mouse.
    gizmo_hot: Option<Handle>,
    gizmo_drag: Option<GizmoDrag>,
    /// Where the brush is on the garment (under the mouse).
    brush_hit: Option<RayHit>,
    stroke: Option<Stroke>,
    undo: Vec<UndoStep>,
    /// Last undo step: (garment, kind, time). Quick repeats of the same kind
    /// of change (dragging a number) share one undo step.
    undo_last: Option<(usize, &'static str, f64)>,
    /// Scene part whose vertices moved and needs re-uploading this frame.
    dirty_part: Option<usize>,
    /// The body's surface for "keep outside the skin" (built when first needed).
    body_cache: Option<TriBvh>,

    anim: AnimPlayer,
    /// The animation time or settings changed: work out the pose again.
    anim_dirty: bool,
    /// Head file to add to the next body that loads (sent by Unreal).
    pending_head: Option<PathBuf>,
}

impl ThreadSmithApp {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        open_path: Option<PathBuf>,
        garment_path: Option<PathBuf>,
    ) -> Self {
        cc.egui_ctx.set_visuals(egui::Visuals::dark());
        let wake_ctx = cc.egui_ctx.clone();
        let bridge = Bridge::start(move || wake_ctx.request_repaint());
        let (library, outfits, library_error) = match Library::open(Library::default_location()) {
            Ok(lib) => {
                let list = lib.list();
                (Some(lib), list, None)
            }
            Err(e) => (None, Vec::new(), Some(format!("{e:#}"))),
        };
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
            garments: Vec::new(),
            selected_garment: 0,
            fit_settings: FitSettings::default(),
            skin_settings: SkinSettings::default(),
            loading: None,
            pending_garment: garment_path,
            ctx: cc.egui_ctx.clone(),
            bridge,
            unreal_body: None,
            pending_unreal: None,
            library,
            outfits,
            outfit_name: String::new(),
            outfit_category: "Top".into(),
            apply_to_character: true,
            pending_save: None,
            status: "Open a MetaHuman body (FBX) or any glTF / OBJ model to start.".into(),
            error: None,
            tool: Tool::Camera,
            brush: BrushSettings::default(),
            gizmo_hot: None,
            gizmo_drag: None,
            brush_hit: None,
            stroke: None,
            undo: Vec::new(),
            undo_last: None,
            dirty_part: None,
            body_cache: None,
            anim: AnimPlayer::new(Some(Library::default_location().join("Animations"))),
            anim_dirty: false,
            pending_head: None,
        };
        if let Some(e) = library_error {
            app.error = Some(format!("Couldn't open the outfit library: {e}"));
        }
        if app.viewport.is_none() {
            app.error = Some("Could not start the GPU renderer (wgpu).".into());
        }
        if let Some(p) = open_path {
            app.start_load(p, LoadKind::Body);
        }
        app
    }

    // ---- Loading -----------------------------------------------------------

    fn open_dialog(&mut self, kind: LoadKind) {
        let title = match kind {
            LoadKind::Body => "Open a body (MetaHuman FBX, glTF, OBJ)",
            LoadKind::Garment => "Add a garment",
        };
        let picked = rfd::FileDialog::new()
            .set_title(title)
            .add_filter("3D models", ts_io::SUPPORTED_EXTENSIONS)
            .pick_file();
        if let Some(path) = picked {
            self.start_load(path, kind);
        }
    }

    fn has_body(&self) -> bool {
        self.scene
            .as_ref()
            .is_some_and(|s| garment::body_part(s).is_some())
    }

    fn start_load(&mut self, path: PathBuf, kind: LoadKind) {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let head = if kind == LoadKind::Body {
            self.pending_head.take()
        } else {
            None
        };
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut result = ts_io::load(&path);
            // A MetaHuman head sent with the body: add it, skinned to the body skeleton.
            if let (Ok(scene), Some(head)) = (&mut result, head) {
                match ts_io::load(&head) {
                    Ok(h) => {
                        scene.add_skinned_parts(h, PartRole::Head);
                    }
                    Err(e) => log::warn!("Couldn't load the head {}: {e:#}", head.display()),
                }
            }
            let _ = tx.send((path, kind, result));
        });
        self.status = format!("Loading {name}…");
        self.error = None;
        self.loading = Some((name, rx));
    }

    fn poll_loading(&mut self, ctx: &egui::Context) {
        let Some((_, rx)) = &self.loading else { return };
        match rx.try_recv() {
            Ok((path, LoadKind::Garment, Ok(file))) => {
                self.loading = None;
                self.add_garment_file(file, &path);
            }
            Ok((path, LoadKind::Body, Ok(scene))) => {
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
                self.unreal_body = self.pending_unreal.take();
                if let Some(u) = &self.unreal_body {
                    self.status = format!(
                        "Received {} from Unreal. Add a garment to start dressing.",
                        if u.character.is_empty() {
                            "a body"
                        } else {
                            &u.character
                        }
                    );
                }
                if let Some(g) = self.pending_garment.take() {
                    self.start_load(g, LoadKind::Garment);
                }
            }
            Ok((path, _, Err(e))) => {
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
        self.garments.clear();
        self.selected_garment = 0;
        self.needs_upload = true;
        self.undo.clear();
        self.undo_last = None;
        self.body_cache = None;
        self.gizmo_drag = None;
        self.stroke = None;
        self.anim.stop();
        self.anim_dirty = false;
    }

    fn add_garment_file(&mut self, file: Scene, path: &std::path::Path) {
        let Some(scene) = &mut self.scene else {
            self.error = Some("Open a body first (File → Open body), then add clothes.".into());
            return;
        };
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        match garment::add_garments(scene, file) {
            Ok(new) => {
                let had_reference = new.first().is_some_and(|g| g.reference.is_some());
                self.selected_garment = self.garments.len();
                self.status = format!(
                    "Added {} garment(s) from {name}{}. Next: Fit to body, then Auto skin.",
                    new.len(),
                    if had_reference {
                        " and lined it up using its reference body"
                    } else {
                        ""
                    }
                );
                self.garments.extend(new);
                self.needs_upload = true;
            }
            Err(e) => self.error = Some(e),
        }
    }

    fn handle_dropped_files(&mut self, ctx: &egui::Context) {
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .collect()
        });
        // Dropping a file onto a loaded body adds it as clothing.
        if let Some(path) = dropped.into_iter().next() {
            let kind = if self.has_body() {
                LoadKind::Garment
            } else {
                LoadKind::Body
            };
            self.start_load(path, kind);
        }
    }

    // ---- Unreal bridge and library ---------------------------------------------

    fn poll_bridge(&mut self) {
        while let Ok(ev) = self.bridge.events.try_recv() {
            match ev {
                BridgeEvent::LoadBody(req) => {
                    let path = PathBuf::from(&req.file);
                    self.pending_head = req
                        .head_file
                        .as_ref()
                        .filter(|h| !h.is_empty())
                        .map(PathBuf::from);
                    self.pending_unreal = Some(req);
                    self.start_load(path, LoadKind::Body);
                }
                BridgeEvent::OutfitSent(Ok(asset)) => {
                    self.status = format!("Sent to Unreal: {asset}");
                }
                BridgeEvent::OutfitSent(Err(e)) => {
                    self.status = "Sending to Unreal failed.".into();
                    self.error = Some(format!(
                        "The outfit is saved in the library, but sending it to Unreal failed:\n{e}"
                    ));
                }
                BridgeEvent::ServerError(e) => self.status = e,
            }
        }
    }

    fn refresh_library(&mut self) {
        if let Some(lib) = &self.library {
            self.outfits = lib.list();
        }
    }

    /// Save the included, skinned garments as an outfit (and send it to Unreal).
    fn save_outfit(&mut self, rs: &egui_wgpu::RenderState, joint_world: &[Mat4], send: bool) {
        let Some(scene) = self.scene.as_mut() else {
            return;
        };
        let Some(library) = &self.library else {
            self.error = Some("The outfit library isn't available.".into());
            return;
        };
        let chosen: Vec<usize> = self
            .garments
            .iter()
            .filter(|g| g.include && g.status == GarmentStatus::Skinned)
            .map(|g| g.part)
            .collect();
        if chosen.is_empty() {
            self.error = Some("Nothing to save yet: fit and Auto skin at least one garment (and tick \"include\").".into());
            return;
        }

        // ---- Thumbnail: only the chosen garments, framed nicely ----
        let mut thumbnail = None;
        if let Some(viewport) = &mut self.viewport {
            let saved: Vec<bool> = scene.parts.iter().map(|p| p.visible).collect();
            for (i, p) in scene.parts.iter_mut().enumerate() {
                p.visible = chosen.contains(&i);
            }
            viewport.set_visibility(scene);
            let pts: Vec<Vec3> = chosen
                .iter()
                .flat_map(|&i| scene.parts[i].mesh.skinned(joint_world).0)
                .collect();
            let mut cam = OrbitCamera {
                yaw: 0.35,
                pitch: 0.12,
                ..Default::default()
            };
            if let Some(b) = ts_core::Bounds::from_points(pts) {
                cam.frame(b);
                cam.distance *= 0.95;
            }
            let settings = DrawSettings {
                wireframe: false,
                shaded: true,
                grid: false,
                overlay_lines: Vec::new(),
            };
            {
                let mut renderer = rs.renderer.write();
                viewport.render(
                    &rs.device,
                    &rs.queue,
                    &mut renderer,
                    [512, 512],
                    &cam,
                    &settings,
                );
            }
            if let Some((width, height, rgba)) = viewport.read_pixels(&rs.device, &rs.queue) {
                thumbnail = Some(Thumbnail {
                    width,
                    height,
                    rgba,
                });
            }
            for (p, v) in scene.parts.iter_mut().zip(saved) {
                p.visible = v;
            }
            viewport.set_visibility(scene);
        }

        // ---- Save ----
        let name = if self.outfit_name.trim().is_empty() {
            chosen
                .iter()
                .map(|&i| scene.parts[i].name.clone())
                .collect::<Vec<_>>()
                .join(" + ")
        } else {
            self.outfit_name.trim().to_string()
        };
        let character = self
            .unreal_body
            .as_ref()
            .map(|u| u.character.clone())
            .filter(|c| !c.is_empty())
            .unwrap_or_else(|| scene.name.clone());
        let body = BodyInfo {
            character: character.clone(),
            bones: scene.skeleton.len(),
            unreal_skeleton: self
                .unreal_body
                .as_ref()
                .and_then(|u| u.skeleton_asset.clone()),
            unreal_body: self.unreal_body.as_ref().and_then(|u| u.body_asset.clone()),
        };
        let meshes: Vec<ts_io::ExportMesh> = chosen
            .iter()
            .map(|&i| ts_io::ExportMesh {
                name: &scene.parts[i].name,
                mesh: &scene.parts[i].mesh,
                color: [0.8, 0.8, 0.8],
            })
            .collect();
        let outfit = match library.save(
            &name,
            &self.outfit_category,
            body,
            &scene.skeleton,
            &meshes,
            thumbnail.as_ref(),
        ) {
            Ok(o) => o,
            Err(e) => {
                self.error = Some(format!("Couldn't save the outfit: {e:#}"));
                return;
            }
        };
        self.status = format!("Saved \"{}\" to the library.", outfit.info.name);
        if send {
            let msg = OutfitMessage {
                outfit_dir: outfit.dir.to_string_lossy().into_owned(),
                mesh_file: outfit.mesh_path().to_string_lossy().into_owned(),
                name: outfit.info.name.clone(),
                category: outfit.info.category.clone(),
                character,
                skeleton_asset: outfit.info.body.unreal_skeleton.clone(),
                body_asset: outfit.info.body.unreal_body.clone(),
                apply_to_character: self.apply_to_character,
            };
            let ctx = self.ctx.clone();
            self.bridge.send_outfit(msg, move || ctx.request_repaint());
            self.status = format!("Saved \"{}\". Sending to Unreal…", outfit.info.name);
        }
        self.refresh_library();
    }

    fn library_panel(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new(format!("Outfit library ({})", self.outfits.len()))
            .default_open(true)
            .show(ui, |ui| {
                let Some(lib) = &self.library else {
                    ui.label(RichText::new("Library not available.").weak());
                    return;
                };
                ui.horizontal(|ui| {
                    if ui.small_button("Refresh").clicked() {
                        self.outfits = lib.list();
                    }
                    if ui.small_button("Open folder").clicked() {
                        open_in_explorer(&lib.root);
                    }
                });
                if self.outfits.is_empty() {
                    ui.label(RichText::new("No outfits saved yet.").weak().small());
                }
                for o in &self.outfits {
                    ui.horizontal(|ui| {
                        ui.label(&o.info.name);
                        ui.label(
                            RichText::new(format!(
                                "{} · {}",
                                o.info.category, o.info.body.character
                            ))
                            .weak()
                            .small(),
                        );
                    })
                    .response
                    .on_hover_text(o.dir.display().to_string());
                }
            });
    }

    // ---- Posing --------------------------------------------------------------

    fn reset_pose(&mut self) {
        self.anim.stop();
        if let Some(scene) = &self.scene {
            self.pose = Pose::bind(&scene.skeleton);
            self.pose_euler = vec![Vec3::ZERO; scene.skeleton.len()];
            self.needs_pose_update = true;
        }
    }

    fn apply_euler(&mut self, joint: usize) {
        if self.pose.animated.is_some() {
            self.stop_animation();
        }
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
                if ui.button("Open body…").clicked() {
                    ui.close();
                    self.open_dialog(LoadKind::Body);
                }
                if ui
                    .add_enabled(self.has_body(), egui::Button::new("Add garment…"))
                    .clicked()
                {
                    ui.close();
                    self.open_dialog(LoadKind::Garment);
                }
                if ui.button("Add animations…").clicked() {
                    ui.close();
                    self.add_animations_dialog();
                }
                ui.separator();
                if ui
                    .add_enabled(self.scene.is_some(), egui::Button::new("Close all"))
                    .clicked()
                {
                    ui.close();
                    self.scene = None;
                    self.garments.clear();
                    self.undo.clear();
                    self.body_cache = None;
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
            if ui.button("Open body…").clicked() {
                self.open_dialog(LoadKind::Body);
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
                    let role = match part.role {
                        PartRole::Body => "",
                        PartRole::Garment => " · garment",
                        PartRole::Reference => " · reference",
                        PartRole::Head => " · head",
                    };
                    ui.indent(("part info", &part.name), |ui| {
                        ui.label(
                            RichText::new(format!(
                                "{} triangles{skin}{role}",
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

    // ---- Garment editing helpers --------------------------------------------

    /// Is the body in its bind pose? Brushes only work then (the garment is
    /// edited in its resting shape).
    fn at_bind_pose(&self) -> bool {
        self.pose.animated.is_none() && self.pose_euler.iter().all(|e| *e == Vec3::ZERO)
    }

    /// Save how garment `gi` is now, so the next change can be undone.
    /// Quick repeats of the same `kind` of change share one step.
    fn remember(&mut self, gi: usize, kind: &'static str) {
        let now = self.ctx.input(|i| i.time);
        if let Some((g, k, t)) = self.undo_last {
            if g == gi && k == kind && now - t < 0.8 {
                self.undo_last = Some((gi, kind, now));
                return;
            }
        }
        let (Some(scene), Some(g)) = (&self.scene, self.garments.get(gi)) else {
            return;
        };
        self.undo.push(UndoStep {
            garment: gi,
            mesh: scene.parts[g.part].mesh.clone(),
            placement: g.placement,
            status: g.status,
        });
        if self.undo.len() > UNDO_LIMIT {
            self.undo.remove(0);
        }
        self.undo_last = Some((gi, kind, now));
    }

    fn undo_step(&mut self) {
        let Some(step) = self.undo.pop() else {
            self.status = "Nothing to undo.".into();
            return;
        };
        self.undo_last = None;
        let (Some(scene), Some(g)) = (&mut self.scene, self.garments.get_mut(step.garment)) else {
            return;
        };
        scene.parts[g.part].mesh = step.mesh;
        g.placement = step.placement;
        g.status = step.status;
        g.report = None;
        self.selected_garment = step.garment;
        self.dirty_part = Some(g.part);
        self.status = "Undone.".into();
    }

    /// Change the selected garment's placement (from the gizmo or the numbers).
    fn set_placement(&mut self, gi: usize, placement: Placement, kind: &'static str) {
        if self.garments[gi].placement == placement {
            return;
        }
        self.remember(gi, kind);
        let Some(scene) = &mut self.scene else { return };
        let g = &mut self.garments[gi];
        let was_fitted = g.status != GarmentStatus::Added;
        g.placement = placement;
        if g.use_reference && g.reference.is_some() {
            // The reference body only fits garments where they were made.
            g.use_reference = false;
        }
        garment::apply_placement(scene, g);
        self.dirty_part = Some(g.part);
        if was_fitted {
            self.status = "Moved: the fit was undone. Fit to body again when it's in place.".into();
        }
    }

    fn tool_buttons(&mut self, ui: &mut egui::Ui, tools: &[Tool]) {
        ui.horizontal(|ui| {
            for &t in tools {
                let label = format!("{} ({:?})", t.label(), t.key());
                if ui
                    .selectable_label(self.tool == t, label)
                    .on_hover_text(t.help())
                    .clicked()
                {
                    self.tool = if self.tool == t { Tool::Camera } else { t };
                }
            }
        });
    }

    fn garment_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Garment");
        if !self.has_body() {
            ui.label(RichText::new("Open a skinned body first.").weak());
            return;
        }
        if ui.button("Add garment…").clicked() {
            self.open_dialog(LoadKind::Garment);
        }
        if self.garments.is_empty() {
            ui.label(RichText::new("Or drag a clothing file onto the window.").weak());
            return;
        }
        self.selected_garment = self.selected_garment.min(self.garments.len() - 1);
        let gi = self.selected_garment;
        {
            let Some(scene) = &self.scene else { return };
            if self.garments.len() > 1 {
                egui::ComboBox::from_label("Garment")
                    .selected_text(scene.parts[self.garments[gi].part].name.clone())
                    .show_ui(ui, |ui| {
                        for (i, g) in self.garments.iter().enumerate() {
                            ui.selectable_value(
                                &mut self.selected_garment,
                                i,
                                &scene.parts[g.part].name,
                            );
                        }
                    });
            }
            let g = &self.garments[gi];
            ui.label(RichText::new(&scene.parts[g.part].name).strong());
            let color = match g.status {
                GarmentStatus::Added => Color32::from_rgb(220, 170, 80),
                GarmentStatus::Fitted => Color32::from_rgb(150, 200, 255),
                GarmentStatus::Skinned => Color32::from_rgb(120, 210, 120),
            };
            ui.label(RichText::new(g.status.label()).color(color));
        }
        ui.horizontal(|ui| {
            if ui
                .add_enabled(!self.undo.is_empty(), egui::Button::new("Undo (Ctrl+Z)"))
                .clicked()
            {
                self.undo_step();
            }
        });
        ui.add_space(6.0);
        self.place_section(ui, gi);
        ui.add_space(6.0);
        self.fit_section(ui, gi);
        ui.add_space(6.0);
        self.sculpt_section(ui);
        ui.add_space(6.0);
        self.skin_section(ui, gi);
        ui.add_space(6.0);
        self.save_section(ui);
    }

    fn place_section(&mut self, ui: &mut egui::Ui, gi: usize) {
        ui.label(RichText::new("1. Place").strong());
        ui.label(
            RichText::new("Line the garment up on the body before fitting.")
                .weak()
                .small(),
        );
        self.tool_buttons(ui, &Tool::PLACE);
        let mut p = self.garments[gi].placement;
        let mut changed = false;
        egui::Grid::new("placement").num_columns(4).show(ui, |ui| {
            // Move, in centimeters.
            ui.label("Move (cm)");
            for v in [
                &mut p.translation.x,
                &mut p.translation.y,
                &mut p.translation.z,
            ] {
                let mut cm = *v * 100.0;
                if ui
                    .add(egui::DragValue::new(&mut cm).speed(0.2).max_decimals(1))
                    .changed()
                {
                    *v = cm / 100.0;
                    changed = true;
                }
            }
            ui.end_row();
            // Turn, in degrees.
            ui.label("Turn (°)");
            let (x, y, z) = p.rotation.to_euler(EulerRot::XYZ);
            let mut e = [x.to_degrees(), y.to_degrees(), z.to_degrees()];
            let mut turned = false;
            for v in e.iter_mut() {
                turned |= ui
                    .add(egui::DragValue::new(v).speed(0.5).max_decimals(1))
                    .changed();
            }
            if turned {
                p.rotation = Quat::from_euler(
                    EulerRot::XYZ,
                    e[0].to_radians(),
                    e[1].to_radians(),
                    e[2].to_radians(),
                );
                changed = true;
            }
            ui.end_row();
            // Stretch per axis, in percent.
            ui.label("Stretch (%)");
            for i in 0..3 {
                let mut pc = p.scale[i] * 100.0;
                if ui
                    .add(
                        egui::DragValue::new(&mut pc)
                            .speed(0.3)
                            .range(5.0..=500.0)
                            .max_decimals(1),
                    )
                    .changed()
                {
                    p.scale[i] = pc / 100.0;
                    changed = true;
                }
            }
            ui.end_row();
        });
        let mut size_pc = p.size * 100.0;
        if ui
            .add(egui::Slider::new(&mut size_pc, 25.0..=300.0).text("size (%)"))
            .changed()
        {
            p.size = size_pc / 100.0;
            changed = true;
        }
        if changed {
            self.set_placement(gi, p, "place");
        }
        ui.horizontal(|ui| {
            if ui
                .button("Grow to clear body")
                .on_hover_text("Makes the garment just big enough that it doesn't sink into the body. Good for clothes that come in too small.")
                .clicked()
            {
                self.remember(gi, "grow");
                if let Some(scene) = &mut self.scene {
                    let g = &mut self.garments[gi];
                    match garment::grow_to_clear_body(scene, g) {
                        Ok(size) => {
                            self.status = format!(
                                "Size {:.0}%. Now fit to body to shrink wrap it on.",
                                size * 100.0
                            );
                            self.dirty_part = Some(g.part);
                        }
                        Err(e) => self.error = Some(e),
                    }
                }
            }
            if ui.button("Reset placement").clicked() {
                self.set_placement(gi, Placement::default(), "reset placement");
            }
        });
    }

    fn fit_section(&mut self, ui: &mut egui::Ui, gi: usize) {
        ui.label(RichText::new("2. Fit (shrink wrap)").strong());
        let g = &mut self.garments[gi];
        if g.reference.is_some() {
            ui.checkbox(&mut g.use_reference, "Use the file's reference body")
                .on_hover_text("Keeps the garment's distance from its original body, measured on the new body. Turned off when you move the garment by hand.");
        }
        let wraps = g.reference.is_none() || !g.use_reference;
        let mut gap_mm = self.fit_settings.gap * 1000.0;
        if ui
            .add(egui::Slider::new(&mut gap_mm, 0.5..=20.0).text("gap from skin (mm)"))
            .changed()
        {
            self.fit_settings.gap = gap_mm / 1000.0;
        }
        if wraps {
            ui.add(
                egui::Slider::new(&mut self.fit_settings.tightness, 0.0..=1.0)
                    .text("tightness"),
            )
            .on_hover_text("0 only pushes the garment out of the skin. 1 pulls loose cloth all the way in to the skin.");
        }
        ui.add(egui::Slider::new(&mut self.fit_settings.smoothing, 0..=40).text("keep shape"))
            .on_hover_text(
                "Higher keeps the garment's own shape; lower hugs the body more closely.",
            );
        let mut fit = false;
        let mut reset = false;
        ui.horizontal(|ui| {
            fit = ui.button("Fit to body").clicked();
            reset = ui
                .button("Reset")
                .on_hover_text("Back to how it was added (placement too).")
                .clicked();
        });
        if fit || reset {
            self.remember(gi, if fit { "fit" } else { "reset" });
            let Some(scene) = &mut self.scene else { return };
            let g = &mut self.garments[gi];
            if fit {
                match garment::fit(scene, g, &self.fit_settings) {
                    Ok(()) => {
                        self.status =
                            "Fitted. Touch it up with the Sculpt brushes, then Auto skin.".into()
                    }
                    Err(e) => self.error = Some(e),
                }
            } else {
                garment::reset(scene, g);
            }
            self.dirty_part = Some(g.part);
        }
    }

    fn sculpt_section(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("3. Sculpt").strong());
        self.tool_buttons(ui, &Tool::SCULPT);
        let mut radius_cm = self.brush.radius * 100.0;
        if ui
            .add(
                egui::Slider::new(&mut radius_cm, 0.5..=40.0)
                    .logarithmic(true)
                    .text("radius (cm)"),
            )
            .on_hover_text("Shortcut: [ and ]")
            .changed()
        {
            self.brush.radius = radius_cm / 100.0;
        }
        ui.add(egui::Slider::new(&mut self.brush.strength, 0.05..=1.0).text("strength"));
        ui.checkbox(&mut self.brush.keep_outside, "Keep outside the skin")
            .on_hover_text("When you let go, anything pushed into the body pops back out.");
        if !self.at_bind_pose() {
            ui.label(
                RichText::new("Reset the pose (Pose → Reset to bind pose) to sculpt.")
                    .color(Color32::from_rgb(220, 170, 80))
                    .small(),
            );
        }
    }

    fn skin_section(&mut self, ui: &mut egui::Ui, gi: usize) {
        ui.label(RichText::new("4. Auto skin").strong());
        let mut dist_cm = self.skin_settings.max_distance * 100.0;
        if ui.add(egui::Slider::new(&mut dist_cm, 1.0..=20.0).text("copy distance (cm)"))
            .on_hover_text("Parts closer to the body than this copy its weights directly. Further parts are filled in smoothly.")
            .changed()
        {
            self.skin_settings.max_distance = dist_cm / 100.0;
        }
        ui.checkbox(
            &mut self.skin_settings.rigid,
            "Rigid (whole piece follows one bone)",
        );
        if ui.button("Auto skin").clicked() {
            self.remember(gi, "skin");
            if let Some(scene) = &mut self.scene {
                let g = &mut self.garments[gi];
                match garment::skin(scene, g, &self.skin_settings) {
                    Ok(()) => self.status = "Skinned. Pose the body to test it.".into(),
                    Err(e) => self.error = Some(e),
                }
                self.dirty_part = Some(g.part);
            }
        }
        let g = &mut self.garments[gi];
        if let Some(r) = &g.report {
            ui.label(
                RichText::new(format!(
                    "{} copied · {} filled in · {} nearest",
                    fmt_count(r.matched),
                    fmt_count(r.inpainted),
                    fmt_count(r.fallback)
                ))
                .weak()
                .small(),
            );
        }
        ui.checkbox(&mut g.include, "Include in the outfit");
    }

    fn save_section(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("5. Save and send").strong());
        ui.add(egui::TextEdit::singleline(&mut self.outfit_name).hint_text("Outfit name"));
        egui::ComboBox::from_label("Type")
            .selected_text(self.outfit_category.clone())
            .show_ui(ui, |ui| {
                for c in CATEGORIES {
                    ui.selectable_value(&mut self.outfit_category, c.to_string(), *c);
                }
            });
        ui.horizontal(|ui| {
            if ui.button("Save to library").clicked() {
                self.pending_save = Some(false);
            }
            let send = ui.add_enabled(
                self.bridge.plugin_connected(),
                egui::Button::new("Send to Unreal"),
            );
            let send = send.on_disabled_hover_text(
                "Open Unreal with the ThreadSmith Bridge plugin turned on.",
            );
            if send.clicked() {
                self.pending_save = Some(true);
            }
        });
        ui.checkbox(
            &mut self.apply_to_character,
            "Put it on the character in Unreal",
        );
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
            .map(|p| (p, scene.skeleton.joints[p].name.clone()));
        ui.label(
            RichText::new(&name)
                .strong()
                .color(Color32::from_rgb(255, 210, 90)),
        );
        if let Some((pi, pname)) = parent {
            if ui
                .link(format!("parent: {pname}"))
                .on_hover_text("Select the parent bone")
                .clicked()
            {
                self.selected_joint = Some(pi);
            }
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
                let (dot, text) = if self.bridge.plugin_connected() {
                    (Color32::from_rgb(90, 210, 110), "Unreal connected")
                } else {
                    (Color32::from_gray(110), "Unreal not connected")
                };
                ui.label(RichText::new(text).color(dot)).on_hover_text(
                    "Open Unreal 5.8 with the ThreadSmith Bridge plugin turned on to send bodies and outfits back and forth.",
                );
                ui.separator();
                ui.label(
                    RichText::new(
                        "Left drag: orbit · Right / middle drag: pan · Scroll: zoom · F: frame · Ctrl+Z: undo",
                    )
                    .weak(),
                );
            });
        });
    }

    // ---- Animation ---------------------------------------------------------------

    fn add_animations_dialog(&mut self) {
        let Some(files) = rfd::FileDialog::new()
            .set_title("Add animations (Mixamo FBX)")
            .add_filter("FBX animation", &["fbx"])
            .pick_files()
        else {
            return;
        };
        let (n, err) = self.anim.add_files(&files);
        self.status = format!("Added {n} animation(s).");
        if let Some(e) = err {
            self.error = Some(e);
        }
    }

    /// Per frame: loading, playing, scanning and the live clipping check.
    fn anim_update(&mut self, ctx: &egui::Context) {
        let skeleton = self.scene.as_ref().map(|s| &s.skeleton);
        if let Some(e) = self.anim.poll_loading(skeleton) {
            self.error = Some(format!("Couldn't load that animation:\n{e}"));
        }
        self.anim.poll_scan();
        let dt = ctx.input(|i| i.stable_dt).min(0.1);
        let advanced = self.anim.advance(dt);
        if advanced || self.anim_dirty {
            self.anim_dirty = false;
            if let Some(scene) = &self.scene {
                if let Some(locals) = self.anim.pose(&scene.skeleton) {
                    self.pose.animated = Some(locals);
                    self.needs_pose_update = true;
                }
            }
        }
        // Live clipping marks (limited to about 8 times a second while playing).
        self.anim.since_check += dt;
        if self.anim.show_clipping && self.anim.retarget.is_some() {
            let time = self.anim.time;
            let due = !self.anim.playing || self.anim.since_check > 0.12;
            if due && self.anim.clip_checked_at != Some(time) {
                if let Some(scene) = &self.scene {
                    let world = self.pose.world_matrices(&scene.skeleton);
                    self.anim.clip_points = animation::clipping_points(scene, &world);
                    self.anim.clip_checked_at = Some(time);
                    self.anim.since_check = 0.0;
                }
            }
        } else {
            self.anim.clip_points.clear();
            self.anim.clip_checked_at = None;
        }
        if self.anim.playing || self.anim.loading.is_some() || self.anim.scan.is_some() {
            ctx.request_repaint();
        }
    }

    fn stop_animation(&mut self) {
        self.anim.stop();
        self.pose.animated = None;
        self.needs_pose_update = true;
    }

    fn animation_panel(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new(format!("Animation ({})", self.anim.entries.len()))
            .default_open(true)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    if ui.button("Add animations…").clicked() {
                        self.add_animations_dialog();
                    }
                    if let Some(f) = &self.anim.folder {
                        if ui.small_button("Open folder").clicked() {
                            open_in_explorer(f);
                        }
                    }
                    if ui.small_button("Refresh").clicked() {
                        self.anim.refresh();
                    }
                });
                if self.anim.entries.is_empty() {
                    ui.label(
                        RichText::new(
                            "Add Mixamo animations (FBX, \"With Skin\", 30 fps) to test the clothes in motion.",
                        )
                        .weak()
                        .small(),
                    );
                    return;
                }
                let mut pick = None;
                egui::ScrollArea::vertical()
                    .id_salt("anim list")
                    .max_height(150.0)
                    .show(ui, |ui| {
                        for (i, e) in self.anim.entries.iter().enumerate() {
                            let mut label = RichText::new(&e.name);
                            if e.error.is_some() {
                                label = label.color(Color32::from_rgb(230, 110, 90));
                            }
                            let r = ui.selectable_label(self.anim.current == Some(i), label);
                            let r = match &e.error {
                                Some(err) => r.on_hover_text(err),
                                None => r,
                            };
                            if r.clicked() {
                                pick = Some(i);
                            }
                        }
                    });
                if let Some(i) = pick {
                    if self.scene.is_none() {
                        self.error = Some("Open a body first, then play an animation on it.".into());
                    } else {
                        let sk = self.scene.as_ref().map(|s| &s.skeleton);
                        self.anim.select(i, sk);
                        self.anim_dirty = true;
                    }
                }
                if self.anim.loading.is_some() {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Loading animation…");
                    });
                }
                if self.anim.retarget.is_none() {
                    return;
                }
                let d = self.anim.duration();
                ui.horizontal(|ui| {
                    let label = if self.anim.playing { "Pause" } else { "Play" };
                    if ui.button(label).clicked() {
                        if !self.anim.playing && self.anim.time >= d {
                            self.anim.time = 0.0;
                        }
                        self.anim.playing = !self.anim.playing;
                    }
                    if ui
                        .button("Stop")
                        .on_hover_text("Back to the rest pose")
                        .clicked()
                    {
                        self.stop_animation();
                    }
                    ui.add(
                        egui::DragValue::new(&mut self.anim.speed)
                            .speed(0.02)
                            .range(0.05..=4.0)
                            .prefix("speed ×"),
                    );
                });
                if self.anim.retarget.is_none() {
                    return;
                }
                let mut t = self.anim.time;
                if ui
                    .add(egui::Slider::new(&mut t, 0.0..=d.max(0.001)).text("s"))
                    .changed()
                {
                    self.anim.time = t;
                    self.anim.playing = false;
                    self.anim_dirty = true;
                }
                ui.horizontal(|ui| {
                    ui.checkbox(&mut self.anim.looping, "Loop");
                    if ui
                        .checkbox(&mut self.anim.in_place, "In place")
                        .on_hover_text("Keep the body over the same spot instead of walking off.")
                        .changed()
                    {
                        self.anim_dirty = true;
                    }
                });
                if let Some(r) = &self.anim.retarget {
                    let text = format!("{} bones matched", r.matched);
                    if r.matched < 20 {
                        ui.label(
                            RichText::new(format!(
                                "{text}: this doesn't look like a Mixamo skeleton."
                            ))
                            .color(Color32::from_rgb(220, 170, 80))
                            .small(),
                        );
                    } else {
                        ui.label(RichText::new(text).weak().small());
                    }
                }

                // ---- Clipping ----
                ui.add_space(4.0);
                ui.checkbox(&mut self.anim.show_clipping, "Show clipping (red marks)")
                    .on_hover_text("Marks every point where a skinned garment pokes through the skin.");
                let skinned = self.garments.iter().any(|g| g.status == GarmentStatus::Skinned);
                if !skinned {
                    ui.label(
                        RichText::new("Auto skin a garment to check it for clipping.")
                            .weak()
                            .small(),
                    );
                    return;
                }
                if self.anim.show_clipping {
                    let n = self.anim.clip_points.len();
                    let (text, color) = if n == 0 {
                        ("No clipping on this frame".to_string(), Color32::from_rgb(120, 210, 120))
                    } else {
                        (format!("{n} points poking through on this frame"), Color32::from_rgb(235, 90, 80))
                    };
                    ui.label(RichText::new(text).color(color).small());
                }
                if self.anim.scan.is_some() {
                    ui.add(egui::ProgressBar::new(self.anim.scan_progress).text("Scanning…"));
                } else if ui
                    .button("Scan whole animation")
                    .on_hover_text("Checks every other frame for clipping and lists the worst moments.")
                    .clicked()
                {
                    if let Some(scene) = &self.scene {
                        self.anim.start_scan(scene);
                    }
                }
                let mut jump = None;
                if let Some(result) = &self.anim.scan_result {
                    let bad = result.iter().filter(|f| f.points > 0).count();
                    if bad == 0 {
                        ui.label(
                            RichText::new(format!("No clipping in any of {} frames checked.", result.len()))
                                .color(Color32::from_rgb(120, 210, 120)),
                        );
                    } else {
                        ui.label(
                            RichText::new(format!("Clipping in {bad} of {} frames checked. Worst:", result.len()))
                                .color(Color32::from_rgb(235, 90, 80)),
                        );
                        let mut worst: Vec<_> = result.iter().filter(|f| f.points > 0).collect();
                        worst.sort_by_key(|f| std::cmp::Reverse(f.points));
                        ui.horizontal_wrapped(|ui| {
                            for f in worst.iter().take(6) {
                                if ui
                                    .small_button(format!("{:.2}s · {}", f.time, f.points))
                                    .on_hover_text("Go to this moment")
                                    .clicked()
                                {
                                    jump = Some(f.time);
                                }
                            }
                        });
                        ui.label(
                            RichText::new("Fix it with Grab / Inflate (reset the pose first), a bigger gap, or less tightness, then scan again.")
                                .weak()
                                .small(),
                        );
                    }
                }
                if let Some(t) = jump {
                    self.anim.time = t;
                    self.anim.playing = false;
                    self.anim_dirty = true;
                }
            });
    }

    // ---- Viewport tools (gizmo and brushes) ------------------------------------

    fn active_gizmo(&self) -> Option<Gizmo> {
        if !self.tool.is_gizmo() || self.scene.is_none() {
            return None;
        }
        let g = self.garments.get(self.selected_garment)?;
        Some(Gizmo::new(
            self.tool,
            g.gizmo_center(),
            g.placement.rotation,
            &self.camera,
        ))
    }

    /// The point on the selected garment under screen point `mouse`.
    fn brush_raycast(&self, mouse: Vec2, rect: Vec2) -> Option<RayHit> {
        if !self.tool.is_brush() || !self.at_bind_pose() {
            return None;
        }
        let g = self.garments.get(self.selected_garment)?;
        let part = self.scene.as_ref()?.parts.get(g.part)?;
        if !part.visible {
            return None;
        }
        let (o, d) = self.camera.ray(mouse, rect);
        sculpt::raycast(&part.mesh.positions, &part.mesh.indices, o, d)
    }

    fn tool_shortcuts(&mut self, ui: &egui::Ui) {
        if ui.ctx().egui_wants_keyboard_input() {
            return;
        }
        let undo = ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Z));
        if undo {
            self.undo_step();
        }
        ui.input(|i| {
            if i.modifiers.command || i.modifiers.alt {
                return;
            }
            for t in Tool::PLACE
                .iter()
                .chain(&Tool::SCULPT)
                .chain([&Tool::Camera])
            {
                if i.key_pressed(t.key()) {
                    self.tool = *t;
                }
            }
            if i.key_pressed(egui::Key::OpenBracket) {
                self.brush.radius = (self.brush.radius / 1.15).max(0.005);
            }
            if i.key_pressed(egui::Key::CloseBracket) {
                self.brush.radius = (self.brush.radius * 1.15).min(0.4);
            }
        });
    }

    /// Gizmo and brush input. Returns true if the camera may use this drag.
    fn garment_tools(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        rect: egui::Rect,
    ) -> bool {
        let rect_size = Vec2::new(rect.width(), rect.height());
        let local = |p: egui::Pos2| Vec2::new(p.x - rect.min.x, p.y - rect.min.y);
        let hover = response.hover_pos().map(local);
        let pointer = response.interact_pointer_pos().map(local);
        let (alt, ctrl) = ui.input(|i| (i.modifiers.alt, i.modifiers.command));
        let gizmo = self.active_gizmo();

        // What's under the mouse.
        if self.gizmo_drag.is_none() {
            self.gizmo_hot = match (&gizmo, hover) {
                (Some(g), Some(m)) if !alt => g.pick(&self.camera, m, rect_size),
                _ => None,
            };
        }
        if self.stroke.is_none() {
            self.brush_hit = hover
                .filter(|_| !alt)
                .and_then(|m| self.brush_raycast(m, rect_size));
        }

        // Start a gizmo drag or a brush stroke (Alt + drag always orbits).
        if response.drag_started_by(PointerButton::Primary) && !alt {
            let origin = ui.input(|i| i.pointer.press_origin()).map(local);
            let gi = self.selected_garment;
            if let (Some(g), Some(m)) = (&gizmo, origin) {
                if let Some(h) = g.pick(&self.camera, m, rect_size) {
                    self.undo_last = None;
                    self.gizmo_drag = Some(tools::begin_drag(g, h, m, self.garments[gi].placement));
                    self.gizmo_hot = Some(h);
                }
            }
            if self.gizmo_drag.is_none() {
                if let Some(hit) = origin.and_then(|m| self.brush_raycast(m, rect_size)) {
                    self.undo_last = None;
                    self.remember(gi, "sculpt");
                    if let Some(scene) = &self.scene {
                        let mesh = &scene.parts[self.garments[gi].part].mesh;
                        self.stroke = Some(match self.tool {
                            Tool::Grab => Stroke::Grab {
                                grab: GrabStroke::begin(
                                    &mesh.positions,
                                    hit.point,
                                    self.brush.radius,
                                ),
                                start: hit.point,
                            },
                            _ => Stroke::Paint {
                                graph: WeldGraph::new(&mesh.positions, &mesh.indices, 1e-4),
                            },
                        });
                        self.brush_hit = Some(hit);
                    }
                }
            }
        }

        // Keep dragging.
        if response.dragged_by(PointerButton::Primary) {
            if let Some(m) = pointer {
                let moved = self
                    .gizmo_drag
                    .as_ref()
                    .map(|d| d.update(&self.camera, m, rect_size));
                if let Some(p) = moved {
                    self.set_placement(self.selected_garment, p, "gizmo");
                }
                if self.stroke.is_some() {
                    self.stroke_step(m, rect_size, ctrl);
                }
            }
        }

        // Let go.
        let released = !ui.input(|i| i.pointer.primary_down());
        if released || response.drag_stopped() {
            self.gizmo_drag = None;
            if self.stroke.take().is_some() {
                self.finish_stroke();
            }
        }
        let busy = self.gizmo_drag.is_some() || self.stroke.is_some();
        if busy || self.brush_hit.is_some() {
            ui.ctx().request_repaint();
        }
        !busy
    }

    /// One frame of a brush stroke with the mouse at `m`.
    fn stroke_step(&mut self, m: Vec2, rect_size: Vec2, ctrl: bool) {
        let Some(scene) = &mut self.scene else { return };
        let Some(g) = self.garments.get(self.selected_garment) else {
            return;
        };
        let part = g.part;
        let mesh = &mut scene.parts[part].mesh;
        let radius = self.brush.radius;
        let strength = self.brush.strength;
        match &self.stroke {
            Some(Stroke::Grab { grab, start }) => {
                let Some(now) = self.camera.screen_to_plane(m, rect_size, *start) else {
                    return;
                };
                grab.apply(&mut mesh.positions, now - *start);
                if let Some(h) = &mut self.brush_hit {
                    h.point = now;
                }
            }
            Some(Stroke::Paint { graph }) => {
                let (o, d) = self.camera.ray(m, rect_size);
                let Some(hit) = sculpt::raycast(&mesh.positions, &mesh.indices, o, d) else {
                    self.brush_hit = None;
                    return;
                };
                self.brush_hit = Some(hit);
                match self.tool {
                    Tool::Relax => sculpt::relax(
                        &mut mesh.positions,
                        graph,
                        hit.point,
                        radius,
                        strength * 0.35,
                    ),
                    Tool::Inflate => {
                        let amount = strength * radius * 0.04 * if ctrl { -1.0 } else { 1.0 };
                        sculpt::inflate(
                            &mut mesh.positions,
                            &mesh.indices,
                            graph,
                            hit.point,
                            radius,
                            amount,
                        );
                    }
                    _ => {}
                }
            }
            None => return,
        }
        mesh.compute_normals();
        self.dirty_part = Some(part);
    }

    fn finish_stroke(&mut self) {
        if !self.brush.keep_outside {
            return;
        }
        let Some(scene) = &mut self.scene else { return };
        if self.body_cache.is_none() {
            self.body_cache = garment::body_bvh(scene).map(|(_, b)| b);
        }
        let (Some(body), Some(g)) = (&self.body_cache, self.garments.get(self.selected_garment))
        else {
            return;
        };
        garment::keep_outside(scene, g, body, self.fit_settings.gap);
        self.dirty_part = Some(g.part);
    }

    /// Tool buttons floating in the top-left corner of the viewport.
    fn viewport_toolbar(&mut self, ui: &egui::Ui, rect: egui::Rect) {
        if self.garments.is_empty() {
            return;
        }
        egui::Area::new(ui.scope_id().with("viewport tools"))
            .fixed_pos(rect.min + egui::vec2(8.0, 8.0))
            .show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let all = [Tool::Camera]
                            .into_iter()
                            .chain(Tool::PLACE)
                            .chain(Tool::SCULPT);
                        for (n, t) in all.enumerate() {
                            if n == 1 || n == 4 {
                                ui.separator();
                            }
                            if ui
                                .selectable_label(self.tool == t, t.label())
                                .on_hover_text(format!("{}  (key: {:?})", t.help(), t.key()))
                                .clicked()
                            {
                                self.tool = t;
                            }
                        }
                    });
                });
            });
        if self.tool != Tool::Camera {
            let text = if self.tool.is_brush() && !self.at_bind_pose() {
                "Reset the pose to sculpt.".to_string()
            } else {
                format!("{}   ·   Alt + drag: orbit", self.tool.help())
            };
            ui.painter().text(
                rect.left_bottom() + egui::vec2(10.0, -10.0),
                egui::Align2::LEFT_BOTTOM,
                text,
                egui::FontId::proportional(14.0),
                Color32::from_gray(200),
            );
        }
    }

    // ---- Viewport ------------------------------------------------------------

    fn viewport_ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let (rect, response) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
        let rect_size = Vec2::new(rect.width(), rect.height());

        self.tool_shortcuts(ui);
        let camera_free = self.garment_tools(ui, &response, rect);

        // Camera controls.
        let delta = response.drag_delta();
        if !camera_free {
            // A gizmo drag or brush stroke is using the mouse.
        } else if response.dragged_by(PointerButton::Primary) {
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
        if self.needs_upload {
            if let (Some(scene), Some(viewport)) = (&self.scene, &mut self.viewport) {
                viewport.set_scene(&rs.device, scene, &joint_world);
                self.needs_upload = false;
                self.needs_pose_update = false;
            }
        }
        if let Some(send) = self.pending_save.take() {
            self.save_outfit(rs, &joint_world, send);
        }
        let gizmo_now = self.active_gizmo();
        let Some(viewport) = &mut self.viewport else {
            return;
        };

        if let Some(scene) = &self.scene {
            if self.needs_upload {
                viewport.set_scene(&rs.device, scene, &joint_world);
                self.needs_upload = false;
                self.needs_pose_update = false;
                self.dirty_part = None;
            } else if self.needs_pose_update {
                viewport.update_pose(&rs.queue, scene, &joint_world);
                self.needs_pose_update = false;
            }
            if let Some(i) = self.dirty_part.take() {
                viewport.update_part(&rs.queue, scene, i, &joint_world);
            }
        }

        let mut overlay_lines = match (&self.scene, self.show_skeleton) {
            (Some(scene), true) => {
                skeleton_lines(scene, &joint_world, self.selected_joint, &self.camera)
            }
            _ => Vec::new(),
        };
        if let Some(g) = gizmo_now {
            let hot = self
                .gizmo_drag
                .as_ref()
                .map(|d| d.handle)
                .or(self.gizmo_hot);
            overlay_lines.extend(g.lines(hot));
        }
        if !self.anim.clip_points.is_empty() {
            let red = Vec4::new(1.0, 0.15, 0.1, 1.0);
            let s = self.camera.distance * 0.004;
            for p in &self.anim.clip_points {
                for axis in [Vec3::X, Vec3::Y, Vec3::Z] {
                    overlay_lines.push(LineVertex::new(*p - axis * s, red));
                    overlay_lines.push(LineVertex::new(*p + axis * s, red));
                }
            }
        }
        if let Some(h) = &self.brush_hit {
            overlay_lines.extend(tools::brush_circle(
                h.point,
                h.normal,
                self.brush.radius,
                self.stroke.is_some(),
            ));
        }
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

        self.viewport_toolbar(ui, rect);

        if self.scene.is_none() && self.loading.is_none() {
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "File → Open body…   or drop a file here",
                egui::FontId::proportional(18.0),
                Color32::from_gray(140),
            );
        }
    }
}

impl eframe::App for ThreadSmithApp {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll_bridge();
        self.poll_loading(&ctx);
        self.anim_update(&ctx);
        self.handle_dropped_files(&ctx);

        egui::Panel::top("menu").show(ui, |ui| self.menu_bar(ui));
        egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui));
        egui::Panel::left("scene")
            .resizable(true)
            .default_size(280.0)
            .show(ui, |ui| {
                self.library_panel(ui);
                ui.separator();
                self.scene_panel(ui);
            });
        egui::Panel::right("pose")
            .resizable(true)
            .default_size(300.0)
            .show(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    self.garment_panel(ui);
                    ui.separator();
                    self.animation_panel(ui);
                    ui.separator();
                    self.pose_panel(ui);
                });
            });
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

/// Open a folder in Windows Explorer (or the system file browser elsewhere).
fn open_in_explorer(path: &std::path::Path) {
    #[cfg(windows)]
    let cmd = "explorer";
    #[cfg(target_os = "macos")]
    let cmd = "open";
    #[cfg(all(unix, not(target_os = "macos")))]
    let cmd = "xdg-open";
    let _ = std::process::Command::new(cmd).arg(path).spawn();
}
