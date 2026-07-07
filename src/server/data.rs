use axum::extract::{Path, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};

use crate::server::state::SharedState;

/// GET /data/{*path} — serve files from the current output dir (dataset.json,
/// quadtree.json, tiles/…/mesh_lodN.bin, ortho.png) to the 3D viewer.
pub async fn file(
    State(state): State<SharedState>,
    Path(rel): Path<String>,
) -> Result<Response, StatusCode> {
    let root = {
        let inner = state.inner.lock().unwrap();
        inner.output.clone().ok_or(StatusCode::NOT_FOUND)?
    };
    // No traversal out of the output dir.
    if rel.split('/').any(|c| c == ".." || c.is_empty()) {
        return Err(StatusCode::BAD_REQUEST);
    }
    let path = root.join(&rel);
    let bytes = tokio::fs::read(&path).await.map_err(|_| StatusCode::NOT_FOUND)?;
    let mime = match path.extension().and_then(|e| e.to_str()) {
        Some("json") => "application/json",
        Some("png") => "image/png",
        Some("vrt") => "application/xml",
        _ => "application/octet-stream",
    };
    // Meshes and photos are immutable per build hash; metadata may change
    // between runs.
    let cache = if mime == "application/json" { "no-cache" } else { "max-age=60" };
    Ok((
        [(header::CONTENT_TYPE, mime), (header::CACHE_CONTROL, cache)],
        bytes,
    )
        .into_response())
}
