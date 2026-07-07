# TerrainTile

Headless terrain pipeline for Bevy with a web UI. Reads Norwegian height
data (GeoTIFF, e.g. hoydedata.no), fetches orthophotos (Norge i bilder WMS
or any XYZ server), and produces a streamable tile dataset: crack-free LOD
meshes, material masks, vegetation densities, metadata and a quadtree.

Runs as a server (e.g. on NixOS); everything is controlled from the
browser: configuration, live progress/log, and — when the job is done — a
3D viewer where you fly around the generated terrain (WASD + mouse,
distance-based LOD streaming straight from the tile dataset).

```sh
nix run                      # serves http://0.0.0.0:8080
nix run . -- --port 9000     # custom port
```

The frontend is embedded in the binary; the browser loads Three.js from a
CDN, so the *client* machine needs internet access for the 3D view.

## Web UI

- **Pipeline** — pick height data and output dir with the built-in
  server-side file browser, tune tiles/LOD/orthophoto/masks, start the job
  and watch stage messages, per-tile progress, rate/ETA and warnings live
  (SSE). Interrupted or reconfigured runs resume incrementally as before.
- **3D-visning** — renders the *entire* dataset: a far layer draws every
  tile at coarse LOD in a single `BatchedMesh` draw call (served
  pre-concatenated as `/data/far.bin`, subsampled server-side to a ~2M
  vertex budget) textured with a server-built overview mosaic of all
  orthophotos (`/data/overview.png`); near the camera, individual tiles
  stream in at distance-based LOD with full-res `ortho.png`, and the
  coarse instances underneath are hidden (no z-fighting). Click the canvas
  to fly: **WASD** move, **Q/E** down/up, **Shift** boost, mouse wheel
  adjusts speed.

## Pipeline

1. **Import** — pick a folder, files or a hoydedata.no **zip** directly.
   Zip metadata is scanned in place via GDAL `/vsizip/` (no extraction, no
   RAM); at run start the rasters are streamed out to `out/source/` once
   (resume-aware). CRS is auto-detected; all files must share CRS,
   resolution and sit on the same global pixel grid — checked up front so
   tiles are guaranteed to fit 100% at their seams. Files are mosaicked
   with a GDAL VRT; nothing is ever loaded fully into RAM (windowed reads
   per tile, streaming writes).
2. **Tiling** — choose tile size (256–2048 m) and LOD count. Tiles are cut
   on a fixed grid with a shared-edge apron: neighboring tiles sample the
   same source pixels, so edge vertices and normals are bitwise identical —
   no cracks, verified by the validation pass.
3. **Orthophoto** — per tile, the exact bbox is requested from Norge i
   bilder WMS in the dataset CRS (pixel centers land exactly on the vertex
   grid), or sampled bilinearly from an XYZ WebMercator server. Everything
   is disk-cached under `out/cache/`.
4. **Masks** — per-pixel classification combining photo (greenness,
   brightness, saturation, local texture) and DTM (slope, height):
   grass, forest, rock, dirt, sand, snow, water, road — normalized to sum
   255 so a shader can blend materials directly. Tree/bush densities are
   separate unnormalized layers for instancing.
5. **Incremental builds** — every tile records two fingerprints: one over
   everything its meshes depend on (source data, grid, LODs, nodata) and
   one for its masks (thresholds, ortho source). A rerun rebuilds only
   outputs whose inputs changed: new mask thresholds regenerate masks
   without touching meshes; nothing changed means nothing is rebuilt.
   `metadata.json` is written last (atomic rename), so an interrupted run
   resumes where it stopped.
6. **Validation** — all files present, metadata complete, and neighboring
   LOD0 edges bitwise equal.

## Output layout

```
out/
├── dataset.json          # CRS, origin, resolution, grid dims, height range
├── quadtree.json         # precomputed LOD quadtree over the tile grid
├── mosaic.vrt
└── tiles/tile_x{X}_y{Y}/
    ├── mesh_lod0.bin … mesh_lod{N}.bin
    ├── mask_grass.png … mask_road.png     # material weights, sum=255
    ├── mask_veg_trees.png, mask_veg_bushes.png
    ├── ortho.png
    └── metadata.json     # bbox, min/max height, avg slope/normal,
                          # center, neighbors, LOD + mask file lists
```

## mesh.bin format (`TTM1`)

Little-endian, GPU-ready:

```
magic   [u8;4] = "TTM1"
u32     vertex_count
u32     index_count
f32*3*N positions   x=east, y=up, z=south (m), origin = tile NW corner
f32*3*N normals
f32*2*N uvs         0..1 over the tile
f32*4*N tangents    xyzw
u32*M   indices     triangle list, CCW from above
```

## Using in Bevy

World placement from `metadata.json`: pick a scene origin `(E0, N0)` (e.g.
`dataset.json.origin`), then spawn each tile at
`Vec3::new(bbox.west - E0, 0.0, N0 - bbox.north)`.

```rust
fn load_ttm(bytes: &[u8]) -> Mesh {
    let vc = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize;
    let ic = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
    let f = |o: usize, n: usize| -> Vec<f32> {
        bytes[o..o + n * 4].chunks_exact(4)
            .map(|c| f32::from_le_bytes(c.try_into().unwrap())).collect()
    };
    let mut o = 12;
    let pos = f(o, vc * 3); o += vc * 12;
    let nrm = f(o, vc * 3); o += vc * 12;
    let uv  = f(o, vc * 2); o += vc * 8;
    let tan = f(o, vc * 4); o += vc * 16;
    let idx: Vec<u32> = bytes[o..o + ic * 4].chunks_exact(4)
        .map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect();

    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD);
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION,
        pos.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect::<Vec<_>>());
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL,
        nrm.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect::<Vec<_>>());
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0,
        uv.chunks_exact(2).map(|c| [c[0], c[1]]).collect::<Vec<_>>());
    mesh.insert_attribute(Mesh::ATTRIBUTE_TANGENT,
        tan.chunks_exact(4).map(|c| [c[0], c[1], c[2], c[3]]).collect::<Vec<_>>());
    mesh.insert_indices(Indices::U32(idx));
    mesh
}
```

Material blending: sample the mask PNGs in a custom shader and mix your PBR
materials by weight — geometry and masks stay fixed while materials remain
swappable.

## Norge i bilder

The WMS (`wms.geonorge.no/skwms1/wms.nib`) requires a BAAT ticket from
Geonorge. Paste the full URL including `LAYERS=ortofoto&ticket=...` in the
web UI. Without access, the XYZ fallback (ESRI World Imagery) works unauthenticated.

## Memory

Designed for big datasets on small machines: windowed VRT reads, meshes
streamed to disk without vertex buffers in RAM, u8 masks/ortho, row-wise
coordinate transforms. Per-worker RAM is bounded by tile size (~25 MB at
1024 m / 1 m); the thread slider caps workers.
