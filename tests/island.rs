use terraintile::gen::island;
use terraintile::gen::world::WorldParams;

fn world() -> WorldParams {
    WorldParams {
        seed: 42,
        island_m: 17321.0, // ≈ 300 km²
        margin_m: 20_000.0,
        tile_size_m: 2048.0,
        resolution: 32.0,
        lods: 3,
    }
}

/// Everything outside the island's maximum reach is exactly 0, and the
/// `surely_sea` early-out never claims sea where there is land.
#[test]
fn sea_margin_is_flat_and_surely_sea_never_lies() {
    let w = world();
    let (cx, cy) = w.center();
    let size = w.size_m();
    let step = 977.0; // irregular step → decorrelated from tiles
    let mut x = 0.0;
    while x < size {
        let mut y = 0.0;
        while y < size {
            let h = island::height_at(&w, x, y);
            if island::surely_sea(&w, x, y, x, y) {
                assert_eq!(h, 0.0, "surely_sea løy ved ({x:.0},{y:.0}): h={h}");
            }
            let d = (x - cx).hypot(y - cy);
            if d > w.island_m / 2.0 * 1.3 + 4000.0 {
                assert_eq!(h, 0.0, "hav ved ({x:.0},{y:.0}) skal være 0, fikk {h}");
            }
            y += step;
        }
        x += step;
    }
}

/// Walking inward from open sea along many directions, the coast shows
/// both wide, gentle beaches (0 < h < 3.5 m over ≥ 300 m) and sheer
/// cliffs (≥ 12 m within 100 m of the waterline).
#[test]
fn coast_has_beaches_and_cliffs() {
    let w = world();
    let (cx, cy) = w.center();
    let step = 20.0;
    let mut beaches = 0usize;
    let mut cliffs = 0usize;
    for k in 0..96 {
        let ang = k as f64 / 96.0 * std::f64::consts::TAU;
        let (ux, uy) = (ang.cos(), ang.sin());
        let mut coast = None; // distance-from-center of the first land sample
        let mut beach_m = 0.0f64;
        let mut cliff_here = false;
        let mut d = w.island_m; // start well outside
        while d > 0.0 {
            let h = island::height_at(&w, cx + ux * d, cy + uy * d);
            if h > 0.0 && coast.is_none() {
                coast = Some(d);
            }
            if let Some(c) = coast {
                let inland = c - d;
                if h > 0.0 && h < 3.5 {
                    beach_m += step;
                }
                if inland <= 100.0 && h >= 12.0 {
                    cliff_here = true;
                }
                if inland > 800.0 {
                    break;
                }
            }
            d -= step;
        }
        if beach_m >= 300.0 {
            beaches += 1;
        }
        if cliff_here {
            cliffs += 1;
        }
    }
    assert!(beaches >= 8, "bare {beaches}/96 retninger har bred strand");
    assert!(cliffs >= 8, "bare {cliffs}/96 retninger har klippekyst");
}

/// Rounded interior with no mountains: real height variation, but every
/// inland slope stays below a road-worthy grade.
#[test]
fn island_terrain_is_rounded_and_drivable() {
    let w = world();
    let (cx, cy) = w.center();
    let res = 8.0;
    let mut inland = 0usize;
    let mut hmax = 0.0f32;
    let mut gmax = 0.0f32;
    let r = w.island_m / 2.0 * 0.6; // safely inside the coast
    let mut x = cx - r;
    while x < cx + r {
        let mut y = cy - r;
        while y < cy + r {
            let h = island::height_at(&w, x, y);
            assert!(h <= 90.0, "fjell ({h} m) ved ({x:.0},{y:.0})");
            hmax = hmax.max(h);
            if h > 3.5 {
                inland += 1;
                let dx = island::height_at(&w, x + res, y) - island::height_at(&w, x - res, y);
                let dy = island::height_at(&w, x, y + res) - island::height_at(&w, x, y - res);
                let grade = (dx / (2.0 * res as f32)).hypot(dy / (2.0 * res as f32));
                if grade > gmax {
                    gmax = grade;
                    assert!(
                        grade <= 0.08,
                        "for bratt for bilveg ({:.1} %) ved ({x:.0},{y:.0})",
                        grade * 100.0
                    );
                }
            }
            y += 331.0;
        }
        x += 331.0;
    }
    assert!(inland > 500, "for få innlandssamples: {inland}");
    assert!(hmax > 25.0, "flatt terreng — makshøyde bare {hmax:.0} m");
}

/// Same params → bitwise identical; different seed → different island.
#[test]
fn deterministic() {
    let w = world();
    let w2 = WorldParams { seed: 43, ..w };
    let (cx, cy) = w.center();
    let mut diff = false;
    for i in 0..200 {
        let x = cx + (i as f64 - 100.0) * 61.0;
        let y = cy + (i as f64 - 100.0) * 37.0;
        let a = island::height_at(&w, x, y);
        let b = island::height_at(&w, x, y);
        assert_eq!(a.to_bits(), b.to_bits());
        if island::height_at(&w2, x, y).to_bits() != a.to_bits() {
            diff = true;
        }
    }
    assert!(diff, "ulik seed må gi ulikt terreng");
}
