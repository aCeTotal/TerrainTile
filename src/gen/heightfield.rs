use std::sync::Arc;

use crate::edit::store::EditStore;
use crate::gen::island;
use crate::gen::world::WorldParams;

/// The pipeline's height source: same contract as the old GDAL reader —
/// windowed reads in global pixel coordinates — but backed by the
/// procedural island generator plus the sculpted delta overlays. `Sync`,
/// so rayon workers share one instance.
pub struct HeightSource {
    world: WorldParams,
    edits: Arc<EditStore>,
}

impl HeightSource {
    pub fn new(world: WorldParams, edits: Arc<EditStore>) -> Self {
        Self { world, edits }
    }

    pub fn edits(&self) -> &EditStore {
        &self.edits
    }

    pub fn world(&self) -> &WorldParams {
        &self.world
    }

    /// Row-major heights for a window in global pixel coordinates. The
    /// window may extend outside the world (tile aprons at the border);
    /// the height function is defined everywhere.
    pub fn read(&self, px: i64, py: i64, w: usize, h: usize) -> Vec<f32> {
        let mut out = Vec::with_capacity(w * h);
        for row in 0..h {
            for col in 0..w {
                out.push(self.sample_px(px + col as i64, py + row as i64));
            }
        }
        out
    }

    /// Composite height (generator + sculpt delta) at a global sample.
    pub fn sample_px(&self, px: i64, py: i64) -> f32 {
        self.base_px(px, py) + self.edits.delta_at(px, py)
    }

    /// True if the sample-space bbox is certainly open sea in the
    /// generator AND has no sculpt overlays touching it.
    pub fn surely_sea(&self, x0: f64, y0: f64, x1: f64, y1: f64) -> bool {
        island::surely_sea(&self.world, x0, y0, x1, y1)
            && !self.edits.any_delta_in(
                (x0 / self.world.resolution).floor() as i64,
                (y0 / self.world.resolution).floor() as i64,
                (x1 / self.world.resolution).ceil() as i64,
                (y1 / self.world.resolution).ceil() as i64,
            )
    }

    /// Generator height only, without edit overlays.
    pub fn base_px(&self, px: i64, py: i64) -> f32 {
        let res = self.world.resolution;
        island::height_at(&self.world, px as f64 * res, py as f64 * res)
    }

    /// Composite height at a world coordinate in meters (x east, y south
    /// from the NW corner — the mesh x/z convention), nearest sample.
    pub fn height_at_m(&self, x: f64, y: f64) -> f32 {
        let res = self.world.resolution;
        self.sample_px((x / res).round() as i64, (y / res).round() as i64)
    }
}
