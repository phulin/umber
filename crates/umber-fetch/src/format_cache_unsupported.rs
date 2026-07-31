//! Fail-closed format-cache authority for platforms without anchored host I/O.

use std::fs::File;
use std::path::{Path, PathBuf};

use super::{FormatCacheError, authority_error};

pub(super) struct Authority;
pub(super) struct KeyGuard;

impl Authority {
    pub(super) fn open(root: &Path, _: bool) -> Result<Self, FormatCacheError> {
        Err(authority_error(
            root,
            "anchored format-cache I/O is unsupported on this platform",
        ))
    }

    pub(super) fn lock(&self, _: &str) -> Result<KeyGuard, FormatCacheError> {
        unreachable!("unsupported authority cannot be constructed")
    }

    pub(super) fn open_entry(&self, _: &str) -> Result<Option<File>, FormatCacheError> {
        unreachable!("unsupported authority cannot be constructed")
    }

    pub(super) fn quarantine(&self, _: &str) -> Result<(), FormatCacheError> {
        unreachable!("unsupported authority cannot be constructed")
    }

    pub(super) fn publish(&self, _: &str, _: &[u8]) -> Result<bool, FormatCacheError> {
        unreachable!("unsupported authority cannot be constructed")
    }

    pub(super) fn path(&self, _: &str) -> PathBuf {
        unreachable!("unsupported authority cannot be constructed")
    }
}
