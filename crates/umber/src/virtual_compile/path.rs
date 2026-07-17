use std::path::Path;

use sha2::{Digest, Sha256};

use super::{FileKind, FileRequestKey};
use umber_vfs::{VirtualPath, VirtualPathError};

pub(crate) enum RequestedFile {
    UserOnly(VirtualPath),
    Remote {
        user_path: Option<VirtualPath>,
        key: FileRequestKey,
    },
}

impl RequestedFile {
    pub(crate) fn parse(kind: FileKind, name: &str) -> Result<Self, VirtualPathError> {
        if name.starts_with('/') {
            let normalized = with_default_extension(name, extension(kind))?;
            let path = VirtualPath::user(&format!("/{normalized}"))?;
            return Ok(Self::UserOnly(path));
        }

        let normalized = match normalize_components(name) {
            Ok(normalized) => normalized,
            Err(_) if has_parent_component(name) => {
                let normalized = normalize_external_path(name, extension(kind))?;
                let digest = Sha256::digest(normalized.as_bytes())
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>();
                let key = FileRequestKey::new(kind, &format!(".host-path/{digest}"))
                    .expect("opaque host-path keys are valid VFS request keys");
                return Ok(Self::Remote {
                    user_path: None,
                    key,
                });
            }
            Err(error) => return Err(error),
        };
        if normalized.is_empty() {
            return Err(VirtualPathError::new("path does not name a file"));
        }
        let normalized = with_default_extension(&normalized.join("/"), extension(kind))?;
        let user_path = VirtualPath::user(&normalized)?;
        let key = FileRequestKey::new(kind, &normalized)
            .expect("TeX-normalized relative names are valid VFS request keys");
        Ok(Self::Remote {
            user_path: Some(user_path),
            key,
        })
    }
}

fn has_parent_component(path: &str) -> bool {
    path.split('/').any(|component| component == "..")
}

fn normalize_external_path(path: &str, extension: &str) -> Result<String, VirtualPathError> {
    if path.is_empty() {
        return Err(VirtualPathError::new("path is empty"));
    }
    if path.contains('\0') || path.contains('\\') || path.contains(':') {
        return Err(VirtualPathError::new(
            "NUL, backslash, colon, and URL-shaped paths are not allowed",
        ));
    }
    let mut components = path
        .split('/')
        .filter(|component| !component.is_empty() && *component != ".")
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let Some(file_name) = components.last_mut() else {
        return Err(VirtualPathError::new("path does not name a file"));
    };
    if file_name == ".." {
        return Err(VirtualPathError::new("path does not name a file"));
    }
    if !extension.is_empty() && Path::new(file_name.as_str()).extension().is_none() {
        file_name.push('.');
        file_name.push_str(extension);
    }
    Ok(components.join("/"))
}

fn extension(kind: FileKind) -> &'static str {
    match kind {
        FileKind::TexInput => "tex",
        FileKind::Tfm => "tfm",
        FileKind::Image => "",
        _ => unreachable!("the TeX resolver only accepts TeX file kinds"),
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
    if !extension.is_empty() && Path::new(last).extension().is_none() {
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
