use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use terraintile::edit::brush::{self, HeightStroke, Tool};
use terraintile::edit::cover::Cover;
use terraintile::edit::store::EditStore;
use terraintile::gen::heightfield::HeightSource;
use terraintile::gen::world::WorldParams;
use terraintile::pipeline::build;
use terraintile::pipeline::config::PipelineConfig;
use terraintile::pipeline::progress::Progress;
use terraintile::pipeline::runner;
use terraintile::tile::classdef::default_classes;
use terraintile::validate::check;

/// Island 8 km centered in a 49 152 m world → the world center (24 576 m)
/// is exactly the seam between tile 11 and 12, in the middle of the island.
fn world() -> WorldParams {
    WorldParams {
        seed: 9,
        island_m: 8000.0,
        margin_m: 20_000.0,
        tile_size_m: 2048.0,
        resolution: 32.0,
        lods: 2,
    }
}

const SEAM: f64 = 24576.0;

fn cfg(out: &Path) -> PipelineConfig {
    PipelineConfig {
        output: out.to_path_buf(),
        world: world(),
        threads: 2,
        force: false,
        classes: default_classes(),
    }
}

fn generate(cfg: &PipelineConfig) {
    let (tx, rx) = crossbeam_channel::unbounded();
    runner::run(cfg.clone(), tx, Arc::new(AtomicBool::new(false)));
    for msg in rx.iter() {
        match msg {
            Progress::Error(e) => panic!("pipeline error: {e}"),
            Progress::Finished { report, .. } => {
                assert!(report.ok(), "generering skal validere: {:?}", report.edge_mismatches);
                return;
            }
            _ => {}
        }
    }
    panic!("no Finished message");
}

/// class.bin → (size, idx bytes, weight bytes).
fn read_classbin(path: &Path) -> (usize, Vec<u8>, Vec<u8>) {
    let buf = std::fs::read(path).unwrap();
    assert_eq!(&buf[0..4], b"TTC1");
    let size = u32::from_le_bytes(buf[4..8].try_into().unwrap()) as usize;
    let block = size * size * 4;
    (size, buf[8..8 + block].to_vec(), buf[8 + block..8 + 2 * block].to_vec())
}

/// A sculpt stroke straight across a tile seam: after rebuilding the dirty
/// set, neighboring edges must still be bitwise identical, and the edit
/// must actually have changed the mesh.
#[test]
fn seam_stroke_no_cracks() {
    let dir = std::env::temp_dir().join(format!("terraintile-edit-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let out = dir.join("out");
    let cfg = cfg(&out);
    generate(&cfg);

    let grid = cfg.world.grid().unwrap();
    let src = HeightSource::new(cfg.world, Arc::new(EditStore::open(&out, &grid)));
    let cover = Cover::open(&out, &grid);
    let before = terraintile::export::meshbin::read_positions(
        &out.join("tiles/tile_x11_y11/mesh_lod0.bin"),
    )
    .unwrap();

    // Center exactly on the seam at the island's midpoint.
    let stroke = HeightStroke {
        tool: Tool::Raise,
        x: SEAM,
        z: SEAM - 1024.0,
        radius: 400.0,
        strength: 25.0,
        target_h: None,
    };
    let apron = 1usize << (cfg.world.lods - 1);
    let dirty = brush::apply_height(&src, &grid, apron, &[stroke]).unwrap();
    assert!(dirty.len() >= 2, "strøket berører begge sider av sømmen: {dirty:?}");

    let hashes = terraintile::pipeline::hash::compute(&cfg);
    let tiles_dir = out.join("tiles");
    for t in &dirty {
        build::build_tile(&cfg, &grid, &src, &cover, &tiles_dir, *t, &hashes).unwrap();
    }

    let report = check::run(&out, &grid, cfg.world.lods, true);
    assert!(report.ok(), "kantavvik etter sculpt: {:?}", report.edge_mismatches);

    let after =
        terraintile::export::meshbin::read_positions(&out.join("tiles/tile_x11_y11/mesh_lod0.bin"))
            .unwrap();
    assert_ne!(
        before.iter().map(|p| p[1].to_bits()).collect::<Vec<_>>(),
        after.iter().map(|p| p[1].to_bits()).collect::<Vec<_>>(),
        "sculpt skal endre høydene"
    );

    // A rerun of the full pipeline must keep the edit (delta fingerprints
    // in the tile hashes) and still validate.
    generate(&cfg);
    let kept =
        terraintile::export::meshbin::read_positions(&out.join("tiles/tile_x11_y11/mesh_lod0.bin"))
            .unwrap();
    assert_eq!(
        after.iter().map(|p| p[1].to_bits()).collect::<Vec<_>>(),
        kept.iter().map(|p| p[1].to_bits()).collect::<Vec<_>>(),
        "regenerering skal bevare sculpt-redigeringen"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// Paint a class polygon across a seam: both tiles get bitwise identical
/// class values along the shared edge, and meshes are untouched.
#[test]
fn class_paint_seam_consistent() {
    let dir = std::env::temp_dir().join(format!("terraintile-paint-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let out = dir.join("out");
    let cfg = cfg(&out);
    generate(&cfg);

    let grid = cfg.world.grid().unwrap();
    let src = HeightSource::new(cfg.world, Arc::new(EditStore::open(&out, &grid)));
    let cover = Cover::open(&out, &grid);
    let mesh_path = out.join("tiles/tile_x11_y11/mesh_lod0.bin");
    let mtime = |p: &Path| std::fs::metadata(p).unwrap().modified().unwrap();
    let mesh_t = mtime(&mesh_path);

    // Road-class square straddling the seam, in the middle of the island.
    let (cx, cz) = (SEAM, SEAM - 1024.0);
    let poly =
        [[cx - 300.0, cz - 300.0], [cx + 300.0, cz - 300.0], [cx + 300.0, cz + 300.0], [cx - 300.0, cz + 300.0]];
    let road = cfg.classes.iter().find(|c| c.road).unwrap().id;
    let dirty = cover.fill_polygon(road, &poly, false).unwrap();
    assert!(dirty.len() >= 2, "malingen berører begge sider av sømmen: {dirty:?}");

    let hashes = terraintile::pipeline::hash::compute(&cfg);
    for t in &dirty {
        build::build_tile(&cfg, &grid, &src, &cover, &out.join("tiles"), *t, &hashes).unwrap();
    }
    assert_eq!(mtime(&mesh_path), mesh_t, "maling skal ikke bygge mesh på nytt");

    // The painted class appears, and the shared edge column is identical
    // in both tiles (idx + weights, bitwise).
    let (size, idx_a, w_a) = read_classbin(&out.join("tiles/tile_x11_y11/class.bin"));
    let (_, idx_b, w_b) = read_classbin(&out.join("tiles/tile_x12_y11/class.bin"));
    let mid = size / 2;
    let east_a = (mid * size + (size - 1)) * 4;
    assert!(
        idx_a[east_a..east_a + 4].contains(&(road as u8))
            && w_a[east_a + idx_a[east_a..east_a + 4].iter().position(|v| *v == road as u8).unwrap()] > 60,
        "vei-klassen skal være malt ved sømmen"
    );
    let mut diffs = 0;
    for row in 0..size {
        let a = (row * size + (size - 1)) * 4; // tile A east column
        let b = (row * size) * 4; // tile B west column
        if idx_a[a..a + 4] != idx_b[b..b + 4] || w_a[a..a + 4] != w_b[b..b + 4] {
            diffs += 1;
        }
    }
    assert_eq!(diffs, 0, "klasseverdier langs sømmen skal være bitvis like");

    let _ = std::fs::remove_dir_all(&dir);
}
