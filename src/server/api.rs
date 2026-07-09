use std::path::PathBuf;

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::gen::world::WorldParams;
use crate::pipeline::config::PipelineConfig;
use crate::server::state::SharedState;
use crate::server::{classes, project, run};

type ApiError = (StatusCode, Json<Value>);

fn bad(msg: impl std::fmt::Display) -> ApiError {
    (StatusCode::BAD_REQUEST, Json(json!({ "error": msg.to_string() })))
}

/// GET /api/status — full snapshot + defaults for the new-project dialog.
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
            "world": WorldParams::default(),
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
/// pick the project folder on the machine the server runs on.
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
        if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            entries.push(BrowseEntry { name, dir: true });
        }
    }
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(Json(json!({
        "path": path.display().to_string(),
        "parent": path.parent().map(|p| p.display().to_string()),
        "entries": entries,
    })))
}

#[derive(Deserialize)]
pub struct NewProjectBody {
    output: String,
    world: WorldParams,
    #[serde(default)]
    threads: usize,
    #[serde(default)]
    force: bool,
}

/// POST /api/project/new — validate world params and start generation.
/// Also used to regenerate/continue an opened project. Fresh projects get
/// the default material classes and the embedded PBR sets installed.
pub async fn project_new(
    State(state): State<SharedState>,
    Json(body): Json<NewProjectBody>,
) -> Result<Json<Value>, ApiError> {
    if body.output.trim().is_empty() {
        return Err(bad("velg prosjektmappe først"));
    }
    body.world.validate().map_err(|e| bad(format!("{e:#}")))?;
    let grid = body.world.grid().map_err(|e| bad(format!("{e:#}")))?;
    let output = PathBuf::from(body.output.trim());
    let class_list = match project::load(&output) {
        Some(p) if !p.classes.is_empty() => p.classes,
        _ => {
            let out = output.clone();
            tokio::task::spawn_blocking(move || classes::install_defaults(&out))
                .await
                .map_err(bad)?
                .map_err(|e| bad(format!("{e:#}")))?
        }
    };
    let cfg = PipelineConfig {
        output,
        world: body.world,
        threads: body.threads,
        force: body.force,
        classes: class_list,
    };
    run::start(&state, cfg).map_err(bad)?;
    Ok(Json(json!({
        "ok": true,
        "tiles_x": grid.tiles_x,
        "tiles_y": grid.tiles_y,
        "count": grid.count(),
        "tile_px": grid.tile_px,
    })))
}

/// POST /api/cancel — request cancellation of the active run.
pub async fn cancel(State(state): State<SharedState>) -> Json<Value> {
    Json(json!({ "cancelled": run::cancel(&state) }))
}
