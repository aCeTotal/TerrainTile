use serde::{Deserialize, Serialize};

use crate::ortho::sample::RgbGrid;
use crate::tile::mesh::{normal_at, HeightPatch};

/// Thresholds for DTM-based classification. Photo-based rules refine these.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct MaskParams {
    pub rock_slope_start: f32,
    pub rock_slope_full: f32,
    pub snow_height_start: f32,
    pub snow_height_full: f32,
    pub dirt_slope_start: f32,
    pub dirt_slope_full: f32,
}

impl Default for MaskParams {
    fn default() -> Self {
        Self {
            rock_slope_start: 35.0,
            rock_slope_full: 50.0,
            snow_height_start: 1000.0,
            snow_height_full: 1250.0,
            dirt_slope_start: 18.0,
            dirt_slope_full: 32.0,
        }
    }
}

pub const MASK_NAMES: [&str; 8] =
    ["grass", "forest", "rock", "dirt", "sand", "snow", "water", "road"];

const GRASS: usize = 0;
const FOREST: usize = 1;
const ROCK: usize = 2;
const DIRT: usize = 3;
const SAND: usize = 4;
const SNOW: usize = 5;
const WATER: usize = 6;
const ROAD: usize = 7;

/// Soft material weights per vertex sample; `(n+1)^2` per layer. Stored as
/// u8 throughout to keep memory low; all material layers sum to 255 per
/// pixel. Vegetation densities (trees/bushes, photo-derived) are separate
/// unnormalized layers for instancing in the engine.
pub struct TileMasks {
    pub size: usize,
    pub layers: Vec<Vec<u8>>,
    pub trees: Option<Vec<u8>>,
    pub bushes: Option<Vec<u8>>,
}

/// Classify every vertex sample from slope/height (DTM) and, when available,
/// orthophoto color. Photo rules give the detail; DTM rules are the fallback
/// and hard physical constraints (steep = rock, high = snow).
pub fn classify(
    patch: &HeightPatch,
    res: f64,
    ortho: Option<&RgbGrid>,
    p: &MaskParams,
) -> TileMasks {
    let size = patch.n + 1;
    let mut scores = vec![vec![0u8; size * size]; MASK_NAMES.len()];

    // Luminance grid for texture analysis: tree canopies have high local
    // contrast in orthophotos, mown grass and fields are smooth.
    let lum: Option<Vec<u8>> = ortho.map(|o| {
        o.data
            .iter()
            .map(|[r, g, b]| {
                (0.299 * *r as f32 + 0.587 * *g as f32 + 0.114 * *b as f32).round() as u8
            })
            .collect()
    });
    let mut trees = ortho.map(|_| vec![0u8; size * size]);
    let mut bushes = ortho.map(|_| vec![0u8; size * size]);

    for i in 0..size {
        for j in 0..size {
            let idx = i * size + j;
            let h = patch.h(i as i64, j as i64);
            let nrm = normal_at(patch, i as i64, j as i64, 1, res);
            let slope = nrm[1].clamp(-1.0, 1.0).acos().to_degrees();

            let rock_dtm = smooth(p.rock_slope_start, p.rock_slope_full, slope);
            let snow_dtm = smooth(p.snow_height_start, p.snow_height_full, h);
            let mut s = [0f32; 8];

            if let Some(o) = ortho {
                let [r, g, b] = o.data[idx];
                let (r, g, b) = (r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0);
                let v = 0.299 * r + 0.587 * g + 0.114 * b;
                let maxc = r.max(g).max(b);
                let sat = if maxc > 1e-6 { (maxc - r.min(g).min(b)) / maxc } else { 0.0 };
                let exg = 2.0 * g - r - b;
                let veg = smooth(0.02, 0.15, exg);

                s[SNOW] = smooth(0.70, 0.85, v)
                    * (1.0 - smooth(0.15, 0.35, sat))
                    * (0.4 + 0.6 * snow_dtm);
                s[WATER] = (1.0 - smooth(0.25, 0.45, v))
                    * smooth(-0.05, 0.05, b - r)
                    * (1.0 - smooth(1.0, 4.0, slope));
                s[FOREST] = veg * (1.0 - smooth(0.30, 0.45, v));
                s[GRASS] = veg * smooth(0.28, 0.42, v);
                s[ROAD] = (1.0 - smooth(0.08, 0.18, sat))
                    * band(v, 0.18, 0.30, 0.45, 0.58)
                    * (1.0 - smooth(3.0, 8.0, slope))
                    * (1.0 - veg);
                s[SAND] = smooth(0.08, 0.18, r - b)
                    * smooth(0.45, 0.60, v)
                    * (1.0 - veg)
                    * (1.0 - smooth(5.0, 15.0, slope));
                s[DIRT] = smooth(0.02, 0.09, r - g)
                    * smooth(0.03, 0.12, g - b)
                    * band(v, 0.20, 0.32, 0.50, 0.65)
                    * (1.0 - veg);
                s[ROCK] = (1.0 - smooth(0.15, 0.35, sat))
                    * band(v, 0.28, 0.40, 0.70, 0.85)
                    * smooth(20.0, 35.0, slope)
                    * (1.0 - veg);
                // Hard DTM constraints override photo ambiguity.
                s[ROCK] = s[ROCK].max(rock_dtm * (1.0 - s[SNOW]));
                s[SNOW] = s[SNOW].max(snow_dtm * (1.0 - rock_dtm));

                let tex = texture_at(lum.as_ref().unwrap(), size, i, j);
                let tree = s[FOREST] * (0.3 + 0.7 * smooth(0.02, 0.08, tex));
                let bush = veg * smooth(0.015, 0.05, tex) * (1.0 - s[FOREST]);
                trees.as_mut().unwrap()[idx] = (tree.clamp(0.0, 1.0) * 255.0).round() as u8;
                bushes.as_mut().unwrap()[idx] = (bush.clamp(0.0, 1.0) * 255.0).round() as u8;
            } else {
                s[ROCK] = rock_dtm;
                s[SNOW] = snow_dtm * (1.0 - rock_dtm);
                s[DIRT] = smooth(p.dirt_slope_start, p.dirt_slope_full, slope)
                    * (1.0 - s[ROCK])
                    * (1.0 - s[SNOW]);
                s[GRASS] = (1.0 - s[ROCK] - s[SNOW] - s[DIRT]).max(0.0);
            }

            let sum: f32 = s.iter().sum();
            if sum < 1e-6 {
                s[GRASS] = 1.0;
            }
            for (k, layer) in scores.iter_mut().enumerate() {
                layer[idx] = (s[k].clamp(0.0, 1.0) * 255.0).round() as u8;
            }
        }
    }

    for layer in &mut scores {
        blur3(layer, size);
    }
    for layer in [&mut trees, &mut bushes].into_iter().flatten() {
        blur3(layer, size);
    }

    // Normalize so layers sum to 255 — shaders can blend directly.
    for idx in 0..size * size {
        let sum: u32 = scores.iter().map(|l| l[idx] as u32).sum::<u32>().max(1);
        for layer in scores.iter_mut() {
            layer[idx] = ((layer[idx] as u32 * 255) / sum).min(255) as u8;
        }
    }
    TileMasks { size, layers: scores, trees, bushes }
}

/// Local 3x3 luminance standard deviation, normalized to 0..1.
#[inline]
fn texture_at(lum: &[u8], size: usize, i: usize, j: usize) -> f32 {
    let mut sum = 0f32;
    let mut sq = 0f32;
    let mut cnt = 0f32;
    for di in -1i64..=1 {
        for dj in -1i64..=1 {
            let ii = i as i64 + di;
            let jj = j as i64 + dj;
            if ii >= 0 && jj >= 0 && ii < size as i64 && jj < size as i64 {
                let v = lum[(ii as usize) * size + jj as usize] as f32 / 255.0;
                sum += v;
                sq += v * v;
                cnt += 1.0;
            }
        }
    }
    let mean = sum / cnt;
    (sq / cnt - mean * mean).max(0.0).sqrt()
}

#[inline]
fn smooth(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Rises over [a,b], flat 1 over [b,c], falls over [c,d].
#[inline]
fn band(x: f32, a: f32, b: f32, c: f32, d: f32) -> f32 {
    smooth(a, b, x) * (1.0 - smooth(c, d, x))
}

/// In-place 3x3 box blur on u8, sliding three-row window — O(rows) extra
/// memory, not a full copy.
fn blur3(data: &mut [u8], size: usize) {
    let mut prev: Option<Vec<u8>> = None;
    let mut curr = data[0..size].to_vec();
    for i in 0..size {
        let next = (i + 1 < size).then(|| data[(i + 1) * size..(i + 2) * size].to_vec());
        for j in 0..size {
            let mut sum = 0u16;
            let mut cnt = 0u16;
            for row in [prev.as_ref(), Some(&curr), next.as_ref()].into_iter().flatten() {
                for dj in -1i64..=1 {
                    let jj = j as i64 + dj;
                    if jj >= 0 && jj < size as i64 {
                        sum += row[jj as usize] as u16;
                        cnt += 1;
                    }
                }
            }
            data[i * size + j] = (sum / cnt) as u8;
        }
        prev = Some(std::mem::take(&mut curr));
        curr = next.unwrap_or_default();
    }
}
