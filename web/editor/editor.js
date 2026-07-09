// Editor orchestrator: pointer handling for the edit modes, cursor, and
// dispatch to sculpt/paint (meshes mode lives in meshes.js).

import * as brush from './brush.js';
import * as sculpt from './sculpt.js';
import * as classesUi from './classes.js';
import * as meshes from './meshes.js';
import * as roads from './spline.js';
import * as scatter from './scatter.js';
import * as plots from './plots.js';
import * as aerial from './aerial.js';
import * as lasso from './lasso.js';
import * as overlay from './overlay.js';
import { initClasses } from '../terrain-material.js';
import { editor, initToolbar, nudgeRadius } from './toolbar.js';

let camera = null;
let canvas = null;
let dragging = false;
let sampler = null;
let onModeChange = null;

export function isFly() {
  return editor.mode === 'fly';
}

/// The camera the viewer should render with right now (aerial or null).
export function activeCamera() {
  return aerial.camera();
}

export function aerialActive() {
  return aerial.active();
}

export function toggleAerial(force) {
  const on = aerial.toggle(force);
  if (!on) lasso.disarm();
  plots.onModeChanged();
  return on;
}

export function resize() {
  aerial.resize();
}

export function init(scene, cam, cv, ds, modeChanged) {
  camera = cam;
  canvas = cv;
  onModeChange = modeChanged;
  brush.initCursor(scene);
  overlay.init(scene);
  aerial.init(ds, cv);
  meshes.init(scene, camera, canvas);
  classesUi.init();
  plots.init();
  initClasses(); // shader-side material arrays
  for (const btn of document.querySelectorAll('.aerial-btn')) {
    btn.addEventListener('click', () => toggleAerial());
  }
  document.addEventListener('keydown', (e) => {
    if (e.target.tagName === 'INPUT' || e.target.tagName === 'SELECT') return;
    if (e.code === 'KeyF' && !isFly()) toggleAerial();
  });
  initToolbar((mode) => {
    dragging = false;
    brush.hideCursor();
    if (mode !== 'texture') classesUi.deactivate();
    if (mode === 'fly' || mode === 'sculpt') toggleAerial(false);
    if (mode === 'tomt') toggleAerial(true);
    plots.onModeChanged();
    if (onModeChange) onModeChange(mode);
  });

  canvas.addEventListener('pointermove', onMove);
  canvas.addEventListener('pointerdown', onDown);
  window.addEventListener('pointerup', onUp);
  canvas.addEventListener(
    'wheel',
    (e) => {
      if (isFly()) return;
      e.preventDefault();
      nudgeRadius(-e.deltaY);
    },
    { passive: false },
  );
}

function brushMode() {
  return editor.mode === 'sculpt'; // texture paints via the aerial lasso
}

function onMove(e) {
  if (aerial.active()) return; // aerial input belongs to lasso/pan
  if (editor.mode === 'meshes') {
    meshes.onMove(e);
    return;
  }
  if (!brushMode()) return;
  const point = brush.pick(e, camera, canvas);
  brush.showCursor(point, editor.radius);
  if (dragging && point && sampler(point, editor.radius)) applyAt(point);
}

function onDown(e) {
  if (aerial.active()) return;
  if (editor.mode === 'meshes') {
    if (e.button === 0) {
      dragging = true;
      meshes.onDown(e);
    }
    return;
  }
  if (!brushMode() || e.button !== 0) return;
  const point = brush.pick(e, camera, canvas);
  if (!point) return;
  dragging = true;
  sampler = brush.strokeSampler();
  if (editor.mode === 'sculpt') sculpt.beginStroke(point);
  applyAt(point);
}

function onUp() {
  if (!dragging) return;
  dragging = false;
  if (editor.mode === 'sculpt') sculpt.endStroke();
  else if (editor.mode === 'meshes') meshes.onUp();
}

/// Terrain tiles were rebuilt server-side (sculpt/road landed).
export function onTiles() {
  roads.onTerrainChanged();
}

export function onScatter() {
  scatter.reload();
}

export function onConform() {
  meshes.onConform();
  roads.onTerrainChanged();
}

function applyAt(point) {
  if (editor.mode === 'sculpt') sculpt.applyAt(point);
}
