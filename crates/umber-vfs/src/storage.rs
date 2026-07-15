use std::collections::BTreeMap;
use std::fmt;

use tex_content::{ContentDomain, ContentIdentity};

use crate::{FileContentId, FileOrigin, VirtualFile, VirtualPath};

/// The four ownership layers in a virtual workspace.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(u8)]
pub enum LayerKind {
    User = 1,
    ResolvedResource = 2,
    AcceptedGenerated = 3,
    PendingGenerated = 4,
}

/// Exact `/job` lookup precedence; pending stage ordering is added by transactions.
pub const JOB_LAYER_PRECEDENCE: [LayerKind; 3] = [
    LayerKind::PendingGenerated,
    LayerKind::AcceptedGenerated,
    LayerKind::User,
];

/// Exact `/texlive` lookup precedence.
pub const DISTRIBUTION_LAYER_PRECEDENCE: [LayerKind; 1] = [LayerKind::ResolvedResource];

/// The result of adding an immutable binding to a layer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InsertOutcome {
    Inserted,
    AlreadyPresent,
}

/// A deterministic immutable-path registration failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ImmutableBindingError {
    WrongOrigin {
        layer: LayerKind,
        origin: FileOrigin,
    },
    WrongRoot {
        layer: LayerKind,
        path: VirtualPath,
    },
    Conflict {
        layer: LayerKind,
        path: VirtualPath,
        existing: FileContentId,
        incoming: FileContentId,
    },
    OriginConflict {
        layer: LayerKind,
        path: VirtualPath,
        existing: FileOrigin,
        incoming: FileOrigin,
    },
}

impl fmt::Display for ImmutableBindingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WrongOrigin { layer, origin } => {
                write!(f, "origin {origin:?} cannot be stored in {layer:?}")
            }
            Self::WrongRoot { layer, path } => {
                write!(f, "path {path} is outside the root owned by {layer:?}")
            }
            Self::Conflict {
                layer,
                path,
                existing,
                incoming,
            } => write!(
                f,
                "immutable binding conflict in {layer:?} at {path}: {existing} != {incoming}"
            ),
            Self::OriginConflict {
                layer,
                path,
                existing,
                incoming,
            } => write!(
                f,
                "immutable origin conflict in {layer:?} at {path}: {existing:?} != {incoming:?}"
            ),
        }
    }
}

impl std::error::Error for ImmutableBindingError {}

/// One deterministically ordered immutable ownership layer.
#[derive(Clone, Debug)]
pub struct FileLayer {
    kind: LayerKind,
    files: BTreeMap<VirtualPath, VirtualFile>,
}

impl FileLayer {
    #[must_use]
    pub fn new(kind: LayerKind) -> Self {
        Self {
            kind,
            files: BTreeMap::new(),
        }
    }

    #[must_use]
    pub const fn kind(&self) -> LayerKind {
        self.kind
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.files.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    pub fn insert(&mut self, file: VirtualFile) -> Result<InsertOutcome, ImmutableBindingError> {
        validate_ownership(self.kind, &file)?;
        if let Some(existing) = self.files.get(file.path()) {
            if existing.content_id() != file.content_id() {
                return Err(ImmutableBindingError::Conflict {
                    layer: self.kind,
                    path: file.path().clone(),
                    existing: existing.content_id(),
                    incoming: file.content_id(),
                });
            }
            if existing.origin() != file.origin() {
                return Err(ImmutableBindingError::OriginConflict {
                    layer: self.kind,
                    path: file.path().clone(),
                    existing: existing.origin().clone(),
                    incoming: file.origin().clone(),
                });
            }
            return Ok(InsertOutcome::AlreadyPresent);
        }
        self.files.insert(file.path().clone(), file);
        Ok(InsertOutcome::Inserted)
    }

    pub(crate) fn files(&self) -> impl Iterator<Item = (&VirtualPath, &VirtualFile)> {
        self.files.iter()
    }
}

fn validate_ownership(kind: LayerKind, file: &VirtualFile) -> Result<(), ImmutableBindingError> {
    let origin_matches = matches!(
        (kind, file.origin()),
        (LayerKind::User, FileOrigin::User)
            | (LayerKind::ResolvedResource, FileOrigin::Resolved(_))
            | (
                LayerKind::AcceptedGenerated | LayerKind::PendingGenerated,
                FileOrigin::Generated { .. }
            )
    );
    if !origin_matches {
        return Err(ImmutableBindingError::WrongOrigin {
            layer: kind,
            origin: file.origin().clone(),
        });
    }

    let path_matches = match kind {
        LayerKind::User | LayerKind::AcceptedGenerated | LayerKind::PendingGenerated => {
            file.path().as_str().starts_with("/job/")
        }
        LayerKind::ResolvedResource => file.path().as_str().starts_with("/texlive/"),
    };
    if !path_matches {
        return Err(ImmutableBindingError::WrongRoot {
            layer: kind,
            path: file.path().clone(),
        });
    }
    Ok(())
}

/// Stable identity of all file bindings, origins, and layer ownership.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct StorageIdentity(ContentIdentity);

impl StorageIdentity {
    #[must_use]
    pub const fn identity(self) -> ContentIdentity {
        self.0
    }
}

impl fmt::Display for StorageIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0.hex())
    }
}

/// Separate deterministic storage for the four VFS ownership layers.
#[derive(Clone, Debug)]
pub struct LayeredFileStorage {
    user: FileLayer,
    resolved_resource: FileLayer,
    accepted_generated: FileLayer,
    pending_generated: FileLayer,
}

impl LayeredFileStorage {
    #[must_use]
    pub fn new() -> Self {
        Self {
            user: FileLayer::new(LayerKind::User),
            resolved_resource: FileLayer::new(LayerKind::ResolvedResource),
            accepted_generated: FileLayer::new(LayerKind::AcceptedGenerated),
            pending_generated: FileLayer::new(LayerKind::PendingGenerated),
        }
    }

    #[must_use]
    pub const fn layer(&self, kind: LayerKind) -> &FileLayer {
        match kind {
            LayerKind::User => &self.user,
            LayerKind::ResolvedResource => &self.resolved_resource,
            LayerKind::AcceptedGenerated => &self.accepted_generated,
            LayerKind::PendingGenerated => &self.pending_generated,
        }
    }

    pub fn insert(
        &mut self,
        kind: LayerKind,
        file: VirtualFile,
    ) -> Result<InsertOutcome, ImmutableBindingError> {
        match kind {
            LayerKind::User => &mut self.user,
            LayerKind::ResolvedResource => &mut self.resolved_resource,
            LayerKind::AcceptedGenerated => &mut self.accepted_generated,
            LayerKind::PendingGenerated => &mut self.pending_generated,
        }
        .insert(file)
    }

    /// Computes storage identity in explicit layer and canonical-path order.
    #[must_use]
    pub fn identity(&self) -> StorageIdentity {
        let mut preimage = Vec::new();
        preimage.push(1); // Layered storage schema version.
        for kind in [
            LayerKind::User,
            LayerKind::ResolvedResource,
            LayerKind::AcceptedGenerated,
            LayerKind::PendingGenerated,
        ] {
            let layer = self.layer(kind);
            preimage.push(kind as u8);
            preimage.extend_from_slice(&(layer.len() as u64).to_le_bytes());
            for (path, file) in layer.files() {
                let path_bytes = path.as_str().as_bytes();
                preimage.extend_from_slice(&(path_bytes.len() as u64).to_le_bytes());
                preimage.extend_from_slice(path_bytes);
                preimage.extend_from_slice(&file.content_id().identity().bytes());
                encode_origin(file.origin(), &mut preimage);
            }
        }
        StorageIdentity(ContentIdentity::for_domain(
            ContentDomain::VirtualFileStorage,
            &preimage,
        ))
    }
}

impl Default for LayeredFileStorage {
    fn default() -> Self {
        Self::new()
    }
}

fn encode_origin(origin: &FileOrigin, bytes: &mut Vec<u8>) {
    match origin {
        FileOrigin::User => bytes.push(1),
        FileOrigin::Resolved(request) => {
            bytes.push(2);
            bytes.push(request.domain() as u8);
            bytes.push(request.kind() as u8);
            let name = request.name().as_bytes();
            bytes.extend_from_slice(&(name.len() as u64).to_le_bytes());
            bytes.extend_from_slice(name);
        }
        FileOrigin::Generated {
            producer,
            build,
            stage,
        } => {
            bytes.push(3);
            bytes.extend_from_slice(&producer.get().to_le_bytes());
            bytes.extend_from_slice(&build.get().to_le_bytes());
            bytes.extend_from_slice(&stage.get().to_le_bytes());
        }
    }
}
