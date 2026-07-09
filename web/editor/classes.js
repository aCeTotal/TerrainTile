// Texture mode: material classes. Create/edit classes (terrain gates,
// weight, sharp transitions), upload PBR materials (.zip or loose maps),
// and lasso-paint coverage on the aerial view.

import * as aerial from './aerial.js';
import * as lasso from './lasso.js';
import { initClasses as reloadShaderClasses } from '../terrain-material.js';

const $ = (id) => document.getElementById(id);

let classes = [];
let selected = null; // class id
let saveTimer = null;
let paintMode = null; // null | 'paint' | 'erase'

export async function init() {
  $('class-new').addEventListener('click', addClass);
  $('class-delete').addEventListener('click', removeClass);
  $('class-paint').addEventListener('click', () => setPaint('paint'));
  $('class-erase').addEventListener('click', () => setPaint('erase'));
  $('mat-file').addEventListener('change', uploadMaterial);
  $('mat-upload').addEventListener('click', () => $('mat-file').click());
  for (const [id, key, parse] of [
    ['cls-name', 'name', (v) => v],
    ['cls-color', 'color', (v) => v],
    ['cls-weight', 'weight', parseFloat],
    ['cls-hmin', 'h_min', optFloat],
    ['cls-hmax', 'h_max', optFloat],
    ['cls-smin', 'slope_min', optFloat],
    ['cls-smax', 'slope_max', optFloat],
  ]) {
    $(id).addEventListener('change', (e) => setField(key, parse(e.target.value)));
  }
  $('cls-base').addEventListener('change', (e) => setField('base', e.target.checked));
  $('cls-sharp').addEventListener('change', (e) => setField('sharp', e.target.checked));
  await refresh();
}

function optFloat(v) {
  return v === '' ? null : parseFloat(v);
}

async function refresh() {
  try {
    const res = await fetch('/api/classes');
    if (!res.ok) return;
    classes = (await res.json()).classes || [];
  } catch {
    return;
  }
  if (selected === null && classes.length) selected = classes[0].id;
  renderList();
  renderDetails();
}

function current() {
  return classes.find((c) => c.id === selected) || null;
}

function renderList() {
  const el = $('class-list');
  el.innerHTML = '';
  for (const c of classes) {
    const div = document.createElement('div');
    div.className = 'class-item' + (c.id === selected ? ' selected' : '');
    div.innerHTML = `<span class="swatch" style="background:${c.color}"></span>${c.name}`;
    div.addEventListener('click', () => {
      selected = c.id;
      renderList();
      renderDetails();
    });
    el.appendChild(div);
  }
}

function renderDetails() {
  const c = current();
  $('class-details').classList.toggle('hidden', !c);
  if (!c) return;
  $('cls-name').value = c.name;
  $('cls-color').value = c.color;
  $('cls-weight').value = c.weight ?? 1;
  $('cls-hmin').value = c.h_min ?? '';
  $('cls-hmax').value = c.h_max ?? '';
  $('cls-smin').value = c.slope_min ?? '';
  $('cls-smax').value = c.slope_max ?? '';
  $('cls-base').checked = !!c.base;
  $('cls-sharp').checked = !!c.sharp;

  const mats = $('mat-list');
  mats.innerHTML = '';
  c.materials.forEach((m, i) => {
    const name = m.dir.split('/').pop();
    const row = document.createElement('div');
    row.className = 'mat-item';
    row.innerHTML =
      `<span>${name}</span>
       <input type="range" min="0" max="1" step="0.05" value="${m.amount}" title="Synlighet">
       <select><option value="mix"${m.mode !== 'top' ? ' selected' : ''}>bland</option>
               <option value="top"${m.mode === 'top' ? ' selected' : ''}>oppå</option></select>
       <button class="tbtn small">✕</button>`;
    row.querySelector('input').addEventListener('change', (e) => {
      m.amount = parseFloat(e.target.value);
      scheduleSave();
    });
    row.querySelector('select').addEventListener('change', (e) => {
      m.mode = e.target.value;
      scheduleSave();
    });
    row.querySelector('button').addEventListener('click', async () => {
      await fetch(`/api/classes/${c.id}/material/${encodeURIComponent(name)}/delete`, {
        method: 'POST',
      });
      c.materials.splice(i, 1);
      renderDetails();
      reloadShaderClasses();
    });
    mats.appendChild(row);
  });
}

function setField(key, value) {
  const c = current();
  if (!c) return;
  c[key] = value;
  renderList();
  scheduleSave();
}

function addClass() {
  const id = classes.reduce((m, c) => Math.max(m, c.id), -1) + 1;
  classes.push({
    id,
    name: `klasse ${id}`,
    color: '#888888',
    avg_color: '#888888',
    base: false,
    sharp: false,
    water: false,
    road: false,
    weight: 1.0,
    h_min: null,
    h_max: null,
    slope_min: null,
    slope_max: null,
    materials: [],
  });
  selected = id;
  renderList();
  renderDetails();
  scheduleSave();
}

function removeClass() {
  classes = classes.filter((c) => c.id !== selected);
  selected = classes.length ? classes[0].id : null;
  renderList();
  renderDetails();
  scheduleSave();
}

function scheduleSave() {
  clearTimeout(saveTimer);
  saveTimer = setTimeout(async () => {
    try {
      const res = await fetch('/api/classes', {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ classes }),
      });
      const data = await res.json();
      if (!res.ok) alert(data.error || 'kunne ikke lagre klassene');
      reloadShaderClasses();
    } catch (err) {
      console.warn('classes:', err);
    }
  }, 600);
}

async function uploadMaterial() {
  const c = current();
  const file = $('mat-file').files[0];
  if (!c || !file) return;
  const res = await fetch(
    `/api/classes/${c.id}/material/${encodeURIComponent(file.name)}`,
    { method: 'POST', body: file },
  );
  const data = await res.json();
  if (!res.ok) {
    alert(data.error || 'opplasting feilet');
    return;
  }
  classes = data.classes;
  renderDetails();
  reloadShaderClasses();
}

/* ---------- aerial painting ---------- */

function setPaint(mode) {
  paintMode = paintMode === mode ? null : mode;
  $('class-paint').classList.toggle('active', paintMode === 'paint');
  $('class-erase').classList.toggle('active', paintMode === 'erase');
  if (!paintMode) {
    lasso.disarm();
    return;
  }
  aerial.toggle(true);
  arm();
}

function arm() {
  const c = current();
  if (!c) return;
  lasso.arm(
    async (polygon) => {
      try {
        await fetch('/api/classes/paint', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ class: c.id, polygon, erase: paintMode === 'erase' }),
        });
      } catch (err) {
        console.warn('paint:', err);
      }
      if (paintMode) arm(); // stay armed for the next stroke
    },
    c.color,
    paintMode === 'erase',
  );
}

/// Leaving texture mode: stop painting.
export function deactivate() {
  paintMode = null;
  $('class-paint').classList.remove('active');
  $('class-erase').classList.remove('active');
  lasso.disarm();
}
