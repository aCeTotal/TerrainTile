// Road tool: click to place control points, live Catmull-Rom preview,
// Enter/double-click commits (the server flattens a slope-limited strip
// and paints the road class), Backspace removes the last point, Esc
// cancels. Committed roads render as terrain-draped asphalt ribbons.

import * as THREE from 'three';
import * as ground from './ground.js';

let scene = null;
let roadsGroup = null;
let previewLine = null;
let points = null; // Vector3[] while placing
let splines = []; // committed, from project.json + this session
let redrapeTimer = null;

const asphalt = new THREE.MeshStandardMaterial({
  color: 0x2e2e30,
  roughness: 0.95,
  metalness: 0.0,
  polygonOffset: true,
  polygonOffsetFactor: -2,
});

export function init(sc) {
  scene = sc;
  roadsGroup = new THREE.Group();
  scene.add(roadsGroup);

  document.addEventListener('keydown', (e) => {
    if (!points) return;
    if (e.code === 'Enter') commit();
    else if (e.code === 'Backspace') {
      e.preventDefault();
      points.pop();
      preview();
    } else if (e.code === 'Escape') cancel();
  });
}

export function load(saved) {
  splines = saved;
  rebuildRibbons();
}

/* ---------- click placement ---------- */

export function active() {
  return !!points;
}

export function addControlPoint(point) {
  if (!points) points = [];
  points.push(point.clone());
  preview();
}

export function onDoubleClick() {
  if (points && points.length >= 2) commit();
}

function cancel() {
  points = null;
  preview();
}

function preview() {
  if (previewLine) {
    roadsGroup.remove(previewLine);
    previewLine = null;
  }
  if (!points || points.length < 2) return;
  const curve = new THREE.CatmullRomCurve3(points, false, 'centripetal', 0.5);
  const dense = curve.getSpacedPoints(Math.max(8, points.length * 12));
  const geo = new THREE.BufferGeometry().setFromPoints(
    dense.map((p) => new THREE.Vector3(p.x, p.y + 1.5, p.z)),
  );
  previewLine = new THREE.Line(geo, new THREE.LineBasicMaterial({ color: 0xffcc44 }));
  roadsGroup.add(previewLine);
}

async function commit() {
  const pts = points;
  points = null;
  preview();
  if (!pts || pts.length < 2) return;
  const width = parseFloat(document.getElementById('road-width').value) || 12;

  // Smooth the clicks with Catmull-Rom, sampled every ~5 m.
  const curve = new THREE.CatmullRomCurve3(pts, false, 'centripetal', 0.5);
  const count = Math.max(2, Math.ceil(curve.getLength() / 5));
  const dense = curve.getSpacedPoints(count).map((p) => [p.x, p.z]);

  const spl = { id: crypto.randomUUID(), kind: 'road', width, points: dense };
  splines.push(spl);
  buildRibbon(spl);
  try {
    await fetch('/api/edit/spline', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(spl),
    });
  } catch (err) {
    console.warn('spline:', err);
  }
}

/* ---------- ribbons ---------- */

function buildRibbon(spl) {
  const pts = spl.points;
  if (pts.length < 2) return;
  const half = spl.width / 2;
  const pos = new Float32Array(pts.length * 2 * 3);
  let h = 1;
  for (let i = 0; i < pts.length; i++) {
    const [x, z] = pts[i];
    const [px, pz] = pts[Math.max(0, i - 1)];
    const [nx, nz] = pts[Math.min(pts.length - 1, i + 1)];
    let dx = nx - px;
    let dz = nz - pz;
    const l = Math.hypot(dx, dz) || 1;
    dx /= l;
    dz /= l;
    h = ground.heightAt(x, z, h);
    const y = h + 0.15;
    pos.set([x - dz * half, y, z + dx * half, x + dz * half, y, z - dx * half], i * 6);
  }
  const idx = [];
  for (let i = 0; i < pts.length - 1; i++) {
    const a = i * 2;
    idx.push(a, a + 1, a + 2, a + 2, a + 1, a + 3);
  }
  const geo = new THREE.BufferGeometry();
  geo.setAttribute('position', new THREE.BufferAttribute(pos, 3));
  geo.setIndex(idx);
  geo.computeVertexNormals();
  const mesh = new THREE.Mesh(geo, asphalt);
  mesh.userData.splineId = spl.id;
  roadsGroup.add(mesh);
}

function rebuildRibbons() {
  for (const child of [...roadsGroup.children]) {
    if (child.userData.splineId) {
      roadsGroup.remove(child);
      child.geometry.dispose();
    }
  }
  for (const spl of splines) buildRibbon(spl);
}

/// Terrain rebuilt under the roads (flattening landed) — re-drape.
export function onTerrainChanged() {
  clearTimeout(redrapeTimer);
  redrapeTimer = setTimeout(rebuildRibbons, 1200);
}
