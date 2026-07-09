// Shared "what is the terrain height here" raycast, used by road ribbons,
// mesh snapping and placement restore.

import * as THREE from 'three';
import * as far from '../far.js';
import * as near from '../near.js';

const rc = new THREE.Raycaster();
const down = new THREE.Vector3(0, -1, 0);

export function heightAt(x, z, fallback = 0) {
  rc.set(new THREE.Vector3(x, 10000, z), down);
  const hits = rc.intersectObjects([...near.allMeshes(), ...far.seaMeshes()], false);
  return hits.length ? hits[0].point.y : fallback;
}
