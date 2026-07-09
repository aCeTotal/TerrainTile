// Shared brush mechanics: raycast into the terrain, ring cursor, and
// stroke-point sampling along a drag.

import * as THREE from 'three';
import * as far from '../far.js';
import * as near from '../near.js';

const raycaster = new THREE.Raycaster();
const pointer = new THREE.Vector2();

let ring = null;

export function initCursor(scene) {
  const geo = new THREE.RingGeometry(0.92, 1.0, 48);
  const mat = new THREE.MeshBasicMaterial({
    color: 0x4da3ff,
    transparent: true,
    opacity: 0.8,
    side: THREE.DoubleSide,
    depthTest: false,
  });
  ring = new THREE.Mesh(geo, mat);
  ring.rotation.x = -Math.PI / 2;
  ring.renderOrder = 999;
  ring.visible = false;
  scene.add(ring);
}

/// Terrain point under the mouse event, or null.
export function pick(event, camera, canvas) {
  const r = canvas.getBoundingClientRect();
  pointer.x = ((event.clientX - r.left) / r.width) * 2 - 1;
  pointer.y = -((event.clientY - r.top) / r.height) * 2 + 1;
  raycaster.setFromCamera(pointer, camera);
  // The sea plane makes open water sculptable/paintable too.
  const hits = raycaster.intersectObjects([...near.allMeshes(), ...far.seaMeshes()], false);
  return hits.length ? hits[0].point : null;
}

export function showCursor(point, radius) {
  if (!ring) return;
  if (!point) {
    ring.visible = false;
    return;
  }
  ring.visible = true;
  ring.position.set(point.x, point.y + 0.5, point.z);
  ring.scale.setScalar(radius);
}

export function hideCursor() {
  if (ring) ring.visible = false;
}

/// Emits brush centers spaced along a drag: at most one per `spacing`
/// fraction of the radius, so stroke density is radius-independent.
export function strokeSampler(spacing = 0.35) {
  let last = null;
  return (point, radius) => {
    if (last && last.distanceTo(point) < radius * spacing) return false;
    last = point.clone();
    return true;
  };
}
