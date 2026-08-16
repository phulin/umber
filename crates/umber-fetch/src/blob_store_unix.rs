//! Directory-handle-relative authority for verified native blobs.

use std::ffi::OsStr;
use std::fs::File;
use std::io;
use std::os::fd::{AsFd, OwnedFd};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use rustix::fs::{AtFlags, FlockOperation, Mode, OFlags, RenameFlags};

use super::{CacheError, authority_error};

const DIRECTORY_MODE: Mode = Mode::RUSR.union(Mode::WUSR).union(Mode::XUSR);
const FILE_MODE: Mode = Mode::RUSR.union(Mode::WUSR);

pub(super) struct Authority {
    _root: OwnedFd,
    namespace: OwnedFd,
    display: PathBuf,
}

pub(super) struct KeyGuard {
    _file: File,
}

impl Authority {
    pub(super) fn open(root: &Path, create_namespace: bool) -> Result<Self, CacheError> {
        let root_fd = open_root(root, create_namespace)?;
        let namespace = match open_directory_at(&root_fd, super::BLOB_DIRECTORY) {
            Ok(fd) => fd,
            Err(error) if error.kind() == io::ErrorKind::NotFound && create_namespace => {
                match mkdir_at(&root_fd, super::BLOB_DIRECTORY) {
                    Ok(()) => sync_fd(&root_fd)
                        .map_err(|error| CacheError::io("sync anchored root", root, error))?,
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                    Err(error) => {
                        return Err(CacheError::io(
                            "create anchored namespace",
                            root.join(super::BLOB_DIRECTORY),
                            error,
                        ));
                    }
                }
                open_directory_at(&root_fd, super::BLOB_DIRECTORY).map_err(|error| {
                    CacheError::io(
                        "open anchored namespace",
                        root.join(super::BLOB_DIRECTORY),
                        error,
                    )
                })?
            }
            Err(error) => {
                return Err(CacheError::io(
                    "open anchored namespace",
                    root.join(super::BLOB_DIRECTORY),
                    error,
                ));
            }
        };
        Ok(Self {
            _root: root_fd,
            namespace,
            display: root.join(super::BLOB_DIRECTORY),
        })
    }

    pub(super) fn lock(&self, name: &str) -> Result<KeyGuard, CacheError> {
        let lock_name = format!(".{name}.lock");
        let fd = open_file_at(
            &self.namespace,
            &lock_name,
            OFlags::RDWR | OFlags::CREATE | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        )
        .map_err(|error| CacheError::io("open anchored key lock", self.path(&lock_name), error))?;
        let file = File::from(fd);
        rustix::fs::flock(&file, FlockOperation::LockExclusive).map_err(|error| {
            CacheError::io(
                "lock anchored key",
                self.path(&lock_name),
                io::Error::from(error),
            )
        })?;
        Ok(KeyGuard { _file: file })
    }

    pub(super) fn open_entry(&self, name: &str) -> Result<Option<File>, CacheError> {
        match open_file_at(
            &self.namespace,
            name,
            OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NONBLOCK,
        ) {
            Ok(fd) => {
                let file = File::from(fd);
                let metadata = file.metadata().map_err(|error| {
                    CacheError::io("inspect anchored entry", self.path(name), error)
                })?;
                if !metadata.file_type().is_file() {
                    return Err(authority_error(
                        &self.path(name),
                        "entry is not a regular file",
                    ));
                }
                Ok(Some(file))
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(CacheError::io(
                "open anchored entry",
                self.path(name),
                error,
            )),
        }
    }

    pub(super) fn entry_names(&self) -> Result<Vec<String>, CacheError> {
        let mut directory = rustix::fs::Dir::read_from(&self.namespace).map_err(|error| {
            CacheError::io(
                "enumerate anchored namespace",
                &self.display,
                io::Error::from(error),
            )
        })?;
        let mut names = Vec::new();
        for entry in &mut directory {
            let entry = entry.map_err(|error| {
                CacheError::io(
                    "enumerate anchored namespace",
                    &self.display,
                    io::Error::from(error),
                )
            })?;
            let name = entry.file_name().to_str().map_err(|_| {
                authority_error(&self.display, "cache entry name is not valid UTF-8")
            })?;
            if name != "." && name != ".." {
                names.push(name.to_owned());
            }
        }
        names.sort();
        Ok(names)
    }

    pub(super) fn quarantine(&self, name: &str) -> Result<(), CacheError> {
        static NONCE: AtomicU64 = AtomicU64::new(0);
        let quarantine = format!(
            ".corrupt-{}-{}-{}",
            std::process::id(),
            NONCE.fetch_add(1, Ordering::Relaxed),
            name
        );
        rename_noreplace_at(&self.namespace, name, &self.namespace, &quarantine)
            .map_err(|error| CacheError::io("quarantine anchored entry", self.path(name), error))?;
        unlink_at(&self.namespace, &quarantine).map_err(|error| {
            CacheError::io("remove quarantined entry", self.path(&quarantine), error)
        })?;
        sync_fd(&self.namespace)
            .map_err(|error| CacheError::io("sync namespace", &self.display, error))
    }

    pub(super) fn publish(&self, name: &str, bytes: &[u8]) -> Result<bool, CacheError> {
        static NONCE: AtomicU64 = AtomicU64::new(0);
        let temporary = format!(
            ".tmp-{}-{}",
            std::process::id(),
            NONCE.fetch_add(1, Ordering::Relaxed)
        );
        let fd = open_file_at(
            &self.namespace,
            &temporary,
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        )
        .map_err(|error| {
            CacheError::io("create anchored temporary", self.path(&temporary), error)
        })?;
        let mut file = File::from(fd);
        let result = (|| {
            use std::io::Write as _;
            file.write_all(bytes)?;
            file.sync_all()?;
            match rename_noreplace_at(&self.namespace, &temporary, &self.namespace, name) {
                Ok(()) => {
                    sync_fd(&self.namespace)?;
                    Ok(true)
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(false),
                Err(error) => Err(error),
            }
        })();
        drop(file);
        if result
            .as_ref()
            .is_err_and(|error| error.kind() != io::ErrorKind::AlreadyExists)
            || matches!(result, Ok(false))
        {
            let _ = unlink_at(&self.namespace, &temporary);
        }
        result.map_err(|error| CacheError::io("publish anchored entry", self.path(name), error))
    }

    pub(super) fn path(&self, name: &str) -> PathBuf {
        self.display.join(name)
    }
}

fn open_root(path: &Path, create: bool) -> Result<OwnedFd, CacheError> {
    let (mut fd, components): (OwnedFd, Vec<&OsStr>) = if path.is_absolute() {
        (
            open_directory_path(Path::new("/"))
                .map_err(|error| CacheError::io("open filesystem root", Path::new("/"), error))?,
            path.components()
                .filter_map(|component| match component {
                    Component::Normal(value) => Some(value),
                    Component::RootDir => None,
                    _ => Some(OsStr::new("")),
                })
                .collect(),
        )
    } else {
        (
            open_directory_path(Path::new("."))
                .map_err(|error| CacheError::io("open current directory", Path::new("."), error))?,
            path.components()
                .map(|component| match component {
                    Component::Normal(value) => value,
                    _ => OsStr::new(""),
                })
                .collect(),
        )
    };
    if components.iter().any(|component| component.is_empty()) {
        return Err(authority_error(
            path,
            "cache root contains a non-normal component",
        ));
    }
    for component in components {
        let name = component.to_string_lossy();
        match open_directory_at(&fd, &name) {
            Ok(next) => fd = next,
            Err(error) if error.kind() == io::ErrorKind::NotFound && create => {
                match mkdir_at(&fd, &name) {
                    Ok(()) => sync_fd(&fd).map_err(|error| {
                        CacheError::io("sync anchored root parent", path, error)
                    })?,
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                    Err(error) => {
                        return Err(CacheError::io("create anchored root", path, error));
                    }
                }
                fd = open_directory_at(&fd, &name)
                    .map_err(|error| CacheError::io("open anchored root", path, error))?;
            }
            Err(error) => {
                return Err(CacheError::io("open anchored root", path, error));
            }
        }
    }
    Ok(fd)
}

fn validate_name(name: &str) -> io::Result<()> {
    if name.is_empty() || name == "." || name == ".." || name.contains('/') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "unsafe cache component",
        ));
    }
    if name.as_bytes().contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "NUL in cache component",
        ));
    }
    Ok(())
}

fn open_directory_path(path: &Path) -> io::Result<OwnedFd> {
    rustix::fs::open(
        path,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        Mode::empty(),
    )
    .map_err(io::Error::from)
}

fn open_directory_at(parent: impl AsFd, name: &str) -> io::Result<OwnedFd> {
    open_file_at(
        parent,
        name,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
    )
}

fn open_file_at(parent: impl AsFd, name: &str, flags: OFlags) -> io::Result<OwnedFd> {
    validate_name(name)?;
    rustix::fs::openat(parent, name, flags, FILE_MODE).map_err(io::Error::from)
}

fn mkdir_at(parent: impl AsFd, name: &str) -> io::Result<()> {
    validate_name(name)?;
    rustix::fs::mkdirat(parent, name, DIRECTORY_MODE).map_err(io::Error::from)
}

fn unlink_at(parent: impl AsFd, name: &str) -> io::Result<()> {
    validate_name(name)?;
    rustix::fs::unlinkat(parent, name, AtFlags::empty()).map_err(io::Error::from)
}

fn rename_noreplace_at(
    old_parent: impl AsFd,
    old: &str,
    new_parent: impl AsFd,
    new: &str,
) -> io::Result<()> {
    validate_name(old)?;
    validate_name(new)?;
    rustix::fs::renameat_with(old_parent, old, new_parent, new, RenameFlags::NOREPLACE)
        .map_err(io::Error::from)
}

fn sync_fd(fd: impl AsFd) -> io::Result<()> {
    rustix::fs::fsync(fd).map_err(io::Error::from)
}
