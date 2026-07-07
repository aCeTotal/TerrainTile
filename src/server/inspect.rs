use std::path::PathBuf;

use serde::Serialize;

use crate::import::dataset::{self, DatasetInfo};

/// Result of inspecting the chosen input. Zip archives are only listed
/// (central directory read) — never opened with GDAL, since GeoTIFF
/// metadata at the end of a compressed member would force a full
/// decompression per file.
pub enum Inspect {
    Full(DatasetInfo),
    Zip { zips: usize, rasters: usize },
}

/// JSON shape of a scan result for the browser.
#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum ScanDto {
    Full {
        files: usize,
        crs: String,
        resolution: f64,
        extent: [f64; 4],
        width_px: usize,
        height_px: usize,
    },
    Zip {
        zips: usize,
        rasters: usize,
    },
}

pub fn inspect(paths: &[PathBuf]) -> anyhow::Result<Inspect> {
    let is_zip = |p: &PathBuf| {
        p.extension().and_then(|e| e.to_str()).map(str::to_lowercase).as_deref() == Some("zip")
    };
    let zips: Vec<&PathBuf> = paths.iter().filter(|p| p.is_file() && is_zip(p)).collect();
    if zips.is_empty() {
        return Ok(Inspect::Full(dataset::scan(paths)?));
    }
    let mut rasters = 0;
    for z in &zips {
        rasters += crate::import::zips::list_rasters(z)?.len();
    }
    Ok(Inspect::Zip { zips: zips.len(), rasters })
}

impl Inspect {
    pub fn dto(&self) -> ScanDto {
        match self {
            Inspect::Full(info) => ScanDto::Full {
                files: info.files.len(),
                crs: info.crs.clone(),
                resolution: info.resolution,
                extent: [info.extent.0, info.extent.1, info.extent.2, info.extent.3],
                width_px: info.width_px,
                height_px: info.height_px,
            },
            Inspect::Zip { zips, rasters } => ScanDto::Zip { zips: *zips, rasters: *rasters },
        }
    }
}
