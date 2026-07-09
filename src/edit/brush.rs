//! Brush strokes applied to the edit overlays. The client previews the
//! same math on loaded geometry; the server side here is authoritative.

use std::collections::BTreeSet;

use anyhow::Result;
use serde::Deserialize;

use crate::gen::heightfield::HeightSource;
use crate::tile::grid::{TileGrid, TileId};

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tool {
    Raise,
    Lower,
    Flatten,
    Smooth,
}

#[derive(Clone, Copy, Debug, Deserialize)]
pub struct HeightStroke {
    pub tool: Tool,
    /// Center in world meters (mesh x/z convention).
    pub x: f64,
    pub z: f64,
    pub radius: f64,
    /// Raise/Lower: meters at the center per stroke event.
    /// Flatten/Smooth: lerp factor 0..1 per stroke event.
    pub strength: f32,
    /// Flatten target height, captured at stroke start by the client.
    pub target_h: Option<f32>,
}

#[inline]
fn falloff(dist: f64, radius: f64) -> f32 {
    let t = (1.0 - dist / radius).clamp(0.0, 1.0) as f32;
    t * t * (3.0 - 2.0 * t)
}

/// Apply height strokes to the delta overlays. Returns every tile whose
/// outputs are stale — the stroke's tiles plus the apron neighborhood
/// (neighbor normals read these samples). `apron_px = 1 << (lods-1)`.
pub fn apply_height(
    src: &HeightSource,
    grid: &TileGrid,
    apron_px: usize,
    strokes: &[HeightStroke],
) -> Result<BTreeSet<TileId>> {
    let store = src.edits();
    let res = grid.resolution;
    let n = grid.tile_px as i64;
    let mut dirty = BTreeSet::new();

    for s in strokes {
        // Affected sample range (delta samples are owned, so the last
        // world vertex column/row is untouchable — deep sea anyway).
        let px0 = (((s.x - s.radius) / res).floor() as i64).max(0);
        let px1 = (((s.x + s.radius) / res).ceil() as i64).min(grid.tiles_x as i64 * n - 1);
        let py0 = (((s.z - s.radius) / res).floor() as i64).max(0);
        let py1 = (((s.z + s.radius) / res).ceil() as i64).min(grid.tiles_y as i64 * n - 1);
        if px0 > px1 || py0 > py1 {
            continue;
        }

        for ty in py0.div_euclid(n)..=py1.div_euclid(n) {
            for tx in px0.div_euclid(n)..=px1.div_euclid(n) {
                let key = (tx as usize, ty as usize);
                store.modify_delta(key, |d| {
                    for py in py0.max(ty * n)..=py1.min(ty * n + n - 1) {
                        for px in px0.max(tx * n)..=px1.min(tx * n + n - 1) {
                            let dist =
                                (px as f64 * res - s.x).hypot(py as f64 * res - s.z);
                            let fall = falloff(dist, s.radius);
                            if fall <= 0.0 {
                                continue;
                            }
                            let i = ((py - ty * n) * n + (px - tx * n)) as usize;
                            match s.tool {
                                Tool::Raise => d[i] += s.strength * fall,
                                Tool::Lower => d[i] -= s.strength * fall,
                                Tool::Flatten => {
                                    let target =
                                        s.target_h.unwrap_or_else(|| src.sample_px(px, py));
                                    let cur = src.sample_px(px, py);
                                    d[i] += (target - cur) * fall * s.strength.clamp(0.0, 1.0);
                                }
                                Tool::Smooth => {
                                    // Box average of the pre-stroke composite.
                                    let mut sum = 0.0f32;
                                    for (dx, dy) in
                                        [(1, 0), (-1, 0), (0, 1), (0, -1), (0, 0)]
                                    {
                                        sum += src.sample_px(px + dx, py + dy);
                                    }
                                    let cur = src.sample_px(px, py);
                                    d[i] += (sum / 5.0 - cur)
                                        * fall
                                        * s.strength.clamp(0.0, 1.0);
                                }
                            }
                        }
                    }
                })?;
            }
        }
        mark_dirty(&mut dirty, grid, apron_px, s.x, s.z, s.radius);
    }
    Ok(dirty)
}

/// All tiles whose outputs depend on samples inside the circle — the
/// stroke bbox expanded by the apron (largest LOD stride), because
/// neighbors' normals read across the seam.
fn mark_dirty(
    dirty: &mut BTreeSet<TileId>,
    grid: &TileGrid,
    apron_px: usize,
    x: f64,
    z: f64,
    radius: f64,
) {
    let apron_m = grid.resolution * apron_px as f64;
    let t = grid.tile_size_m;
    let x0 = (((x - radius - apron_m) / t).floor() as i64).max(0);
    let x1 = (((x + radius + apron_m) / t).floor() as i64).min(grid.tiles_x as i64 - 1);
    let z0 = (((z - radius - apron_m) / t).floor() as i64).max(0);
    let z1 = (((z + radius + apron_m) / t).floor() as i64).min(grid.tiles_y as i64 - 1);
    for ty in z0..=z1 {
        for tx in x0..=x1 {
            dirty.insert(TileId { x: tx as usize, y: ty as usize });
        }
    }
}
