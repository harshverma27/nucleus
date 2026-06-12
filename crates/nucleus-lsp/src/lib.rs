//! The Nucleus language server.
//!
//! A [`tower-lsp`](tower_lsp) server that wraps [`nucleus_compiler`] to give
//! editors live `stm32.toml` feedback: conflict diagnostics, pin hover (the
//! alternate-function table from [`nucleus_db`]), and pin-name completion.
//!
//! The VS Code extension is a thin client that spawns `nucleus lsp` (which calls
//! [`run_stdio`]) and speaks LSP over stdio — all logic lives here, per the
//! project's scope-discipline rules.

pub mod analysis;
mod server;

pub use server::run_stdio;
