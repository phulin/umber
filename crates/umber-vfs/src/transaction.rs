use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::Arc;

use crate::{
    BuildId, FileLayer, FileOrigin, ImmutableBindingError, LayerKind, LayeredFileStorage,
    ProducerId, SnapshotError, StageId, VfsLimitError, VfsLimitKind, VfsLimits, VfsSnapshot,
    VirtualFile, VirtualPath,
};

/// One cross-producer overwrite explicitly permitted by a build plan.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct DeclaredReplacement {
    pub path: VirtualPath,
    pub previous: ProducerId,
    pub replacing: ProducerId,
}

/// Immutable policy for one multi-stage build attempt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BuildPlan {
    build: BuildId,
    invalidated_accepted: BTreeSet<VirtualPath>,
    replacements: BTreeSet<DeclaredReplacement>,
}

impl BuildPlan {
    #[must_use]
    pub fn new(build: BuildId) -> Self {
        Self {
            build,
            invalidated_accepted: BTreeSet::new(),
            replacements: BTreeSet::new(),
        }
    }

    #[must_use]
    pub const fn build(&self) -> BuildId {
        self.build
    }

    pub fn invalidate_accepted(&mut self, path: VirtualPath) -> Result<(), TransactionError> {
        require_job_path(&path)?;
        self.invalidated_accepted.insert(path);
        Ok(())
    }

    pub fn declare_replacement(
        &mut self,
        path: VirtualPath,
        previous: ProducerId,
        replacing: ProducerId,
    ) -> Result<(), TransactionError> {
        require_job_path(&path)?;
        self.replacements.insert(DeclaredReplacement {
            path,
            previous,
            replacing,
        });
        Ok(())
    }

    fn permits(&self, path: &VirtualPath, previous: ProducerId, replacing: ProducerId) -> bool {
        self.replacements.contains(&DeclaredReplacement {
            path: path.clone(),
            previous,
            replacing,
        })
    }
}

/// A deterministic generated-file transaction failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TransactionError {
    InvalidGeneratedPath {
        path: VirtualPath,
    },
    PendingLayerNotEmpty,
    StageIdExhausted {
        build: BuildId,
    },
    UndeclaredCollision {
        path: VirtualPath,
        previous: ProducerId,
        replacing: ProducerId,
    },
    Limit(VfsLimitError),
    Storage(ImmutableBindingError),
    Snapshot(SnapshotError),
}

impl fmt::Display for TransactionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidGeneratedPath { path } => {
                write!(f, "generated path is outside /job: {path}")
            }
            Self::PendingLayerNotEmpty => {
                f.write_str("accepted VFS state contains a pending generated layer")
            }
            Self::StageIdExhausted { build } => {
                write!(
                    f,
                    "build {} exhausted its stage identity space",
                    build.get()
                )
            }
            Self::UndeclaredCollision {
                path,
                previous,
                replacing,
            } => write!(
                f,
                "producer {} cannot replace producer {} at {path} without a build-plan declaration",
                replacing.get(),
                previous.get()
            ),
            Self::Limit(error) => error.fmt(f),
            Self::Storage(error) => error.fmt(f),
            Self::Snapshot(error) => error.fmt(f),
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

impl From<SnapshotError> for TransactionError {
    fn from(value: SnapshotError) -> Self {
        Self::Snapshot(value)
    }
}

/// Accepted VFS state and its generated-file limits.
#[derive(Clone, Debug)]
pub struct VirtualFs {
    storage: LayeredFileStorage,
    limits: VfsLimits,
}

impl VirtualFs {
    pub fn new(limits: VfsLimits) -> Result<Self, TransactionError> {
        Self::from_storage(LayeredFileStorage::new(), limits)
    }

    pub fn from_storage(
        storage: LayeredFileStorage,
        limits: VfsLimits,
    ) -> Result<Self, TransactionError> {
        let limits = limits.validate()?;
        if !storage.layer(LayerKind::PendingGenerated).is_empty() {
            return Err(TransactionError::PendingLayerNotEmpty);
        }
        validate_storage_limits(&storage, &limits)?;
        Ok(Self { storage, limits })
    }

    #[must_use]
    pub const fn storage(&self) -> &LayeredFileStorage {
        &self.storage
    }

    #[must_use]
    pub const fn limits(&self) -> VfsLimits {
        self.limits
    }

    #[must_use]
    pub fn snapshot(&self) -> VfsSnapshot {
        self.storage.snapshot()
    }

    pub fn begin_build(&mut self, plan: BuildPlan) -> BuildTransaction<'_> {
        BuildTransaction::new(&mut self.storage, self.limits, plan)
    }
}

/// Summary of one successfully published stage write set.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StageCommit {
    pub producer: ProducerId,
    pub stage: StageId,
    pub paths: Vec<VirtualPath>,
    pub logical_bytes: usize,
}

/// Summary of one atomically accepted generated layer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AcceptedBuild {
    pub build: BuildId,
    pub generated_files: usize,
    pub logical_bytes: usize,
}

/// Private multi-stage overlay for one build attempt.
pub struct BuildTransaction<'a> {
    target: &'a mut LayeredFileStorage,
    limits: VfsLimits,
    working: LayeredFileStorage,
    plan: BuildPlan,
    next_stage: u64,
    issued_snapshots: RefCell<Vec<VfsSnapshot>>,
}

impl<'fs> BuildTransaction<'fs> {
    pub(crate) fn new(
        target: &'fs mut LayeredFileStorage,
        limits: VfsLimits,
        plan: BuildPlan,
    ) -> Self {
        let mut working = target.clone();
        working.replace_layer(FileLayer::new(LayerKind::PendingGenerated));
        Self {
            target,
            limits,
            working,
            plan,
            next_stage: 1,
            issued_snapshots: RefCell::new(Vec::new()),
        }
    }

    #[must_use]
    pub fn snapshot(&self) -> VfsSnapshot {
        self.issue_snapshot()
    }

    pub fn begin_stage<'stage>(
        &'stage mut self,
        producer: ProducerId,
    ) -> Result<StageTransaction<'stage, 'fs>, TransactionError> {
        let stage = StageId::new(self.next_stage);
        self.next_stage =
            self.next_stage
                .checked_add(1)
                .ok_or(TransactionError::StageIdExhausted {
                    build: self.plan.build,
                })?;
        let snapshot = self.issue_snapshot();
        Ok(StageTransaction {
            build: self,
            producer,
            stage,
            snapshot,
            writes: BTreeMap::new(),
            logical_bytes: 0,
        })
    }

    pub fn accept(self) -> Result<AcceptedBuild, TransactionError> {
        let pending = self.working.layer(LayerKind::PendingGenerated);
        let generated_files = pending.len();
        let logical_bytes = layer_bytes(pending)?;
        self.limits
            .check(VfsLimitKind::GeneratedFiles, generated_files)?;
        self.limits
            .check(VfsLimitKind::GeneratedBytes, logical_bytes)?;

        let accepted = pending.reclassified(LayerKind::AcceptedGenerated)?;
        let mut published = self.working.clone();
        published.replace_layer(accepted);
        published.replace_layer(FileLayer::new(LayerKind::PendingGenerated));
        *self.target = published;
        Ok(AcceptedBuild {
            build: self.plan.build,
            generated_files,
            logical_bytes,
        })
    }

    pub fn discard(self) {}

    fn issue_snapshot(&self) -> VfsSnapshot {
        let snapshot = self
            .working
            .snapshot_with_invalidated_accepted(self.plan.invalidated_accepted.iter().cloned())
            .expect("build plans validate generated invalidation paths");
        self.issued_snapshots.borrow_mut().push(snapshot.clone());
        snapshot
    }
}

impl Drop for BuildTransaction<'_> {
    fn drop(&mut self) {
        for snapshot in self.issued_snapshots.get_mut() {
            snapshot.invalidate();
        }
    }
}

/// One producer's private complete-file write set.
pub struct StageTransaction<'stage, 'fs> {
    build: &'stage mut BuildTransaction<'fs>,
    producer: ProducerId,
    stage: StageId,
    snapshot: VfsSnapshot,
    writes: BTreeMap<VirtualPath, Arc<[u8]>>,
    logical_bytes: usize,
}

impl StageTransaction<'_, '_> {
    #[must_use]
    pub fn snapshot(&self) -> VfsSnapshot {
        self.snapshot.clone()
    }

    pub fn write(&mut self, path: VirtualPath, bytes: Vec<u8>) -> Result<(), TransactionError> {
        require_job_path(&path)?;
        let limits = self.build.limits;
        limits.check(VfsLimitKind::OneFileBytes, bytes.len())?;
        let replaced = self.writes.get(&path).map_or(0, |bytes| bytes.len());
        let next_bytes = limits.checked_replacement_total(
            VfsLimitKind::StageBytes,
            self.logical_bytes,
            replaced,
            bytes.len(),
        )?;
        let next_files = self.writes.len() + usize::from(!self.writes.contains_key(&path));
        limits.check(VfsLimitKind::StageFiles, next_files)?;
        self.writes.insert(path, Arc::from(bytes));
        self.logical_bytes = next_bytes;
        Ok(())
    }

    pub fn finish(self) -> Result<StageCommit, TransactionError> {
        let mut pending = self
            .build
            .working
            .layer(LayerKind::PendingGenerated)
            .clone();
        let paths = self.writes.keys().cloned().collect::<Vec<_>>();
        for (path, bytes) in &self.writes {
            if let Some(existing) = pending.get(path) {
                let FileOrigin::Generated {
                    producer: previous, ..
                } = existing.origin()
                else {
                    unreachable!("pending layers contain only generated files")
                };
                if *previous != self.producer
                    && !self.build.plan.permits(path, *previous, self.producer)
                {
                    return Err(TransactionError::UndeclaredCollision {
                        path: path.clone(),
                        previous: *previous,
                        replacing: self.producer,
                    });
                }
            }
            pending.replace(VirtualFile::new(
                path.clone(),
                Arc::clone(bytes),
                FileOrigin::Generated {
                    producer: self.producer,
                    build: self.build.plan.build,
                    stage: self.stage,
                },
            ))?;
        }
        let generated_bytes = layer_bytes(&pending)?;
        let limits = self.build.limits;
        limits.check(VfsLimitKind::GeneratedFiles, pending.len())?;
        limits.check(VfsLimitKind::GeneratedBytes, generated_bytes)?;
        self.build.working.replace_layer(pending);
        self.snapshot.invalidate();
        Ok(StageCommit {
            producer: self.producer,
            stage: self.stage,
            paths,
            logical_bytes: self.logical_bytes,
        })
    }

    pub fn discard(self) {}
}

impl Drop for StageTransaction<'_, '_> {
    fn drop(&mut self) {
        self.snapshot.invalidate();
    }
}

fn require_job_path(path: &VirtualPath) -> Result<(), TransactionError> {
    if path.as_str().starts_with("/job/") {
        Ok(())
    } else {
        Err(TransactionError::InvalidGeneratedPath { path: path.clone() })
    }
}

fn layer_bytes(layer: &FileLayer) -> Result<usize, TransactionError> {
    layer.files().try_fold(0usize, |total, (_, file)| {
        total
            .checked_add(file.bytes().len())
            .ok_or(TransactionError::Limit(VfsLimitError::LimitExceeded {
                kind: VfsLimitKind::GeneratedBytes,
                limit: usize::MAX,
                attempted: usize::MAX,
            }))
    })
}

fn validate_storage_limits(
    storage: &LayeredFileStorage,
    limits: &VfsLimits,
) -> Result<(), TransactionError> {
    for kind in [
        LayerKind::User,
        LayerKind::ResolvedResource,
        LayerKind::AcceptedGenerated,
    ] {
        let layer = storage.layer(kind);
        for (_, file) in layer.files() {
            limits.check(VfsLimitKind::OneFileBytes, file.bytes().len())?;
        }
        let (files_kind, bytes_kind) = match kind {
            LayerKind::User => (VfsLimitKind::UserFiles, VfsLimitKind::UserBytes),
            LayerKind::ResolvedResource => {
                (VfsLimitKind::ResolvedFiles, VfsLimitKind::ResolvedBytes)
            }
            LayerKind::AcceptedGenerated => {
                (VfsLimitKind::GeneratedFiles, VfsLimitKind::GeneratedBytes)
            }
            LayerKind::PendingGenerated => unreachable!(),
        };
        limits.check(files_kind, layer.len())?;
        limits.check(bytes_kind, layer_bytes(layer)?)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
