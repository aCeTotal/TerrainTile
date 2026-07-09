// Meshes mode: upload GLB models, click-place with a transform gizmo,
// drag-scatter along a stroke. Placements persist via PUT /api/placements
// and are restored from project.json.

import * as THREE from 'three';
import { GLTFLoader } from 'three/addons/loaders/GLTFLoader.js';
import { TransformControls } from 'three/addons/controls/TransformControls.js';
import * as brush from './brush.js';
import * as roads from './spline.js';
import * as scatter from './scatter.js';
import * as aerial from './aerial.js';
import * as lasso from './lasso.js';
import * as ground from './ground.js';
import { editor } from './toolbar.js';

const $ = (id) => document.getElementById(id);

let scene = null;
let camera = null;
let canvas = null;
let group = null; // all placed instances
let gizmo = null;
let gizmoBusy = false;
let active = null; // selected asset file name
let selected = null; // selected placement id
let placements = [];
const objects = new Map(); // id -> Object3D
const loader = new GLTFLoader();
const assetCache = new Map(); // asset path -> Promise<Object3D template>
const raycaster = new THREE.Raycaster();
let saveTimer = null;

export function init(sc, cam, cv) {
  scene = sc;
  camera = cam;
  canvas = cv;
  group = new THREE.Group();
  scene.add(group);
  roads.init(scene);
  scatter.init(scene, template);

  gizmo = new TransformControls(camera, canvas);
  scene.add(gizmo.getHelper ? gizmo.getHelper() : gizmo);
  gizmo.addEventListener('dragging-changed', (e) => { gizmoBusy = e.value; });
  gizmo.addEventListener('objectChange', () => {
    const p = placements.find((x) => x.id === selected);
    const o = objects.get(selected);
    if (p && o) {
      // Meshes always sit on the terrain: translate snaps Y to the ground.
      if (gizmo.mode === 'translate') {
        o.position.y = ground.heightAt(o.position.x, o.position.z, o.position.y);
      }
      p.pos = [o.position.x, o.position.y, o.position.z];
      p.rot_y = o.rotation.y;
      p.scale = o.scale.x;
      scheduleSave();
    }
  });

  for (const btn of document.querySelectorAll('#meshes-panel [data-mtool]')) {
    btn.addEventListener('click', () => {
      editor.meshTool = btn.dataset.mtool;
      for (const b of document.querySelectorAll('#meshes-panel [data-mtool]')) {
        b.classList.toggle('active', b === btn);
      }
      lasso.disarm();
      if (btn.dataset.mtool === 'scatter') armScatter();
    });
  }
  $('conform').addEventListener('click', async () => {
    $('conform').disabled = true;
    try {
      await fetch('/api/edit/conform', { method: 'POST' });
    } finally {
      $('conform').disabled = false;
    }
  });
  canvas.addEventListener('dblclick', () => {
    if (editor.mode === 'meshes' && editor.meshTool === 'road') roads.onDoubleClick();
  });
  for (const btn of document.querySelectorAll('#meshes-panel [data-gizmo]')) {
    btn.addEventListener('click', () => {
      gizmo.setMode(btn.dataset.gizmo);
      for (const b of document.querySelectorAll('#meshes-panel [data-gizmo]')) {
        b.classList.toggle('active', b === btn);
      }
    });
  }
  $('asset-upload').addEventListener('click', () => $('asset-file').click());
  $('asset-file').addEventListener('change', uploadFile);
  $('asset-select').addEventListener('change', () => { active = $('asset-select').value; });
  document.addEventListener('keydown', (e) => {
    if (e.target.tagName === 'INPUT' || e.target.tagName === 'SELECT') return;
    if (editor.mode === 'meshes' && (e.code === 'Delete' || e.code === 'Backspace')) {
      removeSelected();
    }
  });

  refreshAssets();
  loadProject();
}

/* ---------- assets ---------- */

async function refreshAssets(pick) {
  const res = await fetch('/api/assets');
  const data = await res.json();
  const sel = $('asset-select');
  sel.innerHTML = (data.assets || [])
    .map((n) => `<option value="${n}">${n}</option>`)
    .join('') || '<option value="">(last opp en modell)</option>';
  if (pick) sel.value = pick;
  active = sel.value || null;
}

async function uploadFile() {
  const file = $('asset-file').files[0];
  if (!file) return;
  const res = await fetch(`/api/assets/${encodeURIComponent(file.name)}`, {
    method: 'POST',
    body: file,
  });
  const data = await res.json();
  if (!res.ok) { alert(data.error || 'opplasting feilet'); return; }
  await refreshAssets(file.name);
}

/// Scatter subtool: lasso an area on the aerial view; the server expands
/// it into instances with the current parameters.
function armScatter() {
  aerial.toggle(true);
  lasso.arm((polygon) => {
    if (active) scatter.addArea(`assets/${active}`, polygon);
    if (editor.meshTool === 'scatter') armScatter(); // stay armed
  }, '#8bc34a');
}

export function template(asset) {
  if (!assetCache.has(asset)) {
    assetCache.set(
      asset,
      new Promise((resolve, reject) => {
        loader.load(`/data/${asset}`, (g) => {
          g.scene.traverse((o) => { o.castShadow = true; });
          resolve(g.scene);
        }, undefined, reject);
      }),
    );
  }
  return assetCache.get(asset);
}

/* ---------- placements ---------- */

async function addInstance(p) {
  try {
    const tpl = await template(p.asset);
    const o = tpl.clone(true);
    o.position.set(p.pos[0], p.pos[1], p.pos[2]);
    o.rotation.y = p.rot_y;
    o.scale.setScalar(p.scale);
    o.userData.placementId = p.id;
    group.add(o);
    objects.set(p.id, o);
  } catch (err) {
    console.warn(`asset ${p.asset}:`, err);
  }
}

function select(id) {
  selected = id;
  const o = id && objects.get(id);
  if (o) gizmo.attach(o);
  else gizmo.detach();
}

function removeSelected() {
  if (!selected) return;
  const o = objects.get(selected);
  if (o) group.remove(o);
  objects.delete(selected);
  placements = placements.filter((p) => p.id !== selected);
  select(null);
  scheduleSave();
}

function scheduleSave() {
  clearTimeout(saveTimer);
  saveTimer = setTimeout(async () => {
    try {
      await fetch('/api/placements', {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ placements }),
      });
    } catch (err) {
      console.warn('placements:', err);
    }
  }, 500);
}

async function loadProject() {
  try {
    const res = await fetch('/data/project.json?v=' + Date.now());
    if (!res.ok) return;
    const p = await res.json();
    for (const o of objects.values()) group.remove(o);
    objects.clear();
    select(null);
    placements = p.placements || [];
    for (const pl of placements) addInstance(pl);
    roads.load(p.splines || []);
    scatter.load(p.scatter || []);
  } catch (err) {
    console.warn('project.json:', err);
  }
}

/// "Tilpass terreng" finished server-side: placements were re-snapped.
export function onConform() {
  loadProject();
}

/* ---------- pointer handling (from editor.js) ---------- */

function place(point, rotY = 0, scale = 1) {
  if (!active) return;
  const p = {
    id: crypto.randomUUID(),
    asset: `assets/${active}`,
    pos: [point.x, point.y, point.z],
    rot_y: rotY,
    scale,
  };
  placements.push(p);
  addInstance(p).then(() => select(p.id));
  scheduleSave();
}

function pickInstance(e) {
  const r = canvas.getBoundingClientRect();
  const v = new THREE.Vector2(
    ((e.clientX - r.left) / r.width) * 2 - 1,
    -((e.clientY - r.top) / r.height) * 2 + 1,
  );
  raycaster.setFromCamera(v, camera);
  const hits = raycaster.intersectObjects(group.children, true);
  let o = hits[0]?.object || null;
  while (o && !o.userData.placementId) o = o.parent;
  return o?.userData.placementId || null;
}

export function onDown(e) {
  if (gizmoBusy || e.button !== 0) return;
  const tool = editor.meshTool || 'place';
  if (tool === 'road') {
    const pt = brush.pick(e, camera, canvas);
    if (pt) roads.addControlPoint(pt);
    return;
  }
  if (tool === 'scatter') return; // scatter draws on the aerial lasso
  // place/select: an existing instance takes priority over placing
  const hit = pickInstance(e);
  if (hit) { select(hit); return; }
  const pt = brush.pick(e, camera, canvas);
  if (pt) place(pt);
}

export function onMove(e) {
  if (editor.meshTool === 'road' && roads.active()) {
    const pt = brush.pick(e, camera, canvas);
    if (pt) brush.showCursor(pt, (parseFloat($('road-width').value) || 12) / 2);
  }
}

export function onUp() {}
