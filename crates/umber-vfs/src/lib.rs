//! Host-neutral virtual filesystem primitives for Umber.

use std::fmt;
use std::path::Path;

mod file;
mod limits;
mod resource;
mod snapshot;
mod storage;
mod transaction;

pub use file::{
    BuildId, FileContentId, FileOrigin, PathBindingId, ProducerId, StageId, VirtualFile,
};
pub use limits::{VfsLimitError, VfsLimitKind, VfsLimits};
pub use resource::{
    FileKind, FileProvisioner, FileRequest, FileRequestBatch, FileRequestKey, ProvisionError,
    ProvisionOutcome, RequestKeyError, ResolvedFile, ResourceDomain, RetryError,
    UserRegistrationError,
};
pub use snapshot::{SnapshotError, SnapshotRetention, VfsSnapshot, VirtualRoot};
pub use storage::{
    DISTRIBUTION_LAYER_PRECEDENCE, FileLayer, ImmutableBindingError, InsertOutcome,
    JOB_LAYER_PRECEDENCE, LayerKind, LayeredFileStorage, StorageIdentity,
};
pub use transaction::{
    AcceptedBuild, BuildPlan, BuildTransaction, DeclaredReplacement, StageCommit, StageTransaction,
    TransactionError, VirtualFs,
};

/// A canonical absolute path in Umber's virtual namespace.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct VirtualPath(String);

impl VirtualPath {
    /// Canonicalizes a user or generated file path beneath `/job`.
    ///
    /// Relative paths are rooted beneath `/job`; absolute paths must already
    /// use that root.
    pub fn user(path: &str) -> Result<Self, VirtualPathError> {
        let absolute = path.starts_with('/');
        let components = normalize_components(path)?;
        let components = if absolute {
            require_root(components, "job")?
        } else {
            components
        };
        if components.is_empty() {
            return Err(VirtualPathError::MISSING_FILE_NAME);
        }
        Ok(Self(format!("/job/{}", components.join("/"))))
    }

    /// Canonicalizes an absolute distribution file path beneath `/texlive`.
    pub fn distribution(path: &str) -> Result<Self, VirtualPathError> {
        if !path.starts_with('/') {
            return Err(VirtualPathError::DISTRIBUTION_PATH_MUST_BE_ABSOLUTE);
        }
        let components = require_root(normalize_components(path)?, "texlive")?;
        if components.is_empty() {
            return Err(VirtualPathError::MISSING_FILE_NAME);
        }
        Ok(Self(format!("/texlive/{}", components.join("/"))))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    #[must_use]
    pub fn as_path(&self) -> &Path {
        Path::new(&self.0)
    }
}

impl fmt::Display for VirtualPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A deterministic virtual-path validation failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VirtualPathError {
    message: &'static str,
}

impl VirtualPathError {
    const TRAVERSAL: Self = Self {
        message: "parent traversal is not allowed",
    };
    const INVALID_SYNTAX: Self = Self {
        message: "NUL, backslash, colon, and URL-shaped paths are not allowed",
    };
    const EMPTY: Self = Self {
        message: "path is empty",
    };
    const MISSING_FILE_NAME: Self = Self {
        message: "path does not name a file",
    };
    const DISTRIBUTION_PATH_MUST_BE_ABSOLUTE: Self = Self {
        message: "distribution paths must be absolute under /texlive",
    };

    /// Creates an error for a higher-level path policy.
    ///
    /// Canonicalization errors are constructed by [`VirtualPath`]. This
    /// constructor lets domain-specific path policies retain this common error
    /// boundary without moving their policy into the VFS crate.
    #[must_use]
    pub const fn new(message: &'static str) -> Self {
        Self { message }
    }

    #[must_use]
    pub const fn message(&self) -> &'static str {
        self.message
    }
}

impl fmt::Display for VirtualPathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.message)
    }
}

impl std::error::Error for VirtualPathError {}

fn normalize_components(path: &str) -> Result<Vec<&str>, VirtualPathError> {
    if path.is_empty() {
        return Err(VirtualPathError::EMPTY);
    }
    if path.contains('\0') || path.contains('\\') || path.contains(':') {
        return Err(VirtualPathError::INVALID_SYNTAX);
    }

    let mut components = Vec::new();
    for component in path.split('/') {
        match component {
            "" | "." => {}
            ".." => return Err(VirtualPathError::TRAVERSAL),
            component => components.push(component),
        }
    }
    Ok(components)
}

fn require_root<'a>(
    components: Vec<&'a str>,
    root: &str,
) -> Result<Vec<&'a str>, VirtualPathError> {
    let Some((actual, suffix)) = components.split_first() else {
        return Err(VirtualPathError::MISSING_FILE_NAME);
    };
    if *actual != root {
        return Err(VirtualPathError::new(
            "absolute path is outside its required virtual root",
        ));
    }
    Ok(suffix.to_vec())
}

#[cfg(test)]
mod tests;
