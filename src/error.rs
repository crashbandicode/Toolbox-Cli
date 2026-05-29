//! Unified error type for the library's high-level API.
//!
//! The low-level format parsers expose their own granular error types
//! ([`crate::bflyt::BflytError`], [`crate::bntx::BntxError`]) so callers
//! can match on parse/encode specifics. This [`Error`] is what the
//! higher-level surface (texture pipeline, SARC archive helpers, and the
//! manifest orchestration) returns; it wraps the format errors plus I/O,
//! image, and archive failures so any of them can flow through `?`.

use thiserror::Error;

/// Errors returned by the library's high-level API.
#[derive(Debug, Error)]
pub enum Error {
    /// A BFLYT parse, write, or mutation error.
    #[error("BFLYT error: {0}")]
    Bflyt(#[from] crate::bflyt::BflytError),

    /// A BNTX parse, write, or mutation error.
    #[error("BNTX error: {0}")]
    Bntx(#[from] crate::bntx::BntxError),

    /// An underlying I/O failure (reading/writing files).
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// A texture-pipeline failure (image decode, BC7 encode, Tegra swizzle).
    #[error("texture pipeline error: {0}")]
    Texpipe(String),

    /// A SARC archive pack/unpack failure.
    #[error("SARC error: {0}")]
    Sarc(String),

    /// A manifest parse or application failure.
    #[error("manifest error: {0}")]
    Manifest(String),

    /// A miscellaneous, caller-facing error message.
    #[error("{0}")]
    Other(String),
}

/// Convenience alias used throughout the high-level API.
pub type Result<T> = std::result::Result<T, Error>;
