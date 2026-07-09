// Editor mode state + toolbar DOM. Modes: fly (pure viewer), sculpt,
// texture, meshes. Keys: 1/2/3 pick an editor mode, Tab toggles fly.

const $ = (id) => document.getElementById(id);

export const editor = {
  mode: 'fly', // 'fly' | 'sculpt' | 'texture' | 'meshes' | 'tomt'
  tool: 'raise', // sculpt: 'raise' | 'lower' | 'flatten' | 'smooth'
  meshTool: 'place', // meshes: 'place' | 'road' | 'scatter'
  radius: 60, // meters; also road width and scatter spacing
  strength: 1.0,
};

let onMode = null;

export function initToolbar(onModeChange) {
  onMode = onModeChange;

  for (const btn of document.querySelectorAll('#toolbar [data-mode]')) {
    btn.addEventListener('click', () => setMode(btn.dataset.mode));
  }
  for (const btn of document.querySelectorAll('#sculpt-tools [data-tool]')) {
    btn.addEventListener('click', () => {
      editor.tool = btn.dataset.tool;
      markActive('#sculpt-tools [data-tool]', 'tool', editor.tool);
    });
  }

  bindSlider('brush-radius', 'brush-radius-val', (v) => { editor.radius = v; }, (v) => v + ' m');
  bindSlider('brush-strength', 'brush-strength-val', (v) => { editor.strength = v; }, (v) => v.toFixed(1));

  document.addEventListener('keydown', (e) => {
    if (e.target.tagName === 'INPUT' || e.target.tagName === 'SELECT') return;
    if (!document.getElementById('view-viewer') || $('view-viewer').classList.contains('hidden')) return;
    if (e.code === 'Tab') {
      e.preventDefault();
      setMode(editor.mode === 'fly' ? (editor.lastEdit || 'sculpt') : 'fly');
    } else if (e.code === 'Digit1') setMode('sculpt');
    else if (e.code === 'Digit2') setMode('texture');
    else if (e.code === 'Digit3') setMode('meshes');
    else if (e.code === 'Digit4') setMode('tomt');
  });

  updatePanels();
}

export function setMode(mode) {
  if (editor.mode === mode) return;
  editor.mode = mode;
  if (mode !== 'fly') editor.lastEdit = mode;
  updatePanels();
  if (onMode) onMode(mode);
}

/// Adjust the brush radius (mouse wheel in edit mode).
export function nudgeRadius(dir) {
  editor.radius = Math.max(4, Math.min(500, editor.radius * (dir > 0 ? 1.15 : 0.87)));
  $('brush-radius').value = editor.radius;
  $('brush-radius-val').textContent = Math.round(editor.radius) + ' m';
}

function updatePanels() {
  markActive('#toolbar [data-mode]', 'mode', editor.mode);
  markActive('#sculpt-tools [data-tool]', 'tool', editor.tool);
  $('sculpt-tools').classList.toggle('hidden', editor.mode !== 'sculpt');
  $('paint-panel').classList.toggle('hidden', editor.mode !== 'texture');
  $('brush-panel').classList.toggle('hidden', editor.mode !== 'sculpt');
  $('meshes-panel').classList.toggle('hidden', editor.mode !== 'meshes');
  $('tomt-panel').classList.toggle('hidden', editor.mode !== 'tomt');
}

function markActive(selector, key, value) {
  for (const btn of document.querySelectorAll(selector)) {
    btn.classList.toggle('active', btn.dataset[key] === value);
  }
}

function bindSlider(id, valId, set, fmt) {
  const el = $(id);
  const update = () => {
    const v = parseFloat(el.value);
    set(v);
    $(valId).textContent = fmt(v);
  };
  el.addEventListener('input', update);
  update();
}
