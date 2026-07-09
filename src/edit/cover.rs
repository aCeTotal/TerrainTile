//! Painted class coverage: a global 8 m grid, stored as one 8-bit gray
//! PNG per terrain tile per class under `<output>/classes/<id>/`. Only
//! painted tiles have files. Samples have GLOBAL ownership (like the
//! sculpt deltas), so bilinear reads are identical from every tile —
//! the seam guarantee holds for painted materials too.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use anyhow::{Context, Result};

use crate::tile::grid::{TileGrid, TileId};

/// Meters per coverage sample.
pub const COVER_RES: f64 = 8.0;

/// Bound on cached grids; files are always on disk.
const MAX_CACHED: usize = 256;

type Key = (u32, usize, usize); // (class, tx, ty)

pub struct Cover {
    dir: PathBuf,
    /// Coverage samples per tile edge.
    n: usize,
    tiles_x: usize,
    tiles_y: usize,
    tile_size_m: f64,
    grids: RwLock<HashMap<Key, Arc<Vec<u8>>>>,
    fps: RwLock<HashMap<Key, u64>>,
}

impl Cover {
    pub fn open(output: &Path, grid: &TileGrid) -> Self {
        Self {
            dir: output.join("classes"),
            n: (grid.tile_size_m / COVER_RES).round() as usize,
            tiles_x: grid.tiles_x,
            tiles_y: grid.tiles_y,
            tile_size_m: grid.tile_size_m,
            grids: RwLock::new(HashMap::new()),
            fps: RwLock::new(HashMap::new()),
        }
    }

    /// Bilinear coverage 0..1 at a world position in meters.
    pub fn sample(&self, class: u32, x_m: f64, y_m: f64) -> f32 {
        let cx = x_m / COVER_RES - 0.5;
        let cy = y_m / COVER_RES - 0.5;
        let x0 = cx.floor();
        let y0 = cy.floor();
        let fx = (cx - x0) as f32;
        let fy = (cy - y0) as f32;
        let (x0, y0) = (x0 as i64, y0 as i64);
        let v = |gx: i64, gy: i64| self.at(class, gx, gy);
        let a = v(x0, y0) + (v(x0 + 1, y0) - v(x0, y0)) * fx;
        let b = v(x0, y0 + 1) + (v(x0 + 1, y0 + 1) - v(x0, y0 + 1)) * fx;
        a + (b - a) * fy
    }

    /// Owned sample at global coverage coordinates; 0 outside/unpainted.
    fn at(&self, class: u32, gx: i64, gy: i64) -> f32 {
        let n = self.n as i64;
        let (tx, ty) = (gx.div_euclid(n), gy.div_euclid(n));
        if tx < 0 || ty < 0 || tx >= self.tiles_x as i64 || ty >= self.tiles_y as i64 {
            return 0.0;
        }
        match self.grid((class, tx as usize, ty as usize)) {
            None => 0.0,
            Some(g) => {
                g[(gy.rem_euclid(n) * n + gx.rem_euclid(n)) as usize] as f32 / 255.0
            }
        }
    }

    fn path(&self, key: Key) -> PathBuf {
        self.dir.join(key.0.to_string()).join(format!("tile_x{}_y{}.png", key.1, key.2))
    }

    fn grid(&self, key: Key) -> Option<Arc<Vec<u8>>> {
        if let Some(g) = self.grids.read().unwrap().get(&key) {
            return Some(g.clone());
        }
        // Negative results are cached as fingerprint 0 to avoid re-statting.
        if self.fps.read().unwrap().get(&key) == Some(&0) {
            return None;
        }
        let img = match image::open(self.path(key)) {
            Ok(i) => i.to_luma8(),
            Err(_) => {
                self.fps.write().unwrap().insert(key, 0);
                return None;
            }
        };
        let data = img.into_raw();
        if data.len() != self.n * self.n {
            self.fps.write().unwrap().insert(key, 0);
            return None;
        }
        self.fps.write().unwrap().insert(key, fnv(&data));
        let arc = Arc::new(data);
        self.cache(key, arc.clone());
        Some(arc)
    }

    /// Rasterize a closed polygon (world meters) into a class's coverage.
    /// Returns the terrain tiles whose outputs are now stale.
    pub fn fill_polygon(
        &self,
        class: u32,
        poly: &[[f64; 2]],
        erase: bool,
    ) -> Result<BTreeSet<TileId>> {
        let mut dirty = BTreeSet::new();
        if poly.len() < 3 {
            return Ok(dirty);
        }
        let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for p in poly {
            x0 = x0.min(p[0]);
            y0 = y0.min(p[1]);
            x1 = x1.max(p[0]);
            y1 = y1.max(p[1]);
        }
        let n = self.n as i64;
        let gx0 = ((x0 / COVER_RES).floor() as i64).max(0);
        let gx1 = ((x1 / COVER_RES).ceil() as i64).min(self.tiles_x as i64 * n - 1);
        let gy0 = ((y0 / COVER_RES).floor() as i64).max(0);
        let gy1 = ((y1 / COVER_RES).ceil() as i64).min(self.tiles_y as i64 * n - 1);

        for ty in gy0.div_euclid(n)..=gy1.div_euclid(n) {
            for tx in gx0.div_euclid(n)..=gx1.div_euclid(n) {
                let key = (class, tx as usize, ty as usize);
                let mut data = match self.grid(key) {
                    Some(g) => g.as_ref().clone(),
                    None => {
                        if erase {
                            continue; // nothing to erase here
                        }
                        vec![0u8; self.n * self.n]
                    }
                };
                let mut touched = false;
                for gy in gy0.max(ty * n)..=gy1.min(ty * n + n - 1) {
                    for gx in gx0.max(tx * n)..=gx1.min(tx * n + n - 1) {
                        let x = (gx as f64 + 0.5) * COVER_RES;
                        let y = (gy as f64 + 0.5) * COVER_RES;
                        if inside(poly, x, y) {
                            let i = ((gy - ty * n) * n + (gx - tx * n)) as usize;
                            data[i] = if erase { 0 } else { 255 };
                            touched = true;
                        }
                    }
                }
                if !touched {
                    continue;
                }
                self.save(key, data)?;
                // Terrain tiles sharing this coverage tile go stale, plus
                // neighbors (bilinear + compositor blur read across edges).
                for dy in -1i64..=1 {
                    for dx in -1i64..=1 {
                        let (ax, ay) = (tx + dx, ty + dy);
                        if ax >= 0
                            && ay >= 0
                            && ax < self.tiles_x as i64
                            && ay < self.tiles_y as i64
                        {
                            dirty.insert(TileId { x: ax as usize, y: ay as usize });
                        }
                    }
                }
            }
        }
        Ok(dirty)
    }

    fn save(&self, key: Key, data: Vec<u8>) -> Result<()> {
        let path = self.path(key);
        std::fs::create_dir_all(path.parent().unwrap())?;
        let img = image::GrayImage::from_raw(self.n as u32, self.n as u32, data.clone())
            .context("dekningsstørrelse")?;
        img.save(&path).with_context(|| format!("kan ikke skrive {}", path.display()))?;
        self.fps.write().unwrap().insert(key, fnv(&data));
        self.cache(key, Arc::new(data));
        Ok(())
    }

    /// Combined fingerprint of a terrain tile's coverage inputs for a
    /// class: own file + the 8 neighbors (blur/bilinear read across).
    /// 0 = untouched everywhere.
    pub fn fingerprint(&self, class: u32, t: TileId) -> u64 {
        let mut h = 0xcbf29ce484222325u64;
        let mut any = false;
        for dy in -1i64..=1 {
            for dx in -1i64..=1 {
                let (tx, ty) = (t.x as i64 + dx, t.y as i64 + dy);
                let fp = if tx < 0
                    || ty < 0
                    || tx >= self.tiles_x as i64
                    || ty >= self.tiles_y as i64
                {
                    0
                } else {
                    self.fp((class, tx as usize, ty as usize))
                };
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

    fn fp(&self, key: Key) -> u64 {
        if let Some(fp) = self.fps.read().unwrap().get(&key) {
            return *fp;
        }
        // Loading computes and caches the fingerprint (or 0 if absent).
        let _ = self.grid(key);
        self.fps.read().unwrap().get(&key).copied().unwrap_or(0)
    }

    /// Coverage samples per tile edge (for clients drawing the overlay).
    pub fn samples_per_tile(&self) -> usize {
        self.n
    }

    pub fn tile_size_m(&self) -> f64 {
        self.tile_size_m
    }

    fn cache(&self, key: Key, value: Arc<Vec<u8>>) {
        let mut m = self.grids.write().unwrap();
        if m.len() >= MAX_CACHED {
            if let Some(k) = m.keys().find(|k| **k != key).copied() {
                m.remove(&k);
            }
        }
        m.insert(key, value);
    }
}

/// Even-odd point-in-polygon.
fn inside(poly: &[[f64; 2]], x: f64, y: f64) -> bool {
    let mut hit = false;
    let mut j = poly.len() - 1;
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[j]);
        if (a[1] > y) != (b[1] > y) && x < (b[0] - a[0]) * (y - a[1]) / (b[1] - a[1]) + a[0] {
            hit = !hit;
        }
        j = i;
    }
    hit
}

fn fnv(bytes: &[u8]) -> u64 {
    let mut h = 0xcbf29ce484222325u64;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h.max(1)
}
