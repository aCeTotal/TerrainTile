//! Background rebuild worker: collects dirty tiles from brush strokes,
//! waits for the stroke to settle, rebuilds the affected tiles via the
//! ordinary pipeline tile builder, then notifies SSE clients so the viewer
//! re-fetches them.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use crossbeam_channel::{Receiver, RecvTimeoutError};
use rayon::prelude::*;
use serde_json::json;

use crate::edit::cover::Cover;
use crate::gen::heightfield::HeightSource;
use crate::pipeline::build::build_tile;
use crate::pipeline::config::PipelineConfig;
use crate::server::state::SharedState;
use crate::tile::grid::{TileGrid, TileId};

const SETTLE: Duration = Duration::from_millis(500);

/// Spawn the worker thread. Send dirty tile sets on the returned channel.
pub fn spawn(
    state: SharedState,
    cfg: PipelineConfig,
    grid: TileGrid,
    src: Arc<HeightSource>,
    cover: Arc<Cover>,
) -> crossbeam_channel::Sender<BTreeSet<TileId>> {
    let (tx, rx) = crossbeam_channel::unbounded::<BTreeSet<TileId>>();
    std::thread::spawn(move || worker(state, cfg, grid, src, cover, rx));
    tx
}

fn worker(
    state: SharedState,
    cfg: PipelineConfig,
    grid: TileGrid,
    src: Arc<HeightSource>,
    cover: Arc<Cover>,
    rx: Receiver<BTreeSet<TileId>>,
) {
    let hashes = crate::pipeline::hash::compute(&cfg);
    let tiles_dir = cfg.output.join("tiles");
    let mut dirty: BTreeSet<TileId> = BTreeSet::new();

    loop {
        // Block for the first dirty set, then keep absorbing until the
        // stroke has settled for SETTLE.
        if dirty.is_empty() {
            match rx.recv() {
                Ok(set) => dirty.extend(set),
                Err(_) => return, // project closed
            }
        }
        loop {
            match rx.recv_timeout(SETTLE) {
                Ok(set) => dirty.extend(set),
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }

        let batch: Vec<TileId> = std::mem::take(&mut dirty).into_iter().collect();
        let failed: Vec<String> = batch
            .par_iter()
            .filter_map(|&t| {
                build_tile(&cfg, &grid, &src, &cover, &tiles_dir, t, &hashes)
                    .err()
                    .map(|e| format!("{}: {e:#}", t.name()))
            })
            .collect();
        for f in &failed {
            eprintln!("rebuild: {f}");
        }

        // Touch dataset.json so the far.bin cache rebuilds lazily.
        let ds = cfg.output.join("dataset.json");
        if let Ok(bytes) = std::fs::read(&ds) {
            let _ = std::fs::write(&ds, bytes);
        }

        let names: Vec<String> = batch.iter().map(|t| t.name()).collect();
        let _ = state.events.send(json!({ "type": "tiles", "tiles": names }).to_string());

        // Terrain changed under scatter areas → their instances re-snap.
        if let Some(p) = crate::server::project::load(&cfg.output) {
            let hit = p
                .scatter
                .iter()
                .any(|a| crate::edit::scatter::touches(a, &batch, grid.tile_size_m));
            if hit && crate::edit::scatter::write_all(&cfg.output, &p.scatter, &src).is_ok() {
                let _ = state.events.send(json!({ "type": "scatter" }).to_string());
            }
        }
    }
}
