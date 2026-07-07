use std::path::Path;
use std::sync::atomic::Ordering;

use anyhow::{Context, Result};
use axum::extract::State;
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use image::RgbImage;
use rayon::prelude::*;
use serde::Deserialize;

use crate::server::far::fresh;
use crate::server::state::SharedState;

/// Longest edge of the overview mosaic in pixels.
const MAX_PX: u32 = 4096;

/// GET /data/overview.png — one downscaled mosaic of every tile's
/// orthophoto, used to texture the whole-terrain layer. Built once in the
/// background (202 while building, 404 if the dataset has no orthophotos),
/// cached under `cache/overview.png` until the next run.
pub async fn overview(State(state): State<SharedState>) -> Result<Response, StatusCode> {
    let root = state.output().ok_or(StatusCode::NOT_FOUND)?;
    let stamp = root.join("dataset.json");
    let cache = root.join("cache").join("overview.png");
    if fresh(&cache, &stamp) {
        let bytes = tokio::fs::read(&cache).await.map_err(|_| StatusCode::NOT_FOUND)?;
        return Ok((
            [
                (header::CONTENT_TYPE, "image/png"),
                (header::CACHE_CONTROL, "no-cache"),
            ],
            bytes,
        )
            .into_response());
    }
    if fresh(&root.join("cache").join("overview.none"), &stamp) {
        return Err(StatusCode::NOT_FOUND);
    }
    if !state.overview_building.swap(true, Ordering::SeqCst) {
        let state2 = state.clone();
        tokio::task::spawn_blocking(move || {
            if let Err(e) = build(&root) {
                eprintln!("overview.png: {e:#}");
            }
            state2.overview_building.store(false, Ordering::SeqCst);
        });
    }
    Err(StatusCode::ACCEPTED)
}

#[derive(Deserialize)]
struct Ds {
    tiles_x: usize,
    tiles_y: usize,
}

fn build(root: &Path) -> Result<()> {
    let ds: Ds = serde_json::from_slice(&std::fs::read(root.join("dataset.json"))?)
        .context("dataset.json")?;
    let cell = (MAX_PX / ds.tiles_x.max(ds.tiles_y) as u32).clamp(1, 64);

    // Decode + shrink in parallel; tiles without ortho.png stay neutral.
    let coords: Vec<(usize, usize)> = (0..ds.tiles_y)
        .flat_map(|y| (0..ds.tiles_x).map(move |x| (x, y)))
        .collect();
    let thumbs: Vec<((usize, usize), Option<RgbImage>)> = coords
        .par_iter()
        .map(|&(x, y)| {
            let path = root.join("tiles").join(format!("tile_x{x}_y{y}")).join("ortho.png");
            let thumb = image::open(&path)
                .ok()
                .map(|img| img.thumbnail_exact(cell, cell).to_rgb8());
            ((x, y), thumb)
        })
        .collect();

    let cache_dir = root.join("cache");
    std::fs::create_dir_all(&cache_dir)?;
    if thumbs.iter().all(|(_, t)| t.is_none()) {
        std::fs::write(cache_dir.join("overview.none"), b"")?;
        return Ok(());
    }

    let mut mosaic = RgbImage::from_pixel(
        ds.tiles_x as u32 * cell,
        ds.tiles_y as u32 * cell,
        image::Rgb([110, 125, 100]),
    );
    for ((x, y), thumb) in thumbs {
        if let Some(t) = thumb {
            image::imageops::replace(&mut mosaic, &t, (x as u32 * cell) as i64, (y as u32 * cell) as i64);
        }
    }
    let tmp = cache_dir.join("overview.png.tmp");
    mosaic.save_with_format(&tmp, image::ImageFormat::Png)?;
    std::fs::rename(tmp, cache_dir.join("overview.png"))?;
    Ok(())
}
