use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result};
use crossbeam_channel::Sender;
use rayon::prelude::*;
use serde::Serialize;

use crate::export::{maskpng, meshbin, quadtree};
use crate::import::dataset::DatasetInfo;
use crate::import::reader::HeightReader;
use crate::import::vrt;
use crate::ortho::fetch::Fetcher;
use crate::ortho::sample;
use crate::pipeline::config::PipelineConfig;
use crate::pipeline::progress::Progress;
use crate::tile::grid::{TileGrid, TileId};
use crate::tile::masks::{self, MASK_NAMES};
use crate::tile::mesh::{self, HeightPatch};
use crate::tile::meta::{Bbox, LodEntry, Neighbors, TileMeta};
use crate::validate::check;

/// Runs the whole pipeline from raw user input (folders, files or zips).
/// Panics in the pipeline thread are caught and reported to the GUI instead
/// of dying silently.
pub fn run(
    cfg: PipelineConfig,
    inputs: Vec<std::path::PathBuf>,
    tx: Sender<Progress>,
    cancel: Arc<AtomicBool>,
) {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run_inner(&cfg, &inputs, &tx, &cancel)
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

fn run_inner(
    cfg: &PipelineConfig,
    inputs: &[std::path::PathBuf],
    tx: &Sender<Progress>,
    cancel: &AtomicBool,
) -> Result<()> {
    let started = Instant::now();
    let tiles_dir = cfg.output.join("tiles");
    std::fs::create_dir_all(&tiles_dir)?;

    // Zips are extracted BEFORE any GDAL access: GeoTIFF metadata often
    // sits at the end of the file, and reading it through a compressed
    // zip stream means decompressing the entire file — per file.
    let source_files = match resolve_inputs(cfg, inputs, tx, cancel) {
        Ok(f) => f,
        Err(e) if cancel.load(Ordering::Relaxed) => {
            let _ = tx.send(Progress::Cancelled { done: 0, total: 0 });
            let _ = e;
            return Ok(());
        }
        Err(e) => return Err(e),
    };

    let _ = tx.send(Progress::Stage("Fyller innelukkede datahull (nodata) i kildene".into()));
    let mosaic_files = crate::import::fill::fill_all(&source_files, &cfg.output, |m| {
        let _ = tx.send(Progress::Stage(m));
    })?;

    let _ = tx.send(Progress::Stage("Skanner høydedata (CRS, oppløsning, grid)".into()));
    let info = &crate::import::dataset::scan(&mosaic_files)?;

    let _ = tx.send(Progress::Stage("Bygger mosaikk (VRT)".into()));
    let vrt_path = cfg.output.join("mosaic.vrt");
    vrt::build_mosaic(&info.files, &vrt_path)?;

    let grid = TileGrid::new(info, cfg.tile_size_m, cfg.lods)?;
    let all = grid.tiles();
    let total = all.len();

    // Incremental: a tile is rebuilt only if the fingerprints of what its
    // outputs were built from no longer match the current configuration.
    let hashes = crate::pipeline::hash::compute(cfg, info, &source_files);
    let pending: Vec<TileId> = all
        .iter()
        .copied()
        .filter(|t| cfg.force || read_meta(&tiles_dir, *t).map_or(true, |m| {
            m.mesh_hash != hashes.mesh || m.masks_hash != hashes.masks || stale_fill(&m)
        }))
        .collect();
    let skipped = total - pending.len();
    if skipped > 0 {
        let _ = tx.send(Progress::Stage(format!(
            "Inkrementelt: {skipped} fliser er uendret og hoppes over"
        )));
    }
    let _ = tx.send(Progress::Stage(format!(
        "Prosesserer {} fliser ({} x {} grid, {} px per flis)",
        pending.len(),
        grid.tiles_x,
        grid.tiles_y,
        grid.tile_px
    )));

    let fetcher = cfg.ortho.clone().map(|s| Arc::new(Fetcher::new(s)));

    // Preflight: verify the orthophoto source on ONE small request before
    // touching thousands of tiles — an invalid ticket/URL must stop the run
    // with one clear error, not 18000 warnings.
    if let (Some(f), Some(t)) = (&fetcher, pending.first()) {
        let _ = tx.send(Progress::Stage("Tester ortofoto-kilden".into()));
        let provider = &cfg.ortho.as_ref().unwrap().provider;
        sample::sample_tile(f, provider, &info.crs, &info.crs_wkt, grid.bbox(*t), 16, "preflight")
            .map_err(|e| {
                anyhow::anyhow!(
                    "ortofoto-kilden feiler: {e:#}\n\
                     Rett WMS-URL/ticket (eller bytt til XYZ / slå av ortofoto) og start igjen."
                )
            })?;
    }

    let done = AtomicUsize::new(skipped);
    let failed = AtomicUsize::new(0);

    let pool = rayon::ThreadPoolBuilder::new().num_threads(cfg.threads).build()?;
    pool.install(|| {
        pending.par_iter().for_each(|&t| {
            if cancel.load(Ordering::Relaxed) {
                return;
            }
            match process_tile(cfg, info, &grid, &vrt_path, fetcher.as_deref(), &tiles_dir, t, &hashes, tx) {
                Ok(()) => {
                    let d = done.fetch_add(1, Ordering::Relaxed) + 1;
                    let _ = tx.send(Progress::TileDone { done: d, total, name: t.name() });
                }
                Err(e) => {
                    let f = failed.fetch_add(1, Ordering::Relaxed) + 1;
                    let _ = tx.send(Progress::Warn(format!("{} feilet: {e:#}", t.name())));
                    // Mass failure (e.g. ticket expired mid-run): stop
                    // instead of burning hours — resume retries the rest.
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
    quadtree::write(&cfg.output.join("quadtree.json"), &quadtree::build(&grid, cfg.lods))?;
    write_dataset_json(cfg, info, &grid, &tiles_dir, &all)?;

    let _ = tx.send(Progress::Stage("Validerer datasettet".into()));
    let report = check::run(&cfg.output, &grid, cfg.lods, cfg.overlap);

    let _ = tx.send(Progress::Finished {
        tiles: total,
        skipped,
        failed: failed.load(Ordering::Relaxed),
        secs: started.elapsed().as_secs_f64(),
        report,
    });
    Ok(())
}

/// Expand user input to plain raster files. Zip archives are extracted to
/// `out/source/`, streaming file by file (bounded RAM), in parallel, with
/// resume (existing file with right size is skipped).
fn resolve_inputs(
    cfg: &PipelineConfig,
    inputs: &[std::path::PathBuf],
    tx: &Sender<Progress>,
    cancel: &AtomicBool,
) -> Result<Vec<std::path::PathBuf>> {
    use crate::import::zips;
    let is_zip = |p: &std::path::Path| {
        p.extension().and_then(|e| e.to_str()).map(str::to_lowercase).as_deref() == Some("zip")
    };
    // (zip, inner) jobs + passthrough paths (files and folders).
    let mut jobs: Vec<(std::path::PathBuf, String)> = Vec::new();
    let mut plain: Vec<std::path::PathBuf> = Vec::new();
    for p in inputs {
        if p.is_file() && is_zip(p) {
            for inner in zips::list_rasters(p)? {
                jobs.push((p.clone(), inner));
            }
        } else {
            plain.push(p.clone());
        }
    }
    if jobs.is_empty() {
        return Ok(plain);
    }

    let dest = cfg.output.join("source");
    std::fs::create_dir_all(&dest)?;
    let total = jobs.len();
    let _ = tx.send(Progress::Stage(format!("Pakker ut {total} filer fra ZIP")));
    let done = AtomicUsize::new(0);
    let mut extracted: Vec<std::path::PathBuf> = jobs
        .par_iter()
        .map(|(zip, inner)| {
            if cancel.load(Ordering::Relaxed) {
                anyhow::bail!("avbrutt");
            }
            let out = zips::extract(zip, inner, &dest)?;
            let d = done.fetch_add(1, Ordering::Relaxed) + 1;
            let _ = tx.send(Progress::Stage(format!(
                "Pakker ut {d}/{total}: {}",
                inner.rsplit('/').next().unwrap()
            )));
            Ok(out)
        })
        .collect::<Result<_>>()?;
    extracted.extend(plain);
    Ok(extracted)
}

fn read_meta(tiles_dir: &Path, t: TileId) -> Option<TileMeta> {
    let bytes = std::fs::read(tiles_dir.join(t.name()).join("metadata.json")).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Built with nodata before the current fill algorithm — its heights may
/// contain crater artifacts and must be regenerated.
fn stale_fill(m: &TileMeta) -> bool {
    m.had_nodata && m.fill_version < crate::import::fill::FILL_VERSION
}

#[allow(clippy::too_many_arguments)]
fn process_tile(
    cfg: &PipelineConfig,
    info: &DatasetInfo,
    grid: &TileGrid,
    vrt_path: &Path,
    fetcher: Option<&Fetcher>,
    tiles_dir: &Path,
    t: TileId,
    hashes: &crate::pipeline::hash::BuildHashes,
    tx: &Sender<Progress>,
) -> Result<()> {
    thread_local! {
        static READER: RefCell<Option<HeightReader>> = const { RefCell::new(None) };
    }

    // Partial rebuild: meshes and masks have separate fingerprints, so a
    // changed mask threshold regenerates masks without touching meshes.
    let old = if cfg.force { None } else { read_meta(tiles_dir, t) };
    let refill = old.as_ref().is_some_and(stale_fill);
    let need_mesh = old.as_ref().is_none_or(|m| m.mesh_hash != hashes.mesh) || refill;
    let need_masks = old.as_ref().is_none_or(|m| m.masks_hash != hashes.masks) || refill;

    let n = grid.tile_px;
    let apron = 1usize << (cfg.lods - 1);
    let size = n + 2 * apron + 1;
    let (opx, opy) = grid.origin_px(t);

    let (data, had_nodata) = READER.with(|r| {
        let mut r = r.borrow_mut();
        if r.is_none() {
            *r = Some(HeightReader::open(vrt_path, cfg.nodata_height)?);
        }
        r.as_ref().unwrap().read(opx - apron as i64, opy - apron as i64, size, size)
    })?;
    if had_nodata {
        let _ = tx.send(Progress::Warn(format!(
            "{}: nodata (hav/kant) fylt med nodata-høyde",
            t.name()
        )));
    }
    let patch = HeightPatch { data, n, apron };
    let stats = mesh::stats(&patch, grid.resolution);

    let dir = tiles_dir.join(t.name());
    std::fs::create_dir_all(&dir)?;

    let lods = if need_mesh {
        let mut lods = Vec::with_capacity(cfg.lods);
        for k in 0..cfg.lods {
            let geo = mesh::LodGeometry {
                patch: &patch,
                stride: 1 << k,
                res: grid.resolution,
                overlap: cfg.overlap,
            };
            let file = format!("mesh_lod{k}.bin");
            meshbin::write(&dir.join(&file), &geo)?;
            lods.push(LodEntry {
                level: k,
                step_m: grid.resolution * (1 << k) as f64,
                file,
                vertices: geo.vertex_count(),
                triangles: geo.index_count() / 3,
            });
        }
        lods
    } else {
        old.as_ref().unwrap().lods.clone()
    };

    // With orthophoto enabled, a fetch failure fails the whole tile: no
    // metadata is written, so resume retries it later (e.g. with a fresh
    // ticket) instead of silently baking DTM-only masks into the dataset.
    let bbox = grid.bbox(t);
    let (mask_files, textures) = if need_masks {
        let ortho_grid = match fetcher {
            Some(f) => {
                let provider = &cfg.ortho.as_ref().unwrap().provider;
                Some(
                    sample::sample_tile(f, provider, &info.crs, &info.crs_wkt, bbox, n + 1, &t.name())
                        .context("ortofoto")?,
                )
            }
            None => None,
        };

        let m = masks::classify(&patch, grid.resolution, ortho_grid.as_ref(), &cfg.masks);
        let mut mask_files = BTreeMap::new();
        for (name, layer) in MASK_NAMES.iter().zip(&m.layers) {
            let file = format!("mask_{name}.png");
            maskpng::save_gray(&dir.join(&file), m.size, layer)?;
            mask_files.insert((*name).to_string(), file);
        }
        for (name, layer) in [("veg_trees", &m.trees), ("veg_bushes", &m.bushes)] {
            if let Some(layer) = layer {
                let file = format!("mask_{name}.png");
                maskpng::save_gray(&dir.join(&file), m.size, layer)?;
                mask_files.insert(name.to_string(), file);
            }
        }
        let mut textures = BTreeMap::new();
        if let Some(g) = &ortho_grid {
            maskpng::save_rgb(&dir.join("ortho.png"), g.size, &g.data)?;
            textures.insert("ortho".to_string(), "ortho.png".to_string());
        }
        (mask_files, textures)
    } else {
        let old = old.as_ref().unwrap();
        (old.masks.clone(), old.textures.clone())
    };

    let meta = TileMeta {
        id: t.name(),
        x: t.x,
        y: t.y,
        bbox: Bbox { west: bbox.0, south: bbox.1, east: bbox.2, north: bbox.3 },
        min_height: stats.min_h,
        max_height: stats.max_h,
        average_slope_deg: stats.avg_slope_deg,
        average_normal: stats.avg_normal,
        center: [
            (bbox.0 + bbox.2) / 2.0,
            (bbox.1 + bbox.3) / 2.0,
            ((stats.min_h + stats.max_h) / 2.0) as f64,
        ],
        neighbors: Neighbors {
            north: grid.neighbor(t, 0, -1).map(|n| n.name()),
            south: grid.neighbor(t, 0, 1).map(|n| n.name()),
            east: grid.neighbor(t, 1, 0).map(|n| n.name()),
            west: grid.neighbor(t, -1, 0).map(|n| n.name()),
        },
        lods,
        masks: mask_files,
        textures,
        had_nodata,
        fill_version: crate::import::fill::FILL_VERSION,
        mesh_hash: hashes.mesh.clone(),
        masks_hash: hashes.masks.clone(),
    };
    crate::tile::meta::write_atomic(&dir.join("metadata.json"), &meta)
        .context("kunne ikke skrive metadata")?;
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
    /// Dataset north-west corner (west, north) in CRS units.
    origin: [f64; 2],
    extent: [f64; 4],
    min_height: f32,
    max_height: f32,
    masks: [&'static str; 8],
    quadtree: &'a str,
    tiles_dir: &'a str,
}

fn write_dataset_json(
    cfg: &PipelineConfig,
    info: &DatasetInfo,
    grid: &TileGrid,
    tiles_dir: &Path,
    all: &[TileId],
) -> Result<()> {
    let mut min_h = f32::MAX;
    let mut max_h = f32::MIN;
    for t in all {
        let path = tiles_dir.join(t.name()).join("metadata.json");
        if let Ok(bytes) = std::fs::read(&path) {
            if let Ok(m) = serde_json::from_slice::<TileMeta>(&bytes) {
                min_h = min_h.min(m.min_height);
                max_h = max_h.max(m.max_height);
            }
        }
    }
    let ds = DatasetJson {
        crs: &info.crs,
        resolution: info.resolution,
        tile_size_m: cfg.tile_size_m,
        tile_px: grid.tile_px,
        overlap: cfg.overlap,
        lods: cfg.lods,
        tiles_x: grid.tiles_x,
        tiles_y: grid.tiles_y,
        origin: [grid.origin.0, grid.origin.1],
        extent: [info.extent.0, info.extent.1, info.extent.2, info.extent.3],
        min_height: min_h,
        max_height: max_h,
        masks: MASK_NAMES,
        quadtree: "quadtree.json",
        tiles_dir: "tiles",
    };
    std::fs::write(cfg.output.join("dataset.json"), serde_json::to_vec_pretty(&ds)?)?;
    Ok(())
}
