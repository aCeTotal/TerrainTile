// Editor-only 3D markings (scatter outlines, plots, building zones):
// one group with a global visibility toggle. Players never see these —
// nothing here is ever baked into the terrain data.

import * as THREE from 'three';

let grp = null;

export function init(scene) {
  grp = new THREE.Group();
  scene.add(grp);
}

export function group() {
  return grp;
}

export function setVisible(v) {
  if (grp) grp.visible = v;
}

export function visible() {
  return grp ? grp.visible : true;
}

/// Closed outline at a fixed height, in the marking color.
export function makeLoop(points, color, y = 3.0) {
  const v = points.map(([x, z]) => new THREE.Vector3(x, y, z));
  v.push(v[0].clone());
  const geo = new THREE.BufferGeometry().setFromPoints(v);
  return new THREE.Line(geo, new THREE.LineBasicMaterial({ color, depthTest: false }));
}
