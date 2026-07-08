// Parsers for the tile mesh formats served under /data/.

import * as THREE from 'three';

// Parse a TTM1 mesh file (positions/normals/uvs/tangents/indices) into a
// BufferGeometry; tangents are skipped.
export function parseTtm(buf) {
  const dv = new DataView(buf);
  if (dv.getUint32(0, true) !== 0x314d5454) throw new Error('ikke en TTM1-fil'); // "TTM1" LE
  const vc = dv.getUint32(4, true);
  const ic = dv.getUint32(8, true);
  let o = 12;
  const pos = new Float32Array(buf, o, vc * 3); o += vc * 12;
  const nrm = new Float32Array(buf, o, vc * 3); o += vc * 12;
  const uv = new Float32Array(buf, o, vc * 2); o += vc * 8;
  o += vc * 16; // tangents unused
  const idx = new Uint32Array(buf, o, ic);

  // Attributes are views into the fetched file; drop the CPU copy once the
  // GPU has it, or every visible tile pins ~5 MB of heap (tab crash).
  function freeArray() {
    this.array = null;
  }
  const geo = new THREE.BufferGeometry();
  geo.setAttribute('position', new THREE.BufferAttribute(pos, 3).onUpload(freeArray));
  geo.setAttribute('normal', new THREE.BufferAttribute(nrm, 3).onUpload(freeArray));
  geo.setAttribute('uv', new THREE.BufferAttribute(uv, 2).onUpload(freeArray));
  geo.setIndex(new THREE.BufferAttribute(idx, 1).onUpload(freeArray));
  geo.boundingSphere = new THREE.Sphere(); // computed from positions otherwise
  computeBounds(geo, pos);
  return geo;
}

// Bounding sphere from positions BEFORE the array is freed on upload —
// three would otherwise try to compute it from a nulled array.
function computeBounds(geo, pos) {
  const box = new THREE.Box3();
  const v = new THREE.Vector3();
  for (let i = 0; i < pos.length; i += 3) {
    v.set(pos[i], pos[i + 1], pos[i + 2]);
    box.expandByPoint(v);
  }
  box.getBoundingSphere(geo.boundingSphere);
}

// Triangle indices for a regular v×v vertex grid (two CCW triangles per
// quad, same winding as the pipeline). Shared by every far tile.
export function gridIndices(v) {
  const q = v - 1;
  const idx = new Uint32Array(q * q * 6);
  let o = 0;
  for (let i = 0; i < q; i++) {
    for (let j = 0; j < q; j++) {
      const a = i * v + j;
      idx[o++] = a; idx[o++] = a + v; idx[o++] = a + 1;
      idx[o++] = a + 1; idx[o++] = a + v; idx[o++] = a + v + 1;
    }
  }
  return idx;
}

// Parse the far.bin header. Returns null if the magic doesn't match.
export function parseFarHeader(buf) {
  const dv = new DataView(buf);
  if (dv.getUint32(0, true) !== 0x31465454) return null; // "TTF1" LE
  return { count: dv.getUint32(4, true), vertsPerEdge: dv.getUint32(8, true) };
}
