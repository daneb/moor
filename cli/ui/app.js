/* moor console. Plain JS, no build step, no network beyond 127.0.0.1.
   Sections: core (token, api, state, routing, DOM, markdown) · rail ·
   views · jobs dock · boot. Everything from a sandbox is rendered as
   text; the markdown renderer escapes before it transforms. */
'use strict';

/* ---------- token & api ---------- */
const TOKEN = (() => {
  const m = location.hash.match(/^#([0-9a-f]{48})$/);
  if (m) {
    try { sessionStorage.setItem('moor-token', m[1]); } catch (_) {}
    history.replaceState(null, '', '#/');
    return m[1];
  }
  try { return sessionStorage.getItem('moor-token') || ''; } catch (_) { return ''; }
})();

async function api(path, opts = {}) {
  const headers = { 'x-moor-token': TOKEN };
  let body = opts.body;
  if (body !== undefined && typeof body !== 'string') {
    headers['content-type'] = 'application/json';
    body = JSON.stringify(body);
  } else if (typeof body === 'string') {
    headers['content-type'] = 'text/plain; charset=utf-8';
  }
  const r = await fetch('/api' + path, { method: opts.method || 'GET', headers, body });
  let data = null;
  try { data = await r.json(); } catch (_) { data = { error: 'the console returned something that was not JSON' }; }
  if (!r.ok) {
    const err = new Error(data.error || ('HTTP ' + r.status));
    err.status = r.status;
    throw err;
  }
  return data;
}

/* ---------- state ---------- */
const S = {
  route: { view: 'home' },
  overview: null,
  projects: {},
  specs: {},
  runs: {},
  audits: {},
  settings: {},
  transcripts: {},
  diffs: {},
  evidence: {},
  loading: {},
  errors: {},
  jobs: [],
  jobById: {},
  activeJob: null,
  dockOpen: false,
  ui: {
    node: null,          // selected journey node id
    arm: null,           // {kind:'approve'|'reject', why}
    editing: null,       // {kind, text}
    composer: '',
    role: 'brainstorm',
    newSession: false,
    auditKinds: new Set(),
    evFile: null,
    specForm: null,
    recipeForm: null,
    diffOpen: false,
    liveTurn: null,
  },
};

const PROVIDERS = {
  claude: {
    name: 'Claude Code', vendor: 'Anthropic',
    blurb: 'Ships in every image. Uses a claude.ai subscription token or an API key.',
    secrets: ['CLAUDE_CODE_OAUTH_TOKEN', 'ANTHROPIC_API_KEY'],
    howto: 'On this Mac: `claude setup-token`, then store CLAUDE_CODE_OAUTH_TOKEN below (or ANTHROPIC_API_KEY for metered credits).',
    driver: 'claude-code',
  },
  copilot: {
    name: 'GitHub Copilot', vendor: 'GitHub',
    blurb: 'Enterprise or personal Copilot via the agentic `copilot` CLI, in the moor/copilot images.',
    secrets: ['COPILOT_GITHUB_TOKEN', 'GH_TOKEN'],
    howto: 'On this Mac: `copilot /login` (device flow — a personal access token will not work). moor picks the login up on `up`; or store COPILOT_GITHUB_TOKEN below.',
    driver: 'copilot',
  },
  kiro: {
    name: 'Kiro', vendor: 'AWS',
    blurb: 'Kiro CLI, including IAM Identity Center (enterprise) licences.',
    secrets: [],
    howto: 'On this Mac: `kiro-cli login` (add `--license pro --identity-provider <url> --region <region>` for Identity Center), then `up`.',
    driver: 'kiro',
  },
};

const NODES = [
  { id: 'spec', n: 1, title: 'Spec', sub: 'G0 checks', stages: ['spec'] },
  { id: 'approve-spec', n: 2, title: 'Approve spec', sub: '', stages: ['spec_approval'], decide: 'spec' },
  { id: 'plan', n: 3, title: 'Plan', sub: 'plan + tasks', stages: ['plan'] },
  { id: 'g1', n: 4, title: 'Check plan', sub: 'G1 checks', stages: ['plan_gate'] },
  { id: 'approve-plan', n: 5, title: 'Approve plan', sub: '', stages: ['plan_approval'], decide: 'plan' },
  { id: 'build', n: 6, title: 'Build', sub: 'G2 · G2.5 · G3', stages: ['run'] },
  { id: 'merge', n: 7, title: 'Approve merge', sub: '', stages: ['merge_approval'], decide: 'merge' },
  { id: 'ship', n: 8, title: 'Ship', sub: 'branch · push · PR', stages: ['complete'] },
];

/* ---------- dom ---------- */
function h(tag, attrs, ...kids) {
  const e = document.createElement(tag);
  if (attrs) {
    for (const [k, v] of Object.entries(attrs)) {
      if (v === null || v === undefined || v === false) continue;
      if (k === 'class') e.className = v;
      else if (k === 'html') e.innerHTML = v;
      else if (k.startsWith('on')) e.addEventListener(k.slice(2), v);
      else if (k === 'value') e.value = v;
      else if (k === 'checked') e.checked = !!v;
      else if (k === 'disabled') e.disabled = !!v;
      else e.setAttribute(k, v === true ? '' : v);
    }
  }
  append(e, kids);
  return e;
}
function append(e, kids) {
  for (const k of kids) {
    if (k === null || k === undefined || k === false) continue;
    if (Array.isArray(k)) append(e, k);
    else if (k instanceof Node) e.appendChild(k);
    else e.appendChild(document.createTextNode(String(k)));
  }
}
function clear(e) { while (e.firstChild) e.removeChild(e.firstChild); return e; }
const esc = (s) => String(s).replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));

function btn(label, onclick, cls = '', extra = {}) {
  return h('button', { class: 'btn ' + cls, onclick, ...extra }, label);
}
function chip(text, cls = '') { return h('span', { class: 'chip ' + cls }, text); }
function verdictChip(v) { return chip((v || 'unknown').toUpperCase(), v || ''); }

function fmtTime(iso) {
  if (!iso) return '';
  const d = new Date(iso);
  if (isNaN(d)) return String(iso).slice(0, 19);
  const now = new Date();
  const sameDay = d.toDateString() === now.toDateString();
  const t = d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit' });
  return sameDay ? t : d.toLocaleDateString([], { month: 'short', day: 'numeric' }) + ' ' + t;
}
function ago(iso) {
  if (!iso) return '';
  const s = Math.max(0, (Date.now() - new Date(iso).getTime()) / 1000);
  if (s < 60) return Math.floor(s) + 's ago';
  if (s < 3600) return Math.floor(s / 60) + 'm ago';
  if (s < 86400) return Math.floor(s / 3600) + 'h ago';
  return Math.floor(s / 86400) + 'd ago';
}
function dur(a, b) {
  if (!a) return '';
  const ms = (b ? new Date(b) : new Date()) - new Date(a);
  const s = Math.max(0, Math.floor(ms / 1000));
  return s < 60 ? s + 's' : Math.floor(s / 60) + 'm ' + (s % 60) + 's';
}

/* ---------- toasts & modal ---------- */
function toast(msg, cls = '', ms = 4200) {
  const t = h('div', { class: 'toast ' + cls }, msg);
  document.getElementById('toasts').appendChild(t);
  setTimeout(() => { t.style.opacity = '0'; t.style.transition = 'opacity 0.3s'; setTimeout(() => t.remove(), 320); }, ms);
}
function modal(content) {
  const root = document.getElementById('modal');
  clear(root);
  root.classList.remove('hidden');
  root.appendChild(h('div', { class: 'modal', onclick: (e) => e.stopPropagation() }, content));
  root.onclick = closeModal;
}
function closeModal() {
  const root = document.getElementById('modal');
  root.classList.add('hidden');
  clear(root);
}

/* ---------- markdown (escape first, then transform) ---------- */
function inline(s) {
  s = esc(s);
  s = s.replace(/`([^`]+)`/g, '<code>$1</code>');
  s = s.replace(/\*\*([^*]+)\*\*/g, '<strong>$1</strong>');
  s = s.replace(/(^|[^*\w])\*([^*\n]+)\*(?!\w)/g, '$1<em>$2</em>');
  s = s.replace(/\[([^\]]+)\]\((https?:\/\/[^)\s]+)\)/g, '<a href="$2" target="_blank" rel="noopener noreferrer">$1</a>');
  return s;
}
function md(text) {
  if (!text) return '';
  const lines = String(text).replace(/\r/g, '').split('\n');
  const out = [];
  let i = 0;
  // front matter
  if (lines[0] === '---') {
    const end = lines.indexOf('---', 1);
    if (end > 0) {
      const rows = [];
      let key = null;
      for (const l of lines.slice(1, end)) {
        const m = l.match(/^([A-Za-z_][\w-]*):\s*(.*)$/);
        if (m) { key = m[1]; rows.push([key, m[2]]); }
        else if (/^\s+-\s/.test(l) && rows.length) { rows[rows.length - 1][1] += (rows[rows.length - 1][1] ? ', ' : '') + l.replace(/^\s+-\s*/, '').replace(/^"|"$/g, ''); }
        else if (/^\s+\w+:/.test(l) && rows.length) { rows[rows.length - 1][1] += (rows[rows.length - 1][1] ? ' · ' : '') + l.trim(); }
      }
      out.push('<div class="fm">' + rows.map(([k, v]) => `<b>${esc(k)}</b><span>${esc(v.replace(/^"|"$/g, ''))}</span>`).join('') + '</div>');
      i = end + 1;
    }
  }
  let list = null; // 'ul' | 'ol'
  const closeList = () => { if (list) { out.push(`</${list}>`); list = null; } };
  let para = [];
  const flushPara = () => { if (para.length) { out.push('<p>' + para.map(inline).join('<br>') + '</p>'); para = []; } };
  while (i < lines.length) {
    let l = lines[i];
    if (/^\s*```/.test(l)) {
      flushPara(); closeList();
      const buf = [];
      i++;
      while (i < lines.length && !/^\s*```/.test(lines[i])) { buf.push(lines[i]); i++; }
      i++;
      out.push('<pre><code>' + esc(buf.join('\n')) + '</code></pre>');
      continue;
    }
    const hm = l.match(/^(#{1,6})\s+(.*)$/);
    if (hm) {
      flushPara(); closeList();
      const lvl = Math.min(hm[1].length, 3);
      let t = inline(hm[2]);
      t = t.replace(/^(AC-\d+|T-\d+)\b/, '<span class="ac">$1</span>');
      out.push(`<h${lvl}>${t}</h${lvl}>`);
      i++; continue;
    }
    if (/^\s*oracle:\s/.test(l)) {
      flushPara(); closeList();
      out.push('<span class="oracle">' + inline(l.trim()) + '</span>');
      i++; continue;
    }
    if (/^\s*(---+|\*\*\*+)\s*$/.test(l)) { flushPara(); closeList(); out.push('<hr>'); i++; continue; }
    if (/^\s*>/.test(l)) {
      flushPara(); closeList();
      const buf = [];
      while (i < lines.length && /^\s*>/.test(lines[i])) { buf.push(lines[i].replace(/^\s*>\s?/, '')); i++; }
      out.push('<blockquote>' + buf.map(inline).join('<br>') + '</blockquote>');
      continue;
    }
    if (/^\s*\|/.test(l) && i + 1 < lines.length && /^\s*\|?\s*:?-{2,}/.test(lines[i + 1])) {
      flushPara(); closeList();
      const cells = (row) => row.trim().replace(/^\||\|$/g, '').split('|').map((c) => inline(c.trim()));
      const head = cells(l);
      i += 2;
      const rows = [];
      while (i < lines.length && /^\s*\|/.test(lines[i])) { rows.push(cells(lines[i])); i++; }
      out.push('<table><thead><tr>' + head.map((c) => `<th>${c}</th>`).join('') + '</tr></thead><tbody>' +
        rows.map((r) => '<tr>' + r.map((c) => `<td>${c}</td>`).join('') + '</tr>').join('') + '</tbody></table>');
      continue;
    }
    const lm = l.match(/^\s*([-*+]|\d+\.)\s+(.*)$/);
    if (lm) {
      flushPara();
      const kind = /\d/.test(lm[1]) ? 'ol' : 'ul';
      if (list !== kind) { closeList(); list = kind; out.push(`<${kind}>`); }
      let body = lm[2];
      let cls = '';
      if (/^\[x\]/i.test(body)) { cls = ' class="task-done"'; body = '✓ ' + body.slice(3); }
      else if (/^\[ \]/.test(body)) { cls = ' class="task-open"'; body = '○ ' + body.slice(3); }
      out.push(`<li${cls}>${inline(body)}</li>`);
      i++; continue;
    }
    if (l.trim() === '') { flushPara(); closeList(); i++; continue; }
    if (list && /^\s{2,}/.test(l)) { out[out.length - 1] = out[out.length - 1].replace(/<\/li>$/, ' ' + inline(l.trim()) + '</li>'); i++; continue; }
    para.push(l);
    i++;
  }
  flushPara(); closeList();
  return out.join('\n');
}
function mdView(text) { return h('div', { class: 'md', html: md(text) }); }

function diffView(patch) {
  const pre = h('pre', { class: 'raw' });
  for (const line of String(patch || '').split('\n')) {
    let cls = '';
    if (/^(\+\+\+|---|diff --git)/.test(line)) cls = 'file';
    else if (line.startsWith('+')) cls = 'add';
    else if (line.startsWith('-')) cls = 'del';
    else if (line.startsWith('@@')) cls = 'hunk';
    pre.appendChild(h('span', { class: cls }, line + '\n'));
  }
  return pre;
}

/* ---------- routing ---------- */
function parseRoute() {
  const hash = location.hash.replace(/^#\/?/, '');
  const seg = hash.split('/').filter(Boolean).map(decodeURIComponent);
  if (seg.length === 0) return { view: 'home' };
  if (seg[0] === 'new') return { view: 'new' };
  if (seg[0] === 'import') return { view: 'import' };
  if (seg[0] === 'p' && seg[1]) {
    const r = { view: 'project', project: seg[1], tab: 'journey' };
    if (seg[2] === 'spec' && seg[3]) { r.slug = seg[3]; r.node = seg[4] || null; }
    else if (seg[2] === 'runs') { r.tab = 'runs'; r.run = seg[3] || null; }
    else if (seg[2]) r.tab = seg[2];
    return r;
  }
  return { view: 'home' };
}
function go(path) { location.hash = '#/' + path; }
function routeFor(r) {
  if (r.view !== 'project') return '#/' + (r.view === 'home' ? '' : r.view);
  let p = '#/p/' + r.project;
  if (r.slug) { p += '/spec/' + r.slug; if (r.node) p += '/' + r.node; }
  else if (r.tab === 'runs') { p += '/runs'; if (r.run) p += '/' + r.run; }
  else if (r.tab && r.tab !== 'journey') p += '/' + r.tab;
  return p;
}
window.addEventListener('hashchange', () => {
  const prev = S.route;
  S.route = parseRoute();
  if (prev.project !== S.route.project || prev.slug !== S.route.slug) { S.ui.arm = null; S.ui.editing = null; S.ui.diffOpen = false; }
  render();
});

/* ---------- loading ---------- */
async function load(key, path, store, force = false) {
  if (!force && store[key] !== undefined && store[key] !== null) return store[key];
  if (S.loading[key]) return null;
  S.loading[key] = true;
  delete S.errors[key];
  try {
    const v = await api(path);
    store[key] = v;
  } catch (e) {
    S.errors[key] = e.message;
  } finally {
    delete S.loading[key];
    render();
  }
  return store[key];
}
const loadOverview = (force) => load('overview', '/overview', { get overview() { return S.overview; }, set overview(v) { S.overview = v; } }, force);
const loadProject = (p, force) => load(p, '/projects/' + p, S.projects, force);
const loadSpec = (p, slug, force) => load(p + '/' + slug, `/projects/${p}/specs/${slug}`, S.specs, force);
const loadRun = (p, id, force) => load(p + '/' + id, `/projects/${p}/runs/${id}`, S.runs, force);
const loadAudit = (p, force) => load(p, `/projects/${p}/audit?limit=400`, S.audits, force);
const loadSettings = (p, force) => load(p, `/projects/${p}/settings`, S.settings, force);
const loadTranscript = (p, force) => load(p, `/projects/${p}/transcript`, S.transcripts, force);
const loadDiff = (p, slug, force) => load(p + '/' + slug, `/projects/${p}/specs/${slug}/diff`, S.diffs, force);

function invalidate(project) {
  // Refetch in place: what is on screen stays until the fresh copy lands,
  // so a finished job does not flash skeletons over a page being read.
  loadOverview(true);
  if (!project) return;
  if (S.projects[project]) loadProject(project, true); else delete S.projects[project];
  for (const store of [S.specs, S.runs, S.diffs, S.evidence]) {
    for (const k of Object.keys(store)) if (k.startsWith(project + '/')) delete store[k];
  }
  delete S.audits[project];
  delete S.settings[project];
  delete S.transcripts[project];
}
function refresh() {
  S.loading = {};
  invalidate(S.route.project);
  render();
}
/* ---------- rail ---------- */
function stepIndex(spec, unshipped) {
  if (!spec) return 0;
  if (spec.complete) return (unshipped || []).includes(spec.slug) ? 8 : 9;
  const n = NODES.findIndex((nd) => nd.stages.includes(spec.stage));
  return n >= 0 ? n + 1 : 0;
}
function miniBar(spec, unshipped, cls = 'minibar') {
  const cur = stepIndex(spec, unshipped);
  const bar = h('span', { class: cls });
  for (let i = 1; i <= 8; i++) {
    const c = i < cur ? 'done' : i === cur ? 'now' : '';
    bar.appendChild(h('i', { class: c }));
  }
  return bar;
}
function needsYou(p) {
  const s = p.active_spec;
  if (p.blockers && p.blockers.length) return 'blocked: ' + p.blockers[0].reason;
  if (s && s.pending_decision) return 'approve the ' + s.pending_decision;
  if ((p.unshipped || []).length) return 'ship ' + p.unshipped[0];
  return null;
}

function renderRail() {
  const rail = clear(document.getElementById('rail'));
  rail.appendChild(h('div', { class: 'brand', onclick: () => go('') }, h('span', { class: 'dot' }), 'moor', h('span', { class: 'ver' }, 'console')));
  const list = h('div', { class: 'rail-list' });
  const ov = S.overview;
  rail.appendChild(h('div', { class: 'rail-label' }, 'Projects', h('button', { class: 'btn ghost sm', onclick: () => go('new'), title: 'New or import' }, '+')));
  if (!ov) {
    list.appendChild(h('div', { class: 'skeleton' }));
    list.appendChild(h('div', { class: 'skeleton' }));
  } else {
    const sorted = [...ov.projects].sort((a, b) => (needsYou(b) ? 1 : 0) - (needsYou(a) ? 1 : 0) || (b.up - a.up) || a.name.localeCompare(b.name));
    for (const p of sorted) {
      const active = S.route.project === p.name;
      const need = needsYou(p);
      list.appendChild(h('div', { class: 'proj' + (active ? ' active' : ''), onclick: () => go('p/' + p.name) },
        h('span', { class: 'led' + (p.up ? ' up' : '') }),
        h('span', { class: 'name ellipsis' }, p.name),
        need ? h('span', { class: 'needs', title: need }) : h('span', { class: 'agent' }, p.agent),
        h('span', { class: 'sub' },
          p.up ? (p.active_spec ? [miniBar(p.active_spec, p.unshipped), h('span', { class: 'ellipsis' }, p.active_spec.slug)] : h('span', {}, p.error ? 'status unavailable' : 'no spec in progress')) : h('span', {}, 'down'))));
    }
    if (!ov.projects.length) list.appendChild(h('div', { class: 'empty' }, h('strong', {}, 'No projects yet'), 'Create one or import a repo.'));
  }
  rail.appendChild(list);
  rail.appendChild(h('div', { class: 'rail-foot' },
    h('button', { class: 'btn ghost sm', onclick: toggleTheme, title: 'Theme' }, '◐'),
    h('button', { class: 'btn ghost sm', onclick: refresh, title: 'Refresh (r)' }, '↻'),
    h('span', { class: 'right' }, h('span', { class: 'kbd' }, '⌘K'), ' actions')));
}
function toggleTheme() {
  const cur = document.documentElement.getAttribute('data-theme') === 'light' ? 'dark' : 'light';
  document.documentElement.setAttribute('data-theme', cur);
  try { localStorage.setItem('moor-theme', cur); } catch (_) {}
}

/* ---------- views: home ---------- */
function viewHome() {
  const ov = S.overview;
  if (!ov) { loadOverview(); return [h('div', { class: 'skeleton' }), h('div', { class: 'skeleton' })]; }
  const needs = ov.projects.filter((p) => p.up && needsYou(p));
  const out = [];
  out.push(h('div', { class: 'hero' },
    h('div', {}, h('h1', {}, 'Harbour'), h('p', {}, `${ov.projects.length} project${ov.projects.length === 1 ? '' : 's'} · ${ov.projects.filter((p) => p.up).length} up · ${needs.length} waiting on you`)),
    h('div', { class: 'row' }, btn('Import a repo', () => go('import')), btn('New project', () => go('new'), 'primary'))));
  if (needs.length) {
    out.push(h('div', { class: 'card' }, h('h3', {}, 'Needs you'), h('div', { class: 'needs-lane' },
      needs.map((p) => {
        const s = p.active_spec;
        return h('div', { class: 'need', onclick: () => go('p/' + p.name + (s ? '/spec/' + s.slug : '')) },
          h('span', { class: 'big-dot' }, chip(p.name, 'yellow')),
          h('div', {}, h('div', { class: 'who' }, s ? s.slug : p.name), h('div', { class: 'what' }, s ? s.label : needsYou(p))),
          h('span', { class: 'row' }, chip(needsYou(p), 'warn'), '→'));
      }))));
  }
  if (!ov.projects.length) {
    out.push(h('div', { class: 'empty' }, h('strong', {}, 'No projects yet'), 'A project is one sealed sandbox around one repository. Create one from scratch, or import a repo you already have.'));
    return out;
  }
  out.push(h('div', { class: 'grid three' }, ov.projects.map((p) => {
    const s = p.active_spec;
    const need = needsYou(p);
    return h('div', { class: 'card pcard' + (need ? ' needs' : ''), onclick: () => go('p/' + p.name) },
      h('div', { class: 'head' }, h('span', { class: 'led' + (p.up ? ' up' : '') }), h('span', { class: 'pname' }, p.name), h('span', { class: 'right chip' }, p.agent), p.is_default ? chip('default') : null),
      h('div', { class: 'img' }, p.image, p.github_repo ? ' · ' + p.github_repo : ''),
      p.up ? (s ? h('div', { class: 'spec' }, h('div', { class: 'row' }, miniBar(s, p.unshipped, 'steps'), h('span', { class: 's' }, s.slug)), h('div', { class: 'l' }, s.label + (p.open_specs > 1 ? ` · ${p.open_specs - 1} more in progress` : '')))
        : h('div', { class: 'spec' }, h('div', { class: 'l' }, p.error ? 'Pipeline status unavailable' : `No spec in progress · ${p.spec_count || 0} done`)))
        : h('div', { class: 'spec' }, h('div', { class: 'l' }, 'Sandbox is down')),
      h('div', { class: 'foot' }, need ? chip(need, 'warn') : (p.up ? chip('idle') : null),
        h('span', { class: 'right' }), p.up ? btn('Open', (e) => { e.stopPropagation(); go('p/' + p.name); }, 'sm') : btn('Up', (e) => { e.stopPropagation(); startJob({ kind: 'up', project: p.name }); }, 'sm primary')));
  })));
  return out;
}

/* ---------- views: new & import ---------- */
function imageOptions(images, current) {
  const base = ['moor/base:latest', 'moor/node:latest', 'moor/rust:latest', 'moor/python:latest'];
  const all = Array.from(new Set([...(images || []), ...base]));
  return all.map((i) => h('option', { value: i, selected: i === current }, i));
}
function viewNew() {
  const f = S.ui.newForm || (S.ui.newForm = { name: '', image: 'moor/base:latest', agent: 'claude', github: false });
  return [
    h('div', { class: 'hero' }, h('div', {}, h('h1', {}, 'New project'), h('p', {}, 'A fresh repository in its own sealed sandbox. Nothing on this Mac is mounted into it.'))),
    h('div', { class: 'card', style: 'max-width:640px' },
      h('div', { class: 'col' },
        h('div', { class: 'field' }, h('label', {}, 'Name'), h('input', { placeholder: 'my-app', value: f.name, oninput: (e) => f.name = e.target.value.trim() }), h('span', { class: 'hint' }, 'Lowercase letters, digits and dashes. Also the container and volume names.')),
        h('div', { class: 'field' }, h('label', {}, 'Image'), h('select', { onchange: (e) => f.image = e.target.value }, imageOptions(S.images, f.image)), h('span', { class: 'hint' }, 'The toolchain inside the sandbox. Copilot projects compose with the moor/copilot layer automatically.')),
        h('div', { class: 'field' }, h('label', {}, 'Agent'), h('div', { class: 'segments' }, Object.keys(PROVIDERS).map((a) => h('button', { class: f.agent === a ? 'on' : '', onclick: (e) => { f.agent = a; render(); } }, PROVIDERS[a].name)))),
        h('label', { class: 'row' }, h('input', { type: 'checkbox', checked: f.github, onchange: (e) => f.github = e.target.checked }), 'Also create a private GitHub repo with gh (you are confirming that here)'),
        h('div', { class: 'row' }, btn('Cancel', () => go(''), 'ghost'), h('span', { class: 'right' }), btn('Create project', () => {
          if (!/^[a-z0-9][a-z0-9-]*$/.test(f.name)) return toast('Give the project a lowercase name with dashes.', 'warn');
          startJob({ kind: 'new', name: f.name, image: f.image, agent: f.agent, github: f.github });
          S.ui.newForm = null; go('');
        }, 'primary')))),
  ];
}
function viewImport() {
  const f = S.ui.importForm || (S.ui.importForm = { name: '', from: '', image: '', agent: 'claude', github: false, dir: null });
  if (!f.dir && !S.loading.fs) {
    S.loading.fs = true;
    api('/fs').then((d) => { f.dir = d; }).catch((e) => toast(e.message, 'err')).finally(() => { delete S.loading.fs; render(); });
  }
  const browse = (path) => { S.loading.fs = true; api('/fs?path=' + encodeURIComponent(path)).then((d) => { f.dir = d; }).catch((e) => toast(e.message, 'err')).finally(() => { delete S.loading.fs; render(); }); };
  const d = f.dir;
  return [
    h('div', { class: 'hero' }, h('div', {}, h('h1', {}, 'Import a repository'), h('p', {}, 'Its full history crosses as a one-shot git bundle. The source folder is never mounted.'))),
    h('div', { class: 'grid two' },
      h('div', { class: 'card' },
        h('h3', {}, 'Pick the repo'),
        d ? h('div', { class: 'col' },
          h('div', { class: 'row mono small' }, btn('↑', () => d.parent && browse(d.parent), 'sm', { disabled: !d.parent }), btn('~', () => browse(d.home), 'sm'), h('span', { class: 'ellipsis grow' }, d.path)),
          h('div', { class: 'fs' }, d.dirs.map((x) => h('div', { class: 'fsrow' + (x.git ? ' git' : ''), onclick: () => { if (x.git) { f.from = x.path; if (!f.name) f.name = x.name.toLowerCase().replace(/[^a-z0-9-]+/g, '-').replace(/^-|-$/g, ''); render(); } else browse(x.path); } },
            h('span', {}, (x.git ? '◆ ' : '▸ ') + x.name), h('span', { class: 'tag' }, x.kind), h('span', { class: 'tag' }, x.git ? 'git' : '')))),
          h('span', { class: 'hint small' }, '◆ a git repository (click to choose) · ▸ a folder (click to open)')) : h('div', { class: 'skeleton' })),
      h('div', { class: 'card' },
        h('h3', {}, 'Project'),
        h('div', { class: 'col' },
          h('div', { class: 'field' }, h('label', {}, 'Source path'), h('input', { value: f.from, placeholder: '/Users/you/Repos/my-service', oninput: (e) => f.from = e.target.value.trim() })),
          h('div', { class: 'field' }, h('label', {}, 'Name'), h('input', { value: f.name, placeholder: 'my-service', oninput: (e) => f.name = e.target.value.trim() })),
          h('div', { class: 'field' }, h('label', {}, 'Image'), h('select', { onchange: (e) => f.image = e.target.value }, h('option', { value: '', selected: !f.image }, 'auto-detect from the repo'), imageOptions(S.images, f.image))),
          h('div', { class: 'field' }, h('label', {}, 'Agent'), h('div', { class: 'segments' }, Object.keys(PROVIDERS).map((a) => h('button', { class: f.agent === a ? 'on' : '', onclick: () => { f.agent = a; render(); } }, PROVIDERS[a].name)))),
          h('label', { class: 'row' }, h('input', { type: 'checkbox', checked: f.github, onchange: (e) => f.github = e.target.checked }), 'Create a private GitHub repo if it has no remote'),
          h('div', { class: 'row' }, btn('Cancel', () => go(''), 'ghost'), h('span', { class: 'right' }), btn('Import', () => {
            if (!f.from) return toast('Pick the repository to import.', 'warn');
            if (!/^[a-z0-9][a-z0-9-]*$/.test(f.name)) return toast('Give the project a lowercase name with dashes.', 'warn');
            startJob({ kind: 'import', name: f.name, from: f.from, image: f.image || undefined, agent: f.agent, github: f.github });
            S.ui.importForm = null; go('');
          }, 'primary'))))),
  ];
}

/* ---------- views: project ---------- */
function projectTabs(p, r) {
  const tabs = [['journey', 'Journey'], ['specs', 'Specs'], ['agent', 'Agent'], ['runs', 'Runs & evidence'], ['audit', 'Audit'], ['settings', 'Settings']];
  const cur = r.slug ? 'journey' : r.tab;
  return h('div', { class: 'tabs' }, tabs.map(([id, label]) => h('button', { class: 'tab' + (cur === id ? ' active' : ''), onclick: () => go('p/' + r.project + (id === 'journey' ? '' : '/' + id)) }, label,
    id === 'specs' && p && p.report ? h('span', { class: 'count' }, p.report.specs.filter((s) => !s.complete).length) : null)));
}
function activeSpecOf(p, r) {
  if (!p || !p.report) return null;
  const slug = r.slug || (p.active && p.active.slug);
  return p.report.specs.find((s) => s.slug === slug) || null;
}

function viewProject(r) {
  const p = S.projects[r.project];
  if (p === undefined) { loadProject(r.project); return [h('div', { class: 'skeleton' }), h('div', { class: 'skeleton' })]; }
  if (S.errors[r.project]) return [h('div', { class: 'banner fail' }, h('div', {}, h('b', {}, 'Could not load ' + r.project), h('div', { class: 'small' }, S.errors[r.project])), h('span', { class: 'right' }), btn('Retry', () => { delete S.errors[r.project]; loadProject(r.project, true); }, 'sm'))];
  const tab = r.slug ? 'journey' : r.tab;
  switch (tab) {
    case 'specs': return viewSpecs(p, r);
    case 'agent': return viewAgent(p, r);
    case 'runs': return viewRuns(p, r);
    case 'audit': return viewAudit(p, r);
    case 'settings': return viewSettings(p, r);
    default: return viewJourney(p, r);
  }
}

function downBanner(p) {
  return h('div', { class: 'banner warn' }, h('div', { class: 'grow' }, h('b', {}, `${p.name} is down.`), ' Its sandbox has to be running before moor can say where things stand or do anything inside it.'),
    btn('Start sandbox', () => startJob({ kind: 'up', project: p.name }), 'primary'));
}

/* ----- journey ----- */
function nodeStates(spec, specDetail, unshipped) {
  const cur = stepIndex(spec, unshipped);
  const gates = (specDetail && specDetail.report && specDetail.report.gates) || [];
  const byGate = {};
  for (const g of gates) byGate[g.gate] = g;
  const runs = (specDetail && specDetail.report && specDetail.report.runs) || [];
  const latest = runs.length ? runs[runs.length - 1] : null;
  return NODES.map((nd) => {
    let state = nd.n < cur ? 'done' : nd.n === cur ? 'now' : 'todo';
    if (spec && spec.complete && cur === 9) state = 'done';
    let sub = nd.sub;
    if (nd.id === 'spec' && byGate.G0) { sub = 'G0 ' + byGate.G0.verdict; if (byGate.G0.verdict === 'fail' && state !== 'done') state = 'bad'; }
    if (nd.id === 'g1' && byGate.G1) { sub = 'G1 ' + byGate.G1.verdict; if (byGate.G1.verdict === 'fail' && state !== 'done') state = 'bad'; }
    if (nd.id === 'build' && latest) { sub = `${runs.length} run${runs.length === 1 ? '' : 's'} · last ${latest.verdict || '?'}`; if (latest.verdict === 'fail' && state === 'now') state = 'bad'; }
    if (nd.decide && spec && spec.approval && spec.approval.stage === nd.decide && state === 'now') { sub = spec.approval.standing; if (spec.approval.standing !== 'absent') state = 'wait'; }
    if (nd.id === 'ship' && spec && spec.complete) sub = cur === 9 ? 'shipped' : 'ready to ship';
    return { ...nd, state, sub };
  });
}

function viewJourney(p, r) {
  if (!p.up) return [projectHeader(p, r), downBanner(p), projectFacts(p)];
  const spec = activeSpecOf(p, r);
  const out = [projectHeader(p, r)];
  if (p.report && p.report.blockers.length) {
    for (const b of p.report.blockers) {
      if (b.reason === 'no specs') continue;
      out.push(h('div', { class: 'banner fail' }, h('div', {}, h('b', {}, 'Blocked: ' + b.reason), h('div', { class: 'mono small' }, 'Fix: moor keel ' + b.command.replace(/^keel\s+/, '')))));
    }
  }
  if (!spec) {
    out.push(h('div', { class: 'nowcard' }, h('div', { class: 'big wait' }, '—'),
      h('div', {}, h('div', { class: 'stepno' }, p.name), h('div', { class: 'title' }, 'No spec in progress'), h('div', { class: 'what' }, 'Start one: give it a short name, write what the change must do, then send it in. Or describe the outcome loosely and let a recipe drive the whole pipeline.')),
      h('div', { class: 'actions' }, btn('Brainstorm with the agent', () => go('p/' + p.name + '/agent')), btn('Write a recipe', () => { S.ui.recipeForm = S.ui.recipeForm || {}; go('p/' + p.name + '/specs'); }), btn('New spec', () => { S.ui.specForm = S.ui.specForm || { step: 1 }; go('p/' + p.name + '/specs'); }, 'primary'))));
    out.push(projectFacts(p));
    return out;
  }
  const key = p.name + '/' + spec.slug;
  const detail = S.specs[key];
  if (detail === undefined) loadSpec(p.name, spec.slug);
  const states = nodeStates(spec, detail, p.unshipped);
  const cur = stepIndex(spec, p.unshipped);
  const selected = r.node || S.ui.node || (states.find((n) => n.state === 'now' || n.state === 'bad' || n.state === 'wait') || states[7]).id;

  out.push(nowCard(p, spec, cur, r));
  const map = h('div', { class: 'card journey' }, h('div', { class: 'map' }, states.map((nd, idx) => h('div', { class: 'node ' + nd.state + (selected === nd.id ? ' selected' : ''), onclick: () => { S.ui.node = nd.id; S.ui.arm = null; S.ui.editing = null; go('p/' + p.name + '/spec/' + spec.slug + '/' + nd.id); } },
    h('span', { class: 'line' + (idx + 1 < cur ? ' done' : '') }),
    h('span', { class: 'ring' }, nd.state === 'done' ? '✓' : nd.state === 'bad' ? '!' : String(nd.n)),
    h('span', { class: 'nt' }, nd.title), nd.decide ? h('span', { class: 'decide' }, 'your call') : null,
    h('span', { class: 'ns' }, nd.sub)))));
  out.push(map);
  out.push(nodeDetail(p, spec, detail, states.find((n) => n.id === selected), r));
  return out;
}

function projectHeader(p, r) {
  const other = p.report ? p.report.specs.filter((s) => !s.complete) : [];
  return h('div', { class: 'row wrap' },
    h('span', { class: 'led' + (p.up ? ' up' : '') }), h('span', { style: 'font-weight:700;font-size:18px;letter-spacing:-0.02em' }, p.name),
    chip(p.agent), h('span', { class: 'mono small dim' }, p.image), p.github_repo ? h('a', { class: 'mono small', href: 'https://github.com/' + p.github_repo, target: '_blank', rel: 'noopener noreferrer' }, p.github_repo) : null,
    p.git ? h('span', { class: 'mono small dim', title: 'git state in the sandbox' }, `${p.git.branch || 'detached'} · ${p.git.dirty ? p.git.dirty + ' changed' : 'clean'}${p.git.behind ? ' · ' + p.git.behind + ' behind' : ''}${p.git.ahead ? ' · ' + p.git.ahead + ' ahead' : ''}`) : null,
    h('span', { class: 'right' }),
    p.up && other.length > 1 && (r.tab === 'journey' || r.slug) ? h('select', { class: 'btn sm', onchange: (e) => go('p/' + p.name + '/spec/' + e.target.value) }, other.map((s) => h('option', { value: s.slug, selected: s.slug === (r.slug || (p.active && p.active.slug)) }, s.slug + ' · ' + s.label))) : null,
    p.up ? btn('Stop', () => startJob({ kind: 'down', project: p.name }), 'sm ghost') : btn('Start', () => startJob({ kind: 'up', project: p.name }), 'sm primary'),
    btn('↻', () => refresh(), 'sm ghost', { title: 'Refresh' }));
}
function projectFacts(p) {
  return h('div', { class: 'grid three' },
    h('div', { class: 'card' }, h('h3', {}, 'Sandbox'), h('div', { class: 'mono small' }, p.sandbox_container), h('div', { class: 'small muted' }, `cpu ${p.resources.cpu} · mem ${p.resources.mem} · pids ${p.resources.pids}`), h('div', { class: 'small muted' }, p.egress.length ? 'egress: ' + p.egress.join(', ') : 'egress: default deny, no allowlist')),
    h('div', { class: 'card' }, h('h3', {}, 'Credentials'), (p.secrets_status || []).map((s) => h('div', { class: 'row small' }, h('span', { class: 'mono' }, s.name), h('span', { class: 'right' }), chip(s.source, s.source === 'missing' ? 'warn' : 'pass')))),
    h('div', { class: 'card' }, h('h3', {}, 'Attestation'), p.posture ? postureView(p.posture, true) : h('div', { class: 'small muted' }, 'No attestation yet — written on the next start.')));
}

function nowCard(p, spec, cur, r) {
  const st = spec.step;
  const dec = spec.pending_decision;
  const appr = spec.approval;
  const busy = S.jobs.find((j) => j.project === p.name && !j.done && ['go', 'approve', 'reject', 'ship', 'recipe', 'spec_push', 'gate'].includes(j.kind));
  const actions = [];
  const open = (node) => go('p/' + p.name + '/spec/' + spec.slug + '/' + node);
  let title, what, badge = String(cur), badgeCls = '';
  if (spec.complete && cur === 8) {
    title = 'Approved. Ship it.'; what = 'The merge is approved and the work is still uncommitted in the sandbox. Shipping commits it on its own branch, pushes from this Mac with your GitHub login, and opens a pull request.';
    badge = '✓'; badgeCls = 'done';
    actions.push(btn('Review & ship', () => open('ship'), 'primary'));
  } else if (spec.complete) {
    title = 'Done'; what = 'This spec went all the way through and has been shipped.'; badge = '✓'; badgeCls = 'done';
  } else if (!st) {
    title = 'At ' + spec.stage; what = 'A stage this console does not know; use the command line for it.'; badge = '?';
  } else {
    title = st.title.charAt(0).toUpperCase() + st.title.slice(1);
    what = st.what;
    if (appr && appr.standing === 'rejected') what = `Rejected${appr.by ? ' by ' + appr.by : ''}${appr.note ? ': “' + appr.note + '”' : ''}. Revise the ${appr.stage}, then re-check it; the earlier decision stays on record until you approve the revision.`;
    if (appr && appr.standing === 'superseded') what = `The ${appr.stage} changed after it was approved. Re-check it, then approve the revision.`;
    if (dec) {
      actions.push(btn('Reject…', () => { S.ui.arm = { kind: 'reject', why: '' }; open(NODES.find((n) => n.decide === dec).id); }, 'danger'));
      actions.push(btn('Review & approve', () => open(NODES.find((n) => n.decide === dec).id), 'primary'));
    } else if (spec.automatic) {
      if (spec.stage === 'run') {
        actions.push(btn('Re-check without the agent', () => startJob({ kind: 'go', project: p.name, check: true }), '', { disabled: !!busy }));
        actions.push(btn(busy ? [h('span', { class: 'spin' }), 'Building…'] : 'Build it', () => startJob({ kind: 'go', project: p.name }), 'primary', { disabled: !!busy }));
      } else {
        actions.push(btn(busy ? [h('span', { class: 'spin' }), 'Running…'] : 'Run the next step', () => startJob({ kind: 'go', project: p.name }), 'primary', { disabled: !!busy }));
      }
    }
  }
  const pinned = p.active && p.active.pin === spec.slug;
  return h('div', { class: 'nowcard' },
    h('div', { class: 'big ' + badgeCls }, badge),
    h('div', {}, h('div', { class: 'stepno' }, `${p.name} · ${spec.slug} · ${spec.complete ? 'done' : 'step ' + (st ? st.n : '?') + ' of 7'}${pinned ? ' · pinned' : p.active && p.active.slug === spec.slug && p.active.why ? ' · ' + p.active.why.replace('_', ' ') : ''}`),
      h('div', { class: 'title' }, title), h('div', { class: 'what' }, what),
      p.active && p.active.stale_pin ? h('div', { class: 'small warn', style: 'margin-top:6px;color:var(--warn)' }, `Pinned spec ${p.active.stale_pin} is done or gone; showing this one instead.`) : null),
    h('div', { class: 'actions' }, pinned ? btn('Unpin', () => pinSpec(p.name, null), 'ghost sm') : btn('Pin', () => pinSpec(p.name, spec.slug), 'ghost sm', { title: 'Make this the spec moor next follows' }), actions));
}
async function pinSpec(project, slug) {
  try { await api(`/projects/${project}/pin`, { method: 'POST', body: { slug } }); toast(slug ? `Pinned ${slug}; ${project} is now the default project.` : 'Unpinned.', 'ok'); invalidate(project); render(); }
  catch (e) { toast(e.message, 'err'); }
}

/* ----- node detail ----- */
function gateCard(g, title) {
  if (!g) return null;
  const checks = g.checks || [];
  const counts = ['pass', 'fail', 'blocked'].map((v) => [v, checks.filter((c) => c.verdict === v).length]);
  return h('div', { class: 'card' },
    h('div', { class: 'gatehead' }, h('span', { class: 'g' }, title || g.gate), verdictChip(g.verdict), h('span', { class: 'small dim' }, counts.filter(([, n]) => n).map(([v, n]) => `${n} ${v}`).join(' · ')), h('span', { class: 'right small dim mono' }, g.run ? 'run ' + g.run + ' · ' : '', fmtTime(g.generated_at))),
    h('div', { class: 'checks' }, checks.map((c) => h('div', { class: 'check' }, verdictChip(c.verdict), h('div', {}, h('span', { class: 'id' }, c.id), c.detail ? h('div', { class: 'detail-text' }, c.detail) : null,
      c.expected || c.actual ? h('div', { class: 'ea' }, h('b', {}, 'expected'), h('span', {}, c.expected || ''), h('b', {}, 'actual'), h('span', {}, c.actual || '')) : null)))));
}
function approvalsCard(detail, stage) {
  const hist = ((detail && detail.approvals && detail.approvals.history) || []).filter((a) => !stage || a.stage === stage);
  return h('div', { class: 'card' }, h('h3', {}, stage ? stage + ' approvals' : 'Approvals'),
    hist.length ? h('div', { class: 'approvals' }, hist.slice().reverse().map((a) => h('div', { class: 'appr' }, chip(a.decision, a.decision), h('div', {}, h('span', { class: 'by' }, a.by || 'unknown'), ' · ', h('span', { class: 'at' }, fmtTime(a.at)), a.gate_verdict ? [' · gate ', chip(a.gate_verdict, a.gate_verdict)] : null, a.note ? h('div', { class: 'small muted' }, '“' + a.note + '”') : null)))) : h('div', { class: 'small dim' }, 'None recorded yet.'));
}
function artifactPanel(p, spec, detail, kind, opts = {}) {
  const text = detail && detail.artifacts ? detail.artifacts[kind] : undefined;
  const ed = S.ui.editing && S.ui.editing.kind === kind ? S.ui.editing : null;
  const head = h('div', { class: 'row', style: 'margin-bottom:8px' }, h('h3', { style: 'margin:0' }, kind + '.md'), h('span', { class: 'right' }),
    text !== null && text !== undefined && !ed ? btn('Edit', () => { S.ui.editing = { kind, text }; render(); }, 'sm ghost') : null,
    ed ? [btn('Cancel', () => { S.ui.editing = null; render(); }, 'sm ghost'), btn('Save into sandbox', async () => {
      try { await api(`/projects/${p.name}/specs/${spec.slug}/artifact/${kind}`, { method: 'PUT', body: ed.text }); toast(`${kind}.md saved. ${opts.recheck ? 'Re-check it to record the change.' : ''}`, 'ok'); S.ui.editing = null; delete S.specs[p.name + '/' + spec.slug]; render(); }
      catch (e) { toast(e.message, 'err'); }
    }, 'sm primary')] : null);
  if (detail === undefined) return h('div', { class: 'card' }, head, h('div', { class: 'skeleton' }));
  if (text === null || text === undefined) return h('div', { class: 'card' }, head, h('div', { class: 'small dim' }, opts.missing || `No ${kind}.md yet.`));
  return h('div', { class: 'card' }, head, ed ? h('textarea', { class: 'editor', value: ed.text, oninput: (e) => ed.text = e.target.value, spellcheck: 'false' }) : mdView(text));
}
function decisionBox(p, spec, decide, label) {
  const pending = spec.pending_decision === decide;
  const arm = S.ui.arm;
  const busy = S.jobs.find((j) => j.project === p.name && !j.done && ['approve', 'reject'].includes(j.kind));
  if (!pending) {
    return h('div', { class: 'card inset' }, h('h3', {}, 'Decision'), h('div', { class: 'small muted' }, spec.complete || NODES.find((n) => n.decide === decide).n < stepIndex(spec, p.unshipped) ? `The ${decide} is approved.` : `Not at this step yet — it is at ${spec.label}.`));
  }
  if (arm && arm.kind === 'approve') {
    return h('div', { class: 'confirm' }, h('b', {}, `Approve ${label}?`), h('div', { class: 'small' }, 'This records your decision, under your git identity, in the audit chain. It is the human checkpoint the agent cannot pass on its own.'),
      h('div', { class: 'row' }, btn('Cancel', () => { S.ui.arm = null; render(); }, 'ghost'), h('span', { class: 'right' }), btn('Yes, approve', () => { S.ui.arm = null; startJob({ kind: 'approve', project: p.name }); }, 'primary', { disabled: !!busy })));
  }
  if (arm && arm.kind === 'reject') {
    return h('div', { class: 'confirm reject' }, h('b', {}, `Reject ${label}`), h('div', { class: 'small' }, 'Say why, so the next attempt can address it. The reason is recorded with the rejection.'),
      h('textarea', { class: 'editor', style: 'min-height:80px', placeholder: 'e.g. needs a rollback plan', value: arm.why, oninput: (e) => arm.why = e.target.value }),
      h('div', { class: 'row' }, btn('Cancel', () => { S.ui.arm = null; render(); }, 'ghost'), h('span', { class: 'right' }), btn('Record rejection', () => { if (!arm.why.trim()) return toast('Say why.', 'warn'); const why = arm.why; S.ui.arm = null; startJob({ kind: 'reject', project: p.name, why }); }, 'danger', { disabled: !!busy })));
  }
  return h('div', { class: 'card', style: 'border-color:var(--yellow-dim)' }, h('h3', {}, 'Your decision'), h('div', { class: 'small muted', style: 'margin-bottom:10px' }, `Read the ${decide === 'merge' ? 'change and its checks' : decide} on the left, then decide. Approval takes two clicks; rejection needs a reason.`),
    h('div', { class: 'row' }, btn('Reject…', () => { S.ui.arm = { kind: 'reject', why: '' }; render(); }, 'danger'), h('span', { class: 'right' }), btn('Approve…', () => { S.ui.arm = { kind: 'approve' }; render(); }, 'primary')));
}
function gateOf(detail, id) { return ((detail && detail.report && detail.report.gates) || []).filter((g) => g.gate === id).pop(); }
function runsOf(detail) { return (detail && detail.report && detail.report.runs) || []; }

function nodeDetail(p, spec, detail, nd, r) {
  if (!nd) return null;
  const busy = S.jobs.find((j) => j.project === p.name && !j.done && ['go', 'gate', 'approve', 'reject', 'ship'].includes(j.kind));
  const main = [], side = [];
  const at = (stage) => spec.stage === stage && !spec.complete;
  switch (nd.id) {
    case 'spec': {
      main.push(artifactPanel(p, spec, detail, 'spec', { recheck: true }));
      side.push(h('div', { class: 'card inset' }, h('h3', {}, 'Checks'), h('div', { class: 'small muted', style: 'margin-bottom:8px' }, 'G0 checks the spec itself: EARS-form criteria, oracles, scope, budgets, no vague words.'),
        h('div', { class: 'row' }, btn(at('spec') && busy ? [h('span', { class: 'spin' }), 'Checking…'] : 'Run G0 checks', () => startJob({ kind: 'gate', project: p.name, slug: spec.slug, gate: 'g0' }), at('spec') ? 'primary' : '', { disabled: !!busy }))));
      side.push(gateCard(gateOf(detail, 'G0'), 'G0'));
      side.push(approvalsCard(detail, 'spec'));
      break;
    }
    case 'approve-spec': {
      main.push(artifactPanel(p, spec, detail, 'spec'));
      side.push(decisionBox(p, spec, 'spec', 'the spec for ' + spec.slug));
      side.push(gateCard(gateOf(detail, 'G0'), 'G0'));
      side.push(approvalsCard(detail, 'spec'));
      break;
    }
    case 'plan': {
      const hasPlan = detail && detail.artifacts && detail.artifacts.plan;
      if (at('plan')) main.push(h('div', { class: 'banner yellow' }, h('div', { class: 'grow' }, h('b', {}, 'Draft the plan.'), ' The pipeline computes the blast radius of the approved spec and scaffolds plan.md and tasks.md from it. Nothing is built yet.'), btn(busy ? [h('span', { class: 'spin' }), 'Planning…'] : 'Draft plan + tasks', () => startJob({ kind: 'go', project: p.name }), 'primary', { disabled: !!busy })));
      main.push(artifactPanel(p, spec, detail, 'plan', { missing: hasPlan ? '' : 'No plan yet. Draft it from this step once the spec is approved.' }));
      main.push(artifactPanel(p, spec, detail, 'tasks'));
      side.push(h('div', { class: 'card inset' }, h('h3', {}, 'What a plan holds'), h('div', { class: 'small muted' }, 'The approach, the files it may touch (declared and computed blast radius), a rollback, and tasks that each name the criteria they satisfy, a line budget and an exit command.')));
      break;
    }
    case 'g1': {
      main.push(artifactPanel(p, spec, detail, 'plan', { recheck: true }));
      main.push(artifactPanel(p, spec, detail, 'tasks', { recheck: true }));
      side.push(h('div', { class: 'card inset' }, h('h3', {}, 'Checks'), h('div', { class: 'small muted', style: 'margin-bottom:8px' }, 'G1 checks the plan: every criterion covered by a task, budgets, blast radius, reviewable size.'),
        btn(at('plan_gate') && busy ? [h('span', { class: 'spin' }), 'Checking…'] : 'Run G1 checks', () => startJob({ kind: 'gate', project: p.name, slug: spec.slug, gate: 'g1' }), at('plan_gate') ? 'primary' : '', { disabled: !!busy })));
      side.push(gateCard(gateOf(detail, 'G1'), 'G1'));
      break;
    }
    case 'approve-plan': {
      main.push(artifactPanel(p, spec, detail, 'plan'));
      main.push(artifactPanel(p, spec, detail, 'tasks'));
      side.push(decisionBox(p, spec, 'plan', 'the plan for ' + spec.slug));
      side.push(gateCard(gateOf(detail, 'G1'), 'G1'));
      side.push(approvalsCard(detail, 'plan'));
      break;
    }
    case 'build': {
      const runs = runsOf(detail);
      const latest = runs[runs.length - 1];
      const sel = S.ui.buildRun && runs.find((x) => x.id === S.ui.buildRun) ? S.ui.buildRun : (latest && latest.id);
      const selRun = runs.find((x) => x.id === sel);
      if (at('run')) main.push(h('div', { class: 'banner yellow' }, h('div', { class: 'grow' }, h('b', {}, 'Build it.'), ` The agent (${PROVIDERS[p.agent] ? PROVIDERS[p.agent].name : p.agent}) works through the tasks inside the sandbox; G2 checks build, tests, lint, oracles and budgets; G2.5 reviews; G3 ratifies. A build that fails stops here with its evidence.`),
        h('div', { class: 'row' }, btn('Re-check without the agent', () => startJob({ kind: 'go', project: p.name, check: true }), '', { disabled: !!busy, title: 'keel run --no-driver: check the work already in the sandbox' }), btn(busy ? [h('span', { class: 'spin' }), 'Building…'] : 'Build', () => startJob({ kind: 'go', project: p.name }), 'primary', { disabled: !!busy }))));
      if (latest && latest.verdict === 'fail' && at('run')) main.push(h('div', { class: 'banner fail' }, h('div', { class: 'grow' }, h('b', {}, 'The latest build did not pass.'), ' Ask the agent to fix what it failed on, then re-check without rebuilding; or build again from the spec.'),
        btn('Ask the agent to fix it', () => { S.ui.role = 'build'; S.ui.composer = `Fix what the latest build of ${spec.slug} failed on; its failed checks are in its newest run under .keel/runs/`; go('p/' + p.name + '/agent'); })));
      main.push(h('div', { class: 'card' }, h('h3', {}, 'Runs for this spec'), runs.length ? runs.slice().reverse().map((x) => h('div', { class: 'runrow' + (x.id === sel ? ' active' : ''), onclick: () => { S.ui.buildRun = x.id; render(); } },
        h('span', { class: 'mono' }, x.id), h('span', { class: 'small muted' }, x.failing && x.failing.length ? x.failing.map((f) => f.id || f).join(', ') : (x.events ? x.events + ' events' : '')), verdictChip(x.verdict), btn('Evidence →', (e) => { e.stopPropagation(); go('p/' + p.name + '/runs/' + x.id); }, 'sm ghost'))) : h('div', { class: 'small dim' }, 'No runs yet.')));
      if (selRun && selRun.gates) for (const g of selRun.gates) main.push(gateCard(g, g.gate));
      side.push(h('div', { class: 'card inset' }, h('h3', {}, 'Tasks'), detail && detail.artifacts && detail.artifacts.tasks ? mdView(detail.artifacts.tasks.replace(/^---[\s\S]*?---\n/, '')) : h('div', { class: 'small dim' }, 'No tasks.md.')));
      break;
    }
    case 'merge': {
      const key = p.name + '/' + spec.slug;
      const d = S.diffs[key];
      if (d === undefined && (at('merge_approval') || spec.complete)) loadDiff(p.name, spec.slug);
      const runs = runsOf(detail);
      const latest = runs[runs.length - 1];
      main.push(h('div', { class: 'card' }, h('div', { class: 'row', style: 'margin-bottom:8px' }, h('h3', { style: 'margin:0' }, 'What the build changed'), h('span', { class: 'small dim' }, '(the pipeline’s own records aside)'), h('span', { class: 'right' }), d ? btn(S.ui.diffOpen ? 'Hide full diff' : 'Full diff', () => { S.ui.diffOpen = !S.ui.diffOpen; render(); }, 'sm ghost') : null),
        d === undefined ? h('div', { class: 'skeleton' }) : d ? [h('pre', { class: 'raw' }, d.stat || '(no tracked changes)'), d.new_files.length ? h('div', { class: 'small muted' }, 'New files: ' + d.new_files.join(', ')) : null, S.ui.diffOpen ? diffView(d.patch) : null] : h('div', { class: 'small dim' }, 'Diff unavailable.')));
      if (latest && latest.gates) for (const g of latest.gates) main.push(gateCard(g, g.gate));
      side.push(decisionBox(p, spec, 'merge', 'merging ' + spec.slug));
      side.push(approvalsCard(detail, 'merge'));
      break;
    }
    case 'ship': {
      const ready = spec.complete && (p.unshipped || []).includes(spec.slug);
      main.push(h('div', { class: 'card' }, h('h3', {}, 'Ship'), h('p', { class: 'muted' }, 'Shipping commits the spec’s change, its spec folder and its runs on a branch named moor/' + spec.slug + ' inside the sandbox, streams that branch out as a git bundle, pushes it from this Mac with your own GitHub login (the sandbox never holds a token), opens a pull request, and returns the sandbox to its trunk.'),
        p.git ? h('div', { class: 'small mono muted' }, `sandbox: ${p.git.branch || 'detached'} · ${p.git.dirty} changed · ${p.git.upstream ? p.git.upstream : 'no upstream'}`) : null,
        p.github_repo ? h('div', { class: 'small mono muted' }, 'remote: ' + p.github_repo) : h('div', { class: 'small', style: 'color:var(--warn)' }, 'No GitHub repo is recorded for this project; the push will fail without a remote.'),
        h('div', { class: 'row', style: 'margin-top:12px' }, h('span', { class: 'right' }), ready ? btn(busy ? [h('span', { class: 'spin' }), 'Shipping…'] : 'Ship ' + spec.slug, () => startJob({ kind: 'ship', project: p.name, spec: spec.slug }), 'primary', { disabled: !!busy }) : chip(spec.complete ? 'shipped' : 'not yet approved'))));
      side.push(approvalsCard(detail));
      break;
    }
  }
  if (S.errors[p.name + '/' + spec.slug]) main.unshift(h('div', { class: 'banner fail' }, S.errors[p.name + '/' + spec.slug]));
  return h('div', { class: 'detail' }, h('div', { class: 'col' }, main), h('div', { class: 'side' }, side));
}
/* ----- specs ----- */
function viewSpecs(p, r) {
  const out = [projectHeader(p, r)];
  if (!p.up) { out.push(downBanner(p)); return out; }
  const specs = p.report ? p.report.specs : [];
  const sf = S.ui.specForm, rf = S.ui.recipeForm;
  out.push(h('div', { class: 'row' }, h('div', {}, h('h2', { style: 'margin:0' }, 'Specs'), h('div', { class: 'small muted' }, 'A feature is one spec. Write it here with whatever help you like, or describe the outcome loosely as a recipe and let the pipeline draft the spec.')),
    h('span', { class: 'right' }), btn('Write a recipe', () => { S.ui.recipeForm = rf ? null : { slug: '', scope: ['src/**'], description: '' }; S.ui.specForm = null; render(); }, rf ? '' : ''), btn('New spec', () => { S.ui.specForm = sf ? null : { step: 1, slug: '', title: '', scope: ['src/**'], text: '' }; S.ui.recipeForm = null; render(); }, 'primary')));
  if (sf) out.push(specWizard(p, sf));
  if (rf) out.push(recipeForm(p, rf));
  const open = specs.filter((s) => !s.complete), done = specs.filter((s) => s.complete);
  const table = (list, title) => h('div', { class: 'card' }, h('h3', {}, title), list.length ? h('table', { class: 'list' }, h('thead', {}, h('tr', {}, h('th', {}, 'Spec'), h('th', {}, 'Step'), h('th', {}, 'Waiting on'), h('th', {}))),
    h('tbody', {}, list.map((s) => h('tr', { class: 'click' + (p.active && p.active.slug === s.slug ? ' active' : ''), onclick: () => go('p/' + p.name + '/spec/' + s.slug) },
      h('td', { class: 'mono' }, p.active && p.active.pin === s.slug ? '📌 ' : '', s.slug), h('td', {}, miniBar(s, p.unshipped, 'steps'), s.label),
      h('td', {}, s.pending_decision ? chip('your ' + s.pending_decision + ' approval', 'warn') : (p.unshipped || []).includes(s.slug) ? chip('ship', 'warn') : s.automatic ? chip('the pipeline') : s.complete ? chip('shipped', 'pass') : ''),
      h('td', { class: 'nowrap' }, h('span', { class: 'small dim' }, '→')))))) : h('div', { class: 'small dim' }, 'None.'));
  out.push(table(open, 'In progress'));
  if (done.length) out.push(table(done, 'Done'));
  return out;
}
function chipsInput(list, placeholder) {
  const wrap = h('div', { class: 'chips-input' });
  const draw = () => {
    clear(wrap);
    list.forEach((v, i) => wrap.appendChild(h('span', { class: 'chip' }, v, h('button', { onclick: () => { list.splice(i, 1); draw(); } }, '×'))));
    wrap.appendChild(h('input', { placeholder, onkeydown: (e) => { if ((e.key === 'Enter' || e.key === ',') && e.target.value.trim()) { e.preventDefault(); list.push(e.target.value.trim()); e.target.value = ''; draw(); wrap.querySelector('input').focus(); } } }));
  };
  draw();
  return wrap;
}
function specWizard(p, f) {
  if (f.step === 1) {
    return h('div', { class: 'card', style: 'border-color:var(--yellow-dim)' }, h('h3', {}, 'New spec · 1 of 2 · name it'),
      h('div', { class: 'grid two' },
        h('div', { class: 'field' }, h('label', {}, 'Short name'), h('input', { placeholder: 'login-endpoint', value: f.slug, oninput: (e) => f.slug = e.target.value.trim() }), h('span', { class: 'hint' }, 'Lowercase, dashes. Becomes .keel/specs/<name>/ in the sandbox.')),
        h('div', { class: 'field' }, h('label', {}, 'Title'), h('input', { placeholder: 'Add a login endpoint', value: f.title, oninput: (e) => f.title = e.target.value }))),
      h('div', { class: 'field', style: 'margin-top:10px' }, h('label', {}, 'Scope — the paths the change may touch (globs; Enter to add)'), chipsInput(f.scope, 'src/**')),
      h('div', { class: 'row', style: 'margin-top:12px' }, btn('Cancel', () => { S.ui.specForm = null; render(); }, 'ghost'), h('span', { class: 'right' }), btn('Write the spec →', async () => {
        if (!/^[a-z0-9][a-z0-9-]*$/.test(f.slug)) return toast('Give the spec a lowercase name with dashes.', 'warn');
        try { const d = await api('/specs/starter', { method: 'POST', body: { slug: f.slug, title: f.title, scope: f.scope } }); f.text = d.text; f.step = 2; render(); } catch (e) { toast(e.message, 'err'); }
      }, 'primary')));
  }
  return h('div', { class: 'card', style: 'border-color:var(--yellow-dim)' }, h('h3', {}, 'New spec · 2 of 2 · write it, then send it in'),
    h('div', { class: 'small muted', style: 'margin-bottom:8px' }, 'The rules at the top are what G0 will check (they are stripped on push). Paste this into your assistant of choice, or write it by hand. Pushing sends it into the sandbox, makes it the active spec, and runs its checks.'),
    h('textarea', { class: 'editor', style: 'min-height:520px', value: f.text, oninput: (e) => f.text = e.target.value, spellcheck: 'false' }),
    h('div', { class: 'row', style: 'margin-top:10px' }, btn('← Back', () => { f.step = 1; render(); }, 'ghost'), btn('Copy', () => navigator.clipboard.writeText(f.text).then(() => toast('Copied.', 'ok')), 'ghost'), h('span', { class: 'right' }), btn('Push & check', () => { startJob({ kind: 'spec_push', project: p.name, text: f.text }); S.ui.specForm = null; go('p/' + p.name + '/spec/' + f.slug + '/spec'); }, 'primary')));
}
function recipeForm(p, f) {
  return h('div', { class: 'card', style: 'border-color:var(--yellow-dim)' }, h('h3', {}, 'Recipe · describe the outcome, loosely'),
    h('div', { class: 'small muted', style: 'margin-bottom:8px' }, 'A recipe drives the whole pipeline from one description: the agent writes the spec (with write-only tools, so it cannot start building), the checks run with a bounded self-correction loop, and it pauses at every approval for you. Re-run it to continue after you decide.'),
    h('div', { class: 'grid two' },
      h('div', { class: 'field' }, h('label', {}, 'Spec name'), h('input', { placeholder: 'greet-function', value: f.slug, oninput: (e) => f.slug = e.target.value.trim() })),
      h('div', { class: 'field' }, h('label', {}, 'Scope (Enter to add)'), chipsInput(f.scope, 'src/**'))),
    h('div', { class: 'field', style: 'margin-top:10px' }, h('label', {}, 'What you want'), h('textarea', { class: 'editor', style: 'min-height:160px;font-family:var(--sans)', placeholder: 'I want a function greet(name) that returns "Hello, <name>!". Plain JS, no dependencies…', value: f.description, oninput: (e) => f.description = e.target.value })),
    h('div', { class: 'row', style: 'margin-top:10px' }, btn('Cancel', () => { S.ui.recipeForm = null; render(); }, 'ghost'), h('span', { class: 'right' }), btn('Write recipe & run it', async () => {
      if (!/^[a-z0-9][a-z0-9-]*$/.test(f.slug)) return toast('Give the spec a lowercase name with dashes.', 'warn');
      if (!f.description.trim()) return toast('Describe what you want.', 'warn');
      try { const d = await api(`/projects/${p.name}/recipes`, { method: 'POST', body: { slug: f.slug, scope: f.scope, description: f.description } }); startJob({ kind: 'recipe', project: p.name, path: d.path }); S.ui.recipeForm = null; go('p/' + p.name + '/spec/' + f.slug); } catch (e) { toast(e.message, 'err'); }
    }, 'primary')));
}

/* ----- agent ----- */
function viewAgent(p, r) {
  const out = [projectHeader(p, r)];
  if (!p.up) { out.push(downBanner(p)); return out; }
  const t = S.transcripts[p.name];
  if (t === undefined) loadTranscript(p.name);
  const turns = (t && t.turns) || [];
  const live = S.jobs.find((j) => j.project === p.name && j.kind === 'ask' && !j.done);
  const prov = PROVIDERS[p.agent] || { name: p.agent };
  const cred = (p.secrets_status || []).some((s) => (PROVIDERS[p.agent] || { secrets: [] }).secrets.includes(s.name) && s.source !== 'missing');
  out.push(h('div', { class: 'row' }, h('div', {}, h('h2', { style: 'margin:0' }, 'Agent · ' + prov.name), h('div', { class: 'small muted' }, 'One recorded turn at a time. Brainstorm is read-only; build may edit files and run checks, but can never approve, reach a shell, or leave the sandbox.')),
    h('span', { class: 'right' }), p.session ? chip('session ' + p.session.slice(0, 8), 'info') : chip('no session'), p.agent !== 'kiro' && !cred ? chip('no credential', 'warn') : null));
  const lastAgent = turns.slice().reverse().find((x) => x.response);
  const chat = h('div', { class: 'chat' });
  if (t === undefined) chat.appendChild(h('div', { class: 'skeleton' }));
  else if (!turns.length) chat.appendChild(h('div', { class: 'empty' }, h('strong', {}, 'No turns yet'), 'Ask what it would take to make a change, or tell it to fix what the last build failed on.'));
  for (const x of turns) {
    chat.appendChild(h('div', { class: 'turn' }, h('div', { class: 'bubble me' }, h('div', { class: 'meta' }, chip(x.role || 'brainstorm'), fmtTime(x.ts)), x.prompt),
      h('div', { class: 'bubble agent' }, h('div', { class: 'meta' }, prov.name, x === lastAgent ? btn('Turn into a recipe', () => recipeFromText(p, x.response), 'sm ghost') : null), mdView(x.response))));
  }
  if (live) chat.appendChild(h('div', { class: 'turn' }, h('div', { class: 'bubble me' }, h('div', { class: 'meta' }, chip(live.role || S.ui.role), 'now'), live.prompt || '…'), h('div', { class: 'bubble agent live' }, h('div', { class: 'meta' }, h('span', { class: 'spin' }), ' thinking · ' + dur(live.started)), h('pre', { class: 'raw', style: 'max-height:200px' }, (live.lines || []).slice(-30).join('\n')))));
  out.push(chat);
  out.push(h('div', { class: 'composer' },
    h('textarea', { placeholder: S.ui.role === 'build' ? 'Tell it what to change. It can edit files and run keel’s checks through MCP, nothing else.' : 'Ask it to think a change through against the real repository…', value: S.ui.composer, oninput: (e) => S.ui.composer = e.target.value, onkeydown: (e) => { if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) { e.preventDefault(); sendTurn(p); } } }),
    h('div', { class: 'row' }, h('div', { class: 'toggle' }, ['brainstorm', 'build'].map((role) => h('button', { class: S.ui.role === role ? 'on' : '', onclick: () => { S.ui.role = role; render(); } }, role))),
      h('label', { class: 'row small muted' }, h('input', { type: 'checkbox', checked: S.ui.newSession, onchange: (e) => S.ui.newSession = e.target.checked }), 'new session'),
      h('span', { class: 'right small dim' }, '⌘↵ to send'), btn(live ? [h('span', { class: 'spin' }), 'Waiting…'] : 'Send', () => sendTurn(p), 'primary', { disabled: !!live }))));
  return out;
}
function sendTurn(p) {
  const prompt = S.ui.composer.trim();
  if (!prompt) return;
  startJob({ kind: 'ask', project: p.name, role: S.ui.role, new: S.ui.newSession, prompt }, { prompt, role: S.ui.role });
  S.ui.composer = ''; S.ui.newSession = false;
  render();
}
async function recipeFromText(p, text) {
  try { const d = await api(`/projects/${p.name}/recipes`, { method: 'POST', body: { text } }); modal([h('h2', {}, 'Recipe written'), h('div', { class: 'small muted mono' }, d.path), h('pre', { class: 'raw' }, d.text), h('div', { class: 'foot' }, btn('Close', closeModal, 'ghost'), btn('Run it', () => { closeModal(); startJob({ kind: 'recipe', project: p.name, path: d.path }); }, 'primary'))]); }
  catch (e) { modal([h('h2', {}, 'That answer is not a recipe'), h('div', { class: 'small muted' }, 'A recipe starts with a front matter block (slug, scope) followed by free text. Ask the agent to answer in that form, or write one from the Specs tab.'), h('pre', { class: 'raw' }, e.message), h('div', { class: 'foot' }, btn('Close', closeModal))]); }
}

/* ----- runs & evidence ----- */
function viewRuns(p, r) {
  const out = [projectHeader(p, r)];
  if (!p.up) { out.push(downBanner(p)); return out; }
  const runs = (p.runs || []).slice().sort((a, b) => (b.started_at || '').localeCompare(a.started_at || ''));
  const sel = r.run;
  out.push(h('div', { class: 'row' }, h('div', {}, h('h2', { style: 'margin:0' }, 'Runs & evidence'), h('div', { class: 'small muted' }, 'Every build run keeps its own evidence in the sandbox: gate results, build/test/lint output, the diff, oracle results, and the agent’s trajectory.')),
    h('span', { class: 'right' }), bundleButton(p, sel)));
  const list = h('div', { class: 'card' }, h('h3', {}, runs.length + ' runs'), runs.length ? runs.map((x) => h('div', { class: 'runrow' + (x.id === sel ? ' active' : ''), onclick: () => go('p/' + p.name + '/runs/' + x.id) }, h('span', { class: 'mono' }, x.id), h('span', { class: 'small muted ellipsis' }, x.spec, ' · ', ago(x.started_at), ' · ', dur(x.started_at, x.finished_at)), verdictChip(x.verdict), h('span', { class: 'dim' }, '→'))) : h('div', { class: 'small dim' }, 'No runs yet.'));
  if (!sel) { out.push(list); return out; }
  const key = p.name + '/' + sel;
  const d = S.runs[key];
  if (d === undefined) loadRun(p.name, sel);
  const detail = [];
  if (d === undefined) detail.push(h('div', { class: 'skeleton' }));
  else if (S.errors[key]) detail.push(h('div', { class: 'banner fail' }, S.errors[key]));
  else {
    const run = d.run || {};
    detail.push(h('div', { class: 'card' }, h('div', { class: 'row' }, h('h2', { class: 'mono', style: 'margin:0' }, d.id), verdictChip(run.verdict), h('span', { class: 'small muted' }, run.spec, ' · keel ', run.keel_version, ' · ', fmtTime(run.started_at), ' → ', dur(run.started_at, run.finished_at)), h('span', { class: 'right' }), run.spec ? btn('Open spec', () => go('p/' + p.name + '/spec/' + run.spec + '/build'), 'sm ghost') : null),
      run.base_commit ? h('div', { class: 'small mono dim' }, 'base ' + run.base_commit) : null));
    for (const g of d.gates) detail.push(gateCard(g, g.gate));
    const ev = S.ui.evFile;
    const evKey = key + '/' + ev;
    const evText = S.evidence[evKey];
    if (ev && evText === undefined && !S.loading[evKey]) { S.loading[evKey] = true; api(`/projects/${p.name}/runs/${sel}/evidence/${ev}`).then((x) => { S.evidence[evKey] = x.text; }).catch((e) => { S.evidence[evKey] = 'could not read: ' + e.message; }).finally(() => { delete S.loading[evKey]; render(); }); }
    detail.push(h('div', { class: 'card' }, h('h3', {}, 'Evidence files'), h('div', { class: 'detail', style: 'grid-template-columns:240px 1fr' },
      h('div', {}, d.evidence.length ? d.evidence.map((f) => h('div', { class: 'evfile' + (ev === f.name ? ' active' : ''), onclick: () => { S.ui.evFile = f.name; render(); } }, f.name, h('span', { class: 'dim' }, fmtSize(f.size)))) : h('div', { class: 'small dim' }, 'None.')),
      ev ? (evText === undefined ? h('div', { class: 'skeleton' }) : /\.patch$/.test(ev) ? diffView(evText) : h('pre', { class: 'raw' }, evText)) : h('div', { class: 'empty' }, 'Pick a file to read it.'))));
    detail.push(h('div', { class: 'card' }, h('h3', {}, `Trajectory · ${d.trajectory.length} events`), d.trajectory.length ? h('div', { class: 'traj' }, d.trajectory.map((e) => h('div', {}, h('span', { class: 'dim' }, e.seq), h('span', { class: 'dim' }, fmtTime(e.t)), h('span', { class: 'k' }, e.kind), h('span', { class: 'ellipsis' }, trajSummary(e))))) : h('div', { class: 'small dim' }, 'No trajectory recorded.')));
  }
  out.push(h('div', { class: 'detail', style: 'grid-template-columns:420px minmax(0,1fr)' }, list, h('div', { class: 'col' }, detail)));
  return out;
}
function fmtSize(n) { return n < 1024 ? n + ' B' : n < 1048576 ? (n / 1024).toFixed(1) + ' KB' : (n / 1048576).toFixed(1) + ' MB'; }
function trajSummary(e) {
  const skip = new Set(['t', 'seq', 'kind']);
  return Object.entries(e).filter(([k]) => !skip.has(k)).map(([k, v]) => `${k}=${typeof v === 'string' ? v : JSON.stringify(v)}`).join('  ');
}
function bundleButton(p, run) {
  return btn('Bundle evidence…', () => {
    const f = { out: (S.overview && S.overview.home ? S.overview.home.replace(/\/\.moor$/, '') : '') + '/Downloads', run: run || '' };
    modal([h('h2', {}, 'Evidence bundle'), h('div', { class: 'small muted' }, 'Packs the run’s evidence with this project’s audit chain in a throwaway container with no network, verifies it in a second one, and writes one .tar.gz you can hand over.'),
      h('div', { class: 'field' }, h('label', {}, 'Run (blank = latest)'), h('input', { value: f.run, class: 'mono', oninput: (e) => f.run = e.target.value.trim() })),
      h('div', { class: 'field' }, h('label', {}, 'Write into'), h('input', { value: f.out, oninput: (e) => f.out = e.target.value.trim() })),
      h('div', { class: 'foot' }, btn('Cancel', closeModal, 'ghost'), btn('Bundle & verify', () => { closeModal(); startJob({ kind: 'bundle', project: p.name, run: f.run || undefined, out: f.out }); }, 'primary'))]);
  });
}

/* ----- audit ----- */
function entrySummary(e) {
  const d = e.data || {};
  switch (e.kind) {
    case 'exec': case 'next': case 'spec': case 'ship': case 'studio': case 'ui': case 'approve': return (d.argv || []).join(' ') + (d.exit_code !== undefined && d.exit_code !== 0 ? `  → exit ${d.exit_code}` : '');
    case 'push': return `git push ${d.ref || ''} @ ${(d.commit || '').slice(0, 10)}`;
    case 'gate': return `${d.gate} ${d.verdict} · ${d.spec} · run ${d.run}`;
    case 'approval': return `${d.stage} ${d.decision} · ${d.spec} · by ${d.by || 'unknown'}${d.note ? ' · “' + d.note + '”' : ''}`;
    case 'run_start': return `run ${d.run} started · ${d.spec}${d.task ? ' · ' + d.task : ''}`;
    case 'run_end': return `run ${d.run} ${d.verdict}`;
    case 'egress': return `${d.verdict || ''} ${d.domain || d.host || ''} ${d.method || ''}`.trim();
    case 'tripwire': return `TRIPWIRE · ${d.domain || JSON.stringify(d)}`;
    case 'tripwire-check': return 'breakout battery ran';
    case 'attest': return 'sandbox posture attested · ' + Object.entries(d.properties || {}).filter(([, v]) => v.status !== 'proven').map(([k]) => k + ' unproven').join(', ') || 'sandbox posture attested · all proven';
    case 'agent-turn': return `${d.agent || ''} ${d.role} turn · ${d.result}${d.num_turns ? ' · ' + d.num_turns + ' steps' : ''}${d.permission_denials ? ' · ' + d.permission_denials + ' denials' : ''}`;
    case 'recipe-event': return `${d.slug || ''} ${d.event || ''} ${d.stage || ''} ${d.result || ''}`.trim();
    case 'legacy_seal': return 'legacy chain sealed';
    default: return JSON.stringify(d).slice(0, 200);
  }
}
function postureView(p, compact) {
  const props = (p && p.properties) || {};
  return h('div', { class: 'posture' }, Object.entries(props).map(([k, v]) => [h('span', {}, k), chip(v.status, v.status), compact ? null : h('span', { class: 'ev' }, v.evidence || '')]));
}
function viewAudit(p, r) {
  const out = [projectHeader(p, r)];
  const a = S.audits[p.name];
  if (a === undefined) loadAudit(p.name);
  out.push(h('div', { class: 'row wrap' }, h('div', {}, h('h2', { style: 'margin:0' }, 'Audit trail'), h('div', { class: 'small muted' }, 'One hash-linked record on this Mac, written only by moor: every command, push, gate verdict, approval, agent turn and egress request. The sandbox cannot reach it.')),
    h('span', { class: 'right' }), btn('Self-test', () => startJob({ kind: 'selftest', project: p.name }), '', { disabled: !p.up, title: 'Re-check every hardening control against the live container' }), btn('Export…', () => {
      const f = { out: (S.overview && S.overview.home ? S.overview.home.replace(/\/\.moor$/, '') : '') + '/Downloads' };
      modal([h('h2', {}, 'Export the trail'), h('div', { class: 'small muted' }, 'The chain plus the runs’ exported bundles, as one tar.gz.'), h('div', { class: 'field' }, h('label', {}, 'Write into'), h('input', { value: f.out, oninput: (e) => f.out = e.target.value.trim() })), h('div', { class: 'foot' }, btn('Cancel', closeModal, 'ghost'), btn('Export', () => { closeModal(); startJob({ kind: 'audit_export', project: p.name, out: f.out }); }, 'primary'))]);
    }), btn('Verify chain', () => startJob({ kind: 'audit_verify', project: p.name }), 'primary')));
  if (a === undefined) { out.push(h('div', { class: 'skeleton' })); return out; }
  if (S.errors[p.name]) { out.push(h('div', { class: 'banner fail' }, S.errors[p.name])); }
  if (a.tripwires) out.push(h('div', { class: 'banner fail' }, h('div', {}, h('b', {}, `${a.tripwires} tripwire entr${a.tripwires === 1 ? 'y' : 'ies'}.`), ' Something inside the sandbox tried to reach the canary domain. There is no legitimate reason for that; see docs/THREAT-MODEL.md.')));
  const kinds = Object.entries(a.kinds || {});
  const sel = S.ui.auditKinds;
  out.push(h('div', { class: 'grid', style: 'grid-template-columns: minmax(0,1fr) 300px' },
    h('div', { class: 'card' },
      h('div', { class: 'filters', style: 'margin-bottom:10px' }, h('span', { class: 'small dim' }, `${a.total} entries`), kinds.map(([k, n]) => h('button', { class: 'chip' + (sel.has(k) ? ' yellow' : ''), style: 'cursor:pointer', onclick: () => { sel.has(k) ? sel.delete(k) : sel.add(k); render(); } }, `${k} ${n}`)), sel.size ? btn('clear', () => { sel.clear(); render(); }, 'sm ghost') : null),
      h('div', {}, a.entries.slice().reverse().filter((e) => !sel.size || sel.has(e.kind)).map((e) => h('div', { class: 'entry' + (e.kind === 'tripwire' ? ' tripwire' : '') }, h('span', { class: 'seq' }, '#' + e.seq), h('span', { class: 'ts' }, fmtTime(e.ts)), chip(e.kind + (e.data && e.data.source === 'sandbox' ? ' ·sbx' : ''), e.kind === 'tripwire' ? 'fail' : e.kind === 'approval' ? 'approved' : e.kind === 'gate' ? (e.data.verdict || '') : ''),
        h('div', {}, h('div', { class: 'sum' }, entrySummary(e)), h('details', { class: 'raw-json' }, h('summary', {}, (e.hash || '').slice(0, 16) + '…'), h('pre', { class: 'raw', style: 'max-height:240px' }, JSON.stringify(e, null, 2)))))))),
    h('div', { class: 'col' },
      h('div', { class: 'card' }, h('h3', {}, 'Chain'), h('div', { class: 'small mono muted', style: 'overflow-wrap:anywhere' }, a.path), h('div', { class: 'small muted', style: 'margin-top:6px' }, `${a.total} entries · keel.chain/1 format${a.legacy ? ' · a sealed legacy chain precedes it' : ''}. Verify recomputes every hash and names the first entry that was edited, deleted, reordered or forged.`)),
      h('div', { class: 'card' }, h('h3', {}, 'Sandbox attestation'), p.posture ? [h('div', { class: 'small muted', style: 'margin-bottom:8px' }, 'Derived from docker inspect on the last start and handed to the pipeline; it refuses to run an agent without the required properties proven.'), postureView(p.posture, false)] : h('div', { class: 'small dim' }, 'No attestation on record.')))));
  return out;
}

/* ----- settings ----- */
function viewSettings(p, r) {
  const out = [projectHeader(p, r)];
  const s = S.settings[p.name];
  if (s === undefined) { loadSettings(p.name); out.push(h('div', { class: 'skeleton' })); return out; }
  if (S.errors[p.name]) out.push(h('div', { class: 'banner fail' }, S.errors[p.name]));
  const f = S.ui.settingsForm && S.ui.settingsForm.project === p.name ? S.ui.settingsForm : (S.ui.settingsForm = { project: p.name, egress: [...s.egress], cpu: s.resources.cpu, mem: s.resources.mem, pids: s.resources.pids, github_repo: s.github_repo || '', image: s.image, secrets: [...s.secrets], secretValues: {} });
  const status = {};
  for (const x of s.secrets_status || []) status[x.name] = x.source;
  const saveManifest = async (patch, msg) => { try { await api(`/projects/${p.name}/manifest`, { method: 'PUT', body: patch }); toast(msg || 'Saved. Restart the sandbox to apply.', 'ok'); S.ui.settingsForm = null; invalidate(p.name); render(); } catch (e) { toast(e.message, 'err'); } };
  const restart = h('div', { class: 'banner info' }, h('div', { class: 'grow' }, 'Manifest changes take effect on the next start of the sandbox.'), btn(p.up ? 'Restart sandbox' : 'Start sandbox', () => startJob({ kind: 'up', project: p.name }), 'sm primary'));

  // providers
  const provCards = Object.entries(PROVIDERS).map(([id, pr]) => {
    const have = pr.secrets.some((n) => status[n] && status[n] !== 'missing') || (id === 'copilot' && s.copilot_device_token);
    const on = s.agent === id;
    return h('div', { class: 'provider' + (on ? ' on' : ''), onclick: () => { if (!on) saveManifest({ agent: id, image: s.image }, `Agent set to ${pr.name}. Restart the sandbox to apply.`); } },
      h('div', { class: 'pn' }, pr.name, h('span', { class: 'small dim' }, pr.vendor), h('span', { class: 'right' }), on ? chip('active', 'yellow') : null),
      h('div', { class: 'pd' }, pr.blurb),
      h('div', { class: 'row small' }, id === 'kiro' ? chip('login on host', 'info') : have ? chip('credential found', 'pass') : chip('no credential', 'warn')),
      h('div', { class: 'small dim', style: 'font-family:var(--mono);font-size:11px' }, pr.howto));
  });
  out.push(h('div', { class: 'card' }, h('h3', {}, 'Agent provider'), h('div', { class: 'small muted', style: 'margin-bottom:10px' }, 'Which agent the sandbox is set up for. Claude ships in every image; Copilot composes with the moor/copilot layer; Kiro uses the host login. Secrets are declared per project and resolved from your shell or the macOS Keychain on every start — values never live in the manifest.'), h('div', { class: 'providers' }, provCards)));

  // drivers
  if (s.up) out.push(h('div', { class: 'card' }, h('h3', {}, 'Build driver (keel)'), h('div', { class: 'small muted', style: 'margin-bottom:8px' }, 'The driver keel runs a task through when it builds. The default is what `moor go` uses; a conformance check runs the driver in a scratch repository and records whether it behaves.'),
    (s.drivers || []).map((d) => h('div', { class: 'driver' }, h('span', { class: 'radio' + (d.default ? ' on' : ''), style: 'cursor:pointer', onclick: async () => { try { await api(`/projects/${p.name}/drivers/default`, { method: 'POST', body: { id: d.id } }); toast(`Default driver is now ${d.id}.`, 'ok'); invalidate(p.name); render(); } catch (e) { toast(e.message, 'err'); } } }),
      h('div', {}, h('div', { class: 'did' }, d.id, d.default ? [' ', chip('default', 'yellow')] : null, PROVIDERS[s.agent] && PROVIDERS[s.agent].driver === d.id ? [' ', chip('matches agent', 'info')] : null), h('div', { class: 'dp' }, d.path)),
      btn('Check', () => startJob({ kind: 'driver_check', project: p.name, id: d.id }), 'sm ghost'), h('span', {}))),
    s.drivers && s.drivers.length === 0 ? h('div', { class: 'small dim' }, 'No drivers listed — is keel initialised in this workspace?') : null));
  else out.push(h('div', { class: 'card' }, h('h3', {}, 'Build driver (keel)'), h('div', { class: 'small dim' }, 'Start the sandbox to see and change keel’s drivers.')));

  // secrets
  out.push(h('div', { class: 'card' }, h('h3', {}, 'Credentials'), h('div', { class: 'small muted', style: 'margin-bottom:8px' }, 'Each declared name is resolved, in order, from your shell’s environment, then the macOS Keychain. Setting one here stores it in the Keychain for this project only; the value is sent to moor on stdin and never appears in a command line, this page’s job list, or the audit chain.'),
    f.secrets.map((name) => {
      const src = status[name] || 'missing';
      const v = f.secretValues[name] || '';
      return h('div', { class: 'secret' }, h('div', {}, h('span', { class: 'sn' }, name), ' ', chip(src === 'env' ? 'from your shell' : src === 'keychain' ? 'Keychain' : 'not set', src === 'missing' ? 'warn' : 'pass')),
        h('input', { type: 'password', placeholder: 'paste value…', value: v, autocomplete: 'off', oninput: (e) => f.secretValues[name] = e.target.value }),
        h('div', { class: 'row' }, btn('Store', () => { if (!v.trim()) return toast('Paste the value first.', 'warn'); startJob({ kind: 'secret_set', project: p.name, name, value: v }); f.secretValues[name] = ''; render(); }, 'sm primary', { disabled: !v.trim() }), src === 'keychain' ? btn('Remove', () => startJob({ kind: 'secret_unset', project: p.name, name }), 'sm ghost') : null, btn('×', () => { f.secrets = f.secrets.filter((n) => n !== name); render(); }, 'sm ghost', { title: 'Stop declaring this secret' })));
    }),
    h('div', { class: 'row', style: 'margin-top:8px' }, h('input', { class: 'mono', placeholder: 'DECLARE_ANOTHER_NAME', style: 'background:var(--bg-inset);border:1px solid var(--border);border-radius:6px;padding:5px 8px', onkeydown: (e) => { if (e.key === 'Enter' && /^[A-Z_][A-Z0-9_]*$/.test(e.target.value.trim())) { f.secrets.push(e.target.value.trim()); e.target.value = ''; render(); } } }), h('span', { class: 'right' }), JSON.stringify(f.secrets) !== JSON.stringify(s.secrets) ? btn('Save declared names', () => saveManifest({ secrets: f.secrets }), 'sm primary') : null)));

  // sandbox
  out.push(h('div', { class: 'grid two' },
    h('div', { class: 'card' }, h('h3', {}, 'Egress allowlist'), h('div', { class: 'small muted', style: 'margin-bottom:8px' }, 'Hostnames the sandbox may reach through the proxy. Everything else is denied and logged. GitHub pushes no longer need an entry: they leave from this Mac.'), chipsInput(f.egress, 'registry.npmjs.org'),
      h('div', { class: 'row', style: 'margin-top:10px' }, h('span', { class: 'right' }), btn('Save allowlist', () => saveManifest({ egress: f.egress }), 'sm primary'))),
    h('div', { class: 'card' }, h('h3', {}, 'Sandbox'), h('div', { class: 'grid', style: 'grid-template-columns:1fr 1fr 1fr;gap:8px' },
      h('div', { class: 'field' }, h('label', {}, 'CPU'), h('input', { value: f.cpu, oninput: (e) => f.cpu = e.target.value })), h('div', { class: 'field' }, h('label', {}, 'Memory'), h('input', { value: f.mem, oninput: (e) => f.mem = e.target.value })), h('div', { class: 'field' }, h('label', {}, 'PIDs'), h('input', { type: 'number', value: f.pids, oninput: (e) => f.pids = +e.target.value }))),
      h('div', { class: 'field', style: 'margin-top:8px' }, h('label', {}, 'Image'), h('select', { onchange: (e) => f.image = e.target.value }, imageOptions(s.images, f.image))),
      h('div', { class: 'field', style: 'margin-top:8px' }, h('label', {}, 'GitHub repo (owner/name)'), h('input', { value: f.github_repo, placeholder: 'you/your-repo', oninput: (e) => f.github_repo = e.target.value })),
      h('div', { class: 'row', style: 'margin-top:10px' }, h('span', { class: 'small mono dim ellipsis' }, s.manifest_path), h('span', { class: 'right' }), btn('Save', () => saveManifest({ resources: { cpu: f.cpu, mem: f.mem, pids: f.pids }, image: f.image, github_repo: f.github_repo }), 'sm primary')))));
  out.push(restart);
  out.push(h('div', { class: 'card' }, h('h3', {}, 'Maintenance'), h('div', { class: 'row wrap' },
    btn('Doctor (git state)', () => startJob({ kind: 'doctor', project: p.name }), '', { disabled: !p.up }), btn('Pull trunk from GitHub', () => startJob({ kind: 'pull', project: p.name }), '', { disabled: !p.up }), btn('Self-test hardening', () => startJob({ kind: 'selftest', project: p.name }), '', { disabled: !p.up }),
    h('span', { class: 'right' }), p.up ? btn('Stop sandbox', () => startJob({ kind: 'down', project: p.name }), 'danger') : btn('Start sandbox', () => startJob({ kind: 'up', project: p.name }), 'primary'))));
  if (s.keel_config) out.push(h('details', { class: 'card' }, h('summary', { style: 'cursor:pointer' }, h('b', {}, 'keel.toml'), h('span', { class: 'small dim' }, ' — the pipeline’s own configuration inside the sandbox (read-only here)')), h('pre', { class: 'raw', style: 'margin-top:10px' }, s.keel_config)));
  return out;
}

/* ---------- jobs & dock ---------- */
async function startJob(body, extra = {}) {
  try {
    const job = await api('/jobs', { method: 'POST', body });
    Object.assign(job, extra);
    S.jobs.push(job); S.jobById[job.id] = job; S.activeJob = job.id; S.dockOpen = true;
    subscribe(job);
    renderDock(); render();
    return job;
  } catch (e) { toast(e.message, 'err', 6000); }
}
function subscribe(job) {
  job.lines = job.lines || [];
  const es = new EventSource(`/api/jobs/${job.id}/stream?token=${TOKEN}&after=${job.next || 0}`);
  es.addEventListener('lines', (ev) => { const d = JSON.parse(ev.data); job.lines.push(...d.lines); job.next = d.next; renderDock(); if (job.kind === 'ask' && S.route.tab === 'agent') render(); });
  es.addEventListener('done', (ev) => { const d = JSON.parse(ev.data); Object.assign(job, d, { lines: job.lines.concat(d.lines || []) }); es.close(); finished(job); });
  es.onerror = () => { if (job.done) es.close(); };
}
function finished(job) {
  const ok = job.exit_code === 0;
  toast(`${job.label} ${ok ? 'finished' : 'failed (exit ' + job.exit_code + ')'}`, ok ? 'ok' : 'err');
  invalidate(job.project);
  if (job.kind === 'new' || job.kind === 'import') S.overview = null;
  renderDock(); render();
}
function toggleDock() { S.dockOpen = !S.dockOpen; renderDock(); }
function renderDock() {
  const dock = clear(document.getElementById('dock'));
  dock.classList.toggle('open', S.dockOpen);
  const running = S.jobs.filter((j) => !j.done);
  const last = S.jobs[S.jobs.length - 1];
  dock.appendChild(h('div', { class: 'dockbar', onclick: toggleDock },
    h('span', { class: 'title' }, 'Activity'),
    running.length ? h('span', { class: 'jdot run', style: 'width:8px;height:8px;border-radius:50%;background:var(--yellow);display:inline-block;animation:blink 1s infinite' }) : null,
    h('span', { class: 'status grow' }, running.length ? `${running.length} running · ${running.map((j) => j.label).join(', ')}` : last ? `${last.label} · ${last.exit_code === 0 ? 'ok' : 'exit ' + last.exit_code} · ${ago(last.finished)}` : 'nothing running — actions you take show their output here'),
    h('span', { class: 'small dim' }, S.dockOpen ? '▾' : '▴')));
  if (!S.dockOpen) return;
  const act = S.jobById[S.activeJob] || last;
  const body = h('div', { class: 'dockbody' });
  body.appendChild(h('div', { class: 'joblist' }, S.jobs.slice().reverse().map((j) => h('div', { class: 'jobrow' + (act && act.id === j.id ? ' active' : ''), onclick: (e) => { e.stopPropagation(); S.activeJob = j.id; renderDock(); } },
    h('span', { class: 'jdot ' + (!j.done ? 'run' : j.exit_code === 0 ? 'ok' : 'err') }), h('div', {}, h('div', { class: 'jl' }, j.label), h('div', { class: 'jp' }, (j.project || '') + ' · ' + (j.done ? dur(j.started, j.finished) : ago(j.started))))))));
  if (act) {
    const con = h('pre', { class: 'console' });
    for (const line of act.lines || []) {
      let cls = '';
      if (/^(error|TAMPERED|\[FAIL\]|FAIL\b)/.test(line) || /\bFAILED\b/.test(line)) cls = 'err';
      else if (/^(\[PASS\]|chain OK|✓)|\bpass\b/.test(line)) cls = 'ok';
      else if (/^\s*(Next|Then|Or|See):/.test(line)) cls = 'next';
      else if (/^==>/.test(line)) cls = 'hl';
      con.appendChild(h('span', { class: cls }, line + '\n'));
    }
    if (!act.done) con.appendChild(h('span', { class: 'dim' }, '… running ' + dur(act.started)));
    body.appendChild(h('div', { class: 'jobout' }, h('div', { class: 'jobhead' }, h('span', {}, 'moor ' + act.argv.join(' ')), h('span', { class: 'right' }), !act.done ? btn('Stop', () => api(`/jobs/${act.id}/cancel`, { method: 'POST' }).catch((e) => toast(e.message, 'err')), 'sm danger') : chip(act.exit_code === 0 ? 'exit 0' : 'exit ' + act.exit_code, act.exit_code === 0 ? 'pass' : 'fail')), con));
    requestAnimationFrame(() => { con.scrollTop = con.scrollHeight; });
  }
  dock.appendChild(body);
}

/* ---------- palette ---------- */
function paletteItems() {
  const items = [];
  const r = S.route;
  const ov = S.overview;
  if (r.project) {
    const p = S.projects[r.project];
    items.push({ t: 'Run the next step (moor go)', k: 'go', f: () => startJob({ kind: 'go', project: r.project }) });
    items.push({ t: 'Approve what is waiting', k: 'approve', f: () => { const s = p && activeSpecOf(p, r); if (s && s.pending_decision) go('p/' + r.project + '/spec/' + s.slug + '/' + NODES.find((n) => n.decide === s.pending_decision).id); else toast('Nothing is waiting for your decision.', 'warn'); } });
    items.push({ t: 'Ship the approved spec', k: 'ship', f: () => startJob({ kind: 'ship', project: r.project }) });
    items.push({ t: 'Ask the agent', k: 'agent', f: () => go('p/' + r.project + '/agent') });
    items.push({ t: 'New spec', k: 'spec', f: () => { S.ui.specForm = { step: 1, slug: '', title: '', scope: ['src/**'], text: '' }; go('p/' + r.project + '/specs'); } });
    items.push({ t: 'Verify the audit chain', k: 'audit', f: () => startJob({ kind: 'audit_verify', project: r.project }) });
    items.push({ t: 'Self-test the sandbox', k: 'selftest', f: () => startJob({ kind: 'selftest', project: r.project }) });
    for (const [id, l] of [['journey', 'Journey'], ['specs', 'Specs'], ['agent', 'Agent'], ['runs', 'Runs & evidence'], ['audit', 'Audit'], ['settings', 'Settings']]) items.push({ t: l + ' tab', k: r.project, f: () => go('p/' + r.project + (id === 'journey' ? '' : '/' + id)) });
    if (p && p.report) for (const s of p.report.specs.filter((x) => !x.complete)) items.push({ t: 'Open spec ' + s.slug, k: s.label, f: () => go('p/' + r.project + '/spec/' + s.slug) });
  }
  if (ov) for (const p of ov.projects) {
    const need = p.up && needsYou(p);
    if (need && !r.project) items.push({ t: `${p.name}: ${need}`, k: 'needs you', f: () => go('p/' + p.name + (p.active_spec ? '/spec/' + p.active_spec.slug : '')) });
    items.push({ t: 'Open project ' + p.name, k: p.up ? 'up' : 'down', f: () => go('p/' + p.name) });
  }
  items.push({ t: 'Harbour (all projects)', k: 'home', f: () => go('') });
  items.push({ t: 'New project', k: 'new', f: () => go('new') });
  items.push({ t: 'Import a repository', k: 'import', f: () => go('import') });
  items.push({ t: 'Toggle theme', k: 'theme', f: toggleTheme });
  items.push({ t: 'Refresh', k: 'r', f: refresh });
  return items;
}
function openPalette() {
  const root = document.getElementById('palette');
  root.classList.remove('hidden');
  clear(root);
  let q = '', idx = 0;
  const all = paletteItems();
  const list = h('div', { class: 'items' });
  const draw = () => {
    clear(list);
    const items = all.filter((i) => !q || (i.t + ' ' + i.k).toLowerCase().includes(q.toLowerCase()));
    items.forEach((i, n) => list.appendChild(h('div', { class: 'item' + (n === idx ? ' on' : ''), onclick: () => { closePalette(); i.f(); } }, h('span', {}, i.t), h('span', { class: 'k' }, i.k))));
    if (!items.length) list.appendChild(h('div', { class: 'item dim' }, 'Nothing matches. Open a project for its actions (go, approve, ship, agent…).'));
    return items;
  };
  let items = draw();
  const input = h('input', { placeholder: 'What do you want to do?', oninput: (e) => { q = e.target.value; idx = 0; items = draw(); }, onkeydown: (e) => {
    if (e.key === 'ArrowDown') { idx = Math.min(idx + 1, items.length - 1); draw(); e.preventDefault(); }
    else if (e.key === 'ArrowUp') { idx = Math.max(idx - 1, 0); draw(); e.preventDefault(); }
    else if (e.key === 'Enter' && items[idx]) { closePalette(); items[idx].f(); }
    else if (e.key === 'Escape') closePalette();
  } });
  root.appendChild(h('div', { class: 'box', onclick: (e) => e.stopPropagation() }, input, list));
  root.onclick = closePalette;
  input.focus();
}
function closePalette() { document.getElementById('palette').classList.add('hidden'); }

/* ---------- render ---------- */
function render() {
  renderRail();
  const main = clear(document.getElementById('main'));
  const r = S.route;
  const p = r.project ? S.projects[r.project] : null;
  const crumbs = h('div', { class: 'crumbs' }, h('a', { href: '#/', style: 'color:var(--text-muted);font-weight:500' }, 'moor'));
  if (r.view === 'project') { crumbs.appendChild(h('span', { class: 'sep' }, '/')); crumbs.appendChild(h('a', { href: '#/p/' + r.project, style: 'color:var(--text)' }, r.project)); if (r.slug) { crumbs.appendChild(h('span', { class: 'sep' }, '/')); crumbs.appendChild(h('span', { class: 'slug' }, r.slug)); } }
  else if (r.view !== 'home') { crumbs.appendChild(h('span', { class: 'sep' }, '/')); crumbs.appendChild(h('span', {}, r.view === 'new' ? 'New project' : 'Import')); }
  main.appendChild(h('div', { class: 'topbar' }, crumbs, r.view === 'project' ? projectTabs(p, r) : null, h('span', { class: 'right' }), Object.keys(S.loading).length ? h('span', { class: 'spin', style: 'width:12px;height:12px;border:2px solid var(--text-dim);border-right-color:transparent;border-radius:50%;animation:spin 0.8s linear infinite' }) : null));
  const view = h('div', { class: 'view' });
  let content;
  try {
    content = r.view === 'home' ? viewHome() : r.view === 'new' ? viewNew() : r.view === 'import' ? viewImport() : viewProject(r);
  } catch (e) {
    content = h('div', { class: 'banner fail' }, h('div', {}, h('b', {}, 'The page hit an error drawing this view.'), h('pre', { class: 'raw' }, String(e.stack || e))));
    console.error(e);
  }
  append(view, [content]);
  main.appendChild(view);
}

/* ---------- boot ---------- */
(function boot() {
  try { const t = localStorage.getItem('moor-theme'); if (t) document.documentElement.setAttribute('data-theme', t); } catch (_) {}
  if (!TOKEN) {
    document.getElementById('main').appendChild(h('div', { class: 'view' }, h('div', { class: 'banner warn' }, h('div', {}, h('b', {}, 'No console key.'), ' Open the exact URL that `moor ui` printed in the terminal; the key after # is this run’s only way in.'))));
    return;
  }
  S.route = parseRoute();
  document.addEventListener('keydown', (e) => {
    if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'k') { e.preventDefault(); openPalette(); }
    else if (e.key === 'Escape') { closePalette(); closeModal(); }
    else if (e.key === 'r' && !e.metaKey && !e.ctrlKey && !/input|textarea|select/i.test(e.target.tagName)) refresh();
  });
  api('/jobs').then((d) => { for (const j of d.jobs) { j.lines = []; S.jobs.push(j); S.jobById[j.id] = j; if (!j.done) subscribe(j); } renderDock(); }).catch(() => {});
  api('/images').then((d) => { S.images = d.images; }).catch(() => {});
  setInterval(() => { if (document.visibilityState === 'visible' && S.jobs.some((j) => !j.done)) renderDock(); }, 1000);
  renderDock();
  render();
  loadOverview();
})();
