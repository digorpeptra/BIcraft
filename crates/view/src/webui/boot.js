// Starts the browser build of the BIcraft app (apps/bicraft-web, docs/webui.md): the module that
// wasm-bindgen generated, then its WebAssembly. Its `main` takes over the canvas.
import init from "./bicraft_web.js";

init({ module_or_path: new URL("./bicraft_web_bg.wasm", import.meta.url) }).catch((e) => {
  const el = document.getElementById("bicraft_loading");
  if (el) el.textContent = "BIcraft could not start: " + e + ". A browser with WebGPU or WebGL2 is required.";
});
