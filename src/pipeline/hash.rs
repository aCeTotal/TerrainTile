//! Stable build fingerprints for incremental rebuilds. Each tile's
//! metadata records the hash of everything that influenced its meshes and
//! its masks; on the next run only outputs whose inputs changed are
//! rebuilt. FNV-1a over canonical strings — stable across runs and
//! toolchains (std's DefaultHasher is not).

use crate::import::dataset::DatasetInfo;
use crate::ortho::source::Provider;
use crate::pipeline::config::PipelineConfig;

/// Bump to force a full rebuild after breaking output-format changes.
const VERSION: u32 = 1;

pub struct BuildHashes {
    pub mesh: String,
    pub masks: String,
}

pub fn compute(cfg: &PipelineConfig, info: &DatasetInfo, files: &[std::path::PathBuf]) -> BuildHashes {
    // Source fingerprint: name + size of every ORIGINAL input raster (not
    // the nodata-filled copies — their fill is versioned per tile instead,
    // so a fill-algorithm change rebuilds only tiles that had nodata).
    let mut src = String::new();
    for f in files {
        let name = f.file_name().and_then(|n| n.to_str()).unwrap_or("?");
        let len = std::fs::metadata(f).map(|m| m.len()).unwrap_or(0);
        src.push_str(&format!("{name}:{len};"));
    }
    let base = format!(
        "v{VERSION};crs={};res={};origin={:.3},{:.3};tile={};overlap={};nodata={};src={src}",
        info.crs,
        info.resolution,
        info.origin.0,
        info.origin.1,
        cfg.tile_size_m,
        cfg.overlap,
        cfg.nodata_height,
    );

    let mesh = format!("{base};lods={}", cfg.lods);

    let m = &cfg.masks;
    let ortho = match &cfg.ortho {
        None => "none".to_string(),
        Some(o) => match &o.provider {
            Provider::Nib { .. } => "nib".to_string(),
            Provider::Wms { base_url } => format!("wms:{}", strip_auth(base_url)),
            Provider::Xyz { url_template, zoom } => format!("xyz:{url_template}:{zoom}"),
        },
    };
    let masks = format!(
        "{base};masks={},{},{},{},{},{};ortho={ortho}",
        m.rock_slope_start,
        m.rock_slope_full,
        m.snow_height_start,
        m.snow_height_full,
        m.dirt_slope_start,
        m.dirt_slope_full,
    );

    BuildHashes { mesh: hex(fnv1a(&mesh)), masks: hex(fnv1a(&masks)) }
}

/// Credentials rotate without changing the imagery — don't let a fresh
/// ticket invalidate every mask.
fn strip_auth(url: &str) -> String {
    let (base, query) = url.split_once('?').unwrap_or((url, ""));
    let kept: Vec<&str> = query
        .split('&')
        .filter(|kv| {
            let key = kv.split('=').next().unwrap_or("").to_lowercase();
            !matches!(key.as_str(), "ticket" | "token" | "gkt")
        })
        .collect();
    format!("{base}?{}", kept.join("&"))
}

fn fnv1a(s: &str) -> u64 {
    let mut h = 0xcbf29ce484222325u64;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

fn hex(v: u64) -> String {
    format!("{v:016x}")
}
