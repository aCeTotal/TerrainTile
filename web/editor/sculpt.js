// Sculpt mode: instant preview on loaded geometry + debounced authoritative
// strokes to the server (same math server-side; rebuilt tiles stream back).

import * as near from '../near.js';
import { editor } from './toolbar.js';

const queue = [];
let flushTimer = null;
let flattenTarget = null;
let normalTimer = null;
const touched = new Set(); // meshes needing recomputed normals

export function beginStroke(point) {
  flattenTarget = point.y;
}

export function applyAt(point) {
  const s = {
    tool: editor.tool,
    x: point.x,
    z: point.z,
    radius: editor.radius,
    strength: editor.tool === 'raise' || editor.tool === 'lower'
      ? editor.strength
      : Math.min(1, editor.strength * 0.35),
    target_h: editor.tool === 'flatten' ? flattenTarget : null,
  };
  preview(s);
  queue.push(s);
  if (!flushTimer) flushTimer = setTimeout(flush, 250);
}

export function endStroke() {
  flush();
}

// Mirror of the server-side falloff so the preview matches the rebuild.
function preview(s) {
  for (const { tileX, tileY, mesh } of near.meshesInRadius(s.x, s.z, s.radius)) {
    const pos = mesh.geometry.attributes.position;
    const ox = mesh.position.x;
    const oz = mesh.position.z;
    for (let i = 0; i < pos.count; i++) {
      const dx = pos.getX(i) + ox - s.x;
      const dz = pos.getZ(i) + oz - s.z;
      const d = Math.hypot(dx, dz);
      let t = Math.max(0, Math.min(1, 1 - d / s.radius));
      const fall = t * t * (3 - 2 * t);
      if (fall <= 0) continue;
      const y = pos.getY(i);
      if (s.tool === 'raise') pos.setY(i, y + s.strength * fall);
      else if (s.tool === 'lower') pos.setY(i, y - s.strength * fall);
      else if (s.tool === 'flatten') pos.setY(i, y + (s.target_h - y) * fall * s.strength);
      // smooth: no preview — the rebuild lands in under a second
    }
    pos.needsUpdate = true;
    touched.add(mesh);
    void tileX; void tileY;
  }
  // Normals are recomputed once per pause, not per stroke event — the
  // full recompute on a 257² grid is the expensive part.
  clearTimeout(normalTimer);
  normalTimer = setTimeout(() => {
    for (const mesh of touched) mesh.geometry.computeVertexNormals();
    touched.clear();
  }, 120);
}

async function flush() {
  clearTimeout(flushTimer);
  flushTimer = null;
  if (!queue.length) return;
  const strokes = queue.splice(0);
  try {
    await fetch('/api/edit/height', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ strokes }),
    });
  } catch (err) {
    console.warn('sculpt:', err);
  }
}
