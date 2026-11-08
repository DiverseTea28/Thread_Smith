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
use crate::generate::{GenOutput, GeneratePanel};
use crate::ui_icons::{self, UiIcons};
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

/// The four workspaces (tabs along the top, like Blender's).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Workspace {
    /// Open a body and clothing files, pick from the outfit library.
    Import,
    /// Make clothing with AI.
    Generate,
    /// Place, fit, sculpt and skin; test with poses and animations.
    Fitment,
    /// Save to the library and send to Unreal.
    Export,
}

impl Workspace {
    const ALL: [Workspace; 4] = [Self::Import, Self::Generate, Self::Fitment, Self::Export];

    fn label(self) -> &'static str {
        match self {
            Self::Import => "Import",
            Self::Generate => "Generate",
            Self::Fitment => "Fitment",
            Self::Export => "Export",
        }
    }

    fn key(self) -> egui::Key {
        match self {
            Self::Import => egui::Key::Num1,
            Self::Generate => egui::Key::Num2,
            Self::Fitment => egui::Key::Num3,
            Self::Export => egui::Key::Num4,
        }
    }
}

/// Something waiting on "save your changes first?".
#[derive(Clone)]
enum PendingAction {
    Close,
    New,
    Open(PathBuf),
}

/// A project being saved or opened in the background.
enum ProjectJob {
    Saving { path: PathBuf, rx: Receiver<anyhow::Result<()>>, then: Option<PendingAction> },
    Opening { path: PathBuf, rx: Receiver<anyhow::Result<crate::project::Project>> },
}

const RECENT_KEY: &str = "threadsmith_recent_projects";

/// Result of a background file load.
type LoadResult = (PathBuf, LoadKind, anyhow::Result<Scene>);

/// How a garment was before a change, for Undo.
struct UndoStep {
    garment: usize,
    mesh: Mesh,
    /// Only kept when the triangles change (Reduce triangles).
    original: Option<Mesh>,
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
    /// Relax / Push/Pull / Shrink: dabs every frame while the mouse is held.
    Paint { graph: WeldGraph },
}

/// The lattice (cage) being edited on the selected garment.
struct LatticeEdit {
    part: usize,
    /// The garment's points when the lattice was made.
    rest: Vec<Vec3>,
    lattice: ts_fit::Lattice,
    /// What the lattice last produced (if the garment changes some other
    /// way, the lattice starts over around the new shape).
    result: Vec<Vec3>,
    hot: Option<usize>,
    /// Point being dragged: (index, where it started, where the mouse grabbed it).
    drag: Option<(usize, Vec3, Vec3)>,
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
    /// Remove the body's skin under the clothes on export (and preview it on the Export tab).
    hide_body: bool,
    /// Cached covered-triangle list: (signature, body part, hidden triangles).
    body_cover: Option<(u64, usize, Vec<bool>)>,
    /// Signature of the hidden triangles currently shown in the viewport.
    body_preview: Option<u64>,
    needs_pose_update: bool,

    garments: Vec<GarmentWork>,
    selected_garment: usize,
    fit_settings: FitSettings,
    skin_settings: SkinSettings,

    loading: Option<(String, Receiver<LoadResult>)>,
    /// For screenshots and testing: a garment to add once the body is open.
    dev_garment: Option<PathBuf>,
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
    /// A part whose material changed (re-upload its look).
    dirty_material: Option<usize>,
    lattice: Option<LatticeEdit>,
    /// Points along each side of a new lattice (2..=5).
    lattice_size: usize,
    /// Share of triangles to keep with "Reduce triangles".
    detail_ratio: f32,
    /// The body's surface for "keep outside the skin" (built when first needed).
    body_cache: Option<TriBvh>,

    anim: AnimPlayer,
    /// The animation time or settings changed: work out the pose again.
    anim_dirty: bool,
    /// Head file to add to the next body that loads (sent by Unreal).
    pending_head: Option<PathBuf>,

    /// The AI "Generate clothing" window.
    generate: GeneratePanel,
    icons: UiIcons,
    workspace: Workspace,
    /// The open project file (None until saved or opened).
    project_path: Option<PathBuf>,
    /// Changed since the last save.
    project_dirty: bool,
    recent_projects: Vec<PathBuf>,
    project_job: Option<ProjectJob>,
    /// Asking "save changes?" before doing this.
    pending_action: Option<PendingAction>,
    /// The user chose to close without saving.
    allow_close: bool,
    window_title: String,
    /// Small pictures of textures for the Layers list (by content hash).
    layer_thumbs: std::collections::HashMap<u64, Option<egui::TextureHandle>>,
    /// Outfit library thumbnails, loaded when first shown.
    library_thumbs: std::collections::HashMap<PathBuf, Option<egui::TextureHandle>>,
    /// An AI garment file being loaded: its preset and prompt, for placing it.
    pending_ai: Option<(ts_gen::Garment, String)>,
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
        let gen_settings: ts_gen::EngineSettings = cc
            .storage
            .and_then(|s| eframe::get_value(s, crate::generate::SETTINGS_KEY))
            .unwrap_or_default();
        let generate = GeneratePanel::new(&cc.egui_ctx, gen_settings);
        let viewport = cc
            .wgpu_render_state
            .as_ref()
            .map(|rs| Viewport::new(&rs.device, &rs.queue));
        let mut app = Self {
            scene: None,
            pose: Pose::default(),
            pose_euler: Vec::new(),
            selected_joint: None,
            joint_filter: String::new(),
            camera: OrbitCamera::default(),
            show_shaded: true,
            show_wireframe: false,
            show_skeleton: false,
            show_grid: true,
            viewport,
            needs_upload: false,
            hide_body: true,
            body_cover: None,
            body_preview: None,
            needs_pose_update: false,
            garments: Vec::new(),
            selected_garment: 0,
            fit_settings: FitSettings::default(),
            skin_settings: SkinSettings::default(),
            loading: None,
            dev_garment: std::env::var_os("THREADSMITH_GARMENT").map(PathBuf::from),
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
            dirty_material: None,
            lattice: None,
            lattice_size: 3,
            detail_ratio: 0.5,
            body_cache: None,
            anim: AnimPlayer::new(Some(Library::default_location().join("Animations"))),
            anim_dirty: false,
            pending_head: None,
            generate,
            icons: UiIcons::load(&cc.egui_ctx),
            workspace: Workspace::Import,
            project_path: None,
            project_dirty: false,
            recent_projects: cc
                .storage
                .and_then(|s| eframe::get_value::<Vec<PathBuf>>(s, RECENT_KEY))
                .unwrap_or_default()
                .into_iter()
                .filter(|p| p.is_file())
                .collect(),
            project_job: None,
            pending_action: None,
            allow_close: false,
            window_title: String::new(),
            layer_thumbs: Default::default(),
            library_thumbs: Default::default(),
            pending_ai: None,
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
        // For screenshots and testing: start on a given tab.
        if let Ok(tab) = std::env::var("THREADSMITH_TAB") {
            if let Some(w) = Workspace::ALL.into_iter().find(|w| w.label().eq_ignore_ascii_case(&tab)) {
                app.workspace = w;
            }
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
                let before = self.garments.len();
                self.add_garment_file(file, &path);
                if let Some((preset, prompt)) = self.pending_ai.take() {
                    self.place_ai_garments(before, &preset, &prompt);
                }
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
                // A new body starts a new project (so Save doesn't overwrite the old one).
                self.project_path = None;
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
        self.project_dirty = true;
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
                self.project_dirty = true;
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
        // A picture dropped on the Generate tab is its reference picture.
        if self.workspace == Workspace::Generate {
            if let Some(p) = dropped.iter().find(|p| GeneratePanel::is_picture(p)) {
                self.generate.set_reference(ctx, p.clone());
                return;
            }
        }
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
        let cover = if self.hide_body {
            let parts: Vec<usize> = self
                .garments
                .iter()
                .filter(|g| g.include && g.status == GarmentStatus::Skinned)
                .map(|g| g.part)
                .collect();
            self.body_cover(&parts)
        } else {
            None
        };
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
                outline_part: None,
                shadow: None,
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
                material: scene.parts[i].material.as_ref(),
            })
            .collect();
        let mut outfit = match library.save(
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
        if let Some((b, hidden)) = cover.filter(|(_, h)| h.iter().any(|&x| x)) {
            let body = ts_fit::without_triangles(&scene.parts[b].mesh, &hidden);
            let export = ts_io::ExportMesh {
                name: &scene.parts[b].name,
                mesh: &body,
                color: [0.8, 0.8, 0.8],
                material: scene.parts[b].material.as_ref(),
            };
            if let Err(e) = library.save_trimmed_body(&mut outfit, &scene.skeleton, &export) {
                log::warn!("Couldn't save the trimmed body: {e:#}");
                self.status.push_str(" (The body under the clothes couldn't be hidden.)");
            }
        }
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
                if ui.button("New project").clicked() {
                    ui.close();
                    self.request(PendingAction::New);
                }
                if ui.button("Open project…     Ctrl+O").clicked() {
                    ui.close();
                    self.open_project_dialog();
                }
                ui.add_enabled_ui(!self.recent_projects.is_empty(), |ui| {
                    ui.menu_button("Open recent", |ui| {
                        for p in self.recent_projects.clone() {
                            if ui.button(project_label(&p)).on_hover_text(p.display().to_string()).clicked() {
                                ui.close();
                                self.request(PendingAction::Open(p));
                            }
                        }
                    });
                });
                if ui.add_enabled(self.scene.is_some(), egui::Button::new("Save project       Ctrl+S")).clicked() {
                    ui.close();
                    self.save_project(false, None);
                }
                if ui.add_enabled(self.scene.is_some(), egui::Button::new("Save project as…  Ctrl+Shift+S")).clicked() {
                    ui.close();
                    self.save_project(true, None);
                }
                ui.separator();
                if ui.add(ui_icons::button(&self.icons.import, "Open body…")).clicked() {
                    ui.close();
                    self.open_dialog(LoadKind::Body);
                }
                if ui
                    .add_enabled(self.has_body(), ui_icons::button(&self.icons.import, "Add garment…"))
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
                    self.project_path = None;
                    self.project_dirty = false;
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

        // ---- Layers: one row per mesh, with a picture, show / hide and delete ----
        ui.label(RichText::new("Layers").strong());
        let mut changed_visibility = false;
        let mut select = None;
        let mut delete = None;
        let ctx = ui.ctx().clone();
        for (i, part) in scene.parts.iter_mut().enumerate() {
            if part.role == PartRole::Reference {
                continue; // helper bodies stay out of the list
            }
            let garment = self.garments.iter().position(|g| g.part == i);
            let selected = garment.is_some() && garment == Some(self.selected_garment);
            let row = ui.horizontal(|ui| {
                ui.set_min_height(44.0);
                // Picture: the texture, or the colour swatch.
                let (rect, _) = ui.allocate_exact_size(egui::vec2(40.0, 40.0), Sense::hover());
                let thumb = part
                    .material
                    .as_ref()
                    .and_then(|m| m.base_color_texture.as_ref())
                    .and_then(|t| layer_thumb(&ctx, &mut self.layer_thumbs, t));
                match thumb {
                    Some(t) => {
                        ui.painter().image(t.id(), rect, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
                    }
                    None => {
                        let c = part.material.as_ref().map(|m| m.base_color).filter(|c| c[..3].iter().any(|v| *v < 0.98));
                        let col = match (c, part.role) {
                            (Some(c), _) => Color32::from_rgb((c[0].powf(1.0 / 2.2) * 255.0) as u8, (c[1].powf(1.0 / 2.2) * 255.0) as u8, (c[2].powf(1.0 / 2.2) * 255.0) as u8),
                            (None, PartRole::Garment) => Color32::from_rgb(200, 120, 90),
                            _ => Color32::from_rgb(150, 160, 180),
                        };
                        ui.painter().rect_filled(rect, 4.0, col);
                    }
                }
                ui.vertical(|ui| {
                    let name = RichText::new(&part.name);
                    let name = if selected { name.strong().color(Color32::from_rgb(255, 170, 60)) } else { name };
                    if ui.add(egui::Label::new(name).truncate().sense(Sense::click())).clicked() {
                        if let Some(g) = garment {
                            select = Some(g);
                        }
                    }
                    let kind = match part.role {
                        PartRole::Body => "body",
                        PartRole::Head => "head",
                        PartRole::Garment => match garment.map(|g| self.garments[g].status) {
                            Some(GarmentStatus::Skinned) => "garment · skinned",
                            Some(GarmentStatus::Fitted) => "garment · fitted",
                            _ => "garment",
                        },
                        PartRole::Reference => "",
                    };
                    ui.label(RichText::new(format!("{kind} · {} tris", fmt_count(part.mesh.triangle_count()))).weak().small());
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if garment.is_some()
                        && ui
                            .add(egui::Button::image(ui_icons::small(&self.icons.trash)).frame(false))
                            .on_hover_text("Delete this garment")
                            .clicked()
                    {
                        delete = Some(i);
                    }
                    let eye = if part.visible { &self.icons.eye } else { &self.icons.eye_off };
                    if ui
                        .add(egui::Button::image(ui_icons::small(eye)).frame(false))
                        .on_hover_text(if part.visible { "Hide" } else { "Show" })
                        .clicked()
                    {
                        part.visible = !part.visible;
                        changed_visibility = true;
                    }
                });
            });
            if selected {
                ui.painter().rect_stroke(row.response.rect.expand(2.0), 4.0, (1.0, Color32::from_rgb(255, 150, 40)), egui::StrokeKind::Inside);
            }
        }
        if changed_visibility {
            if let Some(v) = &mut self.viewport {
                v.set_visibility(scene);
            }
        }
        if let Some(g) = select {
            self.selected_garment = g;
        }
        if let Some(i) = delete {
            self.delete_part(i);
            return;
        }
        let Some(scene) = &mut self.scene else { return };
        ui.add_space(6.0);

        let skeleton = &scene.skeleton;
        egui::CollapsingHeader::new(format!("Skeleton ({} bones)", skeleton.len()))
            .default_open(false)
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
        self.project_dirty = true;
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
            original: (kind == "detail").then(|| g.original.clone()),
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
        let topology_changed = step.mesh.indices.len() != scene.parts[g.part].mesh.indices.len();
        scene.parts[g.part].mesh = step.mesh;
        if let Some(o) = step.original {
            g.original = o;
        }
        if topology_changed {
            self.needs_upload = true;
            self.lattice = None;
        }
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
        if self.garments.is_empty() {
            ui.label(RichText::new("No clothing yet. Add some on the Import tab, or make some on the Generate tab.").weak());
            ui.horizontal(|ui| {
                if ui.button("Go to Import").clicked() {
                    self.set_workspace(Workspace::Import);
                }
                if ui.button("Go to Generate").clicked() {
                    self.set_workspace(Workspace::Generate);
                }
            });
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
        self.detail_section(ui, gi);
        ui.add_space(6.0);
        self.material_section(ui, gi);
    }

    /// Poly count: make the garment lighter (keeps UVs, skin and openings).
    fn detail_section(&mut self, ui: &mut egui::Ui, gi: usize) {
        let Some(tris) = self.scene.as_ref().zip(self.garments.get(gi)).map(|(s, g)| s.parts[g.part].mesh.triangle_count()) else { return };
        egui::CollapsingHeader::new(RichText::new("Poly count").strong())
            .id_salt("detail section")
            .default_open(false)
            .show(ui, |ui| {
                ui.label(format!("{tris} triangles"));
                let mut pct = self.detail_ratio * 100.0;
                if ui.add(egui::Slider::new(&mut pct, 5.0..=95.0).suffix("%").text("keep"))
                    .on_hover_text("How many of the triangles to keep. Sleeve and neck openings stay exactly as they are.")
                    .changed()
                {
                    self.detail_ratio = pct / 100.0;
                }
                ui.label(RichText::new(format!("About {} after.", (tris as f32 * self.detail_ratio) as usize)).weak().small());
                if ui.button("Reduce triangles").clicked() {
                    self.reduce_triangles(gi);
                }
            });
    }

    fn reduce_triangles(&mut self, gi: usize) {
        self.undo_last = None;
        self.remember(gi, "detail");
        let ratio = self.detail_ratio;
        let (Some(scene), Some(g)) = (&mut self.scene, self.garments.get_mut(gi)) else { return };
        let mesh = &scene.parts[g.part].mesh;
        let before = mesh.triangle_count();
        let s = ts_fit::simplify::simplify(mesh, ratio, true);
        let new = ts_fit::simplify::apply(mesh, &s);
        if g.original.positions.len() == mesh.positions.len() {
            g.original = ts_fit::simplify::apply(&g.original, &s);
        }
        let after = new.triangle_count();
        scene.parts[g.part].mesh = new;
        self.needs_upload = true;
        self.lattice = None;
        self.body_cover = None;
        self.status = format!("Triangles: {before} down to {after}. Ctrl+Z to undo.");
    }

    /// Colour, shine and texture maps of the selected garment.
    fn material_section(&mut self, ui: &mut egui::Ui, gi: usize) {
        let Some(part_index) = self.garments.get(gi).map(|g| g.part) else { return };
        let Some(scene) = &mut self.scene else { return };
        let Some(part) = scene.parts.get_mut(part_index) else { return };
        let has_uvs = part.mesh.uvs.len() == part.mesh.positions.len();
        let mut changed = false;
        egui::CollapsingHeader::new(RichText::new("Material").strong())
            .id_salt("material section")
            .default_open(true)
            .show(ui, |ui| {
                let Some(m) = &mut part.material else {
                    ui.label(RichText::new("This garment has no material yet.").weak().small());
                    if ui.button("Add a material").clicked() {
                        part.material = Some(ts_core::Material::with_color(format!("M_{}", part.name), [0.8, 0.8, 0.8]));
                        changed = true;
                    }
                    return;
                };
                egui::Grid::new("material grid").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
                    ui.label("Colour").on_hover_text("Multiplies the texture (white keeps it as it is).");
                    let mut rgb = [m.base_color[0], m.base_color[1], m.base_color[2]];
                    if ui.color_edit_button_rgb(&mut rgb).changed() {
                        m.base_color[..3].copy_from_slice(&rgb);
                        changed = true;
                    }
                    ui.end_row();
                    ui.label("Hue");
                    changed |= ui.add(egui::Slider::new(&mut m.hue, -180.0..=180.0).suffix("°")).changed();
                    ui.end_row();
                    ui.label("Saturation");
                    changed |= ui.add(egui::Slider::new(&mut m.saturation, 0.0..=2.0)).changed();
                    ui.end_row();
                    ui.label("Brightness");
                    changed |= ui.add(egui::Slider::new(&mut m.brightness, 0.0..=2.0)).changed();
                    ui.end_row();
                    ui.label("Roughness").on_hover_text("0 = shiny, 1 = matte.");
                    changed |= ui.add(egui::Slider::new(&mut m.roughness, 0.0..=1.0)).changed();
                    ui.end_row();
                    ui.label("Metal");
                    changed |= ui.add(egui::Slider::new(&mut m.metallic, 0.0..=1.0)).changed();
                    ui.end_row();
                });
                if ui.small_button("Reset colour").clicked() {
                    m.hue = 0.0;
                    m.saturation = 1.0;
                    m.brightness = 1.0;
                    m.base_color = [1.0, 1.0, 1.0, m.base_color[3]];
                    changed = true;
                }
                ui.add_space(4.0);
                ui.label(RichText::new("Texture maps").small().strong());
                for (label, normal) in [("Colour map", false), ("Normal map", true)] {
                    let slot = if normal { &mut m.normal_texture } else { &mut m.base_color_texture };
                    ui.horizontal(|ui| {
                        ui.label(label);
                        let name = slot.as_ref().map_or("none".to_string(), |t| t.name.clone());
                        ui.label(RichText::new(name).weak().small());
                    });
                    ui.horizontal(|ui| {
                        let replace = ui.add_enabled(has_uvs, egui::Button::new("Replace…").small());
                        let replace = replace.on_disabled_hover_text("This garment has no UVs, so it can't show a texture.");
                        if replace.clicked() {
                            if let Some(path) = rfd::FileDialog::new()
                                .set_title(format!("Pick a {}", label.to_lowercase()))
                                .add_filter("Images", &["png", "jpg", "jpeg", "webp", "tga", "bmp", "tif", "tiff"])
                                .pick_file()
                            {
                                match ts_io::load_texture(&path) {
                                    Ok(t) => {
                                        *slot = Some(t);
                                        changed = true;
                                    }
                                    Err(e) => self.error = Some(format!("{e:#}")),
                                }
                            }
                        }
                        if slot.is_some() && ui.small_button("Remove").clicked() {
                            *slot = None;
                            changed = true;
                        }
                    });
                }
                ui.label(
                    RichText::new("Colour changes are baked into the texture when you export. The normal map isn't shown here, but it goes to Unreal.")
                        .weak()
                        .small(),
                );
            });
        if changed {
            self.dirty_material = Some(part_index);
            self.project_dirty = true;
        }
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
                    let inner = garment::inner_parts(scene, &self.garments, gi);
                    let g = &mut self.garments[gi];
                    match garment::grow_to_clear_body(scene, g, &inner) {
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
        {
            let g = &mut self.garments[gi];
            let mut layer = g.layer;
            ui.horizontal(|ui| {
                ui.label("Layer");
                egui::ComboBox::from_id_salt("garment layer")
                    .selected_text(garment::LAYER_NAMES[(layer.clamp(1, 4) - 1) as usize])
                    .show_ui(ui, |ui| {
                        for (n, name) in garment::LAYER_NAMES.iter().enumerate() {
                            ui.selectable_value(&mut layer, n as u8 + 1, *name);
                        }
                    })
                    .response
                    .on_hover_text("Garments on higher layers are fitted over the ones below (a jacket over a shirt). Fit inner layers first.");
            });
            if layer != g.layer {
                g.layer = layer;
                self.project_dirty = true;
            }
            if let Some(scene) = &self.scene {
                let inner = garment::inner_parts(scene, &self.garments, gi);
                if !inner.is_empty() {
                    ui.label(RichText::new(format!("Fits over {} garment(s) on lower layers.", inner.len())).weak().small());
                }
            }
        }
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
            let inner = garment::inner_parts(scene, &self.garments, gi);
            let g = &mut self.garments[gi];
            if fit {
                match garment::fit(scene, g, &inner, &self.fit_settings) {
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
        ui.add_space(4.0);
        self.tool_buttons(ui, &Tool::SHAPE);
        if self.tool == Tool::Lattice {
            let mut size = self.lattice_size;
            ui.add(egui::Slider::new(&mut size, 2..=5).text("points per side"))
                .on_hover_text("2 bends the garment as a whole; 5 lets you shape smaller areas.");
            if size != self.lattice_size {
                // Keep what's been done and start a new cage around it.
                self.lattice_size = size;
                self.lattice = None;
            }
            ui.horizontal(|ui| {
                let moved = self.lattice.as_ref().is_some_and(|l| l.lattice.is_moved());
                if ui.add_enabled(moved, egui::Button::new("Undo lattice")).on_hover_text("Put the garment back the way it was before this cage.").clicked() {
                    self.reset_lattice();
                }
                if ui.add_enabled(moved, egui::Button::new("New cage")).on_hover_text("Keep the shape and start a fresh, square cage around it.").clicked() {
                    self.lattice = None;
                }
            });
        }
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
        ui.label(RichText::new("Save and send").strong());
        ui.add(egui::TextEdit::singleline(&mut self.outfit_name).hint_text("Outfit name"));
        egui::ComboBox::from_label("Type")
            .selected_text(self.outfit_category.clone())
            .show_ui(ui, |ui| {
                for c in CATEGORIES {
                    ui.selectable_value(&mut self.outfit_category, c.to_string(), *c);
                }
            });
        ui.horizontal(|ui| {
            if ui.add(ui_icons::button(&self.icons.wardrobe, "Save to library")).clicked() {
                self.pending_save = Some(false);
            }
            let send = ui.add_enabled(
                self.bridge.plugin_connected(),
                ui_icons::button(&self.icons.export, "Send to Unreal"),
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
        if ui
            .checkbox(&mut self.hide_body, "Hide body under clothes")
            .on_hover_text(
                "Removes the skin under the clothes so it can't poke through.\n\
                 Unreal gets a trimmed copy of the body; your original body isn't changed.",
            )
            .changed()
        {
            self.project_dirty = true;
        }
    }

    /// Signature of everything the covered-body result depends on.
    fn cover_signature(&self, parts: &[usize]) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        let Some(scene) = &self.scene else { return 0 };
        garment::body_part(scene).hash(&mut h);
        for &i in parts {
            i.hash(&mut h);
            if let Some(p) = scene.parts.get(i) {
                p.mesh.positions.len().hash(&mut h);
                for v in p.mesh.positions.iter().step_by(7) {
                    v.x.to_bits().hash(&mut h);
                    v.y.to_bits().hash(&mut h);
                    v.z.to_bits().hash(&mut h);
                }
            }
        }
        h.finish()
    }

    /// Body triangles covered by these garment parts (cached).
    fn body_cover(&mut self, parts: &[usize]) -> Option<(usize, Vec<bool>)> {
        if parts.is_empty() {
            return None;
        }
        let sig = self.cover_signature(parts);
        if let Some((s, b, hidden)) = &self.body_cover {
            if *s == sig {
                return Some((*b, hidden.clone()));
            }
        }
        let (b, hidden) = garment::covered_body(self.scene.as_ref()?, parts)?;
        self.body_cover = Some((sig, b, hidden.clone()));
        Some((b, hidden))
    }

    /// On the Export tab with "Hide body under clothes" on: what to hide.
    fn body_preview_wanted(&mut self) -> Option<(u64, usize, Vec<bool>)> {
        if self.workspace != Workspace::Export || !self.hide_body || self.needs_upload {
            return None;
        }
        let scene = self.scene.as_ref()?;
        let parts: Vec<usize> = self
            .garments
            .iter()
            .filter(|g| g.include && matches!(g.status, GarmentStatus::Fitted | GarmentStatus::Skinned))
            .filter(|g| scene.parts.get(g.part).is_some_and(|p| p.visible))
            .map(|g| g.part)
            .collect();
        let (b, hidden) = self.body_cover(&parts)?;
        Some((self.body_cover.as_ref()?.0, b, hidden))
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
                .chain(&Tool::SHAPE)
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
        if self.tool == Tool::Lattice {
            return self.lattice_input(ui, response, rect_size, hover, pointer, alt);
        }
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
                        if self.tool == Tool::Shrink {
                            let inner = garment::inner_parts(scene, &self.garments, gi);
                            self.body_cache = garment::collision_bvh(scene, &inner);
                        }
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

    /// Make (or remake) the lattice around the selected garment when needed.
    fn ensure_lattice(&mut self) {
        if self.tool != Tool::Lattice || !self.at_bind_pose() {
            self.lattice = None;
            return;
        }
        let Some(scene) = &self.scene else { return };
        let Some(part) = self.garments.get(self.selected_garment).map(|g| g.part) else {
            self.lattice = None;
            return;
        };
        let positions = &scene.parts[part].mesh.positions;
        let fresh = self
            .lattice
            .as_ref()
            .is_some_and(|l| l.part == part && (l.drag.is_some() || l.result == *positions));
        if fresh {
            return;
        }
        let Some(bounds) = ts_core::Bounds::from_points(positions.iter().copied()) else { return };
        self.lattice = Some(LatticeEdit {
            part,
            rest: positions.clone(),
            lattice: ts_fit::Lattice::new(bounds, self.lattice_size),
            result: positions.clone(),
            hot: None,
            drag: None,
        });
    }

    /// Put the garment back to how it was before the current cage.
    fn reset_lattice(&mut self) {
        if self.lattice.is_none() {
            return;
        }
        self.undo_last = None;
        let gi = self.selected_garment;
        self.remember(gi, "lattice");
        let (Some(l), Some(scene)) = (&mut self.lattice, &mut self.scene) else { return };
        let part = l.part;
        let rest = l.rest.clone();
        l.lattice.reset();
        l.result = rest.clone();
        let mesh = &mut scene.parts[part].mesh;
        mesh.positions = rest;
        mesh.compute_normals();
        self.dirty_part = Some(part);
        self.project_dirty = true;
    }

    /// Lattice tool input. Returns true if the camera may use this drag.
    fn lattice_input(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        rect_size: Vec2,
        hover: Option<Vec2>,
        pointer: Option<Vec2>,
        alt: bool,
    ) -> bool {
        self.ensure_lattice();
        let Some(l) = &mut self.lattice else { return true };
        if l.drag.is_none() {
            l.hot = hover
                .filter(|_| !alt)
                .and_then(|m| tools::pick_lattice_point(&l.lattice, &self.camera, m, rect_size));
        }
        if response.drag_started_by(PointerButton::Primary) && !alt {
            let origin = ui.input(|i| i.pointer.press_origin()).map(|p| Vec2::new(p.x - response.rect.min.x, p.y - response.rect.min.y));
            if let Some(i) = origin.and_then(|m| tools::pick_lattice_point(&l.lattice, &self.camera, m, rect_size)) {
                let start = l.lattice.points[i];
                let grab = origin.and_then(|m| self.camera.screen_to_plane(m, rect_size, start)).unwrap_or(start);
                l.drag = Some((i, start, grab));
                l.hot = Some(i);
                let gi = self.selected_garment;
                self.undo_last = None;
                self.remember(gi, "lattice");
            }
        }
        let Some(l) = &mut self.lattice else { return true };
        if let (Some((i, start, grab)), Some(m)) = (l.drag, pointer) {
            if response.dragged_by(PointerButton::Primary) {
                if let Some(now) = self.camera.screen_to_plane(m, rect_size, grab) {
                    l.lattice.points[i] = start + (now - grab);
                    if let Some(scene) = &mut self.scene {
                        let mesh = &mut scene.parts[l.part].mesh;
                        mesh.positions = l.lattice.deform(&l.rest);
                        mesh.compute_normals();
                        l.result = mesh.positions.clone();
                        self.dirty_part = Some(l.part);
                        self.project_dirty = true;
                    }
                }
            }
        }
        let released = !ui.input(|i| i.pointer.primary_down());
        let dragging = l.drag.is_some();
        if dragging && (released || response.drag_stopped()) {
            l.drag = None;
            self.finish_stroke();
            // Keep the cage: what's on screen now counts as the lattice's result.
            if let (Some(l), Some(scene)) = (&mut self.lattice, &self.scene) {
                l.result = scene.parts[l.part].mesh.positions.clone();
            }
        }
        if dragging || self.lattice.as_ref().is_some_and(|l| l.hot.is_some()) {
            ui.ctx().request_repaint();
        }
        !dragging && self.lattice.as_ref().is_none_or(|l| l.hot.is_none())
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
                    Tool::Shrink => {
                        if let Some(surface) = &self.body_cache {
                            sculpt::shrink(
                                &mut mesh.positions,
                                graph,
                                surface,
                                hit.point,
                                radius,
                                strength * 0.3,
                                self.fit_settings.gap,
                            );
                        }
                    }
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
        // The body plus lower-layer garments; rebuilt for each stroke because
        // other garments may have changed.
        let inner = garment::inner_parts(scene, &self.garments, self.selected_garment);
        self.body_cache = garment::collision_bvh(scene, &inner);
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
                            .chain(Tool::SCULPT)
                            .chain(Tool::SHAPE);
                        for (n, t) in all.enumerate() {
                            if n == 1 || n == 4 || n == 8 {
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
            let text = if self.tool.edits_shape() && !self.at_bind_pose() {
                "Reset the pose to sculpt.".to_string()
            } else {
                format!("{}   ·   Alt + drag: orbit", self.tool.help())
            };
            ui.painter().text(
                rect.left_top() + egui::vec2(12.0, 52.0),
                egui::Align2::LEFT_TOP,
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

        let fitting = self.workspace == Workspace::Fitment;
        if fitting {
            self.tool_shortcuts(ui);
        }
        let camera_free = if fitting { self.garment_tools(ui, &response, rect) } else { true };

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
                viewport.set_scene(&rs.device, &rs.queue, scene, &joint_world);
                self.needs_upload = false;
                self.needs_pose_update = false;
                self.body_preview = None;
            }
        }
        if let Some(send) = self.pending_save.take() {
            self.save_outfit(rs, &joint_world, send);
        }
        let body_hide = self.body_preview_wanted();
        let gizmo_now = self.active_gizmo();
        let outline_part = self.selected_outline();
        let shadow = self.floor_shadow();
        let Some(viewport) = &mut self.viewport else {
            return;
        };

        if let Some(scene) = &self.scene {
            if self.needs_upload {
                viewport.set_scene(&rs.device, &rs.queue, scene, &joint_world);
                self.needs_upload = false;
                self.needs_pose_update = false;
                self.dirty_part = None;
                self.body_preview = None;
            } else if self.needs_pose_update {
                viewport.update_pose(&rs.queue, scene, &joint_world);
                self.needs_pose_update = false;
            }
            if let Some(i) = self.dirty_part.take() {
                viewport.update_part(&rs.queue, scene, i, &joint_world);
            }
            if let Some(i) = self.dirty_material.take() {
                viewport.refresh_material(&rs.device, &rs.queue, scene, i, &joint_world);
            }
            // Export tab: show the body with the skin under the clothes removed.
            match &body_hide {
                Some((sig, b, hidden)) if self.body_preview != Some(*sig) => {
                    viewport.set_hidden_triangles(&rs.queue, scene, *b, Some(hidden));
                    self.body_preview = Some(*sig);
                }
                None if self.body_preview.is_some() => {
                    if let Some(b) = garment::body_part(scene) {
                        viewport.set_hidden_triangles(&rs.queue, scene, b, None);
                    }
                    self.body_preview = None;
                }
                _ => {}
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
        if let Some(l) = self.lattice.as_ref().filter(|_| self.tool == Tool::Lattice) {
            overlay_lines.extend(tools::lattice_lines(&l.lattice, l.hot, &self.camera));
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
            outline_part,
            shadow,
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

        if self.workspace == Workspace::Fitment {
            self.viewport_toolbar(ui, rect);
        }
        if self.scene.is_some() {
            self.pose_picker(ui, rect);
        }

        if self.scene.is_none() && self.loading.is_none() {
            let logo = &self.icons.logo_text;
            let w = (rect.width() * 0.32).clamp(160.0, 360.0);
            let size = egui::vec2(w, w * logo.size()[1] as f32 / logo.size()[0] as f32);
            let logo_rect = egui::Rect::from_center_size(rect.center() - egui::vec2(0.0, size.y * 0.5 + 24.0), size);
            ui.painter().image(
                logo.id(),
                logo_rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                Color32::from_white_alpha(70),
            );
            ui.painter().text(
                rect.center() + egui::vec2(0.0, 18.0),
                egui::Align2::CENTER_CENTER,
                "Import > Open body…   or drop a file here",
                egui::FontId::proportional(18.0),
                Color32::from_gray(140),
            );
        }
    }
}

impl ThreadSmithApp {
    /// Quick pose switcher at the bottom of the viewport: bind pose or an animation.
    fn pose_picker(&mut self, ui: &egui::Ui, rect: egui::Rect) {
        let current = match self.anim.current.and_then(|i| self.anim.entries.get(i)) {
            Some(e) if self.anim.retarget.is_some() || self.anim.loading.is_some() => e.name.clone(),
            _ => "Bind pose".to_string(),
        };
        let mut pick: Option<Option<usize>> = None;
        let mut toggle_play = false;
        egui::Area::new(ui.scope_id().with("pose picker"))
            .pivot(egui::Align2::CENTER_BOTTOM)
            .fixed_pos(rect.center_bottom() - egui::vec2(0.0, 14.0))
            .show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Pose").weak());
                        egui::ComboBox::from_id_salt("pose picker combo")
                            .selected_text(current.clone())
                            .width(180.0)
                            .show_ui(ui, |ui| {
                                if ui.selectable_label(current == "Bind pose", "Bind pose").clicked() {
                                    pick = Some(None);
                                }
                                for (i, e) in self.anim.entries.iter().enumerate() {
                                    if ui.selectable_label(self.anim.current == Some(i) && current != "Bind pose", &e.name).clicked() {
                                        pick = Some(Some(i));
                                    }
                                }
                                if self.anim.entries.is_empty() {
                                    ui.label(RichText::new("Add Mixamo animations on the Fitment tab.").weak().small());
                                }
                            });
                        if current != "Bind pose" {
                            let label = if self.anim.playing { "Pause" } else { "Play" };
                            if ui.button(label).clicked() {
                                toggle_play = true;
                            }
                        }
                    });
                });
            });
        match pick {
            Some(None) => self.reset_pose(),
            Some(Some(i)) => {
                let sk = self.scene.as_ref().map(|s| &s.skeleton);
                self.anim.select(i, sk);
                self.anim.playing = true;
                self.anim_dirty = true;
            }
            None => {}
        }
        if toggle_play {
            self.anim.playing = !self.anim.playing;
        }
    }
}

impl ThreadSmithApp {
    /// Remove a garment from the scene (and fix up every index that pointed past it).
    fn delete_part(&mut self, index: usize) {
        let Some(scene) = &mut self.scene else { return };
        if index >= scene.parts.len() {
            return;
        }
        let name = scene.parts[index].name.clone();
        scene.parts.remove(index);
        self.garments.retain(|g| g.part != index);
        for g in &mut self.garments {
            if g.part > index {
                g.part -= 1;
            }
            g.reference = match g.reference {
                Some(r) if r == index => None,
                Some(r) if r > index => Some(r - 1),
                r => r,
            };
        }
        self.selected_garment = self.selected_garment.min(self.garments.len().saturating_sub(1));
        // Undo steps point at old garment numbers: start afresh.
        self.undo.clear();
        self.undo_last = None;
        self.stroke = None;
        self.gizmo_drag = None;
        self.needs_upload = true;
        self.project_dirty = true;
        self.status = format!("Deleted {name}.");
    }
}

impl ThreadSmithApp {
    /// The selected garment gets an outline on the Fitment tab.
    fn selected_outline(&self) -> Option<usize> {
        if self.workspace != Workspace::Fitment {
            return None;
        }
        self.garments.get(self.selected_garment).map(|g| g.part)
    }

    /// A soft shadow on the floor under the character (from its bones).
    fn floor_shadow(&self) -> Option<(Vec3, f32)> {
        let scene = self.scene.as_ref()?;
        if scene.skeleton.is_empty() {
            return None;
        }
        let joints: Vec<Vec3> = self
            .pose
            .world_matrices(&scene.skeleton)
            .iter()
            .map(|m| m.w_axis.truncate())
            .collect();
        let n = joints.len() as f32;
        let centre = joints.iter().fold(Vec3::ZERO, |a, b| a + *b) / n;
        let spread = joints
            .iter()
            .map(|p| Vec2::new(p.x - centre.x, p.z - centre.z).length())
            .fold(0.0f32, f32::max);
        Some((Vec3::new(centre.x, 0.0, centre.z), (spread * 0.9).clamp(0.3, 1.5)))
    }
}

impl ThreadSmithApp {
    // ---- Projects ----------------------------------------------------------------

    fn build_project(&self) -> Option<crate::project::Project> {
        let scene = self.scene.as_ref()?;
        Some(crate::project::Project {
            scene: scene.clone(),
            garments: self.garments.iter().map(crate::project::SavedGarment::from_work).collect(),
            selected_garment: self.selected_garment,
            fit_settings: self.fit_settings.clone(),
            skin_settings: self.skin_settings.clone(),
            unreal_body: self.unreal_body.clone(),
            outfit_name: self.outfit_name.clone(),
            outfit_category: self.outfit_category.clone(),
            apply_to_character: self.apply_to_character,
            hide_body: self.hide_body,
        })
    }

    fn project_name(&self) -> String {
        match &self.project_path {
            Some(p) => project_label(p),
            None => self
                .unreal_body
                .as_ref()
                .map(|u| u.character.clone())
                .filter(|c| !c.is_empty())
                .or_else(|| self.scene.as_ref().map(|s| s.name.clone()))
                .unwrap_or_else(|| "Untitled".into()),
        }
    }

    /// Save (to the current file, or ask where if `ask` or there isn't one yet).
    fn save_project(&mut self, ask: bool, then: Option<PendingAction>) {
        if self.project_job.is_some() {
            return;
        }
        let path = match (&self.project_path, ask) {
            (Some(p), false) => p.clone(),
            _ => {
                let dir = crate::project::default_dir();
                let _ = std::fs::create_dir_all(&dir);
                let Some(mut p) = rfd::FileDialog::new()
                    .set_title("Save ThreadSmith project")
                    .set_directory(&dir)
                    .set_file_name(crate::project::suggested_name(&self.project_name()))
                    .add_filter("ThreadSmith project", &[crate::project::EXTENSION])
                    .save_file()
                else {
                    return;
                };
                if p.extension().is_none() {
                    p.set_extension(crate::project::EXTENSION);
                }
                p
            }
        };
        let Some(project) = self.build_project() else { return };
        let (tx, rx) = mpsc::channel();
        let target = path.clone();
        let wake = self.ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(project.save(&target));
            wake.request_repaint();
        });
        self.status = format!("Saving {}…", project_label(&path));
        self.project_job = Some(ProjectJob::Saving { path, rx, then });
    }

    fn open_project_dialog(&mut self) {
        let dir = crate::project::default_dir();
        if let Some(p) = rfd::FileDialog::new()
            .set_title("Open ThreadSmith project")
            .set_directory(&dir)
            .add_filter("ThreadSmith project", &[crate::project::EXTENSION])
            .pick_file()
        {
            self.request(PendingAction::Open(p));
        }
    }

    fn open_project(&mut self, path: PathBuf) {
        if self.project_job.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        let source = path.clone();
        let wake = self.ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(crate::project::Project::load(&source));
            wake.request_repaint();
        });
        self.status = format!("Opening {}…", project_label(&path));
        self.project_job = Some(ProjectJob::Opening { path, rx });
    }

    fn apply_project(&mut self, project: crate::project::Project, path: PathBuf) {
        self.set_scene(project.scene);
        self.garments = project.garments.into_iter().map(|g| g.into_work()).collect();
        self.selected_garment = project.selected_garment.min(self.garments.len().saturating_sub(1));
        self.fit_settings = project.fit_settings;
        self.skin_settings = project.skin_settings;
        self.unreal_body = project.unreal_body;
        self.outfit_name = project.outfit_name;
        self.outfit_category = project.outfit_category;
        self.apply_to_character = project.apply_to_character;
        self.hide_body = project.hide_body;
        self.body_cover = None;
        self.project_path = Some(path.clone());
        self.project_dirty = false;
        self.add_recent(path);
        self.set_workspace(if self.garments.is_empty() { Workspace::Import } else { Workspace::Fitment });
    }

    fn add_recent(&mut self, path: PathBuf) {
        self.recent_projects.retain(|p| p != &path);
        self.recent_projects.insert(0, path);
        self.recent_projects.truncate(10);
    }

    fn poll_project_job(&mut self) {
        let Some(job) = &self.project_job else { return };
        match job {
            ProjectJob::Saving { path, rx, then } => {
                let Ok(result) = rx.try_recv() else { return };
                let (path, then) = (path.clone(), then.clone());
                self.project_job = None;
                match result {
                    Ok(()) => {
                        self.status = format!("Saved project {}.", project_label(&path));
                        self.project_path = Some(path.clone());
                        self.project_dirty = false;
                        self.add_recent(path);
                        if let Some(a) = then {
                            self.do_action(a);
                        }
                    }
                    Err(e) => self.error = Some(format!("Couldn't save the project:\n{e:#}")),
                }
            }
            ProjectJob::Opening { path, rx } => {
                let Ok(result) = rx.try_recv() else { return };
                let path = path.clone();
                self.project_job = None;
                match result {
                    Ok(project) => {
                        self.apply_project(project, path.clone());
                        self.status = format!(
                            "Opened project {} · {} garment(s).",
                            project_label(&path),
                            self.garments.len()
                        );
                    }
                    Err(e) => {
                        self.recent_projects.retain(|p| p != &path || p.is_file());
                        self.error = Some(format!("Couldn't open the project:\n{e:#}"));
                    }
                }
            }
        }
    }

    /// Do something that would lose unsaved work, asking first if needed.
    fn request(&mut self, action: PendingAction) {
        if self.project_dirty && self.scene.is_some() {
            self.pending_action = Some(action);
        } else {
            self.do_action(action);
        }
    }

    fn do_action(&mut self, action: PendingAction) {
        match action {
            PendingAction::Close => {
                self.allow_close = true;
                self.ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            PendingAction::New => {
                self.scene = None;
                self.garments.clear();
                self.undo.clear();
                self.body_cache = None;
                self.selected_joint = None;
                self.unreal_body = None;
                self.outfit_name.clear();
                if let Some(v) = &mut self.viewport {
                    v.clear_scene();
                }
                self.project_path = None;
                self.project_dirty = false;
                self.set_workspace(Workspace::Import);
                self.status = "New project. Open a body to start.".into();
            }
            PendingAction::Open(p) => self.open_project(p),
        }
    }

    fn project_shortcuts(&mut self, ctx: &egui::Context) {
        let (save_as, save, open) = ctx.input_mut(|i| {
            (
                i.consume_key(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, egui::Key::S),
                i.consume_key(egui::Modifiers::COMMAND, egui::Key::S),
                i.consume_key(egui::Modifiers::COMMAND, egui::Key::O),
            )
        });
        if save_as && self.scene.is_some() {
            self.save_project(true, None);
        } else if save && self.scene.is_some() {
            self.save_project(false, None);
        }
        if open {
            self.open_project_dialog();
        }
    }

    /// Closing the window with unsaved changes asks first.
    fn watch_close(&mut self, ctx: &egui::Context) {
        if ctx.input(|i| i.viewport().close_requested())
            && self.project_dirty
            && self.scene.is_some()
            && !self.allow_close
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.pending_action = Some(PendingAction::Close);
        }
    }

    fn update_title(&mut self, ctx: &egui::Context) {
        let title = if self.scene.is_some() {
            format!("ThreadSmith - {}{}", self.project_name(), if self.project_dirty { " *" } else { "" })
        } else {
            "ThreadSmith".to_string()
        };
        if title != self.window_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.window_title = title;
        }
    }

    fn save_prompt(&mut self, ctx: &egui::Context) {
        let Some(action) = self.pending_action.clone() else { return };
        let what = match &action {
            PendingAction::Close => "before closing",
            PendingAction::New => "before starting a new project",
            PendingAction::Open(_) => "before opening another project",
        };
        egui::Window::new("Unsaved changes")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ctx, |ui| {
                ui.label(format!("Save your changes to {} {what}?", self.project_name()));
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button(RichText::new("Save").strong()).clicked() {
                        self.pending_action = None;
                        self.save_project(false, Some(action.clone()));
                    }
                    if ui.button("Don't save").clicked() {
                        self.pending_action = None;
                        self.do_action(action.clone());
                    }
                    if ui.button("Cancel").clicked() {
                        self.pending_action = None;
                    }
                });
            });
    }

    fn set_workspace(&mut self, ws: Workspace) {
        if ws != Workspace::Fitment {
            // Gizmos and brushes only live on the Fitment tab.
            self.tool = Tool::Camera;
            self.gizmo_drag = None;
            self.stroke = None;
        }
        self.workspace = ws;
    }

    /// The Import / Generate / Fitment / Export tabs. Ctrl+1…4 switch too.
    fn workspace_tabs(&mut self, ui: &mut egui::Ui) {
        if !ui.ctx().egui_wants_keyboard_input() {
            let pressed = ui.input(|i| {
                Workspace::ALL
                    .into_iter()
                    .find(|w| i.modifiers.command && i.key_pressed(w.key()))
            });
            if let Some(w) = pressed {
                self.set_workspace(w);
            }
        }
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            for (n, w) in Workspace::ALL.into_iter().enumerate() {
                let selected = self.workspace == w;
                let text = RichText::new(format!("  {}  ", w.label())).size(15.0);
                let text = if selected { text.strong().color(Color32::WHITE) } else { text };
                let icon = match w {
                    Workspace::Import => Some(&self.icons.import),
                    Workspace::Export => Some(&self.icons.export),
                    Workspace::Generate => Some(&self.icons.generate),
                    Workspace::Fitment => Some(&self.icons.fitment),
                };
                let mut button = match icon {
                    Some(i) => ui_icons::button(i, text),
                    None => egui::Button::new(text),
                };
                button = button
                    .selected(selected)
                    .min_size(egui::vec2(110.0, 28.0))
                    .corner_radius(egui::CornerRadius { nw: 6, ne: 6, sw: 0, se: 0 });
                if ui
                    .add(button)
                    .on_hover_text(format!("Ctrl+{}", n + 1))
                    .clicked()
                {
                    self.set_workspace(w);
                }
            }
        });
    }

    /// Import tab: the body, clothing files, and the outfit library.
    fn import_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Import");
        ui.add_space(4.0);
        ui.label(RichText::new("Project").strong());
        ui.horizontal(|ui| {
            if ui.button("Open project…").clicked() {
                self.open_project_dialog();
            }
            if ui.add_enabled(self.scene.is_some(), egui::Button::new("Save project")).clicked() {
                self.save_project(false, None);
            }
        });
        if !self.recent_projects.is_empty() {
            ui.label(RichText::new("Recent").weak().small());
            for p in self.recent_projects.clone().into_iter().take(5) {
                if ui
                    .link(project_label(&p))
                    .on_hover_text(p.display().to_string())
                    .clicked()
                {
                    self.request(PendingAction::Open(p));
                }
            }
        }
        ui.add_space(8.0);
        ui.label(RichText::new("Body").strong());
        if ui.add(ui_icons::button(&self.icons.import, "Open body…")).clicked() {
            self.open_dialog(LoadKind::Body);
        }
        let unreal = if self.bridge.plugin_connected() {
            "Unreal is connected: use Send to ThreadSmith on a character there."
        } else {
            "Or open Unreal with the ThreadSmith Bridge plugin and use Send to ThreadSmith."
        };
        ui.label(RichText::new(unreal).weak().small());
        ui.add_space(8.0);

        ui.label(RichText::new("Clothing").strong());
        ui.horizontal(|ui| {
            if ui
                .add_enabled(self.has_body(), ui_icons::button(&self.icons.import, "Add garment file…"))
                .on_disabled_hover_text("Open a body first.")
                .clicked()
            {
                self.open_dialog(LoadKind::Garment);
            }
            if ui.add(ui_icons::button(&self.icons.generate, "Make one with AI")).clicked() {
                self.set_workspace(Workspace::Generate);
            }
        });
        ui.label(RichText::new("FBX, glTF / GLB or OBJ. You can also drop files onto the window.").weak().small());
        ui.add_space(8.0);

        ui.horizontal(|ui| {
            ui.add(ui_icons::small(&self.icons.wardrobe));
            ui.label(RichText::new(format!("Outfit library ({})", self.outfits.len())).strong());
            if let Some(lib) = &self.library {
                if ui.small_button("Refresh").clicked() {
                    self.outfits = lib.list();
                    self.library_thumbs.clear();
                }
                if ui.small_button("Open folder").clicked() {
                    open_in_explorer(&lib.root);
                }
            }
        });
        if self.outfits.is_empty() {
            ui.label(RichText::new("Nothing saved yet. Outfits you save on the Export tab show up here.").weak());
            return;
        }
        let ctx = ui.ctx().clone();
        let tile = egui::vec2(96.0, 118.0);
        let mut add = None;
        ui.horizontal_wrapped(|ui| {
            for (i, o) in self.outfits.iter().enumerate() {
                let thumb = self
                    .library_thumbs
                    .entry(o.thumbnail_path())
                    .or_insert_with(|| load_thumb(&ctx, &o.thumbnail_path()))
                    .clone();
                let (rect, resp) = ui.allocate_exact_size(tile, Sense::click());
                let painter = ui.painter_at(rect);
                painter.rect_filled(rect, 6.0, if resp.hovered() { Color32::from_gray(48) } else { Color32::from_gray(36) });
                let img = egui::Rect::from_min_size(rect.min + egui::vec2(6.0, 6.0), egui::vec2(84.0, 84.0));
                let _ = match &thumb {
                    Some(t) => painter.image(t.id(), img, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE),
                    None => painter.rect_filled(img, 4.0, Color32::from_gray(28)),
                };
                let mut name = o.info.name.clone();
                if name.chars().count() > 14 {
                    name = name.chars().take(13).collect::<String>() + "…";
                }
                painter.text(rect.center_bottom() - egui::vec2(0.0, 6.0), egui::Align2::CENTER_BOTTOM, name, egui::FontId::proportional(12.0), Color32::from_gray(210));
                let resp = resp.on_hover_text(format!("{} · {}\nDouble-click to add it to the scene.", o.info.name, o.info.category));
                if resp.double_clicked() {
                    add = Some(i);
                }
            }
        });
        if let Some(i) = add {
            if self.has_body() {
                let path = self.outfits[i].mesh_path();
                self.start_load(path, LoadKind::Garment);
                self.set_workspace(Workspace::Fitment);
            } else {
                self.error = Some("Open a body first, then add outfits to it.".into());
            }
        }
    }

    /// Export tab: what goes into the outfit, then save / send.
    fn export_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Export");
        ui.add_space(4.0);
        let Some(scene) = &self.scene else {
            ui.label(RichText::new("Open a body and dress it first.").weak());
            return;
        };
        if self.garments.is_empty() {
            ui.label(RichText::new("No clothing to export yet.").weak());
            return;
        }
        ui.label(RichText::new("In this outfit").strong());
        for g in &mut self.garments {
            ui.horizontal(|ui| {
                ui.checkbox(&mut g.include, &scene.parts[g.part].name);
                let (text, color) = match g.status {
                    GarmentStatus::Skinned => ("ready", Color32::from_rgb(120, 210, 120)),
                    GarmentStatus::Fitted => ("not skinned", Color32::from_rgb(230, 170, 80)),
                    GarmentStatus::Added => ("not fitted", Color32::from_rgb(230, 130, 90)),
                };
                ui.label(RichText::new(text).small().color(color));
            });
        }
        if self.garments.iter().any(|g| g.include && g.status != GarmentStatus::Skinned) {
            ui.label(
                RichText::new("Pieces that aren't skinned won't move with the body in Unreal. Finish them on the Fitment tab.")
                    .small()
                    .color(Color32::from_rgb(230, 170, 80)),
            );
        }
        ui.add_space(8.0);
        self.save_section(ui);
    }
}

impl ThreadSmithApp {
    /// Put newly added AI garments (from index `first`) on the body using
    /// their preset, and set up skinning / saving to match.
    fn place_ai_garments(&mut self, first: usize, preset: &ts_gen::Garment, prompt: &str) {
        let Some(scene) = &mut self.scene else { return };
        let mut notes = Vec::new();
        for gi in first..self.garments.len() {
            let g = &mut self.garments[gi];
            scene.parts[g.part].name = format!("AI {}", preset.label);
            g.layer = garment::default_layer(&preset.id);
            notes.push(garment::auto_place(scene, g, &preset.place));
            // Hollow cloth that still sinks into the body: grow it just enough.
            if preset.skin == "cloth" && preset.place.pair.is_empty() {
                let layer = g.layer;
                let part = g.part;
                let inner: Vec<usize> = self
                    .garments
                    .iter()
                    .filter(|o| o.layer < layer && o.part != part)
                    .map(|o| o.part)
                    .collect();
                let g = &mut self.garments[gi];
                if let Ok(size) = garment::grow_to_clear_body(scene, g, &inner) {
                    if size > 1.001 {
                        notes.push(format!("Grew it to {:.0}% so it clears the body.", size * 100.0));
                    }
                }
            }
        }
        self.skin_settings.rigid = preset.skin == "rigid";
        if !preset.library_category.is_empty() {
            self.outfit_category = preset.library_category.clone();
        }
        if self.outfit_name.trim().is_empty() {
            let words: String = prompt.split_whitespace().take(4).collect::<Vec<_>>().join(" ");
            self.outfit_name = if words.is_empty() { preset.label.clone() } else { format!("{} {}", words, preset.label) };
        }
        self.body_cache = None;
        self.needs_upload = true;
        self.status = format!(
            "Added the AI {}. {} Check it with Move / Rotate / Scale, then Fit to body and Auto skin.",
            preset.label.to_lowercase(),
            notes.join(" ")
        );
    }
}

impl eframe::App for ThreadSmithApp {
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, crate::generate::SETTINGS_KEY, &self.generate.settings);
        eframe::set_value(storage, RECENT_KEY, &self.recent_projects);
    }

    fn on_exit(&mut self) {
        self.generate.shutdown();
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll_bridge();
        self.poll_loading(&ctx);
        if self.loading.is_none() && self.scene.is_some() {
            if let Some(p) = self.dev_garment.take() {
                self.start_load(p, LoadKind::Garment);
            }
        }
        self.anim_update(&ctx);
        self.handle_dropped_files(&ctx);
        self.project_shortcuts(&ctx);
        self.poll_project_job();
        self.watch_close(&ctx);
        self.update_title(&ctx);
        if let Some(GenOutput::Mesh { path, garment, prompt }) = self.generate.poll(&ctx) {
            if self.has_body() {
                self.pending_ai = Some((garment, prompt));
                self.start_load(path, LoadKind::Garment);
                self.set_workspace(Workspace::Fitment);
            } else {
                self.error = Some(format!("The garment was made ({}), but there's no body open to put it on.", path.display()));
            }
        }

        egui::Panel::top("menu").show(ui, |ui| self.menu_bar(ui));
        egui::Panel::top("workspaces").show(ui, |ui| self.workspace_tabs(ui));
        egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui));
        egui::Panel::left("scene")
            .resizable(true)
            .default_size(280.0)
            .show(ui, |ui| {
                self.scene_panel(ui);
            });
        let (id, width) = match self.workspace {
            Workspace::Generate => ("right_generate", 420.0),
            _ => ("right_main", 320.0),
        };
        egui::Panel::right(id)
            .resizable(true)
            .default_size(width)
            .min_size(300.0)
            .show(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| match self.workspace {
                    Workspace::Import => self.import_panel(ui),
                    Workspace::Generate => {
                        let has_body = self.has_body();
                        self.generate.panel(ui, has_body);
                    }
                    Workspace::Fitment => {
                        self.garment_panel(ui);
                        ui.separator();
                        self.animation_panel(ui);
                        ui.separator();
                        self.pose_panel(ui);
                    }
                    Workspace::Export => self.export_panel(ui),
                });
            });
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| self.viewport_ui(ui, frame));

        self.save_prompt(&ctx);

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

/// Load an outfit thumbnail as a texture (None if it can't be read).
fn load_thumb(ctx: &egui::Context, path: &std::path::Path) -> Option<egui::TextureHandle> {
    let img = image::open(path).ok()?.thumbnail(128, 128).to_rgba8();
    let size = [img.width() as usize, img.height() as usize];
    Some(ctx.load_texture(
        path.display().to_string(),
        egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw()),
        egui::TextureOptions::LINEAR,
    ))
}

/// "Ryan.tsproj" → "Ryan".
fn project_label(path: &std::path::Path) -> String {
    path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "project".into())
}

/// A 64 px picture of a texture for the Layers list (made once per texture).
fn layer_thumb(
    ctx: &egui::Context,
    cache: &mut std::collections::HashMap<u64, Option<egui::TextureHandle>>,
    t: &ts_core::Texture,
) -> Option<egui::TextureHandle> {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    t.bytes.len().hash(&mut h);
    t.bytes[..t.bytes.len().min(4096)].hash(&mut h);
    t.bytes[t.bytes.len().saturating_sub(4096)..].hash(&mut h);
    cache
        .entry(h.finish())
        .or_insert_with(|| {
            let img = image::load_from_memory(&t.bytes).ok()?.thumbnail(64, 64).to_rgba8();
            let size = [img.width() as usize, img.height() as usize];
            Some(ctx.load_texture(
                format!("layer-{}", t.name),
                egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw()),
                egui::TextureOptions::LINEAR,
            ))
        })
        .clone()
}
