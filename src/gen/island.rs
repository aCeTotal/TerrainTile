//! One giant island centered in the world with rounded, rolling relief —
//! no mountains, smooth transitions, a flattened tread on top of every
//! rise, and every inland slope gentle enough to carry a road straight up
//! it. The domain-warped coastline meets the sea as a wide beach where the
//! terrain is low and as a sheer cliff where it is high; craggy rock
//! outcrops line the cliff edges.
//! Sea is exactly 0.0. Pure function of (params, global coordinate) — the
//! crack-free seam guarantee.
//!
//! Coordinates here are sample space: x east, y south from the world's NW
//! corner, in meters.

use crate::gen::noise;
use crate::gen::world::WorldParams;

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

    // Interior relief: rounded, low-frequency fields only. Amplitudes and
    // wavelengths are chosen so the combined inland gradient stays below a
    // road-worthy grade (~8 %) even through the domain warp. Every field is
    // pushed through smooth01, whose zero derivative at both ends gives
    // smooth valley floors and a flattened tread on top of every rise.
    let plains =
        5.0 + 10.0 * (0.5 + 0.5 * noise::fbm(s ^ 1, wx / 9000.0, wy / 9000.0, 3, 2.0, 0.5));
    let hills =
        26.0 * smooth01((0.5 + 0.5 * noise::fbm(s ^ 6, wx / 3500.0, wy / 3500.0, 4, 2.0, 0.5)) / 0.8);
    // Broad uplands: gentle swells that lift whole sectors of the island.
    let swell = 24.0 * smooth(0.0, 0.6, noise::fbm(s ^ 2, wx / 15000.0, wy / 15000.0, 3, 2.0, 0.5));
    let interior = plains + hills + swell;

    // The coast profile is decided by the terrain height itself: low ground
    // ramps into the sea as a wide beach, high ground is cut off as a sheer
    // cliff dropping straight to the water.
    let cliffy = smooth(20.0, 30.0, interior);
    let ramp_beach = smooth01((d / 2000.0) as f32);
    let ramp_cliff = smooth01((d / 45.0) as f32);
    let ramp = ramp_beach + (ramp_cliff - ramp_beach) * cliffy;

    // Rock outcrops along the cliff edges: ridged noise in a narrow band
    // behind the wall gives sharp, craggy shapes that read as bare rock
    // under a stone texture. Zero in beach sectors and inland, and faded
    // to zero at the waterline itself so the water's edge stays smooth.
    let rock_zone =
        cliffy * smooth01((d / 60.0) as f32) * (1.0 - smooth01(((d - 100.0) / 200.0) as f32));
    let rock = 12.0 * rock_zone * noise::ridged(s ^ 5, wx / 150.0, wy / 150.0, 4, 2.0, 0.5);

    (interior * ramp + rock).max(0.0)
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
