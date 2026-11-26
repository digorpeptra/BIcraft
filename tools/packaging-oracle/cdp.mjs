// usage: node cdp.mjs PORT "JS expression"
// Evaluates one expression in Desktop's report-view page (WebView2 DevTools protocol) and prints
// the JSON result. Meant for expressions that read or click the page's DOM (controls, text,
// attributes); never for reading the page's script objects.
const [port, expr] = process.argv.slice(2);
const list = await (await fetch(`http://127.0.0.1:${port}/json`)).json();
const page = list.find(p => p.type === 'page' && p.url.includes('reportView'));
if (!page) { console.log('null'); process.exit(0); }
const ws = new WebSocket(page.webSocketDebuggerUrl);
await new Promise(r => { ws.onopen = r; });
const res = await new Promise(r => {
  ws.onmessage = e => { const m = JSON.parse(e.data); if (m.id === 1) r(m); };
  ws.send(JSON.stringify({ id: 1, method: 'Runtime.evaluate', params: { expression: expr, returnByValue: true, awaitPromise: true } }));
});
ws.close();
console.log(JSON.stringify(res.result?.result?.value ?? res.result?.exceptionDetails?.text ?? null));
