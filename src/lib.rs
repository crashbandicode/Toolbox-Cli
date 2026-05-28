//! Library entry point.
//!
//! Most users invoke this project as the `toolbox-cli` binary, but the
//! BFLYT/BNTX format parsers are also reusable as a library. The bin
//! crate (`src/main.rs`) is a thin wrapper around `verbs::dispatch`.
//!
//! Public re-exports here are kept minimal; consumers should reach into
//! the `bflyt` and `bntx` modules directly.

pub mod bflyt;
pub mod bntx;
pub mod manifest;
pub mod texpipe;
pub mod verbs;
