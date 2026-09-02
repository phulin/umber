use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use tex_content::{ContentDomain, ContentIdentity, SharedBytes};

use crate::{FileOrigin, FileRequestKey, VirtualFile, VirtualPath};

#[derive(Clone, Debug)]
pub(crate) struct JobPath(VirtualPath);

impl JobPath {
    pub(crate) fn new(path: VirtualPath) -> Result<Self, VirtualPath> {
        if path.as_str().starts_with("/job/") {
            Ok(Self(path))
        } else {
            Err(path)
        }
    }

    pub(crate) fn as_path(&self) -> &VirtualPath {
        &self.0
    }
}

#[derive(Clone, Debug)]
pub(crate) struct DistributionPath(VirtualPath);

impl DistributionPath {
    pub(crate) fn new(path: VirtualPath) -> Result<Self, VirtualPath> {
        if path.as_str().starts_with("/texlive/") {
            Ok(Self(path))
        } else {
            Err(path)
        }
    }
}

/// Stable identity of all file bindings, origins, and ownership classes.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct StorageIdentity(ContentIdentity);

impl StorageIdentity {
    #[must_use]
    pub const fn identity(self) -> ContentIdentity {
        self.0
    }
}

impl fmt::Display for StorageIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0.hex())
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct UserFiles(BTreeMap<VirtualPath, VirtualFile>);

#[derive(Clone, Debug, Default)]
pub(crate) struct ResolvedFiles(BTreeMap<VirtualPath, VirtualFile>);

#[derive(Clone, Debug, Default)]
pub(crate) struct GeneratedFiles(BTreeMap<VirtualPath, VirtualFile>);

macro_rules! file_map_accessors {
    ($map:ident) => {
        impl $map {
            pub(crate) fn len(&self) -> usize {
                self.0.len()
            }

            pub(crate) fn get(&self, path: &VirtualPath) -> Option<&VirtualFile> {
                self.0.get(path)
            }

            pub(crate) fn files(
                &self,
            ) -> std::collections::btree_map::Iter<'_, VirtualPath, VirtualFile> {
                self.0.iter()
            }
        }
    };
}

file_map_accessors!(UserFiles);
file_map_accessors!(GeneratedFiles);

impl ResolvedFiles {
    pub(crate) fn get(&self, path: &VirtualPath) -> Option<&VirtualFile> {
        self.0.get(path)
    }

    pub(crate) fn files(&self) -> std::collections::btree_map::Iter<'_, VirtualPath, VirtualFile> {
        self.0.iter()
    }
}

impl UserFiles {
    pub(crate) fn replace(&mut self, path: JobPath, bytes: SharedBytes) {
        let path = path.0;
        self.0.insert(
            path.clone(),
            VirtualFile::new(path, bytes, FileOrigin::User),
        );
    }
}

impl ResolvedFiles {
    pub(crate) fn insert(
        &mut self,
        path: DistributionPath,
        bytes: SharedBytes,
        request: FileRequestKey,
    ) {
        let path = path.0;
        self.0.insert(
            path.clone(),
            VirtualFile::new(path, bytes, FileOrigin::Resolved(request)),
        );
    }
}

impl GeneratedFiles {
    pub(crate) fn replace(&mut self, path: JobPath, bytes: SharedBytes) {
        let path = path.0;
        self.0.insert(
            path.clone(),
            VirtualFile::new(path, bytes, FileOrigin::Generated),
        );
    }
}

/// One durable copy-on-write VFS generation.
///
/// Each field has a distinct map type whose only constructors assign its
/// required root and origin. A durable generation therefore cannot contain a
/// pending file or an invalid root/origin combination.
#[derive(Clone, Debug, Default)]
pub(crate) struct StorageGeneration {
    pub(crate) user: Arc<UserFiles>,
    pub(crate) resolved: Arc<ResolvedFiles>,
    pub(crate) accepted_generated: Arc<GeneratedFiles>,
}

/// Private mutable handle for durable copy-on-write storage.
#[derive(Clone, Debug, Default)]
pub(crate) struct WorkspaceStorage {
    generation: Arc<StorageGeneration>,
}

impl WorkspaceStorage {
    pub(crate) fn user(&self) -> &UserFiles {
        &self.generation.user
    }

    pub(crate) fn resolved(&self) -> &ResolvedFiles {
        &self.generation.resolved
    }

    pub(crate) fn replace_user(&mut self, path: JobPath, bytes: SharedBytes) {
        let generation = Arc::make_mut(&mut self.generation);
        Arc::make_mut(&mut generation.user).replace(path, bytes);
    }

    pub(crate) fn insert_resolved(
        &mut self,
        path: DistributionPath,
        bytes: SharedBytes,
        request: FileRequestKey,
    ) {
        let generation = Arc::make_mut(&mut self.generation);
        Arc::make_mut(&mut generation.resolved).insert(path, bytes, request);
    }

    pub(crate) fn clear_resolved(&mut self) {
        Arc::make_mut(&mut self.generation).resolved = Arc::default();
    }

    pub(crate) fn publish_generated(&mut self, generated: Arc<GeneratedFiles>) {
        Arc::make_mut(&mut self.generation).accepted_generated = generated;
    }

    pub(crate) fn clear_generated(&mut self) {
        Arc::make_mut(&mut self.generation).accepted_generated = Arc::default();
    }

    pub(crate) fn shared_generation(&self) -> Arc<StorageGeneration> {
        Arc::clone(&self.generation)
    }
}

impl StorageGeneration {
    pub(crate) fn identity(&self, pending: Option<&GeneratedFiles>) -> StorageIdentity {
        let mut preimage = Vec::new();
        preimage.push(1); // Layered storage schema version retained for compatibility.
        encode_map(1, &self.user.0, &mut preimage);
        encode_map(2, &self.resolved.0, &mut preimage);
        encode_map(3, &self.accepted_generated.0, &mut preimage);
        if let Some(pending) = pending {
            encode_map(4, &pending.0, &mut preimage);
        } else {
            encode_map(4, &BTreeMap::new(), &mut preimage);
        }
        StorageIdentity(ContentIdentity::for_domain(
            ContentDomain::VirtualFileStorage,
            &preimage,
        ))
    }
}

fn encode_map(tag: u8, files: &BTreeMap<VirtualPath, VirtualFile>, preimage: &mut Vec<u8>) {
    preimage.push(tag);
    preimage.extend_from_slice(&(files.len() as u64).to_le_bytes());
    for (path, file) in files {
        let path_bytes = path.as_str().as_bytes();
        preimage.extend_from_slice(&(path_bytes.len() as u64).to_le_bytes());
        preimage.extend_from_slice(path_bytes);
        preimage.extend_from_slice(&file.content_id().identity().bytes());
        encode_origin(file.origin(), preimage);
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
        FileOrigin::Generated => bytes.push(3),
    }
}
