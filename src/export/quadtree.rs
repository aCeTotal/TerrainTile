use std::path::Path;

use anyhow::Result;
use serde::Serialize;

use crate::tile::grid::{TileGrid, TileId};

/// Precomputed quadtree over the tile grid. Leaves reference tiles; inner
/// nodes carry the LOD level to stream at that depth, so the engine never
/// computes this itself.
#[derive(Serialize)]
pub struct QNode {
    /// (west, south, east, north) in CRS units.
    pub bbox: [f64; 4],
    /// Suggested LOD when rendering this node as a whole.
    pub lod: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tile: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<QNode>,
}

pub fn build(grid: &TileGrid, lods: usize) -> QNode {
    let side = grid.tiles_x.max(grid.tiles_y).next_power_of_two();
    node(grid, lods, 0, 0, side).expect("root always covers at least one tile")
}

fn node(grid: &TileGrid, lods: usize, x0: usize, y0: usize, side: usize) -> Option<QNode> {
    if x0 >= grid.tiles_x || y0 >= grid.tiles_y {
        return None;
    }
    let x1 = (x0 + side).min(grid.tiles_x);
    let y1 = (y0 + side).min(grid.tiles_y);
    let nw = grid.bbox(TileId { x: x0, y: y0 });
    let se = grid.bbox(TileId { x: x1 - 1, y: y1 - 1 });
    let bbox = [nw.0, se.1, se.2, nw.3];
    let lod = (side.trailing_zeros() as usize).min(lods - 1);

    if side == 1 {
        return Some(QNode { bbox, lod: 0, tile: Some(TileId { x: x0, y: y0 }.name()), children: vec![] });
    }
    let half = side / 2;
    let children = [
        (x0, y0),
        (x0 + half, y0),
        (x0, y0 + half),
        (x0 + half, y0 + half),
    ]
    .into_iter()
    .filter_map(|(cx, cy)| node(grid, lods, cx, cy, half))
    .collect();
    Some(QNode { bbox, lod, tile: None, children })
}

pub fn write(path: &Path, root: &QNode) -> Result<()> {
    std::fs::write(path, serde_json::to_vec_pretty(root)?)?;
    Ok(())
}
