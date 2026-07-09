//! One giant Florida / Palm Beach-style island centered in the world:
//! mostly flat lowland with large usable plains, occasional terraced
//! plateaus, a realistic domain-warped coastline all around, and a huge,
//! gently sloping beach. Sea is exactly 0.0. Pure function of
//! (params, global coordinate) — the crack-free seam guarantee.
//!
//! Coordinates here are sample space: x east, y south from the world's NW
//! corner, in meters.

use crate::gen::noise;
use crate::gen::world::WorldParams;

/// Terrace operator: pulls heights toward flat steps of `step` meters.
/// `sharp` > 1 flattens the treads and steepens the risers — this is what
/// produces buildable plateaus.
pub fn terrace(h: f32, step: f32, sharp: f32) -> f32 {
    let t = h / step;
    let base = t.floor();
    let f = t - base;
    let s = (f - 0.5) * 2.0;
    let s = s.signum() * s.abs().powf(sharp);
    (base + 0.5 + s * 0.5) * step
}

/// Signed distance to the coastline in meters, > 0 inland (radial
/// approximation): a superellipse (rounded square) with bays and headlands
/// from angular noise. The caller passes domain-warped coordinates.
fn coast_d(w: &WorldParams, x: f64, y: f64) -> f64 {
    let (cx, cy) = w.center();
    let dx = x - cx;
    let dy = y - cy;
    let d = (dx * dx + dy * dy).sqrt();
    let a = w.island_m / 2.0;
    if d < 1.0 {
        return a;
    }
    let (c, s) = (dx / d, dy / d);
    // Superellipse exponent 4: square-ish landmass with rounded corners.
    let r_dir = a / (c.abs().powi(4) + s.abs().powi(4)).powf(0.25);
    // Bays and headlands, continuous around the unit circle.
    let rmod = 1.0 + 0.08 * noise::fbm(w.seed ^ 0xc0a5, c * 2.0, s * 2.0, 3, 2.0, 0.5) as f64;
    r_dir * rmod - d
}

pub fn height_at(w: &WorldParams, x_m: f64, y_m: f64) -> f32 {
    // Domain warp gives the coastline its organic shape.
    let (wx, wy) = noise::warp(w.seed, x_m, y_m, 2500.0, 1.0 / 18000.0);
    let d = coast_d(w, wx, wy);
    if d <= 0.0 {
        return 0.0;
    }
    let s = w.seed;
    // Palm Beach: a giant, gently sloping beach — 0 → 3 m over ~500 m.
    let beach = 3.0 * smooth01((d / 500.0) as f32);
    // Florida lowland: rolling plains 4–14 m, very low frequency = flat.
    let plains =
        4.0 + 10.0 * (0.5 + 0.5 * noise::fbm(s ^ 1, wx / 12000.0, wy / 12000.0, 4, 2.0, 0.5));
    // Occasional higher flat plateaus, hard-terraced (buildable treads).
    let pmask = smooth(0.55, 0.72, noise::fbm(s ^ 2, wx / 20000.0, wy / 20000.0, 3, 2.0, 0.5));
    let ph = 12.0 + 18.0 * (0.5 + 0.5 * noise::fbm(s ^ 3, wx / 9000.0, wy / 9000.0, 4, 2.0, 0.5));
    let plateau = terrace(ph, 8.0, 3.5) * pmask;
    // Interior ramps up over ~3 km from the coast.
    let inland = smooth01((d / 3000.0) as f32);
    (beach + inland * (plains + plateau)).max(0.0)
}

/// Conservative "this tile is certainly open sea" test for a sample-space
/// bbox (pad it with the apron before calling). Lets the pipeline mark
/// thousands of pure-sea tiles flat without sampling any noise: max island
/// reach = superellipse diagonal (1.19a) × angular modulation (1.08),
/// plus the domain warp's maximum displacement, with margin.
pub fn surely_sea(w: &WorldParams, x0: f64, y0: f64, x1: f64, y1: f64) -> bool {
    let (cx, cy) = w.center();
    let nx = cx.clamp(x0, x1);
    let ny = cy.clamp(y0, y1);
    let dmin = (nx - cx).hypot(ny - cy);
    dmin > w.island_m / 2.0 * 1.3 + 4000.0
}

#[inline]
fn smooth01(x: f32) -> f32 {
    let t = x.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[inline]
fn smooth(e0: f32, e1: f32, x: f32) -> f32 {
    smooth01((x - e0) / (e1 - e0))
}
