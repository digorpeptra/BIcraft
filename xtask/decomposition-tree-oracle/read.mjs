// Decomposition tree oracle reader (docs/render.md, "Decomposition tree"). Opens every page of
// plan.json in the report view of one Power BI Desktop instance through WebView2's DevTools
// protocol (no screen, no window focus) and reads what the page's decomposition tree draws:
// nodes (label, value, bar, position, on the selected path), level headers (title, subtitle,
// AI marker), connector lines, scroll arrows. This only observes the rendered page; nothing is
// copied from Desktop's code.
// usage: node read.mjs DEVTOOLS_PORT PLAN.json OUT.json [SETTLE_MS] [SHOTS_DIR]
import { readFileSync, writeFileSync, mkdirSync } from 'node:fs';
import { join } from 'node:path';

const [port, planFile, outFile, settleArg, shots] = process.argv.slice(2);
const SETTLE = Number(settleArg || 6000);
const plan = JSON.parse(readFileSync(planFile, 'utf8'));
const list = await (await fetch(`http://127.0.0.1:${port}/json`)).json();
const target = list.find(p => p.type === 'page' && p.url.includes('reportView'));
if (!target) { console.error('no report view page on DevTools port', port); process.exit(2); }
const ws = new WebSocket(target.webSocketDebuggerUrl);
let nextId = 0; const pending = new Map();
ws.onmessage = e => { const m = JSON.parse(e.data); if (m.id && pending.has(m.id)) { pending.get(m.id)(m); pending.delete(m.id); } };
await new Promise(r => ws.onopen = r);
const send = (method, params = {}) => new Promise(r => { const i = ++nextId; const t = setTimeout(() => { pending.delete(i); r({ result: { exceptionDetails: { text: 'timeout: ' + method } } }); }, 30000);
  pending.set(i, m => { clearTimeout(t); r(m); }); ws.send(JSON.stringify({ id: i, method, params })); });
const ev = async (expr) => {
  const r = await send('Runtime.evaluate', { expression: expr, returnByValue: true, awaitPromise: true });
  if (r.result?.exceptionDetails) throw new Error('page script: ' + JSON.stringify(r.result.exceptionDetails).slice(0, 400));
  return r.result?.result?.value;
};
const sleep = ms => new Promise(r => setTimeout(r, ms));

const READ = (rect) => `(() => {
  const area = document.querySelector('.displayArea'); const a = area.getBoundingClientRect(); const k = ${plan.width} / a.width;
  const rect = ${JSON.stringify(rect)};
  let best = null, bd = 1e9;
  for (const c of area.querySelectorAll('.visualContainer')) { const r = c.getBoundingClientRect(); const d = Math.abs((r.x - a.x) * k - rect[0]) + Math.abs((r.y - a.y) * k - rect[1]); if (d < bd && c.querySelector('.visual-decompositionTreeVisual')) { bd = d; best = c; } }
  if (!best || bd > 20) return null;
  const v = best.querySelector('.visual-decompositionTreeVisual'); const o = v.getBoundingClientRect();
  const box = (e) => { const r = e.getBoundingClientRect(); return { x: r.x - o.x, y: r.y - o.y, w: r.width, h: r.height }; };
  const txt = (e) => e ? (e.textContent || '').trim() : null;
  const nodes = [...v.querySelectorAll('g.treeNode')].map(g => { const b = box(g.querySelector('rect.dataBarBackground') || g); const val = g.querySelector('rect.valueDataBar'); const lab = g.querySelector('tspan.label');
    return { x: b.x, y: b.y, bg: b.w, bar: val ? box(val).w : null, label: txt(lab), value: txt(g.querySelector('tspan.value')), path: !!(lab && lab.classList.contains('pathNode')), opacity: getComputedStyle(g).opacity,
      fill: val ? getComputedStyle(val).fill : null, font: lab ? getComputedStyle(lab).fontSize : null, weight: lab ? getComputedStyle(lab).fontWeight : null, bar_y: val ? box(val).y : null, bar_h: val ? box(val).h : null, label_y: lab ? box(lab).y : null, value_y: g.querySelector('tspan.value') ? box(g.querySelector('tspan.value')).y : null }; });
  const headers = [...v.querySelectorAll('g.levelHeader')].map(h => { const t = h.querySelector('tspan.levelTitle'); if (!t) return null; const own = (e) => e ? [...e.childNodes].filter(n => n.nodeType === 3).map(n => n.textContent).join('').trim() : null; const ai = h.querySelector('tspan.aiLevel'); const s = h.querySelector('tspan.levelSubtitle');
    return { x: box(t).x, title: own(t), subtitle: own(s) || null, ai: !!(ai && ai.getBoundingClientRect().width > 0), note: (h.querySelector('rect.iconOverlay') || {getAttribute: () => null}).getAttribute('aria-label') }; }).filter(Boolean);
  const links = [...v.querySelectorAll('path.link')].map(p => ({ ...box(p), stroke: getComputedStyle(p).stroke, width: getComputedStyle(p).strokeWidth, d: p.getAttribute('d') }));
  const more = [...v.querySelectorAll('text.loadMore')].map(m => box(m));
  const first = nodes.length ? Math.min(...nodes.map(n => n.y)) : 0;
  return { k, width: o.width, height: o.height, nodes, headers, links,
    more_down: more.filter(m => m.y > first).length, more_up: more.filter(m => m.y <= first).length, more,
    bar_width: nodes.length ? Math.max(...nodes.map(n => n.bg)) : null };
})()`;

const tab = async (name) => ev(`(() => { const t = [...document.querySelectorAll('[role=tab]')].find(t => t.textContent.trim().replace(/x$/, '') === ${JSON.stringify(name)}); if (!t) return false; t.click(); return true; })()`);

const out = { version: 1, devtools: Number(port), settle_ms: SETTLE, started: new Date().toISOString(), pages: [], errors: [] };
if (shots) mkdirSync(shots, { recursive: true });
for (const p of plan.pages) {
  try {
    const ok = await tab(p.page);
    if (!ok) { out.errors.push({ page: p.page, error: 'page tab not found' }); continue; }
    await send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: 1, y: 1 });
    await sleep(SETTLE);
    const tree = await ev(READ([p.x, p.y, p.width, p.height]));
    out.pages.push({ page: p.page, visual: p.visual, tree });
    if (shots) { const s = await send('Page.captureScreenshot', { format: 'png' }); if (s.result?.data) writeFileSync(join(shots, p.page + '.png'), Buffer.from(s.result.data, 'base64')); }
    console.log(p.page, tree ? tree.nodes.length + ' nodes' : 'no tree');
  } catch (e) { out.errors.push({ page: p.page, error: String(e).slice(0, 300) }); }
  writeFileSync(outFile, JSON.stringify(out, null, 1));
}
out.finished = new Date().toISOString();
writeFileSync(outFile, JSON.stringify(out, null, 1));
ws.close();
