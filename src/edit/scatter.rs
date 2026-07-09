//! Scatter areas: a lasso polygon + parameters, expanded deterministically
//! into instances (jittered grid seeded by the area) — nothing per-instance
//! is ever stored, so Bevy and the viewer can both regenerate the exact
//! same forest from `scatter.json`.

use std::path::Path;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::gen::heightfield::HeightSource;
use crate::gen::noise;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScatterArea {
    pub id: String,
    /// Asset path relative to the output dir, e.g. "assets/tre.glb".
    pub asset: String,
    /// Closed polygon in world meters.
    pub polygon: Vec<[f64; 2]>,
    pub seed: u64,
    /// Instances per hectare (before spacing/polygon rejection).
    pub density_ha: f64,
    /// Minimum spacing in meters (grid cell size).
    pub min_spacing: f64,
    #[serde(default = "yes")]
    pub rot_random: bool,
    #[serde(default = "one")]
    pub scale_min: f64,
    #[serde(default = "one")]
    pub scale_max: f64,
}

fn yes() -> bool {
    true
}

fn one() -> f64 {
    1.0
}

#[derive(Clone, Debug, Serialize)]
pub struct Instance {
    pub pos: [f64; 3],
    pub rot_y: f64,
    pub scale: f64,
}

/// Deterministic expansion: one candidate per `min_spacing` grid cell,
/// kept with probability density·cell_area, jittered, rejected outside
/// the polygon or in the sea, snapped to the composite terrain height.
pub fn instances(area: &ScatterArea, src: &HeightSource) -> Vec<Instance> {
    let mut out = Vec::new();
    if area.polygon.len() < 3 || area.min_spacing <= 0.1 {
        return out;
    }
    let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for p in &area.polygon {
        x0 = x0.min(p[0]);
        y0 = y0.min(p[1]);
        x1 = x1.max(p[0]);
        y1 = y1.max(p[1]);
    }
    let cell = area.min_spacing;
    let keep_p = (area.density_ha * cell * cell / 10_000.0).clamp(0.0, 1.0) as f32;
    let jitter = (cell * 0.45).max(0.0);

    let (cx0, cx1) = ((x0 / cell).floor() as i64, (x1 / cell).ceil() as i64);
    let (cy0, cy1) = ((y0 / cell).floor() as i64, (y1 / cell).ceil() as i64);
    for cy in cy0..=cy1 {
        for cx in cx0..=cx1 {
            if noise::hash2(area.seed, cx, cy) >= keep_p {
                continue;
            }
            let jx = (noise::hash2(area.seed ^ 0x11, cx, cy) as f64 * 2.0 - 1.0) * jitter;
            let jy = (noise::hash2(area.seed ^ 0x22, cx, cy) as f64 * 2.0 - 1.0) * jitter;
            let x = (cx as f64 + 0.5) * cell + jx;
            let y = (cy as f64 + 0.5) * cell + jy;
            if !inside(&area.polygon, x, y) {
                continue;
            }
            let h = src.height_at_m(x, y);
            if h <= 0.2 {
                continue; // never in the sea
            }
            let rot = if area.rot_random {
                noise::hash2(area.seed ^ 0x33, cx, cy) as f64 * std::f64::consts::TAU
            } else {
                0.0
            };
            let t = noise::hash2(area.seed ^ 0x44, cx, cy) as f64;
            let scale = area.scale_min + (area.scale_max - area.scale_min) * t;
            out.push(Instance { pos: [x, h as f64, y], rot_y: rot, scale });
        }
    }
    out
}

#[derive(Serialize)]
struct ScatterJson<'a> {
    areas: Vec<AreaJson<'a>>,
}

#[derive(Serialize)]
struct AreaJson<'a> {
    id: &'a str,
    asset: &'a str,
    instances: Vec<Instance>,
}

/// Expand every area and write `<output>/scatter.json` (atomic) — consumed
/// by the viewer and by Bevy.
pub fn write_all(output: &Path, areas: &[ScatterArea], src: &HeightSource) -> Result<()> {
    let doc = ScatterJson {
        areas: areas
            .iter()
            .map(|a| AreaJson { id: &a.id, asset: &a.asset, instances: instances(a, src) })
            .collect(),
    };
    let path = output.join("scatter.json");
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec(&doc)?)?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

/// True if the area's bbox intersects any of the given tiles.
pub fn touches(area: &ScatterArea, tiles: &[crate::tile::grid::TileId], tile_size: f64) -> bool {
    let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for p in &area.polygon {
        x0 = x0.min(p[0]);
        y0 = y0.min(p[1]);
        x1 = x1.max(p[0]);
        y1 = y1.max(p[1]);
    }
    tiles.iter().any(|t| {
        let tx0 = t.x as f64 * tile_size;
        let ty0 = t.y as f64 * tile_size;
        tx0 < x1 && tx0 + tile_size > x0 && ty0 < y1 && ty0 + tile_size > y0
    })
}

fn inside(poly: &[[f64; 2]], x: f64, y: f64) -> bool {
    let mut hit = false;
    let mut j = poly.len() - 1;
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[j]);
        if (a[1] > y) != (b[1] > y) && x < (b[0] - a[0]) * (y - a[1]) / (b[1] - a[1]) + a[0] {
            hit = !hit;
        }
        j = i;
    }
    hit
}
