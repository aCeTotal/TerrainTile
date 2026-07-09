use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

use crate::tile::grid::TileGrid;

/// Everything that defines a generated world: one big island surrounded by
/// sea on every side. Serialized into project.json; two worlds with equal
/// params are bitwise identical.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct WorldParams {
    pub seed: u64,
    /// Island edge length in meters (island area ≈ island_m²).
    pub island_m: f64,
    /// Sea margin in meters on every side (1 mil = 10 000 m).
    pub margin_m: f64,
    pub tile_size_m: f64,
    /// Meters per height sample.
    pub resolution: f64,
    /// Number of LOD levels (LOD0 = full resolution, each level halves).
    pub lods: usize,
}

impl Default for WorldParams {
    fn default() -> Self {
        Self {
            seed: 1,
            island_m: 22360.0, // ≈ 500 km²
            margin_m: 50_000.0, // 5 mil
            tile_size_m: 1024.0,
            resolution: 4.0,
            lods: 6,
        }
    }
}

impl WorldParams {
    /// World edge length: island + sea margin on both sides, rounded UP to
    /// whole tiles.
    pub fn size_m(&self) -> f64 {
        ((self.island_m + 2.0 * self.margin_m) / self.tile_size_m).ceil() * self.tile_size_m
    }

    /// Island center = world center.
    pub fn center(&self) -> (f64, f64) {
        (self.size_m() / 2.0, self.size_m() / 2.0)
    }

    /// Height samples per world edge.
    pub fn width_px(&self) -> usize {
        (self.size_m() / self.resolution).round() as usize
    }

    pub fn validate(&self) -> Result<()> {
        if self.island_m < 8000.0 {
            bail!("øya må være minst 8 km bred");
        }
        if !(20_000.0..=100_000.0).contains(&self.margin_m) {
            bail!("havmarginen må være 2–10 mil");
        }
        if self.tile_size_m < 256.0 {
            bail!("flisstørrelsen må være minst 256 m");
        }
        self.grid()?; // tile_px integer + divisible by the largest LOD stride
        Ok(())
    }

    pub fn grid(&self) -> Result<TileGrid> {
        let size = self.size_m();
        TileGrid::new(
            self.resolution,
            self.width_px(),
            self.width_px(),
            (0.0, size),
            self.tile_size_m,
            self.lods,
        )
    }
}
