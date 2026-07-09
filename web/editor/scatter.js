// Scatter areas: lasso a polygon, pick a model + density/spacing/scale,
// and the server expands it deterministically into scatter.json. Rendered
// as one InstancedMesh per area; instances re-snap automatically when the
// terrain rebuilds underneath (SSE `scatter`).

import * as THREE from 'three';
import * as overlay from './overlay.js';

const $ = (id) => document.getElementById(id);

let scene = null;
let templateOf = null; // (asset) => Promise<Object3D> — from meshes.js
let areas = [];
let saveTimer = null;
const rendered = new Map(); // area id -> { mesh: InstancedMesh, loop: Line }

export function init(sc, templateFn) {
  scene = sc;
  templateOf = templateFn;
}

export function load(saved) {
  areas = saved || [];
  renderList();
  reload();
}

export function params() {
  return {
    density_ha: parseFloat($('sc-density').value) || 100,
    min_spacing: parseFloat($('sc-spacing').value) || 4,
    scale_min: parseFloat($('sc-smin').value) || 0.8,
    scale_max: parseFloat($('sc-smax').value) || 1.3,
  };
}

export function addArea(asset, polygon) {
  const p = params();
  areas.push({
    id: crypto.randomUUID(),
    asset,
    polygon,
    seed: crypto.getRandomValues(new Uint32Array(1))[0],
    rot_random: true,
    ...p,
  });
  renderList();
  save();
}

function removeArea(id) {
  areas = areas.filter((a) => a.id !== id);
  drop(id);
  renderList();
  save();
}

function save() {
  clearTimeout(saveTimer);
  saveTimer = setTimeout(async () => {
    try {
      await fetch('/api/scatter', {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ scatter: areas }),
      });
      // The SSE `scatter` event triggers reload().
    } catch (err) {
      console.warn('scatter:', err);
    }
  }, 300);
}

function renderList() {
  const el = $('scatter-list');
  el.innerHTML = '';
  for (const a of areas) {
    const row = document.createElement('div');
    row.className = 'mat-item';
    row.innerHTML = `<span>${a.asset.split('/').pop()} (${a.density_ha}/ha)</span>
      <button class="tbtn small">✕</button>`;
    row.querySelector('button').addEventListener('click', () => removeArea(a.id));
    el.appendChild(row);
  }
}

function drop(id) {
  const r = rendered.get(id);
  if (!r) return;
  if (r.mesh) {
    scene.remove(r.mesh);
    r.mesh.dispose();
  }
  if (r.loop) overlay.group().remove(r.loop);
  rendered.delete(id);
}

/// Re-fetch scatter.json and rebuild the instanced meshes.
export async function reload() {
  let doc;
  try {
    const res = await fetch('/data/scatter.json?v=' + Date.now());
    if (!res.ok) return;
    doc = await res.json();
  } catch {
    return;
  }
  for (const id of [...rendered.keys()]) drop(id);
  for (const area of doc.areas || []) {
    const def = areas.find((a) => a.id === area.id);
    if (!area.instances.length) continue;
    let tpl;
    try {
      tpl = await templateOf(area.asset);
    } catch {
      continue;
    }
    // First real mesh in the model carries the instancing.
    let srcMesh = null;
    tpl.traverse((o) => {
      if (!srcMesh && o.isMesh) srcMesh = o;
    });
    if (!srcMesh) continue;
    const inst = new THREE.InstancedMesh(srcMesh.geometry, srcMesh.material, area.instances.length);
    inst.castShadow = true;
    const m = new THREE.Matrix4();
    const q = new THREE.Quaternion();
    const up = new THREE.Vector3(0, 1, 0);
    area.instances.forEach((it, i) => {
      q.setFromAxisAngle(up, it.rot_y);
      m.compose(
        new THREE.Vector3(it.pos[0], it.pos[1], it.pos[2]),
        q,
        new THREE.Vector3(it.scale, it.scale, it.scale),
      );
      inst.setMatrixAt(i, m);
    });
    inst.instanceMatrix.needsUpdate = true;
    scene.add(inst);
    const loop = def ? overlay.makeLoop(def.polygon, 0x8bc34a) : null;
    if (loop) overlay.group().add(loop);
    rendered.set(area.id, { mesh: inst, loop });
  }
}
