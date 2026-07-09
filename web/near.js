// Near layer: individual tile meshes with distance-based LOD, streamed
// around the camera. Tells the far layer which tiles it covers so the
// coarse instances underneath are hidden.

import * as THREE from 'three';
import { parseTtm } from './ttm.js';
import { createTerrainMaterial } from './terrain-material.js';
import { createSimpleTerrainMaterial } from './terrain-simple.js';

const MAX_INFLIGHT = 6;
const MAX_TILES = 200;

let scene = null;
let dataset = null;
let setCovered = null; // far.setTileCovered
let fallbackRich = null; // PBR without class textures, if class.bin is missing
let cheapMat = null; // procedural ramp — everything beyond the rich ring
let richCount = 2; // quality-driven via setRichCount
let inFlight = 0;
const tiles = new Map(); // "x,y" -> entry
const covered = new Set(); // keys with a visible near mesh
const versions = new Map(); // "x,y" -> cache-buster after server rebuilds
const flat = new Set(); // pure-sea tiles: never streamed, sea plane covers them

export function init(sc, ds, setTileCovered) {
  scene = sc;
  dataset = ds;
  setCovered = setTileCovered;
  fallbackRich = createTerrainMaterial();
  cheapMat = createSimpleTerrainMaterial();
  // Row-major hex bitset from dataset.json.
  if (ds.flat_tiles) {
    for (let i = 0; i < ds.tiles_x * ds.tiles_y; i++) {
      const byte = parseInt(ds.flat_tiles.substr((i >> 3) * 2, 2), 16);
      if (byte & (1 << (i % 8))) flat.add(`${i % ds.tiles_x},${Math.floor(i / ds.tiles_x)}`);
    }
  }
}

/// How many of the nearest tiles get the full PBR material with their own
/// class textures. Steered by the viewer's adaptive quality.
export function setRichCount(k) {
  richCount = Math.max(1, Math.min(9, k));
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
  const v = versions.get(`${x},${y}`);
  return `/data/tiles/tile_x${x}_y${y}/${file}` + (v ? `?v=${v}` : '');
}

// Raw class.bin (TTC1) → two DataTextures: top-4 class indices (NEAREST —
// ids must never interpolate) and weights (linear).
async function loadClassTex(x, y) {
  const res = await fetch(tileUrl(x, y, 'class.bin'));
  if (!res.ok) return null;
  const buf = new Uint8Array(await res.arrayBuffer());
  if (buf.length < 8 || buf[0] !== 0x54 || buf[1] !== 0x54 || buf[2] !== 0x43) return null;
  const size = new DataView(buf.buffer).getUint32(4, true);
  const block = size * size * 4;
  if (buf.length < 8 + block * 2) return null;
  const mk = (arr, nearest) => {
    const t = new THREE.DataTexture(arr, size, size, THREE.RGBAFormat);
    t.minFilter = t.magFilter = nearest ? THREE.NearestFilter : THREE.LinearFilter;
    t.wrapS = t.wrapT = THREE.ClampToEdgeWrapping;
    t.needsUpdate = true;
    return t;
  };
  return { idx: mk(buf.slice(8, 8 + block), true), w: mk(buf.slice(8 + block), false) };
}

// Give a tile the full PBR material driven by its own class textures.
async function promote(entry, x, y) {
  entry.promoting = true;
  try {
    const ct = await loadClassTex(x, y);
    if (ct) {
      entry.classTex = ct;
      entry.rich = createTerrainMaterial({ classIdx: ct.idx, classW: ct.w });
    } else {
      entry.rich = fallbackRich;
    }
    if (entry.mesh && entry.wantRich) entry.mesh.material = entry.rich;
  } finally {
    entry.promoting = false;
  }
}

function demote(entry) {
  if (entry.mesh) entry.mesh.material = cheapMat;
  if (entry.rich && entry.rich !== fallbackRich) entry.rich.dispose();
  if (entry.classTex) {
    entry.classTex.idx.dispose();
    entry.classTex.w.dispose();
  }
  entry.rich = null;
  entry.classTex = null;
}

// Exactly ONE tile — the one closest to the camera — gets LOD0 and the
// full PBR material. Everything else is cheap: coarse LOD by distance,
// correct silhouette, procedural colors.
function desiredLod(dist, hero) {
  if (hero) return 0;
  const t = dataset.tile_size_m;
  const lod = Math.max(1, Math.round(Math.log2(Math.max(dist, t) / t)));
  return Math.min(dataset.lods - 1, lod);
}

// Only switch LOD once the boundary is crossed with ~10% margin, so a tile
// sitting on a threshold doesn't reload every update.
function wantLod(entry, dist, hero) {
  const want = desiredLod(dist, hero);
  if (hero || entry.lod < 0 || want === entry.lod) return want;
  const margin =
    want > entry.lod ? desiredLod(dist * 0.9, false) : desiredLod(dist * 1.1, false);
  return margin === want ? want : entry.lod;
}

async function loadLod(entry, x, y, lod) {
  entry.loading = true;
  inFlight++;
  try {
    const res = await fetch(tileUrl(x, y, `mesh_lod${lod}.bin`));
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    const geo = parseTtm(await res.arrayBuffer());

    // The nearest tiles are promoted to rich materials by update().
    const mesh = new THREE.Mesh(geo, entry.rich || cheapMat);
    mesh.position.set(x * dataset.tile_size_m, 0, y * dataset.tile_size_m);
    mesh.receiveShadow = true;

    if (entry.mesh) {
      scene.remove(entry.mesh);
      entry.mesh.geometry.dispose();
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
  }
  demote(entry);
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
      if (flat.has(`${x},${y}`)) continue; // sea plane covers it
      const dist = Math.hypot((x + 0.5) * t - cx, (y + 0.5) * t - cz);
      if (dist < nearDist) wanted.push([dist, x, y]);
    }
  }
  wanted.sort((a, b) => a[0] - b[0]);
  if (wanted.length > MAX_TILES) wanted.length = MAX_TILES;

  const keep = new Set();
  for (let i = 0; i < wanted.length; i++) {
    const [dist, x, y] = wanted[i];
    const hero = i === 0;
    const key = `${x},${y}`;
    keep.add(key);
    let entry = tiles.get(key);
    if (!entry) {
      entry = {
        mesh: null, lod: -1, loading: false, failed: false, fails: 0,
        rich: null, classTex: null, promoting: false, wantRich: false,
      };
      tiles.set(key, entry);
    }
    // The nearest richCount tiles carry the expensive PBR material with
    // their own class textures; only the hero casts shadows.
    entry.wantRich = i < richCount;
    if (entry.wantRich && !entry.rich && !entry.promoting) promote(entry, x, y);
    if (!entry.wantRich && entry.rich) demote(entry);
    if (entry.mesh) {
      entry.mesh.material = entry.wantRich && entry.rich ? entry.rich : cheapMat;
      entry.mesh.castShadow = hero;
    }
    if (entry.failed && performance.now() >= entry.retryAt) entry.failed = false;
    if (entry.failed || entry.loading || inFlight >= MAX_INFLIGHT) continue;
    const want = wantLod(entry, dist, hero);
    if (want !== entry.lod) loadLod(entry, x, y, want);
  }

  for (const [key, entry] of tiles) {
    if (!keep.has(key) && !entry.loading) unload(key, entry);
  }
}

/* ---------- editor hooks ---------- */

/// Server rebuilt these tiles: bust the cache and reload mesh + classes at
/// the current LOD on the next update tick.
export function refresh(names) {
  const now = Date.now();
  for (const name of names) {
    const m = /^tile_x(\d+)_y(\d+)$/.exec(name);
    if (!m) continue;
    const key = `${m[1]},${m[2]}`;
    versions.set(key, now);
    flat.delete(key); // a rebuilt sea tile may have been sculpted into land
    const entry = tiles.get(key);
    if (entry) {
      entry.lod = -1; // force mesh reload
      if (entry.rich) demote(entry); // re-promote reloads the class textures
    }
  }
}

/// Loaded meshes, for the editor's raycaster.
export function allMeshes() {
  const out = [];
  for (const t of tiles.values()) if (t.mesh) out.push(t.mesh);
  return out;
}

/// Loaded meshes whose tile intersects the circle, for sculpt preview.
export function meshesInRadius(x, z, r) {
  const t = dataset.tile_size_m;
  const out = [];
  for (const [key, entry] of tiles) {
    if (!entry.mesh) continue;
    const [tx, ty] = key.split(',').map(Number);
    if (x + r < tx * t || x - r > (tx + 1) * t) continue;
    if (z + r < ty * t || z - r > (ty + 1) * t) continue;
    out.push({ tileX: tx, tileY: ty, mesh: entry.mesh });
  }
  return out;
}

