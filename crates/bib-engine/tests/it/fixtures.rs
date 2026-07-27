//! The translated upstream suite's one host-filesystem seam.
//!
//! `clippy.toml` routes engine I/O through `tex-state::World`, and the
//! workspace denies the policy rather than warning it. A host-side test that
//! consumes a committed fixture is outside that rule, but the allowance
//! belongs in one declared place rather than at each call site that happens to
//! need bytes off disk, so every upstream module reads its fixtures here.

#![allow(clippy::disallowed_methods)] // host-side reads of committed fixtures

use std::path::Path;

/// Reads a committed fixture, naming the missing path if it is absent.
pub fn read(path: impl AsRef<Path>) -> Vec<u8> {
    let path = path.as_ref();
    std::fs::read(path)
        .unwrap_or_else(|error| panic!("committed fixture {}: {error}", path.display()))
}

/// Reads a fixture the caller tolerates being absent.
///
/// The upstream suite drives resource requests generated from a control file,
/// and not every requested name has a committed counterpart; those requests
/// are meant to go unprovisioned rather than fail the test.
pub fn read_optional(path: impl AsRef<Path>) -> Option<Vec<u8>> {
    std::fs::read(path).ok()
}
