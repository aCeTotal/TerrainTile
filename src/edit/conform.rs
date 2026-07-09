//! "Tilpass terreng": re-apply every road spline with a wider feather and
//! re-snap all placements to the resulting terrain, so roads and props sit
//! naturally after sculpting and painting are done.

use std::collections::BTreeSet;
use std::path::Path;

use anyhow::Result;

use crate::edit::cover::Cover;
use crate::edit::spline;
use crate::gen::heightfield::HeightSource;
use crate::server::project::{self, Spline};
use crate::tile::grid::{TileGrid, TileId};

pub fn apply(
    output: &Path,
    src: &HeightSource,
    cover: &Cover,
    road_class: Option<u32>,
    grid: &TileGrid,
    apron_px: usize,
    splines: &[Spline],
) -> Result<BTreeSet<TileId>> {
    let mut dirty = BTreeSet::new();
    for s in splines {
        // Feather = full width (double the normal commit) → gentler
        // shoulders that blend the road into the terrain.
        dirty.extend(spline::apply(
            src,
            cover,
            road_class,
            grid,
            apron_px,
            s.width,
            s.width,
            &s.points,
        )?);
    }

    // Re-snap every placed mesh to the (possibly changed) ground.
    project::update(output, |p| {
        for pl in &mut p.placements {
            pl.pos[1] = src.height_at_m(pl.pos[0], pl.pos[2]) as f64;
        }
    })?;
    Ok(dirty)
}
