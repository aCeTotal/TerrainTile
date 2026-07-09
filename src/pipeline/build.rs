use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result};

use crate::edit::cover::Cover;
use crate::export::{classbin, maskpng, meshbin};
use crate::gen::heightfield::HeightSource;
use crate::pipeline::config::PipelineConfig;
use crate::pipeline::hash::BuildHashes;
use crate::tile::classes;
use crate::tile::grid::{TileGrid, TileId};
use crate::tile::mesh::{self, HeightPatch};
use crate::tile::meta::{Bbox, LodEntry, Neighbors, TileMeta};

pub fn read_meta(tiles_dir: &Path, t: TileId) -> Option<TileMeta> {
    let bytes = std::fs::read(tiles_dir.join(t.name()).join("metadata.json")).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// The tile's expected fingerprints: the world-wide config hashes plus the
/// tile's edit overlays (own + neighbor deltas, per-class coverage).
/// Heights feed the class gates too, so the delta fingerprint is part of
/// both.
pub fn tile_hashes(
    hashes: &BuildHashes,
    src: &HeightSource,
    cover: &Cover,
    cfg: &PipelineConfig,
    t: TileId,
) -> (String, String) {
    let d = src.edits().height_fingerprint(t);
    let mesh = if d != 0 { format!("{}#d{d:016x}", hashes.mesh) } else { hashes.mesh.clone() };
    let mut cov = String::new();
    for c in &cfg.classes {
        let fp = cover.fingerprint(c.id, t);
        if fp != 0 {
            cov.push_str(&format!("c{}:{fp:016x};", c.id));
        }
    }
    let masks = if d != 0 || !cov.is_empty() {
        format!("{}#d{d:016x}#{cov}", hashes.masks)
    } else {
        hashes.masks.clone()
    };
    (mesh, masks)
}

/// Build one tile: read the apron-padded height window from the source,
/// write LOD meshes and class textures, then metadata (atomic — the
/// resume marker). Meshes and class outputs have separate fingerprints,
/// so a class change regenerates class textures without touching meshes.
pub fn build_tile(
    cfg: &PipelineConfig,
    grid: &TileGrid,
    src: &HeightSource,
    cover: &Cover,
    tiles_dir: &Path,
    t: TileId,
    hashes: &BuildHashes,
) -> Result<()> {
    let old = if cfg.force { None } else { read_meta(tiles_dir, t) };
    let (mesh_hash, masks_hash) = tile_hashes(hashes, src, cover, cfg, t);
    let need_mesh = old.as_ref().is_none_or(|m| m.mesh_hash != mesh_hash);
    let need_masks = old.as_ref().is_none_or(|m| m.masks_hash != masks_hash);

    let n = grid.tile_px;
    // The compositor's blur needs at least PAD real neighbor samples.
    let apron = (1usize << (cfg.world.lods - 1)).max(classes::PAD);
    let size = n + 2 * apron + 1;
    let (opx, opy) = grid.origin_px(t);

    let dir = tiles_dir.join(t.name());
    std::fs::create_dir_all(&dir)?;

    // Pure open sea (including the apron) → minimal flat quad meshes, no
    // class files. `surely_sea` skips the noise sampling for the vast
    // majority of margin tiles; the rest are checked exactly. A sculpted
    // sea tile has a delta overlay and never counts as flat.
    let res = grid.resolution;
    let (x0, y0) = ((opx - apron as i64) as f64 * res, (opy - apron as i64) as f64 * res);
    let (x1, y1) = (x0 + (size - 1) as f64 * res, y0 + (size - 1) as f64 * res);
    let patch = if src.surely_sea(x0, y0, x1, y1) {
        None
    } else {
        let data = src.read(opx - apron as i64, opy - apron as i64, size, size);
        if data.iter().all(|h| *h == 0.0) {
            None
        } else {
            Some(HeightPatch { data, n, apron })
        }
    };
    let Some(patch) = patch else {
        return write_flat_tile(cfg, grid, tiles_dir, t, mesh_hash, masks_hash, need_mesh);
    };
    let stats = mesh::stats(&patch, grid.resolution);

    let lods = if need_mesh {
        let mut lods = Vec::with_capacity(cfg.world.lods);
        for k in 0..cfg.world.lods {
            let geo = mesh::LodGeometry {
                patch: &patch,
                stride: 1 << k,
                res: grid.resolution,
                overlap: true,
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

    let bbox = grid.bbox(t);
    let mask_files = if need_masks {
        let c = classes::composite(&patch, grid.resolution, &cfg.classes, cover, opx, opy);
        classbin::write(&dir.join("class.bin"), &c)?;
        // Per-class grayscale layers for external engines (Bevy).
        let mut mask_files = BTreeMap::new();
        for id in c.present() {
            let file = format!("class_{id}.png");
            maskpng::save_gray(&dir.join(&file), c.size, &c.layer(id))?;
            mask_files.insert(id.to_string(), file);
        }
        mask_files
    } else {
        old.as_ref().unwrap().masks.clone()
    };

    let meta = TileMeta {
        id: t.name(),
        x: t.x,
        y: t.y,
        bbox: Bbox { west: bbox.0, south: bbox.1, east: bbox.2, north: bbox.3 },
        flat: false,
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
        mesh_hash,
        masks_hash,
    };
    crate::tile::meta::write_atomic(&dir.join("metadata.json"), &meta)
        .context("kunne ikke skrive metadata")?;
    Ok(())
}

/// Write a pure-sea tile: one flat quad per LOD, no class files.
fn write_flat_tile(
    cfg: &PipelineConfig,
    grid: &TileGrid,
    tiles_dir: &Path,
    t: TileId,
    mesh_hash: String,
    masks_hash: String,
    need_mesh: bool,
) -> Result<()> {
    let dir = tiles_dir.join(t.name());
    let mut lods = Vec::with_capacity(cfg.world.lods);
    for k in 0..cfg.world.lods {
        let file = format!("mesh_lod{k}.bin");
        if need_mesh {
            meshbin::write_flat(&dir.join(&file), grid.tile_size_m)?;
        }
        lods.push(LodEntry {
            level: k,
            step_m: grid.resolution * (1 << k) as f64,
            file,
            vertices: 4,
            triangles: 2,
        });
    }
    let bbox = grid.bbox(t);
    let meta = TileMeta {
        id: t.name(),
        x: t.x,
        y: t.y,
        bbox: Bbox { west: bbox.0, south: bbox.1, east: bbox.2, north: bbox.3 },
        flat: true,
        min_height: 0.0,
        max_height: 0.0,
        average_slope_deg: 0.0,
        average_normal: [0.0, 1.0, 0.0],
        center: [(bbox.0 + bbox.2) / 2.0, (bbox.1 + bbox.3) / 2.0, 0.0],
        neighbors: Neighbors {
            north: grid.neighbor(t, 0, -1).map(|n| n.name()),
            south: grid.neighbor(t, 0, 1).map(|n| n.name()),
            east: grid.neighbor(t, 1, 0).map(|n| n.name()),
            west: grid.neighbor(t, -1, 0).map(|n| n.name()),
        },
        lods,
        masks: BTreeMap::new(),
        mesh_hash,
        masks_hash,
    };
    crate::tile::meta::write_atomic(&dir.join("metadata.json"), &meta)
        .context("kunne ikke skrive metadata")
}
