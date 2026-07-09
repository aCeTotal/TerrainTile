//! Class compositor: turns terrain gates × painted coverage into a per-
//! vertex top-4 (class index, weight) representation the shader samples
//! directly. Weights are computed over an apron-padded grid and blurred
//! for soft class transitions BEFORE cropping, so both sides of a tile
//! seam see identical values — no visible mask seams.

use crate::edit::cover::Cover;
use crate::tile::classdef::ClassDef;
use crate::tile::mesh::{normal_at, HeightPatch};

/// Blur passes for the soft default transitions; `sharp` classes skip it.
const BLUR_PASSES: usize = 3;

/// Extra samples around the vertex grid so the blur has real neighbors.
/// The tile builder guarantees the patch apron is at least this wide —
/// otherwise the blur would read window-clamped values and break seams.
pub const PAD: usize = BLUR_PASSES;

pub struct TileClasses {
    /// Vertex grid edge (n + 1).
    pub size: usize,
    /// Top-4 class ids per sample.
    pub idx: Vec<[u8; 4]>,
    /// Matching weights, sum 255.
    pub w: Vec<[u8; 4]>,
}

impl TileClasses {
    /// Reconstruct one class's grayscale weight layer (for engine export).
    pub fn layer(&self, class: u8) -> Vec<u8> {
        let mut out = vec![0u8; self.size * self.size];
        for (i, o) in out.iter_mut().enumerate() {
            for k in 0..4 {
                if self.idx[i][k] == class {
                    *o = self.w[i][k];
                }
            }
        }
        out
    }

    /// Class ids that actually appear in the tile.
    pub fn present(&self) -> Vec<u8> {
        let mut seen = [false; 256];
        for (i, ids) in self.idx.iter().enumerate() {
            for k in 0..4 {
                if self.w[i][k] > 0 {
                    seen[ids[k] as usize] = true;
                }
            }
        }
        (0u16..256).filter(|c| seen[*c as usize]).map(|c| c as u8).collect()
    }
}

/// `(opx, opy)` = tile origin in global height samples; `res` m/sample.
pub fn composite(
    patch: &HeightPatch,
    res: f64,
    classes: &[ClassDef],
    cover: &Cover,
    opx: i64,
    opy: i64,
) -> TileClasses {
    let n = patch.n;
    let size = n + 1;
    let padded = size + 2 * PAD;

    // Raw weight fields per class over the padded grid.
    let mut fields: Vec<Vec<f32>> = Vec::with_capacity(classes.len());
    for c in classes {
        let mut f = vec![0.0f32; padded * padded];
        for gy in 0..padded {
            for gx in 0..padded {
                let i = (gx as i64) - PAD as i64;
                let j = (gy as i64) - PAD as i64;
                let h = patch.h(j, i);
                let nrm = normal_at(patch, j, i, 1, res);
                let slope = nrm[1].clamp(-1.0, 1.0).acos().to_degrees();
                let gate = c.gate(h, slope);
                if gate <= 0.0 {
                    continue;
                }
                let coverage = if c.base {
                    1.0
                } else {
                    cover.sample(c.id, (opx + i) as f64 * res, (opy + j) as f64 * res)
                };
                f[gy * padded + gx] = gate * coverage * c.weight.max(0.0);
            }
        }
        if !c.sharp {
            for _ in 0..BLUR_PASSES {
                blur3f(&mut f, padded);
            }
        }
        fields.push(f);
    }

    // Crop, normalize, top-4.
    let mut idx = vec![[0u8; 4]; size * size];
    let mut w = vec![[0u8; 4]; size * size];
    for y in 0..size {
        for x in 0..size {
            let src = (y + PAD) * padded + (x + PAD);
            let dst = y * size + x;
            let mut top: [(f32, u8); 4] = [(0.0, 0); 4];
            for (ci, f) in fields.iter().enumerate() {
                let v = f[src];
                if v > top[3].0 {
                    top[3] = (v, classes[ci].id as u8);
                    top.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
                }
            }
            let sum: f32 = top.iter().map(|t| t.0).sum();
            if sum <= 0.0 {
                // Shouldn't happen with a base filler class, but never
                // leave a hole: fall back to the first class.
                idx[dst] = [classes[0].id as u8; 4];
                w[dst] = [255, 0, 0, 0];
                continue;
            }
            let mut acc = 0u32;
            for k in 0..4 {
                idx[dst][k] = top[k].1;
                let b = if k == 3 {
                    (255 - acc.min(255)) as u8
                } else {
                    ((top[k].0 / sum * 255.0).round() as u32).min(255 - acc.min(255)) as u8
                };
                w[dst][k] = b;
                acc += b as u32;
            }
        }
    }
    TileClasses { size, idx, w }
}

/// In-place 3×3 box blur on f32, sliding three-row window.
fn blur3f(data: &mut [f32], size: usize) {
    let mut prev: Option<Vec<f32>> = None;
    let mut curr = data[0..size].to_vec();
    for i in 0..size {
        let next = (i + 1 < size).then(|| data[(i + 1) * size..(i + 2) * size].to_vec());
        for j in 0..size {
            let mut sum = 0.0f32;
            let mut cnt = 0.0f32;
            for row in [prev.as_ref(), Some(&curr), next.as_ref()].into_iter().flatten() {
                for dj in -1i64..=1 {
                    let jj = j as i64 + dj;
                    if jj >= 0 && jj < size as i64 {
                        sum += row[jj as usize];
                        cnt += 1.0;
                    }
                }
            }
            data[i * size + j] = sum / cnt;
        }
        prev = Some(std::mem::take(&mut curr));
        curr = next.unwrap_or_default();
    }
}
