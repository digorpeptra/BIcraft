// Interaction oracle driver (docs/interaction-oracle.md). Plays the steps of plan.json in the
// report view of one Power BI Desktop instance through WebView2's DevTools protocol (no screen,
// no window focus) and records, after each step, what every visual of the page displays.
// usage: node drive.mjs DEVTOOLS_PORT PLAN.json OUT.json [SETTLE_MS]
//
// Readings come from what the page draws and exposes to assistive technology: element geometry,
// fill opacity, text and ARIA attributes (accessible names, `aria-selected`, `aria-posinset`).
// Clicks and keys are sent as input events. No script object of the page is read, and nothing is
// copied from Desktop's code.
import { readFileSync, writeFileSync } from 'node:fs';

const [port, planFile, outFile, settleArg] = process.argv.slice(2);
const SETTLE = Number(settleArg || 6000);
const plan = JSON.parse(readFileSync(planFile, 'utf8'));
const list = await (await fetch(`http://127.0.0.1:${port}/json`)).json();
const target = list.find(p => p.type === 'page' && p.url.includes('reportView'));
if (!target) { console.error('no report view page on DevTools port', port); process.exit(2); }
const ws = new WebSocket(target.webSocketDebuggerUrl);
let nextId = 0; const pending = new Map();
ws.onmessage = e => { const m = JSON.parse(e.data); if (m.id && pending.has(m.id)) { pending.get(m.id)(m); pending.delete(m.id); } };
await new Promise(r => ws.onopen = r);
// every call times out (a dialog or a reload of the view must not hang the run)
const send = (method, params = {}) => new Promise(r => { const i = ++nextId; const t = setTimeout(() => { pending.delete(i); r({ result: { exceptionDetails: { text: 'timeout: ' + method } } }); }, 30000);
  pending.set(i, m => { clearTimeout(t); r(m); }); ws.send(JSON.stringify({ id: i, method, params })); });
ws.onclose = () => { console.error('DevTools connection closed'); for (const [, f] of pending) f({ result: { exceptionDetails: { text: 'connection closed' } } }); pending.clear(); };
const ev = async (expr) => {
  const r = await send('Runtime.evaluate', { expression: expr, returnByValue: true, awaitPromise: true });
  if (r.result?.exceptionDetails) throw new Error('page script: ' + JSON.stringify(r.result.exceptionDetails).slice(0, 400));
  return r.result?.result?.value;
};
const sleep = ms => new Promise(r => setTimeout(r, ms));
const now = () => new Date().toISOString();

// Page-side helpers, installed once (and again after a reload of the view). They read the DOM
// only: geometry, computed fill opacity, text and ARIA attributes.
const HELPERS = String.raw`
window.__pbcOracle = (() => {
  const area = () => document.querySelector('.displayArea');
  const scale = () => { const a = area().getBoundingClientRect(); return { a, k: ${plan.width} / a.width }; };
  const containers = () => { const { a, k } = scale(); return [...area().querySelectorAll('.visualContainer')].map(v => {
    const r = v.getBoundingClientRect(); const vis = v.querySelector('.visual');
    return { el: v, x: (r.x - a.x) * k, y: (r.y - a.y) * k, w: r.width * k, h: r.height * k,
             type: ((vis && vis.className.toString().match(/visual-(\w+)/)) || [])[1] || null }; }); };
  const find = (rect) => { let best = null, bd = 1e9; for (const c of containers()) { const d = Math.abs(c.x - rect[0]) + Math.abs(c.y - rect[1]); if (d < bd) { bd = d; best = c; } } return bd <= 12 ? best : null; };
  const center = (el) => { const b = el.getBoundingClientRect(); return [b.x + b.width / 2, b.y + b.height / 2]; };
  const clean = (t) => (t || '').replace(/\s+/g, ' ').trim();
  const classes = (e) => (e.getAttribute('class') || '').split(/\s+/);
  // a number as an accessible name shows it ("6171", "1 234,5")
  const num = (s) => { if (s == null) return undefined; const t = String(s).replace(/[\s  ]/g, '').replace(',', '.'); if (t === '') return undefined; const x = Number(t); return Number.isFinite(x) ? x : undefined; };
  // a category as shown: a number when the text is one (month numbers), else the text
  const cat = (t) => { const x = num(t); return x !== undefined && /^-?[\d\s  .,]+$/.test(t) ? x : t; };
  // an axis tick's text (its title holds the whole text when the label is cut short)
  const tickText = (e) => { const t = e.querySelector('title'); return clean(t ? t.textContent : e.textContent); };
  // "<field> <category>. <measure> <number> (<share>)." -> [category text, number]
  const parseLabel = (label, field) => { const parts = (label || '').replace(/\.\s*$/, '').split('. ');
    let c = parts[0] || ''; if (field && c.startsWith(field + ' ')) c = c.slice(field.length + 1);
    const m = (parts[1] || '').replace(/\s*\([^)]*\)\s*$/, '').match(/(-?[\d\s  .,]*\d)\s*$/);
    return [c, m ? num(m[1]) : undefined]; };
  const arcRadius = (e) => { const m = (e.getAttribute('d') || '').match(/A\s*(-?[\d.]+(?:e-?\d+)?)/); return m ? Number(m[1]) : undefined; };
  // The marks a chart draws. Bars and columns: the value is the mark's accessible name, the
  // category the axis tick at the same position, the highlighted part the overlay drawn over the
  // mark (its length over the mark's). Pie slices: category and value from the accessible name,
  // the highlighted share from the overlay's radius (its area over the slice's). Line points are
  // read by keyboard (lineFocus, linePoint).
  const marks = (root, field) => { const out = [];
    const ticks = new Map(); for (const t of root.querySelectorAll('[data-automation-type=axis-tick-text][aria-posinset]')) ticks.set(t.getAttribute('aria-posinset'), tickText(t));
    const overlays = [];
    for (const e of root.querySelectorAll('svg rect, svg path')) { const cl = classes(e); const tag = e.tagName.toLowerCase();
      const isBar = tag === 'rect' && (cl.includes('bar') || cl.includes('column'));
      if (isBar && cl.includes('highlight')) { overlays.push(e); continue; }
      if (tag === 'path' && cl.includes('slice-highlight')) { overlays.push(e); continue; }
      const label = e.getAttribute('aria-label'); if (label == null) continue;
      if (isBar) { const p = e.getAttribute('aria-posinset');
        out.push({ el: e, kind: cl.includes('bar') ? 'bar' : 'column', cat: cat(ticks.get(p) ?? ''), value: num(label), selected: e.getAttribute('aria-selected') === 'true', opacity: getComputedStyle(e).fillOpacity }); continue; }
      if (tag === 'path' && cl.includes('slice')) { const [c, v] = parseLabel(label, field);
        out.push({ el: e, kind: 'slice', cat: cat(c), value: v, selected: e.getAttribute('aria-selected') === 'true', opacity: getComputedStyle(e).fillOpacity, r: arcRadius(e) }); } }
    // an overlay belongs to the bar at the same place, or to the slice of the same rank
    const slices = out.filter(m => m.kind === 'slice'); let si = 0;
    for (const o of overlays) {
      if (o.tagName.toLowerCase() === 'path') { const s = slices[si++]; const r = arcRadius(o); if (s && s.r && r !== undefined) s.ratio = (r / s.r) ** 2; continue; }
      const x = Number(o.getAttribute('x')), y = Number(o.getAttribute('y')), w = Number(o.getAttribute('width')), h = Number(o.getAttribute('height'));
      const m = out.find(m => (m.kind === 'column' && Number(m.el.getAttribute('x')) === x) || (m.kind === 'bar' && Number(m.el.getAttribute('y')) === y));
      if (!m || m.value === undefined) continue;
      // an overlay of no length draws nothing: no highlighted part is shown
      const full = m.kind === 'column' ? Number(m.el.getAttribute('height')) : Number(m.el.getAttribute('width'));
      const part = m.kind === 'column' ? h : w;
      if (full > 0 && part > 0) m.highlight = m.value * part / full; }
    return out; };
  const markSummary = (root, field) => marks(root, field).map(m => { const p = { cat: m.cat, value: m.value };
      if (m.highlight !== undefined) p.highlight = m.highlight; if (m.ratio !== undefined) p.ratio = m.ratio;
      if (m.selected) p.selected = true; if (m.opacity !== '1' && m.opacity !== '') p.dimmed = true; return p; });
  const table = (root) => [...root.querySelectorAll('[role=row]')].map(r => ({ cells: [...r.querySelectorAll('[role=columnheader],[role=gridcell],[role=rowheader]')].filter(c => !c.classList.contains('prefix-cell')).map(c => clean(c.innerText)), selected: r.getAttribute('aria-selected') === 'true' })).filter(r => r.cells.length);
  const slicerItems = (root) => [...root.querySelectorAll('.slicerItemContainer, [role=option]')].map(i => ({ text: clean(i.getAttribute('title') || i.innerText), selected: i.getAttribute('aria-selected') === 'true' || i.getAttribute('aria-checked') === 'true' || i.classList.contains('selected') || !!i.querySelector('.partiallySelected, .selected') }));
  const read = (spec) => { const out = []; for (const v of spec) { const c = find([v.x, v.y]); if (!c) { out.push({ name: v.name, found: false }); continue; }
      const r = { name: v.name, found: true, dom_type: c.type, text: clean(c.el.innerText).slice(0, 2000) };
      const aria = [...c.el.querySelectorAll('svg.card, [aria-label]')].find(e => e.matches('svg.card')); if (aria) r.aria = aria.getAttribute('aria-label');
      if (/Chart$/.test(c.type || '') && c.type !== 'lineChart') r.points = markSummary(c.el, v.field);
      if (c.type === 'tableEx') r.rows = table(c.el);
      if (/[sS]licer/.test(c.type || '')) { r.items = slicerItems(c.el); const inp = [...c.el.querySelectorAll('input')].map(i => i.value).filter(x => x !== ''); if (inp.length) r.inputs = inp; }
      out.push(r); }
    return out; };
  // A line chart draws one focusable point at a time: focus it; the driver moves it with the
  // arrow keys and reads each point's accessible name and place.
  const lineFocus = (rect) => { const c = find(rect); if (!c || c.type !== 'lineChart') return null; const p = c.el.querySelector('circle.focus-element'); if (!p) return null; p.focus(); return Number(p.getAttribute('aria-setsize')) || null; };
  const linePoint = (field) => { const p = document.activeElement; if (!p || !classes(p).includes('focus-element')) return null; const [c, v] = parseLabel(p.getAttribute('aria-label'), field);
    return { pos: Number(p.getAttribute('aria-posinset')), cat: cat(c), value: v, selected: p.getAttribute('aria-selected') === 'true', at: center(p) }; };
  const blur = () => { const a = document.activeElement; if (a && a.blur) a.blur(); return true; };
  // where to click: a chart mark, a table row (its first cell), a slicer item, a button
  const point = (rect, label, field) => { const c = find(rect); if (!c) return null;
    const m = marks(c.el, field).filter(m => String(m.cat) === String(label));
    if (m.length) return center(m[0].el);
    for (const cell of c.el.querySelectorAll('[role=gridcell],[role=rowheader]')) if (!cell.classList.contains('prefix-cell') && clean(cell.innerText) === label) return center(cell);
    return null; };
  const item = (rect, label) => { const c = find(rect); if (!c) return null;
    for (const i of c.el.querySelectorAll('.slicerItemContainer, [role=option]')) { if (clean(i.getAttribute('title') || i.innerText) === label) return center(i); }
    for (const t of c.el.querySelectorAll('.slicerText, span, div')) if (t.children.length === 0 && clean(t.innerText) === label) return center(t);
    return null; };
  const box = (rect) => { const c = find(rect); return c ? center(c.el) : null; };
  const tab = (name) => { const t = [...document.querySelectorAll('[role=tab]')].find(t => t.textContent.trim().replace(/x$/, '') === name); if (!t) return false; t.click(); return true; };
  const activeTab = () => { const t = [...document.querySelectorAll('[role=tab][aria-selected=true]')].map(t => t.textContent.trim().replace(/x$/, '')); return t[t.length - 1] || null; };
  return { read, point, item, box, tab, activeTab, lineFocus, linePoint, blur, containers: () => containers().map(c => ({ x: c.x, y: c.y, w: c.w, h: c.h, type: c.type })) };
})(); true`;

const install = () => ev(HELPERS);
const call = async (fn, ...args) => {
  const expr = `(window.__pbcOracle || (${'eval'})(${JSON.stringify(HELPERS)}) && window.__pbcOracle).${fn}(${args.map(a => JSON.stringify(a)).join(',')})`;
  return ev(expr);
};
const click = async (xy, ctrl) => {
  const [x, y] = xy; const modifiers = ctrl ? 2 : 0;
  await send('Input.dispatchMouseEvent', { type: 'mouseMoved', x, y, modifiers });
  await send('Input.dispatchMouseEvent', { type: 'mousePressed', x, y, button: 'left', buttons: 1, clickCount: 1, modifiers });
  await send('Input.dispatchMouseEvent', { type: 'mouseReleased', x, y, button: 'left', buttons: 0, clickCount: 1, modifiers });
  await send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: 1, y: 1 });
};
const key = async (k, code) => {
  await send('Input.dispatchKeyEvent', { type: 'keyDown', key: k, code: k, windowsVirtualKeyCode: code });
  await send('Input.dispatchKeyEvent', { type: 'keyUp', key: k, code: k, windowsVirtualKeyCode: code });
};
// The column name of a visual's category field (`Date[Month]` -> `Month`): accessible names start
// with it.
const fieldOf = v => { const f = (v.fields || []).find(f => f[0] === 'Category' || f[0] === 'Axis'); const m = f && /\[([^\]]+)\]$/.exec(f[1]); return m ? m[1] : null; };
// Every point of a line chart, in axis order: its focusable point moved with the right arrow.
const linePoints = async (v) => {
  const n = await call('lineFocus', [v.x, v.y]); if (!n) return null;
  const by = new Map();
  for (let k = 0; k <= n && by.size < n; k++) {
    const p = await call('linePoint', fieldOf(v)); if (p && !by.has(p.pos)) by.set(p.pos, p);
    await key('ArrowRight', 39); await sleep(150);
  }
  await call('blur');
  return [...by.values()].sort((a, b) => a.pos - b.pos);
};
const pageOf = id => plan.scenarios.find(s => s.page === id);
const visualOf = (sc, name) => sc.visuals.find(v => v.name === name) || null;
const goto = async (id) => { const ok = await call('tab', id); await send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: 1, y: 1 }); await sleep(SETTLE + 2000); return ok; };
const readPage = async (sc) => {
  const reading = await call('read', sc.visuals.map(v => ({ name: v.name, x: v.x, y: v.y, field: fieldOf(v) })));
  for (const r of reading || []) {
    if (r.dom_type !== 'lineChart') continue;
    const pts = await linePoints(visualOf(sc, r.name));
    if (pts) r.points = pts.map(p => { const o = { cat: p.cat, value: p.value }; if (p.selected) o.selected = true; return o; });
  }
  return reading;
};

await install();
const out = { version: 2, devtools: Number(port), settle_ms: SETTLE, started: now(), steps: [], errors: [] };
const only = process.env.PBC_ORACLE_ONLY ? process.env.PBC_ORACLE_ONLY.split(',') : null;
for (const sc of plan.scenarios) {
  if (!sc.steps.length) continue;
  if (only && !only.includes(sc.page)) continue;
  const navStart = now();
  const shown = await goto(sc.page);
  if (!shown) { out.errors.push({ page: sc.page, error: 'page tab not found' }); continue; }
  let windowStart = navStart;
  for (const st of sc.steps) {
    const stepStart = st.actions.length ? now() : windowStart;
    let onPage = sc;
    const done = [];
    for (const a of st.actions) {
      try {
        if (a.do === 'page') { const at = now(); const ok = await goto(a.page); onPage = pageOf(a.page) || onPage; done.push({ ...a, ok, at }); continue; }
        const v = visualOf(onPage, a.visual);
        const rect = v ? [v.x, v.y] : null;
        let xy = null;
        if (a.do === 'click') {
          xy = await call('point', rect, a.point, v && fieldOf(v));
          if (!xy && v && v.type === 'lineChart') { const p = (await linePoints(v) || []).find(p => String(p.cat) === String(a.point)); xy = p ? p.at : null; }
        }
        else if (a.do === 'item') xy = await call('item', rect, a.item);
        else if (a.do === 'button') xy = await call('box', rect);
        if (!xy) { done.push({ ...a, ok: false, error: 'target not found' }); out.errors.push({ step: st.id, action: a, error: 'target not found' }); continue; }
        await click(xy, a.ctrl || a.do === 'button');
        done.push({ ...a, ok: true, at: now() });
        await sleep(2500);
      } catch (e) { done.push({ ...a, ok: false, error: String(e).slice(0, 300) }); out.errors.push({ step: st.id, error: String(e).slice(0, 300) }); }
    }
    await sleep(SETTLE);
    let reading = null;
    try { reading = await readPage(onPage); } catch (e) { out.errors.push({ step: st.id, error: 'read: ' + String(e).slice(0, 300) }); }
    const stepEnd = now();
    out.steps.push({ id: st.id, page: sc.page, nav_start: navStart, shown_page: onPage.page, active_tab: await call('activeTab'), start: stepStart, end: stepEnd, actions: done, visuals: reading });
    console.log(st.id, done.map(d => d.ok).join(','), reading ? reading.filter(r => r.found).length + '/' + reading.length : 'no reading');
    windowStart = stepEnd;
    writeFileSync(outFile, JSON.stringify(out, null, 1));
  }
}
out.finished = now();
writeFileSync(outFile, JSON.stringify(out, null, 1));
ws.close();
