use std::fmt;
use std::path::Path;

use super::{FileKind, FileRequestKey};

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct VirtualPath(String);

impl VirtualPath {
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

pub(crate) enum RequestedFile {
    UserOnly(VirtualPath),
    Remote {
        user_path: VirtualPath,
        key: FileRequestKey,
    },
}

impl RequestedFile {
    pub(crate) fn parse(kind: FileKind, name: &str) -> Result<Self, VirtualPathError> {
        if name.starts_with('/') {
            let normalized = with_default_extension(name, kind.extension())?;
            let path = VirtualPath::user(&format!("/{normalized}"))?;
            return Ok(Self::UserOnly(path));
        }

        let normalized = normalize_components(name)?;
        if normalized.is_empty() {
            return Err(VirtualPathError::MISSING_FILE_NAME);
        }
        let normalized = with_default_extension(&normalized.join("/"), kind.extension())?;
        let user_path = VirtualPath::user(&normalized)?;
        let key = FileRequestKey::from_normalized(kind, normalized);
        Ok(Self::Remote { user_path, key })
    }
}

pub(crate) fn normalize_request_name(
    kind: FileKind,
    name: &str,
) -> Result<String, VirtualPathError> {
    match RequestedFile::parse(kind, name)? {
        RequestedFile::Remote { key, .. } => Ok(key.name().to_owned()),
        RequestedFile::UserOnly(_) => Err(VirtualPathError {
            message: "remote request keys must be relative names",
        }),
    }
}

pub(crate) fn user_path_for_key(key: &FileRequestKey) -> VirtualPath {
    VirtualPath(format!("/job/{}", key.name()))
}

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
        return Err(VirtualPathError {
            message: "absolute path is outside its required virtual root",
        });
    }
    Ok(suffix.to_vec())
}

fn with_default_extension(path: &str, extension: &str) -> Result<String, VirtualPathError> {
    let components = normalize_components(path)?;
    let Some(last) = components.last() else {
        return Err(VirtualPathError::MISSING_FILE_NAME);
    };
    if Path::new(last).extension().is_none() {
        let extended = format!("{last}.{extension}");
        let prefix = &components[..components.len() - 1];
        return Ok(if prefix.is_empty() {
            extended
        } else {
            format!("{}/{}", prefix.join("/"), extended)
        });
    }
    Ok(components.join("/"))
}
