//! Git-backed validation for closed, repository-owned fixture directories.

#![allow(clippy::disallowed_methods)] // host-only fixture authority and payload reads

use std::collections::BTreeSet;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail, ensure};

const INVENTORY_NAME: &str = "case.inventory";
const SCHEMA: &str = "closed-case-v1";

/// A validated closed fixture case.
#[derive(Debug)]
pub struct ClosedCase {
    repository: PathBuf,
    case_relative: PathBuf,
    has_inventory: bool,
    root: PathBuf,
    payloads: BTreeSet<String>,
}

impl ClosedCase {
    /// Validates `case_relative` against the Git checkout containing the
    /// process's current directory.
    pub fn discover(case_relative: impl AsRef<Path>) -> Result<Self> {
        let repository = crate::repository_root_at(&std::env::current_dir()?)?;
        Self::discover_at(&repository, case_relative)
    }

    /// Validates a case against an explicitly selected Git checkout.
    ///
    /// This seam exists for hermetic adversarial tests. Production tests
    /// should use [`Self::discover`] so a test binary reused across worktrees
    /// never retains its builder checkout as fixture authority.
    pub fn discover_at(repository: &Path, case_relative: impl AsRef<Path>) -> Result<Self> {
        Self::discover_inner(repository, case_relative.as_ref(), true)
    }

    /// Validates a closed tracked directory whose payload inventory is owned
    /// directly by Git rather than by a `case.inventory` manifest.
    pub fn discover_tracked(case_relative: impl AsRef<Path>) -> Result<Self> {
        let repository = crate::repository_root_at(&std::env::current_dir()?)?;
        Self::discover_tracked_at(&repository, case_relative)
    }

    /// Validates an unmanifested closed tracked directory in `repository`.
    pub fn discover_tracked_at(repository: &Path, case_relative: impl AsRef<Path>) -> Result<Self> {
        Self::discover_inner(repository, case_relative.as_ref(), false)
    }

    fn discover_inner(
        repository: &Path,
        case_relative: &Path,
        has_inventory: bool,
    ) -> Result<Self> {
        let repository = repository
            .canonicalize()
            .with_context(|| format!("canonicalize fixture authority {}", repository.display()))?;
        let selected_root = git_root(&repository)?;
        ensure!(
            selected_root == repository,
            "fixture authority {} is not the selected Git checkout {}",
            repository.display(),
            selected_root.display()
        );

        let case_relative = checked_relative(case_relative)?;
        let root = checked_directory_ancestry(&repository, &case_relative)?;
        ensure!(
            git_root(&root)? == repository,
            "closed fixture resolves outside selected Git checkout: {}",
            root.display()
        );

        let tracked = tracked_regular_files(&repository, &case_relative)?;
        let (payloads, declared) = if has_inventory {
            declared_inventory(&root)?
        } else {
            ensure!(!tracked.is_empty(), "closed fixture directory is empty");
            (tracked.clone(), tracked.clone())
        };
        ensure!(
            tracked == declared,
            "closed fixture Git inventory mismatch: declared={declared:?}, tracked={tracked:?}"
        );
        let present = present_regular_files(&root)?;
        ensure!(
            present == declared,
            "closed fixture filesystem inventory mismatch: declared={declared:?}, present={present:?}"
        );

        Ok(Self {
            repository,
            case_relative,
            has_inventory,
            root,
            payloads,
        })
    }

    /// Reads a declared payload after validation.
    pub fn read(&self, name: &str) -> Result<Vec<u8>> {
        let path = self.payload_path(name)?;
        fs::read(&path).with_context(|| format!("read fixture payload {}", path.display()))
    }

    /// Resolves a declared payload to its validated local regular-file path.
    ///
    /// Role-bearing metadata must use this method instead of joining an
    /// untrusted role onto a case root. Closed-case payloads are deliberately
    /// single-component names, so absolute, dot, traversal, and nested paths
    /// are rejected before inventory membership and file type are checked.
    pub fn payload_path(&self, name: &str) -> Result<PathBuf> {
        let path = checked_relative(Path::new(name))
            .with_context(|| format!("invalid closed fixture payload role {name:?}"))?;
        ensure!(
            path.components().count() == 1,
            "closed fixture payload role must be a single filename: {name}"
        );
        ensure!(
            self.payloads.contains(name),
            "undeclared closed fixture payload: {name}"
        );
        let current = Self::discover_inner(
            &self.repository,
            &self.case_relative,
            self.has_inventory,
        )
        .context("revalidate closed fixture before payload access")?;
        ensure!(
            current.root == self.root && current.payloads == self.payloads,
            "closed fixture authority changed after discovery"
        );
        let path = current.root.join(path);
        let metadata = fs::symlink_metadata(&path)
            .with_context(|| format!("inspect fixture payload {}", path.display()))?;
        ensure!(
            metadata.is_file() && !metadata.file_type().is_symlink(),
            "fixture payload is not a regular file: {}",
            path.display()
        );
        Ok(path)
    }

    /// Reads a declared UTF-8 payload after validation.
    pub fn read_to_string(&self, name: &str) -> Result<String> {
        String::from_utf8(self.read(name)?)
            .with_context(|| format!("fixture payload is not UTF-8: {name}"))
    }

    /// Returns a validated payload path for consumers that require a host path.
    pub fn path(&self, name: &str) -> Result<PathBuf> {
        ensure!(
            self.payloads.contains(name),
            "undeclared closed fixture payload: {name}"
        );
        Ok(self.root.join(name))
    }
}

fn declared_inventory(root: &Path) -> Result<(BTreeSet<String>, BTreeSet<String>)> {
    let inventory_path = root.join(INVENTORY_NAME);
    let inventory = fs::read_to_string(&inventory_path)
        .with_context(|| format!("read {}", inventory_path.display()))?;
    let mut lines = inventory.lines();
    ensure!(
        lines.next() == Some(SCHEMA),
        "{} must begin with {SCHEMA}",
        inventory_path.display()
    );
    let mut payloads = BTreeSet::new();
    for name in lines.filter(|line| !line.is_empty()) {
        let path = checked_relative(Path::new(name))
            .with_context(|| format!("invalid inventory entry {name:?}"))?;
        ensure!(
            path.components().count() == 1,
            "inventory entry is outside the case root: {name}"
        );
        ensure!(
            name != INVENTORY_NAME,
            "{INVENTORY_NAME} is metadata, not a payload"
        );
        ensure!(
            payloads.insert(name.to_owned()),
            "duplicate inventory entry: {name}"
        );
    }
    ensure!(!payloads.is_empty(), "closed fixture inventory is empty");
    let declared = payloads
        .iter()
        .cloned()
        .chain(std::iter::once(INVENTORY_NAME.to_owned()))
        .collect();
    Ok((payloads, declared))
}

fn checked_relative(path: &Path) -> Result<PathBuf> {
    ensure!(!path.as_os_str().is_empty(), "fixture path is empty");
    let mut checked = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(name) => {
                ensure!(
                    name != "target",
                    "target-backed fixture authority is forbidden: {}",
                    path.display()
                );
                checked.push(name);
            }
            _ => bail!(
                "fixture path must be a normalized repository-relative path: {}",
                path.display()
            ),
        }
    }
    Ok(checked)
}

fn checked_directory_ancestry(repository: &Path, relative: &Path) -> Result<PathBuf> {
    let mut current = repository.to_owned();
    for component in relative.components() {
        let Component::Normal(name) = component else {
            unreachable!("checked_relative returned a non-normal component");
        };
        current.push(name);
        let metadata = fs::symlink_metadata(&current)
            .with_context(|| format!("inspect closed fixture ancestry {}", current.display()))?;
        ensure!(
            !metadata.file_type().is_symlink(),
            "closed fixture ancestry contains a symlink: {}",
            current.display()
        );
        ensure!(
            metadata.is_dir(),
            "closed fixture ancestry component is not a directory: {}",
            current.display()
        );
    }

    let resolved = current
        .canonicalize()
        .with_context(|| format!("canonicalize closed fixture {}", current.display()))?;
    ensure!(
        resolved.starts_with(repository),
        "closed fixture resolves outside selected Git checkout: {}",
        current.display()
    );
    ensure!(
        !resolved
            .strip_prefix(repository)
            .expect("resolved fixture is beneath repository")
            .components()
            .any(|component| component.as_os_str() == "target"),
        "target-backed fixture authority is forbidden: {}",
        current.display()
    );
    Ok(resolved)
}

fn git_root(path: &Path) -> Result<PathBuf> {
    let output = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(["rev-parse", "--path-format=absolute", "--show-toplevel"])
        .output()
        .with_context(|| format!("resolve Git checkout at {}", path.display()))?;
    ensure!(
        output.status.success(),
        "Git could not resolve fixture authority at {}: {}",
        path.display(),
        String::from_utf8_lossy(&output.stderr).trim()
    );
    PathBuf::from(String::from_utf8_lossy(&output.stdout).trim())
        .canonicalize()
        .context("canonicalize Git checkout")
}

fn tracked_regular_files(repository: &Path, case_relative: &Path) -> Result<BTreeSet<String>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repository)
        .args(["ls-files", "--stage", "-z", "--"])
        .arg(case_relative)
        .output()
        .context("read fixture inventory from Git")?;
    ensure!(output.status.success(), "git ls-files failed");
    let prefix = format!("{}/", case_relative.to_string_lossy());
    output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
        .map(|entry| {
            let entry = std::str::from_utf8(entry).context("non-UTF-8 Git fixture entry")?;
            let (metadata, path) = entry
                .split_once('\t')
                .context("malformed git ls-files entry")?;
            let mode = metadata
                .split_whitespace()
                .next()
                .context("missing Git file mode")?;
            ensure!(
                matches!(mode, "100644" | "100755"),
                "tracked fixture entry is not a regular file: {path} (mode {mode})"
            );
            let name = path
                .strip_prefix(&prefix)
                .with_context(|| format!("tracked fixture entry is outside case: {path}"))?;
            ensure!(
                !name.contains('/'),
                "closed fixture contains nested tracked entry: {path}"
            );
            Ok(name.to_owned())
        })
        .collect()
}

fn present_regular_files(root: &Path) -> Result<BTreeSet<String>> {
    fs::read_dir(root)
        .with_context(|| format!("read closed fixture {}", root.display()))?
        .map(|entry| {
            let entry = entry?;
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path)?;
            ensure!(
                metadata.is_file() && !metadata.file_type().is_symlink(),
                "fixture entry is not a regular file: {}",
                path.display()
            );
            entry
                .file_name()
                .into_string()
                .map_err(|_| anyhow::anyhow!("non-UTF-8 fixture filename: {}", path.display()))
        })
        .collect()
}
