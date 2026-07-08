// Pipeline UI: config form, server-side file browser, SSE progress.

const $ = (id) => document.getElementById(id);

const state = {
  inputs: [],
  output: null,
  scanned: false,
  running: false,
  hasDataset: false,
  defaults: null,
  tileTimes: [], // [t_ms, done] for rate/ETA
};

/* ---------- mask sliders ---------- */

const MASK_FIELDS = [
  { key: 'rock_slope_start', label: 'Fjell fra (°)', min: 10, max: 60 },
  { key: 'rock_slope_full', label: 'Fjell fullt (°)', min: 20, max: 80 },
  { key: 'snow_height_start', label: 'Snø fra (m)', min: 0, max: 2500 },
  { key: 'snow_height_full', label: 'Snø fullt (m)', min: 0, max: 3000 },
  { key: 'dirt_slope_start', label: 'Jord fra (°)', min: 5, max: 40 },
  { key: 'dirt_slope_full', label: 'Jord fullt (°)', min: 10, max: 60 },
];

function buildMaskSliders(defaults) {
  const wrap = $('mask-sliders');
  wrap.innerHTML = '';
  for (const f of MASK_FIELDS) {
    const label = document.createElement('label');
    label.className = 'field';
    label.innerHTML =
      `<span>${f.label}: <b id="mask-${f.key}-val">${defaults[f.key]}</b></span>
       <input type="range" id="mask-${f.key}" min="${f.min}" max="${f.max}" step="1" value="${defaults[f.key]}">`;
    wrap.appendChild(label);
    label.querySelector('input').addEventListener('input', (e) => {
      $(`mask-${f.key}-val`).textContent = e.target.value;
    });
  }
}

function maskValues() {
  const m = {};
  for (const f of MASK_FIELDS) m[f.key] = parseFloat($(`mask-${f.key}`).value);
  return m;
}

/* ---------- file browser modal ---------- */

const modal = {
  mode: null, // 'input' | 'output'
  path: null,
  selected: new Set(),
  onChoose: null,
};

async function browseTo(path) {
  const url = '/api/browse' + (path ? `?path=${encodeURIComponent(path)}` : '');
  const res = await fetch(url);
  const data = await res.json();
  if (data.error) { alert(data.error); return; }
  modal.path = data.path;
  modal.parent = data.parent;
  modal.selected.clear();
  $('modal-path').value = data.path;
  const list = $('modal-list');
  list.innerHTML = '';
  for (const e of data.entries) {
    if (modal.mode !== 'input' && !e.dir) continue;
    const div = document.createElement('div');
    div.className = 'entry';
    div.innerHTML = `<span class="icon">${e.dir ? '📁' : '🗺'}</span><span>${e.name}</span>`;
    div.addEventListener('click', () => {
      if (e.dir) {
        browseTo(joinPath(modal.path, e.name));
      } else {
        const full = joinPath(modal.path, e.name);
        if (modal.selected.has(full)) modal.selected.delete(full);
        else modal.selected.add(full);
        div.classList.toggle('selected');
        updateModalSelection();
      }
    });
    list.appendChild(div);
  }
  updateModalSelection();
}

function joinPath(dir, name) {
  return dir.endsWith('/') ? dir + name : dir + '/' + name;
}

function updateModalSelection() {
  const n = modal.selected.size;
  $('modal-selection').textContent =
    n > 0 ? `${n} fil${n === 1 ? '' : 'er'} valgt` : 'Ingen filer valgt — «Velg» tar hele mappen.';
  if (modal.mode === 'output') $('modal-selection').textContent = 'Utdata skrives til denne mappen.';
  if (modal.mode === 'project') $('modal-selection').textContent = 'Mappen åpnes som prosjekt.';
}

function openModal(mode, title) {
  modal.mode = mode;
  $('modal-title').textContent = title;
  $('modal').classList.remove('hidden');
  browseTo(mode === 'input' ? null : state.output);
}

function closeModal() { $('modal').classList.add('hidden'); }

$('modal-close').addEventListener('click', closeModal);
$('modal-up').addEventListener('click', () => modal.parent && browseTo(modal.parent));
$('modal-path').addEventListener('keydown', (e) => {
  if (e.key === 'Enter') browseTo($('modal-path').value);
});
$('modal-choose').addEventListener('click', () => {
  if (modal.mode === 'project') {
    openProject(modal.path);
  } else if (modal.mode === 'output') {
    state.output = modal.path;
    $('output-path').innerHTML = `<code>${modal.path}</code>`;
    $('output-path').classList.remove('muted');
  } else {
    state.inputs = modal.selected.size > 0 ? [...modal.selected] : [modal.path];
    $('input-list').innerHTML = state.inputs.map((p) => `<code>${p}</code>`).join('');
    $('input-list').classList.remove('muted');
    scanInputs();
  }
  closeModal();
  updateStartButton();
});

$('pick-input').addEventListener('click', () => openModal('input', 'Velg høydedata'));
$('pick-output').addEventListener('click', () => openModal('output', 'Velg utmappe'));
$('pick-project').addEventListener('click', () => openModal('project', 'Åpne prosjekt'));

/* ---------- open existing project ---------- */

async function openProject(path) {
  const res = await fetch('/api/open', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ path }),
  });
  const data = await res.json();
  if (!res.ok) { alert(data.error || 'kunne ikke åpne prosjektet'); return; }

  state.output = data.output;
  $('output-path').innerHTML = `<code>${data.output}</code>`;
  $('output-path').classList.remove('muted');

  const c = data.config;
  if (c) {
    $('tile-size').value = String(c.tile_size_m);
    $('overlap').checked = c.overlap;
    $('lods').value = c.lods;
    $('lods-val').textContent = c.lods;
    $('threads').value = c.threads;
    $('threads-val').textContent = c.threads;
    $('nodata').value = c.nodata_height;
    for (const f of MASK_FIELDS) {
      $(`mask-${f.key}`).value = c.masks[f.key];
      $(`mask-${f.key}-val`).textContent = c.masks[f.key];
    }
    $('use-ortho').checked = !!c.ortho;
    $('ortho-config').classList.toggle('hidden', !c.ortho);
    if (c.ortho) {
      document.querySelector(`input[name="ortho-kind"][value="${c.ortho.kind}"]`).checked = true;
      for (const kind of ['nib', 'wms', 'xyz']) {
        $(`ortho-${kind}`).classList.toggle('hidden', c.ortho.kind !== kind);
      }
      if (c.ortho.kind === 'nib') $('nib-user').value = c.ortho.username;
      if (c.ortho.kind === 'wms') $('wms-url').value = c.ortho.url;
      if (c.ortho.kind === 'xyz') {
        $('xyz-url').value = c.ortho.url;
        $('zoom').value = c.ortho.zoom;
        $('zoom-val').textContent = c.ortho.zoom;
      }
    }
    state.inputs = c.inputs;
    $('input-list').innerHTML = state.inputs.map((p) => `<code>${p}</code>`).join('');
    $('input-list').classList.remove('muted');
    scanInputs();
  }
  state.hasDataset = data.has_dataset;
  if (data.has_dataset && !state.running) $('open-viewer').classList.remove('hidden');
  updateStartButton();
}

/* ---------- scan + grid preview ---------- */

async function scanInputs() {
  state.scanned = false;
  $('scan-error').classList.add('hidden');
  $('scan-info').classList.remove('hidden');
  $('scan-info').textContent = 'Skanner høydedata…';
  updateStartButton();
  try {
    const res = await fetch('/api/scan', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ paths: state.inputs }),
    });
    const data = await res.json();
    if (!res.ok) throw new Error(data.error || 'skanning feilet');
    if (data.kind === 'zip') {
      $('scan-info').innerHTML =
        `<b>${data.rasters}</b> høydefiler i <b>${data.zips}</b> ZIP-arkiv.<br>
         Arkivet pakkes ut til &lt;utmappe&gt;/source/ ved start; CRS, oppløsning og grid valideres da.`;
    } else {
      const w = data.extent[2] - data.extent[0];
      const h = data.extent[3] - data.extent[1];
      $('scan-info').innerHTML =
        `<b>${data.files}</b> filer &nbsp;•&nbsp; ${data.crs} &nbsp;•&nbsp; ${data.resolution} m/px<br>
         Utstrekning: ${w.toFixed(0)} × ${h.toFixed(0)} m (${data.width_px} × ${data.height_px} px)`;
      updateGridPreview();
    }
    state.scanned = true;
  } catch (err) {
    $('scan-info').classList.add('hidden');
    $('scan-error').classList.remove('hidden');
    $('scan-error').textContent = String(err.message || err);
  }
  updateStartButton();
}

async function updateGridPreview() {
  const q = `tile_size_m=${$('tile-size').value}&lods=${$('lods').value}`;
  try {
    const res = await fetch(`/api/grid?${q}`);
    const data = await res.json();
    $('grid-info').textContent = res.ok
      ? `Fliser: ${data.tiles_x} × ${data.tiles_y} = ${data.count} (${data.tile_px} px per flis)`
      : data.error;
  } catch { /* no scan yet */ }
}

$('tile-size').addEventListener('change', updateGridPreview);
$('lods').addEventListener('input', () => { $('lods-val').textContent = $('lods').value; updateGridPreview(); });
$('threads').addEventListener('input', () => { $('threads-val').textContent = $('threads').value; });
$('zoom').addEventListener('input', () => { $('zoom-val').textContent = $('zoom').value; });

/* ---------- ortho toggles ---------- */

$('use-ortho').addEventListener('change', () => {
  $('ortho-config').classList.toggle('hidden', !$('use-ortho').checked);
});
for (const radio of document.querySelectorAll('input[name="ortho-kind"]')) {
  radio.addEventListener('change', () => {
    for (const kind of ['nib', 'wms', 'xyz']) {
      $(`ortho-${kind}`).classList.toggle('hidden', radio.value !== kind);
    }
  });
}

/* ---------- start / cancel ---------- */

function updateStartButton() {
  const ready = state.scanned && state.inputs.length > 0 && state.output && !state.running;
  $('start').disabled = !ready;
  $('start-hint').classList.toggle('hidden', ready || state.running);
  $('start').classList.toggle('hidden', state.running);
  $('cancel').classList.toggle('hidden', !state.running);
}

$('start').addEventListener('click', async () => {
  const kind = document.querySelector('input[name="ortho-kind"]:checked').value;
  const ortho = !$('use-ortho').checked ? null
    : kind === 'nib' ? { kind: 'nib', username: $('nib-user').value, password: $('nib-pass').value }
    : kind === 'wms' ? { kind: 'wms', url: $('wms-url').value }
    : { kind: 'xyz', url: $('xyz-url').value, zoom: parseInt($('zoom').value) };
  const body = {
    inputs: state.inputs,
    output: state.output,
    tile_size_m: parseFloat($('tile-size').value),
    overlap: $('overlap').checked,
    lods: parseInt($('lods').value),
    threads: parseInt($('threads').value),
    nodata_height: parseFloat($('nodata').value) || 0,
    force: $('force').checked,
    masks: maskValues(),
    ortho,
  };
  const res = await fetch('/api/start', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(body),
  });
  const data = await res.json();
  if (!res.ok) alert(data.error || 'kunne ikke starte');
});

$('cancel').addEventListener('click', () => fetch('/api/cancel', { method: 'POST' }));

/* ---------- progress rendering ---------- */

function setRunning(running) {
  state.running = running;
  const pill = $('run-pill');
  pill.className = 'pill ' + (running ? 'running' : 'idle');
  pill.textContent = running ? 'Kjører' : 'Inaktiv';
  updateStartButton();
}

function setProgress(done, total) {
  const pct = total > 0 ? (100 * done) / total : 0;
  $('progress-fill').style.width = pct.toFixed(1) + '%';
  $('progress-count').textContent = total > 0 ? `${done} / ${total} fliser (${pct.toFixed(1)} %)` : '';

  const now = performance.now();
  state.tileTimes.push([now, done]);
  while (state.tileTimes.length > 2 && now - state.tileTimes[0][0] > 30000) state.tileTimes.shift();
  const [t0, d0] = state.tileTimes[0];
  const dt = (now - t0) / 1000;
  if (dt > 2 && done > d0) {
    const rate = (done - d0) / dt;
    $('progress-rate').textContent = `${rate.toFixed(1)} fliser/s`;
    const eta = (total - done) / rate;
    $('progress-eta').textContent = 'ca. ' + formatEta(eta) + ' igjen';
  }
}

function formatEta(s) {
  if (s > 5400) return (s / 3600).toFixed(1) + ' t';
  if (s > 90) return Math.round(s / 60) + ' min';
  return Math.round(s) + ' s';
}

function appendLog(text, cls = '') {
  const log = $('log');
  if (log.lastChild && log.lastChild.textContent === text) return;
  const div = document.createElement('div');
  if (cls) div.className = cls;
  div.textContent = text;
  log.appendChild(div);
  while (log.childNodes.length > 2000) log.removeChild(log.firstChild);
  log.scrollTop = log.scrollHeight;
}

function showReport(report) {
  const el = $('report');
  el.classList.remove('hidden');
  const ok = !report.missing.length && !report.meta_errors.length && !report.edge_mismatches.length;
  if (ok) {
    el.innerHTML = '<div class="report-ok">✓ Validering OK — ingen hull, ingen sprekker.</div>';
  } else {
    el.innerHTML = `<div class="report-bad">Validering: ${report.missing.length} manglende filer, `
      + `${report.meta_errors.length} metadatafeil, ${report.edge_mismatches.length} kantavvik`
      + `${report.truncated ? ' (avkortet liste)' : ''}</div>`;
    for (const line of [...report.missing, ...report.meta_errors, ...report.edge_mismatches]) {
      appendLog(line, 'err');
    }
  }
}

function applySnapshot(snap) {
  setRunning(snap.running);
  $('status').textContent = snap.status;
  if (snap.total > 0) setProgress(snap.done, snap.total);
  $('log').innerHTML = '';
  for (const line of snap.log) appendLog(line, line.startsWith('⚠') ? 'warn' : '');
  if (snap.report) showReport(snap.report);
}

function handleEvent(ev) {
  switch (ev.type) {
    case 'snapshot':
      applySnapshot(ev.snapshot);
      break;
    case 'started':
      setRunning(true);
      state.tileTimes = [];
      $('log').innerHTML = '';
      $('report').classList.add('hidden');
      $('open-viewer').classList.add('hidden');
      $('progress-fill').style.width = '0%';
      $('progress-count').textContent = $('progress-rate').textContent = $('progress-eta').textContent = '';
      break;
    case 'stage':
      $('status').textContent = ev.text;
      appendLog(ev.text);
      break;
    case 'tile':
      $('status').textContent = `Flis ${ev.done}/${ev.total}  (${ev.name})`;
      setProgress(ev.done, ev.total);
      break;
    case 'warn':
      appendLog('⚠ ' + ev.text, 'warn');
      break;
    case 'cancelled':
      setRunning(false);
      $('status').textContent = ev.text;
      appendLog(ev.text);
      break;
    case 'error':
      setRunning(false);
      $('status').textContent = 'Feil: ' + ev.text;
      appendLog('Feil: ' + ev.text, 'err');
      $('run-pill').className = 'pill error';
      $('run-pill').textContent = 'Feil';
      break;
    case 'finished':
      setRunning(false);
      $('status').textContent = ev.text;
      appendLog(ev.text);
      showReport(ev.report);
      state.hasDataset = true;
      $('open-viewer').classList.remove('hidden');
      break;
  }
}

function connectSse() {
  const es = new EventSource('/api/events');
  es.onmessage = (e) => handleEvent(JSON.parse(e.data));
  es.onerror = () => {
    $('status').textContent = 'Mistet kontakt med serveren — prøver igjen…';
    // EventSource reconnects automatically.
  };
}

/* ---------- tabs + viewer ---------- */

let viewer = null;

async function showViewer() {
  $('tab-viewer').classList.add('active');
  $('tab-pipeline').classList.remove('active');
  $('view-viewer').classList.remove('hidden');
  $('view-pipeline').classList.add('hidden');
  location.hash = 'viewer';
  if (!viewer) {
    const mod = await import('/js/viewer.js');
    viewer = mod;
  }
  viewer.enter();
}

function showPipeline() {
  $('tab-pipeline').classList.add('active');
  $('tab-viewer').classList.remove('active');
  $('view-pipeline').classList.remove('hidden');
  $('view-viewer').classList.add('hidden');
  location.hash = '';
  if (viewer) viewer.leave();
}

$('tab-viewer').addEventListener('click', showViewer);
$('tab-pipeline').addEventListener('click', showPipeline);
$('open-viewer').addEventListener('click', showViewer);

/* ---------- init ---------- */

async function init() {
  const res = await fetch('/api/status');
  const data = await res.json();
  state.defaults = data.defaults;
  state.hasDataset = data.has_dataset;
  buildMaskSliders(data.defaults.masks);
  $('wms-url').value = data.defaults.wms_url;
  $('xyz-url').value = data.defaults.xyz_url;
  if (data.snapshot.output) {
    state.output = data.snapshot.output;
    $('output-path').innerHTML = `<code>${state.output}</code>`;
    $('output-path').classList.remove('muted');
  }
  if (data.has_dataset && !data.snapshot.running) $('open-viewer').classList.remove('hidden');
  applySnapshot(data.snapshot);
  connectSse();
  if (location.hash === '#viewer') showViewer();
}

init();
