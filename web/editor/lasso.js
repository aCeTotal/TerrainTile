// Freehand lasso ("freetool") drawn on a 2D overlay canvas above the
// aerial view; the finished stroke becomes a closed polygon in world
// coordinates. Shared by texture paint, scatter areas and plots.

import * as aerial from './aerial.js';

let overlay = null;
let ctx = null;
let cb = null; // (polygonWorld, erase) -> void
let color = '#4da3ff';
let points = null; // screen points while drawing
let erase = false;

function ensureOverlay() {
  if (overlay) return;
  overlay = document.createElement('canvas');
  overlay.id = 'lasso-overlay';
  overlay.style.cssText =
    'position:absolute;inset:0;width:100%;height:100%;pointer-events:none;z-index:5';
  document.getElementById('view-viewer').appendChild(overlay);
  ctx = overlay.getContext('2d');
  overlay.addEventListener('pointerdown', onDown);
  overlay.addEventListener('pointermove', onMove);
  window.addEventListener('pointerup', onUp);
}

/// Arm the lasso: the next left-drag on the aerial view draws a polygon
/// and calls back with world coordinates. Stays armed until disarm().
export function arm(onPolygon, strokeColor = '#4da3ff', eraseMode = false) {
  ensureOverlay();
  cb = onPolygon;
  color = strokeColor;
  erase = eraseMode;
  overlay.style.pointerEvents = 'auto';
  overlay.style.cursor = 'crosshair';
}

export function disarm() {
  if (!overlay) return;
  cb = null;
  points = null;
  overlay.style.pointerEvents = 'none';
  clear();
}

export function armed() {
  return !!cb;
}

function fit() {
  if (overlay.width !== overlay.clientWidth || overlay.height !== overlay.clientHeight) {
    overlay.width = overlay.clientWidth;
    overlay.height = overlay.clientHeight;
  }
}

function clear() {
  if (ctx) ctx.clearRect(0, 0, overlay.width, overlay.height);
}

function onDown(e) {
  if (!cb || e.button !== 0 || !aerial.active()) return;
  fit();
  points = [[e.offsetX, e.offsetY]];
}

function onMove(e) {
  if (!points) return;
  const [lx, ly] = points[points.length - 1];
  if (Math.hypot(e.offsetX - lx, e.offsetY - ly) < 4) return;
  points.push([e.offsetX, e.offsetY]);
  clear();
  ctx.strokeStyle = color;
  ctx.fillStyle = color + '33';
  ctx.lineWidth = 2;
  ctx.setLineDash(erase ? [6, 4] : []);
  ctx.beginPath();
  ctx.moveTo(points[0][0], points[0][1]);
  for (const [x, y] of points) ctx.lineTo(x, y);
  ctx.closePath();
  ctx.stroke();
  ctx.fill();
}

function onUp(e) {
  if (!points) return;
  const pts = points;
  points = null;
  clear();
  if (pts.length < 3 || !cb) return;
  const r = overlay.getBoundingClientRect();
  const poly = pts.map(([x, y]) =>
    aerial.screenToWorld({ clientX: x + r.left, clientY: y + r.top }),
  );
  cb(poly, erase);
  void e;
}
