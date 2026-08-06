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

/// A Git-validated fixture paired with its typed, conventional contract.
///
/// This is the common host-facing entry point for established fixture
/// families. It derives ordered roles from their stable naming convention:
/// `expected.*` payloads are outputs, `case.json` is metadata, and all other
/// payloads form the exact source closure. Families with richer declarative
/// metadata may construct [`Contract`] directly.
#[derive(Debug)]
pub struct FixtureCase {
    contract: Contract,
    case: ClosedCase,
}

impl FixtureCase {
    /// Discovers a `case.inventory`-backed fixture.
    pub fn discover(
        case_relative: impl AsRef<Path>,
        primary: impl Into<String>,
        profile: impl Into<String>,
    ) -> Result<Self> {
        let case_relative = case_relative.as_ref();
        let case = ClosedCase::discover(case_relative)?;
        Self::from_case(case_relative, primary.into(), profile.into(), case)
    }

    /// Explicit-checkout variant for an inventory-backed fixture.
    pub fn discover_at(
        repository: &Path,
        case_relative: impl AsRef<Path>,
        primary: impl Into<String>,
        profile: impl Into<String>,
    ) -> Result<Self> {
        let case_relative = case_relative.as_ref();
        let case = ClosedCase::discover_at(repository, case_relative)?;
        Self::from_case(case_relative, primary.into(), profile.into(), case)
    }

    /// Discovers a fixture whose exact inventory is owned directly by Git.
    pub fn discover_tracked(
        case_relative: impl AsRef<Path>,
        primary: impl Into<String>,
        profile: impl Into<String>,
    ) -> Result<Self> {
        let case_relative = case_relative.as_ref();
        let case = ClosedCase::discover_tracked(case_relative)?;
        Self::from_case(case_relative, primary.into(), profile.into(), case)
    }

    /// Explicit-checkout variant used by adversarial inventory tests.
    pub fn discover_tracked_at(
        repository: &Path,
        case_relative: impl AsRef<Path>,
        primary: impl Into<String>,
        profile: impl Into<String>,
    ) -> Result<Self> {
        let case_relative = case_relative.as_ref();
        let case = ClosedCase::discover_tracked_at(repository, case_relative)?;
        Self::from_case(case_relative, primary.into(), profile.into(), case)
    }

    /// Discovers the declarative classic-BibTeX case schema without a second
    /// inventory or digest validator in bibliography tests.
    pub fn discover_classic_bibtex(case_relative: impl AsRef<Path>) -> Result<Self> {
        let case_relative = case_relative.as_ref();
        let case = ClosedCase::discover(case_relative)?;
        let metadata: serde_json::Value = serde_json::from_slice(&case.read("case.json")?)?;
        ensure!(
            metadata["schema"] == "classic-bibtex-closed-case-v1",
            "unsupported classic BibTeX case schema"
        );
        let id = case_relative
            .file_name()
            .and_then(|name| name.to_str())
            .context("classic BibTeX case ID is not UTF-8")?;
        ensure!(metadata["case"] == id, "classic BibTeX case identity drift");
        let profile = metadata["compatibility"]
            .as_str()
            .context("classic BibTeX compatibility profile is missing")?;
        let declarations = metadata["files"]
            .as_array()
            .context("classic BibTeX files must be an array")?;
        let mut aux_inputs = declarations.iter().filter_map(|declaration| {
            (declaration["role"] == "input")
                .then(|| declaration["path"].as_str())
                .flatten()
                .filter(|name| name.ends_with(".aux"))
        });
        let primary = aux_inputs
            .next()
            .context("classic BibTeX case has no AUX input")?;
        ensure!(
            aux_inputs.next().is_none(),
            "classic BibTeX case has multiple AUX inputs"
        );
        let mut fixture =
            Self::from_case(case_relative, primary.to_owned(), profile.to_owned(), case)?;
        let mut declared = BTreeMap::new();
        for declaration in declarations {
            let name = declaration["path"]
                .as_str()
                .context("classic BibTeX file path is missing")?;
            let role = match declaration["role"].as_str() {
                Some("input") => FileRole::Input,
                Some("output") => FileRole::ExpectedOutput,
                _ => bail!("invalid classic BibTeX role for {name}"),
            };
            let digest = declaration["sha256"]
                .as_str()
                .context("classic BibTeX file digest is missing")?
                .to_owned();
            ensure!(
                declared.insert(name.to_owned(), (role, digest)).is_none(),
                "duplicate classic BibTeX file declaration: {name}"
            );
        }
        let mut inputs = Vec::new();
        for file in &mut fixture.contract.files {
            if file.name.as_str() == "case.json" {
                file.role = FileRole::Metadata;
                continue;
            }
            let (role, digest) = declared
                .remove(file.name.as_str())
                .with_context(|| format!("undeclared classic BibTeX payload: {}", file.name))?;
            file.role = role;
            file.sha256 = Some(digest);
            if role == FileRole::Input && file.name != fixture.contract.source_closure.primary {
                inputs.push(file.name.clone());
            }
        }
        ensure!(
            declared.is_empty(),
            "classic BibTeX metadata names absent payloads: {declared:?}"
        );
        fixture.contract.source_closure.inputs = inputs;
        fixture.contract.validate(&fixture.case)?;
        Ok(fixture)
    }

    fn from_case(
        case_relative: &Path,
        primary: String,
        profile: String,
        case: ClosedCase,
    ) -> Result<Self> {
        let family = case_relative
            .parent()
            .context("closed-case identity has no family")?;
        let id = case_relative
            .file_name()
            .and_then(|name| name.to_str())
            .context("closed-case identity is not UTF-8")?;
        let primary = PayloadName::new(primary)?;
        let mut inputs = Vec::new();
        let files = case
            .payload_names()
            .map(|name| {
                let role = if name.starts_with("expected.") {
                    FileRole::ExpectedOutput
                } else if name == "case.json" {
                    FileRole::Metadata
                } else {
                    FileRole::Input
                };
                let name = PayloadName::new(name)?;
                if role == FileRole::Input && name != primary {
                    inputs.push(name.clone());
                }
                Ok(TrackedFile {
                    name,
                    role,
                    sha256: None,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let destination = RepositoryPath::new(case_relative)?;
        let contract = Contract {
            identity: CaseIdentity::new(family, id)?,
            files,
            status: CaseStatus::Pass,
            profile: CaseProfile(profile),
            source_closure: SourceClosure { primary, inputs },
            publication: PublicationMetadata {
                destination: destination.clone(),
                authorities: vec![destination],
            },
        };
        contract.validate(&case)?;
        Ok(Self { contract, case })
    }

    fn validated(&self) -> Result<ValidatedCase<'_>> {
        self.contract.validate(&self.case)
    }

    pub fn read(&self, name: &str) -> Result<Vec<u8>> {
        self.validated()?.read(name)
    }

    pub fn read_to_string(&self, name: &str) -> Result<String> {
        self.validated()?.read_to_string(name)
    }

    pub fn path(&self, name: &str) -> Result<PathBuf> {
        self.validated()?.path(name)
    }

    pub fn payload_path(&self, name: &str) -> Result<PathBuf> {
        self.path(name)
    }

    pub fn stage_into(&self, destination: &Path) -> Result<StagedCase> {
        self.validated()?.stage_into(destination)
    }

    #[must_use]
    pub fn contract(&self) -> &Contract {
        &self.contract
    }
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
    pub fn read(&self, name: &str) -> Result<Vec<u8>> {
        self.case.read(name)
    }

    pub fn read_to_string(&self, name: &str) -> Result<String> {
        self.case.read_to_string(name)
    }

    pub fn path(&self, name: &str) -> Result<PathBuf> {
        self.case.payload_path(name)
    }

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
            for file in &self.contract.files {
                let bytes = self.case.read(file.name.as_str())?;
                fs::write(destination.join(file.name.as_str()), bytes)?;
            }
            if self.case.has_inventory() {
                seal_candidate_inventory(
                    destination,
                    self.contract.files.iter().map(|file| file.name.as_str()),
                )?;
            }
            StagedCase::validate(destination)
        })();
        if staged.is_err() {
            let _ = fs::remove_dir_all(destination);
        }
        staged
    }
}

/// Writes the canonical inventory for a non-authoritative candidate directory.
/// Publication remains fixturegen's responsibility.
pub fn seal_candidate_inventory<'a>(
    root: &Path,
    names: impl IntoIterator<Item = &'a str>,
) -> Result<()> {
    fs::write(root.join(INVENTORY_NAME), candidate_inventory_bytes(names)?)?;
    Ok(())
}

/// Serializes the one canonical candidate inventory representation.
pub fn candidate_inventory_bytes<'a>(names: impl IntoIterator<Item = &'a str>) -> Result<Vec<u8>> {
    let mut inventory = String::from(INVENTORY_SCHEMA);
    inventory.push('\n');
    let mut seen = BTreeSet::new();
    for name in names {
        let name = PayloadName::new(name)?;
        ensure!(
            seen.insert(name.as_str().to_owned()),
            "duplicate staged inventory entry {name}"
        );
        inventory.push_str(name.as_str());
        inventory.push('\n');
    }
    ensure!(!seen.is_empty(), "staged case inventory is empty");
    Ok(inventory.into_bytes())
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
