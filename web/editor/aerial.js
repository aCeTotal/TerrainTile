// Aerial view ("flyfoto"): the whole terrain seen straight from above
// through an orthographic camera — the far layer + sea plane is the whole
// picture. Right/middle-drag pans, wheel zooms at the cursor; the left
// button belongs to the active tool (lasso, plots).

import * as THREE from 'three';

let cam = null;
let canvas = null;
let on = false;
let world = { w: 0, h: 0 };
let panning = false;

export function init(ds, cv) {
  canvas = cv;
  world.w = ds.tiles_x * ds.tile_size_m;
  world.h = ds.tiles_y * ds.tile_size_m;
  cam = new THREE.OrthographicCamera(-1, 1, 1, -1, 1, 40000);
  cam.position.set(world.w / 2, 20000, world.h / 2);
  cam.up.set(0, 0, -1); // north (−z) is up on screen
  cam.lookAt(world.w / 2, 0, world.h / 2);
  resize();

  canvas.addEventListener('pointerdown', (e) => {
    if (on && (e.button === 1 || e.button === 2)) panning = true;
  });
  window.addEventListener('pointerup', () => { panning = false; });
  canvas.addEventListener('pointermove', (e) => {
    if (!on || !panning) return;
    const r = canvas.getBoundingClientRect();
    cam.position.x -= (e.movementX / r.width) * (cam.right - cam.left) / cam.zoom;
    cam.position.z -= (e.movementY / r.height) * (cam.top - cam.bottom) / cam.zoom;
  });
  canvas.addEventListener(
    'wheel',
    (e) => {
      if (!on) return;
      e.preventDefault();
      const [wx, wz] = screenToWorld(e);
      cam.zoom = Math.min(80, Math.max(0.9, cam.zoom * (e.deltaY < 0 ? 1.2 : 1 / 1.2)));
      cam.updateProjectionMatrix();
      // Keep the point under the cursor fixed while zooming.
      const [nx, nz] = screenToWorld(e);
      cam.position.x += wx - nx;
      cam.position.z += wz - nz;
    },
    { passive: false },
  );
}

export function toggle(force) {
  on = force !== undefined ? force : !on;
  return on;
}

export function active() {
  return on;
}

export function camera() {
  return on ? cam : null;
}

export function resize() {
  if (!cam || !canvas) return;
  const aspect = (canvas.clientWidth || 1) / (canvas.clientHeight || 1);
  const half = Math.max(world.w, world.h) / 2;
  cam.left = -half * Math.max(1, aspect);
  cam.right = half * Math.max(1, aspect);
  cam.top = half * Math.max(1, 1 / aspect);
  cam.bottom = -half * Math.max(1, 1 / aspect);
  cam.updateProjectionMatrix();
}

/// World [x, z] under a pointer event.
export function screenToWorld(e) {
  const r = canvas.getBoundingClientRect();
  const u = ((e.clientX - r.left) / r.width) * 2 - 1;
  const v = -(((e.clientY - r.top) / r.height) * 2 - 1);
  const x = cam.position.x + (u * (cam.right - cam.left)) / 2 / cam.zoom;
  const z = cam.position.z - (v * (cam.top - cam.bottom)) / 2 / cam.zoom;
  return [x, z];
}
