// Whole-terrain layer: every tile at coarse LOD in ONE BatchedMesh (single
// draw call), streamed from /data/far.bin. Instances under loaded near
// tiles are hidden so the detailed meshes never z-fight with the coarse
// ones.

import * as THREE from 'three';
import { gridIndices, parseFarHeader } from './ttm.js';
import { createSimpleTerrainMaterial, setOverview } from './terrain-simple.js';

let batched = null;
let material = null;
let dataset = null;
let sea = null; // one plane at y=0 covering the whole world
let instances = new Map(); // "x,y" -> instanceId
let hiddenWanted = new Set(); // near tiles that arrived before their far instance
let loadedCount = 0;
let totalCount = 0;

export function progress() {
  return { loaded: loadedCount, total: totalCount };
}

/// The sea plane, so editor raycasts can hit open water.
export function seaMeshes() {
  return sea ? [sea] : [];
}

export async function init(scene, ds, isTileCovered) {
  dataset = ds;
  material = createSimpleTerrainMaterial();

  // Flat sea tiles are not in far.bin and never stream — one huge plane
  // at exactly y=0 is the entire ocean (the simple material's water path
  // colors it and sinks it WATER_DROP).
  const W = ds.tiles_x * ds.tile_size_m;
  const H = ds.tiles_y * ds.tile_size_m;
  const seaGeo = new THREE.PlaneGeometry(W, H);
  seaGeo.rotateX(-Math.PI / 2);
  sea = new THREE.Mesh(seaGeo, createSimpleTerrainMaterial());
  sea.position.set(W / 2, 0, H / 2);
  scene.add(sea);

  loadOverview(W, H); // class colors for the far/cheap layers

  const res = await fetch('/data/far.bin');
  if (!res.ok) {
    console.warn('far.bin:', res.status);
    return;
  }
  const buf = await res.arrayBuffer();
  const head = parseFarHeader(buf);
  if (!head) {
    console.warn('far.bin: ukjent format');
    return;
  }
  const v = head.vertsPerEdge;
  totalCount = head.count;
  const vertsPerTile = v * v;
  const idxPerTile = (v - 1) * (v - 1) * 6;
  batched = new THREE.BatchedMesh(
    head.count,
    head.count * vertsPerTile,
    head.count * idxPerTile,
    material,
  );
  batched.receiveShadow = true;
  scene.add(batched);

  const sharedIdx = new THREE.BufferAttribute(gridIndices(v), 1);
  const T = dataset.tile_size_m;
  const W = dataset.tiles_x * T;
  const H = dataset.tiles_y * T;
  const recordSize = 8 + vertsPerTile * 24;

  // Parse + upload in slices so a huge dataset never freezes the tab.
  let off = 12;
  let sliceStart = performance.now();
  for (let k = 0; k < head.count; k++, off += recordSize) {
    const dv = new DataView(buf, off, 8);
    const tx = dv.getUint32(0, true);
    const ty = dv.getUint32(4, true);
    const src = new Float32Array(buf, off + 8, vertsPerTile * 3);
    const nrm = new Float32Array(buf, off + 8 + vertsPerTile * 12, vertsPerTile * 3);

    // Bake world offset into positions and global UVs over the dataset —
    // one shared overview texture covers every instance.
    const pos = new Float32Array(vertsPerTile * 3);
    const uv = new Float32Array(vertsPerTile * 2);
    const ox = tx * T;
    const oz = ty * T;
    for (let i = 0; i < vertsPerTile; i++) {
      const x = src[i * 3] + ox;
      const z = src[i * 3 + 2] + oz;
      pos[i * 3] = x;
      pos[i * 3 + 1] = src[i * 3 + 1];
      pos[i * 3 + 2] = z;
      uv[i * 2] = x / W;
      uv[i * 2 + 1] = z / H;
    }
    const geo = new THREE.BufferGeometry();
    geo.setAttribute('position', new THREE.BufferAttribute(pos, 3));
    geo.setAttribute('normal', new THREE.BufferAttribute(nrm, 3));
    geo.setAttribute('uv', new THREE.BufferAttribute(uv, 2));
    geo.setIndex(sharedIdx);

    const gid = batched.addGeometry(geo);
    const iid = batched.addInstance ? batched.addInstance(gid) : gid;
    const key = `${tx},${ty}`;
    instances.set(key, iid);
    if (hiddenWanted.has(key) || isTileCovered(tx, ty)) {
      batched.setVisibleAt(iid, false);
    }
    loadedCount++;

    if (performance.now() - sliceStart > 12) {
      await new Promise((r) => setTimeout(r, 0));
      sliceStart = performance.now();
    }
  }
}

// Bake of every tile's strongest class color, built server-side.
async function loadOverview(W, H) {
  try {
    const res = await fetch('/data/overview.png');
    if (!res.ok) return;
    const bitmap = await createImageBitmap(await res.blob());
    const tex = new THREE.Texture(bitmap);
    tex.flipY = false; // v=0 is the world's north edge = image top row
    tex.colorSpace = THREE.SRGBColorSpace;
    tex.needsUpdate = true;
    setOverview(tex, W, H);
  } catch {
    // procedural ramp remains the fallback
  }
}

// Hide/show the coarse instance under a near tile.
export function setTileCovered(x, y, covered) {
  const key = `${x},${y}`;
  if (covered) hiddenWanted.add(key);
  else hiddenWanted.delete(key);
  const iid = instances.get(key);
  if (batched && iid !== undefined) batched.setVisibleAt(iid, !covered);
}
