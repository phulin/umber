use std::fmt;
use std::sync::Arc;

use tex_content::{ContentDomain, ContentIdentity};

use crate::{FileRequestKey, VirtualPath};

/// Stable identity of exact immutable VFS file bytes.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct FileContentId(ContentIdentity);

impl FileContentId {
    #[must_use]
    pub fn for_bytes(bytes: &[u8]) -> Self {
        Self(ContentIdentity::for_domain(
            ContentDomain::VirtualFile,
            bytes,
        ))
    }

    #[must_use]
    pub const fn identity(self) -> ContentIdentity {
        self.0
    }

    #[must_use]
    pub const fn from_identity_bytes(bytes: [u8; 32]) -> Self {
        Self(ContentIdentity::new(bytes))
    }

    #[must_use]
    pub const fn bytes(self) -> [u8; 32] {
        self.0.bytes()
    }
}

impl fmt::Display for FileContentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0.hex())
    }
}

/// Stable identity of one canonical path bound to immutable content.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PathBindingId(ContentIdentity);

impl PathBindingId {
    #[must_use]
    pub fn for_path_and_content(path: &VirtualPath, content_id: FileContentId) -> Self {
        let path_bytes = path.as_str().as_bytes();
        let mut preimage = Vec::with_capacity(8 + path_bytes.len() + 32);
        preimage.extend_from_slice(&(path_bytes.len() as u64).to_le_bytes());
        preimage.extend_from_slice(path_bytes);
        preimage.extend_from_slice(&content_id.identity().bytes());
        Self(ContentIdentity::for_domain(
            ContentDomain::VirtualPathBinding,
            &preimage,
        ))
    }

    #[must_use]
    pub const fn identity(self) -> ContentIdentity {
        self.0
    }
}

impl fmt::Display for PathBindingId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0.hex())
    }
}

macro_rules! numeric_id {
    ($name:ident, $description:literal) => {
        #[doc = $description]
        #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name(u64);

        impl $name {
            #[must_use]
            pub const fn new(value: u64) -> Self {
                Self(value)
            }

            #[must_use]
            pub const fn get(self) -> u64 {
                self.0
            }
        }
    };
}

numeric_id!(ProducerId, "Stable identity of a generated-file producer.");
numeric_id!(BuildId, "Stable identity of a pending or accepted build.");
numeric_id!(StageId, "Stable identity of one stage within a build.");

/// Provenance retained with immutable file bytes.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum FileOrigin {
    User,
    /// A host-provided resource accepted for this exact typed request.
    Resolved(FileRequestKey),
    Generated {
        producer: ProducerId,
        build: BuildId,
        stage: StageId,
    },
}

/// A complete immutable virtual file with shared byte ownership.
#[derive(Clone, Debug)]
pub struct VirtualFile {
    path: VirtualPath,
    bytes: Arc<[u8]>,
    content_id: FileContentId,
    binding_id: PathBindingId,
    origin: FileOrigin,
}

impl VirtualFile {
    #[must_use]
    pub fn new(path: VirtualPath, bytes: impl Into<Arc<[u8]>>, origin: FileOrigin) -> Self {
        let bytes = bytes.into();
        let content_id = FileContentId::for_bytes(&bytes);
        let binding_id = PathBindingId::for_path_and_content(&path, content_id);
        Self {
            path,
            bytes,
            content_id,
            binding_id,
            origin,
        }
    }

    #[must_use]
    pub const fn path(&self) -> &VirtualPath {
        &self.path
    }

    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    #[must_use]
    pub fn shared_bytes(&self) -> Arc<[u8]> {
        Arc::clone(&self.bytes)
    }

    #[must_use]
    pub const fn content_id(&self) -> FileContentId {
        self.content_id
    }

    #[must_use]
    pub const fn binding_id(&self) -> PathBindingId {
        self.binding_id
    }

    #[must_use]
    pub fn origin(&self) -> &FileOrigin {
        &self.origin
    }
}

impl PartialEq for VirtualFile {
    fn eq(&self, other: &Self) -> bool {
        self.binding_id == other.binding_id && self.origin == other.origin
    }
}

impl Eq for VirtualFile {}
