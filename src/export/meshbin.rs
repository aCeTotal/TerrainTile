//! Binary mesh format `TTM1`, little-endian, made for direct upload to GPU
//! buffers from a Bevy AssetLoader:
//!
//! ```text
//! magic   [u8;4]  = "TTM1"
//! u32     vertex_count
//! u32     index_count
//! f32*3*N positions   (x=east, y=up, z=south; meters, tile-local origin NW)
//! f32*3*N normals
//! f32*2*N uvs         (0..1 over the tile)
//! f32*4*N tangents
//! u32*M   indices     (triangle list, CCW seen from above)
//! ```
//!
//! Written in one streaming pass per attribute block — no vertex buffers are
//! ever held in RAM, so memory use is independent of tile size.

use std::io::{BufWriter, Read, Write};
use std::path::Path;

use anyhow::{bail, Context, Result};

use crate::tile::mesh::LodGeometry;

pub const MAGIC: [u8; 4] = *b"TTM1";

pub fn write(path: &Path, geo: &LodGeometry) -> Result<()> {
    let file = std::fs::File::create(path)
        .with_context(|| format!("kan ikke skrive {}", path.display()))?;
    let mut w = BufWriter::with_capacity(1 << 16, file);
    let vc = geo.vc();

    w.write_all(&MAGIC)?;
    w.write_all(&(geo.vertex_count() as u32).to_le_bytes())?;
    w.write_all(&(geo.index_count() as u32).to_le_bytes())?;

    for i in 0..vc {
        for j in 0..vc {
            write_f32s(&mut w, &geo.position(i, j))?;
        }
    }
    for i in 0..vc {
        for j in 0..vc {
            write_f32s(&mut w, &geo.normal(i, j))?;
        }
    }
    for i in 0..vc {
        for j in 0..vc {
            write_f32s(&mut w, &geo.uv(i, j))?;
        }
    }
    for i in 0..vc {
        for j in 0..vc {
            write_f32s(&mut w, &geo.tangent(i, j))?;
        }
    }
    for i in 0..vc - 1 {
        for j in 0..vc - 1 {
            for idx in geo.quad(i, j) {
                w.write_all(&idx.to_le_bytes())?;
            }
        }
    }
    w.flush()?;
    Ok(())
}

#[inline]
fn write_f32s<W: Write>(w: &mut W, vals: &[f32]) -> std::io::Result<()> {
    for f in vals {
        w.write_all(&f.to_le_bytes())?;
    }
    Ok(())
}

/// Read only vertex count and positions — used by validation to compare
/// tile edges.
pub fn read_positions(path: &Path) -> Result<Vec<[f32; 3]>> {
    let mut file = std::fs::File::open(path)
        .with_context(|| format!("kan ikke lese {}", path.display()))?;
    let mut head = [0u8; 12];
    file.read_exact(&mut head)?;
    if head[0..4] != MAGIC {
        bail!("{}: feil magic", path.display());
    }
    let vcount = u32::from_le_bytes(head[4..8].try_into().unwrap()) as usize;
    let mut buf = vec![0u8; vcount * 12];
    file.read_exact(&mut buf)?;
    let mut out = Vec::with_capacity(vcount);
    for c in buf.chunks_exact(12) {
        out.push([
            f32::from_le_bytes(c[0..4].try_into().unwrap()),
            f32::from_le_bytes(c[4..8].try_into().unwrap()),
            f32::from_le_bytes(c[8..12].try_into().unwrap()),
        ]);
    }
    Ok(out)
}
