use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::edit::scatter::{self, ScatterArea};
use crate::server::project;
use crate::server::state::SharedState;

type ApiError = (StatusCode, Json<Value>);

fn bad(msg: impl std::fmt::Display) -> ApiError {
    (StatusCode::BAD_REQUEST, Json(json!({ "error": msg.to_string() })))
}

#[derive(Deserialize)]
pub struct Body {
    scatter: Vec<ScatterArea>,
}

/// PUT /api/scatter — replace the scatter area list; scatter.json (the
/// expanded instances) is rewritten immediately.
pub async fn put(
    State(state): State<SharedState>,
    Json(body): Json<Body>,
) -> Result<Json<Value>, ApiError> {
    let ctx = crate::server::edit::edit_ctx(&state)?;
    let output = state.inner.lock().unwrap().output.clone().ok_or_else(|| bad("ingen prosjekt"))?;
    let areas = body.scatter.clone();
    project::update(&output, |p| p.scatter = body.scatter).map_err(|e| bad(format!("{e:#}")))?;
    tokio::task::spawn_blocking(move || scatter::write_all(&output, &areas, &ctx.src))
        .await
        .map_err(bad)?
        .map_err(|e| bad(format!("{e:#}")))?;
    let _ = state.events.send(json!({ "type": "scatter" }).to_string());
    Ok(Json(json!({ "ok": true })))
}
