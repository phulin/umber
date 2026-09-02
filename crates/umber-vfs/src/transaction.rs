use std::cell::RefCell;
use std::fmt;
use std::sync::Arc;

use crate::storage::{GeneratedFiles, JobPath, WorkspaceStorage};
use crate::{VfsLimitError, VfsLimitKind, VfsLimits, VfsSnapshot, VirtualPath};

/// A deterministic generated-file transaction failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TransactionError {
    InvalidGeneratedPath { path: VirtualPath },
    Limit(VfsLimitError),
}

impl fmt::Display for TransactionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidGeneratedPath { path } => {
                write!(f, "generated path is outside /job: {path}")
            }
            Self::Limit(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for TransactionError {}

impl From<VfsLimitError> for TransactionError {
    fn from(value: VfsLimitError) -> Self {
        Self::Limit(value)
    }
}

/// Summary of one atomically accepted generated set.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AcceptedGenerated {
    pub generated_files: usize,
    pub logical_bytes: usize,
}

/// One private complete generated-file set owned by a project workspace.
///
/// Writes are visible only through candidate snapshots until `accept` replaces
/// the workspace's accepted generated set. Dropping or discarding the
/// transaction publishes nothing.
pub struct GeneratedTransaction<'a> {
    target: &'a mut WorkspaceStorage,
    limits: VfsLimits,
    base: WorkspaceStorage,
    pending: GeneratedFiles,
    logical_bytes: usize,
    issued_snapshots: RefCell<Vec<VfsSnapshot>>,
}

impl<'workspace> GeneratedTransaction<'workspace> {
    pub(crate) fn new(target: &'workspace mut WorkspaceStorage, limits: VfsLimits) -> Self {
        let base = target.clone();
        Self {
            target,
            limits,
            base,
            pending: GeneratedFiles::default(),
            logical_bytes: 0,
            issued_snapshots: RefCell::new(Vec::new()),
        }
    }

    /// Captures an immutable view of the complete candidate written so far.
    #[must_use]
    pub fn snapshot(&self) -> VfsSnapshot {
        let snapshot = VfsSnapshot::with_pending(
            self.base.shared_generation(),
            Arc::new(self.pending.clone()),
        );
        self.issued_snapshots.borrow_mut().push(snapshot.clone());
        snapshot
    }

    /// Adds or replaces one complete generated file in the candidate set.
    pub fn write(&mut self, path: VirtualPath, bytes: Vec<u8>) -> Result<(), TransactionError> {
        let path =
            JobPath::new(path).map_err(|path| TransactionError::InvalidGeneratedPath { path })?;
        self.limits.check(VfsLimitKind::OneFileBytes, bytes.len())?;

        let replaced = self
            .pending
            .get(path.as_path())
            .map_or(0, |file| file.bytes().len());
        let next_bytes = self.limits.checked_replacement_total(
            VfsLimitKind::GeneratedBytes,
            self.logical_bytes,
            replaced,
            bytes.len(),
        )?;
        self.limits.check(VfsLimitKind::StageBytes, next_bytes)?;
        let next_files =
            self.pending.len() + usize::from(self.pending.get(path.as_path()).is_none());
        self.limits
            .check(VfsLimitKind::GeneratedFiles, next_files)?;
        self.limits.check(VfsLimitKind::StageFiles, next_files)?;

        self.pending.replace(path, bytes.into());
        self.logical_bytes = next_bytes;
        Ok(())
    }

    /// Atomically replaces the accepted generated set with this candidate.
    pub fn accept(self) -> Result<AcceptedGenerated, TransactionError> {
        let summary = AcceptedGenerated {
            generated_files: self.pending.len(),
            logical_bytes: self.logical_bytes,
        };
        self.target
            .publish_generated(Arc::new(self.pending.clone()));
        Ok(summary)
    }

    pub fn discard(self) {}
}

impl Drop for GeneratedTransaction<'_> {
    fn drop(&mut self) {
        for snapshot in self.issued_snapshots.get_mut() {
            snapshot.invalidate();
        }
    }
}

#[cfg(test)]
mod tests;
