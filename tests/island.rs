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

/// A transect from open sea to the island center crosses a wide, gentle
/// beach (0 < h < 3.5 m over ≥ 300 m) and reaches real land.
#[test]
fn beach_band_is_wide() {
    let w = world();
    let (cx, cy) = w.center();
    let mut beach_m = 0.0f64;
    let mut max_h = 0.0f32;
    let step = 25.0;
    let mut d = w.island_m; // start well outside
    while d > 0.0 {
        let h = island::height_at(&w, cx + d, cy);
        if h > 0.0 && h < 3.5 {
            beach_m += step;
        }
        max_h = max_h.max(h);
        d -= step;
    }
    assert!(beach_m >= 300.0, "strandbåndet er bare {beach_m:.0} m bredt");
    assert!(max_h > 5.0, "transektet nådde aldri innlandet (max {max_h})");
}

/// Florida: mostly flat — at least half of all inland samples have < 2°
/// slope, and nothing exceeds 60 m.
#[test]
fn island_is_mostly_flat_lowland() {
    let w = world();
    let (cx, cy) = w.center();
    let res = 8.0;
    let mut flatish = 0usize;
    let mut inland = 0usize;
    let r = w.island_m / 2.0 * 0.6; // safely inside the coast
    let mut x = cx - r;
    while x < cx + r {
        let mut y = cy - r;
        while y < cy + r {
            let h = island::height_at(&w, x, y);
            assert!(h <= 60.0, "for høyt ({h} m) ved ({x:.0},{y:.0})");
            if h > 3.5 {
                inland += 1;
                let dx = island::height_at(&w, x + res, y) - island::height_at(&w, x - res, y);
                let dy = island::height_at(&w, x, y + res) - island::height_at(&w, x, y - res);
                let slope = ((dx / (2.0 * res as f32)).hypot(dy / (2.0 * res as f32)))
                    .atan()
                    .to_degrees();
                if slope < 2.0 {
                    flatish += 1;
                }
            }
            y += 331.0;
        }
        x += 331.0;
    }
    assert!(inland > 500, "for få innlandssamples: {inland}");
    assert!(
        flatish * 2 >= inland,
        "bare {flatish}/{inland} innlandssamples er flate (<2°)"
    );
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
