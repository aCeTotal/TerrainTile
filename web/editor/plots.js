// Tomt og bygninger: quads on the aerial view — plots (numbered) and
// building zones (typed footprints Bevy extrudes). Drag to create, drag
// corners to adjust angles, drag inside to move, rotation slider for
// zones. Editor-only: exported to plots.json, never shown to players.

import * as aerial from './aerial.js';
import * as overlay from './overlay.js';
import { editor } from './toolbar.js';

const $ = (id) => document.getElementById(id);

const DEFAULT_TYPES = [
  { name: 'garasje', color: '#8d99ae', floors: 1 },
  { name: 'enebolig', color: '#e07a5f', floors: 2 },
  { name: 'blokk', color: '#bc6c25', floors: 4 },
  { name: 'skyskraper', color: '#5e60ce', floors: 20 },
  { name: 'bybygning', color: '#9c6644', floors: 3 },
];
const PLOT_COLOR = '#4da3ff';

let plots = [];
let zones = [];
let zoneTypes = [];
let selected = null; // { kind: 'plot'|'zone', id }
let tool = null; // 'plot' | 'zone' | null (select/adjust)
let drag = null; // { mode: 'create'|'corner'|'move', ... }
let canvas = null; // 2D overlay
let ctx = null;
let saveTimer = null;
let loops = new Map(); // id -> overlay loop

export function init() {
  canvas = document.createElement('canvas');
  canvas.id = 'plots-overlay';
  canvas.style.cssText =
    'position:absolute;inset:0;width:100%;height:100%;pointer-events:none;z-index:4';
  document.getElementById('view-viewer').appendChild(canvas);
  ctx = canvas.getContext('2d');
  canvas.addEventListener('pointerdown', onDown);
  canvas.addEventListener('pointermove', onMove);
  window.addEventListener('pointerup', onUp);

  $('plot-new').addEventListener('click', () => setTool('plot'));
  $('zone-new').addEventListener('click', () => setTool('zone'));
  $('zone-rot').addEventListener('input', rotateSelected);
  $('zone-floors').addEventListener('change', (e) => {
    const z = selectedZone();
    if (z) {
      z.floors = parseInt(e.target.value) || 1;
      save();
    }
  });
  $('zone-type-add').addEventListener('click', () => {
    const name = $('zone-type-name').value.trim();
    if (!name || zoneTypes.some((t) => t.name === name)) return;
    zoneTypes.push({ name, color: '#cccc66', floors: 1 });
    $('zone-type-name').value = '';
    renderTypeSelect();
    save();
  });
  $('plots-visible').addEventListener('change', (e) => overlay.setVisible(e.target.checked));
  document.addEventListener('keydown', (e) => {
    if (e.target.tagName === 'INPUT' || e.target.tagName === 'SELECT') return;
    if (editor.mode === 'tomt' && (e.code === 'Delete' || e.code === 'Backspace')) {
      removeSelected();
    }
  });

  loadProject();
  requestAnimationFrame(paintLoop);
}

async function loadProject() {
  try {
    const res = await fetch('/data/project.json?v=' + Date.now());
    if (!res.ok) return;
    const p = await res.json();
    plots = p.plots || [];
    zones = p.zones || [];
    zoneTypes = p.zone_types?.length ? p.zone_types : [...DEFAULT_TYPES];
  } catch {
    zoneTypes = [...DEFAULT_TYPES];
  }
  renderTypeSelect();
  rebuildLoops();
}

function renderTypeSelect() {
  $('zone-type').innerHTML = zoneTypes
    .map((t) => `<option value="${t.name}">${t.name}</option>`)
    .join('');
}

function typeColor(name) {
  return zoneTypes.find((t) => t.name === name)?.color || '#cccc66';
}

function save() {
  clearTimeout(saveTimer);
  saveTimer = setTimeout(async () => {
    try {
      await fetch('/api/plots', {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ plots, zones, zone_types: zoneTypes }),
      });
    } catch (err) {
      console.warn('plots:', err);
    }
  }, 500);
  rebuildLoops();
}

/* ---------- 3D outlines (editor-only) ---------- */

function rebuildLoops() {
  for (const l of loops.values()) overlay.group().remove(l);
  loops.clear();
  for (const p of plots) {
    const l = overlay.makeLoop(p.corners, PLOT_COLOR, 2.5);
    overlay.group().add(l);
    loops.set(p.id, l);
  }
  for (const z of zones) {
    const l = overlay.makeLoop(z.corners, typeColor(z.type), 2.5);
    overlay.group().add(l);
    loops.set(z.id, l);
  }
}

/* ---------- tools ---------- */

function setTool(t) {
  tool = tool === t ? null : t;
  $('plot-new').classList.toggle('active', tool === 'plot');
  $('zone-new').classList.toggle('active', tool === 'zone');
  aerial.toggle(true);
  updatePointer();
}

function updatePointer() {
  const active = editor.mode === 'tomt' && aerial.active();
  canvas.style.pointerEvents = active ? 'auto' : 'none';
  canvas.style.cursor = tool ? 'crosshair' : 'default';
}

export function onModeChanged() {
  updatePointer();
}

function selectedZone() {
  return selected?.kind === 'zone' ? zones.find((z) => z.id === selected.id) : null;
}

function selectedQuad() {
  if (!selected) return null;
  return selected.kind === 'plot'
    ? plots.find((p) => p.id === selected.id)
    : zones.find((z) => z.id === selected.id);
}

function removeSelected() {
  if (!selected) return;
  plots = plots.filter((p) => p.id !== selected.id);
  zones = zones.filter((z) => z.id !== selected.id);
  selected = null;
  save();
}

function rotateSelected(e) {
  const q = selectedQuad();
  if (!q) return;
  const target = (parseFloat(e.target.value) * Math.PI) / 180;
  const delta = target - (q._rot || 0);
  q._rot = target;
  const cx = q.corners.reduce((s, c) => s + c[0], 0) / 4;
  const cz = q.corners.reduce((s, c) => s + c[1], 0) / 4;
  const [sn, cs] = [Math.sin(delta), Math.cos(delta)];
  q.corners = q.corners.map(([x, z]) => {
    const dx = x - cx;
    const dz = z - cz;
    return [cx + dx * cs - dz * sn, cz + dx * sn + dz * cs];
  });
  save();
}

/* ---------- pointer interaction (aerial, screen space) ---------- */

function toScreen([x, z]) {
  const r = canvas.getBoundingClientRect();
  const cam = aerial.camera();
  if (!cam) return [0, 0];
  const u = ((x - cam.position.x) * cam.zoom) / ((cam.right - cam.left) / 2);
  const v = ((z - cam.position.z) * cam.zoom) / ((cam.top - cam.bottom) / 2);
  return [((u + 1) / 2) * r.width, ((v + 1) / 2) * r.height];
}

function world(e) {
  return aerial.screenToWorld(e);
}

function hitCorner(e) {
  for (const q of [...plots, ...zones]) {
    for (let i = 0; i < 4; i++) {
      const [sx, sy] = toScreen(q.corners[i]);
      if (Math.hypot(e.offsetX - sx, e.offsetY - sy) < 9) {
        return { q, i };
      }
    }
  }
  return null;
}

function inside(corners, x, z) {
  let hit = false;
  let j = 3;
  for (let i = 0; i < 4; i++) {
    const [ax, az] = corners[i];
    const [bx, bz] = corners[j];
    if (az > z !== bz > z && x < ((bx - ax) * (z - az)) / (bz - az) + ax) hit = !hit;
    j = i;
  }
  return hit;
}

function onDown(e) {
  if (e.button !== 0 || !aerial.active() || editor.mode !== 'tomt') return;
  const [wx, wz] = world(e);
  if (tool) {
    drag = { mode: 'create', start: [wx, wz], now: [wx, wz] };
    return;
  }
  const corner = hitCorner(e);
  if (corner) {
    selected = {
      kind: plots.includes(corner.q) ? 'plot' : 'zone',
      id: corner.q.id,
    };
    drag = { mode: 'corner', q: corner.q, i: corner.i };
    return;
  }
  for (const q of [...zones, ...plots]) {
    if (inside(q.corners, wx, wz)) {
      selected = { kind: plots.includes(q) ? 'plot' : 'zone', id: q.id };
      drag = { mode: 'move', q, last: [wx, wz] };
      syncFields();
      return;
    }
  }
  selected = null;
}

function onMove(e) {
  if (!drag) return;
  const [wx, wz] = world(e);
  if (drag.mode === 'create') {
    drag.now = [wx, wz];
  } else if (drag.mode === 'corner') {
    drag.q.corners[drag.i] = [wx, wz];
  } else if (drag.mode === 'move') {
    const dx = wx - drag.last[0];
    const dz = wz - drag.last[1];
    drag.q.corners = drag.q.corners.map(([x, z]) => [x + dx, z + dz]);
    drag.last = [wx, wz];
  }
}

function onUp() {
  if (!drag) return;
  const d = drag;
  drag = null;
  if (d.mode === 'create') {
    const [x0, z0] = d.start;
    const [x1, z1] = d.now;
    if (Math.abs(x1 - x0) < 2 || Math.abs(z1 - z0) < 2) return;
    const corners = [
      [Math.min(x0, x1), Math.min(z0, z1)],
      [Math.max(x0, x1), Math.min(z0, z1)],
      [Math.max(x0, x1), Math.max(z0, z1)],
      [Math.min(x0, x1), Math.max(z0, z1)],
    ];
    if (tool === 'plot') {
      const number = plots.reduce((m, p) => Math.max(m, p.number), 0) + 1;
      const p = { id: crypto.randomUUID(), number, corners };
      plots.push(p);
      selected = { kind: 'plot', id: p.id };
    } else {
      const z = {
        id: crypto.randomUUID(),
        plot: plots.find((p) => inside(p.corners, (x0 + x1) / 2, (z0 + z1) / 2))?.id || null,
        type: $('zone-type').value,
        corners,
        floors: parseInt($('zone-floors').value) || 1,
      };
      zones.push(z);
      selected = { kind: 'zone', id: z.id };
    }
    setTool(null);
    syncFields();
  }
  save();
}

function syncFields() {
  const z = selectedZone();
  if (z) {
    $('zone-floors').value = z.floors;
    $('zone-rot').value = ((z._rot || 0) * 180) / Math.PI;
  }
}

/* ---------- drawing ---------- */

function paintLoop() {
  requestAnimationFrame(paintLoop);
  if (!ctx) return;
  if (canvas.width !== canvas.clientWidth || canvas.height !== canvas.clientHeight) {
    canvas.width = canvas.clientWidth;
    canvas.height = canvas.clientHeight;
  }
  ctx.clearRect(0, 0, canvas.width, canvas.height);
  if (editor.mode !== 'tomt' || !aerial.active()) return;

  const drawQuad = (corners, color, label, isSel) => {
    ctx.strokeStyle = color;
    ctx.fillStyle = color + (isSel ? '44' : '22');
    ctx.lineWidth = isSel ? 3 : 2;
    ctx.beginPath();
    corners.forEach((c, i) => {
      const [x, y] = toScreen(c);
      if (i === 0) ctx.moveTo(x, y);
      else ctx.lineTo(x, y);
    });
    ctx.closePath();
    ctx.fill();
    ctx.stroke();
    if (isSel) {
      for (const c of corners) {
        const [x, y] = toScreen(c);
        ctx.fillStyle = '#fff';
        ctx.beginPath();
        ctx.arc(x, y, 5, 0, 7);
        ctx.fill();
        ctx.stroke();
      }
    }
    if (label) {
      const cx = corners.reduce((s, c) => s + c[0], 0) / 4;
      const cz = corners.reduce((s, c) => s + c[1], 0) / 4;
      const [x, y] = toScreen([cx, cz]);
      ctx.fillStyle = '#fff';
      ctx.font = '12px system-ui';
      ctx.textAlign = 'center';
      ctx.fillText(label, x, y);
    }
  };

  for (const p of plots) {
    drawQuad(p.corners, PLOT_COLOR, `tomt ${p.number}`, selected?.id === p.id);
  }
  for (const z of zones) {
    drawQuad(z.corners, typeColor(z.type), z.type, selected?.id === z.id);
  }
  if (drag?.mode === 'create') {
    const [x0, z0] = drag.start;
    const [x1, z1] = drag.now;
    drawQuad(
      [
        [x0, z0],
        [x1, z0],
        [x1, z1],
        [x0, z1],
      ],
      tool === 'plot' ? PLOT_COLOR : typeColor($('zone-type').value),
      null,
      true,
    );
  }
}
