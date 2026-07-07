use std::path::Path;

use anyhow::{Context, Result};

/// Write one grayscale mask layer as 8-bit PNG.
pub fn save_gray(path: &Path, size: usize, data: &[u8]) -> Result<()> {
    let img = image::GrayImage::from_raw(size as u32, size as u32, data.to_vec())
        .context("maskestørrelse stemmer ikke")?;
    img.save(path).with_context(|| format!("kan ikke skrive {}", path.display()))?;
    Ok(())
}

/// Write the sampled orthophoto as RGB PNG.
pub fn save_rgb(path: &Path, size: usize, data: &[[u8; 3]]) -> Result<()> {
    let mut buf = Vec::with_capacity(size * size * 3);
    for px in data {
        buf.extend_from_slice(px);
    }
    let img = image::RgbImage::from_raw(size as u32, size as u32, buf)
        .context("ortostørrelse stemmer ikke")?;
    img.save(path).with_context(|| format!("kan ikke skrive {}", path.display()))?;
    Ok(())
}
