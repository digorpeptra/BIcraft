// usage: node probe.mjs PORT
// Prints one JSON line: the page tabs of Desktop's report view and the visible text of the page
// canvas (.displayArea), read through the WebView2 DevTools protocol. Nothing is clicked.
const port = process.argv[2];
const out = { tabs: null, text: '' };
try {
  const list = await (await fetch(`http://127.0.0.1:${port}/json`)).json();
  const page = list.find(p => p.type === 'page' && p.url.includes('reportView'));
  if (page) {
    const ws = new WebSocket(page.webSocketDebuggerUrl);
    await new Promise((r, j) => { ws.onopen = r; ws.onerror = j; });
    const res = await new Promise(r => {
      ws.onmessage = e => { const m = JSON.parse(e.data); if (m.id === 1) r(m); };
      ws.send(JSON.stringify({ id: 1, method: 'Runtime.evaluate', params: { returnByValue: true, expression:
        `(() => ({ tabs: [...document.querySelectorAll('[role=tab]')].map(t => t.textContent.trim()),
                   text: (document.querySelector('.displayArea')?.innerText || '').slice(0, 2000) }))()` } }));
    });
    ws.close();
    const v = res.result?.result?.value;
    if (v) { out.tabs = v.tabs; out.text = v.text; }
  }
} catch (e) { out.error = String(e).slice(0, 200); }
console.log(JSON.stringify(out));
