//! `bicraft-web`: the browser entry point (docs/webui.md). Built for wasm32-unknown-unknown by
//! `cargo xtask webui`; natively it only says so.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]

#[cfg(target_arch = "wasm32")]
fn main() {
    bicraft_web::web::start();
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    eprintln!(
        "bicraft-web runs in a browser: build it with `cargo xtask webui` and open bicraft-cli view's /app/ (docs/webui.md)"
    );
}
