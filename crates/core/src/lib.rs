//! Cardsmith core: the document model, the commands that edit it, and the
//! renderer that turns it into pixels.
//!
//! This crate has no platform dependencies. It is built natively for the
//! headless CLI and tests, and for `wasm32-unknown-unknown` to run inside the
//! app's webview. Keep it that way: no threads, files, clocks or windows here.

pub mod commands;
pub mod doc;
pub mod error;
pub mod serialize;
pub mod tree;

pub use error::{Error, Result};
