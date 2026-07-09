use std::path::PathBuf;

use crate::gen::world::WorldParams;
use crate::tile::classdef::ClassDef;

#[derive(Clone, Debug)]
pub struct PipelineConfig {
    pub output: PathBuf,
    pub world: WorldParams,
    /// 0 = all cores.
    pub threads: usize,
    /// Ignore resume markers and rebuild every tile.
    pub force: bool,
    /// Material classes (gates + painted coverage drive the weights).
    pub classes: Vec<ClassDef>,
}
