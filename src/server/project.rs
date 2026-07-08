use std::path::PathBuf;

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::ortho::source::Provider;
use crate::pipeline::config::PipelineConfig;
use crate::server::state::SharedState;
use crate::tile::masks::MaskParams;

/// Run settings saved to `<output>/project.json` at every start, so the
/// folder can be reopened later and the run continued (resume). Passwords
/// are never stored.
#[derive(Serialize, Deserialize)]
pub struct ProjectFile {
    pub inputs: Vec<PathBuf>,
    pub tile_size_m: f64,
    pub overlap: bool,
    pub lods: usize,
    pub threads: usize,
    pub nodata_height: f32,
    pub masks: MaskParams,
    pub ortho: Option<OrthoSaved>,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum OrthoSaved {
    Nib { username: String },
    Wms { url: String },
    Xyz { url: String, zoom: u8 },
}

pub fn save(cfg: &PipelineConfig, inputs: &[PathBuf]) -> anyhow::Result<()> {
    let p = ProjectFile {
        inputs: inputs.to_vec(),
        tile_size_m: cfg.tile_size_m,
        overlap: cfg.overlap,
        lods: cfg.lods,
        threads: cfg.threads,
        nodata_height: cfg.nodata_height,
        masks: cfg.masks,
        ortho: cfg.ortho.as_ref().map(|o| match &o.provider {
            Provider::Nib { username, .. } => OrthoSaved::Nib { username: username.clone() },
            Provider::Wms { base_url } => OrthoSaved::Wms { url: base_url.clone() },
            Provider::Xyz { url_template, zoom } => {
                OrthoSaved::Xyz { url: url_template.clone(), zoom: *zoom }
            }
        }),
    };
    std::fs::create_dir_all(&cfg.output)?;
    std::fs::write(cfg.output.join("project.json"), serde_json::to_vec_pretty(&p)?)?;
    Ok(())
}

#[derive(Deserialize)]
pub struct OpenBody {
    path: String,
}

/// POST /api/open — open an existing project folder: point the server's
/// output at it (the viewer serves from there immediately) and return the
/// saved settings so the UI can prefill the form and continue the run.
pub async fn open(
    State(state): State<SharedState>,
    Json(body): Json<OpenBody>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let bad = |msg: &str| (StatusCode::BAD_REQUEST, Json(json!({ "error": msg })));
    let path = PathBuf::from(body.path.trim());
    let project = std::fs::read(path.join("project.json"))
        .ok()
        .and_then(|b| serde_json::from_slice::<ProjectFile>(&b).ok());
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
        inner.snapshot.output = Some(path.display().to_string());
        inner.snapshot.report = None;
    }
    Ok(Json(json!({
        "output": path.display().to_string(),
        "has_dataset": has_dataset,
        "config": project,
    })))
}
