use std::path::Path;

use anyhow::{Context, Result};
use gdal::Dataset;

/// Streaming, windowed reader over the mosaic VRT. Never loads the full
/// dataset; each call reads only the requested window from disk.
pub struct HeightReader {
    ds: Dataset,
    width: i64,
    height: i64,
    nodata: Option<f64>,
    /// Height used for nodata samples (typically 0 = sea level).
    fill: f32,
}

impl HeightReader {
    pub fn open(path: &Path, fill: f32) -> Result<Self> {
        let ds = Dataset::open(path).with_context(|| format!("kan ikke åpne {}", path.display()))?;
        let (w, h) = ds.raster_size();
        let nodata = ds.rasterband(1)?.no_data_value();
        Ok(Self { ds, width: w as i64, height: h as i64, nodata, fill })
    }

    /// Read a `w x h` window starting at pixel (px, py). The window may
    /// extend outside the raster; out-of-range samples replicate the edge,
    /// so neighboring tiles always see identical border values.
    /// Returns (heights row-major, had_nodata).
    pub fn read(&self, px: i64, py: i64, w: usize, h: usize) -> Result<(Vec<f32>, bool)> {
        let x0 = px.clamp(0, self.width - 1);
        let y0 = py.clamp(0, self.height - 1);
        let x1 = (px + w as i64).clamp(1, self.width);
        let y1 = (py + h as i64).clamp(1, self.height);
        let (iw, ih) = ((x1 - x0) as usize, (y1 - y0) as usize);

        let band = self.ds.rasterband(1)?;
        let buf = band
            .read_as::<f32>((x0 as isize, y0 as isize), (iw, ih), (iw, ih), None)
            .context("lesefeil i høydedata")?;
        let inner = buf.data();

        let mut out = vec![0f32; w * h];
        for row in 0..h {
            let sy = ((py + row as i64).clamp(y0, y1 - 1) - y0) as usize;
            for col in 0..w {
                let sx = ((px + col as i64).clamp(x0, x1 - 1) - x0) as usize;
                out[row * w + col] = inner[sy * iw + sx];
            }
        }

        let had_nodata = self.fill_nodata(&mut out, w, h);
        Ok((out, had_nodata))
    }

    /// Replace nodata samples with the constant fill height. The value must
    /// depend only on the sample's global position — never on the read
    /// window — otherwise neighboring tiles disagree along shared edges
    /// and validation reports cracks.
    fn fill_nodata(&self, data: &mut [f32], _w: usize, _h: usize) -> bool {
        let nd = self.nodata.map(|n| n as f32);
        let mut found = false;
        for v in data.iter_mut() {
            if v.is_nan() || Some(*v) == nd {
                *v = self.fill;
                found = true;
            }
        }
        found
    }
}
