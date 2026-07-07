use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use gdal::Dataset;

/// Summary of one or more height rasters that together form the input dataset.
#[derive(Debug, Clone)]
pub struct DatasetInfo {
    pub files: Vec<PathBuf>,
    /// CRS key, e.g. "EPSG:25833".
    pub crs: String,
    pub crs_wkt: String,
    /// Meters per pixel (square pixels required).
    pub resolution: f64,
    /// Top-left (west, north) of merged extent in CRS units.
    pub origin: (f64, f64),
    /// (west, south, east, north)
    pub extent: (f64, f64, f64, f64),
    pub width_px: usize,
    pub height_px: usize,
}

const EPS: f64 = 1e-6;

/// Scan a folder or file list for height rasters and validate that they
/// share CRS, resolution and pixel grid. Zip archives are extracted by the
/// pipeline before this runs — GDAL must never read GeoTIFFs through a
/// compressed zip stream (metadata at the file end forces a full
/// decompression per file).
pub fn scan(paths: &[PathBuf]) -> Result<DatasetInfo> {
    let mut files: Vec<PathBuf> = Vec::new();
    for p in paths {
        if p.is_dir() {
            collect_rasters(p, &mut files)?;
        } else {
            files.push(p.clone());
        }
    }
    files.sort();
    files.dedup();
    if files.is_empty() {
        bail!("fant ingen høydefiler (.tif/.tiff)");
    }

    let mut crs: Option<(String, String)> = None;
    let mut resolution: Option<f64> = None;
    let mut extent = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    let mut corners: Vec<(f64, f64)> = Vec::with_capacity(files.len());

    for f in &files {
        let ds = Dataset::open(f).with_context(|| format!("kan ikke åpne {}", f.display()))?;
        let sr = ds
            .spatial_ref()
            .with_context(|| format!("{}: mangler koordinatsystem", f.display()))?;
        if sr.is_geographic() {
            bail!(
                "{}: geografisk CRS (lat/lon). Reprojiser til projisert CRS først, f.eks:\n\
                 gdalwarp -t_srs EPSG:25833 inn.tif ut.tif",
                f.display()
            );
        }
        let key = match (sr.auth_name(), sr.auth_code()) {
            (Some(n), Ok(c)) => format!("{n}:{c}"),
            _ => sr.to_wkt().unwrap_or_default(),
        };
        let wkt = sr.to_wkt().unwrap_or_default();
        match &crs {
            None => crs = Some((key, wkt)),
            Some((k, _)) if *k != key => {
                bail!("{}: CRS {} avviker fra {}", f.display(), key, k)
            }
            _ => {}
        }

        let gt = ds.geo_transform()?;
        if gt[2].abs() > EPS || gt[4].abs() > EPS {
            bail!("{}: rotert raster støttes ikke", f.display());
        }
        let (rx, ry) = (gt[1], -gt[5]);
        if (rx - ry).abs() > EPS {
            bail!("{}: ikke kvadratiske piksler ({rx} x {ry})", f.display());
        }
        match resolution {
            None => resolution = Some(rx),
            Some(r) if (r - rx).abs() > EPS => {
                bail!("{}: oppløsning {rx} m avviker fra {r} m", f.display())
            }
            _ => {}
        }

        let (w, h) = ds.raster_size();
        let west = gt[0];
        let north = gt[3];
        let east = west + rx * w as f64;
        let south = north - rx * h as f64;
        corners.push((west, north));
        extent.0 = extent.0.min(west);
        extent.1 = extent.1.min(south);
        extent.2 = extent.2.max(east);
        extent.3 = extent.3.max(north);
    }

    let resolution = resolution.unwrap();

    // All files must sit on the same global pixel grid — otherwise the VRT
    // would resample and tiles could not match 100% at their seams.
    for (f, (west, north)) in files.iter().zip(&corners) {
        let dx = (west - extent.0) / resolution;
        let dy = (extent.3 - north) / resolution;
        if (dx - dx.round()).abs() > 1e-6 || (dy - dy.round()).abs() > 1e-6 {
            bail!(
                "{}: ligger ikke på samme pikselgrid som resten (offset {dx:.6}, {dy:.6} px)",
                f.display()
            );
        }
    }
    let (crs, crs_wkt) = crs.unwrap();
    let width_px = ((extent.2 - extent.0) / resolution).round() as usize;
    let height_px = ((extent.3 - extent.1) / resolution).round() as usize;

    Ok(DatasetInfo {
        files,
        crs,
        crs_wkt,
        resolution,
        origin: (extent.0, extent.3),
        extent,
        width_px,
        height_px,
    })
}

fn collect_rasters(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            collect_rasters(&path, out)?;
        } else if matches!(
            path.extension().and_then(|e| e.to_str()).map(str::to_lowercase).as_deref(),
            Some("tif") | Some("tiff")
        ) {
            out.push(path);
        }
    }
    Ok(())
}
