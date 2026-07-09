// Start screen (Nytt prosjekt / Åpne prosjekt) and the new-project dialog
// with world parameters and a live tile/disk estimate.

import { openFolderModal } from './browse.js';

const $ = (id) => document.getElementById(id);

let defaults = null;
let onOpened = null; // ({ hasDataset }) => void — app.js hides overlay etc.
let output = null;

export function initStart(d, opened) {
  defaults = d;
  onOpened = opened;

  $('start-new').addEventListener('click', openDialog);
  $('start-open').addEventListener('click', openExisting);
  $('dlg-close').addEventListener('click', () => $('dialog').classList.add('hidden'));
  $('dlg-folder').addEventListener('click', async () => {
    const path = await openFolderModal('Velg prosjektmappe', output || defaults.home);
    if (path) {
      output = path;
      $('dlg-folder-path').textContent = path;
      updateEstimate();
    }
  });
  $('dlg-random').addEventListener('click', () => {
    $('dlg-seed').value = randomSeed();
  });
  for (const id of ['dlg-island', 'dlg-margin', 'dlg-tile', 'dlg-res', 'dlg-lods']) {
    $(id).addEventListener('change', updateEstimate);
  }
  $('dlg-create').addEventListener('click', create);
}

export function showStart(canContinue) {
  $('start-continue').classList.toggle('hidden', !canContinue);
  $('start-overlay').classList.remove('hidden');
}

export function hideStart() {
  $('start-overlay').classList.add('hidden');
  $('dialog').classList.add('hidden');
}

function randomSeed() {
  return crypto.getRandomValues(new Uint32Array(1))[0];
}

function openDialog() {
  const w = defaults.world;
  $('dlg-margin').value = String(w.margin_m);
  $('dlg-tile').value = String(w.tile_size_m);
  $('dlg-res').value = String(w.resolution);
  $('dlg-lods').value = String(w.lods);
  $('dlg-seed').value = randomSeed();
  $('dialog').classList.remove('hidden');
  updateEstimate();
}

function params() {
  const areaKm2 = parseFloat($('dlg-island').value);
  return {
    seed: parseInt($('dlg-seed').value) || 1,
    island_m: Math.round(Math.sqrt(areaKm2) * 1000),
    margin_m: parseFloat($('dlg-margin').value),
    tile_size_m: parseFloat($('dlg-tile').value),
    resolution: parseFloat($('dlg-res').value),
    lods: parseInt($('dlg-lods').value),
  };
}

/// Mirror of the server-side validation, so errors show before submit.
function validate(w) {
  const n = w.tile_size_m / w.resolution;
  if (!Number.isInteger(n)) return 'flisstørrelse må være et helt antall piksler';
  const stride = 1 << (w.lods - 1);
  if (n % stride !== 0) return `flis på ${n} px må være delelig med ${stride} (LOD${w.lods - 1})`;
  return null;
}

function updateEstimate() {
  const w = params();
  const err = validate(w);
  const el = $('dlg-estimate');
  if (err) {
    el.innerHTML = `<span class="err">${err}</span>`;
    $('dlg-create').disabled = true;
    return;
  }
  const size = Math.ceil((w.island_m + 2 * w.margin_m) / w.tile_size_m) * w.tile_size_m;
  const tiles = (size / w.tile_size_m) ** 2;
  // Sea tiles are near-free; land + coast band is what costs disk.
  const landEdge = w.island_m + 2 * 4000;
  const landTiles = Math.min(tiles, Math.ceil(landEdge / w.tile_size_m) ** 2);
  const n = w.tile_size_m / w.resolution;
  // Mesh bytes across all LODs ≈ LOD0 × 4/3; masks are small next to this.
  const bytes = landTiles * ((n + 1) ** 2 * 48 + n ** 2 * 24) * 1.34;
  const gb = bytes / 1e9;
  el.innerHTML =
    `${(size / 1000).toFixed(1)} km verden &nbsp;•&nbsp; ${tiles} fliser (~${landTiles} med land)`
    + ` &nbsp;•&nbsp; ca. ${gb < 1 ? gb.toFixed(1) : gb.toFixed(0)} GB`;
  $('dlg-create').disabled = !output;
}

async function create() {
  const w = params();
  if (validate(w) || !output) return;
  const res = await fetch('/api/project/new', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ output, world: w }),
  });
  const data = await res.json();
  if (!res.ok) { alert(data.error || 'kunne ikke starte genereringen'); return; }
  hideStart();
  onOpened({ hasDataset: false, output, project: { world: w } });
}

async function openExisting() {
  const path = await openFolderModal('Åpne prosjektmappe', defaults.home);
  if (!path) return;
  const res = await fetch('/api/open', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ path }),
  });
  const data = await res.json();
  if (!res.ok) { alert(data.error || 'kunne ikke åpne prosjektet'); return; }
  hideStart();
  onOpened({ hasDataset: data.has_dataset, output: data.output, project: data.project });
}
