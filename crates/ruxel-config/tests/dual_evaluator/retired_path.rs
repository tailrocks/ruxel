//! A harness source that carries a retired module path.
//!
//! The discovery follows the `#[path]` attributes a source really
//! declares, so the commented-out one below names no file: the wiring
//! check must not read `retired.rs`, which was never compiled and does
//! not exist.

// #[path = "retired.rs"]
// mod retired;
