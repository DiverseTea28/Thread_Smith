//! Putting a garment on a body.
//!
//! 1. [`align_to_body`]: if the garment came with the body it was made on,
//!    work out how to scale and move that body onto ours.
//! 2. [`fit_to_body`]: move each garment vertex so it keeps the same gap to
//!    the new body as it had to its old body.
//! 3. [`push_out`]: push any part that sinks into the skin back out.

use glam::{Mat4, Vec3};

use crate::bvh::TriBvh;
use crate::graph::WeldGraph;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct FitSettings {
    /// Smallest gap kept between garment and skin, in meters.
    pub gap: f32,
    /// How much the fit movement is smoothed (more = keeps the garment's
    /// shape better, less = hugs the new body more closely).
    pub smoothing: usize,
    /// Gaps bigger than this (meters) are treated as loose cloth and not copied.
    pub max_offset: f32,
    /// Rounds of "push out of the skin".
    pub push_iterations: usize,
    /// Shrink wrap strength for garments without a reference body: 0 only
    /// pushes the garment out of the skin, 1 pulls loose cloth all the way
    /// in to `gap` from the skin.
    pub tightness: f32,
}

impl Default for FitSettings {
    fn default() -> Self {
        Self {
            gap: 0.004,
            smoothing: 10,
            max_offset: 0.15,
            push_iterations: 6,
            tightness: 0.7,
        }
    }
}

/// Weld tolerance: vertices closer than 0.1 mm are the same point.
const WELD: f32 = 1e-4;

/// How to move `source_body` onto `target_body`: lines up the floor, scales
/// to the same height, and scales width/depth by chest depth (arms don't
/// change chest depth, so this works whatever the arm pose).
pub fn align_to_body(source_body: &[Vec3], target_body: &[Vec3]) -> Mat4 {
    struct Measure {
        floor: f32,
        height: f32,
        chest_depth: f32,
        chest_center: Vec3,
    }
    let measure = |pts: &[Vec3]| -> Option<Measure> {
        let floor = pts.iter().map(|p| p.y).fold(f32::MAX, f32::min);
        let top = pts.iter().map(|p| p.y).fold(f32::MIN, f32::max);
        let full = top - floor;
        if full <= 0.0 {
            return None;
        }
        // Height is measured from the crotch, not the top of the head, so a body
        // without a head (a MetaHuman body mesh) measures the same as one with.
        // A MetaHuman crotch sits at about 40% of the full height.
        let height = crotch_height(pts, floor, full)
            .map(|c| c / 0.40)
            .unwrap_or(full);
        // A thin slice at chest height.
        let (lo, hi) = (floor + height * 0.70, floor + height * 0.76);
        let band: Vec<Vec3> = pts
            .iter()
            .copied()
            .filter(|p| p.y >= lo && p.y <= hi)
            .collect();
        if band.len() < 10 {
            return None;
        }
        let zmin = band.iter().map(|p| p.z).fold(f32::MAX, f32::min);
        let zmax = band.iter().map(|p| p.z).fold(f32::MIN, f32::max);
        let mut xs: Vec<f32> = band.iter().map(|p| p.x).collect();
        xs.sort_by(f32::total_cmp);
        let cx = xs[xs.len() / 2];
        Some(Measure {
            floor,
            height,
            chest_depth: zmax - zmin,
            chest_center: Vec3::new(cx, 0.0, (zmin + zmax) * 0.5),
        })
    };
    let (Some(s), Some(t)) = (measure(source_body), measure(target_body)) else {
        return Mat4::IDENTITY;
    };
    let sy = t.height / s.height;
    let sxz = if s.chest_depth > 1e-4 {
        t.chest_depth / s.chest_depth
    } else {
        sy
    };
    let scale = Vec3::new(sxz, sy, sxz);
    let from = Vec3::new(s.chest_center.x, s.floor, s.chest_center.z);
    let to = Vec3::new(t.chest_center.x, t.floor, t.chest_center.z);
    Mat4::from_translation(to) * Mat4::from_scale(scale) * Mat4::from_translation(-from)
}

/// Height of the crotch above the floor: the lowest point near the middle of the
/// body (between the legs nothing is there). `None` if it doesn't look like legs.
fn crotch_height(pts: &[Vec3], floor: f32, full: f32) -> Option<f32> {
    // Middle of the legs, from points in the lower body.
    let xs: Vec<f32> = pts
        .iter()
        .filter(|p| p.y < floor + full * 0.3)
        .map(|p| p.x)
        .collect();
    if xs.len() < 10 {
        return None;
    }
    // Halfway between the outsides of the legs (a median would land inside
    // one leg, since half the points are on each side).
    let lo = xs.iter().copied().fold(f32::MAX, f32::min);
    let hi = xs.iter().copied().fold(f32::MIN, f32::max);
    let cx = (lo + hi) * 0.5;
    // Walk down the middle line from the belly in thin slices. The first
    // empty stretch is the gap between the legs; the crotch is just above it.
    let half_width = full * 0.015;
    let slice = full * 0.01;
    let count = (full * 0.65 / slice) as usize;
    let mut filled = vec![false; count + 1];
    for p in pts {
        if (p.x - cx).abs() < half_width {
            let i = ((p.y - floor) / slice) as usize;
            if i <= count {
                filled[i] = true;
            }
        }
    }
    let mut i = count;
    while i >= 2 && (filled[i] || filled[i - 1]) {
        i -= 1;
    }
    let crotch = (i + 1) as f32 * slice;
    let ratio = crotch / full;
    (0.30..0.60).contains(&ratio).then_some(crotch)
}

/// Fit garment vertices onto `target`.
///
/// With `source` (the body the garment was made on, already aligned with
/// [`align_to_body`]): each vertex keeps its gap to the body, measured on the
/// new body instead of the old one. Without it: [`shrink_wrap`] pulls loose
/// cloth in by `settings.tightness`. Then [`push_out`] runs.
pub fn fit_to_body(
    positions: &mut [Vec3],
    indices: &[u32],
    source: Option<&TriBvh>,
    target: &TriBvh,
    settings: &FitSettings,
) {
    let graph = WeldGraph::new(positions, indices, WELD);
    if let Some(source) = source {
        let welded = graph.gather(positions);
        let mut disp = vec![Vec3::ZERO; welded.len()];
        let mut known = vec![false; welded.len()];
        for (i, &v) in welded.iter().enumerate() {
            let Some(a) = source.closest(v) else { continue };
            let gap = (v - a.point).dot(a.normal);
            if gap > settings.max_offset {
                continue; // loose cloth far from the body: filled in from neighbours
            }
            let Some(b) = target.closest(a.point) else {
                continue;
            };
            if a.normal.dot(b.normal) < 0.3 {
                continue; // the two bodies disagree here (e.g. armpit): fill in
            }
            let new_pos = b.point + b.normal * gap.max(settings.gap);
            disp[i] = new_pos - v;
            known[i] = true;
        }
        fill_unknown(&graph, &mut disp, &mut known);
        graph.smooth(&mut disp, settings.smoothing);
        for (v, w) in positions.iter_mut().zip(&graph.weld_of) {
            *v += disp[*w as usize];
        }
    } else if settings.tightness > 0.0 {
        shrink_wrap_with_graph(positions, &graph, target, settings);
    }
    push_out_with_graph(
        positions,
        &graph,
        target,
        settings.gap,
        settings.push_iterations,
    );
}

/// Pull loose garment vertices in towards the body (like a shrink wrap).
/// Cloth further than `settings.max_offset` from the skin is left loose; the
/// movement is smoothed so the garment keeps its own shape.
pub fn shrink_wrap(
    positions: &mut [Vec3],
    indices: &[u32],
    target: &TriBvh,
    settings: &FitSettings,
) {
    let graph = WeldGraph::new(positions, indices, WELD);
    shrink_wrap_with_graph(positions, &graph, target, settings);
}

fn shrink_wrap_with_graph(
    positions: &mut [Vec3],
    graph: &WeldGraph,
    target: &TriBvh,
    settings: &FitSettings,
) {
    let welded = graph.gather(positions);
    let mut disp = vec![Vec3::ZERO; welded.len()];
    for (i, &v) in welded.iter().enumerate() {
        let Some(h) = target.closest(v) else { continue };
        let side = (v - h.point).dot(h.normal);
        if side <= settings.gap || side > settings.max_offset {
            continue;
        }
        // Fade out towards max_offset so loose hems don't snap in.
        let fade = 1.0 - ((side - settings.gap) / (settings.max_offset - settings.gap)).powi(2);
        disp[i] = -h.normal * (side - settings.gap) * settings.tightness.clamp(0.0, 1.0) * fade;
    }
    graph.smooth(&mut disp, settings.smoothing);
    for (v, w) in positions.iter_mut().zip(&graph.weld_of) {
        *v += disp[*w as usize];
    }
}

/// Share of garment vertices that are inside the body (or closer than
/// `gap`). Checks at most `max_samples` vertices, spread evenly.
pub fn inside_fraction(positions: &[Vec3], target: &TriBvh, gap: f32, max_samples: usize) -> f32 {
    if positions.is_empty() {
        return 0.0;
    }
    let step = (positions.len() / max_samples.max(1)).max(1);
    let (mut inside, mut total) = (0usize, 0usize);
    for v in positions.iter().step_by(step) {
        if let Some(h) = target.closest(*v) {
            total += 1;
            if (*v - h.point).dot(h.normal) < gap {
                inside += 1;
            }
        }
    }
    if total == 0 {
        0.0
    } else {
        inside as f32 / total as f32
    }
}

/// Smallest uniform scale (around `pivot`, from 1.0 up to `max_scale`) that
/// leaves no more than `allowed` of the garment inside the body. `None` if
/// even `max_scale` isn't enough.
pub fn grow_to_clear(
    positions: &[Vec3],
    pivot: Vec3,
    target: &TriBvh,
    allowed: f32,
    max_scale: f32,
) -> Option<f32> {
    let mut s = 1.0f32;
    let mut scratch = positions.to_vec();
    while s <= max_scale {
        for (o, p) in scratch.iter_mut().zip(positions) {
            *o = pivot + (*p - pivot) * s;
        }
        if inside_fraction(&scratch, target, 0.0, 3000) <= allowed {
            return Some(s);
        }
        s *= 1.02;
    }
    None
}

/// Push any garment vertex that is inside the body (or closer than `gap`)
/// back out along the body's surface normal.
pub fn push_out(
    positions: &mut [Vec3],
    indices: &[u32],
    target: &TriBvh,
    gap: f32,
    iterations: usize,
) {
    let graph = WeldGraph::new(positions, indices, WELD);
    push_out_with_graph(positions, &graph, target, gap, iterations);
}

fn push_out_with_graph(
    positions: &mut [Vec3],
    graph: &WeldGraph,
    target: &TriBvh,
    gap: f32,
    iterations: usize,
) {
    let start = graph.gather(positions);
    let mut welded = start.clone();
    for round in 0..iterations.max(1) {
        let mut moved = false;
        for v in welded.iter_mut() {
            let Some(h) = target.closest(*v) else {
                continue;
            };
            let side = (*v - h.point).dot(h.normal);
            if side < gap {
                *v += h.normal * (gap - side);
                moved = true;
            }
        }
        if !moved {
            break;
        }
        // Smooth the total movement so pushed areas don't get dents and
        // ridges. The last round skips this so nothing is left inside.
        if round + 1 < iterations {
            let mut total: Vec<Vec3> = welded.iter().zip(&start).map(|(a, b)| *a - *b).collect();
            graph.smooth(&mut total, 2);
            for ((v, s), t) in welded.iter_mut().zip(&start).zip(&total) {
                *v = *s + *t;
            }
        }
    }
    for (v, w) in positions.iter_mut().zip(&graph.weld_of) {
        *v = welded[*w as usize];
    }
}

/// Give unknown entries the average of their known neighbours, spreading
/// outwards until everything connected to a known value is filled.
fn fill_unknown(graph: &WeldGraph, values: &mut [Vec3], known: &mut [bool]) {
    loop {
        let mut changed = false;
        let snapshot = known.to_vec();
        for i in 0..values.len() {
            if snapshot[i] {
                continue;
            }
            let mut sum = Vec3::ZERO;
            let mut n = 0;
            for &j in &graph.neighbours[i] {
                if snapshot[j as usize] {
                    sum += values[j as usize];
                    n += 1;
                }
            }
            if n > 0 {
                values[i] = sum / n as f32;
                known[i] = true;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn box_mesh(half: f32) -> (Vec<Vec3>, Vec<u32>) {
        // A closed cube with outward faces.
        let p = |x: f32, y: f32, z: f32| Vec3::new(x, y, z) * half;
        let v = vec![
            p(-1., -1., -1.),
            p(1., -1., -1.),
            p(1., 1., -1.),
            p(-1., 1., -1.),
            p(-1., -1., 1.),
            p(1., -1., 1.),
            p(1., 1., 1.),
            p(-1., 1., 1.),
        ];
        let i = vec![
            0, 2, 1, 0, 3, 2, 4, 5, 6, 4, 6, 7, 0, 1, 5, 0, 5, 4, 3, 6, 2, 3, 7, 6, 0, 4, 7, 0, 7,
            3, 1, 2, 6, 1, 6, 5,
        ];
        (v, i)
    }

    #[test]
    fn push_out_moves_points_outside() {
        let (bv, bi) = box_mesh(1.0);
        let body = TriBvh::new(&bv, &[], &bi);
        let mut pts = vec![Vec3::new(0.9, 0.0, 0.0), Vec3::new(0.0, 0.5, 0.95)];
        push_out(&mut pts, &[], &body, 0.01, 4);
        assert!(pts[0].x >= 1.0 && pts[1].z >= 1.0, "{pts:?}");
    }

    #[test]
    fn shrink_wrap_pulls_loose_points_in() {
        let (bv, bi) = box_mesh(1.0);
        let body = TriBvh::new(&bv, &[], &bi);
        let mut pts = vec![Vec3::new(1.03, 0.0, 0.0)];
        let settings = FitSettings {
            tightness: 1.0,
            smoothing: 0,
            ..Default::default()
        };
        shrink_wrap(&mut pts, &[], &body, &settings);
        assert!((pts[0].x - (1.0 + settings.gap)).abs() < 0.003, "{pts:?}");
    }

    #[test]
    fn grow_clears_small_garment() {
        let (bv, bi) = box_mesh(1.0);
        let body = TriBvh::new(&bv, &[], &bi);
        // A "shirt" ring that is too small (inside the box).
        let ring: Vec<Vec3> = (0..64)
            .map(|i| {
                let a = i as f32 / 64.0 * std::f32::consts::TAU;
                Vec3::new(a.cos() * 0.8, 0.0, a.sin() * 0.8)
            })
            .collect();
        let s = grow_to_clear(&ring, Vec3::ZERO, &body, 0.02, 3.0).unwrap();
        assert!(s > 1.2 && s < 2.0, "{s}");
    }

    #[test]
    fn align_scales_height() {
        let (a, _) = box_mesh(1.0);
        let b: Vec<Vec3> = a.iter().map(|p| *p * 2.0).collect();
        // Need points at chest height: add a column of points.
        let column =
            |s: f32| (0..1000).map(move |i| Vec3::new(0.0, -s + 2.0 * s * i as f32 / 999.0, 0.0));
        let mut src = a.clone();
        src.extend(column(1.0));
        let mut dst = b.clone();
        dst.extend(column(2.0));
        let m = align_to_body(&src, &dst);
        let top = m.transform_point3(Vec3::new(0.0, 1.0, 0.0));
        assert!((top.y - 2.0).abs() < 1e-4, "{top}");
    }
}
