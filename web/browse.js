// Server-side folder picker modal. openFolderModal(title) resolves with the
// chosen absolute path, or null if the user closes the dialog.

const $ = (id) => document.getElementById(id);

let current = null; // { resolve, path, parent }

async function browseTo(path) {
  const url = '/api/browse' + (path ? `?path=${encodeURIComponent(path)}` : '');
  const res = await fetch(url);
  const data = await res.json();
  if (data.error) { alert(data.error); return; }
  current.path = data.path;
  current.parent = data.parent;
  $('modal-path').value = data.path;
  const list = $('modal-list');
  list.innerHTML = '';
  for (const e of data.entries) {
    const div = document.createElement('div');
    div.className = 'entry';
    div.innerHTML = `<span class="icon">📁</span><span>${e.name}</span>`;
    div.addEventListener('click', () => browseTo(joinPath(current.path, e.name)));
    list.appendChild(div);
  }
}

function joinPath(dir, name) {
  return dir.endsWith('/') ? dir + name : dir + '/' + name;
}

function close(result) {
  $('modal').classList.add('hidden');
  const { resolve } = current;
  current = null;
  resolve(result);
}

export function openFolderModal(title, startPath = null) {
  return new Promise((resolve) => {
    current = { resolve, path: null, parent: null };
    $('modal-title').textContent = title;
    $('modal').classList.remove('hidden');
    browseTo(startPath);
  });
}

$('modal-close').addEventListener('click', () => current && close(null));
$('modal-up').addEventListener('click', () => current && current.parent && browseTo(current.parent));
$('modal-path').addEventListener('keydown', (e) => {
  if (e.key === 'Enter' && current) browseTo($('modal-path').value);
});
$('modal-choose').addEventListener('click', () => current && close(current.path));
