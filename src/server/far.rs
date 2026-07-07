use std::io::Read;
use std::path::Path;

use anyhow::{bail, Context, Result};
use axum::extract::State;
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;

use crate::server::state::SharedState;

/// The whole-terrain layer targets roughly this many vertices in total; the
/// per-tile grid is subsampled further until it fits, so even huge datasets
/// render as one cheap mesh.
const TARGET_VERTS: usize = 2_000_000;

/// GET /data/far.bin — every tile's coarsest mesh concatenated into one
/// compact stream so the viewer can show the entire terrain with a single
/// download and a single draw call. Little-endian:
///
/// ```text
/// magic  [u8;4] = "TTF1"
/// u32    tile_count
/// u32    verts_per_edge (v)     same for every tile
/// per tile: u32 x, u32 y, f32 pos[3*v*v], f32 nrm[3*v*v]
/// ```
///
/// Positions are tile-local (TTM convention); indices and UVs are regular
/// grids the client regenerates itself. Cached under `cache/far.bin`,
/// invalidated whenever `dataset.json` is rewritten (end of every run).
pub async fn far_bin(State(state): State<SharedState>) -> Result<Response, StatusCode> {
    let root = state.output().ok_or(StatusCode::NOT_FOUND)?;
    let _guard = state.far_lock.lock().await;
    let cache = root.join("cache").join("far.bin");
    if !fresh(&cache, &root.join("dataset.json")) {
        let root2 = root.clone();
        tokio::task::spawn_blocking(move || build(&root2))
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
            .map_err(|e| {
                eprintln!("far.bin: {e:#}");
                StatusCode::INTERNAL_SERVER_ERROR
            })?;
    }
    let bytes = tokio::fs::read(&cache).await.map_err(|_| StatusCode::NOT_FOUND)?;
    Ok((
        [
            (header::CONTENT_TYPE, "application/octet-stream"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        bytes,
    )
        .into_response())
}

/// True if `cache` exists and is newer than `stamp`.
pub fn fresh(cache: &Path, stamp: &Path) -> bool {
    match (cache.metadata(), stamp.metadata()) {
        (Ok(c), Ok(s)) => matches!(
            (c.modified(), s.modified()),
            (Ok(cm), Ok(sm)) if cm >= sm
        ),
        _ => false,
    }
}

#[derive(Deserialize)]
struct Ds {
    lods: usize,
    tile_px: usize,
    tiles_x: usize,
    tiles_y: usize,
}

fn build(root: &Path) -> Result<()> {
    let ds: Ds = serde_json::from_slice(&std::fs::read(root.join("dataset.json"))?)
        .context("dataset.json")?;
    let lod = ds.lods - 1;
    // Quads per tile edge at the coarsest LOD, then subsample by powers of
    // two until the whole dataset fits the vertex budget.
    let m = ds.tile_px >> lod;
    let count = ds.tiles_x * ds.tiles_y;
    let mut stride = 1usize;
    while count * (m / stride + 1).pow(2) > TARGET_VERTS && (m / stride).is_multiple_of(2) {
        stride *= 2;
    }
    let v = m / stride + 1; // verts per edge in the output
    let src_v = m + 1; // verts per edge in the mesh file

    let mut out: Vec<u8> = Vec::with_capacity(12 + count * (8 + v * v * 24));
    out.extend_from_slice(b"TTF1");
    out.extend_from_slice(&0u32.to_le_bytes()); // tile_count, patched below
    out.extend_from_slice(&(v as u32).to_le_bytes());

    let mut written = 0u32;
    let mut block = vec![0u8; src_v * src_v * 12];
    for y in 0..ds.tiles_y {
        for x in 0..ds.tiles_x {
            let path = root
                .join("tiles")
                .join(format!("tile_x{x}_y{y}"))
                .join(format!("mesh_lod{lod}.bin"));
            let Ok(mut file) = std::fs::File::open(&path) else {
                continue; // failed tile: leave a hole rather than failing all
            };
            let mut head = [0u8; 12];
            file.read_exact(&mut head)?;
            if head[0..4] != *b"TTM1" {
                bail!("{}: feil magic", path.display());
            }
            let vc = u32::from_le_bytes(head[4..8].try_into().unwrap()) as usize;
            if vc != src_v * src_v {
                bail!("{}: {vc} vertekser, forventet {}", path.display(), src_v * src_v);
            }
            out.extend_from_slice(&(x as u32).to_le_bytes());
            out.extend_from_slice(&(y as u32).to_le_bytes());
            // Positions, then normals — laid out identically in the file.
            for _ in 0..2 {
                file.read_exact(&mut block)?;
                for i in (0..src_v).step_by(stride) {
                    for j in (0..src_v).step_by(stride) {
                        let o = (i * src_v + j) * 12;
                        out.extend_from_slice(&block[o..o + 12]);
                    }
                }
            }
            written += 1;
        }
    }
    out[4..8].copy_from_slice(&written.to_le_bytes());

    let cache_dir = root.join("cache");
    std::fs::create_dir_all(&cache_dir)?;
    let tmp = cache_dir.join("far.bin.tmp");
    std::fs::write(&tmp, &out)?;
    std::fs::rename(tmp, cache_dir.join("far.bin"))?;
    Ok(())
}
