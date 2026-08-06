//! Typed metadata and staging for closed repository fixtures.
//!
//! This module validates and stages bytes. It deliberately has no publication
//! operation: authority mutation belongs to `fixturegen`'s atomic transaction.

#![allow(clippy::disallowed_methods)] // host-only fixture staging

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};
use sha2::{Digest, Sha256};

use crate::git_fixture::ClosedCase;

const INVENTORY_NAME: &str = "case.inventory";
const INVENTORY_SCHEMA: &str = "closed-case-v1";

/// A normalized single-file name within a closed case.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct PayloadName(String);

impl PayloadName {
    pub fn new(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        validate_relative(Path::new(&value), false)?;
        ensure!(
            Path::new(&value).components().count() == 1,
            "closed-case payload must be a single filename: {value}"
        );
        ensure!(
            value != INVENTORY_NAME,
            "{INVENTORY_NAME} is inventory metadata, not a payload"
        );
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PayloadName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// A normalized repository-relative path used as Git publication authority.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct RepositoryPath(PathBuf);

impl RepositoryPath {
    pub fn new(value: impl Into<PathBuf>) -> Result<Self> {
        let value = value.into();
        validate_relative(&value, true)?;
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_path(&self) -> &Path {
        &self.0
    }
}

/// Stable case identity, independent of a process or builder checkout.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CaseIdentity {
    pub family: RepositoryPath,
    pub id: String,
}

impl CaseIdentity {
    pub fn new(family: impl Into<PathBuf>, id: impl Into<String>) -> Result<Self> {
        let family = RepositoryPath::new(family)?;
        let id = id.into();
        validate_relative(Path::new(&id), false)?;
        ensure!(
            Path::new(&id).components().count() == 1,
            "case id must be one normalized component: {id}"
        );
        Ok(Self { family, id })
    }

    #[must_use]
    pub fn repository_path(&self) -> PathBuf {
        self.family.as_path().join(&self.id)
    }
}

/// The semantic role of a tracked payload.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileRole {
    Input,
    ExpectedOutput,
    Metadata,
}

/// One ordered payload declaration. A digest, when supplied by an existing
/// format, is verified without forcing digest metadata on locally editable
/// fixture families that did not previously carry it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TrackedFile {
    pub name: PayloadName,
    pub role: FileRole,
    pub sha256: Option<String>,
}

/// A strict expected failure with durable issue and human explanation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Xfail {
    pub issue: String,
    pub reason: String,
}

/// Whether the committed expected artifacts represent a passing case or a
/// deliberately pinned failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CaseStatus {
    Pass,
    Xfail(Xfail),
}

/// Execution/capture profile selected by a fixture consumer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CaseProfile(pub String);

/// The primary source followed by every other input it is allowed to load.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceClosure {
    pub primary: PayloadName,
    pub inputs: Vec<PayloadName>,
}

/// Metadata needed by fixturegen to publish this case. Paths remain
/// declarative here; this crate never mutates them.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PublicationMetadata {
    pub destination: RepositoryPath,
    pub authorities: Vec<RepositoryPath>,
}

/// One complete typed contract for a closed case.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Contract {
    pub identity: CaseIdentity,
    pub files: Vec<TrackedFile>,
    pub status: CaseStatus,
    pub profile: CaseProfile,
    pub source_closure: SourceClosure,
    pub publication: PublicationMetadata,
}

impl Contract {
    /// Validates identity, exact ordered membership, roles, hashes, source
    /// closure, and publication metadata against an already Git-validated case.
    pub fn validate<'a>(&'a self, case: &'a ClosedCase) -> Result<ValidatedCase<'a>> {
        ensure!(
            self.identity.repository_path() == case.repository_relative(),
            "closed-case identity does not match Git path: identity={}, git={}",
            self.identity.repository_path().display(),
            case.repository_relative().display()
        );
        ensure!(
            !self.profile.0.trim().is_empty(),
            "closed-case profile must not be empty"
        );
        if let CaseStatus::Xfail(xfail) = &self.status {
            ensure!(
                !xfail.issue.trim().is_empty(),
                "xfail issue must not be empty"
            );
            ensure!(
                !xfail.reason.trim().is_empty(),
                "xfail reason must not be empty"
            );
        }
        ensure!(!self.files.is_empty(), "closed-case file contract is empty");
        let mut declared = BTreeSet::new();
        for file in &self.files {
            ensure!(
                declared.insert(file.name.as_str()),
                "duplicate closed-case file role: {}",
                file.name
            );
            if let Some(digest) = &file.sha256 {
                validate_digest(digest)?;
                let bytes = case.read(file.name.as_str())?;
                ensure!(
                    digest == &hex_digest(&bytes),
                    "closed-case SHA-256 mismatch for {}",
                    file.name
                );
            }
        }
        let payloads: BTreeSet<_> = case.payload_names().collect();
        ensure!(
            declared == payloads,
            "typed closed-case inventory mismatch: declared={declared:?}, payloads={payloads:?}"
        );
        let declared_order: Vec<_> = self.files.iter().map(|file| file.name.as_str()).collect();
        let payload_order: Vec<_> = case.payload_names().collect();
        ensure!(
            declared_order == payload_order,
            "typed closed-case order mismatch: declared={declared_order:?}, payloads={payload_order:?}"
        );

        let input_names: BTreeSet<_> = self
            .files
            .iter()
            .filter(|file| file.role == FileRole::Input)
            .map(|file| file.name.as_str())
            .collect();
        let mut closure = BTreeSet::new();
        ensure!(
            input_names.contains(self.source_closure.primary.as_str()),
            "primary source {} is not a tracked input",
            self.source_closure.primary
        );
        ensure!(
            closure.insert(self.source_closure.primary.as_str()),
            "duplicate primary source"
        );
        for input in &self.source_closure.inputs {
            ensure!(
                input_names.contains(input.as_str()),
                "source closure entry {input} is not a tracked input"
            );
            ensure!(
                closure.insert(input.as_str()),
                "duplicate source closure entry: {input}"
            );
        }
        ensure!(
            closure == input_names,
            "source closure is not exact: closure={closure:?}, inputs={input_names:?}"
        );
        ensure!(
            self.publication.destination.as_path() == case.repository_relative(),
            "publication destination does not match case identity"
        );
        ensure!(
            !self.publication.authorities.is_empty(),
            "publication must consume at least one Git authority"
        );
        let mut authorities = BTreeSet::new();
        for authority in &self.publication.authorities {
            ensure!(
                authorities.insert(authority.as_path()),
                "duplicate publication authority: {}",
                authority.as_path().display()
            );
        }
        Ok(ValidatedCase {
            contract: self,
            case,
        })
    }
}

/// A contract paired with the Git and filesystem bytes it validated.
#[derive(Debug)]
pub struct ValidatedCase<'a> {
    contract: &'a Contract,
    case: &'a ClosedCase,
}

impl ValidatedCase<'_> {
    /// Stages a byte-exact candidate directory without publishing it.
    pub fn stage_into(&self, destination: &Path) -> Result<StagedCase> {
        ensure!(
            !destination.exists(),
            "closed-case staging destination already exists: {}",
            destination.display()
        );
        fs::create_dir(destination)
            .with_context(|| format!("create staged case {}", destination.display()))?;
        let staged = (|| {
            let mut inventory = String::from(INVENTORY_SCHEMA);
            inventory.push('\n');
            for file in &self.contract.files {
                let bytes = self.case.read(file.name.as_str())?;
                fs::write(destination.join(file.name.as_str()), bytes)?;
                inventory.push_str(file.name.as_str());
                inventory.push('\n');
            }
            fs::write(destination.join(INVENTORY_NAME), inventory)?;
            StagedCase::validate(destination)
        })();
        if staged.is_err() {
            let _ = fs::remove_dir_all(destination);
        }
        staged
    }
}

/// A closed local candidate. It is safe to hand to fixturegen, but does not
/// itself carry authority to publish.
#[derive(Debug)]
pub struct StagedCase {
    root: PathBuf,
    inventory: BTreeMap<String, Vec<u8>>,
}

impl StagedCase {
    pub fn validate(root: &Path) -> Result<Self> {
        let metadata = fs::symlink_metadata(root)
            .with_context(|| format!("inspect staged case {}", root.display()))?;
        ensure!(
            metadata.is_dir() && !metadata.file_type().is_symlink(),
            "staged case is not a directory"
        );
        let mut actual = BTreeMap::new();
        for entry in fs::read_dir(root)? {
            let entry = entry?;
            let kind = entry.file_type()?;
            ensure!(
                kind.is_file() && !kind.is_symlink(),
                "staged case contains a non-regular entry"
            );
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| anyhow::anyhow!("non-UTF-8 staged filename"))?;
            actual.insert(name, fs::read(entry.path())?);
        }
        ensure!(!actual.is_empty(), "staged case inventory is empty");
        if let Some(bytes) = actual.get(INVENTORY_NAME) {
            let text = std::str::from_utf8(bytes).context("staged case inventory is not UTF-8")?;
            let mut lines = text.lines();
            ensure!(
                lines.next() == Some(INVENTORY_SCHEMA),
                "case.inventory must begin with {INVENTORY_SCHEMA}"
            );
            let mut declared = BTreeSet::new();
            for name in lines.filter(|line| !line.is_empty()) {
                let name = PayloadName::new(name)?;
                ensure!(
                    declared.insert(name.as_str().to_owned()),
                    "duplicate staged inventory entry {name}"
                );
            }
            ensure!(!declared.is_empty(), "staged case inventory is empty");
            let present: BTreeSet<_> = actual.keys().cloned().collect();
            let expected = declared
                .into_iter()
                .chain(std::iter::once(INVENTORY_NAME.to_owned()))
                .collect();
            ensure!(
                present == expected,
                "staged closed inventory mismatch: declared={expected:?}, present={present:?}"
            );
        }
        Ok(Self {
            root: root.to_owned(),
            inventory: actual,
        })
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    #[must_use]
    pub fn inventory(&self) -> &BTreeMap<String, Vec<u8>> {
        &self.inventory
    }
}

fn validate_relative(path: &Path, reject_target: bool) -> Result<()> {
    ensure!(!path.as_os_str().is_empty(), "path must not be empty");
    for component in path.components() {
        let Component::Normal(name) = component else {
            bail!("path must be normalized and relative: {}", path.display());
        };
        ensure!(
            !reject_target || name != "target",
            "target-backed publication authority is forbidden: {}",
            path.display()
        );
    }
    Ok(())
}

fn validate_digest(digest: &str) -> Result<()> {
    ensure!(
        digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "SHA-256 must contain exactly 64 hexadecimal digits"
    );
    ensure!(
        digest.bytes().all(|byte| !byte.is_ascii_uppercase()),
        "SHA-256 must use lowercase hexadecimal"
    );
    Ok(())
}

fn hex_digest(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests;
