//! Viewport tools for placing and sculpting a garment.
//!
//! - Move / Rotate / Scale: a gizmo on the selected garment (like Unreal and
//!   Blender). Drag an axis handle, or the centre square for free moves and
//!   overall size.
//! - Grab / Relax / Push/Pull / Shrink: sculpt brushes for touching up after the fit.
//! - Lattice: a cage of 2x2x2 to 5x5x5 points that bends the whole garment.
//!
//! Dragging anywhere that isn't a handle (or isn't the garment, for brushes)
//! still orbits the camera, so the camera never gets "stuck".

use glam::{Quat, Vec2, Vec3, Vec4};

use crate::camera::OrbitCamera;
use crate::garment::Placement;
use crate::viewport::LineVertex;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Camera,
    Move,
    Rotate,
    Scale,
    Grab,
    Relax,
    Inflate,
    Shrink,
    Lattice,
}

impl Tool {
    pub const PLACE: [Tool; 3] = [Tool::Move, Tool::Rotate, Tool::Scale];
    pub const SCULPT: [Tool; 4] = [Tool::Grab, Tool::Relax, Tool::Inflate, Tool::Shrink];
    pub const SHAPE: [Tool; 1] = [Tool::Lattice];

    pub fn label(self) -> &'static str {
        match self {
            Tool::Camera => "Camera",
            Tool::Move => "Move",
            Tool::Rotate => "Rotate",
            Tool::Scale => "Scale",
            Tool::Grab => "Grab",
            Tool::Relax => "Relax",
            Tool::Inflate => "Push/Pull",
            Tool::Shrink => "Shrink",
            Tool::Lattice => "Lattice",
        }
    }

    pub fn key(self) -> egui::Key {
        match self {
            Tool::Camera => egui::Key::Q,
            Tool::Move => egui::Key::W,
            Tool::Rotate => egui::Key::E,
            Tool::Scale => egui::Key::R,
            Tool::Grab => egui::Key::G,
            Tool::Relax => egui::Key::T,
            Tool::Inflate => egui::Key::I,
            Tool::Shrink => egui::Key::S,
            Tool::Lattice => egui::Key::L,
        }
    }

    pub fn help(self) -> &'static str {
        match self {
            Tool::Camera => "Only moves the camera.",
            Tool::Move => "Drag an arrow to move along it, or the centre square to move freely.",
            Tool::Rotate => "Drag a ring to turn the garment around that axis.",
            Tool::Scale => "Drag a handle to stretch along it, or the centre square to change the overall size.",
            Tool::Grab => "Drag on the garment to pull a soft patch of cloth around.",
            Tool::Relax => "Paint over wrinkles and bumps to smooth them out.",
            Tool::Inflate => "Paint to push cloth outwards. Hold Ctrl to pull it in.",
            Tool::Shrink => "Paint over loose cloth to pull it in snug against the body (or the layer below).",
            Tool::Lattice => "Drag the cage points to bend the whole garment smoothly.",
        }
    }

    pub fn is_gizmo(self) -> bool {
        Self::PLACE.contains(&self)
    }

    pub fn is_brush(self) -> bool {
        Self::SCULPT.contains(&self)
    }

    /// Tools that change the garment's shape (only at the bind pose).
    pub fn edits_shape(self) -> bool {
        self.is_brush() || self == Tool::Lattice
    }
}

/// Which part of the gizmo is under the mouse / being dragged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Handle {
    Axis(usize),
    Center,
}

const AXIS_COLORS: [Vec4; 3] = [
    Vec4::new(0.95, 0.25, 0.25, 1.0),
    Vec4::new(0.35, 0.85, 0.3, 1.0),
    Vec4::new(0.3, 0.5, 1.0, 1.0),
];
const HOT: Vec4 = Vec4::new(1.0, 0.85, 0.2, 1.0);
const PICK_PX: f32 = 9.0;
const RING_SEGMENTS: usize = 64;

/// The gizmo for one frame: where it is and how big.
pub struct Gizmo {
    pub tool: Tool,
    pub center: Vec3,
    /// The garment's own axes (used by Scale).
    pub rotation: Quat,
    /// Length of the arrows in meters (keeps a steady size on screen).
    pub size: f32,
}

impl Gizmo {
    pub fn new(tool: Tool, center: Vec3, rotation: Quat, camera: &OrbitCamera) -> Self {
        Self {
            tool,
            center,
            rotation,
            size: camera.distance * 0.18,
        }
    }

    fn axis(&self, i: usize) -> Vec3 {
        let a = [Vec3::X, Vec3::Y, Vec3::Z][i];
        if self.tool == Tool::Scale {
            self.rotation * a
        } else {
            a
        }
    }

    fn ring_points(&self, i: usize) -> Vec<Vec3> {
        let a = self.axis(i);
        let u = a.any_orthonormal_vector();
        let v = a.cross(u);
        (0..=RING_SEGMENTS)
            .map(|k| {
                let t = k as f32 / RING_SEGMENTS as f32 * std::f32::consts::TAU;
                self.center + (u * t.cos() + v * t.sin()) * self.size
            })
            .collect()
    }

    /// The handle under screen point `mouse` (points inside `rect`), if any.
    pub fn pick(&self, camera: &OrbitCamera, mouse: Vec2, rect: Vec2) -> Option<Handle> {
        let proj = |p: Vec3| camera.project(p, rect);
        let c = proj(self.center)?;
        if self.tool != Tool::Rotate && c.distance(mouse) < PICK_PX * 1.4 {
            return Some(Handle::Center);
        }
        let mut best: Option<(Handle, f32)> = None;
        for i in 0..3 {
            let d = if self.tool == Tool::Rotate {
                let pts: Vec<Vec2> = self.ring_points(i).into_iter().filter_map(proj).collect();
                pts.windows(2)
                    .map(|w| segment_distance(mouse, w[0], w[1]))
                    .fold(f32::MAX, f32::min)
            } else {
                match proj(self.center + self.axis(i) * self.size) {
                    Some(end) => segment_distance(mouse, c, end),
                    None => f32::MAX,
                }
            };
            if d < PICK_PX && best.is_none_or(|b| d < b.1) {
                best = Some((Handle::Axis(i), d));
            }
        }
        best.map(|b| b.0)
    }

    /// Lines to draw (on top of everything). `hot` is highlighted.
    pub fn lines(&self, hot: Option<Handle>) -> Vec<LineVertex> {
        let mut out = Vec::new();
        let color = |h: Handle, base: Vec4| if hot == Some(h) { HOT } else { base };
        let mut line = |a: Vec3, b: Vec3, c: Vec4| {
            out.push(LineVertex::new(a, c));
            out.push(LineVertex::new(b, c));
        };
        for (i, base) in AXIS_COLORS.iter().enumerate() {
            let c = color(Handle::Axis(i), *base);
            if self.tool == Tool::Rotate {
                for w in self.ring_points(i).windows(2) {
                    line(w[0], w[1], c);
                }
                continue;
            }
            let a = self.axis(i);
            let end = self.center + a * self.size;
            line(self.center, end, c);
            let side = a.any_orthonormal_vector() * self.size * 0.06;
            let side2 = a.cross(side);
            if self.tool == Tool::Move {
                // Arrow head.
                let back = end - a * self.size * 0.15;
                for s in [side, -side, side2, -side2] {
                    line(end, back + s, c);
                }
            } else {
                // Small box at the end.
                let corners = [side + side2, side - side2, -side - side2, -side + side2];
                for k in 0..4 {
                    line(end + corners[k], end + corners[(k + 1) % 4], c);
                }
            }
        }
        if self.tool != Tool::Rotate {
            // Centre square (in the plane facing the viewer is good enough as
            // a small 3D cross-square around the centre).
            let c = color(Handle::Center, Vec4::new(0.9, 0.9, 0.9, 1.0));
            let s = self.size * 0.07;
            let (u, v) = (Vec3::X * s, Vec3::Y * s);
            let corners = [u + v, u - v, -u - v, -u + v];
            for k in 0..4 {
                line(
                    self.center + corners[k],
                    self.center + corners[(k + 1) % 4],
                    c,
                );
            }
        }
        out
    }
}

/// A gizmo drag in progress.
pub struct GizmoDrag {
    pub handle: Handle,
    pub tool: Tool,
    pub start_mouse: Vec2,
    pub start: Placement,
    pub center: Vec3,
    pub axis: Vec3,
    pub size: f32,
}

impl GizmoDrag {
    /// The placement after dragging to `mouse`.
    pub fn update(&self, camera: &OrbitCamera, mouse: Vec2, rect: Vec2) -> Placement {
        let mut p = self.start;
        let moved = mouse - self.start_mouse;
        let screen_axis = || -> Option<(Vec2, f32)> {
            let c = camera.project(self.center, rect)?;
            let e = camera.project(self.center + self.axis * self.size, rect)?;
            let d = e - c;
            let len = d.length();
            (len > 2.0).then(|| (d / len, len))
        };
        // Position along the dragged axis under the mouse (meters from the
        // gizmo centre): the closest point between the mouse ray and the axis.
        let along_axis = |m: Vec2| -> Option<f32> {
            let (o, d) = camera.ray(m, rect);
            let a = self.axis;
            let w = o - self.center;
            let b = a.dot(d);
            let denom = 1.0 - b * b;
            if denom < 1e-4 {
                return None; // looking straight down the axis
            }
            Some((a.dot(w) - b * d.dot(w)) / denom)
        };
        // Angle around the axis under the mouse, measured in the ring's plane.
        let ring_angle = |m: Vec2| -> Option<f32> {
            let (o, d) = camera.ray(m, rect);
            let a = self.axis;
            let denom = d.dot(a);
            if denom.abs() < 0.15 {
                return None; // ring seen edge-on
            }
            let hit = o + d * ((self.center - o).dot(a) / denom);
            let u = a.any_orthonormal_vector();
            let v = a.cross(u);
            let r = hit - self.center;
            Some(r.dot(v).atan2(r.dot(u)))
        };
        match (self.tool, self.handle) {
            (Tool::Move, Handle::Axis(_)) => {
                if let (Some(a), Some(b)) = (along_axis(self.start_mouse), along_axis(mouse)) {
                    p.translation = self.start.translation + self.axis * (b - a);
                } else if let Some((dir, len)) = screen_axis() {
                    let amount = moved.dot(dir) / len * self.size;
                    p.translation = self.start.translation + self.axis * amount;
                }
            }
            (Tool::Move, Handle::Center) => {
                let a = camera.screen_to_plane(self.start_mouse, rect, self.center);
                let b = camera.screen_to_plane(mouse, rect, self.center);
                if let (Some(a), Some(b)) = (a, b) {
                    p.translation = self.start.translation + (b - a);
                }
            }
            (Tool::Scale, Handle::Axis(i)) => {
                let f = match (along_axis(self.start_mouse), along_axis(mouse)) {
                    (Some(a), Some(b)) => 1.0 + (b - a) / self.size,
                    _ => screen_axis().map_or(1.0, |(dir, len)| 1.0 + moved.dot(dir) / len),
                };
                p.scale[i] = self.start.scale[i] * f.max(0.05);
            }
            (Tool::Scale, Handle::Center) => {
                let f = (1.0 + (moved.x - moved.y) / 300.0).max(0.05);
                p.size = self.start.size * f;
            }
            (Tool::Rotate, Handle::Axis(_)) => {
                let delta = match (ring_angle(self.start_mouse), ring_angle(mouse)) {
                    (Some(a0), Some(a1)) => a1 - a0,
                    _ => {
                        // Edge-on ring: fall back to turning by screen angle.
                        let Some(c) = camera.project(self.center, rect) else {
                            return p;
                        };
                        let d = (mouse - c).to_angle() - (self.start_mouse - c).to_angle();
                        if self.axis.dot(camera.eye() - self.center) > 0.0 {
                            -d
                        } else {
                            d
                        }
                    }
                };
                p.rotation =
                    (Quat::from_axis_angle(self.axis, delta) * self.start.rotation).normalize();
            }
            _ => {}
        }
        p
    }
}

/// Start a drag on `handle`.
pub fn begin_drag(gizmo: &Gizmo, handle: Handle, mouse: Vec2, placement: Placement) -> GizmoDrag {
    let axis = match handle {
        Handle::Axis(i) => gizmo.axis(i),
        Handle::Center => Vec3::ZERO,
    };
    GizmoDrag {
        handle,
        tool: gizmo.tool,
        start_mouse: mouse,
        start: placement,
        center: gizmo.center,
        axis,
        size: gizmo.size,
    }
}

/// Brush settings shown in the Sculpt section.
#[derive(Clone, Debug)]
pub struct BrushSettings {
    /// Meters.
    pub radius: f32,
    /// 0..1.
    pub strength: f32,
    /// Push anything sculpted into the skin back out when a stroke ends.
    pub keep_outside: bool,
}

impl Default for BrushSettings {
    fn default() -> Self {
        Self {
            radius: 0.05,
            strength: 0.5,
            keep_outside: true,
        }
    }
}

/// A circle showing the brush on the surface.
pub fn brush_circle(center: Vec3, normal: Vec3, radius: f32, active: bool) -> Vec<LineVertex> {
    let color = if active {
        HOT
    } else {
        Vec4::new(1.0, 1.0, 1.0, 0.9)
    };
    let u = normal.any_orthonormal_vector();
    let v = normal.cross(u);
    let pts: Vec<Vec3> = (0..=48)
        .map(|k| {
            let t = k as f32 / 48.0 * std::f32::consts::TAU;
            center + (u * t.cos() + v * t.sin()) * radius
        })
        .collect();
    let mut out = Vec::with_capacity(48 * 2 + 2);
    for w in pts.windows(2) {
        out.push(LineVertex::new(w[0], color));
        out.push(LineVertex::new(w[1], color));
    }
    // A short tick along the normal shows which way is "out".
    out.push(LineVertex::new(center, color));
    out.push(LineVertex::new(center + normal * radius * 0.4, color));
    out
}

/// Distance from `p` to the segment a-b (2D).
fn segment_distance(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    let ab = b - a;
    let t = if ab.length_squared() > 0.0 {
        ((p - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0)
    } else {
        0.0
    };
    p.distance(a + ab * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (OrbitCamera, Vec2, Gizmo) {
        let cam = OrbitCamera {
            target: Vec3::new(0.0, 1.0, 0.0),
            yaw: 0.3,
            pitch: 0.2,
            ..Default::default()
        };
        let rect = Vec2::new(1000.0, 800.0);
        let g = Gizmo::new(Tool::Move, Vec3::new(0.0, 1.0, 0.0), Quat::IDENTITY, &cam);
        (cam, rect, g)
    }

    #[test]
    fn pick_axis_and_center() {
        let (cam, rect, g) = setup();
        let c = cam.project(g.center, rect).unwrap();
        assert_eq!(g.pick(&cam, c, rect), Some(Handle::Center));
        let x_mid = cam
            .project(g.center + Vec3::X * g.size * 0.7, rect)
            .unwrap();
        assert_eq!(g.pick(&cam, x_mid, rect), Some(Handle::Axis(0)));
        assert_eq!(g.pick(&cam, c + Vec2::new(300.0, 300.0), rect), None);
    }

    #[test]
    fn move_along_axis_follows_mouse() {
        let (cam, rect, g) = setup();
        let start = cam
            .project(g.center + Vec3::X * g.size * 0.5, rect)
            .unwrap();
        let drag = begin_drag(&g, Handle::Axis(0), start, Placement::default());
        let end = cam
            .project(g.center + Vec3::X * g.size * 1.5, rect)
            .unwrap();
        let p = drag.update(&cam, end, rect);
        assert!((p.translation.x - g.size).abs() < g.size * 0.02, "{p:?}");
        assert!(p.translation.y.abs() < 1e-4 && p.translation.z.abs() < 1e-4);
    }

    #[test]
    fn rotate_matches_visible_direction() {
        let (cam, rect, _) = setup();
        let g = Gizmo::new(Tool::Rotate, Vec3::new(0.0, 1.0, 0.0), Quat::IDENTITY, &cam);
        // Drag a point on the Y ring a little way along the ring; the garment
        // should turn so that point follows the mouse.
        let u = Vec3::Y.any_orthonormal_vector();
        let v = Vec3::Y.cross(u);
        let on_ring = |t: f32| g.center + (u * t.cos() + v * t.sin()) * g.size;
        let a = cam.project(on_ring(0.3), rect).unwrap();
        let b = cam.project(on_ring(0.6), rect).unwrap();
        let drag = begin_drag(&g, Handle::Axis(1), a, Placement::default());
        let p = drag.update(&cam, b, rect);
        let moved = g.center + p.rotation * (on_ring(0.3) - g.center);
        assert!(
            moved.distance(on_ring(0.6)) < g.size * 0.15,
            "{moved} vs {}",
            on_ring(0.6)
        );
    }

    #[test]
    fn scale_center_grows() {
        let (cam, rect, _) = setup();
        let g = Gizmo::new(Tool::Scale, Vec3::new(0.0, 1.0, 0.0), Quat::IDENTITY, &cam);
        let c = cam.project(g.center, rect).unwrap();
        let drag = begin_drag(&g, Handle::Center, c, Placement::default());
        let p = drag.update(&cam, c + Vec2::new(60.0, 0.0), rect);
        assert!(p.size > 1.15 && p.size < 1.25, "{}", p.size);
    }
}

/// The lattice cage: lines between neighbouring points and a small square
/// on each point (yellow when under the mouse).
pub fn lattice_lines(lattice: &ts_fit::Lattice, hot: Option<usize>, camera: &OrbitCamera) -> Vec<LineVertex> {
    let cage = Vec4::new(1.0, 0.62, 0.15, 0.85);
    let point = Vec4::new(1.0, 0.75, 0.3, 1.0);
    let mut v = Vec::new();
    for (a, b) in lattice.edges() {
        v.push(LineVertex::new(lattice.points[a], cage));
        v.push(LineVertex::new(lattice.points[b], cage));
    }
    let inv = camera.view().inverse();
    let right = inv.transform_vector3(Vec3::X).normalize_or_zero();
    let up = inv.transform_vector3(Vec3::Y).normalize_or_zero();
    for (i, p) in lattice.points.iter().enumerate() {
        let is_hot = hot == Some(i);
        let s = camera.distance * if is_hot { 0.009 } else { 0.006 };
        let c = if is_hot { HOT } else { point };
        let corners = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)].map(|(x, y)| *p + (right * x + up * y) * s);
        for k in 0..4 {
            v.push(LineVertex::new(corners[k], c));
            v.push(LineVertex::new(corners[(k + 1) % 4], c));
        }
        if is_hot {
            v.push(LineVertex::new(corners[0], c));
            v.push(LineVertex::new(corners[2], c));
            v.push(LineVertex::new(corners[1], c));
            v.push(LineVertex::new(corners[3], c));
        }
    }
    v
}

/// The lattice point nearest the mouse (within a few pixels).
pub fn pick_lattice_point(lattice: &ts_fit::Lattice, camera: &OrbitCamera, mouse: Vec2, rect_size: Vec2) -> Option<usize> {
    lattice
        .points
        .iter()
        .enumerate()
        .filter_map(|(i, p)| camera.project(*p, rect_size).map(|s| (i, s.distance(mouse), camera.eye().distance(*p))))
        .filter(|(_, d, _)| *d < PICK_PX * 1.4)
        // Nearest on screen; nearer the camera wins a tie.
        .min_by(|a, b| (a.1 + a.2 * 0.5).total_cmp(&(b.1 + b.2 * 0.5)))
        .map(|(i, _, _)| i)
}
