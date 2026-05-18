//! End-to-end keyboard test suite.
//!
//! Entry point for `cargo test --test keyboard`. All test bodies live in
//! `e2e/mod.rs`; this file just pulls in the module.
//!
//! Unix-only: the harness drives `sway --headless` + `wtype` and the
//! fixture sends SIGTERM via `libc::kill`.

#![cfg(unix)]

mod e2e;
