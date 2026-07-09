use std::path::Path;

use anyhow::{Context, Result};

/// Write one grayscale mask layer as 8-bit PNG.
pub fn save_gray(path: &Path, size: usize, data: &[u8]) -> Result<()> {
    let img = image::GrayImage::from_raw(size as u32, size as u32, data.to_vec())
        .context("maskestørrelse stemmer ikke")?;
    img.save(path).with_context(|| format!("kan ikke skrive {}", path.display()))?;
    Ok(())
}

/// Pack four mask layers into one RGBA PNG — a splat texture the viewer
/// samples with a single fetch.
pub fn save_rgba(path: &Path, size: usize, layers: [&[u8]; 4]) -> Result<()> {
    let mut buf = Vec::with_capacity(size * size * 4);
    for i in 0..size * size {
        for layer in &layers {
            buf.push(layer[i]);
        }
    }
    let img = image::RgbaImage::from_raw(size as u32, size as u32, buf)
        .context("maskestørrelse stemmer ikke")?;
    img.save(path).with_context(|| format!("kan ikke skrive {}", path.display()))?;
    Ok(())
}
