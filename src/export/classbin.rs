use std::path::Path;

use anyhow::Result;

use crate::tile::classes::TileClasses;

/// Raw class splat for the viewer, little-endian:
///
/// ```text
/// magic  [u8;4] = "TTC1"
/// u32    size            vertex grid edge
/// u8*4*size²  class indices (top-4 per sample)
/// u8*4*size²  weights, sum 255
/// ```
///
/// Raw bytes (not PNG) so the client can put them straight into
/// DataTextures without a premultiplying canvas round-trip.
pub fn write(path: &Path, c: &TileClasses) -> Result<()> {
    let mut buf = Vec::with_capacity(12 + c.idx.len() * 8);
    buf.extend_from_slice(b"TTC1");
    buf.extend_from_slice(&(c.size as u32).to_le_bytes());
    for v in &c.idx {
        buf.extend_from_slice(v);
    }
    for v in &c.w {
        buf.extend_from_slice(v);
    }
    std::fs::write(path, buf)?;
    Ok(())
}
