use std::collections::BTreeSet;
use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::storage::StorageGeneration;
use crate::{
    DISTRIBUTION_LAYER_PRECEDENCE, JOB_LAYER_PRECEDENCE, LayerKind, LayeredFileStorage,
    StorageIdentity, VirtualFile, VirtualPath,
};

/// Logical ownership retained by one immutable storage generation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SnapshotRetention {
    pub bindings: usize,
    pub logical_bytes: usize,
    pub input_bytes: usize,
    pub generated_bytes: usize,
}

/// One canonical public namespace root for root-level enumeration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VirtualRoot {
    Job,
    Distribution,
}

/// A deterministic snapshot access failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SnapshotError {
    Stale { generation: StorageIdentity },
    EnumerationLimitExceeded { limit: usize },
    InvalidationOutsideJob { path: VirtualPath },
}

impl fmt::Display for SnapshotError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Stale { generation } => {
                write!(f, "VFS snapshot generation {generation} is stale")
            }
            Self::EnumerationLimitExceeded { limit } => {
                write!(f, "VFS enumeration exceeds result limit {limit}")
            }
            Self::InvalidationOutsideJob { path } => {
                write!(f, "accepted generated invalidation is outside /job: {path}")
            }
        }
    }
}

impl std::error::Error for SnapshotError {}

/// A cheap immutable view of one exact VFS storage generation.
///
/// Clones share both retained storage and validity. Explicit invalidation makes
/// every clone stale, allowing stage and build owners to prevent reads after
/// their lifetime ends. Storage mutations alone do not invalidate snapshots.
#[derive(Clone, Debug)]
pub struct VfsSnapshot {
    generation: Arc<StorageGeneration>,
    invalidated_accepted: Arc<BTreeSet<VirtualPath>>,
    valid: Arc<AtomicBool>,
}

impl LayeredFileStorage {
    /// Captures the current generation with no accepted generated invalidations.
    #[must_use]
    pub fn snapshot(&self) -> VfsSnapshot {
        VfsSnapshot::new(self.shared_generation(), BTreeSet::new())
    }

    /// Captures the current generation while hiding selected accepted outputs.
    ///
    /// An invalidated accepted path may still resolve to pending or user data.
    pub fn snapshot_with_invalidated_accepted(
        &self,
        paths: impl IntoIterator<Item = VirtualPath>,
    ) -> Result<VfsSnapshot, SnapshotError> {
        let mut invalidated = BTreeSet::new();
        for path in paths {
            if !path.as_str().starts_with("/job/") {
                return Err(SnapshotError::InvalidationOutsideJob { path });
            }
            invalidated.insert(path);
        }
        Ok(VfsSnapshot::new(self.shared_generation(), invalidated))
    }
}

impl VfsSnapshot {
    fn new(generation: Arc<StorageGeneration>, invalidated: BTreeSet<VirtualPath>) -> Self {
        Self {
            generation,
            invalidated_accepted: Arc::new(invalidated),
            valid: Arc::new(AtomicBool::new(true)),
        }
    }

    /// Returns this snapshot's deterministic storage generation identity.
    #[must_use]
    pub fn generation_identity(&self) -> StorageIdentity {
        self.generation.identity()
    }

    /// Returns logical bindings and bytes owned by the retained generation.
    #[must_use]
    pub fn retention(&self) -> SnapshotRetention {
        let mut bindings = 0usize;
        let mut logical_bytes = 0usize;
        let mut input_bytes = 0usize;
        let mut generated_bytes = 0usize;
        for kind in all_layers() {
            for (_, file) in self.generation.layer(kind).files() {
                bindings += 1;
                logical_bytes += file.bytes().len();
                match kind {
                    LayerKind::User | LayerKind::ResolvedResource => {
                        input_bytes += file.bytes().len();
                    }
                    LayerKind::AcceptedGenerated | LayerKind::PendingGenerated => {
                        generated_bytes += file.bytes().len();
                    }
                }
            }
        }
        SnapshotRetention {
            bindings,
            logical_bytes,
            input_bytes,
            generated_bytes,
        }
    }

    /// Marks this snapshot and all its clones stale.
    pub fn invalidate(&self) {
        self.valid.store(false, Ordering::Release);
    }

    #[must_use]
    pub fn is_stale(&self) -> bool {
        !self.valid.load(Ordering::Acquire)
    }

    /// Reads exactly one canonical path using explicit layer precedence.
    pub fn get(&self, path: &VirtualPath) -> Result<Option<&VirtualFile>, SnapshotError> {
        self.check_valid()?;
        Ok(self.get_valid(path))
    }

    /// Tests one exact canonical path without extension or directory search.
    pub fn contains(&self, path: &VirtualPath) -> Result<bool, SnapshotError> {
        Ok(self.get(path)?.is_some())
    }

    /// Enumerates visible exact paths at or below `prefix` in lexical order.
    ///
    /// `prefix` matches itself or descendants separated by `/`; it never
    /// matches a sibling whose component merely starts with the same bytes.
    /// The method returns an error instead of allocating more than `limit`
    /// result paths.
    pub fn list(
        &self,
        prefix: &VirtualPath,
        limit: usize,
    ) -> Result<Vec<VirtualPath>, SnapshotError> {
        self.check_valid()?;
        let layers: &[LayerKind] = if prefix.as_str().starts_with("/job/") {
            &JOB_LAYER_PRECEDENCE
        } else {
            &DISTRIBUTION_LAYER_PRECEDENCE
        };
        self.list_inner(layers, Some(prefix), limit)
    }

    /// Enumerates every visible path under one namespace root.
    pub fn list_root(
        &self,
        root: VirtualRoot,
        limit: usize,
    ) -> Result<Vec<VirtualPath>, SnapshotError> {
        self.check_valid()?;
        let layers: &[LayerKind] = match root {
            VirtualRoot::Job => &JOB_LAYER_PRECEDENCE,
            VirtualRoot::Distribution => &DISTRIBUTION_LAYER_PRECEDENCE,
        };
        self.list_inner(layers, None, limit)
    }

    fn list_inner(
        &self,
        layers: &[LayerKind],
        prefix: Option<&VirtualPath>,
        limit: usize,
    ) -> Result<Vec<VirtualPath>, SnapshotError> {
        let mut iterators: Vec<_> = layers
            .iter()
            .map(|kind| self.generation.layer(*kind).files().peekable())
            .collect();
        let mut result = Vec::new();

        loop {
            let Some(path) = iterators
                .iter_mut()
                .filter_map(|iterator| iterator.peek().map(|(path, _)| (*path).clone()))
                .min()
            else {
                break;
            };
            for iterator in &mut iterators {
                if iterator
                    .peek()
                    .is_some_and(|(candidate, _)| *candidate == &path)
                {
                    iterator.next();
                }
            }
            if prefix.is_none_or(|prefix| matches_prefix(&path, prefix))
                && self.get_valid(&path).is_some()
            {
                if result.len() == limit {
                    return Err(SnapshotError::EnumerationLimitExceeded { limit });
                }
                result.push(path);
            }
        }
        Ok(result)
    }

    fn check_valid(&self) -> Result<(), SnapshotError> {
        if self.is_stale() {
            Err(SnapshotError::Stale {
                generation: self.generation.identity(),
            })
        } else {
            Ok(())
        }
    }

    fn get_valid(&self, path: &VirtualPath) -> Option<&VirtualFile> {
        let precedence: &[LayerKind] = if path.as_str().starts_with("/job/") {
            &JOB_LAYER_PRECEDENCE
        } else {
            &DISTRIBUTION_LAYER_PRECEDENCE
        };
        precedence.iter().find_map(|kind| {
            if *kind == LayerKind::AcceptedGenerated && self.invalidated_accepted.contains(path) {
                None
            } else {
                self.generation.layer(*kind).get(path)
            }
        })
    }
}

fn all_layers() -> [LayerKind; 4] {
    [
        LayerKind::User,
        LayerKind::ResolvedResource,
        LayerKind::AcceptedGenerated,
        LayerKind::PendingGenerated,
    ]
}

fn matches_prefix(path: &VirtualPath, prefix: &VirtualPath) -> bool {
    path == prefix
        || path
            .as_str()
            .strip_prefix(prefix.as_str())
            .is_some_and(|suffix| suffix.starts_with('/'))
}

#[cfg(test)]
mod tests;
