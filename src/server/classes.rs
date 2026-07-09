//! Material class endpoints: CRUD on the class list, material upload
//! (.zip or loose PBR maps, ambientCG-style names), and lasso paint of
//! class coverage.

use std::io::Read;
use std::path::Path;

use anyhow::{bail, Context, Result};
use axum::body::Bytes;
use axum::extract::{Path as UrlPath, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::server::project;
use crate::server::state::SharedState;
use crate::tile::classdef::{default_classes, ClassDef, MaterialRef};

type ApiError = (StatusCode, Json<Value>);

fn bad(msg: impl std::fmt::Display) -> ApiError {
    (StatusCode::BAD_REQUEST, Json(json!({ "error": msg.to_string() })))
}

/// Textures are stored at this edge length; arrays need equal sizes.
const TEX_SIZE: u32 = 1024;

pub const MAX_UPLOAD: usize = 300 * 1024 * 1024;

/// Viewer texture-array budget: albedo+normal+rough at 1K each.
const MAX_MATERIALS: usize = 16;

/// GET /api/classes — the open project's class list.
pub async fn list(State(state): State<SharedState>) -> Result<Json<Value>, ApiError> {
    let output = state.inner.lock().unwrap().output.clone().ok_or_else(|| bad("ingen prosjekt åpnet"))?;
    let p = project::load(&output).ok_or_else(|| bad("mappen mangler project.json"))?;
    Ok(Json(json!({ "classes": p.classes, "cover_res": crate::edit::cover::COVER_RES })))
}

#[derive(Deserialize)]
pub struct PutBody {
    classes: Vec<ClassDef>,
}

/// PUT /api/classes — replace the class list (gates, weights, order …).
/// Affected tiles rebuild via the class fingerprint on the next pass; we
/// kick the whole grid at the worker, which skips unchanged tiles fast.
pub async fn put(
    State(state): State<SharedState>,
    Json(body): Json<PutBody>,
) -> Result<Json<Value>, ApiError> {
    let total: usize = body.classes.iter().map(|c| c.materials.len()).sum();
    if total > MAX_MATERIALS {
        return Err(bad(format!("maks {MAX_MATERIALS} materialer totalt (GPU-teksturbudsjett)")));
    }
    let ctx = crate::server::edit::edit_ctx(&state)?;
    let output = state.inner.lock().unwrap().output.clone().ok_or_else(|| bad("ingen prosjekt"))?;
    let classes = body.classes.clone();
    project::update(&output, |p| p.classes = classes).map_err(|e| bad(format!("{e:#}")))?;
    // The context caches the class list — rebuild it on next use.
    state.inner.lock().unwrap().edit = None;
    let all: std::collections::BTreeSet<_> = ctx.grid.tiles().into_iter().collect();
    let _ = ctx.dirty_tx.send(all);
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
pub struct PaintBody {
    class: u32,
    polygon: Vec<[f64; 2]>,
    #[serde(default)]
    erase: bool,
}

/// POST /api/classes/paint — lasso polygon fill/erase of class coverage.
pub async fn paint(
    State(state): State<SharedState>,
    Json(body): Json<PaintBody>,
) -> Result<Json<Value>, ApiError> {
    let ctx = crate::server::edit::edit_ctx(&state)?;
    let dirty = tokio::task::spawn_blocking(move || {
        ctx.cover.fill_polygon(body.class, &body.polygon, body.erase).map(|d| (ctx, d))
    })
    .await
    .map_err(bad)?
    .map_err(|e| bad(format!("{e:#}")))?;
    let (ctx, dirty) = dirty;
    let names: Vec<String> = dirty.iter().map(|t| t.name()).collect();
    let _ = ctx.dirty_tx.send(dirty);
    Ok(Json(json!({ "tiles": names })))
}

/// POST /api/classes/{id}/material/{name} — upload a .zip or a loose map.
pub async fn upload(
    State(state): State<SharedState>,
    UrlPath((id, name)): UrlPath<(u32, String)>,
    body: Bytes,
) -> Result<Json<Value>, ApiError> {
    if body.is_empty() || body.len() > MAX_UPLOAD {
        return Err(bad("filen er tom eller for stor"));
    }
    let stem = name
        .rsplit_once('.')
        .map(|(s, _)| s.to_string())
        .unwrap_or_else(|| name.clone())
        .replace(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '-', "_");
    if stem.is_empty() {
        return Err(bad("ugyldig navn"));
    }
    let output = state.inner.lock().unwrap().output.clone().ok_or_else(|| bad("ingen prosjekt åpnet"))?;
    let mut p = project::load(&output).ok_or_else(|| bad("mappen mangler project.json"))?;
    let total: usize = p.classes.iter().map(|c| c.materials.len()).sum();
    if total >= MAX_MATERIALS {
        return Err(bad(format!("maks {MAX_MATERIALS} materialer totalt (GPU-teksturbudsjett)")));
    }
    let Some(class) = p.classes.iter_mut().find(|c| c.id == id) else {
        return Err(bad("ukjent klasse"));
    };

    let dir_rel = format!("materials/{id}/{stem}");
    let dir = output.join(&dir_rel);
    let lower = name.to_lowercase();
    let avg = tokio::task::spawn_blocking(move || -> Result<Option<String>> {
        std::fs::create_dir_all(&dir)?;
        if body.len() >= 4 && &body[0..2] == b"PK" {
            unpack_zip(&body, &dir)
        } else {
            let Some(kind) = detect(&lower) else {
                bail!("gjenkjenner ikke karttypen fra filnavnet (Color/Normal/Roughness…)");
            };
            save_map(&body, &dir, kind)
        }
    })
    .await
    .map_err(bad)?
    .map_err(|e| bad(format!("{e:#}")))?;

    if !class.materials.iter().any(|m| m.dir == dir_rel) {
        class.materials.push(MaterialRef { dir: dir_rel, amount: 1.0, mode: "mix".into() });
    }
    if let Some(avg) = avg {
        class.avg_color = avg;
    }
    let classes = p.classes.clone();
    project::update(&output, |pf| pf.classes = classes).map_err(|e| bad(format!("{e:#}")))?;
    state.inner.lock().unwrap().edit = None; // class list changed
    Ok(Json(json!({ "classes": p.classes })))
}

/// DELETE /api/classes/{id}/material/{name}
pub async fn remove(
    State(state): State<SharedState>,
    UrlPath((id, name)): UrlPath<(u32, String)>,
) -> Result<Json<Value>, ApiError> {
    let output = state.inner.lock().unwrap().output.clone().ok_or_else(|| bad("ingen prosjekt åpnet"))?;
    let dir_rel = format!("materials/{id}/{name}");
    let _ = std::fs::remove_dir_all(output.join(&dir_rel));
    project::update(&output, |p| {
        if let Some(c) = p.classes.iter_mut().find(|c| c.id == id) {
            c.materials.retain(|m| m.dir != dir_rel);
        }
    })
    .map_err(|e| bad(format!("{e:#}")))?;
    state.inner.lock().unwrap().edit = None;
    Ok(Json(json!({ "ok": true })))
}

/* ---------- helpers ---------- */

/// ambientCG-style map detection from a file name.
fn detect(lower: &str) -> Option<&'static str> {
    if lower.contains("color") || lower.contains("albedo") || lower.contains("diffuse") {
        Some("color")
    } else if lower.contains("normal") {
        Some("normal")
    } else if lower.contains("rough") {
        Some("rough")
    } else if lower.contains("displacement") || lower.contains("height") {
        Some("disp")
    } else if lower.contains("ambientocclusion") || lower.contains("_ao") {
        Some("ao")
    } else {
        None
    }
}

/// Decode → resize to TEX_SIZE² → save as `<dir>/<kind>.png`.
/// Returns the mean color for color maps.
fn save_map(bytes: &[u8], dir: &Path, kind: &str) -> Result<Option<String>> {
    let img = image::load_from_memory(bytes).context("kunne ikke dekode bildet")?;
    let img = img.resize_exact(TEX_SIZE, TEX_SIZE, image::imageops::FilterType::Triangle);
    img.save(dir.join(format!("{kind}.png")))?;
    if kind != "color" {
        return Ok(None);
    }
    let rgb = img.to_rgb8();
    let (mut r, mut g, mut b, mut n) = (0u64, 0u64, 0u64, 0u64);
    for px in rgb.pixels().step_by(17) {
        r += px[0] as u64;
        g += px[1] as u64;
        b += px[2] as u64;
        n += 1;
    }
    Ok(Some(format!("#{:02x}{:02x}{:02x}", r / n, g / n, b / n)))
}

fn unpack_zip(bytes: &[u8], dir: &Path) -> Result<Option<String>> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).context("ugyldig zip")?;
    let mut avg = None;
    for i in 0..zip.len() {
        let mut f = zip.by_index(i)?;
        if !f.is_file() {
            continue;
        }
        let lower = f.name().to_lowercase();
        if !(lower.ends_with(".png") || lower.ends_with(".jpg") || lower.ends_with(".jpeg")) {
            continue;
        }
        let Some(kind) = detect(&lower) else { continue };
        let mut buf = Vec::with_capacity(f.size() as usize);
        f.read_to_end(&mut buf)?;
        if let Some(a) = save_map(&buf, dir, kind)? {
            avg = Some(a);
        }
    }
    if !dir.join("color.png").exists() {
        bail!("zip-en inneholdt ingen gjenkjennbar Color-tekstur");
    }
    Ok(avg)
}

/// Install the default classes for a fresh project: the embedded ambientCG
/// sets are written under materials/<class>/<name>/ so everything flows
/// through the same upload path afterwards.
pub fn install_defaults(output: &Path) -> Result<Vec<ClassDef>> {
    const G003C: &[u8] = include_bytes!("../../web/materials/Ground003_2K-PNG_Color.png");
    const G003N: &[u8] = include_bytes!("../../web/materials/Ground003_2K-PNG_NormalGL.png");
    const G003R: &[u8] = include_bytes!("../../web/materials/Ground003_2K-PNG_Roughness.png");
    const G037C: &[u8] = include_bytes!("../../web/materials/Ground037_2K-PNG_Color.png");
    const G037N: &[u8] = include_bytes!("../../web/materials/Ground037_2K-PNG_NormalGL.png");
    const G037R: &[u8] = include_bytes!("../../web/materials/Ground037_2K-PNG_Roughness.png");
    const R051C: &[u8] = include_bytes!("../../web/materials/Rock051_2K-PNG_Color.png");
    const R051N: &[u8] = include_bytes!("../../web/materials/Rock051_2K-PNG_NormalGL.png");
    const R051R: &[u8] = include_bytes!("../../web/materials/Rock051_2K-PNG_Roughness.png");

    let mut classes = default_classes();
    let sets: [(&str, [&[u8]; 3]); 4] = [
        ("materials/1/Ground037", [G037C, G037N, G037R]),
        ("materials/2/Ground003", [G003C, G003N, G003R]),
        ("materials/3/Ground037", [G037C, G037N, G037R]),
        ("materials/4/Rock051", [R051C, R051N, R051R]),
    ];
    for (rel, [c, n, r]) in sets {
        let dir = output.join(rel);
        if dir.join("color.png").exists() {
            continue; // already installed (re-run)
        }
        std::fs::create_dir_all(&dir)?;
        let avg = save_map(c, &dir, "color")?;
        save_map(n, &dir, "normal")?;
        save_map(r, &dir, "rough")?;
        if let (Some(avg), Some(class)) = (
            avg,
            classes.iter_mut().find(|cl| cl.materials.iter().any(|m| m.dir == rel)),
        ) {
            class.avg_color = avg;
        }
    }
    Ok(classes)
}
