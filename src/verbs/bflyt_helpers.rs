//! Shared helpers for BFLYT mutation verbs: read-modify-write boilerplate
//! and the dry-run / safe-write pattern.

use anyhow::{Context, Result};
use std::fs;
use std::path::Path;

use crate::bflyt::{read_bflyt, write_bflyt, BFLYT};

/// Read a BFLYT, hand it to `f` for mutation, then either print the
/// rewritten bytes (dry run) or persist them. Returns the rewritten
/// length on success.
pub fn rewrite_bflyt(
    input: &Path,
    out: Option<&Path>,
    f: impl FnOnce(&mut BFLYT) -> Result<()>,
) -> Result<usize> {
    let bytes = fs::read(input).with_context(|| format!("reading {}", input.display()))?;
    let mut bflyt = read_bflyt(&bytes).map_err(|e| anyhow::anyhow!("{}", e))?;
    f(&mut bflyt)?;
    let written = write_bflyt(&bflyt).map_err(|e| anyhow::anyhow!("{}", e))?;
    let target = out.unwrap_or(input);
    super::write_output(target, &written)?;
    Ok(written.len())
}
