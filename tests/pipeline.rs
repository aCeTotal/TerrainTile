use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use terraintile::import::dataset;
use terraintile::pipeline::config::PipelineConfig;
use terraintile::pipeline::progress::Progress;
use terraintile::pipeline::runner;
use terraintile::tile::masks::MaskParams;

/// Continuous height field over world coordinates, so the seam between the
/// two input files is only visible if the pipeline breaks it.
fn height(x: f64, y: f64) -> f64 {
    100.0 + 20.0 * (x * 0.05).sin() + 20.0 * (y * 0.05).cos()
}

/// Arc/Info ASCII grid — GDAL reads it natively, no binary writer needed.
fn write_asc(path: &PathBuf, xll: f64, yll: f64, cols: usize, rows: usize) {
    let mut s = format!(
        "ncols {cols}\nnrows {rows}\nxllcorner {xll}\nyllcorner {yll}\ncellsize 1.0\nNODATA_value -9999\n"
    );
    for row in 0..rows {
        for col in 0..cols {
            let x = xll + col as f64 + 0.5;
            let y = yll + (rows - 1 - row) as f64 + 0.5;
            s.push_str(&format!("{:.3} ", height(x, y)));
        }
        s.push('\n');
    }
    std::fs::write(path, s).unwrap();
    // Sidecar with CRS so dataset::scan accepts the file.
    std::fs::write(
        path.with_extension("prj"),
        r#"PROJCS["ETRS89 / UTM zone 33N",GEOGCS["ETRS89",DATUM["European_Terrestrial_Reference_System_1989",SPHEROID["GRS 1980",6378137,298.257222101]],PRIMEM["Greenwich",0],UNIT["degree",0.0174532925199433]],PROJECTION["Transverse_Mercator"],PARAMETER["latitude_of_origin",0],PARAMETER["central_meridian",15],PARAMETER["scale_factor",0.9996],PARAMETER["false_easting",500000],PARAMETER["false_northing",0],UNIT["metre",1],AUTHORITY["EPSG","25833"]]"#,
    )
    .unwrap();
}

#[test]
fn full_pipeline_no_cracks() {
    let dir = std::env::temp_dir().join(format!("terraintile-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    // Two adjacent 128x128 px files, 1 m resolution.
    let a = dir.join("a.asc");
    let b = dir.join("b.asc");
    write_asc(&a, 600000.0, 6600000.0, 128, 128);
    write_asc(&b, 600128.0, 6600000.0, 128, 128);

    let info = dataset::scan(&[a.clone(), b.clone()]).unwrap();
    assert_eq!(info.width_px, 256);
    assert_eq!(info.height_px, 128);
    assert_eq!(info.resolution, 1.0);

    let out = dir.join("out");
    let cfg = PipelineConfig {
        output: out.clone(),
        tile_size_m: 32.0,
        overlap: true,
        lods: 3,
        threads: 2,
        nodata_height: 0.0,
        force: false,
        masks: MaskParams::default(),
        ortho: None,
    };
    let (tx, rx) = crossbeam_channel::unbounded();
    runner::run(cfg, vec![a, b], tx, Arc::new(AtomicBool::new(false)));

    let mut finished = None;
    for msg in rx.iter() {
        match msg {
            Progress::Error(e) => panic!("pipeline error: {e}"),
            Progress::Finished { tiles, failed, report, .. } => {
                finished = Some((tiles, failed, report));
            }
            _ => {}
        }
    }
    let (tiles, failed, report) = finished.expect("no Finished message");
    assert_eq!(tiles, 8 * 4);
    assert_eq!(failed, 0);
    assert!(
        report.ok(),
        "validation failed: missing={:?} meta={:?} edges={:?}",
        report.missing,
        report.meta_errors,
        report.edge_mismatches
    );

    // Spot-check one tile: mesh header and metadata.
    let tile = out.join("tiles/tile_x0_y0");
    let pos = terraintile::export::meshbin::read_positions(&tile.join("mesh_lod0.bin")).unwrap();
    assert_eq!(pos.len(), 33 * 33);
    let meta: serde_json::Value =
        serde_json::from_slice(&std::fs::read(tile.join("metadata.json")).unwrap()).unwrap();
    assert_eq!(meta["lods"].as_array().unwrap().len(), 3);
    assert_eq!(meta["neighbors"]["east"], "tile_x1_y0");
    assert!(out.join("quadtree.json").exists());
    assert!(out.join("dataset.json").exists());

    let _ = std::fs::remove_dir_all(&dir);
}

/// Small georeferenced GeoTIFF, like one file from a hoydedata.no export.
/// Columns in `nodata_cols` are written as nodata (-9999).
fn write_tif(path: &PathBuf, xll: f64, yll: f64, cols: usize, rows: usize, nodata_cols: std::ops::Range<usize>) {
    use gdal::spatial_ref::SpatialRef;
    let drv = gdal::DriverManager::get_driver_by_name("GTiff").unwrap();
    let mut ds = drv
        .create_with_band_type::<f32, _>(path, cols, rows, 1)
        .unwrap();
    ds.set_geo_transform(&[xll, 1.0, 0.0, yll + rows as f64, 0.0, -1.0]).unwrap();
    ds.set_spatial_ref(&SpatialRef::from_epsg(25833).unwrap()).unwrap();
    let mut data = Vec::with_capacity(cols * rows);
    for row in 0..rows {
        for col in 0..cols {
            let x = xll + col as f64 + 0.5;
            let y = yll + (rows - 1 - row) as f64 + 0.5;
            data.push(if nodata_cols.contains(&col) { -9999.0 } else { height(x, y) as f32 });
        }
    }
    let mut band = ds.rasterband(1).unwrap();
    band.set_no_data_value(Some(-9999.0)).unwrap();
    let mut buf = gdal::raster::Buffer::new((cols, rows), data);
    band.write((0, 0), (cols, rows), &mut buf).unwrap();
}

#[test]
fn zip_input_extracts_and_runs() {
    use std::io::Write;
    let dir = std::env::temp_dir().join(format!("terraintile-zip-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let a = dir.join("a.tif");
    let b = dir.join("b.tif");
    write_tif(&a, 600000.0, 6600000.0, 64, 64, 0..0);
    write_tif(&b, 600064.0, 6600000.0, 64, 64, 0..0);

    let zip_path = dir.join("dtm1.zip");
    let mut zw = zip::ZipWriter::new(std::fs::File::create(&zip_path).unwrap());
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    for (name, src) in [("dtm1/data/a.tif", &a), ("dtm1/data/b.tif", &b)] {
        zw.start_file(name, opts).unwrap();
        zw.write_all(&std::fs::read(src).unwrap()).unwrap();
    }
    zw.finish().unwrap();

    let out = dir.join("out");
    let cfg = PipelineConfig {
        output: out.clone(),
        tile_size_m: 32.0,
        overlap: true,
        lods: 2,
        threads: 2,
        nodata_height: 0.0,
        force: false,
        masks: MaskParams::default(),
        ortho: None,
    };
    let (tx, rx) = crossbeam_channel::unbounded();
    runner::run(cfg, vec![zip_path], tx, Arc::new(AtomicBool::new(false)));
    let report = rx
        .iter()
        .find_map(|m| match m {
            Progress::Error(e) => panic!("pipeline error: {e}"),
            Progress::Finished { report, .. } => Some(report),
            _ => None,
        })
        .unwrap();
    assert!(report.ok());
    assert!(out.join("source/a.tif").exists(), "zip skal pakkes ut til out/source");
    let ds: serde_json::Value =
        serde_json::from_slice(&std::fs::read(out.join("dataset.json")).unwrap()).unwrap();
    assert_eq!(ds["tiles_x"], 4);
    assert_eq!(ds["tiles_y"], 2);

    let _ = std::fs::remove_dir_all(&dir);
}

/// Nodata crossing a tile seam: fill must be window-independent so both
/// tiles produce bitwise identical edge heights (this was a real bug —
/// nearest-valid fill depended on the read window).
#[test]
fn nodata_at_seam_no_cracks() {
    let dir = std::env::temp_dir().join(format!("terraintile-nodata-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let a = dir.join("a.tif");
    // Nodata stripe over columns 28..36 — the tile seam at x=32 is inside it.
    write_tif(&a, 600000.0, 6600000.0, 64, 64, 28..36);

    let out = dir.join("out");
    let cfg = PipelineConfig {
        output: out.clone(),
        tile_size_m: 32.0,
        overlap: true,
        lods: 2,
        threads: 2,
        nodata_height: 0.0,
        force: false,
        masks: MaskParams::default(),
        ortho: None,
    };
    let (tx, rx) = crossbeam_channel::unbounded();
    runner::run(cfg, vec![a], tx, Arc::new(AtomicBool::new(false)));
    let report = rx
        .iter()
        .find_map(|m| match m {
            Progress::Error(e) => panic!("pipeline error: {e}"),
            Progress::Finished { report, .. } => Some(report),
            _ => None,
        })
        .unwrap();
    assert!(
        report.ok(),
        "nodata ved søm skal ikke gi sprekker: edges={:?}",
        report.edge_mismatches
    );
    let meta: serde_json::Value = serde_json::from_slice(
        &std::fs::read(out.join("tiles/tile_x0_y0/metadata.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(meta["had_nodata"], true);

    let _ = std::fs::remove_dir_all(&dir);
}

/// Changing only mask thresholds must rebuild masks but leave the meshes
/// untouched; an unchanged config must skip every tile.
#[test]
fn incremental_rebuilds_only_changes() {
    let dir = std::env::temp_dir().join(format!("terraintile-incr-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let a = dir.join("a.tif");
    write_tif(&a, 600000.0, 6600000.0, 64, 64, 0..0);

    let out = dir.join("out");
    let cfg = PipelineConfig {
        output: out.clone(),
        tile_size_m: 32.0,
        overlap: true,
        lods: 2,
        threads: 1,
        nodata_height: 0.0,
        force: false,
        masks: MaskParams::default(),
        ortho: None,
    };
    let run = |cfg: PipelineConfig| {
        let (tx, rx) = crossbeam_channel::unbounded();
        runner::run(cfg, vec![a.clone()], tx, Arc::new(AtomicBool::new(false)));
        rx.iter()
            .find_map(|m| match m {
                Progress::Error(e) => panic!("pipeline error: {e}"),
                Progress::Finished { skipped, report, .. } => Some((skipped, report)),
                _ => None,
            })
            .unwrap()
    };

    assert_eq!(run(cfg.clone()).0, 0, "first run builds everything");

    let mesh = out.join("tiles/tile_x0_y0/mesh_lod0.bin");
    let mask = out.join("tiles/tile_x0_y0/mask_grass.png");
    let mtime = |p: &std::path::Path| std::fs::metadata(p).unwrap().modified().unwrap();
    let (mesh_t1, mask_t1) = (mtime(&mesh), mtime(&mask));
    std::thread::sleep(std::time::Duration::from_millis(1100));

    // Only a mask threshold changes: masks rebuild, meshes do not.
    let mut cfg2 = cfg.clone();
    cfg2.masks.rock_slope_start = 40.0;
    let (skipped, report) = run(cfg2.clone());
    assert_eq!(skipped, 0, "mask change must reprocess tiles");
    assert!(report.ok());
    assert_eq!(mtime(&mesh), mesh_t1, "mesh skal ikke bygges på nytt");
    assert!(mtime(&mask) > mask_t1, "masker skal bygges på nytt");

    // Unchanged config: everything skipped.
    assert_eq!(run(cfg2).0, 4, "unchanged rerun must skip all tiles");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn resume_skips_done_tiles() {
    let dir = std::env::temp_dir().join(format!("terraintile-resume-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let a = dir.join("a.asc");
    write_asc(&a, 600000.0, 6600000.0, 64, 64);

    let cfg = PipelineConfig {
        output: dir.join("out"),
        tile_size_m: 32.0,
        overlap: true,
        lods: 2,
        threads: 1,
        nodata_height: 0.0,
        force: false,
        masks: MaskParams::default(),
        ortho: None,
    };

    let run = |cfg: PipelineConfig, inputs: Vec<PathBuf>| {
        let (tx, rx) = crossbeam_channel::unbounded();
        runner::run(cfg, inputs, tx, Arc::new(AtomicBool::new(false)));
        rx.iter()
            .find_map(|m| match m {
                Progress::Finished { skipped, .. } => Some(skipped),
                _ => None,
            })
            .unwrap()
    };
    assert_eq!(run(cfg.clone(), vec![a.clone()]), 0);
    assert_eq!(run(cfg, vec![a]), 4, "second run should skip all 4 tiles");

    let _ = std::fs::remove_dir_all(&dir);
}
