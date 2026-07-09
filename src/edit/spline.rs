//! Road commit: flatten a strip along a dense polyline with a
//! slope-limited longitudinal profile, and paint the road mask under it.
//! Everything lands in the ordinary edit overlays, so seams and resume
//! keep working unchanged.

use std::collections::{BTreeSet, HashMap};

use anyhow::Result;

use crate::edit::cover::Cover;
use crate::gen::heightfield::HeightSource;
use crate::tile::grid::{TileGrid, TileId};

/// Max longitudinal grade (8 % ≈ a steep mountain road).
const MAX_GRADE: f32 = 0.08;

/// Roads never dip below this — a crossing over water becomes a causeway.
const MIN_H: f32 = 0.5;

#[allow(clippy::too_many_arguments)]
pub fn apply(
    src: &HeightSource,
    cover: &Cover,
    road_class: Option<u32>,
    grid: &TileGrid,
    apron_px: usize,
    width: f64,
    feather: f64,
    points: &[[f64; 2]],
) -> Result<BTreeSet<TileId>> {
    let mut dirty = BTreeSet::new();
    if points.len() < 2 {
        return Ok(dirty);
    }
    let profile = profile(src, points);
    let half = width / 2.0;
    let feather = feather.max(2.0 * grid.resolution);

    // Nearest-segment target per affected sample: (dist, target_h).
    let res = grid.resolution;
    let n = grid.tile_px as i64;
    let (w_px, h_px) = (grid.tiles_x as i64 * n, grid.tiles_y as i64 * n);
    let mut samples: HashMap<(i64, i64), (f64, f32)> = HashMap::new();
    for i in 0..points.len() - 1 {
        let (a, b) = (points[i], points[i + 1]);
        let (ha, hb) = (profile[i], profile[i + 1]);
        let pad = half + feather;
        let px0 = (((a[0].min(b[0]) - pad) / res).floor() as i64).max(0);
        let px1 = (((a[0].max(b[0]) + pad) / res).ceil() as i64).min(w_px - 1);
        let py0 = (((a[1].min(b[1]) - pad) / res).floor() as i64).max(0);
        let py1 = (((a[1].max(b[1]) + pad) / res).ceil() as i64).min(h_px - 1);
        let (abx, aby) = (b[0] - a[0], b[1] - a[1]);
        let len2 = (abx * abx + aby * aby).max(1e-9);
        for py in py0..=py1 {
            for px in px0..=px1 {
                let (x, y) = (px as f64 * res, py as f64 * res);
                let t = (((x - a[0]) * abx + (y - a[1]) * aby) / len2).clamp(0.0, 1.0);
                let d = (x - (a[0] + t * abx)).hypot(y - (a[1] + t * aby));
                if d > half + feather {
                    continue;
                }
                let target = ha + (hb - ha) * t as f32;
                let e = samples.entry((px, py)).or_insert((f64::MAX, 0.0));
                if d < e.0 {
                    *e = (d, target);
                }
            }
        }
    }

    // Group by owning tile and write the height deltas.
    type TileSamples = Vec<((i64, i64), (f64, f32))>;
    let mut by_tile: HashMap<(usize, usize), TileSamples> = HashMap::new();
    for (p, v) in &samples {
        let key = ((p.0.div_euclid(n)) as usize, (p.1.div_euclid(n)) as usize);
        by_tile.entry(key).or_default().push((*p, *v));
    }
    let store = src.edits();
    for (key, list) in &by_tile {
        store.modify_delta(*key, |d| {
            for ((px, py), (dist, target)) in list {
                let w = 1.0 - smooth(half, half + feather, *dist) as f32;
                if w <= 0.0 {
                    continue;
                }
                let cur = src.sample_px(*px, *py);
                let base = src.base_px(*px, *py);
                let i = ((py - key.1 as i64 * n) * n + (px - key.0 as i64 * n)) as usize;
                d[i] = cur + (target - cur) * w - base;
            }
        })?;
    }

    // Paint the road class's coverage along the strip (crisp roadway;
    // the class is `sharp`, bilinear sampling rounds the 8 m grid).
    if let Some(rc) = road_class {
        dirty.extend(cover.fill_polygon(rc, &strip_polygon(points, half), false)?);
    }

    // Dirty: the touched tiles plus the apron neighborhood.
    let apron_tiles = 1 + (apron_px as i64 - 1).max(0) / n;
    for key in by_tile.keys() {
        for dy in -apron_tiles..=apron_tiles {
            for dx in -apron_tiles..=apron_tiles {
                let (tx, ty) = (key.0 as i64 + dx, key.1 as i64 + dy);
                if tx >= 0 && ty >= 0 && tx < grid.tiles_x as i64 && ty < grid.tiles_y as i64 {
                    dirty.insert(TileId { x: tx as usize, y: ty as usize });
                }
            }
        }
    }
    Ok(dirty)
}

/// Closed polygon offsetting the polyline `half` meters to each side.
fn strip_polygon(points: &[[f64; 2]], half: f64) -> Vec<[f64; 2]> {
    let mut left = Vec::with_capacity(points.len());
    let mut right = Vec::with_capacity(points.len());
    for i in 0..points.len() {
        let a = points[i.saturating_sub(1)];
        let b = points[(i + 1).min(points.len() - 1)];
        let (mut dx, mut dy) = (b[0] - a[0], b[1] - a[1]);
        let l = dx.hypot(dy).max(1e-9);
        dx /= l;
        dy /= l;
        let p = points[i];
        left.push([p[0] - dy * half, p[1] + dx * half]);
        right.push([p[0] + dy * half, p[1] - dx * half]);
    }
    right.reverse();
    left.extend(right);
    left
}

/// Slope-limited, lightly smoothed height profile along the polyline.
fn profile(src: &HeightSource, points: &[[f64; 2]]) -> Vec<f32> {
    let mut h: Vec<f32> =
        points.iter().map(|p| src.height_at_m(p[0], p[1]).max(MIN_H)).collect();
    let ds: Vec<f32> = points
        .windows(2)
        .map(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]).max(0.1) as f32)
        .collect();
    for i in 1..h.len() {
        h[i] = h[i].clamp(h[i - 1] - MAX_GRADE * ds[i - 1], h[i - 1] + MAX_GRADE * ds[i - 1]);
    }
    for i in (0..h.len() - 1).rev() {
        h[i] = h[i].clamp(h[i + 1] - MAX_GRADE * ds[i], h[i + 1] + MAX_GRADE * ds[i]);
    }
    let copy = h.clone();
    for i in 1..h.len() - 1 {
        h[i] = (copy[i - 1] + 2.0 * copy[i] + copy[i + 1]) / 4.0;
    }
    h
}

#[inline]
fn smooth(e0: f64, e1: f64, x: f64) -> f64 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}
