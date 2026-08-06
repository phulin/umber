use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::storage::{GeneratedFiles, StorageGeneration, WorkspaceStorage};
use crate::{StorageIdentity, VirtualFile, VirtualPath};

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
    pending_generated: Option<Arc<GeneratedFiles>>,
    valid: Arc<AtomicBool>,
}

impl WorkspaceStorage {
    /// Captures the current generation with no accepted generated invalidations.
    #[must_use]
    pub fn snapshot(&self) -> VfsSnapshot {
        VfsSnapshot::new(self.shared_generation(), None)
    }
}

impl VfsSnapshot {
    pub(crate) fn with_pending(
        generation: Arc<StorageGeneration>,
        pending_generated: Arc<GeneratedFiles>,
    ) -> Self {
        Self::new(generation, Some(pending_generated))
    }

    fn new(
        generation: Arc<StorageGeneration>,
        pending_generated: Option<Arc<GeneratedFiles>>,
    ) -> Self {
        Self {
            generation,
            pending_generated,
            valid: Arc::new(AtomicBool::new(true)),
        }
    }

    /// Returns this snapshot's deterministic storage generation identity.
    #[must_use]
    pub fn generation_identity(&self) -> StorageIdentity {
        self.generation.identity(self.pending_generated.as_deref())
    }

    /// Returns logical bindings and bytes owned by the retained generation.
    #[must_use]
    pub fn retention(&self) -> SnapshotRetention {
        let mut bindings = 0usize;
        let mut logical_bytes = 0usize;
        let mut input_bytes = 0usize;
        let mut generated_bytes = 0usize;
        for (files, generated) in [
            (self.generation.user.files(), false),
            (self.generation.resolved.files(), false),
            (self.generation.accepted_generated.files(), true),
        ] {
            for (_, file) in files {
                bindings += 1;
                logical_bytes += file.bytes().len();
                if generated {
                    generated_bytes += file.bytes().len();
                } else {
                    input_bytes += file.bytes().len();
                }
            }
        }
        if let Some(pending) = &self.pending_generated {
            for (_, file) in pending.files() {
                bindings += 1;
                logical_bytes += file.bytes().len();
                generated_bytes += file.bytes().len();
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
        self.list_inner(prefix.as_str().starts_with("/job/"), Some(prefix), limit)
    }

    /// Enumerates every visible path under one namespace root.
    pub fn list_root(
        &self,
        root: VirtualRoot,
        limit: usize,
    ) -> Result<Vec<VirtualPath>, SnapshotError> {
        self.check_valid()?;
        self.list_inner(matches!(root, VirtualRoot::Job), None, limit)
    }

    fn list_inner(
        &self,
        job: bool,
        prefix: Option<&VirtualPath>,
        limit: usize,
    ) -> Result<Vec<VirtualPath>, SnapshotError> {
        let mut iterators: Vec<_> = if job {
            let mut maps = Vec::with_capacity(3);
            if let Some(pending) = &self.pending_generated {
                maps.push(pending.files().peekable());
            }
            maps.push(self.generation.accepted_generated.files().peekable());
            maps.push(self.generation.user.files().peekable());
            maps
        } else {
            vec![self.generation.resolved.files().peekable()]
        };
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
                generation: self.generation.identity(self.pending_generated.as_deref()),
            })
        } else {
            Ok(())
        }
    }

    fn get_valid(&self, path: &VirtualPath) -> Option<&VirtualFile> {
        if path.as_str().starts_with("/job/") {
            self.pending_generated
                .as_ref()
                .and_then(|files| files.get(path))
                .or_else(|| self.generation.accepted_generated.get(path))
                .or_else(|| self.generation.user.get(path))
        } else {
            self.generation.resolved.get(path)
        }
    }
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
