// App shell: start screen wiring, tabs, SSE progress and log.

import { initStart, showStart, hideStart } from './start.js';

const $ = (id) => document.getElementById(id);

const state = {
  running: false,
  hasDataset: false,
  output: null,
  project: null, // saved world/masks from open/new
  tileTimes: [], // [t_ms, done] for rate/ETA
};

/* ---------- progress rendering ---------- */

function setRunning(running) {
  state.running = running;
  const pill = $('run-pill');
  pill.className = 'pill ' + (running ? 'running' : 'idle');
  pill.textContent = running ? 'Genererer' : 'Inaktiv';
  $('cancel').classList.toggle('hidden', !running);
  $('regen').classList.toggle('hidden', running || !state.project);
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

function showProjectInfo() {
  const el = $('project-info');
  if (!state.output) { el.textContent = 'Ingen prosjekt åpnet.'; return; }
  const w = state.project?.world;
  el.innerHTML = `<code>${state.output}</code>` + (w
    ? `<div class="hint">${(w.size_m / 1000).toFixed(1)} km verden • ${w.tile_size_m} m fliser • `
      + `${w.resolution} m/px • seed ${w.seed}</div>`
    : '');
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
      hideStart();
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
    case 'tiles':
      if (viewer) viewer.onTiles(ev.tiles);
      break;
    case 'scatter':
      if (viewer) viewer.onScatter();
      break;
    case 'conform':
      if (viewer) viewer.onConform();
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

/* ---------- regenerate / cancel ---------- */

$('cancel').addEventListener('click', () => fetch('/api/cancel', { method: 'POST' }));

$('regen').addEventListener('click', async () => {
  if (!state.project?.world || !state.output) return;
  const res = await fetch('/api/project/new', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ output: state.output, world: state.project.world, masks: state.project.masks }),
  });
  const data = await res.json();
  if (!res.ok) alert(data.error || 'kunne ikke starte');
});

/* ---------- tabs + viewer ---------- */

let viewer = null;

async function showViewer() {
  $('tab-viewer').classList.add('active');
  $('tab-project').classList.remove('active');
  $('view-viewer').classList.remove('hidden');
  $('view-project').classList.add('hidden');
  location.hash = 'viewer';
  if (!viewer) {
    viewer = await import('/js/viewer.js');
  }
  viewer.enter();
}

function showProject() {
  $('tab-project').classList.add('active');
  $('tab-viewer').classList.remove('active');
  $('view-project').classList.remove('hidden');
  $('view-viewer').classList.add('hidden');
  location.hash = '';
  if (viewer) viewer.leave();
}

$('tab-viewer').addEventListener('click', showViewer);
$('tab-project').addEventListener('click', showProject);
$('open-viewer').addEventListener('click', showViewer);

/* ---------- init ---------- */

function onProjectOpened({ hasDataset, output, project }) {
  state.hasDataset = hasDataset;
  state.output = output;
  state.project = project;
  showProjectInfo();
  setRunning(state.running);
  if (hasDataset) {
    $('open-viewer').classList.remove('hidden');
    showViewer();
  } else {
    showProject();
  }
}

async function init() {
  const res = await fetch('/api/status');
  const data = await res.json();
  state.hasDataset = data.has_dataset;
  state.output = data.snapshot.output || null;
  initStart(data.defaults, onProjectOpened);
  showProjectInfo();
  applySnapshot(data.snapshot);
  connectSse();
  if (data.has_dataset && !data.snapshot.running) $('open-viewer').classList.remove('hidden');

  if (data.snapshot.running) {
    showProject();
  } else if (location.hash === '#viewer' && data.has_dataset) {
    showViewer();
  } else {
    showStart(!!state.output);
    $('start-continue').onclick = () => {
      hideStart();
      if (state.hasDataset) showViewer();
      else showProject();
    };
  }
}

init();
