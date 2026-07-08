//! Clean the source rasters before mosaicking: fill enclosed nodata holes
//! and remove speck noise in open water.
//!
//! DTM deliveries contain nodata both over sea (correct: becomes the 0 m
//! water plane) and as coverage gaps inland (wrong: becomes giant craters).
//! The two are separated topologically: sea always reaches the raster edge,
//! holes are fully enclosed by valid data. Enclosed regions are filled by a
//! BFS wavefront averaging already-valid neighbors — deterministic, so
//! every run and every tile window sees identical values.
//!
//! Water also contains the opposite defect: tiny clusters of valid samples
//! from boats, buoys and wave returns. A valid component wholly surrounded
//! by nodata is kept if it is big enough to be a real islet (or touches the
//! raster edge — it may continue in the neighboring sheet) and erased to
//! nodata otherwise.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use gdal::raster::{Buffer, RasterCreationOptions};
use gdal::{Dataset, DriverManager};

/// Bump when the fill algorithm changes: tiles whose metadata records
/// `had_nodata` and an older fill version are rebuilt, everything else is
/// left alone.
pub const FILL_VERSION: u32 = 2;

/// Valid components up to this size (px = m²) surrounded by nodata are
/// treated as noise, not islets. 64 px = 8 x 8 m: boats and wave clutter go,
/// real skerries are practically always larger.
const MAX_SPECK_PX: usize = 64;

/// Returns the paths to mosaic: the filled copy (under `<out>/filled/`)
/// for sources with enclosed holes, the original otherwise. Results are
/// cached; a source is only reprocessed when it changes.
pub fn fill_all(
    files: &[PathBuf],
    out: &Path,
    mut log: impl FnMut(String),
) -> Result<Vec<PathBuf>> {
    let dir = out.join("filled");
    std::fs::create_dir_all(&dir)?;
    let mut result = Vec::with_capacity(files.len());
    for src in files {
        let name = src.file_name().and_then(|n| n.to_str()).context("filnavn")?;
        let dst = dir.join(name);
        let clean = dir.join(format!("{name}.clean"));
        if newer(&clean, src) {
            result.push(src.clone());
        } else if newer(&dst, src) {
            result.push(dst);
        } else if fill_one(src, &dst, name, &mut log)? {
            result.push(dst);
        } else {
            std::fs::write(&clean, b"")?;
            result.push(src.clone());
        }
    }
    Ok(result)
}

fn newer(cache: &Path, src: &Path) -> bool {
    match (cache.metadata(), src.metadata()) {
        (Ok(c), Ok(s)) => matches!(
            (c.modified(), s.modified()),
            (Ok(cm), Ok(sm)) if cm >= sm
        ),
        _ => false,
    }
}

/// Clean `src` into `dst`; false if the raster needed no changes.
fn fill_one(src: &Path, dst: &Path, name: &str, log: &mut impl FnMut(String)) -> Result<bool> {
    let ds = Dataset::open(src).with_context(|| format!("kan ikke åpne {}", src.display()))?;
    let (w, h) = ds.raster_size();
    let band = ds.rasterband(1)?;
    let Some(nd) = band.no_data_value() else { return Ok(false) };
    let nd = nd as f32;

    let buf = band.read_as::<f32>((0, 0), (w, h), (w, h), None).context("lesefeil")?;
    let mut data = buf.into_shape_and_vec().1;
    let is_nd = |v: f32| v.is_nan() || v == nd;

    // Speck noise first, so it never feeds the hole interpolation below.
    let removed = remove_specks(&mut data, w, h, nd, &is_nd);
    if removed > 0 {
        log(format!("{name}: fjernet {removed} piksler støy i vann (båter/bølger)"));
    }

    // Mark nodata connected to the raster edge (sea / outside coverage).
    let mut open = vec![false; w * h];
    let mut queue = VecDeque::new();
    let push = |i: usize, open: &mut Vec<bool>, queue: &mut VecDeque<usize>, data: &[f32]| {
        if !open[i] && is_nd(data[i]) {
            open[i] = true;
            queue.push_back(i);
        }
    };
    for x in 0..w {
        push(x, &mut open, &mut queue, &data);
        push((h - 1) * w + x, &mut open, &mut queue, &data);
    }
    for y in 0..h {
        push(y * w, &mut open, &mut queue, &data);
        push(y * w + w - 1, &mut open, &mut queue, &data);
    }
    while let Some(i) = queue.pop_front() {
        let (x, y) = (i % w, i / w);
        for (nx, ny) in [(x.wrapping_sub(1), y), (x + 1, y), (x, y.wrapping_sub(1)), (x, y + 1)] {
            if nx < w && ny < h {
                push(ny * w + nx, &mut open, &mut queue, &data);
            }
        }
    }

    // Seed the fill front: enclosed nodata with at least one valid neighbor.
    let mut filled = 0usize;
    let mut front = VecDeque::new();
    let mut queued = vec![false; w * h];
    for i in 0..w * h {
        if is_nd(data[i]) && !open[i] && has_valid_neighbor(&data, w, h, i, &is_nd) {
            front.push_back(i);
            queued[i] = true;
        }
    }
    if front.is_empty() && removed == 0 {
        return Ok(false);
    }
    while let Some(i) = front.pop_front() {
        let (x, y) = (i % w, i / w);
        let mut sum = 0f64;
        let mut cnt = 0f64;
        for (nx, ny) in neighbors8(x, y) {
            if nx < w && ny < h && !is_nd(data[ny * w + nx]) {
                sum += data[ny * w + nx] as f64;
                cnt += 1.0;
            }
        }
        data[i] = (sum / cnt) as f32;
        filled += 1;
        for (nx, ny) in neighbors8(x, y) {
            if nx < w && ny < h {
                let j = ny * w + nx;
                if is_nd(data[j]) && !open[j] && !queued[j] {
                    front.push_back(j);
                    queued[j] = true;
                }
            }
        }
    }
    if filled > 0 {
        log(format!("{name}: fylte {filled} piksler i innelukkede datahull"));
    }

    let gt = ds.geo_transform()?;
    let proj = ds.projection();
    let options = RasterCreationOptions::from_iter([
        "COMPRESS=DEFLATE".to_string(),
        "TILED=YES".to_string(),
        "BIGTIFF=IF_SAFER".to_string(),
    ]);
    let driver = DriverManager::get_driver_by_name("GTiff")?;
    let mut out = driver
        .create_with_band_type_with_options::<f32, _>(dst, w, h, 1, &options)
        .with_context(|| format!("kan ikke skrive {}", dst.display()))?;
    out.set_geo_transform(&gt)?;
    out.set_projection(&proj)?;
    let mut ob = out.rasterband(1)?;
    ob.set_no_data_value(Some(nd as f64))?;
    let mut buffer = Buffer::new((w, h), data);
    ob.write((0, 0), (w, h), &mut buffer)?;
    out.flush_cache()?;
    Ok(true)
}

/// Erase valid components of at most MAX_SPECK_PX pixels that are wholly
/// surrounded by nodata: boat/wave returns in open water. Components that
/// touch the raster edge are kept — they may continue in the next sheet.
/// Returns the number of erased pixels.
fn remove_specks(data: &mut [f32], w: usize, h: usize, nd: f32, is_nd: &impl Fn(f32) -> bool) -> usize {
    let mut visited = vec![false; w * h];
    let mut queue = VecDeque::new();
    let mut comp: Vec<usize> = Vec::new();
    let mut removed = 0usize;

    for start in 0..w * h {
        if visited[start] || is_nd(data[start]) {
            continue;
        }
        comp.clear();
        let mut oversize = false;
        let mut touches_edge = false;
        visited[start] = true;
        queue.push_back(start);
        while let Some(i) = queue.pop_front() {
            let (x, y) = (i % w, i / w);
            if x == 0 || y == 0 || x == w - 1 || y == h - 1 {
                touches_edge = true;
            }
            if comp.len() <= MAX_SPECK_PX {
                comp.push(i);
            } else {
                oversize = true;
            }
            // 8-connectivity: a chain of diagonal samples counts as one
            // formation, so skerry chains are judged by their full size.
            for (nx, ny) in neighbors8(x, y) {
                if nx < w && ny < h {
                    let j = ny * w + nx;
                    if !visited[j] && !is_nd(data[j]) {
                        visited[j] = true;
                        queue.push_back(j);
                    }
                }
            }
        }
        if !oversize && !touches_edge && comp.len() <= MAX_SPECK_PX {
            for &i in &comp {
                data[i] = nd;
            }
            removed += comp.len();
        }
    }
    removed
}

#[inline]
fn has_valid_neighbor(data: &[f32], w: usize, h: usize, i: usize, is_nd: &impl Fn(f32) -> bool) -> bool {
    let (x, y) = (i % w, i / w);
    neighbors8(x, y).into_iter().any(|(nx, ny)| nx < w && ny < h && !is_nd(data[ny * w + nx]))
}

#[inline]
fn neighbors8(x: usize, y: usize) -> [(usize, usize); 8] {
    let (xm, ym) = (x.wrapping_sub(1), y.wrapping_sub(1));
    [(xm, ym), (x, ym), (x + 1, ym), (xm, y), (x + 1, y), (xm, y + 1), (x, y + 1), (x + 1, y + 1)]
}

#[cfg(test)]
mod tests {
    use super::*;

    const ND: f32 = -32767.0;

    fn grid(w: usize, h: usize, cells: &[(usize, usize, f32)]) -> Vec<f32> {
        let mut d = vec![ND; w * h];
        for &(x, y, v) in cells {
            d[y * w + x] = v;
        }
        d
    }

    #[test]
    fn speck_in_open_water_is_erased() {
        let mut d = grid(32, 32, &[(10, 10, 2.0), (11, 10, 2.5), (10, 11, 1.5)]);
        let is_nd = |v: f32| v.is_nan() || v == ND;
        assert_eq!(remove_specks(&mut d, 32, 32, ND, &is_nd), 3);
        assert!(d.iter().all(|v| *v == ND));
    }

    #[test]
    fn islet_larger_than_speck_limit_is_kept() {
        // 9 x 9 block = 81 px > MAX_SPECK_PX.
        let cells: Vec<_> =
            (0..9).flat_map(|dy| (0..9).map(move |dx| (10 + dx, 10 + dy, 5.0))).collect();
        let mut d = grid(64, 64, &cells);
        let is_nd = |v: f32| v.is_nan() || v == ND;
        assert_eq!(remove_specks(&mut d, 64, 64, ND, &is_nd), 0);
        assert_eq!(d.iter().filter(|v| **v != ND).count(), 81);
    }

    #[test]
    fn small_component_touching_edge_is_kept() {
        // May continue in the neighboring sheet — never erase.
        let mut d = grid(32, 32, &[(0, 5, 3.0), (1, 5, 3.0)]);
        let is_nd = |v: f32| v.is_nan() || v == ND;
        assert_eq!(remove_specks(&mut d, 32, 32, ND, &is_nd), 0);
    }

    #[test]
    fn diagonal_chain_counts_as_one_formation() {
        // 70 diagonal-connected px: one formation > MAX_SPECK_PX, kept.
        let cells: Vec<_> = (0..35).flat_map(|i| [(2 + i, 2 + i, 1.0), (3 + i, 2 + i, 1.0)]).collect();
        let mut d = grid(64, 64, &cells);
        let is_nd = |v: f32| v.is_nan() || v == ND;
        assert_eq!(remove_specks(&mut d, 64, 64, ND, &is_nd), 0);
    }
}
