use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use gdal::Dataset;

/// Build a GDAL VRT that mosaics all input files into one virtual raster.
/// Workers open this instead of the individual files.
pub fn build_mosaic(files: &[PathBuf], out: &Path) -> Result<()> {
    let datasets: Vec<Dataset> = files
        .iter()
        .map(|f| Dataset::open(f).with_context(|| format!("kan ikke åpne {}", f.display())))
        .collect::<Result<_>>()?;
    let mut vrt = gdal::programs::raster::build_vrt(Some(out), &datasets, None)
        .context("klarte ikke å bygge VRT-mosaikk")?;
    vrt.flush_cache()?;
    Ok(())
}
