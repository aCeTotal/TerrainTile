use std::sync::Arc;

use terraintile::edit::scatter::{instances, ScatterArea};
use terraintile::edit::store::EditStore;
use terraintile::gen::heightfield::HeightSource;
use terraintile::gen::world::WorldParams;

fn world() -> WorldParams {
    WorldParams {
        seed: 5,
        island_m: 8000.0,
        margin_m: 20_000.0,
        tile_size_m: 2048.0,
        resolution: 32.0,
        lods: 2,
    }
}

fn area(seed: u64) -> ScatterArea {
    // Square in the middle of the island (world center ≈ 24 576 m).
    let (c, r) = (24576.0, 900.0);
    ScatterArea {
        id: "a".into(),
        asset: "assets/tre.glb".into(),
        polygon: vec![[c - r, c - r], [c + r, c - r], [c + r, c + r], [c - r, c + r]],
        seed,
        density_ha: 150.0,
        min_spacing: 6.0,
        rot_random: true,
        scale_min: 0.8,
        scale_max: 1.3,
    }
}

#[test]
fn scatter_is_deterministic_and_valid() {
    let dir = std::env::temp_dir().join(format!("terraintile-scatter-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let w = world();
    let grid = w.grid().unwrap();
    let src = HeightSource::new(w, Arc::new(EditStore::open(&dir, &grid)));

    let a = instances(&area(7), &src);
    let b = instances(&area(7), &src);
    let c = instances(&area(8), &src);
    assert!(a.len() > 20, "for få instanser: {}", a.len());
    assert_eq!(a.len(), b.len(), "samme seed må gi samme antall");
    for (x, y) in a.iter().zip(&b) {
        assert_eq!(x.pos, y.pos);
        assert_eq!(x.rot_y, y.rot_y);
        assert_eq!(x.scale, y.scale);
    }
    assert_ne!(
        a.iter().map(|i| i.pos[0].to_bits()).collect::<Vec<_>>(),
        c.iter().map(|i| i.pos[0].to_bits()).collect::<Vec<_>>(),
        "ulik seed må gi ulik spredning"
    );

    // Inside the polygon, above the sea, respecting min spacing.
    let ar = area(7);
    for i in &a {
        assert!(i.pos[0] >= ar.polygon[0][0] && i.pos[0] <= ar.polygon[1][0]);
        assert!(i.pos[2] >= ar.polygon[0][1] && i.pos[2] <= ar.polygon[2][1]);
        assert!(i.pos[1] > 0.2, "instans i havet: {:?}", i.pos);
        assert!(i.scale >= 0.8 && i.scale <= 1.3);
    }
    for (n, i) in a.iter().enumerate() {
        for j in a.iter().skip(n + 1) {
            let d = (i.pos[0] - j.pos[0]).hypot(i.pos[2] - j.pos[2]);
            assert!(d >= ar.min_spacing * 0.1, "instanser oppå hverandre: {d}");
        }
    }

    let _ = std::fs::remove_dir_all(&dir);
}
