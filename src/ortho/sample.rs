use std::collections::HashMap;

use anyhow::{Context, Result};
use gdal::spatial_ref::{CoordTransform, SpatialRef};
use image::RgbImage;

use crate::ortho::fetch::Fetcher;
use crate::ortho::source::Provider;

/// RGB samples aligned 1:1 with the tile's vertex grid. u8 keeps a 2049x2049
/// grid at ~12 MB instead of ~50 MB.
pub struct RgbGrid {
    pub size: usize,
    pub data: Vec<[u8; 3]>,
}

/// Sample the orthophoto at every vertex position of a terrain tile.
pub fn sample_tile(
    fetcher: &Fetcher,
    provider: &Provider,
    crs: &str,
    crs_wkt: &str,
    bbox: (f64, f64, f64, f64),
    size: usize,
    cache_key: &str,
) -> Result<RgbGrid> {
    match provider {
        Provider::Nib { .. } => sample_nib(fetcher, crs, crs_wkt, bbox, size),
        Provider::Wms { base_url } => sample_wms(fetcher, base_url, crs, bbox, size, cache_key),
        Provider::Xyz { url_template, zoom } => {
            sample_xyz(fetcher, url_template, *zoom, crs_wkt, bbox, size)
        }
    }
}

/// Norge i bilder: tiles live in a native-UTM ArcGIS tile cache. When the
/// dataset shares the cache CRS (hoydedata.no = EPSG:25833 = Nibcache UTM33)
/// vertex positions map straight into cache pixels — no reprojection at
/// all. Other CRS are transformed row by row.
fn sample_nib(
    fetcher: &Fetcher,
    crs: &str,
    crs_wkt: &str,
    bbox: (f64, f64, f64, f64),
    size: usize,
) -> Result<RgbGrid> {
    use crate::ortho::nib::NibClient;
    let client = fetcher.nib.as_ref().context("NiB-klient mangler")?;
    let epsg: u32 = crs.rsplit(':').next().and_then(|c| c.parse().ok()).unwrap_or(0);
    let service = NibClient::service_for_epsg(epsg);
    let info = client.tile_info(service)?;

    let (west, south, east, north) = bbox;
    let target_res = (east - west) / (size - 1) as f64;
    // Coarsest cache level that still meets the dataset resolution; if the
    // cache has nothing that fine, use its finest level.
    let (level, res) = info
        .lods
        .iter()
        .copied()
        .filter(|(_, r)| *r <= target_res * 1.001)
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .or_else(|| info.lods.iter().copied().min_by(|a, b| a.1.total_cmp(&b.1)))
        .unwrap();

    let same_crs = info.wkid == epsg;
    let tf = if same_crs {
        None
    } else {
        let src = SpatialRef::from_wkt(crs_wkt)?;
        let dst = SpatialRef::from_epsg(info.wkid)?;
        Some(CoordTransform::new(&src, &dst).context("CRS-transformasjon feilet")?)
    };

    let step_x = (east - west) / (size - 1) as f64;
    let step_y = (north - south) / (size - 1) as f64;
    let tile_px = info.tile_px as u64;
    let mut tiles: HashMap<(u64, u64), RgbImage> = HashMap::new();
    let mut data = Vec::with_capacity(size * size);
    let mut xs = vec![0f64; size];
    let mut ys = vec![0f64; size];
    let mut zs = vec![0f64; size];
    for i in 0..size {
        for j in 0..size {
            xs[j] = west + j as f64 * step_x;
            ys[j] = north - i as f64 * step_y;
            zs[j] = 0.0;
        }
        if let Some(tf) = &tf {
            tf.transform_coords(&mut xs, &mut ys, &mut zs)
                .context("koordinattransformasjon feilet")?;
        }
        for j in 0..size {
            let px = (xs[j] - info.origin.0) / res;
            let py = (info.origin.1 - ys[j]) / res;
            data.push(bilinear(px, py, |gx, gy| {
                let key = (gx / tile_px, gy / tile_px);
                if !tiles.contains_key(&key) {
                    let img = client.get_tile(service, level, key.1, key.0)?;
                    tiles.insert(key, img);
                }
                let img = &tiles[&key];
                let p = img.get_pixel(
                    ((gx % tile_px) as u32).min(img.width() - 1),
                    ((gy % tile_px) as u32).min(img.height() - 1),
                );
                Ok([p[0], p[1], p[2]])
            })?);
        }
    }
    Ok(RgbGrid { size, data })
}

/// Bilinear interpolation over a pixel-source closure.
fn bilinear(
    px: f64,
    py: f64,
    mut src: impl FnMut(u64, u64) -> Result<[u8; 3]>,
) -> Result<[u8; 3]> {
    let x0 = px.floor().max(0.0);
    let y0 = py.floor().max(0.0);
    let fx = (px - x0) as f32;
    let fy = (py - y0) as f32;
    let mut c = [[0f32; 3]; 4];
    for (k, (dx, dy)) in [(0u64, 0u64), (1, 0), (0, 1), (1, 1)].iter().enumerate() {
        let p = src(x0 as u64 + dx, y0 as u64 + dy)?;
        c[k] = [p[0] as f32, p[1] as f32, p[2] as f32];
    }
    let mut out = [0u8; 3];
    for ch in 0..3 {
        let top = c[0][ch] * (1.0 - fx) + c[1][ch] * fx;
        let bot = c[2][ch] * (1.0 - fx) + c[3][ch] * fx;
        out[ch] = (top * (1.0 - fy) + bot * fy).round().clamp(0.0, 255.0) as u8;
    }
    Ok(out)
}

/// WMS path (Norge i bilder): request the exact tile bbox in the dataset CRS,
/// expanded half a pixel so WMS pixel centers land exactly on the vertex
/// grid — pixel-perfect masks.
fn sample_wms(
    fetcher: &Fetcher,
    base_url: &str,
    crs: &str,
    bbox: (f64, f64, f64, f64),
    size: usize,
    cache_key: &str,
) -> Result<RgbGrid> {
    let res = (bbox.2 - bbox.0) / (size - 1) as f64;
    let half = res / 2.0;
    let req = (bbox.0 - half, bbox.1 - half, bbox.2 + half, bbox.3 + half);
    let img = fetcher.get_wms(base_url, crs, req, size, cache_key)?;
    let mut data = Vec::with_capacity(size * size);
    for p in img.pixels() {
        data.push([p[0], p[1], p[2]]);
    }
    Ok(RgbGrid { size, data })
}

const MERC_HALF: f64 = 20037508.342789244;
const TILE_PX: u64 = 256;

/// XYZ path: transform vertex positions row by row (bounded memory) from
/// the dataset CRS to WebMercator, fetch the covering tiles (disk-cached)
/// and sample bilinearly.
fn sample_xyz(
    fetcher: &Fetcher,
    url_template: &str,
    zoom: u8,
    crs_wkt: &str,
    bbox: (f64, f64, f64, f64),
    size: usize,
) -> Result<RgbGrid> {
    let src = SpatialRef::from_wkt(crs_wkt)?;
    let dst = SpatialRef::from_epsg(3857)?;
    let tf = CoordTransform::new(&src, &dst).context("CRS-transformasjon feilet")?;

    let (west, south, east, north) = bbox;
    let step_x = (east - west) / (size - 1) as f64;
    let step_y = (north - south) / (size - 1) as f64;
    let world_px = (TILE_PX * (1u64 << zoom)) as f64;

    let mut tiles: HashMap<(u64, u64), RgbImage> = HashMap::new();
    let mut data = Vec::with_capacity(size * size);
    let mut xs = vec![0f64; size];
    let mut ys = vec![0f64; size];
    let mut zs = vec![0f64; size];
    for i in 0..size {
        for j in 0..size {
            xs[j] = west + j as f64 * step_x;
            ys[j] = north - i as f64 * step_y;
            zs[j] = 0.0;
        }
        tf.transform_coords(&mut xs, &mut ys, &mut zs)
            .context("koordinattransformasjon feilet")?;
        for j in 0..size {
            let px = ((xs[j] + MERC_HALF) / (2.0 * MERC_HALF) * world_px)
                .clamp(0.0, world_px - 1.0);
            let py = ((MERC_HALF - ys[j]) / (2.0 * MERC_HALF) * world_px)
                .clamp(0.0, world_px - 1.0);
            data.push(sample_px(fetcher, url_template, &mut tiles, zoom, px, py)?);
        }
        // Drop image tiles no longer reachable from later rows (rows move
        // south monotonically) to keep the working set small.
        let min_ty = (py_floor(&ys, world_px) / TILE_PX).saturating_sub(1);
        tiles.retain(|(_, ty), _| *ty + 1 >= min_ty);
    }
    Ok(RgbGrid { size, data })
}

fn py_floor(ys: &[f64], world_px: f64) -> u64 {
    let my = ys.iter().cloned().fold(f64::MIN, f64::max);
    (((MERC_HALF - my) / (2.0 * MERC_HALF) * world_px).clamp(0.0, world_px - 1.0)) as u64
}

/// Bilinear sample at global mercator pixel coordinates.
fn sample_px(
    fetcher: &Fetcher,
    url_template: &str,
    tiles: &mut HashMap<(u64, u64), RgbImage>,
    zoom: u8,
    px: f64,
    py: f64,
) -> Result<[u8; 3]> {
    let x0 = px.floor();
    let y0 = py.floor();
    let fx = (px - x0) as f32;
    let fy = (py - y0) as f32;
    let max = TILE_PX * (1u64 << zoom) - 1;

    let mut c = [[0f32; 3]; 4];
    for (k, (dx, dy)) in [(0u64, 0u64), (1, 0), (0, 1), (1, 1)].iter().enumerate() {
        let gx = (x0 as u64 + dx).min(max);
        let gy = (y0 as u64 + dy).min(max);
        let key = (gx / TILE_PX, gy / TILE_PX);
        if !tiles.contains_key(&key) {
            let img = fetcher.get_xyz(url_template, zoom, key.0, key.1)?;
            tiles.insert(key, img);
        }
        let img = &tiles[&key];
        let p = img.get_pixel((gx % TILE_PX) as u32, (gy % TILE_PX) as u32);
        c[k] = [p[0] as f32, p[1] as f32, p[2] as f32];
    }
    let mut out = [0u8; 3];
    for ch in 0..3 {
        let top = c[0][ch] * (1.0 - fx) + c[1][ch] * fx;
        let bot = c[2][ch] * (1.0 - fx) + c[3][ch] * fx;
        out[ch] = (top * (1.0 - fy) + bot * fy).round().clamp(0.0, 255.0) as u8;
    }
    Ok(out)
}
