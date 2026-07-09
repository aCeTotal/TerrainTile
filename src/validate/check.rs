use std::path::Path;

use serde::Serialize;

use crate::export::meshbin;
use crate::tile::grid::{TileGrid, TileId};
use crate::tile::meta::TileMeta;

const MAX_ENTRIES: usize = 200;

#[derive(Clone, Debug, Default, Serialize)]
pub struct Report {
    pub tiles_total: usize,
    pub missing: Vec<String>,
    pub meta_errors: Vec<String>,
    pub edge_mismatches: Vec<String>,
    /// True if any list was truncated at MAX_ENTRIES.
    pub truncated: bool,
}

impl Report {
    pub fn ok(&self) -> bool {
        self.missing.is_empty() && self.meta_errors.is_empty() && self.edge_mismatches.is_empty()
    }

    fn push(list: &mut Vec<String>, truncated: &mut bool, msg: String) {
        if list.len() < MAX_ENTRIES {
            list.push(msg);
        } else {
            *truncated = true;
        }
    }
}

/// Verify the exported dataset: every tile has all files, metadata parses
/// and is complete, and (with overlap) neighboring tiles share bitwise
/// identical edge heights — the no-cracks guarantee.
pub fn run(out: &Path, grid: &TileGrid, lods: usize, overlap: bool) -> Report {
    let tiles_dir = out.join("tiles");
    let mut rep = Report { tiles_total: grid.count(), ..Default::default() };
    let mut truncated = false;

    for t in grid.tiles() {
        let dir = tiles_dir.join(t.name());
        // Read metadata first: flat sea tiles have minimal meshes and no
        // mask files by design.
        let mut is_flat = false;
        let meta_path = dir.join("metadata.json");
        match std::fs::read(&meta_path) {
            Err(_) => Report::push(&mut rep.missing, &mut truncated, format!("{}/metadata.json", t.name())),
            Ok(bytes) => match serde_json::from_slice::<TileMeta>(&bytes) {
                Err(e) => Report::push(
                    &mut rep.meta_errors,
                    &mut truncated,
                    format!("{}: {e}", t.name()),
                ),
                Ok(m) => {
                    is_flat = m.flat;
                    if m.lods.len() != lods {
                        Report::push(
                            &mut rep.meta_errors,
                            &mut truncated,
                            format!("{}: {} LOD-er i metadata, forventet {lods}", t.name(), m.lods.len()),
                        );
                    }
                }
            },
        }
        for k in 0..lods {
            let f = dir.join(format!("mesh_lod{k}.bin"));
            if !f.exists() {
                Report::push(&mut rep.missing, &mut truncated, format!("{}/mesh_lod{k}.bin", t.name()));
            }
        }
        if !is_flat && !dir.join("class.bin").exists() {
            Report::push(&mut rep.missing, &mut truncated, format!("{}/class.bin", t.name()));
        }
    }

    if overlap && rep.missing.is_empty() {
        check_edges(&tiles_dir, grid, &mut rep, &mut truncated);
    }
    rep.truncated = truncated;
    rep
}

/// Edge heights of one tile's LOD0 mesh.
struct Edges {
    east: Vec<f32>,
    south: Vec<f32>,
    west: Vec<f32>,
    north: Vec<f32>,
}

fn load_edges(tiles_dir: &Path, t: TileId) -> Option<Edges> {
    let pos = meshbin::read_positions(&tiles_dir.join(t.name()).join("mesh_lod0.bin")).ok()?;
    let vc = (pos.len() as f64).sqrt().round() as usize;
    if vc * vc != pos.len() {
        return None;
    }
    let h = |i: usize, j: usize| pos[i * vc + j][1];
    Some(Edges {
        east: (0..vc).map(|i| h(i, vc - 1)).collect(),
        south: (0..vc).map(|j| h(vc - 1, j)).collect(),
        west: (0..vc).map(|i| h(i, 0)).collect(),
        north: (0..vc).map(|j| h(0, j)).collect(),
    })
}

/// Walk the grid row-major; only one row of south edges plus the previous
/// tile's east edge are kept in memory.
fn check_edges(tiles_dir: &Path, grid: &TileGrid, rep: &mut Report, truncated: &mut bool) {
    let mut prev_row_south: Vec<Option<Vec<f32>>> = vec![None; grid.tiles_x];
    for y in 0..grid.tiles_y {
        let mut prev_east: Option<Vec<f32>> = None;
        for x in 0..grid.tiles_x {
            let t = TileId { x, y };
            let Some(e) = load_edges(tiles_dir, t) else {
                Report::push(rep.meta_errors.as_mut(), truncated, format!("{}: uleselig mesh_lod0.bin", t.name()));
                prev_east = None;
                prev_row_south[x] = None;
                continue;
            };
            // Flat sea tiles (2 edge vertices) are exactly 0 along every
            // edge, as is any neighbor's shared edge — skip the bitwise
            // comparison, the resolutions differ by design.
            let flat = |v: &Vec<f32>| v.len() == 2;
            if let Some(we) = &prev_east {
                if !flat(we) && !flat(&e.west) {
                    let diffs = count_diffs(we, &e.west);
                    if diffs > 0 {
                        Report::push(
                            &mut rep.edge_mismatches,
                            truncated,
                            format!("tile_x{}_y{y} øst ≠ {} vest ({diffs} avvik)", x - 1, t.name()),
                        );
                    }
                }
            }
            if let Some(ns) = &prev_row_south[x] {
                if !flat(ns) && !flat(&e.north) {
                    let diffs = count_diffs(ns, &e.north);
                    if diffs > 0 {
                        Report::push(
                            &mut rep.edge_mismatches,
                            truncated,
                            format!("tile_x{x}_y{} sør ≠ {} nord ({diffs} avvik)", y - 1, t.name()),
                        );
                    }
                }
            }
            prev_east = Some(e.east);
            prev_row_south[x] = Some(e.south);
        }
    }
}

fn count_diffs(a: &[f32], b: &[f32]) -> usize {
    if a.len() != b.len() {
        return a.len().max(b.len());
    }
    a.iter().zip(b).filter(|(x, y)| x.to_bits() != y.to_bits()).count()
}
