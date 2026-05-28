//! BNTX writer. NOT YET IMPLEMENTED for editing — for the milestone
//! checkpoint we ship a `read_bntx` that round-trips, then add writing as
//! a follow-up because BNTX writing requires careful handling of the
//! relocation table and string pool.
//!
//! The texture pipeline (`texpipe::add_texture_to_bntx`) currently
//! returns an error directing the user to use the C# CLI for BNTX import
//! until the Rust writer lands. This is a known TODO documented in the
//! README.

use super::*;
use super::error::Error;

pub fn write_bntx(_b: &BNTX) -> Result<Vec<u8>, Error> {
    Err(Error::Format(
        "BNTX writing not yet implemented in this Rust port. \
         Use bntx-inspect for read-only operations; use the upstream Switch \
         Toolbox or the C# Toolbox-Cli for import operations until this lands."
            .into(),
    ))
}
