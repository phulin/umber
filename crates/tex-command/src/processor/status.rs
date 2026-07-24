//! Scanner-status installation and recovery.

/// Persistent scanner status and incomplete-input ownership.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub(crate) struct ScannerState {
    pub(crate) status: ScannerStatus,
    pub(crate) warning_identity: Option<u64>,
}

/// Typed shell for TeX's live `scanner_status`.
///
/// Builder identities refer into [`crate::state::TransientState`]. Scoped
/// installation and restoration are command semantics and are intentionally
/// deferred.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
#[allow(dead_code)] // variants are ownership shells for later scanner semantics
pub(crate) enum ScannerStatus {
    #[default]
    Normal,
    Skipping {
        condition: u64,
    },
    Defining {
        target: Option<u64>,
        builder: u64,
    },
    Matching {
        macro_name: u64,
        builder: u64,
    },
    Aligning {
        alignment: u64,
        builder: u64,
    },
    Absorbing {
        owner: Option<u64>,
        builder: u64,
    },
}
