//! GET /data/overview.png — a world-wide color map baked from each tile's
//! strongest class (`class.bin` top-1 → the class's avg_color). The far
//! layer and the cheap shader sample it, so distant terrain follows the
//! painting. Cached; invalidated whenever dataset.json is rewritten.

use std::io::Read;
use std::path::Path;

use anyhow::{Context, Result};
use axum::extract::State;
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;

use crate::server::project;
use crate::server::state::SharedState;

const CACHE_NAME: &str = "overview_v1.png";

/// Total texture edge cap; texels per tile shrink for huge worlds.
const MAX_EDGE: u32 = 2048;

pub async fn overview(State(state): State<SharedState>) -> Result<Response, StatusCode> {
    let root = state.output().ok_or(StatusCode::NOT_FOUND)?;
    let _guard = state.far_lock.lock().await;
    let cache = root.join("cache").join(CACHE_NAME);
    if !crate::server::far::fresh(&cache, &root.join("dataset.json")) {
        let root2 = root.clone();
        tokio::task::spawn_blocking(move || build(&root2))
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
            .map_err(|e| {
                eprintln!("overview.png: {e:#}");
                StatusCode::INTERNAL_SERVER_ERROR
            })?;
    }
    let bytes = tokio::fs::read(&cache).await.map_err(|_| StatusCode::NOT_FOUND)?;
    Ok((
        [
            (header::CONTENT_TYPE, "image/png"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        bytes,
    )
        .into_response())
}

#[derive(Deserialize)]
struct Ds {
    tiles_x: usize,
    tiles_y: usize,
}

fn build(root: &Path) -> Result<()> {
    let ds: Ds = serde_json::from_slice(&std::fs::read(root.join("dataset.json"))?)
        .context("dataset.json")?;
    let p = project::load(root).context("project.json")?;
    let color_of = |id: u8| -> [u8; 3] {
        p.classes
            .iter()
            .find(|c| c.id == id as u32)
            .map(|c| hex_rgb(if c.avg_color.is_empty() { &c.color } else { &c.avg_color }))
            .unwrap_or([90, 100, 80])
    };
    let sea = p
        .classes
        .iter()
        .find(|c| c.water)
        .map(|c| hex_rgb(&c.color))
        .unwrap_or([26, 92, 115]);

    // Texels per tile edge within the total cap.
    let per = (MAX_EDGE as usize / ds.tiles_x.max(ds.tiles_y)).clamp(1, 16);
    let (w, h) = (ds.tiles_x * per, ds.tiles_y * per);
    let mut img = image::RgbImage::from_pixel(w as u32, h as u32, image::Rgb(sea));

    for ty in 0..ds.tiles_y {
        for tx in 0..ds.tiles_x {
            let path = root
                .join("tiles")
                .join(format!("tile_x{tx}_y{ty}"))
                .join("class.bin");
            let Ok(mut f) = std::fs::File::open(&path) else {
                continue; // flat sea tile → keep the sea color
            };
            let mut head = [0u8; 8];
            if f.read_exact(&mut head).is_err() || &head[0..4] != b"TTC1" {
                continue;
            }
            let size = u32::from_le_bytes(head[4..8].try_into().unwrap()) as usize;
            let mut idx = vec![0u8; size * size * 4];
            if f.read_exact(&mut idx).is_err() {
                continue;
            }
            for sy in 0..per {
                for sx in 0..per {
                    let gx = (sx * (size - 1)) / per.max(1);
                    let gy = (sy * (size - 1)) / per.max(1);
                    let top = idx[(gy * size + gx) * 4];
                    img.put_pixel(
                        (tx * per + sx) as u32,
                        (ty * per + sy) as u32,
                        image::Rgb(color_of(top)),
                    );
                }
            }
        }
    }

    let cache_dir = root.join("cache");
    std::fs::create_dir_all(&cache_dir)?;
    let tmp = cache_dir.join("overview.png.tmp");
    img.save_with_format(&tmp, image::ImageFormat::Png)?;
    std::fs::rename(tmp, cache_dir.join(CACHE_NAME))?;
    Ok(())
}

fn hex_rgb(s: &str) -> [u8; 3] {
    let s = s.trim_start_matches('#');
    if s.len() < 6 {
        return [90, 100, 80];
    }
    let p = |i: usize| u8::from_str_radix(&s[i..i + 2], 16).unwrap_or(90);
    [p(0), p(2), p(4)]
}
