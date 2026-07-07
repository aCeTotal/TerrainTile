use std::path::PathBuf;

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::ortho::source::{OrthoSource, Provider, DEFAULT_NIB_WMS, DEFAULT_XYZ};
use crate::pipeline::config::PipelineConfig;
use crate::server::inspect;
use crate::server::run;
use crate::server::state::SharedState;
use crate::tile::grid::TileGrid;
use crate::tile::masks::MaskParams;

type ApiError = (StatusCode, Json<Value>);

fn bad(msg: impl std::fmt::Display) -> ApiError {
    (StatusCode::BAD_REQUEST, Json(json!({ "error": msg.to_string() })))
}

/// GET /api/status — full snapshot + defaults for the config form.
pub async fn status(State(state): State<SharedState>) -> Json<Value> {
    let inner = state.inner.lock().unwrap();
    let has_dataset = inner
        .output
        .as_ref()
        .is_some_and(|o| o.join("dataset.json").is_file());
    Json(json!({
        "snapshot": inner.snapshot,
        "has_dataset": has_dataset,
        "defaults": {
            "wms_url": DEFAULT_NIB_WMS,
            "xyz_url": DEFAULT_XYZ,
            "masks": MaskParams::default(),
            "home": std::env::var("HOME").unwrap_or_else(|_| "/".into()),
        },
    }))
}

#[derive(Deserialize)]
pub struct BrowseQuery {
    path: Option<String>,
}

#[derive(Serialize)]
struct BrowseEntry {
    name: String,
    dir: bool,
}

/// GET /api/browse?path= — server-side directory listing so the browser can
/// pick input/output paths on the machine the pipeline runs on. Shows
/// directories plus raster/zip files.
pub async fn browse(Query(q): Query<BrowseQuery>) -> Result<Json<Value>, ApiError> {
    let path = PathBuf::from(
        q.path.unwrap_or_else(|| std::env::var("HOME").unwrap_or_else(|_| "/".into())),
    );
    let path = path.canonicalize().map_err(|e| bad(format!("{}: {e}", path.display())))?;
    let mut entries: Vec<BrowseEntry> = Vec::new();
    let rd = std::fs::read_dir(&path).map_err(|e| bad(format!("{}: {e}", path.display())))?;
    for entry in rd.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        let keep = is_dir
            || matches!(
                name.rsplit('.').next().map(str::to_lowercase).as_deref(),
                Some("tif") | Some("tiff") | Some("zip")
            );
        if keep {
            entries.push(BrowseEntry { name, dir: is_dir });
        }
    }
    entries.sort_by(|a, b| b.dir.cmp(&a.dir).then(a.name.cmp(&b.name)));
    Ok(Json(json!({
        "path": path.display().to_string(),
        "parent": path.parent().map(|p| p.display().to_string()),
        "entries": entries,
    })))
}

#[derive(Deserialize)]
pub struct ScanBody {
    paths: Vec<String>,
}

/// POST /api/scan — inspect the chosen height data. GDAL work runs on a
/// blocking thread; big datasets take time and must not stall the server.
pub async fn scan(
    State(state): State<SharedState>,
    Json(body): Json<ScanBody>,
) -> Result<Json<Value>, ApiError> {
    if body.paths.is_empty() {
        return Err(bad("ingen filer valgt"));
    }
    let paths: Vec<PathBuf> = body.paths.iter().map(PathBuf::from).collect();
    let paths2 = paths.clone();
    let result = tokio::task::spawn_blocking(move || inspect::inspect(&paths2))
        .await
        .map_err(|e| bad(format!("skanning feilet: {e}")))?
        .map_err(|e| bad(format!("{e:#}")))?;

    let dto = result.dto();
    let mut inner = state.inner.lock().unwrap();
    inner.scanned = match result {
        inspect::Inspect::Full(info) => Some((paths, info)),
        inspect::Inspect::Zip { .. } => None,
    };
    Ok(Json(serde_json::to_value(dto).unwrap()))
}

#[derive(Deserialize)]
pub struct GridQuery {
    tile_size_m: f64,
    lods: usize,
}

/// GET /api/grid — tile grid preview for the last scanned (non-zip) input.
pub async fn grid(
    State(state): State<SharedState>,
    Query(q): Query<GridQuery>,
) -> Result<Json<Value>, ApiError> {
    let inner = state.inner.lock().unwrap();
    let Some((_, info)) = &inner.scanned else {
        return Err(bad("ingen skannet høydedata"));
    };
    let grid = TileGrid::new(info, q.tile_size_m, q.lods).map_err(|e| bad(format!("{e:#}")))?;
    Ok(Json(json!({
        "tiles_x": grid.tiles_x,
        "tiles_y": grid.tiles_y,
        "count": grid.count(),
        "tile_px": grid.tile_px,
    })))
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum OrthoDto {
    Nib { username: String, password: String },
    Wms { url: String },
    Xyz { url: String, zoom: u8 },
}

#[derive(Deserialize)]
pub struct StartBody {
    inputs: Vec<String>,
    output: String,
    tile_size_m: f64,
    overlap: bool,
    lods: usize,
    threads: usize,
    nodata_height: f32,
    force: bool,
    masks: MaskParams,
    ortho: Option<OrthoDto>,
}

/// POST /api/start — start the pipeline.
pub async fn start(
    State(state): State<SharedState>,
    Json(body): Json<StartBody>,
) -> Result<Json<Value>, ApiError> {
    if body.inputs.is_empty() {
        return Err(bad("velg høydedata først"));
    }
    if body.output.trim().is_empty() {
        return Err(bad("velg utmappe først"));
    }
    let output = PathBuf::from(body.output.trim());
    let cfg = PipelineConfig {
        output: output.clone(),
        tile_size_m: body.tile_size_m,
        overlap: body.overlap,
        lods: body.lods.clamp(1, 8),
        threads: body.threads,
        nodata_height: body.nodata_height,
        force: body.force,
        masks: body.masks,
        ortho: body.ortho.map(|o| OrthoSource {
            provider: match o {
                OrthoDto::Nib { username, password } => Provider::Nib {
                    username: username.trim().to_string(),
                    password,
                },
                OrthoDto::Wms { url } => Provider::Wms { base_url: url },
                OrthoDto::Xyz { url, zoom } => Provider::Xyz { url_template: url, zoom },
            },
            cache_dir: output.join("cache"),
        }),
    };
    let inputs = body.inputs.iter().map(PathBuf::from).collect();
    run::start(&state, cfg, inputs).map_err(bad)?;
    Ok(Json(json!({ "ok": true })))
}

/// POST /api/cancel — request cancellation of the active run.
pub async fn cancel(State(state): State<SharedState>) -> Json<Value> {
    Json(json!({ "cancelled": run::cancel(&state) }))
}
