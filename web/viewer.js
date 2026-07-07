// 3D terrain viewer: the whole dataset as one coarse BatchedMesh (far.js)
// with detailed, ortho-textured tiles streamed around the camera (near.js).
// Fly camera with pointer lock.

import * as THREE from 'three';
import * as far from './far.js';
import * as near from './near.js';

const $ = (id) => document.getElementById(id);

const SKY = 0x9fc4e8;

let renderer, scene, camera, dataset;
let active = false;
let initialized = false;
let nearDist = 2500;

/* ---------- controls ---------- */

const keys = new Set();
let yaw = 0, pitch = -0.6, speed = 150;
let locked = false;

function setupControls(canvas) {
  canvas.addEventListener('click', () => {
    if (active && dataset) canvas.requestPointerLock();
  });
  document.addEventListener('pointerlockchange', () => {
    locked = document.pointerLockElement === canvas;
  });
  document.addEventListener('mousemove', (e) => {
    if (!locked) return;
    yaw -= e.movementX * 0.0022;
    pitch -= e.movementY * 0.0022;
    pitch = Math.max(-1.55, Math.min(1.55, pitch));
  });
  document.addEventListener('keydown', (e) => { if (locked) keys.add(e.code); });
  document.addEventListener('keyup', (e) => keys.delete(e.code));
  canvas.addEventListener('wheel', (e) => {
    speed *= e.deltaY < 0 ? 1.25 : 0.8;
    speed = Math.max(2, Math.min(5000, speed));
  }, { passive: true });
}

function moveCamera(dt) {
  camera.rotation.set(pitch, yaw, 0, 'YXZ');
  if (!locked) return;
  const v = speed * (keys.has('ShiftLeft') || keys.has('ShiftRight') ? 4 : 1) * dt;
  const fwd = new THREE.Vector3();
  camera.getWorldDirection(fwd);
  const right = new THREE.Vector3().crossVectors(fwd, camera.up).normalize();
  if (keys.has('KeyW')) camera.position.addScaledVector(fwd, v);
  if (keys.has('KeyS')) camera.position.addScaledVector(fwd, -v);
  if (keys.has('KeyD')) camera.position.addScaledVector(right, v);
  if (keys.has('KeyA')) camera.position.addScaledVector(right, -v);
  if (keys.has('KeyE') || keys.has('Space')) camera.position.y += v;
  if (keys.has('KeyQ')) camera.position.y -= v;
}

/* ---------- HUD ---------- */

function updateHud() {
  const e = dataset.origin[0] + camera.position.x;
  const n = dataset.origin[1] - camera.position.z;
  $('hud-pos').textContent =
    `Ø ${e.toFixed(0)}  N ${n.toFixed(0)}  H ${camera.position.y.toFixed(0)} m`;
  const s = near.stats();
  const f = far.progress();
  const farTxt = f.total > 0 && f.loaded < f.total
    ? `  •  oversikt ${Math.round((100 * f.loaded) / f.total)} %`
    : '';
  $('hud-tiles').textContent = `${s.meshes} detaljfliser (${s.inFlight} lastes)${farTxt}`;
  $('hud-speed').textContent = `Fart ${speed.toFixed(0)} m/s`;
}

/* ---------- scene setup ---------- */

async function init() {
  const res = await fetch('/data/dataset.json');
  if (!res.ok) return false;
  dataset = await res.json();

  const canvas = $('gl');
  renderer = new THREE.WebGLRenderer({ canvas, antialias: true });
  renderer.setPixelRatio(Math.min(devicePixelRatio, 2));

  scene = new THREE.Scene();
  scene.background = new THREE.Color(SKY);

  const w = dataset.tiles_x * dataset.tile_size_m;
  const h = dataset.tiles_y * dataset.tile_size_m;
  const diag = Math.hypot(w, h);

  // The whole terrain is always in view range; fog only softens the horizon.
  camera = new THREE.PerspectiveCamera(65, 1, 1, Math.max(30000, diag * 1.5));
  camera.rotation.order = 'YXZ';
  const above = Math.max(150, Math.min(3000, Math.max(w, h) * 0.3));
  camera.position.set(w / 2, (dataset.max_height || 100) + above, h / 2);
  scene.fog = new THREE.Fog(SKY, diag * 0.8, Math.max(30000, diag * 1.4));

  scene.add(new THREE.HemisphereLight(0xcfe5ff, 0x3a3f33, 1.1));
  const sun = new THREE.DirectionalLight(0xfff2dd, 1.6);
  sun.position.set(0.4, 1, -0.6);
  scene.add(sun);

  setupControls(canvas);
  near.init(scene, dataset, far.setTileCovered);
  far.init(scene, dataset, near.isCovered); // streams in the background

  $('view-dist').addEventListener('input', (e) => {
    nearDist = parseInt(e.target.value);
    $('dist-val').textContent = (nearDist / 1000).toFixed(1) + ' km';
  });

  window.addEventListener('resize', resize);
  resize();
  return true;
}

function resize() {
  if (!renderer) return;
  const canvas = renderer.domElement;
  const w = canvas.clientWidth || innerWidth;
  const h = canvas.clientHeight || innerHeight;
  renderer.setSize(w, h, false);
  camera.aspect = w / h;
  camera.updateProjectionMatrix();
}

/* ---------- render loop ---------- */

let lastFrame = 0;
let ticker = null;

function frame(now) {
  if (!active) return;
  requestAnimationFrame(frame);
  const dt = Math.min((now - lastFrame) / 1000, 0.1);
  lastFrame = now;
  moveCamera(dt);
  renderer.render(scene, camera);
}

// Tile streaming runs on a timer, not rAF: loads keep flowing even when
// the browser throttles animation frames (hidden tab, headless).
function tick() {
  if (!active || !dataset) return;
  near.update(camera, nearDist);
  updateHud();
  renderer.render(scene, camera);
}

/* ---------- public API (called from app.js) ---------- */

export async function enter() {
  active = true;
  if (!initialized) {
    const ok = await init();
    if (!ok) {
      $('viewer-empty').classList.remove('hidden');
      $('viewer-hud').classList.add('hidden');
      return; // retried next time the tab is opened
    }
    initialized = true;
  }
  $('viewer-empty').classList.add('hidden');
  $('viewer-hud').classList.remove('hidden');
  resize();
  lastFrame = performance.now();
  requestAnimationFrame(frame);
  if (!ticker) ticker = setInterval(tick, 250);
}

export function leave() {
  active = false;
  if (ticker) { clearInterval(ticker); ticker = null; }
  if (document.pointerLockElement) document.exitPointerLock();
}
