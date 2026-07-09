//! Persistent edit overlays. Heights: dense per-tile f32 delta grids with
//! GLOBAL sample ownership (`tx = px / tile_px`), so every reader sees the
//! same value for a given global coordinate — the seam guarantee survives
//! editing. Paint: per-vertex (layer, strength) grids, written identically
//! into every tile that shares an edge vertex.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use anyhow::{Context, Result};

use crate::tile::grid::{TileGrid, TileId};

/// Overlay files live next to the tile's meshes.
const DELTA_FILE: &str = "delta_h.bin";

/// Bound on cached overlay grids; edited files are always on disk.
const MAX_CACHED: usize = 256;

type Key = (usize, usize);

pub struct EditStore {
    tiles_dir: PathBuf,
    /// Height samples per tile edge (delta grid is n×n).
    n: usize,
    tiles_x: usize,
    tiles_y: usize,
    deltas: RwLock<HashMap<Key, Arc<Vec<f32>>>>,
    /// FNV fingerprints of the current overlays (0 = no overlay).
    delta_fps: RwLock<HashMap<Key, u64>>,
}

impl EditStore {
    pub fn open(output: &Path, grid: &TileGrid) -> Self {
        Self {
            tiles_dir: output.join("tiles"),
            n: grid.tile_px,
            tiles_x: grid.tiles_x,
            tiles_y: grid.tiles_y,
            deltas: RwLock::new(HashMap::new()),
            delta_fps: RwLock::new(HashMap::new()),
        }
    }

    pub fn tile_px(&self) -> usize {
        self.n
    }

    fn in_grid(&self, tx: i64, ty: i64) -> Option<Key> {
        if tx < 0 || ty < 0 || tx >= self.tiles_x as i64 || ty >= self.tiles_y as i64 {
            None
        } else {
            Some((tx as usize, ty as usize))
        }
    }

    /// Height delta at a global sample coordinate; 0 outside the world or
    /// where nothing was sculpted.
    pub fn delta_at(&self, px: i64, py: i64) -> f32 {
        let n = self.n as i64;
        let Some(key) = self.in_grid(px.div_euclid(n), py.div_euclid(n)) else {
            return 0.0;
        };
        match self.delta(key) {
            None => 0.0,
            Some(d) => {
                let (lx, ly) = (px.rem_euclid(n) as usize, py.rem_euclid(n) as usize);
                d[ly * self.n + lx]
            }
        }
    }

    /// Cached delta grid for a tile, loaded from disk on first use.
    pub fn delta(&self, key: Key) -> Option<Arc<Vec<f32>>> {
        if let Some(d) = self.deltas.read().unwrap().get(&key) {
            return Some(d.clone());
        }
        let path = self.overlay_path(key, DELTA_FILE);
        let bytes = std::fs::read(&path).ok()?;
        let data: Vec<f32> = bytes
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes(c.try_into().unwrap()))
            .collect();
        let arc = Arc::new(data);
        self.cache(&self.deltas, key, arc.clone());
        self.delta_fps.write().unwrap().insert(key, fnv_bytes(&bytes));
        Some(arc)
    }

    /// Clone-out / mutate / swap-in / persist. The lock is never held while
    /// `f` runs, so `f` may read other tiles (e.g. smooth-brush neighbors);
    /// it then reads pre-stroke values, which is what a brush wants.
    pub fn modify_delta(&self, key: Key, f: impl FnOnce(&mut [f32])) -> Result<()> {
        let mut data = match self.delta(key) {
            Some(d) => d.as_ref().clone(),
            None => vec![0.0; self.n * self.n],
        };
        f(&mut data);
        let mut bytes = Vec::with_capacity(data.len() * 4);
        for v in &data {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        self.persist(key, DELTA_FILE, &bytes)?;
        self.delta_fps.write().unwrap().insert(key, fnv_bytes(&bytes));
        self.cache(&self.deltas, key, Arc::new(data));
        Ok(())
    }

    /// Fingerprint of a tile's height inputs beyond the generator: its own
    /// delta AND the neighbors' (the apron reads their samples — an edit
    /// just across the seam changes this tile's normals).
    pub fn height_fingerprint(&self, t: TileId) -> u64 {
        let mut h = 0xcbf29ce484222325u64;
        let mut any = false;
        for dy in -1i64..=1 {
            for dx in -1i64..=1 {
                let fp = self
                    .in_grid(t.x as i64 + dx, t.y as i64 + dy)
                    .map_or(0, |k| self.overlay_fp(&self.delta_fps, k, DELTA_FILE));
                if fp != 0 {
                    any = true;
                }
                for b in fp.to_le_bytes() {
                    h ^= b as u64;
                    h = h.wrapping_mul(0x100000001b3);
                }
            }
        }
        if any { h } else { 0 }
    }

    /// True if any height delta exists in the sample-space range — used to
    /// keep sculpted sea tiles from being written as flat.
    pub fn any_delta_in(&self, px0: i64, py0: i64, px1: i64, py1: i64) -> bool {
        let n = self.n as i64;
        for ty in px_range(py0, py1, n, self.tiles_y) {
            for tx in px_range(px0, px1, n, self.tiles_x) {
                if self.overlay_fp(&self.delta_fps, (tx, ty), DELTA_FILE) != 0 {
                    return true;
                }
            }
        }
        false
    }

    fn overlay_fp(&self, fps: &RwLock<HashMap<Key, u64>>, key: Key, file: &str) -> u64 {
        if let Some(fp) = fps.read().unwrap().get(&key) {
            return *fp;
        }
        let fp = std::fs::read(self.overlay_path(key, file)).map_or(0, |b| fnv_bytes(&b));
        fps.write().unwrap().insert(key, fp);
        fp
    }

    fn overlay_path(&self, key: Key, file: &str) -> PathBuf {
        self.tiles_dir.join(TileId { x: key.0, y: key.1 }.name()).join(file)
    }

    fn persist(&self, key: Key, file: &str, bytes: &[u8]) -> Result<()> {
        let dir = self.tiles_dir.join(TileId { x: key.0, y: key.1 }.name());
        std::fs::create_dir_all(&dir)?;
        let path = dir.join(file);
        let tmp = path.with_extension("bin.tmp");
        std::fs::write(&tmp, bytes).with_context(|| format!("skrive {}", tmp.display()))?;
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }

    fn cache<T>(&self, map: &RwLock<HashMap<Key, Arc<T>>>, key: Key, value: Arc<T>) {
        let mut m = map.write().unwrap();
        if m.len() >= MAX_CACHED {
            if let Some(k) = m.keys().find(|k| **k != key).copied() {
                m.remove(&k);
            }
        }
        m.insert(key, value);
    }
}

/// Tile index range covering a global sample range, clamped to the grid.
fn px_range(p0: i64, p1: i64, n: i64, tiles: usize) -> std::ops::RangeInclusive<usize> {
    let a = p0.div_euclid(n).clamp(0, tiles as i64 - 1) as usize;
    let b = p1.div_euclid(n).clamp(0, tiles as i64 - 1) as usize;
    a..=b
}

fn fnv_bytes(bytes: &[u8]) -> u64 {
    let mut h = 0xcbf29ce484222325u64;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    // 0 is reserved for "no overlay".
    h.max(1)
}
