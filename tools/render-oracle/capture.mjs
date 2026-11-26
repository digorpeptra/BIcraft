// usage: node capture.mjs PORT OUTDIR PAGEWIDTH PAGE1 PAGE2 ...
// Clicks each page tab in Desktop's report view and captures the page canvas (.displayArea)
// through the DevTools protocol (no screen capture), scaled to PAGEWIDTH pixels.
import { writeFileSync, mkdirSync } from 'node:fs';
const [port, out, pw, ...pages] = process.argv.slice(2);
mkdirSync(out, { recursive: true });
const list = await (await fetch(`http://127.0.0.1:${port}/json`)).json();
const page = list.find(p => p.type === 'page' && p.url.includes('reportView'));
const ws = new WebSocket(page.webSocketDebuggerUrl);
let id = 0; const pending = new Map();
ws.onmessage = e => { const m = JSON.parse(e.data); if (m.id && pending.has(m.id)) { pending.get(m.id)(m); pending.delete(m.id); } };
await new Promise(r => ws.onopen = r);
const send = (method, params = {}) => new Promise(r => { const i = ++id; pending.set(i, r); ws.send(JSON.stringify({ id: i, method, params })); });
const ev = async expr => (await send('Runtime.evaluate', { expression: expr, returnByValue: true, awaitPromise: true })).result?.result?.value;
const sleep = ms => new Promise(r => setTimeout(r, ms));
for (let i = 0; i < pages.length; i++) {
  const name = pages[i];
  const ok = await ev(`(() => { const t = [...document.querySelectorAll('[role=tab]')].find(t => t.textContent.trim().replace(/x$/, '') === ${JSON.stringify(name)}); if (!t) return false; t.click(); return true; })()`);
  // park the pointer so no visual shows its hover state (header icons, tile highlight)
  await send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: 1, y: 1 });
  await sleep(9000);
  const r = await ev(`(() => { const e = document.querySelector('.displayArea'); const r = e.getBoundingClientRect(); return [r.x, r.y, r.width, r.height]; })()`);
  // every text node's computed style and box (page pixels relative to the canvas, before the
  // capture's scaling): NN-<page>.json next to the PNG, for checking font sizes and colours
  const txt = await ev(`(() => { const area = document.querySelector('.displayArea'); const ar = area.getBoundingClientRect(); const out = [];
    const w = document.createTreeWalker(area, NodeFilter.SHOW_TEXT); let n;
    while ((n = w.nextNode())) { const t = n.textContent.trim(); if (!t) continue; const e = n.parentElement; const cs = getComputedStyle(e);
      const rg = document.createRange(); rg.selectNodeContents(n); const b = rg.getBoundingClientRect(); if (b.width === 0) continue;
      // scale: screen px per CSS px of the font size (SVG transforms, CSS transforms); vis: false
      // for visually hidden text (opacity 0, hidden, clipped to a pixel: accessibility headings)
      let scale = 1; if (e instanceof SVGGraphicsElement && e.getScreenCTM()) { const m = e.getScreenCTM(); scale = Math.hypot(m.a, m.b); }
      else if (e.offsetWidth > 0) scale = e.getBoundingClientRect().width / e.offsetWidth;
      let vis = true; for (let a = e; a && a !== area; a = a.parentElement) { const s = getComputedStyle(a); const r = a.getBoundingClientRect();
        if (s.opacity === '0' || s.visibility === 'hidden' || s.display === 'none' || s.clipPath === 'inset(50%)' || /rect\\(0px,? 0px,? 0px,? 0px\\)/.test(s.clip)
          || (s.overflow !== 'visible' && (r.width <= 1 || r.height <= 1))) { vis = false; break; } }
      out.push({ t, tag: e.tagName, ff: cs.fontFamily, fs: cs.fontSize, fw: cs.fontWeight, color: cs.color, fill: cs.fill, x: b.x - ar.x, y: b.y - ar.y, w: b.width, h: b.height, z: ar.width, scale, vis }); }
    return out; })()`);
  writeFileSync(`${out}/${String(i + 1).padStart(2, '0')}-${name}.json`, JSON.stringify(txt, null, 1));
  const shot = await send('Page.captureScreenshot', { format: 'png', captureBeyondViewport: false, clip: { x: r[0], y: r[1], width: r[2], height: r[3], scale: Number(pw) / r[2] } });
  const file = `${out}/${String(i + 1).padStart(2, '0')}-${name}.png`;
  if (shot.result?.data) { writeFileSync(file, Buffer.from(shot.result.data, 'base64')); console.log(name, ok, file); }
  else console.log(name, ok, 'no screenshot', JSON.stringify(shot).slice(0, 300));
}
ws.close();
