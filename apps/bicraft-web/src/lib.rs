//! The BIcraft app in the browser (docs/webui.md): the shared UI ([`bicraft_ui::App`], the same
//! as `bicraft`'s), compiled to WebAssembly and started by eframe's web runner (wgpu:
//! WebGPU, else WebGL2). One UI, two shells: this crate only supplies the browser's shell and data
//! source, which reaches the viewer API of the server that served the page (`bicraft-cli view` at
//! `/app/`, `bicraft-cli serve` at `/w/<id>/view/app/`) with same-origin `fetch` and `EventSource`.
//!
//! - [`protocol`]: the URLs, the page's configuration and the reading of answers (pure, tested
//!   natively);
//! - `web` (wasm32 only): the eframe web runner and the driven data source
//!   ([`bicraft_ui::source::Running::Driven`]).
//!
//! Credentials: the browser's own (the viewer's login cookie or the auth proxy's session), sent
//! by the browser with same-origin requests. The page's interaction token is read from its
//! `<meta>` slot and kept in memory. Browser storage holds the UI preferences only (light or
//! dark, the zoom, the ribbon, the tab; `localStorage`), never a token, a path or report data.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]

pub mod protocol;

#[cfg(target_arch = "wasm32")]
pub mod web;
