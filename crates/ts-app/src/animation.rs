//! Playing animations (Mixamo etc.) on the body to test the clothes, and
//! finding where the clothes poke through the skin.
//!
//! Animations live in `Documents/ThreadSmith/Library/Animations`, so the ones
//! you add are there every time ThreadSmith opens (like MetaTailor's list).

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;

use glam::{Mat4, Vec3};
use ts_core::{AnimClip, PartRole, Retargeter, Scene, Skeleton};
use ts_fit::TriBvh;

/// Garment points further inside the skin than this count as clipping (meters).
pub const CLIP_TOLERANCE: f32 = 0.002;

pub struct AnimEntry {
    pub path: PathBuf,
    pub name: String,
    pub clip: Option<Arc<AnimClip>>,
    pub error: Option<String>,
}

/// One frame of a clipping scan.
#[derive(Clone, Copy, Debug)]
pub struct ScanFrame {
    pub time: f32,
    pub points: usize,
}

pub struct AnimPlayer {
    pub folder: Option<PathBuf>,
    pub entries: Vec<AnimEntry>,
    pub current: Option<usize>,
    /// Built for the current clip and the current body's skeleton.
    pub retarget: Option<Retargeter>,
    pub time: f32,
    pub playing: bool,
    pub speed: f32,
    pub looping: bool,
    pub in_place: bool,
    pub show_clipping: bool,
    /// Clipping points on the frame shown (world space).
    pub clip_points: Vec<Vec3>,
    /// When `clip_points` was worked out (animation time), to avoid redoing it.
    pub clip_checked_at: Option<f32>,
    /// Seconds since the last live clipping check (it's limited while playing).
    pub since_check: f32,
    pub loading: Option<(usize, Receiver<Result<AnimClip, String>>)>,
    pub scan: Option<Receiver<ScanMessage>>,
    pub scan_progress: f32,
    pub scan_result: Option<Vec<ScanFrame>>,
}

pub enum ScanMessage {
    Progress(f32),
    Done(Vec<ScanFrame>),
}

impl AnimPlayer {
    pub fn new(folder: Option<PathBuf>) -> Self {
        let mut p = Self {
            folder,
            entries: Vec::new(),
            current: None,
            retarget: None,
            time: 0.0,
            playing: false,
            speed: 1.0,
            looping: true,
            in_place: true,
            show_clipping: true,
            clip_points: Vec::new(),
            clip_checked_at: None,
            since_check: 0.0,
            loading: None,
            scan: None,
            scan_progress: 0.0,
            scan_result: None,
        };
        p.refresh();
        p
    }

    /// List the animation files in the folder (keeps loaded ones).
    pub fn refresh(&mut self) {
        let Some(folder) = &self.folder else { return };
        let _ = std::fs::create_dir_all(folder);
        let Ok(dir) = std::fs::read_dir(folder) else {
            return;
        };
        let mut paths: Vec<PathBuf> = dir
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("fbx")))
            .collect();
        paths.sort();
        for p in paths {
            if !self.entries.iter().any(|e| e.path == p) {
                self.entries.push(AnimEntry {
                    name: nice_name(&p),
                    path: p,
                    clip: None,
                    error: None,
                });
            }
        }
    }

    /// Copy files into the animations folder and list them.
    /// Returns how many were added and any error message.
    pub fn add_files(&mut self, files: &[PathBuf]) -> (usize, Option<String>) {
        let Some(folder) = self.folder.clone() else {
            return (0, Some("The animations folder isn't available.".into()));
        };
        let _ = std::fs::create_dir_all(&folder);
        let mut added = 0;
        let mut error = None;
        for f in files {
            let Some(name) = f.file_name() else { continue };
            let dest = folder.join(name);
            if dest != *f {
                if let Err(e) = std::fs::copy(f, &dest) {
                    error = Some(format!("Couldn't copy {}: {e}", f.display()));
                    continue;
                }
            }
            added += 1;
        }
        self.refresh();
        (added, error)
    }

    pub fn clip(&self) -> Option<&Arc<AnimClip>> {
        self.current
            .and_then(|i| self.entries.get(i))
            .and_then(|e| e.clip.as_ref())
    }

    /// Pick an animation (loads it in the background the first time).
    pub fn select(&mut self, index: usize, skeleton: Option<&Skeleton>) {
        self.current = Some(index);
        self.time = 0.0;
        self.scan_result = None;
        self.clip_checked_at = None;
        self.retarget = None;
        let entry = &self.entries[index];
        if let (Some(clip), Some(sk)) = (&entry.clip, skeleton) {
            self.retarget = Some(Retargeter::new(clip, sk));
            self.playing = true;
            return;
        }
        if entry.clip.is_none() {
            let path = entry.path.clone();
            let (tx, rx) = mpsc::channel();
            std::thread::spawn(move || {
                let _ = tx.send(ts_io::load_animation(&path).map_err(|e| format!("{e:#}")));
            });
            self.loading = Some((index, rx));
        }
    }

    /// Check on a background load. Returns an error message if it failed.
    pub fn poll_loading(&mut self, skeleton: Option<&Skeleton>) -> Option<String> {
        let (index, rx) = self.loading.as_ref()?;
        let index = *index;
        let result = rx.try_recv().ok()?;
        self.loading = None;
        match result {
            Ok(clip) => {
                let clip = Arc::new(clip);
                self.entries[index].clip = Some(clip.clone());
                if self.current == Some(index) {
                    if let Some(sk) = skeleton {
                        self.retarget = Some(Retargeter::new(&clip, sk));
                        self.playing = true;
                    }
                }
                None
            }
            Err(e) => {
                self.entries[index].error = Some(e.clone());
                Some(e)
            }
        }
    }

    pub fn duration(&self) -> f32 {
        self.clip().map(|c| c.duration()).unwrap_or(0.0)
    }

    /// Move time forward. Returns true if the pose needs updating.
    pub fn advance(&mut self, dt: f32) -> bool {
        if !self.playing || self.retarget.is_none() {
            return false;
        }
        let d = self.duration();
        self.time += dt * self.speed;
        if self.time > d {
            if self.looping && d > 0.0 {
                self.time %= d;
            } else {
                self.time = d;
                self.playing = false;
            }
        }
        true
    }

    /// Bone transforms for the current time.
    pub fn pose(&self, skeleton: &Skeleton) -> Option<Vec<ts_core::Trs>> {
        let r = self.retarget.as_ref()?;
        let clip = self.clip()?;
        Some(r.pose(clip, skeleton, self.time, self.in_place))
    }

    /// Stop and forget the animation pose.
    pub fn stop(&mut self) {
        self.playing = false;
        self.current = None;
        self.retarget = None;
        self.clip_points.clear();
        self.clip_checked_at = None;
        self.scan_result = None;
    }

    /// Scan the whole animation for clipping in the background.
    pub fn start_scan(&mut self, scene: &Scene) {
        let (Some(clip), Some(r)) = (self.clip().cloned(), self.retarget.clone()) else {
            return;
        };
        let scene = scene.clone();
        let in_place = self.in_place;
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let step = 2.0 / clip.fps.max(1.0); // every other frame
            let d = clip.duration();
            let count = (d / step).ceil() as usize + 1;
            let mut out = Vec::with_capacity(count);
            for k in 0..count {
                let time = (k as f32 * step).min(d);
                let locals = r.pose(&clip, &scene.skeleton, time, in_place);
                let world = scene.skeleton.world_matrices(&locals);
                out.push(ScanFrame {
                    time,
                    points: clipping_points(&scene, &world).len(),
                });
                if tx
                    .send(ScanMessage::Progress((k + 1) as f32 / count as f32))
                    .is_err()
                {
                    return;
                }
            }
            let _ = tx.send(ScanMessage::Done(out));
        });
        self.scan = Some(rx);
        self.scan_progress = 0.0;
        self.scan_result = None;
    }

    pub fn poll_scan(&mut self) {
        let Some(rx) = &self.scan else { return };
        while let Ok(m) = rx.try_recv() {
            match m {
                ScanMessage::Progress(p) => self.scan_progress = p,
                ScanMessage::Done(r) => {
                    self.scan_result = Some(r);
                    self.scan = None;
                    return;
                }
            }
        }
    }
}

/// "Samba Dancing (1).fbx" -> "Samba Dancing (1)".
fn nice_name(p: &Path) -> String {
    p.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Points of skinned garments that are inside the body in the given pose.
/// (Garments that aren't skinned yet don't follow the body, so they're skipped.)
pub fn clipping_points(scene: &Scene, joint_world: &[Mat4]) -> Vec<Vec3> {
    let Some(body) = crate::garment::body_part(scene) else {
        return Vec::new();
    };
    let (bp, bn) = scene.parts[body].mesh.skinned(joint_world);
    let bvh = TriBvh::new(&bp, &bn, &scene.parts[body].mesh.indices);
    let mut out = Vec::new();
    for part in scene
        .parts
        .iter()
        .filter(|p| p.role == PartRole::Garment && p.visible && p.mesh.skin.is_some())
    {
        let (gp, _) = part.mesh.skinned(joint_world);
        for p in gp {
            if let Some(h) = bvh.closest(p) {
                if (p - h.point).dot(h.normal) < -CLIP_TOLERANCE {
                    out.push(p);
                }
            }
        }
    }
    out
}
