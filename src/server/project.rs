use std::path::{Path, PathBuf};

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::gen::world::WorldParams;
use crate::pipeline::config::PipelineConfig;
use crate::server::state::SharedState;
use crate::tile::classdef::ClassDef;

/// `<output>/project.json` — everything needed to reopen the project and
/// regenerate or continue it, including material classes, placed meshes
/// and roads.
#[derive(Serialize, Deserialize)]
pub struct ProjectFile {
    pub version: u32,
    pub world: WorldParams,
    #[serde(default)]
    pub classes: Vec<ClassDef>,
    #[serde(default)]
    pub placements: Vec<Placement>,
    #[serde(default)]
    pub splines: Vec<Spline>,
    #[serde(default)]
    pub scatter: Vec<crate::edit::scatter::ScatterArea>,
    #[serde(default)]
    pub plots: Vec<Plot>,
    #[serde(default)]
    pub zones: Vec<Zone>,
    #[serde(default)]
    pub zone_types: Vec<ZoneType>,
}

/// A purchasable plot: a quad with individually adjustable corners.
/// Editor/server/Bevy data only — never rendered for players.
#[derive(Serialize, Deserialize, Clone)]
pub struct Plot {
    pub id: String,
    pub number: u32,
    pub corners: [[f64; 2]; 4],
}

/// A building footprint: Bevy extrudes the quad following its type's
/// template when the building is generated.
#[derive(Serialize, Deserialize, Clone)]
pub struct Zone {
    pub id: String,
    /// Owning plot, when placed inside one.
    #[serde(default)]
    pub plot: Option<String>,
    #[serde(rename = "type")]
    pub kind: String,
    pub corners: [[f64; 2]; 4],
    #[serde(default = "one_floor")]
    pub floors: u32,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct ZoneType {
    pub name: String,
    pub color: String,
    #[serde(default = "one_floor")]
    pub floors: u32,
}

fn one_floor() -> u32 {
    1
}

/// A placed GLB instance (asset path is relative to the output dir).
#[derive(Serialize, Deserialize, Clone)]
pub struct Placement {
    pub id: String,
    pub asset: String,
    pub pos: [f64; 3],
    pub rot_y: f64,
    pub scale: f64,
}

/// A committed drag-stroke (roads etc.), dense polyline in world meters.
#[derive(Serialize, Deserialize, Clone)]
pub struct Spline {
    pub id: String,
    pub kind: String,
    pub width: f64,
    pub points: Vec<[f64; 2]>,
}

/// Save world/classes, preserving placements and splines already on disk.
pub fn save(cfg: &PipelineConfig) -> anyhow::Result<()> {
    let old = load(&cfg.output);
    let p = ProjectFile {
        version: 3,
        world: cfg.world,
        classes: cfg.classes.clone(),
        placements: old.as_ref().map(|o| o.placements.clone()).unwrap_or_default(),
        splines: old.as_ref().map(|o| o.splines.clone()).unwrap_or_default(),
        scatter: old.as_ref().map(|o| o.scatter.clone()).unwrap_or_default(),
        plots: old.as_ref().map(|o| o.plots.clone()).unwrap_or_default(),
        zones: old.as_ref().map(|o| o.zones.clone()).unwrap_or_default(),
        zone_types: old.map(|o| o.zone_types).unwrap_or_default(),
    };
    write(&cfg.output, &p)
}

pub fn load(output: &Path) -> Option<ProjectFile> {
    let bytes = std::fs::read(output.join("project.json")).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn write(output: &Path, p: &ProjectFile) -> anyhow::Result<()> {
    std::fs::create_dir_all(output)?;
    let path = output.join("project.json");
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(p)?)?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

/// Load-modify-write on the open project's file.
pub fn update(output: &Path, f: impl FnOnce(&mut ProjectFile)) -> anyhow::Result<()> {
    let mut p = load(output).ok_or_else(|| anyhow::anyhow!("project.json mangler"))?;
    f(&mut p);
    write(output, &p)
}

#[derive(Deserialize)]
pub struct PlacementsBody {
    placements: Vec<Placement>,
}

/// PUT /api/placements — replace the whole placement list (it is small and
/// the client debounces).
pub async fn save_placements(
    State(state): State<SharedState>,
    Json(body): Json<PlacementsBody>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let output = state.inner.lock().unwrap().output.clone();
    let Some(output) = output else {
        return Err((StatusCode::BAD_REQUEST, Json(json!({ "error": "ingen prosjekt åpnet" }))));
    };
    update(&output, |p| p.placements = body.placements)
        .map_err(|e| (StatusCode::BAD_REQUEST, Json(json!({ "error": format!("{e:#}") }))))?;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
pub struct OpenBody {
    path: String,
}

/// POST /api/open — open an existing project folder: point the server's
/// output at it (the viewer serves from there immediately) and return the
/// saved settings.
pub async fn open(
    State(state): State<SharedState>,
    Json(body): Json<OpenBody>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let bad = |msg: &str| (StatusCode::BAD_REQUEST, Json(json!({ "error": msg })));
    let path = PathBuf::from(body.path.trim());
    let project = load(&path);
    let has_dataset = path.join("dataset.json").is_file();
    if project.is_none() && !has_dataset {
        return Err(bad("mappen har verken project.json eller dataset.json"));
    }
    {
        let mut inner = state.inner.lock().unwrap();
        if inner.snapshot.running {
            return Err(bad("en jobb kjører — kan ikke bytte prosjekt nå"));
        }
        inner.output = Some(path.clone());
        inner.edit = None; // rebuild worker exits when its channel drops
        inner.snapshot.output = Some(path.display().to_string());
        inner.snapshot.report = None;
    }
    Ok(Json(json!({
        "output": path.display().to_string(),
        "has_dataset": has_dataset,
        "project": project,
    })))
}
