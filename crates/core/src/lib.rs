//! Cardsmith core: the document model, the commands that edit it, and the
//! renderer that turns it into pixels.
//!
//! This crate has no platform dependencies. It is built natively for the
//! headless CLI and tests, and for `wasm32-unknown-unknown` to run inside the
//! app's webview. Keep it that way: no threads, files, clocks or windows here.

// `!(x > 0.0)` is deliberate where NaN must count as "not positive". Index
// counters in the pixel loops are clearer than zipped iterators there.
#![allow(clippy::neg_cmp_op_on_partial_ord, clippy::explicit_counter_loop, clippy::large_enum_variant)]

pub mod commands;
pub mod compare;
pub mod doc;
pub mod effects;
pub mod error;
pub mod field;
pub mod paint;
pub mod raster;
pub mod render;
pub mod samples;
pub mod serialize;
pub mod session;
pub mod text;
pub mod tree;

pub use error::{Error, Result};
