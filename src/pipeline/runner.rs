use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use crossbeam_channel::Sender;
use rayon::prelude::*;
use serde::Serialize;

use crate::edit::cover::Cover;
use crate::edit::store::EditStore;
use crate::export::quadtree;
use crate::gen::heightfield::HeightSource;
use crate::pipeline::build::{build_tile, read_meta, tile_hashes};
use crate::pipeline::config::PipelineConfig;
use crate::pipeline::progress::Progress;
use crate::tile::grid::{TileGrid, TileId};
use crate::tile::meta::TileMeta;
use crate::validate::check;

/// Generates the whole world. Panics in the pipeline thread are caught and
/// reported to the GUI instead of dying silently.
pub fn run(cfg: PipelineConfig, tx: Sender<Progress>, cancel: Arc<AtomicBool>) {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run_inner(&cfg, &tx, &cancel)
    }));
    match result {
        Ok(Ok(())) => {}
        Ok(Err(e)) => {
            let _ = tx.send(Progress::Error(format!("{e:#}")));
        }
        Err(panic) => {
            let msg = panic
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| panic.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "ukjent panikk".into());
            let _ = tx.send(Progress::Error(format!("intern feil: {msg}")));
        }
    }
}

fn run_inner(cfg: &PipelineConfig, tx: &Sender<Progress>, cancel: &AtomicBool) -> Result<()> {
    let started = Instant::now();
    let tiles_dir = cfg.output.join("tiles");
    std::fs::create_dir_all(&tiles_dir)?;

    cfg.world.validate()?;
    let grid = cfg.world.grid()?;
    let all = grid.tiles();
    let total = all.len();

    // Incremental: a tile is rebuilt only if the fingerprints of what its
    // outputs were built from no longer match the current configuration
    // (world params + the tile's edit overlays).
    let hashes = crate::pipeline::hash::compute(cfg);
    let src = HeightSource::new(cfg.world, Arc::new(EditStore::open(&cfg.output, &grid)));
    let cover = Cover::open(&cfg.output, &grid);
    let pending: Vec<TileId> = all
        .iter()
        .copied()
        .filter(|t| {
            cfg.force
                || read_meta(&tiles_dir, *t).is_none_or(|m| {
                    let (mesh, masks) = tile_hashes(&hashes, &src, &cover, cfg, *t);
                    m.mesh_hash != mesh || m.masks_hash != masks
                })
        })
        .collect();
    let skipped = total - pending.len();
    if skipped > 0 {
        let _ = tx.send(Progress::Stage(format!(
            "Inkrementelt: {skipped} fliser er uendret og hoppes over"
        )));
    }
    let _ = tx.send(Progress::Stage(format!(
        "Genererer {} fliser ({} x {} grid, {} px per flis)",
        pending.len(),
        grid.tiles_x,
        grid.tiles_y,
        grid.tile_px
    )));

    let done = AtomicUsize::new(skipped);
    let failed = AtomicUsize::new(0);

    let pool = rayon::ThreadPoolBuilder::new().num_threads(cfg.threads).build()?;
    pool.install(|| {
        pending.par_iter().for_each(|&t| {
            if cancel.load(Ordering::Relaxed) {
                return;
            }
            match build_tile(cfg, &grid, &src, &cover, &tiles_dir, t, &hashes) {
                Ok(()) => {
                    let d = done.fetch_add(1, Ordering::Relaxed) + 1;
                    let _ = tx.send(Progress::TileDone { done: d, total, name: t.name() });
                }
                Err(e) => {
                    let f = failed.fetch_add(1, Ordering::Relaxed) + 1;
                    let _ = tx.send(Progress::Warn(format!("{} feilet: {e:#}", t.name())));
                    // Mass failure (e.g. full disk): stop instead of burning
                    // hours — resume retries the rest.
                    if f == 100 {
                        let _ = tx.send(Progress::Warn(
                            "100 fliser har feilet — avbryter. Fiks årsaken og start igjen (resume).".into(),
                        ));
                        cancel.store(true, Ordering::Relaxed);
                    }
                }
            }
        });
    });

    if cancel.load(Ordering::Relaxed) {
        let _ = tx.send(Progress::Cancelled { done: done.load(Ordering::Relaxed), total });
        return Ok(());
    }

    let _ = tx.send(Progress::Stage("Skriver quadtree.json og dataset.json".into()));
    quadtree::write(&cfg.output.join("quadtree.json"), &quadtree::build(&grid, cfg.world.lods))?;
    write_dataset_json(cfg, &grid, &tiles_dir, &all)?;

    let _ = tx.send(Progress::Stage("Validerer datasettet".into()));
    let report = check::run(&cfg.output, &grid, cfg.world.lods, true);

    let _ = tx.send(Progress::Finished {
        tiles: total,
        skipped,
        failed: failed.load(Ordering::Relaxed),
        secs: started.elapsed().as_secs_f64(),
        report,
    });
    Ok(())
}

#[derive(Serialize)]
struct DatasetJson<'a> {
    crs: &'a str,
    resolution: f64,
    tile_size_m: f64,
    tile_px: usize,
    overlap: bool,
    lods: usize,
    tiles_x: usize,
    tiles_y: usize,
    /// World north-west corner (west, north): (0, size_m).
    origin: [f64; 2],
    extent: [f64; 4],
    min_height: f32,
    max_height: f32,
    /// Row-major bitset over the tile grid, hex encoded: 1 = pure-sea tile
    /// (minimal quad mesh, nothing to stream).
    flat_tiles: String,
    flat_count: usize,
    quadtree: &'a str,
    tiles_dir: &'a str,
}

fn write_dataset_json(
    cfg: &PipelineConfig,
    grid: &TileGrid,
    tiles_dir: &Path,
    all: &[TileId],
) -> Result<()> {
    let mut min_h = f32::MAX;
    let mut max_h = f32::MIN;
    let mut flat = vec![0u8; all.len().div_ceil(8)];
    let mut flat_count = 0usize;
    for (i, t) in all.iter().enumerate() {
        let path = tiles_dir.join(t.name()).join("metadata.json");
        if let Ok(bytes) = std::fs::read(&path) {
            if let Ok(m) = serde_json::from_slice::<TileMeta>(&bytes) {
                min_h = min_h.min(m.min_height);
                max_h = max_h.max(m.max_height);
                if m.flat {
                    flat[i / 8] |= 1 << (i % 8);
                    flat_count += 1;
                }
            }
        }
    }
    let s = cfg.world.size_m();
    let ds = DatasetJson {
        crs: "local",
        resolution: grid.resolution,
        tile_size_m: cfg.world.tile_size_m,
        tile_px: grid.tile_px,
        overlap: true,
        lods: cfg.world.lods,
        tiles_x: grid.tiles_x,
        tiles_y: grid.tiles_y,
        origin: [0.0, s],
        extent: [0.0, 0.0, s, s],
        min_height: min_h,
        max_height: max_h,
        flat_tiles: flat.iter().map(|b| format!("{b:02x}")).collect(),
        flat_count,
        quadtree: "quadtree.json",
        tiles_dir: "tiles",
    };
    std::fs::write(cfg.output.join("dataset.json"), serde_json::to_vec_pretty(&ds)?)?;
    Ok(())
}
