//! Test-only native_sim versus Rust PTY differential helpers.
//!
//! Batch 1 compares narrow, typed public MCP outcomes. It intentionally does
//! not replace existing native or fixture parity tests.

pub mod backend;
pub mod model;
pub mod registry;
pub mod scenarios;
