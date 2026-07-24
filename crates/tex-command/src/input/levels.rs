//! Source and stored-token input levels.

use super::source::SourceCursor;

/// One future-relevant registered-source input level.
///
/// Token-list variants are introduced with raw command delivery; source
/// registration does not pre-tokenize input.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct InputLevel {
    pub(crate) identity: u64,
    pub(crate) source: SourceCursor,
}
