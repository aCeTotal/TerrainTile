use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde_json::{json, Value};

use crate::server::state::SharedState;

type ApiError = (StatusCode, Json<Value>);

fn bad(msg: impl std::fmt::Display) -> ApiError {
    (StatusCode::BAD_REQUEST, Json(json!({ "error": msg.to_string() })))
}

pub const MAX_GLB: usize = 100 * 1024 * 1024;

fn valid_name(name: &str) -> bool {
    !name.starts_with('.')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        && (name.ends_with(".glb") || name.ends_with(".gltf"))
}

/// POST /api/assets/{name} — store an uploaded GLB in `<output>/assets/`;
/// the viewer then loads it from `/data/assets/{name}`.
pub async fn upload(
    State(state): State<SharedState>,
    Path(name): Path<String>,
    body: Bytes,
) -> Result<Json<Value>, ApiError> {
    if !valid_name(&name) {
        return Err(bad("ugyldig filnavn — bruk .glb/.gltf uten spesialtegn"));
    }
    if body.is_empty() || body.len() > MAX_GLB {
        return Err(bad("filen er tom eller over 100 MB"));
    }
    let output = state.inner.lock().unwrap().output.clone().ok_or_else(|| bad("ingen prosjekt åpnet"))?;
    let dir = output.join("assets");
    tokio::fs::create_dir_all(&dir).await.map_err(bad)?;
    tokio::fs::write(dir.join(&name), &body).await.map_err(bad)?;
    Ok(Json(json!({ "ok": true, "asset": format!("assets/{name}") })))
}

/// GET /api/assets — the project's uploaded models.
pub async fn list(State(state): State<SharedState>) -> Result<Json<Value>, ApiError> {
    let output = state.inner.lock().unwrap().output.clone().ok_or_else(|| bad("ingen prosjekt åpnet"))?;
    let mut assets: Vec<String> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(output.join("assets")) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if valid_name(&name) {
                assets.push(name);
            }
        }
    }
    assets.sort();
    Ok(Json(json!({ "assets": assets })))
}
