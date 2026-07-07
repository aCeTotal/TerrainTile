use anyhow::{bail, Result};

use crate::import::dataset::DatasetInfo;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct TileId {
    pub x: usize,
    pub y: usize,
}

impl TileId {
    pub fn name(&self) -> String {
        format!("tile_x{}_y{}", self.x, self.y)
    }
}

/// Uniform tile grid over the dataset. Tile (0,0) starts at the dataset's
/// north-west corner; x grows east, y grows south.
#[derive(Clone, Debug)]
pub struct TileGrid {
    pub tiles_x: usize,
    pub tiles_y: usize,
    /// Height samples per tile edge (vertices per edge is `tile_px + 1`
    /// with overlap enabled).
    pub tile_px: usize,
    pub tile_size_m: f64,
    pub resolution: f64,
    /// Dataset north-west corner (west, north) in CRS units.
    pub origin: (f64, f64),
}

impl TileGrid {
    pub fn new(info: &DatasetInfo, tile_size_m: f64, lods: usize) -> Result<Self> {
        let n_f = tile_size_m / info.resolution;
        let n = n_f.round() as usize;
        if n == 0 || (n_f - n as f64).abs() > 1e-9 {
            bail!(
                "flisstørrelse {tile_size_m} m er ikke et helt antall piksler ved {} m oppløsning",
                info.resolution
            );
        }
        let max_stride = 1usize << (lods - 1);
        if n % max_stride != 0 {
            bail!(
                "flisstørrelse {tile_size_m} m = {n} px må være delelig med {max_stride} (LOD{})",
                lods - 1
            );
        }
        let tiles_x = info.width_px / n;
        let tiles_y = info.height_px / n;
        if tiles_x == 0 || tiles_y == 0 {
            bail!(
                "datasettet ({} x {} px) er mindre enn én flis ({n} px)",
                info.width_px,
                info.height_px
            );
        }
        Ok(Self {
            tiles_x,
            tiles_y,
            tile_px: n,
            tile_size_m,
            resolution: info.resolution,
            origin: info.origin,
        })
    }

    pub fn count(&self) -> usize {
        self.tiles_x * self.tiles_y
    }

    pub fn tiles(&self) -> Vec<TileId> {
        let mut v = Vec::with_capacity(self.count());
        for y in 0..self.tiles_y {
            for x in 0..self.tiles_x {
                v.push(TileId { x, y });
            }
        }
        v
    }

    /// Pixel coordinate of the tile's north-west corner in the mosaic.
    pub fn origin_px(&self, t: TileId) -> (i64, i64) {
        ((t.x * self.tile_px) as i64, (t.y * self.tile_px) as i64)
    }

    /// (west, south, east, north) in CRS units.
    pub fn bbox(&self, t: TileId) -> (f64, f64, f64, f64) {
        let west = self.origin.0 + t.x as f64 * self.tile_size_m;
        let north = self.origin.1 - t.y as f64 * self.tile_size_m;
        (west, north - self.tile_size_m, west + self.tile_size_m, north)
    }

    pub fn neighbor(&self, t: TileId, dx: i64, dy: i64) -> Option<TileId> {
        let x = t.x as i64 + dx;
        let y = t.y as i64 + dy;
        if x < 0 || y < 0 || x >= self.tiles_x as i64 || y >= self.tiles_y as i64 {
            None
        } else {
            Some(TileId { x: x as usize, y: y as usize })
        }
    }
}
