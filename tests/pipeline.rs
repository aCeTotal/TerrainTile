use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use terraintile::gen::world::WorldParams;
use terraintile::pipeline::config::PipelineConfig;
use terraintile::pipeline::progress::Progress;
use terraintile::pipeline::runner;
use terraintile::tile::classdef::default_classes;
use terraintile::validate::check::Report;

/// Coarse world so the whole run takes seconds: island ≈ 300 km² with a
/// 2 mil sea margin → 29x29 tiles of 64 px, most of them flat sea.
fn world() -> WorldParams {
    WorldParams {
        seed: 42,
        island_m: 17321.0,
        margin_m: 20_000.0,
        tile_size_m: 2048.0,
        resolution: 32.0,
        lods: 3,
    }
}

fn cfg(out: &Path, world: WorldParams) -> PipelineConfig {
    PipelineConfig {
        output: out.to_path_buf(),
        world,
        threads: 2,
        force: false,
        classes: default_classes(),
    }
}

/// Run to completion; panics on pipeline error.
fn run(cfg: PipelineConfig) -> (usize, usize, Report) {
    let (tx, rx) = crossbeam_channel::unbounded();
    runner::run(cfg, tx, Arc::new(AtomicBool::new(false)));
    rx.iter()
        .find_map(|m| match m {
            Progress::Error(e) => panic!("pipeline error: {e}"),
            Progress::Finished { skipped, failed, report, .. } => Some((skipped, failed, report)),
            _ => None,
        })
        .expect("no Finished message")
}

fn tmp(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("terraintile-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn full_generation_no_cracks() {
    let dir = tmp("gen");
    let out = dir.join("out");
    let (skipped, failed, report) = run(cfg(&out, world()));
    assert_eq!(skipped, 0);
    assert_eq!(failed, 0);
    assert!(
        report.ok(),
        "validation failed: missing={:?} meta={:?} edges={:?}",
        report.missing,
        report.meta_errors,
        report.edge_mismatches
    );
    assert_eq!(report.tiles_total, 28 * 28);

    // Corner tile is pure sea: flat, minimal quad mesh, no mask files.
    let sea = out.join("tiles/tile_x0_y0");
    let pos = terraintile::export::meshbin::read_positions(&sea.join("mesh_lod0.bin")).unwrap();
    assert_eq!(pos.len(), 4, "havflis skal være et flatt quad");
    let meta: serde_json::Value =
        serde_json::from_slice(&std::fs::read(sea.join("metadata.json")).unwrap()).unwrap();
    assert_eq!(meta["flat"], true);
    assert!(!sea.join("class.bin").exists(), "havflis skal ikke ha klassefiler");

    // Center tile is on the island: full mesh + class textures (grass).
    let land = out.join("tiles/tile_x14_y14");
    let pos = terraintile::export::meshbin::read_positions(&land.join("mesh_lod0.bin")).unwrap();
    assert_eq!(pos.len(), 65 * 65);
    let meta: serde_json::Value =
        serde_json::from_slice(&std::fs::read(land.join("metadata.json")).unwrap()).unwrap();
    assert_eq!(meta["flat"], false);
    assert_eq!(meta["lods"].as_array().unwrap().len(), 3);
    assert!(land.join("class.bin").exists());
    assert!(land.join("class_2.png").exists(), "gress-klassen skal finnes midt på øya");
    assert!(out.join("quadtree.json").exists());

    // Florida island: land above sea, nothing alpine, most tiles flat sea.
    let ds: serde_json::Value =
        serde_json::from_slice(&std::fs::read(out.join("dataset.json")).unwrap()).unwrap();
    assert_eq!(ds["tiles_x"], 28);
    assert_eq!(ds["min_height"].as_f64().unwrap(), 0.0);
    let max = ds["max_height"].as_f64().unwrap();
    assert!(max > 5.0 && max <= 60.0, "øyhøyde utenfor Florida-området: {max}");
    let flat_count = ds["flat_count"].as_u64().unwrap();
    assert!(flat_count > 400, "de fleste fliser skal være flatt hav: {flat_count}");

    let _ = std::fs::remove_dir_all(&dir);
}

/// Same seed → bitwise identical output; different seed → different terrain.
#[test]
fn generation_is_deterministic() {
    let dir = tmp("det");
    let small = WorldParams {
        seed: 7,
        island_m: 8000.0,
        margin_m: 20_000.0,
        tile_size_m: 2048.0,
        resolution: 32.0,
        lods: 2,
    };
    let (a, b, c) = (dir.join("a"), dir.join("b"), dir.join("c"));
    run(cfg(&a, small));
    run(cfg(&b, small));
    run(cfg(&c, WorldParams { seed: 8, ..small }));

    // Center tile sits on the island.
    let mesh = |root: &Path| std::fs::read(root.join("tiles/tile_x12_y12/mesh_lod0.bin")).unwrap();
    assert_eq!(mesh(&a), mesh(&b), "samme seed må gi bitvis likt terreng");
    assert_ne!(mesh(&a), mesh(&c), "ulik seed må gi ulikt terreng");

    let _ = std::fs::remove_dir_all(&dir);
}

/// Changing only mask thresholds must rebuild masks but leave the meshes
/// untouched; an unchanged config must skip every tile.
#[test]
fn incremental_rebuilds_only_changes() {
    let dir = tmp("incr");
    let out = dir.join("out");
    let small = WorldParams {
        seed: 3,
        island_m: 8000.0,
        margin_m: 20_000.0,
        tile_size_m: 2048.0,
        resolution: 32.0,
        lods: 2,
    };
    let total = (small.size_m() / small.tile_size_m).round() as usize;

    assert_eq!(run(cfg(&out, small)).0, 0, "first run builds everything");

    // Center tile is land.
    let mesh = out.join("tiles/tile_x12_y12/mesh_lod0.bin");
    let mask = out.join("tiles/tile_x12_y12/class.bin");
    let mtime = |p: &Path| std::fs::metadata(p).unwrap().modified().unwrap();
    let (mesh_t1, mask_t1) = (mtime(&mesh), mtime(&mask));
    std::thread::sleep(std::time::Duration::from_millis(1100));

    // Only a class gate changes: class textures rebuild, meshes do not.
    let mut cfg2 = cfg(&out, small);
    cfg2.classes[4].slope_min = Some(25.0);
    let (skipped, _, report) = run(cfg2.clone());
    assert_eq!(skipped, 0, "klasseendring må reprosessere fliser");
    assert!(report.ok());
    assert_eq!(mtime(&mesh), mesh_t1, "mesh skal ikke bygges på nytt");
    assert!(mtime(&mask) > mask_t1, "klasseteksturer skal bygges på nytt");

    // Unchanged config: everything skipped.
    assert_eq!(run(cfg2).0, total * total, "unchanged rerun must skip all tiles");

    let _ = std::fs::remove_dir_all(&dir);
}
