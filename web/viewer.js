// 3D terrain viewer: the whole dataset as one coarse BatchedMesh (far.js)
// with detailed, ortho-textured tiles streamed around the camera (near.js).
// Fly camera with pointer lock.

import * as THREE from 'three';
import { RGBELoader } from 'three/addons/loaders/RGBELoader.js';
import * as far from './far.js';
import * as near from './near.js';
import * as ed from './editor/editor.js';

const $ = (id) => document.getElementById(id);

const SKY = 0x9fc4e8;

let renderer, scene, camera, dataset, sun;
let active = false;
let initialized = false;
let nearDist = 2500;

/* ---------- controls ---------- */

const keys = new Set();
let yaw = 0, pitch = -0.6, speed = 500;
let locked = false;

let rmbLook = false; // edit mode: right-drag looks around

function setupControls(canvas) {
  canvas.addEventListener('click', () => {
    if (active && dataset && ed.isFly()) canvas.requestPointerLock();
  });
  document.addEventListener('pointerlockchange', () => {
    locked = document.pointerLockElement === canvas;
  });
  canvas.addEventListener('contextmenu', (e) => e.preventDefault());
  canvas.addEventListener('pointerdown', (e) => {
    if (!ed.isFly() && e.button === 2) rmbLook = true;
  });
  window.addEventListener('pointerup', (e) => {
    if (e.button === 2) rmbLook = false;
  });
  document.addEventListener('mousemove', (e) => {
    if (!locked && !rmbLook) return;
    yaw -= e.movementX * 0.0022;
    pitch -= e.movementY * 0.0022;
    pitch = Math.max(-1.55, Math.min(1.55, pitch));
  });
  document.addEventListener('keydown', (e) => {
    if (e.target.tagName === 'INPUT' || e.target.tagName === 'SELECT') return;
    if (locked || (active && !ed.isFly())) keys.add(e.code);
  });
  document.addEventListener('keyup', (e) => keys.delete(e.code));
  canvas.addEventListener('wheel', (e) => {
    if (!ed.isFly()) return; // edit mode: wheel = brush radius
    speed *= e.deltaY < 0 ? 1.25 : 0.8;
    speed = Math.max(2, Math.min(5000, speed));
  }, { passive: true });
}

function moveCamera(dt) {
  camera.rotation.set(pitch, yaw, 0, 'YXZ');
  if (!locked && ed.isFly()) return;
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
  const ms = quality.avg > 0 ? `  •  ${(quality.avg * 1000).toFixed(1)} ms` : '';
  $('hud-tiles').textContent =
    `${s.meshes} detaljfliser (${s.inFlight} lastes)  •  kvalitet ${Math.round(quality.level * 100)} %${ms}${farTxt}`;
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
  renderer.shadowMap.enabled = true;
  renderer.shadowMap.type = THREE.PCFSoftShadowMap;
  renderer.toneMapping = THREE.ACESFilmicToneMapping;
  renderer.toneMappingExposure = 1.15;

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

  loadSky();
  sun = new THREE.DirectionalLight(0xfff2dd, 2.4);
  sun.castShadow = true;
  sun.shadow.mapSize.set(2048, 2048);
  const sc = sun.shadow.camera;
  // Only the hero tile casts shadows; a tight volume keeps them crisp.
  sc.left = sc.bottom = -400;
  sc.right = sc.top = 400;
  sc.near = 100;
  sc.far = 12000;
  sun.shadow.bias = -0.0003;
  sun.shadow.normalBias = 1.5;
  scene.add(sun);
  scene.add(sun.target);
  applyQuality();

  setupControls(canvas);
  near.init(scene, dataset, far.setTileCovered);
  far.init(scene, dataset, near.isCovered); // streams in the background
  ed.init(scene, camera, canvas, dataset, (mode) => {
    if (mode !== 'fly' && document.pointerLockElement) document.exitPointerLock();
  });

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
  ed.resize();
}

/* ---------- sky ---------- */

// HDRI with real clouds, used both as backdrop and as image-based ambient
// light (scene.environment) — sky-blue from above, warm from the sun side.
// Falls back to a hemisphere light if the download fails (offline).
const HDRI_URL =
  'https://dl.polyhaven.org/file/ph-assets/HDRIs/hdr/1k/kloofendal_48d_partly_cloudy_puresky_1k.hdr';

function loadSky() {
  const fallback = new THREE.HemisphereLight(0xcfe5ff, 0x3a3f33, 0.6);
  scene.add(fallback);
  new RGBELoader().load(
    HDRI_URL,
    (tex) => {
      tex.mapping = THREE.EquirectangularReflectionMapping;
      scene.background = tex;
      scene.environment = tex;
      scene.environmentIntensity = 0.8;
      scene.backgroundIntensity = 1.0;
      scene.remove(fallback);
    },
    undefined,
    () => console.warn('HDRI utilgjengelig — bruker enkel himmel'),
  );
}

/* ---------- adaptive quality: 10 ms frame budget (100 fps) ---------- */

// Rolling frame-time average steers rich-tile count, pixel ratio and
// shadows toward a 10 ms budget. rAF is vsync-capped, so when the display
// itself refreshes slower than 100 Hz the budget falls back to the
// observed refresh interval — the target is frame TIME, not raw fps.
// Starts conservative and works UP when there is headroom — performance
// first, always; extra polish only when the GPU has proven it can pay.
const quality = { level: 0.5, frames: 0, time: 0, minDt: 1 / 60, avg: 0 };

function adaptQuality(dt) {
  if (dt > 1.0) return; // tab was hidden; not a real frame time
  // Decaying minimum ≈ the display's refresh interval under vsync.
  quality.minDt = Math.min(quality.minDt * 1.001, Math.max(dt, 0.002));
  quality.frames++;
  quality.time += Math.min(dt, 0.25);
  if (quality.frames < 30) return;
  const avg = quality.time / quality.frames;
  quality.avg = avg;
  quality.frames = 0;
  quality.time = 0;
  const budget = Math.max(0.01, quality.minDt * 1.02);
  if (avg > budget * 1.12) {
    quality.level = Math.max(0.2, quality.level - (avg > budget * 2 ? 0.3 : 0.15));
  } else if (avg < budget * 0.9) {
    quality.level = Math.min(1.0, quality.level + 0.05);
  } else return;
  applyQuality();
}

function applyQuality() {
  const q = quality.level;
  // Pixel ratio is the biggest lever; never above 1.5 — 4K-supersampling
  // has killed weaker machines outright.
  renderer.setPixelRatio(Math.min(devicePixelRatio, q < 0.4 ? 1 : q < 0.75 ? 1.25 : 1.5));
  near.setRichCount(1 + Math.round(q * 4));
  const wantShadows = q >= 0.45;
  if (sun.castShadow !== wantShadows) sun.castShadow = wantShadows;
  const size = q < 0.7 ? 1024 : 2048;
  if (sun.shadow.mapSize.x !== size) {
    sun.shadow.mapSize.set(size, size);
    if (sun.shadow.map) {
      sun.shadow.map.dispose();
      sun.shadow.map = null;
    }
  }
}

// Keep the shadow volume centered on the camera. The anchor snaps to a
// 2 m grid so the shadow map doesn't shimmer as the camera glides.
const SUN_DIR = new THREE.Vector3(0.5, 0.75, -0.55).normalize();

function updateSun() {
  const ax = Math.round(camera.position.x / 2) * 2;
  const az = Math.round(camera.position.z / 2) * 2;
  sun.target.position.set(ax, 0, az);
  sun.position.set(ax, 0, az).addScaledVector(SUN_DIR, 6000);
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
  adaptQuality(dt);
  updateSun();
  renderer.render(scene, ed.activeCamera() || camera);
}

// Tile streaming runs on a timer, not rAF: loads keep flowing even when
// the browser throttles animation frames (hidden tab, headless).
function tick() {
  if (!active || !dataset) return;
  // In the aerial view the far layer + sea plane is the whole picture —
  // no point streaming detail tiles under it.
  if (!ed.aerialActive()) near.update(camera, nearDist);
  updateHud();
  updateSun();
  renderer.render(scene, ed.activeCamera() || camera);
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

/// SSE `tiles` event: the server rebuilt these after an edit.
export function onTiles(names) {
  if (!initialized) return;
  near.refresh(names);
  ed.onTiles();
}

/// SSE `scatter`: scatter.json was regenerated.
export function onScatter() {
  if (initialized) ed.onScatter();
}

/// SSE `conform`: roads re-blended, placements re-snapped.
export function onConform() {
  if (initialized) ed.onConform();
}
