//! Stable build fingerprints for incremental rebuilds. Each tile's
//! metadata records the hash of everything that influenced its meshes and
//! its class textures; on the next run only outputs whose inputs changed
//! are rebuilt. FNV-1a over canonical strings — stable across runs and
//! toolchains (std's DefaultHasher is not).

use crate::pipeline::config::PipelineConfig;

/// Bump to force a full rebuild after breaking output-format changes.
const VERSION: u32 = 3;

pub struct BuildHashes {
    pub mesh: String,
    pub masks: String,
}

pub fn compute(cfg: &PipelineConfig) -> BuildHashes {
    let w = &cfg.world;
    let base = format!(
        "v{VERSION};seed={};island={};margin={};tile={};res={}",
        w.seed, w.island_m, w.margin_m, w.tile_size_m, w.resolution
    );

    let mesh = format!("{base};lods={}", w.lods);

    // The whole class list (gates, weights, sharp, materials) fingerprints
    // the class textures; painted coverage is folded in per tile.
    let defs = serde_json::to_string(&cfg.classes).unwrap_or_default();
    let masks = format!("{base};classes={}", hex(fnv1a(&defs)));

    BuildHashes { mesh: hex(fnv1a(&mesh)), masks: hex(fnv1a(&masks)) }
}

pub fn fnv1a(s: &str) -> u64 {
    let mut h = 0xcbf29ce484222325u64;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

pub fn hex(v: u64) -> String {
    format!("{v:016x}")
}
