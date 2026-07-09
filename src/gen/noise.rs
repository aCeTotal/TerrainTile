//! Deterministic, seedable value noise. Every function is a pure function
//! of (seed, coordinates) — the same global coordinate always yields the
//! same value, which is what guarantees crack-free tile seams.

/// Integer lattice hash → [0, 1). FNV-1a-style mixing with a finalizer,
/// stable across runs and toolchains.
#[inline]
pub fn hash2(seed: u64, ix: i64, iy: i64) -> f32 {
    let mut h = 0xcbf29ce484222325u64 ^ seed;
    for b in ix.to_le_bytes().into_iter().chain(iy.to_le_bytes()) {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h ^= h >> 33;
    h = h.wrapping_mul(0xff51afd7ed558ccd);
    h ^= h >> 33;
    (h >> 40) as f32 / (1u64 << 24) as f32
}

/// Smoothly interpolated value noise in [-1, 1].
#[inline]
pub fn value(seed: u64, x: f64, y: f64) -> f32 {
    let x0 = x.floor();
    let y0 = y.floor();
    let (ix, iy) = (x0 as i64, y0 as i64);
    let fx = (x - x0) as f32;
    let fy = (y - y0) as f32;
    let sx = fx * fx * (3.0 - 2.0 * fx);
    let sy = fy * fy * (3.0 - 2.0 * fy);
    let v00 = hash2(seed, ix, iy);
    let v10 = hash2(seed, ix + 1, iy);
    let v01 = hash2(seed, ix, iy + 1);
    let v11 = hash2(seed, ix + 1, iy + 1);
    let a = v00 + (v10 - v00) * sx;
    let b = v01 + (v11 - v01) * sx;
    (a + (b - a) * sy) * 2.0 - 1.0
}

/// Fractal brownian motion in [-1, 1].
pub fn fbm(seed: u64, x: f64, y: f64, octaves: u32, lacunarity: f64, gain: f32) -> f32 {
    let mut sum = 0.0f32;
    let mut amp = 1.0f32;
    let mut norm = 0.0f32;
    let (mut fx, mut fy) = (x, y);
    for o in 0..octaves {
        sum += amp * value(seed.wrapping_add((o as u64).wrapping_mul(0x9e3779b97f4a7c15)), fx, fy);
        norm += amp;
        amp *= gain;
        fx *= lacunarity;
        fy *= lacunarity;
    }
    sum / norm
}

/// Ridged multifractal in [0, 1]: sharp crests where the noise crosses zero.
pub fn ridged(seed: u64, x: f64, y: f64, octaves: u32, lacunarity: f64, gain: f32) -> f32 {
    let mut sum = 0.0f32;
    let mut amp = 1.0f32;
    let mut norm = 0.0f32;
    let (mut fx, mut fy) = (x, y);
    for o in 0..octaves {
        let v =
            1.0 - value(seed.wrapping_add((o as u64).wrapping_mul(0x9e3779b97f4a7c15)), fx, fy).abs();
        sum += amp * v * v;
        norm += amp;
        amp *= gain;
        fx *= lacunarity;
        fy *= lacunarity;
    }
    sum / norm
}

/// Domain warp: offset coordinates by two independent FBM fields, so
/// everything sampled through the warped coordinates gets organic,
/// non-circular shapes.
pub fn warp(seed: u64, x: f64, y: f64, amp: f64, freq: f64) -> (f64, f64) {
    let dx = fbm(seed ^ 0x57a7c0a575, x * freq, y * freq, 4, 2.0, 0.5) as f64;
    let dy = fbm(seed ^ 0x9d2c5680dd, x * freq, y * freq, 4, 2.0, 0.5) as f64;
    (x + amp * dx, y + amp * dy)
}
