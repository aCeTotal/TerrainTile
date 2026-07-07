use std::path::PathBuf;

use crate::ortho::source::OrthoSource;
use crate::tile::masks::MaskParams;

#[derive(Clone, Debug)]
pub struct PipelineConfig {
    pub output: PathBuf,
    pub tile_size_m: f64,
    /// Shared edge vertices between neighboring tiles (crack-free).
    pub overlap: bool,
    /// Number of LOD levels (LOD0 = full resolution, each level halves).
    pub lods: usize,
    /// 0 = all cores.
    pub threads: usize,
    /// Height written into nodata samples (0 = sea level). Constant, so
    /// neighboring tiles always agree along shared edges.
    pub nodata_height: f32,
    /// Ignore resume markers and rebuild every tile.
    pub force: bool,
    pub masks: MaskParams,
    /// None = DTM-only masks, no orthophoto download.
    pub ortho: Option<OrthoSource>,
}
