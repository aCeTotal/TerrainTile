// Near layer: individual tile meshes with distance-based LOD and full-res
// orthophoto, streamed around the camera. Tells the far layer which tiles
// it covers so the coarse instances underneath are hidden.

import * as THREE from 'three';
import { parseTtm } from './ttm.js';
import { createTerrainMaterial } from './terrain-material.js';

const MAX_INFLIGHT = 6;

// Adjusted by the viewer's adaptive-quality controller (60 fps target).
let maxTiles = 200;
let lodScale = 3;

export function setQuality(tiles, scale) {
  maxTiles = tiles;
  lodScale = scale;
}

let scene = null;
let dataset = null;
let setCovered = null; // far.setTileCovered
let terrainMat = null; // shared by every untextured tile
let inFlight = 0;
const tiles = new Map(); // "x,y" -> entry
const covered = new Set(); // keys with a visible near mesh

export function init(sc, ds, setTileCovered) {
  scene = sc;
  dataset = ds;
  setCovered = setTileCovered;
  terrainMat = createTerrainMaterial();
}

export function isCovered(x, y) {
  return covered.has(`${x},${y}`);
}

export function stats() {
  let meshes = 0;
  for (const t of tiles.values()) if (t.mesh) meshes++;
  return { meshes, inFlight };
}

function tileUrl(x, y, file) {
  return `/data/tiles/tile_x${x}_y${y}/${file}`;
}

function desiredLod(dist) {
  const t = dataset.tile_size_m;
  // The closest ring is always full resolution, whatever the quality level.
  if (dist < 2 * t) return 0;
  const base = t * lodScale;
  const lod = Math.round(Math.log2(Math.max(dist, base) / base));
  return Math.max(0, Math.min(dataset.lods - 1, lod));
}

// Only switch LOD once the boundary is crossed with ~10% margin, so a tile
// sitting on a threshold doesn't reload every update.
function wantLod(entry, dist) {
  const want = desiredLod(dist);
  if (entry.lod < 0 || want === entry.lod) return want;
  const margin = want > entry.lod ? desiredLod(dist * 0.9) : desiredLod(dist * 1.1);
  return margin === want ? want : entry.lod;
}

async function loadTexture(x, y) {
  try {
    const res = await fetch(tileUrl(x, y, 'ortho.png'));
    if (!res.ok) return null;
    const bitmap = await createImageBitmap(await res.blob());
    const tex = new THREE.Texture(bitmap);
    tex.flipY = false; // uv v=0 is the tile's north edge = image top row
    tex.colorSpace = THREE.SRGBColorSpace;
    tex.anisotropy = 4;
    tex.needsUpdate = true;
    return tex;
  } catch {
    return null;
  }
}

async function loadLod(entry, x, y, lod) {
  entry.loading = true;
  inFlight++;
  try {
    if (entry.texture === undefined) entry.texture = await loadTexture(x, y);
    const res = await fetch(tileUrl(x, y, `mesh_lod${lod}.bin`));
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    const geo = parseTtm(await res.arrayBuffer());

    const material = entry.texture
      ? new THREE.MeshStandardMaterial({ map: entry.texture, roughness: 1.0, metalness: 0.0 })
      : terrainMat;
    const mesh = new THREE.Mesh(geo, material);
    mesh.position.set(x * dataset.tile_size_m, 0, y * dataset.tile_size_m);
    mesh.castShadow = true;
    mesh.receiveShadow = true;

    if (entry.mesh) {
      scene.remove(entry.mesh);
      entry.mesh.geometry.dispose();
      if (entry.mesh.material !== terrainMat) entry.mesh.material.dispose();
    }
    entry.mesh = mesh;
    entry.lod = lod;
    scene.add(mesh);
    covered.add(`${x},${y}`);
    setCovered(x, y, true);
  } catch (err) {
    console.warn(`tile_x${x}_y${y} lod${lod}:`, err);
    // Retry with backoff — a tile that failed once (server restart, resume
    // in progress) must not stay a hole for the rest of the session.
    entry.failed = true;
    entry.retryAt = performance.now() + Math.min(60000, 5000 * ++entry.fails);
  } finally {
    entry.loading = false;
    inFlight--;
  }
}

function unload(key, entry) {
  if (entry.mesh) {
    scene.remove(entry.mesh);
    entry.mesh.geometry.dispose();
    if (entry.mesh.material !== terrainMat) entry.mesh.material.dispose();
  }
  if (entry.texture) entry.texture.dispose();
  tiles.delete(key);
  covered.delete(key);
  const [x, y] = key.split(',').map(Number);
  setCovered(x, y, false);
}

// Called every tick: stream the nearest tiles at the right LOD, drop the
// rest. The far layer covers everything beyond nearDist.
export function update(camera, nearDist) {
  const t = dataset.tile_size_m;
  const cx = camera.position.x;
  const cz = camera.position.z;
  const r = Math.ceil(nearDist / t);
  const tx = Math.floor(cx / t);
  const ty = Math.floor(cz / t);

  const wanted = [];
  for (let y = Math.max(0, ty - r); y <= Math.min(dataset.tiles_y - 1, ty + r); y++) {
    for (let x = Math.max(0, tx - r); x <= Math.min(dataset.tiles_x - 1, tx + r); x++) {
      const dist = Math.hypot((x + 0.5) * t - cx, (y + 0.5) * t - cz);
      if (dist < nearDist) wanted.push([dist, x, y]);
    }
  }
  wanted.sort((a, b) => a[0] - b[0]);
  if (wanted.length > maxTiles) wanted.length = maxTiles;

  const keep = new Set();
  for (const [dist, x, y] of wanted) {
    const key = `${x},${y}`;
    keep.add(key);
    let entry = tiles.get(key);
    if (!entry) {
      entry = { mesh: null, lod: -1, loading: false, texture: undefined, failed: false, fails: 0 };
      tiles.set(key, entry);
    }
    // Only nearby tiles render into the shadow map — distant ones cost a
    // full extra geometry pass without contributing visible shadows.
    if (entry.mesh) entry.mesh.castShadow = dist < 1200;
    if (entry.failed && performance.now() >= entry.retryAt) entry.failed = false;
    if (entry.failed || entry.loading || inFlight >= MAX_INFLIGHT) continue;
    const want = wantLod(entry, dist);
    if (want !== entry.lod) loadLod(entry, x, y, want);
  }

  for (const [key, entry] of tiles) {
    if (!keep.has(key) && !entry.loading) unload(key, entry);
  }
}
