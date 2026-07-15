use std::path::Path;

use super::{FileKind, FileRequestKey};
use umber_vfs::{VirtualPath, VirtualPathError};

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
            return Err(VirtualPathError::new("path does not name a file"));
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
        RequestedFile::UserOnly(_) => Err(VirtualPathError::new(
            "remote request keys must be relative names",
        )),
    }
}

pub(crate) fn user_path_for_key(key: &FileRequestKey) -> Result<VirtualPath, VirtualPathError> {
    VirtualPath::user(key.name())
}

fn normalize_components(path: &str) -> Result<Vec<&str>, VirtualPathError> {
    if path.is_empty() {
        return Err(VirtualPathError::new("path is empty"));
    }
    if path.contains('\0') || path.contains('\\') || path.contains(':') {
        return Err(VirtualPathError::new(
            "NUL, backslash, colon, and URL-shaped paths are not allowed",
        ));
    }

    let mut components = Vec::new();
    for component in path.split('/') {
        match component {
            "" | "." => {}
            ".." => {
                return Err(VirtualPathError::new("parent traversal is not allowed"));
            }
            component => components.push(component),
        }
    }
    Ok(components)
}

fn with_default_extension(path: &str, extension: &str) -> Result<String, VirtualPathError> {
    let components = normalize_components(path)?;
    let Some(last) = components.last() else {
        return Err(VirtualPathError::new("path does not name a file"));
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
