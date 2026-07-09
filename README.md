# TerrainTile

Procedural island-world editor with a web UI. The server generates one
giant, Florida/Palm Beach-style island — flat buildable lowland, terraced
plateaus, a realistic domain-warped coastline and a huge, gently sloping
beach — surrounded by an adjustable sea margin (default 5 mil = 50 km on
every side). The world is cut into a streamable tile dataset: crack-free
LOD meshes, per-tile class textures and a quadtree. Pure-sea tiles are
stored as tiny flat quads, so even a 1000 km² island with 50 km of ocean
stays manageable on disk.

The browser is both viewer and editor, with four modes (keys 1–4, Tab
toggles flying):

- **Sculpt** — raise / lower / flatten / smooth brushes on the terrain.
- **Texture** — user-defined **material classes**: create any number of
  classes, upload PBR materials to them (.zip or loose
  Color/Normal/Roughness maps, ambientCG-style), control each material's
  visibility/blend mode and the class's min/max terrain slope and height
  gates, then paint coverage with a freehand lasso on the aerial view.
  Adjacent classes blend softly by default; classes marked "sharp" (grass
  vs rock) get crisp edges.
- **Mesh** — upload GLB models, click-place with a move/rotate/scale gizmo
  (always snapped to the ground), click-out roads as smooth splines (the
  strip is flattened with a slope-limited profile and the road class
  painted under it), lasso **scatter areas** (density, spacing, random
  rotation/size — expanded deterministically, so Bevy regenerates the
  exact same forest), and a **Tilpass terreng** button that re-blends
  roads into the terrain and re-snaps every placed mesh.
- **Tomt og bygninger** — plots (numbered quads with individually
  draggable corners) and building zones typed as garasje / enebolig /
  blokk / skyskraper / bybygning / custom, with floors and rotation.
  Editor-only markings: players never see them; everything is exported to
  `plots.json` for Bevy and the game server (purchase/build logic).

Texture, Mesh and Tomt all share the **aerial view** ("flyfoto"): a
top-down orthographic view of the whole world with pan/zoom and freehand
lasso drawing (key F).

```sh
nix run                      # serves http://0.0.0.0:8080
nix run . -- --port 9000     # custom port
```

The frontend is embedded in the binary; the browser loads Three.js from a
CDN, so the *client* machine needs internet access for the 3D view.

## Workflow

Opening the page shows **Nytt prosjekt / Åpne prosjekt**. A new project
asks for island size (km²), sea margin (mil), tile size, resolution and a
seed, shows a live tile/disk estimate, then generates the world
(parallel, incremental, resumable — SSE progress in the browser). Fresh
projects get default classes (vann, sand, gress, jord, fjell, vei) with
the embedded PBR sets installed. Opening an existing folder restores
everything: world parameters, sculpted terrain, classes and painted
coverage, placed models, scatter areas, roads, plots and zones.

The viewer streams the world with a whole-terrain BatchedMesh far layer
plus ONE sea plane (flat tiles are never streamed); near tiles get
distance-based LOD, and the nearest ones a full PBR shader driven by
texture arrays (one layer per uploaded material, stochastic anti-tiling)
sampling the tile's top-4 class weights. The far/cheap layers follow the
painting through a server-baked `overview.png`. Adaptive quality steers
pixel ratio, shadows and rich-tile count toward a **10 ms frame budget**
(100 fps where the display allows; rAF is vsync-capped).

## How it stays seamless

Terrain height is a pure function of (seed, global sample coordinate):
domain-warped FBM shaped by a superellipse coast and a terrace operator.
Every edit lives in the same global-coordinate domain — `delta_h.bin`
(f32 height deltas with global sample ownership) and per-tile class
coverage PNGs on a global 8 m grid — so neighboring tiles always read
identical values along shared edges; class weights are composited over an
apron-padded grid and blurred BEFORE cropping. The validation pass proves
edge vertices bitwise identical. Sea level is exactly 0.

Edits go to the server (`/api/edit/height`, `/api/classes/paint`,
`/api/edit/spline`, `/api/edit/conform`), which updates the overlays,
rebuilds affected tiles in the background and pushes SSE events; the
client previews sculpting locally in the meantime. Incremental
fingerprints (world + class defs + per-tile overlay hashes, own and
neighboring) mean a rerun only rebuilds what actually changed.

## Output layout

```
out/
├── project.json          # world, classes, placements, splines, scatter,
│                         # plots, zones, zone_types
├── plots.json            # Bevy-facing plots/zones export
├── scatter.json          # deterministic scatter instances
├── dataset.json          # grid dims, height range, flat-tile bitset
├── quadtree.json
├── assets/               # uploaded GLB models
├── materials/<class>/<name>/{color,normal,rough}.png   # 1K PBR maps
├── classes/<class>/tile_x{X}_y{Y}.png                  # painted coverage
└── tiles/tile_x{X}_y{Y}/
    ├── mesh_lod0.bin … mesh_lod{N}.bin   # TTM1 (flat sea: 4-vertex quad)
    ├── class.bin                         # TTC1: top-4 class idx + weights
    ├── class_<id>.png                    # per-class gray weights (Bevy)
    ├── delta_h.bin                       # sculpt overlay (if edited)
    └── metadata.json
```

## Binary formats

`TTM1` mesh (little-endian, GPU-ready):

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

`TTC1` class splat:

```
magic  [u8;4] = "TTC1"
u32    size            vertex grid edge (n+1)
u8*4*size²  top-4 class indices per sample
u8*4*size²  matching weights, sum 255
```

## Using in Bevy

Spawn each tile at `Vec3::new(bbox.west, 0.0, world_size - bbox.north)`;
parse TTM1 as in earlier revisions of this README (positions/normals/uvs/
tangents/indices, all little-endian). Blend materials by sampling
`class_<id>.png` (or `class.bin`) against the class definitions in
`project.json`. `scatter.json` and `plots.json` are plain JSON with world
coordinates in meters.

## Memory

Designed for big worlds on small machines: heights are generated (and
composited with edit overlays) per tile window — no whole-world buffers
ever exist in RAM. Painted coverage is tiled and sparse (only painted
tiles have files) with an LRU cache. Pure-sea tiles cost bytes. Meshes
stream to disk without vertex buffers in RAM.
