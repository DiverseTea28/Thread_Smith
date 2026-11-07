//! Orbit camera: spins around a target point.

use glam::{Mat4, Vec2, Vec3};
use ts_core::Bounds;

#[derive(Clone, Debug)]
pub struct OrbitCamera {
    pub target: Vec3,
    /// Turn around the up axis, in radians.
    pub yaw: f32,
    /// Look up / down, in radians.
    pub pitch: f32,
    pub distance: f32,
    pub fov_y: f32,
}

impl Default for OrbitCamera {
    fn default() -> Self {
        Self {
            target: Vec3::new(0.0, 0.9, 0.0),
            yaw: 0.45,
            pitch: 0.15,
            distance: 3.5,
            fov_y: 35f32.to_radians(),
        }
    }
}

impl OrbitCamera {
    pub fn eye(&self) -> Vec3 {
        let dir = Vec3::new(
            self.pitch.cos() * self.yaw.sin(),
            self.pitch.sin(),
            self.pitch.cos() * self.yaw.cos(),
        );
        self.target + dir * self.distance
    }

    pub fn view(&self) -> Mat4 {
        glam::camera::rh::view::look_at_mat4(self.eye(), self.target, Vec3::Y)
    }

    pub fn proj(&self, aspect: f32) -> Mat4 {
        let near = (self.distance * 0.01).max(0.001);
        // wgpu uses DirectX-style depth (0 to 1).
        glam::camera::rh::proj::directx::perspective(
            self.fov_y,
            aspect.max(0.01),
            near,
            self.distance * 100.0 + 100.0,
        )
    }

    pub fn view_proj(&self, aspect: f32) -> Mat4 {
        self.proj(aspect) * self.view()
    }

    /// Drag to spin. `delta` is in screen points.
    pub fn orbit(&mut self, delta: Vec2) {
        self.yaw -= delta.x * 0.008;
        self.pitch = (self.pitch + delta.y * 0.008).clamp(-1.55, 1.55);
    }

    /// Drag to slide the view. `delta` in points, `height` = viewport height in points.
    pub fn pan(&mut self, delta: Vec2, height: f32) {
        let view = self.view();
        let right = view.row(0).truncate();
        let up = view.row(1).truncate();
        let scale = 2.0 * self.distance * (self.fov_y * 0.5).tan() / height.max(1.0);
        self.target += (-right * delta.x + up * delta.y) * scale;
    }

    /// Scroll to zoom. Positive = closer.
    pub fn zoom(&mut self, amount: f32) {
        self.distance =
            (self.distance * (1.0 - amount * 0.0015).clamp(0.5, 1.5)).clamp(0.02, 10_000.0);
    }

    /// Fit the whole box in view.
    pub fn frame(&mut self, bounds: Bounds) {
        self.target = bounds.center();
        self.distance = bounds.radius() / (self.fov_y * 0.5).sin() * 1.1;
    }

    /// Screen position (points, inside `rect_size`) of a world point, if in front of the camera.
    pub fn project(&self, p: Vec3, rect_size: Vec2) -> Option<Vec2> {
        let clip = self.view_proj(rect_size.x / rect_size.y) * p.extend(1.0);
        if clip.w <= 0.0 {
            return None;
        }
        let ndc = clip.truncate() / clip.w;
        Some(Vec2::new(
            (ndc.x * 0.5 + 0.5) * rect_size.x,
            (0.5 - ndc.y * 0.5) * rect_size.y,
        ))
    }

    /// Ray from the camera through a screen position (points inside
    /// `rect_size`). Returns (origin, direction).
    pub fn ray(&self, screen: Vec2, rect_size: Vec2) -> (Vec3, Vec3) {
        let inv = self.view_proj(rect_size.x / rect_size.y).inverse();
        let x = screen.x / rect_size.x * 2.0 - 1.0;
        let y = 1.0 - screen.y / rect_size.y * 2.0;
        // DirectX-style depth: 0 is the near plane, 1 the far plane.
        let near = inv.project_point3(Vec3::new(x, y, 0.0));
        let far = inv.project_point3(Vec3::new(x, y, 1.0));
        (near, (far - near).normalize_or(Vec3::NEG_Z))
    }

    /// Where a screen ray meets the plane through `point` facing the camera.
    pub fn screen_to_plane(&self, screen: Vec2, rect_size: Vec2, point: Vec3) -> Option<Vec3> {
        let (o, d) = self.ray(screen, rect_size);
        let n = (self.eye() - self.target).normalize_or(Vec3::Z);
        let denom = d.dot(n);
        if denom.abs() < 1e-6 {
            return None;
        }
        let t = (point - o).dot(n) / denom;
        (t > 0.0).then(|| o + d * t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ray_goes_through_projected_point() {
        let cam = OrbitCamera::default();
        let size = Vec2::new(800.0, 600.0);
        let p = Vec3::new(0.2, 1.1, -0.1);
        let s = cam.project(p, size).unwrap();
        let (o, d) = cam.ray(s, size);
        let closest = o + d * (p - o).dot(d);
        assert!(closest.distance(p) < 1e-3, "{closest} vs {p}");
    }
}
