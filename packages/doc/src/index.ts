/**
 * TypeScript view of the document format. The Rust core (`crates/core`) is the
 * implementation: it validates, edits and renders documents. This package only
 * describes the JSON, for the UI and for the importer that writes it.
 */
export * from "./types.js";
export * from "./build.js";
