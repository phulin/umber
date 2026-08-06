use std::cell::RefCell;
use std::fmt;
use std::sync::Arc;

use crate::{
    FileLayer, FileOrigin, ImmutableBindingError, LayerKind, LayeredFileStorage, VfsLimitError,
    VfsLimitKind, VfsLimits, VfsSnapshot, VirtualFile, VirtualPath,
};

/// A deterministic generated-file transaction failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TransactionError {
    InvalidGeneratedPath { path: VirtualPath },
    Limit(VfsLimitError),
    Storage(ImmutableBindingError),
}

impl fmt::Display for TransactionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidGeneratedPath { path } => {
                write!(f, "generated path is outside /job: {path}")
            }
            Self::Limit(error) => error.fmt(f),
            Self::Storage(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for TransactionError {}

impl From<VfsLimitError> for TransactionError {
    fn from(value: VfsLimitError) -> Self {
        Self::Limit(value)
    }
}

impl From<ImmutableBindingError> for TransactionError {
    fn from(value: ImmutableBindingError) -> Self {
        Self::Storage(value)
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
    target: &'a mut LayeredFileStorage,
    limits: VfsLimits,
    working: LayeredFileStorage,
    logical_bytes: usize,
    issued_snapshots: RefCell<Vec<VfsSnapshot>>,
}

impl<'workspace> GeneratedTransaction<'workspace> {
    pub(crate) fn new(target: &'workspace mut LayeredFileStorage, limits: VfsLimits) -> Self {
        let mut working = target.clone();
        working.replace_layer(FileLayer::new(LayerKind::PendingGenerated));
        Self {
            target,
            limits,
            working,
            logical_bytes: 0,
            issued_snapshots: RefCell::new(Vec::new()),
        }
    }

    /// Captures an immutable view of the complete candidate written so far.
    #[must_use]
    pub fn snapshot(&self) -> VfsSnapshot {
        let snapshot = self.working.snapshot();
        self.issued_snapshots.borrow_mut().push(snapshot.clone());
        snapshot
    }

    /// Adds or replaces one complete generated file in the candidate set.
    pub fn write(&mut self, path: VirtualPath, bytes: Vec<u8>) -> Result<(), TransactionError> {
        require_job_path(&path)?;
        self.limits.check(VfsLimitKind::OneFileBytes, bytes.len())?;

        let pending = self.working.layer(LayerKind::PendingGenerated);
        let replaced = pending.get(&path).map_or(0, |file| file.bytes().len());
        let next_bytes = self.limits.checked_replacement_total(
            VfsLimitKind::GeneratedBytes,
            self.logical_bytes,
            replaced,
            bytes.len(),
        )?;
        self.limits.check(VfsLimitKind::StageBytes, next_bytes)?;
        let next_files = pending.len() + usize::from(pending.get(&path).is_none());
        self.limits
            .check(VfsLimitKind::GeneratedFiles, next_files)?;
        self.limits.check(VfsLimitKind::StageFiles, next_files)?;

        let mut next = pending.clone();
        next.replace(VirtualFile::new(
            path,
            Arc::<[u8]>::from(bytes),
            FileOrigin::Generated,
        ))?;
        self.working.replace_layer(next);
        self.logical_bytes = next_bytes;
        Ok(())
    }

    /// Atomically replaces the accepted generated set with this candidate.
    pub fn accept(self) -> Result<AcceptedGenerated, TransactionError> {
        let pending = self.working.layer(LayerKind::PendingGenerated);
        let summary = AcceptedGenerated {
            generated_files: pending.len(),
            logical_bytes: self.logical_bytes,
        };
        let accepted = pending.reclassified(LayerKind::AcceptedGenerated)?;
        let mut published = self.working.clone();
        published.replace_layer(accepted);
        published.replace_layer(FileLayer::new(LayerKind::PendingGenerated));
        *self.target = published;
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

fn require_job_path(path: &VirtualPath) -> Result<(), TransactionError> {
    if path.as_str().starts_with("/job/") {
        Ok(())
    } else {
        Err(TransactionError::InvalidGeneratedPath { path: path.clone() })
    }
}

#[cfg(test)]
mod tests;
