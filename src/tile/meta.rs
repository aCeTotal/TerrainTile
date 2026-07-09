use std::collections::BTreeMap;
use std::path::Path;

use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Bbox {
    pub west: f64,
    pub south: f64,
    pub east: f64,
    pub north: f64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Neighbors {
    pub north: Option<String>,
    pub south: Option<String>,
    pub east: Option<String>,
    pub west: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct LodEntry {
    pub level: usize,
    pub step_m: f64,
    pub file: String,
    pub vertices: usize,
    pub triangles: usize,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TileMeta {
    pub id: String,
    pub x: usize,
    pub y: usize,
    pub bbox: Bbox,
    pub min_height: f32,
    pub max_height: f32,
    pub average_slope_deg: f32,
    pub average_normal: [f32; 3],
    /// [easting, northing, height] of the tile center.
    pub center: [f64; 3],
    pub neighbors: Neighbors,
    /// Pure open sea: minimal quad meshes, no mask/class files.
    #[serde(default)]
    pub flat: bool,
    pub lods: Vec<LodEntry>,
    pub masks: BTreeMap<String, String>,
    /// Build fingerprints for incremental rebuilds; empty in old datasets,
    /// which simply forces a rebuild once.
    #[serde(default)]
    pub mesh_hash: String,
    #[serde(default)]
    pub masks_hash: String,
}

/// Write via temp file + rename so a present metadata.json always means the
/// tile completed — this is the resume marker.
pub fn write_atomic(path: &Path, meta: &TileMeta) -> Result<()> {
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(meta)?)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}
