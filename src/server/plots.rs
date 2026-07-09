use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::server::project::{self, Plot, Zone, ZoneType};
use crate::server::state::SharedState;

type ApiError = (StatusCode, Json<Value>);

fn bad(msg: impl std::fmt::Display) -> ApiError {
    (StatusCode::BAD_REQUEST, Json(json!({ "error": msg.to_string() })))
}

#[derive(Deserialize)]
pub struct Body {
    plots: Vec<Plot>,
    zones: Vec<Zone>,
    zone_types: Vec<ZoneType>,
}

/// The Bevy-facing export: corners are the authoritative footprint
/// polygons the game extrudes; players never see any of this.
#[derive(Serialize)]
struct PlotsJson<'a> {
    version: u32,
    plots: &'a [Plot],
    zones: &'a [Zone],
    zone_types: &'a [ZoneType],
}

/// PUT /api/plots — replace plots/zones/types; also writes
/// `<output>/plots.json` for Bevy and the game server (purchase/build).
pub async fn put(
    State(state): State<SharedState>,
    Json(body): Json<Body>,
) -> Result<Json<Value>, ApiError> {
    let output = state.inner.lock().unwrap().output.clone().ok_or_else(|| bad("ingen prosjekt åpnet"))?;
    let doc = PlotsJson {
        version: 1,
        plots: &body.plots,
        zones: &body.zones,
        zone_types: &body.zone_types,
    };
    let bytes = serde_json::to_vec_pretty(&doc).map_err(bad)?;
    let path = output.join("plots.json");
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, bytes).map_err(bad)?;
    std::fs::rename(&tmp, &path).map_err(bad)?;

    project::update(&output, |p| {
        p.plots = body.plots;
        p.zones = body.zones;
        p.zone_types = body.zone_types;
    })
    .map_err(|e| bad(format!("{e:#}")))?;
    Ok(Json(json!({ "ok": true })))
}
