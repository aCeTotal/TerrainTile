use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::edit::brush::{self, HeightStroke};
use crate::edit::cover::Cover;
use crate::edit::store::EditStore;
use crate::edit::{rebuild, spline as road};
use crate::gen::heightfield::HeightSource;
use crate::pipeline::config::PipelineConfig;
use crate::server::project::{self, Spline};
use crate::server::state::{EditCtx, SharedState};

type ApiError = (StatusCode, Json<Value>);

fn bad(msg: impl std::fmt::Display) -> ApiError {
    (StatusCode::BAD_REQUEST, Json(json!({ "error": msg.to_string() })))
}

/// The edit context for the open project, created on first use: composite
/// height source + coverage over the saved world params, and a rebuild
/// worker.
pub fn edit_ctx(state: &SharedState) -> Result<Arc<EditCtx>, ApiError> {
    let output = {
        let inner = state.inner.lock().unwrap();
        if inner.snapshot.running {
            return Err(bad("generering pågår — vent til den er ferdig"));
        }
        if let Some(ctx) = &inner.edit {
            return Ok(ctx.clone());
        }
        inner.output.clone().ok_or_else(|| bad("ingen prosjekt åpnet"))?
    };
    let p = project::load(&output).ok_or_else(|| bad("mappen mangler project.json"))?;
    let grid = p.world.grid().map_err(|e| bad(format!("{e:#}")))?;
    let src = Arc::new(HeightSource::new(
        p.world,
        Arc::new(EditStore::open(&output, &grid)),
    ));
    let cover = Arc::new(Cover::open(&output, &grid));
    let cfg = PipelineConfig {
        output: output.clone(),
        world: p.world,
        threads: 0,
        force: false,
        classes: p.classes.clone(),
    };
    let dirty_tx = rebuild::spawn(state.clone(), cfg, grid.clone(), src.clone(), cover.clone());
    let ctx = Arc::new(EditCtx {
        world: p.world,
        classes: p.classes,
        grid,
        src,
        cover,
        dirty_tx,
    });
    state.inner.lock().unwrap().edit = Some(ctx.clone());
    Ok(ctx)
}

#[derive(Deserialize)]
pub struct HeightBody {
    strokes: Vec<HeightStroke>,
}

/// POST /api/edit/height — apply sculpt strokes; affected tiles rebuild in
/// the background and an SSE `tiles` event follows.
pub async fn height(
    State(state): State<SharedState>,
    Json(body): Json<HeightBody>,
) -> Result<Json<Value>, ApiError> {
    let ctx = edit_ctx(&state)?;
    let apron = 1usize << (ctx.world.lods - 1);
    let dirty = tokio::task::spawn_blocking(move || {
        brush::apply_height(&ctx.src, &ctx.grid, apron, &body.strokes)
            .map(|d| (ctx, d))
    })
    .await
    .map_err(bad)?
    .map_err(|e| bad(format!("{e:#}")))?;
    let (ctx, dirty) = dirty;
    let names: Vec<String> = dirty.iter().map(|t| t.name()).collect();
    let _ = ctx.dirty_tx.send(dirty);
    Ok(Json(json!({ "tiles": names })))
}

/// POST /api/edit/spline — commit a road: persist it in project.json,
/// flatten the strip, paint the road class under it, rebuild.
pub async fn spline(
    State(state): State<SharedState>,
    Json(body): Json<Spline>,
) -> Result<Json<Value>, ApiError> {
    if body.points.len() < 2 || body.width <= 0.0 {
        return Err(bad("veien trenger minst to punkter og en bredde"));
    }
    let ctx = edit_ctx(&state)?;
    let output = state.inner.lock().unwrap().output.clone().ok_or_else(|| bad("ingen prosjekt"))?;
    let saved = body.clone();
    project::update(&output, |p| {
        p.splines.retain(|s| s.id != saved.id);
        p.splines.push(saved);
    })
    .map_err(|e| bad(format!("{e:#}")))?;

    let apron = 1usize << (ctx.world.lods - 1);
    let road_class = ctx.classes.iter().find(|c| c.road).map(|c| c.id);
    let dirty = tokio::task::spawn_blocking(move || {
        road::apply(
            &ctx.src,
            &ctx.cover,
            road_class,
            &ctx.grid,
            apron,
            body.width,
            body.width / 2.0,
            &body.points,
        )
        .map(|d| (ctx, d))
    })
    .await
    .map_err(bad)?
    .map_err(|e| bad(format!("{e:#}")))?;
    let (ctx, dirty) = dirty;
    let names: Vec<String> = dirty.iter().map(|t| t.name()).collect();
    let _ = ctx.dirty_tx.send(dirty);
    Ok(Json(json!({ "tiles": names })))
}

/// POST /api/edit/conform — "Tilpass terreng": widen and re-blend every
/// road into the terrain and re-snap all placed meshes to the ground.
pub async fn conform(State(state): State<SharedState>) -> Result<Json<Value>, ApiError> {
    let ctx = edit_ctx(&state)?;
    let output = state.inner.lock().unwrap().output.clone().ok_or_else(|| bad("ingen prosjekt"))?;
    let p = project::load(&output).ok_or_else(|| bad("mappen mangler project.json"))?;
    let apron = 1usize << (ctx.world.lods - 1);
    let road_class = ctx.classes.iter().find(|c| c.road).map(|c| c.id);
    let dirty = tokio::task::spawn_blocking(move || {
        crate::edit::conform::apply(
            &output,
            &ctx.src,
            &ctx.cover,
            road_class,
            &ctx.grid,
            apron,
            &p.splines,
        )
        .map(|d| (ctx, d))
    })
    .await
    .map_err(bad)?
    .map_err(|e| bad(format!("{e:#}")))?;
    let (ctx, dirty) = dirty;
    let names: Vec<String> = dirty.iter().map(|t| t.name()).collect();
    let _ = ctx.dirty_tx.send(dirty);
    let _ = state.events.send(json!({ "type": "conform" }).to_string());
    Ok(Json(json!({ "tiles": names })))
}
